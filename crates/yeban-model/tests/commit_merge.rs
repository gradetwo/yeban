//! 命名分支与**多父合并提交**的端到端判据 [ARCH-OPS-002]。
//!
//! 需求来源是一条**别人登记的**跨 crate needs（本 crate 之外的人写下的）：
//! `docs/ledger/tools-domain-notes.md` §7 needs-5 —— 「`CommitGraph` 缺
//! `create_branch(parent)` 与多父合并提交 API … 让"Musical PR"的 DAG 关系
//! 由 `CommitGraph` 而不是 MCP 层的 `Proposal` 记录承担」。
//! 同一条缺口由 `crates/yeban-mcp/src/domain/proposal.rs` 的模块头逐字登记为
//! 「在指定父提交上创建命名分支：**没有**」「创建多父合并提交：**没有**」。
//!
//! 单元判据（`src/commit.rs` 的 `tests`）钉住两个新 API 的行为；本文件从**crate 外部**
//! 钉住两件单元判据覆盖不到的事：
//!
//! 1. [`a_merge_commit_carries_both_parents_through_the_history_dag_codec`] —— 合并提交经
//!    `history.dag` 的公开编解码面往返后，父集合与深度**逐字段**保真（多父形状是持久化事实，
//!    不是内存里的瞬时状态）；
//! 2. [`the_proposal_shaped_flow_keeps_main_undoable_and_the_proposal_isolated`] ——
//!    按 MCP 提案线的用法走一遍（命名分支 → 提案提交 → 合并），断言被合并的一侧
//!    仍是只读孤岛、主分支的撤销链只含主分支那一侧。
//!
//! 全部判据只依赖 `yeban-model` 自身与它的既有依赖，因此在本机真跑（无重依赖）。

use std::str::FromStr as _;

use yeban_model::{
    CommitDraft, CommitGraph, EntityId, Op, OpOrigin, StampedOp, decode_history_dag,
    encode_history_dag,
};

/// 与 `src/commit.rs` 的判据同一套夹具 ULID（26 字符 Crockford Base32）。
fn fixture_id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

fn add_section_op(index: u128) -> StampedOp {
    let section_id = fixture_id(index);
    StampedOp::new(
        OpOrigin::UserUi,
        1_760_000_000_000,
        Op::SetSection {
            section_id,
            old_section: None,
            new_section: yeban_model::SectionV3 {
                id: section_id,
                name: format!("Section {index}"),
                start_tick: 0,
                end_tick: 960,
                color: None,
            },
        },
    )
}

/// main 上两次提交，再在 `base` 上开一条命名分支并提交**两次**；返回
/// `(graph, base, main_head, proposal_head)`。
///
/// 提案侧刻意比 main 侧更深（深度 3 对 2），这样"合并提交的深度 = 最大父深度 + 1"
/// 与"只取第一父深度"两种实现才会给出不同的值。
fn proposal_shaped_graph() -> (CommitGraph, EntityId, EntityId, EntityId) {
    let mut graph = CommitGraph::new();
    let base = graph
        .genesis(
            CommitDraft::new(fixture_id(1), "main", "agent", "genesis")
                .with_ops(vec![add_section_op(11)]),
        )
        .expect("genesis");
    let main_head = graph
        .append(
            CommitDraft::new(fixture_id(2), "main", "agent", "main second")
                .with_ops(vec![add_section_op(12)]),
        )
        .expect("append main");
    graph
        .create_branch("ai/proposal-1", &base)
        .expect("named branch on an existing commit");
    graph
        .append(
            CommitDraft::new(fixture_id(3), "ai/proposal-1", "agent", "proposal 1")
                .with_ops(vec![add_section_op(13)]),
        )
        .expect("append proposal 1");
    let proposal_head = graph
        .append(CommitDraft::new(
            fixture_id(5),
            "ai/proposal-1",
            "agent",
            "proposal 2",
        ))
        .expect("append proposal 2");
    (graph, base, main_head, proposal_head)
}

#[test]
fn a_merge_commit_carries_both_parents_through_the_history_dag_codec() {
    let (mut graph, base, main_head, proposal_head) = proposal_shaped_graph();
    let merge = graph
        .append_merge(
            CommitDraft::new(fixture_id(4), "main", "agent", "merge proposal")
                .with_ops(vec![add_section_op(14)]),
            &[proposal_head],
        )
        .expect("merge");

    let bytes = encode_history_dag(&graph);
    let back = decode_history_dag(&bytes)
        .expect("decode")
        .expect("非空图谱");
    assert_eq!(back, graph, "图谱必须逐字段往返");

    let merged = back.commit(&merge).expect("merge commit");
    assert_eq!(
        merged.parents,
        vec![main_head, proposal_head],
        "两个父都必须写进 history.dag"
    );
    assert!(merged.is_merge());
    assert_eq!(merged.first_parent(), Some(main_head));
    assert_eq!(
        back.depth_of(&merge).expect("depth"),
        4,
        "深度 = 最大父深度 + 1（第一父 2，第二父 3）"
    );
    assert_ne!(
        back.depth_of(&merge).expect("depth"),
        back.depth_of(&main_head).expect("depth") + 1,
        "深度不得只看第一父"
    );
    assert_eq!(back.branch_head("main").expect("main").head, merge);
    assert_eq!(
        back.branch_head("ai/proposal-1").expect("p1").head,
        proposal_head
    );
    assert_eq!(
        back.ancestry(&merge).expect("ancestry"),
        vec![merge, main_head, base],
        "编码回来之后主干方向仍是第一父"
    );
}

#[test]
fn the_proposal_shaped_flow_keeps_main_undoable_and_the_proposal_isolated() {
    let (mut graph, base, main_head, proposal_head) = proposal_shaped_graph();

    // 提案分支是**命名**分支（不再是孤立根提交），且从 base 长出去。
    let proposal_branch = graph.branch_head("ai/proposal-1").expect("proposal branch");
    assert!(!proposal_branch.anonymous);
    assert_eq!(proposal_branch.head, proposal_head);
    let proposal_first = graph
        .commit(&proposal_head)
        .expect("commit")
        .first_parent()
        .expect("proposal head has a parent");
    assert_eq!(
        graph.commit(&proposal_first).expect("commit").parents,
        vec![base],
        "提案分支从 base 长出去，而不是从 main 的头长出去"
    );
    assert_eq!(graph.depth_of(&proposal_first).expect("depth"), 2);

    // 合并进 main：DAG 关系由图谱承担（第一父 = main 的头，第二父 = 提案头）。
    let merge = graph
        .append_merge(
            CommitDraft::new(fixture_id(4), "main", "agent", "merge proposal"),
            &[proposal_head],
        )
        .expect("merge");
    assert_eq!(
        graph.commit(&merge).expect("merge").parents,
        vec![main_head, proposal_head]
    );

    // 被合并的一侧仍是只读孤岛：头不动，深度与父集合不变。
    assert_eq!(
        graph.branch_head("ai/proposal-1").expect("p1").head,
        proposal_head
    );
    assert_eq!(graph.depth_of(&proposal_head).expect("depth"), 3);

    // 主分支的撤销链只含第一父那一侧 ⇒ 一次撤销回退整套被合并的操作，
    // 而提案自己的 op 留在孤岛上，不进主分支的可撤销序列。
    let undoable = graph.ops_backwards(&merge, 16).expect("ops");
    assert_eq!(undoable.len(), 2, "只有 base 与 main 头的 op");
    let touched: Vec<EntityId> = undoable
        .iter()
        .filter_map(|stamped| match &stamped.op {
            Op::SetSection { section_id, .. } => Some(*section_id),
            _ => None,
        })
        .collect();
    assert_eq!(touched, vec![fixture_id(12), fixture_id(11)]);
    assert!(
        !touched.contains(&fixture_id(13)),
        "提案分支的 op 不得进入主分支的撤销链"
    );
}
