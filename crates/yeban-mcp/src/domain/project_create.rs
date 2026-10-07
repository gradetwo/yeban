//! "从零建一个可渲染的工程"—— `yeban_open_project` 的 **`create: true`** 分支
//! [MCP-TOOL-001]。
//!
//! ## 为什么放在 `yeban_open_project` 而不新增工具
//!
//! 交付形态是"从一个**不存在的路径**开始"（`ADR-0001` D46 的扩张原则：
//! 先扩既有工具的参数，只有确实不合适才新增工具）。`yeban_open_project` 已经拥有
//! 三样本能力需要的东西：`path` 实参、`.yeban.lock` 的取锁协议、以及"打开后成为
//! 会话活跃工程"的落点。`create: true` 因此是"打开一个**新建的**工程"，
//! 不是第二个入口 ⇒ `TOOL_COUNT` 保持 **17**、契约的
//! `definitions.ToolCall.properties.name.enum` 一字不动、没有任何新工具 ID。
//!
//! ## ⚠ 本模块**不**改模型层：主总线身份**不是**任何 `Op` 的载荷
//!
//! 逐条事实（`crates/yeban-model/src/ops.rs`）：
//!
//! - [`Op::AddTrack`] 的前置条件是"音轨自校验通过 + `id` 不重复"（`ops.rs:634`），
//!   它的 `apply` 只做 `doc.insert_track(...)`（`ops.rs:1329`）——
//!   **一次都不写** `master_bus_track_id`；
//! - [`Op::AddRoutingNode`] 只往 `routing_graph.nodes` 里插一个身份（`ops.rs:679`）；
//! - [`Op::ConnectRouting`] 只检查"两端都在 `nodes` 里"再插一条边（`ops.rs:703`）；
//! - 全仓 `grep -rn "master_bus_track_id" crates/yeban-model/src/ops.rs` 的 5 处命中里，
//!   唯一的**写入**是测试夹具（`ops.rs:1820`），其余都是**读**（`ops.rs:645` 的
//!   `RemoveTrack` 前置条件 + `ops.rs:3287` 起的属性测试取样）。
//!
//! ⇒ 模型层的 `Op` 全集**无法**表达"把某条音轨设成主总线"。本能力因此与模型的
//! 既有样本构造器（`yeban_model::samples::filled_project()` /
//! `demo_project()`，两者都直接写 `master_bus_track_id`）走**同一条**口径：
//! **模板文档在作者手里定型**，随后的内容改动才走 `Op`。
//! 这条边界在 `docs/ledger/feature-alignment.md` 登记，本线**没有**新增 `Op` 变体
//! （那要同步 `ops.rs` + `schemas/ops.schema.json` + `error.rs` 穷举映射 + showcase）。
//!
//! ## 建出来的规范形态（默认值，全部字段都是确定性的）
//!
//! ```text
//! master_bus_track_id = deterministic_id("project:{title}:master")
//! tracks[master]      = TrackV3 { kind: Master, name: "Master" }
//! routing_graph.nodes = [master]                     ← 主总线**已在图里**
//! routing_graph.edges = []                           ← 还没有任何声部
//! tracks[track-0]     = TrackV3 { kind: Midi, name: "Track 1" }
//! clip_pool[clip-0]   = ClipContent::Midi { 4 个音符 (C4 D4 E4 G4, 各 1 拍) }
//! tracks[track-0].clips[placement-0] = 起点 0, 时值 1 小节
//! routing_graph.edges[edge-0] = track-0 → master (TrackToBus)
//! ```
//!
//! 三条刻意的口径：
//!
//! 1. **主总线装进 `routing_graph.nodes`**：`render_master` 的
//!    `RenderPlan::compile` 要求 `master` 在节点表里（`yeban-render` 的
//!    `RenderError::MasterNotInGraph`，`render.rs:444`），而
//!    `propose_section` 又会**跳过**已经在节点表里的主总线
//!    （`section_build.rs:945` 的 `if !project.routing_graph.nodes.contains(&bus)`）
//!    —— 两个前提因此同时被满足，且不会产生"重复节点"的 `DUPLICATE_ENTITY_ID`；
//! 2. **默认给内容**：一条 MIDI 轨 + 4 个音符 + 一拍一音的摆放。没有内容的工程
//!    渲染必然 `RENDER_FAILED`（"工程里没有可渲染的内容 (0 帧)"，
//!    `render.rs:1190`），那样"可渲染"就只是句口号；
//! 3. **`propose_section` 仍然要材料**：它要求 `clip_pool` 里至少有一条
//!    MIDI 片段（`section_build.rs:812`，缺了报 `CLIP_NOT_FOUND`）。
//!    默认的 `clip-0` 就是配器骨架的第一份材料 —— 新建之后 `propose_section`
//!    可以直接跑，不需要先绕 `yeban_import_audio`（那个工具只登记 **Audio**
//!    片段，`import_audio.rs:23`，永远满足不了 MIDI 材料的要求）。
//!
//! ## 幂等 / 安全：已存在的文件**明确拒绝**
//!
//! 与 `yeban_save_project` 的语义对齐（那个工具是"刷盘到**当前**工程路径"，
//! 本工具是"写一个**新**文件"）：
//!
//! - `create: true` 且目标路径**已存在** ⇒ `CONFLICT`（`ADR-0001` D25 的 20 值联集里
//!   的既有码），`data.reason = "projectAlreadyExists"`。**绝不**静默覆盖；
//! - `create: true` 与 `readOnly: true` 同给 ⇒ `INVALID_PARAMETER_RANGE`
//!   （只读地新建一个文件是自相矛盾的要求）；
//! - 落盘仍走**唯一**入口 `store::write_project_atomic`（同目录临时文件 + `fsync` +
//!   `rename`，`ARCH-SEC-004`），本模块不自己拼 ZIP、也不自己写盘。

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, Op, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

use super::error::Fault;
use super::ids::deterministic_id;
use super::section_build;
use crate::tools::ErrorCode;

/// 本能力的规范 ID（只用于响应 `data` 与台账的注记，**不新增工具 ID**）。
pub const SPEC_ID: &str = "MCP-TOOL-001";

/// 默认轨道数（1 条 MIDI 轨 + 模板自带的主总线）。
pub const DEFAULT_TRACK_COUNT: u64 = 1;

/// `trackCount` 的上限。
///
/// 与 `section_build::MAX_PARTS` 同源（"一个段落最多几个声部"），
/// 因此不会出现"建得出来、配不出来"的工程。
pub const MAX_TRACK_COUNT: u64 = section_build::MAX_PARTS as u64;

/// 默认种子音符：`(音高, 时值 tick)` —— C4 D4 E4 G4，各 1 拍（960 PPQ）。
pub const DEFAULT_SEED_NOTES: [(u8, u64); 4] = [(60, 960), (62, 960), (64, 960), (67, 960)];

/// `create: true` 时的**可选最小内容**。
///
/// 全部字段都有默认值 ⇒ `create: true` 单独给就足以产出"可渲染、可配器"的工程。
#[derive(Clone, Debug, PartialEq)]
pub struct CreateConfig {
    /// 工程标题（空串 ⇒ 模型默认 `Untitled`）。
    pub title: String,
    /// 速度（BPM）。`None` ⇒ 模型默认 120。
    pub bpm: Option<f64>,
    /// MIDI 轨道数（`1..=MAX_TRACK_COUNT`，默认 1）。
    pub track_count: u64,
    /// 第一个片段的名字（默认 `Motif`）。
    pub clip_name: String,
    /// 种子音符 `(音高 0..=127, 时值 tick >= 1)`；空 ⇒ 默认四个音。
    pub notes: Vec<(u8, u64)>,
}

impl Default for CreateConfig {
    fn default() -> Self {
        Self {
            title: String::new(),
            bpm: None,
            track_count: DEFAULT_TRACK_COUNT,
            clip_name: "Motif".to_owned(),
            notes: DEFAULT_SEED_NOTES.to_vec(),
        }
    }
}

/// 一次建工程的结果（文档 + 它的一份可复算描述）。
#[derive(Clone, Debug, PartialEq)]
pub struct CreatedProject {
    /// 已通过 `validate()` 的工程文档。
    pub project: YebanProjectV1,
    /// 主总线音轨身份（**非 nil**，且在 `routing_graph.nodes` 里）。
    pub master_bus_track_id: EntityId,
    /// 内容改动批次的规范描述（`AddClip` / `AddClipPlacement` / `AddRoutingNode` /
    /// `ConnectRouting`），用于响应 `data.seed`。
    pub ops: Vec<Op>,
}

/// 解析 `create: true` 的可选实参。
///
/// # Errors
///
/// - `title` / `clipName` 不是字符串，或 `bpm` / `trackCount` 不是数字 ⇒
///   [`ErrorCode::InvalidParameterRange`]；
/// - `bpm` 超出模型区间（`20.0..=999.0`）或非有限 ⇒ `INVALID_PARAMETER_RANGE`
///   （与模型 [`yeban_model::error::ModelError::BpmOutOfRange`] 同一条边界，
///   在这里提前拒绝，避免写下一份 `validate()` 会拒的文档）；
/// - `trackCount` 不在 `1..=MAX_TRACK_COUNT` ⇒ `INVALID_PARAMETER_RANGE`；
/// - `notes` 的任一条目形状非法、音高越界、时值为 0 ⇒ `INVALID_PARAMETER_RANGE`。
pub fn parse_config(
    title: Option<&Value>,
    bpm: Option<&Value>,
    track_count: Option<&Value>,
    clip_name: Option<&Value>,
    notes: Option<&Value>,
) -> Result<CreateConfig, Fault> {
    let mut config = CreateConfig::default();
    if let Some(value) = title {
        config.title = value
            .as_str()
            .ok_or_else(|| invalid("`title` 必须是字符串"))?
            .to_owned();
    }
    if let Some(value) = clip_name {
        config.clip_name = value
            .as_str()
            .ok_or_else(|| invalid("`seed.clipName` 必须是字符串"))?
            .to_owned();
    }
    if let Some(value) = bpm {
        let bpm = value.as_f64().ok_or_else(|| invalid("`bpm` 必须是数字"))?;
        if !bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            return Err(invalid(&format!(
                "`bpm` 必须在 {MIN_BPM}..={MAX_BPM}, 实际 {bpm}"
            )));
        }
        config.bpm = Some(bpm);
    }
    if let Some(value) = track_count {
        let count = value
            .as_u64()
            .ok_or_else(|| invalid("`seed.trackCount` 必须是非负整数"))?;
        if count == 0 || count > MAX_TRACK_COUNT {
            return Err(invalid(&format!(
                "`seed.trackCount` 必须在 1..={MAX_TRACK_COUNT}, 实际 {count}"
            )));
        }
        config.track_count = count;
    }
    if let Some(value) = notes {
        let array = value
            .as_array()
            .ok_or_else(|| invalid("`seed.notes` 必须是数组"))?;
        let mut parsed: Vec<(u8, u64)> = Vec::with_capacity(array.len());
        for (index, entry) in array.iter().enumerate() {
            let object = entry
                .as_object()
                .ok_or_else(|| invalid(&format!("`seed.notes[{index}]` 必须是对象")))?;
            let pitch = object
                .get("pitch")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid(&format!("`seed.notes[{index}].pitch` 必须是非负整数")))?;
            if pitch > 127 {
                return Err(invalid(&format!(
                    "`seed.notes[{index}].pitch` 必须在 0..=127, 实际 {pitch}"
                )));
            }
            let duration = object
                .get("durationTicks")
                .and_then(Value::as_u64)
                .unwrap_or(PPQ);
            if duration == 0 {
                return Err(invalid(&format!(
                    "`seed.notes[{index}].durationTicks` 必须 >= 1"
                )));
            }
            parsed.push((u8::try_from(pitch).unwrap_or(u8::MAX), duration));
        }
        config.notes = parsed;
    }
    Ok(config)
}

/// 模型层 BPM 下界（与 `yeban_model::project::MIN_BPM` 同值）。
const MIN_BPM: f64 = 20.0;
/// 模型层 BPM 上界（与 `yeban_model::project::MAX_BPM` 同值）。
const MAX_BPM: f64 = 999.0;
/// 一拍（4/4 的一拍）= 一个四分音符 = `PPQ` tick。
const PPQ: u64 = yeban_model::PPQ;

/// 从零构造一份"可渲染、可配器"的工程。
///
/// # Errors
///
/// - 文档自校验失败（模型层 `validate()` 拒绝）⇒ 对应契约错误码；
/// - 内容批次的前置条件失败 ⇒ 模型层判决（**绝不**写一份半成品）。
pub fn build(config: &CreateConfig, path: &Path) -> Result<CreatedProject, Fault> {
    // 1. 模板文档（作者定型的那一层）：主总线音轨 + 它在路由图里的节点。
    //
    //    ⚠ 这一步**刻意不经 `Op`** —— 模型 `Op` 全集没有写 `master_bus_track_id`
    //    的变体（模块头逐条列了证据）。身份由 (title|path) 确定性派生，
    //    因此同一个请求两次建出的文档逐字节相同。
    let label = if config.title.is_empty() {
        path.display().to_string()
    } else {
        config.title.clone()
    };
    let master_id = deterministic_id(&format!("project:{label}:master"));
    let mut project = YebanProjectV1::default();
    project.title = if config.title.is_empty() {
        project.title.clone()
    } else {
        config.title.clone()
    };
    if let Some(bpm) = config.bpm {
        project.bpm = bpm;
    }
    project.master_bus_track_id = master_id;
    project.tracks.insert(
        master_id,
        TrackV3 {
            id: master_id,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.routing_graph = RoutingGraph {
        nodes: vec![master_id],
        edges: BTreeMap::new(),
    };

    // 2. 内容（**真的走 `Op`**，因此模型层的前置条件与自校验都真的被跑过）。
    let mut ops: Vec<Op> = Vec::new();
    let bar_ticks = PPQ.saturating_mul(4);
    let mut track_ids: Vec<EntityId> = Vec::new();
    for index in 0..config.track_count {
        let track_id = deterministic_id(&format!("project:{label}:track:{index}"));
        ops.push(Op::AddTrack {
            track: TrackV3 {
                id: track_id,
                name: format!("Track {}", index + 1),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            },
        });
        ops.push(Op::AddRoutingNode { node: track_id });
        ops.push(Op::ConnectRouting {
            edge: RoutingEdge {
                id: deterministic_id(&format!("project:{label}:edge:{index}")),
                source_node: track_id,
                destination_node: master_id,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        });
        track_ids.push(track_id);
    }
    // 2a. 第一条轨道上有真实材料：一条 MIDI 片段 + 它的一小节摆放。
    //     `propose_section` 要求 `clip_pool` 里至少有一条带音符的 MIDI 条目
    //     （`section_build.rs:812`），因此这不是"装饰"，是配器骨架的前置条件。
    if let Some(first) = track_ids.first().copied() {
        let clip_id = deterministic_id(&format!("project:{label}:clip:0"));
        let notes: BTreeMap<EntityId, MidiNote> = config
            .notes
            .iter()
            .enumerate()
            .map(|(index, (pitch, duration))| {
                let id = deterministic_id(&format!("project:{label}:note:{index}"));
                let start = (index as u64).saturating_mul(PPQ);
                (id, MidiNote::new(id, start, *pitch, *duration))
            })
            .collect();
        ops.push(Op::AddClip {
            clip: ClipPoolEntry {
                id: clip_id,
                name: config.clip_name.clone(),
                content: ClipContent::Midi { notes },
            },
        });
        ops.push(Op::AddClipPlacement {
            track_id: first,
            placement: ClipPlacement {
                id: deterministic_id(&format!("project:{label}:placement:0")),
                clip_id,
                start_tick: 0,
                duration_ticks: bar_ticks,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        });
    }

    Op::Batch {
        ops: ops.clone(),
        description: format!("create project {label}"),
    }
    .apply(&mut project)
    .map_err(|error| super::error::from_model("新建工程的内容批次", &error))?;
    project
        .validate()
        .map_err(|error| super::error::from_model("新建工程的文档校验", &error))?;
    Ok(CreatedProject {
        project,
        master_bus_track_id: master_id,
        ops,
    })
}

/// 建工程响应的 `data.seed` 载荷（**从文档里数出来的**读数，不是回抄参数）。
#[must_use]
pub fn seed_summary(created: &CreatedProject) -> Value {
    let notes = created
        .project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(BTreeMap::len)
        .sum::<usize>();
    let placements = created
        .project
        .tracks
        .values()
        .map(|track| track.clips.len())
        .sum::<usize>();
    serde_json::json!({
        "template": "master-bus + one midi track",
        "masterBusTrackId": created.master_bus_track_id.to_canonical_string(),
        "masterBusInRoutingGraph": created
            .project
            .routing_graph
            .nodes
            .contains(&created.master_bus_track_id),
        "trackCount": created.project.tracks.len(),
        "routingNodeCount": created.project.routing_graph.nodes.len(),
        "routingEdgeCount": created.project.routing_graph.edges.len(),
        "clipPoolCount": created.project.clip_pool.len(),
        "noteCount": notes,
        "placementCount": placements,
        "opKinds": created.ops.iter().map(Op::name).collect::<Vec<_>>(),
        "renderReady": notes > 0,
        "specId": SPEC_ID,
    })
}

/// `INVALID_PARAMETER_RANGE` 的简写（`ADR-0001` D25 的 20 值联集里的既有码）。
fn invalid(message: &str) -> Fault {
    Fault::domain(ErrorCode::InvalidParameterRange, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/yeban-create-unit/demo.yeban")
    }

    #[test]
    fn default_config_builds_a_renderable_project_with_a_master_bus() {
        let created = build(&CreateConfig::default(), &path()).expect("建工程");
        let project = &created.project;
        assert!(
            !project.master_bus_track_id.is_nil(),
            "主总线身份必须非 nil"
        );
        assert_eq!(project.master_bus_track_id, created.master_bus_track_id);
        assert!(
            project
                .routing_graph
                .nodes
                .contains(&created.master_bus_track_id),
            "主总线必须在路由图节点表里"
        );
        let master = project
            .tracks
            .get(&created.master_bus_track_id)
            .expect("主总线音轨在场");
        assert_eq!(master.kind, TrackKind::Master);
        assert!(!project.routing_graph.edges.is_empty(), "至少一条声部连接");
        assert!(
            project
                .clip_pool
                .values()
                .any(|entry| { entry.content.notes().is_some_and(|notes| !notes.is_empty()) })
        );
        project.validate().expect("新建的工程必须自校验通过");
    }

    #[test]
    fn building_twice_is_byte_for_byte_identical() {
        let first = build(&CreateConfig::default(), &path()).expect("第一次");
        let second = build(&CreateConfig::default(), &path()).expect("第二次");
        assert_eq!(first.project, second.project, "同一请求 ⇒ 同一文档");
        assert_eq!(
            serde_json::to_string(&first.project).expect("序列化"),
            serde_json::to_string(&second.project).expect("序列化"),
            "规范化 JSON 必须逐字节相同"
        );
    }

    #[test]
    fn seed_notes_really_change_the_output() {
        let config = CreateConfig {
            notes: vec![(72, 480)],
            ..CreateConfig::default()
        };
        let created = build(&config, &path()).expect("建工程");
        let notes: Vec<u8> = created
            .project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values().map(|note| note.pitch))
            .collect();
        assert_eq!(notes, vec![72]);
    }

    #[test]
    fn track_count_really_adds_tracks_and_edges() {
        let config = CreateConfig {
            track_count: 3,
            ..CreateConfig::default()
        };
        let created = build(&config, &path()).expect("建工程");
        // 3 条内容轨 + 模板的主总线。
        assert_eq!(created.project.tracks.len(), 4);
        assert_eq!(created.project.routing_graph.edges.len(), 3);
        created.project.validate().expect("自校验");
    }

    #[test]
    fn title_and_bpm_land_in_the_document() {
        let config = CreateConfig {
            title: "Demo".to_owned(),
            bpm: Some(96.0),
            ..CreateConfig::default()
        };
        let created = build(&config, &path()).expect("建工程");
        assert_eq!(created.project.title, "Demo");
        assert!((created.project.bpm - 96.0).abs() < f64::EPSILON);
        // 不同标题 ⇒ 不同身份（身份是 (标题, 角色) 的纯函数）。
        let other = build(
            &CreateConfig {
                title: "Other".to_owned(),
                ..CreateConfig::default()
            },
            &path(),
        )
        .expect("建工程");
        assert_ne!(
            created.master_bus_track_id, other.master_bus_track_id,
            "标题变了 ⇒ 身份必须跟着变"
        );
    }

    #[test]
    fn out_of_range_parameters_are_rejected_with_the_existing_code() {
        let too_fast = parse_config(None, Some(&serde_json::json!(5000.0)), None, None, None)
            .expect_err("越界 BPM 必须被拒");
        assert_eq!(
            too_fast.domain_code(),
            Some(ErrorCode::InvalidParameterRange)
        );
        let zero_tracks = parse_config(None, None, Some(&serde_json::json!(0)), None, None)
            .expect_err("0 轨必须被拒");
        assert_eq!(
            zero_tracks.domain_code(),
            Some(ErrorCode::InvalidParameterRange)
        );
        let bad_note = parse_config(
            None,
            None,
            None,
            None,
            Some(&serde_json::json!([{"pitch": 200}])),
        )
        .expect_err("越界音高必须被拒");
        assert_eq!(
            bad_note.domain_code(),
            Some(ErrorCode::InvalidParameterRange)
        );
        let zero_duration = parse_config(
            None,
            None,
            None,
            None,
            Some(&serde_json::json!([{"pitch": 60, "durationTicks": 0}])),
        )
        .expect_err("0 时值必须被拒");
        assert_eq!(
            zero_duration.domain_code(),
            Some(ErrorCode::InvalidParameterRange)
        );
    }

    #[test]
    fn seed_summary_reports_the_document_not_the_request() {
        let created = build(&CreateConfig::default(), &path()).expect("建工程");
        let summary = seed_summary(&created);
        assert_eq!(summary["masterBusInRoutingGraph"], Value::Bool(true));
        assert_eq!(
            summary["masterBusTrackId"],
            Value::String(created.master_bus_track_id.to_canonical_string())
        );
        assert_eq!(summary["trackCount"], serde_json::json!(2));
        assert_eq!(summary["noteCount"], serde_json::json!(4));
        assert_eq!(summary["routingEdgeCount"], serde_json::json!(1));
    }
}
