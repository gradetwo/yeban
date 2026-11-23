//! **编曲区空白拖拽 = 框选**（line/blank-drag-selection）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! Wang 2026-11-18 裁决（照 Logic Pro）：编曲区空白处拖拽 = 橡皮筋框选片段，
//! ⛔ 不做拖拽滚动（2026-11-07 的「空车道拖动横滚」已废）；滚动只走滚轮 /
//! Shift+滚轮 / 滚动条。本文件钉住新语义的四件事：
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（命中几何） | 纯函数 `host::clip_marquee_hits`：矩形与片段**绘制几何**严格重叠才命中；边接触不算；任意拖拽方向同果；全图矩形按视图序返回全部 |
//! | ②（真实拖拽） | 活窗口里在空白车道按下 → 拖过片段 → 松手：`selected-clip-ulids` == 命中集、主选中 == 命中首元、与块同序的标志逐项对齐 |
//! | ③（单击不变） | 空白处单击（按下即松手、未越 4px 阈值）**不**改剪辑选区 —— 原来的单击选轨语义保留，框选不许吞掉它 |
//! | ④（加选与清空） | Shift 框选（回调层）= 并入现有选区且主选中保住；无修饰框选空命中 = 清空选区 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 建窗口的判据只有一条
//! （②③④ 共用一个活窗口，与 `tests/arrangement_ruler_seek.rs` 同款）；
//! ① 是纯函数判据，不建窗口。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与无头比对交 CI / 盒上验证。

#![allow(unused)]
use std::cell::RefCell;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{ClipView, ViewState, demo_project};
use yeban_app::host::{self, clip_marquee_hits};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{EntityId, TrackKind, TrackV3};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;
/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);
/// 编曲视口的滚动面（`elements.rs` 已注册）。
const FLICKABLE: &str = "arr-lanes-flickable";
// 车道区左缘在 Flickable 局部帧的偏移 = `headers-width` + 5px 间隔
// （`ui/arrangement.slint` 的 `timeline-origin-x`，表头在左时）。
// ⚠ `headers-width` 在实窗里绑的是根属性 `track-headers-width`（240px，
// ⛔ 不是 ArrangementViewport 的组件默认值 216px）⇒ 判据从窗口现读，见
// 各自主流程里的 `origin_x`。

/// 片段的**绘制几何**（与 `host::clip_marquee_hits` 的文档逐项同源 —— 这里是
/// 判据侧的独立复算：x / 宽的钳位、y 的 +4、高 的 max(52, 行高 − 8)）。
fn block_rect(view: &ViewState, clip: &ClipView) -> (f32, f32, f32, f32) {
    let ys = view.track_ys();
    let heights = view.track_heights();
    (
        clip.x.max(0.0),
        ys[clip.track_index] + 4.0,
        clip.width.max(12.0),
        (heights[clip.track_index] - 8.0).max(52.0),
    )
}

// ---------------------------------------------------------------------------
// ① 命中几何（纯函数）
// ---------------------------------------------------------------------------

#[test]
fn marquee_hits_cover_exact_block() {
    let view = ViewState::from_project(&demo_project()).expect("演示工程必须能投影");
    let clip = view.clips.first().expect("演示工程必须有片段");
    let (x, y, w, h) = block_rect(&view, clip);
    // 矩形比块内缩 2px ⇒ 必然严格重叠且只可能命中这一条（块互不重叠）。
    let hits = clip_marquee_hits(&view, x + 2.0, y + 2.0, x + w - 2.0, y + h - 2.0);
    assert_eq!(
        hits,
        vec![clip.placement_id.clone()],
        "内缩矩形必须恰好命中第一条片段"
    );
}

#[test]
fn marquee_hits_full_sheet_returns_all_in_view_order() {
    let view = ViewState::from_project(&demo_project()).expect("演示工程必须能投影");
    let hits = clip_marquee_hits(&view, -100.0, -100.0, 100_000.0, 100_000.0);
    let expected: Vec<String> = view.clips.iter().map(|c| c.placement_id.clone()).collect();
    assert_eq!(hits, expected, "全图矩形必须按视图序返回全部片段");
}

#[test]
fn marquee_hits_empty_area_and_edge_touch_miss() {
    let view = ViewState::from_project(&demo_project()).expect("演示工程必须能投影");
    // 远离一切片段的矩形 ⇒ 空。
    let hits = clip_marquee_hits(&view, 100_000.0, 0.0, 100_050.0, 50.0);
    assert!(hits.is_empty(), "空区域不得命中任何片段：{hits:?}");
    // 与第一条片段的块**边接触**（矩形左缘 == 块右缘）⇒ 不算命中。
    let clip = view.clips.first().expect("演示工程必须有片段");
    let (x, y, w, h) = block_rect(&view, clip);
    let hits = clip_marquee_hits(&view, x + w, y, x + w + 40.0, y + h);
    assert!(
        !hits.contains(&clip.placement_id),
        "边接触不得命中（严格重叠口径）：{hits:?}"
    );
}

#[test]
fn marquee_hits_reversed_corners_match() {
    let view = ViewState::from_project(&demo_project()).expect("演示工程必须能投影");
    let clip = view.clips.first().expect("演示工程必须有片段");
    let (x, y, w, h) = block_rect(&view, clip);
    let forward = clip_marquee_hits(&view, x - 10.0, y - 2.0, x + w + 10.0, y + h + 2.0);
    let reversed = clip_marquee_hits(&view, x + w + 10.0, y + h + 2.0, x - 10.0, y - 2.0);
    assert_eq!(forward, reversed, "任意拖拽方向必须同果");
    assert!(
        forward.contains(&clip.placement_id),
        "包住块的矩形必须命中该片段"
    );
}

// ---------------------------------------------------------------------------
// ②③④ 活窗口：真实拖拽 / 单击 / 回调层加选与清空
// ---------------------------------------------------------------------------

/// 在视图里找一处**空白按下点**：某条轨道行内、可见宽内、任何片段块之外
/// 的一段空档中点（返回内容帧坐标：x = 时间轴像素、y = 行几何帧）。
fn blank_start(view: &ViewState, visible_tx_max: f32) -> (usize, f32, f32) {
    let ys = view.track_ys();
    let heights = view.track_heights();
    for track in 0..view.tracks.len() {
        let mut intervals: Vec<(f32, f32)> = view
            .clips
            .iter()
            .filter(|c| c.track_index == track)
            .map(|c| {
                let (x, _, w, _) = block_rect(view, c);
                (x, x + w)
            })
            .collect();
        intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("有限坐标"));
        let mut cursor = 8.0_f32;
        for (start, end) in intervals {
            if start - cursor >= 40.0 && cursor + 20.0 < visible_tx_max {
                return (track, cursor + 20.0, ys[track] + heights[track] / 2.0);
            }
            cursor = cursor.max(end);
        }
        if visible_tx_max - cursor >= 40.0 && cursor + 20.0 < visible_tx_max {
            return (track, cursor + 20.0, ys[track] + heights[track] / 2.0);
        }
    }
    panic!("演示工程在可见宽内必须有一处空白按下点");
}

#[test]
fn blank_drag_selects_clips_and_click_keeps_selection() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 主窗口");
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<clip-marquee>", "yeban-app", project.clone(), NOW_MS).expect("打开"),
    ));
    let live_view = Rc::new(RefCell::new(view.clone()));
    host::wire_clip_scope(port.ui(), Rc::clone(&live_view), Some(Rc::clone(&undo)));
    port.refresh_tree(None).expect("必须能抓控件树");
    // ⚠ 不把 `port.ui()` 的借用跨在 `dispatch_*`（&mut self）之间持有：
    // 每次读界面状态都现取（与 `arrangement_ruler_seek` 的 Rig 同款）。

    let bounds = port
        .tree()
        .find_by_id(FLICKABLE)
        .expect("控件树里必须有编曲滚动面")
        .bounds
        .expect("滚动面必须有几何");
    let origin_x = f64::from(port.ui().get_track_headers_width()) + 5.0;
    let visible_tx_max = (bounds.width as f32) - (origin_x as f32) - 24.0;

    // 按下点：空白车道；目标：另一条轨道上、可见宽内的一条片段。
    let (start_track, sx, sy) = blank_start(&view, visible_tx_max);
    let target = view
        .clips
        .iter()
        .find(|c| {
            let (x, _, w, _) = block_rect(&view, c);
            c.track_index != start_track && x + w / 2.0 < visible_tx_max
        })
        .expect("演示工程必须有异轨可见片段可作框选目标");
    let (tx0, ty0, tw, th) = block_rect(&view, target);
    let (ex, ey) = (tx0 + tw / 2.0, ty0 + th / 2.0);

    // ② 真实拖拽：按下（Flickable 局部坐标）→ 三步移动（窗口绝对坐标）→ 松手。
    port.dispatch_pointer_down(
        FLICKABLE,
        origin_x + f64::from(sx),
        f64::from(sy),
        PointerButton::Left,
    )
    .expect("空白车道必须可按下");
    for step in 1..=3 {
        let t = f64::from(step) / 3.0;
        port.dispatch_pointer_move(
            f64::from(bounds.x) + origin_x + f64::from(sx) + (f64::from(ex - sx)) * t,
            f64::from(bounds.y) + f64::from(sy) + (f64::from(ey - sy)) * t,
        )
        .expect("拖动必须送达");
    }
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");

    let expected = clip_marquee_hits(&view, sx, sy, ex, ey);
    assert!(
        expected.contains(&target.placement_id),
        "框选矩形必须命中目标片段（判据自身的几何复算）：{expected:?}"
    );
    let selected: Vec<String> = port
        .ui()
        .get_selected_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(selected, expected, "松手后选区集必须 == 命中集");
    assert_eq!(
        port.ui().get_selected_clip_ulid().as_str(),
        expected.first().map_or("", String::as_str),
        "主选中必须 == 命中首元"
    );
    // 与块同序的标志逐项对齐（错位 = 红）。
    let blocks: Vec<String> = port
        .ui()
        .get_timeline_blocks()
        .iter()
        .map(|b| b.clip_ulid.to_string())
        .collect();
    let flags: Vec<bool> = port.ui().get_timeline_block_selected().iter().collect();
    assert_eq!(flags.len(), blocks.len(), "标志必须与块同长");
    for (block, flag) in blocks.iter().zip(flags.iter()) {
        assert_eq!(
            *flag,
            expected.contains(block),
            "块 {block} 的选中标志必须与选区集一致"
        );
    }

    // ③ 空白单击：选区一位不动。
    port.dispatch_pointer_down(
        FLICKABLE,
        origin_x + f64::from(sx),
        f64::from(sy),
        PointerButton::Left,
    )
    .expect("空白车道必须可按下");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");
    let after_click: Vec<String> = port
        .ui()
        .get_selected_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(after_click, expected, "空白单击不得改动剪辑选区");

    // ④ 加选（回调层，Shift 语义）：找一条不在当前选区里的片段，框住它。
    let other = view
        .clips
        .iter()
        .find(|c| !expected.contains(&c.placement_id))
        .expect("演示工程必须有选区外的片段");
    let (ox, oy, ow, oh) = block_rect(&view, other);
    port.ui()
        .invoke_clip_marquee_select(ox + 2.0, oy + 2.0, ox + ow - 2.0, oy + oh - 2.0, true);
    let after_add: Vec<String> = port
        .ui()
        .get_selected_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert!(
        !expected.is_empty()
            && expected.len() == after_add.len() - 1
            && after_add.contains(&other.placement_id)
            && expected.iter().all(|id| after_add.contains(id)),
        "加选必须并入现有选区：{after_add:?}"
    );
    assert_eq!(
        port.ui().get_selected_clip_ulid().as_str(),
        expected.first().map_or("", String::as_str),
        "加选时主选中必须保住原值"
    );

    // ④ 清空：无修饰框选落在空区域 ⇒ 选区清空、主选中为空串。
    port.ui()
        .invoke_clip_marquee_select(100_000.0, 0.0, 100_050.0, 50.0, false);
    assert_eq!(
        port.ui().get_selected_clip_ulids().iter().count(),
        0,
        "空命中框选必须清空选区"
    );
    assert_eq!(
        port.ui().get_selected_clip_ulid().as_str(),
        "",
        "主选中必须随之为空"
    );
}

/// 框选在**内容高出视口**时仍成立（回归 ISS-MWTNA3DX）。
///
/// 演示工程 6 轨内容 424px；b5e1b00d 加 Songmap A/B 条后视口 406px ⇒ 刚好溢出，
/// Flickable 把纵向拖拽认领为滚动、给车道 TouchArea 发 `cancel` ⇒ 框选永远到不了
/// 宿主。本判据用 12 轨工程（内容 760px）钉住：无论视口多高、内容溢出多少，
/// 空白拖拽都必须框选——与主判据同款拖拽，只换工程。
#[test]
fn blank_drag_selects_clips_when_content_overflows_viewport() {
    let mut project = demo_project();
    for index in 0..6 {
        let id = EntityId::new();
        project.tracks.insert(
            id,
            TrackV3 {
                id,
                name: format!("高轨 {index}"),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            },
        );
    }
    let view = ViewState::from_project(&project).expect("十二轨工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 主窗口");
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<clip-marquee-tall>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开"),
    ));
    let live_view = Rc::new(RefCell::new(view.clone()));
    host::wire_clip_scope(port.ui(), Rc::clone(&live_view), Some(Rc::clone(&undo)));
    port.refresh_tree(None).expect("必须能抓控件树");

    let bounds = port
        .tree()
        .find_by_id(FLICKABLE)
        .expect("控件树里必须有编曲滚动面")
        .bounds
        .expect("滚动面必须有几何");
    let origin_x = f64::from(port.ui().get_track_headers_width()) + 5.0;
    let visible_tx_max = (bounds.width as f32) - (origin_x as f32) - 24.0;
    // 内容必溢出：12 轨 × 56px + 88px 垫高 ≫ 视口。
    assert!(
        view.track_sheet_height() + 88.0 > bounds.height as f32,
        "十二轨内容必须高出视口（否则本判据钉不住溢出态）：内容 {} vs 视口 {}",
        view.track_sheet_height() + 88.0,
        bounds.height
    );

    let (start_track, sx, sy) = blank_start(&view, visible_tx_max);
    let target = view
        .clips
        .iter()
        .find(|c| {
            let (x, _, w, _) = block_rect(&view, c);
            c.track_index != start_track && x + w / 2.0 < visible_tx_max
        })
        .expect("演示工程必须有异轨可见片段可作框选目标");
    let (tx0, ty0, tw, th) = block_rect(&view, target);
    let (ex, ey) = (tx0 + tw / 2.0, ty0 + th / 2.0);

    port.dispatch_pointer_down(
        FLICKABLE,
        origin_x + f64::from(sx),
        f64::from(sy),
        PointerButton::Left,
    )
    .expect("空白车道必须可按下");
    for step in 1..=3 {
        let t = f64::from(step) / 3.0;
        port.dispatch_pointer_move(
            f64::from(bounds.x) + origin_x + f64::from(sx) + (f64::from(ex - sx)) * t,
            f64::from(bounds.y) + f64::from(sy) + (f64::from(ey - sy)) * t,
        )
        .expect("拖动必须送达");
    }
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");

    let expected = clip_marquee_hits(&view, sx, sy, ex, ey);
    assert!(
        expected.contains(&target.placement_id),
        "框选矩形必须命中目标片段（判据自身的几何复算）：{expected:?}"
    );
    let selected: Vec<String> = port
        .ui()
        .get_selected_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(selected, expected, "内容溢出时松手后选区集必须 == 命中集");
}
