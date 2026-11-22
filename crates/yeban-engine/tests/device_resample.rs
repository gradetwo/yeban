//! ⭐ **T-FIX-72 行为判据**：流式采样率转换（[`RateConverter`]）的数值面。
//!
//! 与任务正文的验收条目一一对应：
//!
//! 1. **频率保持**：工程率（48000 Hz）的 1000 Hz 正弦被设备率（44100 Hz）
//!    消费后，输出过零率对应的频率仍是 1000 Hz（±1%）⇒ 速度与音高正确；
//!    升采样方向（44100 → 48000）同判。
//! 2. **长跑不漂移**：10 秒后输出帧数与精确有理数模型逐帧对账（±1 帧），
//!    末段过零点位置与理想网格的偏差 < 0.5 输出帧；**块边界连续**：
//!    全程相邻样本跳变受正弦斜率界约束（出现阶跃即红）。
//! 3. **等率直通**：等率构造不出转换器 ⇒ 调用方走逐位直通路径（现行为）。
//!
//! 另加两条结构钉：分块大小不影响输出（逐位一致）；立体声逐通道独立。

use yeban_engine::resample::RateConverter;

/// 生成 `frames` 帧单声道正弦（f64 生成后转 f32，振幅 0.8）。
fn sine(freq_hz: f64, rate: u32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            let phase = 2.0 * core::f64::consts::PI * freq_hz * i as f64 / f64::from(rate);
            (0.8 * libm::sin(phase)) as f32
        })
        .collect()
}

/// 把 `input` 全部喂给转换器（按 `block_frames` 分块），收齐全部输出。
/// 末尾多喂的帧已包含在 `input` 里（调用方负责把冲刷帧算进信号）。
fn convert_all(
    converter: &mut RateConverter,
    input: &[f32],
    channels: usize,
    block_frames: usize,
) -> Vec<f32> {
    let mut out = Vec::new();
    let mut scratch = vec![0.0f32; 8192 * channels];
    for chunk in input.chunks(block_frames * channels) {
        let mut filled = 0usize;
        converter.push_block(chunk, &mut scratch, &mut filled);
        out.extend_from_slice(&scratch[..filled * channels]);
    }
    out
}

/// 数窗口 `[from, to)` 内的上行过零次数。
fn upward_crossings(samples: &[f32], from: usize, to: usize) -> usize {
    samples[from..to]
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count()
}

/// 判据 1（降采样）：48000 的 1000 Hz 正弦 → 44100 消费，频率 ±1%。
#[test]
fn sine_frequency_preserved_when_downsampling_48k_to_44k1() {
    let input = sine(1000.0, 48_000, 144_064);
    let mut converter = RateConverter::new(48_000, 44_100, 1).expect("异率可构造");
    let out = convert_all(&mut converter, &input, 1, 128);
    // 取 0.1 s..1.1 s 的一整秒输出数过零：频率（Hz）= 过零次数 / 1 s。
    let crossings = upward_crossings(&out, 4_410, 4_410 + 44_100);
    let estimated = crossings as f64;
    assert!(
        (estimated - 1000.0).abs() <= 10.0,
        "降采样后频率应为 1000 Hz ±1%，实测 {estimated} Hz（过零 {crossings} 次）"
    );
}

/// 判据 1（升采样）：44100 的 1000 Hz 正弦 → 48000 消费，频率 ±1%。
#[test]
fn sine_frequency_preserved_when_upsampling_44k1_to_48k() {
    let input = sine(1000.0, 44_100, 88_264);
    let mut converter = RateConverter::new(44_100, 48_000, 1).expect("异率可构造");
    let out = convert_all(&mut converter, &input, 1, 128);
    // 取 0.1 s..1.1 s 的一整秒输出（48000 帧）数过零。
    let crossings = upward_crossings(&out, 4_800, 4_800 + 48_000);
    let estimated = crossings as f64;
    assert!(
        (estimated - 1000.0).abs() <= 10.0,
        "升采样后频率应为 1000 Hz ±1%，实测 {estimated} Hz（过零 {crossings} 次）"
    );
}

/// 判据 2：10 秒长跑 —— 输出帧数与有理数模型对账、末段相位不漂、块边界连续。
#[test]
fn long_run_has_no_drift_and_no_block_boundary_steps() {
    // 480000 帧工程信号（10 s）+ 64 帧冲刷（把窗尾的输出排出来）。
    let input = sine(1000.0, 48_000, 480_064);
    let mut converter = RateConverter::new(48_000, 44_100, 1).expect("异率可构造");
    let out = convert_all(&mut converter, &input, 1, 128);

    // 帧数对账：已发出的输出数 = #{k ≥ 0 : floor(k·160/147) ≤ received − 17}
    // （就绪条件：位置的整数部分 + 半窗 ≤ 最后一帧）。
    let received = converter.received();
    let predicted = ((received - 16) * 147 - 1) / 160 + 1;
    let emitted = converter.emitted();
    assert_eq!(out.len() as u64, emitted, "收齐的输出必须等于转换器记账");
    assert!(
        emitted.abs_diff(predicted) <= 1,
        "10 秒后输出帧数应与有理数模型一致（±1）：实测 {emitted}，模型 {predicted}"
    );

    // 相位不漂：取末段（最后 4410 帧内）的最后一个上行过零点，线性插值出
    // 分数位置；它与理想过零网格（每 44.1 帧一个）的偏差必须 < 0.5 帧。
    let tail_start = out.len() - 4_410;
    let mut crossing: Option<f64> = None;
    for i in tail_start..out.len() - 1 {
        if out[i] < 0.0 && out[i + 1] >= 0.0 {
            let frac = f64::from(-out[i]) / f64::from(out[i + 1] - out[i]);
            crossing = Some(i as f64 + frac);
        }
    }
    let crossing = crossing.expect("末段必须有上行过零点");
    let nearest_grid = (crossing / 44.1).round() * 44.1;
    assert!(
        (crossing - nearest_grid).abs() < 0.5,
        "10 秒后过零点位置 {crossing} 与理想网格 {nearest_grid} 的偏差必须 < 0.5 帧（漂移）"
    );

    // 块边界连续：0.8 振幅 1000 Hz 正弦在 44100 Hz 下的最大样本间斜率
    // = 0.8·2π·1000/44100 ≈ 0.114；留 1.4 倍余量 ⇒ 0.16。出现阶跃
    // （丢帧/重复帧/窗错位）会远超这条界。
    let max_step = out
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_step <= 0.16,
        "全程最大相邻跳变 {max_step} 超过斜率界 0.16（块边界不连续）"
    );
}

/// 判据 3：等率 ⇒ 没有转换器（调用方逐位直通）；非法参数同样没有。
#[test]
fn equal_rates_have_no_converter() {
    assert!(RateConverter::new(48_000, 48_000, 2).is_none());
    assert!(RateConverter::new(44_100, 44_100, 1).is_none());
    assert!(RateConverter::new(0, 44_100, 2).is_none());
    assert!(RateConverter::new(48_000, 44_100, 0).is_none());
}

/// 结构钉：分块大小（128 帧量子 vs 逐帧 vs 大块）不影响输出，逐位一致。
#[test]
fn block_size_does_not_change_the_output() {
    let input = sine(440.0, 48_000, 48_064);
    let mut by_quantum = RateConverter::new(48_000, 44_100, 1).expect("异率");
    let a = convert_all(&mut by_quantum, &input, 1, 128);
    let mut by_frame = RateConverter::new(48_000, 44_100, 1).expect("异率");
    let b = convert_all(&mut by_frame, &input, 1, 1);
    let mut by_big = RateConverter::new(48_000, 44_100, 1).expect("异率");
    let c = convert_all(&mut by_big, &input, 1, 4096);
    assert_eq!(a, b, "逐帧推进与量子推进必须逐位一致");
    assert_eq!(a, c, "大块推进与量子推进必须逐位一致");
}

/// 结构钉：立体声逐通道独立 —— 左声道正弦、右声道恒定 0.25，互不串扰。
#[test]
fn stereo_channels_are_converted_independently() {
    let frames = 48_064;
    let mono = sine(1000.0, 48_000, frames);
    let mut input = Vec::with_capacity(frames * 2);
    for sample in &mono {
        input.push(*sample);
        input.push(0.25);
    }
    let mut converter = RateConverter::new(48_000, 44_100, 2).expect("异率");
    let out = convert_all(&mut converter, &input, 2, 128);
    // 稳态段（第 1 秒之后）：右声道必须仍是 0.25（直流逐位通过）。
    let steady = &out[44_100 * 2..];
    for pair in steady.as_chunks::<2>().0 {
        assert!(
            (pair[1] - 0.25).abs() < 1e-6,
            "右声道直流必须逐位通过，实测 {}",
            pair[1]
        );
    }
    // 左声道过零频率仍是 1000 Hz ±1%（在交错流上按步长 2 取左声道；
    // 窗口取 40000 帧 ⇒ 频率 = 过零次数 × 44100 / 40000）。
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let crossings = upward_crossings(&left, 4_000, 4_000 + 40_000);
    let estimated = crossings as f64 * 44_100.0 / 40_000.0;
    assert!(
        (estimated - 1000.0).abs() <= 10.0,
        "左声道频率应为 1000 Hz ±1%，实测 {estimated} Hz"
    );
}
