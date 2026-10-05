//! `yeban_propose_section` 的**章节配器骨架 + 声部连接**生成器 [MCP-TOOL-005]。
//!
//! ## 这一层到底是什么
//!
//! 规范 §7.2 对 `yeban_propose_section` 的要求是"在隔离分支 `ai/proposal-{ulid}`
//! 创建**章节配器骨架**与**声部连接**"。在 `ADR-0001` **D27**（= `HD-12`，2026-10-04 追认）
//! 把 `Op` 全集从 23 扩到 27 之后，这两件事在操作日志层**已经可以表达**：
//!
//! | §7.2 的能力 | 用哪个真实存在的 `Op` 表达 |
//! | :--- | :--- |
//! | 配器骨架里的**片段** | [`Op::AddClip`]（把片段池条目放进 `clip_pool`）+ [`Op::AddClipPlacement`]（摆到声部音轨上） |
//! | 段落 | [`Op::SetSection`]（`old_section: None` 即新建） |
//! | 声部音轨 | [`Op::AddTrack`] |
//! | **声部连接** | [`Op::AddRoutingNode`]（把声部放进 `routing_graph.nodes`）+ [`Op::ConnectRouting`]（声部 → 主总线） |
//!
//! 因此本模块**不再**上报 `unwired = ["clipPoolEntries","routingEdges"]` ——
//! 那个说法建立在"`Op` 全集缺 4 个变体"的**过期事实**上（见 `ADR-0001` D43：
//! 1.0.0 之前没有兼容包袱，过期假设该删就删）。仍然真实存在的缺口只有一处，
//! 且它是**输入侧**而不是操作侧的：`clip_pool` 里必须有**可用材料**
//! （MIDI 且至少一个音符）才能配器 —— 没有任何 MCP 工具能凭空造出片段池条目，
//! 所以缺材料时**如实报 `CLIP_NOT_FOUND` 并说明缺什么**，绝不假装成功、
//! 也绝不退回一个含糊的 `unwired`（见 §[`BuildCode`] 与 `docs/ledger/propose-section-notes.md`）。
//!
//! ## 为什么单独一个模块（零重依赖 ⇒ 本机可真跑）
//!
//! `yeban-mcp` 传递性地依赖 `yeban-render` / `yeban-decode`（rayon/hound/midly/
//! symphonia/rubato），本机根本编不动（`AGENTS.md` §5 的本机纪律）。而"骨架长什么样、
//! 声部连到哪、逆操作能不能逐字节回退"这几件事**只**需要 `yeban-model` + `serde_json`。
//! 把它们抽到这里，就能用裸 `rustc --edition 2024 --test`（判据脚手架
//! `crates/yeban-mcp/verify/section_pure.rs`）在本机**真的执行**
//! —— 与 `ADR-0001` **D28** 第 1 条（纯函数投影层 + 零重依赖模块）同一手法。
//!
//! 本模块**没有** `use crate::…`：它不知道 `Fault` / `ErrorCode` / `Domain` 的存在，
//! 只吐 [`BuildFault`]，由 [`super::section`] 做**唯一一次**错误码映射。
//!
//! ## 材料从哪来（本线裁决，登记为 needs-6 的同类本地决策）
//!
//! 每个声部拿一条**新**片段池条目（[`Op::AddClip`] 的产物），其**内容**取自工程里
//! 已有的可用材料：
//!
//! - 材料 = `clip_pool` 里 [`ClipContent::Midi`] 且**至少一个音符**的条目（按 `BTreeMap`
//!   键序，即确定序）；空 MIDI 片段**不是**材料（它不携带任何音乐内容）；
//! - 第 `i` 个声部取 `materials[i % materials.len()]`（轮转；声部数可以多于材料数）；
//! - 若调用给了 `scale`，材料按**等音类移调**到该调的主音
//!   （`delta = (tonic_pc - 材料最低音的音级) mod 12`，超出 `0..=127` 时做**八度折叠**
//!   ⇒ 音级不丢、音域始终合法）；不给 `scale` 则原样拷贝（`delta = 0`）；
//! - 每个音符在**新片段里**拿到一个新身份（`note:{clip_id}:{原身份}`），因此同一份材料
//!   被多个声部使用时不会出现"两个片段共享同一个音符身份"。
//!
//! 这不是"凭空造音乐"：音符、时值、力度、表现力字段都来自工程里**真实存在**的材料，
//! 本层只做音级平移与身份派生。真正的"写旋律"在 `line/theory-core` 一侧
//! （它的台账 §pending-5 明确写了"没有把 `yeban_propose_section` 接起来"）。

use std::collections::BTreeMap;

use serde_json::{Value, json};

use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, ModelError, Op, PPQ,
    RoutingEdge, RoutingGraph, RoutingKind, SectionV3, TrackKind, TrackV3, YebanProjectV1,
};

use super::ids::deterministic_id;

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

/// 与 [`NOTE_NAMES`] **逐位对应**的音级（0 = C）。
///
/// 两张表长度必须相等 —— 判据 `note_names_and_pitch_classes_are_aligned` 钉住这件事。
pub const NOTE_PITCH_CLASSES: [u8; 17] = [0, 1, 1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 8, 9, 10, 10, 11];

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

/// 声部音轨的界面色标（确定性；只影响界面表现，不承载音频语义）。
const COLORS: [&str; 6] = [
    "#FF8800", "#33AAFF", "#66DD88", "#DD66CC", "#FFD166", "#8C8CFF",
];

/// 本模块能产出的**领域失败类别**（→ [`super::section`] 映射成契约错误码）。
///
/// 刻意**不含**任何新码：`ADR-0001` D25 的 20 值联集是工具级错误码的唯一来源。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildCode {
    /// 未知风格预设 → `STYLE_NOT_FOUND`。
    StyleNotFound,
    /// 工程现有路由图成环 → `CYCLE_DETECTED`。
    CycleDetected,
    /// `bars` 越界 → `OUT_OF_RANGE`。
    OutOfRange,
    /// `scale` 写法非法 → `INVALID_PARAMETER_RANGE`。
    InvalidParameterRange,
    /// 预设表自身自相矛盾（声部数为 0 或超过 [`MAX_PARTS`]）→ `CONFLICT`。
    Conflict,
    /// `clip_pool` 里没有可用材料 → `CLIP_NOT_FOUND`。
    ClipNotFound,
    /// 工程没有主总线音轨（声部连接没有目标）→ `TRACK_NOT_FOUND`。
    TrackNotFound,
}

impl BuildCode {
    /// 全部类别（`super::section` 的"每个类别都落在契约 enum 里"判据遍历它）。
    pub const ALL: [Self; 7] = [
        Self::StyleNotFound,
        Self::CycleDetected,
        Self::OutOfRange,
        Self::InvalidParameterRange,
        Self::Conflict,
        Self::ClipNotFound,
        Self::TrackNotFound,
    ];
}

/// 领域失败（**只**描述领域侧事实，不含契约错误码）。
#[derive(Debug)]
pub enum BuildFault {
    /// 本模块自己判定的领域失败。
    Domain {
        /// 失败类别。
        code: BuildCode,
        /// 人话信息。
        message: String,
        /// 结构化补充（直接进 `ToolResponse.error.data`）。
        data: Value,
    },
    /// 施加到克隆体上时**模型层**给出的失败（交给 `error::from_model` 映射）。
    Model {
        /// 上下文（出错在哪一步）。
        context: &'static str,
        /// 模型层错误。
        error: ModelError,
    },
}

impl BuildFault {
    /// 领域失败类别（模型层失败返回 `None`）。
    #[must_use]
    pub const fn domain_code(&self) -> Option<BuildCode> {
        match self {
            Self::Domain { code, .. } => Some(*code),
            Self::Model { .. } => None,
        }
    }

    /// 构造一条领域失败。
    fn domain(code: BuildCode, message: impl Into<String>, data: Value) -> Self {
        Self::Domain {
            code,
            message: message.into(),
            data,
        }
    }
}

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
    /// 生成的声部音轨身份（顺序 = 风格预设的声部顺序）。
    pub part_track_ids: Vec<EntityId>,
    /// 生成的**片段池条目**身份（[`Op::AddClip`] 的产物，与声部一一对应）。
    pub part_clip_ids: Vec<EntityId>,
    /// 生成的**摆放**身份（[`Op::AddClipPlacement`] 的产物，与声部一一对应）。
    pub placement_ids: Vec<EntityId>,
    /// 生成的**路由边**身份（声部 → 主总线，与声部一一对应）。
    pub routing_edge_ids: Vec<EntityId>,
    /// 本批需要新增的路由节点（声部 + 必要时的主总线）。
    pub added_routing_nodes: Vec<EntityId>,
    /// 将要施加的领域操作（顺序即施加顺序）。
    pub ops: Vec<Op>,
    /// **由 [`SectionPlan::ops`] 推导**的未接线项（不是硬编码的声明）。
    pub unwired: Vec<&'static str>,
}

/// 一个已校验的调性（`"<音名> <调式>"`，大小写不敏感）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scale {
    /// 主音音级（0 = C）。
    pub tonic_pc: u8,
    /// 归一化后的音名。
    pub note: String,
    /// 归一化后的调式（小写）。
    pub mode: String,
}

impl Scale {
    /// 归一化写法（`"C minor"`）—— 进 `section_id` 的确定性种子与响应。
    #[must_use]
    pub fn canonical(&self) -> String {
        format!("{} {}", self.note, self.mode)
    }
}

/// 查风格预设。
///
/// # Errors
///
/// 未知预设 → [`BuildCode::StyleNotFound`]（`data.availablePresets` 给出全部候选）。
pub fn preset_parts(style_preset: &str) -> Result<&'static [&'static str], BuildFault> {
    STYLE_PRESETS
        .iter()
        .find(|(name, _)| *name == style_preset)
        .map(|(_, parts)| *parts)
        .ok_or_else(|| {
            BuildFault::domain(
                BuildCode::StyleNotFound,
                format!("未知风格预设 `{style_preset}`"),
                json!({
                    "availablePresets": STYLE_PRESETS.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
                }),
            )
        })
}

/// 校验调式写法（`"<音名> <调式>"`，大小写不敏感）。
///
/// # Errors
///
/// 缺调式 / 未知音名 / 未知调式 → [`BuildCode::InvalidParameterRange`]。
pub fn parse_scale(scale: &str) -> Result<Scale, BuildFault> {
    let trimmed = scale.trim();
    let (note, mode) = trimmed.split_once(' ').ok_or_else(|| {
        BuildFault::domain(
            BuildCode::InvalidParameterRange,
            format!("`scale` 必须形如 \"C minor\", 实际 `{scale}`"),
            json!({ "noteNames": NOTE_NAMES, "modes": MODES }),
        )
    })?;
    let note = note.trim().to_ascii_uppercase();
    let mode = mode.trim().to_ascii_lowercase();
    let position = NOTE_NAMES
        .iter()
        .position(|name| *name == note)
        .ok_or_else(|| {
            BuildFault::domain(
                BuildCode::InvalidParameterRange,
                format!("未知音名 `{note}`"),
                json!({ "noteNames": NOTE_NAMES }),
            )
        })?;
    if !MODES.contains(&mode.as_str()) {
        return Err(BuildFault::domain(
            BuildCode::InvalidParameterRange,
            format!("未知调式 `{mode}`"),
            json!({ "modes": MODES }),
        ));
    }
    Ok(Scale {
        tonic_pc: NOTE_PITCH_CLASSES[position],
        note,
        mode,
    })
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

/// DFS 着色判环；返回环上的节点序列（含闭合节点），无环返回 `None`。
///
/// 迭代顺序 = `graph.nodes` 的顺序，邻接表按 `edges` 的 `BTreeMap` 键序展开，
/// 因此判定结果与报告内容都是**确定性**的。
///
/// 模型层**不做**环路判定（`RoutingGraph::validate` 只查节点存在性与重复），
/// 本函数补上它：在一个已经成环的工程上再叠加配器只会掩盖问题。
#[must_use]
pub fn detect_cycle(graph: &RoutingGraph) -> Option<Vec<EntityId>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut colors: BTreeMap<EntityId, Color> = graph
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

/// `yeban_propose_section` 里**未接线**的部分 —— 由**真实 op 名字**推导。
///
/// 判据：`AddClip` 缺席 ⇒ `clipPoolEntries`；`AddRoutingNode` 或 `ConnectRouting`
/// 缺席 ⇒ `routingEdges`。这样"上报的缺口"与"实际生成的 op"永远是同一件事，
/// 不会出现"代码已经做了、响应还在喊缺"（这正是本线要修掉的那种过期自我限制）。
#[must_use]
pub fn unwired_for_section_op_kinds(kinds: &[&str]) -> Vec<&'static str> {
    let mut unwired: Vec<&'static str> = Vec::new();
    if !kinds.contains(&"AddClip") {
        unwired.push("clipPoolEntries");
    }
    if !(kinds.contains(&"AddRoutingNode") && kinds.contains(&"ConnectRouting")) {
        unwired.push("routingEdges");
    }
    unwired
}

/// 一条**可用作配器材料**的片段池条目。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Material {
    /// 片段池条目身份。
    id: EntityId,
    /// 该材料最低音的音级（移调锚点）。
    root_pc: u8,
}

/// 工程里全部可用材料（按 `clip_pool` 的 `BTreeMap` 键序 = 确定序）。
fn usable_materials(project: &YebanProjectV1) -> Vec<Material> {
    project
        .clip_pool
        .values()
        .filter_map(|entry| {
            let notes = entry.content.notes()?;
            let root = notes.values().map(|note| note.pitch).min()?;
            Some(Material {
                id: entry.id,
                root_pc: root % 12,
            })
        })
        .collect()
}

/// 声部连接的目标端点：主总线音轨。
///
/// # Errors
///
/// 主总线身份为 nil 或不在 `tracks` 里 → [`BuildCode::TrackNotFound`]。
fn master_bus(project: &YebanProjectV1) -> Result<EntityId, BuildFault> {
    let bus = project.master_bus_track_id;
    if bus.is_nil() || !project.tracks.contains_key(&bus) {
        return Err(BuildFault::domain(
            BuildCode::TrackNotFound,
            "工程没有主总线音轨, 声部连接没有目标端点",
            json!({
                "missing": "masterBusTrack",
                "masterBusTrackId": bus.to_canonical_string(),
                "trackCount": project.tracks.len(),
                "requiredBy": "Op::ConnectRouting 的前置条件 (两端都必须在 routing_graph.nodes 里)",
            }),
        ));
    }
    Ok(bus)
}

/// 按等音类把一条材料的音符移调 `delta` 个半音，并给每个音符派生**新**身份。
///
/// 折叠口径：`pitch + delta > 127` 时减一个八度（`delta ∈ 0..=11` 且
/// `pitch ∈ 0..=127` ⇒ 折叠后必然落在 `0..=126`）。音级因此不丢，音域始终合法。
fn transposed_notes(
    project: &YebanProjectV1,
    source: EntityId,
    clip_id: EntityId,
    delta: u8,
) -> BTreeMap<EntityId, MidiNote> {
    let mut out = BTreeMap::new();
    let Some(notes) = project
        .clip_pool
        .get(&source)
        .and_then(|entry| entry.content.notes())
    else {
        return out;
    };
    for (key, note) in notes {
        let raised = u16::from(note.pitch) + u16::from(delta);
        let folded = if raised > 127 { raised - 12 } else { raised };
        // 不变量: delta ∈ 0..=11 且 pitch ∈ 0..=127 ⇒ folded ∈ 0..=126。
        let pitch = u8::try_from(folded).unwrap_or(note.pitch);
        let id = deterministic_id(&format!("note:{clip_id}:{key}"));
        let mut copy = note.clone();
        copy.id = id;
        copy.pitch = pitch;
        out.insert(id, copy);
    }
    out
}

/// 规划一个章节骨架（段落 + 声部音轨 + 片段池条目 + 摆放 + 声部连接）。
///
/// 返回前会把这**整批** op 施加到一个克隆体上并跑 `validate()` ——
/// 半成品绝不交给调用方。
///
/// # Errors
///
/// - 未知风格预设 → `STYLE_NOT_FOUND`；
/// - `bars` 为 0 或超过 [`MAX_BARS`] → `OUT_OF_RANGE`；`scale` 写法非法 →
///   `INVALID_PARAMETER_RANGE`；
/// - 现有路由图已经成环 → `CYCLE_DETECTED`；
/// - 没有主总线音轨 → `TRACK_NOT_FOUND`；
/// - `clip_pool` 里没有可用材料 → `CLIP_NOT_FOUND`（`data.missing` 说明缺什么）；
/// - 施加到克隆体后模型层校验失败 → [`BuildFault::Model`]。
pub fn plan(
    project: &YebanProjectV1,
    section_name: &str,
    style_preset: &str,
    bars: u64,
    scale: Option<&str>,
) -> Result<SectionPlan, BuildFault> {
    // 1. 参数与工程前置条件（顺序即错误码优先级，判据钉住它）。
    let parts = preset_parts(style_preset)?;
    if parts.is_empty() || parts.len() > MAX_PARTS {
        return Err(BuildFault::domain(
            BuildCode::Conflict,
            format!(
                "风格预设 `{style_preset}` 的声部数 {} 不在 1..={MAX_PARTS}",
                parts.len()
            ),
            json!({ "parts": parts, "maxParts": MAX_PARTS }),
        ));
    }
    if bars == 0 || bars > MAX_BARS {
        return Err(BuildFault::domain(
            BuildCode::OutOfRange,
            format!("`bars` 必须在 1..={MAX_BARS}, 实际 {bars}"),
            json!({ "bars": bars, "maxBars": MAX_BARS }),
        ));
    }
    let scale = match scale {
        Some(text) => Some(parse_scale(text)?),
        None => None,
    };
    if let Some(cycle) = detect_cycle(&project.routing_graph) {
        return Err(BuildFault::domain(
            BuildCode::CycleDetected,
            "工程现有的声部连接存在环路, 拒绝在其上叠加配器骨架",
            json!({
                "cycle": cycle.iter().map(EntityId::to_canonical_string).collect::<Vec<_>>(),
            }),
        ));
    }
    let bus = master_bus(project)?;
    let materials = usable_materials(project);
    if materials.is_empty() {
        return Err(BuildFault::domain(
            BuildCode::ClipNotFound,
            "片段池 `clip_pool` 里没有可用作配器材料的条目 (需要 MIDI 且至少一个音符)",
            json!({
                "missing": "usableClipPoolEntries",
                "acceptedContent": "Midi clip with at least one note",
                "clipPoolEntries": project
                    .clip_pool
                    .keys()
                    .map(EntityId::to_canonical_string)
                    .collect::<Vec<_>>(),
                "clipPoolEntryCount": project.clip_pool.len(),
                "requiredParts": parts.len(),
                "why": "配器骨架的每个声部都要一条真实片段池条目; 本线不做'凭空造 id', \
                        也不退回含糊的 unwired",
            }),
        ));
    }

    // 2. 时间轴与身份（确定性：同一请求 ⇒ 同一份载荷）。
    let ticks_per_bar = ticks_per_bar(project);
    let start_tick = project
        .sections
        .values()
        .map(|section| section.end_tick)
        .max()
        .unwrap_or(0);
    let end_tick = start_tick.saturating_add(ticks_per_bar.saturating_mul(bars));
    let span = end_tick.saturating_sub(start_tick);
    let scale_key = scale
        .as_ref()
        .map_or_else(|| "-".to_owned(), Scale::canonical);
    let section_id = deterministic_id(&format!(
        "section:{section_name}:{style_preset}:{bars}:{start_tick}:{scale_key}"
    ));
    let section = SectionV3 {
        id: section_id,
        name: section_name.to_owned(),
        start_tick,
        end_tick,
        color: None,
    };

    // 3. 每个声部的身份（一次算清，后面几个相位都用它）。
    struct Part {
        track_id: EntityId,
        clip_id: EntityId,
        placement_id: EntityId,
        edge_id: EntityId,
        name: String,
        color: Option<String>,
        material: Material,
    }
    let part_plans: Vec<Part> = parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let name = format!("{section_name} · {part}");
            Part {
                track_id: deterministic_id(&format!("part:{section_id}:{index}:{part}")),
                clip_id: deterministic_id(&format!("clip:{section_id}:{index}:{part}")),
                placement_id: deterministic_id(&format!("placement:{section_id}:{index}:{part}")),
                edge_id: deterministic_id(&format!("edge:{section_id}:{index}:{part}:{bus}")),
                name,
                color: COLORS.get(index % COLORS.len()).map(|c| (*c).to_owned()),
                material: materials[index % materials.len()],
            }
        })
        .collect();

    // 4. 组批（顺序 = 施加顺序；子操作的前置条件按顺序被模型层检查）。
    let mut ops: Vec<Op> = Vec::with_capacity(1 + part_plans.len() * 5);
    ops.push(Op::SetSection {
        section_id,
        old_section: project.sections.get(&section_id).cloned(),
        new_section: section,
    });
    // 4a. 片段池条目：内容取自真实材料（可选的等音类移调），身份是确定性派生。
    for part in &part_plans {
        let delta = scale.as_ref().map_or(0, |scale| {
            (scale.tonic_pc + 12 - part.material.root_pc) % 12
        });
        ops.push(Op::AddClip {
            clip: ClipPoolEntry {
                id: part.clip_id,
                name: part.name.clone(),
                content: ClipContent::Midi {
                    notes: transposed_notes(project, part.material.id, part.clip_id, delta),
                },
            },
        });
    }
    // 4b. 声部音轨。
    for part in &part_plans {
        ops.push(Op::AddTrack {
            track: TrackV3 {
                id: part.track_id,
                name: part.name.clone(),
                kind: TrackKind::Midi,
                color: part.color.clone(),
                ..TrackV3::default()
            },
        });
    }
    // 4c. 把片段摆到声部音轨上（覆盖整个段落）。
    for part in &part_plans {
        ops.push(Op::AddClipPlacement {
            track_id: part.track_id,
            placement: ClipPlacement {
                id: part.placement_id,
                clip_id: part.clip_id,
                start_tick,
                duration_ticks: span,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        });
    }
    // 4d. 路由节点：声部 + **必要时的**主总线（已在节点表里就不重复加 ——
    //     `AddRoutingNode` 的前置条件拒绝重复身份）。
    let mut added_routing_nodes: Vec<EntityId> =
        part_plans.iter().map(|part| part.track_id).collect();
    if !project.routing_graph.nodes.contains(&bus) {
        added_routing_nodes.push(bus);
    }
    for node in &added_routing_nodes {
        ops.push(Op::AddRoutingNode { node: *node });
    }
    // 4e. 声部连接：每个声部 → 主总线（`TrackToBus`，与模型规范样本同口径）。
    for part in &part_plans {
        ops.push(Op::ConnectRouting {
            edge: RoutingEdge {
                id: part.edge_id,
                source_node: part.track_id,
                destination_node: bus,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        });
    }

    let op_kinds: Vec<&str> = ops.iter().map(Op::name).collect();
    let unwired = unwired_for_section_op_kinds(&op_kinds);

    // 5. 在克隆体上整体模拟 + 模型层校验（绝不把半成品交给调用方）。
    let mut simulated = project.clone();
    Op::Batch {
        ops: ops.clone(),
        description: format!("propose_section {section_name}"),
    }
    .apply(&mut simulated)
    .map_err(|error| BuildFault::Model {
        context: "章节骨架模拟",
        error,
    })?;
    simulated.validate().map_err(|error| BuildFault::Model {
        context: "章节骨架校验",
        error,
    })?;

    Ok(SectionPlan {
        section_id,
        start_tick,
        end_tick,
        ticks_per_bar,
        part_track_ids: part_plans.iter().map(|part| part.track_id).collect(),
        part_clip_ids: part_plans.iter().map(|part| part.clip_id).collect(),
        placement_ids: part_plans.iter().map(|part| part.placement_id).collect(),
        routing_edge_ids: part_plans.iter().map(|part| part.edge_id).collect(),
        added_routing_nodes,
        ops,
        unwired,
    })
}

/// 一组 op 的**派生摘要**（`dryRun` 预览与真调用响应共用同一份形状）。
///
/// 它不复制 op 载荷，而是从 op 本体里数出"将会新建哪些实体" ——
/// 于是"将要做什么"与"真的做了什么"不可能漂移（同一个 `ops` 向量）。
#[must_use]
pub fn summarize_ops(ops: &[Op]) -> Value {
    let mut sections: Vec<Value> = Vec::new();
    let mut tracks: Vec<Value> = Vec::new();
    let mut clip_pool_entries: Vec<Value> = Vec::new();
    let mut placements: Vec<Value> = Vec::new();
    let mut routing_nodes: Vec<Value> = Vec::new();
    let mut routing_edges: Vec<Value> = Vec::new();
    for op in ops {
        match op {
            Op::SetSection { new_section, .. } => {
                sections.push(json!({
                    "id": new_section.id.to_canonical_string(),
                    "name": new_section.name,
                    "startTick": new_section.start_tick,
                    "endTick": new_section.end_tick,
                }));
            }
            Op::AddTrack { track } => {
                tracks.push(json!({
                    "id": track.id.to_canonical_string(),
                    "name": track.name,
                    "kind": serde_json::to_value(track.kind).unwrap_or(Value::Null),
                }));
            }
            Op::AddClip { clip } => {
                let note_count = clip.content.notes().map_or(0, BTreeMap::len);
                clip_pool_entries.push(json!({
                    "id": clip.id.to_canonical_string(),
                    "name": clip.name,
                    "notes": note_count,
                }));
            }
            Op::AddClipPlacement {
                track_id,
                placement,
            } => {
                placements.push(json!({
                    "id": placement.id.to_canonical_string(),
                    "trackId": track_id.to_canonical_string(),
                    "clipId": placement.clip_id.to_canonical_string(),
                    "startTick": placement.start_tick,
                    "durationTicks": placement.duration_ticks,
                }));
            }
            Op::AddRoutingNode { node } => {
                routing_nodes.push(Value::from(node.to_canonical_string()));
            }
            Op::ConnectRouting { edge } => {
                routing_edges.push(json!({
                    "id": edge.id.to_canonical_string(),
                    "sourceNode": edge.source_node.to_canonical_string(),
                    "destinationNode": edge.destination_node.to_canonical_string(),
                    "kind": serde_json::to_value(edge.kind).unwrap_or(Value::Null),
                    "gainDb": edge.gain_db,
                }));
            }
            _ => {}
        }
    }
    let kinds: Vec<Value> = ops.iter().map(|op| Value::from(op.name())).collect();
    json!({
        "opCount": ops.len(),
        "opKinds": kinds,
        "sections": sections,
        "tracks": tracks,
        "clipPoolEntries": clip_pool_entries,
        "placements": placements,
        "routingNodes": routing_nodes,
        "routingEdges": routing_edges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::samples::filled_project;

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

    /// 施加一份规划到克隆体，返回施加后的工程。
    fn applied(plan: &SectionPlan, project: &YebanProjectV1) -> YebanProjectV1 {
        let mut after = project.clone();
        Op::Batch {
            ops: plan.ops.clone(),
            description: "判据".to_owned(),
        }
        .apply(&mut after)
        .expect("整批必须能施加");
        after
    }

    #[test]
    fn note_names_and_pitch_classes_are_aligned() {
        assert_eq!(NOTE_NAMES.len(), NOTE_PITCH_CLASSES.len());
        assert!(NOTE_PITCH_CLASSES.iter().all(|pc| *pc < 12));
        for window in NOTE_PITCH_CLASSES.windows(2) {
            assert!(window[0] <= window[1], "音名表按音高升序");
        }
    }

    #[test]
    fn unknown_preset_is_style_not_found() {
        let project = filled_project();
        let fault = plan(&project, "Chorus", "polka", 8, None).expect_err("未知预设");
        assert_eq!(fault.domain_code(), Some(BuildCode::StyleNotFound));
        let BuildFault::Domain { data, .. } = fault else {
            panic!("必须是领域失败");
        };
        assert!(data["availablePresets"].is_array());
    }

    #[test]
    fn bars_and_scale_are_validated() {
        let project = filled_project();
        let fault = plan(&project, "Chorus", "lofi-beats", 0, None).expect_err("bars=0");
        assert_eq!(fault.domain_code(), Some(BuildCode::OutOfRange));
        let fault =
            plan(&project, "Chorus", "lofi-beats", MAX_BARS + 1, None).expect_err("bars 过大");
        assert_eq!(fault.domain_code(), Some(BuildCode::OutOfRange));
        let fault = plan(&project, "Chorus", "lofi-beats", 4, Some("H dorian")).expect_err("音名");
        assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
        let fault = plan(&project, "Chorus", "lofi-beats", 4, Some("C bogus")).expect_err("调式");
        assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
        plan(&project, "Chorus", "lofi-beats", 4, Some("c MINOR")).expect("大小写不敏感");
        let fault = plan(&project, "Chorus", "lofi-beats", 4, Some("Cminor")).expect_err("缺空格");
        assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    }

    #[test]
    fn plan_is_deterministic_and_applies_cleanly() {
        let project = filled_project();
        let first = plan(&project, "Chorus", "synthwave", 8, Some("C minor")).expect("规划");
        let second = plan(&project, "Chorus", "synthwave", 8, Some("C minor")).expect("规划");
        assert_eq!(first, second, "同一请求必须产出同一份规划");
        assert_eq!(first.part_track_ids.len(), 4);
        assert_eq!(first.part_clip_ids.len(), 4);
        assert_eq!(first.placement_ids.len(), 4);
        assert_eq!(first.routing_edge_ids.len(), 4);
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

        let simulated = applied(&first, &project);
        simulated.validate().expect("施加后必须合法");
    }

    /// 判据 ① + ②：骨架（段落 / 片段 / 摆放）与声部连接（节点 / 边）**真的**多出来。
    #[test]
    fn skeleton_and_voice_routing_really_exist_after_applying() {
        let project = filled_project();
        let plan = plan(&project, "Chorus", "synthwave", 8, Some("C minor")).expect("规划");
        let after = applied(&plan, &project);

        // ① 段落 + 片段池条目 + 摆放 + 音轨的具体增量。
        assert_eq!(after.sections.len(), project.sections.len() + 1);
        let section = after.sections.get(&plan.section_id).expect("新段落");
        assert_eq!(section.name, "Chorus");
        assert_eq!(section.start_tick, plan.start_tick);
        assert_eq!(section.end_tick, plan.end_tick);
        assert_eq!(after.clip_pool.len(), project.clip_pool.len() + 4);
        assert_eq!(after.tracks.len(), project.tracks.len() + 4);
        for (index, clip_id) in plan.part_clip_ids.iter().enumerate() {
            let entry = after.clip_pool.get(clip_id).expect("新片段池条目");
            let notes = entry.content.notes().expect("必须是 MIDI 片段");
            assert!(!notes.is_empty(), "配器骨架的片段必须带真实材料: {index}");
            assert!(entry.name.starts_with("Chorus · "), "{}", entry.name);
        }
        for (index, track_id) in plan.part_track_ids.iter().enumerate() {
            let track = after.tracks.get(track_id).expect("新音轨");
            assert_eq!(track.kind, TrackKind::Midi);
            assert_eq!(track.clips.len(), 1, "每个声部恰好一个摆放");
            let placement = track.clips.values().next().expect("摆放");
            assert_eq!(placement.id, plan.placement_ids[index]);
            assert_eq!(placement.clip_id, plan.part_clip_ids[index]);
            assert_eq!(placement.start_tick, plan.start_tick);
            assert_eq!(placement.duration_ticks, plan.end_tick - plan.start_tick);
            assert!(!placement.muted);
            assert!(!placement.loop_config.enabled);
        }

        // ② 声部连接：节点集合 + 边的方向与类型。
        for track_id in &plan.part_track_ids {
            assert!(
                after.routing_graph.nodes.contains(track_id),
                "声部必须在路由图节点表里"
            );
        }
        let bus = project.master_bus_track_id;
        for (index, track_id) in plan.part_track_ids.iter().enumerate() {
            let edge = after
                .routing_graph
                .edges
                .get(&plan.routing_edge_ids[index])
                .expect("新路由边");
            assert_eq!(edge.source_node, *track_id, "方向: 声部 → 主总线");
            assert_eq!(edge.destination_node, bus);
            assert_eq!(edge.kind, RoutingKind::TrackToBus);
            assert_eq!(edge.gain_db, None, "单位增益 = None, 不是 Some(0.0)");
        }
        let outgoing = after
            .routing_graph
            .edges
            .values()
            .filter(|edge| {
                edge.destination_node == bus && plan.part_track_ids.contains(&edge.source_node)
            })
            .count();
        assert_eq!(
            outgoing,
            plan.part_track_ids.len(),
            "声部 → 主总线的边必须恰好每个声部一条"
        );
    }

    /// 判据 ③：整批的**逆操作**把工程逐字节带回调用前。
    #[test]
    fn batch_inverse_restores_the_project_byte_for_byte() {
        let project = filled_project();
        let before = serde_json::to_string(&project).expect("序列化");
        let plan = plan(
            &project,
            "Chorus",
            "cinematic-orchestral",
            4,
            Some("D dorian"),
        )
        .expect("规划");
        let batch = Op::Batch {
            ops: plan.ops.clone(),
            description: "propose_section Chorus".to_owned(),
        };
        let mut after = project.clone();
        batch.apply(&mut after).expect("施加");
        assert_ne!(
            serde_json::to_string(&after).expect("序列化"),
            before,
            "必须真的变了"
        );

        batch
            .apply_inverse(&mut after)
            .expect("逆操作必须可构造可施加");
        assert_eq!(
            serde_json::to_string(&after).expect("序列化"),
            before,
            "逆操作之后必须逐字节回到调用前"
        );
    }

    /// 判据 ④（的纯逻辑半边）：规划本身**只读**，且失败路径也不动工程。
    #[test]
    fn planning_never_mutates_the_project() {
        let project = filled_project();
        let before = serde_json::to_string(&project).expect("序列化");
        plan(&project, "Chorus", "synthwave", 8, Some("C minor")).expect("规划");
        plan(&project, "Chorus", "polka", 8, None).expect_err("未知预设");
        plan(&project, "Chorus", "synthwave", MAX_BARS + 1, None).expect_err("bars 越界");
        assert_eq!(
            serde_json::to_string(&project).expect("序列化"),
            before,
            "规划不得改工程一个字节"
        );
    }

    /// 判据 ⑤：片段池缺件 ⇒ **明确**错误（不是 panic、不是空工程、不是 unwired）。
    #[test]
    fn missing_material_is_a_clear_clip_not_found() {
        // 空池 / 只有音频条目 / 只有空 MIDI 条目 —— 三种都不算"可用材料"。
        let mut empty = filled_project();
        empty.clip_pool.clear();
        let mut audio_only = filled_project();
        audio_only
            .clip_pool
            .retain(|_, entry| entry.content.notes().is_none());
        let mut blank = filled_project();
        blank.clip_pool.clear();
        let blank_id = deterministic_id("blank-clip");
        blank.clip_pool.insert(
            blank_id,
            ClipPoolEntry {
                id: blank_id,
                name: "Blank".to_owned(),
                content: ClipContent::Midi {
                    notes: BTreeMap::new(),
                },
            },
        );

        for (label, project) in [
            ("空池", &empty),
            ("只有音频条目", &audio_only),
            ("只有空 MIDI 条目", &blank),
        ] {
            let fault = plan(project, "Chorus", "lofi-beats", 4, None).expect_err(label);
            assert_eq!(
                fault.domain_code(),
                Some(BuildCode::ClipNotFound),
                "{label}"
            );
            let BuildFault::Domain { data, .. } = fault else {
                panic!("必须是领域失败");
            };
            assert_eq!(data["missing"], "usableClipPoolEntries", "{label}");
            assert_eq!(
                data["clipPoolEntryCount"],
                project.clip_pool.len(),
                "{label}"
            );
            assert!(data["clipPoolEntries"].is_array(), "{label}");
            assert!(data["why"].is_string(), "{label}: 必须说明缺什么");
        }
    }

    /// 判据 ⑤（同类）：没有主总线 ⇒ 也必须是明确错误。
    #[test]
    fn missing_master_bus_is_a_clear_track_not_found() {
        let mut project = filled_project();
        project.master_bus_track_id = EntityId::default();
        let fault = plan(&project, "Chorus", "lofi-beats", 4, None).expect_err("无主总线");
        assert_eq!(fault.domain_code(), Some(BuildCode::TrackNotFound));
        let BuildFault::Domain { data, .. } = fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(data["missing"], "masterBusTrack");
    }

    /// 判据 ⑦：章节名 / 风格 / 小节数 / 调式**真的**影响输出，且各自可解释。
    #[test]
    fn section_name_style_bars_and_scale_really_change_the_output() {
        let project = filled_project();

        // 章节名 → 段落名 + 音轨名 + 身份。
        let chorus = plan(&project, "Chorus", "synthwave", 8, None).expect("规划");
        let verse = plan(&project, "Verse", "synthwave", 8, None).expect("规划");
        assert_ne!(chorus.section_id, verse.section_id);
        assert_ne!(chorus.part_track_ids, verse.part_track_ids);
        let chorus_after = applied(&chorus, &project);
        assert_eq!(
            chorus_after.tracks[&chorus.part_track_ids[0]].name,
            "Chorus · Pad"
        );
        assert_eq!(
            chorus_after.clip_pool[&chorus.part_clip_ids[0]].name,
            "Chorus · Pad"
        );

        // 风格 → 声部数与声部名。
        let lofi = plan(&project, "Chorus", "lofi-beats", 8, None).expect("规划");
        assert_eq!(chorus.part_track_ids.len(), 4, "synthwave 4 声部");
        assert_eq!(lofi.part_track_ids.len(), 3, "lofi-beats 3 声部");
        let lofi_after = applied(&lofi, &project);
        assert_eq!(
            lofi_after.tracks[&lofi.part_track_ids[0]].name,
            "Chorus · Keys"
        );

        // 小节数 → 段落跨度与摆放时值。
        let long = plan(&project, "Chorus", "synthwave", 16, None).expect("规划");
        assert_eq!(
            long.end_tick - long.start_tick,
            2 * (chorus.end_tick - chorus.start_tick)
        );
        assert_ne!(long.section_id, chorus.section_id);

        // 调式 → 材料按**等音类移调**到该调主音（音级不丢, 音域始终合法）。
        let c_minor = plan(&project, "Chorus", "synthwave", 8, Some("C minor")).expect("规划");
        let d_minor = plan(&project, "Chorus", "synthwave", 8, Some("D minor")).expect("规划");
        assert_ne!(
            c_minor.part_clip_ids, d_minor.part_clip_ids,
            "调式进了身份种子"
        );
        let c_after = applied(&c_minor, &project);
        let d_after = applied(&d_minor, &project);
        let source = project
            .clip_pool
            .values()
            .find_map(|entry| entry.content.notes())
            .expect("材料");
        let source_root_pc = source
            .values()
            .map(|note| note.pitch % 12)
            .min()
            .expect("材料最低音的音级");
        let mut expected_source: Vec<u8> = source.values().map(|note| note.pitch).collect();
        expected_source.sort_unstable();
        let c_pitches = sorted_pitches(&c_after.clip_pool[&c_minor.part_clip_ids[0]]);
        let d_pitches = sorted_pitches(&d_after.clip_pool[&d_minor.part_clip_ids[0]]);
        assert_eq!(c_pitches.len(), expected_source.len());
        // 移调口径（与实现同一条规则, 但用**源材料**独立算一遍）。
        let fold = |pitch: u8, tonic: u16| -> u8 {
            let delta = (tonic + 12 - u16::from(source_root_pc)) % 12;
            let raised = u16::from(pitch) + delta;
            u8::try_from(if raised > 127 { raised - 12 } else { raised }).expect("合法")
        };
        let expected_c: Vec<u8> = expected_source
            .iter()
            .map(|pitch| fold(*pitch, 0))
            .collect();
        let expected_d: Vec<u8> = expected_source
            .iter()
            .map(|pitch| fold(*pitch, 2))
            .collect();
        assert_eq!(c_pitches, expected_c, "C minor 的移调口径");
        assert_eq!(d_pitches, expected_d, "D minor 的移调口径");
        assert_ne!(c_pitches, d_pitches, "调式不同 ⇒ 音高不同");
    }

    /// 判据 ⑧：`unwired` 由**真实 op** 推导（不是硬编码声明）。
    #[test]
    fn unwired_is_derived_from_the_real_ops() {
        let project = filled_project();
        let plan = plan(&project, "Chorus", "synthwave", 8, None).expect("规划");
        let kinds: Vec<&str> = plan.ops.iter().map(Op::name).collect();
        assert!(kinds.contains(&"AddClip"), "{kinds:?}");
        assert!(kinds.contains(&"AddTrack"), "{kinds:?}");
        assert!(kinds.contains(&"AddClipPlacement"), "{kinds:?}");
        assert!(kinds.contains(&"AddRoutingNode"), "{kinds:?}");
        assert!(kinds.contains(&"ConnectRouting"), "{kinds:?}");
        assert_eq!(
            plan.unwired,
            Vec::<&'static str>::new(),
            "片段池与声部连接都已接线 ⇒ 不许再上报这两个键"
        );

        // 反向: 把真实缺席的相位摘掉 ⇒ unwired 必须**自己**说话。
        let without_clip: Vec<&str> = kinds
            .iter()
            .copied()
            .filter(|kind| *kind != "AddClip")
            .collect();
        assert_eq!(
            unwired_for_section_op_kinds(&without_clip),
            vec!["clipPoolEntries"]
        );
        let without_routing: Vec<&str> = kinds
            .iter()
            .copied()
            .filter(|kind| *kind != "ConnectRouting")
            .collect();
        assert_eq!(
            unwired_for_section_op_kinds(&without_routing),
            vec!["routingEdges"]
        );
        let without_nodes: Vec<&str> = kinds
            .iter()
            .copied()
            .filter(|kind| *kind != "AddRoutingNode")
            .collect();
        assert_eq!(
            unwired_for_section_op_kinds(&without_nodes),
            vec!["routingEdges"]
        );
    }

    /// 判据 ⑥（的模型侧半边）：同一请求两次规划 ⇒ 同一批 id；
    /// 因此第二次合并会被模型层**拒绝**，且 `Batch` 的原子性保证工程一个字节没变。
    #[test]
    fn a_second_identical_batch_cannot_be_applied_twice() {
        let project = filled_project();
        let first = plan(&project, "Chorus", "acoustic-folk", 2, Some("G major")).expect("规划");
        let second = plan(&project, "Chorus", "acoustic-folk", 2, Some("G major")).expect("规划");
        assert_eq!(first.ops, second.ops, "确定性身份 ⇒ 逐字节同一批 op");
        let mut after = applied(&first, &project);
        let after_once = serde_json::to_string(&after).expect("序列化");
        let failure = Op::Batch {
            ops: second.ops,
            description: "第二次".to_owned(),
        }
        .apply(&mut after)
        .expect_err("第二次必须被拒绝 (身份/状态与文档不符)");
        assert!(
            matches!(
                failure,
                ModelError::DuplicateEntityId { .. } | ModelError::OpStateMismatch { .. }
            ),
            "{failure:?}"
        );
        assert_eq!(
            serde_json::to_string(&after).expect("序列化"),
            after_once,
            "失败的批次必须原子回滚 (ARCH-OPS-002)"
        );
    }

    /// 判据 ④（的清单半边）：`summarize_ops` 的数与**真的**增量一致。
    #[test]
    fn op_summary_matches_the_real_delta() {
        let project = filled_project();
        let plan = plan(&project, "Chorus", "lofi-beats", 3, Some("A minor")).expect("规划");
        let summary = summarize_ops(&plan.ops);
        let after = applied(&plan, &project);
        assert_eq!(summary["opCount"], plan.ops.len());
        assert_eq!(
            summary["sections"].as_array().map(Vec::len),
            Some(after.sections.len() - project.sections.len())
        );
        assert_eq!(
            summary["tracks"].as_array().map(Vec::len),
            Some(after.tracks.len() - project.tracks.len())
        );
        assert_eq!(
            summary["clipPoolEntries"].as_array().map(Vec::len),
            Some(after.clip_pool.len() - project.clip_pool.len())
        );
        assert_eq!(
            summary["routingEdges"].as_array().map(Vec::len),
            Some(after.routing_graph.edges.len() - project.routing_graph.edges.len())
        );
        assert_eq!(
            summary["routingNodes"].as_array().map(Vec::len),
            Some(after.routing_graph.nodes.len() - project.routing_graph.nodes.len())
        );
        assert_eq!(summary["placements"].as_array().map(Vec::len), Some(3));
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
        assert_eq!(fault.domain_code(), Some(BuildCode::CycleDetected));
        let BuildFault::Domain { data, .. } = fault else {
            panic!("必须是领域失败");
        };
        assert!(data["cycle"].is_array());
    }

    /// 主总线**还不在**节点表里时，`AddRoutingNode` 必须被补上（否则
    /// `ConnectRouting` 的前置条件不成立）。
    #[test]
    fn a_master_bus_missing_from_the_node_table_is_added_once() {
        let mut project = filled_project();
        let bus = project.master_bus_track_id;
        project.routing_graph.nodes.retain(|node| *node != bus);
        project
            .routing_graph
            .edges
            .retain(|_, edge| edge.source_node != bus && edge.destination_node != bus);
        project.validate().expect("合法");

        let plan = plan(&project, "Chorus", "lofi-beats", 2, None).expect("规划");
        assert!(
            plan.added_routing_nodes.contains(&bus),
            "必须补主总线节点: {:?}",
            plan.added_routing_nodes
        );
        assert_eq!(
            plan.added_routing_nodes.len(),
            plan.part_track_ids.len() + 1
        );
        let node_ops = plan
            .ops
            .iter()
            .filter(|op| matches!(op, Op::AddRoutingNode { .. }))
            .count();
        assert_eq!(node_ops, plan.added_routing_nodes.len(), "不得重复加节点");
        let after = applied(&plan, &project);
        assert!(after.routing_graph.nodes.contains(&bus));
    }

    /// 主总线**已经**在节点表里时不得重复加（`AddRoutingNode` 拒绝重复身份）。
    #[test]
    fn an_existing_master_bus_node_is_not_added_twice() {
        let project = filled_project();
        assert!(
            project
                .routing_graph
                .nodes
                .contains(&project.master_bus_track_id)
        );
        let plan = plan(&project, "Chorus", "synthwave", 2, None).expect("规划");
        assert_eq!(plan.added_routing_nodes.len(), plan.part_track_ids.len());
        let node_ops = plan
            .ops
            .iter()
            .filter(|op| matches!(op, Op::AddRoutingNode { .. }))
            .count();
        assert_eq!(node_ops, plan.part_track_ids.len());
    }

    /// 声部数多于材料数时按轮转分配（确定性），且每个片段的音符身份互不相同。
    #[test]
    fn parts_round_robin_over_the_available_materials() {
        let project = filled_project();
        let plan = plan(&project, "Chorus", "synthwave", 2, None).expect("规划");
        let after = applied(&plan, &project);
        let mut ids: Vec<EntityId> = Vec::new();
        for clip_id in &plan.part_clip_ids {
            let notes = after.clip_pool[clip_id]
                .content
                .notes()
                .expect("MIDI")
                .keys()
                .copied()
                .collect::<Vec<_>>();
            assert!(!notes.is_empty());
            for id in notes {
                assert!(!ids.contains(&id), "音符身份必须逐片段独立: {id}");
                ids.push(id);
            }
        }
    }

    /// 材料的音符内容必须**逐字段**来自源片段（只有身份与音高可动）。
    #[test]
    fn material_content_is_preserved_except_pitch_and_identity() {
        let project = filled_project();
        let plan = plan(&project, "Chorus", "lofi-beats", 2, None).expect("规划");
        let after = applied(&plan, &project);
        let source = project
            .clip_pool
            .values()
            .find_map(|entry| entry.content.notes())
            .expect("材料");
        let generated = after.clip_pool[&plan.part_clip_ids[0]]
            .content
            .notes()
            .expect("MIDI");
        assert_eq!(generated.len(), source.len());
        let mut expected: Vec<(u64, u64, u8, u8)> = source
            .values()
            .map(|note| {
                (
                    note.start_tick,
                    note.duration_ticks,
                    note.pitch,
                    note.velocity,
                )
            })
            .collect();
        expected.sort_unstable();
        let mut actual: Vec<(u64, u64, u8, u8)> = generated
            .values()
            .map(|note| {
                (
                    note.start_tick,
                    note.duration_ticks,
                    note.pitch,
                    note.velocity,
                )
            })
            .collect();
        actual.sort_unstable();
        assert_eq!(actual, expected, "不给 scale 时材料原样拷贝");
    }

    /// 排序后的音高列表（判据用）。
    fn sorted_pitches(entry: &ClipPoolEntry) -> Vec<u8> {
        let mut pitches: Vec<u8> = entry
            .content
            .notes()
            .map(|notes| notes.values().map(|note| note.pitch).collect())
            .unwrap_or_default();
        pitches.sort_unstable();
        pitches
    }
}
