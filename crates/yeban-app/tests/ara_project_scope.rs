//! ⭐ T-FIX-349：ARA 面板状态随工程切换作废。
//!
//! 本文件只跑一个活窗口 #[test]（Slint 平台每线程一次）：
//! 1. 接线 `wire_plugin_panel` 后把面板读数写成"已绑定"的样子；
//! 2. 调 `reset_plugin_panel_for_project_switch`（`open_archive_runtime` 在换会话
//!    成功后调用的同一个函数）⇒ 行、选中、状态、查询/渲染结果、进度全部复位；
//! 3. 源码级钉住接线的唯一落点：`open_archive_runtime` 必须调用该函数，且 reset
//!    必须清 registry / 丢在飞渲染接收端（代际校验在 drain 里）。
//!
//! ⚠ 真实 ARA 绑定需要本机 ARA 插件，CI 没有；本判据覆盖"工程边界清空"这一层，
//! 插件侧会话对象由 `plugin_panel::AraRegistry` 的 clear 统一丢弃。

#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::Model as _;
use yeban_app::host;
use yeban_ui_test_port::port::Permission;

#[test]
fn switching_projects_wipes_the_gui_ara_panel_state() {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 端口");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let _ui = ui;
    host::wire_plugin_panel(&window, None);

    // 夹具：把面板读成"工程 A 绑过一个会话"的样子。
    let rows = vec![yeban_app::ui::AraRowData {
        id: "ara-old".into(),
        factory: "OldFactory".into(),
        state: "已绑定".into(),
        detail: "old.wav".into(),
    }];
    window.set_ara_rows(slint::ModelRc::new(slint::VecModel::from(rows)));
    window.set_ara_selected_row(0);
    window.set_ara_status("已绑定 ara-old".into());
    window.set_ara_query_result("old query".into());
    window.set_ara_result("old result".into());
    window.set_ara_progress(0.5);
    assert_eq!(window.get_ara_rows().iter().count(), 1);

    // 工程 B：open_archive_runtime 成功换会话后调用的同一个出口。
    host::reset_plugin_panel_for_project_switch(&window);

    assert_eq!(window.get_ara_rows().iter().count(), 0, "面板必须为空");
    assert_eq!(window.get_ara_selected_row(), -1, "选中行必须清空");
    assert_eq!(window.get_ara_status(), "", "绑定状态必须清空");
    assert_eq!(
        window.get_ara_query_result(),
        "",
        "旧查询结果不得留在新工程"
    );
    assert_eq!(window.get_ara_result(), "", "旧渲染结果不得留在新工程");
    assert_eq!(window.get_ara_progress(), 0.0, "进度必须复位");

    // 源码级：唯一清会话落点在 open_archive_runtime，且 reset 清 registry + 丢接收端。
    let host_rs = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    let at = host_rs
        .find("pub fn open_archive_runtime(")
        .expect("open_archive_runtime 必须在");
    let body = &host_rs[at..];
    let end = body.find("\n    pub fn ").expect("下一个方法");
    assert!(
        body[..end].contains("reset_plugin_panel_for_project_switch(ui)"),
        "换工程必须清 ARA 面板（T-FIX-349）"
    );
    let reset_at = host_rs
        .find("pub fn reset_plugin_panel_for_project_switch(")
        .expect("reset 函数必须在");
    let reset_body = &host_rs[reset_at..];
    let reset_end = reset_body.find("\n}\n").expect("reset 函数末尾");
    for needle in [
        ".ara.clear()",
        ".ara_rows.clear()",
        ".selected_ara = None",
        ".render_rx = None",
    ] {
        assert!(
            reset_body[..reset_end].contains(needle),
            "reset 必须包含 {needle}"
        );
    }
}
