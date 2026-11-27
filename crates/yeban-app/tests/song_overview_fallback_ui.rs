//! T-FIX-301 判据：刷新回执必须落到产品可见处。
//!
//! 回退本身是既有设计（渲染失败仍画纯谱面）；这里证的是：
//! ① 带 `error` 的回退 ⇒ 面板状态属性非空、点名「回退」与失败原因、并标注
//!    「能量/频率为谱面代理值」；
//! ② 工程较大档（`score_only = true`、无 error）同样要看得出是纯谱面；
//! ③ 全量成功 ⇒ 状态清空（不会把旧回退提示留在面板上）；
//! ④ T-FIX-815：空工程 ⇒ 中性空状态文案，不报渲染失败、不进回退态。

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::song_overview::{RefreshReport, apply_refresh_report};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

fn rig() -> LivePort<yeban_app::ui::MainWindow> {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("主窗口装配成功")
}

#[test]
fn a_render_failure_is_named_on_the_panel_and_the_sentinels_are_marked() {
    let live = rig();
    let ui = live.ui();
    apply_refresh_report(
        ui,
        &RefreshReport {
            bars: 3,
            cache_hit: false,
            score_only: true,
            empty_project: false,
            error: Some("渲染失败（missing sample），回退到纯谱面分析".to_owned()),
        },
    );
    assert!(ui.get_song_overview_score_only(), "回退态必须进界面");
    let status = ui.get_song_overview_analysis_status().to_string();
    assert!(status.contains("纯谱面回退"), "必须点名回退: {status}");
    assert!(status.contains("渲染失败"), "必须带失败原因: {status}");
    assert!(status.contains("谱面代理"), "哨兵值必须标注代理: {status}");
}

#[test]
fn a_large_project_is_marked_as_score_only_and_a_full_analysis_clears_it() {
    let live = rig();
    let ui = live.ui();
    apply_refresh_report(
        ui,
        &RefreshReport {
            bars: 16,
            cache_hit: false,
            score_only: true,
            empty_project: false,
            error: None,
        },
    );
    let status = ui.get_song_overview_analysis_status().to_string();
    assert!(status.contains("纯谱面分析"), "大工程也必须说清: {status}");
    assert!(status.contains("谱面代理"), "哨兵值必须标注代理: {status}");

    apply_refresh_report(
        ui,
        &RefreshReport {
            bars: 16,
            cache_hit: true,
            score_only: false,
            empty_project: false,
            error: None,
        },
    );
    assert!(!ui.get_song_overview_score_only(), "全量成功必须退出回退态");
    assert_eq!(
        ui.get_song_overview_analysis_status().to_string(),
        "",
        "全量成功必须清空旧提示"
    );
}

/// T-FIX-815：空工程在面板上是中性空状态 —— 文案逐字中性，不报渲染失败，
/// 不进回退态（金色回退色也不亮）。
#[test]
fn an_empty_project_is_a_neutral_empty_state_on_the_panel() {
    let live = rig();
    let ui = live.ui();
    apply_refresh_report(
        ui,
        &RefreshReport {
            bars: 0,
            cache_hit: false,
            score_only: false,
            error: None,
            empty_project: true,
        },
    );
    assert!(!ui.get_song_overview_score_only(), "空状态不是回退态");
    let status = ui.get_song_overview_analysis_status().to_string();
    assert_eq!(
        status, "工程为空：暂无可渲染内容",
        "面板必须写中性空状态文案: {status}"
    );
    assert!(
        !status.contains("渲染失败"),
        "空状态不得报渲染失败: {status}"
    );
    assert!(!status.contains("纯谱面回退"), "空状态不得说回退: {status}");
}
