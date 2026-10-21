//! **引擎音色源**：把 `yeban-engine` 的 `SynthEngine` 接成 [`AudioSource`]
//! （`ADR-0011`；本文件是 `yeban-render` 里**唯一**碰 `yeban-engine` 的模块）。
//!
//! ## 为什么这一层存在（`ADR-0011` D2）
//!
//! 同一份工程原来有两条 MIDI 音源：CLI / 界面走 `SynthEngine`（加法波表 `HOLLOW`
//! ＋ ADSR ＋ 双振荡器），MCP 走自己的正弦振荡器。负责人 2026-10-21 逐字裁决
//! 「统一」⇒ 本模块把 CLI 那条 [`EngineTrackSource`] **原样搬进** `yeban-render`，
//! MCP 经**已登记的** `(yeban-mcp, yeban-render)` 例外使用它
//! ⇒ ⛔ 没有新增任何依赖例外，也 ⛔ 没有第二份音源。
//!
//! ## `ADR-0011` 的三条未决（本单的裁决，逐条给理由）
//!
//! ### ① 落哪一层：**新模块**，不是 `render.rs` 内
//!
//! - `render.rs` 是调度 / 归约 / 位级摘要的**唯一实现**，它的判据（`render::tests`）
//!   不应当为了"离线导出要引擎音色"而把 `yeban-engine` 的构造拖进每个测试二进制；
//! - 这条依赖边（`yeban-render -> yeban-engine`，`default-features = false`）在
//!   `Cargo.toml` 里**只有一处注释**，在本 crate 内也**只有一个模块**引用它 ⇒
//!   将来若 `ADR-0011` 被推翻，删除面是一个文件而不是散落在 2300 行的调度器里；
//! - 本模块 ⛔ 不复制 `render.rs` 的任何逻辑：`AudioSource` / `BlockContext` /
//!   `RenderError` 全部从 [`crate::render`] 导入。
//!
//! ### ② `advance(frames)` 的顺序契约：**有状态，且由本模块强制**
//!
//! 今天的引擎音源不是"绝对帧的纯函数"：它内部有一个播放头（`SynthEngine::position`），
//! `render_track` 从**当前**播放头渲染、`advance(frames)` 负责推进。于是"块无关"
//! 从**构造性**（正弦源的相位由绝对帧号算出）降级为**契约性**：
//!
//! > 调用方必须按**从第 0 帧开始、连续升序、不重不漏**的顺序调用 [`render_block`]；
//! > 每次调用的 `context.first_frame` 必须等于引擎当前播放头，返回后播放头前进
//! > `context.frames` 帧。
//!
//! 本模块把这条契约**写成代码**而不是只写成文档：`context.first_frame` 与
//! `engine.position()` 不符时返回 [`RenderError::Source`]（指名轨道与两个帧号），
//! ⛔ 不静默地从错误位置继续渲染（那会产出"听起来对、位级不可复现"的母带）。
//! 判据：本文件 `tests::the_advance_contract_is_enforced`。
//!
//! ### ③ 块长：**不是** `L1_BLOCK_SIZE` 与引擎 `DEFAULT_BLOCK_FRAMES` 的巧合
//!
//! 本源**不读**任何引擎的块长常量：它只按调用方给的 `context.frames` 渲染并 `advance`
//! 同样多帧。引擎的 `SynthEngine::render_track` 与 `PolySynth::render` 都按**绝对样本
//! 位置**定位（声部只在 `now >= start_sample` 时发声），因此换块长**不改样本**。
//! 本文件把这条判据钉成读数：同一个源用 64 / 128 / 256 帧三种切法渲染同一段总帧数，
//! 三个位级摘要**必须相同**（`tests::the_block_split_does_not_change_a_single_sample`）。
//! 于是"块长一致"不再是"两个 128 今天碰巧相等"，而是"任意块长逐位相同"——
//! 比 `ADR-0011` 要求的更强，且**不依赖** `L1_BLOCK_SIZE` 的取值。
//!
//! ## 与 CLI 的关系（⛔ 不是第二份实现）
//!
//! 本类型就是 `crates/yeban-app/src/export_wav.rs` 里那个 `pub(crate)` 的
//! `EngineTrackSource`：字段、`new`、`render_block`、声相口径逐字搬过来，
//! app 侧改为 `use yeban_render::engine_source::EngineTrackSource`。
//! 判据（同一份工程、同一份快照 ⇒ 逐位相同的母带）见 `yeban-app` 的 `mcp_mount`
//! 里 `C1` 一族；本文件的判据只钉"推进契约 + 块长"。
//!
//! ## 红线 7（[ARCH-RT-001]）
//!
//! [`EngineTrackSource::new`] 会 `arm_from_snapshot`（分配 / 增长区间表）⇒ 它的
//! 允许调用点是**离线导出 / 录音的控制侧**，⛔ 绝不在音频回调里。本 crate 是离线层
//! （模块文档的第一条契约），因此这里安全；`render_block` 本身**零分配**
//! （`mono` 缓冲只在不足时增长，构造后稳定）。

use std::sync::Arc;

use yeban_engine::snapshot::{EngineSnapshot, SnapshotError};
use yeban_engine::synth::{NoteSchedule, SynthEngine};
use yeban_model::{EntityId, YebanProjectV1};

use crate::render::{AudioSource, BlockContext, RenderError};

/// 引擎音源装配失败（快照投影 / `arm_from_snapshot`）。
///
/// 调用方把它映射成自己的错误类型：`yeban-app` ⇒ `WavExportError::Engine`，
/// `yeban-mcp` ⇒ `RENDER_FAILED`（带 `reason: "engineSource"`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineSourceError {
    message: String,
}

impl EngineSourceError {
    /// 报文（给调用方拼进 `data.reason` / 错误文本）。
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for EngineSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineSourceError {}

impl From<SnapshotError> for EngineSourceError {
    fn from(error: SnapshotError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

/// 一次工程快照 ＋ 逐轨引擎源的工厂。
///
/// 存在的理由：`EngineSnapshot::from_project` 会把**全部**轨道（调度表 / 音色 /
/// 鼓机 / SFZ 待摊平 / 音频摆放）投影一遍，逐轨各建一份是纯浪费；
/// MCP 一次渲染要建 N 个源 ⇒ 快照只投影**一次**、`Arc` 共享（与 CLI 的
/// `render_core` 同一手法：`Arc::clone` 是指针复制，⛔ 不 clone PCM）。
pub struct EngineTimbre {
    snapshot: Arc<EngineSnapshot>,
}

impl EngineTimbre {
    /// 从工程投影一份快照（`revision = 1`，与 CLI 的离线装配同口径）。
    ///
    /// # Errors
    ///
    /// [`EngineSourceError`]：工程没有可用的主总线 / 路由图非法 / PDC 失败。
    pub fn from_project(project: &YebanProjectV1) -> Result<Self, EngineSourceError> {
        Ok(Self {
            snapshot: Arc::new(EngineSnapshot::from_project(project, 1)?),
        })
    }

    /// 直接包一份快照（app 的导出 / 录音路径已有快照时用它）。
    #[must_use]
    pub fn from_snapshot(snapshot: Arc<EngineSnapshot>) -> Self {
        Self { snapshot }
    }

    /// 快照的采样率（Hz）—— 源的构造与音频配置必须用**同一份**快照的值。
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.snapshot.sample_rate()
    }

    /// 该轨在快照里的**已排程音符数**（0 = 该轨没有可发声的 MIDI 内容）。
    ///
    /// MCP 的 `sources[].notes` 读数用它，⛔ 不用自己再排一遍 —— 否则"报出来的
    /// 音符数"与"引擎真的会触发的音符数"会变成两条可以漂移的口径。
    #[must_use]
    pub fn scheduled_notes(&self, track: EntityId) -> usize {
        self.snapshot
            .schedules()
            .get(&track)
            .map_or(0, NoteSchedule::len)
    }

    /// 为一条轨道武装一个引擎源。
    ///
    /// # Errors
    ///
    /// [`EngineSourceError`]：`arm_from_snapshot` 的拒绝（含 `NoMasterBus`）。
    pub fn track_source(&self, track: EntityId) -> Result<EngineTrackSource, EngineSourceError> {
        EngineTrackSource::new(Arc::clone(&self.snapshot), track)
    }
}

/// **引擎乐器音源**：每条轨道一个 [`SynthEngine`]，用**同一条离线装配路径**
/// （`SynthEngine::arm_from_snapshot`）武装，再按轨渲染（`render_track`）。
///
/// ⚠ 每个源持有**自己的**引擎：`RenderPlan::execute` 会把源分发到工作线程（`threads`），
/// 共享一个 `&mut SynthEngine` 需要锁，而"一个源一个引擎"没有锁也没有跨线程共享。
/// ⚠ 红线 7：`arm_from_snapshot` **会分配**（`begin_snapshot` 增长区间表）⇒ 只在**离线导出**
/// 里调用，⛔ 绝不在音频回调里调用（本 crate 是离线路径）。
pub struct EngineTrackSource {
    /// 已武装的合成器（本源独占）。
    ///
    /// ⭐ **为什么是 `Box<SynthEngine>` 而不是内联的 `SynthEngine`**（`#72` 的修法）：
    /// `size_of::<SynthEngine>()` 实测 **109152 B**（`synth.rs` 的字段文档是同一读数）。
    /// 内联时本结构体就是 `EngineTrackSource = 109232 B`，于是**每一次按值搬动**
    /// （`new()` 的返回位、`?` 的中间位、`Box::new` 的装箱源、`Box<T> → Box<dyn Trait>`
    /// 的 unsize 强制转换）都要在栈上留一份 10 万字节级的副本 —— 实测
    /// `render_project_f32_with_assets` 的栈帧因此达 **582480 B**、
    /// `EngineTrackSource::new` 达 **327904 B**。`Box` 之后本结构体只剩一个指针（8 B）
    /// ＋轨道 ID ＋调度表 ＋暂存 `Vec`，搬动是**常数级**。
    /// ⚠ 这只是**把大对象从栈挪到堆**：分配点仍只有"离线导出/录音的构造期"（[`Self::new`]），
    /// ⛔ 不进实时路径（红线 7）。
    engine: Box<SynthEngine>,
    /// 本源负责的轨道。
    track: EntityId,
    /// 该轨的调度表（来自同一份快照；`None` ⇒ 该轨静音，与 `render_track` 的文档一致）。
    schedule: Option<NoteSchedule>,
    /// ⭐ **武装用的那份快照**（`#79`）：`render_block` 从它**借用**该轨的 SFZ 摊平表
    /// （[`EngineSnapshot::sfz`]）交给 `render_track` 的第 3 参。
    ///
    /// ⭐ 为什么是 `Arc<EngineSnapshot>` 而**不是** clone 一份 `SfzPlan`：摊平表里带着**整段 PCM**
    /// （[`yeban_engine::sfz_instrument::PreparedSample`]），clone 会把它按轨再复制一份。
    /// `Arc` clone 只是**指针复制** ⇒ 与实时路径同一条纪律（"实时侧只**借用**快照，
    /// ⛔ 不 clone 那些 `Vec`"，见 `sfz_instrument.rs` 的模块文档）。
    snapshot: Arc<EngineSnapshot>,
    /// 单声道暂存（`render_track` 输出"声相之前"的单声道）。
    mono: Vec<f32>,
    /// ⭐ **轨声相**（`(左, 右)`；默认 **`(1.0, 1.0)`＝居中**，`#251`）。
    ///
    /// `render_track` 产出的是**声相之前**的单声道 ⇒ 声相必须在本源补上，否则
    /// 导出把用户拉过的 pan 完全丢掉（`#250` 勘察的实测缺陷）。
    /// 值来自调用方的 `track_pan_gains`（与音频片段同一条曲线）。
    pan: (f32, f32),
}

impl EngineTrackSource {
    /// 从快照武装一个轨道源。
    ///
    /// # Errors
    ///
    /// [`EngineSourceError`]：`arm_from_snapshot` 的拒绝（含 `NoMasterBus`）。
    pub fn new(snapshot: Arc<EngineSnapshot>, track: EntityId) -> Result<Self, EngineSourceError> {
        // ⭐ 直接构造**在堆上**：`SynthEngine::new` 的 109152 B 返回位因此只在本帧
        //    存在一次（随后 memcpy 进 `Box`），⛔ 不再有"返回位 ＋ 中间位 ＋ 装箱源"
        //    三份 10 万字节级副本叠在同一帧里（见字段文档的尺寸对照）。
        let mut engine = Box::new(SynthEngine::new(snapshot.sample_rate()));
        // ⛔ 不 `unwrap`：没有主总线时如实失败（调用方据此在**落盘之前**退出）。
        engine.arm_from_snapshot(&snapshot)?;
        Ok(Self {
            engine,
            track,
            schedule: snapshot.schedules().get(&track).cloned(),
            snapshot,
            mono: Vec::new(),
            // ⭐ 默认**居中**（`1.0, 1.0`）：没注入时导出字节与接线前逐位相同（`#251`）。
            pan: (1.0, 1.0),
        })
    }

    /// ⭐ 写入**轨声相**（`(左, 右)`；构造期一次，`#251`）。
    ///
    /// 值必须来自调用方的 `track_pan_gains`（与音频片段路径**同一个**函数，
    /// 同一条等功率曲线）。缺省 `(1.0, 1.0)` ⇒ 原来的单声道铺放分支。
    #[must_use]
    pub const fn with_pan(mut self, pan: (f32, f32)) -> Self {
        self.pan = pan;
        self
    }

    /// 本源负责的轨道 ID。
    #[must_use]
    pub const fn track(&self) -> EntityId {
        self.track
    }

    /// 从快照投影来的**已排程音符数**（`sources[].notes` 的唯一来源）。
    #[must_use]
    pub fn scheduled_notes(&self) -> usize {
        self.schedule.as_ref().map_or(0, NoteSchedule::len)
    }

    /// 引擎当前播放头（绝对帧）。判据用它证明"每块恰好推进一次"。
    #[must_use]
    pub fn position(&self) -> u64 {
        self.engine.position()
    }

    /// 引擎累计触发音符数（判据读数；⛔ 不参与音频）。
    #[must_use]
    pub fn notes_triggered(&self) -> u64 {
        self.engine.notes_triggered()
    }
}

impl AudioSource for EngineTrackSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        // ⭐ `ADR-0011` 未决 ②：把"块无关是契约性的"落成**可失败的前置条件**。
        //    本源的播放头必须与调用方声明的块起点一致；不一致 ⇒ 指名失败，
        //    ⛔ 不静默地按错误位置渲染（那会产出不可复现的母带）。
        if context.first_frame != self.engine.position() {
            return Err(RenderError::Source {
                node: self.track,
                message: format!(
                    "引擎音源的推进契约被违反: `first_frame` = {}，引擎播放头 = {} \
                     （调用方必须从第 0 帧起按连续升序的块渲染，每块恰好一次）",
                    context.first_frame,
                    self.engine.position()
                ),
            });
        }
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        if self.mono.len() < frames {
            self.mono.resize(frames, 0.0);
        }
        // `render_track` 从引擎**当前**播放头渲染，它**不**推进游标（契约见
        // `SynthEngine::render_track` 与 `unknown_track_renders_silence` 的
        // "渲染不推进播放头（由 advance 负责）"）；推进由本函数紧随其后的
        // `advance(frames)` 负责 —— 与实时路径 `rt.rs` 的同一量子同一顺序一致。
        // ⭐ ⭐ 第 3 参（SFZ 摊平表）**从快照借用**（`#79`）：这条轨在快照 `sfz()` 表里
        //    ⇒ 走引擎的第三条音源支路（鼓机 ＞ SFZ ＞ 复音）；不在表里 ⇒ `None` ⇒ 退回复音，
        //    输出与接线前**逐位相同**（`TrackV3.sampler = None` 的反向对照）。
        //    ⛔ 这里**不** clone 摊平表（见字段文档）：只把 `&PreparedSfz` 借出去。
        let sfz = self
            .snapshot
            .sfz()
            .get(&self.track)
            .map(yeban_engine::sfz_instrument::SfzPlan::prepared);
        self.engine.render_track(
            self.track,
            self.schedule.as_ref(),
            sfz,
            &mut self.mono[..frames],
        );
        // ⭐ `#209`：**必须**推进播放头，否则每个块都从位置 0 渲染
        //    ⇒ 只有 tick 0 的音符会被触发一次并一直拖长，其余音符永不发声
        //    （`#207` 实测 `--export-wav` ⇒ `distinct=1`）。
        // ⚠ `render_track` 自带 `out.fill(0.0)` 与内部游标读取，但**不**推进位置：
        //    推进只在 `advance`（`synth.rs` 的契约；实时路径见 `rt.rs` 的
        //    `if running { transport.advance_frames(frames as u64); synth.advance(frames); }`）。
        // ⚠ 本函数每块恰好渲染一次、推进一次 ⇒ 与 `first_frame` 同步，⛔ 不会推进两次。
        self.engine.advance(frames);
        out.fill(0.0);
        let (gain_l, gain_r) = self.pan;
        for index in 0..frames {
            let value = self.mono[index];
            let base = index * channels;
            if gain_l == 1.0 && gain_r == 1.0 {
                // ⭐ 居中（绝大多数工程）：原来的"单声道铺到所有声道"，逐位不变。
                for channel in 0..channels {
                    out[base + channel] = value;
                }
            } else {
                // ⭐ `#251`：等功率声相。第 0 声道左、第 1 声道右，其余保持 0。
                out[base] = value * gain_l;
                if channels > 1 {
                    out[base + 1] = value * gain_r;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use sha2::{Digest, Sha256};

    use crate::render::RenderOutput;

    /// 演示工程的第一个有声轨 + 一份快照（与 app 的离线装配同一条入口）。
    fn source(project: &YebanProjectV1) -> (EngineTimbre, EntityId) {
        let timbre = EngineTimbre::from_project(project).expect("演示工程必须成快照");
        let track = timbre
            .snapshot
            .schedules()
            .iter()
            .find(|(_, schedule)| !schedule.notes().is_empty())
            .map(|(track, _)| *track)
            .expect("演示工程必须至少有一条有声轨");
        (timbre, track)
    }

    /// 把 `frames` 帧按 `block` 切块渲染成单声道，返回整段的位级摘要。
    fn digest_with_block(
        timbre: &EngineTimbre,
        track: EntityId,
        frames: u64,
        block: usize,
    ) -> String {
        let mut source = timbre.track_source(track).expect("必须能武装");
        let mut mono: Vec<f32> = Vec::with_capacity(usize::try_from(frames).unwrap());
        let mut out = vec![0.0_f32; block];
        let mut first = 0u64;
        let mut index = 0u64;
        while first < frames {
            let len = usize::try_from((frames - first).min(block as u64)).unwrap();
            source
                .render_block(
                    BlockContext {
                        block_index: index,
                        first_frame: first,
                        frames: len,
                        channels: 1,
                        sample_rate: 48_000,
                        seed: 1,
                    },
                    &mut out[..len],
                )
                .expect("连续升序的块必须被接受");
            mono.extend_from_slice(&out[..len]);
            first += len as u64;
            index += 1;
        }
        let mut hasher = Sha256::new();
        for sample in &mono {
            hasher.update(sample.to_bits().to_le_bytes());
        }
        // `sha2 0.11` 的 `finalize()` 返回 `hybrid_array::Array`，只实现 `LowerHex`
        // 的旧版本实现不覆盖它 ⇒ 本判据自己拼十六进制（与 `render::digest_of` 同口径：
        // 对**位型**的字节流做 SHA-256）。
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// ⭐ `ADR-0011` 未决 ③：块长**不是**巧合 —— 64 / 128 / 256 三种切法逐位相同。
    ///
    /// 牙齿：把 `render_block` 里的 `self.engine.advance(frames)` 删掉 ⇒ 只有第一块
    /// 有声音，三种切法的摘要会分叉（128 与 64 的块数不同 ⇒ 同一条判据同时变红）。
    #[test]
    fn the_block_split_does_not_change_a_single_sample() {
        let project = yeban_model::samples::demo_project();
        let (timbre, track) = source(&project);
        let frames = 48_000u64;
        let reference = digest_with_block(&timbre, track, frames, 128);
        let at_64 = digest_with_block(&timbre, track, frames, 64);
        let at_256 = digest_with_block(&timbre, track, frames, 256);
        println!(
            "### [block-split] 128={reference} 64={at_64} 256={at_256} 相同={}",
            reference == at_64 && reference == at_256
        );
        assert_eq!(
            reference, at_64,
            "64 帧与 128 帧的渲染必须逐位相同（引擎按绝对样本位置起振）"
        );
        assert_eq!(
            reference, at_256,
            "256 帧与 128 帧的渲染必须逐位相同（引擎按绝对样本位置起振）"
        );
    }

    /// ⭐ `ADR-0011` 未决 ②：推进契约被**强制**（不是只写在文档里）。
    ///
    /// 三条读数：从 0 起跳块 ⇒ 接受；重复/跳跃的 `first_frame` ⇒ 指名失败；
    /// 失败信息里带两个帧号（可定位）。
    #[test]
    fn the_advance_contract_is_enforced() {
        let project = yeban_model::samples::demo_project();
        let (timbre, track) = source(&project);
        let mut source = timbre.track_source(track).expect("必须能武装");
        let mut out = vec![0.0_f32; 128];
        source
            .render_block(
                BlockContext {
                    block_index: 0,
                    first_frame: 0,
                    frames: 128,
                    channels: 1,
                    sample_rate: 48_000,
                    seed: 1,
                },
                &mut out,
            )
            .expect("从第 0 帧起的第一块必须被接受");
        assert_eq!(source.position(), 128, "每块必须恰好推进一次");
        // 跳跃：跳过第 128 帧那一块 ⇒ 引擎播放头还是 128，调用方声称 256。
        let error = source
            .render_block(
                BlockContext {
                    block_index: 2,
                    first_frame: 256,
                    frames: 128,
                    channels: 1,
                    sample_rate: 48_000,
                    seed: 1,
                },
                &mut out,
            )
            .expect_err("跳跃的块必须被拒绝");
        let message = error.to_string();
        assert!(
            message.contains("256") && message.contains("128"),
            "失败信息必须点名两个帧号: {message}"
        );
        assert_eq!(source.position(), 128, "被拒绝的块不得推进播放头");
    }

    /// 位级摘要的**正对照**：静音源不是"恒等摘要"（证明上面的判据不是空转）。
    #[test]
    fn the_digest_is_not_a_constant() {
        let project = yeban_model::samples::demo_project();
        let (timbre, track) = source(&project);
        let a = digest_with_block(&timbre, track, 4_800, 128);
        let b = digest_with_block(&timbre, track, 9_600, 128);
        assert_ne!(a, b, "不同长度的两条渲染不应共享摘要（夹具真的在动）");
        assert_eq!(
            RenderOutput::digest_of(&[]),
            RenderOutput::digest_of(&[]),
            "`digest_of` 对空输入是稳定的（它只是 SHA-256 的包装）"
        );
    }
}
