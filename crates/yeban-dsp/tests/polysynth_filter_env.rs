//! `yeban_dsp::polysynth` 的**滤波器包络**判据（⭐ `T-FIX-276`）。
//!
//! 口径：采样率固定 **48 kHz**，`PolySynthParams` 的滤波器**启用**
//! （`with_filter(cutoff, res, drive, false)`）—— 旁通时滤波器一次也不 `process`，
//! 滤波器包络无处可落。
//!
//! "亮度"读数是**一阶差分 RMS / 信号 RMS** 的比值，它与幅度无关（幅度同时进分子
//! 分母），因此把"振幅包络"这个变量消掉：只读滤波器截止。亮度判据用 [`HOLLOW`]
//! （多谐波配方）：纯正弦在"两种截止都在信号频率之上"时输出几乎不变，谐波被切才可读。
//!
//! 参照量用**闭式算术**：`cutoff = base × 2^(depth × env)`，env 取该时点的包络段值
//! （attack 末尾 `1.0`、sustain `sustain`、释放之后 `0.0`）。
//!
//! # 判据
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | F1 | `filter_env_depth = 0` ⇒ 与接线前**逐位相同** | 去掉 `depth > 0` 的开关（改成永远推进第二只包络） |
//! | F2 | `depth > 0` ⇒ attack 末尾亮、sustain 落到暗、释放之后回到最暗 | 网格上不重配置滤波器（删 `configure`） |
//! | F3 | 块切分 128 / 37 / 整段 ⇒ 逐位相同 | 网格锚在"调用块局部位置"而不是绝对位置 |
//! | F4 | pluck 读数（短 decay + 高 depth）：起音头 / 尾巴比值 | 同 F2 的注入 |

use yeban_dsp::oscillator::{HOLLOW, WaveRecipe};
use yeban_dsp::polysynth::{
    NoteEvent, PolySynth, PolySynthParams, PolySynthTables, VOICES_PER_SLOT,
};

/// 夹具采样率（Hz）。
const SR: u32 = 48_000;

/// 亮度（一阶差分 RMS / 信号 RMS）：与幅度无关，只随截止频率升高而升高。
fn brightness(samples: &[f32]) -> f64 {
    assert!(
        samples.len() >= 64,
        "亮度窗口至少 64 帧，实得 {}",
        samples.len()
    );
    let mut energy = 0.0f64;
    let mut diff_energy = 0.0f64;
    for (index, sample) in samples.iter().enumerate() {
        energy += f64::from(*sample) * f64::from(*sample);
        if index > 0 {
            let d = f64::from(*sample) - f64::from(samples[index - 1]);
            diff_energy += d * d;
        }
    }
    assert!(energy > 1e-24, "亮度窗口不得是静音（能量 {energy:e}）");
    (diff_energy / energy).sqrt()
}

/// 按 `chunk` 切分渲染一个音符（起点 0、终点 `end_sample`），返回单声道样本。
fn render_chunked(
    params: PolySynthParams,
    recipe: WaveRecipe,
    freq_hz: f32,
    end_sample: u64,
    frames: usize,
    chunk: usize,
) -> Vec<f32> {
    assert!(chunk > 0, "块长必须为正，实得 {chunk}");
    let tables = PolySynthTables::from_recipes(&[recipe]);
    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(SR);
    synth.set_params(params, &tables);
    synth.note_on(NoteEvent::new(0, end_sample, freq_hz, 1.0), &tables);
    let mut buffer = vec![0.0f32; chunk];
    let mut out = Vec::with_capacity(frames);
    let mut position = 0u64;
    while out.len() < frames {
        let take = chunk.min(frames - out.len());
        synth.render(&tables, position, &mut buffer[..take]);
        out.extend_from_slice(&buffer[..take]);
        position += take as u64;
    }
    out
}

/// 启用滤波器的底座：截止 300 Hz、振幅包络**满延音 + 超长释放**
/// （释放 10 s ⇒ 松键后仍有音可读，滤波器关合才可读）。
fn base_params() -> PolySynthParams {
    PolySynthParams::new()
        .with_filter(300.0, 0.3, 0.0, false)
        .with_envelope(0.005, 0.08, 1.0, 10.0)
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

/// **F1**：`filter_env_depth = 0` ⇒ 与接线前逐位相同。
///
/// 接线前的状态 = 只调 `with_filter` / `with_envelope` 的器件参数；接线后同一台
/// 器件多带一条 `with_filter_env(..., 0.0)`。深度 0 必须让第二只包络**一次也不
/// 推进**、网格**一次也不重配置**。
#[test]
fn f1_filter_envelope_depth_zero_is_bit_identical() {
    const FRAMES: usize = 4096;
    let mutated = base_params().with_filter_env(0.1, 0.2, 0.5, 0.2, 0.0);
    let reference = render_chunked(base_params(), HOLLOW, 110.0, 2400, FRAMES, 128);
    let candidate = render_chunked(mutated, HOLLOW, 110.0, 2400, FRAMES, 128);
    assert_eq!(
        bits(&reference),
        bits(&candidate),
        "深度 0 必须逐位相同（第二只包络不得推进、网格不得重配置）"
    );
    assert_eq!(mutated.filter_env_attack_s(), 0.1);
    assert_eq!(mutated.filter_env_decay_s(), 0.2);
    assert_eq!(mutated.filter_env_sustain(), 0.5);
    assert_eq!(mutated.filter_env_release_s(), 0.2);
    assert_eq!(mutated.filter_env_depth(), 0.0);
}

/// **F2**：`depth = 3` 八度 ⇒ 亮度随包络三段走（起亮 → sustain 落暗 → 释放回最暗）。
#[test]
fn f2_the_envelope_opens_and_closes_the_filter() {
    // 包络：attack 50 ms（到 1.0）、decay 100 ms（到 0.1）、sustain 0.1、
    // release 200 ms（到 0）。深度 3 八度 ⇒ cutoff 300 → 2400 → 369 → 300。
    let params = base_params().with_filter_env(0.05, 0.1, 0.1, 0.2, 3.0);
    let total = 72_000usize; // 1.5 s
    let samples = render_chunked(params, HOLLOW, 110.0, 24_000, total, 128);
    assert_eq!(samples.len(), total);

    let window = |from: usize, to: usize| brightness(&samples[from..to]);
    // 40–50 ms：env ≈ 1.0（≈2400 Hz）
    let at_attack = window(1_920, 2_400);
    // 145–150 ms：decay 末尾（≈369 Hz）
    let at_sustain = window(6_960, 7_440);
    // 300–302 ms：sustain 段（≈369 Hz）
    let sustained = window(13_920, 14_400);
    // 700–705 ms：释放早已结束（≈300 Hz）
    let after_release = window(33_600, 34_080);

    eprintln!(
        "F2 亮度读数：attack={at_attack:.6} decayEnd={at_sustain:.6} sustain={sustained:.6} afterRelease={after_release:.6}"
    );
    assert!(
        at_attack > at_sustain * 1.5,
        "attack 段必须明显比 sustain 段亮（实得 {at_attack:.6} vs {at_sustain:.6}）"
    );
    assert!(
        sustained > after_release * 1.05,
        "sustain 段必须比释放之后亮（实得 {sustained:.6} vs {after_release:.6}）"
    );
    assert!(
        (sustained / at_sustain - 1.0).abs() < 0.2,
        "decay 末尾与 sustain 段是同一电平（实得 {at_sustain:.6} vs {sustained:.6}）"
    );
}

/// **F3**：块切分（128 / 37 / 整段）⇒ 逐位相同（网格锚在绝对位置）。
#[test]
fn f3_filter_envelope_is_chunk_independent() {
    let params = base_params().with_filter_env(0.05, 0.1, 0.1, 0.2, 3.0);
    let reference = render_chunked(params, HOLLOW, 110.0, 24_000, 8_000, 128);
    for chunk in [37usize, 8_000] {
        assert_eq!(
            bits(&reference),
            bits(&render_chunked(params, HOLLOW, 110.0, 24_000, 8_000, chunk)),
            "块长 {chunk} 与 128 必须逐位相同"
        );
    }
}

/// **F4**：pluck 读数（attack 0、decay 30 ms、sustain 0、depth 4 八度）。
#[test]
fn f4_pluck_shorts_the_filter_life() {
    let params = base_params().with_filter_env(0.0, 0.03, 0.0, 0.0, 4.0);
    let samples = render_chunked(params, HOLLOW, 110.0, 9_600, 9_600, 128);
    assert_eq!(samples.len(), 9_600);
    // 起音头 0–10 ms：env 从 1 衰减到 ≈0.67（300 × 16 = 4800 Hz 起）
    let head = brightness(&samples[0..480]);
    // 120–160 ms：env = 0（300 Hz）
    let tail = brightness(&samples[4_800..8_640]);
    eprintln!(
        "F4 pluck 亮度读数：head={head:.6} tail={tail:.6} 比值={:.2}",
        head / tail
    );
    assert!(
        head > tail * 1.5,
        "pluck 的起音头必须明显比尾巴亮（实得 {head:.6} vs {tail:.6}）"
    );
}
