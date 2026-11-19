//! ⭐ **SFZ `<effect>` 效果总线判据**（`ADR-0077` D5/D7 的引擎半边）。
//!
//! # 量什么
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | SFX-1 | 全 0 发送 ⇒ 与"没有 `<effect>`"**逐位相同** | `to_bits` 逐样本相等 |
//! | SFX-2 | `effect1=100` ＋ `fx1 type=reverb` ⇒ 块末**湿声可闻**（尾部能量 > 0） | 末块 RMS |
//! | SFX-3 | `fx1 type=chorus`（不支持）⇒ **不接线** ＋ 装载期仍告警 | 输出与无效果逐位相同 ＋ `EffectNotProcessed` |
//! | SFX-6 | 总线缓冲每块清零（不累积上一块） | 连续两块的尾部能量不随块号增长 |
//!
//! ⚠ 不判的：`dsp_order` 排序（本片每条总线只有一台器件 ⇒ 排序位不可观测）、
//! `aux*`/`main`（`ADR-0077` D1 明确不建）。

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};
use yeban_sfz::parser::Warning;

const HZ: f32 = 48_000.0;
/// 样本帧数：480 帧（10 ms）⇒ 混响尾巴远长于干声。
const SAMPLE_FRAMES: usize = 480;
/// 块长（帧）。
const BLOCK: usize = 256;

fn prepare(text: &str) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, HZ, SAMPLE_FRAMES);
    PreparedSfz::prepare(&sfz, &sample, HZ).expect("摊平必须成功")
}

/// 触发一个音、渲染 `blocks` 块，返回每块的样本（按块拼接）。
fn render_note(plan: &PreparedSfz, blocks: usize) -> Vec<f32> {
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(plan, BLOCK);
    assert_eq!(pool.trigger_note(plan, 60, 100, 12_000, 1.0, None), 1);
    let mut rendered = Vec::with_capacity(blocks * BLOCK);
    for block in 0..blocks {
        let mut out = vec![0.0f32; BLOCK];
        pool.render_into(plan, block as u64 * BLOCK as u64, &mut out);
        rendered.extend(out);
    }
    rendered
}

fn rms(samples: &[f32]) -> f32 {
    let sum: f32 = samples.iter().map(|sample| sample * sample).sum();
    (sum / samples.len() as f32).sqrt()
}

/// SFX-1：`effect1=0` ⇒ 与没有 `<effect>` 的同一份乐器**逐位相同**。
#[test]
fn a_zero_send_is_bit_identical_to_a_plain_instrument() {
    let plain = render_note(&prepare("<region> sample=a.wav lokey=60 hikey=60"), 4);
    let zero = render_note(
        &prepare("<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=0"),
        4,
    );
    let bits = |samples: &[f32]| {
        samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&plain), bits(&zero), "发送量 0 必须逐位不变");
    assert!(rms(&plain) > 0.0, "干声必须真的出声");
}

/// SFX-2：`effect1=100` ＋ `fx1 type=reverb` ⇒ 干声结束后**仍有湿声**（尾部能量）。
///
/// ⚠ 尾巴有 ~1800 帧的物理延迟（预延迟 576 ＋ 最短梳状线 ≈ 1200）⇒ 必须
/// 渲染足够多的块才能观测到，⛔ 不是"接线失败"。
#[test]
fn a_full_send_puts_an_audible_tail_on_the_bus() {
    let plain = render_note(&prepare("<region> sample=a.wav lokey=60 hikey=60"), 16);
    let wet = render_note(
        &prepare(
            "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
        ),
        16,
    );
    // 干声 480 帧 ⇒ 第 3 块之后必须已经静音；湿声此时才刚开始出现。
    let tail_plain = &plain[3 * BLOCK..4 * BLOCK];
    assert!(
        rms(tail_plain) < 1.0e-6,
        "干声在 3 块之后必须已经静音（读数 {}）",
        rms(tail_plain)
    );
    for block in 3..8 {
        assert!(
            rms(&plain[block * BLOCK..(block + 1) * BLOCK]) < 1.0e-6,
            "干声第 {block} 块必须静音"
        );
    }
    let wet_tail: f32 = (7..12)
        .map(|block| rms(&wet[block * BLOCK..(block + 1) * BLOCK]))
        .fold(0.0f32, f32::max);
    assert!(
        wet_tail > 1.0e-4,
        "混响尾巴必须可闻（7..12 块最大 RMS {}）",
        wet_tail
    );
}

/// SFX-3：不支持的 `type` ⇒ 不接线（逐位等于无效果）＋ 装载期仍告警。
#[test]
fn an_unsupported_type_stays_dry_and_still_warns() {
    let text = "<effect> bus=fx1 type=chorus\n<region> sample=a.wav lokey=60 hikey=60 effect1=100";
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("必须可装载");
    assert!(
        sfz.warnings()
            .iter()
            .any(|warning| matches!(warning, Warning::EffectNotProcessed { .. })),
        "不支持的 type 必须仍然响亮告警"
    );
    let plain = render_note(&prepare("<region> sample=a.wav lokey=60 hikey=60"), 4);
    let unsupported = render_note(&prepare(text), 4);
    let bits = |samples: &[f32]| {
        samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&plain), bits(&unsupported), "未接线必须逐位等于干声");
}

/// SFX-6：总线缓冲每块清零 —— 输入停止后尾巴**单调衰减**，不因缓冲残留而累积。
#[test]
fn the_bus_buffers_are_cleared_every_block() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, BLOCK);
    assert_eq!(pool.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
    let mut levels = Vec::new();
    for block in 0..80 {
        let mut out = vec![0.0f32; BLOCK];
        pool.render_into(&plan, block as u64 * BLOCK as u64, &mut out);
        levels.push(rms(&out));
    }
    let peak = levels[20..40].iter().cloned().fold(0.0f32, f32::max);
    let late = levels[72..80].iter().cloned().fold(0.0f32, f32::max);
    assert!(peak > 1.0e-4, "尾巴必须真的出现（读数 {peak}）");
    assert!(
        late < peak * 0.5,
        "后段必须明显衰减（残留会把它抬起来）：peak={peak} late={late}"
    );
}

/// SFX-7（`ADR-0077` D8 的实时面）：**稳态渲染不再重建总线**。
///
/// # 判定量
///
/// 池的 `bus_builds` 读数（每次 `BusBank` 构造 ＋1，构造 ＝ 分配 `Vec` 与
/// `Reverb`）。`warm_buses` 一次之后，80 块渲染里它**必须不增长** —— 增长就
/// 意味着音频回调里分配过，违 [MUST-GATE-001]。
///
/// # 正对照
///
/// 不预热直接渲染：读数从 `0` 到 `1`。这一腿证明"读数恒 0"不是仪器失灵。
#[test]
fn steady_state_rendering_never_rebuilds_the_bus() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    // 正对照：没有预热 ⇒ 第一块就会建一次（兜底路径）。
    let mut lazy = SfzVoicePool::new(HZ);
    assert_eq!(lazy.bus_builds(), 0, "新池还没有总线");
    assert_eq!(lazy.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
    let mut out = vec![0.0f32; BLOCK];
    lazy.render_into(&plan, 0, &mut out);
    assert_eq!(
        lazy.bus_builds(),
        1,
        "兜底路径必须恰好建一次（仪器有判别力）"
    );

    // 判据腿：预热后稳定渲染，不再增长。
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, BLOCK);
    let after_warm = pool.bus_builds();
    assert_eq!(pool.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
    for block in 0..80 {
        let mut out = vec![0.0f32; BLOCK];
        pool.render_into(&plan, block as u64 * BLOCK as u64, &mut out);
    }
    assert_eq!(
        pool.bus_builds(),
        after_warm,
        "稳态渲染必须**零重建**（重建 ＝ 在回调里分配，违 [MUST-GATE-001]）"
    );
}
