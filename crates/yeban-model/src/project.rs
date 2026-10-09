//! 顶层工程文档 `YebanProjectV1` 与持久化 AST [MODEL-AST-002/003/004, MODEL-ISO-001]。
//!
//! 本模块是夜半工程文件 `.yeban` 内 `project.json` 的**唯一权威定义**：
//!
//! - [`YebanProjectV1`] — 持久化文档层（`MODEL-ISO-001` 的第一层）。
//!   挥发性运行态（播放头、isPlaying、插件 PID）与本机配置（声卡端口绑定、
//!   外部编辑器路径、云端 Token）**严禁**出现在这里。
//! - [`RoutingGraph`] — 唯一声学路由真理源 [MODEL-AST-004]。
//! - [`YebanProjectV1::check_readable`] — `schema_version` / `min_reader_version`
//!   双重版本门 [ROAD-M1-001, RSK-16]。
//!
//! ## 与 `schemas/project.schema.json` 的契约
//!
//! 机器契约文件 `schemas/project.schema.json` 是跨实现（Rust serde ↔ Python jsonschema）
//! 的对账口径。本模块按它落地以下不可协商的形状：
//!
//! 1. 顶层必填键 `schema_version` / `min_reader_version` / `writer_version` / `id` /
//!    `title` / `bpm` / `time_signature` / `audio_config` / `tracks` / `clip_pool` /
//!    `routing_graph` 全部存在；
//! 2. **顶层没有冗余 `sample_rate`** —— 采样率的唯一来源是 `audio_config.sample_rate`
//!    [MODEL-AST-002]；同理 `pan_law` 只存在于 `audio_config` 内，不再另设顶层副本；
//! 3. `id` 是 26 字符 Crockford Base32 ULID（由 [`EntityId`] 的类型不变量保证）；
//! 4. 数值范围：`bpm` ∈ 20.0..=999.0、`pan` ∈ -1.0..=1.0、
//!    `time_signature.numerator` ∈ 1..=32、`time_signature.denominator` ∈ {1,2,4,8,16,32}、
//!    `sample_rate` / `block_size` / `bit_depth` / `pan_law` 取规范枚举；
//! 5. `tracks` / `clip_pool` 是 JSON **对象**（即 `BTreeMap`，键序确定）
//!    [MODEL-AST-003]；`routing_graph.edges` 是 JSON **数组**（契约要求），
//!    内存中仍是 `BTreeMap`，序列化时按键升序展开为数组（见 [`RoutingGraph`]）。
//!
//! ### 契约冲突的裁决（已闭合）
//!
//! 本模块第一版实测出唯一的契约分歧：`schemas/project.schema.json` 曾把
//! `writer_version` 声明为 `integer`，而架构 §2.2 与 ADR-0001 D3 裁决它是
//! **应用语义版本字符串**。裁决结果是契约改为 `type: string` + semver `pattern`
//! （ADR-0001 **D11**），因此现在实现与契约**零分歧**。
//!
//! 这条"零分歧"不是靠人盯：`implementation_matches_the_schema_type_table_exactly`
//! **直接读真实的 `schemas/project.schema.json`**（`schema_top_level_types()`），
//! 逐键比较 serde 实际输出的 JSON 类型。手抄一份"契约类型表"曾经让第二份事实源
//! 在契约修好后变成谎言 —— 现在只剩一个事实源。
//! 冲突的实测留痕见 `docs/ledger/model-core-provenance.md`。
//!
//! ## 反序列化的宽容度（ADR-0001 D43 第 2 条）
//!
//! 本模块**不再**为了"旧文件还能读"而宽容。`#[serde(default)]` 只允许出现在两类位置：
//!
//! 1. `Option<T>` —— `None` 是模型自己定义的一等状态（"派生自目标"/"未标注"/"单位增益"…），
//!    这是**语义**而非兼容；
//! 2. 自由文本注记（`description` / `author`）与注记列表（`tags`）—— 空串/空集就是
//!    "没有注记"的确定编码，不参与工程语义。
//!
//! 其余字段一律**必需**：缺键 ⇒ `serde` 的 `missing field` 错误（响亮失败）。
//! 配套约束：必需字段**不得**同时 `skip_serializing_if` 到默认值上，否则本写入器产出的
//! 文档（省略了默认值）会被本读取器拒绝 —— **宽容读与省略写必须成对取消**。
//! 逐项复审表见 `docs/ledger/model-no-compat-notes.md`。

use std::collections::BTreeMap;

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::ModelError;
use crate::ids::{AssetHash, EntityId};
use crate::music::{CurveType, MidiNote};

/// 本写入器产出的文档 schema 版本 [ADR-0001 D3]。
///
/// 规范 §2.2 的代码注释写"固定为 3"，那是 Groove v3 编号体系被重基为
/// `v0.0.1` 起步 / `v1.0.0` 首发之前的遗留数字；ADR-0001 D3 裁决首个稳定
/// 文档 schema 版本为 `1`，且历史上从未发布过 `schema_version = 2|3` 的文档。
pub const SCHEMA_VERSION: u32 = 1;

/// 本文档格式要求的最低读取器版本。
pub const MIN_READER_VERSION: u32 = 1;

/// 本读取器自身实现的 schema 版本（版本门的比较基准）。
pub const READER_SCHEMA_VERSION: u32 = 1;

/// 写入程序版本标记：直接取 crate 的语义版本（当前 `0.0.1`，见 ADR-0001 D3）。
pub const DEFAULT_WRITER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 速度下界（含）。
pub const MIN_BPM: f64 = 20.0;

/// 速度上界（含）。
pub const MAX_BPM: f64 = 999.0;

/// 默认速度。
pub const DEFAULT_BPM: f64 = 120.0;

/// 默认确定性随机种子（ASCII `YEBAN` 前缀，便于在十六进制转储里一眼认出）。
pub const DEFAULT_RNG_SEED: u64 = 0x5945_4241_4E00_0001;

/// 拍号 [MODEL-AST-002]。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSignature {
    /// 分子 1..=32。
    pub numerator: u8,
    /// 分母 ∈ {1,2,4,8,16,32}。
    pub denominator: u8,
}

impl TimeSignature {
    /// 校验拍号取值范围。
    ///
    /// # Errors
    ///
    /// 分子越界 → [`ModelError::TimeSignatureNumeratorOutOfRange`]；
    /// 分母不在允许集合 → [`ModelError::TimeSignatureDenominatorUnsupported`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if !(1..=32).contains(&self.numerator) {
            return Err(ModelError::TimeSignatureNumeratorOutOfRange {
                value: self.numerator,
            });
        }
        if !matches!(self.denominator, 1 | 2 | 4 | 8 | 16 | 32) {
            return Err(ModelError::TimeSignatureDenominatorUnsupported {
                value: self.denominator,
            });
        }
        Ok(())
    }
}

impl Default for TimeSignature {
    /// 4/4 拍。
    fn default() -> Self {
        Self {
            numerator: 4,
            denominator: 4,
        }
    }
}

/// 位深 [MODEL-AST-002]（规范枚举，序列化为精确字符串）。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BitDepth {
    /// 16-bit 整数。
    Int16,
    /// 24-bit 整数。
    Int24,
    /// 32-bit 浮点。
    #[default]
    Float32,
}

/// 声相衰减律 [MODEL-AST-002]（规范枚举，序列化为精确字符串）。
///
/// 唯一来源是 `audio_config.pan_law`，顶层不再另设副本。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanLaw {
    /// 线性。
    Linear,
    /// 等功率 -3 dB。
    #[default]
    ConstantPowerMinus3dB,
    /// 等功率 -4.5 dB。
    ConstantPowerMinus4_5dB,
    /// 等功率 -6 dB。
    ConstantPowerMinus6dB,
}

/// 采样率 [MODEL-AST-002]（规范枚举 `{44100,48000,88200,96000,192000}`）。
///
/// 手写 `Serialize`/`Deserialize`：JSON 契约要求该字段是**整数**，
/// 而派生的枚举序列化会写成字符串，两者不兼容。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SampleRate {
    /// 44.1 kHz。
    Hz44100,
    /// 48 kHz。
    Hz48000,
    /// 88.2 kHz。
    Hz88200,
    /// 96 kHz。
    Hz96000,
    /// 192 kHz。
    Hz192000,
}

impl SampleRate {
    /// 全部合法取值（升序）。
    pub const ALL: [Self; 5] = [
        Self::Hz44100,
        Self::Hz48000,
        Self::Hz88200,
        Self::Hz96000,
        Self::Hz192000,
    ];

    /// 转换为 Hz。
    #[must_use]
    pub const fn hz(self) -> u32 {
        match self {
            Self::Hz44100 => 44_100,
            Self::Hz48000 => 48_000,
            Self::Hz88200 => 88_200,
            Self::Hz96000 => 96_000,
            Self::Hz192000 => 192_000,
        }
    }

    /// 从 Hz 解析，拒绝规范枚举之外的取值。
    ///
    /// # Errors
    ///
    /// 取值不在 `ALL` 中 → [`ModelError::SampleRateUnsupported`]。
    pub fn from_hz(value: u32) -> Result<Self, ModelError> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.hz() == value)
            .ok_or(ModelError::SampleRateUnsupported { value })
    }
}

impl Default for SampleRate {
    /// 48 kHz（内部处理基准）。
    fn default() -> Self {
        Self::Hz48000
    }
}

impl Serialize for SampleRate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.hz())
    }
}

impl<'de> Deserialize<'de> for SampleRate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = u32::deserialize(deserializer)?;
        Self::from_hz(raw).map_err(serde::de::Error::custom)
    }
}

/// 缓冲区长度 (frames) [MODEL-AST-002]（规范枚举 `{64,128,256,512,1024}`）。
///
/// 与 [`SampleRate`] 同理手写 serde：契约要求整数。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlockSize {
    /// 64 frames。
    Frames64,
    /// 128 frames。
    Frames128,
    /// 256 frames。
    Frames256,
    /// 512 frames。
    Frames512,
    /// 1024 frames。
    Frames1024,
}

impl BlockSize {
    /// 全部合法取值（升序）。
    pub const ALL: [Self; 5] = [
        Self::Frames64,
        Self::Frames128,
        Self::Frames256,
        Self::Frames512,
        Self::Frames1024,
    ];

    /// 转换为 frames。
    #[must_use]
    pub const fn frames(self) -> u32 {
        match self {
            Self::Frames64 => 64,
            Self::Frames128 => 128,
            Self::Frames256 => 256,
            Self::Frames512 => 512,
            Self::Frames1024 => 1024,
        }
    }

    /// 从 frames 解析，拒绝规范枚举之外的取值。
    ///
    /// # Errors
    ///
    /// 取值不在 `ALL` 中 → [`ModelError::BlockSizeUnsupported`]。
    pub fn from_frames(value: u32) -> Result<Self, ModelError> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.frames() == value)
            .ok_or(ModelError::BlockSizeUnsupported { value })
    }
}

impl Default for BlockSize {
    /// 256 frames（低时延与稳定性的折中）。
    fn default() -> Self {
        Self::Frames256
    }
}

impl Serialize for BlockSize {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.frames())
    }
}

impl<'de> Deserialize<'de> for BlockSize {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = u32::deserialize(deserializer)?;
        Self::from_frames(raw).map_err(serde::de::Error::custom)
    }
}

/// 音频硬件配置 [MODEL-AST-002]。
///
/// **这是采样率的唯一来源**：顶层文档严禁再出现 `sample_rate` 字段。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectAudioConfig {
    /// 采样率。
    pub sample_rate: SampleRate,
    /// 处理缓冲区长度。
    pub block_size: BlockSize,
    /// 位深。
    pub bit_depth: BitDepth,
    /// 声相衰减律。
    pub pan_law: PanLaw,
}

/// 工程元数据 [MODEL-AST-002]。
///
/// 有意**不含** `title`/`bpm`/`time_signature`：它们在顶层是唯一权威来源，
/// 元数据层重复一份就会产生"两个事实源"的漂移风险。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectMetadata {
    /// 作品描述。
    ///
    /// `#[serde(default)]` 是**语义**而非兼容 [ADR-0001 D43]: 描述是自由文本注记,
    /// 空串就是它在定义上的"没有描述"状态, 既不伪造事实也不改变工程行为。
    #[serde(default)]
    pub description: String,
    /// 创建时间 (Unix 毫秒)。
    ///
    /// **必需** [ADR-0001 D43]: `0` 与"1970-01-01 创建"这一**真实**时间戳不可区分,
    /// 默认值销毁的是"未知"这一信息本身。
    pub created_at_unix_ms: u64,
    /// 最后修改时间 (Unix 毫秒)。
    ///
    /// **必需** [ADR-0001 D43]: 同 `created_at_unix_ms`, `0` 会伪装成真实时间戳。
    pub modified_at_unix_ms: u64,
    /// 自由标签。
    ///
    /// `#[serde(default)]` 是**语义** [ADR-0001 D43]: 空集就是"没有标签", 是注记字段的
    /// 确定编码, 不参与任何工程行为。
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 场景启动量化 [MODEL-AST-002]。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaunchQuantization {
    /// 不量化（立即）。
    Off,
    /// 一小节。
    #[default]
    Bar,
    /// 两小节。
    TwoBars,
    /// 四小节。
    FourBars,
    /// 八小节。
    EightBars,
}

/// 传输控制配置 [MODEL-AST-002]。
///
/// 有意**不含** `bpm`/`time_signature`（顶层唯一权威）与播放头位置
/// （属于 `MODEL-ISO-001` 的挥发性运行态，严禁持久化）。
///
/// 三个字段全部**必需** [ADR-0001 D43]：它们都是"会改变走带/录音行为"的配置 ——
/// 缺 `metronome_enabled` 会静默关掉节拍器、缺 `count_in_bars` 会静默取消预备拍、
/// 缺 `launch_quantization` 会静默换成一小节量化。这里的默认值不是"空值"，
/// 而是**伪造一个作者从未做过的选择**。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransportConfig {
    /// 节拍器是否默认开启。
    pub metronome_enabled: bool,
    /// 录音前的预备小节数。
    pub count_in_bars: u8,
    /// 场景启动的默认量化。
    pub launch_quantization: LaunchQuantization,
}

/// 音轨类型 [MODEL-AST-002]（规范枚举）。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackKind {
    /// MIDI / 乐器轨。
    #[default]
    Midi,
    /// 音频轨。
    Audio,
    /// 辅助返回轨 (Aux Return)。
    AuxReturn,
    /// 主总线。
    Master,
}

/// 设备类型 [MODEL-AST-002]。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceKind {
    /// 内置乐器。
    #[default]
    InternalInstrument,
    /// 内置效果器。
    InternalEffect,
    /// 外部乐器（插件宿主）。
    ExternalInstrument,
    /// 外部效果器（插件宿主）。
    ExternalEffect,
}

/// 单个参数值。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ParameterValue {
    /// 参数名。
    pub name: String,
    /// 当前值（必须有限）。
    pub value: f32,
    /// 物理单位（如 `dB` / `Hz` / `%`）。
    ///
    /// `None` 是**一等状态**（"无量纲/未标注"），故保留 `#[serde(default)]`
    /// [ADR-0001 D43 第 2 条对 `Option<T>` 的明确豁免]，并与 `skip_serializing_if` 对偶。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl ParameterValue {
    /// 校验数值有限性（`NaN`/`±Inf` 无法确定性序列化，一律拒绝）。
    ///
    /// # Errors
    ///
    /// `value` 非有限 → [`ModelError::NonFiniteValue`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if !self.value.is_finite() {
            return Err(ModelError::NonFiniteValue {
                field: "device.param.value",
                value: f64::from(self.value),
            });
        }
        Ok(())
    }
}

/// 设备定义（设备链中的一格）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeviceDefinition {
    /// 设备实例身份。
    pub id: EntityId,
    /// 显示名。
    pub name: String,
    /// 设备类型。
    pub kind: DeviceKind,
    /// 是否旁通。
    ///
    /// **必需** [ADR-0001 D43]：缺了它，一个被旁通的设备会被静默接回信号链 ——
    /// 这是"听得出、但读文件时看不见"的行为反转。
    pub bypassed: bool,
    /// 参数列表。
    ///
    /// **必需** [ADR-0001 D43]：这是设备参数的**唯一**载体，缺了它等于把所有参数值
    /// 静默丢成空集（`Vec` 的默认值在这里不是"没有参数"，而是"参数丢了"）。
    pub params: Vec<ParameterValue>,
    /// 本设备引入的处理延迟（采样点）[ARCH-PDC-001]。
    ///
    /// 规范原文要求"每个插件与内置设备必须精确上报其引入的处理延迟"。PDC 的正确性完全依赖
    /// 这个数字：非实时线程对 `RoutingGraph` 做关键路径分析得到 `L_max`，再给每个分支插入
    /// `D_i = L_max - L_i` 的延迟线，使所有分支在汇合点相位对齐 —— 少报或漏报都会造成
    /// 相位错位，而这种错位用耳朵听不出来、只能靠机械对账发现。
    ///
    /// **必需** [ADR-0001 D43]：本字段曾经 `#[serde(default)]`，理由写的是"这样旧文档仍可读"。
    /// 1.0.0 之前没有任何旧文档，于是那条理由只剩下坏处：`0` 与"这台设备零延迟"这一
    /// **合法**上报值不可区分，缺字段会被静默读成"零延迟设备"，而 PDC 的相位对齐恰恰
    /// 依赖这个数字。缺字段 ⇒ 响亮失败。
    pub latency_samples: u32,
}

impl Default for DeviceDefinition {
    fn default() -> Self {
        Self {
            id: EntityId::default(),
            name: "Device".to_owned(),
            kind: DeviceKind::default(),
            bypassed: false,
            params: Vec::new(),
            latency_samples: 0,
        }
    }
}

impl DeviceDefinition {
    /// 校验全部参数有限性。
    ///
    /// # Errors
    ///
    /// 任意参数非有限 → [`ModelError::NonFiniteValue`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        for param in &self.params {
            param.validate()?;
        }
        Ok(())
    }
}

/// 自动化目标的强类型寻址 [MODEL-AST-002]。
///
/// 覆盖音轨音量/声相、**发送增益**（`SendGain`，通过路由边寻址）、
/// 设备参数与宏。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AutomationTarget {
    /// 音轨音量。
    TrackVolume {
        /// 目标音轨。
        track_id: EntityId,
    },
    /// 音轨声相。
    TrackPan {
        /// 目标音轨。
        track_id: EntityId,
    },
    /// 发送增益（路由边 = 发送通路的唯一标识）。
    SendGain {
        /// 所属音轨。
        track_id: EntityId,
        /// 发送所对应的路由边。
        edge_id: EntityId,
    },
    /// 设备参数。
    DeviceParam {
        /// 目标音轨。
        track_id: EntityId,
        /// 设备链下标。
        slot_index: usize,
        /// 参数下标。
        param_index: usize,
    },
    /// 宏控制。
    Macro {
        /// 目标音轨。
        track_id: EntityId,
        /// 宏下标。
        macro_index: usize,
    },
}

/// 宏到参数的映射。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MacroMapping {
    /// 被映射的自动化目标。
    pub target: AutomationTarget,
    /// 映射深度 0.0..=1.0。
    pub depth: f32,
}

impl MacroMapping {
    /// 校验映射深度。
    ///
    /// # Errors
    ///
    /// 深度非有限或越出 0.0..=1.0 → [`ModelError::NonFiniteValue`] / [`ModelError::ProbabilityOutOfRange`]
    /// 以外的语义错误，统一报 [`ModelError::NonFiniteValue`] 或
    /// [`ModelError::MacroDepthOutOfRange`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if !self.depth.is_finite() {
            return Err(ModelError::NonFiniteValue {
                field: "macro.mapping.depth",
                value: f64::from(self.depth),
            });
        }
        if !(0.0..=1.0).contains(&self.depth) {
            return Err(ModelError::MacroDepthOutOfRange { value: self.depth });
        }
        Ok(())
    }
}

/// 宏参数。
///
/// 三个字段全部**必需** [ADR-0001 D43]：`name` 是实体的标签（与 `TrackV3::name` /
/// `DeviceDefinition::name` 这两个无默认值的同族字段一致）、`value` 是宏当前取值、
/// `mappings` 是宏的全部去向 —— 缺任何一个都会静默改变宏的行为。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MacroParameter {
    /// 宏名。
    pub name: String,
    /// 宏当前位置 0.0..=1.0。
    pub value: f32,
    /// 映射列表。
    pub mappings: Vec<MacroMapping>,
}

impl MacroParameter {
    /// 校验宏位置与全部映射。
    ///
    /// # Errors
    ///
    /// 位置非有限或不在 0.0..=1.0，或任一映射非法。
    pub fn validate(&self) -> Result<(), ModelError> {
        if !self.value.is_finite() || !(0.0..=1.0).contains(&self.value) {
            return Err(ModelError::MacroValueOutOfRange { value: self.value });
        }
        for mapping in &self.mappings {
            mapping.validate()?;
        }
        Ok(())
    }
}

/// 自动化点 [MODEL-AST-002]。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AutomationPoint {
    /// 自动化点身份。
    pub id: EntityId,
    /// 时间位置 (tick)。
    pub tick: u64,
    /// 参数值（必须有限）。
    pub value: f32,
    /// 到下一个点的曲线形状。
    ///
    /// **必需** [ADR-0001 D43]：默认 `Linear` 会**伪造一条作者没画过的曲线**，
    /// 而曲线形状直接改变求值结果（渲染/引擎都按它插值）。
    pub curve: CurveType,
}

impl AutomationPoint {
    /// 校验数值有限性。
    ///
    /// # Errors
    ///
    /// `value` 非有限 → [`ModelError::NonFiniteValue`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if !self.value.is_finite() {
            return Err(ModelError::NonFiniteValue {
                field: "automation.point.value",
                value: f64::from(self.value),
            });
        }
        Ok(())
    }
}

/// 自动化取值的**单位**（由 [`AutomationTarget`] 派生，不单独存储）。
///
/// 为什么不把单位存进泳道：单位是**目标语义**的一部分（音量/发送增益必然是 dB、
/// 声相必然是双极、宏必然是 0..=1），把它再抄一份到泳道上只会造出两份会漂移的
/// 事实源（同 ADR-0001 D28 的理由）。规范没有给这个枚举命名，裁决见
/// `docs/ledger/model-automation-notes.md`。
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum AutomationUnit {
    /// 分贝 (dB)。
    Decibels,
    /// 归一化 0.0..=1.0。
    Normalized,
    /// 双极 -1.0..=1.0。
    Bipolar,
    /// 目标自有单位（设备参数：模型不知道它的量纲，取域也不可知）。
    #[default]
    Native,
}

impl AutomationUnit {
    /// 界面轴标签用的符号；未知/无量纲返回空串。
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Decibels => "dB",
            Self::Normalized | Self::Bipolar | Self::Native => "",
        }
    }
}

/// 自动化**取值域**：闭区间 `[min, max]`（单位见泳道目标派生的 [`AutomationUnit`]）。
///
/// ## 为什么字段私有、`Deserialize` 手写
///
/// `min > max` 的区间是一个**无法求值的状态**：按界面轴去缩放会把整条曲线画反，
/// 按引擎去钳位会把所有值钳到一端。模型层没有（也不希望新增）一个"区间反了"的
/// `ModelError` 变体 —— 新增变体会让下游 `yeban-mcp` 的
/// `code_for_model` 穷举 match 编译失败。于是这里把该状态**做成不可表示**：
/// 字段私有 + 构造与反序列化都把两端点按定义排序，`min <= max` 与有限性因此是
/// 类型不变量，而不是一条需要报错的检查。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutomationValueDomain {
    min: f32,
    max: f32,
}

impl AutomationValueDomain {
    /// 由两个端点构造：端点按定义**排序**为闭区间 `[min, max]`（这不是"修正输入"，
    /// 而是本区间的定义 —— 区间与端点的书写顺序无关）。
    ///
    /// # Errors
    ///
    /// 任一端点非有限 → [`ModelError::NonFiniteValue`]（非有限值无法确定性序列化）。
    pub fn new(first: f32, second: f32) -> Result<Self, ModelError> {
        if !first.is_finite() {
            return Err(ModelError::NonFiniteValue {
                field: "automation.lane.domain.min",
                value: f64::from(first),
            });
        }
        if !second.is_finite() {
            return Err(ModelError::NonFiniteValue {
                field: "automation.lane.domain.max",
                value: f64::from(second),
            });
        }
        Ok(Self {
            min: first.min(second),
            max: first.max(second),
        })
    }

    /// 取值域下界（恒 `<= max`）。
    #[must_use]
    pub const fn min(self) -> f32 {
        self.min
    }

    /// 取值域上界（恒 `>= min`）。
    #[must_use]
    pub const fn max(self) -> f32 {
        self.max
    }

    /// 区间宽度（恒 `>= 0`）。
    #[must_use]
    pub fn span(self) -> f32 {
        self.max - self.min
    }

    /// 常量端点的内部构造：调用者保证 `min <= max` 且两者有限。
    pub(crate) fn pinned(min: f32, max: f32) -> Self {
        debug_assert!(min.is_finite() && max.is_finite() && min <= max);
        Self { min, max }
    }
}

impl serde::Serialize for AutomationValueDomain {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AutomationValueDomain", 2)?;
        state.serialize_field("min", &self.min)?;
        state.serialize_field("max", &self.max)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for AutomationValueDomain {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            min: f32,
            max: f32,
        }
        let repr = Repr::deserialize(deserializer)?;
        Self::new(repr.min, repr.max).map_err(serde::de::Error::custom)
    }
}

/// 自动化**写模式**（录制时如何把参数动作写进泳道）。
///
/// 规范四份正文都没有给"读/写模式"命名或定义（实测：只有 `CurveType`/`AutomationLane`
/// 这些类型名），因此这是本线的工程裁决，代价与依据记在
/// `docs/ledger/model-automation-notes.md`。
///
/// `Off` 仍是 `Default` 的派生值（用于内存构造），但 [ADR-0001 D43] 之后它**不再**是
/// 反序列化的兜底：`AutomationLane::write_mode` 缺失即报错。"不录制"是唯一的**安全**
/// 默认（默认"录制"会凭空产生写入），但它仍然是一个**选择**，必须被显式写出。
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum AutomationWriteMode {
    /// 不录制。
    #[default]
    Off,
    /// 走带时只要参数被改动就写入（写到底）。
    Write,
    /// 仅在该参数正被手动触碰时写入（松手即停）。
    Touch,
    /// 一直写入，直到显式停止（松手不停）。
    Latch,
}

impl AutomationWriteMode {
    /// 是否为 `Off`（隐式泳道的判据之一：`Off` 的泳道不参与 `is_implicit` 的判定）。
    #[must_use]
    pub const fn is_off(&self) -> bool {
        matches!(self, Self::Off)
    }
}

/// 自动化泳道（一个目标一条）[MODEL-AST-002]。
///
/// ## 泳道身份 = `AutomationTarget`
///
/// 泳道**没有**第二个身份字段：它在 `TrackV3::automation_lanes` 里的键就是
/// [`AutomationTarget`]，并且结构体里那份 `target` 必须与键一致（否则
/// [`ModelError::AutomationLaneTargetMismatch`]）。刻意不引入 `id: EntityId`：
/// 两个身份必然漂移（ADR-0001 D28 的同一理由），而寻址/撤销/下游调用全都按目标走。
///
/// ## 字段的必需性 [ADR-0001 D43]
///
/// - `read_enabled` / `write_mode` / `points` **必需**：它们曾经的 `#[serde(default)]`
///   唯一理由就是"旧工程可读"，而 `read_enabled = true` / `write_mode = Off` 本身就是
///   完整表达，写出来零成本。缺字段 ⇒ 响亮失败，绝不静默补一个开关状态。
///   与之配套，二者**不再** `skip_serializing_if`：否则本写入器写出的文档（默认值被省略）
///   会被本读取器拒绝 —— 宽容读与省略写必须同时取消。
/// - `domain` 保留 `#[serde(default)]`：它是 `Option<T>`，`None` 是**一等语义**
///   （"派生自目标"，见 [`AutomationLane::effective_domain`]），不是兼容让步。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AutomationLane {
    /// 该泳道控制的目标。
    pub target: AutomationTarget,
    /// 自动化点集合，键为点身份（`BTreeMap` 保证迭代顺序确定）[MODEL-AST-003]。
    ///
    /// **必需**：空泳道的确定编码是 `{}`（本写入器总是写出它），缺键意味着文件被截断。
    pub points: BTreeMap<EntityId, AutomationPoint>,
    /// **读**开关：走带/离线渲染是否应用本泳道。`false` 表示泳道被关掉
    /// （求值入口返回"无自动化值"，而不是返回曲线上的值）。
    ///
    /// **必需**：开关状态不许被默认值伪造 —— 缺 `read_enabled` 的文档会被静默当成
    /// "读打开"，于是被作者关掉的自动化悄悄生效。
    pub read_enabled: bool,
    /// **写**模式（录制）。
    ///
    /// **必需**：`Off` 是一个**选择**（"不录制"），不是一个可以省略的空值。
    pub write_mode: AutomationWriteMode,
    /// 该泳道的显式取值域覆盖；`None` 表示取目标的固有值域
    /// （见 `AutomationLane::effective_domain`）。
    ///
    /// `None` 是**一等语义**（"派生自目标"，信息不丢失），故保留 `#[serde(default)]`
    /// [ADR-0001 D43 对 `Option<T>` 的豁免]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<AutomationValueDomain>,
}

impl AutomationLane {
    /// 隐式泳道的规范形状：`SetAutomationPoint` 首次触碰某目标时创建的那一条。
    ///
    /// 这个形状是"隐式"的判据（[`AutomationLane::is_implicit`]）：它**不允许被持久化**
    /// —— 增删采样点会自动创建/回收它，因此它存不存在完全由文档里有没有点决定。
    #[must_use]
    pub fn implicit(target: AutomationTarget) -> Self {
        Self {
            target,
            points: BTreeMap::new(),
            read_enabled: true,
            write_mode: AutomationWriteMode::Off,
            domain: None,
        }
    }

    /// 是否与隐式泳道**逐位不可区分**（无点 + 全默认属性）。
    ///
    /// 这样的泳道不会被持久化：`SetAutomationLane` 拒绝创建它
    /// （否则撤销无法判定泳道该不该存在），`RemoveAutomationPoint` 在点被移空时回收它。
    #[must_use]
    pub fn is_implicit(&self) -> bool {
        self.points.is_empty()
            && self.read_enabled
            && self.write_mode.is_off()
            && self.domain.is_none()
    }

    /// 校验泳道自身：采样点的键/身份一致、取值有限。
    ///
    /// # Errors
    ///
    /// 键与 `point.id` 不一致 → [`ModelError::EntityKeyMismatch`]；
    /// 取值非有限 → [`ModelError::NonFiniteValue`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        for (point_id, point) in &self.points {
            if *point_id != point.id {
                return Err(ModelError::EntityKeyMismatch {
                    key: *point_id,
                    embedded: point.id,
                });
            }
            point.validate()?;
        }
        Ok(())
    }
}

/// `BTreeMap<AutomationTarget, AutomationLane>` 的 JSON 形态：按键升序的数组。
///
/// 为什么不序列化成 JSON 对象：`serde_json` 要求对象键是字符串，而
/// [`AutomationTarget`] 是结构化枚举（JSON 对象），当不了键。
/// 数组顺序由 `BTreeMap` 的键序（[`AutomationTarget`] 的 `Ord`）保证确定，
/// 因此同一工程两次导出的字节完全相同 [MODEL-AST-003]。
mod automation_lane_map {
    use std::collections::BTreeMap;

    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{AutomationLane, AutomationTarget};

    /// 按 `BTreeMap` 键序把泳道展开为数组。
    pub fn serialize<S: Serializer>(
        lanes: &BTreeMap<AutomationTarget, AutomationLane>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(lanes.len()))?;
        for lane in lanes.values() {
            sequence.serialize_element(lane)?;
        }
        sequence.end()
    }

    /// 把数组收拢回 `BTreeMap`；同一目标出现两次即拒绝。
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<AutomationTarget, AutomationLane>, D::Error> {
        let lanes = Vec::<AutomationLane>::deserialize(deserializer)?;
        let mut map: BTreeMap<AutomationTarget, AutomationLane> = BTreeMap::new();
        for lane in lanes {
            if map.insert(lane.target, lane).is_some() {
                return Err(serde::de::Error::custom(
                    "duplicate automation lane target in track.automation_lanes",
                ));
            }
        }
        Ok(map)
    }
}

/// 循环配置。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopConfig {
    /// 是否启用循环。
    ///
    /// **必需** [ADR-0001 D43]：缺了它会把一个打开的循环静默变成关闭。
    pub enabled: bool,
    /// 循环起点 (tick)。
    ///
    /// **必需** [ADR-0001 D43]：缺了它会把循环区间静默搬到 tick 0。
    pub start_tick: u64,
    /// 循环终点 (tick)。
    ///
    /// **必需** [ADR-0001 D43]：`0` 与一个真实的循环终点不可区分。
    pub end_tick: u64,
}

impl LoopConfig {
    /// 校验循环区间。
    ///
    /// # Errors
    ///
    /// 启用循环但 `end_tick <= start_tick` → [`ModelError::ZeroDuration`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.enabled && self.end_tick <= self.start_tick {
            return Err(ModelError::ZeroDuration);
        }
        Ok(())
    }
}

/// 片段池条目 [MODEL-AST-002]。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClipPoolEntry {
    /// 片段身份。
    pub id: EntityId,
    /// 显示名。
    ///
    /// **必需** [ADR-0001 D43]：片段标签是实体自身的一部分（同族字段
    /// `TrackV3::name` / `DeviceDefinition::name` 都没有默认值）。
    pub name: String,
    /// 片段内容。
    pub content: ClipContent,
}

/// 片段内容。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ClipContent {
    /// MIDI 片段：音符集合，键为音符身份（`BTreeMap`）[MODEL-AST-003, MODEL-AST-005]。
    Midi {
        /// 音符集合。
        ///
        /// **必需** [ADR-0001 D43]：这是音符的**唯一**载体，缺键等于静默丢掉整个片段
        /// 的音乐内容（空片段的确定编码是 `{}`）。
        notes: BTreeMap<EntityId, MidiNote>,
    },
    /// 音频片段：内容寻址的资产引用 + 增益。
    Audio {
        /// 资产哈希（CAS 键）[MODEL-AST-007]。
        asset: AssetHash,
        /// 片段增益 (dB)。
        ///
        /// **必需** [ADR-0001 D43]：`0.0` 是一个**真实增益**（单位增益），不是一个
        /// "未填"标记 —— 缺字段会让一个 -1.5 dB 的片段静默变成 0 dB。
        /// 对比 [`RoutingEdge::gain_db`]：那里"单位增益"被显式建模成 `Option::None`，
        /// 是**语义**；这里的裸 `f32` 只能靠"必须写出来"来消除同样的歧义。
        gain_db: f32,
    },
}

impl Default for ClipContent {
    fn default() -> Self {
        Self::Midi {
            notes: BTreeMap::new(),
        }
    }
}

impl ClipContent {
    /// 校验片段内容。
    ///
    /// # Errors
    ///
    /// MIDI 片段：任一音符非法，或音符集合的键与其 `id` 不一致；
    /// 音频片段：增益非有限。
    pub fn validate(&self) -> Result<(), ModelError> {
        match self {
            Self::Midi { notes } => {
                for (key, note) in notes {
                    if *key != note.id {
                        return Err(ModelError::EntityKeyMismatch {
                            key: *key,
                            embedded: note.id,
                        });
                    }
                    note.validate()?;
                }
                Ok(())
            }
            Self::Audio { gain_db, .. } => {
                if !gain_db.is_finite() {
                    return Err(ModelError::NonFiniteValue {
                        field: "clip.audio.gain_db",
                        value: f64::from(*gain_db),
                    });
                }
                Ok(())
            }
        }
    }

    /// 只读访问 MIDI 音符集合；非 MIDI 片段返回 `None`。
    #[must_use]
    pub fn notes(&self) -> Option<&BTreeMap<EntityId, MidiNote>> {
        match self {
            Self::Midi { notes } => Some(notes),
            Self::Audio { .. } => None,
        }
    }

    /// 可变访问 MIDI 音符集合；非 MIDI 片段返回 `None`。
    #[must_use]
    pub fn notes_mut(&mut self) -> Option<&mut BTreeMap<EntityId, MidiNote>> {
        match self {
            Self::Midi { notes } => Some(notes),
            Self::Audio { .. } => None,
        }
    }
}

/// 片段摆放（placement）：把片段池条目放到时间轴上。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipPlacement {
    /// 摆放身份。
    pub id: EntityId,
    /// 指向片段池条目。
    pub clip_id: EntityId,
    /// 起始 tick。
    pub start_tick: u64,
    /// 摆放时值 (tick)，必须非零。
    pub duration_ticks: u64,
    /// 循环配置。
    ///
    /// **必需** [ADR-0001 D43]：这是一个**子结构**，默认值会凭空造出一个"循环关闭、
    /// 区间 0..0"的配置，而不是表达"作者没配置循环"。
    pub loop_config: LoopConfig,
    /// 是否静音。
    ///
    /// **必需** [ADR-0001 D43]：缺了它会把一个静音的摆放静默放出来（可听的行为反转）。
    pub muted: bool,
}

impl Default for ClipPlacement {
    fn default() -> Self {
        Self {
            id: EntityId::default(),
            clip_id: EntityId::default(),
            start_tick: 0,
            duration_ticks: crate::ids::PPQ,
            loop_config: LoopConfig::default(),
            muted: false,
        }
    }
}

impl ClipPlacement {
    /// 校验摆放自身的取值范围。
    ///
    /// # Errors
    ///
    /// 时值为零 → [`ModelError::ZeroDuration`]；循环区间非法 → [`ModelError::ZeroDuration`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.duration_ticks == 0 {
            return Err(ModelError::ZeroDuration);
        }
        self.loop_config.validate()
    }
}

/// 音轨 [MODEL-AST-002]。
///
/// `folder_id` **仅**用于界面层树状折叠，严禁承载音频信号语义
/// [ROAD-M1-002]：音频连接一律由 [`RoutingGraph`] 表达。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrackV3 {
    /// 音轨身份。
    pub id: EntityId,
    /// 显示名。
    pub name: String,
    /// 音轨类型。
    pub kind: TrackKind,
    /// 音量 (dB)。
    pub volume_db: f32,
    /// 声相 -1.0..=1.0。
    pub pan: f32,
    /// 静音。
    pub mute: bool,
    /// 独奏。
    pub solo: bool,
    /// 独奏安全（不被其它轨的 solo 静音）。
    ///
    /// **必需** [ADR-0001 D43]：它改变 solo 的混音结果，且与同族字段
    /// `mute` / `solo`（都无默认值）对称 —— 三个混音开关要么都写，要么都别写。
    pub solo_safe: bool,
    /// 所属折叠文件夹（**仅界面语义**）。
    ///
    /// `None` 是**一等状态**（"不折叠进任何文件夹"），保留 `#[serde(default)]`
    /// [ADR-0001 D43 对 `Option<T>` 的豁免]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<EntityId>,
    /// 界面色标。
    ///
    /// `None` 是**一等状态**（"用主题默认色"），保留 `#[serde(default)]`
    /// [ADR-0001 D43 对 `Option<T>` 的豁免]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// 设备链（有序）。
    ///
    /// **必需** [ADR-0001 D43]：设备链是音轨内容的一部分，缺键等于静默清空整条链。
    pub devices: Vec<DeviceDefinition>,
    /// 宏。
    ///
    /// **必需** [ADR-0001 D43]：同上，缺键等于静默清空全部宏（连同它们的映射）。
    pub macros: Vec<MacroParameter>,
    /// 自动化泳道集合（`BTreeMap`，键为目标）[MODEL-AST-003]。
    ///
    /// JSON 形态是**按键升序的数组**：`serde_json` 的对象键必须是字符串，
    /// 而 [`AutomationTarget`] 是结构化枚举（序列化为对象），无法直接当键。
    /// 详见 `automation_lane_map`。
    ///
    /// **必需** [ADR-0001 D43]：缺键等于静默清空整条音轨的自动化。
    #[serde(with = "automation_lane_map")]
    pub automation_lanes: BTreeMap<AutomationTarget, AutomationLane>,
    /// 时间轴摆放集合（`BTreeMap`，键为摆放身份）[MODEL-AST-003]。
    ///
    /// **必需** [ADR-0001 D43]：缺键等于静默清空整条音轨的所有摆放。
    pub clips: BTreeMap<EntityId, ClipPlacement>,
}

impl Default for TrackV3 {
    fn default() -> Self {
        Self {
            id: EntityId::default(),
            name: "Track".to_owned(),
            kind: TrackKind::default(),
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            solo_safe: false,
            folder_id: None,
            color: None,
            devices: Vec::new(),
            macros: Vec::new(),
            automation_lanes: BTreeMap::new(),
            clips: BTreeMap::new(),
        }
    }
}

impl TrackV3 {
    /// 校验音轨自身的取值范围（跨实体引用由 [`YebanProjectV1::validate`] 负责）。
    ///
    /// # Errors
    ///
    /// - 音量/声相非有限 → [`ModelError::NonFiniteValue`]；
    /// - 声相越界 → [`ModelError::PanOutOfRange`]；
    /// - 自动化泳道的键与 `lane.target` 不一致，或泳道的目标指向**别的**音轨
    ///   → [`ModelError::AutomationLaneTargetMismatch`]；
    /// - 设备/宏/自动化/摆放非法 → 冒泡对应错误。
    pub fn validate(&self) -> Result<(), ModelError> {
        if !self.volume_db.is_finite() {
            return Err(ModelError::NonFiniteValue {
                field: "track.volume_db",
                value: f64::from(self.volume_db),
            });
        }
        if !self.pan.is_finite() {
            return Err(ModelError::NonFiniteValue {
                field: "track.pan",
                value: f64::from(self.pan),
            });
        }
        if !(-1.0..=1.0).contains(&self.pan) {
            return Err(ModelError::PanOutOfRange { value: self.pan });
        }
        for device in &self.devices {
            device.validate()?;
        }
        for macro_parameter in &self.macros {
            macro_parameter.validate()?;
        }
        for (target, lane) in &self.automation_lanes {
            if *target != lane.target {
                return Err(ModelError::AutomationLaneTargetMismatch {
                    key: format!("{target:?}"),
                    embedded: format!("{:?}", lane.target),
                });
            }
            // 位置规则：泳道必须挂在**它自己的目标所指的那条音轨**上。
            //
            // 依据：[`AutomationTarget::track_id`] 的文档把"泳道在文档里的位置"定义为该目标
            // 自己的音轨。而 `automation_lanes` 的 JSON 形态是**数组**（数组元素的键就是
            // `lane.target`），因此上面那条"键 == 载荷"的比对在**反序列化得到的**文档上
            // 恒真 —— "泳道挂在别的音轨上"这一种自相矛盾只能在这里拦住。
            //
            // 为什么不能放行：唯一求值入口 [`YebanProjectV1::automation_lane`] 只查目标
            // 自己的音轨，因此挂错音轨的泳道**永远读不到**（静默失效）；而按音轨遍历的
            // 消费者（界面投影逐轨读 `automation_lanes`）却会把这条曲线画出来。同一份
            // 文档在两个消费者眼里成为两件事，且没有任何报错 —— 按 [ADR-0001 D43]
            // "让损坏的文件响亮失败"的口径，这是必须拒绝的自相矛盾输入。
            if target.track_id() != self.id {
                return Err(ModelError::AutomationLaneTargetMismatch {
                    key: self.id.to_canonical_string(),
                    embedded: target.track_id().to_canonical_string(),
                });
            }
            lane.validate()?;
        }
        for (placement_id, placement) in &self.clips {
            if *placement_id != placement.id {
                return Err(ModelError::EntityKeyMismatch {
                    key: *placement_id,
                    embedded: placement.id,
                });
            }
            placement.validate()?;
        }
        Ok(())
    }
}

/// 曲式段落。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SectionV3 {
    /// 段落身份。
    pub id: EntityId,
    /// 段落名（如 `Intro` / `Drop`）。
    pub name: String,
    /// 起始 tick。
    pub start_tick: u64,
    /// 结束 tick。
    pub end_tick: u64,
    /// 界面色标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl SectionV3 {
    /// 校验段落区间。
    ///
    /// # Errors
    ///
    /// `end_tick <= start_tick` → [`ModelError::ZeroDuration`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.end_tick <= self.start_tick {
            return Err(ModelError::ZeroDuration);
        }
        Ok(())
    }
}

/// 场景（Session View 卡片行）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SceneV3 {
    /// 场景身份。
    pub id: EntityId,
    /// 场景名。
    pub name: String,
    /// 场景速度覆盖（`None` 表示跟随工程速度）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo: Option<f64>,
    /// 界面色标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl SceneV3 {
    /// 校验速度覆盖范围。
    ///
    /// # Errors
    ///
    /// 速度非有限或越出 20.0..=999.0 → [`ModelError::BpmOutOfRange`] /
    /// [`ModelError::NonFiniteValue`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if let Some(tempo) = self.tempo {
            if !tempo.is_finite() {
                return Err(ModelError::NonFiniteValue {
                    field: "scene.tempo",
                    value: tempo,
                });
            }
            if !(MIN_BPM..=MAX_BPM).contains(&tempo) {
                return Err(ModelError::BpmOutOfRange { value: tempo });
            }
        }
        Ok(())
    }
}

/// 路由类型 [MODEL-AST-004]。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoutingKind {
    /// 音轨 → 总线。
    #[default]
    TrackToBus,
    /// 总线 → 主输出。
    BusToMaster,
    /// 发送 → 辅助返回。
    SendToAux,
    /// 侧链。
    Sidechain,
}

/// 路由边 [MODEL-AST-004]。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct RoutingEdge {
    /// 边身份。
    pub id: EntityId,
    /// 源节点（音轨/总线身份）。
    pub source_node: EntityId,
    /// 目标节点（音轨/总线身份）。
    pub destination_node: EntityId,
    /// 路由类型。
    pub kind: RoutingKind,
    /// 该边的增益 (dB)；`None` 表示单位增益。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
}

impl RoutingEdge {
    /// 校验边的数值有限性。
    ///
    /// # Errors
    ///
    /// 增益非有限 → [`ModelError::NonFiniteValue`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if let Some(gain_db) = self.gain_db
            && !gain_db.is_finite()
        {
            return Err(ModelError::NonFiniteValue {
                field: "routing.edge.gain_db",
                value: f64::from(gain_db),
            });
        }
        Ok(())
    }
}

/// 唯一声学路由真理源 [MODEL-AST-004, ROAD-M1-002]。
///
/// ## 内存形态 vs 序列化形态（有意不同，且都有规范依据）
///
/// - **内存**：`edges` 是 `BTreeMap<EntityId, RoutingEdge>` —— 红线 4
///   [MODEL-AST-003] 要求持久化 AST 实体集合必须是 `BTreeMap`；
/// - **JSON**：`edges` 必须是**数组** —— `schemas/project.schema.json`
///   把 `routing_graph.edges` 声明为 `array`。
///
/// 因此这里手写 `Serialize`/`Deserialize`：序列化时按键升序把 `BTreeMap`
/// 展开为数组，反序列化时收拢回 `BTreeMap`。**两个要求同时被满足**，
/// 且数组顺序是确定的（键升序），不会出现"同一工程两次导出字节不同"。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoutingGraph {
    /// 参与路由的节点身份列表（音轨/总线）。
    pub nodes: Vec<EntityId>,
    /// 路由边集合，键为边身份。
    pub edges: BTreeMap<EntityId, RoutingEdge>,
}

impl RoutingGraph {
    /// 校验图自洽性。
    ///
    /// 检查项：节点列表无重复；每条边的键与 `id` 一致；边的两端节点都存在；
    /// 每条边的增益有限。
    ///
    /// # Errors
    ///
    /// 违反上述任一条件即返回对应的 [`ModelError`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        let mut seen: Vec<&EntityId> = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            if seen.contains(&node) {
                return Err(ModelError::DuplicateEntityId { id: *node });
            }
            seen.push(node);
        }
        for (key, edge) in &self.edges {
            if *key != edge.id {
                return Err(ModelError::EntityKeyMismatch {
                    key: *key,
                    embedded: edge.id,
                });
            }
            for endpoint in [edge.source_node, edge.destination_node] {
                if !self.nodes.contains(&endpoint) {
                    return Err(ModelError::RoutingNodeNotFound { id: endpoint });
                }
            }
            edge.validate()?;
        }
        Ok(())
    }

    /// 只读访问一条边。
    #[must_use]
    pub fn edge(&self, id: &EntityId) -> Option<&RoutingEdge> {
        self.edges.get(id)
    }
}

/// `RoutingGraph` 的 JSON 中间形态（`edges` 为数组）。
#[derive(Serialize, Deserialize)]
struct RoutingGraphJson {
    nodes: Vec<EntityId>,
    edges: Vec<RoutingEdge>,
}

impl Serialize for RoutingGraph {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let edges: Vec<&RoutingEdge> = self.edges.values().collect();
        let mut state = serializer.serialize_struct("RoutingGraph", 2)?;
        state.serialize_field("nodes", &self.nodes)?;
        state.serialize_field("edges", &edges)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for RoutingGraph {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let json = RoutingGraphJson::deserialize(deserializer)?;
        let mut edges: BTreeMap<EntityId, RoutingEdge> = BTreeMap::new();
        for edge in json.edges {
            if edges.insert(edge.id, edge).is_some() {
                return Err(serde::de::Error::custom(
                    "duplicate routing edge id in routing_graph.edges",
                ));
            }
        }
        Ok(Self {
            nodes: json.nodes,
            edges,
        })
    }
}

/// 资产媒体类型。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaKind {
    /// 音频素材。
    #[default]
    Audio,
    /// MIDI 素材。
    Midi,
    /// 脉冲响应。
    ImpulseResponse,
}

/// 资产元数据（内容寻址池的索引项）[MODEL-AST-007]。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AssetMetadata {
    /// 资产内容的 SHA-256（CAS 键）。
    pub hash: AssetHash,
    /// 归档内的原始路径（仅留痕，不参与寻址）。
    pub original_path: String,
    /// 字节长度。
    pub byte_len: u64,
    /// 媒体类型。
    pub media_kind: MediaKind,
    /// 许可标识（`MUST-GATE-014` 要求逐条可查）。
    pub license: String,
}

/// 顶层工程文档：唯一权威持久化结构 [MODEL-AST-002]。
///
/// 字段与 `schemas/project.schema.json` 的对应关系：
/// 必填键全部为直接字段；`tracks`/`clip_pool` 是 `BTreeMap`（JSON 对象）；
/// `routing_graph` 见 [`RoutingGraph`]。
///
/// **没有顶层 `sample_rate`**：采样率唯一来源是 `audio_config.sample_rate`。
///
/// ## 反序列化严格性 [ADR-0001 D43]
///
/// 顶层每一个键都**必需**：本写入器总是写出全部顶层键，因此"缺键"不可能来自本写入器
/// —— 它只可能来自被截断或异源的文件，必须**响亮失败**而不是被默认值悄悄补全。
/// 唯一例外是 [`YebanProjectV1::author`]（自由文本注记，空串即"未署名"，
/// 是**语义**默认而非兼容）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct YebanProjectV1 {
    /// 文档 schema 版本（本写入器产出 [`SCHEMA_VERSION`] = 1）。
    pub schema_version: u32,
    /// 读取本文档所需的**最低**读取器版本。
    pub min_reader_version: u32,
    /// 写入程序版本标记（语义版本字符串，见 ADR-0001 D3）。
    pub writer_version: String,
    /// 工程身份（26 字符 Crockford Base32 ULID）。
    pub id: EntityId,
    /// 作品标题（顶层唯一权威）。
    pub title: String,
    /// 作者。
    ///
    /// `#[serde(default)]` 是**语义** [ADR-0001 D43]: 署名是自由文本注记, 空串是
    /// "未署名"的确定编码, 不参与任何工程行为。
    #[serde(default)]
    pub author: String,
    /// 速度 20.0..=999.0（顶层唯一权威）。
    pub bpm: f64,
    /// 拍号（顶层唯一权威）。
    pub time_signature: TimeSignature,
    /// 音频配置 —— **采样率与声相律的唯一来源**。
    pub audio_config: ProjectAudioConfig,
    /// 确定性随机种子（`probability` 触发判定等算法的跨机一致性）[MODEL-AST-005]。
    ///
    /// **必需** [ADR-0001 D43]：它决定 `probability` 的触发判定，缺字段会让同一份乐谱
    /// 在"默认种子"下静默换一套触发结果 —— 这是听得出、读文件时看不见的行为漂移。
    pub rng_seed: u64,
    /// 工程元数据。
    ///
    /// **必需** [ADR-0001 D43]：整个子结构都不许被默认值顶替（否则创建/修改时间会被
    /// 伪造成 1970）。
    pub metadata: ProjectMetadata,
    /// 传输控制配置。
    ///
    /// **必需** [ADR-0001 D43]：走带/录音配置必须被显式陈述（见 [`TransportConfig`]）。
    pub transport: TransportConfig,
    /// 曲式段落集合。
    ///
    /// **必需** [ADR-0001 D43]：缺键等于静默清空全部段落。
    pub sections: BTreeMap<EntityId, SectionV3>,
    /// 音轨集合 [MODEL-AST-003]。
    ///
    /// **必需** [ADR-0001 D43]：契约（`schemas/project.schema.json`）早已把 `tracks`
    /// 列为 required，此前的 `#[serde(default)]` 让实现**静默违反自己的契约**；
    /// 而且缺键等于静默清空整首曲子的音轨。
    pub tracks: BTreeMap<EntityId, TrackV3>,
    /// 主总线音轨身份。
    ///
    /// **必需** [ADR-0001 D43]：这是唯一声学出口的身份，缺字段会静默退化成 nil。
    pub master_bus_track_id: EntityId,
    /// 唯一声学路由真理源 [MODEL-AST-004]。
    ///
    /// **必需** [ADR-0001 D43]：契约已列为 required；缺键等于静默清空全部路由。
    pub routing_graph: RoutingGraph,
    /// 场景集合。
    ///
    /// **必需** [ADR-0001 D43]：缺键等于静默清空全部场景。
    pub scenes: BTreeMap<EntityId, SceneV3>,
    /// 片段池。
    ///
    /// **必需** [ADR-0001 D43]：契约已列为 required；缺键等于静默清空整个片段池。
    pub clip_pool: BTreeMap<EntityId, ClipPoolEntry>,
    /// 资产索引（键为内容哈希）。
    ///
    /// **必需** [ADR-0001 D43]：缺键等于静默丢掉整份资产索引。
    pub assets: BTreeMap<AssetHash, AssetMetadata>,
}

impl Default for YebanProjectV1 {
    /// 一个**合法且可读**的空工程（无音轨、无片段、4/4、120 BPM、48 kHz）。
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            min_reader_version: MIN_READER_VERSION,
            writer_version: DEFAULT_WRITER_VERSION.to_owned(),
            id: EntityId::default(),
            title: "Untitled".to_owned(),
            author: String::new(),
            bpm: DEFAULT_BPM,
            time_signature: TimeSignature::default(),
            audio_config: ProjectAudioConfig::default(),
            rng_seed: DEFAULT_RNG_SEED,
            metadata: ProjectMetadata::default(),
            transport: TransportConfig::default(),
            sections: BTreeMap::new(),
            tracks: BTreeMap::new(),
            master_bus_track_id: EntityId::default(),
            routing_graph: RoutingGraph::default(),
            scenes: BTreeMap::new(),
            clip_pool: BTreeMap::new(),
            assets: BTreeMap::new(),
        }
    }
}

impl YebanProjectV1 {
    /// 版本门：判断本文档能否被本读取器安全读取 [ROAD-M1-001, RSK-16]。
    ///
    /// 规则（拒绝而不是猜测）：
    /// 1. `schema_version > READER_SCHEMA_VERSION` → [`ModelError::SchemaVersionTooNew`]；
    /// 2. `min_reader_version > READER_SCHEMA_VERSION` → [`ModelError::ReaderTooOld`]。
    ///
    /// # Errors
    ///
    /// 上述两条之一成立时返回对应错误。
    pub fn check_readable(&self) -> Result<(), ModelError> {
        if self.schema_version > READER_SCHEMA_VERSION {
            return Err(ModelError::SchemaVersionTooNew {
                found: self.schema_version,
                supported: READER_SCHEMA_VERSION,
            });
        }
        if self.min_reader_version > READER_SCHEMA_VERSION {
            return Err(ModelError::ReaderTooOld {
                required: self.min_reader_version,
                actual: READER_SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    /// 结构校验：全部数值范围与跨实体引用自洽性。
    ///
    /// 校验项（越界即 `Err`，绝不 panic —— 文档来自磁盘/归档，属不可信输入）：
    /// 顶层 `bpm`、拍号、主总线音轨存在且类型正确、集合键与实体 `id` 一致、
    /// `folder_id` 有指向、音轨/片段/资产/路由图各自合法、
    /// 摆放引用的片段存在于片段池。
    ///
    /// 另有一条**跨集合**一致性（本判据的下半部分）：
    /// [`RoutingGraph::nodes`](RoutingGraph::nodes) 必须够得着唯一声学出口 ——
    /// 有音轨时 [`YebanProjectV1::master_bus_track_id`] 必须在 `nodes` 里；
    /// 没有音轨时主总线身份必须是 nil 且 `nodes` 必须为空
    /// （空工程里任何节点身份都悬空）。理由见 `validate` 函数体内那段注释。
    ///
    /// # Errors
    ///
    /// 违反上述任一条件即返回对应的 [`ModelError`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if !self.bpm.is_finite() {
            return Err(ModelError::NonFiniteValue {
                field: "bpm",
                value: self.bpm,
            });
        }
        if !(MIN_BPM..=MAX_BPM).contains(&self.bpm) {
            return Err(ModelError::BpmOutOfRange { value: self.bpm });
        }
        self.time_signature.validate()?;

        if self.tracks.is_empty() {
            if !self.master_bus_track_id.is_nil() {
                return Err(ModelError::TrackNotFound {
                    id: self.master_bus_track_id,
                });
            }
        } else {
            let master =
                self.tracks
                    .get(&self.master_bus_track_id)
                    .ok_or(ModelError::TrackNotFound {
                        id: self.master_bus_track_id,
                    })?;
            if master.kind != TrackKind::Master {
                return Err(ModelError::MasterBusKindMismatch {
                    id: self.master_bus_track_id,
                });
            }
        }

        for (key, track) in &self.tracks {
            if *key != track.id {
                return Err(ModelError::EntityKeyMismatch {
                    key: *key,
                    embedded: track.id,
                });
            }
            track.validate()?;
            if let Some(folder_id) = track.folder_id
                && !self.tracks.contains_key(&folder_id)
            {
                return Err(ModelError::TrackNotFound { id: folder_id });
            }
            for placement in track.clips.values() {
                if !self.clip_pool.contains_key(&placement.clip_id) {
                    return Err(ModelError::ClipNotFound {
                        id: placement.clip_id,
                    });
                }
            }
        }

        for (key, entry) in &self.clip_pool {
            if *key != entry.id {
                return Err(ModelError::EntityKeyMismatch {
                    key: *key,
                    embedded: entry.id,
                });
            }
            entry.content.validate()?;
        }

        for (key, section) in &self.sections {
            if *key != section.id {
                return Err(ModelError::EntityKeyMismatch {
                    key: *key,
                    embedded: section.id,
                });
            }
            section.validate()?;
        }

        for (key, scene) in &self.scenes {
            if *key != scene.id {
                return Err(ModelError::EntityKeyMismatch {
                    key: *key,
                    embedded: scene.id,
                });
            }
            scene.validate()?;
        }

        for (key, asset) in &self.assets {
            if *key != asset.hash {
                return Err(ModelError::AssetKeyMismatch {
                    key: key.clone(),
                    embedded: asset.hash.clone(),
                });
            }
        }

        self.routing_graph.validate()?;

        // 主总线必须**够得着**：把主总线身份放进 `routing_graph.nodes`
        // [MODEL-AST-004, ROAD-M1-002]。
        //
        // `RoutingGraph` 是唯一声学真理源，而"主总线是唯一声学出口"这件事只有
        // 主总线出现在节点集合里才成立：图的下游消费者（`yeban-engine` 的
        // `PdcPlan::compute`）以"主总线在 `nodes` 里"为**前置条件**，不满足就直接
        // 报 `PdcError::UnknownMaster`，一个块都不渲染。在本条判据出现之前，
        // 模型对"有音轨、但主总线不在 `nodes` 里"的文档**照单全收**（`validate()`
        // 返回 `Ok`）—— 模型说合法、引擎说不可渲染，同一份文档在两个消费者眼里
        // 是两件事，且读文件时看不见。按 [ADR-0001 D43] 的"让损坏的文件响亮失败"
        // 口径，这是必须拒绝的自相矛盾输入。
        //
        // 空工程一侧是它的对偶：没有音轨时主总线身份必须是 nil（上面那条
        // `TrackNotFound` 已经管住），此时**任何**节点身份都不可能对应一份存在的
        // 音轨，因此 `nodes` 必须为空。默认工程恰好取这一支。
        //
        // 错误码复用 `RoutingNodeNotFound`（主总线身份就是它在图里该有的那一个
        // 节点身份），理由与 `line/model-automation` 的 N5 相同：新增 `ModelError`
        // 变体会牵动 `yeban-mcp` 的 `code_for_model` 穷举 match 与契约码表，
        // 那是**另一条线**的范围。
        if self.tracks.is_empty() {
            if let Some(node) = self.routing_graph.nodes.first() {
                return Err(ModelError::RoutingNodeNotFound { id: *node });
            }
        } else if !self.routing_graph.nodes.contains(&self.master_bus_track_id) {
            return Err(ModelError::RoutingNodeNotFound {
                id: self.master_bus_track_id,
            });
        }
        Ok(())
    }

    /// 工程采样率的唯一读取入口（顶层没有冗余副本）[MODEL-AST-002]。
    #[must_use]
    pub const fn sample_rate(&self) -> SampleRate {
        self.audio_config.sample_rate
    }

    /// 追加音轨；身份重复即拒绝。
    ///
    /// 载荷先经 [`TrackV3::validate`]（与 [`Op::AddTrack`](crate::ops::Op::AddTrack)
    /// 同一把尺子，也与同族的 [`YebanProjectV1::insert_note`] 对称）：直接落进文档的
    /// 外部数值 —— 音量、声相、宏位置、自动化点值 —— 必须在**入口**就被判定，
    /// 否则文档会先被污染、再由 [`YebanProjectV1::validate`] 在别处报错（类别 1）。
    ///
    /// # Errors
    ///
    /// - 音轨自身非法 → 冒泡 [`TrackV3::validate`] 的错误（含非有限数值）；
    /// - `track.id` 已存在 → [`ModelError::DuplicateEntityId`]。
    pub fn insert_track(&mut self, track: TrackV3) -> Result<(), ModelError> {
        track.validate()?;
        if self.tracks.contains_key(&track.id) {
            return Err(ModelError::DuplicateEntityId { id: track.id });
        }
        self.tracks.insert(track.id, track);
        Ok(())
    }

    /// 移除音轨并返回它（撤销需要 `previous_track` 载荷）。
    ///
    /// # Errors
    ///
    /// 音轨不存在 → [`ModelError::TrackNotFound`]。
    pub fn remove_track(&mut self, id: &EntityId) -> Result<TrackV3, ModelError> {
        self.tracks
            .remove(id)
            .ok_or(ModelError::TrackNotFound { id: *id })
    }

    /// 只读访问音轨。
    ///
    /// # Errors
    ///
    /// 音轨不存在 → [`ModelError::TrackNotFound`]。
    pub fn track(&self, id: &EntityId) -> Result<&TrackV3, ModelError> {
        self.tracks
            .get(id)
            .ok_or(ModelError::TrackNotFound { id: *id })
    }

    /// 可变访问音轨。
    ///
    /// # Errors
    ///
    /// 音轨不存在 → [`ModelError::TrackNotFound`]。
    pub fn track_mut(&mut self, id: &EntityId) -> Result<&mut TrackV3, ModelError> {
        self.tracks
            .get_mut(id)
            .ok_or(ModelError::TrackNotFound { id: *id })
    }

    /// 追加片段池条目；身份重复即拒绝。
    ///
    /// 载荷先经 [`ClipContent::validate`]（与 [`Op::AddClip`](crate::ops::Op::AddClip)
    /// 同一把尺子）：音频片段的 `gain_db` 与 MIDI 音符的 `probability` 都是外部数值，
    /// 落在入口之外检查会让文档先被污染（类别 1）。
    ///
    /// # Errors
    ///
    /// - 内容非法 → 冒泡 [`ClipContent::validate`] 的错误（含非有限数值）；
    /// - `entry.id` 已存在 → [`ModelError::DuplicateEntityId`]。
    pub fn insert_clip(&mut self, entry: ClipPoolEntry) -> Result<(), ModelError> {
        entry.content.validate()?;
        if self.clip_pool.contains_key(&entry.id) {
            return Err(ModelError::DuplicateEntityId { id: entry.id });
        }
        self.clip_pool.insert(entry.id, entry);
        Ok(())
    }

    /// 移除片段池条目并返回它。
    ///
    /// # Errors
    ///
    /// 片段不存在 → [`ModelError::ClipNotFound`]。
    pub fn remove_clip(&mut self, id: &EntityId) -> Result<ClipPoolEntry, ModelError> {
        self.clip_pool
            .remove(id)
            .ok_or(ModelError::ClipNotFound { id: *id })
    }

    /// 可变访问片段池条目。
    ///
    /// # Errors
    ///
    /// 片段不存在 → [`ModelError::ClipNotFound`]。
    pub fn clip_mut(&mut self, id: &EntityId) -> Result<&mut ClipPoolEntry, ModelError> {
        self.clip_pool
            .get_mut(id)
            .ok_or(ModelError::ClipNotFound { id: *id })
    }

    /// 在指定片段中插入音符；`clip_id` 必须存在且必须是 MIDI 片段。
    ///
    /// # Errors
    ///
    /// - 片段不存在 → [`ModelError::ClipNotFound`]；
    /// - 片段是音频片段 → [`ModelError::ClipContentKindMismatch`]；
    /// - 音符身份已存在 → [`ModelError::DuplicateEntityId`]；
    /// - 音符本身非法 → 冒泡音符校验错误。
    pub fn insert_note(&mut self, clip_id: &EntityId, note: MidiNote) -> Result<(), ModelError> {
        note.validate()?;
        let entry = self.clip_mut(clip_id)?;
        let notes = entry
            .content
            .notes_mut()
            .ok_or(ModelError::ClipContentKindMismatch { id: *clip_id })?;
        if notes.contains_key(&note.id) {
            return Err(ModelError::DuplicateEntityId { id: note.id });
        }
        notes.insert(note.id, note);
        Ok(())
    }

    /// 移除片段中的音符并返回它（撤销需要 `previous_note` 载荷）。
    ///
    /// # Errors
    ///
    /// - 片段不存在 → [`ModelError::ClipNotFound`]；
    /// - 片段不是 MIDI 片段 → [`ModelError::ClipContentKindMismatch`]；
    /// - 音符不存在 → [`ModelError::NoteNotFound`]。
    pub fn remove_note(
        &mut self,
        clip_id: &EntityId,
        note_id: &EntityId,
    ) -> Result<MidiNote, ModelError> {
        let entry = self.clip_mut(clip_id)?;
        let notes = entry
            .content
            .notes_mut()
            .ok_or(ModelError::ClipContentKindMismatch { id: *clip_id })?;
        notes
            .remove(note_id)
            .ok_or(ModelError::NoteNotFound { id: *note_id })
    }

    /// 可变访问片段中的音符。
    ///
    /// # Errors
    ///
    /// 片段或音符不存在时返回对应错误。
    pub fn note_mut(
        &mut self,
        clip_id: &EntityId,
        note_id: &EntityId,
    ) -> Result<&mut MidiNote, ModelError> {
        let entry = self.clip_mut(clip_id)?;
        let notes = entry
            .content
            .notes_mut()
            .ok_or(ModelError::ClipContentKindMismatch { id: *clip_id })?;
        notes
            .get_mut(note_id)
            .ok_or(ModelError::NoteNotFound { id: *note_id })
    }

    /// 只读访问片段中的音符。
    ///
    /// # Errors
    ///
    /// 片段或音符不存在时返回对应错误。
    pub fn note(&self, clip_id: &EntityId, note_id: &EntityId) -> Result<&MidiNote, ModelError> {
        let entry = self
            .clip_pool
            .get(clip_id)
            .ok_or(ModelError::ClipNotFound { id: *clip_id })?;
        let notes = entry
            .content
            .notes()
            .ok_or(ModelError::ClipContentKindMismatch { id: *clip_id })?;
        notes
            .get(note_id)
            .ok_or(ModelError::NoteNotFound { id: *note_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // 样本夹具的唯一事实源在 `crate::samples`（同一个 `filled_project` 也被
    // `examples/export_schema_samples.rs` 使用，避免"测试一套、导出另一套"）。
    use crate::samples::{filled_project, fixture_id, master_track, midi_clip, midi_track};

    #[test]
    fn schema_version_is_one_per_adr_0001_d3() {
        assert_eq!(SCHEMA_VERSION, 1);
        assert_eq!(YebanProjectV1::default().schema_version, 1);
    }

    #[test]
    fn default_project_is_valid_and_readable() {
        let project = YebanProjectV1::default();
        assert_eq!(project.check_readable(), Ok(()));
        assert_eq!(project.validate(), Ok(()));
    }

    #[test]
    fn version_gate_rejects_too_new_schema() {
        let project = YebanProjectV1 {
            schema_version: SCHEMA_VERSION + 1,
            ..YebanProjectV1::default()
        };
        assert_eq!(
            project.check_readable(),
            Err(ModelError::SchemaVersionTooNew {
                found: SCHEMA_VERSION + 1,
                supported: READER_SCHEMA_VERSION,
            })
        );
    }

    #[test]
    fn version_gate_rejects_reader_that_is_too_old() {
        let project = YebanProjectV1 {
            min_reader_version: READER_SCHEMA_VERSION + 1,
            ..YebanProjectV1::default()
        };
        assert_eq!(
            project.check_readable(),
            Err(ModelError::ReaderTooOld {
                required: READER_SCHEMA_VERSION + 1,
                actual: READER_SCHEMA_VERSION,
            })
        );
    }

    #[test]
    fn bpm_out_of_range_is_rejected() {
        for bad in [19.9_f64, 999.5, 0.0, -120.0, f64::NAN, f64::INFINITY] {
            let project = YebanProjectV1 {
                bpm: bad,
                ..YebanProjectV1::default()
            };
            assert!(
                project.validate().is_err(),
                "bpm {bad} 必须被拒绝, 实际 {:?}",
                project.validate()
            );
        }
        for good in [MIN_BPM, 120.0, MAX_BPM] {
            let project = YebanProjectV1 {
                bpm: good,
                ..YebanProjectV1::default()
            };
            assert_eq!(project.validate(), Ok(()), "bpm {good} 必须被接受");
        }
    }

    #[test]
    fn time_signature_constraints_are_enforced() {
        let bad_numerator = YebanProjectV1 {
            time_signature: TimeSignature {
                numerator: 0,
                denominator: 4,
            },
            ..YebanProjectV1::default()
        };
        assert_eq!(
            bad_numerator.validate(),
            Err(ModelError::TimeSignatureNumeratorOutOfRange { value: 0 })
        );

        let bad_numerator_high = YebanProjectV1 {
            time_signature: TimeSignature {
                numerator: 33,
                denominator: 4,
            },
            ..YebanProjectV1::default()
        };
        assert_eq!(
            bad_numerator_high.validate(),
            Err(ModelError::TimeSignatureNumeratorOutOfRange { value: 33 })
        );

        let bad_denominator = YebanProjectV1 {
            time_signature: TimeSignature {
                numerator: 7,
                denominator: 7,
            },
            ..YebanProjectV1::default()
        };
        assert_eq!(
            bad_denominator.validate(),
            Err(ModelError::TimeSignatureDenominatorUnsupported { value: 7 })
        );

        for denominator in [1_u8, 2, 4, 8, 16, 32] {
            let ok = TimeSignature {
                numerator: 6,
                denominator,
            };
            assert_eq!(ok.validate(), Ok(()), "分母 {denominator} 应合法");
        }
    }

    /// 拍号分子是**闭区间** `1..=32`：两个端点都必须放行。
    ///
    /// 实测：把 `!(1..=32).contains(&self.numerator)` 改成 `!(1..32)` 时，全仓判据
    /// 保持全绿 —— 既有判据钉住了 0 与 33 这两个**区间外**的点，却没有钉住区间内
    /// 的上端点 32。
    #[test]
    fn the_time_signature_numerator_upper_bound_is_inclusive() {
        for numerator in [1_u8, 32] {
            assert_eq!(
                TimeSignature {
                    numerator,
                    denominator: 4,
                }
                .validate(),
                Ok(()),
                "分子 {numerator} 是闭区间的端点, 必须合法"
            );
        }
    }

    #[test]
    fn sample_rate_and_block_size_enums_reject_unknown_values() {
        assert_eq!(SampleRate::from_hz(48_000), Ok(SampleRate::Hz48000));
        assert_eq!(
            SampleRate::from_hz(44_101),
            Err(ModelError::SampleRateUnsupported { value: 44_101 })
        );
        assert!(SampleRate::from_hz(0).is_err());
        assert_eq!(BlockSize::from_frames(256), Ok(BlockSize::Frames256));
        assert_eq!(
            BlockSize::from_frames(200),
            Err(ModelError::BlockSizeUnsupported { value: 200 })
        );
        assert!(BlockSize::from_frames(0).is_err());
    }

    #[test]
    fn numeric_enums_serialize_as_integers() {
        let json = serde_json::to_string(&SampleRate::Hz44100).expect("serialize");
        assert_eq!(json, "44100");
        let back: SampleRate = serde_json::from_str("192000").expect("deserialize");
        assert_eq!(back, SampleRate::Hz192000);
        assert!(serde_json::from_str::<SampleRate>("12345").is_err());

        let json = serde_json::to_string(&BlockSize::Frames64).expect("serialize");
        assert_eq!(json, "64");
        assert!(serde_json::from_str::<BlockSize>("65").is_err());
    }

    #[test]
    fn string_enums_match_schema_literals() {
        assert_eq!(
            serde_json::to_string(&BitDepth::Int24).expect("serde"),
            "\"Int24\""
        );
        assert_eq!(
            serde_json::to_string(&PanLaw::ConstantPowerMinus4_5dB).expect("serde"),
            "\"ConstantPowerMinus4_5dB\""
        );
        assert_eq!(
            serde_json::to_string(&TrackKind::AuxReturn).expect("serde"),
            "\"AuxReturn\""
        );
        assert_eq!(
            serde_json::to_string(&PanLaw::ConstantPowerMinus6dB).expect("serde"),
            "\"ConstantPowerMinus6dB\""
        );
        assert_eq!(
            serde_json::to_string(&BitDepth::Float32).expect("serde"),
            "\"Float32\""
        );
    }

    #[test]
    fn top_level_has_no_redundant_sample_rate() {
        let value = serde_json::to_value(YebanProjectV1::default()).expect("to_value");
        let map = value.as_object().expect("object");
        assert!(
            !map.contains_key("sample_rate"),
            "顶层不得有冗余 sample_rate (唯一来源是 audio_config)"
        );
        assert!(
            !map.contains_key("pan_law"),
            "顶层不得有第二份 pan_law (唯一来源是 audio_config)"
        );
        assert_eq!(
            map.get("audio_config")
                .and_then(|audio| audio.get("sample_rate"))
                .and_then(serde_json::Value::as_u64),
            Some(48_000),
            "audio_config.sample_rate 必须是整数 48000"
        );
    }

    #[test]
    fn required_top_level_keys_are_all_serialized() {
        let value = serde_json::to_value(YebanProjectV1::default()).expect("to_value");
        let map = value.as_object().expect("object");
        for key in [
            "schema_version",
            "min_reader_version",
            "writer_version",
            "id",
            "title",
            "bpm",
            "time_signature",
            "audio_config",
            "tracks",
            "clip_pool",
            "routing_graph",
        ] {
            assert!(map.contains_key(key), "schema required key `{key}` 缺失");
        }
        let ulid = map
            .get("id")
            .and_then(serde_json::Value::as_str)
            .expect("id 是字符串");
        assert_eq!(ulid.len(), 26, "id 必须是 26 字符 ULID");
    }

    #[test]
    fn tracks_and_clip_pool_are_json_objects_and_edges_are_arrays() {
        let value = serde_json::to_value(YebanProjectV1::default()).expect("to_value");
        assert!(value["tracks"].is_object(), "tracks 必须是 JSON 对象");
        assert!(value["clip_pool"].is_object(), "clip_pool 必须是 JSON 对象");
        assert!(
            value["routing_graph"]["nodes"].is_array(),
            "routing_graph.nodes 必须是数组"
        );
        assert!(
            value["routing_graph"]["edges"].is_array(),
            "routing_graph.edges 必须是数组 (schemas/project.schema.json)"
        );
    }

    #[test]
    fn routing_edges_serialize_in_key_order_and_round_trip() {
        let node_a = fixture_id(1);
        let node_b = fixture_id(2);
        let mut edges = BTreeMap::new();
        // 逆序插入：序列化必须仍然按键升序（确定性导出）。
        for id in [fixture_id(30), fixture_id(20), fixture_id(10)] {
            edges.insert(
                id,
                RoutingEdge {
                    id,
                    source_node: node_a,
                    destination_node: node_b,
                    kind: RoutingKind::TrackToBus,
                    gain_db: Some(-3.0),
                },
            );
        }
        let graph = RoutingGraph {
            nodes: vec![node_a, node_b],
            edges,
        };
        assert_eq!(graph.validate(), Ok(()));
        let json = serde_json::to_string(&graph).expect("serialize");
        let array_start = json.find("\"edges\":[").expect("edges 是数组");
        let expected_first = fixture_id(10).to_canonical_string();
        let expected_last = fixture_id(30).to_canonical_string();
        let first_pos = json.find(&expected_first).expect("包含最小键");
        let last_pos = json.find(&expected_last).expect("包含最大键");
        assert!(array_start < first_pos && first_pos < last_pos, "{json}");
        let back: RoutingGraph = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, graph);
    }

    #[test]
    fn routing_graph_rejects_dangling_endpoints_and_duplicate_nodes() {
        let node_a = fixture_id(1);
        let ghost = fixture_id(99);
        let edge_id = fixture_id(10);
        let graph = RoutingGraph {
            nodes: vec![node_a],
            edges: BTreeMap::from([(
                edge_id,
                RoutingEdge {
                    id: edge_id,
                    source_node: node_a,
                    destination_node: ghost,
                    kind: RoutingKind::BusToMaster,
                    gain_db: None,
                },
            )]),
        };
        assert_eq!(
            graph.validate(),
            Err(ModelError::RoutingNodeNotFound { id: ghost })
        );

        let duplicated = RoutingGraph {
            nodes: vec![node_a, node_a],
            edges: BTreeMap::new(),
        };
        assert_eq!(
            duplicated.validate(),
            Err(ModelError::DuplicateEntityId { id: node_a })
        );
    }

    /// 主总线必须出现在 `routing_graph.nodes` 里 —— 两个方向各一条。
    ///
    /// 与 `routing_graph_rejects_dangling_endpoints_and_duplicate_nodes` 的分工：
    /// 那一条问的是**单张图内部**自洽（边的两端必须在 `nodes` 里）；本判据问的是
    /// **跨集合**自洽（唯一声学出口必须够得着）。图自己的 `validate()` 结构上问不出
    /// 后者 —— 它看不到 `tracks` 与 `master_bus_track_id`。
    ///
    /// 下游证据：`crates/yeban-engine/src/graph.rs` 的 `PdcPlan::compute` 以"主总线在
    /// `nodes` 里"为前置条件，否则 `PdcError::UnknownMaster`。
    #[test]
    fn master_bus_track_must_be_reachable_in_the_routing_graph() {
        let master_id = fixture_id(3);
        let mut project = YebanProjectV1 {
            tracks: BTreeMap::from([(master_id, master_track(master_id))]),
            master_bus_track_id: master_id,
            ..YebanProjectV1::default()
        };
        // 没有 `nodes` 时引擎拒绝渲染；模型必须与它同口径。
        assert_eq!(
            project.validate(),
            Err(ModelError::RoutingNodeNotFound { id: master_id }),
            "主总线不在 routing_graph.nodes 里必须被拒绝 (下游 PdcPlan::compute 会报 UnknownMaster)"
        );

        project.routing_graph.nodes = vec![master_id];
        assert_eq!(
            project.validate(),
            Ok(()),
            "把主总线放进 nodes 之后必须合法"
        );

        // 可经操作日志达成的形态：`RemoveTrack` 不会回收节点身份 —— 被移走的那条
        // 音轨在 `nodes` 里留下的是一条悬空身份（本判据只钉主总线这一条，悬空
        // 非主总线节点的口径未在本条裁决）。
        let lead_id = fixture_id(4);
        project
            .insert_track(midi_track(lead_id))
            .expect("插入一条 MIDI 轨");
        project.routing_graph.nodes = vec![master_id, lead_id];
        assert_eq!(project.validate(), Ok(()));
        let removed = project.remove_track(&lead_id).expect("被插入的轨必须存在");
        assert_eq!(removed.id, lead_id);
        assert_eq!(
            project.routing_graph.nodes,
            vec![master_id, lead_id],
            "RemoveTrack 不动 routing_graph.nodes"
        );
    }

    /// 空工程一侧：任何节点身份都不可能对应存在的音轨 ⇒ 必须为空。
    #[test]
    fn an_empty_project_may_not_declare_routing_nodes() {
        let ghost = fixture_id(77);
        let mut project = YebanProjectV1::default();
        assert_eq!(project.validate(), Ok(()));
        project.routing_graph.nodes = vec![ghost];
        assert_eq!(
            project.validate(),
            Err(ModelError::RoutingNodeNotFound { id: ghost }),
            "0 轨工程声明节点身份必须被拒绝 (没有任何音轨能拥有它)"
        );
    }

    #[test]
    fn duplicate_routing_edge_in_json_is_rejected() {
        let node_a = fixture_id(1);
        let node_b = fixture_id(2);
        let edge_id = fixture_id(10);
        let edge = RoutingEdge {
            id: edge_id,
            source_node: node_a,
            destination_node: node_b,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        };
        let edge_json = serde_json::to_string(&edge).expect("serialize");
        let json = format!(
            "{{\"nodes\":[{:?},{:?}],\"edges\":[{edge_json},{edge_json}]}}",
            node_a.to_canonical_string(),
            node_b.to_canonical_string()
        );
        assert!(serde_json::from_str::<RoutingGraph>(&json).is_err());
    }

    #[test]
    fn track_pan_out_of_range_is_rejected() {
        let id = fixture_id(5);
        let track = TrackV3 {
            pan: 1.5,
            ..midi_track(id)
        };
        assert_eq!(
            track.validate(),
            Err(ModelError::PanOutOfRange { value: 1.5 })
        );
        let nan = TrackV3 {
            pan: f32::NAN,
            ..midi_track(id)
        };
        match nan.validate() {
            Err(ModelError::NonFiniteValue { field, value }) => {
                assert_eq!(field, "track.pan");
                assert!(value.is_nan(), "NaN 必须原样报出, 实际 {value}");
            }
            other => panic!("NaN 声相必须被拒绝, 实际 {other:?}"),
        }
        let edges = [-1.0_f32, -0.5, 0.0, 1.0];
        for pan in edges {
            let ok = TrackV3 {
                pan,
                ..midi_track(id)
            };
            assert_eq!(ok.validate(), Ok(()), "pan {pan} 合法");
        }
    }

    #[test]
    fn collection_key_must_match_embedded_id() {
        let key = fixture_id(1);
        let other = fixture_id(2);
        let master_id = fixture_id(3);
        let mut project = YebanProjectV1 {
            tracks: BTreeMap::from([
                (master_id, master_track(master_id)),
                (key, midi_track(other)),
            ]),
            master_bus_track_id: master_id,
            ..YebanProjectV1::default()
        };
        assert_eq!(
            project.validate(),
            Err(ModelError::EntityKeyMismatch {
                key,
                embedded: other,
            })
        );

        project.tracks = BTreeMap::from([(key, midi_track(key))]);
        project.master_bus_track_id = key;
        // 非 Master 类型不能当主总线
        assert_eq!(
            project.validate(),
            Err(ModelError::MasterBusKindMismatch { id: key })
        );
    }

    #[test]
    fn dangling_folder_and_clip_references_are_rejected() {
        let track_id = fixture_id(1);
        let master_id = fixture_id(2);
        let ghost = fixture_id(77);
        let mut project = YebanProjectV1 {
            tracks: BTreeMap::from([
                (master_id, master_track(master_id)),
                (
                    track_id,
                    TrackV3 {
                        folder_id: Some(ghost),
                        ..midi_track(track_id)
                    },
                ),
            ]),
            master_bus_track_id: master_id,
            ..YebanProjectV1::default()
        };
        assert_eq!(
            project.validate(),
            Err(ModelError::TrackNotFound { id: ghost })
        );

        project.tracks.get_mut(&track_id).expect("track").folder_id = None;
        let placement_id = fixture_id(50);
        project
            .tracks
            .get_mut(&track_id)
            .expect("track")
            .clips
            .insert(
                placement_id,
                ClipPlacement {
                    id: placement_id,
                    clip_id: ghost,
                    ..ClipPlacement::default()
                },
            );
        assert_eq!(
            project.validate(),
            Err(ModelError::ClipNotFound { id: ghost })
        );
    }

    #[test]
    fn note_insertion_and_removal_are_checked() {
        let track_id = fixture_id(1);
        let master_id = fixture_id(2);
        let clip_id = fixture_id(3);
        let note_id = fixture_id(4);
        let mut project = YebanProjectV1 {
            tracks: BTreeMap::from([
                (master_id, master_track(master_id)),
                (track_id, midi_track(track_id)),
            ]),
            master_bus_track_id: master_id,
            // 主总线必须在 `nodes` 里，否则 `validate()` 会按"够不着唯一声学出口"拒绝
            // （本条判据末尾就断言了 `validate() == Ok`）。
            routing_graph: RoutingGraph {
                nodes: vec![master_id],
                edges: BTreeMap::new(),
            },
            ..YebanProjectV1::default()
        };
        project
            .insert_clip(midi_clip(clip_id))
            .expect("insert clip");

        let note = MidiNote::new(note_id, 0, 60, 240);
        project.insert_note(&clip_id, note.clone()).expect("insert");
        assert_eq!(project.note(&clip_id, &note_id).expect("get"), &note);
        assert_eq!(
            project.insert_note(&clip_id, note.clone()),
            Err(ModelError::DuplicateEntityId { id: note_id })
        );

        let illegal = MidiNote {
            velocity: 200,
            ..MidiNote::new(fixture_id(9), 0, 60, 240)
        };
        assert_eq!(
            project.insert_note(&clip_id, illegal),
            Err(ModelError::VelocityOutOfRange { value: 200 })
        );

        let removed = project.remove_note(&clip_id, &note_id).expect("remove");
        assert_eq!(removed, note);
        assert_eq!(
            project.remove_note(&clip_id, &note_id),
            Err(ModelError::NoteNotFound { id: note_id })
        );
        assert_eq!(project.validate(), Ok(()));
    }

    /// 类别⑤（幂等性）：[`YebanProjectV1`] 的六个**增删入口**重复调用第二遍必须被拒绝，
    /// 且文档**逐字节不动**。
    ///
    /// 为什么需要这条：既有判据 `note_insertion_and_removal_are_checked` 只覆盖
    /// `insert_note` / `remove_note` 的错误码，**没有**断言"被拒之后文档没变"，
    /// 而 `insert_track` / `insert_clip` / `remove_track` / `remove_clip` 四条
    /// **连错误码都没有判据**。"先改一半再发现身份重复"正是这类入口最容易犯的错。
    /// 读数（改动前，在 `src/project.rs` 的测试区内取行号）：`TrackNotFound` 只有 1 处命中，
    /// 落在 `validate()` 判据 `dangling_folder_and_clip_references_are_rejected` 里；
    /// `ClipNotFound` 同理 1 处；`DuplicateEntityId` 2 处，分别在路由节点与音符上 ——
    /// 四条增删入口一个都没有。
    ///
    /// 单位：六个入口各一对调用，比较的是 `serde_json::to_vec` 的字节数组。
    #[test]
    fn repeated_insert_and_remove_are_rejected_without_touching_the_document() {
        /// 先记下"第一遍之前"的字节，再断言"第一遍真的改了"，
        /// 最后断言第二遍返回**指定错误码**且字节回到第一遍之后的那一串。
        fn assert_repeat_rejected(
            label: &str,
            mut project: YebanProjectV1,
            expected: ModelError,
            apply: impl Fn(&mut YebanProjectV1) -> Result<(), ModelError>,
        ) {
            let before = serde_json::to_vec(&project).expect("序列化");
            apply(&mut project).unwrap_or_else(|error| panic!("{label} 第一遍必须成功: {error}"));
            let after_first = serde_json::to_vec(&project).expect("序列化");
            assert_ne!(before, after_first, "{label} 第一遍必须真的改变文档");
            assert_eq!(
                apply(&mut project),
                Err(expected),
                "{label} 第二遍必须被拒绝"
            );
            assert_eq!(
                serde_json::to_vec(&project).expect("序列化"),
                after_first,
                "{label} 第二遍被拒之后文档必须逐字节不动"
            );
            project
                .validate()
                .unwrap_or_else(|error| panic!("{label} 两次调用之后文档必须仍合法: {error}"));
        }

        let track_id = fixture_id(1);
        let master_id = fixture_id(2);
        let clip_id = fixture_id(3);
        let note_id = fixture_id(4);

        // 夹具：一条主总线 + 一个 MIDI 片段（主总线必须在 `nodes` 里，否则 `validate()` 拒绝）。
        let fixture = || {
            let mut project = YebanProjectV1 {
                tracks: BTreeMap::from([(master_id, master_track(master_id))]),
                master_bus_track_id: master_id,
                routing_graph: RoutingGraph {
                    nodes: vec![master_id],
                    edges: BTreeMap::new(),
                },
                ..YebanProjectV1::default()
            };
            project
                .insert_clip(midi_clip(clip_id))
                .expect("夹具片段必须能插入");
            project
        };

        // ① 同一音轨插入两次。
        let track = midi_track(track_id);
        assert_repeat_rejected(
            "insert_track",
            fixture(),
            ModelError::DuplicateEntityId { id: track_id },
            |project| project.insert_track(track.clone()),
        );

        // ② 同一片段插入两次。
        let extra_clip = midi_clip(fixture_id(5));
        assert_repeat_rejected(
            "insert_clip",
            fixture(),
            ModelError::DuplicateEntityId { id: fixture_id(5) },
            |project| project.insert_clip(extra_clip.clone()),
        );

        // ③ 同一音轨移除两次（先建一条待移除的音轨）。
        let mut with_track = fixture();
        with_track
            .insert_track(midi_track(track_id))
            .expect("建待移除音轨");
        assert_repeat_rejected(
            "remove_track",
            with_track,
            ModelError::TrackNotFound { id: track_id },
            |project| project.remove_track(&track_id).map(|_removed| ()),
        );

        // ④ 同一片段移除两次。
        assert_repeat_rejected(
            "remove_clip",
            fixture(),
            ModelError::ClipNotFound { id: clip_id },
            |project| project.remove_clip(&clip_id).map(|_removed| ()),
        );

        // ⑤ 同一音符插入两次。
        let note = MidiNote::new(note_id, 0, 60, 240);
        assert_repeat_rejected(
            "insert_note",
            fixture(),
            ModelError::DuplicateEntityId { id: note_id },
            |project| project.insert_note(&clip_id, note.clone()),
        );

        // ⑥ 同一音符移除两次。
        let mut with_note = fixture();
        with_note
            .insert_note(&clip_id, MidiNote::new(note_id, 0, 60, 240))
            .expect("建待移除音符");
        assert_repeat_rejected(
            "remove_note",
            with_note,
            ModelError::NoteNotFound { id: note_id },
            |project| project.remove_note(&clip_id, &note_id).map(|_removed| ()),
        );
    }

    #[test]
    fn audio_clip_rejects_note_operations() {
        let clip_id = fixture_id(3);
        let mut project = YebanProjectV1::default();
        project
            .insert_clip(ClipPoolEntry {
                id: clip_id,
                name: "Kick".to_owned(),
                content: ClipContent::Audio {
                    asset: AssetHash::of_bytes(b"kick"),
                    gain_db: 0.0,
                },
            })
            .expect("insert");
        assert_eq!(
            project.insert_note(&clip_id, MidiNote::default()),
            Err(ModelError::ClipContentKindMismatch { id: clip_id })
        );
    }

    #[test]
    fn btreemap_iteration_and_serialization_are_order_independent() {
        let master_id = fixture_id(2);
        let ascending = [fixture_id(10), fixture_id(20), fixture_id(30)];
        let mut forward = YebanProjectV1 {
            tracks: BTreeMap::from([(master_id, master_track(master_id))]),
            master_bus_track_id: master_id,
            ..YebanProjectV1::default()
        };
        let mut backward = forward.clone();
        for id in ascending {
            forward.insert_track(midi_track(id)).expect("insert");
        }
        for id in ascending.into_iter().rev() {
            backward.insert_track(midi_track(id)).expect("insert");
        }
        assert_eq!(
            serde_json::to_string(&forward).expect("serde"),
            serde_json::to_string(&backward).expect("serde"),
            "插入顺序不得影响序列化字节 (BTreeMap 确定性)"
        );
    }

    #[test]
    fn project_serde_round_trip_preserves_everything() {
        let project = filled_project();
        assert_eq!(project.validate(), Ok(()), "{:?}", project.validate());
        let json = serde_json::to_string_pretty(&project).expect("serialize");
        let back: YebanProjectV1 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, project);
    }

    /// [ARCH-PDC-001, ADR-0001 D43] `bypassed` / `params` / `latency_samples` 现在**必需**。
    ///
    /// 本判据是旧判据（"缺 `latency_samples` 读成 0"）的**反向**：D43 之后
    /// 缺失必须响亮失败，而**显式** `0` 仍然是合法的"零延迟设备"上报值，必须能往返。
    #[test]
    fn device_latency_bypass_and_params_are_required_and_round_trip() {
        let full = r#"{
            "id": "00000000000000000000000000",
            "name": "Device",
            "kind": "InternalInstrument",
            "bypassed": false,
            "params": [],
            "latency_samples": 1024
        }"#;
        let device: DeviceDefinition = serde_json::from_str(full).expect("完整设备必须可读");
        assert_eq!(device.latency_samples, 1024);
        assert_eq!(device.validate(), Ok(()));

        // 三个字段各自缺失都必须被单独拒绝（不是"读成 0 / 空集"）。
        for (missing, partial) in [
            (
                "latency_samples",
                r#"{"id":"00000000000000000000000000","name":"D","kind":"InternalInstrument","bypassed":false,"params":[]}"#,
            ),
            (
                "bypassed",
                r#"{"id":"00000000000000000000000000","name":"D","kind":"InternalInstrument","params":[],"latency_samples":0}"#,
            ),
            (
                "params",
                r#"{"id":"00000000000000000000000000","name":"D","kind":"InternalInstrument","bypassed":false,"latency_samples":0}"#,
            ),
        ] {
            let error =
                serde_json::from_str::<DeviceDefinition>(partial).expect_err("缺必需字段必须失败");
            let expected = format!("missing field `{missing}`");
            assert!(
                error.to_string().contains(&expected),
                "错误必须点名 {expected}, 实测: {error}"
            );
        }

        // **显式** 0 是合法上报值（零延迟设备），必需性不等于"必须非零"。
        let mut zero = device.clone();
        zero.latency_samples = 0;
        let json = serde_json::to_string(&zero).expect("serialize");
        assert!(
            json.contains("\"latency_samples\":0"),
            "显式 0 必须真的被序列化出来: {json}"
        );
        let back: DeviceDefinition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, zero);

        // 填充样本必须覆盖非零情形, 否则"字段可用"这件事没有样本证据
        let filled = crate::samples::filled_project();
        let any_nonzero = filled
            .tracks
            .values()
            .flat_map(|track| track.devices.iter())
            .any(|d| d.latency_samples > 0);
        assert!(any_nonzero, "填充样本里至少要有一个非零延迟设备");
    }

    #[test]
    fn device_param_and_macro_ranges_are_checked() {
        let device = DeviceDefinition {
            params: vec![ParameterValue {
                name: "cutoff".to_owned(),
                value: f32::INFINITY,
                unit: Some("Hz".to_owned()),
            }],
            ..DeviceDefinition::default()
        };
        assert!(matches!(
            device.validate(),
            Err(ModelError::NonFiniteValue { .. })
        ));

        let macro_parameter = MacroParameter {
            value: 1.5,
            ..MacroParameter::default()
        };
        assert_eq!(
            macro_parameter.validate(),
            Err(ModelError::MacroValueOutOfRange { value: 1.5 })
        );

        let mapping = MacroMapping {
            target: AutomationTarget::TrackVolume {
                track_id: fixture_id(1),
            },
            depth: f32::NAN,
        };
        assert!(matches!(
            mapping.validate(),
            Err(ModelError::NonFiniteValue { .. })
        ));
    }

    #[test]
    fn section_scene_and_loop_ranges_are_checked() {
        let section = SectionV3 {
            id: fixture_id(1),
            name: "Intro".to_owned(),
            start_tick: 960,
            end_tick: 960,
            color: None,
        };
        assert_eq!(section.validate(), Err(ModelError::ZeroDuration));

        let scene = SceneV3 {
            id: fixture_id(2),
            name: "Verse".to_owned(),
            tempo: Some(1000.0),
            color: None,
        };
        assert_eq!(
            scene.validate(),
            Err(ModelError::BpmOutOfRange { value: 1000.0 })
        );

        let loop_config = LoopConfig {
            enabled: true,
            start_tick: 480,
            end_tick: 480,
        };
        assert_eq!(loop_config.validate(), Err(ModelError::ZeroDuration));
        let disabled = LoopConfig {
            enabled: false,
            start_tick: 480,
            end_tick: 0,
        };
        assert_eq!(disabled.validate(), Ok(()));
    }

    /// 读取 `schemas/project.schema.json` 里每个顶层键声明的 JSON 类型。
    ///
    /// 这里**故意不手抄**契约：手抄出来的第二份事实源会在契约改动后变成谎言
    /// （第一版就是手抄表 + 断言"分歧恰好是 writer_version 一项"，而契约修好之后
    /// 那张表仍在宣称契约写的是 `integer`）。现在这份测试直接读真实契约文件，
    /// 于是"实现与契约的类型分歧"这件事只有一个事实源。
    fn schema_top_level_types() -> std::collections::BTreeMap<String, String> {
        let path = schema_path("project.schema.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("读取 {} 失败: {err}", path.display()));
        let schema: serde_json::Value =
            serde_json::from_str(&text).expect("schema 必须是合法 JSON");
        let mut out = std::collections::BTreeMap::new();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema 必须有 properties 对象");
        for (key, spec) in properties {
            // 只比较**直接声明 type** 的键；用 oneOf/anyOf 表达的键交给真正的
            // jsonschema 对账 (scripts/gates/validate_schemas.py) 去管。
            if let Some(type_name) = spec.get("type").and_then(serde_json::Value::as_str) {
                out.insert(key.clone(), type_name.to_owned());
            }
        }
        out
    }

    /// `schemas/` 目录：`<repo>/schemas/<name>`。
    fn schema_path(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("schemas")
            .join(name)
    }

    /// `serde_json::Value` 的 JSON Schema 类型名。
    fn json_type(value: &serde_json::Value) -> &'static str {
        match value {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "boolean",
            serde_json::Value::Number(number) => {
                if number.is_i64() || number.is_u64() {
                    "integer"
                } else {
                    "number"
                }
            }
            serde_json::Value::String(_) => "string",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "object",
        }
    }

    /// 实现的序列化结果与 `schemas/project.schema.json` 的顶层类型**必须完全一致**。
    ///
    /// 历史：这条判据最初手抄了一张契约类型表，并断言"分歧恰好只有 `writer_version` 一项"
    /// （当时契约写 `integer`，而 ADR-0001 D3 裁定它是语义版本字符串）。
    /// 契约已按 D11 收紧为 `string`，于是那项"已留痕的分歧"**不再存在**，
    /// 本判据随之改为直接读契约文件并断言**零分歧** —— 契约再漂移就立刻变红。
    #[test]
    fn implementation_matches_the_schema_type_table_exactly() {
        let value = serde_json::to_value(YebanProjectV1::default()).expect("to_value");
        let mut divergences: Vec<String> = Vec::new();
        let table = schema_top_level_types();
        assert!(
            table.contains_key("writer_version"),
            "契约必须声明 writer_version 的类型"
        );
        for (key, expected) in &table {
            let Some(actual_value) = value.get(key) else {
                divergences.push(format!("{key}: 契约要求但实现未序列化"));
                continue;
            };
            let actual = json_type(actual_value);
            if actual != expected {
                divergences.push(format!("{key}: schema={expected}, actual={actual}"));
            }
        }
        assert_eq!(
            divergences,
            Vec::<String>::new(),
            "实现与 schemas/project.schema.json 的顶层类型出现分歧；\
             要么改实现，要么改契约并在 docs/adr/ADR-0001 留痕（不许两边各说各话）"
        );
    }

    /// 样本导出：走 `crate::samples::export_all`（与
    /// `examples/export_schema_samples.rs` **同一个**入口），命名遵循
    /// `scripts/gates/validate_schemas.py` 的 `project.<name>.json` / `ops.<name>.json` 前缀约定。
    #[test]
    fn export_schema_samples_to_target() {
        let dir = crate::samples::default_out_dir();
        let written = crate::samples::export_all(&dir).expect("导出规范样本");
        assert_eq!(written.len(), 4, "必须导出 4 份样本: {written:?}");
        for path in &written {
            assert!(path.is_file(), "{} 必须存在", path.display());
        }

        // 两次导出必须逐字节相同（样本是跨语言对账口径，不能漂移）。
        let before = read_all(&written);
        let again = crate::samples::export_all(&dir).expect("再次导出规范样本");
        assert_eq!(before, read_all(&again), "样本导出必须逐字节稳定");
    }

    /// 读出一批样本文件的内容（顺序与传入一致）。
    fn read_all(paths: &[std::path::PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|path| std::fs::read_to_string(path).expect("read sample"))
            .collect()
    }

    // -----------------------------------------------------------------------
    // 类别 1（非有限输入）与类别 4（参数极值）：机械枚举 + 逐项判定
    //
    // 手册口径：「先用 grep 机械列出同类全部候选，再汇成一张表逐项判定」。
    // Rust 没有反射，因此清单直接从源码文本推出来（`struct`／`enum` 体，
    // 枚举载荷字段、`Option<f32>`、`Vec<f32>` 这类容器类型与数值类型别名都看得见），
    // 再与冻结表**双向**比较。
    // -----------------------------------------------------------------------

    use crate::ops::Op;

    /// 三个非有限 `f32`：逐项判定必须对它们**逐一**成立。
    const NON_FINITE_F32: [f32; 3] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

    /// 三个非有限 `f64`。
    const NON_FINITE_F64: [f64; 3] = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY];

    /// 机械清单（本 crate 全部数值字段声明的唯一事实源）。
    ///
    /// 键的形态 `<crate 相对路径>::<拥有者>::<成员>`；枚举载荷字段写作
    /// `<变体>.<字段>`（一个枚举里可以有两个同名字段，例如 `Op::SetParam.old_val`
    /// 与 `Op::SetMacro.old_val`）。
    ///
    /// 量什么：`crates/yeban-model/src/**/*.rs` 里**承载数值**的字段声明条数 ——
    /// 类型原文里出现 `f32`／`f64` 标识符（含 `Option<f32>`／`Vec<f32>`／`[f64; 4]`），
    /// 或类型是数值类型别名（`type X = f32;`）。单位：条（实测 27）。
    const NUMERIC_FIELD_INVENTORY: &[&str] = &[
        // `ModelError` 的 `value` 是**输出载体**（错误里携带的被拒值），不是入口：
        // 它不进文档、不需要判据，故在 `NUMERIC_FIELD_POLICY` 里显式豁免。
        "src/error.rs::ModelError::ProbabilityOutOfRange.value",
        "src/error.rs::ModelError::BpmOutOfRange.value",
        "src/error.rs::ModelError::NonFiniteValue.value",
        "src/error.rs::ModelError::PanOutOfRange.value",
        "src/error.rs::ModelError::MacroValueOutOfRange.value",
        "src/error.rs::ModelError::MacroDepthOutOfRange.value",
        // 持久化实体与 Op 载荷
        "src/music.rs::MidiNote::probability",
        "src/ops.rs::Op::SetRoutingGain.old_gain_db",
        "src/ops.rs::Op::SetRoutingGain.new_gain_db",
        "src/ops.rs::Op::SetParam.old_val",
        "src/ops.rs::Op::SetParam.new_val",
        "src/ops.rs::Op::SetMacro.old_val",
        "src/ops.rs::Op::SetMacro.new_val",
        "src/project.rs::ParameterValue::value",
        "src/project.rs::MacroMapping::depth",
        "src/project.rs::MacroParameter::value",
        "src/project.rs::AutomationPoint::value",
        "src/project.rs::AutomationValueDomain::min",
        "src/project.rs::AutomationValueDomain::max",
        // `Repr` 是 `AutomationValueDomain::deserialize` 的私有中间体，两端点
        // 只能经 `AutomationValueDomain::new` 进入（同一个有限性判据）。
        "src/project.rs::Repr::min",
        "src/project.rs::Repr::max",
        "src/project.rs::ClipContent::Audio.gain_db",
        "src/project.rs::TrackV3::volume_db",
        "src/project.rs::TrackV3::pan",
        "src/project.rs::SceneV3::tempo",
        "src/project.rs::RoutingEdge::gain_db",
        "src/project.rs::YebanProjectV1::bpm",
    ];

    /// 一个数值字段的判定探针：`Err(理由)` 表示判定不成立。
    type NumericProbe = fn() -> Result<(), String>;

    /// **逐项判定表**：清单里除 `ModelError` 输出载体之外的每一个数值字段一行。
    ///
    /// 行 = `(清单键, 判定结论, 探针)`。键集合必须**恰好**等于
    /// `NUMERIC_FIELD_INVENTORY` 减去那 6 个输出载体（多一行、少一行都红）。
    const NUMERIC_FIELD_POLICY: &[(&str, &str, NumericProbe)] = &[
        (
            "src/music.rs::MidiNote::probability",
            "非有限拒绝（ProbabilityOutOfRange）；0.0..=1.0 外拒绝；None=必然触发",
            probe_midi_note_probability,
        ),
        (
            "src/ops.rs::Op::SetRoutingGain.old_gain_db",
            "旧值只做逐位比较：NaN 不可能与文档里的有限值逐位相等 ⇒ OpStateMismatch",
            probe_set_routing_gain_old,
        ),
        (
            "src/ops.rs::Op::SetRoutingGain.new_gain_db",
            "非有限拒绝（routing.edge.gain_db）；有限极值接受（dB 无模型级范围）",
            probe_set_routing_gain_new,
        ),
        (
            "src/ops.rs::Op::SetParam.old_val",
            "旧值只做逐位比较：NaN ⇒ OpStateMismatch",
            probe_set_param_old,
        ),
        (
            "src/ops.rs::Op::SetParam.new_val",
            "非有限拒绝（param.value）；声相 -1..=1、宏 0..=1 外拒绝；音量/设备参数无范围",
            probe_set_param_new,
        ),
        (
            "src/ops.rs::Op::SetMacro.old_val",
            "旧值只做逐位比较：NaN ⇒ OpStateMismatch",
            probe_set_macro_old,
        ),
        (
            "src/ops.rs::Op::SetMacro.new_val",
            "非有限与越界都报 MacroValueOutOfRange；0.0/1.0 接受",
            probe_set_macro_new,
        ),
        (
            "src/project.rs::ParameterValue::value",
            "非有限拒绝（device.param.value）；有限极值接受（取值域模型不可知）",
            probe_parameter_value,
        ),
        (
            "src/project.rs::MacroMapping::depth",
            "非有限拒绝（macro.mapping.depth）；0.0..=1.0 外报 MacroDepthOutOfRange",
            probe_macro_mapping_depth,
        ),
        (
            "src/project.rs::MacroParameter::value",
            "非有限与越界都报 MacroValueOutOfRange；0.0/1.0 接受",
            probe_macro_parameter_value,
        ),
        (
            "src/project.rs::AutomationPoint::value",
            "非有限拒绝（automation.point.value）；有限极值接受（求值端保证有限输出）",
            probe_automation_point_value,
        ),
        (
            "src/project.rs::AutomationValueDomain::min",
            "非有限拒绝（automation.lane.domain.min）；端点按定义排序",
            probe_domain_min,
        ),
        (
            "src/project.rs::AutomationValueDomain::max",
            "非有限拒绝（automation.lane.domain.max）",
            probe_domain_max,
        ),
        (
            "src/project.rs::Repr::min",
            "私有反序列化中间体：只能经 new() 进入 ⇒ 与 AutomationValueDomain::min 同一判据",
            probe_domain_min,
        ),
        (
            "src/project.rs::Repr::max",
            "私有反序列化中间体：只能经 new() 进入 ⇒ 与 AutomationValueDomain::max 同一判据",
            probe_domain_max,
        ),
        (
            "src/project.rs::ClipContent::Audio.gain_db",
            "非有限拒绝（clip.audio.gain_db）；有限极值接受；入口（AddClip/insert_clip）已校验",
            probe_clip_audio_gain,
        ),
        (
            "src/project.rs::TrackV3::volume_db",
            "非有限拒绝（track.volume_db）；dB 无模型级范围 ⇒ 有限极值接受",
            probe_track_volume_db,
        ),
        (
            "src/project.rs::TrackV3::pan",
            "非有限拒绝（track.pan）；-1.0..=1.0 外报 PanOutOfRange",
            probe_track_pan,
        ),
        (
            "src/project.rs::SceneV3::tempo",
            "非有限拒绝（scene.tempo）；20.0..=999.0 外报 BpmOutOfRange；None=跟随工程",
            probe_scene_tempo,
        ),
        (
            "src/project.rs::RoutingEdge::gain_db",
            "非有限拒绝（routing.edge.gain_db）；None=单位增益；有限极值接受",
            probe_routing_edge_gain,
        ),
        (
            "src/project.rs::YebanProjectV1::bpm",
            "非有限拒绝（bpm）；20.0..=999.0 外报 BpmOutOfRange",
            probe_project_bpm,
        ),
    ];

    /// `crates/yeban-model/src/**` 下（递归、按路径排序）的全部 `*.rs` 文件。
    fn source_files() -> Vec<std::path::PathBuf> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
                .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", dir.display()))
                .map(|entry| entry.expect("目录项").path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    out.push(path);
                }
            }
        }
        let mut out = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut out,
        );
        out
    }

    /// 剥掉字段声明前的可见性前缀（`pub` / `pub(crate)` / `pub(super)` / `pub(in …)`）。
    ///
    /// 字段的**数值性**与它的可见性无关，因此扫描器不得因为可见性写法不同而漏掉字段
    /// （实测：`pub(super) gain_db: f32` 对改动前的清单判据完全不可见）。
    fn strip_visibility(fragment: &str) -> &str {
        let Some(rest) = fragment.strip_prefix("pub") else {
            return fragment;
        };
        if let Some(rest) = rest.strip_prefix('(') {
            return rest
                .find(')')
                .map_or(fragment, |close| rest[close + 1..].trim_start());
        }
        if rest.starts_with(char::is_whitespace) {
            return rest.trim_start();
        }
        // 例如字段名 `public`：`pub` 只是它的前缀，不是可见性关键字。
        fragment
    }

    /// 从一个 `name: type` 片段解析出 `(字段名, 类型原文)`。
    fn field_name_and_type(fragment: &str) -> Option<(String, String)> {
        let fragment = fragment.trim().trim_end_matches(',').trim();
        // 属性与可见性都只是前缀：`#[serde(default)] pub(super) x: f32` 也是数值字段。
        let fragment = fragment
            .strip_prefix("#[")
            .and_then(|rest| rest.find(']').map(|close| &rest[close + 1..]))
            .map_or(fragment, str::trim_start);
        let fragment = strip_visibility(fragment);
        let (name, ty) = fragment.split_once(':')?;
        let name = name.trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return None;
        }
        Some((name.to_owned(), ty.trim().to_owned()))
    }

    /// 类型原文里是否出现 `f32`／`f64` 这两个**独立的标识符**。
    ///
    /// 按 token 而不是逐字比较：`Vec<f32>`、`[f64; 4]`、`Option<Vec<f32>>` 都是
    /// 承载数值的字段声明。实测（收紧前）：把 `probe_curve: Vec<f32>` 加进 `src/**` 时
    /// `every_numeric_field_in_the_model_is_inventoried` 保持全绿 —— 数值字段可以藏进容器类型。
    fn is_numeric_type(ty: &str) -> bool {
        ty.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|token| token == "f32" || token == "f64")
    }

    /// 数值字段名；`aliases` 里的类型别名也算数值（`type X = f32;` 之后的 `X`）。
    ///
    /// 实测（收紧前）：`type ProbeAlias = f32;` 加 `pub gain: ProbeAlias` 对清单判据
    /// 完全不可见 —— 别名是绕过"新数值字段必被登记"的第二条常见路径。
    fn numeric_field_with(
        fragment: &str,
        aliases: &std::collections::BTreeSet<String>,
    ) -> Option<String> {
        let (name, ty) = field_name_and_type(fragment)?;
        let numeric = is_numeric_type(&ty)
            || ty
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(|token| aliases.contains(token));
        numeric.then_some(name)
    }

    /// 识别 `type NAME = <类型>;` 声明，返回 `(别名, 别名是否数值)`。
    fn type_alias(line: &str) -> Option<(String, bool)> {
        let rest = line
            .strip_prefix("pub(crate) ")
            .or_else(|| line.strip_prefix("pub "))
            .unwrap_or(line);
        let rest = rest.strip_prefix("type ")?;
        let (name, rhs) = rest.split_once('=')?;
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        Some((name.to_owned(), is_numeric_type(rhs)))
    }

    /// 一行声明里的**全部**数值字段名。
    ///
    /// 三种形态必须都看得见（改动前它们都是盲区：把这样一个字段加进 `src/**`
    /// 不会让任何判据变红）：
    ///
    /// - 一行多个载荷字段：`One { gain_db: f32, pan: f32 },`；
    /// - 写在一行里的载荷：`enum P { One { gain_db: f32 } }`；
    /// - 写在一行里的声明：`struct S { pub gain_db: f32 }`。
    ///
    /// 做法：先剥掉行尾注释，再取出每一对花括号里的内容（嵌套时外层与内层都取出，
    /// 于是 `enum P { One { gain_db: f32 } }` 的内层能被看到），按 `,` 拆开逐段判定；
    /// 一行里一个花括号都没有时（普通的 `struct` 字段行）整行就是一段。
    fn numeric_fields_on_line(line: &str) -> Vec<String> {
        numeric_fields_on_line_with(line, &std::collections::BTreeSet::new())
    }

    /// 同上，但把数值类型别名也算作数值（`scan_numeric_fields` 用它）。
    fn numeric_fields_on_line_with(
        line: &str,
        aliases: &std::collections::BTreeSet<String>,
    ) -> Vec<String> {
        let line = line.split("//").next().unwrap_or(line);
        let mut groups: Vec<&str> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        for (index, ch) in line.char_indices() {
            match ch {
                '{' => starts.push(index + 1),
                '}' => {
                    if let Some(start) = starts.pop() {
                        groups.push(&line[start..index]);
                    }
                }
                _ => {}
            }
        }
        if groups.is_empty() {
            groups.push(line);
        }
        let mut found = Vec::new();
        for group in groups {
            for piece in group.split(',') {
                if let Some(field) = numeric_field_with(piece, aliases) {
                    found.push(field);
                }
            }
        }
        found
    }

    /// 声明行上第一个载荷变体的名字（`enum P { One { … } }` ⇒ `One`）。
    fn variant_on_declaration_line(line: &str) -> String {
        let body = line.find('{').map_or("", |open| &line[open + 1..]);
        body.trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect()
    }

    /// 识别 `struct`／`enum` 声明行（必须同一行开 `{`），返回 `(是否枚举, 类型名)`。
    fn type_declaration(line: &str) -> Option<(bool, String)> {
        let rest = line
            .strip_prefix("pub(crate) ")
            .or_else(|| line.strip_prefix("pub "))
            .unwrap_or(line);
        let (is_enum, rest) = if let Some(rest) = rest.strip_prefix("enum ") {
            (true, rest)
        } else {
            (false, rest.strip_prefix("struct ")?)
        };
        if !rest.contains('{') {
            return None;
        }
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some((is_enum, name))
    }

    /// 扫描源码，抽出全部数值字段声明。
    ///
    /// 两遍：先收集数值类型别名（`type X = f32;` ⇒ 字段类型写 `X` 也算数值），
    /// 再抽字段。别名先于字段是因为 `src/**` 的读序与声明序无关。
    fn scan_numeric_fields() -> Vec<String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let sources: Vec<(String, String)> = source_files()
            .into_iter()
            .map(|path| {
                let relative = path
                    .strip_prefix(root)
                    .expect("src 下的文件必在 crate 根之下")
                    .to_string_lossy()
                    .replace('\\', "/");
                (
                    relative,
                    std::fs::read_to_string(&path).expect("读取源文件"),
                )
            })
            .collect();

        // 第一遍：数值类型别名。
        let mut aliases: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (_, text) in &sources {
            for line in text.lines() {
                if let Some((name, true)) = type_alias(line.trim_start()) {
                    aliases.insert(name);
                }
            }
        }

        let mut found = Vec::new();
        for (relative, text) in &sources {
            let mut owner: Option<(bool, String, usize)> = None;
            let mut variant = String::new();
            for line in text.lines() {
                let trimmed = line.trim_start();
                let indent = line.len() - trimmed.len();
                if let Some((is_enum, name, decl_indent)) = owner.clone() {
                    // 声明体的终点：缩进不深于声明行、且以 `}` 开头的那一行。
                    if trimmed.starts_with('}') && indent <= decl_indent {
                        owner = None;
                        variant.clear();
                        continue;
                    }
                    if is_enum
                        && trimmed
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_uppercase())
                    {
                        let candidate: String = trimmed
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        if !candidate.is_empty() {
                            variant.clone_from(&candidate);
                        }
                    }
                    for field in numeric_fields_on_line_with(trimmed, &aliases) {
                        let member = if is_enum && !variant.is_empty() {
                            format!("{variant}.{field}")
                        } else {
                            field
                        };
                        found.push(format!("{relative}::{name}::{member}"));
                    }
                    continue;
                }
                if let Some((is_enum, name)) = type_declaration(trimmed) {
                    // 声明行**自己**也可能带载荷（`enum P { One { gain_db: f32 } }` 或
                    // `struct S { pub gain_db: f32 }` 一行写完），因此声明行同样要走一遍
                    // 字段抽取；否则这类字段对清单判据不可见（纪律 B：多行签名要看得见）。
                    let variant_here = if is_enum {
                        variant_on_declaration_line(trimmed)
                    } else {
                        String::new()
                    };
                    for field in numeric_fields_on_line_with(trimmed, &aliases) {
                        let member = if is_enum && !variant_here.is_empty() {
                            format!("{variant_here}.{field}")
                        } else {
                            field
                        };
                        found.push(format!("{relative}::{name}::{member}"));
                    }
                    owner = Some((is_enum, name, indent));
                    variant = variant_here;
                }
            }
        }
        found.sort();
        found.dedup();
        found
    }

    /// 断言：三个非有限 `f32` 都必须被 `NonFiniteValue` 拒绝，且 `field` 逐字等于 `expected`。
    fn rejects_non_finite_f32(
        expected: &'static str,
        mut check: impl FnMut(f32) -> Result<(), ModelError>,
    ) -> Result<(), String> {
        for value in NON_FINITE_F32 {
            match check(value) {
                Err(ref error) if field_of(error) == Some(expected) => {}
                other => {
                    return Err(format!(
                        "非有限 {value} 必须以 NonFiniteValue({expected}) 拒绝, 实际 {other:?}"
                    ));
                }
            }
        }
        Ok(())
    }

    /// 同上，`f64`。
    fn rejects_non_finite_f64(
        expected: &'static str,
        mut check: impl FnMut(f64) -> Result<(), ModelError>,
    ) -> Result<(), String> {
        for value in NON_FINITE_F64 {
            match check(value) {
                Err(ref error) if field_of(error) == Some(expected) => {}
                other => {
                    return Err(format!(
                        "非有限 {value} 必须以 NonFiniteValue({expected}) 拒绝, 实际 {other:?}"
                    ));
                }
            }
        }
        Ok(())
    }

    /// 取 `NonFiniteValue` 携带的规范字段路径。
    fn field_of(error: &ModelError) -> Option<&'static str> {
        match error {
            ModelError::NonFiniteValue { field, .. } => Some(field),
            _ => None,
        }
    }

    /// 断言结果被**指定**错误拒绝。
    fn rejects_matching(
        what: &str,
        result: Result<(), ModelError>,
        matcher: impl Fn(&ModelError) -> bool,
    ) -> Result<(), String> {
        match result {
            Err(ref error) if matcher(error) => Ok(()),
            other => Err(format!("{what} 必须被指定错误拒绝, 实际 {other:?}")),
        }
    }

    /// 断言结果被接受（有限极值必须走得通）。
    fn accepts(what: &str, result: Result<(), ModelError>) -> Result<(), String> {
        match result {
            Ok(()) => Ok(()),
            Err(error) => Err(format!("{what} 必须被接受, 实际 {error}")),
        }
    }

    fn probe_midi_note_probability() -> Result<(), String> {
        for value in NON_FINITE_F32 {
            let note = MidiNote {
                probability: Some(value),
                ..MidiNote::default()
            };
            rejects_matching("非有限概率", note.validate(), |error| {
                matches!(error, ModelError::ProbabilityOutOfRange { .. })
            })?;
        }
        for bad in [1.5_f32, -0.1] {
            rejects_matching(
                "越界概率",
                MidiNote {
                    probability: Some(bad),
                    ..MidiNote::default()
                }
                .validate(),
                |error| matches!(error, ModelError::ProbabilityOutOfRange { .. }),
            )?;
        }
        for good in [0.0_f32, 0.5, 1.0] {
            accepts(
                "闭区间概率",
                MidiNote {
                    probability: Some(good),
                    ..MidiNote::default()
                }
                .validate(),
            )?;
        }
        accepts(
            "None（必然触发）",
            MidiNote {
                probability: None,
                ..MidiNote::default()
            }
            .validate(),
        )
    }

    fn probe_parameter_value() -> Result<(), String> {
        let parameter = |value| ParameterValue {
            name: "cutoff".to_owned(),
            value,
            unit: None,
        };
        rejects_non_finite_f32("device.param.value", |value| parameter(value).validate())?;
        for value in [-f32::MAX, f32::MAX] {
            accepts("设备参数有限极值", parameter(value).validate())?;
        }
        Ok(())
    }

    fn probe_macro_mapping_depth() -> Result<(), String> {
        let mapping = |depth| MacroMapping {
            target: AutomationTarget::TrackVolume {
                track_id: fixture_id(5),
            },
            depth,
        };
        rejects_non_finite_f32("macro.mapping.depth", |value| mapping(value).validate())?;
        for bad in [1.5_f32, -0.5] {
            rejects_matching("深度越界", mapping(bad).validate(), |error| {
                matches!(error, ModelError::MacroDepthOutOfRange { .. })
            })?;
        }
        for good in [0.0_f32, 1.0] {
            accepts("闭区间深度", mapping(good).validate())?;
        }
        Ok(())
    }

    fn probe_macro_parameter_value() -> Result<(), String> {
        let macro_parameter = |value| MacroParameter {
            name: "Brightness".to_owned(),
            value,
            mappings: Vec::new(),
        };
        for value in NON_FINITE_F32 {
            rejects_matching(
                "非有限宏位置",
                macro_parameter(value).validate(),
                |error| matches!(error, ModelError::MacroValueOutOfRange { .. }),
            )?;
        }
        for bad in [1.5_f32, -0.5] {
            rejects_matching("越界宏位置", macro_parameter(bad).validate(), |error| {
                matches!(error, ModelError::MacroValueOutOfRange { .. })
            })?;
        }
        for good in [0.0_f32, 1.0] {
            accepts("闭区间宏位置", macro_parameter(good).validate())?;
        }
        Ok(())
    }

    fn probe_automation_point_value() -> Result<(), String> {
        let point = |value| AutomationPoint {
            id: fixture_id(6),
            tick: 0,
            value,
            curve: CurveType::Linear,
        };
        rejects_non_finite_f32("automation.point.value", |value| point(value).validate())?;
        for value in [-f32::MAX, f32::MAX] {
            accepts("自动化点有限极值", point(value).validate())?;
        }
        Ok(())
    }

    fn probe_domain_min() -> Result<(), String> {
        rejects_non_finite_f32("automation.lane.domain.min", |value| {
            AutomationValueDomain::new(value, 0.0).map(|_| ())
        })?;
        let flipped = AutomationValueDomain::new(12.0, -60.0).map_err(|error| error.to_string())?;
        if (flipped.min(), flipped.max()) != (-60.0, 12.0) {
            return Err(format!(
                "端点必须按定义排序, 实际 ({}, {})",
                flipped.min(),
                flipped.max()
            ));
        }
        Ok(())
    }

    fn probe_domain_max() -> Result<(), String> {
        rejects_non_finite_f32("automation.lane.domain.max", |value| {
            AutomationValueDomain::new(0.0, value).map(|_| ())
        })
    }

    fn probe_clip_audio_gain() -> Result<(), String> {
        let asset = AssetHash::of_bytes(b"probe");
        let audio = |gain_db| ClipContent::Audio {
            asset: asset.clone(),
            gain_db,
        };
        rejects_non_finite_f32("clip.audio.gain_db", |value| audio(value).validate())?;
        for value in [0.0_f32, -f32::MAX, f32::MAX] {
            accepts("音频片段有限增益极值", audio(value).validate())?;
        }
        Ok(())
    }

    fn probe_track_volume_db() -> Result<(), String> {
        let track = |volume_db| TrackV3 {
            volume_db,
            ..midi_track(fixture_id(1))
        };
        rejects_non_finite_f32("track.volume_db", |value| track(value).validate())?;
        for value in [0.0_f32, -f32::MAX, f32::MAX] {
            accepts("音轨音量有限极值", track(value).validate())?;
        }
        Ok(())
    }

    fn probe_track_pan() -> Result<(), String> {
        let track = |pan| TrackV3 {
            pan,
            ..midi_track(fixture_id(1))
        };
        rejects_non_finite_f32("track.pan", |value| track(value).validate())?;
        for bad in [1.5_f32, -1.5, 2.0] {
            rejects_matching("声相越界", track(bad).validate(), |error| {
                matches!(error, ModelError::PanOutOfRange { .. })
            })?;
        }
        for edge in [-1.0_f32, -0.5, 0.0, 1.0] {
            accepts("闭区间声相端点", track(edge).validate())?;
        }
        Ok(())
    }

    fn probe_scene_tempo() -> Result<(), String> {
        let scene = |tempo| SceneV3 {
            id: fixture_id(4),
            name: "Verse".to_owned(),
            tempo,
            color: None,
        };
        rejects_non_finite_f64("scene.tempo", |value| scene(Some(value)).validate())?;
        for bad in [19.9_f64, 999.5, 0.0, -120.0] {
            rejects_matching("速度越界", scene(Some(bad)).validate(), |error| {
                matches!(error, ModelError::BpmOutOfRange { .. })
            })?;
        }
        accepts("None（跟随工程速度）", scene(None).validate())?;
        for good in [MIN_BPM, 120.0, MAX_BPM] {
            accepts("闭区间速度端点", scene(Some(good)).validate())?;
        }
        Ok(())
    }

    fn probe_routing_edge_gain() -> Result<(), String> {
        let edge = |gain_db| RoutingEdge {
            id: fixture_id(3),
            source_node: fixture_id(1),
            destination_node: fixture_id(2),
            kind: RoutingKind::TrackToBus,
            gain_db,
        };
        rejects_non_finite_f32("routing.edge.gain_db", |value| edge(Some(value)).validate())?;
        accepts("None（单位增益）", edge(None).validate())?;
        for value in [-f32::MAX, f32::MAX] {
            accepts("路由增益有限极值", edge(Some(value)).validate())?;
        }
        Ok(())
    }

    fn probe_project_bpm() -> Result<(), String> {
        let project = |bpm| YebanProjectV1 {
            bpm,
            ..YebanProjectV1::default()
        };
        rejects_non_finite_f64("bpm", |value| project(value).validate())?;
        for bad in [19.9_f64, 999.5, 0.0, -120.0] {
            rejects_matching("bpm 越界", project(bad).validate(), |error| {
                matches!(error, ModelError::BpmOutOfRange { .. })
            })?;
        }
        for good in [MIN_BPM, 120.0, MAX_BPM] {
            accepts("闭区间 bpm 端点", project(good).validate())?;
        }
        Ok(())
    }

    /// Op 探针夹具：一条主总线音轨（含一个设备参数与一个宏）+ 一条路由边。
    #[derive(Clone, Copy)]
    struct NumericFixture {
        master: EntityId,
        edge: EntityId,
        point: EntityId,
    }

    fn numeric_fixture() -> (YebanProjectV1, NumericFixture) {
        let master = fixture_id(901);
        let edge_id = fixture_id(902);
        let doc = YebanProjectV1 {
            master_bus_track_id: master,
            tracks: BTreeMap::from([(
                master,
                TrackV3 {
                    id: master,
                    name: "Master".to_owned(),
                    kind: TrackKind::Master,
                    volume_db: -6.0,
                    pan: 0.0,
                    devices: vec![DeviceDefinition {
                        id: fixture_id(903),
                        name: "Synth".to_owned(),
                        params: vec![ParameterValue {
                            name: "cutoff".to_owned(),
                            value: 1200.0,
                            unit: None,
                        }],
                        ..DeviceDefinition::default()
                    }],
                    macros: vec![MacroParameter {
                        name: "Brightness".to_owned(),
                        value: 0.5,
                        mappings: Vec::new(),
                    }],
                    ..TrackV3::default()
                },
            )]),
            routing_graph: RoutingGraph {
                nodes: vec![master],
                edges: BTreeMap::from([(
                    edge_id,
                    RoutingEdge {
                        id: edge_id,
                        source_node: master,
                        destination_node: master,
                        kind: RoutingKind::BusToMaster,
                        gain_db: None,
                    },
                )]),
            },
            ..YebanProjectV1::default()
        };
        assert_eq!(doc.validate(), Ok(()), "Op 探针夹具必须是合法文档");
        let fixture = NumericFixture {
            master,
            edge: edge_id,
            point: fixture_id(904),
        };
        (doc, fixture)
    }

    /// 施加 `op` 并断言它被拒绝、文档逐字节不动、且被拒之后文档仍合法。
    fn apply_rejected(
        doc: &mut YebanProjectV1,
        op: &Op,
        label: &str,
    ) -> Result<ModelError, String> {
        let before = serde_json::to_vec(doc).map_err(|error| error.to_string())?;
        let Err(error) = op.apply(doc) else {
            return Err(format!("{label}: 必须被拒绝, 却成功了"));
        };
        let after = serde_json::to_vec(doc).map_err(|error| error.to_string())?;
        if before != after {
            return Err(format!("{label}: 被拒之后文档必须逐字节不动"));
        }
        doc.validate()
            .map_err(|error| format!("{label}: 被拒之后文档必须仍合法: {error}"))?;
        Ok(error)
    }

    /// 施加 `op` 并断言它被接受、且接受之后文档仍合法。
    fn apply_accepted(doc: &mut YebanProjectV1, op: &Op, label: &str) -> Result<(), String> {
        op.apply(doc)
            .map_err(|error| format!("{label}: 必须被接受, 实际 {error}"))?;
        doc.validate()
            .map_err(|error| format!("{label}: 接受之后文档必须仍合法: {error}"))
    }

    fn probe_set_param_old() -> Result<(), String> {
        let (mut doc, f) = numeric_fixture();
        for value in NON_FINITE_F32 {
            let op = Op::SetParam {
                target: AutomationTarget::TrackVolume { track_id: f.master },
                old_val: value,
                new_val: -6.0,
            };
            match apply_rejected(&mut doc, &op, "SetParam(old_val 非有限)") {
                Ok(ModelError::OpStateMismatch { .. }) => {}
                Ok(other) => {
                    return Err(format!(
                        "旧值只做逐位比较, 非有限旧值必须报 OpStateMismatch, 实际 {other:?}"
                    ));
                }
                Err(reason) => return Err(reason),
            }
        }
        Ok(())
    }

    fn probe_set_param_new() -> Result<(), String> {
        // ① 音量目标：非有限拒绝（field = "param.value"），有限极值接受。
        let (mut doc, f) = numeric_fixture();
        let volume = |new_val| Op::SetParam {
            target: AutomationTarget::TrackVolume { track_id: f.master },
            old_val: -6.0,
            new_val,
        };
        for value in NON_FINITE_F32 {
            match apply_rejected(&mut doc, &volume(value), "SetParam(音量非有限)") {
                Ok(ModelError::NonFiniteValue {
                    field: "param.value",
                    ..
                }) => {}
                Ok(other) => {
                    return Err(format!(
                        "音量非有限必须报 NonFiniteValue(\"param.value\"), 实际 {other:?}"
                    ));
                }
                Err(reason) => return Err(reason),
            }
        }
        for value in [-f32::MAX, f32::MAX] {
            let (mut doc, f) = numeric_fixture();
            let op = Op::SetParam {
                target: AutomationTarget::TrackVolume { track_id: f.master },
                old_val: -6.0,
                new_val: value,
            };
            apply_accepted(&mut doc, &op, "SetParam(音量有限极值)")?;
        }

        // ② 声相目标：非有限与越界都拒绝，闭区间端点接受。
        let (mut doc, f) = numeric_fixture();
        let pan = |new_val| Op::SetParam {
            target: AutomationTarget::TrackPan { track_id: f.master },
            old_val: 0.0,
            new_val,
        };
        for value in NON_FINITE_F32 {
            match apply_rejected(&mut doc, &pan(value), "SetParam(声相非有限)") {
                Ok(ModelError::NonFiniteValue {
                    field: "param.value",
                    ..
                }) => {}
                Ok(other) => {
                    return Err(format!(
                        "声相非有限必须报 NonFiniteValue(\"param.value\"), 实际 {other:?}"
                    ));
                }
                Err(reason) => return Err(reason),
            }
        }
        for bad in [1.5_f32, -1.5, 2.0] {
            match apply_rejected(&mut doc, &pan(bad), "SetParam(声相越界)") {
                Ok(ModelError::PanOutOfRange { .. }) => {}
                Ok(other) => {
                    return Err(format!("声相 {bad} 必须报 PanOutOfRange, 实际 {other:?}"));
                }
                Err(reason) => return Err(reason),
            }
        }
        for edge in [-1.0_f32, 0.0, 1.0] {
            let (mut doc, f) = numeric_fixture();
            let op = Op::SetParam {
                target: AutomationTarget::TrackPan { track_id: f.master },
                old_val: 0.0,
                new_val: edge,
            };
            apply_accepted(&mut doc, &op, "SetParam(声相端点)")?;
        }

        // ③ 宏目标：非有限先被有限性判据拦下（field = "param.value"），
        //    有限越界才报 MacroValueOutOfRange（`validate_param_value` 的先后顺序即口径）。
        let (mut doc, f) = numeric_fixture();
        let macro_target = |new_val| Op::SetParam {
            target: AutomationTarget::Macro {
                track_id: f.master,
                macro_index: 0,
            },
            old_val: 0.5,
            new_val,
        };
        for value in NON_FINITE_F32 {
            match apply_rejected(&mut doc, &macro_target(value), "SetParam(宏非有限)") {
                Ok(ModelError::NonFiniteValue {
                    field: "param.value",
                    ..
                }) => {}
                Ok(other) => {
                    return Err(format!(
                        "宏位置非有限必须报 NonFiniteValue(\"param.value\"), 实际 {other:?}"
                    ));
                }
                Err(reason) => return Err(reason),
            }
        }
        for bad in [1.5_f32, -0.5] {
            match apply_rejected(&mut doc, &macro_target(bad), "SetParam(宏越界)") {
                Ok(ModelError::MacroValueOutOfRange { .. }) => {}
                Ok(other) => {
                    return Err(format!(
                        "宏位置 {bad} 必须报 MacroValueOutOfRange, 实际 {other:?}"
                    ));
                }
                Err(reason) => return Err(reason),
            }
        }

        // ④ 设备参数目标：非有限拒绝，取值域模型不可知 ⇒ 有限极值接受。
        let (mut doc, f) = numeric_fixture();
        let device = |new_val| Op::SetParam {
            target: AutomationTarget::DeviceParam {
                track_id: f.master,
                slot_index: 0,
                param_index: 0,
            },
            old_val: 1200.0,
            new_val,
        };
        for value in NON_FINITE_F32 {
            match apply_rejected(&mut doc, &device(value), "SetParam(设备参数非有限)") {
                Ok(ModelError::NonFiniteValue {
                    field: "param.value",
                    ..
                }) => {}
                Ok(other) => {
                    return Err(format!(
                        "设备参数非有限必须报 NonFiniteValue(\"param.value\"), 实际 {other:?}"
                    ));
                }
                Err(reason) => return Err(reason),
            }
        }
        for value in [-f32::MAX, f32::MAX] {
            let (mut doc, f) = numeric_fixture();
            let op = Op::SetParam {
                target: AutomationTarget::DeviceParam {
                    track_id: f.master,
                    slot_index: 0,
                    param_index: 0,
                },
                old_val: 1200.0,
                new_val: value,
            };
            apply_accepted(&mut doc, &op, "SetParam(设备参数极值)")?;
        }

        // ⑤ 发送增益必须继续走 SetRoutingGain（Option 语义不可经 SetParam 破坏）。
        let (mut doc, f) = numeric_fixture();
        let send = Op::SetParam {
            target: AutomationTarget::SendGain {
                track_id: f.master,
                edge_id: f.edge,
            },
            old_val: 0.0,
            new_val: -3.0,
        };
        match apply_rejected(&mut doc, &send, "SetParam(SendGain)") {
            Ok(ModelError::AutomationTargetNotApplicable { .. }) => Ok(()),
            Ok(other) => Err(format!(
                "SendGain 必须报 AutomationTargetNotApplicable, 实际 {other:?}"
            )),
            Err(reason) => Err(reason),
        }
    }

    fn probe_set_macro_old() -> Result<(), String> {
        let (mut doc, f) = numeric_fixture();
        for value in NON_FINITE_F32 {
            let op = Op::SetMacro {
                track_id: f.master,
                macro_index: 0,
                old_val: value,
                new_val: 0.5,
            };
            match apply_rejected(&mut doc, &op, "SetMacro(old_val 非有限)") {
                Ok(ModelError::OpStateMismatch { .. }) => {}
                Ok(other) => {
                    return Err(format!("非有限旧值必须报 OpStateMismatch, 实际 {other:?}"));
                }
                Err(reason) => return Err(reason),
            }
        }
        Ok(())
    }

    fn probe_set_macro_new() -> Result<(), String> {
        let (mut doc, f) = numeric_fixture();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.5, -0.5] {
            let op = Op::SetMacro {
                track_id: f.master,
                macro_index: 0,
                old_val: 0.5,
                new_val: bad,
            };
            match apply_rejected(&mut doc, &op, "SetMacro(新值)") {
                Ok(ModelError::MacroValueOutOfRange { .. }) => {}
                Ok(other) => {
                    return Err(format!(
                        "宏新值 {bad} 必须报 MacroValueOutOfRange, 实际 {other:?}"
                    ));
                }
                Err(reason) => return Err(reason),
            }
        }
        for good in [0.0_f32, 1.0] {
            let (mut doc, f) = numeric_fixture();
            let op = Op::SetMacro {
                track_id: f.master,
                macro_index: 0,
                old_val: 0.5,
                new_val: good,
            };
            apply_accepted(&mut doc, &op, "SetMacro(闭区间端点)")?;
        }
        Ok(())
    }

    fn probe_set_routing_gain_old() -> Result<(), String> {
        let (mut doc, f) = numeric_fixture();
        for value in NON_FINITE_F32 {
            let op = Op::SetRoutingGain {
                edge_id: f.edge,
                old_gain_db: Some(value),
                new_gain_db: Some(-3.0),
            };
            match apply_rejected(&mut doc, &op, "SetRoutingGain(old 非有限)") {
                Ok(ModelError::OpStateMismatch { .. }) => {}
                Ok(other) => {
                    return Err(format!(
                        "非有限旧增益必须报 OpStateMismatch, 实际 {other:?}"
                    ));
                }
                Err(reason) => return Err(reason),
            }
        }
        Ok(())
    }

    fn probe_set_routing_gain_new() -> Result<(), String> {
        let (mut doc, f) = numeric_fixture();
        for value in NON_FINITE_F32 {
            let op = Op::SetRoutingGain {
                edge_id: f.edge,
                old_gain_db: None,
                new_gain_db: Some(value),
            };
            match apply_rejected(&mut doc, &op, "SetRoutingGain(new 非有限)") {
                Ok(ModelError::NonFiniteValue {
                    field: "routing.edge.gain_db",
                    ..
                }) => {}
                Ok(other) => {
                    return Err(format!(
                        "非有限新增益必须报 NonFiniteValue(\"routing.edge.gain_db\"), 实际 {other:?}"
                    ));
                }
                Err(reason) => return Err(reason),
            }
        }
        for value in [
            None,
            Some(-f32::MAX),
            Some(f32::MAX),
            Some(-60.0),
            Some(12.0),
        ] {
            let (mut doc, f) = numeric_fixture();
            let op = Op::SetRoutingGain {
                edge_id: f.edge,
                old_gain_db: None,
                new_gain_db: value,
            };
            apply_accepted(&mut doc, &op, "SetRoutingGain(有限或 None)")?;
        }
        Ok(())
    }

    /// 扫描器的**正向对照**：写在一行里的载荷字段必须被看见。
    ///
    /// 为什么需要：`every_numeric_field_in_the_model_is_inventoried` 的全部判别力都来自
    /// `scan_numeric_fields` 的识别器。实测（改动前）：下面三种形态对识别器**完全不可见** ——
    /// 把任意一种加进 `src/**`，那条判据保持全绿，于是"新数值字段必被登记"这条常驻判据
    /// 可以被三种常见写法绕过。这条判据按合成输入逐个钉住识别器的牙口
    /// （与 `tests/model_isolation.rs` 里 `code_has` 的正向对照同一手法）。
    ///
    /// 期望值口径：每行返回的字段名按出现顺序、允许重复（外层与内层花括号都会被扫描，
    /// 因此 `numeric_field` 对同样的 `name: f32` 可能命中两次）；调用方
    /// `scan_numeric_fields` 负责排序与去重。
    #[test]
    fn the_numeric_field_scanner_sees_one_line_payloads_and_restricted_visibility() {
        // 一行多个载荷字段。
        assert_eq!(
            numeric_fields_on_line("One { gain_db: f32, pan: f32 },"),
            vec!["gain_db".to_owned(), "pan".to_owned()]
        );
        // 写在一行里的载荷（整个 `enum` 一行写完）。
        assert_eq!(
            numeric_fields_on_line("pub enum P { One { gain_db: f32 } }"),
            vec!["gain_db".to_owned()]
        );
        // 写在一行里的声明（`struct` 字段在声明行自己身上）。
        assert_eq!(
            numeric_fields_on_line("struct S { pub gain_db: f32 }"),
            vec!["gain_db".to_owned()]
        );
        // 受限可见性、`Option<f32>`、属性前缀三种写法都要看得见。
        assert_eq!(
            numeric_fields_on_line("    pub(super) gain_db: f32,"),
            vec!["gain_db".to_owned()]
        );
        assert_eq!(
            numeric_fields_on_line("    pub(crate) level: Option<f32>,"),
            vec!["level".to_owned()]
        );
        assert_eq!(
            numeric_fields_on_line("#[serde(default)] pub x: Option<f64>,"),
            vec!["x".to_owned()]
        );
        // 负向对照：非数值字段、注释、属性行不许被收进来（否则判据会变成"什么都收"的假绿）。
        assert!(numeric_fields_on_line("    pub name: String,").is_empty());
        assert!(numeric_fields_on_line("    pub n: u64,").is_empty());
        assert!(numeric_fields_on_line("    /// 示例 { gain: f32 }").is_empty());
        assert!(numeric_fields_on_line("    #[serde(default)]").is_empty());
        assert_eq!(
            numeric_fields_on_line("    pub gain_db: f32, // 备注 { x: f32 }"),
            vec!["gain_db".to_owned()]
        );
        // `public` 只是以 `pub` 开头，它不是可见性关键字。
        assert_eq!(
            numeric_fields_on_line("    public: f32,"),
            vec!["public".to_owned()]
        );
    }

    /// 识别器的**第三组**正/负对照：容器类型里的数值标量 + 数值类型别名。
    ///
    /// 为什么需要：识别器原先只把类型原文与
    /// `"f32" | "f64" | "Option<f32>" | "Option<f64>"` **逐字**比较。实测（收紧前）：
    /// 往 `src/**` 里加 `pub probe_curve: Vec<f32>` 或
    /// `type ProbeAlias = f32;` + `pub gain: ProbeAlias` 时，
    /// 常驻判据 `every_numeric_field_in_the_model_is_inventoried` **保持全绿** ——
    /// 两种常见写法可以各藏一个数值字段过清单判据。本判据按合成输入逐个钉住新的识别面
    /// （正对照必须收进来，负对照必须不收）。
    #[test]
    fn the_numeric_field_scanner_sees_container_types_and_type_aliases() {
        // 正对照：容器里的数值标量按**标识符**判定，不要求逐字相等。
        assert_eq!(
            numeric_fields_on_line("    pub probe_curve: Vec<f32>,"),
            vec!["probe_curve".to_owned()]
        );
        assert_eq!(
            numeric_fields_on_line("    pub probe_curve: [f64; 4],"),
            vec!["probe_curve".to_owned()]
        );
        assert_eq!(
            numeric_fields_on_line("    pub probe_curve: Option<Vec<f32>>,"),
            vec!["probe_curve".to_owned()]
        );

        // 正对照：数值类型别名（`type X = f32;`）之后，类型写 `X` 的字段也是数值字段。
        let mut numeric_aliases = std::collections::BTreeSet::new();
        numeric_aliases.insert("ProbeAlias".to_owned());
        assert_eq!(
            numeric_fields_on_line_with("    pub gain: ProbeAlias,", &numeric_aliases),
            vec!["gain".to_owned()]
        );
        assert_eq!(
            numeric_fields_on_line_with("    pub gain: Option<Vec<ProbeAlias>>,", &numeric_aliases),
            vec!["gain".to_owned()]
        );
        // 别名本身要被识别出来（数值 / 非数值两种）。
        assert_eq!(
            type_alias("type ProbeAlias = f32;"),
            Some(("ProbeAlias".to_owned(), true))
        );
        assert_eq!(
            type_alias("pub type NameAlias = String;"),
            Some(("NameAlias".to_owned(), false))
        );
        assert_eq!(
            type_alias("type Err = ModelError;"),
            Some(("Err".to_owned(), false))
        );
        // 带泛型参数或没有 `=` 的声明不是别名。
        assert_eq!(type_alias("type Generic<T> = Vec<T>;"), None);
        assert_eq!(type_alias("typealias X;"), None);

        // 负对照：别名集合里没有的类型 / 非数值容器 / 非数值标量都不许被收进来。
        assert!(
            numeric_fields_on_line_with("    pub name: NameAlias,", &numeric_aliases).is_empty()
        );
        assert!(
            numeric_fields_on_line_with("    pub ids: Vec<EntityId>,", &numeric_aliases).is_empty()
        );
        assert!(numeric_fields_on_line_with("    pub n: u64,", &numeric_aliases).is_empty());
    }

    /// 类别 1／4 的**机械清单**：源码里每一个 `f32`／`f64` 字段都必须在冻结表里。
    ///
    /// 这条判据挡的是"新增了一个外部数值字段，但没人给它判据"—— 本仓库命中过的
    /// 7 类缺陷里第 1 类的入口。漏登记与多登记都会红。
    #[test]
    fn every_numeric_field_in_the_model_is_inventoried() {
        let found = scan_numeric_fields();
        let mut expected: Vec<String> = NUMERIC_FIELD_INVENTORY
            .iter()
            .map(|key| (*key).to_owned())
            .collect();
        expected.sort();
        assert_eq!(
            found, expected,
            "源码里的数值字段与冻结清单（NUMERIC_FIELD_INVENTORY）不一致;\n\
             新增字段请登记并补判定, 删字段请同步清单"
        );
        assert_eq!(found.len(), 27, "实测读数: 27 条数值字段声明");
    }

    /// 逐项判定：清单里每个**入口**字段都有一行判定，且该行的探针必须真的成立。
    ///
    /// `ModelError` 的 6 个 `value` 字段是错误输出载体（不是入口），
    /// 因此被**显式**豁免 —— 豁免集合本身也被钉住（恰好 6 条），不能悄悄扩大。
    #[test]
    fn every_numeric_entry_point_has_a_passing_policy_probe() {
        let inventory: std::collections::BTreeSet<&str> =
            NUMERIC_FIELD_INVENTORY.iter().copied().collect();
        let outputs: std::collections::BTreeSet<&str> = inventory
            .iter()
            .copied()
            .filter(|key| key.starts_with("src/error.rs::ModelError::"))
            .collect();
        assert_eq!(
            outputs.len(),
            6,
            "输出载体豁免必须恰好 6 条（ModelError 的 value 字段）: {outputs:?}"
        );
        let expected: std::collections::BTreeSet<&str> =
            inventory.difference(&outputs).copied().collect();
        let registered: std::collections::BTreeSet<&str> = NUMERIC_FIELD_POLICY
            .iter()
            .map(|(key, _, _)| *key)
            .collect();
        assert_eq!(
            registered, expected,
            "判定表与机械清单必须**双向**对齐（漏一行/多一行都红）"
        );

        for (key, verdict, probe) in NUMERIC_FIELD_POLICY {
            probe().unwrap_or_else(|reason| panic!("{key} 的判定「{verdict}」不成立: {reason}"));
        }
    }

    /// 类别 1：`Op::AddClip` 与 `YebanProjectV1::insert_clip` 曾经**不校验载荷**。
    ///
    /// 缺陷形态（改动前实测）：`apply` 返回 `Ok(())`、片段入池，随后
    /// `YebanProjectV1::validate()` 才报 `NonFiniteValue`/`ProbabilityOutOfRange`
    /// —— 也就是说"先污染、后报错"，而两个入口的书写者都以为载荷被检查过。
    #[test]
    fn add_clip_and_insert_clip_reject_non_finite_content() {
        for gain in NON_FINITE_F32 {
            let audio = ClipPoolEntry {
                id: fixture_id(910),
                name: "Audio".to_owned(),
                content: ClipContent::Audio {
                    asset: AssetHash::of_bytes(b"probe"),
                    gain_db: gain,
                },
            };

            // ① Op 入口
            let (mut doc, _) = numeric_fixture();
            let before = serde_json::to_vec(&doc).expect("序列化");
            let error = Op::AddClip {
                clip: audio.clone(),
            }
            .apply(&mut doc)
            .expect_err("非有限片段增益必须被拒");
            assert!(
                matches!(&error, ModelError::NonFiniteValue { field, .. } if *field == "clip.audio.gain_db"),
                "实际 {error:?}"
            );
            assert_eq!(
                serde_json::to_vec(&doc).expect("序列化"),
                before,
                "被拒之后文档必须逐字节不动"
            );
            assert!(doc.clip_pool.is_empty(), "被拒之后片段池必须为空");
            assert_eq!(doc.validate(), Ok(()));

            // ② 文档级入口
            let mut doc = YebanProjectV1::default();
            let error = doc.insert_clip(audio).expect_err("非有限片段增益必须被拒");
            assert!(
                matches!(&error, ModelError::NonFiniteValue { field, .. } if *field == "clip.audio.gain_db"),
                "实际 {error:?}"
            );
            assert!(doc.clip_pool.is_empty(), "被拒之后片段池必须为空");
            assert_eq!(doc.validate(), Ok(()));

            // ③ MIDI 片段的音符概率（同一入口的另一种载荷）
            let note = MidiNote {
                probability: Some(gain),
                ..MidiNote::default()
            };
            let midi = ClipPoolEntry {
                id: fixture_id(911),
                name: "Midi".to_owned(),
                content: ClipContent::Midi {
                    notes: BTreeMap::from([(note.id, note)]),
                },
            };
            let (mut doc, _) = numeric_fixture();
            let before = serde_json::to_vec(&doc).expect("序列化");
            let error = Op::AddClip { clip: midi.clone() }
                .apply(&mut doc)
                .expect_err("非有限音符概率必须被拒");
            assert!(
                matches!(error, ModelError::ProbabilityOutOfRange { .. }),
                "实际 {error:?}"
            );
            assert_eq!(serde_json::to_vec(&doc).expect("序列化"), before);
            assert!(doc.clip_pool.is_empty());
            assert_eq!(doc.validate(), Ok(()));

            let mut doc = YebanProjectV1::default();
            assert!(doc.insert_clip(midi).is_err());
            assert!(doc.clip_pool.is_empty());
        }
    }

    /// 类别 1：`insert_track` 曾不校验载荷（同族的 `insert_note` 校验），
    /// 于是"先污染、后由 `validate()` 报错"。既有期望值（合法音轨可插入、
    /// 身份重复报 `DuplicateEntityId`）一个都没动。
    #[test]
    fn insert_track_rejects_non_finite_payloads() {
        for volume in NON_FINITE_F32 {
            let mut doc = YebanProjectV1::default();
            let track = TrackV3 {
                id: fixture_id(920),
                name: "T".to_owned(),
                volume_db: volume,
                ..TrackV3::default()
            };
            let error = doc.insert_track(track).expect_err("非有限音量必须被拒");
            assert!(
                matches!(&error, ModelError::NonFiniteValue { field, .. } if *field == "track.volume_db"),
                "实际 {error:?}"
            );
            assert!(doc.tracks.is_empty(), "被拒之后音轨集合必须为空");
        }
        for pan in [f32::NAN, 1.5] {
            let mut doc = YebanProjectV1::default();
            let track = TrackV3 {
                id: fixture_id(921),
                name: "T".to_owned(),
                pan,
                ..TrackV3::default()
            };
            assert!(doc.insert_track(track).is_err(), "声相 {pan} 必须被拒");
            assert!(doc.tracks.is_empty());
        }

        let mut doc = YebanProjectV1::default();
        let track = midi_track(fixture_id(922));
        doc.insert_track(track.clone()).expect("合法音轨必须能插入");
        assert_eq!(
            doc.insert_track(track),
            Err(ModelError::DuplicateEntityId {
                id: fixture_id(922)
            }),
            "身份重复仍必须报 DuplicateEntityId"
        );
    }

    /// 类别 1：**载荷类型间接含 `f32`** 的其余 `Op` 入口也必须拒绝非有限值。
    ///
    /// `SetParam`／`SetMacro`／`SetRoutingGain` 由 `NUMERIC_FIELD_POLICY` 的三行覆盖；
    /// 这里补齐 `AddTrack`（`TrackV3::volume_db`）、`SetAutomationPoint`／
    /// `SetAutomationLane`（`AutomationPoint::value`）与 `Batch`（整批原子回滚）。
    #[test]
    fn every_numeric_op_entry_point_rejects_non_finite_payloads() {
        for value in NON_FINITE_F32 {
            // AddTrack
            let (mut doc, _) = numeric_fixture();
            let track = TrackV3 {
                id: fixture_id(930),
                name: "New".to_owned(),
                volume_db: value,
                ..TrackV3::default()
            };
            let error = Op::AddTrack { track }
                .apply(&mut doc)
                .expect_err("AddTrack 非有限音量必须被拒");
            assert!(
                matches!(&error, ModelError::NonFiniteValue { field, .. } if *field == "track.volume_db"),
                "实际 {error:?}"
            );
            assert!(doc.tracks.len() == 1, "被拒之后不得新增音轨");
            assert_eq!(doc.validate(), Ok(()));

            // SetAutomationPoint
            let (mut doc, f) = numeric_fixture();
            let error = Op::SetAutomationPoint {
                target: AutomationTarget::TrackVolume { track_id: f.master },
                point_id: f.point,
                old_point: None,
                new_point: AutomationPoint {
                    id: f.point,
                    tick: 0,
                    value,
                    curve: CurveType::Linear,
                },
            }
            .apply(&mut doc)
            .expect_err("SetAutomationPoint 非有限点值必须被拒");
            assert!(
                matches!(&error, ModelError::NonFiniteValue { field, .. } if *field == "automation.point.value"),
                "实际 {error:?}"
            );
            assert_eq!(doc.validate(), Ok(()));

            // SetAutomationLane
            let (mut doc, f) = numeric_fixture();
            let target = AutomationTarget::TrackVolume { track_id: f.master };
            let error = Op::SetAutomationLane {
                target,
                old_lane: None,
                new_lane: AutomationLane {
                    target,
                    points: BTreeMap::from([(
                        f.point,
                        AutomationPoint {
                            id: f.point,
                            tick: 0,
                            value,
                            curve: CurveType::Linear,
                        },
                    )]),
                    read_enabled: true,
                    write_mode: crate::project::AutomationWriteMode::Off,
                    domain: None,
                },
            }
            .apply(&mut doc)
            .expect_err("SetAutomationLane 非有限点值必须被拒");
            assert!(
                matches!(&error, ModelError::NonFiniteValue { field, .. } if *field == "automation.point.value"),
                "实际 {error:?}"
            );
            assert!(doc.automation_lane(&target).is_none());
            assert_eq!(doc.validate(), Ok(()));
        }

        // Batch：子操作非法 ⇒ 整批不生效（原子性 + 非有限拒绝）。
        let (mut doc, f) = numeric_fixture();
        let before = serde_json::to_vec(&doc).expect("序列化");
        let batch = Op::Batch {
            ops: vec![Op::SetParam {
                target: AutomationTarget::TrackVolume { track_id: f.master },
                old_val: -6.0,
                new_val: f32::NAN,
            }],
            description: "非有限子操作".to_owned(),
        };
        assert!(batch.apply(&mut doc).is_err(), "Batch 内的非有限写必须被拒");
        assert_eq!(
            serde_json::to_vec(&doc).expect("序列化"),
            before,
            "Batch 失败必须整批回滚"
        );
        assert_eq!(doc.validate(), Ok(()));
    }
}
