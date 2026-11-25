//! T-FIX-125：离线渲染与插件扫描的路径白名单缺口。
//!
//! 漏洞：`yeban_load_plugin` / `yeban_ara_bind` 的实参先过
//! `Host::validate_plugin_path`（形态＋`canonicalize`＋白名单根），但
//! 离线渲染直接把工程 AST 里的 `track.devices[].plugin.path` 送进
//! `Host::load` / `Host::load_effect`（`domain/render.rs` 两处），
//! `yeban_scan_plugins` 也把调用方 `paths` 直拼进 `Host::scan`——
//! 打开并渲染一份恶意 `.yeban` 工程即可绕过工具校验 dlopen 任意本地动态库。
//!
//! 本文件钉住三条：
//! 1. 渲染乐器／效果器绑定：根外真实插件在 dlopen 之前就被白名单拒绝
//!    （`pluginLoad` / `pluginEffectLoad` + "不在任何插件根内"）；
//! 2. `stub:` 桩绑定不受影响（仍能渲染）；
//! 3. 扫描：调用方给的根外目录响亮拒绝；不存在的目录仍按既有口径静默跳过。
//!
//! 红→绿：基线上 1、2 号判据的渲染会**成功**（夹具真插件被 dlopen 并
//! 出声），3 号判据的扫描会列出根外插件 —— 因此本文件在基线上是红的；
//! 修复后三条全部按白名单拒绝／放行。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::render::{self, RenderArtifact};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::{
    AssetHash, AssetMetadata, ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition,
    DeviceKind, EntityId, LoopConfig, MediaKind, PluginBinding, RoutingEdge, RoutingKind,
    SampleRate, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_plugin_host::Host;

/// 平台动态库扩展名（与宿主 `host.rs` 同款口径）。
#[cfg(target_os = "windows")]
const DYLIB_EXT: &str = "dll";
/// 平台动态库扩展名（与宿主 `host.rs` 同款口径）。
#[cfg(target_os = "macos")]
const DYLIB_EXT: &str = "dylib";
/// 平台动态库扩展名（与宿主 `host.rs` 同款口径）。
#[cfg(all(unix, not(target_os = "macos")))]
const DYLIB_EXT: &str = "so";

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
            "yeban-mcp-whitelist-{}-{tag}-{nanos}-{}",
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

/// 用 `rustc` 把宿主夹具（`yeban-plugin-host/tests/fixtures/fixture_plugin.rs`）
/// 现场编译成 cdylib，返回产物路径。产物落在调用方给的临时目录里 ——
/// 该目录不在任何插件根内（`YEBAN_VST3_PATHS` 未指向它），因此是"根外真插件"。
fn compile_fixture(dir: &Path) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../yeban-plugin-host/tests/fixtures/fixture_plugin.rs");
    assert!(source.is_file(), "夹具源码不在 {source:?}");
    let out = dir.join(format!("fixture_plugin.{DYLIB_EXT}"));
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let output = Command::new(&rustc)
        .args([
            "--crate-type",
            "cdylib",
            "--edition",
            "2021",
            "-o",
            out.to_str().expect("产物路径可转 UTF-8"),
            source.to_str().expect("夹具路径可转 UTF-8"),
        ])
        .output()
        .unwrap_or_else(|error| panic!("启动 {rustc} 失败: {error}"));
    assert!(
        output.status.success(),
        "夹具编译失败（{}）：\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.is_file(), "夹具产物不在 {out:?}");
    out
}

/// 断言：给定路径**不在**任何插件白名单根内（否则本文件的"根外"前提不成立，
/// 判据会假绿 —— 与其假绿不如响亮失败）。
fn assert_out_of_root(path: &Path) {
    let path_str = path.to_str().expect("路径可转 UTF-8");
    assert!(
        Host::validate_plugin_path(path_str).is_err(),
        "前提不成立：{path_str} 居然在白名单根内，判据无法区分有无白名单"
    );
}

// ---------------------------------------------------------------------------
// 渲染夹具：Master + 一条带音频片段的轨，轨上挂一台外部插件设备
// ---------------------------------------------------------------------------

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

struct Fixture {
    project: YebanProjectV1,
    asset: AssetHash,
    bytes: Vec<u8>,
}

/// 一份 "Master + 一条音轨 → Master" 的工程，音轨带一个 1 秒恒定电平音频
/// 片段；`plugin_device` 给出时挂在轨设备链上（乐器或效果器）。
fn fixture(bytes: Vec<u8>, plugin_device: Option<DeviceDefinition>) -> Fixture {
    let asset = AssetHash::of_bytes(&bytes);
    let master = id(1);
    let lead = id(2);
    let clip = id(10);
    let placement = id(20);
    let edge = id(40);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "plugin whitelist fixture".to_owned(),
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
    let mut devices = Vec::new();
    if let Some(device) = plugin_device {
        devices.push(device);
    }
    let mut track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Audio,
        volume_db: 0.0,
        pan: 0.0,
        devices,
        ..TrackV3::default()
    };
    track.clips.insert(
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
    project.tracks.insert(lead, track);
    project.routing_graph.nodes = vec![lead, master];
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: lead,
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
        asset,
        bytes,
    }
}

fn pool(fixture: &Fixture) -> BTreeMap<AssetHash, Vec<u8>> {
    let mut pool = BTreeMap::new();
    pool.insert(fixture.asset.clone(), fixture.bytes.clone());
    pool
}

/// 走**领域渲染本体**（`render::build`），不经过工具管线。
fn build_with(
    project: &YebanProjectV1,
    assets: &BTreeMap<AssetHash, Vec<u8>>,
    path: &Path,
    arguments: Value,
) -> Result<RenderArtifact, Fault> {
    let request =
        render::validate(arguments.as_object().expect("参数必须是对象")).expect("参数合法");
    let project_path = path.with_extension("project.yeban");
    render::build(project, &project_path, &request, NOW_MS, assets)
}

/// 默认实参：48 kHz / wav / 不做归一化。
fn args(wav_path: &Path) -> Value {
    json!({
        "format": "wav",
        "sampleRate": 48000,
        "path": wav_path.display().to_string(),
    })
}

fn constant(frames: usize, level: f32) -> Vec<f32> {
    vec![level; frames * 2]
}

fn plugin_device(kind: DeviceKind, path: &str, class: Option<&str>) -> DeviceDefinition {
    DeviceDefinition {
        id: id(102),
        name: "ext-plugin".to_owned(),
        kind,
        bypassed: false,
        params: Vec::new(),
        latency_samples: 0,
        plugin: Some(PluginBinding {
            path: path.to_owned(),
            class: class.map(str::to_owned),
            state: Vec::new(),
        }),
    }
}

// ---------------------------------------------------------------------------
// 扫描夹具：走 tools/call 的分发器
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

/// 渲染 [①]：轨上挂根外真插件作**乐器**时，渲染必须在 dlopen 之前就被白名单
/// 拒绝 —— 基线上它会**成功**（夹具真被 dlopen 并接管音源），因此本判据
/// 在基线是红的。
#[test]
fn render_rejects_out_of_root_instrument_plugin_before_dlopen() {
    let scratch = Scratch::new("render-instrument");
    let plugin = compile_fixture(&scratch.dir);
    assert_out_of_root(&plugin);
    let samples = constant(480, 0.5);
    let device = plugin_device(
        DeviceKind::ExternalInstrument,
        plugin.to_str().expect("路径可转 UTF-8"),
        Some("TestSine"),
    );
    let fixture = fixture(wav_f32(2, 48_000, &samples), Some(device));
    let out = scratch.join("master.wav");
    let error = build_with(&fixture.project, &pool(&fixture), &out, args(&out))
        .expect_err("根外插件绑定必须让渲染失败，不得 dlopen");
    let message = format!("{error:?}");
    assert!(
        message.contains("pluginLoad"),
        "失败原因应指向乐器装载（pluginLoad），实为 {message}"
    );
    assert!(
        message.contains("不在任何插件根内"),
        "失败原因应是白名单拒绝（不在任何插件根内），而不是加载本身失败，实为 {message}"
    );
}

/// 渲染 [②]：轨上挂根外真插件作**效果器**时，同样必须在 dlopen 之前被拒绝。
#[test]
fn render_rejects_out_of_root_effect_plugin_before_dlopen() {
    let scratch = Scratch::new("render-effect");
    let plugin = compile_fixture(&scratch.dir);
    assert_out_of_root(&plugin);
    let samples = constant(480, 0.5);
    let device = plugin_device(
        DeviceKind::ExternalEffect,
        plugin.to_str().expect("路径可转 UTF-8"),
        Some("TestGain"),
    );
    let fixture = fixture(wav_f32(2, 48_000, &samples), Some(device));
    let out = scratch.join("master.wav");
    let error = build_with(&fixture.project, &pool(&fixture), &out, args(&out))
        .expect_err("根外插件绑定必须让渲染失败，不得 dlopen");
    let message = format!("{error:?}");
    assert!(
        message.contains("pluginEffectLoad"),
        "失败原因应指向效果器装载（pluginEffectLoad），实为 {message}"
    );
    assert!(
        message.contains("不在任何插件根内"),
        "失败原因应是白名单拒绝（不在任何插件根内），而不是加载本身失败，实为 {message}"
    );
}

/// 渲染 [③]（正向对照）：`stub:` 桩绑定不受白名单影响，仍能渲染成功。
/// 白名单本身原样放行 `stub:`，本判据钉住"修复不得误伤桩"。
#[test]
fn render_still_accepts_stub_effect_binding() {
    let scratch = Scratch::new("render-stub");
    let samples = constant(480, 0.5);
    let device = plugin_device(DeviceKind::ExternalEffect, "stub:gain", None);
    let fixture = fixture(wav_f32(2, 48_000, &samples), Some(device));
    let out = scratch.join("master.wav");
    build_with(&fixture.project, &pool(&fixture), &out, args(&out))
        .expect("stub:gain 绑定仍应渲染成功");
}

/// 扫描 [④]：调用方给的根外目录必须响亮拒绝，⛔ 不走到 `Host::scan` 的 dlopen。
/// 基线上扫描会**成功**并列出根外夹具（`usable: true`），因此本判据在基线是红的。
#[test]
fn scan_rejects_out_of_root_caller_dir() {
    let scratch = Scratch::new("scan-out-of-root");
    let plugin = compile_fixture(&scratch.dir);
    assert_out_of_root(&plugin);
    let (mut dispatcher, auth) = dispatcher();
    let out = call(
        &mut dispatcher,
        &auth,
        "yeban_scan_plugins",
        json!({"paths": [scratch.dir.display().to_string()]}),
    );
    assert_eq!(out["status"], "error", "根外扫描目录必须响亮拒绝: {out}");
    assert_eq!(
        out["error"]["code"], "INVALID_PARAMETER_RANGE",
        "根外扫描目录应是 INVALID_PARAMETER_RANGE: {out}"
    );
    let message = out["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("不在任何插件根内"),
        "失败原因应是白名单拒绝（不在任何插件根内），实为 {message}"
    );
}

/// 扫描 [⑤]（既有口径对照）：不存在的调用方目录仍静默跳过（`Host::scan`
/// 的口径），⛔ 不得因此变成失败 —— 白名单只拦真实存在的根外目录。
#[test]
fn scan_still_skips_missing_caller_dir_silently() {
    let scratch = Scratch::new("scan-missing");
    let missing = scratch.join("does-not-exist");
    let (mut dispatcher, auth) = dispatcher();
    let out = call(
        &mut dispatcher,
        &auth,
        "yeban_scan_plugins",
        json!({"paths": [missing.display().to_string()]}),
    );
    assert_eq!(out["status"], "success", "不存在的目录仍应静默跳过: {out}");
    let plugins = out["data"]["plugins"].as_array().expect("plugins 数组");
    assert_eq!(plugins.len(), 0, "不存在的目录不得产出扫描行: {out}");
    assert!(
        plugins
            .iter()
            .all(|row| row["path"].as_str().unwrap_or("") != missing.display().to_string()),
        "不存在的目录不得产出扫描行: {out}"
    );
}
