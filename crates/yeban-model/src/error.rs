//! 数据模型错误类型。
//!
//! 所有校验失败一律返回 `Result<_, ModelError>`，绝不在解析路径上 panic：
//! 工程文件来自磁盘、MCP 代理或第三方归档，全部属于不可信输入。

use thiserror::Error;

use crate::ids::{AssetHash, EntityId};

/// `yeban-model` 的统一错误类型。
///
/// 只派生 `PartialEq` 而不派生 `Eq`：`ProbabilityOutOfRange` 携带 `f32`，而 `f32`
/// 不满足 `Eq`。这里保留 `PartialEq` 是为了让测试可以直接对错误做断言。
#[derive(Debug, Error, PartialEq)]
pub enum ModelError {
    /// MIDI 音高越界 (允许 0..=127)。
    #[error("pitch {value} out of range 0..=127")]
    PitchOutOfRange {
        /// 实际收到的音高值。
        value: u16,
    },

    /// 力度越界 (允许 0..=127)。
    #[error("velocity {value} out of range 0..=127")]
    VelocityOutOfRange {
        /// 实际收到的力度值。
        value: u16,
    },

    /// 概率字段越界 (允许 0.0..=1.0，且必须有限)。
    #[error("probability {value} out of range 0.0..=1.0")]
    ProbabilityOutOfRange {
        /// 实际收到的概率值。
        value: f32,
    },

    /// ratchet 连击数越界 (允许 1..=16)。
    #[error("ratchet {value} out of range 1..=16")]
    RatchetOutOfRange {
        /// 实际收到的连击数。
        value: u8,
    },

    /// 微时值偏移越界 (允许 -240..=240 tick，即 ±1/16 音符 @960 PPQ)。
    #[error("micro timing {value} ticks out of range -240..=240")]
    MicroTimingOutOfRange {
        /// 实际收到的 tick 偏移。
        value: i32,
    },

    /// 时长为零：音符必须有正时长。
    #[error("duration must be non-zero")]
    ZeroDuration,

    /// 哈希字符串不是 64 位小写十六进制 SHA-256。
    #[error("`{value}` is not a 64-char lowercase hex SHA-256 digest")]
    InvalidHash {
        /// 实际收到的字符串。
        value: String,
    },

    /// `EntityId` 文本不是合法的 26 字符 Crockford Base32 ULID。
    #[error("`{value}` is not a valid 26-char Crockford Base32 ULID")]
    InvalidEntityId {
        /// 实际收到的字符串。
        value: String,
    },

    /// 文档 schema 版本高于本读取器支持的上限，必须拒绝而不是猜测。
    #[error("document schema_version {found} is newer than supported {supported}")]
    SchemaVersionTooNew {
        /// 文档声明的版本。
        found: u32,
        /// 本读取器支持的最大版本。
        supported: u32,
    },

    /// 文档 schema 版本低于 `min_reader_version`，读取器太旧。
    #[error("document requires reader >= {required}, this reader is {actual}")]
    ReaderTooOld {
        /// 文档要求的读取器版本。
        required: u32,
        /// 本读取器版本。
        actual: u32,
    },

    /// 速度 (BPM) 越界 (允许 20.0..=999.0，且必须有限)。
    #[error("bpm {value} out of range 20.0..=999.0")]
    BpmOutOfRange {
        /// 实际收到的速度值。
        value: f64,
    },

    /// 拍号分子越界 (允许 1..=32)。
    #[error("time signature numerator {value} out of range 1..=32")]
    TimeSignatureNumeratorOutOfRange {
        /// 实际收到的分子。
        value: u8,
    },

    /// 拍号分母不在允许集合 `{1,2,4,8,16,32}` 中。
    #[error("time signature denominator {value} is not one of 1,2,4,8,16,32")]
    TimeSignatureDenominatorUnsupported {
        /// 实际收到的分母。
        value: u8,
    },

    /// 采样率不在规范允许集合 `{44100,48000,88200,96000,192000}` 中。
    #[error("sample rate {value} is not one of 44100,48000,88200,96000,192000")]
    SampleRateUnsupported {
        /// 实际收到的采样率 (Hz)。
        value: u32,
    },

    /// 缓冲区长度不在规范允许集合 `{64,128,256,512,1024}` 中。
    #[error("block size {value} is not one of 64,128,256,512,1024")]
    BlockSizeUnsupported {
        /// 实际收到的缓冲区长度 (frames)。
        value: u32,
    },

    /// 浮点字段不是有限值 (`NaN` / `±Inf` 一律拒绝：它们无法确定性地序列化)。
    #[error("{field} must be finite, got {value}")]
    NonFiniteValue {
        /// 字段名 (规范字段路径)。
        field: &'static str,
        /// 实际收到的值。
        value: f64,
    },

    /// 声相越界 (允许 -1.0..=1.0)。
    #[error("pan {value} out of range -1.0..=1.0")]
    PanOutOfRange {
        /// 实际收到的声相值。
        value: f32,
    },

    /// 同一集合中出现了重复身份。
    #[error("duplicate entity id `{id}`")]
    DuplicateEntityId {
        /// 冲突的身份。
        id: EntityId,
    },

    /// 引用了不存在的音轨。
    #[error("track `{id}` not found")]
    TrackNotFound {
        /// 缺失的音轨身份。
        id: EntityId,
    },

    /// 引用了不存在的片段池条目。
    #[error("clip `{id}` not found in clip pool")]
    ClipNotFound {
        /// 缺失的片段身份。
        id: EntityId,
    },

    /// 引用了不存在的音符。
    #[error("note `{id}` not found")]
    NoteNotFound {
        /// 缺失的音符身份。
        id: EntityId,
    },

    /// 引用了不存在的片段摆放 (placement)。
    #[error("clip placement `{id}` not found")]
    ClipPlacementNotFound {
        /// 缺失的摆放身份。
        id: EntityId,
    },

    /// 引用了不存在的路由边。
    #[error("routing edge `{id}` not found")]
    RoutingEdgeNotFound {
        /// 缺失的路由边身份。
        id: EntityId,
    },

    /// 片段仍被摆放引用，不能从片段池移除 [ARCH-OPS-001]。
    #[error("clip `{clip_id}` is still referenced by {placement_count} placement(s)")]
    ClipInUse {
        /// 片段身份。
        clip_id: EntityId,
        /// 仍引用它的摆放数量。
        placement_count: usize,
    },
    /// 路由节点仍被路由边引用，不能移除 [ARCH-OPS-001]。
    #[error("routing node `{node}` is still referenced by {edge_count} edge(s)")]
    RoutingNodeInUse {
        /// 节点身份。
        node: EntityId,
        /// 仍引用它的边数量。
        edge_count: usize,
    },
    /// 引用了不存在的路由节点。
    #[error("routing node `{id}` not found in routing graph")]
    RoutingNodeNotFound {
        /// 缺失的路由节点身份。
        id: EntityId,
    },

    /// 设备插槽下标越界。
    #[error("device slot index {index} out of range (0..={len})")]
    DeviceSlotOutOfRange {
        /// 请求的插槽下标。
        index: usize,
        /// 当前设备链长度。
        len: usize,
    },

    /// 参数下标越界。
    #[error("parameter index {index} out of range (len {len})")]
    ParamIndexOutOfRange {
        /// 请求的参数下标。
        index: usize,
        /// 该设备的参数个数。
        len: usize,
    },

    /// 宏下标越界。
    #[error("macro index {index} out of range (len {len})")]
    MacroIndexOutOfRange {
        /// 请求的宏下标。
        index: usize,
        /// 该音轨的宏个数。
        len: usize,
    },

    /// 引用了不存在的自动化点。
    #[error("automation point `{id}` not found")]
    AutomationPointNotFound {
        /// 缺失的自动化点身份。
        id: EntityId,
    },

    /// 引用了不存在的曲式段落。
    #[error("section `{id}` not found")]
    SectionNotFound {
        /// 缺失的段落身份。
        id: EntityId,
    },

    /// 引用了不存在的场景。
    #[error("scene `{id}` not found")]
    SceneNotFound {
        /// 缺失的场景身份。
        id: EntityId,
    },

    /// 引用了不存在的提交。
    #[error("commit `{id}` not found in commit graph")]
    CommitNotFound {
        /// 缺失的提交身份。
        id: EntityId,
    },

    /// 引用了不存在的分支。
    #[error("branch `{name}` not found")]
    BranchNotFound {
        /// 缺失的分支名。
        name: String,
    },

    /// 集合键与实体内部身份不一致。
    #[error("collection key `{key}` does not match embedded entity id `{embedded}`")]
    EntityKeyMismatch {
        /// 集合键。
        key: EntityId,
        /// 实体内部携带的身份。
        embedded: EntityId,
    },

    /// 资产集合的键与 `AssetMetadata.hash` 不一致。
    #[error("asset key `{key}` does not match embedded hash `{embedded}`")]
    AssetKeyMismatch {
        /// 集合键（内容哈希）。
        key: AssetHash,
        /// 元数据内部携带的哈希。
        embedded: AssetHash,
    },

    /// 自动化泳道的目标与它在文档里的**位置**不一致。两种情形共用本变体：
    ///
    /// 1. `TrackV3::automation_lanes` 的**键**与该泳道内部携带的 `lane.target` 不一致
    ///    （键 ≠ 载荷）；
    /// 2. 泳道挂在了**别的**音轨的 `automation_lanes` 里 —— 即 `target.track_id()`
    ///    不等于宿主音轨的 `id`（`AutomationTarget::track_id` 的文档把"泳道在文档里的
    ///    位置"定义为该目标自己的音轨）。
    ///
    /// 两种情形都让同一份文档在不同消费者眼里成为两件事（唯一求值入口按目标查、
    /// 界面投影按音轨遍历），因此一律**响亮拒绝**而不是猜一个。
    #[error("automation lane target does not match its position: {key} vs {embedded}")]
    AutomationLaneTargetMismatch {
        /// 情形 1：集合键（目标调试形式）；情形 2：宿主音轨身份（规范 ULID 文本）。
        key: String,
        /// 情形 1：泳道内部携带的目标调试形式；情形 2：目标音轨身份（规范 ULID 文本）。
        embedded: String,
    },

    /// `master_bus_track_id` 指向的音轨不是 `Master` 类型。
    #[error("master bus track `{id}` must have kind `Master`")]
    MasterBusKindMismatch {
        /// 被引用的音轨身份。
        id: EntityId,
    },

    /// 该操作要求 MIDI 片段，但目标是音频片段（或反之）。
    #[error("clip `{id}` does not hold the content kind required by this operation")]
    ClipContentKindMismatch {
        /// 片段身份。
        id: EntityId,
    },

    /// 宏位置越界 (允许 0.0..=1.0)。
    #[error("macro value {value} out of range 0.0..=1.0")]
    MacroValueOutOfRange {
        /// 实际收到的宏位置。
        value: f32,
    },

    /// 宏到参数的映射深度越界 (允许 0.0..=1.0)。
    #[error("macro mapping depth {value} out of range 0.0..=1.0")]
    MacroDepthOutOfRange {
        /// 实际收到的映射深度。
        value: f32,
    },

    /// 操作载荷与文档当前状态不一致。
    ///
    /// 两种触发场景：`Op::apply` 时"载荷里的旧值与文档不符"（拒绝执行），
    /// 以及 `Op::invert` 时"该操作并未作用于给定文档"（拒绝把撤销打错文档）。
    #[error("`{op}` does not match the document state (payload / post-condition mismatch)")]
    OpStateMismatch {
        /// `Op` 的 JSON 变体名，便于日志定位。
        op: &'static str,
    },

    /// 该自动化目标不能被当前操作寻址。
    #[error("automation target cannot be addressed by this operation: {detail}")]
    AutomationTargetNotApplicable {
        /// 人话解释（例如"发送增益必须走 SetRoutingGain 以保留 Option 语义"）。
        detail: &'static str,
    },
}
