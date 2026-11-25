//! `yeban_dsp::polysynth` 的**全局 LFO 接线**判据（`T-FIX-277`）。
//!
//! 背景：`oscillator.rs` 的 [`Lfo`](yeban_dsp::oscillator::Lfo) 早已完整
//! （正弦/三角/方波/锯齿、one-shot），模块文档也写"所有声部共用一个 LFO"，
//! 但 [`PolySynth`](yeban_dsp::polysynth::PolySynth) 与引擎侧对它**零引用** ——
//! 颤音/振幅颤动/滤波器摇摆全都不出声。本文件钉住接线后的行为。
//!
//! # 口径（与 `polysynth_render.rs` 同款）
//!
//! - 采样率固定 **48 kHz**；所有 DFT 都用**矩形窗**，只把"某个频率的 bin
//!   有没有能量"当读数，**不**把它当幅度谱；
//! - LFO 速率固定 **5 Hz**（周期 0.2 s = 9600 帧）；窗口一律从 0.1 s
//!   （包络已到延音，振幅恒定）开始取，窗口长 **0.1 s**（半个 LFO 周期）
//!   或 **0.01 s**（1/20 周期，振幅判据用）；
//! - 频率读数用**线性插值零交叉间隔**（与 `polysynth_render.rs` 同口径）。
//!
//! # 判据
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | L1 | `depth == 0` 时渲染与无 LFO 参数**逐位**相同（默认音色与滤波器启用两种） | LFO 分支在 `depth == 0` 时仍推进/应用 |
//! | L2 | 音高目标：`depth = 100` 音分时半周期窗口基频极差 `> 20 Hz` | 音高目标没接线 |
//! | L3 | 振幅目标：`depth = 0.5` 时 10 ms 窗口峰值极大/极小 `> 1.5×` | 振幅目标没接线 |
//! | L4 | 截止目标：滤波器启用时 7 次谐波（3080 Hz）bin 能量随窗口变化 `> 2×`，且 `debug_lfo_cutoff_hz()` 在相位 0.25 读 ≈4000 Hz、相位 0.5 读 ≈1000 Hz，`depth == 0` 时读数恒等于基值 | 截止目标没接线 / 取值器算错 |
//! | L5 | 同一音符用 64 / 128 / 256 帧三种切法渲染 ⇒ **逐位**相同 | LFO 网格锚在调用块上 |
//! | L6 | 先空渲染再触发与直接触发**逐位**相同（无声帧不推进 LFO） | 空帧也推进 LFO |
//! | L7 | 非法参数钳制/回退与下标映射（`LfoWave::from_index` / `LfoTarget::from_index`） | 钳制写错 / 下标映射错 |

use yeban_dsp::oscillator::{HOLLOW, LfoWave, WaveRecipe};
use yeban_dsp::polysynth::{
    LfoTarget, NoteEvent, PolySynth, PolySynthParams, PolySynthTables, VOICES_PER_SLOT,
};

/// 夹具采样率（`f64` 口径）。
const SR_F64: f64 = 48_000.0;
/// LFO 速率（Hz）：周期 0.2 s = 9600 帧。
const LFO_RATE: f32 = 5.0;

/// 单谐波配方：波形是**纯正弦** ⇒ 每周期恰好 2 次零交叉、
/// 每个采样块的峰值正比于包络。判据用它把"波形"这个变量消掉。
const PURE: WaveRecipe = &[(1, 1.0)];

// ---------------------------------------------------------------------------
// 读数工具（都是测量工具，不是被测代码；口径与 `polysynth_render.rs` 同款）
// ---------------------------------------------------------------------------

/// 单 bin DFT 的幅度（矩形窗；单位：与输入同量纲的幅度）。
fn dft_bin(samples: &[f32], freq_hz: f64, sample_rate: f64) -> f64 {
    let mut re = 0.0f64;
    let mut im = 0.0f64;
    for (index, sample) in samples.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let phase = core::f64::consts::TAU * freq_hz * index as f64 / sample_rate;
        re += f64::from(*sample) * phase.cos();
        im += f64::from(*sample) * phase.sin();
    }
    2.0 * (re * re + im * im).sqrt() / samples.len() as f64
}

/// 线性插值零交叉推出的基频 (Hz)。
fn fundamental_from_crossings(samples: &[f32], sample_rate: f64) -> f64 {
    let mut times: Vec<f64> = Vec::new();
    for (index, pair) in samples.windows(2).enumerate() {
        let (a, b) = (f64::from(pair[0]), f64::from(pair[1]));
        if (a < 0.0) != (b < 0.0) {
            times.push(index as f64 + a / (a - b));
        }
    }
    if times.len() < 2 {
        return 0.0;
    }
    let span = times[times.len() - 1] - times[0];
    if span <= 0.0 {
        return 0.0;
    }
    (times.len() - 1) as f64 / 2.0 / span * sample_rate
}

/// 渲染一个音符（起点 0、终点 `end_sample`、给定配方与参数），返回单声道样本。
fn render_single(
    tables: &PolySynthTables,
    params: PolySynthParams,
    freq_hz: f32,
    end_sample: u64,
    frames: usize,
) -> Vec<f32> {
    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    synth.set_params(params, tables);
    synth.note_on(NoteEvent::new(0, end_sample, freq_hz, 1.0), tables);
    let mut out = vec![0.0f32; frames];
    synth.render(tables, 0, &mut out);
    out
}

/// 按 `chunks` 给出的切分渲染同一段（给定参数）：返回拼接后的样本。
fn render_split_with_params(
    tables: &PolySynthTables,
    params: PolySynthParams,
    notes: &[NoteEvent],
    chunks: &[usize],
) -> Vec<f32> {
    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    synth.set_params(params, tables);
    for note in notes {
        synth.note_on(*note, tables);
    }
    let largest = chunks.iter().copied().max().unwrap_or(1);
    let mut buffer = vec![0.0f32; largest];
    let mut out = Vec::new();
    let mut position = 0u64;
    for chunk in chunks {
        synth.render(tables, position, &mut buffer[..*chunk]);
        out.extend_from_slice(&buffer[..*chunk]);
        position += *chunk as u64;
    }
    out
}

/// 按固定块长 `chunk` 渲染 `total` 帧（切分集合的生成器；与
/// `polysynth_render.rs` 同款，含尾部余数）。
fn chunked(total: usize, chunk: usize) -> Vec<usize> {
    let mut chunks = vec![chunk; total / chunk];
    let rest = total % chunk;
    if rest > 0 {
        chunks.push(rest);
    }
    chunks
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

// ---------------------------------------------------------------------------
// L1：depth == 0 ⇒ 逐位相同
// ---------------------------------------------------------------------------

/// L1a：默认音色 + "狂野"的 LFO 参数（方波 / 100 Hz / 截止目标）但深度 0 ⇒
/// 与无 LFO 参数逐位相同。
#[test]
fn lfo_depth_zero_is_bit_identical_default_voice() {
    let tables = PolySynthTables::from_recipes(&[PURE]);
    let plain = render_single(&tables, PolySynthParams::new(), 440.0, 96_000, 48_000);
    let with_lfo = render_single(
        &tables,
        PolySynthParams::new().with_lfo(LfoWave::Square, 100.0, 0.0, LfoTarget::Cutoff),
        440.0,
        96_000,
        48_000,
    );
    assert_eq!(bits(&plain), bits(&with_lfo));
}

/// L1b：滤波器启用（截止 1000 Hz、旁通关）+ 深度 0 ⇒ 与无 LFO 参数逐位相同。
/// 滤波器是状态机：这里钉住"LFO 分支在深度 0 时一次也不碰滤波器"。
#[test]
fn lfo_depth_zero_is_bit_identical_filter_engaged() {
    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    let params = PolySynthParams::new().with_filter(1000.0, 0.0, 0.0, false);
    let plain = render_single(&tables, params, 440.0, 96_000, 48_000);
    let with_lfo = render_single(
        &tables,
        params.with_lfo(LfoWave::Saw, 100.0, 0.0, LfoTarget::Cutoff),
        440.0,
        96_000,
        48_000,
    );
    assert_eq!(bits(&plain), bits(&with_lfo));
}

// ---------------------------------------------------------------------------
// L2：音高目标 —— 基频随 LFO 周期摆动
// ---------------------------------------------------------------------------

/// L2：`depth = 100` 音分、5 Hz 正弦 ⇒ 半周期窗口（0.1 s）的基频极差 > 20 Hz。
///
/// 闭式参照：窗口平均音分 ≈ ±(2/π)×100 ≈ ±63.7 音分 ⇒
/// 频率比 `2^(±63.7/1200)` ≈ ±3.75% ⇒ 440 Hz 上下 ≈ 423.5 / 456.5 Hz，
/// 极差 ≈ 33 Hz。阈值 20 Hz 留了近 40% 的裕量（网格量化只会让读数更小一点，
/// 不会让它消失）。
#[test]
fn lfo_pitch_target_swings_fundamental() {
    let tables = PolySynthTables::from_recipes(&[PURE]);
    let params = PolySynthParams::new().with_lfo(LfoWave::Sine, LFO_RATE, 100.0, LfoTarget::Pitch);
    let out = render_single(&tables, params, 440.0, 96_000, 48_000);
    // 从 0.1 s 起（包络已到延音）取 9 个半周期窗口。
    let mut frequencies = Vec::with_capacity(9);
    for index in 0..9 {
        let start = 4800 + index * 4800;
        let window = &out[start..start + 4800];
        frequencies.push(fundamental_from_crossings(window, SR_F64));
    }
    let max = frequencies
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let min = frequencies.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(
        max - min > 20.0,
        "L2: pitch LFO did not swing the fundamental: frequencies={frequencies:?}"
    );
    // 读数本身也得在 440 Hz 附近（不是"摆动了但基准跑偏"）。
    assert!(
        (max + min) / 2.0 > 420.0 && (max + min) / 2.0 < 460.0,
        "L2: swung frequencies are not centred on 440 Hz: frequencies={frequencies:?}"
    );
}

// ---------------------------------------------------------------------------
// L3：振幅目标 —— 窗口峰值随 LFO 周期摆动
// ---------------------------------------------------------------------------

/// L3：`depth = 0.5`、5 Hz 正弦 ⇒ 10 ms 窗口峰值的极大/极小 > 1.5×。
///
/// 口径说明：不用窗口 RMS —— `mean((1 + 0.5·sin)²)` 在正负半周期上**相同**
/// （`sin²` 的半周期均值都是 0.5），RMS 量不出振幅颤动；窗口峰值 ≈ 窗口内
/// 最大增益 × 载波峰值（10 ms 窗口含 4.4 个载波周期，采样峰值 ≥ 真峰值的
/// 99.7%）。闭式参照：相位 0.25 处窗口增益 ≈ 1.5、相位 0.75 处 ≈ 0.5 ⇒
/// 比值 ≈ 2.8×。阈值 1.5× 留了近一半的裕量。
#[test]
fn lfo_amplitude_target_swings_window_peaks() {
    let tables = PolySynthTables::from_recipes(&[PURE]);
    let params =
        PolySynthParams::new().with_lfo(LfoWave::Sine, LFO_RATE, 0.5, LfoTarget::Amplitude);
    let out = render_single(&tables, params, 440.0, 96_000, 48_000);
    // 从 0.1 s 起取 90 个 10 ms 窗口的峰值。
    let mut peaks = Vec::with_capacity(90);
    for index in 0..90 {
        let start = 4800 + index * 480;
        let window = &out[start..start + 480];
        let peak = window
            .iter()
            .fold(0.0f32, |acc, sample| acc.max(sample.abs()));
        peaks.push(peak);
    }
    let max = peaks.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let min = peaks.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(min > 0.0, "L3: silent window — the note did not sound");
    assert!(
        max / min > 1.5,
        "L3: amplitude LFO did not swing window peaks: max={max}, min={min}"
    );
}

// ---------------------------------------------------------------------------
// L4：截止目标 —— 谐波能量摆动 + 取值器读数
// ---------------------------------------------------------------------------

/// L4a：滤波器启用（截止 1000 Hz）+ `depth = 2` 八度 ⇒ 7 次谐波（3080 Hz，
/// `HOLLOW` 的第 4 个分量，幅度 0.14）的 bin 能量在半周期窗口间变化 > 2×。
///
/// 闭式参照：截止在 250..4000 Hz 之间摆动；3080 Hz 在截止 250 Hz 时是
/// 3.6 个八度之上（四极梯形滤波器约 87 dB 衰减），在截止 4000 Hz 时在通带内。
/// 窗口平均后的能量比预期数十倍；阈值 2× 非常保守。
#[test]
fn lfo_cutoff_target_swings_harmonic_energy() {
    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    let params = PolySynthParams::new()
        .with_filter(1000.0, 0.0, 0.0, false)
        .with_lfo(LfoWave::Sine, LFO_RATE, 2.0, LfoTarget::Cutoff);
    let out = render_single(&tables, params, 440.0, 96_000, 48_000);
    // 从 0.1 s 起取 9 个半周期窗口的 3080 Hz bin 幅度。
    let mut magnitudes = Vec::with_capacity(9);
    for index in 0..9 {
        let start = 4800 + index * 4800;
        let window = &out[start..start + 4800];
        magnitudes.push(dft_bin(window, 3080.0, SR_F64));
    }
    let max = magnitudes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min = magnitudes.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(
        min > 0.0,
        "L4a: no energy at 3080 Hz — the note did not sound"
    );
    assert!(
        max / min > 2.0,
        "L4a: cutoff LFO did not swing the 7th-harmonic energy: magnitudes={magnitudes:?}"
    );
}

/// L4b：`debug_lfo_cutoff_hz()` 的读数 —— 相位 0.25 处 ≈ 4000 Hz
/// （`1000 × 2^2`），相位 0.5 处 ≈ 1000 Hz（`1000 × 2^0`）。
///
/// 口径：`Lfo::process` 先取值后推进相位 ⇒ 渲染 N 帧后读到的值是第 N 帧
/// （相位 `(N-1)/9600`）的值；2400 帧后相位 ≈ 0.2499 ⇒ 正弦 ≈ 1.0 ⇒
/// 截止 ≈ 4000 Hz；4800 帧后相位 ≈ 0.4999 ⇒ 正弦 ≈ 0.00065 ⇒ 截止 ≈ 1000.9 Hz。
/// 容差取 ±20 Hz / ±5 Hz（`f32` 相位累加误差远小于此）。
#[test]
fn lfo_cutoff_accessor_reads_effective_cutoff() {
    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    let params = PolySynthParams::new()
        .with_filter(1000.0, 0.0, 0.0, false)
        .with_lfo(LfoWave::Sine, LFO_RATE, 2.0, LfoTarget::Cutoff);
    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    synth.set_params(params, &tables);
    synth.note_on(NoteEvent::new(0, 96_000, 440.0, 1.0), &tables);
    let mut out = vec![0.0f32; 2400];
    synth.render(&tables, 0, &mut out);
    let at_quarter = synth.debug_lfo_cutoff_hz();
    assert!(
        (at_quarter - 4000.0).abs() < 20.0,
        "L4b: at phase 0.25 the effective cutoff should be ~4000 Hz, got {at_quarter}"
    );
    synth.render(&tables, 2400, &mut out);
    let at_half = synth.debug_lfo_cutoff_hz();
    assert!(
        (at_half - 1000.0).abs() < 5.0,
        "L4b: at phase 0.5 the effective cutoff should be ~1000 Hz, got {at_half}"
    );
}

/// L4c：`depth == 0` 时取值器恒等于基值（"没有调制"时的读数）。
#[test]
fn lfo_cutoff_accessor_is_base_when_depth_zero() {
    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    let params = PolySynthParams::new()
        .with_filter(1000.0, 0.0, 0.0, false)
        .with_lfo(LfoWave::Sine, LFO_RATE, 0.0, LfoTarget::Cutoff);
    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    synth.set_params(params, &tables);
    synth.note_on(NoteEvent::new(0, 96_000, 440.0, 1.0), &tables);
    let mut out = vec![0.0f32; 4800];
    synth.render(&tables, 0, &mut out);
    assert_eq!(synth.debug_lfo_cutoff_hz(), 1000.0);
}

// ---------------------------------------------------------------------------
// L5：块切分不变性（LFO 网格锚在绝对位置，不在调用块上）
// ---------------------------------------------------------------------------

/// L5：同一音符用 64 / 128 / 256 帧三种切法渲染 ⇒ **逐位**相同
/// （与 `polysynth_render.rs` 的 P8 同口径；LFO 接线后仍须成立）。
#[test]
fn lfo_render_is_invariant_to_chunk_size() {
    let tables = PolySynthTables::from_recipes(&[PURE]);
    let params = PolySynthParams::new().with_lfo(LfoWave::Sine, LFO_RATE, 100.0, LfoTarget::Pitch);
    let notes = [NoteEvent::new(0, 96_000, 440.0, 1.0)];
    let total = 48_000usize;
    let by_64 = render_split_with_params(&tables, params, &notes, &chunked(total, 64));
    let by_128 = render_split_with_params(&tables, params, &notes, &chunked(total, 128));
    let by_256 = render_split_with_params(&tables, params, &notes, &chunked(total, 256));
    assert_eq!(bits(&by_64), bits(&by_128));
    assert_eq!(bits(&by_64), bits(&by_256));
}

// ---------------------------------------------------------------------------
// L6：无声帧不推进 LFO
// ---------------------------------------------------------------------------

/// L6：先空渲染 4800 帧再触发（起点 4800）与直接在 4800 触发 ⇒ **逐位**相同。
/// 空渲染走 early return、LFO 一次也不推进；若空帧推进了 LFO，两段的相位
/// 会错开 4800 帧 ⇒ 逐位比较立刻变红。
#[test]
fn lfo_does_not_advance_without_sounding_voices() {
    let tables = PolySynthTables::from_recipes(&[PURE]);
    let params = PolySynthParams::new().with_lfo(LfoWave::Sine, LFO_RATE, 100.0, LfoTarget::Pitch);
    // A：先空渲染 4800 帧（位置 0..4800），再在 4800 触发并渲染到 9600。
    let mut first = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    first.set_params(params, &tables);
    let mut silence = vec![0.0f32; 4800];
    first.render(&tables, 0, &mut silence);
    assert!(!silence.is_empty());
    assert!(silence.iter().all(|sample| *sample == 0.0));
    first.note_on(NoteEvent::new(4800, 96_000, 440.0, 1.0), &tables);
    let mut out_a = vec![0.0f32; 4800];
    first.render(&tables, 4800, &mut out_a);
    // B：直接在 4800 触发并渲染同一段。
    let mut second = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    second.set_params(params, &tables);
    second.note_on(NoteEvent::new(4800, 96_000, 440.0, 1.0), &tables);
    let mut out_b = vec![0.0f32; 4800];
    second.render(&tables, 4800, &mut out_b);
    assert_eq!(bits(&out_a), bits(&out_b));
}

// ---------------------------------------------------------------------------
// L7：钳制 / 回退 / 下标映射
// ---------------------------------------------------------------------------

/// L7a：非法速率/深度回退到默认值、越界值钳到上限。
#[test]
fn lfo_params_clamp_and_fall_back() {
    // 速率：NaN / 负值 ⇒ 默认 5.0；500 ⇒ 钳到 100。
    assert_eq!(
        PolySynthParams::new()
            .with_lfo(LfoWave::Sine, f32::NAN, 100.0, LfoTarget::Pitch)
            .lfo_rate_hz(),
        5.0
    );
    assert_eq!(
        PolySynthParams::new()
            .with_lfo(LfoWave::Sine, -3.0, 100.0, LfoTarget::Pitch)
            .lfo_rate_hz(),
        5.0
    );
    assert_eq!(
        PolySynthParams::new()
            .with_lfo(LfoWave::Sine, 500.0, 100.0, LfoTarget::Pitch)
            .lfo_rate_hz(),
        100.0
    );
    // 深度：NaN / 负值 ⇒ 0.0（关）；5000 ⇒ 钳到 1200。
    assert_eq!(
        PolySynthParams::new()
            .with_lfo(LfoWave::Sine, 5.0, f32::NAN, LfoTarget::Pitch)
            .lfo_depth(),
        0.0
    );
    assert_eq!(
        PolySynthParams::new()
            .with_lfo(LfoWave::Sine, 5.0, -1.0, LfoTarget::Pitch)
            .lfo_depth(),
        0.0
    );
    assert_eq!(
        PolySynthParams::new()
            .with_lfo(LfoWave::Sine, 5.0, 5000.0, LfoTarget::Pitch)
            .lfo_depth(),
        1200.0
    );
}

/// L7b：默认参数是"关"（深度 0、正弦、5 Hz、音高目标）。
#[test]
fn lfo_defaults_are_off() {
    let params = PolySynthParams::new();
    assert_eq!(params.lfo_wave(), LfoWave::Sine);
    assert_eq!(params.lfo_rate_hz(), 5.0);
    assert_eq!(params.lfo_depth(), 0.0);
    assert_eq!(params.lfo_target(), LfoTarget::Pitch);
}

/// L7c：波形与目标的下标映射（含越界钳到最后一档）。
#[test]
fn lfo_index_mappings() {
    assert_eq!(LfoWave::from_index(0), LfoWave::Sine);
    assert_eq!(LfoWave::from_index(1), LfoWave::Triangle);
    assert_eq!(LfoWave::from_index(2), LfoWave::Square);
    assert_eq!(LfoWave::from_index(3), LfoWave::Saw);
    assert_eq!(LfoWave::from_index(255), LfoWave::Saw);
    assert_eq!(LfoTarget::from_index(0), LfoTarget::Pitch);
    assert_eq!(LfoTarget::from_index(1), LfoTarget::Amplitude);
    assert_eq!(LfoTarget::from_index(2), LfoTarget::Cutoff);
    assert_eq!(LfoTarget::from_index(255), LfoTarget::Cutoff);
}
