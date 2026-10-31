//! 成功响应的 **`data` 形状**（`tools/list` 的 `outputSchema`）[#176，`#174` 坑⑥]。
//!
//! ## 为什么需要它
//!
//! `#174` 的外部 agent 试跑实测：**21 个工具全无 `outputSchema`** ⇒
//! `data.proposal.proposalId` / `appliedOps` / `peak.before|after` /
//! `sends[].gainDb` / `inserts[].unrecognisedParams` / `undoable` 这些字段
//! **只能靠试**。声明一份 schema 不能让 agent 少调一次工具，但能让它**读到**
//! "这次成功调用会返回什么形状"。
//!
//! ## 这个键描述的是**哪一层**
//!
//! MCP 的 `outputSchema` 约定上描述"工具的结构化输出"。本 crate 的工具响应是
//! `ToolResponse` 信封（`{"status":"success","data":{…}}`，见 [`crate::tools::ToolResponse`]），
//! 因此这里声明的对象是**成功响应的 `data`**（不是信封、也不是错误响应）。
//! 这条口径**不是**口头的：每份 schema 都带 `"x-yeban-validates": "result.data"`，
//! 判据 `tests/props_176.rs` 用它决定校验哪一层。
//!
//! ## 字段名与类型**从真实响应抽**，不是理想形状
//!
//! 每份 schema 的字段逐个来自实现里的响应拼装点，判据用**真实 `tools/call` 响应**
//! 反校（[`validate`] 核 `required` ＋ 顶层 `type` ＋ 已声明字段的类型 ＋ `const` ＋ `enum`；
//! [`validate_complete`] 额外核"声明的每个字段名都出现"）。覆盖面就是 [`COVERED_TOOLS`]
//! 那一份清单（⛔ 别在别处抄它的长度）。
//!
//! ⚠ 一个已知的**两种成功形状**：`yeban_merge_proposal` 在"这份提案已经合并过"
//! 时返回一条**更短**的记录（只有 `merged` / `alreadyMerged` / `appliedOps` /
//! `proposal`，没有 `commit` / `baseCommitMoved` / `projectDigest`）。
//! 因此那份 schema 的 `required` 只列**两种形状都有**的字段，并在 `description`
//! 里逐字说明缺的是哪几个 —— 写成"理想形状"会让幂等合并的那次响应**违反自己的 schema**。
//!
//! ⚠ 第二种**按参数分支**的形状（`ADR-0020`）：`yeban_render_master` 的
//! `contributions` 缺省 `false` ⇒ 响应里**没有** `contributions` / `contributionsBasis`；
//! 打开时才出现。这两键同样只进 `properties`、不进 `required`，而
//! [`validate_complete`] 的"全字段"反校必须用 `contributions: true` 的那次响应
//! （`tests/props_176.rs` 与 `tests/render_master.rs` 各有一处）。
//! ⛔ 缺省形状用 [`validate`]（它只核 `required` 与已出现字段的类型）。
//!
//! ⭐ `ADR-0032`（缺口 `G17`）：**凡 [`COVERED_TOOLS`] 列出的 schema 都声明 `commitModel`** —— 它是每条
//! 成功回执都存在的自述键，由 [`with_commit_model`] 在 `finish` 之外**统一**挂上
//! （`properties` + `required` 一起）。真实响应同样由 `apply_inner` 统一注入，
//! 因此"声明的"与"发的"仍是同一件事，⛔ 不是理想形状。
//!
//! ## 校验器做的事（并**被真的跑**）
//!
//! 1. 顶层 `type`（必须是 object）；
//! 2. `required` 里的每个名字都在这一层的数据里；
//! 3. 已声明且出现的字段，其 `type` 与声明一致（`integer` 与 `number` 分开；
//!    `["number","null"]` 这类联合类型按"任一匹配"处理）；
//! 4. 已声明且出现的字段，其 `const` 与取值**逐值相等**；
//! 5. 已声明且出现的字段，其 `enum` 含该取值；
//! 6. ⭐ **逐层递归**（s29 起）：对象字段再核一遍 2–5，数组的每个元素按 `items` 再核一遍，
//!    路径带下标（`$.warnings[0].code`）。
//!
//! ⚠ 第 4、5 条是 `MCP-TOOL-005` 的 edit_automation／redo／reject_proposal 批**补上**的：在此之前 `const` 与 `enum` **只声明不核对**，
//! 于是 `edit_notes` 那份把 `pendingMerge` 写成 `const true` 的 schema 与它自己 `autoMerge: true`
//! 的真实响应（该键为 `false`）**互相矛盾却全绿**。反面判据 `the_validator_can_go_red_and_the_schema_is_not_idealised`
//! 现在把这两类矛盾都钉成"能变红"。
//!
//! ⚠ 第 6 条是 **s28 第二片**补的，病根与上一条同型且更大：校验器原来**只在顶层跑一次**，
//! 于是 `warnings` 里声明的 `items.code.enum` 从不核对 ⇒ 名册可以随便改而全绿（台账 §22.16）。
//! "声明了但从不核对"的键就是装饰，装饰越多越容易把矛盾藏进 CI 的绿灯里。
//!
//! 它**仍不是**通用 JSON Schema 实现：不处理 `$ref` / `oneOf` / `minimum` / `additionalProperties` /
//! `format`；本模块的 schema 只用上述**六种**关键字（`type` / `required` / `const` / `enum` /
//! `properties` / `items`），而这六种**全部真的核对**，好让"声明的"与"可执行的"是同一件事。

use serde_json::{Map, Value, json};

/// 这些 schema 校验的层级（`ToolResponse` 信封里的 `data`）。
pub const VALIDATES: &str = "result.data";

/// 已经声明 `outputSchema` 的主力工具（`#176` 的五工具面 ＋ 提案类三工具 ＋
/// [MCP-TOOL-005] 扩展票再补 `reject_proposal` / `redo` / `edit_automation`，见台账）。
///
/// 顺序 = 注册表顺序里的出现顺序（现读 `tools.rs` 里 `name:` 那几行的行号）；判据要求
/// 这份清单与真实广播面**双向一致**（每个列出的工具都有 schema；每个有 schema 的工具都在清单里）。
///
/// ⚠ 这是**唯一**一份覆盖面清单：模块文档、判据与 `commitModel` 的说明都引用它，
/// ⛔ 不在别处再抄一遍数量（"工具数手抄值会陈旧"这条教训在本仓库两周内被证明过两次）。
pub const COVERED_TOOLS: [&str; 10] = [
    "yeban_propose_section",
    "yeban_edit_notes",
    "yeban_set_macro",
    "yeban_render_master",
    "yeban_merge_proposal",
    "yeban_reject_proposal",
    "yeban_undo",
    "yeban_redo",
    "yeban_edit_automation",
    "yeban_query_engine_state",
];

/// 按工具名取成功响应的 `data` schema；未声明时为 `None`。
///
/// `None` 是**诚实的缺省**：没写 schema 的工具**不广播** `outputSchema` 键，
/// 而不是广播一个空对象让客户端以为"没有字段"。
#[must_use]
pub fn for_tool(name: &str) -> Option<Value> {
    match name {
        "yeban_propose_section" => Some(propose_section()),
        "yeban_edit_notes" => Some(edit_notes()),
        "yeban_set_macro" => Some(set_macro()),
        "yeban_render_master" => Some(render_master()),
        "yeban_merge_proposal" => Some(merge_proposal()),
        "yeban_reject_proposal" => Some(reject_proposal()),
        "yeban_undo" => Some(undo()),
        "yeban_redo" => Some(redo()),
        "yeban_edit_automation" => Some(edit_automation()),
        "yeban_query_engine_state" => Some(query_engine_state()),
        _ => None,
    }
}

/// 校验失败的一处读数（**路径 + 原因**，不是布尔）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// 出问题的 JSON 路径（`$` 开头；数组下标写成 `$.warnings[0].code`，s29 起递归会带下标）。
    pub path: String,
    /// 人话原因（逐字进判据输出）。
    pub message: String,
}

impl Violation {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

/// 用一份 schema 校验一份 `data`：顶层必须是对象，且逐层核对 `type` / `const` / `enum` / `required` / `properties` / `items`。
///
/// # Errors
///
/// 上述任一条不成立。
pub fn validate(schema: &Value, data: &Value) -> Result<(), Violation> {
    check_declared(schema, data, "$")?;
    let object = match data.as_object() {
        Some(object) => object,
        None => {
            return Err(Violation::new(
                "$",
                format!("顶层必须是对象, 实际是 {}", json_type_name(data)),
            ));
        }
    };
    check_object_fields(schema, object, "$")?;
    Ok(())
}

/// 已声明字段的逐层核对：`required` 的缺席 ＋ `properties` 的递归 ＋ 数组的 `items` 递归。
///
/// ⚠ 本函数于 s29 之前**只在顶层跑一次**（`properties` 的每个值只过 [`check_declared`]，只看类型）。
/// 那时的病根与 `const`/`enum` 那一条同型：`warnings` 声明了 `items`，而 `items` 里的
/// `code.enum` **从不核对** ⇒ 广播出去的形状与真实响应互相矛盾却全绿（台账 §22.16）。
/// 现在声明面与可执行面重新变成同一件事：本模块用到的关键字 = `type` / `required` / `const` /
/// `enum` / `properties` / `items`，**这六个全部真的核对**。
fn check_object_fields(
    schema: &Value,
    object: &serde_json::Map<String, Value>,
    path: &str,
) -> Result<(), Violation> {
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for name in required {
            let name = name.as_str().unwrap_or_default();
            if !object.contains_key(name) {
                return Err(Violation::new(
                    format!("{path}.{name}"),
                    "`required` 声明了这个字段, 但响应里没有",
                ));
            }
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            if let Some(value) = object.get(name) {
                check_declared_field(property, value, &format!("{path}.{name}"))?;
            }
        }
    }
    Ok(())
}

/// 一处已声明字段对一份取值的递归核对：先 [`check_declared`]（类型 / `const` / `enum`），
/// 再按取值的形状递归进 `properties`（对象）或 `items`（数组）。
fn check_declared_field(schema: &Value, value: &Value, path: &str) -> Result<(), Violation> {
    check_declared(schema, value, path)?;
    if let Some(object) = value.as_object() {
        check_object_fields(schema, object, path)?;
    } else if let Some(elements) = value.as_array()
        && let Some(items) = schema.get("items")
    {
        for (index, element) in elements.iter().enumerate() {
            check_declared_field(items, element, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

/// [`validate`] ＋ "**声明的每个字段名都出现**"（用于反校一份"全字段"的真实响应）。
///
/// 为什么与 [`validate`] 分开：JSON Schema 的 `properties` 语义是"声明了但可以缺席"，
/// 而本判据要的是更强的一句话 —— "这份 schema 是从真实响应抽出来的，不是凭想象写的"。
/// 两种成功形状的工具（见模块文档）只用 [`validate`]。
///
/// # Errors
///
/// [`validate`] 的任何失败，或某个已声明字段没有出现。
pub fn validate_complete(schema: &Value, data: &Value) -> Result<(), Violation> {
    validate(schema, data)?;
    let object = data.as_object().ok_or_else(|| {
        Violation::new(
            "$",
            format!("顶层必须是对象, 实际是 {}", json_type_name(data)),
        )
    })?;
    for name in declared_fields(schema) {
        if !object.contains_key(&name) {
            return Err(Violation::new(
                format!("$.{name}"),
                "schema 声明了这个字段, 但这份真实响应里没有",
            ));
        }
    }
    Ok(())
}

/// schema 顶层 `properties` 的键（升序 = 与对象序列化顺序无关的确定性读数）。
#[must_use]
pub fn declared_fields(schema: &Value) -> Vec<String> {
    let mut names: Vec<String> = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// 一处声明对一份取值的核对：`type` ＋ `const` ＋ `enum`。
///
/// ⚠ 后两者于 `MCP-TOOL-005` 的 edit_automation／redo／reject_proposal 批**才真的核对**。病根是实测过的：`edit_notes`
/// 曾把 `pendingMerge` 写成 `const true`，而校验器只核 `type` ⇒ 那份 schema 与
/// `autoMerge: true` 的真实响应**互相矛盾却全绿**（"声明了但从不核对"的键就是
/// 装饰，装饰越多越容易把矛盾藏进 CI 的绿灯里）。
/// 现在 `const` 与 `enum` 都按**逐值相等**处理（`serde_json::Value` 的 `PartialEq`）。
/// 声明面仍然刻意小：本模块只用 `type` / `required` / `const` / `enum` / `properties` / `items`
/// 这六种关键字，而**这六种全部真的核对**（前四种在 [`check_declared`]，后两种在
/// [`check_object_fields`] / [`check_declared_field`]，s29 起），
/// 所以"声明的"与"可执行的"是同一件事（`$ref` / `oneOf` / `minimum` 一律不用）。
fn check_declared(schema: &Value, value: &Value, path: &str) -> Result<(), Violation> {
    if let Some(kind) = schema.get("type") {
        let matches = match kind {
            Value::String(kind) => json_type_matches(kind, value),
            Value::Array(kinds) => kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|kind| json_type_matches(kind, value)),
            _ => true,
        };
        if !matches {
            return Err(Violation::new(
                path,
                format!(
                    "schema 声明 type = {kind}, 实际是 {}",
                    json_type_name(value)
                ),
            ));
        }
    }
    if let Some(constant) = schema.get("const")
        && constant != value
    {
        return Err(Violation::new(
            path,
            format!("schema 声明 const = {constant}, 实际是 {value}"),
        ));
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.iter().any(|candidate| candidate == value)
    {
        return Err(Violation::new(
            path,
            format!("schema 声明 enum = {allowed:?}, 实际是 {value}"),
        ));
    }
    Ok(())
}

fn json_type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_f64() => "number",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// 一份 schema 的公共外壳（`type: object` ＋ 校验层级标记）。
fn envelope(title: &str, description: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(
        "$schema".to_owned(),
        Value::from("https://json-schema.org/draft/2020-12/schema"),
    );
    map.insert("title".to_owned(), Value::from(title));
    map.insert("description".to_owned(), Value::from(description));
    map.insert("x-yeban-validates".to_owned(), Value::from(VALIDATES));
    map.insert("type".to_owned(), Value::from("object"));
    map
}

fn finish(mut map: Map<String, Value>, required: &[&str], properties: Value) -> Value {
    map.insert(
        "required".to_owned(),
        Value::Array(required.iter().map(|name| Value::from(*name)).collect()),
    );
    map.insert("properties".to_owned(), properties);
    Value::Object(map)
}

/// `commitModel` 的声明（`ADR-0032` **D1/D3**）。
///
/// 取值⛔ 不在这里手抄：直接引用 [`crate::domain::COMMIT_MODEL_VALUES`]。写死四条
/// 就会有"schema 说四条、实现发五条"的两份清单，而那正是本票要治的病。
fn commit_model_schema() -> Value {
    json!({
        "type": "string",
        "enum": crate::domain::COMMIT_MODEL_VALUES.to_vec(),
        "description": "本次调用的**提交模型**（`ADR-0032`）：`proposal` = 工程头没动，必须再调 \
            `yeban_merge_proposal` 才落盘；`proposalAutoMerged` = 同一次调用里已当场合并 \
            （`arguments.autoMerge`），⛔ 不需要也不必再合并；`direct` = 当场生效、没有\"下一步去合并\"；\
            `none` = 本次不产生任何提交（⛔ 不等于\"没有副作用\" —— 是否写盘请读 `path` / \
            `sideEffect` / `appliedInOfflineRender`，提交数请读 `commitDelta` / `committed`）。"
    })
}

/// 给一份 schema 补上 `commitModel`（`required` 与 `properties` **一起**补）。
///
/// 做成"在 `finish` 外面包一层"、而不是往每份字面量里各加一个键，有两个实测理由：
///
/// 1. `render_master` 的 `json!` 字面量早已贴着 `serde_json` 宏的递归上限
///    （见 [`render_master`] 里 `midiSource` 的来历），再往字面量里加一个键就编不过；
/// 2. 这个键在实现里只有**一个**来源（`domain::commit_model_of` → `apply_inner` 的注入），
///    每份 schema 里它是同一句话 ⇒ 逐处手抄等于把本票要治的"易错"搬进生产代码。
///
/// ⚠ 覆盖面 = [`COVERED_TOOLS`] 列出的那些工具。**没有**被列出的工具（含 `list_proposals`
/// 等）的响应同样带这个键（注入点在
/// 唯一出口上），只是**尚未**声明 `outputSchema` —— 把声明面扩大到注册表里的**每一个**工具
/// 不是本票的范围（`list_proposals` 与 `launch_scene` 另有"必须不广播"的判据钉着）。
fn with_commit_model(mut schema: Value) -> Value {
    if let Value::Object(map) = &mut schema {
        if let Some(Value::Array(required)) = map.get_mut("required") {
            required.push(Value::from(crate::tools::COMMIT_MODEL_KEY));
        }
        if let Some(Value::Object(properties)) = map.get_mut("properties") {
            properties.insert(
                crate::tools::COMMIT_MODEL_KEY.to_owned(),
                commit_model_schema(),
            );
        }
    }
    schema
}

/// 提案记录的**摘要**形状（[`crate::domain::proposal::Proposal::summary`]）。
fn proposal_summary_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "proposalId", "branch", "kind", "title", "description", "status", "baseCommit",
            "headCommit", "mergeCommit", "createdAt", "resolvedAt", "resolution", "opCount",
            "opKinds"
        ],
        "properties": {
            "proposalId": {"type": "string"},
            "branch": {"type": "string"},
            "kind": {"type": "string"},
            "title": {"type": "string"},
            "description": {"type": "string"},
            "status": {"type": "string"},
            "baseCommit": {"type": "string"},
            "headCommit": {"type": "string"},
            "mergeCommit": {"type": ["string", "null"]},
            "createdAt": {"type": "integer"},
            "resolvedAt": {"type": ["integer", "null"]},
            "resolution": {"type": ["string", "null"]},
            "opCount": {"type": "integer"},
            "opKinds": {"type": "array", "items": {"type": "string"}}
        }
    })
}

/// 提案记录的**完整**形状（摘要 ＋ 逐条 op；`yeban_merge_proposal` 的 `proposal`）。
fn proposal_detail_schema() -> Value {
    let mut schema = proposal_summary_schema();
    if let Value::Object(map) = &mut schema
        && let Some(Value::Object(properties)) = map.get_mut("properties")
    {
        properties.insert(
            "ops".to_owned(),
            json!({"type": "array", "items": {"type": "object"}}),
        );
    }
    schema
}

/// 撤销 / 重做响应里的模型读数（[`crate::domain::undo_display_value`]）。
fn undo_display_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "branch", "head", "commitCount", "branchCount",
            "undone", "undoable", "canUndo", "canRedo"
        ],
        "properties": {
            "branch": {"type": "string"},
            "head": {"type": ["string", "null"]},
            "commitCount": {"type": "integer"},
            "branchCount": {"type": "integer"},
            "undone": {"type": "integer"},
            "undoable": {"type": "integer"},
            "canUndo": {"type": "boolean"},
            "canRedo": {"type": "boolean"}
        }
    })
}

/// ⭐ `data.warnings[].code` 的**逐工具子集**（提案族三个广播工具）。全名册的唯一权威是
/// `schemas/mcp-tools.schema.json` 的 `definitions.WarningCode.enum`（机械对账 =
/// `scripts/guards/check_warning_codes.py`：实现侧构造点与该名册必须**集合相等**）；
/// 这里的子集由**调用链现读**得来（台账 §22.16）：`proposal_warnings`（`domain/mod.rs`，=
/// `effect_param_warnings` ＋ `device_on_silent_track_warnings`）的唯一调用点是 `propose_draft`，
/// 而 `propose_draft` 的读数经 `apply_propose` 拼装点同时喂给 `propose_section` / `edit_notes` /
/// `set_macro` ⇒ 这三个工具只会发这两枚。
/// ⚠ 名册里的第五枚 `sfzSamplesUnreadable` ⛔ 不在任何子集里 —— 它只出自 `yeban_load_sfz` /
/// `yeban_render_sfz_wav`（`domain/sfz.rs` 的 `samples_warning`，唯一两个调用点在该文件的响应构造器里），
/// 而 `for_tool` 对扩展工具返回 `None`（不广播 outputSchema，常驻判据
/// `a_missing_schema_is_none_not_an_empty_object` 就在钉这条）⇒ 它由契约名册兜住，⛔ 不许为凑等式造假 schema。
const PROPOSAL_WARNING_CODES: [&str; 2] = ["deviceOnSilentTrack", "unrecognisedEffectParam"];

/// ⭐ `data.warnings[].code` 的逐工具子集（母带）。两条来源 `clipping_warning` +
/// `silent_sources_warning` 只在 `domain/render.rs` 的同一处 `warnings` 装配里链起来
/// （全库现读：这两个函数各只有一个调用点，都在那一条 `let warnings =` 上）⇒ 只有
/// `yeban_render_master` 发这两枚（台账 §22.16 的现读）。
const RENDER_WARNING_CODES: [&str; 2] = ["exportClipping", "silentSources"];

/// 提案族 `warnings[]` 的**逐条形状** —— 三个工具共用一份，因为它们的读数来自同一拼装点
/// （⛔ 不是三份手抄：`edit_notes` 原本内联了这一份，只是 `code` 没带名册）。
///
/// `required` 只放 `code` + `hint`：**两条**构造点都带这两个键，其余键随 code 变化
/// （未识别参数四键 / 静音轨六键）⇒ 取**并集**逐个声明类型，但 ⛔ 不声明 `additionalProperties: false`
/// （那会把另一条形如实地判成违规）。并集里每个键的取值都是**非空具体值**（`DeviceDefinition.name: String`、
/// `recognised_names(): Vec<&'static str>`、`TrackPlayableSurvey.sounding_placements: usize`）⇒ 类型可以放心核。
#[must_use]
fn proposal_warning_items() -> Value {
    json!({
        "type": "object",
        "required": ["code", "hint"],
        "properties": {
            "code": {"type": "string", "enum": PROPOSAL_WARNING_CODES.to_vec()},
            "deviceName": {"type": "string"},
            "paramName": {"type": "string"},
            "knownParams": {"type": "array", "items": {"type": "string"}},
            "trackId": {"type": "string"},
            "track": {"type": "string"},
            "deviceId": {"type": "string"},
            "placements": {"type": "integer"},
            "soundingPlacements": {"type": "integer"},
            "notes": {"type": "integer"},
            "triggeringNotes": {"type": "integer"},
            "hint": {"type": "string"}
        }
    })
}

/// 母带 `warnings[]` 的逐条形状：⛔ 只声明 `code` + `hint` 这两条**两条来源都有**的键。
/// 其余键的差别不是"没想到"，是**未证**：`clipping_warning` 的 `truePeakDbfs` / `headroomDb`
/// 走 `json_f32(Option<f32>)`，缺读数时是 `null`（顶层那两条正是这么声明的），
/// 而 `silentSources` 一条都不带它们 ⇒ 按并集声明会把真实响应**自己拒掉**。
/// 假广播比弱广播更糟，所以这里宁可弱。
#[must_use]
fn render_warning_items() -> Value {
    json!({
        "type": "object",
        "required": ["code", "hint"],
        "properties": {
            "code": {"type": "string", "enum": RENDER_WARNING_CODES.to_vec()},
            "hint": {"type": "string"}
        }
    })
}

/// `yeban_propose_section` 的成功响应 `data`（创作面提案，与 [`edit_notes`] 同拼装点
/// `domain::apply_propose`，[MCP-TOOL-005] 广播面扩展票）。
fn propose_section() -> Value {
    with_commit_model(finish(
        envelope(
            "yeban_propose_section 成功响应的 data",
            "缺省形状：`pendingMerge: true` / `projectUnchanged: true` / \
             `mergeTool: \"yeban_merge_proposal\"`（提案不改工程，`merge` 前另给一次\
             `projectDigest` 作为未变证据）。\
             ⚠ **两种成功形状**（按 `autoMerge` 参数分支，[ADR-0029] D3 两段式回执）：\
             `autoMerge: true` 时**多出** `autoMerged`（恒 `true`）与 `merge`（逐字 = \
             `yeban_merge_proposal` 正常合并的全字段回执，其形状见该工具的 outputSchema；\
             ⚠ 它**不含**嵌套 `commitModel`，顶层那一份只注入一次），并且**翻值**：\
             `pendingMerge` → `false`、`projectUnchanged` → `false`、`mergeTool` → \
             `\"applied-in-process:autoMerge\"`、`proposal` 换成合并后的投影\
             ⇒ 这三个键⛔ 不用 `const`，`autoMerged`/`merge` ⛔ 不进 `required`。\
             `proposal` 缺省是**摘要**（14 键），仅 `includeOps: true` 时多一个 `ops` 键\
             ⇒ `ops` 只进 `properties`。`warnings[]` 与 `edit_notes` 同一份 plan 阶段读数\
             （`code` / `hint` 逐条），可能为空数组。",
        ),
        &[
            "created",
            "projectUnchanged",
            "pendingMerge",
            "mergeTool",
            "projectDigest",
            "unwired",
            "willCreate",
            "warnings",
            "proposal",
        ],
        json!({
            "created": {"type": "boolean"},
            "projectUnchanged": {"type": "boolean"},
            "pendingMerge": {"type": "boolean"},
            "mergeTool": {"type": "string"},
            "projectDigest": {"type": "string"},
            "unwired": {"type": "array", "items": {"type": "string"}},
            "willCreate": {
                "type": "object",
                "required": ["opCount", "opKinds"],
                "properties": {
                    "opCount": {"type": "integer"},
                    "opKinds": {"type": "array", "items": {"type": "string"}},
                    "sections": {"type": "array"},
                    "tracks": {"type": "array"},
                    "clipPoolEntries": {"type": "array"},
                    "placements": {"type": "array"},
                    "routingNodes": {"type": "array"},
                    "routingEdges": {"type": "array"}
                }
            },
            "warnings": {"type": "array", "items": proposal_warning_items()},
            "proposal": proposal_detail_schema(),
            "autoMerged": {"type": "boolean"},
            "merge": {"type": "object"}
        }),
    ))
}

/// `yeban_edit_notes`（以及其余两个提案类工具）的成功响应 `data`。
///
/// 来源：`domain::apply_propose` 的 `ToolResponse::success(json!({…}))`。
fn edit_notes() -> Value {
    with_commit_model(finish(
        envelope(
            "yeban_edit_notes 成功响应的 data",
            "缺省形状：`pendingMerge: true` / `projectUnchanged: true`（本调用**永远**先写隔离提案分支）；\
             `proposal` 是**摘要**（不含逐条 `ops`，除非 `includeOps: true`）。\
             ⚠ **两种成功形状**（按 `autoMerge` 参数分支，[ADR-0029] D3，口径与 \
             `yeban_propose_section` 逐字相同）：`autoMerge: true` 时**多出** `autoMerged`（恒 `true`）\
             与 `merge`（= `yeban_merge_proposal` 正常合并的全字段回执，不含嵌套 `commitModel`），\
             并且翻值：`pendingMerge` → `false`、`projectUnchanged` → `false`、`mergeTool` → \
             `\"applied-in-process:autoMerge\"`、`proposal` 随合并后状态\
             ⇒ 这三键⛔ 不用 `const`，`autoMerged`/`merge` ⛔ 不进 `required`。\
             `warnings[]` 逐条报出**非阻断**问题：`code:\"unrecognisedEffectParam\"` = 引擎不认但又不是\
             拼错的设备参数名（带 `deviceName` / `paramName` / `knownParams` —— \
             后者逐字列出引擎认的全部拼写，可当场改拼写重发）；`code:\"deviceOnSilentTrack\"` = 设备装到了\
             一条没有任何可发声内容的音轨上（带 `trackId` / `track` / `placements` / `notes` 与逐字 `hint`）。\
             两者都可能为空数组。",
        ),
        &[
            "created",
            "projectUnchanged",
            "pendingMerge",
            "mergeTool",
            "projectDigest",
            "unwired",
            "willCreate",
            "warnings",
            "proposal",
        ],
        json!({
            "created": {"type": "boolean"},
            "projectUnchanged": {"type": "boolean"},
            "pendingMerge": {"type": "boolean"},
            "mergeTool": {"type": "string"},
            "projectDigest": {"type": "string"},
            "unwired": {"type": "array", "items": {"type": "string"}},
            "willCreate": {
                "type": "object",
                "required": ["opCount", "opKinds"],
                "properties": {
                    "opCount": {"type": "integer"},
                    "opKinds": {"type": "array", "items": {"type": "string"}},
                    "sections": {"type": "array"},
                    "tracks": {"type": "array"},
                    "clipPoolEntries": {"type": "array"},
                    "placements": {"type": "array"},
                    "routingNodes": {"type": "array"},
                    "routingEdges": {"type": "array"}
                }
            },
            "warnings": {
                "type": "array",
                "items": proposal_warning_items()
            },
            "proposal": proposal_detail_schema(),
            "autoMerged": {"type": "boolean"},
            "merge": {"type": "object"}
        }),
    ))
}

/// `yeban_set_macro` 的成功响应 `data`（第三个提案类工具，与 [`edit_notes`] /
/// [`propose_section`] 共用拼装点 `domain::apply_propose`，形状逐字同源）。
fn set_macro() -> Value {
    with_commit_model(finish(
        envelope(
            "yeban_set_macro 成功响应的 data",
            "宏调整展开为级联自动化 op 后走**同一条**提案路径 ⇒ 形状与 `yeban_edit_notes` \
             逐字相同（键集合与两条分支语义见该工具的 schema，[ADR-0029]）：缺省 \
             `pendingMerge: true` / `projectUnchanged: true`；`autoMerge: true` 时多出 \
             `autoMerged` / `merge` 并翻值（`mergeTool: \"applied-in-process:autoMerge\"`）。\
             `willCreate.opKinds` 反映宏展开的自动化形态；`warnings[]` 同一路算子，宏调用\
             通常为空数组。`proposal` 缺省是摘要，仅 `includeOps: true` 时多 `ops` 一键。",
        ),
        &[
            "created",
            "projectUnchanged",
            "pendingMerge",
            "mergeTool",
            "projectDigest",
            "unwired",
            "willCreate",
            "warnings",
            "proposal",
        ],
        json!({
            "created": {"type": "boolean"},
            "projectUnchanged": {"type": "boolean"},
            "pendingMerge": {"type": "boolean"},
            "mergeTool": {"type": "string"},
            "projectDigest": {"type": "string"},
            "unwired": {"type": "array", "items": {"type": "string"}},
            "willCreate": {
                "type": "object",
                "required": ["opCount", "opKinds"],
                "properties": {
                    "opCount": {"type": "integer"},
                    "opKinds": {"type": "array", "items": {"type": "string"}}
                }
            },
            "warnings": {"type": "array", "items": proposal_warning_items()},
            "proposal": proposal_detail_schema(),
            "autoMerged": {"type": "boolean"},
            "merge": {"type": "object"}
        }),
    ))
}

/// `yeban_merge_proposal` 的成功响应 `data`（**两种形状**）。
fn merge_proposal() -> Value {
    with_commit_model(finish(
        envelope(
            "yeban_merge_proposal 成功响应的 data",
            "正常合并：`commit` 是**多父合并提交**（`parents` 从图谱读回），`baseCommitMoved` 说明基线是否移动过。\
             ⚠ 幂等合并（`alreadyMerged: true`）返回**更短**的形状：只有 `merged` / `alreadyMerged` / \
             `appliedOps` / `proposal`，没有 `commit` / `baseCommitMoved` / `projectDigest` ⇒ 因此这三个键不在 `required` 里。",
        ),
        &["merged", "alreadyMerged", "appliedOps", "proposal"],
        json!({
            "merged": {"type": "boolean"},
            "alreadyMerged": {"type": "boolean"},
            "appliedOps": {"type": "integer"},
            "baseCommitMoved": {"type": "boolean"},
            "projectDigest": {"type": "string"},
            "commit": {
                "type": "object",
                "required": [
                    "id", "branch", "message", "atomicBatch",
                    "parents", "parentCount", "isMerge", "depth"
                ],
                "properties": {
                    "id": {"type": "string"},
                    "branch": {"type": "string"},
                    "message": {"type": "string"},
                    "atomicBatch": {"type": "boolean"},
                    "parents": {"type": "array", "items": {"type": "string"}},
                    "parentCount": {"type": "integer"},
                    "isMerge": {"type": "boolean"},
                    "depth": {"type": "integer"}
                }
            },
            "proposal": proposal_detail_schema()
        }),
    ))
}

/// `yeban_reject_proposal` 的成功响应 `data`（**两种形状**，口径同 [`merge_proposal`]）。
fn reject_proposal() -> Value {
    with_commit_model(finish(
        envelope(
            "yeban_reject_proposal 成功响应的 data",
            "只改记录、**绝不**删除。⚠ 两种形状（拼装点 `domain::apply_reject`）：对**已经**被拒绝的\
             提案再拒一次 ⇒ 短形状，只有 `rejected` / `alreadyRejected`（恒 `true`）/ `proposal`（**摘要**）；\
             真的拒绝一次 ⇒ 长形状，多出 `archived`（恒 `true`）与 `reason`，且 `proposal` 换成**完整载荷**\
             （这里⚠ **没有** `includeOps` 开关 —— 长形状无条件带 `ops`）⇒ `archived` / `reason` \
             ⛔ 不进 `required`。`rejected` 两种形状都恒 `true` ⇒ 用 `const` 钉住（本校验器**真的**核 `const`）。\
             ⚠ 已合并的提案走**失败**回执（`Conflict` ＋ `proposalId`），⛔ 不是成功形状、不进这份 schema。\
             `commitModel` = `none`：拒绝只动提案记录，不产生任何工程提交。",
        ),
        &["rejected", "alreadyRejected", "proposal"],
        json!({
            "rejected": {"type": "boolean", "const": true},
            "alreadyRejected": {"type": "boolean"},
            "archived": {"type": "boolean", "const": true},
            "reason": {"type": "string"},
            "proposal": proposal_detail_schema()
        }),
    ))
}

/// `yeban_edit_automation` 的成功响应 `data`（**单形状**：键恒在、值可为 `null`）。
fn edit_automation() -> Value {
    with_commit_model(finish(
        envelope(
            "yeban_edit_automation 成功响应的 data",
            "只有一条成功形状，但**多个键可取 `null`**：拼装点 `AutomationEdit::data()` 的口径是\
             \"键**始终在**（⛔ 不静默省略）\"—— `readOnly: true`（没给 `point` 实参 ⇒ 一位都不改）时\
             `written` / `valuesAfter` / `projectDigestAfter` 都是 `null`，`residualReason: null` 配\
             `appliedInOfflineRender: false` 的含义是\"没有可求值点 / 读开关关闭\"（与\"有落点但没生效\"\
             分开，⛔ 不拿一个残差类冒充）。`dryRun` 预览与真做**共用同一个拼装函数** ⇒ 两种调用不可能漂移。\
             `offlineRenderError` 只在**离线导出本身失败**时非 `null`（同一处会报 `RENDER_FAILED`）。\
             `origin.kind` 恒 `\"McpEdit\"`：MCP 直接编辑不创建提案。\
             ⭐ 本校验器**会递归**（s29 起）⇒ 下面 `lane` / `read` / `origin` 的嵌套键是**可执行断言**：\
             逐键核 `type`、该层 `required` 缺席即红、路径带下标（形如 `$.lane.points[0]`）。\
             在此之前它们只是文档（原句实测留痕：「本校验器⛔ 不递归」⇒ 这三层的 `required` 从写下到 s29\
             **一次都没被跑过**）。对齐过的构造点：`domain/automation.rs` 里 `lane` 的七个键**全部无条件**\
             `insert`，而 `read` 与 `origin` 是**同一处 JSON 字面量** ⇒ 三者的键不会自己缺席。\
             `applied` / `commitCount` 也在 `required` 里，因为补插点在**唯一**那条成功出口上\
             （`domain/automation.rs:843-847`，`apply` 的末尾、`if` 块之外）⇒ **预览与真做都带**这两个键；\
             别把 `applied` 读成\"只有真做才有\"：它与 `readOnly` 是**同一个判定**（`write.is_none()`），\
             两者的差异只在来源（`readOnly` 由 `data()` 拼、`applied` 由出口补）。形状差异看 `written`：\
             真做是对象、预览是 `null`。",
        ),
        &[
            "trackId",
            "target",
            "targetLabel",
            "laneKind",
            "readOnly",
            "lane",
            "read",
            "written",
            "valuesAfter",
            "appliedInOfflineRender",
            "residualReason",
            "offlineRenderError",
            "projectDigestBefore",
            "projectDigestAfter",
            "origin",
            "applied",
            "commitCount",
        ],
        json!({
            "trackId": {"type": "string"},
            "target": {"type": ["object", "null"]},
            "targetLabel": {"type": "string"},
            "laneKind": {"type": "string"},
            "readOnly": {"type": "boolean"},
            "lane": {
                "type": "object",
                "required": ["exists", "readEnabled", "writeMode", "unit", "domain", "pointCount", "points"],
                "properties": {
                    "exists": {"type": "boolean"},
                    "readEnabled": {"type": "boolean"},
                    "writeMode": {"type": ["string", "null"]},
                    "unit": {"type": "string"},
                    "domain": {"type": ["object", "null"]},
                    "pointCount": {"type": "integer"},
                    "points": {"type": "array", "items": {"type": "object"}}
                }
            },
            "read": {
                "type": "object",
                "required": ["ticks", "values", "entry"],
                "properties": {
                    "ticks": {"type": "array", "items": {"type": "integer"}},
                    "values": {"type": "array"},
                    "entry": {"type": "string"}
                }
            },
            "written": {"type": ["object", "null"]},
            "valuesAfter": {"type": ["array", "null"]},
            "appliedInOfflineRender": {"type": "boolean"},
            "residualReason": {"type": ["string", "null"]},
            "offlineRenderError": {"type": ["string", "null"]},
            "projectDigestBefore": {"type": "string"},
            "projectDigestAfter": {"type": ["string", "null"]},
            "applied": {"type": "boolean"},
            "commitCount": {"type": "integer"},
            "origin": {
                "type": "object",
                "required": ["kind", "author", "note"],
                "properties": {
                    "kind": {"type": "string"},
                    "author": {"type": "string"},
                    "note": {"type": "string"}
                }
            }
        }),
    ))
}

/// `yeban_query_engine_state` 的成功响应 `data`（只读）。
fn query_engine_state() -> Value {
    with_commit_model(finish(
        envelope(
            "yeban_query_engine_state 成功响应的 data",
            "三份状态各有唯一来源（`stateSources` 逐条给出）。`track` 只在给了 `trackId` 时是对象，\
             缺省是 `null`；`sends` 同样只在点名音轨时出现（缺省调用是诊断包 `projectIncluded:false` 的路径，\
             不把工程内容写进去）。`readingsStream` 只在给了 `since` 时出现，故不在 `properties` 里。\
             ⭐ `session.undoCursor` 的口径（`#177`）= 可撤销步数 = 已应用的 op 数，与 `yeban_undo` 响应的 \
             `before.undoable` 同一个数（旧的'已撤销步数'口径已废弃）。",
        ),
        &["session", "engine", "track", "stateSources"],
        json!({
            "session": {
                "type": "object",
                "required": [
                    "playheadTicks", "isPlaying", "transportState", "ticksPerBeat",
                    "playheadBar", "playheadBarOffsetTicks", "taskCount",
                    "pluginPids", "openWindowCount", "undoCursor"
                ],
                "properties": {
                    "playheadTicks": {"type": "integer"},
                    "isPlaying": {"type": "boolean"},
                    "transportState": {"type": "string"},
                    "ticksPerBeat": {"type": "integer"},
                    "playheadBar": {"type": ["integer", "null"]},
                    "playheadBarOffsetTicks": {"type": ["integer", "null"]},
                    "taskCount": {"type": "integer"},
                    "pluginPids": {"type": "array", "items": {"type": "integer"}},
                    "openWindowCount": {"type": "integer"},
                    // ⭐ `#177` 坑⑪：口径 = 可撤销步数（= 从根算起已应用的 op 数），
                    // 与 `yeban_undo` 响应的 `before.undoable` **逐值相同**。
                    "undoCursor": {
                        "type": "integer",
                        "description": "撤销游标位置 = 从根算起**已应用**的 op 数 = 现在还能撤销的步数（与 `yeban_undo` 的 `before.undoable` 同一个数；`#177` 之前是'已撤销步数'，两个读数会差一个数）"
                    }
                }
            },
            "engine": {
                "type": "object",
                "required": [
                    "sampleRate", "sampleRateSource", "bufferFrames", "bufferSource",
                    "mirrorPresent", "readingsRevision", "sceneLaunchesApplied",
                    "sceneLaunchesAppliedSource"
                ],
                "properties": {
                    "sampleRate": {"type": "integer"},
                    "sampleRateSource": {"type": "string"},
                    "bufferFrames": {"type": ["integer", "null"]},
                    "bufferSource": {"type": "string"},
                    "mirrorPresent": {"type": "boolean"},
                    "integratedLufs": {"type": ["number", "null"]},
                    "momentaryLufs": {"type": ["number", "null"]},
                    "shortTermLufs": {"type": ["number", "null"]},
                    "loudnessRangeLu": {"type": ["number", "null"]},
                    "truePeakDbfs": {"type": ["number", "null"]},
                    "sceneLaunchesApplied": {
                        "type": ["integer", "null"],
                        "description": "引擎累计**真的生效**过的量化场景启动次数（宿主心跳注入；`null` = 这一代引擎拿不到该读数）。`yeban_launch_scene` 的 `queued:true` 之后用它确认到点生效。"
                    },
                    "sceneLaunchesAppliedSource": {"type": "string"},
                    "mirrorSampleRate": {"type": ["integer", "null"]},
                    "sampleRateMatchesMirror": {"type": ["boolean", "null"]},
                    "readingsRevision": {"type": "integer"}
                }
            },
            "track": {"type": ["object", "null"]},
            "sends": {"type": "array", "items": {"type": "object"}},
            "stateSources": {"type": "object"}
        }),
    ))
}

/// `yeban_render_master` 的成功响应 `data`（全部是**实测**数字）。
fn render_master() -> Value {
    // ⭐ `#251`/`#268`：`midiSource` 在 `finish` 之后单独插入 —— 这份 `json!` 的键数已经贴着
    // `serde_json` 宏的递归上限（默认 128），再往字面量里加一个键就会
    // `recursion limit reached`。单独插入与写在字面量里是**同一份** schema。
    let mut schema = finish(
        envelope(
            "yeban_render_master 成功响应的 data",
            "全部字段都是实测值（帧数 / 字节数 / SHA-256 / 块数 / 削波样本数 / 真峰值）。\
             `peak.before|after` 是**归一化前后**的浮点母带样本峰值（⛔ 不是文件峰值）；\
             文件层面的削波读数是顶层 `clippedSamples`，样本间过冲是顶层 `truePeakDbfs`。\
             `truePeakDbfs` / `headroomDb` 在无法测量时是 `null`（不编造 0）。\
             ⭐ `#225`：`warnings[]` 是**结果质量**提示（非阻断），两条来源：\
             ① `clippedSamples > 0` 时恰好一条 `code:\"exportClipping\"`（含 `clippedSamples` / `truePeakDbfs` /\
             `suggestedTrimDb`（公式 `-(ceil(truePeakDbfs * 2) / 2)`）/ `loudestSource` /\
             `hint`）；② 可闻、排了内容、却 `peak == 0.0` 的源由一条 `code:\"silentSources\"`\
             逐条点名（含 `count` / `sources[]` / `hint`）；两条都不出现时是**空数组**。⛔ 它不自动改增益；`sources[].peak` / `.peakDbfs`\
             是逐源峰值（源节点输出，⛔ 不含插入链与路由边增益）。\
             ⭐ `#268`（`ADR-0011`）：顶层 `midiSource` **明说** MIDI 音源是\
             `yeban-engine` 的 `SynthEngine`（`kind:\"engine-instrument\"`、\
             `engineTimbre:true`），与界面/CLI 的 `--export-wav` 同一个音色；\
             带 SFZ 采样器的轨由采样器发声，见 `sources[].sampler`。\
             ⭐ `#315`/`#337`：侧链边有**三个读数**——\
             `sidechainEdgesSummedAsOrdinaryAudio` 数\"侧链边里有几条**真的**被当成普通\
             音频边求和进了母带\"；`#337` 起侧链边不再进音频子图 ⇒ 该字段**恒为 `0`**\
             （⚠ 与 `unsupported` 里的 `sidechainRouting` **不是一回事**：那个键只说\
             \"图里有没有这种数据\"、悬挂边也照报）。\
             `sidechainEdgesInRoutingGraph` 数\"路由图里声明了几条\"。\
             `sidechainEdgesDeclaredWithoutKeyInput` 数\"声明了、但**没有键输入路径**的\
             有几条\" —— 本渲染器没有检波器/键输入消费面，因此它等于声明条数。\
             三者对照读：`0 / 0 / 0` = 没有侧链；`0 / N / N` = N 条侧链声明了、但一条都\
             没有被求值（\"你的侧链现在什么都不做\"）。⛔ 单看 `0` 会以为问题消失了。\
             口径逐字：`sidechainEdgesSummedAsOrdinaryAudio` 说的是\"这条边走了普通音频\
             求和路径\"、⛔ 不是\"+6 dB 了几路\"。\
             ⭐ `ADR-0020`：请求给 `contributions: true` 时**额外**出现\
             `contributions[]`（每条非 master 轨一行：`{trackId, name, alreadyMuted,\
             deltaRmsDbfs, deltaPeakDbfs}`，两次**未归一化**母带逐样本差的 RMS / 峰值，\
             单位 dBFS；逐样本无差异或渲染前已静音 ⇒ null）与 `contributionsBasis`\
             （口径说明 + `renders` = 实际渲染次数）。缺省时**两个键都不出现**\
             ⇒ 不在 `required` 列表里；开关⛔ 不改写盘字节。",
        ),
        &[
            "rendered",
            "request",
            "path",
            "format",
            "container",
            "sampleRate",
            "channels",
            "bitDepth",
            "frames",
            "durationSeconds",
            "blocks",
            "blockSize",
            "bytes",
            "headerBytes",
            "payloadBytes",
            "sha256",
            "masterDigest",
            "longestPathFrames",
            "latencySource",
            "nodeCount",
            "sourceNodes",
            "sources",
            "midiSource",
            "probability",
            "ratchet",
            "audio",
            "peak",
            "clippedSamples",
            "truePeakDbfs",
            "headroomDb",
            "warnings",
            "normalize",
            "loudness",
            "atomic",
            "strategy",
            "dither",
            "bwf",
            "unsupported",
            "unsupportedCounts",
            "sidechainEdgesSummedAsOrdinaryAudio",
            "sidechainEdgesInRoutingGraph",
            "sidechainEdgesDeclaredWithoutKeyInput",
            "projectDigest",
            "renderedAtUnixMs",
        ],
        json!({
            "rendered": {"type": "boolean"},
            "request": {"type": "object"},
            "path": {"type": "string"},
            "format": {"type": "string"},
            "container": {"type": "string"},
            "sampleRate": {"type": "integer"},
            "channels": {"type": "integer"},
            "bitDepth": {"type": "integer"},
            "frames": {"type": "integer"},
            "durationSeconds": {"type": "number"},
            "blocks": {"type": "integer"},
            "blockSize": {"type": "integer"},
            "bytes": {"type": "integer"},
            "headerBytes": {"type": "integer"},
            "payloadBytes": {"type": "integer"},
            "sha256": {"type": "string"},
            "masterDigest": {"type": "string"},
            "longestPathFrames": {"type": "integer"},
            "latencySource": {"type": "string"},
            "nodeCount": {"type": "integer"},
            "sourceNodes": {"type": "array", "items": {"type": "string"}},
            "sources": {"type": "array", "items": {"type": "object"}},
            "probability": {"type": "object"},
            "ratchet": {"type": "object"},
            "audio": {"type": "object"},
            "peak": {"type": "object"},
            "clippedSamples": {"type": "integer"},
            "truePeakDbfs": {"type": ["number", "null"]},
            "headroomDb": {"type": ["number", "null"]},
            "warnings": {"type": "array", "items": render_warning_items()},
            "normalize": {"type": "object"},
            "loudness": {"type": "object"},
            "atomic": {"type": "boolean"},
            "strategy": {"type": "string"},
            "dither": {"type": "string"},
            "bwf": {"type": "object"},
            "unsupported": {"type": "array", "items": {"type": "string"}},
            "unsupportedCounts": {"type": "object"},
            "projectDigest": {"type": "string"},
            "renderedAtUnixMs": {"type": "integer"}
        }),
    );
    if let Value::Object(map) = &mut schema
        && let Some(Value::Object(properties)) = map.get_mut("properties")
    {
        properties.insert("midiSource".to_owned(), json!({"type": "object"}));
        // ⭐ `#315`：两个侧链读数同样是**单独插入**（同一处递归上限），
        // 与 Required 里的两条一一对应。
        properties.insert(
            "sidechainEdgesSummedAsOrdinaryAudio".to_owned(),
            json!({"type": "integer"}),
        );
        properties.insert(
            "sidechainEdgesInRoutingGraph".to_owned(),
            json!({"type": "integer"}),
        );
        properties.insert(
            "sidechainEdgesDeclaredWithoutKeyInput".to_owned(),
            json!({"type": "integer"}),
        );
        // ⭐ `ADR-0020`：可选键 ⇒ ⛔ 不进 `required`（缺省时它们不存在）。
        properties.insert("contributions".to_owned(), json!({"type": "array"}));
        properties.insert("contributionsBasis".to_owned(), json!({"type": "object"}));
    }
    with_commit_model(schema)
}

/// `yeban_undo` 的成功响应 `data`。
fn undo() -> Value {
    steps_receipt(
        "undone",
        "yeban_undo",
        "`before` / `after` 是模型读数（与界面同源）。`cursorPersisted` 恒为 `false`：\
         撤销游标是**会话运行态**，不进 `project.json` / `history.dag`（MODEL-ISO-001）。",
    )
}

/// `yeban_redo` 的成功响应 `data`（与 [`undo`] **共用构造器**）。
fn redo() -> Value {
    steps_receipt(
        "redone",
        "yeban_redo",
        "键集与 `yeban_undo` 逐字镜像（拼装点 `domain::apply_redo` 就是照 `apply_undo` 写的），\
         只有第一个键名是 `redone`。⚠ **`undoneTotal` 这个名字在重做回执里也叫 `undoneTotal`**\
         （来自 `outcome.undone_total`）—— schema 照抄真实键名，⛔ 顺手改名（改名是另一张票）。\
         `cursorPersisted` 恒为 `false`：游标是会话运行态（MODEL-ISO-001）。",
    )
}

/// 撤销 / 重做的**同一份**回执形状：只有"这次到底做没做"那个键名随动词变。
///
/// 为什么做成构造器而不是两份字面量：`apply_redo` 的 `json!` 与 `apply_undo` 的键集是
/// **逐字镜像**（实测：`requestedSteps`/`steps`/`undoneTotal`/`opKinds`/`branch`/
/// `projectDigest`/`cursorPersisted`/`before`/`after` 九个同名同序），抄两遍就等于把
/// "两份清单会漂移"这个本票要治的病搬进生产代码。
fn steps_receipt(verb_key: &str, tool_name: &str, description: &str) -> Value {
    let mut properties = json!({
        "requestedSteps": {"type": "integer"},
        "steps": {"type": "integer"},
        "undoneTotal": {"type": "integer"},
        "opKinds": {"type": "array", "items": {"type": "string"}},
        "branch": {"type": "string"},
        "projectDigest": {"type": "string"},
        "cursorPersisted": {"type": "boolean", "const": false},
        "before": undo_display_schema(),
        "after": undo_display_schema()
    });
    properties
        .as_object_mut()
        .expect("字面量必须是对象")
        .insert(verb_key.to_owned(), json!({"type": "boolean"}));
    let required: Vec<&str> = vec![
        verb_key,
        "requestedSteps",
        "steps",
        "undoneTotal",
        "opKinds",
        "branch",
        "projectDigest",
        "cursorPersisted",
        "before",
        "after",
    ];
    with_commit_model(finish(
        envelope(&format!("{tool_name} 成功响应的 data"), description),
        &required,
        properties,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools;

    #[test]
    fn every_covered_tool_is_registered_and_vice_versa() {
        for name in COVERED_TOOLS {
            assert!(
                tools::tool(name).is_some(),
                "`{name}` 声明了 outputSchema 但没在注册表里"
            );
            assert!(for_tool(name).is_some(), "`{name}` 必须有 schema");
        }
        // 反向：注册表里**有** schema 的工具必须全部出现在清单里（清单是唯一清单）。
        let mut with_schema: Vec<&str> = tools::TOOLS
            .iter()
            .filter(|spec| for_tool(spec.name).is_some())
            .map(|spec| spec.name)
            .collect();
        with_schema.sort_unstable();
        let mut covered = COVERED_TOOLS;
        covered.sort_unstable();
        assert_eq!(
            with_schema, covered,
            "outputSchema 清单与注册表必须双向一致"
        );
        assert_eq!(
            COVERED_TOOLS.len(),
            10,
            "广播面 = #176 五工具 ＋ 提案类 propose_section/set_macro ＋ \
             reject_proposal/redo/edit_automation（[MCP-TOOL-005] 扩面批）"
        );
    }

    #[test]
    fn every_schema_is_an_object_shape_with_required_fields() {
        for name in COVERED_TOOLS {
            let schema = for_tool(name).expect("schema");
            assert_eq!(schema["type"], "object", "`{name}` 顶层必须是 object");
            assert_eq!(
                schema["x-yeban-validates"], VALIDATES,
                "`{name}` 必须声明校验层级（否则判据不知道该核哪一层）"
            );
            let required = schema["required"].as_array().expect("required");
            assert!(!required.is_empty(), "`{name}` 的 required 不得为空");
            assert!(
                !declared_fields(&schema).is_empty(),
                "`{name}` 必须声明 properties"
            );
            // `required` 必须是 `properties` 的子集（否则 schema 自相矛盾）。
            let properties = schema["properties"].as_object().expect("properties");
            for field in required {
                let field = field.as_str().expect("字段名是字符串");
                assert!(
                    properties.contains_key(field),
                    "`{name}` 的 required 字段 `{field}` 没有 properties 声明"
                );
            }
            // 每个 required 字段都必须有 type（否则校验器无从下手）。
            for (field, property) in properties {
                assert!(property.get("type").is_some(), "`{name}.{field}` 缺少 type");
            }
        }
    }

    #[test]
    fn the_validator_actually_rejects_broken_responses() {
        let schema = undo();
        let good = json!({
            // ⭐ `ADR-0032`：`commitModel` 进了 required ⇒ 手写夹具必须带上，
            // 且取值与 `undo` 的真实自述一致（`direct`）。
            "commitModel": "direct",
            "undone": true,
            "requestedSteps": 1,
            "steps": 1,
            "undoneTotal": 1,
            "opKinds": ["SetTempo"],
            "branch": "main",
            "projectDigest": "abc",
            "cursorPersisted": false,
            "before": {
                "branch": "main", "head": null, "commitCount": 0, "branchCount": 1,
                "undone": 0, "undoable": 0, "canUndo": false, "canRedo": false
            },
            "after": {
                "branch": "main", "head": null, "commitCount": 0, "branchCount": 1,
                "undone": 1, "undoable": 1, "canUndo": true, "canRedo": false
            }
        });
        validate_complete(&schema, &good).expect("合法响应必须通过");

        // ① 少一个 required 字段。
        let mut missing = good.clone();
        missing
            .as_object_mut()
            .expect("对象")
            .remove("cursorPersisted");
        let violation = validate(&schema, &missing).expect_err("少 required 必须红");
        assert_eq!(violation.path, "$.cursorPersisted");

        // ② 类型不对（`undoneTotal` 声明 integer，给字符串）。
        let mut wrong = good.clone();
        wrong["undoneTotal"] = Value::from("1");
        let violation = validate(&schema, &wrong).expect_err("类型不对必须红");
        assert_eq!(violation.path, "$.undoneTotal");

        // ③ 顶层不是对象。
        let violation = validate(&schema, &json!([])).expect_err("数组必须红");
        assert_eq!(violation.path, "$");

        // ④ 声明的字段缺席（`validate_complete` 的那一条）：
        //    `yeban_query_engine_state.sends` 只在点名音轨时出现 ⇒ 不在 required 里。
        //    ⚠ `session` / `engine` 必须**写全**（照 `domain::engine_state` 的
        //    `session_value` / `engine_value` 现读）：s29 起校验器递归进嵌套对象，
        //    旧的 `{}` 占位在"只看顶层"时代是安全的，现在是**假夹具**（run `41228291334` 就红在这里）。
        let engine_schema = query_engine_state();
        let without_sends = json!({
            "commitModel": "none",
            "session": {
                "playheadTicks": 0,
                "isPlaying": false,
                "transportState": "stopped",
                "ticksPerBeat": 960,
                "playheadBar": 0,
                "playheadBarOffsetTicks": 0,
                "taskCount": 0,
                "pluginPids": [],
                "openWindowCount": 0,
                "undoCursor": 0
            },
            "engine": {
                "sampleRate": 48_000,
                "sampleRateSource": "project.audio_config.sample_rate",
                "bufferFrames": null,
                "bufferSource": "unavailable",
                "mirrorPresent": false,
                "readingsRevision": 0,
                "sceneLaunchesApplied": null,
                "sceneLaunchesAppliedSource": "unavailable"
            },
            "track": null,
            "stateSources": {
                "playhead": "yeban_model::SessionRuntimeState",
                "sampleRate": "project.audio_config.sample_rate"
            }
        });
        assert!(
            validate(&engine_schema, &without_sends).is_ok(),
            "sends 不是 required"
        );
        validate_complete(&engine_schema, &without_sends).expect_err("声明了就必须出现");
    }

    #[test]
    fn a_missing_schema_is_none_not_an_empty_object() {
        // ⛔ 不广播空 schema：`None` ⇒ `tools/list` 里**没有** outputSchema 这个键。
        assert!(for_tool("yeban_open_project").is_none());
        assert!(for_tool("yeban_list_proposals").is_none());
        assert!(for_tool("yeban_query_project").is_none());
    }

    /// 递归必须真的进到**数组元素里面**，并把下标写进路径（s29 的核心判据）。
    ///
    /// 在此之前的校验器只核到"它是 object"，于是 `items` 里声明的 `code.enum`、逐键 `type`、
    /// 该层的 `required` **全是装饰**（台账 §22.16 的现读：`warnings` 声明了 `items` 而 `items`
    /// 里的东西一次都没被核对）。⚠ 这条判据必须用**真实广播 schema** 的节点：
    /// 手造一份 schema 的话，即使校验器仍然不递归，它也照样绿 —— 那正是"假验证"。
    #[test]
    fn the_recursion_reaches_inside_array_items_and_indexes_the_path() {
        let warnings =
            for_tool("yeban_propose_section").expect("schema")["properties"]["warnings"].clone();

        // ① 元素内部的键类型：`knownParams` 声明为 string 数组 ⇒ 第 2 个元素是数字必须红。
        let violation = check_declared_field(
            &warnings,
            &json!([{"code": "unrecognisedEffectParam", "knownParams": ["cutoff", 5], "hint": "x"}]),
            "$.warnings",
        )
        .expect_err("knownParams 的元素不是字符串, 必须红");
        assert_eq!(
            violation.path, "$.warnings[0].knownParams[1]",
            "递归必须把两层下标都写进路径: {violation}"
        );

        // ② 元素那一层自己的 `required`：少 `hint` 必须红，红在带下标的元素路径上。
        let violation = check_declared_field(
            &warnings,
            &json!([{"code": "deviceOnSilentTrack", "trackId": "t1"}]),
            "$.warnings",
        )
        .expect_err("items 层的 required 缺席, 必须红");
        assert_eq!(violation.path, "$.warnings[0].hint", "{violation}");

        // ③ 正反两路：两条构造点的**真实形状**都必须过（只留反面判据会把假广播也验成绿）。
        check_declared_field(
            &warnings,
            &json!([
                {"code": "unrecognisedEffectParam", "deviceName": "EQ", "paramName": "cuttoff",
                 "knownParams": ["cutoff"], "hint": "引擎不认这个拼写"},
                {"code": "deviceOnSilentTrack", "trackId": "t1", "track": "Bass", "deviceId": "d1",
                 "deviceName": "Amp", "placements": 2, "soundingPlacements": 0, "notes": 0,
                 "triggeringNotes": 0, "hint": "该轨没有任何片段"}
            ]),
            "$.warnings",
        )
        .expect("两条构造点的真实形状必须通过");
    }

    /// 越名册的 `warnings[].code` 必须被**自己广播的 schema** 拒掉（逐工具子集，s29）。
    #[test]
    fn a_warning_code_outside_the_broadcast_subset_is_rejected() {
        // 提案族三个广播工具：母带那两枚对它们是越界。
        for tool in [
            "yeban_propose_section",
            "yeban_edit_notes",
            "yeban_set_macro",
        ] {
            for code in ["exportClipping", "silentSources", "sfzSamplesUnreadable"] {
                let violation =
                    broadcast_warning_check(tool, &json!([{"code": code, "hint": "x"}]))
                        .err()
                        .unwrap_or_else(|| panic!("`{tool}` 广播了子集, 越界的 `{code}` 必须红"));
                assert_eq!(
                    violation.path, "$.warnings[0].code",
                    "`{tool}` / `{code}`: {violation}"
                );
            }
        }
        // 母带反过来：提案族那两枚对它是越界。
        for code in ["deviceOnSilentTrack", "unrecognisedEffectParam"] {
            let violation = broadcast_warning_check(
                "yeban_render_master",
                &json!([{"code": code, "hint": "x"}]),
            )
            .err()
            .unwrap_or_else(|| panic!("render_master 广播了母带子集, 提案族的 `{code}` 必须红"));
            assert_eq!(
                violation.path, "$.warnings[0].code",
                "`{code}`: {violation}"
            );
        }
    }

    /// 广播的子集 ↔ 契约名册（`definitions.WarningCode.enum`）的对账。
    ///
    /// 与 `scripts/guards/check_warning_codes.py` 的分工：那条守卫核"**实现侧构造点** == 契约 enum"
    /// （集合相等），这一条核"**广播侧声明** ⊆ 契约 enum"再加逐工具子集不许退化。
    /// 两边合起来才把"名册"钉成一件事：契约、实现、`tools/list` 三处不可能各说一套。
    #[test]
    fn the_broadcast_warning_subsets_match_the_measured_call_chain() {
        let contract_raw = include_str!("../../../schemas/mcp-tools.schema.json");
        let contract_value: Value =
            serde_json::from_str(contract_raw).expect("契约必须是合法 JSON");
        let contract: std::collections::BTreeSet<String> =
            contract_value["definitions"]["WarningCode"]["enum"]
                .as_array()
                .expect("契约必须有 WarningCode.enum")
                .iter()
                .map(|code| code.as_str().expect("code 是字符串").to_owned())
                .collect();
        assert_eq!(contract.len(), 5, "契约名册现读 5 枚: {contract:?}");

        let broadcast = |tool: &str| -> std::collections::BTreeSet<String> {
            let schema = for_tool(tool).expect("schema");
            let enum_value =
                &schema["properties"]["warnings"]["items"]["properties"]["code"]["enum"];
            enum_value
                .as_array()
                .unwrap_or_else(|| panic!("`{tool}` 广播的 warnings[].code 必须带 enum"))
                .iter()
                .map(|code| code.as_str().expect("code 是字符串").to_owned())
                .collect()
        };

        let proposal: std::collections::BTreeSet<String> = PROPOSAL_WARNING_CODES
            .iter()
            .map(|code| (*code).to_owned())
            .collect();
        let render: std::collections::BTreeSet<String> = RENDER_WARNING_CODES
            .iter()
            .map(|code| (*code).to_owned())
            .collect();
        for tool in [
            "yeban_propose_section",
            "yeban_edit_notes",
            "yeban_set_macro",
        ] {
            assert_eq!(
                broadcast(tool),
                proposal,
                "`{tool}` 的子集必须逐字等于提案族名册"
            );
        }
        assert_eq!(
            broadcast("yeban_render_master"),
            render,
            "母带的子集必须逐字等于渲染族名册"
        );

        // 广播侧不得发明契约之外的拼写。
        let mut all: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for tool in COVERED_TOOLS {
            let schema = for_tool(tool).expect("schema");
            if schema["properties"]["warnings"]["items"]["properties"]["code"]["enum"]
                == Value::Null
            {
                continue;
            }
            all.extend(broadcast(tool));
        }
        let invented: Vec<String> = all.difference(&contract).cloned().collect();
        assert!(
            invented.is_empty(),
            "广播侧发明了契约外的 code: {invented:?}"
        );

        // ⭐ 第五枚**故意**不在任何子集里（它只出自两个扩展工具，而扩展工具不广播 outputSchema）。
        // 这一条把那句话钉成读数：契约与广播的差集必须**恰好**是它。
        let unbroadcast: Vec<String> = contract.difference(&all).cloned().collect();
        assert_eq!(
            unbroadcast,
            ["sfzSamplesUnreadable".to_owned()],
            "契约里唯一不该被广播的 code 必须是 sfzSamplesUnreadable（其余四枚都必须广播）"
        );
    }

    /// 用某工具**广播的** `warnings` 节点核一批警告（判据共用的小入口，走真实递归路径）。
    fn broadcast_warning_check(tool: &str, warnings: &Value) -> Result<(), Violation> {
        let schema = for_tool(tool).expect("schema")["properties"]["warnings"].clone();
        check_declared_field(&schema, warnings, "$.warnings")
    }
}
