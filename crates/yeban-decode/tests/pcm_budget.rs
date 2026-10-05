//! `HD-24`：资源预算（[`PcmBudget`]）的**集成**判据 —— 从 crate 外部调用点看这些闸门。
//!
//! 与 `src/limits.rs` 的单元判据分工：
//! - 单元判据跑在**零第三方依赖**的纯逻辑层（本机可执行，见 notes §本机真跑）；
//! - 本文件经过 symphonia（真实解码），因此**只在 CI 上执行**。
//!
//! 判据编号沿用 notes 的 C 系列。

use yeban_decode::limits::{LimitViolation, PcmBudget, check_layout, pcm_bytes_for};
use yeban_decode::{DecodeError, DecodeOptions, DecodeResult, DecodedAsset, decode_bytes};

// 上游调用形状的类型别名（写短一点，也避免 `clippy::type_complexity` 之类的噪声）。
type DecodeEntry = fn(&[u8], &DecodeOptions) -> DecodeResult<DecodedAsset>;
type ResampleEntry = fn(&[f32], u16, u32, u32) -> DecodeResult<Vec<f32>>;
type ResampleBudgetEntry = fn(&[f32], u16, u32, u32, &PcmBudget) -> DecodeResult<Vec<f32>>;
type ResampleAssetEntry = fn(&DecodedAsset, u32) -> DecodeResult<DecodedAsset>;
type ResampleAssetBudgetEntry = fn(&DecodedAsset, u32, &PcmBudget) -> DecodeResult<DecodedAsset>;

/// 构造一个最小合法 PCM16 单声道 WAV（与 `src/testfix.rs` 同格式；集成测试拿不到
/// `#[cfg(test)]` 的夹具，所以这里独立写一份）。
fn wav_pcm16_mono(frames: usize, sample_rate: u32) -> Vec<u8> {
    let data_len = u32::try_from(frames * 2).expect("fixture fits u32");
    let mut out = Vec::with_capacity(44 + frames * 2);
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
    out.resize(44 + frames * 2, 0);
    out
}

/// C50 —— **上游源码兼容**（`yeban-mcp::domain::render` 的调用形状）。
///
/// `D43` 授权推翻**本 crate 的旧设计**，但推翻不等于允许把别的工作线编译搞红：
/// `yeban-mcp` 用的是 `decode_bytes(bytes, &DecodeOptions::default())` 与
/// `resample_interleaved(&trimmed, ch, from, to)`，这两个形状必须继续成立。
/// 这条判据用函数指针强制核对签名（写错一个参数就在这里红）。
#[test]
fn upstream_call_forms_still_typecheck_against_the_budget_api() {
    let _: DecodeEntry = decode_bytes;
    let _: ResampleEntry = yeban_decode::resample_interleaved;
    let _: ResampleAssetEntry = yeban_decode::resample_asset;
    // 新增的"预算显式"入口。
    let _: ResampleBudgetEntry = yeban_decode::resample_interleaved_with_budget;
    let _: ResampleAssetBudgetEntry = yeban_decode::resample_asset_with_budget;
    // 默认选项携带的就是推导出来的默认预算（单一事实源）。
    let options = DecodeOptions::default();
    let budget: PcmBudget = options.budget;
    assert_eq!(budget, PcmBudget::default());
}

/// C51 —— 默认预算的**依据可被判据复算**（`HD-24` 的"为什么是这个数"）。
#[test]
fn default_budget_is_recomputed_from_the_product_requirements() {
    let reference = pcm_bytes_for(3 * 60 * 60, 96_000, 2).expect("3 h @ 96 kHz stereo");
    let multitrack = pcm_bytes_for(30 * 60, 96_000, 8).expect("30 min @ 96 kHz 8 ch");
    let budget = PcmBudget::default();
    assert_eq!(budget.max_pcm_bytes, reference.max(multitrack));
    // 等价时长：96 kHz 立体声 3 小时 ⇔ 96 kHz 8 声道 45 分钟。
    assert_eq!(budget.interleaved_samples_limit() / 2 / 96_000, 3 * 60 * 60);
    assert_eq!(budget.interleaved_samples_limit() / 8 / 96_000, 45 * 60);
    assert_ne!(budget.max_pcm_bytes, 2 * 1024 * 1024 * 1024);
}

/// C52 —— 默认预算**恰好**容纳 3 小时 96 kHz 立体声，多一帧即拒（边界闭区间）。
#[test]
fn default_budget_admits_the_documented_session_and_refuses_one_more_frame() {
    let budget = PcmBudget::default();
    let frames = 3 * 60 * 60 * 96_000;
    assert_eq!(check_layout(2, 96_000, frames, &budget), Ok(()));
    let err = check_layout(2, 96_000, frames + 1, &budget).unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("PCM budget") || text.contains("duration cap"),
        "got {text}"
    );
}

/// C53 —— 可配置的小预算能拦下一份**真实解码**的 WAV，且错误文本精确、可自纠。
#[test]
fn a_small_pcm_budget_stops_a_real_wav_with_a_precise_error() {
    let bytes = wav_pcm16_mono(64, 8_000);
    let strict = DecodeOptions {
        budget: PcmBudget::new(u64::MAX, 128, 64, 768_000, 60),
        ..DecodeOptions::default()
    };
    let err = decode_bytes(&bytes, &strict).unwrap_err();
    assert!(matches!(err, DecodeError::Budget(_)), "got {err}");
    let text = err.to_string();
    assert!(text.contains("PCM budget"), "got {text}");
    assert!(text.contains("64 frames x 1 channels"), "got {text}");
    assert!(text.contains("128-byte"), "got {text}");
    // 同一份字节在默认预算下正常解出 64 帧 ⇒ 上面红的是预算而不是格式。
    let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
    assert_eq!(asset.frame_count(), 64);
    assert_eq!(asset.sample_rate(), 8_000);
}

/// C54 —— 输入字节闸门精确到字节，且发生在探测之前。
#[test]
fn the_input_byte_cap_is_precise_and_precedes_probing() {
    let bytes = wav_pcm16_mono(8, 8_000);
    let strict = DecodeOptions {
        budget: PcmBudget {
            max_input_bytes: 16,
            ..PcmBudget::default()
        },
        ..DecodeOptions::default()
    };
    let err = decode_bytes(&bytes, &strict).unwrap_err();
    assert!(
        matches!(
            err,
            DecodeError::Budget(LimitViolation::InputTooLarge {
                bytes: 60,
                limit: 16
            })
        ),
        "got {err}"
    );
    assert!(err.to_string().contains("60 bytes"), "got {err}");
}

/// C55 —— 时长闸门独立生效（字节预算宽到用不完也会被拒）。
#[test]
fn the_duration_cap_is_independent_and_precise() {
    let bytes = wav_pcm16_mono(16_000, 8_000); // 2 秒 @8 kHz 单声道
    let strict = DecodeOptions {
        budget: PcmBudget {
            max_duration_secs: 1,
            ..PcmBudget::default()
        },
        ..DecodeOptions::default()
    };
    let err = decode_bytes(&bytes, &strict).unwrap_err();
    assert!(
        matches!(
            err,
            DecodeError::Budget(LimitViolation::DurationTooLong { .. })
        ),
        "got {err}"
    );
    assert!(err.to_string().contains("duration cap"), "got {err}");
    // 恰好 1 秒通过（闭区间），且默认预算下 2 秒也通过。
    let exact = wav_pcm16_mono(8_000, 8_000);
    assert_eq!(decode_bytes(&exact, &strict).unwrap().frame_count(), 8_000);
    assert_eq!(
        decode_bytes(&bytes, &DecodeOptions::default())
            .unwrap()
            .frame_count(),
        16_000
    );
}

/// C56 —— `audio-render` 在用的重采样路径走**调用方**的预算，而不是写死常量。
#[test]
fn the_resample_path_obeys_the_callers_budget() {
    let bytes = wav_pcm16_mono(48_000, 48_000); // 1 秒 @48 kHz 单声道
    let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
    assert_eq!(asset.frame_count(), 48_000);

    // 只够"理想输出"（44100 帧）的预算：重采样器实际要分配的缓冲含滤波器延迟/余量，
    // 因此必须被拒 —— 证明闸门发生在 `try_reserve` 之前的真实长度上。
    let ideal_only = PcmBudget::new(u64::MAX, 44_100 * 4, 64, 768_000, 60);
    let err = yeban_decode::resample_interleaved_with_budget(
        asset.samples(),
        1,
        48_000,
        44_100,
        &ideal_only,
    )
    .unwrap_err();
    assert!(matches!(err, DecodeError::Budget(_)), "got {err}");
    assert!(err.to_string().contains("PCM budget"), "got {err}");

    // 同一个调用在默认预算下成功，且长度满足既有契约。
    let out = yeban_decode::resample_interleaved_with_budget(
        asset.samples(),
        1,
        48_000,
        44_100,
        &PcmBudget::default(),
    )
    .unwrap();
    yeban_decode::limits::check_resampled_len(
        48_000,
        44_100,
        48_000,
        u64::try_from(out.len()).unwrap(),
    )
    .expect("长度契约");

    // 采样率闸门同样走调用方预算。
    let narrow = PcmBudget::new(u64::MAX, 1 << 30, 64, 44_100, 60);
    assert!(matches!(
        yeban_decode::resample_interleaved_with_budget(asset.samples(), 1, 48_000, 44_100, &narrow),
        Err(DecodeError::Budget(
            LimitViolation::SampleRateTooHigh { .. }
        ))
    ));
}

/// C57 —— 解码正确性回归：预算收紧到"恰好够"时，解出的样本值与默认预算下**逐位相同**。
#[test]
fn tightening_the_budget_to_the_exact_pcm_size_does_not_change_the_samples() {
    let bytes = wav_pcm16_mono(1_024, 44_100);
    let default = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
    let exact = DecodeOptions {
        budget: PcmBudget {
            max_pcm_bytes: u64::try_from(default.samples().len()).unwrap() * 4,
            ..PcmBudget::default()
        },
        ..DecodeOptions::default()
    };
    let tightened = decode_bytes(&bytes, &exact).unwrap();
    assert_eq!(tightened.frame_count(), default.frame_count());
    assert_eq!(tightened.pcm_hash(), default.pcm_hash());
    assert_eq!(
        tightened
            .samples()
            .iter()
            .map(|s| s.to_bits())
            .collect::<Vec<_>>(),
        default
            .samples()
            .iter()
            .map(|s| s.to_bits())
            .collect::<Vec<_>>()
    );
}
