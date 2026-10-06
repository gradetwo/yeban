//! **生产窗口的运行期重投影钩子** —— `ROAD-M4-008` 选项 (a) 的**第 (b) 项**。
//!
//! ## 它补的是哪一个洞（实测，不是转述）
//!
//! 第一片把"唯一可变权威 + 投影"做成了可判据的能力，但那个能力住在
//! `crates/yeban-app/src/live_surface.rs` —— 它只被**测试目标**用 `#[path]` 装进去
//! （它依赖的 `yeban-ui-test-port` / `yeban-ui-mcp` 是 dev-dependency，不进产品二进制）。
//! 第二片把 GUI 的**写**入口搬到了那个权威上，于是 GUI **自己**的动作会重投影
//! （`host::refresh_undo_window` / `host::wire_roll_edit` 都用 `port.try_project()`）。
//!
//! 剩下的是**会话侧**：一次 `tools/call`（例如 AI 经环回控制面发 `yeban_edit_automation`）
//! 改的是同一个 `Domain`，但它**不经过** GUI 的任何回调，因此生产窗口不会刷新。
//!
//! ## 为什么是"事件驱动"而不是"定时器"（本轮的结构性裁决）
//!
//! 本模块**不装定时器、不起后台线程**。数据流是：
//!
//! ```text
//! 会话侧一次真的改了工程的请求
//!   └─ yeban-mcp 的 HttpServer::respond：释放分发器锁之后、写出响应之前
//!        └─ RevisionSink::notify(新修订号)            ← 【定义好的时点】
//!             └─ AuthorityMirror::event_loop_sink 的闭包（跑在服务线程上）
//!                  └─ slint::invoke_from_event_loop(...)  ← 唯一的跨线程手段
//!                       └─ UI 线程：AuthorityMirror::sync_weak
//!                            └─ host::apply_view（把权威工程的投影注入活窗口）
//! ```
//!
//! 这个形态**可以被无头判据确定性地驱动**：通知发生在响应字节写出**之前**，因此
//! "客户端收到响应"蕴含"通知已经发生"；判据随后把事件循环里那一条排队的调用跑掉
//! （`slint::run_event_loop` 在弹出的队列上 FIFO 执行），不需要 sleep、不需要轮询、
//! 不需要等待任何时钟。
//!
//! ## 它**不是**第二个写者（承载性约束）
//!
//! - 读权威只用 [`ProjectAuthorityHandle::apply_revision`] 与
//!   [`ProjectAuthorityHandle::project`] —— 两个都是 `&Domain` 的只读口，
//!   签名里没有 `&mut`；
//! - 本模块**没有**任何能推进修订号的调用：推进只发生在
//!   `yeban_mcp::domain::apply` 一处；
//! - 那个修订号观察者的类型是 `Fn(u64)`（见 `HttpServer::set_project_revision_sink`），
//!   它拿不到 `Domain`、`Dispatcher` 或任何工具入口；
//! - `projected` 只是"这个窗口已经投影过哪一版权威"的**书签**（与
//!   `live_surface::LiveAdminSurface::authority_revision` 同义），它推进的是**投影游标**，
//!   不是权威的修订号。
//!
//! ## 失败时出声
//!
//! `slint::invoke_from_event_loop` 在没有事件循环的平台上返回
//! `Err(EventLoopError::NoEventLoopProvider)`（上游语义：`Platform::new_event_loop_proxy`
//! 的默认实现返回 `None`）。这时**没有任何东西被静默吞掉**：本模块打一行 stderr，
//! 并且 `sync_weak` 停留在"未投影"状态 —— 下一次通知会再试一次，而不是把
//! "已经投影过了"记成一句不成立的话。
//!
//! 生产平台（`slint` 的 `backend-winit`）实现了 `new_event_loop_proxy`，因此这一跳
//! 在生产里成立；宿主自己**不**探测平台能力（那会变成第二套事实源）。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bridge::ViewState;
use crate::host;
use crate::mcp_mount::{InProcessMcp, ProjectAuthorityHandle, WeakProjectAuthorityHandle};
use crate::ui::MainWindow;
use yeban_mcp::transport::http::ProjectRevisionSink;

/// 一次重投影尝试的读数（判据与日志共用；**不是** `bool`，因为三种结局必须能分开）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionOutcome {
    /// 权威的修订号没有前进 ⇒ 一位没动（常态：只读调用 / 已经投影过这一版）。
    Unchanged,
    /// 权威有新的工程，已经重新投影并注入**这个**活窗口。
    Reprojected {
        /// 投影完之后权威的施加修订号。
        revision: u64,
    },
    /// 权威的会话没有活跃工程（例如控制面 `yeban_close_project`）⇒ 投影不动。
    NoActiveProject {
        /// 权威当前的施加修订号。
        revision: u64,
    },
    /// 权威服务已经停机（弱句柄升级不到）⇒ **放弃**，并如实报告。
    AuthorityGone,
    /// 窗口已经销毁 ⇒ 放弃。
    WindowGone,
}

/// **生产窗口**与唯一可变权威之间的投影镜子。
///
/// 它持有三样东西，**没有一样**是工程本身：
///
/// | 字段 | 是什么 | 为什么是它 |
/// | :--- | :--- | :--- |
/// | `window` | 活窗口的**弱**引用 | 重投影必须落在**同一个**窗口上，但镜子不许延长窗口寿命 |
/// | `authority` | 权威的**弱**句柄 | 强句柄会与服务内的观察者形成引用环（见 [`WeakProjectAuthorityHandle`]） |
/// | `projected` | 已投影的权威修订号 | "要不要重投影"的开关，与修订号同口径 |
///
/// 它是 `Send + Sync` 的（`slint::Weak<MainWindow>` 与 `Weak<HttpServer>` 都是），
/// 因此可以被安全地放进 `invoke_from_event_loop` 的闭包里。
pub struct AuthorityMirror {
    /// 活窗口（弱引用）。
    ///
    /// 是 `slint::Weak`（**不是** `std::sync::Weak`）：上游为它写了
    /// `unsafe impl Send` / `unsafe impl Sync`（`i-slint-core` 的 `api.rs:1247-1250`），
    /// 这正是"从别的线程把重投影交回 UI 线程"在 Slint 里成立的原因。
    window: slint::Weak<MainWindow>,
    /// 唯一可变权威（弱引用）。
    authority: WeakProjectAuthorityHandle,
    /// 已经投影过的那一版权威修订号。
    projected: AtomicU64,
}

impl AuthorityMirror {
    /// 以"当前窗口已经画着权威当前这一版工程"为起点建镜子。
    ///
    /// 起点读数取自权威本身（[`ProjectAuthorityHandle::apply_revision`]），而不是
    /// 调用方随手给一个数：生产路径上窗口就是用同一个工程建出来的
    /// （`mount_in_process_mcp` 把 `loaded.archive.project` 的克隆交给了
    /// `Domain::open_in_memory`），因此"已投影 = 当前修订号"是一个**可读的事实**。
    #[must_use]
    pub fn new(ui: &MainWindow, authority: &ProjectAuthorityHandle) -> Self {
        Self {
            window: slint::ComponentHandle::as_weak(ui),
            authority: authority.downgrade(),
            projected: AtomicU64::new(authority.apply_revision()),
        }
    }

    /// 已经投影过的权威修订号（判据用）。
    #[must_use]
    pub fn projected_revision(&self) -> u64 {
        self.projected.load(Ordering::SeqCst)
    }

    /// **重投影钩子的本体**：读权威 → 投影 → 注入**这个**活窗口。
    ///
    /// 它是幂等的、只读权威的、并且是判据唯一需要驱动的函数（判据直接调它就能
    /// 在无头环境里确定性地把"会话侧改了工程"变成"窗口上看得见"）。
    ///
    /// 顺序是契约（与 `live_surface::LiveAdminSurface::sync_authority` 同款）：
    /// 1. 读修订号；`projected` 已经不小于它 ⇒ [`ProjectionOutcome::Unchanged`]；
    /// 2. 取权威工程；`None` ⇒ [`ProjectionOutcome::NoActiveProject`]（**不**假装清空界面）；
    /// 3. 投影（`ViewState::from_project`）并注入（`host::apply_view`）——
    ///    **只有成功了**才把 `projected` 推上去：投影失败会让下一次重试，
    ///    而不是把"已经投影过了"记成一句不成立的话。
    ///
    /// 卷帘偏移复用界面上的当前值：会话侧改了工程**不该**把卷帘滚回起点
    /// （与 `host::refresh_undo_window` 同一条纪律）。
    pub fn sync_now(&self, ui: &MainWindow) -> ProjectionOutcome {
        let Some(authority) = self.authority.upgrade() else {
            return ProjectionOutcome::AuthorityGone;
        };
        let revision = authority.apply_revision();
        if self.projected.load(Ordering::SeqCst) >= revision {
            return ProjectionOutcome::Unchanged;
        }
        let Some(project) = authority.project() else {
            return ProjectionOutcome::NoActiveProject { revision };
        };
        match ViewState::from_project(&project) {
            Ok(view) => {
                let scroll_x = ui.get_roll_scroll_x();
                host::apply_view(
                    ui,
                    &view,
                    slint::ComponentHandle::window(ui).size().width as f32,
                    scroll_x,
                );
                self.projected.store(revision, Ordering::SeqCst);
                ProjectionOutcome::Reprojected { revision }
            }
            Err(error) => {
                // 投影失败**出声**：工程已经变了，但这一帧画不出来。
                // 静默吞掉会让"AI 改了但界面没动"变成一个查不出的现象。
                eprintln!("[yeban-app] 会话侧改工程后重投影失败: {error}");
                ProjectionOutcome::Unchanged
            }
        }
    }

    /// 同一件事的**弱**形态（给已经 marshal 到 UI 线程的闭包用）。
    #[must_use]
    pub fn sync_weak(&self) -> ProjectionOutcome {
        match self.window.upgrade() {
            Some(ui) => self.sync_now(&ui),
            None => ProjectionOutcome::WindowGone,
        }
    }

    /// **生产驱动**：造出装到控制面上的那个修订号观察者。
    ///
    /// 它跑在**服务线程**上，因此只做一件事：把重投影 marshal 到 UI 线程
    /// （`[ARCH-TOP-002]`：UI 对象只许在 UI 线程上碰）。它**不**读权威、**不**碰窗口
    /// —— 那些都在 [`Self::sync_weak`] 里、在 UI 线程上做。
    ///
    /// 返回值可以直接交给
    /// [`InProcessMcp::set_project_revision_sink`](crate::mcp_mount::InProcessMcp::set_project_revision_sink)。
    #[must_use]
    pub fn event_loop_sink(mirror: &Arc<Self>) -> ProjectRevisionSink {
        let mirror = Arc::clone(mirror);
        Arc::new(move |revision: u64| {
            // 已经投影过这一版（或更新）⇒ 连 marshal 都不必做（只读请求根本不会走到这里）。
            if mirror.projected.load(Ordering::SeqCst) >= revision {
                return;
            }
            let queued = Arc::clone(&mirror);
            if let Err(error) = slint::invoke_from_event_loop(move || {
                let _ = queued.sync_weak();
            }) {
                eprintln!(
                    "[yeban-app] 会话侧改工程后无法把重投影送到 UI 线程 (修订号 {revision}): {error} \
                     —— 该平台的 new_event_loop_proxy 返回 None（生产用 backend-winit 时不会发生）"
                );
            }
        })
    }

    /// **生产路径的安装点**：建镜子 + 把它的观察者装到控制面上。
    ///
    /// 返回 `None` 表示服务已经停机（观察者**没装上**）—— 调用方如实报告，
    /// 而不是留一个"看起来装了"的钩子。返回值必须被调用方**持有到事件循环结束**
    /// （它持有窗口与权威的弱引用；丢掉它会让观察者手里的 `Arc` 成为唯一持有者，
    /// 那虽然仍能工作，但日志里的 `projected_revision` 就没人读了）。
    #[must_use]
    pub fn install(ui: &MainWindow, mount: &InProcessMcp) -> Option<Arc<Self>> {
        let authority = mount.project_authority();
        let mirror = Arc::new(Self::new(ui, &authority));
        if mount.set_project_revision_sink(Some(Self::event_loop_sink(&mirror))) {
            Some(mirror)
        } else {
            None
        }
    }
}

impl std::fmt::Debug for AuthorityMirror {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 刻意不打印窗口 / 权威（弱引用升级会短暂延长它们的寿命，调试输出不该有副作用）。
        formatter
            .debug_struct("AuthorityMirror")
            .field("projected", &self.projected.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}
