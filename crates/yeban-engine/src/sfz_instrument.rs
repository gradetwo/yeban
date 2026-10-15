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
//! | ① | 力度增益 | **本片已修**（每力度一张表，⛔ 不再冻结在 100） |
//! | ② | `trigger` 门控 | **本片已修**（走 `matches_query`；门控不过的区域掩码全 0） |
//! | ③ | 无 `offset`/`end` | **本片补了 `start`/`end`（`SampleSpan`）字段**，但渲染**仍未应用**它 —— 应用它必须**同时**改离线侧，否则两侧口径又会分叉（下一步） |
//! | ④ | 无 `group`/`off_by`/`note_polyphony`/`seq_` | 未做（语义由 `yeban-sfz` 裁、槽位与窃取由 `SynthEngine` 裁 ⇒ 属步 2） |
//! | ⑤ | 无字节预算、无流式 | 未做（PCM 整段 clone 进摊平表；无预算上限、无流式解码） |
//! | ⑥ | 同内容不同路径重复解码 | 未做（每个 region / 每条路径各自解码，⛔ 无去重） |
//! | ⑦ | CC 轴交叉淡化 | 未做（回调侧没有 CC 状态 ⇒ 与离线 `playback_spec` 一致地**跳过**该轴） |
//! | ⑧ | `loop_window` / `direction` / `pan` / `off_mode` | 未做（读数在 `PlaybackSpec` 里，摊平表未携带） |
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
//! ⚠ `span` 的**起点**（`offset`）与循环**本片都不做**（源位置从 `0.0` 起，
//! 与摊平表的一次性调用同口径）。
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
//! `PreparedSfz::render_block_into` 是**无状态的一次性调用**（每次从源位置 `0.0` 起播），
//! 且**尚未应用 `span`**（见上表 ③）。引擎声部是**有状态**的：它应用 `span` 终点、
//! 播完即退休。因此"逐样本最大绝对差 ＝ 0"这条判据**只覆盖触发量子**，且夹具必须是
//! **没有显式 `offset`/`end`** 的区域；两条的差别登记为 needs（见下表）。
//!
//! ## 本片新增的 needs（⛔ 不假装已有）
//!
//! | # | 缺口 | 为什么需要裁决 |
//! | :--- | :--- | :--- |
//! | sfz-needs-1 | **轨推子/静音没有乘到 SFZ 声部上** | 本片的逐样本对账口径是 `render_block_into(note, velocity)`（它**没有**轨增益参数）⇒ 声部增益＝`region.gain_at(...)`。接线轨增益要先裁"谁拥有力度增益"（SFZ 曲线的 vs `synth::velocity_gain`），否则会双重计力度。⚠ 与鼓机（乘 `ScheduledNote::gain`）**不对称** |
//! | sfz-needs-2 | **池满（> 16 条区域同时匹配）只丢弃 ＋ 计数，没有窃取口径** | `yeban-sfz` 的 `VoicePool` 有确定性窃取，但它是**全局 512 声部**、且 victim 选择是另一份口径 ⇒ 两套形状要一条 ADR（见 `docs/ledger/sfz-core-notes.md`） |
//! | sfz-needs-3 | **鼓机 ＋ SFZ 同轨的取舍**（本片＝鼓机优先，SFZ 池被释放） | 与 `crate::drums` 的 needs-3 同族：模型若禁止这种组合，本条删除 |
//! | sfz-needs-4 | **`span` 终点已在引擎侧应用、离线侧未应用** | 两侧口径分叉（离线 `render_note_into` / 摊平表 `render_block_into` 都仍从源 `0.0` 播到 PCM 末）⇒ 修它要**同时**改离线侧，属步 3 |
//! | sfz-needs-5 | **轨↔SFZ 的模型绑定不存在** | `yeban-model` 里 SFZ 命中只有一行注释（`model/src/lib.rs:16`）、`DeviceKind` 四档没有 sampler ⇒ 今天只能由引擎 API（[`crate::snapshot::EngineSnapshot::with_sfz`]）显式注入 |
//! | sfz-needs-6 | **循环 / ADSR / `off_mode` / 多采样 / 每区域独立 PCM** | 完全未做：本片一条轨共用**一份** PCM（摊平表只有 `pcm` 一个 `Vec`）⇒ 多 `sample=` 的区域会播同一份数据（步 3） |

use yeban_sfz::crossfade::{XfAxis, XfDirection};
use yeban_sfz::error::SfzError;
use yeban_sfz::instrument::{Instrument, RegionQuery};
// ⚠ 这两个是 **lib 构建**就需要的（`load` / `with_instrument` 都调用它们）⇒ ⛔ 不能放在 `#[cfg(test)]` 下，
// 否则非测试构建会报 "cannot find type `ParseLimits`"（本批实测过一次：三项门全红）。
use yeban_sfz::parser::{ParseLimits, parse_text};
use yeban_sfz::playback::{RenderRates, SampleSpan};

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
                if !(rate.is_finite() && rate > 0.0) || source.pcm.is_empty() {
                    continue;
                }
                let mut position = 0.0f32;
                let mut region_written = 0usize;
                for slot in out.iter_mut() {
                    let index = position as usize;
                    if index + 1 >= source.pcm.len() {
                        break;
                    }
                    let frac = position - index as f32;
                    let sample = source.pcm[index] * (1.0 - frac) + source.pcm[index + 1] * frac;
                    *slot += sample * gain;
                    position += rate;
                    region_written += 1;
                }
                written = written.max(region_written);
            }
            written
        })
    }
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
    /// ⭐ `offset` / `end` 归约出的**源采样区间**（半开 `[start, end)`，单位源采样帧）。
    ///
    /// `None` 表示该区域**不产生采样输出**（`end=-1`，或显式区间落在 `offset` 之前）。
    /// ⚠ **本片只把它摊平登记**，⛔ 渲染**尚未应用**（应用它必须同时改离线侧，
    /// 否则两侧口径会再次分叉）。
    span: Option<SampleSpan>,
}

impl PreparedRegion {
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

    /// ⭐ **源采样终点（半开）**：`min(span 终点, 样本播完)`。
    ///
    /// * [`Self::span`] 为 `None` ⇒ `None`：该区域**不产生采样输出**（`end=-1` / 空区间）
    ///   ⇒ 实时侧⛔ 不为它建声部；
    /// * `span.end` 未指定（`offset`/`end` 都没写）⇒ 取 `pcm_len`（"到采样末尾"）；
    /// * 显式 `end` 超出 PCM ⇒ 夹到 `pcm_len`（⛔ 不越界读）。
    ///
    /// ⚠ **只算终点**：`span` 的起点（`offset`）本片不应用（声部从源位置 `0.0` 起），
    /// 与 [`PreparedSfz::render_block_into`] 的起点口径一致。
    #[must_use]
    pub fn source_end(&self, pcm_len: usize) -> Option<u64> {
        let pcm_len = pcm_len as u64;
        self.span
            .map(|span| span.end.unwrap_or(pcm_len).min(pcm_len))
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

/// ✂️ **摊平后的乐器**：区域表 ＋ PCM，**构造期**建好，回调侧只读。
///
/// # 分配纪律（红线 7 / [MUST-GATE-001]）
///
/// * [`PreparedSfz::prepare`]：**允许分配**（区域表与 PCM 的 `Vec` 在构造期/控制侧建立）；
/// * [`PreparedSfz::render_block_into`]：**禁止分配** —— 它只做位测试、数组读取与
///   `out` 的原地写入（⛔ 无 `Vec::push`／`Box`／`String`／锁／I/O）。
///   违反的后果：实时回调内产生堆分配/释放或锁等待 ⇒ 可能造成音频线程不可预测的停顿
///   （红线 7 的一票否决项），由 `tests/rt_zero_alloc.rs` 的**计数型全局分配器**窗口判据检测。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSfz {
    regions: Vec<PreparedRegion>,
    pcm: Vec<f32>,
}

impl PreparedSfz {
    /// **构造期**摊平：把区域谓词、速率、增益与样本一次算好（⭐ 允许分配）。
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
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]。
    pub fn prepare(
        instrument: &SfzInstrument,
        source: &InMemorySample,
        engine_hz: f32,
    ) -> Result<Self, SfzError> {
        let rates = RenderRates::new(source.sample_hz, engine_hz);
        instrument.with_instrument(|parsed| {
            let mut regions = Vec::with_capacity(parsed.regions().len());
            for region in parsed.regions() {
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
                    span: region.playback_span(),
                });
            }
            Self {
                regions,
                pcm: source.pcm.clone(),
            }
        })
    }

    /// 摊平占用的**内存字节数**（读数用：区域表 ＋ PCM）。
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        core::mem::size_of::<PreparedRegion>() * self.regions.len()
            + core::mem::size_of::<f32>() * self.pcm.len()
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

    /// ⭐ 摊平后的 PCM（**只读借用**：实时侧⛔ 不 clone、⛔ 不 `Arc` clone 这个 `Vec`）。
    ///
    /// 本切片一条轨共用**一份** PCM（多 `sample=` 的区域会播同一份数据 ⇒ 见模块头
    /// sfz-needs-6）。
    #[must_use]
    pub fn pcm(&self) -> &[f32] {
        &self.pcm
    }

    /// ⭐ **回调侧渲染**：把一个音符混进 `out`，返回写入帧数。**零分配**（只读 + 原地写）。
    ///
    /// ⭐ **多匹配语义 = `all`**：匹配的每条区域都叠加（与离线
    /// [`SfzInstrument::render_note_into`] 同一口径）。返回值 = 各匹配区域写入帧数的
    /// **最大值**（⛔ 不是各区域之和）。
    ///
    /// 增益的相乘顺序与离线 `PlaybackSpec::total_gain()` **逐位相同**：
    /// `(volume × 力度) × ((键轴 × 力度淡入) × 力度淡出)`。
    /// 与离线侧的**逐样本最大绝对差**为 0（判据 `prepared_matches_offline_sample_by_sample`
    /// 钉住；`trigger` 门控与多匹配两片另有判据）。
    ///
    /// ⚠ **本函数不应用 [`PreparedRegion::span`]**（步 1 的已知局限 ③）：源侧终点取 PCM
    /// 长度 ⇒ 与接线前逐样本**逐位相同**。引擎实时声部**应用** `span` 终点
    /// （见 [`SfzVoice`] 与模块头的口径边界）。
    pub fn render_block_into(&self, note: u8, velocity: u8, out: &mut [f32]) -> usize {
        let n = note as usize;
        let mut written = 0usize;
        for region in &self.regions {
            if !region.accepts(note, velocity) {
                continue;
            }
            let rate = region.rate_per_note[n];
            if !(rate.is_finite() && rate > 0.0) || self.pcm.is_empty() {
                continue;
            }
            let gain = region.gain_at(note, velocity);
            let mut position = 0.0f32;
            let region_written = mix_span_into(
                &self.pcm,
                &mut position,
                rate,
                gain,
                self.pcm.len() as u64,
                out,
            );
            written = written.max(region_written);
        }
        written
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
/// `source_end` 由调用方给出：
/// * 摊平表的一次性调用给 `pcm.len()`（⛔ 不看 `span`，保持接线前的口径）；
/// * 实时声部给 [`PreparedRegion::source_end`]（`min(span 终点, pcm.len())`）。
///
/// ⚠ **零分配、零锁、零 I/O**：只读 `pcm`、原地写 `out` [MUST-GATE-001]。
pub(crate) fn mix_span_into(
    pcm: &[f32],
    position: &mut f32,
    rate: f32,
    gain: f32,
    source_end: u64,
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
        *slot += sample * gain;
        *position += rate;
        written += 1;
    }
    written
}

/// 每轨的 **SFZ 声部数上限**（定长池）。
///
/// ⚠ 与 `yeban-sfz` 的**全局 512 声部** `VoicePool` 是**两套形状**：那个池管
/// `group`/`off_by`/`note_polyphony` 一类语义与它自己的 victim 选择，本池只管
/// "本轨有哪几个采样在响" ⇒ 窃取口径要一条 ADR（模块头 sfz-needs-2）。
pub const SFZ_VOICES_PER_TRACK: usize = 16;

/// ✂️ **实时侧的一个 SFZ 声部**：定长、`Copy`、无指针 [ARCH-RT-001]。
///
/// 一个声部 ＝ **一条匹配区域**（SFZ 里每个 `region` 是一个独立声部 ⇒ 多匹配 ＝ 叠加）。
/// 触发时（[`SfzVoicePool::trigger_note`]）把摊平表里的标量**预计算**进来：
///
/// * `rate` ＝ `region.rate_per_note[note]`（速率预计算 ⇒ ⛔ 逐样本不做除法/`exp`）；
/// * `gain` ＝ `region.gain_at(note, velocity)`（与摊平口径**逐位相同**）；
/// * `source_end` ＝ [`PreparedRegion::source_end`]（`min(span 终点, 样本播完)`）。
///
/// # 终点口径（**无包络时的诚实占位**）
///
/// `stop = min(end_sample, 样本播完)`，其中
///
/// * ⛔ **跳过 `ampeg_*`**（ADSR / 力度包络本片不做）⇒ 音符**没有**包络收尾；
/// * ⭐ **用样本自然结束**：源位置走到 `source_end` 即退休；
/// * `end_sample`（音符的引擎终点）先到也退休。
///
/// ⛔ 本片不做循环、不做 `off_mode`、不做多采样（模块头 sfz-needs-6）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfzVoice {
    /// 源采样位置（帧；浮点 ⇒ 分数速率与线性插值）。
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
        source_end: 0,
        end_sample: 0,
    };

    /// 组装一个**在发声**的声部（**触发期**调用；⛔ 逐样本路径不调用）。
    #[must_use]
    pub const fn new(
        note: u8,
        velocity: u8,
        rate: f32,
        gain: f32,
        source_end: u64,
        end_sample: u64,
    ) -> Self {
        Self {
            position: 0.0,
            rate,
            gain,
            note,
            velocity,
            active: true,
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
/// * 池满（> [`SFZ_VOICES_PER_TRACK`] 条区域同时匹配）⇒ **丢弃并计数**
///   （[`Self::dropped`]）；⛔ 不 panic、⛔ 不扩容、⛔ 不发明窃取口径（sfz-needs-2）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfzVoicePool {
    /// 定长声部表（下标稳定 ⇒ 触发分配是确定性的：取最低空闲槽 [ARCH-DET-001]）。
    voices: [SfzVoice; SFZ_VOICES_PER_TRACK],
    /// 因池满被丢弃的声部数（**累计、单调递增**；与 [`Self::active_count`] 的量规配对）。
    dropped: u64,
}

impl SfzVoicePool {
    /// 空池（⛔ 无分配：整个池是栈上/结构体内的定长数组）。
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            voices: [SfzVoice::EMPTY; SFZ_VOICES_PER_TRACK],
            dropped: 0,
        }
    }

    /// ⭐ **为一个音符建声部**：**每一条匹配区域**占一个槽（多匹配 ＝ 叠加）。
    ///
    /// 返回**新建的声部数**（0 ＝ 没有区域匹配，或匹配的区域不产生采样输出）。
    /// 覆盖度：池满时多出来的区域被丢弃并计入 [`Self::dropped`]。
    pub fn trigger_note(
        &mut self,
        plan: &PreparedSfz,
        note: u8,
        velocity: u8,
        end_sample: u64,
    ) -> usize {
        let pcm_len = plan.pcm().len();
        let n = note as usize;
        let mut created = 0usize;
        for region in plan.regions() {
            if !region.accepts(note, velocity) {
                continue;
            }
            let rate = region.rate_per_note[n];
            if !(rate.is_finite() && rate > 0.0) || pcm_len == 0 {
                continue;
            }
            // ⭐ 终点口径：`span() == None`（`end=-1` / 空区间）⇒ 不建声部。
            let Some(source_end) = region.source_end(pcm_len) else {
                continue;
            };
            let voice = SfzVoice::new(
                note,
                velocity,
                rate,
                region.gain_at(note, velocity),
                source_end,
                end_sample,
            );
            match self.voices.iter().position(|voice| !voice.active) {
                Some(index) => {
                    self.voices[index] = voice;
                    created += 1;
                }
                None => self.dropped = self.dropped.saturating_add(1),
            }
        }
        created
    }

    /// 渲染本池的**全部活声部**到 `out`（`position` ＝ 本量子的**引擎绝对**起点）。
    ///
    /// **零分配**：只读 `plan` 的 PCM 与已有数组、原地写 `out` [MUST-GATE-001]。
    pub fn render_into(&mut self, plan: &PreparedSfz, position: u64, out: &mut [f32]) {
        let pcm = plan.pcm();
        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            // 终点口径：音符先到终点 ⇒ 本量子只渲染到它为止。
            let remaining = voice
                .end_sample
                .saturating_sub(position)
                .min(out.len() as u64) as usize;
            if remaining == 0 {
                voice.silence();
                continue;
            }
            let written = mix_span_into(
                pcm,
                &mut voice.position,
                voice.rate,
                voice.gain,
                voice.source_end,
                &mut out[..remaining],
            );
            // ⭐ 用样本自然结束：源位置走完 ⇒ 退休（⛔ 无包络、⛔ 无循环）。
            if written < remaining {
                voice.silence();
            }
        }
    }

    /// 释放**全部**声部（`seek` / 换音源 ⇒ ⛔ 不留上一件音源的尾巴）。
    ///
    /// ⚠ 保留 [`Self::dropped`]：它是**累计**读数（与"活声部数"这张量规配对），
    /// 与 `DrumMachine::reset` 保留 `drum_hits` 同口径。
    pub fn silence(&mut self) {
        for voice in &mut self.voices {
            voice.silence();
        }
    }

    /// 活声部数（**量规**：可升可降）。
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.voices.iter().filter(|voice| voice.active).count()
    }

    /// 因池满被丢弃的声部数（**累计**，单调递增）。
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
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
/// * [`Self::prepare`]：**允许分配**（它调 [`PreparedSfz::prepare`]，建区域表与 PCM）；
/// * ⛔ **不在快照边界摊平**：快照边界（`SynthEngine::begin_snapshot` / `arm_from_snapshot`）
///   一个字都不碰它 —— 那两条路径会分配（见 `crate::synth::SynthEngine::arm_from_snapshot`）；
/// * 实时侧只**借用** [`Self::prepared`] 的 `&PreparedSfz`（**PCM 借用快照**），
///   ⛔ 不 clone、⛔ 不 `Arc` clone 那个 `Vec`。
///
/// ⚠ **轨↔SFZ 的绑定不由模型给**（模块头 sfz-needs-5）⇒ 只能由引擎 API 显式注入
/// （[`crate::snapshot::EngineSnapshot::with_sfz`]）。
#[derive(Clone, Debug, PartialEq)]
pub struct SfzPlan {
    /// 重数据：摊平后的区域表 ＋ PCM（控制侧构造、实时侧只读）。
    prepared: PreparedSfz,
}

impl SfzPlan {
    /// 控制侧摊平（**允许分配**）：见 [`PreparedSfz::prepare`] 的口径。
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

    /// 摊平占用的**内存字节数**（读数用：区域表 ＋ PCM）。
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
