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
//! | 10 | 延迟表来自 `DeviceDefinition::latency_samples`（含旁通不算） | 自建第二延迟来源 |
//! | 11 | `format` 只改容器字节，不改音频负载 | 在音频路径上按格式分叉 |
//! | 12 | 实测数字自洽：frames/blocks/bytes/header/payload/sha256 | 报估算值 |
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
        }
    }
}

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
    let note = id(30);
    let edge = id(40);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "render fixture".to_owned(),
        bpm: spec.bpm,
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
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": "yeban_render_master", "arguments": arguments}
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
        first["data"]["masterDigest"],
        "b242d510d581541732134d3c0e233a11d6045ffb65535675adc73501fb28eefb",
        "母带样本的位级摘要必须逐位固定"
    );
    assert_eq!(
        first["data"]["sha256"], "b9472fcd20086efd4953d457dde26169d1374f592bc90a0cf7b81304e0b69cc4",
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
