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
//! 三个**器件族**各一张字段表 ＋ 一张把三个族合起来的落点枚举：
//!
//! | 类型 | 内容 |
//! | :--- | :--- |
//! | [`StripField`] | 通道条 22 个可自动化字段（`41` 个拼写） |
//! | [`ReverbField`] | Freeverb 的 4 个标量字段（`6` 个拼写） |
//! | [`ConvolutionField`] | 卷积混响的 4 个标量字段（`8` 个拼写；IR 形状不落点） |
//! | [`DeviceParamField`] / [`DeviceParamFamily`] | 三族的合流枚举与器件族标签 |
//!
//! ⛔ **本模块不含**任何 I/O、模型类型或 GUI：每个 `sanitise` 都与器件内部**同一次**
//! 钳制对齐（[`ChannelStripParams::sanitised`] / [`crate::reverb::Reverb::set_params`] /
//! [`crate::convolution_reverb::ConvolutionReverb::set_params`]），`read`/`store` 只读写这三个参数结构。
//! 名字表的消费方（静态投影 `strip_of` / `reverb_of` / `convolution_of`、离线施加、
//! 实时块边界落点）都在 `yeban-render` 与 `yeban-engine`，它们⛔ 不得再造第二张表。

use crate::channel_strip::ChannelStripParams;
use crate::convolution_reverb::{
    ConvolutionReverbParams, MAX_IR_GAIN_DB, MAX_PRE_DELAY_SECONDS, MIN_IR_GAIN_DB,
};
use crate::reverb::{REVERB_MAX_PREDELAY_S, ReverbParams};

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
///   （`conv_ir_decay*` / `conv_ir_seed`）是构造期烘焙、不可自动化；其余标量由对应
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
    ];

    /// **全部拼写** → 字段（`41` 条；这是"哪个名字能落点"的**唯一**名字表）。
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
/// 刻意**不含** IR 形状（`conv_ir_decay_s` / `conv_ir_decay` / `conv_ir_seed`）：
/// 那三个是**构造期烘焙**，换值要重新合成 IR 并走 `set_impulse_response`
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

/// 一条 `DeviceParam` 泳道在离线插入链上的**落点**：器件族 ＋ 字段。
///
/// [`Self::from_name`] 是“哪个名字有落点”的**唯一**判定处：静态投影
/// （`strip_of` / `reverb_of` / `convolution_of`）与块边界自动化都从这张表出发。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceParamField {
    /// 通道条字段。
    Strip(StripField),
    /// Freeverb 标量字段。
    Reverb(ReverbField),
    /// 卷积混响**标量**字段。
    Convolution(ConvolutionField),
}

/// 强制实例化按**器件族**分开。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceParamFamily {
    /// 通道条。
    Strip,
    /// Freeverb。
    Reverb,
    /// 卷积混响。
    Convolution,
}

impl DeviceParamField {
    /// 名字 → 落点（大小写不敏感）。顺序：通道条 → 混响 → 卷积。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        StripField::from_name(name)
            .map(Self::Strip)
            .or_else(|| ReverbField::from_name(name).map(Self::Reverb))
            .or_else(|| ConvolutionField::from_name(name).map(Self::Convolution))
    }

    /// 规范名。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Strip(field) => field.canonical_name(),
            Self::Reverb(field) => field.canonical_name(),
            Self::Convolution(field) => field.canonical_name(),
        }
    }

    /// 器件族（强制实例化用）。
    #[must_use]
    pub const fn family(self) -> DeviceParamFamily {
        match self {
            Self::Strip(_) => DeviceParamFamily::Strip,
            Self::Reverb(_) => DeviceParamFamily::Reverb,
            Self::Convolution(_) => DeviceParamFamily::Convolution,
        }
    }

    /// 与器件**同一次**钳制/回落（只给恒等判定用）。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        match self {
            Self::Strip(field) => field.sanitise(value),
            Self::Reverb(field) => field.sanitise(value),
            Self::Convolution(field) => field.sanitise(value),
        }
    }
}
