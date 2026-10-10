//! **R70② 落地**：为 5 个此前**只有"两次运行互比"（自比）**判据、**零字面摘要锚点**
//! 的模块补上**冻结摘要**。
//!
//! # 为什么需要它（机械读数）
//!
//! 本 crate 有 **12** 条"两次运行互比"型判据（`chunking_does_not_change_output`、
//! `render_is_bit_identical_under_any_block_split`、`the_same_input_twice_is_bit_identical`、
//! `gated_loudness_is_bit_deterministic_and_blocking_invariant`、
//! `splitting_a_stream_into_blocks_is_bit_identical_to_one_pass` …）。
//! ⚠ **"两次运行相同"是自比，不是字节契约**：一个**确定性的**算法改动会让它们**全绿**，
//! 而交付的样本流**已经变了**。按模块查字面摘要锚点（`fnv1a`／64 位字面量）：
//! `convolution_stereo` 14｜`channel_strip` 8｜`convolution` 2｜（`meter` 另有
//! `frozen_pre_hoist…`）**有**锚点；**`compressor`／`polysynth`／`drums`／`loudness`／
//! `oversample` 的计数是 0** ⇒ 本文件为这 5 个各补一条。
//!
//! # ⭐ "上游决定字节"的三分类（逐条判定，写进每条判据的文档）
//!
//! | 模块 | 类别 | 判定依据 | 断言形态 |
//! | :-- | :-- | :-- | :-- |
//! | `oversample` | **③ 我们定输入与运算** | 运行期只有查表与乘加（该文件里的 `sin`／`cos` 全在 `mod tests` 内） | **硬断言**字面摘要（IEEE 精确类） |
//! | `compressor` | **① 上游定运算** | 运行期有 `(-1/(t·fs)).exp()` 与 `10·log10(ms)`（宿主 libm 的超越函数） | **平台感知**：冻结架构 aarch64 硬断言，其它架构**点名跳过并打印**（⚠ 跳过**不是**通过） |
//! | `polysynth` | **①** | 其滤波级在 `configure` 里用 `tan(π·fc/fs)`（宿主 libm） | 同上 |
//! | `drums` | **①** | 其声部滤波级同样是 `LadderFilter::configure`（`tan`） | 同上 |
//! | `loudness` | **①** | K 加权与读数走 `log10`（宿主 libm） | 同上 |
//!
//! ⚠ **本文件不改变任何既有判据的"自比"性质** —— 自比判据作为**结构性质**（分块不变性、
//! 幂等性）仍然有效；本文件补的是它们**缺的那一层**：交付字节的**绝对**锚点。
//!
//! ⚠ **覆盖范围（明说）**：本文件钉的是"该夹具 ⇒ 该摘要"。它**不**覆盖夹具未走到的分支。

use yeban_dsp::compressor::{Compressor, CompressorParams};
use yeban_dsp::drums::{DRUM_SLOTS, DrumHit, DrumMachine, DrumVoice};
use yeban_dsp::loudness::GatedLoudness;
use yeban_dsp::oscillator::{HOLLOW, Wavetable};
use yeban_dsp::oversample::Oversampler2x;
use yeban_dsp::polysynth::{
    NoteEvent, PolySynth, PolySynthParams, PolySynthTables, VOICES_PER_SLOT,
};

/// 判据用的采样率（Hz）。
const SR: f32 = 48_000.0;

/// FNV-1a 64 位：对一串 `f32` 的**位型**求摘要（单位：无符号 64 位整数）。
fn fnv1a64(samples: &[f32]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// 平台上是否是**冻结架构**（裁决 R24：只有它要求跨平台逐位相同）。
const FROZEN_ARCHITECTURE: bool = cfg!(target_arch = "aarch64");

/// 平台感知断言：冻结架构上硬断言；其它架构**点名跳过并打印**（跳过不是通过）。
fn assert_frozen(label: &str, digest: u64, frozen: u64, frames: usize) {
    if FROZEN_ARCHITECTURE {
        assert_eq!(
            digest, frozen,
            "{label}：摘要漂移了（{frames} 帧，实得 {digest:#018x}，冻结 {frozen:#018x}）"
        );
    } else {
        eprintln!(
            "[yeban-dsp/{label}] ⚠ 点名跳过：非冻结架构上不比对绝对摘要 \
             （实得 {digest:#018x}，冻结 {frozen:#018x}，{frames} 帧）—— \
             **跳过不是通过**（裁决 R24）"
        );
    }
}

// ---------------------------------------------------------------------------
// 类别 ③：运行期无超越函数 ⇒ 硬断言
// ---------------------------------------------------------------------------

/// 量什么：`Oversampler2x::process_round_trip` 在 **256 帧**确定输入上的输出位型摘要
/// （FNV-1a 64，单位：无符号 64 位整数）。
///
/// **类别 ③（我们定输入与运算）**：该器件的运行期路径只有**查表与乘加**
/// （`oversample.rs` 里的 `sin`／`cos`／`log10` 全部位于 `mod tests` 内）
/// ⇒ 属 IEEE 精确类 ⇒ **跨架构逐位相同**，可以硬断言字面摘要（裁决 R24 不需要 ulp 预算）。
#[test]
fn the_oversampler_output_digest_is_frozen() {
    /// 观测帧数。
    const FRAMES: usize = 256;
    let input: Vec<f32> = (0..FRAMES)
        .map(|index| ((index * index) % 97) as f32 / 97.0 - 0.5)
        .collect();
    let mut os = Oversampler2x::new();
    let mut out = vec![0.0f32; FRAMES];
    let mut up = vec![0.0f32; 2 * FRAMES];
    let mut scratch = vec![0.0f32; 4_096];
    os.process_round_trip(&input, &mut out, &mut up, &mut scratch);
    let digest = fnv1a64(&out);
    assert_ne!(digest, 0, "夹具必须真的产生输出（摘要不得为 0）");
    assert_eq!(
        digest, 0xc191_0685_3907_21aa,
        "过采样往返的输出摘要漂移了（{FRAMES} 帧）"
    );
}

// ---------------------------------------------------------------------------
// 类别 ①：运行期含宿主 libm 的超越函数 ⇒ 平台感知
// ---------------------------------------------------------------------------

/// 量什么：`Compressor::process_stereo` 在 **512 帧**常量 0.9 上的输出位型摘要。
///
/// **类别 ①（上游定运算）**：运行期含 `(-1/(t·fs)).exp()` 与 `10·log10(mean_square)`
/// —— `exp`／`log10` 走**宿主 libm** ⇒ 按裁决 R24／R25 **只在冻结架构上**比对绝对摘要。
#[test]
fn the_compressor_output_digest_is_frozen_on_the_frozen_architecture() {
    /// 观测帧数。
    const FRAMES: usize = 512;
    let params = CompressorParams::DEFAULT;
    let mut comp = Compressor::new(params, SR);
    let mut left = vec![0.9f32; FRAMES];
    let mut right = vec![0.9f32; FRAMES];
    comp.process_stereo(&mut left, &mut right);
    let digest = fnv1a64(&left) ^ fnv1a64(&right).rotate_left(17);
    assert_ne!(digest, 0, "夹具必须真的产生输出");
    assert_frozen("compressor", digest, 0xb2be_0890_f2d4_dc89, 2 * FRAMES);
}

/// 量什么：`PolySynth` 在 **2 048 帧** A4 上的输出位型摘要。
///
/// **类别 ①**：其滤波级在 `configure` 里用 `tan(π·fc/fs)`（宿主 libm）
/// ⇒ 平台感知。
#[test]
fn the_polysynth_output_digest_is_frozen_on_the_frozen_architecture() {
    /// 观测帧数。
    const FRAMES: usize = 2_048;
    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    synth.set_params(PolySynthParams::new(), &tables);
    synth.note_on(NoteEvent::new(0, 10_000, 440.0, 1.0), &tables);
    let mut out = vec![0.0f32; FRAMES];
    synth.render(&tables, 0, &mut out);
    let digest = fnv1a64(&out);
    assert_ne!(digest, 0, "夹具必须真的产生输出");
    assert_frozen("polysynth", digest, 0x2aea_c93d_99c5_72f5, FRAMES);
}

/// 量什么：`DrumMachine` 触发一次底鼓后 **2 048 帧**的输出位型摘要。
///
/// **类别 ①**：鼓声部的滤波级同样是 `LadderFilter::configure`（`tan`，宿主 libm）
/// ⇒ 平台感知。
#[test]
fn the_drum_machine_output_digest_is_frozen_on_the_frozen_architecture() {
    /// 观测帧数。
    const FRAMES: usize = 2_048;
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    machine.set_params(yeban_dsp::drums::DrumKitParams::DEFAULT);
    machine.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
    let mut out = vec![0.0f32; FRAMES];
    machine.render(0, &mut out);
    let digest = fnv1a64(&out);
    assert_ne!(digest, 0, "夹具必须真的产生输出");
    assert_frozen("drums", digest, 0x04fb_663a_9c48_0348, FRAMES);
}

/// 量什么：`GatedLoudness` 在 **8 192 帧**确定序列上的两个读数（`momentary_lufs`、
/// `short_term_lufs`）的位型摘要。
///
/// **类别 ①**：K 加权与读数走 `log10`（宿主 libm）⇒ 平台感知。
#[test]
fn the_gated_loudness_digest_is_frozen_on_the_frozen_architecture() {
    /// 观测帧数：**4 s**。⚠ 两版夹具的实测修正：`8 192` 帧（`170 ms`）⇒ 两个读数恒
    /// `-inf`（瞬态窗 `400 ms` 未填满）；`96 000` 帧（`2 s`）⇒ `[-4.54, -inf]`
    /// （**短时窗是 3 s**，仍未填满）⇒ 取 。
    const FRAMES: usize = 192_000;
    let mut meter = GatedLoudness::new_48k();
    let left: Vec<f32> = (0..FRAMES)
        .map(|index| ((index * 7) % 101) as f32 / 101.0 - 0.5)
        .collect();
    let right: Vec<f32> = left.iter().rev().copied().collect();
    meter.add_stereo(&left, &right);
    let readings = [meter.momentary_lufs(), meter.short_term_lufs()];
    let digest = fnv1a64(&readings);
    assert!(
        readings.iter().all(|v| v.is_finite()),
        "夹具必须让两个读数都有限（实得 {readings:?}）"
    );
    assert_frozen("loudness", digest, 0x851e_31a9_f14c_3fc7, FRAMES);
}

/// 量什么：`Wavetable::from_recipe` 的表位型摘要（**表本身**也是交付物）。
///
/// **类别 ①（上游定运算）** —— ⚠ **这一条的分类被 CI 当场否证过**：
/// 第一版把它归为"类别 ③（无超越函数）"并**硬断言**，结果 `rust (yeban-dsp)`（Linux x86_64）
/// 与 `windows` 两个作业都在 `the_factory_wavetable_digest_is_frozen` 上红
/// （`工厂波表的表位型摘要漂移了（9 级）`）。
/// 复盘：第一版只 `grep` 了 `from_recipe` 的**本体**，漏了它调用的 helper `render_level`
/// —— 后者对**每个表项**算 `phase.sin()`（`core::f32::consts::TAU * h * i / len`），
/// 而 `sin` 走**宿主 libm** ⇒ 属 4096 ulp 预算类（裁决 R24）。
/// ⇒ 改为平台感知：冻结架构硬断言，其它架构**点名跳过并打印**（跳过不是通过）。
///
/// （同批的 `oversample` 硬断言**通过了**两个平台 ⇒ 那个"类别 ③"的判定成立。）
#[test]
fn the_factory_wavetable_digest_is_frozen_on_the_frozen_architecture() {
    let table = Wavetable::from_recipe(HOLLOW);
    let mut bits: Vec<f32> = Vec::new();
    for level in 0..table.level_count() {
        bits.extend_from_slice(table.level_samples(level));
    }
    let digest = fnv1a64(&bits);
    assert!(!bits.is_empty(), "夹具必须取到非空的表数据");
    assert_frozen(
        "wavetable",
        digest,
        0x3ece_b129_9f3a_6006,
        table.level_count(),
    );
}
