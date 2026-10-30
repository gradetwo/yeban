//! **编曲剪辑平移**（`clip-{ulid}-body` 拖动面）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! `Op::MoveClipPlacement` 在模型与 MCP 两侧早已存在，而界面侧在此切片之前**一处都没有**
//! 使用它（`grep -rn MoveClipPlacement crates/yeban-app/src` 为空）⇒ 用户拖不动任何剪辑。
//! 本切片把拖动接成"拖动期只写草稿、松手恰好提交一次"的手势：
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（拖动期不提交） | 按下 → 4 步移动期间 `clip-drag-ulid` / `clip-drag-preview-dx` 是草稿读数，**工程起点 tick 与提交数一位不动** |
//! | ②（输入面不动） | 拖动中 `clip-{ulid}-body` 的包围盒与按下前逐值相同 ⇒ `mouse-x` 的坐标系不随画面移动（否则位移读数自我抵消） |
//! | ③（画面只在剪辑的并集内变） | 拖动帧与按下帧的差异像素**全部**落在「原位置 ∪ 预览位置」内 |
//! | ④（松手恰好一次提交） | `up` 之后提交数 **+1**、工程起点 == 吸附后的目标（默认缩放 30 ⇒ 40px = 1200 tick）、草稿清零 |
//! | ⑤（`Escape` 零提交） | 第二次拖动中途 `Escape` ⇒ 提交数不变、工程逐字节不变、帧与提交后的帧逐像素相同 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的且每线程只能装配一次 ⇒ 本文件只有一个
//! `#[test]`（与 `tests/clip_selection.rs` 同款）。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与无头比对交 CI。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// `-D warnings` 会判红（与 `tests/roll_edit_tools.rs` 同款）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveControlPlane, build_live_ui};
use serde_json::json;
use slint::Model as _;
use yeban_app::host::{ROLL_SNAP_GRID_TICKS, roll_ticks_per_pixel, wire_clip_drag};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_KEY_PRESS, METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE,
    METHOD_DISPATCH_POINTER_UP, METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::{Rect, Rgb8Image};

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 一次拖动注入的**水平位移**（窗口逻辑像素，向右为正）。
///
/// 40px × 默认缩放 30 = 1200 tick = **5 个**吸附格（240）⇒ 期望值可以手算，
/// 不依赖被测的换算函数（否则判据只是把被测逻辑抄一遍）。
const DRAG_DX_PX: f64 = 40.0;

/// 拖动分几步注入（每步 10px ⇒ 第一步就越过规范的 4px 阈值）。
const DRAG_STEPS: usize = 4;

/// 默认横向缩放（`bridge::DEFAULT_TICKS_PER_PIXEL`）—— 判据按它算期望值。
const DEFAULT_TPP: u64 = 30;

/// 吸附网格（`ROLL_SNAP_GRID_TICKS`）的格数换算：40px × 30 = 1200 tick。
const EXPECTED_DELTA_TICKS: u64 = 1200;

/// 装配：真窗口 ＋ 撤销端口 ＋ **与产品同一条** `wire_clip_drag`。
fn assemble() -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<clip-drag>", "yeban-app", project, NOW_MS).expect("打开"),
    ));
    wire_clip_drag(ui.ui(), &port);
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

/// 按语义 ID 在包围盒中心按下；返回拖动面的绝对窗口原点（后续 `move` 用绝对坐标）。
fn pointer_down_at_center(
    plane: &mut LiveControlPlane,
    seq: i64,
    element_id: &str,
    bounds: (i32, i32, u32, u32),
) -> (f64, f64) {
    let (x, y, width, height) = bounds;
    let x_offset = f64::from(width) / 2.0;
    let y_offset = f64::from(height) / 2.0;
    let down = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    (f64::from(x) + x_offset, f64::from(y) + y_offset)
}

/// 从按下点分 `DRAG_STEPS` 步向右拖 `DRAG_DX_PX`（**绝对窗口坐标**注入）。
fn pointer_drag_right(plane: &mut LiveControlPlane, seq: i64, origin: (f64, f64)) {
    for step in 1..=DRAG_STEPS {
        #[allow(clippy::cast_precision_loss)]
        let fraction = step as f64 / DRAG_STEPS as f64;
        let x = origin.0 + DRAG_DX_PX * fraction;
        let moved = plane.plane().try_line(&request_line(
            seq + i64::try_from(step).expect("步号"),
            METHOD_DISPATCH_POINTER_MOVE,
            Some(json!({ "x": x, "y": origin.1 })),
        ));
        assert!(
            !moved.is_error(),
            "第 {step} 步移动必须落到真实窗口: {moved:?}"
        );
    }
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

/// 权威工程里某条摆放的起点 tick。
fn clip_start_tick(project: &yeban_model::YebanProjectV1, ulid: &str) -> Option<u64> {
    let id = ulid.parse::<yeban_model::EntityId>().ok()?;
    project
        .tracks
        .values()
        .find_map(|track| track.clips.get(&id).map(|placement| placement.start_tick))
}

/// 两帧在 `area` **之外**的差异像素数。`0` = "期望区域外逐字节相同"。
fn diff_outside(before: &Rgb8Image, after: &Rgb8Image, area: Rect) -> u64 {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸"
    );
    let mut outside = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            let inside = (x as i32) >= area.x
                && (x as i32) < area.right()
                && (y as i32) >= area.y
                && (y as i32) < area.bottom();
            if !inside && before.pixel(x, y) != after.pixel(x, y) {
                outside += 1;
            }
        }
    }
    outside
}

/// 两帧的全帧差异像素数。
fn diff_total(before: &Rgb8Image, after: &Rgb8Image) -> u64 {
    diff_outside(
        before,
        after,
        Rect::new(0, 0, before.width(), before.height()),
    )
}

/// 把包围盒扩成剪辑**整块**（包头 18px + 剪辑体），再按 `dx` 向右扩张出预览区域
/// （再留 2px 余量吸收边框 / 抗锯齿的取整）。
fn union_rect((x, y, width, height): (i32, i32, u32, u32), dx: i32) -> Rect {
    let full_x = x - 2;
    let full_y = y - 18 - 2;
    let full_w = width + 4;
    let full_h = height + 18 + 4;
    Rect::new(full_x, full_y, full_w + dx.unsigned_abs(), full_h)
}

#[test]
fn dragging_a_clip_body_moves_it_with_exactly_one_snapped_op() {
    let (window, port, mut plane) = assemble();
    assert_eq!(
        roll_ticks_per_pixel(&window),
        DEFAULT_TPP,
        "判据按默认缩放 {DEFAULT_TPP} 算期望值（没人改过缩放）"
    );

    let clip_ids: Vec<String> = window
        .get_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert!(!clip_ids.is_empty(), "演示工程必须至少有一条剪辑");
    let project_before = port.project();
    // 选一条**起点落在吸附网格上**的剪辑：这样 +1200 tick 之后仍是网格倍数，
    // 期望值可以手算（`old + EXPECTED_DELTA_TICKS`），不必复刻吸附实现。
    let (ulid, old_start) = clip_ids
        .iter()
        .find_map(|id| {
            let start = clip_start_tick(&project_before, id)?;
            (start % ROLL_SNAP_GRID_TICKS == 0).then(|| (id.clone(), start))
        })
        .expect("演示工程至少要有一条起点落在吸附网格上的剪辑");

    let body_id = format!("clip-{ulid}-body");
    let body_before = element_bounds(&mut plane, 10, &body_id);
    assert!(
        body_before.2 > 0 && body_before.3 > 0,
        "拖动面必须有可见几何: {body_before:?}"
    );
    let baseline = plane.capture().expect("Tier-1 截图");
    let commits_before = port.display().commit_count;

    // ---- 诊断探针（不是判据）：绕开指针链，宿主直接写草稿属性 ⇒ 画面是否跟着动 ----
    //
    // 这条读数是 2026-10-30 加的：③ 曾在 mac / windows 两腿报"0 个差异像素"，
    // 而 ① 证明属性确实是 40.0。探针把"属性 ⇒ 像素"与"指针 ⇒ 属性"两段分开。
    // 它不改任何被断言的状态（探针末尾清零、并与 `baseline` 逐像素复核复原）。
    {
        let before_probe = plane.capture().expect("Tier-1 截图");
        window.set_clip_drag_ulid(ulid.clone().into());
        window.set_clip_drag_preview_dx(40.0);
        let after_probe = plane.capture().expect("Tier-1 截图");
        report_line(&format!(
            "[clip-drag] 诊断：直接写 `clip-drag-ulid` + `clip-drag-preview-dx`=40 ⇒ 差异像素 {}（0 = 绑定/重绘没动）",
            diff_total(&before_probe, &after_probe)
        ));
        window.set_clip_drag_ulid("".into());
        window.set_clip_drag_preview_dx(0.0);
        let restored = plane.capture().expect("Tier-1 截图");
        report_line(&format!(
            "[clip-drag] 诊断：清零草稿后与基准差异 {}（0 = 复原）",
            diff_total(&baseline, &restored)
        ));
    }

    let header_before = element_bounds(&mut plane, 11, &format!("clip-{ulid}-header"));

    // ---- ① + ②：拖动期只有草稿，工程与输入面都不动 ----
    let origin = pointer_down_at_center(&mut plane, 20, &body_id, body_before);
    pointer_drag_right(&mut plane, 30, origin);
    assert_eq!(
        window.get_clip_drag_ulid().as_str(),
        ulid.as_str(),
        "按下必须把身份写进 `clip-drag-ulid`"
    );
    assert!(
        (window.get_clip_drag_preview_dx() - 40.0).abs() < 1e-3,
        "预览位移必须是 40px（1200 tick ÷ 30），实测 {}",
        window.get_clip_drag_preview_dx()
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "拖动期**不得**提交任何 op"
    );
    assert_eq!(
        clip_start_tick(&port.project(), &ulid),
        Some(old_start),
        "拖动期工程起点**不得**变化"
    );
    assert_eq!(
        element_bounds(&mut plane, 60, &body_id),
        body_before,
        "输入面的包围盒在拖动中不得移动（`mouse-x` 的坐标系必须稳定）"
    );

    // ---- ③：画面差异全部落在「原位置 ∪ 预览位置」内 ----
    // 诊断读数（与 ③ 同一次抓帧）：拖动中的草稿属性 + 包头（它随所属剪辑移动，
    // 因此它的 `ui/node` 几何是"布局是否真的动了"的直接读数）。
    let header_during = element_bounds(&mut plane, 61, &format!("clip-{ulid}-header"));
    report_line(&format!(
        "[clip-drag] 诊断：拖动中 ulid={:?} dx={} body={body_before:?} \
         header 前={header_before:?} 中={header_during:?}",
        window.get_clip_drag_ulid().as_str(),
        window.get_clip_drag_preview_dx(),
    ));
    let during = plane.capture().expect("Tier-1 截图");
    let union = union_rect(
        body_before,
        i32::try_from(EXPECTED_DELTA_TICKS / DEFAULT_TPP).expect("dx"),
    );
    let inside = {
        let total = diff_total(&baseline, &during);
        let outside = diff_outside(&baseline, &during, union);
        assert_eq!(
            outside, 0,
            "拖动期的差异像素必须全部落在剪辑的并集区域 {union:?} 内（实测 {outside} 个越界；\
             总差异 {total}，body={body_before:?}，header 前={header_before:?} 中={header_during:?}）"
        );
        total - outside
    };
    assert!(
        inside > 0,
        "预览位移必须**真的画出来**（实测 0 个差异像素；body={body_before:?}，\
         header 前={header_before:?} 中={header_during:?}，\
         `clip-drag-ulid`={:?}，`clip-drag-preview-dx`={}）",
        window.get_clip_drag_ulid().as_str(),
        window.get_clip_drag_preview_dx(),
    );

    // ---- ④：松手恰好一次提交，落在吸附后的目标 ----
    pointer_up(&mut plane, 100);
    assert_eq!(
        window.get_clip_drag_ulid().as_str(),
        "",
        "松手后草稿身份必须清零"
    );
    assert_eq!(
        window.get_clip_drag_preview_dx(),
        0.0,
        "松手后草稿位移必须清零（位置由重投影给出）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次拖动 = **恰好一步**可撤销的编辑"
    );
    let expected = old_start + EXPECTED_DELTA_TICKS;
    assert_eq!(
        clip_start_tick(&port.project(), &ulid),
        Some(expected),
        "起点必须落在 {old_start} + {EXPECTED_DELTA_TICKS}（= {expected}）"
    );
    let committed = plane.capture().expect("Tier-1 截图");
    assert_eq!(
        diff_outside(&baseline, &committed, union),
        0,
        "提交后的画面差异也必须落在同一块并集区域内"
    );

    // ---- ⑤：第二次拖动中途 `Escape` ⇒ 零提交、画面回到提交后的那一帧 ----
    let body_after = element_bounds(&mut plane, 110, &body_id);
    assert_ne!(
        body_after, body_before,
        "提交后剪辑必须真的挪到了新位置（拖动面的包围盒应当变了）"
    );
    let commits_after_commit = port.display().commit_count;
    let origin = pointer_down_at_center(&mut plane, 120, &body_id, body_after);
    pointer_drag_right(&mut plane, 130, origin);
    assert!(
        (window.get_clip_drag_preview_dx() - 40.0).abs() < 1e-3,
        "第二次拖动的草稿位移必须生效（否则下面的取消判据空转）"
    );
    press_escape(&mut plane, 200);
    pointer_up(&mut plane, 210);
    assert_eq!(
        window.get_clip_drag_ulid().as_str(),
        "",
        "`Escape` 必须清掉草稿身份"
    );
    assert_eq!(
        port.display().commit_count,
        commits_after_commit,
        "`Escape` 取消不得提交任何 op"
    );
    assert_eq!(
        clip_start_tick(&port.project(), &ulid),
        Some(expected),
        "`Escape` 取消后工程起点必须一位不变"
    );
    let after_escape = plane.capture().expect("Tier-1 截图");
    assert_eq!(
        diff_total(&committed, &after_escape),
        0,
        "`Escape` 取消后画面必须与提交后的那一帧**逐像素相同**"
    );

    report_line(&format!(
        "[clip-drag] 剪辑 `{ulid}` 起点 {old_start} → {expected}（+{EXPECTED_DELTA_TICKS} tick）；\
         一次拖动恰好 1 次提交；第二次拖动 `Escape` 取消后提交数不变、帧与提交后逐像素相同"
    ));
}
