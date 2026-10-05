//! 契约棘轮：`schemas/project.schema.json` 的 `required` ⇔ **实现**的必需性。
//!
//! 规范来源与裁决：
//!
//! - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` **D10**
//!   （契约与规范冲突时以规范为准，并且**改契约**留痕）、**D43**（无兼容包袱：
//!   设计上必需 ⇒ 缺键即报错；语义上可选 ⇒ `Option<T>` / 空集才允许默认）；
//! - `docs/ledger/model-no-compat-notes.md` **§5**（收紧清单）与 **§5.4**（18 项 D43 豁免集）；
//! - `MODEL-AST-002`（`YebanProjectV1`）、`MODEL-AST-003`（`BTreeMap` 确定性，红线 4）。
//!
//! ## 为什么要有这条常设判据
//!
//! 已经发生过两类**真实缺陷**，都是"契约的 `required` 与实现的必需性各说各话"：
//!
//! 1. **契约比实现更紧 / 实现静默违反契约**：`tracks` / `clip_pool` / `routing_graph`
//!    被契约列为必需，而实现用 `#[serde(default)]` 兜底 —— 于是一份被截断的文件能"读成功"，
//!    整首曲子的音轨被静默清空。
//! 2. **实现比契约更严 / 坏文件能过门禁**：实现要求 `rng_seed`、`tracks[*].solo_safe` 等字段，
//!    契约却不要求 —— 于是一份缺字段的文件能通过 schema 门禁，却在加载时炸掉。
//!
//! 本文件把这两个方向**都**做成机械判据，并且**直接读契约文件**：
//! 手抄的第二份事实源会在契约改动后变成谎言（同
//! `src/samples.rs::schema_op_variant_names` 的理由），所以这里一个 `required` 名都不手抄。
//!
//! ## 判据清单（9 条 + 1 条 `#[ignore]`）
//!
//! | # | 判据 | 方向 | 测的是什么 |
//! | :-- | :--- | :--- | :--- |
//! | ① | [`fixture_round_trips_and_is_rich_enough_for_the_ratchet`] | — | 夹具自身合法且够富（棘轮不会空转） |
//! | ② | [`direction_a_every_contract_required_path_is_emitted_by_the_writer`] | A | 契约 `required` 的**每一条路径**（递归）都出现在写入器的序列化结果里 |
//! | ③ | [`direction_b_every_contract_required_path_is_required_by_the_reader`] | B | 逐个 `required` 路径**删键** ⇒ `from_value::<YebanProjectV1>` **必须失败** |
//! | ④ | [`direction_b_d43_exempt_paths_tolerate_missing_keys`] | B 反面 | §5.4 的 18 项豁免**确实**可以缺键（证明豁免集没写错） |
//! | ⑤ | [`contract_required_is_disjoint_from_the_d43_exemption_set`] | §5.4 | 豁免项**绝不**能被契约 `required`（否则写入器输出被判非法） |
//! | ⑥ | [`reader_required_paths_are_frozen_debt_in_the_contract`] | B′ | "实现要求但契约没要求"（schema 更松）只能等于**已登记的收紧待办**，新增即红 |
//! | ⑦ | [`contract_self_check_refs_resolve_and_required_names_are_declared`] | 契约自身 | `$ref` 可解析；每个 `required` 名都在同一对象的 `properties` 里（防拼写错）；无未支持的 JSON Schema 形态 |
//! | ⑧ | [`ref_resolver_and_required_name_check_have_teeth`] | 契约自身 | 用**合成 schema** 证明 ⑦ 的解析器真的会报错（不是永真判据） |
//! | ⑨ | [`no_vacuous_required_paths_in_the_ratchet`] | 反空洞 | 每一条"必达"的 `required` 都被夹具真实触达（不许有静默盲区） |
//! | ⑩ | [`print_contract_ratchet_table_for_the_notes`]（`#[ignore]`） | — | 生成 notes 引用的**真实错误文本**表；不进 CI |
//!
//! ## schema 路径记号（本文件的唯一记号，两个方向共用）
//!
//! | 记号 | 含义 |
//! | :--- | :--- |
//! | `#` | 契约根（`YebanProjectV1` 文档） |
//! | `.name` | `properties.name`，或 `oneOf` 分支的**标签名**（如 `content.Midi`） |
//! | `*` | `additionalProperties` 的**每一个**条目（`BTreeMap` 值域） |
//! | `[]` | `items` 的**每一个**元素 |
//! | `oneOf[i]` | **无标签**的 `oneOf`/`anyOf` 分支（如取值域的 `null` / 对象两支） |
//!
//! 例：`#.tracks.*.automation_lanes[].points.*.id`。
//!
//! ## 边界（诚实声明）
//!
//! - 本判据只覆盖 `YebanProjectV1`（`schemas/project.schema.json`），**不覆盖**容器、
//!   `ops.schema.json`、`mcp-tools.schema.json` 等其它契约；
//! - 只判 `required` 的**存在性/必需性**，**不做**完整 JSON Schema 校验
//!   （类型/枚举/区间由 `scripts/gates/validate_schemas.py` 与 CI 的 jsonschema 承担）；
//! - `required` 只在**父值存在**时才可判：`slide.oneOf[1].duration_ticks` 这类
//!   "可选子对象的内部必需字段"在整份文档里一个 `slide` 都没有时无法判定 —— 此时
//!   本判据**如实报告**（⑨ 的 eprintln），不假装判过；
//! - `$defs` / `definitions` 里的模板只在被 `$ref` 引用到时才检查。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::Value;
use yeban_model::YebanProjectV1;
use yeban_model::samples::filled_project;

// ---------------------------------------------------------------------------
// §5.4 豁免集（**显式常量**：docs/ledger/model-no-compat-notes.md §5.4）
// ---------------------------------------------------------------------------

/// D43 语义可选字段：**绝不能**出现在契约的任何 `required` 里 [ADR-0001 D43, notes §5.4]。
///
/// 逐项对应 `docs/ledger/model-no-compat-notes.md` **§5.4** 的表格（18 项）：
///
/// | # | 本常量里的路径 | §5.4 行 | 写入侧行为 |
/// | :-- | :--- | :--- | :--- |
/// | 1 | `#.author` | 根 `author` | 总是写出，但**读取器接受缺失** |
/// | 2 | `#.metadata.description` | `metadata.description` | 总是写出，读取器接受缺失 |
/// | 3 | `#.metadata.tags` | `metadata.tags` | 总是写出，读取器接受缺失 |
/// | 4 | `#.tracks.*.devices[].params[].unit` | `devices[*].params[*].unit` | `None` ⇒ 不落盘 |
/// | 5 | `#.tracks.*.automation_lanes[].domain` | `automation_lanes[*].domain` | `None` ⇒ 不落盘 |
/// | 6 | `#.tracks.*.folder_id` | `tracks[*].folder_id` | `None` ⇒ 不落盘 |
/// | 7 | `#.tracks.*.color` | `tracks[*].color` | `None` ⇒ 不落盘 |
/// | 8 | `#.sections.*.color` | `sections[*].color` | `None` ⇒ 不落盘 |
/// | 9 | `#.scenes.*.tempo` | `scenes[*].tempo` | `None` ⇒ 不落盘 |
/// | 10 | `#.scenes.*.color` | `scenes[*].color` | `None` ⇒ 不落盘 |
/// | 11 | `#.routing_graph.edges[].gain_db` | `routing_graph.edges[*].gain_db` | `Option<f32>` ⇒ 不落盘 |
/// | 12–18 | `#.clip_pool.*.content.Midi.notes.*.{probability,ratchet,micro_timing_ticks,slide,pitch_bend_curve,syllable,phonemes}` | 同左（7 个表现力字段） | `None` / 空集 ⇒ 不落盘（稀疏编码） |
///
/// **注意反例**：`clip_pool[*].content.Audio.gain_db` **不在**本表里 —— 那里 `gain_db`
/// 是裸 `f32`，所以它**是**必需的（§5.4 原文明确点名了这个对比）。
const D43_EXEMPT_REQUIRED_PATHS: &[&str] = &[
    "#.author",
    "#.metadata.description",
    "#.metadata.tags",
    "#.tracks.*.devices[].params[].unit",
    "#.tracks.*.automation_lanes[].domain",
    "#.tracks.*.folder_id",
    "#.tracks.*.color",
    "#.sections.*.color",
    "#.scenes.*.tempo",
    "#.scenes.*.color",
    "#.routing_graph.edges[].gain_db",
    "#.clip_pool.*.content.Midi.notes.*.probability",
    "#.clip_pool.*.content.Midi.notes.*.ratchet",
    "#.clip_pool.*.content.Midi.notes.*.micro_timing_ticks",
    "#.clip_pool.*.content.Midi.notes.*.slide",
    "#.clip_pool.*.content.Midi.notes.*.pitch_bend_curve",
    "#.clip_pool.*.content.Midi.notes.*.syllable",
    "#.clip_pool.*.content.Midi.notes.*.phonemes",
];

/// **已退役**的"契约收紧待办"登记表（曾经 = notes §5.1/§5.2 的 12 项）。
///
/// 历史：`schemas/project.schema.json` 曾经只 `required` 11 个根键，而实现要求
/// `rng_seed` / `metadata` / `tracks[*].solo_safe` … —— 那是"实现要求但契约没要求"
/// （schema 更松 ⇒ 坏文件能过门禁）。本线**只读**契约，所以当时把这份漂移**点名登记**
/// 在这里，判据 ⑥ 写成 `debt ⊆ PENDING`（单向包含）：契约收紧线落地后 `debt` 变空集，
/// 判据不会变成跨线的定时炸弹，而**新**漂移不在表里 ⇒ 当场红。
///
/// **现在（契约收紧已合并进 `main`；`schemas/project.schema.json` sha256
/// `5eb3362ab3fb7efc9ee879a173eff30ded48b560ba01e73524114002f0f47e59`，根 `required` 18 键）
/// 这张表已清空 —— 于是判据 ⑥ 退化成一个**硬等式**：任何
/// "实现要求但契约没要求"的字段都直接变红，没有豁免余地。**
///
/// 若将来真的需要"先登记、后收紧"的过渡期，请把路径加回这里并**在 notes 里写明**
/// 由哪条线负责清空 —— 但**不要**为了让它绿而放宽本判据。
const CONTRACT_TIGHTENING_PENDING: &[&str] = &[];

// ---------------------------------------------------------------------------
// 夹具 / 契约读取
// ---------------------------------------------------------------------------

/// `schemas/project.schema.json` 的路径（`<repo>/schemas/...`）。
fn schema_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schemas")
        .join("project.schema.json")
}

/// 读契约并解析为 JSON（**只读**：本线不改 `schemas/**`）。
fn load_schema() -> Value {
    let path = schema_file();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} 不是合法 JSON: {error}", path.display()))
}

/// 完整填充的规范样本（`samples::filled_project`）的 JSON 形态。
///
/// 用**完整填充**的样本而不是手写小对象，是判据 ②③ 的前提：它同时证明这些键
/// **本来就在写入器写出的字节里**。
fn fixture() -> Value {
    serde_json::to_value(filled_project()).expect("filled_project 必须可序列化")
}

/// 该 schema 路径是否属于 §5.4 豁免集。
fn is_exempt(schema_path: &str) -> bool {
    D43_EXEMPT_REQUIRED_PATHS.contains(&schema_path)
}

// ---------------------------------------------------------------------------
// 路径工具
// ---------------------------------------------------------------------------

/// 拼一段 schema 路径：`[]` 直接粘连，其余用 `.`。
fn child_path(parent: &str, segment: &str) -> String {
    if segment == "[]" {
        format!("{parent}[]")
    } else {
        format!("{parent}.{segment}")
    }
}

/// `oneOf` 分支的路径段：**有标签**（该分支 `required` 恰好一项且 `properties` 里有它）
/// 时用标签名 —— 于是路径与落在磁盘上的 JSON 键一一对应；否则退回 `oneOf[i]`。
fn branch_segment(branch: &Value) -> Option<String> {
    let required = branch.get("required")?.as_array()?;
    if required.len() != 1 {
        return None;
    }
    let tag = required.first()?.as_str()?;
    let declared = branch.get("properties")?.as_object()?;
    declared.contains_key(tag).then(|| tag.to_owned())
}

/// 有标签分支的**标签值 sub-schema**：`branch.properties[tag]`。
///
/// 契约里它一定存在（[`branch_segment`] 就是从 `properties` 里认出来的）；退而求其次
/// 返回分支本身，以免遍历器在畸形契约上 panic（畸形契约由判据 ⑦ 报红）。
fn tagged_branch_spec<'a>(branch: &'a Value, tag: &str) -> &'a Value {
    branch
        .get("properties")
        .and_then(|properties| properties.get(tag))
        .unwrap_or(branch)
}

/// `serde_json::Value` 的 JSON Schema 类型名。
fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// schema 声明的类型是否与实例形状相容（`type` 为数组时任一命中即可；缺 `type` 视为相容）。
fn type_matches(schema: &Value, instance: &Value) -> bool {
    match schema.get("type") {
        None => true,
        Some(Value::String(name)) => json_type(instance) == name,
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(Value::as_str)
            .any(|name| json_type(instance) == name),
        Some(_) => false,
    }
}

/// RFC 6901 JSON 指针拼接（转义 `~` 与 `/`，因为 `EntityId` 之外的键原则上可能含它们）。
fn join_pointer(pointer: &str, key: &str) -> String {
    let escaped = key.replace('~', "~0").replace('/', "~1");
    format!("{pointer}/{escaped}")
}

/// 按 JSON 指针 + 键名删掉一个键（返回是否真的删掉了 —— 删不到就是判据自己错了）。
///
/// **这里测的是"键不存在"**，不是"键是 `null`"：本写入器对语义可选的字段用
/// `Option::None` + `skip_serializing_if`，落盘形态是**键消失**；而 `#[serde(default)]`
/// 的兜底也只在**键不存在**时生效。写 `null` 是另一回事（`Option<T>` 会把它读成 `None`，
/// 那就无法区分"作者显式写了 `null`"与"键被截断"）—— 所以本判据**只删键**。
fn delete_key(root: &mut Value, pointer: &str, key: &str) -> bool {
    root.pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .is_some_and(|object| object.remove(key).is_some())
}

// ---------------------------------------------------------------------------
// 契约自身的遍历（schema → schema，不碰实例）
// ---------------------------------------------------------------------------

/// 一条 `required` 名的登记。
#[derive(Debug, Clone, Copy)]
struct RequiredFact {
    /// 这个名字是否在**同一对象**的 `properties` 里被声明（防拼写错）。
    declared: bool,
    /// 这个名字是否"必达"：从根到其宿主的每一步都经过 `required` 字段
    /// （`oneOf` 分支内部一律视为**不必达** —— 分支是备选，另由组级判据兜底）。
    guaranteed: bool,
}

/// 一个 `oneOf`/`anyOf` 分支。
#[derive(Debug, Clone)]
struct ChoiceBranch {
    /// 组（`oneOf` 所在节点的路径）。
    group: String,
    /// 分支前缀路径。
    prefix: String,
    /// 组宿主是否必达。
    group_guaranteed: bool,
}

/// 契约自身的机械事实（全部由 `walk_schema` 从真实契约文件推出来）。
#[derive(Debug, Default)]
struct SchemaFacts {
    /// 每个 `properties` 里声明的字段：路径 → 是否被同一对象的 `required` 收下。
    declared_properties: BTreeMap<String, bool>,
    /// 每个 `required` 名：路径 → 登记。
    required: BTreeMap<String, RequiredFact>,
    /// 所有 `oneOf`/`anyOf` 分支。
    choice_branches: Vec<ChoiceBranch>,
    /// 所有 `$ref` 字面量。
    refs: BTreeSet<String>,
    /// 遍历器**不支持**的 JSON Schema 关键字（出现即红：不许静默漏判）。
    unsupported: BTreeSet<String>,
}

/// 解析 `#` 开头的本地 `$ref`（`Value::pointer` 自带 `~0`/`~1` 反转义）。
fn resolve_ref<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    if pointer.is_empty() {
        return Some(root);
    }
    if !pointer.starts_with('/') {
        return None;
    }
    root.pointer(pointer)
}

/// 遍历契约（**所有** `oneOf` 分支都走，与实例无关）。
fn walk_schema(node: &Value, root: &Value, path: &str, guaranteed: bool, facts: &mut SchemaFacts) {
    if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        facts.refs.insert(reference.to_owned());
        if let Some(target) = resolve_ref(root, reference) {
            walk_schema(target, root, path, guaranteed, facts);
        }
        return;
    }
    for keyword in [
        "allOf",
        "not",
        "if",
        "then",
        "else",
        "patternProperties",
        "dependentRequired",
        "dependentSchemas",
        "unevaluatedProperties",
        "propertyNames",
        "contains",
        "prefixItems",
    ] {
        if node.get(keyword).is_some() {
            facts
                .unsupported
                .insert(format!("{path} 使用了不支持的 `{keyword}`"));
        }
    }

    let required: BTreeSet<&str> = node
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let properties = node.get("properties").and_then(Value::as_object);

    if let Some(properties) = properties {
        for (name, spec) in properties {
            let child = child_path(path, name);
            let is_required = required.contains(name.as_str());
            facts.declared_properties.insert(child.clone(), is_required);
            if is_required {
                facts.required.insert(
                    child.clone(),
                    RequiredFact {
                        declared: true,
                        guaranteed,
                    },
                );
            }
            walk_schema(spec, root, &child, guaranteed && is_required, facts);
        }
    }
    // `required` 里的名字必须在**同一对象**的 `properties` 里 —— 否则它永远不可能被满足
    // （拼写错的 required 是"契约自己有病"，判据 ⑦ 报红）。
    for name in &required {
        let declared = properties.is_some_and(|map| map.contains_key(*name));
        if !declared {
            facts.required.insert(
                child_path(path, name),
                RequiredFact {
                    declared: false,
                    guaranteed,
                },
            );
        }
    }
    if let Some(spec) = node
        .get("additionalProperties")
        .filter(|spec| spec.is_object())
    {
        walk_schema(spec, root, &child_path(path, "*"), guaranteed, facts);
    }
    if let Some(spec) = node.get("items").filter(|spec| spec.is_object()) {
        walk_schema(spec, root, &child_path(path, "[]"), guaranteed, facts);
    }
    for keyword in ["oneOf", "anyOf"] {
        let Some(branches) = node.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        for (index, branch) in branches.iter().enumerate() {
            // **有标签**的分支（外部标签枚举）：路径段就是标签名，于是路径与落盘的 JSON 键
            // 一一对应 —— 并且**只把标签名走一次**（标签既是分支的 required，也是它的
            // `properties` 键；再走一遍 `properties` 会得到 `content.Midi.Midi`，
            // 于是 `deletion_targets` 解析不到实例，判据 ⑥ 会**静默空转**）。
            if let Some(tag) = branch_segment(branch) {
                let prefix = child_path(path, &tag);
                facts.choice_branches.push(ChoiceBranch {
                    group: path.to_owned(),
                    prefix: prefix.clone(),
                    group_guaranteed: guaranteed,
                });
                facts.declared_properties.insert(prefix.clone(), true);
                facts.required.insert(
                    prefix.clone(),
                    RequiredFact {
                        declared: true,
                        guaranteed: false,
                    },
                );
                walk_schema(
                    tagged_branch_spec(branch, &tag),
                    root,
                    &prefix,
                    false,
                    facts,
                );
                continue;
            }
            let prefix = child_path(path, &format!("oneOf[{index}]"));
            facts.choice_branches.push(ChoiceBranch {
                group: path.to_owned(),
                prefix: prefix.clone(),
                group_guaranteed: guaranteed,
            });
            walk_schema(branch, root, &prefix, false, facts);
        }
    }
}

/// 收集契约自身的机械事实。
fn schema_facts() -> SchemaFacts {
    let schema = load_schema();
    let mut facts = SchemaFacts::default();
    walk_schema(&schema, &schema, "#", true, &mut facts);
    facts
}

// ---------------------------------------------------------------------------
// 实例引导的遍历（schema × 夹具 JSON）
// ---------------------------------------------------------------------------

/// 一条"删键探针"：从 `pointer` 指向的对象里删掉 `key`。
#[derive(Debug, Clone)]
struct Probe {
    /// 所有者对象的 RFC 6901 指针（根为 `""`）。
    pointer: String,
    /// 要删掉的键名。
    key: String,
    /// 该键的 schema 路径。
    schema_path: String,
}

/// 方向 A / B 的实测事实（全部由 `walk_instance` 从真实契约 × 真实夹具推出来）。
#[derive(Debug, Default)]
struct InstanceFacts {
    /// 方向 A：契约 `required` 了、对象也在，但序列化结果里**没有**这个键。
    missing: BTreeSet<String>,
    /// 方向 A：`oneOf`/`anyOf` **没有任何**分支被满足。
    unmatched_one_of: BTreeSet<String>,
    /// 方向 A：契约说要对象（带 `required`），序列化结果却不是对象 ⇒ 无法判定。
    type_mismatch: BTreeSet<String>,
    /// 方向 B 的删键探针（已按"指针 + 键"去重）。
    probes: Vec<Probe>,
    /// 被夹具真实触达的 `required` 路径（反空洞判据 ⑨ 用）。
    touched: BTreeSet<String>,
}

/// 分支是否适用于这个实例：类型相容 **且** 分支自己的 `required` 全部在场。
fn branch_applies(branch: &Value, instance: &Value) -> bool {
    if !type_matches(branch, instance) {
        return false;
    }
    let Some(object) = instance.as_object() else {
        return true;
    };
    branch
        .get("required")
        .and_then(Value::as_array)
        .is_none_or(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .all(|name| object.contains_key(name))
        })
}

/// 遍历"契约 × 夹具"，收集方向 A 的违反与方向 B 的删键探针。
fn walk_instance(
    schema: &Value,
    root: &Value,
    instance: Option<&Value>,
    path: &str,
    pointer: &str,
    facts: &mut InstanceFacts,
) {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if let Some(target) = resolve_ref(root, reference) {
            walk_instance(target, root, instance, path, pointer, facts);
        }
        return;
    }

    // `oneOf` / `anyOf`：**至少一个**分支被满足即可（外部标签枚举的 `Midi` / `Audio` 两支）。
    if let Some(branches) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
    {
        let Some(value) = instance else {
            return;
        };
        let mut matched = 0_usize;
        for (index, branch) in branches.iter().enumerate() {
            if !branch_applies(branch, value) {
                continue;
            }
            matched += 1;
            // 有标签的分支：标签名只走一次（见 `walk_schema` 里的同一处说明）。
            if let Some(tag) = branch_segment(branch) {
                let prefix = child_path(path, &tag);
                facts.touched.insert(prefix.clone());
                if value.as_object().is_some_and(|map| map.contains_key(&tag)) {
                    facts.probes.push(Probe {
                        pointer: pointer.to_owned(),
                        key: tag.clone(),
                        schema_path: prefix.clone(),
                    });
                } else {
                    facts.missing.insert(prefix.clone());
                }
                if let Some(child) = value.as_object().and_then(|map| map.get(&tag)) {
                    walk_instance(
                        tagged_branch_spec(branch, &tag),
                        root,
                        Some(child),
                        &prefix,
                        &join_pointer(pointer, &tag),
                        facts,
                    );
                }
                continue;
            }
            walk_instance(
                branch,
                root,
                Some(value),
                &child_path(path, &format!("oneOf[{index}]")),
                pointer,
                facts,
            );
        }
        if matched == 0 {
            facts.unmatched_one_of.insert(path.to_owned());
        }
        return;
    }

    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    if let Some(value) = instance {
        if !required.is_empty() && value.as_object().is_none() {
            facts.type_mismatch.insert(path.to_owned());
            return;
        }
        if let Some(object) = value.as_object() {
            for name in &required {
                let child = child_path(path, name);
                facts.touched.insert(child.clone());
                if object.contains_key(*name) {
                    facts.probes.push(Probe {
                        pointer: pointer.to_owned(),
                        key: (*name).to_owned(),
                        schema_path: child,
                    });
                } else {
                    facts.missing.insert(child);
                }
            }
        }
    }

    let declared: BTreeSet<&str> = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|map| map.keys().map(String::as_str).collect())
        .unwrap_or_default();

    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, spec) in properties {
            let child = schema_object_child(instance, name);
            let child_pointer = match child {
                Some(_) => join_pointer(pointer, name),
                None => String::new(),
            };
            walk_instance(
                spec,
                root,
                child,
                &child_path(path, name),
                &child_pointer,
                facts,
            );
        }
    }
    if let Some(spec) = schema
        .get("additionalProperties")
        .filter(|spec| spec.is_object())
        && let Some(object) = instance.and_then(Value::as_object)
    {
        for (key, value) in object {
            if declared.contains(key.as_str()) {
                continue;
            }
            walk_instance(
                spec,
                root,
                Some(value),
                &child_path(path, "*"),
                &join_pointer(pointer, key),
                facts,
            );
        }
    }
    if let Some(spec) = schema.get("items").filter(|spec| spec.is_object())
        && let Some(items) = instance.and_then(Value::as_array)
    {
        for (index, value) in items.iter().enumerate() {
            walk_instance(
                spec,
                root,
                Some(value),
                &child_path(path, "[]"),
                &format!("{pointer}/{index}"),
                facts,
            );
        }
    }
}

/// `instance.name`（实例不是对象、或缺这个键时返回 `None`）。
fn schema_object_child<'a>(instance: Option<&'a Value>, name: &str) -> Option<&'a Value> {
    instance
        .and_then(Value::as_object)
        .and_then(|map| map.get(name))
}

/// 收集"契约 × 夹具"的实测事实。
fn instance_facts() -> InstanceFacts {
    let schema = load_schema();
    let fixture = fixture();
    let mut facts = InstanceFacts::default();
    walk_instance(&schema, &schema, Some(&fixture), "#", "", &mut facts);
    facts
}

// ---------------------------------------------------------------------------
// 实例侧路径解析（把 schema 路径落到具体 JSON 指针 + 键）
// ---------------------------------------------------------------------------

/// 实例路径的一段。
#[derive(Debug, Clone)]
enum Segment {
    /// 具体键名。
    Key(String),
    /// `*`：对象的每一个值。
    AnyKey,
    /// `[]`：数组的每一个元素。
    AnyIndex,
}

/// 把 schema 路径解析成实例路径段；遇到**无法落到实例键上**的形态（`oneOf[i]`）返回 `None`。
fn parse_instance_path(path: &str) -> Option<Vec<Segment>> {
    let body = path.strip_prefix('#')?;
    let mut segments = Vec::new();
    for token in body.split('.').filter(|token| !token.is_empty()) {
        if token.contains("oneOf[") || token.starts_with("anyOf[") {
            return None;
        }
        if let Some(stem) = token.strip_suffix("[]") {
            segments.push(Segment::Key(stem.to_owned()));
            segments.push(Segment::AnyIndex);
        } else if token == "*" {
            segments.push(Segment::AnyKey);
        } else {
            segments.push(Segment::Key(token.to_owned()));
        }
    }
    Some(segments)
}

/// 收集"删掉这个键"的具体目标：`(所有者对象的指针, 键名)`。
fn deletion_targets(root: &Value, path: &str) -> Option<Vec<(String, String)>> {
    let segments = parse_instance_path(path)?;
    let mut out = Vec::new();
    collect_deletions(root, "", &segments, &mut out);
    Some(out)
}

fn collect_deletions(
    value: &Value,
    pointer: &str,
    segments: &[Segment],
    out: &mut Vec<(String, String)>,
) {
    let Some((head, rest)) = segments.split_first() else {
        return;
    };
    match head {
        Segment::AnyKey => {
            if let Some(object) = value.as_object() {
                for (key, child) in object {
                    collect_deletions(child, &join_pointer(pointer, key), rest, out);
                }
            }
        }
        Segment::AnyIndex => {
            if let Some(items) = value.as_array() {
                for (index, child) in items.iter().enumerate() {
                    collect_deletions(child, &format!("{pointer}/{index}"), rest, out);
                }
            }
        }
        Segment::Key(name) => {
            if rest.is_empty() {
                if value
                    .as_object()
                    .is_some_and(|object| object.contains_key(name))
                {
                    out.push((pointer.to_owned(), name.clone()));
                }
            } else if let Some(child) = value.as_object().and_then(|object| object.get(name)) {
                collect_deletions(child, &join_pointer(pointer, name), rest, out);
            }
        }
    }
}

/// 某个 schema 路径前缀下是否有被触达的实例。
fn touched_under(touched: &BTreeSet<String>, prefix: &str) -> bool {
    touched.iter().any(|path| {
        path == prefix
            || path.starts_with(&format!("{prefix}."))
            || path.starts_with(&format!("{prefix}["))
    })
}

// ---------------------------------------------------------------------------
// ① 夹具自身
// ---------------------------------------------------------------------------

/// 夹具必须"合法 + 够富"：棘轮不能空转。
///
/// "够富"是判据 ③④ 的**前提** —— 若夹具里 `clip_pool` 是空的，那么
/// `clip_pool[*].content.Midi.notes[*].*` 这些 `required` 就一个探针也生不出来，
/// 判据会变成永真。⑨ 专门把这种空洞变成红。
#[test]
fn fixture_round_trips_and_is_rich_enough_for_the_ratchet() {
    let project = filled_project();
    let value = serde_json::to_value(&project).expect("to_value");
    let back: YebanProjectV1 = serde_json::from_value(value.clone()).expect("from_value");
    assert_eq!(back, project, "filled_project 必须能 JSON 往返且逐字段相等");
    assert_eq!(
        project.validate(),
        Ok(()),
        "夹具自己必须是合法工程（否则判据 ②③ 测的是坏输入）"
    );

    let counts = [
        ("tracks", project.tracks.len()),
        ("clip_pool", project.clip_pool.len()),
        ("sections", project.sections.len()),
        ("scenes", project.scenes.len()),
        ("assets", project.assets.len()),
        ("routing_graph.edges", project.routing_graph.edges.len()),
    ];
    for (name, count) in counts {
        assert!(
            count > 0,
            "夹具的 `{name}` 不能为空（否则相关 required 判据空转）"
        );
    }
    let devices: usize = project
        .tracks
        .values()
        .map(|track| track.devices.len())
        .sum();
    let macros: usize = project
        .tracks
        .values()
        .map(|track| track.macros.len())
        .sum();
    let lanes: usize = project
        .tracks
        .values()
        .map(|track| track.automation_lanes.len())
        .sum();
    let clips: usize = project.tracks.values().map(|track| track.clips.len()).sum();
    assert!(devices > 0, "夹具必须至少有一个 device（含 params）");
    assert!(macros > 0, "夹具必须至少有一个 macro（含 mappings）");
    assert!(lanes > 0, "夹具必须至少有一条自动化泳道（含 points）");
    assert!(clips > 0, "夹具必须至少有一个时间轴摆放（含 loop_config）");
    let midi_notes: usize = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(std::collections::BTreeMap::len)
        .sum();
    assert!(midi_notes > 0, "夹具必须至少有一个 MIDI 音符");
}

// ---------------------------------------------------------------------------
// ② 方向 A：契约不能比实现更紧（写入器的输出必须满足契约的每一条 required）
// ---------------------------------------------------------------------------

/// **方向 A**：递归走契约的每一条 `required` 路径，断言它**存在于**写入器的序列化结果里。
///
/// 递归的三种"下去"方式：`properties` → 子对象、`additionalProperties` → 每一个
/// map 条目（`tracks[*]` / `clip_pool[*]` / `notes[*]` …）、`items` → 每一个数组元素
/// （`routing_graph.edges[*]` / `devices[*]` / `params[*]` / `automation_lanes[*]` …）。
///
/// `oneOf` 的语义是"**至少一个**分支的 `required` 被满足"，因此 `clip_pool[*].content`
/// 的 `Midi` / `Audio` 两支**不要求**同时满足 —— 一条 `Midi` 条目不会因为缺 `Audio`
/// 的 `asset` 而被判违约。
///
/// **条件性**：`slide.oneOf[1].duration_ticks` 这类"可选子对象的内部必需字段"只在
/// 父值真的出现时才可判；父值缺席时它不构成违约（这也正是 JSON Schema 的语义）。
#[test]
fn direction_a_every_contract_required_path_is_emitted_by_the_writer() {
    let facts = instance_facts();

    assert!(
        facts.type_mismatch.is_empty(),
        "契约说有 required 的节点必须是 JSON 对象，但这些路径在序列化结果里不是对象 \
         （形状分歧，本判据无法判定）: {:?}",
        facts.type_mismatch
    );
    assert!(
        facts.unmatched_one_of.is_empty(),
        "这些 `oneOf` 节点在序列化结果里**没有任何**分支的 required 被满足: {:?}",
        facts.unmatched_one_of
    );
    assert!(
        facts.missing.is_empty(),
        "【方向 A 违约】契约 `required` 了这些路径，但 filled_project 的序列化结果里**没有**这些键 \
         —— 要么写入器漏写（改实现），要么契约过紧（改契约并在 docs/adr/ADR-0001 留痕）: {:?}",
        facts.missing
    );
    assert!(
        facts.touched.len() >= 20,
        "只判到 {} 条 required 路径，太少 —— 夹具或遍历器出问题了",
        facts.touched.len()
    );
}

// ---------------------------------------------------------------------------
// ③ 方向 B：契约不能比实现更松（坏文件不许过门禁）
// ---------------------------------------------------------------------------

/// **方向 B**：对契约里**每一个** `required` 字段路径，把完整工程的那一处**删掉**
/// （键不存在，而不是写成 `null` —— 见 [`delete_key`]），
/// 再 `serde_json::from_value::<YebanProjectV1>` —— **必须失败**。
///
/// 某个路径删掉后**仍然读成功** ⇒ 说明"实现其实不要求它" ⇒ 契约比实现更紧，
/// 这条棘轮当场红并**点名那个路径**。
///
/// §5.4 的 18 项豁免**不参与**本判据（它们是语义可选字段，契约一旦 required 它们，
/// 本写入器自己的输出就会被判非法）—— 由判据 ④⑤ 单独负责。
#[test]
fn direction_b_every_contract_required_path_is_required_by_the_reader() {
    let facts = instance_facts();
    let mut probes: BTreeMap<String, Vec<Probe>> = BTreeMap::new();
    for probe in &facts.probes {
        if is_exempt(&probe.schema_path) {
            continue;
        }
        probes
            .entry(probe.schema_path.clone())
            .or_default()
            .push(probe.clone());
    }

    let mut drift: Vec<String> = Vec::new();
    for (path, list) in &probes {
        for probe in list {
            let mut document = fixture();
            assert!(
                delete_key(&mut document, &probe.pointer, &probe.key),
                "删键探针自己失效了: {path} @ {} / {}",
                probe.pointer,
                probe.key
            );
            if serde_json::from_value::<YebanProjectV1>(document).is_ok() {
                drift.push(format!(
                    "{path}（实例 {} 的键 `{}`）删掉后**仍然读成功**",
                    probe.pointer, probe.key
                ));
            }
        }
    }

    assert!(
        drift.is_empty(),
        "【方向 B 违约】契约 `required` 了这些路径，但实现**不**要求它们 \
         （删掉相应键仍能反序列化）—— 契约比实现更紧，本写入器自己的输出会被判非法:\n{}",
        drift.join("\n")
    );
    assert!(
        probes.len() >= 20,
        "只探到 {} 条 required 路径，太少 —— 夹具或遍历器出问题了",
        probes.len()
    );
}

// ---------------------------------------------------------------------------
// ④ 方向 B 的反面：§5.4 豁免项**确实**可以缺键
// ---------------------------------------------------------------------------

/// §5.4 的 18 项豁免逐个"删键 ⇒ 必须**读成功**"。
///
/// 这一条专门证明**豁免集没写错**：若 §5.4 里某一项其实是**必需**的（清单有误），
/// 本判据会红并点名它 —— 那时**不要**改豁免集换绿，把清单报给集成者。
///
/// 夹具里本来就不落盘的豁免项（`folder_id` / `SceneV3::color` / `MidiNote::slide` /
/// `pitch_bend_curve` / `phonemes`）在这里记为"缺席"：它们的"缺键可读"由判据 ① 的
/// 整份往返已经证明。
#[test]
fn direction_b_d43_exempt_paths_tolerate_missing_keys() {
    let fixture = fixture();
    let mut absent: Vec<&str> = Vec::new();
    let mut present_paths = 0_usize;
    let mut deleted_keys = 0_usize;

    for path in D43_EXEMPT_REQUIRED_PATHS {
        let targets = deletion_targets(&fixture, path)
            .unwrap_or_else(|| panic!("§5.4 豁免路径无法解析到实例上: {path}"));
        if targets.is_empty() {
            absent.push(path);
            continue;
        }
        present_paths += 1;
        for (pointer, key) in targets {
            let mut document = fixture.clone();
            assert!(delete_key(&mut document, &pointer, &key));
            deleted_keys += 1;
            serde_json::from_value::<YebanProjectV1>(document).unwrap_or_else(|error| {
                panic!(
                    "§5.4 豁免路径 `{path}` 删掉键 `{key}`（实例 {pointer}）之后读取失败: {error}\n\
                     若它其实是必需的，说明 §5.4 清单有误 —— 把清单报给集成者，不要改豁免集换绿。"
                )
            });
        }
    }

    assert_eq!(
        D43_EXEMPT_REQUIRED_PATHS.len(),
        18,
        "§5.4 是 18 项（逐项对应见常量上的表）"
    );
    assert!(
        present_paths >= 8,
        "只有 {present_paths}/18 条豁免路径在夹具里真的落了盘（删了 {deleted_keys} 个键）\
         —— 夹具太贫瘠，本条判据形同虚设。缺席（由判据 ① 覆盖）: {absent:?}"
    );
    eprintln!(
        "[schema-ratchet] §5.4 豁免: {present_paths}/18 条落盘并删键验证成功（共 {deleted_keys} 个键）; \
         缺席 {}/18: {absent:?}",
        absent.len()
    );
}

// ---------------------------------------------------------------------------
// ⑤ §5.4 的机械形式：豁免项绝不能被契约 required
// ---------------------------------------------------------------------------

/// §5.4 的 18 项**一律不得**出现在契约的任何 `required` 里 [ADR-0001 D43]。
///
/// 这是 notes §5.4 "判据 ④ 是这条原则的机械形式"的落地版，而且是**直接读契约**的：
/// 收紧线若把某个语义可选字段收进 `required`，本判据立刻红 —— 那比"漏检"更糟，
/// 它会制造"实现与契约互相指责"的假事故（写入器的输出被判非法）。
#[test]
fn contract_required_is_disjoint_from_the_d43_exemption_set() {
    let facts = schema_facts();

    let violations: Vec<&str> = D43_EXEMPT_REQUIRED_PATHS
        .iter()
        .copied()
        .filter(|path| facts.required.contains_key(*path))
        .collect();
    assert!(
        violations.is_empty(),
        "【§5.4 违约】这些 D43 豁免项被契约 `required` 了（写入器会在默认值上省略它们，\
         于是写入器自己的输出会被 schema 判为非法）: {violations:?}\n\
         依据: docs/ledger/model-no-compat-notes.md §5.4"
    );

    let declared = D43_EXEMPT_REQUIRED_PATHS
        .iter()
        .filter(|path| facts.declared_properties.contains_key(**path))
        .count();
    assert!(
        declared >= 2,
        "豁免集与契约几乎完全脱节（只有 {declared} 项在契约里被声明）—— \
         说明本文件的路径记号与契约漂移了，请先修判据"
    );
    eprintln!(
        "[schema-ratchet] §5.4 豁免集: 18 项, 其中 {declared} 项已被契约声明为属性、0 项被 required"
    );
}

// ---------------------------------------------------------------------------
// ⑥ 方向 B′：实现要求但契约没要求 = 已登记的收紧待办，新增即红
// ---------------------------------------------------------------------------

/// **方向 B′（schema 更松 ⇒ 坏文件能过门禁）**：契约里**声明为属性但没进 `required`**
/// 的每一个路径，逐个删键：
///
/// - 删掉后**读成功** ⇒ 实现也不要求它（契约与实现一致，或契约更紧，由判据 ③ 管）；
/// - 删掉后**读失败** ⇒ 实现要求它，而契约**没**要求它 ⇒ 一份缺这个字段的文件能通过
///   schema 门禁、却在加载时炸掉 —— 这正是"坏文件能过门禁"的形态。
///
/// 这类路径必须全部落在 [`CONTRACT_TIGHTENING_PENDING`] 里。**该表已在契约收紧落地后清空**，
/// 于是本条判据现在是一个**硬等式**：`debt` 必须为空 —— 任何"实现要求但契约没要求"都当场红。
/// （若将来需要过渡期，把路径加回该常量并写明由谁清空；**不要**为换绿而放宽本判据。）
///
/// 为什么断言写成 `debt ⊆ PENDING` 而不是 `debt == PENDING`：`PENDING` 是**只许缩小的上界**，
/// 不是"应当存在的债务清单" —— 收紧线落地后它变空集，判据**依然绿**（不会变成跨线的定时炸弹），
/// 而任何**新**漂移都不在表里 ⇒ **当场红**。
///
/// ⚠ 局限：本判据只能判**契约已经声明**的属性。契约里连 `properties` 都没有的字段
/// （如收紧前的 `metadata` / `devices`）在这里看不见 —— 那是"契约缺失"，只能由契约线
/// 补上属性后才能被这台棘轮盯住（收紧之后它们立刻进 `required`，由判据 ③ 接管）。
#[test]
fn reader_required_paths_are_frozen_debt_in_the_contract() {
    let facts = schema_facts();
    let fixture = fixture();
    let pending: BTreeSet<&str> = CONTRACT_TIGHTENING_PENDING.iter().copied().collect();

    let candidates: Vec<&String> = facts
        .declared_properties
        .iter()
        .filter(|(_, is_required)| !**is_required)
        .map(|(path, _)| path)
        .collect();

    let mut unjudgeable: Vec<&String> = Vec::new();
    let mut unjudged: Vec<&String> = Vec::new();
    let mut judged = 0_usize;
    let mut debt: BTreeSet<String> = BTreeSet::new();
    for path in &candidates {
        let Some(targets) = deletion_targets(&fixture, path) else {
            unjudgeable.push(path);
            continue;
        };
        if targets.is_empty() {
            // 契约声明了这个属性，但夹具里一处都没落盘 ⇒ 删键无从谈起。
            // 只有 §5.4 豁免项允许这样（它们的"缺键可读"由判据 ①④ 覆盖）；
            // 其余情况说明判决在某条路径上**空转**了，必须报红而不是假装判过。
            unjudged.push(path);
            continue;
        }
        judged += 1;
        for (pointer, key) in targets {
            let mut document = fixture.clone();
            assert!(delete_key(&mut document, &pointer, &key));
            if serde_json::from_value::<YebanProjectV1>(document).is_err() {
                debt.insert((*path).clone());
            }
        }
    }

    assert!(
        unjudgeable.is_empty(),
        "这些「契约声明为属性」的路径含本判据不支持的分支记号（`oneOf[i]`），无法落到实例上: \
         {unjudgeable:?} —— 请扩展路径解析器，**不许**悄悄跳过"
    );
    let leaked: Vec<&&String> = unjudged.iter().filter(|path| !is_exempt(path)).collect();
    assert!(
        leaked.is_empty(),
        "【棘轮盲区】契约声明了这些**非豁免**的可选属性，但夹具里一处都没落盘 ⇒ \
         方向 B′ 在它们上面空转（既不能判「实现要求」，也不能判「实现不要求」）: {leaked:?}\n\
         修法二选一: 把 samples::filled_project() 填上，或证明它确属 §5.4 语义可选并登记豁免。"
    );
    assert!(
        judged >= 3,
        "方向 B′ 只真正判到 {judged} 条路径（候选 {} 条）—— 太少，判据在空转",
        candidates.len()
    );

    let new_debt: Vec<&String> = debt
        .iter()
        .filter(|path| !pending.contains(path.as_str()))
        .collect();
    assert!(
        new_debt.is_empty(),
        "【方向 B′ 违约】出现了**新的**「实现要求但契约没要求」的字段 —— \
         一份缺这些字段的文件能通过 schema 门禁、却在加载时失败（坏文件能过门禁）:\n{new_debt:?}\n\
         合法出路: 让 `schemas/**` 的所有者把它们加进对应对象的 `required`（本线不改契约）。\
         已登记待办: {CONTRACT_TIGHTENING_PENDING:?}"
    );

    eprintln!(
        "[schema-ratchet] 方向 B′: 契约声明为可选属性 {} 条（真正判到 {judged} 条，夹具里不落盘 {} 条）; \
         其中「实现要求但契约没要求」 {} 条: {:?}; 已登记过渡上限 {} 条",
        candidates.len(),
        unjudged.len(),
        debt.len(),
        debt,
        CONTRACT_TIGHTENING_PENDING.len()
    );
}

// ---------------------------------------------------------------------------
// ⑦ 契约自身的完整性
// ---------------------------------------------------------------------------

/// 契约自己必须自洽，否则前面两个方向都在判一个"有病的契约"：
///
/// 1. **`$ref` 必须可解析**（本契约当前不用 `$ref`，但判据留着：契约一旦引入就必须能解析）；
/// 2. **每个 `required` 名必须在同一对象的 `properties` 里** ——
///    `{"required": ["numenator"]}` 这种拼写错是一个**永不可能满足**的 required
///    （本写入器永远写不出 `numenator`，schema 校验永远失败）；
/// 3. **不许出现本遍历器不支持的 JSON Schema 关键字**（`allOf` / `patternProperties` / …）——
///    出现了就说明本判据的覆盖是**不完整**的，必须显式扩展遍历器，而不是悄悄漏判。
#[test]
fn contract_self_check_refs_resolve_and_required_names_are_declared() {
    let schema = load_schema();
    let facts = schema_facts();

    let unresolved: Vec<&String> = facts
        .refs
        .iter()
        .filter(|reference| resolve_ref(&schema, reference).is_none())
        .collect();
    assert!(
        unresolved.is_empty(),
        "契约里的 `$ref` 无法解析（改契约的人请修，本线只读它）: {unresolved:?}"
    );

    let undeclared: Vec<&String> = facts
        .required
        .iter()
        .filter(|(_, fact)| !fact.declared)
        .map(|(path, _)| path)
        .collect();
    assert!(
        undeclared.is_empty(),
        "这些 `required` 名在**同一对象**的 `properties` 里根本不存在 —— \
         拼写错会造成一个永远无法满足的 required（本写入器永远写不出这个键）: {undeclared:?}"
    );

    assert!(
        facts.unsupported.is_empty(),
        "契约使用了本判据的遍历器不支持的 JSON Schema 形态，覆盖因此**不完整**: {:?}\n\
         正确做法是扩展 tests/schema_ratchet.rs 的 walk_schema/walk_instance，\
         而不是让判据悄悄漏判。",
        facts.unsupported
    );

    assert!(
        facts.required.len() >= 11,
        "只从契约里读出 {} 条 required 路径 —— 读契约这一步出错了",
        facts.required.len()
    );
    assert!(
        !facts.refs.is_empty() || facts.required.len() >= 11,
        "契约完整性判据空转"
    );
}

// ---------------------------------------------------------------------------
// ⑧ ⑦ 的牙齿：合成 schema
// ---------------------------------------------------------------------------

/// 用**合成 schema** 证明 ⑦ 的两条检查真的会报错（不是永真判据）：
///
/// - `$ref` 指向不存在的 `$defs` ⇒ [`resolve_ref`] 返回 `None`；
/// - `required` 里拼错的名字 ⇒ 在 [`SchemaFacts::required`] 里标记为 `declared == false`。
#[test]
fn ref_resolver_and_required_name_check_have_teeth() {
    let synthetic: Value = serde_json::from_str(
        r##"{
          "$defs": {
            "box": { "type": "object", "required": ["side"], "properties": { "side": {"type": "number"} } }
          },
          "type": "object",
          "required": ["good", "sdie"],
          "properties": {
            "good": { "$ref": "#/$defs/box" },
            "broken": { "$ref": "#/$defs/nope" }
          }
        }"##,
    )
    .expect("合成 schema 必须是合法 JSON");

    assert!(
        resolve_ref(&synthetic, "#/$defs/box").is_some(),
        "`#/$defs/box` 必须能解析"
    );
    assert!(
        resolve_ref(&synthetic, "#/$defs/box/properties/side").is_some(),
        "深指针必须能解析"
    );
    assert!(
        resolve_ref(&synthetic, "#/$defs/nope").is_none(),
        "指向不存在的 `$defs` 必须解析失败（否则 ⑦ 是永真判据）"
    );
    assert!(
        resolve_ref(&synthetic, "https://example.com/other.schema.json").is_none(),
        "外部 `$ref` 本判据不支持，必须**显式**判定为不可解析（而不是假装解析成功）"
    );

    let mut facts = SchemaFacts::default();
    walk_schema(&synthetic, &synthetic, "#", true, &mut facts);
    assert_eq!(
        facts.required.get("#.good").map(|fact| fact.declared),
        Some(true),
        "拼写正确的 required 必须被登记为 declared"
    );
    assert_eq!(
        facts.required.get("#.sdie").map(|fact| fact.declared),
        Some(false),
        "拼写错的 required 必须被登记为 **未声明**（⑦ 由此报红）"
    );
    assert_eq!(
        facts.required.get("#.good.side").map(|fact| fact.declared),
        Some(true),
        "`$ref` 目标里的 required 必须被递归检查到"
    );
}

// ---------------------------------------------------------------------------
// ⑨ 反空洞
// ---------------------------------------------------------------------------

/// 每一条"**必达**"的 `required` 路径都必须被夹具真实触达，`oneOf` 组至少有一条分支被触达。
///
/// "必达" = 从根到它的宿主每一步都经过 `required` 字段（`oneOf` 分支内部不算）。
/// 若某条必达的 required 一个探针都生不出来，唯一可能是夹具里那个**集合是空的**
/// （例如 `tracks = {}`）—— 那么方向 A/B 在那条路径上就是**永真**的，
/// 棘轮上有一个静默盲区。这里把它变成红。
///
/// 不满足"必达"的 required（如 `slide.oneOf[1].duration_ticks`：`slide` 本身可选）
/// 允许零探针，但**如实打印**，不假装判过。
#[test]
fn no_vacuous_required_paths_in_the_ratchet() {
    let schema = schema_facts();
    let facts = instance_facts();

    let mut holes: Vec<&String> = schema
        .required
        .iter()
        .filter(|(path, fact)| fact.guaranteed && !facts.touched.contains(*path))
        .map(|(path, _)| path)
        .collect();
    holes.sort();

    let mut groups: BTreeMap<&str, (bool, Vec<&str>)> = BTreeMap::new();
    for branch in &schema.choice_branches {
        let entry = groups
            .entry(branch.group.as_str())
            .or_insert((false, Vec::new()));
        entry.0 |= branch.group_guaranteed;
        entry.1.push(branch.prefix.as_str());
    }
    let mut group_holes: Vec<&str> = Vec::new();
    let mut unexercised_branches: Vec<&str> = Vec::new();
    for (group, (guaranteed, prefixes)) in &groups {
        let exercised = prefixes
            .iter()
            .any(|prefix| touched_under(&facts.touched, prefix));
        if *guaranteed && !exercised {
            group_holes.push(group);
        }
        for prefix in prefixes {
            if !touched_under(&facts.touched, prefix) {
                unexercised_branches.push(prefix);
            }
        }
    }

    assert!(
        holes.is_empty(),
        "【棘轮盲区】这些「必达」的 required 路径在夹具里一个探针都没有 —— \
         说明某个**必需的集合是空的**（夹具太贫瘠），方向 A/B 在这里是永真的: {holes:?}\n\
         修法二选一: 把 samples::filled_project() 填满，或证明该 required 是死条款并改契约。"
    );
    assert!(
        group_holes.is_empty(),
        "【棘轮盲区】这些「必达」的 oneOf 组一条分支都没被夹具触达: {group_holes:?}"
    );
    eprintln!(
        "[schema-ratchet] 覆盖: 契约 required 路径 {} 条（其中必达 {} 条）; \
         oneOf 组 {} 个; 未被夹具选中的分支 {} 条（合法：分支是备选）: {unexercised_branches:?}",
        schema.required.len(),
        schema
            .required
            .values()
            .filter(|fact| fact.guaranteed)
            .count(),
        groups.len(),
        unexercised_branches.len()
    );
}

// ---------------------------------------------------------------------------
// ⑩ 给 notes 的真实错误文本（不进 CI）
// ---------------------------------------------------------------------------

/// 打印判据 ③④⑥ 的**真实**输出表，供 `docs/ledger/schema-ratchet-notes.md` 逐字引用。
///
/// `cargo test -p yeban-model --test schema_ratchet -- --ignored --nocapture`
#[test]
#[ignore = "生成给 notes 引用的真实错误文本表；不进 CI"]
fn print_contract_ratchet_table_for_the_notes() {
    let facts = instance_facts();
    println!("=== 方向 B: 每个契约 required 路径删键后的真实错误 ===");
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for probe in &facts.probes {
        if !seen.insert(probe.schema_path.as_str()) {
            continue;
        }
        let mut document = fixture();
        assert!(delete_key(&mut document, &probe.pointer, &probe.key));
        let verdict = match serde_json::from_value::<YebanProjectV1>(document) {
            Ok(_) => "*** 读成功（漂移！）***".to_owned(),
            Err(error) => error.to_string(),
        };
        println!("{:<70} {}", probe.schema_path, verdict);
    }

    println!("\n=== §5.4 豁免: 删键后必须读成功 ===");
    let fixture = fixture();
    for path in D43_EXEMPT_REQUIRED_PATHS {
        let targets = deletion_targets(&fixture, path).expect("豁免路径必须可解析");
        if targets.is_empty() {
            println!("{path:<70} (夹具里不落盘; 由整份往返覆盖)");
            continue;
        }
        for (pointer, key) in targets {
            let mut document = fixture.clone();
            assert!(delete_key(&mut document, &pointer, &key));
            let verdict = match serde_json::from_value::<YebanProjectV1>(document) {
                Ok(_) => "Ok(缺键可读)".to_owned(),
                Err(error) => format!("*** Err({error}) ***"),
            };
            println!("{path:<70} {verdict}  [{pointer} / {key}]");
        }
    }

    println!("\n=== 方向 B′: 契约声明为可选属性、但实现要求的路径 ===");
    let schema = schema_facts();
    for (path, is_required) in &schema.declared_properties {
        if *is_required {
            continue;
        }
        let Some(targets) = deletion_targets(&fixture, path) else {
            println!("{path:<70} (无法解析到实例)");
            continue;
        };
        if targets.is_empty() {
            println!("{path:<70} (夹具里不存在, 无从判定)");
            continue;
        }
        for (pointer, key) in targets {
            let mut document = fixture.clone();
            assert!(delete_key(&mut document, &pointer, &key));
            let verdict = match serde_json::from_value::<YebanProjectV1>(document) {
                Ok(_) => "Ok(缺键可读 ⇒ 实现不要求)".to_owned(),
                Err(error) => format!("*** Err ⇒ 实现要求但契约没要求: {error} ***"),
            };
            println!("{path:<70} {verdict}  [{pointer} / {key}]");
        }
    }
}
