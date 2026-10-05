//! [ADR-0001 D43] 反序列化**严格性**判据。
//!
//! 规范来源：
//!
//! - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` **D43**：
//!   1.0.0 之前没有历史包袱与兼容需求，设计上必需的字段**要求**它（缺失即报错）；
//!   2. 语义上天然可选的 —— `Option<T>`、空串/空集 —— 才保留 `#[serde(default)]`；
//!   3. 版本只用于**拒绝不匹配**，不用于兼容多版本。
//! - `docs/ledger/model-no-compat-notes.md`（本线的逐项复审表、真实错误文本与 contract 清单）。
//!
//! 本文件是本线交给下游（`schemas/**` 集成者、`yeban-app`、`yeban-mcp`）的
//! **可执行契约**：只要本文件全绿，"实现要求的字段集合"就是可机械核对的。
//!
//! 判据清单：
//!
//! | # | 判据 | 测的是什么 |
//! | :-- | :--- | :--- |
//! | ① | [`every_removed_default_is_now_required_with_exact_field_name`] | 38 个项目字段 + 3 个 `CommitGraph` 字段：**逐个**缺键 ⇒ 报错，错误文本逐个点名 |
//! | ② | [`tolerated_defaults_are_accepted_when_absent_and_take_declared_values`] | 18 个保留 default 的字段：缺键 ⇒ **成功**且取到声明默认值（证明没有过度收紧） |
//! | ③ | [`removing_a_tolerated_default_from_the_filled_project_still_parses`] | 从一个**完整工程**里删掉保留 default 的键 ⇒ 仍然可读 |
//! | ④ | [`production_writer_bytes_round_trip_exactly`] | 本写入器产出的字节 → 读回 → 再写出，**逐字节相同**（宽容读与省略写成对） |
//! | ⑤ | [`yeban_container_round_trips_the_strict_project`] | `.yeban` 容器往返（工程 + `history.dag` + 写入确定性） |
//! | ⑥ | [`schema_version_mismatch_is_rejected_and_never_migrated`] | `schema_version` / `min_reader_version` 不匹配 ⇒ 明确拒绝 |
//! | ⑦ | [`canonical_samples_are_still_legal_and_byte_stable`] | `samples::export_all` 的四份规范样本仍然全部合法且逐字节稳定 |

use std::collections::BTreeMap;
use std::str::FromStr as _;

use serde_json::Value;
use yeban_model::container::{
    ContainerEntry, ContainerLimits, HISTORY_DAG_NAME, PROJECT_JSON_NAME, read_project_container,
    write_container, write_project_container,
};
use yeban_model::samples::{default_out_dir, default_project, export_all, filled_project};
use yeban_model::{
    AssetHash, AutomationLane, CommitGraph, EntityId, MidiNote, ModelError, ParameterValue,
    ProjectMetadata, READER_SCHEMA_VERSION, RoutingEdge, SCHEMA_VERSION, SceneV3, SectionV3,
    TrackV3, YebanProjectV1,
};

/// 构造确定性的规范 ULID 文本（与 crate 内夹具同一口径）。
fn id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

// ---------------------------------------------------------------------------
// 路径工具：`*` = "按确定顺序取第一个能让剩余路径成立的子值"
// ---------------------------------------------------------------------------

/// 把含 `*` 的路径解析成**具体**路径（逐段的键或下标）。
///
/// `*` 的语义是"第一个能让剩余路径成立的子值"，因此它既能穿过 `BTreeMap` 对象
/// （键序确定），也能穿过 `Vec` 数组（下标确定），还能穿过**枚举变体** —— 例如
/// `clip_pool/*/content/Midi/notes` 会跳过 `ClipContent::Audio` 的条目。
fn resolve(value: &Value, path: &[&str]) -> Option<Vec<String>> {
    let Some((head, rest)) = path.split_first() else {
        return Some(Vec::new());
    };
    match value {
        Value::Object(map) => {
            if *head == "*" {
                for (key, child) in map {
                    if let Some(tail) = resolve(child, rest) {
                        let mut full = vec![key.clone()];
                        full.extend(tail);
                        return Some(full);
                    }
                }
                None
            } else if rest.is_empty() {
                map.contains_key(*head).then(|| vec![(*head).to_owned()])
            } else {
                let tail = resolve(map.get(*head)?, rest)?;
                let mut full = vec![(*head).to_owned()];
                full.extend(tail);
                Some(full)
            }
        }
        Value::Array(items) => {
            if *head == "*" {
                for (index, child) in items.iter().enumerate() {
                    if let Some(tail) = resolve(child, rest) {
                        let mut full = vec![index.to_string()];
                        full.extend(tail);
                        return Some(full);
                    }
                }
                None
            } else {
                let index = head.parse::<usize>().ok()?;
                let child = items.get(index)?;
                if rest.is_empty() {
                    Some(vec![index.to_string()])
                } else {
                    let tail = resolve(child, rest)?;
                    let mut full = vec![index.to_string()];
                    full.extend(tail);
                    Some(full)
                }
            }
        }
        _ => None,
    }
}

/// 按**具体**路径删除一个键（返回是否真的删掉了）。
fn remove_concrete(value: &mut Value, path: &[String]) -> bool {
    let Some((head, rest)) = path.split_first() else {
        return false;
    };
    if rest.is_empty() {
        return match value {
            Value::Object(map) => map.remove(head).is_some(),
            Value::Array(items) => head
                .parse::<usize>()
                .ok()
                .filter(|index| *index < items.len())
                .map(|index| items.remove(index))
                .is_some(),
            _ => false,
        };
    }
    match value {
        Value::Object(map) => map
            .get_mut(head)
            .is_some_and(|child| remove_concrete(child, rest)),
        Value::Array(items) => head
            .parse::<usize>()
            .ok()
            .and_then(|index| items.get_mut(index))
            .is_some_and(|child| remove_concrete(child, rest)),
        _ => false,
    }
}

fn concrete(value: &Value, path: &str) -> Vec<String> {
    let steps: Vec<&str> = path.split('/').collect();
    resolve(value, &steps).unwrap_or_else(|| panic!("路径 `{path}` 在夹具 JSON 里解析不到"))
}

// ---------------------------------------------------------------------------
// ① 被删掉 `#[serde(default)]` 的字段：逐个必需
// ---------------------------------------------------------------------------

/// 被删掉 `#[serde(default)]` 的字段表：`(字段名, 填充工程里的 JSON 路径)`。
///
/// 38 项 = `YebanProjectV1` 10 + `ProjectMetadata` 2 + `TransportConfig` 3
/// + `DeviceDefinition` 3 + `MacroParameter` 3 + `AutomationLane` 3
/// + `AutomationPoint` 1 + `LoopConfig` 3 + `ClipPoolEntry` 1
/// + `ClipContent::Midi` 1 + `ClipContent::Audio` 1 + `ClipPlacement` 2 + `TrackV3` 5。
///
/// 另有 `CommitGraph` 3 项（见 [`commit_graph_removed_defaults_are_required`]），故被删
/// default 的字段合计 41 项。
const REQUIRED_FIELDS: &[(&str, &str)] = &[
    // YebanProjectV1（顶层 10 项）
    ("rng_seed", "rng_seed"),
    ("metadata", "metadata"),
    ("transport", "transport"),
    ("sections", "sections"),
    ("tracks", "tracks"),
    ("master_bus_track_id", "master_bus_track_id"),
    ("routing_graph", "routing_graph"),
    ("scenes", "scenes"),
    ("clip_pool", "clip_pool"),
    ("assets", "assets"),
    // ProjectMetadata（2 项；`description` / `tags` 是注记，保留 default）
    ("created_at_unix_ms", "metadata/created_at_unix_ms"),
    ("modified_at_unix_ms", "metadata/modified_at_unix_ms"),
    // TransportConfig（3 项，全部必需）
    ("metronome_enabled", "transport/metronome_enabled"),
    ("count_in_bars", "transport/count_in_bars"),
    ("launch_quantization", "transport/launch_quantization"),
    // DeviceDefinition（3 项）
    ("bypassed", "tracks/*/devices/0/bypassed"),
    ("params", "tracks/*/devices/0/params"),
    ("latency_samples", "tracks/*/devices/0/latency_samples"),
    // MacroParameter（3 项，全部必需）
    ("name", "tracks/*/macros/0/name"),
    ("value", "tracks/*/macros/0/value"),
    ("mappings", "tracks/*/macros/0/mappings"),
    // AutomationLane（3 项必需；`domain` 是 Option，保留 default）
    ("points", "tracks/*/automation_lanes/0/points"),
    ("read_enabled", "tracks/*/automation_lanes/0/read_enabled"),
    ("write_mode", "tracks/*/automation_lanes/0/write_mode"),
    // AutomationPoint（1 项）
    ("curve", "tracks/*/automation_lanes/0/points/*/curve"),
    // LoopConfig（3 项，全部必需）
    ("enabled", "tracks/*/clips/*/loop_config/enabled"),
    ("start_tick", "tracks/*/clips/*/loop_config/start_tick"),
    ("end_tick", "tracks/*/clips/*/loop_config/end_tick"),
    // ClipPoolEntry（1 项）
    ("name", "clip_pool/*/name"),
    // ClipContent
    ("notes", "clip_pool/*/content/Midi/notes"),
    ("gain_db", "clip_pool/*/content/Audio/gain_db"),
    // ClipPlacement（2 项，全部必需）
    ("loop_config", "tracks/*/clips/*/loop_config"),
    ("muted", "tracks/*/clips/*/muted"),
    // TrackV3（5 项必需；`folder_id` / `color` 是 Option，保留 default）
    ("solo_safe", "tracks/*/solo_safe"),
    ("devices", "tracks/*/devices"),
    ("macros", "tracks/*/macros"),
    ("automation_lanes", "tracks/*/automation_lanes"),
    ("clips", "tracks/*/clips"),
];

/// 逐个字段：从**完整规范样本**里删掉该键，`serde_json::from_str::<YebanProjectV1>`
/// 必须失败，且错误文本必须精确点名缺失的字段。
///
/// 用**完整样本**（而不是手写小对象）是关键：它同时证明"这些键本来就在写出的字节里"，
/// 因此"必需"不会让本写入器读不了自己的输出。走 `from_str` 而不是 `from_value`，
/// 是为了让错误文本带上**位置**（`at line 1 column N`）—— 那才是磁盘上真文件的报错形状。
fn required_field_error_texts() -> Vec<(String, String, String)> {
    let project = serde_json::to_value(filled_project()).expect("规范样本必须可序列化");
    let mut rows = Vec::new();
    for (field, path) in REQUIRED_FIELDS {
        let target = concrete(&project, path);
        let mut mutated = project.clone();
        assert!(
            remove_concrete(&mut mutated, &target),
            "表错了：`{field}` 的路径 `{path}` 在规范样本里删不掉（解析结果 {target:?}）"
        );
        let text = serde_json::to_string(&mutated).expect("序列化");
        let error = match serde_json::from_str::<YebanProjectV1>(&text) {
            Ok(_) => panic!("过度宽容：缺 `{field}`（路径 {path}）竟然读成功了"),
            Err(error) => error,
        };
        rows.push(((*field).to_owned(), (*path).to_owned(), error.to_string()));
    }
    rows
}

/// ① 逐字段断言（每项一条判据，失败信息里带字段名与路径）。
#[test]
fn every_removed_default_is_now_required_with_exact_field_name() {
    let rows = required_field_error_texts();
    assert_eq!(
        rows.len(),
        REQUIRED_FIELDS.len(),
        "所有被删 default 的字段都必须被逐个覆盖"
    );
    for (field, path, text) in &rows {
        let expected = format!("missing field `{field}`");
        assert!(
            text.contains(&expected),
            "字段 `{field}`（路径 `{path}`）缺键必须报「{expected}」，实测：{text}"
        );
    }
}

/// ①b `CommitGraph` 的三个集合（`history.dag` 的载荷）同样逐个必需。
#[test]
fn commit_graph_removed_defaults_are_required() {
    let graph = serde_json::to_value(CommitGraph::new()).expect("空图谱可序列化");
    for field in ["commits", "branches", "depths"] {
        let mut mutated = graph.clone();
        assert!(
            mutated
                .as_object_mut()
                .expect("graph 是对象")
                .remove(field)
                .is_some(),
            "空图谱必须写出 `{field}`（否则本写入器读不了自己的输出）"
        );
        let text = serde_json::to_string(&mutated).expect("序列化");
        let error = serde_json::from_str::<CommitGraph>(&text).expect_err("缺必需字段必须失败");
        let expected = format!("missing field `{field}`");
        assert!(
            error.to_string().contains(&expected),
            "缺 `{field}` 必须报「{expected}」，实测：{error}"
        );
    }
}

/// ①c 供 notes 逐字引用错误文本表：
/// `cargo test -p yeban-model --test no_compat -- --ignored --nocapture`
#[test]
#[ignore = "只为生成 notes 里的错误文本表, 不参与门禁"]
fn print_exact_error_text_table_for_the_notes() {
    for (field, path, text) in required_field_error_texts() {
        println!("{field} | {path} | {text}");
    }
    let graph = serde_json::to_value(CommitGraph::new()).expect("空图谱可序列化");
    for field in ["commits", "branches", "depths"] {
        let mut mutated = graph.clone();
        mutated.as_object_mut().expect("对象").remove(field);
        let text = serde_json::to_string(&mutated).expect("序列化");
        let error = serde_json::from_str::<CommitGraph>(&text).expect_err("必须失败");
        println!("{field} | history.dag | {error}");
    }
    // 容器路径的报错形状（`ContainerError::InvalidProjectJson` 带 serde 原始文本）。
    let mut mutated = serde_json::to_value(filled_project()).expect("序列化");
    let target = concrete(&mutated, "rng_seed");
    assert!(remove_concrete(&mut mutated, &target));
    let manifest = write_container(&[
        ContainerEntry::new(
            PROJECT_JSON_NAME,
            serde_json::to_vec(&mutated).expect("序列化"),
        ),
        ContainerEntry::new(HISTORY_DAG_NAME, b"{}".to_vec()),
    ])
    .expect("写容器");
    let error =
        read_project_container(&manifest, &ContainerLimits::default()).expect_err("必须失败");
    println!("rng_seed | .yeban 容器 | {error}");
}

// ---------------------------------------------------------------------------
// ② 保留 default 的字段：缺键 ⇒ 成功且取到声明默认值
// ---------------------------------------------------------------------------

/// ② 18 个保留 `#[serde(default)]` 的字段逐个"缺键 ⇒ 成功 + 声明默认值"。
///
/// 这一条专门证明**没有过度收紧**：D43 允许 `Option<T>` 与"空串/空集 = 无注记"
/// 保留语义默认，若把它们也变成必需，本判据会立刻变红。
#[test]
fn tolerated_defaults_are_accepted_when_absent_and_take_declared_values() {
    // 1) `YebanProjectV1::author`（自由文本注记："未署名"）
    let mut project = serde_json::to_value(default_project()).expect("默认工程可序列化");
    assert!(
        project
            .as_object_mut()
            .expect("对象")
            .remove("author")
            .is_some(),
        "默认工程必须写出 `author` 的确定编码（空串）"
    );
    let parsed: YebanProjectV1 = serde_json::from_value(project).expect("缺 author 必须可读");
    assert_eq!(parsed.author, "", "author 缺失必须取声明默认（空串）");

    // 2) `ProjectMetadata::{description, tags}`（注记）
    let parsed: ProjectMetadata =
        serde_json::from_str(r#"{"created_at_unix_ms":0,"modified_at_unix_ms":0}"#)
            .expect("缺 description/tags 必须可读");
    assert_eq!(parsed.description, "");
    assert!(parsed.tags.is_empty());

    // 3) `ParameterValue::unit`（Option：None = "无量纲/未标注"）
    let parsed: ParameterValue =
        serde_json::from_str(r#"{"name":"cutoff","value":1200.0}"#).expect("缺 unit 必须可读");
    assert_eq!(parsed.unit, None);

    // 4) `AutomationLane::domain`（Option：None = "派生自目标"）
    let parsed: AutomationLane = serde_json::from_value(serde_json::json!({
        "target": { "TrackVolume": { "track_id": id(2).to_string() } },
        "points": {},
        "read_enabled": true,
        "write_mode": "Off"
    }))
    .expect("缺 domain 必须可读");
    assert_eq!(parsed.domain, None);
    assert_eq!(
        parsed.effective_domain(),
        parsed.target.nominal_domain(),
        "domain=None 必须回落到目标的固有值域"
    );

    // 5) `TrackV3::{folder_id, color}`（Option：None = "不折叠" / "用主题默认色"）
    let parsed: TrackV3 = serde_json::from_value(serde_json::json!({
        "id": id(3).to_string(),
        "name": "Lead",
        "kind": "Midi",
        "volume_db": 0.0,
        "pan": 0.0,
        "mute": false,
        "solo": false,
        "solo_safe": false,
        "devices": [],
        "macros": [],
        "automation_lanes": [],
        "clips": {}
    }))
    .expect("缺 folder_id/color 必须可读");
    assert_eq!(parsed.folder_id, None);
    assert_eq!(parsed.color, None);

    // 6) `SectionV3::color`
    let parsed: SectionV3 = serde_json::from_value(serde_json::json!({
        "id": id(70).to_string(),
        "name": "Intro",
        "start_tick": 0,
        "end_tick": 960
    }))
    .expect("缺 color 必须可读");
    assert_eq!(parsed.color, None);

    // 7) `SceneV3::{tempo, color}`（None = "跟随工程速度" / "用主题默认色"）
    let parsed: SceneV3 = serde_json::from_value(serde_json::json!({
        "id": id(80).to_string(),
        "name": "Scene 1"
    }))
    .expect("缺 tempo/color 必须可读");
    assert_eq!(parsed.tempo, None);
    assert_eq!(parsed.color, None);

    // 8) `RoutingEdge::gain_db`（None = 单位增益；对比裸 `f32` 的 `ClipContent::Audio::gain_db`，
    //    后者因为不可区分而被 D43 改成必需）
    let parsed: RoutingEdge = serde_json::from_value(serde_json::json!({
        "id": id(60).to_string(),
        "source_node": id(2).to_string(),
        "destination_node": id(1).to_string(),
        "kind": "TrackToBus"
    }))
    .expect("缺 gain_db 必须可读");
    assert_eq!(parsed.gain_db, None);

    // 9) `MidiNote` 的七个表现力字段：本写入器在默认值上**不落盘**（稀疏编码），
    //    因此"缺键"就是它的正常输出形状 —— 必须仍然可读。
    let note = MidiNote::new(id(100), 0, 60, 480);
    let json = serde_json::to_string(&note).expect("音符可序列化");
    for omitted in [
        "probability",
        "ratchet",
        "micro_timing_ticks",
        "slide",
        "pitch_bend_curve",
        "syllable",
        "phonemes",
    ] {
        assert!(
            !json.contains(omitted),
            "默认值 `{omitted}` 必须不落盘（稀疏编码），实测 {json}"
        );
    }
    let parsed: MidiNote = serde_json::from_str(&json).expect("缺七个可选字段必须可读");
    assert_eq!(parsed.probability, None);
    assert_eq!(parsed.ratchet, None);
    assert_eq!(parsed.micro_timing_ticks, None);
    assert_eq!(parsed.slide, None);
    assert!(parsed.pitch_bend_curve.is_empty());
    assert_eq!(parsed.syllable, None);
    assert!(parsed.phonemes.is_empty());
}

// ---------------------------------------------------------------------------
// ③ 从完整工程里删掉保留 default 的键 ⇒ 仍然可读
// ---------------------------------------------------------------------------

/// 保留 default 的字段在规范样本里"实际存在"的那 13 条路径（其余 5 条在样本里本来
/// 就不落盘 —— `folder_id` / `SceneV3::color` / `MidiNote::slide` / `pitch_bend_curve`
/// / `phonemes` —— 它们由判据 ② 的"缺键即读"覆盖）。
const TOLERATED_PATHS: &[&str] = &[
    "author",
    "metadata/description",
    "metadata/tags",
    "tracks/*/devices/0/params/0/unit",
    "tracks/*/automation_lanes/0/domain",
    "tracks/*/color",
    "sections/*/color",
    "scenes/*/tempo",
    "routing_graph/edges/*/gain_db",
    "clip_pool/*/content/Midi/notes/*/probability",
    "clip_pool/*/content/Midi/notes/*/ratchet",
    "clip_pool/*/content/Midi/notes/*/micro_timing_ticks",
    "clip_pool/*/content/Midi/notes/*/syllable",
];

/// ③ 逐个删掉"保留 default"的键之后，整份工程**仍然必须读得进来**。
#[test]
fn removing_a_tolerated_default_from_the_filled_project_still_parses() {
    let project = serde_json::to_value(filled_project()).expect("规范样本必须可序列化");
    for path in TOLERATED_PATHS {
        let target = concrete(&project, path);
        let mut mutated = project.clone();
        assert!(
            remove_concrete(&mut mutated, &target),
            "表错了：`{path}` 在规范样本里删不掉"
        );
        let parsed: YebanProjectV1 = serde_json::from_value(mutated).unwrap_or_else(|error| {
            panic!("过度收紧：删掉可选键 `{path}` 之后工程读不回来了：{error}")
        });
        assert_eq!(
            parsed.validate(),
            Ok(()),
            "删掉可选键 `{path}` 之后工程必须仍然合法"
        );
    }
}

// ---------------------------------------------------------------------------
// ④ 本写入器产出的字节 → 读回 → 再写出，逐字节相同
// ---------------------------------------------------------------------------

/// ④ 规范样本与默认工程：`to_string` → `from_str` → `to_string` **逐字节相同**。
///
/// 这就是"宽容读与省略写成对"的机械证明：写入器省略的每一个键，读取器都必须接受；
/// 一旦哪个被省略的键变成必需（或反过来，哪个必需的键被写成可省略），本判据立刻变红。
#[test]
fn production_writer_bytes_round_trip_exactly() {
    for (label, project) in [
        ("filled_project", filled_project()),
        ("default_project", default_project()),
    ] {
        let first = serde_json::to_string(&project).expect("序列化");
        let back: YebanProjectV1 = serde_json::from_str(&first)
            .unwrap_or_else(|error| panic!("{label} 读不回来：{error}"));
        assert_eq!(back, project, "{label} 往返后必须相等");
        let second = serde_json::to_string(&back).expect("再序列化");
        assert_eq!(first, second, "{label} 往返必须逐字节相同");
        // 美化形态（落盘口径）也必须逐字节稳定。
        let pretty_first = serde_json::to_string_pretty(&project).expect("美化");
        let pretty_back: YebanProjectV1 = serde_json::from_str(&pretty_first).expect("读回");
        assert_eq!(
            serde_json::to_string_pretty(&pretty_back).expect("再美化"),
            pretty_first,
            "{label} 美化往返必须逐字节相同"
        );
    }
}

// ---------------------------------------------------------------------------
// ⑤ `.yeban` 容器往返
// ---------------------------------------------------------------------------

/// ⑤ 工程 + `history.dag` 装进 `.yeban` 容器，读回后工程相等、dag 逐字节相同，
/// 且同一输入两次写入的归档字节相同（`ARCH-DET-001`）。
#[test]
fn yeban_container_round_trips_the_strict_project() {
    let project = filled_project();
    let dag = serde_json::to_vec(&CommitGraph::new()).expect("dag 序列化");
    let assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();

    let bytes = write_project_container(&project, &dag, &assets).expect("写容器");
    let again = write_project_container(&project, &dag, &assets).expect("再写容器");
    assert_eq!(
        bytes, again,
        "同一输入两次写入必须逐字节相同 [ARCH-DET-001]"
    );

    let archive =
        read_project_container(&bytes, &ContainerLimits::default()).expect("读回容器必须成功");
    assert_eq!(archive.project, project, "容器往返后工程必须相等");
    assert_eq!(archive.history_dag, dag, "history.dag 必须逐字节相同");
    assert!(archive.assets.is_empty());

    // 严格性在容器路径上同样生效：把工程 JSON 里一个必需键删掉再装进去，必须被拒。
    let mut mutated = serde_json::to_value(&project).expect("序列化");
    let target = concrete(&mutated, "rng_seed");
    assert!(remove_concrete(&mut mutated, &target));
    let broken_json = serde_json::to_vec(&mutated).expect("序列化");
    let manifest = write_container(&[
        ContainerEntry::new(PROJECT_JSON_NAME, broken_json),
        ContainerEntry::new(HISTORY_DAG_NAME, dag.clone()),
    ])
    .expect("写容器");
    let error = read_project_container(&manifest, &ContainerLimits::default())
        .expect_err("缺 rng_seed 的容器必须被拒");
    assert!(
        error.to_string().contains("missing field `rng_seed`"),
        "实测：{error}"
    );
}

// ---------------------------------------------------------------------------
// ⑥ 版本门：只用于拒绝不匹配
// ---------------------------------------------------------------------------

/// ⑥ `schema_version` / `min_reader_version` 不匹配 ⇒ 明确拒绝，绝不"迁移"或"猜"。
///
/// D43 第 4 条：版本号只承担**拒绝**职责，不承担"兼容多版本"职责。因此这里同时钉住
/// 正面（新版本被拒）与反面（本版本文档被接受、且常量恒为 1）。
#[test]
fn schema_version_mismatch_is_rejected_and_never_migrated() {
    assert_eq!(
        SCHEMA_VERSION, 1,
        "首个稳定 schema 版本恒为 1 [ADR-0001 D3]"
    );
    assert_eq!(READER_SCHEMA_VERSION, 1);

    let mut project = filled_project();
    project.schema_version = SCHEMA_VERSION + 1;
    let json = serde_json::to_string(&project).expect("序列化");
    // 版本号是**数据**，反序列化本身不拦（拦在这里会把"文件损坏"和"版本太新"混为一谈）。
    let parsed: YebanProjectV1 = serde_json::from_str(&json).expect("版本号是数据, 解析不拦");
    let error = parsed.check_readable().expect_err("更新版本必须被拒");
    assert!(
        matches!(
            error,
            ModelError::SchemaVersionTooNew {
                found: 2,
                supported: 1
            }
        ),
        "实测：{error:?}"
    );
    assert!(
        error.to_string().contains("schema_version"),
        "错误文本必须点名 schema_version，实测：{error}"
    );

    let mut project = filled_project();
    project.min_reader_version = READER_SCHEMA_VERSION + 1;
    let json = serde_json::to_string(&project).expect("序列化");
    let parsed: YebanProjectV1 = serde_json::from_str(&json).expect("解析");
    let error = parsed
        .check_readable()
        .expect_err("更高的读取器要求必须被拒");
    assert!(
        matches!(
            error,
            ModelError::ReaderTooOld {
                required: 2,
                actual: 1
            }
        ),
        "实测：{error:?}"
    );

    // 本版本 + 本读取器要求 ⇒ 接受。
    assert_eq!(filled_project().check_readable(), Ok(()));

    // 版本号缺失同样是硬错误（它没有 `#[serde(default)]`，也不该有）。
    let mut missing = serde_json::to_value(filled_project()).expect("序列化");
    assert!(
        missing
            .as_object_mut()
            .expect("对象")
            .remove("schema_version")
            .is_some()
    );
    let error =
        serde_json::from_value::<YebanProjectV1>(missing).expect_err("缺 schema_version 必须被拒");
    assert!(
        error.to_string().contains("missing field `schema_version`"),
        "实测：{error}"
    );
}

// ---------------------------------------------------------------------------
// ⑦ 规范样本仍然全部合法
// ---------------------------------------------------------------------------

/// ⑦ `samples::export_all` 的四份规范样本：写盘成功（内含 `validate()` +
/// `check_readable()`）、可逐份读回、且两次导出逐字节相同。
#[test]
fn canonical_samples_are_still_legal_and_byte_stable() {
    let base = default_out_dir().join("no-compat-judgement");
    let first_dir = base.join("first");
    let second_dir = base.join("second");

    let first = export_all(&first_dir).expect("规范样本必须全部合法");
    let second = export_all(&second_dir).expect("第二次导出必须同样合法");
    assert_eq!(first.len(), 4, "规范样本恒为四份");

    for (left, right) in first.iter().zip(second.iter()) {
        let left_bytes = std::fs::read(left).expect("读第一个样本");
        let right_bytes = std::fs::read(right).expect("读第二个样本");
        assert_eq!(
            left_bytes,
            right_bytes,
            "{} 两次导出必须逐字节相同",
            left.display()
        );
    }

    // 两份工程样本必须能原样读回并合法。
    for name in ["project.default.json", "project.filled.json"] {
        let text = std::fs::read_to_string(first_dir.join(name)).expect("读工程样本");
        let project: YebanProjectV1 =
            serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name} 读不回来：{error}"));
        assert_eq!(project.validate(), Ok(()), "{name} 必须合法");
        assert_eq!(project.check_readable(), Ok(()), "{name} 必须可读");
        assert_eq!(
            serde_json::to_string_pretty(&project).expect("再序列化") + "\n",
            text,
            "{name} 必须逐字节稳定"
        );
    }

    // 删除前先确认目标就是本判据自己创建的那个子目录（不做任何计算路径的删除）。
    assert_eq!(
        base.file_name().and_then(std::ffi::OsStr::to_str),
        Some("no-compat-judgement"),
        "拒绝删除非本判据创建的目录：{}",
        base.display()
    );
    std::fs::remove_dir_all(&base).expect("清理临时样本目录");
}
