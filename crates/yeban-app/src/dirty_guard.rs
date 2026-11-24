//! 工程「脏标记」的**纯逻辑层**（T-FIX-208，数据丢失级缺陷）。
//!
//! # 它回答的问题
//!
//! 「退出 / 关窗」之前要不要拦一下问用户「有未保存改动，要不要先存」？
//! 答案不能靠「界面上看起来改过」这种猜测 —— 那正是本仓反复警告的**假读数**。
//! 唯一可靠的判据是**提交头指针**：
//!
//! > 保存成功的那一刻，记下当前活跃分支头 `saved_head`；
//! > 此后 `display().head != saved_head` 就意味着「在上次保存之后又提交了改动」⇒ 脏。
//!
//! # 为什么不新造「第二个计数器 / 布尔」
//!
//! `UndoPort` 的每次编辑提交（`Op::AddNote` 等）都会在 `CommitGraph` 上产生新 commit，
//! 活跃分支头随之前移 —— 这是**唯一的真源**。任何"我在 UI 侧另设一个 dirty=true"的
//! 做法都会在撤销 / 重做 / 时光机跳转后与真源漂移（撤销回到保存点明明应该不脏）。
//! 用 head 指针比对则天然正确处理这些情形：撤销回保存点 ⇒ head 相同 ⇒ 不脏。
//!
//! # 无工程 / 新工程的空基线
//!
//! `None` 既表示「没有活跃工程」，也表示「新工程还没有任何提交」。两种情况都以
//! 空工程为基线：`saved_head = None` 且 `current = None` ⇒ **不脏**（没东西可丢）；
//! 一旦产生第一个 commit（`current = Some(_)`）就变脏 ⇒ 退出会拦。
//!
//! # 零 Slint 依赖
//!
//! 本模块只依赖 `std` + `yeban-model` 的 `EntityId`，因此判据能在本机真跑
//! （无需桌面会话）；Slint 属性 / 对话框接线全留在 `host` / `main` 两处薄层。
use std::cell::RefCell;
use std::rc::Rc;

use yeban_model::EntityId;

/// 一个会话的「上次保存点」。
///
/// 仅持有 `RefCell<Option<EntityId>>`：它不是第二份工程状态，只是**保存那一刻**的
/// 头指针快照。构造时以空工程为基线（`None`），与「刚启动 / 新工程未保存」一致。
#[derive(Debug, Default)]
pub struct SavedHead(RefCell<Option<EntityId>>);

impl SavedHead {
    /// 以空工程为基线新建一个保存点（无工程 / 新工程未保存）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 记下「刚刚保存成功」这一刻的头指针。
    ///
    /// 只有 `dispatch_save` 返回 `Saved` 才应调这里（失败 / 取消都不动保存点，
    /// 否则「存失败却被当成已存」会把未保存改动静默丢掉）。
    pub fn mark_saved(&self, head: Option<EntityId>) {
        *self.0.borrow_mut() = head;
    }

    /// 当前保存点（用于回执 / 判据读数）。
    #[must_use]
    pub fn saved(&self) -> Option<EntityId> {
        *self.0.borrow()
    }

    /// 当前头相对保存点是否「脏」（有未保存改动）。
    ///
    /// 纯函数式判定：两个头指针相等（含同为 `None`）⇒ 不脏；不等 ⇒ 脏。
    #[must_use]
    pub fn is_dirty(&self, current: Option<EntityId>) -> bool {
        current != *self.0.borrow()
    }
}

/// 无副作用的脏判定（给判据 / 其它装配直接复用，不要求持有 `SavedHead`）。
///
/// 语义与 [`SavedHead::is_dirty`] 完全一致：头指针不等即脏，同为 `None` 即不脏。
#[must_use]
pub fn head_is_dirty(current: Option<&EntityId>, saved: Option<&EntityId>) -> bool {
    current != saved
}

// ---- 当前 UI 线程的保存点槽 ------------------------------------------------
//
// 为什么是 thread_local：Slint 的所有 UI 回调（`on_command_execute` / 保存 /
// 三个三选 callback / 窗口 close_requested）都在**同一个 UI 线程**上执行，
// 因此一个线程单例就能让这几处共享同一个保存点，⛔ 不必给
// `wire_aether_actions`（20+ 测试调用点）/ `wire_save_with_engine` 改签名。
//
// 未安装时（判据装配 / 无头测试不调 `wire_quit_guard`）槽为 `None` ⇒
// `mark_saved` 是 no-op、`current_is_dirty` 恒 false ⇒ 退出行为与改动前逐位一致，
// 既有判据不受影响。这是"新装配才启用、老装配零干扰"的降级纪律。
thread_local! {
    static ACTIVE_SAVED_HEAD: RefCell<Option<Rc<SavedHead>>> = const { RefCell::new(None) };
}

/// 在 GUI 装配时安装本线程的保存点（只应由 `host::wire_quit_guard` 调用一次）。
pub fn install(head: Rc<SavedHead>) {
    ACTIVE_SAVED_HEAD.with(|slot| *slot.borrow_mut() = Some(head));
}

/// 保存成功后记账当前头指针（只应由保存回调在 `Saved` 分支调用）。
///
/// 失败 / 取消**不得**调这里：否则「存失败却被当成已存」会把未保存改动静默丢掉。
/// 未安装时是 no-op（判据装配不适用）。
pub fn mark_saved(current: Option<EntityId>) {
    ACTIVE_SAVED_HEAD.with(|slot| {
        if let Some(head) = slot.borrow().as_ref() {
            head.mark_saved(current);
        }
    });
}

/// 当前头相对保存点是否脏（app-quit / 关窗判定用）。未安装 ⇒ false（不拦）。
#[must_use]
pub fn current_is_dirty(current: Option<EntityId>) -> bool {
    ACTIVE_SAVED_HEAD.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|head| head.is_dirty(current))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 每个测试里 h1 / h2 各生成一次并复用：需要「同一头相等、不同头不等」，
    // `EntityId::new()` 每次给一个新身份，正合用。
    fn head() -> EntityId {
        EntityId::new()
    }

    #[test]
    fn empty_project_is_not_dirty() {
        let saved = SavedHead::new();
        assert!(!saved.is_dirty(None), "无工程 / 新工程未改不应拦退出");
        assert_eq!(saved.saved(), None);
    }

    #[test]
    fn first_commit_makes_it_dirty() {
        let saved = SavedHead::new();
        assert!(saved.is_dirty(Some(head())), "新增第一个提交后应脏");
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let saved = SavedHead::new();
        let h = head();
        assert!(saved.is_dirty(Some(h)));
        saved.mark_saved(Some(h));
        assert!(!saved.is_dirty(Some(h)), "保存后同一头应不脏");
        assert_eq!(saved.saved(), Some(h));
    }

    #[test]
    fn subsequent_edit_renews_dirty() {
        let saved = SavedHead::new();
        let h1 = head();
        saved.mark_saved(Some(h1));
        let h2 = head();
        assert_ne!(h1, h2, "夹具必须给出两个不同的头");
        assert!(saved.is_dirty(Some(h2)), "保存后又提交 ⇒ 应重新变脏");
    }

    #[test]
    fn undo_back_to_saved_point_is_clean() {
        let saved = SavedHead::new();
        let h1 = head();
        let h2 = head();
        saved.mark_saved(Some(h1));
        // 提交 h2 后脏；撤销回 h1 后头指针回到保存点 ⇒ 不脏（不是另设的布尔能表达的）。
        assert!(saved.is_dirty(Some(h2)));
        assert!(!saved.is_dirty(Some(h1)), "撤销回保存点应不脏");
    }

    #[test]
    fn failed_save_does_not_move_the_point() {
        let saved = SavedHead::new();
        let h1 = head();
        saved.mark_saved(Some(h1));
        // 失败保存「不动保存点」由调用纪律保证（只有 Saved 分支 mark_saved）；
        // 这里钉住：只要没调 mark_saved，头变了依然脏。
        let h2 = head();
        assert!(saved.is_dirty(Some(h2)));
        assert_eq!(saved.saved(), Some(h1));
    }

    #[test]
    fn pure_helper_matches_struct() {
        let saved = SavedHead::new();
        let h1 = head();
        saved.mark_saved(Some(h1));
        assert_eq!(
            saved.is_dirty(Some(h1)),
            head_is_dirty(Some(&h1), saved.saved().as_ref())
        );
        assert!(!head_is_dirty(None, None));
        assert!(head_is_dirty(Some(&h1), None));
    }
}
