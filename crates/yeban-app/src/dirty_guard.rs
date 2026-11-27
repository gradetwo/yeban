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
/// `.0` 是保存那一刻的**头指针快照**；`.1` 是 T-FIX-736 新增的**文档指纹**
/// （`UndoPort::fingerprint()`，模型容器字节的 SHA-256）。它不是第二份工程状态，
/// 只是保存那一刻的两项读数。构造时以空工程为基线，与「刚启动 / 新工程未保存」一致。
///
/// ## 为什么需要指纹（T-FIX-736）
///
/// 撤销只前移 `cursor.skip`、**不动分支头**：保存后撤销到更早的版本，头指针仍等于
/// 保存点 ⇒ 旧的单看 head 判定会说「干净」⇒ 关窗丢改动（假阴性）；反之保存后新增
/// 提交再撤销回保存点，头指针又不等于保存点 ⇒ 误报脏（假阳性）。指纹（工程字节的
/// 摘要）才是「文档此刻是否与磁盘版一致」的真源，能同时堵住这两头。
/// ⚠ head 比对仍保留（`.0`），供未装指纹层的旧判据装配逐位兼容。
#[derive(Debug, Default)]
pub struct SavedHead(
    RefCell<Option<EntityId>>,
    RefCell<Option<String>>, // T-FIX-736：保存那一刻的文档指纹
);

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
    /// ⚠ 同时清空指纹快照，退化为纯 head 判定；装配需在随后调
    /// [`SavedHead::mark_saved_fingerprint`] 补上指纹（两步都在 `Saved` 分支）。
    pub fn mark_saved(&self, head: Option<EntityId>) {
        *self.0.borrow_mut() = head;
        *self.1.borrow_mut() = None;
    }

    /// 记下「刚刚保存成功」这一刻的文档指纹（T-FIX-736）。
    ///
    /// 与 [`SavedHead::mark_saved`] 配套，必须同在 `Saved` 分支调用；
    /// 传 `None`（拿不到指纹）时退化为纯 head 判定，不静默当成「一致」。
    pub fn mark_saved_fingerprint(&self, fingerprint: Option<String>) {
        *self.1.borrow_mut() = fingerprint;
    }

    /// 当前保存点（用于回执 / 判据读数）。
    #[must_use]
    pub fn saved(&self) -> Option<EntityId> {
        *self.0.borrow()
    }

    /// 当前保存点的文档指纹（T-FIX-736，判据读数）。
    #[must_use]
    pub fn saved_fingerprint(&self) -> Option<String> {
        self.1.borrow().clone()
    }

    /// 当前头相对保存点是否「脏」（有未保存改动）。
    ///
    /// 纯函数式判定：两个头指针相等（含同为 `None`）⇒ 不脏；不等 ⇒ 脏。
    /// ⚠ 只看 head；T-FIX-736 的指纹判定走 [`SavedHead::is_dirty_with_fingerprint`]。
    #[must_use]
    pub fn is_dirty(&self, current: Option<EntityId>) -> bool {
        current != *self.0.borrow()
    }

    /// T-FIX-736：带文档指纹的脏判定 —— 撤销/重做场景的真源。
    ///
    /// - 两侧指纹**都在**（保存时记了、此刻也拿得到）⇒ 只比指纹：相等即干净，
    ///   不等即脏（同时堵住「撤销后误判干净」与「撤销回保存点误报脏」）；
    /// - 任一指纹缺失 ⇒ 回退到 head 比对，与改动前逐位一致（旧判据装配零干扰）。
    #[must_use]
    pub fn is_dirty_with_fingerprint(
        &self,
        current: Option<EntityId>,
        current_fingerprint: Option<String>,
    ) -> bool {
        let saved_fp = self.1.borrow();
        if let (Some(saved), Some(current)) = (saved_fp.as_ref(), current_fingerprint) {
            return saved != &current;
        }
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
    // T-FIX-736：当前工程的指纹快照（判定前由装配层现读现写）。
    static ACTIVE_CURRENT_FINGERPRINT: RefCell<Option<String>> = const { RefCell::new(None) };
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

/// 保存成功后记账当前文档指纹（T-FIX-736，与 [`mark_saved`] 同在 `Saved` 分支）。
/// 未安装时是 no-op。
pub fn mark_saved_fingerprint(fingerprint: Option<String>) {
    ACTIVE_SAVED_HEAD.with(|slot| {
        if let Some(head) = slot.borrow().as_ref() {
            head.mark_saved_fingerprint(fingerprint);
        }
    });
}

/// 装配层在判定前现读现写「当前工程指纹」（T-FIX-736）。
/// `None` 表示拿不到指纹 ⇒ 判定退化为纯 head 比对（逐位兼容）。
pub fn set_current_fingerprint(fingerprint: Option<String>) {
    ACTIVE_CURRENT_FINGERPRINT.with(|slot| *slot.borrow_mut() = fingerprint);
}

/// 当前头相对保存点是否脏（app-quit / 关窗判定用）。未安装 ⇒ false（不拦）。
///
/// T-FIX-736：从线程槽现读装配层刚写入的当前指纹（[`set_current_fingerprint`]），
/// 交给 [`SavedHead::is_dirty_with_fingerprint`] —— 两侧指纹都在时比指纹（堵住
/// 「撤销后误判干净」与「撤销回保存点误报脏」），否则回退纯 head 比对。
/// ⚠ 签名与既有判据逐字不变，app-quit / 关窗两处判定点 ⛔ 不必改。
#[must_use]
pub fn current_is_dirty(current: Option<EntityId>) -> bool {
    let current_fp = ACTIVE_CURRENT_FINGERPRINT.with(|slot| slot.borrow().clone());
    ACTIVE_SAVED_HEAD.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|head| head.is_dirty_with_fingerprint(current, current_fp))
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

    // ---- T-FIX-736：文档指纹判定（撤销只动游标、不动分支头的两难） ----

    /// 保存点 = (同一 head, 指纹 F_saved)。current 指纹 ≠ F_saved ⇒ 脏，即使 head 相同。
    #[test]
    fn undo_to_an_earlier_document_is_dirty_even_when_head_matches() {
        let saved = SavedHead::new();
        let h2 = head();
        saved.mark_saved(Some(h2));
        saved.mark_saved_fingerprint(Some("F_saved".to_owned()));
        // 撤销到更早版本：head 仍是 H2（分支头不动），但文档指纹变了。
        assert_eq!(Some(h2), saved.saved(), "夹具 head 必须与保存点相同");
        assert!(
            saved.is_dirty_with_fingerprint(Some(h2), Some("F_earlier".to_owned())),
            "保存后撤销 ⇒ 文档不同于磁盘版 ⇒ 必须脏（旧逻辑只看 head 会漏）"
        );
        assert!(
            !saved.is_dirty_with_fingerprint(Some(h2), Some("F_saved".to_owned())),
            "指纹回到保存点 ⇒ 干净"
        );
    }

    /// 保存后新增提交 H3 再撤销回保存点：head 变（H3≠H2）但指纹回到 F_save ⇒ 干净。
    #[test]
    fn undo_back_to_saved_fingerprint_is_clean_even_when_head_differs() {
        let saved = SavedHead::new();
        let h2 = head();
        let h3 = head();
        assert_ne!(h2, h3);
        saved.mark_saved(Some(h2));
        saved.mark_saved_fingerprint(Some("F_saved".to_owned()));
        assert!(
            !saved.is_dirty_with_fingerprint(Some(h3), Some("F_saved".to_owned())),
            "撤销回保存点的文档指纹 ⇒ 干净（旧逻辑只看 head 会误报脏）"
        );
        assert!(
            saved.is_dirty(Some(h3)),
            "纯 head 判定仍按旧口径（误报脏），指纹判定才修正"
        );
    }

    /// 任一指纹缺失 ⇒ 回退纯 head 判定（逐位兼容旧判据装配）。
    #[test]
    fn missing_fingerprint_falls_back_to_head_comparison() {
        let saved = SavedHead::new();
        let h1 = head();
        saved.mark_saved(Some(h1)); // 不记指纹 ⇒ saved_fp = None
        assert_eq!(saved.saved_fingerprint(), None);
        let h2 = head();
        assert!(saved.is_dirty_with_fingerprint(Some(h2), Some("whatever".to_owned())));
        assert!(!saved.is_dirty_with_fingerprint(Some(h1), Some("whatever".to_owned())));
        // 保存侧有指纹、当前侧缺失 ⇒ 也回退 head。
        saved.mark_saved_fingerprint(Some("F".to_owned()));
        assert!(!saved.is_dirty_with_fingerprint(Some(h1), None));
    }
}
