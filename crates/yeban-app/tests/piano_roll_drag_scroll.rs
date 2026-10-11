//! `AGENTS.md` §3.6 双重验证：**已存在**的卷帘拖动滚动手势（`[ROAD-M3-002]`，账本第 199 轮）。
//!
//! ## 被验证的链路（本判据**不重做**手势，只补上「注入指针拖动 ⇒ 断言偏移」这一路驱动）
//!
//! | 环 | 位置 |
//! | :--- | :--- |
//! | 输入面 | `ui/console/piano_roll.slint` 的拖动 `TouchArea`（`dragging` / `drag-last-x`），**语义 ID** `piano-roll-scroll-area` |
//! | 报告增量 | 同文件：`root.scroll-requested(root.drag-last-x - self.mouse-x)`（本版本 `PointerEvent` **没有** `delta-x` ⇒ 用 `last − now`） |
//! | 上抛 | `console_tabs.slint` ⇒ `app.slint` |
//! | 宿主消费 | `src/host.rs` 的 `ui.on_scroll_requested(move | delta | …)`：**宿主拥有**偏移，`advance_scroll(current, delta) = (current + delta).max(0.0)` 之后经 `apply_view` 重新注入 `roll-scroll-x` |
//!
//! ⇒ 「**Slint 发增量、宿主拥有偏移**」。`piano_roll.slint` 的注释明记：`Flickable` 会与宿主偏移
//! **双重计算**，故不用。本判据就是那句话的**直接检验**：注入 `down → move(Σdx = ±40) → up`
//! ⇒ 偏移变化**恰好 ±40**，⛔ **绝不出现 ±80**。
//!
//! ## 读数从哪来（不是从零造）
//!
//! `roll-scroll-x` 是宿主拥有的 `in-out property <float>`（`ui/app.slint`），读数是
//! `window.get_roll_scroll_x()`。仓内已有**别的**驱动（缩放快捷键）断言它，但**注入指针拖动去驱动它
//! 的判据在本次之前一处都没有** ⇒ 本判据补的正是这一路。
//!
//! ## 方向口径（本判据**不猜**，并把矛盾如实登记）
//!
//! `.slint` 的注释写「向右拖 ⇒ 内容左移 ⇒ 偏移增加」，而宿主侧算术是
//! `delta = last − now` 加 `advance_scroll = current + delta` ⇒ 向右拖给出**负**增量。
//! 两者至少在字面上不一致。因此本判据**不断言「哪个方向是 +」**，只断言与符号无关的硬性质：
//! ① 一次拖动记的增量**恰好等于位移量**（⛔ 不是 2×）；② 两个方向都覆盖；
//! ③ `up` 之后继续 `move` ⇒ 偏移**一位不变**。实测到的符号由 `report_line` 打进日志。
//!
//! ## 寻址能力的如实登记（⛔ 不假装）
//!
//! `yeban-ui-test-port` 的端口树**按 `accessible-id` 寻址**（`inspect.rs` 只登记语义可寻址节点；
//! 无该 ID ⇒ `Ok(None)`；格式不合法 ⇒ `Err(MalformedId)`；重复 ID ⇒ `TreeError::DuplicateId`），
//! 因此 `ui/node` 的 `elementId` 就是**语义 ID 寻址**。上游 `i-slint-backend-testing` **没有**
//! `find_by_accessible_id`（只有 `find_by_element_id`，键是 `组件名::局部名`）⇒ 跨到上游控件句柄
//! 仍需 `accessible-id → element_id` 的映射；本判据**不走那条路**（登记为缺口，见交付报告）。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与无头比对**交 CI**，判决以 CI 的
//! `rust (yeban-app)` / `ui-mcp` 腿为准。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分（与 `live_ui_mcp.rs` 的全量用法不同）⇒ 其余项
// 在本测试目标里是 dead_code，而 `-D warnings` 会把它判红（本机 clippy 实测）。
#[allow(dead_code)]
mod live;

use live::{LiveControlPlane, build_live_ui};
use serde_json::json;
use yeban_ui_mcp::live::{ProbeOptions, ScreenshotProbe, request_line};
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 拖动面的**稳定语义 ID**（`[UI-TEST-001]` §12.2；与 `piano-roll-velocity-lane` 等同族）。
const SCROLL_AREA: &str = "piano-roll-scroll-area";

/// 装配一条真实控制面（与 `live_ui_mcp.rs` 的入口同款）。
fn assemble() -> (yeban_app::ui::MainWindow, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, plane)
}

/// 取某个**语义 ID** 在运行时控件树里的包围盒。
///
/// 端口里那份几何会**过期**（账本 @1437 的教训）⇒ 每次拖动前都重读一次。
fn element_bounds(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> serde_json::Value {
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
    bounds
}

/// 按**语义 ID** 按下，分 `steps` 步水平拖动合计 `dx`（正 = **指针向右**），然后松手。
fn drag_by_id(plane: &mut LiveControlPlane, seq: i64, element_id: &str, dx: f64, steps: usize) {
    assert!(steps >= 1, "至少要有一步移动，否则不是拖动");
    let bounds = element_bounds(plane, seq, element_id);
    let x_offset = bounds["width"].as_f64().expect("width") / 2.0;
    let y_offset = bounds["height"].as_f64().expect("height") / 2.0;
    let start_x = bounds["x"].as_f64().expect("x") + x_offset;
    let start_y = bounds["y"].as_f64().expect("y") + y_offset;

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
    assert!(
        !down.is_error(),
        "`{element_id}` 按下必须落到真实窗口: {down:?}"
    );

    let step = dx / steps as f64;
    for index in 0..steps {
        let x = start_x + step * (index + 1) as f64;
        let moved = plane.plane().try_line(&request_line(
            seq + 2 + i64::try_from(index).expect("步号"),
            METHOD_DISPATCH_POINTER_MOVE,
            Some(json!({ "x": x, "y": start_y })),
        ));
        assert!(
            !moved.is_error(),
            "`{element_id}` 的第 {} 步移动必须落到真实窗口: {moved:?}",
            index + 1
        );
    }

    let up = plane.plane().try_line(&request_line(
        seq + 100,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(
        !up.is_error(),
        "`{element_id}` 松手必须落到真实窗口: {up:?}"
    );
}

/// 一次拖动的偏移变化量（读数来自宿主拥有的 `roll-scroll-x`）。
fn drag_delta(
    window: &yeban_app::ui::MainWindow,
    plane: &mut LiveControlPlane,
    seq: i64,
    dx: f64,
) -> f32 {
    let before = window.get_roll_scroll_x();
    drag_by_id(plane, seq, SCROLL_AREA, dx, 4);
    window.get_roll_scroll_x() - before
}

/// 用**控制面自己的探针**抓一帧的像素证据（`[MUST-GATE-015]` / `[UI-MCP-003]`）。
///
/// ⚠ `capture()` 属于 **`LiveUi`**，⛔ 不属于 `LiveControlPlane`（CI 已判决过一次）；
/// 探针这条路是 `live_ui_mcp.rs` 里**已在用**的读数（指纹 ＋ 遮罩证据）。
fn probe_shot(plane: &mut LiveControlPlane) -> ScreenshotProbe {
    plane
        .plane()
        .probe(&ProbeOptions::new(SCROLL_AREA, "卷帘拖动手势面"))
        .expect("探针必须能读到拖动面并给出截图证据")
        .screenshot
}

/// 判据 ①（§3.6 第一条）：拖动面有**稳定语义 ID**，且它在**控件树 JSON** 里可见。
#[test]
fn the_scroll_area_has_a_stable_semantic_id_visible_in_the_tree() {
    let (window, mut plane) = assemble();

    // 默认底部标签必须是卷帘 —— 否则拖动面 `visible: false`，注入会如实地在树里找不到它。
    assert_eq!(
        window.get_console_tab(),
        0,
        "默认底部标签必须是卷帘（拖动面只在那时可见）"
    );

    let bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    report_line(&format!(
        "[roll-drag] 语义 ID `{SCROLL_AREA}` 在运行时树里的包围盒: {bounds}"
    ));

    // 注册表（无窗口也能工作的那份）里也必须登记 ⇒ 与 `.slint` 的 `accessible-id` 双侧对账。
    assert!(
        plane.registry().find_by_id(SCROLL_AREA).is_some(),
        "`{SCROLL_AREA}` 必须登记在**注册表**里（[UI-TEST-001]：否则两侧对账会漂移）"
    );
}

/// 判据 ②（§3.6 核心）：**不双重计数** ＋ **两个方向** ＋ **`up` 之后不再累加**。
#[test]
fn the_drag_reports_exactly_the_delta_sum_in_both_directions() {
    let (window, mut plane) = assemble();
    assert_eq!(window.get_console_tab(), 0, "默认底部标签是卷帘");
    assert_eq!(window.get_roll_scroll_x(), 0.0, "起点偏移必须是 0");

    // ---- ① 两个方向各 40px：每个方向记的增量只能是 **0（被 0 下限夹住）或 ±40** ----
    let left = drag_delta(&window, &mut plane, 10, -40.0);
    let right = drag_delta(&window, &mut plane, 20, 40.0);
    report_line(&format!(
        "[roll-drag] 指针向左 40px ⇒ Δ={left}；再向右 40px ⇒ Δ={right}（幅度只许是 0 或 40，⛔ 80 即双重计数）"
    ));
    for (label, delta) in [("向左", left), ("向右", right)] {
        let magnitude = delta.abs();
        assert!(
            magnitude < 0.5 || (magnitude - 40.0).abs() < 0.5,
            "{label} 拖 40px 的偏移变化只能是 0（被 0 下限夹住）或 ±40；实际 {delta} —— ⛔ ±80 表示 Slint 增量与宿主偏移被双重计数"
        );
    }
    assert!(
        left.abs() > 0.5 || right.abs() > 0.5,
        "两个方向至少有一个必须真的推动偏移（否则拖动这条路根本没接上）: 左 {left} / 右 {right}"
    );

    // ---- ② 反向必然抵掉：从非零偏移出发，反向拖同样的量 ⇒ 回到 0 ----
    // 先把偏移推到 40（用**刚刚证明确实能推动**的那个方向）。
    let push = if right.abs() > 0.5 { 40.0 } else { -40.0 };
    let pushed = window.get_roll_scroll_x();
    let back = drag_delta(&window, &mut plane, 30, -push);
    let now = window.get_roll_scroll_x();
    report_line(&format!(
        "[roll-drag] 反向抵消：偏移 {pushed} ⇒ {now}（Δ={back}）"
    ));
    assert!(
        back.abs() < 0.5 || (back.abs() - 40.0).abs() < 0.5,
        "反向抵消的幅度同样只许是 0 或 40；实际 {back}"
    );

    // ---- ③ 连续同向拖动：**每一步**都只记一次（⛔ 不是 2×）----
    let base = window.get_roll_scroll_x();
    let step1 = drag_delta(&window, &mut plane, 40, push);
    let step2 = drag_delta(&window, &mut plane, 50, push);
    report_line(&format!(
        "[roll-drag] 连续两次同向 40px：Δ1={step1} Δ2={step2}（起点 {base}）"
    ));
    for (label, step) in [("第一次", step1), ("第二次", step2)] {
        let magnitude = step.abs();
        assert!(
            magnitude < 0.5 || (magnitude - 40.0).abs() < 0.5,
            "{label} 同向拖动只许记一次 40（或 0）；实际 {step}"
        );
    }

    // ---- ④ `up` 之后继续 `move`：偏移**一位不变**（`dragging` 已复位）----
    let before_stray = window.get_roll_scroll_x();
    let moved_after_up = plane.plane().try_line(&request_line(
        60,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({ "x": 5.0, "y": 5.0 })),
    ));
    assert!(
        !moved_after_up.is_error(),
        "`up` 之后的移动注入本身必须成功（否则测的不是「无拖动」这一路）: {moved_after_up:?}"
    );
    let after_stray = window.get_roll_scroll_x();
    report_line(&format!(
        "[roll-drag] `up` 之后再来一次 move：偏移 {before_stray} ⇒ {after_stray}（必须一位不变）"
    ));
    assert!(
        (after_stray - before_stray).abs() < 0.001,
        "`up` 之后继续 `move` **不许**改变偏移（`dragging` 已复位）：{before_stray} ⇒ {after_stray}"
    );
}

/// 判据 ③（`[UI-MCP-003]`）：**遮罩动态区域**后的像素证据 —— 同一状态**稳定**（指纹一致），偏移改变后**必须变**。
///
/// ⚠ 截图入口用**控制面自己的探针**（`ControlPlane::probe` 的 `screenshot` 证据链：指纹 / 遮罩字段）。
/// `capture()` 属于 **`LiveUi`**（`live_surface.rs` 的 `impl LiveUi`，1282–1355），
/// ⛔ **不属于** `LiveControlPlane`（impl 起于 1556）—— 本判据第一版正是在这里编译失败（CI 已判决）。
#[test]
fn the_roll_pixels_follow_the_offset_after_masking_dynamic_regions() {
    let (window, mut plane) = assemble();
    assert_eq!(window.get_console_tab(), 0, "默认底部标签是卷帘");

    // 同一状态抓两次：指纹必须**逐字节一致**（渲染确定），且遮罩确实生效。
    let first = probe_shot(&mut plane);
    let again = probe_shot(&mut plane);
    report_line(&format!(
        "[roll-drag] 同一状态两帧: 指纹 {} / {}；遮罩 {} 区 effective={}；非黑 {}；颜色 {} 种",
        first.fingerprint,
        again.fingerprint,
        first.masked_regions,
        first.mask_effective,
        first.non_black_pixels,
        first.distinct_colors
    ));
    assert_eq!(
        first.fingerprint, again.fingerprint,
        "同一状态的两次抓帧必须**逐字节一致**（渲染确定）"
    );
    assert!(
        first.mask_dynamic && first.mask_effective && first.masked_regions > 0,
        "[UI-MCP-002]: 动态区域必须被请求遮罩、**确实生效**且遮罩区数非 0（实际 dynamic={} effective={} regions={}）",
        first.mask_dynamic,
        first.mask_effective,
        first.masked_regions
    );

    // 滚一段：两个方向各试一次，取**确实推动了偏移**的那一次（口径与判据 ② 相同，不猜符号）。
    let before = window.get_roll_scroll_x();
    let left = drag_delta(&window, &mut plane, 80, -120.0);
    if left.abs() < 0.5 {
        let right = drag_delta(&window, &mut plane, 90, 120.0);
        report_line(&format!(
            "[roll-drag] 视觉前置：向左 120px 被夹住（Δ={left}），改用向右 120px（Δ={right}）"
        ));
    }
    let after = window.get_roll_scroll_x();
    assert!(
        (after - before).abs() >= 40.0,
        "本判据的视觉前置：偏移必须先真的滚起来（|Δ| ≥ 40）；实际 {before} ⇒ {after}"
    );
    let scrolled = probe_shot(&mut plane);
    report_line(&format!(
        "[roll-drag] 偏移 {before} ⇒ {after}: 指纹 {} ⇒ {}（必须不同，才算「真的滚了」）",
        first.fingerprint, scrolled.fingerprint
    ));
    assert_ne!(
        first.fingerprint, scrolled.fingerprint,
        "滚动之后像素指纹必须改变（否则滚动没有落到画面上）"
    );
}
