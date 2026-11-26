//! `yeban_query_project` 的**稀疏视图**与**分页** [MCP-TOOL-004]。
//!
//! ## 两个独立的口子，因为它们是两类不同的事故
//!
//! 1. **字段选择器**（`fields`）：白名单外的选择器一律 `INVALID_FIELD_SELECTOR`。
//!    "静默返回空对象"是最坏的一种成功 —— Agent 会以为自己拿到了数据。
//! 2. **分页**（`limit` / `offset`）：实体的扁平索引被切片，因此响应体积与工程规模
//!    **解耦**。索引项只带 `{kind, id, name}`，**从不**包含音符集 ——
//!    一个百万音符的工程与一个空工程的响应体积在同一量级。
//!    `project.clip_pool` 默认也只给**摘要**（`id` / `name` / `kind` / `noteCount` /
//!    `endTick`），⛔ 不再把每个音符序列化进响应；要全量音符必须**显式**选
//!    `clip_pool.full`（该选择器与音符数线性，适用于明确知道自己在拉大对象的调用方）。
//!    "同一量级"由单元判据
//!    `a_twenty_thousand_note_project_stays_below_sixty_four_kilobytes` 与
//!    `response_bytes_do_not_grow_with_the_number_of_notes` 钉住（20k 实测 4,787 B、
//!    100k 实测 4,789 B；修前 20k `limit:1` 为 3,130,680 B）。
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
///
/// ⭐ [G15] 例外：`sections.timeline` 是**派生视图** —— 除了根键 `sections`
/// （其值与单独选择 `sections` 逐字节同形，满足"每个选择器都取得到根键"的单元判据），
/// 还输出一个独立顶层键 `sectionsTimeline`（按 `(start_tick, id)` 升序的段落数组）。
/// `sections` 的原始键序是**身份序**（ULID 插入序，⛔ 不是时间序），调用方本来必须自己重排。
pub const SELECTORS: [&str; 32] = [
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
    "tempo_map",
    "time_signature_map",
    "key_signature",
    "audio_config",
    "transport",
    "metadata",
    "tracks",
    // UI 轨道交互线新增：`track_order` 是 schema 顶层键，须进白名单
    //（`view_selectors` 判据双向对账：schema 有而白名单无 ⇒ 静默漏字段）。
    "track_order",
    // C3 歌词轨新增：`lyrics` 是 schema 顶层键，须进白名单（同上）。
    "lyrics",
    "sections",
    "scenes",
    "clip_pool",
    "assets",
    "embedded_samples",
    "routing_graph",
    "tracks.id",
    "tracks.name",
    "tracks.kind",
    "sections.name",
    "sections.timeline",
    "clip_pool.name",
    // ⭐ T-FIX-366：全量音符的**显式**出口。默认 `clip_pool` 只给摘要；
    // 这个选择器才返回带 `content.Midi.notes` 的完整片段池（体积与音符数线性）。
    "clip_pool.full",
];

/// `limit` 缺省值。
pub const DEFAULT_LIMIT: u64 = 100;

/// `limit` 上限（超过则**夹紧**并在 `page.limitClamped` 里如实报告）。
pub const MAX_LIMIT: u64 = 1000;

/// `tools/list` 里 `yeban_query_project.fields.items` 的 schema（`#177` 坑⑩）。
///
/// 此前 `fields` 只广播 `{"type": "array"}` ⇒ 合法字段名**无从得知**，外部 Agent
/// 只能猜，而猜错一律 `INVALID_FIELD_SELECTOR`。这里的 `enum` **就是** [`SELECTORS`]
/// 本身：白名单只有一份，广播面与解析面因此不可能漂移（判据 `tests/trap_177.rs`
/// 逐字对账 —— 不是手抄一份 24 项的清单）。
#[must_use]
pub fn selector_items() -> Value {
    serde_json::json!({
        "type": "string",
        "enum": SELECTORS,
        "description": "字段选择器白名单 (与解析器的 SELECTORS 同一份常量); 不在枚举里的名字一律 `INVALID_FIELD_SELECTOR`",
    })
}

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

/// `clip_pool` 的默认摘要：每个片段只给身份、名字、类型、音符数与末端 tick。
///
/// ⭐ T-FIX-366：这是"响应体积与音符数解耦"的落点。原先 `project_view` 无条件把
/// `YebanProjectV1` 整个序列化，20,000 音符工程 `limit:1` 仍有 3.13 MB；摘要后
/// 默认响应不再包含任何音符。全量音符必须显式选 [`SELECTORS`] 里的
/// `clip_pool.full`（见 [`project_view`] 的 `sections.timeline` 同款例外臂）。
fn clip_pool_summary(project: &YebanProjectV1) -> Value {
    let mut out = Map::new();
    for (id, entry) in &project.clip_pool {
        let (kind, note_count, end_tick) = match &entry.content {
            yeban_model::project::ClipContent::Midi { notes } => (
                "midi",
                notes.len(),
                notes
                    .values()
                    .map(|note| note.start_tick.saturating_add(note.duration_ticks))
                    .max()
                    .unwrap_or(0),
            ),
            yeban_model::project::ClipContent::Audio { .. } => ("audio", 0, 0),
        };
        out.insert(
            id.to_canonical_string(),
            serde_json::json!({
                "id": id.to_canonical_string(),
                "name": entry.name,
                "kind": kind,
                "noteCount": note_count,
                "endTick": end_tick,
            }),
        );
    }
    Value::Object(out)
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

/// 稀疏视图：`fields` 为空时是工程的**稀疏顶层视图**（`clip_pool` 只给摘要）。
///
/// ⭐ T-FIX-366：唯一例外是显式选择 `clip_pool.full` —— 它返回原始片段池
/// （含每个音符），体积与音符数线性；默认视图与 `clip_pool` 选择器都不含音符。
///
/// # Errors
///
/// 工程无法序列化成 JSON（理论上不会）。
pub fn project_view(project: &YebanProjectV1, fields: &[String]) -> Result<Value, Fault> {
    let full = serde_json::to_value(project).map_err(|error| {
        Fault::domain(ErrorCode::IoError, format!("序列化工程视图失败: {error}"))
    })?;
    let mut object = full.as_object().cloned().unwrap_or_default();
    // ⭐ T-FIX-366：默认 `project.clip_pool` 只留摘要；全量音符走 `clip_pool.full`。
    object.insert("clip_pool".to_owned(), clip_pool_summary(project));
    // ⭐ T-FIX-659：`tracks[].clips` 里各摆放对象补 `placement_id` 别名（与 `id`
    // 同值），让调用方一眼分清摆放身份与材质 `clip_id`；旧 `id`/`clip_id` 字段不动，
    // 模型与落盘格式也不动（只改查询视图的投影，在字段选择之前注入，各选择器都可见）。
    add_placement_id_aliases(&mut object);
    if fields.is_empty() {
        return Ok(Value::Object(object));
    }
    let mut out = Map::new();
    for selector in fields {
        match selector.split_once('.') {
            None => {
                if let Some(value) = object.get(selector) {
                    out.insert(selector.clone(), value.clone());
                } else {
                    out.insert(selector.clone(), Value::Null);
                }
            }
            // ⭐ T-FIX-366：全量片段池的显式出口。必须排在通用下钻臂之前 ——
            // 通用臂会把 `full` 当成片段条目里的叶子，取不到值。
            Some(("clip_pool", "full")) => {
                out.insert(
                    "clip_pool".to_owned(),
                    full.get("clip_pool").cloned().unwrap_or(Value::Null),
                );
            }
            // ⭐ [G15] 派生视图：段落按时间轴升序（走独立输出键，⛔ 不改 `sections`
            // 原始形状）。必须排在通用下钻臂**之前** —— 否则这里会静默产出空对象。
            // ⚠ 根键 `sections` 必须**同时**出现（取不到根键 = 白名单与数据模型漂移，
            // 单元判据 `every_selector_resolves_against_a_filled_project` 钉住这件事）；
            // 其值与单独选择 `sections` 时**逐字节同形**（同一份序列化投影的克隆）。
            Some(("sections", "timeline")) => {
                out.insert(
                    "sections".to_owned(),
                    object.get("sections").cloned().unwrap_or(Value::Null),
                );
                let mut timeline: Vec<Value> = Vec::with_capacity(project.sections.len());
                for section in project.sections.values() {
                    timeline.push(serde_json::to_value(section).map_err(|error| {
                        Fault::domain(
                            ErrorCode::IoError,
                            format!("序列化段落时间轴视图失败: {error}"),
                        )
                    })?);
                }
                timeline.sort_by(|left, right| {
                    let key = |entry: &Value| {
                        (
                            entry["start_tick"].as_u64().unwrap_or_default(),
                            entry["id"].as_str().unwrap_or_default().to_owned(),
                        )
                    };
                    key(left).cmp(&key(right))
                });
                out.insert("sectionsTimeline".to_owned(), Value::Array(timeline));
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

/// 给查询视图里每条摆放对象补 `placement_id`（= `id` 的别名）。
///
/// 只改 `project_view` 产出的投影：模型不变、落盘不变；`clips` 为空的音轨无事可做。
fn add_placement_id_aliases(object: &mut Map<String, Value>) {
    let Some(Value::Object(tracks)) = object.get_mut("tracks") else {
        return;
    };
    for track in tracks.values_mut() {
        let Some(Value::Object(clips)) = track.get_mut("clips") else {
            continue;
        };
        for clip in clips.values_mut() {
            let Some(id) = clip.get("id").cloned() else {
                continue;
            };
            if let Some(clip) = clip.as_object_mut() {
                clip.insert("placement_id".to_owned(), id);
            }
        }
    }
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

    /// T-FIX-659：查询视图里 `tracks[].clips` 的每个摆放对象补 `placement_id` 别名
    /// （与 `id` 同值），旧 `id`/`clip_id` 字段不动；空轨无事可做。
    #[test]
    fn query_project_clips_carry_placement_id_alias() {
        let project = filled_project();
        let view = project_view(&project, &[]).expect("缺省视图");
        let tracks = view["tracks"].as_object().expect("tracks 必须是对象");
        let mut checked = 0usize;
        for track in tracks.values() {
            let Some(clips) = track.get("clips").and_then(Value::as_object) else {
                continue;
            };
            for clip in clips.values() {
                let clip = clip.as_object().expect("摆放必须是对象");
                assert_eq!(
                    clip["placement_id"], clip["id"],
                    "placement_id 必须等于 id: {clip:?}"
                );
                assert!(
                    clip.contains_key("clip_id"),
                    "旧 clip_id 字段必须保留: {clip:?}"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "样本工程必须至少有一个摆放，否则本判据是空转");
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

    /// **边界**：`limit` 恰好等于 [`MAX_LIMIT`] 时**不是**夹紧（夹紧只发生在 `>`）。
    ///
    /// 既有判据只喂了 `100_000`（远超上限）⇒ `>` 与 `>=` 在那里同解，而
    /// `limitClamped` 是**响应里给调用方看**的一个字段：恰好给到上限却被报成
    /// "被夹紧了"，调用方会以为自己要的页大小没被满足（`page.limit` 是对的，
    /// 但那一位在说谎）。
    #[test]
    fn a_limit_equal_to_the_maximum_is_not_reported_as_clamped() {
        let at_max = parse(&args(serde_json::json!({ "limit": MAX_LIMIT }))).expect("恰好上限");
        assert_eq!(at_max.limit, MAX_LIMIT);
        assert!(
            !at_max.limit_clamped,
            "恰好等于上限不得报 limitClamped: {at_max:?}"
        );
        let over = parse(&args(serde_json::json!({ "limit": MAX_LIMIT + 1 }))).expect("一格超限");
        assert_eq!(over.limit, MAX_LIMIT);
        assert!(over.limit_clamped, "超过上限一格必须报 limitClamped");
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

    /// ⭐ T-FIX-366：把 `filled_project` 的第一个片段换成 `count` 个音符。
    fn project_with_notes(count: usize) -> YebanProjectV1 {
        let mut project = filled_project();
        let clip_id = *project.clip_pool.keys().next().expect("演示工程必须有片段");
        let mut notes = std::collections::BTreeMap::new();
        for index in 0..count {
            let id = yeban_model::EntityId::new();
            let note = yeban_model::MidiNote::new(id, index as u64 * 10, 60, 480);
            notes.insert(id, note);
        }
        project.clip_pool.get_mut(&clip_id).expect("片段").content =
            yeban_model::project::ClipContent::Midi { notes };
        project
    }

    fn response_bytes(project: &YebanProjectV1) -> usize {
        let page = data(
            project,
            &Query {
                limit: 1,
                ..Query::default()
            },
        )
        .expect("分页视图");
        serde_json::to_vec(&page).expect("序列化响应").len()
    }

    /// 验收 1：20,000 音符工程 `limit:1` 的响应必须 < 64 KB（修前约 3.13 MB）。
    #[test]
    fn a_twenty_thousand_note_project_stays_below_sixty_four_kilobytes() {
        let project = project_with_notes(20_000);
        let bytes = response_bytes(&project);
        assert!(
            bytes < 64 * 1024,
            "T-FIX-366：20,000 音符 limit=1 响应必须 <64KB，实测 {bytes} 字节"
        );
    }

    /// 验收 2：100,000 音符的响应体积不得随音符数线性增长。
    ///
    /// 两个工程只差 `noteCount` 的位数与随机身份，摘要字段数量完全相同；给 512 字节
    /// 余量吸收 JSON 数字/键序差异，仍远小于线性增长（20k→100k 至少会多 5 倍）。
    #[test]
    fn response_bytes_do_not_grow_with_the_number_of_notes() {
        let twenty = response_bytes(&project_with_notes(20_000));
        let hundred = response_bytes(&project_with_notes(100_000));
        assert!(
            hundred < twenty + 512,
            "T-FIX-366：响应体积随音符数增长（20k={twenty}，100k={hundred}）"
        );
    }

    /// 验收 4：`clip_pool` 选择器给摘要；全量音符必须显式 `clip_pool.full`。
    #[test]
    fn clip_pool_is_summary_by_default_and_full_only_by_explicit_selector() {
        let project = project_with_notes(2);
        let clip_id = project
            .clip_pool
            .keys()
            .next()
            .expect("片段")
            .to_canonical_string();

        let default_view = project_view(&project, &[]).expect("缺省视图");
        let clip = default_view["clip_pool"][&clip_id]
            .as_object()
            .expect("摘要对象");
        assert_eq!(clip["noteCount"], 2);
        assert_eq!(clip["kind"], "midi");
        assert!(
            !clip.contains_key("content"),
            "T-FIX-366：默认 clip_pool 不得含 content/音符: {clip:?}"
        );

        let selected = project_view(&project, &["clip_pool".to_owned()]).expect("clip_pool");
        assert!(
            !selected["clip_pool"][&clip_id]
                .as_object()
                .expect("摘要")
                .contains_key("content"),
            "T-FIX-366：`clip_pool` 选择器也必须给摘要"
        );

        let full = project_view(&project, &["clip_pool.full".to_owned()]).expect("全量");
        assert!(
            full["clip_pool"][&clip_id]["content"]["Midi"]["notes"].is_object(),
            "T-FIX-366：`clip_pool.full` 必须返回全量音符"
        );
    }

    /// 契约（T-FIX-442）：默认 `query_project` 是**稀疏视图**，不得当完整
    /// `YebanProjectV1` 反序列化（`clip_pool` 摘要里没有 `content`）。
    /// 红对照：全量序列化本身**能**反序列化（同一断言形状在完整文档上为真），
    /// 因此本判据不是空洞的 —— 它只在默认视图变回全量时变红。
    #[test]
    fn default_view_cannot_deserialize_as_full_project() {
        let project = project_with_notes(2);
        let view = project_view(&project, &[]).expect("缺省视图");
        assert!(
            serde_json::from_value::<YebanProjectV1>(view).is_err(),
            "T-FIX-442：默认视图必须稀疏，不得反序列化成完整 YebanProjectV1"
        );
        let full = serde_json::to_value(&project).expect("全量序列化");
        assert!(
            serde_json::from_value::<YebanProjectV1>(full).is_ok(),
            "全量序列化必须能读回模型类型，否则本判据是空洞的"
        );
    }

    /// 契约（T-FIX-442）：稀疏默认视图 + 显式 `clip_pool.full` 叠回后，
    /// 必须逐位等于全量序列化 —— 这正是 `transpose_clip` 读回走的协议。
    #[test]
    fn sparse_view_plus_explicit_clip_pool_full_deserializes() {
        let project = project_with_notes(2);
        let mut view = project_view(&project, &[]).expect("缺省视图");
        let full = project_view(&project, &["clip_pool.full".to_owned()]).expect("全量池子");
        view["clip_pool"] = full["clip_pool"].clone();
        // T-FIX-659：视图的 clips 每个摆放多一个 `placement_id` 别名（= `id`）；
        // 442 的逐位相等只看除别名外的部分，别名本身由
        // `query_project_clips_carry_placement_id_alias` 钉住。
        if let Some(tracks) = view["tracks"].as_object_mut() {
            for track in tracks.values_mut() {
                if let Some(clips) = track.get_mut("clips").and_then(Value::as_object_mut) {
                    for clip in clips.values_mut() {
                        if let Some(clip) = clip.as_object_mut() {
                            clip.remove("placement_id");
                        }
                    }
                }
            }
        }
        assert_eq!(
            view,
            serde_json::to_value(&project).expect("全量序列化"),
            "T-FIX-442：稀疏视图叠回全量池子后必须逐位等于全量序列化"
        );
        let document: YebanProjectV1 =
            serde_json::from_value(view).expect("叠回后必须能读回模型类型");
        assert_eq!(document.clip_pool.len(), project.clip_pool.len());
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
    /// 分页的**缺省上限**是已发布的字面值 100，而且"不给 `limit`"真的用它。
    ///
    /// 注入（实测红）：`DEFAULT_LIMIT` 100 → 101 ⇒ 既有判据**全绿**（它们从不提这个数）；
    /// 本判据红。
    #[test]
    fn the_default_page_size_is_the_published_hundred() {
        assert_eq!(DEFAULT_LIMIT, 100);
        assert_eq!(MAX_LIMIT, 1000);
        let query = parse(&args(serde_json::json!({}))).expect("缺省查询");
        assert_eq!(query.limit, 100, "不给 `limit` ⇒ 用缺省页大小");
        assert_eq!(query.offset, 0);
        assert!(!query.limit_clamped, "缺省不是被夹紧来的");
        // 端点：(1000, 1001) 这一对是"夹紧"的边界。
        assert_eq!(
            parse(&args(serde_json::json!({"limit": 1000})))
                .expect("上界")
                .limit,
            1000
        );
        let clamped = parse(&args(serde_json::json!({"limit": 1001}))).expect("夹紧");
        assert_eq!(clamped.limit, 1000);
        assert!(clamped.limit_clamped);
    }
    /// **方向 5：分页的两个端点与两个极值**。
    ///
    /// 既有判据 `pagination_is_deterministic_and_bounded` 只打到"offset 越界 ⇒ 空页"。
    /// 本判据把四格补齐（注入都能红）：
    /// 1. `offset == total`（恰好最后一页之后）⇒ `returned == 0`、`hasMore == false`、**不是**错误；
    /// 2. `limit == 剩余条数` 与 `limit == 剩余 + 1` ⇒ 两者 `returned` 相同、都 `hasMore == false`；
    /// 3. `offset == u64::MAX` ⇒ 走 `usize::try_from(..).unwrap_or(usize::MAX)` 的饱和路径，
    ///    不得 panic、不得算错（`end` 不得回绕）；
    /// 4. `limit > MAX_LIMIT` ⇒ 夹紧到 `MAX_LIMIT` 且 `limitClamped == true`，而
    ///    `page.limit` 报的仍是**调用方给的值**（夹紧是**上报**，不是改写）。
    ///
    /// 注入（实测红）：把 `let start = …min(total)` 的 `.min(total)` 去掉 ⇒ 第 1 条红
    /// （`get(start..end)` 拿到 `None` ⇒ 空页仍然对，但 `offset == total` 时 `end < start`
    /// 会让 `returned` 饱和成 0 —— 第 1 条因此专打 `hasMore` 与 `total`）；
    /// 把 `end.saturating_add(limit)` 换成 `start + limit` ⇒ 第 3 条在 debug 下 panic。
    #[test]
    fn pagination_extremes_are_exact_at_the_edges() {
        let project = filled_project();
        let all = data(&project, &Query::default()).expect("全量");
        let total = all["page"]["total"].as_u64().expect("total");
        assert!(total >= 4, "样本的实体索引必须够大: {total}");

        // ① 恰好 `offset == total`。
        let end_page = data(
            &project,
            &Query {
                offset: total,
                ..Query::default()
            },
        )
        .expect("offset == total 不是错误");
        assert_eq!(end_page["page"]["returned"], 0);
        assert_eq!(end_page["page"]["hasMore"], false);
        assert_eq!(end_page["page"]["total"], total);
        assert_eq!(end_page["entities"].as_array().map(Vec::len), Some(0));

        // ② `limit == 剩余` 与 `limit == 剩余 + 1`。
        let remaining = total - 1;
        let exact = data(
            &project,
            &Query {
                limit: remaining,
                offset: 1,
                ..Query::default()
            },
        )
        .expect("恰好剩余");
        let one_more = data(
            &project,
            &Query {
                limit: remaining + 1,
                offset: 1,
                ..Query::default()
            },
        )
        .expect("剩余 + 1");
        assert_eq!(exact["page"]["returned"], remaining);
        assert_eq!(one_more["page"]["returned"], remaining);
        assert_eq!(exact["page"]["hasMore"], false);
        assert_eq!(one_more["page"]["hasMore"], false);

        // ③ `offset == u64::MAX`：饱和路径，不 panic、不回绕。
        let huge = data(
            &project,
            &Query {
                offset: u64::MAX,
                limit: MAX_LIMIT,
                ..Query::default()
            },
        )
        .expect("极大 offset 不是错误");
        assert_eq!(huge["page"]["returned"], 0);
        assert_eq!(huge["page"]["hasMore"], false);
        assert_eq!(huge["page"]["offset"], u64::MAX, "上报原样");

        // ④ 夹紧：`limit` 报原值，`page.limit` 报原值，只有 `limitClamped` 说明被夹。
        let clamped = parse(&args(serde_json::json!({"limit": MAX_LIMIT + 7}))).expect("夹紧");
        assert_eq!(clamped.limit, MAX_LIMIT);
        assert!(clamped.limit_clamped);
        let page = data(&project, &clamped).expect("夹紧后的页");
        assert_eq!(page["page"]["limit"], MAX_LIMIT, "夹紧后按上界报");
        assert_eq!(page["page"]["limitClamped"], true);
        assert!(page["page"]["returned"].as_u64().expect("returned") <= MAX_LIMIT);
    }
}
