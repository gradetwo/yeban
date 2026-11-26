//! T-FIX-420：Audio 轨挂乐器插件必须响亮拒绝。
//!
//! 缺陷：`yeban_load_plugin` 只校验 `trackId` 存在，不校验 `track.kind`，
//! 于是 `kind:"instrument"` 的外部乐器插件可以挂到 Audio 轨上，返回
//! `created:true`、`autoMerged:true`、`warnings:[]`；但渲染仍只走音频片段，
//! 插件实例从未被使用，输出与装载前逐字节相同 —— 成功回执掩盖了目标轨
//! 语义不匹配（ISS-MWVJR3F6-B08F0F）。
//!
//! 裁决（管理侧已定，参照 Logic Pro）：装载时校验 `track.kind`，Audio 轨挂
//! `instrument` 直接响亮拒绝（`INVALID_PARAMETER_RANGE`，报文点名目标轨
//! 的 kind 与要求）。只报 warning、仍返回成功的做法不采纳 —— 那正是
//! 本次缺陷的形态。
//!
//! 本文件钉住四条：
//! 1. Audio 轨挂 instrument → 拒绝，错误码 `INVALID_PARAMETER_RANGE`，
//!    报文含轨 kind（`Audio`）与"乐器只上 MIDI 轨"（kind 缺省即 instrument
//!    的一路同样拒绝）；
//! 2. 被拒绝的装载不留任何副作用 —— 工程逐字节不变，渲染输出逐字节不变；
//! 3. MIDI 轨挂 instrument 照常成功（回归守卫）；
//! 4. Audio 轨挂 effect 照常成功（回归守卫）。
//!
//! 红→绿：基线上 1、2 号判据的装载会**成功**（设备真的挂上 Audio 轨，
//! 工程随之改变），因此本文件在基线上是红的；修复后四条全部通过。
//! 注意 2 号判据的红对照细节：被错误挂上的插件渲染时从不被使用，
//! 因此单比渲染字节在基线上也会相同 —— 红来自工程被改动（设备已挂上），
//! 这正是"成功回执掩盖语义不匹配"的缺陷形态本身。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::render::{self, RenderArtifact};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::{
    AssetHash, AssetMetadata, ClipContent, ClipPlacement, ClipPoolEntry, DeviceKind, EntityId,
    LoopConfig, MediaKind, RoutingEdge, RoutingKind, SampleRate, TrackKind, TrackV3,
    YebanProjectV1,
};

const NOW_MS: u64 = 1_760_000_000_000;
/// 120 BPM / 960 PPQ 下 **1 秒** = 1920 tick。
const ONE_SECOND_TICKS: u64 = 1_920;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 临时目录（`Drop` 时删除）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-load-kind-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// 确定性 ULID（末 4 位是种子），跨运行逐字节稳定。
fn id(seed: u32) -> EntityId {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    EntityId::from_str(&text).expect("合法 Crockford Base32 ULID")
}

/// 组装一份最小 RIFF/WAVE（44 字节头 + `data`）。
fn wav_bytes(channels: u16, sample_rate: u32, tag: u16, bits: u16, payload: &[u8]) -> Vec<u8> {
    let block_align = channels * (bits / 8);
    let byte_rate = sample_rate * u32::from(block_align);
    let mut out = Vec::with_capacity(44 + payload.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(
        &u32::try_from(36 + payload.len())
            .expect("小文件")
            .to_le_bytes(),
    );
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(payload.len()).expect("小文件").to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// 32-bit float WAV（`WAVE_FORMAT_IEEE_FLOAT` = tag 3）。
fn wav_f32(channels: u16, sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        payload.extend_from_slice(&sample.to_le_bytes());
    }
    wav_bytes(channels, sample_rate, 3, 32, &payload)
}

fn constant(frames: usize, level: f32) -> Vec<f32> {
    vec![level; frames * 2]
}

struct Fixture {
    project: YebanProjectV1,
    audio_track: EntityId,
    midi_track: EntityId,
    asset: AssetHash,
    bytes: Vec<u8>,
}

/// "Master + 一条带音频片段的 Audio 轨 + 一条空 MIDI 轨 → Master" 的工程。
fn fixture(bytes: Vec<u8>) -> Fixture {
    let asset = AssetHash::of_bytes(&bytes);
    let master = id(1);
    let audio = id(2);
    let midi = id(3);
    let clip = id(10);
    let placement = id(20);
    let edge_audio = id(40);
    let edge_midi = id(41);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "load plugin track kind fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0001,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.audio_config.sample_rate = SampleRate::Hz48000;
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut audio_track = TrackV3 {
        id: audio,
        name: "Lead".to_owned(),
        kind: TrackKind::Audio,
        volume_db: 0.0,
        pan: 0.0,
        devices: Vec::new(),
        ..TrackV3::default()
    };
    audio_track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: ONE_SECOND_TICKS,
            muted: false,
            offset_samples: None,
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
            loop_config: LoopConfig::default(),
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Tone".to_owned(),
            content: ClipContent::Audio {
                asset: asset.clone(),
                gain_db: 0.0,
            },
        },
    );
    project.assets.insert(
        asset.clone(),
        AssetMetadata {
            hash: asset.clone(),
            original_path: "samples/tone.wav".to_owned(),
            byte_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            media_kind: MediaKind::Audio,
            license: "CC0-1.0".to_owned(),
        },
    );
    project.tracks.insert(audio, audio_track);
    project.tracks.insert(
        midi,
        TrackV3 {
            id: midi,
            name: "Keys".to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes = vec![audio, midi, master];
    project.routing_graph.edges.insert(
        edge_audio,
        RoutingEdge {
            id: edge_audio,
            source_node: audio,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
    project.routing_graph.edges.insert(
        edge_midi,
        RoutingEdge {
            id: edge_midi,
            source_node: midi,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
    project
        .validate()
        .unwrap_or_else(|error| panic!("夹具工程必须合法: {error}"));
    Fixture {
        project,
        audio_track: audio,
        midi_track: midi,
        asset,
        bytes,
    }
}

fn pool(fixture: &Fixture) -> BTreeMap<AssetHash, Vec<u8>> {
    let mut pool = BTreeMap::new();
    pool.insert(fixture.asset.clone(), fixture.bytes.clone());
    pool
}

/// 测试用分发器（全量 scope、生产模式、固定时钟）。
fn dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(NOW_MS);
    (dispatcher, auth)
}

/// 走 `tools/call`，返回 `(http 状态, JSON-RPC 层结果)`。
fn call_raw(
    dispatcher: &mut Dispatcher,
    auth: &str,
    name: &str,
    arguments: Value,
) -> (u16, Result<Value, yeban_mcp::jsonrpc::ErrorObject>) {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    match (response.result, response.error) {
        (Some(result), None) => (outcome.http_status, Ok(result)),
        (None, Some(error)) => (outcome.http_status, Err(error)),
        other => panic!("result 与 error 必须恰好一个: {other:?}"),
    }
}

/// 走 `tools/call` 并断言这是**带内** `ToolResponse`（成功或领域失败）。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let (status, outcome) = call_raw(dispatcher, auth, name, arguments);
    assert_eq!(
        status, 200,
        "{name} 必须是带内结果（领域失败也走 ToolResponse）"
    );
    outcome.unwrap_or_else(|error| panic!("{name} 不该有 JSON-RPC 层错误: {error:?}"))
}

/// 把夹具工程写成真容器并经 `yeban_open_project` 打开为活跃工程。
fn open_fixture(scratch: &Scratch, dispatcher: &mut Dispatcher, auth: &str, fixture: &Fixture) {
    let history = serde_json::to_vec(&yeban_model::CommitGraph::new()).expect("空图谱 JSON");
    let bytes =
        yeban_model::container::write_project_container(&fixture.project, &history, &pool(fixture))
            .expect("写真容器");
    let path = scratch.join("fixture.yeban");
    fs::write(&path, &bytes).expect("写容器工程");
    let opened = call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(
        opened["status"], "success",
        "打开夹具工程必须成功: {opened}"
    );
}

/// 走**领域渲染本体**（`render::build`），不经过工具管线。
fn build_with(
    project: &YebanProjectV1,
    assets: &BTreeMap<AssetHash, Vec<u8>>,
    path: &Path,
) -> Result<RenderArtifact, Fault> {
    let request = render::validate(
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": path.display().to_string(),
        })
        .as_object()
        .expect("参数必须是对象"),
    )
    .expect("参数合法");
    let project_path = path.with_extension("project.yeban");
    render::build(project, &project_path, &request, NOW_MS, assets)
}

/// 活跃工程的规范化文本（用于"逐字节相同"的断言）。
fn project_bytes(dispatcher: &Dispatcher) -> String {
    let project = dispatcher.domain().active_project().expect("活跃工程");
    let mut text = serde_json::to_string_pretty(project).expect("序列化");
    text.push('\n');
    text
}

/// 某条轨当前挂的设备数。
fn device_count(dispatcher: &Dispatcher, track_id: EntityId) -> usize {
    dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .tracks
        .get(&track_id)
        .expect("目标轨必须在场")
        .devices
        .len()
}

/// 某条轨当前挂的设备 kind（调用方保证恰有一台）。
fn device_kind(dispatcher: &Dispatcher, track_id: EntityId) -> DeviceKind {
    let devices = &dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .tracks
        .get(&track_id)
        .expect("目标轨必须在场")
        .devices;
    assert_eq!(devices.len(), 1, "目标轨应恰有一台设备");
    devices[0].kind
}

fn load_plugin(
    dispatcher: &mut Dispatcher,
    auth: &str,
    track_id: EntityId,
    plugin_path: &str,
    kind: Option<&str>,
    auto_merge: bool,
) -> Value {
    let mut args = json!({
        "trackId": track_id.to_canonical_string(),
        "pluginPath": plugin_path,
    });
    if let Some(kind) = kind {
        args["kind"] = json!(kind);
    }
    // 提案语义：缺省只建提案、不改工程（`pendingMerge: true`）；`autoMerge: true`
    // 才当场合入、设备落地。简报里的成功回执（`created:true`、`autoMerged:true`）
    // 就是 autoMerge 一路的形状，回归守卫必须走这一路才钉得住"设备真的挂上"。
    if auto_merge {
        args["autoMerge"] = json!(true);
    }
    call(dispatcher, auth, "yeban_load_plugin", args)
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

/// [①] Audio 轨挂 instrument → 响亮拒绝：错误码 `INVALID_PARAMETER_RANGE`，
/// 报文点名目标轨的 kind（`Audio`）与"乐器只上 MIDI 轨"。基线上这条装载
/// 会**成功**（设备真的挂上 Audio 轨），因此本判据在基线是红的。
/// kind 缺省时默认即 instrument，同样必须拒绝。
#[test]
fn audio_track_rejects_instrument_with_track_kind_in_message() {
    let scratch = Scratch::new("reject-instrument");
    let samples = constant(480, 0.5);
    let fixture = fixture(wav_f32(2, 48_000, &samples));
    let (mut dispatcher, auth) = dispatcher();
    open_fixture(&scratch, &mut dispatcher, &auth, &fixture);
    let before = project_bytes(&dispatcher);

    let out = load_plugin(
        &mut dispatcher,
        &auth,
        fixture.audio_track,
        "stub:sine",
        Some("instrument"),
        false,
    );
    assert_eq!(out["status"], "error", "Audio 轨挂乐器必须响亮拒绝: {out}");
    assert_eq!(
        out["error"]["code"], "INVALID_PARAMETER_RANGE",
        "Audio 轨挂乐器应是 INVALID_PARAMETER_RANGE: {out}"
    );
    let message = out["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("Audio"),
        "拒绝报文必须点名目标轨的 kind（Audio），实为 {message}"
    );
    assert!(
        message.contains("只上 MIDI 轨"),
        "拒绝报文必须写清要求（乐器只上 MIDI 轨），实为 {message}"
    );
    // 拒绝不得留副作用：工程逐字节不变，Audio 轨仍无设备。
    assert_eq!(
        project_bytes(&dispatcher),
        before,
        "被拒绝的装载不得改动工程"
    );
    assert_eq!(
        device_count(&dispatcher, fixture.audio_track),
        0,
        "Audio 轨上不得多出设备"
    );

    // kind 缺省即 instrument：同样拒绝。
    let out = load_plugin(
        &mut dispatcher,
        &auth,
        fixture.audio_track,
        "stub:sine",
        None,
        false,
    );
    assert_eq!(
        out["status"], "error",
        "缺省 kind（即 instrument）同样必须拒绝: {out}"
    );
    assert_eq!(
        out["error"]["code"], "INVALID_PARAMETER_RANGE",
        "缺省 kind 拒绝应是 INVALID_PARAMETER_RANGE: {out}"
    );
    assert_eq!(
        project_bytes(&dispatcher),
        before,
        "被拒绝的装载不得改动工程"
    );
}

/// [②] 被拒绝的装载不留任何副作用：拒绝前后的渲染输出逐字节相同。
/// 基线上装载会成功并改动工程（设备挂上 Audio 轨），因此本判据在基线
/// 是红的 —— 红来自工程被改动；单比渲染字节在基线上也会相同（被错误
/// 挂上的插件渲染时从不被使用），那正是本单报告的缺陷形态。
#[test]
fn rejected_load_leaves_render_output_byte_identical() {
    let scratch = Scratch::new("render-unchanged");
    let samples = constant(480, 0.5);
    let fixture = fixture(wav_f32(2, 48_000, &samples));
    let (mut dispatcher, auth) = dispatcher();
    open_fixture(&scratch, &mut dispatcher, &auth, &fixture);
    let assets = pool(&fixture);

    let project = dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .clone();
    let before =
        build_with(&project, &assets, &scratch.join("before.wav")).expect("拒绝前渲染必须成功");
    let project_before = project_bytes(&dispatcher);

    let out = load_plugin(
        &mut dispatcher,
        &auth,
        fixture.audio_track,
        "stub:sine",
        Some("instrument"),
        false,
    );
    assert_eq!(out["status"], "error", "Audio 轨挂乐器必须响亮拒绝: {out}");

    let project = dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .clone();
    let after =
        build_with(&project, &assets, &scratch.join("after.wav")).expect("拒绝后渲染必须成功");
    assert_eq!(
        after.bytes, before.bytes,
        "拒绝前后的渲染输出必须逐字节相同"
    );
    assert_eq!(
        after.sha256, before.sha256,
        "拒绝前后的渲染 SHA-256 必须相同"
    );
    assert_eq!(
        project_bytes(&dispatcher),
        project_before,
        "被拒绝的装载不得改动工程"
    );
    assert_eq!(
        device_count(&dispatcher, fixture.audio_track),
        0,
        "Audio 轨上不得多出设备"
    );
}

/// [③]（回归守卫）MIDI 轨挂 instrument 照常成功，设备真的落在 MIDI 轨上。
#[test]
fn midi_track_accepts_instrument() {
    let scratch = Scratch::new("midi-instrument");
    let samples = constant(480, 0.5);
    let fixture = fixture(wav_f32(2, 48_000, &samples));
    let (mut dispatcher, auth) = dispatcher();
    open_fixture(&scratch, &mut dispatcher, &auth, &fixture);

    let out = load_plugin(
        &mut dispatcher,
        &auth,
        fixture.midi_track,
        "stub:sine",
        Some("instrument"),
        true,
    );
    assert_eq!(out["status"], "success", "MIDI 轨挂乐器必须照常成功: {out}");
    assert_eq!(
        out["data"]["autoMerged"], true,
        "autoMerge 一路的回执须含 autoMerged:true: {out}"
    );
    assert_eq!(
        device_count(&dispatcher, fixture.midi_track),
        1,
        "乐器设备必须真的落在 MIDI 轨上"
    );
    assert_eq!(
        device_kind(&dispatcher, fixture.midi_track),
        DeviceKind::ExternalInstrument,
        "MIDI 轨上挂的必须是外部乐器"
    );
    // Audio 轨未被波及。
    assert_eq!(
        device_count(&dispatcher, fixture.audio_track),
        0,
        "Audio 轨不得被波及"
    );
}

/// [④]（回归守卫）Audio 轨挂 effect 照常成功，设备真的落在 Audio 轨上。
#[test]
fn audio_track_accepts_effect() {
    let scratch = Scratch::new("audio-effect");
    let samples = constant(480, 0.5);
    let fixture = fixture(wav_f32(2, 48_000, &samples));
    let (mut dispatcher, auth) = dispatcher();
    open_fixture(&scratch, &mut dispatcher, &auth, &fixture);

    let out = load_plugin(
        &mut dispatcher,
        &auth,
        fixture.audio_track,
        "stub:gain",
        Some("effect"),
        true,
    );
    assert_eq!(
        out["status"], "success",
        "Audio 轨挂效果器必须照常成功: {out}"
    );
    assert_eq!(
        out["data"]["autoMerged"], true,
        "autoMerge 一路的回执须含 autoMerged:true: {out}"
    );
    assert_eq!(
        device_count(&dispatcher, fixture.audio_track),
        1,
        "效果器设备必须真的落在 Audio 轨上"
    );
    assert_eq!(
        device_kind(&dispatcher, fixture.audio_track),
        DeviceKind::ExternalEffect,
        "Audio 轨上挂的必须是外部效果器"
    );
}
