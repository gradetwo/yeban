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
//! ## "拒绝时不留半成品"
//!
//! `write_plain_wav` 自带该纪律（先 `check_match` 校验，再建文件）。本文件在**调用顺序**上
//! 不破坏它：**全部渲染与量化都在内存里完成**，只在最后一步调用一次 `write_plain_wav`。

use std::path::{Path, PathBuf};

use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::{NoteSchedule, SynthEngine};
use yeban_model::project::SampleRate;
use yeban_model::{EntityId, YebanProjectV1};
use yeban_render::dither::{BitDepth, quantize};
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
    /// 三个参数全 `None` 时的口径（＝旧行为的**语义**）：
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
    /// - **位深 24-bit 整数**（旧行为）。
    #[must_use]
    pub fn resolve(
        project: &YebanProjectV1,
        sample_rate: Option<SampleRate>,
        bit_depth: Option<BitDepth>,
        seconds: Option<f32>,
    ) -> Self {
        let sample_rate = sample_rate.unwrap_or_else(|| project.sample_rate()).hz();
        let seconds = seconds.unwrap_or(DEFAULT_WAV_SECONDS);
        // f64 而不是 f32：192 kHz × 长时长时 f32 的尾数不够（32 bit 只到 ~7 位十进制）。
        let frames = (f64::from(seconds) * f64::from(sample_rate)).round() as u64;
        Self {
            frames,
            sample_rate,
            bit_depth: bit_depth.unwrap_or(BitDepth::Int24),
            ..Self::default()
        }
    }
}

/// 导出报告（形态照 `MidiExportReport`：路径／帧数／采样率／声道／位深／字节数）。
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// 被**如实跳过**的音频片段原因（例如声道无法映射、坏 WAV、重采样失败）。
    /// ⚠ 素材采样率 ≠ 导出采样率**不再**进这里：`#141` 起离线导出会重采样到导出率。
    pub audio_clips_skipped: Vec<String>,
    /// ⭐ 控制侧 SFZ 解码读数（任务单 `#79`；键 = 轨）。空 = 本次导出没走模型 SFZ 链条
    /// （没有 `TrackV3.sampler`，或走了 `--sfz` 旁路 / 最小音源）。
    ///
    /// ⚠ **缺样本在这里可见**（`missing_sample_paths` / `silent_region_count`）⇒ 报告⛔ 不静默。
    pub sfz_decoded: Vec<(EntityId, crate::sfz_load::SfzDecodeReport)>,
}

impl WavExportReport {
    /// 一行可读摘要（CLI 打印它）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "WAV 已写出: {}（{} 帧 / {} Hz / {} 声道 / {} bit / {} 字节 / 音源 {} / digest {}）",
            self.path.display(),
            self.frames,
            self.sample_rate,
            self.channels,
            self.bits,
            self.bytes,
            self.source,
            self.digest
        )
    }
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
}

/// 首尾包络长度（帧）：避免方波起止的爆音。
const TONE_FADE_FRAMES: u64 = 64;

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
                    // 960 PPQ ⇒ 帧：`tick * sample_rate / 960`（整数运算，⛔ 不引入浮点漂移）。
                    let start_tick = placement.start_tick + note.start_tick;
                    let start = start_tick.saturating_mul(u64::from(sample_rate)) / 960;
                    let duration =
                        (note.duration_ticks.max(1)).saturating_mul(u64::from(sample_rate)) / 960;
                    let end = start + duration.max(1);
                    // 预先计算频率及 `2 * f / fs`（供渲染循环廉价算相位），以及幅度。
                    let frequency = 440.0_f64 * 2.0_f64.powf((f64::from(note.pitch) - 69.0) / 12.0);
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

    /// 该帧上所有正在发声的音符之和（`[-1, 1]` 之外的削波交给渲染层的母线）。
    fn sample_at(&self, frame: u64) -> f32 {
        let mut value = 0.0_f32;
        for &(start, end, phase_step, amp) in &self.notes {
            if frame < start || frame >= end {
                continue;
            }
            let offset = frame - start;
            // 通过整数取模判定方波正负半周，免去 `f64 % f64` 的浮点求余与 `powf`。
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

/// **引擎乐器音源**：每条轨道一个 [`SynthEngine`]，用**同一条离线装配路径**
/// （`SynthEngine::arm_from_snapshot`）武装，再按轨渲染（`render_track`）。
///
/// ⚠ 每个源持有**自己的**引擎：`RenderPlan::execute` 会把源分发到工作线程（`threads`），
/// 共享一个 `&mut SynthEngine` 需要锁，而"一个源一个引擎"没有锁也没有跨线程共享。
/// ⚠ 红线 7：`arm_from_snapshot` **会分配**（`begin_snapshot` 增长区间表）⇒ 只在**离线导出**
/// 里调用，⛔ 绝不在音频回调里调用（本文件是离线路径）。
pub(crate) struct EngineTrackSource {
    /// 已武装的合成器（本源独占）。
    ///
    /// ⭐ **为什么是 `Box<SynthEngine>` 而不是内联的 `SynthEngine`**（本票 #72 的修法）：
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
    /// ⭐ **武装用的那份快照**（任务单 `#79`）：`render_block` 从它**借用**该轨的 SFZ 摊平表
    /// （[`EngineSnapshot::sfz`]）交给 `render_track` 的第 3 参。
    ///
    /// ⭐ 为什么是 `Arc<EngineSnapshot>` 而**不是** clone 一份 `SfzPlan`：摊平表里带着**整段 PCM**
    /// （[`yeban_engine::sfz_instrument::PreparedSample`]），clone 会把它按轨再复制一份。
    /// `Arc` clone 只是**指针复制** ⇒ 与实时路径同一条纪律（"实时侧只**借用**快照，
    /// ⛔ 不 clone 那些 `Vec`"，见 `sfz_instrument.rs` 的模块文档）。
    snapshot: std::sync::Arc<EngineSnapshot>,
    /// 单声道暂存（`render_track` 输出"声相之前"的单声道）。
    mono: Vec<f32>,
}

impl EngineTrackSource {
    /// 从快照武装一个轨道源。
    ///
    /// # Errors
    ///
    /// `arm_from_snapshot` 的拒绝（含 `NoMasterBus`）。
    pub(crate) fn new(
        snapshot: std::sync::Arc<EngineSnapshot>,
        track: EntityId,
    ) -> Result<Self, WavExportError> {
        // ⭐ 直接构造**在堆上**：`SynthEngine::new` 的 109152 B 返回位因此只在本帧
        //    存在一次（随后 memcpy 进 `Box`），⛔ 不再有"返回位 ＋ 中间位 ＋ 装箱源"
        //    三份 10 万字节级副本叠在同一帧里（见字段文档的尺寸对照）。
        let mut engine = Box::new(SynthEngine::new(snapshot.sample_rate()));
        // ⛔ 不 `unwrap`：没有主总线时如实失败（调用方据此在**落盘之前**退出）。
        engine
            .arm_from_snapshot(&snapshot)
            .map_err(|error| WavExportError::Engine(error.to_string()))?;
        Ok(Self {
            engine,
            track,
            schedule: snapshot.schedules().get(&track).cloned(),
            snapshot,
            mono: Vec::new(),
        })
    }
}

impl AudioSource for EngineTrackSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        if self.mono.len() < frames {
            self.mono.resize(frames, 0.0);
        }
        // `render_track` 复用内部游标推进（与实时路径同一个函数）。
        // ⭐ ⭐ 第 3 参（SFZ 摊平表）**从快照借用**（任务单 `#79`）：这条轨在快照 `sfz()` 表里
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
        out.fill(0.0);
        for index in 0..frames {
            let value = self.mono[index];
            for channel in 0..channels {
                out[index * channels + channel] = value;
            }
        }
        Ok(())
    }
}

impl AudioSource for NoteToneSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        out.fill(0.0);
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        let track_gain = self.track_gain();
        for index in 0..frames {
            let frame = context.first_frame + index as u64;
            let mut value = self.sample_at(frame);
            // ⭐ **引擎层的轨增益门**乘在**本源输出之外**（与 `SfzTrackSource::ensure_rendered`
            //    同形）。`1.0` 是 IEEE 恒等 ⇒ 跳过乘法，"调用方没给轨参数"的读数逐位不变。
            if track_gain != 1.0 {
                value *= track_gain;
            }
            for channel in 0..channels {
                // ⭐ 累加进**已清零**的缓冲（与 `SfzTrackSource`／音频片段同口径）：这样
                //    `-x * 0.0 = -0.0` 会被 `+0.0` 归一 ⇒ 静音档是**逐位** 0（`to_bits() == 0`），
                //    而不是 `0x8000_0000`。非零样本上 `0.0 + x == x` 逐位相等。
                out[index * channels + channel] += value;
            }
        }
        Ok(())
    }
}

/// 一条轨道的音符，按 960 PPQ 换算成 `(起始帧, 时值帧, 音高, 力度)`（⛔ 与最小音源同一口径）。
pub(crate) fn track_notes_in_frames(
    project: &YebanProjectV1,
    track_id: EntityId,
    sample_rate: u32,
) -> Vec<(u64, u64, u8, u8)> {
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
                let start_tick = placement.start_tick + note.start_tick;
                let start = start_tick.saturating_mul(u64::from(sample_rate)) / 960;
                let duration =
                    (note.duration_ticks.max(1)).saturating_mul(u64::from(sample_rate)) / 960;
                notes.push((start, duration.max(1), note.pitch, note.velocity));
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
    // ⭐ **一条真源**：导出与录音都走 `render_core::render_project_f32`
    // （慢路径校验 → 计划编译 → 音源装配 → 执行 → digest 全在那里）。
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
        audio_clips_skipped: rendered.audio_clips_skipped,
        sfz_decoded: rendered.sfz_decoded,
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
            WavExportOptions::resolve(&project_48k, None, None, None),
            WavExportOptions::default(),
            "48 kHz 工程 + 全缺省 必须与旧行为逐字段相同"
        );

        // ② 44.1 kHz 工程 ⇒ 跟随工程，且**时长仍是 8 秒**（352800 帧，⛔ 不是 384000）。
        let mut project_44k = project_48k.clone();
        project_44k.audio_config.sample_rate = SampleRate::Hz44100;
        let followed = WavExportOptions::resolve(&project_44k, None, None, None);
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
            WavExportOptions::resolve(&project_48k, None, Some(BitDepth::Float32), None).bit_depth,
            BitDepth::Float32
        );
        assert_eq!(
            WavExportOptions::resolve(&project_48k, None, None, None).bit_depth,
            BitDepth::Int24,
            "缺省位深仍是 24-bit 整数"
        );
    }
}
