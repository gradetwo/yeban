//! 设备参数落点表（任务单 `#267` / `#293` / `#304`；本模块由 `yeban-render` 下移）。
//!
//! ## 为什么住在 `yeban-dsp`
//!
//! 依赖方向是 `render → engine → dsp`：离线渲染链（`yeban-render`）与实时插入链
//! （`yeban-engine`）都要读**同一组**"哪个名字有落点、落点字段怎么钳制"的规则，
//! 而它们唯一共同的下游是 `yeban-dsp`。这张表放在任何一侧都会让另一侧要么复制一份、
//! 要么反向依赖 ⇒ 只能下移到这里。
//!
//! ## 表的内容
//!
//! 四个**器件族**各一张字段表 ＋ 一张把四个族合起来的落点枚举：
//!
//! | 类型 | 内容 |
//! | :--- | :--- |
//! | [`StripField`] | 通道条 28 个可自动化字段（`53` 个拼写） |
//! | [`ReverbField`] | Freeverb 的 4 个标量字段（`6` 个拼写） |
//! | [`ConvolutionField`] | 卷积混响的 4 个标量字段（`8` 个拼写；IR 形状不落点） |
//! | [`DelayField`] | 乒乓延迟的 9 个字段（`15` 个拼写；音符同步四字段只在投影期换算） |
//! | [`TapeField`] | 磁带饱和的 5 个标量字段（`10` 个拼写） |
//! | [`DeviceParamField`] / [`DeviceParamFamily`] | 四族的合流枚举与器件族标签 |
//!
//! ⛔ **本模块不含**任何 I/O、模型类型或 GUI：每个 `sanitise` 都与器件内部**同一次**
//! 钳制对齐（[`ChannelStripParams::sanitised`] / [`crate::reverb::Reverb::set_params`] /
//! [`crate::convolution_reverb::ConvolutionReverb::set_params`]），`read`/`store` 只读写这三个参数结构。
//! 名字表的消费方（静态投影 `strip_of` / `tape_of` / `reverb_of` / `convolution_of`、离线施加、
//! 实时块边界落点）都在 `yeban-render` 与 `yeban-engine`，它们⛔ 不得再造第二张表。

use crate::channel_strip::ChannelStripParams;
use crate::convolution_reverb::{
    ConvolutionReverbParams, MAX_IR_GAIN_DB, MAX_PRE_DELAY_SECONDS, MIN_IR_GAIN_DB,
};
use crate::delay::{DelayParams, MAX_DELAY_SECONDS};
use crate::reverb::{REVERB_MAX_PREDELAY_S, ReverbParams};
use crate::tape::TapeParams;

/// 通道条**实际会用的**输出增益值（`dB`）—— 与器件内部同一次钳制
/// （[`ChannelStripParams::sanitised`]，非有限值归 `0 dB`）。
///
/// 设备参数自动化的恒等判定必须比这个值，而不是比原始参数：`-100 dB` 与
/// `-60 dB` 交给器件是**同一个值**，把它们当"不同"会让恒等快路径漏掉一次
/// 不必要的器件实例化。
/// 任务单 `#267` 起它只是 [`StripField::sanitise`] 的一个特例（⛔ 不是第二套钳制）。
#[must_use]
pub fn sanitised_output_gain_db(value: f32) -> f32 {
    StripField::OutputGainDb.sanitise(value)
}

/// 通道条上**可被设备参数自动化落点**的字段（任务单 `#267`；`#304` 扩压缩）。
///
/// 与 `yeban_render::inserts::StripProjection::apply` 的**通道条分支一一对应**：同一组名字、同一个别名表、
/// 同一个目标字段。⛔ 刻意**不含**：
///
/// - **混响 / 卷积**（`reverb_*` / `conv_*`）：它们不是**通道条**字段，而是
///   [`ReverbField`] / [`ConvolutionField`] 的落点（`#293`）。其中卷积的 IR 形状
///   （`conv_ir_decay*` / `conv_ir_seed` / `conv_ir_stereo`）是构造期烘焙、不可自动化；其余标量由对应
///   器件的 `set_params` 在块边界改（⛔ 不是 `ChannelStrip::set_params`）；
/// - 未知旋钮。
///
/// ⭐ **压缩 8 字段自 `#304` 起在本表里**（14 个拼写：`compressor_enabled`、
/// `threshold_db` / `threshold`、`ratio`、`knee_db` / `knee`、`detector_s` /
/// `detector`、`attack_s` / `attack`、`release_s` / `release`、`makeup_db` /
/// `makeup`）。它们与增益 / EQ / 滤波共用**同一台** `ChannelStrip`：块边界写进
/// `ChannelStripParams`，`ChannelStrip::set_params` 转给 `Compressor::set_params`
/// （⚠ 该函数**不清弹道状态**）。`compressor_enabled` 是这一级的开关字段；
/// 静态关闭时，其它压缩字段的泳道仍会报 applied 但**听不出**（与 `eq_enabled: 0`
/// ＋ EQ 参数泳道同款，`#304` 明确登记）。
///
/// ## 单位（原生绝对值，⛔ 不归一化）
///
/// 字段名自带单位：`_db` = dB、`_hz` = Hz、`_s` = 秒、`resonance` / `drive` /
/// `mid_q` / `ratio` = 旋钮原值。
/// 开关字段（[`Self::EqEnabled`] / [`Self::FilterEnabled`] /
/// [`Self::CompressorEnabled`]）的自动化值是 `0.0` = 假、其它 = 真（与 `yeban_render::inserts::flag` 的
/// "非零即真"同式）—— 读数与写入都用这个口径。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StripField {
    /// 输入增益（trim，dB）。`input_gain_db` / `input_gain` / `trim_db`。
    InputGainDb,
    /// 输出增益（推子，dB）。`output_gain_db` / `output_gain` / `fader_db`。
    OutputGainDb,
    /// EQ 级开关。`eq_enabled`。
    EqEnabled,
    /// 滤波级开关。`filter_enabled`。
    FilterEnabled,
    /// EQ 低频增益（dB）。`eq_low_gain` / `low_gain`。
    EqLowGain,
    /// EQ 低频转折频率（Hz）。`eq_low_freq` / `low_freq`。
    EqLowFreq,
    /// EQ 中频增益（dB）。`eq_mid_gain` / `mid_gain`。
    EqMidGain,
    /// EQ 中频中心频率（Hz）。`eq_mid_freq` / `mid_freq`。
    EqMidFreq,
    /// EQ 中频 Q。`eq_mid_q` / `mid_q`。
    EqMidQ,
    /// EQ 高频增益（dB）。`eq_high_gain` / `high_gain`。
    EqHighGain,
    /// EQ 高频转折频率（Hz）。`eq_high_freq` / `high_freq`。
    EqHighFreq,
    /// 低通截止频率（Hz）。`cutoff_hz`（⚠ `filter_cutoff_hz` 与引擎一样**不认**）。
    CutoffHz,
    /// 低通谐振。`resonance` / `filter_resonance`。
    Resonance,
    /// 低通驱动。`drive` / `filter_drive`。
    Drive,
    /// 压缩级开关（`0.0` = 关，其它 = 开）。`compressor_enabled`。
    CompressorEnabled,
    /// 压缩阈值（dBFS）。`threshold_db` / `threshold`。
    ThresholdDb,
    /// 压缩比率（`≥ 1`）。`ratio`。
    Ratio,
    /// 压缩软膝宽度（dB）。`knee_db` / `knee`。
    KneeDb,
    /// 检波器平均窗口（秒）。`detector_s` / `detector`。
    DetectorS,
    /// 起振时间常数（秒）。`attack_s` / `attack`。
    AttackS,
    /// 释放时间常数（秒）。`release_s` / `release`。
    ReleaseS,
    /// makeup 补偿增益（dB）。`makeup_db` / `makeup`。
    MakeupDb,
    /// 低切转折频率（Hz，`<= 20` 旁通）。`hpf_hz` / `high_pass_hz`。
    HpfHz,
    /// 动态 EQ 的中心频率（Hz）。`deq_freq` / `dyn_eq_freq`。
    DynEqFreq,
    /// 动态 EQ 的触发阈值（dBFS）。`deq_threshold_db` / `dyn_eq_threshold_db`。
    DynEqThresholdDb,
    /// 动态 EQ 的最大衰减量（dB，`0` 旁通）。`deq_cut_db` / `dyn_eq_cut_db`。
    DynEqMaxCutDb,
    /// 立体声宽度（M/S 缩放，`1` = 原样）。`strip_width` / `channel_width`。
    ///
    /// ⛔ **不叫 `width`**：Freeverb 自己有一个 `width`（`ReverbField::Width`），
    /// 两条拼写撞车会让 `from_name` 把混响的宽度解析成通道条的宽度 —— 实测
    /// 那会让返回总线整体相位错乱（全曲相关度 0.81 → 0.00）。
    Width,
    /// mono-maker 的转折频率（Hz，`0` = 旁通）。`mono_below_hz` / `mono_maker_hz`。
    ///
    /// `#344`：把这一频率以下的 side 整段丢掉（`0 = 旁通`），让 120 Hz 以下钉在中间。
    /// ⛔ **不叫 `width` / `mono`**：`mono` 太泛（会被误当成"整条单声道开关"，
    /// 那是 `strip_width = 0`），而 `width` 已经归 [`Self::Width`]。
    MonoBelowHz,
}

impl StripField {
    /// 全部字段（每个字段的**规范名**见 [`Self::canonical_name`]）。
    pub const ALL: &'static [Self] = &[
        Self::InputGainDb,
        Self::OutputGainDb,
        Self::EqEnabled,
        Self::FilterEnabled,
        Self::EqLowGain,
        Self::EqLowFreq,
        Self::EqMidGain,
        Self::EqMidFreq,
        Self::EqMidQ,
        Self::EqHighGain,
        Self::EqHighFreq,
        Self::CutoffHz,
        Self::Resonance,
        Self::Drive,
        Self::CompressorEnabled,
        Self::ThresholdDb,
        Self::Ratio,
        Self::KneeDb,
        Self::DetectorS,
        Self::AttackS,
        Self::ReleaseS,
        Self::MakeupDb,
        Self::HpfHz,
        Self::DynEqFreq,
        Self::DynEqThresholdDb,
        Self::DynEqMaxCutDb,
        Self::Width,
        Self::MonoBelowHz,
    ];

    /// **全部拼写** → 字段（`51` 条；这是"哪个名字能落点"的**唯一**名字表）。
    ///
    /// 与 `yeban_render::inserts::StripProjection::apply` 的通道条分支逐字同集：7 个参数各有 2 个拼写，
    /// 两个增益各有 3 个，两个开关与 `cutoff_hz` 各 1 个；⭐ `#304` 起压缩的 8 个
    /// 字段共 14 个拼写也在这里（⛔ 不再有第二张 `STRIP_COMPRESSOR_SPELLINGS`）。
    /// `yeban_render::automation::APPLIED_DEVICE_PARAM_NAMES` 必须与本表**完全一致**
    /// （由 `the_applied_names_and_the_field_table_agree` 双向钉住）。
    pub const SPELLINGS: &'static [(&'static str, Self)] = &[
        ("input_gain_db", Self::InputGainDb),
        ("input_gain", Self::InputGainDb),
        ("trim_db", Self::InputGainDb),
        ("output_gain_db", Self::OutputGainDb),
        ("output_gain", Self::OutputGainDb),
        ("fader_db", Self::OutputGainDb),
        ("eq_enabled", Self::EqEnabled),
        ("filter_enabled", Self::FilterEnabled),
        ("eq_low_gain", Self::EqLowGain),
        ("low_gain", Self::EqLowGain),
        ("eq_low_freq", Self::EqLowFreq),
        ("low_freq", Self::EqLowFreq),
        ("eq_mid_gain", Self::EqMidGain),
        ("mid_gain", Self::EqMidGain),
        ("eq_mid_freq", Self::EqMidFreq),
        ("mid_freq", Self::EqMidFreq),
        ("eq_mid_q", Self::EqMidQ),
        ("mid_q", Self::EqMidQ),
        ("eq_high_gain", Self::EqHighGain),
        ("high_gain", Self::EqHighGain),
        ("eq_high_freq", Self::EqHighFreq),
        ("high_freq", Self::EqHighFreq),
        ("cutoff_hz", Self::CutoffHz),
        ("resonance", Self::Resonance),
        ("filter_resonance", Self::Resonance),
        ("drive", Self::Drive),
        ("filter_drive", Self::Drive),
        // ⭐ `#304`：压缩 8 字段 / 14 拼写（与 `StripProjection::apply` 同集）。
        ("compressor_enabled", Self::CompressorEnabled),
        ("threshold_db", Self::ThresholdDb),
        ("threshold", Self::ThresholdDb),
        ("ratio", Self::Ratio),
        ("knee_db", Self::KneeDb),
        ("knee", Self::KneeDb),
        ("detector_s", Self::DetectorS),
        ("detector", Self::DetectorS),
        ("attack_s", Self::AttackS),
        ("attack", Self::AttackS),
        ("release_s", Self::ReleaseS),
        ("release", Self::ReleaseS),
        ("makeup_db", Self::MakeupDb),
        ("makeup", Self::MakeupDb),
        ("hpf_hz", Self::HpfHz),
        ("high_pass_hz", Self::HpfHz),
        ("deq_freq", Self::DynEqFreq),
        ("dyn_eq_freq", Self::DynEqFreq),
        ("deq_threshold_db", Self::DynEqThresholdDb),
        ("dyn_eq_threshold_db", Self::DynEqThresholdDb),
        ("deq_cut_db", Self::DynEqMaxCutDb),
        ("dyn_eq_cut_db", Self::DynEqMaxCutDb),
        ("strip_width", Self::Width),
        ("channel_width", Self::Width),
        // `#344` mono-maker：一个字段两个拼写（`mono_below_hz` 是规范名）。
        ("mono_below_hz", Self::MonoBelowHz),
        ("mono_maker_hz", Self::MonoBelowHz),
    ];

    /// 名字 → 字段（**大小写不敏感**，别名表与 `yeban_render::inserts::StripProjection::apply` 逐字同集）。
    ///
    /// `None` = 本字段集不认这个名字（混响 / 卷积 / 未知旋钮）。⚠ 本函数是
    /// "哪个名字能落点"的唯一判定处：`yeban_render::automation::APPLIED_DEVICE_PARAM_NAMES`
    /// 是它的**可读清单**，两者由 `the_applied_names_and_the_field_table_agree` 钉住。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::SPELLINGS
            .iter()
            .find(|(spelling, _)| name.eq_ignore_ascii_case(spelling))
            .map(|(_, field)| *field)
    }

    /// 规范名（进读数 / 文档的那一个拼写）。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::InputGainDb => "input_gain_db",
            Self::OutputGainDb => "output_gain_db",
            Self::EqEnabled => "eq_enabled",
            Self::FilterEnabled => "filter_enabled",
            Self::EqLowGain => "eq_low_gain",
            Self::EqLowFreq => "eq_low_freq",
            Self::EqMidGain => "eq_mid_gain",
            Self::EqMidFreq => "eq_mid_freq",
            Self::EqMidQ => "eq_mid_q",
            Self::EqHighGain => "eq_high_gain",
            Self::EqHighFreq => "eq_high_freq",
            Self::CutoffHz => "cutoff_hz",
            Self::Resonance => "resonance",
            Self::Drive => "drive",
            Self::CompressorEnabled => "compressor_enabled",
            Self::ThresholdDb => "threshold_db",
            Self::Ratio => "ratio",
            Self::KneeDb => "knee_db",
            Self::DetectorS => "detector_s",
            Self::AttackS => "attack_s",
            Self::ReleaseS => "release_s",
            Self::MakeupDb => "makeup_db",
            Self::HpfHz => "hpf_hz",
            Self::DynEqFreq => "deq_freq",
            Self::DynEqThresholdDb => "deq_threshold_db",
            Self::DynEqMaxCutDb => "deq_cut_db",
            Self::Width => "strip_width",
            Self::MonoBelowHz => "mono_below_hz",
        }
    }

    /// 本字段是否是**开关**（读作 `1.0` / `0.0`，写时非零即真）。
    #[must_use]
    pub const fn is_flag(self) -> bool {
        matches!(
            self,
            Self::EqEnabled | Self::FilterEnabled | Self::CompressorEnabled
        )
    }

    /// 从**已 sanitised 的**通道条参数里读这个字段（开关读成 `1.0` / `0.0`）。
    ///
    /// 与 [`Self::store`] 是同一张字段表的两个方向：`store` 之后 `read` 必得
    /// （钳制后的）存入值。
    #[must_use]
    pub fn read(self, params: &ChannelStripParams) -> f32 {
        match self {
            Self::InputGainDb => params.input_gain_db,
            Self::OutputGainDb => params.output_gain_db,
            Self::EqEnabled => f32::from(u8::from(params.eq_enabled)),
            Self::FilterEnabled => f32::from(u8::from(params.filter_enabled)),
            Self::EqLowGain => params.eq.low_gain,
            Self::EqLowFreq => params.eq.low_freq,
            Self::EqMidGain => params.eq.mid_gain,
            Self::EqMidFreq => params.eq.mid_freq,
            Self::EqMidQ => params.eq.mid_q,
            Self::EqHighGain => params.eq.high_gain,
            Self::EqHighFreq => params.eq.high_freq,
            Self::CutoffHz => params.filter.cutoff_hz,
            Self::Resonance => params.filter.resonance,
            Self::Drive => params.filter.drive,
            Self::CompressorEnabled => f32::from(u8::from(params.compressor_enabled)),
            Self::ThresholdDb => params.compressor.threshold_db,
            Self::Ratio => params.compressor.ratio,
            Self::KneeDb => params.compressor.knee_db,
            Self::DetectorS => params.compressor.detector_s,
            Self::AttackS => params.compressor.attack_s,
            Self::ReleaseS => params.compressor.release_s,
            Self::MakeupDb => params.compressor.makeup_db,
            Self::HpfHz => params.hpf.freq_hz,
            Self::DynEqFreq => params.dyn_eq.freq_hz,
            Self::DynEqThresholdDb => params.dyn_eq.threshold_db,
            Self::DynEqMaxCutDb => params.dyn_eq.max_cut_db,
            Self::Width => params.width,
            Self::MonoBelowHz => params.mono_below_hz,
        }
    }

    /// 把**原生绝对值**写进参数（开关：非零即真，与 `yeban_render::inserts::flag` 同式）。
    ///
    /// ⚠ 不在这里钳制：钳制是器件自己的纪律（`ChannelStrip::set_params` 内部调
    /// [`ChannelStripParams::sanitised`]）。[`Self::sanitise`] 只是把**同一次**钳制
    /// 提前算给恒等判定看。
    pub fn store(self, params: &mut ChannelStripParams, value: f32) {
        match self {
            Self::InputGainDb => params.input_gain_db = value,
            Self::OutputGainDb => params.output_gain_db = value,
            Self::EqEnabled => params.eq_enabled = value != 0.0,
            Self::FilterEnabled => params.filter_enabled = value != 0.0,
            Self::EqLowGain => params.eq.low_gain = value,
            Self::EqLowFreq => params.eq.low_freq = value,
            Self::EqMidGain => params.eq.mid_gain = value,
            Self::EqMidFreq => params.eq.mid_freq = value,
            Self::EqMidQ => params.eq.mid_q = value,
            Self::EqHighGain => params.eq.high_gain = value,
            Self::EqHighFreq => params.eq.high_freq = value,
            Self::CutoffHz => params.filter.cutoff_hz = value,
            Self::Resonance => params.filter.resonance = value,
            Self::Drive => params.filter.drive = value,
            Self::CompressorEnabled => params.compressor_enabled = value != 0.0,
            Self::ThresholdDb => params.compressor.threshold_db = value,
            Self::Ratio => params.compressor.ratio = value,
            Self::KneeDb => params.compressor.knee_db = value,
            Self::DetectorS => params.compressor.detector_s = value,
            Self::AttackS => params.compressor.attack_s = value,
            Self::ReleaseS => params.compressor.release_s = value,
            Self::MakeupDb => params.compressor.makeup_db = value,
            Self::HpfHz => params.hpf.freq_hz = value,
            Self::DynEqFreq => params.dyn_eq.freq_hz = value,
            Self::DynEqThresholdDb => params.dyn_eq.threshold_db = value,
            Self::DynEqMaxCutDb => params.dyn_eq.max_cut_db = value,
            Self::Width => params.width = value,
            Self::MonoBelowHz => params.mono_below_hz = value,
        }
    }

    /// 与器件内部**同一次**钳制：把值写进一份 `DEFAULT` 参数、`sanitised()`、再读回来。
    ///
    /// ⛔ 不复制上下界：`ChannelStripParams::sanitised` 是唯一一份钳制规则（包括
    /// `cutoff_hz` 的非有限值归**上界**这条不对称裁决）。把它抄到这里必然漂移。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        let mut params = ChannelStripParams::DEFAULT;
        self.store(&mut params, value);
        self.read(&params.sanitised())
    }
}

/// Freeverb 上**可被 `DeviceParam` 自动化落点**的字段。
///
/// 与 `reverb_of` 的静态投影**同一张拼写表**（[`Self::SPELLINGS`]）：静态投影与
/// 块边界自动化读的是同一组名字，⛔ 不写第二份。刻意**不含** `reverb_mix` /
/// `reverb_width`（与引擎 `ReverbProjection::apply` 同口径，那两个名字**不认**）。
///
/// 四个字段都是器件 `Reverb::set_params` 的标量入参；该函数文档承诺**零分配、可逐块**
/// ⇒ 块边界自动化不需要重新分配延迟线（`set_sample_rate` 才是分配入口）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReverbField {
    /// 衰减长度（0..1）。`reverb_size`。
    Size,
    /// 尾巴高频阻尼（0..1）。`reverb_damp`。
    Damp,
    /// 湿/干平衡（0..1）。`reverb_wet` / `reverb_wet_mix`。
    Wet,
    /// 湿路预延迟（秒）。`reverb_predelay` / `reverb_predelay_s`。
    Predelay,
}

impl ReverbField {
    /// 全部字段。
    pub const ALL: &'static [Self] = &[Self::Size, Self::Damp, Self::Wet, Self::Predelay];

    /// **全部拼写** → 字段（`6` 条 = `4` 个字段）。
    pub const SPELLINGS: &'static [(&'static str, Self)] = &[
        ("reverb_size", Self::Size),
        ("reverb_damp", Self::Damp),
        ("reverb_wet", Self::Wet),
        ("reverb_wet_mix", Self::Wet),
        ("reverb_predelay", Self::Predelay),
        ("reverb_predelay_s", Self::Predelay),
    ];

    /// 名字 → 字段（大小写不敏感）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::SPELLINGS
            .iter()
            .find(|(spelling, _)| name.eq_ignore_ascii_case(spelling))
            .map(|(_, field)| *field)
    }

    /// 规范名。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Size => "reverb_size",
            Self::Damp => "reverb_damp",
            Self::Wet => "reverb_wet",
            Self::Predelay => "reverb_predelay_s",
        }
    }

    /// 从投影参数里读这个字段。
    #[must_use]
    pub fn read(self, params: &ReverbParams) -> f32 {
        match self {
            Self::Size => params.size,
            Self::Damp => params.damp,
            Self::Wet => params.mix,
            Self::Predelay => params.predelay,
        }
    }

    /// 把**原生绝对值**写进投影参数（⛔ 不在这里钳制：钳制在器件的 `set_params`）。
    pub fn store(self, params: &mut ReverbParams, value: f32) {
        match self {
            Self::Size => params.size = value,
            Self::Damp => params.damp = value,
            Self::Wet => params.mix = value,
            Self::Predelay => params.predelay = value,
        }
    }

    /// 与 `Reverb::set_params` **同一次**钳制/回落（只给恒等判定用）。
    ///
    /// 非有限值：`size` / `damp` / `mix` 回落到 [`ReverbParams::default`] 的对应值，
    /// `predelay` 回落到 `0.0`（与器件逐字一致）。由真实器件的网格判据钉住。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        let fallback = ReverbParams::default();
        let finite = |value: f32, fallback: f32| {
            if value.is_finite() { value } else { fallback }
        };
        match self {
            Self::Size => finite(value, fallback.size).clamp(0.0, 1.0),
            Self::Damp => finite(value, fallback.damp).clamp(0.0, 1.0),
            Self::Wet => finite(value, fallback.mix).clamp(0.0, 1.0),
            Self::Predelay => finite(value, 0.0).clamp(0.0, REVERB_MAX_PREDELAY_S),
        }
    }
}

/// 卷积混响上**可被 `DeviceParam` 自动化落点**的**标量**字段。
///
/// 刻意**不含** IR 形状（`conv_ir_decay_s` / `conv_ir_decay` / `conv_ir_seed` / `conv_ir_stereo`）：
/// 那四个是**构造期烘焙**，换值要重新合成 IR 并走 `set_impulse_response`
/// （`yeban-dsp` 文档逐字“分配入口之一”）⇒ 不是块边界能改的标量。
///
/// 四个字段都是 `ConvolutionReverb::set_params` 的入参；该函数文档承诺**零分配、
/// 可以逐块调用** ⇒ 块边界自动化不需要重烘 IR。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConvolutionField {
    /// 湿路预延迟（秒，0..0.1）。`conv_predelay*`。
    PreDelay,
    /// 干路电平（0..1）。`conv_dry`。
    Dry,
    /// 湿路电平（0..1）。`conv_wet` / `conv_wet_mix`。
    Wet,
    /// IR 增益（dB，−60..+24）。`conv_ir_gain_db` / `conv_ir_gain`。
    IrGainDb,
}

impl ConvolutionField {
    /// 全部字段。
    pub const ALL: &'static [Self] = &[Self::PreDelay, Self::Dry, Self::Wet, Self::IrGainDb];

    /// **全部拼写** → 字段（`8` 条 = `4` 个字段）。
    pub const SPELLINGS: &'static [(&'static str, Self)] = &[
        ("conv_predelay_s", Self::PreDelay),
        ("conv_pre_delay_s", Self::PreDelay),
        ("conv_predelay", Self::PreDelay),
        ("conv_dry", Self::Dry),
        ("conv_wet", Self::Wet),
        ("conv_wet_mix", Self::Wet),
        ("conv_ir_gain_db", Self::IrGainDb),
        ("conv_ir_gain", Self::IrGainDb),
    ];

    /// 名字 → 字段（大小写不敏感）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::SPELLINGS
            .iter()
            .find(|(spelling, _)| name.eq_ignore_ascii_case(spelling))
            .map(|(_, field)| *field)
    }

    /// 规范名。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::PreDelay => "conv_predelay_s",
            Self::Dry => "conv_dry",
            Self::Wet => "conv_wet",
            Self::IrGainDb => "conv_ir_gain_db",
        }
    }

    /// 从投影参数里读这个字段。
    #[must_use]
    pub fn read(self, params: &ConvolutionReverbParams) -> f32 {
        match self {
            Self::PreDelay => params.pre_delay_s,
            Self::Dry => params.dry,
            Self::Wet => params.wet,
            Self::IrGainDb => params.ir_gain_db,
        }
    }

    /// 把**原生绝对值**写进投影参数（⛔ 钳制在器件的 `set_params`）。
    pub fn store(self, params: &mut ConvolutionReverbParams, value: f32) {
        match self {
            Self::PreDelay => params.pre_delay_s = value,
            Self::Dry => params.dry = value,
            Self::Wet => params.wet = value,
            Self::IrGainDb => params.ir_gain_db = value,
        }
    }

    /// 与 `ConvolutionReverb::set_params` **同一次**钳制/回落（只给恒等判定用）。
    ///
    /// 上下界直接引用 `yeban-dsp` 的 `pub` 常量（`MAX_PRE_DELAY_SECONDS` /
    /// `MIN_IR_GAIN_DB` / `MAX_IR_GAIN_DB`），⛔ 不手抄数字。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        let fallback = ConvolutionReverbParams::default();
        let finite = |value: f32, fallback: f32| {
            if value.is_finite() { value } else { fallback }
        };
        match self {
            Self::PreDelay => finite(value, fallback.pre_delay_s).clamp(0.0, MAX_PRE_DELAY_SECONDS),
            Self::Dry => finite(value, fallback.dry).clamp(0.0, 1.0),
            Self::Wet => finite(value, fallback.wet).clamp(0.0, 1.0),
            Self::IrGainDb => {
                finite(value, fallback.ir_gain_db).clamp(MIN_IR_GAIN_DB, MAX_IR_GAIN_DB)
            }
        }
    }
}

/// 乒乓延迟的**投影期参数**：器件的 5 个标量字段 ＋ 音符同步的 4 个输入字段。
///
/// 为什么不直接用 [`DelayParams`]：同步四字段是**投影期输入**（按工程 `bpm` 换算成
/// `time_s`），不是器件的运行参数；`delay.rs` 本单零改动 ⇒ 器件结构不能扩。
/// 投影层（`yeban-render` / `yeban-engine` 各自的 `delay_of`）先把设备参数读进
/// 这一份，再经 [`delay_params_from_stage`] 换算成器件参数。
///
/// 舞台缺省遵设计 §3.3（与器件缺省**在两处不同**：`mix = 0` ⇒ 级缺省关闭，
/// `ping_pong = true` ⇒ 功能缺省即乒乓；其余与 [`DelayParams::new`] 同值）；
/// 同步字段的缺省按设计 §3.3（关、3、8、附点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelayStageParams {
    /// 延迟时间（秒，写值；同步开时被换算值覆盖）。
    pub time_s: f32,
    /// 反馈量（0..0.95）。
    pub feedback: f32,
    /// 湿电平（0..1）；钳制后为 0 ⇒ 运行态不建器件（见 `-0.0` 注记）。
    pub mix: f32,
    /// 反馈高频阻尼（0..1）。
    pub damp: f32,
    /// 交叉馈送（回声左右交替）。
    pub ping_pong: bool,
    /// 音符同步开关。
    pub sync_enabled: bool,
    /// 同步分子（拍数，全音符为单位的分母体系）。
    pub sync_num: f32,
    /// 同步分母。
    pub sync_den: f32,
    /// 附点（×1.5）。
    pub sync_dotted: bool,
}

impl Default for DelayStageParams {
    fn default() -> Self {
        Self {
            time_s: 0.3,
            feedback: 0.3,
            mix: 0.0,
            damp: 0.0,
            ping_pong: true,
            sync_enabled: false,
            sync_num: 3.0,
            sync_den: 8.0,
            sync_dotted: true,
        }
    }
}

/// 乒乓延迟上**可被 `DeviceParam` 自动化落点**的字段。
///
/// 与静态投影（`delay_projection` / 引擎 `delay_of`）**同一张拼写表**
///（[`Self::SPELLINGS`]）：静态投影与块边界自动化读的是同一组名字，⛔ 不写第二份。
///
/// 同步四字段只在投影期换算：块边界写同步字段只改投影结构里的输入值，
/// 不重算 `time_s`（块边界没有 `bpm`）；同步泳道的"应用"口径见 ADR-0061。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DelayField {
    /// 延迟时间（秒）。`delay_time_s` / `delay_time`。
    TimeS,
    /// 反馈量（0..0.95）。`delay_feedback` / `delay_fb`。
    Feedback,
    /// 湿电平（0..1）。`delay_mix` / `delay_wet`。
    Mix,
    /// 反馈高频阻尼（0..1）。`delay_damp` / `delay_damping`。
    Damp,
    /// 交叉馈送开关。`delay_ping_pong` / `delay_pingpong`。
    PingPong,
    /// 音符同步开关。`delay_sync_enabled` / `delay_sync`。
    SyncEnabled,
    /// 同步分子（1..16）。`delay_sync_num`。
    SyncNum,
    /// 同步分母（1..16）。`delay_sync_den`。
    SyncDen,
    /// 附点开关。`delay_sync_dotted`。
    SyncDotted,
}

impl DelayField {
    /// 全部字段。
    pub const ALL: &'static [Self] = &[
        Self::TimeS,
        Self::Feedback,
        Self::Mix,
        Self::Damp,
        Self::PingPong,
        Self::SyncEnabled,
        Self::SyncNum,
        Self::SyncDen,
        Self::SyncDotted,
    ];

    /// **全部拼写** → 字段（`15` 条 = `9` 个字段）。
    pub const SPELLINGS: &'static [(&'static str, Self)] = &[
        ("delay_time_s", Self::TimeS),
        ("delay_time", Self::TimeS),
        ("delay_feedback", Self::Feedback),
        ("delay_fb", Self::Feedback),
        ("delay_mix", Self::Mix),
        ("delay_wet", Self::Mix),
        ("delay_damp", Self::Damp),
        ("delay_damping", Self::Damp),
        ("delay_ping_pong", Self::PingPong),
        ("delay_pingpong", Self::PingPong),
        ("delay_sync_enabled", Self::SyncEnabled),
        ("delay_sync", Self::SyncEnabled),
        ("delay_sync_num", Self::SyncNum),
        ("delay_sync_den", Self::SyncDen),
        ("delay_sync_dotted", Self::SyncDotted),
    ];

    /// 名字 → 字段（大小写不敏感）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::SPELLINGS
            .iter()
            .find(|(spelling, _)| name.eq_ignore_ascii_case(spelling))
            .map(|(_, field)| *field)
    }

    /// 规范名。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::TimeS => "delay_time_s",
            Self::Feedback => "delay_feedback",
            Self::Mix => "delay_mix",
            Self::Damp => "delay_damp",
            Self::PingPong => "delay_ping_pong",
            Self::SyncEnabled => "delay_sync_enabled",
            Self::SyncNum => "delay_sync_num",
            Self::SyncDen => "delay_sync_den",
            Self::SyncDotted => "delay_sync_dotted",
        }
    }

    /// 本字段是否是**开关**（读作 `1.0` / `0.0`，写时非零即真）。
    #[must_use]
    pub const fn is_flag(self) -> bool {
        matches!(self, Self::PingPong | Self::SyncEnabled | Self::SyncDotted)
    }

    /// 从投影参数里读这个字段（开关读成 `1.0` / `0.0`）。
    #[must_use]
    pub fn read(self, params: &DelayStageParams) -> f32 {
        match self {
            Self::TimeS => params.time_s,
            Self::Feedback => params.feedback,
            Self::Mix => params.mix,
            Self::Damp => params.damp,
            Self::PingPong => f32::from(u8::from(params.ping_pong)),
            Self::SyncEnabled => f32::from(u8::from(params.sync_enabled)),
            Self::SyncNum => params.sync_num,
            Self::SyncDen => params.sync_den,
            Self::SyncDotted => f32::from(u8::from(params.sync_dotted)),
        }
    }

    /// 把**原生绝对值**写进投影参数（⛔ 不在这里钳制：钳制在器件的 `process`
    /// 与 [`Self::sanitise`] 的恒等判定里）。
    pub fn store(self, params: &mut DelayStageParams, value: f32) {
        match self {
            Self::TimeS => params.time_s = value,
            Self::Feedback => params.feedback = value,
            Self::Mix => params.mix = value,
            Self::Damp => params.damp = value,
            Self::PingPong => params.ping_pong = value != 0.0,
            Self::SyncEnabled => params.sync_enabled = value != 0.0,
            Self::SyncNum => params.sync_num = value,
            Self::SyncDen => params.sync_den = value,
            Self::SyncDotted => params.sync_dotted = value != 0.0,
        }
    }

    /// 与器件**同一次**钳制/回落（只给恒等判定与运行态的建级门用）。
    ///
    /// 器件字段沿 `Delay::process` 的入口钳制（`time` 钳到 `0.001..MAX_DELAY_SECONDS`、
    /// `feedback` 钳到 `0..0.95`、`mix` / `damp` 钳到 `0..1`，非有限值回落到下界）；
    /// 同步分子/分母钳到 `1..16`（非有限值回落到缺省 3 / 8）；开关读作 `1.0` / `0.0`。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        match self {
            Self::TimeS => {
                if value.is_finite() {
                    value.clamp(0.001, MAX_DELAY_SECONDS)
                } else {
                    0.001
                }
            }
            Self::Feedback => {
                if value.is_finite() {
                    value.clamp(0.0, 0.95)
                } else {
                    0.0
                }
            }
            Self::Mix | Self::Damp => {
                if value.is_finite() {
                    value.clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            Self::PingPong | Self::SyncEnabled | Self::SyncDotted => {
                f32::from(u8::from(value != 0.0))
            }
            Self::SyncNum => {
                if value.is_finite() {
                    value.clamp(1.0, 16.0)
                } else {
                    3.0
                }
            }
            Self::SyncDen => {
                if value.is_finite() {
                    value.clamp(1.0, 16.0)
                } else {
                    8.0
                }
            }
        }
    }
}

/// 音符同步换算（投影期、控制线程做）：`time_s = 60/bpm × 4 × num/den × (附点 ? 1.5 : 1)`。
///
/// `num` / `den` 先钳到 `1..16` 再取整（拍数是整数）；结果钳到器件域
/// `0.001..MAX_DELAY_SECONDS`（84 BPM 附点二分 2.1429 s ⇒ 钳到 2.0 s）。
#[must_use]
pub fn delay_sync_time_s(bpm: f64, num: f32, den: f32, dotted: bool) -> f32 {
    let num = num.clamp(1.0, 16.0).round();
    let den = den.clamp(1.0, 16.0).round();
    let quarters = 4.0 * (f64::from(num) / f64::from(den)) * if dotted { 1.5 } else { 1.0 };
    ((60.0 / bpm) * quarters) as f32
}

/// 投影期参数 → 器件参数：同步开且 `bpm` 有效时 `time_s` 被换算值覆盖，
/// 否则保留写值（器件 `process` 入口再钳制一次）。
///
/// `bpm` 无效（非有限或 ≤ 0）时同步**不换算**（退回写值，不静默编一个速度）；
/// 该回落口径见 ADR-0061。
#[must_use]
pub fn delay_params_from_stage(stage: &DelayStageParams, bpm: f64) -> DelayParams {
    let mut params = DelayParams {
        time_s: stage.time_s,
        feedback: stage.feedback,
        mix: stage.mix,
        damp: stage.damp,
        ping_pong: stage.ping_pong,
    };
    if stage.sync_enabled && bpm.is_finite() && bpm > 0.0 {
        params.time_s = delay_sync_time_s(bpm, stage.sync_num, stage.sync_den, stage.sync_dotted)
            .clamp(0.001, MAX_DELAY_SECONDS);
    }
    params
}

/// 磁带饱和上**可被 `DeviceParam` 自动化落点**的字段。
///
/// 与 `tape_of` 的静态投影**同一张拼写表**（[`Self::SPELLINGS`]）：静态投影与
/// 块边界自动化读的是同一组名字，⛔ 不写第二份。
///
/// 四个字段都是 `TapeSaturation::set_params` 的入参；该函数文档承诺**零分配、
/// 可以逐块调用** ⇒ 块边界自动化不需要重建过采样器（`configure` 才是状态初始化入口）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TapeField {
    /// 饱和驱动（0..1）。`tape_drive` / `saturation_drive`。
    Drive,
    /// 干湿比（0..1）。`tape_mix` / `saturation_mix`。
    Mix,
    /// 输出补偿增益（dB，−24..+24）。`tape_trim_db` / `saturation_trim_db`。
    TrimDb,
    /// 湿路低通截止频率（Hz，2000..20000）。`tape_hf_hz` / `saturation_hf_hz`。
    HfHz,
    /// ⭐ 曲线不对称量（0..0.5；`0` ＝ 对称 ⇒ 纯奇次）。`tape_asym` / `saturation_asym`。
    ///
    /// 它产生**偶次**谐波（磁带"暖"的来源）—— `pade_tanh` 是奇函数，只出奇次。
    /// 见 `crate::tape` 模块文档的「⭐ 偶次谐波」一节。
    Asym,
}

impl TapeField {
    /// 全部字段。
    pub const ALL: &'static [Self] =
        &[Self::Drive, Self::Mix, Self::TrimDb, Self::HfHz, Self::Asym];

    /// **全部拼写** → 字段（`10` 条 = `5` 个字段）。
    pub const SPELLINGS: &'static [(&'static str, Self)] = &[
        ("tape_drive", Self::Drive),
        ("saturation_drive", Self::Drive),
        ("tape_mix", Self::Mix),
        ("saturation_mix", Self::Mix),
        ("tape_trim_db", Self::TrimDb),
        ("saturation_trim_db", Self::TrimDb),
        ("tape_hf_hz", Self::HfHz),
        ("saturation_hf_hz", Self::HfHz),
        ("tape_asym", Self::Asym),
        ("saturation_asym", Self::Asym),
    ];

    /// 名字 → 字段（大小写不敏感）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::SPELLINGS
            .iter()
            .find(|(spelling, _)| name.eq_ignore_ascii_case(spelling))
            .map(|(_, field)| *field)
    }

    /// 规范名。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Drive => "tape_drive",
            Self::Mix => "tape_mix",
            Self::TrimDb => "tape_trim_db",
            Self::HfHz => "tape_hf_hz",
            Self::Asym => "tape_asym",
        }
    }

    /// 从投影参数里读这个字段。
    #[must_use]
    pub fn read(self, params: &TapeParams) -> f32 {
        match self {
            Self::Drive => params.drive,
            Self::Mix => params.mix,
            Self::TrimDb => params.trim_db,
            Self::HfHz => params.hf_hz,
            Self::Asym => params.asym,
        }
    }

    /// 把**原生绝对值**写进投影参数（⛔ 不在这里钳制：钳制在器件的 `set_params`）。
    pub fn store(self, params: &mut TapeParams, value: f32) {
        match self {
            Self::Drive => params.drive = value,
            Self::Mix => params.mix = value,
            Self::TrimDb => params.trim_db = value,
            Self::HfHz => params.hf_hz = value,
            Self::Asym => params.asym = value,
        }
    }

    /// 与器件内部**同一次**钳制：把值写进一份 `DEFAULT` 参数、`sanitised()`、再读回来。
    ///
    /// ⛔ 不复制上下界：[`TapeParams::sanitised`] 是唯一一份钳制规则。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        let mut params = TapeParams::DEFAULT;
        self.store(&mut params, value);
        self.read(&params.sanitised())
    }
}

/// 一条 `DeviceParam` 泳道在离线插入链上的**落点**：器件族 ＋ 字段。
///
/// [`Self::from_name`] 是“哪个名字有落点”的**唯一**判定处：静态投影
/// （`strip_of` / `tape_of` / `reverb_of` / `convolution_of`）与块边界自动化都从这张表出发。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceParamField {
    /// 通道条字段。
    Strip(StripField),
    /// 磁带饱和标量字段。
    Tape(TapeField),
    /// Freeverb 标量字段。
    Reverb(ReverbField),
    /// 卷积混响**标量**字段。
    Convolution(ConvolutionField),
    /// 乒乓延迟字段。
    Delay(DelayField),
}

/// 强制实例化按**器件族**分开。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceParamFamily {
    /// 通道条。
    Strip,
    /// 磁带饱和。
    Saturation,
    /// Freeverb。
    Reverb,
    /// 卷积混响。
    Convolution,
    /// 乒乓延迟。
    Delay,
}

impl DeviceParamField {
    /// 名字 → 落点（大小写不敏感）。顺序：通道条 → 饱和 → 混响 → 卷积 → 延迟。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        StripField::from_name(name)
            .map(Self::Strip)
            .or_else(|| TapeField::from_name(name).map(Self::Tape))
            .or_else(|| ReverbField::from_name(name).map(Self::Reverb))
            .or_else(|| ConvolutionField::from_name(name).map(Self::Convolution))
            .or_else(|| DelayField::from_name(name).map(Self::Delay))
    }

    /// 规范名。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Strip(field) => field.canonical_name(),
            Self::Tape(field) => field.canonical_name(),
            Self::Reverb(field) => field.canonical_name(),
            Self::Convolution(field) => field.canonical_name(),
            Self::Delay(field) => field.canonical_name(),
        }
    }

    /// 器件族（强制实例化用）。
    #[must_use]
    pub const fn family(self) -> DeviceParamFamily {
        match self {
            Self::Strip(_) => DeviceParamFamily::Strip,
            Self::Tape(_) => DeviceParamFamily::Saturation,
            Self::Reverb(_) => DeviceParamFamily::Reverb,
            Self::Convolution(_) => DeviceParamFamily::Convolution,
            Self::Delay(_) => DeviceParamFamily::Delay,
        }
    }

    /// 与器件**同一次**钳制/回落（只给恒等判定用）。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        match self {
            Self::Strip(field) => field.sanitise(value),
            Self::Tape(field) => field.sanitise(value),
            Self::Reverb(field) => field.sanitise(value),
            Self::Convolution(field) => field.sanitise(value),
            Self::Delay(field) => field.sanitise(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 84 BPM 音符同步换算表（设计 §3.3 的数值逐项断言；附点二分换算值为 2.142857，
    /// 钳制到 2.0 s 发生在 `delay_params_from_stage`，不在纯换算里）。
    #[test]
    fn delay_sync_conversion_table_at_84_bpm() {
        let bpm = 84.0;
        let cases = [
            (1.0, 8.0, false, 0.357_142_87),
            (1.0, 8.0, true, 0.535_714_27),
            (1.0, 4.0, false, 0.714_285_73),
            (1.0, 4.0, true, 1.071_428_5),
            (1.0, 2.0, false, 1.428_571_5),
            (1.0, 2.0, true, 2.142_857),
        ];
        for (num, den, dotted, expected) in cases {
            let got = delay_sync_time_s(bpm, num, den, dotted);
            assert!(
                (got - expected).abs() < 1e-4,
                "84 BPM {num}/{den} dotted={dotted}：期望 {expected}，实得 {got}"
            );
        }
        // 钳制在换算的下一层：附点二分经 `delay_params_from_stage` 落到 2.0 s。
        let stage = DelayStageParams {
            sync_enabled: true,
            sync_num: 1.0,
            sync_den: 2.0,
            sync_dotted: true,
            ..DelayStageParams::default()
        };
        assert_eq!(delay_params_from_stage(&stage, bpm).time_s, 2.0);
    }

    /// 同步关 ⇒ 保留写值；bpm 无效 ⇒ 同步不换算（退回写值，不编速度）。
    #[test]
    fn delay_sync_off_or_bad_bpm_keeps_written_time() {
        let mut stage = DelayStageParams {
            time_s: 0.5,
            ..Default::default()
        };
        assert_eq!(
            delay_params_from_stage(&stage, 120.0).time_s,
            0.5,
            "同步关时写值原样进器件"
        );
        stage.sync_enabled = true;
        stage.sync_num = 1.0;
        stage.sync_den = 2.0;
        stage.sync_dotted = true;
        for bad_bpm in [f64::NAN, f64::INFINITY, 0.0, -120.0] {
            assert_eq!(
                delay_params_from_stage(&stage, bad_bpm).time_s,
                0.5,
                "bpm={bad_bpm} 时同步不换算"
            );
        }
        assert!(
            (delay_params_from_stage(&stage, 84.0).time_s - 2.0).abs() < f32::EPSILON,
            "84 BPM 附点二分换算后钳到 2.0 s"
        );
    }

    /// 延迟拼写表：规范名与别名指向同一字段。
    #[test]
    fn delay_spellings_resolve_to_the_same_field() {
        for field in DelayField::ALL {
            assert_eq!(
                DelayField::from_name(field.canonical_name()),
                Some(*field),
                "规范名 {} 必须解析",
                field.canonical_name()
            );
        }
        for (alias, field) in [
            ("delay_time", DelayField::TimeS),
            ("delay_fb", DelayField::Feedback),
            ("delay_wet", DelayField::Mix),
            ("delay_damping", DelayField::Damp),
            ("delay_pingpong", DelayField::PingPong),
            ("delay_sync", DelayField::SyncEnabled),
        ] {
            assert_eq!(
                DelayField::from_name(alias),
                Some(field),
                "别名 {alias} 必须指向同一字段"
            );
        }
        assert_eq!(DelayField::SPELLINGS.len(), 15, "延迟拼写共 15 条");
    }
}
