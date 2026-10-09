//! `yeban_edit_notes` 的 `NoteOp` → [`Op`] 编译 [MCP-TOOL-006]。
//!
//! ## 逆操作**不在这里**
//!
//! 本模块只负责把 JSON 编译成 `yeban-model` 的 `Op` 变体。逆操作一律由
//! [`Op::invert`] 提供 —— 在 MCP 层再写一套 `invert` 就会有两份会漂移的真相
//! （判据 `note_ops_are_reversible_through_the_model_inverse` 直接复用模型侧的判据思路：
//! `apply` 之后 `invert` 回去，序列化字节必须回到原样）。
//!
//! ## 规范缺口（`NoteOp` 的形状没有契约）
//!
//! `schemas/mcp-tools.schema.json` 把 `ops` 声明为无约束数组，
//! `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2 只写 `ops: Vec<NoteOp>`。
//! 本模块因此**定义了**四种 `kind`（`add` / `delete` / `move` / `velocity`）作为
//! 本地决策，并把它登记为待裁决项（见 `docs/ledger/tools-domain-notes.md`）。
//!
//! ## `add.note.probability`：让概率触发从工具面**可达**
//!
//! `MidiNote::probability` 是模型既有字段，但改动之前**没有任何工具**能设置它
//! （`parse_note` 不读它）⇒ 模型的概率触发能力在 MCP 工具面上不可达。本模块新增
//! 一个**可选**字段 [`PROBABILITY_FIELD`]（缺省 = 不写 = 必然触发 = 逐字节等于旧行为），
//! 把字面值搬进 `MidiNote`；"这一遍响不响"的裁决**不在本层**，而在
//! `MidiNote::triggers(rng_seed)`（`MODEL-AST-005`，实时引擎与离线母带共用）。
//!
//! 取值的权威判定也在模型层（[`MidiNote::validate`]）：越界 → `ProbabilityOutOfRange`
//! → 契约码 `OUT_OF_RANGE`。本层只额外拦下"不是数字"这类 JSON 形状错误。
//!
//! ## `add.note.ratchet` / `add.note.microTimingTicks`：让**已实现**的渲染能力可达
//!
//! 同一类缺口的第二个实例。离线母带渲染器**已经**按模型语义消费这两个字段
//! （`crate::domain::render` 的连击一节用 `step = (duration_ticks / ratchet).max(1)`
//! 逐脉冲排程；微时序经 `crate::domain::render_math::note_frame_span` 并入起点），
//! 但在这个字段接线之前，17 个工具的**任何一个**都写不了它们 ⇒ 能力已实现、工具面
//! 不可达，而且把 `ratchet` 写进 `ops[].note` 会被**静默丢弃**（`parse_note` 不读它）。
//!
//! 现在：可选字段（缺省 = `None` = 等价于 1 / 无偏移 = 逐字节等于接线之前的行为），
//! 字面值搬进 [`MidiNote`]，区间由模型层把关（`1..=16` / `-240..=240`）。
//!
//! ## `note` 的未知键：响亮拒绝，不静默丢弃
//!
//! 顶层实参已经是这个口径 —— `ToolCall::from_params` 对不在参数表里的键返回
//! `UnknownParam`（"拼错的参数必须被拒绝, 不能静默忽略"）。这条纪律此前**只守了顶层**：
//! `note` 对象是自由形状，多写的键被原样吞掉。现在 [`reject_unknown_note_fields`] 把
//! 同一口径下沉一层，`data.supportedNoteFields` 逐条列出支持集合。
//!
//! **登记边界（本线未接线，绝不静默）**：模型里还有四个表现力字段没有工具面通路 ——
//! `slide` / `pitchBendCurve` / `syllable` / `phonemes`。渲染器对它们一律如实登记进
//! 响应的 `unsupported`（`noteSlide` / `notePitchBend` / `noteLyrics`），而工具面
//! 现在会**响亮拒绝**这四个键（`data.unsupportedNoteFields`），不再吞掉。
//!
//! ## 材料创建形态（`arguments.create: true`）—— 关闭 needs-8 的 MIDI 那一半
//!
//! 台账 `docs/ledger/tools-domain-notes.md:283` 的 **needs-8** 记的事实是：
//! `yeban_propose_section` 要求 `clip_pool` 里至少有一条"MIDI 且至少一个音符"的材料
//! （`section_build.rs` 的 `usable_materials`，缺了报 `CLIP_NOT_FOUND`），
//! 而"没有任何 MCP 工具能让 Agent 把片段放进池子"⇒ 空池工程做不了配器。
//!
//! 那一半缺口由 [`compile_create`] 关闭：`create: true` 时 `clipId` 是**将要新建的**
//! 片段身份（§7.2 的参数表因此**一字不动** —— `clipId` 仍然是必填的"目标片段"），
//! `ops` 里的 `add` 折成一条 `Op::AddClip` 的**初始内容**。
//!
//! 为什么是"扩 `yeban_edit_notes` 的参数"而不是新增工具：
//! `ADR-0001` **D46** 的扩张原则是"先扩既有工具的参数，只有确实不合适才新增工具"，
//! 而新增工具必须同步 `schemas/mcp-tools.schema.json` 的
//! `properties.name.enum` + `ExtensionToolArguments.$defs` + `allOf` 三处
//!（本线禁改 `schemas/**`）。已有的先例是同一条原则下的
//! `yeban_open_project` 的 `create`/`seed`（`domain/project_create.rs`）。
//!
//! 三条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | `create: true` 且池里**已有** `clipId` | `CONFLICT`（`reason = clipAlreadyExists`）—— 与 `yeban_open_project` 的 `create` 同款：绝不覆盖 |
//! | `create: true` 且 `ops` 里有 `delete`/`move`/`velocity` | `INVALID_PARAMETER_RANGE`（`reason = createRequiresAddOps`）—— 新片段里还没有音符可以被它们指向 |
//! | `create: true` 且两个 `add` 抢同一个音符身份 | `INVALID_PARAMETER_RANGE`（`reason = duplicateNoteId`）—— 不静默去重 |
//!
//! 创建出来的片段**只在池子里**（本工具不摆放；摆放是 `Op::AddClipPlacement` 的事，
//! 而"配器"只要求池里有材料）。这一点如实写在 [`compile_create`] 的文档与响应里。
//!
//! ## 摆放形态（`arguments.placement`）—— 关闭 needs-6 的"放置/引用片段"那一半
//!
//! 台账 `docs/ledger/mcp-tools-expansion-notes.md` §6 的 **needs-6** 记的事实是：
//! "没有『放置/引用片段』的工具" —— `Op::AddClipPlacement` 在 MCP 侧只有三个写者
//! （`yeban_open_project` 的 `seed`、`yeban_propose_section` 自建的声部、以及
//! `yeban_import_audio` 的**音频**片段），而**已经躺在 `clip_pool` 里的片段**
//! （例如上面 `create: true` 刚建出来的 MIDI 材料）**没有任何工具**能摆到时间轴上
//! ⇒ 渲染器只遍历 `track.clips`，因此那些材料一帧都不出声。
//!
//! [`parse_placement`] 补上这一半：`placement` 在场时，除音符编辑之外再产出一条
//! [`Op::AddClipPlacement`]，把 `clipId` 摆到 `trackId` 的 `startTick` 上。
//!
//! 三条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | `placement` 与 `create: true` 同给 | `INVALID_PARAMETER_RANGE`（`reason = "placementIsNotCreation"`）—— 先 `create` 再 `place`，两步各自成一个提案 |
//! | `placement` 里的键不在 [`PLACEMENT_FIELDS`] | `INVALID_PARAMETER_RANGE`（`reason = "unknownPlacementField"`） |
//! | 片段推不出长度（非 MIDI / 空 MIDI）且没给 `durationTicks` | `INVALID_PARAMETER_RANGE`（`reason = "durationNotDerivable"`）—— 不猜一个假长度 |
//!
//! 空 `ops` 只在**摆放在场**时被接受：那时"这次调用要做什么"由 `placement` 承载，
//! 音符那一半就是"一个音符都不动"（[`parse_ops`] 自己的空数组守卫**没有**放松）。

use std::collections::BTreeMap;
use std::str::FromStr as _;

use serde_json::{Map, Value};

use yeban_model::music::{MICRO_TIMING_MAX_ABS, RATCHET_MAX, RATCHET_MIN};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, Op, YebanProjectV1,
};

use super::error::{Fault, from_model};
use super::ids::deterministic_id;
use crate::tools::ErrorCode;

/// `create: true` 且没给 `clipName` 时的片段名（**不是**身份，只是给人看的标签）。
pub const DEFAULT_NEW_CLIP_NAME: &str = "Clip";

/// 材料创建形态的开关实参名（`arguments.create`，缺省 `false` = 旧行为）。
///
/// 名字与 `yeban_open_project` 的 `create` **同词同义**（`ADR-0001` D48 的口径：
/// 同一个词必须同一个意思）—— "目标不存在才新建，已存在就响亮拒绝"。
pub const CREATE_PARAM: &str = "create";

/// 材料创建形态的片段名实参（`arguments.clipName`，可选）。
pub const CLIP_NAME_PARAM: &str = "clipName";

/// `add` 音符对象里的**概率触发**字段名（`ops[].note.probability`，可选）。
///
/// 语义与判定入口都在**模型层**，本层只负责搬运字面值：
///
/// - 取值域 `0.0..=1.0`（含端点），由 [`MidiNote::validate`] 把关 ⇒ 越界是
///   `ProbabilityOutOfRange` ⇒ 契约码 `OUT_OF_RANGE`（见 [`super::error::code_for_model`]）；
/// - 缺省（不写这个字段）= `None` = **必然触发**，与加这个字段之前逐字节相同；
/// - "这一遍响不响"由 `MidiNote::triggers(rng_seed)` **确定性**裁决
///   （`MODEL-AST-005`），实时引擎与离线母带用的是同一个入口。
pub const PROBABILITY_FIELD: &str = "probability";

/// `add` 音符对象里的**连击**字段名（`ops[].note.ratchet`，可选）。
///
/// 语义与判定入口都在**模型层**（[`MidiNote::ratchet`]）：`None` 等价于 1；
/// 取值域 `1..=16` 由 [`MidiNote::validate`] 把关（`RATCHET_MIN`/`RATCHET_MAX`）
/// ⇒ 越界是 `RatchetOutOfRange` ⇒ 契约码 `OUT_OF_RANGE`。
///
/// 为什么本字段值得一条工具面通路：离线母带渲染器**已经**按模型语义展开它
/// （`crate::domain::render` 的连击一节：`step = (duration_ticks / ratchet).max(1)`，
/// 与实时引擎同一条公式），但在这个字段接线之前**没有任何工具**能写它 ——
/// 于是"渲染器会展开连击"这件已实现的能力在 17 个工具的面上**不可达**。
/// 交付形态：只搬字面值进 `MidiNote`，"怎么分"不在这层另立第二份规则。
pub const RATCHET_FIELD: &str = "ratchet";

/// `add` 音符对象里的**微时序**字段名（`ops[].note.microTimingTicks`，可选，单位：tick）。
///
/// 语义与判定入口都在**模型层**（[`MidiNote::micro_timing_ticks`]）：`None` 等价于 0；
/// 取值域 `-240..=240` 由 [`MidiNote::validate`] 把关（`MICRO_TIMING_MAX_ABS`）
/// ⇒ 越界是 `MicroTimingOutOfRange` ⇒ 契约码 `OUT_OF_RANGE`。
///
/// 渲染器同样**已经**把它并入起点（`crate::domain::render` 的排程：
/// `placement.start_tick + note.start_tick + micro_timing_ticks`，经
/// `crate::domain::render_math::note_frame_span`）。
pub const MICRO_TIMING_FIELD: &str = "microTimingTicks";

/// `add.note` 对象**允许**出现的全部键。
///
/// 与 [`parse_note`] 真正读取的键**同源**（判据 `expressive_note_field_names_are_pinned`
/// 钉住"不多报"）：集合之外的键一律**响亮拒绝**（[`reject_unknown_note_fields`]），
/// 绝不静默丢弃 —— 顶层实参已经是这个口径（`ToolCall` 的 `UnknownParam`：拼错的参数
/// 必须被拒绝、不能静默忽略），同一条纪律不许只守一层。
pub const NOTE_FIELDS: &[&str] = &[
    "id",
    "startTick",
    "pitch",
    "durationTicks",
    "velocity",
    PROBABILITY_FIELD,
    RATCHET_FIELD,
    MICRO_TIMING_FIELD,
];

/// `yeban_edit_notes` 的**摆放**实参名（`arguments.placement`，可选）。
///
/// 语义：把**已经在 `clip_pool` 里**的片段摆到 `trackId` 的时间轴上
/// （模型 `Op::AddClipPlacement`，渲染器**真的**消费它 —— `crate::domain::render`
/// 只遍历 `track.clips`，池子里没被摆放的片段一帧都不出声）。
///
/// 为什么扩本工具而不新增工具：台账 `docs/ledger/mcp-tools-expansion-notes.md` §6 的
/// needs-6 记的事实是"没有『放置/引用片段』的工具"，并给出两条出路 —— 新增
/// `yeban_place_clip`，**或扩展 `yeban_edit_notes`**。`ADR-0001` **D46** 的扩张原则是
/// "先扩既有工具的参数，只有确实不合适才新增工具"，而新增工具必须同步
/// `schemas/mcp-tools.schema.json` 的 `name.enum` + `ExtensionToolArguments` + `allOf`
/// 三处（本线禁改 `schemas/**`）。
///
/// 本工具此前已经有 `create: true`（**建**材料，`f1098e2`）这一形态；本参数补上它的
/// 下一半（**摆**材料）。`§7.2` 的参数表因此**一字未动**：`ops` 仍是必填实参
/// （只摆放的调用给空数组，见 [`parse_ops`] 的空数组口径）。
pub const PLACEMENT_FIELD: &str = "placement";

/// `placement` 对象里 `add` 形态**允许**出现的全部键。
///
/// 与 [`parse_placement`] 真正读取的键**同源**（判据 `placement_field_names_are_pinned`
/// 钉住"不多报"）：集合之外的键一律**响亮拒绝**
/// （[`reject_placement_fields`]），绝不静默丢弃 —— 与 [`NOTE_FIELDS`] 同一口径。
///
/// 这个词表**只**管 `add` 形态的**内容键**：`kind` 是形态判别键，不在本表里。
pub const PLACEMENT_FIELDS: &[&str] = &["startTick", "durationTicks", "placementId", "muted"];

/// `placement.kind` 的字段名（形态判别键，可选；缺省 = [`PLACEMENT_KIND_ADD`]）。
///
/// 与 `ops[].kind` 同一风格：`kind` 说的是"这一次摆放编辑是哪个动词"，
/// 其余键是那个动词的载荷。
pub const PLACEMENT_KIND_FIELD: &str = "kind";

/// `placement.kind` 的**新增**形态：把**已在池子里**的片段摆到时间轴上
/// （[`Op::AddClipPlacement`]）。也是 `kind` 缺省时的形态 ⇒ 缺省路径逐字节不变。
pub const PLACEMENT_KIND_ADD: &str = "add";

/// `placement.kind` 的**平移**形态：改动一条**已经存在**的摆放的起点。
pub const PLACEMENT_KIND_MOVE: &str = "move";

/// `placement.kind` 的**取走**形态：把一条**已经存在**的摆放从时间轴上移除。
pub const PLACEMENT_KIND_REMOVE: &str = "remove";

/// `placement.kind` 的合法取值集合（错误信息与判据共用同一份真相）。
pub const PLACEMENT_KINDS: [&str; 3] = [
    PLACEMENT_KIND_ADD,
    PLACEMENT_KIND_MOVE,
    PLACEMENT_KIND_REMOVE,
];

/// `add` 形态允许的键 = [`PLACEMENT_FIELDS`] **加上**判别键。
///
/// 单列一个常量是为了不动 [`PLACEMENT_FIELDS`]：后者是 `add` 形态的内容键表，
/// 由判据 `placement_field_names_are_pinned` 逐个钉住；扩展形态时**不改**那张表。
pub const PLACEMENT_ADD_FIELDS: &[&str] = &[
    PLACEMENT_KIND_FIELD,
    "startTick",
    "durationTicks",
    "placementId",
    "muted",
];

/// `move` 形态允许的键：判别键 + 被平移的摆放身份 + **新的**起点。
///
/// `durationTicks` / `muted` 不在表里：[`Op::MoveClipPlacement`] 的载荷**只有**
/// `old_start_tick` / `new_start_tick`，模型层没有"改时值 / 改静音"的变体
/// ⇒ 给出这两个键是**已知但此形态不适用**，[`reject_placement_fields`] 会响亮拒绝
/// （`placementFieldNotApplicable`），绝不静默丢弃。
pub const PLACEMENT_MOVE_FIELDS: &[&str] = &[PLACEMENT_KIND_FIELD, "startTick", "placementId"];

/// `remove` 形态允许的键：判别键 + 被取走的摆放身份。
pub const PLACEMENT_REMOVE_FIELDS: &[&str] = &[PLACEMENT_KIND_FIELD, "placementId"];

/// 一次 `placement` 实参要做的**摆放编辑**（三种形态的编译结果）。
///
/// 三个变体逐一对应模型层的三个 `Op`：`Add` → [`Op::AddClipPlacement`]、
/// `Move` → [`Op::MoveClipPlacement`]、`Remove` → [`Op::RemoveClipPlacement`]。
/// 撤销仍然只有**一份**事实源：本层只把字面值搬进 `Op`，逆操作一律由
/// `Op::invert` 提供（与 [`super::compile`] 同一纪律）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementEdit {
    /// 把池子里的材料摆到 `trackId` 上。
    Add(ClipPlacement),
    /// 把 `placement_id` 这条已有摆放的起点从 `previous_start_tick` 挪到
    /// `new_start_tick`（两者都取自 / 写回**文档**，不是调用方的声明）。
    Move {
        /// 被平移的摆放身份。
        placement_id: EntityId,
        /// 文档里的现值（模型层据此判 `OpStateMismatch`）。
        previous_start_tick: u64,
        /// 目标起点。
        new_start_tick: u64,
    },
    /// 把 `placement_id` 这条已有摆放从时间轴上取走。
    Remove {
        /// 被取走的摆放身份。
        placement_id: EntityId,
        /// 文档里的现值（模型层的撤销载荷，必须逐字段等于文档现值）。
        previous_placement: ClipPlacement,
    },
}

impl PlacementEdit {
    /// 该形态在 `placement.kind` 里的字面名字（错误信息与判据共用同一份真相）。
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Add(_) => PLACEMENT_KIND_ADD,
            Self::Move { .. } => PLACEMENT_KIND_MOVE,
            Self::Remove { .. } => PLACEMENT_KIND_REMOVE,
        }
    }
}

/// 单个片段的**发声数**上限（同时发声的音符数）。
///
/// §7.2 要求 `yeban_edit_notes` "自动进行音域与发声数合法性校验"；
/// 音域（`0..=127`）由模型层把关，发声数这一半由本常量把关。
/// 32 是"一个片段内同时 32 个音符"的保守上限（超过它多半是批量生成的产物，
/// 而不是编曲意图）；超过即 `OUT_OF_RANGE` 并回报**峰值重叠**与位置。
pub const MAX_POLYPHONY: usize = 32;

/// 一个音符编辑操作。
#[derive(Clone, Debug, PartialEq)]
pub enum NoteOp {
    /// 插入音符。
    Add {
        /// 完整音符。
        note: Box<MidiNote>,
    },
    /// 删除音符。
    Delete {
        /// 音符身份。
        note_id: EntityId,
    },
    /// 平移音符。
    Move {
        /// 音符身份。
        note_id: EntityId,
        /// tick 增量。
        delta_tick: i64,
        /// 半音增量。
        delta_pitch: i8,
    },
    /// 修改力度。
    Velocity {
        /// 音符身份。
        note_id: EntityId,
        /// 新力度 `0..=127`。
        velocity: u8,
    },
}

impl NoteOp {
    /// 该操作在 `arguments.ops[].kind` 里的字面名字（错误信息与判据共用同一份真相）。
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Delete { .. } => "delete",
            Self::Move { .. } => "move",
            Self::Velocity { .. } => "velocity",
        }
    }
}

/// 解析 `arguments.ops`。
///
/// 支持的 `kind`（本地定义，见模块头）：
///
/// ```json
/// {"kind":"add","note":{"id":"<可选 26 字符 ULID>","startTick":0,"pitch":60,
///                       "durationTicks":480,"velocity":100,"probability":0.5,
///                       "ratchet":4,"microTimingTicks":-12}}
/// {"kind":"delete","noteId":"<ULID>"}
/// {"kind":"move","noteId":"<ULID>","deltaTick":960,"deltaPitch":12}
/// {"kind":"velocity","noteId":"<ULID>","velocity":80}
/// ```
///
/// `note.probability` / `note.ratchet` / `note.microTimingTicks` 是**可选**字段
/// （缺省逐字节等于旧行为）：给了就是 [`MidiNote`] 对应字段的字面值，语义与判定入口
/// 见 [`PROBABILITY_FIELD`] / [`RATCHET_FIELD`] / [`MICRO_TIMING_FIELD`]。
/// `note` 里 [`NOTE_FIELDS`] 之外的键一律**响亮拒绝**，不静默丢弃。
///
/// # Errors
///
/// - `ops` 不是数组 / 元素不是对象 / 缺字段 / 字段类型不对 / `note` 里有未知键 →
///   `INVALID_PARAMETER_RANGE`（含未知 `kind`）；
/// - 音高、力度、时值、概率、连击、微时序越界 → `OUT_OF_RANGE`；
/// - 身份文本不是合法 ULID → `INVALID_PARAMETER_RANGE`。
pub fn parse_ops(value: &Value) -> Result<Vec<NoteOp>, Fault> {
    let Value::Array(items) = value else {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`ops` 必须是数组",
        ));
    };
    if items.is_empty() {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`ops` 不得为空数组（空操作不是一次编辑请求）",
        ));
    }
    items.iter().map(parse_one).collect()
}

/// 解析单个操作。
fn parse_one(item: &Value) -> Result<NoteOp, Fault> {
    let object = item.as_object().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`ops` 的元素必须是对象, 实际收到 {item}"),
        )
    })?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("kind", "字符串"))?;
    match kind {
        "add" => {
            let note = object
                .get("note")
                .and_then(Value::as_object)
                .ok_or_else(|| missing("note", "对象"))?;
            Ok(NoteOp::Add {
                note: Box::new(parse_note(note)?),
            })
        }
        "delete" => Ok(NoteOp::Delete {
            note_id: read_id(object, "noteId")?,
        }),
        "move" => Ok(NoteOp::Move {
            note_id: read_id(object, "noteId")?,
            delta_tick: read_i64(object, "deltaTick")?,
            delta_pitch: read_i8(object, "deltaPitch")?,
        }),
        "velocity" => Ok(NoteOp::Velocity {
            note_id: read_id(object, "noteId")?,
            velocity: read_range(object, "velocity", 0, 127)?,
        }),
        other => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知 `kind`: `{other}`"),
            serde_json::json!({ "supportedKinds": ["add", "delete", "move", "velocity"] }),
        )),
    }
}

/// 解析 `note` 对象。
fn parse_note(object: &Map<String, Value>) -> Result<MidiNote, Fault> {
    reject_unknown_note_fields(object)?;
    let id = match object.get("id") {
        None | Some(Value::Null) => deterministic_id(&format!(
            "note:{start}:{pitch}:{duration}",
            start = object.get("startTick").and_then(Value::as_u64).unwrap_or(0),
            pitch = object.get("pitch").and_then(Value::as_u64).unwrap_or(0),
            duration = object
                .get("durationTicks")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        )),
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                Fault::domain(ErrorCode::InvalidParameterRange, "`note.id` 必须是字符串")
            })?;
            EntityId::from_str(text).map_err(|error| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`note.id` 不是合法 ULID: {error}"),
                )
            })?
        }
    };
    let start_tick = read_u64(object, "startTick")?;
    let pitch = read_range(object, "pitch", 0, 127)?;
    let duration_ticks = read_u64(object, "durationTicks")?;
    let velocity = match object.get("velocity") {
        None => yeban_model::music::DEFAULT_VELOCITY,
        Some(_) => read_range(object, "velocity", 0, 127)?,
    };
    let mut note = MidiNote::new(id, start_tick, pitch, duration_ticks);
    note.velocity = velocity;
    note.probability = read_probability(object)?;
    note.ratchet = read_ratchet(object)?;
    note.micro_timing_ticks = read_micro_timing(object)?;
    note.validate()
        .map_err(|error| from_model("音符校验", &error))?;
    Ok(note)
}

/// 拒绝 `note` 对象里 [`NOTE_FIELDS`] 之外的键。
///
/// 键序是确定性的（`serde_json::Map` 在本 crate 的 feature 集合下是 `BTreeMap`），
/// 因此同一个非法载荷每次报的是**同一个** `field` —— 判据可以逐字钉住它。
///
/// # Errors
///
/// 出现未知键 ⇒ `INVALID_PARAMETER_RANGE`，`data` 带 `field`（第一个未知键）、
/// `supportedNoteFields`（[`NOTE_FIELDS`]）与 `hint`。
fn reject_unknown_note_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let Some(unknown) = object
        .keys()
        .find(|key| !NOTE_FIELDS.contains(&key.as_str()))
    else {
        return Ok(());
    };
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!("`note` 不接受字段 `{unknown}`（不是可选项缺失, 而是拼写/不支持）"),
        serde_json::json!({
            "field": unknown,
            "supportedNoteFields": NOTE_FIELDS,
            // 模型里有、但工具面**还没有**通路的四个表现力字段: 说出来, 不要吞掉。
            "unsupportedNoteFields": ["slide", "pitchBendCurve", "syllable", "phonemes"],
            "hint": "未知键不静默忽略: 去掉它, 或改用 supportedNoteFields 里的字段",
        }),
    ))
}

/// 读可选的 `note.ratchet`（缺省 = `None` = 等价于 1）。
///
/// 取值的**权威**判定在模型层（[`MidiNote::validate`] 的 `RATCHET_MIN..=RATCHET_MAX`）；
/// 这里额外拦一次同类区间，好让越界带上 `field` / `value` / `min` / `max` 的 `data`
/// 载荷（与 `pitch` / `velocity` / `probability` 的既有口径一致），并把"不是整数"
/// 这类 JSON 形状错误与区间错误分成两个契约码。
fn read_ratchet(object: &Map<String, Value>) -> Result<Option<u8>, Fault> {
    let Some(value) = object.get(RATCHET_FIELD) else {
        return Ok(None);
    };
    let number = value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{RATCHET_FIELD}` 必须是整数, 实际收到 {value}"),
        )
    })?;
    if !(i64::from(RATCHET_MIN)..=i64::from(RATCHET_MAX)).contains(&number) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{RATCHET_FIELD}` 越界: {number} 不在 {RATCHET_MIN}..={RATCHET_MAX}"),
            serde_json::json!({
                "field": RATCHET_FIELD,
                "value": number,
                "min": RATCHET_MIN,
                "max": RATCHET_MAX,
            }),
        ));
    }
    Ok(Some(u8::try_from(number).unwrap_or(RATCHET_MAX)))
}

/// 读可选的 `note.microTimingTicks`（缺省 = `None` = 等价于 0）。
///
/// 与 [`read_ratchet`] 同口径：区间 `-MICRO_TIMING_MAX_ABS..=MICRO_TIMING_MAX_ABS`
/// 的权威判定在模型层，这里补 `field` / `value` / `min` / `max` 的 `data` 并区分
/// 形状错误与区间错误。
fn read_micro_timing(object: &Map<String, Value>) -> Result<Option<i16>, Fault> {
    let Some(value) = object.get(MICRO_TIMING_FIELD) else {
        return Ok(None);
    };
    let number = value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{MICRO_TIMING_FIELD}` 必须是整数, 实际收到 {value}"),
        )
    })?;
    let bound = i64::from(MICRO_TIMING_MAX_ABS);
    if !(-bound..=bound).contains(&number) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!(
                "`{MICRO_TIMING_FIELD}` 越界: {number} 不在 {}..={}",
                -bound, bound
            ),
            serde_json::json!({
                "field": MICRO_TIMING_FIELD,
                "value": number,
                "min": -bound,
                "max": bound,
            }),
        ));
    }
    Ok(Some(i16::try_from(number).unwrap_or(MICRO_TIMING_MAX_ABS)))
}

/// 读可选的 `note.probability`（缺省 = `None` = 必然触发）。
///
/// 取值的**权威**判定在模型层（[`MidiNote::validate`] 的 `0.0..=1.0` 与有限性）；
/// 这里额外拦一次同类区间，好让越界带上 `field` / `value` / `min` / `max` 的 `data`
/// 载荷（与 `pitch` / `velocity` 的既有口径一致），并把"不是数字"这类 JSON 形状
/// 错误与区间错误分成两个契约码。
fn read_probability(object: &Map<String, Value>) -> Result<Option<f32>, Fault> {
    let Some(value) = object.get(PROBABILITY_FIELD) else {
        return Ok(None);
    };
    let raw = value.as_f64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PROBABILITY_FIELD}` 必须是数字, 实际收到 {value}"),
        )
    })?;
    if raw.is_nan() || raw < 0.0 || raw > 1.0 {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{PROBABILITY_FIELD}` 越界: {raw} 不在 0.0..=1.0"),
            serde_json::json!({
                "field": PROBABILITY_FIELD,
                "value": raw,
                "min": 0.0,
                "max": 1.0,
            }),
        ));
    }
    // `MidiNote::probability` 的类型就是 `f32`，所以这一步的 f64 → f32 舍入是**模型
    // 类型本身**要求的（不是本层多加的一次精度损失）：JSON 数字先按 f64 读出、判完区间
    // 再落到 f32，舍入是 IEEE 最近偶数（确定性），存进工程的就是判定用的那个值。
    #[allow(clippy::cast_possible_truncation)]
    Ok(Some(raw as f32))
}

/// 把解析过的操作编译成 [`Op`]（**读文档**补齐撤销载荷，但绝不改文档）。
///
/// `Delete` / `Move` / `Velocity` 需要当前音符状态：`DeleteNote` 自带
/// `previous_note`，`MoveNote` 与 `ModifyNoteVelocity` 的前置条件会核对
/// "音符此刻确实存在且内容一致"。
///
/// # Errors
///
/// - 片段不存在 / 不是 MIDI 片段 → `CLIP_NOT_FOUND`；
/// - 音符不存在 → `ENTITY_NOT_FOUND`；
/// - 模型层校验失败 → [`super::error::code_for_model`] 给出的契约码。
pub fn compile(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    ops: &[NoteOp],
) -> Result<Vec<Op>, Fault> {
    project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    let entry = project
        .clip_pool
        .get(clip_id)
        .ok_or_else(|| Fault::domain(ErrorCode::ClipNotFound, format!("片段不存在: {clip_id}")))?;
    if entry.content.notes().is_none() {
        return Err(Fault::domain(
            ErrorCode::ClipNotFound,
            format!("片段 {clip_id} 不是 MIDI 片段, 没有音符集合"),
        ));
    }

    let mut compiled = Vec::with_capacity(ops.len());
    for op in ops {
        compiled.push(match op {
            NoteOp::Add { note } => Op::AddNote {
                track_id: *track_id,
                clip_id: *clip_id,
                note: (**note).clone(),
            },
            NoteOp::Delete { note_id } => {
                let previous_note = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?
                    .clone();
                Op::DeleteNote {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    previous_note,
                }
            }
            NoteOp::Move {
                note_id,
                delta_tick,
                delta_pitch,
            } => {
                let current = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?;
                // 音域与时间轴越界在**这里**就报 OUT_OF_RANGE（§7.2 给本工具声明的码），
                // 而不是让模型层的 OpStateMismatch 冒泡成 CONFLICT。
                let shifted_pitch = i16::from(current.pitch) + i16::from(*delta_pitch);
                if !(0..=127).contains(&shifted_pitch) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::OutOfRange,
                        format!(
                            "平移后音高越界: {} + {} = {shifted_pitch}",
                            current.pitch, delta_pitch
                        ),
                        serde_json::json!({ "noteId": note_id.to_canonical_string(), "pitch": current.pitch, "deltaPitch": delta_pitch }),
                    ));
                }
                let shifted_tick = i128::from(current.start_tick) + i128::from(*delta_tick);
                if !(0..=i128::from(u64::MAX)).contains(&shifted_tick) {
                    return Err(Fault::domain_with_data(
                        ErrorCode::OutOfRange,
                        format!(
                            "平移后 tick 越界: {} + {} = {shifted_tick}",
                            current.start_tick, delta_tick
                        ),
                        serde_json::json!({ "noteId": note_id.to_canonical_string(), "startTick": current.start_tick, "deltaTick": delta_tick }),
                    ));
                }
                Op::MoveNote {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    delta_tick: *delta_tick,
                    delta_pitch: *delta_pitch,
                }
            }
            NoteOp::Velocity { note_id, velocity } => {
                let current = project
                    .note(clip_id, note_id)
                    .map_err(|error| from_model("音符查找", &error))?;
                Op::ModifyNoteVelocity {
                    track_id: *track_id,
                    clip_id: *clip_id,
                    note_id: *note_id,
                    old_vel: current.velocity,
                    new_vel: *velocity,
                }
            }
        });
    }
    Ok(compiled)
}

/// **材料创建**形态的编译（`arguments.create: true`）：把一组 `add` 折成**一条**
/// [`Op::AddClip`]。
///
/// 与 [`compile`] 的分工：`compile` 改**已存在**的片段（每条 `NoteOp` 一条 `Op`），
/// 本函数建**新**片段（`ops` 全部折进 `AddClip` 的初始内容，因此产物恰好一条 `Op`）。
/// 两者共用同一个 `NoteOp` 解析器与同一个发声数上限常量。
///
/// ⚠ 本函数**不摆放**：新片段只在 `clip_pool` 里。渲染与 `yeban_export_midi` 只遍历
/// `track.clips`，因此未摆放的片段不出声 —— 这是刻意的（"配器材料"只要求池里有材料），
/// 并且如实写在响应 `willCreate.clipPoolEntries` 里，不假装它已经上了时间轴。
///
/// # Errors
///
/// - 音轨不存在 → `TRACK_NOT_FOUND`；
/// - 池里已有该 `clipId` → `CONFLICT`（`data.reason = "clipAlreadyExists"`）；
/// - `ops` 里出现 `add` 之外的操作 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "createRequiresAddOps"`）；
/// - 两个 `add` 用同一个音符身份 → `INVALID_PARAMETER_RANGE`
///   （`data.reason = "duplicateNoteId"`）。
pub fn compile_create(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    clip_name: &str,
    ops: &[NoteOp],
) -> Result<Vec<Op>, Fault> {
    // 与 `compile` 同口径的入口校验：`trackId` 必须是工程里真实存在的音轨。
    project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    if project.clip_pool.contains_key(clip_id) {
        return Err(Fault::domain_with_data(
            ErrorCode::Conflict,
            format!("片段池里已经有身份 {clip_id}, `create: true` 不覆盖既有片段"),
            serde_json::json!({
                "clipId": clip_id.to_canonical_string(),
                "reason": "clipAlreadyExists",
                "hint": "把 `create` 去掉就是一次普通编辑; 要新建请换一个 `clipId`",
            }),
        ));
    }
    let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
    for op in ops {
        match op {
            NoteOp::Add { note } => {
                if notes.insert(note.id, (**note).clone()).is_some() {
                    return Err(Fault::domain_with_data(
                        ErrorCode::InvalidParameterRange,
                        format!("两个 `add` 用了同一个音符身份 {}", note.id),
                        serde_json::json!({
                            "reason": "duplicateNoteId",
                            "noteId": note.id.to_canonical_string(),
                        }),
                    ));
                }
            }
            other => {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!(
                        "`create: true` 时 `ops` 只允许 `add` (新片段里还没有音符可以被 \
                         `delete`/`move`/`velocity` 指向), 实际收到 `{}`",
                        other.kind_name()
                    ),
                    serde_json::json!({
                        "reason": "createRequiresAddOps",
                        "supportedKindsWhenCreating": ["add"],
                        "received": other.kind_name(),
                    }),
                ));
            }
        }
    }
    // `parse_ops` 已经拒绝空数组, 且上面只放行 `add` ⇒ `notes` 至少一条。
    // 仍显式断言: "空 MIDI 片段"不是可用材料 (`section_build` 的 `usable_materials`),
    // 建出它等于把 needs-8 的死角换一个地方。
    if notes.is_empty() {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`create: true` 至少需要一个 `add` 音符: 空片段不是可用材料",
        ));
    }
    Ok(vec![Op::AddClip {
        clip: ClipPoolEntry {
            id: *clip_id,
            name: clip_name.to_owned(),
            content: ClipContent::Midi { notes },
        },
    }])
}

/// 一次**摆放**的确定性标签：`(片段身份, 音轨身份, 起始 tick)`。
///
/// 与 `extension_pure::placement_label`（音频导入那一侧）**分开**一个前缀：
/// 两条路径的片段不是同一类材料，标签也不该长得一样（同一份 `deterministic_id`
/// 输入不同的标签 ⇒ 不同的摆放身份）。
///
/// 时值**不在**标签里：同一片段、同一音轨、同一起点是**同一次摆放**，改时值是
/// "改这一次摆放的长度"，不是凭空多出第二条摆放（重复提交同一起点但不同时值由
/// [`parse_placement`] 判成 `CONFLICT`，与"同身份不同内容的片段"同一口径）。
#[must_use]
pub fn placement_label(clip_id: &str, track_id: &str, start_tick: u64) -> String {
    format!("midi-placement:{clip_id}:{track_id}:{start_tick}")
}

/// 解析 `arguments.placement`，按 `placement.kind` 分发到三种**摆放编辑**。
///
/// | `kind` | 载荷 | 编译成 |
/// | :--- | :--- | :--- |
/// | 缺省 / `add` | `startTick?` / `durationTicks?` / `placementId?` / `muted?` | [`Op::AddClipPlacement`] |
/// | `move` | `placementId`（必填）+ `startTick`（必填 = **新**起点） | [`Op::MoveClipPlacement`] |
/// | `remove` | `placementId`（必填） | [`Op::RemoveClipPlacement`] |
///
/// 为什么有 `move` / `remove`：`f1098e2` 让 `create: true` 把材料**建**进池子，
/// `ff23302` 让 `add` 把材料**摆**上时间轴；到此为止工具面能**加**一条摆放，
/// 却**没有任何**工具能挪动或取走它 —— `Op::MoveClipPlacement` 与
/// `Op::RemoveClipPlacement` 在模型层早已实现（各自带自包含撤销载荷），
/// 渲染器也**真的**按 `track.clips` 出片，因此那两个能力在 17 个工具的面上
/// **不可达**：摆错位置只剩"整次调用撤销"一条路，而撤到那一步之前的编辑会一起丢。
/// 同族缺口的先例是 `probability`（`314d2fc`）与 `ratchet`/`microTimingTicks`
/// （`791a571`）—— 都是"模型/渲染器已经做到、工具面够不着"。
///
/// `placementId` 缺省派生**只**属于 `add`：派生一条新身份不可能命中一条已有摆放，
/// 因此 `move` / `remove` 缺它就是 [`require_placement_id`] 的响亮失败。
///
/// # Errors
///
/// - `placement` 不是对象 ⇒ `INVALID_PARAMETER_RANGE`；
/// - `kind` 不是字符串 / 不在 [`PLACEMENT_KINDS`] 里 ⇒ `INVALID_PARAMETER_RANGE`
///   （`reason = "unknownPlacementKind"`，`data` 列出支持集合）；
/// - 键不在本形态的词表里 ⇒ `INVALID_PARAMETER_RANGE`（见 [`reject_placement_fields`]）；
/// - `add` 形态的一切失败 ⇒ 见 [`parse_placement`]；
/// - `move` / `remove` 找不到那条摆放 ⇒ `ENTITY_NOT_FOUND`（`placementNotFound`）；
/// - `clipId` 与文档里那条摆放引用不一致 ⇒ `INVALID_PARAMETER_RANGE`
///   （`placementClipMismatch`）；
/// - `move` 的目标起点等于现值 ⇒ `CONFLICT`（`placementAlreadyAtStartTick`）——
///   没有可提交的改动，不制造一条空提案。
pub fn parse_placement_edit(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    arguments: &Map<String, Value>,
) -> Result<Option<PlacementEdit>, Fault> {
    let Some(raw) = arguments.get(PLACEMENT_FIELD) else {
        return Ok(None);
    };
    let object = raw.as_object().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}` 必须是对象, 实际收到 {raw}"),
        )
    })?;
    let kind = match object.get(PLACEMENT_KIND_FIELD) {
        None => PLACEMENT_KIND_ADD,
        Some(value) => value.as_str().ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "`{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}` 必须是字符串, 实际收到 {value}"
                ),
                serde_json::json!({
                    "field": format!("{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}"),
                    "supportedPlacementKinds": PLACEMENT_KINDS,
                }),
            )
        })?,
    };
    match kind {
        PLACEMENT_KIND_ADD => {
            Ok(parse_placement(project, track_id, clip_id, arguments)?.map(PlacementEdit::Add))
        }
        PLACEMENT_KIND_MOVE => {
            reject_placement_fields(object, PLACEMENT_MOVE_FIELDS, PLACEMENT_KIND_MOVE)?;
            let placement_id = require_placement_id(object, PLACEMENT_KIND_MOVE)?;
            let Some(new_start_tick) = read_optional_u64(object, "startTick")? else {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!(
                        "`{PLACEMENT_FIELD}` 的 `{PLACEMENT_KIND_MOVE}` 形态必须给出 `startTick` \
                         (它是**目标**起点)"
                    ),
                    serde_json::json!({
                        "reason": "moveRequiresStartTick",
                        "field": format!("{PLACEMENT_FIELD}.startTick"),
                        "placementKind": PLACEMENT_KIND_MOVE,
                    }),
                ));
            };
            let found = existing_placement(
                project,
                track_id,
                &placement_id,
                clip_id,
                PLACEMENT_KIND_MOVE,
            )?;
            if found.start_tick == new_start_tick {
                return Err(Fault::domain_with_data(
                    ErrorCode::Conflict,
                    format!("摆放 {placement_id} 的起点已经是 {new_start_tick}, 没有可提交的改动"),
                    serde_json::json!({
                        "reason": "placementAlreadyAtStartTick",
                        "placementKind": PLACEMENT_KIND_MOVE,
                        "trackId": track_id.to_canonical_string(),
                        "placementId": placement_id.to_canonical_string(),
                        "startTick": new_start_tick,
                        "hint": "幂等重放请用 `idempotencyKey`; 要挪到别处就给不同的 `startTick`",
                    }),
                ));
            }
            Ok(Some(PlacementEdit::Move {
                placement_id,
                previous_start_tick: found.start_tick,
                new_start_tick,
            }))
        }
        PLACEMENT_KIND_REMOVE => {
            reject_placement_fields(object, PLACEMENT_REMOVE_FIELDS, PLACEMENT_KIND_REMOVE)?;
            let placement_id = require_placement_id(object, PLACEMENT_KIND_REMOVE)?;
            let found = existing_placement(
                project,
                track_id,
                &placement_id,
                clip_id,
                PLACEMENT_KIND_REMOVE,
            )?;
            Ok(Some(PlacementEdit::Remove {
                placement_id,
                previous_placement: found,
            }))
        }
        other => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}` 不支持 `{other}`"),
            serde_json::json!({
                "reason": "unknownPlacementKind",
                "field": format!("{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}"),
                "value": other,
                "supportedPlacementKinds": PLACEMENT_KINDS,
                "hint": "缺省 `kind` 等价于 `add`; 未知形态响亮拒绝, 不静默按 add 处理",
            }),
        )),
    }
}

/// 解析 `placement` 的 **`add` 形态**（缺省 = 对象不在场 ⇒ `None` ⇒ 不摆放 =
/// 逐字节等于旧行为）。
///
/// ```json
/// {"placement": {"startTick": 0, "durationTicks": 3840,
///                "placementId": "<可选 26 字符 ULID>", "muted": false}}
/// ```
///
/// 四个内容键全部**可选**：`startTick` 缺省 0；`durationTicks` 缺省由片段内容推导；
/// `placementId` 缺省由 [`placement_label`] 确定性派生；`muted` 缺省 `false`。
/// `kind` 可写可不写；写了必须是 [`PLACEMENT_KIND_ADD`]（其余形态由
/// [`parse_placement_edit`] 分发，不走本函数）。
///
/// 三条刻意设成**响亮失败**的口径（绝不静默降级）：
///
/// | 情形 | 结果 |
/// | :--- | :--- |
/// | `placement` 对象里有 [`PLACEMENT_ADD_FIELDS`] 之外的键 | `INVALID_PARAMETER_RANGE`（`reason = "unknownPlacementField"`，列出支持集合） |
/// | `durationTicks` 缺省、而片段**推不出**长度（非 MIDI 片段 / 空 MIDI 片段） | `INVALID_PARAMETER_RANGE`（`reason = "durationNotDerivable"`）—— 不猜一个假长度 |
/// | 目标音轨上**已经有**这个摆放身份 | `CONFLICT`（逐字段相同 ⇒ `reason = "placementAlreadyExists"`；内容不同 ⇒ `reason = "placementIdConflict"`） |
///
/// # Errors
///
/// - `placement` 不是对象 / 键类型不对 / `placementId` 不是 ULID → `INVALID_PARAMETER_RANGE`；
/// - 音轨不存在 → `TRACK_NOT_FOUND`；片段不存在 → `CLIP_NOT_FOUND`；
/// - `durationTicks == 0` → `INVALID_PARAMETER_RANGE`（模型拒绝零时值的摆放）；
/// - 摆放身份已被占用 → `CONFLICT`；
/// - 模型层 [`ClipPlacement::validate`] 失败 → [`from_model`] 给出的契约码。
pub fn parse_placement(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    arguments: &Map<String, Value>,
) -> Result<Option<ClipPlacement>, Fault> {
    let Some(raw) = arguments.get(PLACEMENT_FIELD) else {
        return Ok(None);
    };
    let object = raw.as_object().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}` 必须是对象, 实际收到 {raw}"),
        )
    })?;
    reject_placement_fields(object, PLACEMENT_ADD_FIELDS, PLACEMENT_KIND_ADD)?;
    // 目标音轨与片段都必须**真的存在**：摆放是"把已有材料放到已有轨道上"，
    // 两个端点缺一个都不是一次摆放（`compile` 的编辑路径有同一对前置条件）。
    //
    // ⚠ 音轨句柄**只查一次**并留到函数末尾（摆放身份的占用判定要读它的 `clips`）：
    // 同一处检查写两遍时，删掉前一处**没有任何判据会变红**（实测：注入后全绿）
    // —— 那种守卫是"看起来在守"的装饰，不留。
    let track = project
        .track(track_id)
        .map_err(|error| from_model("摆放的目标音轨", &error))?;
    let entry = project
        .clip_pool
        .get(clip_id)
        .ok_or_else(|| Fault::domain(ErrorCode::ClipNotFound, format!("片段不存在: {clip_id}")))?;

    let start_tick = read_optional_u64(object, "startTick")?.unwrap_or(0);
    let duration_ticks = match read_optional_u64(object, "durationTicks")? {
        Some(0) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                "`placement.durationTicks` 必须 >= 1 (模型层拒绝零时值的摆放)",
                serde_json::json!({
                    "field": format!("{PLACEMENT_FIELD}.durationTicks"),
                    "value": 0,
                }),
            ));
        }
        Some(value) => value,
        // 缺省 = 片段内容自己的长度（MIDI 片段 = 最后一个音符的结束 tick）。
        // **推不出就不猜**：非 MIDI 片段（音频片段由 `yeban_import_audio` 的
        // `durationTicks` 承载，那里的时值来自素材帧数）与空 MIDI 片段都必须显式给。
        None => match entry.content.notes().and_then(|notes| {
            notes
                .values()
                .map(|note| note.start_tick.saturating_add(note.duration_ticks))
                .max()
        }) {
            Some(extent) if extent > 0 => extent,
            _ => {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    "这个片段推不出长度 (非 MIDI 片段或空 MIDI 片段) ⇒ 必须显式给出 \
                     `placement.durationTicks`",
                    serde_json::json!({
                        "reason": "durationNotDerivable",
                        "field": format!("{PLACEMENT_FIELD}.durationTicks"),
                        "clipId": clip_id.to_canonical_string(),
                        "isMidi": entry.content.notes().is_some(),
                    }),
                ));
            }
        },
    };
    let placement_id = match object.get("placementId") {
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "`placement.placementId` 必须是 26 字符 ULID 字符串",
                )
            })?;
            EntityId::from_str(text).map_err(|error| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`placement.placementId` 不是合法 ULID: {error}"),
                )
            })?
        }
        None => deterministic_id(&placement_label(
            &clip_id.to_canonical_string(),
            &track_id.to_canonical_string(),
            start_tick,
        )),
    };
    let placement = ClipPlacement {
        id: placement_id,
        clip_id: *clip_id,
        start_tick,
        duration_ticks,
        // 循环配置是模型的**必需**子结构 [ADR-0001 D43]：缺省 = 关闭（不重复）。
        // "循环重复"不在渲染的已支持面里（响应 `unsupported: clipLoopRepetition`），
        // 因此这里刻意不暴露 `loopEnabled` —— 那会给出一个渲染不了的旋钮
        // （与 `yeban_import_audio` 同一个理由）。
        loop_config: LoopConfig::default(),
        muted: read_optional_bool(object, "muted")?.unwrap_or(false),
    };
    placement
        .validate()
        .map_err(|error| from_model("摆放校验", &error))?;
    // 摆放身份在**目标音轨**的 `clips` 里必须还没有被占用：`AddClipPlacement`
    // 的前置条件拒绝重复身份，在这里先判一次才能给出 `field`/`reason` 的结构化 `data`
    // （与 `compile_create` 的 `clipAlreadyExists` 同一口径）。
    match track.clips.get(&placement.id) {
        None => {}
        Some(existing) if *existing == placement => {
            return Err(Fault::domain_with_data(
                ErrorCode::Conflict,
                format!(
                    "音轨 {track_id} 上已经有这条摆放 {} (逐字段相同), 没有可提交的改动",
                    placement.id
                ),
                serde_json::json!({
                    "reason": "placementAlreadyExists",
                    "trackId": track_id.to_canonical_string(),
                    "placementId": placement.id.to_canonical_string(),
                    "hint": "幂等重放请用 `idempotencyKey`; 要挪位置就换 `placement.startTick`",
                }),
            ));
        }
        Some(existing) => {
            return Err(Fault::domain_with_data(
                ErrorCode::Conflict,
                format!(
                    "音轨 {track_id} 上已有摆放身份 {}, 但内容不同",
                    placement.id
                ),
                serde_json::json!({
                    "reason": "placementIdConflict",
                    "trackId": track_id.to_canonical_string(),
                    "placementId": placement.id.to_canonical_string(),
                    "existing": serde_json::to_value(existing).unwrap_or(Value::Null),
                    "requested": serde_json::to_value(placement).unwrap_or(Value::Null),
                    "hint": "换一个 `placement.placementId`/`startTick`, 或给出逐字段相同的载荷",
                }),
            ));
        }
    }
    Ok(Some(placement))
}

/// 拒绝 `placement` 对象里 `allowed` 之外的键（与 [`reject_unknown_note_fields`]
/// 同一口径：拼错/不支持的键一律响亮拒绝，绝不静默丢弃）。
///
/// 两种"不在 `allowed` 里"被**分开**报（`kind` 由 `kind` 参数如实带出）：
///
/// | 情形 | `reason` |
/// | :--- | :--- |
/// | 键不在**任何**形态的词表里（拼错 / 根本不支持，例如 `loopEnabled`） | `unknownPlacementField` |
/// | 键在别的形态里合法、但**本**形态不适用（例如 `move` 里的 `muted`） | `placementFieldNotApplicable` |
///
/// 分开的理由：把"这个形态改不了静音"报成"静音不是一个键"会让调用方去猜一个不存在的
/// 替代写法。
///
/// 键序是确定性的（`serde_json::Map` 在本 crate 的 feature 集合下是 `BTreeMap`），
/// 因此同一个非法载荷每次报的是**同一个** `field` —— 判据可以逐字钉住它。
///
/// # Errors
///
/// 出现不属于 `allowed` 的键 ⇒ `INVALID_PARAMETER_RANGE`，`data` 带 `field`
/// （第一个这样的键）、`reason`、`placementKind`、`supportedPlacementFields`
/// （本形态的 `allowed`）与 `hint`。
fn reject_placement_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    kind: &str,
) -> Result<(), Fault> {
    let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) else {
        return Ok(());
    };
    let known_in_another_shape =
        !allowed.contains(&unknown.as_str()) && PLACEMENT_ADD_FIELDS.contains(&unknown.as_str());
    let reason = if known_in_another_shape {
        "placementFieldNotApplicable"
    } else {
        "unknownPlacementField"
    };
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "`{PLACEMENT_FIELD}` 的 `{kind}` 形态不接受字段 `{unknown}` \
             (不是可选项缺失, 而是拼写/不支持/本形态不适用)"
        ),
        serde_json::json!({
            "reason": reason,
            "field": unknown,
            "placementKind": kind,
            "supportedPlacementFields": allowed,
            "hint": "未知键不静默忽略: 去掉它, 或改用 supportedPlacementFields 里的字段",
        }),
    ))
}

/// 读 `placement.placementId`（**必填** ULID；缺失 / 非字符串 / 不是 ULID 都拒绝）。
///
/// `move` / `remove` 两个形态都要指名一条**已经存在**的摆放，因此身份是必填的 ——
/// 缺省派生（[`placement_label`]）**只**属于 `add` 形态：派生一条新身份不可能命中
/// 一条已有摆放。
///
/// # Errors
///
/// 缺失 ⇒ `INVALID_PARAMETER_RANGE`（`reason = "{kind}RequiresPlacementId"`）；
/// 形状不对 ⇒ `INVALID_PARAMETER_RANGE`。
fn require_placement_id(object: &Map<String, Value>, kind: &str) -> Result<EntityId, Fault> {
    let Some(value) = object.get("placementId") else {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}` 的 `{kind}` 形态必须给出 `placementId`"),
            serde_json::json!({
                "reason": format!("{kind}RequiresPlacementId"),
                "field": format!("{PLACEMENT_FIELD}.placementId"),
                "placementKind": kind,
                "hint": "`placementId` 缺省派生只属于 `add` 形态; `move`/`remove` 必须指名已有摆放",
            }),
        ));
    };
    let text = value.as_str().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`placement.placementId` 必须是 26 字符 ULID 字符串",
        )
    })?;
    EntityId::from_str(text).map_err(|error| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`placement.placementId` 不是合法 ULID: {error}"),
        )
    })
}

/// 在目标音轨上找出 `placement_id` 这条**已经存在**的摆放，并核对调用方报的 `clip_id`。
///
/// `clipId` 是 `yeban_edit_notes` 的**必填**实参（`§7.2` 的参数表），因此 `move` /
/// `remove` 也要求调用方把它写出来；它与文档里那条摆放的 `clip_id` **必须一致** ——
/// 不一致说明调用方手上的摆放和文档里的不是同一条，这时**不**猜、也**不**静默改用文档
/// 里的那一条，而是响亮失败（`reason = "placementClipMismatch"`）。
///
/// # Errors
///
/// - 音轨不存在 ⇒ `TRACK_NOT_FOUND`；
/// - 该音轨上没有这条摆放 ⇒ `ENTITY_NOT_FOUND`（`reason = "placementNotFound"`）；
/// - `clipId` 与文档不一致 ⇒ `INVALID_PARAMETER_RANGE`（`reason = "placementClipMismatch"`）。
fn existing_placement(
    project: &YebanProjectV1,
    track_id: &EntityId,
    placement_id: &EntityId,
    clip_id: &EntityId,
    kind: &str,
) -> Result<ClipPlacement, Fault> {
    let track = project
        .track(track_id)
        .map_err(|error| from_model("摆放编辑的目标音轨", &error))?;
    let Some(found) = track.clips.get(placement_id) else {
        return Err(Fault::domain_with_data(
            ErrorCode::EntityNotFound,
            format!("音轨 {track_id} 上没有摆放 {placement_id}"),
            serde_json::json!({
                "reason": "placementNotFound",
                "placementKind": kind,
                "trackId": track_id.to_canonical_string(),
                "placementId": placement_id.to_canonical_string(),
                "placementCount": track.clips.len(),
                "hint": "`add` 形态才是新建; `move`/`remove` 只能作用在**已有**的摆放上",
            }),
        ));
    };
    if found.clip_id != *clip_id {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "摆放 {placement_id} 引用的是片段 {}, 不是调用方给出的 `clipId` {clip_id}",
                found.clip_id
            ),
            serde_json::json!({
                "reason": "placementClipMismatch",
                "placementKind": kind,
                "trackId": track_id.to_canonical_string(),
                "placementId": placement_id.to_canonical_string(),
                "clipId": clip_id.to_canonical_string(),
                "placementClipId": found.clip_id.to_canonical_string(),
            }),
        ));
    }
    Ok(*found)
}

/// 读一个**可选**的非负整数键（缺省 = `None`；负数、非整数、溢出都拒绝）。
///
/// 与 `import_audio` 的 `parse_optional_u64` 同一条口径（那里读的是顶层实参，名字不带
/// `placement.` 前缀，因此错误的措辞不同）。它只做 **JSON 形状**读取，不携带领域语义 ——
/// "时值必须非零""音轨必须存在"这类裁决分别在 [`parse_placement`] 与模型层。
fn read_optional_u64(object: &Map<String, Value>, field: &str) -> Result<Option<u64>, Fault> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}.{field}` 必须是非负整数, 实际收到 {value}"),
        )
    })
}

/// 读一个**可选**的布尔键（缺省 = `None`；非布尔拒绝）。
fn read_optional_bool(object: &Map<String, Value>, field: &str) -> Result<Option<bool>, Fault> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    value.as_bool().map(Some).ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{PLACEMENT_FIELD}.{field}` 必须是布尔值, 实际收到 {value}"),
        )
    })
}

/// 峰值同时发声数（在 `[start, start + duration)` 上的最大重叠）。
#[must_use]
pub fn peak_polyphony(notes: impl IntoIterator<Item = (u64, u64)>) -> usize {
    let mut events: Vec<(u64, i64)> = Vec::new();
    for (start, duration) in notes {
        events.push((start, 1));
        events.push((start.saturating_add(duration), -1));
    }
    // 同一 tick 上"结束"排在"开始"之前: 首尾相接的两个音符不算重叠。
    events.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    let mut current = 0_i64;
    let mut peak = 0_i64;
    for (_, delta) in events {
        current += delta;
        peak = peak.max(current);
    }
    usize::try_from(peak).unwrap_or(usize::MAX)
}

/// 检查某片段在施加 `ops` **之后**的发声数（在克隆体上模拟，不改原文档）。
///
/// 这条路径**同时覆盖**两个形态：`ops` 是编辑操作时它读的是既有片段的新状态；
/// `ops` 是 [`compile_create`] 的那一条 `Op::AddClip` 时，模拟里新片段已经存在
/// ⇒ 读到的就是**新片段**的峰值。因此材料创建不需要第二份发声数检查。
///
/// # Errors
///
/// - 模拟时模型层失败 → [`from_model`] 给出的契约码；
/// - 超过 [`MAX_POLYPHONY`] → `OUT_OF_RANGE`（带 `peak` / `limit` / `clipId`）。
pub fn check_polyphony(
    project: &YebanProjectV1,
    clip_id: &EntityId,
    ops: &[Op],
) -> Result<usize, Fault> {
    let mut simulated = project.clone();
    for op in ops {
        op.apply(&mut simulated)
            .map_err(|error| from_model("音符操作模拟", &error))?;
    }
    let notes = simulated
        .clip_pool
        .get(clip_id)
        .and_then(|entry| entry.content.notes())
        .map(|notes| {
            notes
                .values()
                .map(|note| (note.start_tick, note.duration_ticks))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let peak = peak_polyphony(notes);
    if peak > MAX_POLYPHONY {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("片段 {clip_id} 的发声数峰值 {peak} 超过上限 {MAX_POLYPHONY}"),
            serde_json::json!({ "clipId": clip_id.to_canonical_string(), "peak": peak, "limit": MAX_POLYPHONY }),
        ));
    }
    Ok(peak)
}

/// 缺字段的统一错误。
fn missing(field: &str, expected: &str) -> Fault {
    Fault::domain(
        ErrorCode::InvalidParameterRange,
        format!("缺少 `{field}`（期望 {expected}）"),
    )
}

/// 读 `0..=max` 的整数。
fn read_range(object: &Map<String, Value>, field: &str, min: u8, max: u8) -> Result<u8, Fault> {
    let value = object.get(field).ok_or_else(|| missing(field, "整数"))?;
    let number = value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是整数, 实际收到 {value}"),
        )
    })?;
    if !(i64::from(min)..=i64::from(max)).contains(&number) {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{field}` 越界: {number} 不在 {min}..={max}"),
            serde_json::json!({ "field": field, "value": number, "min": min, "max": max }),
        ));
    }
    Ok(u8::try_from(number).unwrap_or(max))
}

/// 读非负整数。
fn read_u64(object: &Map<String, Value>, field: &str) -> Result<u64, Fault> {
    let value = object
        .get(field)
        .ok_or_else(|| missing(field, "非负整数"))?;
    value.as_u64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是非负整数, 实际收到 {value}"),
        )
    })
}

/// 读 `i64`。
fn read_i64(object: &Map<String, Value>, field: &str) -> Result<i64, Fault> {
    let value = object.get(field).ok_or_else(|| missing(field, "整数"))?;
    value.as_i64().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是整数, 实际收到 {value}"),
        )
    })
}

/// 读 `i8`。
fn read_i8(object: &Map<String, Value>, field: &str) -> Result<i8, Fault> {
    let number = read_i64(object, field)?;
    i8::try_from(number).map_err(|_| {
        Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`{field}` 越界: {number} 不在 -128..=127"),
            serde_json::json!({ "field": field, "value": number }),
        )
    })
}

/// 读 `EntityId`。
fn read_id(object: &Map<String, Value>, field: &str) -> Result<EntityId, Fault> {
    let value = object
        .get(field)
        .ok_or_else(|| missing(field, "26 字符 ULID 字符串"))?;
    let text = value.as_str().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是字符串, 实际收到 {value}"),
        )
    })?;
    EntityId::from_str(text).map_err(|error| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 不是合法 ULID: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::samples::filled_project;

    fn lead_clip(project: &YebanProjectV1) -> (EntityId, EntityId) {
        let clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_some())
            .expect("样本里必须有 MIDI 片段");
        let track = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Midi)
            .expect("样本里必须有 MIDI 音轨");
        (track.id, clip.id)
    }

    #[test]
    fn parse_rejects_unknown_kind_and_bad_shapes() {
        for broken in [
            serde_json::json!([]),
            serde_json::json!([{"kind": "explode"}]),
            serde_json::json!([{"kind": "delete"}]),
            serde_json::json!([{"kind": "move", "noteId": "x", "deltaTick": 1, "deltaPitch": 0}]),
            serde_json::json!("not an array"),
        ] {
            let fault = parse_ops(&broken).expect_err("必须被拒");
            assert!(
                matches!(
                    fault.domain_code(),
                    Some(ErrorCode::InvalidParameterRange | ErrorCode::OutOfRange)
                ),
                "{broken} 得到了 {:?}",
                fault.domain_code()
            );
        }
    }

    #[test]
    fn parse_rejects_out_of_range_pitch_and_velocity() {
        let fault = parse_ops(&serde_json::json!([{
            "kind": "add",
            "note": {"startTick": 0, "pitch": 128, "durationTicks": 480}
        }]))
        .expect_err("音高越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));

        let fault = parse_ops(&serde_json::json!([{
            "kind": "velocity", "noteId": "01J8ZQ00000000000000000001", "velocity": 200
        }]))
        .expect_err("力度越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));

        let fault = parse_ops(&serde_json::json!([{
            "kind": "add",
            "note": {"startTick": 0, "pitch": 60, "durationTicks": 0}
        }]))
        .expect_err("零时值");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
    }

    #[test]
    fn compile_reads_the_undo_payload_from_the_document() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let note_id = project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("MIDI")
            .keys()
            .next()
            .copied()
            .expect("至少一个音符");
        let ops =
            compile(&project, &track_id, &clip_id, &[NoteOp::Delete { note_id }]).expect("编译");
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            Op::DeleteNote { previous_note, .. } => {
                assert_eq!(
                    previous_note,
                    project.note(&clip_id, &note_id).expect("音符"),
                    "撤销载荷必须来自当前文档"
                );
            }
            other => panic!("应当是 DeleteNote: {other:?}"),
        }
    }

    #[test]
    fn compile_reports_missing_note_as_entity_not_found() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let ghost = deterministic_id("ghost-note");
        let fault = compile(
            &project,
            &track_id,
            &clip_id,
            &[NoteOp::Velocity {
                note_id: ghost,
                velocity: 1,
            }],
        )
        .expect_err("音符不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
    }

    #[test]
    fn peak_polyphony_ignores_abutting_notes() {
        assert_eq!(peak_polyphony([] as [(u64, u64); 0]), 0);
        assert_eq!(peak_polyphony([(0, 100)]), 1);
        // 首尾相接不算重叠。
        assert_eq!(peak_polyphony([(0, 100), (100, 100)]), 1);
        // 真正重叠。
        assert_eq!(peak_polyphony([(0, 200), (100, 100)]), 2);
        assert_eq!(peak_polyphony([(0, 1000), (0, 1000), (0, 1000)]), 3);
    }

    #[test]
    fn polyphony_guard_is_reachable_and_bounded() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let mut ops = Vec::new();
        for index in 0..(MAX_POLYPHONY + 1) {
            let id = deterministic_id(&format!("flood-{index}"));
            let mut note = MidiNote::new(id, 0, 60, 960);
            note.velocity = 100;
            ops.push(NoteOp::Add {
                note: Box::new(note),
            });
        }
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        let fault = check_polyphony(&project, &clip_id, &compiled).expect_err("必须越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        // 样本片段在 tick 0 已经有一个音符, 而注入的音符都在 tick 0 ⇒ 峰值 = 注入数 + 1。
        // 因此去掉 2 个（33 - 2 + 1 = 32）刚好落在上限之内。
        let ok = compile(&project, &track_id, &clip_id, &ops[2..]).expect("编译");
        assert_eq!(
            check_polyphony(&project, &clip_id, &ok).expect("上限之内"),
            MAX_POLYPHONY
        );
    }

    // -----------------------------------------------------------------------
    // 概率触发字段（`add.note.probability`）—— 让模型能力在工具面可达
    // -----------------------------------------------------------------------

    /// 缺省不写 ⇒ `None`（= 必然触发 = 加这个字段之前的逐字节行为）。
    #[test]
    fn add_without_probability_stays_none() {
        let ops = parse_ops(&Value::Array(vec![add_json(0, 60)])).expect("解析");
        let NoteOp::Add { note } = &ops[0] else {
            panic!("必须是 add");
        };
        assert_eq!(note.probability, None, "缺省必须是不写这个字段");
    }

    /// 给了就**逐值**搬进 `MidiNote`（判定不在这一层）。
    #[test]
    fn add_carries_the_probability_verbatim() {
        for (literal, expected) in [(0.0f64, 0.0f32), (0.5, 0.5), (1.0, 1.0), (0.25, 0.25)] {
            let mut item = add_json(0, 60);
            item["note"][PROBABILITY_FIELD] = serde_json::json!(literal);
            let ops = parse_ops(&Value::Array(vec![item])).expect("解析");
            let NoteOp::Add { note } = &ops[0] else {
                panic!("必须是 add");
            };
            assert_eq!(note.probability, Some(expected), "字面值 {literal}");
        }
    }

    /// 越界 ⇒ `OUT_OF_RANGE`（带 `field` / `value` / `min` / `max`），不是静默夹紧。
    #[test]
    fn probability_out_of_range_is_out_of_range() {
        for bad in [-0.001f64, 1.001, 2.0, 1e9] {
            let mut item = add_json(0, 60);
            item["note"][PROBABILITY_FIELD] = serde_json::json!(bad);
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange), "值 {bad}");
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败");
            };
            let data = data.as_ref().expect("必须带 data");
            assert_eq!(data["field"], PROBABILITY_FIELD);
            assert_eq!(data["min"], 0.0);
            assert_eq!(data["max"], 1.0);
        }
    }

    /// 形状错（不是数字）⇒ `INVALID_PARAMETER_RANGE`，而不是被当成 0 或 1。
    #[test]
    fn probability_must_be_a_number() {
        for bad in [
            serde_json::json!("0.5"),
            serde_json::json!(true),
            serde_json::json!([0.5]),
        ] {
            let mut item = add_json(0, 60);
            item["note"][PROBABILITY_FIELD] = bad.clone();
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "值 {bad}"
            );
        }
    }

    /// 概率随 `add` 一路进 `Op::AddNote`（`create: true` 的新片段也一样），
    /// 因此它**真的**进工程、也**真的**可回退。
    #[test]
    fn probability_reaches_the_add_note_op_and_the_create_form() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let mut item = add_json(0, 72);
        item["note"][PROBABILITY_FIELD] = serde_json::json!(0.5);
        let ops = parse_ops(&Value::Array(vec![item])).expect("解析");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        match &compiled[0] {
            Op::AddNote { note, .. } => assert_eq!(note.probability, Some(0.5)),
            other => panic!("必须是 AddNote, 实际 {other:?}"),
        }
        // `create: true` 走同一个 [`parse_note`] ⇒ 新片段里的音符也带着它。
        let fresh = project_without_midi_clips();
        let seed_id = deterministic_id("probability-create");
        let created = compile_create(&fresh, &track_id, &seed_id, "Seed", &ops).expect("创建");
        let Op::AddClip { clip } = &created[0] else {
            panic!("必须是 AddClip");
        };
        let note = clip
            .content
            .notes()
            .expect("MIDI")
            .values()
            .next()
            .expect("至少一个音符");
        assert_eq!(note.probability, Some(0.5));
    }

    // -----------------------------------------------------------------------
    // 连击 / 微时序字段（`add.note.ratchet` / `add.note.microTimingTicks`）
    // —— 渲染器**已经**按模型语义实现这两个字段，工具面此前写不进去
    // -----------------------------------------------------------------------

    /// 缺省不写 ⇒ `None`（= `ratchet` 等价于 1、微时序等价于 0 = 加这两个字段之前的
    /// 逐字节行为）。
    #[test]
    fn add_without_ratchet_or_micro_timing_stays_none() {
        let ops = parse_ops(&Value::Array(vec![add_json(0, 60)])).expect("解析");
        let NoteOp::Add { note } = &ops[0] else {
            panic!("必须是 add");
        };
        assert_eq!(note.ratchet, None, "缺省必须是不写这个字段");
        assert_eq!(note.micro_timing_ticks, None, "缺省必须是不写这个字段");
    }

    /// 字段名的字面拼写被钉住（工具面的实参名是契约的一部分，改名会让既有 Agent 静默降级），
    /// 且支持集合与 `parse_note` 真正读的键**同源**（多报一个键就是假话）。
    #[test]
    fn expressive_note_field_names_are_pinned() {
        assert_eq!(RATCHET_FIELD, "ratchet");
        assert_eq!(MICRO_TIMING_FIELD, "microTimingTicks");
        for expected in [
            "id",
            "startTick",
            "pitch",
            "durationTicks",
            "velocity",
            "probability",
            "ratchet",
            "microTimingTicks",
        ] {
            assert!(
                NOTE_FIELDS.contains(&expected),
                "支持集合必须含 {expected}: {NOTE_FIELDS:?}"
            );
        }
        assert_eq!(
            NOTE_FIELDS.len(),
            8,
            "支持集合不得多报未读的键: {NOTE_FIELDS:?}"
        );
    }

    /// 给了就**逐值**搬进 `MidiNote`，并一路进 `Op::AddNote`。
    #[test]
    fn ratchet_and_micro_timing_reach_the_add_note_op() {
        for (ratchet, micro) in [(1u8, 0i16), (2, -12), (16, 240), (4, -240)] {
            let mut item = add_json(0, 64);
            item["note"]["ratchet"] = serde_json::json!(ratchet);
            item["note"]["microTimingTicks"] = serde_json::json!(micro);
            let ops = parse_ops(&Value::Array(vec![item])).expect("解析");
            let NoteOp::Add { note } = &ops[0] else {
                panic!("必须是 add");
            };
            assert_eq!(note.ratchet, Some(ratchet), "ratchet 字面值");
            assert_eq!(note.micro_timing_ticks, Some(micro), "微时序字面值");

            let project = filled_project();
            let (track_id, clip_id) = lead_clip(&project);
            let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
            match &compiled[0] {
                Op::AddNote { note, .. } => {
                    assert_eq!(note.ratchet, Some(ratchet));
                    assert_eq!(note.micro_timing_ticks, Some(micro));
                }
                other => panic!("必须是 AddNote, 实际 {other:?}"),
            }
        }
    }

    /// 越界 ⇒ `OUT_OF_RANGE`（带 `field` / `value` / `min` / `max`），不是静默夹紧。
    #[test]
    fn ratchet_and_micro_timing_out_of_range_are_out_of_range() {
        for bad in [0i64, 17, -1, 200] {
            let mut item = add_json(0, 60);
            item["note"]["ratchet"] = serde_json::json!(bad);
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::OutOfRange),
                "ratchet {bad}"
            );
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败");
            };
            let data = data.as_ref().expect("必须带 data");
            assert_eq!(data["field"], "ratchet");
            assert_eq!(data["min"], 1);
            assert_eq!(data["max"], 16);
        }
        for bad in [241i64, -241, 1000] {
            let mut item = add_json(0, 60);
            item["note"]["microTimingTicks"] = serde_json::json!(bad);
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::OutOfRange),
                "微时序 {bad}"
            );
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败");
            };
            let data = data.as_ref().expect("必须带 data");
            assert_eq!(data["field"], "microTimingTicks");
            assert_eq!(data["min"], -240);
            assert_eq!(data["max"], 240);
        }
    }

    /// 形状错（不是整数）⇒ `INVALID_PARAMETER_RANGE`，而不是被截断、取整或当成缺省。
    #[test]
    fn ratchet_and_micro_timing_must_be_integers() {
        for bad in [
            serde_json::json!(2.5),
            serde_json::json!("4"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!([4]),
        ] {
            for field in ["ratchet", "microTimingTicks"] {
                let mut item = add_json(0, 60);
                item["note"][field] = bad.clone();
                let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
                assert_eq!(
                    fault.domain_code(),
                    Some(ErrorCode::InvalidParameterRange),
                    "{field} = {bad}"
                );
            }
        }
    }

    /// `note` 里的未知键 ⇒ 响亮拒绝（列出支持集合），**绝不**静默丢弃。
    ///
    /// 与顶层实参口径同源（`tools.rs` 的 `UnknownParam`：拼错的参数必须被拒绝、不能静默
    /// 忽略）—— 同一条纪律不许只守一层。`slide` / `pitchBendCurve` / `syllable` /
    /// `phonemes` 四个模型字段**仍然**没有工具面通路，它们必须**响亮地**说出来，
    /// 而不是原样吞掉。
    #[test]
    fn unknown_note_fields_are_rejected_with_the_supported_set() {
        for unknown in ["slyde", "ratchett", "microtimingticks", "noteId", "slide"] {
            let mut item = add_json(0, 60);
            item["note"][unknown] = serde_json::json!(1);
            let fault = parse_ops(&Value::Array(vec![item])).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{unknown}"
            );
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败");
            };
            let data = data.as_ref().expect("必须带 data");
            assert_eq!(data["field"], unknown);
            let supported = data["supportedNoteFields"].as_array().expect("必须是数组");
            for expected in ["startTick", "pitch", "ratchet", "microTimingTicks"] {
                assert!(
                    supported.iter().any(|value| value == expected),
                    "支持集合必须含 {expected}: {data}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // 材料创建形态（`create: true`）—— 关闭 needs-8 的 MIDI 那一半
    // -----------------------------------------------------------------------

    /// 一条 `add` 的 JSON（身份缺省 ⇒ 由 `parse_note` 确定性派生）。
    fn add_json(start: u64, pitch: u8) -> Value {
        serde_json::json!({
            "kind": "add",
            "note": {"startTick": start, "pitch": pitch, "durationTicks": 480},
        })
    }

    /// 池子里**没有** MIDI 片段的工程（needs-8 的负样本：只剩音频条目）。
    fn project_without_midi_clips() -> YebanProjectV1 {
        let mut project = filled_project();
        project
            .clip_pool
            .retain(|_, entry| entry.content.notes().is_none());
        project
    }

    /// `create: true` ⇒ 恰好**一条** `Op::AddClip`，内容 = 全部 `add`，名字/身份逐字段可控。
    #[test]
    fn create_compiles_adds_into_one_add_clip_op() {
        let project = project_without_midi_clips();
        assert!(
            project
                .clip_pool
                .values()
                .all(|entry| entry.content.notes().is_none()),
            "负样本里不得有 MIDI 片段"
        );
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:material");
        let ops =
            parse_ops(&serde_json::json!([add_json(0, 60), add_json(480, 64)])).expect("解析");

        let compiled = compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect("创建");
        assert_eq!(compiled.len(), 1, "整批 `add` 只折成一条 Op");
        let Op::AddClip { clip } = &compiled[0] else {
            panic!("必须是 Op::AddClip, 实际 {}", compiled[0].name());
        };
        assert_eq!(clip.id, clip_id);
        assert_eq!(clip.name, "Seed");
        let notes = clip.content.notes().expect("必须是 MIDI 内容");
        assert_eq!(notes.len(), 2);
        // `notes` 是 `BTreeMap<EntityId, _>` ⇒ 迭代序是**身份序**，不是插入序。
        let mut pitches: Vec<u8> = notes.values().map(|note| note.pitch).collect();
        pitches.sort_unstable();
        assert_eq!(pitches, vec![60, 64]);

        // 确定性：同一请求 ⇒ 同一份载荷（`dryRun` 预览才能等于真做）。
        let again = compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect("创建");
        assert_eq!(compiled, again, "同一请求必须产出逐字段相同的 op");

        // 结果真的进得了池子，且**这就是** `propose_section` 要的材料口径。
        let mut after = project.clone();
        Op::Batch {
            ops: compiled,
            description: "判据".to_owned(),
        }
        .apply(&mut after)
        .expect("施加");
        let entry = after.clip_pool.get(&clip_id).expect("池里必须有新条目");
        assert!(
            entry.content.notes().is_some_and(|notes| !notes.is_empty()),
            "新条目必须是 `usable_materials` 认的形态 (MIDI 且至少一个音符)"
        );
    }

    /// `create: true` 且池里已有该身份 ⇒ `CONFLICT`（绝不覆盖别人的片段）。
    #[test]
    fn create_refuses_an_existing_clip_identity() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let ops = parse_ops(&serde_json::json!([add_json(0, 60)])).expect("解析");
        let fault =
            compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect_err("必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "CONFLICT");
        assert_eq!(value["error"]["data"]["reason"], "clipAlreadyExists");
    }

    /// `create: true` 只允许 `add`；`delete`/`move`/`velocity` 必须响亮失败。
    #[test]
    fn create_refuses_every_non_add_operation() {
        let project = project_without_midi_clips();
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:kinds");
        let note = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        for (kind, op) in [
            (
                "delete",
                serde_json::json!({"kind": "delete", "noteId": note}),
            ),
            (
                "move",
                serde_json::json!({"kind": "move", "noteId": note, "deltaTick": 1, "deltaPitch": 0}),
            ),
            (
                "velocity",
                serde_json::json!({"kind": "velocity", "noteId": note, "velocity": 1}),
            ),
        ] {
            let mut items = vec![add_json(0, 60)];
            items.push(op);
            let ops = parse_ops(&Value::Array(items)).expect("解析");
            let fault =
                compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{kind}"
            );
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "createRequiresAddOps");
            assert_eq!(value["error"]["data"]["received"], kind);
        }
    }

    /// 两个 `add` 抢同一个音符身份 ⇒ 响亮失败（不静默去重）。
    #[test]
    fn create_refuses_duplicate_note_identities() {
        let project = project_without_midi_clips();
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:dupes");
        let shared = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let ops = parse_ops(&serde_json::json!([
            {"kind": "add", "note": {"id": shared, "startTick": 0, "pitch": 60, "durationTicks": 480}},
            {"kind": "add", "note": {"id": shared, "startTick": 0, "pitch": 64, "durationTicks": 480}},
        ]))
        .expect("解析");
        let fault =
            compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect_err("必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "duplicateNoteId");
    }

    /// 空音符集不是可用材料 —— 直接调 [`compile_create`]（**绕过** `parse_ops` 的
    /// "空数组不是一次编辑请求"那条规则）才能碰到这个分支。
    ///
    /// 为什么值得一条判据：`parse_ops` 今天恰好拦住了空数组，于是这个守卫从工具面
    /// **不可达**；不可达的守卫没有任何判据能证明它还在（注入证明：删掉它，全绿）。
    /// 这条判据让它可达一次，代价是一行 `Vec::new()`。
    #[test]
    fn create_refuses_an_empty_note_set() {
        let project = project_without_midi_clips();
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:empty");
        let fault = compile_create(&project, &track_id, &clip_id, "Seed", &[])
            .expect_err("空音符集必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "INVALID_PARAMETER_RANGE");
    }

    /// 发声数上限在**材料创建**这条路上也有牙：`check_polyphony` 先施加 `ops`
    /// 再读池子，因此 `compile_create` 的 `Op::AddClip` 一进模拟体，读到的
    /// 就是**新片段**的峰值 —— 换句话说，创建形态不需要第二份发声数检查，
    /// 但它继承了同一份上限。判据钉住这一点（把 `check_polyphony` 从创建路径上
    /// 摘掉，这条会红）。
    #[test]
    fn create_enforces_the_polyphony_limit() {
        let project = project_without_midi_clips();
        let (track_id, _) = lead_clip(&filled_project());
        let clip_id = deterministic_id("clip:needs-8:polyphony");
        let flood: Vec<Value> = (0..=MAX_POLYPHONY)
            .map(|index| {
                // ⚠ 音高必须**逐条不同**：`parse_note` 在缺 `id` 时按
                // `(start, pitch, duration)` 派生身份，重复的音高会撞成 duplicateNoteId。
                serde_json::json!({
                    "kind": "add",
                    "note": {
                        "startTick": 0,
                        "pitch": u8::try_from(60 + index).expect("音高"),
                        "durationTicks": 960,
                    },
                })
            })
            .collect();
        let ops = parse_ops(&Value::Array(flood)).expect("解析");
        let compiled = compile_create(&project, &track_id, &clip_id, "Seed", &ops).expect("建");
        let fault = check_polyphony(&project, &clip_id, &compiled).expect_err("必须越界");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["peak"], MAX_POLYPHONY + 1);
        assert_eq!(value["error"]["data"]["limit"], MAX_POLYPHONY);
        // 去掉一个 ⇒ 峰值 = 上限 ⇒ 通过（上限本身合法）。
        let ok_ops = parse_ops(&Value::Array(
            (1..=MAX_POLYPHONY)
                .map(|index| {
                    serde_json::json!({
                        "kind": "add",
                        "note": {
                            "startTick": 0,
                            "pitch": u8::try_from(60 + index).expect("音高"),
                            "durationTicks": 960,
                        },
                    })
                })
                .collect(),
        ))
        .expect("解析");
        let ok = compile_create(&project, &track_id, &clip_id, "Seed", &ok_ops).expect("建");
        assert_eq!(
            check_polyphony(&project, &clip_id, &ok).expect("上限之内"),
            MAX_POLYPHONY
        );
    }

    // -----------------------------------------------------------------------
    // 摆放形态（`arguments.placement`）—— 关闭 needs-6 的"放置/引用片段"那一半
    // -----------------------------------------------------------------------

    /// 一份 `placement` 实参（键序稳定：判据要逐字钉住错误载荷）。
    fn placement_args(value: Value) -> Map<String, Value> {
        let mut arguments = Map::new();
        arguments.insert(PLACEMENT_FIELD.to_owned(), value);
        arguments
    }

    /// 把一组 op 当成**一次提交**来施加/回退（与 `propose_draft` 的封装同一形状）。
    fn batch(ops: &[Op]) -> Op {
        Op::Batch {
            ops: ops.to_vec(),
            description: "判据".to_owned(),
        }
    }

    /// 一个**只有一条 MIDI 片段、且该片段还没被摆放**的工程
    /// （阴性前提：摆放判据必须能看到"池里有、时间轴上没有"这个真实状态）。
    fn unplaced_midi_clip() -> (YebanProjectV1, EntityId, EntityId) {
        let mut project = filled_project();
        // 把每一条音轨上的摆放全部清掉（池子不动）—— 于是池里的 MIDI 片段
        // 一条都没上时间轴，正是 needs-6 描述的状态。
        for track in project.tracks.values_mut() {
            track.clips.clear();
        }
        let (track_id, clip_id) = lead_clip(&project);
        assert!(
            project.tracks.values().all(|track| track.clips.is_empty()),
            "阴性前提: 时间轴上必须没有任何摆放"
        );
        assert!(project.clip_pool.contains_key(&clip_id), "材料必须在池子里");
        (project, track_id, clip_id)
    }

    /// 字段名与支持集合被钉住（不多报一个键，也不少报一个）。
    #[test]
    fn placement_field_names_are_pinned() {
        assert_eq!(PLACEMENT_FIELD, "placement");
        assert_eq!(
            PLACEMENT_FIELDS,
            ["startTick", "durationTicks", "placementId", "muted"]
        );
    }

    /// 缺省（不给 `placement`）= `None` = 不摆放：接线之前的行为逐字节不变。
    #[test]
    fn no_placement_argument_means_no_placement() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let parsed = parse_placement(&project, &track_id, &clip_id, &Map::new()).expect("缺省");
        assert_eq!(parsed, None);
    }

    /// 四个键全部缺省时：起点 0、时值 = 片段内容长度、身份由标签确定性派生、不静音。
    #[test]
    fn placement_defaults_come_from_the_clip_content() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let placement = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({})),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(placement.clip_id, clip_id);
        assert_eq!(placement.start_tick, 0);
        assert!(!placement.muted);
        assert_eq!(
            placement.loop_config,
            LoopConfig::default(),
            "刻意不暴露循环旋钮 (渲染没有展开它)"
        );
        // 时值 = 片段里最后一个音符的结束 tick（不猜、不夹紧）。
        let expected = project.clip_pool[&clip_id]
            .content
            .notes()
            .expect("MIDI")
            .values()
            .map(|note| note.start_tick + note.duration_ticks)
            .max()
            .expect("至少一个音符");
        assert_eq!(placement.duration_ticks, expected);
        // 身份 = `placement_label` 的确定性派生（同一请求 ⇒ 同一身份）。
        assert_eq!(
            placement.id,
            deterministic_id(&placement_label(
                &clip_id.to_canonical_string(),
                &track_id.to_canonical_string(),
                0
            ))
        );
        // 两条独立事实：同一个标签两次派生必须同值；不同起点必须不同值。
        assert_eq!(
            deterministic_id(&placement_label("c", "t", 0)),
            deterministic_id(&placement_label("c", "t", 0))
        );
        assert_ne!(
            deterministic_id(&placement_label("c", "t", 0)),
            deterministic_id(&placement_label("c", "t", 1))
        );
    }

    /// 显式给的两个键**逐字**落进载荷（起点进标签 ⇒ 换起点换身份）。
    #[test]
    fn explicit_start_and_duration_reach_the_placement() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let explicit = deterministic_id("placement:explicit");
        let placement = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "startTick": 7680,
                "durationTicks": 960,
                "placementId": explicit.to_canonical_string(),
                "muted": true,
            })),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(placement.start_tick, 7680);
        assert_eq!(placement.duration_ticks, 960);
        assert_eq!(placement.id, explicit);
        assert!(placement.muted);
    }

    /// `durationTicks` 推不出来的两种片段都必须**响亮**要求显式给（不猜假长度）。
    #[test]
    fn duration_must_be_explicit_when_the_clip_cannot_derive_it() {
        let (mut project, track_id, clip_id) = unplaced_midi_clip();
        // ① 空 MIDI 片段：把音符清空。
        let notes = project
            .clip_pool
            .get_mut(&clip_id)
            .and_then(|entry| entry.content.notes_mut())
            .expect("MIDI 片段");
        notes.clear();
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({})),
        )
        .expect_err("空片段推不出长度");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "durationNotDerivable");
        assert_eq!(value["error"]["data"]["isMidi"], true);
        // 显式给时值 ⇒ 空 MIDI 片段也能摆（诚实: 它只是不出声）。
        let placed = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"durationTicks": 960})),
        )
        .expect("显式时值")
        .expect("在场");
        assert_eq!(placed.duration_ticks, 960);
        // ② 非 MIDI 片段（音频条目）同理。
        let project = filled_project();
        let audio = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有音频条目")
            .id;
        let fault = parse_placement(
            &project,
            &track_id,
            &audio,
            &placement_args(serde_json::json!({})),
        )
        .expect_err("音频片段推不出长度");
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "durationNotDerivable");
        assert_eq!(value["error"]["data"]["isMidi"], false);
    }

    /// 时值 0 在**这一层**就被拒（模型层同样拒绝；这里多给 `field`/`value` 的结构化载荷）。
    #[test]
    fn zero_duration_is_refused_with_the_field() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"durationTicks": 0})),
        )
        .expect_err("零时值必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["field"], "placement.durationTicks");
        assert_eq!(value["error"]["data"]["value"], 0);
    }

    /// 未知键**响亮拒绝**并列出支持集合（与 `note` 的未知键同一口径，绝不静默丢弃）。
    #[test]
    fn unknown_placement_fields_are_rejected_with_the_supported_set() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        for unknown in ["start", "starttick", "loopEnabled", "clipId"] {
            let mut payload = serde_json::json!({"startTick": 0});
            payload[unknown] = serde_json::json!(1);
            let fault = parse_placement(&project, &track_id, &clip_id, &placement_args(payload))
                .expect_err("必须拒绝");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "unknownPlacementField");
            assert_eq!(value["error"]["data"]["field"], unknown);
            let supported = value["error"]["data"]["supportedPlacementFields"]
                .as_array()
                .expect("必须是数组");
            for expected in PLACEMENT_FIELDS {
                assert!(
                    supported.iter().any(|item| item == expected),
                    "支持集合必须含 {expected}"
                );
            }
        }
    }

    /// 形状错（不是对象 / 键类型不对 / 身份不是 ULID / 负数）都是参数错误，不是静默缺省。
    #[test]
    fn bad_placement_shapes_are_parameter_errors() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let cases = [
            serde_json::json!("place it"),
            serde_json::json!({"startTick": -1}),
            serde_json::json!({"startTick": 1.5}),
            serde_json::json!({"durationTicks": "960"}),
            serde_json::json!({"muted": "yes"}),
            serde_json::json!({"placementId": "not-a-ulid"}),
            serde_json::json!({"placementId": 42}),
        ];
        for case in cases {
            let fault =
                parse_placement(&project, &track_id, &clip_id, &placement_args(case.clone()))
                    .expect_err("必须拒绝");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{case}"
            );
        }
    }

    /// 两个端点必须真的存在：音轨不存在 ⇒ `TRACK_NOT_FOUND`；片段不存在 ⇒ `CLIP_NOT_FOUND`。
    #[test]
    fn both_endpoints_must_exist() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let ghost = deterministic_id("ghost");
        let fault = parse_placement(
            &project,
            &ghost,
            &clip_id,
            &placement_args(serde_json::json!({})),
        )
        .expect_err("幽灵音轨");
        assert_eq!(fault.domain_code(), Some(ErrorCode::TrackNotFound));
        let fault = parse_placement(
            &project,
            &track_id,
            &ghost,
            &placement_args(serde_json::json!({})),
        )
        .expect_err("幽灵片段");
        assert_eq!(fault.domain_code(), Some(ErrorCode::ClipNotFound));
    }

    /// 同一条摆放重复提交 ⇒ `CONFLICT`（逐字段相同的重放请走 `idempotencyKey`），
    /// 同一身份不同内容 ⇒ 同样是 `CONFLICT`，但 `reason` 不同。
    #[test]
    fn an_existing_placement_identity_is_a_conflict() {
        let (mut project, track_id, clip_id) = unplaced_midi_clip();
        let first = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 0})),
        )
        .expect("解析")
        .expect("在场");
        // 先把这条摆放"做出来"（直接施加到克隆体，模拟上一次调用已经合并）。
        Op::AddClipPlacement {
            track_id,
            placement: first,
        }
        .apply(&mut project)
        .expect("施加");
        // ① 逐字段相同 ⇒ placementAlreadyExists。
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 0})),
        )
        .expect_err("已存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "placementAlreadyExists");
        assert_eq!(
            value["error"]["data"]["placementId"],
            first.id.to_canonical_string()
        );
        // ② 同身份、不同时值 ⇒ placementIdConflict。
        let fault = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "startTick": 0,
                "placementId": first.id.to_canonical_string(),
                "durationTicks": 480,
            })),
        )
        .expect_err("身份被占用");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["reason"], "placementIdConflict");
    }

    /// 摆放编译成**一条** `Op::AddClipPlacement`，端点与载荷逐字段等于解析结果。
    #[test]
    fn placement_compiles_into_one_add_clip_placement_op() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let placement = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 1920, "muted": true})),
        )
        .expect("解析")
        .expect("在场");
        let ops = vec![Op::AddClipPlacement {
            track_id,
            placement,
        }];
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].name(), "AddClipPlacement");
        // 施加到克隆体 ⇒ 时间轴上真的多出这一条（渲染器读的就是 `track.clips`）。
        let mut after = project.clone();
        batch(&ops).apply(&mut after).expect("整批施加");
        assert_eq!(after.tracks[&track_id].clips[&placement.id], placement);
        assert_eq!(after.clip_pool[&clip_id], project.clip_pool[&clip_id]);
        // 逆操作逐字节回到原位（`Batch` 的逆 = 逆序取逆）。
        let mut back = after;
        batch(&ops).apply_inverse(&mut back).expect("回退");
        assert_eq!(
            serde_json::to_value(&back).expect("序列化"),
            serde_json::to_value(&project).expect("序列化"),
            "撤销必须逐字段复原"
        );
    }

    // -----------------------------------------------------------------------
    // 摆放**编辑**形态（`placement.kind` = `move` / `remove`）
    //
    // 这一组钉住的是"模型层早已实现、工具面此前够不着"的那两个 `Op`：
    // `Op::MoveClipPlacement` 与 `Op::RemoveClipPlacement`（渲染器**真的**按
    // `track.clips` 出片 ⇒ 能不能挪 / 能不能取走是可听的能力）。
    // -----------------------------------------------------------------------

    /// 形态名与**每个形态**的词表被钉住：扩展形态不得发明内容键。
    #[test]
    fn placement_edit_kinds_and_field_sets_are_pinned() {
        assert_eq!(PLACEMENT_KIND_FIELD, "kind");
        assert_eq!(
            PLACEMENT_KINDS,
            [
                PLACEMENT_KIND_ADD,
                PLACEMENT_KIND_MOVE,
                PLACEMENT_KIND_REMOVE
            ]
        );
        assert_eq!(
            PLACEMENT_ADD_FIELDS,
            ["kind", "startTick", "durationTicks", "placementId", "muted"]
        );
        assert_eq!(PLACEMENT_MOVE_FIELDS, ["kind", "startTick", "placementId"]);
        assert_eq!(PLACEMENT_REMOVE_FIELDS, ["kind", "placementId"]);
        // `add` 的**内容键**表没有被这次扩展改动（判据 `placement_field_names_are_pinned`
        // 仍逐字成立）。
        assert_eq!(
            PLACEMENT_FIELDS,
            ["startTick", "durationTicks", "placementId", "muted"]
        );
        for fields in [PLACEMENT_MOVE_FIELDS, PLACEMENT_REMOVE_FIELDS] {
            assert!(
                fields.contains(&PLACEMENT_KIND_FIELD),
                "每个形态都要能写 `kind`"
            );
            for field in fields
                .iter()
                .filter(|field| **field != PLACEMENT_KIND_FIELD)
            {
                assert!(
                    PLACEMENT_FIELDS.contains(field),
                    "扩展形态不得发明新的内容键: {field}"
                );
            }
        }
    }

    /// 一份**已经摆好**一条 MIDI 片段的工程，外加那条摆放本身（`move`/`remove` 的夹具）。
    fn placed_midi_clip() -> (YebanProjectV1, EntityId, EntityId, ClipPlacement) {
        let (mut project, track_id, clip_id) = unplaced_midi_clip();
        let placement = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 960, "durationTicks": 480})),
        )
        .expect("解析")
        .expect("在场");
        Op::AddClipPlacement {
            track_id,
            placement,
        }
        .apply(&mut project)
        .expect("施加");
        (project, track_id, clip_id, placement)
    }

    /// `move` 的**旧**起点取自文档、**新**起点取自实参 —— 两者都不是调用方声明的。
    #[test]
    fn move_edit_takes_the_old_tick_from_the_document_and_the_new_tick_from_the_arguments() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let edit = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_MOVE,
                "placementId": existing.id.to_canonical_string(),
                "startTick": 4321,
            })),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(
            edit,
            PlacementEdit::Move {
                placement_id: existing.id,
                previous_start_tick: 960,
                new_start_tick: 4321,
            }
        );
        assert_eq!(edit.kind_name(), PLACEMENT_KIND_MOVE);
    }

    /// `remove` 的撤销载荷是**文档里那一条摆放本身**（逐字段相等）。
    #[test]
    fn remove_edit_carries_the_document_placement_as_the_undo_payload() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let edit = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_REMOVE,
                "placementId": existing.id.to_canonical_string(),
            })),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(
            edit,
            PlacementEdit::Remove {
                placement_id: existing.id,
                previous_placement: existing,
            }
        );
        assert_eq!(edit.kind_name(), PLACEMENT_KIND_REMOVE);
    }

    /// 两个新形态都**真的**编译成模型里对应的那一个 `Op`，并且**可逆**
    /// （逆操作只有一份事实源：`Op::invert`）。
    #[test]
    fn move_and_remove_edits_are_reversible_through_the_model_inverse() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let before = serde_json::to_value(&project).expect("序列化");

        // ---- move: 960 → 4321, 施加后真的挪了, 逆操作逐字节回到原位 ----
        let moved = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_MOVE,
                "placementId": existing.id.to_canonical_string(),
                "startTick": 4321,
            })),
        )
        .expect("解析")
        .expect("在场");
        let PlacementEdit::Move {
            placement_id,
            previous_start_tick,
            new_start_tick,
        } = moved
        else {
            panic!("必须是 move 形态");
        };
        let move_op = Op::MoveClipPlacement {
            track_id,
            placement_id,
            old_start_tick: previous_start_tick,
            new_start_tick,
        };
        assert_eq!(move_op.name(), "MoveClipPlacement");
        let mut after_move = project.clone();
        batch(std::slice::from_ref(&move_op))
            .apply(&mut after_move)
            .expect("施加");
        assert_eq!(
            after_move.tracks[&track_id].clips[&existing.id].start_tick,
            4321
        );
        assert_ne!(
            serde_json::to_value(&after_move).expect("序列化"),
            before,
            "move 必须真的改变文档, 否则下面那条回退断言什么也没证明"
        );
        let mut back_from_move = after_move;
        batch(&[move_op])
            .apply_inverse(&mut back_from_move)
            .expect("回退");
        assert_eq!(
            serde_json::to_value(&back_from_move).expect("序列化"),
            before,
            "move 的逆操作必须逐字段复原"
        );

        // ---- remove: 时间轴上真的空了一条, 逆操作逐字节回到原位 ----
        let removed = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_REMOVE,
                "placementId": existing.id.to_canonical_string(),
            })),
        )
        .expect("解析")
        .expect("在场");
        let PlacementEdit::Remove {
            placement_id,
            previous_placement,
        } = removed
        else {
            panic!("必须是 remove 形态");
        };
        let remove_op = Op::RemoveClipPlacement {
            track_id,
            placement_id,
            previous_placement,
        };
        assert_eq!(remove_op.name(), "RemoveClipPlacement");
        let mut after_remove = project.clone();
        batch(std::slice::from_ref(&remove_op))
            .apply(&mut after_remove)
            .expect("施加");
        assert!(
            after_remove.tracks[&track_id].clips.is_empty(),
            "remove 必须真的把这条摆放从时间轴上取走"
        );
        assert!(
            after_remove.clip_pool.contains_key(&clip_id),
            "remove **只**取走摆放, 池子里的材料不动"
        );
        let mut back_from_remove = after_remove;
        batch(&[remove_op])
            .apply_inverse(&mut back_from_remove)
            .expect("回退");
        assert_eq!(
            serde_json::to_value(&back_from_remove).expect("序列化"),
            before,
            "remove 的逆操作必须逐字段复原"
        );
    }

    /// 缺省的 `kind` 与显式 `add` 逐字段等价，且等于 `add` 解析器的结果
    /// （缺省路径逐字节不变）。
    #[test]
    fn absent_or_explicit_add_kind_parses_to_the_same_edit() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        let absent = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 480})),
        )
        .expect("解析")
        .expect("在场");
        let explicit = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"kind": PLACEMENT_KIND_ADD, "startTick": 480})),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(absent, explicit);
        assert_eq!(absent.kind_name(), PLACEMENT_KIND_ADD);
        let direct = parse_placement(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"startTick": 480})),
        )
        .expect("解析")
        .expect("在场");
        assert_eq!(absent, PlacementEdit::Add(direct));
        // 对象不在场 ⇒ `None`（不摆放）。
        assert_eq!(
            parse_placement_edit(&project, &track_id, &clip_id, &Map::new()).expect("缺省"),
            None
        );
    }

    /// 形态判别键的坏形状与未知形态都**响亮拒绝**，并列出支持集合（不静默按 `add` 处理）。
    #[test]
    fn unknown_placement_kinds_are_rejected_with_the_supported_set() {
        let (project, track_id, clip_id) = unplaced_midi_clip();
        for payload in [
            serde_json::json!({"kind": "adds"}),
            serde_json::json!({"kind": "delete"}),
            serde_json::json!({"kind": ""}),
        ] {
            let fault =
                parse_placement_edit(&project, &track_id, &clip_id, &placement_args(payload))
                    .expect_err("未知形态必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "unknownPlacementKind");
            let supported = value["error"]["data"]["supportedPlacementKinds"]
                .as_array()
                .expect("必须是数组");
            for expected in PLACEMENT_KINDS {
                assert!(
                    supported.iter().any(|item| item == expected),
                    "支持集合必须含 {expected}"
                );
            }
        }
        // `kind` 不是字符串 ⇒ 形状错误, 不是"未知形态"。
        let fault = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({"kind": 3})),
        )
        .expect_err("`kind` 必须是字符串");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(
            value["error"]["data"]["field"],
            format!("{PLACEMENT_FIELD}.{PLACEMENT_KIND_FIELD}")
        );
    }

    /// `move` / `remove` 的**必填**载荷缺了就响亮拒绝（派生身份只属于 `add`）。
    #[test]
    fn move_and_remove_require_their_own_payload() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let id = existing.id.to_canonical_string();
        let cases = [
            (
                serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "startTick": 10}),
                "moveRequiresPlacementId",
            ),
            (
                serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": id}),
                "moveRequiresStartTick",
            ),
            (
                serde_json::json!({"kind": PLACEMENT_KIND_REMOVE}),
                "removeRequiresPlacementId",
            ),
        ];
        for (payload, reason) in cases {
            let fault =
                parse_placement_edit(&project, &track_id, &clip_id, &placement_args(payload))
                    .expect_err("缺必填载荷必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], reason);
        }
    }

    /// 已知但**本形态不适用**的键报 `placementFieldNotApplicable`，真拼错的键仍报
    /// `unknownPlacementField` —— 两者不混为一谈，也不静默丢弃。
    #[test]
    fn inapplicable_and_misspelled_placement_fields_are_told_apart() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let id = existing.id.to_canonical_string();
        let inapplicable = [
            serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": id, "startTick": 1, "muted": true}),
            serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": id, "startTick": 1, "durationTicks": 480}),
            serde_json::json!({"kind": PLACEMENT_KIND_REMOVE, "placementId": id, "startTick": 1}),
            serde_json::json!({"kind": PLACEMENT_KIND_REMOVE, "placementId": id, "muted": false}),
        ];
        for payload in inapplicable {
            let fault = parse_placement_edit(
                &project,
                &track_id,
                &clip_id,
                &placement_args(payload.clone()),
            )
            .expect_err("本形态不适用的键必须被拒");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{payload}"
            );
            let value = fault.into_result().expect("带内");
            assert_eq!(
                value["error"]["data"]["reason"], "placementFieldNotApplicable",
                "{payload}"
            );
            let supported = value["error"]["data"]["supportedPlacementFields"]
                .as_array()
                .expect("必须是数组");
            assert!(
                !supported
                    .iter()
                    .any(|item| *item == value["error"]["data"]["field"]),
                "报出来的字段不得出现在本形态的支持集合里: {payload}"
            );
        }
        // 真拼错 / 根本不支持的键（`mutedd` / `loopEnabled`）仍报 `unknownPlacementField`。
        for unknown in ["mutedd", "loopEnabled"] {
            let mut payload = serde_json::json!({
                "kind": PLACEMENT_KIND_MOVE,
                "placementId": id,
                "startTick": 1,
            });
            payload[unknown] = serde_json::json!(true);
            let fault =
                parse_placement_edit(&project, &track_id, &clip_id, &placement_args(payload))
                    .expect_err("未知键必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "unknownPlacementField");
            assert_eq!(value["error"]["data"]["field"], unknown);
        }
    }

    /// 指名的摆放不在目标音轨上 ⇒ `ENTITY_NOT_FOUND`（不是静默新建一条）。
    #[test]
    fn move_and_remove_refuse_a_placement_that_is_not_on_the_track() {
        let (project, track_id, clip_id, _) = placed_midi_clip();
        let ghost = deterministic_id("placement:not-on-this-track").to_canonical_string();
        for payload in [
            serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": ghost, "startTick": 10}),
            serde_json::json!({"kind": PLACEMENT_KIND_REMOVE, "placementId": ghost}),
        ] {
            let kind = payload["kind"].as_str().expect("kind").to_owned();
            let fault =
                parse_placement_edit(&project, &track_id, &clip_id, &placement_args(payload))
                    .expect_err("幽灵摆放必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::EntityNotFound));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "placementNotFound");
            assert_eq!(value["error"]["data"]["placementKind"], kind.as_str());
        }
    }

    /// `clipId` 与文档里那条摆放引用的片段不一致 ⇒ 响亮失败（不静默改用文档那一条）。
    #[test]
    fn move_and_remove_refuse_a_clip_id_that_is_not_the_placed_one() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let other = project
            .clip_pool
            .keys()
            .find(|id| **id != clip_id)
            .copied()
            .expect("样本里必须还有第二条片段池条目");
        let id = existing.id.to_canonical_string();
        for payload in [
            serde_json::json!({"kind": PLACEMENT_KIND_MOVE, "placementId": id, "startTick": 10}),
            serde_json::json!({"kind": PLACEMENT_KIND_REMOVE, "placementId": id}),
        ] {
            let fault = parse_placement_edit(&project, &track_id, &other, &placement_args(payload))
                .expect_err("片段不一致必须被拒");
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["data"]["reason"], "placementClipMismatch");
            assert_eq!(
                value["error"]["data"]["placementClipId"],
                clip_id.to_canonical_string()
            );
            assert_eq!(
                value["error"]["data"]["clipId"],
                other.to_canonical_string()
            );
        }
    }

    /// `move` 到**原起点** ⇒ `CONFLICT`（没有可提交的改动；不制造一条空提案）。
    #[test]
    fn move_to_the_current_start_tick_is_a_loud_conflict() {
        let (project, track_id, clip_id, existing) = placed_midi_clip();
        let fault = parse_placement_edit(
            &project,
            &track_id,
            &clip_id,
            &placement_args(serde_json::json!({
                "kind": PLACEMENT_KIND_MOVE,
                "placementId": existing.id.to_canonical_string(),
                "startTick": existing.start_tick,
            })),
        )
        .expect_err("零位移必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::Conflict));
        let value = fault.into_result().expect("带内");
        assert_eq!(
            value["error"]["data"]["reason"],
            "placementAlreadyAtStartTick"
        );
        assert_eq!(value["error"]["data"]["startTick"], existing.start_tick);
    }
}
