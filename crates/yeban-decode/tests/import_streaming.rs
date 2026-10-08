//! `import_path` 的**流式摘要**判据 —— 从 crate 外部调用点看磁盘导入。
//!
//! 这条线把 `import_path` 从"整份容器读进内存再算 SHA-256"改成"分块喂
//! `yeban_model::AssetHasher`"，因此峰值内存从 ×1 份 PCM + 整份容器降到 ×1 份 PCM
//! （`docs/ledger/decode-limits-notes.md` §2.3 的 `import_path` / `import_bytes` 行）。
//!
//! 判据的口径是**结果等价**：磁盘路径与内存路径对同一份字节必须给出同一个
//! `AssetHash`、同一份 PCM、同一份事实。摘要口径变了会在这里红 —— 而
//! `AssetHash` 是 CAS 的键，改它等于把已有容器的 `assets/{sha256}` 全部作废。
//!
//! 与 `tests/pcm_budget.rs` 的分工：那边钉预算闸门，这边钉摘要与解码路径。
//! 两者都经过 symphonia，因此只在 CI 上执行（本机可跑的是 `src/limits.rs` /
//! `src/duration.rs` / `src/testfix.rs` 里零第三方依赖的单元判据）。

use std::path::PathBuf;

use yeban_decode::{
    AssetHash, DecodeError, DecodeOptions, ImportedAsset, asset_index, import_bytes, import_path,
};

/// 从磁盘导入一份字节，用完即删（返回值不借用临时目录）。
fn import_temp_file(bytes: &[u8], name: &str, options: &DecodeOptions) -> ImportedAsset {
    let path = temp_path(name);
    std::fs::write(&path, bytes).expect("夹具文件必须写得进临时目录");
    let outcome = import_path(&path, "CC0-1.0", options);
    let cleanup = std::fs::remove_file(&path);
    let asset = outcome.expect("临时夹具文件必须能被导入");
    cleanup.expect("夹具文件必须删得掉");
    asset
}

/// 临时文件路径。用进程号 + 名字，避免并行判据互相覆盖。
fn temp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("yeban-import-{}-{name}.wav", std::process::id()));
    path
}

/// 构造一个标准 44 字节头的 PCM16 单声道 WAV（与 `tests/pcm_budget.rs` 同一份最小夹具）。
///
/// `values` 直接决定解码后的样本，因此调用方可以按住"解出来的 PCM 必须是什么"。
fn wav_pcm16_mono(sample_rate: u32, values: &[i16]) -> Vec<u8> {
    let data_len = u32::try_from(values.len() * 2).expect("fixture fits u32");
    let mut out = Vec::with_capacity(44 + values.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// 判据 I1 —— 磁盘导入的摘要必须等于"整份字节一次性摘要"，且**跨过** 64 KiB 的分块边界。
///
/// `HASH_BUFFER_BYTES` = 64 KiB，本夹具 200 000 个样本 = 400 000 字节 ⇒ 至少 7 次
/// `read` 返回满缓冲，因此这条判据真的走了分块循环，而不是"一次读完"的退化情形。
///
/// 它同时是**注入点 ①**（见提交说明）：任何"只把第一块喂进摘要"的缺陷都会让
/// `asset_hash()` 与 `AssetHash::of_bytes(&bytes)` 不等 ⇒ 在这里红。
#[test]
fn a_multi_chunk_file_hashes_exactly_like_the_whole_byte_string() {
    // 16-bit 单声道 200 000 帧：400 044 字节（44 字节头 + 400 000 字节 PCM）。
    let values: Vec<i16> = (0..200_000u32)
        .map(|index| (index % 4_001) as i16 - 2_000)
        .collect();
    let bytes = wav_pcm16_mono(48_000, &values);
    assert!(
        bytes.len() > 6 * 64 * 1024,
        "夹具必须跨过多个 64 KiB 哈希块，实际 {} 字节",
        bytes.len()
    );

    let imported = import_temp_file(&bytes, "multi-chunk", &DecodeOptions::default());

    // ① 摘要 = 整份字节的 SHA-256（内容寻址的键）。
    assert_eq!(imported.asset_hash(), &AssetHash::of_bytes(&bytes));
    // ② 与"内存入口"给出的索引逐字段相同（byte_len / media_kind / license / path）。
    let from_bytes = import_bytes(&bytes, "unused", "CC0-1.0", &DecodeOptions::default()).unwrap();
    assert_eq!(imported.index.hash, from_bytes.index.hash);
    assert_eq!(imported.index.byte_len, from_bytes.index.byte_len);
    assert_eq!(imported.index.byte_len, bytes.len() as u64);
    assert_eq!(imported.index.media_kind, from_bytes.index.media_kind);
    assert_eq!(imported.index.license, from_bytes.index.license);
    // ③ 解码结果也必须**逐位**相同：分块摘要不能顺手改掉解码路径。
    assert_eq!(imported.decoded.frame_count(), 200_000);
    assert_eq!(imported.decoded.sample_rate(), 48_000);
    assert_eq!(imported.pcm_hash(), from_bytes.pcm_hash());
    assert_eq!(
        imported
            .decoded
            .samples()
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        from_bytes
            .decoded
            .samples()
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    );
}

/// 判据 I2 —— 磁盘入口与内存入口对**同一份小文件**逐字段等价（含 `original_path` 之外的索引字段）。
///
/// 小文件（< 一个哈希块）覆盖 `read` 只返回一个短块的分支；I1 覆盖多块分支。
/// 两条合起来把 `hash_reader` 的循环边界夹住。
#[test]
fn the_disk_entry_and_the_memory_entry_agree_on_a_short_file() {
    let bytes = wav_pcm16_mono(8_000, &[0, 1_000, -1_000, 2_000, -2_000, 3_000]);
    let imported = import_temp_file(&bytes, "short", &DecodeOptions::default());
    let from_bytes =
        import_bytes(&bytes, "short.wav", "CC0-1.0", &DecodeOptions::default()).unwrap();

    assert_eq!(imported.index.hash, from_bytes.index.hash);
    assert_eq!(imported.index.hash, AssetHash::of_bytes(&bytes));
    assert_eq!(imported.index.byte_len, from_bytes.index.byte_len);
    assert_eq!(imported.pcm_hash(), from_bytes.pcm_hash());
    assert_eq!(imported.decoded.frame_count(), 6);
    // 内容寻址的第一性：同一份字节换一个路径，键不变。
    assert_eq!(
        imported.index.hash,
        asset_index(&bytes, "elsewhere.wav", "CC0-1.0").hash
    );
    assert_ne!(imported.index.original_path, from_bytes.index.original_path);
}

/// 判据 I3 —— 超预算的文件在读之前就被拒，且错误是**类型化**的（不是 panic，不是 I/O 错）。
///
/// `import_path` 不在音频线程（后台 IO 池），但它面对的是**任意字节**的用户文件，
/// 因此契约与解码路径相同：只允许返回 `DecodeError`。
#[test]
fn an_over_budget_file_is_refused_without_decoding_it() {
    let bytes = wav_pcm16_mono(8_000, &[0; 1_024]);
    let strict = DecodeOptions {
        budget: yeban_decode::PcmBudget {
            max_input_bytes: 64, // 夹具是 44 + 2048 = 2092 字节
            ..yeban_decode::PcmBudget::default()
        },
        ..DecodeOptions::default()
    };
    let path = temp_path("over-budget");
    std::fs::write(&path, &bytes).expect("夹具文件必须写得进临时目录");
    let outcome = import_path(&path, "CC0-1.0", &strict);
    let cleanup = std::fs::remove_file(&path);
    cleanup.expect("夹具文件必须删得掉");

    let err = outcome.expect_err("超出输入字节预算的文件必须被拒");
    assert!(
        matches!(
            err,
            DecodeError::Budget(yeban_decode::limits::LimitViolation::InputTooLarge {
                bytes: 2_092,
                limit: 64
            })
        ),
        "expected InputTooLarge at 2092 bytes, got {err}"
    );
    // 同一份字节在默认预算下正常导入 ⇒ 上面红的是闸门，不是夹具坏了。
    let ok = import_temp_file(&bytes, "over-budget-ok", &DecodeOptions::default());
    assert_eq!(ok.decoded.frame_count(), 1_024);
}
