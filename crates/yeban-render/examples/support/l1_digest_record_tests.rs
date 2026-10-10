//! # `MUST-GATE-002` L1 摘要记录的**本机零依赖验证脚手架**（**不是** cargo 目标）
//!
//! 与 [`l1_receipt_tests.rs`](l1_receipt_tests.rs) 同一套路：`yeban-render` 含 `rayon`/`hound`，
//! 但摘要记录的**字段表 / 归一化口径 / 判决 / JSON 往返 / SHA-256 / WAV 容器**全是纯逻辑，
//! 零第三方依赖 —— 因此可以在本机真跑：
//!
//! ```text
//! rustc --edition 2024 --test -D warnings -W missing_docs \
//!   crates/yeban-render/examples/support/l1_digest_record_tests.rs -o /tmp/l1-digest-record-tests
//! /tmp/l1-digest-record-tests
//! ```
//!
//! ## 覆盖范围（如实声明）
//!
//! - ✅ 覆盖：摘要记录的**归一化口径**（参与/仅记录字段表）、**三种判决**（PASS / FAIL / SKIP）、
//!   "跨平台 ⇒ SKIP 而不是 FAIL"、"同平台不同 ⇒ FAIL"、"非参与字段变了仍然绿"、
//!   JSON 往返与确定性、严格解析（未知字段/坏值/自相矛盾不 panic）、
//!   SHA-256 的 FIPS 已知向量、WAV 容器载荷与位型串接**逐字节相同**、
//!   位型→载荷→哈希这条链的**无损自证**。
//! - ❌ **不覆盖**：任何真正调用 `RenderPlan::execute` 的东西（需要 `rayon`）——
//!   真实渲染的端到端判据在 `tests/l1_digest_parity.rs`，由 `cargo test`（CI 或已预热的本机）判定。
//!
//! 本文件不被 `cargo` 自动发现（`examples/support/` 下没有 `main.rs`），因此不会给 CI
//! 增加编译目标 —— 这是刻意的（与两份既有脚手架一致）。

// 本文件是 `rustc --test` 的 **crate root**, 不是库；被包含模块的公开 API 在这里没有
// "外部消费者", 因此 `dead_code` 会误报。真实判定由 CI 的
// `cargo clippy -p yeban-render --all-targets -- -D warnings` 执行。
#![allow(dead_code)]

#[path = "l1_digest_record.rs"]
mod l1_digest_record;

use std::collections::BTreeMap;

use l1_digest_record::{
    CrossPlatform, DigestEnvelope, DigestRecord, DigestScope, JudgeError, Participation,
    PlatformIdentity, RenderParams, SCHEMA, Verdict, WAV_HEADER_BYTES, compared_fields,
    encode_wav_f32_le, format_utc, hex_lower, judge, judge_policy, latency_token, parse,
    participation_of, pcm_bits_bytes, recorded_only_fields, report, same_platform,
    sample_digest_of, sha256, split_rustc_version, to_json, to_json_line, to_pretty_json,
    wav_data_payload,
};

// ---------------------------------------------------------------------------
// 脚手架夹具
// ---------------------------------------------------------------------------

/// 32 轨参考工程 A 的**摘要记录形状**：只填判据真正读到的字段。
///
/// 样本由确定性脚手架合成（本文件不渲染）：真读数与它的差别只在具体位型，
/// 而判决读的是"两个记录是否逐字段相同"，因此这不妨碍判据的判别力。
fn record_for(samples: &[f32]) -> DigestRecord {
    build(samples, |_| {})
}

/// 造一份记录，`tweak` 用来注入单个字段的变化。
fn build(samples: &[f32], tweak: impl FnOnce(&mut DigestRecord)) -> DigestRecord {
    let payload = pcm_bits_bytes(samples);
    let wav = encode_wav_f32_le(samples, 2, 48_000);
    let bits = hex_lower(&sha256(&payload));
    let mut record = DigestRecord {
        schema: SCHEMA.to_owned(),
        params: RenderParams {
            fixture: "reference-a".to_owned(),
            seed: 0x5EED,
            sample_rate: 48_000,
            channels: 2,
            frames: samples.len() as u64 / 2,
            tracks: 32,
            block_size: 128,
            gain_db: "none".to_owned(),
            threads: "auto".to_owned(),
            latency: BTreeMap::new(),
        },
        platform: PlatformIdentity {
            target_arch: std::env::consts::ARCH.to_owned(),
            target_os: std::env::consts::OS.to_owned(),
            target_env: String::new(),
            target_endian: if cfg!(target_endian = "big") {
                "big".to_owned()
            } else {
                "little".to_owned()
            },
            target_pointer_width: usize::BITS.to_string(),
            target_triple: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            rustc_release: "1.99.0".to_owned(),
            rustc_host: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            rustc_commit: "b940084d7".to_owned(),
            rustc_commit_date: "2026-09-28".to_owned(),
            isa_features: "baseline".to_owned(),
        },
        envelope: DigestEnvelope {
            algorithm: "sha256".to_owned(),
            input: "canonical-pcm-bits".to_owned(),
            sample_format: "f32-le".to_owned(),
            wav_encoding: "pcm-f32-le".to_owned(),
            scope: DigestScope::PcmPayloadOnly,
            wav_bytes: WAV_HEADER_BYTES + payload.len() as u64,
        },
        digest: bits.clone(),
        sample_digest: sample_digest_of(samples),
        // 以下全是**仅记录**字段：刻意填"本机此刻"的值，判据 ⑥ 会改动它们并断言仍然绿。
        rustc_version: "rustc 1.99.0 (b940084d7 2026-09-28)".to_owned(),
        host_name: "scaffold-host".to_owned(),
        host_os_version: "scaffold-os-1.0".to_owned(),
        generated_at_utc: "1970-01-01T00:00:00Z".to_owned(),
        notes: String::new(),
    };
    assert_eq!(
        record.sample_digest, bits,
        "无损编码下位型串接与 data 载荷必须给出同一个 SHA-256"
    );
    assert_eq!(
        wav.len() as u64,
        WAV_HEADER_BYTES + payload.len() as u64,
        "WAV 头布局必须是规范里的固定布局"
    );
    tweak(&mut record);
    record
}

/// 32 轨 × 8 帧（64 样本）的确定性合成母带：位型多样（含 `-0.0`、次正规、极值）。
fn synthetic_master() -> Vec<f32> {
    let mut out = Vec::with_capacity(64);
    let mut state = 0x5EED_1234u32;
    for index in 0..64u32 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let unit = state as f32 / u32::MAX as f32;
        let value = match index % 8 {
            0 => -0.0,
            1 => f32::from_bits(1), // 最小次正规
            2 => f32::MAX,
            3 => f32::MIN_POSITIVE,
            4 => 1.0,
            5 => -1.0,
            6 => 1.0e-30,
            _ => (unit - 0.5) * 2.0,
        };
        out.push(value);
    }
    out
}

// ---------------------------------------------------------------------------
// SHA-256 与 WAV 容器
// ---------------------------------------------------------------------------

/// 判据：SHA-256 对上 FIPS 180-4 的已知向量（自带实现不许自欺）。
#[test]
fn sha256_matches_the_fips_vectors() {
    assert_eq!(
        hex_lower(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex_lower(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex_lower(&sha256(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    // 跨块边界 + 长度字段进位（一兆个 'a'，FIPS 的长消息向量）。
    let million: Vec<u8> = std::iter::repeat_n(b'a', 1_000_000).collect();
    assert_eq!(
        hex_lower(&sha256(&million)),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

/// 判据：WAV 的 `data` 载荷**逐字节等于**位型串接 ⇒ `pcm-payload-only` 口径自证无损。
#[test]
fn wav_payload_is_byte_identical_to_the_sample_bits() {
    let samples = synthetic_master();
    let wav = encode_wav_f32_le(&samples, 2, 48_000);
    let payload = wav_data_payload(&wav).expect("自研容器必须能取出 data 载荷");
    assert_eq!(payload, pcm_bits_bytes(&samples).as_slice());
    assert_eq!(
        hex_lower(&sha256(payload)),
        sample_digest_of(&samples),
        "载荷哈希与位型哈希必须相同（这就是'无损'的可机器检验定义）"
    );
    assert_eq!(payload.len() as u64, samples.len() as u64 * 4);
}

/// 判据：有损编码**不许**用 `pcm-payload-only` 口径（否则会静默丢掉量化信息）。
#[test]
fn lossy_encoding_is_refused_by_validation() {
    let samples = synthetic_master();
    let lossy = build(&samples, |record| {
        record.envelope.wav_encoding = "pcm-s16-le".to_owned();
    });
    let error = lossy.validate().expect_err("有损编码必须被拒");
    assert_eq!(error.code, "validate-wav-lossless-rule");
}

/// 判据：`wav_bytes` 永远是**整个文件**的字节数，与 `scope` 无关（口径只改覆盖范围）。
#[test]
fn wav_bytes_always_counts_the_whole_file() {
    let samples = synthetic_master();
    let payload_only = build(&samples, |_| {});
    assert_eq!(
        payload_only.envelope.wav_bytes,
        WAV_HEADER_BYTES + samples.len() as u64 * 4
    );
    assert!(payload_only.validate().is_ok());
    let whole = build(&samples, |record| {
        record.envelope.scope = DigestScope::WholeFile;
    });
    assert!(whole.validate().is_ok(), "同一份文件只用换口径, 字节数不变");
    assert_eq!(whole.envelope.wav_bytes, payload_only.envelope.wav_bytes);
    // 字节数漏算头 ⇒ 自相矛盾。
    let wrong = build(&samples, |record| {
        record.envelope.wav_bytes -= WAV_HEADER_BYTES;
    });
    let error = wrong.validate().expect_err("漏算头的字节数必须被拒");
    assert_eq!(error.code, "wav-bytes-mismatch");
}

// ---------------------------------------------------------------------------
// 归一化口径（字段表）
// ---------------------------------------------------------------------------

/// 判据：字段表本身是自洽的、且**参与**与**仅记录**两类都非空。
///
/// 还有一条容易漏的：每个字段都必须能被 JSON 序列化出来（否则"字段表"就是一份
/// 与实现无关的散文）。这里用序列化 + 解析往返来钉住它。
#[test]
fn field_table_matches_the_serialized_record() {
    let record = record_for(&synthetic_master());
    let json = to_json(&record);
    let parsed = parse(&json).expect("自己写出的 JSON 必须能解析");
    assert_eq!(parsed, record);
    assert_eq!(to_json(&parsed), json, "序列化必须逐字节确定");
    // 字段表里的每一个名字都必须真的出现在 JSON 里。
    for name in compared_fields().into_iter().chain(recorded_only_fields()) {
        assert!(
            json.contains(&format!("\"{name}\":")),
            "字段表声称有 `{name}`, 但 JSON 里没有"
        );
    }
    // 反向：JSON 的顶层字段数必须等于字段表长度（不许有"表外字段"）。
    // 每个顶层字段都是 `"name":` 形态；字符串值内不含裸引号（转义过），因此计数可靠。
    let top_level = json.matches("\":").count();
    assert_eq!(
        top_level,
        compared_fields().len() + recorded_only_fields().len(),
        "JSON 的顶层字段数必须等于字段表长度"
    );
    assert!(participation_of("digest").is_some_and(|p| p == Participation::Compared));
    assert!(participation_of("generated_at_utc").is_some_and(|p| p == Participation::RecordedOnly));
    assert!(participation_of("nope").is_none(), "未知字段必须查不到");
}

// ---------------------------------------------------------------------------
// 判决：PASS / FAIL / SKIP
// ---------------------------------------------------------------------------

/// 判据：同一份摘要 ⇒ `PASS`（退出码 0），且"仅记录字段的差异"如实报告但不改判决。
#[test]
fn identical_records_pass_even_with_recorded_only_differences() {
    let samples = synthetic_master();
    let reference = build(&samples, |_| {});
    let local = build(&samples, |record| {
        record.host_name = "a-completely-different-host".to_owned();
        record.host_os_version = "other-os-9.9".to_owned();
        record.generated_at_utc = "2026-09-29T12:34:56Z".to_owned();
        record.rustc_version = "rustc 1.99.0 (different-build 2026-09-28)".to_owned();
        record.notes = "本机重算".to_owned();
    });
    let judgement = judge(&reference, &local).expect("同 schema 必须可比");
    assert_eq!(judgement.verdict, Verdict::Pass);
    assert_eq!(judgement.reason, "digest-identical");
    assert_eq!(judgement.verdict.exit_code(), 0);
    assert!(judgement.digest_equal);
    assert!(judgement.diffs.is_empty(), "参与字段不许有差异");
    assert_eq!(
        judgement.recorded_only_differences.len(),
        5,
        "仅记录字段的差异必须如实报告（不许隐藏）"
    );
    let text = report(&judgement);
    assert!(text.starts_with("VERDICT PASS\n"));
    assert!(text.contains("recorded-only field=host_name"));
}

/// 判据：**同平台**下参与字段被改（ISA 写错）⇒ `FAIL`（退出码 1），并点名该字段。
#[test]
fn a_wrong_isa_on_the_same_platform_is_a_hard_fail() {
    let samples = synthetic_master();
    let reference = build(&samples, |_| {});
    let local = build(&samples, |record| {
        record.platform.isa_features = "x86-64-v3+fma".to_owned();
    });
    let judgement = judge(&reference, &local).expect("同 schema 必须可比");
    assert_eq!(judgement.verdict, Verdict::Fail);
    assert_eq!(judgement.reason, "compared-field-mismatch");
    assert_eq!(judgement.verdict.exit_code(), 1);
    assert!(judgement.same_platform);
    assert!(judgement.toolchain_locked);
    assert_eq!(judgement.diffs.len(), 1);
    assert_eq!(judgement.diffs[0].field, "isa_features");
    let text = report(&judgement);
    assert!(text.starts_with("VERDICT FAIL\n"));
    assert!(text.contains("diff field=isa_features"));
}

/// 判据：**参与字段全部相同、只有读数不同** ⇒ `FAIL`，原因码是 `digest-mismatch`。
///
/// 这条是"判据不是恒绿"的关键：哈希本身不同，必须硬红。
/// 注意 `digest` 与 `sample_digest` 必须**一起**改 —— 无损编码下它们是同一个 SHA-256，
/// 只改一个会让记录自相矛盾（而"自相矛盾"是比"不同"更早的失败，见下一条判据）。
#[test]
fn a_changed_digest_with_identical_parameters_is_a_hard_fail() {
    let samples = synthetic_master();
    let reference = build(&samples, |_| {});
    let local = build(&samples, |record| {
        record.digest = "0".repeat(64);
        record.sample_digest = "0".repeat(64);
    });
    let judgement = judge(&reference, &local).expect("同 schema 必须可比");
    assert_eq!(judgement.verdict, Verdict::Fail);
    assert_eq!(judgement.reason, "digest-mismatch");
    assert!(!judgement.digest_equal);
    assert!(!judgement.sample_digest_equal);
    assert!(judgement.diffs.iter().any(|diff| diff.field == "digest"));
    assert!(
        judgement
            .diffs
            .iter()
            .any(|diff| diff.field == "sample_digest")
    );
}

/// 判据：**只改 `digest` 不改 `sample_digest`** ⇒ 记录自相矛盾（无损编码下两者必同）。
///
/// 这条防的是"两个读数是编的"：它让[`DigestRecord::validate`] 成为**交叉校验**，
/// 而不是对序列化结果的复读。
#[test]
fn a_digest_that_contradicts_the_sample_digest_is_refused() {
    let samples = synthetic_master();
    let record = build(&samples, |record| {
        record.digest = "0".repeat(64);
    });
    let error = record.validate().expect_err("自相矛盾的记录必须被拒");
    assert_eq!(error.code, "payload-and-bits-digest-differ");
    let local = build(&samples, |_| {});
    let error = judge(&local, &record).expect_err("坏记录不许给出判决");
    assert!(matches!(error, JudgeError::LocalMalformed(_)));
}

/// 判据：**跨平台**（不同 arch / 不同三元组）⇒ `SKIP`（退出码 **2**，不是 0，也不是 1）。
///
/// 这是本模块最不容含糊的一条：跨平台的哈希差异是 `MUST-GATE-003` 的领域，
/// 本门禁**没有**判定 —— 不许写成"通过"，也不许写成"渲染不确定"。
#[test]
fn a_cross_platform_reference_is_skipped_not_failed() {
    let samples = synthetic_master();
    let reference = build(&samples, |_| {});
    let local = build(&samples, |record| {
        record.platform.target_arch = "x86_64".to_owned();
        record.platform.target_triple = "x86_64-unknown-linux-gnu".to_owned();
    });
    let judgement = judge(&reference, &local).expect("schema 相同 ⇒ 仍可给结论");
    assert_eq!(judgement.verdict, Verdict::Skip);
    assert_eq!(judgement.reason, "cross-platform");
    assert_eq!(judgement.verdict.exit_code(), 2, "SKIP 的退出码必须是 2");
    assert_ne!(judgement.verdict.exit_code(), 0);
    assert!(!judgement.same_platform);
    let text = report(&judgement);
    assert!(text.starts_with("VERDICT SKIP\n"), "不许把 SKIP 写成 PASS");
    assert!(!text.contains("VERDICT PASS"));
    assert!(text.contains("exit_code=2"));
    assert!(text.contains("请勿记为通过"));
}

/// 判据：**同平台但工具链未锁定**（发布号不同）⇒ `SKIP`，原因码与跨平台**可区分**。
#[test]
fn a_different_toolchain_release_is_skipped_with_its_own_reason() {
    let samples = synthetic_master();
    let reference = build(&samples, |_| {});
    let local = build(&samples, |record| {
        record.platform.rustc_release = "1.100.0".to_owned();
    });
    let judgement = judge(&reference, &local).expect("同 schema 必须可比");
    assert_eq!(judgement.verdict, Verdict::Skip);
    assert_eq!(judgement.reason, "toolchain-not-locked");
    assert!(judgement.same_platform, "平台是相同的");
    assert!(!judgement.toolchain_locked);
    assert!(
        !judgement.diffs.is_empty(),
        "rustc_release 是参与字段, 差异必须如实列出"
    );
    let text = report(&judgement);
    assert!(text.starts_with("VERDICT SKIP\n"));
    assert!(text.contains("reason=toolchain-not-locked"));
}

/// 判据：参考摘要**自相矛盾** ⇒ 明确错误（不是 panic，也不是"通过"）。
#[test]
fn a_malformed_reference_is_a_clear_error() {
    let samples = synthetic_master();
    let reference = build(&samples, |record| {
        record.digest = "not-hex".to_owned();
    });
    let local = build(&samples, |_| {});
    let error = judge(&reference, &local).expect_err("坏摘要不许给出判决");
    assert!(matches!(error, JudgeError::ReferenceMalformed(_)));
    assert!(error.to_string().contains("参考摘要不可用"));
}

/// 判据：schema 不同 ⇒ 明确错误（不许猜一个判决），且**解析器拒绝不认识的版本**。
///
/// 两条一起才完整：`judge` 里的 `SchemaMismatch` 覆盖"未来版本的记录被**认识**它的
/// 解析器读进来"这条路（必须能说出"不可比"）；`parse` 里的白名单覆盖"谁都不认识它"。
#[test]
fn a_schema_mismatch_refuses_to_judge() {
    let samples = synthetic_master();
    let reference = build(&samples, |_| {});
    let local = build(&samples, |record| {
        record.schema = "yeban-l1-digest/2".to_owned();
    });
    assert!(
        local.validate().is_ok(),
        "版本不同但自洽的记录必须能通过自洽性检查（否则连'不可比'都说不出口）"
    );
    let error = judge(&reference, &local).expect_err("schema 不同不许比对");
    assert!(matches!(error, JudgeError::SchemaMismatch { .. }));
    assert!(error.to_string().contains("无法逐字段比对"));
    // 解析器侧：不认识的版本必须被**响亮地**拒绝，而不是猜着解析。
    let json = to_json(&local);
    let error = parse(&json).expect_err("不认识的 schema 必须被拒");
    assert!(error.to_string().contains("不认识的 schema"), "{error}");
}

// ---------------------------------------------------------------------------
// 确定性 / 往返 / 严格解析
// ---------------------------------------------------------------------------

/// 判据 ⑦：摘要生成器**确定性** —— 同输入两次产出**逐字节相同**（单行与多行两种形态）。
#[test]
fn generation_is_byte_identical_for_the_same_input() {
    let samples = synthetic_master();
    let first = build(&samples, |_| {});
    let second = build(&samples, |_| {});
    assert_eq!(to_json_line(&first), to_json_line(&second));
    assert_eq!(to_pretty_json(&first), to_pretty_json(&second));
    // 两种形态必须是**同一份数据**（多行只是排版，不许改变读数）。
    assert_eq!(parse(&to_json_line(&first)).unwrap(), first);
    assert_eq!(parse(&to_pretty_json(&first)).unwrap(), first);
    assert_eq!(
        to_json_line(&first).lines().count(),
        1,
        "单行形态必须真的只有一行"
    );
    assert!(
        to_pretty_json(&first).lines().count() > 10,
        "多行形态必须真的多行"
    );
    // 注入一个仅记录字段 ⇒ 单行文本**必须**变化（证明它不是"文本恒定"的假绿），
    // 但逐字段比对仍然 PASS（判据 ⑥ 的另一半，在同一个夹具上被证明）。
    let touched = build(&samples, |record| {
        record.host_name = "another-host".to_owned();
    });
    assert_ne!(to_json_line(&first), to_json_line(&touched));
    assert_eq!(judge(&first, &touched).unwrap().verdict, Verdict::Pass);
}

/// 判据：**种子变化必然改变 digest**（负向对照：防止"digest 是常量"的假绿）。
///
/// 脚手架用"样本不同 ⇒ 位型不同 ⇒ 哈希不同"来表达这条；真实渲染侧的对应判据在
/// `tests/l1_digest_parity.rs`（真的换种子重渲染）。
#[test]
fn a_different_seed_necessarily_changes_the_digest() {
    let mut other = synthetic_master();
    other[7] += 1.0;
    let reference = build(&synthetic_master(), |_| {});
    let local = build(&other, |record| {
        record.params.seed = 0x5EEE;
    });
    assert_ne!(reference.digest, local.digest);
    assert_ne!(reference.sample_digest, local.sample_digest);
    let judgement = judge(&reference, &local).expect("可比");
    assert_eq!(judgement.verdict, Verdict::Fail);
    assert_eq!(judgement.reason, "digest-mismatch");
}

/// 判据：严格解析 —— 未知字段 / 坏值 / 自相矛盾都是**明确错误**，绝不 panic。
#[test]
fn malformed_records_are_errors_not_panics() {
    let record = record_for(&synthetic_master());
    let json = to_json(&record);
    // 未知字段（拼错一个字段名会让"参与比对"静默少一个 ⇒ 必须拒绝）。
    let unknown = json.replace("\"digest\":", "\"digset\":");
    let error = parse(&unknown).expect_err("未知字段必须被拒");
    assert!(error.to_string().contains("未知字段"), "{error}");
    // 缺字段。
    let missing = json.replace("\"digest_scope\":\"pcm-payload-only\",", "");
    assert!(parse(&missing).is_err());
    // 坏 digest（长度不对）。
    let bad_digest = json.replace(&record.digest, "abc");
    assert!(parse(&bad_digest).is_err());
    // 自相矛盾：无损编码下 digest 与 sample_digest 不同。
    // ⚠ 关键：`digest` 是 `sample_digest` 的**子串**，所以不能用裸 `replace`
    // （会一次改掉两个字段 ⇒ 记录变成"另一个自洽的读数"，判据就空转了）。
    // 必须用带引号的字段名做精确锚点（这是 L3 的"注入必须命中且只命中锚点"）。
    let anchor = format!("\"sample_digest\":\"{}\"", record.sample_digest);
    let replacement = format!("\"sample_digest\":\"{}\"", "f".repeat(64));
    let contradictory = json.replace(&anchor, &replacement);
    assert_ne!(contradictory, json, "注入必须真的改到东西");
    assert!(
        contradictory.contains(&format!("\"digest\":\"{}\"", record.digest)),
        "注入不许碰到 digest 字段"
    );
    let error = parse(&contradictory).expect_err("自相矛盾必须被拒");
    assert!(
        error.to_string().contains("payload-and-bits-digest-differ"),
        "{error}"
    );
    // 顶层不是对象。
    assert!(parse("[1,2,3]").is_err());
    assert!(parse("").is_err());
    // 顶层不是 JSON 也是错误。
    assert!(parse("digest 1234").is_err());
}

/// 判据：延迟表的规范文本是**键升序、确定**的（人读报告与比对都依赖它）。
#[test]
fn latency_tokens_are_order_independent_and_deterministic() {
    let mut forward = BTreeMap::new();
    forward.insert("00000000000000000000000001".to_owned(), 0u32);
    forward.insert("00000000000000000000000002".to_owned(), 64u32);
    let mut backward = BTreeMap::new();
    backward.insert("00000000000000000000000002".to_owned(), 64u32);
    backward.insert("00000000000000000000000001".to_owned(), 0u32);
    assert_eq!(latency_token(&forward), latency_token(&backward));
    assert_eq!(
        latency_token(&forward),
        "00000000000000000000000001:0,00000000000000000000000002:64"
    );
    assert_eq!(latency_token(&BTreeMap::new()), "none");
}

/// 判据：`format_utc` 对已知时刻给出正确的日历（它是**仅记录**字段，但写错了会让账本说谎）。
#[test]
fn utc_formatting_matches_known_instants() {
    assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_utc(1), "1970-01-01T00:00:01Z");
    assert_eq!(format_utc(86_399), "1970-01-01T23:59:59Z");
    assert_eq!(format_utc(86_400), "1970-01-02T00:00:00Z");
    assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z"); // 2000 是闰年（整百但被 400 整除）
    assert_eq!(format_utc(1_709_164_800), "2024-02-29T00:00:00Z"); // 2024 是闰年
    assert_eq!(format_utc(1_759_638_400), "2025-10-05T04:26:40Z"); // 本线参考摘要用的时刻
    assert_eq!(format_utc(4_102_444_800), "2100-01-01T00:00:00Z"); // 2100 不是闰年（3 月 1 日推后一位）
    assert_eq!(format_utc(4_107_542_400), "2100-03-01T00:00:00Z");
    // 负值（1970 之前）也必须确定，而不是 panic。
    assert_eq!(format_utc(-1), "1969-12-31T23:59:59Z");
    assert_eq!(format_utc(-86_400), "1969-12-31T00:00:00Z");
}

/// 判据：`split_rustc_version` 对真实与退化输入都不编造。
///
/// 本机实测的两种形态：`1.99.0 (b940084d7eb6a2… 2026-09-28)`（rustc 1.99.0 的 `-vV`）
/// 与 CI 归档日志里的 `1.99.0 (b940084d7 2026-09-28)`（同一个 commit 的**短写**）。
#[test]
fn rustc_version_splitting_never_invents_fields() {
    let (release, commit, date) = split_rustc_version(Some(
        "1.99.0 (b940084d7eb6a299eb4bfeb8e34901bc051e7ac4 2026-09-28)",
    ));
    assert_eq!(release, "1.99.0");
    assert_eq!(commit, "b940084d7eb6a299eb4bfeb8e34901bc051e7ac4");
    assert_eq!(date, "2026-09-28");
    let (release, commit, date) = split_rustc_version(Some("1.99.0"));
    assert_eq!(release, "1.99.0");
    assert!(commit.is_empty() && date.is_empty());
    let (release, commit, date) = split_rustc_version(None);
    assert!(release.is_empty() && commit.is_empty() && date.is_empty());
}

/// 判据：**跨平台策略**必须显式、具名，且默认永远是规范口径。
///
/// 三条口径：默认 ⇒ `SKIP`（不是通过）；显式 `DigestParity` + 读数相同 ⇒
/// `PASS-CROSS-PLATFORM`（记号与 `PASS` 不同，退出码 0）；显式 `DigestParity` + 读数不同 ⇒
/// 仍然 `SKIP`（跨平台差异是 `MUST-GATE-003` 的领域，判红就是假红）。
#[test]
fn cross_platform_policy_is_explicit_and_distinguishable() {
    let samples = synthetic_master();
    let local = build(&samples, |_| {});
    let mut foreign = local.clone();
    // ⚠ 异平台按**编译期事实**取反构造，不许写死 —— run 37267482265 就是因为把异平台写死成
    // `x86_64-unknown-linux-gnu`，而 CI 恰好就是那个平台，于是"异平台"不异了。
    let foreign_arch = if cfg!(target_arch = "x86_64") {
        "aarch64"
    } else {
        "x86_64"
    };
    let foreign_triple = if cfg!(target_arch = "x86_64") {
        "aarch64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    foreign.platform.target_arch = foreign_arch.to_owned();
    foreign.platform.target_os = if cfg!(target_arch = "x86_64") {
        "macos".to_owned()
    } else {
        "linux".to_owned()
    };
    foreign.platform.target_env = if cfg!(target_arch = "x86_64") {
        String::new()
    } else {
        "gnu".to_owned()
    };
    foreign.platform.target_triple = foreign_triple.to_owned();
    foreign.platform.rustc_host = foreign_triple.to_owned();
    foreign.validate().expect("只改平台身份仍然是自洽的记录");
    assert_ne!(foreign_arch, std::env::consts::ARCH, "异平台必须真的不同");
    assert!(
        !same_platform(&foreign.platform, &local.platform),
        "异平台必须真的不同"
    );

    // 默认策略 = 规范口径。
    let default = judge(&foreign, &local).expect("可比");
    assert_eq!(default.verdict, Verdict::Skip);
    assert_eq!(default.reason, "cross-platform");
    assert_eq!(default.verdict.exit_code(), 2);
    assert!(!report(&default).contains("VERDICT PASS"));

    // 显式策略 + 读数相同。
    let probe = judge_policy(&foreign, &local, CrossPlatform::DigestParity).expect("可比");
    assert_eq!(probe.verdict, Verdict::PassCrossPlatform);
    assert_eq!(probe.reason, "cross-platform-digest-identical");
    assert_eq!(probe.verdict.exit_code(), 0);
    let text = report(&probe);
    assert!(text.starts_with("VERDICT PASS-CROSS-PLATFORM\n"));
    assert!(!text.contains("VERDICT PASS\n"), "不许冒充普通的 PASS");
    assert!(text.contains("不是 MUST-GATE-002 的通过"));

    // 显式策略 + 读数不同 ⇒ 仍然 SKIP（不是 FAIL）。
    let mut different = foreign.clone();
    different.digest = "0".repeat(64);
    different.sample_digest = "0".repeat(64);
    let mismatch = judge_policy(&different, &local, CrossPlatform::DigestParity).expect("可比");
    assert_eq!(mismatch.verdict, Verdict::Skip);
    assert_ne!(mismatch.verdict, Verdict::Fail);
    assert_eq!(mismatch.verdict.exit_code(), 2);

    // 默认值必须是规范口径（不是"更强的那条"）。
    assert_eq!(CrossPlatform::default(), CrossPlatform::Skip);
}
