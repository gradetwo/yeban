//! 渲染核心（**新增项的独立小模块**）：把工程渲染成交错 `f32` 样本。
//!
//! ## 为什么单独一个文件
//!
//! ⚠ 本会话的机械教训：把新增的公开项**追加在 `#[cfg(test)] mod tests` 之后**会触发
//! `clippy::items_after_test_module`（已咬过两次）。把新增项放进**独立模块**可以一次性避开该形态。
//!
//! ## 与 `export_wav.rs` 的关系（**去重已完成**）
//!
//! 本函数是导出与录音的**单一真源**（同一批步骤：慢路径校验 → 计划编译 → 音源装配 → 执行 →
//! digest）。`export_wav.rs` 的四个入口 `export_wav` / `export_wav_with_sfz` /
//! `export_wav_with_sfz_file` / `export_wav_with_sfz_file_and_assets` 逐级收束，末端那个
//! **已改为调用本模块的** [`render_project_f32_with_assets`]（`export_wav.rs:414-417`）；
//! `record_wav::record_project` 也走同一条 [`render_project_f32`]（`record_wav.rs:700`）。
//! ⇒ ⭐ "真正的去重（让导出也调用本函数）"**已经落地**，不再是"下一步"。
//! 判据 `recording_the_project_is_bit_identical_to_exporting_it` 仍是这条"两侧逐位一致"契约的
//! **回归网**：一旦漂移，该判据立刻红 ✓
//!
//! ## ⚠ 红线 7 与**调用方约束**
//!
//! 本函数**允许分配**（解析 SFZ／装载快照／建缓冲）⇒ ⭐ **两侧调用方（导出与录音）都必须是
//! 离线／控制侧**：`--export-wav` 是离线批处理，`record_wav::record_project` 只在**喂环之前**
//! 调它。⛔ **绝不可**在音频回调里调用；录音侧只有 `push_frame` 那一段是"禁止分配／加锁／I/O"的。
//!
//! # Errors
//!
//! 与 [`crate::export_wav::export_wav`] 相同（选项非法 / 计划编译 / 引擎装载 / 执行失败）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use yeban_model::{AssetHash, EntityId, YebanProjectV1};
use yeban_render::render::{AudioSource, BlockContext, RenderError, RenderOptions, RenderPlan};

use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::SynthEngine;

use crate::audio_clip;
use crate::export_wav::{
    EngineTrackSource, NoteToneSource, WavExportError, WavExportOptions, track_notes_in_frames,
};

/// 一条轨的复合源：**先 MIDI 基座、后音频片段**（顺序与 MCP 的 `TrackSource` 相同，
/// 且与块边界无关 ⇒ 逐位可复现）。
struct TrackSource {
    /// 既有音源（引擎乐器 / SFZ / 最小音源）。
    base: Box<dyn AudioSource>,
    /// 音频片段源（按资产哈希升序，每个素材一个）。
    clips: Vec<audio_clip::AudioClipSource>,
}

impl AudioSource for TrackSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        // 基座会清零（三个既有源都 `out.fill(0.0)`）；音频片段只**累加**。
        self.base.render_block(context, out)?;
        for clip in &mut self.clips {
            clip.render_block(context, out)?;
        }
        Ok(())
    }
}

/// 一次离线渲染的**全部读数**（含音频片段的渲染 / 跳过计数）。
#[derive(Clone, Debug, PartialEq)]
pub struct RenderProjectOutput {
    /// 交错 `f32` 样本。
    pub samples: Vec<f32>,
    /// 帧数。
    pub frames: u64,
    /// 音源文案（含音频片段计数）。
    pub source_label: String,
    /// 渲染输出的位级摘要（十六进制）。
    pub digest: String,
    /// 真的进了母带的音频摆放**个数**（不是素材个数）。
    pub audio_clips_rendered: u64,
    /// 被**如实跳过**的音频片段原因（例如采样率不匹配）。
    pub audio_clips_skipped: Vec<String>,
}

/// 把工程渲染成交错 `f32`：`(样本, 帧数, 音源文案, digest 十六进制)`。
///
/// ⚠ 本入口**没有**资产池 ⇒ 音频片段被如实跳过并计数（`record_wav` 与既有判据仍用它）。
/// 生产导出（`--export-wav`）走 [`render_project_f32_with_assets`]。
pub fn render_project_f32(
    project: &YebanProjectV1,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
) -> Result<(Vec<f32>, u64, String, String), WavExportError> {
    let output = render_project_f32_with_assets(project, options, sfz, &[])?;
    Ok((
        output.samples,
        output.frames,
        output.source_label,
        output.digest,
    ))
}

/// 与 [`render_project_f32`] 相同，但**带上归档资产池** ⇒ `ClipContent::Audio` 真的发声。
///
/// `assets` 的形状就是 `ProjectArchive.assets`。给空切片时与 [`render_project_f32`]
/// 逐位相同（音频片段被跳过并计数）。
///
/// # Errors
///
/// 与 [`render_project_f32`] 相同。
pub fn render_project_f32_with_assets(
    project: &YebanProjectV1,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
    assets: &[(AssetHash, Vec<u8>)],
) -> Result<RenderProjectOutput, WavExportError> {
    if options.frames == 0 {
        return Err(WavExportError::Empty("请求的帧数为 0".to_owned()));
    }
    // ---- 0. 引擎装载（**慢路径校验先行**：引擎文档要求先做慢路径校验，再进入渲染循环）----
    //
    // 为什么提前：`arm_from_snapshot` 在**没有可武装的轨/音色/鼓**时返回 `NoMasterBus`；
    // 它是"这个工程能不能用引擎渲染"的判定，理应在计划编译与任何 I/O 之前发生
    // ⇒ 失败时一个字节都不会落盘，也不会先被计划层的次生错误（例如"没有注册样本源"）掩盖。
    // 带 SFZ ⇒ 走 SFZ 音源（⛔ 不需要引擎快照；SFZ 自己的解析在离线路径里做）。
    let snapshot = if options.use_engine_source && sfz.is_none() {
        let snapshot = EngineSnapshot::from_project(project, 1)
            .map_err(|error| WavExportError::Engine(error.to_string()))?;
        // ⭐ **慢路径校验只做一次**，且必须在这里做：武装是**逐源**发生的，
        // 而"没有任何可渲染轨"时一个源都不会被造出来 ⇒ 引擎的 `NoMasterBus` 裁决会被
        // 计划层的次生错误（"没有注册样本源"）掩盖（本机实测：清空轨道表时正是这样）。
        // ⇒ 用一个**探针引擎**先武装一次，让"能不能用引擎渲染这个工程"的判定发生在
        //   计划编译与任何 I/O **之前**（引擎文档要求先做慢路径校验）。
        let mut probe = SynthEngine::new(snapshot.sample_rate());
        probe
            .arm_from_snapshot(&snapshot)
            .map_err(|error| WavExportError::Engine(error.to_string()))?;
        Some(snapshot)
    } else {
        None
    };

    let options_render = RenderOptions {
        frames: options.frames,
        sample_rate: options.sample_rate,
        channels: options.channels,
        seed: options.seed,
        ..RenderOptions::default()
    };

    // ---- 1. 编译计划（图与 master 都来自工程本身，⛔ 不猜）----
    let mut plan = RenderPlan::compile(
        &project.routing_graph,
        project.master_bus_track_id,
        options_render,
    )
    .map_err(|error| WavExportError::Render(error.to_string()))?;

    // ---- 2. 每条非 master 轨道一个源 ----
    //
    // 默认走**引擎乐器**：先投影快照（`EngineSnapshot::from_project`），再按轨武装
    // （`arm_from_snapshot`，与实时路径同一条装配序列）。任何一步失败都在**落盘之前**返回
    // ⇒ 不留半成品。`use_engine_source = false` 时用最小音源（判据的对照 + 极端回退）。
    // 计划**自己**的节点表是唯一权威：`RenderPlan::nodes()`（图里存在但不可达的节点会被剪枝，
    // 给它塞源会得到"节点 … 不在渲染计划里（未知或被剪枝）" —— 本机实测踩过两次）。
    let renderable: Vec<EntityId> = plan
        .nodes()
        .into_iter()
        .filter(|id| *id != project.master_bus_track_id && project.tracks.contains_key(id))
        .collect();
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    let mut source_label: String = if let Some((instrument, sfz_path)) = sfz.as_ref() {
        let mut labels: Vec<String> = Vec::new();
        for track_id in renderable.clone() {
            let notes = track_notes_in_frames(project, track_id, options.sample_rate);
            let track_source = match sfz_path {
                Some(sfz_path) => crate::sfz_load::SfzTrackSource::new_from_sfz_path(
                    instrument.clone(),
                    sfz_path,
                    &notes,
                    options.sample_rate as f32,
                    options.channels,
                ),
                None => crate::sfz_load::SfzTrackSource::new(
                    instrument.clone(),
                    &notes,
                    options.sample_rate as f32,
                    options.channels,
                ),
            };
            labels.push(track_source.sample_source());
            sources.insert(track_id, Box::new(track_source));
        }
        labels
            .first()
            .map_or_else(|| "sfz(无区域)".to_owned(), |label| format!("sfz({label})"))
    } else if let Some(snapshot) = snapshot.as_ref() {
        for track_id in renderable.clone() {
            sources.insert(
                track_id,
                Box::new(EngineTrackSource::new(snapshot, track_id)?),
            );
        }
        "引擎乐器（SynthEngine::arm_from_snapshot + render_track）".to_owned()
    } else {
        for track_id in renderable.clone() {
            sources.insert(
                track_id,
                Box::new(NoteToneSource::from_track(
                    project,
                    track_id,
                    options.sample_rate,
                )),
            );
        }
        "最小音源（方波 + 首尾包络；回退）".to_owned()
    };

    // ---- 2b. 音频片段（`ClipContent::Audio`）叠加 ----
    //
    // 引擎快照把音频片段当静音（`snapshot.rs` 的 `project_schedules` 第 5 条登记），
    // 因此音频摆放必须由**本层**叠加；语义与 MCP 的 `AudioClipSource` 逐条对齐
    // （见 `crate::audio_clip` 的模块文档：帧区间 / 硬切 / 累加 / 增益 / 声道布局）。
    let mut audio_cache = BTreeMap::new();
    let mut audio_skipped: Vec<String> = Vec::new();
    let mut audio_rendered = 0_u64;
    for track_id in renderable.clone() {
        let clips = audio_clip::resolve_track_clips(
            project,
            track_id,
            options.sample_rate,
            options.channels,
            assets,
            &mut audio_cache,
            &mut audio_skipped,
        );
        if clips.sources.is_empty() {
            continue;
        }
        audio_rendered = audio_rendered.saturating_add(clips.spans);
        if let Some(base) = sources.remove(&track_id) {
            sources.insert(
                track_id,
                Box::new(TrackSource {
                    base,
                    clips: clips.sources,
                }),
            );
        }
    }
    if audio_rendered > 0 || !audio_skipped.is_empty() {
        source_label.push_str(&format!(
            " + audio-clips(rendered={audio_rendered} skipped={})",
            audio_skipped.len()
        ));
    }

    // ---- 3. 执行（全部在内存里）----
    let output = plan
        .execute(sources)
        .map_err(|error| WavExportError::Render(error.to_string()))?;

    // ---- 4. 逐个样本量化（TPDF 抖动；位深 24 bit 整数 PCM）----
    let mut digest = String::with_capacity(output.digest.len() * 2);
    for byte in &output.digest {
        use core::fmt::Write;
        let _ = write!(digest, "{byte:02x}");
    }
    Ok(RenderProjectOutput {
        samples: output.samples,
        frames: output.frames,
        source_label,
        digest,
        audio_clips_rendered: audio_rendered,
        audio_clips_skipped: audio_skipped,
    })
}
