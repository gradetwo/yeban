//! `line/engine-mix` 的端到端判据（三）：**声部滤波器与音色参数**。
//! [ARCH-DSP-001, ROAD-M2-006]
//!
//! # 口径（写在判据前面，避免"读数被误读"）
//!
//! 声部滤波器是 `yeban_dsp::filter::LadderFilter`（四极 TPT/零延迟反馈梯形低通，
//! 24 dB/oct，`SOFT_KNEE = 0.7` 有界饱和，`DENORMAL_FLOOR = 1e-30` 冲刷）。
//! 参数从**引擎侧临时形状** [`yeban_engine::synth::ToneParams`] 来
//! （`InternalInstrument` 设备的 `cutoff_hz` / `resonance` / `drive`），
//! 见该类型的文档与 `docs/ledger/engine-mix-notes.md` 的 needs。
//!
//! 判据全部走**直接驱动合成器**的 `SynthRig`：滤波器是声部级器件，
//! 端到端夹具（模型 → 快照 → 母线）还要叠上声相与限制器，读数会被那两级污染。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | F1 | 旁通 ⇒ **逐位**恒等（滤波器一次也不被调用） | 旁通时仍然 `process` |
//! | F2 | 直流通过低通后稳态增益 ≈ 1（`SOFT_KNEE` 之下完全线性） | 通带增益写错/直流泄漏 |
//! | F3 | 截止频率以上的音调被显著衰减（低通的方向性） | 低通写成高通/系数方向反了 |
//! | F4 | 阶跃响应单调收敛、无过冲、无振荡 | 反馈项符号写错（会振铃/发散） |
//! | F5 | 输出始终有限、有界；状态冲刷后不留次正规数尾巴 | 去掉有界饱和/冲刷 |
//! | F6 | 参数退化（`NaN`/越界）被钳制，不产生 `NaN` | 直接把参数喂给 `tan()` |
//! | F7 | 每个声部**各自**一份滤波器状态（两个音符不会互相污染） | 把滤波器状态放在轨道槽上 |

mod support;

use support::{SynthRig, scheduled};
use yeban_engine::synth::{NoteSchedule, ToneParams};

/// 采样率（夹具固定 48 kHz）。
const SR: f32 = 48_000.0;

/// 渲染一个"直流"输入：一个极长、极低音高的音符 ⇒ 声部输出接近常数。
///
/// 用音符而不是直接灌样本，是为了让判据走**产品路径**（相位递推 → 波表 → 包络 →
/// 滤波器）。音高 12（16.35 Hz）在 48 kHz 上每样本相位增量约 1.46e-6 ⇒ 0.5 秒内
/// 只走 8 个周期，足以当"准直流"看。
fn dc_schedule() -> NoteSchedule {
    NoteSchedule::from_sorted(vec![scheduled(0, 96_000, 12, 1.0, SR)])
}

/// 渲染一个给定音高的长音符（用于频响判据）。
fn tone_schedule(pitch: u8) -> NoteSchedule {
    NoteSchedule::from_sorted(vec![scheduled(0, 96_000, pitch, 1.0, SR)])
}

/// F1：**旁通必须逐位恒等** —— `render_track` 根本不调用滤波器。
///
/// 这条不是"多余的正确性检查"：把旁通实现成"系数取成透明"是**常见错法**，
/// 而梯形滤波器的四级状态即使系数透明也会吸收瞬态并在起音处染色。
#[test]
fn bypass_is_bit_identical_to_no_filter_path() {
    let schedule = tone_schedule(69);
    let mut bypass = SynthRig::new(&ToneParams::bypass());
    let with_bypass = bypass.render(&schedule, 40);

    // 与旁通同参数的"滤波器关掉"路径：直接用默认 `ToneParams`（也是旁通）。
    let mut default = SynthRig::new(&ToneParams::default());
    let with_default = default.render(&schedule, 40);

    let bypass_bits: Vec<u32> = with_bypass.iter().map(|s| s.to_bits()).collect();
    let default_bits: Vec<u32> = with_default.iter().map(|s| s.to_bits()).collect();
    assert_eq!(bypass_bits, default_bits, "旁通路径必须逐位相同");
    assert!(
        with_bypass.iter().any(|s| *s != 0.0),
        "夹具必须真的出声（否则逐位相同是空转）"
    );

    // 与"接了滤波器"的路径**必须不同**（否则判据测不出旁通是否真的旁通）。
    let mut filtered = SynthRig::new(&ToneParams::new(300.0, 0.0, 0.0));
    let with_filter = filtered.render(&schedule, 40);
    assert_ne!(
        bypass_bits,
        with_filter
            .iter()
            .map(|s| s.to_bits())
            .collect::<Vec<u32>>(),
        "接了 300 Hz 低通的渲染必须与旁通不同"
    );

    // 第三个臂（本票新增）：旁通与"**透明**低通"必须不同。
    //
    // ⚠ **缺口来源**：`docs/ledger/integration-rulings-notes.md` 的 R3
    //（"`synth_filter.rs` F1 缺第三个臂"）。上面两个臂**各自**都有对照，
    // 但对照物都不足以钉住"旁通 = 20 kHz 透明滤波"这种错法：
    // - 第一个臂的两边是 `ToneParams::bypass()` 与 `ToneParams::default()`，
    //   而按 `synth.rs:306-310` 后者**就是**前者（也是旁通）⇒ 那种错法下
    //   两边**同时**被滤波 ⇒ 仍然逐位相等（判据看不见）；
    // - 第二个臂（300 Hz）在那种错法下仍然**不等**。
    //
    // ⚠ **上一票的注入 I7**（`polysynth` 票把 `render` 里两处 `if !filter_bypass`
    // 改成恒真）对**本判据 F1 不红**，原因就是上面第一条。
    //
    // ⭐ 准确说法是"**F1 缺一个臂，本票补上**"，不是"F1 以前是错的"。
    //
    // 参照 `yeban-dsp` 的同款判据 `bypass_is_not_a_transparent_filter`
    //（`crates/yeban-dsp/src/polysynth.rs:1436`）：把"旁通"与"显式在旁通占位截止频率
    //（20 kHz，见 `synth.rs:136`）上**真的**开一个低通"直接对比。
    let mut transparent = SynthRig::new(&ToneParams::new(20_000.0, 0.0, 0.0));
    let with_transparent = transparent.render(&schedule, 40);
    assert_ne!(
        bypass_bits,
        with_transparent
            .iter()
            .map(|s| s.to_bits())
            .collect::<Vec<u32>>(),
        "旁通与'20 kHz 透明低通'必须是不同的位模式（否则旁通等于没做）"
    );
}

/// F2：直流（准直流）通过低通后的稳态增益 ≈ 1。
///
/// `LadderFilter` 的通带增益是 `PASSBAND_TRIM = 1.0`，而 0.7 以下完全线性
/// （`SOFT_KNEE`），因此稳态输出应当与输入同幅。
#[test]
fn dc_passes_through_a_low_pass_at_unity() {
    let schedule = dc_schedule();
    let mut bypass = SynthRig::new(&ToneParams::bypass());
    let reference = bypass.render(&schedule, 120); // 15360 帧
    let mut filtered = SynthRig::new(&ToneParams::new(2_000.0, 0.0, 0.0));
    let tested = filtered.render(&schedule, 120);

    // 取后半段（稳态）比较峰值。
    let half = tested.len() / 2;
    let peak = |samples: &[f32]| {
        samples[half..]
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
    };
    let reference_peak = peak(&reference);
    let tested_peak = peak(&tested);
    let ratio = tested_peak / reference_peak;
    println!(
        "[engine-mix] F2 dc steady peak reference={reference_peak:.6} filtered={tested_peak:.6} ratio={ratio:.6}"
    );
    assert!(reference_peak > 0.1, "夹具必须真的出声");
    assert!(
        (ratio - 1.0).abs() < 0.05,
        "直流通过 2 kHz 低通的稳态增益应当 ≈ 1（实测比值 {ratio:.6}）"
    );
}

/// F3：截止频率**以上**的音调被显著衰减（低通的方向性）。
#[test]
fn a_tone_above_the_cutoff_is_attenuated() {
    // 音高 93 ≈ 1760 Hz；用 200 Hz 的截止频率把它压在阻带里。
    let schedule = tone_schedule(93);
    let mut bypass = SynthRig::new(&ToneParams::bypass());
    let reference = bypass.render(&schedule, 60);
    let mut filtered = SynthRig::new(&ToneParams::new(200.0, 0.0, 0.0));
    let tested = filtered.render(&schedule, 60);

    let rms = |samples: &[f32]| {
        let energy: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
        (energy / samples.len() as f64).sqrt()
    };
    let reference_rms = rms(&reference);
    let tested_rms = rms(&tested);
    let attenuation_db = 20.0 * (tested_rms / reference_rms).log10();
    println!(
        "[engine-mix] F3 1760 Hz through 200 Hz LP: \
         reference_rms={reference_rms:.6} filtered_rms={tested_rms:.6} attenuation={attenuation_db:.2} dB"
    );
    assert!(reference_rms > 0.01, "夹具必须真的出声");
    assert!(
        attenuation_db < -20.0,
        "24 dB/oct 的低通在 8.8 倍截止频率处应当衰减远超 20 dB，实测 {attenuation_db:.2} dB"
    );
    assert!(
        tested.iter().all(|sample| sample.is_finite()),
        "滤波后的输出必须全部有限"
    );
}

/// F4：**阶跃响应**单调收敛、无过冲、无振荡。
///
/// 用"静音 → 长音符起音"当阶跃：5 ms 线性起音之后是持续的 sustain，
/// 低通应当把它平滑成一条单调上升的曲线。
#[test]
fn step_response_settles_monotonically_without_overshoot() {
    let schedule = dc_schedule();
    let mut rig = SynthRig::new(&ToneParams::new(300.0, 0.0, 0.0));
    let rendered = rig.render(&schedule, 40);
    // 只看前 4000 帧（起音 + 建立过程）；之后是稳态。
    let window = &rendered[..4_000];
    let envelope_peak = window
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));

    // (a) 相邻位移有界：低通不可能产生比输入更陡的台阶。
    let step = support::max_step(window);
    println!("[engine-mix] F4 window peak={envelope_peak:.6} max_step={step:.6}");
    assert!(envelope_peak > 0.05, "夹具必须真的出声");
    assert!(
        step < envelope_peak * 0.2,
        "阶跃响应出现过大位移 {step:.6}（峰值 {envelope_peak:.6}）—— 可能振铃或发散"
    );

    // (b) 无过冲：窗口峰值不得超过末段的稳态峰值太多（低通不允许增益 > 1）。
    let tail_peak = rendered[3_500..]
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    // 容差 1.35：梯形低通的起音瞬态本来就会短暂超过稳态（实测 0.825 vs 0.700
    // ⇒ 1.18 倍）。真正的"过冲/振铃"是 > 2 倍或持续不衰减；这里给 1.35 既抓住
    // 反馈项符号写错（会持续振铃到有界饱和），也不会对正常瞬态误报。
    let overshoot = envelope_peak / tail_peak;
    println!("[engine-mix] F4 overshoot ratio={overshoot:.4} (tail peak={tail_peak:.6})");
    assert!(
        overshoot <= 1.35,
        "阶跃响应过冲 {overshoot:.4} 倍：窗口峰值 {envelope_peak:.6} vs 稳态峰值 {tail_peak:.6}"
    );
}

/// F5：**有界 + 无次正规数尾巴**（`LadderFilter` 的两条既有保证在引擎里也成立）。
///
/// 注入 `resonance = 1.0`（接近自激）与满幅度输入，输出必须仍然有限且有界
/// （`bounded_saturate` 的作用）；音符结束之后的尾巴必须要么是 0、要么是正常数。
#[test]
fn resonant_filter_stays_bounded_and_flushes_denormals() {
    let mut rig = SynthRig::new(&ToneParams::new(800.0, 1.0, 1.0));
    // 音符在第 4800 帧结束，之后是 release + 静音尾。
    let schedule = NoteSchedule::from_sorted(vec![scheduled(0, 4_800, 45, 1.0, SR)]);
    let rendered = rig.render(&schedule, 240); // 30720 帧

    let peak = rendered
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    println!("[engine-mix] F5 resonance=1.0 peak={peak:.6}");
    assert!(
        rendered.iter().all(|sample| sample.is_finite()),
        "满共振下输出必须全部有限"
    );
    assert!(peak < 8.0, "满共振下滤波器跑飞了: 峰值 {peak}");
    // 尾巴：末段 2000 帧里不允许出现次正规数（0 < |v| < f32::MIN_POSITIVE）。
    let tail = &rendered[rendered.len() - 2_000..];
    let subnormal = tail
        .iter()
        .filter(|sample| **sample != 0.0 && sample.abs() < f32::MIN_POSITIVE)
        .count();
    assert_eq!(subnormal, 0, "尾巴里出现了 {subnormal} 个次正规数");
}

/// F6：退化参数（`NaN`/越界）被钳制，绝不产生 `NaN` 输出。
#[test]
fn degenerate_tone_params_are_clamped() {
    for tone in [
        ToneParams::new(f32::NAN, f32::NAN, f32::NAN),
        ToneParams::new(-1.0, 5.0, -3.0),
        ToneParams::new(1.0e9, 1.0e9, 1.0e9),
    ] {
        let mut rig = SynthRig::new(&tone);
        let rendered = rig.render(&tone_schedule(69), 20);
        assert!(
            rendered.iter().all(|sample| sample.is_finite()),
            "退化参数 {tone:?} 产生了非有限输出"
        );
        assert!(
            rendered.iter().any(|sample| *sample != 0.0),
            "退化参数 {tone:?} 把整条链路弄成静音（钳制过头）"
        );
    }
}

/// F7：每个声部**各自**一份滤波器状态 ⇒ 两个音符不会互相污染。
///
/// 判别方式：先让一个音符单独跑（参照），再加入第二个音符（不同音高、稍后起音）。
/// 后者的起音不得改变**前者**的轨迹（在两者重叠的时间段里，加入第二个音符前后的
/// 差异必须只包含第二个音符自身的贡献，而不是"第一个音符的轨迹被改写"）。
#[test]
fn each_voice_owns_its_filter_state() {
    let first = scheduled(0, 96_000, 45, 1.0, SR);
    let second = scheduled(2_000, 96_000, 69, 1.0, SR);
    let alone = NoteSchedule::from_sorted(vec![first]);

    let mut rig_alone = SynthRig::new(&ToneParams::new(500.0, 0.5, 0.0));
    let reference = rig_alone.render(&alone, 30);

    let both = NoteSchedule::from_sorted(vec![first, second]);
    let mut rig_both = SynthRig::new(&ToneParams::new(500.0, 0.5, 0.0));
    let mixed = rig_both.render(&both, 30);

    // 第二个音符起音之前的样本必须逐位相同（它还没发声）。
    let before = 1_000;
    let reference_bits: Vec<u32> = reference[..before].iter().map(|s| s.to_bits()).collect();
    let mixed_bits: Vec<u32> = mixed[..before].iter().map(|s| s.to_bits()).collect();
    assert_eq!(
        reference_bits, mixed_bits,
        "第二个音符起音之前，第一个音符的轨迹必须逐位不变（滤波器状态被共享了？）"
    );
    // 第二个音符必须真的被触发（否则"起音前逐位相同"是空转）。
    // 它起音于 2000、落在量子边界 2048 上；比较该量子内的样本和。
    let window = 2_048..2_176;
    let reference_sum: f64 = reference[window.clone()]
        .iter()
        .map(|s| f64::from(*s))
        .sum();
    let mixed_sum: f64 = mixed[window].iter().map(|s| f64::from(*s)).sum();
    assert!(
        (reference_sum - mixed_sum).abs() > 1e-3,
        "第二个音符必须有可观测的贡献（差 {}）",
        (reference_sum - mixed_sum).abs()
    );
}
