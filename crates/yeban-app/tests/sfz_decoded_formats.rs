//! 判据（任务单 `#85`）：SFZ 样本的**通用解码腿** —— **FLAC** 真的被解出来，且与 WAV 腿
//! **逐位同口径**。
//!
//! ## 本单改了哪两件事（`#83` 的乙口径 ⇒ 甲口径）
//!
//! 1. `crates/yeban-app/Cargo.toml` 新增**一条工作区内部**依赖边 `yeban-decode`
//!    （人类批文："加 `yeban-decode`"）；⛔ 没有新的第三方包。
//! 2. `crates/yeban-app/src/sfz_load.rs` 的 `decode_sample_classified` **按后缀分派**：
//!    `.wav` / `.wave` ⇒ 既有 hound 腿（**逐位不变**）；**其余** ⇒ `yeban_decode::decode_path`
//!    ＋ **显式折单声道**（`step_by(channels)` 取声道 0 —— 与 hound 腿同一口径）。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `differing` | 两次导出（WAV 版 vs FLAC 版）里 `f32` **位型不同**的样本 | 个 |
//! | `max_abs_diff` | 同上，逐样本 `abs(a − b)` 的最大值 | 线性幅度 |
//! | `fnv1a_64` | 一条导出的**交错 f32 样本**（逐位卷字节） | 无符号 64 位整数 |
//! | `decoded_samples` | 真的进了提供表的样本份数 | 份 |
//! | `silent_region_count` | 引擎口径：缺样本 ⇒ `NO_SAMPLE` 的区域数 | 区域数 |
//! | `sample_hz` | 读取器上报的**文件自身**采样率 | Hz |
//! | `channels` / `interleaved samples` | FLAC 夹具解码后的声道数 / 交织样本数 | 个 / 个 |
//!
//! ## 判据清单
//!
//! | # | 判据 | 作用 |
//! | :--- | :--- | :--- |
//! | ① | `a_flac_sample_is_bit_identical_to_the_wav_leg_after_the_normalisation_fix` | **本单核心**：同一份 PCM 的 WAV／FLAC 导出**逐位相同**（`#254` 统一分母后） |
//! | ② | `a_stereo_flac_is_folded_to_channel_zero_like_the_wav_leg` | 折单声道口径与 WAV 腿一致（⭐ 注入 ② 的靶子） |
//! | ③ | `a_flac_at_another_sample_rate_is_read_at_its_own_rate` | 采样率不匹配 ⇒ **不重采样**，按引擎既有播放速率口径 |
//!
//! ## 夹具来源（⛔ 不是"随便造的字节"）
//!
//! 三份 FLAC 都由**参考编码器** `flac`（libFLAC 1.5.0）从**本判据自己写的** 24-bit PCM WAV
//! **无损**转出，命令逐字：`flac -s -f --no-padding -8 -o <out>.flac <in>.wav`
//! （`--no-padding` 只是去掉 libFLAC 默认的 8 KiB PADDING 块 ⇒ 夹具小到可嵌入；
//! ⛔ 不影响任何 PCM 字节）。每份的 SHA-256 在下面逐条登记，判据用
//! `yeban_model::AssetHash` 机械核对（摘要不符 = 夹具被换过 = 红）。
//!
//! ⚠ **实测差因（⛔ 不是"FLAC 有损"）**：`#254` 之前两条腿的**归一化分母不同** ——
//!
//! | 腿 | 公式（`24-bit` 24 位整数 `raw`） | 分母 |
//! | :--- | :--- | :--- |
//! | WAV（hound，`read_plain_wav_mono_f32`） | ~~`raw as f32 * (1 / 8388607)`~~ → `raw as f32 * (1 / 8388608)` | ~~`2^23 − 1`~~ → `2^23` |
//! | FLAC／OGG（symphonia → `DecodedAsset::samples`） | `(raw << 8) as f32 / 2^31` = `raw / 8388608` | `2^23` |
//!
//! ⭐ **`#254` 已把 WAV 腿统一到 `2^23`**（`read_plain_wav_mono_f32` 用
//! `BitDepth::scale()` 而不是 `BitDepth::full_scale_max()`）⇒ 同一份无损 PCM 的两条腿
//! 现在**逐位相同**（判据 ① 的 `differing == 0`、`max_abs_diff == 0`、两个 FNV 指纹相同）。
//! 判据 ① 据此断言相等，并逐位钉住"两条腿都是 `raw / 2^23`"。
//! ⚠ 这也顺带移动了 `sfz_sample_formats::BASELINE_WAV_READ` 的指纹（已按三件套重钉）。
//!
//! [`BitDepth::full_scale_max`]: yeban_render::dither::BitDepth::full_scale_max
//!
//! [`DecodeOptions`]: yeban_decode::DecodeOptions

use std::path::{Path, PathBuf};

use yeban_app::render_core::{RenderProjectOutput, render_project_f32_with_assets};
use yeban_app::sfz_load::read_sample_file;
use yeban_decode::{DecodeOptions, decode_path};
use yeban_model::{AssetHash, EntityId, SamplerBinding, YebanProjectV1};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav_mono_f32, write_plain_wav};

/// `24-bit` 满量程（夹具写入常量样本用）。
const I24_SCALE: f32 = 8_388_608.0;

/// ⭐ 两条腿**统一后**的归一化分母（`#254`）：WAV 腿与 symphonia 腿都用 `2^23`。
///
/// `#254` 之前 WAV 腿用 `2^23 − 1`（[`BitDepth::Int24::full_scale_max`]），两条腿的相对差
/// 是 `1/8388607 ≈ 1.1920929e-7`。统一之后这个常量只剩**历史记录**用途（打印出来对照），
/// ⛔ 不再被任何"差必须 ≤ 界"的断言使用。
const DENOMINATOR_GAP: f32 = 1.0 / 8_388_607.0;

/// 阶梯夹具的八段：`(线性幅度, 帧数)`，共 `4800` 帧。
///
/// ⚠ **⛔ 刻意不是常量**：八个互不相同（含正负）的值让"WAV 腿 vs FLAC 腿"的比较**不是**
/// "两份都恰好是 `0.25`"这种空对空 —— 任何增益／符号／分段的错位都会改变指纹。
const STAIR_SEGMENTS: [(f32, usize); 8] = [
    (0.25, 600),
    (-0.5, 600),
    (0.125, 600),
    (-0.75, 600),
    (0.5, 600),
    (-0.125, 600),
    (0.375, 600),
    (-0.25, 600),
];

/// 立体声夹具的**左**声道（= [`STAIR_SEGMENTS`] 展开后的 4800 帧）。
///
/// ⚠ 右声道恒为 `-0.5`（与左声道**处处不同**）⇒ 把"折单声道"那一步改成交织直传，
/// 端到端必然变（注入 ② 的靶子）。
fn stair_values() -> Vec<f32> {
    STAIR_SEGMENTS
        .iter()
        .flat_map(|(value, frames)| std::iter::repeat_n(*value, *frames))
        .collect()
}

// ---------------------------------------------------------------------------
// FLAC 夹具（参考编码器产出；见模块文档"夹具来源"）
// ---------------------------------------------------------------------------

/// 单声道 `48 kHz` 阶梯（8 段 × 600 帧）的 FLAC。
///
/// `flac -s -f --no-padding -8` —— 747 字节，
/// SHA-256 `72fb9bef678b1702765f77e80eb96944cfd2f870b0cc0c43cf288cf031844d6e`。
const STAIR_FLAC_HEX: &str = concat!(
    "664c614300000022100010000000690002160bb80170000012c08fb7d0e31aad25f89e75a33e0d5f2c9a030000120000",
    "0000000000000000000000000000100084000028200000007265666572656e6365206c6962464c414320312e352e3020",
    "323032353032313100000000fff8ca0c007c13000012003fffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffff800fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff003ffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "fffffffffffffffffffffc001fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe00001ffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "fffffffffffffffffffffffffffffffffffffffffe00ffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ff00ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffff08cdfff87a0c0102bfea13000013003fffffffffffffffffffffffff803fffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffc0fce9",
);

/// 立体声 `48 kHz` 的 FLAC：**左** = 阶梯，**右** = 常量 `-0.5`。
///
/// `flac -s -f --no-padding -8` —— 755 字节，
/// SHA-256 `074b15f1c5e43d43f399bb30c6c0d7c5f08baecb2d6aa6890323b3d1399ceb28`。
const STEREO_FLAC_HEX: &str = concat!(
    "664c6143000000221000100000006d00021a0bb80370000012c057c9653b61f2e8b321004fb044a27e2a030000120000",
    "0000000000000000000000000000100084000028200000007265666572656e6365206c6962464c414320312e352e3020",
    "323032353032313100000000fff8ca1c002b13000012003fffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffff800fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff003ffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "fffffffffffffffffffffc001fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe00001ffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "fffffffffffffffffffffffffffffffffffffffffe00ffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ff00ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffff01000007d872fff87a1c0102bf8d13000013003fffffffffffffffffffffffff",
    "803fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffc0400001c0cde8",
);

/// 单声道 `44.1 kHz` 常量 `0.25`（4800 帧）的 FLAC。
///
/// `flac -s -f --no-padding -8` —— 134 字节，
/// SHA-256 `f260a5ccb29cd1ae06279ed8562322009c423c51ac71869dfc770a64eb6cf65f`。
const M44_FLAC_HEX: &str = concat!(
    "664c6143000000221000100000000c00000e0ac44170000012c0f4f6966c32883135d55fb385cc739c82030000120000",
    "0000000000000000000000000000100084000028200000007265666572656e6365206c6962464c414320312e352e3020",
    "323032353032313100000000fff8c90c00c10100000935d3fff8790c0102bf4c010000093296",
);

/// 十六进制串 ⇒ 字节（夹具嵌入用；⛔ 不引第三方 hex crate）。
fn unhex(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "十六进制串必须是偶数长度");
    (0..hex.len() / 2)
        .map(|index| {
            u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("夹具十六进制必须合法")
        })
        .collect()
}

/// 取一份夹具，并**机械核对**它的 SHA-256（模块文档里登记的那个）—— 换过即红。
fn fixture(hex: &str, expected_sha256: &str, expected_bytes: usize) -> Vec<u8> {
    let bytes = unhex(hex);
    assert_eq!(bytes.len(), expected_bytes, "夹具字节数必须与登记值一致");
    assert_eq!(&bytes[0..4], b"fLaC", "夹具必须是 FLAC 容器");
    assert_eq!(
        AssetHash::of_bytes(&bytes).as_str(),
        expected_sha256,
        "夹具的 SHA-256 必须与登记值一致（摘要不符 = 夹具被换过）"
    );
    bytes
}

// ---------------------------------------------------------------------------
// 工程/渲染夹具（形态照 `sfz_sample_formats.rs`）
// ---------------------------------------------------------------------------

/// 临时目录。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-decoded-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 32f 位深让"落盘"这一步**不引入抖动**（`quantize` 对 `Float32` 是透传）。
fn float_options() -> yeban_app::export_wav::WavExportOptions {
    yeban_app::export_wav::WavExportOptions {
        bit_depth: BitDepth::Float32,
        ..yeban_app::export_wav::WavExportOptions::default()
    }
}

/// 写一份**多段常量**的 `24-bit` 单声道 WAV（⛔ 不是单一常量 ⇒ 指纹有区分力）。
fn write_segments_wav(path: &Path, segments: &[(f32, usize)]) {
    let values: Vec<i32> = segments
        .iter()
        .flat_map(|(value, frames)| std::iter::repeat_n((I24_SCALE * value) as i32, *frames))
        .collect();
    write_plain_wav(
        path,
        &PcmFormat::integer(1, 48_000, 24),
        &PcmBuffer::Int24(values),
    )
    .expect("写 WAV 夹具");
}

/// 写一份 `24-bit` 立体声 WAV（左右声道**逐帧交替** ⇒ 折单声道那一步必须取声道 0）。
fn write_stereo_wav(path: &Path, left: &[f32], right: &[f32]) {
    assert_eq!(left.len(), right.len(), "两声道帧数必须相同");
    let mut values: Vec<i32> = Vec::with_capacity(left.len() * 2);
    for (l, r) in left.iter().zip(right) {
        values.push((I24_SCALE * l) as i32);
        values.push((I24_SCALE * r) as i32);
    }
    write_plain_wav(
        path,
        &PcmFormat::integer(2, 48_000, 24),
        &PcmBuffer::Int24(values),
    )
    .expect("写立体声 WAV 夹具");
}

/// 工程里那条**带 MIDI 音符**的非主总线轨。
fn midi_track_of(project: &YebanProjectV1) -> EntityId {
    *project
        .tracks
        .iter()
        .find(|(id, track)| {
            **id != project.master_bus_track_id
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .map(|(id, _)| id)
        .expect("filled_project 必须有一条带 MIDI 音符的非主总线轨")
}

/// 那条轨上出现过的全部音高（排序 ＋ 去重）。
fn pitches_of(project: &YebanProjectV1, track_id: EntityId) -> Vec<u8> {
    let mut pitches: Vec<u8> = project
        .tracks
        .get(&track_id)
        .expect("轨必须在")
        .clips
        .values()
        .filter_map(|placement| project.clip_pool.get(&placement.clip_id))
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().map(|note| note.pitch))
        .collect();
    pitches.sort_unstable();
    pitches.dedup();
    assert!(!pitches.is_empty(), "夹具必须真的带音符");
    pitches
}

/// 写一份"每个工程音高一条区域、全部指同一份 `sample`"的 SFZ。
fn write_sfz(dir: &Path, name: &str, pitches: &[u8], sample: &str) -> PathBuf {
    let mut text = String::new();
    for pitch in pitches {
        text.push_str(&format!(
            "<region> sample={sample} key={pitch} pitch_keycenter={pitch}\n"
        ));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("写 SFZ 夹具");
    path
}

/// 把工程那条 MIDI 轨的 `sampler` 指向 `sample` 的 SFZ，然后**导出一次**。
fn render_bound(dir: &Path, sfz_name: &str, sample: &str) -> RenderProjectOutput {
    let mut project = yeban_model::samples::filled_project();
    // ⭐ `#243`：WAV/FLAC 两条腿的**逐样本差**只应来自解码器（`#243` 时是归一化分母；
    // `#254` 统一分母之后是"没有差"）；自动化是另一个自变量 ⇒ 在夹具里清掉
    // （两条腿仍然同一条增益路径，但没有自动化包络）。
    for track in project.tracks.values_mut() {
        track.automation_lanes.clear();
    }
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let sfz = write_sfz(dir, sfz_name, &pitches, sample);
    project.tracks.get_mut(&lead).expect("轨必须在").sampler = Some(SamplerBinding {
        path: sfz.to_string_lossy().into_owned(),
    });
    render_project_f32_with_assets(&project, float_options(), None, &[])
        .expect("样本读不到**不是**错误（区域静音 ＋ 计数）")
}

/// 非零样本**个数**（单位：个）。
fn nonzero(samples: &[f32]) -> usize {
    samples.iter().filter(|sample| **sample != 0.0).count()
}

/// FNV-1a 64（逐位；把每个 `f32` 的 4 个字节按小端卷进哈希）—— 与 `sfz_model_chain` 同一算式。
fn fnv1a_64(samples: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// ⭐ 判据 ①（**本单核心**）：**同一份 PCM 的 WAV 版与 FLAC 版导出逐位相同**（`#254` 之后）。
///
/// ## 夹具
///
/// * `stair.wav` —— 本判据用 `write_plain_wav` 现写的 `24-bit` 单声道八段阶梯（4800 帧）；
/// * `stair.flac` —— **同一份 PCM** 经参考编码器 `flac --no-padding -8` 无损转出（747 字节）。
///
/// ## 判定量
///
/// * FLAC 那次的报告：`decoded_samples == 1`、`unreadable_samples` 空、`silent_region_count == 0`
///   （⭐ 这一条同时钉住"FLAC **不再**是 `UnsupportedFormat`"）；
/// * `nonzero(flac) == nonzero(wav) > 0`（两条腿都真的发声 ⇒ ⛔ 不是"缺样本 ⇒ 全静音"）；
/// * **同分母（机械化）**：对八段的 `raw` 值逐位核对
///   `wav_pcm == raw as f32 * (1/8388608)` **且** `flac_pcm == raw as f32 * (1/8388608)`
///   （`#254` 之前 WAV 腿是 `1/8388607`）；
/// * **渲染层**：`max_abs_diff == 0`（同分母 ⇒ 无损两面必须给出**逐位**相同的 `f32`）、
///   `fnv1a_wav == fnv1a_flac`。
///
/// ## 实测读数（本机 `aarch64/macos`）
///
/// | 时间点 | `differing` | `max_abs_diff` | `fnv1a_wav` | `fnv1a_flac` |
/// | :--- | :--- | :--- | :--- | :--- |
/// | `#254` 之前（2026-10-16） | `9598` | `5.960464478e-8` | `0xd034eabf3ed36e39` | `0xa6468fe2ca254801` |
/// | **`#254` 之后（2026-10-20）** | **`0`** | **`0.000000000e0`** | **`0x5265a49d67ae4575`** | **`0x5265a49d67ae4575`** |
///
/// ## 为什么现在相等（⛔ 不是"FLAC 有损"）
///
/// FLAC **无损**：`flac` 从 24-bit PCM 转出、解回来的是**同一个整数**（`raw`）。`#254` 之前
/// 差出在**归一化分母**：hound 腿算 `raw as f32 * (1/(2^23 − 1))`，symphonia 腿算
/// `(raw << 8) as f32 / 2^31 = raw / 2^23`。`#254` 把 hound 腿改成 `2^23` ⇒ 两条腿同一个
/// 公式 ⇒ `differing == 0`。下面八段的**逐位**核对把这个"同分母"钉成实测事实。
///
/// ⚠ **怎么变红**：① 把分派关掉（一切走 hound）⇒ FLAC 版全静音 ⇒ `nonzero` 与
/// `unreadable` 两条同时红；② 把任一条腿的分母改回 `2^23 − 1`（或换成 `2^24`）⇒
/// 逐位核对与 `max_abs_diff == 0` 同时红；③ 换掉夹具（SHA-256 不符）⇒ `fixture()` 红。
#[test]
fn a_flac_sample_is_bit_identical_to_the_wav_leg_after_the_normalisation_fix() {
    let dir = scratch("flac-e2e");
    write_segments_wav(&dir.join("stair.wav"), &STAIR_SEGMENTS);
    std::fs::write(
        dir.join("stair.flac"),
        fixture(
            STAIR_FLAC_HEX,
            "72fb9bef678b1702765f77e80eb96944cfd2f870b0cc0c43cf288cf031844d6e",
            747,
        ),
    )
    .expect("写 FLAC 夹具");

    let wav = render_bound(&dir, "wav.sfz", "stair.wav");
    let flac = render_bound(&dir, "flac.sfz", "stair.flac");

    // FLAC 那一次必须**真的**走了通用解码腿（⛔ 不是"两边都缺样本 ⇒ 都静音"）。
    assert_eq!(flac.sfz_decoded.len(), 1, "模型链条必须注入一条 SFZ 摊平表");
    let (_, report) = &flac.sfz_decoded[0];
    assert_eq!(
        report.decoded_samples, 1,
        "`stair.flac` 必须真的进提供表：{:?}",
        report.unreadable_sample_reasons
    );
    assert!(
        report.unreadable_samples.is_empty(),
        "⭐ FLAC 不再落任何读失败档（`UnsupportedFormat` 已收窄）：{:?}",
        report.unreadable_sample_reasons
    );
    assert_eq!(report.silent_region_count, 0, "所有区域都必须有样本");

    // ---- 逐样本差（本单核心读数）----
    assert_eq!(
        wav.samples.len(),
        flac.samples.len(),
        "两条腿的导出帧数必须相同"
    );
    let mut differing = 0usize;
    let mut max_abs_diff = 0.0_f32;
    let mut first_diff: Option<(usize, f32, f32)> = None;
    for (index, (left, right)) in wav.samples.iter().zip(&flac.samples).enumerate() {
        if left.to_bits() != right.to_bits() {
            differing += 1;
            let difference = (left - right).abs();
            if difference > max_abs_diff {
                max_abs_diff = difference;
            }
            if first_diff.is_none() {
                first_diff = Some((index, *left, *right));
            }
        }
    }
    let wav_hash = fnv1a_64(&wav.samples);
    let flac_hash = fnv1a_64(&flac.samples);
    let peak = wav
        .samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));
    println!(
        "[app-sfz-decoded/e2e] wav_samples={} flac_samples={} differing={differing} \
         max_abs_diff={max_abs_diff:.9e} first_diff={first_diff:?} peak={peak:.9} \
         denominator_gap={DENOMINATOR_GAP:.9e} fnv1a_wav=0x{wav_hash:016x} \
         fnv1a_flac=0x{flac_hash:016x} wav_nonzero={} flac_nonzero={} flac_report={}",
        wav.samples.len(),
        flac.samples.len(),
        nonzero(&wav.samples),
        nonzero(&flac.samples),
        report.summary(),
    );

    assert!(
        nonzero(&wav.samples) > 0,
        "夹具必须真的发声（⛔ 否则任何'差'都不可判）"
    );
    assert_eq!(
        nonzero(&flac.samples),
        nonzero(&wav.samples),
        "两条腿的非零样本个数必须相同（⛔ 差一个数量级 = 有一条腿没读/静音）"
    );
    let samples = &wav.samples;
    assert!(
        samples.iter().any(|sample| *sample < 0.0) && samples.iter().any(|sample| *sample > 0.0),
        "阶梯夹具必须同时有正负样本（⛔ 单一常量会让比较失去区分力）"
    );

    // ---- ⭐ 差因（机械化）：两条腿的**归一化分母**不同 ⇒ 逐位核对两个公式 ----
    // WAV 腿：`read_plain_wav_mono_f32` 用 `BitDepth::scale()` = `2^23`（`#254`）。
    // FLAC 腿：symphonia 把 24-bit 左对齐到 i32 再除 `2^31` ⇒ 等于 `raw / 2^23`。
    let (wav_pcm, wav_rate, _) =
        read_plain_wav_mono_f32(dir.join("stair.wav")).expect("回读 WAV 夹具");
    let asset = decode_path(&dir.join("stair.flac"), &DecodeOptions::default())
        .expect("直接解码 FLAC 夹具");
    assert_eq!(wav_rate, 48_000);
    assert_eq!(asset.sample_rate(), 48_000);
    assert_eq!(wav_pcm.len(), asset.samples().len());
    let mut cursor = 0usize;
    for (value, frames) in STAIR_SEGMENTS {
        let raw = (I24_SCALE * value) as i32;
        assert_eq!(
            wav_pcm[cursor],
            raw as f32 * (1.0 / 8_388_608.0),
            "WAV 腿必须**逐位**是 `raw × 1/2^23`（`#254` 起；raw={raw}，段值={value}）"
        );
        assert_eq!(
            asset.samples()[cursor],
            raw as f32 * (1.0 / 8_388_608.0),
            "FLAC 腿必须**逐位**是 `raw × 1/2^23`（raw={raw}，段值={value}）"
        );
        assert_eq!(
            wav_pcm[cursor],
            asset.samples()[cursor],
            "⭐ 同一个 `raw` 的两条腿必须**逐位**相同（`#254` 统一分母；raw={raw}）"
        );
        cursor += frames;
    }

    // ---- ⭐ 同分母 ⇒ 渲染层必须**逐位**相同（不是"差在某个界之下"）----
    assert_eq!(
        max_abs_diff,
        0.0,
        "⭐ 两条腿同分母（都是 `2^23`）⇒ 无损两面**必须**逐位相同；若 > 0，说明有一条腿\
         的分母被改回 `2^23 − 1`（实得 {max_abs_diff:.9e}；旧口径的相对差量级\
         ≈ {:.9e}）",
        peak * DENOMINATOR_GAP
    );
    assert_eq!(
        wav_hash, flac_hash,
        "同分母 ⇒ FNV 指纹必须相同（wav=0x{wav_hash:016x} flac=0x{flac_hash:016x}）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据 ②（**折单声道口径**；⭐ 注入 ② 的靶子）：立体声 FLAC 与 WAV **同款**取声道 0。
///
/// ## 夹具
///
/// * `stair.wav` —— 单声道八段阶梯（**WAV 腿**的同口径参照）；
/// * `stair.flac` —— 同一份单声道 PCM 的 FLAC（**FLAC 腿**的同口径参照）；
/// * `stereo.wav` —— 逐帧交替 `(阶梯, −0.5)` 的**立体声** WAV（走既有 hound 腿）；
/// * `stereo.flac` —— **同一份立体声 PCM** 的 FLAC（755 字节）。
///
/// ## 判定量
///
/// * 夹具自证：`decode_path(stereo.flac)` ⇒ `channels == 2`、交织样本 `9600`、
///   且**第 0 与第 1 个样本不同**（⛔ 若夹具退化成单声道，本判据测不出任何东西）；
/// * **WAV 腿内部一致**：立体声 WAV 的导出指纹 == 单声道 WAV 参照（既有口径：取**声道 0**）；
/// * **FLAC 腿内部一致**：立体声 FLAC 的导出指纹 == 单声道 FLAC 参照（⭐ 折单声道那一枪的靶子）；
/// * `read_sample_file(stereo.flac)` ⇒ 折后 `4800` 帧（⛔ 不是 `9600`）。
///
/// ⚠ **为什么两条腿各自比**：`#254` 之前两条腿的归一化分母不同（见模块文档），跨腿比会把
/// "分母差"和"折单声道"两件事混在一条断言里。`#254` 之后分母已统一（两腿逐位相同），但
/// 分开比仍然更好：下面那两条**只**对折法敏感，红了就只有一个原因。
///
/// ⚠ **注入 ②**：把 `decode_with_yeban_decode` 里的 `step_by(channels)` 改成 `step_by(1)`
/// （交织直传）⇒ 折后变 9600 帧、左右交替 ⇒ `flac_hash` 与帧数断言**同时红**。
#[test]
fn a_stereo_flac_is_folded_to_channel_zero_like_the_wav_leg() {
    let dir = scratch("flac-stereo");
    let left = stair_values();
    let right = vec![-0.5_f32; left.len()];
    write_segments_wav(&dir.join("stair.wav"), &STAIR_SEGMENTS);
    write_stereo_wav(&dir.join("stereo.wav"), &left, &right);
    std::fs::write(
        dir.join("stereo.flac"),
        fixture(
            STEREO_FLAC_HEX,
            "074b15f1c5e43d43f399bb30c6c0d7c5f08baecb2d6aa6890323b3d1399ceb28",
            755,
        ),
    )
    .expect("写立体声 FLAC 夹具");
    std::fs::write(
        dir.join("stair.flac"),
        fixture(
            STAIR_FLAC_HEX,
            "72fb9bef678b1702765f77e80eb96944cfd2f870b0cc0c43cf288cf031844d6e",
            747,
        ),
    )
    .expect("写单声道 FLAC 参照");

    // ---- 夹具自证：它**真的**是立体声，且左右不同（否则注入 ② 测不出东西）----
    let asset = decode_path(&dir.join("stereo.flac"), &DecodeOptions::default())
        .expect("立体声 FLAC 夹具必须能解");
    assert_eq!(asset.channels(), 2, "夹具必须是 2 声道");
    assert_eq!(
        asset.samples().len(),
        9_600,
        "夹具必须是 4800 帧 × 2 声道的交织"
    );
    assert_ne!(
        asset.samples()[0],
        asset.samples()[1],
        "夹具的左右声道必须不同（⛔ 否则'折单声道'那一步不可判）"
    );

    // ---- 两条腿各自的"取声道 0"参照 ----
    let mono_wav = render_bound(&dir, "mono-wav.sfz", "stair.wav");
    let mono_flac = render_bound(&dir, "mono-flac.sfz", "stair.flac");
    let stereo_wav = render_bound(&dir, "stereo-wav.sfz", "stereo.wav");
    let stereo_flac = render_bound(&dir, "stereo-flac.sfz", "stereo.flac");

    for (name, out) in [("mono-flac", &mono_flac), ("stereo-flac", &stereo_flac)] {
        let (_, report) = &out.sfz_decoded[0];
        assert_eq!(
            report.decoded_samples, 1,
            "`{name}` 的样本必须真的进提供表：{:?}",
            report.unreadable_sample_reasons
        );
    }

    // 折后帧数：4800（⛔ 不是 9600）。`read_sample_file` 的第二个返回值就是帧数。
    let (sample, frames, describe) =
        read_sample_file(&dir.join("stereo.flac")).expect("立体声 FLAC 必须可读");
    assert_eq!(
        frames, 4_800,
        "折单声道后必须是 4800 帧（⛔ 交织直传会是 9600）"
    );
    assert_eq!(sample.pcm.len(), 4_800, "PCM 长度也必须折到 4800");
    assert_eq!(sample.sample_hz, 48_000.0, "采样率原样上报");
    assert!(
        describe.contains("frames=4800"),
        "描述必须写折后帧数：{describe}"
    );

    let mono_wav_hash = fnv1a_64(&mono_wav.samples);
    let stereo_wav_hash = fnv1a_64(&stereo_wav.samples);
    let mono_flac_hash = fnv1a_64(&mono_flac.samples);
    let stereo_flac_hash = fnv1a_64(&stereo_flac.samples);
    println!(
        "[app-sfz-decoded/stereo] wav_leg mono=0x{mono_wav_hash:016x} stereo=0x{stereo_wav_hash:016x} \
         flac_leg mono=0x{mono_flac_hash:016x} stereo=0x{stereo_flac_hash:016x} \
         folded_frames={frames} interleaved_before_fold=9600 wav_nonzero={} flac_nonzero={}",
        nonzero(&mono_wav.samples),
        nonzero(&mono_flac.samples),
    );

    assert!(nonzero(&mono_wav.samples) > 0, "WAV 参照必须真的发声");
    assert_eq!(
        stereo_wav_hash, mono_wav_hash,
        "既有 WAV 腿对立体声也是**取声道 0**（0x{stereo_wav_hash:016x} vs 0x{mono_wav_hash:016x}）"
    );
    assert_eq!(
        stereo_flac_hash, mono_flac_hash,
        "⭐ FLAC 腿必须与它自己的单声道参照**逐位相同**（取声道 0）：\
         0x{stereo_flac_hash:016x} vs 0x{mono_flac_hash:016x} —— \
         若不等，最可能是把**交织**样本当成了单声道"
    );
    assert_eq!(
        stereo_flac.samples.len(),
        mono_flac.samples.len(),
        "折单声道后导出长度必须与参照一致"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据 ③（**采样率不匹配的处理方式**）：`44.1 kHz` 的 FLAC **不重采样**，
/// 原样上报文件自己的采样率；播放速率的换算交给引擎既有口径。
///
/// ## 为什么是"报数"而不是"重采样"（择优理由）
///
/// * **既有口径就是这样**：WAV 腿从来把文件采样率原样放进 `InMemorySample::sample_hz`，
///   由 `yeban_engine::sfz_instrument` 的 `RenderRates::new(source.sample_hz, engine_hz)`
///   把它算进**播放速率**（`playback_rate = pitch_ratio × sample_hz / engine_hz`，
///   见 `crates/yeban-mcp/tests/sfz_sample_files.rs` 的 `a_44k_sample_file_plays_at_its_own_rate...`）。
///   ⇒ 新腿沿用**同一个**口径 ⇒ 两条腿对同一份 PCM 给同一个音频结果。
/// * **重采样会制造第二条口径**：`rubato` 一进来，44.1 kHz 的样本值就变成"解码 × sinc"，
///   而 WAV 腿对 44.1 kHz 的 WAV **从不重采样**（只把 `sample_hz` 原样上报）⇒ 两条腿对
///   同一份 PCM 会给出**不同**的样本值（`#254` 之后两条腿的**分母**已统一，所以差别会
///   **只**来自重采样本身）。
///   ⛔ 本单**不动**这条口径（判据 ① 钉的是同分母后的逐位相同）。
///
/// ## 判定量
///
/// * `sample_hz == 44100`（⛔ 不是 `48000`）；
/// * 帧数 `4800`（⛔ 不是 `4800 × 48000 / 44100 ≈ 5224` ⇒ 读层没重采样）；
/// * `describe` 含 `rate=44100`；
/// * 端到端仍**出声**（`nonzero > 0`）且 `decoded_samples == 1`。
#[test]
fn a_flac_at_another_sample_rate_is_read_at_its_own_rate() {
    let dir = scratch("flac-44k");
    std::fs::write(
        dir.join("m44.flac"),
        fixture(
            M44_FLAC_HEX,
            "f260a5ccb29cd1ae06279ed8562322009c423c51ac71869dfc770a64eb6cf65f",
            134,
        ),
    )
    .expect("写 44.1 kHz FLAC 夹具");

    let asset =
        decode_path(&dir.join("m44.flac"), &DecodeOptions::default()).expect("44.1k FLAC 必须能解");
    assert_eq!(asset.sample_rate(), 44_100, "夹具必须是 44.1 kHz");

    let (sample, frames, describe) =
        read_sample_file(&dir.join("m44.flac")).expect("44.1k FLAC 必须可读");
    assert_eq!(
        sample.sample_hz, 44_100.0,
        "⭐ 上报**文件自己**的采样率（⛔ 不是工程采样率 48000）"
    );
    assert_eq!(
        frames, 4_800,
        "⭐ 读层**不重采样**（若见 5224 一类数字 ⇒ 有人在这里插了 sinc）"
    );
    assert_eq!(sample.pcm.len(), 4_800);
    assert!(
        describe.contains("rate=44100") && describe.contains("frames=4800"),
        "描述必须写文件采样率与折后帧数：{describe}"
    );

    let out = render_bound(&dir, "m44.sfz", "m44.flac");
    let (_, report) = &out.sfz_decoded[0];
    println!(
        "[app-sfz-decoded/44k] sample_hz={} frames={frames} describe={describe} \
         nonzero={} report={}",
        sample.sample_hz,
        nonzero(&out.samples),
        report.summary()
    );
    assert_eq!(report.decoded_samples, 1, "44.1k FLAC 必须真的被解码");
    assert!(
        nonzero(&out.samples) > 0,
        "44.1k 样本必须照常发声（播放速率由引擎按 sample_hz/engine_hz 换算）"
    );

    std::fs::remove_dir_all(&dir).ok();
}
