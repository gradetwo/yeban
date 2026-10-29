//! `TrackV3::folder_id` 的**链接形状**校验缺口登记（⛔ 本片只登记，不补校验）。
//!
//! ## 这片在数什么（口径先说清）
//!
//! 单位＝**次**：数 `YebanProjectV1::validate()` 对一份**只有 `folder_id` 形状非法**的
//! 工程返回的是 `Ok` 还是 `Err`。三种非法形状由 `ADR-0004` 点名——它的 **Q13**
//! （`docs/adr/ADR-0004-...md:407-428`）原文：「**自指**（`folder_id == track.id`）、
//! **环**（a→b→a）、**指向主总线**今天**全部通过**」，并另把"工具面没有能设置 `folder_id`
//! 的形态"登记为诚实边界（`ADR-0023:79-82`）。
//!
//! ⚠ **为什么本片只登记、不修**：`ADR-0004` 的**状态是 `Proposed（待人类裁决）`**，
//! Q13 给了三个选项（A 拒绝／B 访问集上限／C 不校验）并**建议 A+B 合并**，但那条建议
//! **尚未被负责人批准**。⛔ 按 `AGENTS.md` §5 第 7 条，Agent 不得替人类落这一签，
//! 所以补校验是**裁决之后**的片；本片只把"今天是什么样"钉成机械事实。
//!
//! 现读的代码事实：`folder_id` 只查**存在性**
//! （`crates/yeban-model/src/project.rs:1883` 的
//! `if let Some(folder_id) = track.folder_id && !self.tracks.contains_key(&folder_id)`）
//! ⇒ 三种形状指向的都是**存在**的轨（主总线本身也在 `tracks` 里），于是全部放行。
//! ⚠ 顺带登记一处**文档行号漂移**：`ADR-0004` 两处写 `project.rs:1673-1676`／`project.rs:1143`，
//! 现读分别是 **`:1883`** 与 **`:1235`**（同一文件在两周内长大了）。
//!
//! ## 为什么这不是重复既有判据
//!
//! crate 内已有一条引用形态普查判据
//! `project.rs` 的 `every_reference_kind_is_either_validated_or_explicitly_existence_checked_elsewhere`，
//! 它钉的是 `folder_id` → **不存在**的 id ⇒ `TrackNotFound`（"指向不存在的轨"这一形态）。
//! 本片补的是**它没有覆盖的另一半**：指向**存在**但形状非法的轨。⛔ 两片不重叠。
//!
//! ## ⛔ 这片不回答的事
//!
//! "该不该补校验"、"补了以后是**响亮拒绝**还是**降级为顶层**（把 `folder_id` 置 `None`）"
//! 是产品语义裁决（且新增 `ModelError` 变体会牵动契约），⛔ 由人类裁、留痕 `docs/adr/`。
//! 所以每条判据钉的是**今天的现状**：一旦有人补上校验，这些判据会**变红**——那是
//! 要求他**显式改这里**、并把裁决编号写回来的门，⛔ 不是要维护的"通过"。
//!
//! ## 本机证据（做完已还原，⛔ 不在树里）
//!
//! 向 `project.rs` 的 `validate()` **注入**三种拒绝（自指／`folder_id == master_bus_track_id`／
//! 带访问集的环walk）后重跑本文件：`3 passed; 3 failed`，红的**恰好**是三条"现状登记"判据，
//! 而对照臂、阳性对照、非真空下界三条**仍绿** ⇒ 证明这三条钉的是**活**代码路径，
//! 不是碰巧返回 `Ok`。还原 `project.rs`（`git checkout --`）后 **6 passed**。
//! 夹具读数（单位：条）：`filled_project()` 共 **4** 条轨，其中 `Master` **1** 条、非主总线 **3** 条。

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

/// **非真空下界**：夹具必须有 **≥ 2** 条非主总线轨，否则下面的"环"判据是空跑。
#[test]
fn the_fixture_has_enough_non_master_tracks_for_the_shape_probes() {
    let doc = filled_project();
    let ids = non_master_ids(&doc);
    assert!(
        ids.len() >= 2,
        "夹具只有 {} 条非主总线轨 ⇒ 环／自指两条判据无法构造，先修夹具",
        ids.len()
    );
    // 证明"非主总线"这个过滤**是活的**：现读夹具 = **4** 条轨、其中**恰 1** 条 `Master`。
    assert_eq!(
        ids.len() + 1,
        doc.tracks.len(),
        "非主总线集 + 主总线必须恰好等于全部轨道数，否则本文件的过滤口径不可信"
    );
}

/// **阳性对照（这条是其余三条的前提）**：存在性检查**真的在跑**——指向不存在的轨 ⇒ 红。
/// 没有这条，下面的三个 `Ok` 也可能只是"整个 `folder_id` 分支是死代码"。
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
        .expect_err("指向不存在的 folder_id 必须被拒（现状：存在性检查在跑）");
    assert!(
        matches!(err, ModelError::TrackNotFound { id } if id == ghost),
        "拒绝理由必须是『轨道不存在』，实测为 {err:?}"
    );
}

/// **对照臂**：同一夹具、⛔ 不动任何 `folder_id` ⇒ 必须 `Ok`。
/// 它证明后面三条的 `Ok` 不是"夹具本身已经坏到 validate 提前返回"，而是**形状没被管**。
#[test]
fn the_untouched_baseline_fixture_validates() {
    filled_project()
        .validate()
        .expect("基准夹具必须合法，否则后面三条的 Ok 没有意义");
}

/// **缺口一：自指**（`folder_id == track.id`）今天通过。
#[test]
fn validate_accepts_a_self_referential_folder_link_today() {
    let mut doc = filled_project();
    let me = *non_master_ids(&doc).first().expect("夹具必须有非主总线轨");
    let track = doc.tracks.get_mut(&me).expect("夹具里取到自己刚读到的 id");
    track.folder_id = Some(track.id);
    doc.validate().expect(
        "⚠ 现状登记：自指的 folder_id 今天通过校验（ADR-0004 的 Q13 点名的陷阱）；\
                补校验 ⇒ 本条按设计变红，请连同裁决编号一起改这里",
    );
}

/// **缺口二：两节点环**（a→b→a）今天通过。
#[test]
fn validate_accepts_a_two_node_folder_cycle_today() {
    let mut doc = filled_project();
    let ids = non_master_ids(&doc);
    let (a, b) = (
        *ids.first().expect("环判据需要两条轨"),
        *ids.get(1).expect("同上"),
    );
    doc.tracks.get_mut(&a).expect("a 在夹具里").folder_id = Some(b);
    doc.tracks.get_mut(&b).expect("b 在夹具里").folder_id = Some(a);
    doc.validate()
        .expect("⚠ 现状登记：a→b→a 的 folder 环今天通过校验（同一条 ADR 点名）");
}

/// **缺口三：指向主总线**今天通过——主总线**也在 `tracks` 里**，所以存在性检查放行它。
#[test]
fn validate_accepts_a_folder_link_pointing_at_the_master_bus_today() {
    let mut doc = filled_project();
    let master = doc.master_bus_track_id;
    let child = *non_master_ids(&doc).first().expect("夹具必须有非主总线轨");
    doc.tracks
        .get_mut(&child)
        .expect("夹具里取到自己刚读到的 id")
        .folder_id = Some(master);
    doc.validate().expect(
        "⚠ 现状登记：把普通轨挂到主总线下今天通过校验——\
                          主总线自身就在 tracks 里，存在性检查管不到它",
    );
}
