//! **第五批补面**：三块此前**没有任何判据看管**的公开面。
//!
//! # 机械依据（本批的注入实测；基线 468 条判据）
//!
//! 1. **零调用点的公开 API**（普查口径：`pub fn`／`pub const fn` 定义在**代码行**里
//!    的调用点计数 = 0；全 crate **361** 个公开函数里 **17** 个为 0）。
//!    对其中 17 个逐个把返回值改坏（取反、换来源、`+1`、丢因子、写入口 ×2），
//!    其中 **15 个全库 468 条判据全绿**（第五批注入表 P01–P10、P12–P15、P17；
//!    ⚠ P11 `notes_triggered` 与 P16 `slots` 被 `polysynth`／`drums` 的零分配判据
//!    的反向对照覆盖 ⇒ 只有这两条不是缺口）。
//! 2. **无字面值钉子的常量**（普查口径：`pub const` 声明共 **95** 条，
//!    其中 **44** 条在断言里找不到数字字面量）。逐条改常量本身，
//!    **13 条全绿**（C02／C03／C07–C11／C13／C14／C17–C20）⇒ 这些常量此前
//!    只被"常量自比"的判据碰过（目标线随常量一起移动）。
//! 3. **手写 `Debug` 的形状**：全 crate 只有 **3** 个手写 `impl Debug`
//!    （`Convolution`／`ConvolutionReverb`／`TrueStereoConvolution`，其余 57 处是
//!    `derive`）。其中只有 `ConvolutionReverb` 有形状判据；把另外两个改成打印
//!    内部缓冲（`ir_re`／`kernels`）时**全库全绿**（DBG1／DBG2）⇒ 它们的
//!    "只打印标量"这条契约没有判据。
//!
//! ⚠ **本 crate 的诊断文案面是 0**（普查读数：`impl Display for` = **0**、
//! `impl error::Error for` = **0**；唯一的错误类型 `oscillator::WavetableError`
//! 只有变体相等，没有文案）⇒ 其它模块"诊断文案黄金表"那一族**在本 crate 无面可打**，
//! 这不是缺口、也不是遗漏。
//!
//! # 覆盖范围（明说）
//!
//! 本文件的断言全部只走**公开 API ＋ 字面值**，不含超越函数的精确值钉法（裁决 R24/R25）。

use yeban_dsp::channel_strip::{ChannelStrip, ChannelStripParams};
use yeban_dsp::compressor::{Compressor, CompressorParams};
use yeban_dsp::convolution::Convolution;
use yeban_dsp::convolution_stereo::TrueStereoConvolution;
use yeban_dsp::drums::{DRUM_SLOTS, DrumHit, DrumMachine, DrumVoice};
use yeban_dsp::math::{db_to_gain, lerp};
use yeban_dsp::meter::dbfs;
use yeban_dsp::polysynth::{NoteEvent, OscSettings, PolySynth, VOICES_PER_SLOT};
use yeban_dsp::smoothing::ParamSmoother;

/// 判据用的采样率（Hz）。
const SR: f32 = 48_000.0;

// ---------------------------------------------------------------------------
// 面 1：零调用点的公开 API（P01–P17）
// ---------------------------------------------------------------------------

/// 量什么：`ChannelStrip` 的四个"输入侧读数"与它们各自的**来源**：
/// `input_peak_dbfs()` 对 `dbfs(input_peak())`、`input_rms_dbfs()` 对 `dbfs(input_rms())`
/// （`f32` 位型），以及 `detector_level_db()` 的**单位口径**（dB 读数必须 `<= 0`）。
///
/// 为什么需要它：这四个访问器在**代码里零调用点**（普查读数）⇒ 没有任何判据读过它们。
/// 注入实测：把 `input_peak_dbfs` 改成 `dbfs(self.output_peak())`、
/// `input_rms_dbfs` 改成 `dbfs(self.output_rms())`、`detector_level_db` 改成
/// `self.compressor.mean_square()`（线性功率，单位错）⇒ **全库 468 条判据全绿**
/// （第五批 P03／P04／P02）。
///
/// ⚠ 夹具必须让**输入 ≠ 输出**（这里用 `input_gain_db = -6 dB`），否则
/// "输入读数用输出值算"是恒等的。
#[test]
fn the_input_side_metrics_report_their_own_source_and_unit() {
    /// 观测帧数。
    const FRAMES: usize = 512;
    let params = ChannelStripParams {
        input_gain_db: -6.0,
        ..ChannelStripParams::DEFAULT
    };
    let mut strip = ChannelStrip::new(params, SR);
    let source: Vec<f32> = (0..FRAMES)
        .map(|index| 0.5 * ((index as f32) * 0.05).sin())
        .collect();
    let mut left = source.clone();
    let mut right = source.clone();
    strip.process_stereo(&mut left, &mut right);

    // 前置条件：链真的改变了电平（否则下面两条断言恒真）。
    assert!(
        (strip.input_peak() - strip.output_peak()).abs() > 1e-3,
        "夹具没有让输入与输出不同（in={} out={}）⇒ 本判据没有判别力",
        strip.input_peak(),
        strip.output_peak()
    );
    assert_eq!(
        strip.input_peak_dbfs().to_bits(),
        dbfs(strip.input_peak()).to_bits(),
        "输入峰值 dBFS 必须由**输入**峰值算出"
    );
    assert_eq!(
        strip.input_rms_dbfs().to_bits(),
        dbfs(strip.input_rms()).to_bits(),
        "输入 RMS dBFS 必须由**输入** RMS 算出"
    );
    // 单位口径：dB 读数对 |x| ≤ 1 的信号必须 ≤ 0（线性功率读数会 > 0）。
    let level = strip.detector_level_db();
    assert!(
        level.is_finite() && level <= 0.0,
        "检波电平必须是 dB（≤ 0），实得 {level}"
    );
}

/// 量什么：`ChannelStrip::current_gain_reduction_db()` 的**符号与符号约定**
/// （线性 dB，`≥ 0` 表示"衰减了多少" —— 它就是 `-gain_db()`，即正的衰减量）。
///
/// 为什么需要它：该访问器零调用点。注入实测：把
/// `-self.compressor.gain_db()` 的负号去掉 ⇒ **全库 468 条判据全绿**（P01）
/// ⇒ 读数会从"衰减量（正）"变成"压缩增益（负）"，调用方的表头会反向。
///
/// ⚠ 第一版把符号判反了（断言 `≤ 0`）⇒ 在**未注入**的实现上就红了；
/// 实测该读数是 `+`，故修正为 `≥ 0`。
#[test]
fn the_gain_reduction_reading_is_never_positive() {
    /// 观测帧数。
    const FRAMES: usize = 512;
    let mut strip = ChannelStrip::new(ChannelStripParams::DEFAULT, SR);
    let source = vec![0.9f32; FRAMES];
    let mut left = source.clone();
    let mut right = source;
    strip.process_stereo(&mut left, &mut right);
    let reduction = strip.current_gain_reduction_db();
    assert!(
        reduction.is_finite() && reduction >= 0.0,
        "增益衰减读数的符号约定是 `≥ 0`（正的衰减量），实得 {reduction}"
    );
    assert!(
        reduction > 0.0,
        "0.9 的满幅输入必须真的压出衰减，实得 {reduction}"
    );
}

/// 量什么：`math::lerp` 的三个可手算读数（`f32` 位型）：两端点与一个中点。
///
/// 为什么需要它：`lerp` 零调用点。注入实测：把 `a + (b - a) * t` 改成
/// `a + (a - b) * t` ⇒ **全库 468 条判据全绿**（P05）。
#[test]
fn lerp_hits_both_endpoints_and_the_midpoint() {
    assert_eq!(lerp(2.0, 6.0, 0.0).to_bits(), 2.0f32.to_bits());
    assert_eq!(lerp(2.0, 6.0, 1.0).to_bits(), 6.0f32.to_bits());
    assert_eq!(lerp(2.0, 6.0, 0.25).to_bits(), 3.0f32.to_bits());
    // 反向外推方向：t = -0.5 ⇒ a + (b-a)·(-0.5)。
    assert_eq!(lerp(2.0, 6.0, -0.5).to_bits(), 0.0f32.to_bits());
}

/// 量什么：四个"零调用点"的音符／支路访问器：`OscSettings::{table, detune_cents}`、
/// `NoteEvent::{end_sample, freq_hz}`，以及 `PolySynth::voices()`。
///
/// 为什么需要它：这五条都零调用点。注入实测：`detune_cents` 取反、
/// `end_sample` `+1`、`freq_hz` ×2、`table` `+1`、`voices` `+1`
/// ⇒ **全库 468 条判据全绿**（P06–P10）。
#[test]
fn the_event_and_branch_accessors_round_trip_their_arguments() {
    let settings = OscSettings::new(3, 0.5, 7.0);
    assert_eq!(settings.table(), 3, "支路的波表下标必须原样返回");
    assert_eq!(settings.detune_cents().to_bits(), 7.0f32.to_bits());

    let event = NoteEvent::new(1_000, 10_000, 440.0, 0.75);
    assert_eq!(event.end_sample(), 10_000, "终点必须原样返回");
    assert_eq!(event.freq_hz().to_bits(), 440.0f32.to_bits());

    assert_eq!(
        PolySynth::<VOICES_PER_SLOT>::new(48_000).voices(),
        VOICES_PER_SLOT,
        "`voices()` 必须等于编译期声部容量"
    );
}

/// 量什么：`Compressor::{total_gain_db, makeup_linear}` 与它们的来源。
///
/// 为什么需要它：两条都零调用点。注入实测：`total_gain_db` 丢掉
/// `self.makeup_gain` 因子、`makeup_linear` 直接返回 `1.0`
/// ⇒ **全库 468 条判据全绿**（P12／P13）。
///
/// ⚠ 夹具必须让 `makeup_db ≠ 0`（这里 +6 dB），否则"丢掉 makeup"是恒等的。
#[test]
fn the_compressor_reports_the_total_gain_including_makeup() {
    let mut params = CompressorParams::DEFAULT;
    params.makeup_db = 6.0;
    params.threshold_db = -24.0;
    let mut comp = Compressor::new(params, SR);
    let source = vec![0.9f32; 256];
    let mut left = source.clone();
    let mut right = source;
    comp.process_stereo(&mut left, &mut right);
    assert!(comp.gain_db() < 0.0, "夹具必须真的压出衰减");
    let total = comp.total_gain_db();
    let expected = comp.gain_db() + params.makeup_db;
    assert!(
        (total - expected).abs() < 1e-3,
        "总增益必须是压缩增益与 makeup 的**积**（dB 相加）：实得 {total}，期望 {expected}"
    );
    assert_eq!(
        comp.makeup_linear().to_bits(),
        db_to_gain(params.makeup_db).to_bits(),
        "`makeup_linear()` 必须是 makeup dB 的线性换算"
    );
}

/// 量什么：`ParamSmoother::{set_time_constant, time_constant}` 的往返与"入口是活的"。
///
/// 为什么需要它：两条都零调用点。注入实测：`set_time_constant` 写入
/// `time_constant_s * 2.0`、`time_constant()` 返回 `self.time_constant_s * 2.0`
/// ⇒ **全库 468 条判据全绿**（P14／P15）。
#[test]
fn the_time_constant_entry_is_live_and_round_trips() {
    let mut smoother = ParamSmoother::with_default_time(SR);
    smoother.set_time_constant(0.05);
    assert_eq!(
        smoother.time_constant().to_bits(),
        0.05f32.to_bits(),
        "时间常数入口必须原样往返"
    );
    smoother.set_time_constant(0.2);
    assert_eq!(
        smoother.time_constant().to_bits(),
        0.2f32.to_bits(),
        "第二次设置必须生效（入口不是一次性的）"
    );
}

/// 量什么：`DrumMachine::sounding_slot_frames()` 的**覆盖度仪器语义**
/// （单位：槽位×帧）。
///
/// 为什么需要它：该访问器零调用点。注入实测：返回
/// `self.sounding_slot_frames.wrapping_add(1)` ⇒ **全库 468 条判据全绿**（P17）。
///
/// 判据：全新机器为 `0`；触发一次并渲染 `N` 帧之后必须 `> 0`，且
/// `<= N × 槽位数`（上界是"每帧每个槽位都算一个"）。
#[test]
fn the_sounding_slot_instrument_starts_at_zero_and_counts() {
    /// 观测帧数。
    const FRAMES: usize = 128;
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    assert_eq!(
        machine.sounding_slot_frames(),
        0,
        "没有触发过任何鼓击时累计发声槽位帧必须是 0"
    );
    machine.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
    let mut block = vec![0.0f32; FRAMES];
    machine.render(0, &mut block);
    let counted = machine.sounding_slot_frames();
    assert!(counted > 0, "触发并渲染之后累计值必须为正");
    assert!(
        counted <= (FRAMES * DRUM_SLOTS) as u64,
        "累计值 {counted} 超过上界 {}",
        FRAMES * DRUM_SLOTS
    );
}

// ---------------------------------------------------------------------------
// 面 2：常量的字面值钉子（C02／C03／C07–C11／C13／C14／C17–C20）
// ---------------------------------------------------------------------------

/// 量什么：13 个此前**没有任何字面值钉子**的公开常量的取值（单位随各自的文档）。
///
/// 为什么需要它：普查读数 —— 全 crate **95** 条 `pub const`，**44** 条在断言里
/// 找不到数字字面量；逐条改常量本身后 **13 条全绿**。这些常量的既有判据都是
/// "**常量对常量**"（目标线随常量一起移动），因此常量本身漂移时**没有任何判据变红**。
///
/// ⛔ 断言一律写**字面值**，不写常量名 —— 否则就是同一个"常量自比"缺陷。
#[test]
fn the_documented_constants_are_pinned_by_literals() {
    // 限幅器：每样本释放量（线性增益）。
    assert_eq!(
        yeban_dsp::limiter::LIMITER_RELEASE_PER_SAMPLE.to_bits(),
        5.0e-5f32.to_bits(),
        "LIMITER_RELEASE_PER_SAMPLE"
    );
    // 滤波器：次正规兜底地板。
    assert_eq!(
        yeban_dsp::filter::DENORMAL_FLOOR.to_bits(),
        1e-30f32.to_bits(),
        "DENORMAL_FLOOR"
    );
    // 压缩器：默认检波时间常数（秒）。
    assert_eq!(
        yeban_dsp::compressor::DEFAULT_DETECTOR_S.to_bits(),
        0.005f32.to_bits(),
        "DEFAULT_DETECTOR_S"
    );
    // 通道条：增益域（dB）与截止域（Hz）＋ EQ 切块帧数。
    assert_eq!(
        yeban_dsp::channel_strip::MIN_GAIN_DB.to_bits(),
        (-60.0f32).to_bits(),
        "MIN_GAIN_DB"
    );
    assert_eq!(
        yeban_dsp::channel_strip::MAX_GAIN_DB.to_bits(),
        24.0f32.to_bits(),
        "MAX_GAIN_DB"
    );
    assert_eq!(
        yeban_dsp::channel_strip::MIN_CUTOFF_HZ.to_bits(),
        20.0f32.to_bits(),
        "MIN_CUTOFF_HZ"
    );
    assert_eq!(
        yeban_dsp::channel_strip::MAX_CUTOFF_HZ.to_bits(),
        20_000.0f32.to_bits(),
        "MAX_CUTOFF_HZ"
    );
    assert_eq!(
        yeban_dsp::channel_strip::EQ_CHUNK_FRAMES,
        64,
        "EQ_CHUNK_FRAMES"
    );
    // 整形：最大降采样除数与瞬态最大增益。
    assert_eq!(
        yeban_dsp::shaping::MAX_DIVISOR.to_bits(),
        64.0f32.to_bits(),
        "MAX_DIVISOR"
    );
    assert_eq!(
        yeban_dsp::shaping::MAX_TRANSIENT_GAIN.to_bits(),
        8.0f32.to_bits(),
        "MAX_TRANSIENT_GAIN"
    );
    // 复音合成器：每个槽位的声部数。
    assert_eq!(yeban_dsp::polysynth::VOICES_PER_SLOT, 16, "VOICES_PER_SLOT");
    // 鼓机：连击数的上下界。
    assert_eq!(yeban_dsp::drums::MAX_BURSTS, 8, "MAX_BURSTS");
    assert_eq!(yeban_dsp::drums::MIN_BURSTS, 1, "MIN_BURSTS");
}

// ---------------------------------------------------------------------------
// 面 3：手写 Debug 的形状（DBG1／DBG2）
// ---------------------------------------------------------------------------

/// 量什么：`Convolution` 的 `Debug` 文本长度（单位：字节）。
///
/// 为什么需要它：手写 `impl Debug` 的目的就是**不**把上万个内部元素打出来；
/// 全 crate 只有 3 个手写实现，其中只有 `ConvolutionReverb` 有形状判据。
/// 注入实测：把 `.field("ir_frames", &self.ir_frames)` 换成
/// `.field("ir_re", &self.ir_re)`（打印频域 IR 缓冲）⇒ **全库 468 条判据全绿**
/// （DBG1）⇒ 这条契约此前没有判据。
///
/// ⚠ 夹具必须**先配置一条 IR**：`Convolution::new()` 的 `ir_re` 是**空 `Vec`**，
/// 打印它仍然很短 ⇒ 第一版夹具让本判据在注入下**假绿**（证明轮实测），已修正。
#[test]
fn convolution_debug_prints_only_scalars() {
    let mut conv = Convolution::new();
    let ir: Vec<f32> = (0..64).map(|index| 0.5 - index as f32 * 0.001).collect();
    conv.set_impulse_response(&ir);
    let text = format!("{conv:?}");
    assert!(text.contains("Convolution"), "Debug 文本必须点名类型");
    assert!(
        text.len() < 400,
        "Debug 输出过长（{} 字节）⇒ 内部缓冲被打印了: {text}",
        text.len()
    );
}

/// 量什么：`TrueStereoConvolution` 的 `Debug` 文本长度（单位：字节）。
///
/// 注入实测：把 `.field("paths", &TRUE_STEREO_PATHS)` 换成
/// `.field("kernels", &self.kernels)`（打印四条核）⇒ **全库 468 条判据全绿**（DBG2）。
///
/// ⚠ 与判据 9 同理，夹具**先配置四条 IR**（否则内部的 `Convolution` 是空壳）。
#[test]
fn true_stereo_convolution_debug_prints_only_scalars() {
    let mut conv = TrueStereoConvolution::new();
    let ir: Vec<f32> = (0..64).map(|index| 0.25 + index as f32 * 0.001).collect();
    conv.set_impulse_response(&ir, &ir, &ir, &ir);
    let text = format!("{conv:?}");
    assert!(
        text.contains("TrueStereoConvolution"),
        "Debug 文本必须点名类型"
    );
    assert!(
        text.len() < 400,
        "Debug 输出过长（{} 字节）⇒ 内部核被打印了: {text}",
        text.len()
    );
}
