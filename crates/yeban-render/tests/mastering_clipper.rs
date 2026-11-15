//! C-1 母带判据：ExportPreset 的 clipper 段并入「施加后复测」闭环。
//!
//! 夹具（本文件内构造，口径写明）：48 kHz、2 s 立体声；
//! 瞬态重的合成信号 = 每 0.25 s 一个 5 ms 的 0.95 幅度脉冲串（鼓点式瞬态）
//! ＋ 0.25 幅度的 220 Hz 持续音垫。crest 高 ⇒ clipper 有削的空间。

use yeban_dsp::clipper::ClipperParams;
use yeban_render::mastering::ExportPreset;

const RATE: u32 = 48_000;

fn fixture() -> (Vec<f32>, Vec<f32>) {
    let frames = (RATE * 2) as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..frames {
        let pad = 0.25 * (std::f64::consts::TAU * 220.0 * i as f64 / f64::from(RATE)).sin();
        let phase = i % 12_000;
        let transient = if phase < 240 {
            0.95 * (1.0 - phase as f64 / 240.0)
        } else {
            0.0
        };
        let v = (pad + transient) as f32;
        left[i] = v;
        right[i] = v;
    }
    (left, right)
}

fn crest_db(reading: yeban_render::mastering::MasterLoudness) -> f32 {
    reading.true_peak_dbtp - reading.integrated_lufs
}

/// 带 clipper 的预设：复测应达目标附近，且 crest 削减量为正。
#[test]
fn clipper_inside_the_preset_reduces_crest_and_the_servo_still_lands() {
    let (mut l1, mut r1) = fixture();
    let plain = ExportPreset::new(Some(-14.0), Some(-1.0))
        .apply(RATE, &mut l1, &mut r1)
        .expect("48 kHz 可测");
    assert_eq!(plain.crest_reduction_db, 0.0, "无 clipper ⇒ crest 削减恒 0");

    let (mut l2, mut r2) = fixture();
    let clipped = ExportPreset::new(Some(-14.0), Some(-1.0))
        .with_clipper(Some(ClipperParams::DEFAULT))
        .apply(RATE, &mut l2, &mut r2)
        .expect("48 kHz 可测");
    assert!(
        clipped.crest_reduction_db > 0.3,
        "clipper 应削掉 crest，实测 {} dB",
        clipped.crest_reduction_db
    );
    assert!(
        crest_db(clipped.before) > crest_db(plain.before) - 0.01
            && (clipped.after.integrated_lufs + 14.0).abs() < 0.5,
        "复测须落在目标容差内：实测 {} LUFS",
        clipped.after.integrated_lufs
    );
    assert!(
        clipped.after.true_peak_dbtp <= -1.0 + 0.07,
        "真峰值须被上限兜住：{} dBTP",
        clipped.after.true_peak_dbtp
    );
    assert_eq!(clipped.target_shortfall_db, None, "达标 ⇒ 无响亮差距回执");
}

/// 达不到目标时响亮回执：目标 0 LUFS + ceiling −1 dBTP 物理上够不到 ⇒
/// `target_shortfall_db` 必须给出差距数值、`limits_reached` 必须为真。
#[test]
fn an_unreachable_target_reports_its_shortfall_loudly() {
    let (mut l, mut r) = fixture();
    let outcome = ExportPreset::new(Some(0.0), Some(-1.0))
        .with_clipper(Some(ClipperParams {
            drive_db: 18.0,
            ..ClipperParams::DEFAULT
        }))
        .apply(RATE, &mut l, &mut r)
        .expect("48 kHz 可测");
    let shortfall = outcome
        .target_shortfall_db
        .expect("达不到目标必须响亮报差距，⛔ 不得静默归一化");
    assert!(
        (shortfall - (0.0 - outcome.after.integrated_lufs)).abs() < 1e-4,
        "差距数值必须等于目标 − 复测：报 {shortfall}，复测 {} LUFS",
        outcome.after.integrated_lufs
    );
    assert!(shortfall > 0.5, "差距应在容差之外：{shortfall} dB");
    assert!(
        outcome.limits_reached,
        "已挂 clipper 且仍达不到 ⇒ 已达削波与限幅上限"
    );
}
