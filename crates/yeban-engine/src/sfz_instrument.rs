//! SFZ 乐器的**适配层**（`yeban-sfz` → 引擎）。[ROAD-M2-005, ROAD-M3-004]
//!
//! `yeban-sfz` 是 SFZ v2 的**唯一实现**（零拷贝解析器、区域/曲线/力度/交叉淡化/声部池）。
//! 它的 API 是**借用式**的：
//!
//! ```text
//! pub fn parse_text<'a>(text: &'a str, limits: &ParseLimits) -> Result<Instrument<'a>, SfzError>
//! pub struct Region<'a> { … }        // 同样带生命周期
//! ```
//!
//! 于是"把 `String` 与解析结果放进**同一个结构体**"会变成**自引用结构** —— 安全 Rust 里
//! 表达不了（必须 `unsafe` 或引入 `ouroboros` 一类新依赖）。本适配层因此采用**闭包式**设计：
//! 结构体只持有**文本与元数据**，解析在**使用时**发生，生命周期被限制在闭包内。
//!
//! # ⚠ 两条必须记住的事
//!
//! ① **本类型⛔ 不在实时音频路径上**：`with_instrument` 会**重新解析**（⇒ **堆分配**），
//!    若在音频回调里调用就**违反红线 7**（实时回调内禁任何堆分配/释放、禁锁等待、禁阻塞 I/O）
//!    [MUST-GATE-001]。回调里只能读**预先摊平**的只读数据。
//! ② **实时路径的正解（下一步）**：在**构造期/离线期**把 `Instrument::regions()` 摊平成
//!    **自有数据**（定长数组/预分配 `Vec`，`f32` 键范围与力度范围直接内联），回调侧只做
//!    整数/浮点比较与索引 ⇒ 零分配、零锁。⛔ 当前**尚未**实现这一步，故本类型只用于
//!    **控制侧**（加载、校验、计数、选择区域）。
//!
//! 本模块**不新增任何所有权机制**：只包一层文本 + 元数据。
//!
//! # ✂️ 摊平表（[`PreparedSfz`]）的**口径**与已知局限
//!
//! 摊平表是实时回调唯一能读的形态。它必须与**离线**路径（[`SfzInstrument::render_note_into`]）
//! 用**同一份**口径，否则同一条 SFZ 在导出与实时两条路上会发出不同的声音。
//!
//! ## 口径（本片修正后的三条）
//!
//! 1. **区域选择**：两侧都用 `yeban-sfz` 的 `Region::matches_query` —— 即
//!    `Instrument::region_for_with` 内部那个谓词（键 ＋ 力度 ＋ 通道 ＋ `trigger` 事件 ＋
//!    keyswitch ＋ CC，严格策略：门控没有外部状态 ⇒ **不匹配**）。
//!    ⛔ 引擎**不再**自拼 `matches_key && matches_velocity`（那会静默绕过 `trigger=release`
//!    一类的门控，把 release 采样在**按下**时打出来）。
//! 2. **多匹配语义 = `all`（叠加）**：一次触发下**所有**匹配区域都发声并相加（SFZ 里每个
//!    `region` 是独立声部），⛔ 不是「只取第一条」。两侧同此。
//! 3. **增益**：按 `PlaybackSpec::total_gain()` 的乘积结构摊平成
//!    `volume × 力度 × 键轴淡化 × 力度轴淡化(in/out 分开)`，相乘顺序与离线路径**逐位相同**
//!    （见 [`PreparedSfz::render_block_into`]）。⛔ 不再冻结在 `(note, velocity) = (60, 100)`。
//!
//! ## 仍未摊平 / 仍未应用（如实登记，⛔ 不假装已有）
//!
//! | # | 局限 | 状态 |
//! | :--- | :--- | :--- |
//! | ① | 力度增益 | **已修**（步 1：每力度一张表，⛔ 不再冻结在 100） |
//! | ② | `trigger` 门控 | **已修**（步 1：走 `matches_query`；门控不过的区域掩码全 0） |
//! | ③ | `offset`/`end`（`SampleSpan`） | ⭐ **步 3 已闭合**：离线 `render_note_into`／摊平 `render_block_into`／实时声部三条路共用 [`reduce_span`] 的**一份**归约 ⇒ `start`（起点）与 `end`（终点）都**真正应用**，两侧逐样本差 ＝ 0 |
//! | ④ | 无 `group`/`off_by`/`note_polyphony`/`seq_` | 未做（语义由 `yeban-sfz` 裁、槽位与窃取由 `SynthEngine` 裁 ⇒ 属步 2） |
//! | ⑤ | 无字节预算、无流式 | 未做（PCM 整段 clone 进摊平表；无预算上限、无流式解码） |
//! | ⑥ | 同一样本引两次 | ⭐ **步 3 已修**：去重键 ＝ `region.sample_path()`、容器 ＝ `BTreeMap`（确定性 [MODEL-AST-003]）⇒ 同一路径只 materialize **一份** [`PreparedSample`]；读数见 [`PreparedSfz::sample_count`] / [`PreparedSfz::pcm_bytes`] |
//! | ⑦ | CC 轴交叉淡化 | 未做（回调侧没有 CC 状态 ⇒ 与离线 `playback_spec` 一致地**跳过**该轴） |
//! | ⑧ | `loop_window` / `direction` / `pan` / `off_mode` | 未做（读数在 `PlaybackSpec` 里，摊平表未携带） |
//! | ⑨ | ⭐ **多 `sample=` 每区域各播自己** | ⭐ **步 3 已做**：摊平表持 `samples: Vec<PreparedSample>` ＋ [`PreparedRegion::sample_index`] 指路；⚠ 单样本重载 [`PreparedSfz::prepare`] **仍是广播**（键/力度映射对、**音色内容错** ⇒ 读数 `sample_content=broadcast(1)`） |
//!
//! # 引擎侧声部（任务 `#66` 的步 2：`SfzPlan` / `SfzVoicePool` / 分派次序）
//!
//! 摊平表（[`PreparedSfz`]）此前**只被测试引用**（`engine/lib.rs` 的模块地图里
//! `synth` 一节仍写"没有采样播放/SFZ"）。本片把它接进
//! [`crate::synth::SynthEngine::render_track`] 的**第三条音源支路**：
//!
//! | 件 | 归属 | 形状 |
//! | :--- | :--- | :--- |
//! | [`SfzPlan`] | 快照（控制侧投影，随快照共享） | 一条轨的摊平表（重数据：区域表 ＋ PCM 的 `Vec`） |
//! | [`SfzVoice`] / [`SfzVoicePool`] | [`crate::synth::SynthEngine`] 的轨道槽 | 定长 `[SfzVoice; 16]`，**全 `Copy`** |
//! | `render_track` 的第 3 参 | 快照 → 实时 | `sfz: Option<&PreparedSfz>`（与既有 `Option<&NoteSchedule>` **对称**） |
//!
//! ## 分派次序（**鼓机 ＞ SFZ ＞ 复音**）
//!
//! 一条轨至多一件音源（[`crate::drums`] §4.1 的既有裁决）。三件同时在场时的次序是
//! **鼓机 ＞ SFZ ＞ 复音**：`drum_note_map.is_some()` ⇒ 走鼓机（SFZ 池**被释放**）；
//! 否则 `sfz.is_some()` ⇒ 走 SFZ（⛔ 不叠加复音）；否则走复音。
//! ⚠ "鼓机 ＋ SFZ 同轨"是**引擎的取舍**、⛔ 不是模型语义 ⇒ 登记为 needs（见下表）。
//!
//! ## 终点口径（**无包络时的诚实占位**）
//!
//! 声部的停止条件是
//! `stop = min(end_sample, span 终点 → 引擎帧, 样本播完)`：
//!
//! * ⛔ **跳过 `ampeg_*`**：ADSR / 力度包络**本片不做** ⇒ 音符**没有**包络收尾；
//! * ⭐ **用样本自然结束**：源位置走到 [`PreparedRegion::source_end`]
//!   （`min(span 终点, PCM 长度)`；线性插值需要 `position` 与 `position + 1` 两个样本）
//!   即退休；
//! * 音符先到 [`crate::synth::ScheduledNote::end_sample`] 也退休。
//!
//! ⚠ `span` 的**起点**（`offset`）与**终点**（`end`）**步 3 已应用**：三条路都从
//! `span.start` 起播、在 `span.end` 停（[`reduce_span`] 的唯一归约）。循环**仍不做**。
//!
//! ## 分配纪律（红线 7 / [MUST-GATE-001]）
//!
//! * [`SfzPlan::prepare`]：**允许分配**（控制线程的构造期）；
//! * **快照边界⛔ 不摊平**：`SynthEngine::begin_snapshot` **不**碰 SFZ
//!   （`arm_from_snapshot` 会分配 ⇒ 它只能在**窗口外**被调用）；
//! * 逐样本路径只读 `&PreparedSfz`、原地写 `out`：**零分配、零锁、零 I/O**；
//! * ⭐ **PCM 借用快照**：`render_track` 收的是 `&PreparedSfz`（快照里那一份），
//!   ⛔ 不 clone `Vec`、⛔ 不 `Arc` clone `Vec`；
//! * ⚠ 摊平表带着**构造期的 `engine_hz`**（`rate_per_note`）⇒ **换采样率**必须由
//!   **控制侧在窗口外重新摊平**并发布一份新快照；引擎⛔ 不在快照边界重算，
//!   **在飞的声部**保留触发时抄下的 `rate`（实测见 `tests/rt_zero_alloc.rs` 的 ㉙）；
//! * **退休归属不变**：SFZ 声部的退休是**池内状态翻转**（`active = false`），
//!   ⛔ 不碰快照退役队列（旧快照仍由 `crate::snapshot` 的 `RetireProducer`
//!   交回控制线程释放）。
//!
//! ## ⚠ 与摊平表 `render_block_into` 的**口径边界**（如实登记）
//!
//! `PreparedSfz::render_block_into` 是**无状态的一次性调用**（每次从源位置 `span.start` 起播，
//! 立即播完），引擎声部是**有状态**的（跨量子延续 `position`、播完即退休）。
//! ⭐ 步 3 之后**两条路的数值口径完全相同**（`span` 起点/终点、样本、速率、增益都在
//! [`reduce_span`] / `gain_at` / `rate_per_note` 上对齐）⇒ "逐样本最大绝对差 ＝ 0"覆盖
//! **整段**（不再是"只覆盖触发量子"），判据见 `tests/sfz_instrument.rs` 的 S1/S1b/S3。
//! ⚠ 唯一的**额外因子**是引擎层的**轨增益门**（`track_gain`，见 `needs-1`）：
//! 摊平表那条路**没有轨上下文** ⇒ 对账时取**单位门**（`track_gain = 1.0`，`× 1.0` 逐位不变）
//! ⇒ 上式仍逐位成立；⭐ "引擎只多乘轨增益、⛔ 不碰力度"由 S18 逐位钉住。
//! 除此之外唯一的差别是**状态归属**（谁记住位置、谁负责退休），⛔ 不是数值口径。
//!
//! # 任务 `#67` 的步 3：**多采样** ＋ ⭐ **绑定注入 API**（⭐ 控制侧；⛔ 不发明模型语义）
//!
//! | 件 | 形状 | 说明 |
//! | :--- | :--- | :--- |
//! | [`PreparedSample`] | 摊平表里的**一份**样本（`path` ＋ `pcm` ＋ `sample_hz`） | 去重键 ＝ `region.sample_path()`；容器 ＝ `BTreeMap` ⇒ 顺序确定 [MODEL-AST-003] |
//! | [`PreparedRegion::sample_index`] | `u16` 指路 | `NO_SAMPLE` 哨兵 ＝ 该区域**不产生采样输出** |
//! | [`PreparedSfz::prepare_with_samples`] | 控制侧构造 | 每区域**播自己**那份样本；同一路径**只 materialize 一次**；⭐ 缺样本 ⇒ **静音 ＋ 计数**（⛔ 不发明替代内容） |
//! | [`PreparedSfz::prepare`] | 单样本**广播**（兼容重载，⛔ 未删） | 所有 `sample_index = 0`；⚠ **音色内容错**（各 region 播同一份）⇒ 读数 `sample_content=broadcast(1)` |
//! | [`SfzPlan::prepare_with_samples`] | 快照投影 | 经 [`crate::snapshot::EngineSnapshot::with_sfz`] 注入 ⇒ **唯一**的公开绑定路径 |
//!
//! ⛔ **解码在控制侧**：引擎只接**已经解码**的 [`InMemorySample`]（⛔ 不引 `symphonia`/`rubato`）。
//! 调用方先用 [`SfzInstrument::referenced_sample_paths`] 拿到"这条 SFZ 到底引用哪些路径"
//! （排序去重），再按 `crates/yeban-app/src/sfz_load.rs` 的形状（真读 PCM WAV、读不到 ⇒
//! 如实回退 ＋ 把原因写进来源文案）解码成 `&[(String, InMemorySample)]`。
//! ⭐ ⭐ **引擎侧对"没给"的路径只做一件事：出静音 ＋ 计数**（[`PreparedSfz::silent_region_count`]
//! ／[`PreparedSfz::missing_sample_paths`]），⛔ **不发明替代内容** ——
//! "用什么替代"是**控制侧**的产品决定（归属裁决见台账 `R711`；旧行为"回退到 `samples[0]`"
//! 是**内容错**，已删除 ⇒ `needs-7` 闭合）。
//!
//! ## needs 汇总（⭐ 本步闭合的标注"已闭合"，未闭合的说明卡在哪）
//!
//! | # | 缺口 | 为什么需要裁决 |
//! | :--- | :--- | :--- |
//! | sfz-needs-1 | **轨推子/静音没有乘到 SFZ 声部上** | ⭐ ⭐ **R711 已裁决、本步已闭合**：**乐器层**拥有乐器内增益（`volume_gain`／`velocity_gain[128]`／`key_crossfade`／`velocity_fade_*`／`amp_veltrack`），**引擎层**拥有轨增益（推子＝音量、静音、独奏）⇒ ⭐ **两者相乘、⛔ 不重复**。落地口径：`snapshot::project_schedules` 把**同一个** `track_gain_scale(...)` 结果写进 [`crate::synth::NoteSchedule::track_gain`]（与鼓机/复音读的 `note.gain` **同源**）⇒ [`SfzVoicePool::trigger_note`] 把它乘在 `region.gain_at(...)` **之外** ⇒ ⭐ SFZ 与鼓机**同径**（判据：`tests/sfz_instrument.rs` 的 S17／S18）|
//! | sfz-needs-2 | **池满（> 16 条区域同时匹配）只丢弃 ＋ 计数，没有窃取口径** | ⭐ ⭐ **本单已闭合**：引擎侧 SFZ 池**采纳引擎既有那一套**（`yeban_dsp::polysynth::PolySynth` 的三段键 `(层级, 触发序号, 槽位下标)`，见 [`SfzVoicePool::select_victim`]）＋ 3 ms 指数淡出（`yeban-sfz` 的 `StealFade` 形状、**同一把** `steal_fade_frames` 舵）。`yeban_sfz::voice_pool::VoicePool::select_victim` 是**它自己那一份**（`(层级, 注入电平, 序号, 下标)` ＋ 立即接管），且那个池**不被引擎使用** ⇒ ⛔ 不构成"两份实现"。落地：池满走 `steal` —— victim 在自己槽里淡出、新声部**挂起在同一槽**（`pending`）⇒ 定容池⛔ 不需要额外 slack 槽；真丢弃只剩"挂起者还没发声就被再一次换掉"（计 `dropped`，⛔ 不静默），窃取计 `stolen`（判据 S8/S19）|
//! | sfz-needs-3 | **鼓机 ＋ SFZ 同轨的取舍**（本片＝鼓机优先，SFZ 池被释放） | ⭐ ⭐ **本单已闭合（信号腿）**：裁决**维持** `鼓机 ＞ SFZ ＞ 复音`（与 [`crate::drums`] §4.1 的既有裁决一致 ⇒ ⛔ **不改成叠加**）；真正缺的是"用户看得出被忽略了" ⇒ 新增**投影读数** [`crate::synth::SynthEngine::armed_sources`] ／ [`crate::synth::SynthEngine::ignored_sources`]（三件同轨 ⇒ 武装 `3`、被忽略 `2`；判据 S20）。⚠ **模型面**那一半（"模型若禁止这种组合，本条删除"）仍挂在 [`crate::drums`] 的 `needs-3`（属模型面，本单⛔ 不动）|
//! | sfz-needs-4 | **`span` 终点已在引擎侧应用、离线侧未应用** | ⭐ **步 3 已闭合**：离线／摊平／实时三条路共用 [`reduce_span`]，`start`＋`end` 都应用；Criterion S3 量到**逐样本差 ＝ 0**（详见 `docs/ledger/sfz-core-notes.md` §13.2） |
//! | sfz-needs-5 | **轨↔SFZ 的模型绑定不存在**（⭐ 本步只把它做成**可经公开路径调用的 API**） | `yeban-model` 里 SFZ 命中只有一行注释（`model/src/lib.rs:16`）、`DeviceKind` 四档没有 sampler ⇒ 今天只能由引擎 API（[`crate::snapshot::EngineSnapshot::with_sfz`] ＋ [`SfzPlan::prepare_with_samples`]）显式注入。**"SFZ 如何进 `YebanProjectV1`"仍未裁**（⛔ 不发明模型语义：本步不加 `DeviceKind` 档、不改 schema）（未闭合、属模型面） |
//! | sfz-needs-6 | **循环 / ADSR / `off_mode`** | ⭐ **步 3 只闭合了它的"多采样／每区域独立 PCM"那一半**；循环、`ampeg_*`、`off_mode`、`group`/`off_by` 仍完全未做（未闭合） |
//! | sfz-needs-7 | ⭐ **缺样本的引擎侧回退是"广播到 `samples[0]`"（内容错）** | ⭐ ⭐ **R711 已裁决、本步已闭合**：**引擎出静音并计数**（[`PreparedSfz::silent_region_count`] ＋ [`PreparedSfz::missing_sample_paths`]）、**合成替代内容是控制侧的决定**（想给替代样本 ⇒ 调用方显式放进提供表，或用广播重载 [`PreparedSfz::prepare`] 明确声明）。落地口径：`prepare_with_samples` 遇缺失路径 ⇒ 该区域 `sample_index = NO_SAMPLE` ⇒ 触发时**不建声部**（与 `span()==None` 同径）⇒ ⛔ **旧"广播到 `samples[0]`"已删除**（判据：`tests/sfz_instrument.rs` 的 S15）|
//! | sfz-needs-8 | ⭐ **摊平表不含样本字节预算 / 流式** | `prepare_with_samples` 把每个被引用的 `InMemorySample::pcm` **整段 clone** 进摊平表（去重后 N 份）；无上限、无流式、无共享（`Arc`）口径（未闭合） |

use std::collections::BTreeMap;

// ⭐ 窃取淡出的**帧数口径**：引擎里 PolySynth 用的就是这一个（3 ms × 采样率，
// 上限 0.5 s、下限 1 帧）⇒ SFZ 池与复音池共用**同一个**舵，⛔ 不是第二份常数。
use yeban_dsp::polysynth::steal_fade_frames_for;
use yeban_sfz::crossfade::{XfAxis, XfDirection};
use yeban_sfz::error::SfzError;
use yeban_sfz::instrument::{Instrument, Region, RegionQuery};
// ⚠ 这两个是 **lib 构建**就需要的（`load` / `with_instrument` 都调用它们）⇒ ⛔ 不能放在 `#[cfg(test)]` 下，
// 否则非测试构建会报 "cannot find type `ParseLimits`"（本批实测过一次：三项门全红）。
use yeban_sfz::parser::{ParseLimits, parse_text};
use yeban_sfz::playback::{RenderRates, SampleSpan};
// ⭐ 衰减底（−60 dBFS）取自 `yeban-sfz` 的 [`StealFade`](yeban_sfz::voice_pool::StealFade)：
// 引擎侧的 SFZ 窃取淡出与该库的 3 ms 指数淡出**同形状**（`FLOOR^(n/N)`），
// ⛔ 不是引擎自造的一条包络（victim **选择**口径则由引擎自己拥有，见 `SfzVoicePool::select_victim`）。
use yeban_sfz::voice_pool::STEAL_FADE_FLOOR;

/// 已加载的 SFZ 乐器：**文本 + 元数据**（⛔ 不含借用式解析结果）。
#[derive(Debug, Clone)]
pub struct SfzInstrument {
    /// 原始 SFZ 文本（自己持有 ⇒ 无生命周期参数）。
    text: String,
    /// 构造期解析得到的**区域数**（元数据，供控制侧快速查询）。
    region_count: usize,
}

impl SfzInstrument {
    /// 构造期解析一次：**校验**文本并记下元数据。
    ///
    /// ⚠ 本函数会分配（`String` 已由调用方给出；解析器内部有分配）⇒ **只允许在控制侧调用**。
    ///
    /// # Errors
    /// 文本非法（语法、超限等）时原样透出 [`SfzError`]，⛔ 不 panic。
    pub fn load(text: String, limits: &ParseLimits) -> Result<Self, SfzError> {
        let region_count = parse_text(&text, limits)?.regions().len();
        Ok(Self { text, region_count })
    }

    /// 区域数（规范里的 `region` 条目个数）。
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.region_count
    }

    /// 原始文本（调用方可能需要原样回写/哈希）。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// **用时现解析**：把借用式 [`Instrument`] 限制在闭包内 ⇒ 无需自引用结构。
    ///
    /// ⚠ **控制侧专用**（见模块头文档 ①）：本函数会重新解析 ⇒ 会分配。
    ///
    /// # Errors
    /// 若文本在此期间变得非法（正常路径不会发生），透出 [`SfzError`]。
    pub fn with_instrument<R>(&self, f: impl FnOnce(&Instrument<'_>) -> R) -> Result<R, SfzError> {
        let limits = ParseLimits::default();
        let instrument = parse_text(&self.text, &limits)?;
        Ok(f(&instrument))
    }

    /// 在给定 `note`/`velocity` 下**选中**的区域（控制侧的查询口径）。
    ///
    /// ⭐ 复用 `yeban-sfz` 的**唯一**选择谓词 `Region::matches_query`（即
    /// `Instrument::region_for_with` 内部那个）：键/力度之外还包含通道、`trigger` 事件、
    /// keyswitch 与 CC 门控（严格策略：没有外部门控状态 ⇒ 不匹配）。
    /// ⛔ 不再自拼 `matches_key && matches_velocity` —— 那会绕过门控（`trigger=release`
    /// 的 region 会进 note-on 的命中集）。
    ///
    /// # Errors
    /// 同 [`Self::with_instrument`]。
    pub fn regions_matching(&self, note: u8, velocity: u8) -> Result<Vec<usize>, SfzError> {
        self.with_instrument(|instrument| {
            let query = RegionQuery::new(note, velocity);
            instrument
                .regions()
                .iter()
                .enumerate()
                .filter(|(_, region)| region.matches_query(&query))
                .map(|(index, _)| index)
                .collect::<Vec<usize>>()
        })
    }

    /// ⭐ **这条 SFZ 到底引用哪些样本路径**（排序 ＋ 去重 ⇒ 确定性顺序）。
    ///
    /// 它是**控制侧解码**的入口：调用方拿这张表去真读 PCM WAV
    /// （形状见 `crates/yeban-app/src/sfz_load.rs:238-269`），把结果整理成
    /// `&[(String, InMemorySample)]` 再交给 [`PreparedSfz::prepare_with_samples`]。
    /// ⚠ 路径是 [`yeban_sfz::instrument::Region::sample_path`] 的原样读数（含 `default_path`
    /// 前缀、⛔ 不做任何规范化/存在性检查）。
    ///
    /// ⚠ **会分配**（重新解析 ＋ `BTreeSet`）⇒ 控制侧专用（见模块头文档 ①）。
    ///
    /// # Errors
    /// 同 [`Self::with_instrument`]。
    pub fn referenced_sample_paths(&self) -> Result<Vec<String>, SfzError> {
        self.with_instrument(|instrument| {
            let mut paths = std::collections::BTreeSet::new();
            for region in instrument.regions() {
                paths.insert(region.sample_path().into_owned());
            }
            paths.into_iter().collect::<Vec<String>>()
        })
    }
}

/// **只在内存里的样本源**（本切片的"样本源"）。
///
/// ⚠ 本仓目前**没有**采样文件读取/解码 ⇒ 样本数据必须由**调用方**提供；
/// 本类型是离线/控制侧的最小载体（构造期分配，⛔ 不用于实时回调）。
#[derive(Debug, Clone)]
pub struct InMemorySample {
    /// 单声道 PCM（归一化到 `[-1, 1]`）。
    pub pcm: Vec<f32>,
    /// 该 PCM 的原始采样率（Hz）—— 换算交给 `yeban-sfz` 的 `RenderRates`。
    pub sample_hz: f32,
}

impl InMemorySample {
    /// 造一条**极短正弦**（判据/体验用的最小样本源）：`cycles` 个周期 @ `sample_hz`。
    #[must_use]
    pub fn sine(freq_hz: f32, sample_hz: f32, frames: usize) -> Self {
        let pcm = (0..frames)
            .map(|index| {
                let phase = core::f32::consts::TAU * freq_hz * (index as f32) / sample_hz;
                phase.sin() * 0.5
            })
            .collect();
        Self { pcm, sample_hz }
    }
}

impl SfzInstrument {
    /// **离线**渲染一个音符到 `out`（`+=` 叠加），返回写入的帧数。
    ///
    /// ⭐ 复用 `yeban-sfz` 自己的口径：区域选择用 `Region::matches_query`（含 trigger /
    /// keyswitch / 通道 / CC 门控）、播放速率用 `Region::playback_rate`、总增益用
    /// `PlaybackSpec::total_gain`（⛔ 引擎不重写这些规则）。
    /// 样本数据来自调用方给的 [`InMemorySample`]（本仓尚无采样文件读取）。
    ///
    /// ⭐ **多匹配语义 = `all`**：匹配的**每一条**区域都渲染并叠加（SFZ 里每个 `region` 是
    /// 独立声部），⛔ 不是「只取 `find` 的第一条」。返回值 = 各匹配区域写入帧数的**最大值**
    /// （⛔ 不是各区域之和；所有区域共享同一份 `source.pcm`，故它即最长那条的帧数）。
    /// [`PreparedSfz::render_block_into`] 用同一口径。
    ///
    /// ⭐ **`offset`/`end` 已应用**（步 3）：源位置从 `span.start` 起、在
    /// `min(span.end, PCM 长度)` 停 —— 与摊平表/实时声部共用 [`reduce_span`] 的**一份**归约。
    /// `end=-1`（[`SampleEnd::Silent`](yeban_sfz::instrument::SampleEnd)）⇒ 该区域**不写任何帧**。
    ///
    /// ⚠ **只覆盖离线**：本函数在调用时**解析**（⇒ 分配）⇒ ⛔ 不得在音频回调里调用（红线 7）。
    /// 实时侧用 [`PreparedSfz`]（构造期摊平）。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]；没有匹配区域 ⇒ 返回 `Ok(0)`（合法：该音符不发声）。
    pub fn render_note_into(
        &self,
        source: &InMemorySample,
        note: u8,
        velocity: u8,
        out: &mut [f32],
        engine_hz: f32,
    ) -> Result<usize, SfzError> {
        let pcm_len = source.pcm.len();
        let rates = RenderRates::new(source.sample_hz, engine_hz);
        let query = RegionQuery::new(note, velocity);
        self.with_instrument(|instrument| {
            let mut written = 0usize;
            for region in instrument.regions() {
                if !region.matches_query(&query) {
                    continue;
                }
                // ⭐ 复用 `yeban-sfz` 自己的换算：速率与总增益都来自它的 `PlaybackSpec`
                let rate = region.playback_rate(note, rates);
                let gain = region.playback_spec(note, velocity, rates).total_gain();
                if !(rate.is_finite() && rate > 0.0) || pcm_len == 0 {
                    continue;
                }
                // ⭐ `offset`/`end`：与摊平表、实时声部**同一份**归约（`None` ⇒ 不产生输出）。
                let Some((start, source_end)) = reduce_span(region.playback_span(), pcm_len) else {
                    continue;
                };
                let mut position = start;
                // 离线/摊平这条路**不窃取**：淡出状态恒为 [`FadeRamp::NONE`]
                // （`gain == 1.0` ⇒ 那个逐帧乘法逐位不变）。
                let mut fade = FadeRamp::NONE;
                let region_written = mix_span_into(
                    &source.pcm,
                    &mut position,
                    rate,
                    gain,
                    source_end,
                    &mut fade,
                    out,
                );
                written = written.max(region_written);
            }
            written
        })
    }
}

/// ⭐ **`span`（`offset`/`end`）归约的唯一实现**：离线 [`SfzInstrument::render_note_into`]、
/// 摊平 [`PreparedSfz::render_block_into`] 与实时声部（[`SfzVoicePool`]）三条路共用它
/// ⇒ 两侧口径不会分叉（本仓"⛔ 不许两份实现"的机械纪律）。
///
/// 返回**半开区间** `[start, end)`（单位：源采样帧）的起点（`f32`，供逐样本位置使用）
/// 与终点（`u64`）：
///
/// * `span == None`（`end=-1`，或显式区间落在 `offset` 之前）⇒ `None` ＝ **不产生采样输出**；
/// * `span.end == None` ⇒ 终点取 `pcm_len`（"播到采样末尾"）；
/// * 显式 `end` 超出 PCM ⇒ 夹到 `pcm_len`（⛔ 不越界读）；
/// * 归约后**空区间**（`end <= start`，例如 `offset` 落在 PCM 之外）⇒ `None`
///   （⛔ 不是"从 0 播一整份"）。
///
/// ⚠ `start as f32`：逐样本位置本来就是 `f32`（`mix_span_into` 的口径），
/// 超过 2²⁴ 帧的 `offset` 会有位级舍入 —— 如实登记，⛔ 不假装是精确整数。
///
/// ⚠ **零分配、零锁、零 I/O**（纯整数/浮点比较）⇒ 可被实时声部在**触发期**调用 [MUST-GATE-001]。
#[must_use]
pub fn reduce_span(span: Option<SampleSpan>, pcm_len: usize) -> Option<(f32, u64)> {
    let span = span?;
    let pcm_len = pcm_len as u64;
    let end = span.end.unwrap_or(pcm_len).min(pcm_len);
    if end <= span.start {
        return None;
    }
    Some((span.start as f32, end))
}

/// ✂️ **摊平后的一个区域**：回调侧**只读**（⛔ 无 `Vec`／`Box`／`String`，全部定长）。
///
/// 键/力度的可播性、以及**全部静态门控**（通道 / `trigger` 事件 / keyswitch / CC）
/// 在**构造期**由 `yeban-sfz` 的 `Region::matches_query` 求值后**打包成 128 位掩码** ⇒
/// 回调侧只做位测试，⛔ 不在回调里调用任何解析/分配函数。
/// 门控未通过的区域**两张掩码都清 0** ⇒ 它**永远不会**进回调的命中集
/// （含 `trigger=release` 一类的 region）。
///
/// 增益按 `PlaybackSpec::total_gain()` 的乘积结构拆成四份（`volume`／力度／键轴淡化／
/// 力度轴淡化），使回调侧的相乘顺序与离线路径**逐位相同**：
/// `(volume × 力度) × ((键轴 × 力度淡入) × 力度淡出)`。
/// 因此 `(note, velocity)` 的取值**不再冻结**在 `(60, 100)`。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRegion {
    /// 键掩码：第 `n` 位为 1 表示该区域接受 `note == n`（门控不过时全 0）。
    key_mask: [u64; 2],
    /// 力度掩码：第 `v` 位为 1 表示该区域接受 `velocity == v`（门控不过时全 0）。
    vel_mask: [u64; 2],
    /// 每个音符的播放速率（构造期由 `Region::playback_rate` 求值）。
    rate_per_note: [f32; 128],
    /// 键轴（`xf*_*key`）交叉淡化的每音符因子；无该轴时恒 `1.0`。
    key_crossfade: [f32; 128],
    /// 力度轴**淡入**（`xfin_*vel`）段的每力度因子；无该段时恒 `1.0`。
    velocity_fade_in: [f32; 128],
    /// 力度轴**淡出**（`xfout_*vel`）段的每力度因子；无该段时恒 `1.0`。
    ///
    /// 与 [`Self::velocity_fade_in`] **分开**存：`Region::crossfade_gain` 是一条
    /// 从左到右的乘法链，`(K * 淡入) * 淡出` 与 `K * (淡入 * 淡出)` 在 binary32 下
    /// 不一定同值 ⇒ 分开存才能逐位还原该链的结合序。
    velocity_fade_out: [f32; 128],
    /// `PlaybackSpec::gain`：`volume` 换算出的线性增益（与键/力度无关）。
    volume_gain: f32,
    /// `PlaybackSpec::velocity_gain` 的**每力度**读数（⛔ 不再冻结在力度 100）。
    velocity_gain: [f32; 128],
    /// ⭐ **该区域用哪一份摊平样本**（[`PreparedSfz::samples`] 的下标）。
    ///
    /// [`Self::NO_SAMPLE`] ＝ 该区域**不产生采样输出**（调用方没给它的 `sample_path`）
    /// ⇒ 渲染与建声部时整条跳过。
    sample_index: u16,
    /// ⭐ `offset` / `end` 归约出的**源采样区间**（半开 `[start, end)`，单位源采样帧）。
    ///
    /// `None` 表示该区域**不产生采样输出**（`end=-1`，或显式区间落在 `offset` 之前）。
    /// ⭐ **步 3 起渲染真正应用它**（[`reduce_span`]）：起点从 `span.start` 开始、
    /// 终点停在 `min(span.end, PCM 长度)` ⇒ 离线/摊平/实时三条路同口径。
    span: Option<SampleSpan>,
}

impl PreparedRegion {
    /// ⭐ 「没有样本」哨兵（[`Self::sample_index`] 的取值）：该区域**不产生采样输出**。
    ///
    /// 出现条件只有一个：调用方给的样本表里**没有**该区域的 `sample_path`
    /// ⇒ 如实标静音 ＋ 计数（⛔ 不静默、⛔ 不 panic、⛔ **不去播别人的采样**）。
    /// ⚠ 归属裁决（台账 R711）：引擎出静音，**合成替代内容是控制侧的决定**。
    pub const NO_SAMPLE: u16 = u16::MAX;

    /// 该区域是否接受 `(note, velocity)`（⭐ 纯位测试：无分配、无解析）。
    #[must_use]
    pub fn accepts(&self, note: u8, velocity: u8) -> bool {
        let n = note as usize;
        let v = velocity as usize;
        self.key_mask[n / 64] & (1u64 << (n % 64)) != 0
            && self.vel_mask[v / 64] & (1u64 << (v % 64)) != 0
    }

    /// 键掩码的**只读副本**（判据用：`trigger=release` 的区域必须为 `[0, 0]`）。
    #[must_use]
    pub fn key_mask(&self) -> [u64; 2] {
        self.key_mask
    }

    /// 力度掩码的**只读副本**（判据用）。
    #[must_use]
    pub fn velocity_mask(&self) -> [u64; 2] {
        self.vel_mask
    }

    /// ⭐ 该区域的**源采样区间**（`offset`/`end` 归约；`None` ＝ 不产生采样输出）。
    #[must_use]
    pub fn span(&self) -> Option<SampleSpan> {
        self.span
    }

    /// ⭐ 该区域**用哪一份摊平样本**（[`PreparedSfz::samples`] 的下标）。
    #[must_use]
    pub const fn sample_index(&self) -> u16 {
        self.sample_index
    }

    /// ⭐ 该区域**有没有**样本可播（`false` ＝ [`Self::NO_SAMPLE`]）。
    #[must_use]
    pub const fn has_sample(&self) -> bool {
        self.sample_index != Self::NO_SAMPLE
    }

    /// ⭐ **`span` 的归约结果**：`(源起点, 源终点)`（半开）；`None` ＝ 不产生采样输出。
    ///
    /// 直接转交 [`reduce_span`]（⭐ **唯一**归约）⇒ 离线/摊平/实时三条路不可能分叉。
    #[must_use]
    pub fn span_bounds(&self, pcm_len: usize) -> Option<(f32, u64)> {
        reduce_span(self.span, pcm_len)
    }

    /// ⭐ **源采样起点**（`offset`，已归约）；`None` ＝ 不产生采样输出。
    #[must_use]
    pub fn source_start(&self, pcm_len: usize) -> Option<f32> {
        self.span_bounds(pcm_len).map(|(start, _)| start)
    }

    /// ⭐ **源采样终点（半开）**：`min(span 终点, 样本播完)`；`None` ＝ 不产生采样输出。
    ///
    /// * [`Self::span`] 为 `None` ⇒ `None`（`end=-1` / 空区间）⇒ 实时侧⛔ 不为它建声部；
    /// * `span.end` 未指定（`offset`/`end` 都没写）⇒ 取 `pcm_len`（"到采样末尾"）；
    /// * 显式 `end` 超出 PCM ⇒ 夹到 `pcm_len`（⛔ 不越界读）；
    /// * 归约后空区间（`end <= start`）⇒ `None`（⛔ 不是"从 0 播"）。
    #[must_use]
    pub fn source_end(&self, pcm_len: usize) -> Option<u64> {
        self.span_bounds(pcm_len).map(|(_, end)| end)
    }

    /// 给定时点的**合并线性增益**（与离线 `PlaybackSpec::total_gain` 逐位相同）。
    #[must_use]
    pub fn gain_at(&self, note: u8, velocity: u8) -> f32 {
        let n = note as usize;
        let v = velocity as usize;
        let base = self.volume_gain * self.velocity_gain[v];
        let crossfade =
            (self.key_crossfade[n] * self.velocity_fade_in[v]) * self.velocity_fade_out[v];
        base * crossfade
    }
}

/// ✂️ **摊平后的乐器**：区域表 ＋ **样本表**，**构造期**建好，回调侧只读。
///
/// # 样本形态（⭐ 任务 `#67` 的步 3）
///
/// * [`Self::prepare_with_samples`]：**每区域播自己**那份样本 —— 去重键 ＝ `region.sample_path()`
///   字符串、容器 ＝ `BTreeMap`（确定性顺序 [MODEL-AST-003]）⇒ 同一路径**只 materialize 一份**
///   [`PreparedSample`]；下标由 [`PreparedRegion::sample_index`] 指路；
/// * [`Self::prepare`]：单样本**广播**（兼容重载，⛔ 未删）⇒ 所有 `sample_index = 0`：
///   键/力度映射**对**、**音色内容错**（各 region 播同一份）⇒ 读数 `sample_content=broadcast(1)`。
///
/// # 分配纪律（红线 7 / [MUST-GATE-001]）
///
/// * [`Self::prepare`] / [`Self::prepare_with_samples`]：**允许分配**（区域表与样本 PCM 的
///   `Vec` 在构造期/控制侧建立）；
/// * [`Self::render_block_into`]：**禁止分配** —— 它只做位测试、数组读取与
///   `out` 的原地写入（⛔ 无 `Vec::push`／`Box`／`String`／锁／I/O）。
///   违反的后果：实时回调内产生堆分配/释放或锁等待 ⇒ 可能造成音频线程不可预测的停顿
///   （红线 7 的一票否决项），由 `tests/rt_zero_alloc.rs` 的**计数型全局分配器**窗口判据检测。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSfz {
    regions: Vec<PreparedRegion>,
    /// ⭐ **去重后的样本表**：每个被区域引用的 `sample_path` 至多一份。
    samples: Vec<PreparedSample>,
    /// 建表口径：`true` ⇒ 单样本重载 [`Self::prepare`] 的**广播**路径（⛔ 内容错，已登记）。
    broadcast: bool,
    /// ⭐ 缺样本 ⇒ 标 `NO_SAMPLE`（**不产生采样输出**）的区域数（**计数**，⛔ 不静默）。
    ///
    /// ⚠ 归属裁决（台账 R711）：引擎只出**静音** ＋ 计数，⛔ **不发明替代内容**
    /// （旧行为"回退到 `samples[0]`"是**内容错**，已删除）。
    silent_regions: usize,
    /// ⭐ 缺样本的**路径**（排序 ＋ 去重 ⇒ 确定性顺序；报告/诊断用）。
    missing_samples: Vec<String>,
}

/// ✂️ **摊平后的一份样本**：路径（去重键）＋ PCM ＋ 原始采样率。
///
/// ⚠ PCM 是**整段 clone** 进来的（构造期分配）：无字节预算、无流式、无 `Arc` 共享
/// ⇒ 登记为 sfz-needs-8。回调侧只**借用**它（⛔ 不 clone、⛔ 不 `Arc` clone）。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSample {
    /// 去重键 ＝ [`yeban_sfz::instrument::Region::sample_path`] 的字符串（⛔ 不规范化、⛔ 不查文件）。
    ///
    /// ⚠ [`PreparedSfz::prepare`] 的**广播**路径没有路径身份 ⇒ 它是**空串**。
    path: String,
    /// 单声道 PCM（归一化到 `[-1, 1]`）。
    pcm: Vec<f32>,
    /// 该 PCM 的原始采样率（Hz；摊平期已在 `Region::playback_rate` 里换算过）。
    sample_hz: f32,
}

impl PreparedSample {
    /// 去重键（`region.sample_path()` 的原样字符串；广播路径为空串）。
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// ⭐ 只读 PCM（**实时侧借用它**，⛔ 不 clone 这个 `Vec`）。
    #[must_use]
    pub fn pcm(&self) -> &[f32] {
        &self.pcm
    }

    /// 原始采样率（Hz）。
    #[must_use]
    pub const fn sample_hz(&self) -> f32 {
        self.sample_hz
    }

    /// 帧数。
    #[must_use]
    pub fn frames(&self) -> usize {
        self.pcm.len()
    }

    /// PCM 字节数（⛔ 不含 `path` 文本）。
    #[must_use]
    pub fn pcm_bytes(&self) -> usize {
        core::mem::size_of::<f32>() * self.pcm.len()
    }

    /// 从调用方给的 [`InMemorySample`] **clone** 一份（构造期；`path` 是去重键）。
    fn from_in_memory(path: String, source: &InMemorySample) -> Self {
        Self {
            path,
            pcm: source.pcm.clone(),
            sample_hz: source.sample_hz,
        }
    }
}

impl PreparedSfz {
    /// **构造期**摊平（**单样本广播**）：把区域谓词、速率、增益与样本一次算好（⭐ 允许分配）。
    ///
    /// ⭐ **区域选择口径 = `Region::matches_query`**（与离线侧、与 `Instrument::region_for_with`
    /// 同一份谓词）。门控（通道 / `trigger` / keyswitch / CC / `held_notes`）在键与力度之外
    /// **独立于** `(note, velocity)`，因此它的结果可以压成一次布尔判断：
    /// 取该区域键/力度两维**都命中**的一个探测点求 `matches_query`，答案就是门控本身；
    /// 门控不过 ⇒ 两张掩码清 0，该区域**永不**进回调的命中集。
    ///
    /// ⭐ 增益按 `total_gain()` 的乘积结构摊平（⛔ 不再冻结在 `(60, 100)`）：见
    /// [`PreparedRegion`] 的字段文档与 [`PreparedRegion::gain_at`]。
    ///
    /// ⚠ ⭐ **这是广播口径**（兼容重载）：**每一**个 region 的 `sample_index` 都是 `0`
    /// —— 键/力度映射**对**，但**音色内容错**（多 `sample=` 的区域会播同一份 PCM）。
    /// 读数 [`Self::sample_content_label`] 为 `broadcast(1)`。要"每区域各播自己"请用
    /// [`Self::prepare_with_samples`]。⛔ 本函数**不**被删：既有调用方与步 2 的逐位读数靠它。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]。
    pub fn prepare(
        instrument: &SfzInstrument,
        source: &InMemorySample,
        engine_hz: f32,
    ) -> Result<Self, SfzError> {
        // 广播：只有**一份**样本（路径身份为空串），所有区域都指它。
        let samples = vec![PreparedSample::from_in_memory(String::new(), source)];
        let sample_hz = source.sample_hz;
        let regions = flatten_regions(instrument, engine_hz, |_region| (0u16, sample_hz))?;
        Ok(Self {
            regions,
            samples,
            broadcast: true,
            silent_regions: 0,
            missing_samples: Vec::new(),
        })
    }

    /// ⭐ **构造期**摊平（**多采样**）：每个区域播**它自己的** `sample=`。
    ///
    /// # 口径（三条，都可机械判定）
    ///
    /// 1. **去重键**＝[`yeban_sfz::instrument::Region::sample_path`] 的**字符串**原样
    ///    （⛔ 不规范化、⛔ 不看文件内容）；容器 ＝ `BTreeMap` ⇒ 顺序确定 [MODEL-AST-003]。
    ///    **同一样本引两次只 materialize 一份**（[`Self::sample_count`] ＝ 1、
    ///    [`Self::pcm_bytes`] ＝ 一份的字节数）。提供表里同一路径出现两次时**首个为准**。
    /// 2. **解码在控制侧**：本函数只接**已经解码**的 [`InMemorySample`] ⇒ 引擎⛔ 不引
    ///    `symphonia`/`rubato`、⛔ 不做任何 I/O。调用方先用
    ///    [`SfzInstrument::referenced_sample_paths`] 取路径全集，再按
    ///    `crates/yeban-app/src/sfz_load.rs` 的形状真读 PCM WAV。
    /// 3. ⭐ ⭐ **缺样本 ⇒ 静音 ＋ 计数**（⛔ 不静默、⛔ **不发明内容**）：
    ///    `provided_samples` 里**没有**某个 `sample_path` 时 —— 该区域标
    ///    [`PreparedRegion::NO_SAMPLE`]（不产生任何采样输出，触发时**不建声部**）
    ///    并计入 [`Self::silent_region_count`]，路径进 [`Self::missing_sample_paths`]。
    ///    ⛔ **本函数不再"回退到 `samples[0]`"**：那会让缺样本的区域去播**别人的采样**
    ///    （听得出、读文件看不见的**内容错**）。⭐ 归属裁决（台账 R711）：**引擎出静音并计数**、
    ///    **合成替代内容是控制侧的决定**（想给替代样本 ⇒ 由调用方显式放进 `provided_samples`，
    ///    或先用 [`Self::prepare`] 明确声明"这一份就是全部区域的样本"）。
    ///
    /// ⚠ 样本数上限：`sample_index` 是 `u16` ⇒ 去重后至多 `65534` 份（超出时后续区域被记为
    /// `NO_SAMPLE`，⛔ 不 panic、⛔ 不回绕）。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]。
    pub fn prepare_with_samples(
        instrument: &SfzInstrument,
        provided_samples: &[(String, InMemorySample)],
        engine_hz: f32,
    ) -> Result<Self, SfzError> {
        // ① 提供表的**确定性索引**（BTreeMap）：同一路径出现两次 ⇒ 首个为准。
        let mut provided_index: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, (path, _sample)) in provided_samples.iter().enumerate() {
            provided_index.entry(path.as_str()).or_insert(index);
        }
        // ② 样本表按**区域引用顺序**去重建立（⛔ 不预置"回退目标"：缺样本不回退）。
        let mut samples: Vec<PreparedSample> = Vec::new();
        let mut index_of: BTreeMap<String, u16> = BTreeMap::new();
        let mut missing: Vec<String> = Vec::new();
        let mut silent_regions = 0usize;
        let regions = flatten_regions(instrument, engine_hz, |region| {
            let path = region.sample_path();
            // 已 materialize 过 ⇒ 直接复用（**去重**：同一路径只解码/只 clone 一次）。
            if let Some(index) = index_of.get(path.as_ref()) {
                return (*index, samples[usize::from(*index)].sample_hz);
            }
            // 提供表里有这份路径 ⇒ materialize 一份新的（按**首次引用顺序**编号）。
            if let Some(provided) = provided_index.get(path.as_ref()).copied() {
                let (path, source) = &provided_samples[provided];
                let index = u16::try_from(samples.len()).unwrap_or(PreparedRegion::NO_SAMPLE - 1);
                samples.push(PreparedSample::from_in_memory(path.clone(), source));
                let sample_hz = samples[usize::from(index)].sample_hz;
                index_of.insert(path.clone(), index);
                return (index, sample_hz);
            }
            // ⭐ 缺样本：**静音 ＋ 计数**（⛔ 不静默、⛔ 不去播别人的采样）。
            missing.push(path.clone().into_owned());
            silent_regions += 1;
            // 没有样本 ⇒ 速率表用引擎采样率（该区域**永不渲染**，读数不进结论）。
            (PreparedRegion::NO_SAMPLE, engine_hz)
        })?;
        missing.sort();
        missing.dedup();
        Ok(Self {
            regions,
            samples,
            broadcast: false,
            silent_regions,
            missing_samples: missing,
        })
    }

    /// 摊平占用的**内存字节数**（读数用：区域表 ＋ 样本 PCM ＋ 样本路径文本）。
    ///
    /// ⚠ `path` 只算 `len()`（⛔ 不含 `String` 的容量余量）；广播路径是空串 ⇒ 0。
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        core::mem::size_of::<PreparedRegion>() * self.regions.len()
            + self
                .samples
                .iter()
                .map(|sample| sample.pcm_bytes() + sample.path.len())
                .sum::<usize>()
    }

    /// 区域数（摊平后）。
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// 摊平后的区域表（**只读**；判据与控制侧读数用）。
    #[must_use]
    pub fn regions(&self) -> &[PreparedRegion] {
        &self.regions
    }

    /// ⭐ 摊平后的**样本表**（去重后；**只读借用**：实时侧⛔ 不 clone）。
    #[must_use]
    pub fn samples(&self) -> &[PreparedSample] {
        &self.samples
    }

    /// ⭐ 单份样本（`sample_index` 之外 / `NO_SAMPLE` ⇒ `None`）。
    #[must_use]
    pub fn sample(&self, index: u16) -> Option<&PreparedSample> {
        if index == PreparedRegion::NO_SAMPLE {
            return None;
        }
        self.samples.get(usize::from(index))
    }

    /// ⭐ **样本数（去重后）＝ 摊平期的样本 clone 次数**（"解码次数"的机械读数）：
    /// 同一路径引两次 ⇒ `1`。
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// ⭐ **全部样本的 PCM 字节数**（去重后；⛔ 不含路径文本）。
    #[must_use]
    pub fn pcm_bytes(&self) -> usize {
        self.samples
            .iter()
            .map(PreparedSample::pcm_bytes)
            .sum::<usize>()
    }

    /// ⭐ 是否走**单样本广播**重载（[`Self::prepare`]）⇒ ⛔ 音色内容错（模块头 ⑨）。
    #[must_use]
    pub const fn is_broadcast(&self) -> bool {
        self.broadcast
    }

    /// ⭐ 缺样本 ⇒ 标 [`PreparedRegion::NO_SAMPLE`]（**不产生采样输出**）的**区域数**。
    ///
    /// ⚠ 这就是"缺样本"的**唯一**计数口径（⛔ 不再有第二个"回退"计数）：
    /// 引擎只出静音 ＋ 计数、⛔ 不发明替代内容（归属裁决见台账 R711）。
    #[must_use]
    pub const fn silent_region_count(&self) -> usize {
        self.silent_regions
    }

    /// ⭐ 缺样本的**路径**（排序 ＋ 去重）。
    #[must_use]
    pub fn missing_sample_paths(&self) -> &[String] {
        &self.missing_samples
    }

    /// ⭐ **样本内容读数**（报告用）：
    /// `broadcast(N)` / `distinct(N)`，缺样本时追加 `+silent(M)`。
    #[must_use]
    pub fn sample_content_label(&self) -> String {
        let mut label = if self.broadcast {
            format!("broadcast({})", self.samples.len())
        } else {
            format!("distinct({})", self.samples.len())
        };
        if self.silent_regions > 0 {
            label.push_str(&format!("+silent({})", self.silent_regions));
        }
        label
    }

    /// ⭐ **回调侧渲染**：把一个音符混进 `out`，返回写入帧数。**零分配**（只读 + 原地写）。
    ///
    /// ⭐ **多匹配语义 = `all`**：匹配的每条区域都叠加（与离线
    /// [`SfzInstrument::render_note_into`] 同一口径）。返回值 = 各匹配区域写入帧数的
    /// **最大值**（⛔ 不是各区域之和）。
    ///
    /// ⭐ **每区域播自己那份样本**（[`PreparedRegion::sample_index`]）；`NO_SAMPLE` 的区域跳过。
    ///
    /// 增益的相乘顺序与离线 `PlaybackSpec::total_gain()` **逐位相同**：
    /// `(volume × 力度) × ((键轴 × 力度淡入) × 力度淡出)`。
    /// ⭐ **`span`（`offset`/`end`）已应用**（步 3）：源位置从 `span.start` 起、
    /// 在 `min(span.end, PCM 长度)` 停（[`PreparedRegion::span_bounds`] ⇒ [`reduce_span`]）
    /// ⇒ 与离线侧、与实时声部的**逐样本最大绝对差为 0**，且覆盖**整段**（含显式 `end`）。
    pub fn render_block_into(&self, note: u8, velocity: u8, out: &mut [f32]) -> usize {
        let n = note as usize;
        let mut written = 0usize;
        for region in &self.regions {
            if !region.accepts(note, velocity) {
                continue;
            }
            let Some(sample) = self.sample(region.sample_index) else {
                continue;
            };
            let pcm = sample.pcm();
            let rate = region.rate_per_note[n];
            if !(rate.is_finite() && rate > 0.0) || pcm.is_empty() {
                continue;
            }
            let Some((start, source_end)) = region.span_bounds(pcm.len()) else {
                continue;
            };
            let gain = region.gain_at(note, velocity);
            let mut position = start;
            // 摊平表的一次性调用**不窃取**（与上一条路同款：`× 1.0` 逐位不变）。
            let mut fade = FadeRamp::NONE;
            let region_written =
                mix_span_into(pcm, &mut position, rate, gain, source_end, &mut fade, out);
            written = written.max(region_written);
        }
        written
    }
}

/// ⭐ **区域摊平的唯一实现**（构造期）：把一条 SFZ 的每条区域变成 [`PreparedRegion`]。
///
/// `resolve` 决定"这条区域用哪份样本"：
/// * [`PreparedSfz::prepare`] 传常数 `(0, source.sample_hz)`（**广播**）；
/// * [`PreparedSfz::prepare_with_samples`] 传路径查表（**多采样** ＋ 去重；缺样本 ⇒ 静音 ＋ 计数）。
///
/// ⭐ 每个区域的**速率表**用**它自己那份样本**的采样率换算（`RenderRates::new(sample_hz, engine_hz)`）
/// ⇒ 多采样下不同源采样率的样本各自换算正确（⛔ 不是全表一个 rates）。
fn flatten_regions(
    instrument: &SfzInstrument,
    engine_hz: f32,
    mut resolve: impl FnMut(&Region<'_>) -> (u16, f32),
) -> Result<Vec<PreparedRegion>, SfzError> {
    instrument.with_instrument(|parsed| {
        let mut regions = Vec::with_capacity(parsed.regions().len());
        for region in parsed.regions() {
            let (sample_index, sample_hz) = resolve(region);
            let rates = RenderRates::new(sample_hz, engine_hz);
            let mut key_mask = [0u64; 2];
            let mut vel_mask = [0u64; 2];
            let mut rate_per_note = [0.0f32; 128];
            for note in 0u16..128 {
                let note_u8 = note as u8;
                if region.matches_key(note_u8) {
                    key_mask[note as usize / 64] |= 1u64 << (note % 64);
                }
                rate_per_note[note as usize] = region.playback_rate(note_u8, rates);
            }
            for velocity in 0u16..128 {
                if region.matches_velocity(velocity as u8) {
                    vel_mask[velocity as usize / 64] |= 1u64 << (velocity % 64);
                }
            }
            // ⭐ 门控：取"键与力度都命中"的一个探测点 ⇒ `matches_query` 的答案只剩
            // 通道 / trigger / keyswitch / CC 这些与 (note, velocity) 无关的门控。
            // 门控不过 ⇒ 掩码清 0（`trigger=release` 的 region 因此不进 note-on 掩码）。
            let probe = first_set_bit(&key_mask).zip(first_set_bit(&vel_mask));
            let gate_open = probe.is_some_and(|(note, velocity)| {
                region.matches_query(&RegionQuery::new(note, velocity))
            });
            if !gate_open {
                key_mask = [0u64; 2];
                vel_mask = [0u64; 2];
            }

            // ⭐ 增益：`total_gain() = (volume × 力度) × 交叉淡化`，而交叉淡化是一条
            // 从左到右的乘法链：键轴段（in/out）→ 力度轴段（in/out）→ CC 段（**跳过**，
            // 因为回调侧没有 CC 状态，离线 `playback_spec` 也是 `cc = None`）。
            // 分开摊平键轴、力度淡入、力度淡出三张表，回调侧就能**逐位**还原该链。
            let mut key_crossfade = [1.0f32; 128];
            let mut velocity_fade_in = [1.0f32; 128];
            let mut velocity_fade_out = [1.0f32; 128];
            for crossfade in &region.crossfades {
                match crossfade.axis {
                    XfAxis::Key => {
                        for (note, slot) in key_crossfade.iter_mut().enumerate() {
                            *slot *= crossfade.gain_at(note as u8);
                        }
                    }
                    XfAxis::Velocity => {
                        let table = match crossfade.direction {
                            XfDirection::In => &mut velocity_fade_in,
                            XfDirection::Out => &mut velocity_fade_out,
                        };
                        for (velocity, slot) in table.iter_mut().enumerate() {
                            *slot *= crossfade.gain_at(velocity as u8);
                        }
                    }
                    // CC 轴：回调侧没有 CC 状态 ⇒ 与离线 `playback_spec` 一致地跳过
                    // （`Region::crossfade_gain` 对未提供状态的 CC 段就是 `continue`）。
                    XfAxis::Cc(_) => {}
                }
            }
            let mut velocity_gain = [0.0f32; 128];
            for (velocity, slot) in velocity_gain.iter_mut().enumerate() {
                *slot = region.velocity_gain(velocity as u8);
            }

            regions.push(PreparedRegion {
                key_mask,
                vel_mask,
                rate_per_note,
                key_crossfade,
                velocity_fade_in,
                velocity_fade_out,
                volume_gain: region.linear_gain(),
                velocity_gain,
                sample_index,
                span: region.playback_span(),
            });
        }
        regions
    })
}

/// ⭐ **窃取淡出的一段斜坡**（逐帧推进；`Copy`、定长、⛔ 无指针）。
///
/// 形状与 [`StealFade`](yeban_sfz::voice_pool::StealFade) 的 `gain_at` **同一条**
/// （`FLOOR^(n/N)`，`−60 dBFS` 收底）：第 0 帧 `gain == 1.0`，第 `N−1` 帧约 `1.2e-3`，
/// 走完 `N` 帧后归 `false` 并把 `gain` 置 `0.0`（⇒ 与接管者之间不留跃变）。
///
/// ⚠ **非淡出**用 [`FadeRamp::NONE`]（`decay == 1.0`）：逐帧那个乘法是 `× 1.0`，
/// 在 binary32 下**逐位不变** ⇒ 离线/摊平那条路与接线前的读数一字不差。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FadeRamp {
    /// 是否仍在淡出（`false` ⇒ [`Self::NONE`] 语义，逐帧增益恒 `1.0`）。
    fading: bool,
    /// 剩余淡出帧数（`fading == true` 时恒 `>= 1`）。
    remaining: u32,
    /// 当前逐帧增益（非淡出恒 `1.0`）。
    gain: f32,
    /// 每帧乘的衰减因子 `FLOOR^(1/N)`（触发期算好 ⇒ ⛔ 逐样本无 `powf`）。
    decay: f32,
}

impl FadeRamp {
    /// **不淡出**（`decay == 1.0` ⇒ 逐帧乘法是恒等）。
    pub const NONE: Self = Self {
        fading: false,
        remaining: 0,
        gain: 1.0,
        decay: 1.0,
    };

    /// 按**帧数**构造一段 3 ms 指数淡出；`frames == 0` ⇒ [`Self::NONE`]（硬窃取）。
    ///
    /// `decay = STEAL_FADE_FLOOR^(1/frames)` 用 `f64` 算一次（**触发期**，
    /// 与 PolySynth 的 `note_on` 允许 `exp` 同一条纪律）⇒ 逐样本只有一次乘法。
    #[must_use]
    pub fn new(frames: u32) -> Self {
        if frames == 0 {
            return Self::NONE;
        }
        let decay = f64::from(STEAL_FADE_FLOOR).powf(1.0 / f64::from(frames)) as f32;
        Self {
            fading: true,
            remaining: frames,
            gain: 1.0,
            decay,
        }
    }

    /// 是否仍在淡出（`false` ⇒ 未淡出**或**已走完）。
    #[must_use]
    pub const fn is_fading(&self) -> bool {
        self.fading
    }

    /// 剩余淡出帧数（非淡出恒 `0`）。
    #[must_use]
    pub const fn remaining(&self) -> u32 {
        self.remaining
    }

    /// 推进一帧；返回 `false` 表示**淡出已走完**（调用方应交出剩余帧）。
    fn advance(&mut self) -> bool {
        self.remaining = self.remaining.saturating_sub(1);
        self.gain *= self.decay;
        if self.remaining == 0 {
            self.gain = 0.0;
            self.fading = false;
            false
        } else {
            true
        }
    }
}

/// ⭐ **全仓唯一的逐样本混音循环**（线性插值 ＋ 增益乘加 ＋ 源采样终点）。
///
/// `position` 是**可变**的源采样位置 ⇒ 无状态的一次性调用
/// （[`PreparedSfz::render_block_into`]）与跨量子延续的实时声部
/// （[`SfzVoicePool`]）走**同一段代码**（本仓"不许两份实现"的机械纪律；
/// 两份实现在 binary32 下无法维持"逐位相同"）。
///
/// 停止条件（写每个样本**之前**）：`floor(position) + 1 >= source_end` ——
/// 线性插值需要 `position` 与 `position + 1` 两个样本。
/// `source_end` 由调用方给出（⭐ 三条路都是 [`reduce_span`] 的归约结果）：
/// * 摊平表的一次性调用给 [`PreparedRegion::span_bounds`] 的终点（`span` 已应用）；
/// * 实时声部给同一个 [`PreparedRegion::span_bounds`]（触发期抄下）；
/// * 离线 [`SfzInstrument::render_note_into`] 给同一份归约。
///
/// ⭐ `fade` 是**窃取淡出**：非淡出传 [`FadeRamp::NONE`]（`× 1.0` 逐位不变）；
/// 淡出中**写满 `remaining` 帧就返回**（`written == remaining`）⇒ 调用方可以就地
/// 提升挂起声部、用**同一段**代码继续写剩下的帧。
///
/// ⚠ **零分配、零锁、零 I/O**：只读 `pcm`、原地写 `out` [MUST-GATE-001]。
pub(crate) fn mix_span_into(
    pcm: &[f32],
    position: &mut f32,
    rate: f32,
    gain: f32,
    source_end: u64,
    fade: &mut FadeRamp,
    out: &mut [f32],
) -> usize {
    let mut written = 0usize;
    for slot in out.iter_mut() {
        let index = *position as usize;
        if index as u64 + 1 >= source_end || index + 1 >= pcm.len() {
            break;
        }
        let frac = *position - index as f32;
        let sample = pcm[index] * (1.0 - frac) + pcm[index + 1] * frac;
        // 非淡出时 `fade.gain == 1.0` ⇒ `(sample × gain) × 1.0` 与接线前**逐位相同**。
        *slot += sample * gain * fade.gain;
        *position += rate;
        written += 1;
        if fade.fading && !fade.advance() {
            break;
        }
    }
    written
}

/// 每轨的 **SFZ 声部数上限**（定长池）。
///
/// ⚠ 与 `yeban-sfz` 的**全局 512 声部** `VoicePool` 是**两套形状**：那个池管
/// `group`/`off_by`/`note_polyphony` 一类语义，且它的 `note_on` 走**立即接管**
/// （把 victim 句柄交还调用方、由渲染器对残余输出淡出）；本池只管"本轨有哪几个
/// 采样在响"，victim 选择与淡出都在池**内**（见 [`SfzVoicePool::select_victim`]）。
/// ⚠ 那个池**不被引擎使用**（引擎只从 `yeban-sfz` 借区域选择/速率/增益/`StealFade`）
/// ⇒ 它的 `select_victim` 是**它自己那一份**，⛔ 不构成"引擎里有两份实现"。
pub const SFZ_VOICES_PER_TRACK: usize = 16;

/// ✂️ **实时侧的一个 SFZ 声部**：定长、`Copy`、无指针 [ARCH-RT-001]。
///
/// 一个声部 ＝ **一条匹配区域**（SFZ 里每个 `region` 是一个独立声部 ⇒ 多匹配 ＝ 叠加）。
/// 触发时（[`SfzVoicePool::trigger_note`]）把摊平表里的标量**预计算**进来：
///
/// * `rate` ＝ `region.rate_per_note[note]`（速率预计算 ⇒ ⛔ 逐样本不做除法/`exp`）；
/// * `gain` ＝ `region.gain_at(note, velocity)`（**乐器层**，与摊平口径**逐位相同**）
///   **×** `track_gain`（**引擎层**的轨增益门，归属裁决见台账 `R711`）
///   ⇒ ⭐ 两者**相乘、⛔ 不重复**（引擎⛔ 不碰力度）；
/// * `sample_index` ＝ [`PreparedRegion::sample_index`]（⭐ 步 3：多采样下读**自己**那份 PCM）；
/// * `source_start` / `source_end` ＝ [`PreparedRegion::span_bounds`] 的**同一份**归约
///   （`offset` 起点与 `min(end, 样本播完)` 终点）。
///
/// # 终点口径（**无包络时的诚实占位**）
///
/// `stop = min(end_sample, source_end, 样本播完)`，其中
///
/// * ⛔ **跳过 `ampeg_*`**（ADSR / 力度包络本片不做）⇒ 音符**没有**包络收尾；
/// * ⭐ **用样本自然结束**：源位置走到 `source_end` 即退休；
/// * `end_sample`（音符的引擎终点）先到也退休。
///
/// ⛔ 本片不做循环、不做 `off_mode`（模块头 sfz-needs-6）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfzVoice {
    /// 源采样位置（帧；浮点 ⇒ 分数速率与线性插值）。
    ///
    /// ⭐ 触发时初始化为 [`reduce_span`] 归约出的**源起点**（`offset`，步 3）
    /// ⇒ `offset=N` 的声部从源采样 `N` 起播，⛔ 不是从 0。
    position: f32,
    /// 每样本的源位置增量（触发时从摊平表抄来）。
    rate: f32,
    /// 逐样本增益（触发时算好；见 [`PreparedRegion::gain_at`]）。
    gain: f32,
    /// 触发该声部的音高（诊断与判据用）。
    note: u8,
    /// 触发该声部的力度（诊断与判据用）。
    velocity: u8,
    /// 是否在发声（`false` ⇒ 逐样本循环整段跳过）。
    active: bool,
    /// ⭐ **该声部播哪一份摊平样本**（[`PreparedSfz::samples`] 的下标；触发时抄自
    /// [`PreparedRegion::sample_index`]）⇒ 多采样下各声部读**自己**那份 PCM。
    sample_index: u16,
    /// **源采样终点（半开）**：见类型文档的"终点口径"。
    source_end: u64,
    /// **引擎侧终点**（绝对样本位置）：来自 [`crate::synth::ScheduledNote::end_sample`]。
    end_sample: u64,
}

impl SfzVoice {
    /// 空闲声部（哨兵；`active == false` ⇒ 不参与渲染）。
    pub const EMPTY: Self = Self {
        position: 0.0,
        rate: 0.0,
        gain: 0.0,
        note: 0,
        velocity: 0,
        active: false,
        sample_index: PreparedRegion::NO_SAMPLE,
        source_end: 0,
        end_sample: 0,
    };

    /// 组装一个**在发声**的声部（**触发期**调用；⛔ 逐样本路径不调用）。
    ///
    /// `source_start` ＝ [`reduce_span`] 归约出的源起点（`offset`）；`source_end` ＝ 同一份
    /// 归约的终点。两者与摊平表/离线侧**同源** ⇒ 两侧逐样本相同。
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        note: u8,
        velocity: u8,
        rate: f32,
        gain: f32,
        sample_index: u16,
        source_start: f32,
        source_end: u64,
        end_sample: u64,
    ) -> Self {
        Self {
            position: source_start,
            rate,
            gain,
            note,
            velocity,
            active: true,
            sample_index,
            source_end,
            end_sample,
        }
    }

    /// 是否在发声。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// 触发该声部的音高。
    #[must_use]
    pub const fn note(&self) -> u8 {
        self.note
    }

    /// 触发该声部的力度。
    #[must_use]
    pub const fn velocity(&self) -> u8 {
        self.velocity
    }

    /// ⭐ 该声部播哪一份摊平样本（[`PreparedSfz::samples`] 的下标）。
    #[must_use]
    pub const fn sample_index(&self) -> u16 {
        self.sample_index
    }

    /// 当前源采样位置（帧）。
    #[must_use]
    pub const fn source_position(&self) -> f32 {
        self.position
    }

    /// 源采样终点（半开）。
    #[must_use]
    pub const fn source_end(&self) -> u64 {
        self.source_end
    }

    /// 引擎侧终点（绝对样本位置）。
    #[must_use]
    pub const fn end_sample(&self) -> u64 {
        self.end_sample
    }

    /// 释放（退休）。⛔ 不是堆释放：只是把状态翻回哨兵 [红线 7]。
    pub(crate) fn silence(&mut self) {
        *self = Self::EMPTY;
    }
}

/// 每轨的 **SFZ 声部池**：定长 `[SfzVoice; 16]`，**全 `Copy`** [ARCH-RT-001]。
///
/// # 纪律（与红线 7 配套）
///
/// * **触发**（[`Self::trigger_note`]）：只在**已有数组**里找最低空闲槽 ⇒ 零分配；
/// * **渲染**（[`Self::render_into`]）：只读 `&PreparedSfz`、原地写 `out` ⇒ 零分配；
/// * **退休**是**池内**状态翻转（`active = false`）⇒ ⛔ 不碰快照退役队列
///   （退役归属不变：旧快照仍由 `crate::snapshot` 的 `RetireProducer` 交回控制线程）；
/// * 池满（> [`SFZ_VOICES_PER_TRACK`] 条区域同时匹配）⇒ ⭐ **窃取**（⛔ 不是丢弃）：
///   按 [`Self::select_victim`] 的三段键选 victim，victim 在自己的槽里走
///   [`FadeRamp`]（3 ms 指数淡出），新声部**挂起**在同一槽、淡出走完即接管
///   （[`Self::pending_count`]）；⛔ 不 panic、⛔ 不扩容。
///   ⭐ **真丢弃**只剩一种情形：某槽的挂起者还没轮到发声就被**再一次**窃取换掉
///   ⇒ 计入 [`Self::dropped`]（累计、单调），⛔ 不静默。
///
/// # ⭐ victim 选择口径（**引擎既有那一套**，与 `yeban_dsp::polysynth::PolySynth` 同形）
///
/// 排序键（字典序，取最小者，见 [`Self::select_victim`]）：
/// `(层级, 触发序号, 槽位下标)`。层级 `0` ＝ **正在淡出**（即 PolySynth 的"已在
/// Release **或**电平 < −60 dBFS"那一档；本池⛔ 无包络、也⛔ 无电平注入 ⇒ 只剩
/// "已在淡出"这一条可判），层级 `1` ＝ 其余在响的声部；同层级取**触发最早**者、
/// 仍并列取**下标最小**者。三级合起来是全序 [ARCH-DET-001]。
///
/// ⚠ `yeban_sfz::voice_pool::VoicePool::select_victim` 是**另一份**（它自己那份：
/// `(层级, 注入电平 dBFS, 触发序号, 下标)`，且走**立即接管**把 victim 句柄交还调用方）
/// —— 那个池**不被引擎使用**（见 [`SFZ_VOICES_PER_TRACK`]）⇒ ⛔ 引擎里没有"两份实现"。
///
/// ⭐ 本条同时裁决了模块头登记的那处"立即接管 vs 额外 slack 槽"：**slack 就在槽内**
/// （`pending` 与 victim 共用同一个槽，与 `PolySynth` 的 `PendingNote` 同款）
/// ⇒ 定容池不需要第 17 个槽，⛔ 也不是"立即接管"。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfzVoicePool {
    /// 定长声部表（下标稳定 ⇒ 触发分配是确定性的：取最低空闲槽 [ARCH-DET-001]）。
    voices: [SfzVoice; SFZ_VOICES_PER_TRACK],
    /// 每槽**挂起**的待接管声部（淡出窗口里与 victim 共用一个槽 ⇒ 定容池也能同时
    /// 容纳"正在淡出的旧声部"与"刚开始的新声部"，⛔ 不需要额外的 slack 槽）。
    pending: [SfzVoice; SFZ_VOICES_PER_TRACK],
    /// 该槽是否有挂起声部。
    pending_active: [bool; SFZ_VOICES_PER_TRACK],
    /// 每槽的窃取淡出状态（非淡出恒 [`FadeRamp::NONE`]）。
    fade: [FadeRamp; SFZ_VOICES_PER_TRACK],
    /// 每槽最近一次触发的序号（越大越晚；victim 键的第二段）。
    order: [u64; SFZ_VOICES_PER_TRACK],
    /// 下一个触发序号（单调递增、`wrapping_add`）。
    order_counter: u64,
    /// 窃取淡出帧数（默认 3 ms × 采样率；`0` ＝ 硬窃取，判据可注入）。
    steal_fade_frames: u32,
    /// 因池满被**丢弃**的声部数（**累计、单调递增**；与 [`Self::active_count`] 的量规配对）。
    dropped: u64,
    /// 池满时**窃取**（换掉别人）的声部数（**累计、单调递增**）。
    stolen: u64,
}

impl SfzVoicePool {
    /// 空池（⛔ 无分配：整个池是栈上/结构体内的定长数组）；淡出帧数按 `sample_rate` 定。
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            voices: [SfzVoice::EMPTY; SFZ_VOICES_PER_TRACK],
            pending: [SfzVoice::EMPTY; SFZ_VOICES_PER_TRACK],
            pending_active: [false; SFZ_VOICES_PER_TRACK],
            fade: [FadeRamp::NONE; SFZ_VOICES_PER_TRACK],
            order: [0; SFZ_VOICES_PER_TRACK],
            order_counter: 0,
            steal_fade_frames: steal_fade_frames_for(sample_rate),
            dropped: 0,
            stolen: 0,
        }
    }

    /// 空池（48 kHz 的 3 ms 淡出）；引擎侧用 [`Self::new`] 传**真实采样率**。
    #[must_use]
    pub fn empty() -> Self {
        Self::new(48_000.0)
    }

    /// 采样率变化（快照边界）：重算 3 ms 的帧数。
    ///
    /// ⚠ **在响的淡出窗口不重算** —— 它的帧数在窃取那一刻就定下了
    /// （与 `PolySynth` 的 `start_steal_fade` 同一条口径）。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.steal_fade_frames = steal_fade_frames_for(sample_rate);
    }

    /// 覆盖窃取淡出帧数（**判据/注入用**；`0` ＝ 硬窃取）。
    ///
    /// 产物路径由采样率决定；存在的理由是"淡出确实生效"这条判据必须能对着
    /// **硬窃取**变红（与 `PolySynth::set_steal_fade_frames` 同款）。
    pub fn set_steal_fade_frames(&mut self, frames: u32) {
        self.steal_fade_frames = frames;
    }

    /// 当前窃取淡出帧数（默认 3 ms × 采样率）。
    #[must_use]
    pub const fn steal_fade_frames(&self) -> u32 {
        self.steal_fade_frames
    }

    /// ⭐ **为一个音符建声部**：**每一条匹配区域**占一个槽（多匹配 ＝ 叠加）。
    ///
    /// 返回**本次占用空槽的声部数**（0 ＝ 没有区域匹配、匹配的区域不产生采样输出，
    /// 或池满走了窃取 —— 窃取占的是**别人**的槽，⛔ 不计入"新建"）。
    /// 覆盖度：池满时多出来的区域走 [`Self::steal`]（记 [`Self::stolen`]）；
    /// 只有"挂起者被换掉"才是真丢弃（记 [`Self::dropped`]，⛔ 不静默）。
    ///
    /// ⭐ 步 3：每个声部抄下**该区域自己**的 `sample_index`（多采样）与 `span` 归约出的
    /// **源起点**（`offset`）；`NO_SAMPLE` 的区域⛔ 不建声部。
    ///
    /// # ⭐ 轨增益门（`track_gain`；归属裁决见台账 R711）
    ///
    /// 声部增益 ＝ **乐器层**（`region.gain_at`：`volume_gain`／`velocity_gain[128]`／
    /// `key_crossfade`／`velocity_fade_*`／`amp_veltrack`）× **引擎层**（本参数：
    /// 可闻门 × 推子，与鼓机/复音拿到的那一份**同一个** `track_gain_scale` 值）。
    /// ⇒ ⭐ 两者**相乘、⛔ 不重复**：引擎层⛔ 不碰力度（那由乐器层拥有）。
    pub fn trigger_note(
        &mut self,
        plan: &PreparedSfz,
        note: u8,
        velocity: u8,
        end_sample: u64,
        track_gain: f32,
    ) -> usize {
        let n = note as usize;
        let mut created = 0usize;
        for region in plan.regions() {
            if !region.accepts(note, velocity) {
                continue;
            }
            // ⭐ 缺样本（`NO_SAMPLE`）⇒ 与 `span_bounds() == None` **同径**：⛔ 不建声部
            // （引擎⛔ 不发明替代内容 —— 那是控制侧的决定；归属裁决见台账 R711 的 `needs-7`）。
            let Some(sample) = plan.sample(region.sample_index) else {
                continue;
            };
            let pcm_len = sample.pcm().len();
            let rate = region.rate_per_note[n];
            if !(rate.is_finite() && rate > 0.0) || pcm_len == 0 {
                continue;
            }
            // ⭐ 起点/终点口径：同一份归约（`span() == None` / 空区间 ⇒ 不建声部）。
            let Some((source_start, source_end)) = region.span_bounds(pcm_len) else {
                continue;
            };
            // ⭐ 引擎层的轨增益门**乘在乐器层增益之外**（两个归属各一份 ⇒ ⛔ 不重复）。
            let gain = region.gain_at(note, velocity) * track_gain;
            let voice = SfzVoice::new(
                note,
                velocity,
                rate,
                gain,
                region.sample_index,
                source_start,
                source_end,
                end_sample,
            );
            match self.voices.iter().position(|voice| !voice.active) {
                Some(index) => {
                    self.order_counter = self.order_counter.wrapping_add(1);
                    self.order[index] = self.order_counter;
                    self.voices[index] = voice;
                    self.pending_active[index] = false;
                    self.fade[index] = FadeRamp::NONE;
                    created += 1;
                }
                // ⭐ 池满 ⇒ **窃取**（sfz-needs-2 的落地点）；⛔ 不再丢弃。
                None => self.steal(voice),
            }
        }
        created
    }

    /// ⭐ **池满时换掉一个声部**（`trigger_note` 的池满分支）。
    ///
    /// * **软窃取**（`steal_fade_frames > 0`，默认 3 ms）：victim 继续占着自己的槽、
    ///   进入 [`FadeRamp`] 淡出；新声部**挂起**在同一槽的 `pending` 上，淡出走完
    ///   由 [`Self::render_into`] 就地接管 ⇒ "旧声部淡出"与"新声部起音"**永不同时
    ///   发声**（不可能叠加爆音），且定容池不需要额外的槽。
    /// * **硬窃取**（帧数 `0`，仅注入路径）：立即接管，⛔ 不留淡出。
    ///
    /// ⚠ 若选中的 victim **已经有挂起者**（还没轮到发声），那个挂起者被换掉
    /// ⇒ 它是一次**真丢弃**：计入 [`Self::dropped`]（⛔ 不静默）。
    fn steal(&mut self, voice: SfzVoice) {
        let Some(victim) = self.select_victim() else {
            // 不可达：容量 ≥ 1 且"池满"意味至少一个槽 active。
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        self.order_counter = self.order_counter.wrapping_add(1);
        let order = self.order_counter;
        let fade = FadeRamp::new(self.steal_fade_frames);
        self.stolen = self.stolen.saturating_add(1);
        if !fade.is_fading() {
            self.voices[victim] = voice;
            self.order[victim] = order;
            self.pending_active[victim] = false;
            self.fade[victim] = FadeRamp::NONE;
            return;
        }
        if self.pending_active[victim] {
            // 上一位挂起者还没发声就被换掉 ⇒ 真丢弃（⛔ 不静默）。
            self.dropped = self.dropped.saturating_add(1);
        }
        self.pending[victim] = voice;
        self.pending_active[victim] = true;
        self.order[victim] = order;
        self.fade[victim] = fade;
    }

    /// ⭐ **确定性窃取目标**（同一输入序列永远同一结果 [ARCH-DET-001]）。
    ///
    /// 排序键（字典序，取最小者；见类型文档的"victim 选择口径"）：
    /// `(层级, 触发序号, 槽位下标)`；层级 `0` ＝ 正在淡出，`1` ＝ 其余。
    /// 全程线性扫描、零分配；池空 ⇒ `None`（调用方按"不可达"计数，⛔ 不 panic）。
    #[must_use]
    pub fn select_victim(&self) -> Option<usize> {
        let mut best: Option<(u8, u64, u32)> = None;
        let mut best_index = None;
        for (index, voice) in self.voices.iter().enumerate() {
            if !voice.active {
                continue;
            }
            // `0` = 正在淡出（优先被换）；`1` = 其余在响的声部。
            let key = (
                u8::from(!self.fade[index].is_fading()),
                self.order[index],
                index as u32,
            );
            if best.is_none_or(|current| key < current) {
                best = Some(key);
                best_index = Some(index);
            }
        }
        best_index
    }

    /// 渲染本池的**全部活声部**到 `out`（`position` ＝ 本量子的**引擎绝对**起点）。
    ///
    /// **零分配**：只读 `plan` 的样本 PCM 与已有数组、原地写 `out` [MUST-GATE-001]。
    /// ⭐ 每个声部读**它自己**那份样本（[`SfzVoice::sample_index`]）。
    /// ⭐ 正在淡出的槽在**本块内**走完淡出时就地接管挂起者、用同一段
    /// [`mix_span_into`] 继续写剩下的帧（帧精确、⛔ 不留一个量子的空档）。
    pub fn render_into(&mut self, plan: &PreparedSfz, position: u64, out: &mut [f32]) {
        for index in 0..SFZ_VOICES_PER_TRACK {
            self.render_slot(index, plan, position, out);
        }
    }

    /// 渲染**一个槽**（`render_into` 的逐槽体；拆出来是为了借用清晰）。
    fn render_slot(&mut self, index: usize, plan: &PreparedSfz, position: u64, out: &mut [f32]) {
        let mut offset = 0usize;
        let mut victim_stopped = false;
        // --- 1) 淡出段：victim 在自己的窗口里继续发声，逐帧乘衰减增益 ---
        if self.voices[index].active && self.fade[index].is_fading() {
            let voice = self.voices[index];
            let limit = voice
                .end_sample
                .saturating_sub(position)
                .min(out.len() as u64) as usize;
            if limit == 0 {
                // 引擎终点先到 ⇒ 本槽（含挂起者）退休，⛔ 不静默地继续跑。
                self.silence_slot(index);
                return;
            }
            let Some(sample) = plan.sample(voice.sample_index) else {
                // 摊平表与池不匹配（例如热更了另一份表）⇒ 如实退休。
                self.silence_slot(index);
                return;
            };
            let mut local = voice.position;
            let mut fade = self.fade[index];
            let written = mix_span_into(
                sample.pcm(),
                &mut local,
                voice.rate,
                voice.gain,
                voice.source_end,
                &mut fade,
                &mut out[..limit],
            );
            self.voices[index].position = local;
            self.fade[index] = fade;
            offset = written;
            // 源/引擎终点在淡出走完之前先到 ⇒ victim 已经静音（见下面第 2 步）。
            victim_stopped = written < limit;
        }
        // --- 2) 提升：淡出走完 **或** victim 提前静音 ⇒ 挂起者接管这一槽 ---
        if self.pending_active[index] && (!self.fade[index].is_fading() || victim_stopped) {
            self.voices[index] = self.pending[index];
            self.pending_active[index] = false;
            self.fade[index] = FadeRamp::NONE;
        }
        // --- 3) 正常段：本槽现在的声部写本块剩余帧 ---
        let voice = self.voices[index];
        if !voice.active {
            return;
        }
        // 引擎终点（音符先到终点 ⇒ 本量子只渲染到它为止）。
        if voice.end_sample <= position.saturating_add(offset as u64) {
            self.silence_slot(index);
            return;
        }
        let remaining = voice
            .end_sample
            .saturating_sub(position)
            .min(out.len() as u64) as usize;
        let Some(sample) = plan.sample(voice.sample_index) else {
            self.silence_slot(index);
            return;
        };
        let mut local = voice.position;
        let mut fade = self.fade[index];
        let written = mix_span_into(
            sample.pcm(),
            &mut local,
            voice.rate,
            voice.gain,
            voice.source_end,
            &mut fade,
            &mut out[offset..remaining],
        );
        self.voices[index].position = local;
        self.fade[index] = fade;
        // ⭐ 用样本自然结束：源位置走完 ⇒ 退休（⛔ 无包络、⛔ 无循环）。
        if written < remaining - offset {
            self.silence_slot(index);
        }
    }

    /// 释放**一个**槽：声部与挂起者一起退休（淡出状态归 [`FadeRamp::NONE`]）。
    fn silence_slot(&mut self, index: usize) {
        self.voices[index].silence();
        self.pending_active[index] = false;
        self.fade[index] = FadeRamp::NONE;
    }

    /// 释放**全部**声部（`seek` / 换音源 ⇒ ⛔ 不留上一件音源的尾巴）。
    ///
    /// ⚠ 保留 [`Self::dropped`] / [`Self::stolen`]：它们是**累计**读数（与"活声部数"
    /// 这张量规配对），与 `DrumMachine::reset` 保留 `drum_hits` 同口径。
    pub fn silence(&mut self) {
        for index in 0..SFZ_VOICES_PER_TRACK {
            self.silence_slot(index);
        }
    }

    /// 活声部数（**量规**：可升可降；淡出中的声部**仍计**）。
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.voices.iter().filter(|voice| voice.active).count()
    }

    /// 挂起声部数（**量规**：某个槽正在淡出、新声部等接管时的占用数）。
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending_active.iter().filter(|active| **active).count()
    }

    /// 因池满被丢弃的声部数（**累计**，单调递增）。
    ///
    /// ⚠ 窃取**不**计入这里（那是 [`Self::stolen`]）；只有"挂起者被换掉"这类
    /// **真丢弃**才计入（⛔ 不静默）。
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// 池满时**窃取**（换掉别人）的声部数（**累计**，单调递增）。
    #[must_use]
    pub const fn stolen(&self) -> u64 {
        self.stolen
    }

    /// 某槽剩余的窃取淡出帧数（`0` ＝ 不在淡出；下标越界也返回 `0`）。
    #[must_use]
    pub fn fade_remaining_at(&self, index: usize) -> u32 {
        self.fade.get(index).map_or(0, |fade| {
            if fade.is_fading() {
                fade.remaining()
            } else {
                0
            }
        })
    }

    /// 只读声部表（判据用）。
    #[must_use]
    pub const fn voices(&self) -> &[SfzVoice; SFZ_VOICES_PER_TRACK] {
        &self.voices
    }
}

/// ✂️ **一条轨的 SFZ 投影**：控制侧构造期建好、实时侧只读。
///
/// 它存在的理由与 [`crate::insert::ConvolutionPlan`] 同一句：**模型层放不下样本数组**
/// （`ParameterValue` 只有 `name`/`value`/`unit` 三个字段）⇒ "SFZ 文本 → 区域表 ＋ PCM"
/// 的摊平必须发生在**控制线程的构造期**，结果随快照一起送到音频线程。
///
/// # 分配纪律（红线 7 / [MUST-GATE-001]）
///
/// * [`Self::prepare`] / [`Self::prepare_with_samples`]：**允许分配**（建区域表与样本 PCM）；
/// * ⛔ **不在快照边界摊平**：快照边界（`SynthEngine::begin_snapshot` / `arm_from_snapshot`）
///   一个字都不碰它 —— 那两条路径会分配（见 `crate::synth::SynthEngine::arm_from_snapshot`）；
/// * 实时侧只**借用** [`Self::prepared`] 的 `&PreparedSfz`（**PCM 借用快照**），
///   ⛔ 不 clone、⛔ 不 `Arc` clone 那些 `Vec`。
///
/// ⚠ **轨↔SFZ 的绑定不由模型给**（模块头 sfz-needs-5）⇒ 只能由引擎 API 显式注入
/// （[`crate::snapshot::EngineSnapshot::with_sfz`]）。⭐ 步 3 起这条公开路径**可用多采样**
/// 走通：`referenced_sample_paths` →（控制侧解码）→ [`Self::prepare_with_samples`] →
/// `EngineSnapshot::with_sfz`（判据见 `tests/sfz_instrument.rs` 的 S11/S12）。
#[derive(Clone, Debug, PartialEq)]
pub struct SfzPlan {
    /// 重数据：摊平后的区域表 ＋ PCM（控制侧构造、实时侧只读）。
    prepared: PreparedSfz,
}

impl SfzPlan {
    /// 控制侧摊平（**允许分配**）：见 [`PreparedSfz::prepare`] 的口径（**单样本广播**）。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]（与 [`PreparedSfz::prepare`] 同）。
    pub fn prepare(
        instrument: &SfzInstrument,
        source: &InMemorySample,
        engine_hz: f32,
    ) -> Result<Self, SfzError> {
        Ok(Self {
            prepared: PreparedSfz::prepare(instrument, source, engine_hz)?,
        })
    }

    /// ⭐ **控制侧多采样摊平**：把 `[(sample_path, 已解码样本)]` 交给
    /// [`PreparedSfz::prepare_with_samples`]（去重 ＋ 缺样本**静音** ＋ 计数）。
    ///
    /// 这是**绑定注入 API 的公开入口**：调用方建好本 plan 之后，用
    /// [`crate::snapshot::EngineSnapshot::with_sfz`] 把它注入某一条轨的快照 —— 全仓**唯一**
    /// 的"轨↔SFZ"绑定路径（模型层今天仍**没有**这条语义 ⇒ sfz-needs-5）。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]（与 [`PreparedSfz::prepare_with_samples`] 同）。
    pub fn prepare_with_samples(
        instrument: &SfzInstrument,
        samples: &[(String, InMemorySample)],
        engine_hz: f32,
    ) -> Result<Self, SfzError> {
        Ok(Self {
            prepared: PreparedSfz::prepare_with_samples(instrument, samples, engine_hz)?,
        })
    }

    /// 从**已经摊平**的表直接建（控制侧；判据与"摊平与控制侧分离"的场景用）。
    #[must_use]
    pub const fn from_prepared(prepared: PreparedSfz) -> Self {
        Self { prepared }
    }

    /// 实时侧只读的摊平表 —— `SynthEngine::render_track` 的 `sfz` 参数就是它。
    #[must_use]
    pub const fn prepared(&self) -> &PreparedSfz {
        &self.prepared
    }

    /// 区域数（控制侧读数）。
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.prepared.region_count()
    }

    /// 摊平占用的**内存字节数**（读数用：区域表 ＋ **全部**样本 PCM ＋ 样本路径文本）。
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.prepared.memory_bytes()
    }
}

/// 掩码里**第一个置位**的下标（`[u64; 2]` 的低半在前）⇒ `0..=127`；全 0 时 `None`。
///
/// 只在构造期调用（`prepare`），⛔ 不在回调路径上。
fn first_set_bit(mask: &[u64; 2]) -> Option<u8> {
    for (half, word) in mask.iter().enumerate() {
        if *word != 0 {
            let bit = word.trailing_zeros() as usize;
            return u8::try_from(half * 64 + bit).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小 SFZ：两条区域（键范围不同）。
    const MINIMAL: &str = "<region> sample=a.wav key=60\n<region> sample=b.wav key=64\n";

    /// [ROAD-M2-005] 判据 ①：`load` 一段最小文本 ⇒ `region_count` 等于期望值。
    #[test]
    fn load_records_the_region_count() {
        let instrument = SfzInstrument::load(MINIMAL.to_owned(), &ParseLimits::default())
            .expect("最小 SFZ 必须解析成功");
        assert_eq!(instrument.region_count(), 2, "两条 `<region>` ⇒ 区域数 2");
        // 单区域对照（⛔ 防止把"1"写死成常量也能过）
        let one = SfzInstrument::load(
            "<region> sample=a.wav key=60\n".to_owned(),
            &ParseLimits::default(),
        )
        .expect("单区域必须解析成功");
        assert_eq!(one.region_count(), 1, "单区域 ⇒ 1");
    }

    /// [ROAD-M2-005] 判据 ②：`with_instrument` 能在给定 note/velocity 下查到区域。
    #[test]
    fn regions_matching_selects_by_key_and_velocity() {
        let instrument =
            SfzInstrument::load(MINIMAL.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        assert_eq!(
            instrument.regions_matching(60, 100).expect("查询"),
            vec![0],
            "note 60 ⇒ 第 0 条"
        );
        assert_eq!(
            instrument.regions_matching(64, 100).expect("查询"),
            vec![1],
            "note 64 ⇒ 第 1 条"
        );
        assert!(
            instrument
                .regions_matching(72, 100)
                .expect("查询")
                .is_empty(),
            "无匹配键 ⇒ 空（⛔ 不是 panic、也不是默认第一条）"
        );
        // `with_instrument` 本身：闭包内能读到区域表
        let paths = instrument
            .with_instrument(|i| {
                i.regions()
                    .iter()
                    .map(|region| region.sample_path().to_string())
                    .collect::<Vec<_>>()
            })
            .expect("闭包式访问");
        assert_eq!(paths, vec!["a.wav".to_owned(), "b.wav".to_owned()]);
    }

    /// [ROAD-M2-005] 判据 ③：非法/超限 SFZ ⇒ 返回 [`SfzError`]，⛔ **不 panic**。
    ///
    /// ⭐ **实测登记（本节的重要读数）**：空文本、纯空白、以及坏指令 `<region> ???`
    /// 都被 `yeban-sfz` **接受**为 `Ok(0 区域)` —— 库把无法识别的 opcode 当作**忽略**而非错误。
    /// ⛔ 这不是我的缺陷、也不该由我在引擎侧"补一个错误"（那是**发明规范**）；
    /// 因此本判据分两半：
    ///   ① **不 panic**：三类退化输入都必须**返回**（`Ok` 或 `Err` 皆可）；
    ///   ② **必然报错**的机械证据：用**超限**输入（`max_line_bytes` 极小）⇒ 必须 `Err`。
    #[test]
    fn invalid_or_empty_sfz_returns_an_error_without_panicking() {
        // ① 退化输入：只要求"不 panic"（实测三者皆 Ok(0)，如实打印）
        for (label, text) in [
            ("空文本", ""),
            ("纯空白", "   \n\t\n"),
            ("坏指令", "<region> ???\n"),
        ] {
            let result = SfzInstrument::load(text.to_owned(), &ParseLimits::default());
            println!(
                "[sfz] {label}: {:?}",
                result.as_ref().map(SfzInstrument::region_count)
            );
            if let Ok(instrument) = result {
                assert_eq!(instrument.region_count(), 0, "{label} 若成功则区域数应为 0");
            }
        }
        // ② 超限输入 ⇒ **必须 Err**（库自己的判据也用这一口径）
        let tiny = ParseLimits {
            max_line_bytes: 8,
            ..ParseLimits::default()
        };
        let over = SfzInstrument::load("<region> sample=abcdefgh.wav key=60\n".to_owned(), &tiny);
        println!(
            "[sfz] 超限输入: {:?}",
            over.as_ref().map(SfzInstrument::region_count)
        );
        assert!(
            over.is_err(),
            "超限行必须返回 SfzError（这才是'非法 ⇒ 报错'的机械证据）"
        );
        // ③ 正常 limits 下同一条文本必须**成功**（反向对照：证明上面的 Err 来自 limits 而非文本本身）
        let normal = SfzInstrument::load(
            "<region> sample=abcdefgh.wav key=60\n".to_owned(),
            &ParseLimits::default(),
        );
        assert!(
            normal.is_ok(),
            "同一文本在默认 limits 下必须成功（否则上面的 Err 另有原因）"
        );
    }

    /// ⭐ 本任务的核心判据 a：**某音符触发 ⇒ 输出非静音**，并且与"不触发"**可区分**。
    ///
    /// 样本源是**内存里的极短正弦**（本仓尚无采样文件读取 ⇒ 样本由调用方提供）。
    /// 通路里的区域选择、播放速率、总增益**全部来自 `yeban-sfz`**（⛔ 引擎不重写）。
    #[test]
    fn triggering_a_region_produces_non_silence_and_is_distinguishable() {
        const SFZ: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n";
        let instrument = SfzInstrument::load(SFZ.to_owned(), &ParseLimits::default())
            .expect("最小 SFZ 必须成功");
        let sample = InMemorySample::sine(440.0, 48_000.0, 480);

        // ① 命中的音符（key=60）⇒ 非静音
        let mut hit = vec![0.0f32; 512];
        let written = instrument
            .render_note_into(&sample, 60, 100, &mut hit, 48_000.0)
            .expect("渲染不得报错");
        let energy: f32 = hit.iter().map(|s| s * s).sum();
        println!("[sfz] key=60: 写入 {written} 帧，能量 = {energy:.6}");
        assert!(written > 0, "命中区域必须写入样本帧（实得 {written}）");
        assert!(energy > 1e-6, "命中区域必须**非静音**（能量 {energy}）");

        // ② 不命中的音符（key=72）⇒ 静音 ⇒ ⭐ 与①**可区分**
        let mut miss = vec![0.0f32; 512];
        let written_miss = instrument
            .render_note_into(&sample, 72, 100, &mut miss, 48_000.0)
            .expect("不命中也不得报错");
        let energy_miss: f32 = miss.iter().map(|s| s * s).sum();
        println!("[sfz] key=72: 写入 {written_miss} 帧，能量 = {energy_miss:.6}");
        assert_eq!(written_miss, 0, "不命中的音符不得写入帧");
        assert_eq!(energy_miss, 0.0, "不命中的音符必须是静音");
        assert!(energy > energy_miss, "⭐ 可区分：命中的能量必须大于不命中");
    }

    /// ⭐ 新判据：**`trigger=release` 的区域不进 `note-on` 掩码**。
    ///
    /// 口径：摊平表建表时用 `Region::matches_query`（与 `Instrument::region_for_with`
    /// 同一个谓词）过门控；门控不过 ⇒ **两张掩码都清 0** ⇒ 回调侧的
    /// [`PreparedRegion::accepts`] 永远为假。离线侧同此口径。
    ///
    /// 反向对照：该 release 区域在**键/力度**上确实匹配（用 `matches_key`/`matches_velocity`
    /// 直接读回）⇒ 掩码全 0 只可能来自**门控**，⛔ 不是"它本来就不匹配"。
    #[test]
    fn trigger_release_never_enters_the_note_on_mask() {
        const SFZ: &str = "<region> sample=attack.wav key=60 pitch_keycenter=60\n\
                           <region> sample=release.wav key=60 pitch_keycenter=60 trigger=release\n";
        let instrument =
            SfzInstrument::load(SFZ.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        let sample = InMemorySample::sine(440.0, 48_000.0, 480);
        let prepared = PreparedSfz::prepare(&instrument, &sample, 48_000.0).expect("摊平");

        assert_eq!(
            prepared.region_count(),
            2,
            "两条区域都要摊平进来（⛔ 不是把 release 那条丢弃）"
        );
        let attack = &prepared.regions()[0];
        let release = &prepared.regions()[1];
        // ① 攻击区域：note-on 掩码里有键 60
        assert!(attack.accepts(60, 100), "attack 区域必须在 note-on 掩码里");
        assert_ne!(attack.key_mask(), [0u64; 2], "attack 的键掩码必须非空");
        // ② release 区域：两张掩码**全 0** ⇒ 永不进 note-on 命中集
        assert_eq!(
            release.key_mask(),
            [0u64; 2],
            "⭐ trigger=release 的键掩码必须全 0（不进 note-on 掩码）"
        );
        assert_eq!(
            release.velocity_mask(),
            [0u64; 2],
            "⭐ trigger=release 的力度掩码必须全 0"
        );
        assert!(
            !release.accepts(60, 100),
            "trigger=release 不得被 note-on 接受"
        );

        // ③ 反向对照：它**在键/力度上确实匹配** ⇒ 全 0 只能来自门控
        let raw_match = instrument
            .with_instrument(|parsed| {
                let region = &parsed.regions()[1];
                region.matches_key(60) && region.matches_velocity(100)
            })
            .expect("闭包式访问");
        assert!(
            raw_match,
            "该 release 区域在键/力度上**确实**匹配 ⇒ 掩码清 0 只能来自 trigger 门控"
        );

        // ④ 离线侧同一口径：只剩 release 区域时 note-on 必须**一帧都不写**
        let only_release = SfzInstrument::load(
            "<region> sample=release.wav key=60 pitch_keycenter=60 trigger=release\n".to_owned(),
            &ParseLimits::default(),
        )
        .expect("解析必须成功");
        let mut out = vec![0.0f32; 64];
        let written = only_release
            .render_note_into(&sample, 60, 100, &mut out, 48_000.0)
            .expect("渲染不得报错");
        assert_eq!(written, 0, "只有 release 区域时 note-on 必须写 0 帧");
        assert!(
            out.iter().all(|s| *s == 0.0),
            "只有 release 区域时 note-on 必须静音"
        );
        // 对照臂：同一份样本在 fixture（含 attack）里必须**非静音**，否则上面是空转
        let mut both = vec![0.0f32; 64];
        let written_both = instrument
            .render_note_into(&sample, 60, 100, &mut both, 48_000.0)
            .expect("渲染不得报错");
        assert!(
            written_both > 0 && both.iter().any(|s| *s != 0.0),
            "含 attack 区域的 fixture 必须非静音（否则本判据是空转）"
        );
    }

    /// ⭐ 判据（本片新增）：摊平增益 = `PlaybackSpec::total_gain`，**全 128×128 网格逐位相同**。
    ///
    /// 场景刻意同时给出**键轴**与**力度轴**的交叉淡化（in 与 out 都有），
    /// 用来钉住"键轴 / 力度淡入 / 力度淡出分开摊平、回调侧按同一结合序相乘"这条口径：
    /// 若把力度淡入与淡出并成一张表（`K * (in * out)`），binary32 下会与
    /// `(K * in) * out` 不同值 ⇒ 本判据会红。
    #[test]
    fn flattened_gain_equals_total_gain_over_the_whole_grid() {
        const XF: &str = "<region> sample=a.wav key=60 pitch_keycenter=60 volume=-3 \
             xfin_lokey=48 xfin_hikey=72 xfout_lokey=48 xfout_hikey=72 \
             xfin_lovel=40 xfin_hivel=90 xfout_lovel=60 xfout_hivel=110\n";
        let instrument =
            SfzInstrument::load(XF.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        let sample = InMemorySample::sine(440.0, 48_000.0, 480);
        let prepared = PreparedSfz::prepare(&instrument, &sample, 48_000.0).expect("摊平");
        let flat = &prepared.regions()[0];

        // 鉴别力自检：至少有一个轴在网格上真的改变了增益（⛔ 否则本判据对结合序无话可说）
        assert_ne!(
            flat.gain_at(60, 100),
            flat.gain_at(60, 40),
            "力度轴交叉淡化必须在网格上生效"
        );
        assert_ne!(
            flat.gain_at(60, 100),
            flat.gain_at(72, 100),
            "键轴交叉淡化必须在网格上生效"
        );

        let rates = RenderRates::new(48_000.0, 48_000.0);
        let (mismatches, first) = instrument
            .with_instrument(|parsed| {
                let region = &parsed.regions()[0];
                let mut mismatches = 0usize;
                let mut first: Option<(u8, u8, u32, u32)> = None;
                for n in 0u16..128 {
                    for v in 0u16..128 {
                        let want = region.playback_spec(n as u8, v as u8, rates).total_gain();
                        let got = flat.gain_at(n as u8, v as u8);
                        if want.to_bits() != got.to_bits() {
                            mismatches += 1;
                            if first.is_none() {
                                first = Some((n as u8, v as u8, want.to_bits(), got.to_bits()));
                            }
                        }
                    }
                }
                (mismatches, first)
            })
            .expect("闭包式访问");
        println!(
            "[sfz] 摊平增益 vs total_gain：128×128 网格里位型不同的点数 = {mismatches}（首个不同点 = {first:?}）"
        );
        assert_eq!(
            mismatches, 0,
            "⭐ 摊平增益必须与 total_gain **逐位相同**（首个不同点 = {first:?}）"
        );
    }

    /// ⭐ 判据（本片新增）：`offset`/`end` 归约出的 `SampleSpan` 已进摊平表。
    ///
    /// ⚠ 本判据**只证明字段被摊平登记**，⛔ 不证明渲染应用了它 —— 渲染仍未应用
    /// （见模块头文档的局限表第 ③ 条）。
    #[test]
    fn prepared_region_carries_the_source_span() {
        let instrument = SfzInstrument::load(
            "<region> sample=a.wav key=60 pitch_keycenter=60 offset=10 end=200\n\
             <region> sample=b.wav key=64 pitch_keycenter=60 end=-1\n"
                .to_owned(),
            &ParseLimits::default(),
        )
        .expect("解析必须成功");
        let sample = InMemorySample::sine(440.0, 48_000.0, 480);
        let prepared = PreparedSfz::prepare(&instrument, &sample, 48_000.0).expect("摊平");
        assert_eq!(
            prepared.regions()[0].span(),
            Some(SampleSpan {
                start: 10,
                end: Some(201)
            }),
            "offset=10 end=200（含端点）⇒ 半开区间 [10, 201)"
        );
        assert_eq!(
            prepared.regions()[1].span(),
            None,
            "end=-1 ⇒ 不产生采样输出（`SampleEnd::Silent`）"
        );
    }

    /// [MUST-GATE-001] 判据 ④：**本类型不在实时路径上** —— 机械部分。
    ///
    /// 机械可判的部分：`with_instrument` / `regions_matching` **必然分配**（重新解析 +
    /// `Vec`），因此它们**不可能**出现在实时回调里。这里用`Vec`返回值与"重新解析"两条
    /// 事实把它钉住；⭐ **"回调里只用摊平数据"这一条属于文档约束**，见模块头文档 ②
    /// （⛔ 机器暂时判不了"某函数有没有被回调调用"，故不假装有机械判据）。
    #[test]
    fn the_adapter_is_documented_as_control_side_only() {
        let instrument =
            SfzInstrument::load(MINIMAL.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        // `regions_matching` 返回 `Vec` ⇒ 一定有堆分配 ⇒ 不能进实时回调
        let selected: Vec<usize> = instrument.regions_matching(60, 100).expect("查询");
        assert_eq!(selected.len(), 1, "查询结果本身就是堆分配的 `Vec`");
        // 每次 `with_instrument` 都**重新解析** ⇒ 用"区域数恒定但地址不同"的间接证据说明
        // 每次调用都会重建区域表（控制侧可接受，实时侧不可接受）。
        let first = instrument
            .with_instrument(|i| i.regions().as_ptr() as usize)
            .expect("闭包");
        let second = instrument
            .with_instrument(|i| i.regions().as_ptr() as usize)
            .expect("闭包");
        println!("[sfz] 两次解析的区域表地址: {first} / {second}");
        assert_eq!(instrument.region_count(), 2, "区域数不随解析次数变化");
    }
}
