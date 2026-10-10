//! **音频片段渲染进母带**的端到端判据 [MCP-TOOL-008, ROAD-M4-005, ARCH-DSP-002, ARCH-PDC-001]。
//!
//! 这一份判据回答的是 `line/mcp-render` 留在能力矩阵里的那一格：
//! `audioClips` 曾经是 `unsupported`（CAS 资产字节根本没被消费）。现在
//! `ClipContent::Audio` 真的进母带，因此这里逐条钉住：
//!
//! | # | 判据 | 被什么注入破坏 |
//! | :--- | :--- | :--- |
//! | 1 | 素材样本**真的**出现在母带里（容差 = 抖动 + 量化） | 静音源/丢弃片段 |
//! | 2 | 片段**落在 placement 的帧位置**，起止 ≤ 1 帧 | tick→帧换算错、落位 +1 |
//! | 3 | `mute` / `solo` / 片段 `gain_db` 门控可测 | 门控被绕过 |
//! | 4 | 采样率不一致 ⇒ **真重采样**且时长不变 | 改标签/丢帧（假重采样） |
//! | 5 | 采样率不一致**不是**错误（旧的 `unwired` 拒绝已退役） | 退回"一票拒绝" |
//! | 6 | 两次渲染逐字节相同 + **钉死摘要常量** | 引入熵源 |
//! | 7 | 缺资产（索引与池里都没有）⇒ `assetMissing` | 静默当静音 |
//! | 8 | 池里的字节与声明的哈希不符 ⇒ `assetHashMismatch` | 关掉完整性校验 |
//! | 9 | 声明了但池里没有字节 ⇒ 登记 `audioClips`（不假装渲染） | 静默当"渲染过" |
//! | 10 | 坏/截断资产 ⇒ 契约内错误码，**不 panic** | 吞成静音 |
//! | 11 | `dryRun` 不落盘且给出同一批实测数字 | dryRun 真写盘 |
//! | 12 | 同 `idempotencyKey` 不重复渲染 | 关掉幂等缓存 |
//! | 13 | PDC：带延迟的音轨与零延迟音轨实测差 `L_max` 帧 | 音频片段绕过延迟表 |
//!
//! ## 夹具纪律（与 `render_master.rs` 一致）
//!
//! - 素材**由代码生成**（仓库里不放音频文件，AGENTS.md §2 红线 9）；
//! - 一律用 **32-bit float WAV**：`yeban-decode` 的台账记载它对 f32 WAV 是
//!   **逐位透传**（`docs/ledger/decode-core-notes.md` §3.2），因此"母带里的样本"
//!   与"我写进去的样本"可以直接比 —— 没有"解码器缩放口径"这一层不确定性；
//! - 临时目录全部落在 `std::env::temp_dir()`，[`Scratch`] 的 `Drop` 负责删除。

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
use yeban_mcp::tools::ErrorCode;
use yeban_model::{
    AssetHash, AssetMetadata, ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition,
    DeviceKind, EntityId, MediaKind, RoutingEdge, RoutingKind, SampleRate, TrackKind, TrackV3,
    YebanProjectV1,
};
use yeban_render::rf64::parse_container;

/// 判据用的固定注入时钟（与 `render_master.rs` 同一个值 ⇒ 两份台账可比）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 120 BPM / 960 PPQ 下 **1 秒** = 1920 tick。
const ONE_SECOND_TICKS: u64 = 1_920;

// ---------------------------------------------------------------------------
// 夹具脚手架
// ---------------------------------------------------------------------------

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
            "yeban-mcp-audio-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 目录里的条目名（判"dryRun 一个字节都没写"用）。
    fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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

// ---------------------------------------------------------------------------
// 素材夹具：代码生成的 WAV 字节（独立实现，不用生产代码）
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
///
/// `samples` 是**交织**的 f32；`channels` 必须整除它的长度。
fn wav_f32(channels: u16, sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        payload.extend_from_slice(&sample.to_le_bytes());
    }
    wav_bytes(channels, sample_rate, 3, 32, &payload)
}

/// 16-bit PCM WAV（tag 1）。
fn wav_s16(channels: u16, sample_rate: u32, samples: &[i16]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        payload.extend_from_slice(&sample.to_le_bytes());
    }
    wav_bytes(channels, sample_rate, 1, 16, &payload)
}

/// 一段**逐位可复现**的立体声素材：两个声道用不同的阶梯，便于发现声道串位。
///
/// 取值全部是 1/64 的整数倍 ⇒ 在 f32 与 24-bit 里**精确可表示**，
/// 因此"母带里的样本"与"写进去的样本"之间的差只可能来自抖动与量化。
fn pattern(frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);
    for index in 0..frames {
        let left = ((index % 64) as f32) / 64.0 - 0.5;
        let right = 0.25 - ((index % 32) as f32) / 64.0;
        out.push(left);
        out.push(right);
    }
    out
}

/// 恒定电平的立体声素材（PDC 判据用它：电平本身就是一个可读的"事件"）。
fn constant(frames: usize, level: f32) -> Vec<f32> {
    vec![level; frames * 2]
}

// ---------------------------------------------------------------------------
// 工程夹具
// ---------------------------------------------------------------------------

/// 夹具旋钮。默认值 = "一个 48 kHz 立体声片段摆在 0 tick、时值 1 秒、增益 0 dB"。
struct Tune {
    /// 片段增益 (dB)。
    clip_gain_db: f32,
    /// placement 起始 tick。
    start_tick: u64,
    /// placement 时值（tick）。
    duration_ticks: u64,
    /// placement 是否静音。
    placement_muted: bool,
    /// 音轨是否静音。
    track_mute: bool,
    /// 音轨是否独奏。
    track_solo: bool,
    /// 是否往会话 CAS 池里放字节。
    provide_bytes: bool,
    /// 是否在工程 `assets` 索引里声明。
    declare_in_index: bool,
    /// 音轨设备链的延迟（帧）。
    device_latency: u32,
}

impl Default for Tune {
    fn default() -> Self {
        Self {
            clip_gain_db: 0.0,
            start_tick: 0,
            duration_ticks: ONE_SECOND_TICKS,
            placement_muted: false,
            track_mute: false,
            track_solo: false,
            provide_bytes: true,
            declare_in_index: true,
            device_latency: 0,
        }
    }
}

/// 一份 "Master + 一条音轨 → Master" 的工程，带一个音频片段。
struct Fixture {
    project: YebanProjectV1,
    asset: AssetHash,
    bytes: Option<Vec<u8>>,
}

/// 用给定的素材字节构造夹具工程。
fn fixture(bytes: Vec<u8>, tune: &Tune) -> Fixture {
    let asset = AssetHash::of_bytes(&bytes);
    let master = id(1);
    let lead = id(2);
    let clip = id(10);
    let placement = id(20);
    let edge = id(40);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "audio clip fixture".to_owned(),
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
    let devices = if tune.device_latency == 0 {
        Vec::new()
    } else {
        vec![DeviceDefinition {
            id: id(101),
            name: "lat".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: Vec::new(),
            latency_samples: tune.device_latency,
        }]
    };
    let mut track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Audio,
        volume_db: 0.0,
        pan: 0.0,
        mute: tune.track_mute,
        solo: tune.track_solo,
        devices,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: tune.start_tick,
            duration_ticks: tune.duration_ticks,
            muted: tune.placement_muted,
            ..ClipPlacement::default()
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Kick".to_owned(),
            content: ClipContent::Audio {
                asset: asset.clone(),
                gain_db: tune.clip_gain_db,
            },
        },
    );
    if tune.declare_in_index {
        project.assets.insert(
            asset.clone(),
            AssetMetadata {
                hash: asset.clone(),
                original_path: "samples/kick.wav".to_owned(),
                byte_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                media_kind: MediaKind::Audio,
                license: "CC0-1.0".to_owned(),
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
    Fixture {
        project,
        asset,
        bytes: tune.provide_bytes.then_some(bytes),
    }
}

/// 会话 CAS 池：`bytes` 为 `None` 时是空池（"内存注入的会话"形态）。
fn pool(fixture: &Fixture) -> BTreeMap<AssetHash, Vec<u8>> {
    let mut pool = BTreeMap::new();
    if let Some(bytes) = &fixture.bytes {
        pool.insert(fixture.asset.clone(), bytes.clone());
    }
    pool
}

/// 走**领域渲染本体**（`render::build`），不经过工具管线。
///
/// 直接用 `build` 是为了能注入一份**手工构造**的资产池（例如"池里的字节与声明的
/// 哈希不符"——走 `Domain::put_asset` 是造不出来的，它总是用字节算出键）。
fn build_with(
    project: &YebanProjectV1,
    assets: &BTreeMap<AssetHash, Vec<u8>>,
    path: &Path,
    arguments: Value,
) -> Result<RenderArtifact, Fault> {
    let request =
        render::validate(arguments.as_object().expect("参数必须是对象")).expect("参数合法");
    // 夹具里的"工程文件"必须与产物是**两个不同**的路径，否则会撞上
    // `guard_output_path`（"输出不能是工程文件本身"）—— 那正是 `render_master.rs`
    // 判据 8 钉住的护栏。这里从产物路径派生一个同目录的假工程路径即可。
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

/// 独立解码器：24-bit 小端负载 → `[-1, 1)` 浮点（不用生产代码）。
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

/// 从产物里取 `data` 负载（24-bit 交织样本）。
fn payload_samples(artifact: &RenderArtifact) -> Vec<f32> {
    let parsed = parse_container(&artifact.bytes).expect("产物必须是合法 WAVE 容器");
    decode_i24(&artifact.bytes[parsed.data.clone()])
}

/// 负载的 SHA-256（钉死常量用；与容器头部无关）。
fn payload_sha256(artifact: &RenderArtifact) -> String {
    let parsed = parse_container(&artifact.bytes).expect("产物必须是合法 WAVE 容器");
    AssetHash::of_bytes(&artifact.bytes[parsed.data.clone()])
        .as_str()
        .to_owned()
}

/// 某一帧**左声道**的样本（带符号）。
///
/// PDC 判据必须读符号：`-0.25` 与 `+0.25` 的 `frame_peak` 完全相同，
/// 而"快支路被补上"这件事恰恰表现为**电平从 +0.5 变成 +0.25 再变成 −0.25**。
fn frame_value(samples: &[f32], frame: usize) -> f32 {
    samples[frame * 2]
}

/// 一帧的最大绝对样本值。
fn frame_peak(samples: &[f32], frame: usize) -> f32 {
    let base = frame * 2;
    samples[base].abs().max(samples[base + 1].abs())
}

/// 第一个"有信号"（|样本| > 阈值）的帧号；没有则 `None`。
///
/// 阈值取 `0.01`（约 −40 dBFS）：抖动只有 ±1 LSB（≈1.2e-7），因此静音段的
/// 每个样本都远在阈值之下，不会把"抖出来的 ±1 LSB"误读成一个事件。
fn first_audible_frame(samples: &[f32], threshold: f32) -> Option<usize> {
    (0..samples.len() / 2).find(|&frame| frame_peak(samples, frame) > threshold)
}

/// 最后一个"有信号"的帧号。
fn last_audible_frame(samples: &[f32], threshold: f32) -> Option<usize> {
    (0..samples.len() / 2)
        .rev()
        .find(|&frame| frame_peak(samples, frame) > threshold)
}

/// 整段母带的峰值。
fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

// ---------------------------------------------------------------------------
// 判据 1：素材样本真的进了母带
// ---------------------------------------------------------------------------

/// 判据 1 [①]：一个音频片段渲染进母带，输出与**该资产的样本**逐样本对应。
///
/// ## 容差与理由
///
/// 母带是 24-bit **整数** PCM，导出前加 TPDF 抖动。因此文件里的样本与浮点素材
/// 之间只可能差两处：
///
/// 1. **抖动**：`tpdf_lsb ∈ (-1, +1)` LSB（TPDF 峰峰 2 LSB，见 `dither.rs`）；
/// 2. **四舍五入到整数**：至多 0.5 LSB。
///
/// 合计上界 1.5 LSB，判据取 **2 LSB**（`2 / 2²³ ≈ 2.38e-7`）。
/// 这个容差**不是放水**：素材取值是 1/64 的整数倍（在 24-bit 上精确可表示），
/// 因此"源被换成静音""片段被丢弃""增益算错"造成的偏差都在 1e-2 量级以上，
/// 会被这条容差稳稳抓住（`docs/ledger/audio-render-notes.md` 有注入实测）。
#[test]
fn an_audio_clip_renders_into_the_master_matching_the_asset_samples() {
    let scratch = Scratch::new("samples");
    let samples = pattern(480);
    let fixture = fixture(wav_f32(2, 48_000, &samples), &Tune::default());
    let out = scratch.join("master.wav");
    let artifact = build_with(&fixture.project, &pool(&fixture), &out, args(&out)).expect("渲染");
    let data = artifact.response_data();

    // 能力矩阵：它现在**真的**渲染了，不再登记 `audioClips`。
    assert_eq!(data["audio"]["wired"], true);
    assert_eq!(data["audio"]["clipsRendered"], 1);
    assert_eq!(data["audio"]["assetsReferenced"], 1);
    assert_eq!(data["audio"]["assets"][0]["bytesPresent"], true);
    assert_eq!(data["audio"]["assets"][0]["resampled"], false);
    assert_eq!(data["audio"]["assets"][0]["channelLayout"], "identity");
    assert_eq!(data["audio"]["assets"][0]["sourceSampleRate"], 48_000);
    assert_eq!(data["audio"]["assets"][0]["sourceChannels"], 2);
    assert_eq!(data["sources"][0]["kind"], "audio-clip");
    assert_eq!(data["sources"][0]["audioClips"], 1);
    assert_eq!(data["sources"][0]["audioClipsUnrendered"], 0);
    assert_eq!(data["sources"][0]["audioClipsGated"], 0);
    assert_eq!(data["sources"][0]["notes"], 0);
    assert_eq!(
        data["unsupported"],
        json!([]),
        "干净工程不该登记任何 unsupported"
    );

    // 逐样本对账：前 480 帧 == 素材；其余是静音（只允许 ±1 LSB 的抖动）。
    let rendered = payload_samples(&artifact);
    assert_eq!(rendered.len(), 48_000 * 2);
    let tolerance = 2.0 / 8_388_608.0;
    let mut worst = 0.0f32;
    for (index, expected) in samples.iter().enumerate() {
        worst = worst.max((rendered[index] - expected).abs());
    }
    assert!(
        worst <= tolerance,
        "素材样本没有逐样本出现在母带里: 最大偏差 {worst} > 容差 {tolerance}"
    );
    for frame in 480..48_000 {
        assert!(
            frame_peak(&rendered, frame) <= tolerance,
            "第 {frame} 帧不该有信号 (素材只有 480 帧)"
        );
    }
}

// ---------------------------------------------------------------------------
// 判据 2：落位
// ---------------------------------------------------------------------------

/// 判据 2 [②]：片段落在 `placement.start_tick` 换算出的帧上，起止误差 ≤ 1 帧。
///
/// 判据取**恰好相等**（比"≤ 1 个 L1 处理量子（128 帧）"严格一百多倍）：
/// `ticks_to_frames` 是纯整数 + 一次 `round` 的换算，没有理由差一帧。
#[test]
fn the_clip_lands_exactly_on_the_placement_start_frame() {
    let scratch = Scratch::new("landing");
    let samples = pattern(240);
    // 摆在 0.5 秒处（960 tick @120BPM）。
    let tune = Tune {
        start_tick: 960,
        duration_ticks: 960,
        ..Tune::default()
    };
    let fixture = fixture(wav_f32(2, 48_000, &samples), &tune);
    let out = scratch.join("master.wav");
    let artifact = build_with(&fixture.project, &pool(&fixture), &out, args(&out)).expect("渲染");
    let rendered = payload_samples(&artifact);

    assert_eq!(artifact.frames, 48_000, "1 秒母带");
    assert_eq!(
        first_audible_frame(&rendered, 0.01),
        Some(24_000),
        "0.5 秒处 = 第 24000 帧 (960 tick @120BPM / 48kHz)"
    );
    assert_eq!(
        last_audible_frame(&rendered, 0.01),
        Some(24_000 + 239),
        "240 帧素材必须整段落在区间内 (起止误差 0 帧 ≤ 1 帧)"
    );
    // 起点之前**没有**任何事件（抖动不构成事件）。
    for frame in 0..24_000 {
        assert!(frame_peak(&rendered, frame) <= 1.0 / 8_388_608.0);
    }
}

// ---------------------------------------------------------------------------
// 判据 3：门控
// ---------------------------------------------------------------------------

/// 判据 3 [③]：`placement.muted` / 音轨 `mute` / `solo` / 片段 `gain_db` 都真的生效。
///
/// 每一条都用一个**可测的差异**证明，而不是"字段被读到了"：
/// 静音 ⇒ 母带峰值 ≤ 1 LSB；−6 dB ⇒ 峰值减半（±1%）；独奏本轨 ⇒ 仍然发声。
#[test]
fn mute_solo_and_clip_gain_measurably_gate_the_audio_clip() {
    let scratch = Scratch::new("gating");
    let samples = constant(480, 0.5);

    let render_case = |tune: &Tune, name: &str| -> (RenderArtifact, Value) {
        let fixture = fixture(wav_f32(2, 48_000, &samples), tune);
        let out = scratch.join(name);
        let artifact =
            build_with(&fixture.project, &pool(&fixture), &out, args(&out)).expect("渲染");
        let data = artifact.response_data();
        (artifact, data)
    };

    // (a) 基线：恒定 0.5。
    let (base, base_data) = render_case(&Tune::default(), "base.wav");
    let base_peak = peak(&payload_samples(&base));
    assert!(
        (base_peak - 0.5).abs() < 1.0e-6,
        "基线峰值应为 0.5, 实测 {base_peak}"
    );
    assert_eq!(base_data["sources"][0]["audible"], true);

    // (b) 片段增益 −6 dB ⇒ 峰值减半。
    let (quiet, quiet_data) = render_case(
        &Tune {
            clip_gain_db: -6.0,
            ..Tune::default()
        },
        "gain.wav",
    );
    let quiet_peak = peak(&payload_samples(&quiet));
    let expected = 0.5 * 10f32.powf(-6.0 / 20.0);
    assert!(
        (quiet_peak - expected).abs() < expected * 0.01,
        "−6 dB 片段增益必须可测: 期望 {expected}, 实测 {quiet_peak}"
    );
    assert_eq!(quiet_data["audio"]["clipsRendered"], 1);

    // (c) placement 静音 ⇒ 它**不算内容**：整个工程只剩 0 帧可渲染, 因此如实
    // 报 `RENDER_FAILED`, 而不是写一份全静音的母带冒充成功。这与 MIDI 摆放的
    // 既有语义完全一致（`audible_end_tick` 同样跳过静音的摆放）。
    let muted_fixture = fixture(
        wav_f32(2, 48_000, &samples),
        &Tune {
            placement_muted: true,
            ..Tune::default()
        },
    );
    let muted_out = scratch.join("placement-muted.wav");
    let fault = build_with(
        &muted_fixture.project,
        &pool(&muted_fixture),
        &muted_out,
        args(&muted_out),
    )
    .expect_err("静音的摆放不构成内容");
    assert_eq!(fault.domain_code(), Some(ErrorCode::RenderFailed));
    let muted_value = fault.into_result().expect("带内");
    assert_eq!(muted_value["error"]["data"]["endTick"], 0);
    assert!(!muted_out.exists(), "失败时不得留下半成品");

    // (d) 音轨 mute ⇒ 静音，且如实报 `audible = false` + 被门控的片段数。
    let (track_muted, track_muted_data) = render_case(
        &Tune {
            track_mute: true,
            ..Tune::default()
        },
        "track-muted.wav",
    );
    assert!(
        peak(&payload_samples(&track_muted)) <= 2.0 / 8_388_608.0,
        "mute 的音轨不得发声"
    );
    assert_eq!(track_muted_data["sources"][0]["audible"], false);
    assert_eq!(track_muted_data["sources"][0]["audioClipsGated"], 1);
    assert_eq!(track_muted_data["audio"]["clipsRendered"], 0);

    // (e) 本轨 solo ⇒ 仍然发声（solo 只门控别人）。
    let (solo, solo_data) = render_case(
        &Tune {
            track_solo: true,
            ..Tune::default()
        },
        "solo.wav",
    );
    assert!((peak(&payload_samples(&solo)) - 0.5).abs() < 1.0e-6);
    assert_eq!(solo_data["sources"][0]["audible"], true);
}

// ---------------------------------------------------------------------------
// 判据 4 / 5：采样率
// ---------------------------------------------------------------------------

/// 判据 4 [④]：素材采样率 ≠ 渲染采样率 ⇒ **真的重采样**，且时长不变。
///
/// 这条判据的判别力在于它能区分三种实现：
///
/// | 实现 | 结果 |
/// | :--- | :--- |
/// | 真重采样（rubato sinc） | 1 秒素材 ⇒ 1 秒音频（约 48000 帧） |
/// | 改采样率标签（把 44100 帧当 48 kHz 用） | 音频只有 0.919 秒 ⇒ **判据红** |
/// | 丢帧 | 时长对但音高错 ⇒ 由 `resampled` 标志 + 长度契约兜住 |
///
/// 阈值取 47000 帧（留 ~2% 给重采样滤波器的边缘过渡，rubato 的 sinc 半径是
/// `sinc_len/2 = 128` 帧，本机影子实现是线性插值，两者都远小于 2%）。
#[test]
fn a_44k1_asset_in_a_48k_project_is_resampled_and_keeps_its_duration() {
    let scratch = Scratch::new("resample");
    // 1 秒 @44.1 kHz 的恒定电平素材。
    let samples = constant(44_100, 0.5);
    let fixture = fixture(wav_f32(2, 44_100, &samples), &Tune::default());
    let out = scratch.join("master.wav");
    let artifact = build_with(&fixture.project, &pool(&fixture), &out, args(&out)).expect("渲染");
    let data = artifact.response_data();

    assert_eq!(data["audio"]["assets"][0]["resampled"], true);
    assert_eq!(data["audio"]["assets"][0]["sourceSampleRate"], 44_100);
    assert_eq!(data["audio"]["assets"][0]["targetSampleRate"], 48_000);
    assert_eq!(
        data["audio"]["assets"][0]["channelLayout"], "identity",
        "重采样不得改变声道布局"
    );
    // 长度契约: `ceil(in * out_rate / in_rate)` ± (0.1% 或 8 帧, 取大者)。
    let rendered_frames = data["audio"]["assets"][0]["renderedFrames"]
        .as_u64()
        .expect("renderedFrames");
    let ideal = 48_000u64;
    let slack = (ideal / 1_000).max(8);
    assert!(
        rendered_frames.abs_diff(ideal) <= slack,
        "重采样后的帧数 {rendered_frames} 偏离理想值 {ideal} 超过 {slack} 帧"
    );

    // 真正的判别力: 音频必须几乎铺满 1 秒 —— "改标签"的实现只会铺 0.919 秒。
    let rendered = payload_samples(&artifact);
    assert_eq!(rendered.len(), 48_000 * 2);
    let late = (47_000..48_000)
        .filter(|&frame| frame_peak(&rendered, frame) > 0.01)
        .count();
    assert!(
        late > 800,
        "47000 帧之后只有 {late} 帧有信号 —— 素材被当成 48 kHz 直接搬过来了(时长缩水)"
    );
    assert_eq!(
        first_audible_frame(&rendered, 0.01),
        Some(0),
        "重采样不引入前置静音"
    );
}

/// 判据 5 [④]：采样率不一致**不再是错误**（旧的 `unwired: "resampler"` 拒绝退役）。
#[test]
fn a_sample_rate_mismatch_is_no_longer_an_error() {
    let scratch = Scratch::new("rate-mismatch");
    let samples = pattern(480);
    let fixture = fixture(wav_f32(2, 48_000, &samples), &Tune::default());
    let out = scratch.join("master.wav");
    // 工程 48 kHz, 请求 96 kHz: 素材要被升采样, MIDI 直接按 96 kHz 合成。
    let artifact = build_with(
        &fixture.project,
        &pool(&fixture),
        &out,
        json!({"format": "wav", "sampleRate": 96000, "path": out.display().to_string()}),
    )
    .expect("96 kHz 请求必须成功");
    let data = artifact.response_data();
    assert_eq!(data["sampleRate"], 96_000);
    assert_eq!(data["audio"]["assets"][0]["resampled"], true);
    assert_eq!(data["audio"]["assets"][0]["targetSampleRate"], 96_000);
    assert!(
        data.get("unwired").is_none(),
        "重采样已接线, 不该再有 `unwired`: {data}"
    );
    // 口径必须写清"用的是哪一种"重采样。
    let method = data["audio"]["resampler"]["method"]
        .as_str()
        .expect("resampler.method");
    assert!(method.contains("rubato"), "{method}");
    assert!(method.contains("BlackmanHarris2"), "{method}");
}

// ---------------------------------------------------------------------------
// 判据 6：确定性 + 钉死摘要
// ---------------------------------------------------------------------------

/// 判据 6 [⑤]：同输入两次渲染**逐字节相同**，且命中钉死的摘要常量。
///
/// 常量由**独立的 Python 实现**算出（`docs/ledger/audio-render-notes.md` §5 记录了
/// 口径：同一份 f32 素材 → SHA-256 over IEEE-754 位型；24-bit 负载由独立的
/// xorshift32 + TPDF 复算）。它们与 `render_master.rs` 的两个常量同一性质：
/// **同平台同工具链**下跨机器可比（[ARCH-DET-001] 的 L1 形式）。
///
/// 为什么这一条不做跨架构承诺：重采样含超越函数（rubato 的窗系数），按
/// `ADR-0001` **D32** 属"超越函数类"。本判据的夹具**同率**（48 kHz → 48 kHz，
/// 逐位透传、零滤波），因此这条链上没有超越函数，位级常量是站得住的。
#[test]
fn two_renders_of_the_same_project_are_byte_identical_and_hit_the_pinned_hashes() {
    let scratch = Scratch::new("determinism");
    let samples = pattern(480);
    let fixture = fixture(wav_f32(2, 48_000, &samples), &Tune::default());
    let out = scratch.join("master.wav");
    let first = build_with(&fixture.project, &pool(&fixture), &out, args(&out)).expect("渲染一");
    let second = build_with(&fixture.project, &pool(&fixture), &out, args(&out)).expect("渲染二");

    assert_eq!(first.bytes, second.bytes, "两次渲染必须逐字节相同");
    assert_eq!(first.sha256, second.sha256);
    assert_eq!(first.master_digest, second.master_digest);
    assert_eq!(payload_sha256(&first), payload_sha256(&second));

    assert_eq!(
        first.master_digest, PINNED_MASTER_DIGEST,
        "母带浮点摘要变了 —— 要么改了渲染语义, 要么引入了熵源"
    );
    assert_eq!(
        payload_sha256(&first),
        PINNED_PAYLOAD_SHA256,
        "24-bit 负载变了 —— 抖动/量化/素材有一条不再确定"
    );
    assert_eq!(
        first.sha256, PINNED_FILE_SHA256,
        "整份文件的 SHA-256 变了 —— 容器头部或负载不再确定"
    );
}

/// 母带**浮点**样本位型的 SHA-256（`RenderArtifact::master_digest`）。
const PINNED_MASTER_DIGEST: &str =
    "c600d4f011f6e223d41363cfc8763b13f1c55ba426f0a95b895c0461c8f3a213";

/// 24-bit PCM **负载**的 SHA-256（不含容器头部）。
const PINNED_PAYLOAD_SHA256: &str =
    "1c6747217c02b1c93c3d24673568be2dc5ea58c5644552fb4fd5ca7a92775f31";

/// 整份 RIFF 文件的 SHA-256。
const PINNED_FILE_SHA256: &str = "6d290a90a091128c1842c6a7ce10f044773dafdcfc919b534f441f42d7522c37";

// ---------------------------------------------------------------------------
// 判据 7 / 8 / 9：资产缺失、完整性、以及"声明了但没有字节"
// ---------------------------------------------------------------------------

/// 判据 7 [⑥a]：片段引用的资产**索引与池里都没有** ⇒ 明确的 `assetMissing`。
#[test]
fn an_asset_that_exists_nowhere_is_a_named_error() {
    let scratch = Scratch::new("missing");
    let samples = pattern(240);
    let tune = Tune {
        provide_bytes: false,
        declare_in_index: false,
        ..Tune::default()
    };
    let fixture = fixture(wav_f32(2, 48_000, &samples), &tune);
    let out = scratch.join("master.wav");
    let fault =
        build_with(&fixture.project, &pool(&fixture), &out, args(&out)).expect_err("必须报错");
    assert_eq!(fault.domain_code(), Some(ErrorCode::RenderFailed));
    let value = fault.into_result().expect("带内");
    assert_eq!(value["error"]["code"], "RENDER_FAILED");
    assert_eq!(value["error"]["data"]["reason"], "assetMissing");
    assert_eq!(
        value["error"]["data"]["asset"],
        fixture.asset.as_str(),
        "必须报出是哪一个资产"
    );
    assert_eq!(value["error"]["data"]["declaredInIndex"], false);
    assert!(!out.exists(), "失败时不得留下半成品");
}

/// 判据 8 [⑥b]：会话池里的字节与它声明的 SHA-256 不符 ⇒ `assetHashMismatch`。
///
/// 这条走的是**手工构造的池**：`Domain::put_asset` 永远用字节算出键，因此
/// "键与内容不符"只能从 `render::build` 的 `AssetStore` 注入点造出来 ——
/// 这也正是它存在的理由（第三道完整性校验，前两道在容器的读写两侧）。
#[test]
fn bytes_that_do_not_match_the_declared_hash_are_rejected() {
    let scratch = Scratch::new("mismatch");
    let good = wav_f32(2, 48_000, &pattern(240));
    let fixture = fixture(good.clone(), &Tune::default());
    let out = scratch.join("master.wav");
    // 池里放**别的**字节, 但键仍是工程声明的那个哈希。
    let mut tampered: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
    let mut other = good.clone();
    other[20] ^= 0xFF; // 动一个字节
    tampered.insert(fixture.asset.clone(), other.clone());
    let fault = build_with(&fixture.project, &tampered, &out, args(&out)).expect_err("必须报错");
    assert_eq!(fault.domain_code(), Some(ErrorCode::RenderFailed));
    let value = fault.into_result().expect("带内");
    assert_eq!(value["error"]["data"]["reason"], "assetHashMismatch");
    assert_eq!(value["error"]["data"]["asset"], fixture.asset.as_str());
    assert_eq!(
        value["error"]["data"]["actual"],
        AssetHash::of_bytes(&other).as_str(),
        "必须报出实际算出的哈希, 便于自纠"
    );
    assert!(!out.exists());
}

/// 判据 9 [口径]：工程**声明**了资产、会话池里没有字节 ⇒ 登记 `audioClips` + 静音。
///
/// 这是 `Domain::open_in_memory` 注入的会话的形态（那条种子路径本来就不携带资产
/// 载荷；`ADR-0001 D43` 之后**没有**第二条路能造出这种会话）。口径的选择写成判据：
/// **不假装渲染过**，也不把它当成工程损坏（工程确实声明了它）。
#[test]
fn a_declared_asset_without_payload_is_registered_rather_than_faked() {
    let scratch = Scratch::new("absent-payload");
    let samples = pattern(240);
    let tune = Tune {
        provide_bytes: false,
        declare_in_index: true,
        ..Tune::default()
    };
    let fixture = fixture(wav_f32(2, 48_000, &samples), &tune);
    let out = scratch.join("master.wav");
    let artifact = build_with(&fixture.project, &pool(&fixture), &out, args(&out)).expect("渲染");
    let data = artifact.response_data();
    assert_eq!(data["unsupported"], json!(["audioClips"]));
    assert_eq!(data["unsupportedCounts"]["audioClips"], 1);
    assert_eq!(data["audio"]["clipsRendered"], 0);
    assert_eq!(data["audio"]["assets"][0]["bytesPresent"], false);
    assert_eq!(data["audio"]["assets"][0]["declaredInIndex"], true);
    assert_eq!(data["sources"][0]["audioClips"], 0);
    assert_eq!(data["sources"][0]["audioClipsUnrendered"], 1);
    assert!(
        peak(&payload_samples(&artifact)) <= 2.0 / 8_388_608.0,
        "没有字节就只能当静音"
    );
}

// ---------------------------------------------------------------------------
// 判据 10：坏资产不 panic
// ---------------------------------------------------------------------------

/// 判据 10 [⑦]：坏资产（垃圾字节 / 截断 / 不支持位深）⇒ 契约内错误码，**不 panic**。
#[test]
fn broken_assets_are_contract_errors_not_panics() {
    let scratch = Scratch::new("broken");

    // (a) 完全不是音频。
    let garbage = b"this is definitely not an audio container at all".to_vec();
    // (b) `data` 块声明 4096 帧但文件被截断到 512 字节。
    let mut truncated = wav_s16(1, 48_000, &[1_000i16; 4096]);
    truncated.truncate(512);
    // (c) 12-bit PCM：本构建没有这个解码器。
    let odd_depth = wav_bytes(1, 48_000, 1, 12, &[0u8; 64]);

    for (context, bytes) in [
        ("垃圾字节", garbage),
        ("截断的 data 块", truncated),
        ("12-bit PCM", odd_depth),
    ] {
        let fixture = fixture(bytes, &Tune::default());
        let out = scratch.join("broken.wav");
        let fault = match build_with(&fixture.project, &pool(&fixture), &out, args(&out)) {
            Ok(_) => panic!("{context}: 坏资产必须报错, 不许静音成功"),
            Err(fault) => fault,
        };
        // 契约内的错误码: 要么 `RENDER_FAILED`, 要么 `IO_ERROR`（解码器报 I/O 时）。
        let code = fault.domain_code().expect("必须是领域码");
        assert!(
            matches!(code, ErrorCode::RenderFailed | ErrorCode::IoError),
            "{context}: 错误码 {code:?} 不在预期集合里"
        );
        let value = fault.into_result().expect("带内");
        let rendered = value["error"]["code"].as_str().expect("code");
        assert!(
            ErrorCode::SCHEMA_CONTRACT
                .iter()
                .any(|known| known.as_str() == rendered),
            "{context}: {rendered} 不在契约的 error.code 枚举里"
        );
        assert_eq!(
            value["error"]["data"]["reason"], "assetDecodeFailed",
            "{context}: 必须说清是解码失败"
        );
        assert!(!out.exists(), "{context}: 失败时不得留下半成品");
    }
}

// ---------------------------------------------------------------------------
// 判据 11 / 12：dryRun 与幂等（走真实工具管线）
// ---------------------------------------------------------------------------

/// 注入一份夹具工程 + 资产字节的分发器。
fn dispatcher_with(fixture: &Fixture, path: &Path) -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(NOW_MS);
    dispatcher
        .domain_mut()
        .open_in_memory(path.to_path_buf(), fixture.project.clone(), false)
        .expect("注入夹具工程");
    if let Some(bytes) = &fixture.bytes {
        let hash = dispatcher
            .domain_mut()
            .put_asset(bytes.clone())
            .expect("放进会话 CAS 池");
        assert_eq!(hash, fixture.asset, "CAS 键必须由字节算出");
    }
    (dispatcher, auth)
}

/// 走真实 `tools/call` 管线（工具路径上不允许 JSON-RPC 层错误）。
fn call(dispatcher: &mut Dispatcher, auth: &str, arguments: Value) -> Value {
    call_named(dispatcher, auth, "yeban_render_master", arguments)
}

/// 与 [`call`] 同一条管线，但**点名工具**（`yeban_import_audio` 等也走这里）。
fn call_named(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
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

/// 判据 11 [⑧]：`dryRun` 给同一批**实测**数字，且一个字节都不写。
#[test]
fn dry_run_renders_the_audio_clip_without_writing_anything() {
    let scratch = Scratch::new("dryrun");
    let samples = pattern(480);
    let fixture = fixture(wav_f32(2, 48_000, &samples), &Tune::default());
    let (mut dispatcher, auth) = dispatcher_with(&fixture, &scratch.join("demo.yeban"));
    let before = scratch.entries();

    let result = call(
        &mut dispatcher,
        &auth,
        json!({"format": "wav", "sampleRate": 48000, "dryRun": true}),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(scratch.entries(), before, "dryRun 不许写任何文件");
    let preview = &result["data"]["preview"];
    assert_eq!(preview["audio"]["clipsRendered"], 1);
    assert_eq!(preview["audio"]["assets"][0]["bytesPresent"], true);
    assert_eq!(preview["frames"], 48_000);
    assert_eq!(
        preview["payloadBytes"], 288_000,
        "48000 帧 × 2 声道 × 3 字节"
    );
    assert_eq!(preview["writesFile"], true, "预览要如实说真调用会写盘");
    assert_eq!(
        preview["wouldWrite"]["sha256"]
            .as_str()
            .expect("sha256")
            .len(),
        64
    );
}

/// 判据 12 [⑨]：同 `idempotencyKey` 不重复渲染（`mtime` 不变 + 删掉后重放不重建）。
#[test]
fn the_same_idempotency_key_does_not_render_the_audio_clip_twice() {
    let scratch = Scratch::new("idempotency");
    let samples = pattern(480);
    let fixture = fixture(wav_f32(2, 48_000, &samples), &Tune::default());
    let (mut dispatcher, auth) = dispatcher_with(&fixture, &scratch.join("demo.yeban"));
    let out = scratch.join("idem.wav");
    let arguments = json!({
        "format": "wav",
        "sampleRate": 48000,
        "path": out.display().to_string(),
        "idempotencyKey": "audio-clip-k1",
    });

    let first = call(&mut dispatcher, &auth, arguments.clone());
    assert_eq!(first["status"], "success", "{first}");
    let metadata = fs::metadata(&out).expect("产物必须存在");
    let digest = AssetHash::of_bytes(&fs::read(&out).expect("读产物"))
        .as_str()
        .to_owned();

    let second = call(&mut dispatcher, &auth, arguments.clone());
    // 幂等命中时响应外面多一层 `replayed` 信封（主体仍是首次的 `ToolResponse`）。
    assert_eq!(second["replayed"], true, "第二次必须命中幂等缓存");
    assert_eq!(
        second["response"]["result"], first,
        "重放的主体必须与首次逐字节相同"
    );
    let after = fs::metadata(&out).expect("产物必须还在");
    assert_eq!(
        metadata.modified().expect("mtime"),
        after.modified().expect("mtime"),
        "同幂等键的重放不得重新渲染"
    );
    assert_eq!(
        AssetHash::of_bytes(&fs::read(&out).expect("读产物")).as_str(),
        digest,
        "重放必须给出同一份产物"
    );

    // 删掉产物后重放: 幂等缓存命中 ⇒ **不重建**（真渲染才会重建）。
    fs::remove_file(&out).expect("删产物");
    let third = call(&mut dispatcher, &auth, arguments);
    assert_eq!(third["replayed"], true, "{third}");
    assert!(
        !out.exists(),
        "幂等命中时不该重新渲染 —— 重新渲染会再写一次文件"
    );
}

// ---------------------------------------------------------------------------
// 判据 13：PDC
// ---------------------------------------------------------------------------

/// 判据 13 [⑩]：音频片段参与 [ARCH-PDC-001] 的延迟对齐，实测偏移 = `L_max`。
///
/// 夹具：两条音轨引用**不同电平**的两个素材（+0.5 / −0.25），都摆在 0 tick：
///
/// - `lat` 轨挂一个 48 帧延迟的设备 ⇒ 它的边延迟是 `L_max − 48 = 0`；
/// - `fast` 轨没有设备 ⇒ 它的边延迟是 `L_max − 0 = 48`。
///
/// 于是母带是：`[0,48)` = 0.5；`[48,480)` = 0.25；`[480,528)` = −0.25；之后静音。
/// 三个电平台阶把"补偿了恰好 48 帧"变成一个**逐样本可读**的读数，
/// 同时证明音频片段用的就是 `DeviceDefinition::latency_samples` 这张**唯一的**表
/// （响应里的 `latencySource` 也一并钉住）。
#[test]
fn pdc_shifts_the_audio_clip_branch_by_exactly_the_device_latency() {
    let scratch = Scratch::new("pdc");
    let slow_bytes = wav_f32(2, 48_000, &constant(480, 0.5));
    let fast_bytes = wav_f32(2, 48_000, &constant(480, -0.25));
    let slow_hash = AssetHash::of_bytes(&slow_bytes);
    let fast_hash = AssetHash::of_bytes(&fast_bytes);

    let master = id(1);
    let slow_track = id(2);
    let fast_track = id(3);
    let slow_clip = id(10);
    let fast_clip = id(11);
    let slow_placement = id(20);
    let fast_placement = id(21);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "pdc fixture".to_owned(),
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
    project.tracks.insert(
        slow_track,
        TrackV3 {
            id: slow_track,
            name: "lat".to_owned(),
            kind: TrackKind::Audio,
            devices: vec![DeviceDefinition {
                id: id(101),
                name: "lat48".to_owned(),
                kind: DeviceKind::InternalEffect,
                bypassed: false,
                params: Vec::new(),
                latency_samples: 48,
            }],
            clips: BTreeMap::from([(
                slow_placement,
                ClipPlacement {
                    id: slow_placement,
                    clip_id: slow_clip,
                    duration_ticks: ONE_SECOND_TICKS,
                    ..ClipPlacement::default()
                },
            )]),
            ..TrackV3::default()
        },
    );
    project.tracks.insert(
        fast_track,
        TrackV3 {
            id: fast_track,
            name: "fast".to_owned(),
            kind: TrackKind::Audio,
            clips: BTreeMap::from([(
                fast_placement,
                ClipPlacement {
                    id: fast_placement,
                    clip_id: fast_clip,
                    duration_ticks: ONE_SECOND_TICKS,
                    ..ClipPlacement::default()
                },
            )]),
            ..TrackV3::default()
        },
    );
    for (clip_id, name, hash) in [
        (slow_clip, "slow", slow_hash.clone()),
        (fast_clip, "fast", fast_hash.clone()),
    ] {
        project.clip_pool.insert(
            clip_id,
            ClipPoolEntry {
                id: clip_id,
                name: name.to_owned(),
                content: ClipContent::Audio {
                    asset: hash.clone(),
                    gain_db: 0.0,
                },
            },
        );
        project.assets.insert(
            hash.clone(),
            AssetMetadata {
                hash,
                original_path: format!("samples/{name}.wav"),
                byte_len: 44 + 480 * 8,
                media_kind: MediaKind::Audio,
                license: "CC0-1.0".to_owned(),
            },
        );
    }
    project.routing_graph.nodes = vec![slow_track, fast_track, master];
    for (seed, source) in [(40u32, slow_track), (41, fast_track)] {
        let edge_id = id(seed);
        project.routing_graph.edges.insert(
            edge_id,
            RoutingEdge {
                id: edge_id,
                source_node: source,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }
    project.validate().expect("夹具必须合法");

    let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
    assets.insert(slow_hash, slow_bytes);
    assets.insert(fast_hash, fast_bytes);
    let out = scratch.join("master.wav");
    let artifact = build_with(&project, &assets, &out, args(&out)).expect("渲染");
    let data = artifact.response_data();
    assert_eq!(data["longestPathFrames"], 48, "L_max 只能来自设备延迟");
    assert_eq!(
        data["latencySource"], "DeviceDefinition::latency_samples (track_latencies)",
        "音频片段不许自建第二延迟来源"
    );
    // 设备链的**参数**没有求值 ⇒ `deviceChainDsp` 必须登记（与 `render_master.rs`
    // 的延迟判据同一处置）。延迟本身是真的, 未实现的是 DSP 参数。
    assert_eq!(data["unsupported"], json!(["deviceChainDsp"]));
    assert_eq!(data["audio"]["clipsRendered"], 2);

    let rendered = payload_samples(&artifact);
    let tolerance = 1.0e-5;
    assert!(
        (frame_value(&rendered, 0) - 0.5).abs() < tolerance,
        "第 0 帧只有长支路 (+0.5)"
    );
    assert!(
        (frame_value(&rendered, 47) - 0.5).abs() < tolerance,
        "补偿还没生效: 前 48 帧只有长支路"
    );
    assert!(
        (frame_value(&rendered, 48) - 0.25).abs() < tolerance,
        "第 48 帧起快支路 (−0.25) 被补上 —— 实测偏移必须是 L_max: 实测 {}",
        frame_value(&rendered, 48)
    );
    assert!((frame_value(&rendered, 479) - 0.25).abs() < tolerance);
    assert!(
        (frame_value(&rendered, 480) + 0.25).abs() < tolerance,
        "长支路素材只有 480 帧, 之后只剩快支路的 −0.25: 实测 {}",
        frame_value(&rendered, 480)
    );
    assert!((frame_value(&rendered, 527) + 0.25).abs() < tolerance);
    assert!(frame_peak(&rendered, 528) <= 2.0 / 8_388_608.0);
    assert_eq!(
        first_audible_frame(&rendered, 0.01),
        Some(0),
        "带延迟设备的那条支路不补延迟"
    );
    let shifted = (48..480)
        .filter(|&frame| (frame_value(&rendered, frame) - 0.25).abs() < tolerance)
        .count();
    assert_eq!(shifted, 432, "48..480 共 432 帧必须是补偿后的电平 +0.25");
    let fast_only = (480..528)
        .filter(|&frame| (frame_value(&rendered, frame) + 0.25).abs() < tolerance)
        .count();
    assert_eq!(
        fast_only, 48,
        "480..528 共 48 帧只剩快支路的 −0.25 —— 这就是 48 帧补偿的**尾部**证据"
    );
}

// ---------------------------------------------------------------------------
// 判据 14 / 15：**只用 MCP 工具**从零把音频片段放上轨道并渲染出来
// ---------------------------------------------------------------------------

/// 一份"有音轨、没有片段池条目、没有摆放、池里没有字节"的起点工程。
///
/// 这正是 `yeban_import_audio` + `trackId` 要驱动的那条链路的起点：17 个工具里
/// **没有任何**工具能造出片段池条目或摆放（`propose_section` 需要工程里先有 MIDI 材料），
/// 因此本判据从"工程有音轨、音频侧一切为空"开始，把"登记 + 摆放"整件事交给工具。
fn empty_audio_project_around(bytes: &[u8]) -> Fixture {
    let mut fixture = fixture(
        bytes.to_vec(),
        &Tune {
            declare_in_index: false,
            provide_bytes: false,
            ..Tune::default()
        },
    );
    fixture.project.clip_pool.clear();
    for track in fixture.project.tracks.values_mut() {
        track.clips.clear();
    }
    fixture.project.assets.clear();
    fixture.bytes = None;
    fixture.project.validate().expect("起点工程必须合法");
    fixture
}

/// 判据 14 [端到端]：`yeban_import_audio`（带 `trackId`）→ `yeban_render_master`，
/// **不带任何判据旁路**：全程走真实 `tools/call` 管线。
///
/// 这条判据的牙齿：
/// - 把 `yeban_import_audio` 的摆放那一半去掉（只登记 `clip_pool`）⇒
///   `clipsRendered` 变 0、母带里没有素材样本 ⇒ 红；
/// - 把 `trackId` 悄悄忽略掉 ⇒ 同上；
/// - 把池里的字节当成"声明即可"（读 `project.assets` 而不是 CAS 池）⇒
///   `bytesPresent` 变 `false` ⇒ 红。
#[test]
fn the_mcp_tools_alone_place_and_render_an_audio_clip_from_zero() {
    let scratch = Scratch::new("tool-e2e");
    // 单声道 4800 帧 @48 kHz = 100 ms、恒定 0.5（母带里必须能测到这个电平）。
    let samples = vec![0.5f32; 4_800];
    let bytes = wav_f32(1, 48_000, &samples);
    let fixture = empty_audio_project_around(&bytes);
    let wav = scratch.join("kick.wav");
    fs::write(&wav, &bytes).expect("写素材文件");
    let out = scratch.join("master.wav");
    let (mut dispatcher, auth) = dispatcher_with(&fixture, &scratch.join("demo.yeban"));

    // ---- ① 导入 + 摆放：一个工具、一次提交、两个字面事实 ----
    let imported = call_named(
        &mut dispatcher,
        &auth,
        "yeban_import_audio",
        json!({
            "name": "Kick",
            "path": wav.display().to_string(),
            "trackId": id(2).to_canonical_string(),
            "startTick": 0,
            "gainDb": 0.0,
        }),
    );
    assert_eq!(imported["status"], "success", "{imported}");
    let data = &imported["data"];
    assert_eq!(data["created"], true, "片段是新的: {data}");
    assert_eq!(
        data["placement"]["placed"], true,
        "摆放必须真的提交: {data}"
    );
    assert_eq!(data["placement"]["trackId"], id(2).to_canonical_string());
    assert_eq!(data["placement"]["startTick"], 0);
    assert_eq!(data["decoded"]["frames"], 4_800, "单声道 4800 帧: {data}");
    assert_eq!(
        data["placement"]["durationTicks"], 192,
        "4800 帧 @48k / 120 BPM ⇒ 0.1 s = 0.2 拍 ⇒ 192 tick (整数关系): {data}"
    );
    let placement_id = data["placement"]["placementId"]
        .as_str()
        .expect("placementId")
        .to_owned();

    // 工程侧：摆放真的落在音轨上（不是只出现在响应里）。
    let project = dispatcher.domain().active_project().expect("活跃工程");
    let landed = project
        .tracks
        .get(&id(2))
        .expect("音轨在场")
        .clips
        .get(&EntityId::from_str(&placement_id).expect("ULID"))
        .expect("摆放必须在音轨的 clips 上");
    assert_eq!(landed.duration_ticks, 192);
    assert!(
        project.assets.is_empty(),
        "工程 assets 索引仍是空的 (needs-3)"
    );

    // ---- ② 渲染：片段真的进母带，且字节真的从会话 CAS 池里读到 ----
    let rendered = call_named(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    let data = &rendered["data"];
    assert_eq!(data["audio"]["clipsRendered"], 1, "{data}");
    let assets = data["audio"]["assets"].as_array().expect("assets");
    assert_eq!(assets.len(), 1);
    assert_eq!(
        assets[0]["bytesPresent"], true,
        "渲染读的是会话 CAS 池, 不是 project.assets 索引: {assets:?}"
    );
    assert_eq!(assets[0]["renderedFrames"], 4_800, "{assets:?}");
    assert_eq!(assets[0]["resampled"], false, "同为 48 kHz ⇒ 不重采样");
    assert!(
        !data["unsupported"]
            .as_array()
            .expect("unsupported")
            .iter()
            .any(|key| key == "audioClips"),
        "已渲染的片段不得登记为 unsupported: {data}"
    );
    let source = data["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .find(|source| source["node"] == id(2).to_canonical_string())
        .expect("音频轨必须在 sources 里");
    assert_eq!(source["audioClips"], 1, "{source}");
    assert_eq!(source["audioClipsUnrendered"], 0, "{source}");
    assert_eq!(source["kind"], "audio-clip", "{source}");

    // ---- ③ 母带载荷里真的有那段素材（不是"报了个数字"） ----
    let produced = fs::read(&out).expect("读产物");
    let parsed = parse_container(&produced).expect("产物必须是合法 WAVE 容器");
    let rendered_samples = decode_i24(&produced[parsed.data.clone()]);
    // 实测口径（音频片段路径）：`pan == 0` 时增益**不做** −3 dB 衰减 ⇒ 单声道 0.5
    // 原样进两个输出声道（与 MIDI 合成源总是走等功率声相不同）。
    let expected = 0.5f32;
    assert!(
        (peak(&rendered_samples) - expected).abs() < 1.0e-3,
        "素材电平 0.5 必须出现在母带里 (实测口径期望 {expected}): 实测 {}",
        peak(&rendered_samples)
    );
    assert!(
        (frame_value(&rendered_samples, 0) - expected).abs() < 1.0e-3,
        "第 0 帧就是素材: 实测 {}",
        frame_value(&rendered_samples, 0)
    );
    assert!(
        (frame_value(&rendered_samples, 4_799) - expected).abs() < 1.0e-3,
        "第 4799 帧仍在摆放时值内: 实测 {}",
        frame_value(&rendered_samples, 4_799)
    );
}

/// 判据 15 [端到端 + 确定性的载体]：导入 + 摆放 ⇒ 保存 ⇒ 关闭 ⇒ 重开 ⇒ 再渲染，
/// **两处的事实与产物摘要必须一致**。
///
/// 这条判据钉住的是"字节从哪来"的**结论**：`yeban_import_audio` **不写**
/// `project.assets` 索引（`needs-3`），而容器的 `assets/{sha256}` 条目由会话 CAS 池写出，
/// 与索引无关。因此重开之后渲染仍然读得到字节（`bytesPresent: true`）。
///
/// 牙齿：谁把"保存/打开"改成以 `project.assets` 索引为字节的准入条件，
/// 这一条立刻红（重开之后 `bytesPresent` 变 `false`、`clipsRendered` 变 0）。
#[test]
fn a_placed_audio_clip_survives_save_close_reopen_without_an_assets_index() {
    let scratch = Scratch::new("tool-e2e-reopen");
    let samples = vec![0.5f32; 4_800];
    let bytes = wav_f32(1, 48_000, &samples);
    let fixture = empty_audio_project_around(&bytes);
    let project_path = scratch.join("demo.yeban");
    let wav = scratch.join("kick.wav");
    fs::write(&wav, &bytes).expect("写素材文件");
    let first_out = scratch.join("first.wav");
    let second_out = scratch.join("second.wav");
    let (mut dispatcher, auth) = dispatcher_with(&fixture, &project_path);

    let imported = call_named(
        &mut dispatcher,
        &auth,
        "yeban_import_audio",
        json!({
            "name": "Kick",
            "path": wav.display().to_string(),
            "trackId": id(2).to_canonical_string(),
            "startTick": 0,
        }),
    );
    assert_eq!(imported["data"]["placement"]["placed"], true, "{imported}");

    let saved = call_named(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["status"], "success", "{saved}");
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    assert_eq!(
        saved["data"]["assets"], 1,
        "容器里必须有 1 条 assets/{{sha256}}（字节随池走, 与索引无关）: {saved}"
    );

    let first = call_named(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({"format": "wav", "sampleRate": 48000, "path": first_out.display().to_string()}),
    );
    assert_eq!(first["data"]["audio"]["clipsRendered"], 1, "{first}");

    // 关闭（保存并释放锁）⇒ 重新打开 ⇒ 再渲染。
    let closed = call_named(&mut dispatcher, &auth, "yeban_close_project", json!({}));
    assert_eq!(closed["status"], "success", "{closed}");
    let reopened = call_named(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({"path": project_path.display().to_string()}),
    );
    assert_eq!(reopened["status"], "success", "{reopened}");
    assert_eq!(
        reopened["data"]["assets"], 1,
        "重开后池里必须有那份字节: {reopened}"
    );
    assert_eq!(
        reopened["data"]["project"]["clipCount"], 1,
        "片段池条目跨保存存活: {reopened}"
    );
    // `project.assets` 索引**仍然**是空的：登记写入的是会话 CAS 池 + `clip_pool`，
    // 索引由**没有任何工具**写入（`needs-3`）。这条断言刻意钉住现状 ——
    // 若将来给索引补一条 `Op`（那是模型层 + 契约的改动），这里必须**显式**改，
    // 而不是让"索引悄悄多了一条"在别处漂移。
    assert!(
        dispatcher
            .domain()
            .active_project()
            .expect("活跃工程")
            .assets
            .is_empty(),
        "重开之后 project.assets 索引仍然是空的 (needs-3)"
    );

    let second = call_named(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({"format": "wav", "sampleRate": 48000, "path": second_out.display().to_string()}),
    );
    assert_eq!(second["status"], "success", "{second}");
    assert_eq!(
        second["data"]["audio"]["clipsRendered"], 1,
        "重开之后片段仍然进母带: {second}"
    );
    assert_eq!(
        second["data"]["audio"]["assets"][0]["bytesPresent"], true,
        "重开之后字节仍然在会话 CAS 池里: {second}"
    );
    // 两次渲染：注入时钟固定 ⇒ `bext` 的墙钟字段也固定 ⇒ 产物逐字节相同。
    let first_bytes = fs::read(&first_out).expect("第一次产物");
    let second_bytes = fs::read(&second_out).expect("第二次产物");
    assert_eq!(
        first_bytes, second_bytes,
        "同一工程 + 同一注入时钟 ⇒ 两次渲染必须逐字节相同"
    );
    assert_eq!(
        first["data"]["sha256"], second["data"]["sha256"],
        "同一工程 + 同一注入时钟 ⇒ 同一 sha256"
    );
}

/// **>2 声道的素材被拒绝，而不是静默丢声道**（`RENDER_FAILED` +
/// `data.reason = "assetChannelLayout"`），且被拒的渲染**不落盘**。
///
/// 为什么必须有这一条：`clip_math::channel_layout(4, 2)` 返回 `None`，而这条 `None`
/// 分支是"绝不悄悄降混"这条契约的**唯一**证据。它此前没有任何判据 —— 第四批注入
/// （`assetChannelLayout` 那条分支被改成 `continue`）在**全量**测试下全绿。
///
/// 注入（实测红）：把这条 `return Err(...)` 换成 `continue`（把 4 声道素材当"没渲染"）
/// ⇒ 本判据红。
#[test]
fn a_multichannel_asset_is_refused_instead_of_silently_downmixed() {
    let scratch = Scratch::new("channel-layout");
    // 4 声道：两份立体声图案拼起来（解码只看 WAV 头里的声道数与帧数）。
    let mut samples = pattern(480);
    samples.extend(pattern(480));
    let quad = fixture(wav_f32(4, 48_000, &samples), &Tune::default());
    let out = scratch.join("master.wav");
    let fault = build_with(&quad.project, &pool(&quad), &out, args(&out))
        .expect_err("4 声道素材进立体声母线必须被拒");
    assert_eq!(fault.domain_code(), Some(ErrorCode::RenderFailed));
    let value = fault.into_result().expect("带内");
    assert_eq!(value["error"]["code"], "RENDER_FAILED");
    assert_eq!(value["error"]["data"]["reason"], "assetChannelLayout");
    assert_eq!(value["error"]["data"]["sourceChannels"], 4);
    assert_eq!(value["error"]["data"]["targetChannels"], 2);
    assert_eq!(
        value["error"]["data"]["supported"],
        json!([1, 2]),
        "报文必须列出支持的声道数, 便于自纠"
    );
    assert!(!out.exists(), "被拒的渲染不得留下半成品");

    // 阴性对照: 单声道素材（supported 里的另一项）照旧渲染得出来。
    let single = fixture(wav_f32(1, 48_000, &pattern(240)), &Tune::default());
    let mono_out = scratch.join("mono.wav");
    let artifact = build_with(&single.project, &pool(&single), &mono_out, args(&mono_out))
        .expect("单声道素材必须能渲染");
    assert_eq!(
        artifact.response_data()["audio"]["assets"][0]["channelLayout"],
        "mono-to-all",
        "单声道素材走 `ChannelLayout::MonoToAll` 的规范名 (与 clip_math 同一份真相)"
    );
}
