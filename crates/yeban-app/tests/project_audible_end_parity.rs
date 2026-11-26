//! T-FIX-467：工程可闻末端**单一口径**判据。
//!
//! `host.rs::audible_end_seconds`（GUI 播种）已委托
//! `export_wav::project_audible_end_seconds`（CLI 共用助手）。本文件在
//! 测试侧保留一份**独立参考实现**（旧的三跳公式，含自己的上限常量），
//! 对空工程 / 变速工程 / 超 3600 秒工程逐值对账：
//! - 等价判据：参考实现 == 共用助手（三者逐一相等）；
//! - 红对照：把参考上限人为改成 60 秒后必须与助手不等 —— 证明任何一侧
//!   的上限漂移都会让等价判据变红，而不是碰巧同色。

use yeban_model::YebanProjectV1;

/// 独立参考常量（⛔ 不与生产代码共享，否则红对照失去意义）。
const LEGACY_CAP_SECONDS: f64 = 3600.0;

/// 旧副本逐字公式的独立副本（视图投影取 `ViewState::from_project`；
/// `project_with_view_state` 与它只在缩放/光标/行高/选中态上不同，
/// `content_end_tick` 只取剪辑与段落末端，读数同源）。
fn legacy_end_seconds_with_cap(project: &YebanProjectV1, cap: f64) -> Option<f64> {
    let end_tick = yeban_app::bridge::ViewState::from_project(project)
        .ok()?
        .content_end_tick();
    if end_tick == 0 {
        return None;
    }
    let hz = project.sample_rate().hz();
    let frames = project.tempo().frames_at_tick(end_tick, yeban_model::PPQ, hz)?;
    #[allow(clippy::cast_precision_loss)]
    let seconds = frames as f64 / f64::from(hz);
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some((seconds.min(cap) * 1000.0).ceil() / 1000.0)
}

fn legacy_end_seconds(project: &YebanProjectV1) -> Option<f64> {
    legacy_end_seconds_with_cap(project, LEGACY_CAP_SECONDS)
}

fn shared_end_seconds(project: &YebanProjectV1) -> Option<f64> {
    yeban_app::export_wav::project_audible_end_seconds(project)
}

/// ① 空工程（只有母线轨，末端 0）⇒ 两侧同为 `None`。
fn empty_project() -> YebanProjectV1 {
    yeban_model::samples::default_project()
}

/// ② 变速工程（分段 tempo 换算，只取整一次）。
fn tempo_changed_project() -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    project.tempo_map.insert(3840, 60.0);
    project
}

/// ③ 超一小时工程（末端远超 3600 秒上限）。
fn over_cap_project() -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    let last = project
        .sections
        .values_mut()
        .next_back()
        .expect("夹具必须有段落");
    last.end_tick = 10_000_000;
    project
}

#[test]
fn gui_legacy_formula_and_shared_helper_agree_on_all_fixture_classes() {
    for (name, project) in [
        ("empty", empty_project()),
        ("tempo-changed", tempo_changed_project()),
        ("over-cap", over_cap_project()),
    ] {
        assert_eq!(
            legacy_end_seconds(&project),
            shared_end_seconds(&project),
            "{name} 工程：旧 GUI 播种公式与共用助手必须逐值相等"
        );
    }
}

#[test]
fn red_control_a_drifting_cap_is_caught() {
    let project = over_cap_project();
    let shared = shared_end_seconds(&project).expect("超长工程必须有读数");
    let drifted = legacy_end_seconds_with_cap(&project, 60.0).expect("参考实现必须有读数");
    assert_ne!(
        drifted, shared,
        "红对照：任一侧上限漂到 60 秒时，等价判据必须变红（实际都为 {drifted}）"
    );
    // 超长工程必须真的被 3600 秒上限夹住，否则上面 red control 是假红。
    assert!(
        (shared - LEGACY_CAP_SECONDS).abs() < 0.001,
        "超长工程应播 3600.000，实际 {shared}"
    );
}
