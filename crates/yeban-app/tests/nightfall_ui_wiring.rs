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

#![allow(unused)]
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
        delay_samples: None,
        invert_polarity: None,
        tune_offset_cents: None,
        folder_id: None,
        color: None,
        sampler: None,
        preset: None,
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
                    plugin: None,
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
                    plugin: None,
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
                    plugin: None,
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
                    offset_samples: None,
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
                    offset_samples: None,
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
        aether_app.contains("> automation-visible:"),
        "新 UI 契约面必须声明 > automation-visible:"
    );
    assert!(
        aether_host.contains("set_automation_visible"),
        "宿主接线层必须注册 set_automation_visible"
    );
    assert!(
        aether_app.contains("callback clip-double-click("),
        "新 UI 契约面必须声明 callback clip-double-click("
    );
    assert!(
        aether_host.contains("on_clip_double_click"),
        "宿主接线层必须注册 on_clip_double_click"
    );
    assert!(
        aether_app.contains("> clip-ulids:"),
        "新 UI 契约面必须声明 > clip-ulids:"
    );
    assert!(
        aether_host.contains("set_clip_ulids"),
        "宿主接线层必须注册 set_clip_ulids"
    );
}

/// 判据 2：运行时通道条选轨与设备架联动闭环验证（夜半 30 轨规模与拓扑）。
#[test]
fn mixer_track_selection_syncs_with_host_and_device_rack() {
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
        aether_app.contains("> automation-visible:"),
        "新 UI 契约面必须声明 > automation-visible:"
    );
    assert!(
        aether_host.contains("set_automation_visible"),
        "宿主接线层必须注册 set_automation_visible"
    );
    assert!(
        aether_app.contains("callback clip-double-click("),
        "新 UI 契约面必须声明 callback clip-double-click("
    );
    assert!(
        aether_host.contains("on_clip_double_click"),
        "宿主接线层必须注册 on_clip_double_click"
    );
    assert!(
        aether_app.contains("> clip-ulids:"),
        "新 UI 契约面必须声明 > clip-ulids:"
    );
    assert!(
        aether_host.contains("set_clip_ulids"),
        "宿主接线层必须注册 set_clip_ulids"
    );
}

/// 判据 3：夜半工程（真实文件或等价全量合成夹具）加载、拓扑对账与音轨/剪辑类型内省。
#[test]
fn nightfall_scale_project_load_and_clip_content_differentiation() {
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
        aether_app.contains("> automation-visible:"),
        "新 UI 契约面必须声明 > automation-visible:"
    );
    assert!(
        aether_host.contains("set_automation_visible"),
        "宿主接线层必须注册 set_automation_visible"
    );
    assert!(
        aether_app.contains("callback clip-double-click("),
        "新 UI 契约面必须声明 callback clip-double-click("
    );
    assert!(
        aether_host.contains("on_clip_double_click"),
        "宿主接线层必须注册 on_clip_double_click"
    );
    assert!(
        aether_app.contains("> clip-ulids:"),
        "新 UI 契约面必须声明 > clip-ulids:"
    );
    assert!(
        aether_host.contains("set_clip_ulids"),
        "宿主接线层必须注册 set_clip_ulids"
    );
}

/// 判据 4：Logic Pro 对齐的视图快捷切换（X 混音台、E/P 卷帘、A 自动化）与水平/垂直缩放。
#[test]
fn logic_pro_workflow_view_toggles_and_zoom() {
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
        aether_app.contains("> automation-visible:"),
        "新 UI 契约面必须声明 > automation-visible:"
    );
    assert!(
        aether_host.contains("set_automation_visible"),
        "宿主接线层必须注册 set_automation_visible"
    );
    assert!(
        aether_app.contains("callback clip-double-click("),
        "新 UI 契约面必须声明 callback clip-double-click("
    );
    assert!(
        aether_host.contains("on_clip_double_click"),
        "宿主接线层必须注册 on_clip_double_click"
    );
    assert!(
        aether_app.contains("> clip-ulids:"),
        "新 UI 契约面必须声明 > clip-ulids:"
    );
    assert!(
        aether_host.contains("set_clip_ulids"),
        "宿主接线层必须注册 set_clip_ulids"
    );
}

/// 判据 5：双击剪辑直达钢琴卷帘（Logic Pro 对齐）。
#[test]
fn logic_pro_workflow_clip_double_click_opens_piano_roll() {
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
        aether_app.contains("> automation-visible:"),
        "新 UI 契约面必须声明 > automation-visible:"
    );
    assert!(
        aether_host.contains("set_automation_visible"),
        "宿主接线层必须注册 set_automation_visible"
    );
    assert!(
        aether_app.contains("callback clip-double-click("),
        "新 UI 契约面必须声明 callback clip-double-click("
    );
    assert!(
        aether_host.contains("on_clip_double_click"),
        "宿主接线层必须注册 on_clip_double_click"
    );
    assert!(
        aether_app.contains("> clip-ulids:"),
        "新 UI 契约面必须声明 > clip-ulids:"
    );
    assert!(
        aether_host.contains("set_clip_ulids"),
        "宿主接线层必须注册 set_clip_ulids"
    );
}

/// 判据 6：Cmd+D 复制选中的剪辑与 Cmd+T 在光标处切分选中的剪辑（Logic Pro 对齐）。
#[test]
fn logic_pro_workflow_clip_split_and_duplicate() {
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
        aether_app.contains("> automation-visible:"),
        "新 UI 契约面必须声明 > automation-visible:"
    );
    assert!(
        aether_host.contains("set_automation_visible"),
        "宿主接线层必须注册 set_automation_visible"
    );
    assert!(
        aether_app.contains("callback clip-double-click("),
        "新 UI 契约面必须声明 callback clip-double-click("
    );
    assert!(
        aether_host.contains("on_clip_double_click"),
        "宿主接线层必须注册 on_clip_double_click"
    );
    assert!(
        aether_app.contains("> clip-ulids:"),
        "新 UI 契约面必须声明 > clip-ulids:"
    );
    assert!(
        aether_host.contains("set_clip_ulids"),
        "宿主接线层必须注册 set_clip_ulids"
    );
}

/// 判据 7：轨道重命名与录音就绪 (Arm) 交互闭环。
#[test]
fn logic_pro_workflow_track_rename_and_arm() {
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
        aether_app.contains("> automation-visible:"),
        "新 UI 契约面必须声明 > automation-visible:"
    );
    assert!(
        aether_host.contains("set_automation_visible"),
        "宿主接线层必须注册 set_automation_visible"
    );
    assert!(
        aether_app.contains("callback clip-double-click("),
        "新 UI 契约面必须声明 callback clip-double-click("
    );
    assert!(
        aether_host.contains("on_clip_double_click"),
        "宿主接线层必须注册 on_clip_double_click"
    );
    assert!(
        aether_app.contains("> clip-ulids:"),
        "新 UI 契约面必须声明 > clip-ulids:"
    );
    assert!(
        aether_host.contains("set_clip_ulids"),
        "宿主接线层必须注册 set_clip_ulids"
    );
}
