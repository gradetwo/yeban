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
//! ## 四个夹具（A / A2 / B / C）
//!
//! | 夹具 | 内容 | 期望 |
//! | :--- | :--- | :--- |
//! | A | 单条音频轨，**32-bit float** 立体声素材（两侧解码都走恒等路径） | **逐位相同** |
//! | A2 | 单条音频轨，**16-bit 整数**素材（`#254` 的对齐全量程） | **逐位相同** |
//! | B | 单条 MIDI 轨（3 个音符，无概率触发 / 无 ratchet / 无微时值） | ⭐ `#268`：MCP 也走 `SynthEngine` ⇒ **逐位相同**（修前是"已知不同"，见下） |
//! | C | A ＋ B（两条轨，都直连主总线） | 同上 ⇒ **逐位相同** |
//!
//! ## `#268` 改了什么（旧→新）
//!
//! `#253` 的读数（`E1`）：MCP 用 `MidiSynthSource`（正弦 ＋ 线性 5/10 ms 包络）、
//! CLI 用 `SynthEngine`（加法波表 `HOLLOW` ＋ ADSR ＋ 双振荡器）⇒ B 差 `63/64`
//! 个十六进制字符、C 差 `58/64`，峰值差 `0.1512%`。`ADR-0011`（负责人 2026-10-21
//! 逐字裁决「统一」）之后 MCP 的音源就是引擎音源 ⇒ B/C 应变成**逐位相同**。
//! 本文件因此把那两条判据从"已知不同 + 钉差值"改成"**必须相同** + 钉新摘要"。
//! ⚠ A / A2 的常量**一位不动**（音频路径没有换）：它们是"没有退化"的对照。
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
//! ⛔ 不证跨块的音源状态契约（引擎音源是**有状态**的：推进顺序的判据在本仓库的
//! `yeban-render::engine_source::tests`，不在本文件）；
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
    SamplerBinding, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::{BitDepth, PcmBuffer, quantize};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

use crate::export_wav::write_interleaved_wav;

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

/// ⭐ **夹具 A2（16-bit 整数素材）修好后的目标值**（`#254`，2026-10-20 本机实测）。
///
/// 修前（`#253`）两侧是**两**个串（MCP `896faecb…` / CLI `cdb02cd1…`，差 `59/64`），
/// 因为 CLI 那条腿把整数除 `full_scale_max`（`2^(n-1) - 1`）。`#254` 把它对齐到
/// `2^(bits-1)`（`BitDepth::scale()`）⇒ 两侧**同一个**串。
/// ⚠ 改它就是改判据：必须先说明为什么。
const A2_DIGEST: &str = "896faecb4cb23de1737ef9fffef9d93abe7177878a30ed0bbb155b1bc3c57ff1";

/// 夹具 A2 修好后的母带峰值 `f32` **位型**（`0x3e7a0000` = `0.244140625`）。
///
/// 修前 MCP/CLI 是 `0.244140625` / `0.244148076`（比 `0.999969483` = `32767/32768`）；
/// 修后**逐位相同**。⛔ 这里必须钉位型而不是近似值：峰值比正是本单的判据。
const A2_PEAK_BITS: u32 = 0x3e7a_0000;

/// ⭐ 夹具 B（纯 MIDI）**统一之后的**目标值（`#268` / `ADR-0011`，2026-10-21 本机实测）。
///
/// 修前（`#253`）两侧是**两**个串（MCP 正弦 vs CLI 引擎，差 `63/64`）；`#268` 之后
/// MCP 的音源就是 `yeban-engine` 的 `SynthEngine`，两侧因此**同一个串**。
/// ⚠ 改它就是改判据：必须先说明为什么。
const B_DIGEST: &str = "a0adca6f2fdbc86847958bf15beaec295596795b789677c886c674303a2c0c9b";

/// ⭐ 夹具 C（混合）**统一之后的**目标值（`#268`，2026-10-21 本机实测）。
///
/// 它 = 同一份 MIDI 内容（引擎音色）⊕ 夹具 A 的音频；音频部分两侧早已逐位相同。
const C_DIGEST: &str = "264e4d68629cee58962cdfb2233e422897945fb5e0d1b4d7ed2b28f7098635f9";

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
/// 为什么必须是 **float32**：夹具 A 要测的是**渲染管线本身**，⛔ 不是解码归一化。
/// 浮点在两条解码路径上都是**恒等**（scale = `1.0`）⇒ 不引入任何分母。
/// ⚠ 整数素材的两侧分母在 `#254` 之前确实不同（CLI `/32767` vs MCP `/32768`）；
/// 那条差异现在由判据 A2 单独钉住（`#254` 已把 CLI 对齐到 `2^(bits-1)` ⇒ 两侧相同），
/// 因此 A 用 float32 只是为了把"解码"这个自变量从 A 里摘掉，不是因为"整数必然不同"。
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

/// MCP 母带文件的 **`data` 负载**（24-bit 整数码的原始字节，按样本顺序）。
///
/// 解析口径与理由见 [`sample_diff`] 的文档（不用 hound，改用本仓的 rf64 解析器）。
fn mcp_pcm_payload(path: &Path) -> Vec<u8> {
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
    bytes[parsed.data.clone()].to_vec()
}

/// 把 MCP 落盘的母带 `data` 负载解回 24-bit 整数码（PCM 是小端三字节）。
fn mcp_pcm_codes(path: &Path) -> Vec<i32> {
    let payload = mcp_pcm_payload(path);
    let (three_byte_chunks, remainder) = payload.as_chunks::<3>();
    assert!(
        remainder.is_empty(),
        "MCP 母带负载必须是每样本 3 字节，实际 {} 字节",
        payload.len()
    );
    three_byte_chunks
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
        .collect()
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
    let mcp_codes = mcp_pcm_codes(path);
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

/// ⭐ 判据 A2（§1.2 的额外读数）：**16-bit 整数素材** ⇒ 两侧解码归一化**必须同口径**。
///
/// ## 历史（`#253` 测出，`#254` 修好）
///
/// ## 这条判据的 CLI 腿**落在哪个函数**（`#254` 注入实测）
///
/// 夹具 A2 是**音频片段**（`add_audio_clip`）⇒ CLI 腿走
/// `crate::audio_clip::normalize_samples`，⛔ **不是** `yeban_render::wav::read_plain_wav_mono_f32`。
/// 两处用的是**同一张归一化表**（`#253` 时都是 `BitDepth::full_scale_max()` = `2^(n-1) − 1`）。
/// 实测：只把 `read_plain_wav_mono_f32` 改回 `/32767` ⇒ **本条不红**（它只影响 SFZ／WAV
/// 素材腿）；把 `normalize_samples` 改回 `/32767` ⇒ 本条立刻红。⇒ 两侧分母都在 `#254` 里改了。
///
/// `#253` 首次测出：MCP 走 `yeban_decode`（symphonia：整数 `/32768`），CLI 走
/// `full_scale_max` 那一路（`/32767`）⇒ 母带差一个 `32768/32767` 的常数因子。
/// 当时的实测（本机 stable，`main` @ `f510a1a0`）：
///
/// | 量 | `#253` 读数（修前） |
/// | :--- | :--- |
/// | MCP `masterDigest` | `896faecb4cb23de1737ef9fffef9d93abe7177878a30ed0bbb155b1bc3c57ff1` |
/// | CLI `digest` | `cdb02cd103d02f901a1e449580693457618d249ffc5f7511b312a8f47e3774a4` |
/// | 十六进制差 | `59/64` |
/// | 峰值 | MCP `0.244140625` / CLI `0.244148076` |
/// | **MCP/CLI 峰值比** | **`0.999969483`**（= `32767/32768`） |
/// | 样本差 | `max=64` LSB、`RMS=20.286` LSB、`>1 LSB` 的样本 `92718/192000` |
///
/// ## 为什么 `/32768` 才对（`#254` 的裁决）
///
/// 二补码 16-bit 的区间是 `[-32768, 32767]` —— **负端比正端多一个码**。
/// 除 `32768` ⇒ `[-1.0, 0.999969482]`（对称、不越界，`-32768` **正好** `-1.0`）；
/// 除 `32767` ⇒ `[-1.0000305, 1.0]`（**负半轴越出满量程 1 LSB**）。
/// 且 `BitDepth::scale()`（`2^(n-1)`）正是量化器的缩放系数，解码该是它的逆。
/// ⇒ `#254` 把 CLI 的两条腿都改成 `2^(n-1)`（`yeban-render/src/wav.rs` 的
/// `read_plain_wav_mono_f32` ＋ `yeban-app/src/audio_clip.rs` 的 `normalize_samples`），
/// **MCP 侧一个字节都没动**（symphonia 本来就是 `2^(n-1)`）。
///
/// 修后本判据断言：两侧摘要**相同**、母带峰值**逐位相同**、`>1 LSB` 的样本数为 `0`。
/// ⚠ 这条判据现在有牙：任何一侧把分母改成 `2^(n-1) - 1`（或别的）都会让它红。
#[test]
fn the_int16_audio_fixture_digests_are_identical_after_the_full_scale_normalization_fix() {
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
        reading.same(),
        "16-bit 素材的两侧解码归一化都是 2^(bits-1)（`#254`）⇒ 摘要必须相同；\
         若不同，说明有一条解码腿把分母改回了 2^(bits-1) - 1：MCP={} CLI={}",
        reading.mcp_digest,
        reading.cli_digest
    );
    assert_eq!(
        reading.mcp_digest, A2_DIGEST,
        "MCP 侧的 16-bit 素材摘要必须逐字等于 `#254` 实测值"
    );
    assert_eq!(
        reading.cli_digest, A2_DIGEST,
        "CLI 侧的 16-bit 素材摘要必须逐字等于 `#254` 实测值（修前是 cdb02cd1…）"
    );
    assert_eq!(
        reading.mcp_peak_before.to_bits(),
        reading.cli_peak.to_bits(),
        "摘要相同 ⇒ 两侧 f32 母带峰值也必须逐位相同：{:.9} vs {:.9}",
        reading.mcp_peak_before,
        reading.cli_peak
    );
    assert_eq!(
        reading.mcp_peak_before.to_bits(),
        A2_PEAK_BITS,
        "16-bit 素材的母带峰值必须逐位等于 `#254` 实测值（{:.9}）",
        reading.mcp_peak_before
    );
    assert_eq!(
        reading.diff.over_one_lsb, 0,
        "两侧 f32 必须逐位一致 ⇒ 量化差只剩 TPDF 抖动底（>1 LSB 的样本数必须是 0，\
         实际 {} / {}）",
        reading.diff.over_one_lsb, reading.diff.compared
    );
    let ratio = f64::from(reading.mcp_peak_before) / f64::from(reading.cli_peak);
    println!(
        "### [A2] MCP/CLI 峰值比 = {ratio:.9}（修前 0.999969483 = 32767/32768；修后必须 = 1）"
    );
    assert_eq!(
        ratio, 1.0,
        "修后两侧峰值比必须**正好** 1.0（⛔ 不再是 32767/32768 = 0.999969483）"
    );
}

/// ⭐ 判据 B（§2.2）：**纯 MIDI 工程 ⇒ 两侧摘要逐位相同**（`#268` / `ADR-0011`）。
///
/// 修前（`#253` 的 `E1` 读数）：MCP 用 `MidiSynthSource`（正弦 ＋ 线性包络）、CLI 用
/// `SynthEngine`（波表 ＋ ADSR）⇒ 摘要差 `63/64`、峰值差 `0.1512%`。负责人逐字裁决
/// 「统一」之后两条路只有**一个**音源 ⇒ 本判据反过来要求**逐位相同**：任何一侧把
/// 音源换回去（或换块长/换采样口径）都会让它红。
#[test]
fn the_pure_midi_fixture_digests_are_identical_after_the_engine_voice_unification() {
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

    assert!(
        reading.same(),
        "纯 MIDI 夹具两侧必须逐位相同（MCP 与 CLI 都走 SynthEngine）；\
         若不同，说明有一条腿还在用别的 MIDI 音源：MCP={} CLI={}",
        reading.mcp_digest,
        reading.cli_digest
    );
    assert!(
        reading.cli_peak > 0.01 && reading.mcp_peak_before > 0.01,
        "两侧都必须真的出声（MCP {:.9} / CLI {:.9}）",
        reading.mcp_peak_before,
        reading.cli_peak
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
    assert_eq!(
        reading.mcp_digest, B_DIGEST,
        "MCP 侧的纯 MIDI 摘要必须逐字等于 `#268` 实测值（修前是已知不同的两个串）"
    );
}

/// ⭐ 判据 C（§2.3）：**混合工程 ⇒ 两侧摘要逐位相同**（`#268` / `ADR-0011`）。
///
/// C = 夹具 A 的音频 ⊕ 夹具 B 的 MIDI。音频那一路两侧早就是同一份实现（A / A2 钉住），
/// MIDI 那一路现在也是同一份引擎音源 ⇒ 整条母带逐位相同。修前（`#253`）这里差
/// `58/64`，且差值几乎等于 B 的差值（音频部分抵消）—— 那条读数已经随 `E1` 修好退休。
#[test]
fn the_mixed_fixture_digests_are_identical_after_the_engine_voice_unification() {
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

    assert!(
        reading.same(),
        "混合夹具两侧必须逐位相同：MCP={} CLI={}",
        reading.mcp_digest,
        reading.cli_digest
    );
    assert!(
        reading.cli_peak > 0.1,
        "夹具 C 必须真的出声（峰值 {:.9}）",
        reading.cli_peak
    );
    assert_eq!(
        reading.mcp_peak_before.to_bits(),
        reading.cli_peak.to_bits(),
        "摘要相同 ⇒ 两侧 f32 母带峰值也必须逐位相同"
    );
    assert_eq!(
        reading.diff.over_one_lsb, 0,
        "两侧 f32 逐位一致 ⇒ `|Δ| > 1 LSB` 的样本数必须是 0，实际 {} / {}",
        reading.diff.over_one_lsb, reading.diff.compared
    );
    assert_eq!(
        reading.mcp_digest, C_DIGEST,
        "MCP 侧的混合夹具摘要必须逐字等于 `#268` 实测值"
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

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#272`：**采样器的 C1 判据从"峰值相等"收紧成"字节相等"**
// ---------------------------------------------------------------------------
//
// ## 为什么这一节存在（`#270` / `#271` 的逐字事实）
//
// 既有的采样器判据（`embedded_sampler.rs:255-262`）只断言 **峰值** 相同：
// MCP 的 `SamplerTrackSource`（`domain/sfz.rs::prepare_sampler`，逐音符烘 PCM）与
// CLI 的引擎 SFZ 支路（`render_core` → `arm_pending_sfz` → `SynthEngine`）今天
// 在**四个**区域参数上走的是**两条不同的实现**：
//
// | 区域参数 | 引擎（CLI） | MCP | 今天的差 |
// | :--- | :--- | :--- | :--- |
// | `loop_mode` / `loop_start` / `loop_end` | 回绕（`mix_span_into`） | 只响一遍（`render_pcm_into`） | 有 |
// | `offset` / `end` | 按 `SampleSpan` 切片 | 从 0 播到样本尾 | 有 |
// | `ampeg_*` | 幅度包络（`AmpegRun`） | 不应用 | 有 |
// | 多个 `region` 命中同一音符 | **全部叠加**（每个 region 是独立声部） | **只取第一条**（`.find`） | 有 |
//
// 因此"峰值相等"只是"峰一致"，⛔ 不是字节一致。本节的判据对象换成
// `masterDigest` / `digest`（都是 `RenderOutput::digest_of`：对母带每个 `f32` 的
// IEEE-754 位型做 SHA-256，在主总线增益之后、抖动/量化之前重算）——
// **两侧同一个哈希函数**，所以它测的是"母带 f32 是否逐位相同"。
//
// ## 本节的夹具（一个 SFZ ＋ 一个工程，五条区域）
//
// `SFZ_ALL` 把四项各放在一个**独立音高**上（60 / 62 / 64 / 67×2）⇒ 每个音符只
// 触发一项，差异可归因。另有一条**基线**区域（`SFZ_PLAIN`，无任何四项参数）作为
// 正对照：如果基线也不相等，那不是"四项"的问题，而是整条采样器路径不一致。
//
// ## 判据的读法（⭐ 今天必须读作"不同"）
//
// 本节的**主张**是"迁移后两侧必须字节相等"。今天它**不是**绿的：四项都还没迁。
// 因此：
// * [`the_sampler_export_digests_are_not_yet_equal_and_the_four_features_localize_it`]
//   **钉住今天的事实**（基线相同、四项各自不同、无采样器正对照相同），并逐项打印
//   首个 `>1 LSB` 的样本位置 —— 它就是"这一项是不是差异来源"的读数；
// * [`the_sampler_export_must_be_byte_identical_after_the_sfz_migration`] 是**严格
//   验收判据**（`assert_eq!`），今天 `#[ignore]`：把它拉起来（`--ignored`）就会红，
//   红的每一条都点名一项未迁移的参数。迁移完成后删掉 `#[ignore]` 即为验收。
//
// ⚠ `#[ignore]` **不是**放宽判据：正常档里它一条断言都不执行；它的值在
// `--ignored` 档与报告里（`#272` 的交付就是那组红的读数）。
//
// ## 它**不**证什么
//
// ⛔ 没人耳听；⛔ 不证用户机器上的真 SFZ 库（夹具写在 `std::env::temp_dir()` 下，
// `Drop` 删除）；⛔ 不证迁移**之后**的字节（那要等生产代码真的把四项接进 MCP）。

/// 采样器的样本：**48 000 帧 Float32 单声道**（1 秒 @ 48 kHz）。
///
/// 为什么是 Float32：两条读取腿（`yeban-mcp::domain::sfz::read_sample_file` 与
/// `yeban-app::sfz_load::decode_sample_classified`）在 WAV 上都走
/// `yeban_render::wav::read_plain_wav_mono_f32`，而 Float32 的归一化因子是 `1.0`
/// ⇒ **不把"解码归一化"这个自变量混进来**（整数素材那条口径已由夹具 A2 钉住）。
///
/// 为什么不是常量：`offset`/`end`、`loop_*` 只有在样本**有形状**时才会改变输出；
/// 常量样本上"裁掉前 5000 帧"与"从 0 播"逐位相同 ⇒ 判据会假绿。
fn structured_sampler_sample(scratch: &Scratch) -> std::path::PathBuf {
    let frames = 48_000usize;
    let mut samples = Vec::with_capacity(frames);
    for index in 0..frames {
        // 整数算术的阶梯（⛔ 不用 sin/cos）：跨平台逐位一致的确定性素材。
        let step = ((index * 37) % 997) as f32 / 997.0 - 0.5;
        samples.push(step * 0.8);
    }
    let path = scratch.join("tone.wav");
    write_plain_wav(
        &path,
        &PcmFormat::float(1, SAMPLE_RATE, 32),
        &PcmBuffer::Float32(samples),
    )
    .expect("写 Float32 单声道采样器夹具");
    path
}

/// 基线区域：**没有任何** `#272` 四项参数（正对照）。
const SFZ_PLAIN: &str = "<region> sample=tone.wav key=60 pitch_keycenter=60\n";

/// `loop_mode=loop_continuous`（循环窗口 1000..3000 样本帧）。
const SFZ_LOOP: &str = "<region> sample=tone.wav key=60 pitch_keycenter=60 \
                         loop_mode=loop_continuous loop_start=1000 loop_end=3000\n";

/// `offset=5000 end=15000`（引擎按 span 切片，MCP 从 0 播到样本尾）。
const SFZ_SPAN: &str = "<region> sample=tone.wav key=60 pitch_keycenter=60 offset=5000 end=15000\n";

/// `ampeg_attack=0.05 ampeg_release=0.2`（引擎有包络，MCP 一个包络值都不读）。
const SFZ_AMPEG: &str = "<region> sample=tone.wav key=60 pitch_keycenter=60 \
                          ampeg_attack=0.05 ampeg_release=0.2\n";

/// **双层区域**：两条 `region` 命中同一个音符（引擎叠加，MCP 只取第一条）。
const SFZ_LAYERS: &str = "<region> sample=tone.wav key=60 pitch_keycenter=60 volume=-6\n\
                          <region> sample=tone.wav key=60 pitch_keycenter=60 volume=-6\n";

/// **一个 SFZ 覆盖四项**：60 = 循环、62 = 区间、64 = 包络、67 = 双层（两条）。
///
/// 四个音高互不相同 ⇒ 任一音符的差异只能来自它命中的那一项，⛔ 不会串味。
///
/// 逐行原文住在 `crates/yeban-app/tests/fixtures/sampler_c1.sfz`（`include_str!`
/// 编译进判据 ⇒ 判据读的就是仓库里那一份，⛔ 不是运行期再拼一份字符串）。
const SFZ_ALL: &str = include_str!("../../tests/fixtures/sampler_c1.sfz");

/// 一条采样器轨的工程：`Master` ＋ 一条 MIDI 轨（`TrackV3.sampler = sfz`）。
///
/// `notes` 是 `(start_tick, pitch)`，时值统一 960 tick（0.5 秒 @ 120 BPM / 960 PPQ）；
/// 片段时长 = [`END_TICK`]（2 秒）。`sampler = None` 是"**无采样器轨**"正对照。
fn sampler_project(notes: &[(u64, u8)], sampler: Option<&Path>) -> YebanProjectV1 {
    let mut project = empty_project();
    let track = add_track(&mut project, 2, TrackKind::Midi, "Sampler");
    project.tracks.get_mut(&track).expect("夹具轨").sampler = sampler.map(|path| SamplerBinding {
        path: path.display().to_string(),
    });
    let mut map = BTreeMap::new();
    for (index, (start_tick, pitch)) in notes.iter().enumerate() {
        let id = fixed(100 + index as u128);
        map.insert(id, MidiNote::new(id, *start_tick, *pitch, 960));
    }
    place_clip(
        &mut project,
        track,
        10,
        "sampler",
        ClipContent::Midi { notes: map },
    );
    project.validate().expect("采样器夹具工程必须合法");
    project
}

/// 首个"超出抖动底"（`|Δ| > 1` LSB）的样本：下标、帧、声道、两侧整数码与差值。
#[derive(Clone, Copy, Debug)]
struct DiffPoint {
    /// 交错样本下标（声道合计）。
    index: usize,
    /// 帧号（`index / 2`）。
    frame: u64,
    /// 声道（0 = 左，1 = 右）。
    channel: usize,
    /// MCP 落盘的 24-bit 整数码。
    mcp_code: i32,
    /// CLI 的 f32 母带按同一 `2^23` 尺度取整后的整数码（⛔ 不加抖动）。
    cli_code: i32,
    /// `mcp_code - cli_code`（LSB）。
    delta_lsb: i64,
}

/// 找首个 `|Δ| > 1 LSB` 的样本（`None` = 两侧只差 TPDF 抖动底）。
fn first_sample_diff(path: &Path, cli_samples: &[f32]) -> Option<DiffPoint> {
    for (index, (&mcp_code, &cli)) in mcp_pcm_codes(path).iter().zip(cli_samples).enumerate() {
        let cli_code = quantize_like_export(cli);
        let delta_lsb = i64::from(mcp_code) - i64::from(cli_code);
        if delta_lsb.abs() > 1 {
            return Some(DiffPoint {
                index,
                frame: (index / 2) as u64,
                channel: index % 2,
                mcp_code,
                cli_code,
                delta_lsb,
            });
        }
    }
    None
}

/// 一次采样器 C1 读数的全部字段（每一条都打印，⛔ 不让人猜）。
struct SamplerByteReading {
    tag: String,
    mcp_digest: String,
    cli_digest: String,
    mcp_file_sha256: String,
    cli_file_sha256: String,
    mcp_payload_sha256: String,
    cli_payload_sha256: String,
    mcp_peak: f32,
    cli_peak: f32,
    diff: SampleDiff,
    first: Option<DiffPoint>,
    label: String,
    /// CLI 腿的 `source_label`（`render_core` 的 `sfz(...)` 行就在里面）。
    cli_label: String,
    unsupported: Vec<String>,
    sampler_regions: u64,
    sampler_notes: u64,
    notes_looping: u64,
    notes_using_file_sample: u64,
    regions_with_envelope: u64,
}

impl SamplerByteReading {
    fn same(&self) -> bool {
        self.mcp_digest == self.cli_digest
    }

    fn print(&self) {
        println!(
            "### [{}] masterDigest: MCP={} CLI={} 相同={}（十六进制差 {}/64）",
            self.tag,
            self.mcp_digest,
            self.cli_digest,
            self.same(),
            digest_hex_distance(&self.mcp_digest, &self.cli_digest),
        );
        println!(
            "### [{}] 导出文件 sha256: MCP={} CLI={} 相同={}（⚠ 容器/抖动不同 ⇒ 本条不可比）",
            self.tag,
            self.mcp_file_sha256,
            self.cli_file_sha256,
            self.mcp_file_sha256 == self.cli_file_sha256,
        );
        println!(
            "### [{}] PCM 负载 sha256（同一抖动种子）: MCP={} CLI={} 相同={}",
            self.tag,
            self.mcp_payload_sha256,
            self.cli_payload_sha256,
            self.mcp_payload_sha256 == self.cli_payload_sha256,
        );
        println!(
            "### [{}] 峰值: MCP.before={:.9} CLI.f32={:.9} 位型相同={}",
            self.tag,
            self.mcp_peak,
            self.cli_peak,
            self.mcp_peak.to_bits() == self.cli_peak.to_bits(),
        );
        println!(
            "### [{}] 样本差: 比较={} >1LSB={} max={} LSB RMS={:.3} 首个>1LSB={:?}",
            self.tag,
            self.diff.compared,
            self.diff.over_one_lsb,
            self.diff.max_abs_lsb,
            self.diff.rms_lsb,
            self.diff.first_over_one_lsb,
        );
        match self.first {
            Some(point) => println!(
                "### [{}] 首个不同样本: 下标={} 帧={} 声道={} MCP码={} CLI码={} 差={} LSB",
                self.tag,
                point.index,
                point.frame,
                point.channel,
                point.mcp_code,
                point.cli_code,
                point.delta_lsb,
            ),
            None => println!("### [{}] 首个不同样本: 无（只剩抖动底）", self.tag),
        }
        println!(
            "### [{}] per-track: label={}（regions={} notes={} looping={} fileSample={} envelope={}）",
            self.tag,
            self.label,
            self.sampler_regions,
            self.sampler_notes,
            self.notes_looping,
            self.notes_using_file_sample,
            self.regions_with_envelope,
        );
        println!("### [{}] CLI label: {}", self.tag, self.cli_label);
        println!("### [{}] unsupported: {:?}", self.tag, self.unsupported);
    }
}

/// 跑一条采样器 C1 读数：MCP 导出 ＋ CLI 导出 ＋ 三种"sha256"并排。
fn measure_sampler(tag: &str, project: &YebanProjectV1, scratch: &Scratch) -> SamplerByteReading {
    let response = mcp_render(project, &[], scratch, tag, FRAMES, false, SAMPLE_RATE);
    let cli = cli_render(project, &[], FRAMES, SAMPLE_RATE);
    let mcp_path = scratch.join(&format!("{tag}.master.wav"));
    let diff = sample_diff(&mcp_path, &cli.samples);
    let first = first_sample_diff(&mcp_path, &cli.samples);
    // MCP 的 `data` 负载（24-bit 整数码的字节）。
    let mcp_payload_sha256 = AssetHash::of_bytes(&mcp_pcm_payload(&mcp_path))
        .as_str()
        .to_owned();
    // CLI 的 f32 母带按 **MCP 侧同一个抖动种子**量化后的负载：
    // 若两侧 f32 逐位相同，这一步应当重建出与 MCP 逐字节相同的 `data` 负载。
    let mut rng = yeban_render::rng::dither_rng_for(project.rng_seed, project.master_bus_track_id);
    let cli_pcm = quantize(&cli.samples, BitDepth::Int24, &mut rng);
    let cli_payload_sha256 = AssetHash::of_bytes(&cli_pcm.to_le_bytes())
        .as_str()
        .to_owned();
    // CLI 自己的导出文件（plain WAV；与 MCP 的 BWF 容器**不是**同一种字节）。
    let cli_path = scratch.join(&format!("{tag}.cli.wav"));
    let _ = write_interleaved_wav(
        &cli_path,
        &cli.samples,
        2,
        SAMPLE_RATE,
        project.rng_seed,
        BitDepth::Int24,
    )
    .expect("CLI 导出腿必须能落盘");
    let cli_file_sha256 = AssetHash::of_bytes(&std::fs::read(&cli_path).expect("读 CLI 导出文件"))
        .as_str()
        .to_owned();
    let data = &response["data"];
    let sources = data["sources"].as_array().expect("sources");
    let sampler = sources
        .iter()
        .find(|source| !source["sampler"].is_null())
        .expect("夹具必须有一条采样器源的读数")["sampler"]
        .clone();
    let unsupported: Vec<String> = data["unsupported"]
        .as_array()
        .expect("unsupported")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    let reading = SamplerByteReading {
        tag: tag.to_owned(),
        mcp_digest: data["masterDigest"]
            .as_str()
            .expect("data.masterDigest")
            .to_owned(),
        cli_digest: cli.digest.clone(),
        mcp_file_sha256: data["sha256"].as_str().expect("data.sha256").to_owned(),
        cli_file_sha256,
        mcp_payload_sha256,
        cli_payload_sha256,
        mcp_peak: data["peak"]["before"].as_f64().expect("data.peak.before") as f32,
        cli_peak: cli
            .samples
            .iter()
            .fold(0.0_f32, |acc, sample| acc.max(sample.abs())),
        diff,
        first,
        label: sampler["label"].as_str().expect("label").to_owned(),
        cli_label: cli.source_label.clone(),
        unsupported,
        sampler_regions: sampler["regions"].as_u64().expect("regions"),
        sampler_notes: sampler["notes"].as_u64().expect("notes"),
        notes_looping: sampler["notesLooping"].as_u64().expect("notesLooping"),
        notes_using_file_sample: sampler["notesUsingFileSample"]
            .as_u64()
            .expect("notesUsingFileSample"),
        regions_with_envelope: sampler["regionsWithEnvelope"]
            .as_u64()
            .expect("regionsWithEnvelope"),
    };
    reading.print();
    reading
}

/// ⭐ `#272` 判据 1–5：**同一工程、同一 SFZ ⇒ 两侧的字节摘要并排读数**。
///
/// 今天的事实（本判据钉住的）：
/// * **基线**（无四项参数）：两侧 `masterDigest` **相同** ⇒ 判据本身不假红；
/// * **四项**各自：两侧 `masterDigest` **不同**，且每项都给出首个 `>1 LSB` 的样本；
/// * **无采样器轨**：两侧 `masterDigest` **相同** ⇒ 差异确实来自采样器路径；
/// * 五条区域的 `SFZ_ALL`：per-track 读数点名 `regions=5`，`unsupported` 点名
///   `sfzLoopExpansion` / `sfzSampleSpan` / `sfzRegionEnvelope` / `sfzRegionLayering`；
///   ⭐ `#273` 之后双层区域**有**对应的 `unsupported` 键（修前是**未登记**的分叉）。
///   单层区域⛔ 不登记该键（触发器不是恒真）。
#[test]
fn the_sampler_export_digests_are_not_yet_equal_and_the_four_features_localize_it() {
    let scratch = Scratch::new("sampler-c1");
    structured_sampler_sample(&scratch);

    // ---- 正对照 1：基线（无四项参数）⇒ 必须相同 ----
    let plain_sfz = scratch.write_sfz("plain.sfz", SFZ_PLAIN);
    let plain = measure_sampler(
        "baseline",
        &sampler_project(&[(0, 60)], Some(&plain_sfz)),
        &scratch,
    );
    assert!(
        plain.same(),
        "⭐ 基线区域（无 loop/offset/ampeg/双层）⇒ 两侧 masterDigest **必须相同**；\
         若不同 ⇒ 采样器路径整体不一致，⛔ 不是四项的问题。MCP={} CLI={}",
        plain.mcp_digest,
        plain.cli_digest
    );

    // ---- 正对照 2：无采样器轨 ⇒ 必须相同 ----
    let no_sampler =
        measure_sampler_no_sampler("no-sampler", &sampler_project(&[(0, 60)], None), &scratch);
    assert!(
        no_sampler.same(),
        "⭐ 无采样器轨（两侧都走引擎音源）⇒ masterDigest **必须相同**（判据不假红）：\
         MCP={} CLI={}",
        no_sampler.mcp_digest,
        no_sampler.cli_digest
    );
    assert_eq!(
        no_sampler.mcp_payload_sha256, no_sampler.cli_payload_sha256,
        "无采样器轨 ⇒ 同一抖动种子下 PCM 负载也必须逐字节相同"
    );

    // ---- 四项逐项：每项都必须真的是差异来源 ----
    let loop_sfz = scratch.write_sfz("loop.sfz", SFZ_LOOP);
    let span_sfz = scratch.write_sfz("span.sfz", SFZ_SPAN);
    let ampeg_sfz = scratch.write_sfz("ampeg.sfz", SFZ_AMPEG);
    let layers_sfz = scratch.write_sfz("layers.sfz", SFZ_LAYERS);
    let features: [(&str, &Path); 4] = [
        ("loop", &loop_sfz),
        ("span", &span_sfz),
        ("ampeg", &ampeg_sfz),
        ("layers", &layers_sfz),
    ];
    for (name, sfz_path) in features {
        let reading = measure_sampler(
            &format!("only-{name}"),
            &sampler_project(&[(0, 60)], Some(sfz_path)),
            &scratch,
        );
        assert!(
            !reading.same(),
            "⭐ `{name}` 今天必须是差异来源（MCP 不应用它、引擎应用它）；\
             MCP={} CLI={} —— 若这条红，说明它已被迁移，请把本断言改成相等并挪进严格判据",
            reading.mcp_digest,
            reading.cli_digest
        );
        assert!(
            reading.first.is_some(),
            "`{name}`: 摘要不同 ⇒ 必须存在首个 >1 LSB 的样本（实际没有 ⇒ 差异只在抖动底，\
             报我）"
        );
        // 夹具自证：CLI 的 `sfz(...)` 行必须包含与 MCP **逐字相同**的那一段。
        assert!(
            reading.cli_label.contains(&reading.label),
            "`{name}`: CLI 的 source_label 必须包含与 MCP 逐字相同的 sfz(...) 行: {} vs {}",
            reading.cli_label,
            reading.label
        );
        if name == "layers" {
            assert_eq!(reading.sampler_regions, 2, "双层夹具必须装出 2 条区域");
            assert_eq!(reading.sampler_notes, 1, "双层夹具只有一个音符");
            // ⭐ `#273`：两条区域命中同一音符 ⇒ 必须登记这条分叉。
            assert!(
                reading
                    .unsupported
                    .iter()
                    .any(|entry| entry == "sfzRegionLayering"),
                "双层夹具必须登记 `sfzRegionLayering`: {:?}",
                reading.unsupported
            );
        } else {
            // 单层夹具（loop / span / ampeg）⛔ 不得登记：触发器不是恒真。
            assert!(
                !reading
                    .unsupported
                    .iter()
                    .any(|entry| entry == "sfzRegionLayering"),
                "单层夹具 `{name}` 不得登记 `sfzRegionLayering`: {:?}",
                reading.unsupported
            );
        }
    }

    // ---- 五条区域一起上：夹具自证 ----
    let all_sfz = scratch.write_sfz("all.sfz", SFZ_ALL);
    let all = measure_sampler(
        "all-four",
        &sampler_project(
            &[(0, 60), (960, 62), (1_920, 64), (2_880, 67)],
            Some(&all_sfz),
        ),
        &scratch,
    );
    println!("### SFZ_ALL（逐字）:\n{SFZ_ALL}");
    assert!(
        all.label
            .starts_with("sfz(tracks=1 regions=5 samples=1 silent=0"),
        "夹具必须真的让 5 条区域被装载（{SFZ_ALL}）: {}",
        all.label
    );
    assert_eq!(all.sampler_regions, 5, "区域数必须是 5");
    assert_eq!(all.sampler_notes, 4, "四个音符必须都被排程");
    assert_eq!(all.notes_looping, 1, "恰好一个音符落在循环区域上");
    assert_eq!(
        all.notes_using_file_sample, 4,
        "四个音符都必须落在文件样本上"
    );
    assert_eq!(
        all.regions_with_envelope, 1,
        "恰好一个音符落在 ampeg 区域上"
    );
    for key in ["sfzLoopExpansion", "sfzSampleSpan", "sfzRegionEnvelope"] {
        assert!(
            all.unsupported.iter().any(|entry| entry == key),
            "SFZ_ALL 必须让 MCP 登记 `{key}`: {:?}",
            all.unsupported
        );
    }
    // ⭐ `#273`：双层区域（key=67 有两条 region）必须**点名**登记；修前这里断言"没有"。
    assert!(
        all.unsupported
            .iter()
            .any(|entry| entry == "sfzRegionLayering"),
        "双层区域必须登记 `sfzRegionLayering`（⛔ 不许心知肚明地不报）: {:?}",
        all.unsupported
    );
    // 触发器⛔ 不是恒真：单层夹具（基线 / loop / span / ampeg）一条都不许出现。
    for reading in [&plain, &no_sampler] {
        assert!(
            !reading
                .unsupported
                .iter()
                .any(|entry| entry == "sfzRegionLayering"),
            "单层/无采样器夹具 `{}` 不得登记 `sfzRegionLayering`: {:?}",
            reading.tag,
            reading.unsupported
        );
    }
    assert!(
        all.cli_label.contains(&all.label),
        "CLI 的 source_label 必须包含与 MCP 逐字相同的 sfz(...) 行: {} vs {}",
        all.cli_label,
        all.label
    );
    assert!(
        all.cli_label
            .contains("sfz(tracks=1 regions=5 samples=1 silent=0"),
        "CLI 腿也必须装出 5 条区域（⛔ 不是只有 MCP 一份）: {}",
        all.cli_label
    );
    assert!(
        !all.same(),
        "⭐ 五项一起（四项未迁移）⇒ 两侧 masterDigest 必须不同；若相同 ⇒ 夹具没触发四项"
    );
    let point = all.first.expect("五项一起时必须有首个不同样本");
    println!(
        "### all-four 首个不同样本落在帧 {}（音符序号 = {}，@ {:.3} s）",
        point.frame,
        point.frame / 24_000,
        point.frame as f64 / f64::from(SAMPLE_RATE)
    );

    // ---- 判据 3 的正对照：既有的"峰值相等"判据在**基线**上仍绿 ----
    //
    // ⚠ 四项一起上时峰值**不一定**相等（双层区域：MCP 只播第一条 ⇒ 0.124294676，
    // 引擎叠加两条 ⇒ 0.248589352）。本单**不删**既有判据；这里的正对照只证明
    // "无四项参数时峰值相等"这条今天仍为真，⛔ 不拿它去要求四项也必须峰值相等。
    assert_eq!(
        plain.mcp_peak.to_bits(),
        plain.cli_peak.to_bits(),
        "既有判据（峰值相等）在基线上今天为真：MCP={} CLI={} —— 本单 ⛔ 不删它",
        plain.mcp_peak,
        plain.cli_peak
    );
    println!(
        "### 峰值判据的盲区（本单的新读数）：loop/span/ampeg 三项**峰值相同**\
         （MCP=CLI={:.9}）而 masterDigest 不同；只有 layers 的峰值也不同。\
         ⇒ 「峰值相等」对 3/4 项是盲的。",
        plain.cli_peak
    );
}

/// 无采样器轨时的读数：`measure_sampler` 要求有 per-track 采样器读数，这条腿没有。
fn measure_sampler_no_sampler(
    tag: &str,
    project: &YebanProjectV1,
    scratch: &Scratch,
) -> SamplerByteReading {
    let response = mcp_render(project, &[], scratch, tag, FRAMES, false, SAMPLE_RATE);
    let cli = cli_render(project, &[], FRAMES, SAMPLE_RATE);
    let mcp_path = scratch.join(&format!("{tag}.master.wav"));
    let diff = sample_diff(&mcp_path, &cli.samples);
    let first = first_sample_diff(&mcp_path, &cli.samples);
    let mcp_payload_sha256 = AssetHash::of_bytes(&mcp_pcm_payload(&mcp_path))
        .as_str()
        .to_owned();
    let mut rng = yeban_render::rng::dither_rng_for(project.rng_seed, project.master_bus_track_id);
    let cli_pcm = quantize(&cli.samples, BitDepth::Int24, &mut rng);
    let cli_payload_sha256 = AssetHash::of_bytes(&cli_pcm.to_le_bytes())
        .as_str()
        .to_owned();
    let cli_path = scratch.join(&format!("{tag}.cli.wav"));
    let _ = write_interleaved_wav(
        &cli_path,
        &cli.samples,
        2,
        SAMPLE_RATE,
        project.rng_seed,
        BitDepth::Int24,
    )
    .expect("CLI 导出腿必须能落盘");
    let cli_file_sha256 = AssetHash::of_bytes(&std::fs::read(&cli_path).expect("读 CLI 导出文件"))
        .as_str()
        .to_owned();
    let data = &response["data"];
    assert!(
        data["sources"]
            .as_array()
            .expect("sources")
            .iter()
            .all(|source| source["sampler"].is_null()),
        "正对照工程不得有任何采样器源: {response}"
    );
    let unsupported: Vec<String> = data["unsupported"]
        .as_array()
        .expect("unsupported")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    let reading = SamplerByteReading {
        tag: tag.to_owned(),
        mcp_digest: data["masterDigest"]
            .as_str()
            .expect("data.masterDigest")
            .to_owned(),
        cli_digest: cli.digest.clone(),
        mcp_file_sha256: data["sha256"].as_str().expect("data.sha256").to_owned(),
        cli_file_sha256,
        mcp_payload_sha256,
        cli_payload_sha256,
        mcp_peak: data["peak"]["before"].as_f64().expect("data.peak.before") as f32,
        cli_peak: cli
            .samples
            .iter()
            .fold(0.0_f32, |acc, sample| acc.max(sample.abs())),
        diff,
        first,
        label: "(无采样器轨)".to_owned(),
        cli_label: cli.source_label.clone(),
        unsupported,
        sampler_regions: 0,
        sampler_notes: 0,
        notes_looping: 0,
        notes_using_file_sample: 0,
        regions_with_envelope: 0,
    };
    reading.print();
    reading
}

/// ⭐ `#272` **严格验收判据**（迁移引擎 SFZ 之后必须绿）。
///
/// 今天 `#[ignore]`：正常档不执行，`--ignored` 档会红在这四项上 —— 那组红的读数
/// 就是本单的交付。迁移完成后：删掉 `#[ignore]`、删掉
/// [`the_sampler_export_digests_are_not_yet_equal_and_the_four_features_localize_it`]
/// 里的 `!reading.same()` 断言，本节即为验收基线。
#[ignore = "#272 S0 验收基线：四项未迁引擎 SFZ ⇒ 今天必红；迁移后删掉本标记"]
#[test]
fn the_sampler_export_must_be_byte_identical_after_the_sfz_migration() {
    let scratch = Scratch::new("sampler-c1-strict");
    structured_sampler_sample(&scratch);

    let cases: [(&str, &str, Vec<(u64, u8)>); 5] = [
        ("baseline", SFZ_PLAIN, vec![(0, 60)]),
        ("loop", SFZ_LOOP, vec![(0, 60)]),
        ("span", SFZ_SPAN, vec![(0, 60)]),
        ("ampeg", SFZ_AMPEG, vec![(0, 60)]),
        ("layers", SFZ_LAYERS, vec![(0, 60)]),
    ];
    let mut readings: Vec<(String, SamplerByteReading)> = Vec::new();
    for (name, sfz_text, notes) in cases {
        let sfz = scratch.write_sfz(&format!("strict-{name}.sfz"), sfz_text);
        let reading = measure_sampler(
            &format!("strict-{name}"),
            &sampler_project(&notes, Some(&sfz)),
            &scratch,
        );
        readings.push((name.to_owned(), reading));
    }
    // 先全部渲染完再断言：失败时四条读数（含首个不同样本）都已在日志里。
    for (name, reading) in readings {
        assert_eq!(
            reading.mcp_digest,
            reading.cli_digest,
            "⭐ 严格验收（`{name}`）：同一工程、同一 SFZ ⇒ MCP 与 CLI 的母带字节必须相同。\
             MCP={} CLI={}；首个不同样本={:?}；峰值 MCP={} CLI={}。\
             ⚠ 这条红就是在说 `{name}` 还没迁进 MCP —— ⛔ 不要改夹具去回避它。",
            reading.mcp_digest,
            reading.cli_digest,
            reading.first,
            reading.mcp_peak,
            reading.cli_peak
        );
    }
}
