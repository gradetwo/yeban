//! ⭐ T-FIX-736：脏标记不能只看分支头 —— 撤销只前移 `cursor.skip`、**不动 head**，
//! 于是「保存后撤销」会被误判为干净（关窗丢改动，假阴性）、「保存后新增再撤销回
//! 保存点」会被误报脏（假阳性）。修法：保存点与判定同时纳入**文档指纹**
//! （`UndoPort::fingerprint()` = 模型容器字节的 SHA-256）。
//!
//! ## 判据接真实 UndoPort（⛔ 不是两个假 EntityId）
//!
//! 每个场景都：`UndoSession::open` → `UndoPort::new` → `commit_ops` 真提交 →
//! `perform(UiAction::Undo)` 真撤销 → 读 `port.fingerprint()` / `port.display().head`。
//! 然后把保存点/当前读数喂给 `dirty_guard` 的真实 API，断言 `current_is_dirty()`。
//!
//! ## 三个场景（派单验收逐条）
//!
//! | # | 序列 | 期望 |
//! | :-- | :--- | :--- |
//! | 1 | 保存 → 编辑 → 撤销回保存点 | 干净 |
//! | 2 | 保存 → 撤销 | 脏 |
//! | 3 | 保存 → 新增 → 撤销回保存点 | 干净 |
//!
//! ## 反向腿
//!
//! 把 `dirty_guard::current_is_dirty` 改回「只比 head」（忽略指纹）⇒ 场景 2 红
//! （撤销后 head 没变却判干净）；场景 3 也会红（head 变了却判脏）。

use std::rc::Rc;

use yeban_app::dirty_guard;
use yeban_app::undo::{UiAction, UndoPort, UndoSession, undo_session};
use yeban_model::Op;
use yeban_model::samples::filled_project;

const NOW: u64 = 1_760_000_000_000;

/// 造一个真实 UndoPort（与 `undo.rs` 内部判据同款夹具）。
fn port() -> UndoPort {
    let session =
        UndoSession::open("<T-FIX-736>", "yeban-app", filled_project(), NOW).expect("打开撤销会话");
    UndoPort::new(session)
}

/// 把夹具音符力度改成 `vel` 并提交一条 op —— 每次不同 `vel` 产生**不同工程态**。
///
/// ⚠ 不能复用 `wiring_fixture().op()`：它的 old/new 是固定的，重复提交幂等 ⇒
/// 撤销后指纹不变，场景就失去意义（本判据第一次写错的地方）。
fn commit_velocity(port: &UndoPort, vel: u8, seq: u64) {
    let fixture = undo_session::wiring_fixture(&port.project()).expect("夹具");
    let current = port
        .project()
        .clip_pool
        .get(&fixture.clip_id)
        .and_then(|entry| entry.content.notes())
        .and_then(|notes| notes.get(&fixture.note_id))
        .expect("音符")
        .velocity;
    let op = Op::ModifyNoteVelocity {
        track_id: fixture.track_id,
        clip_id: fixture.clip_id,
        note_id: fixture.note_id,
        old_vel: current,
        new_vel: vel,
    };
    port.commit_ops(NOW + seq, "判据夹具", vec![op])
        .expect("提交");
}

/// 把「此刻已保存」写进 dirty_guard 的线程槽（head + 指纹），并设当前指纹=已保存态。
fn mark_saved_now(port: &UndoPort) {
    let head = port.display().head;
    let fp = port.fingerprint();
    assert!(fp.is_some(), "夹具必须能算出指纹");
    dirty_guard::mark_saved(head);
    dirty_guard::mark_saved_fingerprint(fp.clone());
    dirty_guard::set_current_fingerprint(fp);
}

/// 判定当前是否脏（先设当前指纹，再走真实判定入口）。
fn is_dirty_now(port: &UndoPort) -> bool {
    dirty_guard::set_current_fingerprint(port.fingerprint());
    dirty_guard::current_is_dirty(port.display().head)
}

/// 场景 1：保存 → 编辑 → 撤销回保存点 ⇒ 干净。
#[test]
fn save_edit_undo_back_to_saved_point_is_clean() {
    let saved = Rc::new(dirty_guard::SavedHead::new());
    dirty_guard::install(Rc::clone(&saved));

    let p = port();
    mark_saved_now(&p);
    let saved_fp = p.fingerprint().expect("保存指纹");

    // 编辑一次 → 脏 → 撤销回保存点。
    commit_velocity(&p, 90, 1);
    assert!(is_dirty_now(&p), "编辑后必须脏");
    p.perform(UiAction::Undo);
    assert_eq!(
        p.fingerprint().as_deref(),
        Some(saved_fp.as_str()),
        "撤销应回到保存指纹"
    );

    assert!(!is_dirty_now(&p), "撤销回保存点 ⇒ 干净");
}

/// 场景 2（核心假阴性）：保存 → 撤销 ⇒ 脏。
///
/// 「保存后撤销」在真实 UndoSession 里 head 仍是保存时的分支头（撤销只改 cursor.skip，
/// 见 undo_session.rs display()/head()）。纯 head 判定会说干净、关窗丢改动；
/// 指纹判定必须识别「文档已不同于磁盘版」⇒ 脏。
#[test]
fn save_then_undo_is_dirty_even_though_head_does_not_move() {
    let saved = Rc::new(dirty_guard::SavedHead::new());
    dirty_guard::install(Rc::clone(&saved));

    let p = port();
    // pristine --c1--> A(vel=100) --c2--> B(vel=110)；在 B 保存；撤销 1 步回 A(≠B)。
    commit_velocity(&p, 100, 1); // A
    commit_velocity(&p, 110, 2); // B
    mark_saved_now(&p); // 保存点 = B
    let head_b = p.display().head;
    let fp_b = p.fingerprint().expect("B 指纹");
    assert_eq!(fp_b.len(), 64, "指纹必须是 SHA-256 十六进制");

    p.perform(UiAction::Undo); // 回 A：head 仍 B（分支头不动），指纹 = A ≠ B
    assert_eq!(p.display().head, head_b, "夹具前提：撤销不动分支头");
    assert_ne!(
        p.fingerprint().as_deref(),
        Some(fp_b.as_str()),
        "文档指纹必须已变（A≠B）"
    );
    assert!(is_dirty_now(&p), "保存后撤销 ⇒ 必须脏（关窗要拦）");
}

/// 场景 3（假阳性）：保存 → 新增提交 → 撤销回保存点 ⇒ 干净。
#[test]
fn save_then_add_then_undo_back_to_saved_fingerprint_is_clean() {
    let saved = Rc::new(dirty_guard::SavedHead::new());
    dirty_guard::install(Rc::clone(&saved));

    let p = port();
    commit_velocity(&p, 100, 1); // 第一条
    commit_velocity(&p, 110, 2); // 保存点 H2（vel=110）
    mark_saved_now(&p);
    let saved_head = p.display().head;
    let saved_fp = p.fingerprint().expect("保存指纹");

    // 新增 H3（vel=120）→ head 变 → 脏。
    commit_velocity(&p, 120, 3);
    assert_ne!(p.display().head, saved_head, "新增提交后 head 已变");
    assert!(is_dirty_now(&p), "新增后必须脏");

    // 撤销回 H2 文档态（vel=110）→ head 仍 H3，但指纹回到 F_saved。
    p.perform(UiAction::Undo);
    let head_after_undo = p.display().head;
    assert_ne!(
        head_after_undo, saved_head,
        "夹具前提：撤销回退后 head 仍停在 H3"
    );
    assert_eq!(
        p.fingerprint().as_deref(),
        Some(saved_fp.as_str()),
        "撤销一步应回到保存点 H2 的文档指纹"
    );
    assert!(
        !is_dirty_now(&p),
        "撤销回保存点文档态 ⇒ 干净（纯 head 判定会误报脏）"
    );
}
