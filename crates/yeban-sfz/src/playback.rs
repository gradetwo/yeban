//! 把一次 note-on 变成**可渲染的采样描述**：`yeban-engine` 接入采样播放的接口。
//!
//! 本模块补上 `crate::lib.rs` 文档里点名的缺口：「静态预分配声部池（SFZ 采样源待接入）」。
//! 它只做**纯数值换算**（音高比、步进比、线性增益、循环窗口），不做 I/O、不解码音频、
//! 不查采样字节。因此它**不依赖任何新依赖**，也不需要采样数据入库。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M2-005（SFZ 引擎）
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2 [ARCH-RT-001]（RT 零分配）
//!
//! 格式事实与出处（全部取自 <https://sfzformat.com/opcodes/>）：
//! - `pitch_keycenter`：采样原始音高对应的 MIDI 根音。默认 60。
//!   <https://sfzformat.com/opcodes/pitch_keycenter/>
//! - `transpose`：移调，单位半音，默认 0，范围 -127..=127。
//!   <https://sfzformat.com/opcodes/transpose/>
//! - `tune`：微调，单位音分（cent），默认 0，SFZ1 范围 -100..=100。
//!   <https://sfzformat.com/opcodes/tune/>
//! - `bend_up`：弯音轮上推的弯音范围，单位音分，默认 200，范围 -9600..=9600。
//!   <https://sfzformat.com/opcodes/bend_up/>
//! - `bend_down`：弯音轮下推的弯音范围，单位音分，默认 -200，范围 -9600..=9600。
//!   <https://sfzformat.com/opcodes/bend_down/>
//! - `volume`：音量，单位 dB，默认 0，规范范围 -144..=6。
//!   <https://sfzformat.com/opcodes/volume/>
//! - `offset`：采样起始偏移，单位采样点，默认 0。
//!   <https://sfzformat.com/opcodes/offset/>
//! - `end`：采样播放终点，单位采样点，**含**端点；默认 unspecified；`end=-1` 不发声。
//!   <https://sfzformat.com/opcodes/end/>
//! - `direction`：`forward`（默认）/ `reverse`（SFZ v2）。
//!   <https://sfzformat.com/opcodes/direction/>
//!
//! # 换算口径
//!
//! ```text
//! 音高比 pitch_ratio = 2 ^ ( ((note - pitch_keycenter + transpose) * 100 + tune) / 1200 )
//! 步进比 rate        = pitch_ratio * (sample_hz / engine_hz)
//! 线性增益 gain      = 2 ^ ( volume_db * log2(10) / 20 )
//! 采样区间 span      = [ offset , end + 1 )   （end 缺省 ⇒ 终点由解码器给出）
//! ```
//!
//! `rate` 的单位是「每输出一个采样所前进的**源采样**个数」。`region.pitch_keycenter`、
//! `region.transpose`、`region.tune` 只决定音高；采样文件自身的采样率只能由解码器给出，
//! 因此它作为 [`RenderRates`] 的输入（**不是**本 crate 猜测的值）。
//!
//! # 力度 → 增益的两段口径
//!
//! [`PlaybackSpec::gain`] 只含 `volume`（dB → 线性）；力度 → 振幅是**另一段**，
//! 由 [`PlaybackSpec::velocity_gain`] 携带（`amp_velcurve_N` 点表优先，否则
//! `amp_veltrack` 幂律；全部出处与工程裁决见 [`crate::velocity`]）。
//! 合并值是 [`PlaybackSpec::total_gain`]。这样分段是为了不静默改变既有消费方的电平：
//! 加这个字段之前 `gain` 就只含 `volume`。
//!
//! # 刻意不做的换算（避免发明语义）
//!
//! - **弯音轮状态**：`bend_up` / `bend_down` 只作为字段（[`PlaybackSpec::bend_up`] /
//!   [`PlaybackSpec::bend_down`]）与两个**纯函数**（[`Region::bend_cents`] /
//!   [`Region::bend_ratio`]）带出；本 crate **不持有**弯音轮值 —— 那是引擎的 MIDI 输入。
//! - **声相定律**：`pan` 原样以百分比输出；`pan_law` 是引擎侧的 need（N4）。
//! - **循环窗口缺省**：本 crate 不解码音频，所以 `loop_mode` 缺省是
//!   [`LoopMode::NoLoop`]（见 `crate::instrument::Region` 文档），一律不循环。
//! - **无有效循环窗口的降级**：`loop_mode` 要求循环、而 `loop_end <= loop_start` 时，
//!   [`Region::loop_window`] 返回 `None`。这是本模块的工程裁决：零长度循环没有定义，
//!   返回 `None` 表示「按不循环渲染」，绝不返回一个 `start == end` 的死循环窗口。
//! - **`direction=reverse` 只作为字段传递**：本 crate 不做逐样本读取，因此**不规定**
//!   反向播放时循环窗口的移动方向（那是渲染器的语义），只把规范给出的取值原样带出。
//!
//! # 实时安全
//!
//! [`Region::playback_spec`] / [`Instrument::playback_for`] 全程**零堆分配、零锁、
//! 零 I/O**：它们只读已构造好的 region 索引并返回 `Copy` 的 [`PlaybackSpec`]。
//! 音高/增益使用 `exp2`（与 `crate::voice_pool::StealFade::gain_at` 同类的浮点原语）；
//! 跨架构逐位一致性 (ARCH-DET-002) 未验证，与 `StealFade` 登记在同一条 pending 上。

use crate::instrument::{
    Instrument, LoopMode, NotePolyphony, OffMode, PlayDirection, Region, RegionQuery, SampleEnd,
    Trigger,
};

/// 采样率回退值 (Hz)：输入采样率非有限或非正时使用，与
/// [`crate::voice_pool::StealFade::new`] 的回退口径一致。
pub const FALLBACK_SAMPLE_RATE: f32 = 48_000.0;

/// 一个八度内的音分数（`transpose` 半音与 `tune` 音分统一到这个单位）。
const CENTS_PER_OCTAVE: f64 = 1200.0;

/// 渲染采样所需的两个采样率 (Hz)。
///
/// `sample_hz` 是采样文件自身的采样率，只能由解码器给出；`engine_hz` 是工程 /
/// 引擎的运行采样率。`copy` 语义，可在实时路径构造。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRates {
    /// 采样文件的原始采样率 (Hz)。
    pub sample_hz: f32,
    /// 工程 / 引擎采样率 (Hz)。
    pub engine_hz: f32,
}

impl RenderRates {
    /// 构造采样率对（不做校验；校验发生在 [`RenderRates::sanitized`]）。
    #[must_use]
    pub fn new(sample_hz: f32, engine_hz: f32) -> Self {
        Self {
            sample_hz,
            engine_hz,
        }
    }

    /// 把非有限 / 非正的采样率换成 [`FALLBACK_SAMPLE_RATE`]。
    ///
    /// 该回退保证 [`RenderRates`] 永远产生有限的步进比，绝不产生 `NaN` / 除零。
    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            sample_hz: sanitize_rate(self.sample_hz),
            engine_hz: sanitize_rate(self.engine_hz),
        }
    }
}

impl Default for RenderRates {
    /// 采样率与引擎采样率都是 [`FALLBACK_SAMPLE_RATE`]（即不做采样率换算）。
    fn default() -> Self {
        Self {
            sample_hz: FALLBACK_SAMPLE_RATE,
            engine_hz: FALLBACK_SAMPLE_RATE,
        }
    }
}

fn sanitize_rate(hz: f32) -> f32 {
    if hz.is_finite() && hz > 0.0 {
        hz
    } else {
        FALLBACK_SAMPLE_RATE
    }
}

/// 一段有效的循环窗口，单位是**采样点**，半开区间 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopWindow {
    /// 循环起点（含）。
    pub start: u32,
    /// 循环终点（不含）。
    pub end: u32,
}

impl LoopWindow {
    /// 循环长度（采样点）。构造上恒 `>= 1`（见 [`Region::loop_window`]）。
    #[must_use]
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// 是否为空窗口。构造上恒为 `false`；保留该方法是 `len()` 的配对 API。
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// 一次 note-on 读取的**源采样区间**，半开区间 `[start, end)`，单位是**源采样点**。
///
/// 来源：`offset`（起点，含）与 `end`（终点，**含**）——
/// <https://sfzformat.com/opcodes/offset/>、<https://sfzformat.com/opcodes/end/>。
///
/// # 为什么终点是 `u64`
///
/// [`LoopWindow`] 用 `u32`，本结构用 `u64`：`end` 的规范上界是 `4294967295`，
/// 而半开区间的终点需要 `end + 1 = 4294967296`，`u32` 装不下 ——
/// 饱和到 `u32::MAX` 会少播最后一个采样点，闭区间语义就无法无歧义地表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleSpan {
    /// 起始采样点（含）。等于 `offset`。
    pub start: u64,
    /// 终点采样点（**不含**）。`None` 表示「到采样末尾」，终点只能由解码器给出。
    pub end: Option<u64>,
}

impl SampleSpan {
    /// 区间长度（采样点）。终点未知（播到采样末尾）时返回 `None`。
    ///
    /// 用 `saturating_sub`：字段是 `pub`，手工构造出 `end < start` 也不 panic。
    #[must_use]
    pub fn len(self) -> Option<u64> {
        self.end.map(|end| end.saturating_sub(self.start))
    }

    /// 是否为空区间（构造上为 `false`；保留为 `len()` 的配对 API）。
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == Some(0)
    }
}

/// 一次 note-on 的可渲染采样描述。
///
/// `Copy`、不含指针、不含堆数据：渲染器可以把它存进自己的预分配声部槽，
/// 逐样本路径随时读取 [ARCH-RT-001]。
///
/// **采样身份不在本结构里**：`sample` 路径与已解码的采样缓冲的对应关系是
/// **加载期**的事（引擎按 region 建立索引）。实时路径只读本结构的数值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackSpec {
    /// 触发音符（MIDI 号 0..=127）。
    pub note: u8,
    /// 触发力度 0..=127。**不**折进 [`PlaybackSpec::gain`]：力度的影响单独由
    /// [`PlaybackSpec::velocity_gain`] 携带（理由见模块文档与
    /// [`PlaybackSpec::total_gain`]）。
    pub velocity: u8,
    /// 该 region 的根音（`pitch_keycenter`）。
    pub pitch_keycenter: i32,
    /// 该 region 的移调（半音）。
    pub transpose: i32,
    /// 该 region 的微调（音分）。
    pub tune: i32,
    /// 该 region 的 `bend_up`（音分，原样；缺省 [`crate::BEND_UP_DEFAULT_CENTS`]）。
    ///
    /// 弯音范围**不是**每样本都用的量：调用方把它与弯音轮值一起送进
    /// [`Region::bend_cents`] / [`Region::bend_ratio`]。单独带出是为了让消费方能自己换算，
    /// 也为了 [`PlaybackSpec::bend_up`] 与 [`PlaybackSpec::bend_down`] 在 `Copy` 结构里可读。
    pub bend_up: i32,
    /// 该 region 的 `bend_down`（音分，原样；缺省 [`crate::BEND_DOWN_DEFAULT_CENTS`]）。
    ///
    /// 允许为正（规范正文明写正值可用于齐特琴 / 吉他），见 [`Region::bend_down`]。
    pub bend_down: i32,
    /// 音高比：`2 ^ (((note - pitch_keycenter + transpose) * 100 + tune) / 1200)`。
    pub pitch_ratio: f32,
    /// 步进比：每输出一个采样前进的源采样个数（已含采样率换算）。
    pub rate: f32,
    /// 该 region 的 `volume`（dB，原样）。
    pub volume_db: f32,
    /// `volume` 换算出的**线性**增益：`10 ^ (volume_db / 20)`。
    ///
    /// **不含**力度：只折 `volume`。力度 → 振幅在
    /// [`PlaybackSpec::velocity_gain`]，两者相乘见 [`PlaybackSpec::total_gain`]。
    pub gain: f32,
    /// 该 region 的 `amp_veltrack`（%，原样；缺省
    /// [`crate::velocity::AMP_VELTRACK_DEFAULT`]）。
    ///
    /// 只在源文件**没有**给出 `amp_velcurve_N` 时决定
    /// [`PlaybackSpec::velocity_gain`]；单独带出是为了让消费方能自己换算。
    pub amp_veltrack: f32,
    /// 力度 → **线性振幅**的因子（0.0 = 静音，1.0 = 满幅）。
    ///
    /// 就是 [`crate::instrument::Region::velocity_gain`]（`velocity` 处的读数）：
    /// 文件给了 `amp_velcurve_N` 就用那条点表，否则用 `amp_veltrack` 幂律
    /// （缺省 100 ⇒ `(v/127)^2`）。出处与工程裁决见 [`crate::velocity`]。
    ///
    /// **刻意不折进 [`PlaybackSpec::gain`]**：加这个字段前 `gain` 只含 `volume`，
    /// 既有消费方（以及本 crate 的既有判据）按那个口径读；把力度并进去会静默改变
    /// 它们的电平。要用合并值请调 [`PlaybackSpec::total_gain`]。
    pub velocity_gain: f32,
    /// 交叉淡化 → **线性振幅**的因子（0.0 = 静音，1.0 = 满幅）。
    ///
    /// 就是 [`Region::crossfade_gain`] 在**触发时刻**的读数：`xfin_*` / `xfout_*`
    /// 各段各自给出一个 [0, 1] 的因子并**相乘**（出处与工程裁决见
    /// [`crate::crossfade`]）。没有 `xfin_*` / `xfout_*` 的 region 恒为 1.0，
    /// 因此合并值 [`PlaybackSpec::total_gain`] 对它们逐位不变。
    ///
    /// 这里只是快照：CC 驱动的段随 CC 值变化，逐样本路径应改用
    /// [`Region::crossfade_gain`] 并传入当时的 CC 状态。未提供 CC 状态的构造路径
    /// （[`Region::playback_spec`]）只折键盘与力度两个轴。
    pub crossfade_gain: f32,
    /// 该 region 的 `pan`（百分比，原样；声相定律由调用方决定）。
    pub pan: f32,
    /// 该 region 的 `trigger`（原样）。
    pub trigger: Trigger,
    /// 该 region 的 `off_mode`（原样；缺省 [`OffMode::Fast`]）。
    ///
    /// 与 [`PlaybackSpec::ignores_note_off`] 的分工：后者为真时 note-off **根本不结束**
    /// 声部（`loop_mode=one_shot`）；本字段描述 note-off **真的到达之后**怎么结束。
    /// 只有 [`OffMode::Fast`] 允许立刻切断，见 [`PlaybackSpec::cuts_at_note_off`]。
    pub off_mode: OffMode,
    /// 该 region 的 `off_time`（秒，原样；`None` ＝ 源文件未给出）。
    ///
    /// 原始取值，**不**做缺省回填：消费方用 [`PlaybackSpec::effective_off_time`] 取
    /// 生效值（缺省 [`crate::OFF_TIME_DEFAULT_SECONDS`]）。该时长只对
    /// `off_mode=time` 的关断有意义（<https://sfzformat.com/opcodes/off_time/>）。
    pub off_time: Option<f32>,
    /// 生效的循环模式（原样，**但** `trigger=release` / `release_key` 强制
    /// [`LoopMode::OneShot`]，见 [`Region::effective_loop_mode`]）。
    pub loop_mode: LoopMode,
    /// 有效循环窗口；`None` 表示按不循环渲染。
    pub loop_window: Option<LoopWindow>,
    /// 该 region 的 `offset`（采样点，原样）。
    pub offset: u32,
    /// 该 region 的 `end`（原样；`Inclusive` 时**含**端点）。
    pub end: SampleEnd,
    /// 该 region 的 `direction`（原样）。
    pub direction: PlayDirection,
    /// `offset` / `end` 归约出的源采样区间；`None` 表示不产生采样输出
    /// （`end=-1`，或显式区间落在 `offset` 之前）。
    pub span: Option<SampleSpan>,
    /// 独占组（`group`）。
    pub group: u32,
    /// 被谁关掉（`off_by`）。
    pub off_by: u32,
    /// 同一音高在 polyphony group（[`PlaybackSpec::group`]）内的同时发声数限制
    /// （`note_polyphony` + `note_selfmask`，缺省 [`NotePolyphony::UNLIMITED`]）。
    ///
    /// 判定函数是 [`crate::voice_pool::VoicePool::apply_note_polyphony`]；语义与
    /// 规范出处见 [`NotePolyphony`]。
    pub note_polyphony: NotePolyphony,
    /// 该 region 的 `<region>` 段头所在行号（1-based，诊断用）。
    pub source_line: usize,
}

impl PlaybackSpec {
    /// `volume`、力度与交叉淡化三段的**合并**线性增益：
    /// `gain * velocity_gain * crossfade_gain`。
    ///
    /// 三段刻意分开（见 [`PlaybackSpec::gain`] / [`PlaybackSpec::velocity_gain`] /
    /// [`PlaybackSpec::crossfade_gain`] 的文档）；本方法是唯一的合并点。逐样本路径上
    /// 是两次乘法，不分配、不加锁。没有 `xfin_*` / `xfout_*` 的 region 的
    /// `crossfade_gain` 恰为 1.0，乘以它**逐位不变**。
    #[must_use]
    pub fn total_gain(&self) -> f32 {
        self.gain * self.velocity_gain * self.crossfade_gain
    }

    /// 是否循环（`loop_mode` 要求循环**且**窗口有效）。
    #[must_use]
    pub fn loops(&self) -> bool {
        self.loop_window.is_some()
    }

    /// 是否忽略 note-off（`loop_mode=one_shot`，鼓组常用）。
    #[must_use]
    pub fn ignores_note_off(&self) -> bool {
        self.loop_mode == LoopMode::OneShot
    }

    /// note-off 到达时，声部是否**可以立刻**结束（`off_mode=fast`，规范缺省）。
    ///
    /// `false` 表示规范要求一段放大器包络 release（`off_mode=normal`）或一段由
    /// [`PlaybackSpec::off_time`] 给定的淡化时长（`off_mode=time`）：
    /// 实现属于引擎侧，本 crate 只报告契约。
    /// 与 [`PlaybackSpec::ignores_note_off`] 相互独立：后者为真时 note-off 不结束声部，
    /// 本谓词就无从谈起。
    #[must_use]
    pub fn cuts_at_note_off(&self) -> bool {
        self.off_mode.cuts_voice_immediately()
    }

    /// 生效的 `off_time`（秒）：源文件给出则原样返回，否则是规范缺省
    /// [`crate::OFF_TIME_DEFAULT_SECONDS`]（0.006 s）。
    ///
    /// 只在 `off_mode=time` 的关断路径上有意义（见
    /// <https://sfzformat.com/opcodes/off_time/>）。零分配，可在实时路径调用。
    #[must_use]
    pub fn effective_off_time(&self) -> f32 {
        self.off_time.unwrap_or(crate::OFF_TIME_DEFAULT_SECONDS)
    }

    /// 是否不产生采样输出（`end=-1`，或显式区间为空）。
    ///
    /// **该 region 仍然被触发**：调用方仍然可以用 [`PlaybackSpec::group`] /
    /// [`PlaybackSpec::off_by`] 让它关掉别的 region —— 这正是规范为 `end=-1` 给出的用途
    /// （<https://sfzformat.com/opcodes/end/>）。
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.span.is_none()
    }

    /// 是否反向播放（`direction=reverse`）。
    #[must_use]
    pub fn plays_reverse(&self) -> bool {
        self.direction == PlayDirection::Reverse
    }
}

/// [`Instrument::playback_for`] 的结果：选中的 region **加**它的可渲染描述。
///
/// `region` 用于加载期解析采样身份（[`Region::sample_path`]）；`spec` 用于实时路径。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionPlay<'i, 'a> {
    /// 选中的 region（借用自 [`Instrument`]）。
    pub region: &'i Region<'a>,
    /// 该 region 的可渲染采样描述。
    pub spec: PlaybackSpec,
}

impl<'a> Region<'a> {
    /// 音高比：`2 ^ (((note - pitch_keycenter + transpose) * 100 + tune) / 1200)`。
    ///
    /// 全程 `f64` 计算后收窄到 `f32`。零分配，可在实时路径调用。
    #[must_use]
    pub fn pitch_ratio(&self, note: u8) -> f32 {
        let semitones = f64::from(i32::from(note)) - f64::from(self.pitch_keycenter)
            + f64::from(self.transpose);
        let cents = semitones * 100.0 + f64::from(self.tune);
        (cents / CENTS_PER_OCTAVE).exp2() as f32
    }

    /// 步进比：`pitch_ratio(note) * sample_hz / engine_hz`。
    ///
    /// 非法采样率按 [`RenderRates::sanitized`] 回退，因此返回值恒为有限正数
    /// （`pitch_ratio` 的合法输入范围是 `2^-21.2 ..= 2^31.7`，见模块测试）。
    #[must_use]
    pub fn playback_rate(&self, note: u8, rates: RenderRates) -> f32 {
        let rates = rates.sanitized();
        self.pitch_ratio(note) * (rates.sample_hz / rates.engine_hz)
    }

    /// 弯音轮在 `wheel`（0..=127，中位 [`crate::PITCH_BEND_CENTER`]）处贡献的**音分数**。
    ///
    /// 规范语义（<https://sfzformat.com/opcodes/bend_up/>、
    /// <https://sfzformat.com/opcodes/bend_down/>）：轮子朝一个方向走 `bend_up` 音分，
    /// 朝另一个方向走 `bend_down` 音分。本 crate 按**线性**在两个端点之间插值：
    ///
    /// ```text
    /// wheel > 64    ⇒  bend_up   * (wheel - 64) / 63
    /// wheel == 64   ⇒  0
    /// wheel < 64    ⇒  bend_down * (64 - wheel) / 64
    /// ```
    ///
    /// `bend_up` / `bend_down` **可正可负**：规范正文明写 `bend_up` 为负时「轮子上推使音高
    /// 下降」，`bend_down` 为正时两个方向都往上弯。因此本函数不做符号钳制。
    ///
    /// 取值返回音分数（`i32`）。解析路径保证两个范围都在 ±9600 内（见
    /// [`crate::BEND_RANGE_MAX_CENTS`]），此时插值结果必然也在 ±9600 内，不会溢出；
    /// 手工构造的 region 若把范围设成 `i32::MAX` / `i32::MIN`，本函数仍**不 panic**：
    /// 中间乘积走 `i128` 并饱和到 `i32`。
    ///
    /// 零分配、无锁、无 I/O，可在实时路径调用。[`crate::PITCH_BEND_CENTER`] 是「未弯音」；
    /// `wheel` 以 `u8` 传入，越界（>127）在类型上不可能。14 位轮值（0..=16383）
    /// 的归约属于调用方。
    #[must_use]
    pub fn bend_cents(&self, wheel: u8) -> i32 {
        let scaled = |range: i32, steps: u8, total: i32| -> i32 {
            let product = i128::from(range) * i128::from(steps);
            let quotient = product / i128::from(total);
            i32::try_from(quotient).unwrap_or(if quotient > 0 { i32::MAX } else { i32::MIN })
        };
        match wheel.cmp(&crate::PITCH_BEND_CENTER) {
            core::cmp::Ordering::Equal => 0,
            // 上界 63 个刻度（65..=127）映射到完整的 `bend_up`。
            core::cmp::Ordering::Greater => {
                scaled(self.bend_up, wheel - crate::PITCH_BEND_CENTER, 63)
            }
            // 下界 64 个刻度（0..=63）映射到完整的 `bend_down`。
            core::cmp::Ordering::Less => {
                scaled(self.bend_down, crate::PITCH_BEND_CENTER - wheel, 64)
            }
        }
    }

    /// 含弯音的**音高比**：`pitch_ratio(note) * 2 ^ (bend_cents(wheel) / 1200)`。
    ///
    /// 这是把一次 note-on 的完整音高信息折算成单一乘数的唯一位置：
    /// [`Region::pitch_ratio`] 仍是「不含弯音」的口径（既有消费方按那个口径读），
    /// 本函数只是它乘上 [`Region::bend_cents`] 的指数。
    ///
    /// **裁决（工程）**：弯音是**乘法**作用于音高比，不是加到音分上再一起取指数 ——
    /// 规范给的是两个独立范围（`bend_up` / `bend_down` 音分），乘法与「先合成总音分」
    /// 在数学上等价，但乘法不必重新读 `pitch_keycenter` / `transpose` / `tune`。
    /// 逐位一致性沿用 [`Region::pitch_ratio`] 的同一条 pending（`exp2` 的跨架构口径）。
    ///
    /// `wheel == PITCH_BEND_CENTER` 时结果与 [`Region::pitch_ratio`] **逐位相同**：
    /// 指数是 `2^0 = 1`，一次乘 1.0 不改变任何有限值（含 `-0.0` 与次正规数）。
    /// 零分配，可在实时路径调用。
    #[must_use]
    pub fn bend_ratio(&self, note: u8, wheel: u8) -> f32 {
        let bend = f64::from(self.bend_cents(wheel)) / CENTS_PER_OCTAVE;
        self.pitch_ratio(note) * bend.exp2() as f32
    }

    /// `volume` (dB) → 线性增益：`10 ^ (volume / 20)`。
    ///
    /// 解析路径保证 `volume` 有限（[`crate::parser::OpcodeValue::as_f32`] 拒绝
    /// `NaN` / `±Inf`），因此本函数对解析得到的 region 不会返回 `NaN`。
    /// 手工构造的 region 若把 `volume` 设为非有限值，结果是 IEEE 语义的 `NaN` / `inf`
    /// —— 本 crate **不**发明钳制策略。
    #[must_use]
    pub fn linear_gain(&self) -> f32 {
        (f64::from(self.volume) * std::f64::consts::LOG2_10 / 20.0).exp2() as f32
    }

    /// 有效循环窗口。
    ///
    /// 返回 `Some` 的条件：**生效的**循环模式（[`Region::effective_loop_mode`]，
    /// `trigger=release` / `release_key` 会被覆盖成 `one_shot`）是 `loop_continuous`
    /// 或 `loop_sustain`，**且** `loop_end > loop_start`。`no_loop` / `one_shot`
    /// 恒返回 `None`（即使文件写了 `loop_start` / `loop_end`）。
    #[must_use]
    pub fn loop_window(&self) -> Option<LoopWindow> {
        if !matches!(
            self.effective_loop_mode(),
            LoopMode::LoopContinuous | LoopMode::LoopSustain
        ) {
            return None;
        }
        if self.loop_end > self.loop_start {
            Some(LoopWindow {
                start: self.loop_start,
                end: self.loop_end,
            })
        } else {
            None
        }
    }

    /// 该 region 实际读取的源采样区间；`None` 表示不产生采样输出。
    ///
    /// 规则全部来自 `offset` / `end` 的规范语义
    /// （<https://sfzformat.com/opcodes/offset/>、<https://sfzformat.com/opcodes/end/>）：
    ///
    /// | `end` | 结果 |
    /// | :--- | :--- |
    /// | 未指定 | `Some(SampleSpan { start: offset, end: None })`（播到采样末尾） |
    /// | `-1` | `None`（不发声，但 region 仍然被触发） |
    /// | 显式 `n` | `Some(SampleSpan { start: offset, end: Some(n + 1) })`（`end` **含**端点） |
    ///
    /// **工程裁决**：显式 `end` 落在 `offset` 之前（`end + 1 <= offset`）时返回 `None`
    /// —— 没有任何采样点可读。这与 [`Region::loop_window`] 拒绝零长度窗口是同一条口径：
    /// 不返回反向区间，也不发明「反向自动交换两端」的语义。
    ///
    /// 零分配，可在实时路径调用。
    #[must_use]
    pub fn playback_span(&self) -> Option<SampleSpan> {
        let start = u64::from(self.offset);
        match self.end {
            SampleEnd::Silent => None,
            SampleEnd::Unspecified => Some(SampleSpan { start, end: None }),
            SampleEnd::Inclusive(end) => {
                // `u64` 加法：`end = u32::MAX` 时终点是 4294967296，不回绕。
                let end = u64::from(end) + 1;
                if end <= start {
                    None
                } else {
                    Some(SampleSpan {
                        start,
                        end: Some(end),
                    })
                }
            }
        }
    }

    /// 生成该 region 的可渲染采样描述。零分配，可在实时路径调用。
    ///
    /// 这条重载只吃 `note` / `velocity` 两个标量，因此
    /// [`PlaybackSpec::crossfade_gain`] **只折键盘与力度两个轴**：CC 驱动的交叉淡化段
    /// 需要 [`RegionQuery::cc`]，本方法没有那段状态，于是它们按「未提供状态」处理
    /// （该段 1.0，见 [`Region::crossfade_gain`]）。要带上 CC 状态请用
    /// [`Region::playback_spec_with`]。
    #[must_use]
    pub fn playback_spec(&self, note: u8, velocity: u8, rates: RenderRates) -> PlaybackSpec {
        self.playback_spec_with(&RegionQuery::new(note, velocity), rates)
    }

    /// 生成该 region 的可渲染采样描述，交叉淡化按 `query` 的完整状态求值。
    ///
    /// 与 [`Region::playback_spec`] 的唯一差别是 [`PlaybackSpec::crossfade_gain`]：
    /// 这里能看到 `query.cc`，因此 CC 轴也参与。零分配，可在实时路径调用。
    #[must_use]
    pub fn playback_spec_with(&self, query: &RegionQuery<'_>, rates: RenderRates) -> PlaybackSpec {
        let note = query.note;
        let velocity = query.velocity;
        let rates = rates.sanitized();
        PlaybackSpec {
            note,
            velocity,
            pitch_keycenter: self.pitch_keycenter,
            transpose: self.transpose,
            tune: self.tune,
            bend_up: self.bend_up,
            bend_down: self.bend_down,
            pitch_ratio: self.pitch_ratio(note),
            rate: self.playback_rate(note, rates),
            volume_db: self.volume,
            gain: self.linear_gain(),
            amp_veltrack: self.amp_veltrack,
            velocity_gain: self.velocity_gain(velocity),
            crossfade_gain: self.crossfade_gain(query),
            pan: self.pan,
            trigger: self.trigger,
            off_mode: self.off_mode,
            off_time: self.off_time,
            loop_mode: self.effective_loop_mode(),
            loop_window: self.loop_window(),
            offset: self.offset,
            end: self.end,
            direction: self.direction,
            span: self.playback_span(),
            group: self.group,
            off_by: self.off_by,
            note_polyphony: self.note_polyphony,
            source_line: self.source_line,
        }
    }
}

impl<'a> Instrument<'a> {
    /// 一次调用完成「选 region → 生成可渲染描述」。
    ///
    /// 参数 `query` 的全部字段（音符 / 力度 / 通道 / 轮替序号 / keyswitch / CC）
    /// 都参与选择，语义与 [`Instrument::region_for_with`] 完全一致。
    /// 没有匹配 region 时返回 `None`。全程零分配，可在实时路径调用。
    ///
    /// 交叉淡化按**完整**的 `query` 求值（CC 轴也在内），见
    /// [`Region::playback_spec_with`]。
    #[must_use]
    pub fn playback_for(
        &self,
        query: RegionQuery<'_>,
        rates: RenderRates,
    ) -> Option<RegionPlay<'_, 'a>> {
        let region = self.region_for_query(&query)?;
        Some(RegionPlay {
            region,
            spec: region.playback_spec_with(&query, rates),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PITCH_BEND_CENTER;
    use crate::parser::parse_text;

    const RATES_EQUAL: RenderRates = RenderRates {
        sample_hz: 48_000.0,
        engine_hz: 48_000.0,
    };

    fn close(left: f32, right: f32, tolerance: f32) -> bool {
        (left - right).abs() <= tolerance
    }

    #[test]
    fn pitch_ratio_follows_equal_temperament() {
        let instrument = parse_text(
            "<region>sample=a.wav pitch_keycenter=60",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(region.pitch_ratio(60), 1.0);
        assert_eq!(
            region.pitch_ratio(72),
            2.0,
            "an octave up doubles the ratio"
        );
        assert_eq!(
            region.pitch_ratio(48),
            0.5,
            "an octave down halves the ratio"
        );
        assert!(
            close(region.pitch_ratio(61), 2.0f32.powf(1.0 / 12.0), 1.0e-6),
            "one semitone must be 2^(1/12)"
        );
    }

    #[test]
    fn bend_cents_maps_the_wheel_endpoints_to_the_two_spec_ranges() {
        let instrument = parse_text(
            "<region>sample=a.wav pitch_keycenter=60 bend_up=1200 bend_down=-1200",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];

        // 中位（未弯音）恒为 0，与两个范围取值无关。
        assert_eq!(region.bend_cents(PITCH_BEND_CENTER), 0);
        for (up, down) in [(200, -200), (1200, -1200), (0, 0), (-1200, 1200)] {
            let source = format!("<region>sample=a.wav bend_up={up} bend_down={down}");
            let probe = parse_text(&source, &Default::default()).expect("parses");
            assert_eq!(probe.regions()[0].bend_cents(64), 0, "up={up} down={down}");
        }

        // 端点：轮子上限 127 走满 `bend_up`，轮子下限 0 走满 `bend_down`。
        assert_eq!(region.bend_cents(127), 1200);
        assert_eq!(region.bend_cents(0), -1200);

        // 严格单调（`bend_up` 为正时，轮值越大音分越高）。
        let rising: Vec<i32> = (0u8..=127).map(|wheel| region.bend_cents(wheel)).collect();
        assert!(
            rising.windows(2).all(|pair| pair[0] <= pair[1]),
            "bend_cents must be monotone in the wheel value"
        );
        assert_eq!(rising.len(), 128, "every wheel value is defined");

        // 规范正文明写两个范围都可以为负 / 为正：
        // "If `bend_up` is negative, then moving the pitch wheel up will cause the pitch
        // to move down." 因此上推方向**不**被钳制成正值。
        let inverted = parse_text(
            "<region>sample=a.wav bend_up=-1200 bend_down=1200",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(inverted.regions()[0].bend_cents(127), -1200);
        assert_eq!(inverted.regions()[0].bend_cents(0), 1200);
    }

    #[test]
    fn bend_ratio_at_center_is_bit_identical_to_the_static_pitch_ratio() {
        // 关键相容判据：没有弯音输入时，含弯音的音高比必须与既有 `pitch_ratio` 逐位相同
        // （`2^0 = 1` 恰好是乘法单位元）。若这条红了，说明中位轮值不再是恒等变换，
        // 既有的所有 `pitch_ratio` 消费方会被静默改电平/音高。
        let instrument = parse_text(
            "<region>sample=a.wav pitch_keycenter=48 bend_up=1200 bend_down=-1200",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        for note in [0u8, 1, 48, 60, 72, 127] {
            let ratio = region.pitch_ratio(note);
            assert_eq!(
                region.bend_ratio(note, PITCH_BEND_CENTER).to_bits(),
                ratio.to_bits(),
                "note {note}: the centered wheel must be the identity"
            );
        }

        // 1200 音分的两个端点 ⇒ 正好一个八度，与 `transpose=±12` 同值。
        assert_eq!(region.bend_ratio(48, 127), region.pitch_ratio(48) * 2.0);
        assert_eq!(region.bend_ratio(48, 0), region.pitch_ratio(48) * 0.5);
        assert_eq!(region.bend_ratio(48, 127), region.pitch_ratio(60));
        assert_eq!(region.bend_ratio(48, 0), region.pitch_ratio(36));
    }

    #[test]
    fn the_spec_default_bend_range_is_plus_minus_two_semitones() {
        // 缺省 200 / -200 音分 ⇒ 两个端点各是两个半音。这条把「规范缺省值」
        // 与「可听结果」钉在一起：缺省不是 0，也不是 0 钳位。
        let instrument = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!(region.bend_cents(127), 200);
        assert_eq!(region.bend_cents(0), -200);
        assert!(
            close(region.bend_ratio(60, 127), 2.0f32.powf(2.0 / 12.0), 1.0e-6),
            "the default bend_up must be two semitones"
        );
        assert!(close(
            region.bend_ratio(60, 0),
            2.0f32.powf(-2.0 / 12.0),
            1.0e-6
        ));
    }

    #[test]
    fn bend_cents_never_overflows_the_integer_type() {
        // 字段是 `pub`：手工构造越界 region 也不得 panic / 回绕（叶子 crate 的硬约束）。
        // `bend_cents` 的最坏情形是 ±9600 × 64 = 614 400，远小于 `i32::MAX`。
        let extreme = parse_text(
            "<region>sample=a.wav bend_up=9600 bend_down=-9600",
            &Default::default(),
        )
        .expect("parses");
        let region = &extreme.regions()[0];
        assert_eq!(region.bend_cents(127), 9600);
        assert_eq!(region.bend_cents(0), -9600);
        for wheel in 0u8..=127 {
            let cents = region.bend_cents(wheel);
            assert!(cents.abs() <= 9600, "wheel {wheel} gave {cents}");
        }

        // 手工构造的（解析路径不可能产生的）越界字段：结果必须是确定的算术，不是 panic。
        let mut handcrafted = extreme.regions()[0].clone();
        handcrafted.bend_up = i32::MAX;
        handcrafted.bend_down = i32::MIN;
        assert_eq!(handcrafted.bend_cents(64), 0);
        assert_eq!(
            handcrafted.bend_cents(127),
            i32::MAX,
            "saturating, not wrapping"
        );
        assert_eq!(
            handcrafted.bend_cents(0),
            i32::MIN,
            "saturating, not wrapping"
        );
    }

    #[test]
    fn transpose_and_tune_shift_the_ratio() {
        let by_transpose = parse_text(
            "<region>sample=a.wav pitch_keycenter=60 transpose=12",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(by_transpose.regions()[0].pitch_ratio(60), 2.0);

        // transpose 与「同一音符高 12 个半音」必须给出同一个比值。
        let by_note = parse_text(
            "<region>sample=a.wav pitch_keycenter=60",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(
            by_transpose.regions()[0].pitch_ratio(60),
            by_note.regions()[0].pitch_ratio(72)
        );

        // tune 是音分：+1200 音分 = 一个八度。
        let by_tune = parse_text(
            "<region>sample=a.wav pitch_keycenter=60 tune=100",
            &Default::default(),
        )
        .expect("parses");
        assert!(close(
            by_tune.regions()[0].pitch_ratio(60),
            2.0f32.powf(100.0 / 1200.0),
            1.0e-6
        ));
    }

    #[test]
    fn playback_rate_converts_the_sample_rate() {
        let instrument = parse_text(
            "<region>sample=a.wav pitch_keycenter=60",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];

        // 44.1 kHz 采样在 48 kHz 引擎上：每个输出采样前进 44100/48000 个源采样。
        let rates = RenderRates::new(44_100.0, 48_000.0);
        assert!(close(
            region.playback_rate(60, rates),
            44_100.0 / 48_000.0,
            1.0e-9
        ));
        // 上八度 = 2 × 步进比。
        assert!(close(
            region.playback_rate(72, rates),
            2.0 * 44_100.0 / 48_000.0,
            1.0e-9
        ));
        // 采样率相同 ⇒ 步进比就是音高比。
        assert_eq!(region.playback_rate(60, RATES_EQUAL), 1.0);
    }

    #[test]
    fn invalid_rates_fall_back_and_never_produce_nan() {
        let instrument = parse_text(
            "<region>sample=a.wav pitch_keycenter=60",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        for rates in [
            RenderRates::new(f32::NAN, 48_000.0),
            RenderRates::new(0.0, 48_000.0),
            RenderRates::new(-44_100.0, 48_000.0),
            RenderRates::new(44_100.0, f32::INFINITY),
            RenderRates::new(44_100.0, 0.0),
        ] {
            let rate = region.playback_rate(60, rates);
            assert!(
                rate.is_finite() && rate > 0.0,
                "rate = {rate} for {rates:?}"
            );
        }
        // 两个采样率都非法 ⇒ 回退后相互抵消，步进比等于音高比。
        assert_eq!(
            region.playback_rate(72, RenderRates::new(f32::NAN, f32::NAN)),
            2.0
        );
        assert_eq!(RenderRates::default(), RATES_EQUAL);
    }

    #[test]
    fn linear_gain_is_db_amplitude() {
        let gain_of = |volume: f32| {
            parse_text(
                &format!("<region>sample=a.wav volume={volume}"),
                &Default::default(),
            )
            .expect("parses")
            .regions()[0]
                .linear_gain()
        };
        assert!(close(gain_of(0.0), 1.0, 1.0e-7));
        assert!(close(gain_of(-6.0206), 0.5, 1.0e-5));
        assert!(close(gain_of(6.0), 1.995_262_3, 1.0e-5));
        // `gain` 是 `volume` 的别名（见 instrument.rs 的 build_region）。
        let gain_alias =
            parse_text("<region>sample=a.wav gain=-6.0206", &Default::default()).expect("parses");
        assert!(close(gain_alias.regions()[0].linear_gain(), 0.5, 1.0e-5));
    }

    #[test]
    fn loop_window_requires_a_strictly_positive_length() {
        let window_of = |text: &str| {
            parse_text(text, &Default::default())
                .expect("parses")
                .regions()[0]
                .loop_window()
        };
        let ok =
            window_of("<region>sample=a.wav loop_mode=loop_continuous loop_start=10 loop_end=20");
        assert_eq!(ok, Some(LoopWindow { start: 10, end: 20 }));
        assert_eq!(ok.expect("window").len(), 10);
        assert!(!ok.expect("window").is_empty());

        // 零长度 / 反向窗口 ⇒ None（不返回死循环）。
        assert_eq!(
            window_of("<region>sample=a.wav loop_mode=loop_continuous loop_start=10 loop_end=10"),
            None
        );
        assert_eq!(
            window_of("<region>sample=a.wav loop_mode=loop_sustain loop_start=30 loop_end=20"),
            None
        );
        // no_loop / one_shot / 缺省：即使写了循环点也不循环。
        assert_eq!(
            window_of("<region>sample=a.wav loop_start=10 loop_end=20"),
            None
        );
        assert_eq!(
            window_of("<region>sample=a.wav loop_mode=one_shot loop_start=10 loop_end=20"),
            None
        );
    }

    #[test]
    fn one_shot_ignores_note_off_and_only_loop_modes_loop() {
        let spec_of = |text: &str| {
            parse_text(text, &Default::default())
                .expect("parses")
                .regions()[0]
                .playback_spec(60, 100, RATES_EQUAL)
        };
        let one_shot = spec_of("<region>sample=a.wav loop_mode=one_shot");
        assert!(one_shot.ignores_note_off());
        assert!(!one_shot.loops());

        let no_loop = spec_of("<region>sample=a.wav");
        assert!(!no_loop.ignores_note_off());
        assert!(!no_loop.loops());

        let looping =
            spec_of("<region>sample=a.wav loop_mode=loop_continuous loop_start=0 loop_end=48");
        assert!(looping.loops());
        assert!(!looping.ignores_note_off());
        assert_eq!(looping.loop_window.map(LoopWindow::len), Some(48));

        // `loop_sustain` 也必须被 `loops()` 认作循环（两种循环模式都要覆盖）。
        let sustain =
            spec_of("<region>sample=a.wav loop_mode=loop_sustain loop_start=0 loop_end=64");
        assert!(sustain.loops());
        assert!(!sustain.ignores_note_off());

        // 要求循环但窗口无效 ⇒ `loops()` 为 false（降级为播一遍）。
        let broken_window =
            spec_of("<region>sample=a.wav loop_mode=loop_continuous loop_start=9 loop_end=9");
        assert!(!broken_window.loops());
        assert_eq!(broken_window.loop_window, None);
    }

    #[test]
    fn playback_spec_carries_the_region_fields_and_is_deterministic() {
        let instrument = parse_text(
            "<region>sample=a.wav key=36 pitch_keycenter=60 transpose=-2 tune=50 \
             bend_up=1200 bend_down=1200 \
             volume=-3 pan=-25 loop_mode=loop_sustain loop_start=5 loop_end=105 group=7 off_by=9",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        let spec = region.playback_spec(36, 111, RATES_EQUAL);
        assert_eq!(spec.note, 36);
        assert_eq!(spec.velocity, 111);
        assert_eq!(spec.pitch_keycenter, 60);
        assert_eq!(spec.transpose, -2);
        assert_eq!(spec.tune, 50);
        // `bend_down=1200` 是语料里真实出现的正取值（19 次），必须原样带出、不被钳制。
        assert_eq!(spec.bend_up, 1200);
        assert_eq!(spec.bend_down, 1200);
        assert_eq!(spec.volume_db, -3.0);
        assert_eq!(spec.pan, -25.0);
        assert_eq!(spec.loop_mode, LoopMode::LoopSustain);
        assert_eq!(spec.loop_window.map(LoopWindow::len), Some(100));
        assert_eq!(spec.group, 7);
        assert_eq!(spec.off_by, 9);
        assert_eq!(spec.source_line, region.source_line);
        assert!(close(spec.gain, 10.0f32.powf(-3.0 / 20.0), 1.0e-6));
        assert!(close(spec.rate, region.pitch_ratio(36), 1.0e-9));
        // 同一输入 ⇒ 同一描述（ARCH-DET-001）。
        assert_eq!(spec, region.playback_spec(36, 111, RATES_EQUAL));
    }

    #[test]
    fn playback_for_selects_the_same_region_as_region_for() {
        let text = "<group>key=36 seq_length=2\n\
                    <region>seq_position=1 sample=k1.wav\n\
                    <region>seq_position=2 sample=k2.wav";
        let instrument = parse_text(text, &Default::default()).expect("parses");
        let rates = RenderRates::new(44_100.0, 48_000.0);

        let first = instrument
            .playback_for(RegionQuery::new(36, 100).with_occurrence(0), rates)
            .expect("region matches");
        assert_eq!(first.region.sample, "k1.wav");
        assert_eq!(first.spec.note, 36);
        assert_eq!(first.spec.velocity, 100);

        let second = instrument
            .playback_for(RegionQuery::new(36, 100).with_occurrence(1), rates)
            .expect("region matches");
        assert_eq!(second.region.sample, "k2.wav");

        // 与 region_for_with 的选择结果逐项一致（同一选择算法）。
        for occurrence in 0..6u64 {
            let query = RegionQuery::new(36, 100).with_occurrence(occurrence);
            let chosen = instrument.region_for_with(query);
            let played = instrument
                .playback_for(RegionQuery::new(36, 100).with_occurrence(occurrence), rates);
            assert_eq!(
                chosen.map(|region| region.sample.clone()),
                played.map(|play| play.region.sample.clone()),
                "occurrence {occurrence}"
            );
            assert_eq!(played.map(|play| play.spec.velocity), Some(100));
        }

        // 没有匹配 region ⇒ None（不 panic、不 fallback 到别的 region）。
        assert!(
            instrument
                .playback_for(RegionQuery::new(61, 100), rates)
                .is_none()
        );
        // CC 门控未提供状态 ⇒ 严格策略下不匹配。
        let gated = parse_text(
            "<region>sample=a.wav locc1=64 hicc1=127",
            &Default::default(),
        )
        .expect("parses");
        assert!(
            gated
                .playback_for(RegionQuery::new(60, 100), rates)
                .is_none()
        );
    }

    #[test]
    fn master_scope_reaches_the_playback_spec() {
        // 端到端：`<master>` 层的音量与根音必须出现在可渲染描述里。
        // 改动前 `<master>` 被忽略 ⇒ volume_db 取 global 的 -12、pitch_keycenter 取 60。
        let instrument = parse_text(
            "<global>volume=-12\n\
             <master>volume=-6 pitch_keycenter=48\n\
             <group>key=36\n\
             <region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        let play = instrument
            .playback_for(RegionQuery::new(36, 100), RATES_EQUAL)
            .expect("region matches note 36");
        assert_eq!(play.spec.volume_db, -6.0);
        assert!(close(play.spec.gain, 10.0f32.powf(-6.0 / 20.0), 1.0e-6));
        assert_eq!(play.spec.pitch_keycenter, 48);
        // 36 是根音 48 的低一个八度 ⇒ 音高比 0.5。
        assert_eq!(play.spec.pitch_ratio, 0.5);
        assert!(close(play.spec.rate, 0.5, 1.0e-9));
    }

    #[test]
    fn pitch_ratio_is_finite_positive_over_the_parsed_field_ranges() {
        // 字段范围来自 instrument.rs 的字段声明：note 0..=127、pitch_keycenter -127..=127、
        // transpose -127..=127、tune -100..=100（解析期已强制）。
        let mut ratios = Vec::new();
        for note in [0u8, 1, 60, 64, 127] {
            for pitch_keycenter in [-127i32, 0, 60, 127] {
                for transpose in [-127i32, -1, 0, 1, 127] {
                    for tune in [-100i32, 0, 100] {
                        let region = Region {
                            sample: std::borrow::Cow::Borrowed("a.wav"),
                            default_path: None,
                            lokey: 0,
                            hikey: 127,
                            pitch_keycenter,
                            trigger_by_note: true,
                            trigger: Trigger::Attack,
                            off_mode: OffMode::Fast,
                            off_time: None,
                            lovel: 0,
                            hivel: 127,
                            lochan: 1,
                            hichan: 16,
                            amp_veltrack: crate::velocity::AMP_VELTRACK_DEFAULT,
                            velocity_curve: None,
                            loop_start: 0,
                            loop_end: 0,
                            loop_mode: LoopMode::NoLoop,
                            offset: 0,
                            end: SampleEnd::Unspecified,
                            direction: PlayDirection::Forward,
                            tune,
                            transpose,
                            bend_up: crate::BEND_UP_DEFAULT_CENTS,
                            bend_down: crate::BEND_DOWN_DEFAULT_CENTS,
                            volume: 0.0,
                            pan: 0.0,
                            seq_position: 1,
                            seq_length: 1,
                            group: 0,
                            off_by: 0,
                            note_polyphony: NotePolyphony::UNLIMITED,
                            sw_last: None,
                            sw_default: None,
                            sw_lokey: 0,
                            sw_hikey: 127,
                            sw_down: None,
                            sw_up: None,
                            cc_gates: Vec::new(),
                            crossfades: Vec::new(),
                            labels: crate::label::Labels::default(),
                            source_line: 1,
                        };
                        let ratio = region.pitch_ratio(note);
                        assert!(
                            ratio.is_finite() && ratio > 0.0,
                            "ratio {ratio} for note {note} keycenter {pitch_keycenter} \
                             transpose {transpose} tune {tune}"
                        );
                        ratios.push(ratio);
                    }
                }
            }
        }
        assert_eq!(ratios.len(), 5 * 4 * 5 * 3);
        assert!(ratios.iter().all(|ratio| ratio.is_finite()));
    }

    #[test]
    fn playback_span_is_half_open_from_an_inclusive_end() {
        let span_of = |text: &str| {
            parse_text(text, &Default::default())
                .expect("parses")
                .regions()[0]
                .playback_span()
        };
        // 缺省：从采样点 0 播到采样末尾（终点只能由解码器给出）。
        assert_eq!(
            span_of("<region>sample=a.wav"),
            Some(SampleSpan {
                start: 0,
                end: None
            })
        );
        assert_eq!(
            span_of("<region>sample=a.wav").and_then(SampleSpan::len),
            None
        );
        // `end` **含**端点 ⇒ 半开区间终点 = end + 1（出处 .../opcodes/end/）。
        let span = span_of("<region>sample=a.wav offset=100 end=199").expect("span");
        assert_eq!(
            span,
            SampleSpan {
                start: 100,
                end: Some(200)
            }
        );
        assert_eq!(span.len(), Some(100));
        assert!(!span.is_empty());
        // 单点区间：`offset == end` 仍然读 1 个采样点（含端点的直接推论）。
        assert_eq!(
            span_of("<region>sample=a.wav offset=7 end=7").and_then(SampleSpan::len),
            Some(1)
        );
        // `end=-1` ⇒ 不发声。
        assert_eq!(span_of("<region>sample=a.wav end=-1"), None);
        // 显式终点落在 offset 之前 ⇒ 没有采样点可读（不返回反向区间，不 panic）。
        assert_eq!(span_of("<region>sample=a.wav offset=200 end=100"), None);
        assert_eq!(span_of("<region>sample=a.wav offset=1 end=0"), None);
        // 规范上界：`end=4294967295` 的半开终点是 4294967296，必须用 u64 容纳。
        let top = span_of("<region>sample=a.wav offset=4294967295 end=4294967295").expect("span");
        assert_eq!(
            top,
            SampleSpan {
                start: u64::from(u32::MAX),
                end: Some(4_294_967_296),
            }
        );
        assert_eq!(top.len(), Some(1));
        // 手工构造出 `end < start` 的区间也不 panic（`saturating_sub`）。
        assert_eq!(
            SampleSpan {
                start: 10,
                end: Some(4)
            }
            .len(),
            Some(0)
        );
    }

    #[test]
    fn playback_spec_carries_offset_end_direction_and_silence() {
        let instrument = parse_text(
            "<region>sample=a.wav key=36 offset=100 end=199 direction=reverse",
            &Default::default(),
        )
        .expect("parses");
        let play = instrument
            .playback_for(RegionQuery::new(36, 100), RATES_EQUAL)
            .expect("region matches note 36");
        assert_eq!(play.spec.offset, 100);
        assert_eq!(play.spec.end, SampleEnd::Inclusive(199));
        assert_eq!(play.spec.direction, PlayDirection::Reverse);
        assert!(play.spec.plays_reverse());
        assert!(!play.spec.is_silent());
        assert_eq!(
            play.spec.span,
            Some(SampleSpan {
                start: 100,
                end: Some(200)
            })
        );

        // `end=-1` 的静音 region 必须**仍然**被选中：它是 `group` / `off_by` 的互斥源。
        let silent = parse_text(
            "<region>sample=silence.wav end=-1 group=5 off_by=5",
            &Default::default(),
        )
        .expect("parses");
        let muted = silent
            .playback_for(RegionQuery::new(60, 100), RATES_EQUAL)
            .expect("a silent region is still triggered");
        assert!(muted.spec.is_silent());
        assert_eq!(muted.spec.span, None);
        assert_eq!((muted.spec.group, muted.spec.off_by), (5, 5));
        assert!(!muted.spec.plays_reverse());
    }

    #[test]
    fn playback_spec_carries_the_trigger_and_forces_one_shot_for_releases() {
        // `PlaybackSpec` 是实时路径唯一读取的结构：`trigger` 原样带出，
        // `loop_mode` 带出**生效值**（release 家族强制 one_shot）。
        let instrument = parse_text(
            "<region>sample=release.wav trigger=release loop_mode=loop_continuous \
             loop_start=10 loop_end=20",
            &Default::default(),
        )
        .expect("parses");
        let play = instrument
            .playback_for(RegionQuery::note_off(60, 100, false), RATES_EQUAL)
            .expect("note-off selects the release region");
        assert_eq!(play.spec.trigger, Trigger::Release);
        assert_eq!(play.spec.loop_mode, LoopMode::OneShot, "forced by trigger");
        assert_eq!(play.spec.loop_window, None);
        assert!(!play.spec.loops());
        assert!(play.spec.ignores_note_off());

        // note-on 对同一个乐器没有可播的 region：release region 被事件门控挡掉。
        assert!(
            instrument
                .playback_for(RegionQuery::new(60, 100), RATES_EQUAL)
                .is_none()
        );

        // `trigger=attack` 的对照：loop_mode 原样，循环窗口有效。
        let attack = parse_text(
            "<region>sample=a.wav loop_mode=loop_continuous loop_start=10 loop_end=20",
            &Default::default(),
        )
        .expect("parses");
        let looping = attack
            .playback_for(RegionQuery::new(60, 100), RATES_EQUAL)
            .expect("note-on selects the attack region");
        assert_eq!(looping.spec.trigger, Trigger::Attack);
        assert_eq!(looping.spec.loop_mode, LoopMode::LoopContinuous);
        assert_eq!(
            looping.spec.loop_window,
            Some(LoopWindow { start: 10, end: 20 })
        );
        assert!(looping.spec.loops());
        assert!(!looping.spec.ignores_note_off());
    }

    #[test]
    fn playback_spec_carries_off_mode_and_derives_the_note_off_contract() {
        // 缺省 `fast`：note-off 到达后可以立刻切断声部。
        let fast = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        let spec = fast.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec.off_mode, OffMode::Fast);
        assert!(
            spec.cuts_at_note_off(),
            "off_mode=fast must allow an immediate cut at note-off"
        );

        // `normal` / `time` 要求 release / `off_time` 保持段：不得立刻切断。
        // 二者在登记语料里分别是 818 与 4 次，缺省 `fast` 只有 87 次
        // （`docs/ledger/sfz-core-notes.md` 第 11 节）。
        for (text, expected) in [("normal", OffMode::Normal), ("time", OffMode::Time)] {
            let source = format!("<region>sample=a.wav off_mode={text}");
            let instrument = parse_text(&source, &Default::default()).expect("parses");
            let spec = instrument.regions()[0].playback_spec(60, 100, RATES_EQUAL);
            assert_eq!(spec.off_mode, expected, "off_mode={text}");
            assert!(
                !spec.cuts_at_note_off(),
                "off_mode={text} must not allow an immediate cut"
            );
        }
    }

    #[test]
    fn off_mode_is_orthogonal_to_ignores_note_off() {
        // `loop_mode=one_shot` 让 note-off 不结束声部；`off_mode` 仍是原样取值，
        // 两个谓词互不派生。
        let instrument = parse_text(
            "<region>sample=a.wav loop_mode=one_shot off_mode=normal",
            &Default::default(),
        )
        .expect("parses");
        let spec = instrument.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert!(spec.ignores_note_off());
        assert_eq!(spec.off_mode, OffMode::Normal);
        assert!(!spec.cuts_at_note_off());
    }

    #[test]
    fn playback_spec_carries_off_time_and_resolves_the_spec_default() {
        // 缺省：`off_time` 原样是 `None`，生效值取规范缺省 0.006 秒
        // （<https://sfzformat.com/opcodes/off_time/> 的 Default 列）。
        let default =
            parse_text("<region>sample=a.wav off_mode=time", &Default::default()).expect("parses");
        let spec = default.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec.off_time, None);
        assert_eq!(spec.effective_off_time(), 0.006);

        // 显式给出：原样带出，且与缺省值不同 —— 该字段不是恒等回退。
        let explicit = parse_text(
            "<region>sample=a.wav off_mode=time off_time=0.25",
            &Default::default(),
        )
        .expect("parses");
        let spec = explicit.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec.off_time, Some(0.25));
        assert_eq!(spec.effective_off_time(), 0.25);
        assert_ne!(spec.effective_off_time(), crate::OFF_TIME_DEFAULT_SECONDS);
    }

    #[test]
    fn playback_spec_carries_the_note_polyphony_policy() {
        // 缺省：不限制，`self_mask` 是规范缺省的 `on`。
        let default = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        let spec = default.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec.note_polyphony, NotePolyphony::UNLIMITED);
        assert!(spec.note_polyphony.is_unlimited());

        // 显式：限制量与掩蔽规则都原样带出；检查用的键是 `group`。
        let explicit = parse_text(
            "<master>note_polyphony=3 note_selfmask=off\n<region>sample=a.wav group=7",
            &Default::default(),
        )
        .expect("parses");
        let spec = explicit.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec.note_polyphony.limit, 3);
        assert!(!spec.note_polyphony.self_mask);
        assert_eq!(spec.group, 7, "the polyphony key is the group opcode");
    }

    #[test]
    fn off_time_is_independent_of_the_steal_fade_constant() {
        // 裁决留痕：`off_time` 是 per-region 的关断时长，[ARCH-RT-004] 的 3 ms 是
        // 声部窃取淡出的工程常量。本条判据钉住"建模 `off_time` 不改写那个常量"。
        let steal_millis = crate::STEAL_FADE_MILLIS;
        let instrument = parse_text(
            "<region>sample=a.wav off_mode=time off_time=0.05",
            &Default::default(),
        )
        .expect("parses");
        let spec = instrument.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec.effective_off_time(), 0.05);
        assert_eq!(
            crate::STEAL_FADE_MILLIS,
            steal_millis,
            "the region-level off_time must not rewrite the ARCH-RT-004 steal fade"
        );
    }

    #[test]
    fn playback_spec_carries_the_velocity_gain_without_touching_gain() {
        // 分段口径：`gain` 只含 volume，力度单独走 `velocity_gain`。
        let instrument = parse_text(
            "<region>sample=a.wav volume=-6 amp_velcurve_1=0.2 amp_velcurve_3=0.3",
            &Default::default(),
        )
        .expect("parses");
        let spec = instrument.regions()[0].playback_spec(60, 2, RATES_EQUAL);
        assert_eq!(spec.volume_db, -6.0);
        // `gain` 与力度无关：不同力度下同一个值。
        let other = instrument.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec.gain, other.gain);
        assert_eq!(other.gain, instrument.regions()[0].linear_gain());
        // `velocity_gain` 才是随力度变的那个：规范算例 amp_velcurve_2 = 0.25。
        assert_eq!(spec.velocity_gain, 0.25);
        assert!(other.velocity_gain > 0.25);
        // 合并值只在 total_gain() 里出现。
        assert_eq!(spec.total_gain(), spec.gain * 0.25);
        assert_ne!(spec.total_gain(), spec.gain);
    }

    #[test]
    fn playback_spec_defaults_velocity_to_the_amp_veltrack_curve() {
        let instrument =
            parse_text("<region>sample=a.wav amp_veltrack=0", &Default::default()).expect("parses");
        let spec = instrument.regions()[0].playback_spec(60, 32, RATES_EQUAL);
        assert_eq!(spec.amp_veltrack, 0.0);
        assert_eq!(spec.velocity_gain, 1.0, "amp_veltrack=0 means no tracking");
        assert_eq!(spec.total_gain(), spec.gain);

        let default = parse_text("<region>sample=a.wav", &Default::default()).expect("parses");
        let spec = default.regions()[0].playback_spec(60, 32, RATES_EQUAL);
        assert_eq!(spec.amp_veltrack, crate::AMP_VELTRACK_DEFAULT);
        assert!(spec.velocity_gain < 1.0, "the default curve attenuates");
        assert_eq!(spec.velocity_gain, default.regions()[0].velocity_gain(32));
    }

    #[test]
    fn playback_for_and_playback_spec_agree_on_the_velocity_gain() {
        let instrument = parse_text(
            "<region>key=36 sample=a.wav amp_velcurve_1=0.4",
            &Default::default(),
        )
        .expect("parses");
        let played = instrument
            .playback_for(RegionQuery::new(36, 100), RATES_EQUAL)
            .expect("region covers note 36");
        assert_eq!(played.spec.velocity_gain, played.region.velocity_gain(100));
        assert_eq!(played.spec.velocity, 100);
    }

    #[test]
    fn a_region_without_crossfades_keeps_total_gain_bit_identical() {
        // 没有 `xfin_*` / `xfout_*` 的 region：新因子恰为 1.0，合并值逐位不变。
        let instrument = parse_text(
            "<region>sample=a.wav volume=-6 amp_velcurve_1=0.2 amp_velcurve_3=0.3",
            &Default::default(),
        )
        .expect("parses");
        let spec = instrument.regions()[0].playback_spec(60, 2, RATES_EQUAL);
        assert_eq!(spec.crossfade_gain, 1.0);
        assert_eq!(
            spec.total_gain().to_bits(),
            (spec.gain * spec.velocity_gain).to_bits()
        );
        assert_eq!(spec.total_gain(), spec.gain * 0.25);
    }

    #[test]
    fn playback_spec_carries_the_crossfade_gain_of_the_velocity_axis() {
        let instrument = parse_text(
            "<region>sample=a.wav amp_veltrack=0 xfin_lovel=0 xfin_hivel=100",
            &Default::default(),
        )
        .expect("parses");
        // 力度 50 在等功率曲线的中点是 sqrt(0.5)。
        let spec = instrument.regions()[0].playback_spec(60, 50, RATES_EQUAL);
        assert!((spec.crossfade_gain - 0.5f32.sqrt()).abs() <= 1.0e-6);
        assert_eq!(
            spec.total_gain(),
            spec.gain * spec.velocity_gain * 0.5f32.sqrt()
        );
        assert!(spec.total_gain() < spec.gain);
        // 上界处是满幅：交叉淡化不改变音量。
        let spec = instrument.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec.crossfade_gain, 1.0);
    }

    #[test]
    fn playback_spec_without_cc_state_leaves_the_cc_axis_alone() {
        // 无 CC 状态的重载（`playback_spec`）只折键盘与力度两个轴。
        let instrument = parse_text(
            "<region>sample=a.wav amp_veltrack=0 xfin_hicc1=100",
            &Default::default(),
        )
        .expect("parses");
        let spec = instrument.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec.crossfade_gain, 1.0);
        // 借用版能看到 CC：读数 0 ⇒ 该 region 静音。
        let probe = |_: u8| 0u8;
        let query = RegionQuery::new(60, 100).with_cc(&probe);
        let spec = instrument.regions()[0].playback_spec_with(&query, RATES_EQUAL);
        assert_eq!(spec.crossfade_gain, 0.0);
        assert_eq!(spec.total_gain(), 0.0);
    }

    #[test]
    fn playback_for_passes_the_cc_state_into_the_crossfade_gain() {
        // 端到端：`playback_for` 的一次查询同时喂给选择与交叉淡化。
        let instrument = parse_text(
            "<region>sample=a.wav amp_veltrack=0 xfin_hicc1=100",
            &Default::default(),
        )
        .expect("parses");
        for cc in [0u8, 50, 100, 127] {
            let probe = move |_: u8| cc;
            let play = instrument
                .playback_for(RegionQuery::new(60, 100).with_cc(&probe), RATES_EQUAL)
                .expect("region matches");
            assert_eq!(
                play.spec.crossfade_gain.to_bits(),
                play.region
                    .crossfade_gain(&RegionQuery::new(60, 100).with_cc(&probe))
                    .to_bits(),
                "cc {cc}"
            );
        }
        let silent = |_: u8| 0u8;
        let play = instrument
            .playback_for(RegionQuery::new(60, 100).with_cc(&silent), RATES_EQUAL)
            .expect("region matches");
        assert_eq!(play.spec.crossfade_gain, 0.0);
        // 没有 CC 状态时仍然选中同一个 region，只是该轴不贡献衰减。
        let play = instrument
            .playback_for(RegionQuery::new(60, 100), RATES_EQUAL)
            .expect("region matches");
        assert_eq!(play.spec.crossfade_gain, 1.0);
    }

    // ------------------------------------------------------------------
    // 幂等性（类别 5）：同一对象上重复施加同一个值
    // ------------------------------------------------------------------

    #[test]
    fn sanitized_is_a_fixed_point_and_playback_rate_is_invariant_under_it() {
        // 「只施加一次」与「施加两次」相同：净化过的采样率对再净化是恒等（逐位）。
        let instrument = parse_text(
            "<region>sample=a.wav pitch_keycenter=60",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        for rates in [
            RenderRates::new(44_100.0, 48_000.0),
            RenderRates::new(f32::NAN, 48_000.0),
            RenderRates::new(0.0, -1.0),
            RenderRates::new(f32::INFINITY, f32::NEG_INFINITY),
            RenderRates::new(-44_100.0, 0.0),
            RenderRates::default(),
        ] {
            let once = rates.sanitized();
            let twice = once.sanitized();
            assert_eq!(twice, once, "{rates:?} must be a fixed point");
            assert_eq!(once.sample_hz.to_bits(), twice.sample_hz.to_bits());
            assert_eq!(once.engine_hz.to_bits(), twice.engine_hz.to_bits());
            // 预先净化一次与不净化：步进比逐位相同（第二次净化不再改变任何东西）。
            assert_eq!(
                region.playback_rate(72, rates).to_bits(),
                region.playback_rate(72, once).to_bits(),
                "{rates:?}"
            );
            assert_eq!(
                region.playback_rate(72, once).to_bits(),
                region.playback_rate(72, twice).to_bits(),
                "{rates:?}"
            );
        }
    }

    #[test]
    fn repeating_a_spec_build_is_bit_identical() {
        // 同一 region + 同一 query + 同一采样率：两次构造逐位相同（三个构造入口都覆盖）。
        let source = "<region>sample=a.wav key=36 pitch_keycenter=60 transpose=-2 tune=50 \
                      bend_up=1200 bend_down=1200 volume=-3 pan=-25 \
                      loop_mode=loop_sustain loop_start=5 loop_end=105 \
                      amp_veltrack=75 xfin_lokey=40 xfin_hikey=80 group=7 off_by=9";
        let instrument = parse_text(source, &Default::default()).expect("parses");
        let region = &instrument.regions()[0];
        let rates = RenderRates::new(44_100.0, 48_000.0);
        let query = RegionQuery::new(36, 111);

        let first = region.playback_spec_with(&query, rates);
        let second = region.playback_spec_with(&query, rates);
        for (name, left, right) in [
            ("pitch_ratio", first.pitch_ratio, second.pitch_ratio),
            ("rate", first.rate, second.rate),
            ("volume_db", first.volume_db, second.volume_db),
            ("gain", first.gain, second.gain),
            ("amp_veltrack", first.amp_veltrack, second.amp_veltrack),
            ("velocity_gain", first.velocity_gain, second.velocity_gain),
            (
                "crossfade_gain",
                first.crossfade_gain,
                second.crossfade_gain,
            ),
            ("pan", first.pan, second.pan),
            ("total_gain", first.total_gain(), second.total_gain()),
        ] {
            assert_eq!(
                left.to_bits(),
                right.to_bits(),
                "{name} must be bit identical"
            );
        }
        // 两个构造器（标量版 / 借用版）在同一次查询上给出同一个描述。
        assert_eq!(first, region.playback_spec(36, 111, rates));

        // 端到端入口重复两次：同一个 region、逐位相同的描述。
        let played_once = instrument
            .playback_for(RegionQuery::new(36, 111), rates)
            .expect("region matches");
        let played_twice = instrument
            .playback_for(RegionQuery::new(36, 111), rates)
            .expect("region matches");
        assert_eq!(played_once.region.sample, played_twice.region.sample);
        assert_eq!(played_once.spec, played_twice.spec);
        for (left, right) in [
            (played_once.spec.rate, played_twice.spec.rate),
            (played_once.spec.gain, played_twice.spec.gain),
            (
                played_once.spec.velocity_gain,
                played_twice.spec.velocity_gain,
            ),
            (
                played_once.spec.crossfade_gain,
                played_twice.spec.crossfade_gain,
            ),
        ] {
            assert_eq!(left.to_bits(), right.to_bits());
        }
    }

    #[test]
    fn a_repeated_identical_declaration_leaves_the_reduced_form_unchanged() {
        // 解析层的同值重复：同一个作用域里把同一个 opcode 写成同一个值两次 = 只写一次
        // （`OpcodeMap` 是 `BTreeMap`，后写的同值覆盖前者）。读数落在归约结果上。
        let once =
            parse_text("<region>sample=a.wav volume=-6", &Default::default()).expect("parses");
        let twice = parse_text(
            "<region>sample=a.wav volume=-6 volume=-6",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(
            once.regions(),
            twice.regions(),
            "same opcode, same value, twice"
        );
        let spec_once = once.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        let spec_twice = twice.regions()[0].playback_spec(60, 100, RATES_EQUAL);
        assert_eq!(spec_once, spec_twice);
        assert_eq!(spec_once.gain.to_bits(), spec_twice.gain.to_bits());

        // `<control>` 的两个文件级声明族同样如此（`set_ccN` 与 `label_ccN`）。
        let control_once = parse_text(
            "<control>set_cc7=100 label_cc7=Volume\n<region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        let control_twice = parse_text(
            "<control>set_cc7=100 set_cc7=100 label_cc7=Volume label_cc7=Volume\n\
             <region>sample=a.wav",
            &Default::default(),
        )
        .expect("parses");
        assert_eq!(control_once.cc_defaults(), control_twice.cc_defaults());
        assert_eq!(control_once.cc_labels(), control_twice.cc_labels());
        assert_eq!(control_twice.cc_defaults().get(&7), Some(&100));
        assert_eq!(
            control_twice
                .cc_labels()
                .get(&7)
                .map(|label| label.as_ref()),
            Some("Volume")
        );
    }

    // ------------------------------------------------------------------
    // 多声道一致性（类别 6）：同一个信号喂给 N 路
    // ------------------------------------------------------------------
    //
    // 本 crate **不含**音频缓冲（没有任何逐路样本数据），因此没有左右声道这回事：它唯一的
    // 「多路」结构是 `<effect>` 的 4 路发送量与这里的 MIDI 通道轴。本节把「同一个信号
    // 喂给每一路 ⇒ 各路输出逐位相同」与「只填一路 ⇒ 其余路不被污染」这两条搬到该轴上。

    #[test]
    fn the_same_signal_on_every_matching_channel_is_bit_identical() {
        // 缺省 region 覆盖 1..=16 全部通道：16 路的选中结果与描述必须逐位相同。
        let instrument = parse_text(
            "<region>sample=a.wav key=60 pitch_keycenter=60 volume=-3",
            &Default::default(),
        )
        .expect("parses");
        let baseline = instrument
            .playback_for(RegionQuery::new(60, 100).with_channel(1), RATES_EQUAL)
            .expect("channel 1 matches");
        for channel in 1..=16u8 {
            let play = instrument
                .playback_for(RegionQuery::new(60, 100).with_channel(channel), RATES_EQUAL)
                .expect("a default region covers every MIDI channel");
            assert_eq!(
                play.region.sample, baseline.region.sample,
                "channel {channel}"
            );
            assert_eq!(
                play.spec.gain.to_bits(),
                baseline.spec.gain.to_bits(),
                "channel {channel} must not change the gain"
            );
            assert_eq!(
                play.spec.rate.to_bits(),
                baseline.spec.rate.to_bits(),
                "channel {channel} must not change the rate"
            );
            assert_eq!(
                play.spec, baseline.spec,
                "channel {channel}: the spec carries no per-channel state"
            );
        }
    }

    #[test]
    fn a_single_channel_region_matches_exactly_one_channel_and_never_bleeds() {
        let instrument = parse_text(
            "<region>sample=a.wav key=60 lochan=5 hichan=5",
            &Default::default(),
        )
        .expect("parses");
        for channel in 1..=16u8 {
            let play = instrument
                .playback_for(RegionQuery::new(60, 100).with_channel(channel), RATES_EQUAL);
            if channel == 5 {
                assert!(play.is_some(), "channel 5 is inside [5, 5]");
            } else {
                assert!(
                    play.is_none(),
                    "channel {channel} must not bleed into a channel-5 region"
                );
            }
        }
        // 域外通道值不是「静默落到某一路」：只返回 None，绝不 panic。
        for channel in [0u8, 17, 128, 255] {
            assert!(
                instrument
                    .playback_for(RegionQuery::new(60, 100).with_channel(channel), RATES_EQUAL)
                    .is_none(),
                "channel {channel} is outside the 1..=16 MIDI domain"
            );
        }
    }

    #[test]
    fn a_channel_span_matches_exactly_its_lanes() {
        // 只给一端的两半：`lochan=5` ⇒ 5..=16；`hichan=5` ⇒ 1..=5。
        let upper = parse_text("<region>sample=a.wav key=60 lochan=5", &Default::default())
            .expect("parses");
        let lower = parse_text("<region>sample=a.wav key=60 hichan=5", &Default::default())
            .expect("parses");
        for channel in 1..=16u8 {
            let in_upper = upper
                .playback_for(RegionQuery::new(60, 100).with_channel(channel), RATES_EQUAL)
                .is_some();
            assert_eq!(
                in_upper,
                channel >= 5,
                "lochan=5 must cover 5..=16 (channel {channel})"
            );
            let in_lower = lower
                .playback_for(RegionQuery::new(60, 100).with_channel(channel), RATES_EQUAL)
                .is_some();
            assert_eq!(
                in_lower,
                channel <= 5,
                "hichan=5 must cover 1..=5 (channel {channel})"
            );
        }
        // 两半的并集覆盖全部 16 路、交集只有第 5 路；两半在同一路（第 5 路）上的描述逐位相同。
        let left = upper
            .playback_for(RegionQuery::new(60, 100).with_channel(5), RATES_EQUAL)
            .expect("channel 5 is in both spans");
        let right = lower
            .playback_for(RegionQuery::new(60, 100).with_channel(5), RATES_EQUAL)
            .expect("channel 5 is in both spans");
        assert_eq!(left.spec, right.spec);
        assert_eq!(left.spec.gain.to_bits(), right.spec.gain.to_bits());
    }

    #[test]
    fn an_inverted_channel_range_matches_no_lane_and_never_panics() {
        // 「反向区间」在别的轴上已有同一条口径（`lokey > hikey` 也是永不匹配、
        // 不发明「自动交换两端」的语义，见 `build_region` / `matches_key`）。
        let instrument = parse_text(
            "<region>sample=a.wav key=60 lochan=5 hichan=3",
            &Default::default(),
        )
        .expect("parses");
        let region = &instrument.regions()[0];
        assert_eq!((region.lochan, region.hichan), (5, 3));
        for channel in 0..=255u8 {
            assert!(
                !region.matches_channel(channel),
                "an inverted channel span must match nothing (channel {channel})"
            );
        }
        for channel in [0u8, 1, 3, 4, 5, 16, 255] {
            assert!(
                instrument
                    .playback_for(RegionQuery::new(60, 100).with_channel(channel), RATES_EQUAL)
                    .is_none()
            );
        }
        // 反向区间不改变「同一音符仍在键桶里」这件事：它只是每一路都匹配失败。
        assert!(region.matches_key(60));
    }
}
