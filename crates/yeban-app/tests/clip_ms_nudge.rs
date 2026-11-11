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
    let (window, port, _plane) = assemble();
    let project = port.try_project().expect("工程");
    let audio_ulid = first_placement_of(&project, true);
    let midi_ulid = first_placement_of(&project, false);
    assert_ne!(audio_ulid, midi_ulid, "两条摆放身份必须不同");
    assert!(
        window.get_arrangement_view(),
        "夹具起点必须是编曲视图（否则下面的键判据空转）"
    );
    // 纯函数在同一夹具上给出的数，就是宿主臂将要提交的数（⛔ 不读第二份换算）。
    let ops = clip_ms_nudge_ops_for(&project, &audio_ulid, 1.0, 1);
    assert_eq!(ops.len(), 1, "纯函数必须给音频摆放一条 op");
    assert!(
        clip_ms_nudge_ops_for(&project, &midi_ulid, 1.0, 1).is_empty(),
        "纯函数必须拒绝 MIDI 摆放"
    );
}

/// ② `Cmd/Ctrl+Shift+→` ⇒ 音频摆放 +48 采样（48 kHz 下 1 ms），提交 +1，选中态留下，可连按。
#[test]
fn cmd_ctrl_shift_right_moves_the_selected_audio_clip_by_one_millisecond() {
    let (window, port, _plane) = assemble();
    let project = port.try_project().expect("工程");
    let audio_ulid = first_placement_of(&project, true);
    let old = placement_by_ulid(&project, &audio_ulid)
        .expect("音频摆放")
        .offset_samples
        .unwrap_or(0);
    let commits_at_start = port.display().commit_count;
    assert_eq!(port.display().undoable, 0);

    select_clip(&window, &audio_ulid);
    // `Ctrl+Shift+→`（Linux 的 `Ctrl`；macOS 的 `Cmd` 由 `input.rs` 的判据覆盖）。
    assert!(
        window.invoke_key_action(RIGHT_TEXT.into(), true, true, false, false),
        "`Cmd/Ctrl+Shift+→` 必须被消费"
    );
    let after = port.try_project().expect("工程");
    assert_eq!(
        placement_by_ulid(&after, &audio_ulid)
            .expect("音频摆放")
            .offset_samples,
        Some(old + 48),
        "一格 = +48 采样（48 kHz 下 1 ms）"
    );
    assert_eq!(port.display().undoable, 1, "一次按键 = 一步撤销");
    assert_eq!(
        port.display().commit_count,
        commits_at_start + 1,
        "一次微移 = 一条提交（相对起点 Δ1）"
    );
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        audio_ulid,
        "剪辑不销毁 ⇒ 选中态留下，可以连按第二下"
    );
    // 连按第二下：同一格再 +48（累加，不是重写回同一值）。
    assert!(window.invoke_key_action(RIGHT_TEXT.into(), true, true, false, false));
    let after2 = port.try_project().expect("工程");
    assert_eq!(
        placement_by_ulid(&after2, &audio_ulid)
            .expect("音频摆放")
            .offset_samples,
        Some(old + 96),
        "第二下再 +48 采样"
    );
    assert_eq!(port.display().undoable, 2, "两下 = 两步撤销");
}

/// ③ 0.1 ms 档：`Cmd/Ctrl+Shift+Alt+→` ⇒ +5 采样（4.8 四舍五入）；`←` ⇒ −48 / −5。
#[test]
fn cmd_ctrl_shift_alt_right_moves_the_selected_audio_clip_by_a_tenth_of_a_millisecond() {
    let (window, port, _plane) = assemble();
    let project = port.try_project().expect("工程");
    let audio_ulid = first_placement_of(&project, true);
    let original_offset = placement_by_ulid(&project, &audio_ulid)
        .expect("音频摆放")
        .offset_samples;
    let old = original_offset.unwrap_or(0);

    select_clip(&window, &audio_ulid);
    assert!(
        window.invoke_key_action(RIGHT_TEXT.into(), true, true, true, false),
        "`Cmd/Ctrl+Shift+Alt+→` 必须被消费"
    );
    let after = port.try_project().expect("工程");
    assert_eq!(
        placement_by_ulid(&after, &audio_ulid)
            .expect("音频摆放")
            .offset_samples,
        Some(old + 5),
        "0.1 ms @48k = 4.8 → +5 采样"
    );
    // `←` 0.1 ms 档：−5 回去（与上面同一档、反方向）。0 归一化为 `None`，所以回到的
    // 是**原 `Option` 值**，不是 `Some(0)`（`Op::SetClipPlacementOffset` 的 apply 语义）。
    assert!(window.invoke_key_action(LEFT_TEXT.into(), true, true, true, false));
    let back = port.try_project().expect("工程");
    assert_eq!(
        placement_by_ulid(&back, &audio_ulid)
            .expect("音频摆放")
            .offset_samples,
        original_offset,
        "−0.1 ms 必须回到原值"
    );
    // `←` 1 ms 档：−48。
    assert!(window.invoke_key_action(LEFT_TEXT.into(), true, true, false, false));
    let left = port.try_project().expect("工程");
    assert_eq!(
        placement_by_ulid(&left, &audio_ulid)
            .expect("音频摆放")
            .offset_samples,
        Some(old - 48),
        "−1 ms @48k = −48 采样"
    );
}

/// ④ 作用域：选中 MIDI 片段、或没有选中时，同一把键不消费、0 提交。
#[test]
fn the_ms_nudge_keys_are_not_consumed_for_a_midi_clip_or_with_no_selection() {
    let (window, port, _plane) = assemble();
    let project = port.try_project().expect("工程");
    let audio_ulid = first_placement_of(&project, true);
    let midi_ulid = first_placement_of(&project, false);
    let commits_at_start = port.display().commit_count;

    // MIDI 片段：同一把键不消费（音符已有 `Alt+←/→` 1 tick 微移）。
    select_clip(&window, &midi_ulid);
    assert!(
        !window.invoke_key_action(RIGHT_TEXT.into(), true, true, false, false),
        "MIDI 片段上 `Cmd/Ctrl+Shift+→` 必须不消费"
    );
    assert!(
        !window.invoke_key_action(RIGHT_TEXT.into(), true, true, true, false),
        "MIDI 片段上 0.1 ms 档也必须不消费"
    );
    assert_eq!(
        port.display().commit_count,
        commits_at_start,
        "MIDI 片段上 0 提交"
    );
    assert_eq!(
        placement_by_ulid(&port.try_project().expect("工程"), &audio_ulid)
            .expect("音频摆放")
            .offset_samples
            .unwrap_or(0),
        placement_by_ulid(&project, &audio_ulid)
            .expect("音频摆放")
            .offset_samples
            .unwrap_or(0),
        "音频摆放一位不动"
    );

    // 没有选中：同样不消费（⛔ 不顺手清掉那个陈旧读数）。
    window.set_selected_clip_ulid("".into());
    assert!(
        !window.invoke_key_action(RIGHT_TEXT.into(), true, true, false, false),
        "没有选中时 `Cmd/Ctrl+Shift+→` 必须不消费"
    );
    assert_eq!(
        port.display().commit_count,
        commits_at_start,
        "没有选中时 0 提交"
    );
}

/// ⑤ 视图门控：Session 视图里同一份选中态不消费、0 提交；回编曲视图立刻生效。
#[test]
fn a_selection_left_over_from_the_arrangement_view_is_not_nudged_in_session_view() {
    let (window, port, _plane) = assemble();
    let project = port.try_project().expect("工程");
    let audio_ulid = first_placement_of(&project, true);
    let old = placement_by_ulid(&project, &audio_ulid)
        .expect("音频摆放")
        .offset_samples
        .unwrap_or(0);
    let commits_at_start = port.display().commit_count;

    select_clip(&window, &audio_ulid);
    window.set_arrangement_view(false);
    assert!(
        !window.invoke_key_action(RIGHT_TEXT.into(), true, true, false, false),
        "Session 视图里 `Cmd/Ctrl+Shift+→` 必须不消费"
    );
    assert_eq!(
        port.display().commit_count,
        commits_at_start,
        "Session 视图里 0 提交"
    );
    assert_eq!(
        placement_by_ulid(&port.try_project().expect("工程"), &audio_ulid)
            .expect("音频摆放")
            .offset_samples
            .unwrap_or(0),
        old,
        "Session 视图里工程不变"
    );
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        audio_ulid,
        "选中态留下（⛔ 不顺手清掉）"
    );

    // 回编曲视图：同一把键立刻生效。
    window.set_arrangement_view(true);
    assert!(window.invoke_key_action(RIGHT_TEXT.into(), true, true, false, false));
    assert_eq!(
        placement_by_ulid(&port.try_project().expect("工程"), &audio_ulid)
            .expect("音频摆放")
            .offset_samples,
        Some(old + 48),
        "回编曲视图后一格 = +48 采样"
    );
}

/// ⑥ 一步撤销 ＋ redo：`Undo` 将工程逐字节放回，`Redo` 再移过去。
#[test]
fn undo_puts_the_clip_offset_back_and_redo_moves_it_again() {
    let (window, port, _plane) = assemble();
    let project = port.try_project().expect("工程");
    let audio_ulid = first_placement_of(&project, true);
    let original_offset = placement_by_ulid(&project, &audio_ulid)
        .expect("音频摆放")
        .offset_samples;
    let old = original_offset.unwrap_or(0);

    select_clip(&window, &audio_ulid);
    assert!(window.invoke_key_action(RIGHT_TEXT.into(), true, true, false, false));
    assert_eq!(
        placement_by_ulid(&port.try_project().expect("工程"), &audio_ulid)
            .expect("音频摆放")
            .offset_samples,
        Some(old + 48),
        "先移 +48 采样"
    );
    // `Cmd/Ctrl+Z` = 撤销（`invoke_key_action("z", shift=false, control=false, alt=false, meta=true)`）。
    assert!(window.invoke_key_action("z".into(), false, false, false, true));
    assert_eq!(
        placement_by_ulid(&port.try_project().expect("工程"), &audio_ulid)
            .expect("音频摆放")
            .offset_samples,
        original_offset,
        "撤销后偏移回到原值（0 仍是 None）"
    );
    // `Cmd/Ctrl+Shift+Z` = 重做。
    assert!(window.invoke_key_action("z".into(), true, false, false, true));
    assert_eq!(
        placement_by_ulid(&port.try_project().expect("工程"), &audio_ulid)
            .expect("音频摆放")
            .offset_samples,
        Some(old + 48),
        "重做后又回到 +48 采样"
    );
}
