//! **类判据**：非有限**参数**不得写进任何处理器的状态 [ARCH-RT-001] [ARCH-DET-001]。
//!
//! # 这一条在数什么
//!
//! 对象：本 crate 里所有"把调用方给的**标量参数**存进自己的状态"的器件入口
//! （见 §成员表）。
//! 量：一次非有限参数（`NaN` / `±∞`）之后，在一段**干净**激励上的
//!   ① 非有限输出样本数（个），以及
//!   ② 与"同一个参数取**文档化回落值**"的对照运行不同的样本数（个）。
//! 单位：样本数（个）。逐位比较用 `f32::to_bits`。
//!
//! # 为什么"非有限参数"是与"非有限样本"**不同**的一面
//!
//! [`tests/non_finite_never_enters_a_recursion.rs`] 管的是**样本**入口：坏样本从
//! 音频流里进来。参数是另一条路 —— 它不经过那条入口，而是从控制面（快照边界 /
//! 自动化事件 / 快照切换）进来，写进的是**系数**或**递归状态本身**。
//! `ParamSmoother::value` 就是递推本体：`value += (1 − α)·(target − value)`，
//! 目标一旦是 `NaN`，第一个样本就把 `value` 写成 `NaN`，此后每一步都原样留下
//! （`NaN − NaN = NaN`）—— 输入恢复干净也回不来。实时路径上无法报错，只能在
//! 入口回落。
//!
//! # 判据的形态（每一项都是"与对照**逐位**相同"）
//!
//! 每个成员跑两次：
//!
//! - **对照**：同一个夹具，但那个参数**取它的文档化回落值**（回落是"不移动参数"
//!   时，对照就是"从不设这个参数"）；
//! - **中毒**：同一个夹具，先设一组**合法**参数，再把**同一个**参数设成
//!   `NaN` / `+∞` / `−∞`。
//!
//! 两条必须逐位相同。这条断言同时抓两类坏：输出非有限（`NaN` 直接漏出来），
//! 以及**输出有限却永久错**（状态被写成 `NaN` 之后又被别处的兜底救回一个有限值，
//! 但那个值已经不是回落值了）。只断言"有限"抓不到后者。
//!
//! # 成员表（每一项一条判据，名字就是成员名）
//!
//! | 器件 | 参数入口 | 文档化回落 |
//! | :--- | :--- | :--- |
//! | `smoothing::ParamSmoother` | `set_target` | **不移动**（保持当前目标） |
//! | `smoothing::ParamSmoother` | `snap_to` | **不移动** |
//! | `smoothing::ParamSmoother` | `set_time_constant` | 关闭平滑（`α = 0`） |
//! | `oscillator::WavetableOscillator` | `set_phase` | **不移动**（保持当前相位） |
//! | `oscillator::WavetableOscillator` | `set_frequency` | `0.0` Hz |
//! | `oscillator::WavetableOscillator` | `set_sample_rate` | [`MIN_SAMPLE_RATE`] |
//! | `noise::NoiseGen` | `set_corner_hz` | **不移动** |
//! | `limiter::Limiter` | `set_threshold` | [`LIMITER_THRESHOLD`] |
//! | `limiter::Limiter` | `set_release_per_sample` | [`LIMITER_RELEASE_PER_SAMPLE`] |
//! | `meter::LevelDetector` | `set_quanta_per_second` | [`DEFAULT_QUANTA_PER_SECOND`] |
//! | `envelope::Adsr` | `set_params` | 各段 `0.0`（非有限那一段） |
//! | `envelope::Adsr` | `set_release` | `0.0` |
//! | `filter::LadderFilter` | `configure` | 截止 `20.0` Hz、谐振 `0.0`、驱动 `0.0` |
//! | `comb::CombFilter` | `tune` | 频率 [`MIN_FREQ_HZ`]、谐振 `0.0` |
//!
//! ## 不在射程内（**不是**遗漏，各有理由）
//!
//! 1. **`Params` 结构体入口**（`Reverb::set_params`、`Compressor::set_params`、
//!    `ChannelStrip::set_params`、`ConvolutionReverb::set_params`、
//!    `DrumKitParams`、`PolySynthParams`、`CrushParams` / `EqParams` /
//!    `TransientParams`）已有各自的退化参数判据（`reverb_rt_zero_alloc.rs` 的
//!    "一组非有限参数"、`polysynth_render.rs` 的退化参数扫描、`channel_strip` 与
//!    `compressor` 的 `sanitised_clamps_every_field_into_its_documented_domain`）；
//!    它们的回落是**逐字段**的，本文件的"单一标量"夹具形状装不下。
//! 2. **逐块传参入口**（`Delay::process(params, …)`、`ShapingEq` / `BitCrusher` /
//!    `TransientShaper` 的 `process(…, params, sample_rate)`）的回落也已经是逐字段
//!    的钳制表，且调用点就在样本入口那一层（`tests/non_finite_never_enters_a_recursion.rs`
//!    覆盖了它们的**样本**面）。
//! 3. **纯函数**（`math::*`、`block::*`）没有状态可毒化：`lerp(a, b, NaN)` 返回
//!    `NaN` 是一个全定义的纯函数结果，不是"器件带毒"。
//!
//! ⚠ 覆盖范围（明说）：本文件断言的是**器件状态**，不是锁、不是分配、不是 I/O。
//! 运行期零分配的口径在 `tests/parameter_entry_rt_zero_alloc.rs`。

use yeban_dsp::MIN_SAMPLE_RATE;
use yeban_dsp::comb::{CombFilter, MIN_FREQ_HZ};
use yeban_dsp::envelope::Adsr;
use yeban_dsp::filter::LadderFilter;
use yeban_dsp::limiter::{LIMITER_RELEASE_PER_SAMPLE, LIMITER_THRESHOLD, Limiter};
use yeban_dsp::meter::{DEFAULT_QUANTA_PER_SECOND, LevelDetector};
use yeban_dsp::noise::{NoiseColour, NoiseGen};
use yeban_dsp::oscillator::{FACTORY_RECIPES, Wavetable, WavetableOscillator};
use yeban_dsp::smoothing::ParamSmoother;

/// 采样率（Hz）。全部夹具共用。
const SR: f32 = 48_000.0;

/// 非有限激励表（正对照：表里必须真的含非有限值）。
const HOSTILE: [f32; 3] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

/// 确定性激励的第 `i` 个样本（**不用** `Rng` 对象：夹具只依赖下标）。
fn excite(i: usize) -> f32 {
    (i as f32 * 0.017).sin() * 0.7
}

/// 判据本体。
///
/// `control` 是"那个参数取**文档化回落值**"的运行；`poisoned(v)` 是"先设一组合法
/// 参数、再把同一个参数设成 `v`"的运行。两条必须在 `f32::to_bits` 意义上逐位相同。
fn assert_parameter_never_poisons(
    name: &str,
    control: impl Fn() -> Vec<f32>,
    poisoned: impl Fn(f32) -> Vec<f32>,
) {
    assert!(
        HOSTILE.iter().any(|value| !value.is_finite()),
        "激励表必须含非有限值"
    );
    let control = control();
    assert!(!control.is_empty(), "{name}: 对照运行没有样本");
    assert!(
        control.iter().all(|sample| sample.is_finite()),
        "{name}: 对照运行自己就产生了非有限输出 ⇒ 判据无法归因"
    );
    for hostile in HOSTILE {
        let got = poisoned(hostile);
        assert_eq!(got.len(), control.len(), "{name}: 两次运行的长度必须相同");
        let non_finite = got.iter().filter(|sample| !sample.is_finite()).count();
        assert_eq!(
            non_finite,
            0,
            "{name}: 参数 {hostile:?} 之后干净激励里仍有 {non_finite} 个非有限样本（共 {}）",
            got.len()
        );
        let first_diff = got
            .iter()
            .zip(control.iter())
            .position(|(a, b)| a.to_bits() != b.to_bits());
        if let Some(index) = first_diff {
            panic!(
                "{name}: 参数 {hostile:?} 的运行从第 {index} 个样本起与'该参数取文档化回落值'\
                 的对照分叉：实得 {:?}（位 {:#010x}），对照 {:?}（位 {:#010x}）",
                got[index],
                got[index].to_bits(),
                control[index],
                control[index].to_bits()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 正对照：判据有牙
// ---------------------------------------------------------------------------

/// 量什么：本文件的判据在一条**故意写坏**的递归上必须报红（单位：无 —— 这是一条
/// 二值断言）。
///
/// 判据：`catch_unwind` 必须捕到 panic。若捕不到，说明 [`assert_parameter_never_poisons`]
/// 对"参数直接乘进状态"这种最直白的中毒都不敏感 ⇒ 本文件其余各项的"全绿"是假绿。
#[test]
fn the_harness_has_teeth_a_deliberate_poison_is_caught() {
    let caught = std::panic::catch_unwind(|| {
        assert_parameter_never_poisons(
            "故意写坏的递归",
            || vec![0.0f32; 8],
            |hostile| {
                let mut state = 0.0f32;
                let mut out = Vec::with_capacity(8);
                for _ in 0..8 {
                    state = state * 0.5 + hostile;
                    out.push(state);
                }
                out
            },
        );
    });
    assert!(
        caught.is_err(),
        "判据没抓到故意写坏的递归 ⇒ 本文件的'全绿'没有判别力"
    );
}

// ---------------------------------------------------------------------------
// smoothing::ParamSmoother（三项）
// ---------------------------------------------------------------------------

/// 推进 512 个样本。
fn smoother_steps(smoother: &mut ParamSmoother) -> Vec<f32> {
    (0..512).map(|_| smoother.process()).collect()
}

/// 量什么：`ParamSmoother::set_target` 收到 `NaN` / `±∞` 之后，512 个样本的输出
/// （单位：样本数）。
///
/// 判据：与"从不注入非有限目标"的对照**逐位相同**（回落 = 不移动参数）。
/// 注入（实测红行见报告）：去掉 `set_target` 里的 `is_finite` 判断 ⇒ 第 0 个样本即
/// `NaN`。
#[test]
fn a_non_finite_smoother_target_never_moves_the_recursive_state() {
    assert_parameter_never_poisons(
        "ParamSmoother::set_target",
        || {
            let mut smoother = ParamSmoother::with_default_time(SR);
            smoother.snap_to(0.2);
            smoother.set_target(0.9);
            smoother_steps(&mut smoother)
        },
        |hostile| {
            let mut smoother = ParamSmoother::with_default_time(SR);
            smoother.snap_to(0.2);
            smoother.set_target(0.9);
            smoother.set_target(hostile);
            smoother_steps(&mut smoother)
        },
    );
}

/// 量什么：`ParamSmoother::snap_to` 收到 `NaN` / `±∞` 之后，512 个样本的输出
/// （单位：样本数）。
///
/// 判据：与"从不调用 `snap_to`"的对照**逐位相同**（回落 = 不移动参数）。
/// `snap_to` 同时是**唯一**能把一个已被毒化的平滑器救回来的入口（它写 `value`），
/// 因此它自己也不得写进非有限值。
#[test]
fn a_non_finite_snap_never_moves_the_recursive_state() {
    assert_parameter_never_poisons(
        "ParamSmoother::snap_to",
        || {
            let mut smoother = ParamSmoother::with_default_time(SR);
            smoother.snap_to(0.2);
            smoother.set_target(0.9);
            smoother_steps(&mut smoother)
        },
        |hostile| {
            let mut smoother = ParamSmoother::with_default_time(SR);
            smoother.snap_to(0.2);
            smoother.set_target(0.9);
            smoother.snap_to(hostile);
            smoother_steps(&mut smoother)
        },
    );
}

/// 量什么：`ParamSmoother::set_time_constant` 收到 `NaN` / `±∞` 之后，512 个样本的
/// 输出（单位：样本数）。
///
/// 判据：与"显式关闭平滑（`τ = 0`）"的对照**逐位相同** —— 非有限 `τ` 的文档化回落
/// 就是"关闭平滑"（`α = 0`）。
#[test]
fn a_non_finite_smoothing_time_constant_means_smoothing_off() {
    assert_parameter_never_poisons(
        "ParamSmoother::set_time_constant",
        || {
            let mut smoother = ParamSmoother::new(SR, 0.0);
            smoother.snap_to(0.2);
            smoother.set_target(0.9);
            smoother_steps(&mut smoother)
        },
        |hostile| {
            let mut smoother = ParamSmoother::new(SR, 0.005);
            smoother.snap_to(0.2);
            smoother.set_target(0.9);
            smoother.set_time_constant(hostile);
            smoother_steps(&mut smoother)
        },
    );
}

// ---------------------------------------------------------------------------
// oscillator::WavetableOscillator（三项）
// ---------------------------------------------------------------------------

/// 判据用的波表（构造期分配一次；`sample` 本身不分配）。
fn probe_table() -> Wavetable {
    Wavetable::from_recipe(FACTORY_RECIPES[0].1)
}

/// 推进 4 096 个样本。
fn oscillator_block(oscillator: &mut WavetableOscillator, table: &Wavetable) -> Vec<f32> {
    let mut out = vec![0.0f32; 4_096];
    oscillator.process_block(table, &mut out);
    out
}

/// 量什么：`WavetableOscillator::set_phase` 收到 `NaN` / `±∞` 之后，4 096 个样本的
/// 输出（单位：样本数）。
///
/// 判据：与"从不调用 `set_phase`"的对照**逐位相同**（回落 = 不移动参数）。
/// 相位是递归状态：`NaN` 一旦写进去，每一次 `process` 都把它原样留下
/// （`NaN + inc = NaN`）。
/// 注入（实测红行见报告）：去掉 `set_phase` 里的 `is_finite` 判断 ⇒ 4 096 个样本
/// 全部非有限。
#[test]
fn a_non_finite_phase_never_moves_the_oscillator_state() {
    let table = probe_table();
    assert_parameter_never_poisons(
        "WavetableOscillator::set_phase",
        || {
            let mut oscillator = WavetableOscillator::new(SR);
            oscillator.set_frequency(&table, 440.0);
            oscillator.set_phase(0.25);
            oscillator_block(&mut oscillator, &table)
        },
        |hostile| {
            let mut oscillator = WavetableOscillator::new(SR);
            oscillator.set_frequency(&table, 440.0);
            oscillator.set_phase(0.25);
            oscillator.set_phase(hostile);
            oscillator_block(&mut oscillator, &table)
        },
    );
}

/// 量什么：`WavetableOscillator::set_frequency` 收到 `NaN` / `±∞` 之后，4 096 个样本
/// 的输出（单位：样本数）。
///
/// 判据：与"频率显式设成 `0.0` Hz"的对照**逐位相同**（文档化回落 = `0.0` Hz）。
/// ⚠ 这条的对照是**直流**（0 Hz 的振荡器停在起始相位），因此"逐位相同"本身信息量
/// 低；本条真正有牙的是前半句 —— 未加固的实现会让相位变成 `NaN`，于是 4 096 个
/// 样本全部非有限。
#[test]
fn a_non_finite_frequency_falls_back_to_dc() {
    let table = probe_table();
    assert_parameter_never_poisons(
        "WavetableOscillator::set_frequency",
        || {
            let mut oscillator = WavetableOscillator::new(SR);
            oscillator.set_frequency(&table, 0.0);
            oscillator_block(&mut oscillator, &table)
        },
        |hostile| {
            let mut oscillator = WavetableOscillator::new(SR);
            oscillator.set_frequency(&table, 440.0);
            oscillator.set_frequency(&table, hostile);
            oscillator_block(&mut oscillator, &table)
        },
    );
}

/// 量什么：`WavetableOscillator::set_sample_rate` 收到 `NaN` / `±∞` 之后，4 096 个
/// 样本的输出（单位：样本数）。
///
/// 判据：与"采样率显式设成 [`MIN_SAMPLE_RATE`]"的对照**逐位相同**
/// （`math::sanitise_sample_rate` 的回落）。
#[test]
fn a_non_finite_sample_rate_falls_back_to_the_documented_floor() {
    let table = probe_table();
    assert_parameter_never_poisons(
        "WavetableOscillator::set_sample_rate",
        || {
            let mut oscillator = WavetableOscillator::new(SR);
            oscillator.set_frequency(&table, 440.0);
            oscillator.set_sample_rate(MIN_SAMPLE_RATE, &table);
            oscillator_block(&mut oscillator, &table)
        },
        |hostile| {
            let mut oscillator = WavetableOscillator::new(SR);
            oscillator.set_frequency(&table, 440.0);
            oscillator.set_sample_rate(hostile, &table);
            oscillator_block(&mut oscillator, &table)
        },
    );
}

// ---------------------------------------------------------------------------
// noise::NoiseGen
// ---------------------------------------------------------------------------

/// 确定性白噪声源的第 `i` 个样本（**不**用 `Rng` 对象：夹具只依赖下标）。
fn white(i: usize) -> f32 {
    let mixed = (i as u32)
        .wrapping_mul(2_654_435_761)
        .wrapping_add(1_013_904_223);
    (mixed >> 8) as f32 / 8_388_608.0 - 1.0
}

/// 量什么：`NoiseGen::set_corner_hz`（棕噪声的漏积分转折）收到 `NaN` / `±∞` 之后，
/// 4 096 个样本的输出（单位：样本数）。
///
/// 判据：与"从不注入非有限转折频率"的对照**逐位相同**（回落 = 不移动参数）。
#[test]
fn a_non_finite_noise_corner_never_moves_the_corner() {
    let render = |hostile: Option<f32>| -> Vec<f32> {
        let mut generator = NoiseGen::new();
        generator.set_colour(NoiseColour::Brown);
        generator.set_corner_hz(5.0);
        if let Some(value) = hostile {
            generator.set_corner_hz(value);
        }
        (0..4_096)
            .map(|i| generator.process(white(i), SR))
            .collect()
    };
    assert_parameter_never_poisons(
        "NoiseGen::set_corner_hz",
        || render(None),
        |hostile| render(Some(hostile)),
    );
}

// ---------------------------------------------------------------------------
// limiter::Limiter（两项）
// ---------------------------------------------------------------------------

/// 立体声激励的第 `i` 帧（前 256 帧响、后 256 帧轻）。
fn limiter_excitation(i: usize) -> f32 {
    let envelope = if i < 256 { 1.2 } else { 0.05 };
    envelope * excite(i)
}

/// 跑 4 个 128 帧的立体声块，返回左右交错的输出（1024 个数）。
fn limiter_blocks(limiter: &mut Limiter) -> Vec<f32> {
    let mut out = Vec::with_capacity(1_024);
    for block in 0..4 {
        let mut left = [0.0f32; 128];
        let mut right = [0.0f32; 128];
        for (index, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let i = block * 128 + index;
            *l = limiter_excitation(i);
            *r = limiter_excitation(i + 1);
        }
        limiter.process_stereo(&mut left, &mut right);
        out.extend_from_slice(&left);
        out.extend_from_slice(&right);
    }
    out
}

/// 量什么：`Limiter::set_threshold` 收到 `NaN` / `±∞` 之后，4 个立体声块的输出
/// （单位：样本数）。
///
/// 判据：与"阈值显式设成 [`LIMITER_THRESHOLD`]"的对照**逐位相同**。
#[test]
fn a_non_finite_limiter_threshold_falls_back_to_the_spec_constant() {
    assert_parameter_never_poisons(
        "Limiter::set_threshold",
        || {
            let mut limiter = Limiter::new();
            limiter.set_threshold(LIMITER_THRESHOLD);
            limiter_blocks(&mut limiter)
        },
        |hostile| {
            let mut limiter = Limiter::new();
            limiter.set_threshold(0.5);
            limiter.set_threshold(hostile);
            limiter_blocks(&mut limiter)
        },
    );
}

/// 量什么：`Limiter::set_release_per_sample` 收到 `NaN` / `±∞` 之后，4 个立体声块的
/// 输出（单位：样本数）。
///
/// 判据：与"释放量显式设成 [`LIMITER_RELEASE_PER_SAMPLE`]"的对照**逐位相同**。
/// 夹具的后半段是轻信号（窗口峰值掉回阈值之下）⇒ 增益**回升**，释放量因此在这段
/// 上真的参与运算。
#[test]
fn a_non_finite_limiter_release_falls_back_to_the_spec_constant() {
    assert_parameter_never_poisons(
        "Limiter::set_release_per_sample",
        || {
            let mut limiter = Limiter::new();
            limiter.set_release_per_sample(LIMITER_RELEASE_PER_SAMPLE);
            limiter_blocks(&mut limiter)
        },
        |hostile| {
            let mut limiter = Limiter::new();
            limiter.set_release_per_sample(0.3);
            limiter.set_release_per_sample(hostile);
            limiter_blocks(&mut limiter)
        },
    );
}

// ---------------------------------------------------------------------------
// meter::LevelDetector
// ---------------------------------------------------------------------------

/// 量什么：`LevelDetector::set_quanta_per_second` 收到 `NaN` / `±∞` 之后，64 个量子的
/// `(peak_hold, rms_smoothed)` 读数（单位：样本数）。
///
/// 判据：与"每秒量子数显式设成 [`DEFAULT_QUANTA_PER_SECOND`]"的对照**逐位相同**。
#[test]
fn a_non_finite_quantum_rate_falls_back_to_the_default_ballistics() {
    let read = |hostile: Option<f32>| -> Vec<f32> {
        let mut detector = LevelDetector::new();
        detector.set_quanta_per_second(DEFAULT_QUANTA_PER_SECOND);
        if let Some(value) = hostile {
            detector.set_quanta_per_second(value);
        }
        let mut out = Vec::with_capacity(128);
        for quantum in 0..64 {
            let block: Vec<f32> = (0..128)
                .map(|index| excite(quantum * 128 + index) * 0.3)
                .collect();
            let reading = detector.analyze(&block);
            out.push(reading.peak_hold);
            out.push(reading.rms_smoothed);
        }
        out
    };
    assert_parameter_never_poisons(
        "LevelDetector::set_quanta_per_second",
        || read(None),
        |hostile| read(Some(hostile)),
    );
}

// ---------------------------------------------------------------------------
// envelope::Adsr（两项）
// ---------------------------------------------------------------------------

/// 量什么：`Adsr::set_params` 的 **attack** 收到 `NaN` / `±∞` 之后，2 048 个样本的
/// 输出（单位：样本数）。
///
/// 判据：与"attack 显式设成 `0.0` s"的对照**逐位相同**。
#[test]
fn a_non_finite_attack_falls_back_to_zero_seconds() {
    let render = |attack_s: f32| -> Vec<f32> {
        let mut envelope = Adsr::new();
        envelope.set_sample_rate(SR);
        envelope.set_params(attack_s, 0.2, 0.6, 0.3);
        envelope.gate_on();
        (0..2_048).map(|_| envelope.process(true)).collect()
    };
    assert_parameter_never_poisons("Adsr::set_params", || render(0.0), render);
}

/// 量什么：`Adsr::set_release` 收到 `NaN` / `±∞` 之后，128 个按住的样本 ＋ 512 个
/// 松开的样本的输出（单位：样本数）。
///
/// 判据：与"release 显式设成 `0.0` s"的对照**逐位相同**。
#[test]
fn a_non_finite_release_falls_back_to_zero_seconds() {
    let render = |valid_release: f32, hostile: Option<f32>| -> Vec<f32> {
        let mut envelope = Adsr::new();
        envelope.set_sample_rate(SR);
        envelope.set_params(0.001, 0.05, 0.5, valid_release);
        if let Some(value) = hostile {
            envelope.set_release(value);
        }
        envelope.gate_on();
        let mut out: Vec<f32> = (0..128).map(|_| envelope.process(true)).collect();
        out.extend((0..512).map(|_| envelope.process(false)));
        out
    };
    assert_parameter_never_poisons(
        "Adsr::set_release",
        // 对照：release 直接就是文档化回落值 `0.0` s。
        || render(0.0, None),
        // 中毒：先设一个**不同**的合法 release，再注入非有限值。
        |hostile| render(0.3, Some(hostile)),
    );
}

// ---------------------------------------------------------------------------
// filter::LadderFilter
// ---------------------------------------------------------------------------

/// 量什么：`LadderFilter::configure` 的截止 / 谐振 / 驱动三个旋钮**同时**收到
/// `NaN` / `±∞` 之后，512 个样本的输出（单位：样本数）。
///
/// 判据：与"三个旋钮显式设成各自的回落值（`20.0` Hz / `0.0` / `0.0`）"的对照
/// **逐位相同**。
#[test]
fn a_non_finite_ladder_configuration_falls_back_to_the_lower_corner() {
    let render = |cutoff: f32, resonance: f32, drive: f32| -> Vec<f32> {
        let mut filter = LadderFilter::new();
        filter.configure(SR, cutoff, resonance, drive);
        (0..512).map(|i| filter.process(excite(i))).collect()
    };
    assert_parameter_never_poisons(
        "LadderFilter::configure",
        || render(20.0, 0.0, 0.0),
        |hostile| render(hostile, hostile, hostile),
    );
}

// ---------------------------------------------------------------------------
// comb::CombFilter
// ---------------------------------------------------------------------------

/// 量什么：`CombFilter::tune` 的音高与谐振同时收到 `NaN` / `±∞` 之后，2 048 个样本的
/// 输出（单位：样本数）。
///
/// 判据：与"音高显式设成 [`MIN_FREQ_HZ`]、谐振显式设成 `0.0`"的对照**逐位相同**。
/// 2 048 帧长于回落值下的梳长（`48 000 / 30 = 1 600` 帧），因此延迟线**真的**绕回
/// 输出（窗口短于环长时这条会假绿）。
#[test]
fn a_non_finite_comb_tuning_falls_back_to_the_minimum_frequency() {
    /// 先调 `first`（合法值），再把音高与谐振**同时**设成 `second`，然后跑 8 个块。
    fn render(first: (f32, f32), second: (f32, f32)) -> Vec<f32> {
        let mut comb = CombFilter::new();
        comb.prepare(SR);
        comb.tune(SR, first.0, first.1);
        comb.tune(SR, second.0, second.1);
        let mut out = Vec::with_capacity(2_048);
        for block in 0..8 {
            let input: Vec<f32> = (0..256).map(|index| excite(block * 256 + index)).collect();
            let mut buffer = vec![0.0f32; 256];
            comb.process(&input, &mut buffer);
            out.extend_from_slice(&buffer);
        }
        out
    }
    assert_parameter_never_poisons(
        "CombFilter::tune",
        // 对照：第一调与第二调都是那组**文档化回落值**。
        || render((200.0, 0.7), (MIN_FREQ_HZ, 0.0)),
        |hostile| render((200.0, 0.7), (hostile, hostile)),
    );
}
