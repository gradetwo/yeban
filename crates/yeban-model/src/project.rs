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
//! ### 已知契约冲突（本线不得私自改写 `schemas/`，故留痕待裁决）
//!
//! `schemas/project.schema.json` 把 `writer_version` 声明为 `integer`，
//! 而 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §2.2 与
//! `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` D3 裁决它是
//! **应用语义版本字符串**。本实现按 ADR（更晚、更专门的裁决）与架构正文取 `String`。
//! 详细留痕见 `docs/ledger/model-core-provenance.md`。

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
    #[serde(default)]
    pub description: String,
    /// 创建时间 (Unix 毫秒)。
    #[serde(default)]
    pub created_at_unix_ms: u64,
    /// 最后修改时间 (Unix 毫秒)。
    #[serde(default)]
    pub modified_at_unix_ms: u64,
    /// 自由标签。
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
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransportConfig {
    /// 节拍器是否默认开启。
    #[serde(default)]
    pub metronome_enabled: bool,
    /// 录音前的预备小节数。
    #[serde(default)]
    pub count_in_bars: u8,
    /// 场景启动的默认量化。
    #[serde(default)]
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
    #[serde(default)]
    pub bypassed: bool,
    /// 参数列表。
    #[serde(default)]
    pub params: Vec<ParameterValue>,
}

impl Default for DeviceDefinition {
    fn default() -> Self {
        Self {
            id: EntityId::default(),
            name: "Device".to_owned(),
            kind: DeviceKind::default(),
            bypassed: false,
            params: Vec::new(),
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
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MacroParameter {
    /// 宏名。
    #[serde(default)]
    pub name: String,
    /// 宏当前位置 0.0..=1.0。
    #[serde(default)]
    pub value: f32,
    /// 映射列表。
    #[serde(default)]
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
    #[serde(default)]
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

/// 自动化泳道（一个目标一条）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AutomationLane {
    /// 该泳道控制的目标。
    pub target: AutomationTarget,
    /// 自动化点集合，键为点身份（`BTreeMap` 保证迭代顺序确定）[MODEL-AST-003]。
    #[serde(default)]
    pub points: BTreeMap<EntityId, AutomationPoint>,
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
    #[serde(default)]
    pub enabled: bool,
    /// 循环起点 (tick)。
    #[serde(default)]
    pub start_tick: u64,
    /// 循环终点 (tick)。
    #[serde(default)]
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
    #[serde(default)]
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
        #[serde(default)]
        notes: BTreeMap<EntityId, MidiNote>,
    },
    /// 音频片段：内容寻址的资产引用 + 增益。
    Audio {
        /// 资产哈希（CAS 键）[MODEL-AST-007]。
        asset: AssetHash,
        /// 片段增益 (dB)。
        #[serde(default)]
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
    #[serde(default)]
    pub loop_config: LoopConfig,
    /// 是否静音。
    #[serde(default)]
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
    #[serde(default)]
    pub solo_safe: bool,
    /// 所属折叠文件夹（**仅界面语义**）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<EntityId>,
    /// 界面色标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// 设备链（有序）。
    #[serde(default)]
    pub devices: Vec<DeviceDefinition>,
    /// 宏。
    #[serde(default)]
    pub macros: Vec<MacroParameter>,
    /// 自动化泳道集合（`BTreeMap`，键为目标）[MODEL-AST-003]。
    ///
    /// JSON 形态是**按键升序的数组**：`serde_json` 的对象键必须是字符串，
    /// 而 [`AutomationTarget`] 是结构化枚举（序列化为对象），无法直接当键。
    /// 详见 `automation_lane_map`。
    #[serde(default, with = "automation_lane_map")]
    pub automation_lanes: BTreeMap<AutomationTarget, AutomationLane>,
    /// 时间轴摆放集合（`BTreeMap`，键为摆放身份）[MODEL-AST-003]。
    #[serde(default)]
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
            for (point_id, point) in &lane.points {
                if *point_id != point.id {
                    return Err(ModelError::EntityKeyMismatch {
                        key: *point_id,
                        embedded: point.id,
                    });
                }
                point.validate()?;
            }
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
    #[serde(default)]
    pub author: String,
    /// 速度 20.0..=999.0（顶层唯一权威）。
    pub bpm: f64,
    /// 拍号（顶层唯一权威）。
    pub time_signature: TimeSignature,
    /// 音频配置 —— **采样率与声相律的唯一来源**。
    pub audio_config: ProjectAudioConfig,
    /// 确定性随机种子（`probability` 触发判定等算法的跨机一致性）[MODEL-AST-005]。
    #[serde(default)]
    pub rng_seed: u64,
    /// 工程元数据。
    #[serde(default)]
    pub metadata: ProjectMetadata,
    /// 传输控制配置。
    #[serde(default)]
    pub transport: TransportConfig,
    /// 曲式段落集合。
    #[serde(default)]
    pub sections: BTreeMap<EntityId, SectionV3>,
    /// 音轨集合 [MODEL-AST-003]。
    #[serde(default)]
    pub tracks: BTreeMap<EntityId, TrackV3>,
    /// 主总线音轨身份。
    #[serde(default)]
    pub master_bus_track_id: EntityId,
    /// 唯一声学路由真理源 [MODEL-AST-004]。
    #[serde(default)]
    pub routing_graph: RoutingGraph,
    /// 场景集合。
    #[serde(default)]
    pub scenes: BTreeMap<EntityId, SceneV3>,
    /// 片段池。
    #[serde(default)]
    pub clip_pool: BTreeMap<EntityId, ClipPoolEntry>,
    /// 资产索引（键为内容哈希）。
    #[serde(default)]
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
        Ok(())
    }

    /// 工程采样率的唯一读取入口（顶层没有冗余副本）[MODEL-AST-002]。
    #[must_use]
    pub const fn sample_rate(&self) -> SampleRate {
        self.audio_config.sample_rate
    }

    /// 追加音轨；身份重复即拒绝。
    ///
    /// # Errors
    ///
    /// `track.id` 已存在 → [`ModelError::DuplicateEntityId`]。
    pub fn insert_track(&mut self, track: TrackV3) -> Result<(), ModelError> {
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
    /// # Errors
    ///
    /// `entry.id` 已存在 → [`ModelError::DuplicateEntityId`]。
    pub fn insert_clip(&mut self, entry: ClipPoolEntry) -> Result<(), ModelError> {
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
    use std::str::FromStr;

    /// 构造确定性的规范 ULID 文本（前 6 位固定，余下 20 位十进制序号）。
    pub(crate) fn fixture_id(index: u128) -> EntityId {
        EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
    }

    fn master_track(id: EntityId) -> TrackV3 {
        TrackV3 {
            id,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        }
    }

    fn midi_track(id: EntityId) -> TrackV3 {
        TrackV3 {
            id,
            name: "Lead".to_owned(),
            ..TrackV3::default()
        }
    }

    fn midi_clip(id: EntityId) -> ClipPoolEntry {
        ClipPoolEntry {
            id,
            name: "Clip".to_owned(),
            content: ClipContent::default(),
        }
    }

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
        let project = filled_sample_document();
        assert_eq!(project.validate(), Ok(()), "{:?}", project.validate());
        let json = serde_json::to_string_pretty(&project).expect("serialize");
        let back: YebanProjectV1 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, project);
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

    /// 构造一个把全部子类型都填满的规范样本（样本 JSON 必须逐字节稳定）。
    pub(crate) fn filled_sample_document() -> YebanProjectV1 {
        let master_id = fixture_id(1);
        let lead_id = fixture_id(2);
        let bass_id = fixture_id(3);
        let aux_id = fixture_id(4);

        let mut lead = midi_track(lead_id);
        lead.volume_db = -3.0;
        lead.pan = -0.25;
        lead.color = Some("#FF8800".to_owned());
        lead.devices = vec![DeviceDefinition {
            id: fixture_id(20),
            name: "Yeban PolySynth".to_owned(),
            kind: DeviceKind::InternalInstrument,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "cutoff".to_owned(),
                    value: 1200.0,
                    unit: Some("Hz".to_owned()),
                },
                ParameterValue {
                    name: "resonance".to_owned(),
                    value: 0.35,
                    unit: Some("%".to_owned()),
                },
            ],
        }];
        lead.macros = vec![MacroParameter {
            name: "Brightness".to_owned(),
            value: 0.5,
            mappings: vec![MacroMapping {
                target: AutomationTarget::DeviceParam {
                    track_id: lead_id,
                    slot_index: 0,
                    param_index: 0,
                },
                depth: 0.8,
            }],
        }];
        lead.automation_lanes.insert(
            AutomationTarget::TrackVolume { track_id: lead_id },
            AutomationLane {
                target: AutomationTarget::TrackVolume { track_id: lead_id },
                points: BTreeMap::from([
                    (
                        fixture_id(40),
                        AutomationPoint {
                            id: fixture_id(40),
                            tick: 0,
                            value: -6.0,
                            curve: CurveType::Linear,
                        },
                    ),
                    (
                        fixture_id(41),
                        AutomationPoint {
                            id: fixture_id(41),
                            tick: 3840,
                            value: 0.0,
                            curve: CurveType::SCurve,
                        },
                    ),
                ]),
            },
        );

        let clip_id = fixture_id(10);
        let mut clip = midi_clip(clip_id);
        if let Some(notes) = clip.content.notes_mut() {
            for (index, (start_tick, pitch)) in [(0_u64, 60_u8), (960, 64), (1920, 67), (2880, 72)]
                .into_iter()
                .enumerate()
            {
                let note_id = fixture_id(100 + index as u128);
                notes.insert(
                    note_id,
                    MidiNote {
                        probability: if index == 3 { Some(0.75) } else { None },
                        ratchet: if index == 1 { Some(2) } else { None },
                        micro_timing_ticks: if index == 2 { Some(-12) } else { None },
                        syllable: Some(["do", "re", "mi", "fa"][index].to_owned()),
                        ..MidiNote::new(note_id, start_tick, pitch, 480)
                    },
                );
            }
        }

        let audio_clip_id = fixture_id(11);
        let audio_clip = ClipPoolEntry {
            id: audio_clip_id,
            name: "Kick".to_owned(),
            content: ClipContent::Audio {
                asset: AssetHash::of_bytes(b"yeban-kick-sample"),
                gain_db: -1.5,
            },
        };

        let placement_id = fixture_id(50);
        lead.clips.insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick: 0,
                duration_ticks: 3840,
                loop_config: LoopConfig {
                    enabled: true,
                    start_tick: 0,
                    end_tick: 3840,
                },
                muted: false,
            },
        );
        let audio_placement_id = fixture_id(51);
        let mut bass = TrackV3 {
            id: bass_id,
            name: "Bass".to_owned(),
            kind: TrackKind::Audio,
            volume_db: -6.0,
            pan: 0.0,
            mute: false,
            solo: false,
            solo_safe: true,
            folder_id: None,
            color: Some("#3366FF".to_owned()),
            ..TrackV3::default()
        };
        bass.clips.insert(
            audio_placement_id,
            ClipPlacement {
                id: audio_placement_id,
                clip_id: audio_clip_id,
                start_tick: 1920,
                duration_ticks: 960,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );

        let aux = TrackV3 {
            id: aux_id,
            name: "Aux Reverb".to_owned(),
            kind: TrackKind::AuxReturn,
            volume_db: -9.0,
            ..TrackV3::default()
        };

        let lead_to_master = fixture_id(60);
        let bass_to_master = fixture_id(61);
        let lead_to_aux = fixture_id(62);
        let mut edges = BTreeMap::new();
        for (id, source, destination, kind, gain_db) in [
            (
                lead_to_master,
                lead_id,
                master_id,
                RoutingKind::TrackToBus,
                None,
            ),
            (
                bass_to_master,
                bass_id,
                master_id,
                RoutingKind::TrackToBus,
                None,
            ),
            (
                lead_to_aux,
                lead_id,
                aux_id,
                RoutingKind::SendToAux,
                Some(-12.0),
            ),
        ] {
            edges.insert(
                id,
                RoutingEdge {
                    id,
                    source_node: source,
                    destination_node: destination,
                    kind,
                    gain_db,
                },
            );
        }

        let asset_hash = AssetHash::of_bytes(b"yeban-kick-sample");
        YebanProjectV1 {
            schema_version: SCHEMA_VERSION,
            min_reader_version: MIN_READER_VERSION,
            writer_version: DEFAULT_WRITER_VERSION.to_owned(),
            id: fixture_id(999),
            title: "Yeban Model Core Sample".to_owned(),
            author: "Yeban Project Contributors".to_owned(),
            bpm: 128.0,
            time_signature: TimeSignature {
                numerator: 4,
                denominator: 4,
            },
            audio_config: ProjectAudioConfig {
                sample_rate: SampleRate::Hz48000,
                block_size: BlockSize::Frames256,
                bit_depth: BitDepth::Float32,
                pan_law: PanLaw::ConstantPowerMinus3dB,
            },
            rng_seed: DEFAULT_RNG_SEED,
            metadata: ProjectMetadata {
                description: "Cross-implementation schema sample".to_owned(),
                created_at_unix_ms: 1_760_000_000_000,
                modified_at_unix_ms: 1_760_000_000_000,
                tags: vec!["sample".to_owned(), "schema".to_owned()],
            },
            transport: TransportConfig {
                metronome_enabled: false,
                count_in_bars: 2,
                launch_quantization: LaunchQuantization::Bar,
            },
            sections: BTreeMap::from([
                (
                    fixture_id(70),
                    SectionV3 {
                        id: fixture_id(70),
                        name: "Intro".to_owned(),
                        start_tick: 0,
                        end_tick: 7680,
                        color: Some("#22AA88".to_owned()),
                    },
                ),
                (
                    fixture_id(71),
                    SectionV3 {
                        id: fixture_id(71),
                        name: "Drop".to_owned(),
                        start_tick: 7680,
                        end_tick: 15360,
                        color: None,
                    },
                ),
            ]),
            tracks: BTreeMap::from([
                (master_id, master_track(master_id)),
                (lead_id, lead),
                (bass_id, bass),
                (aux_id, aux),
            ]),
            master_bus_track_id: master_id,
            routing_graph: RoutingGraph {
                nodes: vec![lead_id, bass_id, aux_id, master_id],
                edges,
            },
            scenes: BTreeMap::from([(
                fixture_id(80),
                SceneV3 {
                    id: fixture_id(80),
                    name: "Scene 1".to_owned(),
                    tempo: Some(128.0),
                    color: None,
                },
            )]),
            clip_pool: BTreeMap::from([(clip_id, clip), (audio_clip_id, audio_clip)]),
            assets: BTreeMap::from([(
                asset_hash.clone(),
                AssetMetadata {
                    hash: asset_hash,
                    original_path: "samples/kick.wav".to_owned(),
                    byte_len: 44_100,
                    media_kind: MediaKind::Audio,
                    license: "CC0-1.0".to_owned(),
                },
            )]),
        }
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
        let schema: serde_json::Value = serde_json::from_str(&text).expect("schema 必须是合法 JSON");
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

    /// 样本导出：`scripts/gates/validate_schemas.py` 约定前缀 `project.<name>.json`。
    #[test]
    fn export_schema_samples_to_target() {
        let dir = samples_dir();
        std::fs::create_dir_all(&dir).expect("create samples dir");

        let default_document = YebanProjectV1::default();
        assert_eq!(default_document.validate(), Ok(()));

        let filled_document = filled_sample_document();
        assert_eq!(filled_document.validate(), Ok(()));
        assert_eq!(filled_document.check_readable(), Ok(()));

        for (name, document) in [
            ("project.default.json", &default_document),
            ("project.filled.json", &filled_document),
        ] {
            let mut json = serde_json::to_string_pretty(document).expect("serialize");
            json.push('\n');
            std::fs::write(dir.join(name), json).expect("write sample");
        }

        // 两次导出必须逐字节相同（样本是 CI 对账口径，不能漂移）。
        let first = std::fs::read_to_string(dir.join("project.filled.json")).expect("read");
        let mut again = serde_json::to_string_pretty(&filled_sample_document()).expect("serialize");
        again.push('\n');
        assert_eq!(first, again, "样本导出必须逐字节稳定");
    }

    /// 样本目录：`<repo>/target/schema-samples`（可用 `CARGO_TARGET_DIR` 覆盖）。
    fn samples_dir() -> std::path::PathBuf {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
            || manifest_dir.join("..").join("..").join("target"),
            std::path::PathBuf::from,
        );
        target.join("schema-samples")
    }
}
