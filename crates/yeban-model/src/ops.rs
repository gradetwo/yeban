//! 强类型可逆领域操作日志 [ARCH-OPS-001, ROAD-M1-003]。
//!
//! 所有改变工程文档的动作都被捕获为不可变、强类型、**自包含逆操作**的 [`StampedOp`]。
//!
//! ## 三条设计约束
//!
//! 1. **实时 DSP 事件不是 Op**：`OpOrigin` 严格排除播放/自动化播放这类瞬态渲染流，
//!    数据模型只受版本化领域操作驱动（`ARCH-OPS-001`）。
//! 2. **删除类 Op 自带撤销载荷**：`DeleteNote` 携带 `previous_note`、`RemoveTrack`
//!    携带 `previous_track`…… 撤销因此**不需要**回放历史，单步撤销是 O(log n) 的。
//! 3. **可达性可判定**：[`Op::precondition`] 是只读的"本操作在当前文档上是否可应用"，
//!    [`Op::apply`] 先查前置条件再落盘，[`Op::invert`] 则用逆操作的前置条件反查
//!    "这个 Op 真的作用在这份文档上了吗" —— 于是撤销**不可能**被误打到错误的文档上。
//!
//! ## 规范缺口与裁决（已由 ADR-0001 留痕）
//!
//! 架构 §6.1 给出的 `Op` 全集**无法表达"删除曲式段落 / 删除场景"**：`SetSection`
//! 用 `old_section: Option<SectionV3>` 表示"新建"，但 `old_section == None` 时
//! 它的逆操作必须是删除，而全集里没有对应的变体 —— 这会让
//! `MUST-GATE-010`（状态树逆向幂等性）在"新建段落"这一步必然失败。
//! 因此本模块补上 [`Op::RemoveSection`] 与 [`Op::RemoveScene`] 两个变体，
//! 它们是 `SetSection { old_section: None }` / `SetScene { old_scene: None }` 的逆；
//! 裁决记录见 `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md`
//! **D12**（补两个删除变体，并把 `schemas/ops.schema.json` 的 `op.oneOf` 补齐到 23 个）
//! 与 **D13**（`origin` 改为 `oneOf`：6 个单元变体是纯字符串，
//! `McpProposal` 是外部标签对象，保留 `{proposal_id, agent_name}` 载荷）。
//! 更早的冲突实测留痕见 `docs/ledger/model-core-provenance.md`。
//!
//! **`McpEdit` 的契约（`line/origin-contract`，追平 `line/op-origin-mcp`）**：
//! `OpOrigin` 在 `line/op-origin-mcp` 新增了 [`OpOrigin::McpEdit`]（MCP 直接编辑的作者标签），
//! 而那条线禁改 `schemas/**` ⇒ 漂移当时由测试里的 `PENDING_CONTRACT_ORIGINS` 机械钉住。
//! 现在 `schemas/ops.schema.json` 的 `origin.oneOf` 已在**末尾**补上 `McpEdit` 对象分支
//! （`additionalProperties: false`，只许 `agent_name`），该清单因此清空。
//! 为什么必须"末尾"：两条判据按下标读契约 —— `oneOf[0]` 是单元枚举、`oneOf[1]` 是
//! `McpProposal`（其载荷键在 `oneOf[1].properties.McpProposal.required` 硬编码核对）。
//! 插到前面会让这两处读到错的形状（实测：106 passed / 3 failed）。追平原委与证据见
//! `docs/ledger/origin-contract-notes.md`。
//!
//! 本模块的契约一致性有两条**直接读契约文件**的判据（不手抄第二份事实源）：
//! `op_variants_match_ops_schema_exactly` 与
//! `origin_variants_match_ops_schema_origin_one_of`（另有
//! `mcp_edit_origin_shape_matches_its_contract_branch` 逐字段核对新分支的载荷）。

use serde::{Deserialize, Serialize};

use crate::error::ModelError;
use crate::ids::EntityId;
use crate::music::MidiNote;
use crate::project::{
    AutomationLane, AutomationPoint, AutomationTarget, ClipPlacement, ClipPoolEntry,
    DeviceDefinition, RoutingEdge, SceneV3, SectionV3, TrackV3, YebanProjectV1,
};

/// 操作来源 [ARCH-OPS-001]。
///
/// 严格排除实时 DSP 播放事件：自动化**播放**属于瞬态渲染流，
/// 绝不进入持久化 Op 日志。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum OpOrigin {
    /// 用户界面直接操作。
    UserUi,
    /// MIDI 硬件输入。
    MidiInput,
    /// MCP 代理提案（保留提案身份与代理名，便于审计与一键回滚定位）。
    McpProposal {
        /// 提案身份。
        proposal_id: EntityId,
        /// 提交提案的代理名。
        agent_name: String,
    },
    /// MCP 代理的**直接编辑**：工具在活跃工程上直接改一位并提交，**不创建提案**。
    ///
    /// 与 [`OpOrigin::McpProposal`] 的唯一区别就是"没有提案身份" —— 载荷因此只剩两档
    /// 共有的 `agent_name`。这条变体是对"来源标签必须如实"的补齐：在它出现之前，
    /// MCP 的直接编辑（`yeban_edit_automation` / `yeban_import_audio`）只能借
    /// [`OpOrigin::AutomationRecord`]（"自动化录制落盘"）与 [`OpOrigin::Import`]
    /// （"外部工程/格式导入"）—— 两档描述的都不是"代理直接改活跃工程"这件事。
    ///
    /// ⚠ **契约已追平**（`line/origin-contract`）：`schemas/ops.schema.json` 的
    /// `origin.oneOf` 末尾已有 `McpEdit` 对象分支（`additionalProperties: false`，
    /// 只许 `agent_name`）—— 与 [`OpOrigin::McpProposal`] 并列的第二个对象标签。
    /// 追平前这份漂移由测试里的 `PENDING_CONTRACT_ORIGINS` 机械钉住，现已清空；
    /// 逐字段对照与三条判据的真实要求见 `docs/ledger/origin-contract-notes.md`。
    McpEdit {
        /// 执行直接编辑的代理名。
        agent_name: String,
    },
    /// 撤销/重做自身产生的操作。
    UndoRedo,
    /// 自动化录制（录制结束后的**落盘**动作，不是实时播放）。
    AutomationRecord,
    /// 外部工程/格式导入。
    Import,
    /// 历史数据迁移器产生。
    Migration,
}

/// 带来源与时间戳的操作日志条目 [ARCH-OPS-001]。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StampedOp {
    /// 操作来源。
    pub origin: OpOrigin,
    /// Unix 毫秒时间戳（由调用方提供，模型层不自取时钟以保证可测试性）。
    pub timestamp: u64,
    /// 领域操作本体。
    pub op: Op,
}

impl StampedOp {
    /// 构造一条带戳操作。
    #[must_use]
    pub const fn new(origin: OpOrigin, timestamp: u64, op: Op) -> Self {
        Self {
            origin,
            timestamp,
            op,
        }
    }

    /// 构造一条用户界面来源的操作。
    #[must_use]
    pub const fn user_ui(timestamp: u64, op: Op) -> Self {
        Self::new(OpOrigin::UserUi, timestamp, op)
    }

    /// 应用本体操作。
    ///
    /// # Errors
    ///
    /// 前置条件不成立时返回对应 [`ModelError`]。
    pub fn apply(&self, doc: &mut YebanProjectV1) -> Result<(), ModelError> {
        self.op.apply(doc)
    }

    /// 应用本体操作的逆操作（单步撤销）。
    ///
    /// # Errors
    ///
    /// 逆操作不可构造或不可应用时返回对应 [`ModelError`]。
    pub fn apply_inverse(&self, doc: &mut YebanProjectV1) -> Result<(), ModelError> {
        self.op.apply_inverse(doc)
    }
}

/// 强类型可逆领域操作全集 [ARCH-OPS-001, ROAD-M1-003]。
///
/// 每个**删除/移除**类变体都自带 `previous_*` 撤销载荷，
/// 因此撤销不需要回溯历史，也不会因为历史分支被 GC 而失效。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Op {
    /// 在片段中插入音符。
    AddNote {
        /// 所属音轨。
        track_id: EntityId,
        /// 所属片段。
        clip_id: EntityId,
        /// 被插入的音符。
        note: MidiNote,
    },
    /// 删除音符（自带撤销载荷）。
    DeleteNote {
        /// 所属音轨。
        track_id: EntityId,
        /// 所属片段。
        clip_id: EntityId,
        /// 被删除的音符身份。
        note_id: EntityId,
        /// 删除前的完整音符。
        previous_note: MidiNote,
    },
    /// 平移音符（tick 与音高增量）。
    ///
    /// ⚠ **本变体是全集里唯一的相对变换**：载荷是**增量**（`delta_*`）而不是旧值，
    /// 因此前置条件**无法**判定"这一遍是不是重复施加" —— 同一 `MoveNote` 连续施加两次
    /// 会把增量**再加一次**（对比 [`Op::MoveClipPlacement`]：那个携带 `old_start_tick`
    /// 与 `new_start_tick`，第二遍必然被前置条件拒绝）。
    ///
    /// 这是契约本身（字段形状由 `schemas/ops.schema.json` 规定，模型层改不动），
    /// 不是可以在这里修掉的缺陷；但它意味着**调用方不得盲目重试**同一 `MoveNote`。
    /// 工具面的重放由 `yeban-mcp` 的 `idempotencyKey` 缓存挡住（不是本 crate 的职责）。
    /// 该"唯一例外"由判据 `re_applying_the_same_op_is_rejected_and_leaves_the_document_byte_identical`
    /// 双向登记：谁把 `MoveNote` 改成绝对值形态，那条判据会要求同步更新登记表。
    MoveNote {
        /// 所属音轨。
        track_id: EntityId,
        /// 所属片段。
        clip_id: EntityId,
        /// 音符身份。
        note_id: EntityId,
        /// tick 增量（可为负）。
        delta_tick: i64,
        /// 半音增量（可为负）。
        delta_pitch: i8,
    },
    /// 修改音符力度。
    ModifyNoteVelocity {
        /// 所属音轨。
        track_id: EntityId,
        /// 所属片段。
        clip_id: EntityId,
        /// 音符身份。
        note_id: EntityId,
        /// 修改前力度。
        old_vel: u8,
        /// 修改后力度。
        new_vel: u8,
    },
    /// 在音轨时间轴上摆放片段。
    AddClipPlacement {
        /// 目标音轨。
        track_id: EntityId,
        /// 摆放内容。
        placement: ClipPlacement,
    },
    /// 移除摆放（自带撤销载荷）。
    RemoveClipPlacement {
        /// 目标音轨。
        track_id: EntityId,
        /// 摆放身份。
        placement_id: EntityId,
        /// 移除前的摆放。
        previous_placement: ClipPlacement,
    },
    /// 把片段放进片段池。
    ///
    /// 为什么必须有这个变体：`AddClipPlacement` 要求片段**已经在** `clip_pool` 里，
    /// 而在本变体出现之前**没有任何 `Op` 能把条目放进池子** ⇒ "新建一个片段"
    /// 在操作日志层不可表达（由 `line/tools-domain` 实测发现，见 ADR-0001 D27）。
    AddClip {
        /// 被新增的片段池条目。
        clip: ClipPoolEntry,
    },
    /// 从片段池移除片段（自带撤销载荷）。
    ///
    /// 前置条件：片段存在、且**没有任何摆放引用它**（否则那些摆放会悬空 → [`ModelError::ClipInUse`]）。
    RemoveClip {
        /// 片段身份。
        clip_id: EntityId,
        /// 移除前的完整条目。
        previous_clip: ClipPoolEntry,
    },
    /// 平移摆放。
    MoveClipPlacement {
        /// 目标音轨。
        track_id: EntityId,
        /// 摆放身份。
        placement_id: EntityId,
        /// 平移前起点。
        old_start_tick: u64,
        /// 平移后起点。
        new_start_tick: u64,
    },
    /// 新增音轨。
    AddTrack {
        /// 被新增的音轨。
        track: TrackV3,
    },
    /// 移除音轨（自带撤销载荷）。
    RemoveTrack {
        /// 音轨身份。
        track_id: EntityId,
        /// 移除前的完整音轨。
        previous_track: TrackV3,
    },
    /// 连接一条路由边。
    ConnectRouting {
        /// 被连接的路由边。
        edge: RoutingEdge,
    },
    /// 断开路由边（自带撤销载荷）。
    DisconnectRouting {
        /// 路由边身份。
        edge_id: EntityId,
        /// 断开前的完整边。
        previous_edge: RoutingEdge,
    },
    /// 把节点加入路由图。
    ///
    /// 为什么必须有这个变体：`ConnectRouting` 要求两端**已经在** `routing_graph.nodes` 里，
    /// 而在本变体出现之前**没有任何 `Op` 能把节点放进去** ⇒ 声部连接在操作日志层不可表达
    /// （由 `line/tools-domain` 实测发现，见 ADR-0001 D27）。
    /// 节点按**字典序**插入，因此 `nodes` 恒有序 ⇒ 增删互为逆操作且无需额外载荷。
    AddRoutingNode {
        /// 节点身份（音轨或总线）。
        node: EntityId,
    },
    /// 从路由图移除节点。
    ///
    /// 前置条件：节点存在、且**没有任何边引用它**（否则返回 [`ModelError::RoutingNodeInUse`]）。
    RemoveRoutingNode {
        /// 节点身份。
        node: EntityId,
    },
    /// 设置路由边增益（保留 `Option` 语义：`None` 表示单位增益）。
    SetRoutingGain {
        /// 路由边身份。
        edge_id: EntityId,
        /// 修改前增益。
        old_gain_db: Option<f32>,
        /// 修改后增益。
        new_gain_db: Option<f32>,
    },
    /// 在设备链插槽插入设备。
    InsertDevice {
        /// 目标音轨。
        track_id: EntityId,
        /// 插槽下标（`0..=len`，等于 `len` 表示追加到链尾）。
        slot_index: usize,
        /// 被插入的设备。
        device: DeviceDefinition,
    },
    /// 移除设备（自带撤销载荷）。
    RemoveDevice {
        /// 目标音轨。
        track_id: EntityId,
        /// 插槽下标。
        slot_index: usize,
        /// 移除前的完整设备。
        previous_device: DeviceDefinition,
    },
    /// 设置参数值。
    ///
    /// 目标为 [`AutomationTarget::SendGain`] 时**拒绝**：发送增益必须走
    /// [`Op::SetRoutingGain`]，否则 `None`（单位增益）与 `Some(0.0)` 无法区分，
    /// 撤销将无法精确还原。
    SetParam {
        /// 参数寻址目标。
        target: AutomationTarget,
        /// 修改前数值。
        old_val: f32,
        /// 修改后数值。
        new_val: f32,
    },
    /// 设置音轨**静音**。
    ///
    /// 为什么必须有这个变体（而不是复用 [`Op::SetParam`]）：`SetParam` 的两个载荷都是
    /// `f32`，而 [`TrackV3::mute`] 是 `bool`；[`AutomationTarget`] 也没有静音变体
    /// ⇒ 静音在操作日志层**不可表达**（`ARCH-OPS-001` 的同一族缺口；
    /// 与 D12 / D27 / D42 补 `RemoveSection` / `AddClip` / `SetAutomationLane` 的理由同型）。
    SetTrackMute {
        /// 目标音轨。
        track_id: EntityId,
        /// 修改前静音。
        old_mute: bool,
        /// 修改后静音。
        new_mute: bool,
    },
    /// 设置音轨**独奏**。
    ///
    /// 与 [`Op::SetTrackMute`] 同族：[`TrackV3::solo`] 也是 `bool`。
    /// `TrackV3::solo_safe` **不在**载荷里 —— 本变体只表达这一个开关，不顺手改别的字段。
    SetTrackSolo {
        /// 目标音轨。
        track_id: EntityId,
        /// 修改前独奏。
        old_solo: bool,
        /// 修改后独奏。
        new_solo: bool,
    },
    /// 设置宏位置。
    SetMacro {
        /// 目标音轨。
        track_id: EntityId,
        /// 宏下标。
        macro_index: usize,
        /// 修改前位置 0.0..=1.0。
        old_val: f32,
        /// 修改后位置 0.0..=1.0。
        new_val: f32,
    },
    /// 新增或更新一个自动化点。
    SetAutomationPoint {
        /// 自动化泳道目标。
        target: AutomationTarget,
        /// 自动化点身份。
        point_id: EntityId,
        /// 修改前的点（`None` 表示该点原先不存在）。
        old_point: Option<AutomationPoint>,
        /// 修改后的点。
        new_point: AutomationPoint,
    },
    /// 删除自动化点（自带撤销载荷）。
    RemoveAutomationPoint {
        /// 自动化泳道目标。
        target: AutomationTarget,
        /// 自动化点身份。
        point_id: EntityId,
        /// 删除前的点。
        previous_point: AutomationPoint,
    },
    /// 新增或整体替换一条自动化泳道（含其采样点、读开关、写模式与取值域）。
    ///
    /// `old_lane == None` 表示**新建**；其逆操作是 [`Op::RemoveAutomationLane`]。
    ///
    /// 为什么必须有这个变体：在它出现之前，"一条泳道"只能被采样点**隐式**携带
    /// —— `SetAutomationPoint` 会在目标上自动建一条默认泳道，而没有任何变体能把
    /// "这条泳道是否参与播放（`read_enabled`）/ 录制写模式 / 取值域"表达出来。
    /// 于是规范要求的"自动化曲线"在操作日志层不可表达（与 ADR-0001 D12/D27 同一族）。
    SetAutomationLane {
        /// 自动化泳道目标（同时是泳道的键与身份）。
        target: AutomationTarget,
        /// 修改前的泳道（`None` 表示原先没有这条泳道）。
        old_lane: Option<AutomationLane>,
        /// 修改后的泳道。
        new_lane: AutomationLane,
    },
    /// 删除一条自动化泳道（自带撤销载荷）。
    RemoveAutomationLane {
        /// 自动化泳道目标。
        target: AutomationTarget,
        /// 删除前的完整泳道。
        previous_lane: AutomationLane,
    },
    /// 新增、更新或清空一个曲式段落。
    ///
    /// `old_section == None` 且 `new_section` 存在表示**新建**；
    /// 其逆操作是 [`Op::RemoveSection`]（见模块级"规范缺口留痕"）。
    SetSection {
        /// 段落身份。
        section_id: EntityId,
        /// 修改前的段落。
        old_section: Option<SectionV3>,
        /// 修改后的段落。
        new_section: SectionV3,
    },
    /// 删除曲式段落（自带撤销载荷）。
    ///
    /// 本变体是对架构 §6.1 全集的补齐：没有它，`SetSection { old_section: None }`
    /// 不可逆，`MUST-GATE-010` 必然失败。
    RemoveSection {
        /// 段落身份。
        section_id: EntityId,
        /// 删除前的段落。
        previous_section: SectionV3,
    },
    /// 新增、更新或清空一个场景。
    SetScene {
        /// 场景身份。
        scene_id: EntityId,
        /// 修改前的场景。
        old_scene: Option<SceneV3>,
        /// 修改后的场景。
        new_scene: SceneV3,
    },
    /// 删除场景（自带撤销载荷）。与 [`Op::RemoveSection`] 同因补齐。
    RemoveScene {
        /// 场景身份。
        scene_id: EntityId,
        /// 删除前的场景。
        previous_scene: SceneV3,
    },
    /// 原子批处理：整体成功或整体不生效。
    Batch {
        /// 子操作（按顺序应用）。
        ops: Vec<Op>,
        /// 批次描述（用于界面显示与审计）。
        description: String,
    },
}

/// 判断两个 `f32` 是否**逐位**相同（避免 `NaN`/`-0.0` 带来的语义歧义）。
#[must_use]
fn same_f32(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits()
}

/// 判断两个增益 `Option<f32>` 是否逐位相同。
#[must_use]
fn same_gain(left: Option<f32>, right: Option<f32>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(a), Some(b)) => same_f32(a, b),
        _ => false,
    }
}

/// 计算平移后的 tick；越出 `u64` 表示非法。
#[must_use]
fn shifted_tick(start_tick: u64, delta_tick: i64) -> Option<u64> {
    let shifted = i128::from(start_tick) + i128::from(delta_tick);
    if shifted < 0 || shifted > i128::from(u64::MAX) {
        None
    } else {
        u64::try_from(shifted).ok()
    }
}

/// 计算平移后的音高；越出 `0..=127` 表示非法。
#[must_use]
fn shifted_pitch(pitch: u8, delta_pitch: i8) -> Option<u8> {
    let shifted = i16::from(pitch) + i16::from(delta_pitch);
    if (0..=127).contains(&shifted) {
        u8::try_from(shifted).ok()
    } else {
        None
    }
}

/// 只读查询片段中的音符。
fn find_note<'a>(
    doc: &'a YebanProjectV1,
    clip_id: &EntityId,
    note_id: &EntityId,
) -> Result<Option<&'a MidiNote>, ModelError> {
    doc.note(clip_id, note_id).map(Some).or_else(|error| {
        if matches!(error, ModelError::NoteNotFound { .. }) {
            Ok(None)
        } else {
            Err(error)
        }
    })
}

impl Op {
    /// `Op` 的 JSON 变体名（与 `schemas/ops.schema.json` 的键一致）。
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::AddNote { .. } => "AddNote",
            Self::DeleteNote { .. } => "DeleteNote",
            Self::MoveNote { .. } => "MoveNote",
            Self::ModifyNoteVelocity { .. } => "ModifyNoteVelocity",
            Self::AddClip { .. } => "AddClip",
            Self::RemoveClip { .. } => "RemoveClip",
            Self::AddRoutingNode { .. } => "AddRoutingNode",
            Self::RemoveRoutingNode { .. } => "RemoveRoutingNode",
            Self::AddClipPlacement { .. } => "AddClipPlacement",
            Self::RemoveClipPlacement { .. } => "RemoveClipPlacement",
            Self::MoveClipPlacement { .. } => "MoveClipPlacement",
            Self::AddTrack { .. } => "AddTrack",
            Self::RemoveTrack { .. } => "RemoveTrack",
            Self::ConnectRouting { .. } => "ConnectRouting",
            Self::DisconnectRouting { .. } => "DisconnectRouting",
            Self::SetRoutingGain { .. } => "SetRoutingGain",
            Self::InsertDevice { .. } => "InsertDevice",
            Self::RemoveDevice { .. } => "RemoveDevice",
            Self::SetParam { .. } => "SetParam",
            Self::SetTrackMute { .. } => "SetTrackMute",
            Self::SetTrackSolo { .. } => "SetTrackSolo",
            Self::SetMacro { .. } => "SetMacro",
            Self::SetAutomationPoint { .. } => "SetAutomationPoint",
            Self::RemoveAutomationPoint { .. } => "RemoveAutomationPoint",
            Self::SetAutomationLane { .. } => "SetAutomationLane",
            Self::RemoveAutomationLane { .. } => "RemoveAutomationLane",
            Self::SetSection { .. } => "SetSection",
            Self::RemoveSection { .. } => "RemoveSection",
            Self::SetScene { .. } => "SetScene",
            Self::RemoveScene { .. } => "RemoveScene",
            Self::Batch { .. } => "Batch",
        }
    }

    /// 只读前置条件检查：本操作能否作用在 `doc` 上。
    ///
    /// 逆操作的可应用性也用它判定，因此 [`Op::invert`] 能可靠地拒绝
    /// "把一个 Op 的逆操作打到另一份文档上"。
    ///
    /// # Errors
    ///
    /// 引用缺失、载荷与文档状态不一致、或数值非法时返回对应 [`ModelError`]。
    pub fn precondition(&self, doc: &YebanProjectV1) -> Result<(), ModelError> {
        match self {
            Self::AddNote {
                track_id,
                clip_id,
                note,
            } => {
                doc.track(track_id)?;
                note.validate()?;
                if find_note(doc, clip_id, &note.id)?.is_some() {
                    return Err(ModelError::DuplicateEntityId { id: note.id });
                }
                Ok(())
            }
            Self::DeleteNote {
                track_id,
                clip_id,
                note_id,
                previous_note,
            } => {
                doc.track(track_id)?;
                let current = doc.note(clip_id, note_id)?;
                if current != previous_note {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::MoveNote {
                track_id,
                clip_id,
                note_id,
                delta_tick,
                delta_pitch,
            } => {
                doc.track(track_id)?;
                let current = doc.note(clip_id, note_id)?;
                if shifted_tick(current.start_tick, *delta_tick).is_none() {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                if shifted_pitch(current.pitch, *delta_pitch).is_none() {
                    return Err(ModelError::PitchOutOfRange {
                        value: u16::from(current.pitch),
                    });
                }
                Ok(())
            }
            Self::ModifyNoteVelocity {
                track_id,
                clip_id,
                note_id,
                old_vel,
                new_vel,
            } => {
                doc.track(track_id)?;
                let current = doc.note(clip_id, note_id)?;
                if current.velocity != *old_vel {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                if *new_vel > crate::music::MIDI_VELOCITY_MAX {
                    return Err(ModelError::VelocityOutOfRange {
                        value: u16::from(*new_vel),
                    });
                }
                Ok(())
            }
            Self::AddClipPlacement {
                track_id,
                placement,
            } => {
                let track = doc.track(track_id)?;
                placement.validate()?;
                if !doc.clip_pool.contains_key(&placement.clip_id) {
                    return Err(ModelError::ClipNotFound {
                        id: placement.clip_id,
                    });
                }
                if track.clips.contains_key(&placement.id) {
                    return Err(ModelError::DuplicateEntityId { id: placement.id });
                }
                Ok(())
            }
            Self::RemoveClipPlacement {
                track_id,
                placement_id,
                previous_placement,
            } => {
                let track = doc.track(track_id)?;
                let current = track
                    .clips
                    .get(placement_id)
                    .ok_or(ModelError::ClipPlacementNotFound { id: *placement_id })?;
                if current != previous_placement {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::MoveClipPlacement {
                track_id,
                placement_id,
                old_start_tick,
                ..
            } => {
                let track = doc.track(track_id)?;
                let current = track
                    .clips
                    .get(placement_id)
                    .ok_or(ModelError::ClipPlacementNotFound { id: *placement_id })?;
                if current.start_tick != *old_start_tick {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::AddTrack { track } => {
                track.validate()?;
                if doc.tracks.contains_key(&track.id) {
                    return Err(ModelError::DuplicateEntityId { id: track.id });
                }
                Ok(())
            }
            Self::RemoveTrack {
                track_id,
                previous_track,
            } => {
                if *track_id == doc.master_bus_track_id && !track_id.is_nil() {
                    // 移除主总线会让 `master_bus_track_id` 悬空，破坏文档自洽性。
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                let current = doc.track(track_id)?;
                if current != previous_track {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::AddClip { clip } => {
                // 载荷校验必须在这里做：`content.validate()` 是**唯一**拦住非有限数值的
                // 地方（音频片段的 `gain_db`、MIDI 音符的 `probability`），而
                // `YebanProjectV1::validate()` 只在文档**已经**被污染之后才报错
                // （类别 1「非有限输入」）。同族入口 `AddNote` / `AddTrack` /
                // `InsertDevice` / `ConnectRouting` / `AddClipPlacement` 都先校验载荷，
                // 本变体是**唯一**漏掉的那个。
                clip.content.validate()?;
                if doc.clip_pool.contains_key(&clip.id) {
                    return Err(ModelError::DuplicateEntityId { id: clip.id });
                }
                Ok(())
            }
            Self::RemoveClip { clip_id, .. } => {
                if !doc.clip_pool.contains_key(clip_id) {
                    return Err(ModelError::ClipNotFound { id: *clip_id });
                }
                let placement_count = doc
                    .tracks
                    .values()
                    .flat_map(|track| track.clips.values())
                    .filter(|placement| placement.clip_id == *clip_id)
                    .count();
                if placement_count > 0 {
                    return Err(ModelError::ClipInUse {
                        clip_id: *clip_id,
                        placement_count,
                    });
                }
                Ok(())
            }
            Self::AddRoutingNode { node } => {
                if doc.routing_graph.nodes.contains(node) {
                    return Err(ModelError::DuplicateEntityId { id: *node });
                }
                Ok(())
            }
            Self::RemoveRoutingNode { node } => {
                if !doc.routing_graph.nodes.contains(node) {
                    return Err(ModelError::RoutingNodeNotFound { id: *node });
                }
                let edge_count = doc
                    .routing_graph
                    .edges
                    .values()
                    .filter(|edge| edge.source_node == *node || edge.destination_node == *node)
                    .count();
                if edge_count > 0 {
                    return Err(ModelError::RoutingNodeInUse {
                        node: *node,
                        edge_count,
                    });
                }
                Ok(())
            }
            Self::ConnectRouting { edge } => {
                edge.validate()?;
                for endpoint in [edge.source_node, edge.destination_node] {
                    if !doc.routing_graph.nodes.contains(&endpoint) {
                        return Err(ModelError::RoutingNodeNotFound { id: endpoint });
                    }
                }
                if doc.routing_graph.edges.contains_key(&edge.id) {
                    return Err(ModelError::DuplicateEntityId { id: edge.id });
                }
                Ok(())
            }
            Self::DisconnectRouting {
                edge_id,
                previous_edge,
            } => {
                let current = doc
                    .routing_graph
                    .edges
                    .get(edge_id)
                    .ok_or(ModelError::RoutingEdgeNotFound { id: *edge_id })?;
                if current != previous_edge {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetRoutingGain {
                edge_id,
                old_gain_db,
                new_gain_db,
            } => {
                let current = doc
                    .routing_graph
                    .edges
                    .get(edge_id)
                    .ok_or(ModelError::RoutingEdgeNotFound { id: *edge_id })?;
                if !same_gain(current.gain_db, *old_gain_db) {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                if let Some(gain_db) = new_gain_db
                    && !gain_db.is_finite()
                {
                    return Err(ModelError::NonFiniteValue {
                        field: "routing.edge.gain_db",
                        value: f64::from(*gain_db),
                    });
                }
                Ok(())
            }
            Self::InsertDevice {
                track_id,
                slot_index,
                device,
            } => {
                let track = doc.track(track_id)?;
                if *slot_index > track.devices.len() {
                    return Err(ModelError::DeviceSlotOutOfRange {
                        index: *slot_index,
                        len: track.devices.len(),
                    });
                }
                device.validate()?;
                if track
                    .devices
                    .iter()
                    .any(|existing| existing.id == device.id)
                {
                    return Err(ModelError::DuplicateEntityId { id: device.id });
                }
                Ok(())
            }
            Self::RemoveDevice {
                track_id,
                slot_index,
                previous_device,
            } => {
                let track = doc.track(track_id)?;
                let current =
                    track
                        .devices
                        .get(*slot_index)
                        .ok_or(ModelError::DeviceSlotOutOfRange {
                            index: *slot_index,
                            len: track.devices.len(),
                        })?;
                if current != previous_device {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetParam {
                target,
                old_val,
                new_val,
            } => {
                let current = read_param(doc, *target)?;
                if !same_f32(current, *old_val) {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                validate_param_value(*target, *new_val)
            }
            // 与 `SetParam` 同型：先校验"载荷里的旧值确实是文档现值"，否则 `OpStateMismatch`。
            // `new_* == old_*` 是**合法**的无操作（`SetParam` 也放行）—— 幂等由调用方决定
            // 是否要提交（GUI 侧在值没变时**不**提交，见 `host::end_mixer_drag`）。
            Self::SetTrackMute {
                track_id, old_mute, ..
            } => {
                if doc.track(track_id)?.mute != *old_mute {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetTrackSolo {
                track_id, old_solo, ..
            } => {
                if doc.track(track_id)?.solo != *old_solo {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetMacro {
                track_id,
                macro_index,
                old_val,
                new_val,
            } => {
                let current = read_macro(doc, track_id, *macro_index)?;
                if !same_f32(current, *old_val) {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                validate_macro_value(*new_val)
            }
            Self::SetAutomationPoint {
                target,
                point_id,
                old_point,
                new_point,
            } => {
                if new_point.id != *point_id {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                new_point.validate()?;
                let current = read_automation_point(doc, target, point_id)?;
                if current != old_point.as_ref() {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::RemoveAutomationPoint {
                target,
                point_id,
                previous_point,
            } => {
                let current = read_automation_point(doc, target, point_id)?
                    .ok_or(ModelError::AutomationPointNotFound { id: *point_id })?;
                if current != previous_point {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetAutomationLane {
                target,
                old_lane,
                new_lane,
            } => {
                if new_lane.target != *target {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                // 与"隐式泳道"逐位不可区分的泳道**不允许被显式写入**：
                // 它的存亡由采样点决定（`SetAutomationPoint` 自动建、`RemoveAutomationPoint`
                // 自动回收），一旦允许显式创建，撤销就无法判定它该不该存在
                // （实测：该泳道被点填满又清空后会被自动回收，于是它的逆操作
                //  `RemoveAutomationLane` 找不到泳道而失败）。
                if new_lane.is_implicit() {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                doc.track(&target.track_id())?;
                new_lane.validate()?;
                if doc.automation_lane(target) != old_lane.as_ref() {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::RemoveAutomationLane {
                target,
                previous_lane,
            } => {
                doc.track(&target.track_id())?;
                if previous_lane.is_implicit() {
                    // 同上的镜像：隐式形状的泳道不由本变体负责（它压根不该被持久化）。
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                let current = doc
                    .automation_lane(target)
                    .ok_or(ModelError::OpStateMismatch { op: self.name() })?;
                if current != previous_lane {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetSection {
                section_id,
                old_section,
                new_section,
            } => {
                if new_section.id != *section_id {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                new_section.validate()?;
                if doc.sections.get(section_id) != old_section.as_ref() {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::RemoveSection {
                section_id,
                previous_section,
            } => {
                let current = doc
                    .sections
                    .get(section_id)
                    .ok_or(ModelError::SectionNotFound { id: *section_id })?;
                if current != previous_section {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetScene {
                scene_id,
                old_scene,
                new_scene,
            } => {
                if new_scene.id != *scene_id {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                new_scene.validate()?;
                if doc.scenes.get(scene_id) != old_scene.as_ref() {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::RemoveScene {
                scene_id,
                previous_scene,
            } => {
                let current = doc
                    .scenes
                    .get(scene_id)
                    .ok_or(ModelError::SceneNotFound { id: *scene_id })?;
                if current != previous_scene {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            // 批量的前置条件刻意是"真空真"：原子性由 `commit` 在克隆体上
            // 整体模拟成功后一次性提交来保证（见 `Op::commit`），
            // 在这里重复模拟只会让每次 `apply` 多做一次全文档克隆。
            Self::Batch { .. } => Ok(()),
        }
    }

    /// 应用本操作。
    ///
    /// 先做完整的前置条件检查，再落盘；`Batch` 在克隆体上整体模拟成功后
    /// **一次性提交**，因此天然是原子的（`ARCH-OPS-002` 的"AI 提案一键撤销"）。
    ///
    /// # Errors
    ///
    /// 前置条件不成立时返回对应 [`ModelError`]，且**不改变**文档。
    pub fn apply(&self, doc: &mut YebanProjectV1) -> Result<(), ModelError> {
        self.precondition(doc)?;
        self.commit(doc)
    }

    /// 构造本操作的逆操作。
    ///
    /// `doc` 用于校验"本操作确实已经作用在这份文档上"（例如 `DeleteNote`
    /// 要求音符此刻**不存在**、`AddTrack` 要求音轨此刻**存在**且内容一致）。
    /// 校验失败即返回错误，绝不把撤销打到错误的文档上。
    ///
    /// # Errors
    ///
    /// - 逆操作在给定文档上不可应用 → 返回对应的具体 [`ModelError`]；
    /// - 增量取反溢出 (`i64::MIN` / `i8::MIN`) → [`ModelError::OpStateMismatch`]。
    pub fn invert(&self, doc: &YebanProjectV1) -> Result<Self, ModelError> {
        let inverse = self.structural_inverse()?;
        // `Batch` 的子操作后置条件由逆批次自身的 `apply` 逐步校验（见 `precondition`）。
        if !matches!(self, Self::Batch { .. }) {
            inverse.precondition(doc)?;
        }
        Ok(inverse)
    }

    /// 应用逆操作（等价于 `self.invert(doc)?.apply(doc)`）。
    ///
    /// # Errors
    ///
    /// 逆操作不可构造或不可应用时返回对应 [`ModelError`]。
    pub fn apply_inverse(&self, doc: &mut YebanProjectV1) -> Result<(), ModelError> {
        let inverse = self.invert(doc)?;
        inverse.apply(doc)
    }

    /// 纯结构化逆操作：只使用本操作自带的载荷，不读文档。
    fn structural_inverse(&self) -> Result<Self, ModelError> {
        let inverse = match self {
            Self::AddNote {
                track_id,
                clip_id,
                note,
            } => Self::DeleteNote {
                track_id: *track_id,
                clip_id: *clip_id,
                note_id: note.id,
                previous_note: note.clone(),
            },
            Self::DeleteNote {
                track_id,
                clip_id,
                previous_note,
                ..
            } => Self::AddNote {
                track_id: *track_id,
                clip_id: *clip_id,
                note: previous_note.clone(),
            },
            Self::MoveNote {
                track_id,
                clip_id,
                note_id,
                delta_tick,
                delta_pitch,
            } => Self::MoveNote {
                track_id: *track_id,
                clip_id: *clip_id,
                note_id: *note_id,
                delta_tick: delta_tick
                    .checked_neg()
                    .ok_or(ModelError::OpStateMismatch { op: self.name() })?,
                delta_pitch: delta_pitch
                    .checked_neg()
                    .ok_or(ModelError::OpStateMismatch { op: self.name() })?,
            },
            Self::ModifyNoteVelocity {
                track_id,
                clip_id,
                note_id,
                old_vel,
                new_vel,
            } => Self::ModifyNoteVelocity {
                track_id: *track_id,
                clip_id: *clip_id,
                note_id: *note_id,
                old_vel: *new_vel,
                new_vel: *old_vel,
            },
            Self::AddClipPlacement {
                track_id,
                placement,
            } => Self::RemoveClipPlacement {
                track_id: *track_id,
                placement_id: placement.id,
                previous_placement: *placement,
            },
            Self::RemoveClipPlacement {
                track_id,
                previous_placement,
                ..
            } => Self::AddClipPlacement {
                track_id: *track_id,
                placement: *previous_placement,
            },
            Self::MoveClipPlacement {
                track_id,
                placement_id,
                old_start_tick,
                new_start_tick,
            } => Self::MoveClipPlacement {
                track_id: *track_id,
                placement_id: *placement_id,
                old_start_tick: *new_start_tick,
                new_start_tick: *old_start_tick,
            },
            Self::AddTrack { track } => Self::RemoveTrack {
                track_id: track.id,
                previous_track: track.clone(),
            },
            Self::RemoveTrack { previous_track, .. } => Self::AddTrack {
                track: previous_track.clone(),
            },
            Self::AddClip { clip } => Self::RemoveClip {
                clip_id: clip.id,
                // `ClipPoolEntry` 含 `String`/`BTreeMap`, 不是 `Copy` ⇒ 逆操作必须克隆
                // (与 `ClipPlacement` 那类 `Copy` 载荷不同)。
                previous_clip: clip.clone(),
            },
            Self::RemoveClip { previous_clip, .. } => Self::AddClip {
                clip: previous_clip.clone(),
            },
            Self::AddRoutingNode { node } => Self::RemoveRoutingNode { node: *node },
            Self::RemoveRoutingNode { node } => Self::AddRoutingNode { node: *node },
            Self::ConnectRouting { edge } => Self::DisconnectRouting {
                edge_id: edge.id,
                previous_edge: *edge,
            },
            Self::DisconnectRouting { previous_edge, .. } => Self::ConnectRouting {
                edge: *previous_edge,
            },
            Self::SetRoutingGain {
                edge_id,
                old_gain_db,
                new_gain_db,
            } => Self::SetRoutingGain {
                edge_id: *edge_id,
                old_gain_db: *new_gain_db,
                new_gain_db: *old_gain_db,
            },
            Self::InsertDevice {
                track_id,
                slot_index,
                device,
            } => Self::RemoveDevice {
                track_id: *track_id,
                slot_index: *slot_index,
                previous_device: device.clone(),
            },
            Self::RemoveDevice {
                track_id,
                slot_index,
                previous_device,
            } => Self::InsertDevice {
                track_id: *track_id,
                slot_index: *slot_index,
                device: previous_device.clone(),
            },
            Self::SetParam {
                target,
                old_val,
                new_val,
            } => Self::SetParam {
                target: *target,
                old_val: *new_val,
                new_val: *old_val,
            },
            // 纯结构化取反：只交换"前 / 后"两个布尔。
            Self::SetTrackMute {
                track_id,
                old_mute,
                new_mute,
            } => Self::SetTrackMute {
                track_id: *track_id,
                old_mute: *new_mute,
                new_mute: *old_mute,
            },
            Self::SetTrackSolo {
                track_id,
                old_solo,
                new_solo,
            } => Self::SetTrackSolo {
                track_id: *track_id,
                old_solo: *new_solo,
                new_solo: *old_solo,
            },
            Self::SetMacro {
                track_id,
                macro_index,
                old_val,
                new_val,
            } => Self::SetMacro {
                track_id: *track_id,
                macro_index: *macro_index,
                old_val: *new_val,
                new_val: *old_val,
            },
            Self::SetAutomationPoint {
                target,
                point_id,
                old_point,
                new_point,
            } => match old_point {
                Some(previous) => Self::SetAutomationPoint {
                    target: *target,
                    point_id: *point_id,
                    old_point: Some(*new_point),
                    new_point: *previous,
                },
                None => Self::RemoveAutomationPoint {
                    target: *target,
                    point_id: *point_id,
                    previous_point: *new_point,
                },
            },
            Self::RemoveAutomationPoint {
                target,
                point_id,
                previous_point,
            } => Self::SetAutomationPoint {
                target: *target,
                point_id: *point_id,
                old_point: None,
                new_point: *previous_point,
            },
            Self::SetAutomationLane {
                target,
                old_lane,
                new_lane,
            } => match old_lane {
                Some(previous) => Self::SetAutomationLane {
                    target: *target,
                    old_lane: Some(new_lane.clone()),
                    new_lane: previous.clone(),
                },
                None => Self::RemoveAutomationLane {
                    target: *target,
                    previous_lane: new_lane.clone(),
                },
            },
            Self::RemoveAutomationLane {
                target,
                previous_lane,
            } => Self::SetAutomationLane {
                target: *target,
                old_lane: None,
                new_lane: previous_lane.clone(),
            },
            Self::SetSection {
                section_id,
                old_section,
                new_section,
            } => match old_section {
                Some(previous) => Self::SetSection {
                    section_id: *section_id,
                    old_section: Some(new_section.clone()),
                    new_section: previous.clone(),
                },
                None => Self::RemoveSection {
                    section_id: *section_id,
                    previous_section: new_section.clone(),
                },
            },
            Self::RemoveSection {
                section_id,
                previous_section,
            } => Self::SetSection {
                section_id: *section_id,
                old_section: None,
                new_section: previous_section.clone(),
            },
            Self::SetScene {
                scene_id,
                old_scene,
                new_scene,
            } => match old_scene {
                Some(previous) => Self::SetScene {
                    scene_id: *scene_id,
                    old_scene: Some(new_scene.clone()),
                    new_scene: previous.clone(),
                },
                None => Self::RemoveScene {
                    scene_id: *scene_id,
                    previous_scene: new_scene.clone(),
                },
            },
            Self::RemoveScene {
                scene_id,
                previous_scene,
            } => Self::SetScene {
                scene_id: *scene_id,
                old_scene: None,
                new_scene: previous_scene.clone(),
            },
            Self::Batch { ops, description } => {
                let mut inverted = Vec::with_capacity(ops.len());
                for op in ops.iter().rev() {
                    inverted.push(op.structural_inverse()?);
                }
                Self::Batch {
                    ops: inverted,
                    description: description.clone(),
                }
            }
        };
        Ok(inverse)
    }

    /// 落盘：前置条件已成立，这里只做结构变更。
    fn commit(&self, doc: &mut YebanProjectV1) -> Result<(), ModelError> {
        match self {
            Self::AddClip { clip } => {
                doc.clip_pool.insert(clip.id, clip.clone());
                Ok(())
            }
            Self::RemoveClip { clip_id, .. } => {
                doc.clip_pool.remove(clip_id);
                Ok(())
            }
            Self::AddRoutingNode { node } => {
                let nodes = &mut doc.routing_graph.nodes;
                // 按字典序插入 ⇒ `nodes` 恒有序, 于是增删互为逆且无需载荷。
                let position = nodes.partition_point(|existing| existing < node);
                nodes.insert(position, *node);
                Ok(())
            }
            Self::RemoveRoutingNode { node } => {
                doc.routing_graph.nodes.retain(|existing| existing != node);
                Ok(())
            }
            Self::AddNote { clip_id, note, .. } => doc.insert_note(clip_id, note.clone()),
            Self::DeleteNote {
                clip_id, note_id, ..
            } => doc.remove_note(clip_id, note_id).map(|_removed| ()),
            Self::MoveNote {
                clip_id,
                note_id,
                delta_tick,
                delta_pitch,
                ..
            } => {
                let note = doc.note_mut(clip_id, note_id)?;
                if let Some(start_tick) = shifted_tick(note.start_tick, *delta_tick) {
                    note.start_tick = start_tick;
                }
                if let Some(pitch) = shifted_pitch(note.pitch, *delta_pitch) {
                    note.pitch = pitch;
                }
                Ok(())
            }
            Self::ModifyNoteVelocity {
                clip_id,
                note_id,
                new_vel,
                ..
            } => {
                doc.note_mut(clip_id, note_id)?.velocity = *new_vel;
                Ok(())
            }
            Self::AddClipPlacement {
                track_id,
                placement,
            } => {
                doc.track_mut(track_id)?
                    .clips
                    .insert(placement.id, *placement);
                Ok(())
            }
            Self::RemoveClipPlacement {
                track_id,
                placement_id,
                ..
            } => {
                doc.track_mut(track_id)?.clips.remove(placement_id);
                Ok(())
            }
            Self::MoveClipPlacement {
                track_id,
                placement_id,
                new_start_tick,
                ..
            } => {
                if let Some(placement) = doc.track_mut(track_id)?.clips.get_mut(placement_id) {
                    placement.start_tick = *new_start_tick;
                }
                Ok(())
            }
            Self::AddTrack { track } => doc.insert_track(track.clone()),
            Self::RemoveTrack { track_id, .. } => doc.remove_track(track_id).map(|_removed| ()),
            Self::ConnectRouting { edge } => {
                let graph = &mut doc.routing_graph;
                if graph.edges.insert(edge.id, *edge).is_none() {
                    Ok(())
                } else {
                    Err(ModelError::DuplicateEntityId { id: edge.id })
                }
            }
            Self::DisconnectRouting { edge_id, .. } => {
                doc.routing_graph.edges.remove(edge_id);
                Ok(())
            }
            Self::SetRoutingGain {
                edge_id,
                new_gain_db,
                ..
            } => {
                if let Some(edge) = doc.routing_graph.edges.get_mut(edge_id) {
                    edge.gain_db = *new_gain_db;
                }
                Ok(())
            }
            Self::InsertDevice {
                track_id,
                slot_index,
                device,
            } => {
                doc.track_mut(track_id)?
                    .devices
                    .insert(*slot_index, device.clone());
                Ok(())
            }
            Self::RemoveDevice {
                track_id,
                slot_index,
                ..
            } => {
                doc.track_mut(track_id)?.devices.remove(*slot_index);
                Ok(())
            }
            Self::SetParam {
                target, new_val, ..
            } => write_param(doc, *target, *new_val),
            // 只写那**一个**布尔字段：不碰 `solo_safe`、不碰音量、不碰路由。
            Self::SetTrackMute {
                track_id, new_mute, ..
            } => {
                doc.track_mut(track_id)?.mute = *new_mute;
                Ok(())
            }
            Self::SetTrackSolo {
                track_id, new_solo, ..
            } => {
                doc.track_mut(track_id)?.solo = *new_solo;
                Ok(())
            }
            Self::SetMacro {
                track_id,
                macro_index,
                new_val,
                ..
            } => {
                let track = doc.track_mut(track_id)?;
                if let Some(macro_parameter) = track.macros.get_mut(*macro_index) {
                    macro_parameter.value = *new_val;
                }
                Ok(())
            }
            Self::SetAutomationPoint {
                target,
                point_id,
                new_point,
                ..
            } => {
                let track = doc.track_mut(&target.track_id())?;
                let lane = track
                    .automation_lanes
                    .entry(*target)
                    .or_insert_with(|| AutomationLane::implicit(*target));
                lane.points.insert(*point_id, *new_point);
                Ok(())
            }
            Self::RemoveAutomationPoint {
                target, point_id, ..
            } => {
                let track = doc.track_mut(&target.track_id())?;
                let mut drop_lane = false;
                if let Some(lane) = track.automation_lanes.get_mut(target) {
                    lane.points.remove(point_id);
                    // 只回收"与隐式泳道逐位不可区分"的泳道 —— 这正是
                    // `SetAutomationPoint` 自动创建的那一种，于是自动建/自动收**精确互逆**。
                    // 带读写模式或取值域的泳道即使空着也保留（否则它的属性会在
                    // "移空最后一个点 → 撤销"这一步丢失）。
                    drop_lane = lane.is_implicit();
                }
                if drop_lane {
                    track.automation_lanes.remove(target);
                }
                Ok(())
            }
            Self::SetAutomationLane {
                target, new_lane, ..
            } => {
                doc.track_mut(&target.track_id())?
                    .automation_lanes
                    .insert(*target, new_lane.clone());
                Ok(())
            }
            Self::RemoveAutomationLane { target, .. } => {
                doc.track_mut(&target.track_id())?
                    .automation_lanes
                    .remove(target);
                Ok(())
            }
            Self::SetSection {
                section_id,
                new_section,
                ..
            } => {
                doc.sections.insert(*section_id, new_section.clone());
                Ok(())
            }
            Self::RemoveSection { section_id, .. } => {
                doc.sections.remove(section_id);
                Ok(())
            }
            Self::SetScene {
                scene_id,
                new_scene,
                ..
            } => {
                doc.scenes.insert(*scene_id, new_scene.clone());
                Ok(())
            }
            Self::RemoveScene { scene_id, .. } => {
                doc.scenes.remove(scene_id);
                Ok(())
            }
            Self::Batch { ops, .. } => {
                // 原子性：在克隆体上整体应用成功后再一次性提交。
                // 任何子操作失败都不会污染 `doc`（`ARCH-OPS-002` 的原子回滚语义）。
                let mut probe = doc.clone();
                for op in ops {
                    op.apply(&mut probe)?;
                }
                *doc = probe;
                Ok(())
            }
        }
    }
}

/// 读取 [`AutomationTarget`] 指向的当前参数值。
pub(crate) fn read_param(
    doc: &YebanProjectV1,
    target: AutomationTarget,
) -> Result<f32, ModelError> {
    match target {
        AutomationTarget::TrackVolume { track_id } => Ok(doc.track(&track_id)?.volume_db),
        AutomationTarget::TrackPan { track_id } => Ok(doc.track(&track_id)?.pan),
        AutomationTarget::SendGain { .. } => Err(ModelError::AutomationTargetNotApplicable {
            detail: "send gain must be edited via Op::SetRoutingGain to preserve Option semantics",
        }),
        AutomationTarget::DeviceParam {
            track_id,
            slot_index,
            param_index,
        } => {
            let track = doc.track(&track_id)?;
            let device = track
                .devices
                .get(slot_index)
                .ok_or(ModelError::DeviceSlotOutOfRange {
                    index: slot_index,
                    len: track.devices.len(),
                })?;
            let param = device
                .params
                .get(param_index)
                .ok_or(ModelError::ParamIndexOutOfRange {
                    index: param_index,
                    len: device.params.len(),
                })?;
            Ok(param.value)
        }
        AutomationTarget::Macro {
            track_id,
            macro_index,
        } => read_macro(doc, &track_id, macro_index),
    }
}

/// 写入 [`AutomationTarget`] 指向的参数值。
fn write_param(
    doc: &mut YebanProjectV1,
    target: AutomationTarget,
    value: f32,
) -> Result<(), ModelError> {
    match target {
        AutomationTarget::TrackVolume { track_id } => {
            doc.track_mut(&track_id)?.volume_db = value;
            Ok(())
        }
        AutomationTarget::TrackPan { track_id } => {
            doc.track_mut(&track_id)?.pan = value;
            Ok(())
        }
        AutomationTarget::SendGain { .. } => Err(ModelError::AutomationTargetNotApplicable {
            detail: "send gain must be edited via Op::SetRoutingGain to preserve Option semantics",
        }),
        AutomationTarget::DeviceParam {
            track_id,
            slot_index,
            param_index,
        } => {
            let track = doc.track_mut(&track_id)?;
            let slot_count = track.devices.len();
            let device =
                track
                    .devices
                    .get_mut(slot_index)
                    .ok_or(ModelError::DeviceSlotOutOfRange {
                        index: slot_index,
                        len: slot_count,
                    })?;
            let param_count = device.params.len();
            let param =
                device
                    .params
                    .get_mut(param_index)
                    .ok_or(ModelError::ParamIndexOutOfRange {
                        index: param_index,
                        len: param_count,
                    })?;
            param.value = value;
            Ok(())
        }
        AutomationTarget::Macro {
            track_id,
            macro_index,
        } => {
            let track = doc.track_mut(&track_id)?;
            let macro_count = track.macros.len();
            let macro_parameter =
                track
                    .macros
                    .get_mut(macro_index)
                    .ok_or(ModelError::MacroIndexOutOfRange {
                        index: macro_index,
                        len: macro_count,
                    })?;
            macro_parameter.value = value;
            Ok(())
        }
    }
}

/// 校验参数写入值是否落在该目标允许的范围内。
fn validate_param_value(target: AutomationTarget, value: f32) -> Result<(), ModelError> {
    if !value.is_finite() {
        return Err(ModelError::NonFiniteValue {
            field: "param.value",
            value: f64::from(value),
        });
    }
    match target {
        AutomationTarget::TrackPan { .. } => {
            if !(-1.0..=1.0).contains(&value) {
                return Err(ModelError::PanOutOfRange { value });
            }
            Ok(())
        }
        AutomationTarget::Macro { .. } => validate_macro_value(value),
        _ => Ok(()),
    }
}

/// 读取宏位置。
fn read_macro(
    doc: &YebanProjectV1,
    track_id: &EntityId,
    macro_index: usize,
) -> Result<f32, ModelError> {
    let track = doc.track(track_id)?;
    track
        .macros
        .get(macro_index)
        .map(|macro_parameter| macro_parameter.value)
        .ok_or(ModelError::MacroIndexOutOfRange {
            index: macro_index,
            len: track.macros.len(),
        })
}

/// 校验宏位置范围。
fn validate_macro_value(value: f32) -> Result<(), ModelError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ModelError::MacroValueOutOfRange { value });
    }
    Ok(())
}

/// 读取自动化点（`Ok(None)` 表示该目标/点当前不存在）。
fn read_automation_point<'a>(
    doc: &'a YebanProjectV1,
    target: &AutomationTarget,
    point_id: &EntityId,
) -> Result<Option<&'a AutomationPoint>, ModelError> {
    // 音轨必须存在（`TrackNotFound`），否则"点不存在"会掩盖"目标根本不存在"。
    doc.track(&target.track_id())?;
    Ok(doc
        .automation_lane(target)
        .and_then(|lane| lane.points.get(point_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::splitmix64;
    use crate::project::{
        ClipContent, ClipPoolEntry, LoopConfig, ProjectAudioConfig, RoutingKind, TrackKind,
    };
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use std::str::FromStr;

    /// 本机默认的操作序列长度（CI 上自动提到 [`CI_SEQUENCE_STEPS`]）。
    const LOCAL_SEQUENCE_STEPS: usize = 256;

    /// CI 上的操作序列长度（`MUST-GATE-010` 要求 10,000 步）。
    const CI_SEQUENCE_STEPS: usize = 10_000;

    /// 构造确定性的规范 ULID 文本。
    fn fixture_id(index: u128) -> EntityId {
        EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
    }

    /// 操作序列长度：`YEBAN_PROPTEST_CASES` > `CI` > 本机默认。
    fn sequence_steps() -> usize {
        if let Ok(raw) = std::env::var("YEBAN_PROPTEST_CASES")
            && let Ok(parsed) = raw.parse::<usize>()
            && parsed > 0
        {
            return parsed;
        }
        if std::env::var_os("CI").is_some() {
            CI_SEQUENCE_STEPS
        } else {
            LOCAL_SEQUENCE_STEPS
        }
    }

    /// 固定夹具用的身份常量。
    struct Fixture {
        master: EntityId,
        lead: EntityId,
        bass: EntityId,
        clip: EntityId,
        audio_clip: EntityId,
        note: EntityId,
        edge: EntityId,
        device: EntityId,
        placement: EntityId,
        section: EntityId,
        scene: EntityId,
    }

    fn fixture() -> Fixture {
        Fixture {
            master: fixture_id(1),
            lead: fixture_id(2),
            bass: fixture_id(3),
            clip: fixture_id(10),
            audio_clip: fixture_id(11),
            note: fixture_id(20),
            edge: fixture_id(30),
            device: fixture_id(40),
            placement: fixture_id(50),
            section: fixture_id(60),
            scene: fixture_id(70),
        }
    }

    fn empty_midi_clip(id: EntityId) -> ClipPoolEntry {
        ClipPoolEntry {
            id,
            name: "Clip".to_owned(),
            content: ClipContent::default(),
        }
    }

    /// ops 夹具里那条音量泳道的**完整形状**。
    ///
    /// `showcase_ops()` 的 `RemoveAutomationLane` 必须携带与它逐位相同的撤销载荷，
    /// 因此这里只留一份定义（两份必然漂移）。
    fn fixture_volume_lane(lead_id: EntityId) -> AutomationLane {
        let target = AutomationTarget::TrackVolume { track_id: lead_id };
        AutomationLane {
            target,
            points: BTreeMap::from([(
                fixture_id(80),
                AutomationPoint {
                    id: fixture_id(80),
                    tick: 0,
                    value: -6.0,
                    curve: crate::music::CurveType::Linear,
                },
            )]),
            ..AutomationLane::implicit(target)
        }
    }

    /// 一份"五脏俱全"但体积很小的文档，供属性测试与逐变体测试使用。
    fn fixture_document() -> YebanProjectV1 {
        let f = fixture();
        let mut lead = TrackV3 {
            id: f.lead,
            name: "Lead".to_owned(),
            kind: TrackKind::Midi,
            volume_db: -3.0,
            pan: -0.25,
            ..TrackV3::default()
        };
        lead.devices.push(DeviceDefinition {
            id: f.device,
            name: "PolySynth".to_owned(),
            params: vec![
                crate::project::ParameterValue {
                    name: "cutoff".to_owned(),
                    value: 1200.0,
                    unit: Some("Hz".to_owned()),
                },
                crate::project::ParameterValue {
                    name: "reso".to_owned(),
                    value: 0.3,
                    unit: None,
                },
            ],
            ..DeviceDefinition::default()
        });
        lead.macros.push(crate::project::MacroParameter {
            name: "Brightness".to_owned(),
            value: 0.5,
            ..crate::project::MacroParameter::default()
        });
        lead.automation_lanes.insert(
            AutomationTarget::TrackVolume { track_id: f.lead },
            fixture_volume_lane(f.lead),
        );
        lead.clips.insert(
            f.placement,
            ClipPlacement {
                id: f.placement,
                clip_id: f.clip,
                start_tick: 0,
                duration_ticks: 3840,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );

        let mut clip = empty_midi_clip(f.clip);
        if let Some(notes) = clip.content.notes_mut() {
            notes.insert(f.note, MidiNote::new(f.note, 0, 60, 480));
            let second = fixture_id(21);
            notes.insert(second, MidiNote::new(second, 960, 64, 480));
        }

        let audio_clip = ClipPoolEntry {
            id: f.audio_clip,
            name: "Kick".to_owned(),
            content: ClipContent::Audio {
                asset: crate::ids::AssetHash::of_bytes(b"kick"),
                gain_db: 0.0,
            },
        };

        let project = YebanProjectV1 {
            audio_config: ProjectAudioConfig::default(),
            id: fixture_id(999),
            title: "Ops Fixture".to_owned(),
            tracks: BTreeMap::from([
                (
                    f.master,
                    TrackV3 {
                        id: f.master,
                        name: "Master".to_owned(),
                        kind: TrackKind::Master,
                        ..TrackV3::default()
                    },
                ),
                (f.lead, lead),
                (
                    f.bass,
                    TrackV3 {
                        id: f.bass,
                        name: "Bass".to_owned(),
                        kind: TrackKind::Audio,
                        ..TrackV3::default()
                    },
                ),
            ]),
            master_bus_track_id: f.master,
            routing_graph: crate::project::RoutingGraph {
                nodes: vec![f.master, f.lead, f.bass],
                edges: BTreeMap::from([(
                    f.edge,
                    RoutingEdge {
                        id: f.edge,
                        source_node: f.lead,
                        destination_node: f.master,
                        kind: RoutingKind::TrackToBus,
                        gain_db: None,
                    },
                )]),
            },
            sections: BTreeMap::from([(
                f.section,
                SectionV3 {
                    id: f.section,
                    name: "Intro".to_owned(),
                    start_tick: 0,
                    end_tick: 3840,
                    color: None,
                },
            )]),
            scenes: BTreeMap::from([(
                f.scene,
                SceneV3 {
                    id: f.scene,
                    name: "Scene 1".to_owned(),
                    tempo: None,
                    color: None,
                },
            )]),
            clip_pool: BTreeMap::from([(f.clip, clip), (f.audio_clip, audio_clip)]),
            ..YebanProjectV1::default()
        };
        project.validate().expect("夹具文档必须合法");
        project
    }

    /// 覆盖**每一个**变体的操作脚本（含 `Batch`）。
    ///
    /// 每个操作都针对同一个初始文档构造，测试逐个"应用 → 求逆 → 撤销"，
    /// 因此脚本可以一次性生成。
    fn showcase_ops() -> Vec<Op> {
        let f = fixture();
        let fresh_note = fixture_id(500);
        let fresh_track = fixture_id(501);
        let fresh_placement = fixture_id(502);
        let fresh_edge = fixture_id(503);
        let fresh_device = fixture_id(504);
        let fresh_point = fixture_id(505);
        let fresh_section = fixture_id(506);
        let fresh_scene = fixture_id(507);

        let new_placement = ClipPlacement {
            id: fresh_placement,
            clip_id: f.clip,
            start_tick: 1920,
            duration_ticks: 960,
            loop_config: LoopConfig::default(),
            muted: false,
        };
        let new_edge = RoutingEdge {
            id: fresh_edge,
            source_node: f.bass,
            destination_node: f.master,
            kind: RoutingKind::TrackToBus,
            gain_db: Some(-6.0),
        };
        let new_device = DeviceDefinition {
            id: fresh_device,
            name: "Insert".to_owned(),
            ..DeviceDefinition::default()
        };
        let new_point = AutomationPoint {
            id: fresh_point,
            tick: 1920,
            value: 0.0,
            curve: crate::music::CurveType::Linear,
        };
        let new_section = SectionV3 {
            id: fresh_section,
            name: "Drop".to_owned(),
            start_tick: 3840,
            end_tick: 7680,
            color: None,
        };
        let new_scene = SceneV3 {
            id: fresh_scene,
            name: "Scene 2".to_owned(),
            tempo: Some(140.0),
            color: None,
        };
        let target_volume = AutomationTarget::TrackVolume { track_id: f.lead };
        let target_pan = AutomationTarget::TrackPan { track_id: f.lead };
        let target_param = AutomationTarget::DeviceParam {
            track_id: f.lead,
            slot_index: 0,
            param_index: 0,
        };
        // 新建一条**显式**泳道（带写模式与取值域 ⇒ 与隐式泳道可区分）。
        let new_lane = AutomationLane {
            target: target_pan,
            read_enabled: true,
            write_mode: crate::project::AutomationWriteMode::Touch,
            domain: Some(
                crate::project::AutomationValueDomain::new(-24.0, 6.0).expect("常量端点必然有限"),
            ),
            ..AutomationLane::implicit(target_pan)
        };

        vec![
            Op::AddNote {
                track_id: f.lead,
                clip_id: f.clip,
                note: MidiNote::new(fresh_note, 480, 67, 240),
            },
            Op::DeleteNote {
                track_id: f.lead,
                clip_id: f.clip,
                note_id: f.note,
                previous_note: MidiNote::new(f.note, 0, 60, 480),
            },
            Op::MoveNote {
                track_id: f.lead,
                clip_id: f.clip,
                note_id: f.note,
                delta_tick: 240,
                delta_pitch: 2,
            },
            Op::ModifyNoteVelocity {
                track_id: f.lead,
                clip_id: f.clip,
                note_id: f.note,
                old_vel: 100,
                new_vel: 64,
            },
            Op::AddClipPlacement {
                track_id: f.bass,
                placement: new_placement,
            },
            Op::RemoveClipPlacement {
                track_id: f.lead,
                placement_id: f.placement,
                previous_placement: ClipPlacement {
                    id: f.placement,
                    clip_id: f.clip,
                    start_tick: 0,
                    duration_ticks: 3840,
                    loop_config: LoopConfig::default(),
                    muted: false,
                },
            },
            Op::MoveClipPlacement {
                track_id: f.lead,
                placement_id: f.placement,
                old_start_tick: 0,
                new_start_tick: 960,
            },
            Op::AddTrack {
                track: TrackV3 {
                    id: fresh_track,
                    name: "Pad".to_owned(),
                    ..TrackV3::default()
                },
            },
            Op::RemoveTrack {
                track_id: f.bass,
                previous_track: TrackV3 {
                    id: f.bass,
                    name: "Bass".to_owned(),
                    kind: TrackKind::Audio,
                    ..TrackV3::default()
                },
            },
            Op::AddClip {
                clip: empty_midi_clip(fresh_placement),
            },
            Op::RemoveClip {
                clip_id: f.audio_clip,
                // 必须是夹具文档里**真实存在**的那一条(名字/内容/资产哈希都要对得上),
                // 否则"应用后取逆"还原不出原文档 —— 这正是 `every_variant_applies_and_inverts_exactly` 抓到的。
                previous_clip: ClipPoolEntry {
                    id: f.audio_clip,
                    name: "Kick".to_owned(),
                    content: ClipContent::Audio {
                        asset: crate::ids::AssetHash::of_bytes(b"kick"),
                        gain_db: 0.0,
                    },
                },
            },
            Op::AddRoutingNode {
                node: fresh_placement,
            },
            Op::RemoveRoutingNode {
                // `f.bass` 在夹具的 `routing_graph.nodes` 里, 且**没有任何边引用它**
                // (唯一的边是 lead → master) —— 这正是 `RemoveRoutingNode` 的前置条件。
                node: f.bass,
            },
            Op::ConnectRouting { edge: new_edge },
            Op::DisconnectRouting {
                edge_id: f.edge,
                previous_edge: RoutingEdge {
                    id: f.edge,
                    source_node: f.lead,
                    destination_node: f.master,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            },
            Op::SetRoutingGain {
                edge_id: f.edge,
                old_gain_db: None,
                new_gain_db: Some(-3.0),
            },
            Op::InsertDevice {
                track_id: f.bass,
                slot_index: 0,
                device: new_device.clone(),
            },
            Op::RemoveDevice {
                track_id: f.lead,
                slot_index: 0,
                previous_device: DeviceDefinition {
                    id: f.device,
                    name: "PolySynth".to_owned(),
                    params: vec![
                        crate::project::ParameterValue {
                            name: "cutoff".to_owned(),
                            value: 1200.0,
                            unit: Some("Hz".to_owned()),
                        },
                        crate::project::ParameterValue {
                            name: "reso".to_owned(),
                            value: 0.3,
                            unit: None,
                        },
                    ],
                    ..DeviceDefinition::default()
                },
            },
            Op::SetParam {
                target: target_volume,
                old_val: -3.0,
                new_val: -9.0,
            },
            Op::SetParam {
                target: target_param,
                old_val: 1200.0,
                new_val: 2400.0,
            },
            // 夹具的每条轨道 `mute` / `solo` 都是 `false`（`TrackV3::default`）⇒
            // 这两条的前置条件在 `fixture_document()` 上成立。
            Op::SetTrackMute {
                track_id: f.lead,
                old_mute: false,
                new_mute: true,
            },
            Op::SetTrackSolo {
                track_id: f.bass,
                old_solo: false,
                new_solo: true,
            },
            Op::SetMacro {
                track_id: f.lead,
                macro_index: 0,
                old_val: 0.5,
                new_val: 0.75,
            },
            Op::SetAutomationPoint {
                target: target_volume,
                point_id: fixture_id(80),
                old_point: Some(AutomationPoint {
                    id: fixture_id(80),
                    tick: 0,
                    value: -6.0,
                    curve: crate::music::CurveType::Linear,
                }),
                new_point: AutomationPoint {
                    id: fixture_id(80),
                    tick: 0,
                    value: -12.0,
                    curve: crate::music::CurveType::SCurve,
                },
            },
            Op::SetAutomationPoint {
                target: target_volume,
                point_id: fresh_point,
                old_point: None,
                new_point,
            },
            Op::RemoveAutomationPoint {
                target: target_volume,
                point_id: fixture_id(80),
                previous_point: AutomationPoint {
                    id: fixture_id(80),
                    tick: 0,
                    value: -6.0,
                    curve: crate::music::CurveType::Linear,
                },
            },
            Op::SetAutomationLane {
                target: target_pan,
                old_lane: None,
                new_lane: new_lane.clone(),
            },
            Op::RemoveAutomationLane {
                target: target_volume,
                previous_lane: fixture_volume_lane(f.lead),
            },
            Op::SetSection {
                section_id: f.section,
                old_section: Some(SectionV3 {
                    id: f.section,
                    name: "Intro".to_owned(),
                    start_tick: 0,
                    end_tick: 3840,
                    color: None,
                }),
                new_section: SectionV3 {
                    id: f.section,
                    name: "Intro A".to_owned(),
                    start_tick: 0,
                    end_tick: 1920,
                    color: None,
                },
            },
            Op::SetSection {
                section_id: fresh_section,
                old_section: None,
                new_section,
            },
            Op::RemoveSection {
                section_id: f.section,
                previous_section: SectionV3 {
                    id: f.section,
                    name: "Intro".to_owned(),
                    start_tick: 0,
                    end_tick: 3840,
                    color: None,
                },
            },
            Op::SetScene {
                scene_id: f.scene,
                old_scene: Some(SceneV3 {
                    id: f.scene,
                    name: "Scene 1".to_owned(),
                    tempo: None,
                    color: None,
                }),
                new_scene: SceneV3 {
                    id: f.scene,
                    name: "Scene 1".to_owned(),
                    tempo: Some(120.0),
                    color: None,
                },
            },
            Op::SetScene {
                scene_id: fresh_scene,
                old_scene: None,
                new_scene,
            },
            Op::RemoveScene {
                scene_id: f.scene,
                previous_scene: SceneV3 {
                    id: f.scene,
                    name: "Scene 1".to_owned(),
                    tempo: None,
                    color: None,
                },
            },
            Op::Batch {
                ops: vec![
                    Op::ModifyNoteVelocity {
                        track_id: f.lead,
                        clip_id: f.clip,
                        note_id: f.note,
                        old_vel: 100,
                        new_vel: 90,
                    },
                    Op::SetRoutingGain {
                        edge_id: f.edge,
                        old_gain_db: None,
                        new_gain_db: Some(-4.0),
                    },
                ],
                description: "AI 提案".to_owned(),
            },
        ]
    }

    #[test]
    fn every_variant_is_covered_by_the_showcase_script() {
        let names: Vec<&str> = showcase_ops().iter().map(Op::name).collect();
        for expected in [
            "AddNote",
            "DeleteNote",
            "MoveNote",
            "ModifyNoteVelocity",
            "AddClipPlacement",
            "RemoveClipPlacement",
            "MoveClipPlacement",
            "AddTrack",
            "RemoveTrack",
            "ConnectRouting",
            "DisconnectRouting",
            "SetRoutingGain",
            "InsertDevice",
            "RemoveDevice",
            "SetParam",
            "SetTrackMute",
            "SetTrackSolo",
            "SetMacro",
            "SetAutomationPoint",
            "RemoveAutomationPoint",
            "SetAutomationLane",
            "RemoveAutomationLane",
            "SetSection",
            "RemoveSection",
            "SetScene",
            "RemoveScene",
            "Batch",
        ] {
            assert!(names.contains(&expected), "脚本缺少变体 {expected}");
        }
    }

    /// 新增两个"池/图成员"变体后，规范 §7.2 的"新建片段 → 摆放 → 撤销"整链必须真的走得通，
    /// 且**每一步的破坏性尝试都要被拒绝**（[ARCH-OPS-001]，ADR-0001 D27）。
    ///
    /// 这条判据的价值在于：它同时钉住"能力**可表达**"与"悬空引用**不被允许**"——
    /// 前者是这条 ADR 存在的理由，后者是它不引入新破绽的保证。
    #[test]
    fn clip_pool_and_routing_node_ops_are_guarded_and_reversible() {
        let f = fixture();
        let mut doc = fixture_document();

        // ① 仍被摆放引用的片段不能移除。
        let error = Op::RemoveClip {
            clip_id: f.clip,
            previous_clip: empty_midi_clip(f.clip),
        }
        .apply(&mut doc)
        .expect_err("被摆放引用的片段必须拒绝移除");
        assert!(
            matches!(error, ModelError::ClipInUse { placement_count, .. } if placement_count >= 1),
            "期望 ClipInUse, 实际 {error:?}"
        );

        // ② 仍被路由边引用的节点不能移除。
        let error = Op::RemoveRoutingNode { node: f.lead }
            .apply(&mut doc)
            .expect_err("被边引用的节点必须拒绝移除");
        assert!(
            matches!(error, ModelError::RoutingNodeInUse { edge_count, .. } if edge_count >= 1),
            "期望 RoutingNodeInUse, 实际 {error:?}"
        );

        // ③ 重复加入要被拒(池与图各一次)。
        let error = Op::AddRoutingNode { node: f.master }
            .apply(&mut doc)
            .expect_err("重复节点必须被拒");
        assert!(matches!(error, ModelError::DuplicateEntityId { .. }));
        let error = Op::AddClip {
            clip: empty_midi_clip(f.clip),
        }
        .apply(&mut doc)
        .expect_err("重复片段必须被拒");
        assert!(matches!(error, ModelError::DuplicateEntityId { .. }));

        // ④ 正向能力: 新建片段 → 摆放 → 逐级撤销 → **逐字节**回到原状。
        //    在 D27 之前这一步在 `Op` 层根本无法表达(没有任何变体能把片段放进池子)。
        let mut chain = fixture_document();
        let before = serde_json::to_string(&chain).expect("serialize");
        let fresh_clip = fixture_id(600);
        let add_clip = Op::AddClip {
            clip: empty_midi_clip(fresh_clip),
        };
        add_clip.apply(&mut chain).expect("新建片段");
        let place = Op::AddClipPlacement {
            track_id: f.bass,
            placement: ClipPlacement {
                id: fixture_id(601),
                clip_id: fresh_clip,
                start_tick: 0,
                duration_ticks: 960,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        };
        place.apply(&mut chain).expect("摆放新建的片段");
        place.apply_inverse(&mut chain).expect("撤销摆放");
        add_clip.apply_inverse(&mut chain).expect("撤销新建片段");
        assert_eq!(
            serde_json::to_string(&chain).expect("serialize"),
            before,
            "新建片段 → 摆放 → 全链撤销必须逐字节回到原状"
        );
    }

    /// 判据（混音开关 `MUST-GATE-010` 的一格）：`SetTrackMute` / `SetTrackSolo` 的三条硬性质
    /// —— **可逆** / **前置条件走既有错误码** / **无操作（`new == old`）被放行**。
    ///
    /// 这三条就是这两个变体进入 `Op` 全集的条件（负责人批准时点名）。
    /// 怎么变红：
    /// - 把 `structural_inverse()` 里两个字段都写成 `*new_*` ⇒ 逆操作的前置条件不成立，
    ///   下面的 `invert` 直接 `Err`；
    /// - 把 `precondition()` 里的比较删掉（或把 `!=` 写成 `==`）⇒ 第二段断言红；
    /// - 把 `commit()` 写成只读不写 ⇒ 第一段的 `read(&doc)` 断言红。
    #[test]
    fn the_mix_switch_ops_are_reversible_and_check_their_payloads() {
        let f = fixture();
        let mut doc = fixture_document();
        // 夹具起点：lead 的 `mute` 与 bass 的 `solo` 都是 `false`。
        assert!(!doc.track(&f.lead).expect("lead").mute);
        assert!(!doc.track(&f.bass).expect("bass").solo);

        // ---- ① 真的写下去，并按载荷精确还原 ----
        let before_mute = doc.clone();
        let mute = Op::SetTrackMute {
            track_id: f.lead,
            old_mute: false,
            new_mute: true,
        };
        mute.apply(&mut doc).expect("静音必须可应用");
        assert!(doc.track(&f.lead).expect("lead").mute, "必须真的写成 true");
        mute.invert(&doc)
            .expect("求逆")
            .apply(&mut doc)
            .expect("逆应用");
        assert_eq!(doc, before_mute, "静音的逆必须逐位还原");

        let before_solo = doc.clone();
        let solo = Op::SetTrackSolo {
            track_id: f.bass,
            old_solo: false,
            new_solo: true,
        };
        solo.apply(&mut doc).expect("独奏必须可应用");
        assert!(doc.track(&f.bass).expect("bass").solo, "必须真的写成 true");
        solo.invert(&doc)
            .expect("求逆")
            .apply(&mut doc)
            .expect("逆应用");
        assert_eq!(doc, before_solo, "独奏的逆必须逐位还原");

        // ---- ② 载荷与文档不符 ⇒ 既有错误码 `OpStateMismatch`（不发明新码, D25） ----
        assert_eq!(
            Op::SetTrackMute {
                track_id: f.lead,
                old_mute: true,
                new_mute: false,
            }
            .apply(&mut doc),
            Err(ModelError::OpStateMismatch { op: "SetTrackMute" }),
            "文档现值是 false 而载荷说 true ⇒ 必须拒绝"
        );
        assert_eq!(
            Op::SetTrackSolo {
                track_id: f.bass,
                old_solo: true,
                new_solo: false,
            }
            .apply(&mut doc),
            Err(ModelError::OpStateMismatch { op: "SetTrackSolo" })
        );
        // 轨道的**其它**布尔字段不许被顺手改掉（载荷里没有 `solo_safe`）。
        assert!(!doc.track(&f.lead).expect("lead").solo_safe);

        // ---- ③ 不存在的轨道 ⇒ 既有的"轨道找不到"，不是新码 ----
        let ghost = fixture_id(998);
        assert!(
            Op::SetTrackMute {
                track_id: ghost,
                old_mute: false,
                new_mute: true,
            }
            .apply(&mut doc)
            .is_err(),
            "不存在的轨道必须被拒绝（既有错误码，不是新码）"
        );

        // ---- ④ 无操作（`new == old`）：与 `SetParam` 同口径 —— **前置条件放行** ----
        //
        // 模型**允许**无操作（`SetParam` 也允许）；"要不要把它变成一次可撤销的编辑"由调用方
        // 决定 —— GUI 侧在值没变时**不提交**（`host::end_mixer_drag` 返回 `false`）。
        let mut noop_doc = fixture_document();
        let noop = Op::SetTrackMute {
            track_id: f.lead,
            old_mute: false,
            new_mute: false,
        };
        assert_eq!(noop.apply(&mut noop_doc), Ok(()), "无操作必须被放行");
        assert_eq!(noop_doc, fixture_document(), "无操作不得改变文档的任何一位");
    }

    #[test]
    fn every_variant_applies_and_inverts_exactly() {
        let mut doc = fixture_document();
        for op in showcase_ops() {
            let snapshot = doc.clone();
            op.apply(&mut doc)
                .unwrap_or_else(|error| panic!("{} 应用失败: {error}", op.name()));
            assert_ne!(doc, snapshot, "{} 必须真的改变文档", op.name());
            let inverse = op
                .invert(&doc)
                .unwrap_or_else(|error| panic!("{} 求逆失败: {error}", op.name()));
            inverse
                .apply(&mut doc)
                .unwrap_or_else(|error| panic!("{} 的逆操作应用失败: {error}", op.name()));
            assert_eq!(doc, snapshot, "{} 的逆操作必须精确还原", op.name());
            doc.validate()
                .unwrap_or_else(|error| panic!("{} 撤销后文档必须仍合法: {error}", op.name()));
        }
    }

    /// 类别⑤（幂等性）：同一 `Op` 连续施加**两次**之后，文档必须与只施加一次**逐字节相同**。
    ///
    /// 为什么需要这条：`MUST-GATE-010` 的 `proptest` 量的是"施加 N 步再逆序撤销 N 步 ⇒
    /// 状态守恒"，也就是**逆向**幂等；改动前 `ops.rs` 的测试模块里 **19** 条 `#[test]`
    /// （读数：`grep -c '^\s*#\[test\]' src/ops.rs`）没有一条量**正向重放**。
    /// 而"重放同一条 op 会不会把状态改两遍"恰恰是重试 / 重连 / AI 代理重复提交的安全前提
    /// （`MCP-TOOL-006` 的 `idempotencyKey` 就是为它存在的）。
    ///
    /// 机械枚举：`showcase_ops()` 的 **35** 条覆盖 `Op` 的**全部 31 个变体**
    /// （由 `every_op_variant_is_declared_in_the_contract` 从源码的穷举 `match` 机械抽取证明），
    /// 每一条都在**全新** `fixture_document()` 上跑（与
    /// `every_variant_applies_and_inverts_exactly` 同一前提：脚本里的每条 op 都针对初始文档构造）。
    ///
    /// 判定分三类，**双向**登记：
    ///
    /// | 类 | 行为 | 期望 |
    /// | :--- | :--- | :--- |
    /// | 值变换（绝大多数） | 第二遍被前置条件拒绝（`Err`） | 文档与第一遍之后**逐字节相同** |
    /// | 相对变换（[`Op::MoveNote`]） | 第二遍被接受，增量**再加一次** | 必须在登记表里，且差值恰好是那一个增量 |
    /// | 其它 | —— | 红 |
    ///
    /// 登记表是双向的：漏登记（观察到的"被接受"不在表里）与多登记（表里的名字没被接受）
    /// 都会让判据变红。因此谁把某个变体从相对改成绝对值（或反之），都必须同步改这张表。
    #[test]
    fn re_applying_the_same_op_is_rejected_and_leaves_the_document_byte_identical() {
        /// 集合里**唯一**的相对变换（见 [`Op::MoveNote`] 的文档）。
        const RELATIVE_TRANSFORMS: [&str; 1] = ["MoveNote"];

        let all_ops = showcase_ops();
        let mut accepted_second_apply: std::collections::BTreeSet<&'static str> =
            std::collections::BTreeSet::new();

        for op in &all_ops {
            let name = op.name();
            let mut doc = fixture_document();
            op.apply(&mut doc)
                .unwrap_or_else(|error| panic!("{name} 第一遍必须成功: {error}"));
            // 空转的前提对照：这条 op 必须真的改变了文档，否则下面的比较没有内容。
            assert_ne!(doc, fixture_document(), "{name} 必须真的改变文档");

            let after_once = serde_json::to_vec(&doc).expect("序列化第一遍之后的状态");
            let second = op.apply(&mut doc);
            let after_twice = serde_json::to_vec(&doc).expect("序列化第二遍之后的状态");

            match second {
                Ok(()) => {
                    accepted_second_apply.insert(name);
                    assert!(
                        RELATIVE_TRANSFORMS.contains(&name),
                        "{name} 的第二遍被接受了，但它不在相对变换登记表里 —— \
                         要么它是缺陷，要么请把它登记进 RELATIVE_TRANSFORMS 并写清理由"
                    );
                    assert_ne!(
                        after_once, after_twice,
                        "{name} 被登记为相对变换，第二遍就必须真的**再动一次**状态"
                    );
                }
                Err(_) => {
                    assert!(
                        !RELATIVE_TRANSFORMS.contains(&name),
                        "{name} 已在相对变换登记表里，第二遍却被拒绝了 —— 请同步删掉登记"
                    );
                    assert_eq!(
                        after_once, after_twice,
                        "{name} 的第二遍被拒绝，文档就必须逐字节不动"
                    );
                }
            }
        }

        assert_eq!(
            accepted_second_apply,
            RELATIVE_TRANSFORMS
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            "相对变换登记表必须与实测的\"第二遍被接受\"集合**一一对应**（双向）"
        );
        assert_eq!(
            all_ops.len(),
            35,
            "showcase 脚本的条数变了 —— 新变体必须同时进脚本与本判据的登记表"
        );
    }

    /// 类别⑤（确定性）：同一条 `Op` / 同一条 `StampedOp` 连续序列化两次必须**逐字节相同**。
    ///
    /// 判什么：`serde_json::to_vec` 两次的字节数组长度与内容（单位 = 字节）。
    /// 为什么需要：`Op` 的载荷里有 `f32`（`SetParam` / `SetRoutingGain` / `SetMacro`）、
    /// 嵌套的 `BTreeMap`（自动化点）与外部标签枚举；任何一处引入哈希序或浮点格式化抖动，
    /// 都会让"同一份 op 日志导出两次不同"，而既有判据只做了一次 `to_value` 往返。
    #[test]
    fn every_op_and_stamped_op_serializes_twice_byte_identically() {
        for op in showcase_ops() {
            let name = op.name();
            let first = serde_json::to_vec(&op).expect("序列化第一遍");
            let second = serde_json::to_vec(&op).expect("序列化第二遍");
            assert_eq!(first, second, "{name} 两次序列化必须逐字节相同");

            let after_round_trip: Op = serde_json::from_slice(&first).expect("反序列化必须成功");
            assert_eq!(
                serde_json::to_vec(&after_round_trip).expect("往返后再序列化"),
                first,
                "{name} 反序列化后再序列化必须回到同一串字节"
            );
        }

        for origin in all_origin_variants() {
            let stamped = StampedOp::new(origin, 42, showcase_ops()[0].clone());
            let first = serde_json::to_vec(&stamped).expect("序列化第一遍");
            let second = serde_json::to_vec(&stamped).expect("序列化第二遍");
            assert_eq!(
                first, second,
                "来源 {:?} 的 StampedOp 两次序列化必须逐字节相同",
                stamped.origin
            );
        }
    }

    #[test]
    fn inverted_batch_is_applied_in_reverse_order() {
        let f = fixture();
        let mut doc = fixture_document();
        let snapshot = doc.clone();
        let batch = Op::Batch {
            ops: vec![
                Op::AddNote {
                    track_id: f.lead,
                    clip_id: f.clip,
                    note: MidiNote::new(fixture_id(600), 0, 60, 240),
                },
                Op::AddNote {
                    track_id: f.lead,
                    clip_id: f.clip,
                    note: MidiNote::new(fixture_id(601), 240, 62, 240),
                },
            ],
            description: "two notes".to_owned(),
        };
        batch.apply(&mut doc).expect("apply");
        assert_eq!(
            doc.clip_pool[&f.clip].content.notes().expect("midi").len(),
            4
        );
        let inverse = batch.invert(&doc).expect("invert");
        match &inverse {
            Op::Batch { ops, .. } => {
                assert_eq!(ops.len(), 2);
                assert!(matches!(ops[0], Op::DeleteNote { .. }));
                assert_eq!(ops[0].name(), "DeleteNote");
            }
            other => panic!("批量的逆必须是批量, 实际 {}", other.name()),
        }
        inverse.apply(&mut doc).expect("apply inverse");
        assert_eq!(doc, snapshot);
    }

    #[test]
    fn batch_is_atomic_when_a_sub_op_fails() {
        let f = fixture();
        let mut doc = fixture_document();
        let snapshot = doc.clone();
        let batch = Op::Batch {
            ops: vec![
                Op::AddNote {
                    track_id: f.lead,
                    clip_id: f.clip,
                    note: MidiNote::new(fixture_id(610), 0, 60, 240),
                },
                // 第二个子操作必然失败（音符不存在）
                Op::ModifyNoteVelocity {
                    track_id: f.lead,
                    clip_id: f.clip,
                    note_id: fixture_id(611),
                    old_vel: 0,
                    new_vel: 1,
                },
            ],
            description: "must roll back".to_owned(),
        };
        assert!(batch.apply(&mut doc).is_err());
        assert_eq!(doc, snapshot, "批量失败必须整体不生效 (原子性)");
    }

    /// `crates/yeban-model/../../schemas/<name>`。
    fn schema_path(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("schemas")
            .join(name)
    }

    /// 解析 `schemas/ops.schema.json`。
    fn ops_schema() -> serde_json::Value {
        let path = schema_path("ops.schema.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
        serde_json::from_str(&text).expect("ops.schema.json 必须是合法 JSON")
    }

    /// 契约里 `op.oneOf[*].required[0]` 声明的变体清单。
    fn schema_op_variant_names() -> std::collections::BTreeSet<String> {
        ops_schema()["properties"]["op"]["oneOf"]
            .as_array()
            .expect("op.oneOf 必须是数组")
            .iter()
            .map(|branch| {
                branch["required"][0]
                    .as_str()
                    .expect("每个分支必须 required 一个变体键")
                    .to_owned()
            })
            .collect()
    }

    /// **本线新增、契约里暂时还没有**的 `Op` 变体（显式欠账清单）。
    ///
    /// `schemas/ops.schema.json` 是**契约**：本线（`line/model-automation`）按工作线纪律
    /// **禁改** `schemas/**`，它由集成者与契约线共同拥有。于是这两个变体此刻只存在于
    /// 枚举里，契约的 `op.oneOf` 仍是 27 个分支。
    ///
    /// 这份清单是**机器校验的欠账**，而不是"把判据放松"：
    /// `op_variants_match_ops_schema_exactly` 断言
    /// `enum − contract == PENDING_CONTRACT_OPS`（且两集合不相交）。因此
    ///
    /// - 契约补上这两个分支 ⇒ 差集变空 ≠ 本清单 ⇒ **判据立刻红并指名"清空本清单"**
    ///   （欠账不会腐烂成静默漂移，也不需要谁记得它）；
    /// - 枚举再多出一个未登记的变体 ⇒ 差集 ≠ 本清单 ⇒ 红；
    /// - 契约少一个分支（枚举有、契约没有、又不在本清单） ⇒ 红。
    ///
    /// 集成者把两个分支加进 `schemas/ops.schema.json` 的 `op.oneOf` 后，
    /// **同时**把这里清成空数组即可（`needs` 里已点名）。
    /// **已清空**：集成者已在 `schemas/ops.schema.json` 的 `op.oneOf` 补上
    /// `SetAutomationLane` / `RemoveAutomationLane`（27 → 29），欠账归零。
    /// 这份清单保留为空数组，是为了让"契约落后于枚举"这件事**仍有地方可登记**
    /// （下一次谁加了变体又来不及改契约，就往这里加一个名字，棘轮会替他记住）。
    const PENDING_CONTRACT_OPS: [&str; 0] = [];

    /// [`PENDING_CONTRACT_OPS`] 的集合形态。
    fn pending_contract_ops() -> std::collections::BTreeSet<String> {
        PENDING_CONTRACT_OPS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    /// 每个 `Op` 变体的 JSON 形状必须与 `schemas/ops.schema.json` **一一对应**。
    ///
    /// 这条判据**直接读契约文件**：不再手抄一份变体名清单（第二份事实源会在契约改动后
    /// 变成谎言 —— `project.rs` 里那版手抄表就是前车之鉴）。
    /// 由于契约的每个分支各自 `required` 一个互不相同的键，`oneOf` 的
    /// "恰好匹配一个"在 JSON 层面等价于"`op` 对象恰好 1 个键，且键名 == 变体名"。
    #[test]
    fn op_variants_match_ops_schema_exactly() {
        let contract = schema_op_variant_names();
        let pending = pending_contract_ops();
        assert!(
            contract.is_disjoint(&pending),
            "契约已经补上了 {pending:?} 中的分支 —— 请把 PENDING_CONTRACT_OPS 清空, \
             否则这条判据会一直假装契约仍缺这两个变体"
        );
        assert_eq!(
            contract.len(),
            31,
            "op.oneOf 必须覆盖 31 个变体, 实际 {}: {contract:?}",
            contract.len()
        );

        let mut implemented: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for op in showcase_ops() {
            let name = op.name();
            implemented.insert(name.to_owned());
            let value = serde_json::to_value(&op).expect("serialize");
            let object = value.as_object().expect("externally tagged object");
            assert_eq!(
                object.len(),
                1,
                "{name} 必须是单键外部标签 (oneOf 恰好匹配一个)"
            );
            assert!(object.contains_key(name), "{name} 的 JSON 键必须同名");
            let back: Op = serde_json::from_value(value).expect("deserialize");
            assert_eq!(back, op, "{name} 必须能往返");
        }
        assert_eq!(
            implemented,
            contract
                .union(&pending)
                .cloned()
                .collect::<std::collections::BTreeSet<String>>(),
            "实现必须与 schemas/ops.schema.json 的 op.oneOf **加上显式欠账 {}** 一一对应",
            PENDING_CONTRACT_OPS.len()
        );
    }

    /// **枚举全集**必须与契约一致 —— 补上 `op_variants_match_ops_schema_exactly` 的盲区。
    ///
    /// 为什么需要这条：那条判据比较的是 `showcase_ops()` 与契约。
    /// 于是"给 `Op` 加了一个变体，但既没加进 `showcase_ops()` 也没加进契约"这种情况
    /// **两边都看不见，判据全绿** —— 而它恰恰是最危险的漂移
    /// （`line/tools-domain` 实测：`Op` 全集缺 `AddClip`/`AddRoutingNode` 等 4 个变体，
    /// 导致规范 §7.2 的"声部连接"在操作日志层**不可表达**，而所有判据都是绿的）。
    ///
    /// 做法：`name()` 里的 `match self` 是**穷举**的（少一个变体就编译不过），
    /// 因此从源码里抽取 `Self::<Variant>` 就是枚举全集 —— 这是不用过程宏也能拿到全集的唯一办法。
    #[test]
    fn every_op_variant_is_declared_in_the_contract() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ops.rs"),
        )
        .expect("读取 ops.rs");
        let start = source
            .find("pub const fn name(&self)")
            .expect("找到 name()");
        let body = &source[start..];
        let end = body.find("\n    }").expect("name() 的结尾");
        let mut declared: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for chunk in body[..end].split("Self::").skip(1) {
            let name: String = chunk
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if !name.is_empty() {
                declared.insert(name);
            }
        }
        let contract = schema_op_variant_names();
        let pending = pending_contract_ops();
        // 双向漂移都必须红。唯一的**显式**例外是 PENDING_CONTRACT_OPS（本线禁改
        // `schemas/**` 造成的已知欠账）：差集必须**恰好等于**那份清单，
        // 多一个、少一个、或契约补上后没清空清单，都会在这里变红。
        assert_eq!(
            declared
                .difference(&contract)
                .cloned()
                .collect::<std::collections::BTreeSet<String>>(),
            pending,
            "枚举里多出来的变体必须**恰好**是 PENDING_CONTRACT_OPS;\n\
             若契约已补齐, 请把该清单清空;\n\
             只在枚举里而契约缺失: {:?}",
            declared.difference(&contract).collect::<Vec<_>>(),
        );
        assert!(
            contract.difference(&declared).next().is_none(),
            "只在契约里而枚举缺失: {:?}",
            contract.difference(&declared).collect::<Vec<_>>(),
        );
    }

    /// **枚举全集**（测试里的单一事实源）：`OpOrigin` 的每一个变体。
    ///
    /// 手写是因为 Rust 没有反射；完整性由
    /// [`Self::declared_origin_variant_names`]（源码扫描）在
    /// `origin_wire_shape_is_frozen_byte_for_byte` 里机械对齐。
    fn all_origin_variants() -> Vec<OpOrigin> {
        vec![
            OpOrigin::UserUi,
            OpOrigin::MidiInput,
            OpOrigin::McpProposal {
                proposal_id: fixture_id(1),
                agent_name: "claude".to_owned(),
            },
            OpOrigin::McpEdit {
                agent_name: "yeban-mcp".to_owned(),
            },
            OpOrigin::UndoRedo,
            OpOrigin::AutomationRecord,
            OpOrigin::Import,
            OpOrigin::Migration,
        ]
    }

    /// 从**源码文本**里抽取 `pub enum OpOrigin` 的全部变体名。
    ///
    /// 为什么扫源码：`OpOrigin` **没有**任何穷举 `match`（它只被 serde 派生消费），
    /// 所以拿不到"编译器保证的全集"。这里用与
    /// `every_op_variant_is_declared_in_the_contract` 同族的做法：只认**第一层**
    /// （`depth == 1`）的标识符 —— 于是 `McpProposal { proposal_id: ... }` 的载荷键
    /// 不会被误当成变体，文档注释整行跳过。
    fn declared_origin_variant_names() -> std::collections::BTreeSet<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ops.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
        let start = source
            .find("pub enum OpOrigin {")
            .expect("找到 OpOrigin 枚举");
        let mut depth: i32 = 0;
        let mut in_comment = false;
        let mut current = String::new();
        let mut declared = std::collections::BTreeSet::new();
        for ch in source[start..].chars() {
            if in_comment {
                if ch == '\n' {
                    in_comment = false;
                }
                continue;
            }
            match ch {
                '/' => {
                    in_comment = true;
                    continue;
                }
                '{' => {
                    depth += 1;
                    current.clear();
                    continue;
                }
                '}' => {
                    depth -= 1;
                    current.clear();
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                _ => {}
            }
            if depth != 1 {
                continue;
            }
            if current.is_empty() {
                if ch.is_ascii_uppercase() {
                    current.push(ch);
                }
            } else if ch.is_ascii_alphanumeric() || ch == '_' {
                current.push(ch);
            } else {
                declared.insert(std::mem::take(&mut current));
            }
        }
        declared
    }

    /// **枚举有、契约 `origin.oneOf` 还没有**的对象标签（显式欠账清单）。
    ///
    /// 与 `PENDING_CONTRACT_OPS` 同一族的机械欠账：`schemas/**` 由契约线独占、
    /// 本线禁改 ⇒ `OpOrigin::McpEdit` 此刻只存在于枚举里。判据
    /// `origin_variants_match_ops_schema_origin_one_of` 断言
    /// "枚举的对象标签 − 契约的对象标签 **恰好等于**这份清单"，因此
    ///
    /// - 契约补上 `McpEdit` 分支 ⇒ 差集变空 ≠ 本清单 ⇒ **立刻红并指名清空本清单**；
    /// - 谁再往枚举里加一个对象标签而没登记 ⇒ 红；
    /// - 契约多出一个枚举里没有的对象分支 ⇒ 红。
    ///
    /// **已清空**：`line/origin-contract` 已在 `schemas/ops.schema.json` 的
    /// `origin.oneOf` 末尾补上 `McpEdit` 对象分支（`additionalProperties: false`，
    /// 只许 `agent_name`），并追平了 `docs/ledger/origin-contract-notes.md` 里的三处改动。
    /// 与 `PENDING_CONTRACT_OPS` 一样保留**空数组**：下一次谁在枚举里加了对象标签
    /// 又来不及改契约，就往这里加一个名字，棘轮会替他记住。
    /// 给契约线的请求原文见 `docs/ledger/op-origin-mcp-notes.md` 的 needs 节。
    const PENDING_CONTRACT_ORIGINS: [&str; 0] = [];

    /// [`PENDING_CONTRACT_ORIGINS`] 的集合形态。
    fn pending_contract_origins() -> std::collections::BTreeSet<String> {
        PENDING_CONTRACT_ORIGINS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    /// 每个 `OpOrigin` 变体的**线上字节**必须逐字节冻住（既有语义一字不改）。
    ///
    /// 这是"新增变体不得顺手改动任何既有变体"的机械钉：重命名、加字段、改标签都会
    /// 在这里变红。它同时是"枚举全集"的棘轮 —— 冻结表必须与源码里的变体**恰好**
    /// 一一对应（多一个没登记 ⇒ 红；登记了但枚举里没有 ⇒ 红）。
    #[test]
    fn origin_wire_shape_is_frozen_byte_for_byte() {
        let frozen: [(&str, OpOrigin, &str); 8] = [
            ("UserUi", OpOrigin::UserUi, "\"UserUi\""),
            ("MidiInput", OpOrigin::MidiInput, "\"MidiInput\""),
            (
                "McpProposal",
                OpOrigin::McpProposal {
                    proposal_id: fixture_id(1),
                    agent_name: "claude".to_owned(),
                },
                "{\"McpProposal\":{\"proposal_id\":\"01J8ZQ00000000000000000001\",\
                 \"agent_name\":\"claude\"}}",
            ),
            (
                "McpEdit",
                OpOrigin::McpEdit {
                    agent_name: "yeban-mcp".to_owned(),
                },
                "{\"McpEdit\":{\"agent_name\":\"yeban-mcp\"}}",
            ),
            ("UndoRedo", OpOrigin::UndoRedo, "\"UndoRedo\""),
            (
                "AutomationRecord",
                OpOrigin::AutomationRecord,
                "\"AutomationRecord\"",
            ),
            ("Import", OpOrigin::Import, "\"Import\""),
            ("Migration", OpOrigin::Migration, "\"Migration\""),
        ];
        let mut frozen_names = std::collections::BTreeSet::new();
        for (name, origin, wire) in &frozen {
            frozen_names.insert((*name).to_owned());
            assert_eq!(
                serde_json::to_string(origin).expect("serialize"),
                *wire,
                "`{name}` 的线上字节变了 —— 既有来源变体的语义**不许**被顺手改动"
            );
            let back: OpOrigin = serde_json::from_str(wire).expect("deserialize");
            assert_eq!(&back, origin);
        }
        assert_eq!(
            frozen_names,
            declared_origin_variant_names(),
            "冻结表必须覆盖 `OpOrigin` 的**全部**变体：新增变体要在这里补一行, \
             并在 PENDING_CONTRACT_ORIGINS 里登记契约欠账"
        );
    }

    /// `OpOrigin` 的两种形状必须与契约的 `origin.oneOf` 对应（ADR-0001 D13）。
    #[test]
    fn origin_variants_match_ops_schema_origin_one_of() {
        let schema = ops_schema();
        let unit_names: std::collections::BTreeSet<String> =
            schema["properties"]["origin"]["oneOf"][0]["enum"]
                .as_array()
                .expect("origin.oneOf[0].enum 必须是数组")
                .iter()
                .map(|name| name.as_str().expect("enum 元素是字符串").to_owned())
                .collect();

        let mut serialized_units: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        let mut serialized_objects: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for origin in all_origin_variants() {
            let value = serde_json::to_value(&origin).expect("serialize");
            match &value {
                serde_json::Value::String(name) => {
                    serialized_units.insert(name.clone());
                }
                serde_json::Value::Object(object) => {
                    assert_eq!(object.len(), 1, "外部标签必须是单键对象: {value}");
                    serialized_objects.insert(object.keys().next().expect("唯一键").clone());
                }
                other => panic!("OpOrigin 只能序列化为纯字符串或单键对象, 实际 {other}"),
            }
            let back: OpOrigin = serde_json::from_value(value).expect("deserialize");
            assert_eq!(back, origin);
        }
        assert_eq!(
            serialized_units, unit_names,
            "单元来源变体必须与契约 origin.oneOf[0].enum 一一对应"
        );

        // 对象分支：契约里每个对象分支的 `required` 就是它的标签。
        let contract_objects: std::collections::BTreeSet<String> =
            schema["properties"]["origin"]["oneOf"]
                .as_array()
                .expect("origin.oneOf 必须是数组")
                .iter()
                .filter_map(|branch| branch.get("required").and_then(serde_json::Value::as_array))
                .flat_map(|required| {
                    required
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .collect();
        let pending = pending_contract_origins();
        assert!(
            contract_objects.is_disjoint(&pending),
            "契约已经补上了 {pending:?} 里的对象分支 —— 请把 PENDING_CONTRACT_ORIGINS 清空"
        );
        assert_eq!(
            serialized_objects
                .difference(&contract_objects)
                .cloned()
                .collect::<std::collections::BTreeSet<String>>(),
            pending,
            "枚举里多出来的对象标签必须**恰好**是 PENDING_CONTRACT_ORIGINS;\n\
             若契约已补齐, 请把该清单清空;\n\
             只在枚举里而契约缺失: {:?}",
            serialized_objects
                .difference(&contract_objects)
                .collect::<Vec<_>>(),
        );
        assert!(
            contract_objects
                .difference(&serialized_objects)
                .next()
                .is_none(),
            "只在契约里而枚举缺失的对象标签: {:?}",
            contract_objects
                .difference(&serialized_objects)
                .collect::<Vec<_>>(),
        );

        // McpProposal 必须落在契约的第二个分支：外部标签对象 + 恰好两个载荷键。
        let proposal = serde_json::to_value(OpOrigin::McpProposal {
            proposal_id: fixture_id(1),
            agent_name: "claude".to_owned(),
        })
        .expect("serialize");
        let object = proposal.as_object().expect("McpProposal 必须是对象");
        assert_eq!(object.len(), 1, "McpProposal 必须是单键外部标签");
        let payload = object
            .get("McpProposal")
            .and_then(serde_json::Value::as_object)
            .expect("载荷必须是对象");
        let contract_payload: std::collections::BTreeSet<String> =
            schema["properties"]["origin"]["oneOf"][1]["properties"]["McpProposal"]["required"]
                .as_array()
                .expect("契约必须声明 McpProposal 的 required")
                .iter()
                .map(|key| key.as_str().expect("键名是字符串").to_owned())
                .collect();
        let actual_payload: std::collections::BTreeSet<String> = payload.keys().cloned().collect();
        assert_eq!(actual_payload, contract_payload);
    }

    /// `McpEdit` 的载荷形状必须与契约分支**逐字段**一致（任务书判据 ①③）。
    ///
    /// 为什么单开一条判据，而不是并进上面那条：
    /// `origin_variants_match_ops_schema_origin_one_of` 只把对象分支当作**标签**核对
    /// （`oneOf[*].required[0]` 的集合差），它对"标签带的载荷"只有一处硬编码检查 ——
    /// 而且只查 `McpProposal`（`oneOf[1].properties.McpProposal.required`，只看键集合，
    /// 不看 `additionalProperties`，也不看类型）。于是契约若被写成
    /// `{"McpEdit": {"type": "object"}}`（没有 `required`、没有
    /// `additionalProperties: false`），上面那条**依然全绿** —— 而契约 ① 的牙齿正是
    /// "只许 `agent_name`"。本判据：
    ///
    /// 1. 按**标签名**定位分支（不写下标 —— 上面那条按下标读契约的教训见本模块头部：
    ///    把新分支插到 `oneOf[0]` 位置实测 106 passed / 3 failed，两条判据读到错的形状）；
    /// 2. 分支与载荷**两处** `additionalProperties` 都必须为 `false`；
    /// 3. `serde_json` 实测载荷的键集合 == 契约载荷的 `required` == 契约载荷声明的
    ///    `properties`（三个集合相等 ⇒ 既没有"实现多写一个字段"，也没有"契约声明了
    ///    一个实现不写的字段"）；
    /// 4. 值的类型也与契约一致（`string`），并能往返。
    #[test]
    fn mcp_edit_origin_shape_matches_its_contract_branch() {
        let origin = OpOrigin::McpEdit {
            agent_name: "yeban-mcp".to_owned(),
        };
        let value = serde_json::to_value(&origin).expect("serialize");
        assert_eq!(
            serde_json::to_string(&origin).expect("serialize"),
            "{\"McpEdit\":{\"agent_name\":\"yeban-mcp\"}}",
            "McpEdit 的线上字节（serde_json 实测）必须与契约分支允许的形状一致"
        );

        let schema = ops_schema();
        let branch = schema["properties"]["origin"]["oneOf"]
            .as_array()
            .expect("origin.oneOf 必须是数组")
            .iter()
            .find(|branch| branch["required"][0].as_str() == Some("McpEdit"))
            .expect("契约必须有一个 `required` 恰好是 [\"McpEdit\"] 的对象分支");
        assert_eq!(branch["type"], "object", "McpEdit 必须是外部标签对象分支");
        assert_eq!(
            branch["required"],
            serde_json::json!(["McpEdit"]),
            "分支的标签键必须恰好是 McpEdit"
        );
        assert_eq!(
            branch["additionalProperties"],
            serde_json::json!(false),
            "分支只许 `McpEdit` 一个键（additionalProperties: false，不许放宽）"
        );

        let payload = &branch["properties"]["McpEdit"];
        assert_eq!(
            payload["additionalProperties"],
            serde_json::json!(false),
            "McpEdit 载荷只许 `agent_name` 一个字段（additionalProperties: false，不许放宽）"
        );
        assert_eq!(
            payload["required"],
            serde_json::json!(["agent_name"]),
            "McpEdit 载荷必须 required 恰好 [`agent_name`]"
        );
        // 载荷字段的**形状**（名字 → 类型）必须恰好是 `{agent_name: string}`。
        // 只比"名字 + type"，不比整段子 schema：`description` 是给人看的文档，
        // 不改变线上形状（`McpProposal.proposal_id` 同样带 description）。
        let contract_types: std::collections::BTreeMap<String, String> = payload["properties"]
            .as_object()
            .expect("契约必须声明载荷的 properties")
            .iter()
            .map(|(name, spec)| {
                (
                    name.clone(),
                    spec["type"]
                        .as_str()
                        .expect("每个载荷字段都要声明 type")
                        .to_owned(),
                )
            })
            .collect();
        assert_eq!(
            contract_types,
            std::collections::BTreeMap::from([("agent_name".to_owned(), "string".to_owned())]),
            "McpEdit 载荷声明的字段形状必须恰好是「一个 string 型的 agent_name」"
        );

        let object = value.as_object().expect("McpEdit 必须是单键外部标签对象");
        assert_eq!(object.len(), 1, "McpEdit 必须是单键外部标签: {value}");
        let serialized_payload = object
            .get("McpEdit")
            .and_then(serde_json::Value::as_object)
            .expect("载荷必须是对象");
        let serialized_keys: std::collections::BTreeSet<String> =
            serialized_payload.keys().cloned().collect();
        let contract_required: std::collections::BTreeSet<String> = payload["required"]
            .as_array()
            .expect("契约必须声明载荷的 required")
            .iter()
            .map(|key| key.as_str().expect("键名是字符串").to_owned())
            .collect();
        let contract_declared: std::collections::BTreeSet<String> = payload["properties"]
            .as_object()
            .expect("契约必须声明载荷的 properties")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            serialized_keys, contract_required,
            "serde 实测载荷键必须与契约 required **逐字段一致**（实测输出 {value}）"
        );
        assert_eq!(
            contract_declared, contract_required,
            "契约声明的载荷字段必须与 required 一致（不许声明一个谁都不会写的字段）"
        );
        assert!(
            serialized_payload["agent_name"].is_string(),
            "agent_name 必须是字符串（契约 type: string），实际 {value}"
        );

        let back: OpOrigin = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back, origin, "McpEdit 必须能往返");
    }

    #[test]
    fn origin_variants_round_trip() {
        for origin in all_origin_variants() {
            let json = serde_json::to_string(&origin).expect("serialize");
            let back: OpOrigin = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, origin);
        }
        let stamped = StampedOp::user_ui(42, showcase_ops()[0].clone());
        let json = serde_json::to_string(&stamped).expect("serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
        for key in ["origin", "timestamp", "op"] {
            assert!(value.get(key).is_some(), "StampedOp 缺键 {key}");
        }
        let back: StampedOp = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, stamped);
    }

    /// 来源标签**不得**影响 Op 的施加/撤销语义：同一个 op 换成 `McpEdit` 之后，
    /// 施加与逆操作的结果必须与 `UserUi` 来源**逐字节相同**。
    ///
    /// 这是任务书判据 ③ 的模型侧那一半：新增变体不可能让撤销/重做"失配"，
    /// 因为 `origin` 只出现在 `StampedOp` 的信封上，逆操作只看 `op` 本体。
    #[test]
    fn the_new_origin_variant_never_changes_an_op_or_its_inverse() {
        let f = fixture();
        let op = Op::AddNote {
            track_id: f.lead,
            clip_id: f.clip,
            note: MidiNote::new(fixture_id(700), 480, 64, 240),
        };
        let mcp_edit = || OpOrigin::McpEdit {
            agent_name: "yeban-mcp".to_owned(),
        };
        let mut by_ui = fixture_document();
        let mut by_mcp = fixture_document();
        StampedOp::new(OpOrigin::UserUi, 42, op.clone())
            .apply(&mut by_ui)
            .expect("apply ui");
        StampedOp::new(mcp_edit(), 42, op.clone())
            .apply(&mut by_mcp)
            .expect("apply mcp");
        assert_eq!(by_ui, by_mcp, "来源标签不得改变工程内容");
        assert_ne!(by_mcp, fixture_document(), "前提: 施加确实改了工程");

        StampedOp::new(OpOrigin::UserUi, 42, op.clone())
            .apply_inverse(&mut by_ui)
            .expect("inverse ui");
        StampedOp::new(mcp_edit(), 42, op)
            .apply_inverse(&mut by_mcp)
            .expect("inverse mcp");
        assert_eq!(by_ui, by_mcp, "逆操作不得因来源标签而不同");
        assert_eq!(by_ui, fixture_document(), "逆操作必须精确还原");
    }

    #[test]
    fn stale_payloads_are_rejected_before_mutating() {
        let f = fixture();
        let mut doc = fixture_document();
        let snapshot = doc.clone();

        let stale_delete = Op::DeleteNote {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            previous_note: MidiNote::new(f.note, 0, 61, 480),
        };
        assert_eq!(
            stale_delete.apply(&mut doc),
            Err(ModelError::OpStateMismatch { op: "DeleteNote" })
        );

        let stale_velocity = Op::ModifyNoteVelocity {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            old_vel: 99,
            new_vel: 50,
        };
        assert_eq!(
            stale_velocity.apply(&mut doc),
            Err(ModelError::OpStateMismatch {
                op: "ModifyNoteVelocity"
            })
        );
        assert_eq!(doc, snapshot, "失败的 apply 绝不能改文档");
    }

    /// "旧值必须与文档现值**逐位**相同"是三个入口（`SetParam` / `SetMacro` /
    /// `SetRoutingGain`）共用的契约，而 `-0.0` 与 `+0.0` 的**位**不同。
    ///
    /// 实测：把 `left.to_bits() == right.to_bits()` 换成 `left == right` 时，全仓判据
    /// 保持全绿 —— 既有判据只用正数与 `NaN` 探过"不相等"，从没有用**符号零**探过
    /// "位不同但 `==` 相等"的那一格；而同载荷的 `NaN` 在 `==` 下也变成"不相等"。
    #[test]
    fn same_f32_is_bitwise_so_signed_zero_and_nan_payloads_are_not_equal() {
        assert!(same_f32(0.0, 0.0));
        assert!(same_f32(-0.0, -0.0));
        assert!(!same_f32(0.0, -0.0), "-0.0 与 +0.0 逐位不同, 必须判为不同");
        assert!(!same_f32(-0.0, 0.0));
        // 同一个 NaN 位模式必须逐位相等（`==` 会把它判成不相等）。
        assert!(same_f32(f32::NAN, f32::NAN));
        assert!(!same_f32(f32::NAN, 1.0));
        // 一个 ULP 也必须不同。
        assert!(!same_f32(1.0, f32::from_bits(1.0_f32.to_bits() + 1)));
        // `Option` 形态走同一把尺子（`SetRoutingGain` 用它）。
        assert!(same_gain(None, None));
        assert!(!same_gain(None, Some(0.0)));
        assert!(!same_gain(Some(0.0), Some(-0.0)));
    }

    /// 音高平移的合法区间是**闭区间** `0..=127`：上端点 127 必须可达。
    ///
    /// 实测：把 `(0..=127).contains(&shifted)` 改成 `(0..127)` 时，全仓判据保持全绿
    /// —— 既有判据只钉住"越界（`delta_pitch = 100`）被拒"这一侧，于是"最高音再也
    /// 移不到"这个缺陷没有任何判据看得见。
    #[test]
    fn moving_a_note_onto_the_top_pitch_is_accepted() {
        assert_eq!(shifted_pitch(0, 0), Some(0));
        assert_eq!(shifted_pitch(127, 0), Some(127));
        assert_eq!(shifted_pitch(127, 1), None);
        assert_eq!(shifted_pitch(0, -1), None);

        let f = fixture();
        let mut doc = fixture_document();
        let pitch = doc.note(&f.clip, &f.note).expect("音符存在").pitch;
        assert_eq!(pitch, 60, "夹具的音高变了, 本判据的增量要跟着改");
        let to_top = Op::MoveNote {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            delta_tick: 0,
            delta_pitch: i8::try_from(i16::from(127_u8) - i16::from(pitch)).expect("差值在 i8 内"),
        };
        to_top.apply(&mut doc).expect("移到音高 127 必须被接受");
        assert_eq!(doc.note(&f.clip, &f.note).expect("音符存在").pitch, 127);

        let beyond = Op::MoveNote {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            delta_tick: 0,
            delta_pitch: 1,
        };
        assert_eq!(
            beyond.apply(&mut doc),
            Err(ModelError::PitchOutOfRange { value: 127 }),
            "127 之上必须被拒"
        );
    }

    /// `SetParam` **永不**接受发送增益目标：那条路径必须用 `SetRoutingGain`
    /// （否则 `Option<f32>` 的"`None` = 单位增益"语义会被压平）。
    ///
    /// 实测：把 `read_param` 的 `SendGain` 分支改成 `Ok(0.0)` 时，全仓判据保持全绿
    /// —— 既有判据用的 `old_val` 恰好是 `0.0`，于是前置条件放行后由 `write_param`
    /// 报出同一个错误码，"两处守卫"里少掉一处也看不出来。这里用一个**陈旧的**
    /// `old_val`，于是唯一的拒绝理由只能是目标不适用。
    #[test]
    fn set_param_never_edits_send_gain_even_with_a_stale_old_value() {
        let f = fixture();
        let mut doc = fixture_document();
        let snapshot = doc.clone();
        let send_gain = Op::SetParam {
            target: AutomationTarget::SendGain {
                track_id: f.lead,
                edge_id: f.edge,
            },
            old_val: 1.0,
            new_val: 0.0,
        };
        assert!(matches!(
            send_gain.apply(&mut doc),
            Err(ModelError::AutomationTargetNotApplicable { .. })
        ));
        assert_eq!(doc, snapshot, "被拒之后文档必须逐字节不动");
    }

    #[test]
    fn out_of_range_payloads_are_rejected() {
        let f = fixture();
        let mut doc = fixture_document();

        let bad_velocity = Op::ModifyNoteVelocity {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            old_vel: 100,
            new_vel: 200,
        };
        assert_eq!(
            bad_velocity.apply(&mut doc),
            Err(ModelError::VelocityOutOfRange { value: 200 })
        );

        let bad_pitch = Op::MoveNote {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            delta_tick: 0,
            delta_pitch: 100,
        };
        assert_eq!(
            bad_pitch.apply(&mut doc),
            Err(ModelError::PitchOutOfRange { value: 60 })
        );

        let underflow = Op::MoveNote {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            delta_tick: -1,
            delta_pitch: 0,
        };
        assert_eq!(
            underflow.apply(&mut doc),
            Err(ModelError::OpStateMismatch { op: "MoveNote" })
        );

        let bad_cutoff = Op::SetParam {
            target: AutomationTarget::DeviceParam {
                track_id: f.lead,
                slot_index: 0,
                param_index: 0,
            },
            old_val: 1200.0,
            new_val: f32::INFINITY,
        };
        assert!(matches!(
            bad_cutoff.apply(&mut doc),
            Err(ModelError::NonFiniteValue { .. })
        ));

        let bad_slot = Op::SetParam {
            target: AutomationTarget::DeviceParam {
                track_id: f.lead,
                slot_index: 7,
                param_index: 0,
            },
            old_val: 0.0,
            new_val: 0.0,
        };
        assert!(matches!(
            bad_slot.apply(&mut doc),
            Err(ModelError::DeviceSlotOutOfRange { index: 7, .. })
        ));

        let send_gain = Op::SetParam {
            target: AutomationTarget::SendGain {
                track_id: f.lead,
                edge_id: f.edge,
            },
            old_val: 0.0,
            new_val: -3.0,
        };
        assert!(matches!(
            send_gain.apply(&mut doc),
            Err(ModelError::AutomationTargetNotApplicable { .. })
        ));

        let bad_macro = Op::SetMacro {
            track_id: f.lead,
            macro_index: 0,
            old_val: 0.5,
            new_val: 1.5,
        };
        assert_eq!(
            bad_macro.apply(&mut doc),
            Err(ModelError::MacroValueOutOfRange { value: 1.5 })
        );

        let bad_pan = Op::SetParam {
            target: AutomationTarget::TrackPan { track_id: f.lead },
            old_val: -0.25,
            new_val: 2.0,
        };
        assert_eq!(
            bad_pan.apply(&mut doc),
            Err(ModelError::PanOutOfRange { value: 2.0 })
        );

        assert_eq!(doc, fixture_document(), "全部失败路径都不得改文档");
    }

    #[test]
    fn removing_the_master_bus_track_is_refused() {
        let f = fixture();
        let mut doc = fixture_document();
        let op = Op::RemoveTrack {
            track_id: f.master,
            previous_track: doc.tracks[&f.master].clone(),
        };
        assert_eq!(
            op.apply(&mut doc),
            Err(ModelError::OpStateMismatch { op: "RemoveTrack" })
        );
    }

    #[test]
    fn invert_rejects_an_op_that_never_touched_this_document() {
        let f = fixture();
        let doc = fixture_document();
        // 这份文档里并没有 fresh_note，因此 `AddNote` 的逆（DeleteNote）不可应用。
        let never_applied = Op::AddNote {
            track_id: f.lead,
            clip_id: f.clip,
            note: MidiNote::new(fixture_id(900), 0, 60, 240),
        };
        assert_eq!(
            never_applied.invert(&doc),
            Err(ModelError::NoteNotFound {
                id: fixture_id(900)
            })
        );

        // 已应用的 `AddNote` 求逆必须成功。
        let mut applied = doc.clone();
        never_applied.apply(&mut applied).expect("apply");
        assert!(never_applied.invert(&applied).is_ok());
    }

    #[test]
    fn apply_inverse_is_exactly_equivalent_to_invert_then_apply() {
        let f = fixture();
        let mut direct = fixture_document();
        let mut staged = fixture_document();
        let op = Op::SetRoutingGain {
            edge_id: f.edge,
            old_gain_db: None,
            new_gain_db: Some(-1.5),
        };
        op.apply(&mut direct).expect("apply");
        op.apply(&mut staged).expect("apply");
        op.apply_inverse(&mut direct).expect("apply_inverse");
        let inverse = op.invert(&staged).expect("invert");
        inverse.apply(&mut staged).expect("apply");
        assert_eq!(direct, staged);
        assert_eq!(direct, fixture_document());
    }

    /// 固定种子的确定性 PRNG：同一种子恒产生同一序列（测试可复现）。
    struct StableRng(u64);

    impl StableRng {
        fn new(seed: u64) -> Self {
            Self(splitmix64(seed ^ 0x5945_4241_4E00_5EED))
        }

        fn next_u64(&mut self) -> u64 {
            self.0 = splitmix64(self.0);
            self.0
        }

        fn below(&mut self, bound: usize) -> usize {
            if bound <= 1 {
                0
            } else {
                let bound = u64::try_from(bound).unwrap_or(u64::MAX);
                usize::try_from(self.next_u64() % bound).unwrap_or(0)
            }
        }

        /// 在 `0..=255` 上取一个随机字节。
        fn byte(&mut self) -> u8 {
            u8::try_from(self.below(256)).unwrap_or(0)
        }

        /// 从集合的前 `window` 个键里挑一个（O(window)，避免每步遍历整张表）。
        fn pick<K: Ord + Copy, V>(&mut self, map: &BTreeMap<K, V>, window: usize) -> Option<K> {
            if map.is_empty() {
                return None;
            }
            let bound = map.len().min(window.max(1));
            map.keys().nth(self.below(bound)).copied()
        }
    }

    /// 片段内的音符数量（非 MIDI 片段计 0）。
    fn notes_count(entry: &ClipPoolEntry) -> usize {
        entry.content.notes().map_or(0, BTreeMap::len)
    }

    /// 从节点切片里挑一个（O(window)）。
    fn pick_node(rng: &mut StableRng, nodes: &[EntityId]) -> Option<EntityId> {
        if nodes.is_empty() {
            return None;
        }
        let bound = nodes.len().min(8);
        nodes.get(rng.below(bound)).copied()
    }

    /// 文档规模上限：属性测试要跑 10,000 步，实体无限增长会让
    /// `Batch` 的原子克隆与 `validate` 变成瓶颈，因此给"新增类"操作设上限。
    fn has_room(doc: &YebanProjectV1) -> bool {
        doc.tracks.len() < 32
            && doc.clip_pool.len() < 16
            && doc.sections.len() < 16
            && doc.scenes.len() < 16
            && doc.routing_graph.edges.len() < 32
    }

    /// 生成一个**保证可应用**的操作（载荷全部取自当前文档）。
    fn generate_op(rng: &mut StableRng, doc: &YebanProjectV1, counter: &mut u128) -> Op {
        let fresh = |counter: &mut u128| {
            *counter += 1;
            fixture_id(*counter + 10_000)
        };
        let kind = rng.below(24);
        let midi_clip = || {
            doc.clip_pool
                .values()
                .find(|entry| entry.content.notes().is_some())
                .map(|entry| entry.id)
        };
        let a_note = || {
            doc.clip_pool
                .values()
                .filter_map(|entry| entry.content.notes())
                .flat_map(BTreeMap::iter)
                .next()
        };

        match kind {
            0 => {
                let room = has_room(doc)
                    && doc
                        .clip_pool
                        .values()
                        .map(notes_count)
                        .all(|count| count < 64);
                if room
                    && let Some(clip_id) =
                        midi_clip().or_else(|| doc.clip_pool.keys().next().copied())
                {
                    let track_id = rng.pick(&doc.tracks, 8).unwrap_or(doc.master_bus_track_id);
                    return Op::AddNote {
                        track_id,
                        clip_id,
                        note: MidiNote::new(fresh(counter), u64::from(rng.byte()) * 240, 60, 240),
                    };
                }
            }
            1 => {
                if let (Some(clip_id), Some((note_id, note))) = (midi_clip(), a_note()) {
                    let track_id = rng.pick(&doc.tracks, 8).unwrap_or(doc.master_bus_track_id);
                    return Op::DeleteNote {
                        track_id,
                        clip_id,
                        note_id: *note_id,
                        previous_note: note.clone(),
                    };
                }
            }
            2 => {
                if let (Some(clip_id), Some((note_id, note))) = (midi_clip(), a_note()) {
                    let track_id = rng.pick(&doc.tracks, 8).unwrap_or(doc.master_bus_track_id);
                    let delta_pitch = if note.pitch < 64 { 1_i8 } else { -1 };
                    // 夹住下界：start_tick + delta_tick 必须 >= 0，否则 Op 不可应用。
                    let floor = -i64::try_from(note.start_tick).unwrap_or(i64::MAX);
                    let delta_tick = (i64::from(rng.byte()) - 128).max(floor);
                    return Op::MoveNote {
                        track_id,
                        clip_id,
                        note_id: *note_id,
                        delta_tick,
                        delta_pitch,
                    };
                }
            }
            3 => {
                if let (Some(clip_id), Some((note_id, note))) = (midi_clip(), a_note()) {
                    let track_id = rng.pick(&doc.tracks, 8).unwrap_or(doc.master_bus_track_id);
                    return Op::ModifyNoteVelocity {
                        track_id,
                        clip_id,
                        note_id: *note_id,
                        old_vel: note.velocity,
                        new_vel: rng.byte() % 128,
                    };
                }
            }
            4 => {
                if has_room(doc)
                    && let (Some(track_id), Some(clip_id)) =
                        (rng.pick(&doc.tracks, 8), rng.pick(&doc.clip_pool, 8))
                {
                    let id = fresh(counter);
                    return Op::AddClipPlacement {
                        track_id,
                        placement: ClipPlacement {
                            id,
                            clip_id,
                            start_tick: u64::from(rng.byte()) * 240,
                            duration_ticks: 960,
                            loop_config: LoopConfig::default(),
                            muted: false,
                        },
                    };
                }
            }
            5 => {
                let candidate = doc
                    .tracks
                    .values()
                    .find(|track| !track.clips.is_empty())
                    .and_then(|track| {
                        track
                            .clips
                            .values()
                            .next()
                            .map(|placement| (track.id, *placement))
                    });
                if let Some((track_id, placement)) = candidate {
                    return Op::RemoveClipPlacement {
                        track_id,
                        placement_id: placement.id,
                        previous_placement: placement,
                    };
                }
            }
            6 => {
                let candidate = doc
                    .tracks
                    .values()
                    .find(|track| !track.clips.is_empty())
                    .and_then(|track| {
                        track
                            .clips
                            .values()
                            .next()
                            .map(|placement| (track.id, placement.id, placement.start_tick))
                    });
                if let Some((track_id, placement_id, start_tick)) = candidate {
                    return Op::MoveClipPlacement {
                        track_id,
                        placement_id,
                        old_start_tick: start_tick,
                        new_start_tick: start_tick + 480,
                    };
                }
            }
            7 => {
                if has_room(doc) {
                    return Op::AddTrack {
                        track: TrackV3 {
                            id: fresh(counter),
                            name: "Generated".to_owned(),
                            kind: TrackKind::Midi,
                            ..TrackV3::default()
                        },
                    };
                }
            }
            8 => {
                let removable = doc
                    .tracks
                    .values()
                    .find(|track| {
                        track.id != doc.master_bus_track_id && track.kind != TrackKind::Master
                    })
                    .cloned();
                if let Some(track) = removable {
                    return Op::RemoveTrack {
                        track_id: track.id,
                        previous_track: track,
                    };
                }
            }
            9 => {
                if has_room(doc)
                    && let (Some(source), Some(destination)) = (
                        pick_node(rng, &doc.routing_graph.nodes),
                        pick_node(rng, &doc.routing_graph.nodes),
                    )
                {
                    let id = fresh(counter);
                    return Op::ConnectRouting {
                        edge: RoutingEdge {
                            id,
                            source_node: source,
                            destination_node: destination,
                            kind: RoutingKind::SendToAux,
                            gain_db: Some(-6.0),
                        },
                    };
                }
            }
            10 => {
                if let Some(edge_id) = rng.pick(&doc.routing_graph.edges, 8) {
                    let edge = doc.routing_graph.edges[&edge_id];
                    return Op::DisconnectRouting {
                        edge_id,
                        previous_edge: edge,
                    };
                }
            }
            11 => {
                if let Some(edge_id) = rng.pick(&doc.routing_graph.edges, 8) {
                    let gain = doc.routing_graph.edges[&edge_id].gain_db;
                    let new_gain = Some(f32::from(rng.byte()) - 12.0);
                    return Op::SetRoutingGain {
                        edge_id,
                        old_gain_db: gain,
                        new_gain_db: new_gain,
                    };
                }
            }
            12 => {
                if has_room(doc)
                    && let Some(track_id) = rng.pick(&doc.tracks, 8)
                {
                    let len = doc.tracks[&track_id].devices.len();
                    let slot_index = rng.below(len + 1);
                    return Op::InsertDevice {
                        track_id,
                        slot_index,
                        device: DeviceDefinition {
                            id: fresh(counter),
                            name: "Generated".to_owned(),
                            ..DeviceDefinition::default()
                        },
                    };
                }
            }
            13 => {
                let candidate = doc
                    .tracks
                    .values()
                    .find(|track| !track.devices.is_empty())
                    .and_then(|track| {
                        track
                            .devices
                            .first()
                            .map(|device| (track.id, device.clone()))
                    });
                if let Some((track_id, previous_device)) = candidate {
                    return Op::RemoveDevice {
                        track_id,
                        slot_index: 0,
                        previous_device,
                    };
                }
            }
            14 => {
                if let Some(track_id) = rng.pick(&doc.tracks, 8) {
                    let track = &doc.tracks[&track_id];
                    let target = match rng.below(3) {
                        0 if !track.devices.is_empty() && !track.devices[0].params.is_empty() => {
                            AutomationTarget::DeviceParam {
                                track_id,
                                slot_index: 0,
                                param_index: 0,
                            }
                        }
                        1 if !track.macros.is_empty() => AutomationTarget::Macro {
                            track_id,
                            macro_index: 0,
                        },
                        2 => AutomationTarget::TrackPan { track_id },
                        _ => AutomationTarget::TrackVolume { track_id },
                    };
                    if let Ok(old_val) = read_param(doc, target) {
                        let new_val = match target {
                            AutomationTarget::TrackPan { .. } => 0.5,
                            AutomationTarget::Macro { .. } => 0.25,
                            _ => f32::from(rng.byte()) - 24.0,
                        };
                        return Op::SetParam {
                            target,
                            old_val,
                            new_val,
                        };
                    }
                }
            }
            15 => {
                let candidate = doc
                    .tracks
                    .values()
                    .find(|track| !track.macros.is_empty())
                    .map(|track| (track.id, track.macros[0].value));
                if let Some((track_id, old_val)) = candidate {
                    return Op::SetMacro {
                        track_id,
                        macro_index: 0,
                        old_val,
                        new_val: 0.5,
                    };
                }
            }
            16 => {
                if let Some(track_id) = rng.pick(&doc.tracks, 8) {
                    let target = AutomationTarget::TrackVolume { track_id };
                    let existing = doc.tracks[&track_id]
                        .automation_lanes
                        .get(&target)
                        .and_then(|lane| lane.points.values().next().copied());
                    let (point_id, old_point) = match existing {
                        Some(point) => (point.id, Some(point)),
                        None => (fresh(counter), None),
                    };
                    return Op::SetAutomationPoint {
                        target,
                        point_id,
                        old_point,
                        new_point: AutomationPoint {
                            id: point_id,
                            tick: u64::from(rng.byte()) * 240,
                            value: 0.5,
                            curve: crate::music::CurveType::Linear,
                        },
                    };
                }
            }
            17 => {
                let candidate = doc.tracks.values().find_map(|track| {
                    track.automation_lanes.iter().find_map(|(target, lane)| {
                        lane.points.values().next().map(|point| (*target, *point))
                    })
                });
                if let Some((target, previous_point)) = candidate {
                    return Op::RemoveAutomationPoint {
                        target,
                        point_id: previous_point.id,
                        previous_point,
                    };
                }
            }
            18 => {
                if let Some(section_id) = rng.pick(&doc.sections, 8) {
                    let previous = doc.sections[&section_id].clone();
                    return Op::SetSection {
                        section_id,
                        old_section: Some(previous.clone()),
                        new_section: SectionV3 {
                            id: previous.id,
                            name: previous.name.clone(),
                            start_tick: previous.start_tick,
                            end_tick: previous.end_tick + 960,
                            color: previous.color.clone(),
                        },
                    };
                }
                if has_room(doc) {
                    let id = fresh(counter);
                    return Op::SetSection {
                        section_id: id,
                        old_section: None,
                        new_section: SectionV3 {
                            id,
                            name: "Generated".to_owned(),
                            start_tick: 0,
                            end_tick: 960,
                            color: None,
                        },
                    };
                }
            }
            19 => {
                if let Some(section_id) = rng.pick(&doc.sections, 8) {
                    let previous_section = doc.sections[&section_id].clone();
                    return Op::RemoveSection {
                        section_id,
                        previous_section,
                    };
                }
            }
            20 => {
                if let Some(scene_id) = rng.pick(&doc.scenes, 8) {
                    let previous = doc.scenes[&scene_id].clone();
                    return Op::SetScene {
                        scene_id,
                        old_scene: Some(previous.clone()),
                        new_scene: SceneV3 {
                            id: previous.id,
                            name: previous.name.clone(),
                            tempo: Some(120.0),
                            color: previous.color.clone(),
                        },
                    };
                }
                if has_room(doc) {
                    let id = fresh(counter);
                    return Op::SetScene {
                        scene_id: id,
                        old_scene: None,
                        new_scene: SceneV3 {
                            id,
                            name: "Generated".to_owned(),
                            tempo: None,
                            color: None,
                        },
                    };
                }
            }
            21 => {
                if let Some(scene_id) = rng.pick(&doc.scenes, 8) {
                    let previous_scene = doc.scenes[&scene_id].clone();
                    return Op::RemoveScene {
                        scene_id,
                        previous_scene,
                    };
                }
            }
            22 => {
                // 泳道增改：目标固定用 `TrackPan`（与 16/17 的 `TrackVolume` 分开，
                // 两条泳道的生成序列互不干扰）。写模式**恒定非 Off** ⇒ 新泳道绝不与
                // 隐式泳道逐位不可区分（否则前置条件会拒绝），且与任何旧状态都不同。
                if let Some(track_id) = rng.pick(&doc.tracks, 8) {
                    let target = AutomationTarget::TrackPan { track_id };
                    let existing = doc.tracks[&track_id].automation_lanes.get(&target).cloned();
                    let mut new_lane = existing
                        .clone()
                        .unwrap_or_else(|| AutomationLane::implicit(target));
                    new_lane.write_mode = match new_lane.write_mode {
                        crate::project::AutomationWriteMode::Touch => {
                            crate::project::AutomationWriteMode::Latch
                        }
                        _ => crate::project::AutomationWriteMode::Touch,
                    };
                    return Op::SetAutomationLane {
                        target,
                        old_lane: existing,
                        new_lane,
                    };
                }
            }
            23 => {
                let candidate = doc.tracks.values().find_map(|track| {
                    track.automation_lanes.iter().find_map(|(target, lane)| {
                        if lane.is_implicit() {
                            None
                        } else {
                            Some((*target, lane.clone()))
                        }
                    })
                });
                if let Some((target, previous_lane)) = candidate {
                    return Op::RemoveAutomationLane {
                        target,
                        previous_lane,
                    };
                }
            }
            _ => {
                if has_room(doc)
                    && rng.below(4) == 0
                    && let (Some(track_id), Some(clip_id)) = (rng.pick(&doc.tracks, 8), midi_clip())
                {
                    return Op::Batch {
                        ops: vec![Op::AddNote {
                            track_id,
                            clip_id,
                            note: MidiNote::new(fresh(counter), 0, 72, 120),
                        }],
                        description: "generated batch".to_owned(),
                    };
                }
            }
        }

        // 兜底：`SetRoutingGain`（只要有一条边就必然可应用），否则新增音轨。
        if let Some(edge_id) = rng.pick(&doc.routing_graph.edges, 8) {
            let gain = doc.routing_graph.edges[&edge_id].gain_db;
            return Op::SetRoutingGain {
                edge_id,
                old_gain_db: gain,
                new_gain_db: Some(-1.0),
            };
        }
        Op::AddTrack {
            track: TrackV3 {
                id: fresh(counter),
                name: "Fallback".to_owned(),
                ..TrackV3::default()
            },
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 32,
            max_shrink_iters: 64,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// [MUST-GATE-010, ROAD-M1-006, TEST-SPEC-001] 状态树逆向幂等性。
        ///
        /// 随机生成 `sequence_steps()` 步**保证可应用**的领域操作并逐步 apply，
        /// 然后按逆序逐步 undo，断言文档与初始状态**逐字节严格守恒**。
        /// 序列长度由 `YEBAN_PROPTEST_CASES` 控制（CI 上自动取 10,000）。
        #[test]
        fn state_tree_is_conserved_under_reverse_undo(seed in any::<u64>()) {
            let steps = sequence_steps();
            let initial = fixture_document();
            let mut doc = initial.clone();
            let mut rng = StableRng::new(seed);
            let mut counter: u128 = 0;
            let mut applied: Vec<Op> = Vec::with_capacity(steps);

            for step in 0..steps {
                let op = generate_op(&mut rng, &doc, &mut counter);
                op.apply(&mut doc).unwrap_or_else(|error| {
                    panic!("第 {step} 步生成的 {} 必须可应用: {error}", op.name())
                });
                if step % 64 == 0 {
                    doc.validate().unwrap_or_else(|error| {
                        panic!("第 {step} 步 ({}) 之后文档必须合法: {error}", op.name())
                    });
                }
                applied.push(op);
            }
            prop_assert_eq!(applied.len(), steps);

            for (index, op) in applied.iter().rev().enumerate() {
                let inverse = op.invert(&doc).unwrap_or_else(|error| {
                    panic!("撤销第 {index} 步 ({}) 时求逆失败: {error}", op.name())
                });
                inverse.apply(&mut doc).unwrap_or_else(|error| {
                    panic!("撤销第 {index} 步 ({}) 时逆操作失败: {error}", op.name())
                });
            }
            doc.validate().expect("撤销到底后文档必须仍然合法");
            prop_assert_eq!(&doc, &initial);
        }
    }
}
