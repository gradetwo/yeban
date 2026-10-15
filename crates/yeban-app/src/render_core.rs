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
//! ## ⭐ 轨增益（推子／静音／独奏）：三条支路**同径**（本单闭合 `R713` 登记的边界）
//!
//! | 支路 | 音源 | 轨增益从哪来 |
//! | :--- | :--- | :--- |
//! | 引擎乐器（默认） | [`EngineTrackSource`] → `SynthEngine::render_track` | ⭐ 引擎**内部**：鼓机读 `ScheduledNote::gain`、复音读同一个 `note.gain`、SFZ 声部读 `NoteSchedule::track_gain` —— 三者都来自 `snapshot::project_schedules` 里**同一次** [`yeban_engine::synth::track_gain_scale`] |
//! | **SFZ 旁路**（`--sfz`） | [`crate::sfz_load::SfzTrackSource`] | ⭐ **本模块**在构造期逐轨调**同一个** [`track_gain_of`]（⇒ 同一个 `track_gain_scale`）再经 `with_track_gain` 注入 |
//! | 最小音源（`use_engine_source = false`） | [`NoteToneSource`] | ⚠ **未接**（见下） |
//!
//! ⚠ **为什么 SFZ 旁路原先吃不到**：它**整个替换**引擎音源，且本模块在带 SFZ 时
//! **不建引擎快照**（上面的 `snapshot` 判据）⇒ 那份"投影期已乘好的"轨增益不存在。
//! 修法**不是**在这里写一份 `volume_db ⇒ 线性`（那会与引擎分叉），而是调引擎的
//! **同一个**算子。
//!
//! ⚠ **仍然如实登记的缺口**：最小音源 [`NoteToneSource`] **没有**轨增益（它只作判据对照
//! 与极端回退 —— 产品路径 `--export-wav` / `record_wav` 都给 `use_engine_source = true`）。
//! ⛔ 本单**不动**它：它不属本单受理的那条边界（SFZ 旁路），且改它会让那条对照臂的产物
//! 字节变化，需要单独一条判据与登记。
//!
//! ⚠ **音频片段**（[`crate::audio_clip`]）另有一套自己的"片段增益 ＋ 轨音量 ＋ 声相"
//! 组合（`audio_clip.rs:282` 的 `db_to_linear(clip_gain_db + track_volume_db)`）——
//! 它**不是** `track_gain_scale`，本单如实登记，⛔ 不在此处改（会动既有音频片段判据）。
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

/// 一条轨的**引擎层轨增益门**（可闻门 ＝ 静音/独奏 × 推子 × …）——⭐ **引擎的同一个算子**。
///
/// ⛔ 本函数**不**自己换算 dB：它只把工程的 `mute` / `solo` / `solo_safe` / `volume_db`
/// 原样交给引擎的 [`yeban_engine::synth::track_gain_scale`] —— 那个函数是引擎快照投影
/// （`snapshot::project_schedules`）与声部槽（`SynthEngine::set_track_gain_scales`）
/// 共用的**唯一**实现（⛔ 不是"这里再写一遍再对齐"）。
///
/// ⚠ `any_solo` 与引擎**同口径**：扫**全部**轨道（含主总线那一份），见
/// `crates/yeban-engine/src/snapshot.rs:907`。
///
/// ⚠ 找不到该轨（正常不会发生：`renderable` 已按 `project.tracks` 过滤）⇒ `0.0`
/// （静音）—— 宁可无声，⛔ 不是"忘了乘 ⇒ 满增益"。
fn track_gain_of(project: &YebanProjectV1, track_id: EntityId, any_solo: bool) -> f32 {
    project.tracks.get(&track_id).map_or(0.0, |track| {
        yeban_engine::synth::track_gain_scale(
            track.mute,
            track.solo,
            track.solo_safe,
            any_solo,
            track.volume_db,
        )
    })
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
        // ⭐ **轨增益门**（推子 × 静音/独奏）：`any_solo` 与引擎同口径（含主总线那一份）。
        // ⚠ 这段必须在这里算：SFZ 分支**不建引擎快照**（见上面的 `snapshot` 判据），
        // 因此拿不到 `NoteSchedule::track_gain`；改调**同一个** `track_gain_scale` 就不会分叉。
        let any_solo = project.tracks.values().any(|track| track.solo);
        for track_id in renderable.clone() {
            let notes = track_notes_in_frames(project, track_id, options.sample_rate);
            let track_gain = track_gain_of(project, track_id, any_solo);
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
            }
            // ⭐ 与引擎侧同形（`NoteSchedule::with_track_gain`）：轨增益乘在**乐器层输出之外**。
            .with_track_gain(track_gain);
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

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::TrackV3;

    /// 夹具：`filled_project` 里那条**带 MIDI 音符**的非主总线轨（＝ 工程里唯一会发声的
    /// 可渲染轨：另一条音频轨没有资产池 ⇒ 被如实跳过），把三个自变量写进去。
    fn fixture(volume_db: f32, mute: bool, solo_elsewhere: bool) -> (YebanProjectV1, EntityId) {
        let mut project = yeban_model::samples::filled_project();
        let lead = *project
            .tracks
            .iter()
            .find(|(id, track)| {
                **id != project.master_bus_track_id
                    && track.clips.values().any(|placement| {
                        project
                            .clip_pool
                            .get(&placement.clip_id)
                            .is_some_and(|entry| entry.content.notes().is_some())
                    })
            })
            .map(|(id, _)| id)
            .expect("filled_project 必须有一条带 MIDI 音符的非主总线轨");
        {
            let track = project.tracks.get_mut(&lead).expect("上面刚找到");
            track.volume_db = volume_db;
            track.mute = mute;
        }
        if solo_elsewhere {
            let other = EntityId::new();
            project.tracks.insert(
                other,
                TrackV3 {
                    id: other,
                    name: "Solo".to_owned(),
                    solo: true,
                    ..TrackV3::default()
                },
            );
            project.routing_graph.nodes.push(other);
        }
        (project, lead)
    }

    /// ⭐ 判据（本单新增）：**同一个算子**。
    ///
    /// # 量什么（先说再跑）
    ///
    /// 同一份工程上，[`track_gain_of`]（＝ SFZ 旁路注入的那个标量）与引擎快照投影写进
    /// `NoteSchedule::track_gain` 的那个标量，**逐位相同的 f32**（单位：线性增益）。
    ///
    /// * 对象 ＝ 一条轨的轨增益门（可闻门 × 推子）；
    /// * 单位 ＝ 线性幅度倍数（无量纲，`1.0` ＝ 单位增益、`0.0` ＝ 静音）。
    ///
    /// # 为什么必须逐位比
    ///
    /// 判据是"**与引擎用同一个函数／同一套字段**，⛔ 不另写一份 `volume_db ⇒ 线性` 换算"。
    /// 只比"接近"会放过"两份实现恰好都差不多"的情形；逐位比恰好就是"同一次调用的结果"
    /// （引擎侧注释的原话：`slot.gain_scale` 与 `NoteSchedule::track_gain` **逐位相等**）。
    ///
    /// # 怎么变红
    ///
    /// 把 [`track_gain_of`] 改成自己换算（例如 `db_to_linear(volume_db)` 而丢掉
    /// `track_is_audible` 门）⇒ 静音档 / 别轨独奏档当场不等；换成
    /// `10f32.powf(db / 20.0)` ⇒ 末位与引擎的 `exp2` 口径不同，`to_bits()` 也当场不等。
    #[test]
    fn the_sfz_track_gain_is_bit_identical_to_the_engine_projection() {
        for (volume_db, mute, solo_elsewhere) in [
            (0.0_f32, false, false),
            (-6.0, false, false),
            (-3.5, false, false),
            (0.0, true, false),
            (0.0, false, true),
        ] {
            let (project, lead) = fixture(volume_db, mute, solo_elsewhere);
            let any_solo = project.tracks.values().any(|track| track.solo);
            let ours = track_gain_of(&project, lead, any_solo);
            let snapshot = EngineSnapshot::from_project(&project, 1).expect("夹具工程必须成快照");
            let theirs = snapshot
                .schedules()
                .get(&lead)
                .expect("本轨必须有调度表")
                .track_gain();
            println!(
                "[app-sfz-gain] volume_db={volume_db} mute={mute} solo_elsewhere={solo_elsewhere}: \
                 app={ours:.9} ({:#010x}) vs 引擎投影={theirs:.9} ({:#010x})",
                ours.to_bits(),
                theirs.to_bits()
            );
            assert_eq!(
                ours.to_bits(),
                theirs.to_bits(),
                "⭐ SFZ 旁路注入的轨增益必须与引擎投影 `NoteSchedule::track_gain` **逐位相同**\
                 （volume_db={volume_db} mute={mute} solo_elsewhere={solo_elsewhere}）"
            );
        }
    }
}
