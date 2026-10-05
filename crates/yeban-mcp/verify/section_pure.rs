//! 本机零重依赖验证脚手架 —— `yeban_propose_section` 的**真**骨架生成器 [MCP-TOOL-005]。
//!
//! 它**不是** cargo 目标（不在 `tests/` 下）：`yeban-mcp` 一旦依赖
//! `yeban-render` / `yeban-decode`（rayon/hound/midly/symphonia/rubato）就无法在本机编译
//! （`AGENTS.md` §5 的本机纪律），而"骨架长什么样、声部连到哪、逆操作能不能逐字节回退"
//! 这几件事只依赖 `yeban-model` + `serde_json`。
//!
//! ```text
//! bash scripts/dev/cargo-local.sh build -p yeban-model
//! bash scripts/dev/cargo-local.sh build -p yeban-theory   # `line/theory-wiring` 新增的依赖边
//! DEPS=target/debug/deps
//! YM=$(ls $DEPS/libyeban_model-*.rlib | head -1)
//! JS=$(ls $DEPS/libserde_json-*.rlib | head -1)
//! YT=$(ls $DEPS/libyeban_theory-*.rlib | head -1)
//! rustc --edition 2024 --test -D warnings crates/yeban-mcp/verify/section_pure.rs \
//!   --extern yeban_model=$YM --extern serde_json=$JS --extern yeban_theory=$YT \
//!   -L dependency=$DEPS -o /tmp/section_pure
//! /tmp/section_pure
//! ```
//!
//! `--extern yeban_theory=…` 不是可选项：本文件用 `#[path]` 引入的
//! `src/domain/section_build.rs` 现在就 `use yeban_theory::…`（`ADR-0001` D49 的接线），
//! 少这一个 `--extern` 连编译都过不去 —— 这也是判据 ⑥「依赖边真的存在」的**编译级**证据。
//!
//! 关键点：本文件用 `#[path]` 引入**真实源文件**（`src/domain/section_build.rs` 与
//! `src/domain/ids.rs`），不是抄一份会漂移的副本 —— 本机跑过的就是 CI 跑的那一份代码。
//! `section_build.rs` 自带的 `#[cfg(test)] mod tests` 会随 `--test` 一起编译执行，
//! 因此 ①–⑧ 的主体判据在本机**真的**跑；本文件在此之上加三条**独立口径**的对抗性判据：
//!
//! - `routing_edges_are_never_cyclic`：生成的连接图必须是 DAG（自环/回边直接红）；
//! - `skeleton_ids_are_never_nil_and_are_distinct`：任何新身份都不得是 nil、
//!   同一批里不得撞车、派生音符身份不得复用源材料身份（"凭空造 id" 最危险的形态
//!   就是造出 nil、撞车或与既有实体同名）；
//! - `missing_material_reports_the_typed_fault`：缺材料必须是**类型化**的领域失败。
//!
//! `ADR-0001` **D49**（`line/theory-wiring`）之后再加三条**独立口径**的接线判据 ——
//! 它们全部**直接问 theory**（`GenreLibrary` / `ScaleKind` / `ChordSpan`），不读本模块的
//! 中间量，因此"把 theory 的查询换成硬编码常量"这类注入会在这里变红：
//!
//! - `dorian_and_minor_differ_exactly_by_the_sixth`：`D dorian` / `D minor` 的逐音输出
//!   （写死期望值）；
//! - `every_theory_genre_maps_to_a_voice_count_from_its_own_chords`：182 条流派逐条比对
//!   "声部数 = theory 给的和弦构成音数"；
//! - `the_preset_catalogue_is_the_theory_library`：候选清单**就是** `GenreLibrary::ids()`。

// 本文件是 `rustc --test` 的 **crate root**, 不是库; 被包含模块的公开 API 在这里
// 没有"外部消费者", 因此 `dead_code` 会误报。真实判定由 CI 的
// `cargo clippy -p yeban-mcp --all-targets -- -D warnings` 在 lib crate 上执行。
#![allow(dead_code)]

#[path = "../src/domain/ids.rs"]
mod ids;
#[path = "../src/domain/section_build.rs"]
mod section_build;

use std::collections::BTreeMap;

use yeban_model::{EntityId, Op, RoutingGraph, samples::filled_project};

use section_build::{BuildCode, BuildFault, SectionPlan, detect_cycle, plan};

/// 把一份规划的整批 op 施加到样本工程的克隆体上。
fn applied(
    project: &yeban_model::YebanProjectV1,
    plan: &SectionPlan,
) -> yeban_model::YebanProjectV1 {
    let mut after = project.clone();
    Op::Batch {
        ops: plan.ops.clone(),
        description: "harness".to_owned(),
    }
    .apply(&mut after)
    .expect("整批必须能施加");
    after
}

#[test]
fn routing_edges_are_never_cyclic() {
    let project = filled_project();
    let plan = plan(&project, "Chorus", "synthwave", 4, Some("C minor")).expect("规划");
    let after = applied(&project, &plan);

    // 声部连接不得引入任何环（判据独立于 section_build 内部的 DFS 实现）。
    for (index, edge_id) in plan.routing_edge_ids.iter().enumerate() {
        let edge = after
            .routing_graph
            .edges
            .get(edge_id)
            .unwrap_or_else(|| panic!("第 {index} 条边必须存在"));
        assert_ne!(edge.source_node, edge.destination_node, "自环: {edge:?}");
    }
    let mut edges: BTreeMap<EntityId, yeban_model::RoutingEdge> = BTreeMap::new();
    for (id, edge) in &after.routing_graph.edges {
        if plan.routing_edge_ids.contains(id) {
            edges.insert(*id, *edge);
        }
    }
    let graph = RoutingGraph {
        nodes: after.routing_graph.nodes.clone(),
        edges,
    };
    assert!(
        detect_cycle(&graph).is_none(),
        "声部连接必须是 DAG: {:?}",
        detect_cycle(&graph)
    );
}

#[test]
fn skeleton_ids_are_never_nil_and_are_distinct() {
    let project = filled_project();
    let plan = plan(&project, "Chorus", "orchestral_film_score", 2, None).expect("规划");
    // **新建实体**的身份：段落 / 片段池条目 / 摆放 / 音轨 / 路由边。
    let mut minted: Vec<EntityId> = vec![plan.section_id];
    minted.extend(plan.part_track_ids.iter().copied());
    minted.extend(plan.part_clip_ids.iter().copied());
    minted.extend(plan.placement_ids.iter().copied());
    minted.extend(plan.routing_edge_ids.iter().copied());
    for id in &minted {
        assert!(!id.is_nil(), "身份不得是 nil");
    }
    for (index, id) in minted.iter().enumerate() {
        assert!(
            !minted[index + 1..].contains(id),
            "同一批里的身份必须互不相同: {id}"
        );
    }
    // 路由节点表里的身份**必须**是声部音轨本身（+ 可能的主总线）——
    // 不得凭空造出第三类节点身份。
    let bus = project.master_bus_track_id;
    for node in &plan.added_routing_nodes {
        assert!(
            plan.part_track_ids.contains(node) || *node == bus,
            "路由节点必须是真实音轨身份: {node}"
        );
    }
    for (index, node) in plan.added_routing_nodes.iter().enumerate() {
        assert!(
            !plan.added_routing_nodes[index + 1..].contains(node),
            "路由节点不得重复: {node}"
        );
    }

    // 生成出来的**音符**身份也不得与材料里的音符撞车（逐片段独立）。
    let after = applied(&project, &plan);
    let source_ids: Vec<EntityId> = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.keys().copied())
        .collect();
    for clip_id in &plan.part_clip_ids {
        let notes = after.clip_pool[clip_id]
            .content
            .notes()
            .expect("生成的片段必须是 MIDI");
        assert!(!notes.is_empty(), "生成的片段必须带材料");
        for id in notes.keys() {
            assert!(
                !source_ids.contains(id),
                "派生音符身份不得复用源材料的身份: {id}"
            );
        }
    }
}

#[test]
fn missing_material_reports_the_typed_fault() {
    let mut project = filled_project();
    project.clip_pool.clear();
    let fault = plan(&project, "Chorus", "lo_fi_hip_hop", 4, None).expect_err("无材料");
    assert_eq!(fault.domain_code(), Some(BuildCode::ClipNotFound));
    assert!(matches!(fault, BuildFault::Domain { .. }));
}

/// 判据 ①（独立口径）：调式语义真的进了输出，且 `D dorian` / `D minor` 的差异**恰好**
/// 是第六级 —— 期望值写死，不由 `section_build` 的任何函数算出来。
#[test]
fn dorian_and_minor_differ_exactly_by_the_sixth() {
    // 材料: D4(62) / Bb4(70) / B4(71) / F4(65) / D5(74)，最低音音级 = 2 ⇒ 目标 D 时移调 = 0。
    let mut project = filled_project();
    let material_id = project
        .clip_pool
        .values()
        .find(|entry| entry.content.notes().is_some())
        .map(|entry| entry.id)
        .expect("样本必须有 MIDI 材料");
    let entry = project.clip_pool.get_mut(&material_id).expect("材料");
    let mut notes = BTreeMap::new();
    for (index, (pitch, tick)) in [(62u8, 0u64), (70, 480), (71, 960), (65, 1440), (74, 1920)]
        .into_iter()
        .enumerate()
    {
        let id = ids::deterministic_id(&format!("harness-note:{index}"));
        notes.insert(id, yeban_model::MidiNote::new(id, tick, pitch, 480));
    }
    let Some(slot) = entry.content.notes_mut() else {
        panic!("材料必须是 MIDI");
    };
    *slot = notes;
    project.validate().expect("夹具必须合法");

    let pitches = |scale: &str| -> Vec<u8> {
        let plan = plan(&project, "Verse", "lo_fi_hip_hop", 2, Some(scale)).expect("规划");
        let after = applied(&project, &plan);
        let mut values: Vec<u8> = after.clip_pool[&plan.part_clip_ids[0]]
            .content
            .notes()
            .expect("MIDI")
            .values()
            .map(|note| note.pitch)
            .collect();
        values.sort_unstable();
        values
    };
    assert_eq!(pitches("D dorian"), vec![62, 65, 69, 71, 74], "D dorian");
    assert_eq!(pitches("D minor"), vec![62, 65, 70, 70, 74], "D minor");
}

/// 判据 ②（独立口径）：182 条 theory 流派逐条比对 —— 声部数必须等于
/// **theory 自己展开的**和弦构成音数。把 `preset_parts` 换成硬编码常量即红。
#[test]
fn every_theory_genre_maps_to_a_voice_count_from_its_own_chords() {
    use yeban_theory::genre::GenreLibrary;
    use yeban_theory::pitch::PitchClass;

    let mut seen: Vec<usize> = Vec::new();
    for rule in GenreLibrary::all() {
        let scale = rule.primary_scale(PitchClass::C).expect("theory 音阶");
        let expected = rule
            .typical_progressions
            .iter()
            .map(|text| yeban_theory::progression::Progression::parse(text).expect("theory 走向"))
            .flat_map(|progression| progression.chords(&scale))
            .map(|chord| chord.pitch_classes().len())
            .max()
            .expect("至少一个和弦");
        let parts = section_build::preset_parts(rule.id).expect("合法预设");
        assert_eq!(parts.len(), expected, "流派 {}", rule.id);
        if !seen.contains(&expected) {
            seen.push(expected);
        }
    }
    seen.sort_unstable();
    assert_eq!(seen, vec![3, 4], "theory 全库只给出 3 与 4 两种构成音数");
}

/// 判据 ③（独立口径）：候选清单**就是** theory 的流派 ID 清单。
#[test]
fn the_preset_catalogue_is_the_theory_library() {
    use yeban_theory::genre::GenreLibrary;

    assert_eq!(section_build::available_presets(), GenreLibrary::ids());
    let mut project = filled_project();
    project.clip_pool.clear();
    let fault = plan(&project, "Chorus", "yeban_unknown_style", 4, None).expect_err("未知风格");
    assert_eq!(fault.domain_code(), Some(BuildCode::StyleNotFound));
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败");
    };
    assert_eq!(
        data["availablePresets"],
        serde_json::json!(GenreLibrary::ids())
    );
}
