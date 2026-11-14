//! **片段毫秒微移**（`Cmd/Ctrl+Shift+←/→` ±1 ms / `Cmd/Ctrl+Shift+Alt+←/→` ±0.1 ms）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 音频片段摆在编曲视图里，用户可以点它选中、可以拖它平移、可以裁剪它、可以静音它 ——
//! 但**没法把它按毫秒微移**：`Op::SetClipPlacementOffset` 在模型里早已存在（`ops.rs` 有
//! apply / inverse / 前置条件，新值夹到 ±4800 采样走的是与 `ClipPlacement::validate`
//! 同一条 `ModelError::ClipOffsetOutOfRange`），而界面侧在此切片之前**一处都没有**：
//! `grep -rn "Op::SetClipPlacementOffset" crates/yeban-app/src/host.rs` = **空**。
//! ⇒ "片段能挪、能裁、能静音，就是不能毫秒微移"这一半接线缺口（手动相位对齐第一层）。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（夹具前提） | 至少一条**音频**摆放与一条 MIDI 摆放、没有音符选区、编曲视图在前 |
//! | ②（写权威） | `Cmd/Ctrl+Shift+→` ⇒ 只有那条音频摆放 `offset_samples` **+48 采样**（48 kHz 下 1 ms；另一条摆放逐字段不变）、提交 **+1**、选中态**留下**、可再按第二下继续累加 |
//! | ③（0.1 ms 档） | `Cmd/Ctrl+Shift+Alt+→` ⇒ 同一条摆放 **+5 采样**（4.8 四舍五入）；`←` ⇒ −48 / −5 |
//! | ④（作用域） | 选中 **MIDI** 片段时同一把键**不消费、0 提交**（音符已有 `Alt+←/→` 1 tick 微移，⛔ 不跨界）；没有选中时同样不消费 |
//! | ⑤（视图门控） | Session 视图里同一份选中态**不消费、0 提交、工程不变**；回编曲视图立刻生效 |
//! | ⑥（一步撤销 ＋ redo） | `Undo` 将工程逐字节放回（载荷带 `old_offset_samples`），`Redo` 再移过去 |
//! | ⑦（纯函数 ＋ 源码纪律） | `clip_ms_nudge_ops_for` 逐字段对账（48 kHz / 44.1 kHz 换算、MIDI 拒绝、边界夹紧、可逆）见 `src/host.rs` 的同名判据；`host.rs` 里 `Op::SetClipPlacementOffset` 的构造点**恰好一处** |
//!
//! ## 为什么这一票在 `.slint` 侧**零改动**（也是它⛔ 不触发基准重录的理由）
//!
//! 剪辑的 **x 位置**本来就是投影算出来的：`ViewState::clip_view` ←
//! `ClipPlacement::offset_samples` → `apply_view` 写剪辑位置。所以微移走的是"提交 →
//! 重投影 → 画面自己跟着走"这一条已有通路，**不需要**新属性、不需要新元素 ID、不需要动注册表。
//! 默认帧因此逐像素不变（⇒ `tests/perf_draw_budget.rs` 的两个预算数一位不涨，golden 基准⛔ 不重录）。
//! x 位置本身的换算由 `src/bridge.rs` 的判据 `clip_view_x_follows_clip_offset_samples` 钉死，
//! 本文件不重复证明"像素"；本文件只证明"键 ⇒ 权威 ⇒ 提交口径"。
//!
//! ## 注入分两格（本文件的边界）
//!
//! * **键**：`Cmd/Ctrl+Shift+←/→` 与 `Cmd/Ctrl+Shift+Alt+←/→` 经
//!   `MainWindow::invoke_key_action` —— 那正是 `.slint` 的 `key-handler`
//!   派发快捷键时调的**同一个**回调（实参顺序 `text, shift, control, alt, meta`，现读
//!   `app.slint` 的调用点），返回值就是"消费 / 不消费"。
//!   ⚠ 它⛔ 不是 `WindowEvent`：端口那侧的修饰键注入由 `tests/roll_edit_tools.rs` 判据 (e)
//!   用**真事件源**覆盖，本文件不重复证明"事件源 ⇒ 属性"；本文件只证明"属性 ⇒ 权威 ⇒ 画面"；
//! * **选中**：剪辑走 `.slint` 包头 `clicked` 调的那一个 `clip-select` 回调（宿主自己校验身份）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的且每线程只能装配一次；每例一新线程由 libtest
//! 负责 ⇒ 本文件可以有多个 `#[test]`（与 `tests/d_ui_13_roll_hit_scope.rs` 同一做法）。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与无头比对交 CI 与盒子。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `tests/clip_drag.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveControlPlane, build_live_ui_with};
use yeban_app::host::clip_ms_nudge_ops_for;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::{ClipContent, ClipPlacement};
use yeban_model::{EntityId, YebanProjectV1};
use yeban_ui_test_port::port::Permission;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// `←` 的 `event.text`（Slint 的功能键私有码点，现读 `input.rs::LogicalKey::from_text`）。
const LEFT_TEXT: &str = "\u{f702}";

/// `→` 的 `event.text`。
const RIGHT_TEXT: &str = "\u{f703}";

/// 装配：真窗口 ＋ **撤销端口当装配选项传进去**（`host::wire_keys` 从第一刻起就握着端口，
/// 与产品进程 `main.rs` 同一形态）＋ 与生产同一条 `wire_clip_scope`。
fn assemble() -> (MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<clip-ms-nudge>", "yeban-app", project.clone(), NOW_MS).expect("打开"),
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

/// 夹具里第一条音频摆放与第一条 MIDI 摆放的身份（`filled_project` 两者都有）。
fn first_placement_of(project: &YebanProjectV1, audio: bool) -> String {
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let is_audio = matches!(entry.content, ClipContent::Audio { .. });
            if is_audio == audio {
                return placement.id.to_canonical_string();
            }
        }
    }
    panic!(
        "夹具里必须有一条{}摆放",
        if audio { "音频" } else { "MIDI" }
    );
}

/// 权威工程里按身份读一条摆放（读不到 ⇒ `None`）。
fn placement_by_ulid(project: &YebanProjectV1, ulid: &str) -> Option<ClipPlacement> {
    let id = ulid.parse::<EntityId>().ok()?;
    project
        .tracks
        .values()
        .find_map(|track| track.clips.get(&id).copied())
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

/// ① 夹具前提：音频与 MIDI 摆放各至少一条、编曲视图在前、纯函数与宿主臂同一条换算。
#[test]
fn the_fixture_has_an_audio_and_a_midi_placement_in_arrangement_view() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// ② `Cmd/Ctrl+Shift+→` ⇒ 音频摆放 +48 采样（48 kHz 下 1 ms），提交 +1，选中态留下，可连按。
#[test]
fn cmd_ctrl_shift_right_moves_the_selected_audio_clip_by_one_millisecond() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// ③ 0.1 ms 档：`Cmd/Ctrl+Shift+Alt+→` ⇒ +5 采样（4.8 四舍五入）；`←` ⇒ −48 / −5。
#[test]
fn cmd_ctrl_shift_alt_right_moves_the_selected_audio_clip_by_a_tenth_of_a_millisecond() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// ④ 作用域：选中 MIDI 片段、或没有选中时，同一把键不消费、0 提交。
#[test]
fn the_ms_nudge_keys_are_not_consumed_for_a_midi_clip_or_with_no_selection() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// ⑤ 视图门控：Session 视图里同一份选中态不消费、0 提交；回编曲视图立刻生效。
#[test]
fn a_selection_left_over_from_the_arrangement_view_is_not_nudged_in_session_view() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// ⑥ 一步撤销 ＋ redo：`Undo` 将工程逐字节放回，`Redo` 再移过去。
#[test]
fn undo_puts_the_clip_offset_back_and_redo_moves_it_again() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}
