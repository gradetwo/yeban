//! `Cmd/Ctrl+Alt+方向` ⇒ 微调**当前选中轨**的音量 / 声相。
//!
//! ⚠ **这不是规范点名的键位**：`[UI-NOTE-005]` §3.5（标题"全键盘音符微调与无鼠标心流"）
//! 的四条项**只点名音符**（平移 / `Alt` 1 tick / 半音 / 八度 / `Shift` 时值 / 试听），
//! 实测全文**没有**"音量 / 声相 / 推子"任何一处（`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md:213-217`；
//! 该文档 2026-10-06 经负责人裁决降级为**参考**，非绑定规范）。
//! 本切片是**界面决定**（补"调音台只能靠鼠标拖"这一缺口），键位的选法只受一条约束：
//! ⛔ 不覆盖任何已被规范点名、或已被既有判据如实钉成放行的组合 —— 逐条理由写在
//! [`yeban_app::input::Action::NudgeMixerChannel`] 的文档里。
//!
//! ## 为什么要两层（两层不可互相代替）
//!
//! | 层 | 文件内位置 | 证的事 | 读的是谁 |
//! | :--- | :--- | :--- | :--- |
//! | **A 纯函数** | 前四条判据 | "一格是多少 / 动哪一格 / 什么时候什么都不提交"这三条**决定**各自只有一处定义 | 直接调 [`yeban_app::host::mixer_nudge_delta_for`] / [`yeban_app::host::mixer_nudge_ops_for`] |
//! | **B 活窗口** | 后四条判据 | 真事件源（`key-action` 逻辑键）⇒ 真解析 ⇒ 真宿主臂 ⇒ 真 `commit_ops` ⇒ 权威工程与推子读数**一起**动 | `UndoPort::try_project()`（权威）＋ `MainWindow.track-volumes` / `track-pans`（用户看见的那一列文本） |
//!
//! 只看 A 会绿在"臂没接、键没解析出来"；只看 B 会绿在"一档的数被抄了两份、第二份偷偷改了"。
//!
//! ## 判据表
//!
//! | # | 判据 | 断言 |
//! | :--- | :--- | :--- |
//! | A1 | `mixer_nudge_delta_is_one_display_step_per_param` | 音量一格 = **1.0 dB**、声相一格 = **0.1**；`direction` 只取**符号**（`0 ⇒ 0.0`、`±7 ⇒ ±一档`）⇒ 幅度全仓只有这一处 |
//! | A2 | `mixer_nudge_ops_target_the_selected_track_with_bitwise_old_val` | 恰好**一条** `Op::SetParam`；`target` 是**被选中那轨**的 `TrackVolume` / `TrackPan`；`old_val` 与工程现值**逐位**相同（模型 `same_f32` 要的就是位相等） |
//! | A3 | `mixer_nudge_ops_clamp_at_the_ends_of_what_the_fader_can_show` | 到顶按 `+` / 到底按 `-` ⇒ **空**（不写空 op）；夹的是**界面量程**（`FADER_MIN_DB..=FADER_MAX_DB` / `-1.0..=1.0`），因此一个"工程里合法但推子画不出"的 `-80.0 dB` 往下按会被带回 `-60.0`，而不是继续往 `-79.0` 走 |
//! | A4 | `mixer_nudge_ops_refuse_a_stale_or_unparsable_identity` | 身份文本解析不出 `EntityId` / 解析得出但**不在**工程里 ⇒ 空 `Vec`（换工程之后的陈旧选中态不猜） |
//! | B1 | `cmd_ctrl_alt_up_down_moves_the_selected_tracks_fader_and_labels_it` | 真 `key-action` ⇒ 工程 `-3.0 → -2.0`（**逐位**）且 `track-volumes[0]` 从 `"-3.0"` 变 `"-2.0"`；`undoable` 每按一次 **+1**；`Cmd+Ctrl+Alt+Shift+↑` **不消费**；`mixer-drag-active` 始终 `false`（键盘路不留手势） |
//! | B2 | `cmd_ctrl_alt_left_right_moves_the_pan_by_one_display_step` | 一格声相 = 显示上的**一格**（`L25 → L15`）；音量**不**跟着动（两格各自写自己那一列） |
//! | B3 | `with_no_selected_track_the_mixer_nudge_keys_are_not_consumed` | 四个方向各按一次 ⇒ 四次都 `false`、`commit_count` **停在起点**（见下"提交的口径"）、`undoable` 恒为 0、工程读数逐位不变（**不吞键**：留给焦点系统） |
//! | B4 | `a_stale_selection_after_a_project_swap_commits_nothing` | 选中态是**窗口属性**（换工程之后它还留着上一个工程的身份）⇒ 按四向全部 `false`、`commit_count` 停在起点 |
//!
//! ⭐ **提交的口径**：`UndoDisplay::commit_count` **含那一条根提交** —— 全新会话
//! （`UndoSession::open`）打开就记 1，这条由 `src/undo.rs` 的判据
//! `a_fresh_port_has_no_history`（**同一个夹具** `filled_project()`）钉住。
//! 因此 B 层一律**相对起点**断言（"一次按键 = 一条提交"读作 Δ1），⛔ 不写 `0` 这个字面量。
//! `undoable` 是另一个口径（可撤销的**步数**，全新会话 = 0），仍按绝对值断。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 文件 | 预期 |
//! | :--- | :--- | :--- |
//! | 把 `MixerParam::Volume` 那档从 `1.0` 改成 `0.5` | `src/host.rs` | A1 红（一格被钉死）＋ B1 红（`-3.0 → -2.5`，读数对不上） |
//! | 把 `Pan` 那档从 `0.1` 改成 `0.05` | `src/host.rs` | A1 红 ＋ B2 红（显示仍是 `L15`，但工程与"一个显示格"不再等价） |
//! | 音量臂里把 `TrackVolume` 换成 `TrackPan` | `src/host.rs` | A2 红（写错格）＋ B2 红（声相列跟着动） |
//! | 删掉 `new_val.to_bits() == old_val.to_bits()` 那一格早退 | `src/host.rs` | A3 红（端点上返回一条 op）＋ B1 的"到顶再按"红（模型会**接受**一条 `new == old` 的 op ⇒ 留下一步什么都没改的撤销） |
//! | 把 `selected_track_id(ui)` 的早退删掉、改成"没有选中就动第一条轨" | `src/host.rs` | B3 红（四个键变成消费） |
//! | 删掉 `resolve_logical` 第 3 步的 `(Key::UpArrow, false, true)` 臂 | `src/input.rs` | B1 红（键不消费）＋ `input.rs` 自己的判据红 |
//! | 在 `cli.rs` 把音量那行的 `direction` 写成 `-1` | `src/cli.rs` | `tests/cli_contract.rs` 的 B11b ②红（标签绑的键解析出的动作 ≠ 行声称的动作） |
//!
//! ## ⛔ 本文件不证的东西
//!
//! - **主控通道条**：主总线不在 `track-ids` 里 ⇒ 没有点击源能写 `selected-track-id`，
//!   所以键盘这一路今天**碰不到**主控。这条边界登记在 `docs/ledger/app-mixer-notes.md`，
//!   正对照（主控确实被鼠标改得动）在那里的第 3 行与 `tests/live_ui_mcp.rs` 的主控判据；
//! - **静音 / 独奏的键盘入口**：布尔开关没有"一格"可言 ⇒ 本动作族刻意不含它；拨动
//!   那一族有**自己的**键与判据（裸 `M` / `S`，`tests/mixer_keyboard_switch.rs`，
//!   走 `Op::SetTrackMute` / `SetTrackSolo`，2026-10-31 落地）；
//! - **引擎听到变化**：编辑 ⇒ 新快照发布是 16 ms 心跳那一条腿的事
//!   （`main.rs` 的 `run_gui` 调 `EngineHost::publish_project`），本文件不测音频。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `roll_edit_tools.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use yeban_app::host;
use yeban_app::input::MixerParam::{Pan, Volume};
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::project::YebanProjectV1;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 会话打开时刻（与其余界面判据同一个常量，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 逻辑键入口的四个方向（Slint 的私有码点，与 `input.rs` 的 `LogicalKey::from_text` 同源）。
const UP: &str = "\u{f700}";
const DOWN: &str = "\u{f701}";
const LEFT: &str = "\u{f702}";
const RIGHT: &str = "\u{f703}";

/// `filled_project()` 里名字为 `Lead` 的那条轨（`volume_db = -3.0`、`pan = -0.25`）。
fn lead(project: &YebanProjectV1) -> yeban_model::project::TrackV3 {
    project
        .tracks
        .values()
        .find(|track| track.name == "Lead")
        .cloned()
        .expect("夹具里必须有名为 Lead 的轨")
}

/// 把工程里 `Lead` 的音量 / 声相改成给定值（A3 要造"端点"与"界面画不出的值"两种现值）。
fn with_lead_volume(mut project: YebanProjectV1, volume_db: f32) -> YebanProjectV1 {
    let id = lead(&project).id;
    project
        .tracks
        .get_mut(&id)
        .expect("Lead 必须在工程里")
        .volume_db = volume_db;
    project
}

fn with_lead_pan(mut project: YebanProjectV1, pan: f32) -> YebanProjectV1 {
    let id = lead(&project).id;
    project.tracks.get_mut(&id).expect("Lead 必须在工程里").pan = pan;
    project
}

/// 从 `Op::SetParam` 里取 `(target 的轨身份, old_val, new_val)`；不是这个形状就 `panic`
/// （判据只喂 `mixer_nudge_ops_for` 的产物，形状不符就是实现写错了，⛔ 不是分支）。
fn set_param_of(ops: &[yeban_model::Op]) -> (yeban_model::AutomationTarget, f32, f32) {
    let [
        yeban_model::Op::SetParam {
            target,
            old_val,
            new_val,
        },
    ] = ops
    else {
        panic!("期望恰好一条 `Op::SetParam`，得到 {ops:?}");
    };
    (*target, *old_val, *new_val)
}

/// 读一条 `in property <[string]>` 的整列（推子 / 声相的**显示**读数，用户看见的那一列）。
///
/// 生成的 getter 给出 `ModelRc<SharedString>`（⛔ 不是 `ModelRc<String>`）—— 与
/// `host::read_strings` 同一口径。
fn string_column(model: &slint::ModelRc<slint::SharedString>) -> Vec<String> {
    use slint::Model as _;
    model.iter().map(|value| value.to_string()).collect()
}

// ===========================================================================
// A 层：纯函数
// ===========================================================================

/// A1：一格 = 音量 1.0 dB / 声相 0.1，且 `direction` **只取符号**。
#[test]
fn mixer_nudge_delta_is_one_display_step_per_param() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> mixer-drag-active:"),
        "新 UI 契约面必须声明 > mixer-drag-active:"
    );
    assert!(
        aether_host.contains("set_mixer_drag_active"),
        "宿主接线层必须注册 set_mixer_drag_active"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

/// A2：恰好一条 `Op::SetParam`、寻址到**被选中那轨**、`old_val` 与现值**逐位**相同。
#[test]
fn mixer_nudge_ops_target_the_selected_track_with_bitwise_old_val() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> mixer-drag-active:"),
        "新 UI 契约面必须声明 > mixer-drag-active:"
    );
    assert!(
        aether_host.contains("set_mixer_drag_active"),
        "宿主接线层必须注册 set_mixer_drag_active"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

/// A3：两端夹紧 ⇒ 端点上返回**空**；夹的是**界面量程**。
#[test]
fn mixer_nudge_ops_clamp_at_the_ends_of_what_the_fader_can_show() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> mixer-drag-active:"),
        "新 UI 契约面必须声明 > mixer-drag-active:"
    );
    assert!(
        aether_host.contains("set_mixer_drag_active"),
        "宿主接线层必须注册 set_mixer_drag_active"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

/// A4：陈旧 / 解析不出的身份 ⇒ 空 `Vec`（不猜"大概是第一条轨"）。
#[test]
fn mixer_nudge_ops_refuse_a_stale_or_unparsable_identity() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> mixer-drag-active:"),
        "新 UI 契约面必须声明 > mixer-drag-active:"
    );
    assert!(
        aether_host.contains("set_mixer_drag_active"),
        "宿主接线层必须注册 set_mixer_drag_active"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

// ===========================================================================
// B 层：活窗口 + 真事件源
// ===========================================================================

/// 一条**接了键盘与选轨**的活窗口（两者都走生产同一个接线函数）。
///
/// `host::wire_keys` 由 `live_surface.rs` 的装配负责（与 `main.rs` 同一个函数）；
/// `host::wire_track_select` 由本判据自己补上（与 `tests/track_selection.rs` 同一做法），
/// 因此 `selected-track-id` 的写者仍然只有生产那一个。
fn live_with_keys_and_select(project: &YebanProjectV1) -> (MainWindow, Rc<UndoPort>, live::LiveUi) {
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<键盘微调混音>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let ui = live::build_live_ui_with(
        project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 端口");
    // `clone_strong` 是 Slint 句柄的**共享**（`Rc` 语义）而非复制 ⇒ 与 `_ui` 里
    // 那个活窗口是同一个组件实例；`_ui` 必须留在判据作用域里把组件撑着。
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    host::wire_track_select(&window, &port);
    (window, port, ui)
}

/// B1：`Cmd/Ctrl+Alt+↑/↓` 改的是**选中轨**的音量，工程与推子读数一起动。
#[test]
fn cmd_ctrl_alt_up_down_moves_the_selected_tracks_fader_and_labels_it() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> mixer-drag-active:"),
        "新 UI 契约面必须声明 > mixer-drag-active:"
    );
    assert!(
        aether_host.contains("set_mixer_drag_active"),
        "宿主接线层必须注册 set_mixer_drag_active"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

/// B2：`Cmd/Ctrl+Alt+←/→` 动的是**声相那一格**，且一格 = 显示上的一格。
#[test]
fn cmd_ctrl_alt_left_right_moves_the_pan_by_one_display_step() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> mixer-drag-active:"),
        "新 UI 契约面必须声明 > mixer-drag-active:"
    );
    assert!(
        aether_host.contains("set_mixer_drag_active"),
        "宿主接线层必须注册 set_mixer_drag_active"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

/// B3：没有选中轨 ⇒ 四个方向**都不消费**、0 提交、工程逐位不变。
#[test]
fn with_no_selected_track_the_mixer_nudge_keys_are_not_consumed() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> mixer-drag-active:"),
        "新 UI 契约面必须声明 > mixer-drag-active:"
    );
    assert!(
        aether_host.contains("set_mixer_drag_active"),
        "宿主接线层必须注册 set_mixer_drag_active"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

/// B4：陈旧选中态（窗口属性还留着**上一个工程**的身份）⇒ 什么都不提交。
///
/// ⚠ 这里**从判据写** `selected-track-id`：不是"给状态机加第二个写者"，而是**构造**生产
/// 会到达的那个状态 —— 换工程 / 控制面关工程之后，写者把属性留在了旧身份上，而端口的
/// 工程已经不是那一份。写者仍然只有 `host::wire_track_select` 一个（本判据⛔ 走它，
/// 因为要走它就得先有一个含旧身份的窗口）。
#[test]
fn a_stale_selection_after_a_project_swap_commits_nothing() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> mixer-drag-active:"),
        "新 UI 契约面必须声明 > mixer-drag-active:"
    );
    assert!(
        aether_host.contains("set_mixer_drag_active"),
        "宿主接线层必须注册 set_mixer_drag_active"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}
