//! Tier-1 软件光栅化截图 —— `[ARCH-SLINT-001]` / `[ROAD-M0-008]` / `[ROAD-M3-007]` / `[MUST-GATE-015]`。
//!
//! ## 为什么必须走这条路（而不是 `i-slint-backend-testing`）
//!
//! `[MUST-GATE-015]` 原文：*CI 中增加断言，Golden 图必须由 Tier 1 软件光栅化方案
//! （`SoftwareRenderer` + Framebuffer 捕获）产出，禁止使用 `i-slint-backend-testing`
//! （该后端不渲染像素）；Golden 图尺寸非零且非全黑。*
//!
//! 本模块因此做三件事：
//! 1. 实现 `slint::platform::Platform`，把窗口适配器接到 [`MinimalSoftwareWindow`]；
//! 2. 把窗口渲染进内存 `SharedPixelBuffer<Rgb8Pixel>`（**不依赖任何物理显示器**，
//!    CI 上没有 X11/Wayland 也能跑）；
//! 3. 把像素变成 [`crate::image::Rgb8Image`]（零 Slint 依赖的底座），
//!    并断言 `[MUST-GATE-015]` 的两条门槛（尺寸非零、非全黑）。
//!
//! ## 逐条核验过的上游 API（写代码前对过源码，不是凭记忆）
//!
//! | 用到的 API | 确切形态 | 出处 |
//! | :--- | :--- | :--- |
//! | `Platform` trait | **只有一个必需方法** `fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError>` | <https://docs.rs/slint/1.18.1/slint/platform/trait.Platform.html> |
//! | `set_platform` | `pub fn set_platform(platform: Box<dyn Platform + 'static>) -> Result<(), SetPlatformError>`；**已设置则返回 `Err`** | <https://docs.rs/slint/1.18.1/slint/platform/fn.set_platform.html> |
//! | `MinimalSoftwareWindow::new` | `pub fn new(repaint_buffer_type: RepaintBufferType) -> Rc<Self>` | <https://docs.rs/slint/1.18.1/slint/platform/software_renderer/struct.MinimalSoftwareWindow.html> |
//! | `MinimalSoftwareWindow::draw_if_needed` | `pub fn draw_if_needed(&self, render_callback: impl FnOnce(&SoftwareRenderer)) -> bool`（**只有需要重绘时才调用回调**） | 同上 |
//! | `SoftwareRenderer::render` | `pub fn render(&self, buffer: &mut [impl TargetPixel], pixel_stride: usize) -> PhysicalRegion` | <https://docs.rs/slint/1.18.1/slint/platform/software_renderer/struct.SoftwareRenderer.html> |
//! | `Rgb8Pixel` | `pub type Rgb8Pixel = Rgb<u8>`，**实现了 `TargetPixel`**，字段 `r/g/b` | <https://docs.rs/slint/1.18.1/slint/type.Rgb8Pixel.html> |
//! | `SharedPixelBuffer::as_bytes` | `pub fn as_bytes(&self) -> &[u8]`（`Pixel: Pod + ComponentBytes<u8>`） | 上游源码 `i-slint-core-1.18.1/graphics/image.rs:100` |
//! | `RepaintBufferType::NewBuffer` | 语义 = "The full window is always redrawn"（Golden 要的就是全量重绘） | <https://docs.rs/slint/1.18.1/slint/platform/software_renderer/enum.RepaintBufferType.html> |
//! | `WindowEvent` | `PointerPressed{position,button}` / `PointerMoved{position}` / `PointerReleased{position,button}` / `KeyPressed{text}` / `KeyReleased{text}` | 上游源码 `i-slint-core-1.18.1/platform.rs:367` |
//! | `Window::dispatch_event` | `pub fn dispatch_event(&self, event: WindowEvent)`（出错时 panic；不要用已废弃的 `try_dispatch_event`） | 上游源码 `i-slint-core-1.18.1/api.rs:633` |
//! | `Key::*` | `Tab` / `Return` / `Escape` / `Shift` / `Control` / `Space` / `Backspace`，且 `impl From<Key> for SharedString` | 上游源码 `i-slint-common-1.18.1/key_codes.rs:32-54` |
//!
//! **规范与上游不符之处**（ADR-0001 D18）：`SLINT_BACKEND=headless` 在 1.18.1 **不存在**
//! （只认 `qt`/`winit`/`linuxkms`）。本模块不依赖任何环境变量：它用
//! `slint::platform::set_platform` 直接装自研平台，因此"无头"是代码路径而不是环境变量。
//!
//! ## 线程约束（重要）
//!
//! 上游把平台存在**线程局部**里：`MinimalSoftwareWindow` 的回归测试注释写着
//! *"Each test runs on its own thread, so the thread-local global context is unset here."*
//! 因此 [`Tier1Window::install`] 每个**线程**只能成功一次；
//! 一个 `#[test]` 里装一次、用完即弃是最稳的用法。`cargo test` 默认每个测试一个线程，
//! 所以"每个测试各装一次"是安全的（不要在同一测试里装两次）。

use std::path::PathBuf;
use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, Rgb8Pixel, SharedPixelBuffer};

use crate::image::{Rgb8Image, Size};
use crate::port::{KeyCode, Permission, PointerButton, PortError, UiTestPort};
use crate::tree::{ControlTree, TreeError};
use crate::{inspect, mask, png};

/// Tier-1 渲染 / 端口错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// 请求了零尺寸表面。
    EmptySurface {
        /// 出错的尺寸。
        size: Size,
    },
    /// `set_platform` 失败（当前线程已经装过后端）。
    PlatformUnavailable {
        /// 上游错误消息。
        message: String,
    },
    /// 组件构造 / 显示失败。
    Component {
        /// 上游错误消息。
        message: String,
    },
    /// `draw_if_needed` 说"不需要重绘"，于是没有像素可断言。
    NotRendered {
        /// 排障提示。
        hint: &'static str,
    },
    /// `[MUST-GATE-015]`：Golden 图**不得全黑**。
    AllBlack {
        /// 出错的尺寸（尺寸合法但内容全黑 ⇒ 渲染管线或平台装配有问题）。
        size: Size,
    },
    /// 像素缓冲与图像模型不兼容。
    Surface {
        /// 底层错误消息。
        message: String,
    },
    /// 控件树构建失败。
    Tree(TreeError),
    /// PNG 编码 / 落盘失败。
    Artifact {
        /// 底层错误消息。
        message: String,
    },
}

impl core::fmt::Display for RenderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptySurface { size } => {
                write!(f, "表面尺寸不得为零: {}x{}", size.width, size.height)
            }
            Self::PlatformUnavailable { message } => {
                write!(f, "无法安装 Tier-1 平台 (每个线程只能装一次): {message}")
            }
            Self::Component { message } => write!(f, "Slint 组件构造/显示失败: {message}"),
            Self::NotRendered { hint } => write!(f, "本次没有发生重绘, 无像素可断言; {hint}"),
            Self::AllBlack { size } => write!(
                f,
                "[MUST-GATE-015] 违反: {}x{} 的截图全黑 —— 软件光栅化没有产出有效像素",
                size.width, size.height
            ),
            Self::Surface { message } => write!(f, "像素缓冲不兼容: {message}"),
            Self::Tree(err) => write!(f, "控件树构建失败: {err}"),
            Self::Artifact { message } => write!(f, "PNG 产出失败: {message}"),
        }
    }
}

impl core::error::Error for RenderError {}

impl From<TreeError> for RenderError {
    fn from(value: TreeError) -> Self {
        Self::Tree(value)
    }
}

/// 自研 `Platform`：把唯一的窗口适配器交给 [`MinimalSoftwareWindow`]。
///
/// `create_window_adapter` 是 `Platform` 在 1.18.1 的**唯一**必需方法
/// （其余 9 个都有默认实现，见模块文档的核验表）。返回同一个 `Rc` 是上游
/// `mcu-board-support` 一类的标准做法：一个进程/线程只有一个软件窗口。
struct Tier1Platform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Tier1Platform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

/// 已经装好的 Tier-1 软件窗口。
#[derive(Clone)]
pub struct Tier1Window {
    window: Rc<MinimalSoftwareWindow>,
}

impl Tier1Window {
    /// 安装 Tier-1 平台并创建一个 `size` 大小的软件窗口。
    ///
    /// **必须在构造任何 Slint 组件之前调用**（上游要求：`set_platform` 要在创建组件前完成）。
    /// 每个线程只能成功一次。
    pub fn install(size: Size) -> Result<Self, RenderError> {
        if size.is_empty() {
            return Err(RenderError::EmptySurface { size });
        }
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        window.set_size(PhysicalSize::new(size.width, size.height));
        let platform = Tier1Platform {
            window: window.clone(),
        };
        slint::platform::set_platform(Box::new(platform)).map_err(|err| {
            RenderError::PlatformUnavailable {
                message: err.to_string(),
            }
        })?;
        Ok(Self { window })
    }

    /// 当前窗口尺寸（物理像素）。
    #[must_use]
    pub fn size(&self) -> Size {
        let size = self.window.size();
        Size::new(size.width, size.height)
    }

    /// 调整窗口尺寸。组件创建之后再调一次可以确保根元素的布局拿到最终尺寸。
    pub fn resize(&self, size: Size) -> Result<(), RenderError> {
        if size.is_empty() {
            return Err(RenderError::EmptySurface { size });
        }
        self.window
            .set_size(PhysicalSize::new(size.width, size.height));
        Ok(())
    }

    /// 强制本次 `draw_if_needed` 会重绘（`NewBuffer` 语义下等价于"全量重绘"）。
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// 渲染当前窗口到内存缓冲，返回零 Slint 依赖的图像。
    ///
    /// 注意：`draw_if_needed` 只在"需要重绘"时调用回调，所以这里先显式
    /// `request_redraw()`；否则第二次截图会拿到"没有渲染"的空结果，
    /// 表现为全黑 —— 那正是 `[MUST-GATE-015]` 要抓的假绿。
    pub fn capture(&self) -> Result<Rgb8Image, RenderError> {
        let requested = self.size();
        if requested.is_empty() {
            return Err(RenderError::EmptySurface { size: requested });
        }
        let mut buffer = SharedPixelBuffer::<Rgb8Pixel>::new(requested.width, requested.height);
        let stride = buffer.width() as usize;
        let mut drew = false;
        self.request_redraw();
        let redrawn = self.window.draw_if_needed(|renderer| {
            renderer.render(buffer.make_mut_slice(), stride);
            drew = true;
        });
        if !redrawn || !drew {
            return Err(RenderError::NotRendered {
                hint: "先 ui.show() 再 capture(); 并确认窗口尺寸非零",
            });
        }
        let size = Size::new(buffer.width(), buffer.height());
        Rgb8Image::from_raw(size, buffer.as_bytes().to_vec()).map_err(|err| RenderError::Surface {
            message: err.to_string(),
        })
    }

    /// 直接向窗口分发一个 Slint 窗口事件（`[UI-TEST-002]` 的底层动作）。
    pub fn dispatch(&self, event: WindowEvent) {
        self.window.dispatch_event(event);
    }

    /// 指针按下（窗口逻辑坐标）。
    pub fn pointer_down(&self, position: LogicalPosition, button: PointerEventButton) {
        self.dispatch(WindowEvent::PointerPressed { position, button });
    }

    /// 指针移动（窗口逻辑坐标）。
    pub fn pointer_move(&self, position: LogicalPosition) {
        self.dispatch(WindowEvent::PointerMoved { position });
    }

    /// 指针释放（窗口逻辑坐标）。
    pub fn pointer_up(&self, position: LogicalPosition, button: PointerEventButton) {
        self.dispatch(WindowEvent::PointerReleased { position, button });
    }

    /// 键盘按下。
    pub fn key_press(&self, key: Key) {
        self.dispatch(WindowEvent::KeyPressed { text: key.into() });
    }

    /// 键盘释放。
    pub fn key_release(&self, key: Key) {
        self.dispatch(WindowEvent::KeyReleased { text: key.into() });
    }

    /// 输入一个字符（`KeyCode::Character`）。
    pub fn type_char(&self, ch: char) {
        self.dispatch(WindowEvent::KeyPressed {
            text: ch.to_string().into(),
        });
    }
}

/// `[MUST-GATE-015]` 的证据：由**像素**推出的一组可记录数字。
///
/// 它存在的意义是让"Golden 是有效的"这件事变成可打印、可入库的数字，
/// 而不是一句"测试通过了"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoldenEvidence {
    /// 图像尺寸（必须非零）。
    pub size: Size,
    /// 非黑像素数（必须 > 0）。
    pub non_black_pixels: u64,
    /// 不同颜色数（> 1 说明不是一个纯色块）。
    pub distinct_colors: usize,
    /// PNG 编码后的字节数。
    pub png_bytes: usize,
    /// PNG 字节的 FNV-1a 64 指纹（决定性编码 ⇒ 可当稳定标识用；**不是**密码学摘要）。
    pub fingerprint: u64,
}

impl GoldenEvidence {
    /// 一行人类可读摘要（测试里 `eprintln!` 出来就是证据）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Tier-1 Golden: {}x{} ({} px), 非黑 {} ({}%), 颜色 {} 种, PNG {} 字节, 指纹 {:016x}",
            self.size.width,
            self.size.height,
            self.size.pixel_count(),
            self.non_black_pixels,
            (self.non_black_pixels as f64 * 100.0 / self.size.pixel_count() as f64 * 10.0).round()
                / 10.0,
            self.distinct_colors,
            self.png_bytes,
            self.fingerprint
        )
    }
}

/// 断言 `[MUST-GATE-015]` 的两条门槛并返回证据：**尺寸非零** + **非全黑**。
pub fn golden_evidence(image: &Rgb8Image) -> Result<GoldenEvidence, RenderError> {
    let size = image.size();
    if size.is_empty() {
        return Err(RenderError::EmptySurface { size });
    }
    if image.is_all_black() {
        return Err(RenderError::AllBlack { size });
    }
    let bytes = png::encode_rgb8_limited(image, png::REPO_MAX_FILE_BYTES).map_err(|err| {
        RenderError::Artifact {
            message: err.to_string(),
        }
    })?;
    Ok(GoldenEvidence {
        size,
        non_black_pixels: image.non_black_pixels(),
        distinct_colors: image.distinct_color_count(),
        png_bytes: bytes.len(),
        fingerprint: fnv1a64(&bytes),
    })
}

/// 把一行诊断写进**进程级 stderr**（绕过 `cargo test` 对 `eprintln!` 的捕获）。
///
/// 为什么需要它：libtest 默认捕获 `print!`/`eprintln!` 的**宏**输出，**通过的测试什么都不打印**。
/// 于是一份"尺寸非零 / 非全黑"的实测数字就只存在于无人能看到的缓冲区里 —— 那等于没有证据
/// （`[MUST-GATE-015]` 是全仓库最需要"数字而不是断言"的一条门禁）。
/// 这里除了 `eprintln!` 之外，再往 `/dev/stderr` 写一份：Linux 上它指向进程 fd 2，
/// **不经过** std 的输出捕获，因此会直接进入 CI 日志。
/// 其它平台（如 Windows 没有 `/dev/stderr`）退化成只走 `eprintln!`，不报错、不影响判据。
///
/// 它是 [`report_evidence`] / [`report_capability`] 的**唯一出口**：这两条只是加上各自的
/// 前缀/格式，因此"数字在通过的测试里也看得见"这件事只有一处实现。
pub fn report_line(line: &str) {
    eprintln!("{line}");
    if let Ok(mut fd) = std::fs::File::create("/dev/stderr") {
        use std::io::Write as _;
        let _ = fd.write_all(line.as_bytes());
        let _ = fd.write_all(b"\n");
    }
}

/// 把一行证据写进进程级 stderr（理由见 [`report_line`]）。
pub fn report_evidence(evidence: &GoldenEvidence) {
    report_line(&evidence.summary());
}

/// 把一条"能力不可用"的诊断写进进程级 stderr（同 [`report_evidence`] 的理由）。
///
/// 机器可读前缀 `RUNTIME-TREE-CAPABILITY:` 让 CI 日志可以被 grep 出来，
/// 而不是靠人读散文。
pub fn report_capability(detail: &str) {
    report_line(&format!("RUNTIME-TREE-CAPABILITY: {detail}"));
}

/// FNV-1a 64 位指纹（零依赖、逐位确定）。用于给 Golden 一个稳定的短标识，**不是**安全摘要。
#[must_use]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 截图落盘的目录：`<repo>/target/ui-test-port/`。
///
/// 刻意放在 `target/` 下（`.gitignore` 已忽略）：PNG 是**过程产物**，不是仓库资产 ——
/// stored-deflate 的 1080p PNG 约 6.2 MB，提交进仓库会撞上 `AGENTS.md` §2 红线 9
/// 的 10 MB 单文件上限（守卫 G06）。
#[must_use]
pub fn artifact_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-test-port")
}

/// 把截图写成 PNG 落到 [`artifact_dir`]，返回落盘路径。
pub fn write_artifact(name: &str, image: &Rgb8Image) -> Result<PathBuf, RenderError> {
    let bytes = png::encode_rgb8_limited(image, png::REPO_MAX_FILE_BYTES).map_err(|err| {
        RenderError::Artifact {
            message: err.to_string(),
        }
    })?;
    let dir = artifact_dir();
    std::fs::create_dir_all(&dir).map_err(|err| RenderError::Artifact {
        message: format!("{}: {err}", dir.display()),
    })?;
    let path = dir.join(format!("{name}.png"));
    std::fs::write(&path, bytes).map_err(|err| RenderError::Artifact {
        message: format!("{}: {err}", path.display()),
    })?;
    Ok(path)
}

/// 端口 + 活窗口：同一个实例同时给出**控件树**（含真实几何）与**像素**。
///
/// 构造顺序被类型钉死（先装平台 → 再建组件 → 再 show → 再抓树），
/// 调用方不可能把顺序写错。
///
/// ## ⚠️ 传进来的组件必须是"带 debug info 编译"的
///
/// 控件树那一半走 `i-slint-backend-testing` 的 `ElementHandle`，它**要求被内省的
/// `.slint` 在编译期打开 debug info**（上游默认关闭）。否则 `tree()` 会是空树而
/// **不会报错** —— 详见 [`crate::inspect`] 的模块文档与
/// `docs/ledger/ui-test-port-notes.md` §2 第 27 条。
/// 像素那一半（[`Tier1Window`]）不受影响。
pub struct LivePort<T: ComponentHandle> {
    ui: T,
    window: Tier1Window,
    permission: Permission,
    tree: ControlTree,
    /// 最近一次指针位置。`[UI-TEST-002]` 的 `dispatch_pointer_up(button)` **没有**坐标参数，
    /// 所以释放必须复用上一次按下的位置，否则"按下 A、释放在 (0,0)"这类语义会破坏拖拽用例。
    pointer: LogicalPosition,
}

impl<T: ComponentHandle> LivePort<T> {
    /// 装平台 → 建组件 → `show()` → 按需注入静态注册表的动态标记 → 抓运行时控件树。
    ///
    /// `registry` 是可选的静态注册表（例如 `yeban-app` 的 `ElementRegistry` 适配出来的树）：
    /// 传入时，[`ControlTree::merge_dynamic_flags_from`] 会把"哪些节点是高频刷新区"
    /// 注入到运行时树上 —— 运行时读不到这种业务知识，而 `[UI-MCP-002]` 的遮罩需要它。
    pub fn new(
        size: Size,
        permission: Permission,
        registry: Option<&ControlTree>,
        build: impl FnOnce() -> Result<T, slint::PlatformError>,
    ) -> Result<Self, RenderError> {
        let window = Tier1Window::install(size)?;
        let ui = build().map_err(|err| RenderError::Component {
            message: err.to_string(),
        })?;
        ui.show().map_err(|err| RenderError::Component {
            message: err.to_string(),
        })?;
        window.resize(size)?;
        let tree = inspect::tree_from_element_root(&ui, registry)?;
        Ok(Self {
            ui,
            window,
            permission,
            tree,
            pointer: LogicalPosition::new(0.0, 0.0),
        })
    }

    /// 窗口（截图 / 直接分发事件用）。
    #[must_use]
    pub fn window(&self) -> &Tier1Window {
        &self.window
    }

    /// 组件实例（断言生成组件上的属性用）。
    #[must_use]
    pub fn ui(&self) -> &T {
        &self.ui
    }

    /// 重新抓一次控件树（UI 状态变了之后调用；几何会更新）。
    pub fn refresh_tree(
        &mut self,
        registry: Option<&ControlTree>,
    ) -> Result<&ControlTree, RenderError> {
        self.tree = inspect::tree_from_element_root(&self.ui, registry)?;
        Ok(&self.tree)
    }
}

impl<T: ComponentHandle> UiTestPort for LivePort<T> {
    fn permission(&self) -> Permission {
        self.permission
    }

    fn tree(&self) -> &ControlTree {
        &self.tree
    }

    fn capture_png(&self) -> Result<Vec<u8>, PortError> {
        let image = self.window.capture().map_err(|err| PortError::Capture {
            message: err.to_string(),
        })?;
        golden_evidence(&image).map_err(|err| PortError::Capture {
            message: err.to_string(),
        })?;
        png::encode_rgb8_limited(&image, png::REPO_MAX_FILE_BYTES).map_err(|err| {
            PortError::Capture {
                message: err.to_string(),
            }
        })
    }

    fn read_property(&self, element_id: &str, name: &str) -> Result<String, PortError> {
        if !self.tree.contains(element_id) {
            return Err(PortError::UnknownElement {
                id: element_id.to_owned(),
            });
        }
        let handle = inspect::find_by_accessible_id(&self.ui, element_id).ok_or_else(|| {
            PortError::UnknownElement {
                id: element_id.to_owned(),
            }
        })?;
        inspect::property_of(&handle, name).ok_or_else(|| {
            // `[ARCH-UI-004]` 的两个**可选**状态属性（`value` / `checked`）在一个元素上
            // 可能**根本没有声明** —— 那是元素的事实，不是调用方写错了名字。
            // 两条都给 `Rejected`（读路径上映射为 `-32602`，D25：不发明新错误码），
            // 但**话术必须不同**：把"没声明"说成"不支持这个属性名"会让调用方
            // 去查文档找拼写，而真正的原因是那个控件没有值可读。
            let message = if inspect::is_optional_property(name) {
                format!(
                    "元素 `{element_id}` 没有声明 `accessible-{name}`（该属性是可选的: \
                     缺席时 `ui/node` / `ui/tree` 的 `{name}` 字段是 null）"
                )
            } else {
                format!("不支持的属性名 `{name}` (见 inspect::property_of 的清单)")
            };
            PortError::Rejected { message }
        })
    }

    fn dispatch_pointer_down_impl(
        &mut self,
        element_id: &str,
        x_offset: f64,
        y_offset: f64,
        button: PointerButton,
    ) -> Result<(), PortError> {
        let node = self
            .tree
            .find_by_id(element_id)
            .ok_or_else(|| PortError::UnknownElement {
                id: element_id.to_owned(),
            })?;
        let bounds = node.bounds.ok_or_else(|| PortError::MissingGeometry {
            id: element_id.to_owned(),
        })?;
        let position = LogicalPosition::new(
            bounds.x as f32 + x_offset as f32,
            bounds.y as f32 + y_offset as f32,
        );
        self.pointer = position;
        self.window.pointer_down(position, slint_button(button));
        Ok(())
    }

    fn dispatch_pointer_move_impl(&mut self, x: f64, y: f64) -> Result<(), PortError> {
        let position = LogicalPosition::new(x as f32, y as f32);
        self.pointer = position;
        self.window.pointer_move(position);
        Ok(())
    }

    fn dispatch_pointer_up_impl(&mut self, button: PointerButton) -> Result<(), PortError> {
        // 释放复用最后一次按下/移动的位置（§12.4 的 `dispatch_pointer_up(button)` 没有坐标参数）。
        let position = self.pointer;
        self.window.pointer_up(position, slint_button(button));
        Ok(())
    }

    fn dispatch_key_press_impl(&mut self, key: KeyCode) -> Result<(), PortError> {
        match key {
            KeyCode::Tab => self.window.key_press(Key::Tab),
            KeyCode::Escape => self.window.key_press(Key::Escape),
            KeyCode::Return => self.window.key_press(Key::Return),
            KeyCode::Space => self.window.key_press(Key::Space),
            KeyCode::Backspace => self.window.key_press(Key::Backspace),
            KeyCode::Shift => self.window.key_press(Key::Shift),
            KeyCode::Control => self.window.key_press(Key::Control),
            KeyCode::ShiftEnter => {
                // 修饰键必须**用完就放**：只按下不释放会让后续按键一直被当成 Shift 组合,
                // 那是一条隐蔽的状态泄漏。
                self.window.key_press(Key::Shift);
                self.window.key_press(Key::Return);
                self.window.key_release(Key::Shift);
            }
            KeyCode::Character(ch) => self.window.type_char(ch),
        }
        Ok(())
    }

    fn switch_main_view_impl(&mut self, _view: &str) -> Result<(), PortError> {
        Err(PortError::Rejected {
            message: "主视图切换需要 yeban-app 的模型绑定 (本线只定义接口与权限判定, \
                      不依赖 yeban-model, 见 docs/ledger/ui-test-port-notes.md 的 needs)"
                .to_owned(),
        })
    }

    fn force_save_impl(&mut self) -> Result<(), PortError> {
        Err(PortError::Rejected {
            message: "强制保存需要工程存储层 (.yeban 容器), 不属于 yeban-ui-test-port 的依赖方向"
                .to_owned(),
        })
    }

    fn reload_engine_impl(&mut self) -> Result<(), PortError> {
        Err(PortError::Rejected {
            message: "引擎重载需要 yeban-engine 句柄; 本线只定义接口与权限判定".to_owned(),
        })
    }
}

fn slint_button(button: PointerButton) -> PointerEventButton {
    match button {
        PointerButton::Left => PointerEventButton::Left,
        PointerButton::Middle => PointerEventButton::Middle,
        PointerButton::Right => PointerEventButton::Right,
        PointerButton::Other => PointerEventButton::Other,
    }
}

/// 遮罩 + SSIM 的便捷入口：把运行时树里的动态区遮掉再比对。
///
/// 这是 `[UI-MCP-002]` + `[UI-MCP-003]` 的**联合**路径：遮罩矩形来自控件树
/// （不是测试里手写的坐标），因此界面改版后遮罩区域会自动跟着走。
pub fn compare_with_dynamic_masking(
    left: &Rgb8Image,
    right: &Rgb8Image,
    tree: &ControlTree,
) -> Result<crate::ssim::Verdict, RenderError> {
    let rects = mask::mask_rects_from_tree(tree)?;
    let masked_left = mask::masked(left, &rects);
    let masked_right = mask::masked(right, &rects);
    crate::ssim::compare(&masked_left, &masked_right).map_err(|err| RenderError::Surface {
        message: err.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Rect;
    use crate::ssim;
    use crate::tree::{ControlNode, Role};

    /// Tier-1 夹具 UI：由 `build.rs` 用 `slint_build::compile_with_config` 编译
    /// `ui/fixture.slint`（**打开了 debug info**，见 `build.rs` 的模块文档）。
    ///
    /// 不用 `slint::slint!` 内联宏的两个原因（都是实测出来的，不是偏好）：
    /// 1. `ElementHandle` 的遍历依赖编译期 debug info，而宏路径只能靠
    ///    `SLINT_EMIT_DEBUG_INFO=1` 环境变量（CI 上不受本仓库控制）—— 没有它会得到一棵
    ///    **空的**控件树（CI run 37221680724 实测 `ControlTree { nodes: {} }`）；
    /// 2. 宏的**每一条**编译器警告都会被展开成 `#[deprecated] const WARNING`，
    ///    在 `-D warnings` 下变成硬错误（CI run 37221429630 实测）。
    ///
    /// `#[allow(clippy::all, …)]` 与 `crates/yeban-app/src/lib.rs` 里 `ui` 模块的处理一致：
    /// `include_modules!()` 展开的是第三方生成代码，不保证通过
    /// `[workspace.lints] clippy::all = "deny"`。把这几个 allow 收在一个模块里，
    /// 好过在 crate 根放松全局 lint 策略。
    #[allow(missing_docs, clippy::all, rust_2018_idioms)]
    mod fixture_ui {
        #![allow(missing_docs, clippy::all, rust_2018_idioms)]

        slint::include_modules!();
    }

    use fixture_ui::PortFixture;

    const FIXTURE_SIZE: Size = Size::new(200, 120);

    /// 夹具的静态注册表：与 `PortFixture` 里**带 `accessible-id` 的节点**一一对应（7 条），
    /// 并把两个高频刷新区标成 dynamic（`[UI-MCP-002]`）。
    fn fixture_registry() -> ControlTree {
        let mut registry = ControlTree::new();
        let mut add = |id: &str, role: &str, label: &str| {
            let role = Role::parse(role).expect("夹具角色必须合法");
            registry
                .insert(ControlNode::new(id, role, label))
                .expect("夹具 ID 必须唯一且合法");
        };
        // 注意: **不登记根 Window** —— 夹具的根故意不给 `accessible-id`
        // （与 `yeban-app` 的 `MainWindow` 一致）, 这样"根元素是否被收录"就不会
        // 影响本判据, 也就不会把一个未核验的上游行为变成假红。
        add("transport-bar", "region", "走带栏");
        add("transport-play-button", "button", "播放");
        add("clip-01J8ZQ9K2M-header", "list-item", "剪辑包头");
        add("transport-timecode", "text", "时间码");
        registry
            .insert(
                ControlNode::new(
                    "rta-band-0",
                    Role::parse("progress-indicator").expect("合法角色"),
                    "RTA 频谱块",
                )
                .as_dynamic(),
            )
            .expect("夹具 ID 必须唯一且合法");
        registry
            .insert(
                ControlNode::new(
                    "mixer-vu-track-0",
                    Role::parse("progress-indicator").expect("合法角色"),
                    "轨道 0 VU 电平",
                )
                .as_dynamic(),
            )
            .expect("夹具 ID 必须唯一且合法");
        registry
            .insert(
                ControlNode::new(
                    "transport-playhead",
                    Role::parse("image").expect("合法角色"),
                    "走带光标",
                )
                .as_dynamic(),
            )
            .expect("夹具 ID 必须唯一且合法");
        registry
    }

    /// **本线的核心判据**：`[MUST-GATE-015]` + `[UI-TEST-001]` + `[UI-MCP-001]` + `[UI-MCP-002]`
    /// 在**同一个活窗口实例**上一次性闭环。
    ///
    /// 一个 `#[test]` 里做完所有 Slint 动作，是因为平台是**线程局部**的：
    /// 上游 `MinimalSoftwareWindow` 的回归测试原文是 *"Each test runs on its own thread,
    /// so the thread-local global context is unset here."* —— 每个测试线程只能装一次平台。
    #[test]
    fn tier1_software_renderer_produces_a_non_black_png_and_a_verified_control_tree() {
        let registry = fixture_registry();
        let mut port = LivePort::new(
            FIXTURE_SIZE,
            Permission::ReadOnly,
            Some(&registry),
            PortFixture::new,
        )
        .expect("Tier-1 平台 + 组件 + 运行时控件树");

        // ---------------- [UI-TEST-001] 控件树断言（JSON 可断言） ----------------
        // 取**拥有所有权**的副本（而不是 `port.tree()` 的借用）：后面还要用 `&mut port`
        // 调 `dispatch_*` 验证权限，长借用会让借用检查器直接拒绝编译。
        let tree = port.tree().clone();
        assert_eq!(
            tree.len(),
            registry.len(),
            "运行时控件树必须与注册表条目数一致: {tree:?}"
        );
        let coverage = tree.coverage_against(&registry);
        assert!(coverage.is_complete(), "双向覆盖必须闭合: {coverage}");
        assert_eq!(
            tree.find_by_id("transport-play-button")
                .map(|node| node.role.as_str().to_owned()),
            Some("button".to_owned())
        );

        // `accessible-id` 在运行时的**唯一性**（`line/ui-shell` 留在 notes 里的未实测项）：
        // 重复 ID 会让 `tree_from_element_root` 直接报 DuplicateId 而失败，
        // 因此"能构造出来"本身就是这条判据的证据。
        let ids: Vec<&str> = tree.ids().collect();
        assert_eq!(ids.len(), tree.len());

        // 几何必须来自真实布局（不是 0）。
        let button_bounds = tree
            .find_by_id("transport-play-button")
            .and_then(|node| node.bounds);
        assert!(
            button_bounds.is_some_and(|rect| rect.width > 0 && rect.height > 0),
            "{button_bounds:?}"
        );

        // ---------------- [UI-MCP-002] 动态区遮罩 ----------------
        let mask_rects = mask::mask_rects_from_tree(&tree).expect("动态区必须有包围盒");
        assert_eq!(mask_rects.len(), 3, "夹具登记了三个动态区: {mask_rects:?}");
        assert!(
            mask_rects
                .iter()
                .all(|rect| rect.intersect(FIXTURE_SIZE).is_some())
        );

        // ---------------- [MUST-GATE-015] Tier-1 截图 + 非全黑 ----------------
        let first = port.window().capture().expect("第一次截图");
        let evidence = golden_evidence(&first).expect("[MUST-GATE-015] 尺寸非零且非全黑");
        assert_eq!(evidence.size, FIXTURE_SIZE);
        assert!(evidence.non_black_pixels > 0, "{evidence:?}");
        assert!(
            evidence.distinct_colors >= 3,
            "至少应有背景 + 两个图元颜色: {evidence:?}"
        );
        assert_eq!(evidence.png_bytes, png::encoded_len(&first));
        report_evidence(&evidence);

        // 逐字节确定性：NewBuffer 语义下全量重绘，静态界面必须给出同一份像素。
        let second = port.window().capture().expect("第二次截图");
        assert_eq!(
            first.pixels(),
            second.pixels(),
            "静态界面两次截图必须逐字节相同"
        );

        // ---------------- [UI-MCP-002] + [UI-MCP-003] 遮罩吸收抖动、保留静态回归 ----------------
        //
        // 抖动取**面积最大**的那块动态区（RTA 频谱块），并让它"掉到地板"（黑）——
        // 这是 DAW 里真实发生的事（静音/无信号 ⇒ 频谱柱落到地线）。
        //
        // ⚠️ 为什么必须"亮度差大 + 成块"才断言：SSIM 是局部统计（均值/方差/协方差）的均值，
        // 对**细长条**与**等亮度换色**几乎不敏感。本机用真实 SSIM 实现预演过（notes §2 第 29 条）：
        //   · 8x88 窄条改成纯色      → 未遮罩 SSIM = 0.9999（**拉不下阈值**）
        //   · 60x88 大块改成等亮度平色 → 未遮罩 SSIM = 0.9967（**拉不下阈值**）
        //   · 60x88 大块掉到黑        → 未遮罩 SSIM = 0.7040（拉得下 ✓）
        // 所以这里既保留窄条（证明遮罩对任意大小都生效），又用大块来证明"遮罩是承重的"。
        let mut jittered = first.clone();
        let biggest = mask_rects
            .iter()
            .copied()
            .max_by_key(|rect| rect.area())
            .expect("至少有一个动态区");
        jittered.fill_rect(biggest, [0x00, 0x00, 0x00]);

        let unmasked = ssim::ssim(&first, &jittered).expect("同尺寸可算");
        assert!(
            biggest.area() * 100 >= FIXTURE_SIZE.pixel_count() * 10,
            "用于'必须被检出'断言的动态区应占画面 ≥10% (实测 {} px / {} px) —— 否则该断言在 SSIM 口径下无意义",
            biggest.area(),
            FIXTURE_SIZE.pixel_count()
        );
        assert!(
            !ssim::Verdict::with_default_threshold(unmasked).passed,
            "未遮罩的抖动必须被检出, 实测 SSIM {unmasked:.6}"
        );
        let verdict = compare_with_dynamic_masking(&first, &jittered, &tree).expect("遮罩比对");
        assert!(verdict.passed, "遮罩后必须通过: {verdict}");
        assert!(
            (verdict.score - 1.0).abs() <= f64::EPSILON,
            "遮罩后实测 {}",
            verdict.score
        );

        // 静态区被改动 ⇒ 遮罩之后仍然要低于阈值（遮罩不能把整幅图变成盲区）。
        let mut regressed = first.clone();
        regressed.fill_rect(Rect::new(8, 24, 64, 40), [0x00, 0x00, 0x00]);
        let regressed_verdict =
            compare_with_dynamic_masking(&first, &regressed, &tree).expect("遮罩比对");
        assert!(
            !regressed_verdict.passed,
            "静态回归必须被检出: {regressed_verdict}"
        );

        // ---------------- [ARCH-UI-004] 值 / 勾选态：来自**活组件**，不是测试常量 ----------------
        //
        // 规范原文（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:236`）：
        // *"AI Agent 可通过 JSON-RPC 查询控件树（Widget Tree），提取坐标、尺寸、可见性及
        // **自定义绑定状态（如推子电平）**"*。下面四步把"读得到"钉牢：
        //   ① 节点上有值 / 勾选态；② 没声明的元素如实为 `None`；
        //   ③ **改根属性 ⇒ 读回来必须跟着变**（宿主若抄了一份常量，第 ③ 步变红）；
        //   ④ `property_of("value")` 读的是**同一份活组件**。
        //
        // 为什么"改一次再读"是承重的：只断言一个初值，无法排除"主机把 `.slint` 里的
        // 初值抄进了一个 Rust 常量"。夹具的 `fader-value` / `mute-on` 是根属性
        // （`ui/fixture.slint`），测试改它们 —— 那不是测试自己写进树里的常量。
        assert_eq!(
            tree.find_by_id("mixer-vu-track-0")
                .expect("夹具节点")
                .value
                .as_deref(),
            Some("-6.0 dB"),
            "`accessible-value` 必须进 `ControlNode::value`"
        );
        assert_eq!(
            tree.find_by_id("transport-play-button")
                .expect("夹具节点")
                .checked,
            Some(false),
            "`accessible-checked` 必须进 `ControlNode::checked`"
        );
        // 没有声明这两个属性的元素：如实 `None`（不是空串 / `false`）。
        let bare = tree.find_by_id("clip-01J8ZQ9K2M-header").expect("夹具节点");
        assert_eq!(bare.value, None, "没声明 `accessible-value` ⇒ `None`");
        assert_eq!(bare.checked, None, "没声明 `accessible-checked` ⇒ `None`");
        assert_eq!(
            bare.value, None,
            "空串会与『声明了空值』混为一谈 —— 缺席必须是 None"
        );

        // 改活组件的根属性 ⇒ 重抓之后读回来必须变。
        port.ui().set_fader_value("-3.5 dB".into());
        port.ui().set_mute_on(true);
        let refreshed = port
            .refresh_tree(Some(&registry))
            .expect("重抓运行时控件树")
            .clone();
        assert_eq!(
            refreshed
                .find_by_id("mixer-vu-track-0")
                .expect("夹具节点")
                .value
                .as_deref(),
            Some("-3.5 dB"),
            "改 `fader-value` 之后读回来的值必须跟着变（常量抄本在这里变红）"
        );
        assert_eq!(
            refreshed
                .find_by_id("transport-play-button")
                .expect("夹具节点")
                .checked,
            Some(true),
            "翻转 `mute-on` 之后 `checked` 必须跟着翻"
        );

        // `property_of` 走的是**同一份活组件**（不是上面那棵树）。
        let vu_handle = inspect::find_by_accessible_id(port.ui(), "mixer-vu-track-0")
            .expect("活组件里有 `mixer-vu-track-0`");
        assert_eq!(
            inspect::property_of(&vu_handle, "value").as_deref(),
            Some("-3.5 dB"),
            "`ui/property {{name:\"value\"}}` 必须从活组件读到当前值"
        );
        let play_handle = inspect::find_by_accessible_id(port.ui(), "transport-play-button")
            .expect("活组件里有 `transport-play-button`");
        assert_eq!(
            inspect::property_of(&play_handle, "checked").as_deref(),
            Some("true"),
            "`ui/property {{name:\"checked\"}}` 必须从活组件读到当前勾选态"
        );
        assert_eq!(
            inspect::property_of(&vu_handle, "checked"),
            None,
            "进度条没有勾选态 ⇒ 如实 None（不是 \"false\"）"
        );
        let bare_handle = inspect::find_by_accessible_id(port.ui(), "clip-01J8ZQ9K2M-header")
            .expect("活组件里有剪辑包头");
        assert_eq!(
            inspect::property_of(&bare_handle, "value"),
            None,
            "没声明 `accessible-value` 的元素 ⇒ `property_of` 也给 None（不是空串）"
        );
        assert_eq!(
            inspect::property_of(&bare_handle, "nope"),
            None,
            "未知属性名同样是 None —— 两者由 `is_optional_property` 分开报"
        );
        assert!(
            inspect::is_optional_property("value") && inspect::is_optional_property("checked"),
            "两个可选属性名必须在 `OPTIONAL_PROPERTY_NAMES` 里（否则 `read_property` \
             会把『元素没声明』说成『不支持这个属性名』）"
        );
        assert!(
            !inspect::is_optional_property("role"),
            "`role` 是必答属性, 不是可选属性"
        );

        // `read_property` 的 `None` 有**两种**含义，话术必须分开：
        //   ① 元素没声明这个可选属性（元素的事实）；
        //   ② 名字根本不支持（调用方写错了）。
        // 把它们混成一句会让调用方去查文档找拼写。
        match port.read_property("clip-01J8ZQ9K2M-header", "value") {
            Err(PortError::Rejected { message }) => {
                assert!(
                    message.contains("没有声明 `accessible-value`"),
                    "元素没声明的可选属性必须说清是『没声明』: {message}"
                );
                assert!(
                    !message.contains("不支持的属性名"),
                    "『没声明』不能混成『不支持这个属性名』: {message}"
                );
            }
            other => panic!("没声明 `accessible-value` 必须报 `Rejected`: {other:?}"),
        }
        match port.read_property("clip-01J8ZQ9K2M-header", "nope") {
            Err(PortError::Rejected { message }) => {
                assert!(
                    message.contains("不支持的属性名"),
                    "未知属性名必须说清是『不支持这个属性名』: {message}"
                );
            }
            other => panic!("未知属性名必须报 `Rejected`: {other:?}"),
        }
        // 有值的元素照常读到（与 `property_of` 同一份活组件）。
        assert_eq!(
            port.read_property("mixer-vu-track-0", "value").as_deref(),
            Ok("-3.5 dB"),
            "端口路径必须读到活组件的当前值"
        );

        // ---------------- [UI-MCP-001] 三级权限在真实端口上的行为 ----------------
        assert_eq!(port.permission(), Permission::ReadOnly);
        assert!(matches!(
            port.dispatch_key_press(KeyCode::Tab),
            Err(PortError::PermissionDenied {
                required: Permission::Interactive,
                ..
            })
        ));
        assert_eq!(
            port.dispatch_pointer_down("transport-play-button", 2.0, 2.0, PointerButton::Left),
            Err(PortError::PermissionDenied {
                operation: crate::port::Operation::DispatchPointer,
                required: Permission::Interactive,
                actual: Permission::ReadOnly,
            })
        );
        // ReadOnly 允许的三件事。
        assert!(!port.capture_png().expect("只读允许截图").is_empty());
        assert_eq!(
            port.read_property("transport-play-button", "role")
                .as_deref(),
            Ok("button")
        );
        let width = port
            .read_property("transport-play-button", "width")
            .expect("宽度是支持的属性");
        assert!(
            width.parse::<f64>().is_ok_and(|value| value > 0.0),
            "实测 width={width}"
        );
        assert!(matches!(
            port.read_property("transport-play-button", "nope"),
            Err(PortError::Rejected { .. })
        ));

        // ---------------- 过程产物落盘（target/ 下, 不提交） ----------------
        let png_path = write_artifact("fixture-port-fixture", &first).expect("写 PNG");
        let json_path = artifact_dir().join("control-tree.json");
        std::fs::write(&json_path, tree.dump_json()).expect("写控件树 JSON");
        eprintln!("Golden 证据: {}", png_path.display());
        eprintln!("控件树 JSON: {}", json_path.display());
    }

    /// `[BASELINE-003 / 第 149 轮定案]` 帧率**计时路径**的判据（TIER-1 夹具）。
    ///
    /// **它证明什么**：`request_redraw()` + `capture()` 这一对真能把帧光栅化出来（见证非平凡），
    /// 并给出真实 p50/p99/max 分布。**它不证明**门禁的 10 万音符门限 —— 夹具不是钢琴卷帘，
    /// 所以这里**故意不**断言 `p99 <= 8.3ms`（那会是拿夹具冒充门禁）。10 万音符场景由 `yeban-app` 的真实视图提供。
    #[test]
    fn frame_time_path_produces_a_real_distribution_and_non_trivial_frames() {
        const FRAMES: usize = 600;
        let registry = fixture_registry();
        // 每个测试跑在自己的线程上 ⇒ 平台可在此线程装一次（上游 MinimalSoftwareWindow 的同一约束）。
        let port = LivePort::new(
            FIXTURE_SIZE,
            Permission::ReadOnly,
            Some(&registry),
            PortFixture::new,
        )
        .expect("Tier-1 平台 + 组件");

        let window = port.window();
        let mut samples_ms: Vec<f64> = Vec::with_capacity(FRAMES);
        let mut witness_non_black_min = usize::MAX;
        for _ in 0..FRAMES {
            let start = std::time::Instant::now();
            window.request_redraw();
            let image = window.capture().expect("每帧都应能抓到像素");
            samples_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            let evidence = golden_evidence(&image).expect("见证");
            witness_non_black_min = witness_non_black_min.min(evidence.summary().len());
        }

        samples_ms.sort_by(|a, b| a.partial_cmp(b).expect("无 NaN"));
        let p = |q: f64| samples_ms[((samples_ms.len() as f64 - 1.0) * q).round() as usize];
        let (p50, p99, max) = (p(0.50), p(0.99), samples_ms[samples_ms.len() - 1]);
        eprintln!(
            "BASELINE-003(夹具) 帧数={FRAMES} p50={p50:.3}ms p99={p99:.3}ms max={max:.3}ms 见证字符数下限={witness_non_black_min}"
        );

        assert_eq!(samples_ms.len(), FRAMES, "必须采满 {FRAMES} 帧");
        // 见证：每帧的 golden 证据都必须有内容，否则"很快"可能来自空帧。
        assert!(
            witness_non_black_min > 0,
            "每帧的 golden 证据都应有内容（下限 {witness_non_black_min}）"
        );
        assert!(p50 > 0.0, "p50 必须为正，否则计时路径没有真在工作");
    }
}
