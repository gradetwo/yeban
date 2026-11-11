//! 自动相位对齐的渲染侧判据：两轨 solo 渲染 + 互相关测量。
//!
//! 量的是**渲染出的样本**，不是参数：演示工程的鼓轨（PolySynth，6 个音符）
//! 原样复制到主音轨上、只加 `offset_samples` 平移，测出的 lag 必须等于
//! 平移量；零平移必须测出 0；静音轨与非源节点必须响亮拒绝。

use yeban_dsp::phase_align_auto::Confidence;
use yeban_model::samples::demo_project;
use yeban_model::{ClipPlacement, EntityId, YebanProjectV1};
use yeban_render::engine_source::ArmedSfzInput;
use yeban_render::phase_align_auto::{PhaseAlignAutoError, measure_stem_delay};
use yeban_render::song_overview::{RenderSettings, SongOverviewError};

/// 演示工程没有采样器绑定 ⇒ provider 绝不能被调用（被调用即 panic）。
fn never_called_provider() -> impl FnMut(EntityId, &str) -> ArmedSfzInput {
    |_, _| panic!("演示工程没有采样器绑定，provider 不应被调用")
}

/// 按轨名找轨 id（演示工程的轨名唯一）。
fn track_id_by_name(project: &YebanProjectV1, name: &str) -> EntityId {
    project
        .tracks
        .values()
        .find(|track| track.name == name)
        .unwrap_or_else(|| panic!("演示工程没有 {name} 轨"))
        .id
}

/// 把 `source` 轨的全部摆放复制到 `target` 轨上（同一 clip、同时值），只加
/// `offset_samples` 平移；同时把 `source` 的设备链复制过去（同一音色，
/// 否则两轨波形不同、相关系数上不去）。
fn copy_placements_with_offset(
    project: &mut YebanProjectV1,
    source: EntityId,
    target: EntityId,
    offset_samples: Option<i32>,
) {
    let (placements, devices) = {
        let source_track = &project.tracks[&source];
        (
            source_track.clips.values().cloned().collect::<Vec<_>>(),
            source_track.devices.clone(),
        )
    };
    let target_track = project
        .tracks
        .get_mut(&target)
        .expect("目标轨必须在工程里");
    target_track.clips.clear();
    for placement in placements {
        let id = EntityId::new();
        target_track.clips.insert(
            id,
            ClipPlacement {
                id,
                offset_samples,
                ..placement
            },
        );
    }
    target_track.devices = devices;
}

#[test]
fn shifted_copy_measures_the_exact_offset() {
    let mut project = demo_project();
    let drums = track_id_by_name(&project, "鼓");
    let lead = track_id_by_name(&project, "主音");
    copy_placements_with_offset(&mut project, drums, lead, Some(2_400));

    let measured = measure_stem_delay(
        &project,
        &RenderSettings::default(),
        never_called_provider(),
        drums,
        lead,
        2_400,
    )
    .expect("两轨都有声，必须可测");
    // 目标轨晚到 2400 采样（offset_samples 是精确采样平移）。
    assert_eq!(measured.measurement.lag_samples_rounded, 2_400);
    assert!((measured.measurement.lag_samples - 2_400.0).abs() < 1.0);
    assert!(measured.measurement.correlation > 0.9);
    assert!(!measured.measurement.polarity_inverted);
    assert_eq!(measured.measurement.confidence, Confidence::High);
    assert_eq!(measured.sample_rate_hz, 48_000);
}

#[test]
fn identical_copies_measure_zero_lag() {
    let mut project = demo_project();
    let drums = track_id_by_name(&project, "鼓");
    let lead = track_id_by_name(&project, "主音");
    copy_placements_with_offset(&mut project, drums, lead, None);

    let measured = measure_stem_delay(
        &project,
        &RenderSettings::default(),
        never_called_provider(),
        drums,
        lead,
        2_400,
    )
    .expect("两轨都有声，必须可测");
    assert_eq!(measured.measurement.lag_samples_rounded, 0);
    assert!(measured.measurement.correlation > 0.99);
    assert!(!measured.measurement.polarity_inverted);
}

#[test]
fn silent_track_is_refused() {
    // 弦乐轨没有任何摆放 ⇒ solo 渲染全零 ⇒ 测量拒绝，不返回假 lag。
    let project = demo_project();
    let drums = track_id_by_name(&project, "鼓");
    let strings = track_id_by_name(&project, "弦乐");
    let error = measure_stem_delay(
        &project,
        &RenderSettings::default(),
        never_called_provider(),
        drums,
        strings,
        2_400,
    )
    .expect_err("静音轨必须拒绝");
    assert_eq!(
        error,
        PhaseAlignAutoError::Measure(yeban_dsp::phase_align_auto::MeasureDelayError::SilentTarget)
    );
}

#[test]
fn master_bus_is_not_a_source_track() {
    let project = demo_project();
    let drums = track_id_by_name(&project, "鼓");
    let master = project.master_bus_track_id;
    let error = measure_stem_delay(
        &project,
        &RenderSettings::default(),
        never_called_provider(),
        drums,
        master,
        2_400,
    )
    .expect_err("主总线不是源节点，必须拒绝");
    assert_eq!(
        error,
        PhaseAlignAutoError::Render(SongOverviewError::NotASourceTrack { track: master })
    );
}
