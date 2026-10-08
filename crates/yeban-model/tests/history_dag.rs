//! `history.dag` 编解码口径的**端到端**判据 [ARCH-OPS-002, ARCH-SEC-003]。
//!
//! 需求来源是两条**别人登记的**跨 crate needs：
//!
//! - `docs/ledger/undo-wiring-notes.md` §9 needs-2：「`history.dag` 的公开编解码面 …
//!   建议把 `yeban-mcp/src/domain/store.rs::decode_history_dag` 那套口径提到 `yeban-model`」；
//! - `docs/ledger/app-mixer-notes.md` §7 第 7 条：「`force_save` 写出**空字节**的
//!   `history.dag` … 提交图谱的权威内容属 `yeban-model::commit`」。
//!
//! 单元判据（`commit::tests`）钉住编解码本身；本文件钉住它与**容器**（规范 §5.3 的
//! `history.dag` 条目）以及**已有消费者的真实字节**的对接：
//!
//! 1. [`history_dag_survives_the_container_round_trip`] —— 模型编码的字节进容器、出容器后
//!    逐字段读回同一图谱；
//! 2. [`history_dag_bytes_land_in_the_named_entry`] —— 那串字节确实落在 §5.3 的
//!    `history.dag` 条目名下，且容器层逐字节保真（含空字节与任意字节）；
//! 3. [`mcp_caliber_bytes_are_accepted`] —— `yeban-mcp` 今天写出的字节形状
//!    （`serde_json::to_vec(CommitGraph)`）能被本 crate 读回 ⇒ 两侧是**同一份**口径；
//! 4. [`app_style_empty_history_dag_is_read_back_as_no_history`] —— `yeban-app` 今天落的空字节
//!    （`crates/yeban-app/src/cli.rs` 的 `history_dag: Vec::new()`）读回是"还没有历史"，
//!    不是错误（因此把写出侧接到本口径不会让既有工程读不回来）。
//!
//! 全部判据只依赖 `yeban-model` 自身与它的既有依赖，因此在本机真跑（无重依赖）。

use std::collections::BTreeMap;
use std::str::FromStr as _;

use yeban_model::container::{
    ContainerLimits, HISTORY_DAG_NAME, read_container, read_project_container,
    write_project_container,
};
use yeban_model::{
    CommitDraft, CommitGraph, EntityId, Op, OpOrigin, StampedOp, decode_history_dag,
    encode_history_dag,
};

/// 与 `src/commit.rs` 的判据同一套夹具 ULID（26 字符 Crockford Base32）。
fn fixture_id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

fn mute_op(index: u128) -> StampedOp {
    StampedOp::new(
        OpOrigin::UserUi,
        1_760_000_000_000,
        Op::SetTrackMute {
            track_id: fixture_id(index),
            old_mute: false,
            new_mute: true,
        },
    )
}

/// 两个命名提交（根 + 一次追加），各自带一条 op ⇒ 载荷非空。
fn fixture_graph() -> CommitGraph {
    let mut graph = CommitGraph::new();
    graph
        .genesis(
            CommitDraft::new(fixture_id(1), "main", "agent", "genesis").with_ops(vec![mute_op(1)]),
        )
        .expect("genesis");
    graph
        .append(
            CommitDraft::new(fixture_id(2), "main", "agent", "second").with_ops(vec![mute_op(2)]),
        )
        .expect("append");
    graph
}

#[test]
fn history_dag_survives_the_container_round_trip() {
    let project = yeban_model::samples::filled_project();
    let graph = fixture_graph();
    let dag = encode_history_dag(&graph);

    let bytes = write_project_container(&project, &dag, &BTreeMap::new()).expect("写出容器");
    let archive = read_project_container(&bytes, &ContainerLimits::default()).expect("读回容器");

    assert_eq!(archive.project, project, "工程文档必须逐字段往返");
    assert_eq!(archive.history_dag, dag, "history.dag 必须逐字节保真");
    assert_eq!(
        decode_history_dag(&archive.history_dag).expect("解码"),
        Some(graph),
        "容器往返之后必须读回同一张提交图谱"
    );
}

#[test]
fn history_dag_bytes_land_in_the_named_entry() {
    let project = yeban_model::samples::default_project();
    let graph = fixture_graph();
    let dag = encode_history_dag(&graph);

    let bytes = write_project_container(&project, &dag, &BTreeMap::new()).expect("写出容器");
    let archive = read_container(&bytes, &ContainerLimits::default()).expect("读回裸容器");
    let entry = archive
        .get(HISTORY_DAG_NAME)
        .expect("§5.3 的 history.dag 条目");
    assert_eq!(entry.data, dag, "编码字节必须原样落在 history.dag 条目里");

    // 容器层**不**解读这一条目：任意字节（含空字节）都逐字节往返。
    for raw in [
        b"".as_slice(),
        b"{\"not\":\"a graph\"}".as_slice(),
        b"\x00\xff".as_slice(),
    ] {
        let bytes = write_project_container(&project, raw, &BTreeMap::new()).expect("写出容器");
        let archive =
            read_project_container(&bytes, &ContainerLimits::default()).expect("读回容器");
        assert_eq!(archive.history_dag, raw, "容器层不得重编码 history.dag");
    }
}

#[test]
fn mcp_caliber_bytes_are_accepted() {
    // `yeban-mcp` 今天写出的就是 `serde_json::to_vec(graph)`
    // （`crates/yeban-mcp/src/domain/store.rs` 的 `container_bytes`）。
    // 本 crate 必须读得回这串字节，否则"把口径提到模型层"会是一次格式变更。
    let graph = fixture_graph();
    let mcp_bytes = serde_json::to_vec(&graph).expect("mcp 口径");
    assert_eq!(
        decode_history_dag(&mcp_bytes).expect("解码"),
        Some(graph),
        "模型层必须接受 yeban-mcp 已写出的字节"
    );
}

#[test]
fn app_style_empty_history_dag_is_read_back_as_no_history() {
    // `yeban-app` 今天 `force_save` 落的是空字节（`history_dag: Vec::new()`）。
    // 空条目读回必须是"还没有历史"而不是错误 —— 既有工程不会因为接线而打不开。
    let project = yeban_model::samples::default_project();
    let bytes = write_project_container(&project, b"", &BTreeMap::new()).expect("写出容器");
    let archive = read_project_container(&bytes, &ContainerLimits::default()).expect("读回容器");
    assert!(archive.history_dag.is_empty());
    assert_eq!(
        decode_history_dag(&archive.history_dag).expect("解码"),
        None
    );
}
