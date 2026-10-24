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
//! **分支顺序不是契约**（⛔ 不要靠"必须放末尾"）：历史上两条判据曾**按下标**读 ——
//! `origin.oneOf[0]` 当单元枚举；把 `McpEdit` 插到 `[0]` 会让它们读到**错的形状**
//! （实测：106 passed / 3 failed）。⚠ 那个"挪动时要同步下标"的约定**⛔ 不是守卫**：
//! 顺序一变，按下标读会**静默**取到 `McpProposal`／`McpEdit` 分支而判据仍绿。
//! ⇒ 现已改成**按内容定位**（谁的 `enum` 是数组，谁就是单元分支；找不到即**响亮失败**），
//! 由 `origin_unit_branch_is_found_regardless_of_branch_order` 钉住顺序无关性。
//! ⚠ 唯一仍按**位置**读的是 `op.oneOf[*].required[0]`（每个分支只需一个变体键 ⇒ 与顺序无关）。
//! 追平原委与证据见 `docs/ledger/origin-contract-notes.md`。
//!
//! 本模块的契约一致性有两条**直接读契约文件**的判据（不手抄第二份事实源）：
//! `op_variants_match_ops_schema_exactly` 与
//! `origin_variants_match_ops_schema_origin_one_of`（另有
//! `mcp_edit_origin_shape_matches_its_contract_branch` 逐字段核对新分支的载荷）。

use serde::{Deserialize, Serialize};

use crate::error::ModelError;
use crate::ids::{AssetHash, EntityId};
use crate::music::MidiNote;
use crate::project::{
    AssetMetadata, AutomationLane, AutomationPoint, AutomationTarget, ClipContent, ClipPlacement,
    ClipPoolEntry, DeviceDefinition, MAX_BPM, MIN_BPM, RoutingEdge, SamplerBinding, SceneV3,
    SectionV3, TrackV3, TransportConfig, YebanProjectV1,
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
    /// 设置音符的**时间位置**（起点 tick 与时值），不改任何其他字段。
    ///
    /// ⚠ **本变体补上了什么**：⭐ 在它出现之前，[`MidiNote::start_tick`] /
    /// [`MidiNote::duration_ticks`] **只能在 [`Op::AddNote`] 时**给定 ——
    /// [`Op::MoveNote`] 只按增量平移起点、⛔ 改不了时值，而"删 + 加"会在操作日志层
    /// 留下**两条** op，并要求调用方自己把音高 / 力度 / 概率 / 连击 / 微时序 / 滑音 /
    /// 弯音曲线等全部表现力字段抄一遍（漏抄一个就是**静默的表现力丢失**）。
    ///
    /// ⚠ **为何安全（完全可逆）**：⭐ 两个字段都是**标量** ⇒ 自带
    /// `old_start_tick` / `old_duration_ticks` 就能逐字节还原，⛔ 不读历史。
    /// 本变体**只写**这两个字段：⛔ 不碰 `id` / `pitch` / `velocity` / `probability` /
    /// `ratchet` / `micro_timing_ticks` / `slide` / `pitch_bend_curve` / `syllable` /
    /// `phonemes`。
    ///
    /// ⚠ **前置条件**：① 音轨与音符都必须存在；② 文档现值必须**逐字段**等于
    /// `old_start_tick` / `old_duration_ticks`（陈旧载荷 ⇒ [`ModelError::OpStateMismatch`]，
    /// 与 [`Op::ModifyNoteVelocity`] 同口径）；③ `new_duration_ticks` **必须 ≥ 1**
    /// （⭐ 复用 [`MidiNote::validate`] 已有的 [`ModelError::ZeroDuration`]，
    /// ⛔ 不自造新错误变体）。`new_start_tick` 是 `u64` 全域 —— 起点没有上界，
    /// 与 [`Op::AddNote`] 的口径一致。
    ///
    /// ⚠ **与摆放窗口的关系**：音符住在 `ClipPoolEntry::content` 里，播放时引擎把音符
    /// **裁剪到摆放区间** ⇒ 把时值拉长到超出摆放末端**不会**改写摆放，也⛔ 不会让
    /// 文档非法；要改摆放长度请用 [`Op::SetClipPlacementDuration`]。
    SetNoteTiming {
        /// 所属音轨。
        track_id: EntityId,
        /// 所属片段。
        clip_id: EntityId,
        /// 音符身份。
        note_id: EntityId,
        /// 修改前的起点 tick（来自当前文档）。
        old_start_tick: u64,
        /// 修改前的时值 tick（来自当前文档）。
        old_duration_ticks: u64,
        /// 修改后的起点 tick。
        new_start_tick: u64,
        /// 修改后的时值 tick（必须 ≥ 1）。
        new_duration_ticks: u64,
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
    /// 设置**片段池条目的显示名**（[`ClipPoolEntry::name`]）。
    ///
    /// ⚠ **本变体补上了什么**：⭐ **在它出现之前，`ClipPoolEntry::name` 在操作日志层
    /// 不可表达** —— 唯一的入口是 [`Op::AddClip`]（新建时把整个 `ClipPoolEntry` 交进去），
    /// 而它之后**没有任何 `Op` 能改名** ⇒ 改名只能"删 + 加"，⭐ 而 [`Op::RemoveClip`]
    /// 还要求**没有任何摆放引用它**（[`ModelError::ClipInUse`]）⇒ 调用方得先把该片段的
    /// **每一处摆放**拆掉、改完名再摆回去。这是 `ARCH-OPS-001` 那一族的又一处
    /// （与 `D27` 补 `AddClip`、[`Op::SetTrackName`]、[`Op::SetAuthor`] 的成因同型）。
    ///
    /// ⚠ **为何安全**：⭐ `name` 是**纯展示注记**、⛔ 无身份语义 —— 条目身份是
    /// [`ClipPoolEntry::id`]，而 `clip_pool` 的**键**就是那个 `id`
    /// （`BTreeMap<EntityId, ClipPoolEntry>`）⇒ ⭐ 改 `name` **不动键**、⛔ 不让任何
    /// 摆放（引用 `clip_id`）悬空。本变体只写这一个字段：⛔ 不碰 `id` / `content`，
    /// 也⛔ 不碰 `clip_pool` 的键集合与任何摆放。
    ///
    /// ⚠ 命名用 `old_*` / `new_*`（**set** 类，照 [`Op::SetTrackName`] / [`Op::SetAuthor`]），
    /// ⛔ **不是** `previous_*`（那是 delete/remove 类的惯例，见 [`Op::RemoveClip`]）。
    SetClipName {
        /// 目标片段身份（即 `clip_pool` 的键）。
        clip_id: EntityId,
        /// 修改前的显示名。
        old_name: String,
        /// 修改后的显示名。
        new_name: String,
    },
    /// 设置**音频片段的增益**（`ClipContent::Audio::gain_db`，单位 dB）。
    ///
    /// ⚠ **本变体补上了什么**：`gain_db` 在创建时由 `yeban_import_audio` 的
    /// `gainDb` 写一次（`crates/yeban-mcp/src/domain/import_audio.rs:396` 解析、
    /// `:501` 落进 `ClipContent::Audio`），而 `Op` 全集此前**没有任何变体写得进它**
    /// ⇒ ⭐ **音频片段建得出、增益改不了**。这是 `ARCH-OPS-001` 那一族的又一处
    /// （与 [`Op::SetClipName`] / [`Op::SetTitle`] 同型）。
    ///
    /// ⚠ **寻址与内容类型门**：目标由载荷自带的 `clip_id`（`clip_pool` 的键）给出。
    /// 片段必须**存在**（[`ModelError::ClipNotFound`]）且内容必须是
    /// `ClipContent::Audio`，否则 [`ModelError::ClipContentKindMismatch`] ——
    /// MIDI 片段没有 `gain_db` 可读，**⛔ 不 panic、⛔ 不静默**。
    ///
    /// ⚠ **值域照抄现状**：`ClipContent::validate` 对 `gain_db` **只**查有限性
    /// （字段名 `clip.audio.gain_db`，`crates/yeban-model/src/project.rs:1040-1047`），
    /// **没有任何 min/max、没有 dB 上下限** ⇒ 本变体收**任意有限 `f32`**，
    /// ⛔ 不发明范围；非有限（NaN/±Inf）由前置条件拒（[`ModelError::NonFiniteValue`]）。
    ///
    /// ⚠ 命名用 `old_*` / `new_*`（**set** 类，照 [`Op::SetRoutingGain`] /
    /// [`Op::SetClipName`]），⛔ **不是** `previous_*`；`f32` 的比较是**逐位**的
    /// （`same_f32`，`-0.0` 与 `0.0` 位不同 ⇒ 陈旧载荷一律 `OpStateMismatch`）。
    SetClipGain {
        /// 目标片段身份（即 `clip_pool` 的键）。
        clip_id: EntityId,
        /// 修改前的增益 (dB)。
        old_gain_db: f32,
        /// 修改后的增益 (dB)。
        new_gain_db: f32,
    },
    /// 切换**设备旁通**（[`DeviceDefinition::bypassed`]）。
    ///
    /// ⚠ **本变体补上了什么**：⭐ **在它出现之前，`DeviceDefinition::bypassed` **只能**在
    /// [`Op::InsertDevice`] 时由调用方给值**，而**它之后没有任何 `Op` 能改它** ⇒ ⭐ 今天
    /// **连一次"旁通这台设备"都做不出来**（`Default` 与所有夹具都写死 `false`）。⚠ 这是
    /// `ARCH-OPS-001` 那一族的又一处（与 `D27` 补 `AddClip`、[`Op::SetTrackName`]、
    /// [`Op::SetClipName`]、[`Op::SetDeviceName`]、[`Op::SetAuthor`] 的成因同型）。
    ///
    /// ⚠ **为何安全（可逆）**：⭐ 旁通是**布尔开关** ⇒ 切回去**即完全复原**（无副作用、
    /// 无丢失信息）。⚠ **它有真正的行为重量** —— [`DeviceDefinition::bypassed`] 的字段文档
    /// 逐字写着「**必需** [ADR-0001 D43]：缺了它，一个被旁通的设备会被**静默接回信号链**
    /// —— 这是"听得出、但读文件时看不见"的行为反转」，而下游确实读它：
    /// [`crate::ops`] 之外，`yeban-engine` 的延迟汇总按它跳过该设备
    /// （`LatencyTable::from_tracks`：**旁通设备不计入**，对相位没有贡献）、
    /// `yeban-engine::drums` 对 `bypassed` 的设备**整体忽略**。
    /// ⭐ ⚠ **PDC 无需联动**：延迟表是**从当前文档推导的投影** ⇒ 本变体改完文档之后，
    /// 下次推导自然反映新旁通位（⭐ 该因果由 `latency_table_sums_model_device_latency_and_skips_bypassed` 守护）。
    ///
    /// ⚠ 本变体只写这一个字段：⛔ 不碰 `id` / `name` / `kind` / `params` /
    /// `latency_samples`（⭐ 延迟值**照旧保留**，只是旁通期间不计入相位）。
    /// ⚠ **寻址用 `track_id` + `device_id` 两个身份、⛔ 不用下标**（`devices` 是 `Vec`，
    /// 增删会移位下标）。⚠ 命名用 `old_*` / `new_*`（**set** 类，照 [`Op::SetTrackMute`]），
    /// ⛔ **不是** `previous_*`。
    SetDeviceBypassed {
        /// 目标音轨（设备的容器）。
        track_id: EntityId,
        /// 目标设备身份。
        device_id: EntityId,
        /// 修改前的旁通状态。
        old_bypassed: bool,
        /// 修改后的旁通状态。
        new_bypassed: bool,
    },
    /// 设置**设备显示名**（[`DeviceDefinition::name`]）。
    ///
    /// ⚠ **本变体补上了什么**：⭐ **在它出现之前，`DeviceDefinition::name` 在操作日志层
    /// 不可表达** —— 唯一的入口是 [`Op::InsertDevice`]（插入时把整个 `DeviceDefinition`
    /// 交进去），而它之后**没有任何 `Op` 能改名** ⇒ 改名只能"删 + 加"。⭐ 这个实体上
    /// "删 + 加"有**两个**坏处：⭐ ① 同 [`Op::SetClipName`] / [`Op::SetTrackName`]，
    /// 设备的身份会变（对 PDC 与参数自动化而言那是另一个设备）；⭐ ② **`TrackV3::devices`
    /// 是 `Vec`** ⇒ 既有 `InsertDevice` / `RemoveDevice` 用 `slot_index` 寻址，
    /// 而 `remove` / `insert` 会**移位下标** ⇒ 调用方还得自己重算链上每一格的下标。
    /// 这是 `ARCH-OPS-001` 那一族的又一处（与 `D27` 补 `AddClip`、[`Op::SetTrackName`]、
    /// [`Op::SetClipName`]、[`Op::SetAuthor`] 的成因同型）。
    ///
    /// ⚠ **为何安全**：⭐ `name` 是**纯展示注记**、⛔ 无身份语义 —— 设备身份是
    /// [`DeviceDefinition::id`] ⇒ ⭐ 改 `name` ⛔ 不动身份、⛔ 不动下标、⛔ 不让任何
    /// 引用悬空。本变体只写这一个字段：⛔ 不碰 `id` / `kind` / `params` /
    /// `latency_samples`，⭐ **也⛔ 不碰 `bypassed`** —— ⚠ **那是有意不做的**：
    /// `bypassed` 是**行为开关**（与 [`TrackV3::solo_safe`] 同类），⛔ 不属于"显示名"这一票。
    ///
    /// ⚠ **寻址用 `track_id` + `device_id` 两个身份、⛔ 不用下标** —— 因为 `devices` 是
    /// `Vec` 且增删会移位下标（见上）。⚠ 命名用 `old_*` / `new_*`（**set** 类，照
    /// [`Op::SetTrackName`] / [`Op::SetClipName`]），⛔ **不是** `previous_*`。
    SetDeviceName {
        /// 目标音轨（设备的容器）。
        track_id: EntityId,
        /// 目标设备身份。
        device_id: EntityId,
        /// 修改前的显示名。
        old_name: String,
        /// 修改后的显示名。
        new_name: String,
    },
    /// 设置摆放**长度**（[`ClipPlacement::duration_ticks`]）。
    ///
    /// ⚠ **本变体补上了什么**：⭐ **在它出现之前，`ClipPlacement::duration_ticks` **只在创建时**
    /// 能给**（[`Op::AddClipPlacement`] 的载荷带它）⇒ ⭐ **创建之后改不了长度**（⭐ 修剪/拉伸
    /// 都做不了）；⭐ 而 [`Op::MoveClipPlacement`] **只写 `start_tick`** ⇒ 想改长度只能
    /// "删掉摆放再重建"，⭐ 那会**换掉摆放身份**。⚠ `ARCH-OPS-001` 那一族的又一处。
    ///
    /// ⚠ **为何安全（完全可逆）**：⭐ 长度是**标量窗口** ⇒ 存 `old_duration_ticks` 即完全复原。
    /// ⭐ ⚠ **"改短再改长"⛔ 不丢音符**：⭐ 音符住在 [`ClipPoolEntry::content`] 里，⭐ 而
    /// `duration_ticks` 只决定**播放窗口** —— ⭐ `yeban-engine` 的注释逐字「音符时值被
    /// **裁剪到摆放区间** `[start_tick, start_tick + duration_ticks)`」，⭐ ⚠ **那个裁剪发生在
    /// 快照/渲染构建期、⛔ 不是在 `apply` 时截断数据**。
    ///
    /// ⚠ ⭐ **与 `loop_config` 的关系（已核实、如实登记）**：⭐ **校验层**上两者**解耦** ——
    /// [`ClipPlacement::validate`] 只做两件独立的事（`duration_ticks == 0 ⇒ ZeroDuration`
    /// 与 `loop_config.validate()`），⭐ 而 [`LoopConfig`] 用的是它**自己的摆放相对 tick 区间**
    /// （`{enabled, start_tick, end_tick}`）、⭐ **没有任何"必须与 `duration_ticks` 一致"的校验**
    /// ⇒ ⭐ **本变体⛔ 不动 `loop_config`**。⚠ ⭐ **但有一处**行为关联**（`#241`）**：
    /// `yeban-mcp` 的离线渲染**真的展开**循环，`N = ceil((duration_ticks - start) / (end - start))`
    /// ⇒ ⭐ **改长度会改变重复遍数**（⭐ 那是**长度本来就该决定的量**、⭐ 两个字段改后仍各自自洽；
    /// 区间落到新末端之外时按"不重复"播放，见 [`crate::ClipPlacement::loop_repeat`]）。
    ///
    /// ⚠ **值域**：⭐ `new_duration_ticks` **必须 ≥ 1** ⇒ ⭐ `0` 复用既有
    /// [`ModelError::ZeroDuration`]（⭐ 与 [`ClipPlacement::validate`] 同一口径、⛔ 不自造新错误）。
    /// ⚠ 本变体只写这一个字段：⛔ 不碰 `id` / `clip_id` / `start_tick` / `muted` / `loop_config`。
    /// ⚠ **寻址用 `track_id` + `placement_id` 双定位**（`clips` 是 `BTreeMap`）。
    /// ⚠ 命名用 `old_*` / `new_*`（**set** 类，照 [`Op::MoveClipPlacement`] 的同族），
    /// ⛔ **不是** `previous_*`。
    SetClipPlacementDuration {
        /// 目标音轨（摆放的容器）。
        track_id: EntityId,
        /// 目标摆放身份（即 `clips` 的键）。
        placement_id: EntityId,
        /// 修改前的长度（tick）。
        old_duration_ticks: u64,
        /// 修改后的长度（tick，**必须 ≥ 1**）。
        new_duration_ticks: u64,
    },
    /// 切换**摆放静音**（[`ClipPlacement::muted`]）。
    ///
    /// ⚠ **本变体补上了什么**：⭐ **在它出现之前，`ClipPlacement::muted` **只在创建时**能给
    /// （[`Op::AddClipPlacement`] 的载荷带它）**，而**它之后没有任何 `Op` 能改它** ——
    /// [`Op::MoveClipPlacement`] **只写 `start_tick`** ⇒ ⭐ **创建之后改不了静音**；⭐ 想改
    /// 只能"删掉摆放再建一个"，⭐ 而重建会**换掉摆放身份**（`EntityId`）。
    /// ⚠ 这是 `ARCH-OPS-001` 那一族的又一处（与 `D27` 补 `AddClip`、[`Op::SetTrackName`]、
    /// [`Op::SetClipName`]、[`Op::SetDeviceName`]、[`Op::SetDeviceBypassed`] 的成因同型）。
    ///
    /// ⚠ **为何安全（可逆）**：⭐ 静音是**布尔开关** ⇒ 切回去即完全复原。⚠ **它有真正的
    /// 行为重量** —— 四处下游读它并按此**不发声/不导出**：`yeban-engine::snapshot`、
    /// `yeban-render::{als,logic}`、以及 `yeban-mcp::domain::render` 的离线路径。
    ///
    /// ⚠ 本变体只写这一个字段：⛔ 不碰 `id` / `clip_id` / `start_tick`，⭐ **也⛔ 不碰
    /// `duration_ticks` 与 `loop_config`** —— ⚠ **那两个是**有意不做**的**：⭐ 改
    /// `duration_ticks` 会变**摆放长度**（与"静音"不同类）、⭐ `loop_config` 是结构体
    /// （要另议），⛔ 不属于本票的"开关"语义。
    /// ⚠ **寻址用 `track_id` + `placement_id` 双定位**（摆放住在音轨里）—— ⚠ 但本条
    /// **不必防下标移位**：⭐ `TrackV3::clips` 是 **`BTreeMap<EntityId, ClipPlacement>`**
    /// （键 = 摆放身份，⭐ 稳定）⇒ ⭐ 与 `devices`（`Vec`）那处**不同**。
    /// ⚠ 命名用 `old_*` / `new_*`（**set** 类，照 [`Op::SetTrackMute`] / [`Op::SetDeviceBypassed`]），
    /// ⛔ **不是** `previous_*`。
    SetClipPlacementMuted {
        /// 目标音轨（摆放的容器）。
        track_id: EntityId,
        /// 目标摆放身份（即 `clips` 的键）。
        placement_id: EntityId,
        /// 修改前的静音状态。
        old_muted: bool,
        /// 修改后的静音状态。
        new_muted: bool,
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
    /// 设置**走带配置**（[`TransportConfig`] 的三个字段整体替换）。
    ///
    /// 为什么必须有这个变体（而不是复用 [`Op::SetParam`]）：`SetParam` 的载荷是 `f32`
    /// 与 [`AutomationTarget`]，而 [`TransportConfig`] 里是 `bool` / `u8` /
    /// [`LaunchQuantization`] ⇒ 走带配置在操作日志层**不可表达**（与 `SetTrackMute`
    /// 不得不用 `bool` 载荷、`D12` / `D27` / `D42` 补 `RemoveSection` / `AddClip` /
    /// `SetAutomationLane` 的理由**同型**：载荷类型对不上，不是"懒得复用"）。
    ///
    /// ⚠ **整体替换而不是逐字段**：三个字段是**一个作者选择**（`ADR-0001 D43` 的
    /// "必须被显式陈述"），拆成三条 `Op` 会让"撤销一步"只回退其中一个字段、
    /// 留下一个作者从未选过的组合 ⇒ 故这里是**一条** `Op`。
    SetTransportConfig {
        /// 修改前的完整走带配置。
        old_config: TransportConfig,
        /// 修改后的完整走带配置。
        new_config: TransportConfig,
    },
    /// 设置**速度**（顶层 `bpm`，`[MODEL-AST-002]`）。
    ///
    /// 为什么必须有这个变体：`bpm` 是**顶层唯一权威**且 `TransportConfig` **有意不含它**
    /// （见 `TransportConfig` 的文档）⇒ 在它之前，`Op` 全集**无法表达"改速度"** ——
    /// 打开工程之后速度就固定了。契约 `schemas/project.schema.json` 早已给出合法区间
    /// [`MIN_BPM`]..=[`MAX_BPM`]（`20.0..=999.0`）⇒ 数据层早就准备好被改。
    ///
    /// ⚠ `bpm` 是 **`f64`**：`validate` 的相等判定用**逐位**比较（`to_bits`），
    /// 因此 `NaN` 的两个实例**不相等**，且 `NaN` 落在区间外 ⇒ 两条独立防线都拦住它。
    /// 载荷与文档现值都必须是**有限且在区间内**的值（⛔ 不靠 `apply` 之后的
    /// `YebanProjectV1::validate` 兜底 —— 那会让"施加成功但文档非法"成为可能）。
    SetTempo {
        /// 修改前的速度（BPM）。
        old_bpm: f64,
        /// 修改后的速度（BPM）。
        new_bpm: f64,
    },
    /// 设置**作者署名**（顶层 `author`，`[MODEL-AST-002]`）。
    ///
    /// 为什么必须有这个变体：`author` 在 `project.schema.json` 里**有字段定义**
    /// （非必需），而 `Op` 全集此前**没有任何变体写得进它** ⇒ ⭐ **打开一个作者写好的
    /// 工程之后，⭐ 没有任何通路能改署名**。它与 [`Op::SetTransportConfig`] / [`Op::SetTempo`]
    /// 同族（顶层文档字段），但载荷是 **`String`**（⛔ 不是 `Copy`）⇒ `apply` 与
    /// `structural_inverse` 都必须**克隆**（照 `AddClip` 那类含 `String` 载荷的先例）。
    ///
    /// ⚠ 命名用 `old_*` / `new_*`（**set** 类），⛔ **不是** `previous_*` —— 后者在本仓
    /// 是 **delete / remove** 类的惯例（`RemoveClip` / `RemoveTrack`），那里的载荷只为撤销存在。
    SetAuthor {
        /// 修改前的署名。
        old_author: String,
        /// 修改后的署名。
        new_author: String,
    },
    /// 设置**作品标题**（顶层 `title`，`[MODEL-AST-002]`）。
    ///
    /// 为什么必须有这个变体：`title` 是**顶层唯一权威**且**必需**的字段
    /// （`schemas/project.schema.json` 的 `required` 里有它），而 `Op` 全集此前
    /// **没有任何变体写得进它** —— 全仓唯一的**结构写**点是创建工程时的
    /// `domain/project_create.rs:370` ⇒ ⭐ **工程建得出、标题改不了**。
    /// 它与 [`Op::SetAuthor`] / [`Op::SetTempo`] 同族（顶层文档字段），载荷是
    /// **`String`**（⛔ 不是 `Copy`）⇒ `apply` 与 `structural_inverse` 都必须**克隆**
    /// （照 `SetAuthor` 那类含 `String` 载荷的先例）。
    ///
    /// ⚠ **本变体不收窄值域**：`title` 在契约里是裸 `{"type": "string"}`
    /// （无 `minLength` / `maxLength` / `pattern`），`YebanProjectV1::validate`
    /// 也**不查**它 ⇒ 空串与任意 UTF-8 都是**合法**的明示值（同 [`Op::SetAuthor`]，
    /// ⛔ 不发明"非空"或长度上限）。
    ///
    /// ⚠ 命名用 `old_*` / `new_*`（**set** 类，照 [`Op::SetAuthor`]），⛔ **不是**
    /// `previous_*` —— 后者在本仓是 **delete / remove** 类的惯例。
    SetTitle {
        /// 修改前的标题。
        old_title: String,
        /// 修改后的标题。
        new_title: String,
    },
    /// 声明（或更新）一条**资产索引**（`[MODEL-AST-007]` / `[MUST-GATE-014]`）。
    ///
    /// ⚠ 本变体是**三处已登记缺口的共同根**：在它之前，模型**没有任何 `Op` 能写
    /// `YebanProjectV1::assets`** ⇒ 于是 ① 资产导入链零调用者、② **工程层**许可判定
    /// 无落点、③ [`AssetMetadata::license`] / [`AssetMetadata::media_kind`] 只写不读。
    /// `crates/yeban-mcp/src/domain/import_audio.rs` 的注释逐字记着这件事：
    /// 「工程 assets 索引（许可/原路径/字节数）同样没有 `Op` ⇒ 本工具只登记 `clip_pool`
    /// 条目，不写索引（needs-3）」。
    ///
    /// ## 形状：照 [`Op::SetScene`]（⛔ 不是照 `SetAuthor` / `SetTempo`）
    ///
    /// 三条先例里，`SetScene` / `SetSection` 用 `old_*: Option<T>` **同时编码"创建/更新"**，
    /// 而 `SetAuthor` / `SetTempo` 的 `old_*` 是裸值（只编码"更新"）。本变体选前者，理由：
    ///
    /// - ⭐ **"声明"天然含创建**：资产是**内容寻址**的（键 = `AssetHash`），第一份声明就是
    ///   创建，之后同哈希再声明就是更新（改许可/原路径）。若用裸值，就**没法表达创建**
    ///   —— 而创建恰恰是今天最缺的那一步。
    /// - ⭐ **身份与载荷分离**：`asset_hash` 是**键**（`BTreeMap` 的键），`AssetMetadata::hash`
    ///   是载荷里的**同一身份**。与 `SetScene` 的 `scene_id` / `SceneV3::id` 完全同型 ⇒
    ///   前置条件里要做一次"键 == 载荷身份"的一致性检查。
    /// - ⚠ `SetAuthor` / `SetTempo` 那两个先例的载荷**没有内部身份**（`String` / `f64`），
    ///   所以它们不需要这个检查 —— 它们不适用于本变体。
    SetAsset {
        /// 资产身份（内容寻址的 CAS 键，即 `assets` 映射的键）[MODEL-AST-007]。
        asset_hash: AssetHash,
        /// 声明**之前**该资产的索引项；`None` 表示**本工程尚未声明过**它（创建）。
        old_asset: Option<AssetMetadata>,
        /// 声明**之后**的索引项（含许可、原路径、字节数、介质种类）。
        new_asset: AssetMetadata,
    },
    /// 删除一条资产索引（自带撤销载荷）。与 [`Op::RemoveScene`] 同因补齐。
    ///
    /// ⚠ 它**不是**可选件：[`Op::SetAsset`] 的 `old_asset: None` 编码"创建"，而创建态的
    /// 逆操作**必须**是删除（见 `structural_inverse` 的说明）⇒ 少了本变体，
    /// "创建一个资产"这件事就**撤销不了**。
    ///
    /// ⚠ 它**只**删除 `assets` 里的索引项，⛔ **不碰归档里的资产字节**（字节在
    /// `ProjectArchive` 里，由容器层拥有）—— 与 [`Op::RemoveClip`] 只删 `clip_pool`
    /// 条目同理。
    RemoveAsset {
        /// 资产身份（内容寻址的 CAS 键）。
        asset_hash: AssetHash,
        /// 删除**之前**的索引项（撤销用）。
        previous_asset: AssetMetadata,
    },
    /// 设置音轨**色标**（[`TrackV3::color`]）。
    ///
    /// ⚠ **本变体补上了什么**：⭐ **在它出现之前，`TrackV3::color` 在操作日志层
    /// 不可表达** —— 它只能在**建轨时**由 [`Op::AddTrack`] 的载荷给值，而它之后
    /// **没有任何 `Op` 能改它** ⇒ ⭐ 改色标只能"删轨 + 重建"，⭐ 那会**换掉 `EntityId`**
    /// 并连带丢掉该轨的设备链 / 自动化 / 路由边（⭐ 那些都以 `track_id` 为键）。
    /// ⚠ ⭐ **与本族前三例的对照**：⭐ `SectionV3::color` 与 `SceneV3::color` **已有通路**
    /// （⭐ 走 [`Op::SetSection`]／[`Op::SetScene`] 的"合并式 patch"：**缺省 = 保留现值、
    /// `None` = 清空、`Some` = 设值**）⇒ ⭐ **只有音轨这一处缺**，⛔ 故本单只加这一个变体。
    ///
    /// ⚠ **为何安全**：⭐ `color` 是**纯展示注记**、⛔ 无身份语义（身份是 [`TrackV3::id`]）；
    /// ⚠ 且 [`Option::None`] 是**一等状态**——字段文档逐字写着「`None` 是**一等状态**
    /// （"用主题默认色"）」⇒ ⭐ 设成 `None` ⛔ 不是"丢失"，⭐ 而是"回到主题默认"。
    /// ⭐ 消费者在 `yeban-app` 的 UI 投影（`track_colors()` / `track_color_labels()` /
    /// `track_color_or_default`）⇒ ⭐ **有行为重量**；⚠ 而**非法色值的处理不在模型层**：
    /// ⭐ `parse_hex_color` 返回 `Option`、非法即**回退默认**（⭐ ⛔ 不报错）⇒ ⭐ **本变体
    /// 照 `SectionV3`／`SceneV3` 的既有口径收任意字符串、⛔ 不自造格式校验**。
    ///
    /// ⚠ 本变体只写这一个字段：⛔ 不碰 `id` / `name` / `kind` / `devices` / `macros` /
    /// `automation_lanes` / `clips` / `volume_db` / `pan` / `mute` / `solo` / `solo_safe` /
    /// `folder_id`。
    /// ⚠ 命名用 `old_*` / `new_*`（**set** 类，照 [`Op::SetTrackName`]），⛔ **不是** `previous_*`。
    SetTrackColor {
        /// 目标音轨。
        track_id: EntityId,
        /// 修改前的色标（`None` = 主题默认色）。
        old_color: Option<String>,
        /// 修改后的色标（`None` = 清除 = 回到主题默认色）。
        new_color: Option<String>,
    },
    /// 设置音轨的**采样器绑定**（[`TrackV3::sampler`]，即"这条轨用哪一份采样器定义文件"）。
    ///
    /// ⚠ **本变体补上了什么**：⭐ **在它出现之前，`TrackV3::sampler` 在操作日志层
    /// 不可表达** —— 唯一的入口是 [`Op::AddTrack`]（建轨时把整个 `TrackV3` 交进去），
    /// 而它之后**没有任何 `Op` 能改它** ⇒ 换采样器只能"删轨 + 重建"，⭐ 那会**换掉
    /// `EntityId`**、并连带丢掉该轨的设备链 / 自动化 / 路由边（⭐ 那些都以 `track_id`
    /// 为键）。这是 `ARCH-OPS-001` 那一族的又一处（与 `D27` 补 `AddClip`、
    /// [`Op::SetTrackName`] / [`Op::SetTrackColor`] 的成因同型）。
    ///
    /// ⚠ 载荷是 [`SamplerBinding`]（**一个路径字符串**），⛔ 不是已解析的乐器：
    /// 模型的持久化 AST 里**不放** `SfzInstrument` / PCM / `Vec<f32>`
    /// （理由见 [`SamplerBinding`] 的文档）。本变体因此只是"把这一个字段换个值"。
    ///
    /// ⚠ 两边都是 `Option<SamplerBinding>`：`None` 是**一等状态**（"这条轨不用采样器"，
    /// 见字段自己的文档）⇒ 设成 `None` ⛔ 不是"丢失"，⭐ 而是"解绑"。
    /// 相等是**逐字节 + 同态**的（`None == None`、`Some(a) == Some(b)` 当且仅当
    /// `path` 逐字节相等）⇒ 与 [`Op::SetTrackColor`] 同一"陈旧载荷必被拒"口径。
    ///
    /// ⛔ **本变体不发明路径语义**：不查文件存在性、不查扩展名、不规范化、不解析
    /// （与 [`TrackV3::sampler`] 的字段文档同一口径 —— 模型把路径当留痕字符串）。
    ///
    /// ⚠ 本变体只写这一个字段：⛔ 不碰 `id` / `name` / `kind` / `color` / `devices` /
    /// `macros` / `automation_lanes` / `clips` / `volume_db` / `pan` / `mute` / `solo` /
    /// `solo_safe` / `folder_id`，也⛔ 不碰其它轨。
    /// ⚠ 命名用 `old_*` / `new_*`（**set** 类，照 [`Op::SetTrackColor`]），⛔ **不是** `previous_*`。
    SetTrackSampler {
        /// 目标音轨。
        track_id: EntityId,
        /// 修改前的绑定（`None` = 该轨本来不用采样器）。
        old_sampler: Option<SamplerBinding>,
        /// 修改后的绑定（`None` = 解绑 = 回到"不用采样器"）。
        new_sampler: Option<SamplerBinding>,
    },
    /// 设置音轨**名字**（[`TrackV3::name`]）。
    ///
    /// ⚠ **本变体补上了什么**：⭐ **在它出现之前，`TrackV3::name` 在操作日志层
    /// 不可表达** —— 唯一的入口是 [`Op::AddTrack`]（建轨时把整个 `TrackV3` 交进去），
    /// 而它之后**没有任何 `Op` 能改名** ⇒ 改名只能"删轨 + 重建"，⭐ 那会**换掉
    /// `EntityId`**、并连带丢掉该轨的设备链 / 自动化 / 路由边（⭐ 那些都以 `track_id` 为键）。
    /// 这是 `ARCH-OPS-001` 那一族的又一处（与 `D27` 补 `AddClip`、`D42` 补
    /// `SetAutomationLane`、以及 [`Op::SetAuthor`] / [`Op::SetTrackMute`] 的成因同型）。
    ///
    /// ⚠ `name` 是**纯展示注记**、⛔ 无身份语义（身份是 [`TrackV3::id`]）⇒ 改它
    /// **不影响任何不变式**，所以本变体只写这一个字段：⛔ 不碰 `volume_db` / `pan` /
    /// `mute` / `solo` / `solo_safe` / `folder_id`，也⛔ 不碰其它轨。
    ///
    /// ⚠ 命名用 `old_*` / `new_*`（**set** 类，照 [`Op::SetAuthor`]），⛔ **不是**
    /// `previous_*`（那是 delete/remove 类的惯例）。
    SetTrackName {
        /// 目标音轨。
        track_id: EntityId,
        /// 修改前的名字。
        old_name: String,
        /// 修改后的名字。
        new_name: String,
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

/// 判断两个速度 `f64` 是否**逐位**相同（与 [`same_f32`] 同一条口径）。
///
/// ⚠ 逐位而不是 `==`：`NaN` 的两个实例用 `==` 比较**不相等**、而 `-0.0 == 0.0` 用 `==`
/// 却**相等** ⇒ 逐位让"同一个 NaN 载荷"也被一致地判为不匹配（`NaN` 本来就不在
/// [`MIN_BPM`]..=[`MAX_BPM`] 区间内，故两层防线各自独立）。
#[must_use]
fn same_f64(left: f64, right: f64) -> bool {
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
            Self::SetNoteTiming { .. } => "SetNoteTiming",
            Self::AddClip { .. } => "AddClip",
            Self::RemoveClip { .. } => "RemoveClip",
            Self::SetClipName { .. } => "SetClipName",
            Self::SetClipGain { .. } => "SetClipGain",
            Self::AddRoutingNode { .. } => "AddRoutingNode",
            Self::RemoveRoutingNode { .. } => "RemoveRoutingNode",
            Self::AddClipPlacement { .. } => "AddClipPlacement",
            Self::RemoveClipPlacement { .. } => "RemoveClipPlacement",
            Self::MoveClipPlacement { .. } => "MoveClipPlacement",
            Self::SetClipPlacementMuted { .. } => "SetClipPlacementMuted",
            Self::SetClipPlacementDuration { .. } => "SetClipPlacementDuration",
            Self::AddTrack { .. } => "AddTrack",
            Self::RemoveTrack { .. } => "RemoveTrack",
            Self::ConnectRouting { .. } => "ConnectRouting",
            Self::DisconnectRouting { .. } => "DisconnectRouting",
            Self::SetRoutingGain { .. } => "SetRoutingGain",
            Self::InsertDevice { .. } => "InsertDevice",
            Self::RemoveDevice { .. } => "RemoveDevice",
            Self::SetDeviceName { .. } => "SetDeviceName",
            Self::SetDeviceBypassed { .. } => "SetDeviceBypassed",
            Self::SetParam { .. } => "SetParam",
            Self::SetTrackMute { .. } => "SetTrackMute",
            Self::SetTrackName { .. } => "SetTrackName",
            Self::SetTrackColor { .. } => "SetTrackColor",
            Self::SetTrackSampler { .. } => "SetTrackSampler",
            Self::SetTransportConfig { .. } => "SetTransportConfig",
            Self::SetTempo { .. } => "SetTempo",
            Self::SetAuthor { .. } => "SetAuthor",
            Self::SetTitle { .. } => "SetTitle",
            Self::SetAsset { .. } => "SetAsset",
            Self::RemoveAsset { .. } => "RemoveAsset",
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
            Self::SetNoteTiming {
                track_id,
                clip_id,
                note_id,
                old_start_tick,
                old_duration_ticks,
                new_duration_ticks,
                ..
            } => {
                doc.track(track_id)?;
                let current = doc.note(clip_id, note_id)?;
                // 两个旧值必须**同时**对上：它们是一次时间变换的完整撤销载荷。
                if current.start_tick != *old_start_tick
                    || current.duration_ticks != *old_duration_ticks
                {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                // 与 `MidiNote::validate` 同一条 `ZeroDuration` 口径（⛔ 不自造新错误变体）。
                if *new_duration_ticks == 0 {
                    return Err(ModelError::ZeroDuration);
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
            Self::SetClipPlacementDuration {
                track_id,
                placement_id,
                old_duration_ticks,
                new_duration_ticks,
            } => {
                let track = doc.track(track_id)?;
                let current = track
                    .clips
                    .get(placement_id)
                    .ok_or(ModelError::ClipPlacementNotFound { id: *placement_id })?;
                // ⚠ `u64` 的比较是**值比较** ⇒ 陈旧载荷必被拒。
                if current.duration_ticks != *old_duration_ticks {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                // ⚠ **新值必须 ≥ 1**：⭐ 与 [`ClipPlacement::validate`] 同一条 `ZeroDuration`
                // 口径（⭐ ⛔ 不自造新错误变体）。
                if *new_duration_ticks == 0 {
                    return Err(ModelError::ZeroDuration);
                }
                Ok(())
            }
            Self::SetClipPlacementMuted {
                track_id,
                placement_id,
                old_muted,
                ..
            } => {
                let track = doc.track(track_id)?;
                // ⚠ 摆放**按身份**取（`clips` 是 `BTreeMap`，键 = 摆放身份）⇒ 有**专用**
                // 错误变体可复用（⛔ 不必像 `devices`（`Vec`）那样借用下标越界错误）。
                let current = track
                    .clips
                    .get(placement_id)
                    .ok_or(ModelError::ClipPlacementNotFound { id: *placement_id })?;
                // ⚠ `bool` 的比较是**值比较**（与 [`Op::SetTrackMute`] 同口径）⇒ 陈旧载荷必被拒。
                if current.muted != *old_muted {
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
            Self::SetClipName {
                clip_id, old_name, ..
            } => {
                let clip = doc
                    .clip_pool
                    .get(clip_id)
                    .ok_or(ModelError::ClipNotFound { id: *clip_id })?;
                // ⚠ `String` 的相等是**逐字节**的（与 [`Op::SetTrackName`] / [`Op::SetAuthor`]
                // 同一口径）⇒ 陈旧载荷必被拒。
                if clip.name != *old_name {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetClipGain {
                clip_id,
                old_gain_db,
                new_gain_db,
            } => {
                let clip = doc
                    .clip_pool
                    .get(clip_id)
                    .ok_or(ModelError::ClipNotFound { id: *clip_id })?;
                // ⚠ 内容类型门必须**早于**陈旧载荷比较：MIDI 片段没有 `gain_db` 可读
                // ⇒ 报 `ClipContentKindMismatch`（⛔ 不 panic、⛔ 不静默当作 0 dB）。
                let ClipContent::Audio { gain_db, .. } = &clip.content else {
                    return Err(ModelError::ClipContentKindMismatch { id: *clip_id });
                };
                // `f32` 逐位比较（`-0.0` 与 `0.0` 位不同）⇒ 陈旧载荷必被拒。
                if !same_f32(*gain_db, *old_gain_db) {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                // ⚠ 值域**照抄现状**：只查有限性，**没有** dB 上下限（见枚举文档）。
                if !new_gain_db.is_finite() {
                    return Err(ModelError::NonFiniteValue {
                        field: "clip.audio.gain_db",
                        value: f64::from(*new_gain_db),
                    });
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
            Self::SetDeviceBypassed {
                track_id,
                device_id,
                old_bypassed,
                ..
            } => {
                let track = doc.track(track_id)?;
                // ⚠ **按身份找、不按下标**（`devices` 是 `Vec`，增删会移位下标）。
                let index = track
                    .devices
                    .iter()
                    .position(|existing| existing.id == *device_id)
                    .ok_or(ModelError::DeviceSlotOutOfRange {
                        index: track.devices.len(),
                        len: track.devices.len(),
                    })?;
                // ⚠ `bool` 的比较是**值比较**（与 [`Op::SetTrackMute`] 同口径）⇒ 陈旧载荷必被拒。
                if track.devices[index].bypassed != *old_bypassed {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetDeviceName {
                track_id,
                device_id,
                old_name,
                ..
            } => {
                let track = doc.track(track_id)?;
                // ⚠ **按身份找、不按下标**（`devices` 是 `Vec`，增删会移位下标）。
                // ⚠ 缺失时复用既有 `DeviceSlotOutOfRange`：`index` 传链长 = "请求落在
                // 链尾之后"，是"这个身份不在链上"的**准确**读数（本仓没有"按 id 报设备
                // 不存在"的模型错误变体，⛔ 不为此新增变体）。
                let index = track
                    .devices
                    .iter()
                    .position(|existing| existing.id == *device_id)
                    .ok_or(ModelError::DeviceSlotOutOfRange {
                        index: track.devices.len(),
                        len: track.devices.len(),
                    })?;
                // ⚠ `String` 的相等是**逐字节**的（与 [`Op::SetTrackName`] /
                // [`Op::SetClipName`] 同一口径）⇒ 陈旧载荷必被拒。
                if track.devices[index].name != *old_name {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
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
            Self::SetTrackColor {
                track_id,
                old_color,
                ..
            } => {
                // ⚠ `Option<String>` 的相等是**逐字节 + 同态**的（`None == None`、`Some(a) == Some(b)`
                // 当且仅当 `a`/`b` 逐字节相等）⇒ 与 [`Op::SetTrackName`] 同一"陈旧载荷必被拒"口径。
                if doc.track(track_id)?.color != *old_color {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            // 与 `SetTrackColor` **逐字同型**（同样是 `Option<T>` 的"逐字节 + 同态"比较）：
            // `None` 是一等状态（"这条轨不用采样器"）⇒ 陈旧载荷必被拒。
            Self::SetTrackSampler {
                track_id,
                old_sampler,
                ..
            } => {
                if doc.track(track_id)?.sampler != *old_sampler {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetTrackName {
                track_id, old_name, ..
            } => {
                // ⚠ `String` 的相等是**逐字节**的（与 [`Op::SetAuthor`] 同一口径）
                // ⇒ 陈旧载荷必被拒。
                if doc.track(track_id)?.name != *old_name {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
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
            Self::SetTransportConfig { old_config, .. } => {
                if doc.transport != *old_config {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetTempo { old_bpm, new_bpm } => {
                // ① 载荷自身必须是**有限且在区间内**的值（⛔ 不靠 apply 之后的
                //    `YebanProjectV1::validate` 兜底）。
                if !new_bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(new_bpm) {
                    return Err(ModelError::BpmOutOfRange { value: *new_bpm });
                }
                // ② 文档现值必须逐位等于 `old_bpm`（⛔ 不信调用方声明的旧状态）。
                if !same_f64(doc.bpm, *old_bpm) {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::RemoveAsset {
                asset_hash,
                previous_asset,
            } => {
                // ⚠ `AssetHash` **不是** `Copy` ⇒ 用 `get` 的 `Option` 判定，⛔ 不移动它。
                // ⚠ 引用缺失**复用** `OpStateMismatch`（与 `SetScene` 的"键 == 载荷身份"
                // 失败同一口径）：新加一个 `ModelError` 变体会牵动 `yeban-mcp` 的
                // `code_for_model` 穷举 match 与契约码表 —— 那是**另一条线**的范围。
                let Some(current) = doc.assets.get(asset_hash) else {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                };
                if current != previous_asset {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetAsset {
                asset_hash,
                old_asset,
                new_asset,
            } => {
                // 身份一致性：键（`assets` 的键）必须等于载荷里的身份，与 `SetScene` 同型。
                if new_asset.hash != *asset_hash {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                // 陈旧载荷必被拒：文档现值必须**逐字段**等于声明的 `old_asset`
                // （`None` = 本工程尚未声明过它）。
                if doc.assets.get(asset_hash) != old_asset.as_ref() {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetAuthor {
                old_author,
                new_author: _,
            } => {
                // ⚠ `String` 的相等是**逐字节**的（`str` 的 `PartialEq` 就是字节序比较）
                // ⇒ 与数值字段的逐位比较同一条口径：陈旧载荷必被拒。
                if doc.author != *old_author {
                    return Err(ModelError::OpStateMismatch { op: self.name() });
                }
                Ok(())
            }
            Self::SetTitle {
                old_title,
                new_title: _,
            } => {
                // 与 `SetAuthor` 同一条口径：`String` 逐字节比较，陈旧标题必被拒。
                // ⚠ `title` 在契约与 `validate` 里都**没有**长度/非空约束 ⇒ 这里也⛔ 不发明。
                if doc.title != *old_title {
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
            Self::SetNoteTiming {
                track_id,
                clip_id,
                note_id,
                old_start_tick,
                old_duration_ticks,
                new_start_tick,
                new_duration_ticks,
            } => Self::SetNoteTiming {
                track_id: *track_id,
                clip_id: *clip_id,
                note_id: *note_id,
                old_start_tick: *new_start_tick,
                old_duration_ticks: *new_duration_ticks,
                new_start_tick: *old_start_tick,
                new_duration_ticks: *old_duration_ticks,
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
            Self::SetClipPlacementDuration {
                track_id,
                placement_id,
                old_duration_ticks,
                new_duration_ticks,
            } => Self::SetClipPlacementDuration {
                track_id: *track_id,
                placement_id: *placement_id,
                old_duration_ticks: *new_duration_ticks,
                new_duration_ticks: *old_duration_ticks,
            },
            Self::SetClipPlacementMuted {
                track_id,
                placement_id,
                old_muted,
                new_muted,
            } => Self::SetClipPlacementMuted {
                track_id: *track_id,
                placement_id: *placement_id,
                old_muted: *new_muted,
                new_muted: *old_muted,
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
            Self::SetClipName {
                clip_id,
                old_name,
                new_name,
            } => Self::SetClipName {
                clip_id: *clip_id,
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            },
            // 纯结构化取反：`f32` 是 `Copy`，只交换"前 / 后"两个增益
            // （与 [`Op::SetRoutingGain`] 同型；⛔ 不伪造旧值）。
            Self::SetClipGain {
                clip_id,
                old_gain_db,
                new_gain_db,
            } => Self::SetClipGain {
                clip_id: *clip_id,
                old_gain_db: *new_gain_db,
                new_gain_db: *old_gain_db,
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
            Self::SetDeviceBypassed {
                track_id,
                device_id,
                old_bypassed,
                new_bypassed,
            } => Self::SetDeviceBypassed {
                track_id: *track_id,
                device_id: *device_id,
                old_bypassed: *new_bypassed,
                new_bypassed: *old_bypassed,
            },
            Self::SetDeviceName {
                track_id,
                device_id,
                old_name,
                new_name,
            } => Self::SetDeviceName {
                track_id: *track_id,
                device_id: *device_id,
                old_name: new_name.clone(),
                new_name: old_name.clone(),
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
            // 纯结构化取反：只交换"前 / 后"两份配置（与 `SetTrackMute` 同型）。
            Self::SetTransportConfig {
                old_config,
                new_config,
            } => Self::SetTransportConfig {
                old_config: *new_config,
                new_config: *old_config,
            },
            // 纯结构化取反：只交换"前 / 后"两个速度（与 `SetTransportConfig` 同型）。
            Self::SetTempo { old_bpm, new_bpm } => Self::SetTempo {
                old_bpm: *new_bpm,
                new_bpm: *old_bpm,
            },
            // `String` 不是 `Copy` ⇒ 必须克隆（与 `RemoveClip` 的 `previous_clip.clone()` 同理）。
            Self::SetAuthor {
                old_author,
                new_author,
            } => Self::SetAuthor {
                old_author: new_author.clone(),
                new_author: old_author.clone(),
            },
            // 同 `SetAuthor`：`String` 不是 `Copy` ⇒ 克隆交换（⛔ 不伪造旧值）。
            Self::SetTitle {
                old_title,
                new_title,
            } => Self::SetTitle {
                old_title: new_title.clone(),
                new_title: old_title.clone(),
            },
            // ⭐ **照 [`Op::SetSection`] 的先例**（⛔ 不是纯交换）：`old_asset` 为 `None`
            // 表示"创建"，而**创建态没有可交换的旧值** ⇒ 逆操作**必须是删除变体**。
            // `SetScene`/`RemoveScene` 与 `SetSection`/`RemoveSection` 成对存在，正因为
            // 同一条道理。⚠ 若在这里用"两个 `Some` 互绕"来凑一个纯交换，就会**伪造一份
            // 载荷**（把"没有旧值"偷换成"旧值等于新值"）⇒ 撤销会把资产留在文档里。
            Self::SetAsset {
                asset_hash,
                old_asset,
                new_asset,
            } => match old_asset {
                Some(previous) => Self::SetAsset {
                    asset_hash: asset_hash.clone(),
                    old_asset: Some(new_asset.clone()),
                    new_asset: previous.clone(),
                },
                None => Self::RemoveAsset {
                    asset_hash: asset_hash.clone(),
                    previous_asset: new_asset.clone(),
                },
            },
            Self::RemoveAsset {
                asset_hash,
                previous_asset,
            } => Self::SetAsset {
                asset_hash: asset_hash.clone(),
                old_asset: None,
                new_asset: previous_asset.clone(),
            },
            // 纯结构化取反：只交换"前 / 后"两个布尔。
            Self::SetTrackColor {
                track_id,
                old_color,
                new_color,
            } => Self::SetTrackColor {
                track_id: *track_id,
                old_color: new_color.clone(),
                new_color: old_color.clone(),
            },
            // 纯结构化取反：只交换"前 / 后"两个 `Option<SamplerBinding>`（同 `SetTrackColor`）。
            Self::SetTrackSampler {
                track_id,
                old_sampler,
                new_sampler,
            } => Self::SetTrackSampler {
                track_id: *track_id,
                old_sampler: new_sampler.clone(),
                new_sampler: old_sampler.clone(),
            },
            Self::SetTrackName {
                track_id,
                old_name,
                new_name,
            } => Self::SetTrackName {
                track_id: *track_id,
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            },
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
            // 只写那**一个**显示名字段：⛔ 不碰 `id` / `content`、⛔ 不动 `clip_pool`
            // 的键、⛔ 不碰任何摆放。
            Self::SetClipName {
                clip_id, new_name, ..
            } => {
                let clip = doc
                    .clip_pool
                    .get_mut(clip_id)
                    .ok_or(ModelError::ClipNotFound { id: *clip_id })?;
                clip.name = new_name.clone();
                Ok(())
            }
            // 只写那**一个** `gain_db` 字段：⛔ 不碰 `id` / `name` / `asset`、
            // ⛔ 不动 `clip_pool` 的键、⛔ 不碰任何摆放。
            Self::SetClipGain {
                clip_id,
                new_gain_db,
                ..
            } => {
                let clip = doc
                    .clip_pool
                    .get_mut(clip_id)
                    .ok_or(ModelError::ClipNotFound { id: *clip_id })?;
                // 前置条件已经做过同一道内容类型门；apply 也**必*要**自己做一遍
                // （⛔ 不 assume 调用方一定先跑 precondition —— `apply` 是公开面）。
                let ClipContent::Audio { gain_db, .. } = &mut clip.content else {
                    return Err(ModelError::ClipContentKindMismatch { id: *clip_id });
                };
                *gain_db = *new_gain_db;
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
            // 只写那**两个**时间字段：⛔ 不碰 `id` / `pitch` / 力度 / 任何表现力字段。
            Self::SetNoteTiming {
                clip_id,
                note_id,
                new_start_tick,
                new_duration_ticks,
                ..
            } => {
                let note = doc.note_mut(clip_id, note_id)?;
                note.start_tick = *new_start_tick;
                note.duration_ticks = *new_duration_ticks;
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
            // 只写那**一个**长度字段：⛔ 不碰 `id` / `clip_id` / `start_tick` / `muted` /
            // `loop_config`（⭐ 与 `loop_config` 的判定关联只**登记**、⛔ 不在此联动）。
            Self::SetClipPlacementDuration {
                track_id,
                placement_id,
                new_duration_ticks,
                ..
            } => {
                if let Some(placement) = doc.track_mut(track_id)?.clips.get_mut(placement_id) {
                    placement.duration_ticks = *new_duration_ticks;
                }
                Ok(())
            }
            // 只写那**一个**静音开关：⛔ 不碰 `id` / `clip_id` / `start_tick` /
            // `duration_ticks` / `loop_config`。
            Self::SetClipPlacementMuted {
                track_id,
                placement_id,
                new_muted,
                ..
            } => {
                if let Some(placement) = doc.track_mut(track_id)?.clips.get_mut(placement_id) {
                    placement.muted = *new_muted;
                }
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
            // 只写那**一个**旁通开关：⛔ 不碰 `id` / `name` / `kind` / `params` /
            // `latency_samples`（延迟值照旧保留，只是旁通期间不计入相位）。
            Self::SetDeviceBypassed {
                track_id,
                device_id,
                new_bypassed,
                ..
            } => {
                let track = doc.track_mut(track_id)?;
                let index = track
                    .devices
                    .iter()
                    .position(|existing| existing.id == *device_id)
                    .ok_or(ModelError::DeviceSlotOutOfRange {
                        index: track.devices.len(),
                        len: track.devices.len(),
                    })?;
                track.devices[index].bypassed = *new_bypassed;
                Ok(())
            }
            // 只写那**一个**显示名字段：⛔ 不碰 `id` / `kind` / `params` /
            // `latency_samples`，⭐ 也⛔ 不碰 `bypassed`（行为开关，本票有意不做）。
            Self::SetDeviceName {
                track_id,
                device_id,
                new_name,
                ..
            } => {
                let track = doc.track_mut(track_id)?;
                let index = track
                    .devices
                    .iter()
                    .position(|existing| existing.id == *device_id)
                    .ok_or(ModelError::DeviceSlotOutOfRange {
                        index: track.devices.len(),
                        len: track.devices.len(),
                    })?;
                track.devices[index].name = new_name.clone();
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
            // 只写那**一个**色标字段：⛔ 不碰 `id` / `name` / `kind` / 集合 / 混音标量。
            // ⚠ `new_color: None` = **清除**（回到主题默认色），⛔ 不是"丢失"。
            Self::SetTrackColor {
                track_id,
                new_color,
                ..
            } => {
                doc.track_mut(track_id)?.color = new_color.clone();
                Ok(())
            }
            // 只写那**一个**绑定字段：⛔ 不碰设备链、⛔ 不碰 `color`/`name`/集合/混音标量。
            // ⚠ `new_sampler: None` = **解绑**（这条轨不再用采样器），⛔ 不是"丢失"。
            // ⚠ 载荷是路径字符串，⛔ 模型**不解析**它（那是控制侧的事）。
            Self::SetTrackSampler {
                track_id,
                new_sampler,
                ..
            } => {
                doc.track_mut(track_id)?.sampler = new_sampler.clone();
                Ok(())
            }
            // 只写那**一个**名字字段：不碰 `volume_db` / `pan` / `mute` / `solo` /
            // `solo_safe` / `folder_id`，也⛔ 不碰其它轨、⛔ 不碰 `id`。
            Self::SetTrackName {
                track_id, new_name, ..
            } => {
                doc.track_mut(track_id)?.name = new_name.clone();
                Ok(())
            }
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
            // 整体替换三个字段：⛔ 不逐字段写（见枚举文档：三个字段是一个作者选择）。
            Self::SetTransportConfig { new_config, .. } => {
                doc.transport = *new_config;
                Ok(())
            }
            // 只写那**一个**顶层字段：⛔ 不碰 `time_signature`、⛔ 不碰 `TransportConfig`。
            Self::SetTempo { new_bpm, .. } => {
                doc.bpm = *new_bpm;
                Ok(())
            }
            // 只写那**一个**顶层字段：⛔ 不碰 `title`、⛔ 不碰 `bpm`。
            Self::SetAuthor { new_author, .. } => {
                doc.author = new_author.clone();
                Ok(())
            }
            // 只写那**一个**顶层字段：⛔ 不碰 `author`、⛔ 不碰 `bpm`、⛔ 不碰 `id`。
            Self::SetTitle { new_title, .. } => {
                doc.title = new_title.clone();
                Ok(())
            }
            // 只取走那**一个**索引项：⛔ 不碰 `clip_pool`、⛔ 不碰归档里的字节。
            Self::RemoveAsset { asset_hash, .. } => {
                doc.assets.remove(asset_hash);
                Ok(())
            }
            // 只写那**一个**索引项：⛔ 不碰 `clip_pool`、⛔ 不碰资产字节（字节在归档里）。
            Self::SetAsset {
                asset_hash,
                new_asset,
                ..
            } => {
                doc.assets.insert(asset_hash.clone(), new_asset.clone());
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
        ClipContent, ClipPoolEntry, LaunchQuantization, LoopConfig, MediaKind, ProjectAudioConfig,
        RoutingKind, TrackKind,
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
    /// 夹具音频片段引用的资产身份（内容寻址键）。
    fn kick_asset() -> crate::ids::AssetHash {
        crate::ids::AssetHash::of_bytes(b"kick")
    }

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
            // ⚠ 夹具的音频片段引用了 `b"kick"` 这份资产 ⇒ 它的**索引项**必须存在，
            // 否则 `RemoveAsset` 的代表实例没有可删的对象（也让夹具更接近真实文档）。
            assets: BTreeMap::from([(
                kick_asset(),
                AssetMetadata {
                    hash: kick_asset(),
                    original_path: "samples/kick.wav".to_owned(),
                    byte_len: 1_024,
                    media_kind: MediaKind::Audio,
                    license: "CC0-1.0".to_owned(),
                },
            )]),
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
            // 音符时间位置：**此前只能"删 + 加"** —— `MoveNote` 只平移起点、
            // 时值没有任何字段级通路。0/480 ⇒ 240/960（后移 240 tick、拉长到两倍）。
            Op::SetNoteTiming {
                track_id: f.lead,
                clip_id: f.clip,
                note_id: f.note,
                old_start_tick: 0,
                old_duration_ticks: 480,
                new_start_tick: 240,
                new_duration_ticks: 960,
            },
            Op::AddClipPlacement {
                track_id: f.bass,
                placement: new_placement,
            },
            // 摆放静音切换：**此前创建之后改不了**（`MoveClipPlacement` 只写 `start_tick`）。
            // ⚠ 必须放在下面那条 `RemoveClipPlacement` **之前** —— 那条会把 `f.placement`
            // 从 lead 轨取走，而本变体的前置条件要求摆放**还在**。
            Op::SetClipPlacementMuted {
                track_id: f.lead,
                placement_id: f.placement,
                old_muted: false,
                new_muted: true,
            },
            // 摆放长度：**此前创建之后改不了**（`MoveClipPlacement` 只写 `start_tick`）。
            // ⚠ 同样必须放在下面那条 `RemoveClipPlacement` **之前**（⭐ 它会把 `f.placement`
            // 取走、⭐ 而本变体的前置条件要求摆放还在）。⭐ 3840 ⇒ 1920 = 修剪一半。
            Op::SetClipPlacementDuration {
                track_id: f.lead,
                placement_id: f.placement,
                old_duration_ticks: 3840,
                new_duration_ticks: 1920,
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
            // 走带配置：夹具的 `transport` 是 `TransportConfig::default()`
            // （`metronome_enabled=false` / `count_in_bars=0` / `Bar`）⇒ `old_config`
            // 取默认值，前置条件在 `fixture_document()` 上成立。
            Op::SetTransportConfig {
                old_config: TransportConfig::default(),
                new_config: TransportConfig {
                    metronome_enabled: true,
                    count_in_bars: 2,
                    launch_quantization: LaunchQuantization::TwoBars,
                },
            },
            // 速度：夹具的 `bpm` 是 `DEFAULT_BPM`（`YebanProjectV1::default()`）⇒
            // `old_bpm` 取它，前置条件在 `fixture_document()` 上成立。
            Op::SetTempo {
                old_bpm: crate::project::DEFAULT_BPM,
                new_bpm: 140.0,
            },
            // 署名：夹具的 `author` 是空串（`YebanProjectV1::default()`）⇒ 前置条件成立。
            Op::SetAuthor {
                old_author: String::new(),
                new_author: "Yeban Author".to_owned(),
            },
            // 标题：夹具的 `title` 是 `"Ops Fixture"`（见 `fixture_document`）⇒ 前置条件成立。
            // ⭐ 本变体补的是「建得出、改不了」的那**最后一个**顶层文档字段（`title`）。
            Op::SetTitle {
                old_title: "Ops Fixture".to_owned(),
                new_title: "夜半 · Yeban".to_owned(),
            },
            // 改名：**唯一的名字通路**（此前只能删轨重建 ⇒ 换 `EntityId`）。
            // 音轨色标：**此前只能在建轨时给**（`AddTrack` 的载荷）⇒ 改色只能删轨重建。
            // ⚠ `old_color: None` 对应夹具的 `TrackV3::default()`（⭐ `color` 默认 `None`
            // ＝"用主题默认色"，⭐ 是一等状态）；⭐ `new_color: Some(…)` 用#RRGGBB 形态。
            Op::SetTrackColor {
                track_id: f.lead,
                old_color: None,
                new_color: Some("#22AA88".to_owned()),
            },
            // 采样器绑定：**此前只能在建轨时给**（`AddTrack` 的载荷）⇒ 换 SFZ 只能删轨重建。
            // ⚠ `old_sampler: None` 对应夹具的 `TrackV3::default()`（⭐ `sampler` 默认 `None`
            // ＝"这条轨不用采样器"，⭐ 是一等状态）；⭐ 新值只是一个**路径字符串**，
            // ⛔ 不带任何已解析乐器（模型的持久化 AST 里不放运行时对象）。
            Op::SetTrackSampler {
                track_id: f.lead,
                old_sampler: None,
                new_sampler: Some(SamplerBinding {
                    path: "instruments/lead.sfz".to_owned(),
                }),
            },
            Op::SetTrackName {
                track_id: f.lead,
                old_name: "Lead".to_owned(),
                new_name: "Lead (renamed)".to_owned(),
            },
            // 改片段显示名：**唯一的改名通路**（此前只能删 + 加，而 `RemoveClip`
            // 还要求先把该片段的每一处摆放拆掉）。
            // ⚠ 用 `f.clip`（而不是 `f.audio_clip`）—— 后者在同一条 showcase 里
            // 已被 `RemoveClip` 取走，本变体的前置条件要求条目**在**池子里。
            Op::SetClipName {
                clip_id: f.clip,
                old_name: "Clip".to_owned(),
                new_name: "Clip (renamed)".to_owned(),
            },
            // 改**音频片段**增益：**建得出、改不了**的那一半（`import_audio` 只在创建时写
            // `gainDb`）。⚠ 目标必须是**音频**片段（`f.audio_clip`，夹具里 `gain_db = 0.0`
            // 且**没有任何摆放**引用它）—— 内容类型门由本变体自己判；MIDI 片段会红
            // `ClipContentKindMismatch`（那条由专项判据覆盖，⛔ 不放进 showcase）。
            Op::SetClipGain {
                clip_id: f.audio_clip,
                old_gain_db: 0.0,
                new_gain_db: -6.0,
            },
            // 改设备显示名：**唯一的改名通路**（此前只能"删 + 加"，而 `devices` 是
            // `Vec` ⇒ 增删还会移位下标）。
            // ⚠ 目标用 `fixture_document()` 里 lead 轨上那个设备（`f.device`，名
            // "PolySynth"）；showcase 自己的 `InsertDevice` 加的是 `f.bass` 上的另一台。
            Op::SetDeviceName {
                track_id: f.lead,
                device_id: f.device,
                old_name: "PolySynth".to_owned(),
                new_name: "PolySynth (renamed)".to_owned(),
            },
            // 旁通切换：**此前连一次"旁通"都做不出来**（`Default` 与所有夹具写死 `false`）。
            // ⚠ 目标仍是 lead 轨那台设备（`f.device`）；旁通是布尔开关 ⇒ 切回去即复原。
            Op::SetDeviceBypassed {
                track_id: f.lead,
                device_id: f.device,
                old_bypassed: false,
                new_bypassed: true,
            },
            // 资产声明（**创建**：夹具里尚未声明过这份新哈希 ⇒ `old_asset: None`）。
            Op::SetAsset {
                asset_hash: crate::ids::AssetHash::of_bytes(b"snare"),
                old_asset: None,
                new_asset: AssetMetadata {
                    hash: crate::ids::AssetHash::of_bytes(b"snare"),
                    original_path: "samples/snare.wav".to_owned(),
                    byte_len: 2_048,
                    media_kind: MediaKind::Audio,
                    license: "CC-BY-4.0".to_owned(),
                },
            },
            // 资产删除（夹具里已登记的 `b"kick"`）。
            Op::RemoveAsset {
                asset_hash: kick_asset(),
                previous_asset: AssetMetadata {
                    hash: kick_asset(),
                    original_path: "samples/kick.wav".to_owned(),
                    byte_len: 1_024,
                    media_kind: MediaKind::Audio,
                    license: "CC0-1.0".to_owned(),
                },
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

    /// `SetTrackSampler` 只写**那一个**字段，且"陈旧载荷必被拒"（任务单 `#76`）。
    ///
    /// 判据的设计意图：`sampler` 是**路径引用**（⛔ 不是运行时对象），所以这一条同时钉三件事：
    ///
    /// 1. **只有 `sampler` 变** —— ⛔ 不动设备链 / 名字 / 色标 / 集合 / 混音标量；
    /// 2. **陈旧载荷必被拒** —— 声明"旧值"与文档现值不符时 `OpStateMismatch`，且文档一位不动；
    /// 3. **`None` 是一等状态** —— 逆操作把 `Some(path)` 还原成 `None`（解绑），
    ///    ⛔ 不是"丢失"（`None` 在字段自己的文档里就写着"这条轨不用采样器"）。
    ///
    /// ⚠ 路径**原样**写进文档：⛔ 模型不解析、不查存在性（那是 `yeban-engine` / `yeban-sfz`
    /// 的后续单要做的事；本变体只管"这一个字段能表达、能序列化、能撤销"）。
    #[test]
    fn set_track_sampler_writes_exactly_one_field_and_rejects_a_stale_payload() {
        let f = fixture();
        let mut doc = fixture_document();
        let binding = SamplerBinding {
            path: "instruments/lead.sfz".to_owned(),
        };
        let before = doc.track(&f.lead).expect("lead 轨在夹具里").clone();

        // ① 陈旧载荷：文档现值是 `None`，声明 `Some(…)` 必须被拒，且文档一位不动。
        let stale = Op::SetTrackSampler {
            track_id: f.lead,
            old_sampler: Some(binding.clone()),
            new_sampler: None,
        };
        assert_eq!(
            stale.apply(&mut doc),
            Err(ModelError::OpStateMismatch {
                op: "SetTrackSampler"
            }),
            "陈旧载荷必须被拒"
        );
        assert_eq!(doc, fixture_document(), "被拒的操作不得改动文档");

        // ② 正路：`None` ⇒ `Some(path)`，且**只有** `sampler` 变。
        let op = Op::SetTrackSampler {
            track_id: f.lead,
            old_sampler: None,
            new_sampler: Some(binding.clone()),
        };
        assert_eq!(op.precondition(&doc), Ok(()));
        op.apply(&mut doc).expect("施加");
        let after = doc.track(&f.lead).expect("lead 轨在夹具里");
        assert_eq!(after.sampler, Some(binding.clone()));
        assert_eq!(
            TrackV3 {
                sampler: before.sampler.clone(),
                ..after.clone()
            },
            before,
            "`SetTrackSampler` 只许改 `sampler`：其余字段必须逐字段不变"
        );

        // ③ 逆操作：`Some(path)` ⇒ `None`（一等状态 = 解绑），逐字节精确还原。
        let inverse = op.invert(&doc).expect("求逆");
        assert_eq!(
            inverse.name(),
            "SetTrackSampler",
            "同族 `Option<T>` setter 的逆仍是它自己（照 `SetTrackColor`）"
        );
        inverse.apply(&mut doc).expect("施加逆操作");
        assert_eq!(doc, fixture_document(), "逆操作必须精确还原");
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

    /// 摆放的 `clip_id` 必须在片段池里：Op 入口自己就要拦住悬空引用。
    ///
    /// 实测：把 `AddClipPlacement` 的 `doc.clip_pool.contains_key(...)` 检查关掉时，
    /// `apply` 返回 `Ok(())` 并把悬空摆放落进文档 —— 既有判据只从
    /// `YebanProjectV1::validate`（另一层）看过悬空引用，于是"先污染、后报错"的那条
    /// 入口没有任何判据看得见。
    #[test]
    fn add_clip_placement_rejects_a_clip_that_is_not_in_the_pool() {
        let f = fixture();
        let mut doc = fixture_document();
        let snapshot = doc.clone();
        let ghost_clip = fixture_id(700);
        let op = Op::AddClipPlacement {
            track_id: f.bass,
            placement: ClipPlacement {
                id: fixture_id(701),
                clip_id: ghost_clip,
                start_tick: 0,
                duration_ticks: 960,
                ..ClipPlacement::default()
            },
        };
        assert_eq!(
            op.apply(&mut doc),
            Err(ModelError::ClipNotFound { id: ghost_clip })
        );
        assert_eq!(doc, snapshot, "被拒之后文档必须逐字节不动");
    }

    /// `AddRoutingNode` 的文档承诺 `nodes` 恒按字典序 ⇒ 两条**独立**的加法可交换，
    /// 且结果与插入顺序无关（顺序独立性 / `ARCH-DET-001` 的写入确定性）。
    ///
    /// 实测：把 `partition_point` + 有序 `insert` 换成 `push` 时，全仓判据保持全绿
    /// —— 于是 `nodes` 的字节形态取决于操作顺序，同一份逻辑内容会写出两种工程字节。
    #[test]
    fn routing_node_insertion_is_order_independent_and_keeps_the_list_sorted() {
        let first = fixture_id(800);
        let second = fixture_id(801);

        let mut forward = fixture_document();
        Op::AddRoutingNode { node: second }
            .apply(&mut forward)
            .expect("加第二个节点");
        Op::AddRoutingNode { node: first }
            .apply(&mut forward)
            .expect("加第一个节点");

        let mut backward = fixture_document();
        Op::AddRoutingNode { node: first }
            .apply(&mut backward)
            .expect("加第一个节点");
        Op::AddRoutingNode { node: second }
            .apply(&mut backward)
            .expect("加第二个节点");

        assert_eq!(
            forward, backward,
            "两条独立的 AddRoutingNode 必须可交换（结果与顺序无关）"
        );
        let mut expected = forward.routing_graph.nodes.clone();
        expected.sort();
        assert_eq!(
            forward.routing_graph.nodes, expected,
            "routing_graph.nodes 必须恒按字典序"
        );
        forward.validate().expect("加完节点之后文档仍然合法");

        // 成对撤销必须逐位回到原状。
        let snapshot = fixture_document();
        Op::AddRoutingNode { node: second }
            .apply_inverse(&mut backward)
            .expect("撤销第二个节点");
        Op::AddRoutingNode { node: first }
            .apply_inverse(&mut backward)
            .expect("撤销第一个节点");
        assert_eq!(backward, snapshot, "成对撤销必须逐位回到原状");
    }

    /// 三个可寻址向量轴（设备槽 / 设备参数 / 宏）的**下标 1** 必须被真正寻址。
    ///
    /// 实测：夹具里只有 1 个设备、1 个宏，而既有判据从不用 `param_index = 1` ⇒ 把
    /// `write_param` 的 `get_mut(slot_index)` / `get_mut(param_index)` /
    /// `get_mut(macro_index)`、或 `read_param` 的 `get(param_index)`、或 `read_macro`
    /// 的 `get(macro_index)` 换成 `get(0)` 时，全仓判据保持全绿 —— "写到了 0 号而不是
    /// 被寻址的那一个"没有任何判据看得见。
    #[test]
    fn indexed_targets_address_the_exact_device_param_and_macro() {
        let f = fixture();
        let mut doc = fixture_document();
        // 造出"三个轴上都有下标 1"的文档。
        doc.track_mut(&f.lead)
            .expect("lead")
            .devices
            .push(DeviceDefinition {
                id: fixture_id(41),
                name: "Second".to_owned(),
                params: vec![
                    crate::project::ParameterValue {
                        name: "a".to_owned(),
                        value: 1.0,
                        unit: None,
                    },
                    crate::project::ParameterValue {
                        name: "b".to_owned(),
                        value: 2.0,
                        unit: None,
                    },
                ],
                ..DeviceDefinition::default()
            });
        doc.track_mut(&f.lead)
            .expect("lead")
            .macros
            .push(crate::project::MacroParameter {
                name: "Second".to_owned(),
                value: 0.25,
                mappings: Vec::new(),
            });
        doc.validate().expect("探针文档必须合法");

        let param = |doc: &YebanProjectV1, slot: usize, index: usize| -> f32 {
            doc.track(&f.lead).expect("lead").devices[slot].params[index].value
        };
        let macro_value = |doc: &YebanProjectV1, index: usize| -> f32 {
            doc.track(&f.lead).expect("lead").macros[index].value
        };

        // ---- ① 设备参数：寻址 (slot 1, param 1) ----
        let before = doc.clone();
        assert_eq!(param(&doc, 1, 1), 2.0, "夹具的下标 (1,1) 值变了");
        let op = Op::SetParam {
            target: AutomationTarget::DeviceParam {
                track_id: f.lead,
                slot_index: 1,
                param_index: 1,
            },
            old_val: 2.0,
            new_val: 7.5,
        };
        op.apply(&mut doc).expect("SetParam 必须可应用");
        assert_eq!(param(&doc, 1, 1), 7.5, "必须写到被寻址的那一格");
        assert_eq!(param(&doc, 0, 0), 1200.0, "0 号设备 0 号参数不许被动");
        assert_eq!(param(&doc, 0, 1), 0.3, "0 号设备 1 号参数不许被动");
        assert_eq!(param(&doc, 1, 0), 1.0, "1 号设备 0 号参数不许被动");
        op.invert(&doc)
            .expect("求逆")
            .apply(&mut doc)
            .expect("撤销");
        assert_eq!(doc, before, "下标 1 的设备参数写入必须逐位可逆");

        // ---- ② 宏：寻址 macro_index 1 ----
        let before = doc.clone();
        assert_eq!(macro_value(&doc, 1), 0.25, "夹具的 1 号宏值变了");
        let op = Op::SetMacro {
            track_id: f.lead,
            macro_index: 1,
            old_val: 0.25,
            new_val: 0.75,
        };
        op.apply(&mut doc).expect("SetMacro 必须可应用");
        assert_eq!(macro_value(&doc, 1), 0.75, "必须写到被寻址的宏");
        assert_eq!(macro_value(&doc, 0), 0.5, "0 号宏不许被动");
        op.invert(&doc)
            .expect("求逆")
            .apply(&mut doc)
            .expect("撤销");
        assert_eq!(doc, before, "下标 1 的宏写入必须逐位可逆");

        // ---- ②′ 同一个宏轴也可以经 `SetParam`（`write_param` 的宏分支）寻址 ----
        // `Op::SetMacro` 与 `Op::SetParam { target: Macro }` 走的是**两条**不同的写路径，
        // 因此这一格必须单独钉住：把 `write_param` 的 `get_mut(macro_index)` 换成
        // `get_mut(0)` 时，只有这一格会变红（实测：只用 `Op::SetMacro` 时它保持全绿）。
        let before = doc.clone();
        let op = Op::SetParam {
            target: AutomationTarget::Macro {
                track_id: f.lead,
                macro_index: 1,
            },
            old_val: 0.25,
            new_val: 0.75,
        };
        op.apply(&mut doc).expect("SetParam(Macro) 必须可应用");
        assert_eq!(
            macro_value(&doc, 1),
            0.75,
            "SetParam(Macro) 必须写到被寻址的宏"
        );
        assert_eq!(macro_value(&doc, 0), 0.5, "SetParam(Macro) 不许碰到 0 号宏");
        op.invert(&doc)
            .expect("求逆")
            .apply(&mut doc)
            .expect("撤销");
        assert_eq!(doc, before, "SetParam(Macro) 必须逐位可逆");

        // ---- ③ 越界下标必须报**它们自己的**错误码（不是笼统的状态不符） ----
        let snapshot = doc.clone();
        let bad_param = Op::SetParam {
            target: AutomationTarget::DeviceParam {
                track_id: f.lead,
                slot_index: 1,
                param_index: 9,
            },
            old_val: 0.0,
            new_val: 0.0,
        };
        assert!(
            matches!(
                bad_param.apply(&mut doc),
                Err(ModelError::ParamIndexOutOfRange { index: 9, .. })
            ),
            "参数下标越界必须报 ParamIndexOutOfRange"
        );
        let bad_macro = Op::SetMacro {
            track_id: f.lead,
            macro_index: 9,
            old_val: 0.0,
            new_val: 0.5,
        };
        assert!(
            matches!(
                bad_macro.apply(&mut doc),
                Err(ModelError::MacroIndexOutOfRange { index: 9, .. })
            ),
            "宏下标越界必须报 MacroIndexOutOfRange"
        );
        let bad_slot = Op::SetParam {
            target: AutomationTarget::DeviceParam {
                track_id: f.lead,
                slot_index: 9,
                param_index: 0,
            },
            old_val: 0.0,
            new_val: 0.0,
        };
        assert!(
            matches!(
                bad_slot.apply(&mut doc),
                Err(ModelError::DeviceSlotOutOfRange { index: 9, .. })
            ),
            "设备槽越界必须报 DeviceSlotOutOfRange"
        );
        assert_eq!(doc, snapshot, "越界路径不得改文档");
    }

    /// **每一个**自带 `previous_*` 撤销载荷的删除类变体都要核对"载荷 == 文档现值"。
    ///
    /// 实测：`stale_payloads_are_rejected_before_mutating` 只覆盖 `DeleteNote` 与
    /// `ModifyNoteVelocity`；把 `RemoveClipPlacement` / `RemoveDevice` / `RemoveTrack`
    /// / `DisconnectRouting` / `RemoveSection` / `RemoveScene` /
    /// `RemoveAutomationPoint` / `RemoveAutomationLane` 的比对**逐个**关掉时，全仓判据
    /// 保持全绿 —— 于是一个陈旧的撤销载荷会静默删掉**别的**东西。
    #[test]
    fn every_remove_with_a_previous_payload_rejects_a_stale_payload() {
        let f = fixture();
        let volume_target = AutomationTarget::TrackVolume { track_id: f.lead };
        let cases: Vec<(&'static str, Op)> = vec![
            (
                "RemoveClipPlacement",
                Op::RemoveClipPlacement {
                    track_id: f.lead,
                    placement_id: f.placement,
                    previous_placement: ClipPlacement {
                        start_tick: 1,
                        ..fixture_document().tracks[&f.lead].clips[&f.placement]
                    },
                },
            ),
            (
                "RemoveDevice",
                Op::RemoveDevice {
                    track_id: f.lead,
                    slot_index: 0,
                    previous_device: DeviceDefinition {
                        name: "Stale".to_owned(),
                        ..fixture_document().tracks[&f.lead].devices[0].clone()
                    },
                },
            ),
            (
                "RemoveTrack",
                Op::RemoveTrack {
                    track_id: f.bass,
                    previous_track: TrackV3 {
                        name: "Stale".to_owned(),
                        ..fixture_document().tracks[&f.bass].clone()
                    },
                },
            ),
            (
                "DisconnectRouting",
                Op::DisconnectRouting {
                    edge_id: f.edge,
                    previous_edge: RoutingEdge {
                        gain_db: Some(-1.0),
                        ..fixture_document().routing_graph.edges[&f.edge]
                    },
                },
            ),
            (
                "RemoveSection",
                Op::RemoveSection {
                    section_id: f.section,
                    previous_section: SectionV3 {
                        end_tick: 1,
                        ..fixture_document().sections[&f.section].clone()
                    },
                },
            ),
            (
                "RemoveScene",
                Op::RemoveScene {
                    scene_id: f.scene,
                    previous_scene: SceneV3 {
                        name: "Stale".to_owned(),
                        ..fixture_document().scenes[&f.scene].clone()
                    },
                },
            ),
            (
                "RemoveAutomationPoint",
                Op::RemoveAutomationPoint {
                    target: volume_target,
                    point_id: fixture_id(80),
                    previous_point: AutomationPoint {
                        value: 0.0,
                        ..fixture_document().tracks[&f.lead].automation_lanes[&volume_target].points
                            [&fixture_id(80)]
                    },
                },
            ),
            (
                "RemoveAutomationLane",
                Op::RemoveAutomationLane {
                    target: volume_target,
                    previous_lane: AutomationLane {
                        write_mode: crate::project::AutomationWriteMode::Write,
                        ..fixture_document().tracks[&f.lead].automation_lanes[&volume_target]
                            .clone()
                    },
                },
            ),
        ];

        for (name, op) in cases {
            let mut doc = fixture_document();
            let snapshot = doc.clone();
            assert_eq!(
                op.apply(&mut doc),
                Err(ModelError::OpStateMismatch { op: name }),
                "{name} 的陈旧载荷必须被拒绝"
            );
            assert_eq!(doc, snapshot, "{name} 被拒之后文档必须逐字节不动");
        }
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
        // ⚠ **计数不写死**（负责人 2026-10-14 授权：纯计数类断言放开、功能优先）。
        // `showcase_ops()` 的**完整性**（每个 `Op` 变体都有代表实例）由
        // `every_op_variant_round_trips_byte_for_byte_or_is_explicitly_not_applicable`
        // 保证：它对**现读的**变体清单逐个取 `showcase_ops()` 的实例，取不到记 `Fail`，
        // 并断言 `table.len() == declared.len()` 且**零失败** ⇒ 少一条实例必红
        // ⇒ 把 38 写在这里是**冗余**，而且每加一个 `Op` 就要改一次。
        // 这里只留**防空**：脚本解析出 0 条必须**响亮失败**（台账第 41 条）。
        assert!(
            !all_ops.is_empty(),
            "showcase_ops() 解析出 0 条 ⇒ 脚本失灵，判据必须响亮失败而不是空绿"
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
    /// ⭐ **按 `enum` 键**取 `origin.oneOf` 里的**单元变体**分支（⛔ **不按下标**）。
    ///
    /// ⚠ 为什么不能写 `oneOf[0]`：`origin.oneOf` 的分支**顺序**不是契约的一部分 ——
    /// 实测（本仓历史）把 `McpEdit` 插到 `oneOf[0]` 位置会让按下标读的判据读到**错的形状**
    /// （`106 passed / 3 failed`）。⭐ 而"挪动时要记得同步下标"**⛔ 不是守卫**：
    /// 顺序一变，按下标读会**静默**取到 `McpProposal`／`McpEdit` 分支，判据却仍绿。
    ///
    /// ⭐ 现在改成**按内容定位**：谁的 `enum` 是数组，谁就是单元枚举分支。
    /// ⚠ **找不到 ⇒ 返回 `None`，调用方必须响亮失败**（第 41 条）——
    /// ⛔ 不许退回"找不到就跳过"，那会让判据永远绿。
    fn origin_unit_enum_branch(schema: &serde_json::Value) -> Option<&serde_json::Value> {
        let branches = schema["properties"]["origin"]["oneOf"].as_array()?;
        assert!(
            !branches.is_empty(),
            "`origin.oneOf` 是空数组 ⇒ 契约损坏或路径变了（⛔ 不是静默跳过）"
        );
        branches.iter().find(|branch| branch["enum"].is_array())
    }

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
    /// ⚠ **本轮非空**（第一次重新启用这份欠账）：新增的 `SetAsset` / `RemoveAsset`
    /// 是**资产声明**那两个变体，集成者用 `scripts/dev/gen-ops-schema-branches.py`
    /// 从枚举**现读**并重生成 `properties.op.oneOf` ⇒ 补齐后本清单**必须清空**，
    /// 否则 `op_variants_match_ops_schema_exactly` 会一直红并指名"清空本清单"。
    // ⚠ 已清空（**集成者**在补完契约分支后清）：契约现在**含有** `SetAsset` / `RemoveAsset`
    // ⇒ `enum − contract == ∅` ⇒ 若仍留着这两条，本清单会与差集不等并**指名"清空本清单"**
    // （⭐ 这正是欠账机制的设计: ⭐ 补完即红、⭐ 不靠谁记得）。
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
        // ⚠ **计数不写死**（负责人 2026-10-14 授权：纯计数类断言放开、功能优先）。
        // 「契约 ↔ 枚举」的**集合相等**已由 `every_op_variant_is_declared_in_the_contract`
        // 与 `op_variants_match_ops_schema_exactly` 保证 ⇒ 把 34 写在这里是**冗余**，
        // 而且每加一个 `Op` 就要改一次（本会话已因此级联过两次）。
        // 这里只留**防空**：解析失灵必须**响亮失败**（台账第 41 条），⛔ 不许静默通过。
        assert!(
            !contract.is_empty(),
            "op.oneOf 解析出 0 个变体 ⇒ 解析失灵或契约损坏: {contract:?}"
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
        let raw = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ops.rs"),
        )
        .expect("读取 ops.rs");
        // ⚠ 读入后**立刻归一到 LF**（Windows 工作树是 CRLF ⇒ 多行锚点会找不到）。
        let source = normalize_lf(&raw);
        // ⭐ 用**花括号配对**取代 `find("\n    }")`：同时免疫 CRLF 与缩进。
        let body = braced_body_after(&source, "pub const fn name(&self)");
        let mut declared: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for chunk in body.split("Self::").skip(1) {
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

    /// ⭐ **顺序无关的证明**（内存 JSON，⛔ 不碰 `schemas/**`）：把带 `enum` 的分支
    /// **挪到末尾**（模拟"有人插一个分支到前面"）⇒ 按内容定位仍取到**同一批**单元变体，
    /// ⭐ 而同一份内存文档若按**下标 `[0]`** 读会取到 `McpProposal`（⛔ 不是 enum）。
    ///
    /// ⚠ 存在理由：`origin.oneOf` 的分支顺序**不是契约**，而按下标读的那两处会**静默**
    /// 取错分支（历史实测 `106 passed / 3 failed`）⇒ "挪动时记得同步下标"⛔ 不是守卫。
    ///
    /// ⚠ 实测教训（本判据第一版）：`serde_json::json!` 会**移动**它的操作数 ⇒ 同一批
    /// `Value` 不能在两个 `json!` 之间复用（第二处会拿到 `Null`，让判据假红）。
    /// 这里用**独立的字面构造**，⛔ 不共享变量。
    #[test]
    fn origin_unit_branch_is_found_regardless_of_branch_order() {
        let wrap = |one_of: serde_json::Value| serde_json::json!({"properties": {"origin": {"oneOf": one_of}}});
        let unit_literal = || serde_json::json!({"enum": ["UserUi", "MidiInput", "UndoRedo"]});
        let proposal_literal = || serde_json::json!({"required": ["McpProposal"]});
        let mcp_edit_literal = || serde_json::json!({"required": ["McpEdit"]});

        // 契约现状：enum 分支在 `[0]`。
        let as_declared = wrap(serde_json::json!([
            unit_literal(),
            proposal_literal(),
            mcp_edit_literal()
        ]));
        // ⭐ 变异：把 enum 分支挪到**末尾**（= 有人插了一个分支到前面）。
        let reordered = wrap(serde_json::json!([
            proposal_literal(),
            mcp_edit_literal(),
            unit_literal()
        ]));

        let from_declared = origin_unit_enum_branch(&as_declared).expect("现状必须找得到");
        let from_reordered = origin_unit_enum_branch(&reordered).expect("挪动后仍必须找得到");
        assert_eq!(
            from_declared["enum"], from_reordered["enum"],
            "按**内容**定位必须与分支顺序无关"
        );
        assert_eq!(
            from_reordered["enum"],
            serde_json::json!(["UserUi", "MidiInput", "UndoRedo"]),
            "挪动之后取到的必须仍是**单元变体**那一支"
        );

        // ⭐ 反证：同一份挪动后的文档，若按**下标 [0]** 读会取到 `McpProposal`（⛔ 不是 enum）。
        let by_index = &reordered["properties"]["origin"]["oneOf"][0];
        assert!(
            by_index["enum"].is_null(),
            "证明：按 `[0]` 读挪动后的文档取到的**不是** enum 分支（旧写法的失效形态）"
        );
        assert_eq!(
            by_index["required"][0], "McpProposal",
            "证明：按下标读到的确实是 `McpProposal` 分支"
        );

        // ⭐ 防空（第 41 条）：没有 enum 分支 ⇒ 返回 None（调用方随之响亮失败）。
        let no_enum = wrap(serde_json::json!([proposal_literal(), mcp_edit_literal()]));
        assert!(
            origin_unit_enum_branch(&no_enum).is_none(),
            "没有 enum 分支时必须返回 None（⛔ 不静默给空表）"
        );
        // 空 `oneOf` ⇒ 必须**响亮失败**（panic），⛔ 不是静默跳过。
        let empty = wrap(serde_json::json!([]));
        let panicked = std::panic::catch_unwind(|| origin_unit_enum_branch(&empty).is_some());
        assert!(panicked.is_err(), "空 `oneOf` 必须响亮失败（第 41 条）");
    }

    /// `OpOrigin` 的两种形状必须与契约的 `origin.oneOf` 对应（ADR-0001 D13）。
    #[test]
    fn origin_variants_match_ops_schema_origin_one_of() {
        let schema = ops_schema();
        let unit_names: std::collections::BTreeSet<String> = origin_unit_enum_branch(&schema)
            .expect("origin.oneOf 里必须有一个带 `enum` 的分支（单元变体）")["enum"]
            .as_array()
            .expect("带 `enum` 的那个 `origin.oneOf` 分支必须是数组")
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

    /// `routing_graph.nodes` 恒按字典序 [MODEL-AST-003]。
    ///
    /// 为什么需要：`commit` 用 `partition_point` 做二分插入（注释写明"按字典序插入 ⇒
    /// `nodes` 恒有序"），而 `RoutingGraph::validate()` **不**检查有序（只查端点与重复）。
    /// 实测（本次注入）：把 `nodes.insert(position, *node)` 换成 `nodes.push(*node)` 时，
    /// 全仓判据保持全绿 —— 既有夹具的 `nodes` 是 `[1, 2, 3]`，被判据加进去的新身份恰好
    /// 都大于它们，于是 push 与二分插入给出同一个列表。
    /// 本判据插入一个身份**小于**全部既有节点的节点，直接钉住列表顺序。
    #[test]
    fn routing_node_ops_keep_the_node_list_sorted() {
        let mut doc = fixture_document();
        let before = doc.routing_graph.nodes.clone();
        // R93：`windows(2).all(…)` 在**长度 < 2** 时恒真 ⇒ 先钉住夹具下界，
        // 否则"插入最小节点后仍有序"可以在退化夹具上空转。
        assert!(
            before.len() >= 2,
            "夹具必须至少两个节点, 否则下面的有序性断言是空转的: {before:?}"
        );
        assert!(
            before.windows(2).all(|pair| pair[0] < pair[1]),
            "夹具自身必须已经有序: {before:?}"
        );

        let smallest = fixture_id(0);
        Op::AddRoutingNode { node: smallest }
            .apply(&mut doc)
            .expect("新增路由节点");
        assert_eq!(
            doc.routing_graph.nodes.first(),
            Some(&smallest),
            "新节点必须排在最前（push 会把它放到末尾）"
        );
        assert!(
            doc.routing_graph.nodes.len() >= 3,
            "插入后必须至少三个节点, 否则有序性断言在退化输入上空转"
        );
        assert!(
            doc.routing_graph
                .nodes
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "nodes 必须保持字典序: {:?}",
            doc.routing_graph.nodes
        );
        assert_eq!(doc.validate(), Ok(()));

        Op::RemoveRoutingNode { node: smallest }
            .apply(&mut doc)
            .expect("移除路由节点");
        assert_eq!(doc.routing_graph.nodes, before, "增删必须互为逆");
    }

    /// `MoveNote` 的增量取反溢出（`i64::MIN`）必须被**拒绝**，不得 wrapping。
    ///
    /// 为什么需要：`structural_inverse` 用 `checked_neg` 防溢出，但它只有在"逆操作的
    /// 前置条件也恰好放行"时才可观测。实测（本次注入）：把 `delta_tick.checked_neg()…?`
    /// 换成 `delta_tick.wrapping_neg()` 时全仓判据保持全绿 —— 既有判据里音符的
    /// `start_tick` 都是 0，于是逆操作前置条件的 `shifted_tick` 负数守卫先报错，
    /// `checked_neg` 从未被单独观察到。
    /// 本判据把音符放到时钟顶端（`u64::MAX`）：`u64::MAX + i64::MIN` 仍是合法 tick，
    /// 逆操作的前置条件放行，唯一的守卫就是 `checked_neg`。
    #[test]
    fn inverting_a_min_delta_move_note_is_rejected_not_wrapped() {
        let f = fixture();
        let mut doc = fixture_document();
        doc.note_mut(&f.clip, &f.note).expect("音符存在").start_tick = u64::MAX;

        let op = Op::MoveNote {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            delta_tick: i64::MIN,
            delta_pitch: 0,
        };
        assert_eq!(
            op.invert(&doc),
            Err(ModelError::OpStateMismatch { op: "MoveNote" }),
            "i64::MIN 的取反溢出必须报 OpStateMismatch, 不得 wrapping 成一个错误的逆操作"
        );
    }

    /// `AddClipPlacement` 必须在落盘**之前**拒绝"片段池里没有这个片段"（类别 1：先污染、后报错）。
    ///
    /// 为什么需要：`commit` 会无条件把摆放插进 `track.clips`，入口唯一的悬空引用检查
    /// 是 `precondition` 里的 `doc.clip_pool.contains_key(&placement.clip_id)`。
    /// 实测（本次注入）：把那个 `if` 的条件改成恒假时全仓判据保持全绿 —— `apply` 返回
    /// `Ok(())`、文档里多出一条指向不存在片段的摆放，直到 `YebanProjectV1::validate()`
    /// 才报错（而撤销/保存路径未必每次都跑全量 `validate`）。本判据钉住 Op 入口的
    /// 具体错误码与"被拒之后文档逐字节不动"。
    #[test]
    fn add_clip_placement_rejects_a_placement_for_a_clip_outside_the_pool() {
        let f = fixture();
        let mut doc = fixture_document();
        let before = serde_json::to_vec(&doc).expect("序列化");
        let ghost = fixture_id(999);
        assert!(
            !doc.clip_pool.contains_key(&ghost),
            "探针片段必须不在池子里"
        );
        let error = Op::AddClipPlacement {
            track_id: f.bass,
            placement: ClipPlacement {
                id: fixture_id(998),
                clip_id: ghost,
                start_tick: 0,
                duration_ticks: 960,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        }
        .apply(&mut doc)
        .expect_err("指向池外片段的摆放必须被拒");
        assert_eq!(error, ModelError::ClipNotFound { id: ghost });
        assert_eq!(
            serde_json::to_vec(&doc).expect("序列化"),
            before,
            "被拒之后文档必须逐字节不动"
        );
        assert_eq!(doc.validate(), Ok(()));
    }

    /// `ConnectRouting` 必须在落盘**之前**拒绝"端点不在 `routing_graph.nodes` 里"。
    ///
    /// 为什么需要：同族的数值校验（`edge.validate()`）已有判据
    /// （`connect_routing_rejects_a_non_finite_gain_before_mutating`），但**端点存在性**
    /// 那一段 `for endpoint in [edge.source_node, edge.destination_node]` 没有判据。
    /// 实测（本次注入）：把那个 `if` 的条件改成恒假时全仓判据保持全绿 —— `apply` 返回
    /// `Ok(())`、悬空端点的边入图，直到 `RoutingGraph::validate()` 才报错。
    /// 本判据对源端点与目标端点**各**钉一次，并钉住"被拒之后文档逐字节不动"。
    #[test]
    fn connect_routing_rejects_an_endpoint_outside_the_node_list() {
        let f = fixture();
        let ghost = fixture_id(997);
        for (source, destination) in [(ghost, f.master), (f.bass, ghost)] {
            let mut doc = fixture_document();
            let before = serde_json::to_vec(&doc).expect("序列化");
            let edge_id = fixture_id(996);
            let error = Op::ConnectRouting {
                edge: RoutingEdge {
                    id: edge_id,
                    source_node: source,
                    destination_node: destination,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            }
            .apply(&mut doc)
            .expect_err("端点不在 nodes 里的路由边必须被拒");
            assert_eq!(error, ModelError::RoutingNodeNotFound { id: ghost });
            assert_eq!(
                serde_json::to_vec(&doc).expect("序列化"),
                before,
                "被拒之后文档必须逐字节不动"
            );
            assert!(!doc.routing_graph.edges.contains_key(&edge_id));
            assert_eq!(doc.validate(), Ok(()));
        }
    }

    /// `ConnectRouting` 必须先校验载荷的数值有限性（类别 1：先污染、后报错）。
    ///
    /// 为什么需要：`commit` 会无条件把边插进 `routing_graph.edges`，入口唯一的载荷校验
    /// 是 `precondition` 里的 `edge.validate()`。实测（本次注入）：删掉那一行时全仓判据
    /// 保持全绿 —— `apply` 返回 `Ok(())`、非有限增益入图，随后
    /// `YebanProjectV1::validate()` 才报错。本判据钉住 Op 入口的具体错误码与
    /// "被拒之后文档逐字节不动"。
    #[test]
    fn connect_routing_rejects_a_non_finite_gain_before_mutating() {
        let f = fixture();
        for gain in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut doc = fixture_document();
            let before = serde_json::to_vec(&doc).expect("序列化");
            let edge_id = fixture_id(31);
            let error = Op::ConnectRouting {
                edge: RoutingEdge {
                    id: edge_id,
                    source_node: f.bass,
                    destination_node: f.master,
                    kind: RoutingKind::TrackToBus,
                    gain_db: Some(gain),
                },
            }
            .apply(&mut doc)
            .expect_err("非有限增益的路由边必须被拒");
            assert!(
                matches!(&error, ModelError::NonFiniteValue { field, .. } if *field == "routing.edge.gain_db"),
                "实际 {error:?}"
            );
            assert_eq!(
                serde_json::to_vec(&doc).expect("序列化"),
                before,
                "被拒之后文档必须逐字节不动"
            );
            assert!(!doc.routing_graph.edges.contains_key(&edge_id));
            assert_eq!(doc.validate(), Ok(()));
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

    /// `SetNoteTiming` 只改起点与时值，且旧值对不上时**逐字节不动**。
    ///
    /// 为什么需要：这条 op 的存在理由就是"不经过删 + 加" ⇒ 若它顺手丢掉别的字段，
    /// 或陈旧载荷被放行，它就比删 + 加更危险（调用方以为只是改时间）。这里把
    /// "表现力字段原样保留"与"两个旧值必须同时对上"都钉住。
    #[test]
    fn set_note_timing_moves_and_resizes_without_touching_other_fields() {
        let f = fixture();
        let mut doc = fixture_document();
        // 给音符塞满**非默认**表现力字段，任何一处被覆盖都会在下面被抓到。
        {
            let note = doc.note_mut(&f.clip, &f.note).expect("音符存在");
            note.velocity = 33;
            note.probability = Some(0.25);
            note.ratchet = Some(3);
            note.micro_timing_ticks = Some(-17);
            note.syllable = Some("la".to_owned());
            note.phonemes.push("L".to_owned());
        }
        let before = doc.note(&f.clip, &f.note).expect("音符存在").clone();
        assert_eq!((before.start_tick, before.duration_ticks), (0, 480));
        let before_doc = doc.clone();

        let timing = Op::SetNoteTiming {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            old_start_tick: 0,
            old_duration_ticks: 480,
            new_start_tick: 1920,
            new_duration_ticks: 240,
        };
        timing.apply(&mut doc).expect("合法的时间写入必须被接受");
        let after = doc.note(&f.clip, &f.note).expect("音符存在");
        assert_eq!((after.start_tick, after.duration_ticks), (1920, 240));
        assert_eq!(after.id, before.id, "身份不能变");
        assert_eq!(after.pitch, before.pitch, "音高不能变");
        assert_eq!(after.velocity, before.velocity, "力度不能变");
        assert_eq!(after.probability, before.probability, "概率不能变");
        assert_eq!(after.ratchet, before.ratchet, "连击不能变");
        assert_eq!(
            after.micro_timing_ticks, before.micro_timing_ticks,
            "微时序不能变"
        );
        assert_eq!(after.syllable, before.syllable, "歌词不能变");
        assert_eq!(after.phonemes, before.phonemes, "音素不能变");

        // 逆操作由自带旧值构造：必须逐字节还原（含上面塞满的表现力字段）。
        timing.apply_inverse(&mut doc).expect("逆操作可应用");
        assert_eq!(doc, before_doc, "撤销必须逐字节还原");

        // 两个旧值**各自**对不上都必须被拒，且文档逐字节不动。
        let mut fresh = fixture_document();
        for stale in [
            Op::SetNoteTiming {
                track_id: f.lead,
                clip_id: f.clip,
                note_id: f.note,
                old_start_tick: 1,
                old_duration_ticks: 480,
                new_start_tick: 0,
                new_duration_ticks: 480,
            },
            Op::SetNoteTiming {
                track_id: f.lead,
                clip_id: f.clip,
                note_id: f.note,
                old_start_tick: 0,
                old_duration_ticks: 479,
                new_start_tick: 0,
                new_duration_ticks: 480,
            },
        ] {
            assert_eq!(
                stale.apply(&mut fresh),
                Err(ModelError::OpStateMismatch {
                    op: "SetNoteTiming"
                })
            );
            assert_eq!(fresh, fixture_document(), "被拒的写入不得改文档");
        }

        // 零时值：复用 `MidiNote::validate` 的 `ZeroDuration`，⛔ 不新增错误变体。
        let zero = Op::SetNoteTiming {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            old_start_tick: 0,
            old_duration_ticks: 480,
            new_start_tick: 0,
            new_duration_ticks: 0,
        };
        assert_eq!(zero.apply(&mut fresh), Err(ModelError::ZeroDuration));
        assert_eq!(fresh, fixture_document(), "零时值不得改文档");
    }

    /// 时间轴平移的 **`u64` 上界**：越过 `u64::MAX` 必须被拒。
    ///
    /// 为什么需要（第五轮注入实测）：上界由**两道**防线共同承担 —— `shifted_tick` 里的
    /// `shifted > i128::from(u64::MAX)` 判定，与 `u64::try_from(shifted).ok()` 的兜底。
    /// 单独关掉任一条时全仓判据保持全绿（另一条兜住）；把**两条同时**关掉（上界判定
    /// 删掉 + 兜底改成 `unwrap_or(0)`）时全仓判据**仍然全绿** ⇒ "越过 `u64::MAX` 非法"
    /// 这条契约此前没有任何判据。
    #[test]
    fn shifting_a_note_past_the_u64_tick_ceiling_is_rejected() {
        assert_eq!(shifted_tick(0, 0), Some(0));
        assert_eq!(shifted_tick(u64::MAX, 0), Some(u64::MAX));
        assert_eq!(
            shifted_tick(u64::MAX - 1, 1),
            Some(u64::MAX),
            "上界本身必须可达"
        );
        assert_eq!(shifted_tick(u64::MAX, 1), None, "越过 u64::MAX 必须非法");
        assert_eq!(shifted_tick(u64::MAX, i64::MAX), None);
        assert_eq!(shifted_tick(0, -1), None, "越过 0 必须非法");

        // 端到端：负向越界必须被拒，且不改文档。
        let f = fixture();
        let mut doc = fixture_document();
        let before = doc.clone();
        let backwards = Op::MoveNote {
            track_id: f.lead,
            clip_id: f.clip,
            note_id: f.note,
            delta_tick: i64::MIN,
            delta_pitch: 0,
        };
        assert_eq!(
            backwards.apply(&mut doc),
            Err(ModelError::OpStateMismatch { op: "MoveNote" })
        );
        assert_eq!(doc, before, "越界的平移绝不能改文档");
    }

    /// `Op` 的**顺序独立性只在明确的子集上成立**：目标不相交的操作可交换；
    /// 目标相交的操作**不可交换**（后一条载荷里的旧值会被前一条改写）。
    ///
    /// 为什么需要（第五轮普查）：全仓 `grep -rn commut` **零命中** —— "哪些子集可交换"
    /// 这条契约此前没有任何判据；顺序敏感性只由各变体自己的 `old_*` 守卫间接表达，
    /// 而"顺序**有**意义"这一半从没被正面写下来过。
    #[test]
    fn disjoint_target_ops_commute_while_same_target_ops_do_not() {
        let f = fixture();
        let mute_lead = Op::SetTrackMute {
            track_id: f.lead,
            old_mute: false,
            new_mute: true,
        };
        let mute_bass = Op::SetTrackMute {
            track_id: f.bass,
            old_mute: false,
            new_mute: true,
        };

        // 可交换：两条操作作用在**不同**音轨上 ⇒ 两种顺序必须得到同一份文档。
        let mut forward = fixture_document();
        mute_lead.apply(&mut forward).expect("静音 lead");
        mute_bass.apply(&mut forward).expect("静音 bass");
        let mut backward = fixture_document();
        mute_bass.apply(&mut backward).expect("静音 bass");
        mute_lead.apply(&mut backward).expect("静音 lead");
        assert_eq!(forward, backward, "目标不相交的操作必须可交换");
        assert_ne!(forward, fixture_document(), "两种顺序都必须真的改动了文档");
        assert!(forward.track(&f.lead).expect("lead").mute);
        assert!(forward.track(&f.bass).expect("bass").mute);

        // 不可交换：两条操作改**同一**音轨的同一个字段 ⇒ 倒序时旧值是陈旧的。
        let on = Op::SetTrackMute {
            track_id: f.lead,
            old_mute: false,
            new_mute: true,
        };
        let off = Op::SetTrackMute {
            track_id: f.lead,
            old_mute: true,
            new_mute: false,
        };
        let mut in_order = fixture_document();
        on.apply(&mut in_order).expect("开");
        off.apply(&mut in_order).expect("关");
        assert!(
            !in_order.track(&f.lead).expect("lead").mute,
            "正序必须回到未静音"
        );

        let mut reversed = fixture_document();
        assert_eq!(
            off.apply(&mut reversed),
            Err(ModelError::OpStateMismatch { op: "SetTrackMute" }),
            "同目标操作的顺序**有意义**：倒序时载荷旧值与文档现值不符"
        );
        assert_eq!(reversed, fixture_document(), "被拒的操作不得改文档");
    }

    /// `StampedOp::user_ui` 必须**原样**保留来源与时间戳。
    ///
    /// 为什么需要（第六轮注入实测）：把时间戳换成 `timestamp.saturating_add(1)` 时全仓
    /// 判据保持全绿 —— 本构造器零产线调用点，且没有任何判据断言过它写进去的两个字段。
    #[test]
    fn user_ui_stamps_the_ui_origin_and_the_given_timestamp() {
        let stamped = StampedOp::user_ui(
            1_760_000_000_123,
            Op::SetTrackMute {
                track_id: fixture_id(2),
                old_mute: false,
                new_mute: true,
            },
        );
        assert_eq!(stamped.origin, OpOrigin::UserUi);
        assert_eq!(stamped.timestamp, 1_760_000_000_123);
        // 与通用入口同源：`new(UserUi, t, op)` 必须逐字段相同。
        assert_eq!(
            stamped,
            StampedOp::new(
                OpOrigin::UserUi,
                1_760_000_000_123,
                Op::SetTrackMute {
                    track_id: fixture_id(2),
                    old_mute: false,
                    new_mute: true,
                }
            )
        );
    }

    /// 空 `Batch` 与**嵌套** `Batch` 的行为逐条钉住（顺序独立性的其余形态）。
    ///
    /// 为什么需要（第八轮面 5）：第六轮只打了"`Batch` 子操作倒序"（由既有的
    /// `inverted_batch_is_applied_in_reverse_order` 判红），**空批次**与**嵌套批次**
    /// 这两格没有判据：空批次必须是恒等（应用与求逆都不改文档、逆还是空批次），
    /// 嵌套批次的内层顺序必须与外层一致。
    #[test]
    fn empty_and_nested_batches_are_identity_and_ordered() {
        let f = fixture();
        let mut doc = fixture_document();
        let before = doc.clone();

        // ---- 空批次：应用是恒等，逆操作还是空批次 ----
        let empty = Op::Batch {
            ops: Vec::new(),
            description: String::new(),
        };
        empty.apply(&mut doc).expect("空批次必须通过");
        assert_eq!(doc, before, "空批次不得改文档");
        assert_eq!(
            empty.invert(&doc).expect("空批次的逆操作"),
            Op::Batch {
                ops: Vec::new(),
                description: String::new(),
            }
        );

        // ---- 嵌套批次：内层顺序与外层一致（先 AddNote 再 DeleteNote ⇒ 回到原状）----
        let note_id = fixture_id(90);
        let note = MidiNote::new(note_id, 960, 64, 480);
        let add = Op::AddNote {
            track_id: f.lead,
            clip_id: f.clip,
            note: note.clone(),
        };
        let remove = Op::DeleteNote {
            track_id: f.lead,
            clip_id: f.clip,
            note_id,
            previous_note: note.clone(),
        };
        let nested = Op::Batch {
            ops: vec![
                Op::Batch {
                    ops: vec![add.clone()],
                    description: "inner".to_owned(),
                },
                remove.clone(),
            ],
            description: "outer".to_owned(),
        };
        nested.apply(&mut doc).expect("嵌套批次必须通过");
        assert_eq!(doc, before, "先加后删必须回到原状");
        assert_eq!(doc.validate(), Ok(()), "批次结束后文档必须自洽");

        // R58：上面两条 `assert_eq!` 依赖 `YebanProjectV1::PartialEq` —— 同一个 `==` 上
        // 必须有一条 `assert_ne!`，否则 `derive PartialEq` 被削弱成"恒等"时它们全部变成
        // 空判据而**没有一条变红**。
        {
            let mut only_added = fixture_document();
            Op::AddNote {
                track_id: f.lead,
                clip_id: f.clip,
                note: note.clone(),
            }
            .apply(&mut only_added)
            .expect("只加音符必须通过");
            assert_ne!(only_added, before, "只加音符的文档必须与原件不同");
            assert_eq!(only_added, only_added.clone());
        }

        // 顺序**有**意义：倒过来（先删后加）时内层的前置条件不成立。
        let reversed = Op::Batch {
            ops: vec![
                remove,
                Op::Batch {
                    ops: vec![add],
                    description: "inner".to_owned(),
                },
            ],
            description: "outer".to_owned(),
        };
        let mut other = fixture_document();
        assert!(
            reversed.apply(&mut other).is_err(),
            "顺序有意义：先删一个不存在的音符必须被拒"
        );
        assert_eq!(other, fixture_document(), "失败的批次不得改文档");
    }

    /// `Op` 的 `==` 必须**区分载荷与变体**（R58 的探针）。
    ///
    /// 为什么需要（第八轮 R58 对照实验）：把 `Op` 的 `derive PartialEq` 删掉、换成"恒真"
    /// 手写 impl 后，全仓判据**一条都不红** —— 而本 crate 有**成批**依赖
    /// `assert_eq!(back, op)` 的往返判据（`op_variants_match_ops_schema_exactly`、
    /// 历史 DAG 编解码、样本往返……）。那批判据在 `PartialEq` 被削弱后会**同时变成空判据**。
    /// 本判据落在**同一个** `==` 上：同一变体不同载荷、同一变体不同目标、不同变体，
    /// 三者都必须判为不同。
    #[test]
    fn op_equality_separates_payloads_targets_and_variants() {
        let a = Op::SetTrackMute {
            track_id: fixture_id(2),
            old_mute: false,
            new_mute: true,
        };
        assert_eq!(a, a.clone(), "同一值必须与自己相等（成对的正侧）");
        assert_ne!(
            a,
            Op::SetTrackMute {
                track_id: fixture_id(2),
                old_mute: false,
                new_mute: false,
            },
            "同一变体、不同载荷必须判为不同"
        );
        assert_ne!(
            a,
            Op::SetTrackMute {
                track_id: fixture_id(3),
                old_mute: false,
                new_mute: true,
            },
            "同一变体、不同目标必须判为不同"
        );
        assert_ne!(
            a,
            Op::SetTrackSolo {
                track_id: fixture_id(2),
                old_solo: false,
                new_solo: true,
            },
            "不同变体必须判为不同"
        );
    }
    /// **R48 两半 ＋ R51**：`AutomationTarget` 的 `validate_param_value` 分支必须**逐臂点名**。
    ///
    /// 为什么需要（第九轮 · 生产代码通配臂普查）：`ops.rs:1688` 的
    /// `match target { … TrackPan … Macro … _ => Ok(()) }` 是**生产代码里唯一**一处覆盖
    /// 公开枚举的通配兜底 —— 给 `AutomationTarget` 加第 6 个**有界**变体时，它会**默认通过**
    /// （R48 的 fail-open 形态）。本判据给每个臂编号（无通配符 `match` 探针 ⇒ 加变体即编译错），
    /// 并逐臂钉住 `validate_param_value` 在该臂上的判决。
    #[test]
    fn every_automation_target_arm_is_numbered_and_its_verdict_is_pinned() {
        let track = fixture_id(2);
        let edge = fixture_id(30);
        let volume = AutomationTarget::TrackVolume { track_id: track };
        let pan = AutomationTarget::TrackPan { track_id: track };
        let send = AutomationTarget::SendGain {
            track_id: track,
            edge_id: edge,
        };
        let device = AutomationTarget::DeviceParam {
            track_id: track,
            slot_index: 0,
            param_index: 0,
        };
        let mac = AutomationTarget::Macro {
            track_id: track,
            macro_index: 0,
        };

        /// R48 探针：**无通配符** `match`。加变体而不更新本函数 ⇒ 编译错。
        const fn arm(target: AutomationTarget) -> usize {
            match target {
                AutomationTarget::TrackVolume { .. } => 0,
                AutomationTarget::TrackPan { .. } => 1,
                AutomationTarget::SendGain { .. } => 2,
                AutomationTarget::DeviceParam { .. } => 3,
                AutomationTarget::Macro { .. } => 4,
            }
        }

        const ARM_COUNT: usize = 5;
        // (编号, 目标, 该臂的判决：`true` = 取值域由本函数判, `false` = 本函数放行)
        let table: [(usize, AutomationTarget, bool); ARM_COUNT] = [
            (0, volume, false),
            (1, pan, true),
            (2, send, false),
            (3, device, false),
            (4, mac, true),
        ];
        let mut numbered: Vec<usize> = Vec::new();
        for (index, target, bounded) in table {
            assert_eq!(arm(target), index, "臂编号必须与表行一致");
            // 无界臂：任意有限值都必须放行（它们的取值域由别处/界面负责）。
            if !bounded {
                assert_eq!(
                    validate_param_value(target, 4_000.0),
                    Ok(()),
                    "臂 {index} 在模型层无固有界 ⇒ 有限值必须放行"
                );
            }
            numbered.push(index);
        }
        assert_eq!(
            numbered,
            (0..ARM_COUNT).collect::<Vec<_>>(),
            "臂编号必须恰好是 0..N（R48 的臂编号断言）"
        );

        // 有界两臂：两侧边界都要点名（拒绝侧 + 恰好到界）。
        assert_eq!(
            validate_param_value(pan, 1.0),
            Ok(()),
            "声相上界本身必须放行"
        );
        assert_eq!(
            validate_param_value(pan, 2.0),
            Err(ModelError::PanOutOfRange { value: 2.0 })
        );
        assert_eq!(validate_param_value(mac, 1.0), Ok(()), "宏上界本身必须放行");
        assert_eq!(
            validate_param_value(mac, 1.5),
            Err(ModelError::MacroValueOutOfRange { value: 1.5 })
        );
        // 非有限值先于臂判定被拒（与臂无关的那条前置条件）。
        assert_eq!(
            validate_param_value(volume, f32::INFINITY),
            Err(ModelError::NonFiniteValue {
                field: "param.value",
                value: f64::INFINITY,
            })
        );
    }

    /// `[MODEL-AST-002]` / `[ARCH-OPS-001]`：**速度可改且可逆**。
    ///
    /// 三条读数：
    /// ① 改速度 ⇒ `doc.bpm` 变成新值，且**只有它**变（⛔ 不碰 `time_signature`、
    ///    ⛔ 不碰 `transport`）；
    /// ② `inverse` 应用之后**逐字节**回到原状（`f64` 走逐位比较 ⇒ 连位型都不变）；
    /// ③ 陈旧的 `old_bpm`（不等于文档现值）必须报 `OpStateMismatch`，且**文档逐字节不动**。
    #[test]
    fn set_tempo_is_reversible_and_rejects_a_stale_old_bpm() {
        let mut doc = fixture_document();
        let before = serde_json::to_vec(&doc).expect("序列化");
        let old_bpm = doc.bpm;
        assert_eq!(old_bpm, crate::project::DEFAULT_BPM, "夹具口径");

        // ① 改动只落在 bpm 上。
        let op = Op::SetTempo {
            old_bpm,
            new_bpm: 140.0,
        };
        op.precondition(&doc).expect("前置条件必须成立");
        op.apply(&mut doc).expect("必须能应用");
        assert_eq!(doc.bpm, 140.0, "bpm 必须变成新值");
        let mut expected = fixture_document();
        expected.bpm = 140.0;
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            serde_json::to_vec(&expected).expect("s"),
            "改动必须**只**落在 bpm 上（⛔ 不碰 time_signature / transport）"
        );

        // ② 逆操作逐字节回到原状。
        op.apply_inverse(&mut doc).expect("逆必须能应用");
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            before,
            "逆操作之后必须逐字节回到原状"
        );

        // ③ 陈旧的 old_bpm 被拒，且文档逐字节不动。
        let stale = Op::SetTempo {
            old_bpm: old_bpm + 1.0,
            new_bpm: 140.0,
        };
        let stale_before = serde_json::to_vec(&doc).expect("s");
        let error = stale.apply(&mut doc).expect_err("陈旧旧值必须被拒");
        assert!(
            matches!(error, ModelError::OpStateMismatch { .. }),
            "必须报 OpStateMismatch, 实际 {error:?}"
        );
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            stale_before,
            "被拒之后文档必须逐字节不动"
        );

        // ④ 非有限与越界的新值都必须被**前置条件**拒绝（⛔ 不靠 apply 之后的
        //    `YebanProjectV1::validate` 兜底）。
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0] {
            let op = Op::SetTempo {
                old_bpm,
                new_bpm: bad,
            };
            let error = op
                .precondition(&doc)
                .expect_err("非法新值必须被前置条件拒绝");
            assert!(
                matches!(error, ModelError::BpmOutOfRange { .. }),
                "new_bpm={bad} 必须报 BpmOutOfRange, 实际 {error:?}"
            );
        }
    }

    /// `[MODEL-AST-002]` / `[ARCH-OPS-001]`：**署名可改且可逆**。
    ///
    /// 四条读数：
    /// ① 改动**只**落在 `author`（与"同工程只换 author"的克隆**逐字节**比较）；
    /// ② `inverse` 应用之后**逐字节**回到原状（`String` 克隆往返不丢字节）；
    /// ③ 陈旧的 `old_author`（不等于文档现值）必须报 `OpStateMismatch`，且文档逐字节不动；
    /// ④ ⚠ **多字节 UTF-8** 也要逐字节往返（`String` 的相等是字节序比较，
    ///    但这里仍显式用非 ASCII 证明"不是按字符数比较"）。
    #[test]
    fn set_author_is_reversible_and_rejects_a_stale_old_author() {
        let mut doc = fixture_document();
        let before = serde_json::to_vec(&doc).expect("序列化");
        let old_author = doc.author.clone();

        // ① 改动只落在 author 上（用非 ASCII，证明逐字节而非逐字符）。
        let new_author = "夜半 · Yeban".to_owned();
        let op = Op::SetAuthor {
            old_author: old_author.clone(),
            new_author: new_author.clone(),
        };
        op.precondition(&doc).expect("前置条件必须成立");
        op.apply(&mut doc).expect("必须能应用");
        assert_eq!(doc.author, new_author, "author 必须变成新值");
        let mut expected = fixture_document();
        expected.author = new_author.clone();
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            serde_json::to_vec(&expected).expect("s"),
            "改动必须**只**落在 author 上"
        );

        // ② 逆操作逐字节回到原状。
        op.apply_inverse(&mut doc).expect("逆必须能应用");
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            before,
            "逆操作之后必须逐字节回到原状"
        );

        // ③ 陈旧的 old_author 被拒，且文档逐字节不动。
        let stale = Op::SetAuthor {
            old_author: "别的人".to_owned(),
            new_author: new_author.clone(),
        };
        let stale_before = serde_json::to_vec(&doc).expect("s");
        let error = stale.apply(&mut doc).expect_err("陈旧旧值必须被拒");
        assert!(
            matches!(error, ModelError::OpStateMismatch { .. }),
            "必须报 OpStateMismatch, 实际 {error:?}"
        );
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            stale_before,
            "被拒之后文档必须逐字节不动"
        );

        // ④ 长度相同但内容不同的旧值也必须被拒（逐字节，不是"长度相同就算相等"）。
        let same_len: String = "x".repeat(old_author.len().max(1));
        if same_len != old_author {
            let same_len_stale = Op::SetAuthor {
                old_author: same_len,
                new_author: new_author.clone(),
            };
            assert!(
                same_len_stale.precondition(&doc).is_err(),
                "等长但内容不同的旧值必须被拒"
            );
        }
    }

    /// `[ARCH-OPS-001]` / `[MODEL-AST-002]`：**音频片段增益可改且可逆**。
    ///
    /// 为什么必须有它：`ClipContent::Audio::gain_db` 在创建时由 `yeban_import_audio`
    /// 的 `gainDb` 写一次（`crates/yeban-mcp/src/domain/import_audio.rs:396` / `:501`），
    /// 而 `Op` 全集此前**没有**写它的变体 ⇒ ⭐ **音频片段建得出、增益改不了**。
    ///
    /// 七条读数：
    /// ① 改动**只**落在 `gain_db`（与"同工程只换 gain"的克隆**逐字节**比较）；
    /// ② `inverse` 应用之后**逐字节**回到原状；
    /// ③ 陈旧的 `old_gain_db` 必须报 `OpStateMismatch`，且文档逐字节不动；
    /// ④ **位不同**的旧值（`-0.0` vs `+0.0`）也被拒（逐位，⛔ 不是 `==`）；
    /// ⑤ 目标是 **MIDI 片段** ⇒ `ClipContentKindMismatch`（⛔ panic、⛔ 静默），文档不动；
    /// ⑥ 新值非有限 ⇒ `NonFiniteValue { field: "clip.audio.gain_db" }`，文档不动；
    /// ⑦ **值域敞开**：有限极值照收（现状只有有限性检查，**没有** dB 上下限）。
    #[test]
    fn set_clip_gain_is_reversible_and_rejects_a_stale_old_gain_and_a_midi_clip() {
        let f = fixture();
        let mut doc = fixture_document();
        let before = serde_json::to_vec(&doc).expect("序列化");
        let old_gain_db = match &doc.clip_pool[&f.audio_clip].content {
            ClipContent::Audio { gain_db, .. } => *gain_db,
            ClipContent::Midi { .. } => panic!("夹具的 audio_clip 必须是音频片段"),
        };
        assert_eq!(old_gain_db, 0.0, "夹具口径");

        // ① 改动只落在 gain_db 上。
        let op = Op::SetClipGain {
            clip_id: f.audio_clip,
            old_gain_db,
            new_gain_db: -6.0,
        };
        op.precondition(&doc).expect("前置条件必须成立");
        op.apply(&mut doc).expect("必须能应用");
        let mut expected = fixture_document();
        let ClipContent::Audio { gain_db, .. } = &mut expected
            .clip_pool
            .get_mut(&f.audio_clip)
            .expect("条目")
            .content
        else {
            panic!("夹具的 audio_clip 必须是音频片段");
        };
        *gain_db = -6.0;
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            serde_json::to_vec(&expected).expect("s"),
            "改动必须**只**落在 gain_db 上（⛔ 不碰 name / asset / 摆放）"
        );

        // ② 逆操作逐字节回到原状。
        op.apply_inverse(&mut doc).expect("逆必须能应用");
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            before,
            "逆操作之后必须逐字节回到原状"
        );

        // ③ 陈旧的 old_gain_db 被拒，且文档逐字节不动。
        let stale = Op::SetClipGain {
            clip_id: f.audio_clip,
            old_gain_db: 1.5,
            new_gain_db: -3.0,
        };
        let stale_before = serde_json::to_vec(&doc).expect("s");
        let error = stale.apply(&mut doc).expect_err("陈旧旧值必须被拒");
        assert!(
            matches!(error, ModelError::OpStateMismatch { .. }),
            "必须报 OpStateMismatch, 实际 {error:?}"
        );
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            stale_before,
            "被拒之后文档必须逐字节不动"
        );

        // ④ `-0.0` 与 `+0.0` 数值相等但**位不同** ⇒ 逐位比较必须拒。
        assert!(!same_f32(-0.0, 0.0));
        let signed_zero_stale = Op::SetClipGain {
            clip_id: f.audio_clip,
            old_gain_db: -0.0,
            new_gain_db: -3.0,
        };
        assert!(
            signed_zero_stale.precondition(&doc).is_err(),
            "-0.0 与 +0.0 位不同, 必须判为陈旧载荷"
        );

        // ⑤ 目标是 MIDI 片段 ⇒ 内容类型门（⛔ panic ⛔ 静默）。
        let midi_target = Op::SetClipGain {
            clip_id: f.clip,
            old_gain_db: 0.0,
            new_gain_db: -3.0,
        };
        let midi_before = serde_json::to_vec(&doc).expect("s");
        let error = midi_target.apply(&mut doc).expect_err("MIDI 片段必须被拒");
        assert_eq!(
            error,
            ModelError::ClipContentKindMismatch { id: f.clip },
            "必须报既有的 ClipContentKindMismatch（⛔ 新变体）"
        );
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            midi_before,
            "被拒之后文档必须逐字节不动"
        );

        // ⑥ 非有限新值 ⇒ NonFiniteValue("clip.audio.gain_db")。
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let non_finite = Op::SetClipGain {
                clip_id: f.audio_clip,
                old_gain_db,
                new_gain_db: bad,
            };
            let error = non_finite
                .precondition(&doc)
                .expect_err("非有限新增益必须被拒");
            assert!(
                matches!(
                    error,
                    ModelError::NonFiniteValue {
                        field: "clip.audio.gain_db",
                        ..
                    }
                ),
                "必须报 NonFiniteValue(\"clip.audio.gain_db\"), 实际 {error:?}"
            );
        }

        // ⑦ 值域敞开：有限极值照收，且读回**逐位**相同（⛔ 不发明 dB 范围）。
        for extreme in [f32::MIN, f32::MAX, f32::MIN_POSITIVE, -0.0] {
            let op = Op::SetClipGain {
                clip_id: f.audio_clip,
                old_gain_db,
                new_gain_db: extreme,
            };
            op.precondition(&doc)
                .expect("有限极值必须被接受（dB 无模型级范围）");
            let mut landed = fixture_document();
            op.apply(&mut landed).expect("必须能应用");
            let ClipContent::Audio { gain_db, .. } = &landed.clip_pool[&f.audio_clip].content
            else {
                panic!("夹具的 audio_clip 必须是音频片段");
            };
            assert_eq!(
                gain_db.to_bits(),
                extreme.to_bits(),
                "读回必须逐位等于写入的有限极值（实测值 {extreme}）"
            );
        }
    }

    /// `[MODEL-AST-002]` / `[ARCH-OPS-001]`：**标题可改且可逆**。
    ///
    /// 为什么必须有它：`title` 是**必需**的顶层字段，而 `Op` 全集此前没有写它的变体
    /// ⇒ 唯一的结构写点是创建工程时的 `domain/project_create.rs:370`
    /// ⇒ ⭐ **工程建得出、标题改不了**。本判据钉住改的那一半。
    ///
    /// 五条读数：
    /// ① 改动**只**落在 `title`（与"同工程只换 title"的克隆**逐字节**比较）；
    /// ② `inverse` 应用之后**逐字节**回到原状（`String` 克隆往返不丢字节）；
    /// ③ 陈旧的 `old_title` 必须报 `OpStateMismatch`，且文档逐字节不动；
    /// ④ **等长但内容不同**的旧标题也被拒（证明是逐字节，不是"长度相同就算相等"）；
    /// ⑤ **空串是合法的明示值**（契约是裸 `{"type":"string"}`、`validate` 不查标题
    ///    ⇒ ⛔ 本变体不发明"非空"约束）。
    #[test]
    fn set_title_is_reversible_and_rejects_a_stale_old_title() {
        let mut doc = fixture_document();
        let before = serde_json::to_vec(&doc).expect("序列化");
        let old_title = doc.title.clone();
        assert_eq!(old_title, "Ops Fixture", "夹具口径");

        // ① 改动只落在 title 上（用非 ASCII，证明逐字节而不是逐字符）。
        let new_title = "夜半 · Yeban".to_owned();
        let op = Op::SetTitle {
            old_title: old_title.clone(),
            new_title: new_title.clone(),
        };
        op.precondition(&doc).expect("前置条件必须成立");
        op.apply(&mut doc).expect("必须能应用");
        assert_eq!(doc.title, new_title, "title 必须变成新值");
        let mut expected = fixture_document();
        expected.title = new_title.clone();
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            serde_json::to_vec(&expected).expect("s"),
            "改动必须**只**落在 title 上（⛔ 不碰 author / bpm / id）"
        );

        // ② 逆操作逐字节回到原状。
        op.apply_inverse(&mut doc).expect("逆必须能应用");
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            before,
            "逆操作之后必须逐字节回到原状"
        );

        // ③ 陈旧的 old_title 被拒，且文档逐字节不动。
        let stale = Op::SetTitle {
            old_title: "别的标题".to_owned(),
            new_title: new_title.clone(),
        };
        let stale_before = serde_json::to_vec(&doc).expect("s");
        let error = stale.apply(&mut doc).expect_err("陈旧旧值必须被拒");
        assert!(
            matches!(error, ModelError::OpStateMismatch { .. }),
            "必须报 OpStateMismatch, 实际 {error:?}"
        );
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            stale_before,
            "被拒之后文档必须逐字节不动"
        );

        // ④ 等长但内容不同的旧值也必须被拒（逐字节，不是"长度相同就算相等"）。
        let same_len: String = "x".repeat(old_title.len());
        assert_ne!(same_len, old_title, "构造的旧值必须真的不同");
        let same_len_stale = Op::SetTitle {
            old_title: same_len,
            new_title: new_title.clone(),
        };
        assert!(
            same_len_stale.precondition(&doc).is_err(),
            "等长但内容不同的旧值必须被拒"
        );

        // ⑤ 空串是**合法的明示值** —— 契约与 `validate` 都没有"非空"约束。
        let empty = Op::SetTitle {
            old_title: old_title.clone(),
            new_title: String::new(),
        };
        empty.precondition(&doc).expect("空串标题必须是合法载荷");
        let mut cleared = fixture_document();
        empty.apply(&mut cleared).expect("清空标题必须能应用");
        assert_eq!(cleared.title, "", "空串必须如实落地");
        assert!(
            cleared.validate().is_ok(),
            "空标题的工程必须仍然合法（⛔ 不发明非空约束）"
        );
    }

    /// **全变体往返审计**（`[ARCH-OPS-001]` 的通则，机械化）。
    ///
    /// 指标（对象 + 单位）：对 `showcase_ops()` 里**每一个** `Op` 实例（单位 = 条），
    /// `apply` ⇒ `apply_inverse` 之后，工程的**逐字节**序列化是否回到原位。
    ///
    /// 三类结果，**全部显式分类**（⛔ 不静默跳过 —— 本仓的"空气隙"纪律）：
    /// * `Pass` —— 施加成功、逆施加成功、且逐字节回到原位；
    /// * `NotApplicable` —— **有理由**地不适用（例如该 op 在一份文档上只能施加一次而
    ///   夹具状态不满足），理由逐条记进表；
    /// * `Fail` —— 施加或逆施加失败，**或**往返之后字节不同。`Fail` 会让判据红。
    ///
    /// ⚠ 变体清单**从源码现读**（`enum Op` 的花括号配对 + 顶层变体行），
    /// ⛔ 不手抄数字（第 44 条）。
    ///
    /// ⚠ 本判据**不复用** `re_applying_...` 的 `RELATIVE_TRANSFORMS` 例外表：
    /// `MoveNote` 是唯一的**相对**变换（载荷是增量），它的"第二遍被接受"是**已知且已登记**
    /// 的行为，而**往返**（正一遍 + 逆一遍）对它仍然必须逐字节回原位 —— 这正是本判据要判的。
    #[test]
    fn every_op_variant_round_trips_byte_for_byte_or_is_explicitly_not_applicable() {
        #[derive(Debug, PartialEq, Eq)]
        enum Verdict {
            Pass,
            /// ⚠ **本轮实测：本臂当前不可达** —— 34/34 变体在 `fixture_document()` 上
            /// 的施加性检查**全部通过** ⇒ 没有任何一条走这里。保留它是因为
            /// **"⛔ 不许静默跳过"** 要求分类**在结构上存在**（将来若某个变体的前置条件
            /// 在标准夹具上不再成立，它会走这条臂而不是被悄悄剔除）。
            /// `#[allow(dead_code)]` 是**如实**标注这个现状，⛔ 不是绕过。
            #[allow(dead_code)]
            NotApplicable(&'static str),
            Fail(String),
        }

        // ---- 变体全集：从源码现读（**复用**模块级辅助）----
        let declared = declared_op_variants();

        // ---- 逐变体审计（每个变体取 showcase 里**第一个**实例）----
        let showcase = showcase_ops();
        let mut table: Vec<(String, Verdict)> = Vec::new();
        for name in &declared {
            let Some(op) = showcase.iter().find(|op| op.name() == name.as_str()) else {
                table.push((
                    name.clone(),
                    Verdict::Fail("showcase_ops() 里没有这个变体的代表实例".to_owned()),
                ));
                continue;
            };
            let mut doc = fixture_document();
            let before = match serde_json::to_vec(&doc) {
                Ok(bytes) => bytes,
                Err(error) => {
                    table.push((
                        name.clone(),
                        Verdict::Fail(format!("序列化前置失败: {error}")),
                    ));
                    continue;
                }
            };
            // ⚠ **适用性检查**（⛔ 不许静默跳过）：前置条件不成立 ⇒ 明确记为
            // "不适用"并记下理由，而不是当成失败（那会把"夹具不合法"与"模型缺陷"混在一起）。
            if let Err(error) = op.precondition(&doc) {
                table.push((
                    name.clone(),
                    Verdict::NotApplicable(match error {
                        ModelError::OpStateMismatch { .. } => "前置条件: 载荷与文档状态不符",
                        _ => "前置条件不成立",
                    }),
                ));
                continue;
            }
            if let Err(error) = op.apply(&mut doc) {
                table.push((
                    name.clone(),
                    Verdict::Fail(format!("apply 失败: {error:?}")),
                ));
                continue;
            }
            if let Err(error) = op.apply_inverse(&mut doc) {
                table.push((
                    name.clone(),
                    Verdict::Fail(format!("apply_inverse 失败: {error:?}")),
                ));
                continue;
            }
            let after = match serde_json::to_vec(&doc) {
                Ok(bytes) => bytes,
                Err(error) => {
                    table.push((
                        name.clone(),
                        Verdict::Fail(format!("序列化后置失败: {error}")),
                    ));
                    continue;
                }
            };
            if before == after {
                table.push((name.clone(), Verdict::Pass));
            } else {
                table.push((
                    name.clone(),
                    Verdict::Fail(format!(
                        "往返之后字节不同: 前 {} 字节 / 后 {} 字节",
                        before.len(),
                        after.len()
                    )),
                ));
            }
        }

        // ---- 逐条打印（供人读；测试输出用 --nocapture 看）----
        for (name, verdict) in &table {
            match verdict {
                Verdict::Pass => eprintln!("ROUND-TRIP {name:24} PASS"),
                Verdict::NotApplicable(reason) => {
                    eprintln!("ROUND-TRIP {name:24} NOT-APPLICABLE ({reason})");
                }
                Verdict::Fail(reason) => eprintln!("ROUND-TRIP {name:24} FAIL ({reason})"),
            }
        }
        let passed = table.iter().filter(|(_, v)| *v == Verdict::Pass).count();
        let na = table
            .iter()
            .filter(|(_, v)| matches!(v, Verdict::NotApplicable(_)))
            .count();
        let failed = table
            .iter()
            .filter(|(_, v)| matches!(v, Verdict::Fail(_)))
            .count();
        eprintln!(
            "ROUND-TRIP SUMMARY declared={} pass={passed} not_applicable={na} fail={failed}",
            declared.len()
        );

        // ---- 判据：覆盖必须等于现读的变体数，且**零失败** ----
        assert_eq!(
            table.len(),
            declared.len(),
            "每个变体都要有一行（⛔ 不许静默跳过）"
        );
        let failures: Vec<&(String, Verdict)> = table
            .iter()
            .filter(|(_, v)| matches!(v, Verdict::Fail(_)))
            .collect();
        assert!(failures.is_empty(), "有变体未能逐字节往返: {failures:#?}");
    }

    /// 四条**新 `set` 类**（`SetTransportConfig` / `SetTempo` / `SetAuthor` / `SetTitle`）的
    /// `old_*` 相等口径**在合法文档上必须一致**（`[ARCH-OPS-001]` 的"陈旧载荷必被拒"）。
    ///
    /// ⚠ 四条里前三条的**手段不同**，这是实测事实（⛔ 不是推断）：
    /// * `SetTransportConfig` —— 派生的 `PartialEq`（⚙ 内部是逐字段、浮点用 `==`）；
    /// * `SetTempo` —— `same_f64`（**逐位**，`to_bits`）；
    /// * `SetAuthor` —— `String` 的 `==`（**逐字节**）；
    /// * `SetTitle` —— 同 `SetAuthor`（`String` 逐字节）。
    ///
    /// 差异只可能出在**浮点**上：`same_f64` 让 `-0.0` 与 `+0.0` 不等，而 `==` 让它们相等。
    /// ⚠ 但 `bpm` 的合法区间是 `MIN_BPM..=MAX_BPM`（`20.0..=999.0`）⇒ 一份 `validate()`
    /// 通过的文档里 `doc.bpm == ±0.0` **不可能** ⇒ ⭐ **那条差异路径不可达**。
    /// 本判据把这句话**机械地钉住**：在合法文档上，四者对"相等"与"不等"给出**同一**判定。
    /// 若将来谁把 `same_f64` 改成 `==`、或把 `MIN_BPM` 调成 `0.0`，本判据会说话。
    #[test]
    fn set_class_old_equality_agrees_on_valid_documents() {
        // ① `SetTransportConfig`：逐字段构造"只差一个字段"的旧值。
        let doc = fixture_document();
        let base = doc.transport;
        let mut variants: Vec<TransportConfig> = Vec::new();
        let mut toggled = base;
        toggled.metronome_enabled = !toggled.metronome_enabled;
        variants.push(toggled);
        let mut bars = base;
        bars.count_in_bars = bars.count_in_bars.wrapping_add(1);
        variants.push(bars);
        let mut quant = base;
        quant.launch_quantization = match quant.launch_quantization {
            LaunchQuantization::Off => LaunchQuantization::Bar,
            _ => LaunchQuantization::Off,
        };
        variants.push(quant);
        for (i, v) in variants.iter().enumerate() {
            let op = Op::SetTransportConfig {
                old_config: *v,
                new_config: base,
            };
            assert!(
                op.precondition(&doc).is_err(),
                "① 变体 {i} 与文档现值不同 ⇒ 必须被拒"
            );
        }
        let same = Op::SetTransportConfig {
            old_config: base,
            new_config: base,
        };
        assert!(same.precondition(&doc).is_ok(), "① 相等时必须接受");

        // ② `SetTempo`：合法区间内只有"不同数值"一种情形（-0.0/0.0 不在合法区间）。
        let op = Op::SetTempo {
            old_bpm: doc.bpm + 1.0,
            new_bpm: doc.bpm,
        };
        assert!(op.precondition(&doc).is_err(), "② 不同数值必须被拒");
        let same = Op::SetTempo {
            old_bpm: doc.bpm,
            new_bpm: doc.bpm,
        };
        assert!(same.precondition(&doc).is_ok(), "② 相等时必须接受");

        // ③ `SetAuthor`：只有"不同字符串"一种情形。
        let op = Op::SetAuthor {
            old_author: format!("{}x", doc.author),
            new_author: doc.author.clone(),
        };
        assert!(op.precondition(&doc).is_err(), "③ 不同串必须被拒");
        let same = Op::SetAuthor {
            old_author: doc.author.clone(),
            new_author: doc.author.clone(),
        };
        assert!(same.precondition(&doc).is_ok(), "③ 相等时必须接受");

        // ④ `SetTitle`：与 `SetAuthor` 同一条 `String` 逐字节口径。
        let op = Op::SetTitle {
            old_title: format!("{}x", doc.title),
            new_title: doc.title.clone(),
        };
        assert!(op.precondition(&doc).is_err(), "④ 不同标题必须被拒");
        let same = Op::SetTitle {
            old_title: doc.title.clone(),
            new_title: doc.title.clone(),
        };
        assert!(same.precondition(&doc).is_ok(), "④ 相等时必须接受");

        // ⑤ 合法文档上：`doc.bpm` 与 `0.0` 的关系（证明 `-0.0` 那条路**不可达**）。
        assert!(
            (MIN_BPM..=MAX_BPM).contains(&doc.bpm),
            "⑤ 合法文档的 bpm 必在区间内 ⇒ `-0.0 == 0.0` 那条差异不可达"
        );
    }

    /// `[MODEL-AST-007]` / `[MUST-GATE-014]`：**资产声明可改、可删、可逆**。
    ///
    /// 五条读数：
    /// ① **创建**（`old_asset: None`）只往 `assets` 加**一个**键，且⛔ 不碰 `clip_pool`；
    /// ② 创建的**结构逆**必须是 `RemoveAsset`（⛔ 不是"两个 `Some` 互绕"）⇒ 撤销后键消失、
    ///    文档**逐字节**回到创建前；
    /// ③ **更新**（`old_asset: Some(现值)`）的结构逆是 `SetAsset` 本身（纯交换）⇒
    ///    撤销后逐字节回原位；
    /// ④ 陈旧的 `old_asset`（等于别的值 / 该有却没有）⇒ `OpStateMismatch`，且文档不动；
    /// ⑤ `RemoveAsset`：现值不符、以及删一个**不存在**的键 ⇒ 都 `OpStateMismatch`。
    ///
    /// ⚠ 判据用 `structural_inverse()`（纯函数、⛔ 不读文档）来判"逆是哪个**变体**"，
    /// 用 `apply_inverse(&doc)` 来判"往返逐字节"。两者**不能混**：`invert(&doc)` 会拿
    /// **施加后**的文档去校验逆的前置条件（见 `invert` 的正文），因此它只适用于
    /// "已经施加过"的时刻 —— 这与本判据"先看逆变体"的意图不同。
    #[test]
    fn set_asset_is_reversible_and_rejects_a_stale_old_value() {
        let new_hash = crate::ids::AssetHash::of_bytes(b"snare");
        let new_asset = AssetMetadata {
            hash: new_hash.clone(),
            original_path: "samples/snare.wav".to_owned(),
            byte_len: 2_048,
            media_kind: MediaKind::Audio,
            license: "CC-BY-4.0".to_owned(),
        };

        // ① 创建：只加一个键，⛔ 不碰 clip_pool。
        let mut doc = fixture_document();
        let before = serde_json::to_vec(&doc).expect("序列化");
        let clips_before = doc.clip_pool.clone();
        let create = Op::SetAsset {
            asset_hash: new_hash.clone(),
            old_asset: None,
            new_asset: new_asset.clone(),
        };
        create.precondition(&doc).expect("创建的前置条件必须成立");

        // ② 结构逆必须是 RemoveAsset。
        let structural = create.structural_inverse().expect("创建必须有结构逆");
        assert!(
            matches!(structural, Op::RemoveAsset { .. }),
            "创建的**结构逆**必须是 RemoveAsset（⛔ 不是两个 Some 互绕），实际 {structural:?}"
        );

        create.apply(&mut doc).expect("必须能创建");
        assert_eq!(doc.assets.get(&new_hash), Some(&new_asset));
        assert_eq!(doc.clip_pool, clips_before, "创建资产 ⛔ 不得碰 clip_pool");
        assert_eq!(doc.assets.len(), 2, "夹具原有 1 条 + 新建 1 条");
        create.apply_inverse(&mut doc).expect("逆必须能应用");
        assert!(!doc.assets.contains_key(&new_hash), "撤销后键必须消失");
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            before,
            "创建后撤销必须**逐字节**回到原位"
        );

        // ③ 更新：结构逆仍是 SetAsset；往返逐字节。
        let original = doc
            .assets
            .get(&kick_asset())
            .expect("夹具必须有 kick 资产")
            .clone();
        let mut updated = original.clone();
        updated.license = "CC-BY-4.0".to_owned();
        let before_update = serde_json::to_vec(&doc).expect("s");
        let update = Op::SetAsset {
            asset_hash: kick_asset(),
            old_asset: Some(original.clone()),
            new_asset: updated.clone(),
        };
        update.precondition(&doc).expect("更新的前置条件必须成立");
        assert!(
            matches!(update.structural_inverse(), Ok(Op::SetAsset { .. })),
            "更新的结构逆仍是 SetAsset"
        );
        update.apply(&mut doc).expect("必须能更新");
        assert_eq!(doc.assets.get(&kick_asset()), Some(&updated));
        update.apply_inverse(&mut doc).expect("逆必须能应用");
        assert_eq!(
            serde_json::to_vec(&doc).expect("s"),
            before_update,
            "更新后撤销必须**逐字节**回到原位"
        );

        // ④ 陈旧的 old_asset 被拒，且文档不动。
        for stale in [
            Op::SetAsset {
                asset_hash: kick_asset(),
                old_asset: Some(updated.clone()),
                new_asset: new_asset.clone(),
            },
            Op::SetAsset {
                asset_hash: new_hash.clone(),
                old_asset: Some(new_asset.clone()),
                new_asset: new_asset.clone(),
            },
        ] {
            let snapshot = serde_json::to_vec(&doc).expect("s");
            let error = stale.apply(&mut doc).expect_err("陈旧旧值必须被拒");
            assert!(
                matches!(error, ModelError::OpStateMismatch { .. }),
                "必须报 OpStateMismatch, 实际 {error:?}"
            );
            assert_eq!(
                serde_json::to_vec(&doc).expect("s"),
                snapshot,
                "被拒之后文档必须逐字节不动"
            );
        }

        // ⑤ RemoveAsset：键不存在、以及现值不符，都必须被拒。
        let missing = Op::RemoveAsset {
            asset_hash: new_hash.clone(),
            previous_asset: new_asset.clone(),
        };
        assert!(
            matches!(
                missing.precondition(&doc),
                Err(ModelError::OpStateMismatch { .. })
            ),
            "删除一个不存在的键必须被拒（⛔ 不静默成功）"
        );
        let wrong_previous = Op::RemoveAsset {
            asset_hash: kick_asset(),
            previous_asset: new_asset.clone(),
        };
        assert!(
            matches!(
                wrong_previous.precondition(&doc),
                Err(ModelError::OpStateMismatch { .. })
            ),
            "previous_asset 与现值不符必须被拒"
        );
        let good = Op::RemoveAsset {
            asset_hash: kick_asset(),
            previous_asset: original.clone(),
        };
        good.precondition(&doc).expect("正确的删除必须成立");
        let snapshot = serde_json::to_vec(&doc).expect("s");
        good.apply(&mut doc).expect("必须能删除");
        assert!(!doc.assets.contains_key(&kick_asset()));
        good.apply_inverse(&mut doc).expect("逆必须能应用");
        assert_eq!(serde_json::to_vec(&doc).expect("s"), snapshot);
    }
    /// 把源码文本**归一成 LF**（CRLF 与裸 CR 都变成 `\n`）。
    ///
    /// ⚠ 为什么需要：判据用**多行字面量**当锚点（`"\n    }"` 之类）。本仓 `.gitattributes`
    /// 不存在、`core.autocrlf` 在 Windows runner 上默认为真 ⇒ ⭐ **同一个文件在 Windows 的
    /// 工作树里是 CRLF** ⇒ 内嵌 `\n` 的锚点**找不到**，而 linux/macOS 腿全绿 ⇒
    /// ⭐ **只有 windows 腿红**。照 `project.rs` 与 `model_isolation.rs` 已有的范式，读入后
    /// **立刻**归一。
    fn normalize_lf(text: &str) -> String {
        text.replace("\r\n", "\n").replace('\r', "\n")
    }

    /// 取 `needle` **之后**第一个花括号块的内容（`needle` 必须在块之前）。
    ///
    /// ⚠ 为什么不用 `find("\n    }")` 这类**靠缩进的锚点**：那样的锚点同时受**行尾**
    /// （CRLF）与**缩进**（rustfmt 变化）影响。花括号配对对两者都免疫。
    /// ⚠ **找不到块 ⇒ `panic`**（第 41 条：⛔ 不许静默返回空串让判据空绿）。
    fn braced_body_after<'a>(text: &'a str, needle: &str) -> &'a str {
        let at = text
            .find(needle)
            .unwrap_or_else(|| panic!("源码里找不到锚点 `{needle}`"));
        let rest = &text[at..];
        let open = rest
            .find('{')
            .unwrap_or_else(|| panic!("锚点 `{needle}` 之后没有花括号块"));
        let body = &rest[open + 1..];
        let mut depth = 1_i32;
        for (idx, ch) in body.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &body[..idx];
                    }
                }
                _ => {}
            }
        }
        panic!("锚点 `{needle}` 的花括号块没有闭合");
    }

    /// `pub enum Op` 的**顶层变体名全集**，从源码**现读**（⛔ 不手抄）。
    ///
    /// ⚠ 这是**唯一**一份变体名抽取器：往返审计与 `Op::name()` 守卫**共用**它，
    /// ⛔ 不许各写一份（第 44 条：手抄/复制的清单会漂移）。
    /// ⚠ 抽不到东西 ⇒ `panic`（第 41 条：⛔ 不留永远绿的判据）。
    fn declared_op_variants() -> Vec<String> {
        let src = include_str!("ops.rs");
        let start = src.find("pub enum Op {").expect("枚举存在");
        let rest = &src[start..];
        let mut depth = 0usize;
        let mut end = 0usize;
        for (idx, ch) in rest.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = idx;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &rest[..=end];
        let mut declared: Vec<String> = Vec::new();
        for line in body.lines() {
            let Some(t) = line.strip_prefix("    ") else {
                continue;
            };
            if t.starts_with(' ') || t.is_empty() {
                continue;
            }
            let name: String = t
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            let after = &t[name.len()..];
            if name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && (after.starts_with(" {") || after.starts_with(','))
            {
                declared.push(name);
            }
        }
        declared.sort_unstable();
        declared.dedup();
        assert!(
            !declared.is_empty(),
            "从源码现读变体清单失败（解析器坏了）=> 响亮失败而不是空绿"
        );
        declared
    }

    /// ⭐ **CRLF 回归判据**（内存文本 —— 本地无法造 CRLF 工作树，故用合成文本证明）。
    ///
    /// ⚠ **先纠正一条我自己的错误推断**：我早先以为"CRLF 会让 `find("\n    }")` 找不到锚点"。
    /// ⭐ **实测不成立** —— CRLF 的 `...\r\n    }` **仍然含有**子串 `\n    }` ⇒ 那个 `find` 在
    /// CRLF 上**照样命中**。所以本单在**这两处**的真正收益是**稳健性/一致性**（与仓内 6 处范式
    /// 统一、且不再依赖缩进），⛔ **不是**修一个已证的 CRLF 缺陷。
    ///
    /// ⭐ 但归一**确实**有必要：一旦把 CRLF 文本交给**逐行**消费者（`split('\n')`／
    /// `ends_with`／`==` 比较），⭐ **残留的 `\r` 会出现在每行末尾** ⇒ 那些比较会失败。
    /// 本判据把"归一之后 `\r` 一个不剩"钉住。
    #[test]
    fn lf_normalisation_and_brace_matching_survive_a_crlf_working_tree() {
        let lf = "pub const fn name(&self) -> &'static str {\n    match self {\n        Self::A { .. } => \"A\",\n        Self::B { .. } => \"B\",\n    }\n}\n";
        let crlf = lf.replace('\n', "\r\n");
        assert!(crlf.contains("\r\n"), "夹具必须真的含 CRLF");
        assert_ne!(lf, crlf, "两种行尾必须真的不同，否则本判据空转");
        assert!(!lf.contains('\r'), "LF 版本不得含 CR");
        assert!(crlf.contains('\r'), "CRLF 版本本来含 CR");

        // ⭐ 归一之后：`\r` 一个都不剩（这是归一的**可判**收益）。
        let normalised = normalize_lf(&crlf);
        assert!(!normalised.contains('\r'), "归一之后不得再有 CR 残留");
        assert_eq!(normalised, lf, "归一之后必须与 LF 版本逐字节相同");

        let anchor = "pub const fn name(&self) -> &'static str {";
        // ⭐ 归一 + 花括号配对 ⇒ **两种行尾给出同一段正文**。
        let lf_normalised = normalize_lf(lf);
        let from_lf = braced_body_after(&lf_normalised, anchor);
        let from_crlf = braced_body_after(&normalised, anchor);
        assert_eq!(from_lf, from_crlf, "归一之后两种行尾必须得到同一段正文");
        assert!(from_lf.contains("Self::A") && from_lf.contains("Self::B"));

        // ⚠ **记录实测事实（⛔ 不要重复我早先的错误推断）**：缩进锚点 `"\n    }"` 在
        // **未归一的 CRLF** 文本上**仍然命中**（`\r\n    }` 含该子串）⇒ 它不是这两处的
        // 真正脆弱点。本判据把它**钉成事实**，免得后人再据错因去做改动。
        assert!(
            crlf.find("\n    }").is_some(),
            "实测：CRLF 文本仍含该子串 ⇒ 缩进锚点不是 CRLF 脆弱点（纠正早先推断）"
        );

        // ⭐ 第 41 条：锚点缺失／块未闭合必须**响亮失败**（⛔ 不返回空串）。
        let missing =
            std::panic::catch_unwind(|| braced_body_after(lf, "pub fn nonexistent(").len());
        assert!(
            missing.is_err(),
            "锚点缺失必须 panic（⛔ 不许静默给空正文）"
        );
        let unclosed =
            std::panic::catch_unwind(|| braced_body_after("fn f() { { ", "fn f()").len());
        assert!(unclosed.is_err(), "花括号未闭合必须 panic");
    }

    /// 从 `Op::name()` 的**源码正文**现读 `Self::<变体> => "<字符串>"` 臂。
    ///
    /// ⚠ 这是本判据的**独立真相**：它**不调用** `name()`（那会自我确认），
    /// 而是把**源码里写下的**映射读出来。
    fn name_arms_from_source() -> (
        std::collections::BTreeMap<String, String>,
        Vec<String>,
        Vec<String>,
    ) {
        // ⚠ 同一个理由：归一 + 花括号配对（`include_str!` 把工作树里的行尾也编进来）。
        let src = normalize_lf(include_str!("ops.rs"));
        let body = braced_body_after(&src, "pub const fn name(&self) -> &'static str {");
        let mut map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut duplicate_strings: Vec<String> = Vec::new();
        let mut duplicate_arms: Vec<String> = Vec::new();
        for line in body.lines() {
            let Some(at) = line.find("Self::") else {
                continue;
            };
            let after = &line[at + "Self::".len()..];
            let variant: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let Some(quote) = line.find('"') else {
                continue;
            };
            let Some(close) = line[quote + 1..].find('"') else {
                continue;
            };
            let value = line[quote + 1..quote + 1 + close].to_owned();
            if variant.is_empty() || value.is_empty() {
                continue;
            }
            if !seen.insert(value.clone()) {
                duplicate_strings.push(value.clone());
            }
            if map.insert(variant.clone(), value).is_some() {
                duplicate_arms.push(variant);
            }
        }
        assert!(
            !map.is_empty(),
            "从 name() 源码抽到 0 条臂 => 抽取器失灵 => 响亮失败"
        );
        (map, duplicate_strings, duplicate_arms)
    }

    /// **`Op::name()` 的返回值内容必须有守**（`[ARCH-OPS-001]` / JSON 变体名）。
    ///
    /// ⚠ 它今天**唯一的**保护是**编译器**（漏臂 => `E0004`）。⭐ 但**两个变体返回同一
    /// 字符串**编译得过、今天没有任何判据会红。
    ///
    /// ⭐ **第二重后果（实测）**：`name()` 在生产里被当**线上标识**用 ——
    /// `yeban-mcp/src/domain/automation.rs:258` 把它序列化进 MCP 输出；
    /// `yeban-mcp/src/domain/import_audio.rs:1185` 把它当契约字面量比对。
    /// => 重名不是"不好看"，是线上载荷里的**歧义标识**。
    #[test]
    fn every_op_name_is_unique_and_equals_its_variant_name() {
        let declared = declared_op_variants();
        let showcase = showcase_ops();
        assert!(
            !showcase.is_empty(),
            "showcase_ops() 为空 => 本判据会空绿 => 响亮失败"
        );
        let (arms, duplicate_strings, duplicate_arms) = name_arms_from_source();

        assert!(
            duplicate_strings.is_empty(),
            "Op::name() 有**两个变体返回同一字符串** {duplicate_strings:?} => 它是 JSON \
             变体名（线上载荷标识），重名会让载荷歧义"
        );
        assert!(
            duplicate_arms.is_empty(),
            "Op::name() 有变体出现**两条臂** {duplicate_arms:?} => 后一条覆盖前一条"
        );

        let mismatched: Vec<String> = arms
            .iter()
            .filter(|(variant, value)| variant != value)
            .map(|(variant, value)| format!("{variant} => {value}"))
            .collect();
        assert!(
            mismatched.is_empty(),
            "Op::name() 的文档承诺值与**变体名**一致，但这些不一致: {mismatched:#?}"
        );

        let missing: Vec<&String> = declared
            .iter()
            .filter(|name| !arms.contains_key(name.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "这些变体在 Op::name() 里**没有臂** => 编译器本该拦住（E0004）: {missing:?}"
        );

        let covered: std::collections::BTreeSet<&str> = showcase
            .iter()
            .map(|op| {
                let name = op.name().to_owned();
                arms.iter()
                    .find(|(_, value)| **value == name)
                    .map(|(variant, _)| variant.as_str())
                    .unwrap_or("<未登记>")
            })
            .collect();
        let uncovered: Vec<&String> = declared
            .iter()
            .filter(|name| !covered.contains(name.as_str()))
            .collect();
        assert!(
            uncovered.is_empty(),
            "这些变体没有 showcase 实例 => 运行期一侧漏判了它们: {uncovered:?}"
        );

        eprintln!(
            "OP-NAME variants={} arms={} unique_strings={} equal_to_variant={}",
            declared.len(),
            arms.len(),
            duplicate_strings.is_empty(),
            mismatched.is_empty()
        );
    }
}
