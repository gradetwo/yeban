//! `yeban_query_project` 的**稀疏视图**与**分页** [MCP-TOOL-004]。
//!
//! ## 两个独立的口子，因为它们是两类不同的事故
//!
//! 1. **字段选择器**（`fields`）：白名单外的选择器一律 `INVALID_FIELD_SELECTOR`。
//!    "静默返回空对象"是最坏的一种成功 —— Agent 会以为自己拿到了数据。
//! 2. **分页**（`limit` / `offset`）：实体的扁平索引被切片，因此响应体积与工程规模
//!    **解耦**。索引项只带 `{kind, id, name}`，**从不**包含音符集 ——
//!    一个百万音符的工程与一个空工程的响应体积在同一量级。
//!
//! ## 为什么选择器是白名单而不是自由 JSONPath
//!
//! 白名单可以**静态**对账：`SELECTORS` 里的每一项都必须能在
//! `YebanProjectV1` 的序列化结果里取到值（判据
//! `every_selector_resolves_against_a_filled_project`）。自由 JSONPath 做不到这件事，
//! 而"选择器语法"本身会变成第二份需要与 `schemas/project.schema.json` 同步的契约。

use serde_json::{Map, Value};

use yeban_model::YebanProjectV1;

use super::error::Fault;
use crate::tools::ErrorCode;

/// 允许的字段选择器（白名单；`snake_case` 与 `schemas/project.schema.json` 一致）。
///
/// 两类形态：
/// - **顶层字段**（`tracks` / `bpm` / …）；
/// - **一层下钻**（`tracks.id`）：根是"实体集合对象"时，按 BTreeMap 键序展开成
///   `{id: {leaf: value}}`；根是普通对象（如 `routing_graph`）时，直接取 `leaf`。
pub const SELECTORS: [&str; 24] = [
    "id",
    "title",
    "author",
    "bpm",
    "schema_version",
    "min_reader_version",
    "writer_version",
    "rng_seed",
    "master_bus_track_id",
    "time_signature",
    "audio_config",
    "transport",
    "metadata",
    "tracks",
    "sections",
    "scenes",
    "clip_pool",
    "assets",
    "routing_graph",
    "tracks.id",
    "tracks.name",
    "tracks.kind",
    "sections.name",
    "clip_pool.name",
];

/// `limit` 缺省值。
pub const DEFAULT_LIMIT: u64 = 100;

/// `limit` 上限（超过则**夹紧**并在 `page.limitClamped` 里如实报告）。
pub const MAX_LIMIT: u64 = 1000;

/// 一次解析并校验过的查询请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    /// 分页大小（已夹紧到 `1..=`[`MAX_LIMIT`]）。
    pub limit: u64,
    /// 分页偏移。
    pub offset: u64,
    /// 字段选择器（去重、保序；空表示"全量顶层视图"）。
    pub fields: Vec<String>,
    /// 调用方给的 `limit` 是否被夹紧。
    pub limit_clamped: bool,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            offset: 0,
            fields: Vec::new(),
            limit_clamped: false,
        }
    }
}

/// 从工具实参解析查询请求。
///
/// # Errors
///
/// - `limit` / `offset` 不是非负整数，或 `limit == 0` → `INVALID_PARAMETER_RANGE`；
/// - `fields` 里有白名单外的选择器（或元素不是字符串）→ `INVALID_FIELD_SELECTOR`。
pub fn parse(arguments: &Map<String, Value>) -> Result<Query, Fault> {
    let mut query = Query::default();
    if let Some(limit) = read_u64(arguments, "limit")? {
        if limit == 0 {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`limit` 必须 >= 1（要空结果请用 offset 越界）",
            ));
        }
        if limit > MAX_LIMIT {
            query.limit_clamped = true;
            query.limit = MAX_LIMIT;
        } else {
            query.limit = limit;
        }
    }
    if let Some(offset) = read_u64(arguments, "offset")? {
        query.offset = offset;
    }
    if let Some(value) = arguments.get("fields") {
        query.fields = parse_fields(value)?;
    }
    Ok(query)
}

/// 读一个非负整数参数（负数与溢出都拒绝，而不是静默当 0）。
fn read_u64(arguments: &Map<String, Value>, name: &str) -> Result<Option<u64>, Fault> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    if let Some(number) = value.as_u64() {
        return Ok(Some(number));
    }
    Err(Fault::domain(
        ErrorCode::InvalidParameterRange,
        format!("`{name}` 必须是非负整数, 实际收到 {value}"),
    ))
}

/// 校验并归一化字段选择器（去重、保序）。
fn parse_fields(value: &Value) -> Result<Vec<String>, Fault> {
    let Value::Array(items) = value else {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidFieldSelector,
            "`fields` 必须是字符串数组",
            serde_json::json!({ "received": value.clone() }),
        ));
    };
    let mut fields: Vec<String> = Vec::new();
    for item in items {
        let Some(name) = item.as_str() else {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidFieldSelector,
                format!("`fields` 的元素必须是字符串, 实际收到 {item}"),
                serde_json::json!({ "availableSelectors": SELECTORS }),
            ));
        };
        if !SELECTORS.contains(&name) {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidFieldSelector,
                format!("未知字段选择器 `{name}`"),
                serde_json::json!({ "unknown": name, "availableSelectors": SELECTORS }),
            ));
        }
        if !fields.iter().any(|known| known == name) {
            fields.push(name.to_owned());
        }
    }
    Ok(fields)
}

/// 返回 `data`：`{project, entities, page}`。
///
/// # Errors
///
/// 工程无法序列化成 JSON（理论上不会）。
pub fn data(project: &YebanProjectV1, query: &Query) -> Result<Value, Fault> {
    let index = entity_index(project);
    let total = index.len();
    let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
    let start = usize::try_from(query.offset)
        .unwrap_or(usize::MAX)
        .min(total);
    let end = start.saturating_add(limit).min(total);
    let page = index
        .get(start..end)
        .map_or_else(Vec::new, <[Value]>::to_vec);

    let mut root = Map::new();
    root.insert("project".to_owned(), project_view(project, &query.fields)?);
    root.insert("entities".to_owned(), Value::Array(page));
    root.insert(
        "page".to_owned(),
        serde_json::json!({
            "limit": query.limit,
            "offset": query.offset,
            "total": total,
            "returned": end.saturating_sub(start),
            "hasMore": end < total,
            "limitClamped": query.limit_clamped,
        }),
    );
    Ok(Value::Object(root))
}

/// 稀疏视图：`fields` 为空时是工程的**全量顶层视图**。
///
/// # Errors
///
/// 工程无法序列化成 JSON（理论上不会）。
pub fn project_view(project: &YebanProjectV1, fields: &[String]) -> Result<Value, Fault> {
    let full = serde_json::to_value(project).map_err(|error| {
        Fault::domain(ErrorCode::IoError, format!("序列化工程视图失败: {error}"))
    })?;
    let object = full.as_object().cloned().unwrap_or_default();
    if fields.is_empty() {
        return Ok(Value::Object(object));
    }
    let mut out = Map::new();
    for selector in fields {
        match selector.split_once('.') {
            None => {
                if let Some(value) = object.get(selector) {
                    out.insert(selector.clone(), value.clone());
                }
            }
            Some((root, leaf)) => {
                let collected = match object.get(root) {
                    // 普通对象 + 一层下钻（如 `routing_graph.nodes`）。
                    Some(Value::Object(inner)) if inner.contains_key(leaf) => {
                        let mut merged = out
                            .get(root)
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        merged.insert(leaf.to_owned(), inner[leaf].clone());
                        merged
                    }
                    // 实体集合对象 + 一层下钻（如 `tracks.id`）：按键序展开。
                    Some(Value::Object(collection)) => {
                        let mut merged = out
                            .get(root)
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        for (key, entry) in collection {
                            let Some(leaf_value) =
                                entry.as_object().and_then(|entry| entry.get(leaf))
                            else {
                                continue;
                            };
                            let mut record = merged
                                .get(key)
                                .and_then(Value::as_object)
                                .cloned()
                                .unwrap_or_default();
                            record.insert(leaf.to_owned(), leaf_value.clone());
                            merged.insert(key.clone(), Value::Object(record));
                        }
                        merged
                    }
                    _ => Map::new(),
                };
                out.insert(root.to_owned(), Value::Object(collected));
            }
        }
    }
    Ok(Value::Object(out))
}

/// 实体索引：确定性顺序（`tracks` → `sections` → `scenes` → `clip_pool`，组内按身份升序）。
///
/// **刻意不含音符** —— 这是"防止海量音符撑爆 Agent 上下文"的落点。
#[must_use]
pub fn entity_index(project: &YebanProjectV1) -> Vec<Value> {
    let mut index = Vec::with_capacity(
        project.tracks.len()
            + project.sections.len()
            + project.scenes.len()
            + project.clip_pool.len(),
    );
    let mut push = |kind: &str, id: String, name: &str, extra: Option<(&str, Value)>| {
        let mut record = Map::new();
        record.insert("kind".to_owned(), Value::from(kind));
        record.insert("id".to_owned(), Value::from(id));
        record.insert("name".to_owned(), Value::from(name));
        if let Some((key, value)) = extra {
            record.insert(key.to_owned(), value);
        }
        index.push(Value::Object(record));
    };
    for (id, track) in &project.tracks {
        push(
            "track",
            id.to_canonical_string(),
            &track.name,
            Some(("trackKind", Value::from(format!("{:?}", track.kind)))),
        );
    }
    for (id, section) in &project.sections {
        push("section", id.to_canonical_string(), &section.name, None);
    }
    for (id, scene) in &project.scenes {
        push("scene", id.to_canonical_string(), &scene.name, None);
    }
    for (id, clip) in &project.clip_pool {
        push("clip", id.to_canonical_string(), &clip.name, None);
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::samples::filled_project;

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn every_selector_resolves_against_a_filled_project() {
        let project = filled_project();
        for selector in SELECTORS {
            let value = project_view(&project, &[selector.to_owned()]).expect("视图");
            let root = selector.split('.').next().expect("根");
            assert!(
                value.get(root).is_some(),
                "选择器 `{selector}` 在序列化结果里取不到值 —— 白名单与数据模型漂移了"
            );
        }
    }

    #[test]
    fn unknown_selector_is_invalid_field_selector() {
        let fault = parse(&args(serde_json::json!({"fields": ["tracks[0]"]})))
            .expect_err("非法选择器必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidFieldSelector));
        let value = fault.into_result().expect("带内错误");
        assert_eq!(value["error"]["code"], "INVALID_FIELD_SELECTOR");
        assert!(value["error"]["data"]["availableSelectors"].is_array());

        // 元素不是字符串也是选择器问题。
        let fault = parse(&args(serde_json::json!({"fields": [1]}))).expect_err("必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidFieldSelector));
        // fields 不是数组。
        let fault = parse(&args(serde_json::json!({"fields": "tracks"}))).expect_err("必须被拒");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidFieldSelector));
    }

    #[test]
    fn limit_and_offset_are_validated_and_clamped() {
        assert_eq!(
            parse(&args(serde_json::json!({}))).expect("缺省"),
            Query::default()
        );
        let fault = parse(&args(serde_json::json!({"limit": 0}))).expect_err("0 非法");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let fault = parse(&args(serde_json::json!({"limit": -3}))).expect_err("负数非法");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let fault = parse(&args(serde_json::json!({"offset": -1}))).expect_err("负数非法");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let clamped = parse(&args(serde_json::json!({"limit": 100_000}))).expect("夹紧");
        assert_eq!(clamped.limit, MAX_LIMIT);
        assert!(clamped.limit_clamped);
    }

    #[test]
    fn pagination_is_deterministic_and_bounded() {
        let project = filled_project();
        let all = data(&project, &Query::default()).expect("全量");
        let total = all["page"]["total"].as_u64().expect("total");
        assert!(total > 0);
        assert_eq!(all["page"]["returned"], total);
        assert_eq!(all["page"]["hasMore"], false);

        let first = data(
            &project,
            &Query {
                limit: 2,
                offset: 0,
                ..Query::default()
            },
        )
        .expect("第一页");
        let second = data(
            &project,
            &Query {
                limit: 2,
                offset: 2,
                ..Query::default()
            },
        )
        .expect("第二页");
        assert_eq!(first["page"]["returned"], 2);
        assert_ne!(first["entities"][0], second["entities"][0]);
        // 越界 offset 不是错误, 只是空页。
        let beyond = data(
            &project,
            &Query {
                offset: total + 10,
                ..Query::default()
            },
        )
        .expect("越界");
        assert_eq!(beyond["page"]["returned"], 0);
        assert_eq!(beyond["page"]["hasMore"], false);
    }

    #[test]
    fn notes_never_enter_the_entity_index() {
        let project = filled_project();
        let text = serde_json::to_string(&entity_index(&project)).expect("序列化");
        assert!(!text.contains("start_tick"), "索引不得含音符字段: {text}");
        assert!(text.contains("\"kind\":\"clip\""));
    }

    #[test]
    fn sparse_view_keeps_only_the_requested_selectors() {
        let project = filled_project();
        let view =
            project_view(&project, &["bpm".to_owned(), "tracks.name".to_owned()]).expect("视图");
        assert!(view.get("bpm").is_some());
        assert!(view.get("title").is_none(), "未请求的顶层字段不得出现");
        let tracks = view["tracks"].as_object().expect("tracks 对象");
        assert!(!tracks.is_empty());
        for (_, record) in tracks {
            let keys: Vec<&str> = record
                .as_object()
                .expect("记录")
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(keys, vec!["name"], "只允许被请求的那一层下钻字段");
        }
        // routing_graph 是普通对象, 一层下钻取数组。
        let routing = project_view(&project, &["routing_graph.nodes".to_owned()]).expect("视图");
        assert!(routing["routing_graph"]["nodes"].is_array());
    }

    #[test]
    fn duplicate_selectors_are_deduplicated_and_order_preserved() {
        let query = parse(&args(serde_json::json!({
            "fields": ["bpm", "title", "bpm"]
        })))
        .expect("解析");
        assert_eq!(query.fields, vec!["bpm".to_owned(), "title".to_owned()]);
    }
}
