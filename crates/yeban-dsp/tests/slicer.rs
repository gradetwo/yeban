//! 音频切片的判据：onset 切点、tape 变速变调、淡入淡出、确定性。
//!
//! ## 这条判据要证的句子
//!
//! > 已知起音位置的合成音频，切点落在起音处（一跳以内）；tape 变速后
//! > 音高与时长按比值走；每片首尾都按 4 ms 淡入、10 ms 淡出；同一输入
//! > 跑两次切点逐位相同。

use yeban_dsp::slicer::{
    DEFAULT_MIN_GAP_MS, FADE_IN_MS, FADE_OUT_MS, OnsetOptions, apply_fades, detect_onsets,
    tape_varispeed,
};

const SAMPLE_RATE: u32 = 48_000;

/// 一段正弦（`freq` Hz、`amplitude` 幅值、`frames` 帧，初相 0）。
fn sine(freq: f32, amplitude: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|index| {
            let phase = 2.0 * std::f32::consts::PI * freq * index as f32 / SAMPLE_RATE as f32;
            amplitude * phase.sin()
        })
        .collect()
}

/// 零交叉数测频率（Hz）：跳过首尾各 10%（淡入淡出与拉伸延迟不在测量区内）。
/// 纯正弦下该读数精确到一个零交叉以内。
fn measure_freq_hz(samples: &[f32]) -> f64 {
    let skip = samples.len() / 10;
    let window = &samples[skip..samples.len() - skip];
    let mut crossings = 0usize;
    for pair in window.windows(2) {
        if (pair[0] < 0.0) != (pair[1] < 0.0) {
            crossings += 1;
        }
    }
    let secs = window.len() as f64 / f64::from(SAMPLE_RATE);
    crossings as f64 / (2.0 * secs)
}

/// 判据（**切点正确**）：三段 440 Hz 突发（起音 0.20 s / 0.70 s / 1.20 s，
/// 段间静音），检出的三个切点各落在真起音 ±20 ms 内（≈ 两跳），且不多不少。
#[test]
fn onsets_land_on_known_burst_starts() {
    let total = (1.6 * SAMPLE_RATE as f32) as usize;
    let mut audio = vec![0.0f32; total];
    for (start_s, len_s) in [(0.20f32, 0.25f32), (0.70, 0.25), (1.20, 0.25)] {
        let start = (start_s * SAMPLE_RATE as f32) as usize;
        let len = (len_s * SAMPLE_RATE as f32) as usize;
        for (offset, sample) in sine(440.0, 0.8, len).iter().enumerate() {
            audio[start + offset] = *sample;
        }
    }
    let onsets = detect_onsets(&audio, SAMPLE_RATE, &OnsetOptions::default());
    assert_eq!(onsets.len(), 3, "三段突发 ⇒ 三个切点，不多不少: {onsets:?}");
    for (got, want_s) in onsets.iter().zip([0.20f32, 0.70, 1.20]) {
        let want = (want_s * SAMPLE_RATE as f32) as usize;
        let error = got.abs_diff(want);
        assert!(
            error <= 960,
            "切点 {got} 离真起音 {want} 差 {error} 帧（> 20 ms）"
        );
    }
}

/// 判据（**tape 变速变调**）：440 Hz 正弦 0.10 s，`rate = 2` ⇒ 880 Hz、
/// 时长一半；`rate = 0.5` ⇒ 220 Hz、时长两倍。
#[test]
fn tape_varispeed_moves_pitch_and_duration_together() {
    let input = sine(440.0, 0.8, 4800);
    let fast = tape_varispeed(&input, 2.0).expect("rate 2 合法");
    assert_eq!(fast.len(), 2400, "2× 速 ⇒ 时长一半");
    let fast_freq = measure_freq_hz(&fast);
    assert!(
        (fast_freq - 880.0).abs() < 15.0,
        "2× 速 ⇒ 880 Hz，实测 {fast_freq:.1} Hz"
    );
    let slow = tape_varispeed(&input, 0.5).expect("rate 0.5 合法");
    assert_eq!(slow.len(), 9600, "0.5× 速 ⇒ 时长两倍");
    let slow_freq = measure_freq_hz(&slow);
    assert!(
        (slow_freq - 220.0).abs() < 15.0,
        "0.5× 速 ⇒ 220 Hz，实测 {slow_freq:.1} Hz"
    );
}

/// 判据（**淡入淡出已施加**）：常数 1.0 的片段，首样本 0、淡入结束处 1、
/// 末样本 0、淡入淡出段内是线性斜坡（不是台阶、不是静默）。
#[test]
fn fades_ramp_in_over_4_ms_and_out_over_10_ms() {
    let mut samples = vec![1.0f32; SAMPLE_RATE as usize];
    apply_fades(&mut samples, SAMPLE_RATE);
    let fade_in = ((FADE_IN_MS / 1000.0) * SAMPLE_RATE as f32).round() as usize;
    let fade_out = ((FADE_OUT_MS / 1000.0) * SAMPLE_RATE as f32).round() as usize;
    assert_eq!(fade_in, 192, "4 ms @ 48 kHz = 192 帧");
    assert_eq!(fade_out, 480, "10 ms @ 48 kHz = 480 帧");
    assert_eq!(samples[0], 0.0, "首样本必须 0");
    assert_eq!(samples[fade_in], 1.0, "淡入结束处必须回到 1");
    assert_eq!(samples[SAMPLE_RATE as usize - 1], 0.0, "末样本必须 0");
    // 线性：淡入中点是 0.5（淡入从 0/192 起步，中点恰为 96/192）；
    // 淡出从 479/480 起步、末样本 0，中点是 239/480（离散步进所致，
    // 与 0.5 的差 < 一个步长 1/480）。
    let mid_in = samples[fade_in / 2];
    assert!(
        (mid_in - 0.5).abs() < 1e-6,
        "淡入中点应为 0.5，实测 {mid_in}"
    );
    let mid_out = samples[SAMPLE_RATE as usize - fade_out / 2];
    let expected_mid_out = (fade_out - 1 - fade_out / 2) as f32 / fade_out as f32;
    assert_eq!(
        mid_out, expected_mid_out,
        "淡出中点应为 {expected_mid_out}，实测 {mid_out}"
    );
    assert!(
        (mid_out - 0.5).abs() < 1.0 / fade_out as f32,
        "淡出中点与 0.5 的差应 < 一个步长，实测 {mid_out}"
    );
}

/// 判据（**确定性**）：同一输入跑两次，切点逐位相同。
#[test]
fn detecting_the_same_audio_twice_gives_the_same_cuts() {
    let mut audio = vec![0.0f32; SAMPLE_RATE as usize];
    for (offset, sample) in sine(440.0, 0.8, 12_000).iter().enumerate() {
        audio[9600 + offset] = *sample;
    }
    let first = detect_onsets(&audio, SAMPLE_RATE, &OnsetOptions::default());
    let second = detect_onsets(&audio, SAMPLE_RATE, &OnsetOptions::default());
    assert_eq!(first, second, "同一输入两次检测必须逐位相同");
    assert_eq!(first.len(), 1, "一段突发 ⇒ 一个切点");
}

/// 判据（**最小间隔**）：两段突发只隔 30 ms（小于缺省 80 ms）⇒ 只留
/// 通量大的那一个切点（弱峰被吞掉，不是两个）。
#[test]
fn onsets_closer_than_the_min_gap_collapse_to_one() {
    let total = (1.0 * SAMPLE_RATE as f32) as usize;
    let mut audio = vec![0.0f32; total];
    for (start_s, amplitude) in [(0.20f32, 0.8f32), (0.23, 0.4)] {
        let start = (start_s * SAMPLE_RATE as f32) as usize;
        for (offset, sample) in sine(440.0, amplitude, 9600).iter().enumerate() {
            audio[start + offset] = *sample;
        }
    }
    let onsets = detect_onsets(&audio, SAMPLE_RATE, &OnsetOptions::default());
    assert_eq!(
        onsets.len(),
        1,
        "30 ms 间隔 < 缺省 {DEFAULT_MIN_GAP_MS} ms ⇒ 只留一个切点: {onsets:?}"
    );
}
