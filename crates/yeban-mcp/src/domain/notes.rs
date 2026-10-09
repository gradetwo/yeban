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
//!
//! ## 静态混音值形态（`ops[].kind == "setParam"`）—— 关闭"工具面写不了静态混音值"
//!
//! 模型早就有 [`Op::SetParam`]（目标 [`AutomationTarget::TrackVolume`] / [`AutomationTarget::TrackPan`]，
//! `read_param` / `write_param` 直接读写 `TrackV3::volume_db` / `TrackV3::pan`），
//! 而在这个 kind 之前，**17** 个工具里**没有**任何一个能写它们：`yeban_edit_automation`
//! 写的是 [`Op::SetAutomationPoint`]（自动化**点**），读侧能报 `staticValue` 却没有写侧；
//! `yeban_import_audio` 的 `gainDb` 是**片段**增益，不是音轨静态值。
//!
//! 为什么落在本工具的 `ops[]` 上（而不是新增工具、也不改 `yeban_edit_automation` 的实参）：
//!
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2 给本工具的行是
//!   `ops: Vec<NoteOp>` —— `NoteOp` 的 JSON 形状**没有契约**
//!   （`schemas/mcp-tools.schema.json` 只把本工具列在 `name.enum` 里，
//!   实参由 `crate::tools::ToolSpec::input_schema` 逐条派生），
//!   因此**加一个 kind** 不改 `schemas/**` 一个字节；
//! - 本工具是**扩展工具**名单之外的那十个之一，而扩展工具（含 `yeban_edit_automation`）
//!   的实参集合被 `schemas/mcp-tools.schema.json` 的
//!   `definitions.ExtensionToolArguments.$defs` 逐字段钉住
//!   （判据 `tests/contract.rs::extension_argument_constraints_match_the_registry`）
//!   ⇒ 往它们身上加实参会**必须**同步 `schemas/**`（本线禁改）；
//! - `ADR-0001` **D46** 的扩张原则是"先扩既有工具的参数，只有确实不合适才新增工具"。
//!
//! 语义与模型**同源**，本层不另立第二份：
//!
//! - 目标名用 `yeban_edit_automation` 的同一份词汇表（[`LaneKind`]），只放行
//!   [`StaticLane::NAMES`] 这两个"有静态值"的目标；另外三个仍是**响亮失败**；
//! - `old_val`（撤销载荷）从**当前文档**读，走 [`AutomationTarget::static_value`]
//!   （与 `yeban_edit_automation` 的 `staticValue` 读数**同一个**入口），
//!   因此撤销是模型自己的 [`Op::invert`]，本层不写逆操作；
//! - 值域判定（声相 `-1.0..=1.0`、非有限值）**不在本层**：`Op::SetParam` 的
//!   `apply` 会调模型自己的 `validate_param_value`，本层只拦"不是数字"这类 JSON 形状错误。
//!
//! 落地路径与本工具既有的音符编辑**完全相同**：先提案、再 `yeban_merge_proposal`。
//!
//! ## 音轨开关形态（`ops[].kind == "setTrackMute"` / `"setTrackSolo"`）
//! —— 关闭"工具面写不了静音 / 独奏"这一半
//!
//! 上一票让 `setParam` 能写音轨的**静态**音量与声相，但同一排通道条上另外两个开关
//! 仍然够不着：模型有 [`Op::SetTrackMute`] / [`Op::SetTrackSolo`]（载荷是 `bool`，
//! 各自带自包含的 `old_mute` / `old_solo` 撤销载荷），母带渲染器**真的**读它们
//! （`super::render` 的 `audible` 判定），而这两个变体在整个 `crates/yeban-mcp` 里
//! **一次都没有被构造过** ⇒ 也就是"渲染器会静音"这件已实现的能力在 17 个工具的
//! 面上**不可达**（与 `setParam` 之前那一票同型的缺口）。
//!
//! 为什么**不能**把它们塞进 `setParam` 的两个目标：`AutomationTarget` 没有静音 /
//! 独奏变体，而 `SetParam` 的两个载荷都是 `f32`（`TrackV3::mute` / `solo` 是 `bool`）
//! ⇒ 布尔开关在 `SetParam` 里**不可表达**。因此本文件加两个 `kind`，名字是模型 `Op`
//! 变体名的小驼峰（与 [`SET_PARAM_KIND`] 同一条命名规则）。
//!
//! 两条刻意设成**响亮失败**的口径（绝不静默降级）：
//!
//! | 情形 | 结果 |
//! | :--- | :--- |
//! | `value` 不是 JSON 布尔（`1` / `"true"` / 缺字段） | `INVALID_PARAMETER_RANGE`（`reason = "valueMustBeBoolean"`）—— 不做真假值强转 |
//! | 开关对象里有 `kind` / `value` 之外的键（含嵌套 `trackId`） | `INVALID_PARAMETER_RANGE`（`reason = "unknownFlagField"`）—— 目标音轨是**顶层** `trackId`，嵌套写它只会被静默忽略 |
//!
//! `old_mute` / `old_solo` 从**当前文档**读（不是调用方的声明），因此模型的
//! `OpStateMismatch` 前置条件天然成立，撤销仍是模型自己的 [`Op::invert`]。

use std::collections::BTreeMap;
use std::str::FromStr as _;

use serde_json::{Map, Value};

use yeban_model::music::{MICRO_TIMING_MAX_ABS, RATCHET_MAX, RATCHET_MIN};
use yeban_model::{
    AutomationTarget, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote,
    Op, TrackV3, YebanProjectV1,
};

use super::error::{Fault, from_model};
use super::extension_pure::LaneKind;
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

/// `ops[].kind` 的**静态混音值**形态名（写 [`Op::SetParam`]）。
///
/// 与模型 `Op` 变体名同词（`SetParam` 的小驼峰），与其余四个 kind
/// （`add` / `delete` / `move` / `velocity`）同一风格。
pub const SET_PARAM_KIND: &str = "setParam";

/// `setParam` 的**目标**字段名（`ops[].lane`，必填）。
///
/// 借用 `yeban_edit_automation` 的同名实参与 [`LaneKind`] 的同一份词汇表
/// （`ADR-0001` D48：同一个词必须同一个意思）——目标名逐字等于 `project.json` 的变体名。
pub const SET_PARAM_LANE_FIELD: &str = "lane";

/// `setParam` 的**新值**字段名（`ops[].value`，必填，数字）。
pub const SET_PARAM_VALUE_FIELD: &str = "value";

/// `ops[].kind` 的**音轨静音**形态名（写 [`Op::SetTrackMute`]）。
///
/// 与模型 `Op` 变体名同词（`SetTrackMute` 的小驼峰），与 [`SET_PARAM_KIND`] 同一条规则。
pub const SET_TRACK_MUTE_KIND: &str = "setTrackMute";

/// `ops[].kind` 的**音轨独奏**形态名（写 [`Op::SetTrackSolo`]）。
pub const SET_TRACK_SOLO_KIND: &str = "setTrackSolo";

/// 音轨开关形态的**新值**字段名（`ops[].value`，必填，布尔）。
///
/// 与 [`SET_PARAM_VALUE_FIELD`] 逐字同词（`ADR-0001` D48：同一个词必须同一个意思 ——
/// "这次要写进去的值"），但**类型不同**：开关只收 JSON 布尔，不做真假值强转。
pub const TRACK_FLAG_VALUE_FIELD: &str = "value";

/// 音轨开关形态允许出现的**全部**键（判别键 + 新值键）。
///
/// 目标音轨**不在**这里：它是工具顶层的 `trackId`（与 `setParam` 同一条口径）。
/// 多写一个键（尤其是嵌套的 `trackId`）是**响亮失败**，不静默丢弃。
pub const TRACK_FLAG_FIELDS: [&str; 2] = ["kind", TRACK_FLAG_VALUE_FIELD];

/// `ops[].kind` 的**全集**（规范顺序：四个音符 / 摆放形态在前，音轨级形态在后）。
///
/// 错误信息（[`parse_one`] 的未知 `kind`）与判据共用这一份真相。
pub const OP_KINDS: [&str; 7] = [
    "add",
    "delete",
    "move",
    "velocity",
    SET_PARAM_KIND,
    SET_TRACK_MUTE_KIND,
    SET_TRACK_SOLO_KIND,
];

/// `setParam` 能写的**静态目标**（[`Op::SetParam`] 里"有静态值可写"的那两个）。
///
/// 为什么单列一个二值枚举而不是直接收 [`LaneKind`]：`SendGain` 的模型语义是
/// `Option<f32>`（`None` = 单位增益），`SetParam` **明文拒绝**它（必须走
/// `Op::SetRoutingGain`）；`DeviceParam` / `Macro` 的静态写入在本工具面**没有**
/// 通路。用二值类型把"哪三个不可写"变成**不可表达**，比在 `compile` 里补一条
/// 不可达分支更诚实。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticLane {
    /// 音轨静态音量（`TrackV3::volume_db`，单位 dB，有限值）。
    TrackVolume,
    /// 音轨静态声相（`TrackV3::pan`，-1.0..=1.0）。
    TrackPan,
}

impl StaticLane {
    /// 允许的目标名，**规范顺序**（错误信息的 `allowed` 与判据共用）。
    ///
    /// 从 [`LaneKind`] 自己的 `as_str` 派生 ⇒ 词汇表**只有一份**（不是第二张手写表）。
    pub const NAMES: [&'static str; 2] =
        [LaneKind::TrackVolume.as_str(), LaneKind::TrackPan.as_str()];

    /// 把目标落到具体音轨上（[`Op::SetParam`] 的载荷）。
    #[must_use]
    pub fn target(self, track_id: EntityId) -> AutomationTarget {
        match self {
            Self::TrackVolume => AutomationTarget::TrackVolume { track_id },
            Self::TrackPan => AutomationTarget::TrackPan { track_id },
        }
    }
}

/// 通道条上的一个**布尔开关**（[`Op::SetTrackMute`] / [`Op::SetTrackSolo`]）。
///
/// 为什么单列一个二值枚举：两个 `kind` 的**载荷完全相同**（一个 `bool`），
/// 差异只在目标字段与模型变体上。用类型把这条差异收成一处，
/// [`parse_one`] 与 [`compile`] 各自只有**一个**开关分支（不是两份会漂移的复制）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackFlag {
    /// 音轨静音（`TrackV3::mute`，[`Op::SetTrackMute`]）。
    Mute,
    /// 音轨独奏（`TrackV3::solo`，[`Op::SetTrackSolo`]）。
    Solo,
}

impl TrackFlag {
    /// 两个形态名，**规范顺序**（错误信息的 `allowed` 与判据共用）。
    pub const NAMES: [&'static str; 2] = [SET_TRACK_MUTE_KIND, SET_TRACK_SOLO_KIND];

    /// 该开关在 `arguments.ops[].kind` 里的字面名字（错误信息与判据共用同一份真相）。
    #[must_use]
    pub const fn kind_name(self) -> &'static str {
        match self {
            Self::Mute => SET_TRACK_MUTE_KIND,
            Self::Solo => SET_TRACK_SOLO_KIND,
        }
    }

    /// 撤销载荷要读的**当前**开关态（`TrackV3::mute` / `TrackV3::solo`）。
    ///
    /// 与模型 `apply` 的前置条件读的是**同一个字段**：本层不复制那份判定，
    /// 只是把文档现值搬进 `Op` 的 `old_*`。
    #[must_use]
    pub const fn read(self, track: &TrackV3) -> bool {
        match self {
            Self::Mute => track.mute,
            Self::Solo => track.solo,
        }
    }

    /// 编译成模型变体（`old_*` 由调用方从文档读入）。
    #[must_use]
    pub const fn compile(self, track_id: EntityId, old: bool, new: bool) -> Op {
        match self {
            Self::Mute => Op::SetTrackMute {
                track_id,
                old_mute: old,
                new_mute: new,
            },
            Self::Solo => Op::SetTrackSolo {
                track_id,
                old_solo: old,
                new_solo: new,
            },
        }
    }
}

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
    /// 写**静态混音值**（[`Op::SetParam`]）：音轨音量或声相。
    ///
    /// 这是本枚举里第一个**音轨级**（而非音符级）的形态：它不读、不写任何音符，
    /// 目标音轨由调用方的 `trackId` 给出（见 [`compile`]）。值域判定在模型层
    /// （[`Op::SetParam`] 的 `apply` → `validate_param_value`）。
    SetParam {
        /// 目标（只有"有静态值"的两个变体）。
        lane: StaticLane,
        /// 目标值；`TrackVolume` 单位 dB（有限值），`TrackPan` ∈ -1.0..=1.0。
        value: f32,
    },
    /// 写一个**音轨开关**（[`Op::SetTrackMute`] / [`Op::SetTrackSolo`]）。
    ///
    /// 与 [`Self::SetParam`] 同族（音轨级、目标由顶层 `trackId` 给出），
    /// 但载荷是**布尔**：模型把 `mute` / `solo` 从 `SetParam` 里分出去的理由
    /// （`f32` 写不了 `bool`）在工具面这一侧同样成立。撤销载荷 `old_*` 由
    /// [`TrackFlag::read`] 从**当前文档**读，不是调用方声明。
    SetTrackFlag {
        /// 哪个开关。
        flag: TrackFlag,
        /// 目标态。
        value: bool,
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
            Self::SetParam { .. } => SET_PARAM_KIND,
            Self::SetTrackFlag { flag, .. } => flag.kind_name(),
        }
    }

    /// 该形态是否**读/写音符**（即是否必须在一条 MIDI 片段上施加）。
    ///
    /// [`Self::SetParam`] 与 [`Self::SetTrackFlag`] 都是**音轨级**的：它们跟片段内容
    /// 无关。这条区分让 [`compile`] 的"必须是 MIDI 片段"断言只在真的有音符操作时成立
    /// （旧行为逐字节不变：四个音符形态的调用仍然要求 MIDI 材料）。
    #[must_use]
    pub const fn is_note_level(&self) -> bool {
        !matches!(self, Self::SetParam { .. } | Self::SetTrackFlag { .. })
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
/// {"kind":"setParam","lane":"TrackVolume","value":-6.0}
/// {"kind":"setParam","lane":"TrackPan","value":-0.25}
/// {"kind":"setTrackMute","value":true}
/// {"kind":"setTrackSolo","value":false}
/// ```
///
/// `note.probability` / `note.ratchet` / `note.microTimingTicks` 是**可选**字段
/// （缺省逐字节等于旧行为）：给了就是 [`MidiNote`] 对应字段的字面值，语义与判定入口
/// 见 [`PROBABILITY_FIELD`] / [`RATCHET_FIELD`] / [`MICRO_TIMING_FIELD`]。
/// `note` 里 [`NOTE_FIELDS`] 之外的键一律**响亮拒绝**，不静默丢弃。
///
/// `setParam` / `setTrackMute` / `setTrackSolo` 是**音轨级**形态（见
/// [`NoteOp::is_note_level`]）：`setParam` 的 `lane` 只认 [`StaticLane::NAMES`]，
/// 其余三个自动化目标名（`SendGain` / `DeviceParam` / `Macro`）与未知名字都是
/// **响亮失败**（`INVALID_PARAMETER_RANGE`，`data.allowed` 给出全集）。
/// 值的范围判定**不在本层**（见 [`compile`]）；两个开关形态的 `value` 只收 JSON 布尔。
///
/// # Errors
///
/// - `ops` 不是数组 / 元素不是对象 / 缺字段 / 字段类型不对 / `note` 里有未知键 /
///   开关对象里有 [`TRACK_FLAG_FIELDS`] 之外的键 →
///   `INVALID_PARAMETER_RANGE`（含未知 `kind`、未知 `lane`、不可写 `lane`、
///   非布尔开关值）；
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
        SET_PARAM_KIND => Ok(NoteOp::SetParam {
            lane: parse_static_lane(object)?,
            value: read_number(object, SET_PARAM_VALUE_FIELD)?,
        }),
        SET_TRACK_MUTE_KIND => Ok(NoteOp::SetTrackFlag {
            flag: TrackFlag::Mute,
            value: parse_track_flag_value(object)?,
        }),
        SET_TRACK_SOLO_KIND => Ok(NoteOp::SetTrackFlag {
            flag: TrackFlag::Solo,
            value: parse_track_flag_value(object)?,
        }),
        other => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知 `kind`: `{other}`"),
            serde_json::json!({ "supportedKinds": OP_KINDS }),
        )),
    }
}

/// 解析 `setParam` 的 `lane`（只认 [`StaticLane::NAMES`]）。
///
/// 词汇表与 `yeban_edit_automation` **同一份**（[`LaneKind`]）：已知但不可写的三个目标是
/// **响亮失败**，未知名字（别名 / 拼错）是另一条响亮失败，两条都不静默回退到默认值。
///
/// | 情形 | `data.reason` |
/// | :--- | :--- |
/// | `lane` 不是字符串 | `laneMustBeString` |
/// | 名字是另外三个自动化目标（`SendGain` / `DeviceParam` / `Macro`） | `staticLaneNotApplicable` |
/// | 名字不在 [`LaneKind`] 的词汇表里（别名 / 拼错） | `unknownStaticLane` |
/// | `lane` 缺失 | 统一的缺字段错误（无 `data`） |
fn parse_static_lane(object: &Map<String, Value>) -> Result<StaticLane, Fault> {
    let raw = object
        .get(SET_PARAM_LANE_FIELD)
        .ok_or_else(|| missing(SET_PARAM_LANE_FIELD, "字符串"))?;
    let text = raw.as_str().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{SET_PARAM_LANE_FIELD}` 必须是字符串, 实际收到 {raw}"),
            serde_json::json!({
                "field": SET_PARAM_LANE_FIELD,
                "reason": "laneMustBeString",
                "allowed": StaticLane::NAMES,
            }),
        )
    })?;
    let Some(kind) = LaneKind::parse(text) else {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知静态目标 `{text}`"),
            serde_json::json!({
                "field": SET_PARAM_LANE_FIELD,
                "reason": "unknownStaticLane",
                "received": text,
                "allowed": StaticLane::NAMES,
                "note": "只接受 project.json 的规范变体名 (不接受 trackVolume 这类别名)",
            }),
        ));
    };
    // 另外三个目标是**已知但此形态不适用**：说清楚它们各自为什么不适用，
    // 而不是笼统地报"未知名字"。
    match kind {
        LaneKind::TrackVolume => Ok(StaticLane::TrackVolume),
        LaneKind::TrackPan => Ok(StaticLane::TrackPan),
        LaneKind::SendGain => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`setParam` 不接受目标 `SendGain`: `SendGain` 的取值是 `Option<f32>` \
             (`None` = 单位增益), `Op::SetParam` 明文拒绝它; 发送增益必须走 `Op::SetRoutingGain`",
            serde_json::json!({
                "field": SET_PARAM_LANE_FIELD,
                "reason": "staticLaneNotApplicable",
                "received": kind.as_str(),
                "allowed": StaticLane::NAMES,
            }),
        )),
        LaneKind::DeviceParam | LaneKind::Macro => Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{SET_PARAM_KIND}` 不接受目标 `{}`: 本形态只写**音轨**的静态音量/声相; \
                 设备参数与宏的静态写入在工具面没有通路",
                kind.as_str()
            ),
            serde_json::json!({
                "field": SET_PARAM_LANE_FIELD,
                "reason": "staticLaneNotApplicable",
                "received": kind.as_str(),
                "allowed": StaticLane::NAMES,
            }),
        )),
    }
}

/// 读一个**音轨开关**的目标态（`ops[].value`，只收 JSON 布尔）。
///
/// 先把开关对象上**不该出现**的键拒掉（[`reject_track_flag_fields`]），再读值：
/// 顺序是刻意的 —— 嵌套的 `trackId` 是最危险的错键（写错音轨却静默成功），
/// 它必须在任何"值看起来没问题"的路径之前就被点名。
///
/// `1` / `0` / `"true"` / `null` 一律**响亮失败**，不做真假值强转：模型 `TrackV3::mute`
/// 是 `bool`，一次"猜调用方意思"的强转就是第二份语义。
fn parse_track_flag_value(object: &Map<String, Value>) -> Result<bool, Fault> {
    reject_track_flag_fields(object)?;
    let raw = object
        .get(TRACK_FLAG_VALUE_FIELD)
        .ok_or_else(|| missing(TRACK_FLAG_VALUE_FIELD, "布尔"))?;
    raw.as_bool().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{TRACK_FLAG_VALUE_FIELD}` 必须是布尔, 实际收到 {raw} \
                 (开关不做真假值强转: `1` / `\"true\"` 都不是布尔)"
            ),
            serde_json::json!({
                "field": TRACK_FLAG_VALUE_FIELD,
                "reason": "valueMustBeBoolean",
                "received": raw,
            }),
        )
    })
}

/// 拒绝开关对象里 [`TRACK_FLAG_FIELDS`] 之外的键。
///
/// 与 [`reject_unknown_note_fields`] 同一口径（"拼错的键必须被拒绝, 不能静默忽略"），
/// 只是对象更小。`data.hint` 明确写出目标音轨的正确位置（顶层 `trackId`）——
/// 最常见的错法是把它嵌套进操作对象，那一写会被静默忽略、开关落到**别的**音轨上。
fn reject_track_flag_fields(object: &Map<String, Value>) -> Result<(), Fault> {
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !TRACK_FLAG_FIELDS.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(Fault::domain_with_data(
        ErrorCode::InvalidParameterRange,
        format!(
            "音轨开关操作里有不支持的键: {} (支持集合只有 {TRACK_FLAG_FIELDS:?})",
            unknown.join(", ")
        ),
        serde_json::json!({
            "reason": "unknownFlagField",
            "unsupportedFields": unknown,
            "supportedFlagFields": TRACK_FLAG_FIELDS,
            "hint": "目标音轨是工具顶层的 `trackId`; 嵌套在操作对象里的 `trackId` 不会被读取",
        }),
    ))
}

/// 读一个**有限**的 JSON 数字（`f64` → `f32`，与 `value` 的模型类型同宽）。
///
/// 两条防线各管一段，**没有一条是摆设**：
///
/// - JSON 解析器本身不会产出非有限的 `f64`（`serde_json` 对溢出成 `±inf` 的字面量
///   返回 `NumberOutOfRange`），所以"输入是 `NaN` / `±inf`"这条路走不通；
/// - **但收窄到 `f32` 会**：`1e300` 是有限 `f64`，`as f32` 之后是 `inf`。因此有限性
///   检查放在**收窄之后**，判的是模型真正收到的那个 `f32`。
///
/// 越界时本层报 `INVALID_PARAMETER_RANGE`（`data.reason = "nonFiniteValue"`）；
/// 模型 [`Op::SetParam`] 的 `validate_param_value` 对同一个 `f32` 也判 `NonFiniteValue`
/// ⇒ 契约码一致（`domain/error.rs` 的映射），不是两份口径。
/// 范围（声相 `-1.0..=1.0`）**不在这里**：那是模型的事。
fn read_number(object: &Map<String, Value>, field: &str) -> Result<f32, Fault> {
    let raw = object.get(field).ok_or_else(|| missing(field, "数字"))?;
    let number = raw.as_f64().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是数字, 实际收到 {raw}"),
            serde_json::json!({ "field": field, "reason": "valueMustBeNumber" }),
        )
    })?;
    // `Op::SetParam` 的载荷类型就是 `f32`, 所以这一步的 f64 → f32 舍入是模型类型本身
    // 要求的 (与 `read_probability` 同型): IEEE 最近偶数, 确定性。
    #[allow(clippy::cast_possible_truncation)]
    let value = number as f32;
    if !value.is_finite() {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 收窄到 f32 后不是有限数: 输入 {number}, f32 {value}"),
            serde_json::json!({
                "field": field,
                "value": number,
                "narrowedToF32": value.to_string(),
                "reason": "nonFiniteValue",
            }),
        ));
    }
    Ok(value)
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
/// `SetParam` 是**音轨级**的：目标由 `track_id` 给出，`old_val` 从当前文档读
/// （[`AutomationTarget::static_value`]，与 `yeban_edit_automation` 的 `staticValue`
/// 读数同一个入口），因此模型的 `OpStateMismatch` 前置条件天然成立。
///
/// [`NoteOp::SetTrackFlag`] 同样是**音轨级**的：`old_mute` / `old_solo` 由
/// [`TrackFlag::read`] 从当前文档读（模型 `apply` 的前置条件读的是同一个字段）。
///
/// # Errors
///
/// - 音轨不存在 → `TRACK_NOT_FOUND`；
/// - 片段不存在 → `CLIP_NOT_FOUND`；**有音符操作**且片段不是 MIDI → `CLIP_NOT_FOUND`
///   （纯 `setParam` / 纯开关调用不要求片段是 MIDI：它们不读片段内容）；
/// - 音符不存在 → `ENTITY_NOT_FOUND`；
/// - 模型层校验失败 → [`super::error::code_for_model`] 给出的契约码。
pub fn compile(
    project: &YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
    ops: &[NoteOp],
) -> Result<Vec<Op>, Fault> {
    let track = project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    let entry = project
        .clip_pool
        .get(clip_id)
        .ok_or_else(|| Fault::domain(ErrorCode::ClipNotFound, format!("片段不存在: {clip_id}")))?;
    // "必须是 MIDI 片段"这条断言只在**真的有音符操作**时成立：`setParam` 一个音符都不读。
    // 四个音符形态的调用因此逐字节等于旧行为（它们总是走到这条断言）。
    if ops.iter().any(NoteOp::is_note_level) && entry.content.notes().is_none() {
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
            NoteOp::SetParam { lane, value } => {
                let target = lane.target(*track_id);
                // 撤销载荷来自**唯一**的静态值入口 (`AutomationTarget::static_value`)：
                // 本层不自己读 `track.volume_db` / `track.pan`（那会是第二份真相）。
                let old_val = target
                    .static_value(project)
                    .map_err(|error| from_model("静态值读取", &error))?;
                Op::SetParam {
                    target,
                    old_val,
                    new_val: *value,
                }
            }
            NoteOp::SetTrackFlag { flag, value } => {
                // 撤销载荷来自**当前文档**（模型 `apply` 的前置条件读同一个字段）；
                // 本层不自己写 `Op::invert`（那是模型的唯一事实源）。
                flag.compile(*track_id, flag.read(track), *value)
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
                         `delete`/`move`/`velocity` 指向; 音轨级的 \
                         `setParam`/`setTrackMute`/`setTrackSolo` 与建材料无关), \
                         实际收到 `{}`",
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

    /// `ops[].kind == "setParam"` 的**规范**形状：解析 → 编译 → 真的改工程 → 逆操作回原。
    ///
    /// 这一条是"工具面写不了静态混音值"缺口的**字面**判据：它钉住
    /// `old_val` 来自当前文档（不是调用方声明）、`new_val` 是 `Op::SetParam` 的载荷、
    /// 且 `Op::invert` 能逐字节回退。
    #[test]
    fn set_param_compiles_against_the_document_and_inverts_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        let volume_before = project.track(&track_id).expect("音轨").volume_db;
        let pan_before = project.track(&track_id).expect("音轨").pan;

        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "TrackVolume", "value": -9.5},
            {"kind": "setParam", "lane": "TrackPan", "value": 0.75}
        ]))
        .expect("规范形状必须被接受");
        assert!(ops.iter().all(|op| !op.is_note_level()), "两条都是音轨级");
        assert_eq!(ops[0].kind_name(), SET_PARAM_KIND);

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 2);
        match &compiled[0] {
            Op::SetParam {
                target:
                    AutomationTarget::TrackVolume {
                        track_id: target_track,
                    },
                old_val,
                new_val,
            } => {
                assert_eq!(*target_track, track_id);
                assert_eq!(*old_val, volume_before, "撤销载荷必须来自当前文档");
                assert_eq!(*new_val, -9.5);
            }
            other => panic!("应当是 TrackVolume 的 SetParam: {other:?}"),
        }

        Op::Batch {
            ops: compiled.clone(),
            description: "setParam".to_owned(),
        }
        .apply(&mut project)
        .expect("施加");
        assert_eq!(project.track(&track_id).expect("音轨").volume_db, -9.5);
        assert_eq!(project.track(&track_id).expect("音轨").pan, 0.75);

        for op in compiled.iter().rev() {
            op.apply_inverse(&mut project).expect("逆操作");
        }
        let restored = project.track(&track_id).expect("音轨");
        assert_eq!(restored.volume_db, volume_before, "音量必须逐字节回原值");
        assert_eq!(restored.pan, pan_before, "声相必须逐字节回原值");
    }

    /// 声相值域**不在本层**：越界的 `value` 在模型自己的 `validate_param_value` 处
    /// 变成 `PanOutOfRange`（契约码 `OUT_OF_RANGE`），本层只搬字面值。
    #[test]
    fn set_param_pan_range_is_judged_by_the_model_not_by_this_layer() {
        let project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        // 本层必须**接受**这个形状（-1.0..=1.0 的判定不属于它）。
        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "TrackPan", "value": 2.0}
        ]))
        .expect("本层不做值域判定");
        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        let mut simulated = project.clone();
        let failure = compiled[0]
            .apply(&mut simulated)
            .expect_err("模型必须拒绝越界声相");
        assert_eq!(
            super::super::error::code_for_model(&failure),
            ErrorCode::OutOfRange
        );
    }

    /// 另外三个自动化目标名与别名都是**响亮失败**：不静默回退、不猜。
    #[test]
    fn set_param_rejects_non_static_lanes_and_aliases() {
        for (payload, expected_reason) in [
            (
                serde_json::json!([{"kind": "setParam", "lane": "SendGain", "value": 0.0}]),
                Some("staticLaneNotApplicable"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "DeviceParam", "value": 0.0}]),
                Some("staticLaneNotApplicable"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "Macro", "value": 0.0}]),
                Some("staticLaneNotApplicable"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "trackVolume", "value": 0.0}]),
                Some("unknownStaticLane"),
            ),
            // 缺 `value` / `value` 不是数字 / `lane` 不是字符串 / 缺 `lane`。
            (
                serde_json::json!([{"kind": "setParam", "lane": "TrackVolume"}]),
                None,
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": "TrackVolume", "value": "loud"}]),
                Some("valueMustBeNumber"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "lane": 3, "value": 0.0}]),
                Some("laneMustBeString"),
            ),
            (
                serde_json::json!([{"kind": "setParam", "value": 0.0}]),
                None,
            ),
            // 有限 `f64` 收窄到 `f32` 会溢出成 `inf` ⇒ 这一条**可达**（不是摆设）。
            (
                serde_json::json!([{"kind": "setParam", "lane": "TrackVolume", "value": 1e300}]),
                Some("nonFiniteValue"),
            ),
        ] {
            let fault = parse_ops(&payload).expect_err(&format!("必须被拒: {payload}"));
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{payload}"
            );
            let Fault::Domain { data, .. } = &fault else {
                panic!("必须是领域失败: {payload}");
            };
            let Some(reason) = expected_reason else {
                assert!(data.is_none(), "缺字段错误不该带 data: {payload}");
                continue;
            };
            let data = data.clone().expect("形状错误必须带 data");
            assert_eq!(data["reason"], reason, "{payload} 的 data: {data}");
            if reason.contains("Lane") {
                assert_eq!(
                    data["allowed"],
                    serde_json::json!(StaticLane::NAMES),
                    "{payload} 必须报出允许集合"
                );
            }
        }
    }

    /// 纯 `setParam` 调用**不要求**片段是 MIDI（它一个音符都不读）；
    /// 同一片段上的音符操作**仍然**要求 MIDI（旧行为逐字节不变）。
    #[test]
    fn set_param_alone_does_not_require_a_midi_clip() {
        let project = filled_project();
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段");
        let track_id = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;
        let clip_id = audio_clip.id;

        let ops = parse_ops(&serde_json::json!([
            {"kind": "setParam", "lane": "TrackVolume", "value": -12.0}
        ]))
        .expect("解析");
        compile(&project, &track_id, &clip_id, &ops).expect("纯静态写入不要求 MIDI 材料");

        let note_ops = parse_ops(&serde_json::json!([
            {"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}
        ]))
        .expect("解析");
        let fault = compile(&project, &track_id, &clip_id, &note_ops)
            .expect_err("音符操作仍然要求 MIDI 材料");
        assert_eq!(fault.domain_code(), Some(ErrorCode::ClipNotFound));
    }

    /// `ops[].kind == "setTrackMute"` / `"setTrackSolo"` 的**规范**形状：
    /// 解析 → 编译 → 真的改工程 → 逆操作回原。
    ///
    /// 这一条是"工具面写不了静音 / 独奏"缺口的**字面**判据：它钉住
    /// `old_mute` / `old_solo` 来自当前文档（不是调用方声明）、`new_*` 是模型变体的载荷、
    /// 且 `Op::invert` 能逐字节回退。
    ///
    /// 文档先被推到"两个开关**已经打开**"再写 `false`：这条安排是判据的**牙齿** ——
    /// 文档值恰好等于注入常量时，"把 `old_*` 写死成常量"的注入会全绿（实测过一次）。
    ///
    /// 注入（实测红）：删掉 `parse_one` 的两个分支 ⇒ 未知 `kind`；把
    /// [`TrackFlag::read`] 写死成常量 ⇒ 这里报 `old_mute` 不是 `true`。
    #[test]
    fn track_flags_compile_against_the_document_and_invert_byte_for_byte() {
        let mut project = filled_project();
        let (track_id, clip_id) = lead_clip(&project);
        {
            let track = project.track_mut(&track_id).expect("音轨");
            track.mute = true;
            track.solo = true;
        }
        let track_before = project.track(&track_id).expect("音轨").clone();
        let mute_before = track_before.mute;
        let solo_before = track_before.solo;
        assert!(mute_before && solo_before, "夹具前提: 两个开关先是打开的");

        let ops = parse_ops(&serde_json::json!([
            {"kind": "setTrackMute", "value": false},
            {"kind": "setTrackSolo", "value": false}
        ]))
        .expect("规范形状必须被接受");
        assert!(ops.iter().all(|op| !op.is_note_level()), "两条都是音轨级");
        assert_eq!(ops[0].kind_name(), SET_TRACK_MUTE_KIND);
        assert_eq!(ops[1].kind_name(), SET_TRACK_SOLO_KIND);

        let compiled = compile(&project, &track_id, &clip_id, &ops).expect("编译");
        assert_eq!(compiled.len(), 2);
        match &compiled[0] {
            Op::SetTrackMute {
                track_id: target,
                old_mute,
                new_mute,
            } => {
                assert_eq!(*target, track_id);
                assert_eq!(*old_mute, mute_before, "撤销载荷必须来自当前文档");
                assert!(!*new_mute, "目标态是调用方给的 false");
            }
            other => panic!("应当是 SetTrackMute: {other:?}"),
        }
        match &compiled[1] {
            Op::SetTrackSolo {
                track_id: target,
                old_solo,
                new_solo,
            } => {
                assert_eq!(*target, track_id);
                assert_eq!(*old_solo, solo_before, "撤销载荷必须来自当前文档");
                assert!(!*new_solo);
            }
            other => panic!("应当是 SetTrackSolo: {other:?}"),
        }

        Op::Batch {
            ops: compiled.clone(),
            description: "track flags".to_owned(),
        }
        .apply(&mut project)
        .expect("施加");
        let muted = project.track(&track_id).expect("音轨");
        assert!(!muted.mute, "合并后静音必须真的关掉");
        assert!(!muted.solo, "合并后独奏必须真的关掉");
        assert_eq!(muted.volume_db, track_before.volume_db, "开关不碰音量");
        assert_eq!(muted.pan, track_before.pan, "开关不碰声相");
        assert_eq!(
            muted.solo_safe, track_before.solo_safe,
            "`Op::SetTrackSolo` 不顺手改 `solo_safe`"
        );

        for op in compiled.iter().rev() {
            op.apply_inverse(&mut project).expect("逆操作");
        }
        assert_eq!(
            project.track(&track_id).expect("音轨"),
            &track_before,
            "逆操作必须逐字段回到原音轨"
        );
    }

    /// 开关的**形状**错误全部响亮失败：非布尔值、缺字段、多写的键（含嵌套 `trackId`）。
    ///
    /// 注入：把 `raw.as_bool()` 换成 `raw.as_bool().unwrap_or(false)` ⇒ 前两条不再红。
    #[test]
    fn track_flags_reject_non_boolean_values_and_unknown_keys() {
        // 非布尔值。
        for payload in [
            serde_json::json!([{"kind": "setTrackMute", "value": 1}]),
            serde_json::json!([{"kind": "setTrackSolo", "value": "true"}]),
            serde_json::json!([{"kind": "setTrackMute", "value": null}]),
            serde_json::json!([{"kind": "setTrackSolo", "value": ["yes"]}]),
        ] {
            let fault = parse_ops(&payload).expect_err(&format!("必须被拒: {payload}"));
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(
                value["error"]["data"]["reason"], "valueMustBeBoolean",
                "{payload}"
            );
        }

        // 缺 `value`：统一的缺字段错误（不带 data）。
        let fault = parse_ops(&serde_json::json!([{"kind": "setTrackMute"}])).expect_err("缺字段");
        let Fault::Domain { data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        assert!(data.is_none(), "缺字段错误不该带 data: {data:?}");

        // 开关对象里 `kind` / `value` 之外的键：**响亮失败**，并指出目标音轨的
        // 正确位置是顶层 `trackId`（嵌套写它会被静默忽略 ⇒ 开关落到别的音轨上）。
        for payload in [
            serde_json::json!([{"kind": "setTrackMute", "value": true,
                               "trackId": "01ARZ3NDEKTSV4RRFFQ69G5FAV"}]),
            serde_json::json!([{"kind": "setTrackSolo", "value": false, "lane": "TrackVolume"}]),
        ] {
            let fault = parse_ops(&payload).expect_err(&format!("必须被拒: {payload}"));
            assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
            let value = fault.into_result().expect("带内");
            assert_eq!(
                value["error"]["data"]["reason"], "unknownFlagField",
                "{payload}"
            );
            assert_eq!(
                value["error"]["data"]["supportedFlagFields"],
                serde_json::json!(TRACK_FLAG_FIELDS),
                "{payload}"
            );
            assert!(
                value["error"]["data"]["hint"]
                    .as_str()
                    .is_some_and(|hint| hint.contains("trackId")),
                "必须指出目标音轨在顶层: {payload}"
            );
        }

        // 猜一个更短的名字（`setMute`）不是别名，而是**未知 kind**：错误里给出全集。
        let fault = parse_ops(&serde_json::json!([{"kind": "setMute", "value": true}]))
            .expect_err("`setMute` 不是本工具的形态名");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(
            value["error"]["data"]["supportedKinds"],
            serde_json::json!(OP_KINDS),
            "未知 kind 必须报出全集 (含两个新开关)"
        );
    }

    /// 两个开关是**音轨级**的：纯开关调用不要求片段是 MIDI（一个音符都不读），
    /// 而 `create: true` 的形状里它们仍然被响亮拒绝。
    ///
    /// 注入：把 `NoteOp::is_note_level` 改回"只有 `SetParam` 是音轨级" ⇒ 第一条红。
    #[test]
    fn track_flags_alone_do_not_require_a_midi_clip() {
        let project = filled_project();
        let audio_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_none())
            .expect("样本里必须有非 MIDI 片段");
        let track_id = project
            .tracks
            .values()
            .find(|track| track.kind == yeban_model::TrackKind::Audio)
            .expect("样本里必须有音频轨")
            .id;

        let ops = parse_ops(&serde_json::json!([
            {"kind": "setTrackMute", "value": true},
            {"kind": "setTrackSolo", "value": false}
        ]))
        .expect("解析");
        let compiled =
            compile(&project, &track_id, &audio_clip.id, &ops).expect("纯开关写入不要求 MIDI 材料");
        assert_eq!(compiled.len(), 2);

        // `kind` 的全集必须真的登记这两个名字（错误信息的 `supportedKinds` 与判据共用）。
        assert_eq!(OP_KINDS.len(), 7);
        assert_eq!(TrackFlag::NAMES, [SET_TRACK_MUTE_KIND, SET_TRACK_SOLO_KIND]);
        assert!(OP_KINDS.contains(&SET_TRACK_MUTE_KIND));
        assert!(OP_KINDS.contains(&SET_TRACK_SOLO_KIND));
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
            // 两个音轨级开关与建材料**无关**：`create: true` 只收 `add`。
            (
                "setTrackMute",
                serde_json::json!({"kind": "setTrackMute", "value": true}),
            ),
            (
                "setTrackSolo",
                serde_json::json!({"kind": "setTrackSolo", "value": true}),
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
