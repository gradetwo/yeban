//! 撤销的**界面侧**入口 —— ADR-0001 **D45** 的"人按 `Cmd+Z` 真的能撤销"这一半。
//!
//! ## 三条边界（本模块的设计就是这三条）
//!
//! 1. **同一份实现**：本模块用 `#[path]` 引入 `crates/yeban-mcp/src/undo_session.rs`
//!    —— 与 MCP 侧**字面上同一份源码**（`yeban-mcp` 的 `lib.rs` 也 `pub mod undo_session;`）。
//!    判据 `undo_session::tests::no_second_undo_implementation_exists_in_the_workspace`
//!    扫描两个 crate 的生产源码，任何自写的反向应用循环都变红。
//! 2. **零 Slint 依赖**：本模块只依赖 `std` + `yeban-model`（+ 共享实现）。
//!    理由不是洁癖：`yeban-app` 一旦编译就要拖 Slint/femtovg/winit 的重依赖，
//!    而"哪个动作改了工程、改了几步、前后指纹是多少"是**最值得在本机真跑**的东西。
//!    薄薄一层 Slint 接线因此全部留在 [`crate::host`] 的 `wire_undo` / `apply_undo`。
//! 3. **每个界面动作都进日志**：所有撤销类 UI 事件都只能走 [`UndoPort::perform`]，
//!    它把"动作 + 后果 + 游标前后 + 工程指纹前后"记成 [`UiActionRecord`]。
//!    判据因此可以断言"**点了它 ⇒ 工程真的回退了一版**"，而不是
//!    "控件树里有这个元素"（三方表的错位 5 明确警告过那种假证据）。
//!
//! ## 谁是权威：两个后端（`ROAD-M4-008` 选项 (a) 第二片）
//!
//! [`UndoPort`] 内部是一个 [`UndoBackend`]：
//!
//! - **挂载了进程内控制面** ⇒ [`UndoPort::from_authority`]：端口只握
//!   `ProjectAuthorityHandle`，读（投影 / 显示态 / 图谱 / 指纹）与写（撤销 / 重做 / 提交）
//!   全部落到控制面正在服务的**那一个** `Domain`（唯一可变权威）。这是**生产 GUI 的常态**
//!   （`src/main.rs` 的 `run_gui` 先挂控制面再建端口）。
//! - **没有控制面**（默认构建 / 运行期开关关着 / `.yeban.lock` 被别的形态持有）⇒
//!   [`UndoPort::new`]：端口持有自己的一份 `UndoSession`。那时进程里**不存在**第二个写者，
//!   因此它仍是唯一权威。
//!
//! 两个后端的**模型语义完全一致**（同一份 `undo_session.rs`）；差别只在"权威住在哪"。
//! 注意 `#[path]` 引入的是**另一个 crate 里的另一个类型**：`yeban_app::undo::UndoSession`
//! 与 `yeban_mcp::undo_session::UndoSession` 在类型系统里无法互换，所以权威后端不能
//! "共享同一个实例"，只能**委派**（这也是 [`HostAction`] 存在的原因）。
//!
//! ## 显示态来自模型读数
//!
//! 能不能撤销、还能撤几步、已撤几步、提交多少条 —— 全部由
//! [`undo_session::UndoDisplay`] 从 `CommitGraph` + `UndoCursor` 读出，
//! 界面**不自己算**（`views` 只是把读数写进 Slint 属性，见 `host::apply_undo`）。
//!
//! ## 会话运行态 [MODEL-ISO-001]
//!
//! [`UndoPort`] 里的"时光机弹窗是否打开"是**界面运行态**；游标是**会话运行态**。
//! 两者都不进 `.yeban`：判据
//! `crates/yeban-mcp/tests/undo_wiring.rs::the_cursor_never_reaches_the_project_container`
//! 用容器字节级证据钉住这一点。

#![allow(clippy::module_inception)]

#[path = "../../yeban-mcp/src/undo_session.rs"]
pub mod undo_session;

use std::cell::{Cell, RefCell};

use yeban_model::{CommitGraph, EntityId, Op, OpOrigin, YebanProjectV1};

// `ROAD-M4-008` 选项 (a) 第二片：挂载了控制面时，GUI 的写入口落到**那一个** `Domain` 上。
// 这两个类型只存在于非默认 feature `in-process-mcp` 下（默认构建里没有 `yeban-mcp`）。
#[cfg(feature = "in-process-mcp")]
use yeban_mcp::domain::error::Fault;
#[cfg(feature = "in-process-mcp")]
use yeban_mcp::domain::{HostAction, HostOutcome};

#[cfg(feature = "in-process-mcp")]
use crate::mcp_mount::ProjectAuthorityHandle;

pub use undo_session::{
    CommitRequest, UndoDisplay, UndoRefusal, UndoSession, UndoState, project_fingerprint,
};

use crate::input::{Action, InputContext, LogicalKey, Modifiers, PhysicalKey, Resolution};

/// 界面上的撤销类动作（**全部**撤销入口都归到这里）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    /// `[D56]` 人工导出诊断包（把调试信息与相关文件打包, 供复现排查）。
    ExportDiagnostics,
    /// `[UI-NOTE-003]` 切到某个卷帘工具（**视图状态**；由键盘路径派发，界面侧应用）。
    /// `N2` 裁决 (1) 之后：GUI 的逻辑键路径（`host::wire_keys`）经这里下发，
    /// 宿主随即把 `active-tool` 写到界面上（本端口碰不到 Slint）。
    SelectTool(crate::input::Tool),
    /// `Cmd+Z` / 时光机里的"撤销一步"按钮。
    Undo,
    /// `Cmd+Shift+Z`。
    Redo,
    /// 多步撤销（时光机点击某个节点）。
    UndoMany(usize),
    /// 打开 / 关闭 / 切换时光机弹窗（只改界面运行态，不动工程）。
    OpenUndoTree,
    /// 关闭时光机弹窗。
    CloseUndoTree,
    /// 切换时光机弹窗。
    ToggleUndoTree,
}

impl UiAction {
    /// 动作名（进动作日志；判据据它断言"点了哪一个"）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::UndoMany(_) => "undo-many",
            Self::OpenUndoTree => "open-undo-tree",
            Self::CloseUndoTree => "close-undo-tree",
            Self::ToggleUndoTree => "toggle-undo-tree",
            Self::ExportDiagnostics => "export-diagnostics",
            Self::SelectTool(_) => "select-tool",
        }
    }
}

/// 键盘策略表 → 界面动作的**唯一**映射。
///
/// `[UI-A11Y-001]` §7.1 的物理扫描码解析住在 [`crate::input`]（纯 Rust、22 条判据），
/// 这里只把解析结果接到撤销入口上。`main.rs` 的按键分发与它**共用**这一个函数，
/// 因此"`Cmd+Z` 解析对了但没人接"这类错位不可能再出现。
#[must_use]
pub const fn dispatch_key(action: Action) -> Option<UiAction> {
    match action {
        Action::Undo => Some(UiAction::Undo),
        Action::Redo => Some(UiAction::Redo),
        Action::OpenTimeMachine => Some(UiAction::OpenUndoTree),
        // `[UI-NOTE-003]` 工具选择走**同一**唯一下发点；它不改工程, 由宿主的界面侧应用。
        Action::SelectTool(tool) => Some(UiAction::SelectTool(tool)),
        _ => None,
    }
}

/// 键盘事件（**物理码入口**）→ 会话动作：**整条链的唯一落点**。
///
/// `context` 就是 `crate::input` 的策略表（物理扫描码绑定 + IME 合成态拦截 +
/// 焦点规则 `[UI-A11Y-001/002]`），因此本函数**不复制**任何键盘策略：
/// 它只把策略表判定出的 [`Action`] 交给 [`dispatch_key`]，再落到 [`UndoPort::perform`]。
///
/// 返回 `None` = 这个键与撤销无关（调用方应当放行给别的处理器）。
/// 注意"文本输入框聚焦 + `Cmd+Z`"按规范是 `PassThrough`（给文本框做文本撤销），
/// 因此那时这里同样返回 `None` —— 那是策略表说的，不是本函数猜的。
pub fn perform_key(
    port: &UndoPort,
    context: &InputContext,
    key: PhysicalKey,
    modifiers: Modifiers,
) -> Option<ActionOutcome> {
    perform_logical_key(port, context, key.logical(), modifiers)
}

/// 同 [`perform_key`]，但收的是 **GUI 路径的逻辑键**（`N2` 裁决 (1)）。
///
/// 两条入口共用同一张策略表（`InputContext::resolve` 就是
/// `resolve_logical(key.logical(), ..)`），因此这里再写一遍 `match` 只会制造第二个真相源。
/// 存在的理由：Slint 的 `KeyEvent` 没有物理码，GUI **只能**给出逻辑键 ——
/// 而"人按 `Cmd+Z` 真的能撤销"（ADR-0001 **D45**）必须落在**同一个** `UndoPort` 上。
pub fn perform_logical_key(
    port: &UndoPort,
    context: &InputContext,
    key: LogicalKey,
    modifiers: Modifiers,
) -> Option<ActionOutcome> {
    match context.resolve_logical(key, modifiers) {
        Resolution::Action(action) => dispatch_key(action).map(|ui_action| port.perform(ui_action)),
        Resolution::PassThrough | Resolution::ConsumedByIme => None,
    }
}

/// 一次动作的后果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionOutcome {
    /// 工程**真的**改动了若干步。
    Changed {
        /// 实际步数。
        steps: usize,
        /// 执行后已撤销的总步数。
        undone_total: usize,
        /// 被处理的 op 变体名。
        op_kinds: Vec<String>,
    },
    /// 被拒绝（工程与游标一位不动）。
    Refused {
        /// 人话原因（来自 [`UndoRefusal`]）。
        detail: String,
    },
    /// 只改了界面运行态（工程与游标都没动）。
    DisplayOnly,
}

impl ActionOutcome {
    /// `true` = 工程真的被改动了（判据的"真的回退了一版"就是这一条）。
    #[must_use]
    pub const fn changed(&self) -> bool {
        matches!(self, Self::Changed { .. })
    }
}

/// 一次界面动作的**记录**（动作日志的一行）。
///
/// 这是"接线"的可机械验证证据：判据不需要看控件树，只需要看这一行里
/// 指纹前后是否真的变了、游标是否真的前移了。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiActionRecord {
    /// 动作名（[`UiAction::name`]）。
    pub action: &'static str,
    /// 后果。
    pub outcome: ActionOutcome,
    /// 动作前的工程指纹（模型容器字节的 SHA-256）。
    pub fingerprint_before: Option<String>,
    /// 动作后的工程指纹。
    pub fingerprint_after: Option<String>,
    /// 动作前已撤销步数。
    pub undone_before: usize,
    /// 动作后已撤销步数。
    pub undone_after: usize,
    /// 动作后还能撤销几步（模型读数）。
    pub undoable_after: usize,
    /// 动作前提交数。
    pub commits_before: usize,
    /// 动作后提交数。
    pub commits_after: usize,
    /// 动作后时光机弹窗是否打开。
    pub undo_tree_open: bool,
}

impl UiActionRecord {
    /// `true` = 这一次动作**真的**改了工程（逐字节可验证）。
    #[must_use]
    pub fn changed_project(&self) -> bool {
        self.outcome.changed()
            && self.fingerprint_before.is_some()
            && self.fingerprint_before != self.fingerprint_after
    }
}

/// 撤销端口的**后端** —— 决定"这一份工程 / 图谱 / 游标的权威是谁"。
///
/// | 变体 | 什么时候 | 权威是谁 |
/// | :--- | :--- | :--- |
/// | [`Self::Local`] | 没挂载控制面（默认构建，或 `--features in-process-mcp` 但运行期开关关着 / 拿不到 `.yeban.lock`） | 本端口持有的那一个 `UndoSession` |
/// | [`Self::Authority`] | 挂载了进程内控制面（`src/main.rs` 的 `mount_in_process_mcp` 成功） | 控制面正在服务的那一个 `Domain`（`ROAD-M4-008` 选项 (a)） |
///
/// 为什么是枚举而不是 trait 对象：两个后端的操作集合完全一样，枚举让"此刻谁是权威"
/// 在装配点一眼可见，也不会为 `dyn` 引入额外的对象安全约束。
///
/// 注意 [`Self::Local`] 里的 `Box`：`UndoSession` 比句柄大得多，
/// 不装箱会撞上 `clippy::large_enum_variant`（`-D warnings` 下是硬错误）。
#[derive(Debug)]
enum UndoBackend {
    /// GUI 自己打开的会话（**没有**控制面时的唯一一份）。
    Local(Box<UndoSession>),
    /// 控制面正在服务的那一个 `Domain`（挂载了控制面时的**唯一可变权威**）。
    #[cfg(feature = "in-process-mcp")]
    Authority(ProjectAuthorityHandle),
}

/// 界面 → 会话的**唯一**入口。
///
/// 为什么用 `RefCell` / `Cell` 而不是 `&mut self`：Slint 的回调是 `Fn`
/// （`on_toggle_undo_tree(move || …)`），闭包里只能拿到 `Rc<UndoPort>`。
/// 内部可变性让"回调里调 `perform`"这一件事在类型上成立，而不必把整个端口
/// 变成 `Rc<RefCell<UndoPort>>`（那会让每次调用都可能撞上借用冲突）。
#[derive(Debug)]
pub struct UndoPort {
    session: RefCell<UndoBackend>,
    actions: RefCell<Vec<UiActionRecord>>,
    open: Cell<bool>,
}

impl UndoPort {
    /// 从一个已打开的会话构造端口（**没有**控制面时的形态）。
    #[must_use]
    pub fn new(session: UndoSession) -> Self {
        Self {
            session: RefCell::new(UndoBackend::Local(Box::new(session))),
            actions: RefCell::new(Vec::new()),
            open: Cell::new(false),
        }
    }

    /// 以**控制面正在服务的那一个 `Domain`** 为唯一可变权威构造端口
    /// （`ROAD-M4-008` 选项 (a) 第二片）。
    ///
    /// 这样构造出来的端口**不持有任何工程 / 图谱 / 游标副本**：`display` / `project` /
    /// `graph` / `fingerprint` 与三个写入口（`Undo` / `Redo` / `commit_ops`）全部委派给
    /// 同一个句柄，因此"人在界面上按 `Cmd+Z`"与"AI 发 `yeban_undo`"改的是**同一串字节**。
    /// 这也正是"GUI 不再持有唯一可变副本"的可机械验证形态：把句柄拿走，端口什么都读不到。
    #[cfg(feature = "in-process-mcp")]
    #[must_use]
    pub fn from_authority(authority: ProjectAuthorityHandle) -> Self {
        Self {
            session: RefCell::new(UndoBackend::Authority(authority)),
            actions: RefCell::new(Vec::new()),
            open: Cell::new(false),
        }
    }

    /// 显示态（**模型读数**：能不能撤 / 还能撤几步 / 已撤几步 / 提交数 / 分支）。
    #[must_use]
    pub fn display(&self) -> UndoDisplay {
        match &*self.session.borrow() {
            UndoBackend::Local(session) => session.display(),
            #[cfg(feature = "in-process-mcp")]
            UndoBackend::Authority(authority) => display_from_authority(authority.undo_display()),
        }
    }

    /// 权威工程的**一份快照**（界面重投影用）。
    ///
    /// 权威会话此刻没有活跃工程时返回 `None`（控制面可以关掉工程）。
    #[must_use]
    pub fn try_project(&self) -> Option<YebanProjectV1> {
        match &*self.session.borrow() {
            UndoBackend::Local(session) => Some(session.project().clone()),
            #[cfg(feature = "in-process-mcp")]
            UndoBackend::Authority(authority) => authority.project(),
        }
    }

    /// 权威工程的一份拷贝（界面重投影用）。
    ///
    /// # Panics
    ///
    /// 权威会话没有活跃工程时 panic。需要容忍"工程被控制面关掉"的调用方用
    /// [`Self::try_project`]；`main.rs` 的装配路径保证端口存在时工程一定在。
    #[must_use]
    pub fn project(&self) -> YebanProjectV1 {
        self.try_project()
            .expect("撤销端口必须有一份工程（权威会话此刻没有活跃工程）")
    }

    /// 提交图谱的一份拷贝（**只读**用途：时光机画版本树、保存 `history.dag`）。
    ///
    /// 为什么给拷贝而不是 `&CommitGraph`：端口内部用 `RefCell` 持有会话，
    /// 借出去的引用会与 `perform` 的借用撞车。图谱本身是 `BTreeMap` 集合，
    /// 拷贝的代价是 O(提交数)。
    #[must_use]
    pub fn graph(&self) -> CommitGraph {
        match &*self.session.borrow() {
            UndoBackend::Local(session) => session.graph().clone(),
            #[cfg(feature = "in-process-mcp")]
            UndoBackend::Authority(authority) => authority.graph(),
        }
    }

    /// 当前工程指纹（模型容器字节的 SHA-256）。
    #[must_use]
    pub fn fingerprint(&self) -> Option<String> {
        match &*self.session.borrow() {
            UndoBackend::Local(session) => session.fingerprint().ok(),
            #[cfg(feature = "in-process-mcp")]
            UndoBackend::Authority(authority) => authority
                .project()
                .as_ref()
                .and_then(|project| project_fingerprint(project).ok()),
        }
    }

    /// 时光机弹窗是否打开（**界面运行态**，不是模型读数）。
    #[must_use]
    pub const fn undo_tree_open(&self) -> bool {
        self.open.get()
    }

    /// 动作日志（按发生顺序）。
    #[must_use]
    pub fn records(&self) -> Vec<UiActionRecord> {
        self.actions.borrow().clone()
    }

    /// 最近一次动作。
    #[must_use]
    pub fn last_record(&self) -> Option<UiActionRecord> {
        self.actions.borrow().last().cloned()
    }

    /// 未来的**编辑入口**（本线只接线撤销；编辑侧见台账的未实现项）。
    ///
    /// # Errors
    ///
    /// 见 [`undo_session::commit`]；权威路径下权威的拒绝原因原文照传
    /// （[`yeban_mcp::domain::error::Fault`] → [`UndoRefusal::Model`]）。
    pub fn commit_ops(
        &self,
        now_ms: u64,
        message: &str,
        ops: Vec<Op>,
    ) -> Result<EntityId, UndoRefusal> {
        match &mut *self.session.borrow_mut() {
            UndoBackend::Local(session) => session.commit(CommitRequest {
                now_ms,
                origin: OpOrigin::UserUi,
                message: message.to_owned(),
                ops,
            }),
            #[cfg(feature = "in-process-mcp")]
            UndoBackend::Authority(authority) => authority
                .apply_host(HostAction::Commit {
                    now_ms,
                    origin: OpOrigin::UserUi,
                    message: message.to_owned(),
                    ops,
                })
                .map_err(refusal_from_fault)
                .and_then(|outcome| {
                    outcome.commit.ok_or_else(|| UndoRefusal::Model {
                        detail: "权威提交成功但没有返回提交身份".to_owned(),
                    })
                }),
        }
    }

    /// 执行一个界面动作，并把结果记进动作日志。
    ///
    /// **这是所有撤销类 UI 事件的唯一落点**（`main.rs` 的 `Cmd+Z` 与时光机按钮、
    /// `input.rs` 的策略表都汇到这里）。
    pub fn perform(&self, action: UiAction) -> ActionOutcome {
        let before_fingerprint = self.fingerprint();
        let (undone_before, commits_before) = {
            let display = self.display();
            (display.undone, display.commit_count)
        };

        let outcome = match action {
            // `[UI-NOTE-003]` 工具选择是**视图状态**：端口碰不到 Slint, 因此这里只报告结果,
            // 设置 `active-tool` 属性由宿主的界面侧完成（N2 接线后即可）。
            UiAction::SelectTool(_) => ActionOutcome::DisplayOnly,
            // [D56] 人工导出诊断包。**不**改工程状态 ⇒ `DisplayOnly`。
            // 调的是与 MCP 工具**同一个** `yeban_diagnostics::export_diagnostics`（判据 4）。
            UiAction::ExportDiagnostics => {
                let dir = std::env::temp_dir();
                let config = yeban_diagnostics::unavailable_config_json();
                let inputs = yeban_diagnostics::BundleInputs {
                    config_json: Some(&config),
                    ..Default::default()
                };
                match yeban_diagnostics::export_diagnostics(&dir, inputs) {
                    Ok(report) => eprintln!("诊断包已写出: {}", report.path.display()),
                    Err(err) => eprintln!("诊断包导出失败: {err}"),
                }
                ActionOutcome::DisplayOnly
            }
            UiAction::Undo => self.undo_steps(1),
            UiAction::UndoMany(steps) => self.undo_steps(steps),
            UiAction::Redo => self.redo_steps(1),
            UiAction::OpenUndoTree => {
                self.open.set(true);
                ActionOutcome::DisplayOnly
            }
            UiAction::CloseUndoTree => {
                self.open.set(false);
                ActionOutcome::DisplayOnly
            }
            UiAction::ToggleUndoTree => {
                self.open.set(!self.open.get());
                ActionOutcome::DisplayOnly
            }
        };

        let after_fingerprint = self.fingerprint();
        let (undone_after, undoable_after, commits_after) = {
            let display = self.display();
            (display.undone, display.undoable, display.commit_count)
        };
        self.actions.borrow_mut().push(UiActionRecord {
            action: action.name(),
            outcome: outcome.clone(),
            fingerprint_before: before_fingerprint,
            fingerprint_after: after_fingerprint,
            undone_before,
            undone_after,
            undoable_after,
            commits_before,
            commits_after,
            undo_tree_open: self.open.get(),
        });
        outcome
    }

    fn undo_steps(&self, steps: usize) -> ActionOutcome {
        match &mut *self.session.borrow_mut() {
            UndoBackend::Local(session) => match session.undo_steps(steps) {
                Ok(outcome) => local_outcome(&outcome),
                Err(refusal) => ActionOutcome::Refused {
                    detail: refusal.to_string(),
                },
            },
            #[cfg(feature = "in-process-mcp")]
            UndoBackend::Authority(authority) => {
                match authority.apply_host(HostAction::Undo { steps }) {
                    Ok(outcome) => host_outcome(outcome),
                    Err(fault) => ActionOutcome::Refused {
                        detail: refusal_from_fault(fault).to_string(),
                    },
                }
            }
        }
    }

    fn redo_steps(&self, steps: usize) -> ActionOutcome {
        match &mut *self.session.borrow_mut() {
            UndoBackend::Local(session) => match session.redo_steps(steps) {
                Ok(outcome) => local_outcome(&outcome),
                Err(refusal) => ActionOutcome::Refused {
                    detail: refusal.to_string(),
                },
            },
            #[cfg(feature = "in-process-mcp")]
            UndoBackend::Authority(authority) => {
                match authority.apply_host(HostAction::Redo { steps }) {
                    Ok(outcome) => host_outcome(outcome),
                    Err(fault) => ActionOutcome::Refused {
                        detail: refusal_from_fault(fault).to_string(),
                    },
                }
            }
        }
    }
}

/// 本地会话的一次撤销 / 重做结果 → 界面动作后果。
fn local_outcome(outcome: &undo_session::UndoOutcome) -> ActionOutcome {
    ActionOutcome::Changed {
        steps: outcome.steps,
        undone_total: outcome.undone_total,
        op_kinds: outcome
            .op_kinds
            .iter()
            .map(|kind| (*kind).to_owned())
            .collect(),
    }
}

/// 权威会话的一次撤销 / 重做 / 提交结果 → 界面动作后果。
#[cfg(feature = "in-process-mcp")]
fn host_outcome(outcome: HostOutcome) -> ActionOutcome {
    ActionOutcome::Changed {
        steps: outcome.steps,
        undone_total: outcome.undone_total,
        op_kinds: outcome.op_kinds,
    }
}

/// 权威的显示态（`yeban_mcp` 侧类型）→ 界面侧的同名字段。
///
/// 两个 `UndoDisplay` 是**同一份源码**（`undo_session.rs`）在两个 crate 里各自实例化的
/// 类型（见本模块文档第 1 条），因此只能逐字段搬运 —— 这也正是"把同一个实例交给两边"
/// 在类型系统里不成立的那件事的可见代价。
#[cfg(feature = "in-process-mcp")]
fn display_from_authority(display: yeban_mcp::undo_session::UndoDisplay) -> UndoDisplay {
    UndoDisplay {
        branch: display.branch,
        head: display.head,
        commit_count: display.commit_count,
        branch_count: display.branch_count,
        undone: display.undone,
        undoable: display.undoable,
        can_undo: display.can_undo,
        can_redo: display.can_redo,
    }
}

/// 权威的拒绝（`Fault`）→ 界面侧的 [`UndoRefusal`]（一句人话，原文照传）。
#[cfg(feature = "in-process-mcp")]
fn refusal_from_fault(fault: Fault) -> UndoRefusal {
    match fault {
        Fault::Domain { message, .. } => UndoRefusal::Model { detail: message },
        Fault::Impl { error } => UndoRefusal::Model {
            detail: format!("{error:?}"),
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_planned_note_commits_through_the_port_and_undo_returns_it() {
        // `[UI-NOTE-003]` 第 259 轮第 4 步的两半: **恰 +1** 与**撤销可反转**。
        // 直接构造端口（与 main.rs / 本文件既有判据同一手法）, 不需要窗口 —— 判的是**模型侧的提交与逆操作**。
        let mut project = filled_project();
        let view = crate::bridge::ViewState::from_project(&project).expect("投影");
        // 实测教训: `filled_project()` 的**第一条轨道可能没有片段** ⇒ 必须找**第一条有片段的**轨道
        // （第一版写 `iter().next()` 于是 panic "轨道必须有片段" —— 判据的失败信息是我自己的夹具备注）。
        let (track_id, clip_id, start_tick) = project
            .tracks
            .iter()
            .find_map(|(track_id, track)| {
                track
                    .clips
                    .values()
                    .next()
                    .map(|placement| (*track_id, placement.clip_id, placement.start_tick))
            })
            .expect("夹具里至少要有一条带片段的轨道");
        let count = |project: &yeban_model::YebanProjectV1| {
            project
                .clip_pool
                .get(&clip_id)
                .and_then(|entry| entry.content.notes())
                .map_or(0, std::collections::BTreeMap::len)
        };
        let before = count(&project);
        let plan = crate::bridge::NotePlan {
            start_tick: start_tick + 240,
            pitch: 64,
            duration_ticks: view.ppq,
        };
        let port = UndoPort::new(
            UndoSession::open("<判据>", "yeban-app", project.clone(), NOW).expect("打开"),
        );
        let op = crate::bridge::plan_to_add_note(plan, track_id, clip_id, EntityId::new());
        port.commit_ops(NOW, "pencil: add note", vec![op])
            .expect("提交必须成功");
        assert_eq!(count(&port.project()), before + 1, "提交后必须**恰 +1**");
        // 撤销必须回到原值 —— 这是"可撤销是构造上的"这句话的判据。
        port.perform(UiAction::Undo);
        assert_eq!(count(&port.project()), before, "撤销后必须回到原值");
        let _ = &mut project;
    }

    #[test]
    fn select_tool_is_dispatched_and_named_but_carries_no_model_action() {
        // 判据: 工具选择经**唯一下发点** `dispatch_key` 得到 `UiAction::SelectTool`, 名字固定,
        // 且端口侧只报显示态（视图状态不进提交图, 也不改工程）。
        for digit in 1_u8..=5 {
            let tool = crate::input::Tool::from_digit(digit).expect("1..5 都是工具");
            let action = crate::input::Action::SelectTool(tool);
            assert_eq!(
                dispatch_key(action),
                Some(UiAction::SelectTool(tool)),
                "数字键 {digit} 的工具选择必须被下发（N2 接好键盘源后即生效）"
            );
            assert_eq!(UiAction::SelectTool(tool).name(), "select-tool");
        }
    }

    use super::*;
    use yeban_model::samples::filled_project;
    use yeban_model::{OpOrigin, UndoCursor};

    const NOW: u64 = 1_760_000_000_000;

    fn port() -> UndoPort {
        let session =
            UndoSession::open("<判据>", "yeban-app", filled_project(), NOW).expect("打开");
        UndoPort::new(session)
    }

    /// 夹具 op（与 MCP 侧同一个 `wiring_fixture`）。
    fn fixture_op(port: &UndoPort) -> Op {
        let fixture = undo_session::wiring_fixture(&port.project()).expect("夹具");
        fixture.op()
    }

    /// 判据 ⑨（界面侧）：**点了它 ⇒ 工程真的回退了一版**（动作日志断言）。
    #[test]
    fn a_ui_action_really_rolls_the_project_back_one_version() {
        let port = port();
        let pristine = port.fingerprint().expect("指纹");
        let op = fixture_op(&port);
        port.commit_ops(NOW + 1, "判据夹具", vec![op])
            .expect("提交");
        let edited = port.fingerprint().expect("指纹");
        assert_ne!(pristine, edited, "夹具必须真的改了工程");
        assert_eq!(port.display().undoable, 1, "提交之后可撤 1 步");

        // —— 这就是"点击"：界面动作走的是唯一入口 ——
        let outcome = port.perform(UiAction::Undo);
        assert_eq!(
            outcome,
            ActionOutcome::Changed {
                steps: 1,
                undone_total: 1,
                op_kinds: vec!["Batch".to_owned()],
            }
        );
        assert_eq!(
            port.fingerprint().expect("指纹"),
            pristine,
            "工程真的回退了一版"
        );

        let record = port.last_record().expect("动作日志");
        assert_eq!(record.action, "undo");
        assert!(record.changed_project(), "动作日志必须证明工程真的变了");
        assert_eq!(record.fingerprint_before.as_deref(), Some(edited.as_str()));
        assert_eq!(record.fingerprint_after.as_deref(), Some(pristine.as_str()));
        assert_eq!(record.undone_before, 0);
        assert_eq!(record.undone_after, 1);
        assert_eq!(record.undoable_after, 0);
        assert_eq!(record.commits_before, record.commits_after, "撤销不动图谱");
        assert_eq!(port.records().len(), 1);
    }

    /// 判据 ⑨（界面侧）：重做同样真的改工程。
    #[test]
    fn a_redo_action_really_restores_the_edited_bytes() {
        let port = port();
        let op = fixture_op(&port);
        port.commit_ops(NOW + 1, "判据夹具", vec![op])
            .expect("提交");
        let edited = port.fingerprint().expect("指纹");
        port.perform(UiAction::Undo);
        let outcome = port.perform(UiAction::Redo);
        assert_eq!(
            outcome,
            ActionOutcome::Changed {
                steps: 1,
                undone_total: 0,
                op_kinds: vec!["Batch".to_owned()],
            }
        );
        assert_eq!(port.fingerprint().expect("指纹"), edited);
        assert!(port.last_record().expect("日志").changed_project());
    }

    /// 判据 ⑨：没有历史时**拒绝**，且动作日志如实记录"工程没变"。
    #[test]
    fn a_refused_ui_action_is_recorded_as_not_changing_the_project() {
        let port = port();
        let before = port.fingerprint().expect("指纹");
        let outcome = port.perform(UiAction::Undo);
        assert!(
            matches!(outcome, ActionOutcome::Refused { .. }),
            "{outcome:?}"
        );
        let record = port.last_record().expect("日志");
        assert!(!record.changed_project(), "拒绝路径不得声称改了工程");
        assert_eq!(record.fingerprint_before, record.fingerprint_after);
        assert_eq!(port.fingerprint().expect("指纹"), before);
        assert_eq!(record.undone_after, 0);
    }

    /// 判据 ⑨：时光机开关**只**改界面运行态（不碰工程、不碰游标）。
    #[test]
    fn the_undo_tree_toggle_only_touches_ui_runtime_state() {
        let port = port();
        let before = port.fingerprint().expect("指纹");
        assert!(!port.undo_tree_open());
        assert_eq!(
            port.perform(UiAction::ToggleUndoTree),
            ActionOutcome::DisplayOnly
        );
        assert!(port.undo_tree_open());
        assert_eq!(
            port.perform(UiAction::ToggleUndoTree),
            ActionOutcome::DisplayOnly
        );
        assert!(!port.undo_tree_open());
        assert_eq!(
            port.perform(UiAction::OpenUndoTree),
            ActionOutcome::DisplayOnly
        );
        assert!(port.undo_tree_open());
        assert_eq!(
            port.perform(UiAction::CloseUndoTree),
            ActionOutcome::DisplayOnly
        );
        assert!(!port.undo_tree_open());
        assert_eq!(port.fingerprint().expect("指纹"), before, "工程一位不动");
        assert!(!port.last_record().expect("日志").changed_project());
        assert_eq!(port.records().len(), 4, "四次动作四条记录");
    }

    /// 判据 ⑨：多步撤销（时光机节点）一次真的回退多步。
    #[test]
    fn undo_many_steps_back_in_one_action() {
        let port = port();
        let pristine = port.fingerprint().expect("指纹");
        for step in 0..3_u8 {
            let op = {
                let fixture = undo_session::wiring_fixture(&port.project()).expect("夹具");
                let current = port
                    .project()
                    .clip_pool
                    .get(&fixture.clip_id)
                    .and_then(|entry| entry.content.notes())
                    .and_then(|notes| notes.get(&fixture.note_id))
                    .expect("音符")
                    .velocity;
                Op::ModifyNoteVelocity {
                    track_id: fixture.track_id,
                    clip_id: fixture.clip_id,
                    note_id: fixture.note_id,
                    old_vel: current,
                    new_vel: 30 + step,
                }
            };
            port.commit_ops(NOW + u64::from(step) + 1, "判据夹具", vec![op])
                .expect("提交");
        }
        assert_eq!(port.display().undoable, 3);
        let outcome = port.perform(UiAction::UndoMany(3));
        assert_eq!(
            outcome,
            ActionOutcome::Changed {
                steps: 3,
                undone_total: 3,
                op_kinds: vec!["Batch".to_owned(); 3],
            }
        );
        assert_eq!(port.fingerprint().expect("指纹"), pristine);
        let record = port.last_record().expect("日志");
        assert_eq!(record.undone_before, 0);
        assert_eq!(record.undone_after, 3);
        assert_eq!(record.undoable_after, 0);
    }

    /// 判据：键盘策略表 → 界面动作的映射（`Cmd+Z` / `Cmd+Shift+Z` / `Cmd+Shift+H`）。
    #[test]
    fn key_actions_map_to_the_undo_entry_points() {
        assert_eq!(dispatch_key(Action::Undo), Some(UiAction::Undo));
        assert_eq!(dispatch_key(Action::Redo), Some(UiAction::Redo));
        assert_eq!(
            dispatch_key(Action::OpenTimeMachine),
            Some(UiAction::OpenUndoTree)
        );
        for other in [Action::PlayPause, Action::Duplicate, Action::Cancel] {
            assert_eq!(dispatch_key(other), None, "{other:?} 与撤销无关");
        }
        // 映射出来的动作真的有效：`Cmd+Z` 的落点能改工程。
        let port = port();
        let op = fixture_op(&port);
        port.commit_ops(NOW + 1, "判据夹具", vec![op])
            .expect("提交");
        let pristine = port.fingerprint().expect("指纹");
        port.perform(UiAction::Undo);
        assert_eq!(port.display().undone, 1);
        assert!(port.last_record().expect("日志").changed_project());
        let _ = pristine;
    }

    /// 判据 ⑬：动作日志是顺序累积的（同一端口上连按 `Cmd+Z` 逐步回退）。
    #[test]
    fn repeated_undo_actions_accumulate_in_the_log() {
        let port = port();
        let op = fixture_op(&port);
        port.commit_ops(NOW + 1, "判据夹具", vec![op])
            .expect("提交");
        assert!(port.perform(UiAction::Undo).changed());
        assert!(matches!(
            port.perform(UiAction::Undo),
            ActionOutcome::Refused { .. }
        ));
        let records = port.records();
        assert_eq!(records.len(), 2);
        assert!(records[0].changed_project());
        assert!(!records[1].changed_project());
        assert_eq!(records[1].undone_before, 1, "第二次动作前已经撤了 1 步");
        assert_eq!(records[1].undone_after, 1);
    }

    /// 判据 ⑨：键盘那一跳接上策略表之后**真的**回退工程（`Cmd+Z` 整条链）。
    #[test]
    fn the_keyboard_chain_really_rolls_the_project_back() {
        let port = port();
        let pristine = port.fingerprint().expect("指纹");
        let op = fixture_op(&port);
        port.commit_ops(NOW + 1, "判据夹具", vec![op])
            .expect("提交");
        assert_ne!(port.fingerprint().expect("指纹"), pristine);

        // 画布聚焦 + `Cmd+Z` ⇒ 真的撤一步。
        let canvas = InputContext::new(); // 启动态 = 画布聚焦、非合成态
        let outcome = perform_key(&port, &canvas, PhysicalKey::KeyZ, Modifiers::meta());
        assert_eq!(
            outcome,
            Some(ActionOutcome::Changed {
                steps: 1,
                undone_total: 1,
                op_kinds: vec!["Batch".to_owned()],
            })
        );
        assert_eq!(
            port.fingerprint().expect("指纹"),
            pristine,
            "工程真的回退了"
        );
        assert_eq!(port.last_record().expect("日志").action, "undo");

        // `Cmd+Shift+Z` ⇒ 重做回到编辑过的那一版。
        let outcome = perform_key(&port, &canvas, PhysicalKey::KeyZ, Modifiers::ctrl_shift());
        assert!(matches!(
            outcome,
            Some(ActionOutcome::Changed { steps: 1, .. })
        ));
        assert_ne!(
            port.fingerprint().expect("指纹"),
            pristine,
            "重做真的改回来了"
        );

        // `Cmd+Shift+H` ⇒ 打开时光机（只改界面运行态）。
        assert_eq!(
            perform_key(&port, &canvas, PhysicalKey::KeyH, Modifiers::ctrl_shift()),
            Some(ActionOutcome::DisplayOnly)
        );
        assert!(port.undo_tree_open());

        // 与撤销无关的单键 ⇒ 放行。
        assert_eq!(
            perform_key(&port, &canvas, PhysicalKey::Space, Modifiers::none()),
            None
        );
        // 文本输入框聚焦 + `Cmd+Z` ⇒ 策略表说 `PassThrough`（给文本框），因此这里放行。
        let mut composing = InputContext::new();
        composing.set_focus(crate::input::Focus::TextInput);
        assert_eq!(
            perform_key(&port, &composing, PhysicalKey::KeyZ, Modifiers::meta()),
            None,
            "文本框里的 Cmd+Z 必须留给文本框（[UI-A11Y-002]）"
        );
    }

    /// 判据（`N2` 裁决 (1)）：**逻辑键**那一跳也真的回退工程 —— 也就是 GUI 上的 `Cmd+Z`。
    ///
    /// 与上一条判据的关系：上一条证明物理码入口（无头端口）的整条链，这一条证明
    /// GUI 唯一拿得到的那套输入（逻辑键名 + 修饰位）落到**同一个** `UndoPort`。
    /// 两条都过 ⇒ "人按 `Cmd+Z` 与 AI 发工具调用改同一串字节"（D45）在两条入口上都成立。
    #[test]
    fn the_logical_key_chain_really_rolls_the_project_back() {
        let port = port();
        let pristine = port.fingerprint().expect("指纹");
        let op = fixture_op(&port);
        port.commit_ops(NOW + 1, "判据夹具", vec![op])
            .expect("提交");
        let edited = port.fingerprint().expect("指纹");
        assert_ne!(edited, pristine, "夹具必须真的改了工程");

        let canvas = InputContext::new(); // 启动态 = 画布聚焦、非合成态
        // GUI 上 `Cmd+Z` 的 `event.text` 是 `"z"`；macOS 上 Slint 把 `Cmd` 映射到
        // `KeyboardModifiers::control`，两条都由 `Modifiers::command()` 覆盖。
        let outcome = perform_logical_key(
            &port,
            &canvas,
            LogicalKey::Character('z'),
            Modifiers::meta(),
        );
        assert!(
            matches!(outcome, Some(ActionOutcome::Changed { steps: 1, .. })),
            "逻辑键 `Ctrl/Cmd+z` 必须真的撤销一步: {outcome:?}"
        );
        assert_eq!(
            port.fingerprint().expect("指纹"),
            pristine,
            "逻辑键路径必须真的把工程回退一版"
        );
        assert_eq!(port.last_record().expect("日志").action, "undo");

        // `Cmd+Shift+Z` ⇒ 重做（逻辑键 + `shift` 位）。
        let outcome = perform_logical_key(
            &port,
            &canvas,
            LogicalKey::Character('z'),
            Modifiers::ctrl_shift(),
        );
        assert!(matches!(
            outcome,
            Some(ActionOutcome::Changed { steps: 1, .. })
        ));
        assert_eq!(port.fingerprint().expect("指纹"), edited);

        // 与撤销无关的逻辑键仍走**同一**下发点（`SelectTool` 只报显示态，不改工程）。
        assert_eq!(
            perform_logical_key(
                &port,
                &canvas,
                LogicalKey::Character('3'),
                Modifiers::none()
            ),
            Some(ActionOutcome::DisplayOnly),
            "工具选择的逻辑键必须经唯一下发点落到端口（界面属性由宿主写）"
        );
        // 未绑定的逻辑键放行（不消费）。
        assert_eq!(
            perform_logical_key(
                &port,
                &canvas,
                LogicalKey::Character('q'),
                Modifiers::none()
            ),
            None
        );
        // 文本输入框聚焦 + `Cmd+Z` ⇒ 策略表说 `PassThrough`（留给文本框做文本撤销）。
        let mut text = InputContext::new();
        text.set_focus(crate::input::Focus::TextInput);
        assert_eq!(
            perform_logical_key(&port, &text, LogicalKey::Character('z'), Modifiers::meta()),
            None,
            "文本框里的 Cmd+Z 必须留给文本框（[UI-A11Y-002]）"
        );
    }

    /// 判据：会话态是从一份**全新的**会话开始的（打开边界在端口这一层也成立）。
    #[test]
    fn a_fresh_port_has_no_history() {
        let port = port();
        assert_eq!(port.display().undoable, 0);
        assert_eq!(port.display().undone, 0);
        assert!(!port.display().can_undo);
        assert_eq!(port.display().commit_count, 1, "只有一条根提交");
        // `UndoCursor` 的存在本身证明"游标是类型上不可序列化的会话态"。
        let cursor = UndoCursor::new();
        assert_eq!(cursor.undone(), 0);
        let _ = OpOrigin::UserUi;
    }
}
