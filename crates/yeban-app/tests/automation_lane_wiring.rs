//! **自动化泳道在活编曲视图里的可见性**（T-FIX-227 / 覆盖 ISS-MWTUMQ6G 家族的另一半）。
//!
//! 修前：宿主写了 9 个 `automation-lane-*` 数组与 `automation-visible`，但活视图
//! `ui/arrangement.slint` 里 `automation` 命中 0 ⇒ 泳道整块不存在、`Ctrl+Shift+A`
//! 只翻一个没人读的布尔，界面毫无变化。
//!
//! 判据（全部走**运行时树**，⛔ 不查源码字符串）：
//!
//! 1. 默认（`automation-visible = true`）⇒ 投影里的每条泳道都在树里；
//! 2. `Ctrl+Shift+A`（`Action::ToggleAutomation`）⇒ 泳道从树里消失，再按一次回来；
//! 3. 菜单命令 `toggle-automation`（`execute-action` 表）⇒ 与键**同一条落点**：
//!    `automation-visible` 回读 + 树同步变化。
//!
//! ⚠ 单 `#[test]`：Slint 平台每线程只能装一次。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
mod live;

use std::cell::RefCell;
use std::rc::Rc;

use live::{LiveControlPlane, LiveWiringOptions, build_live_ui_with};
use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_mcp::live::request_line;
use yeban_ui_test_port::port::Permission;

const NOW_MS: u64 = 1_760_000_000_000;

fn node_exists(plane: &mut LiveControlPlane, seq: i64, id: &str) -> bool {
    !plane
        .plane()
        .try_line(&request_line(
            seq,
            "ui/node",
            Some(serde_json::json!({ "elementId": id })),
        ))
        .is_error()
}

#[test]
fn automation_lanes_exist_in_the_live_arrangement_and_follow_both_toggles() {
    // `demo_project()` 自带自动化泳道（`automation.rs` 的判据同样以它为前提）。
    let project = yeban_app::bridge::demo_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    assert!(
        !view.automation_lanes.is_empty(),
        "夹具前提：投影里必须有泳道"
    );
    let lane_ids: Vec<String> = view
        .automation_lanes
        .iter()
        .map(|lane| lane.element_id.clone())
        .collect();

    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<automation-lane-wiring>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::ReadOnly,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("活界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    // 菜单命令（`toggle-automation`）由 `wire_aether_actions` 接管 —— 与产品进程同一条接线。
    host::wire_aether_actions(
        &window,
        Rc::clone(&port),
        Rc::new(RefCell::new(EngineHost::new())),
        Rc::new(RefCell::new(view.clone())),
    );
    let mut plane = ui.into_control_plane(Permission::ReadOnly);

    // ① 默认可见：每条泳道都在树里。
    assert!(window.get_automation_visible(), "默认必须展开自动化泳道");
    for (offset, lane_id) in lane_ids.iter().enumerate() {
        assert!(
            node_exists(&mut plane, 10 + offset as i64, lane_id),
            "默认视图里泳道 `{lane_id}` 必须在运行时树里（修前永远不在）"
        );
    }

    // ② `Ctrl+Shift+A`：收起 ⇒ 从树里消失；再按一次 ⇒ 回来。
    assert!(window.invoke_key_action("a".into(), true, true, false, false));
    assert!(
        !window.get_automation_visible(),
        "第一次 Ctrl+Shift+A 必须收起"
    );
    for (offset, lane_id) in lane_ids.iter().enumerate() {
        assert!(
            !node_exists(&mut plane, 30 + offset as i64, lane_id),
            "收起后泳道 `{lane_id}` 必须从树里消失"
        );
    }
    assert!(window.invoke_key_action("a".into(), true, true, false, false));
    assert!(
        window.get_automation_visible(),
        "第二次 Ctrl+Shift+A 必须展开"
    );
    assert!(
        node_exists(&mut plane, 50, &lane_ids[0]),
        "展开后第一条泳道必须回到树里"
    );

    // ③ 菜单命令：同一落点（`automation-visible` 回读 + 树同步）。
    window.invoke_command_execute("toggle-automation".into());
    assert!(
        !window.get_automation_visible(),
        "菜单「自动化泳道」必须收起"
    );
    assert!(
        !node_exists(&mut plane, 60, &lane_ids[0]),
        "菜单收起后泳道必须从树里消失（与 Ctrl+Shift+A 同一条落点）"
    );
    window.invoke_command_execute("toggle-automation".into());
    assert!(window.get_automation_visible(), "再点一次菜单必须展开");
    assert!(node_exists(&mut plane, 70, &lane_ids[0]));
}
