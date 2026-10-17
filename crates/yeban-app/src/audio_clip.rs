//! 音频片段音源：把工程里**已摆放**的 `ClipContent::Audio` 从归档资产池搬进离线母带。
//!
//! ## 为什么需要它
//!
//! `EngineSnapshot::from_project` 的文档说得很清楚：`ClipContent::Audio` **不产生声音**
//! （`crates/yeban-engine/src/snapshot.rs` 的 `project_schedules` 第 5 条）⇒ 走引擎音源的
//! `--export-wav` 对音频摆放**全程静音**。MCP 的渲染器（`crates/yeban-mcp/src/domain/render.rs`
//! 的 `AudioClipSource`）**已经**真的渲染它们；本模块把**同一套语义**接到 app 的离线导出上。
//!
//! ## 语义（逐条对齐 MCP 的 `render_clip_math` / `AudioClipSource`，⛔ 不另立规则）
//!
//! | 维度 | 口径 |
//! | :--- | :--- |
//! | 帧区间 | `[ticks_to_frames(start), ticks_to_frames(start + max(duration, 1)))`，四舍五入、终点至少 `start + 1`。这里**复用引擎的** [`samples_per_tick`] / [`tick_to_sample`]（与 `EngineSnapshot` 的排程同一个函数），而不另写一份换算。 |
//! | 硬切 | 摆放时值之外一律不加；素材比区间长就截断，比区间短就留静音尾巴。 |
//! | 叠加 | **累加**（调用方已清零）；同一素材的多个摆放按 `(start, end, gain 位型)` 全序依次累加（[ARCH-DET-002]，浮点加法不满足结合律）。 |
//! | 增益 | `db_to_linear(clip.gain_db + track.volume_db)`（一次超越函数；非有限按 0 dB），`mute`/`solo` 门控与 MCP 的 `combined_gain_db` 同式。 |
//! | 声相 | `pan == 0` ⇒ `(gain, gain)`；否则 `yeban_engine::mixer::pan_gains`（与 MCP 同一条等功率曲线）。 |
//! | 声道 | `Identity` / `MonoToAll` / `StereoToMono`；其余组合**拒绝**（⛔ 不悄悄丢声道）。 |
//!
//! ## 三条**如实登记**的 app 侧差异（⛔ 不假装与 MCP 完全一致）
//!
//! 1. **读字节要过临时文件**：`yeban-render` 只公开 `read_plain_wav(path)`（没有内存读取器），
//!    而资产字节在 `ProjectArchive.assets` 里。⇒ 本模块把字节写进 `std::env::temp_dir()` 的
//!    唯一临时名、用**本仓唯一的 WAV 读取器**解码、随即删除（`TempFile` 的 `Drop`）。
//!    ⛔ 不新增 `yeban-render` 的函数、⛔ 不引入新依赖边、⛔ 不另写第二份 WAV 解析器。
//! 2. **离线导出这条支路也重采样**（⭐ 任务单 `#141` 闭合了 `#138` 留下的另一半）：
//!    解码后的素材率 ≠ 导出率时，`decode_checked` 调用**实时注入侧同一个**
//!    [`crate::audio_inject::resample_to_target`]（＝ `yeban_decode::resample_interleaved`，
//!    rubato sinc，[ARCH-DSP-002] / D26 的唯一实现），把样本转到导出率再交给渲染。
//!    ⛔ 没有第二份重采样器、⛔ 没有第二份"钉帧数"逻辑。
//!    ⛔ 绝不"改个采样率标签"（那是改时长）或"丢帧"（那是改音高）—— 两者都是静默的错误音频。
//!    ⚠ **仍不能处理的形态照旧响亮跳过**：声道无法映射（例如 3 声道 → 立体声母线）、
//!    坏 WAV、重采样失败 ⇒ 逐字原因进 `audio_clips_skipped`，且导出报告里
//!    `audio-clips(rendered=… skipped=…)` 的计数**明说少了几段**（⛔ 不静默少一段）。
//! 3. **多声道归一化**：共享的 `read_plain_wav_mono_f32` 会把多声道折成**第 0 声道**，
//!    而 MCP 支持 `Identity`（立体声素材进立体声母线）。⇒ 本模块用 `read_plain_wav` 取
//!    **保留声道数**的交织样本，并用**同一张归一化表**（`BitDepth::full_scale_max`；
//!    浮点直用）折成 `f32`。这张表就是 `read_plain_wav_mono_f32` 文档里那张。
//!
//! ## 本模块只服务**离线导出**（实时那半在 `crate::audio_inject`）
//!
//! 本模块只在离线导出（`render_core` → `RenderPlan::execute` 的源）里被构造。
//! 实时快照的音频 PCM 注入与采样率转换住 `crate::audio_inject`（`#136`／`#138`），
//! ⛔ 本模块不参与实时回调。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_engine::mixer::{PanLaw, pan_gains};
use yeban_engine::synth::{samples_per_tick, tick_to_sample};
use yeban_model::{AssetHash, ClipContent, EntityId, YebanProjectV1};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::render::{AudioSource, BlockContext, RenderError, db_to_linear};
use yeban_render::wav::read_plain_wav;

/// 一份已解码的素材：交织 `f32` ＋ 声道数 ＋ 采样率 ＋ 帧数。
///
/// ⚠ 两个来源：`decode_asset_bytes`（实时注入侧）保留**素材自身**的采样率；
/// `decode_checked`（离线导出侧）在两率不同时已把样本换成**导出率**（`#141`）。
#[derive(Debug)]
pub(crate) struct DecodedAsset {
    /// 交织 `f32`（归一化到 `[-1, 1]`）。
    samples: Vec<f32>,
    /// 素材原始声道数（决定声道布局映射）。
    source_channels: u16,
    /// 本份样本的采样率（Hz）—— 实时注入侧按它决定是否重采样（`#138`）；
    /// 离线导出侧由 `decode_checked` 换算到导出率（`#141`）。
    sample_rate: u32,
    /// 帧数（= 样本数 / 声道数）。
    frames: u64,
}

impl DecodedAsset {
    /// 拆成交织样本 ＋ 声道数 ＋ 采样率（注入侧要 `Vec<f32>` 进 `Arc<[f32]>`）。
    ///
    /// `frames` 不进返回值：它是 `samples.len() / channels`，而引擎的
    /// `AudioPcm::new` 会**现算**一次（同一份事实，⛔ 不搬第二份）。
    #[must_use]
    pub(crate) fn into_parts(self) -> (Vec<f32>, u16, u32) {
        (self.samples, self.source_channels, self.sample_rate)
    }

    /// 帧数（导入侧据此把区域帧长换成 tick）。
    #[cfg(feature = "experimental-logic-import")]
    #[must_use]
    pub(crate) const fn frames(&self) -> u64 {
        self.frames
    }

    /// 素材自身的采样率（Hz）。
    #[cfg(feature = "experimental-logic-import")]
    #[must_use]
    pub(crate) const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// 一个资产的解析结果（`Skipped` 的原因会进导出报告）。
#[derive(Clone, Debug)]
pub(crate) enum AssetResolution {
    /// 可用（多个摆放共享同一份 `Arc`，只解码一次）。
    Ready(Arc<DecodedAsset>),
    /// **如实跳过**（原因进 `audio_clips_skipped`）。
    Skipped(String),
}

/// 素材声道数 → 母线声道数的映射方式（与 MCP 的 `ChannelLayout` 逐条同义）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChannelLayout {
    /// 声道数相同：逐声道一一对应。
    Identity,
    /// 单声道素材：复制到每一个母线声道。
    MonoToAll,
    /// 双声道素材进单声道母线：`(L + R) / 2`。
    StereoToMono,
}

/// 一个摆放的**帧区间**与左右增益。
#[derive(Clone, Copy, Debug)]
struct ClipSpan {
    /// 起始帧（含）。
    start: i64,
    /// 结束帧（不含）。
    end: i64,
    /// 左声道增益。
    gain_l: f32,
    /// 右声道增益。
    gain_r: f32,
}

/// 音频片段音源：把一份**已经解码好**的 PCM 按摆放的帧区间搬进母带。
///
/// `pub(crate)`：只有 `render_core` 需要把它与 MIDI 音源叠成一条轨的复合源。
pub(crate) struct AudioClipSource {
    /// 共享的已解码素材。
    asset: Arc<DecodedAsset>,
    /// 声道布局。
    layout: ChannelLayout,
    /// 摆放区间，**已按 `(start, end, gain 位型)` 全序排好**。
    spans: Vec<ClipSpan>,
}

impl AudioClipSource {
    /// 本块内某一帧在**素材**里的下标（超出素材则 `None`）。
    fn asset_frame(&self, span: &ClipSpan, global: i64) -> Option<usize> {
        let offset = global - span.start;
        if offset < 0 || u64::try_from(offset).ok()? >= self.asset.frames {
            return None;
        }
        usize::try_from(offset).ok()
    }

    /// 按 `[ARCH-DET-002]` 的固定全序排好区间。
    fn sort_spans(&mut self) {
        self.spans.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then(a.end.cmp(&b.end))
                .then(a.gain_l.to_bits().cmp(&b.gain_l.to_bits()))
                .then(a.gain_r.to_bits().cmp(&b.gain_r.to_bits()))
        });
    }
}

/// 某一输出声道的增益（第 0 声道左、第 1 声道右，其余不参与）。
fn channel_gain(span: &ClipSpan, channel: usize) -> f32 {
    match channel {
        0 => span.gain_l,
        1 => span.gain_r,
        _ => 0.0,
    }
}

impl AudioSource for AudioClipSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        // 调用方（`TrackSource` 的 MIDI 基座）已经写过/清过 `out`；这里只**累加**。
        let channels = context.channels;
        if channels == 0 || self.spans.is_empty() || self.asset.frames == 0 {
            return Ok(());
        }
        let source_channels = usize::from(self.asset.source_channels);
        let block_start = i64::try_from(context.first_frame).unwrap_or(i64::MAX);
        let block_end =
            block_start.saturating_add(i64::try_from(context.frames).unwrap_or(i64::MAX));
        for span in &self.spans {
            if span.start >= block_end {
                break;
            }
            if span.end <= block_start {
                continue;
            }
            let from = span.start.max(block_start);
            let to = span.end.min(block_end);
            for global in from..to {
                let Some(asset_frame) = self.asset_frame(span, global) else {
                    continue;
                };
                let source_base = asset_frame * source_channels;
                let index = usize::try_from(global - block_start).unwrap_or(0);
                let out_base = index * channels;
                match self.layout {
                    ChannelLayout::Identity => {
                        for channel in 0..channels {
                            let gain = channel_gain(span, channel);
                            out[out_base + channel] +=
                                self.asset.samples[source_base + channel] * gain;
                        }
                    }
                    ChannelLayout::MonoToAll => {
                        let value = self.asset.samples[source_base];
                        for channel in 0..channels {
                            out[out_base + channel] += value * channel_gain(span, channel);
                        }
                    }
                    ChannelLayout::StereoToMono => {
                        let mixed = (self.asset.samples[source_base]
                            + self.asset.samples[source_base + 1])
                            * 0.5;
                        out[out_base] += mixed * span.gain_l;
                    }
                }
            }
        }
        Ok(())
    }
}

/// 一条轨道解析出的音频片段音源 ＋ 被搬进母带的**摆放个数**。
pub(crate) struct TrackClips {
    /// 每份素材一个源（同一素材的多个摆放共用一个源）。
    pub(crate) sources: Vec<AudioClipSource>,
    /// 真的进了母带的摆放**个数**（判据与报告都读它）。
    pub(crate) spans: u64,
}

/// 判定声道布局；不支持的组合返回 `None`（调用方据此**跳过并报告**）。
fn channel_layout(asset_channels: u16, out_channels: usize) -> Option<ChannelLayout> {
    if asset_channels == 0 || out_channels == 0 {
        return None;
    }
    if usize::from(asset_channels) == out_channels {
        return Some(ChannelLayout::Identity);
    }
    if asset_channels == 1 {
        return Some(ChannelLayout::MonoToAll);
    }
    if asset_channels == 2 && out_channels == 1 {
        return Some(ChannelLayout::StereoToMono);
    }
    None
}

/// 摆放的帧区间 `[start, end)`：**复用引擎的**每 tick 帧数（与快照排程同一个函数）。
fn clip_frame_span(start_tick: u64, duration_ticks: u64, bpm: f64, sample_rate: u32) -> (i64, i64) {
    let per_tick = samples_per_tick(bpm, sample_rate);
    let start = tick_to_sample(start_tick, per_tick).unwrap_or(0);
    let end_tick = start_tick.saturating_add(duration_ticks.max(1));
    let end = tick_to_sample(end_tick, per_tick).unwrap_or(0);
    let start = i64::try_from(start).unwrap_or(i64::MAX);
    let end = i64::try_from(end).unwrap_or(i64::MAX);
    (start, end.max(start.saturating_add(1)))
}

/// 解析一条轨道上全部**可渲染**的音频摆放。
///
/// `cache` 按资产哈希缓存解码结果（同一素材只解码一次）；`skipped` 收集**如实跳过**的原因。
pub(crate) fn resolve_track_clips(
    project: &YebanProjectV1,
    track_id: EntityId,
    sample_rate: u32,
    out_channels: usize,
    assets: &[(AssetHash, Vec<u8>)],
    cache: &mut BTreeMap<AssetHash, AssetResolution>,
    skipped: &mut Vec<String>,
) -> TrackClips {
    let empty = TrackClips {
        sources: Vec::new(),
        spans: 0,
    };
    let Some(track) = project.tracks.get(&track_id) else {
        return empty;
    };
    // solo 语义与 MCP 的 `build_source` 同式（只作用于源轨）。
    let any_solo = project.tracks.values().any(|candidate| candidate.solo);
    let audible = !track.mute && (!any_solo || track.solo || track.solo_safe);
    let track_volume_db = if track.volume_db.is_finite() {
        track.volume_db
    } else {
        0.0
    };
    // 每个资产 → 它在这条轨道上的摆放区间（BTreeMap 的哈希序 = 确定的叠加顺序）。
    let mut per_asset: BTreeMap<AssetHash, (Arc<DecodedAsset>, Vec<ClipSpan>)> = BTreeMap::new();
    for placement in track.clips.values() {
        if placement.muted {
            continue;
        }
        let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
            skipped.push(format!(
                "摆放 {} 引用了片段池里不存在的片段（已跳过）",
                placement.id.to_canonical_string()
            ));
            continue;
        };
        let ClipContent::Audio { asset, gain_db } = &entry.content else {
            continue;
        };
        if !audible {
            skipped.push(format!(
                "轨 {track_id} 被 mute/solo 门控：音频片段 {} 不发声",
                entry.name
            ));
            continue;
        }
        let clip_gain_db = if gain_db.is_finite() { *gain_db } else { 0.0 };
        let gain = db_to_linear(clip_gain_db + track_volume_db);
        let (gain_l, gain_r) = if track.pan.is_finite() && track.pan != 0.0 {
            let (left, right) =
                pan_gains(track.pan, PanLaw::from_model(project.audio_config.pan_law));
            (gain * left, gain * right)
        } else {
            (gain, gain)
        };
        let resolution = resolve_asset(asset, sample_rate, assets, cache);
        let prepared = match resolution {
            AssetResolution::Ready(prepared) => prepared,
            AssetResolution::Skipped(reason) => {
                skipped.push(format!(
                    "片段 {}（{}）：{reason}",
                    entry.name,
                    asset.as_str()
                ));
                continue;
            }
        };
        if channel_layout(prepared.source_channels, out_channels).is_none() {
            skipped.push(format!(
                "片段 {}（{}）：素材 {} 声道无法映射到 {} 声道母线（已跳过，⛔ 不悄悄丢声道）",
                entry.name,
                asset.as_str(),
                prepared.source_channels,
                out_channels
            ));
            continue;
        }
        let (start, end) = clip_frame_span(
            placement.start_tick,
            placement.duration_ticks,
            project.bpm,
            sample_rate,
        );
        let entry = per_asset
            .entry(asset.clone())
            .or_insert_with(|| (Arc::clone(&prepared), Vec::new()));
        entry.1.push(ClipSpan {
            start,
            end,
            gain_l,
            gain_r,
        });
    }
    let spans_total: u64 = per_asset
        .values()
        .map(|(_, spans)| u64::try_from(spans.len()).unwrap_or(u64::MAX))
        .sum();
    let sources: Vec<AudioClipSource> = per_asset
        .into_iter()
        .map(|(_, (asset, spans))| {
            let layout = channel_layout(asset.source_channels, out_channels)
                .unwrap_or(ChannelLayout::MonoToAll);
            let mut source = AudioClipSource {
                asset,
                layout,
                spans,
            };
            source.sort_spans();
            source
        })
        .collect();
    TrackClips {
        sources,
        spans: spans_total,
    }
}

/// 解析一个资产：先查缓存，再校验哈希，再解码，最后查采样率。
fn resolve_asset(
    hash: &AssetHash,
    target_rate: u32,
    assets: &[(AssetHash, Vec<u8>)],
    cache: &mut BTreeMap<AssetHash, AssetResolution>,
) -> AssetResolution {
    if let Some(cached) = cache.get(hash) {
        return cached.clone();
    }
    let resolution = match assets.iter().find(|(candidate, _)| candidate == hash) {
        None => AssetResolution::Skipped(
            "归档资产池里没有这份字节（工程引用了它，但池里没有）".to_owned(),
        ),
        Some((_, bytes)) => match decode_checked(hash, bytes, target_rate) {
            Ok(asset) => AssetResolution::Ready(Arc::new(asset)),
            Err(reason) => AssetResolution::Skipped(reason),
        },
    };
    cache.insert(hash.clone(), resolution.clone());
    resolution
}

/// 校验哈希 ⇒ 解码 ⇒ 采样率不符则**重采样到导出率**（⭐ `#141`）。
///
/// 重采样复用实时注入侧的 **同一个** [`crate::audio_inject::resample_to_target`]
/// （＝ `yeban_decode::resample_interleaved`，rubato sinc）：输出帧数被钉到
/// `ceil(源帧数 × 导出率 / 素材率)`，因此**时长不变、音高不变**（⛔ 不是"改标签"）。
///
/// 两率相同 ⇒ **原样返回解码结果**（⛔ 不复制、⛔ 不滤波 ⇒ 逐位不变；判据见本文件
/// 单元测试 `same_rate_decode_is_bit_identical_to_the_plain_decoder`）。
///
/// 重采样失败 ⇒ 返回**逐字原因**（调用方 `AssetResolution::Skipped` ⇒ 进导出报告），
/// ⛔ 不静默替换成静音。
fn decode_checked(
    hash: &AssetHash,
    bytes: &[u8],
    target_rate: u32,
) -> Result<DecodedAsset, String> {
    let mut decoded = decode_asset_bytes(hash, bytes)?;
    if decoded.sample_rate == target_rate {
        return Ok(decoded);
    }
    let source_rate = decoded.sample_rate;
    let channels = decoded.source_channels;
    let samples = crate::audio_inject::resample_to_target(
        &decoded.samples,
        channels,
        source_rate,
        target_rate,
    )
    .map_err(|reason| {
        format!(
            "素材采样率 {source_rate} Hz ≠ 导出采样率 {target_rate} Hz，{reason}；\
             已**跳过**而不是按错误音高播放"
        )
    })?;
    decoded.frames = u64::try_from(samples.len() / usize::from(channels.max(1))).unwrap_or(0);
    decoded.samples = samples;
    decoded.sample_rate = target_rate;
    Ok(decoded)
}

/// 校验哈希 ⇒ 解码（**不**比对采样率）。
///
/// 任务单 `#136`／`#138` 的实时注入侧复用它：采样率**换算**由
/// `crate::audio_inject` 在注入前做（`#138`：重采样到快照率）；引擎的
/// `EngineSnapshot::with_audio_asset` 仍是**最后一道**拒绝（采样率不符 ⇒
/// `AudioClipError::SampleRateMismatch`，⛔ 不在 app 侧删它）。这里只负责"字节是不是
/// 它声明的那个哈希"与"WAV 能不能解码"两件事 —— 与离线 `decode_checked` 的**前两步
/// 逐字相同**，⛔ 没有第二份解码器。
pub(crate) fn decode_asset_bytes(hash: &AssetHash, bytes: &[u8]) -> Result<DecodedAsset, String> {
    let actual = AssetHash::of_bytes(bytes);
    if actual != *hash {
        return Err(format!(
            "资产池里的字节与声明的 SHA-256 不符（声明 {hash}，实际 {actual}）"
        ));
    }
    // ⭐ `#152`：容器里可以放**源文件原样字节**（Logic demo 的 `.aif` 就是 AIFF/PCM）。
    // 这不改变"唯一一份归一化表"：解码后仍走下面同一个 `DecodedAsset` 形状。
    if bytes.len() >= 12 && &bytes[..4] == b"FORM" {
        if &bytes[8..12] == b"AIFC" {
            return Err("AIFC（压缩 AIFF）不在本仓支持面内（只支持未压缩 AIFF/PCM）".to_owned());
        }
        if &bytes[8..12] == b"AIFF" {
            return decode_aiff_bytes(bytes);
        }
    }
    decode_wav_bytes(bytes)
}

/// AIFF（`FORM`/`AIFF`，**未压缩 PCM**）解码：16 / 24 / 32-bit 大端整数。
///
/// 为什么 app 侧自己读：`yeban-render` 的 `read_plain_wav` 是 WAV 专用（`#152` 的
/// 容器要放 Logic demo 的 `.aif` 原样字节，而 `yeban-decode` 明确不启用 AIFF）。
/// 只覆盖 AIFF 的 PCM 子集；`AIFC`（压缩变体）**明确拒绝**，⛔ 不猜压缩格式。
///
/// 读法与公开的 AIFF 结构一致：`COMM` 给声道/帧/位深/80-bit 采样率，`SSND` 给
/// 大端交织样本。所有长度都做边界检查 → 畸形文件返回逐字原因（⛔ 不 panic）。
fn decode_aiff_bytes(bytes: &[u8]) -> Result<DecodedAsset, String> {
    let mut channels: Option<u16> = None;
    let mut declared_frames: Option<u64> = None;
    let mut bits: Option<u16> = None;
    let mut sample_rate: Option<u32> = None;
    let mut sound: Option<(usize, usize)> = None;
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_be_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = at + 8;
        let end = body.checked_add(size).ok_or("AIFF 块长度溢出")?;
        if end > bytes.len() {
            return Err(format!(
                "AIFF 块 `{}` 越过文件末尾",
                String::from_utf8_lossy(id)
            ));
        }
        match id {
            b"COMM" => {
                if size < 18 {
                    return Err(format!("AIFF COMM 块只有 {size} 字节（至少 18）"));
                }
                channels = Some(u16::from_be_bytes([bytes[body], bytes[body + 1]]));
                declared_frames = Some(u64::from(u32::from_be_bytes([
                    bytes[body + 2],
                    bytes[body + 3],
                    bytes[body + 4],
                    bytes[body + 5],
                ])));
                bits = Some(u16::from_be_bytes([bytes[body + 6], bytes[body + 7]]));
                sample_rate = Some(read_extended_80(&bytes[body + 8..body + 18])?);
            }
            b"SSND" => {
                if size < 8 {
                    return Err(format!("AIFF SSND 块只有 {size} 字节（至少 8）"));
                }
                let offset = u32::from_be_bytes([
                    bytes[body],
                    bytes[body + 1],
                    bytes[body + 2],
                    bytes[body + 3],
                ]) as usize;
                let start = body
                    .checked_add(8)
                    .and_then(|value| value.checked_add(offset))
                    .ok_or("AIFF SSND 偏移溢出")?;
                if start > end {
                    return Err("AIFF SSND 的 offset 越过块末尾".to_owned());
                }
                sound = Some((start, end));
            }
            _ => {}
        }
        at = end.checked_add(size & 1).ok_or("AIFF 块对齐溢出")?;
    }
    let channels = channels.ok_or_else(|| "AIFF 缺 COMM 块".to_owned())?;
    let declared_frames = declared_frames.ok_or_else(|| "AIFF 缺 COMM 块".to_owned())?;
    let bits = bits.ok_or_else(|| "AIFF 缺 COMM 块".to_owned())?;
    let sample_rate = sample_rate.ok_or_else(|| "AIFF 缺 COMM 块".to_owned())?;
    let (start, end) = sound.ok_or_else(|| "AIFF 缺 SSND 块".to_owned())?;
    if channels == 0 || channels > 64 {
        return Err(format!("AIFF 声道数 {channels} 不在 1..=64"));
    }
    if !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(format!("AIFF 位深 {bits} 不是 8/16/24/32"));
    }
    if sample_rate == 0 || sample_rate > 768_000 {
        return Err(format!("AIFF 采样率 {sample_rate} Hz 不在 1..=768000"));
    }
    let bytes_per_sample = usize::from(bits / 8);
    let frame_bytes = bytes_per_sample
        .checked_mul(usize::from(channels))
        .ok_or("AIFF 帧字节数溢出")?;
    if frame_bytes == 0 {
        return Err("AIFF 帧字节数为 0".to_owned());
    }
    let available = (end - start) / frame_bytes;
    let frames = usize::try_from(declared_frames).map_err(|_| "AIFF 帧数溢出".to_owned())?;
    if available < frames {
        return Err(format!(
            "AIFF SSND 数据不足：声明 {frames} 帧，实际只有 {available} 帧"
        ));
    }
    let total_samples = frames
        .checked_mul(usize::from(channels))
        .ok_or("AIFF 样本数溢出")?;
    let mut samples = Vec::with_capacity(total_samples);
    let scale = 1.0_f32 / (1u64 << (bits - 1)) as f32;
    let mut cursor = start;
    for _ in 0..total_samples {
        let value = match bits {
            8 => {
                let raw = bytes[cursor] as i8;
                cursor += 1;
                f32::from(raw)
            }
            16 => {
                let raw = i16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]);
                cursor += 2;
                f32::from(raw)
            }
            24 => {
                let raw =
                    i32::from_be_bytes([bytes[cursor], bytes[cursor + 1], bytes[cursor + 2], 0])
                        >> 8;
                cursor += 3;
                raw as f32
            }
            _ => {
                let raw = i32::from_be_bytes([
                    bytes[cursor],
                    bytes[cursor + 1],
                    bytes[cursor + 2],
                    bytes[cursor + 3],
                ]);
                cursor += 4;
                raw as f32
            }
        };
        samples.push(value * scale);
    }
    Ok(DecodedAsset {
        samples,
        source_channels: channels,
        sample_rate,
        frames: u64::try_from(frames).unwrap_or(u64::MAX),
    })
}

/// AIFF 的 80-bit IEEE 754 扩展精度采样率（1 位符号 + 15 位指数 + 64 位尾数）。
fn read_extended_80(bytes: &[u8]) -> Result<u32, String> {
    if bytes.len() < 10 {
        return Err("AIFF 采样率字段短于 10 字节".to_owned());
    }
    let exponent = u16::from_be_bytes([bytes[0], bytes[1]]);
    let mantissa = u64::from_be_bytes([
        bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
    ]);
    if exponent == 0 || mantissa == 0 {
        return Ok(0);
    }
    let value = (mantissa as f64 / (1u64 << 63) as f64) * 2f64.powi(i32::from(exponent) - 16383);
    if !value.is_finite() || value <= 0.0 {
        return Err("AIFF 采样率不是有限正数".to_owned());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value.round() as u32)
}

/// 把资产字节写进唯一临时文件、用**本仓唯一的 WAV 读取器**解码、随即删除。
///
/// ⛔ 不新增 `yeban-render` 的内存读取器（那是另一个 crate 的公开面）；用临时文件这条
/// **纯 app 内**的路子。
fn decode_wav_bytes(bytes: &[u8]) -> Result<DecodedAsset, String> {
    use std::io::Write;

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut last_error = String::new();
    for _ in 0..8 {
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yeban-audio-clip-{}-{serial}.wav",
            std::process::id()
        ));
        // ⭐ `create_new`（`O_EXCL`）⇒ 不跟随既有符号链接、不覆盖别人的文件：
        // 临时目录是共享的，普通 `write` 会跟着一个预先放好的符号链接写到别处去。
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let _guard = TempFile(path.clone());
                file.write_all(bytes)
                    .map_err(|error| format!("写临时 WAV 失败: {error}"))?;
                drop(file);
                let (format, buffer) =
                    read_plain_wav(&path).map_err(|error| format!("读临时 WAV 失败: {error}"))?;
                let samples = normalize_samples(&buffer);
                let source_channels = format.channels.max(1);
                let frames =
                    u64::try_from(samples.len() / usize::from(source_channels)).unwrap_or(0);
                return Ok(DecodedAsset {
                    samples,
                    source_channels,
                    sample_rate: format.sample_rate,
                    frames,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_error = error.to_string();
                continue;
            }
            Err(error) => return Err(format!("创建临时 WAV 失败: {error}")),
        }
    }
    Err(format!("创建临时 WAV 失败（连续重名）: {last_error}"))
}

/// 临时文件的删除守卫（`Drop` 时删除，成功失败都不留垃圾）。
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 交织 PCM ⇒ 归一化 `f32`（**保留声道数**）。
///
/// 归一化表与 [`yeban_render::wav::read_plain_wav_mono_f32`] 的文档**逐条相同**
/// （浮点直用；`Int16` / `Int24` 除以 `BitDepth::full_scale_max`），差异只在"不折成单声道"。
fn normalize_samples(buffer: &PcmBuffer) -> Vec<f32> {
    let scale = match buffer.depth() {
        BitDepth::Float32 => 1.0_f32,
        depth => 1.0_f32 / (depth.full_scale_max().max(1) as f32),
    };
    match buffer {
        PcmBuffer::Int16(values) => values
            .iter()
            .map(|value| f32::from(*value) * scale)
            .collect(),
        PcmBuffer::Int24(values) => values.iter().map(|value| *value as f32 * scale).collect(),
        PcmBuffer::Float32(values) => values.iter().map(|value| *value * scale).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use yeban_render::rf64::PcmFormat;
    use yeban_render::wav::write_plain_wav;

    use super::*;

    /// 造一份**合成 AIFF**（16-bit / 2 声道 / 3 帧 / 48 kHz；⛔ 不含任何 demo 字节）。
    ///
    /// 结构：`FORM` + `AIFF` + `COMM`（channels / frames / bits / 80-bit rate）+ `SSND`。
    fn synthetic_aiff() -> Vec<u8> {
        let channels: u16 = 2;
        let frames: u32 = 3;
        let rate: u32 = 48_000;
        // 80-bit extended：48000 = 1.46484375 × 2^15 ⇒ 指数 16383+15，尾数 = 48000 << 48。
        let exponent = (16_383 + 15u16).to_be_bytes();
        let mantissa = (u64::from(rate) << 48).to_be_bytes();
        let mut comm = Vec::new();
        comm.extend_from_slice(&channels.to_be_bytes());
        comm.extend_from_slice(&frames.to_be_bytes());
        comm.extend_from_slice(&16u16.to_be_bytes());
        comm.extend_from_slice(&exponent);
        comm.extend_from_slice(&mantissa);
        assert_eq!(comm.len(), 18);
        let samples: [i16; 6] = [0, 16_384, -16_384, 32_767, -32_768, 100];
        let mut ssnd = Vec::new();
        ssnd.extend_from_slice(&0u32.to_be_bytes()); // offset
        ssnd.extend_from_slice(&0u32.to_be_bytes()); // block size
        for sample in samples {
            ssnd.extend_from_slice(&sample.to_be_bytes());
        }
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&0u32.to_be_bytes()); // 回填
        file.extend_from_slice(b"AIFF");
        file.extend_from_slice(b"COMM");
        file.extend_from_slice(&(comm.len() as u32).to_be_bytes());
        file.extend_from_slice(&comm);
        file.extend_from_slice(b"SSND");
        file.extend_from_slice(&(ssnd.len() as u32).to_be_bytes());
        file.extend_from_slice(&ssnd);
        let size = u32::try_from(file.len() - 8).expect("夹具长度");
        file[4..8].copy_from_slice(&size.to_be_bytes());
        file
    }

    /// 判据（`#152`）：AIFF/PCM 真的能解码，且事实来自字节本身。
    #[test]
    fn aiff_pcm_is_decoded_from_the_source_bytes() {
        let bytes = synthetic_aiff();
        let hash = AssetHash::of_bytes(&bytes);
        let decoded = decode_asset_bytes(&hash, &bytes).expect("合成 AIFF 必须能解码");
        assert_eq!(decoded.frames, 3);
        assert_eq!(decoded.source_channels, 2);
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.samples.len(), 6);
        assert!((decoded.samples[0] - 0.0).abs() < 1e-6);
        assert!((decoded.samples[1] - 0.5).abs() < 1e-3);
        assert!((decoded.samples[2] + 0.5).abs() < 1e-3);
        assert!((decoded.samples[3] - 0.999_969).abs() < 1e-3);
        assert!((decoded.samples[4] + 1.0).abs() < 1e-6);
        // 哈希键仍必须是**源字节**的 sha256（⛔ 解码不改写它）。
        assert_eq!(hash, AssetHash::of_bytes(&bytes));
    }

    /// 写一份**真 WAV** 字节（440 Hz / 16-bit / 单声道；写完即删临时文件）。
    fn wav_bytes(tag: &str, rate: u32, frames: usize) -> Vec<u8> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yeban-141-audio-clip-{tag}-{}-{serial}.wav",
            std::process::id()
        ));
        let samples: Vec<i16> = (0..frames)
            .map(|index| {
                let phase = core::f64::consts::TAU * 440.0 * (index as f64) / f64::from(rate);
                (0.25 * 32_767.0 * phase.sin()).round() as i16
            })
            .collect();
        write_plain_wav(
            &path,
            &PcmFormat::integer(1, rate, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        let bytes = std::fs::read(&path).expect("读夹具 WAV");
        std::fs::remove_file(&path).expect("删夹具 WAV");
        bytes
    }

    /// 判据 §4③（单元级）：48k 素材 + 48k 导出 ⇒ **逐位相同**（`f32::to_bits`），
    /// 即 `decode_checked` 在同率时不复制、不滤波、不改样本。
    #[test]
    fn same_rate_decode_is_bit_identical_to_the_plain_decoder() {
        let bytes = wav_bytes("same", 48_000, 4_800);
        let hash = AssetHash::of_bytes(&bytes);
        let plain = decode_asset_bytes(&hash, &bytes).expect("纯解码必须成功");
        let checked = decode_checked(&hash, &bytes, 48_000).expect("同率检查必须成功");
        assert_eq!(checked.sample_rate, 48_000, "同率 ⇒ 采样率原样");
        assert_eq!(checked.frames, plain.frames, "同率 ⇒ 帧数原样");
        assert_eq!(checked.samples.len(), plain.samples.len());
        let plain_bits: Vec<u32> = plain.samples.iter().map(|value| value.to_bits()).collect();
        let checked_bits: Vec<u32> = checked
            .samples
            .iter()
            .map(|value| value.to_bits())
            .collect();
        assert_eq!(
            checked_bits, plain_bits,
            "同率样本必须逐位相同（f32::to_bits）"
        );
        println!(
            "[#141/same-rate] frames={} samples={} f32-bits-equal=true",
            checked.frames,
            checked.samples.len()
        );
    }

    /// 判据 §4①②（单元级）：44.1k → 48k 的输出帧数被钉到
    /// `ceil(源帧 × 48000 / 44100)` ⇒ 时长不变；样本数确实变了 ⇒ ⛔ 不是"只改标签"。
    #[test]
    fn a_44k1_decode_is_resampled_and_the_frame_count_is_pinned() {
        let frames = 4_410usize;
        let bytes = wav_bytes("up", 44_100, frames);
        let hash = AssetHash::of_bytes(&bytes);
        let checked = decode_checked(&hash, &bytes, 48_000).expect("44.1k ⇒ 48k 必须成功");
        let expected = (frames as u128 * 48_000).div_ceil(44_100);
        assert_eq!(checked.sample_rate, 48_000, "换算后采样率 = 导出率");
        assert_eq!(
            u128::from(checked.frames),
            expected,
            "帧数必须按时长口径 ceil 钉死"
        );
        assert_eq!(
            checked.samples.len(),
            usize::try_from(expected).expect("帧数放得进 usize")
        );
        assert_ne!(
            checked.samples.len(),
            frames,
            "44.1k→48k 必须真的多出样本（⛔ 只改采样率标签）"
        );
        println!(
            "[#141/44k1] frames {frames} → {}（{:.3} s 不变）；samples {frames} → {}",
            checked.frames,
            checked.frames as f64 / 48_000.0,
            checked.samples.len()
        );
    }

    /// 判据 §4④（单元级）：重采样失败必须是**逐字可读的 Err**（调用方据此记跳过），
    /// ⛔ 不是静默静音。
    #[test]
    fn a_resample_failure_returns_a_loud_reason() {
        let bytes = wav_bytes("fail", 44_100, 441);
        let hash = AssetHash::of_bytes(&bytes);
        let reason = decode_checked(&hash, &bytes, 0).expect_err("0 Hz 目标率必须失败");
        assert!(
            reason.contains("重采样失败"),
            "失败原因必须明说重采样失败: {reason}"
        );
        assert!(
            reason.contains("已**跳过**"),
            "失败原因必须明说会跳过（⛔ 不静默）: {reason}"
        );
        println!("[#141/resample-failure] reason={reason}");
    }
}
