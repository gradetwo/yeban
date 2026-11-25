//! 判据：**对话框内点「换成 X」删行时，焦点在幸存行上不能错行**
//! （T-ISS-MWVUOZCU，T-FIX-427 回执 §8 的 follow-up）。
//!
//! `on_missing_swap_suggested` 是对话框内的删行路：用户在某一行点已装候选
//! 「换成 X」⇒ 宿主把那一行换上并从列表删掉。T-FIX-333 的对话框内 swap 守卫
//! 只处理焦点在**被删行**上的情况（先挪到 ✕）；焦点在**幸存行**上时，
//! `publish_missing_rows` 重发表后行号整体前移，`focused-row-slot` 的旧值
//! 就落在下一行上 —— 与 T-FIX-427 的救援路径同一类缺陷。本文件钉住 swap 路
//! 的重基：焦点在行 1 的格子上，swap 删掉行 0 ⇒ 焦点必须仍指向**同一行**
//! （同一条轨），即槽位按新行号重基（旧槽位 −10）。
//!
//! 走**生产接线**（`host::wire_missing_sounds_dialog` +
//! `wire_sound_library_dialog_with`，与 `missing_sounds_rescue_focus.rs`
//! 同款），直接调生产回调 `invoke_missing_swap_suggested`：
//!
//! ① 三行缺失，行 0 是已装包的轨；`Tab` 到行 1 的格子；
//! ② 行 0「换成 X」（生产回调）⇒ 行 0 换上并消失，**焦点必须仍指向同一行**
//!    （读数 111 → 101），行数 2，面板还开着；
//! ③ 再 `Tab` 一圈：只剩两行的格子 + 「先不管」+ ✕，不越界、不卡死。
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证焦点在**被删行**上的 swap（对话框内守卫先挪到 ✕，
//! `missing_sounds_candidate_focus.rs` 已钉住；本文件只证幸存行重基）；
//! ⛔ 不证救援路径（`missing_sounds_rescue_focus.rs`，T-FIX-427 的单另有判据）；
//! ⛔ 不证 `Shift+Tab` 反向（平台通道注不进来，同款边界）；
//! ⛔ 不证物理键盘焦点在模型重发表后是否留在滑上来的行上（取决于 Slint
//! repeater 的元素复用，headless 环境无法观测 —— 与 T-FIX-427 同款边界，
//! 本单只证源码级槽位读数）。

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
        .expect("swap 焦点：注入 Tab");
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
        "swap 焦点：Tab 10 下都没走到行 {row}（当前槽位 {}）",
        live.ui().get_missing_sounds_focused_control()
    );
}

/// ①②③：焦点在行 1 的格子上，swap 删掉行 0 ⇒ 焦点仍指向同一行（槽位 −10）。
#[test]
fn swap_deleting_a_row_above_the_focused_row_rebases_the_focus_slot() {
    let Some((pack_id, pack_name, pack_path)) = first_installed_pack() else {
        eprintln!("[swap/focus] 本机没有已装的内置音源，跳过");
        return;
    };
    let bass_path = format!("assets/samples/{pack_id}/Programs/missing-{pack_id}.sfz");
    let (project, ids) = project_with_missing_tracks(
        "swap-rebase",
        &[
            ("Bass", bass_path),
            ("Lead", MISSING_PATH.to_owned()),
            ("Keys", MISSING_PATH.to_owned()),
        ],
    );
    let bass_id = ids[0];
    let lead_id = ids[1];
    // 行表前提：行 0 是已装包的轨，行 1/2 是没装的 —— 否则"swap 删行 0"测的不是这件。
    let rows = yeban_app::missing_sounds::collect(&project, None);
    assert_eq!(rows.len(), 3, "swap 焦点：夹具必须正好三行缺失");
    assert_eq!(
        rows[0].pack_id.as_deref(),
        Some(pack_id.as_str()),
        "swap 焦点：行 0 必须对应已装包 {pack_id}（否则 swap 换不上它）"
    );

    let (mut live, port) = live_window(&project, "swap-rebase");
    let ui = live.ui().clone_strong();
    assert!(ui.get_missing_sounds_open(), "打开工程就该弹出救援面板");
    assert_eq!(ui.get_missing_row_count(), 3, "swap 焦点：行数必须是 3");

    let focused = tab_into_row(&mut live, 1);
    assert!(
        (111..=114).contains(&focused),
        "swap 焦点：必须先落在行 1 的格子里（当前 {focused}）"
    );

    // 行 0「换成 X」：直接调生产回调。焦点在幸存行上时对话框内守卫不介入
    // （它只处理焦点在被删行上的情况）；鼠标点候选时焦点同样不动 ——
    // 直接调与真实走法在宿主侧是同一条删行路。
    ui.invoke_missing_swap_suggested(0, pack_path.clone().into(), pack_name.into());

    // ② 焦点必须仍指向同一行（旧槽位 −10），行数 2，面板还开着。
    assert_eq!(ui.get_missing_row_count(), 2, "swap 焦点：行数必须剩 2");
    assert!(
        ui.get_missing_sounds_open(),
        "swap 焦点：还有行没解决 ⇒ 面板还开着"
    );
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        focused - 10,
        "swap 焦点：焦点必须仍指向同一行（旧槽位 {focused} 重基为 {}）",
        focused - 10
    );
    let sampler = port
        .project()
        .tracks
        .get(&bass_id)
        .and_then(|track| track.sampler.clone())
        .expect("swap 焦点：行 0 的轨必须已经绑上");
    assert_eq!(
        sampler.path, pack_path,
        "swap 焦点：绑的必须正是刚换上的那一件"
    );
    // 同一条轨还在表里（行 1 的轨现在是行 0）。
    assert!(
        port.project().tracks.contains_key(&lead_id),
        "swap 焦点：幸存行（行 1 的轨）必须还在工程里"
    );

    // ③ 再 Tab 一圈：只剩两行的格子 + 「先不管」+ ✕，不越界、不卡死。
    let mut visited = vec![focused - 10];
    for _ in 0..10 {
        let slot = tab_once(&mut live);
        visited.push(slot);
        assert!(
            slot == 0 || slot == 1 || (101..=104).contains(&slot) || (111..=114).contains(&slot),
            "swap 焦点：删行后 Tab 只能走到幸存行的格子 / 1 / 0（走到 {slot}）"
        );
        if slot == 0 {
            break;
        }
    }
    assert_eq!(
        *visited.last().expect("至少走一步"),
        0,
        "swap 焦点：Tab 一圈必须绕回 ✕（走了 {visited:?}）"
    );
    report_line("missing-sounds-swap-focus rebase: focus-11x swap-to-10x tab-loop");
}
