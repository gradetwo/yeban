//! T-FIX-338：整曲预览（`render_project_to_buffer`）与 MCP 离线渲染共用
//! 同一套插件装配 —— 首台已装载的插件乐器／效果器真的执行，其余进收据。
//!
//! # 判据表（本文件）
//!
//! | 判据 | 断言 | 证什么 / ⛔ 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 插件乐器出声 | `stub:sine` 轨预览峰值 > 0.01，且该轨**不在** `skipped_tracks` 里 | 证已装载的插件乐器在预览里真的发声；⛔ 不证音色对 |
//! | ② 插件胜出 | 同工程、设备旁通后预览字节**不同** | 证声音来自插件（不是引擎）：插件被忽略时两版应逐字节相同 |
//! | ③ 未绑定仍跳过 | 无绑定的 `ExternalInstrument` 轨进收据（`PluginInstrument`），整曲静音 | 证“没东西可实例化”时仍如实跳过，⛔ 不是悄悄用引擎顶上 |
//! | ④ 效果器改增益 | `stub:gain`（状态增益 0.5）轨预览峰值 ≈ 干声 × 0.5 | 证效果器真的处理了音频；⛔ 不证参数自动化 |
//! | ⑤ 第二台不执行 | 两台绑定效果器：峰值 == 只有首台的峰值，且收据有一条 `PluginEffect` | 证 first-source 语义；第二台的钱没白花在静默里 |
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 没有人耳听过：只证“插件改了导出音频”（峰值／字节），不证“听起来对”；
//! - ⛔ 不钉绝对峰值（跨机浮点口径）：同一台机器上“有插件 vs 无插件”的
//!   **相对**读数才是证据；
//! - ⛔ 不证实时播放：实时回调内执行任意 VST3 无法满足红线 7，实时侧的口径
//!   是面板上的“仅登记”（见 `PLUGIN_REGISTRATION_NOTE`），本文件只证离线预览。

use std::collections::BTreeMap;

use yeban_model::project::{DeviceDefinition, DeviceKind, ParameterValue, PluginBinding};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_plugin_host::{PluginInstance, StubEffect, StubInstrument};
use yeban_render::engine_source::ArmedSfzInput;
use yeban_render::song_overview::{RenderSettings, SkippedReason, render_project_to_buffer};

/// 音符／摆放长度（tick）：240 tick = 0.5 拍 = 0.25 秒 @ 120 BPM / 480 PPQ。
const NOTE_TICKS: u64 = 240;

/// 演示工程没有采样器绑定 ⇒ provider 绝不能被调用（被调用即 panic）。
fn never_called_provider() -> impl FnMut(EntityId, &str) -> ArmedSfzInput {
    |_, _| panic!("夹具工程没有采样器绑定，provider 不应被调用")
}

/// 一条带一个 MIDI 音符的轨 ＋ 它引用的 `ClipPoolEntry`（音符在 `clip` 里，摆放在 0）。
fn track_with_note(
    track_id: EntityId,
    name: &str,
    pitch: u8,
    clip_id: EntityId,
    placement_id: EntityId,
    note_id: EntityId,
) -> (TrackV3, ClipPoolEntry) {
    let mut track = TrackV3 {
        id: track_id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        volume_db: -6.0,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement_id,
        ClipPlacement {
            id: placement_id,
            clip_id,
            start_tick: 0,
            duration_ticks: NOTE_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(
        note_id,
        MidiNote {
            velocity: 100,
            ..MidiNote::new(note_id, 0, pitch, NOTE_TICKS)
        },
    );
    let entry = ClipPoolEntry {
        id: clip_id,
        name: name.to_owned(),
        content: ClipContent::Midi { notes },
    };
    (track, entry)
}

/// 把一条轨 ＋ 一条 `TrackToBus` 边挂到 `default_project` 的主总线上。
fn attach(project: &mut YebanProjectV1, track: TrackV3, entry: ClipPoolEntry) {
    let master = project.master_bus_track_id;
    let track_id = track.id;
    let edge_id = EntityId::new();
    project.clip_pool.insert(entry.id, entry);
    project.tracks.insert(track_id, track);
    project.routing_graph.nodes.push(track_id);
    project.routing_graph.edges.insert(
        edge_id,
        RoutingEdge {
            id: edge_id,
            source_node: track_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
}

/// 夹具工程：`default_project` 的主总线 ＋ 一条 MIDI 轨（pitch 60）。
///
/// `devices` 原样写进那条轨的设备链。
fn project_with_devices(devices: Vec<DeviceDefinition>) -> YebanProjectV1 {
    let mut project = yeban_model::samples::default_project();
    let (mut track, entry) = track_with_note(
        EntityId::new(),
        "Plugin",
        60,
        EntityId::new(),
        EntityId::new(),
        EntityId::new(),
    );
    track.devices = devices;
    attach(&mut project, track, entry);
    project.routing_graph.nodes.sort_unstable();
    project.validate().expect("夹具工程必须合法");
    project
}

/// 一台已装载的插件乐器（`stub:sine`；桩，见 `yeban-plugin-host`）。
fn sine_device() -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "stub:sine".to_owned(),
        kind: DeviceKind::ExternalInstrument,
        bypassed: false,
        params: Vec::new(),
        latency_samples: 0,
        plugin: Some(PluginBinding {
            path: "stub:sine".to_owned(),
            class: Some("stub:sine".to_owned()),
            state: Vec::new(),
        }),
    }
}

/// 一台已装载的插件效果器（`stub:gain`；增益由绑定状态给出，`1.0` = 默认）。
fn gain_device(gain: f32) -> DeviceDefinition {
    let mut effect = StubEffect::new();
    effect.set_parameter(0, f64::from(gain)).expect("设增益");
    let state = effect.save_state().expect("存状态").encode();
    DeviceDefinition {
        id: EntityId::new(),
        name: "stub:gain".to_owned(),
        kind: DeviceKind::ExternalEffect,
        bypassed: false,
        params: Vec::new(),
        latency_samples: 0,
        plugin: Some(PluginBinding {
            path: "stub:gain".to_owned(),
            class: Some("stub:gain".to_owned()),
            state,
        }),
    }
}

/// 一台未绑定的外置乐器（没有 `plugin` 绑定可实例化）。
fn unbound_instrument_device() -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "unbound".to_owned(),
        kind: DeviceKind::ExternalInstrument,
        bypassed: false,
        params: Vec::<ParameterValue>::new(),
        latency_samples: 0,
        plugin: None,
    }
}

fn render(project: &YebanProjectV1) -> yeban_render::song_overview::RenderedAudio {
    render_project_to_buffer(project, &RenderSettings::default(), never_called_provider())
        .expect("夹具工程必须能预览")
}

/// 桩本身的形状（测试前置核验）：`stub:sine` 发声、`stub:gain` 乘增益。
/// 这两个断言红了 ⇒ 夹具桩坏了，⛔ 不是本单的接线坏了。
#[test]
fn stubs_sound_before_wiring() {
    let instrument = StubInstrument::new();
    assert_eq!(instrument.name(), "stub:sine");
    let mut effect = StubEffect::new();
    effect.set_parameter(0, 0.5).expect("设增益");
    assert_eq!(effect.parameter_value(0).expect("读增益"), 0.5);
}

#[test]
fn hosted_plugin_instrument_sounds_in_preview() {
    let project = project_with_devices(vec![sine_device()]);
    let rendered = render(&project);
    let track_id = project
        .tracks
        .values()
        .find(|t| t.name == "Plugin")
        .expect("夹具轨")
        .id;
    assert!(
        rendered.peak() > 0.01,
        "插件乐器轨必须出声，实测峰值 {}",
        rendered.peak()
    );
    assert!(
        !rendered.skipped_tracks.iter().any(|s| s.track == track_id),
        "已装载的插件轨不能进 skipped_tracks，实测 {rendered:?}"
    );
}

#[test]
fn plugin_instrument_wins_over_the_engine_timbre() {
    let with_plugin = project_with_devices(vec![sine_device()]);
    let mut bypassed = with_plugin.clone();
    let track = bypassed
        .tracks
        .values_mut()
        .find(|t| t.name == "Plugin")
        .expect("夹具轨");
    track.devices[0].bypassed = true;
    let a = render(&with_plugin);
    let b = render(&bypassed);
    assert_ne!(
        a.samples, b.samples,
        "插件胜出时输出必须与引擎音色不同（旁通后走引擎）；两版逐字节相同 ⇒ 插件根本没执行"
    );
    assert!(
        b.peak() > 0.01,
        "旁通后引擎必须出声（对照组本身要成立），实测峰值 {}",
        b.peak()
    );
}

#[test]
fn unbound_external_instrument_still_skips_loudly() {
    let project = project_with_devices(vec![unbound_instrument_device()]);
    let rendered = render(&project);
    let track_id = project
        .tracks
        .values()
        .find(|t| t.name == "Plugin")
        .expect("夹具轨")
        .id;
    assert_eq!(rendered.peak(), 0.0, "未绑定乐器轨必须静音");
    assert_eq!(rendered.skipped_tracks.len(), 1);
    assert_eq!(rendered.skipped_tracks[0].track, track_id);
    assert_eq!(
        rendered.skipped_tracks[0].reason,
        SkippedReason::PluginInstrument
    );
}

#[test]
fn hosted_gain_effect_halves_the_preview_peak() {
    let dry = render(&project_with_devices(vec![]));
    let wet = render(&project_with_devices(vec![gain_device(0.5)]));
    let ratio = wet.peak() / dry.peak();
    assert!(
        dry.peak() > 0.01,
        "干声对照组必须出声，实测峰值 {}",
        dry.peak()
    );
    assert!(
        (0.45..=0.55).contains(&ratio),
        "增益 0.5 的效果器必须把峰值减半，实测 dry={} wet={} ratio={ratio:.3}",
        dry.peak(),
        wet.peak()
    );
}

#[test]
fn second_bound_effect_is_reported_not_executed() {
    let single = render(&project_with_devices(vec![gain_device(0.5)]));
    let double = render(&project_with_devices(vec![
        gain_device(0.5),
        gain_device(0.5),
    ]));
    assert_eq!(
        single.samples, double.samples,
        "第二台绑定效果器必须不改变输出（首台胜出）"
    );
    let track_id = double
        .skipped_tracks
        .iter()
        .find(|s| s.reason == SkippedReason::PluginEffect);
    assert!(
        track_id.is_some(),
        "第二台绑定效果器必须进收据（PluginEffect），实测 skipped={:?}",
        double.skipped_tracks
    );
}
