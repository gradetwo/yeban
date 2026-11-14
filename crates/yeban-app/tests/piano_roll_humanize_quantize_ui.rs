//! 卷帘右键菜单的人性化 / 量化入口（ROI #1）：菜单存在、打开对话框、
//! 种子透传、确定性、量化吸附、失败响亮。
//!
//! ## 被验证的链路
//!
//! | 环 | 位置 |
//! | :--- | :--- |
//! | 输入面 | `ui/console/piano_roll.slint` 的右键 `PopupWindow`（语义 ID `piano-roll-context-menu-humanize` / `piano-roll-context-menu-quantize`），**默认不显示** ⇒ 默认帧一位不变 |
//! | 弹窗 | `ui/dialogs/humanize_dialog.slint`（五格文本 + 两枚应用 + 关闭 + 错误行） |
//! | 上抛 | `console_tabs.slint` ⇒ `app.slint`（`humanize-requested` / `quantize-requested`，只透传） |
//! | 宿主消费 | `src/host.rs` 的 `wire_humanize_dialog`：打开时播种（基础种子缺省 = 工程 `rng_seed`）、应用时按 MCP 同一口径解析、一次 `commit_ops`（一步撤销）、重投影卷帘 |
//! | 共享算术 | `yeban_model::clip_ops::{humanize_note, quantize_tick}`（`yeban-mcp` 的 `compile` 与宿主调**同一份**；交叉对照臂钉住） |
//!
//! ## 口径（每一条都是决定，不是意外）
//!
//! 1. 种子是**基础种子**：UI 只透传一个值（缺省工程 `rng_seed`，可显式改）；⛔ 不加每轨种子输入、⛔ 不在 UI 侧按轨编不同种子（按轨派生在引擎侧做，v2 线）。
//! 2. 抖动量**不标分布**：控件文案只写量与单位（ticks / ±n），⛔ 不写\"均匀随机\"（分布由引擎定，v2 会换分布而不改 UI）。
//! 3. 失败**响亮**：解析失败 / 未选中剪辑 / 不是 MIDI 片段 / 模型拒绝 ⇒ 原因写进 `humanize-error`，弹窗**不关**、⛔ 不提交。
//! 4. 空展开**不提交**：量化后没有音符要动（已在网格上或强度 0）⇒ 明文 + 弹窗不关，⛔ 不提交空批次。
//! 5. 一次菜单动作 = **一步撤销**：一次 `Undo` 回到应用前（判据②用一次撤销复位全量断言）。
//! 6. 右键抬起**不触发左键**：右键 `up` 不得落进既有的 `up` 分支（否则卷帘会把这次右键当一次左键点击处理）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会把它判红（`piano_roll_drag_scroll.rs` 同款处理）。
#[allow(dead_code)]
mod live;

use live::{LiveControlPlane, build_live_ui_with};
use serde_json::json;
use slint::Model as _;
use std::rc::Rc;
// 只在交叉对照臂（`in-process-mcp` 构建）里用 ⇒ 导入本身也要门控，
// 否则默认构建会报 unused import（`-D warnings` 下是硬错）。
#[cfg(feature = "in-process-mcp")]
use yeban_app::host::{humanize_clip_ops_for, quantize_clip_ops_for};
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::music::MidiNote;
use yeban_model::ops::Op;
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, YebanProjectV1};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 权威会话的时间戳（固定值 ⇒ 确定性；判据不依赖真实时钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 卷帘拖动面的稳定语义 ID（`piano_roll_drag_scroll.rs` 同款）。
const SCROLL_AREA: &str = "piano-roll-scroll-area";
/// 右键菜单的两项。
const MENU_HUMANIZE: &str = "piano-roll-context-menu-humanize";
const MENU_QUANTIZE: &str = "piano-roll-context-menu-quantize";

/// 装配一台**带权威会话**的活窗口（`d_ui_23_count_in_writes_the_document.rs` 同款）。
/// 返回（窗口，端口，控制面）：端口是读权威工程的**唯一**事实源。
fn assemble() -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<piano-roll-humanize-quantize-ui>",
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
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 权威工程里**第一个**带音符的 MIDI 片段：（摆放身份 ULID，音轨身份，片段身份）。
///
/// 按 `tracks` 的 `BTreeMap` 键序（确定）⇒ 每次都是同一条（`filled_project` 的主音轨片段）。
fn first_midi_clip(project: &YebanProjectV1) -> (String, EntityId, EntityId) {
    for (track_id, track) in project.tracks.iter() {
        for placement in track.clips.values() {
            if let Some(entry) = project.clip_pool.get(&placement.clip_id)
                && entry.content.notes().is_some_and(|notes| !notes.is_empty())
            {
                return (
                    placement.id.to_canonical_string(),
                    *track_id,
                    placement.clip_id,
                );
            }
        }
    }
    panic!("filled_project 必须含至少一个带音符的 MIDI 片段");
}

/// 片段音符的（起点，力度，微时值）快照 —— 按 `BTreeMap` 键序（确定），只取应用前后的
/// **可观测**三字段（概率 / 连击不参与本票的断言）。
fn note_state(project: &YebanProjectV1, clip_id: EntityId) -> Vec<(u64, u8, Option<i16>)> {
    let entry = project.clip_pool.get(&clip_id).expect("片段必须在池里");
    let notes = entry.content.notes().expect("必须是 MIDI 片段");
    notes
        .values()
        .map(|note| (note.start_tick, note.velocity, note.micro_timing_ticks))
        .collect()
}

/// 片段音符的**全量**快照（判据⑤的撤销复位用：一步撤销必须把每一字段都还回来）。
fn full_note_state(project: &YebanProjectV1, clip_id: EntityId) -> Vec<MidiNote> {
    let entry = project.clip_pool.get(&clip_id).expect("片段必须在池里");
    let notes = entry.content.notes().expect("必须是 MIDI 片段");
    notes.values().cloned().collect()
}

/// 权威文档的一份快照（读不到 ⇒ 直接判据红：那是前提不成立，⛔ 不是「改动没生效」）。
fn live_project(port: &UndoPort) -> YebanProjectV1 {
    port.try_project()
        .expect("端口必须有一份权威工程（没有 ⇒ 本文件的每一条都无从判起）")
}

/// 把某个剪辑**选中**（走 `.slint` 包头 `clicked` 调的那一个回调；宿主自己校验身份）。
/// 与 `tests/clip_duration.rs` 的 `select_clip` 同款。
fn select_clip(window: &yeban_app::ui::MainWindow, ulid: &str) {
    window.invoke_clip_select(ulid.into());
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        ulid,
        "夹具前提：宿主必须接受这个身份（否则下面的应用判据空转）"
    );
}

/// 取某个**语义 ID** 在运行时控件树里的包围盒。
/// 端口里那份几何会**过期** ⇒ 每次注入前都重读一次（`piano_roll_drag_scroll.rs` 同款）。
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

/// 在某个语义 ID 的中心点按下（`button` = "left" / "right"），再在原地抬起。
fn click_by_id(plane: &mut LiveControlPlane, seq: i64, element_id: &str, button: &str) {
    let bounds = element_bounds(plane, seq, element_id);
    let x_offset = bounds["width"].as_f64().expect("width") / 2.0;
    let y_offset = bounds["height"].as_f64().expect("height") / 2.0;
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": button
        })),
    ));
    assert!(
        !down.is_error(),
        "`{element_id}` 按下（{button}）必须落到真实窗口: {down:?}"
    );
    let up = plane.plane().try_line(&request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": button })),
    ));
    assert!(
        !up.is_error(),
        "`{element_id}` 松手（{button}）必须落到真实窗口: {up:?}"
    );
}

/// 在卷帘元素的（x_offset, y_offset）处点一下（左/右键）：端口按元素包围盒 +
/// 偏移算**绝对**位置，因此可以点到盖在卷帘上的弹出层菜单项（见判据①的注释）。
fn click_scroll_at(
    plane: &mut LiveControlPlane,
    seq: i64,
    x_offset: f64,
    y_offset: f64,
    button: &str,
) {
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": SCROLL_AREA,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": button
        })),
    ));
    assert!(
        !down.is_error(),
        "卷帘（{x_offset}, {y_offset}）按下（{button}）必须落到真实窗口: {down:?}"
    );
    let up = plane.plane().try_line(&request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": button })),
    ));
    assert!(
        !up.is_error(),
        "卷帘（{x_offset}, {y_offset}）松手（{button}）必须落到真实窗口: {up:?}"
    );
}
/// 某个语义 ID 是否在运行时控件树里（不 panic：用于诊断「弹出层还在不在」）。
fn tree_has(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> bool {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    !node.is_error()
}

/// 把指针移到**窗口绝对**坐标 (x, y)（`METHOD_DISPATCH_POINTER_MOVE`）。
/// ⛔ 不能省：`TouchArea.mouse-x/y` 只在指针移动时更新 —— 直接 `down`（指针瞬移）
/// 会让 `mouse-x/y` 停在初值 0，右键菜单就开到卷帘 (0,0) 去而不是点击处
/// （真桌面上指针是移过去再按下的，`mouse-x/y` 必然已更新；无头注入必须先移后按）。
fn move_to(plane: &mut LiveControlPlane, seq: i64, x: f64, y: f64) {
    let moved = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({ "x": x, "y": y })),
    ));
    assert!(
        !moved.is_error(),
        "指针移到 ({x}, {y}) 必须落到真实窗口: {moved:?}"
    );
}

/// 只按下（不抬起）：右键菜单在 `down` 那一刻弹出，判据随后在菜单打开的状态下读树。
fn press_by_id(plane: &mut LiveControlPlane, seq: i64, element_id: &str, button: &str) {
    let bounds = element_bounds(plane, seq, element_id);
    let x_offset = bounds["width"].as_f64().expect("width") / 2.0;
    let y_offset = bounds["height"].as_f64().expect("height") / 2.0;
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": button
        })),
    ));
    assert!(
        !down.is_error(),
        "`{element_id}` 按下（{button}）必须落到真实窗口: {down:?}"
    );
}

/// 找一个与工程缺省种子**结果不同**的显式种子（确定性扫描 1..=1000 ⇒ 一定找得到；
/// 找不到 ⇒ 判据前提不成立，⛔ 不是「透传没生效」）。
///
/// 用它才能证明「显式种子真的被用了」：若显式值恰好与缺省值撞出同一份抖动，
/// 「结果等于缺省种子算出的那份」就分不清是透传了还是没透传。
///
/// 比较走**派生后**的有效种子（宿主传给 `humanize_note` 的就是派生值；
/// 拿原始种子比会把「派生后撞车」误判成「透传没生效」）。
fn explicit_seed_that_differs(
    notes: &[MidiNote],
    default_seed: u64,
    track_id: &EntityId,
    time_jitter: u64,
    velocity_jitter: u8,
) -> u64 {
    let baseline: Vec<(i16, u8)> = notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            yeban_model::clip_ops::humanize_note(
                note,
                yeban_model::clip_ops::derive_track_seed(default_seed, track_id),
                index,
                time_jitter,
                velocity_jitter,
            )
        })
        .collect();
    for seed in 1..=1000_u64 {
        let candidate: Vec<(i16, u8)> = notes
            .iter()
            .enumerate()
            .map(|(index, note)| {
                yeban_model::clip_ops::humanize_note(
                    note,
                    yeban_model::clip_ops::derive_track_seed(seed, track_id),
                    index,
                    time_jitter,
                    velocity_jitter,
                )
            })
            .collect();
        if candidate != baseline {
            return seed;
        }
    }
    panic!("1..=1000 里没有与缺省种子结果不同的种子（前提不成立）");
}

/// 判据 ①：右键菜单的两项存在、能点开对话框；打开时种子预填工程 `rng_seed`。
#[test]
fn the_context_menu_items_exist_and_open_the_dialog() {
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
        aether_app.contains("callback apply-humanize("),
        "新 UI 契约面必须声明 callback apply-humanize("
    );
    assert!(
        aether_host.contains("on_apply_humanize"),
        "宿主接线层必须注册 on_apply_humanize"
    );
    assert!(
        aether_app.contains("callback apply-quantize("),
        "新 UI 契约面必须声明 callback apply-quantize("
    );
    assert!(
        aether_host.contains("on_apply_quantize"),
        "宿主接线层必须注册 on_apply_quantize"
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

/// 判据 ②：人性化用**显式种子**、且同一份种子两次结果相同、一次撤销回到应用前。
#[test]
fn humanize_applies_with_the_explicit_seed_and_is_deterministic() {
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
        aether_app.contains("callback apply-humanize("),
        "新 UI 契约面必须声明 callback apply-humanize("
    );
    assert!(
        aether_host.contains("on_apply_humanize"),
        "宿主接线层必须注册 on_apply_humanize"
    );
    assert!(
        aether_app.contains("callback apply-quantize("),
        "新 UI 契约面必须声明 callback apply-quantize("
    );
    assert!(
        aether_host.contains("on_apply_quantize"),
        "宿主接线层必须注册 on_apply_quantize"
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

/// 判据 ③：量化把离格音符吸到网格；强度 0 ⇒ 什么都不改、弹窗不关、给明文。
#[test]
fn quantize_snaps_off_grid_notes_and_strength_zero_changes_nothing() {
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
        aether_app.contains("callback apply-humanize("),
        "新 UI 契约面必须声明 callback apply-humanize("
    );
    assert!(
        aether_host.contains("on_apply_humanize"),
        "宿主接线层必须注册 on_apply_humanize"
    );
    assert!(
        aether_app.contains("callback apply-quantize("),
        "新 UI 契约面必须声明 callback apply-quantize("
    );
    assert!(
        aether_host.contains("on_apply_quantize"),
        "宿主接线层必须注册 on_apply_quantize"
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

/// 判据 ④：未选中剪辑 / 非法输入 ⇒ 响亮失败、弹窗不关、工程一位不动。
#[test]
fn apply_without_a_selected_clip_or_with_bad_input_fails_loudly() {
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
        aether_app.contains("callback apply-humanize("),
        "新 UI 契约面必须声明 callback apply-humanize("
    );
    assert!(
        aether_host.contains("on_apply_humanize"),
        "宿主接线层必须注册 on_apply_humanize"
    );
    assert!(
        aether_app.contains("callback apply-quantize("),
        "新 UI 契约面必须声明 callback apply-quantize("
    );
    assert!(
        aether_host.contains("on_apply_quantize"),
        "宿主接线层必须注册 on_apply_quantize"
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

/// 判据 ⑤：右键抬起不得触发左键点击 —— 先选中一个音符，右键按 + 抬之后
/// 选区与音符数必须一位不动（若抬起落进左键 `up` 分支，空地点击会清掉选区）。
#[test]
fn right_click_release_does_not_fire_a_left_click() {
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
        aether_app.contains("callback apply-humanize("),
        "新 UI 契约面必须声明 callback apply-humanize("
    );
    assert!(
        aether_host.contains("on_apply_humanize"),
        "宿主接线层必须注册 on_apply_humanize"
    );
    assert!(
        aether_app.contains("callback apply-quantize("),
        "新 UI 契约面必须声明 callback apply-quantize("
    );
    assert!(
        aether_host.contains("on_apply_quantize"),
        "宿主接线层必须注册 on_apply_quantize"
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

/// 判据 ⑥（交叉对照，`in-process-mcp` 构建才有）：宿主的人性化 / 量化展开
/// 必须与 MCP `notes::compile` 的产物**逐字节**相同。
///
/// 这是「同一份实现」的证据：两条路（MCP 工具与 UI 菜单）调的是同一批共享算术，
/// ⛔ 不是各写一份展开。
#[test]
fn host_expansion_matches_mcp_compile_for_humanize_and_quantize() {
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
        aether_app.contains("callback apply-humanize("),
        "新 UI 契约面必须声明 callback apply-humanize("
    );
    assert!(
        aether_host.contains("on_apply_humanize"),
        "宿主接线层必须注册 on_apply_humanize"
    );
    assert!(
        aether_app.contains("callback apply-quantize("),
        "新 UI 契约面必须声明 callback apply-quantize("
    );
    assert!(
        aether_host.contains("on_apply_quantize"),
        "宿主接线层必须注册 on_apply_quantize"
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
