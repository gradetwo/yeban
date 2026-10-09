//! **`yeban_render_master` 的端到端判据**：它真的渲染、真的落盘、真的诚实
//! [MCP-TOOL-008, ROAD-M4-005]。
//!
//! 这个文件是"十个工具全部真做事"的最后一块判决书：上一线唯一未接线的一半就是这里。
//! 每一条判据都按"**能变红**"的方式写 —— 用一个可以被注入破坏的量（文件字节、
//! inode、错误码、峰值、目录条目）而不是"函数返回了 `Ok`"。
//!
//! ## 判据地图
//!
//! | # | 判据 | 可被什么注入破坏 |
//! | :--- | :--- | :--- |
//! | 1 | 1 秒 48k 立体声 ⇒ 文件字节 == 独立算出的头 + 帧×声道×位深/8 | 写错长度/静音文件 |
//! | 2 | 同输入两次渲染 ⇒ 逐字节相同 | 引入熵源/时间戳 |
//! | 3 | `normalize=true` ⇒ 峰值 == 满量程（±1e-6）；全零信号是恒等 | 归一化空操作 |
//! | 4 | `dryRun` 不产生任何文件、工程字节不变、预览给路径与帧数 | dryRun 真写盘 |
//! | 5 | 同 `idempotencyKey` 不重复渲染（inode 不变 + 删文件后不重建） | 关掉幂等缓存 |
//! | 6 | 输出目录不可写 ⇒ `IO_ERROR`、原文件逐字节不变、无临时残留 | 原地写/不清理 |
//! | 7 | 参数非法与不可实现的组合 ⇒ 契约内错误码（`INVALID_PARAMETER_RANGE` / `RENDER_FAILED`） | 发明新码/假装成功 |
//! | 8 | 输出路径不得是工程文件或锁文件 | 去掉护栏 |
//! | 9 | 母带不是静音且声相真的生效（左右 RMS 不对称） | 静音产物/忽略 pan |
//! | 9b | 经 `yeban_edit_notes` 写的静音真的让源轨不发声（`audible=false`、浮点母带为 0），写回 `false` ⇒ 产物逐字节相同 | 工具面写不了开关 / 渲染器忽略 `mute` |
//! | 10 | 延迟表来自 `DeviceDefinition::latency_samples`（含旁通不算） | 自建第二延迟来源 |
//! | 11 | `format` 只改容器字节，不改音频负载 | 在音频路径上按格式分叉 |
//! | 12 | 实测数字自洽：frames/blocks/bytes/header/payload/sha256 | 报估算值 |
//! | 12b | 循环区间短于摆放跨度 ⇒ 登记 `clipLoopRepetition`，且母带与"关循环"逐位相同 | 只在区间越过摆放末端时才报（静默丢掉重复） |
//!
//! ## 临时目录纪律
//!
//! 全部落在 `std::env::temp_dir()/<唯一子目录>`，[`Scratch`] 的 `Drop` 负责恢复
//! 权限并删除 —— **绝不污染仓库**。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::ErrorCode;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, RoutingEdge, RoutingKind, SampleRate, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::rf64::{BEXT_FIXED_LEN, parse_container};

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 一个临时目录，`Drop` 时恢复权限并删除。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-render-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn text(&self, name: &str) -> String {
        self.join(name).display().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o755));
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// 目录里的全部条目名（判"没有临时文件残留"用）。
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 确定性 ULID（末 4 位是种子），因此同一判据里的工程身份逐字节稳定。
fn id(seed: u32) -> EntityId {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    EntityId::from_str(&text).expect("合法 Crockford Base32 ULID")
}

/// 工程夹具的可调旋钮。
struct Spec {
    /// 120 BPM / 960 PPQ 下 1 秒 = 1920 tick。
    end_tick: u64,
    velocity: u8,
    pan: f32,
    volume_db: f32,
    bpm: f64,
    devices: Vec<DeviceDefinition>,
    loop_config: LoopConfig,
    with_placement: bool,
    /// 音符的**概率触发**取值（`None` = 必然触发，逐字节等于加这个旋钮之前）。
    probability: Option<f32>,
    /// 工程随机种子（`YebanProjectV1::rng_seed`）—— 概率判定的**唯一**熵输入。
    rng_seed: u64,
    /// 音符身份种子（改它 = 换一个音符身份 ⇒ 触发判定可能翻转）。
    note_seed: u32,
    /// 音符的**连击**细分次数（`None` = 不连击）。
    ratchet: Option<u8>,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            end_tick: 1_920,
            velocity: 100,
            pan: 0.0,
            volume_db: 0.0,
            bpm: 120.0,
            devices: Vec::new(),
            loop_config: LoopConfig::default(),
            with_placement: true,
            probability: None,
            rng_seed: DEFAULT_RNG_SEED,
            note_seed: 30,
            ratchet: None,
        }
    }
}

/// 夹具的默认工程种子（沿用改这个旋钮之前的硬编码值）。
const DEFAULT_RNG_SEED: u64 = 0x5945_4241_4E00_0001;

/// 默认夹具（`Spec::default()`：无概率、无连击）的**母带样本**位级摘要 —— 钉死的常量。
///
/// 它是"默认路径逐位不变"的锚：任何改动（连击展开是一例）只要碰了默认路径，
/// 这里就会红。判据见 `two_renders_of_the_same_project_are_byte_identical`。
const DEFAULT_MASTER_DIGEST: &str =
    "b242d510d581541732134d3c0e233a11d6045ffb65535675adc73501fb28eefb";

/// 同一份默认夹具的 RIFF **文件字节** SHA-256 —— 钉死的常量。
const DEFAULT_MASTER_SHA256: &str =
    "b9472fcd20086efd4953d457dde26169d1374f592bc90a0cf7b81304e0b69cc4";

/// 设备夹具。
fn device(seed: u32, latency_samples: u32, bypassed: bool) -> DeviceDefinition {
    DeviceDefinition {
        id: id(seed),
        name: format!("dev{seed}"),
        kind: DeviceKind::InternalInstrument,
        bypassed,
        params: Vec::new(),
        latency_samples,
    }
}

/// "Master + 一条 MIDI 轨 → Master"的工程（判据全部建立在它上面）。
fn project(spec: &Spec) -> YebanProjectV1 {
    let master = id(1);
    let lead = id(2);
    let clip = id(10);
    let placement = id(20);
    let note = id(spec.note_seed);
    let edge = id(40);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "render fixture".to_owned(),
        bpm: spec.bpm,
        rng_seed: spec.rng_seed,
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
    let mut track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        pan: spec.pan,
        volume_db: spec.volume_db,
        devices: spec.devices.clone(),
        ..TrackV3::default()
    };
    if spec.with_placement {
        track.clips.insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: clip,
                start_tick: 0,
                duration_ticks: spec.end_tick,
                loop_config: spec.loop_config,
                muted: false,
            },
        );
        let mut notes = BTreeMap::new();
        notes.insert(
            note,
            MidiNote {
                velocity: spec.velocity,
                probability: spec.probability,
                ratchet: spec.ratchet,
                ..MidiNote::new(note, 0, 69, spec.end_tick)
            },
        );
        project.clip_pool.insert(
            clip,
            ClipPoolEntry {
                id: clip,
                name: "Clip".to_owned(),
                content: ClipContent::Midi { notes },
            },
        );
    }
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
        },
    );
    project
        .validate()
        .unwrap_or_else(|error| panic!("夹具工程必须合法: {error}"));
    project
}

/// 注入夹具工程的分发器（`open_in_memory` 不碰文件系统）。
fn dispatcher_with(project: &YebanProjectV1, path: &Path) -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(path.to_path_buf(), project.clone(), false)
        .expect("注入夹具工程");
    (dispatcher, auth)
}

/// 走真实 `tools/call` 管线；工具路径上**不允许**任何 JSON-RPC 层错误。
fn call(dispatcher: &mut Dispatcher, auth: &str, arguments: Value) -> Value {
    call_tool(dispatcher, auth, "yeban_render_master", arguments)
}

/// 与 [`call`] 同一条管线，但**点名**工具（供"先经 `yeban_edit_notes` 写、再渲染"这类
/// 跨工具判据使用；本文件绝大多数判据只调 `yeban_render_master`，因此 [`call`] 不变）。
fn call_tool(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
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
        (Some(result), None) => result,
        (None, Some(error)) => panic!("工具路径上不该有 JSON-RPC 错误: {error:?}"),
        other => panic!("result 与 error 必须恰好一个: {other:?}"),
    }
}

/// 断言带内失败并检查错误码落在契约 enum 内。
fn assert_domain_error(value: &Value, expected: &str, context: &str) {
    assert_eq!(value["status"], "error", "{context}: {value}");
    assert_eq!(value["error"]["code"], expected, "{context}: {value}");
    assert!(
        ErrorCode::SCHEMA_CONTRACT
            .iter()
            .any(|code| code.as_str() == expected),
        "{context}: {expected} 不在契约联集里"
    );
}

/// 24-bit 小端负载 → `[-1, 1)` 浮点样本（独立解码器，不用生产代码）。
fn decode_i24(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<3>()
        .0
        .iter()
        .map(|chunk| {
            let raw =
                i32::from(chunk[0]) | (i32::from(chunk[1]) << 8) | (i32::from(chunk[2]) << 16);
            let signed = (raw << 8) >> 8;
            signed as f32 / 8_388_608.0
        })
        .collect()
}

/// 峰值（独立实现）。
fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0f32, |acc, sample| acc.max(sample.abs()))
}

/// 每声道 RMS（独立实现）。
fn channel_rms(samples: &[f32], channels: usize, channel: usize) -> f32 {
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for (index, sample) in samples.iter().enumerate() {
        if index % channels == channel {
            sum += f64::from(*sample) * f64::from(*sample);
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    (sum / count as f64).sqrt() as f32
}

/// 判据用的**独立**时间换算：1 tick 等于多少母带帧。
///
/// 夹具固定 120 BPM / 960 PPQ / 48 kHz ⇒ `48000 * 60 / (120 * 960) = 25`。
/// 这里显式算出来（不是抄一个 25），这样换了夹具参数这条换算会跟着变。
fn frames_per_tick() -> usize {
    const SAMPLE_RATE: usize = 48_000;
    const BPM: usize = 120;
    const PPQ: usize = 960;
    SAMPLE_RATE * 60 / (BPM * PPQ)
}

/// 左声道 `[start, start + frames)` 的 RMS（帧号是**母带帧**，不是 tick）。
fn window_rms(samples: &[f32], channels: usize, start: usize, frames: usize) -> f64 {
    let available = samples.len() / channels;
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for index in start..(start + frames).min(available) {
        let value = f64::from(samples[index * channels]);
        sum += value * value;
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    (sum / count as f64).sqrt()
}

/// 左声道 `[start, start + frames)` 的峰值（帧号是**母带帧**）。
fn window_peak(samples: &[f32], channels: usize, start: usize, frames: usize) -> f32 {
    let available = samples.len() / channels;
    let mut peak = 0.0f32;
    for index in start..(start + frames).min(available) {
        peak = peak.max(samples[index * channels].abs());
    }
    peak
}

/// **独立**的头部长度预言（规范常量手算，不用生产代码的 `header_bytes()`）。
///
/// `RIFF`/`RF64`/`BW64` 前缀 12 字节（fourcc + 长度 + `WAVE`）；
/// RF64/BW64 多一个 `ds64`（8 + 28）；双声道 24-bit 用 16 字节 `fmt `（8 + 16）；
/// `bext` = 8 + 602 + coding history；`data` chunk 头 8 字节。
fn expected_header_bytes(container: &str, coding_history_len: usize) -> usize {
    let prefix = 12;
    let ds64 = if container == "RIFF" { 0 } else { 8 + 28 };
    let fmt = 8 + 16;
    let bext_payload = BEXT_FIXED_LEN + coding_history_len;
    let bext = 8 + bext_payload + (bext_payload % 2);
    let data_header = 8;
    prefix + ds64 + fmt + bext + data_header
}

/// 读回产物并核对"头 + 帧×声道×位深/8"的自洽性，返回解码后的样本。
fn verify_file_shape(result: &Value, path: &Path) -> Vec<f32> {
    let data = &result["data"];
    let bytes = fs::read(path).expect("产物必须存在");
    assert_eq!(
        bytes.len(),
        usize::try_from(data["bytes"].as_u64().expect("bytes")).expect("合理尺寸"),
        "响应里的 bytes 必须等于磁盘字节数"
    );
    let parsed = parse_container(&bytes).expect("产物必须是合法 WAVE 容器");
    let payload_len = parsed.data.len();
    let frames = usize::try_from(data["frames"].as_u64().expect("frames")).expect("合理帧数");
    let channels = usize::try_from(data["channels"].as_u64().expect("channels")).expect("声道");
    let bits = usize::try_from(data["bitDepth"].as_u64().expect("bitDepth")).expect("位深");
    assert_eq!(
        payload_len,
        frames * channels * bits / 8,
        "负载长度必须自洽"
    );
    assert_eq!(
        payload_len,
        usize::try_from(data["payloadBytes"].as_u64().expect("payloadBytes")).expect("尺寸")
    );
    assert_eq!(parsed.sizes.sample_count, frames as u64, "ds64/RF64 帧数");
    assert_eq!(usize::from(parsed.format.channels), channels);
    assert_eq!(
        parsed.format.sample_rate,
        data["sampleRate"].as_u64().unwrap_or(0) as u32
    );
    assert_eq!(usize::from(parsed.format.bits_per_sample), bits);
    assert!(!parsed.format.is_float, "母带是整数 PCM");
    // 文件总长度 == 独立算出的头 + 负载(+偶数补位)。
    let container = data["container"].as_str().expect("container");
    let coding_history = data["bwf"]["codingHistory"]
        .as_str()
        .expect("codingHistory");
    let header = expected_header_bytes(container, coding_history.len());
    assert_eq!(
        header,
        usize::try_from(data["headerBytes"].as_u64().expect("headerBytes")).expect("尺寸"),
        "响应的 headerBytes 必须等于独立手算的头部长度"
    );
    assert_eq!(
        bytes.len(),
        header + payload_len + (payload_len % 2),
        "文件字节数必须 == 头 + 帧×声道×位深/8 (+补位)"
    );
    // bext 必须携带工程 ULID（ARCH-FMT-001 的映射）。
    let bext = parsed.bext.expect("必须写 bext");
    assert_eq!(bext.originator_reference, id(999).to_canonical_string());
    assert_eq!(bext.originator, "Yeban DAW");
    assert_eq!(bext.coding_history, coding_history);
    decode_i24(&bytes[parsed.data.clone()])
}

// ---------------------------------------------------------------------------
// 判据 1 / 12：尺寸自洽 + 实测数字
// ---------------------------------------------------------------------------

#[test]
fn one_second_of_stereo_master_is_header_plus_frames_times_channels_times_depth() {
    let scratch = Scratch::new("size");
    let spec = Spec::default();
    let (mut dispatcher, auth) = dispatcher_with(&project(&spec), &scratch.join("demo.yeban"));
    let out = scratch.join("master.wav");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let data = &result["data"];
    // 120 BPM / 960 PPQ / 1920 tick == 恰好 1 秒 @ 48 kHz。
    assert_eq!(data["frames"], 48_000);
    assert_eq!(data["channels"], 2);
    assert_eq!(data["bitDepth"], 24);
    assert_eq!(data["container"], "RIFF");
    assert_eq!(data["blocks"], 375, "48000 / 128");
    assert_eq!(data["durationSeconds"], 1.0);
    assert_eq!(
        data["latencySource"],
        "DeviceDefinition::latency_samples (track_latencies)"
    );
    assert_eq!(data["atomic"], true);
    let samples = verify_file_shape(&result, &out);
    assert_eq!(samples.len(), 48_000 * 2);
    assert_eq!(
        data["sha256"].as_str().expect("sha256").len(),
        64,
        "整份文件的 SHA-256 (十六进制)"
    );
    assert_eq!(
        data["masterDigest"].as_str().expect("masterDigest").len(),
        64,
        "母带样本的位级 SHA-256"
    );
    assert_eq!(entries(&scratch.dir), vec!["master.wav"], "不许有临时残留");
}

// ---------------------------------------------------------------------------
// 判据 2：确定性
// ---------------------------------------------------------------------------

#[test]
fn two_renders_of_the_same_project_are_byte_identical() {
    let scratch = Scratch::new("determinism");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let first_path = scratch.join("a.wav");
    let second_path = scratch.join("b.wav");
    let first = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": first_path.display().to_string()}),
    );
    let second = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": second_path.display().to_string()}),
    );
    assert_eq!(first["status"], "success", "{first}");
    assert_eq!(second["status"], "success", "{second}");
    let a = fs::read(&first_path).expect("产物 a");
    let b = fs::read(&second_path).expect("产物 b");
    assert_eq!(a, b, "同一工程两次渲染必须逐字节相同");
    assert_eq!(first["data"]["sha256"], second["data"]["sha256"]);
    assert_eq!(
        first["data"]["masterDigest"],
        second["data"]["masterDigest"]
    );
    assert_eq!(first["data"]["frames"], second["data"]["frames"]);

    // **交叉机位对齐（钉死的常量）**: 夹具完全确定（固定 ULID、固定 `rng_seed`、
    // 固定注入时钟、无熵源），因此母带样本摘要与文件摘要都必须是**固定常量**。
    // 这条判据同时是 L1 位级一致 [ARCH-DET-002] 的可比形式:
    // 若它变红, 要么夹具漂了, 要么合成/抖动路径引入了平台相关行为 —— 两种都必须查。
    assert_eq!(
        first["data"]["masterDigest"], DEFAULT_MASTER_DIGEST,
        "母带样本的位级摘要必须逐位固定"
    );
    assert_eq!(
        first["data"]["sha256"], DEFAULT_MASTER_SHA256,
        "RIFF 文件字节的 SHA-256 必须逐位固定"
    );
}

// ---------------------------------------------------------------------------
// 判据 3：归一化（含全零信号边界）
// ---------------------------------------------------------------------------

#[test]
fn normalize_hits_full_scale_and_leaves_all_zero_signal_alone() {
    let scratch = Scratch::new("normalize");
    // 低音量工程: 不归一化时峰值远小于满量程。
    let quiet = Spec {
        velocity: 8,
        volume_db: -18.0,
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&quiet), &scratch.join("demo.yeban"));
    let plain_path = scratch.join("plain.wav");
    let plain = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": plain_path.display().to_string()}),
    );
    let before = plain["data"]["peak"]["before"]
        .as_f64()
        .expect("peak.before");
    assert!(
        before < 0.5 && before > 0.0,
        "夹具必须真的低音量, 实测 {before}"
    );
    assert_eq!(plain["data"]["normalize"]["requested"], false);
    assert_eq!(plain["data"]["normalize"]["applied"], false);

    let normalized_path = scratch.join("normalized.wav");
    let normalized = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "normalize": true,
            "path": normalized_path.display().to_string(),
        }),
    );
    assert_eq!(normalized["status"], "success", "{normalized}");
    assert_eq!(normalized["data"]["normalize"]["applied"], true);
    let after = normalized["data"]["peak"]["after"]
        .as_f64()
        .expect("peak.after");
    // 目标满量程; f32 除法 + 乘法各一次舍入 ⇒ 误差上界 ~2^-24, 取 1e-6 容差。
    assert!(
        (after - 1.0).abs() < 1.0e-6,
        "归一化后峰值 {after} 必须命中满量程"
    );
    // 文件里的样本也真的被放大了（不是只改响应数字）。
    let samples = verify_file_shape(&normalized, &normalized_path);
    assert!(peak(&samples) > 0.99, "归一化后的产物必须接近满量程");
    assert_ne!(
        fs::read(&plain_path).expect("plain"),
        fs::read(&normalized_path).expect("normalized"),
        "归一化必须改变音频字节"
    );

    // 全零信号（力度 0）: 归一化是**恒等**, 且如实说明, 而不是"成功"。
    let silent = Spec {
        velocity: 0,
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&silent), &scratch.join("silent.yeban"));
    let silent_path = scratch.join("silent.wav");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "normalize": true,
            "path": silent_path.display().to_string(),
        }),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["data"]["normalize"]["requested"], true);
    assert_eq!(
        result["data"]["normalize"]["applied"], false,
        "全零信号不归一化"
    );
    assert!(
        result["data"]["normalize"]["note"].is_string(),
        "必须给出边界说明: {result}"
    );
    assert_eq!(result["data"]["peak"]["before"], 0.0);
    assert_eq!(result["data"]["peak"]["after"], 0.0);
    let samples = verify_file_shape(&result, &silent_path);
    // 抖动是"降位深"这一步的伴生 [ARCH-FMT-001]: 全零的**浮点**母带经 TPDF 抖动后
    // 会出现 ±1 LSB 的量化噪声。这是正确的 dither 行为而不是"信号" —— 因此这里断言
    // "≤ 2 LSB", 而不是"逐样本为 0"（后者会把正确的抖动判成缺陷）。
    let lsb = 1.0f32 / 8_388_608.0;
    assert!(
        samples
            .iter()
            .all(|sample| sample.is_finite() && sample.abs() <= 2.0 * lsb),
        "全零信号只允许出现 ≤ 1 LSB 的抖动噪声"
    );
    assert!(peak(&samples) <= 2.0 * lsb);
}

// ---------------------------------------------------------------------------
// 判据 3b：响度目标（`targetLufs`）—— 读数必须真、目标必须有牙、默认路径必须不变
// ---------------------------------------------------------------------------

/// 判据 3b-i：**不给** `targetLufs` 时也报**实测**读数，且判定是 `noTarget`。
///
/// 这是"新字段的默认路径"判据：默认响应必须已经带着响度块（否则默认路径的读数
/// 就没人测了），但**不许**有判定（`deltaLu == null`）。
#[test]
fn loudness_reading_is_reported_on_the_default_path_without_a_target() {
    let scratch = Scratch::new("loudness-default");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "dryRun": true}),
    );
    assert_eq!(result["status"], "success", "{result}");
    let loudness = &result["data"]["preview"]["loudness"];
    assert_eq!(loudness["verdict"], "noTarget", "{result}");
    assert_eq!(loudness["targetLufs"], Value::Null);
    assert_eq!(loudness["deltaLu"], Value::Null);
    assert_eq!(loudness["measured"], true, "夹具母带不是静音: {result}");
    let measured = loudness["measuredIntegratedLufs"]
        .as_f64()
        .unwrap_or_else(|| panic!("必须有实测读数: {result}"));
    // 门限积分的读数必须落在 LS.1770 的绝对门限之上、满量程之下。
    assert!(
        measured.is_finite() && measured > -70.0 && measured <= 0.0,
        "实测读数是合法 LUFS 才会在这里: {measured}"
    );
    assert_eq!(loudness["toleranceLu"], 0.5);
    // 洞：默认路径的读数**不是** 0，也不是某个常数 —— 它必须来自母带信号。
    assert!(
        (measured - 0.0).abs() > 1.0,
        "读数不许是个用来占位的 0: {measured}"
    );
}

/// 判据 3b-ii：**判定必须有牙** —— 目标被忽略时本判据变红。
///
/// 三件事一起钉：
/// 1. 目标 = 实测 ⇒ `pass` 且 `deltaLu ≈ 0`；
/// 2. 目标 = 实测 − 6 LU ⇒ `fail` + `RENDER_FAILED`（带内结构化数据里给出实测/目标/差值）；
/// 3. **同一个目标会产出不同的读数**：两个请求（一个合理、一个离谱）在同一工程上
///    给出**不同**的 `verdict`，而**音频字节相同** —— 证明判定读的是母带而不是常量。
///
/// 注入（本机真的做过，见台账）：把 `build` 里的
/// `if loudness.verdict == VERDICT_FAIL { return Err(loudness.failure()); }` 换成
/// 注释掉 ⇒ 第 2 步不再报错 ⇒ 本判据红。
#[test]
fn the_loudness_target_has_teeth() {
    let scratch = Scratch::new("loudness-teeth");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));

    // 1) 先拿到实测值（不给目标）。
    let probe_path = scratch.join("probe.wav");
    let probe = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": probe_path.display().to_string()}),
    );
    assert_eq!(probe["status"], "success", "{probe}");
    let measured = probe["data"]["loudness"]["measuredIntegratedLufs"]
        .as_f64()
        .expect("实测读数");
    assert!(measured.is_finite());

    // 2) 目标 = 实测 ⇒ 达标。
    let hit_path = scratch.join("hit.wav");
    let hit = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "targetLufs": measured,
            "path": hit_path.display().to_string(),
        }),
    );
    assert_eq!(hit["status"], "success", "{hit}");
    let verdict = &hit["data"]["loudness"];
    assert_eq!(verdict["verdict"], "pass", "{hit}");
    // 目标值是 **f32**（契约里 `targetLufs` 是 `number`，实现按 f32 收窄）。
    // 把响应里的 f64 写回 JSON 再读回 f32 会经过一次十进制往返，因此这里比**数值**：
    // 往返误差必须小于 1e-6 LU（比容差小五个数量级）。
    let echoed = verdict["targetLufs"].as_f64().expect("targetLufs");
    assert!(
        (echoed - measured).abs() < 1.0e-6,
        "回显的目标 {echoed} 必须等于实测值 {measured} 的 f32 表示"
    );
    let delta = verdict["deltaLu"].as_f64().expect("deltaLu");
    assert!(
        delta.abs() < 1.0e-4,
        "目标 = 实测时差值必须是数值噪声: {delta}"
    );
    // 判定**不改音频**：产物与不给目标时逐字节相同。
    assert_eq!(
        fs::read(&probe_path).expect("probe"),
        fs::read(&hit_path).expect("hit"),
        "响度判定是只读的: 加一个已经达标的 targetLufs 不许改动一个字节"
    );

    // 3) 目标 = 实测 − 6 LU ⇒ **未达标**，带内报 `RENDER_FAILED` + 结构化数字。
    let miss_path = scratch.join("miss.wav");
    let miss = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "targetLufs": measured - 6.0,
            "path": miss_path.display().to_string(),
        }),
    );
    assert_eq!(miss["status"], "error", "{miss}");
    assert_eq!(miss["error"]["code"], "RENDER_FAILED", "{miss}");
    assert_eq!(miss["error"]["data"]["reason"], "loudnessTargetMissed");
    let reported = miss["error"]["data"]["measuredIntegratedLufs"]
        .as_f64()
        .expect("失败报文里必须给实测值");
    assert!(
        (reported - measured).abs() < 1.0e-4,
        "失败报文里的实测值 {reported} 必须等于只读探测到的 {measured}"
    );
    let echoed_target = miss["error"]["data"]["targetLufs"]
        .as_f64()
        .expect("失败报文里必须给目标");
    assert!(
        (echoed_target - (measured - 6.0)).abs() < 1.0e-6,
        "失败报文里的目标 {echoed_target} 必须是请求的那个 (f32 往返)"
    );
    assert_eq!(miss["error"]["data"]["toleranceLu"], 0.5);
    // **失败时不许落盘**：未达标的母带不是可交付物。
    assert!(
        !miss_path.exists(),
        "未达标时不许写出产品: {:?}",
        entries(&scratch.dir)
    );
}

/// 判据 3b-iii：**目标被忽略时本判据变红**（"报告是常量"的负向测量）。
///
/// 三个请求打在同一工程上：`无目标` / `目标 = 实测` / `目标 = 实测 − 12 LU`。
/// 若实现忽略 `targetLufs`，三个 `verdict` 会相同（都 `noTarget`）⇒ 红。
/// 若实测读数是硬编码常量，三者的 `measuredIntegratedLufs` 会与母带无关 ——
/// 用"改工程音量后读数必须跟着走"再钉一次（见下一条判据）。
#[test]
fn the_target_changes_the_verdict_so_it_cannot_be_ignored() {
    let scratch = Scratch::new("loudness-ignored");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let mut verdicts = Vec::new();
    for (index, target) in [None, Some(-3.0f64), Some(-40.0)].into_iter().enumerate() {
        let path = scratch.join(&format!("v{index}.wav"));
        let mut arguments = json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": path.display().to_string(),
        });
        if let Some(target) = target {
            arguments["targetLufs"] = json!(target);
        }
        let result = call(&mut dispatcher, &auth, arguments);
        let verdict = match result["status"].as_str().expect("status") {
            "success" => result["data"]["loudness"]["verdict"]
                .as_str()
                .expect("verdict")
                .to_owned(),
            "error" => {
                assert_eq!(result["error"]["code"], "RENDER_FAILED", "{result}");
                String::from("fail")
            }
            other => panic!("非法的 status: {other}"),
        };
        verdicts.push(verdict);
    }
    assert_eq!(verdicts[0], "noTarget", "{verdicts:?}");
    assert_eq!(verdicts[1], "pass", "{verdicts:?}");
    assert_eq!(verdicts[2], "fail", "{verdicts:?}");
    assert_ne!(verdicts[0], verdicts[1]);
    assert_ne!(verdicts[1], verdicts[2]);
    assert_ne!(verdicts[0], verdicts[2]);
}

/// 判据 3b-iv：读数是**测出来的**（不是常量）—— 改母带电平 ⇒ 读数跟着走。
///
/// 同一工程，只改 `volume_db`（0 dB → −12 dB）⇒ 门限积分读数必须下降约 12 LU。
/// 若有人把读数写死成常量，本判据红。
#[test]
fn the_loudness_reading_follows_the_master_level() {
    let scratch = Scratch::new("loudness-levels");
    let mut readings = Vec::new();
    for (tag, volume_db) in [("loud", 0.0f32), ("quiet", -12.0)] {
        let spec = Spec {
            volume_db,
            ..Spec::default()
        };
        let path = scratch.join(&format!("{tag}.wav"));
        let (mut dispatcher, auth) =
            dispatcher_with(&project(&spec), &scratch.join(&format!("{tag}.yeban")));
        let result = call(
            &mut dispatcher,
            &auth,
            json!({"format": "wav", "sampleRate": 48000, "path": path.display().to_string()}),
        );
        assert_eq!(result["status"], "success", "{result}");
        readings.push(
            result["data"]["loudness"]["measuredIntegratedLufs"]
                .as_f64()
                .expect("读数"),
        );
    }
    let drop = readings[0] - readings[1];
    assert!(
        (drop - 12.0).abs() < 0.6,
        "母带电平降 12 dB ⇒ 读数必须降 ≈12 LU, 实测降了 {drop} ({readings:?})"
    );
}

/// 判据 3b-v：非法目标只用契约内的错误码，且**没有**任何文件被写出。
///
/// 三个输入形状各走一条出口：
/// - 数字越出 `[-70, 0]` ⇒ 领域 `INVALID_PARAMETER_RANGE`（带区间）；
/// - 字符串 ⇒ **JSON-RPC 层** `-32602`（`ToolSpec::validate_arguments` 的类型闸门,
///   它比领域校验更早 —— 这与既有 `normalize`/`path` 的处置完全一致）；
/// - 端点 `-70.0` / `0.0` **合法**（闭区间），因此进入判定并以 `RENDER_FAILED` 收尾。
#[test]
fn an_impossible_loudness_target_is_an_invalid_parameter() {
    let scratch = Scratch::new("loudness-bad");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    for (index, target) in [-70.001f64, 0.001, -1000.0, 12.0].into_iter().enumerate() {
        let result = call(
            &mut dispatcher,
            &auth,
            json!({
                "format": "wav",
                "sampleRate": 48000,
                "targetLufs": target,
                "path": scratch.join(&format!("bad{index}.wav")).display().to_string(),
            }),
        );
        assert_domain_error(
            &result,
            "INVALID_PARAMETER_RANGE",
            &format!("targetLufs = {target}"),
        );
        assert!(
            result["error"]["data"]["reason"].as_str().is_some(),
            "参数错必须带结构化原因: {result}"
        );
        assert!(
            !scratch.join(&format!("bad{index}.wav")).exists(),
            "参数错不许写出任何产品"
        );
    }
    // 边界值本身是**接受**的（闭区间）。
    for target in [-70.0f64, 0.0] {
        let result = call(
            &mut dispatcher,
            &auth,
            json!({"format": "wav", "sampleRate": 48000, "targetLufs": target, "dryRun": true}),
        );
        // 端点合法 ⇒ 结论是"未达标"（RENDER_FAILED），**不是**参数错。
        assert_eq!(
            result["status"], "error",
            "端点 {target} 必须合法到能进入判定: {result}"
        );
        assert_eq!(result["error"]["code"], "RENDER_FAILED", "{result}");
        assert_eq!(result["error"]["data"]["reason"], "loudnessTargetMissed");
    }
    // 非数字形状走 **JSON-RPC 层**的类型闸门（`-32602`），与 `normalize` / `path` 同一条。
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {
            "name": "yeban_render_master",
            "arguments": {"format": "wav", "sampleRate": 48000, "targetLufs": "loud"}
        }
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert!(response.result.is_none(), "{response:?}");
    let error = response.error.expect("字符串目标必须被类型闸门拦下");
    assert_eq!(error.code, -32602, "{error:?}");
    assert!(
        format!("{error:?}").contains("targetLufs"),
        "报文必须点名 `targetLufs`: {error:?}"
    );
}

/// 判据 3b-vi：**纯静音母带 + 目标** ⇒ `measured = null` 且不达标，但**不许**是 NaN/Infinity。
///
/// 边界口径：门限积分的静音读数是负无穷，而负无穷不是合法 JSON。实现把它写成
/// `measuredIntegratedLufs: null` + `measurementNote`，然后按"测不出 ⇒ 无法判定"处理。
#[test]
fn a_silent_master_cannot_meet_a_loudness_target_and_never_reports_infinity() {
    let scratch = Scratch::new("loudness-silent");
    let silent = Spec {
        velocity: 0,
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&silent), &scratch.join("silent.yeban"));
    // 不给目标：读数缺席，但**必须**有原因，且没有任何非有限数字流进响应。
    let plain_path = scratch.join("plain.wav");
    let plain = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": plain_path.display().to_string()}),
    );
    assert_eq!(plain["status"], "success", "{plain}");
    let loudness = &plain["data"]["loudness"];
    assert_eq!(loudness["verdict"], "noTarget");
    assert_eq!(loudness["measured"], false);
    assert_eq!(loudness["measuredIntegratedLufs"], Value::Null);
    assert_eq!(loudness["deltaLu"], Value::Null);
    assert!(
        loudness["measurementNote"].is_string(),
        "缺席必须带原因: {plain}"
    );
    // 给目标：测不出 ⇒ 未达标（不是"假装达标"）。
    let targeted = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "targetLufs": -14.0,
            "path": scratch.join("targeted.wav").display().to_string(),
        }),
    );
    assert_eq!(targeted["status"], "error", "{targeted}");
    assert_eq!(targeted["error"]["code"], "RENDER_FAILED", "{targeted}");
    assert_eq!(
        targeted["error"]["data"]["reason"], "loudnessTargetMissed",
        "测不出就是未达标: {targeted}"
    );
    assert_eq!(
        targeted["error"]["data"]["measuredIntegratedLufs"],
        Value::Null,
        "测不出时不许编一个读数: {targeted}"
    );
    assert!(
        targeted["error"]["data"]["measurementNote"].is_string(),
        "失败报文必须带原因: {targeted}"
    );
    // 全响应的文本里不许出现 Infinity / NaN（那会让契约样本非法）。
    let text = plain.to_string() + &targeted.to_string();
    assert!(!text.contains("Infinity"), "{text}");
    assert!(!text.contains("NaN"), "{text}");
}

/// 判据 3b-vii：**默认路径逐字节不变**（`MUST-GATE-002` 的同平台位级 L1 渲染）。
///
/// 交付形态：**同一份工程**渲染两次，一次不带 `targetLufs`、一次带一个已达标的目标，
/// 两份产物逐字节相同；且**不带目标的那一份**与仓库钉死的两个常量
/// （`masterDigest` / 文件 `sha256`，见
/// [`two_renders_of_the_same_project_are_byte_identical`]）**仍然**相同。
/// 后者才是"本切片没有改动默认渲染路径"的强形式：它不是自比自。
#[test]
fn the_default_rendering_path_stays_bit_identical_with_a_loudness_target_attached() {
    let scratch = Scratch::new("loudness-determinism");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let plain_path = scratch.join("plain.wav");
    let plain = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": plain_path.display().to_string()}),
    );
    assert_eq!(plain["status"], "success", "{plain}");
    // **钉死的常量**（与判据 2 同一对数字）：默认路径的位级身份没有漂。
    assert_eq!(
        plain["data"]["masterDigest"],
        "b242d510d581541732134d3c0e233a11d6045ffb65535675adc73501fb28eefb",
        "默认路径的母带样本摘要必须与本切片之前逐位相同"
    );
    assert_eq!(
        plain["data"]["sha256"], "b9472fcd20086efd4953d457dde26169d1374f592bc90a0cf7b81304e0b69cc4",
        "默认路径的文件摘要必须与本切片之前逐位相同"
    );

    let measured = plain["data"]["loudness"]["measuredIntegratedLufs"]
        .as_f64()
        .expect("实测读数");
    let targeted_path = scratch.join("targeted.wav");
    let targeted = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "targetLufs": measured,
            "path": targeted_path.display().to_string(),
        }),
    );
    assert_eq!(targeted["status"], "success", "{targeted}");
    assert_eq!(
        targeted["data"]["loudness"]["verdict"], "pass",
        "{targeted}"
    );
    let a = fs::read(&plain_path).expect("plain");
    let b = fs::read(&targeted_path).expect("targeted");
    assert_eq!(a, b, "挂一个已达标的目标不许改动产物的任何一个字节");
    assert_eq!(plain["data"]["sha256"], targeted["data"]["sha256"]);
    assert_eq!(
        plain["data"]["masterDigest"],
        targeted["data"]["masterDigest"]
    );
    assert_eq!(plain["data"]["frames"], targeted["data"]["frames"]);
    assert_eq!(
        plain["data"]["peak"]["after"],
        targeted["data"]["peak"]["after"]
    );
}

/// 判据 3b-viii：`dryRun` 与真调用给**同一份**判定（`targetLufs` 也不例外）。
#[test]
fn dry_run_preview_and_the_real_call_agree_on_the_loudness_verdict() {
    let scratch = Scratch::new("loudness-dryrun");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let preview = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "targetLufs": -3.0, "dryRun": true}),
    );
    assert_eq!(preview["status"], "success", "{preview}");
    let preview_loudness = preview["data"]["preview"]["loudness"].clone();
    let real = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "targetLufs": -3.0,
            "path": scratch.join("real.wav").display().to_string(),
        }),
    );
    assert_eq!(real["status"], "success", "{real}");
    let real_loudness = real["data"]["loudness"].clone();
    assert_eq!(
        preview_loudness, real_loudness,
        "预览与真做的响度块必须逐字段相同"
    );
    assert_eq!(preview_loudness["verdict"], "pass");
}

// ---------------------------------------------------------------------------
// 判据 4：dryRun 不落盘
// ---------------------------------------------------------------------------

#[test]
fn dry_run_writes_nothing_and_says_where_it_would_write() {
    let scratch = Scratch::new("dryrun");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let before =
        serde_json::to_string(dispatcher.domain().active_project().expect("工程")).expect("序列化");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "dryRun": true}),
    );
    assert_eq!(result["status"], "success", "{result}");
    let preview = &result["data"]["preview"];
    assert_eq!(preview["wired"], true);
    assert_eq!(preview["writesFile"], true);
    assert_eq!(
        preview["wouldWrite"]["path"],
        scratch.text("demo.master.wav"),
        "缺省路径规则: <工程 stem>.master.<format> 与工程同目录"
    );
    assert_eq!(preview["wouldWrite"]["frames"], 48_000);
    assert_eq!(preview["wouldWrite"]["bytes"], preview["bytes"]);
    assert_eq!(
        preview["wouldWrite"]["sha256"].as_str().expect("sha").len(),
        64,
        "dryRun 也能给出**实测**的文件摘要 (确定性渲染)"
    );
    assert_eq!(preview["normalize"]["applied"], false);
    // 磁盘上什么都不能有（连临时文件都没有）。
    assert!(
        entries(&scratch.dir).is_empty(),
        "dryRun 不许产生任何文件: {:?}",
        entries(&scratch.dir)
    );
    // 工程字节与提交数不变。
    let after =
        serde_json::to_string(dispatcher.domain().active_project().expect("工程")).expect("序列化");
    assert_eq!(before, after, "dryRun 不得改工程字节");
}

// ---------------------------------------------------------------------------
// 判据 5：幂等（不重复渲染）
// ---------------------------------------------------------------------------

#[test]
fn the_same_idempotency_key_does_not_render_twice() {
    let scratch = Scratch::new("idempotency");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let out = scratch.join("master.wav");
    let arguments = json!({
        "format": "wav",
        "sampleRate": 48000,
        "path": out.display().to_string(),
        "idempotencyKey": "render-once",
    });
    let first = call(&mut dispatcher, &auth, arguments.clone());
    assert_eq!(first["status"], "success", "{first}");
    assert!(out.is_file(), "第一次必须真的落盘");
    let modified = fs::metadata(&out)
        .expect("元数据")
        .modified()
        .expect("mtime");
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt as _;
        fs::metadata(&out).expect("元数据").ino()
    };

    let second = call(&mut dispatcher, &auth, arguments.clone());
    assert_eq!(second["replayed"], true, "第二次必须命中幂等缓存");
    assert_eq!(
        second["response"]["result"], first,
        "重放的主体必须与首次逐字节相同"
    );
    assert_eq!(
        fs::metadata(&out)
            .expect("元数据")
            .modified()
            .expect("mtime"),
        modified,
        "命中缓存 ⇒ 产物不得被重写"
    );
    #[cfg(unix)]
    assert_eq!(
        {
            use std::os::unix::fs::MetadataExt as _;
            fs::metadata(&out).expect("元数据").ino()
        },
        inode,
        "命中缓存 ⇒ 产物 inode 不得改变 (原子替换会换 inode)"
    );
    assert_eq!(dispatcher.replayed(), 1);

    // 最强的一条: 删掉产物再重放同键 —— 文件**不会**被重建,
    // 因此第二次调用确实没有进入渲染路径, 而不是"渲染出一样的字节"。
    fs::remove_file(&out).expect("删掉产物");
    let third = call(&mut dispatcher, &auth, arguments);
    assert_eq!(third["replayed"], true);
    assert!(!out.exists(), "同键重放不得重新渲染");
    // 不同键 ⇒ 真渲染（幂等不是"永远不渲"）。
    let other = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": out.display().to_string(),
            "idempotencyKey": "render-again",
        }),
    );
    assert!(other.get("replayed").is_none(), "不同键必须真执行");
    assert!(out.is_file(), "不同键必须真的重新渲染并落盘");
}

// ---------------------------------------------------------------------------
// 判据 6：不可写目录 ⇒ IO_ERROR 且不留半个文件
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn read_only_output_directory_is_an_io_error_and_keeps_the_previous_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = Scratch::new("readonly");
    let locked_dir = scratch.join("locked");
    fs::create_dir_all(&locked_dir).expect("建只读目录");
    let out = locked_dir.join("master.wav");
    // 预先存在一份**旧产物**: 原子写必须保住它, 原地写会把它截断。
    fs::write(&out, b"PREVIOUS-MASTER").expect("占位产物");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));

    fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o555)).expect("改成只读");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o755)).expect("恢复权限");

    assert_domain_error(&result, "IO_ERROR", "只读目录");
    assert_eq!(
        fs::read(&out).expect("读旧产物"),
        b"PREVIOUS-MASTER",
        "失败时旧产物必须逐字节不变 (原地写会截断它)"
    );
    let leftovers: Vec<String> = entries(&locked_dir)
        .into_iter()
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "失败时必须清理临时文件: {leftovers:?}"
    );
}

// ---------------------------------------------------------------------------
// 判据 7：参数非法 / 不可实现的组合 ⇒ 契约内错误码
// ---------------------------------------------------------------------------

#[test]
fn every_impossible_request_returns_a_contract_error_code() {
    let scratch = Scratch::new("errors");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let out = scratch.join("master.wav");
    // 注意: **缺少必填参数**与**参数类型不符**（如 `normalize: "yes"`、`path: 7`）
    // 由契约层（`ToolCall::validate_arguments`）在进入领域之前拦成 JSON-RPC `-32602`，
    // 不属于领域错误码映射；领域层的那道兜底由 `domain/render.rs` 的单元判据钉住。
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "不支持的格式",
            json!({"format": "mp3", "sampleRate": 48000}),
            "INVALID_PARAMETER_RANGE",
        ),
        (
            "采样率不在模型集合",
            json!({"format": "wav", "sampleRate": 12345}),
            "INVALID_PARAMETER_RANGE",
        ),
    ];
    for (context, arguments, expected) in cases {
        let result = call(&mut dispatcher, &auth, arguments);
        assert_domain_error(&result, expected, context);
        assert!(!out.exists(), "{context} 失败时不许留下半成品");
    }
    // 采样率 ≠ 工程采样率**不再是错误**: rubato sinc 重采样已接线
    // (ARCH-DSP-002 / ADR-0001 D26, 见 `line/audio-render`)。
    // 这里断言它真的产出一份 44.1 kHz 母带, 并如实报告重采样口径 ——
    // 上一版的 `data.unwired = "resampler"` 拒绝必须**退役**。
    let mismatched = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 44100}),
    );
    assert_eq!(mismatched["status"], "success", "{mismatched}");
    assert_eq!(mismatched["data"]["sampleRate"], 44100);
    assert_eq!(
        mismatched["data"]["frames"], 44100,
        "120 BPM / 960 PPQ / 1920 tick == 1 秒 @ 44.1 kHz"
    );
    assert_eq!(mismatched["data"]["durationSeconds"], 1.0);
    assert_eq!(mismatched["data"]["audio"]["wired"], true);
    let method = mismatched["data"]["audio"]["resampler"]["method"]
        .as_str()
        .expect("resampler.method");
    assert!(
        method.contains("rubato"),
        "必须写清用的是哪种重采样: {method}"
    );
    assert!(
        mismatched["data"].get("unwired").is_none(),
        "重采样已接线, 不该再出现 `unwired`: {mismatched}"
    );

    // 没有任何内容 ⇒ 0 帧 ⇒ RENDER_FAILED（而不是写一个 0 帧文件冒充成功）。
    let empty = Spec {
        with_placement: false,
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&empty), &scratch.join("empty.yeban"));
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_domain_error(&result, "RENDER_FAILED", "0 帧");
    assert!(!out.exists());

    // 没有活跃工程 ⇒ NO_ACTIVE_PROJECT（会话前提先于参数校验）。
    let token = BearerToken::generate().token;
    let fresh_auth = format!("Bearer {}", token.expose());
    let mut fresh = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    let result = call(
        &mut fresh,
        &fresh_auth,
        json!({"format": "wav", "sampleRate": 48000}),
    );
    assert_domain_error(&result, "NO_ACTIVE_PROJECT", "没有工程");
}

// ---------------------------------------------------------------------------
// 判据 8：输出路径护栏
// ---------------------------------------------------------------------------

#[test]
fn output_path_may_not_be_the_project_or_the_lock_file() {
    let scratch = Scratch::new("guard");
    let project_path = scratch.join("demo.yeban");
    let (mut dispatcher, auth) = dispatcher_with(&project(&Spec::default()), &project_path);
    let cases = [
        project_path.clone(),
        PathBuf::from(format!("{}.lock", project_path.display())),
    ];
    for bad in cases {
        let result = call(
            &mut dispatcher,
            &auth,
            json!({
                "format": "wav",
                "sampleRate": 48000,
                "path": bad.display().to_string(),
            }),
        );
        assert_domain_error(&result, "INVALID_PARAMETER_RANGE", "输出路径护栏");
        assert!(!bad.exists(), "护栏必须在写盘之前生效: {bad:?}");
    }
}

// ---------------------------------------------------------------------------
// 判据 9：母带不是静音 + 声相真的生效
// ---------------------------------------------------------------------------

#[test]
fn the_master_is_not_silent_and_panning_moves_the_image() {
    let scratch = Scratch::new("content");
    // 全左声相: 左声道能量必须显著大于右声道。
    let spec = Spec {
        pan: -1.0,
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&spec), &scratch.join("demo.yeban"));
    let out = scratch.join("master.wav");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{result}");
    let samples = verify_file_shape(&result, &out);
    let left = channel_rms(&samples, 2, 0);
    let right = channel_rms(&samples, 2, 1);
    assert!(left > 0.05, "母带不能是静音, 左声道 RMS = {left}");
    assert!(
        left > right * 50.0,
        "全左声相必须让左右严重不对称: L={left} R={right}"
    );
    assert_eq!(result["data"]["sources"][0]["kind"], "midi-synth-osc");
    assert_eq!(result["data"]["sources"][0]["audible"], true);
    assert_eq!(result["data"]["sources"][0]["notes"], 1);

    // 居中时左右对称。
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("c.yeban"));
    let centered_path = scratch.join("centered.wav");
    let centered = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": centered_path.display().to_string()}),
    );
    let samples = verify_file_shape(&centered, &centered_path);
    let left = channel_rms(&samples, 2, 0);
    let right = channel_rms(&samples, 2, 1);
    assert!(
        (left - right).abs() < 1.0e-4,
        "居中的声相必须左右对称: L={left} R={right}"
    );
}

// ---------------------------------------------------------------------------
// 判据 9b：工具面写进去的静音**真的**在母带里生效（不是只写进了工程 JSON）
// ---------------------------------------------------------------------------

/// 经 `yeban_edit_notes` 的 `ops[].kind == "setTrackMute"` 写静音 ⇒ 母带里那条源轨
/// `audible == false`、浮点母带严格为 0、解码样本只剩 TPDF 抖动；再写回 `false`
/// ⇒ 产物与写之前**逐字节相同**。
///
/// 这条判据把"开关可达"与"渲染器真的读它"接在一起：`render` 的 `audible` 判定读
/// `TrackV3::mute`，而在这个 `kind` 之前整个 `crates/yeban-mcp` **没有**任何 `Op` 写者
/// （`Op::SetTrackMute` / `Op::SetTrackSolo` 的构造点数为 0）⇒ 已实现的静音能力在工具面
/// 不可达。它是 `setParam` 那一票的同一族缺口。
///
/// 可被什么注入破坏：删掉 `parse_one` 的开关分支（工具调用红）、把撤销载荷写死成
/// 常量（`old_mute` 断言红）、或让渲染器忽略 `mute`（`audible` / 峰值红 —— 那条注入
/// 落在另一个 crate，本判据仍会如实变红）。
#[test]
fn a_mute_written_through_the_tool_face_silences_the_source_in_the_master() {
    let scratch = Scratch::new("mute-tool-face");
    let spec = Spec::default();
    let (mut dispatcher, auth) = dispatcher_with(&project(&spec), &scratch.join("demo.yeban"));
    let lead = id(2).to_canonical_string();
    let clip = id(10).to_canonical_string();

    // 基线：未静音时那条源轨真的发声（判据必须有"能区分"的一侧）。
    let plain_path = scratch.join("plain.wav");
    let plain = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": plain_path.display().to_string()}),
    );
    assert_eq!(plain["status"], "success", "{plain}");
    assert_eq!(
        plain["data"]["sources"][0]["audible"], true,
        "未静音时源轨必须发声: {plain}"
    );
    let plain_samples = verify_file_shape(&plain, &plain_path);
    assert!(
        peak(&plain_samples) > DITHER_ONLY_PEAK,
        "基线母带不能只是抖动: 实测峰值 {}",
        peak(&plain_samples)
    );

    // 经工具面写静音（提案 → 合并），撤销载荷必须来自当前文档。
    let written = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead, "clipId": clip, "includeOps": true,
            "ops": [{"kind": "setTrackMute", "value": true}]
        }),
    );
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        written["data"]["proposal"]["ops"][0]["op"]["SetTrackMute"]["old_mute"], false,
        "撤销载荷必须等于文档现值: {written}"
    );
    let proposal_id = written["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "mute"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");

    // 母带说这一轨不发声，且浮点母带严格为 0（文件里只剩 TPDF 抖动）。
    let muted_path = scratch.join("muted.wav");
    let muted = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": muted_path.display().to_string()}),
    );
    assert_eq!(muted["status"], "success", "{muted}");
    assert_eq!(
        muted["data"]["sources"][0]["audible"], false,
        "静音后源轨必须被渲染器判为不发声: {muted}"
    );
    assert_eq!(
        muted["data"]["peak"]["after"], 0.0,
        "抖动前的浮点母带必须严格为 0: {muted}"
    );
    let muted_samples = verify_file_shape(&muted, &muted_path);
    assert!(
        peak(&muted_samples) <= DITHER_ONLY_PEAK,
        "静音后只允许 TPDF 抖动: 实测峰值 {}",
        peak(&muted_samples)
    );

    // 写回 `false` ⇒ 产物与写之前**逐字节相同**（开关完全可逆，音频层面也逐字节）。
    let restored = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead, "clipId": clip, "includeOps": true,
            "ops": [{"kind": "setTrackMute", "value": false}]
        }),
    );
    assert_eq!(restored["status"], "success", "{restored}");
    assert_eq!(
        restored["data"]["proposal"]["ops"][0]["op"]["SetTrackMute"]["old_mute"], true,
        "第二次写入的撤销载荷必须是上一次真的落盘的那个 true: {restored}"
    );
    let restored_id = restored["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": restored_id, "commitMessage": "unmute"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    let unmuted_path = scratch.join("unmuted.wav");
    let unmuted = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": unmuted_path.display().to_string()}),
    );
    assert_eq!(unmuted["status"], "success", "{unmuted}");
    assert_eq!(
        unmuted["data"]["sources"][0]["audible"], true,
        "取消静音后源轨必须重新发声: {unmuted}"
    );
    assert_eq!(
        fs::read(&unmuted_path).expect("产物"),
        fs::read(&plain_path).expect("基线产物"),
        "静音写回 false 之后, 母带产物必须与写之前逐字节相同"
    );
}

// ---------------------------------------------------------------------------
// 判据 10：延迟表来自 DeviceDefinition::latency_samples
// ---------------------------------------------------------------------------

#[test]
fn longest_path_latency_comes_from_the_device_definitions() {
    let scratch = Scratch::new("latency");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let plain = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": scratch.text("plain.wav")}),
    );
    assert_eq!(
        plain["data"]["longestPathFrames"], 0,
        "没有设备 ⇒ 没有补偿延迟"
    );

    // 32 + 16 = 48 帧（未旁通设备相加）；旁通设备**完全不计入**。
    let spec = Spec {
        devices: vec![
            device(101, 32, false),
            device(102, 16, false),
            device(103, 999, true),
        ],
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&spec), &scratch.join("dev.yeban"));
    let out = scratch.join("dev.wav");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(
        result["data"]["longestPathFrames"], 48,
        "延迟只能来自 DeviceDefinition::latency_samples (跳过旁通设备)"
    );
    // 设备链 DSP 没实现 ⇒ 必须如实登记。
    assert!(
        result["data"]["unsupported"]
            .as_array()
            .expect("unsupported")
            .iter()
            .any(|key| key == "deviceChainDsp"),
        "{result}"
    );
    verify_file_shape(&result, &out);
}

// ---------------------------------------------------------------------------
// 判据 11：format 只改容器，不改音频
// ---------------------------------------------------------------------------

#[test]
fn the_requested_format_changes_the_container_but_not_the_audio() {
    let scratch = Scratch::new("format");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let wav_path = scratch.join("a.wav");
    let rf64_path = scratch.join("a.rf64");
    let wav = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": wav_path.display().to_string()}),
    );
    let rf64 = call(
        &mut dispatcher,
        &auth,
        json!({"format": "rf64", "sampleRate": 48000, "path": rf64_path.display().to_string()}),
    );
    assert_eq!(wav["data"]["container"], "RIFF");
    assert_eq!(rf64["data"]["container"], "RF64");
    assert_eq!(
        wav["data"]["bytes"],
        json!(
            wav["data"]["headerBytes"].as_u64().expect("headerBytes")
                + wav["data"]["payloadBytes"].as_u64().expect("payloadBytes")
        ),
        "RIFF 总字节 == 头 + 负载 (偶数负载没有补位)"
    );
    let wav_bytes = fs::read(&wav_path).expect("wav");
    let rf64_bytes = fs::read(&rf64_path).expect("rf64");
    assert_eq!(&wav_bytes[..4], b"RIFF");
    assert_eq!(&rf64_bytes[..4], b"RF64");
    let wav_parsed = parse_container(&wav_bytes).expect("wav 解析");
    let rf64_parsed = parse_container(&rf64_bytes).expect("rf64 解析");
    assert_eq!(
        &wav_bytes[wav_parsed.data.clone()],
        &rf64_bytes[rf64_parsed.data.clone()],
        "容器格式不得改变音频负载"
    );
    assert_eq!(wav["data"]["payloadBytes"], rf64["data"]["payloadBytes"]);
    assert_eq!(wav["data"]["masterDigest"], rf64["data"]["masterDigest"]);
    // bw64 也必须能写（本实现产出的是其 ds64 + fmt + bext 子集）。
    let bw64_path = scratch.join("a.bw64");
    let bw64 = call(
        &mut dispatcher,
        &auth,
        json!({"format": "bw64", "sampleRate": 48000, "path": bw64_path.display().to_string()}),
    );
    assert_eq!(bw64["data"]["container"], "BW64");
    assert_eq!(&fs::read(&bw64_path).expect("bw64")[..4], b"BW64");
}

// ---------------------------------------------------------------------------
// 判据 12：unsupported 载荷是"如实的", 不是一句套话
// ---------------------------------------------------------------------------

#[test]
fn unsupported_features_are_named_and_counted_only_when_present() {
    let scratch = Scratch::new("unsupported");
    // 循环配置请求重复播放, 而本渲染器只渲染第一遍 ⇒ 必须登记 `clipLoopRepetition`。
    let looping = Spec {
        loop_config: LoopConfig {
            enabled: true,
            start_tick: 0,
            end_tick: 3_840,
        },
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&looping), &scratch.join("demo.yeban"));
    let out = scratch.join("master.wav");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{result}");
    let unsupported = result["data"]["unsupported"]
        .as_array()
        .expect("unsupported");
    assert!(
        unsupported.iter().any(|key| key == "clipLoopRepetition"),
        "循环重复没有渲染, 必须登记: {result}"
    );
    assert_eq!(
        result["data"]["unsupportedCounts"]["clipLoopRepetition"], 1,
        "出现次数也要如实给出"
    );
    assert!(
        !unsupported.iter().any(|key| key == "audioClips"),
        "这个夹具里没有音频片段, 不许凭空登记: {unsupported:?}"
    );

    // 干净的工程(无循环/无设备)的 unsupported 必须是空数组 —— 证明上面那条不是套话。
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("d.yeban"));
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": scratch.text("clean.wav")}),
    );
    assert_eq!(
        result["data"]["unsupported"],
        json!([]),
        "干净工程不该有 unsupported 项: {result}"
    );
    assert_eq!(result["data"]["unsupportedCounts"], json!({}));
}

// ---------------------------------------------------------------------------
// 判据 12b：循环区间**短于**摆放跨度 ⇒ 请求了重复 ⇒ 必须登记 + 不得静默丢掉
// ---------------------------------------------------------------------------

/// 渲染一个只调循环配置的夹具工程，返回（响应, 产物路径）。
fn render_with_loop(scratch: &Scratch, tag: &str, loop_config: LoopConfig) -> (Value, PathBuf) {
    let spec = Spec {
        loop_config,
        ..Spec::default()
    };
    assert_eq!(spec.end_tick, 1_920, "夹具的摆放跨度 (tick)");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&spec), &scratch.join(&format!("{tag}.yeban")));
    let out = scratch.join(&format!("{tag}.wav"));
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{tag}: {result}");
    (result, out)
}

/// 循环区间 `0..960` 落在摆放跨度 `0..1920` **之内** ⇒ 摆放结束前必然回卷一次，
/// 也就是"作者请求了重复播放"。本渲染器只渲染第一遍（`0..duration_ticks`），
/// 因此这一条**必须**登记 `clipLoopRepetition`；同时母带字节必须与"循环关掉"
/// 的那一次渲染**逐位相同** —— 后者是"重复真的没有被展开"的机械证据。
///
/// 可被什么注入破坏：
/// - 把登记条件写回 `end_tick > duration_ticks`（只在区间越过摆放末端时才报）
///   ⇒ 本条第一段红：区间短于摆放这个**真会回卷**的形态被静默；
/// - 哪一天渲染器真的展开了重复 ⇒ 第二条 `masterDigest` 相等断言红。
#[test]
fn a_loop_shorter_than_the_placement_is_reported_and_not_expanded() {
    let scratch = Scratch::new("loop-short");
    let short_loop = LoopConfig {
        enabled: true,
        start_tick: 0,
        end_tick: 960,
    };
    let (result, out) = render_with_loop(&scratch, "short", short_loop);
    let unsupported = result["data"]["unsupported"]
        .as_array()
        .expect("unsupported");
    assert!(
        unsupported.iter().any(|key| key == "clipLoopRepetition"),
        "区间短于摆放跨度 = 请求重复, 本渲染器只播第一遍 ⇒ 必须登记: {result}"
    );
    assert_eq!(
        result["data"]["unsupportedCounts"]["clipLoopRepetition"], 1,
        "一个摆放出现一次: {result}"
    );

    // 循环关掉的那一次渲染：同一份夹具，唯一差别是 `loop_config`。
    let (plain, plain_out) = render_with_loop(&scratch, "plain", LoopConfig::default());
    assert_eq!(
        plain["data"]["unsupported"],
        json!([]),
        "关掉循环的工程不该登记: {plain}"
    );
    assert_eq!(
        plain["data"]["masterDigest"], DEFAULT_MASTER_DIGEST,
        "关循环的那一次就是既有默认夹具 (常量锚)"
    );
    assert_eq!(
        result["data"]["masterDigest"], plain["data"]["masterDigest"],
        "重复**没有**被展开: 母带样本摘要必须与关循环时相同"
    );
    assert_eq!(
        result["data"]["sha256"], plain["data"]["sha256"],
        "文件字节摘要同理"
    );
    assert_eq!(
        fs::read(&out).expect("产物"),
        fs::read(&plain_out).expect("产物"),
        "产物文件必须逐字节相同"
    );
}

/// 循环登记的**边界**：只有"恰好覆盖整个摆放跨度"的启用区间才不登记。
///
/// - `0..1920`（== 摆放跨度）⇒ 回卷点与摆放末端重合 ⇒ 播放结果与不循环相同 ⇒ 不登记；
/// - `480..1920`（区间不覆盖，且 `end_tick == duration_ticks`）⇒ **保守登记**：
///   坐标语义在规范里未裁决，本层不为"看起来更准"而少报。
///
/// 可被什么注入破坏：把条件写成 `end_tick >= duration_ticks`（丢掉 `start_tick == 0`
/// 那一半）⇒ 第二段红；写成恒真 ⇒ 第一段红。
#[test]
fn only_a_loop_that_covers_the_whole_placement_is_silent() {
    let scratch = Scratch::new("loop-covers");
    let covering = LoopConfig {
        enabled: true,
        start_tick: 0,
        end_tick: 1_920,
    };
    let (covers, _) = render_with_loop(&scratch, "covers", covering);
    assert_eq!(
        covers["data"]["unsupported"],
        json!([]),
        "区间 == 摆放跨度 ⇒ 没有重复可丢, 不许凭空登记: {covers}"
    );

    let offset = LoopConfig {
        enabled: true,
        start_tick: 480,
        end_tick: 1_920,
    };
    let (shifted, _) = render_with_loop(&scratch, "offset", offset);
    let unsupported = shifted["data"]["unsupported"]
        .as_array()
        .expect("unsupported");
    assert!(
        unsupported.iter().any(|key| key == "clipLoopRepetition"),
        "区间不是 0..duration_ticks 就不是覆盖, 保守登记: {shifted}"
    );
}

// ---------------------------------------------------------------------------
// 判据 13：概率触发（`probability`）走**模型层唯一判定入口**，不是本地掷骰子
// ---------------------------------------------------------------------------

/// 一条**只含 TPDF 抖动**的母带的幅度上界（单位：满量程）。
///
/// `yeban-render` 的 TPDF 抽样落在 `(-1, +1)` LSB 内（`dither::TPDF_PEAK_LSB = 1.0`），
/// 再叠加 24-bit 量化的 `±0.5` LSB 舍入 ⇒ **数字静音**的输入产出的每个样本都在
/// `±1.5` LSB 内。取 `2` LSB 作判据上界（`2 / 2^23 ≈ 2.4e-7`）。
///
/// 为什么需要这个常数：母带**永远**带抖动，所以"不发声"在文件里**不是**全零字节
/// —— 它是"只有抖动"（见下一行的 `peak.after == 0.0`：抖动前的浮点母带才是严格 0）。
const DITHER_ONLY_PEAK: f32 = 2.0 / 8_388_608.0;

/// 渲染一个"单独可调概率"的夹具工程，返回（响应, 解码后的样本）。
fn render_probability(
    scratch: &Scratch,
    tag: &str,
    probability: Option<f32>,
    rng_seed: u64,
    note_seed: u32,
) -> (Value, Vec<f32>) {
    let spec = Spec {
        probability,
        rng_seed,
        note_seed,
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&spec), &scratch.join("p.yeban"));
    let out = scratch.join(&format!("{tag}.wav"));
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{tag}: {result}");
    let samples = verify_file_shape(&result, &out);
    (result, samples)
}

/// **模型层的判定就是渲染器的判定**：对一批 `(概率, 身份)` 组合，逐条比较
/// `MidiNote::triggers(rng_seed)` 与"母带里这个音符到底响没响"。
///
/// 这条判据是"没有第二份实现"的机械证据：它**不**断言某个固定的静音/发声表
/// （那会把模型的哈希抄成第二份事实源），而是**现场问模型**再要求渲染器同意。
///
/// 可被什么注入破坏：把 `note.triggers(project.rng_seed)` 换成任何本地判定
/// （恒真 / 恒假 / `probability >= 0.5` / 换一个哈希）都会让某一行的两边不等。
#[test]
fn the_probability_decision_is_the_models_own_triggers_function() {
    let scratch = Scratch::new("probability-parity");
    // 覆盖端点（必然/永不）与中间值；身份种子变化让中间值的判定两侧都出现。
    let probabilities = [0.0f32, 1.0, 0.25, 0.5, 0.75];
    let mut silenced = 0usize;
    let mut audible = 0usize;
    for (index, note_seed) in [30u32, 31, 32, 33, 34, 35].into_iter().enumerate() {
        let note_id = id(note_seed);
        for probability in probabilities {
            let expected = MidiNote {
                probability: Some(probability),
                ..MidiNote::new(note_id, 0, 69, 1_920)
            }
            .triggers(DEFAULT_RNG_SEED);
            let (result, samples) = render_probability(
                &scratch,
                &format!("p{index}-{probability}"),
                Some(probability),
                DEFAULT_RNG_SEED,
                note_seed,
            );
            let sounds = peak(&samples) > DITHER_ONLY_PEAK;
            assert_eq!(
                sounds, expected,
                "概率 {probability} / 身份 {note_seed}: 模型说 {expected}, 母带说 {sounds}: {result}"
            );
            assert_eq!(
                result["data"]["probability"]["notesSilenced"],
                u64::from(!expected),
                "被静音的音符数必须等于模型判定为不触发的条数: {result}"
            );
            assert_eq!(
                result["data"]["sources"][0]["notesSilenced"],
                u64::from(!expected),
                "逐轨读数也要一致: {result}"
            );
            if expected {
                audible += 1;
            } else {
                silenced += 1;
            }
            // `noteProbability` 已经**做到**了 ⇒ 报它"没渲染"就是假话。
            assert!(
                !result["data"]["unsupported"]
                    .as_array()
                    .expect("unsupported")
                    .iter()
                    .any(|key| key == "noteProbability"),
                "概率触发已接线, 不得再登记 noteProbability: {result}"
            );
        }
    }
    // 判据必须**真的**把两边都覆盖到, 否则"恒静音"或"恒发声"的注入也能全绿。
    assert!(silenced > 0, "这批组合里必须有被判为不触发的音符");
    assert!(audible > 0, "这批组合里必须有被判为触发的音符");
}

/// 概率的**每一半**都是承重的：`0.0` 是不发声的数字静音（不是"小音量"），
/// `1.0` 与缺省是必然发声；且 `rng_seed` 真的参与判定（换种子会翻转某个身份）。
#[test]
fn probability_endpoints_and_the_seed_are_load_bearing() {
    let scratch = Scratch::new("probability-endpoints");
    // `0.0` ⇒ 母带**只有抖动**（抖动前的浮点母带逐样本为 0），响应如实报告一个被静音的音符。
    let (zero, samples) = render_probability(&scratch, "zero", Some(0.0), DEFAULT_RNG_SEED, 30);
    assert!(
        peak(&samples) <= DITHER_ONLY_PEAK,
        "probability=0.0 的音符必须一点声音都不出 (只允许 TPDF 抖动): 实测峰值 {}",
        peak(&samples)
    );
    assert_eq!(
        zero["data"]["peak"]["after"], 0.0,
        "抖动前的浮点母带必须严格为 0: {zero}"
    );
    assert_eq!(zero["data"]["probability"]["notesSilenced"], 1, "{zero}");
    assert_eq!(
        zero["data"]["probability"]["rngSeed"], DEFAULT_RNG_SEED,
        "响应必须报出判定用的种子: {zero}"
    );

    // `1.0` 与**缺省**（不写 `probability`）都必须发声 —— 缺省 = 旧行为。
    let (one, samples) = render_probability(&scratch, "one", Some(1.0), DEFAULT_RNG_SEED, 30);
    assert!(
        peak(&samples) > 0.1,
        "probability=1.0 必须发声 (远高于抖动底噪)"
    );
    assert_eq!(one["data"]["probability"]["notesSilenced"], 0, "{one}");
    let (absent, samples) = render_probability(&scratch, "absent", None, DEFAULT_RNG_SEED, 30);
    assert!(
        peak(&samples) > 0.1,
        "缺省 probability 必须发声 (远高于抖动底噪)"
    );
    assert_eq!(
        absent["data"]["probability"]["notesSilenced"], 0,
        "{absent}"
    );

    // 种子真的进判定：在同一个**身份**上扫种子，找到一对判定不同的种子，
    // 再**渲染两个工程**要求母带的"响没响"跟着变 —— 只查模型不算数，
    // 因为要判的是**渲染器**有没有消费 `rng_seed`。
    let note_id = id(30);
    let decision = |seed: u64| {
        MidiNote {
            probability: Some(0.5),
            ..MidiNote::new(note_id, 0, 69, 1_920)
        }
        .triggers(seed)
    };
    let chosen = (0u64..64).find(|seed| decision(*seed) != decision(DEFAULT_RNG_SEED));
    let Some(other_seed) = chosen else {
        panic!("64 个种子里 0.5 的判定一次都没翻转 ⇒ 种子没被消费");
    };
    let (other, other_samples) =
        render_probability(&scratch, "other-seed", Some(0.5), other_seed, 30);
    let other_sounds = peak(&other_samples) > DITHER_ONLY_PEAK;
    assert_ne!(
        other_sounds,
        peak(&samples) > DITHER_ONLY_PEAK,
        "种子 {other_seed} 与 {DEFAULT_RNG_SEED} 的判定不同, 母带也必须跟着不同: {other}"
    );
    assert_eq!(
        other["data"]["probability"]["rngSeed"], other_seed,
        "响应报的种子必须是这次用的那个: {other}"
    );
}

/// 判定属于**身份**而不是**位置**：把同一个音符在时间轴上平移（以及改力度），
/// 触发结论不变 —— 与模型文档的承诺一致（"概率属于身份"）。
///
/// 可被什么注入破坏：把哈希的输入从 `note.id` 换成 `note.start_tick` 之类。
#[test]
fn the_probability_decision_follows_the_note_identity_not_its_position() {
    for probability in [0.25f32, 0.5, 0.75] {
        let mut decisions = Vec::new();
        for start_tick in [0u64, 480, 960, 1_440] {
            let note_id = id(37);
            let note = MidiNote {
                probability: Some(probability),
                ..MidiNote::new(note_id, start_tick, 69, 240)
            };
            decisions.push(note.triggers(DEFAULT_RNG_SEED));
        }
        assert!(
            decisions.windows(2).all(|pair| pair[0] == pair[1]),
            "概率 {probability}: 平移不该改变触发判定, 实测 {decisions:?}"
        );
    }

    // 端到端复核：同一个身份/概率, 只改起点, 母带里的"响没响"必须一致。
    let scratch = Scratch::new("probability-identity");
    let mut verdicts = Vec::new();
    for (index, start_tick) in [0u64, 480, 960].into_iter().enumerate() {
        let note_id = id(37);
        let spec = Spec {
            probability: Some(0.35),
            note_seed: 37,
            ..Spec::default()
        };
        let mut fixture = project(&spec);
        // 平移音符（**身份不变**，只动位置与摆放下的一小段时值）。
        if let Some(ClipPoolEntry {
            content: ClipContent::Midi { notes },
            ..
        }) = fixture.clip_pool.get_mut(&id(10))
            && let Some(note) = notes.get_mut(&note_id)
        {
            note.start_tick = start_tick;
            note.duration_ticks = 240;
        }
        let (mut dispatcher, auth) = dispatcher_with(&fixture, &scratch.join("i.yeban"));
        let out = scratch.join(&format!("i{index}.wav"));
        let result = call(
            &mut dispatcher,
            &auth,
            json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
        );
        assert_eq!(result["status"], "success", "{result}");
        verdicts.push(result["data"]["probability"]["notesSilenced"].clone());
    }
    assert!(
        verdicts.windows(2).all(|pair| pair[0] == pair[1]),
        "同一身份平移起点后判定必须不变, 实测 {verdicts:?}"
    );
}

/// 两次渲染**逐位相同**（概率判定不引入任何熵），且不同种子确实产出不同母带。
///
/// 可被什么注入破坏：把确定性哈希换成平台 RNG / 线程局部状态。
#[test]
fn probability_renders_are_bit_identical_and_seed_sensitive() {
    let scratch = Scratch::new("probability-determinism");
    let (first, _) = render_probability(&scratch, "once", Some(0.5), DEFAULT_RNG_SEED, 30);
    let (second, _) = render_probability(&scratch, "twice", Some(0.5), DEFAULT_RNG_SEED, 30);
    assert_eq!(
        first["data"]["masterDigest"], second["data"]["masterDigest"],
        "同输入两次渲染的母带摘要必须相同"
    );
    assert_eq!(
        first["data"]["sha256"], second["data"]["sha256"],
        "文件必须逐字节相同"
    );

    // 换一个**判定翻转**的种子 ⇒ 母带摘要必须跟着变（证明种子真的进了渲染）。
    let note_id = id(30);
    let decision = |seed: u64| {
        MidiNote {
            probability: Some(0.5),
            ..MidiNote::new(note_id, 0, 69, 1_920)
        }
        .triggers(seed)
    };
    let Some(other_seed) = (0u64..64).find(|seed| decision(*seed) != decision(DEFAULT_RNG_SEED))
    else {
        panic!("64 个种子里 0.5 的判定一次都没翻转 ⇒ 种子没被消费");
    };
    let (other, _) = render_probability(&scratch, "seed-flip", Some(0.5), other_seed, 30);
    assert_ne!(
        first["data"]["masterDigest"], other["data"]["masterDigest"],
        "判定翻转的种子必须产出不同的母带摘要: {other}"
    );
}

// ---------------------------------------------------------------------------
// 判据 16：连击（`ratchet`）展开 —— 格点与实时引擎同一条公式
// ---------------------------------------------------------------------------

/// 渲染一个"连击次数 / 概率触发 / 音符时值可调"的单音符夹具，返回（响应, 解码样本）。
///
/// 夹具形状与 [`render_probability`] 一致（Master + 一条 MIDI 轨 + 一个摆放），
/// 只多一个 `ratchet` 旋钮。
fn render_note(
    scratch: &Scratch,
    tag: &str,
    ratchet: Option<u8>,
    probability: Option<f32>,
    end_tick: u64,
) -> (Value, Vec<f32>) {
    let spec = Spec {
        ratchet,
        probability,
        end_tick,
        ..Spec::default()
    };
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&spec), &scratch.join(&format!("{tag}.yeban")));
    let out = scratch.join(&format!("{tag}.wav"));
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{tag}: {result}");
    let samples = verify_file_shape(&result, &out);
    (result, samples)
}

/// 一个音符被展开成**恰好 `ratchet` 个脉冲**，格点就是实时引擎的那条公式
/// （`step = duration_ticks / ratchet`，第 `p` 个脉冲起于 `p * step`）。
///
/// "真的重新触发"的机械证据：每个脉冲都从**自己的起音包络**开始 ⇒ 在预期的脉冲
/// 起点上取 60 帧，能量必须远低于脉冲中段。若渲染器忽略 `ratchet`（旧的
/// "一个长音"行为），除第 0 个之外的所有起点都落在长音的**平段**上，这条就红。
#[test]
fn ratchet_expands_one_note_into_exactly_n_pulses_on_the_engines_grid() {
    let scratch = Scratch::new("ratchet-expansion");
    let per_tick = frames_per_tick();
    let duration_ticks = 1_920u64;
    for ratchet in [2u8, 3, 4, 6, 8, 16] {
        let pulses = usize::from(ratchet);
        let (result, samples) = render_note(
            &scratch,
            &format!("r{ratchet}"),
            Some(ratchet),
            None,
            duration_ticks,
        );
        // 机械读数: 排程音符数 = 脉冲数（每条都在响应里，不是估算）。
        assert_eq!(
            result["data"]["sources"][0]["notes"], pulses,
            "ratchet={ratchet}: 排程音符数必须等于脉冲数: {result}"
        );
        assert_eq!(
            result["data"]["sources"][0]["notesRatcheted"], 1,
            "ratchet={ratchet}: 必须有 1 个音符被登记为连击: {result}"
        );
        assert_eq!(
            result["data"]["sources"][0]["ratchetPulses"], pulses,
            "ratchet={ratchet}: 逐轨脉冲数: {result}"
        );
        assert_eq!(
            result["data"]["ratchet"]["notesExpanded"], 1,
            "ratchet={ratchet}: 顶层读数: {result}"
        );
        assert_eq!(
            result["data"]["ratchet"]["pulses"], pulses,
            "ratchet={ratchet}: 顶层脉冲数: {result}"
        );
        // 格点: 第 p 个脉冲起于 p * step 个 tick。
        let step_ticks = duration_ticks / u64::from(ratchet);
        assert!(step_ticks > 0, "夹具的步长必须为正");
        let body = window_rms(&samples, 2, (step_ticks as usize * per_tick) / 2, 1_000);
        assert!(body > 0.1, "ratchet={ratchet}: 脉冲中段必须真的有声音");
        for pulse in 0..pulses {
            let start = pulse * step_ticks as usize * per_tick;
            let head = window_rms(&samples, 2, start, 60);
            assert!(
                head < body * 0.3,
                "ratchet={ratchet}: 脉冲 {pulse} 的起点 {start} 帧能量 {head:.6} \
                 必须显著低于脉冲中段 {body:.6}（起点必须是一次新的起音）"
            );
        }
        // 已经**做到**了 ⇒ 再报"没渲染"就是假话。
        assert!(
            !result["data"]["unsupported"]
                .as_array()
                .expect("unsupported")
                .iter()
                .any(|key| key == "noteRatchet"),
            "ratchet={ratchet}: 连击已展开, 不得再登记 noteRatchet: {result}"
        );
    }
}

/// 余数**不补**：整数除法算出的最后一个脉冲在音符时值**之前**结束，
/// 剩下的那几 tick 必须是静音（只允许抖动底噪）。
///
/// 这条判据钉死"向下取整 + 不补齐"这两个方向：
/// 用**向上**取整（`div_ceil`）或把最后一个脉冲拉到音符末端，尾部都会出现真声音 ⇒ 红。
#[test]
fn the_ratchet_grid_uses_integer_division_and_leaves_the_remainder_silent() {
    let scratch = Scratch::new("ratchet-remainder");
    let per_tick = frames_per_tick();
    // 三组都**有**余数（否则这条判据没有牙齿）。
    for (ratchet, duration_ticks) in [(3u8, 1_000u64), (7, 1_920), (6, 1_000)] {
        let pulses = u64::from(ratchet);
        let step_ticks = duration_ticks / pulses;
        let remainder = duration_ticks - pulses * step_ticks;
        assert!(
            remainder > 0,
            "夹具 ({ratchet}, {duration_ticks}) 必须有余数, 否则判据无意义"
        );
        let (result, samples) = render_note(
            &scratch,
            &format!("rem{ratchet}-{duration_ticks}"),
            Some(ratchet),
            None,
            duration_ticks,
        );
        assert_eq!(result["data"]["sources"][0]["notes"], pulses, "{result}");
        let content_end = (pulses * step_ticks) as usize * per_tick;
        let note_end = duration_ticks as usize * per_tick;
        assert_eq!(
            samples.len() / 2,
            note_end,
            "母带长度 = 摆放时值（余数也占帧, 只是没有声音）"
        );
        // 尾部的余数区间只能是抖动。
        let tail = window_peak(&samples, 2, content_end, note_end - content_end);
        assert!(
            tail <= DITHER_ONLY_PEAK,
            "ratchet={ratchet} / {duration_ticks} tick: 余数区间 \
             [{content_end}, {note_end}) 必须静音（只允许抖动）, 实测峰值 {tail}"
        );
        // 而脉冲中段是真声音 ⇒ 上面那条不是"整段都静音"的巧合。
        let body = window_rms(&samples, 2, (step_ticks as usize * per_tick) / 2, 200);
        assert!(
            body > 0.1,
            "ratchet={ratchet}: 脉冲中段必须真的有声音, 实测 RMS {body:.6}"
        );
    }
}

/// 连击**不引入熵**：同输入两次渲染逐位相同；且 `ratchet = 1` 与**缺省**必须
/// 逐字节等于加这个旋钮之前的默认夹具（钉死的摘要常量）。
#[test]
fn ratchet_renders_are_bit_identical_and_the_default_path_is_unchanged() {
    let scratch = Scratch::new("ratchet-determinism");
    let (first, _) = render_note(&scratch, "det-a", Some(4), None, 1_920);
    let (second, _) = render_note(&scratch, "det-b", Some(4), None, 1_920);
    assert_eq!(
        first["data"]["masterDigest"], second["data"]["masterDigest"],
        "同输入两次渲染的母带摘要必须相同"
    );
    assert_eq!(
        first["data"]["sha256"], second["data"]["sha256"],
        "产物文件必须逐字节相同"
    );
    // 缺省与 = 1 都是"没有连击": 逐位等于钉死的默认夹具常量。
    let (absent, _) = render_note(&scratch, "det-absent", None, None, 1_920);
    let (one, _) = render_note(&scratch, "det-one", Some(1), None, 1_920);
    for (tag, result) in [("absent", &absent), ("one", &one)] {
        assert_eq!(
            result["data"]["masterDigest"], DEFAULT_MASTER_DIGEST,
            "ratchet {tag}: 默认路径必须逐位不变"
        );
        assert_eq!(
            result["data"]["sha256"], DEFAULT_MASTER_SHA256,
            "ratchet {tag}: 默认路径的文件字节必须逐位不变"
        );
        assert_eq!(result["data"]["sources"][0]["notes"], 1, "{result}");
        assert_eq!(result["data"]["ratchet"]["notesExpanded"], 0, "{result}");
        assert_eq!(result["data"]["ratchet"]["pulses"], 0, "{result}");
        assert_eq!(result["data"]["unsupported"], json!([]), "{result}");
    }
    assert_eq!(absent["data"]["sha256"], one["data"]["sha256"]);
    // 连击真的改了母带 —— 否则"展开"是一句空话。
    assert_ne!(
        first["data"]["masterDigest"], absent["data"]["masterDigest"],
        "ratchet=4 与缺省的母带摘要必须不同: {first}"
    );
}

/// 被概率判定判为**不触发**的音符不产生任何脉冲：判定在展开**之前**，
/// 否则"静音的音符"会以连击脉冲的形式留下读数（`ratchetPulses` 不为 0）。
#[test]
fn a_note_that_does_not_trigger_contributes_no_ratchet_pulses() {
    let scratch = Scratch::new("ratchet-silenced");
    let (result, samples) = render_note(&scratch, "silent", Some(4), Some(0.0), 1_920);
    assert_eq!(
        result["data"]["probability"]["notesSilenced"], 1,
        "probability=0.0 的音符必须被判为不触发: {result}"
    );
    assert_eq!(result["data"]["sources"][0]["notes"], 0, "{result}");
    assert_eq!(
        result["data"]["sources"][0]["notesRatcheted"], 0,
        "{result}"
    );
    assert_eq!(result["data"]["sources"][0]["ratchetPulses"], 0, "{result}");
    assert_eq!(result["data"]["ratchet"]["notesExpanded"], 0, "{result}");
    assert_eq!(result["data"]["ratchet"]["pulses"], 0, "{result}");
    assert!(
        peak(&samples) <= DITHER_ONLY_PEAK,
        "被判为不触发的连击音符必须一点声音都不出 (只允许 TPDF 抖动): 实测峰值 {}",
        peak(&samples)
    );
}
