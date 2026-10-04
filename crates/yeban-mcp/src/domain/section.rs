//! `yeban_propose_section` 的章节配器骨架 [MCP-TOOL-005]。
//!
//! ## 规范缺口（**这一半明确没做**，先说清楚）
//!
//! §7.2 的原文是"在隔离分支 `ai/proposal-{ulid}` 创建章节配器骨架与**声部连接**"。
//! 实测 `yeban-model` 的 `Op` 全集后可以确认两件事：
//!
//! 1. **没有 `AddClip` / `RemoveClip` 变体**：片段池条目无法由 `Op` 表达，
//!    因此"配器骨架"里**没有片段**（只有段落 + 音轨）；
//! 2. **没有 `AddRoutingNode` / `RemoveRoutingNode` 变体**：而
//!    [`Op::ConnectRouting`] 的前置条件要求两端**已经在** `routing_graph.nodes` 里，
//!    新音轨的身份不可能预先在节点表里 —— 因此**声部连接无法由 `Op` 全集表达**。
//!
//! 本模块因此只产出"段落 + 音轨"骨架，并把上述两个缺失变体登记为
//! **待裁决的 `Op` 全集缺口**（`docs/ledger/tools-domain-notes.md` 的 needs 清单）。
//! 这不是"忘了"，而是"`Op` 层表达不出来"。
//!
//! ## `CYCLE_DETECTED` 在哪里
//!
//! 模型层**不做**环路判定（`RoutingGraph::validate` 只查节点存在性与重复）。
//! 本模块补上它：[`detect_cycle`] 是真正的 DFS 着色判环，`plan` 在创建骨架**之前**
//! 先检查现有路由图 —— 在一个已经成环的工程上再叠加配器只会掩盖问题。

use serde_json::Value;

use yeban_model::{EntityId, Op, PPQ, RoutingGraph, SectionV3, TrackKind, TrackV3, YebanProjectV1};

use super::error::{Fault, from_model};
use super::ids::deterministic_id;
use crate::tools::ErrorCode;

/// 小节数上限（超过即 `OUT_OF_RANGE`）。
pub const MAX_BARS: u64 = 64;

/// `4/4` 一小节的 tick 数（`960 PPQ`）—— 级联展开的时间跨度基准。
pub const TICKS_PER_BAR_4_4: u64 = PPQ * 4;

/// 每种风格预设的声部数上限（用于生成确定性音轨名）。
pub const MAX_PARTS: usize = 8;

/// 风格预设表：`(预设名, [声部名])`。
///
/// **本地决策**（规范 §7.2 只给了 `stylePreset: String` 与 `STYLE_NOT_FOUND`，
/// 没有给预设清单）。表的**机制**是承重的（未知预设必须 `STYLE_NOT_FOUND`），
/// 表的内容是可替换的：换成 `yeban-theory` / `yeban-services` 的预设库时，
/// 改这一处即可，`plan` 的其余部分不动。
pub const STYLE_PRESETS: [(&str, &[&str]); 4] = [
    ("cinematic-orchestral", &["Strings", "Brass", "Percussion"]),
    ("lofi-beats", &["Keys", "Bass", "Drums"]),
    ("synthwave", &["Pad", "Lead", "Bass", "Arp"]),
    ("acoustic-folk", &["Guitar", "Bass", "Percussion"]),
];

/// 音名表（含等音写法）。
pub const NOTE_NAMES: [&str; 17] = [
    "C", "C#", "DB", "D", "D#", "EB", "E", "F", "F#", "GB", "G", "G#", "AB", "A", "A#", "BB", "B",
];

/// 调式表。
pub const MODES: [&str; 12] = [
    "major",
    "minor",
    "dorian",
    "phrygian",
    "lydian",
    "mixolydian",
    "locrian",
    "harmonic minor",
    "melodic minor",
    "pentatonic major",
    "pentatonic minor",
    "blues",
];

/// 一个已校验的章节骨架规划（**只读计算的产物**）。
#[derive(Clone, Debug, PartialEq)]
pub struct SectionPlan {
    /// 段落身份（确定性）。
    pub section_id: EntityId,
    /// 段落起始 tick。
    pub start_tick: u64,
    /// 段落结束 tick。
    pub end_tick: u64,
    /// 每小节 tick 数。
    pub ticks_per_bar: u64,
    /// 生成的声部音轨身份。
    pub part_track_ids: Vec<EntityId>,
    /// 将要施加的领域操作（顺序即施加顺序）。
    pub ops: Vec<Op>,
}

impl SectionPlan {
    /// `dryRun` 预览用的 JSON。
    #[must_use]
    pub fn preview(&self) -> Value {
        let mut part_track_ids: Vec<Value> = self
            .part_track_ids
            .iter()
            .map(|id| Value::from(id.to_canonical_string()))
            .collect();
        part_track_ids.shrink_to_fit();
        serde_json::json!({
            "sectionId": self.section_id.to_canonical_string(),
            "startTick": self.start_tick,
            "endTick": self.end_tick,
            "ticksPerBar": self.ticks_per_bar,
            "partTrackIds": part_track_ids,
            "ops": ops_to_value(&self.ops),
            "unwired": ["clipPoolEntries", "routingEdges"],
            "unwiredReason": "ARCH-OPS-001 的 Op 全集没有 AddClip / AddRoutingNode, \
                              片段池与路由节点无法由 Op 表达 (见本模块头)",
        })
    }
}

/// 规划一个章节骨架。
///
/// # Errors
///
/// - 未知风格预设 → `STYLE_NOT_FOUND`；
/// - `bars` 为 0 或超过 [`MAX_BARS`]，未知调式 → `OUT_OF_RANGE` / `INVALID_PARAMETER_RANGE`；
/// - 现有路由图已经成环 → `CYCLE_DETECTED`；
/// - 施加到克隆体后模型层校验失败 → [`super::error::code_for_model`] 给出的契约码。
pub fn plan(
    project: &YebanProjectV1,
    section_name: &str,
    style_preset: &str,
    bars: u64,
    scale: Option<&str>,
) -> Result<SectionPlan, Fault> {
    let parts = preset_parts(style_preset)?;
    if parts.is_empty() || parts.len() > MAX_PARTS {
        return Err(Fault::domain(
            ErrorCode::Conflict,
            format!(
                "风格预设 `{style_preset}` 的声部数 {} 不在 1..={MAX_PARTS}",
                parts.len()
            ),
        ));
    }
    if bars == 0 || bars > MAX_BARS {
        return Err(Fault::domain_with_data(
            ErrorCode::OutOfRange,
            format!("`bars` 必须在 1..={MAX_BARS}, 实际 {bars}"),
            serde_json::json!({ "bars": bars, "maxBars": MAX_BARS }),
        ));
    }
    if let Some(scale) = scale {
        parse_scale(scale)?;
    }
    if let Some(cycle) = detect_cycle(&project.routing_graph) {
        return Err(Fault::domain_with_data(
            ErrorCode::CycleDetected,
            "工程现有的声部连接存在环路, 拒绝在其上叠加配器骨架",
            serde_json::json!({
                "cycle": cycle.iter().map(EntityId::to_canonical_string).collect::<Vec<_>>(),
            }),
        ));
    }

    let ticks_per_bar = ticks_per_bar(project);
    let start_tick = project
        .sections
        .values()
        .map(|section| section.end_tick)
        .max()
        .unwrap_or(0);
    let end_tick = start_tick.saturating_add(ticks_per_bar.saturating_mul(bars));

    let section_id = deterministic_id(&format!(
        "section:{section_name}:{style_preset}:{bars}:{start_tick}"
    ));
    let section = SectionV3 {
        id: section_id,
        name: section_name.to_owned(),
        start_tick,
        end_tick,
        color: None,
    };

    let mut ops = Vec::with_capacity(parts.len() + 1);
    ops.push(Op::SetSection {
        section_id,
        old_section: project.sections.get(&section_id).cloned(),
        new_section: section,
    });
    let mut part_track_ids = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let track_id = deterministic_id(&format!("part:{section_id}:{index}:{part}"));
        part_track_ids.push(track_id);
        ops.push(Op::AddTrack {
            track: TrackV3 {
                id: track_id,
                name: format!("{section_name} · {part}"),
                kind: TrackKind::Midi,
                color: palette(index),
                ..TrackV3::default()
            },
        });
    }

    // 在克隆体上整体模拟, 再问模型层"这份结果合法吗" —— 绝不把半成品交给调用方。
    let mut simulated = project.clone();
    Op::Batch {
        ops: ops.clone(),
        description: format!("propose_section {section_name}"),
    }
    .apply(&mut simulated)
    .map_err(|error| from_model("章节骨架模拟", &error))?;
    simulated
        .validate()
        .map_err(|error| from_model("章节骨架校验", &error))?;

    Ok(SectionPlan {
        section_id,
        start_tick,
        end_tick,
        ticks_per_bar,
        part_track_ids,
        ops,
    })
}

/// 查风格预设。
fn preset_parts(style_preset: &str) -> Result<&'static [&'static str], Fault> {
    STYLE_PRESETS
        .iter()
        .find(|(name, _)| *name == style_preset)
        .map(|(_, parts)| *parts)
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::StyleNotFound,
                format!("未知风格预设 `{style_preset}`"),
                serde_json::json!({
                    "availablePresets": STYLE_PRESETS.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
                }),
            )
        })
}

/// 校验调式写法（`"<音名> <调式>"`，大小写不敏感）。
fn parse_scale(scale: &str) -> Result<(String, String), Fault> {
    let trimmed = scale.trim();
    let (note, mode) = trimmed.split_once(' ').ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`scale` 必须形如 \"C minor\", 实际 `{scale}`"),
            serde_json::json!({ "noteNames": NOTE_NAMES, "modes": MODES }),
        )
    })?;
    let note = note.trim().to_ascii_uppercase();
    let mode = mode.trim().to_ascii_lowercase();
    if !NOTE_NAMES.contains(&note.as_str()) {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知音名 `{note}`"),
            serde_json::json!({ "noteNames": NOTE_NAMES }),
        ));
    }
    if !MODES.contains(&mode.as_str()) {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("未知调式 `{mode}`"),
            serde_json::json!({ "modes": MODES }),
        ));
    }
    Ok((note, mode))
}

/// 每小节 tick 数（由工程拍号算出，`960 PPQ`）。
#[must_use]
pub fn ticks_per_bar(project: &YebanProjectV1) -> u64 {
    let signature = project.time_signature;
    let numerator = u64::from(signature.numerator);
    let denominator = u64::from(signature.denominator);
    let ticks = PPQ
        .saturating_mul(numerator)
        .saturating_mul(4)
        .checked_div(denominator)
        .unwrap_or(PPQ);
    ticks.max(1)
}

/// 调色板（确定性；只影响界面表现，不承载音频语义）。
fn palette(index: usize) -> Option<String> {
    const COLORS: [&str; 6] = [
        "#FF8800", "#33AAFF", "#66DD88", "#DD66CC", "#FFD166", "#8C8CFF",
    ];
    COLORS
        .get(index % COLORS.len())
        .map(|color| (*color).to_owned())
}

/// DFS 着色判环；返回环上的节点序列（含闭合节点），无环返回 `None`。
///
/// 迭代顺序 = `graph.nodes` 的顺序，邻接表按 `edges` 的 `BTreeMap` 键序展开，
/// 因此判定结果与报告内容都是**确定性**的。
#[must_use]
pub fn detect_cycle(graph: &RoutingGraph) -> Option<Vec<EntityId>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut colors: std::collections::BTreeMap<EntityId, Color> = graph
        .nodes
        .iter()
        .map(|node| (*node, Color::White))
        .collect();
    let mut stack: Vec<EntityId> = Vec::new();

    for root in &graph.nodes {
        if colors.get(root).copied() != Some(Color::White) {
            continue;
        }
        // 显式栈 DFS：`(节点, 下一步要看的邻接下标)`。
        let mut path: Vec<(EntityId, usize)> = vec![(*root, 0)];
        colors.insert(*root, Color::Gray);
        stack.push(*root);
        while let Some(&(node, cursor)) = path.last() {
            let outgoing: Vec<EntityId> = graph
                .edges
                .values()
                .filter(|edge| edge.source_node == node)
                .map(|edge| edge.destination_node)
                .collect();
            if cursor >= outgoing.len() {
                colors.insert(node, Color::Black);
                stack.pop();
                path.pop();
                continue;
            }
            let next = outgoing[cursor];
            if let Some(last) = path.last_mut() {
                last.1 += 1;
            }
            match colors.get(&next).copied().unwrap_or(Color::White) {
                Color::Gray => {
                    // 找到回边：从栈里截出环。
                    let start = stack.iter().position(|entry| *entry == next).unwrap_or(0);
                    let mut cycle: Vec<EntityId> = stack
                        .get(start..)
                        .map_or_else(Vec::new, <[EntityId]>::to_vec);
                    cycle.push(next);
                    return Some(cycle);
                }
                Color::Black => {}
                Color::White => {
                    colors.insert(next, Color::Gray);
                    stack.push(next);
                    path.push((next, 0));
                }
            }
        }
    }
    None
}

/// 一组 `Op` 的 JSON 列表（`dryRun` 预览与提交记录共用同一份形状）。
#[must_use]
pub fn ops_to_value(ops: &[Op]) -> Value {
    Value::Array(
        ops.iter()
            .map(|op| serde_json::to_value(op).unwrap_or(Value::Null))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use yeban_model::samples::filled_project;
    use yeban_model::{RoutingEdge, RoutingKind};

    /// 一条确定性路由边（测试夹具）。
    fn routing_edge(source: EntityId, destination: EntityId, label: &str) -> RoutingEdge {
        RoutingEdge {
            id: deterministic_id(&format!("edge:{source}:{destination}:{label}")),
            source_node: source,
            destination_node: destination,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        }
    }

    #[test]
    fn unknown_preset_is_style_not_found() {
        let project = filled_project();
        let fault = plan(&project, "Chorus", "polka", 8, None).expect_err("未知预设");
        assert_eq!(fault.domain_code(), Some(ErrorCode::StyleNotFound));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "STYLE_NOT_FOUND");
        assert!(value["error"]["data"]["availablePresets"].is_array());
    }

    #[test]
    fn bars_and_scale_are_validated() {
        let project = filled_project();
        let fault = plan(&project, "Chorus", "lofi-beats", 0, None).expect_err("bars=0");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        let fault =
            plan(&project, "Chorus", "lofi-beats", MAX_BARS + 1, None).expect_err("bars 过大");
        assert_eq!(fault.domain_code(), Some(ErrorCode::OutOfRange));
        let fault = plan(&project, "Chorus", "lofi-beats", 4, Some("H dorian")).expect_err("音名");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let fault = plan(&project, "Chorus", "lofi-beats", 4, Some("C bogus")).expect_err("调式");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        plan(&project, "Chorus", "lofi-beats", 4, Some("c MINOR")).expect("大小写不敏感");
    }

    #[test]
    fn section_plan_is_deterministic_and_applies_cleanly() {
        let project = filled_project();
        let first = plan(&project, "Chorus", "synthwave", 8, Some("C minor")).expect("规划");
        let second = plan(&project, "Chorus", "synthwave", 8, Some("C minor")).expect("规划");
        assert_eq!(first, second, "同一请求必须产出同一份规划");
        assert_eq!(first.part_track_ids.len(), 4);
        assert_eq!(first.ops.len(), 5, "1 段落 + 4 声部音轨");
        assert_eq!(first.ticks_per_bar, 3840, "样本是 4/4, 960 PPQ");

        // 起点接在最后一个段落之后, 且不重叠。
        let last_end = project
            .sections
            .values()
            .map(|section| section.end_tick)
            .max()
            .unwrap_or(0);
        assert_eq!(first.start_tick, last_end);
        assert_eq!(first.end_tick, last_end + 3840 * 8);

        let mut simulated = project.clone();
        Op::Batch {
            ops: first.ops.clone(),
            description: "test".to_owned(),
        }
        .apply(&mut simulated)
        .expect("必须能整体施加");
        simulated.validate().expect("施加后必须合法");
        assert_eq!(simulated.sections.len(), project.sections.len() + 1);
        assert_eq!(simulated.tracks.len(), project.tracks.len() + 4);
    }

    #[test]
    fn cycle_detection_finds_and_reports_cycles() {
        let a = deterministic_id("a");
        let b = deterministic_id("b");
        let c = deterministic_id("c");
        let mut graph = RoutingGraph {
            nodes: vec![a, b, c],
            edges: BTreeMap::new(),
        };
        assert!(detect_cycle(&graph).is_none());
        // a -> b -> c
        for (source, destination) in [(a, b), (b, c)] {
            let edge = routing_edge(source, destination, "x");
            graph.edges.insert(edge.id, edge);
        }
        assert!(detect_cycle(&graph).is_none(), "无环");
        // c -> a 闭合。
        let edge = routing_edge(c, a, "close");
        graph.edges.insert(edge.id, edge);
        let cycle = detect_cycle(&graph).expect("必须找到环");
        assert_eq!(cycle.first(), cycle.last(), "环必须闭合");
        assert!(cycle.len() >= 4, "a->b->c->a: {cycle:?}");
    }

    #[test]
    fn a_cyclic_project_is_refused_before_arranging() {
        let mut project = filled_project();
        let a = deterministic_id("cyc-a");
        let b = deterministic_id("cyc-b");
        project.routing_graph.nodes.push(a);
        project.routing_graph.nodes.push(b);
        for (source, destination) in [(a, b), (b, a)] {
            let edge = routing_edge(source, destination, "cyc");
            project.routing_graph.edges.insert(edge.id, edge);
        }
        project.validate().expect("模型层不判环, 因此它是'合法'的");
        let fault = plan(&project, "Chorus", "lofi-beats", 4, None).expect_err("必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::CycleDetected));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "CYCLE_DETECTED");
        assert!(value["error"]["data"]["cycle"].is_array());
    }
}
