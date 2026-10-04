//! **app 侧的接线点**：活的 `MainWindow` → `LivePort`（Tier-1 光栅化 + 运行时控件树）
//! → `PortAdapter`（零 Slint 的 `UiSurface`）→ `ControlPlane`（`ui/*` 方法）。
//!
//! 规范来源 (Normative)：
//! - `[ARCH-UI-004]` / `[UI-TEST-001]` §12.2：语义 ID 寻址是**唯一**的自动化寻址方式；
//! - `[UI-MCP-001]` §12.3：三级权限（本文件只用 `Permission` → `Scope` 的公开映射）；
//! - `[UI-MCP-002]` §12.5：动态区遮罩（由 `LivePort` 的运行时几何 + 注册表的动态标记合成）；
//! - `[MUST-GATE-015]`：Golden 必须由 **Tier-1 软件光栅化**产出，尺寸非零且非全黑；
//! - ADR-0001 **D28**（投影层零 Slint + **唯一**注入点）、**D22**（debug info 构建期打开，
//!   `build.rs` 已开）、**D18**（`SLINT_BACKEND=headless` 不存在 ⇒ 平台是**装进去**的）。
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
//!   LiveControlPlane ──owns──> ControlPlane ──owns(Box)──> PortAdapter
//!        │                                                     │ owns
//!        │                                                     ▼
//!        │                                       LivePort<MainWindow>   ← 唯一**非 `Send`** 的东西
//!        │                                            │ owns
//!        │                                            ├── MainWindow          （host::build_main_window）
//!        │                                            └── Tier1Window         （MinimalSoftwareWindow）
//!        └──owns──> 对照帧 Rgb8Image（装配时直接抓的一帧）
//! ```
//!
//! **`Send` 边界说明（重要）**：`MainWindow` 与 `MinimalSoftwareWindow` 都是 `Rc` 语义、
//! 平台上下文是**线程局部**的，因此这一整条链**不是 `Send`**。这不是缺陷，是刻意的：
//! `yeban_ui_mcp` 的环回 HTTP 是**单线程串行** `accept`（见那边的 `transport/http.rs` 文件头），
//! 它不要求 `Send`；反过来，为了"能跨线程"去复制一份窗口状态，就会造出**两份事实源** ——
//! 那是本仓库最不愿意付的代价（`[MODEL-AST-002]` 的投影方向只有一个）。
//!
//! ## 为什么这个文件不在 `lib.rs` 的模块表里（以及它怎么被编译）
//!
//! 它需要 `yeban-ui-mcp` 与 `yeban-ui-test-port`，而这两个依赖在
//! `crates/yeban-app/Cargo.toml` 里都是 **dev-dependency**（`AGENTS.md` §2 红线 6：
//! 默认 release 构建里不得开启 `ui-mcp` / 内省能力；`cargo tree -p yeban-app -e normal`
//! 里因此看不到它们）。所以本文件只被测试目标用 `#[path]` 装进去，
//! 与 `src/test_port_adapter.rs` 的既有做法完全同款：
//!
//! ```ignore
//! #[path = "../src/live_surface.rs"]
//! mod live;
//! ```
//!
//! 它**不是**"只在测试里存在的假实现"：`LivePort` / `PortAdapter` / `ControlPlane`
//! 三个类型都是各自 crate 里的公开产品路径，本文件只负责把它们接起来，而且
//! 构造 `MainWindow` 走的仍然是 `host::build_main_window`（**唯一**注入点，D28）。
//! 要把这条路径装进发行版的 `--ui-control-plane` 开关，需要集成者加一个
//! **可选** feature 与 CI 步骤（见 `docs/ledger/live-port-notes.md` 的 needs）。

#![allow(missing_docs, rust_2018_idioms)]

/// `ElementRegistry` → `ControlTree` 的**唯一**实现（同一份文件被本机探针复用）。
#[path = "registry_tree.rs"]
mod registry_tree;

use registry_tree::control_tree_from_registry;

use yeban_app::bridge::{BridgeError, ViewState};
use yeban_app::elements::ElementRegistry;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_model::YebanProjectV1;
use yeban_ui_mcp::live::ControlPlane;
use yeban_ui_mcp::surface::PortAdapter;
use yeban_ui_test_port::port::{Permission, PortError};
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

/// 接线过程可能出的错（投影层 / 注册表适配 / Tier-1 渲染层）。
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
}

impl core::fmt::Display for LiveWiringError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Project(error) => write!(f, "工程投影失败: {error}"),
            Self::Tree(error) => write!(f, "语义注册表适配失败: {error}"),
            Self::Render(error) => write!(f, "Tier-1 执行面装配失败: {error}"),
            Self::Capture(error) => write!(f, "Tier-1 抓帧失败: {error}"),
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
    /// 真实执行面（活窗口 + Tier-1 抓帧），已经过 `PortAdapter` 升格为 `UiSurface`。
    pub surface: LiveSurface,
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
            Permission::Interactive | Permission::Administrative => {
                ControlPlane::interactive_for_tests(Box::new(self.surface))
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

    /// 外壳场景（视口尺寸在这里）。
    #[must_use]
    pub fn scene(&self) -> &DemoScene {
        &self.scene
    }

    /// 装配时直接抓的那一帧。
    #[must_use]
    pub fn reference(&self) -> &Rgb8Image {
        &self.reference
    }

    /// 视口尺寸（`[MUST-GATE-015]` 的"尺寸正确"要对的数）。
    #[must_use]
    pub fn viewport(&self) -> Size {
        Size::new(self.scene.viewport_width, self.scene.viewport_height)
    }
}

/// 以 `project` 为唯一数据源，在**真实界面**上装配一条控制面。
///
/// 构造顺序（不可颠倒，顺序本身就是约束）：
/// 1. 投影：`YebanProjectV1` → `ViewState`（唯一的事实源方向）；
/// 2. 注册表：`ElementRegistry::from_view` → `ControlTree`（语义寻址的全集）；
/// 3. 外壳场景：`DemoScene::from_view`（视口尺寸 + 会话运行态占位）；
/// 4. `LivePort::new`：**装 Tier-1 平台 → 构造 `MainWindow`（`host::build_main_window`）
///    → `show()` → 抓运行时控件树**（含真实几何）；
/// 5. 直接抓一帧留作对照；
/// 6. `PortAdapter::new`：把"怎么拿像素"注入进去，得到零 Slint 的 `UiSurface`。
///
/// # Errors
///
/// 投影失败（[`LiveWiringError::Project`]）或平台/组件/抓帧失败
/// （[`LiveWiringError::Render`]）。
pub fn build_live_ui(
    project: &YebanProjectV1,
    permission: Permission,
) -> Result<LiveUi, LiveWiringError> {
    let view = ViewState::from_project(project)?;
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let registry = control_tree_from_registry(&ElementRegistry::from_view(&view))?;
    // 投影出来的**窗口**：`host::build_main_window` 是唯一的注入实现（D28）。
    let port = LivePort::new(size, permission, Some(&registry), || {
        host::build_main_window(&view, &scene)
    })?;
    let reference = capture_tier1(&port)?;
    // 显式写出函数指针类型：`PortAdapter` 是泛型的，靠"赋值给 `LiveSurface`"反推
    // `F = fn(..)` 属于**强制转换点**，写出来比让读者猜推断结果更清楚。
    let capture: fn(&LivePort<MainWindow>) -> Result<Rgb8Image, PortError> = capture_tier1;
    let surface: LiveSurface = PortAdapter::new(port, "tier1-live-port", capture);
    Ok(LiveUi {
        view,
        registry,
        reference,
        surface,
        scene,
    })
}
