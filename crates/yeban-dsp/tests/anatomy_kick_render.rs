//! Anatomy 底鼓的渲染判据：三层量测、预设、速度-音色映射、确定性。
//!
//! 量法与 `drums_render.rs` 同款（`dft_bin` / `band_fraction` /
//! `decay60_s` / 前沿滑窗包络都是从那里抄来的同一实现）；读数工具在文件头，
//! 判据在后。峰值与频带份额的界先由手算定、再由实测收紧（见各判据的注释）。

use yeban_dsp::drums::{
    ANATOMY_KICK_PRESETS, AnatomyKickParams, AnatomyKickPreset, DRUM_SLOTS, DrumHit, DrumKitParams,
    DrumMachine, DrumVoice, KickStyle,
};

const SR: f64 = 48_000.0;
/// 每次渲染的帧数（1 s：长于所有预设的 −60 dB 尾巴）。
const FRAMES: usize = 48_000;
/// 频谱分析用的前多少帧（85 ms：短到包络还没走完、长到频率分辨率够用）。
const ANALYSIS_FRAMES: usize = 4_096;

// ---------------------------------------------------------------------------
// 读数工具（都是测量工具，不是被测代码；与 `drums_render.rs` 同款）
// ---------------------------------------------------------------------------

/// 绝对值峰值（无量纲）。
fn peak(samples: &[f32]) -> f64 {
    samples
        .iter()
        .fold(0.0f64, |acc, sample| acc.max(f64::from(*sample).abs()))
}

/// 单 bin DFT 幅度（矩形窗；单位：与输入同量纲的幅度）。
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

/// **前沿**滑窗（`env[i] = max|x[i .. i+window]|`）绝对值最大值包络。
fn envelope(samples: &[f32], window: usize) -> Vec<f64> {
    let count = samples.len();
    let mut out = vec![0.0f64; count];
    for (index, slot) in out.iter_mut().enumerate() {
        let high = (index + window).min(count);
        let mut best = 0.0f64;
        for sample in &samples[index..high] {
            let value = f64::from(*sample).abs();
            if value > best {
                best = value;
            }
        }
        *slot = best;
    }
    out
}

/// 衰减拟合用的包络窗（帧）：20 ms。
fn decay_window(sample_rate: f64) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let window = (sample_rate * 0.020) as usize;
    window.max(1)
}

/// 对数线性拟合的 −60 dB 时间（秒）：包络在 −10 dB 到 −40 dB 之间拟合直线，
/// 外推到 −60 dB。`None` = 拟合样本不足或斜率非负。
fn decay60_s(samples: &[f32], sample_rate: f64, from_s: f64) -> Option<f64> {
    let level = peak(samples);
    if level <= 0.0 {
        return None;
    }
    let env = envelope(samples, decay_window(sample_rate));
    let high_band = level * 0.316_227_77; // −10 dB
    let low_band = level * 0.01; // −40 dB
    let mut count = 0.0f64;
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_xx = 0.0f64;
    let mut sum_xy = 0.0f64;
    for (index, value) in env.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let x = index as f64 / sample_rate;
        if x < from_s || *value > high_band || *value < low_band {
            continue;
        }
        let y = value.ln();
        count += 1.0;
        sum_x += x;
        sum_y += y;
        sum_xx += x * x;
        sum_xy += x * y;
    }
    if count < 64.0 {
        return None;
    }
    let denominator = count * sum_xx - sum_x * sum_x;
    if denominator.abs() < 1.0e-18 {
        return None;
    }
    let slope = (count * sum_xy - sum_x * sum_y) / denominator;
    if slope >= 0.0 {
        return None;
    }
    let intercept = (sum_y - slope * sum_x) / count;
    let target = (level * 0.001).ln();
    Some((target - intercept) / slope)
}

/// 线性网格上 `[low_hz, high_hz]` 的幅度和与全带（20 Hz–16 kHz）幅度和之比。
/// 网格步进 20 Hz；返回 `None` 表示全带能量为 0（静音）。
fn band_fraction(samples: &[f32], sample_rate: f64, low_hz: f64, high_hz: f64) -> Option<f64> {
    let mut inside = 0.0f64;
    let mut total = 0.0f64;
    let mut freq = 20.0f64;
    while freq < 16_000.0 {
        let magnitude = dft_bin(samples, freq, sample_rate);
        total += magnitude;
        if freq >= low_hz && freq <= high_hz {
            inside += magnitude;
        }
        freq += 20.0;
    }
    if total <= 0.0 {
        None
    } else {
        Some(inside / total)
    }
}

/// 短窗（1 ms）前沿包络第一次跌破峰值 `0.001` 倍的时刻（秒）。
/// 给 click 层这种 10 ms 量级的衰减用（20 ms 拟合窗会把它抹平）。
/// `None` = 1 s 内没跌破。
fn crossing60_s(samples: &[f32], sample_rate: f64) -> Option<f64> {
    let level = peak(samples);
    if level <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let window = (sample_rate * 0.001) as usize;
    let env = envelope(samples, window.max(1));
    let threshold = level * 0.001;
    for (index, value) in env.iter().enumerate() {
        if index == 0 {
            continue;
        }
        if *value < threshold {
            #[allow(clippy::cast_precision_loss)]
            return Some(index as f64 / sample_rate);
        }
    }
    None
}

/// 打印一行读数（单位写在行里）。
fn report(label: &str, text: &str) {
    println!("[yeban-dsp/anatomy-kick] {label:<26} {text}");
}

// ---------------------------------------------------------------------------
// 渲染夹具
// ---------------------------------------------------------------------------

/// 给定鼓组渲染一记 Anatomy 底鼓（`frames` 帧，`gain` 为速度）。
fn render_anatomy(kit: DrumKitParams, frames: usize, gain: f32) -> Vec<f32> {
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    machine.set_params(kit);
    machine.trigger(DrumHit::new(DrumVoice::Kick, 0, gain));
    let mut out = vec![0.0f32; frames];
    machine.render(0, &mut out);
    out
}

/// 给定 Anatomy 参数渲染一记（`kick_style = Anatomy`，其余取器件默认）。
fn render_anatomy_params(params: AnatomyKickParams, gain: f32) -> Vec<f32> {
    let mut kit = DrumKitParams::DEFAULT;
    kit.kick_style = KickStyle::Anatomy;
    kit.anatomy = params;
    render_anatomy(kit, FRAMES, gain)
}

/// 给定预设渲染一记。
fn render_preset(preset: &AnatomyKickPreset, gain: f32) -> Vec<f32> {
    render_anatomy_params(preset.params, gain)
}

// ---------------------------------------------------------------------------
// A1：默认预设真的发声、有限、峰值在手算带内
// ---------------------------------------------------------------------------

/// 量什么：默认预设一击的峰值（无量纲）与非零样本数。
///
/// 手算：sub 峰值 1.25、thump 峰值 1.1、click 峰值 0.6175，四层相加后过
/// `k = 2.5` 的 tanh 失真器。失真器的输出上界**不是** 1，而是
/// `1/tanh(k) = 1.0136`（曲线按 `tanh(k)` 归一化，输入超过 ±1 时输出趋向
/// 该值）；实测 1.0130 正在渐近线上。界取 (0.5, 1.02]。
#[test]
fn a1_default_preset_really_sounds() {
    let samples = render_anatomy_params(AnatomyKickParams::DEFAULT, 1.0);
    let peak_value = peak(&samples);
    let nonzero = samples.iter().filter(|s| **s != 0.0).count();
    let bad = samples.iter().filter(|s| !s.is_finite()).count();
    report(
        "默认预设峰值 / 非零 / 非有限",
        &format!("{peak_value:.4} / {nonzero} 帧 / {bad} 个"),
    );
    assert_eq!(bad, 0, "非有限样本必须为 0");
    assert!(nonzero > 0, "必须真的发声");
    assert!(
        (0.5..=1.02).contains(&peak_value),
        "峰值 {peak_value} 不在手算带 (0.5, 1.02] 里（tanh 失真器输出上界是 1/tanh(2.5) = 1.0136）"
    );
}

// ---------------------------------------------------------------------------
// A2：三段能量占比 —— Sub 主导、Thump 居中、Click 少量但在场
// ---------------------------------------------------------------------------

/// 量什么：前 85 ms 里 Sub（20–120 Hz）/ Thump（100–500 Hz）/
/// Click（1–4 kHz）三段的幅度份额。
///
/// 手算：sub 峰值 1.25、衰减 0.695 s（85 ms 窗里几乎没衰减）；
/// thump 峰值 1.1、衰减 0.158 s（窗里衰减过半）；click 峰值 0.6175、
/// 衰减 0.01255 s（窗里只剩前 15 ms）。因此 Sub 份额最大、Click 最小、
/// 三段都不为 0。
#[test]
fn a2_three_band_shares() {
    let samples = render_anatomy_params(AnatomyKickParams::DEFAULT, 1.0);
    let head = &samples[..ANALYSIS_FRAMES];
    let sub = band_fraction(head, SR, 20.0, 120.0).expect("非静音 ⇒ 全带能量 > 0");
    let thump = band_fraction(head, SR, 100.0, 500.0).expect("非静音 ⇒ 全带能量 > 0");
    let click = band_fraction(head, SR, 1_000.0, 4_000.0).expect("非静音 ⇒ 全带能量 > 0");
    report(
        "三段份额 Sub/Thump/Click",
        &format!("{sub:.3} / {thump:.3} / {click:.3}"),
    );
    assert!(sub > 0.3, "Sub 份额 {sub} 必须主导（> 0.3）");
    assert!(sub > thump, "Sub 份额必须大于 Thump 份额");
    assert!(thump > click, "Thump 份额必须大于 Click 份额");
    assert!(click > 0.01, "Click 份额 {click} 必须在场（> 0.01）");
}

// ---------------------------------------------------------------------------
// A3：独奏 / 静音隔离 —— 各层只在自己该在的频带里
// ---------------------------------------------------------------------------

/// 量什么：sub 独奏时 Click 带（1–4 kHz）的份额；click 独奏时 Sub 带
///（20–120 Hz）的份额。
///
/// 手算：sub 独奏只剩 sub（48–116 Hz 正弦）+ rumble（36 Hz），但总线
/// tanh 失真器（`k = 2.5`）把 1.25 幅度的正弦削成近方波，奇次谐波按约
/// `1/n` 衰减 ⇒ 1–4 kHz 带仍有约 0.08 的幅度份额（实测 0.078），
/// 界取 < 0.12；click 独奏只剩方波经 Q 4.5、中心 2440 Hz 的带通，
/// 100 Hz 处的裙边衰减 > 60 dB ⇒ Sub 带份额 < 0.05。
#[test]
fn a3_solo_isolates_each_layer_to_its_band() {
    let mut sub_solo = AnatomyKickParams::DEFAULT;
    sub_solo.sub_solo = true;
    let sub_only = render_anatomy_params(sub_solo, 1.0);
    let sub_head = &sub_only[..ANALYSIS_FRAMES];
    let click_share = band_fraction(sub_head, SR, 1_000.0, 4_000.0).expect("非静音 ⇒ 全带能量 > 0");
    report("sub 独奏的 Click 带份额", &format!("{click_share:.4}"));
    assert!(
        click_share < 0.12,
        "sub 独奏时 Click 带份额 {click_share} 必须 < 0.12（tanh 谐波）"
    );

    let mut click_solo = AnatomyKickParams::DEFAULT;
    click_solo.click_solo = true;
    let click_only = render_anatomy_params(click_solo, 1.0);
    let click_head = &click_only[..ANALYSIS_FRAMES];
    let sub_share = band_fraction(click_head, SR, 20.0, 120.0).expect("非静音 ⇒ 全带能量 > 0");
    report("click 独奏的 Sub 带份额", &format!("{sub_share:.4}"));
    assert!(
        sub_share < 0.05,
        "click 独奏时 Sub 带份额 {sub_share} 必须 < 0.05"
    );
}

// ---------------------------------------------------------------------------
// A4：衰减 —— −60 dB 时间与 sub 包络的闭式一致
// ---------------------------------------------------------------------------

/// 量什么：默认预设一击的 −60 dB 时间（秒，`decay60_s` 拟合）。
///
/// 手算：中段由 sub 包络主导（峰值 1.25 → 80 ms 处 `0.45·峰值` → 指数到
/// `0.0001`（0.695 s 处），尾段斜率约 `−14.04 /s`）；深尾由 rumble 主导
///（衰减 0.7645 s、斜率约 `−10.18 /s`，比 sub 慢）⇒ −60 dB 落在约
/// 0.64 s（实测 0.637 s）。界取 [0.50, 0.75]。
#[test]
fn a4_decay_matches_the_sub_envelope_closed_form() {
    let samples = render_anatomy_params(AnatomyKickParams::DEFAULT, 1.0);
    let decay = decay60_s(&samples, SR, 0.0).expect("拟合样本充足 ⇒ 必须有读数");
    report("默认预设 −60 dB 时间", &format!("{decay:.3} s"));
    assert!(
        (0.50..=0.75).contains(&decay),
        "−60 dB 时间 {decay} s 不在手算带 [0.50, 0.75] s 里"
    );
}

// ---------------------------------------------------------------------------
// A5：9 套预设各不相同、都发声、都有限
// ---------------------------------------------------------------------------

/// 量什么：9 套预设各自的峰值（无量纲）与两两是否逐位不同。
///
/// 判据：每套峰值 > 0.05、全部有限；任意两套的渲染不逐位相同
///（参数不同 ⇒ 输出不同；若有两套逐位相同，说明某套参数没进声部）。
#[test]
fn a5_all_nine_presets_sound_and_differ() {
    let mut renders: Vec<(String, Vec<f32>)> = Vec::new();
    for preset in &ANATOMY_KICK_PRESETS {
        let samples = render_preset(preset, 1.0);
        let peak_value = peak(&samples);
        let bad = samples.iter().filter(|s| !s.is_finite()).count();
        report(
            &format!("预设 {} 峰值 / 非有限", preset.id),
            &format!("{peak_value:.4} / {bad} 个"),
        );
        assert_eq!(bad, 0, "{} 产出非有限样本", preset.id);
        assert!(peak_value > 0.05, "{} 的峰值 {peak_value} 太小", preset.id);
        renders.push((preset.id.to_string(), samples));
    }
    // 默认预设的渲染与 anatomy-default 预设逐位相同（同一组参数）。
    let default_render = render_anatomy_params(AnatomyKickParams::DEFAULT, 1.0);
    let anatomy_default = renders
        .iter()
        .find(|(id, _)| id == "anatomy-default")
        .expect("表里必须有 anatomy-default");
    assert_eq!(
        default_render, anatomy_default.1,
        "默认参数与 anatomy-default 预设必须逐位相同"
    );
    for i in 0..renders.len() {
        for j in (i + 1)..renders.len() {
            assert_ne!(
                renders[i].1, renders[j].1,
                "预设 {} 与 {} 的渲染逐位相同 —— 某套参数没进声部",
                renders[i].0, renders[j].0
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A6：速度-音色映射 —— 低速击的 click 尾巴比高速击长
// ---------------------------------------------------------------------------

/// 量什么：click 独奏、速度 0.2 与 1.0 两击的 −60 dB 穿越时间（秒，1 ms 窗）。
///
/// 手算（`gripScale(0.2) ≈ 0.7213` ⇒ `effectiveSoftness ≈ 0.4853`、
/// `decayScale = 1.2`）：click 衰减 0.01392 s（低速）vs 0.01255 s（高速）
/// ⇒ 穿越时间低速比高速长约 11%。界取"低速至少比高速长 5%"。
#[test]
fn a6_velocity_mapping_lengthens_the_soft_click_tail() {
    let mut click_solo = AnatomyKickParams::DEFAULT;
    click_solo.click_solo = true;
    let hard = render_anatomy_params(click_solo, 1.0);
    let soft = render_anatomy_params(click_solo, 0.2);
    let hard_t = crossing60_s(&hard, SR).expect("click 独奏必须跌破 −60 dB");
    let soft_t = crossing60_s(&soft, SR).expect("click 独奏必须跌破 −60 dB");
    report(
        "click −60 dB 穿越 高速/低速",
        &format!("{hard_t:.4} s / {soft_t:.4} s"),
    );
    assert!(
        soft_t > hard_t * 1.05,
        "低速击的 click 尾巴 {soft_t} s 必须比高速击 {hard_t} s 长 5% 以上"
    );
    assert_ne!(hard, soft, "两击的波形必须不同（不只是电平）");
}

// ---------------------------------------------------------------------------
// A7：确定性 —— reset 复现逐位相同、块切分无关
// ---------------------------------------------------------------------------

/// 量什么：同一串触发在"新建两台"与"reset 后重放"下是否逐位相同；
/// 128 帧一块与整块一次是否逐位相同。
#[test]
fn a7_reset_and_block_split_are_bit_exact() {
    let kit = {
        let mut kit = DrumKitParams::DEFAULT;
        kit.kick_style = KickStyle::Anatomy;
        kit.anatomy = AnatomyKickParams::DEFAULT;
        kit
    };
    let render_once = |machine: &mut DrumMachine<DRUM_SLOTS>| {
        machine.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
        machine.trigger(DrumHit::new(DrumVoice::Kick, 24_000, 0.5));
        let mut out = vec![0.0f32; FRAMES];
        machine.render(0, &mut out);
        out
    };
    // 新建两台。
    let mut first = DrumMachine::<DRUM_SLOTS>::new(48_000);
    first.set_params(kit);
    let first_out = render_once(&mut first);
    let mut second = DrumMachine::<DRUM_SLOTS>::new(48_000);
    second.set_params(kit);
    let second_out = render_once(&mut second);
    assert_eq!(first_out, second_out, "两台新建器件必须逐位相同");
    // reset 后重放。
    first.reset();
    let replayed = render_once(&mut first);
    assert_eq!(first_out, replayed, "reset 后重放必须逐位相同");
    // 128 帧一块。
    let mut chunked = DrumMachine::<DRUM_SLOTS>::new(48_000);
    chunked.set_params(kit);
    chunked.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
    chunked.trigger(DrumHit::new(DrumVoice::Kick, 24_000, 0.5));
    let mut chunked_out = vec![0.0f32; FRAMES];
    let mut position = 0usize;
    while position < FRAMES {
        let len = 128usize.min(FRAMES - position);
        chunked.render(position as u64, &mut chunked_out[position..position + len]);
        position += len;
    }
    assert_eq!(first_out, chunked_out, "128 帧分块必须与整块一次逐位相同");
    report("确定性：新建/reset/分块", "三路逐位相同");
}

// ---------------------------------------------------------------------------
// A8：窃取 —— 17 记 Anatomy 底鼓只窃取 1 次、输出全有限
// ---------------------------------------------------------------------------

/// 量什么：池满（16 槽）后第 17 记的 `voice_steals` 读数与输出有限性。
#[test]
fn a8_stealing_an_anatomy_kick_stays_finite() {
    let mut kit = DrumKitParams::DEFAULT;
    kit.kick_style = KickStyle::Anatomy;
    kit.anatomy = AnatomyKickParams::DEFAULT;
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    machine.set_params(kit);
    for _ in 0..17 {
        machine.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
    }
    let mut out = vec![0.0f32; FRAMES];
    machine.render(0, &mut out);
    let bad = out.iter().filter(|s| !s.is_finite()).count();
    report(
        "窃取 17 记后的 voice_steals / 非有限",
        &format!("{} 次 / {bad} 个", machine.voice_steals()),
    );
    assert_eq!(machine.voice_steals(), 1, "第 17 记必须恰好窃取 1 次");
    assert_eq!(bad, 0, "窃取后输出必须全有限");
    assert!(peak(&out) > 0.05, "窃取后仍必须发声");
}

// ---------------------------------------------------------------------------
// A9：经典声部不受影响 —— 默认鼓组的经典底鼓仍发声、仍有限
// ---------------------------------------------------------------------------

/// 量什么：经典底鼓（默认鼓组、不写 `kick_style`）一击的峰值与有限性。
///
/// 这条是轻量护栏：经典声部的逐位读数由 `drums_render.rs` 的 K1/K2 与
/// `frozen_deliverables.rs` 钉住；这里只确认"默认仍是经典、仍发声"。
#[test]
fn a9_classic_kick_is_still_the_default_and_sounds() {
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    machine.set_params(DrumKitParams::DEFAULT);
    assert_eq!(
        machine.params().kick_style,
        KickStyle::Classic,
        "缺省声部必须是 Classic"
    );
    machine.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
    let mut out = vec![0.0f32; FRAMES];
    machine.render(0, &mut out);
    let peak_value = peak(&out);
    let bad = out.iter().filter(|s| !s.is_finite()).count();
    report(
        "经典底鼓峰值 / 非有限",
        &format!("{peak_value:.4} / {bad} 个"),
    );
    assert_eq!(bad, 0);
    assert!(peak_value > 0.05, "经典底鼓必须发声");
}
