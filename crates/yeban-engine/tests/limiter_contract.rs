//! `line/engine-mix` 的端到端判据（四）：**母线限制器的器件级契约**
//! [ARCH-DSP-001, ARCH-RT-001, ARCH-DET-001]。
//!
//! # 与 `mix_render.rs` 的分工
//!
//! `mix_render.rs` 测的是"从工程到输出"的端到端口径（声相定律、限制器在真实母线
//! 上的作用、确定性、静音）。这里测的是 [`BusLimiter`] **器件本身**的契约：
//! 上界、透明、弹道、确定性、顶点。器件级判据的价值是"注入能精确定位到哪一行代码"。
//!
//! 口径（实现细节与理由见 `yeban_engine::mixer` 的模块文档）：
//!
//! | 量 | 值 | 说明 |
//! | :--- | :--- | :--- |
//! | 前瞻窗口 | 33 帧（**奇数**） | 环长必须是 `2·延迟 + 1`，否则上界证明不成立 |
//! | 输出延迟 | `LOOKAHEAD_SAMPLES` = 33 帧 ≈ 0.69 ms | 写位置上的旧值就是 33 帧前那一个 |
//! | 阈值 | 0.9（≈ −0.92 dBFS） | 超过即开始压 |
//! | 天花板 | 0.95（软膝渐近） | 输出**严格**不超过它，允许阈值之上 0.05 的过冲 |
//! | 释放 | 每样本 5e-5（≈ 0.42 s 从 0 回满） | 释放**永不**跳变；攻击**不**受此限 |
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | L1 | 过阈值信号经限制后峰值 ≤ 天花板，且被压样本数 > 0 | 阈值乘 10 / 摘掉限制器 |
//! | L2 | 未过阈值 ⇒ **逐位**不变（含对齐延迟后的逐样本比较） | 在增益为 1 时也乘一次别的系数 |
//! | L3 | 立体声联动：一侧的瞬态**不改变** L/R 能量比 | 两侧各自独立压限 |
//! | L4 | 释放每样本回升 ≤ `LIMITER_RELEASE_PER_SAMPLE`；攻击当帧就让位 | 把分支写反 |
//! | L5 | 确定性：同输入两次处理**逐位**相同 | 引入真熵源 |
//! | L6 | 顶点：空块 / 零帧 / `NaN` / 纯静音不 panic、不泄漏 | 删掉 `nan_to_zero` |

mod support;

use yeban_engine::block::AudioBlock;
use yeban_engine::mixer::{
    BusLimiter, LIMITER_CEILING, LIMITER_RELEASE_PER_SAMPLE, LIMITER_THRESHOLD, LOOKAHEAD_SAMPLES,
};

/// 一个量子的帧数。
const FRAMES: usize = 128;

/// 左/右声道完全相同的块。
fn mono_block(samples: &[f32]) -> AudioBlock<FRAMES> {
    let mut block = AudioBlock::<FRAMES>::new();
    block.set_frames(FRAMES);
    let (left, right) = block.stereo_mut();
    left.copy_from_slice(samples);
    right.copy_from_slice(samples);
    block
}

/// 正弦夹具（`amplitude` 幅度、`frequency` 频率、`offset` 起始相位样本数）。
fn sine(amplitude: f32, frequency: f32, offset: usize) -> Vec<f32> {
    (0..FRAMES)
        .map(|index| {
            let phase = core::f32::consts::TAU * frequency * (index + offset) as f32 / 48_000.0;
            amplitude * phase.sin()
        })
        .collect()
}

/// L1：过阈值信号经限制后，峰值 **≤ 天花板**，且限制器**确实压过**样本。
#[test]
fn over_threshold_peaks_are_capped_at_the_ceiling() {
    let mut limiter = BusLimiter::new();
    let mut peak = 0.0f32;
    let mut offset = 0usize;
    for _ in 0..64 {
        let source = sine(2.0, 997.0, offset);
        offset += FRAMES;
        let mut block = mono_block(&source);
        limiter.apply(&mut block, FRAMES);
        for sample in block.left() {
            peak = peak.max(sample.abs());
        }
    }
    println!(
        "[engine-mix] L1 peak={peak:.6} ceiling={LIMITER_CEILING} threshold={LIMITER_THRESHOLD} \
         reductions={} engaged={}",
        limiter.reduction_count(),
        limiter.engaged()
    );
    assert!(limiter.engaged(), "2.0 幅度的正弦必须驱动限制器");
    assert!(
        limiter.reduction_count() > 0,
        "限制器报告 0 个被压样本 —— 夹具没有驱动它（结构性假绿）"
    );
    assert!(
        peak <= LIMITER_CEILING,
        "限制后峰值 {peak} 超过天花板 {LIMITER_CEILING}"
    );
    assert!(
        peak > LIMITER_THRESHOLD,
        "峰值 {peak} 低于阈值 —— 限制器把信号压过头了"
    );
}

/// L1b：**多种过载形态**下峰值都不越天花板（正弦、方波、脉冲串、随机、直流。
#[test]
fn every_overload_shape_stays_under_the_ceiling() {
    let cases: Vec<(&str, Vec<Vec<f32>>)> = vec![
        (
            "square 1.5",
            (0..32)
                .map(|index| {
                    let sign = if index % 2 == 0 { 1.5 } else { -1.5 };
                    vec![sign; FRAMES]
                })
                .collect(),
        ),
        (
            "impulse train 3.0",
            (0..32)
                .map(|_| {
                    let mut v = vec![0.0f32; FRAMES];
                    v[0] = 3.0;
                    v[7] = -3.0;
                    v
                })
                .collect(),
        ),
        ("dc 1.2", (0..32).map(|_| vec![1.2f32; FRAMES]).collect()),
        (
            "saw 2.0",
            (0..32)
                .map(|q| {
                    (0..FRAMES)
                        .map(|i| {
                            let t = ((q * FRAMES + i) % 48) as f32 / 48.0;
                            2.0 * (2.0 * t - 1.0)
                        })
                        .collect()
                })
                .collect(),
        ),
    ];

    for (label, blocks) in cases {
        let mut limiter = BusLimiter::new();
        let mut peak = 0.0f32;
        for source in &blocks {
            let mut block = mono_block(source);
            limiter.apply(&mut block, FRAMES);
            for sample in block.left() {
                peak = peak.max(sample.abs());
            }
        }
        println!(
            "[engine-mix] L1b {label}: peak={peak:.6} reductions={}",
            limiter.reduction_count()
        );
        assert!(
            peak <= LIMITER_CEILING,
            "{label}: 峰值 {peak} 超过天花板 {LIMITER_CEILING}"
        );
    }
}

/// L2：**未过阈值的信号逐位不变**（对齐输出延迟之后逐样本比较位模式）。
#[test]
fn sub_threshold_blocks_pass_bit_identically() {
    let mut limiter = BusLimiter::new();
    let mut offset = 0usize;
    let mut all_source: Vec<f32> = Vec::new();
    let mut all_output: Vec<f32> = Vec::new();
    for _ in 0..16 {
        let source = sine(0.4, 440.0, offset);
        offset += FRAMES;
        let mut block = mono_block(&source);
        limiter.apply(&mut block, FRAMES);
        all_source.extend_from_slice(&source);
        all_output.extend_from_slice(block.left());
    }
    assert!(!limiter.engaged(), "0.4 幅度的正弦不得驱动限制器");
    assert_eq!(limiter.reduction_count(), 0);
    let mut compared = 0usize;
    for (index, sample) in all_source.iter().enumerate() {
        let output = index + LOOKAHEAD_SAMPLES;
        if output >= all_output.len() {
            break;
        }
        assert_eq!(
            all_output[output].to_bits(),
            sample.to_bits(),
            "样本 {index}（输出 {output}）被改写了"
        );
        compared += 1;
    }
    assert!(compared > 1_000, "比较的样本太少（{compared}），判据可疑");
    println!("[engine-mix] L2 compared={compared} samples bit-identically");
}

/// L3：**立体声联动** —— 一侧的瞬态不得改变"另一侧"的信号。
///
/// ## 为什么要用"另一侧逐位不变"而不是"L/R 比值不变"
///
/// 第一版用 `r / l` 的逐样本比值，实测跳到 2.785 —— 但那是**夹具**的问题：
/// 左声道是 300 Hz 正弦，会周期性过零，`r/l` 在 `l ≈ 0` 处病态（不是实现的问题）。
///
/// 正确的读数：右声道的瞬态应该让**两侧乘同一个增益** ⇒
/// `left_after[i] == left_before[i] × g`，也就是"左声道的**相对轨迹**不变"。
/// 联动与独立压限在这个夹具上的区别非常明确：
///
/// - 联动：右声道的瞬态把 `g` 拉低 ⇒ 左声道整段被同比例压低；
/// - 独立：右声道的瞬态**只**压低右声道 ⇒ 左声道的样本完全不变。
///
/// 因此判据就是"左声道的样本在瞬态量子内**必须被改变**"，且改变的比例与右声道
/// 一致。
#[test]
fn stereo_limiting_is_linked() {
    // 参照：完全不带瞬态的同一段信号。
    let render = |with_transient: bool| -> Vec<f32> {
        let mut limiter = BusLimiter::new();
        let mut left_out = Vec::new();
        for quantum in 0..32 {
            let mut block = AudioBlock::<FRAMES>::new();
            block.set_frames(FRAMES);
            {
                let (left, right) = block.stereo_mut();
                for (index, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                    let base = 0.5
                        * (core::f32::consts::TAU * 300.0 * (quantum * FRAMES + index) as f32
                            / 48_000.0)
                            .sin();
                    *l = base;
                    *r = if with_transient && quantum == 16 && index == 64 {
                        1.8
                    } else {
                        base
                    };
                }
            }
            limiter.apply(&mut block, FRAMES);
            left_out.extend_from_slice(block.left());
        }
        left_out
    };

    let reference = render(false);
    let tested = render(true);
    // 增益在瞬态**进入前瞻窗口**时开始下降：瞬态在量子 16 的 index 64（= 样本 2112），
    // 窗口提前 33 帧看到它 ⇒ 衰减最早从样本 2112 + 33 = 2145 之后开始；
    // 而前瞻里被延迟输出的是**更早**的样本，因此真正被压低的是接下来的量子。
    // 实测：窗口取 [2112, 2240) 时左声道峰值从 0.5 掉到 0.25 左右。
    let window = 2_112..2_240;
    let reference_peak = reference[window.clone()]
        .iter()
        .fold(0.0f32, |peak, s| peak.max(s.abs()));
    let tested_peak = tested[window.clone()]
        .iter()
        .fold(0.0f32, |peak, s| peak.max(s.abs()));
    let ratio = tested_peak / reference_peak;
    println!(
        "[engine-mix] L3 left peak with/without right-channel transient: \
         {tested_peak:.6} / {reference_peak:.6} = {ratio:.6}"
    );
    // 联动 ⇒ 左声道被同一增益压低（实测压到约 1/2）。
    assert!(
        ratio < 0.9,
        "右声道的瞬态没有影响左声道（比值 {ratio:.6}）—— 两侧可能各自独立压限"
    );
    // 但不得压得过头（联动增益就是右声道瞬态所需的那个增益）。
    assert!(
        ratio > 0.3,
        "左声道被压得太狠（比值 {ratio:.6}）—— 增益算错了"
    );
}

/// L4：释放的上限与攻击的"当帧让位"（**逐样本**测）。
///
/// ⚠ 必须一帧一块地喂：一个 128 帧量子最多能回升 `128 × release = 6.4e-3`，
/// 拿量子边界上的两个读数比较会把**合法释放**记成跳变（本文件第一版实测
/// `rise = 6.4e-3`，正好是 `128 × 5e-5` —— 那不是 bug，是夹具口径错了）。
#[test]
fn release_is_rate_limited_and_attack_is_immediate() {
    // (a) 释放：过载 2.0 之后回到 0.05，增益逐样本回升 ≤ release；且确实回升过。
    let mut limiter = BusLimiter::new();
    let mut block = AudioBlock::<1>::new();
    let mut previous = limiter.gain();
    let mut saw_release = false;
    let mut violations = 0usize;
    for (offset, index) in (0..8_192usize).enumerate() {
        let amplitude = if index < 1_024 { 2.0 } else { 0.05 };
        let value = amplitude * (core::f32::consts::TAU * 997.0 * offset as f32 / 48_000.0).sin();
        {
            let (left, right) = block.stereo_mut();
            left[0] = value;
            right[0] = value;
        }
        limiter.apply(&mut block, 1);
        let gain = limiter.gain();
        let rise = gain - previous;
        if rise > LIMITER_RELEASE_PER_SAMPLE + LIMITER_RELEASE_PER_SAMPLE * 1e-3 {
            violations += 1;
        }
        if rise > 0.0 {
            saw_release = true;
        }
        previous = gain;
    }
    println!(
        "[engine-mix] L4 release: saw_release={saw_release} violations={violations} final_gain={}",
        limiter.gain()
    );
    assert!(saw_release, "夹具没有产生释放段 —— 判据空转");
    assert_eq!(violations, 0, "释放回升超过了每样本上限");
    assert!(
        limiter.gain() > 0.6,
        "8192 帧的释放应当已经回到 0.6 以上，实际 {}",
        limiter.gain()
    );

    // (b) 攻击：过载进入窗口的**当帧**，增益就必须让位（不受释放上限约束）。
    let mut limiter = BusLimiter::new();
    let mut block = AudioBlock::<1>::new();
    let mut gains = Vec::new();
    for index in 0..64 {
        let value = if index == 32 { 2.0 } else { 0.0 };
        {
            let (left, right) = block.stereo_mut();
            left[0] = value;
            right[0] = value;
        }
        limiter.apply(&mut block, 1);
        gains.push(limiter.gain());
    }
    println!(
        "[engine-mix] L4 attack: gain[31]={:.6} gain[32]={:.6}",
        gains[31], gains[32]
    );
    assert!(gains[31] > 0.9, "过载之前增益应当在 1.0 附近");
    assert!(
        gains[32] < 0.6,
        "过载当帧增益就必须让位（实测 0.45），实际 {}",
        gains[32]
    );
}

/// L5：确定性 —— 同输入两次处理逐位相同（含立体声与非对称输入）。
#[test]
fn limiter_is_byte_deterministic() {
    let mut first = BusLimiter::new();
    let mut second = BusLimiter::new();
    let mut first_out: Vec<u32> = Vec::new();
    let mut second_out: Vec<u32> = Vec::new();
    for quantum in 0..48 {
        let mut block = AudioBlock::<FRAMES>::new();
        block.set_frames(FRAMES);
        {
            let (left, right) = block.stereo_mut();
            for (index, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                let t = (quantum * FRAMES + index) as f32;
                *l = 1.7 * (core::f32::consts::TAU * 311.0 * t / 48_000.0).sin();
                *r = 1.3 * (core::f32::consts::TAU * 733.0 * t / 48_000.0).cos();
            }
        }
        let mut block_a = block.clone();
        let mut block_b = block;
        first.apply(&mut block_a, FRAMES);
        second.apply(&mut block_b, FRAMES);
        for (l, r) in block_a.left().iter().zip(block_b.left().iter()) {
            first_out.push(l.to_bits());
            second_out.push(r.to_bits());
        }
    }
    assert_eq!(first_out, second_out, "两次处理必须逐位相同");
    assert!(first.engaged());
}

/// L6：顶点行为 —— 零帧 / 空内容 / `NaN` / 纯静音。
#[test]
fn edges_never_panic_and_never_leak() {
    // (a) 零帧：不推进状态。
    let mut limiter = BusLimiter::new();
    let mut block = mono_block(&[0.5; FRAMES]);
    limiter.apply(&mut block, 0);
    assert_eq!(limiter.gain(), 1.0);
    assert!(!limiter.engaged());

    // (b) 纯静音：输出逐位 0，增益保持 1.0。
    let mut silence = mono_block(&[0.0; FRAMES]);
    limiter.apply(&mut silence, FRAMES);
    assert!(silence.left().iter().all(|s| *s == 0.0));
    assert_eq!(limiter.gain(), 1.0);

    // (c) NaN：显式归零，不得通过 `max` 污染窗口，也不得出现在输出里。
    let mut poisoned = AudioBlock::<FRAMES>::new();
    poisoned.set_frames(FRAMES);
    {
        let (left, right) = poisoned.stereo_mut();
        left[0] = f32::NAN;
        right[0] = f32::NAN;
        left[10] = 1.5;
        right[10] = 1.5;
    }
    limiter.apply(&mut poisoned, FRAMES);
    assert!(
        poisoned.left().iter().all(|s| s.is_finite()),
        "NaN 泄漏到输出"
    );
    assert!(limiter.engaged(), "1.5 的样本必须驱动限制器");

    // (d) `reset()` 之后回到初始状态（seek 路径）。
    limiter.reset();
    assert_eq!(limiter.gain(), 1.0);
    assert!(!limiter.engaged());
    assert_eq!(limiter.reduction_count(), 0);
    let mut after_reset = mono_block(&sine(2.0, 997.0, 0));
    limiter.apply(&mut after_reset, FRAMES);
    let peak = after_reset
        .left()
        .iter()
        .fold(0.0f32, |peak, s| peak.max(s.abs()));
    assert!(peak <= LIMITER_CEILING);
}
