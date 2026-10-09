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

use std::collections::BTreeMap;
use std::str::FromStr as _;

use serde_json::{Map, Value};

use yeban_model::music::{MICRO_TIMING_MAX_ABS, RATCHET_MAX, RATCHET_MIN};
use yeban_model::{ClipContent, ClipPoolEntry, EntityId, MidiNote, Op, YebanProjectV1};

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
}
