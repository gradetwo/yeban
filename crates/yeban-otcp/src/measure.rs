//! 量测门（设计稿 §5.3）：量导出文件，不信上报的目标。
//!
//! 导出是 `hybridMode: true` 时，< ~75 音分的修正（实现定义值，
//! ⛔ 不进协议）走音高移位而不走声码器，上报的 `targetMidi` 可能与
//! 实际听到的不一致。驱动用自相关法测导出文件的 F0，对照每音符的
//! `targetMidi` 算偏差：
//! * 中位 |偏差| ≤ `measure_gate_cents`（缺省 50 音分）且
//! * 有声帧检出率 ≥ 80%
//!   才放行；否则 `otcpMeasureFailed`，音频⛔ 不交付（哪怕 OpenTune
//!   报成功）。
//!
//! 口径：
//! * 帧窗 2048 采样、步进 1024 采样；只量音符时间段内的帧（间隙不量）；
//! * 有声：RMS ≥ -60 dBFS 且归一化自相关峰 ≥ 0.3；
//! * F0 范围 55–660 Hz（低于 ~200 Hz 时量测弱，收据如实写检出率，
//!   不凭量测放行低频段——见设计稿 §6.3 的诚实边界）。

use serde_json::{Value, json};

use crate::error::OtcpError;
use crate::pipeline::Note;
use crate::wav::WavData;

/// 量测结果（进收据）。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Measurement {
    /// 中位 |偏差|（音分）。
    pub median_cents: f32,
    /// p90 |偏差|（音分）。
    pub p90_cents: f32,
    /// 有声帧检出率（0.0–1.0）。
    pub detection_rate: f32,
    /// 量过的有声帧数。
    pub voiced_frames: usize,
    /// 音符时间段内的总帧数。
    pub total_frames: usize,
    /// 量测门是否通过。
    pub passed: bool,
    /// 导出文件的 `hybridMode`（上报值，原样记录）。
    pub hybrid_mode: bool,
    /// 导出文件的实际后端（上报值，原样记录）。
    pub backend: String,
}

/// 量导出文件并判门。
///
/// `notes` 是校正后重读的音符表（`targetMidi` 为准）。
/// `gate_cents` 是中位 |偏差| 上限；`hybrid_mode` / `backend` 是
/// 导出结果上报的值（原样进收据）。
pub fn measure_export(
    wav: &WavData,
    notes: &[Note],
    gate_cents: f32,
    hybrid_mode: bool,
    backend: &str,
) -> Result<Measurement, OtcpError> {
    if wav.channels != 1 {
        return Err(OtcpError::with_data(
            "otcpMeasureFailed",
            format!("导出文件是 {} 通道，量测只认单声道", wav.channels),
            json!({"channels": wav.channels}),
        ));
    }
    let samples = &wav.samples;
    let sample_rate = wav.sample_rate;
    let window = 2048usize;
    let hop = 1024usize;
    let mut deviations: Vec<f32> = Vec::new();
    let mut total_frames = 0usize;
    let mut voiced_frames = 0usize;
    for note in notes {
        let start = (note.start_content_sec * f64::from(sample_rate)) as usize;
        let end = (note.end_content_sec * f64::from(sample_rate)) as usize;
        if end <= start || start >= samples.len() {
            continue;
        }
        let end = end.min(samples.len());
        let target_freq = midi_to_freq(note.target_midi);
        let mut frame_start = start;
        while frame_start + window <= end {
            total_frames += 1;
            if let Some(f0) = estimate_f0(&samples[frame_start..frame_start + window], sample_rate)
            {
                voiced_frames += 1;
                let cents = 1200.0 * (f0 / target_freq).log2();
                deviations.push(cents);
            }
            frame_start += hop;
        }
    }
    let detection_rate = if total_frames == 0 {
        0.0
    } else {
        voiced_frames as f32 / total_frames as f32
    };
    let (median_cents, p90_cents) = deviation_stats(&mut deviations);
    let passed = total_frames > 0 && median_cents <= gate_cents && detection_rate >= 0.8;
    let measurement = Measurement {
        median_cents,
        p90_cents,
        detection_rate,
        voiced_frames,
        total_frames,
        passed,
        hybrid_mode,
        backend: backend.to_owned(),
    };
    if passed {
        Ok(measurement)
    } else {
        Err(OtcpError::with_data(
            "otcpMeasureFailed",
            format!(
                "量测门未过：中位 |偏差| {median_cents:.1} 音分（上限 {gate_cents}），检出率 {detection_rate:.2}（下限 0.80）"
            ),
            serde_json::to_value(&measurement).unwrap_or(Value::Null),
        ))
    }
}

/// |偏差| 的中位与 p90（就地排序，取绝对值）。
fn deviation_stats(deviations: &mut [f32]) -> (f32, f32) {
    if deviations.is_empty() {
        return (f32::INFINITY, f32::INFINITY);
    }
    let mut abs: Vec<f32> = deviations.iter().map(|d| d.abs()).collect();
    abs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = percentile(&abs, 0.5);
    let p90 = percentile(&abs, 0.9);
    (median, p90)
}

fn percentile(sorted_abs: &[f32], quantile: f32) -> f32 {
    if sorted_abs.is_empty() {
        return f32::INFINITY;
    }
    let index = ((sorted_abs.len() as f32 - 1.0) * quantile).round() as usize;
    sorted_abs[index.min(sorted_abs.len() - 1)]
}

/// MIDI 音高 → 频率（Hz，A4 = 440）。
#[must_use]
pub fn midi_to_freq(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

/// 自相关法估 F0（单帧，单声道 f32）。
///
/// 有声判据：RMS ≥ -60 dBFS 且归一化自相关峰 ≥ 0.3。
/// 返回 `None` ⇒ 该帧无可靠 F0（无声或弱相关）。
#[must_use]
pub fn estimate_f0(frame: &[f32], sample_rate: u32) -> Option<f32> {
    if frame.len() < 512 || sample_rate == 0 {
        return None;
    }
    let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
    if rms < 0.001 {
        return None;
    }
    let min_lag = (f64::from(sample_rate) / 660.0).ceil() as usize;
    let max_lag = (f64::from(sample_rate) / 55.0).floor() as usize;
    let max_lag = max_lag.min(frame.len() / 2);
    if max_lag <= min_lag {
        return None;
    }
    // 归一化自相关：r[lag] / r[0]。
    let r0: f32 = frame.iter().map(|s| s * s).sum();
    if r0 <= 0.0 {
        return None;
    }
    let mut best_lag = min_lag;
    let mut best_value = f32::NEG_INFINITY;
    for lag in min_lag..=max_lag {
        let mut sum = 0.0f32;
        for i in 0..frame.len() - lag {
            sum += frame[i] * frame[i + lag];
        }
        let value = sum / r0;
        if value > best_value {
            best_value = value;
            best_lag = lag;
        }
    }
    if best_value < 0.3 {
        return None;
    }
    // 抛物线插值（亚采样精度）。
    let lag_f = {
        let r_prev = autocorr_at(frame, best_lag.saturating_sub(1), r0);
        let r_curr = best_value;
        let r_next = autocorr_at(frame, best_lag + 1, r0);
        let denom = r_prev - 2.0 * r_curr + r_next;
        if denom.abs() > 1e-9 {
            best_lag as f32 + 0.5 * (r_prev - r_next) / denom
        } else {
            best_lag as f32
        }
    };
    let f0 = f64::from(sample_rate) / f64::from(lag_f);
    if !(55.0..=660.0).contains(&f0) {
        return None;
    }
    Some(f0 as f32)
}

fn autocorr_at(frame: &[f32], lag: usize, r0: f32) -> f32 {
    if lag >= frame.len() || r0 <= 0.0 {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for i in 0..frame.len() - lag {
        sum += frame[i] * frame[i + lag];
    }
    sum / r0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Note;

    fn sine_note(freq: f32, sample_rate: u32, secs: f32) -> WavData {
        let count = (sample_rate as f32 * secs) as usize;
        let samples: Vec<f32> = (0..count)
            .map(|i| {
                (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin() * 0.5
            })
            .collect();
        WavData {
            channels: 1,
            sample_rate,
            samples,
        }
    }

    fn note_at(start: f64, end: f64, target_midi: f32) -> Note {
        Note {
            index: 0,
            start_content_sec: start,
            end_content_sec: end,
            detected_midi: target_midi,
            target_midi,
            offset_cents: 0.0,
            corrected: true,
        }
    }

    #[test]
    fn estimate_f0_recovers_sine() {
        let wav = sine_note(440.0, 44_100, 0.5);
        let f0 = estimate_f0(&wav.samples[0..2048], 44_100).expect("应检出");
        assert!((f0 - 440.0).abs() < 1.0, "f0={f0}");
    }

    #[test]
    fn estimate_f0_rejects_silence() {
        let frame = vec![0.0f32; 2048];
        assert!(estimate_f0(&frame, 44_100).is_none());
    }

    #[test]
    fn measure_passes_on_pitch_sine() {
        let wav = sine_note(440.0, 44_100, 2.0);
        let notes = vec![note_at(0.0, 2.0, 69.0)];
        let measurement = measure_export(&wav, &notes, 50.0, false, "cpu").expect("应过门");
        assert!(measurement.passed);
        assert!(measurement.median_cents < 5.0);
        assert!(measurement.detection_rate > 0.9);
    }

    #[test]
    fn measure_fails_when_off_by_48_cents_and_gate_is_5() {
        // 452 Hz 相对 A4（440 Hz）约 +47.9 音分。
        let wav = sine_note(452.0, 44_100, 2.0);
        let notes = vec![note_at(0.0, 2.0, 69.0)];
        let error = measure_export(&wav, &notes, 5.0, false, "cpu").expect_err("应不过门");
        assert_eq!(error.reason, "otcpMeasureFailed");
        let median = error.data["median_cents"].as_f64().expect("有中位值");
        assert!((median - 47.9).abs() < 3.0, "median={median}");
    }

    #[test]
    fn measure_fails_on_empty_notes() {
        let wav = sine_note(440.0, 44_100, 1.0);
        let error = measure_export(&wav, &[], 50.0, false, "cpu").expect_err("无音符应不过门");
        assert_eq!(error.reason, "otcpMeasureFailed");
    }
}
