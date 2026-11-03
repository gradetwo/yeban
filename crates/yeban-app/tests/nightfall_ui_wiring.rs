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
//!    设备链 (`device_track_label` / `device_names`) 与发送条 (`send_destination_names`)；
//! 3. **夜半完整工程加载与片段类型识别**：支持加载包含 30 轨、36 个剪辑池条目、56 条自动化曲线
//!    的夜半工程，精确识别 MIDI 与 Audio 片段内容；
//! 4. **CI 兼容与零回归**：若磁盘存在 `/home/crow/work/qoder/nightfall_v17.yeban`，直接进行真文件对账；
//!    若在 GitHub CI 无头环境（无本地大文件），通过同构合成夹具执行等价全量断言。

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::str::FromStr as _;

use slint::Model as _;
use yeban_app::bridge::{TrackHeightLayout, ViewState, display_track};
use yeban_app::host;
use yeban_app::open::open_project_file;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::music::MidiNote;
use yeban_model::project::{
    AutomationLane, AutomationTarget, ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition,
    DeviceKind, LoopConfig, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_760_000_000_000;
const NIGHTFALL_FILE: &str = "/home/crow/work/qoder/nightfall_v17.yeban";

fn eid(s: &str) -> EntityId {
    EntityId::from_str(s).expect("有效 EntityId 规范文本")
}

fn synth_track(id: EntityId, name: &str, kind: TrackKind) -> TrackV3 {
    TrackV3 {
        id,
        name: name.to_string(),
        kind,
        volume_db: 0.0,
        pan: 0.0,
        mute: false,
        solo: false,
        solo_safe: false,
        folder_id: None,
        color: None,
        sampler: None,
        devices: Vec::new(),
        macros: Vec::new(),
        automation_lanes: BTreeMap::new(),
        clips: BTreeMap::new(),
    }
}

/// 构造一个具有与 nightfall_v17.yeban 相同拓扑与规模的工程：
/// 30 轨（25 MIDI，4 AuxReturn，1 Master），包含 MIDI 与音频剪辑，自动化、路由与设备链。
fn synth_nightfall_scale_project() -> YebanProjectV1 {
    // nightfall_v17.yeban 实际轨道身份与类型（按 EntityId 升序排列）
    let track_defs: [(&str, &str, TrackKind); 30] = [
        (
            "01M6AKK32R0PPHTFNDQAVFX7FG",
            "Woodwinds_Arp",
            TrackKind::Midi,
        ),
        (
            "01M6AKK32R7TQAYA9BB7S8KE6C",
            "French_Horns_Theme",
            TrackKind::Midi,
        ),
        (
            "01M6AKK32RB0M80VX9MTSRCYVX",
            "Cello_Bass_Drone",
            TrackKind::Midi,
        ),
        (
            "01M6AKK32RNPFVZJ4W2FMDE3CX",
            "Cinematic_Taiko",
            TrackKind::Midi,
        ),
        (
            "01M6AKK32RWSJDWDG579X3NY40",
            "Choir_Synth_Pad",
            TrackKind::Midi,
        ),
        (
            "01YYYYYYYY0000000002F3EYDH",
            "Solo Violin · Hook",
            TrackKind::Midi,
        ),
        (
            "01YYYYYYYY00000075GYACZ4KK",
            "Cathedral · 9.5s",
            TrackKind::AuxReturn,
        ),
        (
            "01YYYYYYYY00000075KSSD8A88",
            "Ambience · Wind",
            TrackKind::Midi,
        ),
        (
            "01YYYYYYYY00000075M3NV43YC",
            "Harp · Reprise",
            TrackKind::Midi,
        ),
        (
            "01YYYYYYYY000001MSYQW9HCAA",
            "Choir_Synth_Pad · Haas",
            TrackKind::Midi,
        ),
        ("01ZZZ0AR5JZCS6K0DT7M1EV8N2", "Tape Air", TrackKind::Midi),
        (
            "01ZZZM416K0DT7M1EV8N2FW9P3",
            "Strings · Trem",
            TrackKind::Midi,
        ),
        ("01ZZZM41DT7M1EV8N2FW9P3GXA", "Final Bell", TrackKind::Midi),
        ("01ZZZM41M1EV8N2FW9P3GXAQ4H", "FX Riser", TrackKind::Midi),
        (
            "01ZZZM519TBWDYF0H2K4N6Q8SA",
            "Plate · 5.5s",
            TrackKind::AuxReturn,
        ),
        (
            "01ZZZZZZ01S323EHN89WZ580P1",
            "Violins · 8va",
            TrackKind::Midi,
        ),
        ("01ZZZZZZ026Z0B6NFBKM1AFTKN", "Sub · Drone", TrackKind::Midi),
        ("01ZZZZZZ03T33WWANZBZX6R9ZZ", "Low Brass", TrackKind::Midi),
        ("01ZZZZZZ0499YDYXBD2AJSTN37", "Taiko · Sub", TrackKind::Midi),
        ("01ZZZZZZ05KM0KPPYVA2RPCV8W", "Choir · 8va", TrackKind::Midi),
        ("01ZZZZZZ06A5YA4JHN7YBB0SMA", "Arp · 15ma", TrackKind::Midi),
        ("01ZZZZZZ07ZGVXXZ99421RC2S2", "Strings", TrackKind::Midi),
        ("01ZZZZZZ0849YPH77RE4EKZ0BD", "Low End", TrackKind::Midi),
        ("01ZZZZZZ093ZRQB2YHST1H2MK2", "Brass", TrackKind::Midi),
        ("01ZZZZZZ0AQ6XZ13A4J9YXWRGH", "Percussion", TrackKind::Midi),
        (
            "01ZZZZZZ0BWTT81DNMZTNM4A32",
            "Choir & Synth",
            TrackKind::Midi,
        ),
        (
            "01ZZZZZZ0CA3SMPMK4N2F9JD2N",
            "Hall · 3.6s",
            TrackKind::AuxReturn,
        ),
        (
            "01ZZZZZZ0DSKVT3QR58W9KPXDW",
            "Room · 0.8s",
            TrackKind::AuxReturn,
        ),
        ("0SPK09ZEQCWAT9DKHQMFGCDRSD", "Master", TrackKind::Master),
        (
            "55DWXS5XBTYGTQ5WNYFMNXAHTW",
            "Strings · Ostinato",
            TrackKind::Midi,
        ),
    ];

    let master_id = eid("0SPK09ZEQCWAT9DKHQMFGCDRSD");
    let mut tracks = BTreeMap::new();
    let mut routing_graph = yeban_model::project::RoutingGraph::default();

    for (id_str, name, kind) in track_defs {
        let tid = eid(id_str);
        routing_graph.nodes.push(tid);
        let mut t = synth_track(tid, name, kind);
        match kind {
            TrackKind::Master => {
                t.devices.push(DeviceDefinition {
                    id: EntityId::new(),
                    name: "Master CS".to_string(),
                    kind: DeviceKind::InternalEffect,
                    bypassed: false,
                    params: Vec::new(),
                    latency_samples: 0,
                });
            }
            TrackKind::AuxReturn => {
                t.devices.push(DeviceDefinition {
                    id: EntityId::new(),
                    name: format!("REV · {name}"),
                    kind: DeviceKind::InternalEffect,
                    bypassed: false,
                    params: Vec::new(),
                    latency_samples: 0,
                });
            }
            TrackKind::Midi => {
                t.devices.push(DeviceDefinition {
                    id: EntityId::new(),
                    name: format!("VA · {name}"),
                    kind: DeviceKind::InternalInstrument,
                    bypassed: false,
                    params: Vec::new(),
                    latency_samples: 0,
                });
                let target = AutomationTarget::TrackVolume { track_id: tid };
                t.automation_lanes
                    .insert(target, AutomationLane::implicit(target));
            }
            _ => {}
        }
        tracks.insert(tid, t);
    }

    // Clip pool: 22 MIDI clips, 14 Audio clips = 36 total
    let mut clip_pool = BTreeMap::new();
    let mut midi_clip_ids = Vec::new();
    for i in 0..22 {
        let cid = EntityId::new();
        midi_clip_ids.push(cid);
        let mut notes = BTreeMap::new();
        let nid = EntityId::new();
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
    let mut audio_clip_ids = Vec::new();
    for i in 0..14 {
        let cid = EntityId::new();
        audio_clip_ids.push(cid);
        clip_pool.insert(
            cid,
            ClipPoolEntry {
                id: cid,
                name: format!("Audio Stem {i}"),
                content: ClipContent::Audio {
                    asset: yeban_model::AssetHash::of_bytes(format!("stem-{i}").as_bytes()),
                    gain_db: 0.0,
                },
            },
        );
    }

    // 摆放 30 条片段：22 条 MIDI 摆放 + 8 条 Audio 摆放（精确对齐 nightfall_v17.yeban）
    let mut placement_count = 0;
    // 22 条 MIDI 摆放覆盖 MIDI 轨道（排除仅承载音频的轨道）
    let midi_track_ids: Vec<EntityId> = tracks
        .values()
        .filter(|t| t.kind == TrackKind::Midi && t.name != "Tape Air")
        .map(|t| t.id)
        .collect();
    for (i, tid) in midi_track_ids.iter().take(22).enumerate() {
        let pid = EntityId::new();
        let cid = midi_clip_ids[i % 22];
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
            placement_count += 1;
        }
    }
    // 8 条 Audio 摆放：1 条在 Tape Air, 6 条在 FX Riser, 1 条在 Ambience · Wind（对齐夜半真实工程）
    let tape_air_id = eid("01ZZZ0AR5JZCS6K0DT7M1EV8N2");
    let fx_riser_id = eid("01ZZZM41M1EV8N2FW9P3GXAQ4H");
    let wind_id = eid("01YYYYYYYY00000075KSSD8A88");

    let audio_targets = [
        (tape_air_id, 0),
        (fx_riser_id, 1),
        (fx_riser_id, 2),
        (fx_riser_id, 3),
        (fx_riser_id, 4),
        (fx_riser_id, 5),
        (fx_riser_id, 6),
        (wind_id, 7),
    ];
    for (i, (tid, clip_idx)) in audio_targets.iter().enumerate() {
        if let Some(t) = tracks.get_mut(tid) {
            let pid = EntityId::new();
            let cid = audio_clip_ids[*clip_idx];
            t.clips.insert(
                pid,
                ClipPlacement {
                    id: pid,
                    clip_id: cid,
                    start_tick: ((i + 22) as u64) * 960,
                    duration_ticks: 3840,
                    loop_config: LoopConfig::default(),
                    muted: false,
                },
            );
            placement_count += 1;
        }
    }
    assert_eq!(placement_count, 30, "合成夹具必须恰好摆放 30 条片段");

    // 路由图：构建真实 SendToAux 发送边（对齐夜半工程：Woodwinds_Arp 发送到 Hall 和 Room）
    let ww_id = eid("01M6AKK32R0PPHTFNDQAVFX7FG");
    let hall_id = eid("01ZZZZZZ0CA3SMPMK4N2F9JD2N");
    let room_id = eid("01ZZZZZZ0DSKVT3QR58W9KPXDW");

    let e1 = eid("01ZZZZZZ2JZMK2PDNECNWB749D");
    routing_graph.edges.insert(
        e1,
        yeban_model::project::RoutingEdge {
            id: e1,
            source_node: ww_id,
            destination_node: hall_id,
            kind: yeban_model::project::RoutingKind::SendToAux,
            gain_db: Some(-16.0),
        },
    );
    let e2 = eid("01ZZZZZZ2PNG8SN84H987MWK7Q");
    routing_graph.edges.insert(
        e2,
        yeban_model::project::RoutingEdge {
            id: e2,
            source_node: ww_id,
            destination_node: room_id,
            kind: yeban_model::project::RoutingKind::SendToAux,
            gain_db: Some(-20.0),
        },
    );

    YebanProjectV1 {
        title: "夜半 · Nightfall 规模测试夹具".to_string(),
        master_bus_track_id: master_id,
        tracks,
        clip_pool,
        routing_graph,
        ..Default::default()
    }
}

/// 判据 1：源码接线核验 —— 确保 MixerConsole、ConsoleTabs 与 MainWindow 的 track-select 完全闭环。
#[test]
fn slint_source_wirings_cover_mixer_track_selection() {
    let mixer = include_str!("../ui/console/mixer_console.slint").replace("\r\n", "\n");
    let tabs = include_str!("../ui/console/console_tabs.slint").replace("\r\n", "\n");
    let app = include_str!("../ui/app.slint").replace("\r\n", "\n");

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

/// 判据 2：运行时通道条选轨与设备架联动闭环验证（夜半 30 轨规模与拓扑）。
#[test]
fn mixer_track_selection_syncs_with_host_and_device_rack() {
    let (project, source_desc) = if Path::new(NIGHTFALL_FILE).exists() {
        let p = open_project_file(Path::new(NIGHTFALL_FILE))
            .expect("真实夜半工程 nightfall_v17.yeban 必须成功打开");
        (p, "真实 nightfall_v17.yeban 文件")
    } else {
        (synth_nightfall_scale_project(), "等价全量合成夹具")
    };
    println!("[nightfall-test] 正在验证 LivePort 运行时: {source_desc}");

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
    let track_names = view.track_names();
    assert_eq!(track_ids.len(), 29, "夜半工程有 29 条非主总线轨");
    assert_eq!(track_names.len(), 29, "非主总线轨道名数量必须为 29");
    assert_eq!(track_names[0], "Woodwinds_Arp");
    assert_eq!(track_names[14], "Plate · 5.5s");
    assert_eq!(track_names[28], "Strings · Ostinato");

    // 切换到混音台 Tab 1
    ui.set_console_tab(1);

    // 默认没有选轨
    assert_eq!(ui.get_selected_track_id().as_str(), "");

    // 模拟点击通道条 0 (Woodwinds_Arp)
    ui.invoke_track_select(0);
    assert_eq!(
        ui.get_selected_track_id().as_str(),
        track_ids[0].as_str(),
        "点击通道条 0 必须将 selected-track-id 更新为第 0 条轨道的 EntityId"
    );
    assert!(
        ui.get_device_track_label()
            .as_str()
            .contains("Woodwinds_Arp"),
        "设备架标题必须联动切换到 Woodwinds_Arp: {}",
        ui.get_device_track_label()
    );

    // 模拟点击 AuxReturn 通道条 (第 14 轨 Plate · 5.5s)
    ui.invoke_track_select(14);
    assert_eq!(
        ui.get_selected_track_id().as_str(),
        track_ids[14].as_str(),
        "点击通道条 14 (AuxReturn) 必须将 selected-track-id 更新为 Aux 轨道的 EntityId"
    );
    assert!(
        ui.get_device_track_label()
            .as_str()
            .contains("Plate · 5.5s"),
        "设备架标题必须联动切换到 Plate · 5.5s: {}",
        ui.get_device_track_label()
    );

    // 模拟点击末尾通道条 (第 28 轨 Strings · Ostinato)
    ui.invoke_track_select(28);
    assert_eq!(
        ui.get_selected_track_id().as_str(),
        track_ids[28].as_str(),
        "点击通道条 28 必须更新为末尾轨道的 EntityId"
    );
    assert!(
        ui.get_device_track_label()
            .as_str()
            .contains("Strings · Ostinato"),
        "设备架标题必须联动切换到 Strings · Ostinato: {}",
        ui.get_device_track_label()
    );

    // 再次切回通道条 0，验证发送条与路由图联动
    ui.invoke_track_select(0);
    assert_eq!(ui.get_selected_track_id().as_str(), track_ids[0].as_str());
    let view_ww = ViewState::from_project_with_zoom_layout_and_selection(
        &project,
        30,
        &TrackHeightLayout::default(),
        Some(track_ids[0].as_str()),
    )
    .expect("投影 Woodwinds_Arp 选轨视图");
    let dests = view_ww.send_destination_names();
    assert_eq!(
        dests.len(),
        2,
        "Woodwinds_Arp 必须恰好投射出 2 条 SendToAux 目的地"
    );
    assert!(dests.iter().any(|name| name.contains("Hall · 3.6s")));
    assert!(dests.iter().any(|name| name.contains("Room · 0.8s")));

    // 选 AuxReturn 轨道时验证发送条为空
    let view_plate = ViewState::from_project_with_zoom_layout_and_selection(
        &project,
        30,
        &TrackHeightLayout::default(),
        Some(track_ids[14].as_str()),
    )
    .expect("投影 Plate · 5.5s 选轨视图");
    assert!(
        view_plate.send_destination_names().is_empty(),
        "AuxReturn 轨道没有 SendToAux 出边"
    );
}

/// 判据 3：夜半工程（真实文件或等价全量合成夹具）加载、拓扑对账与音轨/剪辑类型内省。
#[test]
fn nightfall_scale_project_load_and_clip_content_differentiation() {
    let (project, source_desc) = if Path::new(NIGHTFALL_FILE).exists() {
        let p = open_project_file(Path::new(NIGHTFALL_FILE))
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

    // 验证轨道类型内省
    let kinds = view.track_kinds();
    assert_eq!(kinds.len(), 29, "非主总线轨道类型数组长度必须恰好为 29");
    let midi_track_count = kinds.iter().filter(|&&k| k == "midi").count();
    let aux_track_count = kinds.iter().filter(|&&k| k == "aux-return").count();
    assert_eq!(midi_track_count, 25, "必须恰好有 25 条 MIDI 轨道");
    assert_eq!(aux_track_count, 4, "必须恰好有 4 条 AuxReturn 轨道");

    // 验证剪辑类型内省
    let contents = view.clip_contents();
    let names = view.clip_names();
    assert_eq!(
        contents.len(),
        30,
        "clip_contents 长度必须与 30 条摆放剪辑完全一致"
    );
    assert_eq!(
        names.len(),
        30,
        "clip_names 长度必须与 30 条摆放剪辑完全一致"
    );

    // 验证同时存在 MIDI 和 Audio 剪辑
    let has_midi = contents.contains(&"midi");
    let has_audio = contents.contains(&"audio");
    assert!(
        has_midi && has_audio,
        "夜半工程必须同时包含 MIDI 与 Audio 片段内容，识别结果: midi={}, audio={}",
        has_midi,
        has_audio
    );
    let midi_count = contents.iter().filter(|&&c| c == "midi").count();
    let audio_count = contents.iter().filter(|&&c| c == "audio").count();
    assert_eq!(midi_count, 22, "夜半工程摆放中必须恰好有 22 条 MIDI 片段");
    assert_eq!(audio_count, 8, "夜半工程摆放中必须恰好有 8 条 Audio 片段");

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
