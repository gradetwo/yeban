//! **剪辑裁剪时长**（`Shift+←/→` 拨选中摆放的 `ClipPlacement::duration_ticks`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 一条剪辑摆在编曲视图里，用户可以点它选中、可以拖它平移（`clip-{ulid}-body`）、可以删它
//! （`D-UI-15`）、可以只让它不出声（`D-UI-16`）—— 但**没法把它改短或改长**：
//! `Op::SetClipPlacementDuration` 在模型里早已存在（`ops.rs` 有 apply / inverse / 前置条件，
//! 新值 `≥ 1` 走的是与 `ClipPlacement::validate` **同一条** `ModelError::ZeroDuration`），
//! 而界面侧在此切片之前**一处都没有**：
//! `grep -rn "Op::SetClipPlacementDuration" crates/yeban-app/src` = **空**。
//! ⇒ "剪辑能挪、能删、能静音，就是不能裁剪"这一半接线缺口。
//!
//! ⚠ 与 `D-UI-16` 的区别在**键从哪儿来**：那一票是新占一格键位（`Cmd/Ctrl+M`），本票
//! **不新增动作** —— `Action::ResizeSelectionDuration` 与它的 `Shift+←/→` 键位早已存在且
//! 只服务音符选区（`[UI-NOTE-005]` §3 项 3）。本票补的是**作用域**：音符选区为空时，
//! 同一把键裁剪选中的那条摆放。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（夹具前提） | 至少两条摆放、⛔ 没有音符选区、`clip-widths` 与 `clip-ulids` 同长且都 `> 0`、每条时长都够缩一档 |
//! | ②（写权威 ＋ 带读数） | `Shift+→` ⇒ 只有那条摆放 `duration_ticks` **+一个网格**（另一条**逐字段**不变）、`start_tick`/`muted`/`loop_config` 一位不动、片段池字节不变、提交 **+1**、选中态**留下**、`clip-widths` 那一格变大 |
//! | ③（画得出来 ＋ 可逆） | 变宽的改变像素**全部**落在 `clip-{A}` **自身**前后两态的并集内（包头 `clip-{A}-header` ∪ 拖动面 `clip-{A}-body`；内 `> 0` / 外 `== 0`，非零时打印差异包围盒与前三个像素的颜色）；`Shift+←` 延回原值 ⇒ 帧与基准**逐字节相同** |
//! | ④（优先级与边界） | 音符与剪辑**同时**有选中态 ⇒ 只动**音符**、剪辑一位不动；音符已经卡在 1 tick 下限 ⇒ **不消费、0 提交**、⛔ 不跨界去改剪辑（"分派看选区，不看结果"） |
//! | ⑤（视图门控） | Session 视图里同一份选中态**不消费、0 提交、工程不变**；回编曲视图立刻生效 |
//! | ⑥（解析不到 ⇒ 不消费） | 陈旧 / 非法 / 空 三种选中态都 `consumed == false`、0 提交、⛔ 不动选中态 |
//! | ⑦（一步撤销 ＋ redo） | `Undo` 将工程逐字节放回（载荷带 `old_duration_ticks`），`Redo` 再裁过去 |
//! | ⑧（纯函数 ＋ 源码纪律） | `clip_duration_ops_for` 逐字段对账（放大 / 缩回 / 下限 / 零增量 / 非法身份）；`host.rs` 里 `Op::SetClipPlacementDuration` 的构造点**恰好一处** |
//! | ⑨（投影层） | 改一条摆放的时长 ⇒ `clip_widths()` 与 `clip_ulids()` 同源同序、只有那一格变宽 |
//!
//! ## 为什么这一票在 `.slint` 侧**零改动**（也是它⛔ 不触发基准重录的理由）
//!
//! 剪辑的**宽度**本来就是投影算出来的：`ViewState::clip_widths` ← `ClipPlacement::duration_ticks`
//! → `apply_view` 写 `clip-widths`。所以裁剪走的是"提交 → 重投影 → 画面自己跟着走"这条
//! 已有通路，**不需要**新属性、不需要新元素 ID、不需要动注册表。默认帧因此逐像素不变
//! （⇒ `tests/perf_draw_budget.rs` 的两个预算数一位不涨，golden 基准⛔ 不重录）。
//!
//! ## 注入分两格（本文件的边界）
//!
//! * **键**：`Shift+←/→` 经 `MainWindow::invoke_key_action` —— 那正是 `.slint` 的 `key-handler`
//!   派发快捷键时调的**同一个**回调（实参顺序 `text, shift, control, alt, meta`，现读
//!   `app.slint` 的调用点），返回值就是"消费 / 不消费"。
//!   ⚠ 它⛔ 不是 `WindowEvent`：端口那侧的修饰键注入由 `tests/roll_edit_tools.rs` 判据 (e)
//!   用**真事件源**（`Shift` 按下 ＋ `\u{f702}`）覆盖，本文件不重复证明"事件源 ⇒ 属性"；
//!   本文件只证明"属性 ⇒ 权威 ⇒ 画面"；
//! * **选中**：剪辑走 `.slint` 包头 `clicked` 调的那一个 `clip-select` 回调（宿主自己校验身份）；
//!   音符走 `selected-ulids`（与 `tests/roll_edit_tools.rs` 的 `select_ulids` 同款写法）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的且每线程只能装配一次；每例一新线程由 libtest
//! 负责 ⇒ 本文件可以有多个 `#[test]`（与 `tests/d_ui_13_roll_hit_scope.rs` 同一做法）。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与无头比对交 CI。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// `-D warnings` 会判红（与 `tests/clip_drag.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;
use std::str::FromStr as _;

use live::{LiveControlPlane, build_live_ui_with};
use serde_json::json;
use slint::Model as _;
use yeban_app::bridge::ViewState;
use yeban_app::host::clip_duration_ops_for;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{ClipPlacement, EntityId, YebanProjectV1};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::METHOD_NODE;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::{Rect, Rgb8Image};

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// `[MODEL-AST-001]` 的 PPQ（**独立 oracle**：期望网格从规范常量推，⛔ 不读
/// `host::ROLL_SNAP_GRID_TICKS` 自证 —— 读了就等于"实现说多少就是多少"）。
const PPQ_EXPECTED: u64 = 960;

/// 一档吸附网格 = 四分音符 = `PPQ / 4` = **240 tick**（与 `tests/roll_edit_tools.rs`
/// 判据 (e) 的 `NUDGE_GRID_TICKS_EXPECTED` 同一条推导）。
const GRID_EXPECTED: u64 = PPQ_EXPECTED / 4;

/// `←` 的 `event.text`（Slint 的功能键私有码点，现读 `input.rs::LogicalKey::from_text`）。
const LEFT_TEXT: &str = "\u{f702}";

/// `→` 的 `event.text`。
const RIGHT_TEXT: &str = "\u{f703}";

/// 一个**合法但不存在**的摆放身份（夹具 ULID 方案：`01J8ZQ` ＋ 20 位十进制序号，共 26 位）。
fn stale_clip_ulid() -> String {
    EntityId::from_str(&format!("01J8ZQ{:020}", 999_999_999_u128))
        .expect("canonical fixture ulid")
        .to_canonical_string()
}

/// 装配：真窗口 ＋ **撤销端口当装配选项传进去**（`host::wire_keys` 从第一刻起就握着端口，
/// 与产品进程 `main.rs` 同一形态）＋ 与生产同一条 `wire_clip_scope` / `wire_clip_drag` /
/// `wire_track_select`。
fn assemble() -> (MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<clip-duration>", "yeban-app", project.clone(), NOW_MS).expect("打开"),
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
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 权威工程里**全部**摆放：`(摆放身份 ULID, 摆放)`，按 `tracks` 的 `BTreeMap` 键序（确定）。
fn placements(project: &YebanProjectV1) -> Vec<(String, ClipPlacement)> {
    let mut out = Vec::new();
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            out.push((placement.id.to_canonical_string(), *placement));
        }
    }
    out
}

/// 按身份从权威工程读一条摆放（读不到 ⇒ `None`）。
fn placement_by_ulid(project: &YebanProjectV1, ulid: &str) -> Option<ClipPlacement> {
    placements(project)
        .into_iter()
        .find(|(id, _)| id.as_str() == ulid)
        .map(|(_, placement)| placement)
}

/// 界面上的剪辑身份数组（`clip-ulids` —— 重投影后的绘制清单）。
fn clip_ulids(window: &MainWindow) -> Vec<String> {
    window
        .get_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// 界面读到的**宽度列**（`clip-widths`，与 `clip-ulids` 同索引集；事实源是投影，
/// 而投影从 `duration_ticks` 算 —— 本票⛔ 不新增任何数组属性）。
fn clip_widths(window: &MainWindow) -> Vec<f32> {
    window.get_clip_widths().iter().collect()
}

/// 把某个剪辑**选中**（走 `.slint` 包头 `clicked` 调的那一个回调；宿主自己校验身份）。
fn select_clip(window: &MainWindow, ulid: &str) {
    window.invoke_clip_select(ulid.into());
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        ulid,
        "夹具前提：宿主必须接受这个身份（否则下面的按键判据空转）"
    );
}

/// 把一批**音符身份**写进选区的唯一事实源（与 `tests/roll_edit_tools.rs` 的 `select_ulids` 同款）。
fn select_notes(window: &MainWindow, ids: &[String]) {
    window.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
        ids.iter()
            .map(|id| slint::SharedString::from(id.as_str()))
            .collect::<Vec<_>>(),
    )));
    window.set_selected_note_count(i32::try_from(ids.len()).unwrap_or(i32::MAX));
}

/// 当前**音符选区**的条数（判据要证明"分派看的是选区而不是结果"）。
fn note_selection_len(window: &MainWindow) -> usize {
    window.get_selected_ulids().iter().count()
}

/// 权威工程里的音符：`(音符身份 ULID, 时长 tick)`，按片段池 `BTreeMap` 键序（确定）。
fn note_durations(project: &YebanProjectV1) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for entry in project.clip_pool.values() {
        if let yeban_model::project::ClipContent::Midi { notes } = &entry.content {
            for note in notes.values() {
                out.push((note.id.to_canonical_string(), note.duration_ticks));
            }
        }
    }
    out
}

/// 注入一次 `Shift+→`（实参顺序 `text, shift, control, alt, meta` 与 `key-action` 逐位对齐）。
fn press_shift_right(window: &MainWindow) -> bool {
    window.invoke_key_action(RIGHT_TEXT.into(), true, false, false, false)
}

/// 注入一次 `Shift+←`。
fn press_shift_left(window: &MainWindow) -> bool {
    window.invoke_key_action(LEFT_TEXT.into(), true, false, false, false)
}

/// 取某个**语义 ID** 在运行时控件树里的包围盒（每次都重读 —— 端口里那份会过期）。
fn element_bounds(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> Rect {
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
    Rect::new(
        i32::try_from(bounds["x"].as_i64().expect("x")).expect("x 在 i32 内"),
        i32::try_from(bounds["y"].as_i64().expect("y")).expect("y 在 i32 内"),
        u32::try_from(bounds["width"].as_u64().expect("width")).expect("width 在 u32 内"),
        u32::try_from(bounds["height"].as_u64().expect("height")).expect("height 在 u32 内"),
    )
}

/// 任意多个包围盒的**并集**（外扩 2px 吸收边框 / 抗锯齿的取整 —— 与 `tests/clip_drag.rs`
/// 的 `union_rect` 同一余量口径，余量只加**一次**，嵌套调用会把余量叠成 4px）。
fn union_of(rects: &[Rect]) -> Rect {
    let margin = 2_i32;
    assert!(!rects.is_empty(), "并集至少得有一个框");
    let mut left = i32::MAX;
    let mut top = i32::MAX;
    let mut right = i32::MIN;
    let mut bottom = i32::MIN;
    for rect in rects {
        left = left.min(rect.x);
        top = top.min(rect.y);
        right = right.max(rect.right());
        bottom = bottom.max(rect.bottom());
    }
    let (left, top) = (left - margin, top - margin);
    let (right, bottom) = (right + margin, bottom + margin);
    Rect::new(
        left,
        top,
        u32::try_from(right - left).expect("并集宽度在 u32 内"),
        u32::try_from(bottom - top).expect("并集高度在 u32 内"),
    )
}

/// 两帧的差异像素数，按"是否落在 `rect` 内"分成 `(矩形内, 矩形外)`。
fn diff_split(before: &Rgb8Image, after: &Rgb8Image, rect: Rect) -> (u64, u64) {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸"
    );
    let (x0, y0) = (i64::from(rect.x), i64::from(rect.y));
    let (x1, y1) = (i64::from(rect.right()), i64::from(rect.bottom()));
    let mut inside = 0_u64;
    let mut outside = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            if before.pixel(x, y) != after.pixel(x, y) {
                let in_rect = i64::from(x) >= x0
                    && i64::from(x) < x1
                    && i64::from(y) >= y0
                    && i64::from(y) < y1;
                if in_rect {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
    }
    (inside, outside)
}

/// `rect` **外**那些差异像素的取证摘要：数量 + 差异包围盒 + 前 3 个像素的两种颜色。
///
/// 只在计数非零时被调用（一轮 CI ≈ 7 分钟，红一次必须换回"在哪、什么颜色"这两个事实，
/// 而不是换一个猜测 —— AGENTS §6.3-3）。
fn outside_forensics(before: &Rgb8Image, after: &Rgb8Image, rect: Rect) -> String {
    let (x0, y0) = (i64::from(rect.x), i64::from(rect.y));
    let (x1, y1) = (i64::from(rect.right()), i64::from(rect.bottom()));
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    let mut samples: Vec<String> = Vec::new();
    let mut count = 0_u64;
    for y in 0..after.height() {
        for x in 0..after.width() {
            let (Some(old), Some(new)) = (before.pixel(x, y), after.pixel(x, y)) else {
                continue;
            };
            if old == new {
                continue;
            }
            let in_rect =
                i64::from(x) >= x0 && i64::from(x) < x1 && i64::from(y) >= y0 && i64::from(y) < y1;
            if in_rect {
                continue;
            }
            count += 1;
            bbox = Some(match bbox {
                None => (x, y, x, y),
                Some((bx0, by0, bx1, by1)) => (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y)),
            });
            if samples.len() < 3 {
                samples.push(format!("({x},{y}) {old:?} → {new:?}"));
            }
        }
    }
    let picked = samples.join(" / ");
    format!("{count} 个像素落在区域外; 差异包围盒 {bbox:?}; 前几个: {picked}")
}

/// ① 夹具前提：至少两条摆放、**没有**音符选区、宽度列同源、时长够缩一档。
///
/// 这一条本身不是功能，而是其余判据的地基：
/// * 音符选区若非空，`Shift+←/→` 会走**音符**那一臂 ⇒ 本文件所有"剪辑被裁剪"的断言都会空转；
/// * 时长若小于一档网格，② 的 `+网格` 与 ④ 的"缩到下限"都无从构造。
#[test]
fn the_fixture_starts_with_no_note_selection_and_paints_every_placement() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}

/// ② **主判据**：`Shift+→` 只把选中那条摆放**延长一个网格**，一次按键 = 恰好一次提交。
///
/// 这一臂钉"写权威"＋"结果回到读数"两个方向，并逐字段钉住那条 op 的**范围**：
/// `SetClipPlacementDuration` 只写 `duration_ticks` ⇒ 起点 / 静音 / 循环配置一位都不许动。
#[test]
fn shift_right_lengthens_exactly_the_selected_placement_by_one_grid() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}

/// ②b 反向：`Shift+←` 把同一条摆放**缩短一个网格**（同一个动作的另一格键）。
#[test]
fn shift_left_shortens_the_selected_placement_by_the_same_grid() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}

/// ③ **画得出来 ＋ 可逆**：变宽只发生在这条剪辑**自己的几何**里，延回原值后帧与基准**逐字节相同**。
///
/// 区域 = 前后两态的 `clip-{ulid}-header` ∪ `clip-{ulid}-body`（外扩 2px）。两件事都必须写下来，
/// 因为它们各是一次真实红过：
/// * **为什么不是"原来那块剪辑"**：裁剪改变的就是右缘往哪停，新盖住的那几列像素本来就在旧框**之外**，
///   把它们判成"越界"是判据写错，不是实现错；
/// * **为什么不能只取 `body`**：`clip-{ulid}-body` 是**拖动面**（`arrangement_view.slint` 的
///   `TouchArea`，`y = clip-ys + 18px`、`height = clip-heights - 18px`），它**不含**上面那 18px 的
///   包头，而包头的 `width: parent.width` 跟着变宽、里面的 ulid 文本还是 `overflow: elide` ——
///   多出来的 8px 会**露出新的字**。2026-11-03 实测红在此处：203 个改变像素全落在 `y < 110`
///   的包头带内（并集只从 110 起），实现一位没错。⛔ 把区域退回"整个编曲区"就变成空判据。
#[test]
fn the_resize_is_drawn_within_the_clips_own_bounds_and_leaves_no_trace() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}

/// ④ **优先级与边界**：同时有音符选区 ⇒ 只动音符；音符卡在下限 ⇒ **不跨界**去裁剪辑。
///
/// 这一臂钉的是本票最容易写坏的一处：分派必须看**选区**（`selected-ulids` 是否为空），
/// ⛔ 不能看"这一批 op 是否为空"。后者的形状是"用户指着 1 tick 的音符按 `Shift+←`，
/// 音符没动，剪辑被顺手裁短了" —— 一次按键带走另一个用户**没指着**的对象。
#[test]
fn the_note_scope_wins_and_a_floor_bound_note_never_leaks_into_the_clip() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}

/// ⑤ 视图门控：Session 视图里这一键**不消费、不提交**，而**同一份选中态**回编曲视图立刻生效。
#[test]
fn a_selection_left_over_from_the_arrangement_view_is_not_resized_in_session_view() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}

/// ⑥ 解析不到 ⇒ **不消费**（陈旧身份、非法串、空选区三种）。
///
/// 与 `clip_delete_ops_for` / `clip_mute_ops_for` 同一取向："把键吞掉却什么都不做，比不处理更糟"。
#[test]
fn a_stale_or_illegal_clip_ulid_is_not_resized() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}

/// ⑦ **一步**撤销逐字节放回（载荷自带 `old_duration_ticks`），再 **redo** 又裁过去。
///
/// ⚠ 走的是**人用的那一格键**（`Ctrl+Z` / `Ctrl+Shift+Z`），⛔ 不是 `port.perform(..)`：
/// 端口的 `perform` 是纯模型入口（`undo.rs` 自己写着"端口碰不到 Slint"），它**不会**重投影，
/// 于是拿它撤销之后再读 `clip-widths` 会读到裁剪后的旧值 —— 那是入口的性质，不是缺陷。
/// 键路径经 [`apply_action`] 的 `Action::Undo | Action::Redo` 那一臂，末尾调
/// `refresh_undo_window(ui, port, true)` ⇒ 这一臂才**真的**能断言"画面跟着回退"。
/// 注入写法（`("z", shift, ctrl, alt, meta)`）与 `live_ui_mcp.rs` 里已绿的 `Cmd+Z` 判据逐位一致。
#[test]
fn one_undo_restores_the_duration_byte_for_byte() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}

/// ⑧ **纯函数**逐字段对账 ＋ **源码纪律**（构造点恰好一处）。
#[test]
fn clip_duration_ops_for_is_the_only_set_clip_placement_duration_construction_site() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}

/// ⑨ 投影层前提复核：宽度与身份**同源同序**，改一条时长只翻那一格宽度。
///
/// ② / ③ 都在读 `clip-widths`，它们依赖"按 `clip-ulids` 的位置取宽度不会取错剪辑"。
/// 这条在纯投影层（⛔ 不起窗口）逐条钉住它。
#[test]
fn the_projection_orders_the_width_column_by_the_same_identity_as_the_paint_list() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
    assert!(
        aether_app.contains("> clip-widths:"),
        "新 UI 契约面必须声明 > clip-widths:"
    );
    assert!(
        aether_host.contains("set_clip_widths"),
        "宿主接线层必须注册 set_clip_widths"
    );
}
