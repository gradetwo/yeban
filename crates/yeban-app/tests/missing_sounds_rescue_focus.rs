//! 判据：**包安装后自动救援删行时，焦点不能留在被删的行上**（T-FIX-344）。
//!
//! `host::rescue_after_pack_installed` 是后台完成的：用户在救援面板点「下载」
//! ⇒ 面板收起、音源管理在前台；关掉音源管理 ⇒ 面板还回来、焦点回 ✕；用户
//! 再 `Tab` 回某一行 —— 这时下载完成、救援把那一行换上并删掉，焦点若还在
//! 被删的行上，`focused-row-slot` 就指着不存在的行（死焦点），之后 `Tab`
//! 的读数与轮转全错。对话框内点的「换成 X」有 T-FIX-333 的按行守卫（换上
//! 前先挪到 ✕）；宿主这条后台删行路没有 —— 本文件钉住它补上同一守卫。
//!
//! 走**生产接线**（`host::wire_missing_sounds_dialog` + `wire_sound_library_dialog_with`，
//! 与 `missing_sounds_rescue_flow.rs` 同款），不走回调置换：
//!
//! ① 两行缺失，行 1 是已装包的轨：点行 1「下载」⇒ 待办记下、面板收起、音源
//!    管理在前；关音源管理 ⇒ 面板还回来、焦点回 ✕；`Tab` 到行 1 的格子；
//! ② 装好那一跳（`rescue_after_pack_installed`，与下载排空同一个函数）⇒
//!    行 1 换上并消失，**焦点必须先落到 ✕（读数 0）**，行数 1，面板还开着；
//! ③ 再 `Tab` 一圈：只剩行 0 的格子 + 「先不管」+ ✕，不越界、不卡死。
//! ④ 单行：焦点在唯一一行的格子上，救援删掉它 ⇒ 面板关闭、行数 0、
//!    `focused-control == -1`（没有死焦点）。
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证真从网络下载成功（"装好"这一跳按唯一入口直接调，与下载排空同一
//! 函数，同 `missing_sounds_rescue_flow.rs` 的做法）；⛔ 不证焦点在**幸存**
//! ⑤ 幸存行重基（T-FIX-427）：焦点在行 1 的格子上，救援删掉行 0 ⇒ 焦点仍
//! 指向同一行（槽位按新行号重基，旧槽位 −10）。
//! ⛔ 不证 `Shift+Tab` 反向（平台通道注不进来，同款边界）。

#![allow(unused)]
use slint::ComponentHandle;
use std::collections::BTreeMap;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::EntityId;
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig, RoutingEdge, RoutingKind, TrackKind,
    TrackV3, YebanProjectV1,
};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::{KeyCode, Permission, UiTestPort};
use yeban_ui_test_port::render::{LivePort, report_line};

const WINDOW: Size = Size::new(1440, 1000);
const NOW_MS: u64 = 1_760_000_000_000;
const NOTE_TICKS: u64 = 480;

/// 行 0 的音源路径：清单里有、但本机没装（`missing_sounds_rescue_flow.rs` 同款）。
const MISSING_PATH: &str = "assets/samples/emily_basic/Programs/emily_basic.sfz";

/// N 条缺音源的 MIDI 轨 + 主总线；`track_order` 定死行序。调用方按顺序给
/// `(轨名, 音源路径)`；返回工程与各轨 id（行号 = 顺序号）。
fn project_with_missing_tracks(
    tag: &str,
    tracks: &[(&str, String)],
) -> (YebanProjectV1, Vec<EntityId>) {
    let master = EntityId::new();
    let ids: Vec<EntityId> = tracks.iter().map(|_| EntityId::new()).collect();
    let mut project = YebanProjectV1 {
        master_bus_track_id: master,
        track_order: ids.clone(),
        ..YebanProjectV1::default()
    };
    for (id, (name, path)) in ids.iter().zip(tracks.iter()) {
        let clip_id = EntityId::new();
        let note = yeban_model::MidiNote::new(EntityId::new(), 0, 60, NOTE_TICKS);
        let mut notes = BTreeMap::new();
        notes.insert(note.id, note);
        project.clip_pool.insert(
            clip_id,
            ClipPoolEntry {
                id: clip_id,
                name: "clip".to_owned(),
                content: ClipContent::Midi { notes },
            },
        );
        let placement = ClipPlacement {
            id: EntityId::new(),
            clip_id,
            start_tick: 0,
            duration_ticks: NOTE_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
        };
        let mut track = TrackV3 {
            id: *id,
            name: format!("{name} {tag}"),
            kind: TrackKind::Midi,
            sampler: Some(yeban_model::SamplerBinding {
                note_octave_override: None,
                path: path.clone(),
            }),
            ..TrackV3::default()
        };
        track.clips.insert(placement.id, placement);
        project.tracks.insert(*id, track);
    }
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let edge = EntityId::new();
    let mut nodes = ids.clone();
    nodes.push(master);
    project.routing_graph.nodes = nodes;
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: ids[0],
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
    project.validate().expect("夹具工程必须合法");
    (project, ids)
}

/// 生产形态的活窗口 ＋ 音源管理/救援面板的**生产接线**（同
/// `missing_sounds_rescue_flow.rs`）。
fn live_window(project: &YebanProjectV1, tag: &str) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            format!("<判据 {tag}>"),
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let mut live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    let ui = live.ui();
    host::wire_sound_library_dialog_with(ui, Some(Rc::clone(&port)));
    let rows = yeban_app::missing_sounds::collect(&port.project(), None);
    assert!(
        !rows.is_empty(),
        "夹具必须真的被判成缺音源（否则本判据是空绿）"
    );
    host::wire_missing_sounds_dialog(ui, Rc::clone(&port), rows);
    (live, port)
}

/// 清单一遍扫：第一件**本机解析得到 .sfz** 的内置音源（同
/// `missing_sounds_rescue_flow.rs`）。
fn first_installed_pack() -> Option<(String, String, String)> {
    let sounds = yeban_app::builtin_sounds::load_builtin_sounds().ok()?;
    for sound in sounds {
        if let Some(path) = yeban_app::missing_sounds::installed_path(
            &sound.id,
            yeban_app::sfz_load::SfzRoots::none(),
        ) {
            return Some((sound.id, sound.name, path));
        }
    }
    None
}

/// `Tab` 一下，读当前焦点槽位。
fn tab_once(live: &mut LivePort<MainWindow>) -> i32 {
    live.dispatch_key_press(KeyCode::Tab)
        .expect("救援焦点：注入 Tab");
    live.ui().get_missing_sounds_focused_control()
}

/// 一直 `Tab` 直到焦点落在行 `row` 里（槽位 `100+row*10+{1..4}`），最多 10 下。
/// 返回落到的槽位；到不了就失败（不是静默跳过 —— 行里没格子就是判据前提坏了）。
fn tab_into_row(live: &mut LivePort<MainWindow>, row: i32) -> i32 {
    for _ in 0..10 {
        let slot = tab_once(live);
        if slot >= 100 + row * 10 && slot < 100 + (row + 1) * 10 {
            return slot;
        }
    }
    panic!(
        "救援焦点：Tab 10 下都没走到行 {row}（当前槽位 {}）",
        live.ui().get_missing_sounds_focused_control()
    );
}

/// 点行 `row` 的「下载」⇒ 待办记下、面板收起、音源管理在前；再关音源管理 ⇒
/// 面板还回来、焦点回 ✕。这是用户真实走的那条路（不是测试专用的捷径）。
fn download_then_return(live: &mut LivePort<MainWindow>, row: i32, pack_id: &str, pack_name: &str) {
    let ui = live.ui();
    ui.invoke_missing_open_pack(row, pack_id.into(), pack_name.into());
    assert!(
        !ui.get_missing_sounds_open(),
        "救援焦点：点「下载」后救援面板必须先收起来"
    );
    assert!(ui.get_sound_library_open(), "救援焦点：音源管理必须在前面");
    ui.invoke_close_sound_library();
    assert!(!ui.get_sound_library_open(), "救援焦点：音源管理必须关掉");
    assert!(
        ui.get_missing_sounds_open(),
        "救援焦点：关掉音源管理后，有未解决的轨 ⇒ 面板必须还回来"
    );
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        0,
        "救援焦点：还回来的面板焦点必须落在 ✕（0 格）"
    );
}

/// ①②③：焦点在行 1 的格子上，救援删掉行 1 ⇒ 焦点先落到 ✕，再 `Tab` 一圈正常。
#[test]
fn rescue_deleting_the_focused_row_moves_focus_to_close_first() {
    let Some((pack_id, pack_name, pack_path)) = first_installed_pack() else {
        eprintln!("[rescue/focus] 本机没有已装的内置音源，跳过");
        return;
    };
    let bass_path = format!("assets/samples/{pack_id}/Programs/missing-{pack_id}.sfz");
    let (project, ids) = project_with_missing_tracks(
        "focus",
        &[("Lead", MISSING_PATH.to_owned()), ("Bass", bass_path)],
    );
    let bass_id = ids[1];
    // 行表前提：行 0 是没装的包，行 1 正是刚装好的这件 —— 否则"删行 1"测的不是这件。
    let rows = yeban_app::missing_sounds::collect(&project, None);
    assert_eq!(rows.len(), 2, "救援焦点：夹具必须正好两行缺失");
    assert_eq!(
        rows[1].pack_id.as_deref(),
        Some(pack_id.as_str()),
        "救援焦点：行 1 必须对应已装包 {pack_id}（否则救援删不掉它）"
    );

    let (mut live, port) = live_window(&project, "focus");
    let ui = live.ui().clone_strong();
    assert!(ui.get_missing_sounds_open(), "打开工程就该弹出救援面板");
    assert_eq!(ui.get_missing_row_count(), 2, "救援焦点：行数必须是 2");
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        0,
        "救援焦点：入口焦点必须落在 ✕（0 格）"
    );

    download_then_return(&mut live, 1, &pack_id, &pack_name);
    let focused = tab_into_row(&mut live, 1);
    assert!(
        (111..=114).contains(&focused),
        "救援焦点：必须先落在行 1 的格子里（当前 {focused}）"
    );

    // 装好那一跳（与下载排空同一个函数）⇒ 行 1 换上并消失。
    host::rescue_after_pack_installed(&ui, pack_id.as_str());

    // ② 焦点必须先落到 ✕（行消失之前就挪走，不留死焦点），不是事后读 -1。
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        0,
        "救援焦点：救援删掉焦点所在行后，焦点必须落在 ✕（0 格）"
    );
    assert_eq!(ui.get_missing_row_count(), 1, "救援焦点：行数必须剩 1");
    assert!(
        ui.get_missing_sounds_open(),
        "救援焦点：还有行没解决 ⇒ 面板还开着"
    );
    let sampler = port
        .project()
        .tracks
        .get(&bass_id)
        .and_then(|track| track.sampler.clone())
        .expect("救援焦点：行 1 的轨必须已经绑上");
    assert_eq!(
        sampler.path, pack_path,
        "救援焦点：绑的必须正是刚装好的那一件"
    );

    // ③ 再 Tab 一圈：只剩行 0 的格子 + 「先不管」+ ✕，不越界、不卡死。
    let mut visited = vec![0];
    for _ in 0..8 {
        let slot = tab_once(&mut live);
        visited.push(slot);
        assert!(
            slot == 0 || slot == 1 || (101..=104).contains(&slot),
            "救援焦点：删行后 Tab 只能走到行 0 的格子 / 1 / 0（走到 {slot}）"
        );
        if slot == 0 {
            break;
        }
    }
    assert_eq!(
        *visited.last().expect("至少走一步"),
        0,
        "救援焦点：Tab 一圈必须绕回 ✕（走了 {visited:?}）"
    );

    live.dispatch_key_press(KeyCode::Escape)
        .expect("救援焦点：注入 Esc");
    assert!(!ui.get_missing_sounds_open(), "救援焦点：Esc 必须关掉弹窗");
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        -1,
        "救援焦点：关闭后焦点必须不在模态内"
    );
    report_line("missing-sounds-rescue-focus two-row: focus-111 rescue-to-0 tab-loop esc-closed");
}

/// ⑤ 幸存行重基（T-FIX-427）：焦点在行 1 的格子上，救援删掉行 0 ⇒ 焦点
/// 必须仍指向**同一行**（同一条轨），即槽位按新行号重基（旧槽位 −10），
/// 不能留在旧值上（旧值在新表里指行 1——原来行 2 的那条轨）。
///
/// 红→绿：未修时 `focused-control` 仍是删行前的旧槽位（或被清掉），
/// 与「同一行」的期望不符；修后读数 = 旧槽位 −10，`Tab` 一圈只走到
/// 合法格子。
#[test]
fn rescue_deleting_a_row_above_the_focused_row_rebases_the_focus_slot() {
    let Some((pack_id, pack_name, pack_path)) = first_installed_pack() else {
        eprintln!("[rescue/focus] 本机没有已装的内置音源，跳过");
        return;
    };
    let bass_path = format!("assets/samples/{pack_id}/Programs/missing-{pack_id}.sfz");
    let (project, ids) = project_with_missing_tracks(
        "rebase",
        &[
            ("Bass", bass_path),
            ("Lead", MISSING_PATH.to_owned()),
            ("Keys", MISSING_PATH.to_owned()),
        ],
    );
    let bass_id = ids[0];
    let lead_id = ids[1];
    let rows = yeban_app::missing_sounds::collect(&project, None);
    assert_eq!(rows.len(), 3, "救援焦点：夹具必须正好三行缺失");
    assert_eq!(
        rows[0].pack_id.as_deref(),
        Some(pack_id.as_str()),
        "救援焦点：行 0 必须对应已装包 {pack_id}（否则救援删不掉它）"
    );

    let (mut live, port) = live_window(&project, "rebase");
    let ui = live.ui().clone_strong();
    assert!(ui.get_missing_sounds_open(), "打开工程就该弹出救援面板");
    assert_eq!(ui.get_missing_row_count(), 3, "救援焦点：行数必须是 3");

    download_then_return(&mut live, 0, &pack_id, &pack_name);
    let focused = tab_into_row(&mut live, 1);
    assert!(
        (111..=114).contains(&focused),
        "救援焦点：必须先落在行 1 的格子里（当前 {focused}）"
    );

    // 装好那一跳：行 0 换上并消失；行 1 变成行 0（同一条轨）。
    host::rescue_after_pack_installed(&ui, pack_id.as_str());

    assert_eq!(ui.get_missing_row_count(), 2, "救援焦点：行数必须剩 2");
    assert!(
        ui.get_missing_sounds_open(),
        "救援焦点：还有行没解决 ⇒ 面板还开着"
    );
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        focused - 10,
        "救援焦点：焦点必须仍指向同一行（旧槽位 {focused} 重基为 {}）",
        focused - 10
    );
    let sampler = port
        .project()
        .tracks
        .get(&bass_id)
        .and_then(|track| track.sampler.clone())
        .expect("救援焦点：行 0 的轨必须已经绑上");
    assert_eq!(
        sampler.path, pack_path,
        "救援焦点：绑的必须正是刚装好的那一件"
    );
    // 同一条轨还在表里（行 1 的轨现在是行 0）。
    assert!(
        port.project().tracks.contains_key(&lead_id),
        "救援焦点：幸存行（行 1 的轨）必须还在工程里"
    );

    // 再 Tab 一圈：只剩两行的格子 + 「先不管」+ ✕，不越界、不卡死。
    let mut visited = vec![focused - 10];
    for _ in 0..10 {
        let slot = tab_once(&mut live);
        visited.push(slot);
        assert!(
            slot == 0 || slot == 1 || (101..=104).contains(&slot) || (111..=114).contains(&slot),
            "救援焦点：删行后 Tab 只能走到幸存行的格子 / 1 / 0（走到 {slot}）"
        );
        if slot == 0 {
            break;
        }
    }
    assert_eq!(
        *visited.last().expect("至少走一步"),
        0,
        "救援焦点：Tab 一圈必须绕回 ✕（走了 {visited:?}）"
    );
    report_line("missing-sounds-rescue-focus rebase: focus-11x rescue-to-10x tab-loop");
}

/// ④ 单行：焦点在唯一一行的格子上，救援删掉它 ⇒ 面板关闭、读数 -1，没有死焦点。
#[test]
fn rescue_deleting_the_last_focused_row_closes_without_a_dead_focus() {
    let Some((pack_id, pack_name, pack_path)) = first_installed_pack() else {
        eprintln!("[rescue/focus] 本机没有已装的内置音源，跳过");
        return;
    };
    let bass_path = format!("assets/samples/{pack_id}/Programs/missing-{pack_id}.sfz");
    let (project, ids) = project_with_missing_tracks("last-row", &[("Bass", bass_path)]);
    let bass_id = ids[0];
    let rows = yeban_app::missing_sounds::collect(&project, None);
    assert_eq!(rows.len(), 1, "救援焦点：单行夹具必须正好一行");
    assert_eq!(
        rows[0].pack_id.as_deref(),
        Some(pack_id.as_str()),
        "救援焦点：唯一一行必须对应已装包 {pack_id}"
    );

    let (mut live, port) = live_window(&project, "last-row");
    let ui = live.ui().clone_strong();
    assert!(ui.get_missing_sounds_open(), "打开工程就该弹出救援面板");
    assert_eq!(ui.get_missing_row_count(), 1, "救援焦点：行数必须是 1");

    download_then_return(&mut live, 0, &pack_id, &pack_name);
    let focused = tab_into_row(&mut live, 0);
    assert!(
        (101..=104).contains(&focused),
        "救援焦点：必须先落在行 0 的格子里（当前 {focused}）"
    );

    host::rescue_after_pack_installed(&ui, pack_id.as_str());

    assert!(
        !ui.get_missing_sounds_open(),
        "救援焦点：行全清后面板必须自己走人"
    );
    assert_eq!(ui.get_missing_row_count(), 0, "救援焦点：行数必须归零");
    // 焦点落在 ✕（0 格）：守卫在行消失前就把它挪到了对话框的入口锚点 ——
    // ✕ 是真实存在的格子，不是已删的行，所以不是死焦点。注意这与 Esc / ✕
    // 手动关窗不同：手动关窗会把焦点复位回快捷键层（`on_missing_close`），
    // 救援关窗历来不复位（改前改后同一行为，本单不动它）；读数 0 只说明焦点
    // 停在已关对话框的入口锚点上，下次打开仍是同一入口。
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        0,
        "救援焦点：面板关闭后焦点必须落在 ✕（0 格），不能指着已删的行"
    );
    let sampler = port
        .project()
        .tracks
        .get(&bass_id)
        .and_then(|track| track.sampler.clone())
        .expect("救援焦点：这条轨必须已经绑上");
    assert_eq!(
        sampler.path, pack_path,
        "救援焦点：绑的必须正是刚装好的那一件"
    );
    report_line("missing-sounds-rescue-focus last-row: focus-10x rescue-closed focus-0");
}
