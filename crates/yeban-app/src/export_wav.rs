//! `--export-wav`：把工程**离线渲染**成一段可播放的 WAV（`[ROAD-M4-*]` 的导出族，与
//! `export_midi`／`export_als`／`export_logic` 并列）。
//!
//! ## 数据流（每一步都复用既有设施，⛔ 无旁路）
//!
//! ```text
//! YebanProjectV1 ──▶ RenderPlan::compile(&project.routing_graph, project.master_bus_track_id, opts)
//!                 ──▶ RenderPlan::execute(sources: BTreeMap<EntityId, Box<dyn AudioSource>>)
//!                 ──▶ RenderOutput { samples: 交错 f32, frames, channels, digest }
//!                 ──▶ dither::quantize(_, options.bit_depth, &mut DeterministicDitherRng)
//!                     （16/24-bit ⇒ TPDF 抖动；32f ⇒ 原样透传）
//!                 ──▶ PcmBuffer::{Int16,Int24,Float32} ──▶ yeban_render::wav::write_plain_wav
//! ```
//!
//! ⛔ 本文件**不写 RIFF 头**（`write_plain_wav` 是唯一落盘点），⛔ 也**不另造渲染路径**
//! （一律 `RenderPlan`，与 `yeban-render` 的基准/门禁同源）。
//!
//! ## 音源：**引擎乐器**（默认），最小音源只作回退与对照
//!
//! 默认路径是**引擎乐器**，与实时路径**同一条装配序列**：
//!
//! ```text
//! EngineSnapshot::from_project(&project, 1)        // 工程 ⇒ 快照（既有）
//!       ⇒ arm_pending_sfz(&snapshot, hz, …)        // ⭐ #79：TrackV3.sampler ⇒ 控制侧解码 ⇒ with_sfz
//!       ⇒ SynthEngine::new(snapshot.sample_rate())
//!       ⇒ engine.arm_from_snapshot(&snapshot)      // ⭐ `mod-engine` 新增的公开入口
//!       ⇒ 每轨一个源：engine.render_track(track, snapshot.schedules().get(&track),
//!                                        snapshot.sfz().get(&track).map(SfzPlan::prepared), &mut mono)
//! ```
//!
//! ⚠ 第 3 个参数是 SFZ 音源（`Option<&PreparedSfz>`）。⭐ **任务单 `#79` 起它从快照里读**
//! （`EngineTrackSource` 持 `Arc<EngineSnapshot>`，⛔ 不 clone PCM）：
//! `TrackV3.sampler` 指到的 SFZ ⇒ 控制侧解码 ⇒ 摊平表 ⇒ 该轨走引擎 SFZ 分支。
//! `sampler = None` 的轨不在表里 ⇒ 那一参是 `None` ⇒ 退回复音合成器，输出与接线前**逐位相同**。
//!
//! ⚠ **`--sfz` 旁路仍在**（本文件下面那段）：它把**整工程**的音源替换成
//! [`crate::sfz_load::SfzTrackSource`]，与按轨的模型链条是两条入口；同时给 ⇒ 旁路优先。
//!
//! ⚠ **红线 7**：`arm_from_snapshot` **会分配**（`begin_snapshot` 增长区间表）⇒ 它的允许调用点是
//! **离线导出／控制线程装载工程／测试**，⛔ **绝不可**在音频回调里调用。本文件是离线路径 ✓。
//! ⚠ 没有主总线时 `arm_from_snapshot` 返回 `SnapshotError::NoMasterBus` ⇒ 本文件在**落盘之前**
//! 如实失败（⛔ 不 `unwrap`、⛔ 不留半成品）。
//!
//! ⭐ **SFZ 音源**（`export_wav_with_sfz` 的 `Some(sfz)`）：带 `--sfz` 时每条可渲染轨用
//! [`crate::sfz_load::SfzTrackSource`] ⇒ 导出的声音来自该 SFZ 的区域；同时给出 SFZ 文件路径时，
//! `sample=` 指向的 PCM WAV 会被**真读**（读不到则如实回退内存正弦）⇒ 报告行 `source=` 明说。
//!
//! `WavExportOptions::use_engine_source = false` 时才用**最小音源**（确定性方波 ＋ 首尾包络，
//! 相位由整数帧号派生）：它是**判据的对照**与极端回退，报告行里的 `source=` 字段**明说用了哪一个**
//! （⛔ 不让人误以为听到了乐器）。
//! ⭐ 它**也吃引擎层的轨增益**（推子／静音／独奏）：[`crate::render_core`] 在构造期用引擎的
//! **同一个**算子 [`yeban_engine::synth::track_gain_scale`] 逐轨算好，再经
//! [`NoteToneSource::with_track_gain`] 注入 —— 与 SFZ 旁路**同形**（见该类型文档）。
//!
//! ⭐ **音频片段**（`ClipContent::Audio`）：引擎快照把音频摆放当静音（`snapshot.rs` 的登记），
//! 因此它们由 [`crate::render_core`] 在 MIDI 基座之上**叠加**（[`crate::audio_clip`]，
//! 语义逐条对齐 MCP 的 `AudioClipSource`）。资产字节来自调用方给的**归档资产池**
//! （[`export_wav_with_sfz_file_and_assets`] 的第五个参数 = `ProjectArchive.assets`）⇒
//! `--export-wav` 真的渲染音频摆放。⛔ 没有资产池的入口（本文件的旧签名 / `record_wav`）
//! 会**如实跳过**并计数，⛔ 不是静默。
//!
//! ⭐ **声相**（`#251`，`#255` 起**四条**路径全覆盖）：MIDI **振荡器**路径（引擎乐器／最小音源）、
//! 音频片段路径、以及**两条采样器入口**（`--sfz` 旁路的 [`crate::sfz_load::SfzTrackSource`]、
//! `TrackV3.sampler` 的引擎轨）**都**应用轨声相，且共用
//! [`crate::audio_clip::track_pan_gains`]（`pan = 0` ⇒ `(1.0, 1.0)`，乘上去**逐位恒等**；
//! 否则走与 MCP **同一条**等功率曲线 `θ = (pan + 1)·π/4`）。
//! ⚠ 修前采样器轨**不**应用声相（与当时 MCP 的 `SamplerTrackSource` 同一条登记差异）；
//! `#255` 把**两侧一起**接上同一条曲线 ⇒ 同一 `pan` 下 CLI 与 MCP 的左右比相同。
//!
//! ## "拒绝时不留半成品"
//!
//! `write_plain_wav` 自带该纪律（先 `check_match` 校验，再建文件）。本文件在**调用顺序**上
//! 不破坏它：**全部渲染与量化都在内存里完成**，只在最后一步调用一次 `write_plain_wav`。

use std::path::{Path, PathBuf};

use yeban_engine::level::dbfs;
#[cfg(test)]
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::project::SampleRate;
use yeban_model::{EntityId, YebanProjectV1};
use yeban_render::dither::{BitDepth, quantize};
// ⭐ `ADR-0011`：引擎音色源已搬到 `yeban-render`（CLI 与 MCP 共用同一份，⛔ 不写第二份）。
#[cfg(test)]
use yeban_render::engine_source::EngineTrackSource;
use yeban_render::render::{AudioSource, BlockContext, RenderError};
use yeban_render::rng::DeterministicDitherRng;
use yeban_render::wav::{format_of, write_plain_wav};

/// 导出参数（秒数／采样率／声道／位深／种子）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavExportOptions {
    /// 总帧数。
    pub frames: u64,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: usize,
    /// 确定性种子（[`ARCH-DET-001]`：渲染不引入真熵源）。
    pub seed: u64,
    /// `true`（默认）= 用**引擎乐器**（`SynthEngine::arm_from_snapshot`）；
    /// `false` = 用**最小音源**回退（方波；判据用它做对照）。
    pub use_engine_source: bool,
    /// 导出的位深（`16` / `24` 整数走 TPDF 抖动，`32f` 浮点透传）。
    ///
    /// 唯一落盘点 [`write_interleaved_wav`] 用它选量化器与容器格式，
    /// ⛔ 位深**不再**硬编码在里面（[`WavExportOptions::resolve`] 是唯一解析处）。
    pub bit_depth: BitDepth,
}

/// `--wav-seconds` / UI 缺省时长（秒）。
///
/// ⭐ 与旧行为**逐字一致**：`--export-wav` 一直导出 8 秒。
pub const DEFAULT_WAV_SECONDS: f32 = 8.0;

impl Default for WavExportOptions {
    fn default() -> Self {
        Self {
            // 8 秒 48 kHz 立体声 24-bit：足以听见演示工程的内容，且导出足够快。
            frames: 48_000 * 8,
            sample_rate: 48_000,
            channels: 2,
            seed: 1,
            use_engine_source: true,
            bit_depth: BitDepth::Int24,
        }
    }
}

impl WavExportOptions {
    /// 从**可选参数**解析出一次导出真正用的选项（CLI 的 `--wav-*` 与 UI 的缺省
    /// 共用这一处 ⇒ ⛔ 不会出现"同一份渲染、两种字节"）。
    ///
    /// 四个参数全 `None` 时的口径（＝旧行为的**语义**）：
    ///
    /// - **采样率跟随工程** `project.audio_config.sample_rate`
    ///   （[`MODEL-AST-002`] 的唯一来源；工程缺省 48 kHz）。为什么必须跟随：
    ///   引擎音源 `SynthEngine::new(snapshot.sample_rate())` 与快照的 `samples_per_tick`
    ///   都按**工程**采样率推进，容器头却按本字段写 —— 两者不等 ⇒ 样本被按错误速率解释
    ///   （音高／时长错），且音频片段会因"素材采样率 ≠ 导出采样率"被整条**跳过**
    ///   （见 [`crate::audio_clip`]）。内置三份夹具工程都是 48 kHz ⇒ 默认产物与旧行为
    ///   **逐字节相同**；`--wav-sample-rate 48000` 可强制回旧口径。
    /// - **时长 8 秒**（[`DEFAULT_WAV_SECONDS`]）⇒ 帧数 = `round(秒 × 采样率)`。
    ///   ⛔ 缺省**不**改成"工程可闻末端"：那不是接线而是**新能力**（导出路径今天没有
    ///   "工程有多长"的唯一权威读数），且会改变既有判据钉住的产物长度。
    /// - **精确帧数**（`--wav-frames`）：与 MCP `render_master` 的 `frames` 同一量纲，
    ///   用来逐字节对齐两侧的产物长度。给了它就**不再**做秒×采样率的换算；
    ///   与 `seconds` 互斥由 CLI 的解析层守卫（⛔ 不在这里靠"谁优先"发明语义）。
    /// - **位深 24-bit 整数**（旧行为）。
    #[must_use]
    pub fn resolve(
        project: &YebanProjectV1,
        sample_rate: Option<SampleRate>,
        bit_depth: Option<BitDepth>,
        seconds: Option<f32>,
        frames: Option<u64>,
    ) -> Self {
        let sample_rate = sample_rate.unwrap_or_else(|| project.sample_rate()).hz();
        // f64 而不是 f32：192 kHz × 长时长时 f32 的尾数不够（32 bit 只到 ~7 位十进制）。
        let frames = frames.unwrap_or_else(|| {
            let seconds = seconds.unwrap_or(DEFAULT_WAV_SECONDS);
            (f64::from(seconds) * f64::from(sample_rate)).round() as u64
        });
        Self {
            frames,
            sample_rate,
            bit_depth: bit_depth.unwrap_or(BitDepth::Int24),
            ..Self::default()
        }
    }
}

/// ⭐ 任务单 `#226`：**最响的源**（照 MCP `warnings[].loudestSource` 的口径）。
///
/// ## 测点口径（与 MCP 逐条同义）
///
/// - `node` = 源节点 = 轨道身份；`track` = `TrackV3.name`（给读数的**人**看的名字）；
/// - `peak` = 该源输出的 `max |sample|`（线性幅度）；`peak_dbfs` = `20·log10(peak)`
///   （测不出 ⇒ `None`，⛔ 不写 `0 dBFS`，那是满量程、语义正好相反）；
/// - `suggested_source_trim_db` = **同一条公式**作用在 `peak_dbfs` 上（只减这一条源需要的量）；
///   `peak_dbfs <= 0` ⇒ `Some(0.0)`（⛔ 不是减一个正数 —— 那会变成**提升**增益）。
///
/// ⚠ **最响的源 ≠ 过载的原因**：多源求和的工程里，最响的**单条源**自己可能根本没到满量程，
/// 削波来自求和。因此 `suggested_source_trim_db == 0.0` 是合法读数，`hint`（见
/// [`WavExportReport::clipping_note`]）会**逐字**说清"单独减它不会解决"。
#[derive(Clone, Debug, PartialEq)]
pub struct WavLoudestSource {
    /// 源节点（= 轨道身份）。
    pub node: EntityId,
    /// 轨道名（`TrackV3.name`；读数里的人话名字）。
    pub track: String,
    /// 该源输出的样本峰值（线性幅度，`max |sample|`）。
    pub peak: f32,
    /// `peak` 的 dBFS；`None` = 测不出（本源没有非零样本）。
    pub peak_dbfs: Option<f32>,
    /// 只把**这一条源**压到 0 dBFS 以下需要的量（dB，负值 = 减）。
    ///
    /// ⚠ 这是**逐源**建议，⛔ 不是母带保证：见 [`WavLoudestSource`] 类型文档。
    pub suggested_source_trim_db: Option<f32>,
}

/// 导出报告（形态照 `MidiExportReport`：路径／帧数／采样率／声道／位深／字节数）。
///
/// ## ⭐ 削波读数与建议 trim（任务单 `#226`；与 MCP 的 `data.*`／`warnings[]` 同名同义）
///
/// | 本结构字段 | MCP 字段 | 单位／语义 |
/// | :--- | :--- | :--- |
/// | [`Self::clipped_samples`] | `clippedSamples` | 会被 24-bit 量化器钳住的**样本数**（`\|s\| > 1.0`） |
/// | [`Self::true_peak_dbfs`] | `truePeakDbfs` | 16× 多相过采样的真峰值（dBFS；`0` = 满量程，正 = 过载） |
/// | [`Self::headroom_db`] | `headroomDb` | `-true_peak_dbfs`（负 = 已过载） |
/// | [`Self::suggested_trim_db`] | `suggestedTrimDb` | 建议的母带 trim（dB，负 = 减；`0.0` = 不用减） |
/// | [`Self::loudest_source`] | `warnings[].loudestSource` | 最响的源（见 [`WavLoudestSource`]） |
///
/// ⚠ **只读**：这些字段在渲染与落盘**之后**从同一份样本上量出来，⛔ 不参与增益、⛔ 不参与
/// 量化 ⇒ 加了读数之后整文件 `sha256` 逐字节不变（判据
/// `reporting_readings_does_not_change_a_single_byte` 用**加读数之前**的常量钉住）。
///
/// ⚠ 为什么 `suggested_trim_db` 是 `f32` 而不是 `Option<f32>`：MCP 的 `suggestedTrimDb`
/// 在"没削波"时是 `0.0`（**数字**），不是 `null`；只有"测不出"才是 `null` 的那两个字段
/// （`truePeakDbfs` / `headroomDb`）才用 `Option`。两侧形状因此一一对应。
///
/// ⚠ `Eq` 被去掉：报告里现在有 `f32` 读数（真峰值／建议 trim），`Eq` 对浮点不成立。
/// 既有判据只用 `PartialEq`（`assert_eq!`）⇒ 语义不变。
#[derive(Clone, Debug, PartialEq)]
pub struct WavExportReport {
    /// 落盘路径。
    pub path: PathBuf,
    /// 总帧数。
    pub frames: u64,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: usize,
    /// 位深（bit）。
    pub bits: u16,
    /// 文件字节数。
    pub bytes: u64,
    /// 渲染输出的位级摘要（十六进制；来自 `RenderOutput::digest`）。
    pub digest: String,
    /// 本次导出用的音源名（如实登记；文件样本会写出路径／帧数／采样率）。
    pub source: String,
    /// 真的进了母带的**音频摆放个数**（`ClipContent::Audio`；0 = 没有音频片段发声）。
    pub audio_clips_rendered: u64,
    /// ⭐ `#249`：循环展开后的 **span 数**（与上面的摆放数分开报；非循环时两者相等）。
    pub audio_clip_spans: u64,
    /// 被**如实跳过**的音频片段原因（例如声道无法映射、坏 WAV、重采样失败）。
    /// ⚠ 素材采样率 ≠ 导出采样率**不再**进这里：`#141` 起离线导出会重采样到导出率。
    pub audio_clips_skipped: Vec<String>,
    /// ⭐ 控制侧 SFZ 解码读数（任务单 `#79`；键 = 轨）。空 = 本次导出没走模型 SFZ 链条
    /// （没有 `TrackV3.sampler`，或走了 `--sfz` 旁路 / 最小音源）。
    ///
    /// ⚠ **缺样本在这里可见**（`missing_sample_paths` / `silent_region_count`）⇒ 报告⛔ 不静默。
    pub sfz_decoded: Vec<(EntityId, crate::sfz_load::SfzDecodeReport)>,
    /// ⭐ 会被 24-bit 量化器钳住的**样本数**（`|sample| > 1.0`；严格大于 —— `±1.0`
    /// 恰好落在满量程端点，不需要钳位）。对象是抖动／量化**之前**的浮点母带。
    pub clipped_samples: u64,
    /// ⭐ 真峰值（dBFS；16× 多相过采样，与 MCP 走**同一个** `measure_master`）。
    ///
    /// `0` = 满量程，正值 = 过载；`None` = 测不出（静音或采样率没有核验系数），
    /// ⛔ **不是** `0 dBFS`。
    pub true_peak_dbfs: Option<f32>,
    /// ⭐ 余量（dB）= `-true_peak_dbfs`（负值 = 已过载）；测不出 ⇒ `None`。
    pub headroom_db: Option<f32>,
    /// ⭐ 建议的**母带** trim（dB；负值 = 减，`0.0` = 不用减）。
    ///
    /// 公式逐字：`suggestedTrimDb = -(ceil(truePeakDbfs * 2) / 2)`
    /// （向上取整到 0.5 dB ⇒ 按它减完必然不再过载，最多多减 0.5 dB）。⛔ 本导出**不自动改**
    /// 增益 —— 那会静默改用户意图。
    pub suggested_trim_db: f32,
    /// ⭐ 公式的输入值（dBFS）与它的来源口径（`"truePeakDbfs"` / `"samplePeakDbfs"`，
    /// 与 MCP 的 `basis` 同义：真峰值测不出时如实退到样本峰值，⛔ 不混成一句）。
    pub trim_basis_dbfs: f32,
    /// ⭐ [`Self::trim_basis_dbfs`] 的来源口径。
    pub trim_basis: &'static str,
    /// ⭐ 最响的源（逐源峰值最大者；并列取 `node` 字典序**最小**的 ⇒ 每次点同一个名字）。
    ///
    /// `None` = 本次没有任何源产生非零样本。
    pub loudest_source: Option<WavLoudestSource>,
}

impl WavExportReport {
    /// 一行可读摘要（CLI 打印它）。
    ///
    /// ⭐ `#226`：**削波时多打一行**（[`Self::clipping_note`]）；⛔ **不削波时逐字不变**
    /// —— 既有的文本判据不会被这一单改动（判据见 `a_clean_project_keeps_the_legacy_summary`）。
    #[must_use]
    pub fn summary(&self) -> String {
        let legacy = format!(
            "WAV 已写出: {}（{} 帧 / {} Hz / {} 声道 / {} bit / {} 字节 / 音源 {} / digest {}）",
            self.path.display(),
            self.frames,
            self.sample_rate,
            self.channels,
            self.bits,
            self.bytes,
            self.source,
            self.digest
        );
        match self.clipping_note() {
            Some(note) => format!("{legacy}\n{note}"),
            None => legacy,
        }
    }

    /// ⭐ `#226`：**削波提示行**（恰好一行；`clipped_samples == 0` ⇒ `None`）。
    ///
    /// 这是 CLI 与控制台唯一的"用户真的看得到"的落点：
    /// - [`Self::summary`] 把它接在既有摘要**之后**（多一行）；
    /// - `--export-wav` 的批处理输出把它作为**额外一行**打印（⛔ 不削波时不打印 ⇒
    ///   既有 stdout 逐字不变）。
    ///
    /// ## 它必须说清的三件事（少一件就是误导）
    ///
    /// 1. **多少样本会被钳**（`clippedSamples`）＋ 真峰值／余量；
    /// 2. **该减多少**（`suggestedTrimDb`，公式逐字进文案）—— 以及这是**母带级**建议；
    /// 3. **最响的源 ≠ 过载的原因**：多源求和时单条源可能自己没过满量程，
    ///    `suggestedSourceTrimDb == 0` ⇒ 单独减它没用（逐字进文案）。
    ///
    /// ⛔ 文案**不**承诺"改完就好听"：它只说"按这个量减完不会钳位"。
    #[must_use]
    pub fn clipping_note(&self) -> Option<String> {
        if self.clipped_samples == 0 {
            return None;
        }
        let true_peak = self
            .true_peak_dbfs
            .map_or_else(|| "null".to_owned(), |value| format!("{value:.6}"));
        let headroom = self
            .headroom_db
            .map_or_else(|| "null".to_owned(), |value| format!("{value:.6}"));
        let loudest = self.loudest_source.as_ref().map_or_else(
            || "最响的源 = null（本次没有任何源产生非零样本；过载来自别处？）".to_owned(),
            |source| {
                let dbfs_text = source
                    .peak_dbfs
                    .map_or_else(|| "null".to_owned(), |value| format!("{value:.6}"));
                // 与 MCP 的 `hint` 同一条判定：按逐源建议是否为 0 区分"自己过载"与"求和过载"。
                let summing = source.suggested_source_trim_db.unwrap_or(0.0) == 0.0;
                let note = if summing {
                    "；⚠ 它**自己**没有过满量程 ⇒ 削波来自**多源求和**，⛔ 单独减这一条不会解决，\
                     请按 suggestedTrimDb 减母带（或逐条配平）"
                } else {
                    "；⚠ 它自己也过了满量程 ⇒ 可以先考虑减它；母带级建议仍以 suggestedTrimDb 为准"
                };
                format!(
                    "最响的源 = `{}`（node `{}`, peakDbfs = {dbfs_text}{note}）",
                    source.track,
                    source.node.to_canonical_string()
                )
            },
        );
        Some(format!(
            "⚠ WAV 母带削波: clippedSamples = {}（这些样本会被 24-bit 量化器钳在 ±1.0）；\
             truePeakDbfs = {true_peak} dBFS / headroomDb = {headroom} dB；\
             建议把母带增益减 {:.1} dB（suggestedTrimDb = -(ceil({} * 2) / 2) = {:.6} dB）；\
             {loudest}；⛔ 本工具**不自动改增益** —— 这里只是读数 + 建议，音频一个字节都没动",
            self.clipped_samples,
            self.suggested_trim_db.abs(),
            self.trim_basis,
            self.suggested_trim_db
        ))
    }
}

/// ⭐ `#226`：削波时建议的 **trim 量**（dB；负值 = 减，`0.0` = 不需要减）。
///
/// ## 公式（逐字）
///
/// ```text
/// suggestedTrimDb = -(ceil(truePeakDbfs * 2) / 2)
/// ```
///
/// 语义与 MCP 的 `render_math::suggested_trim_db` **逐字相同**：把真峰值向上取整到
/// 0.5 dB 的整数倍再取负 ⇒ 按它减完 `truePeakDbfs <= 0`（不会差 0.1 dB 又削一次），
/// 且**最多多减 0.5 dB**。非有限值或 `<= 0` ⇒ `0.0`（⛔ 不返回正数 —— 那会变成提增益）。
///
/// ## 为什么是**复制**而不是引用
///
/// `yeban-app` 的默认档**不依赖** `yeban-mcp`（依赖方向见 `crates/yeban-app/Cargo.toml`
/// 的 `in-process-mcp` 段；MUST-GATE-009 的第一道开关是"默认构建里没有 yeban-mcp"）。
/// 为了复用这 3 行而引一条默认依赖边，会把整个 MCP 依赖图拖进 release 二进制 ——
/// 代价远大于 3 行。⇒ ⭐ **两处同式、各一份 copy**，由判据对账：
/// - 本文件的 `suggested_trim_matches_the_mcp_formula`（公式字面量 ＋ 半 dB 边界）；
/// - 集成判据 `the_readings_match_the_mcp_side_on_the_same_audio`（同一份音频上 CLI 的
///   `suggestedTrimDb` 必须等于 MCP 侧在**同一段样本**上的读数）。
#[must_use]
fn suggested_trim_db(true_peak_dbfs: f32) -> f32 {
    if !true_peak_dbfs.is_finite() || true_peak_dbfs <= 0.0 {
        return 0.0;
    }
    let trimmed = (true_peak_dbfs * 2.0).ceil() / 2.0;
    if trimmed <= 0.0 { 0.0 } else { -trimmed }
}

/// 会被 24-bit 量化器钳住的样本数（`|sample| > 1.0`；口径逐字同 MCP）。
#[must_use]
fn clipped_sample_count(samples: &[f32]) -> u64 {
    samples
        .iter()
        .filter(|sample| sample.is_finite() && sample.abs() > 1.0)
        .count() as u64
}

/// 样本峰值（`max |sample|`；非有限样本忽略）。
#[must_use]
fn sample_peak(samples: &[f32]) -> f32 {
    let mut peak = 0.0_f32;
    for sample in samples {
        if sample.is_finite() {
            peak = peak.max(sample.abs());
        }
    }
    peak
}

/// 真峰值（dBFS）：与 MCP **同一个**测量实现（`yeban-render::mastering::measure_master`，
/// 母带口径 **16×** 多相过采样）。
///
/// 声道布局：`channels >= 2` ⇒ 左 = 声道 0、右 = 声道 1（MCP 的 `MASTER_CHANNELS` 口径）；
/// `channels == 1` ⇒ 同一路喂两个检测器（单声道的真峰值就是它自己）。
/// `measure_master` 拒绝采样率 / 全零 ⇒ `None`（⛔ 不编造 `0 dBFS`）。
#[must_use]
fn measure_true_peak(samples: &[f32], channels: usize, sample_rate: u32) -> Option<f32> {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for frame in 0..frames {
        let base = frame * channels;
        left.push(samples[base]);
        right.push(if channels >= 2 {
            samples[base + 1]
        } else {
            samples[base]
        });
    }
    let master = yeban_render::mastering::measure_master(sample_rate, &left, &right)?;
    master
        .true_peak_dbtp
        .is_finite()
        .then_some(master.true_peak_dbtp)
}

/// 导出失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WavExportError {
    /// 渲染计划编译或执行失败（图不自洽 / 端点缺失 / 选项非法 ……）。
    Render(String),
    /// WAV 写出失败（含"拒绝时不落字节"的那类拒绝）。
    Write(String),
    /// 报告阶段读取文件元数据失败。
    Io(String),
    /// 请求了 0 帧（没有可导出的内容）。
    Empty(String),
    /// 引擎装载失败（快照投影 / `arm_from_snapshot`；例如工程没有主总线 ⇒ `NoMasterBus`）。
    Engine(String),
    /// ⭐ **模型 SFZ 链条解码失败**（任务单 `#79`）：`TrackV3.sampler` 指到的 SFZ 文件
    /// 读不到 / 文本非法。⛔ 不静默回退到复音合成器（那会出一版**不是它**的声音）。
    Sfz(String),
}

impl core::fmt::Display for WavExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Render(reason) => write!(formatter, "渲染失败: {reason}"),
            Self::Write(reason) => write!(formatter, "写出 WAV 失败: {reason}"),
            Self::Io(reason) => write!(formatter, "读取导出结果失败: {reason}"),
            Self::Empty(reason) => write!(formatter, "没有可导出的内容: {reason}"),
            Self::Engine(reason) => write!(formatter, "引擎装载失败: {reason}"),
            Self::Sfz(reason) => write!(formatter, "SFZ 解码失败: {reason}"),
        }
    }
}

impl core::error::Error for WavExportError {}

// ⭐ `ADR-0011`：引擎音源装配错误（`yeban-render` 的公开错误类型）映射到本文件的
// `Engine` 变体 —— 报文逐字保留（`NoMasterBus` 等拒绝原因不能在这里被吞掉）。
impl From<yeban_render::engine_source::EngineSourceError> for WavExportError {
    fn from(error: yeban_render::engine_source::EngineSourceError) -> Self {
        Self::Engine(error.message().to_owned())
    }
}

/// **最小音源**：按音符起止发出廉价方波（首尾各 64 帧线性包络），相位由**整数帧号**派生。
///
/// ⛔ 这不是引擎乐器（见模块文档的诚实登记）；它的职责是"把导出管路打通且**可听**"，
/// 并保持确定性：同一工程 ＋ 同一选项 ⇒ 逐位相同的输出。
///
/// ## ⭐ 引擎层的轨增益门（推子／静音／独奏）
///
/// 与姊妹源 [`crate::sfz_load::SfzTrackSource`] **同形**：本类型**不碰**力度那一半
/// （它自己的 `velocity/127 × 0.2` 是"最小音源"的固有幅度），只把**引擎层的轨增益**
/// 乘在**本源输出之外**（`render_block` 里逐样本）。值由调用方
/// （[`crate::render_core`]）用引擎的**同一个算子**
/// [`yeban_engine::synth::track_gain_scale`] 逐轨算出，再经 [`Self::with_track_gain`]
/// 注入 —— ⛔ 本类型**不**自己写一份 `volume_db ⇒ 线性` 的换算。
///
/// ⚠ 缺省是 **`1.0` ＝ 单位增益**（⛔ 不是 `0.0`：默认不是静音）；`1.0` 是 IEEE 恒等
/// ⇒ 跳过乘法，"调用方没给轨参数"的读数与接线前**逐位相同**。
pub(crate) struct NoteToneSource {
    /// `(绝对起始帧, 结束帧, 2 * freq / sample_rate, 幅度)`。
    notes: Vec<(u64, u64, f64, f32)>,
    /// ⭐ **引擎层的轨增益门**（可闻门 ＝ 静音/独奏 × 推子；默认 **`1.0`＝单位增益**）。
    track_gain: f32,
    /// ⭐ **轨声相**（`(左, 右)`；默认 **`(1.0, 1.0)`＝居中**，`#251`）。
    ///
    /// 值由 [`crate::render_core`] 从 [`crate::audio_clip::track_pan_gains`] 注入 ——
    /// 与音频片段**同一条**等功率曲线，⛔ 本类型不自己算 `cos`/`sin`。
    /// `(1.0, 1.0)` ⇒ 走原来的"单声道铺到所有声道"分支，读数逐位不变。
    pan: (f32, f32),
}

/// 首尾包络长度（帧）：避免方波起止的爆音。
const TONE_FADE_FRAMES: u64 = 64;

// ⭐ tick → 帧的**唯一实现**在 [`yeban_model::time_math::ticks_to_frames`]（`#248`）。
//
// `#222` 起这条换算必须看工程的 `bpm`（旧口径 `tick × sample_rate / 960` 把 BPM
// 钉死为 60 ⇒ 非 60 BPM 的工程音符时间全错）。⚠ 本文件旧副本的求值顺序是
// `tick × 60 / PPQ / bpm × sr`（与 `#247` 之前的 MCP 相同）：在 44.1 kHz 上与引擎
// 差 **118 处一帧**（`bpm=60, tick=168`：旧副本 **7717** vs 模型 **7718**）。
// 本文件因此不再持有第二份算式；病态输入仍按本调用点的既有口径映射成 `0`
// （模型返回 `None`），溢出仍饱和到 `u64::MAX`（模型层保证）。

impl NoteToneSource {
    /// 从一条轨道的全部片段收集音符（绝对帧 = 片段起点 ＋ 音符起点，960 PPQ 换算）。
    pub(crate) fn from_track(
        project: &YebanProjectV1,
        track_id: EntityId,
        sample_rate: u32,
    ) -> Self {
        let mut notes = Vec::new();
        if let Some(track) = project.tracks.get(&track_id) {
            for placement in track.clips.values() {
                let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                    continue;
                };
                let Some(map) = entry.content.notes() else {
                    continue;
                };
                for note in map.values() {
                    // ⭐ `#222`：tick ⇒ 帧**必须看 `bpm`**（旧口径按"960 tick = 1 秒"
                    //    换算 ⇒ 只有 `bpm == 60` 才与引擎／MCP 同解）。
                    let start_tick = placement.start_tick.saturating_add(note.start_tick);
                    let end_tick = start_tick.saturating_add(note.duration_ticks.max(1));
                    let start = yeban_model::time_math::ticks_to_frames(
                        start_tick,
                        project.bpm,
                        yeban_model::PPQ,
                        sample_rate,
                    )
                    .unwrap_or(0);
                    let end = yeban_model::time_math::ticks_to_frames(
                        end_tick,
                        project.bpm,
                        yeban_model::PPQ,
                        sample_rate,
                    )
                    .unwrap_or(0)
                    .max(start.saturating_add(1));
                    // 预先计算频率及 `2 * f / fs`（供渲染循环廉价算相位），以及幅度。
                    // ⭐ `#234`：音高换算走纯 Rust `libm::pow`（⛔ 不是宿主 `f64::powf`）
                    // —— 这是**音频字节路径**，同一工程必须在每个平台渲染出同一串字节
                    // （[ARCH-DET-001]；`#233` 的同一条口径）。
                    let frequency =
                        440.0_f64 * libm::pow(2.0, (f64::from(note.pitch) - 69.0) / 12.0);
                    let phase_step = frequency / (sample_rate as f64) * 2.0;
                    let amp = (f32::from(note.velocity) / 127.0) * 0.2;
                    notes.push((start, end, phase_step, amp));
                }
            }
        }
        notes.sort_unstable_by_key(|n| n.0);
        Self {
            notes,
            // ⭐ 默认**单位增益**（⛔ 不是 `0.0`）—— 与引擎侧
            // `NoteSchedule::from_sorted`／`SfzTrackSource::new` 的默认同口径。
            track_gain: 1.0,
            // ⭐ 默认**居中**（`1.0, 1.0`）：没注入时导出字节与接线前逐位相同（`#251`）。
            pan: (1.0, 1.0),
        }
    }

    /// ⭐ 写入**引擎层的轨增益门**（推子 × 静音/独奏；构造期一次）。
    ///
    /// ⚠ 值必须来自引擎的**同一个算子** [`yeban_engine::synth::track_gain_scale`]
    /// （见字段文档）—— ⛔ 本类型不接受"这里自己换算"的第二份实现。
    /// 与引擎侧 [`yeban_engine::synth::NoteSchedule::with_track_gain`]、以及与姊妹源
    /// [`crate::sfz_load::SfzTrackSource::with_track_gain`] **同名同形**。
    #[must_use]
    pub(crate) const fn with_track_gain(mut self, track_gain: f32) -> Self {
        self.track_gain = track_gain;
        self
    }

    /// ⭐ 本源的**轨增益门**（与力度无关：可闻门 × 推子）。
    ///
    /// ⚠ 这是"注入的那一份标量"，不是"输出里量出来的比例"——判据用它逐位对账引擎投影
    /// （`NoteSchedule::track_gain`）。
    #[must_use]
    pub(crate) const fn track_gain(&self) -> f32 {
        self.track_gain
    }

    /// ⭐ 写入**轨声相**（`(左, 右)`；构造期一次，`#251`）。
    ///
    /// 值必须来自 [`crate::audio_clip::track_pan_gains`]（音频片段路径的**同一个**
    /// 函数）—— ⛔ 本类型不接受"这里自己写一条曲线"。缺省 `(1.0, 1.0)` ⇒
    /// 原来的单声道铺放分支，导出字节逐位不变。
    #[must_use]
    pub(crate) const fn with_pan(mut self, pan: (f32, f32)) -> Self {
        self.pan = pan;
        self
    }

    /// 该帧上所有正在发声的音符之和（`[-1, 1]` 之外的削波交给渲染层的母线）。
    fn sample_at(&self, frame: u64) -> f32 {
        let mut value = 0.0_f32;
        for &(start, end, phase_step, amp) in &self.notes {
            if frame < start || frame >= end {
                continue;
            }
            let offset = frame - start;
            // 通过整数取模判定方波正负半周，免去 `f64 % f64` 的浮点求余与逐帧重算
            // （音高换算的 `pow` 只在 `from_track` 里做一次，见 `#234`）。
            let half_cycles = (offset as f64 * phase_step) as u64;
            let square = if half_cycles.is_multiple_of(2) {
                1.0_f32
            } else {
                -1.0_f32
            };
            let fade_in = offset.min(TONE_FADE_FRAMES) as f32 / TONE_FADE_FRAMES as f32;
            let fade_out = (end - frame).min(TONE_FADE_FRAMES) as f32 / TONE_FADE_FRAMES as f32;
            value += square * fade_in.min(fade_out) * amp;
        }
        value
    }
}

impl AudioSource for NoteToneSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        out.fill(0.0);
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        let track_gain = self.track_gain();
        let (gain_l, gain_r) = self.pan;
        for index in 0..frames {
            let frame = context.first_frame + index as u64;
            let mut value = self.sample_at(frame);
            // ⭐ **引擎层的轨增益门**乘在**本源输出之外**（与 `SfzTrackSource::ensure_rendered`
            //    同形）。`1.0` 是 IEEE 恒等 ⇒ 跳过乘法，"调用方没给轨参数"的读数逐位不变。
            if track_gain != 1.0 {
                value *= track_gain;
            }
            let base = index * channels;
            if gain_l == 1.0 && gain_r == 1.0 {
                for channel in 0..channels {
                    // ⭐ 累加进**已清零**的缓冲（与 `SfzTrackSource`／音频片段同口径）：这样
                    //    `-x * 0.0 = -0.0` 会被 `+0.0` 归一 ⇒ 静音档是**逐位** 0（`to_bits() == 0`），
                    //    而不是 `0x8000_0000`。非零样本上 `0.0 + x == x` 逐位相等。
                    out[base + channel] += value;
                }
            } else {
                // ⭐ `#251`：与引擎乐器源同一个形状（第 0 声道左、第 1 声道右、其余 0）。
                out[base] += value * gain_l;
                if channels > 1 {
                    out[base + 1] += value * gain_r;
                }
            }
        }
        Ok(())
    }
}

/// 一条轨道的音符，按 **960 PPQ ＋ 工程 `bpm`** 换算成
/// `(起始帧, 时值帧, 音高, 力度)`（⛔ 与最小音源同一口径）。
///
/// ⭐ `#222`：旧实现按"960 tick ＝ 1 秒"换算（⛔ 不看 `bpm`），只有 `bpm == 60`
/// 才与 MCP 的 `render_math::note_frame_span` 同解 ⇒ 非 60 BPM 的 `--sfz` 导出
/// 音符时间全错。现在两边走**同一个** [`yeban_model::time_math::ticks_to_frames`] 口径。
pub(crate) fn track_notes_in_frames(
    project: &YebanProjectV1,
    track_id: EntityId,
    sample_rate: u32,
) -> Vec<(u64, u64, u8, u8)> {
    let mut notes = Vec::new();
    if let Some(track) = project.tracks.get(&track_id) {
        for placement in track.clips.values() {
            // 与引擎（`snapshot::project_schedules`）和 MCP 排程同一条口径：
            // 静音的摆放**不发声**（此前这里会把静音摆放也排进去）。
            if placement.muted {
                continue;
            }
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let Some(map) = entry.content.notes() else {
                continue;
            };
            for note in map.values() {
                // 概率触发：判定入口只有模型的 `MidiNote::triggers`（确定性、不掷骰子）。
                if !note.triggers(project.rng_seed) {
                    continue;
                }
                let micro = i64::from(note.micro_timing_ticks.unwrap_or(0));
                let ratchet = u64::from(note.ratchet.unwrap_or(1).clamp(1, 16));
                // 循环展开只由 `yeban-model` 决定；没有循环计划时原样一遍
                // ⇒ 与展开之前逐位相同（ADR-0010 D5）。
                for (copy_start, copy_duration) in
                    placement.note_copies(note.start_tick, note.duration_ticks)
                {
                    let step = (copy_duration / ratchet).max(1);
                    let base_tick = placement.start_tick.saturating_add(copy_start);
                    for pulse in 0..ratchet {
                        let start_tick = base_tick.saturating_add(pulse.saturating_mul(step));
                        // 微时值偏移作用在**起点 tick** 上（与 MCP 的
                        // `render_math::note_frame_span` 同解）；负到越界就夹到 0。
                        let shifted = i128::from(start_tick) + i128::from(micro);
                        let clamped = if shifted <= 0 {
                            0
                        } else {
                            u64::try_from(shifted).unwrap_or(u64::MAX)
                        };
                        let start = yeban_model::time_math::ticks_to_frames(
                            clamped,
                            project.bpm,
                            yeban_model::PPQ,
                            sample_rate,
                        )
                        .unwrap_or(0);
                        let end = yeban_model::time_math::ticks_to_frames(
                            clamped.saturating_add(step),
                            project.bpm,
                            yeban_model::PPQ,
                            sample_rate,
                        )
                        .unwrap_or(0)
                        .max(start.saturating_add(1));
                        notes.push((start, end.saturating_sub(start), note.pitch, note.velocity));
                    }
                }
            }
        }
    }
    notes.sort_unstable();
    notes
}

/// 把工程离线渲染成 WAV（**先全量渲染与量化，最后一步才落盘**）。
///
/// # Errors
///
/// - [`WavExportError::Empty`]：`options.frames == 0`；
/// - [`WavExportError::Render`]：计划编译/执行失败；
/// - [`WavExportError::Write`]：`write_plain_wav` 拒绝（此时**不留任何字节**）。
pub fn export_wav(
    project: &YebanProjectV1,
    path: &Path,
    options: WavExportOptions,
) -> Result<WavExportReport, WavExportError> {
    export_wav_with_sfz(project, path, options, None)
}

/// 与 [`export_wav`] 相同，但可以指定一份**已装载的 SFZ 乐器**（`--sfz <path>` 的接线）。
///
/// 给了 SFZ ⇒ 每条**可渲染**轨用 [`crate::sfz_load::SfzTrackSource`]（SFZ 区域发声，
/// 样本是内存正弦）；⛔ 否则与 [`export_wav`] 逐位相同。
///
/// # Errors
///
/// 与 [`export_wav`] 相同。
pub fn export_wav_with_sfz(
    project: &YebanProjectV1,
    path: &Path,
    options: WavExportOptions,
    sfz: Option<std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>>,
) -> Result<WavExportReport, WavExportError> {
    export_wav_with_sfz_file(project, path, options, sfz.map(|i| (i, None)))
}

/// 与 [`export_wav_with_sfz`] 相同，但可同时给出 **SFZ 文件路径** ⇒ `sample=` 会被**真读**。
///
/// 给了路径 ⇒ 每条音符按它的区域解析 `sample=`（**只支持 `read_plain_wav` 能读的 PCM WAV**）；
/// 读不到 ⇒ 如实回退到内存正弦并把原因写进 `source=`。⛔ 给 `None` 时与
/// [`export_wav_with_sfz`] 逐位相同（内存正弦）。
///
/// # Errors
///
/// 与 [`export_wav`] 相同。
pub fn export_wav_with_sfz_file(
    project: &YebanProjectV1,
    path: &Path,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
) -> Result<WavExportReport, WavExportError> {
    export_wav_with_sfz_file_and_assets(project, path, options, sfz, &[])
}

/// 与 [`export_wav_with_sfz_file`] 相同，但**带上归档资产池** ⇒ `ClipContent::Audio`
/// （工程里摆放的音频片段）真的被渲染。
///
/// `assets` 的形状就是 `ProjectArchive.assets`（`&[(AssetHash, Vec<u8>)]`）。给空切片时
/// 与 [`export_wav_with_sfz_file`] 逐位相同（音频片段被**如实跳过**并计数，⛔ 不是静默）。
///
/// # Errors
///
/// 与 [`export_wav`] 相同。
pub fn export_wav_with_sfz_file_and_assets(
    project: &YebanProjectV1,
    path: &Path,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
    assets: &[(yeban_model::AssetHash, Vec<u8>)],
) -> Result<WavExportReport, WavExportError> {
    export_wav_with_sfz_file_and_assets_under(project, path, options, sfz, assets, None)
}

/// 与 [`export_wav_with_sfz_file_and_assets`] 相同，但**知道工程文件所在目录**
/// （任务单 `#168`：相对 SFZ 路径的第一个候选根）。
///
/// `project_dir = None` ⇒ 与 [`export_wav_with_sfz_file_and_assets`] **逐位相同**。
/// 生产 `--export-wav` 传 `--open` 那个文件的父目录。
///
/// # Errors
///
/// 与 [`export_wav`] 相同，外加"相对路径在全部候选根下都找不到"时的
/// [`WavExportError::Sfz`]（原文逐字带"试过：⟨根1⟩／⟨根2⟩／⟨根3⟩"）。
pub fn export_wav_with_sfz_file_and_assets_under(
    project: &YebanProjectV1,
    path: &Path,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
    assets: &[(yeban_model::AssetHash, Vec<u8>)],
    project_dir: Option<&Path>,
) -> Result<WavExportReport, WavExportError> {
    if options.frames == 0 {
        return Err(WavExportError::Empty("请求的帧数为 0".to_owned()));
    }
    // ⭐ **一条真源**：导出与录音都走 `render_core::render_project_f32_*_under`
    // （慢路径校验 → 计划编译 → 音源装配 → 执行 → digest 全在那里）。
    // ⭐ `#170`：录制那一侧与这里**同一个** `project_dir` 语义（工程文件的父目录）。
    let rendered = crate::render_core::render_project_f32_with_assets_under(
        project,
        options,
        sfz,
        assets,
        project_dir,
    )?;
    let (samples, frames, source_label, digest) = (
        rendered.samples,
        rendered.frames,
        rendered.source_label,
        rendered.digest,
    );
    // 本函数只负责**报告**：量化 ＋ 一次落盘走 [`write_interleaved_wav`]（唯一实现）。
    // ⭐ `#226`：削波读数（**只读**：在抖动／量化**之前**的浮点母带上量，⛔ 不改一个样本）。
    //    口径与 MCP 的 `data.clippedSamples` / `truePeakDbfs` / `headroomDb` 逐条相同
    //    （真峰值走**同一个** `measure_master`；`clipped_sample_count` 是同一判据）。
    let clipped_samples = clipped_sample_count(&samples);
    let (true_peak_dbfs, headroom_db) =
        match measure_true_peak(&samples, options.channels, options.sample_rate) {
            Some(peak) => (Some(peak), Some(-peak)),
            None => (None, None),
        };
    // 建议值的输入：真峰值可测就用它；测不出就如实退到样本峰值（`basis` 逐字说明）。
    let trim_basis = if true_peak_dbfs.is_some() {
        "truePeakDbfs"
    } else {
        "samplePeakDbfs"
    };
    let trim_basis_dbfs = true_peak_dbfs.unwrap_or_else(|| dbfs(sample_peak(&samples)));
    let suggested_trim = suggested_trim_db(trim_basis_dbfs);
    // ⭐ 最响的源：并列时取 `node` 字典序**最小**的（与 MCP 的 `max_by` 同一条 tie-break）。
    let loudest_source = rendered
        .source_peaks
        .iter()
        .filter(|(_, peak)| *peak > 0.0)
        .max_by(|(left_node, left_peak), (right_node, right_peak)| {
            left_peak.total_cmp(right_peak).then_with(|| {
                right_node
                    .to_canonical_string()
                    .cmp(&left_node.to_canonical_string())
            })
        })
        .and_then(|(track_id, peak)| {
            let track = project.tracks.get(track_id)?;
            let peak_dbfs = Some(dbfs(*peak));
            Some(WavLoudestSource {
                node: *track_id,
                track: track.name.clone(),
                peak: *peak,
                peak_dbfs,
                suggested_source_trim_db: peak_dbfs.map(suggested_trim_db),
            })
        });
    let (bytes, bits) = write_interleaved_wav(
        path,
        &samples,
        u16::try_from(options.channels).unwrap_or(u16::MAX),
        options.sample_rate,
        options.seed,
        options.bit_depth,
    )?;
    Ok(WavExportReport {
        path: path.to_path_buf(),
        frames,
        sample_rate: options.sample_rate,
        channels: options.channels,
        bits,
        bytes,
        digest,
        source: source_label,
        audio_clips_rendered: rendered.audio_clips_rendered,
        audio_clip_spans: rendered.audio_clip_spans,
        audio_clips_skipped: rendered.audio_clips_skipped,
        sfz_decoded: rendered.sfz_decoded,
        clipped_samples,
        true_peak_dbfs,
        headroom_db,
        suggested_trim_db: suggested_trim,
        trim_basis_dbfs,
        trim_basis,
        loudest_source,
    })
}

/// ⭐ **把交错 `f32` 量化并落成一个 WAV**（`[ARCH-SEC-004]` 的"拒绝 ⇒ 不留半成品"
/// 由 `write_plain_wav` 保证：它先校验再建文件）。
///
/// ⚠ ⭐ 为什么它必须是一个**共享**函数（⛔ 不是复制一份）：`--export-wav` 与
/// `--midi-in-render` 都要把"内存里的 f32"变成"同一个格式的抖动 PCM"。
/// 两份实现迟早会漂移（位深 / 抖动种子 / 量化取整任一处不同，产物就不再可比）——
/// 那正是"同一份渲染、两种字节"这类最难查的缺陷。⇒ ⭐ **唯一实现**在这里。
///
/// 量化与容器格式**都不在本函数里选择**：位深由调用方给的 `bit_depth` 决定
/// （`quantize` 按它选抖动器，`format_of` 按缓冲反推容器字段 ⇒ 两者不可能不一致），
/// 容器字段的合法性由 `write_plain_wav` 的 `check_match` 兜底。
///
/// 返回 `(文件字节数, 位深)`：字节数取自**落盘后的 `metadata`**（⛔ 不是推算值）。
///
/// # Errors
///
/// [`WavExportError::Write`]（WAV 头/缓冲被拒）或 [`WavExportError::Io`]（读回尺寸失败）。
pub fn write_interleaved_wav(
    path: &Path,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    seed: u64,
    bit_depth: BitDepth,
) -> Result<(u64, u16), WavExportError> {
    let mut rng =
        DeterministicDitherRng::new(u32::try_from(seed & u64::from(u32::MAX)).unwrap_or(1));
    let buffer = quantize(samples, bit_depth, &mut rng);
    let format = format_of(&buffer, channels.max(1), sample_rate);
    write_plain_wav(path, &format, &buffer)
        .map_err(|error| WavExportError::Write(error.to_string()))?;
    let bytes = std::fs::metadata(path)
        .map_err(|error| WavExportError::Io(error.to_string()))?
        .len();
    Ok((bytes, format.bits_per_sample))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⭐ `#226`：建议 trim 公式与 MCP 的 `render_math::suggested_trim_db` **逐字同式**
    /// （本 crate 复制了那 3 行 —— 理由见 [`suggested_trim_db`] 的文档）。
    ///
    /// 钉三件事：① 公式的边界（含 `#224` 的实测值 `1.874512 → −2.0` 与"四舍五入会不够"
    /// 的 `5.46`）；② 减完 `truePeak + trim <= 0`；③ 最多多减 0.5 dB。
    #[test]
    fn suggested_trim_matches_the_mcp_formula() {
        // `(truePeakDbfs, 期望 trim)`；期望值全部手算自 `-(ceil(x * 2) / 2)`。
        let cases: [(f32, f32); 8] = [
            (1.874_512, -2.0), // #224 实测：1.874512 × 2 = 3.749024 ⇒ ceil = 4 ⇒ -2.0
            (0.2, -0.5),
            (5.44, -5.5),
            (5.46, -5.5), // 四舍五入会给 −5.5、ceil 也给 −5.5；关键是下面的 5.4
            (5.4, -5.5),
            (0.0, 0.0),
            (-3.0, 0.0), // 负值（不削波）⛔ 不能变成"提增益"
            (f32::NAN, 0.0),
        ];
        for (peak, expected) in cases {
            assert_eq!(
                suggested_trim_db(peak),
                expected,
                "公式必须逐字相同: peak = {peak}"
            );
        }
        // 两个保证（对一批有限正值逐一验算）。
        for peak in [
            0.01_f32, 0.2, 0.5, 1.0, 1.874_512, 2.907_095, 5.46, 23.886_497,
        ] {
            let trim = suggested_trim_db(peak);
            assert!(
                peak + trim <= 1.0e-6,
                "按建议减完不得再削波: peak = {peak}, trim = {trim}"
            );
            assert!(
                peak + trim >= -0.5 - 1.0e-6,
                "最多多减 0.5 dB: peak = {peak}, trim = {trim}"
            );
            assert_eq!(trim, -((peak * 2.0).ceil() / 2.0));
        }
        // 非有限值不制造正数（那是提增益）。
        assert_eq!(suggested_trim_db(f32::INFINITY), 0.0);
        assert_eq!(suggested_trim_db(f32::NEG_INFINITY), 0.0);
    }

    /// ⭐ `#226`：`clippedSamples` 的口径 = `|sample| > 1.0`（**严格大于**），
    /// 与 MCP 的 `clipped_sample_count` 逐条相同；`±1.0` 恰好在满量程端点，不算削波。
    #[test]
    fn clipped_sample_count_is_strictly_greater_than_full_scale() {
        let samples = [
            0.0_f32,
            1.0,
            -1.0,
            1.000_001,
            -1.000_001,
            f32::NAN,
            f32::INFINITY,
        ];
        assert_eq!(clipped_sample_count(&samples), 2, "只有 |s| 严格 > 1.0 算");
        assert_eq!(clipped_sample_count(&[]), 0);
        assert_eq!(clipped_sample_count(&[0.999_999_f32]), 0);
    }

    /// 判据 a（**真的换了音源**）：引擎乐器路径非静音，且与最小音源路径**逐字节不同**。
    ///
    /// ⚠ 为什么断言"逐字节不同"而不是"更好听"：这是**可机械判定**的判别量；
    /// 两个不同音源在同一工程上产出同一串 24-bit 抖动样本的概率可忽略（若真相同，判据会红并要求解释）。
    #[test]
    fn the_engine_source_is_not_silent_and_differs_from_the_fallback_tone() {
        let project = yeban_model::samples::filled_project();
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-engine-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let engine_path = dir.join("engine.wav");
        let tone_path = dir.join("tone.wav");

        let engine = export_wav(&project, &engine_path, WavExportOptions::default())
            .expect("引擎乐器导出必须成功");
        let fallback = export_wav(
            &project,
            &tone_path,
            WavExportOptions {
                use_engine_source: false,
                ..WavExportOptions::default()
            },
        )
        .expect("回退音源导出必须成功");

        assert!(
            engine.source.contains("引擎乐器"),
            "报告必须明说音源: {}",
            engine.source
        );
        assert!(
            fallback.source.contains("最小音源"),
            "报告必须明说回退: {}",
            fallback.source
        );

        let (_, engine_buffer) =
            yeban_render::wav::read_plain_wav(&engine_path).expect("引擎版可回读");
        let engine_nonzero = match &engine_buffer {
            yeban_render::dither::PcmBuffer::Int24(samples) => {
                samples.iter().filter(|sample| **sample != 0).count()
            }
            other => panic!("默认导出必须是 24-bit 整数: {other:?}"),
        };
        assert!(
            engine_nonzero > 0,
            "引擎乐器路径必须非静音（非零样本数 = {engine_nonzero}）"
        );
        assert_ne!(
            std::fs::read(&engine_path).expect("读引擎版"),
            std::fs::read(&tone_path).expect("读回退版"),
            "引擎乐器与最小音源必须产出不同的字节（否则本判据没有判别力）"
        );
        println!(
            "[wav-engine] 引擎版非零样本 {engine_nonzero}；两版字节数 {} vs {}",
            engine.bytes, fallback.bytes
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// ⭐⭐ 任务单 `#209` 的**回归判据**：离线源必须**每块推进**引擎播放头。
    ///
    /// # 量什么（先说再跑）
    ///
    /// 演示工程第一条有声轨的**第二**个音符起于 tick 480（48 kHz / 120 BPM ⇒ 第
    /// 12000 帧）。逐块（512 帧）渲染 **30** 块 ⇒ 引擎的 `notes_triggered` 必须
    /// **≥ 2**、`position` 必须等于 30 × 512 = **15360**。
    ///
    /// # 怎么变红
    ///
    /// 删掉 `render_block` 里的 `self.engine.advance(frames)` ⇒ 每块都从位置 0
    /// 渲染 ⇒ `position == 0`、只有 tick 0 的 C4 触发一次 ⇒ 两个断言同时红
    /// （`#207` 的 `--export-wav ⇒ distinct=1` 正是这个形状）。
    #[test]
    fn the_offline_source_advances_the_engine_so_later_notes_fire() {
        let project = yeban_model::samples::demo_project();
        let snapshot = std::sync::Arc::new(
            EngineSnapshot::from_project(&project, 1).expect("演示工程必须成快照"),
        );
        let track = snapshot
            .schedules()
            .iter()
            .find(|(_, schedule)| !schedule.notes().is_empty())
            .map(|(track, _)| *track)
            .expect("演示工程必须至少有一条有声轨");
        let mut source = EngineTrackSource::new(snapshot, track).expect("必须能武装");

        const FRAMES: usize = 512;
        const BLOCKS: usize = 30;
        let mut out = vec![0.0_f32; FRAMES];
        for index in 0..BLOCKS {
            source
                .render_block(
                    BlockContext {
                        block_index: index as u64,
                        first_frame: (index * FRAMES) as u64,
                        frames: FRAMES,
                        channels: 1,
                        sample_rate: 48_000,
                        seed: 1,
                    },
                    &mut out,
                )
                .expect("离线块渲染不得失败");
        }
        println!(
            "[wav-advance] blocks={BLOCKS} frames={} position={} notes_triggered={}",
            BLOCKS * FRAMES,
            source.position(),
            source.notes_triggered()
        );
        assert_eq!(
            source.position(),
            (BLOCKS * FRAMES) as u64,
            "⭐ 每块必须推进 {FRAMES} 帧（漏掉 advance ⇒ 位置停在 0）"
        );
        assert!(
            source.notes_triggered() >= 2,
            "⭐ 渲染越过第二个音符的起点（第 12000 帧）后必须至少触发 2 个音符；\
             实测 {}（漏掉 advance ⇒ 只有 tick 0 的 C4）",
            source.notes_triggered()
        );
    }

    /// 判据（`sample=` **真读文件**）：把一条**常量**样本（`0.25` 的 24-bit PCM）当区域样本，
    /// 导出的声音**确实来自该文件** —— 判定量 = 该音符前 2048 个样本的**极差**（max−min）：
    /// 常量样本 ⇒ 极差**极小**；内存正弦 ⇒ 极差**大**。
    ///
    /// ⚠ 这样命名判定量，是为了同时排除"换了但没声音"（极差为 0 且全 0 ⇒ 另断言峰值 > 0）
    /// 与"其实还是正弦"（极差会大）两种假象。
    #[test]
    fn a_sample_file_named_by_the_region_is_really_read_and_drives_the_output() {
        let project = yeban_model::samples::filled_project();
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-sample-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        // ① 造一条**常量** 24-bit 单声道样本（值 = 满量程的 1/4 ⇒ 归一化 0.25）
        let scale = yeban_render::dither::BitDepth::Int24.full_scale_max() / 4;
        let sample_path = dir.join("const.wav");
        let buffer = yeban_render::dither::PcmBuffer::Int24(vec![scale; 4_800]);
        let format = yeban_render::rf64::PcmFormat::integer(1, 48_000, 24);
        yeban_render::wav::write_plain_wav(&sample_path, &format, &buffer).expect("写样本");
        // ② SFZ：每个音高一条区域，sample 指向该文件
        let mut pitches: Vec<u8> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values().map(|note| note.pitch))
            .collect();
        pitches.sort_unstable();
        pitches.dedup();
        let text: String = pitches
            .iter()
            .map(|pitch| format!("<region> sample=const.wav key={pitch}\n"))
            .collect();
        let sfz_path = dir.join("keys.sfz");
        std::fs::write(&sfz_path, &text).expect("写 SFZ");
        let instrument = yeban_engine::sfz_instrument::SfzInstrument::load(
            text,
            &yeban_sfz::parser::ParseLimits::default(),
        )
        .expect("装载");
        // ③ 导两次：带文件样本 vs 只有内存正弦
        let file_path = dir.join("file.wav");
        let memory_path = dir.join("memory.wav");
        let with_file = export_wav_with_sfz_file(
            &project,
            &file_path,
            WavExportOptions::default(),
            Some((
                std::sync::Arc::new(instrument.clone()),
                Some(sfz_path.clone()),
            )),
        )
        .expect("带文件样本的导出");
        let with_memory = export_wav_with_sfz(
            &project,
            &memory_path,
            WavExportOptions::default(),
            Some(std::sync::Arc::new(instrument)),
        )
        .expect("内存正弦的导出");
        println!(
            "[wav-file-sample] file source={} / memory source={}",
            with_file.source, with_memory.source
        );
        assert!(
            with_file.source.contains("file("),
            "来源必须写出文件路径与帧数: {}",
            with_file.source
        );
        assert!(
            with_memory.source.contains("in-memory-sine"),
            "内存版来源必须明说: {}",
            with_memory.source
        );

        let spread_of = |path: &std::path::Path| -> (f32, f32) {
            let (format, buffer) = yeban_render::wav::read_plain_wav(path).expect("可回读");
            let channels = usize::from(format.channels.max(1));
            let samples: Vec<f32> = match &buffer {
                yeban_render::dither::PcmBuffer::Int24(values) => values
                    .iter()
                    .step_by(channels)
                    .map(|value| (*value as f32) / 8_388_608.0)
                    .collect(),
                other => panic!("24-bit 整数: {other:?}"),
            };
            let window = &samples[..samples.len().min(2048)];
            let peak = window
                .iter()
                .fold(0.0_f32, |acc, value| acc.max(value.abs()));
            let (min, max) = window.iter().fold((f32::MAX, f32::MIN), |(lo, hi), value| {
                (lo.min(*value), hi.max(*value))
            });
            (max - min, peak)
        };
        let (file_spread, file_peak) = spread_of(&file_path);
        let (memory_spread, _) = spread_of(&memory_path);
        println!(
            "[wav-file-sample] 文件样本极差 {file_spread:.6}（峰值 {file_peak:.6}）ｖｓ 内存正弦极差 {memory_spread:.6}"
        );
        assert!(file_peak > 0.0, "文件样本版不许是静音（峰值 {file_peak}）");
        assert!(
            file_spread < 0.01,
            "常量样本 ⇒ 前 2048 样本极差必须极小（实际 {file_spread}）"
        );
        assert!(
            memory_spread > file_spread * 10.0,
            "内存正弦的极差必须显著更大（正弦 {memory_spread} ｖｓ 常量 {file_spread}）"
        );
        assert_ne!(
            std::fs::read(&file_path).expect("读文件版"),
            std::fs::read(&memory_path).expect("读内存版"),
            "文件样本与内存正弦必须逐字节不同"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据（`--sfz` 接线）：**同一工程**带 SFZ 与不带 SFZ 导出的 WAV **逐字节不同**，
    /// 且 SFZ 版**非静音**（⛔ 不是"换了但没声音"）。
    ///
    /// ⚠ 为什么这份 SFZ 按**工程里实际出现的音高**逐键写区域：区域不匹配就会是静音，
    /// 那样"可区分"就变成"一个响一个哑"，测不出"真的用 SFZ 发声"（本机刻意这样构造）。
    #[test]
    fn an_sfz_export_differs_from_the_default_export_and_is_not_silent() {
        let project = yeban_model::samples::filled_project();
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-sfz-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        // 工程里的每个音高一条区域 ⇒ 保证 SFZ 真的会发声。
        let mut pitches: Vec<u8> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values().map(|note| note.pitch))
            .collect();
        pitches.sort_unstable();
        pitches.dedup();
        assert!(!pitches.is_empty(), "夹具必须有音符");
        let text: String = pitches
            .iter()
            .map(|pitch| format!("<region> sample=a.wav key={pitch}\n"))
            .collect();
        let instrument = yeban_engine::sfz_instrument::SfzInstrument::load(
            text,
            &yeban_sfz::parser::ParseLimits::default(),
        )
        .expect("夹具 SFZ 必须可装载");
        assert_eq!(instrument.region_count(), pitches.len(), "区域数 == 音高数");

        let plain_path = dir.join("plain.wav");
        let sfz_path = dir.join("sfz.wav");
        let plain =
            export_wav(&project, &plain_path, WavExportOptions::default()).expect("默认导出");
        let with_sfz = export_wav_with_sfz(
            &project,
            &sfz_path,
            WavExportOptions::default(),
            Some(std::sync::Arc::new(instrument)),
        )
        .expect("带 SFZ 导出");
        println!(
            "[wav-sfz] 默认 source={} / 字节 {}；SFZ source={} / 字节 {}",
            plain.source, plain.bytes, with_sfz.source, with_sfz.bytes
        );
        assert!(
            with_sfz.source.contains("sfz"),
            "报告必须明说走了 SFZ: {}",
            with_sfz.source
        );
        assert!(
            with_sfz.source.contains("in-memory-sine"),
            "必须明说样本来源（内存正弦）: {}",
            with_sfz.source
        );
        let (_, buffer) = yeban_render::wav::read_plain_wav(&sfz_path).expect("可回读");
        let nonzero = match &buffer {
            yeban_render::dither::PcmBuffer::Int24(samples) => {
                samples.iter().filter(|sample| **sample != 0).count()
            }
            other => panic!("24-bit 整数: {other:?}"),
        };
        assert!(nonzero > 0, "SFZ 版必须非静音（非零样本 {nonzero}）");
        assert_ne!(
            std::fs::read(&plain_path).expect("读默认版"),
            std::fs::read(&sfz_path).expect("读 SFZ 版"),
            "带 SFZ 与不带 SFZ 必须逐字节不同（否则接线没生效）"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 b（⭐ **语义已按引擎侧修复更新**）：主总线**不在轨道表**里、但工程**仍有可武装的轨道**
    /// ⇒ 导出**应当成功**（⛔ 不再期望 `NoMasterBus`）。
    ///
    /// ## 为什么这条判据的期望被反转过（第 617 轮更正）
    ///
    /// 我最初把它写成"主总线不在 `tracks()` ⇒ `NoMasterBus` ⇒ 失败"。`mod-engine` 随后用一个
    /// 最小夹具发现 `arm_from_snapshot` 的 master 守卫**与 `rt.rs` 的语义不一致**：实时侧只把 master
    /// 当**过滤器**（`id != master`），所以"要求 master 出现在 `tracks()` 里"会**误拒一个合法的最小工程**；
    /// 它已改成"只拒绝**没有可武装的轨/音色/鼓**的快照"（`6b0ef4f9`）。
    /// ⇒ 本判据当时断言的是**被修掉的错误行为**，**它变红正是修复生效的证据**；现在把期望改成新语义。
    #[test]
    fn a_project_whose_master_is_absent_from_the_track_table_still_renders() {
        let mut project = yeban_model::samples::filled_project();
        // 路由图里保留主总线节点（计划仍可编译），只把主总线**轨道**从轨道表里删掉。
        project.tracks.remove(&project.master_bus_track_id);
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-nomaster-track-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let target = dir.join("still-renders.wav");

        let report = export_wav(&project, &target, WavExportOptions::default())
            .expect("仍有可武装的轨道 ⇒ 必须成功（新语义）");
        assert!(
            report.source.contains("引擎乐器"),
            "音源: {}",
            report.source
        );
        assert!(target.exists(), "成功 ⇒ 文件必须存在");
        let (_, buffer) = yeban_render::wav::read_plain_wav(&target).expect("可回读");
        let nonzero = match &buffer {
            yeban_render::dither::PcmBuffer::Int24(samples) => {
                samples.iter().filter(|sample| **sample != 0).count()
            }
            other => panic!("默认导出必须是 24-bit 整数: {other:?}"),
        };
        assert!(
            nonzero > 0,
            "仍有可武装的轨道 ⇒ 不许是静音（非零样本 {nonzero}）"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 c（**失败路径的正确触发条件** ⇒ 失败且不留半成品）：工程里**没有可武装的轨/音色/鼓**
    /// 时，`arm_from_snapshot` 返回 `NoMasterBus`，导出在**落盘之前**失败，目标文件**不存在**。
    ///
    /// ⚠ **重叠声明**：`mod-engine` 的夹具已经断言过 `NoMasterBus` **返回值**本身；
    /// 本判据保留的理由是它覆盖的是**导出这一侧的后果**（`WavExportError::Engine` ＋ **一个字节都不落盘**），
    /// ⛔ 不是重复断言返回值。
    #[test]
    fn a_project_with_nothing_armable_reports_no_master_bus_and_writes_nothing() {
        let mut project = yeban_model::samples::filled_project();
        // 清空**全部**轨道（含主总线）：快照里没有任何可武装的轨/音色/鼓。
        // ⚠ 实测更正：只留主总线**不够** —— 引擎的守卫把主总线自己也算"可武装"，
        // 于是它不再拒绝（随后才在计划执行时因缺源而失败）⇒ 触发条件必须是"一个可武装项都没有"。
        // 路由图保持原样 ⇒ 计划仍可编译（⛔ 不靠改坏图来制造失败）。
        project.tracks.clear();
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-nothing-armable-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let target = dir.join("never.wav");

        let error = export_wav(&project, &target, WavExportOptions::default())
            .expect_err("没有可武装的轨 ⇒ 必须失败");
        assert!(
            matches!(error, WavExportError::Engine(_)),
            "必须是引擎装载失败（NoMasterBus）: {error:?}"
        );
        assert!(!target.exists(), "失败 ⇒ 一个字节都不许写出去");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// ⭐ 两个**行为裁决**的钉子（本切片的选择面）——⛔ 不是"跑通"判据，是"口径不许漂移"判据。
    ///
    /// ① **采样率缺省跟随工程**：工程 48 kHz ⇒ `resolve` 与 `default()` **逐字段相同**
    ///    （⇒ 内置三份夹具的产物与旧行为逐字节一致）；工程 44.1 kHz ⇒ 跟随成 44.1 kHz
    ///    （⛔ 不是固定 48000），且**时长**仍是 8 秒（帧数 = 8 × 采样率，⛔ 不是 384000）。
    /// ② **时长缺省 8 秒**：⛔ 缺省**不**改成"工程可闻末端"。
    /// ③ 位深缺省 24-bit 整数；给了就用给了的。
    #[test]
    fn resolve_follows_the_project_rate_and_keeps_the_eight_second_default() {
        use yeban_model::project::SampleRate;

        // ① 48 kHz 工程（内置夹具的采样率）⇒ 与 `default()` 逐字段相同。
        let project_48k = yeban_model::samples::demo_project();
        assert_eq!(project_48k.sample_rate(), SampleRate::Hz48000);
        assert_eq!(
            WavExportOptions::resolve(&project_48k, None, None, None, None),
            WavExportOptions::default(),
            "48 kHz 工程 + 全缺省 必须与旧行为逐字段相同"
        );

        // ② 44.1 kHz 工程 ⇒ 跟随工程，且**时长仍是 8 秒**（352800 帧，⛔ 不是 384000）。
        let mut project_44k = project_48k.clone();
        project_44k.audio_config.sample_rate = SampleRate::Hz44100;
        let followed = WavExportOptions::resolve(&project_44k, None, None, None, None);
        assert_eq!(followed.sample_rate, 44_100, "缺省采样率必须跟随工程");
        assert_eq!(
            followed.frames,
            44_100 * 8,
            "缺省时长仍是 8 秒（不是固定帧数）"
        );
        assert_ne!(followed.frames, 48_000 * 8, "⛔ 不是把旧帧数原样搬过来");

        // ③ 显式覆盖 ⇒ 三条各自生效。
        let chosen = WavExportOptions::resolve(
            &project_44k,
            Some(SampleRate::Hz96000),
            Some(BitDepth::Int16),
            Some(1.5),
            None,
        );
        assert_eq!(chosen.sample_rate, 96_000);
        assert_eq!(chosen.bit_depth, BitDepth::Int16);
        assert_eq!(chosen.frames, 144_000, "1.5 秒 @ 96 kHz");
        // 未覆盖的字段保持旧缺省（声道 / 种子 / 音源）。
        assert_eq!(chosen.channels, 2);
        assert_eq!(chosen.seed, 1);
        assert!(chosen.use_engine_source);

        // ④ 32f 与 24-bit 都能表达（下层 `quantize` 接受三者）。
        assert_eq!(
            WavExportOptions::resolve(&project_48k, None, Some(BitDepth::Float32), None, None)
                .bit_depth,
            BitDepth::Float32
        );
        assert_eq!(
            WavExportOptions::resolve(&project_48k, None, None, None, None).bit_depth,
            BitDepth::Int24,
            "缺省位深仍是 24-bit 整数"
        );

        // ⑤ 精确帧数原样生效，⛔ 不再按秒×采样率换算（MCP `render_master.frames` 的量纲）。
        let exact = WavExportOptions::resolve(&project_48k, None, None, Some(1.5), Some(12_345));
        assert_eq!(exact.frames, 12_345, "给了帧数就原样用");
        assert_eq!(exact.sample_rate, 48_000, "帧数覆盖不影响采样率");
    }

    /// ⭐ `#248`：最小音源（`--export-wav` 的**回退**路径）取模型层的 tick → 帧，
    /// ⛔ 不再用本地副本的 `tick × 60 / PPQ / bpm × sr` 求值顺序。
    ///
    /// `bpm=60, tick=168` 在 44.1 kHz 上恰好是 `.5` 帧：模型 **7718**（半值远离零），
    /// 旧副本 **7717**（先除后乘在浮点上得 `7717.4999999999995`）。48 kHz 上两种顺序
    /// 都是 **8400** ⇒ 同一判据同时钉住"该变的变了、不该变的没变"。
    ///
    /// 这条测试的牙齿：把 `NoteToneSource::from_track` 或 `track_notes_in_frames`
    /// 换回任何"先算秒数再乘采样率"的本地算式，44.1 kHz 行立刻变红。
    #[test]
    fn the_fallback_tone_source_uses_the_model_tick_grid() {
        let mut project = yeban_model::samples::filled_project();
        project.bpm = 60.0;
        let (track_id, placement_id) = project
            .tracks
            .iter()
            .flat_map(|(track_id, track)| {
                track
                    .clips
                    .iter()
                    .map(move |(placement_id, placement)| (*track_id, *placement_id, placement))
            })
            .find(|(_, _, placement)| {
                placement.start_tick == 0 && placement.duration_ticks == 3_840
            })
            .map(|(track_id, placement_id, _)| (track_id, placement_id))
            .expect("规范工程里有一条 3840 tick 的 MIDI 摆放");
        project
            .tracks
            .get_mut(&track_id)
            .expect("轨存在")
            .clips
            .get_mut(&placement_id)
            .expect("摆放存在")
            .start_tick = 168;

        let tone = NoteToneSource::from_track(&project, track_id, 44_100);
        assert_eq!(
            tone.notes.first().expect("至少一条音符").0,
            7_718,
            "44.1 kHz：模型层给 7718（旧副本给 7717）"
        );
        let frames = track_notes_in_frames(&project, track_id, 44_100);
        assert_eq!(frames.first().expect("至少一条音符").0, 7_718);

        // 正对照：48 kHz 上模型与旧副本都是 8400（这条路径的字节不该因收口而变）。
        let tone_48k = NoteToneSource::from_track(&project, track_id, 48_000);
        assert_eq!(tone_48k.notes.first().expect("至少一条音符").0, 8_400);
        let frames_48k = track_notes_in_frames(&project, track_id, 48_000);
        assert_eq!(frames_48k.first().expect("至少一条音符").0, 8_400);
    }

    /// 把夹具收窄成"一条轨、一个 MIDI 摆放、一颗音符 `start=480, pitch=60, dur=480`"，
    /// 供排程展开的判据共用（120 BPM、960 PPQ、48 kHz ⇒ 1 tick = 25 帧）。
    fn one_note_fixture() -> (YebanProjectV1, EntityId, EntityId, EntityId, EntityId) {
        use yeban_model::music::MidiNote;
        use yeban_model::project::ClipContent;

        let mut project = yeban_model::samples::filled_project();
        project.bpm = 120.0;
        let (track_id, placement_id) = project
            .tracks
            .iter()
            .flat_map(|(track_id, track)| {
                track
                    .clips
                    .iter()
                    .map(move |(placement_id, placement)| (*track_id, *placement_id, placement))
            })
            .find(|(_, _, placement)| {
                placement.start_tick == 0 && placement.duration_ticks == 3_840
            })
            .map(|(track_id, placement_id, _)| (track_id, placement_id))
            .expect("规范工程里有一条 3840 tick 的 MIDI 摆放");
        for (id, track) in &mut project.tracks {
            if *id == track_id {
                track.clips.retain(|id, _| *id == placement_id);
            } else {
                track.clips.clear();
            }
        }
        let clip_id = project.tracks[&track_id].clips[&placement_id].clip_id;
        let entry = project.clip_pool.get_mut(&clip_id).expect("片段存在");
        let ClipContent::Midi { notes } = &mut entry.content else {
            panic!("夹具必须是 MIDI 片段");
        };
        let note_id = *notes.keys().next().expect("夹具至少一条音符");
        notes.clear();
        notes.insert(note_id, MidiNote::new(note_id, 480, 60, 480));
        (project, track_id, placement_id, clip_id, note_id)
    }

    /// `F2`：`track_notes_in_frames` 必须按**引擎／MCP 的同一份排程**展开
    /// `ratchet` 与 `micro_timing_ticks`（修前两者都被忽略 ⇒ `--sfz` 与内嵌采样器
    /// 在带这两个字段的工程上时值错误）。
    ///
    /// 口径（120 BPM、960 PPQ、48 kHz ⇒ 1 tick = 25 帧）：
    /// `startTick=480, durationTicks=480` ⇒ 从 12000 帧起、长 12000 帧；
    /// `microTimingTicks=-240` ⇒ 起点前移 6000 帧；`ratchet=4` ⇒ 四等分，
    /// 每份 120 tick = 3000 帧。
    #[test]
    fn sampler_note_frames_expand_ratchet_and_micro_timing() {
        use yeban_model::project::ClipContent;

        let (mut project, track_id, _placement_id, clip_id, note_id) = one_note_fixture();

        // 对照：默认字段（无 ratchet/micro）⇒ 与修前同解。
        assert_eq!(
            track_notes_in_frames(&project, track_id, 48_000),
            vec![(12_000, 12_000, 60, 100)]
        );

        let ClipContent::Midi { notes } = &mut project.clip_pool.get_mut(&clip_id).unwrap().content
        else {
            unreachable!("夹具是 MIDI 片段");
        };
        notes.get_mut(&note_id).unwrap().micro_timing_ticks = Some(-240);
        assert_eq!(
            track_notes_in_frames(&project, track_id, 48_000),
            vec![(6_000, 12_000, 60, 100)],
            "微时值必须作用在起点 tick 上（-240 tick = -6000 帧）"
        );

        let ClipContent::Midi { notes } = &mut project.clip_pool.get_mut(&clip_id).unwrap().content
        else {
            unreachable!("夹具是 MIDI 片段");
        };
        let note = notes.get_mut(&note_id).unwrap();
        note.micro_timing_ticks = None;
        note.ratchet = Some(4);
        assert_eq!(
            track_notes_in_frames(&project, track_id, 48_000),
            vec![
                (12_000, 3_000, 60, 100),
                (15_000, 3_000, 60, 100),
                (18_000, 3_000, 60, 100),
                (21_000, 3_000, 60, 100),
            ],
            "ratchet=4 必须等分成四个 120 tick 的脉冲"
        );
    }

    /// 同一条排程口径的另外三条：静音摆放、概率触发、循环展开。
    /// 修前 `track_notes_in_frames` **三条都不看** ⇒ `--sfz`/内嵌采样器与引擎不一致。
    #[test]
    fn sampler_note_frames_skip_muted_and_untriggered_notes_and_expand_loops() {
        use yeban_model::project::{ClipContent, LoopConfig};

        let (mut project, track_id, placement_id, clip_id, note_id) = one_note_fixture();

        // ① 静音摆放 ⇒ 不发声。
        project
            .tracks
            .get_mut(&track_id)
            .unwrap()
            .clips
            .get_mut(&placement_id)
            .unwrap()
            .muted = true;
        assert!(
            track_notes_in_frames(&project, track_id, 48_000).is_empty(),
            "静音摆放必须整条跳过"
        );
        project
            .tracks
            .get_mut(&track_id)
            .unwrap()
            .clips
            .get_mut(&placement_id)
            .unwrap()
            .muted = false;

        // ② probability = 0 ⇒ 确定性不触发（判定入口是模型的 `triggers`）。
        let ClipContent::Midi { notes } = &mut project.clip_pool.get_mut(&clip_id).unwrap().content
        else {
            unreachable!("夹具是 MIDI 片段");
        };
        notes.get_mut(&note_id).unwrap().probability = Some(0.0);
        assert!(
            track_notes_in_frames(&project, track_id, 48_000).is_empty(),
            "probability=0 的音符必须不排程"
        );

        // ③ 循环展开：区间 0..960、摆放 1920 tick、音符 0..240 ⇒ 两遍。
        let ClipContent::Midi { notes } = &mut project.clip_pool.get_mut(&clip_id).unwrap().content
        else {
            unreachable!("夹具是 MIDI 片段");
        };
        let note = notes.get_mut(&note_id).unwrap();
        note.probability = None;
        note.start_tick = 0;
        note.duration_ticks = 240;
        let placement = project
            .tracks
            .get_mut(&track_id)
            .unwrap()
            .clips
            .get_mut(&placement_id)
            .unwrap();
        placement.duration_ticks = 1_920;
        placement.loop_config = LoopConfig {
            enabled: true,
            start_tick: 0,
            end_tick: 960,
        };
        assert_eq!(
            track_notes_in_frames(&project, track_id, 48_000),
            vec![(0, 6_000, 60, 100), (24_000, 6_000, 60, 100)],
            "循环区间内的音符必须复制到摆放末端（每遍 960 tick = 24000 帧）"
        );
    }

    /// ⭐ `#251`（S3）：CLI 的 MIDI 路径**真的**应用声相，且与音频片段**同一条**曲线。
    ///
    /// 量什么：每条导出 WAV 的**左右声道峰值**（线性幅度），以及它们的比值 `L/R`。
    ///
    /// 三组 MIDI 输入 ＋ 一组音频片段输入（同一 `pan = -0.5`，同一导出选项）：
    /// ① `pan = -1.0` ⇒ 右峰值**逐位 0**、左峰值 > 0（等功率：`cos(0)=1`、`sin(0)=0`）；
    /// ② `pan = -0.5`（MIDI）⇒ `L/R = tan(3π/8)`；
    /// ③ 同 `pan = -0.5` 的**音频片段**轨 ⇒ 与 ② 的左右比相同（证明同一条 `track_pan_gains`）；
    /// ④ 正对照 `pan = 0` ⇒ [`track_pan_gains`] 逐位等于 `(1.0, 1.0)`（乘上去是恒等）。
    ///
    /// 这条判据的牙齿：把 `render_core` 里任一条 `.with_pan(...)` 删掉 ⇒ ② 退化成 1.0，
    /// 与 ③ 不再相等；把左/右声道写反 ⇒ 比值变成倒数。
    #[test]
    fn midi_panning_matches_the_audio_clip_curve() {
        use yeban_render::dither::PcmBuffer;

        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-pan-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let base = yeban_model::samples::demo_project();
        let assets = yeban_model::samples::demo_project_assets();
        let master = base.master_bus_track_id;
        let midi_track = base
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Midi && !track.clips.is_empty())
            .map(|track| track.id)
            .expect("演示工程必须有一条带音符的 MIDI 轨");
        let clip_track = base
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio && !track.clips.is_empty())
            .map(|track| track.id)
            .expect("演示工程必须有一条带摆放的音频轨");

        // 只留一条可闻轨（其余 mute、目标轨 solo）—— 把"本轨的声相"从总线的求和里隔离出来。
        let render = |track_id: EntityId, pan: f32, tag: &str| -> Vec<f32> {
            let mut project = base.clone();
            for (id, track) in &mut project.tracks {
                if *id == master {
                    continue;
                }
                track.mute = *id != track_id;
                track.solo = *id == track_id;
                track.solo_safe = false;
                track.pan = if *id == track_id { pan } else { 0.0 };
                // ⭐ `#257`：演示工程的贝斯轨带一条**读开的 `TrackPan` 泳道**
                // （`-1 → 0 → +1`；见 `yeban_model::samples::demo_automation_lanes`）。
                // 声相自动化现在真的生效 ⇒ 本判据必须先把泳道清掉，否则量到的是
                // 曲线而不是这里设置的**静态** `pan`（本测试的意图就是静态曲线）。
                track.automation_lanes.clear();
            }
            let path = dir.join(format!("{tag}.wav"));
            let options = WavExportOptions {
                frames: 96_000,
                sample_rate: 48_000,
                channels: 2,
                seed: 1,
                use_engine_source: true,
                bit_depth: BitDepth::Float32,
            };
            export_wav_with_sfz_file_and_assets(&project, &path, options, None, &assets)
                .unwrap_or_else(|error| panic!("{tag} 必须导出成功: {error}"));
            match yeban_render::wav::read_plain_wav(&path)
                .expect("回读 32f WAV")
                .1
            {
                PcmBuffer::Float32(samples) => samples,
                other => panic!("32f 导出必须是 Float32: {other:?}"),
            }
        };
        let peaks = |samples: &[f32]| {
            let mut left = 0.0_f32;
            let mut right = 0.0_f32;
            for frame in samples.as_chunks::<2>().0 {
                left = left.max(frame[0].abs());
                right = right.max(frame[1].abs());
            }
            (left, right)
        };
        let ratio = |samples: &[f32]| {
            let (left, right) = peaks(samples);
            assert!(right > 0.0, "这一路的右声道不该是 0（比值无定义）");
            (left / right, left, right)
        };

        // ① 全左：右声道必须是**逐位** 0（不是"很小"）。
        let (left_peak, right_peak) = peaks(&render(midi_track, -1.0, "midi-left"));
        assert!(left_peak > 0.0, "全左的 MIDI 轨左声道必须有信号");
        assert_eq!(
            right_peak, 0.0,
            "全左（pan = -1）⇒ 右声道必须逐位 0；实测右峰值 = {right_peak}"
        );

        // ② / ③ MIDI 与音频片段在同一个 pan 上必须有同一个左右比。
        let (midi_ratio, midi_l, midi_r) = ratio(&render(midi_track, -0.5, "midi-half"));
        let (clip_ratio, clip_l, clip_r) = ratio(&render(clip_track, -0.5, "clip-half"));
        let (gain_l, gain_r) = yeban_engine::mixer::pan_gains(
            -0.5,
            yeban_engine::mixer::PanLaw::ConstantPowerMinus3dB,
        );
        let expected = f64::from(gain_l) / f64::from(gain_r);
        assert!(
            (f64::from(midi_ratio) - expected).abs() < 1.0e-5,
            "MIDI 的左右比必须落在等功率曲线上: 实测 {midi_ratio}（L={midi_l} R={midi_r}）vs 期望 {expected}"
        );
        assert!(
            (f64::from(clip_ratio) - expected).abs() < 1.0e-5,
            "音频片段的左右比必须落在同一条曲线上: 实测 {clip_ratio}（L={clip_l} R={clip_r}）vs 期望 {expected}"
        );
        assert!(
            (f64::from(midi_ratio) - f64::from(clip_ratio)).abs() < 1.0e-5,
            "MIDI 与音频片段必须是同一条曲线: MIDI={midi_ratio} clip={clip_ratio}"
        );

        // ④ 正对照：pan = 0 ⇒ 单位增益（乘上去逐位恒等）。
        let mut centered = base.clone();
        centered
            .tracks
            .get_mut(&midi_track)
            .expect("MIDI 轨存在")
            .pan = 0.0;
        let track = centered.tracks.get(&midi_track).expect("MIDI 轨存在");
        assert_eq!(
            crate::audio_clip::track_pan_gains(&centered, track),
            (1.0, 1.0),
            "pan = 0 必须是单位增益（绝大多数工程靠这条恒等保持逐位不变）"
        );

        println!(
            "[wav-pan] midi-left: L={left_peak} R={right_peak}; midi(-0.5): L={midi_l} R={midi_r} ratio={midi_ratio}; clip(-0.5): L={clip_l} R={clip_r} ratio={clip_ratio}; expected={expected}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
