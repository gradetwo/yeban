//! `yeban_dsp::polysynth` 的**发声 / 复音 / 窃取 / 确定性**判据
//! [ARCH-RT-004, ARCH-DSP-001, ARCH-DET-001]。
//!
//! # 口径（写在判据前面，避免"读数被误读"）
//!
//! - 采样率固定 **48 kHz**；所有 DFT 都用**矩形窗**（不做窗函数），
//!   因此只把"某个频率的 bin 有没有能量"当读数，**不**把它当幅度谱；
//! - 每一条涉及"参考值"的判据都用**闭式算术或同机测得量**做参照，
//!   不用理想值。`channel_strip` 线的教训：第一版拿"纯基波 RMS"当理想参照，
//!   而 `LadderFilter` 在 0.22× 截止处已有 0.82 dB 通带衰减 ⇒ 参照本身错了。
//!   这里的闭式参照是 `phase_inc × sample_rate / 2³²`（用的是**器件真的用的那个
//!   量化后的相位增量**，不是"440 Hz"这个理想数）；
//! - 频率读数有两个独立口径：**线性插值零交叉间隔**（亚毫赫兹级）与
//!   **单 bin DFT 抛物顶点**（受窗长限制）。两者都必须落在闭式值附近。
//!
//! # 判据
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | P1 | 默认音色弹 A4 ⇒ 零交叉推出的基频 ≈ 闭式值（±1 Hz） | 相位递推写错/不动 |
//! | P2 | 单谐波波表 ⇒ 插值零交叉的基频与闭式值相差 < 0.05 Hz | 同上 |
//! | P3 | 包络起振 ≈ 5 ms、释放尾巴 ≈ `Adsr` 的 8.85 τ（同机闭式） | 包络参数写错 |
//! | P4 | 8 个同时发声的音符**每一个**都在自己的 bin 上有能量 | 声部池串音/只发一个 |
//! | P5 | 超复音上限时窃取**确定的那个**声部（下标 + 可听身份） | 窃取选择写成"永远下标 0" |
//! | P6 | 3 ms 淡出把窃取处的样本跳变压到硬窃取的 1/5 以下 | 把淡出帧数设成 0（API 注入） |
//! | P7 | 同输入两次渲染**逐位**相同 | 引入未初始化状态 |
//! | P8 | 6 种 chunk 切分 ⇒ 逐位相同 | 任何依赖块边界的隐藏状态 |
//! | P9 | 退化参数（NaN/Inf/越界）⇒ 输出全有限、且不静音 | 直接把参数喂给 `tan`/`exp2` |
//!
//! `set_steal_fade_frames(0)` 是 P6 的**产品级注入**；其余注入在报告里以
//! "改源码 → 抓红行 → 还原"的形态给出（不改提交内容）。

use yeban_dsp::oscillator::{HOLLOW, WaveRecipe};
use yeban_dsp::polysynth::{
    NoteEvent, OscSettings, PolySynth, PolySynthParams, PolySynthTables, VOICES_PER_SLOT,
    phase_increment, steal_fade_frames_for,
};

/// 夹具采样率（Hz）。
const SR: f64 = 48_000.0;

/// 单谐波配方：波形是**纯正弦** ⇒ 每周期恰好 2 次零交叉、
/// 每个采样块的峰值正比于包络。判据用它把"波形"这个变量消掉。
const PURE: WaveRecipe = &[(1, 1.0)];

// ---------------------------------------------------------------------------
// 读数工具（都是测量工具，不是被测代码）
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
///
/// 一个周期 2 次零交叉 ⇒ `周期数 = (交叉数 − 1) / 2`。
/// 交叉时刻用相邻两样本线性插值到子样本精度，因此读数精度由**总时长**决定，
/// 不受采样率栅格限制。
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

/// 整数零交叉计数（与引擎 J7 同口径）。
fn zero_crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count()
}

/// 单 bin DFT 的**抛物顶点**基频估计 (Hz)：粗扫 0.25 Hz 栅格，再对峰值三点插值。
fn fundamental_from_dft(samples: &[f32], nominal_hz: f64, sample_rate: f64) -> f64 {
    let step = 0.25f64;
    let low = nominal_hz * 0.9;
    let count = ((nominal_hz * 1.1 - low) / step) as usize;
    let mut magnitudes = Vec::with_capacity(count + 1);
    let mut best = 0usize;
    for index in 0..=count {
        #[allow(clippy::cast_precision_loss)]
        let magnitude = dft_bin(samples, low + index as f64 * step, sample_rate);
        magnitudes.push(magnitude);
        if magnitude > magnitudes[best] {
            best = index;
        }
    }
    if best == 0 || best + 1 >= magnitudes.len() {
        #[allow(clippy::cast_precision_loss)]
        return low + best as f64 * step;
    }
    let (left, mid, right) = (magnitudes[best - 1], magnitudes[best], magnitudes[best + 1]);
    let denom = left - 2.0 * mid + right;
    let delta = if denom.abs() < 1e-18 {
        0.0
    } else {
        0.5 * (left - right) / denom
    };
    #[allow(clippy::cast_precision_loss)]
    let position = best as f64 + delta;
    low + position * step
}

/// 闭式参照：器件真的用的相位增量所对应的频率 (Hz)。
fn closed_form_hz(freq_hz: f32, sample_rate: f32) -> f64 {
    f64::from(phase_increment(freq_hz, sample_rate)) * f64::from(sample_rate) / 4_294_967_296.0
}

/// 渲染一个音符（起点 0、终点 `end_sample`、给定配方），返回单声道样本。
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

/// 按 `chunks` 给出的切分渲染同一段：返回拼接后的样本。
fn render_split(tables: &PolySynthTables, notes: &[NoteEvent], chunks: &[usize]) -> Vec<f32> {
    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    synth.set_params(PolySynthParams::new(), tables);
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

/// 按固定块长 `chunk` 渲染 `total` 帧（切分集合的生成器）。
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
// P1 / P2：基频
// ---------------------------------------------------------------------------

/// P1：**默认音色**（`HOLLOW` 波表、滤波器旁通）弹 A4。
///
/// 读数：整数零交叉计数推出的基频、DFT 抛物顶点的基频、以及闭式参照
/// `phase_increment(440, 48000) × 48000 / 2³²`。
///
/// 判据：两个读数与闭式参照相差 < 1 Hz（DFT 栅格 0.25 Hz ⇒ 1 Hz 是 4 格）。
#[test]
fn p1_default_tone_plays_a4_at_the_closed_form_frequency() {
    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    let frames = 48_000; // 恰好 1 秒
    let rendered = render_single(&tables, PolySynthParams::new(), 440.0, 96_000, frames);
    let expected = closed_form_hz(440.0, 48_000.0);
    let from_crossings = 2.0 * 440.0 * (frames as f64 / SR); // 理想事件数（只用于容差口径）

    let crossings = zero_crossings(&rendered);
    let measured =
        f64::from(u32::try_from(crossings).expect("交叉数放得进 u32")) / 2.0 / (frames as f64 / SR);
    let from_dft = fundamental_from_dft(&rendered, 440.0, SR);

    println!(
        "[yeban-dsp/polysynth] P1 A4 (HOLLOW, 旁通): crossings={crossings} \
         f_crossings={measured:.4} Hz f_dft={from_dft:.4} Hz \
         闭式={expected:.6} Hz 理想事件数={from_crossings:.1}"
    );

    assert!(
        rendered.iter().any(|sample| *sample != 0.0),
        "夹具必须真的出声"
    );
    assert!(
        (f64::from(u32::try_from(crossings).unwrap()) - from_crossings).abs() <= 3.0,
        "零交叉数偏离理想值太多: {crossings} vs {from_crossings:.1}"
    );
    assert!(
        (measured - expected).abs() < 1.0,
        "零交叉推出的基频 {measured:.4} Hz 与闭式 {expected:.6} Hz 相差超过 1 Hz"
    );
    assert!(
        (from_dft - expected).abs() < 1.0,
        "DFT 顶点推出的基频 {from_dft:.4} Hz 与闭式 {expected:.6} Hz 相差超过 1 Hz"
    );
}

/// P2：**单谐波**波表把"波形"这个变量消掉 ⇒ 插值零交叉的读数可以做到亚 0.05 Hz。
///
/// 用两个**非整数周期**的频率（442.7 Hz 与 61.3 Hz）：它们的相位增量都带量化，
/// 因此这条判据测的是"量化后的增量真的被逐样本用上了"，而不是"440 这个理想数"。
#[test]
fn p2_a_pure_table_gives_a_sub_hertz_frequency_reading() {
    let tables = PolySynthTables::from_recipes(&[PURE]);
    for nominal in [442.7f32, 61.3] {
        let frames = 96_000; // 2 秒
        let rendered = render_single(&tables, PolySynthParams::new(), nominal, 192_000, frames);
        let expected = closed_form_hz(nominal, 48_000.0);
        let measured = fundamental_from_crossings(&rendered, SR);
        println!(
            "[yeban-dsp/polysynth] P2 纯正弦 nominal={nominal} Hz: \
             f_crossings={measured:.6} Hz 闭式={expected:.6} Hz 差={:.6} Hz",
            (measured - expected).abs()
        );
        assert!(
            (measured - expected).abs() < 0.05,
            "nominal {nominal} Hz: 读数 {measured:.6} 与闭式 {expected:.6} 相差超过 0.05 Hz"
        );
    }
}

// ---------------------------------------------------------------------------
// P3：包络
// ---------------------------------------------------------------------------

/// P3：包络的**起振时间**与**释放尾巴**（单位：帧；同时换算成 ms）。
///
/// 读数口径：单谐波波表 + 4 kHz（48 kHz ÷ 4 kHz = **恰好 12 帧一个周期**）
/// ⇒ 每 12 帧的峰值是包络的一个**常数倍**，把读数按整段最大值归一化之后，
/// 这个常数被消掉，于是读数就是包络本身。
///
/// 参照（同机闭式，来自 `yeban_dsp::envelope` 的系数定义
/// `time_coefficient = exp(−6 / (time_s × sr))` ⇒ τ = `time_s / 6`）：
/// - 线性起振：`attack_s × sr` = 0.005 × 48000 = **240 帧**；
/// - 释放：从 sustain 0.7 衰减到 `Adsr` 的静音门 1e-4 需要
///   `ln(0.7 / 1e-4) = 8.854` 个时间常数，τ = 0.05 × 48000 / 6 = **400 帧**
///   ⇒ **3542 帧**。
#[test]
fn p3_amplitude_envelope_attacks_in_five_milliseconds_and_releases_in_about_fifty() {
    let tables = PolySynthTables::from_recipes(&[PURE]);
    let end_sample = 24_000u64; // 0.5 s 后松开
    let frames = 40_000usize;
    let rendered = render_single(&tables, PolySynthParams::new(), 4_000.0, end_sample, frames);

    // 每 12 帧（一个周期）的峰值 = 包络 × 常数。
    let blocks: Vec<f32> = rendered
        .chunks(12)
        .map(|block| block.iter().fold(0.0f32, |peak, s| peak.max(s.abs())))
        .collect();
    let maximum = blocks.iter().fold(0.0f32, |peak, b| peak.max(*b));
    assert!(
        maximum > 0.5,
        "夹具必须真的到达满幅（实测峰值 {maximum:.6}）"
    );
    let envelope: Vec<f32> = blocks.iter().map(|b| b / maximum).collect();

    // 起振：第一个非零块 ⇒ 归一化包络第一次 ≥ 0.99 的帧数。
    let first_nonzero = envelope
        .iter()
        .position(|value| *value > 0.0)
        .expect("必须有一个非零块");
    let attack_block = envelope
        .iter()
        .position(|value| *value >= 0.99)
        .expect("包络必须到达 0.99");
    let attack_frames = (attack_block - first_nonzero + 1) * 12;
    let attack_ms = attack_frames as f64 / SR * 1_000.0;

    // 释放：最后一个非零样本相对 `end_sample` 的偏移。
    let last_nonzero = rendered
        .iter()
        .rposition(|sample| *sample != 0.0)
        .expect("必须有一个非零样本");
    let release_frames = last_nonzero as u64 + 1 - end_sample;

    let attack_closed_form = 0.005 * 48_000.0; // 240 帧
    let release_closed_form = (0.7f64 / 1e-4).ln() * (0.05 * 48_000.0 / 6.0); // ≈ 3542 帧
    println!(
        "[yeban-dsp/polysynth] P3 包络: 起振={attack_frames} 帧 ({attack_ms:.3} ms), \
         闭式={attack_closed_form:.0} 帧 (5.000 ms); \
         释放尾巴={release_frames} 帧 ({:.3} ms), 闭式={release_closed_form:.0} 帧",
        release_frames as f64 / SR * 1_000.0
    );

    assert_eq!(first_nonzero, 0, "起点必须在第 0 帧（start_sample = 0）");
    assert!(
        (attack_frames as f64 - attack_closed_form).abs() <= 24.0,
        "起振 {attack_frames} 帧，闭式 {attack_closed_form} 帧（容差 ±24 帧 = ±0.5 ms）"
    );
    assert!(
        (release_frames as f64 - release_closed_form).abs() <= 130.0,
        "释放尾巴 {release_frames} 帧，闭式 {release_closed_form:.0} 帧（容差 ±130 帧）"
    );
    // 静音必须**逐位**归零（不是"很小"）。
    assert!(
        rendered[(end_sample as usize + release_frames as usize)..]
            .iter()
            .all(|sample| *sample == 0.0),
        "释放走完之后必须逐位归零"
    );
}

// ---------------------------------------------------------------------------
// P4 / P5：复音与窃取
// ---------------------------------------------------------------------------

/// P4：**8 个音符同时弹** ⇒ 每一个都必须在自己的基频 bin 上有能量。
///
/// 读数口径：单谐波波表 + 12 000 帧（0.25 s）窗口 ⇒ DFT 栅格 4 Hz；
/// 8 个频率都取 60 Hz 的整数倍（200/260/…/620 Hz）⇒ 每个都在栅格上，
/// 且互不落在对方的 bin 上（间隔 ≥ 60 Hz = 15 个栅格）。
///
/// 判据：每一个 bin 的幅度都 > 0.3（sustain 0.7 的量级），
/// 且最大/最小 < 1.3（等增益等包络 ⇒ 应当几乎相等）。
#[test]
fn p4_eight_simultaneous_notes_all_sound() {
    let tables = PolySynthTables::from_recipes(&[PURE]);
    let frequencies: Vec<f32> = (0..8).map(|index| 200.0 + index as f32 * 60.0).collect();
    let frames = 12_000usize;

    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    synth.set_params(PolySynthParams::new(), &tables);
    for freq in &frequencies {
        synth.note_on(NoteEvent::new(0, 192_000, *freq, 1.0), &tables);
    }
    let mut out = vec![0.0f32; frames];
    synth.render(&tables, 0, &mut out);

    assert_eq!(synth.active_voices(), 8, "8 个音符都必须在池里");
    assert_eq!(synth.voice_steals(), 0, "8 < 16 ⇒ 不该发生窃取");

    let magnitudes: Vec<f64> = frequencies
        .iter()
        .map(|freq| dft_bin(&out, f64::from(*freq), SR))
        .collect();
    let lowest = magnitudes.iter().fold(f64::MAX, |a, b| a.min(*b));
    let highest = magnitudes.iter().fold(f64::MIN, |a, b| a.max(*b));
    println!(
        "[yeban-dsp/polysynth] P4 8 音同时: bins={magnitudes:?} min={lowest:.6} max={highest:.6} ratio={:.4}",
        highest / lowest
    );

    assert!(
        out.iter().all(|sample| sample.is_finite()),
        "复音输出必须全部有限"
    );
    for (freq, magnitude) in frequencies.iter().zip(&magnitudes) {
        assert!(
            *magnitude > 0.3,
            "{freq} Hz 的声部没有发声（bin = {magnitude:.6}）"
        );
    }
    assert!(
        highest / lowest < 1.3,
        "等增益等包络的 8 个声部音量应当接近一致: min={lowest:.6} max={highest:.6}"
    );
}

/// P5：**超出复音上限** ⇒ 窃取的是哪一个？
///
/// 两个独立口径：
/// 1. **下标口径**（机械）：给每个初始音符一个互不相同的 `end_sample` 标签，
///    `note_on` 在被窃取的槽位上立刻写入新音符的标签 ⇒ 读出下标；
/// 2. **可听口径**：被窃取音符的基频 bin 必须**塌下去**，新音符的基频 bin
///    必须**立起来**，而**其余声部的 bin 不动**。
///
/// 判据：下标是 0（起点最小者），可听口径三个条件都成立，
/// 且 `voice_steals()` 恰好加 1。
#[test]
fn p5_over_the_polyphony_limit_a_deterministic_voice_is_stolen() {
    let tables = PolySynthTables::from_recipes(&[PURE]);
    let low: [f32; 4] = [200.0, 300.0, 400.0, 500.0];
    let stolen_note = 900.0f32;
    let mut synth = PolySynth::<4>::new(48_000);
    synth.set_params(PolySynthParams::new(), &tables);
    for (index, freq) in low.iter().enumerate() {
        synth.note_on(
            NoteEvent::new(index as u64 * 128, 90_001 + index as u64, *freq, 1.0),
            &tables,
        );
    }
    // 先渲染 500 帧：包络必须真的在发声（电平为 0 的声部一淡出就被回收）。
    let mut warmup = vec![0.0f32; 500];
    synth.render(&tables, 0, &mut warmup);
    let before: Vec<f64> = low
        .iter()
        .map(|f| dft_bin(&warmup, f64::from(*f), SR))
        .collect();

    // 第 5 个音符 ⇒ 池满 ⇒ 恰好一次窃取。
    synth.note_on(NoteEvent::new(2_000, 99_999, stolen_note, 1.0), &tables);
    assert_eq!(synth.voice_steals(), 1, "第 5 个音符必须逼出一次窃取");
    let victim = (0..4)
        .find(|index| {
            synth
                .debug_voice(*index)
                .is_some_and(|state| state.1 == 99_999)
        })
        .expect("被窃取的槽位必须带上新音符的 end_sample 标签");
    println!(
        "[yeban-dsp/polysynth] P5 窃取: victim_index={victim} (标签 99_999), \
         steals={} 声部起点=0/128/256/384 ⇒ 起点最小者=下标 0",
        synth.voice_steals()
    );
    assert_eq!(victim, 0, "必须窃取 start_sample 最小的那个声部");

    // 走完 3 ms 淡出（144 帧）+ 让新音符进入稳定期。
    let mut after = vec![0.0f32; 12_000];
    synth.render(&tables, 500, &mut after);
    let after_low: Vec<f64> = low
        .iter()
        .map(|f| dft_bin(&after, f64::from(*f), SR))
        .collect();
    let after_new = dft_bin(&after, f64::from(stolen_note), SR);
    println!(
        "[yeban-dsp/polysynth] P5 可听口径: 窃取前 200 Hz bin={:.6} ⇒ 窃取后 {:.6}; \
         新音符 900 Hz bin={after_new:.6}; 其余 300/400/500 Hz={:.6}/{:.6}/{:.6}",
        before[0], after_low[0], after_low[1], after_low[2], after_low[3]
    );

    assert!(
        after_low[0] < 0.05,
        "被窃取的 200 Hz 声部必须消失（bin = {:.6}）",
        after_low[0]
    );
    assert!(
        after_new > 0.3,
        "新音符的 900 Hz 必须真的发声（bin = {after_new:.6}）"
    );
    for (freq, magnitude) in [300.0f32, 400.0, 500.0].iter().zip(&after_low[1..]) {
        assert!(
            *magnitude > 0.3,
            "未被窃取的 {freq} Hz 声部必须继续发声（bin = {magnitude:.6}）"
        );
    }
    assert!(
        after.iter().all(|sample| sample.is_finite()),
        "窃取后的输出必须全部有限"
    );
}

/// P6：**3 ms 淡出把窃取处的样本跳变压下去**（[ARCH-RT-004] / [ARCH-DSP-001]）。
///
/// # 口径（这里踩过两次坑，都写下来）
///
/// ❌ 第一版把"窃取发生"放在渲染窗口**之外**（`note_on` 的 `start_sample` 是
/// warm-up 之后的某个位置），于是被测量的窗口里根本没有台阶 ⇒ 软硬两臂都读到
/// `step=0`（判据自己抓到了夹具写错）。
///
/// ❌ 第二版把 400 帧整段的最大位移当读数 ⇒ 读数被**新音符自己的斜率**主导
/// （新音符 3 kHz：`2π×3000/48000 ≈ 0.39` 每样本）⇒ 软 0.385 / 硬 0.393，
/// 比值 0.98，没有判别力。
///
/// ✅ 现在：读数取**切口本身**的一小段（warm-up 的最后一帧 + 渲染的前 8 帧，共 9 个样本）。
/// 切口处新音符的包络刚走到 `1/240`，它自己的斜率可以忽略；被测量的是
/// "旧波形的瞬时幅度有没有被切断"。
///
/// 同一个夹具跑两次，**唯一**差别是 `set_steal_fade_frames(144)`（默认 3 ms）
/// 对 `set_steal_fade_frames(0)`（硬窃取，产品级注入）。声部池是 4 个**不同音高**
/// 的长音符（避免同相相消），warm-up 512 帧让包络进入 sustain，
/// 然后第 5 个音符在**渲染窗口的第 0 帧**起音（`start_sample = 512` = 当前位置）。
///
/// 同机参照：warm-up 末段 16 帧的最大相邻位移 = "这条信号本来就有的斜率"。
///
/// 判据：硬窃取的切口位移 > 0.3（覆盖度自检：夹具必须真的制造出可听爆音），
/// 且软窃取 < 硬窃取的 1/5。
#[test]
fn p6_the_three_millisecond_fade_bounds_the_stealing_step() {
    /// 切口外的采样数（切口位移只看这一小段）。
    const CUT: usize = 8;
    /// 同机参照窗口：warm-up 末段多少帧。
    const REFERENCE: usize = 16;

    let run = |fade_frames: Option<u32>| -> (f32, f32, f32) {
        let tables = PolySynthTables::from_recipes(&[PURE]);
        let mut synth = PolySynth::<4>::new(48_000);
        synth.set_params(PolySynthParams::new(), &tables);
        if let Some(frames) = fade_frames {
            synth.set_steal_fade_frames(frames);
        }
        // 4 个不同音高的长音符铺满池（不同相 ⇒ 不会互相抵消）。
        for (index, freq) in [110.0f32, 137.0, 163.0, 191.0].iter().enumerate() {
            synth.note_on(
                NoteEvent::new(index as u64 * 32, 96_000, *freq, 1.0),
                &tables,
            );
        }
        let mut warmup = vec![0.0f32; 512];
        synth.render(&tables, 0, &mut warmup);
        // 同机参照：sustain 段本来就有的最大相邻位移。
        let natural = warmup[warmup.len() - REFERENCE - 1..]
            .windows(2)
            .fold(0.0f32, |worst, pair| worst.max((pair[1] - pair[0]).abs()));
        // 第 5 个音符：`start_sample` = 当前位置（512）⇒ 窃取瞬态落在窗口第 0 帧。
        synth.note_on(NoteEvent::new(512, 96_000, 3_000.0, 1.0), &tables);
        assert_eq!(synth.voice_steals(), 1, "夹具必须真的窃取");
        let mut out = vec![0.0f32; 2_048];
        synth.render(&tables, 512, &mut out);

        // 切口：warm-up 最后一帧 + 渲染的前 `CUT` 帧。
        let mut cut_window = Vec::with_capacity(CUT + 1);
        cut_window.push(warmup[warmup.len() - 1]);
        cut_window.extend_from_slice(&out[..CUT]);
        let cut_step = cut_window
            .windows(2)
            .fold(0.0f32, |worst, pair| worst.max((pair[1] - pair[0]).abs()));
        let peak = warmup.iter().fold(0.0f32, |worst, s| worst.max(s.abs()));
        (cut_step, peak, natural)
    };

    let (soft_step, soft_peak, natural) = run(None);
    let (hard_step, hard_peak, _) = run(Some(0));
    println!(
        "[yeban-dsp/polysynth] P6: 软(144 帧) 切口位移={soft_step:.6} | \
         硬(0 帧) 切口位移={hard_step:.6} | 比={:.4} | \
         同机参照(warm-up 末段自然斜率)={natural:.6} | \
         峰值 软={soft_peak:.6} 硬={hard_peak:.6} (默认淡出={} 帧)",
        soft_step / hard_step,
        steal_fade_frames_for(48_000.0)
    );
    assert!(soft_peak > 0.1, "夹具必须真的出声");
    assert!(
        hard_step > 0.3,
        "硬窃取的切口位移只有 {hard_step:.6} —— 夹具没有制造出可听爆音，判据没有判别力"
    );
    assert!(
        soft_step < hard_step * 0.2,
        "3 ms 淡出必须把切口位移压到硬窃取的五分之一以下: soft={soft_step:.6} hard={hard_step:.6}"
    );
    assert!(
        soft_step < 4.0 * natural,
        "软窃取的切口位移 {soft_step:.6} 必须与自然斜率 {natural:.6} 同量级"
    );
}

// ---------------------------------------------------------------------------
// P7 / P8：确定性
// ---------------------------------------------------------------------------

/// 一次性夹具：4 个音符（含一次窃取）的固定调度。
fn fixture_notes() -> Vec<NoteEvent> {
    vec![
        NoteEvent::new(0, 40_000, 110.0, 1.0),
        NoteEvent::new(600, 40_000, 273.5, 0.8),
        NoteEvent::new(1_100, 40_000, 441.0, 0.6),
        NoteEvent::new(1_700, 40_000, 900.0, 0.4),
    ]
}

/// P7：同输入两次渲染**逐位**相同（[ARCH-DET-001] 的 L1 契约）。
///
/// 判据：两次的 `f32::to_bits` 序列完全相等（**不是哈希**，是逐位比较，
/// 因此没有"代理指标"这一层）。
#[test]
fn p7_the_same_input_renders_bit_identical_twice() {
    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    let first = render_split(&tables, &fixture_notes(), &chunked(24_000, 128));
    let second = render_split(&tables, &fixture_notes(), &chunked(24_000, 128));
    assert!(first.iter().any(|sample| *sample != 0.0), "夹具必须出声");
    assert_eq!(first.len(), 24_000);
    assert_eq!(bits(&first), bits(&second), "两次渲染必须逐位相同");
    println!(
        "[yeban-dsp/polysynth] P7 两次渲染: frames={} 逐位相同=true 峰值={:.6}",
        first.len(),
        first.iter().fold(0.0f32, |peak, s| peak.max(s.abs()))
    );
}

/// P8：**多种 chunk 切分 ⇒ 逐位相同**。
///
/// 口径：同一个夹具、同一串输入，只改"每次 `render` 多少帧"。
/// 器件状态只有一个方向（`phase` 递增、包络逐样本、滤波器逐样本），
/// 没有任何依赖块边界的状态 ⇒ 输出与切分**构造性无关**。
///
/// 判据：6 种切分的拼接结果与 128 帧基准**逐位**相同（逐位比较，非哈希）。
#[test]
fn p8_chunk_splits_do_not_change_a_single_bit() {
    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    let notes = fixture_notes();
    let total = 24_000usize;
    let reference = render_split(&tables, &notes, &chunked(total, 128));

    let mut sizes: Vec<usize> = Vec::new();
    for chunk in [1usize, 7, 64, 127, 333, 4_096] {
        let split = render_split(&tables, &notes, &chunked(total, chunk));
        assert_eq!(split.len(), total, "切分 {chunk} 的帧数必须对得上");
        let equal = bits(&split) == bits(&reference);
        sizes.push(chunk);
        println!("[yeban-dsp/polysynth] P8 chunk={chunk:>4} 帧: 与 128 帧基准逐位相同={equal}");
        assert!(equal, "切分 {chunk} 与 128 帧基准出现了位差");
    }
    println!("[yeban-dsp/polysynth] P8 汇总: frames={total} 切分={sizes:?} 全部逐位相同=true");
}

// ---------------------------------------------------------------------------
// P9：退化输入
// ---------------------------------------------------------------------------

/// P9：退化参数与退化音符不产生 `NaN`/`inf`，也不把整条链路弄成静音。
///
/// 覆盖：`NaN`/`inf` 的滤波器参数与包络时间、非有限电平与失谐、
/// 越界波表下标、`NaN`/0/极大/极小 的频率、非法采样率、`NaN` 增益。
#[test]
fn p9_degenerate_inputs_never_produce_non_finite_output() {
    let tables = PolySynthTables::from_recipes(&[PURE]);

    let parameter_sets = [
        PolySynthParams::new().with_filter(f32::NAN, f32::NAN, f32::NAN, false),
        PolySynthParams::new().with_filter(-1.0, 5.0, -3.0, false),
        PolySynthParams::new().with_filter(1.0e9, 1.0e9, 1.0e9, false),
        PolySynthParams::new().with_envelope(f32::NAN, -1.0, f32::NAN, f32::INFINITY),
        PolySynthParams::new().with_oscillators(
            OscSettings::new(usize::MAX, 1.0, f32::NAN),
            OscSettings::new(usize::MAX, 1.0, f32::INFINITY),
        ),
        PolySynthParams::new().with_oscillators(
            OscSettings::new(usize::MAX, f32::INFINITY, -1.0e9),
            OscSettings::off(),
        ),
    ];
    let frequencies = [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        0.0,
        -100.0,
        1.0e9,
        1.0e-9,
    ];
    let gains = [f32::NAN, f32::INFINITY, -1.0, 0.0, 1.0];

    let mut checked = 0usize;
    for params in parameter_sets {
        for freq in frequencies {
            for gain in gains {
                for sample_rate in [0u32, 1, 8_000, 48_000, 192_000, u32::MAX] {
                    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(sample_rate);
                    synth.set_params(params, &tables);
                    synth.note_on(NoteEvent::new(0, 4_800, freq, gain), &tables);
                    let mut out = vec![0.0f32; 512];
                    synth.render(&tables, 0, &mut out);
                    assert!(
                        out.iter().all(|sample| sample.is_finite()),
                        "退化组合产生了非有限输出: params={params:?} freq={freq} gain={gain} sr={sample_rate}"
                    );
                    checked += 1;
                }
            }
        }
    }
    println!("[yeban-dsp/polysynth] P9 退化组合: {checked} 组, 全部输出有限=true");
    assert!(checked >= 6 * 7 * 5 * 6, "覆盖度不足: 只有 {checked} 组");
}
