//! T-FIX-166：编曲区选定轨道头的右键菜单（Logic Pro 轨道头口径）。
//!
//! ## 被验证的链路
//!
//! | 环 | 位置 |
//! | :--- | :--- |
//! | 输入面 | `ui/arrangement.slint` 两份镜像轨道头（左 / 右）的 `TouchArea.pointer-event`，只有右键 `down` 开菜单 |
//! | 弹层 | `track-context-popup`（`PopupWindow`，`close-on-click-outside`，默认不显示） |
//! | 上抛 | 删轨 / 建轨走既有 `execute-action` 表；录音臂走 `toggle-arm(id)`；效果槽走 `track-instrument-change(id, "fx:*")` |
//! | 宿主 | `wire_track_strip` / `wire_track_arm` / `wire_track_instrument_change` 的真实 `UndoPort` 提交 |
//!
//! ## 口径
//!
//! 1. 右键**未选中**轨道：先把该轨选中再弹菜单（Logic 口径）；
//! 2. 点外关闭：`close-on-click-outside`，不是自绘浮层的“下一次点击”补丁；
//! 3. 点删除**真的**删轨：断言权威工程轨道数减一，不只看按钮回调被调用；
//! 4. 条目齐全：8 个语义 ID 全部在弹层打开时进入运行时控件树；
//! 5. 默认帧不回归：弹层只在右键 `down` 时 `show()`，测试外不会出现在控件树里。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
mod live;

use live::{LiveControlPlane, build_live_ui_with};
use serde_json::json;
use slint::Model as _;
use std::rc::Rc;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_KEY_PRESS, METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE,
    METHOD_DISPATCH_POINTER_UP, METHOD_NODE,
};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::png;
use yeban_ui_test_port::port::Permission;

const NOW_MS: u64 = 1_760_000_000_000;
const LANES: &str = "arr-lanes-flickable";
const MENU_ITEMS: [&str; 9] = [
    "arrangement-track-context-arm",
    "arrangement-track-context-rename",
    "arrangement-track-context-delete",
    "arrangement-track-context-add-midi",
    "arrangement-track-context-add-audio",
    "arrangement-track-context-add-aux",
    "arrangement-track-context-instrument-eq",
    "arrangement-track-context-instrument-reverb",
    "arrangement-track-context-instrument-compressor",
];

fn assemble() -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<arrangement-track-context-menu>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开会话"),
    ));
    let ui = build_live_ui_with(
        &project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    // `build_live_ui_with` 不装配产品进程的 `wire_aether_actions`（需要引擎句柄）；
    // 该函数里的 `on_track_selected` 才是“选中下标 → 选中身份字符串 + 重投影”的
    // 产品口径。这里补上它，才能让右键选中第二轨后的删轨真的落在第二轨，而不是
    // `display_track` 的兜底首轨。生产进程在 `main.rs` 里本来就调用它。
    let view = yeban_app::bridge::ViewState::from_project(&project).expect("投影");
    yeban_app::host::wire_aether_actions(
        ui.ui(),
        Rc::clone(&port),
        Rc::new(std::cell::RefCell::new(
            yeban_app::engine_host::EngineHost::new(),
        )),
        Rc::new(std::cell::RefCell::new(view)),
    );
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

fn node_bounds(plane: &mut LiveControlPlane, seq: i64, id: &str) -> serde_json::Value {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": id })),
    ));
    assert!(!node.is_error(), "`{id}` 必须在运行时控件树里: {node:?}");
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "`{id}` 必须有几何包围盒");
    bounds
}

fn tree_has(plane: &mut LiveControlPlane, seq: i64, id: &str) -> bool {
    !plane
        .plane()
        .try_line(&request_line(
            seq,
            METHOD_NODE,
            Some(json!({ "elementId": id })),
        ))
        .is_error()
}

fn move_to(plane: &mut LiveControlPlane, seq: i64, x: f64, y: f64) {
    let moved = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({ "x": x, "y": y })),
    ));
    assert!(!moved.is_error(), "指针移到 ({x}, {y}) 必须成功: {moved:?}");
}

fn pointer_down_at(
    plane: &mut LiveControlPlane,
    seq: i64,
    id: &str,
    x_offset: f64,
    y_offset: f64,
    button: &str,
) {
    let down = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": button
        })),
    ));
    assert!(
        !down.is_error(),
        "`{id}` 在 ({x_offset}, {y_offset}) 按下（{button}）必须落到真实窗口: {down:?}"
    );
}

fn pointer_up(plane: &mut LiveControlPlane, seq: i64, button: &str) {
    let up = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": button })),
    ));
    assert!(!up.is_error(), "`{button}` 松手必须落到真实窗口: {up:?}");
}

/// 在轨道行（`track_index`）的包头里右键：先移指针（`TouchArea.mouse-x/y` 才会更新），
/// 再按下（弹层在 `down` 时出现）。
fn right_click_header(
    plane: &mut LiveControlPlane,
    seq: i64,
    window: &yeban_app::ui::MainWindow,
    track_index: usize,
) -> (f64, f64) {
    let bounds = node_bounds(plane, seq, LANES);
    let ys = window.get_track_ys();
    let heights = window.get_track_heights();
    let y = ys.row_data(track_index).expect("行顶")
        + heights.row_data(track_index).expect("行高") / 2.0;
    let x = 20.0;
    let absolute_x = bounds["x"].as_f64().expect("x") + x;
    let absolute_y = bounds["y"].as_f64().expect("y") + f64::from(y);
    move_to(plane, seq + 1, absolute_x, absolute_y);
    pointer_down_at(plane, seq + 2, LANES, x, f64::from(y), "right");
    pointer_up(plane, seq + 3, "right");
    (absolute_x, absolute_y)
}

/// 点弹层条目。`PopupWindow` 的子节点包围盒是**弹层局部坐标**（实测 x=4 / y=34），
/// 不能把中心点直接交给 `dispatch_pointer_down`（那会点到主窗口的 (125, 49)，落进
/// “点外关闭”分支）。这里用弹层原点 + 条目的局部包围盒，经 `LANES` 的全局包围盒
/// 换算成窗口坐标，才是真点中条目。
fn click_menu_item(plane: &mut LiveControlPlane, seq: i64, popup_x: f64, popup_y: f64, id: &str) {
    let lanes = node_bounds(plane, seq, LANES);
    let item = node_bounds(plane, seq + 1, id);
    let absolute_x = popup_x
        + item["x"].as_f64().expect("item x")
        + item["width"].as_f64().expect("item width") / 2.0;
    let absolute_y = popup_y
        + item["y"].as_f64().expect("item y")
        + item["height"].as_f64().expect("item height") / 2.0;
    pointer_down_at(
        plane,
        seq + 2,
        LANES,
        absolute_x - lanes["x"].as_f64().expect("lanes x"),
        absolute_y - lanes["y"].as_f64().expect("lanes y"),
        "left",
    );
    pointer_up(plane, seq + 3, "left");
}

/// 判据①：右键未选中的第二轨 ⇒ 先选中该轨，再弹出齐全的菜单；点外关闭。
#[test]
fn right_click_header_selects_then_opens_all_items_and_clicks_outside_closes() {
    let (window, _port, mut plane) = assemble();
    let first_id = window.get_tracks().row_data(0).expect("首轨").id;
    let second_id = window.get_tracks().row_data(1).expect("第二轨").id;
    window.set_selected_track_index(first_id);
    assert_ne!(first_id, second_id, "演示工程至少两条可选轨");

    right_click_header(&mut plane, 1, &window, 1);
    assert_eq!(
        window.get_selected_track_index(),
        second_id,
        "右键未选中轨必须先选中该轨（Logic 口径）"
    );
    for (offset, id) in MENU_ITEMS.iter().enumerate() {
        assert!(
            tree_has(&mut plane, 10 + offset as i64, id),
            "轨道右键菜单打开时必须含条目 `{id}`"
        );
    }

    // 点外关闭：点在时间线（不是菜单）上；菜单在包头左列，1000px 之外。
    let bounds = node_bounds(&mut plane, 30, LANES);
    pointer_down_at(
        &mut plane,
        31,
        LANES,
        1000.0,
        bounds["height"].as_f64().expect("height") / 2.0,
        "left",
    );
    pointer_up(&mut plane, 32, "left");
    assert!(
        !tree_has(&mut plane, 33, "arrangement-track-context-delete"),
        "点菜单外必须关闭弹层（close-on-click-outside）"
    );
}

/// 判据③（T-FIX-166 补：**右置**轨道头分支）：`Theme.track-headers-on-right = true`
/// 时右键右侧轨道头同样先选中、弹齐 9 项、点删除真的删轨。
///
/// 同时钉住判据缺口本身：旧判据的固定落点（`LANES` 的 `x = 20`）在右置布局里落在
/// **时间线**，⛔ 不会弹出轨道菜单 —— 修前那一版测试正是因此只覆盖了左置分支。
#[test]
fn right_placed_headers_open_the_same_context_menu_and_delete_the_track() {
    use slint::ComponentHandle as _;
    use yeban_ui_test_port::inspect::find_by_accessible_id;

    let (window, port, mut plane) = assemble();
    // 右置头是产品支持的布局（`persona == AbletonLive` 的默认，也可手动切换）：
    // 直接切主题全局属性，而不是另造一份窗口 —— 判据要覆盖的就是**同一棵树**的镜像分支。
    window
        .global::<yeban_app::ui::Theme<'_>>()
        .set_track_headers_on_right(true);
    assert!(
        window
            .global::<yeban_app::ui::Theme<'_>>()
            .get_track_headers_on_right(),
        "右置布局必须已生效（否则本判据测的还是左置分支）"
    );
    window.window().request_redraw();

    let first_id = window.get_tracks().row_data(0).expect("首轨").id;
    let second_id = window.get_tracks().row_data(1).expect("第二轨").id;
    window.set_selected_track_index(first_id);
    assert_ne!(first_id, second_id, "演示工程至少两条可选轨");

    // ① 对照：旧判据的固定落点（x = 20）在右置布局下是**时间线** ⇒ 不弹轨道菜单。
    //    这一条就是「旧测试覆盖不到右置分支」的机械证明。
    let lanes = node_bounds(&mut plane, 1, LANES);
    let ys = window.get_track_ys();
    let heights = window.get_track_heights();
    let row_y = ys.row_data(1).expect("行顶") + heights.row_data(1).expect("行高") / 2.0;
    let legacy_x = 20.0;
    move_to(
        &mut plane,
        2,
        lanes["x"].as_f64().expect("x") + legacy_x,
        lanes["y"].as_f64().expect("y") + f64::from(row_y),
    );
    pointer_down_at(&mut plane, 3, LANES, legacy_x, f64::from(row_y), "right");
    pointer_up(&mut plane, 4, "right");
    assert!(
        !tree_has(&mut plane, 5, "arrangement-track-context-delete"),
        "右置布局下 x = 20 落在时间线，不该弹出轨道右键菜单（旧判据的固定落点）"
    );

    // ② 真实右置轨道头（画布右缘向左 `headers-width` 那一列的中点）右键 ⇒ 选中第二轨。
    let headers = f64::from(window.get_track_headers_width());
    let right_x = lanes["width"].as_f64().expect("lanes width") - headers / 2.0;
    move_to(
        &mut plane,
        10,
        lanes["x"].as_f64().expect("x") + right_x,
        lanes["y"].as_f64().expect("y") + f64::from(row_y),
    );
    pointer_down_at(&mut plane, 11, LANES, right_x, f64::from(row_y), "right");
    pointer_up(&mut plane, 12, "right");
    assert_eq!(
        window.get_selected_track_index(),
        second_id,
        "右置轨道头右键未选中轨必须先选中该轨（与左置同一口径）"
    );
    for (offset, id) in MENU_ITEMS.iter().enumerate() {
        assert!(
            tree_has(&mut plane, 20 + offset as i64, id),
            "右置轨道头右键菜单打开时必须含条目 `{id}`"
        );
    }

    // ③ 点删除（条目默认无障碍动作 = 鼠标点击的同一落点）⇒ 权威工程轨道数减一。
    let before = port.project().tracks.len();
    find_by_accessible_id(&window, "arrangement-track-context-delete")
        .expect("右置布局下删除条目必须在运行时树里")
        .invoke_accessible_default_action();
    assert_eq!(
        port.project().tracks.len(),
        before - 1,
        "右置轨道头菜单的「删除」必须真的删掉一条轨"
    );
    assert!(
        !tree_has(&mut plane, 40, "arrangement-track-context-delete"),
        "点条目后菜单必须关闭"
    );
}

/// 判据④（源串）：左 / 右两套镜像都必须保留**右置偏移**的坐标口径。
///
/// 行为判据（③）只能证明“点在右置头时菜单会弹”，看不出菜单**定位**公式是否在两套
/// 镜像里漂移（例如右置分支误用左置的 `0px` 起点）。这里机械钉住两件事：
/// ① 两份 `open-track-context-menu(...)` 的 x 都用 `Theme.track-headers-on-right ?
///    root.width - root.headers-width : 0px` 做基底；② 镜像对没有多出第三处（避免有人
/// 复制一份新分支却漏改）。
#[test]
fn both_header_mirrors_keep_the_right_side_offset_in_the_popup_origin() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/arrangement.slint"),
    )
    .expect("读 arrangement.slint");
    let marker = "Theme.track-headers-on-right ? root.width - root.headers-width : 0px";
    assert_eq!(
        source.matches(marker).count(),
        2,
        "左 / 右两套轨道头必须各用一次右置偏移作为菜单 x 基底（多一处或漏一处都说明镜像漂了）"
    );
    assert_eq!(
        source.matches("root.open-track-context-menu(").count(),
        2,
        "轨道右键入口必须恰好两处（左置 / 右置镜像各一）"
    );
}

/// 判据②：菜单「删除当前选中轨道」真的提交删轨（权威工程轨道数减一）。
#[test]
fn delete_item_really_removes_the_track_from_the_authority() {
    let (window, port, mut plane) = assemble();
    let before = port.project().tracks.len();
    assert!(before >= 2, "演示工程至少两条可选轨");

    let (popup_x, popup_y) = right_click_header(&mut plane, 1, &window, 1);
    assert_eq!(
        window.get_selected_track_index(),
        window.get_tracks().row_data(1).expect("第二轨").id,
        "右键先选中第二轨"
    );
    assert!(
        !window.get_selected_track_id().is_empty(),
        "右键选中必须落下轨道身份（删轨按身份作用，而不是 display_track 兜底首轨）"
    );
    click_menu_item(
        &mut plane,
        10,
        popup_x,
        popup_y,
        "arrangement-track-context-delete",
    );

    let after = port.project().tracks.len();
    assert_eq!(
        after,
        before - 1,
        "点删除必须真的删掉一条轨（权威工程轨道数减一）"
    );
    assert!(
        !tree_has(&mut plane, 20, "arrangement-track-context-delete"),
        "点条目后菜单必须关闭"
    );
}

fn press_key(plane: &mut LiveControlPlane, seq: i64, key_code: &str) {
    let pressed = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_KEY_PRESS,
        Some(json!({ "keyCode": key_code })),
    ));
    assert!(
        !pressed.is_error(),
        "键盘注入 {key_code} 必须落到真实窗口: {pressed:?}"
    );
}

/// 判据⑤（T-FIX-195）：菜单「重命名」→ 轨道头内联输入 → Enter 提交到权威工程与界面。
#[test]
fn rename_menu_item_opens_inline_editor_and_enter_commits() {
    use slint::ComponentHandle as _;
    use yeban_ui_test_port::inspect::find_by_accessible_id;

    let (window, port, mut plane) = assemble();
    let index = 1usize;
    let track_id = window
        .get_track_ids()
        .row_data(index)
        .expect("第二轨的实体 ID");
    let old_name = window
        .get_tracks()
        .row_data(index)
        .expect("第二轨")
        .name
        .to_string();
    let (popup_x, popup_y) = right_click_header(&mut plane, 1, &window, index);
    click_menu_item(
        &mut plane,
        10,
        popup_x,
        popup_y,
        "arrangement-track-context-rename",
    );
    assert!(
        !tree_has(&mut plane, 14, "arrangement-track-context-delete"),
        "点「重命名」后菜单必须关闭"
    );
    let input = find_by_accessible_id(&window, "arrangement-track-rename-input")
        .expect("轨道头内联改名输入必须进入运行时子树");
    assert_eq!(
        input.accessible_value().as_deref(),
        Some(old_name.as_str()),
        "编辑器必须预填当前轨道名，而不是空串"
    );
    input.set_accessible_value("重命名后的轨");
    press_key(&mut plane, 20, "Enter");

    let id = track_id
        .as_str()
        .parse::<yeban_model::EntityId>()
        .expect("实体 ID");
    assert_eq!(
        port.project().tracks.get(&id).expect("轨道仍在").name,
        "重命名后的轨",
        "Enter 必须提交 Op::SetTrackName 到权威工程"
    );
    assert_eq!(
        window.get_tracks().row_data(index).expect("第二轨").name,
        "重命名后的轨",
        "重投影后界面轨道名必须同步更新"
    );
    assert!(
        !tree_has(&mut plane, 30, "arrangement-track-rename-input"),
        "提交后内联编辑器必须关闭"
    );
}

/// 判据⑥（T-FIX-195）：Esc 取消不改名；空名不提交。
#[test]
fn rename_editor_escape_and_empty_name_leave_the_track_unchanged() {
    use slint::ComponentHandle as _;
    use yeban_ui_test_port::inspect::find_by_accessible_id;

    let (window, port, mut plane) = assemble();
    let index = 1usize;
    let track_id = window
        .get_track_ids()
        .row_data(index)
        .expect("第二轨的实体 ID");
    let old_name = window
        .get_tracks()
        .row_data(index)
        .expect("第二轨")
        .name
        .to_string();
    let (popup_x, popup_y) = right_click_header(&mut plane, 1, &window, index);
    click_menu_item(
        &mut plane,
        10,
        popup_x,
        popup_y,
        "arrangement-track-context-rename",
    );
    let _ =
        find_by_accessible_id(&window, "arrangement-track-rename-input").expect("改名输入必须出现");
    press_key(&mut plane, 20, "Escape");
    let id = track_id
        .as_str()
        .parse::<yeban_model::EntityId>()
        .expect("实体 ID");
    assert_eq!(
        port.project().tracks.get(&id).expect("轨道仍在").name,
        old_name,
        "Esc 必须取消，不改工程名"
    );
    assert!(
        !tree_has(&mut plane, 30, "arrangement-track-rename-input"),
        "Esc 后内联编辑器必须关闭"
    );

    // 空名：再次打开编辑器，清空草稿后 Enter ⇒ 不提交（工程名仍是旧名）。
    let (popup_x, popup_y) = right_click_header(&mut plane, 40, &window, index);
    click_menu_item(
        &mut plane,
        50,
        popup_x,
        popup_y,
        "arrangement-track-context-rename",
    );
    let input = find_by_accessible_id(&window, "arrangement-track-rename-input")
        .expect("第二次改名输入必须出现");
    input.set_accessible_value("");
    press_key(&mut plane, 60, "Enter");
    assert_eq!(
        port.project().tracks.get(&id).expect("轨道仍在").name,
        old_name,
        "空名不得提交"
    );
}

/// 判据（T-FIX-843）：进入重命名即全选旧名；组字/编辑中点另一条轨道头，
/// 编辑态退出、草稿不提交、原轨名不变。左置轨道头分支。
#[test]
fn rename_editor_selects_all_and_clicking_another_track_cancels() {
    use slint::ComponentHandle as _;
    use yeban_ui_test_port::inspect::find_by_accessible_id;

    let (window, port, mut plane) = assemble();
    let index = 1usize;
    let track_id = window.get_track_ids().row_data(index).expect("第二轨 ID");
    let old_name = window
        .get_tracks()
        .row_data(index)
        .expect("第二轨")
        .name
        .to_string();
    let (popup_x, popup_y) = right_click_header(&mut plane, 1, &window, index);
    click_menu_item(
        &mut plane,
        10,
        popup_x,
        popup_y,
        "arrangement-track-context-rename",
    );
    let input =
        find_by_accessible_id(&window, "arrangement-track-rename-input").expect("重命名输入");
    assert_eq!(input.accessible_value().as_deref(), Some(old_name.as_str()));

    // 全选语义的行为判据：首键必须**替换**旧名，而不是拼到旧名前/后。
    press_key(&mut plane, 20, "x");
    let input =
        find_by_accessible_id(&window, "arrangement-track-rename-input").expect("首键后输入框仍在");
    assert_eq!(
        input.accessible_value().as_deref(),
        Some("x"),
        "进入重命名必须全选旧名：首键应得到纯新名 `x`，不是 `x{old_name}` 或 `{old_name}x`"
    );

    // 点另一条轨道头：选中身份改变 ⇒ 编辑态退出、草稿丢弃。
    let lanes = node_bounds(&mut plane, 30, LANES);
    let ys = window.get_track_ys();
    let heights = window.get_track_heights();
    let row_y = ys.row_data(0).expect("首轨顶") + heights.row_data(0).expect("首轨高") / 2.0;
    move_to(
        &mut plane,
        31,
        lanes["x"].as_f64().expect("x") + 20.0,
        lanes["y"].as_f64().expect("y") + f64::from(row_y),
    );
    pointer_down_at(&mut plane, 32, LANES, 20.0, f64::from(row_y), "left");
    pointer_up(&mut plane, 33, "left");

    assert_eq!(
        window.get_selected_track_index(),
        1,
        "点击首轨头必须切到 1-based 下标 1（否则本判据没有覆盖失焦路径）"
    );
    assert!(
        !tree_has(&mut plane, 34, "arrangement-track-rename-input"),
        "选中身份改变后重命名输入框必须消失"
    );
    let id = track_id
        .as_str()
        .parse::<yeban_model::EntityId>()
        .expect("ID");
    assert_eq!(
        port.project().tracks.get(&id).expect("轨道仍在").name,
        old_name,
        "失焦取消不得把草稿提交给原轨道"
    );
}

/// 判据（T-FIX-843）：点窗口内其它区域（车道空白）同样取消编辑态，且不提交。
#[test]
fn rename_editor_clicking_blank_lane_area_cancels_without_commit() {
    use slint::ComponentHandle as _;
    use yeban_ui_test_port::inspect::find_by_accessible_id;

    let (window, port, mut plane) = assemble();
    let index = 1usize;
    let track_id = window.get_track_ids().row_data(index).expect("第二轨 ID");
    let old_name = window
        .get_tracks()
        .row_data(index)
        .expect("第二轨")
        .name
        .to_string();
    let (popup_x, popup_y) = right_click_header(&mut plane, 1, &window, index);
    click_menu_item(
        &mut plane,
        10,
        popup_x,
        popup_y,
        "arrangement-track-context-rename",
    );
    let _ = find_by_accessible_id(&window, "arrangement-track-rename-input").expect("重命名输入");
    press_key(&mut plane, 20, "y");
    assert_eq!(
        find_by_accessible_id(&window, "arrangement-track-rename-input")
            .expect("输入框仍在")
            .accessible_value()
            .as_deref(),
        Some("y"),
        "首键替换旧名（全选）"
    );

    // 末轨行的最右时间线空白：既不落在轨道头，也不落在片段上。
    let lanes = node_bounds(&mut plane, 30, LANES);
    let ys = window.get_track_ys();
    let heights = window.get_track_heights();
    let last = ys.row_count() - 1;
    let blank_y =
        ys.row_data(last).expect("末轨顶") + heights.row_data(last).expect("末轨高") / 2.0;
    let headers = f64::from(window.get_track_headers_width());
    let blank_x = lanes["width"].as_f64().expect("lanes 宽") - headers - 50.0;
    assert!(
        blank_x > headers + 20.0,
        "空白落点必须落在时间线而非包头（x={blank_x}）"
    );
    move_to(
        &mut plane,
        31,
        lanes["x"].as_f64().expect("x") + blank_x,
        lanes["y"].as_f64().expect("y") + f64::from(blank_y),
    );
    pointer_down_at(&mut plane, 32, LANES, blank_x, f64::from(blank_y), "left");
    pointer_up(&mut plane, 33, "left");

    assert!(
        !tree_has(&mut plane, 34, "arrangement-track-rename-input"),
        "点击窗口其它区域后重命名输入框必须消失"
    );
    let id = track_id
        .as_str()
        .parse::<yeban_model::EntityId>()
        .expect("ID");
    assert_eq!(
        port.project().tracks.get(&id).expect("轨道仍在").name,
        old_name,
        "失焦取消不得提交草稿"
    );
    assert_eq!(
        window.get_tracks().row_data(index).expect("第二轨").name,
        old_name,
        "界面上的轨道名也不得变成草稿"
    );
}

/// 判据（T-FIX-843）：**右置**轨道头镜像实例同样全选旧名、失焦取消。
///
/// 两处 `arrangement-track-rename-input` 的条件分支各只有一份实例；翻转
/// `Theme.track-headers-on-right` 后才走另一份。本判据不只看源码串，而是在
/// 同一棵树里实际进入右置分支并键入首键。
#[test]
fn right_placed_headers_share_select_all_and_blur_cancel() {
    use slint::ComponentHandle as _;
    use yeban_ui_test_port::inspect::find_by_accessible_id;

    let (window, port, mut plane) = assemble();
    window
        .global::<yeban_app::ui::Theme<'_>>()
        .set_track_headers_on_right(true);
    window.window().request_redraw();

    let index = 1usize;
    let track_id = window.get_track_ids().row_data(index).expect("第二轨 ID");
    let old_name = window
        .get_tracks()
        .row_data(index)
        .expect("第二轨")
        .name
        .to_string();
    let lanes = node_bounds(&mut plane, 1, LANES);
    let headers = f64::from(window.get_track_headers_width());
    let right_x = lanes["width"].as_f64().expect("lanes 宽") - headers / 2.0;
    let ys = window.get_track_ys();
    let heights = window.get_track_heights();
    let row_y =
        ys.row_data(index).expect("二轨顶") + heights.row_data(index).expect("二轨高") / 2.0;
    move_to(
        &mut plane,
        2,
        lanes["x"].as_f64().expect("x") + right_x,
        lanes["y"].as_f64().expect("y") + f64::from(row_y),
    );
    pointer_down_at(&mut plane, 3, LANES, right_x, f64::from(row_y), "right");
    pointer_up(&mut plane, 4, "right");
    let popup_x = lanes["x"].as_f64().expect("x") + right_x;
    let popup_y = lanes["y"].as_f64().expect("y") + f64::from(row_y);
    // 右置头的菜单原点可能被右缘夹到 `root.width - 250`，不再等于点击点；
    // 用条目的无障碍默认动作进入重命名，落点与鼠标点击同一条 `activate()`。
    find_by_accessible_id(&window, "arrangement-track-context-rename")
        .expect("右置菜单的重命名条目")
        .invoke_accessible_default_action();
    assert_eq!(
        find_by_accessible_id(&window, "arrangement-track-rename-input")
            .expect("右置重命名输入")
            .accessible_value()
            .as_deref(),
        Some(old_name.as_str())
    );
    press_key(&mut plane, 20, "z");
    assert_eq!(
        find_by_accessible_id(&window, "arrangement-track-rename-input")
            .expect("右置首键后输入框")
            .accessible_value()
            .as_deref(),
        Some("z"),
        "右置实例也必须全选旧名"
    );

    // 点另一条右置轨道头 ⇒ 取消。
    let row0_y = ys.row_data(0).expect("首轨顶") + heights.row_data(0).expect("首轨高") / 2.0;
    move_to(
        &mut plane,
        30,
        lanes["x"].as_f64().expect("x") + right_x,
        lanes["y"].as_f64().expect("y") + f64::from(row0_y),
    );
    pointer_down_at(&mut plane, 31, LANES, right_x, f64::from(row0_y), "left");
    pointer_up(&mut plane, 32, "left");
    assert!(
        !tree_has(&mut plane, 33, "arrangement-track-rename-input"),
        "右置实例失焦后输入框必须消失"
    );
    let id = track_id
        .as_str()
        .parse::<yeban_model::EntityId>()
        .expect("ID");
    assert_eq!(
        port.project().tracks.get(&id).expect("轨道仍在").name,
        old_name,
        "右置实例失焦不得提交草稿"
    );
}

/// 判据（T-FIX-843，源串）：两处镜像输入都必须接全选 + 失焦取消，且
/// `ImeAwareTextInput` 的失焦语义是**可选回调**（其余调用点不受影响）。
#[test]
fn both_rename_inputs_are_wired_for_select_all_and_focus_loss() {
    let arrangement = include_str!("../ui/arrangement.slint");
    assert_eq!(
        arrangement.matches("self.select-all();").count(),
        2,
        "左 / 右两处重命名输入都必须进入编辑即全选"
    );
    assert_eq!(
        arrangement
            .matches("focus-loss-action => { root.cancel-track-rename(); }")
            .count(),
        2,
        "左 / 右两处重命名输入都必须接失焦取消"
    );
    assert_eq!(
        arrangement.matches("cancel-on-focus-loss: true;").count(),
        2,
        "左 / 右两处重命名输入都必须开启失焦取消开关"
    );
    assert_eq!(
        arrangement
            .matches("clear-selection-on-composition: true;")
            .count(),
        2,
        "左 / 右两处重命名输入都必须开启首次组字清旧名"
    );
    let tokens = include_str!("../ui/tokens.slint");
    assert!(
        tokens.contains("callback focus-loss-action();"),
        "ImeAwareTextInput 必须提供可选失焦动作"
    );
    assert!(
        tokens.contains("root.focus-loss-action();"),
        "失焦时必须触发 focus-loss-action"
    );
    assert!(
        tokens.contains("in property <bool> clear-selection-on-composition: false;"),
        "ImeAwareTextInput 必须提供可选的首次组字清旧名开关"
    );
    assert!(
        tokens.contains("root.ime-restore-pending = true;"),
        "组字取消时必须安排旧名恢复"
    );
}

/// 证据：菜单打开态截图（1:1）与 3x 最近邻裁切；只在设了 `T_FIX_166_SHOTS` 时落盘。
#[test]
fn capture_menu_open_evidence_when_requested() {
    let Ok(dir) = std::env::var("T_FIX_166_SHOTS") else {
        return;
    };
    let (window, _port, mut plane) = assemble();
    let (menu_x, menu_y) = right_click_header(&mut plane, 1, &window, 1);

    // `PopupWindow` 子节点的 `bounds` 是弹层**局部**坐标（首项 x=4 / y=4 附近的
    // padding），弹层原点就是右键落点；截图从落点裁 250×278 才是整块菜单。
    let arm = node_bounds(&mut plane, 10, "arrangement-track-context-arm");
    let delete = node_bounds(&mut plane, 11, "arrangement-track-context-delete");
    assert!(arm["x"].as_f64().expect("arm x") >= 0.0);
    assert!(delete["y"].as_f64().expect("delete y") > arm["y"].as_f64().expect("arm y"));
    assert!(menu_x >= 0.0 && menu_y >= 0.0, "菜单外框必须在窗口内");

    let image = plane.capture().expect("抓菜单打开态");
    let path = std::path::Path::new(&dir);
    std::fs::create_dir_all(path).expect("建截图目录");
    std::fs::write(
        path.join("track-context-menu-open-1x.png"),
        png::encode_rgb8(&image),
    )
    .expect("写 1:1 截图");

    // 3x 最近邻：以菜单左侧为起点裁 250×278，再逐像素放大。
    let crop_x = menu_x as u32;
    let crop_y = menu_y as u32;
    let mut crop = yeban_ui_test_port::image::Rgb8Image::new(Size::new(250, 278));
    for y in 0..278 {
        for x in 0..250 {
            if let Some(pixel) = image.pixel(crop_x + x, crop_y + y) {
                crop.set_pixel(x, y, pixel);
            }
        }
    }
    let mut zoom = yeban_ui_test_port::image::Rgb8Image::new(Size::new(750, 834));
    for y in 0..834 {
        for x in 0..750 {
            if let Some(pixel) = crop.pixel(x / 3, y / 3) {
                zoom.set_pixel(x, y, pixel);
            }
        }
    }
    std::fs::write(
        path.join("track-context-menu-open-3x.png"),
        png::encode_rgb8(&zoom),
    )
    .expect("写 3x 截图");
}
