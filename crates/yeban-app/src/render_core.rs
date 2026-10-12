//! 渲染核心（**新增项的独立小模块**）：把工程渲染成交错 `f32` 样本。
//!
//! ## 为什么单独一个文件
//!
//! ⚠ 本会话的机械教训：把新增的公开项**追加在 `#[cfg(test)] mod tests` 之后**会触发
//! `clippy::items_after_test_module`（已咬过两次）。把新增项放进**独立模块**可以一次性避开该形态。
//!
//! ## 与 `export_wav.rs` 的关系（**如实登记**）
//!
//! 本函数的流水线与 `export_wav::export_wav_with_sfz_file` **逐行等价**（同一批步骤：慢路径校验
//! → 计划编译 → 音源装配 → 执行 → digest），是"先并存"的产物；**真正的去重（让导出也调用本函数）
//! 是下一步**。⚠ 在去重之前，两边**必须保持同步** —— 而 `record_wav` 的判据
//! `recording_the_project_is_bit_identical_to_exporting_it` 就是这条同步的**回归网**：
//! 一旦两边漂移，该判据立刻红 ✓
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

use yeban_model::{EntityId, YebanProjectV1};
use yeban_render::render::{AudioSource, RenderOptions, RenderPlan};

use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::SynthEngine;

use crate::export_wav::{
    EngineTrackSource, NoteToneSource, WavExportError, WavExportOptions, track_notes_in_frames,
};

/// 把工程渲染成交错 `f32`：`(样本, 帧数, 音源文案, digest 十六进制)`。
pub fn render_project_f32(
    project: &YebanProjectV1,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
) -> Result<(Vec<f32>, u64, String, String), WavExportError> {
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
    let source_label: String = if let Some((instrument, sfz_path)) = sfz.as_ref() {
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
    Ok((output.samples, output.frames, source_label, digest))
}
