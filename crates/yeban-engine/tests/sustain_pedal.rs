//! ⭐ ADR-0071 的判据：**延音踏板在复音合成器里真的发声**。
//!
//! 投影层（`snapshot.rs` 的 P1–P7）只钉住"终点样本被延后"；这一票钉住
//! "延后的终点真的还在出声"：
//!
//! - 踏板踩下时，音符松键（24000 样本）之后、踏板抬起（48000 样本）之前仍有
//!   持续声；对照组（无踏板泳道）同一窗口基本静音（50 ms 释放尾巴早已走完）；
//! - 踏板抬起之后 + 释放尾巴，声音回到静音；
//! - 半踏板（深度 32）只把声音延到 36000 样本附近，之后静音。
//!
//! 夹具栅格与 `support` 一致：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本。
//! 渲染走产品路径（`EngineSnapshot::from_project` → 实时驱动），2 秒 = 750 量子。

mod support;

use std::collections::BTreeMap;

use support::{NoteSpec, note_project, render, rms_peak};
use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, CurveType, EntityId,
    YebanProjectV1,
};

/// 给夹具工程的唯一 MIDI 轨挂一条 CC64 泳道（线性点）。
fn with_pedal(mut project: YebanProjectV1, track: EntityId, points: &[(u64, f32)]) -> YebanProjectV1 {
    let mut lane_points = BTreeMap::new();
    for (tick, value) in points {
        let id = EntityId::new();
        lane_points.insert(
            id,
            AutomationPoint {
                id,
                tick: *tick,
                value: *value,
                curve: CurveType::Linear,
            },
        );
    }
    let lane = AutomationLane {
        target: AutomationTarget::MidiCc {
            track_id: track,
            controller: 64,
        },
        points: lane_points,
        read_enabled: true,
        write_mode: AutomationWriteMode::Off,
        domain: None,
    };
    let entry = project.tracks.get_mut(&track).expect("夹具轨");
    entry.automation_lanes.insert(lane.target, lane);
    project.validate().expect("夹具必须合法");
    project
}

/// 2 秒渲染（750 量子 × 128 帧）的左声道。
fn render_two_seconds(project: &YebanProjectV1) -> Vec<f32> {
    render(project, 750).left
}

/// 判据 S1：全踏板 —— 松键后、抬起前仍有持续声；抬起后回到静音。
#[test]
fn pedal_holds_sound_past_note_off_until_it_lifts() {
    let fixture = note_project(&[NoteSpec::at(0, 960, 69, 100)]);
    let pedal = with_pedal(fixture.project, fixture.track, &[(0, 127.0), (1920, 0.0)]);
    let control = fixture_project_without_pedal();
    let held = render_two_seconds(&pedal);
    let dry = render_two_seconds(&control);
    assert_eq!(held.len(), 96_000, "2 s @48 kHz");
    assert_eq!(dry.len(), 96_000);

    // 窗口（样本）：音符 0..24000；踏板抬起 48000；释放 50 ms = 2400 样本。
    // 保持窗 30000..46000：对照组释放尾巴（24000 + 2400）早已走完；踏板组仍在持续。
    let (held_rms, held_peak) = rms_peak(&held[30_000..46_000]);
    let (dry_rms, _) = rms_peak(&dry[30_000..46_000]);
    // 抬起后窗 52000..70000：踏板组也已走完释放尾巴。
    let (tail_rms, _) = rms_peak(&held[52_000..70_000]);
    println!(
        "### [S1] 保持窗 RMS: 踏板 {held_rms:.6}（峰值 {held_peak:.6}）/ 对照 {dry_rms:.6}；\
         抬起后窗 RMS: {tail_rms:.6}"
    );
    assert!(
        held_rms > 0.005,
        "踏板保持窗必须真的有持续声，实得 RMS {held_rms:.6}"
    );
    assert!(
        held_rms > 20.0 * dry_rms,
        "踏板保持窗必须比对照组响得多（对照 {dry_rms:.6}、踏板 {held_rms:.6}）"
    );
    assert!(
        tail_rms < 0.2 * held_rms,
        "踏板抬起 + 释放尾巴之后必须回到接近静音（保持窗 {held_rms:.6}、抬起后 {tail_rms:.6}）"
    );
}

/// 判据 S2：半踏板 —— 声音只延到约 36000 样本（1440 tick），之后静音。
#[test]
fn half_pedal_holds_sound_only_part_way() {
    let fixture = note_project(&[NoteSpec::at(0, 960, 69, 100)]);
    let half = with_pedal(fixture.project, fixture.track, &[(0, 32.0), (1920, 0.0)]);
    let samples = render_two_seconds(&half);
    assert_eq!(samples.len(), 96_000);
    // 延后终点 36000：30000..35000 仍在持续；40000..46000 已过终点 + 释放尾巴。
    let (early_rms, _) = rms_peak(&samples[30_000..35_000]);
    let (late_rms, _) = rms_peak(&samples[40_000..46_000]);
    println!("### [S2] 半踏板：30000..35000 RMS {early_rms:.6}；40000..46000 RMS {late_rms:.6}");
    assert!(
        early_rms > 0.005,
        "半踏板延后终点前必须仍有声，实得 RMS {early_rms:.6}"
    );
    assert!(
        late_rms < 0.2 * early_rms,
        "半踏板延后终点 + 释放尾巴之后必须回到接近静音（前 {early_rms:.6}、后 {late_rms:.6}）"
    );
}

/// 与 `note_project` 同一夹具、**不带**踏板泳道的对照工程。
fn fixture_project_without_pedal() -> YebanProjectV1 {
    note_project(&[NoteSpec::at(0, 960, 69, 100)]).project
}
