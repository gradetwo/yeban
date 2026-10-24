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
//! 五个 schema 的字段逐个来自实现里的响应拼装点，判据用**真实 `tools/call` 响应**
//! 反校（[`validate`] 核 `required` ＋ 顶层 `type` ＋ 已声明字段的类型；
//! [`validate_complete`] 额外核"声明的每个字段名都出现"）。
//!
//! ⚠ 一个已知的**两种成功形状**：`yeban_merge_proposal` 在"这份提案已经合并过"
//! 时返回一条**更短**的记录（只有 `merged` / `alreadyMerged` / `appliedOps` /
//! `proposal`，没有 `commit` / `baseCommitMoved` / `projectDigest`）。
//! 因此那份 schema 的 `required` 只列**两种形状都有**的字段，并在 `description`
//! 里逐字说明缺的是哪几个 —— 写成"理想形状"会让幂等合并的那次响应**违反自己的 schema**。
//!
//! ## 校验器只做三件事（并**被真的跑**）
//!
//! 1. 顶层 `type`；
//! 2. `required` 里的每个名字都在 `data` 里；
//! 3. 已声明且出现的字段，其 `type` 与声明一致（`integer` 与 `number` 分开；
//!    `["number","null"]` 这类联合类型按"任一匹配"处理）。
//!
//! 它**不是**通用 JSON Schema 实现（不处理 `$ref` / `oneOf` / `minimum`）；
//! 本模块的 schema 刻意只用上述子集，好让"声明的"与"可执行的"是同一件事。

use serde_json::{Map, Value, json};

/// 这些 schema 校验的层级（`ToolResponse` 信封里的 `data`）。
pub const VALIDATES: &str = "result.data";

/// 已经声明 `outputSchema` 的五个主力工具（`#176` 的范围）。
///
/// 顺序 = 注册表顺序里的出现顺序；判据要求这份清单与真实广播面**双向一致**
/// （每个列出的工具都有 schema；每个有 schema 的工具都在清单里）。
pub const COVERED_TOOLS: [&str; 5] = [
    "yeban_edit_notes",
    "yeban_merge_proposal",
    "yeban_query_engine_state",
    "yeban_render_master",
    "yeban_undo",
];

/// 按工具名取成功响应的 `data` schema；未声明时为 `None`。
///
/// `None` 是**诚实的缺省**：没写 schema 的工具**不广播** `outputSchema` 键，
/// 而不是广播一个空对象让客户端以为"没有字段"。
#[must_use]
pub fn for_tool(name: &str) -> Option<Value> {
    match name {
        "yeban_edit_notes" => Some(edit_notes()),
        "yeban_merge_proposal" => Some(merge_proposal()),
        "yeban_query_engine_state" => Some(query_engine_state()),
        "yeban_render_master" => Some(render_master()),
        "yeban_undo" => Some(undo()),
        _ => None,
    }
}

/// 校验失败的一处读数（**路径 + 原因**，不是布尔）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// 出问题的 JSON 路径（`$` 开头；数组下标不在本校验器的范围内）。
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

/// 用一份 schema 校验一份 `data`：顶层 `type` ＋ `required` ＋ 已声明字段的类型。
///
/// # Errors
///
/// 上述任一条不成立。
pub fn validate(schema: &Value, data: &Value) -> Result<(), Violation> {
    check_type(schema, data, "$")?;
    let object = match data.as_object() {
        Some(object) => object,
        None => {
            return Err(Violation::new(
                "$",
                format!("顶层必须是对象, 实际是 {}", json_type_name(data)),
            ));
        }
    };
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for name in required {
            let name = name.as_str().unwrap_or_default();
            if !object.contains_key(name) {
                return Err(Violation::new(
                    format!("$.{name}"),
                    "`required` 声明了这个字段, 但响应里没有",
                ));
            }
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            if let Some(value) = object.get(name) {
                check_type(property, value, &format!("$.{name}"))?;
            }
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

fn check_type(schema: &Value, value: &Value, path: &str) -> Result<(), Violation> {
    let Some(kind) = schema.get("type") else {
        return Ok(());
    };
    let matches = match kind {
        Value::String(kind) => json_type_matches(kind, value),
        Value::Array(kinds) => kinds
            .iter()
            .filter_map(Value::as_str)
            .any(|kind| json_type_matches(kind, value)),
        _ => true,
    };
    if matches {
        Ok(())
    } else {
        Err(Violation::new(
            path,
            format!(
                "schema 声明 type = {kind}, 实际是 {}",
                json_type_name(value)
            ),
        ))
    }
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

/// 提案记录的**摘要**形状（[`crate::domain::proposal::Proposal::summary`]）。
fn proposal_summary_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "proposalId", "branch", "kind", "title", "status", "baseCommit", "headCommit",
            "mergeCommit", "createdAt", "resolvedAt", "resolution", "opCount", "opKinds"
        ],
        "properties": {
            "proposalId": {"type": "string"},
            "branch": {"type": "string"},
            "kind": {"type": "string"},
            "title": {"type": "string"},
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

/// `yeban_edit_notes`（以及其余两个提案类工具）的成功响应 `data`。
///
/// 来源：`domain::apply_propose` 的 `ToolResponse::success(json!({…}))`。
fn edit_notes() -> Value {
    finish(
        envelope(
            "yeban_edit_notes 成功响应的 data",
            "本调用**永远**只创建一条隔离提案分支（`pendingMerge: true` / `projectUnchanged: true`）；\
             `proposal` 是**摘要**（不含逐条 `ops`，除非 `includeOps: true`）。\
             `warnings[]` 逐条报出**非阻断**问题：`code:\"unrecognisedEffectParam\"` = 引擎不认但又不是\
             拼错的设备参数名（带 `deviceName` / `paramName`）；`code:\"deviceOnSilentTrack\"` = 设备装到了\
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
            "pendingMerge": {"type": "boolean", "const": true},
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
                "items": {
                    "type": "object",
                    "required": ["code", "hint"],
                    "properties": {
                        "code": {"type": "string"},
                        "deviceName": {"type": "string"},
                        "paramName": {"type": "string"},
                        "trackId": {"type": "string"},
                        "track": {"type": "string"},
                        "deviceId": {"type": "string"},
                        "placements": {"type": "integer"},
                        "soundingPlacements": {"type": "integer"},
                        "notes": {"type": "integer"},
                        "triggeringNotes": {"type": "integer"},
                        "hint": {"type": "string"}
                    }
                }
            },
            "proposal": proposal_summary_schema()
        }),
    )
}

/// `yeban_merge_proposal` 的成功响应 `data`（**两种形状**）。
fn merge_proposal() -> Value {
    finish(
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
    )
}

/// `yeban_query_engine_state` 的成功响应 `data`（只读）。
fn query_engine_state() -> Value {
    finish(
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
    )
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
             ⭐ `#225`：`warnings[]` 是**结果质量**提示（非阻断）—— `clippedSamples > 0` 时\
             恰好一条 `code:\"exportClipping\"`（含 `clippedSamples` / `truePeakDbfs` /\
             `suggestedTrimDb`（公式 `-(ceil(truePeakDbfs * 2) / 2)`）/ `loudestSource` /\
             `hint`），否则**空数组**。⛔ 它不自动改增益；`sources[].peak` / `.peakDbfs`\
             是逐源峰值（源节点输出，⛔ 不含插入链与路由边增益）。\
             ⭐ `#268`（`ADR-0011`）：顶层 `midiSource` **明说** MIDI 音源是\
             `yeban-engine` 的 `SynthEngine`（`kind:\"engine-instrument\"`、\
             `engineTimbre:true`），与界面/CLI 的 `--export-wav` 同一个音色；\
             带 SFZ 采样器的轨由采样器发声，见 `sources[].sampler`。\
             ⭐ `#315`：`sidechainEdgesSummedAsOrdinaryAudio` / `sidechainEdgesInRoutingGraph`\
             是**读数**——前者数\"侧链边里有几条**真的**被当成普通音频边求和进了母带\"\
             （⚠ 与 `unsupported` 里的 `sidechainRouting` **不是一回事**：那个键只说\
             \"图里有没有这种数据\"、悬挂边也照报；本字段在悬挂边上读 `0`），后者数\
             \"路由图里声明了几条\"。两者对照读：**声明了 N 条、求和了 M 条**。\
             口径逐字：`M` 说的是\"这条边走了普通音频求和路径\"、⛔ 不是\"+6 dB 了几路\"\
             （只有一条侧链边时 `M = 1`，而其音频与同形 `TrackToBus` 基线**逐字节相同**）。",
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
            "warnings": {"type": "array", "items": {"type": "object"}},
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
    }
    schema
}

/// `yeban_undo` 的成功响应 `data`。
fn undo() -> Value {
    finish(
        envelope(
            "yeban_undo 成功响应的 data",
            "`before` / `after` 是模型读数（与界面同源）。`cursorPersisted` 恒为 `false`：\
             撤销游标是**会话运行态**，不进 `project.json` / `history.dag`（MODEL-ISO-001）。",
        ),
        &[
            "undone",
            "requestedSteps",
            "steps",
            "undoneTotal",
            "opKinds",
            "branch",
            "projectDigest",
            "cursorPersisted",
            "before",
            "after",
        ],
        json!({
            "undone": {"type": "boolean"},
            "requestedSteps": {"type": "integer"},
            "steps": {"type": "integer"},
            "undoneTotal": {"type": "integer"},
            "opKinds": {"type": "array", "items": {"type": "string"}},
            "branch": {"type": "string"},
            "projectDigest": {"type": "string"},
            "cursorPersisted": {"type": "boolean", "const": false},
            "before": undo_display_schema(),
            "after": undo_display_schema()
        }),
    )
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
        assert_eq!(COVERED_TOOLS.len(), 5, "#176 的范围：至少 5 个主力工具");
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
        let engine_schema = query_engine_state();
        let without_sends = json!({
            "session": {},
            "engine": {},
            "track": null,
            "stateSources": {}
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
}
