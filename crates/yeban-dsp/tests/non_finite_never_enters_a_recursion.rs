//! **类判据**：非有限样本**不得**进入任何**递归**处理器的状态 [ARCH-RT-001] [ARCH-DET-001]。
//!
//! # 这一条在数什么
//!
//! 对象：本 crate 里所有"把调用方给的样本混进**递归**状态"的处理器。
//! 量：一次坏样本（`NaN` / `±∞`）之后，再喂一段**干净**音频，输出是否
//!   ① 全部有限，且 ② 与"位置 0 那个样本换成 `0.0`"的对照运行**逐位相同**。
//! 单位：非有限输出样本数（个）/ 与对照不同的样本数（个）。
//!
//! # 为什么这两条要一起断言（只写 ① 会漏掉一类真实的坏）
//!
//! 递归状态里写进 `NaN`／`±∞` 之后，`state = a·state + b·x` 的每一次迭代都把它
//! 原样留下（`NaN · 0.9 = NaN`、`∞ · 0.5 = ∞`）⇒ 输入恢复干净也回不来，
//! `reset()` 之外没有出路。**但症状不总是非有限输出**：`math::exp2` 对非有限输入
//! 返回 `0`，因此 transient shaper 中毒后增益恒为 `0`、湿信号恒为 `0` ——
//! 输出**有限却永久错**。断言 ② 抓得到它，断言 ① 抓不到。
//!
//! # 覆盖范围与**诚实边界**
//!
//! 覆盖（本轮补齐守卫的 8 个类型，全部走**公共 API**）：
//! `reverb::Reverb`、`delay::Delay`、`comb::CombFilter`、`filter::LadderFilter`、
//! `shaping::BitCrusher`、`shaping::ShapingEq`、`shaping::TransientShaper`、
//! `noise::NoiseGen`。
//!
//! 不在射程内（**不是**遗漏，各有理由）：
//!
//! 1. `limiter::Limiter` / `channel_strip::ChannelStrip` / `compressor::Compressor`
//!    在本次改动**之前**就各有入口守卫，且三者的口径**互不相同**（`nan_to_zero`
//!    保留 `±∞`；`sanitize_sample` 把幅度也钳到 `±16`；`finite_or_zero` 归零
//!    `NaN` 与 `±∞`）。本文件只断言它们的 `NaN` 分支与"归零对照"逐位相同 ——
//!    `±∞` 那两档是它们**故意的**口径（见各自的模块文档），不重定义。
//! 2. `convolution` / `convolution_stereo` / `convolution_reverb` **明确**把逐样本
//!    净化写成调用方的职责，且它们的频域延迟线**没有递归**：`NaN` 写进去会在
//!    `partitions` 个块之内被冲掉。因此不在这里。
//! 3. `oversampler2x` 的尾部是 FIR 历史，同样无递归、会自愈。
//! 4. `meter` / `loudness` 是**读取器**，不是信号路径；`meter` 自己已有
//!    `sanitize_sample`。
//!
//! # 旁通路径（⚠ 明说的边界，不是漏洞）
//!
//! `Reverb` / `Delay` / `CombFilter` 在**未配置**（或湿路不可闻）时提前返回，
//! 那条路径**逐位直通** —— 包括非有限样本。守卫在提前返回**之后**，因此它管的是
//! "有没有样本进入递归"：旁通时一个样本都没进去，没有东西可毒化。
//! 这条边界由 [`bypass_paths_stay_bit_exact_passthrough`] 显式钉住。

use yeban_dsp::channel_strip::{ChannelStrip, ChannelStripParams};
use yeban_dsp::comb::CombFilter;
use yeban_dsp::compressor::{Compressor, CompressorParams};
use yeban_dsp::delay::{Delay, DelayParams};
use yeban_dsp::filter::LadderFilter;
use yeban_dsp::limiter::Limiter;
use yeban_dsp::noise::{NoiseColour, NoiseGen};
use yeban_dsp::reverb::{Reverb, ReverbParams};
use yeban_dsp::shaping::{
    BitCrusher, CrushParams, EqParams, ShapingEq, TransientParams, TransientShaper,
};

/// 采样率（Hz）。全部夹具共用。
const SR: f32 = 48_000.0;

/// 干净尾巴的长度（帧）。取 20 000 是有理由的：它**长于**本文件里最长的反馈环
/// （`CombFilter` 在 30 Hz 下 1 600 帧、Freeverb 最长梳状线 ≈1 760 帧、
/// `Delay` 的 10 ms = 480 帧），因此"坏样本绕回输出"这件事**必然**落在窗口内。
/// 窗口短于环长时判据会假绿（实测：64 ＋ 512 帧的窗口下 `Reverb` 报 0 个非有限，
/// 而 20 000 帧的窗口报 17 813 个）。
const TAIL_FRAMES: usize = 20_000;

/// 非有限激励表（正对照：表里必须真的含非有限值）。
const HOSTILE: [f32; 3] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

/// 通用断言：`run(hostile)` 必须**全部有限**，且与 `run(0.0)` **逐位相同**。
///
/// `run` 是一个闭包而不是一个函数指针，理由只有一个：每个夹具的参数表不同。
fn assert_never_leaks(name: &str, run: impl Fn(f32) -> Vec<f32>) {
    // 正对照：夹具确实注入了一个非有限样本。
    assert!(
        HOSTILE.iter().any(|v| !v.is_finite()),
        "激励表必须含非有限值"
    );
    let control = run(0.0);
    assert!(
        control.iter().all(|s| s.is_finite()),
        "{name}: 对照运行（位置 0 = 0.0）自己就产生了非有限输出 ⇒ 判据无法归因"
    );
    for hostile in HOSTILE {
        let got = run(hostile);
        assert_eq!(got.len(), control.len(), "{name}: 两次运行的长度必须相同");

        let non_finite = got.iter().filter(|s| !s.is_finite()).count();
        assert_eq!(
            non_finite,
            0,
            "{name}: 输入 {hostile:?} 之后仍有 {non_finite} 个非有限输出样本（共 {}）",
            got.len()
        );

        let first_diff = got
            .iter()
            .zip(control.iter())
            .position(|(a, b)| a.to_bits() != b.to_bits());
        if let Some(index) = first_diff {
            panic!(
                "{name}: 输入 {hostile} 的运行从第 {index} 个样本起与'该样本为 0.0'的对照分叉：\
                 实得 {:?}（位 {:#010x}），对照 {:?}（位 {:#010x}）",
                got[index],
                got[index].to_bits(),
                control[index],
                control[index].to_bits()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 各处理器的夹具（每个都以"位置 0 的样本"为唯一变量）
// ---------------------------------------------------------------------------

/// 把 `hostile` 放在第 0 帧，后面跟一段干净的确定性激励。
fn excite(len: usize, scale: f32, quantum: u64) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let t = (i as u64 + quantum * 97) % 977;
            scale * (t as f32 / 977.0 - 0.5) * 2.0
        })
        .collect()
}

fn run_reverb(hostile: f32) -> Vec<f32> {
    let mut device = Reverb::new();
    device.set_sample_rate(SR);
    device.set_params(ReverbParams {
        size: 0.5,
        damp: 0.5,
        mix: 0.5,
        width: 1.0,
        predelay: 0.0,
    });
    let mut left = vec![0.0f32; 64];
    let mut right = vec![0.0f32; 64];
    left[0] = hostile;
    right[0] = hostile;
    device.process(&mut left, &mut right);
    let mut left = excite(TAIL_FRAMES, 0.1, 1);
    let mut right = excite(TAIL_FRAMES, 0.1, 1);
    device.process(&mut left, &mut right);
    left
}

fn run_delay(hostile: f32) -> Vec<f32> {
    let mut device = Delay::new();
    device.configure(SR);
    let params = DelayParams {
        time_s: 0.01,
        feedback: 0.5,
        mix: 0.5,
        damp: 0.0,
        ping_pong: false,
    };
    let mut left = vec![0.0f32; 32];
    let mut right = vec![0.0f32; 32];
    left[0] = hostile;
    right[0] = hostile;
    device.process(params, &mut left, &mut right);
    let mut left = excite(TAIL_FRAMES, 0.1, 2);
    let mut right = excite(TAIL_FRAMES, 0.1, 2);
    device.process(params, &mut left, &mut right);
    left
}

fn run_comb(hostile: f32) -> Vec<f32> {
    let mut device = CombFilter::new();
    device.prepare(SR);
    device.tune(SR, 30.0, 0.9);
    let mut input = vec![0.0f32; 64];
    let mut out = vec![0.0f32; 64];
    input[0] = hostile;
    device.process(&input, &mut out);
    let input = excite(TAIL_FRAMES, 0.1, 3);
    let mut out = vec![0.0f32; TAIL_FRAMES];
    device.process(&input, &mut out);
    out
}

fn run_ladder(hostile: f32) -> Vec<f32> {
    let mut device = LadderFilter::new();
    device.configure(SR, 5_000.0, 0.5, 0.0);
    let mut input = vec![0.0f32; 32];
    input[0] = hostile;
    device.process_block(&mut input);
    let mut tail = excite(TAIL_FRAMES, 0.1, 4);
    device.process_block(&mut tail);
    tail
}

fn run_bit_crusher(hostile: f32) -> Vec<f32> {
    let mut device = BitCrusher::new();
    let params = CrushParams {
        bits: 8.0,
        down: 4.0,
        aa: 1.0,
    };
    let n = 4_096;
    let mut left = vec![0.0f32; n];
    let mut right = vec![0.0f32; n];
    left[0] = hostile;
    right[0] = hostile;
    let mut out_l = vec![0.0f32; n];
    let mut out_r = vec![0.0f32; n];
    device.process(&left, &right, &mut out_l, &mut out_r, params, SR);
    let left = excite(n, 0.1, 5);
    let right = excite(n, 0.1, 5);
    let mut wet_l = vec![0.0f32; n];
    let mut wet_r = vec![0.0f32; n];
    device.process(&left, &right, &mut wet_l, &mut wet_r, params, SR);
    wet_l
}

fn run_shaping_eq(hostile: f32) -> Vec<f32> {
    let mut device = ShapingEq::new();
    let params = EqParams {
        low_gain: 6.0,
        low_freq: 200.0,
        mid_gain: 6.0,
        mid_freq: 1_000.0,
        mid_q: 0.9,
        high_gain: 6.0,
        high_freq: 6_000.0,
    };
    let n = 4_096;
    let mut left = vec![0.0f32; n];
    let mut right = vec![0.0f32; n];
    left[0] = hostile;
    right[0] = hostile;
    let mut out_l = vec![0.0f32; n];
    let mut out_r = vec![0.0f32; n];
    device.process(&left, &right, &mut out_l, &mut out_r, params, SR);
    let left = excite(n, 0.1, 6);
    let right = excite(n, 0.1, 6);
    let mut wet_l = vec![0.0f32; n];
    let mut wet_r = vec![0.0f32; n];
    device.process(&left, &right, &mut wet_l, &mut wet_r, params, SR);
    wet_l
}

fn run_transient_shaper(hostile: f32) -> Vec<f32> {
    let mut device = TransientShaper::new();
    let params = TransientParams {
        attack_amt: 1.0,
        sustain_amt: 1.0,
    };
    let n = 4_096;
    let mut left = vec![0.0f32; n];
    let mut right = vec![0.0f32; n];
    left[0] = hostile;
    right[0] = hostile;
    let mut out_l = vec![0.0f32; n];
    let mut out_r = vec![0.0f32; n];
    device.process(&left, &right, &mut out_l, &mut out_r, params, SR);
    let left = excite(n, 0.1, 7);
    let right = excite(n, 0.1, 7);
    let mut wet_l = vec![0.0f32; n];
    let mut wet_r = vec![0.0f32; n];
    device.process(&left, &right, &mut wet_l, &mut wet_r, params, SR);
    wet_l
}

fn run_noise_gen(hostile: f32) -> Vec<f32> {
    let mut device = NoiseGen::new();
    device.set_colour(NoiseColour::Pink);
    let _ = device.process(hostile, SR);
    let tail = excite(4_096, 0.5, 8);
    tail.into_iter().map(|w| device.process(w, SR)).collect()
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

#[test]
fn reverb_never_lets_a_non_finite_sample_into_its_comb_bank() {
    assert_never_leaks("Reverb", run_reverb);
}

#[test]
fn delay_never_lets_a_non_finite_sample_into_its_line() {
    assert_never_leaks("Delay", run_delay);
}

#[test]
fn comb_filter_never_lets_a_non_finite_sample_into_its_line() {
    assert_never_leaks("CombFilter", run_comb);
}

#[test]
fn ladder_filter_never_lets_a_non_finite_sample_into_its_state() {
    assert_never_leaks("LadderFilter", run_ladder);
}

#[test]
fn bit_crusher_never_lets_a_non_finite_sample_into_its_one_poles() {
    assert_never_leaks("BitCrusher", run_bit_crusher);
}

#[test]
fn shaping_eq_never_lets_a_non_finite_sample_into_its_biquads() {
    assert_never_leaks("ShapingEq", run_shaping_eq);
}

/// ⚠ 这一条**不只**钉"输出有限"：transient shaper 中毒后的输出是**有限**的
/// （`exp2` 对非有限输入返回 `0` ⇒ 增益恒 0 ⇒ 湿信号恒 0），只有"与归零对照
/// 逐位相同"这半条抓得到它。实测（守卫撤掉时）：4 096/4 096 个样本与对照不同。
#[test]
fn transient_shaper_never_lets_a_non_finite_sample_into_its_envelopes() {
    assert_never_leaks("TransientShaper", run_transient_shaper);
}

#[test]
fn noise_gen_never_lets_a_non_finite_sample_into_its_colour_state() {
    assert_never_leaks("NoiseGen", run_noise_gen);
}

// ---------------------------------------------------------------------------
// 结构性证据：正对照（判据有判别力）与旁通边界（守卫的射程）
// ---------------------------------------------------------------------------

/// **正对照**：一个**故意不加守卫**的递归一极点。它证明本文件用的
/// "逐位与归零对照比较"这把尺子真的能抓到毒化 —— 若这条不红，上面的判据就是
/// 空断言（在从未注入非有限样本的夹具上永真）。
///
/// 这是**测试自己的**参照实现，不调用本 crate 的任何 DSP 代码。
#[test]
fn positive_control_an_unguarded_recursion_is_detected() {
    fn unguarded_one_pole(hostile: f32) -> Vec<f32> {
        let mut input = vec![0.1f32; 256];
        input[0] = hostile;
        let mut state = 0.0f32;
        input
            .iter()
            .map(|x| {
                state += 0.5 * (x - state);
                state
            })
            .collect()
    }
    let control = unguarded_one_pole(0.0);
    for hostile in HOSTILE {
        let got = unguarded_one_pole(hostile);
        let non_finite = got.iter().filter(|s| !s.is_finite()).count();
        let differs = got
            .iter()
            .zip(control.iter())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        assert!(
            non_finite > 0 || differs > 0,
            "正对照失效：未加守卫的递归在 {hostile:?} 下与非有限对照无差别"
        );
    }
}

/// **旁通边界**：未配置（或湿路不可闻）时三个带旁通的器件是**逐位直通**，
/// 包括非有限样本。守卫在提前返回之后 ⇒ 管的是"有没有样本进入递归"。
///
/// 量什么：输出第 0 个样本与输入第 0 个样本的位型是否相同（单位：`f32` 位型）。
#[test]
fn bypass_paths_stay_bit_exact_passthrough() {
    // Reverb：从未 set_sample_rate ⇒ 未配置。
    {
        let mut device = Reverb::new();
        let mut left = vec![0.0f32; 8];
        let mut right = vec![0.0f32; 8];
        left[0] = f32::NAN;
        right[0] = f32::NEG_INFINITY;
        device.process(&mut left, &mut right);
        assert!(left[0].is_nan(), "未配置的 Reverb 必须逐位直通");
        assert_eq!(right[0].to_bits(), f32::NEG_INFINITY.to_bits());
    }
    // Reverb：已配置但 mix == 0（低于 WET_EPSILON）⇒ 同样的短路。
    {
        let mut device = Reverb::new();
        device.set_sample_rate(SR);
        device.set_params(ReverbParams {
            size: 0.5,
            damp: 0.5,
            mix: 0.0,
            width: 1.0,
            predelay: 0.0,
        });
        let mut left = vec![0.0f32; 8];
        let mut right = vec![0.0f32; 8];
        left[0] = f32::NAN;
        device.process(&mut left, &mut right);
        assert!(left[0].is_nan(), "mix == 0 的 Reverb 必须逐位直通");
    }
    // Delay：从未 configure ⇒ 未配置。
    {
        let mut device = Delay::new();
        let mut left = vec![0.0f32; 8];
        let mut right = vec![0.0f32; 8];
        left[0] = f32::NAN;
        device.process(DelayParams::default(), &mut left, &mut right);
        assert!(left[0].is_nan(), "未配置的 Delay 必须逐位直通");
    }
    // CombFilter：从未 prepare ⇒ 未准备。
    {
        let mut device = CombFilter::new();
        let input = vec![f32::NAN; 8];
        let mut out = vec![7.0f32; 8];
        device.process(&input, &mut out);
        assert_eq!(out, vec![7.0f32; 8], "未准备的 CombFilter 必须不写 out");
    }
}

/// **既有器件的口径**（不重定义，只钉 `NaN` 一档）：`Limiter` / `ChannelStrip` /
/// `Compressor` 在本次改动之前就有入口守卫，`NaN` 与"归零对照"逐位同解。
///
/// 量什么：**两次调用拼起来**的输出（第一次 512 帧 ＋ 第二次 512 帧）。⚠ 只量第二次
/// 会假绿：`Limiter` 的前瞻环只有 33 帧，坏样本在第 33 个输出样本之后就离开环了
/// —— 实测把这个注入（让 `nan_to_zero` 变成恒等）跑在"只量尾巴"的版本上**仍绿**，
/// 拼上第一次调用的输出才变红。
///
/// ⚠ `±∞` **不**在这一条里：三者的口径互不相同且各自**故意**如此
/// （`limiter::nan_to_zero` 保留 `±∞`；`channel_strip` 的入口把它钳到 `±16`；
/// `compressor` 归零）。把它们塞进同一个断言会是在**改写**既有口径。
#[test]
fn the_three_pre_existing_devices_agree_on_the_nan_control_run() {
    assert_never_leaks("Limiter", |hostile| {
        let mut device = Limiter::new();
        let mut left = vec![0.0f32; 512];
        let mut right = vec![0.0f32; 512];
        // 把 ±∞ 换成 NaN：本判据只覆盖 NaN 那一档（见本判据文档）。
        left[0] = if hostile.is_finite() {
            hostile
        } else {
            f32::NAN
        };
        right[0] = left[0];
        device.process_stereo(&mut left, &mut right);
        let mut observed = left.clone();
        let mut tail_l = excite(512, 0.1, 9);
        let mut tail_r = excite(512, 0.1, 9);
        device.process_stereo(&mut tail_l, &mut tail_r);
        observed.extend_from_slice(&tail_l);
        observed
    });
    assert_never_leaks("ChannelStrip", |hostile| {
        let mut device = ChannelStrip::new(ChannelStripParams::default(), SR);
        let mut left = vec![0.0f32; 512];
        let mut right = vec![0.0f32; 512];
        left[0] = if hostile.is_finite() {
            hostile
        } else {
            f32::NAN
        };
        right[0] = left[0];
        device.process_stereo(&mut left, &mut right);
        let mut observed = left.clone();
        let mut tail_l = excite(512, 0.1, 10);
        let mut tail_r = excite(512, 0.1, 10);
        device.process_stereo(&mut tail_l, &mut tail_r);
        observed.extend_from_slice(&tail_l);
        observed
    });
    assert_never_leaks("Compressor", |hostile| {
        let mut device = Compressor::new(CompressorParams::default(), SR);
        let mut left = vec![0.0f32; 512];
        let mut right = vec![0.0f32; 512];
        left[0] = if hostile.is_finite() {
            hostile
        } else {
            f32::NAN
        };
        right[0] = left[0];
        device.process_stereo(&mut left, &mut right);
        let mut observed = left.clone();
        let mut tail_l = excite(512, 0.1, 11);
        let mut tail_r = excite(512, 0.1, 11);
        device.process_stereo(&mut tail_l, &mut tail_r);
        observed.extend_from_slice(&tail_l);
        observed
    });
}
