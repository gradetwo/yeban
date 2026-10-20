//! ⭐ 任务单 `#253` ＝ `C1` 判据：**同一份工程 ⇒ MCP 的 `masterDigest` vs CLI 的 `digest`**。
//!
//! ## 这一节量什么（对象 ＋ 单位，先说再跑）
//!
//! | 量 | 来源 | 单位 |
//! | :--- | :--- | :--- |
//! | `masterDigest` | `yeban_render_master`（内嵌装配的响应 `data.masterDigest`） | 64 位十六进制（每个 f32 的 IEEE-754 位型做 SHA-256） |
//! | `digest` | CLI 导出腿 `render_core::render_project_f32_with_assets_under` 返回的 `RenderProjectOutput::digest` | 同上 |
//! | 母带样本差 | MCP 落盘的 24-bit 母带文件（`rf64::parse_container` 取出 `data` 负载的整数码） vs CLI 的 `f32` 母带按**同一个**量化尺度取整 | LSB（24-bit，1 LSB = 1/8 388 608 满量程） |
//! | `sha256` | MCP 响应（整份文件字节） | 64 位十六进制 |
//!
//! ⚠ 两侧摘要**不是两份实现**：MCP 与 CLI 都在 `RenderOutput::digest_of`
//! （`yeban-render/src/render.rs:357`）上取 SHA-256，且都在**主总线增益之后、归一化 /
//! 抖动 / 量化之前**重算（MCP `domain/render.rs:2259`；CLI `render_core.rs:706`）。
//! 因此本判据真正测的是"**母带 f32 是否逐位相同**"，不是"两份哈希实现是否一致"。
//!
//! ## 三个夹具（§1.2）
//!
//! | 夹具 | 内容 | 期望 |
//! | :--- | :--- | :--- |
//! | A | 单条音频轨，**32-bit float** 立体声素材（两侧解码都走恒等路径） | 若前置条件全满足 ⇒ **可能真相同** |
//! | B | 单条 MIDI 轨（3 个音符，无概率触发 / 无 ratchet / 无微时值） | **应当不同**（`E1`：MCP 正弦振荡器 vs CLI `SynthEngine` 波表＋ADSR） |
//! | C | A ＋ B（两条轨，都直连主总线） | 给差值；**差值应 ≈ B 的差值**（音频部分逐位抵消） |
//!
//! ## 四个前置条件（写在判据里，逐条可反证）
//!
//! 1. **帧数对齐**：两侧都显式给 `FRAMES`（`#252` 起 MCP 支持显式 `frames`）；
//! 2. **`pan = 0`**：两侧都在 `pan != 0` 时才乘等功率曲线（MCP `domain/render.rs:3454`；
//!    CLI `audio_clip.rs:267`）⇒ `pan = 0` 是**恒等**（乘 `1.0`）；
//! 3. **`normalize = false`**：归一化在**摘要之后**（`domain/render.rs:2266` 之后才是第 10 步）；
//! 4. **请求采样率 = 工程采样率**（48 kHz）：MCP 按**请求**率渲染/重采样，CLI 的
//!    `WavExportOptions::resolve` 缺省跟随**工程**率。
//!
//! ## 反证（证明判据有牙，⛔ 不是空转）
//!
//! - **帧数不对齐** ⇒ 两侧摘要**必须不同**（多出的/少掉的样本都进哈希）；
//! - **`pan ≠ 0`** ⇒ 每一侧相对自己 `pan = 0` 的摘要**必须不同**（声相真的接上了）；
//! - **`normalize = true`** ⇒ 产物的**文件字节**必须不同、而 `masterDigest` **必须不变**
//!   （摘要时点在归一化之前）。⚠ 任务单 `§2.6` 的字面是"摘要必须不同"，它与 `§0.1`
//!   "摘要在归一化之前"以及两侧源码**互相矛盾**；本判据按**代码事实**断言，并把两个
//!   摘要原样打印出来。若哪天摘要真的随 `normalize` 变了，这里的等式断言会红 ——
//!   那正是"摘要口径分叉"的告警。
//! - **请求率 ≠ 工程率** ⇒ 摘要**必须不同**（这一条同时改了时长/重采样，是复合差异，
//!   只用来证明"请求率 = 工程率"这条前置条件不是装饰）。
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证 E1 应该怎么修（MCP 正弦 vs CLI 引擎是**已知**的产品面差异，本单只测量）；
//! ⛔ 不证跨平台（`f32` 超越函数走 `libm` 是 `ARCH-DET-001` 的承诺，本判据只在同机同
//! 工具链上钉读数）；⛔ 不证人耳听感。
//!
//! ⚠ 夹具全部写在 `std::env::temp_dir()` 下（[`Scratch`] 的 `Drop` 删除），⛔ 仓库里
//! 不留临时文件。

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use yeban_mcp::security::Channel;
use yeban_model::{
    AssetHash, AssetMetadata, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig,
    MediaKind, MidiNote, ProjectAudioConfig, RoutingEdge, RoutingGraph, RoutingKind, SampleRate,
    TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

use super::embedded_sfz_tools::{Scratch, call_tool};
use super::session_dispatcher;

/// 夹具速度：120 BPM / 960 PPQ ⇒ 1 tick = 25 帧 @ 48 kHz（可手算）。
const BPM: f64 = 120.0;
/// 采样率：工程率 ＝ 导出率 ＝ 请求率（前置条件 4）。
const SAMPLE_RATE: u32 = 48_000;
/// 工程末端：3840 tick = 2 秒 @ 120 BPM。
const END_TICK: u64 = 3_840;
/// 显式帧数：2 秒 @ 48 kHz。
const FRAMES: u64 = 96_000;
/// 音频素材帧数：1 秒 @ 48 kHz。
const ASSET_FRAMES: usize = 48_000;
/// 24-bit 量化尺度 `2^(bits-1)`（`yeban_render::dither::BitDepth::Int24::scale()`）。
const I24_SCALE: f32 = 8_388_608.0;

/// ⭐ **C1 的目标值**（夹具 A，四个前置条件全满足）：两侧的 64 位十六进制摘要。
///
/// 2026-10-20 实测（本机 stable，`line/mod-c1` @ `83ed9814`）；它只由 IEEE-754 基本
/// 运算（无超越函数）决定 ⇒ 跨平台稳定。**改它就是改判据**，必须同时说明原因。
const A_DIGEST: &str = "c7f59b8202bdbc18695f42c5478d622e04b7fcada097bf74d336ed2fe7eca7cf";

/// 夹具 B（纯 MIDI，`E1`）的差值量级 —— 2026-10-20 实测的**已知差异**，单位 24-bit LSB。
///
/// 用"量级区间"而不是精确字符串钉：MCP 侧是 `libm::sinf` 正弦、CLI 侧是引擎的波表
/// 振荡器，两者一旦有一边改音色，**摘要与这组数都会变** ⇒ 这里用 ±1% 的区间钉住"差值
/// 有多大"，既不放过数量级变化，也不把另一平台的末位差异误判成缺陷。
/// ⚠ 若 `E1` 被修好（两侧换成同一个 MIDI 音源），这三个数必须更新并记录**为什么**。
const B_RMS_LSB_2026_10_20: f64 = 1_213_278.260;
/// 最大绝对差（LSB）。
const B_MAX_ABS_LSB_2026_10_20: i64 = 4_951_650;
/// `|Δ| > 1 LSB` 的样本数（共 192000）。
const B_OVER_ONE_LSB_2026_10_20: usize = 93_240;

/// 夹具 C（混合）的差值量级 —— 2026-10-20 实测；应 ≈ B（音频部分逐位抵消）。
const C_RMS_LSB_2026_10_20: f64 = 1_203_212.698;
/// 最大绝对差（LSB）。
const C_MAX_ABS_LSB_2026_10_20: i64 = 4_951_650;
/// `|Δ| > 1 LSB` 的样本数（共 192000）。
const C_OVER_ONE_LSB_2026_10_20: usize = 93_224;

/// 一次渲染的完整读数（MCP ＋ CLI 并排）。
#[derive(Clone, Debug)]
struct Reading {
    /// 夹具/腿的名字（打印用）。
    tag: String,
    /// MCP 的 `data.masterDigest`。
    mcp_digest: String,
    /// CLI 的 `RenderProjectOutput::digest`。
    cli_digest: String,
    /// MCP 的整文件 `data.sha256`。
    mcp_sha256: String,
    /// MCP 的 `data.peak.before`（归一化前的 f32 母带峰值）。
    mcp_peak_before: f32,
    /// MCP 的 `data.peak.after`。
    mcp_peak_after: f32,
    /// MCP 的 `data.normalize.applied`。
    mcp_normalize_applied: bool,
    /// MCP 响应的 `data.frames`。
    mcp_frames: u64,
    /// CLI 的 `RenderProjectOutput::frames`。
    cli_frames: u64,
    /// CLI 母带（抖动/量化**之前**的 f32）的峰值。
    cli_peak: f32,
    /// 两侧母带的逐样本差（单位：24-bit LSB）。
    diff: SampleDiff,
}

impl Reading {
    /// 两个 `masterDigest` 是否相同。
    fn same(&self) -> bool {
        self.mcp_digest == self.cli_digest
    }

    /// 把读数逐行打印（`cargo test -- --nocapture` 直接可读）。
    fn print(&self) {
        println!(
            "### [{}] masterDigest: MCP={} CLI={} 相同={} 十六进制差={}/64",
            self.tag,
            self.mcp_digest,
            self.cli_digest,
            self.same(),
            digest_hex_distance(&self.mcp_digest, &self.cli_digest),
        );
        println!(
            "### [{}] 帧数: MCP={} CLI={} | 峰值: MCP.before={:.9} MCP.after={:.9} \
             CLI.f32={:.9} | normalize.applied={} | MCP.sha256={}",
            self.tag,
            self.mcp_frames,
            self.cli_frames,
            self.mcp_peak_before,
            self.mcp_peak_after,
            self.cli_peak,
            self.mcp_normalize_applied,
            self.mcp_sha256,
        );
        println!(
            "### [{}] 样本差（24-bit LSB）: MCP样本={} CLI样本={} 比较={} >1LSB={} \
             前4096内>1LSB={} max={} RMS={:.3} 首个>1LSB={:?}",
            self.tag,
            self.diff.mcp_len,
            self.diff.cli_len,
            self.diff.compared,
            self.diff.over_one_lsb,
            self.diff.over_one_lsb_first_4096,
            self.diff.max_abs_lsb,
            self.diff.rms_lsb,
            self.diff.first_over_one_lsb,
        );
    }
}

/// 两侧母带的逐样本差（单位：24-bit LSB）。
///
/// ⚠ MCP 落盘的是 **TPDF 抖动 + 24-bit 量化之后**的整数码，而 CLI 的 `digest` 作用在
/// **量化之前**的 f32 上 ⇒ 这里把 CLI 的 f32 按**同一个** `2^23` 尺度取整再比。
/// 因此即使两侧 f32 母带逐位相同，本读数也会有 **≤ 1 LSB** 的抖动底 —— 这正是"摘要
/// 在抖动之前"的可见证据（`>1LSB` 的计数为 0）。
#[derive(Clone, Copy, Debug)]
struct SampleDiff {
    /// MCP 母带的样本数（两侧声道合计）。
    mcp_len: usize,
    /// CLI 母带的样本数（两侧声道合计）。
    cli_len: usize,
    /// 比较了多少个样本（两侧声道合计；长度不一致时只比**公共前缀**）。
    compared: usize,
    /// `|Δ| > 1 LSB`（超出抖动底）的样本数。
    over_one_lsb: usize,
    /// 前 4096 个样本里 `|Δ| > 1 LSB` 的个数（"前 N 个不同样本数"的读数）。
    over_one_lsb_first_4096: usize,
    /// 最大绝对差（LSB）。
    max_abs_lsb: i64,
    /// 差值的 RMS（LSB）。
    rms_lsb: f64,
    /// 第一个 `|Δ| > 1 LSB` 的样本下标（没有 ⇒ `None`）。
    first_over_one_lsb: Option<usize>,
}

/// 两个 64 位十六进制摘要里**不同字符**的个数（0 = 逐字相同）。
fn digest_hex_distance(left: &str, right: &str) -> usize {
    left.bytes()
        .zip(right.bytes())
        .filter(|(a, b)| a != b)
        .count()
}

/// 固定 ULID（同一次测试里两次运行拿到**同一批**实体 ⇒ 计划节点序、求和序可复现）。
///
/// 26 字符 Crockford Base32；`01J` 前缀 ＋ 23 位十进制补零。
fn fixed(index: u128) -> EntityId {
    format!("01J{index:023}")
        .parse()
        .expect("固定夹具身份必须是 26 字符 Crockford Base32 ULID")
}

/// 空工程：4/4、120 BPM、48 kHz、一个主总线（⛔ 没有别的轨道）。
fn empty_project() -> YebanProjectV1 {
    let master = fixed(1);
    YebanProjectV1 {
        bpm: BPM,
        audio_config: ProjectAudioConfig {
            sample_rate: SampleRate::Hz48000,
            ..ProjectAudioConfig::default()
        },
        master_bus_track_id: master,
        tracks: BTreeMap::from([(
            master,
            TrackV3 {
                id: master,
                name: "Master".to_owned(),
                kind: TrackKind::Master,
                ..TrackV3::default()
            },
        )]),
        routing_graph: RoutingGraph {
            nodes: vec![master],
            ..RoutingGraph::default()
        },
        ..YebanProjectV1::default()
    }
}

/// 加一条直连主总线的轨（`TrackToBus`，⛔ 没有 aux send / sidechain）。
fn add_track(project: &mut YebanProjectV1, index: u128, kind: TrackKind, name: &str) -> EntityId {
    let id = fixed(index);
    project.tracks.insert(
        id,
        TrackV3 {
            id,
            name: name.to_owned(),
            kind,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(id);
    let edge = fixed(30 + index);
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: id,
            destination_node: project.master_bus_track_id,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    id
}

/// 把一段内容摆到轨上（`start_tick = 0`，`duration_ticks = END_TICK`）。
fn place_clip(
    project: &mut YebanProjectV1,
    track: EntityId,
    clip_index: u128,
    name: &str,
    content: ClipContent,
) {
    let clip_id = fixed(clip_index);
    project.clip_pool.insert(
        clip_id,
        ClipPoolEntry {
            id: clip_id,
            name: name.to_owned(),
            content,
        },
    );
    let placement_id = fixed(20 + clip_index);
    project
        .tracks
        .get_mut(&track)
        .expect("夹具轨必须存在")
        .clips
        .insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick: 0,
                duration_ticks: END_TICK,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
}

/// 一份**确定性**的 32-bit float 立体声素材（⛔ 不用 `sin` ⇒ 字节跨平台一致）。
///
/// 为什么必须是 **float32**：MCP 走 `yeban_decode`（整数 PCM 归一化 `/32768`），CLI 走
/// `yeban_render::wav::read_plain_wav`（`/full_scale_max` = `/32767`）—— **整数素材**会
/// 因此差一个 `32768/32767` 的常数因子（本判据的 A2 读数把它测出来）。float32 在两条
/// 解码路径上都是**恒等**（scale = 1.0），于是 A 只测渲染管线本身。
fn float_stereo_asset(scratch: &Scratch) -> (AssetHash, Vec<u8>) {
    let mut samples = Vec::with_capacity(ASSET_FRAMES * 2);
    for index in 0..ASSET_FRAMES {
        let left = (((index * 37) % 1000) as f32 / 1000.0 - 0.5) * 0.8;
        let right = (((index * 53 + 7) % 512) as f32 / 512.0 - 0.5) * 0.6;
        samples.push(left);
        samples.push(right);
    }
    let path = scratch.join("asset-f32-stereo.wav");
    write_plain_wav(
        &path,
        &PcmFormat::float(2, SAMPLE_RATE, 32),
        &PcmBuffer::Float32(samples),
    )
    .expect("写 32f 立体声夹具");
    let bytes = std::fs::read(&path).expect("读回 32f 立体声夹具");
    (AssetHash::of_bytes(&bytes), bytes)
}

/// 一份 **16-bit 整数**立体声素材（A2 读数：只用来测两侧解码归一化的口径差）。
fn int16_stereo_asset(scratch: &Scratch) -> (AssetHash, Vec<u8>) {
    let mut samples = Vec::with_capacity(ASSET_FRAMES * 2);
    for index in 0..ASSET_FRAMES {
        let left = (((index * 37) % 1000) as i32 - 500) * 16;
        let right = (((index * 53 + 7) % 512) as i32 - 256) * 16;
        samples.push(i16::try_from(left).expect("夹具在 i16 内"));
        samples.push(i16::try_from(right).expect("夹具在 i16 内"));
    }
    let path = scratch.join("asset-i16-stereo.wav");
    write_plain_wav(
        &path,
        &PcmFormat::integer(2, SAMPLE_RATE, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写 16-bit 立体声夹具");
    let bytes = std::fs::read(&path).expect("读回 16-bit 立体声夹具");
    (AssetHash::of_bytes(&bytes), bytes)
}

/// 把音频素材摆到一条轨上（并把元数据登记进 `project.assets`）。
fn add_audio_clip(
    project: &mut YebanProjectV1,
    track: EntityId,
    clip_index: u128,
    hash: &AssetHash,
    bytes: &[u8],
    gain_db: f32,
) {
    project.assets.insert(
        hash.clone(),
        AssetMetadata {
            hash: hash.clone(),
            original_path: "fixture.wav".to_owned(),
            byte_len: u64::try_from(bytes.len()).expect("夹具字节数放得进 u64"),
            media_kind: MediaKind::Audio,
            license: "test fixture (generated in-memory)".to_owned(),
        },
    );
    place_clip(
        project,
        track,
        clip_index,
        "audio",
        ClipContent::Audio {
            asset: hash.clone(),
            gain_db,
        },
    );
}

/// B/C 共用的 MIDI 内容：3 个音符（⛔ 概率触发 / ratchet / 微时值全关 ⇒ 纯音源差异）。
fn midi_content() -> ClipContent {
    let notes: Vec<(u64, u8, u64)> = vec![(0, 60, 480), (960, 64, 480), (1_920, 67, 480)];
    let mut map = BTreeMap::new();
    for (offset, (start_tick, pitch, duration_ticks)) in notes.into_iter().enumerate() {
        let id = fixed(100 + offset as u128);
        map.insert(id, MidiNote::new(id, start_tick, pitch, duration_ticks));
    }
    ClipContent::Midi { notes: map }
}

/// 夹具 A：纯音频片段工程（32f 素材，`pan = 0`，`gain = 0 dB`）。
fn pure_audio_project(scratch: &Scratch) -> (YebanProjectV1, Vec<(AssetHash, Vec<u8>)>) {
    let (hash, bytes) = float_stereo_asset(scratch);
    let mut project = empty_project();
    let track = add_track(&mut project, 2, TrackKind::Audio, "Audio");
    add_audio_clip(&mut project, track, 10, &hash, &bytes, 0.0);
    project.validate().expect("夹具 A 必须合法");
    (project, vec![(hash, bytes)])
}

/// 夹具 A2：纯音频片段工程，但素材是 **16-bit 整数**（测解码归一化口径）。
fn pure_int16_project(scratch: &Scratch) -> (YebanProjectV1, Vec<(AssetHash, Vec<u8>)>) {
    let (hash, bytes) = int16_stereo_asset(scratch);
    let mut project = empty_project();
    let track = add_track(&mut project, 2, TrackKind::Audio, "Audio");
    add_audio_clip(&mut project, track, 10, &hash, &bytes, 0.0);
    project.validate().expect("夹具 A2 必须合法");
    (project, vec![(hash, bytes)])
}

/// 夹具 B：纯 MIDI 工程。
fn pure_midi_project() -> YebanProjectV1 {
    let mut project = empty_project();
    let track = add_track(&mut project, 2, TrackKind::Midi, "MIDI");
    place_clip(&mut project, track, 10, "lead", midi_content());
    project.validate().expect("夹具 B 必须合法");
    project
}

/// 夹具 C：混合（音频轨 ＋ MIDI 轨，都直连主总线）。
fn mixed_project(scratch: &Scratch) -> (YebanProjectV1, Vec<(AssetHash, Vec<u8>)>) {
    let (hash, bytes) = float_stereo_asset(scratch);
    let mut project = empty_project();
    let audio = add_track(&mut project, 2, TrackKind::Audio, "Audio");
    add_audio_clip(&mut project, audio, 10, &hash, &bytes, 0.0);
    let midi = add_track(&mut project, 3, TrackKind::Midi, "MIDI");
    place_clip(&mut project, midi, 11, "lead", midi_content());
    project.validate().expect("夹具 C 必须合法");
    (project, vec![(hash, bytes)])
}

/// MCP 腿：**生产装配** [`session_dispatcher`] ＋ `yeban_render_master` 工具调用。
///
/// 资产字节经 `Domain::put_asset` 放进会话 CAS 池（与 `yeban_open_project` 从 `.yeban`
/// 容器填充的是**同一个** `assets: BTreeMap<AssetHash, Vec<u8>>`）。
fn mcp_render(
    project: &YebanProjectV1,
    assets: &[(AssetHash, Vec<u8>)],
    scratch: &Scratch,
    tag: &str,
    frames: u64,
    normalize: bool,
    sample_rate: u32,
) -> Value {
    let mut dispatcher = session_dispatcher(
        project,
        &scratch.join(&format!("{tag}.session.yeban")),
        false,
    )
    .expect("内嵌会话装配（生产路径的同一个符号）");
    for (_, bytes) in assets {
        dispatcher
            .domain_mut()
            .put_asset(bytes.clone())
            .expect("资产放进会话 CAS 池");
    }
    let out = scratch.join(&format!("{tag}.master.wav"));
    let result = call_tool(
        &mut dispatcher,
        Channel::Http,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": sample_rate,
            "frames": frames,
            "normalize": normalize,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(result["status"], "success", "{tag}: 渲染必须成功: {result}");
    result
}

/// CLI 腿：`render_core` 的**唯一** f32 导出入口（与 `--export-wav` 同一个函数）。
fn cli_render(
    project: &YebanProjectV1,
    assets: &[(AssetHash, Vec<u8>)],
    frames: u64,
    sample_rate: u32,
) -> crate::render_core::RenderProjectOutput {
    let options = crate::export_wav::WavExportOptions {
        frames,
        sample_rate,
        channels: 2,
        seed: project.rng_seed,
        use_engine_source: true,
        bit_depth: BitDepth::Int24,
    };
    crate::render_core::render_project_f32_with_assets_under(project, options, None, assets, None)
        .expect("CLI 导出腿必须能渲染这份工程")
}

/// CLI 的 f32 母带按 24-bit 尺度取整（与 `quantize_i24` 的**缩放**同口径，但不加抖动）。
fn quantize_like_export(sample: f32) -> i32 {
    let scaled = (sample * I24_SCALE).round();
    if scaled.is_nan() {
        return 0;
    }
    scaled.clamp(-8_388_608.0, 8_388_607.0) as i32
}

/// 把 MCP 落盘的母带文件解回整数码，与 CLI 的 f32 母带逐样本比。
///
/// ⚠ **不用 `yeban_render::wav::read_plain_wav`（hound）**：母带带 `bext`，它的
/// `codingHistory` 今天是 **641 字节（奇数）** ⇒ chunk 末尾有一个偶数补位字节，而
/// hound 3.5.1 的未知 chunk 跳过逻辑**不跳补位字节** ⇒ 实测报
/// `Hound("Failed to read enough bytes.")`（本判据第一次运行就撞上，已上报）。
/// 本函数改用本仓**自己的**容器解析器 [`yeban_render::rf64::parse_container`]：它按规范
/// 处理补位，`ParsedContainer::data` 给出 `data` 负载的精确字节区间。⛔ 这不是"绕过判据"：
/// 比的是 24-bit 整数码本身，读法只影响"能不能读到"，不影响读数。
fn sample_diff(path: &Path, cli_samples: &[f32]) -> SampleDiff {
    let bytes = std::fs::read(path).expect("读 MCP 母带字节");
    let parsed = yeban_render::rf64::parse_container(&bytes).expect("解析 MCP 母带容器");
    assert_eq!(
        parsed.format.channels, 2,
        "MCP 母带必须是立体声（MASTER_CHANNELS = 2），实际 {}",
        parsed.format.channels
    );
    assert_eq!(
        parsed.format.bits_per_sample, 24,
        "MCP 母带必须是 24-bit 整数 PCM，实际 {} bit",
        parsed.format.bits_per_sample
    );
    assert!(!parsed.format.is_float, "MCP 母带必须是整数 PCM");
    let payload = &bytes[parsed.data.clone()];
    let (three_byte_chunks, remainder) = payload.as_chunks::<3>();
    assert!(
        remainder.is_empty(),
        "MCP 母带负载必须是每样本 3 字节，实际 {} 字节",
        payload.len()
    );
    let mcp_codes: Vec<i32> = three_byte_chunks
        .iter()
        .map(|three| {
            let raw =
                i32::from(three[0]) | (i32::from(three[1]) << 8) | (i32::from(three[2]) << 16);
            if raw & 0x0080_0000 != 0 {
                raw - 0x0100_0000
            } else {
                raw
            }
        })
        .collect();
    // 长度不一致（"故意不对齐帧数"的反证）⇒ 只比**公共前缀**：前缀逐位相同、摘要却
    // 不同，说明差在长度而不是差在样本 —— 这正是"帧数也是摘要输入"的证据。
    let compared = mcp_codes.len().min(cli_samples.len());
    let mut over_one_lsb = 0usize;
    let mut over_one_lsb_first_4096 = 0usize;
    let mut max_abs_lsb = 0i64;
    let mut sum_sq = 0.0_f64;
    let mut first_over_one_lsb = None;
    for (index, (&mcp, &cli)) in mcp_codes.iter().zip(cli_samples).take(compared).enumerate() {
        let cli_code = quantize_like_export(cli);
        let diff = (i64::from(mcp) - i64::from(cli_code)).abs();
        if diff > 1 {
            over_one_lsb += 1;
            if index < 4_096 {
                over_one_lsb_first_4096 += 1;
            }
            if first_over_one_lsb.is_none() {
                first_over_one_lsb = Some(index);
            }
        }
        max_abs_lsb = max_abs_lsb.max(diff);
        sum_sq += (diff as f64) * (diff as f64);
    }
    SampleDiff {
        mcp_len: mcp_codes.len(),
        cli_len: cli_samples.len(),
        compared,
        over_one_lsb,
        over_one_lsb_first_4096,
        max_abs_lsb,
        rms_lsb: if compared == 0 {
            0.0
        } else {
            (sum_sq / compared as f64).sqrt()
        },
        first_over_one_lsb,
    }
}

/// 跑一条腿：MCP 渲染 ＋ CLI 渲染 ＋ 并排读数。
///
/// `mcp_frames` / `cli_frames` 分开给 ⇒ "故意不对齐帧数"的反证不需要第二份装配。
#[allow(clippy::too_many_arguments)]
fn measure(
    tag: &str,
    project: &YebanProjectV1,
    assets: &[(AssetHash, Vec<u8>)],
    scratch: &Scratch,
    mcp_frames: u64,
    cli_frames: u64,
    normalize: bool,
    mcp_rate: u32,
    cli_rate: u32,
) -> Reading {
    let response = mcp_render(
        project, assets, scratch, tag, mcp_frames, normalize, mcp_rate,
    );
    let cli = cli_render(project, assets, cli_frames, cli_rate);
    let out = scratch.join(&format!("{tag}.master.wav"));
    let diff = sample_diff(&out, &cli.samples);
    let reading = Reading {
        tag: tag.to_owned(),
        mcp_digest: response["data"]["masterDigest"]
            .as_str()
            .expect("data.masterDigest")
            .to_owned(),
        cli_digest: cli.digest.clone(),
        mcp_sha256: response["data"]["sha256"]
            .as_str()
            .expect("data.sha256")
            .to_owned(),
        mcp_peak_before: response["data"]["peak"]["before"]
            .as_f64()
            .expect("data.peak.before") as f32,
        mcp_peak_after: response["data"]["peak"]["after"]
            .as_f64()
            .expect("data.peak.after") as f32,
        mcp_normalize_applied: response["data"]["normalize"]["applied"]
            .as_bool()
            .expect("data.normalize.applied"),
        mcp_frames: response["data"]["frames"].as_u64().expect("data.frames"),
        cli_frames: cli.frames,
        cli_peak: cli
            .samples
            .iter()
            .fold(0.0_f32, |acc, sample| acc.max(sample.abs())),
        diff,
    };
    reading.print();
    reading
}

/// ⭐ 判据 A（§2.1）：**纯音频、四个前置条件全满足 ⇒ 两侧 `masterDigest` 逐字相同**。
///
/// 这是 `C1` 的**正面目标**：如果它为绿，则"两条导出管线在纯音频工程上逐位一致"
/// 拿到了第一个证据（摘要相同 ⇒ `digest_of` 的输入 f32 位型逐位相同）。
///
/// 非空证明（⛔ 不是"两边都静音"）：
/// - 母带峰值 > 0.1（真的出声）；
/// - 样本级差 `>1 LSB` 的计数 == 0 ⇒ 两侧 f32 一致到量化抖动底；
/// - 两侧 `digest` 都等于下面钉死的 64 位十六进制目标值。
#[test]
fn the_pure_audio_fixture_digests_are_identical_when_the_preconditions_hold() {
    let scratch = Scratch::new("digest-pure-audio");
    let (project, assets) = pure_audio_project(&scratch);
    let reading = measure(
        "A-pure-audio",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );

    assert_eq!(reading.mcp_frames, FRAMES, "MCP 必须按显式帧数渲染");
    assert_eq!(reading.cli_frames, FRAMES, "CLI 必须按显式帧数渲染");
    assert_eq!(reading.mcp_digest.len(), 64, "摘要必须是 64 位十六进制");
    assert!(
        reading.cli_peak > 0.1,
        "夹具 A 必须真的出声（峰值 {:.9} 太小 ⇒ 相等是空转）",
        reading.cli_peak
    );
    assert!(
        reading.same(),
        "⭐ C1/夹具 A：同一工程、帧数对齐、pan=0、normalize=false、请求率=工程率时，\
         两侧 masterDigest **必须相同**。MCP={} CLI={}（十六进制差 {}/64，样本差 RMS={:.3} LSB）\
         —— 若这条红，请把两个值、差值与原因原样上报，⛔ 不要放宽判据。",
        reading.mcp_digest,
        reading.cli_digest,
        digest_hex_distance(&reading.mcp_digest, &reading.cli_digest),
        reading.diff.rms_lsb,
    );
    // ⭐ §323：目标值就是那个 64 位十六进制串 —— 两侧都必须逐字等于它。
    assert_eq!(
        reading.mcp_digest, A_DIGEST,
        "MCP 的 masterDigest 必须等于钉死的 C1 目标值"
    );
    assert_eq!(
        reading.cli_digest, A_DIGEST,
        "CLI 的 digest 必须等于钉死的 C1 目标值"
    );
    assert_eq!(
        reading.mcp_peak_before.to_bits(),
        reading.cli_peak.to_bits(),
        "摘要相同 ⇒ 两侧 f32 母带峰值也必须逐位相同：{:.9} vs {:.9}",
        reading.mcp_peak_before,
        reading.cli_peak
    );
    assert_eq!(
        reading.diff.over_one_lsb, 0,
        "两侧 f32 必须逐位一致 ⇒ 量化差只剩 TPDF 抖动底（>1 LSB 的样本数必须是 0，\
         实际 {} / {}）",
        reading.diff.over_one_lsb, reading.diff.compared
    );
}

/// ⭐ 判据 A2（§1.2 的额外读数）：**16-bit 整数素材** ⇒ 两侧解码归一化口径不同。
///
/// MCP 走 `yeban_decode`（symphonia：整数 `/32768`），CLI 走
/// `read_plain_wav`（`/full_scale_max` = `/32767`）⇒ 母带差一个 `32768/32767` 的
/// 常数因子，摘要**必然不同**。⚠ 这不是 `E1`，也不在 `#250` 的六条清单里 —— 它是本判据
/// 第一次把"纯音频也不总是逐位一致"测出来的地方。本判据只**登记**它（断言不同 ＋ 量级），
/// ⛔ 不改任何一侧的解码器（任务单 `§1.4`）。
#[test]
fn the_int16_audio_fixture_is_known_different_because_the_two_decoders_normalize_differently() {
    let scratch = Scratch::new("digest-int16-audio");
    let (project, assets) = pure_int16_project(&scratch);
    let reading = measure(
        "A2-int16-audio",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );

    assert!(
        !reading.same(),
        "16-bit 素材的两侧解码归一化（/32768 vs /32767）不同 ⇒ 摘要必须不同；\
         若相同，说明有一条解码腿被换掉了：MCP={} CLI={}",
        reading.mcp_digest,
        reading.cli_digest
    );
    assert!(
        reading.diff.over_one_lsb > 0,
        "整数素材的归一化差必须体现在样本上（>1 LSB 的样本数 > 0）"
    );
    let ratio = f64::from(reading.mcp_peak_before) / f64::from(reading.cli_peak);
    println!(
        "### [A2] MCP/CLI 峰值比 = {ratio:.9}（32767/32768 = {:.9}）",
        32_767.0_f64 / 32_768.0
    );
    assert!(
        (ratio - 32_767.0_f64 / 32_768.0).abs() < 1.0e-6,
        "16-bit 素材的 MCP/CLI 峰值比必须 ≈ 32767/32768（MCP 归一化 /32768、CLI /32767），\
         实际 {ratio:.9}"
    );
}

/// ⭐ 判据 B（§2.2）：**纯 MIDI 工程 ⇒ 两侧摘要已知不同，差值钉死**。
///
/// 原因（`E1`）：MCP 用 `MidiSynthSource`（正弦 ＋ 线性 attack/release 包络，
/// `domain/render.rs:2616`），CLI 用 `SynthEngine`（波表振荡器 ＋ ADSR）⇒ 母带**不是**
/// 同一个信号。本单只测量"差多少"，⛔ 不修。
#[test]
fn the_pure_midi_fixture_digests_are_known_different_and_the_gap_is_measured() {
    let scratch = Scratch::new("digest-pure-midi");
    let project = pure_midi_project();
    let reading = measure(
        "B-pure-midi",
        &project,
        &[],
        &scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );

    assert!(!reading.same(), "夹具 B 的摘要必须不同（E1 的正面证据）");
    assert!(
        reading.cli_peak > 0.01 && reading.mcp_peak_before > 0.01,
        "两侧都必须真的出声（MCP {:.9} / CLI {:.9}）",
        reading.mcp_peak_before,
        reading.cli_peak
    );
    assert!(
        reading.diff.over_one_lsb > 0,
        "E1 必须体现在样本上（>1 LSB 的样本数 > 0）"
    );
    // ⭐ "钉死差值有多大"：最大值与超抖动底样本数逐值相等；RMS 钉在 ±1% 区间内
    // （常数是 2026-10-20 的实测值，见常量文档）。
    assert_eq!(
        reading.diff.max_abs_lsb, B_MAX_ABS_LSB_2026_10_20,
        "夹具 B 的最大绝对差必须等于钉死值"
    );
    assert_eq!(
        reading.diff.over_one_lsb, B_OVER_ONE_LSB_2026_10_20,
        "夹具 B 的 `|Δ| > 1 LSB` 样本数必须等于钉死值"
    );
    let b_low = B_RMS_LSB_2026_10_20 * 0.99;
    let b_high = B_RMS_LSB_2026_10_20 * 1.01;
    assert!(
        (b_low..=b_high).contains(&reading.diff.rms_lsb),
        "夹具 B 的差值 RMS 必须落在钉死区间 [{b_low:.3}, {b_high:.3}] LSB 内，\
         实际 {:.3} LSB",
        reading.diff.rms_lsb
    );
}

/// ⭐ 判据 C（§2.3）：**混合工程 ⇒ 给差值，且差值 ≈ B 的差值**（音频部分逐位抵消）。
///
/// 这条腿把"差值的来源"钉到 MIDI 音源：C 的音频轨与 A 逐位相同，因此
/// `MCP(C) - CLI(C)` 只剩 MIDI 那一路的差，量级应与 B 的差一致（容差 2%）。
#[test]
fn the_mixed_fixture_digests_are_known_different_and_the_gap_comes_from_midi() {
    let scratch = Scratch::new("digest-mixed");
    let (project, assets) = mixed_project(&scratch);
    let reading = measure(
        "C-mixed",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );

    assert!(!reading.same(), "夹具 C（混合）的摘要必须不同");
    assert!(
        reading.cli_peak > 0.1,
        "夹具 C 必须真的出声（峰值 {:.9}）",
        reading.cli_peak
    );
    assert_eq!(
        reading.diff.max_abs_lsb, C_MAX_ABS_LSB_2026_10_20,
        "夹具 C 的最大绝对差必须等于钉死值"
    );
    assert_eq!(
        reading.diff.over_one_lsb, C_OVER_ONE_LSB_2026_10_20,
        "夹具 C 的 `|Δ| > 1 LSB` 样本数必须等于钉死值"
    );
    let c_low = C_RMS_LSB_2026_10_20 * 0.98;
    let c_high = C_RMS_LSB_2026_10_20 * 1.02;
    assert!(
        (c_low..=c_high).contains(&reading.diff.rms_lsb),
        "夹具 C 的差值 RMS 必须落在钉死区间 [{c_low:.3}, {c_high:.3}] LSB 内，\
         实际 {:.3} LSB",
        reading.diff.rms_lsb
    );
    // 与 B 的差值比：同一个 MIDI 内容 ⇒ 差值向量必须几乎相同。
    let b_scratch = Scratch::new("digest-mixed-baseline");
    let b = measure(
        "C-baseline-B",
        &pure_midi_project(),
        &[],
        &b_scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );
    let relative = (reading.diff.rms_lsb - b.diff.rms_lsb).abs() / b.diff.rms_lsb.max(1.0e-9);
    println!(
        "### [C] 差值 RMS: C={:.3} LSB vs B={:.3} LSB，相对差={:.6}",
        reading.diff.rms_lsb, b.diff.rms_lsb, relative
    );
    assert!(
        relative < 0.02,
        "C 的差值必须 ≈ B 的差值（音频部分逐位抵消）：相对差 {relative:.6}"
    );
}

/// ⭐ 反证 1（§2.4）：**故意不对齐帧数 ⇒ 摘要必须不同**。
///
/// 四段读数：
/// - 两侧都用 **96000** 帧 ⇒ 相同；两侧都用 **48000** 帧 ⇒ 也相同（对齐档的正对照）；
/// - 同一侧换帧数（96000 → 48000）⇒ 各自摘要都变；
/// - MCP 96000 vs CLI 48000 ⇒ 不同（多出的/少掉的样本都进哈希）；
/// - 但两侧的**公共前缀**逐位相同 ⇒ 差只来自长度，⛔ 不是音源变化。
#[test]
fn misaligned_frames_make_the_digests_differ() {
    let scratch = Scratch::new("digest-frames");
    let (project, assets) = pure_audio_project(&scratch);

    let aligned_full = measure(
        "R1-aligned-full",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );
    assert!(aligned_full.same(), "96000 帧两侧对齐时仍必须相同");
    let aligned_half = measure(
        "R1-aligned-half",
        &project,
        &assets,
        &scratch,
        FRAMES / 2,
        FRAMES / 2,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );
    assert!(
        aligned_half.same(),
        "48000 帧两侧对齐时仍必须相同（否则反证没有隔离变量）"
    );

    let mismatched = measure(
        "R1-misaligned",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES / 2,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );
    assert!(
        !mismatched.same(),
        "⭐ 帧数不对齐 ⇒ 摘要**必须不同**（MCP {} vs CLI {}）",
        mismatched.mcp_digest,
        mismatched.cli_digest
    );
    assert_ne!(
        aligned_full.mcp_digest, aligned_half.mcp_digest,
        "同一侧换帧数也必须改摘要（MCP）"
    );
    assert_ne!(
        aligned_full.cli_digest, aligned_half.cli_digest,
        "同一侧换帧数也必须改摘要（CLI）"
    );
    assert_eq!(
        mismatched.mcp_digest, aligned_full.mcp_digest,
        "MCP 96000 帧那一侧与对齐档逐字相同（反证只改了 CLI 侧）"
    );
    assert_eq!(
        mismatched.cli_digest, aligned_half.cli_digest,
        "CLI 48000 帧那一侧与对齐档逐字相同（反证只改了 MCP 侧）"
    );
    assert_eq!(
        mismatched.diff.over_one_lsb, 0,
        "帧数不对齐时**公共前缀**必须仍逐位相同（差只来自长度，⛔ 不是音源变化）"
    );
    assert_eq!(
        mismatched.diff.compared,
        (FRAMES / 2) as usize * 2,
        "公共前缀必须是被截短那一侧的全部样本"
    );
}

/// ⭐ 反证 2（§2.5）：**`pan ≠ 0` ⇒ 每一侧相对 `pan = 0` 的摘要必须不同**。
///
/// 若"相同" ⇒ 声相没接上（MCP 的 `pan_gains` 或 CLI 的 `track_pan_gains` 没进音频路径），
/// 那要**报**给集成者，⛔ 不是调判据。额外读数：`pan ≠ 0` 时两侧摘要**仍相同**
/// ⇒ 两条等功率曲线逐位同解（这是 A 之外的第二条 C1 证据）。
#[test]
fn a_nonzero_pan_changes_each_side_and_the_two_sides_still_agree() {
    let scratch = Scratch::new("digest-pan");
    let (mut project, assets) = pure_audio_project(&scratch);
    let plain = measure(
        "R2-pan0",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );

    let track = fixed(2);
    project.tracks.get_mut(&track).expect("夹具 A 的音频轨").pan = -0.5;
    project.validate().expect("pan ≠ 0 的夹具必须合法");
    let panned = measure(
        "R2-pan-minus-0.5",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );

    assert_ne!(
        panned.cli_digest, plain.cli_digest,
        "⭐ CLI：pan ≠ 0 必须改变摘要（否则声相没接上）"
    );
    assert_ne!(
        panned.mcp_digest, plain.mcp_digest,
        "⭐ MCP：pan ≠ 0 必须改变摘要（否则声相没接上）"
    );
    assert_eq!(
        panned.mcp_digest, panned.cli_digest,
        "pan ≠ 0 时两侧仍必须逐字相同（同一条等功率曲线）：MCP={} CLI={}",
        panned.mcp_digest, panned.cli_digest
    );
    assert_eq!(
        panned.diff.over_one_lsb, 0,
        "pan ≠ 0 时两侧 f32 仍必须逐位一致（只剩抖动底）"
    );
}

/// ⭐ 反证 3（§2.6）：**`normalize = true` ⇒ 产物文件必须不同、摘要必须不变**。
///
/// 代码事实（`domain/render.rs:2255-2266`）：第 9 步重算 `masterDigest`，第 10 步才归一化。
/// ⇒ 归一化**不进摘要**；若这里摘要变了，说明摘要时点被挪到了归一化之后（口径分叉）。
///
/// ⚠ 任务单 `§2.6` 的字面要求是"摘要必须不同（归一化在摘要之后）"—— 这与
/// `§0.1` 的"两侧摘要在归一化之前"**互相矛盾**。本判据按**代码事实**断言，并把两个摘要
/// 与两个文件 `sha256` 原样打印；报告里登记这条冲突，⛔ 不擅自选边。
#[test]
fn normalize_changes_the_file_but_not_the_master_digest() {
    let scratch = Scratch::new("digest-normalize");
    let (project, assets) = pure_audio_project(&scratch);
    let plain = measure(
        "R3-normalize-false",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );
    let normalized = measure(
        "R3-normalize-true",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES,
        true,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );

    assert!(
        normalized.mcp_normalize_applied,
        "normalize=true 必须真的施加（响应 data.normalize.applied）"
    );
    assert!(
        normalized.mcp_peak_after > plain.mcp_peak_before * 1.5,
        "归一化必须把峰值抬上去：before={:.9} after={:.9}",
        plain.mcp_peak_before,
        normalized.mcp_peak_after
    );
    assert_eq!(
        normalized.mcp_digest, plain.mcp_digest,
        "⭐ 归一化在摘要**之后** ⇒ normalize=true/false 的 masterDigest **必须相同**；\
         若不同 ⇒ 摘要口径分叉（报我）：true={} false={}",
        normalized.mcp_digest, plain.mcp_digest
    );
    assert_ne!(
        normalized.mcp_sha256, plain.mcp_sha256,
        "归一化必须改变**文件字节**（sha256 是抖动/量化之后的产物）"
    );
    assert!(
        normalized.diff.over_one_lsb > 0,
        "归一化后的 MCP 母带必须与 CLI（无归一化）在样本上明显不同"
    );
    println!(
        "### [R3] 摘要不变={} 文件 sha256 变={} | normalize=true 的 MCP 与未归一化 CLI 的样本差 RMS={:.3} LSB",
        normalized.mcp_digest == plain.mcp_digest,
        normalized.mcp_sha256 != plain.mcp_sha256,
        normalized.diff.rms_lsb,
    );
}

/// ⭐ 反证 4（§1.3 的采样率前置）：**请求率 ≠ 工程率 ⇒ 摘要必须不同**。
///
/// ⚠ 这是**复合**差异（请求率变了，2 秒对应的帧数也从 96000 变 88200，且音频素材要被
/// 重采样）—— 它只证明"请求率 = 工程率"这条前置条件**不是装饰**，⛔ 不隔离采样率一项。
#[test]
fn a_request_rate_different_from_the_project_rate_changes_the_digests() {
    let scratch = Scratch::new("digest-rate");
    let (project, assets) = pure_audio_project(&scratch);
    let aligned = measure(
        "R4-rate-aligned",
        &project,
        &assets,
        &scratch,
        FRAMES,
        FRAMES,
        false,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );
    // 2 秒 @ 44.1 kHz = 88200 帧；CLI 仍按工程率 48 kHz / 96000 帧导出。
    let mismatched = measure(
        "R4-rate-mismatched",
        &project,
        &assets,
        &scratch,
        88_200,
        FRAMES,
        false,
        44_100,
        SAMPLE_RATE,
    );
    assert!(aligned.same(), "对齐档必须相同（反证前的正对照）");
    assert!(
        !mismatched.same(),
        "⭐ 请求率 ≠ 工程率 ⇒ 摘要必须不同（MCP 88.2k vs CLI 48k）"
    );
    assert_ne!(mismatched.mcp_frames, mismatched.cli_frames);
}
