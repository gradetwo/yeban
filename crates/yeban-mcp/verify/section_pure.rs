//! 本机零重依赖验证脚手架 —— `yeban_propose_section` 的**真**骨架生成器 [MCP-TOOL-005]。
//!
//! 它**不是** cargo 目标（不在 `tests/` 下）：`yeban-mcp` 一旦依赖
//! `yeban-render` / `yeban-decode`（rayon/hound/midly/symphonia/rubato）就无法在本机编译
//! （`AGENTS.md` §5 的本机纪律），而"骨架长什么样、声部连到哪、逆操作能不能逐字节回退"
//! 这几件事只依赖 `yeban-model` + `serde_json`。
//!
//! ```text
//! bash scripts/dev/cargo-local.sh build -p yeban-model
//! DEPS=target/debug/deps
//! YM=$(ls $DEPS/libyeban_model-*.rlib | head -1)
//! JS=$(ls $DEPS/libserde_json-*.rlib | head -1)
//! rustc --edition 2024 --test -D warnings crates/yeban-mcp/verify/section_pure.rs \
//!   --extern yeban_model=$YM --extern serde_json=$JS -L dependency=$DEPS -o /tmp/section_pure
//! /tmp/section_pure
//! ```
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
    let plan = plan(&project, "Chorus", "cinematic-orchestral", 2, None).expect("规划");
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
    let fault = plan(&project, "Chorus", "lofi-beats", 4, None).expect_err("无材料");
    assert_eq!(fault.domain_code(), Some(BuildCode::ClipNotFound));
    assert!(matches!(fault, BuildFault::Domain { .. }));
}
