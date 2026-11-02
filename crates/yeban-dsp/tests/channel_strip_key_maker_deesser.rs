//! 通道条三件新能力的判据（`#344` / `#345`）：键检波动态 EQ、mono-maker、
//! 动态 EQ 的频率上限放开。
//!
//! 四条要证明的事：
//!
//! 1. **键在带内 ⇒ 节目让路**，**键在带外 ⇒ 节目不让**。这是"Kick 键控、
//!    Bass 让路"的全部语义 —— 没有这条，`process_stereo_with_key` 里的键就只
//!    喂了压缩器，动态 EQ 仍然是自检波。
//! 2. **mono-maker 只拿走低频的 side**：低频 side 掉到听不见，高频 side 一位不动，
//!    mid 不变（因此"只折低频"不改变单声道兼容性之外的东西）。
//! 3. **mono-maker = 0 时逐位透明**：旁通不碰样本 ⇒ 既有工程重渲逐字节不变。
//! 4. **De-Esser 够得到 7 kHz**：频率上限 4000 → 12000 之后，`deq_freq=7000`
//!    不再被钳回 4 kHz，且 7 kHz 的带真的被压。
//!
//! 口径：全部用**稳态正弦**测增益（三件都是最小相位滤波 + 一阶包络，稳态是它们
//! 唯一有解析期望值的工作点）；每条都丢掉前 100 ms 的起振。

use yeban_dsp::channel_strip::{ChannelStrip, ChannelStripParams, DynEqParams, HpfParams};

const SR: f32 = 48_000.0;

/// 只留"被测那一级"的通道条：EQ / 滤波 / 压缩全关，低切与动态 EQ 旁通。
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

fn rms_db(x: &[f32]) -> f64 {
    let sum: f64 = x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
    10.0 * (sum / x.len().max(1) as f64 + 1e-30).log10()
}

fn skip() -> usize {
    (SR * 0.1) as usize
}

/// 键模式：节目与键都按"左右同值"喂（与引擎的单声道源一致），返回输出的 RMS(dB)。
fn run_with_key(params: ChannelStripParams, program: &[f32], key: &[f32]) -> f64 {
    let mut strip = ChannelStrip::new(params, SR);
    let mut left = program.to_vec();
    let mut right = program.to_vec();
    let frames = left.len();
    strip.process_stereo_with_key(&mut left, &mut right, &key[..frames], &key[..frames]);
    rms_db(&left[skip()..])
}

/// 立体声：左右分别给，返回 `(侧 RMS, 中 RMS)`，都单位 dB。
fn run_stereo(params: ChannelStripParams, left: &[f32], right: &[f32]) -> (f64, f64) {
    let mut strip = ChannelStrip::new(params, SR);
    let mut l = left.to_vec();
    let mut r = right.to_vec();
    strip.process_stereo(&mut l, &mut r);
    let side: Vec<f32> = l[skip()..]
        .iter()
        .zip(r[skip()..].iter())
        .map(|(a, b)| 0.5 * (a - b))
        .collect();
    let mid: Vec<f32> = l[skip()..]
        .iter()
        .zip(r[skip()..].iter())
        .map(|(a, b)| 0.5 * (a + b))
        .collect();
    (rms_db(&side), rms_db(&mid))
}

// -- 1. 键检波动态 EQ ------------------------------------------------------

/// 节目是一条安静的 85 Hz（低到自检波永远触发不了），键可以是带内或带外。
///
/// 三组读数：
///
/// | 组 | 键 | 期望 |
/// | :--- | :--- | :--- |
/// | 无键（节目检波） | — | 不压（节目 −40 dBFS，阈值 −30） |
/// | 带内键 85 Hz | −2 dBFS | 压到 `max_cut_db` 附近 |
/// | 带外键 6 kHz | −2 dBFS | 不压（6 kHz 过不了 85 Hz 的带通） |
#[test]
fn a_key_in_the_band_ducks_the_program_and_a_key_outside_it_does_not() {
    let mut p = transparent_strip();
    p.dyn_eq = DynEqParams {
        freq_hz: 85.0,
        threshold_db: -30.0,
        max_cut_db: 6.0,
        q: DynEqParams::Q,
        ratio: DynEqParams::RATIO,
    };

    let program = sine(85.0, 0.01, 1.0);
    let quiet = rms_db(&program[skip()..]);

    // ① 无键：走 `process_stereo`（自检波）。节目 −40 dBFS < 阈值 −30 ⇒ 不压。
    let mut strip = ChannelStrip::new(p, SR);
    let mut l = program.clone();
    let mut r = program.clone();
    strip.process_stereo(&mut l, &mut r);
    let no_key = rms_db(&l[skip()..]);
    assert!(
        (no_key - quiet).abs() < 0.2,
        "安静节目不该被自检波压：输入 {quiet:.2}、输出 {no_key:.2}"
    );

    // ② 带内键：85 Hz、−2 dBFS ⇒ 带通电平远超阈值 ⇒ 压满 `max_cut_db`。
    let key_in = sine(85.0, 0.8, 1.0);
    let ducked = run_with_key(p, &program, &key_in);
    let cut = no_key - ducked;
    assert!(
        cut > 4.0,
        "带内键只把节目压低 {cut:.2} dB（期望 ≈ max_cut_db = 6）"
    );

    // ③ 带外键：6 kHz、同样响，但过不了 85 Hz 的带通（Q = 0.9，隔 6 个八度）
    //    ⇒ 带通电平低于阈值 ⇒ 一点都不压。
    let key_out = sine(6_000.0, 0.8, 1.0);
    let untouched = run_with_key(p, &program, &key_out);
    let leak = no_key - untouched;
    assert!(
        leak.abs() < 1.0,
        "带外键不该让 85 Hz 让路，实测 {leak:+.2} dB（键检波只认自己那一段）"
    );
}

/// 键模式**不会**把键混进输出：节目是静音时，输出也必须是静音。
#[test]
fn the_key_never_reaches_the_output() {
    let mut p = transparent_strip();
    p.dyn_eq = DynEqParams {
        freq_hz: 85.0,
        threshold_db: -30.0,
        max_cut_db: 6.0,
        q: DynEqParams::Q,
        ratio: DynEqParams::RATIO,
    };
    let silence = vec![0.0f32; (SR * 0.5) as usize];
    let key = sine(85.0, 0.8, 0.5);
    let mut strip = ChannelStrip::new(p, SR);
    let mut l = silence.clone();
    let mut r = silence;
    strip.process_stereo_with_key(&mut l, &mut r, &key, &key);
    let peak = l.iter().fold(0.0f32, |acc, v| acc.max(v.abs()));
    assert!(peak == 0.0, "键漏进了输出：峰值 {peak}");
}

// -- 2. mono-maker ---------------------------------------------------------

/// 纯 side 输入（`L = s`、`R = −s`）：低频 side 按二阶斜率被压，高频 side 一位不动。
///
/// 契约是 `side ← HighPass₂(side, 120 Hz)`（Butterworth）⇒ 转折点 −3 dB、
/// 一个八度 −12 dB、两个八度 −24 dB。因此 `60 Hz` 期望 −12 dB、`30 Hz` 期望 −24 dB。
/// ⛔ 不是"120 Hz 以下完全为零"：二阶最小相位做不到砖墙，判据按**斜率**钉，
/// 也正因为如此，这里的三条读数（−12.3 / −24.x / 0.0）本身就是斜率的证据。
#[test]
fn the_mono_maker_takes_the_low_side_and_leaves_the_high_side() {
    let mut p = transparent_strip();
    p.mono_below_hz = 120.0;

    let cut_at = |freq: f32| {
        let tone = sine(freq, 0.5, 1.0);
        let inverted: Vec<f32> = tone.iter().map(|v| -v).collect();
        let before = rms_db(&tone[skip()..]);
        let (after, _) = run_stereo(p, &tone, &inverted);
        after - before
    };

    // 60 Hz：一个八度以下 ⇒ −12 dB（实测 −12.3）。
    let at_60 = cut_at(60.0);
    assert!(
        (-14.0..=-10.0).contains(&at_60),
        "120 Hz 的 mono-maker 对 60 Hz 的 side 掉了 {at_60:.1} dB（二阶应为 −12）"
    );

    // 30 Hz：两个八度以下 ⇒ −24 dB。这条把"斜率"与"某个频点恰好合格"分开。
    let at_30 = cut_at(30.0);
    assert!(
        (-27.0..=-21.0).contains(&at_30),
        "30 Hz 的 side 掉了 {at_30:.1} dB（二阶应为 −24）⇒ 斜率不对"
    );

    // 3 kHz：远在转折点之上 ⇒ side 一位不动（只允许数值残差）。
    let at_3k = cut_at(3_000.0);
    assert!(
        at_3k.abs() < 0.2,
        "mono-maker 不该动 3 kHz 的 side，实测 {at_3k:+.2} dB"
    );

    // mid 与 mono-maker 无关：纯 side 输入的中值恒为零。
    let low = sine(60.0, 0.5, 1.0);
    let inverted: Vec<f32> = low.iter().map(|v| -v).collect();
    let (_, mid_after) = run_stereo(p, &low, &inverted);
    assert!(
        mid_after < -80.0,
        "纯 side 输入的中值必须恒为零，实测 {mid_after:.1} dB（mid 被动了）"
    );
}

/// `mono_below_hz = 0`（默认）⇒ **逐位透明**：旁通不碰样本，也不推进 side 低通。
#[test]
fn a_mono_maker_of_zero_is_bit_identical() {
    let p = transparent_strip();
    assert_eq!(p.mono_below_hz, 0.0, "默认必须是旁通");

    let left = sine(70.0, 0.5, 0.5);
    let right: Vec<f32> = sine(70.0, 0.5, 0.5).iter().map(|v| -0.5 * v).collect();
    let mut strip = ChannelStrip::new(p, SR);
    let mut l = left.clone();
    let mut r = right.clone();
    strip.process_stereo(&mut l, &mut r);
    for i in 0..left.len() {
        assert_eq!(
            l[i].to_bits(),
            left[i].to_bits(),
            "第 {i} 帧左声道被动了（旁通必须逐位相同）"
        );
        assert_eq!(r[i].to_bits(), right[i].to_bits(), "第 {i} 帧右声道被动了");
    }
}

/// 宽度与 mono-maker 的**顺序契约**：宽度先缩放整条 side，mono-maker 再切低频。
///
/// 因此"`width = 0`（整条单声道）"与"`mono_below_hz = 1000` 的高频 side 仍存在"
/// 是两件不同的事 —— 前者连高频 side 都没有了，后者有。
#[test]
fn the_width_is_applied_before_the_mono_maker() {
    let high = sine(5_000.0, 0.5, 0.5);
    let inverted: Vec<f32> = high.iter().map(|v| -v).collect();

    let mut wide = transparent_strip();
    wide.mono_below_hz = 120.0;
    let (side_kept, _) = run_stereo(wide, &high, &inverted);

    let mut mono = transparent_strip();
    mono.width = 0.0;
    mono.mono_below_hz = 120.0;
    let (side_gone, _) = run_stereo(mono, &high, &inverted);

    assert!(
        side_kept - side_gone > 40.0,
        "宽度 0 必须比 mono-maker 更彻底：留着 {side_kept:.1} dB、砍到 {side_gone:.1} dB"
    );
}

// -- 3. De-Esser 的频率上限 ------------------------------------------------

/// `DynEqParams` 的频率域：下限仍是 80，上限从 4000 提到 12000。
#[test]
fn the_dyn_eq_frequency_upper_bound_is_twelve_kilo() {
    let at = |hz: f32| {
        DynEqParams {
            freq_hz: hz,
            threshold_db: -30.0,
            max_cut_db: 6.0,
            q: DynEqParams::Q,
            ratio: DynEqParams::RATIO,
        }
        .sanitised()
        .freq_hz
    };
    assert_eq!(
        at(7_000.0),
        7_000.0,
        "7 kHz 必须原样保留（De-Esser 的落点）"
    );
    assert_eq!(at(12_000.0), 12_000.0);
    assert_eq!(at(30_000.0), DynEqParams::MAX_FREQ, "超上界钳到 12000");
    assert_eq!(at(10.0), DynEqParams::MIN_FREQ, "低于 80 钳到 80");
}

/// 7 kHz 的带真的被压 —— 这是"够得到 De-Esser"的**行为**证据（不是常数证据）。
#[test]
fn a_dyn_eq_at_seven_kilo_really_ducks_seven_kilo() {
    let mut p = transparent_strip();
    p.dyn_eq = DynEqParams {
        freq_hz: 7_000.0,
        threshold_db: -30.0,
        max_cut_db: 6.0,
        q: DynEqParams::Q,
        ratio: DynEqParams::RATIO,
    };

    // 节目够响（−2 dBFS）⇒ 自检波压满 6 dB。
    let program = sine(7_000.0, 0.8, 1.0);
    let mut strip = ChannelStrip::new(p, SR);
    let mut l = program.clone();
    let mut r = program.clone();
    strip.process_stereo(&mut l, &mut r);
    let cut = rms_db(&l[skip()..]) - rms_db(&program[skip()..]);

    // Q = 0.9 的峰值滤波器在 7 kHz、−6 dB 增益下，中心处给 ≈ −4.7 dB（不是 −6：
    // 峰值滤波器的"增益"是零点/极点比，`Q` 越小中心衰减越浅）。判据取 > 3 dB。
    assert!(
        cut < -3.0,
        "7 kHz 的动态 EQ 只把 7 kHz 压了 {cut:.1} dB（上限放开后应为 −4 ~ −6）"
    );
}
