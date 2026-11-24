//! **模型拒绝的可见出口必须统一**（T-FIX-303）。
//!
//! `report_rejected` 是 UI 侧"用户手势被模型拒绝"的唯一回显入口：提示条 ＋ 模块回执
//! 一起写（桌面双击启动的用户看不到 stderr）。本文件钉住**此前漏掉的六条路径**：
//!
//! | # | 路径 | 修前 | 修后 |
//! | :--- | :--- | :--- | :--- |
//! | ① | 轨道重命名 | `eprintln!` ＋ return | `report_rejected(ui, "轨道重命名")` |
//! | ② | Delete 删选区 | `return false` | `report_rejected(ui, "删除选区")` ＋ false |
//! | ③ | Cmd/Ctrl+D 复制选区 | `return false` | `report_rejected(ui, "复制选区")` ＋ false |
//! | ④ | Shift+←/→ 调整时长 | `return false` | `report_rejected(ui, "调整选区时长")` ＋ false |
//! | ⑤ | 裸 M / S 静音独奏 | `return false` | `report_rejected(ui, "静音/独奏切换")` ＋ false |
//! | ⑥ | 设备旁通预检 | 只 `eprintln!` | `report_rejected(ui, "旁通切换")` |
//!
//! ## 注入方式（替身）
//!
//! `UndoPort` 是**具体类型**（不是 trait），测试无法替换实现 ⇒ 本 crate 的
//! `test-support` feature（由自引用 dev-dependency 打开）给了一个**一次性**开关
//! [`UndoPort::refuse_next_commit_for_test`]：下一次 `commit_ops` 无条件返回
//! `UndoRefusal::Model`，且发生在任何状态改动之前（与真实模型拒绝的可观测形状一致）。
//! 开关在**生产构建里不存在**（`cargo tree -p yeban-app -e normal` 没有第二个 yeban-app）。
//!
//! ## 每条判据的两半
//!
//! 1. **拒绝分支**：注入 Err ⇒ 提示条 / 模块回执出现、措辞点名动作、`error == true`，
//!    且工程指纹与 `commit_count` 一位不动；
//! 2. **成功路径回归**：同一条手势再走一次（开关只用一次）⇒ 被消费、工程真的改了、
//!    `commit_count + 1`。
//!
//! ⚠ **有意不改**（与本单无关，逐条核实过）：空选区 / 身份不存在等"没有作用对象"的
//! `return false` 是既有设计（`actions that have no target`），它们**不是**模型拒绝 ——
//! 用户没选中东西时提示"被拒绝"反而误导。T-FIX-219/289 的模态快捷键守卫也不动。

#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::rc::Rc;
use std::str::FromStr as _;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::Model as _;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::ClipContent;
use yeban_model::{EntityId, YebanProjectV1};
use yeban_ui_test_port::port::Permission;

const NOW_MS: u64 = 1_760_000_000_000;
const DELETE_TEXT: &str = "\u{007f}";
const RIGHT_TEXT: &str = "\u{f703}";

/// 一条 Tier-1 活窗口 ＋ 撤销端口（生产同一个 `build_live_ui_with` 装配）。
fn assemble() -> (MainWindow, Rc<UndoPort>, live::LiveUi) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<T-FIX-303 rejection>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            console_tab: 2,
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 ＋ Tier-1 端口");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    (window, port, ui)
}

fn commit_count(port: &UndoPort) -> usize {
    port.display().commit_count
}

/// 提示条 ＋ 模块回执必须一起出现，且点名动作、标成错误。
fn assert_rejected(window: &MainWindow, action: &str) {
    let module = window.get_module_status_text();
    assert!(
        module.contains(action),
        "模块回执必须点名动作 `{action}`，实测 {module:?}"
    );
    assert!(
        !window.get_module_status_succeeded(),
        "模块回执必须标成失败态，实测文本 {module:?}"
    );
    let toast = window.get_toast_message();
    assert!(
        toast.contains(action),
        "提示条必须点名动作 `{action}`，实测 {toast:?}"
    );
    assert!(window.get_toast_is_error(), "提示条必须标成错误");
    assert!(window.get_show_floating_toast(), "提示条必须真的显示");
}

/// 唯一的合法但**不在工程里**的 ULID（26 位 canonical 形态）。
fn stale_ulid() -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{:020}", 999_999_999_u128)).expect("fixture ULID")
}

/// 工程里第一条 MIDI 音符的身份（`clip_pool` 的 BTreeMap 序，确定）。
fn first_note(project: &YebanProjectV1) -> (EntityId, u64) {
    for entry in project.clip_pool.values() {
        if let ClipContent::Midi { notes } = &entry.content
            && let Some(note) = notes.values().next()
        {
            return (note.id, note.duration_ticks);
        }
    }
    panic!("filled_project 必须至少有一颗 MIDI 音符");
}

fn note_duration(project: &YebanProjectV1, note_id: EntityId) -> Option<u64> {
    for entry in project.clip_pool.values() {
        if let ClipContent::Midi { notes } = &entry.content
            && let Some(note) = notes.get(&note_id)
        {
            return Some(note.duration_ticks);
        }
    }
    None
}

fn note_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .map(|entry| match &entry.content {
            ClipContent::Midi { notes } => notes.len(),
            _ => 0,
        })
        .sum()
}

fn select_notes(window: &MainWindow, ids: &[EntityId]) {
    window.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
        ids.iter()
            .map(|id| slint::SharedString::from(id.to_canonical_string()))
            .collect::<Vec<_>>(),
    )));
    window.set_selected_note_count(i32::try_from(ids.len()).unwrap_or(i32::MAX));
}

// ===========================================================================
// ① 轨道重命名
// ===========================================================================

#[test]
fn a_refused_track_rename_is_visibly_reported() {
    let (window, port, _ui) = assemble();
    let track_id = window
        .get_track_ids()
        .row_data(0)
        .expect("界面至少有一条轨");
    let before_name = port.project().tracks[&EntityId::from_str(track_id.as_str()).unwrap()]
        .name
        .clone();
    let before_commits = commit_count(&port);
    assert!(
        !window.get_show_floating_toast(),
        "夹具前提：提示条初始不显示"
    );

    port.refuse_next_commit_for_test();
    window.invoke_track_rename(0, "Criteria Rename".into());
    assert_rejected(&window, "轨道重命名");
    assert_eq!(
        port.project().tracks[&EntityId::from_str(track_id.as_str()).unwrap()].name,
        before_name,
        "被拒绝 ⇒ 工程一位不改"
    );
    assert_eq!(commit_count(&port), before_commits, "被拒绝 ⇒ 提交数不动");

    // 成功路径回归：开关已用掉，同一条手势必须真的改名。
    window.invoke_track_rename(0, "Criteria Rename".into());
    assert_eq!(
        port.project().tracks[&EntityId::from_str(track_id.as_str()).unwrap()].name,
        "Criteria Rename"
    );
    assert_eq!(commit_count(&port), before_commits + 1);
}

// ===========================================================================
// ② Delete 删选区
// ===========================================================================

#[test]
fn a_refused_delete_selection_is_visibly_reported() {
    let (window, port, _ui) = assemble();
    let (note_id, _) = first_note(&port.project());
    select_notes(&window, &[note_id]);
    let before_commits = commit_count(&port);

    port.refuse_next_commit_for_test();
    let consumed = window.invoke_key_action(DELETE_TEXT.into(), false, false, false, false);
    assert!(!consumed, "被拒绝的键盘动作保持不消费（既有设计）");
    assert_rejected(&window, "删除选区");
    assert!(
        note_duration(&port.project(), note_id).is_some(),
        "被拒绝 ⇒ 音符仍在"
    );
    assert_eq!(commit_count(&port), before_commits);

    // 成功路径回归。
    assert!(window.invoke_key_action(DELETE_TEXT.into(), false, false, false, false));
    assert!(
        note_duration(&port.project(), note_id).is_none(),
        "成功路径必须真的删掉"
    );
    assert_eq!(commit_count(&port), before_commits + 1);
}

// ===========================================================================
// ③ Cmd/Ctrl+D 复制选区
// ===========================================================================

#[test]
fn a_refused_duplicate_selection_is_visibly_reported() {
    let (window, port, _ui) = assemble();
    let (note_id, _) = first_note(&port.project());
    select_notes(&window, &[note_id]);
    let before_notes = note_count(&port.project());
    let before_commits = commit_count(&port);

    port.refuse_next_commit_for_test();
    let consumed = window.invoke_key_action("d".into(), false, true, false, false);
    assert!(!consumed);
    assert_rejected(&window, "复制选区");
    assert_eq!(
        note_count(&port.project()),
        before_notes,
        "被拒绝 ⇒ 没有副本"
    );
    assert_eq!(commit_count(&port), before_commits);

    assert!(window.invoke_key_action("d".into(), false, true, false, false));
    assert_eq!(
        note_count(&port.project()),
        before_notes + 1,
        "成功路径必须真的多一颗音符"
    );
    assert_eq!(commit_count(&port), before_commits + 1);
}

// ===========================================================================
// ④ Shift+→ 调整选区时长
// ===========================================================================

#[test]
fn a_refused_resize_selection_duration_is_visibly_reported() {
    let (window, port, _ui) = assemble();
    let (note_id, before_duration) = first_note(&port.project());
    select_notes(&window, &[note_id]);
    let before_commits = commit_count(&port);

    port.refuse_next_commit_for_test();
    let consumed = window.invoke_key_action(RIGHT_TEXT.into(), true, false, false, false);
    assert!(!consumed);
    assert_rejected(&window, "调整选区时长");
    assert_eq!(
        note_duration(&port.project(), note_id),
        Some(before_duration),
        "被拒绝 ⇒ 时长一位不改"
    );
    assert_eq!(commit_count(&port), before_commits);

    assert!(window.invoke_key_action(RIGHT_TEXT.into(), true, false, false, false));
    assert_eq!(
        note_duration(&port.project(), note_id),
        Some(before_duration + 240),
        "成功路径 = 一个吸附网格（960 PPQ / 4）"
    );
    assert_eq!(commit_count(&port), before_commits + 1);
}

// ===========================================================================
// ⑤ 裸 M 静音选中轨
// ===========================================================================

#[test]
fn a_refused_mixer_switch_is_visibly_reported() {
    let (window, port, _ui) = assemble();
    let track_id = window
        .get_track_ids()
        .row_data(0)
        .expect("界面至少有一条轨");
    let id = EntityId::from_str(track_id.as_str()).expect("fixture ULID");
    window.set_selected_track_id(track_id.clone());
    let before_mute = port.project().tracks[&id].mute;
    let before_commits = commit_count(&port);

    port.refuse_next_commit_for_test();
    let consumed = window.invoke_key_action("m".into(), false, false, false, false);
    assert!(!consumed);
    assert_rejected(&window, "静音/独奏切换");
    assert_eq!(
        port.project().tracks[&id].mute,
        before_mute,
        "被拒绝 ⇒ 静音位不变"
    );
    assert_eq!(commit_count(&port), before_commits);

    assert!(window.invoke_key_action("m".into(), false, false, false, false));
    assert_eq!(port.project().tracks[&id].mute, !before_mute);
    assert_eq!(commit_count(&port), before_commits + 1);
}

// ===========================================================================
// ⑥ 设备旁通预检（身份查不到 ⇒ bridge 预检拒绝）
// ===========================================================================

#[test]
fn a_refused_device_bypass_precheck_is_visibly_reported() {
    let (window, port, _ui) = assemble();
    let track_id = window
        .get_track_ids()
        .row_data(0)
        .expect("界面至少有一条轨");
    window.set_selected_track_id(track_id);
    let before_commits = commit_count(&port);
    assert!(
        !window.get_show_floating_toast(),
        "夹具前提：提示条初始不显示"
    );

    // 预检拒绝：身份合法但不在链里 ⇒ bridge 返回 Err（此前只有 stderr）。
    window.invoke_device_bypass_toggle(stale_ulid().to_canonical_string().into());
    assert_rejected(&window, "旁通切换");
    assert_eq!(commit_count(&port), before_commits, "预检拒绝 ⇒ Δ0");

    // 成功路径回归：真实设备身份点一次 ⇒ 真的旁通、提交 +1。
    let device_id = window
        .get_device_ids()
        .row_data(0)
        .expect("展示轨至少一台设备");
    let device = EntityId::from_str(device_id.as_str()).expect("device ULID");
    window.invoke_device_bypass_toggle(device_id);
    assert_eq!(commit_count(&port), before_commits + 1);
    let project = port.project();
    let bypassed = project
        .tracks
        .values()
        .flat_map(|track| track.devices.iter())
        .find(|definition| definition.id == device)
        .map(|definition| definition.bypassed)
        .expect("设备仍在工程里");
    assert!(bypassed, "成功路径必须真的旁通");
}
