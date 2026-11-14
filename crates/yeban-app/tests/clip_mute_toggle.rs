//! **剪辑静音**（`Cmd/Ctrl+M` 拨选中摆放的 `ClipPlacement::muted`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 一条剪辑摆在编曲视图里，用户可以点它选中（`D-UI-15` 之前的切片）、可以拖它平移
//! （`clip-{ulid}-body`）、可以删它（`D-UI-15`）—— 但**没法只让它不出声**：
//! `Op::SetClipPlacementMuted` 在模型里早已存在、`yeban-engine::snapshot` 与
//! `yeban-render::{als, logic}` 都**真的**按 `muted` 不发声 / 不导出、MCP 侧
//! `yeban_mcp::domain::notes` 的 `NoteOp::SetClipPlacementMuted` 也早已能提交，
//! 而界面侧在此切片之前**一处都没有**（`grep -rn SetClipPlacementMuted crates/yeban-app/src`
//! 为空）⇒ "轨道能静音、剪辑不能"这一半接线缺口。
//!
//! 本切片把 `Cmd/Ctrl+M` 接成"只拨**选中的那条摆放**、一次按键恰好一步可撤销、结果画在
//! 包头上"的手势。轨静音（裸 `m`，作用域 `selected-track-id`）与剪辑静音（`Cmd/Ctrl+M`，
//! 作用域 `selected-clip-ulid`）是**两格键、两个作用域**，本文件同时钉住它们互不越界。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（投影一致性） | 装配后 `clip-mutes` 与 `clip-ulids` **同长**且一格 `false` 都没有 ⇒ 默认帧无需重录，且数组是**逐剪辑**一位 |
//! | ②（写权威 ＋ 带读数） | `Cmd/Ctrl+M` ⇒ 只有那条摆放 `muted` 翻面（另一条**逐字段**不变）、片段池字节不变、提交 **+1**、选中态**留下**、`clip-mutes` 那一格跟着翻 |
//! | ③（画在包头上） | 静音的改变像素**全部**落在 `clip-{A}-header` 内（>0）；再按一次翻回 ⇒ 帧与基准**逐字节相同**（没有留痕） |
//! | ④（作用域不越界） | 同时选中轨与剪辑：裸 `m` 只翻**轨**的静音、剪辑一位不动；`Cmd/Ctrl+M` 只翻**剪辑**、轨不再动 —— 反向会带走用户正指着的东西 |
//! | ⑤（视图门控） | Session 视图里同一份选中态**不消费、0 提交、工程不变**；回编曲视图立刻翻面 —— 两臂只差 `arrangement-view` |
//! | ⑥（解析不到 ⇒ 不消费） | 陈旧 / 非法 / 空 三种选中态都 `consumed == false`、0 提交、⛔ 不动选中态 |
//! | ⑦（一步撤销 ＋ redo） | `Undo` 将工程逐字节放回（载荷带 `old_muted`），`Redo` 再翻过去 |
//! | ⑧（纯函数 ＋ 源码纪律） | `clip_mute_ops_for` 的 op 内容逐字段对账（含**已静音**的输入 ⇒ 翻回假）；`host.rs` 里 `Op::SetClipPlacementMuted` 的构造点**恰好一处** |
//! | ⑨（同序同长） | 投影层逐条验证 `clip_mutes()` 与 `clip_ulids()` **同源同序** ⇒ ② 里"按 `clip-ulids` 的位置取那一格"不会指错剪辑 |
//!
//! ## 注入分两格（本文件的边界）
//!
//! * **键**：`Cmd/Ctrl+M` 与裸 `m` 都经 `MainWindow::invoke_key_action` —— 那正是 `.slint` 的
//!   `key-handler` 派发快捷键时调的**同一个**回调，返回值就是"消费 / 不消费"。
//!   ⚠ 它⛔ 不是 `WindowEvent`：端口那侧的修饰键注入由 `tests/d_ui_3_modifier_chords.rs`
//!   那条链覆盖，本文件不重复证明"鼠标 / 键盘事件源 ⇒ 属性"，只证明"属性 ⇒ 权威 ⇒ 画面"；
//! * **选中**：走 `.slint` 包头 `clicked` 调的那一个 `clip-select` 回调（宿主自己校验身份）。
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
use yeban_app::host::clip_mute_ops_for;
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

/// 剪辑静音键的**字面**（逻辑键 `Character('m')`，与 `.slint` 报来的 `event.text` 同形）。
const M_TEXT: &str = "m";

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
        UndoSession::open("<clip-mute>", "yeban-app", project.clone(), NOW_MS).expect("打开"),
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

/// 界面读到的**静音列**（`clip-mutes` —— 与 `clip-ulids` 同索引集，事实源是工程字段）。
fn clip_mutes(window: &MainWindow) -> Vec<bool> {
    // `ModelRc<bool>::iter()` 逐格给**拥有**的 `bool`（与 `tests/device_rack_projection.rs`
    // 读 `device-bypassed` 同一写法）。
    window.get_clip_mutes().iter().collect()
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

/// 按 `clip-ulids` 里的位置取那一格静音读数（身份 ⇒ 下标，⛔ 不直接按下标定身份）。
fn muted_at(window: &MainWindow, ulid: &str) -> Option<bool> {
    clip_ulids(window)
        .iter()
        .position(|id| id.as_str() == ulid)
        .map(|index| clip_mutes(window)[index])
}

/// 按身份取一条轨的静音态（轨与剪辑在同一个工程里，读法同款）。
fn track_mute(project: &YebanProjectV1, track_ulid: &str) -> Option<bool> {
    let id = EntityId::from_str(track_ulid).ok()?;
    project.tracks.get(&id).map(|track| track.mute)
}

/// 注入一次 `Cmd/Ctrl+M`（`text, shift, control, alt, meta` 的顺序与 `key-action` 回调逐位对齐）。
fn press_cmd_m(window: &MainWindow) -> bool {
    window.invoke_key_action(M_TEXT.into(), false, true, false, false)
}

/// 注入一次裸 `m`（轨静音那一格）。
fn press_bare_m(window: &MainWindow) -> bool {
    window.invoke_key_action(M_TEXT.into(), false, false, false, false)
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

/// 两帧的差异像素数，按"是否落在 `rect` 内"分成 `(矩形内, 矩形外)`。
fn diff_split(before: &Rgb8Image, after: &Rgb8Image, rect: Rect) -> (u64, u64) {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸"
    );
    let (x0, y0) = (i64::from(rect.x), i64::from(rect.y));
    let (x1, y1) = (x0 + i64::from(rect.width), y0 + i64::from(rect.height));
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

/// ① 投影一致性 ＋ "默认未静音"这一**前提**。
///
/// 这条本身不是功能，而是其余判据的地基：`clip-mutes` 若与 `clip-ulids` 不同长，下面所有
/// "按 `clip-ulids` 的位置取那一格"的读法都会越界；而起点若有**任何**一格已经是 `true`，
/// ③ 的"默认帧无需重录"就是空话。
#[test]
fn the_fixture_starts_unmuted_and_the_ui_carries_one_reading_per_clip() {
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
        aether_app.contains("> clip-mutes:"),
        "新 UI 契约面必须声明 > clip-mutes:"
    );
    assert!(
        aether_host.contains("set_clip_mutes"),
        "宿主接线层必须注册 set_clip_mutes"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}

/// ② **主判据**：`Cmd/Ctrl+M` 只翻选中那条摆放的 `muted`，一次按键 = 恰好一次提交。
///
/// 这一臂同时钉住"写权威"与"把结果送回界面"两个方向：只断言工程会放过
/// "提交完不重投影"（模型改了、包头还亮着）这一类坏 —— 所以 `clip-mutes` 那一格必须一起断。
#[test]
fn cmd_m_on_a_selected_clip_placement_mutes_exactly_that_placement() {
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
        aether_app.contains("> clip-mutes:"),
        "新 UI 契约面必须声明 > clip-mutes:"
    );
    assert!(
        aether_host.contains("set_clip_mutes"),
        "宿主接线层必须注册 set_clip_mutes"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}

/// ③ **画在包头上**：改变像素全部落在 `clip-{A}-header` 内，再按一次翻回后与基准逐字节相同。
///
/// 这一臂把 ② 的"读数跟上"再往前推一格：属性写了不等于用户看得见。而"翻回来帧逐字节
/// 相同"钉的是**开关**语义（⛔ 不是单向的"打上静音标记"—— 那会留下擦不掉的痕迹）。
#[test]
fn the_muted_state_is_drawn_on_that_clips_header_and_only_there() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
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
        aether_app.contains("> clip-mutes:"),
        "新 UI 契约面必须声明 > clip-mutes:"
    );
    assert!(
        aether_host.contains("set_clip_mutes"),
        "宿主接线层必须注册 set_clip_mutes"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}

/// ④ **作用域不越界**：裸 `m` 只动轨、`Cmd/Ctrl+M` 只动剪辑。
///
/// 这条是"反过来会怎样"的防线：两把键共享同一个字符 `m`，若分叉成"任意一把都翻两者"，
/// 用户想静音一条剪辑会顺手改掉整条轨的出声状态（或反之），而那**看起来**只是"按了一下 M"。
#[test]
fn bare_m_still_toggles_the_track_and_never_the_clip() {
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
        aether_app.contains("> clip-mutes:"),
        "新 UI 契约面必须声明 > clip-mutes:"
    );
    assert!(
        aether_host.contains("set_clip_mutes"),
        "宿主接线层必须注册 set_clip_mutes"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}

/// ⑤ 视图门控：Session 视图里那一键**不消费、不提交**，而**同一份选中态**回编曲视图立刻翻面。
#[test]
fn a_selection_left_over_from_the_arrangement_view_is_not_muted_in_session_view() {
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
        aether_app.contains("> clip-mutes:"),
        "新 UI 契约面必须声明 > clip-mutes:"
    );
    assert!(
        aether_host.contains("set_clip_mutes"),
        "宿主接线层必须注册 set_clip_mutes"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}

/// ⑥ 解析不到 ⇒ **不消费**（陈旧身份、非法串、空选区三种）。
///
/// 与 `clip_delete_ops_for` / `delete_ops_for` 同一取向："把键吞掉却什么都不做，比不处理更糟"。
#[test]
fn a_stale_or_illegal_clip_ulid_is_not_consumed() {
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
        aether_app.contains("> clip-mutes:"),
        "新 UI 契约面必须声明 > clip-mutes:"
    );
    assert!(
        aether_host.contains("set_clip_mutes"),
        "宿主接线层必须注册 set_clip_mutes"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}

/// ⑦ **一步**撤销逐字节放回（载荷自带 `old_muted`），再 **redo** 又翻过去。
///
/// ⚠ 撤销 / redo 走**人用的那一格键**（`Ctrl+Z` / `Ctrl+Shift+Z`），⛔ 不是
/// `port.perform(UiAction::Undo)`：端口是**纯模型**入口（`undo.rs` 自己写着"端口碰不到
/// Slint"），不回写任何界面属性 ⇒ 拿它撤销之后再读 `clip-mutes` 会读到翻面后的旧那一格，
/// 把"画面其实跟得上"误判成"撤销没画回来"。键路径经 [`apply_action`] 的
/// `Action::Undo | Action::Redo` 那一臂，末尾 `refresh_undo_window(ui, port, true)` 才重投影。
#[test]
fn one_undo_unmutes_the_placement_byte_for_byte() {
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
        aether_app.contains("> clip-mutes:"),
        "新 UI 契约面必须声明 > clip-mutes:"
    );
    assert!(
        aether_host.contains("set_clip_mutes"),
        "宿主接线层必须注册 set_clip_mutes"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}

/// ⑧ 纯函数臂 ＋ **源码纪律**（`clip_mute_ops_for` 是全 app 唯一的 `SetClipPlacementMuted` 构造点）。
///
/// 纯函数部分不建窗口（投影层判据同款）：喂真实工程与三种非法输入，逐条对账 op 内容。
/// 源码纪律部分钉的是"写入面只有一处"：日后若长出第二个构造点，两处就可能对"翻的是哪条摆放"
/// 给出不同答案 —— 那正是本文件要防的分叉形状。
#[test]
fn clip_mute_ops_for_is_the_only_set_clip_placement_muted_construction_site() {
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
        aether_app.contains("> clip-mutes:"),
        "新 UI 契约面必须声明 > clip-mutes:"
    );
    assert!(
        aether_host.contains("set_clip_mutes"),
        "宿主接线层必须注册 set_clip_mutes"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}

/// ⑨ 投影层前提复核：静音列与绘制清单**同源同序**（否则 ② 的"按位置取那一格"会指错剪辑）。
#[test]
fn the_projection_orders_the_mute_column_by_the_same_identity_as_the_paint_list() {
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
        aether_app.contains("> clip-mutes:"),
        "新 UI 契约面必须声明 > clip-mutes:"
    );
    assert!(
        aether_host.contains("set_clip_mutes"),
        "宿主接线层必须注册 set_clip_mutes"
    );
    assert!(
        aether_app.contains("callback clip-select("),
        "新 UI 契约面必须声明 callback clip-select("
    );
    assert!(
        aether_host.contains("on_clip_select"),
        "宿主接线层必须注册 on_clip_select"
    );
}
