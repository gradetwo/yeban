//! **app 侧的接线点**：活的 `MainWindow` → `LivePort`（Tier-1 光栅化 + 运行时控件树）
//! → `PortAdapter`（零 Slint 的 `UiSurface`）→ `ControlPlane`（`ui/*` 方法）；
//! 以及**三个管理动作**的真实落地 + **电平消费**（`[ARCH-UI-002]` 的 UI 一半）。
//!
//! 规范来源 (Normative)：
//! - `[ARCH-UI-002]`：电平经无锁 SPSC 解耦，UI 线程定时器批量出队更新 Slint Properties
//!   （生产侧在 `yeban-engine`，消费侧在 [`crate::meters`]，注入在 [`crate::host`]）；
//! - `[ARCH-UI-004]` / `[UI-TEST-001]` §12.2：语义 ID 寻址是**唯一**的自动化寻址方式；
//! - `[UI-MCP-001]` §12.3：三级权限（ReadOnly / Interactive / **Administrative**）——
//!   本文件让 Administrative 的三件事**真的发生**：切主视图 / 强制保存 / 重载引擎；
//! - `[UI-MCP-002]` §12.5：动态区遮罩（由 `LivePort` 的运行时几何 + 注册表的动态标记合成）；
//! - `[MUST-GATE-015]`：Golden 必须由 **Tier-1 软件光栅化**产出，尺寸非零且非全黑；
//! - `[ARCH-SEC-004]`：`ui/force_save` 的落盘走 [`crate::save`] 的原子替换；
//! - ADR-0001 **D28**（投影层零 Slint + **唯一**注入点）、**D22**（debug info 构建期打开，
//!   `build.rs` 已开）、**D18**（`SLINT_BACKEND=headless` 不存在 ⇒ 平台是**装进去**的）、
//!   **D19**（引擎用默认 feature，本文件不出现任何 `cpal::*`）。
//!
//! ## 谁持有谁（这就是本线要建立的接线图）
//!
//! ```text
//!   YebanProjectV1
//!     └── bridge::from_project ──> ViewState ──> elements::ElementRegistry
//!            │                                        │
//!            │                                        └── registry_tree ──> ControlTree（静态注册表）
//!            └── scene::from_view ──> DemoScene          │
//!                                                        ▼
//!   LiveControlPlane ──owns──> ControlPlane ──owns(Box)──> LiveAdminSurface
//!        │                                                     │ owns
//!        │                             ┌───────────────────────┼────────────────────────┐
//!        │                             ▼                       ▼                        ▼
//!        │                   PortAdapter<LivePort<..>, fn>   MainWindow 句柄      MeterRuntime
//!        │                             │ owns                （clone_strong）   （电平消费端）
//!        │                             ▼                       │                        │
//!        │                   LivePort<MainWindow>              │                   MeterBoard
//!        │                     │ owns                           │
//!        │                     ├── MainWindow（host::build_main_window_with_console_tab）
//!        │                     └── Tier1Window（MinimalSoftwareWindow）
//!        └──owns──> 对照帧 Rgb8Image（装配时直接抓的一帧）
//!
//!   EngineHost（引擎宿主，`ui/reload_engine` 才建）──owns──> SnapshotSlot + EngineRuntime
//!        └── 交出一条**新**的 MeterCollector ──adopt──> MeterRuntime（旧读数作废）
//!
//!   实时线程（未来：cpal 回调）持有 MeterPublisher ──SPSC──> MeterRuntime.collector（UI 线程）
//! ```
//!
//! ## 线程 / 所有权边界（**不要**在这里造第二条队列）
//!
//! - **实时线程**（今天由控制面/测试线程显式驱动 `EngineRuntime::process_quantum`，
//!   明天是 cpal 回调）持 `MeterPublisher`；**UI 线程**持 [`MeterRuntime`]。
//!   两者之间只有 `yeban_engine::meter::meter_channel` 建的那条 SPSC。
//! - "取最新"由引擎的 `MeterCollector::drain_latest` + `MeterBoard::ingest`（`supersedes`）
//!   完成 —— app 侧**没有**第二个缓存、**没有** `VecDeque`、**没有** `Mutex`。
//! - 跨线程不复制窗口状态：`Send` 边界就是这条链的边界（见 §"`Send` 边界说明"）。
//!
//! ## `Send` 边界说明（重要）
//!
//! `MainWindow` 与 `MinimalSoftwareWindow` 都是 `Rc` 语义、平台上下文是**线程局部**的，
//! 因此这一整条链**不是 `Send`**。这不是缺陷，是刻意的：
//! `yeban_ui_mcp` 的环回 HTTP 是**单线程串行** `accept`（见那边的 `transport/http.rs` 文件头），
//! 它不要求 `Send`；反过来，为了"能跨线程"去复制一份窗口状态，就会造出**两份事实源** ——
//! 那是本仓库最不愿意付的代价（`[MODEL-AST-002]` 的投影方向只有一个）。
//!
//! ## 为什么这个文件不在 `lib.rs` 的模块表里（以及它怎么被编译）
//!
//! 它需要 `yeban-ui-mcp` 与 `yeban-ui-test-port`，而这两个依赖在
//! `crates/yeban-app/Cargo.toml` 里都是 **dev-dependency**（`AGENTS.md` §2 红线 6：
//! 默认 release 构建里不得开启 `ui-mcp` / 内省能力；`cargo tree -p yeban-app -e normal`
//! 里因此看不到它们）。所以本文件只被测试目标用 `#[path]` 装进去：
//!
//! ```ignore
//! #[path = "../src/live_surface.rs"]
//! mod live;
//! ```
//!
//! 它**不是**"只在测试里存在的假实现"：`LivePort` / `PortAdapter` / `ControlPlane`
//! 三个类型都是各自 crate 里的公开产品路径，本文件只负责把它们接起来，而且
//! 构造 `MainWindow` 走的仍然是 `host::build_main_window*`（**唯一**注入点，D28）。
//!
//! ## 这个接法的固有代价（**显式设计，不是意外**）
//!
//! 用 `#[path]` 从测试目标引入产品源码，意味着这个文件是在**测试 crate** 里被编译的
//! （CI 日志里它的路径长成 `crates/yeban-app/tests/../src/live_surface.rs`）。后果有两条，
//! 都是刻意的取舍：
//!
//! 1. **`-D warnings` 下任何没人用的项都是硬错误**。CI 第 1 轮实测就死在这里：
//!    `error: method \`scene\` is never used`（那个访问器判据没用到）。
//!    处置是**删掉**它（`docs/ledger/live-port-notes.md` §6.1），
//!    而不是 `#[allow(dead_code)]` —— 后者会把"这块代码没人用"这个真实信号一起盖掉。
//!    ⇒ **本文件的公开 API 只包含当前真的被调用的东西**；加方法时要么同时接上调用方，
//!    要么就别加。判据只住在 `crates/yeban-app/tests/live_ui_mcp.rs` **一个**目标里，
//!    这样"用了什么"与"提供了什么"可以对齐（多目标会让某个目标看不到某个方法 ⇒ 假死代码）。
//! 2. **它不会进 release 构建**（dev-dependency 的直接后果，红线 6 要的正是这个）。
//!    哪天要把它变成产品路径，做法是"加一个非默认 feature + 把依赖从 dev 段挪到
//!    `[dependencies]` 的 optional 段"，而不是让测试目标继续当它的唯一编译者。

#![allow(missing_docs, rust_2018_idioms)]

/// `ElementRegistry` → `ControlTree` 的**唯一**实现（同一份文件被本机探针复用）。
#[path = "registry_tree.rs"]
mod registry_tree;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use registry_tree::control_tree_from_registry;

use slint::ComponentHandle as _;

use yeban_app::bridge::{BridgeError, ViewState};
use yeban_app::elements::ElementRegistry;
use yeban_app::engine_host::{
    EditMark, EngineHost, EngineHostError, HeartbeatReadings, SnapshotCounts,
};
use yeban_app::host;
use yeban_app::input::{Focus, InputContext, Modifiers, PhysicalKey, Resolution};
// `ROAD-M4-008` 选项 (a)：控制面会话是**唯一可变权威**，界面是它的投影。
// 这个句柄只存在于非默认 feature `in-process-mcp` 下 —— 默认构建里没有 `yeban-mcp`
// （`grep -c "yeban-mcp" <tree -p yeban-app -e normal>` = 0），因此这里也必须 cfg。
#[cfg(feature = "in-process-mcp")]
use yeban_app::mcp_mount::ProjectAuthorityHandle;
use yeban_app::meters::MeterRuntime;
use yeban_app::save::{SaveError, save_project_file};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_engine::meter::MeterCollector;
use yeban_model::{EntityId, YebanProjectV1};
use yeban_ui_mcp::ime::{ImeFocus, ImeState};
use yeban_ui_mcp::live::ControlPlane;
use yeban_ui_mcp::service::UiService;
use yeban_ui_mcp::surface::{
    AdminReport, PortAdapter, PreviewArguments, PreviewEffect, ReportValue, UiSurface,
};
use yeban_ui_test_port::port::{KeyCode, Permission, PointerButton, PortError, UiTestPort};
use yeban_ui_test_port::render::{LivePort, RenderError, Tier1Window};
use yeban_ui_test_port::tree::{ControlTree, TreeError};
use yeban_ui_test_port::{Rgb8Image, Size};

/// 真实执行面的类型：`LivePort<MainWindow>` + 「怎么拿像素」的**函数指针**。
///
/// 用 `fn` 指针而不是闭包类型，是为了让这个类型可以被写出来（判据要以它作返回类型）。
/// `PortAdapter::new` 只要求 `F: Fn(&P) -> Result<Rgb8Image, PortError>`，
/// 非捕获的 `fn` 项直接满足。
pub type LiveSurface =
    PortAdapter<LivePort<MainWindow>, fn(&LivePort<MainWindow>) -> Result<Rgb8Image, PortError>>;

/// **唯一的**"从活窗口拿 Tier-1 像素"实现（`[MUST-GATE-015]`）。
///
/// 它就是 `yeban_ui_test_port::render::Tier1Window::capture`：`SoftwareRenderer` 渲染进
/// 内存 `SharedPixelBuffer<Rgb8Pixel>`，**不经过任何 `i-slint-backend-testing` 路径**
/// （那个后端不渲染像素，规范明文禁止用它产出 Golden）。
fn capture_tier1(port: &LivePort<MainWindow>) -> Result<Rgb8Image, PortError> {
    port.window().capture().map_err(|error| PortError::Capture {
        message: error.to_string(),
    })
}

/// 装配一条真实执行面时的**可选项**。
///
/// 存在的理由：`build_live_ui(project, permission)` 是"默认形态"（控制台 Tab 0 = 卷帘、
/// 不配保存路径、重载引擎时推 8 个量子），而判据需要三个旋钮：
///
/// | 字段 | 为什么需要它 |
/// | :--- | :--- |
/// | `console_tab` | 混音台（Tab 1）只在被选中时进入运行时控件树 —— "通道条数 == 轨道数"这条判据必须先让它可见 |
/// | `save_path` | `ui/force_save` 必须写到一个**真的路径**；不给路径时它**如实报错**（不是假成功） |
/// | `engine_quanta` | `ui/reload_engine` 重建后推多少个量子；0 = 只重建不推进（合法） |
/// | `undo` | 撤销端口（`None` ⇒ 撤销族与删除如实 `reject`，与产品里没有会话时同款）；判据用它见证"按键 → 可撤销提交" |
#[derive(Debug, Clone)]
pub struct LiveWiringOptions {
    /// 端口三级权限（`[UI-MCP-001]`）。`into_control_plane` 的 `permission` 通常与它相同。
    pub permission: Permission,
    /// 底部控制台的初始 Tab（0 = 卷帘 / 1 = 调音台 / 2 = 设备链）。
    pub console_tab: i32,
    /// `ui/force_save` 的落点（`None` ⇒ 该动作如实报"未配置保存路径"）。
    pub save_path: Option<PathBuf>,
    /// `ui/reload_engine` 每次重建后推进的量子数。
    pub engine_quanta: u64,
    /// 键盘路径的撤销端口（`host::wire_keys` 的第三个参数）。
    ///
    /// 生产 GUI 在 `main.rs` 传的是 `Some(undo_port)`；这里默认 `None`，
    /// 因此"没有撤销会话的装配"照旧如实 `reject`（既有判据 16 的第 ⑥/⑦ 步不变）。
    pub undo: Option<Rc<yeban_app::undo::UndoPort>>,
}

impl Default for LiveWiringOptions {
    fn default() -> Self {
        Self {
            permission: Permission::ReadOnly,
            console_tab: 0,
            save_path: None,
            engine_quanta: 8,
            undo: None,
        }
    }
}

/// **一次心跳的读数**（[`LiveUi::engine_heartbeat`] 的返回值）。
///
/// 它是生产 `run_gui` 的 60Hz 定时器那一跳的**逐字复制**：先按需增量发布快照，再回收
/// （`SnapshotSlot::prune` + `RetireQueue::drain`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineTick {
    /// 这一跳真的发布了新快照时是它的版本号；标记没变（或没有工程可投影）时是 `None`。
    pub published_revision: Option<u64>,
    /// 回收读数（见 [`HeartbeatReadings`]）。
    pub readings: HeartbeatReadings,
}

/// 接线过程可能出的错（投影层 / 注册表适配 / Tier-1 渲染层 / 引擎宿主）。
#[derive(Debug)]
pub enum LiveWiringError {
    /// 工程投影失败（`[MODEL-AST-002]` 的投影层；溢出/非法拍号等）。
    Project(BridgeError),
    /// 语义注册表 → 控件树失败（角色取值或 ID 不在 `KNOWN_ROLES` / 格式非法）。
    Tree(TreeError),
    /// Tier-1 渲染 / 平台 / 组件构造失败。
    Render(RenderError),
    /// 直接抓帧失败（`[MUST-GATE-015]`：尺寸为零或全黑都算失败）。
    Capture(PortError),
    /// 引擎重建失败（`ui/reload_engine`；没有主总线 / 路由图成环 / 模型校验失败）。
    Engine(EngineHostError),
    /// 权威会话**没有活跃工程**（`yeban_close_project` 之后），因此没有可投影的东西。
    ///
    /// 只可能出现在 `build_live_ui_from_authority`（选项 (a) 的装配入口）：
    /// 其余入口的工程由调用方给出，不存在"取不到"。
    #[cfg(feature = "in-process-mcp")]
    NoActiveAuthorityProject,
}

impl core::fmt::Display for LiveWiringError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Project(error) => write!(f, "工程投影失败: {error}"),
            Self::Tree(error) => write!(f, "语义注册表适配失败: {error}"),
            Self::Render(error) => write!(f, "Tier-1 执行面装配失败: {error}"),
            Self::Capture(error) => write!(f, "Tier-1 抓帧失败: {error}"),
            Self::Engine(error) => write!(f, "引擎重建失败: {error}"),
            #[cfg(feature = "in-process-mcp")]
            Self::NoActiveAuthorityProject => {
                f.write_str("权威会话没有活跃工程（`yeban_close_project` 之后），没有可投影的东西")
            }
        }
    }
}

impl core::error::Error for LiveWiringError {}

impl From<BridgeError> for LiveWiringError {
    fn from(value: BridgeError) -> Self {
        Self::Project(value)
    }
}

impl From<TreeError> for LiveWiringError {
    fn from(value: TreeError) -> Self {
        Self::Tree(value)
    }
}

impl From<RenderError> for LiveWiringError {
    fn from(value: RenderError) -> Self {
        Self::Render(value)
    }
}

impl From<PortError> for LiveWiringError {
    fn from(value: PortError) -> Self {
        Self::Capture(value)
    }
}

impl From<EngineHostError> for LiveWiringError {
    fn from(value: EngineHostError) -> Self {
        Self::Engine(value)
    }
}

/// 一次**权威投影刷新**的读数（[`LiveUi::sync_authority`] 的返回值）。
///
/// 为什么做成枚举而不是 `bool`：三种结局在报告里必须能分开 ——
/// "什么都没发生"与"会话被关掉了"是**两件**不同的事，混成一个 `false` 会让
/// 判据只能断言"没刷新"，而分不清"刷新逻辑没跑"与"没有工程可刷"。
#[cfg(feature = "in-process-mcp")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthoritySync {
    /// 权威的施加修订号没动 ⇒ 什么都没做（这是**常态**：只读调用不推进修订号）。
    Unchanged,
    /// 权威上有**新的**工程，已经重新投影并注入同一个活窗口。
    Reprojected {
        /// 刷新后权威的施加修订号。
        revision: u64,
    },
    /// 权威的会话**没有活跃工程** ⇒ 投影**不动**（界面保留最后一次投影）。
    ///
    /// 如实登记：当前界面没有"没有工程"的表示（空工程是另一回事），
    /// 因此这里不假装把界面清空。
    NoActiveProject {
        /// 权威当前的施加修订号。
        revision: u64,
    },
}

/// **管理动作的真正落地 + 电平消费** —— 包住 `PortAdapter<LivePort<MainWindow>>`。
///
/// 它只覆写四个 `*_impl`（`[UI-MCP-001]` §12.3 的 Administrative 一级），其余全部原样
/// 委托给内层执行面。**不**改 `PortAdapter` / `LivePort` / `yeban-ui-test-port` 的公开 API：
/// 那三个 crate 不是本线的地盘，而且"给一个测试端口加产品语义"会污染它们的依赖方向。
///
/// 四个动作各自的可观测副作用：
///
/// | 动作 | 真的做了什么 | 界面/接口上能看到什么 |
/// | :--- | :--- | :--- |
/// | `ui/switch_main_view` | `MainWindow.arrangement-view = view == "arrangement"` | 重抓控件树后 `workspace-session-canvas` / `workspace-arrangement-canvas` 互换；回执里带回读值 |
/// | `ui/force_save` | `save::save_project_file`（临时文件 → `sync_all` → 原子重命名） | 磁盘上真的出现可被 `open_project_file` 读回的容器；回执里有 `bytes` / `saveEpoch` |
/// | `ui/reload_engine` | `EngineHost::reload`（新快照 + 新队列 + 推 N 个量子） | **新**的电平队列被采纳 ⇒ 混音台电平回到下限；回执里有 `generation` / `quanta` / `meterFrames` |
/// | `ui/open_project` | `open::open_project_file`（**同一个** CLI `--open` 入口）＋ [`Self::apply_project`]（投影 → 注册表 → 电平 → 引擎换代） | 同一个活窗口上换成新工程：语义 ID 族、行几何、走带、电平长度全部跟着换；回执里有 `tracks` / `rows` / `meterRows`（三个数等长 ⇒ 无串轨） |
///
/// ## `[UI-A11Y-002]` 的 `is_composing` 防护住在哪里（本文件与 `input.rs` 的分工）
///
/// | 问题 | 答案 |
/// | :--- | :--- |
/// | 状态机在哪 | `yeban_app::input::InputContext`（`src/input.rs`，纯 Rust、22 条判据） |
/// | 本文件持有什么 | **同一个** `Rc<RefCell<InputContext>>`（与 `window.clone_strong()` 同构：共享，不是复制） |
/// | 谁观测它 | `UiSurface::ime_state` → `ui/property {"name":"isComposing"}`（`[UI-A11Y-002]`） |
/// | 谁用它做判断 | `UiSurface::preview_effect` 的按键分支：`InputContext::resolve` 回答"这一键会被怎么处置" |
/// | 生产驱动点 | Slint 平台 IME 事件 → [`LiveUi::input_context`] → `begin_composition` / `end_composition` / `set_focus` |
///
/// **不做什么（刻意的）**：本文件**不**改 `dispatch_key_press` 的分发行为 ——
/// §7.2 的"彻底拦截冒泡分发"约束的是 **Slint 控件层**（真实用户的按键），
/// 而 UI 测试端口注入的按键本来就是拿来**验证**那条防护的（注入被吞掉就测不了了）。
/// 注入路径上 IME 状态的正确用法是**先问后做**：`dryRun` 回报"这一键会被输入法吞掉"。
struct LiveAdminSurface {
    inner: LiveSurface,
    /// 活窗口的强引用（`clone_strong`）。改视图 / 注入电平 / 重抓树都走它。
    window: MainWindow,
    /// 静态语义注册表（动态区标记的来源，重抓树时要重新注入）。
    registry: ControlTree,
    /// 当前工程的**投影缓存**（与它的 `ViewState` 一起换）。
    ///
    /// 它**不是**权威：`ROAD-M4-008` 选项 (a) 之后，权威是 `Domain` 会话
    /// （`authority` 字段），本字段只在 [`Self::apply_project`] 里被权威的工程整体换掉
    /// —— 因此"权威换了工程而界面还画着旧的那份"需要有人**跳过**这一次调用才可能发生，
    /// 而 `sync_authority` 就是那个唯一的调用点。
    project: YebanProjectV1,
    view: ViewState,
    save_path: Option<PathBuf>,
    meters: MeterRuntime,
    engine: EngineHost,
    engine_quanta: u64,
    save_epoch: u64,
    report: Option<AdminReport>,
    /// `[UI-A11Y-002]` 的 IME 状态机 —— **唯一**的一份（观测与按键预览共用它）。
    input: Rc<RefCell<InputContext>>,
    /// **唯一可变权威**的宿主句柄（`ROAD-M4-008` 选项 (a)）。
    ///
    /// `Some` ⇒ 本执行面的工程**只**从它来（装配入口是
    /// [`build_live_ui_from_authority`]，那个函数**不接受**工程参数），
    /// 下面那个 `project` 字段因此只是它的**投影缓存**，刷新由修订号驱动
    /// （[`LiveUi::sync_authority`]）；`None` ⇒ 工程由调用方给出（旧的 [`build_live_ui`] 形态）。
    #[cfg(feature = "in-process-mcp")]
    authority: Option<ProjectAuthorityHandle>,
    /// 已经把哪一版权威投影进来了（`sync_authority` 的比较基准）。
    #[cfg(feature = "in-process-mcp")]
    authority_revision: u64,
    /// 装配时交给键盘路径的撤销端口（[`LiveWiringOptions::undo`]）的**同一份** `Rc`。
    ///
    /// 本字段**不**用来做撤销动作（那是 `host::wire_keys` / `host::wire_mixer_edit` 的
    /// 闭包干的）。它存在的唯一理由在 [`Self::open_project_now`]：那个端口**自己持有一份
    /// 工程**（`UndoPort::try_project` 就是生产心跳的发布来源），因此在它挂着的时候换工程
    /// 会让端口写回旧工程 ⇒ 那种装配下 `ui/open_project` 如实**拒绝**（不留两条事实源）。
    /// `None` ⇒ 本执行面是工程的唯一持有者，换工程可以就地做。
    undo: Option<Rc<yeban_app::undo::UndoPort>>,
}

/// `yeban_app::input::Focus` → 线格式的 [`ImeFocus`]（**唯一**的映射点）。
const fn ime_focus_of(focus: Focus) -> ImeFocus {
    match focus {
        Focus::MainCanvas => ImeFocus::MainCanvas,
        Focus::TextInput => ImeFocus::TextInput,
        Focus::Other => ImeFocus::Other,
    }
}

/// `yeban_ui_test_port::port::KeyCode` → 状态机的物理键 + 修饰键。
///
/// `None` = 这个 `key_code` 在 `[UI-A11Y-001]` 的扫描码表里没有对应（例如裸修饰键
/// `Shift`/`Ctrl`）：宿主直接丢弃、不进状态机（`input.rs` 的边界），
/// 因此它的"处置"是 `pass-through`（不冒充成一个动作）。
fn physical_key_of(key: KeyCode) -> Option<(PhysicalKey, Modifiers)> {
    let bare = Modifiers::none();
    let mapped = match key {
        KeyCode::Tab => (PhysicalKey::Tab, bare),
        KeyCode::Escape => (PhysicalKey::Escape, bare),
        KeyCode::Return => (PhysicalKey::Enter, bare),
        KeyCode::Space => (PhysicalKey::Space, bare),
        KeyCode::Backspace => (PhysicalKey::Backspace, bare),
        // `Shift+Enter` 是**一个 chord**（§12.4 明文），按 Shift 按下的 Enter 解析。
        KeyCode::ShiftEnter => (PhysicalKey::Enter, Modifiers::shift()),
        KeyCode::Character(ch) => match ch.to_ascii_lowercase() {
            'b' => (PhysicalKey::KeyB, bare),
            'z' => (PhysicalKey::KeyZ, bare),
            'h' => (PhysicalKey::KeyH, bare),
            'd' => (PhysicalKey::KeyD, bare),
            'm' => (PhysicalKey::KeyM, bare),
            '[' => (PhysicalKey::BracketLeft, bare),
            ']' => (PhysicalKey::BracketRight, bare),
            digit @ '1'..='5' => (PhysicalKey::Digit(digit as u8 - b'0'), bare),
            _ => return None,
        },
        KeyCode::Shift | KeyCode::Control => return None,
    };
    Some(mapped)
}

/// 一次注入按键的**只读处置结论**（`[UI-A11Y-002]`）。
///
/// 取值来自 [`yeban_ui_mcp::ime`] 的**唯一**词表（不是在这里手写字符串）：
/// 零 Slint 假面报的 `resolution` 用的是同一批常量，因此"同一个词"不会在两个
/// 执行面上分叉。
fn key_resolution(state: &InputContext, key: KeyCode) -> &'static str {
    match physical_key_of(key) {
        Some((physical, modifiers)) => match state.resolve(physical, modifiers) {
            Resolution::ConsumedByIme => yeban_ui_mcp::ime::RESOLUTION_CONSUMED_BY_IME,
            Resolution::Action(_) => yeban_ui_mcp::ime::RESOLUTION_ACTION,
            Resolution::PassThrough => yeban_ui_mcp::ime::RESOLUTION_PASS_THROUGH,
        },
        // 不在 `[UI-A11Y-001]` 的扫描码表里 ⇒ 不归 DAW 管（窗口仍然会收到它）。
        None => yeban_ui_mcp::ime::RESOLUTION_PASS_THROUGH,
    }
}

/// `track-{i}-header`（`[UI-TEST-001]` §12.2）→ 轨道身份（26 字符 `EntityId` 文本）。
///
/// **唯一**的一处解析。为什么需要它：控制面按**语义 ID**寻址（调用方能从 `ui/tree` 发现
/// `track-0-header` 这样的名字），而每轨高度覆盖按**身份**键控（`ADR-0004` S1 /
/// `bridge::TrackHeightLayout`）⇒ 这一格转换不可省。
///
/// 解析不出来 / 下标越界 / 身份为空 ⇒ `None`（调用方报 `-32006 ELEMENT_NOT_FOUND`，
/// 而不是猜第一条轨道）。
fn track_id_of_header(element_id: &str, view: &ViewState) -> Option<String> {
    let index = element_id
        .strip_prefix("track-")?
        .strip_suffix("-header")?
        .parse::<usize>()
        .ok()?;
    view.tracks
        .get(index)
        .map(|track| track.id.clone())
        .filter(|id| !id.is_empty())
}

/// `ui/switch_main_view` 的视图名 → `arrangement-view` 布尔（**唯一**的映射点）。
///
/// 预览与真动作都走它：两处各写一遍 `match` 迟早会出现"预览说 arrangement、
/// 真做却写了 session"这种谁都没写错的漂移。
fn arrangement_view_of(view: &str) -> Option<bool> {
    match view {
        "arrangement" => Some(true),
        "session" => Some(false),
        _ => None,
    }
}

impl LiveAdminSurface {
    /// 重抓运行时控件树（几何 / 可见性变了之后必须做，否则 `ui/tree` 还是旧的）。
    ///
    /// ## 为什么这里**不**先渲染一次（**更正** `bd9fbb6` 的取舍，依据是本轮实测）
    ///
    /// `bd9fbb6` 在这里加过一次 `window().capture()`，理由是"Slint 的几何与 `visible`
    /// 是在渲染（布局）之后才更新的"⇒"改完立刻抓"可能与"抓之前截一张"不同。
    /// 那一句是**从既有判据的调用顺序推出来的**，不是实测的结论（原提交信息自己写的是
    /// "**可能**抓到上一帧的可见性"）。本轮把它量了：
    ///
    /// 1. **行为**：一次真实点击（`toggle-sidebar`，同时改**可见性**与**几何**）之后，
    ///    不经过任何渲染、只做一次全树内省，`ui/tree` 的节点数 **85→81**、
    ///    `sidebar-search-field` **在树里 → 不在树里**、`ui/node` 的
    ///    `sidebar-collapse-button.x` **210→6**（判据
    ///    `a_tree_read_sees_a_click_that_just_happened` 的字面读数）；
    /// 2. **机理**：内省读的是元素的**属性**（`i-slint-backend-testing` 的
    ///    `ElementHandle::absolute_position` / `size` → `ItemRc::geometry`，见上游
    ///    `i-slint-backend-testing-1.18.1/search_api.rs:881` 与 `:893`），而几何在生成的
    ///    代码里就是 `x` / `y` / `width` / `height` 这几个**属性**的惰性求值结果 ——
    ///    属性一读就重算，不需要一次光栅化来"冲刷布局"；
    /// 3. **代价**：`capture()` 是 1920×1080 的 Tier-1 全量软件光栅化，本机实测
    ///    **100.23 ms**；同一次重抓里的全树内省（85 个节点）只有约 **5 ms**
    ///    （`pump_meters()` 105.02 ms − `capture()` 100.23 ms）。这一步现在跑在
    ///    **每一次 `ui/*` 读**上（`refresh_runtime_tree`），留着它等于让每次读都白渲染
    ///    一张没人看的 6 MB 帧。
    ///
    /// ⚠ **未核实（如实登记）**：是否存在某种"只有渲染才 flush"的布局变化。本轮在
    /// 本仓库现有判据覆盖到的变化里**没有找到**（两种 feature 配置下全部判据绿），
    /// 但这条不是证明 —— 若将来出现，症状是"`ui/tree` 的几何/可见性落后一帧"。
    fn refresh_tree(&mut self) -> Result<usize, LiveWiringError> {
        Ok(self
            .inner
            .port_mut()
            .refresh_tree(Some(&self.registry))?
            .len())
    }

    /// 引擎重建（`ui/reload_engine` 与 `apply_project` 共用同一条路径）。
    fn rebuild_engine(&mut self) -> Result<yeban_app::engine_host::EngineRebuild, LiveWiringError> {
        self.engine
            .reload(&self.project, self.engine_quanta)
            .map_err(LiveWiringError::from)
    }

    /// **编辑 ⇒ 发声**的一跳：按需增量发布快照 ＋ 回收（生产 60Hz 心跳的三步）。
    ///
    /// 顺序是契约（与 `src/main.rs` 的 `run_gui` 逐字相同）：
    /// 1. 标记与"当前快照已经反映的"相同 ⇒ **不发布**（也不克隆工程）；
    /// 2. 不同 ⇒ `EngineHost::publish_project` 增量发布（**不是** `reload`：它不换代、
    ///    不动 `quanta` 契约）；
    /// 3. 无论发没发 ⇒ `EngineHost::heartbeat`：`prune` + `drain`。
    ///
    /// ⛔ ②与③必须在**同一条调用序列**里：只发布不回收会留下"只涨不回收"的慢泄漏。
    fn engine_tick(&mut self, mark: EditMark, project: Option<&YebanProjectV1>) -> EngineTick {
        let mut published_revision = None;
        if self.engine.has_engine()
            && self.engine.should_publish(mark)
            && let Some(project) = project
        {
            published_revision = self.engine.publish_project(project, mark).ok();
        }
        EngineTick {
            published_revision,
            readings: self.engine.heartbeat(),
        }
    }

    /// 驱动音频侧 `quanta` 个量子（**不**排空退役队列）—— 设备回调那一侧的替身。
    ///
    /// 本机没有 cpal 回调线程（AGENTS.md §5），判据用它把"读者换了快照、旧快照进了
    /// 退役队列"这个窗口造出来，再由**心跳**回收。
    fn drive_audio(&mut self, quanta: u64) -> u64 {
        self.engine.drive_audio(quanta)
    }

    /// 一轮电平消费：**生产那一份**（[`host::pump_meters`]）＋ 重抓树。
    ///
    /// ## 为什么这里不再自己 `poll` / `snapshot` / `apply_meters`（2026-10-08 收敛）
    ///
    /// 这一份原先是"抽干 ＋ 应用"的**第二份**实现，而它与生产那一份**不等价**：
    /// [`host::pump_meters`] 在注入之前比"界面 `track-names` 行数 vs 投影轨道数"，
    /// 不等长就**跳过注入**并回报 [`host::MeterPump::LengthMismatch`]；这一份**没有那道闸**，
    /// 无条件注入。⇒ 无头 Tier-1 判据面（走本文件）与产品路径跑的是**两条腿**，
    /// 判据面因此可能看不见产品路径上的真实行为（这个缺口登记在
    /// `docs/ledger/feature-alignment.md` 的"60Hz 主线程电平心跳"行）。现在两处
    /// **共用同一个函数**：闸、抽干顺序、返回值形状不可能再漂移。
    ///
    /// **顺序是契约**（与生产同一条）：抽干 → 对齐 → 注入 → 重抓树。
    /// 注入必须发生在重抓树之前，否则 `ui/tree` 读到的是上一帧的标签
    /// （判据 `mixer_meter_labels_match_the_injected_frames` 就是靠这一点）。
    /// 重抓树是**本执行面独有**的一步（生产 GUI 没有内省树），因此留在这一层 ——
    /// 长度不等长时**也要**重抓：界面此刻是对的（[`host::apply_view`] 已经按当前投影
    /// 重置了电平数组），树只是要跟上界面。
    fn pump_meters(&mut self) -> host::MeterPump {
        let pump = host::pump_meters(&self.window, &mut self.meters, &self.view);
        // 电平标签变了 ⇒ 控件树必须重抓（失败也不该让一次电平刷新变成错误：
        // 界面已经是新值，重抓失败只是"控制面看到的树旧了一点"）。
        let _ = self.refresh_tree();
        pump
    }

    /// 换一个工程：投影 → 注入 → 换注册表 → 作废旧电平 → （若引擎已起）换代 → 重抓树。
    ///
    /// 它与 `main.rs` 的 `--project-sample` 走的是**同一条**数据流
    /// （`ViewState::from_project` → `host::apply_view`），只是发生在**已存在的窗口**上。
    /// 因此"换工程 ⇒ 换界面"这件事在同一个活窗口上可判据（不需要第二个窗口，
    /// 也就不需要第二个 Tier-1 平台 —— 那是 `set_platform` 每线程一次的限制）。
    ///
    /// `ADR-0004` S1 / Q5：行高布局与横向缩放都是**视图态**，所以重投影要把它们读回来
    /// （`yeban_app::host::project_with_view_state`，与卷帘偏移同款"宿主拥有、重新注入"）。
    /// 不读它的话，"设置行高（或调过缩放）之后再换工程/刷新"会把它们**静默清零**。
    fn apply_project(&mut self, project: &YebanProjectV1) -> Result<(), LiveWiringError> {
        let view = yeban_app::host::project_with_view_state(project, &self.window)?;
        let registry = control_tree_from_registry(&ElementRegistry::from_view(&view))?;
        host::apply_view(
            &self.window,
            &view,
            slint::ComponentHandle::window(&self.window).size().width as f32,
            0.0,
        );
        self.project = project.clone();
        self.view = view;
        self.registry = registry;
        // 旧读数不再代表任何东西：清空面板，并把"等长静音"写进界面。
        self.meters = MeterRuntime::empty();
        self.save_epoch = 0;
        self.report = None;
        if self.engine.has_engine() {
            let rebuild = self.rebuild_engine()?;
            self.meters.adopt(rebuild.collector);
        }
        self.pump_meters();
        Ok(())
    }

    /// 把这个执行面接到**唯一可变权威**上（`ROAD-M4-008` 选项 (a)）。
    ///
    /// 只记下句柄与"当前已经投影过哪一版"，**不**改工程 —— 投影由
    /// [`Self::sync_authority`] 做。装配入口 [`build_live_ui_from_authority`]
    /// 先按权威的工程把窗口建出来，再调这里。
    #[cfg(feature = "in-process-mcp")]
    fn attach_authority(&mut self, authority: &ProjectAuthorityHandle) {
        self.authority = Some(authority.clone());
        self.authority_revision = authority.apply_revision();
    }

    /// 依**权威的施加修订号**刷新投影（`ROAD-M4-008` 选项 (a) 的刷新点）。
    ///
    /// 顺序是契约，三条都在这里：
    /// 1. 读权威的修订号；与"已经投影过的"相同 ⇒ [`AuthoritySync::Unchanged`]（**常态**：
    ///    一次只读工具调用不推进修订号，因此界面不会被查询刷来刷去）；
    /// 2. 前进 ⇒ 取权威工程的一份快照，走**与装配同一条**数据流
    ///    （[`Self::apply_project`]：`ViewState::from_project` → `host::apply_view` →
    ///    重建注册表 → 引擎换代 → 重抓树）；
    /// 3. **只有投影成功之后**才记下新修订号：失败会让下一次重试（响亮），
    ///    而不是把"已经投影过了"记成一句不成立的话。
    ///
    /// # Errors
    ///
    /// 投影 / 注册表适配 / 引擎换代失败（[`LiveWiringError`]）。
    #[cfg(feature = "in-process-mcp")]
    fn sync_authority(&mut self) -> Result<AuthoritySync, LiveWiringError> {
        let Some(authority) = self.authority.clone() else {
            return Ok(AuthoritySync::Unchanged);
        };
        let revision = authority.apply_revision();
        if revision == self.authority_revision {
            return Ok(AuthoritySync::Unchanged);
        }
        let Some(project) = authority.project() else {
            return Ok(AuthoritySync::NoActiveProject { revision });
        };
        self.apply_project(&project)?;
        self.authority_revision = revision;
        Ok(AuthoritySync::Reprojected { revision })
    }

    /// `ui/switch_main_view`：**真的**改 `MainWindow.arrangement-view` 并回读。
    ///
    /// 名字与 trait 方法（`UiTestPort::switch_main_view_impl`）刻意不同：trait 里那一份
    /// 只是三行委托，语义主体在这里 —— 同名会让"哪一份在跑"变成读者要猜的事。
    fn apply_main_view(&mut self, view: &str) -> Result<(), PortError> {
        let Some(arrangement) = arrangement_view_of(view) else {
            // 参数白名单（`methods::VIEW`）已经拦下非法值（`-32602`）；
            // 这里是**纵深防御**：执行面自己也不接受没定义过的视图名。
            return Err(PortError::Rejected {
                message: format!("未知主视图 `{view}`（合法值: arrangement, session）"),
            });
        };
        self.window.set_arrangement_view(arrangement);
        let nodes = self.refresh_tree().map_err(wiring_rejected)?;
        // 回读（不是回显入参）：证明属性**真的**被写进去了。
        let read_back = self.window.get_arrangement_view();
        self.report = Some(AdminReport::new(
            "switch_main_view",
            vec![
                ("view", ReportValue::Text(view.to_owned())),
                ("arrangementView", ReportValue::Bool(read_back)),
                (
                    "consoleTab",
                    ReportValue::Int(i64::from(self.window.get_console_tab())),
                ),
                ("treeNodes", ReportValue::Uint(nodes as u64)),
            ],
        ));
        Ok(())
    }

    /// `ui/set_track_height`：**真的**改行高视图态并重新投影，回执里给出回读的几何。
    ///
    /// ## 顺序是契约（与 `apply_project` 同一条数据流）
    ///
    /// 1. 按 §12.2 的语义 ID（`track-{i}-header`）解析出**轨道身份**（每轨高度覆盖按身份键控）；
    /// 2. 走**唯一**的 setter `yeban_app::host::set_track_height_override` 写视图态
    ///    （`height_px == 0` = 取消这条覆盖，与那个 setter 的契约逐字相同）；
    /// 3. 读回视图态重投影（不读回来就会把刚设的高度静默清零）；
    /// 4. 换注册表 → 注入 → `pump_meters`（**先注入、后重抓树**：顺序是契约）；
    /// 5. 回执里给出 `changed` 与回读的**矩形高**，服务层原样放进 `result.report`。
    ///
    /// 它**不**碰工程、不落盘、不过引擎：行高是视图态（`ADR-0004` S1 / Q4-A，零 schema）。
    fn apply_track_height(&mut self, element_id: &str, height_px: u32) -> Result<(), PortError> {
        let track_id = track_id_of_header(element_id, &self.view).ok_or_else(|| {
            PortError::UnknownElement {
                id: element_id.to_owned(),
            }
        })?;
        let changed =
            yeban_app::host::set_track_height_override(&self.window, &track_id, height_px);
        // 行高与横向缩放都从窗口读回来（`project_with_view_state`）：改行高不该把缩放弄丢。
        let view = yeban_app::host::project_with_view_state(&self.project, &self.window).map_err(
            |error| PortError::Rejected {
                message: format!("轨道高度重投影失败: {error}"),
            },
        )?;
        let registry =
            control_tree_from_registry(&ElementRegistry::from_view(&view)).map_err(|error| {
                PortError::Rejected {
                    message: error.to_string(),
                }
            })?;
        let width = slint::ComponentHandle::window(&self.window).size().width as f32;
        // 卷帘偏移原样保留：改行高不该把别的视图态弄丢。
        let scroll_x = self.window.get_roll_scroll_x();
        host::apply_view(&self.window, &view, width, scroll_x);
        // 回读（不是回显入参）：`track-heights` 注入的是**矩形高**（行槽高 − 2px 间隙）。
        let drawn_px = view
            .tracks
            .iter()
            .find(|track| track.id == track_id)
            .map_or(0.0_f32, |track| track.height);
        self.view = view;
        self.registry = registry;
        // 电平对齐 + 重抓树（顺序是契约：注入必须在重抓之前）。
        self.pump_meters();
        self.report = Some(AdminReport::new(
            "set_track_height",
            vec![
                ("elementId", ReportValue::Text(element_id.to_owned())),
                ("trackId", ReportValue::Text(track_id)),
                ("heightPx", ReportValue::Uint(u64::from(height_px))),
                ("changed", ReportValue::Bool(changed)),
                ("drawnPx", ReportValue::Float(f64::from(drawn_px))),
            ],
        ));
        Ok(())
    }

    /// `ui/force_save`：**真的**写盘（临时文件 → `sync_all` → 原子重命名）。
    ///
    /// ## 谁是写者（`ROAD-M4-008` 选项 (a)：单一写者会话）
    ///
    /// | 本执行面有没有权威句柄 | 落点 | 为什么 |
    /// | :--- | :--- | :--- |
    /// | **有**（[`build_live_ui_from_authority`] 装配的） | [`ProjectAuthorityHandle::save_to`] —— 控制面会话自己产出字节并写盘 | 挂在控制面上时，那个会话是这份文档的**唯一**磁盘写者；GUI 的本地保存路径（[`save_project_file`]）在会话持锁期间**必然被拒**（判据 ④），所以它**不能**是第二条写路径 |
    /// | **没有**（[`build_live_ui`] / [`build_live_ui_with`] 装配的） | [`save_project_file`]（本地原子写，取排他写建议锁） | 没有控制面时进程里只有它一个写者，本地路径就是那条路 |
    ///
    /// 两条路**不会**同时发生：有权威时**不**调用本地路径，权威拒绝时**不**回退到本地路径
    /// （回退会把"权威说不能写"变成一个偷偷写盘的分支）。因此"保存"这个动作在两种装配下
    /// 各自只有一个落点，且挂载时那个落点是会话。
    fn save_now(&mut self) -> Result<(), PortError> {
        let Some(path) = self.save_path.clone() else {
            // 没有配置落点 = 这条能力在**这个装配上**没接线 ⇒ `-32005` 是诚实的。
            return Err(PortError::Rejected {
                message: "未配置保存路径（`LiveWiringOptions::save_path`）；\
                          `ui/force_save` 不会假装写过一个不存在的文件"
                    .to_owned(),
            });
        };
        #[cfg(feature = "in-process-mcp")]
        if let Some(authority) = self.authority.as_ref() {
            // 保存经**权威自己的**字节（不是界面缓存）—— 因此界面缓存即使陈旧，
            // 写出来的也一定是会话当前那一版工程。
            let saved = authority.save_to(&path).map_err(|fault| PortError::Rejected {
                message: format!(
                    "权威会话拒绝保存（宿主保存动作与 `yeban_save_project` 同一个门）: {fault:?}"
                ),
            })?;
            self.save_epoch = self.save_epoch.saturating_add(1);
            self.report = Some(AdminReport::new(
                "force_save",
                vec![
                    ("saveEpoch", ReportValue::Uint(self.save_epoch)),
                    ("bytes", ReportValue::Uint(saved.bytes as u64)),
                    // 这条读数就是"保存走了权威"的可断言的证据（不是注释里的声明）。
                    ("writtenByAuthority", ReportValue::Bool(true)),
                    ("assets", ReportValue::Uint(saved.assets as u64)),
                    (
                        "historyCommits",
                        ReportValue::Uint(saved.history_commits as u64),
                    ),
                ],
            ));
            return Ok(());
        }
        let saved = save_project_file(&self.project, &path).map_err(|error| match error {
            SaveError::Container(inner) => PortError::Rejected {
                message: format!("容器写出被拒绝: {inner}"),
            },
            other => PortError::Rejected {
                message: other.to_string(),
            },
        })?;
        self.save_epoch = self.save_epoch.saturating_add(1);
        self.report = Some(AdminReport::new(
            "force_save",
            vec![
                ("saveEpoch", ReportValue::Uint(self.save_epoch)),
                ("bytes", ReportValue::Uint(saved.bytes as u64)),
                // 没有权威 ⇒ 本地原子写（与历史行为逐字相同）。
                ("writtenByAuthority", ReportValue::Bool(false)),
                // 容器布局：`project.json` + `history.dag`（空图谱），见 `crate::save` 的边界。
                ("containerEntries", ReportValue::Uint(2)),
            ],
        ));
        Ok(())
    }

    /// `saveFirst` 为真、而当前工程**没有落点**时的那一句话。
    ///
    /// **唯一**一处：`open_project_now`（真调用）与 `preview_effect`（`dryRun`）都调它，
    /// 因此"预览报的失败"与"真调用报的失败"是同一句话（`ADR-0001` D48 的口径）。
    fn no_save_target_for_open() -> PortError {
        PortError::Rejected {
            message: "`saveFirst` 为真但当前工程没有落点（`LiveWiringOptions::save_path`）\
                      ⇒ 拒绝静默丢弃；要丢弃就给 `saveFirst: false`"
                .to_owned(),
        }
    }

    /// `ui/open_project`：在**这个活窗口**上换一份当前工程（CLI `--open` 的控制面孪生）。
    ///
    /// ## 权威（`ADR-0005`：**不造第二个权威**）
    ///
    /// 「打开」只有一条实现：[`yeban_app::open::open_project_file`] —— 与 CLI `--open` 与
    /// `--headless` 用的是**同一个**入口（`crates/yeban-app/src/main.rs` 的
    /// `cli::load_project` 最终落到它）。「注入」也只有一条：[`Self::apply_project`]
    /// （`ViewState` → `host::apply_view` → 换注册表 → 作废旧电平 → `EngineHost::reload`
    /// 换代 → `pump_meters`）—— 与权威重投影 [`Self::sync_authority`] 逐字同一条数据流。
    /// 本方法**不**新增打开实现、**不**新增注入点。
    ///
    /// ## 工程**别的持有者**在场时：拒绝（不留两条事实源）
    ///
    /// | 本执行面持有 | 为什么拒绝 |
    /// | :--- | :--- |
    /// | `authority`（进程内控制面会话） | 工程的唯一可变权威是那个 `Domain`（`ADR-0005`）。在这里换一份会让"界面画的是哪一份"与"会话读写的是哪一份"立刻分叉；而宿主的写入口 `HostAction` 只有 `Undo` / `Redo` / `Commit` —— **没有**"打开工程"这一项（`crates/yeban-mcp/src/domain/mod.rs:669`）⇒ 本执行面无法把这次打开交给权威 |
    /// | `undo`（外部撤销端口） | 那个端口**自己持有一份工程**（`UndoPort::try_project` 是生产心跳的发布来源）⇒ 换工程而不换端口会让端口把旧工程写回引擎。而「打开一个工程 = **新会话**」今天只有一条实现（`crates/yeban-app/src/main.rs:447` 的 `open_undo_session`，它造的是 GUI **自己**那一份会话），本执行面拿不到调用方手里的那个端口 ⇒ 如实拒绝，而不是换一半 |
    ///
    /// 两种情况都用**既有**的 `PortError::Rejected` ⇒ 服务层既有的 `-32005 NOT_IMPLEMENTED`
    /// （`ADR-0001` D25：不发明新错误码），消息点名是哪一个持有者。
    /// 产品形态 `--enable-ui-mcp-http` 恰好两种持有者都没有 ⇒ 那里本方法可用。
    ///
    /// ## 顺序是契约（**失败时当前工程一位不动**）
    ///
    /// 1. 先读**新**文件 —— 读失败（不存在 / 不是容器 / 容器坏了 / 超上限）⇒ 立即返回，
    ///    当前工程、窗口、电平、引擎**一个都没碰**；
    /// 2. `save_first` 为真 ⇒ 先保存**当前**工程（与 `ui/force_save` **同一个**落点、
    ///    同一套锁语义）；当前工程没有落点 ⇒ 拒绝并把选择权交回调用方
    ///    （`saveFirst: false` = 明确同意丢弃）。默认 `true` 与 `yeban_close_project` 的
    ///    `saveFirst` 同义（`crates/yeban-mcp/src/domain/mod.rs:1813`）；
    /// 3. 才换投影（[`Self::apply_project`]）；
    /// 4. 落点跟着新工程走 ⇒ 下一次 `ui/force_save` 写的是**新**文件
    ///    （否则新内容会被写进旧工程的文件里）。
    ///
    /// ⚠ 如实登记的边界：若第 3 步在 `host::apply_view` **之后**失败（引擎换代失败），
    /// 窗口已经是新工程而引擎还是旧一代 —— 这是既有的 [`Self::apply_project`] 语义
    /// （`sync_authority` 那条路同款），因此本方法与权威重投影**同一条**边界，不是新增的。
    fn open_project_now(&mut self, path: &str, save_first: bool) -> Result<(), PortError> {
        #[cfg(feature = "in-process-mcp")]
        if self.authority.is_some() {
            return Err(PortError::Rejected {
                message: "本执行面的工程来自进程内控制面会话（唯一可变权威）⇒ `ui/open_project` \
                          不在这里换工程（宿主的写入口没有『打开工程』这一项）；\
                          请用领域 MCP 的 `yeban_close_project` / `yeban_open_project`"
                    .to_owned(),
            });
        }
        if self.undo.is_some() {
            return Err(PortError::Rejected {
                message: "本执行面挂着外部的撤销端口（它自己持有一份工程）⇒ 在这里换工程会让它\
                          把旧工程写回引擎。「打开一个工程 = 新会话」今天只有\
                          `crates/yeban-app/src/main.rs:447`（`open_undo_session`）那一条实现，\
                          而本执行面拿不到那个端口 ⇒ 如实拒绝，而不是换一半；\
                          请用**没有**撤销端口的装配（产品形态 `--enable-ui-mcp-http` 就是这一种）"
                    .to_owned(),
            });
        }
        // ① 权威打开路径。读失败 ⇒ 到这里就返回：下面一个字节都没改。
        let project =
            yeban_app::open::open_project_file(path).map_err(|error| PortError::Rejected {
                message: format!("打开 `{path}` 失败（当前工程一位没动）: {error}"),
            })?;
        // ② 先保存当前工程（与 `ui/force_save` 同一个落点）。
        //
        // `saveFirst` 要保存而没有落点 ⇒ 拒绝，并把选择权交回调用方。这一句与
        // `preview_effect` **共用同一个构造函数**：D48 要求 `dryRun` 报的失败与真调用
        // 报的失败是同一句话，两处各写一遍迟早会漂移。
        if save_first && self.save_path.is_none() {
            return Err(Self::no_save_target_for_open());
        }
        if save_first {
            self.save_now()?;
        }
        // ③ 唯一注入路径。
        self.apply_project(&project).map_err(wiring_rejected)?;
        // ④ 落点跟着新工程走。
        self.save_path = Some(PathBuf::from(path));
        // 三个**回读**读数（不是回显入参，也不是"我知道它应该是几"）：
        // `tracks` = 手里那份投影的轨道数（投影的权威形状）；
        // `rows` = 界面 `track-names` 的行数；
        // `meterRows` = 界面电平数组的长度。
        // 三者等长正是"没有串轨"的可断言形态（`host::apply_meters` 的 `debug_assert_eq!` 钉的
        // 就是前两者；`.slint` 按下标取，不等长会把 A 轨的电平画到 B 轨上）。
        let tracks = self.view.tracks.len() as u64;
        let rows = slint::Model::row_count(&self.window.get_track_names()) as u64;
        let meter_rows = slint::Model::row_count(&self.window.get_track_meter_levels()) as u64;
        self.report = Some(AdminReport::new(
            "open_project",
            vec![
                ("path", ReportValue::Text(path.to_owned())),
                ("savedFirst", ReportValue::Bool(save_first)),
                ("tracks", ReportValue::Uint(tracks)),
                ("rows", ReportValue::Uint(rows)),
                ("meterRows", ReportValue::Uint(meter_rows)),
                ("saveTarget", ReportValue::Text(path.to_owned())),
            ],
        ));
        Ok(())
    }

    /// `ui/reload_engine`：**真的**重建引擎（新快照 / 新队列 / 推 N 个量子）并交还消费端。
    fn reload_engine_now(&mut self) -> Result<(), PortError> {
        let rebuild = self.rebuild_engine().map_err(wiring_rejected)?;
        let yeban_app::engine_host::EngineRebuild {
            generation,
            revision,
            tracks,
            quanta,
            meter_bulk_publishes,
            meter_frames,
            collector,
            ..
        } = rebuild;
        // 新引擎 ⇒ 旧读数作废；新队列的消费端交给 UI 线程（**唯一**的生产者-消费者关系）。
        self.meters.adopt(collector);
        let pump = self.pump_meters();
        // ⚠ 长度契约不成立时（这一跳刚 `adopt` 了新队列、投影没换 ⇒ 实际到不了）
        // `pump_meters` 与生产路径一样**不注入** ⇒ 界面上**没有**本代引擎的读数。
        // 两个"可见"读数因此如实报 0（与"什么都没抽到"同一个值），而不是把面板里的数
        // 当成界面上的数。
        let (visible_quantum, visible_nodes) = match &pump {
            host::MeterPump::Applied(snapshot) => {
                (snapshot.quantum.unwrap_or(0), snapshot.nodes_seen as u64)
            }
            host::MeterPump::NoEngine | host::MeterPump::LengthMismatch { .. } => (0, 0),
        };
        self.report = Some(AdminReport::new(
            "reload_engine",
            vec![
                ("generation", ReportValue::Uint(generation)),
                ("revision", ReportValue::Uint(revision)),
                ("tracks", ReportValue::Uint(tracks as u64)),
                ("quanta", ReportValue::Uint(quanta)),
                (
                    "meterBulkPublishes",
                    ReportValue::Uint(meter_bulk_publishes),
                ),
                ("meterFrames", ReportValue::Uint(meter_frames)),
                // 新引擎第一次抽帧的量子号（0 = 什么都没抽到，或这一跳没注入）。
                ("visibleQuantum", ReportValue::Uint(visible_quantum)),
                ("visibleNodes", ReportValue::Uint(visible_nodes)),
            ],
        ));
        Ok(())
    }
}

/// `LiveWiringError` → 端口错误（管理动作的失败路径）。
///
/// 注：`PortError` 没有"动作已接线但失败"这一档，因此这里落到 `Rejected`（⇒ `-32005`
/// 带**精确**的消息）。这是已知的映射收窄，登记在 `docs/ledger/app-mixer-notes.md` 的 needs。
fn wiring_rejected(error: LiveWiringError) -> PortError {
    PortError::Rejected {
        message: error.to_string(),
    }
}

impl UiTestPort for LiveAdminSurface {
    fn permission(&self) -> Permission {
        self.inner.permission()
    }

    fn tree(&self) -> &ControlTree {
        self.inner.tree()
    }

    fn capture_png(&self) -> Result<Vec<u8>, PortError> {
        self.inner.capture_png()
    }

    fn read_property(&self, element_id: &str, name: &str) -> Result<String, PortError> {
        self.inner.read_property(element_id, name)
    }

    fn dispatch_pointer_down_impl(
        &mut self,
        element_id: &str,
        x_offset: f64,
        y_offset: f64,
        button: PointerButton,
    ) -> Result<(), PortError> {
        self.inner
            .dispatch_pointer_down_impl(element_id, x_offset, y_offset, button)
    }

    fn dispatch_pointer_move_impl(&mut self, x: f64, y: f64) -> Result<(), PortError> {
        self.inner.dispatch_pointer_move_impl(x, y)
    }

    fn dispatch_pointer_up_impl(&mut self, button: PointerButton) -> Result<(), PortError> {
        self.inner.dispatch_pointer_up_impl(button)
    }

    fn dispatch_key_press_impl(
        &mut self,
        key: yeban_ui_test_port::port::KeyCode,
    ) -> Result<(), PortError> {
        self.inner.dispatch_key_press_impl(key)
    }

    fn switch_main_view_impl(&mut self, view: &str) -> Result<(), PortError> {
        self.apply_main_view(view)
    }

    fn force_save_impl(&mut self) -> Result<(), PortError> {
        self.save_now()
    }

    fn reload_engine_impl(&mut self) -> Result<(), PortError> {
        self.reload_engine_now()
    }

    /// `ui/set_track_height` 的**真实载体**（`ADR-0004` S1 的纵向入口）。
    ///
    /// 语义主体在 [`Self::apply_track_height`]（与 `switch_main_view_impl` →
    /// `apply_main_view` 同款分工：trait 里这一份只是委托，读代码的人不该猜哪一份在跑）。
    fn set_track_height_impl(&mut self, element_id: &str, height_px: u32) -> Result<(), PortError> {
        self.apply_track_height(element_id, height_px)
    }

    /// `ui/open_project` 的**真实载体**（CLI `--open` 在控制面上的孪生）。
    ///
    /// 语义主体在 [`Self::open_project_now`]（同上：trait 里这一份只是委托）。
    fn open_project_impl(&mut self, path: &str, save_first: bool) -> Result<(), PortError> {
        self.open_project_now(path, save_first)
    }
}

impl UiSurface for LiveAdminSurface {
    fn surface_name(&self) -> &'static str {
        self.inner.surface_name()
    }

    fn capture_image(&self) -> Result<Rgb8Image, PortError> {
        self.inner.capture_image()
    }

    /// 读方法在返回前调它：把控制面的运行时树**重抓到当下**（`ui/tree` / `ui/node` /
    /// `ui/property` / `ui/dynamic_regions` / `ui/coverage` / `ui/screenshot` 都走这里）。
    ///
    /// 语义主体是 [`Self::refresh_tree`]（先 `capture()` 再全树内省 —— 几何与 `visible`
    /// 在渲染之后才更新）。注册表必须传**自己那一份**：`apply_project` 会把它和投影一起换掉
    /// （见 [`LiveUi`] 的文档：注册表**不存第二份**）。
    ///
    /// 失败时报 `Rejected` ⇒ 服务层在只读上下文里映射成既有的 `-32602`
    /// （`D25`：不新增错误码），**不回退到旧缓存**。
    fn refresh_runtime_tree(&mut self) -> Result<(), PortError> {
        self.refresh_tree().map(|_| ()).map_err(wiring_rejected)
    }

    fn take_admin_report(&mut self) -> Option<AdminReport> {
        self.report.take()
    }

    /// `[UI-A11Y-002]`：读的是 `LiveAdminSurface` 持有的**那一个** `InputContext`，
    /// 不是新造的影子变量（`Rc` 共享，见本文件的接线表）。
    fn ime_state(&self) -> Option<ImeState> {
        let input = self.input.borrow();
        Some(ImeState {
            composing: input.is_composing(),
            focus: ime_focus_of(input.focus()),
        })
    }

    /// `dryRun` 的只读影响预览（ADR-0001 **D48**）。
    ///
    /// 每一条都**只读**（`&self`）：没有 `set_arrangement_view`、没有写盘、没有 `EngineHost::reload`。
    /// `Err` 表示"这次真调用一定会失败"，且消息与对应的 `*_impl` **逐字相同** ——
    /// 领域侧的口径是"dryRun 只做参数与领域合法性校验、失败如实报"，UI 侧照做。
    fn preview_effect(
        &self,
        method: &str,
        arguments: &PreviewArguments,
    ) -> Result<Option<PreviewEffect>, PortError> {
        let effect = match method {
            yeban_ui_mcp::methods::METHOD_SWITCH_MAIN_VIEW => {
                let view = arguments.text("view").unwrap_or_default();
                let Some(arrangement) = arrangement_view_of(view) else {
                    return Err(PortError::Rejected {
                        message: format!("未知主视图 `{view}`（合法值: arrangement, session）"),
                    });
                };
                PreviewEffect::new(vec![
                    ("view", ReportValue::Text(view.to_owned())),
                    ("arrangementView", ReportValue::Bool(arrangement)),
                    // 当前读数（只读回读；判据会断言"预览说的"就是"真做之后的"）。
                    (
                        "currentArrangementView",
                        ReportValue::Bool(self.window.get_arrangement_view()),
                    ),
                ])
            }
            yeban_ui_mcp::methods::METHOD_FORCE_SAVE => {
                if self.save_path.is_none() {
                    // 与 `save_now` **同一句话**：dryRun 不许把注定失败的保存说成成功预览。
                    return Err(PortError::Rejected {
                        message: "未配置保存路径（`LiveWiringOptions::save_path`）；\
                                  `ui/force_save` 不会假装写过一个不存在的文件"
                            .to_owned(),
                    });
                }
                PreviewEffect::new(vec![
                    (
                        "saveEpoch",
                        ReportValue::Uint(self.save_epoch.saturating_add(1)),
                    ),
                    // 容器布局：`project.json` + `history.dag`（见 `crate::save` 的边界）。
                    ("containerEntries", ReportValue::Uint(2)),
                ])
            }
            yeban_ui_mcp::methods::METHOD_SET_TRACK_HEIGHT => {
                let element_id = arguments.text("elementId").unwrap_or_default();
                let Some(track_id) = track_id_of_header(element_id, &self.view) else {
                    // 语义 ID 不是轨道头 ⇒ 真调用会报 `-32006`；dryRun 也如实报"这条预测不了"。
                    return Ok(None);
                };
                let layout = yeban_app::host::track_height_layout(&self.window);
                let drawn_px = self
                    .view
                    .tracks
                    .iter()
                    .find(|track| track.id == track_id)
                    .map_or(0.0_f32, |track| track.height);
                PreviewEffect::new(vec![
                    ("trackId", ReportValue::Text(track_id.clone())),
                    (
                        "currentBasePx",
                        ReportValue::Uint(u64::from(layout.base_px(&track_id))),
                    ),
                    ("currentDrawnPx", ReportValue::Float(f64::from(drawn_px))),
                    (
                        "requestedPx",
                        ReportValue::Uint(
                            arguments
                                .number("heightPx")
                                .and_then(|value| u64::try_from(value as i64).ok())
                                .unwrap_or(0),
                        ),
                    ),
                ])
            }
            yeban_ui_mcp::methods::METHOD_RELOAD_ENGINE => PreviewEffect::new(vec![
                (
                    "generation",
                    ReportValue::Uint(self.engine.generation().saturating_add(1)),
                ),
                ("quanta", ReportValue::Uint(self.engine_quanta)),
                ("tracks", ReportValue::Uint(self.view.tracks.len() as u64)),
            ]),
            yeban_ui_mcp::methods::METHOD_OPEN_PROJECT => {
                let path = arguments.text("path").unwrap_or_default();
                let save_first = arguments.is_true("saveFirst").unwrap_or(true);
                // **真校验**（`&self`：一个状态位都不改）：
                // ① 有别的持有者 ⇒ 与真调用**同一句话**地拒绝（dryRun 不许把注定失败说成成功）；
                if self.undo.is_some() {
                    return Err(PortError::Rejected {
                        message:
                            "本执行面挂着外部的撤销端口（它自己持有一份工程）⇒ 在这里换工程会让它\
                                  把旧工程写回引擎。「打开一个工程 = 新会话」今天只有\
                                  `crates/yeban-app/src/main.rs:447`（`open_undo_session`）那一条实现，\
                                  而本执行面拿不到那个端口 ⇒ 如实拒绝，而不是换一半；\
                                  请用**没有**撤销端口的装配（产品形态 `--enable-ui-mcp-http` 就是这一种）"
                                .to_owned(),
                    });
                }
                #[cfg(feature = "in-process-mcp")]
                if self.authority.is_some() {
                    return Err(PortError::Rejected {
                        message:
                            "本执行面的工程来自进程内控制面会话（唯一可变权威）⇒ `ui/open_project` \
                                  不在这里换工程（宿主的写入口没有『打开工程』这一项）；\
                                  请用领域 MCP 的 `yeban_close_project` / `yeban_open_project`"
                                .to_owned(),
                    });
                }
                // ② 预读**新**文件（只读）—— 与真调用**同一顺序**：真调用也是先读文件、
                //    再问 `saveFirst`，因此"预览报的失败"就是"真调用会报的那个失败"。
                let opened = yeban_app::open::open_project_file(path).map_err(|error| {
                    PortError::Rejected {
                        message: format!("打开 `{path}` 失败（当前工程一位没动）: {error}"),
                    }
                })?;
                let opened_tracks = ViewState::from_project(&opened)
                    .map(|view| view.tracks.len() as u64)
                    .map_err(|error| PortError::Rejected {
                        message: format!("`{path}` 投影失败: {error}"),
                    })?;
                // ③ `saveFirst` 要保存旧工程，而没有落点 ⇒ 与真调用**同一个构造函数**
                //    （同一句话，不是"两句意思差不多的话"）。
                if save_first && self.save_path.is_none() {
                    return Err(Self::no_save_target_for_open());
                }
                PreviewEffect::new(vec![
                    ("path", ReportValue::Text(path.to_owned())),
                    ("saveFirst", ReportValue::Bool(save_first)),
                    (
                        "currentTracks",
                        ReportValue::Uint(self.view.tracks.len() as u64),
                    ),
                    ("openedTracks", ReportValue::Uint(opened_tracks)),
                    (
                        "currentSaveTarget",
                        ReportValue::Text(
                            self.save_path
                                .as_ref()
                                .map_or_else(String::new, |target| target.display().to_string()),
                        ),
                    ),
                ])
            }
            yeban_ui_mcp::methods::METHOD_DISPATCH_KEY_PRESS => {
                let raw = arguments.text("keyCode").unwrap_or_default();
                // 键名解析与真调用**同一条**（拼错的键在参数/执行面阶段就会被拒，
                // 走不到这里；这里如实报"这条注入在当前 IME/焦点状态下会被怎么处置"）。
                let Ok(key) = KeyCode::parse(raw) else {
                    return Ok(None);
                };
                let input = self.input.borrow();
                PreviewEffect::new(vec![
                    ("keyCode", ReportValue::Text(key.as_str())),
                    ("isComposing", ReportValue::Bool(input.is_composing())),
                    (
                        "focus",
                        ReportValue::Text(ime_focus_of(input.focus()).as_str().to_owned()),
                    ),
                    (
                        "resolution",
                        ReportValue::Text(key_resolution(&input, key).to_owned()),
                    ),
                ])
            }
            // 指针事件的影响只有窗口自己知道 ⇒ 如实报 `None`（服务层写成 `effect: null`），
            // 不编造一份"大概会这样"。
            _ => return Ok(None),
        };
        Ok(Some(effect))
    }
}

/// 装配好的**真实界面 + 真实执行面**。
///
/// ⚠ **投影与注册表不在这里存第二份**：它们只有 `LiveAdminSurface` 里的那一份
/// （`view` / `registry`），而 `apply_project` 会在**同一个活窗口**上把两者一起换掉。
/// 若 `LiveUi` 自己也存一份克隆，换工程之后就会留下**过期**的注册表 ——
/// `ui/coverage` 会拿"新运行时树 + 旧注册表"做双向核对，结果是一堆假 `unknown`
/// （本机探针抓不到，因为它跑的是零 Slint 的那一半；这条注释就是那次复查的产物）。
pub struct LiveUi {
    /// 装配时**直接**从活窗口抓的一帧。
    ///
    /// 它的用途只有一个，但很关键：判据可以拿它独立编码一次，断言
    /// "控制面发出去的 PNG 就是这一帧" —— 把"AI 看到的像素"钉在窗口上。
    pub reference: Rgb8Image,
    /// 真实执行面（活窗口 + Tier-1 抓帧 + 三个管理动作），已经过 `PortAdapter` 与
    /// `LiveAdminSurface` 两级升格为 `UiSurface`。
    surface: LiveAdminSurface,
    /// 外壳场景（会话运行态占位 + 视口尺寸），构造窗口时用过的那一份。
    pub scene: DemoScene,
}

impl LiveUi {
    /// 把执行面装成控制面（`ui/*` 方法）。`permission` 同时决定
    /// §12.3 的三级闸门与 §7.2 的作用域集合（映射在 `yeban_ui_mcp::live` 里，只有一处）。
    #[must_use]
    pub fn into_control_plane(self, permission: Permission) -> LiveControlPlane {
        // 投影与注册表在**把执行面移进 `Box` 之前**取出：它们只有执行面里的那一份
        // （`apply_project` 换掉的也是它）。顺序反了就是 E0382（CI 第一次抓到的就是这条）。
        let view = self.surface.view.clone();
        let registry = self.surface.registry.clone();
        // 窗口句柄也在装箱前取出：它是 `Rc` **共享**（不是第二份窗口状态），
        // 只用来读"这个窗口渲染了几帧"这个探针（见 [`LiveControlPlane::rendered_frames`]）。
        let window = self.surface.inner.port().window().clone();
        let plane = match permission {
            Permission::ReadOnly => ControlPlane::read_only(Box::new(self.surface)),
            // 交互 / 管理两级都需要测试模式：`ui:inject` 在生产模式被硬禁
            // （`yeban_mcp::security::authorize` 的第一件事），这是规范要求的。
            Permission::Interactive => ControlPlane::interactive_for_tests(Box::new(self.surface)),
            // Administrative 的三件事需要 `app:save` / `app:reload-engine` / `app:admin`；
            // 作用域集合由 `scopes_for_permission` 那张唯一的映射表给出（不在这里手写）。
            Permission::Administrative => {
                ControlPlane::administrative_for_tests(Box::new(self.surface))
            }
        };
        LiveControlPlane {
            plane,
            view,
            registry,
            scene: self.scene,
            reference: self.reference,
            window,
        }
    }

    /// **生产模式**的控制面（`--enable-ui-mcp-http` 的**产品形态**）。
    ///
    /// 与 [`Self::into_control_plane`] 的差别**只有**运行模式，但那条差别是承重的：
    /// 上面三个构造点为了让判据能注入事件而用 `RunMode::Test`（名字里的 `for_tests`
    /// 就是契约），而产品进程必须用 `RunMode::Production` —— 于是 `ui:inject` 族
    /// （`ui/dispatch_key_press` / `ui/dispatch_pointer_*`）在服务端被
    /// `yeban_mcp::security::authorize` **硬拒**（`forbidden-in-production`，
    /// 判定先于令牌校验），而 `ui/switch_main_view` / `ui/force_save` /
    /// `ui/reload_engine` 按其 `app:*` scope 授权。
    ///
    /// 作用域集合与上面同一条来源（`yeban_ui_mcp::live::production` →
    /// `scopes_for_permission`）：调用方只说"我要哪一级"，不在这里手写 scope 清单。
    #[must_use]
    pub fn into_production_control_plane(self, permission: Permission) -> LiveControlPlane {
        let view = self.surface.view.clone();
        let registry = self.surface.registry.clone();
        let window = self.surface.inner.port().window().clone();
        let plane = ControlPlane::production(permission, Box::new(self.surface));
        LiveControlPlane {
            plane,
            view,
            registry,
            scene: self.scene,
            reference: self.reference,
            window,
        }
    }

    /// 当前运行时控件树的一份快照（**就是** `ui/tree` 会服务的那棵树，同一对象克隆）。
    ///
    /// 判据用它做"同一个活窗口上换工程前后"的对照；真正的端到端断言仍然走
    /// `ui/tree` / `ui/node`（见 `tests/live_ui_mcp.rs`）。
    #[must_use]
    pub fn tree_snapshot(&self) -> ControlTree {
        self.surface.inner.tree().clone()
    }

    /// 直接从活窗口抓一帧（`[MUST-GATE-015]` 的 Tier-1 路径）。
    ///
    /// # Errors
    ///
    /// 光栅化 / 抓帧失败。
    pub fn capture(&self) -> Result<Rgb8Image, PortError> {
        self.surface.capture_image()
    }

    /// 这个活窗口**已经真的光栅化过**的帧数（**探针**，见 [`Tier1Window::rendered_frames`]）。
    ///
    /// 判据用它钉住"重抓运行时树（全树内省）不得渲染一帧"这条**成本**不变量。
    #[must_use]
    pub fn rendered_frames(&self) -> usize {
        self.surface.inner.port().window().rendered_frames()
    }

    /// 采纳一条新的电平队列（测试注入 / 引擎换代）。
    ///
    /// 生产路径上这条队列由 `EngineHost::reload` 产出；判据用它注入**已知**的
    /// `MeterFrame`，从而把"控件树里的 dBFS == 我喂进去的那一帧"变成可断言的事实。
    pub fn adopt_meter_collector(&mut self, collector: MeterCollector) {
        self.surface.meters.adopt(collector);
    }

    /// 一轮电平消费（60Hz 定时器腿）：**生产那一份**（`host::pump_meters`）＋ 重抓树。
    ///
    /// 返回值是生产的 [`host::MeterPump`]（不再是 `MeterSnapshot`）：长度契约不成立时
    /// 这一跳与生产路径**同语义** —— 抽干照做、**跳过注入**、如实回报
    /// `LengthMismatch { rows, projected }`。返回形状与生产同款，调用方因此不可能
    /// "看不见"那一跳没注入。
    pub fn pump_meters(&mut self) -> host::MeterPump {
        self.surface.pump_meters()
    }

    /// **起一代引擎**：与 `ui/reload_engine` **同一条路径**（`EngineHost::reload`），
    /// 并把新队列交给电平消费端（[`MeterRuntime::adopt`]）。
    ///
    /// 存在的理由：`ui/reload_engine` 只在控制面上可达，而控制面会**消费**执行面
    /// （`into_control_plane` 把它装箱取走）；"编辑 ⇒ 发声"的判据需要在**同一个**活窗口上
    /// 先起引擎、再动混音台、再读快照，因此这里给出同一条路径的第二个入口。
    ///
    /// # Errors
    ///
    /// 工程投影失败（例如没有主总线）—— 引擎没有被换掉（`reload` 的既有契约）。
    pub fn start_engine(&mut self) -> Result<(), LiveWiringError> {
        let rebuild = self.surface.rebuild_engine()?;
        // 新引擎 ⇒ 旧读数作废：与 `reload_engine_now` 同款，消费端交给 UI 线程。
        self.surface.meters.adopt(rebuild.collector);
        Ok(())
    }

    /// **生产心跳的一跳**（`src/main.rs` 的 60Hz 定时器调用序列的逐字复制）：
    /// 按需增量发布快照 ＋ 回收。
    ///
    /// `project` 只在标记变化时才被读取 —— 生产路径传的是 `UndoPort::try_project`（整份
    /// 工程的克隆），因此"没改就不克隆"是这一跳的成本契约。
    pub fn engine_heartbeat(
        &mut self,
        mark: EditMark,
        project: Option<&YebanProjectV1>,
    ) -> EngineTick {
        self.surface.engine_tick(mark, project)
    }

    /// 驱动音频侧 `quanta` 个量子（**不**排空退役队列）—— 设备回调那一侧的替身。
    pub fn drive_audio(&mut self, quanta: u64) -> u64 {
        self.surface.drive_audio(quanta)
    }

    /// 当前快照槽的计数（判据读 `published` / `pending_len` / `pruned`；没有引擎时 `None`）。
    #[must_use]
    pub fn engine_snapshot_counts(&self) -> Option<SnapshotCounts> {
        self.surface.engine.snapshot_counts()
    }

    /// 当前引擎快照里某轨的音量（**dB**，f32）—— 判据读回的"那一个字面字段"。
    ///
    /// 没有引擎 / 快照里没有这条轨道 ⇒ `None`（不猜、不返回 0.0）。
    #[must_use]
    pub fn engine_track_volume_db(&self, track_id: &EntityId) -> Option<f32> {
        self.surface
            .engine
            .current_snapshot()?
            .track(track_id)
            .map(|params| params.volume_db())
    }

    /// 音频读路径累计换过多少次快照（`SnapshotReader::begin_block` 里的计数）。
    #[must_use]
    pub fn engine_snapshot_switches(&self) -> Option<u64> {
        self.surface
            .engine
            .engine_stats()
            .map(|stats| stats.snapshot_switches)
    }

    /// 换一个工程（同一个活窗口上的重新投影 + 重新注入）。
    ///
    /// # Errors
    ///
    /// 投影失败 / 注册表适配失败 / 引擎换代失败。
    pub fn apply_project(&mut self, project: &YebanProjectV1) -> Result<(), LiveWiringError> {
        self.surface.apply_project(project)
    }

    /// **依唯一可变权威刷新投影**（`ROAD-M4-008` 选项 (a)）。
    ///
    /// 只在权威的**施加修订号**前进时重投影。判据因此可以这样写：
    /// "AI 经控制面改了工程" ⇒ 本方法返回 [`AuthoritySync::Reprojected`]，
    /// 而界面上的语义元素随之改变；一次只读调用 ⇒ [`AuthoritySync::Unchanged`]，
    /// 界面**一位不动**（不是"重投影了一遍但恰好相同"）。
    ///
    /// 生产 GUI 的接线点是**周期性的一跳**（与电平消费同款）：本方法幂等，
    /// 每跳调一次即可，不需要"哪条路径改了就记得通知"。
    ///
    /// # Errors
    ///
    /// 投影 / 注册表适配 / 引擎换代失败。只有 [`build_live_ui_from_authority`]
    /// 装配出来的执行面才有权威；其余装配下恒为 [`AuthoritySync::Unchanged`]。
    #[cfg(feature = "in-process-mcp")]
    pub fn sync_authority(&mut self) -> Result<AuthoritySync, LiveWiringError> {
        self.surface.sync_authority()
    }

    /// `[UI-A11Y-002]` 的 IME 状态机句柄 —— **唯一**的那一份（与执行面的观测位、
    /// 按键处置共用同一个对象；`Rc` 共享，不是复制）。
    ///
    /// 用途有两个，都不新造状态：
    /// 1. **生产驱动点**：Slint 平台的 IME 事件（`is_composing` 变化 / 焦点变化）
    ///    调 `begin_composition` / `end_composition` / `set_focus`
    ///    —— 本线已由 [`crate::host::wire_input`] 在装配时接好（`.slint` 的
    ///    `TextInput` 事件源 → 回调 → 这个对象），因此生产路径不再需要外部驱动；
    /// 2. **判据的驱动点**：需要直接读/写这一个对象时用（`tests/live_ui_mcp.rs`）。
    #[must_use]
    pub fn input_context(&self) -> Rc<RefCell<InputContext>> {
        Rc::clone(&self.surface.input)
    }

    /// 活窗口句柄（共享，不是复制：`LiveAdminSurface.window` 是 `clone_strong` 的同一份）。
    ///
    /// 判据用它**在真正的 Slint 事件源上注入**：调用 `.slint` 声明的回调
    /// （`invoke_ime_composition_changed` / `invoke_ime_focus_changed`），
    /// 而不是绕过事件源去写 [`Self::input_context`] 那个对象 —— 否则
    /// "回调 → 状态机"这一段接线就没有被判据覆盖（本线补的正是这一段）。
    #[must_use]
    pub fn ui(&self) -> &MainWindow {
        &self.surface.window
    }

    /// 在真实执行面上注入一次按键（`[UI-TEST-002]` §12.4 的 `ui/dispatch_key_press` **本体**）。
    ///
    /// 判据用它把"注入 → Slint 事件源 → `key-handler` 回调 → 宿主"这一段走全，
    /// **同时**保留 [`Self::sync_authority`] / [`Self::tree_snapshot`] 的树读数能力
    /// （`into_control_plane` 会把执行面装箱取走，两者不可兼得）。
    ///
    /// # Errors
    ///
    /// 端口拒绝（权限不足 / 未知键 / 执行面已失效）。
    pub fn dispatch_key_press(
        &mut self,
        key: yeban_ui_test_port::port::KeyCode,
    ) -> Result<(), PortError> {
        self.surface.inner.dispatch_key_press(key)
    }

    /// 在**这个活窗口**上换一份当前工程（`ui/open_project` 的**本体**，CLI `--open` 的孪生）。
    ///
    /// 存在的理由与 [`Self::dispatch_key_press`] 逐字相同：端到端判据既要走"打开 → 重投影"
    /// 这一段，又要保留 `LiveUi` 自己的读数能力（[`Self::tree_snapshot`] /
    /// [`Self::capture`] / [`Self::pump_meters`] / [`Self::engine_snapshot_counts`] /
    /// [`Self::ui`] 上的窗口属性数组）—— 而把执行面装箱交给控制面
    /// （[`Self::into_control_plane`]）之后，两者不可兼得。
    ///
    /// 它走的是**权限闸门**（`UiTestPort::open_project` ⇒ `Operation::OpenProject`），
    /// 不是绕过它直接调 [`LiveAdminSurface::open_project_now`]：因此"权限不足时实现侧
    /// 一次都没被调用"这条既有的闸门语义在这条入口上也成立。
    ///
    /// 控制面那条路（真的 JSON-RPC 文本 + `ui/tree` / `ui/node` / `ui/screenshot` 读数）
    /// 由 `crates/yeban-app/tests/live_ui_mcp.rs` 的**另一条**判据覆盖。
    ///
    /// # Errors
    ///
    /// 端口拒绝（权限不足 / 当前工程没有落点 / 目标打不开 / 权威或撤销端口在场）。
    pub fn open_project(&mut self, path: &str, save_first: bool) -> Result<(), PortError> {
        UiTestPort::open_project(&mut self.surface, path, save_first)
    }
}

/// 控制面 + 装配时留下的证据（判据用）。
pub struct LiveControlPlane {
    plane: ControlPlane,
    view: ViewState,
    registry: ControlTree,
    scene: DemoScene,
    reference: Rgb8Image,
    /// 活窗口的句柄（`Rc` **共享**，不是第二份窗口状态）。
    ///
    /// 只用来读一个探针：[`Self::rendered_frames`]。它是"读控件树不得渲染一帧"
    /// 这条**成本**不变量的观测点 —— 没有它，`refresh_tree` 里被重新加回一次
    /// `capture()` 也不会有任何判据变红。
    window: Tier1Window,
}

impl LiveControlPlane {
    /// 借出控制面（发 `ui/*` 调用）。
    pub fn plane(&mut self) -> &mut ControlPlane {
        &mut self.plane
    }

    /// 这个活窗口**已经真的光栅化过**的帧数（**探针**，见 [`Tier1Window::rendered_frames`]）。
    ///
    /// 判据用它钉住：`ui/tree` / `ui/node` / `ui/property` 这些**读元数据**的方法
    /// 不得渲染一帧；`ui/screenshot` 必须**恰好**渲染一帧（多一帧 = 白渲染，
    /// 少一帧 = 像素来源可疑）。
    #[must_use]
    pub fn rendered_frames(&self) -> usize {
        self.window.rendered_frames()
    }

    /// **取走服务本体**（`--enable-ui-mcp-http` 的环回传输按值持有它：
    /// `yeban_ui_mcp::transport::mount::UiHttpMount::bind_loopback(service)`）。
    ///
    /// 交出去的是**这一个**服务（同一个令牌 / 同一套作用域 / 同一个执行面），
    /// 因此"判据里跑的控制面"与"环回 socket 上应答的控制面"不可能分叉。
    /// 装配期留下的读数（投影 / 注册表 / 对照帧）在这一步被丢弃 —— 它们只是证据，
    /// 服务不需要它们。
    #[must_use]
    pub fn into_service(self) -> UiService {
        self.plane.into_service()
    }

    /// 投影出来的视图状态。
    #[must_use]
    pub fn view(&self) -> &ViewState {
        &self.view
    }

    /// 静态注册表。
    #[must_use]
    pub fn registry(&self) -> &ControlTree {
        &self.registry
    }

    /// 装配时直接抓的那一帧。
    #[must_use]
    pub fn reference(&self) -> &Rgb8Image {
        &self.reference
    }

    /// 视口尺寸（`[MUST-GATE-015]` 的"尺寸正确"要对的数）。
    ///
    /// 注：**没有**单独的 `scene()` 访问器 —— 判据只用到视口尺寸，
    /// 一个没人调用的公开方法在 `-D warnings` 下是 `dead_code`（CI 第一轮实测踩到：
    /// `error: method scene is never used`）。有用的 API 才留，不留"以后可能有用"。
    #[must_use]
    pub fn viewport(&self) -> Size {
        Size::new(self.scene.viewport_width, self.scene.viewport_height)
    }
}

/// 以 `project` 为唯一数据源，在**真实界面**上装配一条控制面（默认形态）。
///
/// 等价于 [`build_live_ui_with`] + [`LiveWiringOptions::default`] 里换掉权限。
///
/// # Errors
///
/// 同 [`build_live_ui_with`]。
pub fn build_live_ui(
    project: &YebanProjectV1,
    permission: Permission,
) -> Result<LiveUi, LiveWiringError> {
    build_live_ui_with(
        project,
        &LiveWiringOptions {
            permission,
            ..LiveWiringOptions::default()
        },
    )
}

/// **以进程内控制面会话为唯一可变权威**装配真实界面（`ROAD-M4-008` 选项 (a)）。
///
/// 与 [`build_live_ui`] 的差别是**结构性的**，不是风格：这里**不接受**任何工程参数。
/// 工程的唯一来源是 `authority` —— 也就是控制面**正在服务的那一个** `Domain`。
/// 因此"界面画的是哪一份工程"与"控制面读写的是哪一份工程"不可能分叉：
/// 分叉需要一个第二来源，而本函数没有给它入口。
///
/// 之后每次权威改了工程（例如 AI 发了一次 `tools/call`），用 [`LiveUi::sync_authority`]
/// 刷新投影；本函数已经把**当前**那一版投影进去了，因此不需要先调一次同步。
///
/// # Errors
///
/// 权威会话没有活跃工程（[`LiveWiringError::NoActiveAuthorityProject`]），
/// 或投影 / 平台 / 组件 / 抓帧失败（同 [`build_live_ui_with`]）。
#[cfg(feature = "in-process-mcp")]
pub fn build_live_ui_from_authority(
    authority: &ProjectAuthorityHandle,
    permission: Permission,
) -> Result<LiveUi, LiveWiringError> {
    build_live_ui_from_authority_with(
        authority,
        &LiveWiringOptions {
            permission,
            ..LiveWiringOptions::default()
        },
    )
}

/// 同 [`build_live_ui_from_authority`]，但显式给出全部装配选项（控制台 Tab / 保存路径 /
/// 量子数）—— 与 [`build_live_ui_with`] 对 [`build_live_ui`] 的关系同款。
///
/// 存在的理由（`ROAD-M4-008` 选项 (a) 的单一写者判据）：`ui/force_save` 的落点由
/// `save_path` 决定，而"保存走的是权威还是本地路径"必须能在**同一个装配入口**上被断言
/// （[`LiveAdminSurface::save_now`] 的分支）。没有这个入口，判据就只能分别装配两次、
/// 比较两个不同的执行面。
///
/// # Errors
///
/// 同 [`build_live_ui_from_authority`]。
#[cfg(feature = "in-process-mcp")]
pub fn build_live_ui_from_authority_with(
    authority: &ProjectAuthorityHandle,
    options: &LiveWiringOptions,
) -> Result<LiveUi, LiveWiringError> {
    let project = authority
        .project()
        .ok_or(LiveWiringError::NoActiveAuthorityProject)?;
    let mut ui = build_live_ui_with(&project, options)?;
    ui.surface.attach_authority(authority);
    Ok(ui)
}

/// 同 [`build_live_ui`]，但显式给出全部装配选项（控制台 Tab / 保存路径 / 量子数）。
///
/// 构造顺序（不可颠倒，顺序本身就是约束）：
/// 1. 投影：`YebanProjectV1` → `ViewState`（唯一的事实源方向）；
/// 2. 注册表：`ElementRegistry::from_view` → `ControlTree`（语义寻址的全集）；
/// 3. 外壳场景：`DemoScene::from_view`（视口尺寸 + 会话运行态占位）；
/// 4. `LivePort::new`：**装 Tier-1 平台 → 构造 `MainWindow`（`host::build_main_window*`）
///    → `show()` → 抓运行时控件树**（含真实几何）；
/// 5. 直接抓一帧留作对照；
/// 6. `PortAdapter::new`：把"怎么拿像素"注入进去，得到零 Slint 的 `UiSurface`；
/// 7. `LiveAdminSurface`：接上三个管理动作与电平消费端（**不**在这里建引擎 ——
///    `ui/reload_engine` 才建，因此"装配"与"起引擎"是两件可分别观测的事）。
///
/// # Errors
///
/// 投影失败（[`LiveWiringError::Project`]）或平台/组件/抓帧失败
/// （[`LiveWiringError::Render`]）。
pub fn build_live_ui_with(
    project: &YebanProjectV1,
    options: &LiveWiringOptions,
) -> Result<LiveUi, LiveWiringError> {
    let view = ViewState::from_project(project)?;
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let registry = control_tree_from_registry(&ElementRegistry::from_view(&view))?;
    // 投影出来的**窗口**：`host::build_main_window_with_console_tab` 是唯一的注入实现（D28）。
    let console_tab = options.console_tab;
    let port = LivePort::new(size, options.permission, Some(&registry), || {
        host::build_main_window_with_console_tab(&view, &scene, console_tab)
    })?;
    let reference = capture_tier1(&port)?;
    // 活窗口的强引用：管理动作（改属性 / 注入电平 / 重抓树）都要它，而 `LivePort` 把
    // 组件放在自己内部（`ui()` 只借出 `&T`）—— `clone_strong` 是 Slint 组件句柄的
    // **共享**（`Rc` 语义）而不是复制，因此这里不会产生第二份窗口状态。
    let window = port.ui().clone_strong();
    // 显式写出函数指针类型：`PortAdapter` 是泛型的，靠"赋值给 `LiveSurface`"反推
    // `F = fn(..)` 属于**强制转换点**，写出来比让读者猜推断结果更清楚。
    let capture: fn(&LivePort<MainWindow>) -> Result<Rgb8Image, PortError> = capture_tier1;
    let surface: LiveSurface = PortAdapter::new(port, "tier1-live-port", capture);
    // `[UI-A11Y-002]` 的启动态：画布聚焦、非合成（与 `InputContext::new()` 一致）。
    let input = Rc::new(RefCell::new(InputContext::new()));
    // **事件源接线（本线补的那一半）**：`.slint` 的 `TextInput.preedit-text` /
    // `has-focus` 变化 → `MainWindow.ime-composition-changed` / `ime-focus-changed`
    // → `host::wire_input` → 上面那**同一个** `InputContext`。
    // 与 `input_context()` / `ui/property {"name":"isComposing"}` 共享同一个 `Rc`，
    // 因此"界面上的合成态"与"界面看到的合成态"不可能各说各话（不新造状态机）。
    host::wire_input(&window, Rc::clone(&input));
    // `N2` 裁决 (1)：GUI 的**逻辑键**事件源（`ui/app.slint` 的 `key-handler` FocusScope）。
    // 撤销端口来自装配选项：默认 `None`（判据侧没有撤销会话时，撤销族如实 `reject`，
    // 工具 / 视图 / 走带这类界面动作照常生效 —— 判据 16 判的就是它们）。判据用
    // [`LiveWiringOptions::undo`] 把它接上，从而见证"按键 → 撤销端口提交 → 重投影"整段。
    host::wire_keys(&window, Rc::clone(&input), options.undo.clone());
    // **纯视图态的四条回调**（`toggle-view` / `toggle-sidebar` / `toggle-ai-drawer` /
    // `open-musical-pr`）：与产品进程**同一个**接线函数（`main.rs` 的 `wire_callbacks`
    // 也调它），因此端口注入的点击与用户点击走的是同一条链
    // （真实指针事件 → Slint 命中测试 → `.slint` 的 `TouchArea` → 宿主写属性）。
    // 只有属性写入 ⇒ 不改工程、不需要 `UndoPort`，装配顺序上也不与撤销端口耦合。
    host::wire_view_callbacks(&window);
    // 混音台（控制台 Tab 2）的**写入面**：与产品进程（`main.rs` 的 `wire_callbacks`）调的是
    // **同一个**函数。它需要 `UndoPort`（工程内容必须走 `Op`，`ADR-0005`）⇒ 只在装配给了
    // 端口时接上；`options.undo` 为 `None` 的既有判据因此照旧"没有写入面"，行为一位不变。
    if let Some(undo) = options.undo.as_ref() {
        host::wire_mixer_edit(&window, undo);
    }
    let admin = LiveAdminSurface {
        inner: surface,
        window,
        registry,
        project: project.clone(),
        view,
        save_path: options.save_path.clone(),
        meters: MeterRuntime::empty(),
        engine: EngineHost::new(),
        engine_quanta: options.engine_quanta,
        save_epoch: 0,
        report: None,
        input,
        // 默认**没有**权威句柄：`build_live_ui*` 的工程由调用方给出。
        // 接上权威的装配入口是 `build_live_ui_from_authority`（选项 (a)）。
        #[cfg(feature = "in-process-mcp")]
        authority: None,
        #[cfg(feature = "in-process-mcp")]
        authority_revision: 0,
        // 撤销端口的**同一份** `Rc`（不是第二份）：键盘路径的闭包与这里共享它，
        // 因此 [`LiveAdminSurface::open_project_now`] 能如实判断"工程是不是还有别的持有者"。
        undo: options.undo.clone(),
    };
    Ok(LiveUi {
        reference,
        surface: admin,
        scene,
    })
}
