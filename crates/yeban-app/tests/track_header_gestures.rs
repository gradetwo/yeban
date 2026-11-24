//! **轨道头手势**（T-FIX-232）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 活编曲视图的轨道头手势在重构后只剩在死文件里（`grep reorder|height-grab|height-drag
//! crates/yeban-app/ui/arrangement.slint` 为 0），根回调声明了、宿主接了线，却没有任何
//! 活控件触发它们。本切片把死文件的手势分区（名称区 = 重排、底边 6px = 高度）搬进活
//! `ArrangementViewport`，并经 `app.slint` 转发到根回调（宿主接线已在）。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（高度拖拽） | 按住 0 号轨包头底边向上拖 20px ⇒ `track-heights[0]` **恰好 +20**、其余行一位不动、拖拽态收尾清零；高度是**视图态** ⇒ 提交数 Δ0（⛔ 不是 Op） |
//! | ②（`Escape` 取消） | 第二次高度拖拽中途 `Escape` ⇒ 行高写回起点、拖拽态清零、提交数不变 |
//! | ③（孤立移动） | 没有按下的 `move` ⇒ 行几何一位不动（宿主 `active` 守卫） |
//! | ④（拖放重排） | 按住 0 号轨名称区向下拖过 1 号轨中点 ⇒ 插入指示线出现（`preview-index >= 0`）、松手后 `track-ids` 顺序真的变了、提交数 **+1**（恰好一次 `Op::MoveTrack`） |
//! | ⑤（可撤销） | 重排提交后走产品 `undo-step` 入口 `Undo` ⇒ 工程 `track_order` 与窗口轨道列都回到原顺序 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的且每线程只能装配一次 ⇒ 本文件只有一个
//! `#[test]`（与 `tests/clip_drag.rs` 同款）。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// `-D warnings` 会判红（与 `tests/clip_drag.rs` 同款）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveControlPlane, build_live_ui};
use serde_json::json;
use slint::Model as _;
use yeban_app::host::{
    wire_keys, wire_track_height_drag, wire_track_reorder, wire_track_select, wire_undo,
};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_KEY_PRESS, METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE,
    METHOD_DISPATCH_POINTER_UP, METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 打在轨道头上的**横向**落点（元素局部像素）。
///
/// ⚠ 不用"行宽的一半"：编排行里静音 / 独奏两块的命中区是 `x 16..38, y 26..44`，而它们是包头
/// 的**后**置子元素（Slint 自前往后命中 ⇒ 按钮先拿走点击，行头的拖拽面只接管剩下的区域）。
/// `150.0` 是 `tests/d_ui_11_serve_header_gestures.rs` 的 `HEADER_SAFE_X`（标题文本止于
/// `x 112`、色标止于 `x 134`，两者都是非交互元素）。
const HEADER_SAFE_X: f64 = 150.0;

/// 高度拖拽的纵向位移（逻辑像素，向上为正）—— 百分比 100 时基准像素 1:1，期望值手算。
const HEIGHT_DRAG_DY_PX: f64 = 20.0;

/// 一次拖动注入分几步（每步 5px ⇒ 第一步就越过宿主的 4px 阈值）。
const DRAG_STEPS: usize = 4;

/// 装配：真窗口 ＋ 撤销端口 ＋ **与产品同一条** `wire_track_select` /
/// `wire_track_height_drag` / `wire_track_reorder` / `wire_undo` / `wire_keys`（`Escape` 的取消路径需要带端口的 `wire_keys`）。
/// `build_live_ui` 不带 `LiveWiringOptions::undo` ⇒ `wire_track_select` 等在装配里不跑，
/// 判据侧按 `tests/track_selection.rs` 的老办法各自补一次（`live_surface.rs` 注释原话）。
fn assemble() -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<track-header-gestures>", "yeban-app", project, NOW_MS).expect("打开"),
    ));
    wire_track_select(&window, &port);
    wire_track_height_drag(&window, &port);
    wire_track_reorder(&window, &port);
    wire_undo(&window, &port);
    wire_keys(
        &window,
        ui.input_context(),
        Some(Rc::clone(&port)),
        None,
        None,
    );
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 取某个**语义 ID** 在运行时控件树里的包围盒（每次都重读 —— 端口里那份会过期）。
fn element_bounds(
    plane: &mut LiveControlPlane,
    seq: i64,
    element_id: &str,
) -> (i32, i32, u32, u32) {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    assert!(
        !node.is_error(),
        "`{element_id}` 必须在运行时控件树里（[UI-TEST-001] 语义 ID 寻址）: {node:?}"
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "`{element_id}` 必须有几何包围盒");
    (
        i32::try_from(bounds["x"].as_i64().expect("x")).expect("x 在 i32 内"),
        i32::try_from(bounds["y"].as_i64().expect("y")).expect("y 在 i32 内"),
        u32::try_from(bounds["width"].as_u64().expect("width")).expect("width 在 u32 内"),
        u32::try_from(bounds["height"].as_u64().expect("height")).expect("height 在 u32 内"),
    )
}

/// 在语义 ID 的包围盒内 `(x_offset, y_offset)` 处按下；返回绝对窗口原点。
fn pointer_down_at(
    plane: &mut LiveControlPlane,
    seq: i64,
    element_id: &str,
    x_offset: f64,
    y_offset: f64,
) -> (f64, f64) {
    let (x, y, _, _) = element_bounds(plane, seq, element_id);
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    #[allow(clippy::cast_precision_loss)]
    (f64::from(x) + x_offset, f64::from(y) + y_offset)
}

/// 一次**窗口绝对坐标**的移动（`ui/dispatch_pointer_move` 的口径就是绝对值）。
fn pointer_move_to(plane: &mut LiveControlPlane, seq: i64, x: f64, y: f64) {
    let moved = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({ "x": x, "y": y })),
    ));
    assert!(!moved.is_error(), "移动必须落到真实窗口: {moved:?}");
}

/// 松手（`[UI-TEST-002]` §12.4 的口径：位置复用最后一次按下 / 移动）。
fn pointer_up(plane: &mut LiveControlPlane, seq: i64) {
    let up = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
}

/// 注入一次 `Escape`（`Action::Cancel` 的唯一键盘入口）。
fn press_escape(plane: &mut LiveControlPlane, seq: i64) {
    let pressed = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_KEY_PRESS,
        Some(json!({ "keyCode": "Escape" })),
    ));
    assert!(!pressed.is_error(), "`Escape` 注入必须成功: {pressed:?}");
}

/// 注入的行高数组（`[length]` → `Vec<f64>`，与 `tests/live_ui_mcp.rs` 的读数同款）。
fn heights(window: &yeban_app::ui::MainWindow) -> Vec<f64> {
    window.get_track_heights().iter().map(f64::from).collect()
}

/// 窗口自己的轨道身份列（与 `track-{i}-header` 同索引集）。
fn track_ids(window: &yeban_app::ui::MainWindow) -> Vec<String> {
    window
        .get_track_ids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// 工程写出的显式显示顺序（`track_order`；`Op::MoveTrack` 改的就是它；空 = 身份升序默认）。
fn project_track_order(port: &UndoPort) -> Vec<String> {
    port.project()
        .track_order
        .iter()
        .map(|id| id.to_string())
        .collect()
}

#[test]
fn track_header_gestures_drive_height_and_reorder_on_the_live_window() {
    let (window, port, mut plane) = assemble();
    let ids_before = track_ids(&window);
    assert!(
        ids_before.len() >= 3,
        "演示工程必须至少有 3 条轨（重排需要越过一整行）: {ids_before:?}"
    );
    let heights_before = heights(&window);
    assert_eq!(
        heights_before.len(),
        ids_before.len(),
        "行高数组必须与轨道列同长"
    );
    let h0_before = heights_before[0];
    let commits_before = port.display().commit_count;
    let undoable_before = port.display().undoable;
    assert!(
        port.project().track_order.is_empty(),
        "演示工程用空 `track_order`（= 身份升序）作默认显示顺序"
    );

    // ---- ①：按住 0 号轨包头底边向上拖 20px ⇒ 该行恰好 +20、其余行不动、收尾干净 ----
    let header0 = element_bounds(&mut plane, 10, "track-0-header");
    assert!(
        header0.3 as f64 > HEIGHT_DRAG_DY_PX + 6.0,
        "0 号轨包头必须够高（底边 6px 拖拽区 + 20px 上移）: {header0:?}"
    );
    let origin = pointer_down_at(
        &mut plane,
        20,
        "track-0-header",
        HEADER_SAFE_X,
        f64::from(header0.3) - 3.0,
    );
    // 按下即选中（死文件口径）：选中态必须先落到 0 号轨，否则下面的拖拽是在"没选中"的行上。
    assert_eq!(
        window.get_selected_track_id().as_str(),
        ids_before[0].as_str(),
        "按下包头必须先选中 0 号轨"
    );
    for step in 1..=DRAG_STEPS {
        #[allow(clippy::cast_precision_loss)]
        let fraction = step as f64 / DRAG_STEPS as f64;
        pointer_move_to(
            &mut plane,
            30 + i64::try_from(step).expect("步号"),
            origin.0,
            origin.1 - HEIGHT_DRAG_DY_PX * fraction,
        );
    }
    let heights_during = heights(&window);
    assert!(
        (heights_during[0] - (h0_before + HEIGHT_DRAG_DY_PX)).abs() < 1e-3,
        "拖动中 0 号轨行高必须是 {h0_before} + {HEIGHT_DRAG_DY_PX}，实测 {}",
        heights_during[0]
    );
    assert_eq!(
        &heights_during[1..],
        &heights_before[1..],
        "拖动中其余行的行高必须一位不动"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "高度是视图态，拖动期**不得**提交任何 op"
    );
    pointer_up(&mut plane, 40);
    assert!(
        !window.get_track_height_drag_active(),
        "松手后高度拖拽态必须清零"
    );
    assert_eq!(
        window.get_track_height_drag_track().as_str(),
        "",
        "松手后高度拖拽的轨道记录必须清零"
    );
    let heights_after_height_drag = heights(&window);
    assert!(
        (heights_after_height_drag[0] - (h0_before + HEIGHT_DRAG_DY_PX)).abs() < 1e-3,
        "松手后 0 号轨行高必须保持 {}，实测 {}",
        h0_before + HEIGHT_DRAG_DY_PX,
        heights_after_height_drag[0]
    );

    // ---- ②：第二次高度拖拽中途 `Escape` ⇒ 行高写回起点、拖拽态清零、提交数不变 ----
    let header0_after = element_bounds(&mut plane, 50, "track-0-header");
    let origin2 = pointer_down_at(
        &mut plane,
        60,
        "track-0-header",
        HEADER_SAFE_X,
        f64::from(header0_after.3) - 3.0,
    );
    pointer_move_to(&mut plane, 70, origin2.0, origin2.1 - 10.0);
    assert!(
        (heights(&window)[0] - (h0_before + HEIGHT_DRAG_DY_PX + 10.0)).abs() < 1e-3,
        "`Escape` 之前行高必须先被拖高 10px，实测 {}",
        heights(&window)[0]
    );
    press_escape(&mut plane, 80);
    pointer_up(&mut plane, 90);
    assert!(
        !window.get_track_height_drag_active(),
        "`Escape` 必须清掉高度拖拽态"
    );
    assert!(
        (heights(&window)[0] - (h0_before + HEIGHT_DRAG_DY_PX)).abs() < 1e-3,
        "`Escape` 取消后行高必须写回拖拽起点 {}，实测 {}",
        h0_before + HEIGHT_DRAG_DY_PX,
        heights(&window)[0]
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "`Escape` 取消不得提交任何 op"
    );

    // ---- ③：没有按下的 `move` ⇒ 行几何一位不动 ----
    pointer_move_to(&mut plane, 100, origin2.0, origin2.1 - 30.0);
    assert_eq!(
        heights(&window),
        heights_after_height_drag,
        "孤立的 move 永远改不动行高（宿主 `active` 守卫）"
    );

    // ---- ④：按住 0 号轨名称区向下拖过 1 号轨中点 ⇒ 插入线出现、松手换位、恰好一次提交 ----
    // 行几何现读现算：0 号轨已被 ① 拖高，行顶沿与中点都按当前值算（⛔ 不写死 56px）。
    let ys: Vec<f64> = window.get_track_ys().iter().map(f64::from).collect();
    let hs = heights(&window);
    let row1_mid = ys[1] + hs[1] / 2.0;
    let header0_now = element_bounds(&mut plane, 110, "track-0-header");
    let down_y = f64::from(header0_now.1) + f64::from(header0_now.3) / 2.0;
    let target_y = f64::from(header0_now.1) + row1_mid + 10.0;
    let origin3 = pointer_down_at(
        &mut plane,
        120,
        "track-0-header",
        HEADER_SAFE_X,
        f64::from(header0_now.3) / 2.0,
    );
    for step in 1..=DRAG_STEPS {
        #[allow(clippy::cast_precision_loss)]
        let fraction = step as f64 / DRAG_STEPS as f64;
        pointer_move_to(
            &mut plane,
            130 + i64::try_from(step).expect("步号"),
            origin3.0,
            down_y + (target_y - down_y) * fraction,
        );
    }
    assert!(
        window.get_track_reorder_preview_index() >= 0,
        "拖过 4px 阈值后重排预览必须出现（preview-index >= 0），实测 {}",
        window.get_track_reorder_preview_index()
    );
    let line = element_bounds(&mut plane, 140, "track-reorder-insertion-line");
    assert_eq!(line.3, 2, "插入指示线必须是 2px 高: {line:?}");
    pointer_up(&mut plane, 150);
    assert!(
        !window.get_track_reorder_active(),
        "松手后重排拖拽态必须清零"
    );
    assert_eq!(
        window.get_track_reorder_preview_index(),
        -1,
        "松手后插入预览必须清零"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次拖放重排 = **恰好一次**提交（`Op::MoveTrack`）"
    );
    let ids_after = track_ids(&window);
    assert_eq!(
        ids_after[0], ids_before[1],
        "0 号轨必须落到 1 号轨后面：{ids_before:?} → {ids_after:?}"
    );
    assert_eq!(
        ids_after[1], ids_before[0],
        "1 号轨必须顶上来：{ids_before:?} → {ids_after:?}"
    );
    assert_eq!(
        project_track_order(&port),
        ids_after,
        "窗口轨道列必须与工程显示顺序一致"
    );
    assert_eq!(
        port.display().undoable,
        undoable_before + 1,
        "重排提交后可撤销步数必须恰好 +1"
    );

    // ---- ⑤：`Undo`（走产品同一条 `undo-step` 入口：提交 + 重投影）⇒ 工程显示顺序回到原位 ----
    // ⛔ 不直接调 `port.perform(UiAction::Undo)`：端口是纯模型入口，不重投影，窗口轨道列会
    // 停在换位后的顺序（无头实证）；产品撤销按钮走的就是 `invoke_undo_step` 这一条。
    let undone_before = port.display().undone;
    window.invoke_undo_step();
    assert_eq!(
        port.display().undone,
        undone_before + 1,
        "撤销必须真的退一步"
    );
    assert!(
        port.project().track_order.is_empty(),
        "撤销一次必须把 `track_order` 恢复为空（默认身份升序）"
    );
    assert_eq!(
        track_ids(&window),
        ids_before,
        "撤销一次必须把窗口轨道列完整恢复：{ids_before:?}"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "撤销不产生新提交"
    );
}
