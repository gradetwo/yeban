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
//!    当时的处置是**删掉**它（`docs/ledger/live-port-notes.md` §6.1），
//!    而不是 `#[allow(dead_code)]` —— 后者会把"这块代码没人用"这个真实信号一起盖掉。
//!    ⇒ **本文件的公开 API 只包含当前真的被调用的东西**；加方法时要么同时接上调用方，
//!    要么就别加。判据只住在 `crates/yeban-app/tests/live_ui_mcp.rs` **一个**目标里，
//!    这样"用了什么"与"提供了什么"可以对齐（多目标会让某个目标看不到某个方法 ⇒ 假死代码）。
//!    ⚠ **这条的前提于 `D-UI-3` 起已不成立，2026-11-03 实测再核一遍**：装本文件的测试目标
//!    是 `grep -l 'path = "../src/live_surface.rs"' tests/ | wc -l` = **43** 个（单位：个目标），
//!    因此"某目标看不到某方法"是**常态**而非意外。处置随之改成：⛔ 不删判据要用的读数，
//!    在**那一条**读数上放 `#[allow(dead_code)]` ＋ 一行写明**真实调用者住在哪个判据**
//!    （近例：`dispatch_key_chord`、`open_project_via_picker`、`capture`、
//!    `engine_metronome_enabled`、`engine_transport_*`）。信号没被盖掉 —— 被盖掉的只是
//!    "在这个特定目标里没人调"这一条**每目标都不同**的事实。
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
use yeban_app::input::{Focus, InputContext, LogicalKey, Modifiers, PhysicalKey, Resolution};
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
use yeban_ui_test_port::port::{
    ChordKey, KeyChord, KeyChordModifiers, KeyCode, Permission, PointerButton, PortError,
    UiTestPort,
};
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
    /// AI 提案抽屉的**来源**（[`host::wire_proposal_drawer`] 的第二个参数）。
    ///
    /// 默认 `none()` ⇒ 抽屉画"这个装配没有接进程内的提案来源"那句空态（真话）。
    /// ⚠ 这个字段**不由调用方决定**：[`build_live_ui_from_authority_with`] 会把它**覆盖**
    /// 成 `from_authority(authority)` —— 接在权威上的装配不可能"看不见权威里的提案"，
    /// 而不接权威的装配（[`build_live_ui`] / [`build_live_ui_with`]）拿不到句柄。
    pub proposal_source: yeban_app::proposals::ProposalSource,
}

impl Default for LiveWiringOptions {
    fn default() -> Self {
        Self {
            permission: Permission::ReadOnly,
            console_tab: 0,
            save_path: None,
            engine_quanta: 8,
            undo: None,
            proposal_source: yeban_app::proposals::ProposalSource::none(),
        }
    }
}

/// **一次心跳的读数**（[`LiveUi::engine_heartbeat`] 的返回值）。
///
/// 它是生产 `run_gui` 的 60Hz 定时器那一跳的**逐字复制**：先按需增量发布快照，再回收
/// （`SnapshotSlot::prune` + `RetireQueue::drain`）。
///
/// ⚠ ⛔ **不再 `Copy`**（任务单 `#82`）：`publish_error` 是 `String` 而不是一个枚举哨兵，
/// 因为"失败在哪一步 / 是哪个文件"正是用户要看到的东西（`EngineHostError` 的 `Display`
/// 原文），把它压成一个 `bool` 就等于又把信息丢一次。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineTick {
    /// 这一跳真的发布了新快照时是它的版本号；标记没变（或没有工程可投影）时是 `None`。
    pub published_revision: Option<u64>,
    /// ⭐ **这一跳发布失败**的错误原文（`#82` 修掉 `.ok()` 吞错）。
    ///
    /// 三种取值对应三件**不同**的事，⛔ 不许压成一个 `None`：
    ///
    /// | 值 | 含义 |
    /// | :--- | :--- |
    /// | `None` ＋ `published_revision == Some(_)` | 这一跳真的换了一份新快照 |
    /// | `None` ＋ `published_revision == None` | **没有发布**（标记没变 / 没有工程可送 / 没有引擎） |
    /// | `Some(text)` | **发布了但失败** ⇒ 播放侧仍是**上一版**快照 |
    ///
    /// 为什么必须有第三档：失败在修之前与"没改动"**在读数上一模一样**
    /// （都是 `published_revision = None`），于是"用户以为换了 SFZ、其实一次都没生效"
    /// 在控制面上不可见 —— 那正是本单修掉的吞错。
    pub publish_error: Option<String>,
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
    /// ⭐ `D-UI-31`：与 `D-UI-30` 对宿主做的那一跳**同款**形状 —— `Rc<RefCell<..>>`
    /// 而**不是**裸值，因为 [`host::wire_ruler_seek`] / [`host::wire_section_seek`] /
    /// [`host::wire_scene_launch`]（以及既有那两条 `wire_sidebar_contents` /
    /// `wire_clip_scope`）的回调要从这份投影**现读**（网格右缘、小节长度、段落起点、
    /// 场景 tempo），而 [`Self::apply_project`] 会把它整体换掉。
    ///
    /// ⛔ 这不是"第二份投影"：装配只建**一个** `RefCell`，交给上面那五条接线的是
    /// `Rc::clone`（计数 +1）。存裸值的旧形状有两种，都留了洞：
    ///  · 装配期快照（`Rc::new(RefCell::new(view.clone()))`）⇒ 换工程之后读数停在旧工程；
    ///  · 裸值 + 每次换工程重接 ⇒ 每条读投影的接线都要在**每一个**重投影点重接一次，
    ///    漏一个点就回到"停在旧工程"（本文件有两个重投影点：`apply_project` /
    ///    `apply_track_height`；未来的第三个会静默复发 —— 那是 `D-UI-10` 那一族）。
    /// `main.rs` 的产品装配用的正是这一份共享（`view_cell`，`main.rs` 的
    /// `wire_callbacks` 拿 `Rc::clone(&view_cell)`）。
    view: Rc<RefCell<ViewState>>,
    save_path: Option<PathBuf>,
    meters: MeterRuntime,
    /// ⭐ `D-UI-30`：与产品路径（[`host::ProductionLoop`]）与 `main.rs` 的走带**同款**形状
    /// —— `Rc<RefCell<..>>` 而**不是**裸值，因为 [`host::wire_transport`] 的回调要拿
    /// **同一个**实例（`Rc` 计数 +1，不是第二份宿主）。
    ///
    /// ⛔ 这里不存在"第二份走带"：装配接了 `wire_transport` 之后，界面上的播放 / 停止 /
    /// 节拍器按钮与本执行面驱动的 `engine_tick` / `rebuild_engine` 作用在**同一个**宿主上。
    engine: Rc<RefCell<EngineHost>>,
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
            's' => (PhysicalKey::KeyS, bare),
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

/// 一次注入 chord 的**只读处置结论**（与 [`key_resolution`] 共用同一张策略表、
/// 同一个词表 —— 差别只在修饰位来自实参而不是扫描码表）。
fn chord_resolution(state: &InputContext, chord: &KeyChord) -> &'static str {
    let Some(logical) = LogicalKey::from_text(&chord.text()) else {
        // 词表漂移（控制面给了一个宿主解析不出来的 `text`）⇒ 与宿主自己的取向一致：不消费。
        return yeban_ui_mcp::ime::RESOLUTION_PASS_THROUGH;
    };
    let flags = chord.modifiers;
    let modifiers = Modifiers {
        ctrl: flags.ctrl,
        shift: flags.shift,
        alt: flags.alt,
        meta: flags.meta,
    };
    match state.resolve_logical(logical, modifiers) {
        Resolution::ConsumedByIme => yeban_ui_mcp::ime::RESOLUTION_CONSUMED_BY_IME,
        Resolution::Action(_) => yeban_ui_mcp::ime::RESOLUTION_ACTION,
        Resolution::PassThrough => yeban_ui_mcp::ime::RESOLUTION_PASS_THROUGH,
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

    /// 引擎重建（`ui/reload_engine`、`start_engine` 与 `apply_project` 共用同一条路径）。
    ///
    /// ⭐ 任务单 `#136`：来源是装配给本执行面的**同一个** `Rc<UndoPort>`（会话 CAS 池），
    /// 在 `reload` **之前**接上 —— 第 1.3 步要当场注入 `ClipContent::Audio` 的 PCM。
    /// 没有端口（`options.undo == None` 的旧装配）⇒ 不接来源，音频片段被逐条记为
    /// "跳过"（可见，⛔ 不静默），与接线前逐位相同。
    ///
    /// # ⭐ `D-UI-32`：换代之后必须**装载即停住**
    ///
    /// `EngineRuntime` 的走带**构造默认是 `Playing`**（`Transport::free_running`）。那不是
    /// 缺陷而是刻意：接入走带之前 `process_quantum` 的语义就是"快照一发布就从 tick 0 起滚"，
    /// 默认值改成停住会让既有判据变红。引擎侧为此把话写死在
    /// `crates/yeban-engine/src/transport.rs` 的 `Transport` 文档里：
    /// **"要『加载即停住』的控制面发一条 `Stop` 即可（`yeban-app::engine_host` 就是这么做的）"**。
    ///
    /// 生产那条腿照做了（`src/main.rs` 建完引擎就 `engine_handle().borrow_mut().stop()`，
    /// 再把读数投回界面），而**本执行面三处换代站点一处都没做** ⇒ 用户可见的症状是三件：
    ///
    /// | 症状 | 成因 |
    /// | :--- | :--- |
    /// | 界面一开就显示「播放中」，没人按过播放 | 第一跳心跳的 `apply_transport` 如实投出引擎的 `Playing` |
    /// | 播放头自己往前走 | 自由跑：每量子都在推进 tick |
    /// | 第一次按「播放」是**停** | `wire_transport` 读镜面 `playing`（那时已为真）⇒ 走的是"停住"那一支 |
    ///
    /// ⚠ 为什么停在这里、⛔ 不在三个调用点各写一遍：这个缺陷的**成因**就是"两个装配现场
    /// 各抄一次同一步骤，其中一份抄漏了"（`main.rs` 有、`live_surface.rs` 没有）。把它写在
    /// **唯一的换代入口**里 ⇒ 第四个换代点将来也不可能漏。与 `D-UI-30`/`D-UI-31` 同一课。
    ///
    /// ⚠ `reload` 内部**刻意不**发这条 `Stop`（见 `EngineHost::reload` 第 5 步那条注释：
    /// 让命令在量子边界生效要**多推一个量子**，会把 `ui/reload_engine` 报的 `quanta` 顶到
    /// 调用方要的数字之外，实测红在 `tests/live_ui_mcp.rs` 的
    /// `admin_reload_engine_rebuilds_and_resets_the_meter_tap`）。本处的 `stop()` 在
    /// `reload` **之后**，因此 `EngineRebuild` 里那几个数字一位不动；代价只有一个：控制驱动
    /// 形态下多发一条命令 = 多推一个量子，而那条命令是 `Stop` ⇒ **位置一位不走**。
    fn rebuild_engine(&mut self) -> Result<yeban_app::engine_host::EngineRebuild, LiveWiringError> {
        if let Some(undo) = self.undo.as_ref() {
            let port = Rc::clone(undo);
            self.engine
                .borrow_mut()
                .set_audio_asset_source(Box::new(move |hash| port.asset_bytes(hash)));
        }
        let rebuild = self
            .engine
            .borrow_mut()
            .reload(&self.project, self.engine_quanta)
            .map_err(LiveWiringError::from)?;
        // ⭐ 作用域纪律：**先放借、再碰界面**（`apply_transport` 写属性 = 求值 Slint 绑定）。
        let parked = self.engine.borrow_mut().stop();
        host::apply_transport(&self.window, parked);
        Ok(rebuild)
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
    ///
    /// ## ⭐ 失败**不再被吞掉**（任务单 `#82`）
    ///
    /// 这里原先是 `self.engine.publish_project(project, mark).ok()` —— 发布失败与
    /// "标记没变"因此给出**同一个**读数（`published_revision == None`），控制面看不见
    /// "这条编辑一次都没生效"。对 SFZ 绑定尤其致命：用户敲了一条**不存在的路径**，
    /// 界面上输入框留着那条路径，而播放侧静默地用着上一代快照 ⇒
    /// "换了 SFZ 但没生效"变成一个查不出的现象。
    ///
    /// 现在失败进 [`EngineTick::publish_error`]（`EngineHostError` 的 `Display` 原文）
    /// ＋ 出声。⚠ 生产 `run_gui` 那条腿（[`crate::host::ProductionLoop::tick`]）**本来
    /// 就有**这一段 `match`（它不吞）；本单修的是**本执行面**这一份。
    ///
    /// ## ⭐ 原文**同时**投到界面（任务单 `#86` 甲）
    ///
    /// 上面那件事只让失败"在控制面上可读" —— **窗口里仍然看不见**。本单把同一个结局
    /// 交给 [`crate::host::apply_engine_error`]（错误横幅的**唯一写者**），于是
    /// "用户点了 SET 一个坏路径"在无头控件树 JSON 里也**可寻址**（`engine-error-banner`
    /// 的 `accessible-value` = 这段原文）。
    ///
    /// ⚠ 顺序：投影必须在算完 `publish_error` **之后**；`Unchanged` 那一档一位不动
    /// （见 [`crate::host::EnginePublishOutcome`]）—— 否则 60Hz 心跳会让横幅闪一下就走。
    fn engine_tick(&mut self, mark: EditMark, project: Option<&YebanProjectV1>) -> EngineTick {
        let mut published_revision = None;
        let mut publish_error = None;
        // ⭐ `D-UI-29`：与生产 `ProductionLoop::tick` **同一格**取值（只在发布成功时）。
        let mut snapshot_metronome = None;
        // ⭐ `D-UI-30`：借的**作用域只包住动引擎那一段**，放借之后才碰界面 ——
        // 与本文件新接的 [`host::wire_transport`] 共用同一个宿主，那两个回调也会
        // `borrow_mut()` 它。`#212` 的教训就是"持借调界面"。
        let readings = {
            let mut engine = self.engine.borrow_mut();
            if engine.has_engine()
                && engine.should_publish(mark)
                && let Some(project) = project
            {
                match engine.publish_project(project, mark) {
                    Ok(revision) => {
                        published_revision = Some(revision);
                        snapshot_metronome = Some(project.transport.metronome_enabled);
                    }
                    Err(error) => {
                        // ⭐ 保留原文供控制面/判据读（不是 `.ok()`），并出声。
                        eprintln!("[yeban-app] 快照发布失败（播放侧仍是上一版）: {error}");
                        publish_error = Some(error.to_string());
                    }
                }
            }
            engine.heartbeat()
        };
        // ⭐ `#86` 甲：**本执行面**的投影点（与生产 `ProductionLoop::tick` 调的是
        // **同一个**函数 ⇒ 两条腿的界面读数不可能漂移）。
        //
        // ⚠ 用 `host::`（文件头的 `use yeban_app::host;`）而**不是** `crate::host::`：
        // 本文件会被 `#[path]` 装进集成测试目标，那时 `crate` 是**测试 crate**、
        // 里面没有 `host` 模块（`crate::host::` 会在 `cargo test` 时报 E0433）。
        host::apply_engine_error(
            &self.window,
            match (&publish_error, published_revision) {
                (Some(error), _) => host::EnginePublishOutcome::Failed(error),
                (None, Some(_)) => host::EnginePublishOutcome::Published,
                (None, None) => host::EnginePublishOutcome::Unchanged,
            },
        );
        // ⭐ `D-UI-29`：**节拍器镜面**与生产心跳调的是**同一个**校正点 ⇒ 本执行面驱动
        // 的判据也能读到"快照换了 ⇒ 界面跟着换"（撤销 / 控制面写 / 换工程都不留旧值）。
        host::apply_metronome_mirror(&self.window, snapshot_metronome);
        EngineTick {
            published_revision,
            publish_error,
            readings,
        }
    }

    /// 驱动音频侧 `quanta` 个量子（**不**排空退役队列）—— 设备回调那一侧的替身。
    ///
    /// 本机没有 cpal 回调线程（AGENTS.md §5），判据用它把"读者换了快照、旧快照进了
    /// 退役队列"这个窗口造出来，再由**心跳**回收。
    fn drive_audio(&mut self, quanta: u64) -> u64 {
        self.engine.borrow_mut().drive_audio(quanta)
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
        // 投影按 `D-UI-31` 住在共享的那一格：读之前 `borrow`，读完立刻放下
        // （`refresh_tree` 会重投影，绝不能带着 `Ref` 进去 —— `#212` 同一条纪律）。
        let pump = {
            let view = self.view.borrow();
            host::pump_meters(&self.window, &mut self.meters, &view)
        };
        // 电平标签变了 ⇒ 控件树必须重抓（失败也不该让一次电平刷新变成错误：
        // 界面已经是新值，重抓失败只是"控制面看到的树旧了一点"）。
        let _ = self.refresh_tree();
        pump
    }

    /// 换一个工程：投影 → 注入 → 换注册表 → 作废旧电平 → 重接走带 → （若引擎已起）换代 → 重抓树。
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
        // ⭐ `D-UI-31`：写进**共享的那一格**（不是换一个 `Rc`）⇒ 已经接好的标尺 / 段落 /
        // 场景 / 资源栏 / 剪辑作用域五条接线，下一次读投影时读到的就是这一份 ⇒
        // 这里⛔ 不需要（也**不该**）再重接一次。
        *self.view.borrow_mut() = view;
        self.registry = registry;
        // 旧读数不再代表任何东西：清空面板，并把"等长静音"写进界面。
        self.meters = MeterRuntime::empty();
        self.save_epoch = 0;
        self.report = None;
        // ⭐ `D-UI-30`：**重接走带**，与 `main.rs` 换工程那段（`wire_callbacks` + `wire_keys`
        // 用新的 `Rc<YebanProjectV1>` 再走一遍）同一步。Slint 的 `on_*` 是**覆盖**语义
        // ⇒ 这里不是"第二个注册者"，而是把闭包里的工程换成**这一份**。
        // 不重接的后果：录音键读的是装配期那份的 `count_in_bars`（`#132` 那条"第一个读者"
        // 从此变成陈旧值），而 `metronome-on` 的初值也是旧工程里的开关。
        // 宿主 `Rc` 传的是**同一个**实例（计数 +1），因此走带不会因此变成两份。
        host::wire_transport(
            &self.window,
            Rc::clone(&self.engine),
            Some(Rc::new(project.clone())),
            self.undo.clone(),
        );
        if self.engine.borrow().has_engine() {
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
        // ⭐ 抽屉的提案读数**跟着同一次刷新**（与生产窗口的
        // `reproject::AuthorityMirror::sync_now` 同一条口径）：合并提案会推进修订号 ⇒
        // 判据刷完就能在树里看见"少了一条"，不必再点一次打开。仍**只读**权威。
        host::apply_proposal_drawer(
            &self.window,
            &yeban_app::proposals::ProposalSource::from_authority(&authority),
            host::DrawerIntent::default(),
        );
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
        let track_id = {
            let view = self.view.borrow();
            track_id_of_header(element_id, &view)
        }
        .ok_or_else(|| PortError::UnknownElement {
            id: element_id.to_owned(),
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
        // 同上：写共享的那一格（`D-UI-31`）⇒ 读行高/小节长度那几条接线不需要重接。
        *self.view.borrow_mut() = view;
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
    /// | **没有**（[`build_live_ui`] / [`build_live_ui_with`] 装配的） | [`save_project_file`]（本地原子写，取排他写建议锁） | 没有控制面时进程里只有它一个写者，本地路径就是那条路；挂着撤销端口时写端口的工程（`ADR-0057` A），不是界面缓存（否则注入的编辑会被静默丢掉） |
    ///
    /// 两条路**不会**同时发生：有权威时**不**调用本地路径，权威拒绝时**不**回退到本地路径
    /// （回退会把"权威说不能写"变成一个偷偷写盘的分支）。因此"保存"这个动作在两种装配下
    /// 各自只有一个落点，且挂载时那个落点是会话。
    ///
    /// ⭐ 本地那条腿再分两种（缺陷 `D-UI-1` 的同一次修复）：装配给了**本地撤销端口**时，
    /// 落盘的字节取自端口的当前工程（`UndoPort::try_project`，与生产心跳的发布来源同一个
    /// 读数）—— 注入手势改的是端口，`self.project` 只是装配缓存，写它等于丢编辑。
    /// 没有端口时本执行面就是唯一持有者，照旧写缓存。读数进
    /// `writtenFromUndoPort`（`bool`），因此"这一版到底从哪儿来"是可断言的而不是注释。
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
        // [ADR-0057 A] 有撤销端口时保存**端口的工程**，不是界面缓存 `self.project`：
        // `wire_*` 的提交只走 `commit_ops → refresh_undo_window`（重投影窗口，不换
        // `self.project`）⇒ 按旧缓存写盘会把注入的编辑静默丢掉（比「什么都不发生」
        // 更坏）。端口没有活跃工程时回落到缓存（服务端装配恒有工程，此分支只防退化）。
        // `D-UI-1` 追加的一拍：取到端口那一版时**同时**投影进界面 ⇒ 画面 / 控件树 /
        // 磁盘同源（只写盘不重投影的话，保存成功了界面还挂在旧缓存上）。
        let from_port = self.undo.as_ref().and_then(|port| port.try_project());
        let (save_source, written_from_undo_port) = match &from_port {
            Some(project) => {
                self.apply_project(project).map_err(wiring_rejected)?;
                (project.clone(), true)
            }
            None => (self.project.clone(), false),
        };
        let saved = save_project_file(&save_source, &path).map_err(|error| match error {
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
                // 没有权威 ⇒ 本地原子写（无端口时与历史行为逐字相同）。
                ("writtenByAuthority", ReportValue::Bool(false)),
                // 有端口时写的是端口的工程（`ADR-0057` A 的可断言证据）。
                (
                    "writtenFromUndoPort",
                    ReportValue::Bool(written_from_undo_port),
                ),
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
    /// 「打开」这一层只有**一条容器读取核心**：`crate::open` 的 `read_capped`（尺寸闸）
    /// ＋ `open_project_archive`（容器裁决）。本方法用它上面的 **`open_project_file`**
    /// （`crates/yeban-app/src/open.rs:357`）。⚠ 精确口径（2026-10-08 就地改正，原写"与 CLI
    /// `--open` 是同一个入口"）：CLI 那条（`crates/yeban-app/src/main.rs` 的
    /// `cli::load_project`）走的是**同模块的另一个包装** `open_project_document_file`
    /// （`crates/yeban-app/src/open.rs:310`），它多带回 `history.dag` / 资产池与
    /// `NotAYebanContainer` 这一档分类。两者共用**同一条**底层路径，因此不存在第二份容器
    /// 读法；差别只在**返回形态**。本方法的取舍是 `open_project_file` 自己的文档契约：
    /// **只取工程** ⇒ `history.dag` 与资产池**不**在这里恢复（本执行面没有这一层状态的消费者：
    /// 挂着撤销会话时它直接拒绝，见下表）。
    ///
    /// 「注入」也只有一条：[`Self::apply_project`]
    /// （`ViewState` → `host::apply_view` → 换注册表 → 作废旧电平 → `EngineHost::reload`
    /// 换代 → `pump_meters`）—— 与权威重投影 [`Self::sync_authority`] 逐字同一条数据流。
    /// 本方法**不**新增打开实现、**不**新增注入点。
    ///
    /// ## 工程**别的持有者**在场时：拒绝（不留两条事实源）
    ///
    /// | 本执行面持有 | 为什么拒绝 |
    /// | :--- | :--- |
    /// | `authority`（进程内控制面会话） | 工程的唯一可变权威是那个 `Domain`（`ADR-0005`）。在这里换一份会让"界面画的是哪一份"与"会话读写的是哪一份"立刻分叉；而宿主的写入口 `HostAction` 只有 `Undo` / `Redo` / `Commit` —— **没有**"打开工程"这一项（`crates/yeban-mcp/src/domain/mod.rs:669`）⇒ 本执行面无法把这次打开交给权威 |
    /// | `undo`（撤销端口） | 那个端口**自己持有一份工程**（`UndoPort::try_project` 是生产心跳的发布来源）⇒ 换工程而不换端口会让端口把旧工程写回引擎。而「打开一个工程 = **新会话**」只有启动装配那一条实现（`yeban_app::undo::open_undo_session`），本执行面换不了手里的端口 ⇒ 如实拒绝，而不是换一半；测试装配在启动时 `--open` 定死工程 |
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
                message: "本执行面挂着撤销端口（它自己持有一份工程）⇒ 在这里换工程会让它\
                          把旧工程写回引擎。「打开一个工程 = 新会话」只有启动装配那一条实现\
                          （`yeban_app::undo::open_undo_session`），本执行面换不了手里的端口 ⇒ \
                          如实拒绝，而不是换一半；测试装配请在启动时 `--open` 定死工程，\
                          没有撤销端口的装配（产品形态 `--enable-ui-mcp-http`）仍可在这里换工程"
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
        //
        // ⭐ 任务单 `#168`：相对 SFZ 路径从**新工程文件所在目录**开始解析。
        //    `apply_project` 会重建引擎（`reload`），目录必须在它**之前**写下。
        let project_dir = PathBuf::from(path)
            .parent()
            .map(std::path::Path::to_path_buf);
        self.engine.borrow_mut().set_project_dir(project_dir);
        self.apply_project(&project).map_err(wiring_rejected)?;
        // ④ 落点跟着新工程走。
        self.save_path = Some(PathBuf::from(path));
        // 三个**回读**读数（不是回显入参，也不是"我知道它应该是几"）：
        // `tracks` = 手里那份投影的轨道数（投影的权威形状）；
        // `rows` = 界面 `track-names` 的行数；
        // `meterRows` = 界面电平数组的长度。
        // 三者等长正是"没有串轨"的可断言形态（`host::apply_meters` 的 `debug_assert_eq!` 钉的
        // 就是前两者；`.slint` 按下标取，不等长会把 A 轨的电平画到 B 轨上）。
        let tracks = self.view.borrow().tracks.len() as u64;
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

    fn dispatch_pointer_cancel_impl(&mut self) -> Result<(), PortError> {
        self.inner.dispatch_pointer_cancel_impl()
    }

    fn dispatch_key_press_impl(
        &mut self,
        key: yeban_ui_test_port::port::KeyCode,
    ) -> Result<(), PortError> {
        self.inner.dispatch_key_press_impl(key)
    }

    /// `ui/dispatch_key_chord` 的**真实载体**（缺陷 `D-UI-3` 的修复落点）。
    ///
    /// ## 为什么是 `invoke_key_action`，不是 `self.inner`（平台事件）
    ///
    /// `dispatch_key_press_impl` 走的是 `window.key_press` / `type_char` 那一格 ——
    /// 真实的**平台事件**通道。而 `slint::platform::WindowEvent::KeyPressed` 只有
    /// `text`、**没有修饰位**（`i-slint-core-1.18.1/api.rs`），从那条路永远带不上
    /// `Shift+↑` / `Shift+Z` / `Cmd+Shift+Z`：Slint 会把"修饰位"折叠进 `text`
    /// 里（`Shift+Z` 到达时 `text == "Z"`），而本仓的 `input.rs` 恰恰把可打印键
    /// **统一小写化**、由修饰位表达组合 ⇒ 从平台事件那一格注入 chord 只会得到一个裸键。
    ///
    /// 用户真键盘上的完整链路是：
    /// `WindowEvent::KeyPressed` → `.slint` 的 `key-handler` → `key_action(text, shift, ctrl, alt, meta)`
    /// → `host::wire_keys` 的 `on_key_action`（`resolve_logical` + `apply_action`）。
    /// 本方法落在那一格的**入口**上，与用户键**同一个**回调、同一张策略表、同一个
    /// `UndoPort`（`D-UI-1` 修好的那一条），因此它不是"绕过 UI 直接改状态"的旁路。
    fn dispatch_key_chord_impl(&mut self, chord: &KeyChord) -> Result<bool, PortError> {
        let modifiers = chord.modifiers;
        Ok(self.window.invoke_key_action(
            chord.text().into(),
            modifiers.shift,
            modifiers.ctrl,
            modifiers.alt,
            modifiers.meta,
        ))
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
                // 一次 `borrow` 读完两条（语义 ID 与矩形高都出自**同一格**投影），读完就放下 ——
                // 下面那次窗口属性读取会求值 Slint 绑定，⛔ 不许带着 `Ref` 进去（`#212` 纪律）。
                let (track_id, drawn_px) = {
                    let view = self.view.borrow();
                    let Some(track_id) = track_id_of_header(element_id, &view) else {
                        // 语义 ID 不是轨道头 ⇒ 真调用会报 `-32006`；dryRun 也如实报"这条预测不了"。
                        return Ok(None);
                    };
                    let drawn_px = view
                        .tracks
                        .iter()
                        .find(|track| track.id == track_id)
                        .map_or(0.0_f32, |track| track.height);
                    (track_id, drawn_px)
                };
                let layout = yeban_app::host::track_height_layout(&self.window);
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
                    ReportValue::Uint(self.engine.borrow().generation().saturating_add(1)),
                ),
                ("quanta", ReportValue::Uint(self.engine_quanta)),
                (
                    "tracks",
                    ReportValue::Uint(self.view.borrow().tracks.len() as u64),
                ),
            ]),
            yeban_ui_mcp::methods::METHOD_OPEN_PROJECT => {
                let path = arguments.text("path").unwrap_or_default();
                let save_first = arguments.is_true("saveFirst").unwrap_or(true);
                // **真校验**（`&self`：一个状态位都不改）：
                // ① 有别的持有者 ⇒ 与真调用**同一句话**地拒绝（dryRun 不许把注定失败说成成功）；
                if self.undo.is_some() {
                    return Err(PortError::Rejected {
                        message:
                            "本执行面挂着撤销端口（它自己持有一份工程）⇒ 在这里换工程会让它\
                                  把旧工程写回引擎。「打开一个工程 = 新会话」只有启动装配那一条实现\
                                  （`yeban_app::undo::open_undo_session`），本执行面换不了手里的端口 ⇒ \
                                  如实拒绝，而不是换一半；测试装配请在启动时 `--open` 定死工程，\
                                  没有撤销端口的装配（产品形态 `--enable-ui-mcp-http`）仍可在这里换工程"
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
                        ReportValue::Uint(self.view.borrow().tracks.len() as u64),
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
            // `ui/dispatch_key_chord` 的预览：`chord` 是**将要注入的名字**（`ChordKey::parse`
            // ＋ `KeyChord::name` 的产物，⛔ 不是入参原样回显），IME 两格读的是**同一份**
            // `InputContext`（`[UI-A11Y-002]` 的两格口径不变）。
            //
            // 解析不出键名 ⇒ 如实报 `None`（真调用的拒绝发生在服务层参数校验那一步，
            // 预览不替它编一个"大概会注入哪个键"）。
            yeban_ui_mcp::methods::METHOD_DISPATCH_KEY_CHORD => {
                let raw = arguments.text("keyName").unwrap_or_default();
                let Ok(key) = ChordKey::parse(raw) else {
                    return Ok(None);
                };
                let chord = KeyChord::new(
                    key,
                    KeyChordModifiers {
                        shift: arguments.is_true("shift").unwrap_or(false),
                        ctrl: arguments.is_true("ctrl").unwrap_or(false),
                        alt: arguments.is_true("alt").unwrap_or(false),
                        meta: arguments.is_true("meta").unwrap_or(false),
                    },
                );
                let input = self.input.borrow();
                PreviewEffect::new(vec![
                    ("chord", ReportValue::Text(chord.name())),
                    ("isComposing", ReportValue::Bool(input.is_composing())),
                    (
                        "focus",
                        ReportValue::Text(ime_focus_of(input.focus()).as_str().to_owned()),
                    ),
                    (
                        "resolution",
                        ReportValue::Text(chord_resolution(&input, &chord).to_owned()),
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
        // `D-UI-31`：投影住在 `Rc<RefCell<..>>` 里 ⇒ 这里要的是**值**的一份拷贝（判据证据），
        // ⛔ 不是把那个 `Rc` 再分出去（那会让"证据"跟着执行面一起变，失去对照的意义）。
        let view = self.surface.view.borrow().clone();
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
        // `D-UI-31`：投影住在 `Rc<RefCell<..>>` 里 ⇒ 这里要的是**值**的一份拷贝（判据证据），
        // ⛔ 不是把那个 `Rc` 再分出去（那会让"证据"跟着执行面一起变，失去对照的意义）。
        let view = self.surface.view.borrow().clone();
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
        self.surface.engine.borrow().snapshot_counts()
    }

    /// 当前引擎快照里某轨的音量（**dB**，f32）—— 判据读回的"那一个字面字段"。
    ///
    /// 没有引擎 / 快照里没有这条轨道 ⇒ `None`（不猜、不返回 0.0）。
    #[must_use]
    pub fn engine_track_volume_db(&self, track_id: &EntityId) -> Option<f32> {
        self.surface
            .engine
            .borrow()
            .current_snapshot()?
            .track(track_id)
            .map(|params| params.volume_db())
    }

    /// ⭐ 当前引擎快照里的**节拍器开关** —— `transport.metronome_enabled` 在**投影期**写进
    /// 快照的那一格（`EngineSnapshot::metronome_enabled`，逐字 = "快照里有没有节拍器计划"）。
    ///
    /// `D-UI-29` 用它对账**三个**读数：权威文档字段 ⇄ 播放侧真正在用的那一代 ⇄ 界面镜面
    /// `metronome-on`。⛔ 这不是第四份状态：它读的就是音频线程拿去投影的那个字段本身。
    ///
    /// 没有引擎 ⇒ `None`（不猜）。
    // ⚠ `#[allow(dead_code)]` 的必要性见本文件 `LiveUi::dispatch_key_chord` 上方的**近例**：
    // 本文件被 43 个测试目标经 `#[path]` 各自装成私有模块（2026-11-03 现读；单位：个目标），
    // 某条读数只要有一个目标没调它，
    // `dead_code` + `-D warnings` 就把它变成编译失败（CI 实测红在 `test "live_ui_mcp"`：
    // `error: method engine_metronome_enabled is never used`）。⛔ 不是藏死代码：这些读数在
    // `tests/d_ui_29_*` / `tests/d_ui_30_*` 里有真实调用者。
    #[allow(dead_code)]
    #[must_use]
    pub fn engine_metronome_enabled(&self) -> Option<bool> {
        Some(
            self.surface
                .engine
                .borrow()
                .current_snapshot()?
                .metronome_enabled(),
        )
    }

    /// ⭐ 装配**这一条走带**是否在跑 —— 读的是 `EngineHost::transport()`（RT→控制侧的
    /// 镜面读数），⛔ 不是界面属性 `playing`（那只是它的投影）。
    ///
    /// `D-UI-30` 用它见证"界面上的播放按钮真的把命令发给了**本装配**的宿主"：
    /// 没起引擎时 `EngineHost::send_transport` 如实返回 `TransportReading::cold()`
    /// （不 panic、不假装），因此这一位在 `start_engine()` **之前**按下会是 `false`
    /// —— 那正是"没有引擎"的事实。
    // ⚠ `#[allow(dead_code)]`：同款近例见 `engine_metronome_enabled` 上方那条
    // （本文件被多个测试目标经 `#[path]` 各装一份，本读数的调用者是 `tests/d_ui_30_*` 与
    // `tests/d_ui_32_*`）。
    #[allow(dead_code)]
    #[must_use]
    pub fn engine_transport_running(&self) -> bool {
        self.surface.engine.borrow().transport().state.is_running()
    }

    /// ⭐ 装配这一条走带**累计被应用**的命令条数（`TransportReading::commands_applied`）。
    ///
    /// 它是"按钮 ⇒ 引擎"那条链的直接读数：0 = 一条都没发出去（回调没人注册，或没有引擎），
    /// `>0` = 发出去了。⛔ 不是从界面属性反推的。
    // ⚠ `#[allow(dead_code)]`：同款近例见 `engine_metronome_enabled` 上方那条
    // （本文件被多个测试目标经 `#[path]` 各装一份，本读数的调用者是 `tests/d_ui_30_*` 与
    // `tests/d_ui_32_*`）。
    #[allow(dead_code)]
    #[must_use]
    pub fn engine_transport_commands_applied(&self) -> u64 {
        self.surface.engine.borrow().transport().commands_applied
    }

    /// ⭐ 装配这一条走带的**绝对位置**（单位：tick，960 PPQ；读 `TransportReading`）。
    ///
    /// `D-UI-30` 用它判「停止按钮 = 停住**并回起点**」这条语义：起播 + 推量子 ⇒ 位置离开 0，
    /// 按停止 ⇒ 状态停 **且** 位置回 0（⛔ 只看状态会把"停在中途"读成通过）。
    // ⚠ `#[allow(dead_code)]`：同款近例见 `engine_metronome_enabled` 上方那条
    // （本文件被多个测试目标经 `#[path]` 各装一份，本读数的调用者是 `tests/d_ui_30_*` 与
    // `tests/d_ui_32_*`）。
    #[allow(dead_code)]
    #[must_use]
    pub fn engine_transport_position_ticks(&self) -> u64 {
        self.surface.engine.borrow().transport().position_ticks
    }

    /// ⭐ 装配**那一格活投影**里的「当前场景」名（没有启动过场景 ⇒ `None`）。
    ///
    /// `D-UI-31` 用它证一件光看界面属性看不出来的事：[`crate::host::wire_scene_launch`]
    /// 成功后把 `CurrentScene` 写进的是**本执行面读的那同一格**投影，而不是接线时的
    /// 一份装配期快照。判据的对照形状是"界面镜面 `current-scene-index` 已是 2，
    /// 而执行面自己那份投影还说没有场景" —— 那正是两份投影各说各话。
    ///
    /// ⛔ 不是第四份状态：它 `borrow` 的就是 `LiveAdminSurface.view` 那一格。
    /// 没有 `borrow_mut` ⇒ 本读数⛔ 不参与任何写入。
    // ⚠ `#[allow(dead_code)]`：同款近例见 `engine_metronome_enabled` 上方那条
    // （本文件被多个测试目标经 `#[path]` 各装一份，本读数只有 `tests/d_ui_31_*` 调）。
    #[allow(dead_code)]
    #[must_use]
    pub fn view_current_scene_name(&self) -> Option<String> {
        self.surface
            .view
            .borrow()
            .current_scene
            .as_ref()
            .map(|scene| scene.name.clone())
    }

    /// ⭐ 活投影里第 `index` 个**段落**的起点 tick（下标越界 ⇒ `None`，不猜）。
    ///
    /// `D-UI-31` ④⑤ 用它对账「章节卡片按下去之后的引擎位置」：期望值必须从**这一格**
    /// 投影现读 —— 换工程之后若装配用的是装配期快照，这里读到的是新工程的段落而
    /// 位置停在旧工程的 tick，那条红就是本票要抓的形状。
    // ⚠ `#[allow(dead_code)]`：同款近例见 `engine_metronome_enabled` 上方那条。
    #[allow(dead_code)]
    #[must_use]
    pub fn view_section_start_tick(&self, index: usize) -> Option<u64> {
        self.surface
            .view
            .borrow()
            .sections
            .get(index)
            .map(|section| section.start_tick)
    }

    /// ⭐ 活投影里第 `index` 个**场景**的名字（下标越界 ⇒ `None`）。
    ///
    /// `D-UI-31` ⑥ 用它对照状态行里那句「已排队：{name}」，并顺手证"下标解析用的是
    /// 当时那一格投影"（换工程之后同一个下标对应的是**新**场景的名字）。
    // ⚠ `#[allow(dead_code)]`：同款近例见 `engine_metronome_enabled` 上方那条。
    #[allow(dead_code)]
    #[must_use]
    pub fn view_scene_name(&self, index: usize) -> Option<String> {
        self.surface
            .view
            .borrow()
            .scenes
            .get(index)
            .map(|scene| scene.name.clone())
    }

    /// 音频读路径累计换过多少次快照（`SnapshotReader::begin_block` 里的计数）。
    #[must_use]
    pub fn engine_snapshot_switches(&self) -> Option<u64> {
        self.surface
            .engine
            .borrow()
            .engine_stats()
            .map(|stats| stats.snapshot_switches)
    }

    /// ⭐ 当前引擎快照里某轨的**摊平 SFZ 读数**：`(区域数, 内存字节数, 缺样本路径)`。
    ///
    /// 三个量都有单位（区域数 = 条，内存 = 字节，缺样本 = 条路径），全部取自
    /// `SfzPlan::prepared()` 的**公开读数** —— ⛔ 不是"看起来摊平了"的推断。
    ///
    /// `None` = 没有引擎 / **该轨没有摊平表**（未绑定，或绑了但还没摊平）。
    /// 判据靠它区分"已摊平"与"还没摊平"：后者正是"用户以为换了 SFZ、其实没生效"的
    /// 可观测形态（任务单 `#80`/`#82`）。
    #[must_use]
    pub fn engine_sfz_plan_readings(
        &self,
        track_id: &EntityId,
    ) -> Option<(usize, usize, Vec<String>)> {
        let snapshot = self.surface.engine.borrow().current_snapshot()?;
        let prepared = snapshot.sfz_plan(track_id)?.prepared();
        Some((
            prepared.region_count(),
            prepared.memory_bytes(),
            prepared.missing_sample_paths().to_vec(),
        ))
    }

    /// ⭐ 当前引擎快照里**待摊平**（只带路径、还没读盘）的条数。
    ///
    /// `None` = 没有引擎。`Some(0)` = 已摊平 / 一条绑定都没有。
    #[must_use]
    pub fn engine_sfz_pending_len(&self) -> Option<usize> {
        Some(
            self.surface
                .engine
                .borrow()
                .current_snapshot()?
                .sfz_pending()
                .len(),
        )
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

    /// 在这个活窗口上注入一次组合键（`ui/dispatch_key_chord` 的**本体**，缺陷 `D-UI-3`）。
    ///
    /// 存在的理由与 [`Self::dispatch_key_press`] 逐字相同（判据既要走"注入 → 回调 →
    /// 键位表 → 投影"这一段，又要保留 [`Self::tree_snapshot`] / [`Self::capture`] 的读数
    /// 能力 —— `into_control_plane` 会把执行面装箱取走）。
    ///
    /// ⚠ 与 `dispatch_key_press` **不同格**：那条落 `self.inner`（Slint **平台事件**通道），
    /// 这条落 `LiveAdminSurface` 自己的 `dispatch_key_chord_impl`（`.slint` 派发快捷键的
    /// 那个回调 `invoke_key_action`），因为平台事件带不了修饰位。返回值是**界面是否消费**
    /// （`false` = 这一组合没有绑定），⛔ 不是"调用失败"。
    // ⚠ 与模块头 §1 的取向**相冲**，因此把冲在哪写清楚：§1 说"判据只住一个目标 ⇒ 没人用
    // 就删"，但那条前提今天已不成立 —— `grep -l 'path = "../src/live_surface.rs"' tests/`
    // 实测 **43** 个目标装本文件（2026-11-03 现读；单位：个目标），任何一个目标看不到某条
    // 入口都会报 `dead_code`
    // （CI 实测：`error: method dispatch_key_chord is never used`，红在 `live_ui_mcp` 那份），
    // `-D warnings` 再把它变成编译失败。处置按本文件的**近例**（`open_project_via_picker`、
    // `capture` 同款）。⛔ 这不是"藏死代码"：本入口在产品腿
    // 上有真实调用者 —— `ui/dispatch_key_chord` 经 `LiveAdminSurface::dispatch_key_chord_impl`
    // 走的就是同一条实现，被装进测试目标时它只是**那个**目标没调它。
    #[allow(dead_code)]
    pub fn dispatch_key_chord(
        &mut self,
        chord: &yeban_ui_test_port::port::KeyChord,
    ) -> Result<bool, PortError> {
        self.surface.dispatch_key_chord(chord)
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

    /// ⭐ **① 打开工程 —— 原生对话框那一条入口**（任务单 `#86` 乙）。
    ///
    /// 它做的只有两件事，顺序是契约：
    /// 1. 问 [`yeban_app::file_picker::FilePicker`] 要一个路径（**可注入** ⇒ 判据能测
    ///    "对话框被调用了"与"选了路径 ⇒ 工程真的换了"）；
    /// 2. 把选中的路径交给**已有的** [`Self::open_project`] —— ⛔ 本方法**不**另写
    ///    第二条"读盘 + 换工程"的实现，因此权限闸门 / 权威与撤销端口的在场检查 /
    ///    容器裁决全部照旧走原来那一处。
    ///
    /// ## 返回形状（三种结局分得开）
    ///
    /// * [`OpenViaPicker::Cancelled`] —— 用户取消（或没有对话框可用）。
    ///   ⚠ **不是错误**：调用方什么都不做，⛔ 不把它报成"打开失败"。
    ///   ⛔ 因此它**不**返回 `Result`：把"我不想"塞进 `Err` 就是把两件事混成一句。
    /// * [`OpenViaPicker::Opened`] —— 对话框给了路径，且 `open_project` 成功。
    /// * `Err(PortError)` —— 对话框给了路径，但**打开被拒**（权限 / 目标打不开 / …）。
    ///   这里⛔ 不吞错：拒绝理由原样上抛（与 `open_project` 自己的契约一致）。
    ///
    /// ## 边界（如实登记）
    ///
    /// ⚠ 本仓**产品** `run_gui` 里今天**没有**"运行期打开工程"的实现（`--open` 只在
    /// 启动时读一次），因此本入口接在**已有的**运行期打开路径上（`ui/open_project` /
    /// [`Self::open_project`]）。"给产品加一个可见的打开入口"是**另一个缺口**，
    /// 本单⛔ 不假装接上（它会改默认帧 ⇒ 需要重录 `tests/golden/linux/**`）。
    ///
    /// # Errors
    ///
    /// 同 [`Self::open_project`]：端口拒绝（权限不足 / 当前工程没有落点 / 目标打不开 /
    /// 权威或撤销端口在场）。
    // ⚠ `#[allow(dead_code)]`：本文件会被 `#[path]` 装进**十几个**集成测试目标，
    // 而"原生对话框打开工程"只有本单那条判据用到 ⇒ 其余目标（如 `live_ui_mcp`）会报
    // dead_code，而 `-D warnings` 把那条变成编译失败。⛔ 这不是"藏死代码"：
    // 本入口在产品腿上有真实用途（见上面的边界登记）。
    #[allow(dead_code)]
    pub fn open_project_via_picker(
        &mut self,
        picker: &dyn yeban_app::file_picker::FilePicker,
        save_first: bool,
    ) -> Result<OpenViaPicker, PortError> {
        let request =
            yeban_app::file_picker::PickerRequest::open("打开工程").with_extensions(["yeban"]);
        let Some(path) = picker.choose_open(&request) else {
            return Ok(OpenViaPicker::Cancelled);
        };
        let text = path.to_string_lossy().into_owned();
        self.open_project(&text, save_first)?;
        Ok(OpenViaPicker::Opened { path })
    }
}

/// [`LiveUi::open_project_via_picker`] 的**两种成功结局**（取消不是错误）。
///
/// ⚠ `#[allow(dead_code)]` 的理由与上面那个方法**逐字相同**（`#[path]` 装进多个目标）。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenViaPicker {
    /// 用户取消（或没有对话框可用）⇒ 调用方**什么都不做**。
    Cancelled,
    /// 对话框给了路径，且工程已经真的换过了。
    Opened {
        /// 对话框给出的那个路径（**未**做任何改写 —— 本层不加后缀、不规范化）。
        path: std::path::PathBuf,
    },
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
    /// 两个用途：读 [`Self::rendered_frames`] 这个"读控件树不得渲染一帧"的**成本**
    /// 探针；以及给判据抓像素证据（[`Self::capture`]）—— 拖动判据要在注入事件之后
    /// 逐像素比对「原位置 ∪ 预览位置」，而控制面已经把执行面装箱拿走了。
    window: Tier1Window,
}

impl LiveControlPlane {
    /// 借出控制面（发 `ui/*` 调用）。
    pub fn plane(&mut self) -> &mut ControlPlane {
        &mut self.plane
    }

    /// 从**同一个**活窗口抓一帧（`Tier1Window` 是共享句柄，不会造出第二份窗口状态）。
    ///
    /// 与 [`LiveUi::capture`] 走的是同一条光栅化路径；差别只是控制面把执行面装箱
    /// 取走之后，判据仍需要**逐像素**证据（`ui/screenshot` 只给 PNG 字节与指纹）。
    ///
    /// # Errors
    ///
    /// 光栅化 / 抓帧失败。
    #[allow(dead_code)] // `#[path]` 装进多个测试目标：不是每个目标都用它（模块头同款）。
    pub fn capture(&self) -> Result<Rgb8Image, PortError> {
        self.window.capture().map_err(|err| PortError::Capture {
            message: err.to_string(),
        })
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
    // ⭐ 抽屉的来源**由本装配入口决定**，不接受调用方给的那一份：既然这个执行面的工程
    // 唯一来源是权威，"看不见权威里的提案"就不可能在这里成立（覆盖而不是合并）。
    let options = LiveWiringOptions {
        proposal_source: yeban_app::proposals::ProposalSource::from_authority(authority),
        ..options.clone()
    };
    let mut ui = build_live_ui_with(&project, &options)?;
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
    // ⭐ `D-UI-31`：**活投影**那一格 —— 与本执行面**同一个** `Rc<RefCell<ViewState>>`，
    // 交给下面每一条"要读投影"的接线（资源栏 / 剪辑作用域 / 标尺 / 段落 / 场景）。
    // 本地那份 `view` 从此只用来播种窗口（`build_main_window_with_console_tab` 的注入），
    // ⛔ 不再被任何回调持有：旧写法给 `wire_sidebar_contents` 与 `wire_clip_scope`
    // **各造一份** `Rc<RefCell<view.clone()>>`，换工程之后两份都停在装配期那个工程上。
    let live_view = Rc::new(RefCell::new(view.clone()));
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
    host::wire_keys(&window, Rc::clone(&input), options.undo.clone(), None, None);
    // `[UI-NOTE-003]` §3.3 工具矩阵的**点击**入口（台账 R16）：与产品进程
    // （`main.rs` 的 `run_gui`）调的是**同一个** `host::wire_tool_select`，因此无头端口注入的
    // 真实指针事件与用户点击走同一条链。它不改工程 ⇒ 只在装配给了端口时把端口一并交给
    // 那**同一个** `Action::SelectTool` 臂（`None` 时照旧写界面属性，与 `wire_keys` 同款）。
    host::wire_tool_select(&window, options.undo.clone());
    // **纯视图态的四条回调**（`toggle-view` / `toggle-sidebar` / `toggle-ai-drawer` /
    // `open-musical-pr`）：与产品进程**同一个**接线函数（`main.rs` 的 `wire_callbacks`
    // 也调它），因此端口注入的点击与用户点击走的是同一条链
    // （真实指针事件 → Slint 命中测试 → `.slint` 的 `TouchArea` → 宿主写属性）。
    // 只有属性写入 ⇒ 不改工程、不需要 `UndoPort`，装配顺序上也不与撤销端口耦合。
    host::wire_view_callbacks(&window);
    // ⭐ 左侧资源栏的条目**取数**（分类 / 条目点击 ⇒ 重注入数组）：与产品进程
    // （`main.rs` 的 `wire_callbacks`）调的是**同一个** `host::wire_sidebar_contents`，
    // 而且拿的是**同一格**活投影（`D-UI-31`）⇒ 换工程之后分类计数与条目数组跟着新工程走。
    host::wire_sidebar_contents(&window, Rc::clone(&live_view));
    // ⭐ AI 提案抽屉：与产品进程调的是**同一个** `host::wire_proposal_drawer`
    // （`open-musical-pr` 的注册者只有它，因此上一行不再碰这个回调）。来源由装配决定：
    // 接在权威上的装配 ⇒ 打开抽屉时现读权威那份未合并清单；没接权威的装配 ⇒ 空态那句
    // "没有来源"。⛔ 判据侧不存在"绕过来源直接写属性"的第二条路。
    host::wire_proposal_drawer(&window, options.proposal_source.clone());
    // ⭐ `[UI-NOTE-001]` 剪辑选中态 ⇒ 卷帘作用域：与产品进程（`main.rs` 的 `wire_callbacks`）
    // 调的是**同一个** `host::wire_clip_scope`，同样拿**同一格**活投影（`D-UI-31`；
    // 旧写法在这里也 `view.clone()` 造了**第三份**投影）。
    host::wire_clip_scope(&window, Rc::clone(&live_view), options.undo.clone());
    // 混音台（控制台 Tab 2）的**写入面**：与产品进程（`main.rs` 的 `wire_callbacks`）调的是
    // **同一个**函数。它需要 `UndoPort`（工程内容必须走 `Op`，`ADR-0005`）⇒ 只在装配给了
    // 端口时接上；`options.undo` 为 `None` 的既有判据因此照旧"没有写入面"，行为一位不变。
    if let Some(undo) = options.undo.as_ref() {
        host::wire_mixer_edit(&window, undo);
        // ⭐ 设备效果链（控制台 Tab 3）的写入面（任务单 `#157`）：与产品进程
        // （`main.rs` 的 `wire_callbacks`）调的是**同一个** `host::wire_device_rack`，
        // 因此无头端口注入的手势与用户点击走同一条链（`Op` + `UndoPort`）。
        host::wire_device_rack(&window, undo);
        // ⭐ 发送条（控制台 Tab 3 的同一面板）的写入面（任务单 `#159`）：与产品进程
        // （`main.rs` 的 `wire_callbacks`）调的是**同一个** `host::wire_send_strip`，
        // 因此无头端口注入的手势与用户点击走同一条链（`Op` + `UndoPort`）。
        host::wire_send_strip(&window, undo);
        // ⭐ 宏面板（Tab 3 头部「宏 (N)」打开的右侧抽屉）的写入面：与产品进程
        // （`main.rs` 的 `wire_callbacks`）调的是**同一个** `host::wire_macro_panel`，
        // 因此无头端口注入的滑块手势与用户点击走同一条链（`Op` + `UndoPort`）。
        host::wire_macro_panel(&window, undo);
        // ⭐ 走带条 BPM 敲入（回车 / `accepted`）的写入面：与产品进程
        // （`main.rs` 的 `wire_callbacks`）调的是**同一个** `host::wire_bpm_commit`，
        // 因此无头端口注入的 `invoke_bpm_commit` 与用户按回车走同一条链（`Op` + `UndoPort`）。
        host::wire_bpm_commit(&window, undo);
        // ⭐ 轨 / 路由（控制台 Tab 3 的**同一面板**）的写入面（任务单 `#160`）：与产品进程
        // （`main.rs` 的 `wire_callbacks`）调的是**同一个** `host::wire_track_strip`，
        // 因此无头端口注入的建轨 / 删轨手势与用户点击走同一条链（`Op` + `UndoPort`）。
        host::wire_track_strip(&window, undo);
        // ⭐ 建空的 MIDI 片段（控制台 Tab 3 的**同一面板**，任务单 `#163`）：与产品进程
        // （`main.rs` 的 `wire_callbacks`）调的是**同一个** `host::wire_clip_create`，
        // 因此无头端口注入的建片段手势与用户点击走同一条链（`Op` + `UndoPort`）。
        host::wire_clip_create(&window, undo);
        // `[UI-NOTE-003]` 卷帘的工具派发（铅笔 / 橡皮擦）与产品进程
        // （`main.rs` 的 `host::wire_roll_edit`）调的是**同一个**函数。
        //
        // 选区的点击派发器**已经**由 `host::build_main_window_with_console_tab` 装好
        // （`install_roll_click` 的唯一注册点）⇒ 无头面与生产面从此不再分叉：
        // 有端口时两边都"选区 + 铅笔 + 橡皮擦"，没端口时两边都只做选区。
        host::wire_roll_edit(&window, undo);
        // ⭐ 编曲剪辑平移（拖动面）的写入面：与产品进程（`main.rs` 的
        // `host::wire_clip_drag`）调的是**同一个**函数 ⇒ 无头端口注入的拖动
        // 与用户手势走同一条链（拖动期只写草稿，松手 `commit_ops` 一次）。
        host::wire_clip_drag(&window, undo);
        // ⭐ 缺陷 `D-UI-11`（2026-11-01，用这条控制面创作第二首时撞上）：**轨道头的
        // 三条手势在本装配里没接线**。
        //
        // `--enable-ui-mcp-http` 在 `main.rs` 的 `main()` 里提前分流进 `ui_mcp_serve::run`，
        // ⛔ 永远不执行 `run_gui` 的那一段接线（`host::wire_track_select` /
        // `wire_track_height_drag` / `wire_undo` 都在那一段里）。Slint 的回调没注册
        // handler 就是空操作 ⇒ 在这条执行面上：
        //   · 点轨道行**不选轨**（`selected-track-id` 没有写者）⇒ 设备链 / 建片段 /
        //     铅笔提交**恒指向第 1 条轨**（实测：UI 写出来的 6 轨工程里 15 颗剪辑
        //     100% 挂在 MIDI 1，其余 5 条一颗没有）；
        //   · `Alt+↑` / `Alt+↓` 被键面**消费**（`consumed = true`）却什么都不改 ——
        //     比"不消费"更坏：调用方读到的是"这一键有绑定"（`host::apply_action` 的
        //     `SelectTrackBy` 臂落到 `ui.invoke_track_select`，而那条回调没人听）；
        //   · 拖轨道行**不改行高**（整块行头都是拖拽面，按下 = 选中 + 抓住行高）；
        //     界面上的撤销入口（撤销弹窗开关 / 「撤销一步」/ 版本节点 / 走带区 ↻ /
        //     诊断导出）**全部空转** —— `Ctrl+Z` 走 `wire_keys` 那条链照常有效，
        //     分叉只在按钮这一格。
        //
        // ⚠ 为什么此前没有判据抓到它：`tests/track_selection.rs` 那条**名字叫
        // `clicking_the_track_header_...`** 的判据实际调
        // `invoke_accessible_default_action()`（无障碍默认动作），而 `ui/dispatch_pointer_*`
        // 走的是 `window.pointer_down(窗口坐标)` ⇒ Slint 命中测试，两条路在 `.slint` 里
        // 分别落在 Rectangle 的动作与该行的 `TouchArea` 上。判据侧过去**各自补一次**
        // `host::wire_track_select`（`tests/track_selection.rs` /
        // `tests/track_select_keyboard.rs` / `tests/mixer_keyboard_nudge.rs` /
        // `tests/mixer_keyboard_switch.rs`），于是"这份装配接没接"永远读不出来。
        // 同一族的防线是 `tests/d_ui_11_serve_header_gestures.rs` 的名单对账。
        host::wire_track_select(&window, undo);
        host::wire_track_rename(&window, undo);
        host::wire_track_arm(&window);
        host::wire_automation_lane_click(&window, undo);
        // ⭐ 2026-11-07 夜半 UI 轨道交互修复线：缩放控件 / 移轨 / 横向滚动
        // （与 `main.rs` 的产品进程调**同一个**函数 ⇒ 无头面与生产面不分叉；
        // `tests/d_ui_11_serve_header_gestures.rs` 的名单对账要求两处都在场）。
        host::wire_zoom_controls(&window, undo);
        host::wire_track_reorder(&window, undo);
        host::wire_arrangement_wheel(&window, undo);
        host::wire_arrangement_hscroll(&window, undo);
        // `ADR-0004` S1 的纵向入口：与 `wire_track_select` 共用轨道头那一块 `TouchArea`
        // （按下 = 选中 + 抓住行高，移动 = 改高，松手 / `Escape` = 收尾）。
        host::wire_track_height_drag(&window, undo);
        // 撤销的**界面**入口那六个回调（见上面的分解）：产品进程在 `run_gui` 里接的就是
        // 这一对，本装配过去只接了按键那一条。
        host::wire_undo(&window, undo);
    }
    // **四个模块入口**（导出 WAV / 导入 MIDI / 装载 SFZ / 录音）：与产品进程
    // （`main.rs` 的 `run_gui`）调的是**同一个** `host::wire_module_actions`。
    // 它**总是**接：三条只读动作（导出 / 装载 / 录音）在 `options.undo` 为 `None` 时
    // 照常可用，只有 `import-midi` 会如实拒绝（`error=no-undo-port`）——
    // ⛔ 不静默用一个临时会话把编辑吞掉。
    // ⭐ 审评 2026-10-31（P2-3）：`export_wav` 缺省取工程目录（`save_path` 是文件，
    // 故取其父目录；未保存工程 ⇒ `None` ⇒ 相对缺省，旧行为）。
    host::wire_module_actions_for_project(
        &window,
        options.undo.clone(),
        options.save_path.as_deref().and_then(|path| path.parent()),
    );
    // ⭐ `[PATCH UI-WIRING-01]`：**WAV 导出选项框的 `Ctrl/Cmd+O` 路径浏览** 在本装配也要有注册者，
    // 否则 Slint 的 `on_*` 空实现 ⇒ 控制面上按下去什么都不动（与 `D-UI-8` 同族）。
    // 本档没有 OS 窗口 ⇒ 接 `NoPicker`（`choose_save` 恒 `None` = 每次都取消），
    // 与 `wire_save` 那一处逐字同款；`ui/property` 直写 `export-wav-path` 的既有通道不变。
    host::wire_export_wav_browse(&window, std::rc::Rc::new(yeban_app::file_picker::NoPicker));
    // ⭐ 缺陷 `D-UI-8`（2026-11-01，创作第二首时撞上）：**保存入口在本装配里没接线**。
    //
    // 上面的那九个 `wire_*` 是**改工程**的写入面，而"把改完的工程写进文件"这一条
    // 曾经只有 `ui/force_save`（一条**控制面命令**，见 `LiveAdminSurface::save_now`）——
    // `host::wire_save` 的唯一调用点在 `main.rs` 的 `run_gui`，而本档在 `main.rs`
    // 里**直接 `return`**，`run_gui` 根本不跑。后果是用户可见的：顶栏那颗
    // `transport-save-button` 与 `Ctrl+S` 都调 `invoke_save_project`，而回调**没有注册者**
    // ⇒ Slint 空操作。它比"报错"更坏：`save-status` 一位不写，而状态栏那行文本只在
    // `save-status != ""` 时才进控件树 ⇒ 自动化界面**读不到任何失败**，人只看到"点了没反应"。
    //
    // 接法与产品 `run_gui` 逐字同源（同一条 `Op`/会话 → `save_action::dispatch_save` →
    // `apply_save_outcome` 写界面回执），只有一处按本档的形态收窄：
    //
    // | 参数 | 产品 `run_gui` | 本装配 | 为什么 |
    // | :--- | :--- | :--- | :--- |
    // | `port.target` | `--open` 的那个文件 | 同一个来源（`options.save_path`） | 落点只能是装配期给的那个文件 |
    // | `live_port` | 活会话端口 | `options.undo` —— 与那九个写入面**同一个** `Rc` | 写出去的是此刻的工程，不是装配快照 |
    // | `picker` | `NativeFilePicker` | `yeban_app::file_picker::NoPicker` | 本档**没有 OS 窗口**（模块文档：只装 Tier-1 软件平台），弹一次模态原生框既没人能答、又会把控制面钉死。给了落点时产品腿也一次都不问 ⇒ 差异只在"没给 `--open`"那一档 |
    // | `authority` | 挂载时给权威 | `None` | 本装配不挂进程内权威（挂权威的是 `build_live_ui_from_authority_with`，它的 `ui/force_save` 仍走权威那条腿） |
    // | `engine` | 走带那一份宿主 | `None` ⇒ 与 `host::wire_save` 逐位相同 | `NoPicker` ⇒ 「另存为」在本档问不出路径，落点恒为装配期那个文件 ⇒ `#170` 的"目录跟着新文件走"这一跳没有可到达的输入 |
    //
    // ⚠ 由此留下的**如实空白**（已进创作审核报告，不在代码里假装）：本档没给 `--open`
    // 时保存按钮仍是空操作（`NoPicker` 返回 `None` ⇒ 回调提前返回，与"取消不是失败"同一条
    // 口径）。要它变成一句可见的失败，得先决定"无 OS 窗口的界面该怎么问落点"——
    // 那是规范缺口，不是这里能私自补齐的答案。
    host::wire_save(
        &window,
        host::SaveStatus {
            target: options.save_path.clone(),
            project: project.clone(),
        },
        options.undo.clone(),
        std::rc::Rc::new(yeban_app::file_picker::NoPicker),
        None,
    );
    // ⭐ 缺陷 `D-UI-30`（2026-11-03，与 `D-UI-11` 同族、同一格漏）：**走带全家在本装配里
    // 没接线**。`host::wire_transport` 的唯一调用点过去在 `main.rs` 的 `wire_callbacks`，
    // 而 `--enable-ui-mcp-http` 在 `main()` 里提前分流进 `ui_mcp_serve::run` ⇒ 那一段
    // 根本不跑（实测：修前本文件对 `wire_transport` 的引用数是 **0** 处，`main.rs` 是
    // 文本 3 处 / 代码 **1** 处（另两处是注释）—— 单位：处）。
    // Slint 的 `on_*` 没注册者就是空操作，于是这份**人真的拿来作曲的执行面**上：
    //   · 播放 / 停止 / 录音 / 节拍器四颗按钮按下去什么都不动；
    //   · `Space` 比"不消费"更坏 —— `wire_keys` 的 `PlayPause` 臂落到
    //     `ui.invoke_toggle_play()`（`host::apply_action` 返回 `true` = "这一键有绑定"），
    //     而那条回调没人听 ⇒ 界面读起来是"按了没反应"；
    //   · `metronome-on` 的**装配期初值**也一起丢（`wire_transport` 是它唯一的初值写者）。
    //
    // ⚠ 为什么此前没有判据抓到它：判据侧**各自补一次** `host::wire_transport`
    // （`tests/metronome_switch.rs` / `tests/rewind_sound.rs` /
    // `tests/transport_timecode_tick.rs` / `tests/arrangement_ruler_seek.rs` /
    // `tests/d_ui_11_serve_header_gestures.rs`），而且补的是**自己新建的**
    // `Rc<RefCell<EngineHost>>` ⇒ "这份装配接没接、接的是不是同一个宿主"永远读不出来。
    // 同一族的防线是 `tests/d_ui_11_serve_header_gestures.rs` 的名单对账。
    //
    // 因此宿主从裸值升成 `Rc<RefCell<..>>`（与 [`host::ProductionLoop`] / `main.rs` 同款
    // 范式；`Rc` 计数 +1，⛔ 不是第二份宿主），并把**同一个**实例交给 `wire_transport`
    // ⇒ 本执行面的 `engine_tick` / `rebuild_engine` 与用户点击作用在同一条走带上。
    // 工程参数给装配期那份克隆：`apply_project` 换工程后会按 `main.rs` 的 ③ 同款
    // **重接**（`on_*` 是覆盖语义），因此它不是第二份事实源。
    let engine: Rc<RefCell<EngineHost>> = Rc::new(RefCell::new(EngineHost::new()));
    host::wire_transport(
        &window,
        Rc::clone(&engine),
        Some(Rc::new(project.clone())),
        options.undo.clone(),
    );
    // ⭐ 缺陷 `D-UI-31`（2026-11-03，与 `D-UI-11` / `D-UI-30` 同族、同一格漏）：**定位族
    // 在本装配里没接线**。三条的唯一调用点过去都在 `main.rs` 的 `wire_callbacks`
    // （现读：`main.rs` = 3 处、本文件 = 修前 **0** 处；单位：处），而 `--enable-ui-mcp-http`
    // 在 `main()` 里提前分流进 `ui_mcp_serve::run` ⇒ 那一段根本不跑。Slint 的 `on_*`
    // 没注册者就是空操作，于是这份执行面上：
    //   · 拖标尺 / 点标尺不改播放头（`ruler-seek` 没有读者）；屏读器的「上/下一小节」
    //     与「定位到第 N 小节」同样空转；
    //   · 点编曲区的段落卡片不定位（`section-seek`）；
    //   · 点场景按钮不启动场景（`scene-launch`）—— 而 `ui/launch_scene` 那条**控制面**
    //     命令走的是另一条路，因此"工具能做、界面上点不动"这件事在这份装配里一直存在。
    //
    // ⚠ 为什么此前没有判据抓到它：判据侧各自补一次接线（`tests/arrangement_ruler_seek.rs`
    // 就是"新建一个 `Rc<RefCell<EngineHost>>` + 新建一份 `Rc<RefCell<ViewState>>` 再
    // `wire_ruler_seek`"），于是"这份装配接没接、接的是不是同一个宿主/同一格投影"
    // 从外面读不出来。同一族的防线是 `tests/d_ui_11_serve_header_gestures.rs` 的名单对账
    // （本票把那三条从"只在 GUI 有"的例外清单里删掉）。
    //
    // 与 `D-UI-30` 的区别在**投影**那一半：这三条要的不是"重接一次"，而是**同一格活的**
    // `Rc<RefCell<ViewState>>` —— 它们从投影现读小节长度 / 网格右缘 / 段落起点 / 场景 tempo。
    // 拿装配期快照会在换工程之后静默停在旧工程（`apply_project` 有两个重投影点，
    // 靠"每处重接"追平就是把 `D-UI-10` 那一族重新写一遍），因此本票把 `view` 字段
    // 升成共享格（与 `main.rs` 的 `view_cell` 同款），接线的闭包与本执行面读同一份。
    host::wire_ruler_seek(&window, Rc::clone(&engine), Rc::clone(&live_view));
    host::wire_section_seek(&window, Rc::clone(&engine), Rc::clone(&live_view));
    host::wire_scene_launch(&window, Rc::clone(&engine), Rc::clone(&live_view));
    let admin = LiveAdminSurface {
        inner: surface,
        window,
        registry,
        project: project.clone(),
        // 这里是把**那一格本身移进去**（⛔ 不是再造一份）⇒ 本执行面与上面五条接线
        // 读的永远是同一份投影。
        view: live_view,
        save_path: options.save_path.clone(),
        meters: MeterRuntime::empty(),
        engine,
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
    // ⭐ 任务单 `#168`：装配期给了保存落点（`--open` / `--save`）⇒ 那个文件所在目录
    // 就是相对 SFZ 路径的第一个候选根。工程不是从文件打开的 ⇒ `None`（按仓库根 ⇒ cwd）。
    let project_dir = admin
        .save_path
        .as_deref()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf);
    admin.engine.borrow_mut().set_project_dir(project_dir);
    Ok(LiveUi {
        reference,
        surface: admin,
        scene,
    })
}
