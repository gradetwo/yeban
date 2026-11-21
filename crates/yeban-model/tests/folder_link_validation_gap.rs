//! `TrackV3::folder_id` 的**链接形状**回归（T-FIX-52 后：自指与环已关，主总线指向仍按现状）。
//!
//! ## 这片在数什么（口径先说清）
//!
//! 单位＝**次**：数 `YebanProjectV1::validate()` 对一份**只有 `folder_id` 形状非法**的
//! 工程返回的是 `Ok` 还是 `Err`。`ADR-0004` 的 **Q13** 点名了三种形状：自指
//! （`folder_id == track.id`）、环（a→b→a）、指向主总线。
//!
//! T-FIX-52 的任务口径是**只补环检**：`folder_id` 是单出边父子关系，成环必须
//! 响亮拒绝并给出闭合路径。因此本文件现在钉住：
//!
//! - 自指 ⇒ `ModelError::FolderCycle`，路径 `[a, a]`；
//! - 两节点环 ⇒ `ModelError::FolderCycle`，路径 `[a, b, a]`；
//! - 三节点深环 ⇒ `ModelError::FolderCycle`，路径 `[a, b, c, a]`；
//! - 指向主总线 ⇒ **仍按现状通过**（不是环；本单⛔ 不扩权改这条产品语义）。
//!
//! ## 为什么这不是重复既有判据
//!
//! crate 内已有一条引用形态普查判据
//! `every_reference_kind_is_either_validated_or_explicitly_existence_checked_elsewhere`，
//! 它钉的是 `folder_id` → **不存在**的 id ⇒ `TrackNotFound`。
//! 本片补的是**它没有覆盖的另一半**：指向**存在**但成环的轨。
//!
//! ## 夹具读数
//!
//! `filled_project()` 共 **4** 条轨，其中 `Master` **1** 条、非主总线 **3** 条 ⇒ 足够构造
//! 自指、二元环与三元深环。所有路径断言都用规范 ULID 文本逐字比较。

use std::str::FromStr;

use yeban_model::samples::filled_project;
use yeban_model::{EntityId, ModelError, TrackKind, YebanProjectV1};

/// 构造确定性的规范 ULID 文本（与 crate 内夹具同一口径，见 `tests/placement_loop.rs`）。
fn ghost_id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

/// 夹具里**非主总线**的音轨 id，按 `BTreeMap` 的确定性顺序。
fn non_master_ids(doc: &YebanProjectV1) -> Vec<EntityId> {
    doc.tracks
        .iter()
        .filter(|(_, track)| track.kind != TrackKind::Master)
        .map(|(key, _)| *key)
        .collect()
}

/// 断言 `validate()` 报 folder 环，且路径逐项等于期望、报文含每个节点身份。
fn assert_folder_cycle(doc: &YebanProjectV1, expected: &[EntityId]) {
    let error = doc.validate().expect_err("folder 父链成环必须被拒绝");
    let ModelError::FolderCycle { path } = &error else {
        panic!("拒绝理由必须是 FolderCycle，实测为 {error:?}");
    };
    assert_eq!(path.as_slice(), expected, "闭合路径不符: {path:?}");
    assert_eq!(path.first(), path.last(), "路径必须首尾闭合: {path:?}");
    let message = error.to_string();
    for id in expected {
        assert!(
            message.contains(&id.to_canonical_string()),
            "错误报文必须点名环上节点 {id}: {message}"
        );
    }
}

/// **非真空下界**：夹具必须有 **≥ 3** 条非主总线轨，否则深环判据是空跑。
#[test]
fn the_fixture_has_enough_non_master_tracks_for_the_shape_probes() {
    let doc = filled_project();
    let ids = non_master_ids(&doc);
    assert!(
        ids.len() >= 3,
        "夹具只有 {} 条非主总线轨 ⇒ 深环判据无法构造，先修夹具",
        ids.len()
    );
    // 证明"非主总线"这个过滤**是活的**：现读夹具 = **4** 条轨、其中**恰 1** 条 `Master`。
    assert_eq!(
        ids.len() + 1,
        doc.tracks.len(),
        "非主总线集 + 主总线必须恰好等于全部轨道数，否则本文件的过滤口径不可信"
    );
}

/// **阳性对照**：存在性检查**真的在跑**——指向不存在的轨 ⇒ 红。
#[test]
fn a_folder_link_to_a_missing_track_is_still_rejected() {
    let mut doc = filled_project();
    let ghost = ghost_id(9_999);
    let target = *non_master_ids(&doc).first().expect("夹具必须有非主总线轨");
    doc.tracks
        .get_mut(&target)
        .expect("夹具里取到自己刚读到的 id")
        .folder_id = Some(ghost);
    let err = doc
        .validate()
        .expect_err("指向不存在的 folder_id 必须被拒（存在性检查在跑）");
    assert!(
        matches!(err, ModelError::TrackNotFound { id } if id == ghost),
        "拒绝理由必须是『轨道不存在』，实测为 {err:?}"
    );
}

/// **对照臂**：同一夹具、⛔ 不动任何 `folder_id` ⇒ 必须 `Ok`。
/// 合法父子森林逐位不变，是下面三条拒绝判据的前提。
#[test]
fn the_untouched_baseline_fixture_validates() {
    filled_project()
        .validate()
        .expect("基准夹具必须合法，否则后面的拒绝判据没有意义");
}

/// **自指**（`folder_id == track.id`）：拒绝，路径 `[me, me]`。
#[test]
fn validate_rejects_a_self_referential_folder_link() {
    let mut doc = filled_project();
    let me = *non_master_ids(&doc).first().expect("夹具必须有非主总线轨");
    let track = doc.tracks.get_mut(&me).expect("夹具里取到自己刚读到的 id");
    track.folder_id = Some(track.id);
    assert_folder_cycle(&doc, &[me, me]);
}

/// **两节点环**（a→b→a）：拒绝，路径 `[a, b, a]`。
#[test]
fn validate_rejects_a_two_node_folder_cycle() {
    let mut doc = filled_project();
    let ids = non_master_ids(&doc);
    let (a, b) = (
        *ids.first().expect("环判据需要两条轨"),
        *ids.get(1).expect("同上"),
    );
    doc.tracks.get_mut(&a).expect("a 在夹具里").folder_id = Some(b);
    doc.tracks.get_mut(&b).expect("b 在夹具里").folder_id = Some(a);
    assert_folder_cycle(&doc, &[a, b, a]);
}

/// **folder 深层环**（a→b→c→a）：拒绝，路径含全部三个节点。
#[test]
fn validate_rejects_a_deep_folder_cycle() {
    let mut doc = filled_project();
    let ids = non_master_ids(&doc);
    let (a, b, c) = (
        *ids.first().expect("深环判据需要三条轨"),
        *ids.get(1).expect("同上"),
        *ids.get(2).expect("同上"),
    );
    doc.tracks.get_mut(&a).expect("a 在夹具里").folder_id = Some(b);
    doc.tracks.get_mut(&b).expect("b 在夹具里").folder_id = Some(c);
    doc.tracks.get_mut(&c).expect("c 在夹具里").folder_id = Some(a);
    assert_folder_cycle(&doc, &[a, b, c, a]);
}

/// **指向主总线**仍按现状通过——主总线**也在 `tracks` 里**，存在性检查放行它。
/// T-FIX-52 只补环检；这不是环，⛔ 不在本单内改产品语义。
#[test]
fn validate_still_accepts_a_folder_link_pointing_at_the_master_bus() {
    let mut doc = filled_project();
    let master = doc.master_bus_track_id;
    let child = *non_master_ids(&doc).first().expect("夹具必须有非主总线轨");
    doc.tracks
        .get_mut(&child)
        .expect("夹具里取到自己刚读到的 id")
        .folder_id = Some(master);
    doc.validate()
        .expect("指向主总线不是环；T-FIX-52 只补环检，不扩权改这条语义");
}
