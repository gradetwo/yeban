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
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    METHOD_NODE,
};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::png;
use yeban_ui_test_port::port::Permission;

const NOW_MS: u64 = 1_760_000_000_000;
const LANES: &str = "arr-lanes-flickable";
const MENU_ITEMS: [&str; 8] = [
    "arrangement-track-context-arm",
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

/// 证据：菜单打开态截图（1:1）与 3x 最近邻裁切；只在设了 `T_FIX_166_SHOTS` 时落盘。
#[test]
fn capture_menu_open_evidence_when_requested() {
    let Ok(dir) = std::env::var("T_FIX_166_SHOTS") else {
        return;
    };
    let (window, _port, mut plane) = assemble();
    let (menu_x, menu_y) = right_click_header(&mut plane, 1, &window, 1);

    // `PopupWindow` 子节点的 `bounds` 是弹层**局部**坐标（首项 x=4 / y=4 附近的
    // padding），弹层原点就是右键落点；截图从落点裁 250×248 才是整块菜单。
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

    // 3x 最近邻：以菜单左侧为起点裁 250×248，再逐像素放大。
    let crop_x = menu_x as u32;
    let crop_y = menu_y as u32;
    let mut crop = yeban_ui_test_port::image::Rgb8Image::new(Size::new(250, 248));
    for y in 0..248 {
        for x in 0..250 {
            if let Some(pixel) = image.pixel(crop_x + x, crop_y + y) {
                crop.set_pixel(x, y, pixel);
            }
        }
    }
    let mut zoom = yeban_ui_test_port::image::Rgb8Image::new(Size::new(750, 744));
    for y in 0..744 {
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
