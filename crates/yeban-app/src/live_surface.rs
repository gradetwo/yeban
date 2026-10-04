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

use std::path::PathBuf;

use registry_tree::control_tree_from_registry;

use slint::ComponentHandle as _;

use yeban_app::bridge::{BridgeError, ViewState};
use yeban_app::elements::ElementRegistry;
use yeban_app::engine_host::{EngineHost, EngineHostError};
use yeban_app::host;
use yeban_app::meters::{MeterRuntime, MeterSnapshot};
use yeban_app::save::{SaveError, save_project_file};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_engine::meter::MeterCollector;
use yeban_model::YebanProjectV1;
use yeban_ui_mcp::live::ControlPlane;
use yeban_ui_mcp::surface::{AdminReport, PortAdapter, ReportValue, UiSurface};
use yeban_ui_test_port::port::{Permission, PointerButton, PortError, UiTestPort};
use yeban_ui_test_port::render::{LivePort, RenderError};
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
}

impl Default for LiveWiringOptions {
    fn default() -> Self {
        Self {
            permission: Permission::ReadOnly,
            console_tab: 0,
            save_path: None,
            engine_quanta: 8,
        }
    }
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
}

impl core::fmt::Display for LiveWiringError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Project(error) => write!(f, "工程投影失败: {error}"),
            Self::Tree(error) => write!(f, "语义注册表适配失败: {error}"),
            Self::Render(error) => write!(f, "Tier-1 执行面装配失败: {error}"),
            Self::Capture(error) => write!(f, "Tier-1 抓帧失败: {error}"),
            Self::Engine(error) => write!(f, "引擎重建失败: {error}"),
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

/// **管理动作的真正落地 + 电平消费** —— 包住 `PortAdapter<LivePort<MainWindow>>`。
///
/// 它只覆写三个 `*_impl`（`[UI-MCP-001]` §12.3 的 Administrative 一级），其余全部原样
/// 委托给内层执行面。**不**改 `PortAdapter` / `LivePort` / `yeban-ui-test-port` 的公开 API：
/// 那三个 crate 不是本线的地盘，而且"给一个测试端口加产品语义"会污染它们的依赖方向。
///
/// 三个动作各自的可观测副作用：
///
/// | 动作 | 真的做了什么 | 界面/接口上能看到什么 |
/// | :--- | :--- | :--- |
/// | `ui/switch_main_view` | `MainWindow.arrangement-view = view == "arrangement"` | 重抓控件树后 `workspace-session-canvas` / `workspace-arrangement-canvas` 互换；回执里带回读值 |
/// | `ui/force_save` | `save::save_project_file`（临时文件 → `sync_all` → 原子重命名） | 磁盘上真的出现可被 `open_project_file` 读回的容器；回执里有 `bytes` / `saveEpoch` |
/// | `ui/reload_engine` | `EngineHost::reload`（新快照 + 新队列 + 推 N 个量子） | **新**的电平队列被采纳 ⇒ 混音台电平回到下限；回执里有 `generation` / `quanta` / `meterFrames` |
struct LiveAdminSurface {
    inner: LiveSurface,
    /// 活窗口的强引用（`clone_strong`）。改视图 / 注入电平 / 重抓树都走它。
    window: MainWindow,
    /// 静态语义注册表（动态区标记的来源，重抓树时要重新注入）。
    registry: ControlTree,
    /// 当前工程与它的投影（换工程时一起换）。投影是纯函数，两份必然逐字节相同。
    project: YebanProjectV1,
    view: ViewState,
    save_path: Option<PathBuf>,
    meters: MeterRuntime,
    engine: EngineHost,
    engine_quanta: u64,
    save_epoch: u64,
    report: Option<AdminReport>,
}

impl LiveAdminSurface {
    /// 重抓运行时控件树（几何 / 可见性变了之后必须做，否则 `ui/tree` 还是旧的）。
    fn refresh_tree(&mut self) -> Result<usize, LiveWiringError> {
        Ok(self
            .inner
            .port_mut()
            .refresh_tree(Some(&self.registry))?
            .len())
    }

    /// 引擎重建（`ui/reload_engine` 与 `apply_project` 共用同一条路径）。
    fn rebuild_engine(&mut self) -> Result<yeban_app::engine_host::EngineRebuild, LiveWiringError> {
        let rebuild = self.engine.reload(&self.project, self.engine_quanta)?;
        Ok(rebuild)
    }

    /// 一轮电平消费：抽干 → 对齐工程 → 注入 Slint → 重抓树。
    ///
    /// **顺序是契约**：注入必须发生在重抓树之前，否则 `ui/tree` 读到的是上一帧的标签
    /// （判据 `mixer_meter_labels_match_the_injected_frames` 就是靠这一点）。
    fn pump_meters(&mut self) -> MeterSnapshot {
        self.meters.poll(self.view.tracks.len() + 1);
        let snapshot = self.meters.snapshot(&self.view);
        host::apply_meters(&self.window, &snapshot);
        // 电平标签变了 ⇒ 控件树必须重抓（失败也不该让一次电平刷新变成错误：
        // 界面已经是新值，重抓失败只是"控制面看到的树旧了一点"）。
        let _ = self.refresh_tree();
        snapshot
    }

    /// 换一个工程：投影 → 注入 → 换注册表 → 作废旧电平 → （若引擎已起）换代 → 重抓树。
    ///
    /// 它与 `main.rs` 的 `--project-sample` 走的是**同一条**数据流
    /// （`ViewState::from_project` → `host::apply_view`），只是发生在**已存在的窗口**上。
    /// 因此"换工程 ⇒ 换界面"这件事在同一个活窗口上可判据（不需要第二个窗口，
    /// 也就不需要第二个 Tier-1 平台 —— 那是 `set_platform` 每线程一次的限制）。
    fn apply_project(&mut self, project: &YebanProjectV1) -> Result<(), LiveWiringError> {
        let view = ViewState::from_project(project)?;
        let registry = control_tree_from_registry(&ElementRegistry::from_view(&view))?;
        host::apply_view(&self.window, &view);
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

    /// `ui/switch_main_view`：**真的**改 `MainWindow.arrangement-view` 并回读。
    ///
    /// 名字与 trait 方法（`UiTestPort::switch_main_view_impl`）刻意不同：trait 里那一份
    /// 只是三行委托，语义主体在这里 —— 同名会让"哪一份在跑"变成读者要猜的事。
    fn apply_main_view(&mut self, view: &str) -> Result<(), PortError> {
        let arrangement = match view {
            "arrangement" => true,
            "session" => false,
            other => {
                // 参数白名单（`methods::VIEW`）已经拦下非法值（`-32602`）；
                // 这里是**纵深防御**：执行面自己也不接受没定义过的视图名。
                return Err(PortError::Rejected {
                    message: format!("未知主视图 `{other}`（合法值: arrangement, session）"),
                });
            }
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

    /// `ui/force_save`：**真的**写盘（临时文件 → `sync_all` → 原子重命名）。
    fn save_now(&mut self) -> Result<(), PortError> {
        let Some(path) = self.save_path.clone() else {
            // 没有配置落点 = 这条能力在**这个装配上**没接线 ⇒ `-32005` 是诚实的。
            return Err(PortError::Rejected {
                message: "未配置保存路径（`LiveWiringOptions::save_path`）；\
                          `ui/force_save` 不会假装写过一个不存在的文件"
                    .to_owned(),
            });
        };
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
                // 容器布局：`project.json` + `history.dag`（空图谱），见 `crate::save` 的边界。
                ("containerEntries", ReportValue::Uint(2)),
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
        let snapshot = self.pump_meters();
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
                // 新引擎第一次抽帧的量子号（0 = 什么都没抽到）。
                (
                    "visibleQuantum",
                    ReportValue::Uint(snapshot.quantum.unwrap_or(0)),
                ),
                (
                    "visibleNodes",
                    ReportValue::Uint(snapshot.nodes_seen as u64),
                ),
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
}

impl UiSurface for LiveAdminSurface {
    fn surface_name(&self) -> &'static str {
        self.inner.surface_name()
    }

    fn capture_image(&self) -> Result<Rgb8Image, PortError> {
        self.inner.capture_image()
    }

    fn take_admin_report(&mut self) -> Option<AdminReport> {
        self.report.take()
    }
}

/// 装配好的**真实界面 + 真实执行面**。
pub struct LiveUi {
    /// 由 `YebanProjectV1` 投影出来的视图状态（判据据此知道"工程里叫什么"）。
    pub view: ViewState,
    /// 静态语义注册表（`[UI-TEST-001]` 的**声明全集**，无窗口也能构造）。
    pub registry: ControlTree,
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
            view: self.view,
            registry: self.registry,
            scene: self.scene,
            reference: self.reference,
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

    /// 采纳一条新的电平队列（测试注入 / 引擎换代）。
    ///
    /// 生产路径上这条队列由 `EngineHost::reload` 产出；判据用它注入**已知**的
    /// `MeterFrame`，从而把"控件树里的 dBFS == 我喂进去的那一帧"变成可断言的事实。
    pub fn adopt_meter_collector(&mut self, collector: MeterCollector) {
        self.surface.meters.adopt(collector);
    }

    /// 一轮电平消费（60Hz 定时器腿）：抽干 → 面板 → 注入 `.slint` → 重抓树。
    pub fn pump_meters(&mut self) -> MeterSnapshot {
        self.surface.pump_meters()
    }

    /// 换一个工程（同一个活窗口上的重新投影 + 重新注入）。
    ///
    /// # Errors
    ///
    /// 投影失败 / 注册表适配失败 / 引擎换代失败。
    pub fn apply_project(&mut self, project: &YebanProjectV1) -> Result<(), LiveWiringError> {
        self.surface.apply_project(project)
    }
}

/// 控制面 + 装配时留下的证据（判据用）。
pub struct LiveControlPlane {
    plane: ControlPlane,
    view: ViewState,
    registry: ControlTree,
    scene: DemoScene,
    reference: Rgb8Image,
}

impl LiveControlPlane {
    /// 借出控制面（发 `ui/*` 调用）。
    pub fn plane(&mut self) -> &mut ControlPlane {
        &mut self.plane
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
    let admin = LiveAdminSurface {
        inner: surface,
        window,
        registry,
        project: project.clone(),
        view: view.clone(),
        save_path: options.save_path.clone(),
        meters: MeterRuntime::empty(),
        engine: EngineHost::new(),
        engine_quanta: options.engine_quanta,
        save_epoch: 0,
        report: None,
    };
    Ok(LiveUi {
        view,
        registry: admin.registry.clone(),
        reference,
        surface: admin,
        scene,
    })
}
