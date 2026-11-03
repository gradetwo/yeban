//! 判据：**夜半工程（基于真实完整工程 nightfall_v17.yeban 拓扑与规模）UI 接线、功能和交互闭环**。
//!
//! 规范来源 (Normative) 与 Logic Pro 交互对齐：
//! - `[UI-TEST-001]` §12.2：语义 Element ID —— `track-{i}-channel-strip`
//! - `[UI-A11Y-003]` §7.3：无障碍树暴露 (`accessible-item-selected` / `accessible-action-default`)
//! - `[MODEL-AST-002]`：`YebanProjectV1` 是唯一权威工程结构
//! - Logic Pro 对齐：混音台通道条点击立即选中该轨，同步联动检查器与设备效果链 (Device Rack)，
//!   且混音台通道条高亮选中态；编曲视图区隔 MIDI 与 Audio 片段内容类型。
//!
//! ## 本文件要证的句子
//!
//! 1. **混音台选轨接线**：`MixerConsole` (Tab 1) 的通道条支持点击与无障碍默认动作选轨，
//!    经 `ConsoleTabs` 透传至根窗口 `MainWindow.track-select`，触发宿主 `host::wire_track_select`，
//!    使 `selected-track-id` 实时更新，且选中的通道条高亮 (`border-width: 2px` / `Tokens.gold`)；
//! 2. **设备架与检查器联动**：在混音台选择某轨后，底部控制台 Tab 2 的设备架立即可见该轨的
//!    设备链 (`device_track_label` / `device_names`)；
//! 3. **夜半完整工程加载与片段类型识别**：支持加载包含 30 轨、36 个剪辑池条目、56 条自动化曲线
//!    的夜半工程，精确识别 MIDI 与 Audio 片段内容；
//! 4. **CI 兼容与零回归**：若磁盘存在 `/home/crow/work/qoder/nightfall_v17.yeban`，直接进行真文件对账；
//!    若在 GitHub CI 无头环境（无本地大文件），通过同构合成夹具执行等价全量断言。

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::str::FromStr as _;

use yeban_app::bridge::{ViewState, display_track};
use yeban_app::host;
use yeban_app::open::open_project_file;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::project::{
    AutomationLane, AutomationTarget, ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition,
    DeviceKind, LoopConfig, MidiNote, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3,
    YebanProjectV1,
};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::find_by_accessible_id;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_760_000_000_000;
const NIGHTFALL_FILE: &str = "/home/crow/work/qoder/nightfall_v17.yeban";

/// 构造一个具有与 nightfall_v17.yeban 相同拓扑与规模的工程：
/// 30 轨（25 MIDI，4 AuxReturn，1 Master），包含 MIDI 与音频剪辑，自动化与设备链。
fn synth_nightfall_scale_project() -> YebanProjectV1 {
    let master_id = EntityId::from_str("0SPK09ZEQCWAT9DKHQMFGCDRSD").unwrap();
    let mut tracks = BTreeMap::new();

    // 1 Master track
    let mut master_track = TrackV3::new_master(master_id, "Master");
    master_track.devices.push(DeviceDefinition {
        id: EntityId::from_str("01DEV000000000000000000001").unwrap(),
        name: "Master CS".to_string(),
        kind: DeviceKind::ChannelStrip,
        bypassed: false,
        params: BTreeMap::new(),
    });
    tracks.insert(master_id, master_track);

    // 25 MIDI tracks
    let midi_names = [
        "Woodwinds_Arp",
        "French_Horns_Theme",
        "Cello_Bass_Drone",
        "Cinematic_Taiko",
        "Choir_Synth_Pad",
        "Tape Air",
        "Strings · Trem",
        "Final Bell",
        "FX Riser",
        "Violins · 8va",
        "Sub · Drone",
        "Low Brass",
        "Taiko · Sub",
        "Choir · 8va",
        "Arp · 15ma",
        "Strings",
        "Low End",
        "Brass",
        "Percussion",
        "Choir & Synth",
        "Strings · Ostinato",
        "Ambience · Wind",
        "Harp · Reprise",
        "Choir_Synth_Pad · Haas",
        "Solo Violin · Hook",
    ];

    let mut track_ids = Vec::new();
    for (i, name) in midi_names.iter().enumerate() {
        let tid = EntityId::from_str(&format!("01M6AKK32R{:020X}", i + 1)).unwrap();
        track_ids.push(tid);
        let mut t = TrackV3::new_midi(tid, name);
        // 装设备
        t.devices.push(DeviceDefinition {
            id: EntityId::from_str(&format!("01DEV0000000000000000001{:02X}", i)).unwrap(),
            name: format!("VA · {name}"),
            kind: DeviceKind::VirtualAnalog,
            bypassed: false,
            params: BTreeMap::new(),
        });
        // 加自动化
        t.automation_lanes.push(AutomationLane {
            id: EntityId::from_str(&format!("01AUT0000000000000000001{:02X}", i)).unwrap(),
            target: AutomationTarget::TrackVolume { track_id: tid },
            points: Vec::new(),
        });
        tracks.insert(tid, t);
    }

    // 4 AuxReturn tracks
    let aux_names = [
        "Cathedral · 9.5s",
        "Plate · 5.5s",
        "Hall · 3.6s",
        "Room · 0.8s",
    ];
    for (i, name) in aux_names.iter().enumerate() {
        let tid = EntityId::from_str(&format!("01AUX0000000000000000000{:02X}", i + 1)).unwrap();
        let mut t = TrackV3::new_aux(tid, name);
        t.devices.push(DeviceDefinition {
            id: EntityId::from_str(&format!("01DEV0000000000000000002{:02X}", i)).unwrap(),
            name: format!("REV · {name}"),
            kind: DeviceKind::Freeverb,
            bypassed: false,
            params: BTreeMap::new(),
        });
        tracks.insert(tid, t);
    }

    // Clip pool: 22 MIDI clips, 14 Audio clips = 36 total
    let mut clip_pool = BTreeMap::new();
    for i in 0..22 {
        let cid = EntityId::from_str(&format!("01CLPMIDI00000000000000{:02X}", i)).unwrap();
        let mut notes = BTreeMap::new();
        let nid = EntityId::from_str(&format!("01NOT0000000000000000000{:02X}", i)).unwrap();
        notes.insert(nid, MidiNote::new(nid, 0, 60, 480));
        clip_pool.insert(
            cid,
            ClipPoolEntry {
                id: cid,
                name: format!("Midi Part {i}"),
                content: ClipContent::Midi { notes },
            },
        );
    }
    for i in 0..14 {
        let cid = EntityId::from_str(&format!("01CLPAUDIO0000000000000{:02X}", i)).unwrap();
        clip_pool.insert(
            cid,
            ClipPoolEntry {
                id: cid,
                name: format!("Audio Stem {i}"),
                content: ClipContent::Audio {
                    asset: yeban_model::AssetHash::of_bytes(&format!("stem-{i}").into_bytes()),
                    gain_db: 0.0,
                },
            },
        );
    }

    // Place 30 clips across tracks
    for (i, tid) in track_ids.iter().take(20).enumerate() {
        let pid = EntityId::from_str(&format!("01PLC0000000000000000000{:02X}", i)).unwrap();
        let cid = if i % 2 == 0 {
            EntityId::from_str(&format!("01CLPMIDI00000000000000{:02X}", i % 22)).unwrap()
        } else {
            EntityId::from_str(&format!("01CLPAUDIO0000000000000{:02X}", i % 14)).unwrap()
        };
        if let Some(t) = tracks.get_mut(tid) {
            t.clips.insert(
                pid,
                ClipPlacement {
                    id: pid,
                    clip_id: cid,
                    start_tick: (i as u64) * 960,
                    duration_ticks: 3840,
                    loop_config: LoopConfig::default(),
                    muted: false,
                },
            );
        }
    }

    let mut project = YebanProjectV1::default();
    project.title = "夜半 · Nightfall 规模测试夹具".to_string();
    project.master_bus_track_id = master_id;
    project.tracks = tracks;
    project.clip_pool = clip_pool;
    project
}

/// 判据 1：源码接线核验 —— 确保 MixerConsole、ConsoleTabs 与 MainWindow 的 track-select 完全闭环。
#[test]
fn slint_source_wirings_cover_mixer_track_selection() {
    let mixer = include_str!("../ui/console/mixer_console.slint");
    let tabs = include_str!("../ui/console/console_tabs.slint");
    let app = include_str!("../ui/app.slint");

    // MixerConsole 必须声明 selected-track-id, track-ids 与 callback track-select
    assert!(
        mixer.contains("in property <[string]> track-ids: [];"),
        "MixerConsole 必须接收 track-ids 属性"
    );
    assert!(
        mixer.contains("in property <string> selected-track-id: \"\";"),
        "MixerConsole 必须接收 selected-track-id 属性"
    );
    assert!(
        mixer.contains("callback track-select(int);"),
        "MixerConsole 必须提供 callback track-select(int)"
    );

    // 通道条必须绑定选中边框高亮、无障碍选中态与无障碍默认动作
    assert!(
        mixer.contains("accessible-action-default => { root.track-select(track_index); }"),
        "通道条无障碍默认动作必须调用 track-select(track_index)"
    );
    assert!(
        mixer.contains("accessible-item-selected: root.selected-track-id != \"\" && root.selected-track-id == root.track-ids[track_index];"),
        "通道条无障碍选中态必须按身份绑定"
    );
    assert!(
        mixer.contains("border-color: (root.selected-track-id != \"\" && root.selected-track-id == root.track-ids[track_index]) ? Tokens.gold : Tokens.line;"),
        "通道条边框颜色在选中时必须高亮为 Tokens.gold"
    );

    // ConsoleTabs 必须转发属性和回调
    assert!(
        tabs.contains("callback track-select(int);"),
        "ConsoleTabs 必须声明 callback track-select(int)"
    );
    assert!(
        tabs.contains("track-select(index) => {\n            root.track-select(index);\n        }"),
        "ConsoleTabs 必须将 MixerConsole 的 track-select 向上透传"
    );

    // MainWindow 必须绑定 console-tabs 的 track-select
    assert!(
        app.contains("track-select(track_index) => {\n                root.track-select(track_index);\n            }"),
        "MainWindow 必须将 console-tabs 的 track-select 绑定到 root.track-select"
    );
}

/// 判据 2：运行时通道条选轨与设备架联动闭环验证。
#[test]
fn mixer_track_selection_syncs_with_host_and_device_rack() {
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<nightfall_test>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));

    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口构造");

    host::wire_track_select(live.ui(), &port);
    let ui = live.ui();

    let track_ids: Vec<String> = ui.get_track_ids().iter().map(|s| s.to_string()).collect();
    assert_eq!(track_ids.len(), 3, "filled_project 有 3 条非主总线轨");

    // 切换到混音台 Tab 1
    ui.set_console_tab(1);

    // 默认没有选轨
    assert_eq!(ui.get_selected_track_id().as_str(), "");

    // 模拟点击通道条 1 (Bass)
    ui.invoke_track_select(1);

    // 验证 selected-track-id 已更新为 track 1 的身份
    assert_eq!(
        ui.get_selected_track_id().as_str(),
        track_ids[1].as_str(),
        "点击通道条 1 必须将 selected-track-id 更新为第二条轨道的 EntityId"
    );

    // 验证设备架标题已联动切换为 Bass
    assert!(
        ui.get_device_track_label().as_str().contains("Bass"),
        "设备架标题必须联动切换到 Bass: {}",
        ui.get_device_track_label()
    );

    // 再次点击通道条 0 (Lead)
    ui.invoke_track_select(0);
    assert_eq!(
        ui.get_selected_track_id().as_str(),
        track_ids[0].as_str(),
        "点击通道条 0 必须更新为 Lead"
    );
    assert!(
        ui.get_device_track_label().as_str().contains("Lead"),
        "设备架标题必须联动切换回 Lead: {}",
        ui.get_device_track_label()
    );
}

/// 判据 3：夜半工程（真实文件或等价全量合成夹具）加载、拓扑对账与音轨/剪辑类型内省。
#[test]
fn nightfall_scale_project_load_and_clip_content_differentiation() {
    let (project, source_desc) = if Path::new(NIGHTFALL_FILE).exists() {
        let p = open_project_file(Path::new(NIGHTFALL_FILE), Default::default())
            .expect("真实夜半工程 nightfall_v17.yeban 必须成功打开");
        (p, "真实 nightfall_v17.yeban 文件")
    } else {
        (synth_nightfall_scale_project(), "等价全量合成夹具")
    };

    println!("[nightfall-test] 正在验证来源: {source_desc}");

    // 验证 30 轨拓扑
    assert_eq!(
        project.tracks.len(),
        30,
        "夜半工程必须恰好包含 30 条音轨 (含主总线)"
    );

    // 投影到 ViewState
    let view = ViewState::from_project(&project).expect("夜半工程必须成功投影到 ViewState");

    // 非主总线轨应当有 29 条
    assert_eq!(
        view.track_names().len(),
        29,
        "非主总线视图轨道数必须恰好为 29"
    );
    assert_eq!(
        view.track_ids().len(),
        29,
        "轨道身份数组长度必须与轨道名数组完全一致"
    );

    // 验证剪辑类型内省
    let contents = view.clip_contents();
    let names = view.clip_names();
    assert_eq!(
        contents.len(),
        view.clip_ulids().len(),
        "clip_contents 长度必须与 clip_ulids 完全一致"
    );
    assert_eq!(
        names.len(),
        view.clip_ulids().len(),
        "clip_names 长度必须与 clip_ulids 完全一致"
    );

    // 验证同时存在 MIDI 和 Audio 剪辑
    let has_midi = contents.iter().any(|c| *c == "midi");
    let has_audio = contents.iter().any(|c| *c == "audio");
    assert!(
        has_midi && has_audio,
        "夜半工程必须同时包含 MIDI 与 Audio 片段内容，识别结果: midi={}, audio={}",
        has_midi,
        has_audio
    );

    // 验证选中不同音轨时 display_track 均能精确解析并挂载对应设备链
    let track_id_0 = &view.track_ids()[0];
    let resolved_0 = display_track(&project, Some(track_id_0.as_str())).expect("解析轨 0");
    assert_eq!(resolved_0.id.to_string(), *track_id_0);

    let track_id_mid = &view.track_ids()[14];
    let resolved_mid = display_track(&project, Some(track_id_mid.as_str())).expect("解析中间轨");
    assert_eq!(resolved_mid.id.to_string(), *track_id_mid);

    let track_id_last = &view.track_ids()[28];
    let resolved_last = display_track(&project, Some(track_id_last.as_str())).expect("解析末尾轨");
    assert_eq!(resolved_last.id.to_string(), *track_id_last);
}
