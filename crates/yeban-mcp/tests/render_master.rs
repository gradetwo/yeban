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
//! | 9c | 经工具面写的极值起始 tick（`note.startTick` / `placement.startTick` 取到 tick→帧**饱和**的那一个值）⇒ `RENDER_FAILED`（帧数超限），不是整数溢出 panic | 把 `note_frame_span` 的 `start.saturating_add(1)` 写回 `start + 1` |
//! | 9d | 经 `setParam` 的 `DeviceParam` 写的 `output_gain_db` 真的改变 `sha256` / `masterDigest`，写回原值 ⇒ 逐字节相同；白名单外名字与越界下标响亮失败 | 工具面写不进设备参数 / `resolve_device_param` 未被调用 / 渲染器不消费该名字 |
//! | 10 | 延迟表来自 `DeviceDefinition::latency_samples`（含旁通不算） | 自建第二延迟来源 |
//! | 11 | `format` 只改容器字节，不改音频负载 | 在音频路径上按格式分叉 |
//! | 12 | 实测数字自洽：frames/blocks/bytes/header/payload/sha256 | 报估算值 |
//! | 12b | 循环区间短于摆放跨度 ⇒ **真的展开** N 遍；区间终点 `>=` 摆放跨度 ⇒ 与"关循环"逐字节相同 | 只撑长度不复制音符 / 明明不回卷却复制 |
//! | 12c | 循环展开：N=4 的四个分段峰值相等且 > 0，`unsupported` 不含 `clipLoopRepetition` | 用 floor 算遍数 / 尾音跨遍叠加 |
//! | 13a | 音频导出的 `bext` **不含作者字段**（`bwf` **实测键名**里无 `author`/`artist`/`credit`/`copyright`），且 `originatorReference` == 工程 ULID | 往 `bwf` 加一个作者键 |
//! | 13b | 工程作者（`setAuthor` 真的写进去的唯一标记）**不出现在音频产物字节里** | 把 `project.author` 写进 `bext` / `codingHistory` / 任何 chunk |
//! | 13c | 同一标记**不出现在 MIDI 导出字节里**（解回真字节后 `windows(n)` 找） | 把 `project.author` 写进任何 SMF 元事件（轨道名/文本/版权） |
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
use yeban_mcp::domain::render::MAX_RENDER_FRAMES;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::ErrorCode;
use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, ClipContent,
    ClipPlacement, ClipPoolEntry, CurveType, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, ParameterValue, RoutingEdge, RoutingKind, SampleRate, TrackKind, TrackV3,
    YebanProjectV1,
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
///
/// ⭐ **`#268` 重钉（基线三件套）**：`ADR-0011` 把 MCP 的 MIDI 音源从内置正弦换成
/// `yeban-engine` 的 `SynthEngine` ⇒ 默认夹具（含 MIDI）的母带样本与文件字节都变了。
/// 旧值 `b242d510…`（母带）/ `6aa33774…`（文件）是正弦音源的读数；新值由判据自己
/// 报出后照抄。BWF 两条戳、容器布局、抖动口径一位未动（差异只来自音源）。
const DEFAULT_MASTER_DIGEST: &str =
    "17e3db373cd2815ae8fa43e0605654be36d37f47e027501174cd9b3c50834997";

/// 同一份默认夹具的 RIFF **文件字节** SHA-256 —— 钉死的常量。
///
/// **`#214` 重钉（基线三件套）**：旧值 `b9472fcd…` 是在"BWF 的
/// `OriginationDate`/`OriginationTime` 取自注入墙钟（`1_760_000_000_000`
/// = 2025-10-09T08:53:20Z）"时测到的；现在两条戳取自**工程创建时刻**
/// [`FIXTURE_CREATED_AT_MS`]（2023-11-14T22:13:20Z）⇒ 文件字节变了、母带样本
/// 一位没变（`DEFAULT_MASTER_DIGEST` 仍是旧值）。判据逻辑、断言条数、`#[ignore]`
/// 一概未动；新值由判据自己报出后照抄。
///
/// **`#255` 重钉（`bext` 奇长度互通修复）**：该夹具的 `CodingHistory` 是奇数长度,
/// 写入器现在把 NUL 终止符计入 `bext` 负载（chunk 外不再有补位）⇒ 长度字段 +1。
/// 母带样本摘要（`DEFAULT_MASTER_DIGEST`）**逐位未变**；文件总长与补位位置也未变。
const DEFAULT_MASTER_SHA256: &str =
    "837db980db0866f8da53b968a124f1b45494e55b20c3ba86e5697b1e1e99d3b8";

/// 夹具工程的创建时刻 —— BWF 两条 ASCII 戳（`OriginationDate` / `OriginationTime`）
/// 的**唯一**来源（`#214`）。
///
/// 刻意**不等于**注入的墙钟（`1_760_000_000_000`）：若两者相等，"戳来自工程还是
/// 来自墙钟"在读数上不可区分，判据就没有牙。`1_700_000_000_000` = 2023-11-14T22:13:20Z。
const FIXTURE_CREATED_AT_MS: u64 = 1_700_000_000_000;

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
    // `#214`：BWF 的两条 ASCII 戳取自工程创建时刻，不是墙钟 ⇒ 夹具必须显式给它，
    // 否则默认的 `0`（未知）会写成 1970-01-01，与"戳来自工程"这条判据混在一起。
    project.metadata.created_at_unix_ms = FIXTURE_CREATED_AT_MS;
    project.metadata.modified_at_unix_ms = FIXTURE_CREATED_AT_MS;
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

/// 一个**空**的分发器（不注入工程）—— 供 `#225` 复跑 `#224` 那条
/// "`open_project create` ⇒ `propose_section` ⇒ `merge` ⇒ `render_master`"的调用序列。
fn fresh_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

/// ⭐ `#225` 判据夹具：**两条**源轨（`Lead` 与 `Pad`）汇入 Master，音量不同。
///
/// 用途：证明 `sources[].peak` / `loudestSource` 真的按**每一条源**量、而不是
/// 单源工程里"反正只有它"。两条轨的音符起点/时值完全相同，只有音高与轨音量不同。
fn two_track_clip_project(lead_volume_db: f32, pad_volume_db: f32) -> YebanProjectV1 {
    let mut project = project(&Spec {
        volume_db: lead_volume_db,
        ..Spec::default()
    });
    let pad = id(3);
    let clip = id(11);
    let placement = id(21);
    let note = id(31);
    let mut track = TrackV3 {
        id: pad,
        name: "Pad".to_owned(),
        kind: TrackKind::Midi,
        volume_db: pad_volume_db,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: Spec::default().end_tick,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(note, MidiNote::new(note, 0, 57, Spec::default().end_tick));
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "PadClip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.tracks.insert(pad, track);
    project.routing_graph.nodes.push(pad);
    let edge = id(41);
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: pad,
            destination_node: id(1),
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project
        .validate()
        .unwrap_or_else(|error| panic!("两轨夹具必须合法: {error}"));
    project
}

/// 磁盘上这份文件的 SHA-256（**从字节读**，不是回抄响应）。
fn file_sha256(path: &Path) -> String {
    let bytes = fs::read(path).expect("读产物");
    yeban_model::AssetHash::of_bytes(&bytes).as_str().to_owned()
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
// ⭐ 判据 2c（`#252`）：显式帧数 / 时长
// ---------------------------------------------------------------------------

/// ⭐ `#252` 判据 1（**缺省不变**）：先不给新实参（按工程末端推导 = 48 000 帧），
/// 再显式给 `frames: 48000` ⇒ 整文件 `sha256` 与 `masterDigest` **逐位相同**，且
/// 两者都等于**加这个实参之前**钉死的常量。
///
/// 为什么这是"零默认改动"的硬证据：常量 [`DEFAULT_MASTER_DIGEST`] /
/// [`DEFAULT_MASTER_SHA256`] 是修改前就存在的；本判据同时证明"缺省路径没变"与
/// "显式帧数只改长度、不改算出来的样本"。
#[test]
fn the_default_rendering_path_stays_bit_identical_with_an_explicit_frame_count() {
    let scratch = Scratch::new("explicit-frames-default");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let out = scratch.join("master.wav");
    let legacy = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(legacy["status"], "success", "{legacy}");
    assert_eq!(legacy["data"]["frames"], 48_000);
    assert_eq!(legacy["data"]["request"]["frames"], Value::Null);
    assert_eq!(legacy["data"]["request"]["framesSource"], "projectEnd");
    assert_eq!(legacy["data"]["masterDigest"], DEFAULT_MASTER_DIGEST);
    assert_eq!(legacy["data"]["sha256"], DEFAULT_MASTER_SHA256);

    // 同一个输出路径 + 显式帧数 = 工程末端推导出的同一个数。
    let explicit = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "frames": 48_000,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(explicit["status"], "success", "{explicit}");
    assert_eq!(explicit["data"]["frames"], 48_000);
    assert_eq!(explicit["data"]["request"]["frames"], 48_000);
    assert_eq!(explicit["data"]["request"]["framesSource"], "frames");
    assert_eq!(
        explicit["data"]["sha256"], legacy["data"]["sha256"],
        "显式帧数等于工程末端时, 整文件必须逐字节相同"
    );
    assert_eq!(
        explicit["data"]["masterDigest"], legacy["data"]["masterDigest"],
        "显式帧数等于工程末端时, 母带样本必须逐位相同"
    );
    assert_eq!(entries(&scratch.dir), vec!["master.wav"]);
}

/// ⭐ `#252` 判据 3（**帧数真的生效**）：不同 `frames` ⇒ 输出长度不同、摘要不同；
/// `durationSeconds` 与 `frames` 两条入口在同一个采样率下**等价**（同一份摘要）。
#[test]
fn an_explicit_frame_count_really_sets_the_output_length() {
    let scratch = Scratch::new("explicit-frames-effect");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let short_path = scratch.join("short.wav");
    let short = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "frames": 24_000,
            "path": short_path.display().to_string(),
        }),
    );
    assert_eq!(short["status"], "success", "{short}");
    assert_eq!(short["data"]["frames"], 24_000, "显式帧数必须真的生效");
    assert_eq!(short["data"]["durationSeconds"], 0.5);
    let samples = verify_file_shape(&short, &short_path);
    assert_eq!(samples.len(), 24_000 * 2, "负载必须缩短到 24 000 帧");

    // 秒数入口：0.5 s × 48 kHz = 24 000 帧 ⇒ 与上面**同一份**母带样本。
    let seconds_path = scratch.join("seconds.wav");
    let by_seconds = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "durationSeconds": 0.5,
            "path": seconds_path.display().to_string(),
        }),
    );
    assert_eq!(by_seconds["status"], "success", "{by_seconds}");
    assert_eq!(by_seconds["data"]["frames"], 24_000);
    assert_eq!(by_seconds["data"]["request"]["durationSeconds"], 0.5);
    assert_eq!(
        by_seconds["data"]["request"]["framesSource"],
        "durationSeconds"
    );
    assert_eq!(
        by_seconds["data"]["masterDigest"], short["data"]["masterDigest"],
        "两条入口指向同一个帧数时必须产出同一份母带样本"
    );
    assert_eq!(by_seconds["data"]["sha256"], short["data"]["sha256"]);

    // 与缺省（48 000 帧）比：长度、字节数、摘要都必须不同。
    let default = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "dryRun": true}),
    );
    assert_eq!(default["data"]["preview"]["wouldWrite"]["frames"], 48_000);
    assert_ne!(default["data"]["frames"], short["data"]["frames"]);
    assert_ne!(default["data"]["bytes"], short["data"]["bytes"]);
    assert_ne!(
        default["data"]["masterDigest"], short["data"]["masterDigest"],
        "长度不同 ⇒ 样本流不同 ⇒ 摘要必须不同"
    );
}

/// ⭐ `#252` 判据 4（**非法输入被拒**）：0 帧 / 超上限 / 两个都给且冲突 ⇒ 显式错误，
/// 且**一个文件都不写**（连临时文件都没有）。`dryRun` 也走同一条校验。
#[test]
fn illegal_explicit_frames_are_refused_without_writing_a_file() {
    let scratch = Scratch::new("explicit-frames-illegal");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let out = scratch.join("master.wav");
    let path = out.display().to_string();

    let zero = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "frames": 0, "path": path}),
    );
    assert_domain_error(&zero, "INVALID_PARAMETER_RANGE", "frames = 0");
    assert!(
        format!("{zero}").contains("`frames` 必须是 > 0 的整数"),
        "报文必须点名 `frames`: {zero}"
    );

    let over = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "frames": MAX_RENDER_FRAMES + 1,
            "path": path,
        }),
    );
    assert_domain_error(&over, "RENDER_FAILED", "frames 超上限");
    assert_eq!(over["error"]["data"]["maxFrames"], 172_800_000);

    let conflict = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "frames": 48_000,
            "durationSeconds": 0.5,
            "path": path,
        }),
    );
    assert_domain_error(&conflict, "INVALID_PARAMETER_RANGE", "两个都给且冲突");
    assert_eq!(conflict["error"]["data"]["durationFrames"], 24_000.0);
    assert_eq!(conflict["error"]["data"]["frames"], 48_000);

    // `dryRun` 也拒绝（同一条 `validate`），且同样不落盘。
    let dry = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "frames": 0,
            "dryRun": true,
        }),
    );
    assert_domain_error(&dry, "INVALID_PARAMETER_RANGE", "dryRun + frames = 0");
    assert!(
        entries(&scratch.dir).is_empty(),
        "被拒绝的渲染不得留下任何文件: {:?}",
        entries(&scratch.dir)
    );
}

// ---------------------------------------------------------------------------
// 判据 2b：墙钟只进报文（`renderedAtUnixMs`），⛔ 不进文件字节（#214）
// ---------------------------------------------------------------------------

/// `#214` 的**结论判据**：换注入时钟（`Domain::set_now_ms`；夹具固定注入
/// `1_760_000_000_000`）⇒ 报文 `renderedAtUnixMs` 跟着变，**文件字节一位不变**。
///
/// 修前行为（`#209` 实测）：BWF 的 `OriginationDate` / `OriginationTime` 取自这个
/// 墙钟 ⇒ 同一命令跑两次、两次落在不同的秒 ⇒ 整文件 `sha256` 不同（`masterDigest`
/// 却相同，因为 PCM 没变）。修后两条戳取自工程创建时刻
/// （[`FIXTURE_CREATED_AT_MS`]），墙钟只进报文。
///
/// 为什么单列：`renderedAtUnixMs` **不进文件字节** ⇒ 把它写死成 `0` 全仓没有一条
/// 判据会红；而 BWF 的两条戳现在由工程夹具 + `DEFAULT_MASTER_SHA256` 共同钉住。
#[test]
fn the_wall_clock_reaches_the_report_but_not_the_file() {
    const INJECTED_MS: u64 = 1_760_000_000_000; // 2025-10-09T08:53:20Z
    let scratch = Scratch::new("injected-clock");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    // **同一个**输出路径跑两轮：`data.request.path` 属于报文，换路径会让"只差墙钟"
    // 这条枚举变成"差在路径上"。
    let out = scratch.join("clock.wav");
    let arguments = json!({
        "format": "wav",
        "sampleRate": 48000,
        "path": out.display().to_string(),
    });

    let first = call(&mut dispatcher, &auth, arguments.clone());
    assert_eq!(first["status"], "success", "{first}");
    assert_eq!(
        first["data"]["renderedAtUnixMs"], INJECTED_MS,
        "报文必须带上注入时钟: {first}"
    );
    // 两条 ASCII 戳来自**工程**（`1_700_000_000_000` = 2023-11-14T22:13:20Z），
    // 与注入的墙钟（2025-10-09T08:53:20Z）刻意不同 ⇒ 来源可区分。
    assert_eq!(first["data"]["bwf"]["originationDate"], "2023-11-14");
    assert_eq!(first["data"]["bwf"]["originationTime"], "22:13:20");
    let first_bytes = fs::read(&out).expect("第一轮产物");

    // 换一个注入时钟（+1 天 +1 秒 ⇒ 修前这会让两条 BWF 戳都变）。
    const SHIFTED_MS: u64 = INJECTED_MS + 86_401_000; // 2025-10-10T08:53:21Z
    dispatcher.domain_mut().set_now_ms(SHIFTED_MS);
    let second = call(&mut dispatcher, &auth, arguments);
    assert_eq!(second["status"], "success", "{second}");
    assert_eq!(second["data"]["renderedAtUnixMs"], SHIFTED_MS);
    assert_eq!(second["data"]["bwf"]["originationDate"], "2023-11-14");
    assert_eq!(second["data"]["bwf"]["originationTime"], "22:13:20");

    /// 两份 JSON 对象里**取值不同**的键（排序后）。
    fn changed_keys(left: &Value, right: &Value) -> Vec<String> {
        let left = left.as_object().expect("左值是对象");
        let right = right.as_object().expect("右值是对象");
        let mut keys: Vec<String> = left
            .keys()
            .chain(right.keys())
            .filter(|key| left.get(*key) != right.get(*key))
            .cloned()
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    // 机械枚举: `data` 里**恰好**一个顶层键随注入时钟变 —— `renderedAtUnixMs`。
    // 修前 `sha256` 与 `bwf` 也会出现在这里（`#214` 的缺陷本体）。
    let top_changed = changed_keys(&first["data"], &second["data"]);
    assert_eq!(
        top_changed,
        vec!["renderedAtUnixMs".to_owned()],
        "换注入时钟只许改报文里的墙钟读数, 实际 {top_changed:?}"
    );
    // BWF 的两条 ASCII 戳**不**随墙钟变。
    assert_eq!(
        changed_keys(&first["data"]["bwf"], &second["data"]["bwf"]),
        Vec::<String>::new(),
        "BWF 的两条戳不许跟着墙钟走"
    );
    // 墙钟**不许**碰音频，也不许碰文件字节：母带摘要、帧数、容器长度、整文件都相同。
    assert_eq!(
        second["data"]["masterDigest"],
        first["data"]["masterDigest"]
    );
    assert_eq!(second["data"]["frames"], first["data"]["frames"]);
    assert_eq!(second["data"]["bytes"], first["data"]["bytes"]);
    assert_eq!(second["data"]["sha256"], first["data"]["sha256"]);
    let second_bytes = fs::read(&out).expect("第二轮产物");
    assert_eq!(
        second_bytes, first_bytes,
        "换墙钟不许改动产物的任何一个字节（#214 的缺陷就是这一条为假）"
    );
}

// ---------------------------------------------------------------------------
// 判据 2c：分离读数 —— chunk 表（#214 判据 1 / 2 / 3）
// ---------------------------------------------------------------------------

/// 两次渲染（墙钟相差一天一秒）的 **chunk 表**逐项相同、`data` 载荷逐字节相同、
/// 整文件 `sha256` 相同。
///
/// 为什么先分离再修：`#209` 的读数只有"整文件 `sha256` 不同 + `masterDigest` 相同"，
/// 这留下两种可能 —— ① PCM 真的变了（抖动 / 未初始化缓冲），② 只有封装里的时间戳
/// 变了。逐 chunk 的读数把两者分开。**修前实测**（同一条命令、两次进程）：
/// `fmt ` 16 B 相同；`data` 540000 B 相同（`sha256 = 73092e20ada70c6f…`）；
/// `bext` 641 B **不同**，且只有负载偏移 337（= `OriginationTime` 的秒个位）
/// 这 1 个字节在动（`'4'` → `'7'`）。⇒ PCM 没有差异，缺陷只在封装。
///
/// 本判据钉住修后的读数：**三个 chunk 全部逐项相同**。
#[test]
fn two_renders_under_different_wall_clocks_are_byte_identical_chunk_by_chunk() {
    const INJECTED_MS: u64 = 1_760_000_000_000; // 2025-10-09T08:53:20Z
    let scratch = Scratch::new("chunk-table");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let first_path = scratch.join("a.wav");
    let second_path = scratch.join("b.wav");
    let first = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": first_path.display().to_string()}),
    );
    assert_eq!(first["status"], "success", "{first}");
    // 墙钟跨秒（修前 `bext` 的秒个位必变）。
    dispatcher.domain_mut().set_now_ms(INJECTED_MS + 86_401_000);
    let second = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": second_path.display().to_string()}),
    );
    assert_eq!(second["status"], "success", "{second}");

    /// `(fourcc, 负载长度, 负载 SHA-256)`，按文件顺序。
    fn chunk_table(path: &Path) -> Vec<(String, usize, String)> {
        let bytes = fs::read(path).expect("读产物");
        let parsed = parse_container(&bytes).expect("产物必须是合法容器");
        parsed
            .chunks
            .iter()
            .map(|chunk| {
                let payload =
                    &bytes[chunk.payload_offset..chunk.payload_offset + chunk.payload_len];
                (
                    String::from_utf8_lossy(&chunk.fourcc).into_owned(),
                    chunk.payload_len,
                    yeban_model::AssetHash::of_bytes(payload)
                        .as_str()
                        .to_owned(),
                )
            })
            .collect()
    }

    let a = chunk_table(&first_path);
    let b = chunk_table(&second_path);
    // chunk 名字与长度：RIFF + `fmt ` + `bext` + `data`。
    assert_eq!(
        a.iter()
            .map(|(name, len, _)| (name.as_str(), *len))
            .collect::<Vec<_>>(),
        vec![
            ("fmt ", 16),
            // 39 字节 CodingHistory 是奇数 ⇒ 写入器补 1 字节 NUL 终止符并计入负载
            // （chunk 外不再有补位; 文件总长与补位位置都不变）。
            ("bext", BEXT_FIXED_LEN + 40),
            // 夹具：1 秒 @ 48 kHz × 2 声道 × 3 字节 = 288000。
            ("data", 288_000)
        ],
        "chunk 表: {a:?}"
    );
    // 整表逐项相同（名字、长度、负载摘要）⇒ `data` 相同、`bext` 也相同。
    assert_eq!(a, b, "两次渲染（墙钟不同）的 chunk 表必须逐项相同");
    assert_eq!(a[2].2, b[2].2, "`data` 载荷必须逐字节相同");
    assert_eq!(first["data"]["sha256"], second["data"]["sha256"]);
    assert_eq!(
        first["data"]["masterDigest"],
        second["data"]["masterDigest"]
    );
    // 而墙钟确实不同 —— 否则这条判据可能只是因为两次落在同一秒而假绿。
    assert_ne!(
        first["data"]["renderedAtUnixMs"], second["data"]["renderedAtUnixMs"],
        "两次渲染的注入墙钟必须不同, 判据才有牙"
    );
}

// ---------------------------------------------------------------------------
// 判据 2d：判据不是恒真 —— 改一个音符，文件必须变
// ---------------------------------------------------------------------------

/// 同一份工程、只把**一个音符的力度**从 100 改成 101 ⇒ 文件 `sha256` 与母带
/// `masterDigest` **都必须变**，而 BWF 元数据不变（差异在 PCM 里，不在戳里）。
///
/// 为什么需要它：判据 2 / 2b / 2c 断言的是"两次渲染相同"。若文件写入被换成写常量
/// （或渲染被短路成静音），那些判据**全部仍然绿**。本判据是它们的反向对照。
#[test]
fn a_one_note_change_moves_the_file_hash() {
    let scratch = Scratch::new("one-note-change");
    let (mut base, auth) = dispatcher_with(&project(&Spec::default()), &scratch.join("base.yeban"));
    let base_path = scratch.join("base.wav");
    let plain = call(
        &mut base,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": base_path.display().to_string()}),
    );
    assert_eq!(plain["status"], "success", "{plain}");

    let varied_spec = Spec {
        velocity: 101,
        ..Spec::default()
    };
    let (mut varied, varied_auth) =
        dispatcher_with(&project(&varied_spec), &scratch.join("varied.yeban"));
    let varied_path = scratch.join("varied.wav");
    let changed = call(
        &mut varied,
        &varied_auth,
        json!({"format": "wav", "sampleRate": 48000, "path": varied_path.display().to_string()}),
    );
    assert_eq!(changed["status"], "success", "{changed}");

    assert_ne!(
        plain["data"]["sha256"], changed["data"]["sha256"],
        "改一个音符力度必须改变整文件 sha256"
    );
    assert_ne!(
        plain["data"]["masterDigest"], changed["data"]["masterDigest"],
        "改一个音符力度必须改变母带样本摘要"
    );
    assert_eq!(
        plain["data"]["bwf"], changed["data"]["bwf"],
        "差异必须在 PCM 里, 不在 BWF 元数据里"
    );
    assert_eq!(
        fs::read(&base_path).expect("base").len(),
        fs::read(&varied_path).expect("varied").len(),
        "改力度只改样本值, 不改文件长度"
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

/// 判据 3b-iv：响度容差是**闭**区间 —— `|delta|` 恰好等于 `toleranceLu` 时必须判
/// `pass`。
///
/// 既有判据把目标摆在"等于实测"（差值 ~1e-4）与"实测 − 6 LU"（差值 6）两处，
/// 离 0.5 都很远 ⇒ 把 `<=` 写成 `<` 一样全绿。本条把目标摆在**恰好差一个容差**：
/// `measured` 与 `measured - 0.5` 同量级（Sterbenz: `y/2 ≤ x ≤ 2y` ⇒ f32 减法精确）
/// ⇒ `deltaLu` 逐位等于 `0.5`，本判据测的就是那条边界。
#[test]
fn the_loudness_tolerance_boundary_is_inclusive() {
    let scratch = Scratch::new("loudness-boundary");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));

    let probe = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": scratch.text("probe.wav")}),
    );
    assert_eq!(probe["status"], "success", "{probe}");
    let measured = probe["data"]["loudness"]["measuredIntegratedLufs"]
        .as_f64()
        .expect("实测读数");

    let at = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "targetLufs": measured - 0.5,
            "path": scratch.text("at.wav"),
        }),
    );
    assert_eq!(at["status"], "success", "恰好一个容差必须判 pass: {at}");
    let loudness = &at["data"]["loudness"];
    assert_eq!(loudness["toleranceLu"], 0.5, "{at}");
    assert_eq!(
        loudness["deltaLu"], 0.5,
        "差值必须逐位等于容差（否则本条测的不是边界）: {at}"
    );
    assert_eq!(loudness["verdict"], "pass", "{at}");

    // 反向: 一格之外必须仍然 fail（闭区间不等于"永不失败"）。
    let over = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "targetLufs": measured - 0.6,
            "path": scratch.text("over.wav"),
        }),
    );
    assert_eq!(over["error"]["code"], "RENDER_FAILED", "{over}");
    assert_eq!(over["error"]["data"]["reason"], "loudnessTargetMissed");
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
    // ⭐ `#268`：音源换成引擎音色之后母带的实测积分响度从 ≈ −3.0 LUFS 变成
    // ≈ −3.87 LUFS ⇒ "恰好达标"的目标值跟着改到 −4.0（|−3.87 − (−4.0)| < 0.5）。
    for (index, target) in [None, Some(-4.0f64), Some(-40.0)].into_iter().enumerate() {
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
        plain["data"]["masterDigest"], DEFAULT_MASTER_DIGEST,
        "默认路径的母带样本摘要必须与判据 2 钉死的那一个逐位相同"
    );
    assert_eq!(
        plain["data"]["sha256"], DEFAULT_MASTER_SHA256,
        "默认路径的文件摘要必须与判据 2 钉死的那一个逐位相同"
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
        json!({"format": "wav", "sampleRate": 48000, "targetLufs": -4.0, "dryRun": true}),
    );
    assert_eq!(preview["status"], "success", "{preview}");
    let preview_loudness = preview["data"]["preview"]["loudness"].clone();
    let real = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "targetLufs": -4.0,
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

// ⚠ **本平台不可构造"只读目录"**（父目录权限位是 POSIX 语义）⇒ 本判据在
// Windows 侧**未验证**：目录的 `read-only` 属性**不阻止**在其中创建/写文件。
// ⛔ 这里**没有** `#[cfg(not(unix))]` 平行臂 —— 造不出来，硬写只能得到一条
// "在 Windows 上永远为真"的**装饰判据**（看起来测过了，实际零验证力）。
// 要补真覆盖必须换一个**失败发生在旧文件被碰之前**的失败源（见 `store.rs` 的
// `write_project_atomic`：唯一写目标是临时文件，旧文件只在最后一步 `rename`
// 才被替换）—— 那是**新增判据**，不是平行臂。
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
    assert_eq!(result["data"]["sources"][0]["kind"], "engine-instrument");
    assert_eq!(result["data"]["sources"][0]["audible"], true);
    assert_eq!(result["data"]["sources"][0]["notes"], 1);

    // ⭐ `#251`（S0）：MIDI 音源必须**显式**说明是内置正弦振荡器，⛔ 不是引擎音色。
    let midi_source = &result["data"]["midiSource"];
    assert_eq!(midi_source["kind"], "engine-instrument", "{midi_source}");
    assert_eq!(midi_source["engineTimbre"], true, "{midi_source}");
    assert_eq!(midi_source["adr"], "ADR-0011", "{midi_source}");
    let note = midi_source["note"]
        .as_str()
        .unwrap_or_else(|| panic!("`midiSource.note` 必须是字符串: {midi_source}"));
    assert!(
        note.contains("同一个音色") && note.contains("不再使用内置正弦振荡器"),
        "文案必须逐字说明音源与边界: {note}"
    );
    assert!(
        note.contains("SynthEngine"),
        "文案必须点名引擎的合成器: {note}"
    );

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
// 判据 9d：经工具面写的**设备参数**真的改变母带（`#265`）
// ---------------------------------------------------------------------------

/// 一台**内部效果器**：`params[0] = output_gain_db`（渲染器消费），
/// `params[1] = reverb_mix`（渲染器**刻意不认**，用来钉 `paramNotSupported`）。
///
/// ⚠ 名字都取自 `yeban_render/src/inserts.rs` 的白名单/黑名单口径，不是随便编的。
fn strip_effect(seed: u32) -> DeviceDefinition {
    DeviceDefinition {
        id: id(seed),
        name: "Strip".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: vec![
            ParameterValue {
                name: "output_gain_db".to_owned(),
                value: 0.0,
                unit: Some("dB".to_owned()),
            },
            ParameterValue {
                name: "reverb_mix".to_owned(),
                value: 0.5,
                unit: None,
            },
        ],
        latency_samples: 0,
    }
}

/// ⭐ `#265`：`ops[].kind == "setParam"` + `lane:"DeviceParam"` 走完
/// `tools/call` → 提案 → 合并 → 渲染，且：
///
/// 1. `query_project` 读回的参数值**真的变了**（`0.0` → `-12.0` → `0.0`）；
/// 2. 写进去之后母带的 `sha256` / `masterDigest` **必须变化**（不是"写进去但不响"）；
/// 3. 写回原值 ⇒ 产物与基线**逐字节相同**；
/// 4. 白名单外的名字 / 越界的两个下标**响亮失败**，`data.reason` 逐字来自 render
///    （`paramNotSupported` / `slotOutOfRange` / `paramIndexOutOfRange`）。
///
/// 可被什么注入破坏：`compile` 不调用 `resolve_device_param`（不支持的名字也接受）、
/// `Op::SetParam` 的 `target` 没有落进模型（`query_project` 读数不变）、或渲染器不消费
/// `output_gain_db`（两个 `sha256` 相等 ⇒ 本判据当场红）。
#[test]
fn a_device_parameter_written_through_the_tool_face_changes_the_master() {
    let scratch = Scratch::new("device-param-tool-face");
    let spec = Spec {
        devices: vec![strip_effect(50)],
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&spec), &scratch.join("demo.yeban"));
    let lead = id(2).to_canonical_string();
    let clip = id(10).to_canonical_string();

    // ---- 基线：参数 0.0 dB（链为空 ⇒ 不做任何插入处理）----
    let plain_path = scratch.join("plain-device.wav");
    let plain = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": plain_path.display().to_string()}),
    );
    assert_eq!(plain["status"], "success", "{plain}");
    let plain_sha = plain["data"]["sha256"].as_str().expect("sha256").to_owned();
    let plain_digest = plain["data"]["masterDigest"]
        .as_str()
        .expect("masterDigest")
        .to_owned();
    let plain_peak = plain["data"]["peak"]["before"].as_f64().expect("peak");
    let before = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({"fields": ["tracks"]}),
    );
    assert_eq!(before["status"], "success", "{before}");
    assert_eq!(
        before["data"]["project"]["tracks"][&lead]["devices"][0]["params"][0]["value"],
        json!(0.0),
        "基线参数必须是 0.0 dB: {before}"
    );

    // ---- 白名单外 / 越界：三种响亮失败，reason 逐字来自 render ----
    let unsupported = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead, "clipId": clip,
            "ops": [{"kind": "setParam", "lane": "DeviceParam",
                     "slotIndex": 0, "paramIndex": 1, "value": 0.2}]
        }),
    );
    assert_domain_error(&unsupported, "INVALID_PARAMETER_RANGE", "白名单外参数名");
    assert_eq!(
        unsupported["error"]["data"]["reason"], "paramNotSupported",
        "{unsupported}"
    );
    assert_eq!(
        unsupported["error"]["data"]["paramName"], "reverb_mix",
        "{unsupported}"
    );
    println!("### #265 paramNotSupported => {unsupported}");

    for (slot, param, reason) in [
        (99_u64, 0_u64, "slotOutOfRange"),
        (0, 99, "paramIndexOutOfRange"),
    ] {
        let out = call_tool(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": lead, "clipId": clip,
                "ops": [{"kind": "setParam", "lane": "DeviceParam",
                         "slotIndex": slot, "paramIndex": param, "value": 0.2}]
            }),
        );
        assert_domain_error(&out, "OUT_OF_RANGE", "设备参数下标越界");
        assert_eq!(out["error"]["data"]["reason"], reason, "{out}");
        println!("### #265 {reason} => {out}");
    }

    // ---- 真的写进去：提案的撤销载荷来自当前文档，合并后 query_project 读得回 ----
    let written = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead, "clipId": clip, "includeOps": true,
            "ops": [{"kind": "setParam", "lane": "DeviceParam",
                     "slotIndex": 0, "paramIndex": 0, "value": -12.0}]
        }),
    );
    assert_eq!(written["status"], "success", "{written}");
    assert_eq!(
        written["data"]["proposal"]["ops"][0]["op"]["SetParam"]["old_val"],
        json!(0.0),
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
        json!({"proposalId": proposal_id, "commitMessage": "设备参数 -12 dB"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");

    let after = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({"fields": ["tracks"]}),
    );
    println!(
        "### #265 query_project tracks[lead].devices[0].params[0] = {}",
        after["data"]["project"]["tracks"][&lead]["devices"][0]["params"][0]
    );
    assert_eq!(
        after["data"]["project"]["tracks"][&lead]["devices"][0]["params"][0]["value"],
        json!(-12.0),
        "合并后 query_project 必须读回 -12.0: {after}"
    );

    // ---- 它真的响吗：导出哈希与母带摘要必须变化 ----
    let changed_path = scratch.join("changed-device.wav");
    let changed = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": changed_path.display().to_string()}),
    );
    assert_eq!(changed["status"], "success", "{changed}");
    let changed_sha = changed["data"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_owned();
    let changed_digest = changed["data"]["masterDigest"]
        .as_str()
        .expect("masterDigest")
        .to_owned();
    let changed_peak = changed["data"]["peak"]["before"].as_f64().expect("peak");
    println!(
        "### #265 device param: plain sha256={plain_sha} masterDigest={plain_digest} peak={plain_peak}"
    );
    println!(
        "### #265 device param: -12dB sha256={changed_sha} masterDigest={changed_digest} peak={changed_peak}"
    );
    assert_ne!(plain_sha, changed_sha, "写设备参数后导出 sha256 必须变化");
    assert_ne!(
        plain_digest, changed_digest,
        "写设备参数后 masterDigest 必须变化"
    );
    assert!(
        changed_peak < plain_peak,
        "output_gain_db=-12 后峰值必须更低: plain={plain_peak} changed={changed_peak}"
    );

    // ---- 写回 0.0 ⇒ 产物与基线逐字节相同（链重新变为空）----
    let restored = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead, "clipId": clip,
            "ops": [{"kind": "setParam", "lane": "DeviceParam",
                     "slotIndex": 0, "paramIndex": 0, "value": 0.0}]
        }),
    );
    assert_eq!(restored["status"], "success", "{restored}");
    let proposal_id = restored["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "设备参数回 0 dB"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    let restored_path = scratch.join("restored-device.wav");
    let again = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": restored_path.display().to_string()}),
    );
    assert_eq!(again["status"], "success", "{again}");
    println!(
        "### #265 device param: restored sha256={} masterDigest={}",
        again["data"]["sha256"].as_str().unwrap_or("?"),
        again["data"]["masterDigest"].as_str().unwrap_or("?")
    );
    assert_eq!(
        fs::read(&restored_path).expect("产物"),
        fs::read(&plain_path).expect("基线产物"),
        "写回 0.0 dB 之后, 母带产物必须与写之前逐字节相同"
    );
    assert_eq!(again["data"]["sha256"], plain["data"]["sha256"]);
    assert_eq!(again["data"]["masterDigest"], plain["data"]["masterDigest"]);
}

// ---------------------------------------------------------------------------
// 判据 9c：经工具面写进来的**极值 tick** 只能换来 RENDER_FAILED, 不能打死进程
// ---------------------------------------------------------------------------

/// `ticks_to_frames` 在 960 PPQ / 120 BPM / 48 kHz 下是 `frames == tick * 25`；
/// 它的 `frames >= u64::MAX as f64` 分支返回 `u64::MAX`，**第一个**命中的 tick 就是
/// 这个常量（`note_frame_span` 的单元判据 7b 机械复核它）。
const SATURATING_START_TICK: u64 = 737_869_762_948_382_017;

/// 工具面能把 `note.startTick` / `placement.startTick` 写到让 tick→帧**饱和**的取值
/// （实数轴上界由 `u64` 决定，工具面没有更小的上界）。这条判据钉住：那种工程再渲染
/// 一次，得到的是契约内的 `RENDER_FAILED`（帧数超上限），**不是**整数溢出 panic。
///
/// 两个极值都测：`SATURATING_START_TICK`（`ticks_to_frames` 的饱和分支**第一个**
/// 触发点，钉住 `start.saturating_add(1)`）与 `u64::MAX` 本身（把 `clamped` 顶到
/// `u64::MAX`，钉住终点那一次 `clamped.saturating_add(duration_ticks.max(1))` ——
/// 元组里两次加法是**两条**算术，`render_math` 的单元判据 7b / 7c 分别覆盖它们）。
///
/// 可以变红的方式：把 `render_math::note_frame_span` 的任一次 `saturating_add`
/// 写回 `+` ⇒ 本判据在 `yeban_render_master` 那一步
/// `attempt to add with overflow`。这条判据走的是**真 `tools/call` 管线 + 真渲染**，
/// 因此它同时钉住"工具面可达"。
#[test]
fn a_saturated_start_tick_from_the_tool_face_is_a_contract_error_not_a_panic() {
    for (label, extreme) in [("saturating", SATURATING_START_TICK), ("maximal", u64::MAX)] {
        // 两条可达路径各自一条: 音符自己的起点, 与摆放的起点。
        for (shape, extra) in [
            (
                "note",
                json!({"ops": [{
                    "kind": "add",
                    "note": {
                        "startTick": extreme,
                        "pitch": 72,
                        "durationTicks": 480
                    }
                }]}),
            ),
            (
                "placement",
                json!({
                    "ops": [],
                    "placement": {"startTick": extreme, "durationTicks": 480}
                }),
            ),
        ] {
            let tag = format!("{label}-{shape}");
            let scratch = Scratch::new(&tag);
            let spec = Spec::default();
            let (mut dispatcher, auth) =
                dispatcher_with(&project(&spec), &scratch.join("demo.yeban"));
            let lead = id(2).to_canonical_string();
            let clip = id(10).to_canonical_string();

            // 写入提案并合并 —— 这一半此前就已经通过（规划期只看 `u64` 的非负性）。
            let mut arguments = json!({"trackId": lead, "clipId": clip});
            for (key, value) in extra.as_object().expect("对象") {
                arguments[key] = value.clone();
            }
            let written = call_tool(&mut dispatcher, &auth, "yeban_edit_notes", arguments);
            assert_eq!(written["status"], "success", "{tag}: {written}");
            let proposal_id = written["data"]["proposal"]["proposalId"]
                .as_str()
                .expect("proposalId")
                .to_owned();
            let merged = call_tool(
                &mut dispatcher,
                &auth,
                "yeban_merge_proposal",
                json!({"proposalId": proposal_id, "commitMessage": "saturating"}),
            );
            assert_eq!(merged["status"], "success", "{tag}: {merged}");

            // 断言极值**真的**落进了文档（否则这条判据测不到那条算术）。
            let view = call_tool(&mut dispatcher, &auth, "yeban_query_project", json!({}));
            assert_eq!(view["status"], "success", "{tag}: {view}");
            let project_json = &view["data"]["project"];
            let at_extreme = |value: &Value| value.as_u64() == Some(extreme);
            match shape {
                "note" => {
                    let notes = &project_json["clip_pool"][&clip]["content"]["Midi"]["notes"];
                    assert!(
                        notes
                            .as_object()
                            .expect("notes")
                            .values()
                            .any(|note| at_extreme(&note["start_tick"])),
                        "{tag}: 极值起点必须真的写进文档: {notes}"
                    );
                }
                _ => {
                    let clips = &project_json["tracks"][&lead]["clips"];
                    assert!(
                        clips
                            .as_object()
                            .expect("clips")
                            .values()
                            .any(|placement| at_extreme(&placement["start_tick"])),
                        "{tag}: 极值摆放起点必须真的写进文档: {clips}"
                    );
                }
            }

            // 真渲染: 必须是契约内的 RENDER_FAILED（帧数超出 1 小时上限），不是 panic。
            let out = scratch.join(&format!("{tag}.wav"));
            let rendered = call(
                &mut dispatcher,
                &auth,
                json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
            );
            assert_domain_error(&rendered, "RENDER_FAILED", &tag);
            assert!(
                rendered["error"]["data"]["frames"]
                    .as_u64()
                    .is_some_and(|frames| frames > 172_800_000),
                "{tag}: 必须报出真实的超限帧数: {rendered}"
            );
            assert!(
                !out.exists(),
                "{tag}: 被拒绝的渲染不得留下任何文件: {out:?}"
            );
        }
    }
}

/// 判据 9d：单次渲染的帧数上限是**发布的那一条**（1 小时 @ 48 kHz），并且真的以
/// `maxFrames` 出现在拒绝报文里。
///
/// 既有判据（9c）断的是 `frames > 172_800_000`（**字面量**，只比大小）
/// ⇒ 把常量改成两小时（`48_000 * 7_200`）一样全绿：上限**本身**从来没有进过判据。
#[test]
fn the_render_frame_ceiling_is_the_published_hour() {
    const PUBLISHED_MAX_FRAMES: u64 = 48_000 * 3_600; // 172_800_000
    assert_eq!(
        MAX_RENDER_FRAMES, PUBLISHED_MAX_FRAMES,
        "上限 = 1 小时 @ 48 kHz（规范那一条）"
    );

    let scratch = Scratch::new("frame-ceiling");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));
    let lead = id(2).to_canonical_string();
    let clip = id(10).to_canonical_string();
    // 与判据 9c 同一个极值工程：`placement.startTick` 取到 tick→帧饱和的那一格。
    let written = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead,
            "clipId": clip,
            "ops": [],
            "placement": {"startTick": SATURATING_START_TICK, "durationTicks": 480},
        }),
    );
    assert_eq!(written["status"], "success", "{written}");
    let proposal_id = written["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "frame-ceiling"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");

    let out = scratch.join("over.wav");
    let over = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_domain_error(&over, "RENDER_FAILED", "帧数超限");
    assert_eq!(
        over["error"]["data"]["maxFrames"], PUBLISHED_MAX_FRAMES,
        "拒绝报文必须报出**发布的那条**上限: {over}"
    );
    assert!(!out.exists(), "被拒绝的渲染不得留下文件: {out:?}");
}

/// 判据 9e：帧数闸门的**写形**必须紧贴上限（源码级判据）。
///
/// 为什么是源码级而不是行为级：区分 `frames > MAX_RENDER_FRAMES` 与
/// `frames > MAX_RENDER_FRAMES * 2` 的输入是一个帧数落在 `(1 小时, 2 小时]` 的工程，
/// 而**通过闸门之后**的实现会真的合成那一段（1.7e8 帧 ⇒ 约 1.0 GB 容器字节 +
/// 1.4 GB 浮点母带）。让一条常驻判据带上"回归时会吃 2~3 GB 内存、跑几分钟"的代价
/// 不可接受 ⇒ 这里钉**写形**，与 `contract.rs` 里
/// `only_the_loopback_and_dynamic_port_are_used_for_binding` 同一手法。
#[test]
fn the_frame_ceiling_gate_uses_the_constant_itself() {
    let text =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/domain/render.rs"))
            .expect("读 render.rs");
    assert!(
        text.contains("if frames > MAX_RENDER_FRAMES {"),
        "帧数闸门必须写成 `frames > MAX_RENDER_FRAMES`（放宽成系数会被这条判据抓住）"
    );
    assert_eq!(
        text.matches("MAX_RENDER_FRAMES").count(),
        3,
        "上限常量只许出现三次: 定义 + 比较 + 报文里的 maxFrames"
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
    // 内置乐器是音源、不是插入链来源 ⇒ 仍必须如实登记。
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

/// 判据 12b：离线插入链真的消费增益 / 低通；压缩仍整级不执行、仍登记。
///
/// 三个读数都必须从工具响应现算：
/// 1. 写了 `input_gain_db` / `cutoff_hz` 的 `InternalEffect` ⇒ `unsupported` **不含**
///    `deviceChainDsp`，且 `masterDigest` 与无设备基线不同（真的改了音频）；
/// 2. 只写压缩参数 ⇒ `deviceChainDsp` 仍登记，且 `masterDigest` 与基线**逐位相同**
///    （压缩级没有被偷偷执行）。
#[test]
fn offline_inserts_consume_gain_and_filter_but_still_report_the_compressor() {
    let scratch = Scratch::new("strip");
    let render_spec = |spec: &Spec, tag: &str| -> Value {
        let (mut dispatcher, auth) =
            dispatcher_with(&project(spec), &scratch.join(&format!("{tag}.yeban")));
        let out = scratch.join(&format!("{tag}.wav"));
        let result = call(
            &mut dispatcher,
            &auth,
            json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
        );
        assert_eq!(result["status"], "success", "{result}");
        result
    };
    let reported = |result: &Value| -> bool {
        result["data"]["unsupported"]
            .as_array()
            .expect("unsupported")
            .iter()
            .any(|key| key == "deviceChainDsp")
    };
    let effect = |seed: u32, params: &[(&str, f32)]| -> DeviceDefinition {
        let mut device = device(seed, 0, false);
        device.kind = DeviceKind::InternalEffect;
        for (name, value) in params {
            device.params.push(yeban_model::ParameterValue {
                name: (*name).to_owned(),
                value: *value,
                unit: None,
            });
        }
        device
    };

    let baseline = render_spec(&Spec::default(), "baseline");

    let gain = render_spec(
        &Spec {
            devices: vec![effect(80, &[("input_gain_db", -6.0)])],
            ..Spec::default()
        },
        "gain",
    );
    assert!(!reported(&gain), "增益已被离线插入链消费: {gain}");
    assert_ne!(
        gain["data"]["masterDigest"], baseline["data"]["masterDigest"],
        "写进去的 -6 dB 必须改变母带字节"
    );

    let filter = render_spec(
        &Spec {
            devices: vec![effect(81, &[("cutoff_hz", 100.0)])],
            ..Spec::default()
        },
        "filter",
    );
    assert!(!reported(&filter), "低通已被离线插入链消费: {filter}");
    assert_ne!(
        filter["data"]["masterDigest"], baseline["data"]["masterDigest"],
        "100 Hz 低通必须改变 440 Hz 音符的母带字节"
    );

    let compressor = render_spec(
        &Spec {
            devices: vec![effect(
                82,
                &[("threshold_db", -30.0), ("ratio", 4.0), ("makeup_db", 6.0)],
            )],
            ..Spec::default()
        },
        "compressor",
    );
    assert!(
        reported(&compressor),
        "压缩没有被求值 ⇒ 必须登记: {compressor}"
    );
    assert_eq!(
        compressor["data"]["masterDigest"], baseline["data"]["masterDigest"],
        "压缩级离线不执行 ⇒ 母带必须与无设备基线逐位相同"
    );
}

// ---------------------------------------------------------------------------
// 判据 12：unsupported 载荷是"如实的", 不是一句套话
// ---------------------------------------------------------------------------

#[test]
fn unsupported_features_are_named_and_counted_only_when_present() {
    let scratch = Scratch::new("unsupported");
    // 一条带了本切片没有消费的设备参数 ⇒ 必须登记 `deviceChainDsp`
    // （`#241` 之后循环重复不再登记，所以这条判据改用设备链做触发器）。
    let mut device = device(70, 0, false);
    device.params.push(yeban_model::ParameterValue {
        name: "filter_cutoff_hz".to_owned(),
        value: 1_000.0,
        unit: Some("Hz".to_owned()),
    });
    let spec = Spec {
        devices: vec![device],
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
    let unsupported = result["data"]["unsupported"]
        .as_array()
        .expect("unsupported");
    assert!(
        unsupported.iter().any(|key| key == "deviceChainDsp"),
        "未消费的设备参数必须登记: {result}"
    );
    assert_eq!(
        result["data"]["unsupportedCounts"]["deviceChainDsp"], 1,
        "出现次数也要如实给出"
    );
    assert!(
        !unsupported.iter().any(|key| key == "audioClips"),
        "这个夹具里没有音频片段, 不许凭空登记: {unsupported:?}"
    );
    assert!(
        !unsupported.iter().any(|key| key == "clipLoopRepetition"),
        "循环重复已经展开, 但这条夹具没开循环 ⇒ 更不许登记: {unsupported:?}"
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

/// 循环区间 `0..960` 落在摆放跨度 `0..1920` **之内** ⇒ 必然回卷一次
/// ⇒ 循环区间按周期重复、铺满摆放（`N = 1920 / 960 = 2`）。
///
/// 默认夹具的音符横跨 `0..1920`，但循环区间是 `0..960` ⇒ 每一遍的时值裁到
/// 区间末端 `960`，于是母带里是**两个 960 tick 的音符**（0 与 960 起音），
/// 而不是一个 1920 tick 的长音。`unsupported` 必须是空数组：展开之后不再登记
/// `clipLoopRepetition`。
///
/// 可被什么注入破坏：
/// - 只把长度撑长、不复制音符 ⇒ `sources[].notes` 仍是 1，且第 2 段静音；
/// - 复制时不裁尾巴 ⇒ 第 2 段叠加前一遍的尾音，`masterDigest` 与"两段相同"
///   的预期不符（`#241` 的分段峰值判据会红）。
#[test]
fn a_loop_shorter_than_the_placement_is_expanded() {
    let scratch = Scratch::new("loop-short");
    let short_loop = LoopConfig {
        enabled: true,
        start_tick: 0,
        end_tick: 960,
    };
    let (result, out) = render_with_loop(&scratch, "short", short_loop);
    assert_eq!(
        result["data"]["unsupported"],
        json!([]),
        "循环重复已经展开 ⇒ 不许再登记 clipLoopRepetition: {result}"
    );
    assert_eq!(
        result["data"]["unsupportedCounts"],
        json!({}),
        "没有登记项 ⇒ 计数表为空: {result}"
    );
    assert_eq!(
        result["data"]["sources"][0]["notes"], 2,
        "一个 1920 tick 的音符在 0..960 的循环里排成两遍: {result}"
    );
    assert_eq!(
        result["data"]["frames"], 48_000,
        "N=2 遍 × 960 tick (0.5 秒) 恰好铺满 1 秒: {result}"
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
    assert_ne!(
        result["data"]["masterDigest"], plain["data"]["masterDigest"],
        "循环展开必须改变母带样本 (两遍起音 ≠ 一个长音)"
    );
    assert_ne!(
        result["data"]["sha256"], plain["data"]["sha256"],
        "文件字节摘要同理"
    );
    assert_ne!(
        fs::read(&out).expect("产物"),
        fs::read(&plain_out).expect("产物"),
        "产物文件必须不同（重复真的被展开了）"
    );
}

/// 循环**不回卷**的边界：区间终点 `>= 摆放跨度`（含恰好覆盖）⇒ 单遍播放，
/// 与"不循环"逐字节相同、且不登记任何东西。
///
/// - `0..1920`（== 摆放跨度）⇒ 回卷点与摆放末端重合；
/// - `480..1920`（区间不覆盖，但 `end_tick == duration_ticks`）⇒ 摆放内不会回卷；
/// - `0..3840`（终点越过摆放末端）⇒ 更不会回卷。
///
/// 可被什么注入破坏：把 `yeban_model::ClipPlacement::loop_repeat` 的条件写成
/// `end_tick > duration_ticks`
/// （丢掉 `==` 那一半）⇒ `0..1920` 那一段会开始"重复"、字节与关循环不同 ⇒ 红。
#[test]
fn a_loop_that_does_not_wrap_before_the_placement_ends_is_bit_identical() {
    let scratch = Scratch::new("loop-covers");
    let (plain, plain_out) = render_with_loop(&scratch, "plain", LoopConfig::default());
    assert_eq!(
        plain["data"]["masterDigest"], DEFAULT_MASTER_DIGEST,
        "关循环的那一次就是既有默认夹具 (常量锚)"
    );
    for (tag, loop_config) in [
        (
            "covering",
            LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: 1_920,
            },
        ),
        (
            "offset",
            LoopConfig {
                enabled: true,
                start_tick: 480,
                end_tick: 1_920,
            },
        ),
        (
            "beyond",
            LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: 3_840,
            },
        ),
    ] {
        let (result, out) = render_with_loop(&scratch, tag, loop_config);
        assert_eq!(
            result["data"]["unsupported"],
            json!([]),
            "{tag}: 没有回卷 ⇒ 不许登记任何东西: {result}"
        );
        assert_eq!(
            result["data"]["masterDigest"], plain["data"]["masterDigest"],
            "{tag}: 摆放内不会回卷 ⇒ 必须与关循环逐位相同"
        );
        assert_eq!(
            fs::read(&out).expect("产物"),
            fs::read(&plain_out).expect("产物"),
            "{tag}: 产物文件必须逐字节相同"
        );
    }
}

// ---------------------------------------------------------------------------
// 判据 12c：循环重复**真的展开**成 N 遍音符（不只是把长度撑长）
// ---------------------------------------------------------------------------

/// 把默认夹具改成"短音符 + 指定循环配置"：默认夹具的音符横跨整个摆放，
/// 看不出"分段峰值相同"；这一条要的是**每一遍都有一响**。
fn project_with_loop_note(
    loop_config: LoopConfig,
    placement_duration: u64,
    note_start: u64,
    note_duration: u64,
) -> YebanProjectV1 {
    let mut project = project(&Spec {
        end_tick: placement_duration,
        loop_config,
        ..Spec::default()
    });
    let clip = id(10);
    let note_id = id(30);
    let entry = project.clip_pool.get_mut(&clip).expect("默认夹具的片段");
    let ClipContent::Midi { notes } = &mut entry.content else {
        panic!("默认夹具必须是 MIDI 片段");
    };
    let mut note = notes.get(&note_id).expect("默认夹具的音符").clone();
    note.start_tick = note_start;
    note.duration_ticks = note_duration;
    notes.insert(note_id, note);
    project
        .validate()
        .unwrap_or_else(|error| panic!("循环夹具工程必须合法: {error}"));
    project
}

/// ⭐ `#241`：启用循环 ⇒ 循环区间按 `duration_ticks` 重复，音符**真的**排 N 遍。
///
/// 夹具：摆放 `0..1920`（= 1 秒 @ 120 BPM / 960 PPQ / 48 kHz = 48000 帧），
/// 循环区间 `0..480`（= 12000 帧 ⇒ N = 4），音符 `0..240`。
/// ⇒ 期望：4 个音符分别在 0 / 480 / 960 / 1440 tick 起音，四个 12000 帧分段
/// 的峰值相等且 > 0；`unsupported` 不再登记 `clipLoopRepetition`。
///
/// 可被什么注入破坏：
/// - 只把 `end_tick` 撑长、不复制音符 ⇒ 第 2/3/4 段峰值落回抖动底噪；
/// - 复制时漏掉最后一轮（`iterations` 用 floor 而不是 ceil）⇒ 第 4 段静音；
/// - 复制时不裁剪音符尾巴到循环区间末端 ⇒ 分段峰值被前一遍的尾音叠加。
#[test]
fn a_loop_is_expanded_into_n_repetitions() {
    let scratch = Scratch::new("loop-expand");
    let loop_config = LoopConfig {
        enabled: true,
        start_tick: 0,
        end_tick: 480,
    };
    let project = project_with_loop_note(loop_config, 1_920, 0, 240);
    let (mut dispatcher, auth) = dispatcher_with(&project, &scratch.join("loop.yeban"));
    let out = scratch.join("loop.wav");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{result}");
    let unsupported = result["data"]["unsupported"]
        .as_array()
        .expect("unsupported");
    println!(
        "### 循环展开读数: frames={} notes={} unsupported={:?} sha256={}",
        result["data"]["frames"],
        result["data"]["sources"][0]["notes"],
        unsupported,
        result["data"]["sha256"],
    );
    assert_eq!(
        result["data"]["frames"], 48_000,
        "帧数 = 摆放跨度 (1 秒 @ 48 kHz): {result}"
    );
    assert_eq!(
        result["data"]["sources"][0]["notes"], 4,
        "一个音符 × N=4 遍 ⇒ 排程 4 个音符: {result}"
    );
    assert!(
        !unsupported.iter().any(|key| key == "clipLoopRepetition"),
        "循环重复已经展开, 再报它就是假话: {unsupported:?}"
    );

    // 分段峰值：每段 = 一个循环区间（480 tick = 12000 帧）。
    let bytes = fs::read(&out).expect("产物");
    let parsed = yeban_render::rf64::parse_container(&bytes).expect("容器");
    let samples = decode_i24(&bytes[parsed.data.clone()]);
    let segment_frames = 480 * frames_per_tick();
    assert_eq!(
        segment_frames * 4,
        result["data"]["frames"].as_u64().expect("frames") as usize,
        "N=4 段恰好铺满母带 ⇒ 帧数就是 N 倍循环长度"
    );
    let peaks: Vec<f32> = (0..4)
        .map(|index| window_peak(&samples, 2, index * segment_frames, segment_frames))
        .collect();
    println!("### 四段峰值（逐字）: {peaks:?}");
    assert!(
        peaks.iter().all(|value| *value > DITHER_ONLY_PEAK),
        "每一遍都必须真的发声: {peaks:?}"
    );
    for (index, value) in peaks.iter().enumerate() {
        assert!(
            (value - peaks[0]).abs() < 1.0e-6,
            "第 {} 段的峰值必须与第 1 段相同（真的重复同一内容）: {peaks:?}",
            index + 1
        );
    }

    // 正对照：同一份工程**关掉循环** ⇒ 只有第 1 段发声，且字节不同
    // （证明上面的分段读数不是"母带本来就长这样"）。
    let plain_project = project_with_loop_note(LoopConfig::default(), 1_920, 0, 240);
    let (mut plain_dispatcher, plain_auth) =
        dispatcher_with(&plain_project, &scratch.join("plain.yeban"));
    let plain_out = scratch.join("plain.wav");
    let plain = call(
        &mut plain_dispatcher,
        &plain_auth,
        json!({"format": "wav", "sampleRate": 48000, "path": plain_out.display().to_string()}),
    );
    assert_eq!(plain["status"], "success", "{plain}");
    assert_eq!(
        plain["data"]["unsupported"],
        json!([]),
        "关循环的工程不许有任何登记: {plain}"
    );
    let plain_bytes = fs::read(&plain_out).expect("正对照产物");
    let plain_parsed = yeban_render::rf64::parse_container(&plain_bytes).expect("容器");
    let plain_samples = decode_i24(&plain_bytes[plain_parsed.data.clone()]);
    let plain_peaks: Vec<f32> = (0..4)
        .map(|index| window_peak(&plain_samples, 2, index * segment_frames, segment_frames))
        .collect();
    println!("### 关循环的四段峰值（逐字）: {plain_peaks:?}");
    assert!(
        plain_peaks[0] > DITHER_ONLY_PEAK,
        "正对照的第一段必须发声: {plain_peaks:?}"
    );
    assert!(
        plain_peaks[1..]
            .iter()
            .all(|value| *value <= DITHER_ONLY_PEAK),
        "关循环后第 2..4 段必须静音: {plain_peaks:?}"
    );
    assert_ne!(
        result["data"]["sha256"], plain["data"]["sha256"],
        "展开循环必须改变母带字节（否则只是撑长度）"
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
/// "真的重新触发"的机械证据：第 0 个脉冲在**没有前置声部**时起音 ⇒ 前 60 帧能量
/// 必须远低于脉冲中段；其余脉冲的起点必须与"同一音符**不连击**的长音"在同一帧上的
/// 60 帧能量**不同**（引擎的 ADSR 释放尾巴会叠在起点上，所以这里量的是"边界改变了
/// 信号"，⛔ 不是"起点更安静"——后者是已退役的线性包络正弦源的性质）。
///
/// ⚠ `#268` 的语义变化（对着代码读）：引擎音源在 `note.end_sample` 之后走 ADSR
/// release（默认 50 ms），因此相邻脉冲的尾巴会**重叠**；CLI 导出一直如此。
#[test]
fn ratchet_expands_one_note_into_exactly_n_pulses_on_the_engines_grid() {
    let scratch = Scratch::new("ratchet-expansion");
    let per_tick = frames_per_tick();
    let duration_ticks = 1_920u64;
    // 参照：同一音符**不连击** ⇒ 一个长音（起点上不会有重新起振）。
    let (_, plain_samples) = render_note(&scratch, "rplain", None, None, duration_ticks);
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
            if pulse == 0 {
                assert!(
                    head < body * 0.3,
                    "ratchet={ratchet}: 第 0 个脉冲的起点 {start} 帧能量 {head:.6} \
                     必须显著低于脉冲中段 {body:.6}（没有前置声部 ⇒ 必须是新起音）"
                );
            } else {
                let plain_head = window_rms(&plain_samples, 2, start, 60);
                let relative = (head - plain_head).abs() / body.max(f64::MIN_POSITIVE);
                assert!(
                    relative > 0.02,
                    "ratchet={ratchet}: 脉冲 {pulse} 的起点 {start} 帧能量 {head:.6} \
                     必须与不连击的长音在同一帧的 {plain_head:.6} 不同（相对差 {relative:.6}）\
                     ⇒ 边界处确实重新触发了"
                );
            }
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

/// 余数**不补**：整数除法算出的最后一个脉冲在音符时值**之前**结束。
///
/// ⚠ `#268`（对着代码读）：引擎音源在 `note.end_sample` 之后走 ADSR release
/// （默认 50 ms）⇒ 余数区间**不是**数字静音，而是释放尾巴。因此判据从
/// "尾段 ≤ 抖动底噪"改成"尾段峰值**低于**同一帧上'不连击长音'的峰值"：
/// - 用**向上**取整（`div_ceil`）或把最后一个脉冲拉到音符末端 ⇒ 尾段仍在 gate 内
///   ⇒ 峰值与长音的同一窗口**逐位相同** ⇒ 红；
/// - 正常（向下取整）⇒ 尾段已在 release ⇒ 峰值严格更低（默认 release 50 ms，
///   余数区间 25–50 帧 ⇒ 约 1–2% 的下降，远超 f32 噪声）。
#[test]
fn the_ratchet_grid_uses_integer_division_and_leaves_the_remainder_silent() {
    let scratch = Scratch::new("ratchet-remainder");
    let per_tick = frames_per_tick();
    // 三组都**有**余数（否则这条判据没有牙齿）。
    // ⚠ `#268`：引擎的 release 是 50 ms ⇒ 余数窗口（至少 100 帧 = 5% 释放）才
    // 足够把"已 gate off"与"还在 gate 内"分开；因此夹具取较大的 ratchet
    // （余数 = 4–10 tick = 100–250 帧）。
    for (ratchet, duration_ticks) in [(16u8, 1_000u64), (15, 1_000), (12, 1_000)] {
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
        // 参照：同一音符不连击（长音）在同一余数窗口的峰值。
        let (_, plain_samples) = render_note(
            &scratch,
            &format!("remplain{ratchet}-{duration_ticks}"),
            None,
            None,
            duration_ticks,
        );
        let window = note_end - content_end;
        let tail = window_rms(&samples, 2, content_end, window);
        let plain_tail = window_rms(&plain_samples, 2, content_end, window);
        println!(
            "### ratchet={ratchet} / {duration_ticks} tick: 余数窗口 RMS {tail:.9} \
             vs 长音 {plain_tail:.9}（相对 {:.6}）",
            tail / plain_tail
        );
        assert!(
            tail < plain_tail * 0.99,
            "ratchet={ratchet} / {duration_ticks} tick: 余数区间 \
             [{content_end}, {note_end}) 必须已在 release（RMS 至少低 1%）, \
             实测 {tail:.9} vs 长音 {plain_tail:.9}"
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

// ---------------------------------------------------------------------------
// B1：导出物**不带作者**（口径裁定 = "维持现状"）
// ---------------------------------------------------------------------------

/// ⭐ **B1-a：音频导出的 `bext` 元数据**不含任何作者字段**。
///
/// ## 判据在设计上钉住的事实（裁定：音频/工程导出**不加**作者）
///
/// `RenderArtifact` 的 `bwf` 只写 `originator`（⭐ 字面量 `"Yeban DAW"` ＝ **软件名**，
/// ⛔ 不是作者）／`originatorReference`（工程 ULID）／日期时刻／coding history／响度哨兵。
/// ⭐ 给音频产物加作者会改**对外格式**（兼容/互操作成本）⇒ 裁定维持现状。
///
/// ## ✅ 正面锚点（防"导出成空 ⇒ 断言恒真"的假绿）
///
/// `originatorReference` 必须**等于夹具工程的 ULID** —— 没有这一条，一个把 `bwf` 写成
/// 空对象甚至删掉的改动也会让"无作者键"通过。
///
/// ## 什么改动会让它红（注入）
///
/// 在 `render.rs` 的 `bwf` 里插一个作者键（例如
/// `"author": project.author`、`"artist": …`、`"credit": …`、`"copyright": …`）
/// ⇒ 键名遍历命中 ⇒ 红。⚠ 断言遍历的是**实测键名**（`as_object().keys()`），
/// 不是手抄的键清单 ⇒ 加键必被抓。
#[test]
fn the_audio_export_carries_no_author_metadata() {
    let scratch = Scratch::new("bwf-no-author");
    let fixture = project(&Spec::default());
    let project_id = fixture.id.to_canonical_string();
    let (mut dispatcher, auth) = dispatcher_with(&fixture, &scratch.join("demo.yeban"));
    let out = scratch.join("no-author.wav");
    let rendered = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");

    let bwf = rendered["data"]["bwf"].as_object().expect("bwf 对象");
    // ✅ 正面锚点：这一段真的写了元数据，而且写的是**这个工程**的 ULID。
    assert_eq!(
        bwf.get("originatorReference").and_then(Value::as_str),
        Some(project_id.as_str()),
        "`bext` 的 OriginatorReference 必须是工程的 ULID（`ARCH-FMT-001` 的映射）: {bwf:?}"
    );
    assert_eq!(
        bwf.get("originator").and_then(Value::as_str),
        Some("Yeban DAW"),
        "`originator` 是**软件名**（BWF 语义），⛔ 不是作者: {bwf:?}"
    );

    // ⭐ 核心：**实测键名**里不得出现任何作者语义的键。
    let forbidden = ["author", "artist", "credit", "copyright"];
    let offenders: Vec<&String> = bwf
        .keys()
        .filter(|key| {
            let lowered = key.to_ascii_lowercase();
            forbidden.iter().any(|needle| lowered.contains(needle))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "音频导出的 `bext` ⛔ 不得携带作者字段（裁定：维持现状）; 命中: {offenders:?} \
         (实测全部键: {:?})",
        bwf.keys().collect::<Vec<_>>()
    );
}

/// ⭐ **B1-a 的控制臂**：把**注入的工程作者**做成一个唯一标记，再渲染一次 ——
/// 该标记⛔ **不得出现在产物字节里**。
///
/// 为什么需要它（与上一条的分工）：上一条枚举 `bwf` 的**键**；这一条抓的是
/// "作者的值被塞进**别的**地方"（例如塞进 `codingHistory`、塞进一个自定义 chunk、
/// 或塞进 BWF 的 `Description`）—— 那种改动不会新增键名，因此上一条抓不到。
///
/// ## 什么改动会让它红（注入）
///
/// 任何把 `project.author` 写进容器字节的改动（`coding_history(...)` 里拼作者、
/// 新增一个带作者的 chunk、把 `Description` 填成作者）⇒ 标记出现在文件字节里 ⇒ 红。
/// ⚠ **前提是标记真的进了工程**：先断言 `yeban_edit_notes` 的 `setAuthor` 真的改了它
/// （否则这条判据会因为"作者压根没设置"而恒真 —— 那是本会话反复清的那一族）。
#[test]
fn the_audio_export_never_embeds_the_project_author_bytes() {
    const AUTHOR_MARKER: &str = "yeban-author-probe-0123456789abcdef";
    let scratch = Scratch::new("bwf-author-bytes");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));

    let edited = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": id(1).to_canonical_string(),
            "clipId": id(10).to_canonical_string(),
            "ops": [{"kind": "setAuthor", "author": AUTHOR_MARKER}],
        }),
    );
    assert_eq!(edited["status"], "success", "{edited}");
    // ⚠ `yeban_edit_notes` **只建提案**（⛔ 不直接改工程）⇒ 必须再合并一次。
    let proposal = edited["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案身份");
    let merged = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal, "commitMessage": "设置作者（B1 判据夹具）" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    // ⭐ 前置：标记**真的**进了工程（否则下面的"字节里没有它"是恒真的空话）。
    assert_eq!(
        dispatcher
            .domain()
            .active_project()
            .expect("活跃工程")
            .author,
        AUTHOR_MARKER,
        "`setAuthor` 必须真的把作者写进工程，否则本条判据没有前提"
    );

    let out = scratch.join("author-bytes.wav");
    let rendered = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");

    let bytes = fs::read(&out).expect("读产物");
    // ✅ 正面锚点：产物非空且是 RIFF 容器 ⇒ "标记不在"不是"文件是空的"。
    assert!(bytes.len() > 1024, "产物必须有内容: {} 字节", bytes.len());
    assert_eq!(&bytes[..4], b"RIFF", "WAV 必须以 RIFF 开头");
    assert!(
        !bytes
            .windows(AUTHOR_MARKER.len())
            .any(|window| window == AUTHOR_MARKER.as_bytes()),
        "⭐ 工程作者 `{AUTHOR_MARKER}` 出现在音频产物字节里了 —— \
         裁定是「音频/工程导出不加作者」; ⛔ 不要把它写进容器"
    );
}

/// 本地 base64 解码（RFC 4648 §4，只做本文件用得到的那一面）。
///
/// ⚠ **为什么这是测试本地的一份**：MCP 只公开了**编码**侧
/// （`yeban_mcp::domain::export_midi::base64_encode`），没有解码侧。
/// ⚠ 我**实测否决**过一个"不解码"的替代方案：拿 `base64_encode(marker)` 的输出去
/// 在返回的 `content` 里找子串 —— **base64 不是子串保序的**（实测：把标记前置
/// 0/1/2 字节偏移后，编码片段**全部不命中**）⇒ 那种断言**无牙**。
/// ⇒ 要抓"任意新元事件里塞了作者"，就必须**回到真字节**上找，
/// 因此这里与 `tests/extension_tools.rs` 的 `decode_base64` 同源同职责（判据本地）。
fn decode_base64_for_assertions(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u32;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => panic!("非法的 base64 字符: {other}"),
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xFF).expect("一个字节"));
        }
    }
    out
}

/// ⭐ **B1-b：MIDI 导出的字节**不含工程作者**。
///
/// ## 判据在设计上钉住的事实（裁定："若将来要加作者 ⇒ 只加在 MIDI" = **未来选项**）
///
/// ⭐ 今天 MIDI 导出**也不写作者**：`yeban-midi/src/export.rs` 只从工程取 `bpm` 与拍号
/// 生成 conductor 轨（`tempo_map`），`author` 在 `export.rs`／`midi.rs`／
/// `export_midi.rs` 三个文件里**零命中**。⭐ 本条把"今天不写"钉住 ⇒ 将来有人真去加，
/// 会**先红**，从而被迫显式改这条判据（那正是"明示选择"的机制）。
///
/// ## 做法（为什么不是"编码标记再找子串"）
///
/// 返回的 `content` 是 base64 ⇒ 解回**真字节**（本地解码器，见其文档），
/// 在真字节上找标记。⚠ 编码后再找子串的方案被我**实测否决**：base64 不保子串。
///
/// ## 什么改动会让它红（注入）
///
/// 在 SMF 里写任何携带 `project.author` 的元事件（`FF 01` 文本 / `FF 02` 版权 /
/// 轨名 `FF 03`）⇒ 标记出现在字节里 ⇒ 红。⚠ **前提是标记真的进了工程**：
/// 先断言 `setAuthor` ＋ 合并之后 `project.author` 就是标记（否则这条判据恒真）。
#[test]
fn the_midi_export_never_embeds_the_project_author_bytes() {
    const AUTHOR_MARKER: &str = "yeban-author-probe-midi-0123456789abcdef";
    let scratch = Scratch::new("midi-author-bytes");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("demo.yeban"));

    let edited = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": id(1).to_canonical_string(),
            "clipId": id(10).to_canonical_string(),
            "ops": [{"kind": "setAuthor", "author": AUTHOR_MARKER}],
        }),
    );
    assert_eq!(edited["status"], "success", "{edited}");
    let proposal = edited["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案身份");
    let merged = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal, "commitMessage": "设置作者（B1-b 判据夹具）" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    // ⭐ 前置：标记**真的**进了工程。
    assert_eq!(
        dispatcher
            .domain()
            .active_project()
            .expect("活跃工程")
            .author,
        AUTHOR_MARKER,
        "`setAuthor` 必须真的把作者写进工程，否则本条判据没有前提"
    );

    let exported = call_tool(&mut dispatcher, &auth, "yeban_export_midi", json!({}));
    assert_eq!(exported["status"], "success", "{exported}");
    let content = exported["data"]["content"].as_str().expect("base64 正文");
    let bytes = decode_base64_for_assertions(content);
    // ✅ 正面锚点：真字节非空、且是 SMF（`MThd`）⇒ "标记不在"不是"内容为空"。
    assert!(
        bytes.len() > 14,
        "导出的 SMF 必须有内容: {} 字节",
        bytes.len()
    );
    assert_eq!(&bytes[..4], b"MThd", "SMF 必须以 MThd 开头");
    assert_eq!(
        u64::try_from(bytes.len()).expect("尺寸"),
        exported["data"]["bytes"].as_u64().expect("bytes"),
        "解回来的长度必须等于报出的字节数（证明解码器与产出一致，不是空解）"
    );
    assert!(
        !bytes
            .windows(AUTHOR_MARKER.len())
            .any(|window| window == AUTHOR_MARKER.as_bytes()),
        "⭐ 工程作者 `{AUTHOR_MARKER}` 出现在 MIDI 导出字节里了 —— \
         裁定是「维持现状（MIDI 也不写作者）」; 若要改，请**显式**改这条判据"
    );
}

// ---------------------------------------------------------------------------
// 判据 14（#175 / #174 坑⑧）：导出**削波**必须可见
// ---------------------------------------------------------------------------

/// 过载工程 ⇒ `clippedSamples > 0` ＋ `truePeakDbfs > 0` ＋ `headroomDb < 0`。
///
/// #174 实测：响应只给 `peak.before = 1.5748`，文件里 **6134 个样本**被钳在 `±1.0`，
/// 而调用方**无法**从峰值推出"我导出的是一份削波文件"。本条钉死三个新读数，
/// 并用**独立解码**（[`decode_i24`]）核对文件里真的有贴在满量程的样本。
#[test]
fn a_clipping_master_reports_clipped_samples_and_true_peak() {
    let scratch = Scratch::new("clip-visible");
    // 满力度 + +24 dB 推子 ⇒ 浮点母带远超满量程（实测 `peak.before` ≈ 11）。
    let loud = Spec {
        velocity: 127,
        volume_db: 24.0,
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&loud), &scratch.join("loud.yeban"));
    let path = scratch.join("loud.wav");
    let rendered = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": path.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let data = &rendered["data"];
    println!(
        "### 过载工程的读数（逐字）\n{}",
        serde_json::to_string_pretty(data).expect("pretty")
    );

    let peak_before = data["peak"]["before"].as_f64().expect("peak.before");
    assert!(
        peak_before > 1.0,
        "夹具必须真的过载（峰值 > 1.0），实测 {peak_before}"
    );
    let clipped = data["clippedSamples"]
        .as_u64()
        .expect("clippedSamples 必须是整数");
    assert!(
        clipped > 0,
        "过载工程必须报出被 24-bit 量化器钳住的样本数，实际 {clipped}"
    );
    let true_peak = data["truePeakDbfs"]
        .as_f64()
        .expect("truePeakDbfs 必须是数");
    assert!(
        true_peak > 0.0,
        "真峰值必须 > 0 dBFS（过载），实测 {true_peak}"
    );
    let headroom = data["headroomDb"].as_f64().expect("headroomDb 必须是数");
    assert!(
        (headroom + true_peak).abs() < 1.0e-6,
        "headroomDb 必须等于 -truePeakDbfs：{headroom} vs {}",
        -true_peak
    );
    assert!(headroom < 0.0, "过载 ⇒ 负余量，实测 {headroom}");

    // 文件里**真的**有贴在满量程端点的样本（独立解码，不用生产代码）。
    let samples = verify_file_shape(&rendered, &path);
    let full_scale = 8_388_607.0f32 / 8_388_608.0;
    let clamped = samples
        .iter()
        .filter(|sample| sample.abs() >= full_scale)
        .count();
    assert!(
        clamped > 0,
        "写出的文件里必须有钳到满量程端点的样本（否则 '削波' 是假读数）"
    );

    // ⭐ `peak.before/after` 的语义必须**写在响应里**（#174 坑⑧的第二半）。
    let semantics = data["peak"]["semantics"]
        .as_str()
        .expect("peak.semantics 必须是字符串");
    assert!(semantics.contains("归一化"), "{semantics}");
    assert!(semantics.contains("不是文件峰值"), "{semantics}");
    assert!(semantics.contains("不是真峰值"), "{semantics}");
}

/// 普通工程 ⇒ `clippedSamples == 0` ＋ `truePeakDbfs <= 0` ＋ `headroomDb >= 0`。
///
/// 这一条挡住"永远报一个正数"的假读数：同一份夹具（默认力度 / 0 dB 推子）必须干净。
#[test]
fn a_normal_master_reports_zero_clipped_samples_and_no_overshoot() {
    let scratch = Scratch::new("clip-normal");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("normal.yeban"));
    let path = scratch.join("normal.wav");
    let rendered = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": path.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let data = &rendered["data"];
    println!(
        "### 普通工程的削波读数（逐字）: clippedSamples={} truePeakDbfs={} headroomDb={} peak.before={} peak.after={}",
        data["clippedSamples"],
        data["truePeakDbfs"],
        data["headroomDb"],
        data["peak"]["before"],
        data["peak"]["after"],
    );
    assert_eq!(data["clippedSamples"], 0, "普通工程不得报削波样本: {data}");
    let true_peak = data["truePeakDbfs"]
        .as_f64()
        .expect("truePeakDbfs 必须是数");
    assert!(
        true_peak <= 0.0,
        "普通工程的真峰值必须 <= 0 dBFS，实测 {true_peak}"
    );
    let headroom = data["headroomDb"].as_f64().expect("headroomDb 必须是数");
    assert!(headroom >= 0.0, "普通工程余量必须 >= 0 dB，实测 {headroom}");
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#225`：削波预警 ＋ 建议 trim（⛔ 不自动改增益）
// ---------------------------------------------------------------------------
//
// ## 这一节量什么（对象 ＋ 单位）
//
// | 量 | 来源 | 单位 |
// | :--- | :--- | :--- |
// | `warnings[]` 的条数 | `data.warnings`（结果质量提示） | 条 |
// | `suggestedTrimDb` | 预警字段（建议把母带增益减多少） | dB（负值 = 减） |
// | `sources[].peak` | 每个源节点输出的样本峰值 | 满刻度线性幅度 |
// | `sources[].peakDbfs` | 同一个峰值的 dBFS 读数 | dBFS |
// | `loudestSource.track` | 逐源峰值最大的那条轨 | 字符串 |
//
// ## 为什么这几条必须一起写
//
// 只断言"有 warnings 字段"对缺陷是**瞎的**：恒报一条空警告也能通过。
// 因此这里同时钉住 ① 削波时**恰好一条**（且带公式与最响源）、② 不削波时**空数组**、
// ③ 加预警前后整文件 `sha256` **逐字节相同**（用**加预警之前**在 `main` 上测到的常量）、
// ④ 两源工程里 `loudestSource` 真的指向更响的那条轨（不是"只有一条源所以永远对"）。

/// 过载夹具（`velocity=127` ＋ `volume_db=+24`）的 RIFF **文件字节** SHA-256
/// —— 在加 `#225` 预警**之前**（`main` @ 78e92246）实测、之后照抄的锚。
///
/// 它证明"预警只提示、不动音频"：如果探针写了样本、改了增益或改了落盘顺序，
/// 这个值立刻变红。
///
/// **`#255` 重钉（`bext` 奇长度互通修复）**：该夹具的 `CodingHistory` 是奇数长度,
/// 写入器现在把 NUL 终止符计入 `bext` 负载 ⇒ 长度字段 +1。母带样本摘要
/// （`CLIPPING_MASTER_DIGEST`）逐位未变。
///
/// **`#268` 重钉（`ADR-0011` 引擎音色）**：MCP 的 MIDI 音源从正弦换成 `SynthEngine`
/// ⇒ 母带样本摘要 `246bb7f7…` → `d01ba94b…`、文件摘要 `210876d6…` → `5b5f2910…`、
/// `clippedSamples` `91808` → `95164`、`truePeakDbfs` `24.000000` → `23.886497`。
/// 预警只提示、不动音频这条性质**未变**（本夹具仍用上面两个新常量逐字节钉住）。
const CLIPPING_MASTER_SHA256: &str =
    "5b5f291094cf4f6d8baf997f782bf4a8c7ca6d6fa4fcc177fa81b091292f2f52";

/// 同一份过载夹具的**母带样本位级摘要**（也测自加预警之前）。
const CLIPPING_MASTER_DIGEST: &str =
    "d01ba94b29bc8b2688bb4994f73fece1a996f2e441791343e7c8d8e570d1621a";

/// 过载工程 ⇒ `data.warnings[]` **恰好一条** `exportClipping`，带
/// `clippedSamples` / `truePeakDbfs` / **建议 trim** / **最响的源**。
#[test]
fn a_clipping_master_gets_a_clipping_warning_with_a_trim_suggestion() {
    let scratch = Scratch::new("clip-warning");
    let loud = Spec {
        velocity: 127,
        volume_db: 24.0,
        ..Spec::default()
    };
    let (mut dispatcher, auth) = dispatcher_with(&project(&loud), &scratch.join("loud.yeban"));
    let path = scratch.join("loud.wav");
    let rendered = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": path.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let data = &rendered["data"];
    println!(
        "### #225 过载工程（逐字）\n{}",
        serde_json::to_string_pretty(data).expect("pretty")
    );

    let warnings = data["warnings"]
        .as_array()
        .expect("warnings 必须是数组（不削波时是空数组）");
    assert_eq!(
        warnings.len(),
        1,
        "过载工程必须有**恰好一条**削波预警: {data}"
    );
    let warning = &warnings[0];
    assert_eq!(warning["code"], "exportClipping", "{warning}");
    assert_eq!(warning["severity"], "warning", "{warning}");
    assert_eq!(
        warning["blocking"], false,
        "预警不得阻断成功导出: {warning}"
    );
    assert_eq!(
        warning["clippedSamples"], data["clippedSamples"],
        "{warning}"
    );
    assert_eq!(warning["truePeakDbfs"], data["truePeakDbfs"], "{warning}");
    assert_eq!(warning["headroomDb"], data["headroomDb"], "{warning}");

    // ④ 建议值可算：公式逐字 + 按公式验算 + "减完不削波、最多多减 0.5 dB"。
    assert_eq!(
        warning["suggestedTrimDbFormula"], "suggestedTrimDb = -(ceil(truePeakDbfs * 2) / 2)",
        "{warning}"
    );
    let true_peak = warning["truePeakDbfs"]
        .as_f64()
        .expect("truePeakDbfs 必须是数");
    assert!(true_peak > 0.0, "夹具必须过载，实测 {true_peak}");
    let suggested = warning["suggestedTrimDb"]
        .as_f64()
        .expect("suggestedTrimDb 必须是数");
    let expected = -((true_peak * 2.0).ceil() / 2.0);
    assert!(
        (suggested - expected).abs() < 1.0e-9,
        "建议值必须等于公式: expected {expected}, actual {suggested}"
    );
    assert!(
        true_peak + suggested <= 1.0e-6,
        "按建议减完不得再削波: truePeak={true_peak}, trim={suggested}"
    );
    assert!(
        true_peak + suggested >= -0.5 - 1.0e-6,
        "最多多减 0.5 dB: truePeak={true_peak}, trim={suggested}"
    );

    // ② 每源峰值 + 点名最响的源。
    let sources = data["sources"].as_array().expect("sources");
    assert!(!sources.is_empty(), "过载母带不可能没有源: {data}");
    for source in sources {
        let peak = source["peak"]
            .as_f64()
            .unwrap_or_else(|| panic!("sources[].peak 必须是数: {source}"));
        assert!(peak > 0.0, "这条源的峰值必须 > 0: {source}");
        assert!(
            source["peakDbfs"].is_number(),
            "sources[].peakDbfs 必须是数: {source}"
        );
    }
    let loudest = &warning["loudestSource"];
    assert_eq!(loudest["track"], "Lead", "最响的源必须是 Lead: {warning}");
    assert_eq!(
        loudest["node"], data["sources"][0]["node"],
        "单源夹具里最响的源就是唯一那条源: {warning}"
    );
    assert!(
        loudest["peakDbfs"].as_f64().expect("peakDbfs") > 0.0,
        "最响的源自己也过了满量程: {warning}"
    );
    assert!(
        loudest["suggestedSourceTrimDb"]
            .as_f64()
            .expect("suggestedSourceTrimDb")
            < 0.0,
        "最响的源必须给出负的 trim 建议: {warning}"
    );
    // `hint` 必须**逐字**说清"不自动改增益"（agent 读得到的那一句）。
    let hint = warning["hint"].as_str().expect("hint 必须是字符串");
    assert!(hint.contains("不自动改增益"), "{hint}");
    assert!(hint.contains("clippedSamples"), "{hint}");
}

/// ② 不削波 ⇒ **没有**那条预警（`warnings == []`），证明不是恒报。
#[test]
fn a_normal_master_reports_an_empty_warnings_array() {
    let scratch = Scratch::new("clip-warning-normal");
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&Spec::default()), &scratch.join("normal.yeban"));
    let path = scratch.join("normal.wav");
    let rendered = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": path.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let data = &rendered["data"];
    println!(
        "### #225 普通工程（逐字）: clippedSamples={} truePeakDbfs={} warnings={}",
        data["clippedSamples"], data["truePeakDbfs"], data["warnings"]
    );
    assert_eq!(data["clippedSamples"], 0, "夹具必须不削波: {data}");
    assert_eq!(
        data["warnings"],
        json!([]),
        "不削波时 warnings 必须是空数组（⛔ 不是恒报）: {data}"
    );
}

/// ③ ⛔ 不改音频：同一份过载工程，加预警前后整文件 `sha256`（与母带摘要）逐字节相同。
///
/// 两个常量都在 `main` @ 78e92246（**加预警之前**）实测，运行时的响应必须与它们相等。
/// 这一条与上面那条**同一次响应**里还断言了 `warnings.len() == 1` ——
/// 证明"发生预警"与"字节不变"是同一份产物上的两件事，而不是两次跑的巧合。
#[test]
fn the_clipping_warning_does_not_change_a_single_byte() {
    let scratch = Scratch::new("clip-warning-bytes");
    let loud = Spec {
        velocity: 127,
        volume_db: 24.0,
        ..Spec::default()
    };
    let (mut dispatcher, auth) =
        dispatcher_with(&project(&loud), &scratch.join("loud-bytes.yeban"));
    let path = scratch.join("loud-bytes.wav");
    let rendered = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": path.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let data = &rendered["data"];
    assert_eq!(
        data["warnings"].as_array().map(Vec::len),
        Some(1),
        "这一条必须在**真有一条预警**的响应上量字节: {data}"
    );
    assert_eq!(
        data["sha256"], CLIPPING_MASTER_SHA256,
        "加预警前后整文件 sha256 必须逐字节相同"
    );
    assert_eq!(
        data["masterDigest"], CLIPPING_MASTER_DIGEST,
        "母带样本的位级摘要也必须逐位相同"
    );
    // 落盘字节也不能被改写：磁盘上的文件摘要 == 响应里的常量。
    assert_eq!(
        file_sha256(&path),
        CLIPPING_MASTER_SHA256,
        "磁盘上的母带必须与加预警之前的字节相同"
    );
}

/// ②b 两源工程 ⇒ `loudestSource` 真的指向更响的那条轨（不是"只有一条源"）。
#[test]
fn the_loudest_source_is_the_loudest_track() {
    let scratch = Scratch::new("clip-warning-loudest");
    // Lead +6 dB / Pad −18 dB ⇒ Lead 的逐源峰值约为 Pad 的 16 倍，母带因此削波。
    let (mut dispatcher, auth) = dispatcher_with(
        &two_track_clip_project(6.0, -18.0),
        &scratch.join("two.yeban"),
    );
    let path = scratch.join("two.wav");
    let rendered = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": path.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let data = &rendered["data"];
    let sources = data["sources"].as_array().expect("sources");
    assert_eq!(sources.len(), 2, "两轨夹具必须报两条源: {data}");
    let lead = sources
        .iter()
        .find(|source| source["track"] == "Lead")
        .expect("Lead 必须在 sources 里");
    let pad = sources
        .iter()
        .find(|source| source["track"] == "Pad")
        .expect("Pad 必须在 sources 里");
    let lead_peak = lead["peak"].as_f64().expect("lead peak");
    let pad_peak = pad["peak"].as_f64().expect("pad peak");
    println!(
        "### #225 两源工程（逐字）: lead.peak={lead_peak} lead.peakDbfs={} pad.peak={pad_peak} pad.peakDbfs={} warnings={}",
        lead["peakDbfs"], pad["peakDbfs"], data["warnings"]
    );
    assert!(
        lead_peak > pad_peak,
        "Lead (+6 dB) 必须比 Pad (−18 dB) 响: {lead_peak} vs {pad_peak}"
    );
    let warning = &data["warnings"][0];
    assert_eq!(warning["code"], "exportClipping", "{warning}");
    assert_eq!(
        warning["loudestSource"]["track"], "Lead",
        "预警必须点名更响的 Lead: {warning}"
    );
    assert_eq!(
        warning["loudestSource"]["node"], lead["node"],
        "点名的 node 必须是 Lead 的 node: {warning}"
    );
    // 最响源的 trim 建议 = 同一个公式作用在它的 peakDbfs 上。
    let lead_dbfs = lead["peakDbfs"].as_f64().expect("lead peakDbfs");
    let expected = -((lead_dbfs * 2.0).ceil() / 2.0);
    let suggested = warning["loudestSource"]["suggestedSourceTrimDb"]
        .as_f64()
        .expect("suggestedSourceTrimDb");
    assert!(
        (suggested - expected).abs() < 1.0e-9,
        "逐源建议必须同一条公式: expected {expected}, actual {suggested}"
    );
}

/// ① 复跑 `#224` 的实测场景：**不加任何推子**，`open_project create` ⇒
/// `propose_section` ⇒ `merge` ⇒ `render_master` —— 响应必须带削波预警。
///
/// 这是本单的**起因**：agent 编完直接导出就会削波，而工具面此前一个字节都不提。
#[test]
fn a_no_fader_project_gets_a_clipping_warning_through_the_tool_chain() {
    let scratch = Scratch::new("no-fader-clip");
    let (mut dispatcher, auth) = fresh_dispatcher();
    let project_path = scratch.join("no-fader.yeban");
    let created = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({
            "path": project_path.display().to_string(),
            "create": true,
            "title": "No Fader",
            "bpm": 120.0,
        }),
    );
    assert_eq!(created["status"], "success", "{created}");
    let proposed = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Intro", "stylePreset": "pop", "bars": 2 }),
    );
    assert_eq!(proposed["status"], "success", "{proposed}");
    let proposal_id = proposed["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "#225 no fader" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");

    let wav = scratch.join("no-fader.wav");
    let rendered = call(
        &mut dispatcher,
        &auth,
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": wav.display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let data = &rendered["data"];
    let warnings = data["warnings"].as_array().expect("warnings");
    println!(
        "### #224 式无推子工程（逐字）: clippedSamples={} truePeakDbfs={} headroomDb={} warnings={}",
        data["clippedSamples"],
        data["truePeakDbfs"],
        data["headroomDb"],
        serde_json::to_string(warnings).expect("warnings")
    );
    assert!(
        data["clippedSamples"].as_u64().unwrap_or(0) > 0,
        "#224 的夹具必须真的削波（否则本条没有前提）: {data}"
    );
    assert_eq!(warnings.len(), 1, "无推子工程必须有一条削波预警: {data}");
    let warning = &warnings[0];
    assert_eq!(warning["code"], "exportClipping", "{warning}");
    assert_eq!(
        warning["clippedSamples"], data["clippedSamples"],
        "{warning}"
    );
    assert!(
        warning["suggestedTrimDb"]
            .as_f64()
            .is_some_and(|value| value < 0.0),
        "必须给出负的 trim 建议: {warning}"
    );
    assert!(
        warning["loudestSource"]["track"].is_string(),
        "必须点名最响的源: {warning}"
    );
    // ⭐ 这条实测很重要：`#224` 那种**多源求和**的工程里，最响的**单条源**
    // 自己可能没过满量程 ⇒ 逐源建议是 `0.0`，`hint` 必须明说"过载来自求和"。
    let loudest_dbfs = warning["loudestSource"]["peakDbfs"].as_f64();
    let source_trim = warning["loudestSource"]["suggestedSourceTrimDb"]
        .as_f64()
        .expect("suggestedSourceTrimDb");
    if loudest_dbfs.is_some_and(|value| value <= 0.0) {
        assert_eq!(
            source_trim, 0.0,
            "单条源没过满量程 ⇒ 逐源 trim 建议必须是 0: {warning}"
        );
        assert!(
            warning["hint"].as_str().expect("hint").contains("多源求和"),
            "必须说清过载来自求和、单独减最响源没用: {warning}"
        );
    }
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#221`：`TrackV3.sampler` 真的被渲染（验收 `#220` 的缺陷 ① ②）
// ---------------------------------------------------------------------------
//
// ## 这一节量什么（对象 ＋ 单位）
//
// | 量 | 来源 | 单位 |
// | :--- | :--- | :--- |
// | `sha256` | `data.sha256`（母带文件字节） | 64 位十六进制 |
// | `peak.before` | `data.peak.before`（抖动/量化**之前**的浮点母带峰值） | 满刻度线性幅度 |
// | `sources[].sampler.label` | 本单新增的 per-track 读数 | 与 app `--export-wav` 同形的 `sfz(...)` 行 |
// | `sources[].kind` | 音频源种类 | `sfz-sampler` / `engine-instrument` |
//
// ## 为什么这三条判据必须一起写
//
// `#220` 的缺陷 ② 是"**绑了采样器与去掉 sampler 导出逐字节相同**，且 `unsupported=[]`"。
// 只断言"能渲染成功"对这条缺陷是**瞎的** —— 修复前它也成功。所以这里的承重断言是
// **不相等**：三个 `sha256` 两两不同、峰值有差异、且每个响应都带 per-track `sfz(...)`。

/// 一条**采样器轨**的测试 SFZ：单区域，`key=69`（夹具音符的音高），指向给定的样本文件。
fn sampler_sfz_body(sample_file: &str) -> String {
    format!("<region> sample={sample_file} key=69 pitch_keycenter=69\n")
}

/// 手写一份 24-bit 单声道 PCM WAV（常量值；⛔ 不用被测写入器自证）。
fn write_constant_wav(path: &Path, value: f64, frames: usize, sample_rate: u32) {
    let scaled = (value * ((1_i32 << 23) - 1) as f64).round() as i32;
    let mut data = Vec::with_capacity(frames * 3);
    for _ in 0..frames {
        let bytes = scaled.to_le_bytes();
        data.extend_from_slice(&bytes[..3]);
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&u32::try_from(36 + data.len()).expect("小").to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // 单声道
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 3).to_le_bytes());
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&24_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data.len()).expect("小").to_le_bytes());
    out.extend_from_slice(&data);
    fs::write(path, &out).expect("写常量样本夹具");
}

/// 造两份**不同**的采样器（A = 0.5 常量样本，B = 0.25 常量样本），返回 `(A, B)` 路径。
///
/// 为什么用常量：峰值可以由测试**独立算出**（0.5 与 0.25），不依赖被测渲染器的任何读数
/// —— 这样"峰值真的变了"不是自我确认。
fn two_samplers(scratch: &Scratch) -> (String, String) {
    let a_dir = scratch.join("sampler-a");
    let b_dir = scratch.join("sampler-b");
    fs::create_dir_all(&a_dir).expect("建 A 目录");
    fs::create_dir_all(&b_dir).expect("建 B 目录");
    write_constant_wav(&a_dir.join("a.wav"), 0.5, 48_000, 48_000);
    write_constant_wav(&b_dir.join("b.wav"), 0.25, 48_000, 48_000);
    fs::write(a_dir.join("a.sfz"), sampler_sfz_body("a.wav")).expect("写 A.sfz");
    fs::write(b_dir.join("b.sfz"), sampler_sfz_body("b.wav")).expect("写 B.sfz");
    (
        a_dir.join("a.sfz").display().to_string(),
        b_dir.join("b.sfz").display().to_string(),
    )
}

/// 把夹具工程的那条 MIDI 轨绑上采样器（`None` = 不绑）。
fn project_with_sampler(sampler: Option<&str>) -> YebanProjectV1 {
    let mut project = project(&Spec::default());
    let lead = id(2);
    project
        .tracks
        .get_mut(&lead)
        .expect("夹具必须有 Lead 轨")
        .sampler = sampler.map(|path| yeban_model::SamplerBinding {
        path: path.to_owned(),
    });
    project.validate().expect("夹具工程必须合法");
    project
}

/// 渲染一次并返回 `(sha256, peakBefore, 响应)`。
fn render_once(scratch: &Scratch, tag: &str, project: &YebanProjectV1) -> (String, f64, Value) {
    let (mut dispatcher, auth) = dispatcher_with(project, &scratch.join(&format!("{tag}.yeban")));
    let out = scratch.join(&format!("{tag}.wav"));
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{tag}: {result}");
    let sha = result["data"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_owned();
    let peak = result["data"]["peak"]["before"]
        .as_f64()
        .expect("peak.before");
    (sha, peak, result)
}

/// ⭐ 判据 3：**同一工程** ① 无 sampler ② 绑 A ③ 绑 B ⇒ 三个 `sha256` 互不相同、
/// 峰值有差异、每次响应都带 per-track `sfz(...)` 读数（`silent=0` / `missing=[]`）。
///
/// 注入（实测红）：把 [`crate::domain::render`] 里 `build_source` 的 `sampler` 段删掉
/// （= 回到 `#220` 缺陷 ② 的代码形状）⇒ `sha256` 三者**全相等**、峰值全相等、
/// `sources[].sampler` 全为 `null` ⇒ 本判据当场红。
#[test]
fn a_bound_sampler_changes_the_export_and_the_per_track_reading_says_so() {
    let scratch = Scratch::new("sampler-three-way");
    let (a, b) = two_samplers(&scratch);
    let no_sampler = project_with_sampler(None);
    let with_a = project_with_sampler(Some(&a));
    let with_b = project_with_sampler(Some(&b));

    let (sha_none, peak_none, none) = render_once(&scratch, "none", &no_sampler);
    let (sha_a, peak_a, resp_a) = render_once(&scratch, "a", &with_a);
    let (sha_b, peak_b, resp_b) = render_once(&scratch, "b", &with_b);

    println!("### 三个 sha256（逐字）: 无sampler={sha_none} 绑A={sha_a} 绑B={sha_b}");
    println!("### 三个峰值: 无sampler={peak_none:.9} 绑A={peak_a:.9} 绑B={peak_b:.9}");
    println!(
        "### A 的 per-track 读数: kind={} label={}",
        resp_a["data"]["sources"][0]["kind"], resp_a["data"]["sources"][0]["sampler"]["label"]
    );
    println!(
        "### B 的 per-track 读数: kind={} label={}",
        resp_b["data"]["sources"][0]["kind"], resp_b["data"]["sources"][0]["sampler"]["label"]
    );

    // ① 三个文件**两两不同**（这是 `#220` 缺陷 ② 的正面证据）。
    assert_ne!(sha_none, sha_a, "绑了采样器的导出必须与'不绑'不同");
    assert_ne!(sha_none, sha_b, "绑了采样器的导出必须与'不绑'不同");
    assert_ne!(sha_a, sha_b, "两个**不同**的采样器必须导出不同的字节");
    // ② 峰值也必须有差异（独立性：sha 会因容器元数据不同而不同，峰值不会）。
    assert_ne!(
        peak_a.to_bits(),
        peak_b.to_bits(),
        "A(0.5 常量) 与 B(0.25 常量) 的峰值必须不同: {peak_a} vs {peak_b}"
    );
    assert_ne!(
        peak_none.to_bits(),
        peak_a.to_bits(),
        "无采样器 vs A 的峰值必须不同"
    );
    // ③ 峰值比例可以**独立算出**：两份样本是 0.5 与 0.25 常量、区域参数完全相同
    //    ⇒ 峰值之比必须是 2.0（`amp_veltrack` 那一档对两个音符是**同一个**乘子，
    //    因此被比例消掉 —— 这正是"不自我确认"的写法）。
    let ratio = peak_a / peak_b;
    assert!(
        (ratio - 2.0).abs() < 1.0e-3,
        "A/B 的样本幅度是 0.5/0.25 ⇒ 峰值之比应当是 2.0, 实测 {ratio}（{peak_a} / {peak_b}）"
    );
    assert!(
        peak_a < 0.5 && peak_b < 0.25,
        "区域增益（含 `amp_veltrack`）只会**缩小**幅度: {peak_a} / {peak_b}"
    );
    // ④ per-track `sfz(...)` 读数：形状与 app 的 `--export-wav` 同形，且 `silent=0`、`missing=[]`。
    for (tag, response) in [("A", &resp_a), ("B", &resp_b)] {
        let sampler = &response["data"]["sources"][0]["sampler"];
        assert!(
            !sampler.is_null(),
            "{tag}: 绑了采样器就必须有 per-track 读数: {response}"
        );
        let label = sampler["label"].as_str().expect("label");
        assert!(
            label.starts_with("sfz(tracks=1 regions=1 samples=1 silent=0"),
            "{tag}: 读数行必须以 sfz(tracks=1 regions=1 samples=1 silent=0 开头: {label}"
        );
        assert!(
            label.contains("missing=[]"),
            "{tag}: 不得有缺失样本: {label}"
        );
        assert!(
            label.contains("unreadable(not_found=0,unsupported=0,other=0)"),
            "{tag}: 读失败三档必须为 0: {label}"
        );
        assert_eq!(
            sampler["notesUsingFileSample"], 1,
            "{tag}: 必须真的用了文件样本"
        );
        assert!(
            sampler.get("notesUsingSineFallback").is_none(),
            "{tag}: ⭐ `#275` 起采样器轨报告**没有**这个键（引擎无正弦回退；\
             回退只在 `yeban_render_sfz_wav` 旁路）: {sampler}"
        );
        assert_eq!(sampler["silentRegionCount"], 0, "{tag}: 本夹具没有内建静音");
    }
    // ⑤ 源种类说真话（`#220` 缺陷 ② 的另一半；`#268` 起 CPU 轨是 `engine-instrument`）。
    assert_eq!(
        resp_a["data"]["sources"][0]["kind"], "sfz-sampler",
        "绑了采样器 ⇒ 源种类必须点名采样器"
    );
    assert_eq!(
        none["data"]["sources"][0]["kind"], "engine-instrument",
        "不绑采样器 ⇒ 走引擎音色（`#268`：正弦振荡器已退役）"
    );
    // ⑥ `sfzSampler` **不得**出现在 `unsupported` 里：它现在真的被渲染了，报它就是假话。
    for (tag, response) in [("none", &none), ("A", &resp_a), ("B", &resp_b)] {
        let unsupported = response["data"]["unsupported"]
            .as_array()
            .expect("unsupported");
        assert!(
            !unsupported.iter().any(|key| key == "sfzSampler"),
            "{tag}: 采样器已经真的被渲染 ⇒ 不许再报 sfzSampler: {unsupported:?}"
        );
    }
    // ⑦ 干净工程（pan=0、无循环、无设备）的 `unsupported` 必须是空数组。
    assert_eq!(
        resp_a["data"]["unsupported"],
        json!([]),
        "绑了采样器的干净工程不该有任何 unsupported 项"
    );
}

/// ⭐ 判据 3b：绑定的路径**读不到** ⇒ 响亮拒绝（⛔ 静默回退成正弦）。
///
/// 这条是"不许静默"的正面判据：`#220` 缺陷 ② 的形状正是"读不到也不说，照常导出"。
#[test]
fn an_unreadable_sampler_path_is_a_loud_failure_not_a_silent_sine() {
    let scratch = Scratch::new("sampler-missing");
    let missing = scratch.join("no-such.sfz").display().to_string();
    let project = project_with_sampler(Some(&missing));
    let (mut dispatcher, auth) = dispatcher_with(&project, &scratch.join("missing.yeban"));
    let out = scratch.join("missing.wav");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_domain_error(&result, "RENDER_FAILED", "采样器路径不存在");
    assert_eq!(
        result["error"]["data"]["reason"], "samplerUnreadable",
        "必须点名是采样器读不到: {result}"
    );
    assert!(
        result["error"]["data"]["tried"].is_array(),
        "必须报出试过哪些路径: {result}"
    );
    assert!(
        !out.exists(),
        "失败时不许留下半成品（⛔ 不许写一个'静音'或'正弦'的替身）"
    );
}

/// 从 MCP 渲染出的 WAV 里读**逐声道峰值**（24-bit 立体声，单位：满刻度线性幅度）。
///
/// 用 [`parse_container`] 定位 `data` 负载（产物带 `bext`，⛔ 不能假设 44 字节头）。
fn channel_peaks(path: &Path) -> (f64, f64) {
    let bytes = fs::read(path).expect("读回渲染产物");
    let container = parse_container(&bytes).expect("产物必须是合法容器");
    assert_eq!(container.format.channels, 2, "判据要立体声产物");
    assert_eq!(container.format.bits_per_sample, 24, "MCP 母带是 24-bit");
    assert!(!container.format.is_float, "MCP 母带是整数 PCM");
    let payload = &bytes[container.data.clone()];
    let scale = f64::from((1_i32 << 23) - 1);
    let mut peaks = [0.0_f64; 2];
    for frame in payload.as_chunks::<6>().0 {
        for (channel, slot) in peaks.iter_mut().enumerate() {
            let base = channel * 3;
            let raw = i32::from(frame[base])
                | (i32::from(frame[base + 1]) << 8)
                | (i32::from(frame[base + 2]) << 16);
            // 24-bit 小端有符号 ⇒ 符号扩展到 i32。
            let sample = (raw << 8) >> 8;
            *slot = slot.max((f64::from(sample) / scale).abs());
        }
    }
    (peaks[0], peaks[1])
}

/// ⭐ 判据 3c（任务单 `#255`）：**采样器轨的 `pan` 真的生效** —— 与 MIDI 轨**同一条**曲线。
///
/// 修前（`#221` 的登记）：采样器源刻意不乘 `pan_gains`，`pan != 0` 时登记
/// `sfzTrackPan` 说"没应用"。`#255` 把 CLI 与 MCP **两侧一起**接上同一条等功率曲线
/// ⇒ 本判据现在钉三件事：
///
/// - `pan = -1` ⇒ 右声道**只剩抖动**（≤ 1 LSB；量化前的浮点右声道逐位 0 由
///   `render::tests::a_hard_left_sampler_source_writes_a_bitwise_zero_right_channel` 钉住）、
///   左声道 > 0；
/// - `pan = -0.5` ⇒ 采样器轨的左右比 == MIDI 轨的左右比 == `tan(3π/8)`；
/// - `unsupported` **不再**出现 `sfzTrackPan`（报一个已经做到的键就是假话）。
#[test]
fn a_sampler_track_with_pan_applies_the_same_curve_as_midi() {
    let scratch = Scratch::new("sampler-pan");
    let (a, _b) = two_samplers(&scratch);
    let lead = id(2);

    // ---- pan = -0.5：采样器轨 ----
    let mut sampler = project_with_sampler(Some(&a));
    sampler.tracks.get_mut(&lead).expect("Lead").pan = -0.5;
    sampler.validate().expect("夹具工程必须合法");
    let (_sha, _peak, response) = render_once(&scratch, "pan-half", &sampler);
    let (sampler_l, sampler_r) = channel_peaks(&scratch.join("pan-half.wav"));

    // ---- pan = -0.5：同一工程、只是不绑采样器 ⇒ MIDI 振荡器路径 ----
    let mut midi = project_with_sampler(None);
    midi.tracks.get_mut(&lead).expect("Lead").pan = -0.5;
    midi.validate().expect("夹具工程必须合法");
    let (_sha_midi, _peak_midi, _resp_midi) = render_once(&scratch, "pan-half-midi", &midi);
    let (midi_l, midi_r) = channel_peaks(&scratch.join("pan-half-midi.wav"));

    // ---- pan = -1：满分离（右声道必须逐位 0）----
    let mut full_left = project_with_sampler(Some(&a));
    full_left.tracks.get_mut(&lead).expect("Lead").pan = -1.0;
    full_left.validate().expect("夹具工程必须合法");
    let (_sha_left, _peak_left, _resp_left) = render_once(&scratch, "pan-full-left", &full_left);
    let (left_l, left_r) = channel_peaks(&scratch.join("pan-full-left.wav"));

    let sampler_ratio = sampler_l / sampler_r;
    let midi_ratio = midi_l / midi_r;
    let expected = (core::f64::consts::FRAC_PI_8).cos() / (core::f64::consts::FRAC_PI_8).sin();
    println!(
        "### 采样器 pan=-0.5: L={sampler_l:.9} R={sampler_r:.9} ratio={sampler_ratio:.9}；\
         MIDI: L={midi_l:.9} R={midi_r:.9} ratio={midi_ratio:.9}；\
         理论 tan(3π/8)={expected:.9}；差={:.3e}",
        (sampler_ratio - midi_ratio).abs()
    );
    println!("### 采样器 pan=-1: L={left_l:.9} R={left_r:.9}（R 必须 == 0）");
    let unsupported = response["data"]["unsupported"]
        .as_array()
        .expect("unsupported");
    println!("### 采样器轨 pan=-0.5 的 unsupported（逐字）: {unsupported:?}");

    assert!(
        sampler_l > 0.0 && sampler_r > 0.0,
        "pan = -0.5 ⇒ 两个声道都必须有信号：L={sampler_l} R={sampler_r}"
    );
    assert!(
        (sampler_ratio - expected).abs() < 1.0e-6,
        "采样器轨的左右比必须落在等功率曲线上：实得 {sampler_ratio:.9} vs {expected:.9}"
    );
    assert!(
        (sampler_ratio - midi_ratio).abs() < 1.0e-6,
        "⭐ 采样器轨与 MIDI 轨必须同一条曲线：{sampler_ratio:.9} vs {midi_ratio:.9}"
    );
    assert!(left_l > 0.0, "pan = -1 ⇒ 左声道必须仍有信号：L={left_l}");
    // ⚠ 母带**一律做 TPDF 抖动**再落 24-bit ⇒ 零声道在文件里是 ±1 LSB 的抖动底噪，
    //    ⛔ 不是"pan 没生效"。量化**之前**的浮点右声道逐位 0 由 `render.rs` 的
    //    `a_hard_left_sampler_source_writes_a_bitwise_zero_right_channel` 钉住；
    //    这里钉的是文件层面"右声道只剩抖动"（≤ 1 LSB）。
    let one_lsb = 1.0_f64 / f64::from((1_i32 << 23) - 1);
    println!(
        "### 采样器 pan=-1 的右声道峰值 = {left_r:.12e}（TPDF 抖动地板 = {one_lsb:.12e}；\
         量化前应为 0）"
    );
    assert!(
        left_r <= one_lsb,
        "⭐ pan = -1 ⇒ 右声道必须只剩抖动（≤ 1 LSB）：R={left_r:.12e} LSB={one_lsb:.12e}"
    );
    assert!(
        !unsupported.iter().any(|key| key == "sfzTrackPan"),
        "⭐ 采样器轨的 pan 已经生效 ⇒ 不许再报 sfzTrackPan: {unsupported:?}"
    );
}

/// 一次 **JSON-RPC 方法**调用（不是 `tools/call`）—— `tools/list` 这类协议方法用。
fn request(dispatcher: &mut Dispatcher, auth: &str, method: &str, params: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "r",
        "method": method,
        "params": params,
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "`{method}` 不得产出 JSON-RPC 层错误: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// "Master + 两条 MIDI 轨 → Master"的工程（判据 2 用它，两条轨绑**不同**的采样器）。
fn two_track_project() -> YebanProjectV1 {
    let mut project = project(&Spec::default());
    let master = id(1);
    let lead = id(2);
    let rhythm = id(3);
    let clip = id(11);
    let placement = id(21);
    let note = id(31);
    let edge = id(41);
    let mut track = TrackV3 {
        id: rhythm,
        name: "Rhythm".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 1_920,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(
        note,
        MidiNote {
            velocity: 100,
            probability: None,
            ratchet: None,
            ..MidiNote::new(note, 0, 69, 1_920)
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "RhythmClip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.tracks.insert(rhythm, track);
    project.routing_graph.nodes = vec![lead, rhythm, master];
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: rhythm,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.validate().expect("两轨夹具必须合法");
    project
}

/// ⭐ 判据 1 ＋ 2：**工具面逐字**（23 个工具、36 个 kind、含 `setTrackSampler`），
/// 且经这条通路给**两轨绑不同采样器**后 `query_project` 逐轨读得回来。
///
/// 注入（实测红）：
/// - 把 `setTrackSampler` 从 `OP_KINDS` 删掉 ⇒ 第一条断言红（kind 表少一个），
///   且 `yeban_edit_notes` 会逐字报 `未知 kind: setTrackSampler`（`#220` 缺陷 ①）；
/// - 把 `compile` 的 `Op::SetTrackSampler` 臂改成别的东西 ⇒ 第二条断言红
///   （`query_project` 的逐轨读数不变）。
#[test]
fn the_tool_surface_binds_two_different_samplers_and_query_reads_them_back() {
    use yeban_mcp::domain::notes::OP_KINDS;
    use yeban_mcp::tools::TOOL_COUNT;

    let scratch = Scratch::new("sampler-bind");
    let (a, b) = two_samplers(&scratch);
    let project = two_track_project();
    let lead = id(2).to_canonical_string();
    let rhythm = id(3).to_canonical_string();
    let (mut dispatcher, auth) = dispatcher_with(&project, &scratch.join("bind.yeban"));

    // ---- ① tools/list：23 个工具，逐字 ----
    let listed = request(&mut dispatcher, &auth, "tools/list", json!({}));
    let tools = listed["tools"].as_array().expect("tools");
    println!(
        "### tools/list 条数（逐字）: {} (TOOL_COUNT={TOOL_COUNT})",
        tools.len()
    );
    assert_eq!(tools.len(), TOOL_COUNT, "工具面条数必须等于 TOOL_COUNT");
    assert_eq!(
        tools.len(),
        23,
        "`#220` 的逐字读数曾是 22；`#290` 新增 `yeban_launch_scene` ⇒ 23"
    );

    // ---- ② `yeban_edit_notes` 的 kind 表：与 `OP_KINDS` 逐字同序 ----
    let edit = tools
        .iter()
        .find(|tool| tool["name"] == "yeban_edit_notes")
        .expect("必须有 yeban_edit_notes");
    let branches = edit["inputSchema"]["properties"]["ops"]["items"]["oneOf"]
        .as_array()
        .expect("ops.items.oneOf");
    let kinds: Vec<&str> = branches
        .iter()
        .map(|branch| branch["title"].as_str().expect("分支 title 是 kind"))
        .collect();
    println!(
        "### edit_notes 的 kind 表（逐字，{} 个）: {kinds:?}",
        kinds.len()
    );
    assert_eq!(kinds.len(), OP_KINDS.len(), "分支数必须等于 OP_KINDS");
    assert_eq!(kinds.len(), 37, "验收 `#220` 的 35 个 ⇒ 后续新增后为 37");
    // ⚠ 发布的**分支顺序**与 `OP_KINDS` 的顺序**不同**（`op_schema::ops_items` 按
    // 可读性分组，`OP_KINDS` 按桶分组；`#177` 的判据只要求"集合相同 + 分支数相同"）
    // ⇒ 这里比**集合**，并按 `OP_KINDS` 的桶序钉住 `setTrackSampler` 紧跟在
    // `setTrackColor` 之后（本单那一处插入点）。
    let mut sorted_kinds = kinds.clone();
    sorted_kinds.sort_unstable();
    let mut sorted_catalog = OP_KINDS.to_vec();
    sorted_catalog.sort_unstable();
    assert_eq!(
        sorted_kinds, sorted_catalog,
        "kind 表必须与 OP_KINDS 集合相同"
    );
    let color_at = kinds
        .iter()
        .position(|kind| *kind == "setTrackColor")
        .expect("setTrackColor");
    assert_eq!(
        kinds[color_at + 1],
        "setTrackSampler",
        "`setTrackSampler` 必须紧跟在 `setTrackColor` 之后（同一个音轨属性桶）"
    );
    assert!(
        kinds.contains(&"setTrackSampler"),
        "`setTrackSampler` 必须真的在 kind 表里"
    );
    let branch = branches
        .iter()
        .find(|branch| branch["title"] == "setTrackSampler")
        .expect("setTrackSampler 分支");
    assert_eq!(
        branch["properties"]["sampler"]["type"],
        json!(["string", "null"])
    );
    assert_eq!(
        branch["required"],
        json!(["kind", "trackId", "sampler"]),
        "三个字段全部必填（缺键 ⛔ 不等于解绑）"
    );
    assert_eq!(
        branch["x-envelope"]["clipId"], "ignored",
        "采样器形态不读顶层 clipId"
    );

    // ---- ③ 经 MCP 给两条轨绑**不同**的采样器（同一次调用） ----
    let edited = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead,
            "ops": [
                {"kind": "setTrackSampler", "trackId": lead, "sampler": a},
                {"kind": "setTrackSampler", "trackId": rhythm, "sampler": b},
            ],
        }),
    );
    assert_eq!(edited["status"], "success", "绑定必须成功: {edited}");
    // ⚠ 生产流的形状：`yeban_edit_notes` **建提案**，`yeban_merge_proposal` 才施加
    // （⛔ 不是"调用返回 success 就等于写进去了" —— 本判据第一版就是这么假绿的）。
    let proposal = edited["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal, "commitMessage": "绑定两轨采样器"}),
    );
    assert_eq!(
        merged["data"]["merged"], true,
        "提案必须真的被施加: {merged}"
    );

    // ---- ④ query_project 逐轨读数：两条轨的 `sampler` 不同且逐字等于给的值 ----
    let queried = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({"fields": ["tracks"]}),
    );
    assert_eq!(queried["status"], "success", "{queried}");
    let tracks = &queried["data"]["project"]["tracks"];
    let lead_path = tracks[&lead]["sampler"]["path"]
        .as_str()
        .unwrap_or_else(|| panic!("Lead 轨必须读得回 sampler: {queried}"));
    let rhythm_path = tracks[&rhythm]["sampler"]["path"]
        .as_str()
        .unwrap_or_else(|| panic!("Rhythm 轨必须读得回 sampler: {queried}"));
    println!("### query_project 逐轨 sampler（逐字）: lead={lead_path} rhythm={rhythm_path}");
    assert_eq!(lead_path, a, "Lead 轨的采样器必须是 A");
    assert_eq!(rhythm_path, b, "Rhythm 轨的采样器必须是 B");
    assert_ne!(lead_path, rhythm_path, "两条轨必须绑不同的采样器");

    // ---- ⑤ 解绑：`sampler: null` ⇒ 该轨的 `sampler` 键**消失**（`None` 是一等状态） ----
    let unbound = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead,
            "ops": [{"kind": "setTrackSampler", "trackId": lead, "sampler": null}],
        }),
    );
    assert_eq!(unbound["status"], "success", "{unbound}");
    let proposal = unbound["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal, "commitMessage": "解绑一轨"}),
    );
    assert_eq!(merged["data"]["merged"], true, "{merged}");
    let queried = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({"fields": ["tracks"]}),
    );
    assert!(
        queried["data"]["project"]["tracks"][&lead]
            .get("sampler")
            .is_none(),
        "解绑后不得再写出 `sampler` 键: {queried}"
    );
    assert_eq!(
        queried["data"]["project"]["tracks"][&rhythm]["sampler"]["path"], b,
        "解绑一轨不得碰另一轨"
    );
}

/// ⭐ 判据 6/7（第 324 条）：**已知不支持项的全集**里的每一项，都必须在一个**具体输入**下
/// 真的出现在 `unsupported` 里。
///
/// 这是"清单不会腐烂"的机械保证：谁删掉某条登记（或把键改名），这条判据就点名它是哪一条。
/// 反方向由 `note_unsupported` 的 `debug_assert` 守住（不许登记清单外的键）。
#[test]
fn the_known_unsupported_list_is_not_decoration() {
    use yeban_mcp::domain::render::KNOWN_UNSUPPORTED_KEYS;

    let scratch = Scratch::new("known-unsupported");
    // `#255`：`sfzTrackPan` 的触发器已删 ⇒ 采样器夹具不再被这条判据使用。
    let _ = two_samplers(&scratch);
    let lead = id(2);

    // 每一项 → 一个**只触发它**的具体输入（其余项在这条输入下不出现）。
    // 一项"已知不支持键 → 只触发它的具体输入"。⚠ 触发器捕获 `scratch` / 夹具路径的
    // 引用 ⇒ `dyn Fn` 必须带生命周期参数（否则会要求 `'static`）。
    #[allow(clippy::type_complexity)]
    let mut triggers: Vec<(&str, Box<dyn Fn() -> Value + '_>)> = Vec::new();
    // ② `deviceChainDsp`：设备链里带本切片没有消费的参数。
    triggers.push((
        "deviceChainDsp",
        Box::new(|| {
            let mut device = device(70, 0, false);
            device.params.push(yeban_model::ParameterValue {
                name: "filter_cutoff_hz".to_owned(),
                value: 1_000.0,
                unit: Some("Hz".to_owned()),
            });
            let spec = Spec {
                devices: vec![device],
                ..Spec::default()
            };
            render_or_ok(&project(&spec), "dsp")
        }),
    ));
    // ③ `externalPlugins`：未旁通的外部乐器。
    triggers.push((
        "externalPlugins",
        Box::new(|| {
            let mut device = device(71, 0, false);
            device.kind = DeviceKind::ExternalInstrument;
            let spec = Spec {
                devices: vec![device],
                ..Spec::default()
            };
            render_or_ok(&project(&spec), "external")
        }),
    ));
    // ④ `automationLanes`：**有可求值点**、但目标落在**没有落点**的地方。
    //    ⭐ `#296`：主总线 `TrackPan` 已有落点、总线轨 `TrackVolume`（可达）也有 ⇒
    //    这里改用**不可达轨**（在图里、但没有任何边通向 master ⇒ 被计划剪枝）上的
    //    `TrackVolume`：它仍如实报 `busTrack` ⇒ `automationLanes`。
    //    ⚠ 空泳道**不**触发（#243 修的第一处谎）：一条没有点的曲线没有“少做了什么”。
    triggers.push((
        "automationLanes",
        Box::new(|| {
            let mut project = project(&Spec::default());
            let ghost = id(7);
            project.tracks.insert(
                ghost,
                TrackV3 {
                    id: ghost,
                    name: "Ghost".to_owned(),
                    kind: TrackKind::Midi,
                    ..TrackV3::default()
                },
            );
            project.routing_graph.nodes.push(ghost);
            let target = AutomationTarget::TrackVolume { track_id: ghost };
            let point = AutomationPoint {
                id: id(120),
                tick: 0,
                value: 0.25,
                curve: CurveType::Linear,
            };
            let lane = AutomationLane {
                target,
                points: BTreeMap::from([(point.id, point)]),
                read_enabled: true,
                write_mode: AutomationWriteMode::Latch,
                domain: None,
            };
            project
                .tracks
                .get_mut(&ghost)
                .expect("Ghost")
                .automation_lanes
                .insert(target, lane);
            project.validate().expect("夹具合法");
            render_or_ok(&project, "automation")
        }),
    ));
    // ⑤ `noteSlide` / ⑥ `notePitchBend` / ⑦ `noteLyrics`：音符上的三种没有实现的表达。
    for (key, mutate) in [("noteSlide", 0u8), ("notePitchBend", 1), ("noteLyrics", 2)] {
        triggers.push((
            key,
            Box::new(move || {
                let mut project = project(&Spec::default());
                for clip in project.clip_pool.values_mut() {
                    if let ClipContent::Midi { notes } = &mut clip.content {
                        for note in notes.values_mut() {
                            match mutate {
                                0 => {
                                    note.slide = Some(yeban_model::SlideConfig {
                                        duration_ticks: 240,
                                        target_pitch: 72,
                                        curve: yeban_model::CurveType::Linear,
                                    });
                                }
                                1 => note.pitch_bend_curve = vec![(0, 0), (240, 200)],
                                _ => note.syllable = Some("la".to_owned()),
                            }
                        }
                    }
                }
                project.validate().expect("夹具合法");
                render_or_ok(&project, key)
            }),
        ));
    }
    // ⑧ `busTrackClips`：主总线轨上摆一个片段。
    triggers.push((
        "busTrackClips",
        Box::new(|| {
            let mut project = project(&Spec::default());
            let master = id(1);
            let clip = id(12);
            let placement = id(22);
            let note = id(32);
            let mut notes = BTreeMap::new();
            notes.insert(note, MidiNote::new(note, 0, 69, 480));
            project.clip_pool.insert(
                clip,
                ClipPoolEntry {
                    id: clip,
                    name: "BusClip".to_owned(),
                    content: ClipContent::Midi { notes },
                },
            );
            project
                .tracks
                .get_mut(&master)
                .expect("Master")
                .clips
                .insert(
                    placement,
                    ClipPlacement {
                        id: placement,
                        clip_id: clip,
                        start_tick: 0,
                        duration_ticks: 480,
                        loop_config: LoopConfig::default(),
                        muted: false,
                    },
                );
            project.validate().expect("夹具合法");
            render_or_ok(&project, "bus")
        }),
    ));
    // ⑨ `sidechainRouting`：路由图里有一条侧链边。
    triggers.push((
        "sidechainRouting",
        Box::new(|| {
            let mut project = project(&Spec::default());
            let edge = id(42);
            project.routing_graph.edges.insert(
                edge,
                RoutingEdge {
                    id: edge,
                    source_node: id(2),
                    destination_node: id(1),
                    kind: RoutingKind::Sidechain,
                    gain_db: None::<f32>,
                },
            );
            project.validate().expect("夹具合法");
            render_or_ok(&project, "sidechain")
        }),
    ));
    // ⑩ `masterPan` **已退役**（`#296`）：主总线 `pan != 0` 现在真的改变母带左右
    //    声道（静态声相走计划的输出级、`TrackPan` 自动化在执行之后乘绝对增益）
    //    ⇒ 清单里没有它、这里也**没有**触发器。
    //    正向判据见 `a_master_pan_lane_moves_the_master_stereo_image`。
    // ⑪ `audioClips`：工程**声明**了资产、会话池里没有它的字节。
    triggers.push((
        "audioClips",
        Box::new(|| {
            let mut project = project(&Spec::default());
            let hash = yeban_model::AssetHash::of_bytes(b"declared-but-absent");
            project.assets.insert(
                hash.clone(),
                yeban_model::AssetMetadata {
                    hash: hash.clone(),
                    original_path: "absent.wav".to_owned(),
                    byte_len: 4,
                    media_kind: yeban_model::MediaKind::Audio,
                    license: "CC0-1.0".to_owned(),
                },
            );
            let clip = id(13);
            let placement = id(23);
            project.clip_pool.insert(
                clip,
                ClipPoolEntry {
                    id: clip,
                    name: "AudioClip".to_owned(),
                    content: ClipContent::Audio {
                        asset: hash,
                        gain_db: 0.0,
                    },
                },
            );
            project.tracks.get_mut(&lead).expect("Lead").clips.insert(
                placement,
                ClipPlacement {
                    id: placement,
                    clip_id: clip,
                    start_tick: 0,
                    duration_ticks: 480,
                    loop_config: LoopConfig::default(),
                    muted: false,
                },
            );
            project.validate().expect("夹具合法");
            render_or_ok(&project, "audio-absent")
        }),
    ));
    // ⑬ `sfzTrackPan` **已退役**（`#255`）：采样器轨的 pan 现在真的生效（与 MIDI 源
    //    共用 `track_pan_gains`），因此清单里没有它、这里也**没有**触发器。
    //    正向判据见 `a_sampler_track_with_pan_applies_the_same_curve_as_midi`。
    // ⑮ `sfzAmplitude`：命中区域声明了 **`amplitude_oncc1`（CC 调制）**。
    //    ⭐ `#274`：静态 `amplitude` 已由引擎的 SFZ 支路应用 ⇒ 那个触发器退役；
    //    仍为真的是 **CC 那一半**（本层没有 CC 状态）。见
    //    `sfz_amplitude_is_reported_only_for_the_cc_half` 的两臂正对照。
    triggers.push((
        "sfzAmplitude",
        Box::new({
            let scratch = &scratch;
            move || {
                let dir = scratch.join("amp-sfz");
                fs::create_dir_all(&dir).expect("建目录");
                write_constant_wav(&dir.join("m.wav"), 0.5, 480, 48_000);
                fs::write(
                    dir.join("m.sfz"),
                    "<region> sample=m.wav key=69 amplitude_oncc1=20\n",
                )
                .expect("写带 amplitude 的 sfz");
                let path = dir.join("m.sfz").display().to_string();
                render_or_ok(&project_with_sampler(Some(&path)), "sfz-amplitude-cc")
            }
        }),
    ));
    // ⑯ `sfzTuneCcModulation`：命中区域声明了 `tune_oncc1`（CC 调制，本层没有 CC 状态）。
    triggers.push((
        "sfzTuneCcModulation",
        Box::new({
            let scratch = &scratch;
            move || {
                let dir = scratch.join("tune-cc-sfz");
                fs::create_dir_all(&dir).expect("建目录");
                write_constant_wav(&dir.join("t.wav"), 0.5, 480, 48_000);
                fs::write(
                    dir.join("t.sfz"),
                    "<region> sample=t.wav key=69 tune_oncc1=50\n",
                )
                .expect("写带 tune_oncc 的 sfz");
                let path = dir.join("t.sfz").display().to_string();
                render_or_ok(&project_with_sampler(Some(&path)), "sfz-tune-cc")
            }
        }),
    ));
    // ⑰ `sfzVolumeCcModulation`：命中区域声明了 `volume_oncc1`（CC 调制，本层没有 CC 状态）。
    triggers.push((
        "sfzVolumeCcModulation",
        Box::new({
            let scratch = &scratch;
            move || {
                let dir = scratch.join("volume-cc-sfz");
                fs::create_dir_all(&dir).expect("建目录");
                write_constant_wav(&dir.join("v.wav"), 0.5, 480, 48_000);
                fs::write(
                    dir.join("v.sfz"),
                    "<region> sample=v.wav key=69 volume_oncc1=-6\n",
                )
                .expect("写带 volume_oncc 的 sfz");
                let path = dir.join("v.sfz").display().to_string();
                render_or_ok(&project_with_sampler(Some(&path)), "sfz-volume-cc")
            }
        }),
    ));
    // 清单与触发器**逐条重合**（两方向都查：少一个触发器 ⇒ 清单腐烂；多一个 ⇒ 清单漏登记）。
    let listed: Vec<&str> = KNOWN_UNSUPPORTED_KEYS.to_vec();
    let covered: Vec<&str> = triggers.iter().map(|(key, _)| *key).collect();
    println!(
        "### KNOWN_UNSUPPORTED_KEYS（逐字，{} 项）: {listed:?}",
        listed.len()
    );
    assert_eq!(covered.len(), listed.len(), "每一项都必须有触发器");
    for key in &listed {
        assert!(covered.contains(key), "`{key}` 没有触发器 ⇒ 清单在腐烂");
    }
    for (key, trigger) in &triggers {
        let response = trigger();
        let unsupported = response["data"]["unsupported"]
            .as_array()
            .unwrap_or_else(|| panic!("{key}: 响应必须带 unsupported: {response}"));
        println!("### `{key}` 的 unsupported（逐字）: {unsupported:?}");
        assert!(
            unsupported.iter().any(|value| value == key),
            "`{key}` 的输入没有把它登记进 unsupported: {unsupported:?}"
        );
        // 反方向：**出现过的每一个键都必须在清单里**（⛔ 不许有清单外的键）。
        // ⚠ 刻意**不**要求"只出现它自己"：有些输入天然同时触发两条
        // （例如未旁通的外部乐器既算 `externalPlugins`、也算 `deviceChainDsp`），
        // 那种重叠是**如实**的，不是漏登记。
        for value in unsupported {
            let observed = value.as_str().expect("unsupported 的元素是字符串");
            assert!(
                listed.contains(&observed),
                "`{key}` 的输入登记了清单外的键 `{observed}` ⇒ KNOWN_UNSUPPORTED_KEYS 少了它"
            );
        }
    }
}

/// ⭐ `#273` 判据（`#274` 的迁移后形态）：**双层区域真的叠加，且不再登记**。
///
/// 两条腿共用同一份渲染路径：
/// * 单层 ⇒ `sfzRegionLayering` **不出现**（触发器不是恒真）；
/// * 双层 ⇒ 也**不出现**（⭐ `#274`：引擎把**每一条**命中区域都建成声部并叠加 ⇒
///   这个键已退役），且 `peak.before` 必须显著高于单层 —— 证明"不报"是因为
///   **真的做到了**，不是因为把输入弄丢了（第 324 条的反向证明）。
#[test]
fn sfz_region_layering_is_applied_and_no_longer_reported() {
    let scratch = Scratch::new("region-layering");
    let dir = scratch.join("region-layering");
    fs::create_dir_all(&dir).expect("建目录");
    write_constant_wav(&dir.join("tone.wav"), 0.5, 480, 48_000);
    let cases = [
        ("single", "<region> sample=tone.wav key=69 volume=-6\n"),
        (
            "layered",
            "<region> sample=tone.wav key=69 volume=-6\n\
             <region> sample=tone.wav key=69 volume=-6\n",
        ),
    ];
    let mut peaks = Vec::new();
    for (tag, text) in cases {
        let sfz = dir.join(format!("{tag}.sfz"));
        fs::write(&sfz, text).expect("写 sfz");
        let path = sfz.display().to_string();
        let response = render_or_ok(&project_with_sampler(Some(&path)), tag);
        let unsupported = response["data"]["unsupported"]
            .as_array()
            .unwrap_or_else(|| panic!("{tag}: 响应必须带 unsupported: {response}"));
        println!("### [{tag}] unsupported（逐字）: {unsupported:?}");
        assert!(
            !unsupported.iter().any(|value| value == "sfzRegionLayering"),
            "⭐ `{tag}`: `sfzRegionLayering` 已退役（引擎叠加全部区域）⇒ 不许再出现: \
             {unsupported:?}"
        );
        peaks.push(
            response["data"]["peak"]["before"]
                .as_f64()
                .unwrap_or_else(|| panic!("{tag}: 必须带 peak.before: {response}")),
        );
    }
    let ratio = peaks[1] / peaks[0];
    println!(
        "### 双层 / 单层 峰值比 = {ratio:.6}（single={:.9} layered={:.9}）",
        peaks[0], peaks[1]
    );
    assert!(
        ratio > 1.5,
        "双层必须真的叠加（两条区域各 -6 dB ⇒ 约 2×）: ratio={ratio}（single={} layered={}）",
        peaks[0],
        peaks[1]
    );
}

/// ⭐ `#274` 判据 6：`sfzAmplitude` **收窄**的两条控制臂。
///
/// * 只写静态 `amplitude=50` ⇒ ⛔ **不出现**（引擎真的应用它 ⇒ 键退役）；
/// * 只写 `amplitude_oncc1=20` ⇒ **出现**（本层没有 CC 状态 ⇒ 那一半仍为真）。
///
/// 两臂的 SFZ 只差一个 opcode ⇒ 任何"恒真 / 恒假"的实现都会有一条腿红。
#[test]
fn sfz_amplitude_is_reported_only_for_the_cc_half() {
    let scratch = Scratch::new("amplitude-two-arms");
    let dir = scratch.join("amplitude-two-arms");
    fs::create_dir_all(&dir).expect("建目录");
    write_constant_wav(&dir.join("a.wav"), 0.5, 480, 48_000);
    let cases = [
        (
            "static",
            "<region> sample=a.wav key=69 amplitude=50\n",
            false,
        ),
        (
            "cc",
            "<region> sample=a.wav key=69 amplitude_oncc1=20\n",
            true,
        ),
    ];
    for (tag, text, expected) in cases {
        let sfz = dir.join(format!("{tag}.sfz"));
        fs::write(&sfz, text).expect("写 sfz");
        let path = sfz.display().to_string();
        let response = render_or_ok(&project_with_sampler(Some(&path)), tag);
        let unsupported = response["data"]["unsupported"]
            .as_array()
            .unwrap_or_else(|| panic!("{tag}: 响应必须带 unsupported: {response}"));
        let present = unsupported.iter().any(|value| value == "sfzAmplitude");
        println!("### [{tag}] unsupported（逐字）: {unsupported:?}");
        assert_eq!(
            present, expected,
            "⭐ `{tag}`: `sfzAmplitude` 的出现与否必须 = {expected}（静态一半退役、CC 一半留）: \
             {unsupported:?}"
        );
    }
}

/// ⭐ `#274` 判据 8：**声部窃取必须可见**（第 324 条：⛔ 不许静默少做）。
///
/// ## 为什么这一条必须有
///
/// `#274` 之前 MCP 的采样器轨是"逐音符烘 PCM"⇒ **永不丢声部**（20 个同时的音符就有
/// 20 条 PCM）。现在它走引擎的 SFZ 声部池（`SFZ_VOICES_PER_TRACK = 16`）⇒ 池满会
/// **窃取**。这是行为变化，必须能在响应里看见；否则用户会听到"少了几个音"而报告里
/// 一个数都没有。
///
/// 夹具：一条采样器轨 ＋ **20 个同起点的不同音高**（`lokey=0 hikey=127` 的一条区域，
/// 每个音符一个声部）⇒ 20 > 16 ⇒ 引擎窃取 4 条。读数进
/// `data.sources[].sampler.voiceSteals`。
///
/// 反证：把 `EngineTrackSource::steal_counter` 的填回删掉 ⇒ `voiceSteals` 恒 `0`，
/// 本判据当场红（打印的两组数字也会一起暴露）。
#[test]
fn a_sampler_track_over_sixteen_voices_reports_its_steals() {
    let scratch = Scratch::new("sampler-steals");
    let dir = scratch.join("steals-sfz");
    fs::create_dir_all(&dir).expect("建目录");
    write_constant_wav(&dir.join("s.wav"), 0.5, 48_000, 48_000);
    fs::write(
        dir.join("s.sfz"),
        "<region> sample=s.wav lokey=0 hikey=127 pitch_keycenter=60 volume=-6\n",
    )
    .expect("写全音域 sfz");
    let path = dir.join("s.sfz").display().to_string();

    // 20 个同起点的音符（音高 60..79，互不相同 ⇒ 每个音符一条声部）。
    let mut project = project(&Spec::default());
    let lead = id(2);
    let clip = id(10);
    let mut notes = BTreeMap::new();
    for index in 0..20u128 {
        let note = id(200 + index as u32);
        notes.insert(
            note,
            MidiNote::new(note, 0, 60 + u8::try_from(index).unwrap(), 480),
        );
    }
    project.clip_pool.get_mut(&clip).expect("夹具片段").content = ClipContent::Midi { notes };
    project.tracks.get_mut(&lead).expect("Lead 轨").sampler =
        Some(yeban_model::SamplerBinding { path: path.clone() });
    project.validate().expect("20 声部夹具必须合法");

    let (_sha, _peak, response) = render_once(&scratch, "steals", &project);
    let sampler = &response["data"]["sources"][0]["sampler"];
    let steals = sampler["voiceSteals"]
        .as_u64()
        .unwrap_or_else(|| panic!("必须带 voiceSteals: {response}"));
    let notes_reported = sampler["notes"]
        .as_u64()
        .unwrap_or_else(|| panic!("必须带 notes: {response}"));
    println!(
        "### 20 声部夹具: notes={notes_reported} voiceSteals={steals} label={}",
        sampler["label"]
    );
    assert_eq!(notes_reported, 20, "20 个音符必须全部排程");
    assert!(
        steals > 0,
        "⭐ 20 > 16 个同时声部 ⇒ 引擎的 SFZ 声部池必须窃取（读数不许恒 0）: {sampler}"
    );
}

/// 渲染一次并要求成功（`unsupported` 判据用；失败即点名叫测试红）。
fn render_or_ok(project: &YebanProjectV1, tag: &str) -> Value {
    let scratch = Scratch::new(&format!("known-{tag}"));
    let (mut dispatcher, auth) = dispatcher_with(project, &scratch.join("p.yeban"));
    let out = scratch.join("out.wav");
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(
        result["status"], "success",
        "`{tag}` 必须渲染成功: {result}"
    );
    result
}

// ===========================================================================
// 任务单 `#243`：音量自动化（`TrackVolume`）在离线导出里真的被求值
// ===========================================================================

/// 给一条轨加一条 `TrackVolume` 泳道（点按 tick 升序插入）。
fn with_volume_lane(
    mut project: YebanProjectV1,
    track_id: EntityId,
    points: &[(u64, f32)],
    read_enabled: bool,
    first_point_seed: u32,
) -> YebanProjectV1 {
    let target = AutomationTarget::TrackVolume { track_id };
    let mut map = BTreeMap::new();
    for (index, (tick, value)) in points.iter().enumerate() {
        let point = AutomationPoint {
            id: id(first_point_seed + index as u32),
            tick: *tick,
            value: *value,
            curve: CurveType::Linear,
        };
        map.insert(point.id, point);
    }
    let lane = AutomationLane {
        target,
        points: map,
        read_enabled,
        write_mode: AutomationWriteMode::Off,
        domain: None,
    };
    project
        .tracks
        .get_mut(&track_id)
        .expect("目标音轨必须存在")
        .automation_lanes
        .insert(target, lane);
    project.validate().expect("带自动化泳道的夹具必须合法");
    project
}

/// 渲染并把磁盘字节解回浮点样本。
fn render_samples(
    scratch: &Scratch,
    tag: &str,
    project: &YebanProjectV1,
) -> (Value, Vec<f32>, String) {
    let (mut dispatcher, auth) = dispatcher_with(project, &scratch.join(&format!("{tag}.yeban")));
    let out = scratch.join(&format!("{tag}.wav"));
    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{tag}: {result}");
    let samples = verify_file_shape(&result, &out);
    let sha = result["data"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_owned();
    (result, samples, sha)
}

/// 4 等段的左声道峰值。
fn four_window_peaks(samples: &[f32], frames: u64) -> [f32; 4] {
    let window = usize::try_from(frames / 4).expect("窗口");
    let mut peaks = [0.0f32; 4];
    for (index, peak) in peaks.iter_mut().enumerate() {
        *peak = window_peak(samples, 2, index * window, window);
    }
    peaks
}

/// 左右两个声道的峰值（交错立体声）。
fn stereo_peaks(samples: &[f32]) -> (f32, f32) {
    let mut left = 0.0f32;
    let mut right = 0.0f32;
    let (pairs, _tail) = samples.as_chunks::<2>();
    for pair in pairs {
        left = left.max(pair[0].abs());
        right = right.max(pair[1].abs());
    }
    (left, right)
}

fn assert_strictly_increasing(peaks: &[f32; 4], context: &str) {
    println!("### {context}: 4 段峰值 = {peaks:?}");
    assert!(
        peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3],
        "{context}: 4 段必须严格单调递增，实测 {peaks:?}"
    );
}

/// ⭐ 判据 1（核心）：`tick 0 = −24 dB → tick 1920 = 0 dB` 的渐强。
///
/// 4 等段 `window_peak` 严格递增，且 `peak[3]/peak[0] >= 3.0`（理论 15.8×，
/// 下界留余量）。
#[test]
fn a_volume_ramp_is_strictly_monotonic_and_the_last_window_is_at_least_three_times_the_first() {
    let scratch = Scratch::new("automation-ramp");
    let lead = id(2);
    let project = with_volume_lane(
        project(&Spec::default()),
        lead,
        &[(0, -24.0), (1_920, 0.0)],
        true,
        120,
    );
    let (result, samples, sha) = render_samples(&scratch, "ramp", &project);
    let frames = result["data"]["frames"].as_u64().expect("frames");
    assert_eq!(frames, 48_000, "1920 tick @120 BPM / 960 PPQ == 1 秒");
    let peaks = four_window_peaks(&samples, frames);
    assert_strictly_increasing(&peaks, "渐强");
    let ratio = peaks[3] / peaks[0];
    println!("### 渐强 peak[3]/peak[0] = {ratio:.6} / sha256={sha}");
    assert!(ratio >= 3.0, "渐强比值 {ratio} 必须 >= 3.0");
    // 读数：这条泳道真的被应用；没有不支持的自动化目标。
    let data = &result["data"];
    assert_eq!(data["automation"]["lanes"], 1);
    assert_eq!(data["automation"]["evaluableLanes"], 1);
    assert_eq!(data["automation"]["appliedLanes"], 1);
    assert_eq!(data["automation"]["unsupportedLanes"], 0);
    assert_eq!(
        data["automation"]["tracks"][0]["lanes"][0]["target"],
        "TrackVolume"
    );
    assert_eq!(data["automation"]["tracks"][0]["lanes"][0]["applied"], true);
    assert!(
        !data["unsupported"]
            .as_array()
            .expect("unsupported")
            .iter()
            .any(|value| value == "automationLanes"),
        "被应用的 TrackVolume 泳道不许再报 automationLanes: {data}"
    );
}

/// ⭐ 判据 2（正对照）：同工程**删掉泳道** ⇒ 4 段峰值相等（相对差 ≤ 1e-6）且
/// `sha256` == 基线 `DEFAULT_MASTER_SHA256` ⇒ 单调性来自泳道。
#[test]
fn without_the_volume_lane_the_four_windows_are_equal_and_the_baseline_digest_is_unchanged() {
    let scratch = Scratch::new("automation-control");
    let project = project(&Spec::default());
    let (result, samples, sha) = render_samples(&scratch, "control", &project);
    let frames = result["data"]["frames"].as_u64().expect("frames");
    let peaks = four_window_peaks(&samples, frames);
    println!(
        "### 正对照 4 段峰值 = {peaks:?} / sha={sha} / digest={}",
        result["data"]["masterDigest"]
    );
    // ⚠ `#268`：引擎音色是**双振荡器**（默认带 detune）⇒ 缓慢拍频让 0.5 s 窗口的
    // 峰值本来就不相等（实测第一段 0.776 vs 其余 0.551，约 √2 的拍频峰）。
    // 因此正对照的判据改为"**没有渐强**"：4 段峰值不得单调递增（渐强判据要求
    // `peak[3]/peak[0] >= 3`，拍频远达不到）。机械锚仍是下面那条 `sha256`/摘要。
    assert!(
        !(peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3]),
        "没有泳道 ⇒ 4 段峰值不得单调递增（拍频不是渐强）: {peaks:?}"
    );
    for (index, peak) in peaks.iter().enumerate() {
        assert!(*peak > 0.1, "正对照第 {index} 段必须真的有声音: {peaks:?}");
    }
    assert_eq!(sha, DEFAULT_MASTER_SHA256, "无自动化路径必须逐字节等于基线");
    assert_eq!(
        result["data"]["masterDigest"], DEFAULT_MASTER_DIGEST,
        "无自动化路径的母带样本摘要必须不变"
    );
}

/// ⭐ 判据 4：`readEnabled:false` 与“删掉泳道”逐字节相同。
#[test]
fn a_disabled_volume_lane_is_byte_identical_to_removing_it() {
    let scratch = Scratch::new("automation-disabled");
    let lead = id(2);
    let project = with_volume_lane(
        project(&Spec::default()),
        lead,
        &[(0, -24.0), (1_920, 0.0)],
        false,
        130,
    );
    let (result, samples, sha) = render_samples(&scratch, "disabled", &project);
    assert_eq!(sha, DEFAULT_MASTER_SHA256, "关掉的泳道必须等于不存在");
    let frames = result["data"]["frames"].as_u64().expect("frames");
    let peaks = four_window_peaks(&samples, frames);
    // ⚠ `#268`：引擎音色的双振荡器拍频让 4 段峰值本来就不相等（与
    // `without_the_volume_lane_*` 同一条读数）⇒ 这里只钉"没有渐强"，机械锚是
    // 上面的 `sha == DEFAULT_MASTER_SHA256`。
    assert!(
        !(peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3]),
        "关掉的泳道不得产生渐强: {peaks:?}"
    );
    assert_eq!(result["data"]["automation"]["appliedLanes"], 0);
    assert_eq!(result["data"]["automation"]["evaluableLanes"], 0);
    assert_eq!(result["data"]["automation"]["lanes"], 1, "读数里仍可见");
}

/// ⭐ 判据 5：所有点值 == 静态推子 ⇒ 与无泳道**逐字节相同**（走快路径，不是乘 1.0）。
#[test]
fn a_lane_that_is_flat_at_the_static_value_is_byte_identical_to_no_lane() {
    let scratch = Scratch::new("automation-flat");
    let lead = id(2);
    // 静态推子非 0（-3 dB）⇒ 恒等判断不能只靠“值是不是 0”，必须真的比较线性增益。
    let baseline = project(&Spec {
        volume_db: -3.0,
        ..Spec::default()
    });
    let (baseline_result, _, baseline_sha) = render_samples(&scratch, "flat-base", &baseline);
    let project = with_volume_lane(
        project(&Spec {
            volume_db: -3.0,
            ..Spec::default()
        }),
        lead,
        &[(0, -3.0), (960, -3.0), (1_920, -3.0)],
        true,
        140,
    );
    let (result, _samples, sha) = render_samples(&scratch, "flat", &project);
    assert_eq!(sha, baseline_sha, "静态恒等必须逐字节相同");
    assert_eq!(
        result["data"]["masterDigest"], baseline_result["data"]["masterDigest"],
        "静态恒等必须逐位相同"
    );
    assert_eq!(result["data"]["automation"]["appliedLanes"], 1);
    assert_eq!(result["data"]["automation"]["unsupportedLanes"], 0);
}

/// ⭐ `#296` 判据（“只对总线挂 gain”）：**普通源轨**上的巨大 `TrackVolume` 泳道
/// 仍走**源侧**，⛔ 不走 `RenderPlan` 的输出级 —— 否则同一份推子会乘两次。
///
/// 夹具：静态 `-20 dB` ＋ 常值 `0 dB` 泳道（R55 比率 10×）⇒ 输出等于“0 dB 静态、
/// 无泳道”的那条轨（峰值相对差 ≤ 1e-4）；若输出级也乘了 `-20 dB`，峰值会掉回
/// 基线的 1/10。
#[test]
fn a_huge_volume_lane_on_an_ordinary_track_stays_source_side() {
    let scratch = Scratch::new("automation-source-side");
    let lead = id(2);
    let quiet = project(&Spec {
        volume_db: -20.0,
        ..Spec::default()
    });
    let (_, quiet_samples, _) = render_samples(&scratch, "source-side-quiet", &quiet);
    let quiet_peak = quiet_samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    let unity = project(&Spec {
        volume_db: 0.0,
        ..Spec::default()
    });
    let (unity_result, unity_samples, _) = render_samples(&scratch, "source-side-unity", &unity);
    let unity_peak = unity_samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    let project = with_volume_lane(
        project(&Spec {
            volume_db: -20.0,
            ..Spec::default()
        }),
        lead,
        &[(0, 0.0), (1_920, 0.0)],
        true,
        220,
    );
    let (result, samples, sha) = render_samples(&scratch, "source-side", &project);
    let peak = samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    println!(
        "### 普通轨 -20 dB + 0 dB 泳道: peak={peak:.9} unity_peak={unity_peak:.9} \
         quiet_peak={quiet_peak:.9} sha={sha} digest={} unity_digest={}",
        result["data"]["masterDigest"], unity_result["data"]["masterDigest"]
    );
    assert!(
        (peak - unity_peak).abs() <= unity_peak * 1e-4,
        "源侧只乘一次 ⇒ 必须回到 0 dB 静态的峰值: peak={peak} unity={unity_peak} quiet={quiet_peak}"
    );
    assert!(
        peak > quiet_peak * 5.0,
        "若输出级又乘了一次静态推子，峰值会掉回基线: peak={peak} quiet={quiet_peak}"
    );
}

/// ⭐ 判据 6：`mute:true` + 一条 0 dB 的音量泳道 ⇒ 仍全静音、`audible:false`。
#[test]
fn a_zero_db_lane_does_not_resurrect_a_muted_track() {
    let scratch = Scratch::new("automation-mute");
    let lead = id(2);
    // 静态推子 -6 dB、静音；泳道 0 dB ⇒ **非恒等**（比率 ≈ 1.995），真的走到自动化乘法。
    let mut muted_baseline = project(&Spec {
        volume_db: -6.0,
        ..Spec::default()
    });
    muted_baseline.tracks.get_mut(&lead).expect("Lead").mute = true;
    let (_, _, baseline_sha) = render_samples(&scratch, "mute-base", &muted_baseline);
    let mut project = project(&Spec {
        volume_db: -6.0,
        ..Spec::default()
    });
    project.tracks.get_mut(&lead).expect("Lead").mute = true;
    let project = with_volume_lane(project, lead, &[(0, 0.0), (1_920, 0.0)], true, 150);
    let (result, samples, sha) = render_samples(&scratch, "mute", &project);
    assert_eq!(result["data"]["sources"][0]["audible"], false);
    let peak = samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    println!("### 静音轨 + 0 dB 泳道: peak={peak} sha={sha}");
    // 量化前是逐位 0；24-bit TPDF 抖动会给全零信号留下 ≤ 1 LSB
    // （1/8388608 = 1.1920929e-7）。判据：≤ 2 LSB 且与无泳道的静音版逐字节相同。
    assert!(
        peak <= 2.0 / 8_388_608.0,
        "静音轨 + 0 dB 泳道必须仍全静音（peak={peak}）"
    );
    assert_eq!(sha, baseline_sha, "自动化不许改变静音轨的字节");
}

/// ⭐ 判据 7：点超出内容末端 ⇒ `frames` 与无泳道相同（自动化不延长母带）。
#[test]
fn points_past_the_end_do_not_extend_the_master() {
    let scratch = Scratch::new("automation-length");
    let lead = id(2);
    let baseline = project(&Spec::default());
    let (baseline_result, _, _) = render_samples(&scratch, "length-base", &baseline);
    let project = with_volume_lane(
        project(&Spec::default()),
        lead,
        &[(0, -12.0), (99_999, 0.0)],
        true,
        160,
    );
    let (result, _, _) = render_samples(&scratch, "length-auto", &project);
    assert_eq!(
        result["data"]["frames"], baseline_result["data"]["frames"],
        "超出末端的点不许延长母带"
    );
}

/// ⭐ 主总线是**第二个站点**：master 上的 `TrackVolume` 泳道也真的改变母带。
#[test]
fn a_master_volume_ramp_is_monotonic_and_applied() {
    let scratch = Scratch::new("automation-master");
    let master = id(1);
    let project = with_volume_lane(
        project(&Spec::default()),
        master,
        &[(0, -24.0), (1_920, 0.0)],
        true,
        170,
    );
    let (result, samples, _sha) = render_samples(&scratch, "master-ramp", &project);
    let frames = result["data"]["frames"].as_u64().expect("frames");
    let peaks = four_window_peaks(&samples, frames);
    assert_strictly_increasing(&peaks, "主总线渐强");
    assert!(peaks[3] / peaks[0] >= 3.0, "{peaks:?}");
    let lanes = &result["data"]["automation"]["tracks"][0]["lanes"];
    assert_eq!(lanes[0]["target"], "TrackVolume");
    assert_eq!(lanes[0]["applied"], true);
    assert_eq!(result["data"]["automation"]["unsupportedLanes"], 0);
}

/// ⭐ 判据 9（第一处谎）：**空泳道不进 `unsupported`**，但读数里可见。
#[test]
fn an_empty_lane_is_visible_in_the_reading_but_not_reported_as_unsupported() {
    let scratch = Scratch::new("automation-empty");
    let master = id(1);
    let project = with_volume_lane(project(&Spec::default()), master, &[], true, 180);
    let (result, _, sha) = render_samples(&scratch, "empty", &project);
    println!(
        "### 空泳道: sha={sha} automation={}",
        result["data"]["automation"]
    );
    assert_eq!(sha, DEFAULT_MASTER_SHA256, "空泳道等于不存在");
    let data = &result["data"];
    assert!(
        !data["unsupported"]
            .as_array()
            .expect("unsupported")
            .iter()
            .any(|value| value == "automationLanes"),
        "空泳道不许进 unsupported: {data}"
    );
    assert_eq!(data["automation"]["lanes"], 1);
    assert_eq!(data["automation"]["evaluableLanes"], 0);
    assert_eq!(data["automation"]["appliedLanes"], 0);
    assert_eq!(data["automation"]["tracks"][0]["lanes"][0]["points"], 0);
    assert_eq!(
        data["automation"]["tracks"][0]["lanes"][0]["evaluable"],
        false
    );
}

/// ⭐ `#296` 判据 4：**静态**主总线 `pan`（没有自动化泳道）也真的被乘 —— 这是
/// `masterPan` 键退役的**依据**（不是"决定不报它"）。
///
/// `master.pan = +0.5` ⇒ 右声道峰值 > 左声道；`sha`/`masterDigest` 与居中基线不同；
/// `unsupported` 里**不再**出现 `masterPan`（它已不在 `KNOWN_UNSUPPORTED_KEYS` 里）。
#[test]
fn a_static_master_pan_is_applied_and_not_reported() {
    let scratch = Scratch::new("master-pan-static");
    let master = id(1);
    let baseline = project(&Spec::default());
    let (base_result, base_samples, base_sha) =
        render_samples(&scratch, "master-pan-static-base", &baseline);
    let (base_left, base_right) = stereo_peaks(&base_samples);
    assert!(
        (base_left - base_right).abs() <= base_left * 1e-4,
        "正对照（居中）左右峰值必须相等: L={base_left} R={base_right}"
    );
    let mut project = baseline;
    project.tracks.get_mut(&master).expect("Master").pan = 0.5;
    project.validate().expect("夹具合法");
    let (result, samples, sha) = render_samples(&scratch, "master-pan-static", &project);
    let (left, right) = stereo_peaks(&samples);
    let data = &result["data"];
    println!(
        "### 静态 masterPan(+0.5): L={left:.9} R={right:.9} ratio={:.6} sha={sha} \
         digest={} baseline_sha={base_sha} baseline_digest={} unsupported={}",
        right / left,
        data["masterDigest"],
        base_result["data"]["masterDigest"],
        data["unsupported"]
    );
    assert!(
        left > 0.0 && right > 0.0,
        "母带必须非静音: L={left} R={right}"
    );
    assert_ne!(left, right, "静态主总线声相必须让左右声道不再相等");
    assert!(right > left, "pan=+0.5 必须右重: L={left} R={right}");
    assert_ne!(sha, base_sha, "静态主总线声相必须真的改变文件字节");
    assert_ne!(
        data["masterDigest"], base_result["data"]["masterDigest"],
        "静态主总线声相必须真的改变母带样本"
    );
    assert!(
        !data["unsupported"]
            .as_array()
            .expect("unsupported")
            .iter()
            .any(|value| value == "masterPan"),
        "masterPan 已退役 ⇒ 不许再出现在 unsupported: {data}"
    );
}

/// ⭐ `#296` 判据 3：**主总线 `TrackPan`** 现在真的移动立体声像（裁决 A）。
///
/// `pan = +0.5` 的常值泳道 ⇒ 右声道峰值 > 左声道（等功率曲线 `θ = 0.375π`：
/// `cos = 0.3827` / `sin = 0.9239`）；正对照（同一工程删掉泳道）左右**相等**。
/// `sha`/`masterDigest` 与基线都必须真的改变 —— 证明不是"登记成支持了却不动音频"。
#[test]
fn a_master_pan_lane_moves_the_master_stereo_image() {
    let scratch = Scratch::new("automation-master-pan");
    let master = id(1);
    let baseline = project(&Spec::default());
    let (base_result, base_samples, base_sha) =
        render_samples(&scratch, "master-pan-base", &baseline);
    let (base_left, base_right) = stereo_peaks(&base_samples);
    // WAV 是 24-bit TPDF 抖动后的字节 ⇒ 左右只差 ≤ 1 LSB（不是逐位相等）。
    assert!(
        (base_left - base_right).abs() <= base_left * 1e-4,
        "正对照（居中）左右峰值必须相等（≤ 1e-4 相对差）: L={base_left} R={base_right}"
    );
    let mut project = baseline.clone();
    let target = AutomationTarget::TrackPan { track_id: master };
    let point = AutomationPoint {
        id: id(190),
        tick: 0,
        value: 0.5,
        curve: CurveType::Linear,
    };
    project
        .tracks
        .get_mut(&master)
        .expect("Master")
        .automation_lanes
        .insert(
            target,
            AutomationLane {
                target,
                points: BTreeMap::from([(point.id, point)]),
                read_enabled: true,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
    project.validate().expect("夹具合法");
    let (result, samples, sha) = render_samples(&scratch, "master-pan-auto", &project);
    let (left, right) = stereo_peaks(&samples);
    let data = &result["data"];
    println!(
        "### master TrackPan(+0.5): L={left:.9} R={right:.9} ratio={:.6} sha={sha} \
         digest={} baseline_sha={base_sha} baseline_digest={} unsupported={} automation={}",
        right / left,
        data["masterDigest"],
        base_result["data"]["masterDigest"],
        data["unsupported"],
        data["automation"]
    );
    assert!(
        left > 0.0 && right > 0.0,
        "母带必须非静音: L={left} R={right}"
    );
    assert_ne!(left, right, "主总线声相必须让左右声道不再相等");
    assert!(right > left, "pan=+0.5 必须右重: L={left} R={right}");
    assert_ne!(sha, base_sha, "主总线声相必须真的改变文件字节");
    assert_ne!(
        data["masterDigest"], base_result["data"]["masterDigest"],
        "主总线声相必须真的改变母带样本"
    );
    assert!(
        !data["unsupported"]
            .as_array()
            .expect("unsupported")
            .iter()
            .any(|value| value == "automationLanes"),
        "主总线 TrackPan 已应用 ⇒ 不许再登记 automationLanes: {data}"
    );
    assert_eq!(data["automation"]["unsupportedLanes"], 0);
    assert_eq!(data["automation"]["tracks"][0]["lanes"][0]["applied"], true);
    assert_eq!(
        data["automation"]["tracks"][0]["lanes"][0]["supported"],
        true
    );
    assert_eq!(
        data["automation"]["tracks"][0]["lanes"][0]["evaluable"],
        true
    );
}

/// `lead → bus → master` 的总线夹具（`bus` 是一条 `AuxReturn`，没有自己的源）。
///
/// 直连 `lead → master`（`id(40)`）被删掉，信号**只**经总线到母带 —— 这样总线的
/// 推子 / mute / solo / 声相读数不会被直达声盖住。
fn bus_project() -> YebanProjectV1 {
    let master = id(1);
    let lead = id(2);
    let bus = id(3);
    let mut project = project(&Spec::default());
    project.tracks.insert(
        bus,
        TrackV3 {
            id: bus,
            name: "Bus".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(bus);
    let to_bus = id(50);
    project.routing_graph.edges.insert(
        to_bus,
        RoutingEdge {
            id: to_bus,
            source_node: lead,
            destination_node: bus,
            kind: RoutingKind::SendToAux,
            gain_db: None,
        },
    );
    let from_bus = id(51);
    project.routing_graph.edges.insert(
        from_bus,
        RoutingEdge {
            id: from_bus,
            source_node: bus,
            destination_node: master,
            kind: RoutingKind::BusToMaster,
            gain_db: None,
        },
    );
    project.routing_graph.edges.remove(&id(40));
    project.validate().expect("总线夹具合法");
    project
}

/// 给一条轨加一条 `TrackPan` 泳道（点按 tick 升序插入）。
fn with_pan_lane(
    mut project: YebanProjectV1,
    track_id: EntityId,
    points: &[(u64, f32)],
    read_enabled: bool,
    first_point_seed: u32,
) -> YebanProjectV1 {
    let target = AutomationTarget::TrackPan { track_id };
    let mut map = BTreeMap::new();
    for (index, (tick, value)) in points.iter().enumerate() {
        let point = AutomationPoint {
            id: id(first_point_seed + index as u32),
            tick: *tick,
            value: *value,
            curve: CurveType::Linear,
        };
        map.insert(point.id, point);
    }
    let lane = AutomationLane {
        target,
        points: map,
        read_enabled,
        write_mode: AutomationWriteMode::Off,
        domain: None,
    };
    project
        .tracks
        .get_mut(&track_id)
        .expect("目标音轨必须存在")
        .automation_lanes
        .insert(target, lane);
    project.validate().expect("带声相泳道的夹具必须合法");
    project
}

/// 左右声道在 `[start, start + frames)` 内的 RMS（独立实现，单位：样本）。
fn stereo_window_rms(samples: &[f32], channels: usize, start: usize, frames: usize) -> (f64, f64) {
    let available = samples.len() / channels;
    let mut left = 0.0f64;
    let mut right = 0.0f64;
    let mut count = 0usize;
    for frame in start..(start + frames).min(available) {
        let l = f64::from(samples[frame * channels]);
        let r = f64::from(samples[frame * channels + 1]);
        left += l * l;
        right += r * r;
        count += 1;
    }
    if count == 0 {
        return (0.0, 0.0);
    }
    ((left / count as f64).sqrt(), (right / count as f64).sqrt())
}

/// ⭐ B2：**中性总线**（`pan = 0` / `mute = solo = false` / 无泳道）的总线输出级是
/// 恒等 ⇒ `lead → bus → master` 与 `lead → master` **逐字节相同**。
///
/// 这是"B2 不改既有工程"的机械锚：只要总线的三个静态字段保持中性、且没有
/// `TrackPan` 泳道，B2 的输出级就一个样本都不碰。
#[test]
fn a_neutral_bus_is_bit_identical_to_the_direct_baseline() {
    let scratch = Scratch::new("bus-neutral");
    let (result, _samples, sha) = render_samples(&scratch, "neutral-bus", &bus_project());
    println!(
        "### 中性总线: sha={sha} digest={}（直连基线 {DEFAULT_MASTER_SHA256} / {DEFAULT_MASTER_DIGEST}）",
        result["data"]["masterDigest"]
    );
    assert_eq!(
        sha, DEFAULT_MASTER_SHA256,
        "中性总线的整文件 sha 必须等于直连基线"
    );
    assert_eq!(
        result["data"]["masterDigest"], DEFAULT_MASTER_DIGEST,
        "中性总线的母带样本摘要必须等于直连基线"
    );
}

/// ⭐ B2b：总线 `mute = true` ⇒ **整条总线**输出为 0（入边 ＋ 自有源一起静音）。
#[test]
fn a_bus_mute_silences_the_whole_bus() {
    let scratch = Scratch::new("bus-mute");
    let (base, _samples, base_sha) = render_samples(&scratch, "mute-base", &bus_project());
    let mut project = bus_project();
    project.tracks.get_mut(&id(3)).expect("Bus").mute = true;
    project.validate().expect("静音总线夹具合法");
    let (result, samples, sha) = render_samples(&scratch, "mute-on", &project);
    let muted_peak = peak(&samples);
    let frames = result["data"]["frames"].as_u64().expect("frames");
    let muted_windows = four_window_peaks(&samples, frames);
    println!(
        "### 总线 mute: base_sha={base_sha} sha={sha} peak={muted_peak:.9}          four_windows={muted_windows:?} base_peak={:.9}",
        peak(&_samples)
    );
    assert!(
        muted_peak <= DITHER_ONLY_PEAK,
        "总线 mute ⇒ 输出只剩抖动噪声: peak={muted_peak}"
    );
    assert_ne!(sha, base_sha, "总线 mute 必须真的改变文件字节");
    assert_ne!(result["data"]["masterDigest"], base["data"]["masterDigest"]);
}

/// ⭐ B2c：一条源轨 solo ⇒ **未 solo 的总线**静音；总线 `solo_safe = true` ⇒ 仍可闻。
///
/// 两条读数合起来钉住既有语义 `audible = !mute && (!any_solo || solo || solo_safe)`：
/// 没有 `solo_safe` 时总线被 `any_solo` 门掉，加了 `solo_safe` 后母带与"无 solo 的
/// 正对照"**逐字节相同**。
#[test]
fn a_bus_is_muted_by_another_tracks_solo_unless_it_is_solo_safe() {
    let scratch = Scratch::new("bus-solo");
    let (_base, _base_samples, base_sha) = render_samples(&scratch, "solo-base", &bus_project());

    let mut soloed = bus_project();
    soloed.tracks.get_mut(&id(2)).expect("Lead").solo = true;
    soloed.validate().expect("solo 夹具合法");
    let (_solo_result, solo_samples, solo_sha) = render_samples(&scratch, "solo-on", &soloed);
    let solo_peak = peak(&solo_samples);
    println!("### 源轨 solo / 总线未 solo: sha={solo_sha} peak={solo_peak:.9}");
    assert!(
        solo_peak <= DITHER_ONLY_PEAK,
        "未 solo 的总线必须被 any_solo 门掉: peak={solo_peak}"
    );
    assert_ne!(solo_sha, base_sha, "总线被 solo 门掉必须改变文件字节");

    let mut safe = bus_project();
    safe.tracks.get_mut(&id(2)).expect("Lead").solo = true;
    safe.tracks.get_mut(&id(3)).expect("Bus").solo_safe = true;
    safe.validate().expect("solo_safe 夹具合法");
    let (_safe_result, safe_samples, safe_sha) = render_samples(&scratch, "solo-safe", &safe);
    println!(
        "### 源轨 solo / 总线 solo_safe: sha={safe_sha} peak={:.9}",
        peak(&safe_samples)
    );
    assert!(
        peak(&safe_samples) > 0.1,
        "solo_safe 的总线必须仍可闻: peak={}",
        peak(&safe_samples)
    );
    assert_eq!(
        safe_sha, base_sha,
        "solo_safe 的总线 + 唯一源轨 solo ⇒ 与无 solo 正对照逐字节相同"
    );
}

/// ⭐ B2a 静态：总线 `pan = -0.5` ⇒ 整条总线左重；比值 = `tan(3π/8)`。
#[test]
fn a_bus_static_pan_moves_the_whole_bus() {
    let scratch = Scratch::new("bus-pan-static");
    let mut project = bus_project();
    project.tracks.get_mut(&id(3)).expect("Bus").pan = -0.5;
    project.validate().expect("声相夹具合法");
    let (result, samples, sha) = render_samples(&scratch, "pan-static", &project);
    let (left, right) = stereo_peaks(&samples);
    let ratio = left / right;
    let expected = (3.0_f32 * core::f32::consts::FRAC_PI_8).tan();
    println!(
        "### 总线静态 pan(-0.5): L={left:.9} R={right:.9} ratio={ratio:.9} \
         expected={expected:.9} sha={sha} digest={}",
        result["data"]["masterDigest"]
    );
    assert!(
        (ratio - expected).abs() < 1.0e-4,
        "总线 pan(-0.5) 的 L/R 必须等于 tan(3π/8)={expected}: ratio={ratio}"
    );
}

/// ⭐ B2a 泳道：总线的 `TrackPan` 泳道（`-1 → +1`）在输出级真的被应用。
#[test]
fn a_bus_pan_lane_is_applied_at_the_output_stage() {
    let scratch = Scratch::new("bus-pan-lane");
    let bus = id(3);
    let project = with_pan_lane(bus_project(), bus, &[(0, -1.0), (1_920, 1.0)], true, 300);
    let (result, samples, sha) = render_samples(&scratch, "pan-lane", &project);
    let data = &result["data"];
    let frames = usize::try_from(data["frames"].as_u64().expect("frames")).expect("frames");
    // 只看两端的 1/4：中间 1/2 正在扫过中心，会把 L/R 比拉平（实测 2.198，不是判据坏了）。
    let quarter = frames / 4;
    let (early_l, early_r) = stereo_window_rms(&samples, 2, 0, quarter);
    let (late_l, late_r) = stereo_window_rms(&samples, 2, frames - quarter, quarter);
    println!(
        "### 总线 TrackPan 泳道: early L/R={early_l:.9}/{early_r:.9} \
         late L/R={late_l:.9}/{late_r:.9} sha={sha} automation={}",
        data["automation"]
    );
    assert!(
        early_l > early_r * 3.0,
        "泳道起点(-1)必须左重: L={early_l} R={early_r}"
    );
    assert!(
        late_r > late_l * 3.0,
        "泳道终点(+1)必须右重: L={late_l} R={late_r}"
    );
    assert_eq!(data["automation"]["lanes"], 1);
    assert_eq!(data["automation"]["evaluableLanes"], 1);
    assert_eq!(data["automation"]["appliedLanes"], 1);
    assert_eq!(data["automation"]["unsupportedLanes"], 0);
    let lane = &data["automation"]["tracks"][0]["lanes"][0];
    assert_eq!(lane["target"], "TrackPan");
    assert_eq!(lane["supported"], true);
    assert_eq!(lane["applied"], true);
    assert!(
        !data["unsupported"]
            .as_array()
            .expect("unsupported")
            .iter()
            .any(|value| value == "automationLanes"),
        "可达总线的声相泳道已应用 ⇒ 不许再登记 automationLanes: {data}"
    );
}

/// ⭐ `#296` 判据 1：**可达总线轨**的 `TrackVolume` 泳道在**输出级**真的被乘。
///
/// 夹具与 `#257` 的 bus-only 判据同一个（`lead → bus → master`），但这次总线**能到达
/// 母带** ⇒ `supported:true`、`sha`/`masterDigest` 真的改变、4 段峰值按
/// `-24 dB → 0 dB` 单调上升。正对照（同工程删掉泳道）的 `sha` 是三段里唯一不动的。
#[test]
fn a_reachable_bus_volume_lane_is_applied_at_the_output_stage() {
    let scratch = Scratch::new("automation-bus");
    let master = id(1);
    let lead = id(2);
    let bus = id(3);
    let mut project = project(&Spec::default());
    project.tracks.insert(
        bus,
        TrackV3 {
            id: bus,
            name: "Bus".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(bus);
    let to_bus = id(50);
    project.routing_graph.edges.insert(
        to_bus,
        RoutingEdge {
            id: to_bus,
            source_node: lead,
            destination_node: bus,
            kind: RoutingKind::SendToAux,
            gain_db: None,
        },
    );
    let from_bus = id(51);
    project.routing_graph.edges.insert(
        from_bus,
        RoutingEdge {
            id: from_bus,
            source_node: bus,
            destination_node: master,
            kind: RoutingKind::BusToMaster,
            gain_db: None,
        },
    );
    // ⭐ 让信号**只**经总线到母带：删掉默认夹具的 `lead → master` 直连，否则
    // 直达声会盖住总线推子的读数（单调性测不出来）。`lead → bus → master` 保留。
    project.routing_graph.edges.remove(&id(40));
    project.validate().expect("总线夹具合法");
    // 正对照：同一张图、**没有**泳道（总线静态 0 dB）。
    let baseline = project.clone();
    let (base_result, base_samples, base_sha) = render_samples(&scratch, "bus-base", &baseline);
    let base_frames = base_result["data"]["frames"].as_u64().expect("frames");
    let base_peaks = four_window_peaks(&base_samples, base_frames);
    // 被测：总线轨上 `-24 dB → 0 dB` 的 TrackVolume 泳道。
    let project = with_volume_lane(project, bus, &[(0, -24.0), (1_920, 0.0)], true, 200);
    let (result, samples, sha) = render_samples(&scratch, "bus-auto", &project);
    let data = &result["data"];
    let frames = data["frames"].as_u64().expect("frames");
    let peaks = four_window_peaks(&samples, frames);
    assert_strictly_increasing(&peaks, "总线推子渐强");
    assert!(
        peaks[3] / peaks[0] >= 3.0,
        "总线推子渐强比值必须 >= 3.0: {peaks:?}"
    );
    // ⚠ 末段峰值与静态基线**不逐字相等**：音符包络在 1 s 末段已经衰减，而 4 等段
    // 峰值取的是窗口内的最大值 ⇒ `base_peaks` 只作为读数报出，不拿它当判据。
    println!(
        "### 可达总线 TrackVolume: peaks={peaks:?} base_peaks={base_peaks:?} sha={sha} \
         digest={} base_sha={base_sha} base_digest={} unsupported={} automation={}",
        data["masterDigest"],
        base_result["data"]["masterDigest"],
        data["unsupported"],
        data["automation"]
    );
    assert_ne!(sha, base_sha, "总线推子必须真的改变文件字节");
    assert_ne!(
        data["masterDigest"], base_result["data"]["masterDigest"],
        "总线推子必须真的改变母带样本"
    );
    assert!(
        !data["unsupported"]
            .as_array()
            .expect("unsupported")
            .iter()
            .any(|value| value == "automationLanes"),
        "可达总线的音量泳道已应用 ⇒ 不许再登记 automationLanes: {data}"
    );
    assert_eq!(data["automation"]["unsupportedLanes"], 0);
    let bus_report = data["automation"]["tracks"]
        .as_array()
        .expect("tracks")
        .iter()
        .find(|track| track["node"] == bus.to_canonical_string())
        .expect("读数必须包含 bus 轨");
    assert_eq!(bus_report["lanes"][0]["target"], "TrackVolume");
    assert_eq!(bus_report["lanes"][0]["evaluable"], true);
    assert_eq!(bus_report["lanes"][0]["applied"], true);
    assert_eq!(bus_report["lanes"][0]["supported"], true);
}

/// ⭐ 判据 10：同工程两次渲染的 `masterDigest` 相同（自动化不引入熵源）。
#[test]
fn volume_automation_is_deterministic_across_two_renders() {
    let scratch = Scratch::new("automation-determinism");
    let lead = id(2);
    let project = with_volume_lane(
        project(&Spec::default()),
        lead,
        &[(0, -24.0), (1_920, 0.0)],
        true,
        210,
    );
    let (first, _, sha_a) = render_samples(&scratch, "det-a", &project);
    let (second, _, sha_b) = render_samples(&scratch, "det-b", &project);
    println!(
        "### 确定性: sha_a={sha_a} sha_b={sha_b} digest_a={} digest_b={}",
        first["data"]["masterDigest"], second["data"]["masterDigest"]
    );
    assert_eq!(sha_a, sha_b, "两次渲染必须逐字节相同");
    assert_eq!(
        first["data"]["masterDigest"], second["data"]["masterDigest"],
        "两次渲染的母带摘要必须相同"
    );
}

/// ⭐ `#247` 端到端判据：**同一 tick 表**在 44.1 / 48 / 96 kHz 上收口后，
/// 48/96 kHz 的三件套（`frames` / `masterDigest` / `sha256`）与收口前**逐字相同**，
/// 而 44.1 kHz 的 `frames` 从 7717 变为 7718（母带摘要与文件摘要随之改变）。
///
/// 夹具：60 BPM / 960 PPQ，一条 MIDI 音符与摆放都是 `0..168` tick。
/// `168 × 45.9375 = 7717.5` ⇒ 半值远离零 ⇒ **7718**；旧的 MCP 求值顺序
/// （`tick × 60 / ppq / bpm × sr`）在浮点上得 `7717.4999999999995` ⇒ 7717。
/// 48 kHz 是 8400 帧、96 kHz 是 16800 帧，两种顺序都逐位相同。
///
/// 可被什么注入弄红：
/// - 把 `yeban_model::time_math::ticks_to_frames` 的求值顺序改回旧 MCP 顺序
///   ⇒ 44.1 kHz 的 `frames` 变回 7717；
/// - 改坏模型层的 `round`/`samples_per_tick` ⇒ 48/96 kHz 的摘要也变。
#[test]
fn tick_conversion_is_shared_and_keeps_the_48_and_96_khz_summaries() {
    let expected = [
        (
            44_100u32,
            7_718u64,
            "59fd7b37dc94696e9bb81164698a897cfcad7da7b010cf48baad2bfd5b7fb487",
            // `#255`: 奇数长度 CodingHistory 的 bext 长度字段 +1（样本摘要不变）。
            "0c6e61e735f25343a3543b7ff3b2bc87d497668df5d602b1ba7448231be95cbc",
        ),
        (
            48_000,
            8_400,
            "74478758b3953b5e5c6a509a65ed81b51354e4466bdde18cae89136d7c668724",
            // `#255`: 奇数长度 CodingHistory 的 bext 长度字段 +1（样本摘要不变）。
            "0ce848174c76055a4ad19977620feb2387319aaa3d218d6b52102a05f221b74b",
        ),
        (
            96_000,
            16_800,
            "99b44cb15615db342bf39ca06dbb289ca2c19a58977dd0a7ef49403906d895ba",
            // `#255`: 奇数长度 CodingHistory 的 bext 长度字段 +1（样本摘要不变）。
            "3a50bdd3743b878a25e6b7cb82ed61464c12277feea441dfd349f8be1c5cb6b0",
        ),
    ];
    for (rate, frames, digest, sha256) in expected {
        let scratch = Scratch::new("tickconv");
        let spec = Spec {
            bpm: 60.0,
            end_tick: 168,
            ..Spec::default()
        };
        let (mut dispatcher, auth) = dispatcher_with(&project(&spec), &scratch.join("demo.yeban"));
        let out = scratch.join("master.wav");
        let result = call(
            &mut dispatcher,
            &auth,
            json!({"format": "wav", "sampleRate": rate, "path": out.display().to_string()}),
        );
        assert_eq!(result["status"], "success", "{result}");
        let data = &result["data"];
        assert_eq!(data["frames"], frames, "{rate} Hz 的帧数");
        assert_eq!(data["masterDigest"], digest, "{rate} Hz 的母带摘要");
        assert_eq!(data["sha256"], sha256, "{rate} Hz 的文件摘要");
    }
}

/// ⭐ `#268`（`ADR-0011`）：引擎音源装配失败的 `reason` 必须留在**机器可读词表**里。
///
/// 这条映射是**防御性**的：合法工程上，`build` 的探针编译会先拒绝同一批路由错误
/// （没有主总线 / 路由图非法），`EngineSnapshot::from_project` 因此拿不到一个
/// 只在它那里失败、而计划编译通过的工程。即便如此，⛔ 不能让它退化成一条**没有
/// `reason`** 的失败 —— "引擎装不上"必须能被调用方按词分类。
#[test]
fn the_engine_source_failure_reason_is_registered_in_the_vocabulary() {
    let vocabulary = include_str!("fault_vocabulary.rs");
    assert!(
        vocabulary.contains("GOLDEN_REASONS") && vocabulary.contains("\"engineSource\""),
        "`engineSource` 必须登记在 tests/fault_vocabulary.rs 的黄金表里"
    );
}
