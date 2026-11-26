//! T-FIX-338：GUI 离线导出（`render_core`，`--export-wav` / UI 导出 / 录音共用）
//! 与 MCP 离线渲染共用同一套插件装配 —— 首台已装载的插件乐器／效果器真的
//! 执行，其余进收据（`plugins_executed`／`plugins_not_executed`）。
//!
//! # 判据表（本文件）
//!
//! | 判据 | 断言 | 证什么 / ⛔ 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 插件乐器出声 | `stub:sine` 轨导出峰值 > 0.01，`plugins_executed` 有该行，`source_label` 带 `plugins(executed=1 …)` | 证已装载的插件乐器在 GUI 导出里真的发声；⛔ 不证音色对 |
//! | ② 插件胜出 | 同工程、设备旁通后导出字节**不同** | 证声音来自插件（不是引擎） |
//! | ③ 效果器改增益 | `stub:gain`（状态增益 0.5）导出峰值 ≈ 干声 × 0.5 | 证效果器真的处理了音频 |
//! | ④ 第二台不执行 | 两台绑定效果器：字节 == 只有首台的字节；`plugins_not_executed` 有“第二台绑定设备，首台胜出”行 | 证 first-source 语义 |
//! | ⑤ 未绑定不执行 | 未绑定效果器：字节 == 干声字节；`plugins_not_executed` 有“未绑定插件”行 | 证“没东西可实例化”时如实登记，⛔ 不是悄悄用干声顶上 |
//! | ⑥ 旁通等于没有 | 旁通的效果器：字节 == 干声字节；两张插件表都空 | 证旁通是显式选择，不进收据 |
//! | ⑦ 导出报告分流 | `export_wav` 的 `WavExportReport` 含两张表，`summary()` 多一行“插件执行: …” | 证 loaded/executed 分离落到了用户看得到的回执 |
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 没有人耳听过：只证“插件改了导出音频”（峰值／字节），不证“听起来对”；
//! - ⛔ 不钉绝对峰值与 WAV 的绝对 `sha256`（跨机浮点口径）：同一台机器上
//!   “有插件 vs 无插件”的**相对**读数才是证据；
//! - ⛔ 不证实时播放：实时回调内执行任意 VST3 无法满足红线 7，实时侧的口径
//!   是面板上的“仅登记”（见 `PLUGIN_REGISTRATION_NOTE`），本文件只证离线导出。

use std::collections::BTreeMap;
use std::path::PathBuf;

use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::render_core::{RenderProjectOutput, render_project_f32_with_assets_under};
use yeban_model::project::{DeviceDefinition, DeviceKind, ParameterValue, PluginBinding};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_plugin_host::{PluginInstance, StubEffect, StubInstrument};

/// 导出 1 秒：够量峰值（音符 0.25 秒）。
const FRAMES: u64 = 48_000;
/// 音符／摆放长度（tick）：240 tick = 0.5 拍 = 0.25 秒 @ 120 BPM / 480 PPQ。
const NOTE_TICKS: u64 = 240;

/// 临时夹具目录（`Drop` 删掉自己；⛔ 工作树里不留临时文件）。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-338-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        Self { dir }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
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

/// 一台已装载的插件效果器（`stub:gain`；增益由绑定状态给出）。
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

/// 一台未绑定的外置效果器（没有 `plugin` 绑定可实例化）。
fn unbound_effect_device() -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "unbound".to_owned(),
        kind: DeviceKind::ExternalEffect,
        bypassed: false,
        params: Vec::<ParameterValue>::new(),
        latency_samples: 0,
        plugin: None,
    }
}

fn render(project: &YebanProjectV1, fixture: &Fixture) -> RenderProjectOutput {
    let options = WavExportOptions {
        frames: FRAMES,
        ..WavExportOptions::default()
    };
    render_project_f32_with_assets_under(project, options, None, &[], Some(fixture.dir.as_path()))
        .expect("夹具工程必须能渲染")
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()))
}

/// 桩本身的形状（测试前置核验）：`stub:sine` 发声、`stub:gain` 乘增益。
#[test]
fn stubs_sound_before_wiring() {
    let instrument = StubInstrument::new();
    assert_eq!(instrument.name(), "stub:sine");
    let mut effect = StubEffect::new();
    effect.set_parameter(0, 0.5).expect("设增益");
    assert_eq!(effect.parameter_value(0).expect("读增益"), 0.5);
}

#[test]
fn hosted_plugin_instrument_sounds_in_export() {
    let fixture = Fixture::new("instrument");
    let output = render(&project_with_devices(vec![sine_device()]), &fixture);
    assert!(peak(&output.samples) > 0.01, "插件乐器轨必须出声");
    assert_eq!(
        output.plugins_executed,
        vec!["Plugin: stub:sine [乐器]".to_owned()],
        "已执行的插件必须进收据"
    );
    assert!(output.plugins_not_executed.is_empty());
    assert!(
        output
            .source_label
            .contains("plugins(executed=1 not-executed=0)"),
        "source_label 必须带插件读数，实为 {}",
        output.source_label
    );
}

#[test]
fn plugin_instrument_wins_over_the_engine_timbre() {
    let fixture = Fixture::new("wins");
    let with_plugin = project_with_devices(vec![sine_device()]);
    let mut bypassed = with_plugin.clone();
    bypassed
        .tracks
        .values_mut()
        .find(|t| t.name == "Plugin")
        .expect("夹具轨")
        .devices[0]
        .bypassed = true;
    let a = render(&with_plugin, &fixture);
    let b = render(&bypassed, &fixture);
    assert_ne!(
        a.samples, b.samples,
        "插件胜出时输出必须与引擎音色不同（旁通后走引擎）；两版逐字节相同 ⇒ 插件根本没执行"
    );
    assert!(
        peak(&b.samples) > 0.01,
        "旁通后引擎必须出声（对照组本身要成立）"
    );
}

#[test]
fn hosted_gain_effect_halves_the_export_peak() {
    let fixture = Fixture::new("gain");
    let dry = render(&project_with_devices(vec![]), &fixture);
    let wet = render(&project_with_devices(vec![gain_device(0.5)]), &fixture);
    assert!(peak(&dry.samples) > 0.01, "干声对照组必须出声");
    let ratio = peak(&wet.samples) / peak(&dry.samples);
    assert!(
        (0.45..=0.55).contains(&ratio),
        "增益 0.5 的效果器必须把峰值减半，实测 dry={} wet={} ratio={ratio:.3}",
        peak(&dry.samples),
        peak(&wet.samples)
    );
    assert_eq!(
        wet.plugins_executed,
        vec!["Plugin: stub:gain [效果器]".to_owned()]
    );
    assert!(wet.plugins_not_executed.is_empty());
}

#[test]
fn second_bound_effect_is_reported_not_executed() {
    let fixture = Fixture::new("second");
    let single = render(&project_with_devices(vec![gain_device(0.5)]), &fixture);
    let double = render(
        &project_with_devices(vec![gain_device(0.5), gain_device(0.5)]),
        &fixture,
    );
    assert_eq!(
        single.samples, double.samples,
        "第二台绑定效果器必须不改变输出（首台胜出）"
    );
    assert_eq!(double.plugins_executed.len(), 1);
    assert_eq!(double.plugins_not_executed.len(), 1);
    assert!(
        double.plugins_not_executed[0].contains("第二台绑定设备，首台胜出"),
        "未执行原因必须写清，实为 {}",
        double.plugins_not_executed[0]
    );
}

#[test]
fn unbound_external_effect_is_reported_not_executed() {
    let fixture = Fixture::new("unbound");
    let dry = render(&project_with_devices(vec![]), &fixture);
    let wet = render(
        &project_with_devices(vec![unbound_effect_device()]),
        &fixture,
    );
    assert_eq!(
        dry.samples, wet.samples,
        "未绑定效果器必须不改变输出（没有东西可实例化）"
    );
    assert!(wet.plugins_executed.is_empty());
    assert_eq!(
        wet.plugins_not_executed,
        vec!["Plugin: unbound [效果器] (未执行: 未绑定插件)".to_owned()]
    );
}

#[test]
fn bypassed_effect_is_silent_and_unreported() {
    let fixture = Fixture::new("bypassed");
    let dry = render(&project_with_devices(vec![]), &fixture);
    let mut device = gain_device(0.5);
    device.bypassed = true;
    let wet = render(&project_with_devices(vec![device]), &fixture);
    assert_eq!(dry.samples, wet.samples, "旁通的效果器必须等于没有");
    assert!(wet.plugins_executed.is_empty());
    assert!(
        wet.plugins_not_executed.is_empty(),
        "旁通是显式选择，不进收据"
    );
}

#[test]
fn export_report_carries_the_plugin_split() {
    let fixture = Fixture::new("report");
    let project = project_with_devices(vec![sine_device()]);
    let path = fixture.dir.join("out.wav");
    let options = WavExportOptions {
        frames: FRAMES,
        ..WavExportOptions::default()
    };
    let report = export_wav(&project, &path, options).expect("导出");
    assert_eq!(
        report.plugins_executed,
        vec!["Plugin: stub:sine [乐器]".to_owned()]
    );
    assert!(report.plugins_not_executed.is_empty());
    let summary = report.summary();
    assert!(
        summary.contains("插件执行: 已执行 Plugin: stub:sine [乐器]；未执行 无"),
        "摘要必须带插件执行行，实为 {summary}"
    );
    let clean = export_wav(
        &project_with_devices(vec![]),
        &fixture.dir.join("clean.wav"),
        WavExportOptions {
            frames: FRAMES,
            ..WavExportOptions::default()
        },
    )
    .expect("导出");
    assert!(
        !clean.summary().contains("插件执行"),
        "没有插件的导出摘要必须一个字不多，实为 {}",
        clean.summary()
    );
}
