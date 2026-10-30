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
//! | B3 | `with_no_selected_track_the_mixer_nudge_keys_are_not_consumed` | 四个方向各按一次 ⇒ 四次都 `false`、`commit_count` 与 `undoable` 恒为 0、工程读数逐位不变（**不吞键**：留给焦点系统） |
//! | B4 | `a_stale_selection_after_a_project_swap_commits_nothing` | 选中态是**窗口属性**（换工程之后它还留着上一个工程的身份）⇒ 按四向全部 `false`、0 提交 |
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
//! - **静音 / 独奏的键盘入口**：布尔开关没有"一格"可言，本动作族刻意不含它
//!   （见 `crate::input::MixerParam` 的文档）；
//! - **引擎听到变化**：编辑 ⇒ 新快照发布是 16 ms 心跳那一条腿的事
//!   （`main.rs` 的 `run_gui` 调 `EngineHost::publish_project`），本文件不测音频。

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
    // 数值单位：dB（音量）与 -1.0..=1.0 的比值（声相）。这两个字面量是全仓唯一的定义点。
    assert_eq!(host::mixer_nudge_delta_for(Volume, 1), 1.0);
    assert_eq!(host::mixer_nudge_delta_for(Volume, -1), -1.0);
    assert_eq!(host::mixer_nudge_delta_for(Pan, 1), 0.1);
    assert_eq!(host::mixer_nudge_delta_for(Pan, -1), -0.1);
    // `direction` 只取符号 ⇒ 0 是"没有方向"（增量 0.0），越界的整数不会放大步长。
    assert_eq!(host::mixer_nudge_delta_for(Volume, 0), 0.0);
    assert_eq!(host::mixer_nudge_delta_for(Pan, 0), 0.0);
    assert_eq!(host::mixer_nudge_delta_for(Volume, 7), 1.0);
    assert_eq!(host::mixer_nudge_delta_for(Pan, -7), -0.1);
}

/// A2：恰好一条 `Op::SetParam`、寻址到**被选中那轨**、`old_val` 与现值**逐位**相同。
#[test]
fn mixer_nudge_ops_target_the_selected_track_with_bitwise_old_val() {
    use yeban_model::AutomationTarget;

    let project = filled_project();
    let track = lead(&project);
    let id_text = track.id.to_canonical_string();

    let ops = host::mixer_nudge_ops_for(&project, &id_text, Volume, 1);
    let (target, old_val, new_val) = set_param_of(&ops);
    assert_eq!(target, AutomationTarget::TrackVolume { track_id: track.id });
    assert_eq!(
        old_val.to_bits(),
        (-3.0_f32).to_bits(),
        "`old_val` 必须是工程现值"
    );
    assert_eq!(
        new_val.to_bits(),
        (-2.0_f32).to_bits(),
        "-3.0 + 1.0 在 f32 下逐位是 -2.0"
    );

    let ops = host::mixer_nudge_ops_for(&project, &id_text, Pan, 1);
    let (target, old_val, new_val) = set_param_of(&ops);
    assert_eq!(target, AutomationTarget::TrackPan { track_id: track.id });
    assert_eq!(old_val.to_bits(), (-0.25_f32).to_bits());
    // `-0.25 + 0.1` 不是逐位的 `-0.15`（`0.1` 在 f32 里不可精确表示）⇒ 这里断言
    // **显示格**（千分之一取整）而不是尾数：`-0.25 → -0.15` 就是 `L25 → L15`。
    assert_eq!(
        (f64::from(new_val) * 1000.0).round() as i32,
        -150,
        "一格声相必须正好动一个显示格"
    );
    assert!(new_val > old_val, "右 = 数值增大（声相负值在左）");

    // 反方向回到起点附近：两次的**符号**相反，但⛔ 不承诺"逐位回原值"
    // （那是 `+` 与 `-` 各算一次夹紧的两次加法，不是恒等）。
    let back = host::mixer_nudge_ops_for(&project, &id_text, Pan, -1);
    let (_, _, down) = set_param_of(&back);
    assert_eq!((f64::from(down) * 1000.0).round() as i32, -350);
}

/// A3：两端夹紧 ⇒ 端点上返回**空**；夹的是**界面量程**。
#[test]
fn mixer_nudge_ops_clamp_at_the_ends_of_what_the_fader_can_show() {
    use yeban_model::AutomationTarget;

    // 推子能表示的区间（`bridge::FADER_MIN_DB` / `FADER_MAX_DB`）。
    assert!(
        host::mixer_nudge_ops_for(
            &with_lead_volume(filled_project(), 6.0),
            &lead(&filled_project()).id.to_canonical_string(),
            Volume,
            1
        )
        .is_empty(),
        "已经在 +6.0 dB（推子顶）⇒ 再按 `+` 什么都不提交"
    );
    let at_top = with_lead_volume(filled_project(), 6.0);
    let top_id = lead(&at_top).id.to_canonical_string();
    let ops = host::mixer_nudge_ops_for(&at_top, &top_id, Volume, -1);
    let (target, _, new_val) = set_param_of(&ops);
    assert_eq!(
        target,
        AutomationTarget::TrackVolume {
            track_id: lead(&at_top).id
        }
    );
    assert_eq!(new_val.to_bits(), 5.0_f32.to_bits());

    let at_bottom = with_lead_volume(filled_project(), -60.0);
    let bottom_id = lead(&at_bottom).id.to_canonical_string();
    assert!(
        host::mixer_nudge_ops_for(&at_bottom, &bottom_id, Volume, -1).is_empty(),
        "已经在 -60.0 dB（推子底）⇒ 再按 `-` 什么都不提交"
    );
    let ops = host::mixer_nudge_ops_for(&at_bottom, &bottom_id, Volume, 1);
    let (_, _, up) = set_param_of(&ops);
    assert_eq!(up.to_bits(), (-59.0_f32).to_bits());

    // ⭐ 界面量程而不是模型量程：`-80.0 dB` 在**模型**里合法（`validate_param_value`
    // 对音量只查有限值），但推子画不出、`accessible-value-minimum` 也不覆盖它。
    // 往下按 ⇒ 带回推子底端 `-60.0`，⛔ 不是继续往 `-79.0` 走（那会让"键盘改出来的数"
    // 与"鼠标改出来的数"有两种形状）。
    let below = with_lead_volume(filled_project(), -80.0);
    let below_id = lead(&below).id.to_canonical_string();
    let ops = host::mixer_nudge_ops_for(&below, &below_id, Volume, -1);
    let (_, old_val, new_val) = set_param_of(&ops);
    assert_eq!(old_val.to_bits(), (-80.0_f32).to_bits());
    assert_eq!(
        new_val.to_bits(),
        (-60.0_f32).to_bits(),
        "键盘不许造出推子表达不了的数"
    );

    // 声相两端（模型的 `PanOutOfRange` 就挡在这里）。
    let pan_right = with_lead_pan(filled_project(), 1.0);
    let pan_id = lead(&pan_right).id.to_canonical_string();
    assert!(host::mixer_nudge_ops_for(&pan_right, &pan_id, Pan, 1).is_empty());
    assert!(host::mixer_nudge_ops_for(&pan_right, &pan_id, Pan, -1).len() == 1);
    let pan_left = with_lead_pan(filled_project(), -1.0);
    let left_id = lead(&pan_left).id.to_canonical_string();
    assert!(host::mixer_nudge_ops_for(&pan_left, &left_id, Pan, -1).is_empty());
    let (_, _, to_center) = set_param_of(&host::mixer_nudge_ops_for(&pan_left, &left_id, Pan, 1));
    assert_eq!((f64::from(to_center) * 1000.0).round() as i32, -900);
}

/// A4：陈旧 / 解析不出的身份 ⇒ 空 `Vec`（不猜"大概是第一条轨"）。
#[test]
fn mixer_nudge_ops_refuse_a_stale_or_unparsable_identity() {
    let project = filled_project();

    // ① 文本压根不是 `EntityId`：空串（= 窗口属性里"没有明确选中"那一形态）、
    //    含非法字符的串、以及**长度不对**的串（`ulid::Ulid::from_string` 只认 26 字符）。
    for junk in [
        String::new(),
        "not-a-ulid".to_owned(),
        "A".repeat(25),
        "A".repeat(27),
    ] {
        assert!(
            host::mixer_nudge_ops_for(&project, &junk, Volume, 1).is_empty(),
            "解析不出的身份文本 {junk:?} 必须返回空"
        );
    }

    // ② 文本是**良构**身份但不在本工程里（换工程之后 `selected-track-id` 留着上一个
    //    工程的身份，正是这一形态）。
    let ghost = EntityId::new().to_canonical_string();
    assert_eq!(ghost.len(), 26, "`EntityId` 的规范文本是 26 字符");
    // ⚠ 这里**不用** `.all()`：空集上 `all` 恒真 ⇒ "ghost 不在任何轨道里"会在夹具为空时
    // 静默通过（本 crate 的常驻判据 `assertion_bounds_audit.rs` 管的就是这件事）。
    // 换成"集合本身有界 ＋ `contains`"两句合一 ⇒ 夹具为空也会如实红。
    let ids: Vec<String> = project
        .tracks
        .values()
        .map(|track| track.id.to_canonical_string())
        .collect();
    assert!(
        !ids.is_empty() && !ids.contains(&ghost),
        "夹具的轨道身份 {ids:?} 必须非空、且不含这个域外身份 {ghost}"
    );
    assert!(host::mixer_nudge_ops_for(&project, &ghost, Volume, 1).is_empty());
    assert!(host::mixer_nudge_ops_for(&project, &ghost, Pan, 1).is_empty());

    // ③ `direction == 0` ⇒ 增量 0 ⇒ 夹紧后与现值逐位相同 ⇒ 空（与 A1 的符号口径闭环）。
    let id_text = lead(&project).id.to_canonical_string();
    assert!(host::mixer_nudge_ops_for(&project, &id_text, Volume, 0).is_empty());
    assert!(host::mixer_nudge_ops_for(&project, &id_text, Pan, 0).is_empty());
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
    let project = filled_project();
    let lead_id = lead(&project).id.to_canonical_string();
    let (window, port, _ui) = live_with_keys_and_select(&project);

    // 起点读数（权威 + 显示两处一致，否则后面的断言没有意义）。
    assert_eq!(
        lead(&port.try_project().expect("工程")).volume_db.to_bits(),
        (-3.0_f32).to_bits()
    );
    assert_eq!(string_column(&window.get_track_volumes())[0], "-3.0");
    assert_eq!(port.display().undoable, 0);
    assert_eq!(port.display().commit_count, 0);

    // 用**生产的点击源**选中第一条轨（不是从判据里写属性）。
    window.invoke_track_select(0);
    assert_eq!(window.get_selected_track_id().to_string(), lead_id);

    // `Ctrl+Alt+↑`（Linux 的 `Ctrl`；macOS 的 `Cmd` 由 `input.rs` 的判据覆盖）。
    assert!(
        window.invoke_key_action(UP.into(), false, true, true, false),
        "`Cmd/Ctrl+Alt+↑` 必须被消费"
    );
    let after = port.try_project().expect("工程");
    assert_eq!(
        lead(&after).volume_db.to_bits(),
        (-2.0_f32).to_bits(),
        "一格 = +1.0 dB"
    );
    assert_eq!(
        string_column(&window.get_track_volumes())[0],
        "-2.0",
        "推子读数跟着改"
    );
    assert_eq!(port.display().undoable, 1, "一次按键 = 一步撤销");
    assert_eq!(port.display().commit_count, 1);
    assert!(!window.get_mixer_drag_active(), "键盘这一路不留混音手势");

    // 邻居不受影响（作用域 = 选中的那一条）。
    let others = string_column(&window.get_track_volumes());
    assert_eq!(others[1], "0.0", "未选中的轨一格都没动");

    // 再按一次 `↓` ⇒ -3.0，撤销栈走第二步（离散按键各算一步）。
    assert!(window.invoke_key_action(DOWN.into(), false, true, true, false));
    assert_eq!(
        lead(&port.try_project().expect("工程")).volume_db.to_bits(),
        (-3.0_f32).to_bits()
    );
    assert_eq!(port.display().undoable, 2);

    // 多按一个 `Shift` 的组合**未被点名** ⇒ 如实放行（不消费、不提交）。
    assert!(
        !window.invoke_key_action(UP.into(), true, true, true, false),
        "`Cmd/Ctrl+Alt+Shift+↑` 未绑定 ⇒ 不得消费"
    );
    assert_eq!(port.display().commit_count, 2, "放行的键不产生提交");

    // `Cmd/Ctrl+Z` 撤掉刚才那一步 ⇒ 回到 -2.0 那一格的**前**一个状态（-3.0 之后的 -3.0…
    // 逐步核对：栈是 [−2.0, −3.0]，撤一步回到 −2.0）。
    assert!(window.invoke_key_action("z".into(), false, false, false, true));
    assert_eq!(
        lead(&port.try_project().expect("工程")).volume_db.to_bits(),
        (-2.0_f32).to_bits(),
        "撤销必须逐位回到上一格"
    );
    assert_eq!(
        string_column(&window.get_track_volumes())[0],
        "-2.0",
        "撤销之后重投影"
    );

    report_line(&format!(
        "[mixer-keyboard-nudge-volume] -3.0 → -2.0 → -3.0 dB / undoable=2 / commit=2 / \
         邻居 {}/{} 格未动 / 撤一步回到 -2.0",
        others.len() - 1,
        others.len() - 1
    ));
}

/// B2：`Cmd/Ctrl+Alt+←/→` 动的是**声相那一格**，且一格 = 显示上的一格。
#[test]
fn cmd_ctrl_alt_left_right_moves_the_pan_by_one_display_step() {
    let project = filled_project();
    let (window, port, _ui) = live_with_keys_and_select(&project);
    window.invoke_track_select(0);

    assert_eq!(string_column(&window.get_track_pans())[0], "L25");
    let volume_before = string_column(&window.get_track_volumes())[0].clone();

    assert!(window.invoke_key_action(RIGHT.into(), false, true, true, false));
    assert_eq!(
        string_column(&window.get_track_pans())[0],
        "L15",
        "一格声相（0.1）在显示上恰好动一格：L25 → L15"
    );
    let after = port.try_project().expect("工程");
    assert_eq!((f64::from(lead(&after).pan) * 1000.0).round() as i32, -150);
    assert_eq!(
        string_column(&window.get_track_volumes())[0],
        volume_before,
        "声相臂⛔ 不许顺手写音量那一格"
    );
    assert_eq!(port.display().undoable, 1);

    assert!(window.invoke_key_action(LEFT.into(), false, true, true, false));
    assert_eq!(
        string_column(&window.get_track_pans())[0],
        "L25",
        "反向一格回到 L25"
    );
    assert_eq!(port.display().undoable, 2);

    report_line(&format!(
        "[mixer-keyboard-nudge-pan] L25 → L15 → L25 / 音量列保持 {} / undoable=2",
        volume_before
    ));
}

/// B3：没有选中轨 ⇒ 四个方向**都不消费**、0 提交、工程逐位不变。
#[test]
fn with_no_selected_track_the_mixer_nudge_keys_are_not_consumed() {
    let project = filled_project();
    let (window, port, _ui) = live_with_keys_and_select(&project);
    assert!(
        window.get_selected_track_id().is_empty(),
        "启动态没有明确选中"
    );

    let before = port.try_project().expect("工程");
    let volume_before = lead(&before).volume_db.to_bits();
    let pan_before = lead(&before).pan.to_bits();

    for key in [UP, DOWN, LEFT, RIGHT] {
        assert!(
            !window.invoke_key_action(key.into(), false, true, true, false),
            "`{key:?}` 在没有选中轨时必须放行（⛔ 不吞键、不假装改了什么）"
        );
    }
    let after = port.try_project().expect("工程");
    assert_eq!(lead(&after).volume_db.to_bits(), volume_before);
    assert_eq!(lead(&after).pan.to_bits(), pan_before);
    assert_eq!(port.display().commit_count, 0);
    assert_eq!(port.display().undoable, 0);

    report_line("[mixer-keyboard-nudge-empty] 四向各按一次 ⇒ 全部 reject / 0 提交 / 读数逐位不变");
}

/// B4：陈旧选中态（窗口属性还留着**上一个工程**的身份）⇒ 什么都不提交。
///
/// ⚠ 这里**从判据写** `selected-track-id`：不是"给状态机加第二个写者"，而是**构造**生产
/// 会到达的那个状态 —— 换工程 / 控制面关工程之后，写者把属性留在了旧身份上，而端口的
/// 工程已经不是那一份。写者仍然只有 `host::wire_track_select` 一个（本判据⛔ 走它，
/// 因为要走它就得先有一个含旧身份的窗口）。
#[test]
fn a_stale_selection_after_a_project_swap_commits_nothing() {
    let project = filled_project();
    let (window, port, _ui) = live_with_keys_and_select(&project);

    let ghost = EntityId::new().to_canonical_string();
    // 同上：界写在集合本身（`ids.is_empty()`），"不含 ghost"用 `contains` 判 —— 空夹具
    // 会让 `.all()` 恒真，这条判据也就白给。
    let ids: Vec<String> = project
        .tracks
        .values()
        .map(|track| track.id.to_canonical_string())
        .collect();
    assert!(
        !ids.is_empty() && !ids.contains(&ghost),
        "夹具的轨道身份 {ids:?} 必须非空、且不含这个域外身份 {ghost}"
    );
    window.set_selected_track_id(ghost.clone().into());

    let before = lead(&port.try_project().expect("工程")).volume_db.to_bits();
    for key in [UP, DOWN, LEFT, RIGHT] {
        assert!(
            !window.invoke_key_action(key.into(), false, true, true, false),
            "身份 {ghost} 不在工程里 ⇒ `{key:?}` 必须 reject（不猜第一条轨）"
        );
    }
    assert_eq!(
        lead(&port.try_project().expect("工程")).volume_db.to_bits(),
        before,
        "陈旧身份下工程一个字节都不许动"
    );
    assert_eq!(port.display().commit_count, 0);
    assert_eq!(string_column(&window.get_track_volumes())[0], "-3.0");

    report_line("[mixer-keyboard-nudge-stale] 陈旧选中身份 ⇒ 四向 reject / 0 提交 / 读数不变");
}
