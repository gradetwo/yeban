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

use serde_json::json;

use yeban_model::container::{
    ContainerLimits, HISTORY_DAG_NAME, read_container, read_project_container,
    write_project_container,
};
use yeban_model::{
    CommitDraft, CommitGraph, EntityId, HistoryDagError, Op, OpOrigin, StampedOp,
    decode_history_dag, encode_history_dag,
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

// ---------------------------------------------------------------------------
// 不可信 `history.dag` 的边界（`[ARCH-OPS-002]`）
// ---------------------------------------------------------------------------
//
// `history.dag` 来自磁盘或第三方归档。三个集合（`commits` / `branches` / `depths`）
// 是分开保存的，**写入 API** 负责让它们同步，反序列化不做这件事 ⇒ 一份
// "JSON 合法、但集合互相矛盾"的文件在解码时不会发出任何信号。
// 两条被实测的后果（见 `docs/ledger/store-container-notes.md` 的
// `history.dag` 相关条目）：
//
// - 父集合成环 ⇒ `CommitGraph::ancestry` / `ops_backwards` 沿第一父无限前进，
//   进程**挂死**（判据用"① 环"这一条钉住）；
// - 深度缓存被改 ⇒ 快照点（深度 `1, 257, 513, …`）从此算错。
//
// 因此解码边界必须拒绝它们，且**绝不**降级成"空历史"。

/// 三条提交、两个分支头（`main` + 一条匿名分支）的图谱。
///
/// [`fixture_graph`] 只有一条线性分支，不足以覆盖"挪一个分支头"这类改动；
/// 本夹具就是它加上一次 `fork_anonymous`（在根提交上派生）。
fn forkable_graph() -> CommitGraph {
    let mut graph = fixture_graph();
    graph
        .fork_anonymous(
            &fixture_id(1),
            CommitDraft::new(fixture_id(3), "anon-placeholder", "agent", "fork"),
        )
        .expect("fork_anonymous");
    graph
}

/// 一份"**JSON 合法、图谱不自洽**"的 `history.dag` 构造表。
///
/// 每一条都是对一份**合法**图谱（本文件的 `forkable_graph`）的**最小**改动
/// （改一条父边 / 删一个深度条目 / 挪一个分支头 …），因此失败只可能来自被改的那条不变量。
/// 三元组 = `(名字, 字节, 错误文本里必须出现的字样)`。
fn inconsistent_dags() -> Vec<(&'static str, Vec<u8>, &'static str)> {
    let first = fixture_id(1).to_canonical_string();
    let second = fixture_id(2).to_canonical_string();
    let third = fixture_id(3).to_canonical_string();
    let ghost = fixture_id(99).to_canonical_string();

    let base = || serde_json::to_value(forkable_graph()).expect("合法图谱可序列化");
    let bytes = |value: &serde_json::Value| serde_json::to_vec(value).expect("可序列化");
    let mut cases: Vec<(&'static str, Vec<u8>, &'static str)> = Vec::new();

    // ① 父集合成环：两个身份都存在、父边都存在 ⇒ 只有深度递推抓得住它。
    let mut value = base();
    value["commits"][first.as_str()]["parents"] = json!([third]);
    value["commits"][third.as_str()]["parents"] = json!([first]);
    cases.push(("cycle", bytes(&value), "declares depth"));

    // ② 悬空父提交。
    let mut value = base();
    value["commits"][first.as_str()]["parents"] = json!([ghost]);
    cases.push((
        "dangling_parent",
        bytes(&value),
        "references unknown parent",
    ));

    // ③ 深度缓存缺条目（截断文件）。
    let mut value = base();
    value["depths"]
        .as_object_mut()
        .expect("depths 是对象")
        .remove(&second);
    cases.push(("missing_depth", bytes(&value), "has no entry in `depths`"));

    // ④ 深度缓存被改（放行它会让快照点算错）。
    let mut value = base();
    value["depths"][third.as_str()] = json!(1);
    cases.push(("poisoned_depth", bytes(&value), "declares depth"));

    // ⑤ 深度缓存里有不属于任何提交的条目。
    let mut value = base();
    value["depths"][ghost.as_str()] = json!(1);
    cases.push(("orphan_depth", bytes(&value), "entry for unknown commit"));

    // ⑥ 分支头悬空。
    let mut value = base();
    value["branches"]["main"]["head"] = json!(ghost);
    cases.push((
        "dangling_branch_head",
        bytes(&value),
        "points at unknown commit",
    ));

    // ⑦ `commits` 的键与提交内嵌的 `id` 不一致。
    let mut value = base();
    value["commits"][first.as_str()]["id"] = json!(second);
    cases.push((
        "commit_key_mismatch",
        bytes(&value),
        "does not match the embedded id",
    ));

    // ⑧ `branches` 的键与分支内嵌的 `name` 不一致。
    let mut value = base();
    value["branches"]["main"]["name"] = json!("other");
    cases.push((
        "branch_key_mismatch",
        bytes(&value),
        "does not match the embedded name",
    ));

    // ⑨ 声明了分支却一条提交都没有：这是**自相矛盾**，不是"还没有历史"
    //    （自洽性检查必须先于"零提交 ⇒ None"，否则分支声明被静默丢掉）。
    let value = json!({
        "commits": {},
        "branches": {"main": {"name": "main", "head": first, "anonymous": false}},
        "depths": {},
    });
    cases.push((
        "branch_without_any_commit",
        bytes(&value),
        "points at unknown commit",
    ));

    cases
}

#[test]
fn json_valid_but_inconsistent_history_dags_are_rejected_at_the_decode_boundary() {
    let cases = inconsistent_dags();
    assert_eq!(cases.len(), 9, "构造表的条数（每一条不变量至少一条）");
    let mut rejected = 0_usize;
    for (name, raw, needle) in cases {
        // 前提一：字节**真的**是合法 JSON；前提二：它能过 serde 的形状层。
        // 两条件同时成立才说明下面拒绝的原因是"图谱不自洽"，而不是"字节读不出来"。
        let parsed: serde_json::Value = serde_json::from_slice(&raw)
            .unwrap_or_else(|error| panic!("{name}: 夹具必须是合法 JSON（{error}）"));
        assert!(parsed.is_object(), "{name}: 夹具必须是 JSON 对象");
        serde_json::from_slice::<CommitGraph>(&raw)
            .unwrap_or_else(|error| panic!("{name}: 夹具必须能过 serde 形状层（{error}）"));

        match decode_history_dag(&raw) {
            Err(HistoryDagError::InconsistentGraph { detail }) => {
                assert!(
                    detail.contains(needle),
                    "{name}: 错误必须点名 `{needle}`，实测：{detail}"
                );
                rejected += 1;
            }
            other => panic!("{name}: 必须被拒绝为 InconsistentGraph，实测：{other:?}"),
        }
    }
    assert_eq!(rejected, 9, "每一条夹具都必须真的被检查过（不许静默跳过）");
}

#[test]
fn a_decoded_graph_is_always_walkable_to_its_root() {
    // 正面方向：解码**成功**的图谱，每个分支头都能走到根，且祖先链长度恰好等于
    // 它声明的深度。上一条判据拒绝环，这一条钉住"拒绝环"换来的那条性质本身 ——
    // 遍历必然终止，且深度缓存不是装饰品。
    let graph = forkable_graph();
    let decoded = decode_history_dag(&encode_history_dag(&graph))
        .expect("解码")
        .expect("非空图谱");
    assert_eq!(decoded.branches.len(), 2, "夹具：main + 一条匿名分支");
    let mut walked = 0_usize;
    for (name, branch) in &decoded.branches {
        let chain = decoded
            .ancestry(&branch.head)
            .unwrap_or_else(|error| panic!("分支 `{name}` 的祖先链必须走得通：{error}"));
        assert_eq!(
            chain.len() as u64,
            decoded.depth_of(&branch.head).expect("深度"),
            "分支 `{name}` 的链长必须等于声明的深度"
        );
        let root = *chain.last().expect("链非空");
        assert!(
            decoded.commit(&root).expect("root").parents.is_empty(),
            "分支 `{name}` 的链尾必须是根提交"
        );
        walked += 1;
    }
    assert_eq!(walked, 2, "两个分支头都必须真的走过");
}
