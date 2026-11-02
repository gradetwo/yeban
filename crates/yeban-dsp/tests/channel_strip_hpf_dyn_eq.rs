//! 通道条新增两级的判据：低切（`#341`）与动态 EQ（`#342`）。
//!
//! 三条要证明的事：
//!
//! 1. **低切真的切**：转折点以下按 `12 dB/octave` 掉，转折点以上不动。
//! 2. **动态 EQ 只压自己那一段**：带内超阈值被压，带外一位不动。
//! 3. **两级都不在的时候逐位透明**：默认参数（低切旁通、动态 EQ 旁通）下，
//!    一条只开增益的通道条就是恒等变换 —— 这是"加级不改变既有工程"的证据。
//!
//! 判据用正弦测**稳态增益**，不用瞬态：两级都是最小相位滤波器 + 一阶包络，
//! 稳态是它们唯一有解析期望值的工作点。

use yeban_dsp::channel_strip::{ChannelStrip, ChannelStripParams, DynEqParams, HpfParams};

const SR: f32 = 48_000.0;

/// 一条只留增益级的通道条：EQ / 滤波 / 压缩全部关掉，低切与动态 EQ 旁通。
fn transparent_strip() -> ChannelStripParams {
    let mut p = ChannelStripParams::DEFAULT;
    p.input_gain_db = 0.0;
    p.output_gain_db = 0.0;
    p.eq_enabled = false;
    p.filter_enabled = false;
    p.compressor_enabled = false;
    p.hpf = HpfParams::DEFAULT; // freq = 20 Hz = 旁通
    p.dyn_eq = DynEqParams::DEFAULT; // max_cut_db = 0 = 旁通
    p
}

fn sine(freq: f32, amp: f32, seconds: f32) -> Vec<f32> {
    let n = (SR * seconds) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            amp * (core::f32::consts::TAU * freq * t).sin()
        })
        .collect()
}

/// 把一段单声道信号跑过一条通道条，返回（输入 RMS, 输出 RMS），单位 dBFS。
fn run(params: ChannelStripParams, input: &[f32]) -> (f64, f64) {
    let mut strip = ChannelStrip::new(params, SR);
    let mut buf = input.to_vec();
    strip.process_mono(&mut buf);
    // 丢掉前 100 ms：两级都有起振，稳态在后面。
    let skip = (SR * 0.1) as usize;
    (rms_db(&input[skip..]), rms_db(&buf[skip..]))
}

fn rms_db(x: &[f32]) -> f64 {
    let sum: f64 = x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
    10.0 * (sum / x.len().max(1) as f64 + 1e-30).log10()
}

#[test]
fn the_hpf_cuts_below_the_corner_and_leaves_everything_above_it() {
    let mut p = transparent_strip();
    p.hpf = HpfParams { freq_hz: 800.0 };

    // 100 Hz：低于转折点三个八度 ⇒ 二阶高通给 −36 dB 的渐近值。
    let low = sine(100.0, 0.5, 1.0);
    let (_, out) = run(p, &low);
    let cut = out - rms_db(&low[4800..]);
    assert!(
        cut < -25.0,
        "800 Hz 低切对 100 Hz 只掉了 {cut:.1} dB，二阶高通应为 −30 dB 上下"
    );

    // 5 kHz：远在通带里 ⇒ 应该一位不动（此处只允许滤波器的数值残差）。
    let high = sine(5_000.0, 0.5, 1.0);
    let (_, out) = run(p, &high);
    let kept = out - rms_db(&high[4800..]);
    assert!(
        kept.abs() < 0.2,
        "800 Hz 低切不该动 5 kHz，实测 {kept:+.2} dB"
    );
}

#[test]
fn a_hpf_at_or_below_the_bypass_frequency_is_a_no_op() {
    // 20 Hz 与 19 Hz 都落在旁通区（19 Hz 被钳到 20）⇒ 输出逐位相同。
    let input = sine(120.0, 0.5, 0.5);
    let mut a = ChannelStrip::new(
        ChannelStripParams {
            hpf: HpfParams { freq_hz: 20.0 },
            ..transparent_strip()
        },
        SR,
    );
    let mut b = ChannelStrip::new(
        ChannelStripParams {
            hpf: HpfParams { freq_hz: 19.0 },
            ..transparent_strip()
        },
        SR,
    );
    let mut x = input.clone();
    let mut y = input.clone();
    a.process_mono(&mut x);
    b.process_mono(&mut y);
    assert_eq!(x, y, "旁通区的两个低切频率必须给出逐位相同的输出");
}

#[test]
fn the_dynamic_eq_cuts_its_own_band_and_leaves_the_rest_alone() {
    let mut p = transparent_strip();
    p.dyn_eq = DynEqParams {
        freq_hz: 300.0,
        threshold_db: -30.0,
        max_cut_db: 6.0,
        q: DynEqParams::Q,
        ratio: DynEqParams::RATIO,
    };

    // 300 Hz、幅度 0.5（−6 dBFS 的带通电平）⇒ 远超 −30 dB 阈值 ⇒ 吃满 6 dB。
    let inside = sine(300.0, 0.5, 1.0);
    let (_, out_in) = run(p, &inside);
    let cut = out_in - rms_db(&inside[4800..]);
    assert!(
        (-7.5..=-4.0).contains(&cut),
        "带内应被压 4–7.5 dB（上限 6），实测 {cut:+.1} dB"
    );

    // 1 kHz、幅度 0.05：带通在 300 Hz 中心 Q0.9，1 kHz 处已掉到阈值以下 ⇒ 不压。
    let outside = sine(1_000.0, 0.05, 1.0);
    let (_, out_out) = run(p, &outside);
    let delta = out_out - rms_db(&outside[4800..]);
    assert!(
        delta.abs() < 0.5,
        "动态 EQ 不该动带外信号，实测 {delta:+.2} dB"
    );
}

#[test]
fn a_dynamic_eq_with_zero_depth_is_a_no_op() {
    let input = sine(250.0, 0.5, 0.5);
    let mut a = ChannelStrip::new(
        ChannelStripParams {
            dyn_eq: DynEqParams {
                freq_hz: 300.0,
                threshold_db: -60.0,
                max_cut_db: 0.0,
                q: DynEqParams::Q,
                ratio: DynEqParams::RATIO,
            },
            ..transparent_strip()
        },
        SR,
    );
    let mut b = ChannelStrip::new(transparent_strip(), SR);
    let mut x = input.clone();
    let mut y = input.clone();
    a.process_mono(&mut x);
    b.process_mono(&mut y);
    assert_eq!(x, y, "max_cut_db = 0 必须逐位旁通，哪怕阈值低到 −60 dB");
}

#[test]
fn a_fully_bypassed_strip_is_the_identity() {
    let input = sine(440.0, 0.31, 0.25);
    let mut strip = ChannelStrip::new(transparent_strip(), SR);
    let mut buf = input.clone();
    strip.process_mono(&mut buf);
    assert_eq!(buf, input, "全部旁通时必须逐位等于输入");
}

#[test]
fn the_new_stages_keep_the_mono_path_equal_to_a_duplicated_stereo_pair() {
    let mut p = transparent_strip();
    p.hpf = HpfParams { freq_hz: 300.0 };
    p.dyn_eq = DynEqParams {
        freq_hz: 280.0,
        threshold_db: -30.0,
        max_cut_db: 5.0,
        q: DynEqParams::Q,
        ratio: DynEqParams::RATIO,
    };

    // 一段同时有低频与中频的混合信号：低切与动态 EQ 都会动它。
    let n = (SR * 0.5) as usize;
    let mixed: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            0.4 * (core::f32::consts::TAU * 90.0 * t).sin()
                + 0.3 * (core::f32::consts::TAU * 280.0 * t).sin()
        })
        .collect();

    let mut mono_strip = ChannelStrip::new(p, SR);
    let mut mono = mixed.clone();
    mono_strip.process_mono(&mut mono);

    let mut stereo_strip = ChannelStrip::new(p, SR);
    let mut left = mixed.clone();
    let mut right = mixed.clone();
    stereo_strip.process_stereo(&mut left, &mut right);

    for (i, (m, l)) in mono.iter().zip(left.iter()).enumerate() {
        assert_eq!(
            m.to_bits(),
            l.to_bits(),
            "第 {i} 帧：单声道路径与复制成双声道的立体声路径必须逐位相同"
        );
    }
}

/// 左右不同的信号：M/S 分解后 side 不为零，宽度才有意义。
fn stereo_pair() -> (Vec<f32>, Vec<f32>) {
    let n = (SR * 0.3) as usize;
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR;
        l.push(0.40 * (core::f32::consts::TAU * 220.0 * t).sin());
        r.push(0.25 * (core::f32::consts::TAU * 330.0 * t).sin());
    }
    (l, r)
}

fn side_db(left: &[f32], right: &[f32]) -> f64 {
    let sum: f64 = left
        .iter()
        .zip(right.iter())
        .map(|(l, r)| {
            let s = 0.5 * (f64::from(*l) - f64::from(*r));
            s * s
        })
        .sum();
    10.0 * (sum / left.len().max(1) as f64 + 1e-30).log10()
}

#[test]
fn width_zero_folds_the_pair_to_mono() {
    let (left, right) = stereo_pair();
    let mut strip = ChannelStrip::new(
        ChannelStripParams {
            width: 0.0,
            ..transparent_strip()
        },
        SR,
    );
    let (mut l, mut r) = (left, right);
    strip.process_stereo(&mut l, &mut r);
    for (i, (a, b)) in l.iter().zip(r.iter()).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "第 {i} 帧：宽度 0 必须左右逐位相等"
        );
    }
}

#[test]
fn width_one_is_bit_identical_to_the_untouched_pair() {
    let (left, right) = stereo_pair();
    let mut plain = ChannelStrip::new(transparent_strip(), SR);
    let (mut l1, mut r1) = (left.clone(), right.clone());
    plain.process_stereo(&mut l1, &mut r1);

    let mut wide = ChannelStrip::new(
        ChannelStripParams {
            width: 1.0,
            ..transparent_strip()
        },
        SR,
    );
    let (mut l2, mut r2) = (left, right);
    wide.process_stereo(&mut l2, &mut r2);
    assert_eq!(l1, l2, "width = 1 必须逐位不变（左）");
    assert_eq!(r1, r2, "width = 1 必须逐位不变（右）");
}

#[test]
fn width_scales_only_the_side_signal() {
    let (left, right) = stereo_pair();
    let mut plain = ChannelStrip::new(transparent_strip(), SR);
    let (mut l1, mut r1) = (left.clone(), right.clone());
    plain.process_stereo(&mut l1, &mut r1);
    let side_before = side_db(&l1, &r1);

    let mut wide = ChannelStrip::new(
        ChannelStripParams {
            width: 1.5,
            ..transparent_strip()
        },
        SR,
    );
    let (mut l2, mut r2) = (left, right);
    wide.process_stereo(&mut l2, &mut r2);
    let side_after = side_db(&l2, &r2);

    let gain = side_after - side_before;
    // `1.5` = +3.52 dB；允许块边界与 f32 的残差。
    assert!(
        (3.3..=3.7).contains(&gain),
        "宽度 1.5 应只把侧信号抬 3.52 dB，实测 {gain:+.2} dB"
    );

    // 中信号（L+R）必须一位不动。
    let mid_before: f64 = l1
        .iter()
        .zip(r1.iter())
        .map(|(a, b)| f64::from(0.5 * (a + b)).powi(2))
        .sum();
    let mid_after: f64 = l2
        .iter()
        .zip(r2.iter())
        .map(|(a, b)| f64::from(0.5 * (a + b)).powi(2))
        .sum();
    let delta = 10.0 * (mid_after / mid_before).log10();
    assert!(
        delta.abs() < 0.01,
        "中信号不该被宽度改动，实测 {delta:+.3} dB"
    );
}

// ---------------------------------------------------------------------------
// `#346`：动态 EQ 的 Q 与比率是**可调字段**（本仓新增）
// ---------------------------------------------------------------------------

/// 同一频率、同一阈值与深度下，**窄 Q 的凹槽只碰自己那一段**。
///
/// 造法与 `the_dynamic_eq_cuts_its_own_band_and_leaves_the_rest_alone` 同源：
/// 4.8 kHz 的检波器被 4.8 kHz 的正弦喂到远超阈值（`−6 dBFS` 带通电平 vs `−30` 阈值），
/// 然后**在 2.4 kHz 上测稳态增益**——两个八度之外，`Q 0.9` 的峰值滤波器仍有可闻的
/// 拖尾，`Q 4.0` 几乎没有。
///
/// 判据是**差值**而不是绝对值：`2.4 kHz` 处的绝对衰减取决于 RBJ 峰值的过渡带形状，
/// 但"窄的比宽的少压"是单调的、与采样率无关的性质。
#[test]
fn a_narrow_dynamic_eq_q_leaves_the_neighbours_alone() {
    let band = sine(4800.0, 0.5, 1.0);
    let neighbour = sine(2400.0, 0.5, 1.0);

    let cut_at = |q: f32| -> f64 {
        let mut p = transparent_strip();
        p.dyn_eq = DynEqParams {
            freq_hz: 4800.0,
            threshold_db: -30.0,
            max_cut_db: 6.0,
            q,
            ratio: 4.0,
        };
        // 先让检波器吃满带内能量（同一台器件上跑一遍），再测邻居。
        let (_, band_out) = run(p, band.as_slice());
        let band_cut = rms_db(&band) - band_out;
        let (_, nb_out) = run(p, neighbour.as_slice());
        let nb_cut = rms_db(&neighbour) - nb_out;
        // 带内被压、邻居少压 —— 两条一起返回，便于失败时看清楚。
        println!("Q={q}: band_cut={band_cut:+.2} dB  neighbour_cut={nb_cut:+.2} dB");
        nb_cut
    };

    let wide = cut_at(DynEqParams::Q);
    let narrow = cut_at(4.0);
    assert!(
        narrow < wide - 0.5,
        "窄 Q（4.0）在两个八度外必须比缺省 Q（0.9）少压 ≥0.5 dB：窄 {narrow:+.2} / 宽 {wide:+.2}"
    );
}

/// `ratio = 1.0` 是**不压**：超过阈值也只有 0 dB 衰减量。
#[test]
fn a_dynamic_eq_ratio_of_one_never_cuts() {
    let mut p = transparent_strip();
    p.dyn_eq = DynEqParams {
        freq_hz: 4800.0,
        threshold_db: -40.0,
        max_cut_db: 6.0,
        q: 2.0,
        ratio: 1.0,
    };
    let input = sine(4800.0, 0.5, 1.0);
    let (_, out) = run(p, input.as_slice());
    let cut = rms_db(&input) - out;
    assert!(cut.abs() < 0.05, "ratio=1 必须一位不动，实测 {cut:+.3} dB");
}

/// `q` / `ratio` 的钳制：`0` 与 `NaN` 落在缺省，超出上界落上界。
#[test]
fn the_new_dyn_eq_fields_are_sanitised() {
    let p = DynEqParams {
        freq_hz: 4800.0,
        threshold_db: -30.0,
        max_cut_db: 4.0,
        q: 0.0,
        ratio: f32::NAN,
    }
    .sanitised();
    // 有限值走**钳制**（`0` → 下界 `0.3`），非有限值才走**缺省**
    // （与 `clamp_or` 的全仓同一口径：`clamp` 对 `NaN` 是恒等，必须单独兜住）。
    assert_eq!(p.q, DynEqParams::MIN_Q, "q=0 必须钳到下界");
    assert_eq!(p.ratio, DynEqParams::RATIO, "NaN 比率必须归到缺省比率");
    let p = DynEqParams {
        q: f32::NAN,
        ..DynEqParams::DEFAULT
    }
    .sanitised();
    assert_eq!(p.q, DynEqParams::Q, "NaN Q 必须归到缺省 Q");

    let p = DynEqParams {
        q: 1.0e9,
        ratio: 1.0e9,
        ..DynEqParams::DEFAULT
    }
    .sanitised();
    assert_eq!(p.q, DynEqParams::MAX_Q);
    assert_eq!(p.ratio, DynEqParams::MAX_RATIO);
}

// ---------------------------------------------------------------------------
// `#346`：M/S 中频（`ms_mid_gain_db` / `ms_mid_freq_hz`）—— 评审 §2 的
// "Mid Channel 提升 2–4 kHz"
// ---------------------------------------------------------------------------

/// 把一段**立体声**跑过通道条，返回两侧 RMS（dBFS）。
fn run_stereo(params: ChannelStripParams, l: &[f32], r: &[f32]) -> (f64, f64, f64, f64) {
    let mut strip = ChannelStrip::new(params, SR);
    let mut a = l.to_vec();
    let mut b = r.to_vec();
    strip.process_stereo(&mut a, &mut b);
    let skip = (SR * 0.1) as usize;
    (
        rms_db(&l[skip..]),
        rms_db(&a[skip..]),
        rms_db(&r[skip..]),
        rms_db(&b[skip..]),
    )
}

/// mid 被抬、side 一位不动 —— 这是"M/S 中频"的全部意义。
///
/// 造法：`L = R = 3 kHz 正弦` ⇒ 全是 mid；`L = −R` ⇒ 全是 side。
/// 同一条通道条（3 kHz 处 +6 dB）在两种输入上的增益差必须分别是 ≈ +6 dB 与 ≈ 0 dB。
#[test]
fn the_ms_mid_bell_moves_the_mid_and_leaves_the_side_alone() {
    let mut p = transparent_strip();
    p.ms_mid_gain_db = 6.0;
    p.ms_mid_freq_hz = 3000.0;

    let mono = sine(3000.0, 0.25, 1.0);
    let (in_db, out_db, _, _) = run_stereo(p, &mono, &mono);
    let mid_gain = out_db - in_db;
    assert!(
        (4.5..=7.5).contains(&mid_gain),
        "纯 mid 在 3 kHz 应被抬 ≈6 dB（固定 Q 0.9 的峰值），实测 {mid_gain:+.2} dB"
    );

    let inverted: Vec<f32> = mono.iter().map(|x| -x).collect();
    let (in_db, _, _, out_db) = run_stereo(p, &mono, &inverted);
    let side_gain = out_db - in_db;
    assert!(
        side_gain.abs() < 0.05,
        "纯 side（L = −R）必须一位不动，实测 {side_gain:+.3} dB"
    );
}

/// `ms_mid_gain_db == 0` 时**逐位透明**（连 M/S 往返都不做）。
#[test]
fn a_zero_ms_mid_gain_is_bit_identical() {
    let l = sine(3000.0, 0.3, 0.25);
    let r = sine(1370.0, 0.2, 0.25);

    let mut a = ChannelStrip::new(transparent_strip(), SR);
    let (mut x, mut y) = (l.clone(), r.clone());
    a.process_stereo(&mut x, &mut y);

    let mut p = transparent_strip();
    p.ms_mid_gain_db = 0.0;
    p.ms_mid_freq_hz = 3000.0;
    let mut b = ChannelStrip::new(p, SR);
    let (mut x2, mut y2) = (l, r);
    b.process_stereo(&mut x2, &mut y2);
    assert_eq!(x, x2);
    assert_eq!(y, y2);
}
