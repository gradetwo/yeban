//! 裸 `M` / `S` ⇒ 拨动**当前选中轨**的静音 / 独奏。
//!
//! ⚠ **这不是规范点名的键位**：`[UI-NOTE-005]` §3.5（标题"全键盘音符微调与无鼠标心流"）
//! 的四条项**只点名音符**；该文档 2026-10-06 经负责人裁决降级为**参考**，非绑定规范。
//! 本切片是**界面决定**（补"静音 / 独奏只能靠鼠标点"这一缺口），登记在
//! `docs/ledger/app-mixer-notes.md` §7 第 10 行的 ② 那半句。键位的选法只受一条约束：
//! ⛔ 不覆盖任何已被规范点名、或已被既有判据如实钉成放行的组合 —— 裸 `m` / `s` 此前都落到
//! `resolve_logical` 第 6 步的 `_ => PassThrough`（`Cmd/Ctrl+Alt+M` 是"最大化控制台"，另一格），
//! 理由逐条写在 [`yeban_app::input::Action::ToggleTrackMute`] 的文档里。
//!
//! ## 与鼠标开关走**同一条**提交通道（台账 R17 的延伸）
//!
//! | 入口 | 接线 | 提交 |
//! | :--- | :--- | :--- |
//! | 调音台通道条 / 编曲视图轨道头 | `wire_mixer_switch` | `Op::SetTrackMute` / `SetTrackSolo`，标签来自 `mixer_switch_commit_message` |
//! | 键盘 `M` / `S` | `apply_action` 的 `ToggleTrackMute \| ToggleTrackSolo` 臂 | **同一个 `Op`**、**同一条**标签（同一个函数） |
//!
//! ⇒ 撤销树里没有"点出来的"与"按出来的"两种形状。标签本身不带来源视图名，
//! 那条诚实的判据住在 `tests/mixer_toggle_labels.rs`（它同时断言键盘臂**经**共用定义点取标签）。
//!
//! ## 两层（不可互相代替）
//!
//! | 层 | 文件内位置 | 证的事 | 读的是谁 |
//! | :--- | :--- | :--- | :--- |
//! | **A 纯函数** | 前两条判据 | "拨哪个字段 / 什么时候什么都不提交"这两条**决定**各自只有一处定义 | 直接调 [`yeban_app::host::toggle_track_switch_ops_for`] |
//! | **B 活窗口** | 后三条判据 | 真事件源（`key-action` 逻辑键）⇒ 真解析 ⇒ 真宿主臂 ⇒ 真 `commit_ops` ⇒ 权威工程与开关列**一起**动 | `UndoPort::try_project()`（权威）＋ `MainWindow.track-mutes` / `track-solos`（用户看见的勾选态） |
//!
//! 只看 A 会绿在"臂没接、键没解析出来"；只看 B 会绿在"op 被写了两份、第二份偷偷改了目标"。
//!
//! ## 判据表
//!
//! | # | 判据 | 断言 |
//! | :--- | :--- | :--- |
//! | A1 | `toggle_ops_are_one_op_that_flips_the_named_field` | 恰好**一条** `Op::SetTrackMute` / `SetTrackSolo`；寻址**被选中那轨**；`new == !old`（从已静音 / 已独奏的现值拨回 `false` 也成立） |
//! | A2 | `toggle_ops_refuse_a_stale_or_unparsable_identity` | 身份文本解析不出 `EntityId` / 解析得出但**不在**工程里 ⇒ 空 `Vec`（两个开关同口径） |
//! | B1 | `bare_m_and_s_toggle_only_their_own_switch_on_the_selected_track` | 真 `key-action`（裸 `m` / `s`）⇒ 工程 `mute` / `solo` 翻面、开关列第 0 格跟着翻；`undoable` 每按一次 **+1**；`m` ⛔ 不动独奏列、`s` ⛔ 不动静音列；邻居一格未动；`Cmd+Z` 逐步撤回 |
//! | B2 | `with_no_selected_track_the_mute_and_solo_keys_are_not_consumed` | 两键都 `false`、`commit_count` **停在起点**、`undoable` 恒 0、工程与两列逐位不变（**不吞键**） |
//! | B3 | `a_stale_selection_after_a_project_swap_commits_nothing` | 选中态是**窗口属性**（换工程之后还留着上一个工程的身份）⇒ 两键都 `false`、`commit_count` 停在起点 |
//!
//! ⭐ **提交的口径**：`UndoDisplay::commit_count` **含那一条根提交** —— 全新会话
//! （`UndoSession::open`）打开就记 1（`src/undo.rs` 的判据 `a_fresh_port_has_no_history`
//! 用**同一个夹具** `filled_project()` 钉住）。因此 B 层一律**相对起点**断言（"一次按键 =
//! 一条提交"读作 Δ1），⛔ 不写 `0` 这个字面量。`undoable` 是另一个口径（可撤销的**步数**，
//! 全新会话 = 0），仍按绝对值断。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 文件 | 预期 |
//! | :--- | :--- | :--- |
//! | 静音臂里把 `!track.mute` 写成 `track.mute` | `src/host.rs` | A1 红（`new == old`）＋ B1 红（读数不动，模型会拒绝这条空 op） |
//! | 独奏臂里把 `TrackSolo` 换成 `TrackMute` | `src/host.rs` | A1 红（写错字段）＋ B1 红（静音列跟着动） |
//! | 把 `selected_track_id(ui)` 的早退删掉、改成"没有选中就动第一条轨" | `src/host.rs` | B2 红（两键变成消费） |
//! | 把 `toggle_track_switch_ops_for` 的身份解析换成"取第一条轨" | `src/host.rs` | A2 红 ＋ B3 红（陈旧身份也消费） |
//! | 删掉 `resolve_logical` 第 6 步的 `Key::Character('m')` 臂 | `src/input.rs` | B1 红（键不消费）＋ `input.rs` 自己的判据红 |
//! | 在 `cli.rs` 把 `S` 那行的 `direction`/动作写成 `ToggleTrackMute` | `src/cli.rs` | `tests/cli_contract.rs` 的 B11b ②红（标签绑的键解析出的动作 ≠ 行声称的动作） |
//!
//! ## ⛔ 本文件不证的东西
//!
//! - **主控通道条**：主总线不在 `track-ids` 里 ⇒ 没有点击源能写 `selected-track-id`，
//!   键盘这一路今天**碰不到**主控（鼠标的主控开关另有 `mixer-master-*-toggle` 与
//!   `tests/live_ui_mcp.rs` 的判据）。这条边界登记在 `docs/ledger/app-mixer-notes.md`；
//! - **连续量的键盘入口**：音量 / 声相的一格由 `tests/mixer_keyboard_nudge.rs` 覆盖，
//!   本文件只碰布尔开关（没有"一格"可言）；
//! - **引擎听到变化**：编辑 ⇒ 新快照发布是 16 ms 心跳那一条腿的事，本文件不测音频。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `roll_edit_tools.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use yeban_app::host::{self, MixerSwitch};
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::project::{TrackV3, YebanProjectV1};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 会话打开时刻（与其余界面判据同一个常量，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 裸键的文本形态 —— `logical()` 把它们映射成按键字符，`LogicalKey::from_text` 再还原成
/// `Character('m' / 's')`。用文本而不是私有码点：这两个键在 `.slint` 的事件里就是字母本身。
const M: &str = "m";
const S: &str = "s";

/// `filled_project()` 里名字为 `Lead` 的那条轨（`mute = false`、`solo = false`）。
fn lead(project: &YebanProjectV1) -> TrackV3 {
    project
        .tracks
        .values()
        .find(|track| track.name == "Lead")
        .cloned()
        .expect("夹具里必须有名为 Lead 的轨")
}

/// 把 `Lead` 的静音 / 独奏改成给定值（A1 要造"已静音 / 已独奏"的现值）。
fn with_lead_mute(mut project: YebanProjectV1, mute: bool) -> YebanProjectV1 {
    let id = lead(&project).id;
    project.tracks.get_mut(&id).expect("Lead 必须在工程里").mute = mute;
    project
}

fn with_lead_solo(mut project: YebanProjectV1, solo: bool) -> YebanProjectV1 {
    let id = lead(&project).id;
    project.tracks.get_mut(&id).expect("Lead 必须在工程里").solo = solo;
    project
}

/// 从 `Op::SetTrackMute` 里取 `(轨身份, old_mute, new_mute)`；不是这个形状就 `panic`
/// （判据只喂 `toggle_track_switch_ops_for` 的产物，形状不符就是实现写错了）。
fn mute_of(ops: &[yeban_model::Op]) -> (EntityId, bool, bool) {
    let [
        yeban_model::Op::SetTrackMute {
            track_id,
            old_mute,
            new_mute,
        },
    ] = ops
    else {
        panic!("期望恰好一条 `Op::SetTrackMute`，得到 {ops:?}");
    };
    (*track_id, *old_mute, *new_mute)
}

/// 从 `Op::SetTrackSolo` 里取 `(轨身份, old_solo, new_solo)`。
fn solo_of(ops: &[yeban_model::Op]) -> (EntityId, bool, bool) {
    let [
        yeban_model::Op::SetTrackSolo {
            track_id,
            old_solo,
            new_solo,
        },
    ] = ops
    else {
        panic!("期望恰好一条 `Op::SetTrackSolo`，得到 {ops:?}");
    };
    (*track_id, *old_solo, *new_solo)
}

/// 读一条 `in property <[bool]>` 的整列（开关的**勾选态**，用户看见的那一格）。
fn bool_column(model: &slint::ModelRc<bool>) -> Vec<bool> {
    use slint::Model as _;
    model.iter().collect()
}

// ===========================================================================
// A 层：纯函数
// ===========================================================================

/// A1：恰好一条 op、寻址**被选中那轨**、`new == !old`（两个开关各证一次）。
#[test]
fn toggle_ops_are_one_op_that_flips_the_named_field() {
    let project = filled_project();
    let track = lead(&project);
    let id_text = track.id.to_canonical_string();
    assert!(
        !track.mute && !track.solo,
        "夹具起点：Lead 既未静音也未独奏"
    );

    let (target, old_mute, new_mute) = mute_of(&host::toggle_track_switch_ops_for(
        &project,
        &id_text,
        MixerSwitch::Mute,
    ));
    assert_eq!(target, track.id, "静音必须落在被选中那轨上");
    assert!(!old_mute, "`old_mute` 必须是工程现值");
    assert!(new_mute, "拨动 = 取反");

    let (target, old_solo, new_solo) = solo_of(&host::toggle_track_switch_ops_for(
        &project,
        &id_text,
        MixerSwitch::Solo,
    ));
    assert_eq!(target, track.id, "独奏必须落在被选中那轨上");
    assert!(!old_solo, "`old_solo` 必须是工程现值");
    assert!(new_solo, "拨动 = 取反");

    // 从**已打开**的现值拨回 `false`：`!old` 而不是写死 `true`。
    let muted = with_lead_mute(filled_project(), true);
    let muted_id = lead(&muted).id.to_canonical_string();
    let (_, old_mute, new_mute) = mute_of(&host::toggle_track_switch_ops_for(
        &muted,
        &muted_id,
        MixerSwitch::Mute,
    ));
    assert!(old_mute && !new_mute, "已静音 ⇒ 这一键应关掉它");

    let soloed = with_lead_solo(filled_project(), true);
    let soloed_id = lead(&soloed).id.to_canonical_string();
    let (_, old_solo, new_solo) = solo_of(&host::toggle_track_switch_ops_for(
        &soloed,
        &soloed_id,
        MixerSwitch::Solo,
    ));
    assert!(old_solo && !new_solo, "已独奏 ⇒ 这一键应关掉它");
}

/// A2：陈旧 / 解析不出的身份 ⇒ 空 `Vec`（不猜"大概是第一条轨"）。
#[test]
fn toggle_ops_refuse_a_stale_or_unparsable_identity() {
    let project = filled_project();

    // ① 文本压根不是 `EntityId`：空串（= 窗口属性里"没有明确选中"那一形态）、
    //    含非法字符的串、以及**长度不对**的串（`ulid::Ulid::from_string` 只认 26 字符）。
    for junk in [
        String::new(),
        "not-a-ulid".to_owned(),
        "A".repeat(25),
        "A".repeat(27),
    ] {
        for switch in [MixerSwitch::Mute, MixerSwitch::Solo] {
            assert!(
                host::toggle_track_switch_ops_for(&project, &junk, switch).is_empty(),
                "解析不出的身份文本 {junk:?}（{switch:?}）必须返回空"
            );
        }
    }

    // ② 文本是**良构**身份但不在本工程里（换工程之后 `selected-track-id` 留着上一个
    //    工程的身份，正是这一形态）。
    let ghost = EntityId::new().to_canonical_string();
    assert_eq!(ghost.len(), 26, "`EntityId` 的规范文本是 26 字符");
    // ⚠ 不用 `.all()`：空集上恒真 ⇒ 夹具为空时会静默通过（`assertion_bounds_audit.rs` 管的就是这件事）。
    let ids: Vec<String> = project
        .tracks
        .values()
        .map(|track| track.id.to_canonical_string())
        .collect();
    assert!(
        !ids.is_empty() && !ids.contains(&ghost),
        "夹具的轨道身份 {ids:?} 必须非空、且不含这个域外身份 {ghost}"
    );
    for switch in [MixerSwitch::Mute, MixerSwitch::Solo] {
        assert!(
            host::toggle_track_switch_ops_for(&project, &ghost, switch).is_empty(),
            "域外身份 {ghost}（{switch:?}）必须返回空"
        );
    }
}

// ===========================================================================
// B 层：活窗口 + 真事件源
// ===========================================================================

/// 一条**接了键盘与选轨**的活窗口（两者都走生产同一个接线函数）。
fn live_with_keys_and_select(project: &YebanProjectV1) -> (MainWindow, Rc<UndoPort>, live::LiveUi) {
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<键盘拨动静音独奏>", "yeban-app", project.clone(), NOW_MS)
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

/// B1：裸 `m` 只动静音、裸 `s` 只动独奏；每次按键 = 一步撤销；`Cmd+Z` 逐步撤回。
#[test]
fn bare_m_and_s_toggle_only_their_own_switch_on_the_selected_track() {
    let project = filled_project();
    let lead_id = lead(&project).id.to_canonical_string();
    let (window, port, _ui) = live_with_keys_and_select(&project);

    // 起点读数：权威与两列一致，否则后面的断言没有意义。
    let start = lead(&port.try_project().expect("工程"));
    assert!(!start.mute && !start.solo);
    let mutes_at_start = bool_column(&window.get_track_mutes());
    let solos_at_start = bool_column(&window.get_track_solos());
    assert_eq!(
        mutes_at_start.len(),
        3,
        "夹具的通道条数（有界：改了 `filled_project()` 的轨数时**这里先红**）: {mutes_at_start:?}"
    );
    assert_eq!(solos_at_start.len(), 3);
    // ⚠ 用 `contains` 而不是 `.all(..)`：空集上 `.all` 恒真，而本 crate 的常驻判据
    // `assertion_bounds_audit.rs` 要求断言里的 `.all(` 站点有界并登记文件；这里
    // "整列都未静音"用**有界**（上面刚断过 `len == 3`）的 `contains` 表达同一件事。
    assert!(!mutes_at_start.contains(&true), "起点：整列都未静音");
    assert!(!solos_at_start.contains(&true), "起点：整列都未独奏");
    assert_eq!(port.display().undoable, 0);
    let commits_at_start = port.display().commit_count;
    assert_eq!(
        commits_at_start, 1,
        "起点：会话里只有那一条根提交（口径见 `src/undo.rs`）"
    );

    // 用**生产的点击源**选中第一条轨（不是从判据里写属性）。
    window.invoke_track_select(0);
    assert_eq!(window.get_selected_track_id().to_string(), lead_id);

    // 裸 `m` ⇒ 只翻静音。
    assert!(
        window.invoke_key_action(M.into(), false, false, false, false),
        "裸 `M` 必须有选中轨时被消费"
    );
    assert!(lead(&port.try_project().expect("工程")).mute, "静音翻面");
    assert_eq!(
        bool_column(&window.get_track_mutes()),
        {
            let mut expected = mutes_at_start.clone();
            expected[0] = true;
            expected
        },
        "开关列跟着翻，且只翻第 0 格"
    );
    assert_eq!(
        bool_column(&window.get_track_solos()),
        solos_at_start,
        "静音臂⛔ 不许顺手写独奏那一列"
    );
    assert_eq!(port.display().undoable, 1, "一次按键 = 一步撤销");
    assert_eq!(
        port.display().commit_count,
        commits_at_start + 1,
        "一次拨动 = 一条提交（相对起点 Δ1）"
    );
    assert!(!window.get_mixer_drag_active(), "键盘这一路不留混音手势");

    // 再按一次 `m` ⇒ 拨回去（第二次仍是**新的一条**提交，不是撤销）。
    assert!(window.invoke_key_action(M.into(), false, false, false, false));
    assert!(!lead(&port.try_project().expect("工程")).mute);
    assert_eq!(port.display().undoable, 2);

    // 裸 `s` ⇒ 只翻独奏，静音列保持起点（此刻它已经拨回 false）。
    assert!(
        window.invoke_key_action(S.into(), false, false, false, false),
        "裸 `S` 必须有选中轨时被消费"
    );
    assert!(lead(&port.try_project().expect("工程")).solo, "独奏翻面");
    assert_eq!(
        bool_column(&window.get_track_solos()),
        {
            let mut expected = solos_at_start.clone();
            expected[0] = true;
            expected
        },
        "独奏列跟着翻，且只翻第 0 格"
    );
    assert_eq!(
        bool_column(&window.get_track_mutes()),
        mutes_at_start,
        "独奏臂⛔ 不许顺手写静音那一列"
    );
    assert_eq!(port.display().undoable, 3);

    // `Cmd/Ctrl+Z` 撤掉独奏 ⇒ 回到"静音已拨回"的那一格。
    assert!(window.invoke_key_action("z".into(), false, false, false, true));
    assert!(
        !lead(&port.try_project().expect("工程")).solo,
        "撤销收回独奏"
    );
    assert_eq!(
        bool_column(&window.get_track_solos()),
        solos_at_start,
        "撤销之后重投影"
    );
    assert_eq!(port.display().undoable, 2);

    report_line(
        "[mixer-keyboard-switch] mute: false→true→false / solo: false→true→(撤销)false / \
         undoable=3→2 / Δcommit=3 / 各自列只动第 0 格",
    );
}

/// B2：没有选中轨 ⇒ 两键**都不消费**、0 提交、工程与两列逐位不变。
#[test]
fn with_no_selected_track_the_mute_and_solo_keys_are_not_consumed() {
    let project = filled_project();
    let (window, port, _ui) = live_with_keys_and_select(&project);
    assert!(
        window.get_selected_track_id().is_empty(),
        "启动态没有明确选中"
    );

    let before = lead(&port.try_project().expect("工程"));
    let mutes_at_start = bool_column(&window.get_track_mutes());
    let solos_at_start = bool_column(&window.get_track_solos());
    let commits_at_start = port.display().commit_count;

    for key in [M, S] {
        assert!(
            !window.invoke_key_action(key.into(), false, false, false, false),
            "`{key}` 在没有选中轨时必须放行（⛔ 不吞键、不假装改了什么）"
        );
    }
    let after = lead(&port.try_project().expect("工程"));
    assert_eq!(after.mute, before.mute);
    assert_eq!(after.solo, before.solo);
    assert_eq!(bool_column(&window.get_track_mutes()), mutes_at_start);
    assert_eq!(bool_column(&window.get_track_solos()), solos_at_start);
    assert_eq!(
        port.display().commit_count,
        commits_at_start,
        "没有选中轨 ⇒ 两键全部放行 ⇒ 相对起点一条提交都没多"
    );
    assert_eq!(port.display().undoable, 0);

    report_line("[mixer-keyboard-switch-empty] 两键各按一次 ⇒ 全部 reject / Δcommit=0 / 两列不变");
}

/// B3：陈旧选中态（窗口属性还留着**上一个工程**的身份）⇒ 什么都不提交。
///
/// ⚠ 这里**从判据写** `selected-track-id`：不是"给状态机加第二个写者"，而是**构造**生产
/// 会到达的那个状态 —— 换工程 / 控制面关工程之后，写者把属性留在了旧身份上，而端口的
/// 工程已经不是那一份。写者仍然只有 `host::wire_track_select` 一个。
#[test]
fn a_stale_selection_after_a_project_swap_commits_nothing() {
    let project = filled_project();
    let (window, port, _ui) = live_with_keys_and_select(&project);

    let ghost = EntityId::new().to_canonical_string();
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

    let before = lead(&port.try_project().expect("工程"));
    let commits_at_start = port.display().commit_count;
    for key in [M, S] {
        assert!(
            !window.invoke_key_action(key.into(), false, false, false, false),
            "身份 {ghost} 不在工程里 ⇒ `{key}` 必须 reject（不猜第一条轨）"
        );
    }
    let after = lead(&port.try_project().expect("工程"));
    assert_eq!(after.mute, before.mute);
    assert_eq!(after.solo, before.solo);
    assert_eq!(
        port.display().commit_count,
        commits_at_start,
        "陈旧身份 ⇒ 两键全部 reject ⇒ 相对起点一条提交都没多"
    );

    report_line("[mixer-keyboard-switch-stale] 陈旧选中身份 ⇒ 两键 reject / Δcommit=0 / 字段不变");
}
