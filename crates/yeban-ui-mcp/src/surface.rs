//! 执行面：JSON-RPC 方法**真正落到哪里**。
//!
//! ## 分工（这是本 crate 与 `yeban-ui-test-port` 的边界）
//!
//! ```text
//!   AI Agent ──JSON-RPC──> yeban-ui-mcp  ──进程内调用──> yeban-ui-test-port
//!             (本 crate)                   (UiSurface)        (UiTestPort 实现)
//! ```
//!
//! `yeban-ui-test-port` 已经有一套完整的进程内调用面与三级权限
//! （`UiTestPort` / `PortError` / `authorize`，`[UI-MCP-001]` §12.3）。
//! 本 crate **不重写**它，只做一件事：把 JSON-RPC 的请求翻译成那些调用，
//! 并在网络层再判一次 scope。
//!
//! 因此 [`UiSurface`] 是 `UiTestPort` 的**超集**，只多一个方法
//! [`UiSurface::capture_image`]。为什么要多这一个：
//!
//! - `UiTestPort::capture_png` 只给 PNG 字节，而 `[UI-MCP-002]` §12.5 的
//!   **强制遮罩**需要在像素矩阵上把动态区置黑 —— 我们**没有** PNG 解码器
//!   （`yeban-ui-test-port` 手写 PNG 时明确只做编码，见那边 notes §4），
//!   所以遮罩必须在**编码之前**、在 `Rgb8Image` 上做；
//! - Tier-1 的像素证据（尺寸非零 / 非全黑 / 颜色数，`[MUST-GATE-015]`）也来自同一张图。
//!
//! 于是"截图"这条路只有一条：`capture_image()` → （可选）遮罩 → 证据 → PNG 编码。
//! 判据 `screenshot_evidence_requires_a_non_black_non_empty_frame` 钉住这条。

use serde_json::{Map, Value};
use yeban_ui_test_port::image::Rgb8Image;
use yeban_ui_test_port::png::{self, REPO_MAX_FILE_BYTES};
use yeban_ui_test_port::port::{PortError, UiTestPort};

use crate::ime::ImeState;

/// 单帧截图的像素证据（`[MUST-GATE-015]`：尺寸非零且非全黑）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotEvidence {
    /// 宽（像素）。
    pub width: u32,
    /// 高（像素）。
    pub height: u32,
    /// 非黑像素数。
    pub non_black_pixels: u64,
    /// 不同颜色数。
    pub distinct_colors: usize,
    /// 编码后的 PNG 字节数。
    pub png_bytes: usize,
    /// PNG 字节的 FNV-1a 64 指纹（与 `yeban_ui_test_port::render::fnv1a64` **同算法**）：
    /// 让"两次截图是否逐字节相同"可以只比一个短标识。**不是**安全摘要。
    pub fingerprint: u64,
}

impl ShotEvidence {
    /// 由图像 + 已编码的 PNG 字节构造，并执行 `[MUST-GATE-015]` 的两条门槛。
    ///
    /// # Errors
    ///
    /// 尺寸为零，或整帧全黑（"渲染了但什么都没画"是**失败**，不是一张合法截图）。
    pub fn new(image: &Rgb8Image, png_bytes: &[u8]) -> Result<Self, PortError> {
        let size = image.size();
        if size.is_empty() {
            return Err(PortError::Capture {
                message: "Tier-1 帧缓冲尺寸为零: [MUST-GATE-015] 要求尺寸非零".to_owned(),
            });
        }
        if image.is_all_black() {
            return Err(PortError::Capture {
                message: "整帧全黑: [MUST-GATE-015] 要求非全黑 (渲染了但一个像素都没画)".to_owned(),
            });
        }
        Ok(Self {
            width: size.width,
            height: size.height,
            non_black_pixels: image.non_black_pixels(),
            distinct_colors: image.distinct_color_count(),
            png_bytes: png_bytes.len(),
            fingerprint: fnv1a64(png_bytes),
        })
    }
}

/// FNV-1a 64 位指纹（零依赖、逐位确定），与 `yeban_ui_test_port::render::fnv1a64` 同算法。
///
/// 为什么这里再写一遍而不是直接调那边的：那是 `render.rs` 里的函数，而 `render.rs`
/// 依赖 Slint —— 本 crate 的**绝大部分逻辑**（含判据）必须在**零 Slint** 的前提下
/// 在本机真跑（见 `docs/ledger/ui-mcp-notes.md` 的"本机验证"一节）。
/// 两份实现的一致性由 CI 侧判据 `fingerprint_matches_the_tier1_renderer` 逐字节对账 ——
/// 那一条会引用 `render::fnv1a64`，因此它不需要在本机跑。
#[must_use]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 执行面：`yeban-ui-test-port` 的进程内调用面 + Tier-1 像素访问。
///
/// 实现者：
/// - 真实无头窗口：`yeban_ui_test_port::render::LivePort<T>` 经 [`crate::live::ControlPlane`]
///   装配（真实接线在 `crates/yeban-app/src/live_surface.rs`）；
/// - 判据里的假面（零 Slint，可以在本机跑）→ `crate::service::tests`。
pub trait UiSurface: UiTestPort {
    /// 执行面的稳定标识（进响应与日志，**不要**放令牌或路径）。
    fn surface_name(&self) -> &'static str;

    /// 抓 Tier-1 帧缓冲的 RGB8 像素。
    ///
    /// # Errors
    ///
    /// 光栅化失败（`PortError::Capture`）。
    fn capture_image(&self) -> Result<Rgb8Image, PortError>;

    /// 把执行面持有的**运行时树缓存**刷到"当下"。读运行时树的 `ui/*` 方法在返回前都调它。
    ///
    /// ## 为什么需要它（本方法就是"AI 能看见当下的界面"的承重点）
    ///
    /// [`UiTestPort::tree`] 借出的是一棵**缓存**树：`yeban_ui_test_port::render::LivePort`
    /// 只在构造与显式 `refresh_tree` 时重抓它。同一份控制面里 `ui/property` 与
    /// `ui/screenshot` 读的却是**活窗口** ⇒ 两种读法的"新鲜度"会分叉：
    /// 人点一下按钮改了可见性，`ui/property` 立刻读到新值，`ui/tree` / `ui/node`
    /// 却还是上一次重抓那一棵。本方法把这个分叉合上：**读之前先刷新**。
    ///
    /// ## 默认实现为什么是"什么都不做"（而不是一个错误）
    ///
    /// 不是每个执行面都有"活的运行时树"可刷：判据里的零 Slint 假面
    /// （[`crate::testing`]）持有一棵**静态**树，它没有可刷新的来源。对它报错会把
    /// "这个执行面没有活的树"变成一个失败 —— 而那不是失败，是那个执行面的事实。
    /// 有活窗口的执行面（`yeban-app` 的 `LiveAdminSurface`）覆写本方法。
    ///
    /// # Errors
    ///
    /// 重抓失败（执行面自己的错误）。**不回退到旧缓存**：把一棵已知过期的树当成
    /// "当下的界面"发出去，正是本方法要消灭的那种假成功。
    fn refresh_runtime_tree(&mut self) -> Result<(), PortError> {
        Ok(())
    }

    /// 取走**上一个管理动作**的结构化回执（`ui/switch_main_view` / `ui/force_save` /
    /// `ui/reload_engine`）。默认 `None`。
    ///
    /// ## 为什么需要它（"如实报告结果"的可判据形态）
    ///
    /// §12.3 的三个 Administrative 动作在**执行面**上真的做了什么（切到了哪个视图 /
    /// 写了多少字节 / 引擎重建到第几代、推了多少量子），只有执行面自己知道。
    /// 服务层把 `*_impl` 的空返回合成一个 `{"accepted": true}` 是**不够**的：
    /// 那既证明不了副作用发生过，也让"失败被吞掉"与"成功"长得一样。
    ///
    /// 因此：执行面把事实交出来（[`AdminReport`]），服务层原样放进 `result.report`。
    /// **默认实现返回 `None`**，所以既有的假执行面与 [`PortAdapter`] 一行都不用改，
    /// 行为与从前完全一致（有判据钉住这一点：管理动作的**前后**结果在默认执行面上不变）。
    ///
    /// 取走（而不是借出）是刻意的：回执属于"那一次调用"，重复读会让调用方分不清
    /// 两次调用各报了什么。因此命名是 `take_*`。
    fn take_admin_report(&mut self) -> Option<AdminReport> {
        None
    }

    /// `[UI-A11Y-002]` 的 **IME 合成态读数**（`is_composing`）。默认 `None` = 这个执行面
    /// 没有 IME 状态机 ⇒ 服务层**如实报** `-32005 NOT_IMPLEMENTED`，而不是编一个 `false`。
    ///
    /// 真实载体必须由执行面交出**它自己那一份**状态（`crates/yeban-app/src/live_surface.rs`
    /// 持 `Rc<RefCell<InputContext>>`，与按键处置共用同一个对象）——**不是**在这里新造一个
    /// 影子变量。理由与判据见 [`crate::ime`]。
    fn ime_state(&self) -> Option<ImeState> {
        None
    }

    /// `dryRun=true` 时"**将要发生什么**"的只读影响预览（ADR-0001 **D48**）。
    ///
    /// 语义（与领域侧的 `domain::preview` 同族，`crates/yeban-mcp/src/domain/mod.rs:1620`）：
    ///
    /// - **只读**：签名是 `&self` ⇒ 借用检查器不允许它改任何东西（"dryRun 不改状态"
    ///   因此不是靠自觉，与领域侧 `dryRun` 走 `&Domain` 同一套保证）；
    /// - **真校验**：`Err` 表示"这次真调用**一定会失败**"——服务层把它映射成与真调用
    ///   **同一个**错误码（领域侧的口径是"dryRun 只做参数与领域合法性校验"，
    ///   因此"没有活跃工程 / 未配置保存路径"这类失败必须如实报，不许伪造一个成功预览）；
    /// - `Ok(None)` = 这个执行面给不出影响预览（服务层如实报 `preview.effect = null`，
    ///   不编造"大概会这样"）。
    ///
    /// `method` 是线格式方法名（`ui/*`），`arguments` 是**归一化后的实参**
    /// （不含 `dryRun` 自身与 `_` 保留键，见 [`crate::dry_run::normalized_arguments`]）。
    /// 实参以 [`PreviewArguments`] 交出（**不是** `serde_json::Value`）、影响以
    /// [`PreviewEffect`] 交出 —— 与 [`ReportValue`] 同一个理由：接线方（`yeban-app`）
    /// **不依赖 `serde_json`**，不该为了报一句"arrangement-view 会变成 false"而多一个依赖。
    ///
    /// # Errors
    ///
    /// 执行面判定"这次真调用会失败"时返回它自己的 [`PortError`]。
    fn preview_effect(
        &self,
        method: &str,
        arguments: &PreviewArguments,
    ) -> Result<Option<PreviewEffect>, PortError> {
        let _ = (method, arguments);
        Ok(None)
    }
}

/// `dryRun` 预览的**只读实参视图**（`name` → 归一化后的值）。
///
/// 内部持有 `serde_json::Value`，但**一个字段都不公开**：接线方只能通过
/// [`PreviewArguments::text`] / [`PreviewArguments::number`] /
/// [`PreviewArguments::is_true`] 取值，因此不需要 `serde_json` 出现在它的依赖里。
/// 理由与 [`ReportValue`] 的文件级注释逐字相同。
#[derive(Debug, Clone, Default)]
pub struct PreviewArguments {
    entries: Vec<(String, Value)>,
}

impl PreviewArguments {
    /// 由归一化后的实参构造（**唯一**的构造点，在服务层）。
    #[must_use]
    pub fn from_arguments(arguments: &Map<String, Value>) -> Self {
        Self {
            entries: arguments
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        }
    }

    /// 取一个字符串实参。
    #[must_use]
    pub fn text(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| value.as_str())
    }

    /// 取一个数值实参。
    #[must_use]
    pub fn number(&self, name: &str) -> Option<f64> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| value.as_f64())
    }

    /// 取一个布尔实参。
    #[must_use]
    pub fn is_true(&self, name: &str) -> Option<bool> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| value.as_bool())
    }

    /// 实参个数（判据用）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否没有实参。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `dryRun` 的**只读影响预览**：有序字段表。
///
/// 有序（`Vec` 而不是 `Map`）的理由与 [`AdminReport`] 逐字相同：它进响应、日志与
/// artifact，键序必须稳定（`[MODEL-AST-003]` 确定性的精神）。
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewEffect {
    /// 有序字段。
    pub fields: Vec<(&'static str, ReportValue)>,
}

impl PreviewEffect {
    /// 组装。
    #[must_use]
    pub fn new(fields: Vec<(&'static str, ReportValue)>) -> Self {
        Self { fields }
    }

    /// 线格式（唯一转换点）：`{"<field>": …}`。
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut root = Map::new();
        for (name, value) in &self.fields {
            root.insert((*name).to_owned(), value.to_json());
        }
        Value::Object(root)
    }
}

/// 管理动作回执里的一个值。
///
/// 存在的理由：回执要**跨 crate** 交给本 crate（接线方在 `yeban-app`），而把
/// `serde_json::Value` 放进公开签名会让接线方不得不依赖 `serde_json` 才能说一句
/// "写了 4096 字节"。这个枚举只覆盖回执真正需要的形态，转换点**只有一处**
/// （[`ReportValue::to_json`]），因此线上形态仍然稳定。
#[derive(Debug, Clone, PartialEq)]
pub enum ReportValue {
    /// 布尔。
    Bool(bool),
    /// 有符号整数。
    Int(i64),
    /// 非负计数（`u64`）。
    Uint(u64),
    /// 浮点（例如 dBFS / 版本）。
    Float(f64),
    /// 文本（路径之外的一切短标识：视图名 / 容器布局名）。
    Text(String),
}

impl ReportValue {
    /// 线格式（唯一转换点）。
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Bool(value) => Value::from(*value),
            Self::Int(value) => Value::from(*value),
            Self::Uint(value) => Value::from(*value),
            Self::Float(value) => Value::from(*value),
            Self::Text(value) => Value::from(value.as_str()),
        }
    }
}

impl From<bool> for ReportValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for ReportValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<u64> for ReportValue {
    fn from(value: u64) -> Self {
        Self::Uint(value)
    }
}

impl From<usize> for ReportValue {
    fn from(value: usize) -> Self {
        Self::Uint(value as u64)
    }
}

impl From<f64> for ReportValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<&str> for ReportValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for ReportValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

/// 一个管理动作的**结构化回执**（谁做的、报的是什么）。
///
/// `operation` 与方法注册表里的方法名同名（判据
/// `admin_reports_name_the_operation_that_was_actually_run` 钉住"回执不是别人写的"），
/// `fields` 是有序的（`Vec` 而不是 `Map`）：回执进日志与 artifact，键序必须稳定 ——
/// 与 `[MODEL-AST-003]` 的确定性要求同族（红线 4 的精神）。
#[derive(Debug, Clone, PartialEq)]
pub struct AdminReport {
    /// 真的跑过的那个动作（`"switch_main_view"` / `"force_save"` / `"reload_engine"`）。
    pub operation: &'static str,
    /// 有序字段。
    pub fields: Vec<(&'static str, ReportValue)>,
}

impl AdminReport {
    /// 组装。
    #[must_use]
    pub fn new(operation: &'static str, fields: Vec<(&'static str, ReportValue)>) -> Self {
        Self { operation, fields }
    }

    /// 线格式：`{"operation": …, <field>: …}`。
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut root = Map::new();
        root.insert("operation".to_owned(), Value::from(self.operation));
        for (name, value) in &self.fields {
            root.insert((*name).to_owned(), value.to_json());
        }
        Value::Object(root)
    }
}

/// 把一张（可能已遮罩的）图编码成 PNG 并算出证据。
///
/// 这是截图的**唯一出口**：证据与 PNG 必须来自**同一张**图，否则"证据说的是 A 帧、
/// 发出去的是 B 帧"这类错误无法被发现。
///
/// # Errors
///
/// PNG 超过 `limit`（默认 [`REPO_MAX_FILE_BYTES`]，仓库单文件上限 10 MiB），
/// 或 `[MUST-GATE-015]` 的两条门槛不满足。
pub fn encode_with_evidence(
    image: &Rgb8Image,
    limit: usize,
) -> Result<(Vec<u8>, ShotEvidence), PortError> {
    // `[MUST-GATE-015]` 的两条门槛**先**判: 一张空帧/全黑帧根本不该进入编码器
    // (那既浪费时间, 也会让"编码器拒绝了 0 宽 PNG"与"渲染什么都没画"混成同一个错误)。
    if image.size().is_empty() {
        return Err(PortError::Capture {
            message: "Tier-1 帧缓冲尺寸为零: [MUST-GATE-015] 要求尺寸非零".to_owned(),
        });
    }
    if image.is_all_black() {
        return Err(PortError::Capture {
            message: "整帧全黑: [MUST-GATE-015] 要求非全黑 (渲染了但一个像素都没画)".to_owned(),
        });
    }
    let bytes = png::encode_rgb8_limited(image, limit).map_err(|error| PortError::Capture {
        message: error.to_string(),
    })?;
    let evidence = ShotEvidence::new(image, &bytes)?;
    Ok((bytes, evidence))
}

/// 默认的 PNG 体积上限（仓库单文件上限，`AGENTS.md` §2 红线 9）。
pub const DEFAULT_MAX_PNG_BYTES: usize = REPO_MAX_FILE_BYTES;

/// 把任意 `UiTestPort` + 一个"怎么拿像素"的闭包升格成 [`UiSurface`]。
///
/// ## 为什么需要这一层（以及为什么它不是多余的抽象）
///
/// `UiSurface` 只比 `UiTestPort` 多两件事：一个名字，与 `capture_image`。
/// **真实**的像素只能从具体窗口类型拿 —— 在无头形态下那是
/// `yeban_ui_test_port::render::LivePort<T>` 的 `window().capture()`，而
/// `LivePort<T>` 的定义带 `T: slint::ComponentHandle` 约束 ⇒ 想直接为它实现
/// `UiSurface` 就必须在本 crate 里写出 `slint::ComponentHandle`，
/// 也就是**把 slint 变成直接依赖**。
///
/// 那条路被否掉的理由不是"少一个依赖更优雅"，而是**判据的可执行性**：
/// 一旦 `crates/yeban-ui-mcp/src/` 里出现 `use slint::…`，本 crate 里**任何**引用到它的
/// 模块都无法在本机 `rustc --test` 真跑（本机纪律禁止编译 Slint，见
/// `docs/ledger/ui-mcp-notes.md`）。把"取像素"变成一个闭包之后：
///
/// - 持有窗口的一方（`yeban-app` 侧，两行代码）注入真实实现；
/// - 本 crate 的全部逻辑（含截图/遮罩/证据链）保持零 Slint，可以在本机真跑。
///
/// 用法（`yeban-app` 侧）：
///
/// ```ignore
/// let port = yeban_ui_test_port::render::LivePort::new(size, permission, Some(&registry), build)?;
/// let surface = PortAdapter::new(port, "tier1-live-port", |port| {
///     port.window().capture().map_err(|error| PortError::Capture { message: error.to_string() })
/// });
/// ```
pub struct PortAdapter<P, F> {
    port: P,
    name: &'static str,
    capture: F,
}

impl<P, F> PortAdapter<P, F>
where
    P: UiTestPort,
    F: Fn(&P) -> Result<Rgb8Image, PortError>,
{
    /// 组装。
    #[must_use]
    pub fn new(port: P, name: &'static str, capture: F) -> Self {
        Self {
            port,
            name,
            capture,
        }
    }

    /// 内层端口（只读）。
    #[must_use]
    pub fn port(&self) -> &P {
        &self.port
    }

    /// 内层端口（可变）。
    pub fn port_mut(&mut self) -> &mut P {
        &mut self.port
    }
}

impl<P, F> UiSurface for PortAdapter<P, F>
where
    P: UiTestPort,
    F: Fn(&P) -> Result<Rgb8Image, PortError>,
{
    fn surface_name(&self) -> &'static str {
        self.name
    }

    fn capture_image(&self) -> Result<Rgb8Image, PortError> {
        (self.capture)(&self.port)
    }
}

impl<P, F> UiTestPort for PortAdapter<P, F>
where
    P: UiTestPort,
    F: Fn(&P) -> Result<Rgb8Image, PortError>,
{
    fn permission(&self) -> yeban_ui_test_port::port::Permission {
        self.port.permission()
    }
    fn tree(&self) -> &yeban_ui_test_port::tree::ControlTree {
        self.port.tree()
    }
    fn capture_png(&self) -> Result<Vec<u8>, PortError> {
        self.port.capture_png()
    }
    fn read_property(&self, element_id: &str, name: &str) -> Result<String, PortError> {
        self.port.read_property(element_id, name)
    }
    fn dispatch_pointer_down_impl(
        &mut self,
        element_id: &str,
        x_offset: f64,
        y_offset: f64,
        button: yeban_ui_test_port::port::PointerButton,
    ) -> Result<(), PortError> {
        self.port
            .dispatch_pointer_down_impl(element_id, x_offset, y_offset, button)
    }
    fn dispatch_pointer_move_impl(&mut self, x: f64, y: f64) -> Result<(), PortError> {
        self.port.dispatch_pointer_move_impl(x, y)
    }
    fn dispatch_pointer_up_impl(
        &mut self,
        button: yeban_ui_test_port::port::PointerButton,
    ) -> Result<(), PortError> {
        self.port.dispatch_pointer_up_impl(button)
    }
    fn dispatch_key_press_impl(
        &mut self,
        key: yeban_ui_test_port::port::KeyCode,
    ) -> Result<(), PortError> {
        self.port.dispatch_key_press_impl(key)
    }
    fn switch_main_view_impl(&mut self, view: &str) -> Result<(), PortError> {
        self.port.switch_main_view_impl(view)
    }
    fn force_save_impl(&mut self) -> Result<(), PortError> {
        self.port.force_save_impl()
    }
    fn reload_engine_impl(&mut self) -> Result<(), PortError> {
        self.port.reload_engine_impl()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_ui_test_port::image::{Rect, Size};

    fn painted() -> Rgb8Image {
        let mut image = Rgb8Image::new(Size::new(24, 12));
        image.fill_rect(Rect::new(0, 0, 24, 12), [200, 30, 30]);
        image.fill_rect(Rect::new(4, 4, 8, 4), [10, 10, 10]);
        image
    }

    /// 判据 1: 证据与 PNG 来自同一张图；指纹可复现；两次编码逐字节相同。
    #[test]
    fn evidence_and_png_come_from_the_same_frame() {
        let image = painted();
        let (bytes, evidence) = encode_with_evidence(&image, DEFAULT_MAX_PNG_BYTES).expect("编码");
        assert_eq!(evidence.width, 24);
        assert_eq!(evidence.height, 12);
        assert_eq!(evidence.png_bytes, bytes.len());
        assert_eq!(evidence.fingerprint, fnv1a64(&bytes));
        assert_eq!(evidence.distinct_colors, 2);
        assert_eq!(evidence.non_black_pixels, 24 * 12, "两个颜色都不是纯黑");

        let (again, evidence_again) =
            encode_with_evidence(&image, DEFAULT_MAX_PNG_BYTES).expect("编码");
        assert_eq!(bytes, again, "同一帧两次编码必须逐字节相同");
        assert_eq!(evidence, evidence_again);

        // PNG 魔数（证明这真是 PNG，而不是一段随便的字节）。
        assert_eq!(&bytes[..8], &yeban_ui_test_port::png::PNG_SIGNATURE);
    }

    /// 判据 2: `[MUST-GATE-015]` —— 尺寸为零 / 全黑都**报错**，不许当成"成功的一张图"。
    #[test]
    fn screenshot_evidence_requires_a_non_black_non_empty_frame() {
        // 零尺寸图像在**上游类型**里就构造不出来（`Rgb8Image::new` panic、
        // `from_raw` 报 `EmptySize`）—— 所以"尺寸为零"这条门槛有两道网：
        // 上游拒绝构造 + 本函数的第一道检查（防御性，正常路径不可达）。
        assert!(Rgb8Image::from_raw(Size::new(0, 0), Vec::new()).is_err());
        assert!(
            Rgb8Image::from_raw(Size::new(0, 4), Vec::new()).is_err(),
            "任一边为 0 都必须被拒"
        );

        let black = Rgb8Image::new(Size::new(16, 16));
        assert!(black.is_all_black());
        assert!(
            matches!(
                encode_with_evidence(&black, DEFAULT_MAX_PNG_BYTES),
                Err(PortError::Capture { .. })
            ),
            "全黑必须被拒 (MUST-GATE-015)"
        );

        // 一个像素就够翻案。
        let mut almost = Rgb8Image::new(Size::new(16, 16));
        almost.set_pixel(3, 3, [0, 0, 1]);
        assert!(encode_with_evidence(&almost, DEFAULT_MAX_PNG_BYTES).is_ok());
    }

    /// 判据 3: 遮罩在**编码之前**动像素 —— 遮罩后的证据与 PNG 都反映置黑结果。
    ///
    /// （`[UI-MCP-002]` §12.5 的可核验形态：置黑必须真的进到发出去的字节里。）
    #[test]
    fn masking_happens_before_encoding() {
        let image = painted();
        let rects = vec![Rect::new(0, 0, 24, 6)];
        let masked = yeban_ui_test_port::mask::masked(&image, &rects);
        let (raw_bytes, raw_evidence) =
            encode_with_evidence(&image, DEFAULT_MAX_PNG_BYTES).expect("原始帧");
        let (masked_bytes, masked_evidence) =
            encode_with_evidence(&masked, DEFAULT_MAX_PNG_BYTES).expect("遮罩帧");

        assert_ne!(raw_bytes, masked_bytes, "遮罩必须真的改变字节");
        assert_eq!(
            masked_evidence.distinct_colors, 3,
            "遮罩引入了纯黑, 因此颜色数由 2 变 3 (`distinct_color_count` 把黑也算一种颜色)"
        );
        assert!(masked_evidence.non_black_pixels < raw_evidence.non_black_pixels);
        assert_eq!(masked_evidence.non_black_pixels, 24 * 6);
        assert!(yeban_ui_test_port::mask::mask_is_effective(&masked, &rects));
    }

    /// 判据 4: 超过体积上限时必须**显式报错**，不是发出去一个会被仓库红线拒的文件。
    #[test]
    fn oversized_png_is_an_explicit_error() {
        let image = painted();
        let error = encode_with_evidence(&image, 16).expect_err("超过上限必须报错");
        assert!(
            matches!(error, PortError::Capture { .. }),
            "实际错误: {error:?}"
        );
    }

    /// 判据 5: [`PortAdapter`] 是**透明**的 —— 12 个 `UiTestPort` 方法逐个委托，
    /// 只有 `capture_image` 走注入的闭包；`UiSurface::surface_name` 用注入的名字。
    ///
    /// 这条判据让 `yeban-app` 侧那两行接线是**可验证**的（本机零 Slint 就能跑）。
    #[test]
    fn port_adapter_delegates_everything_and_injects_pixels() {
        use crate::testing::{FakeSurface, fixture_tree, shared};
        use yeban_ui_test_port::port::{Permission, UiTestPort};

        let state = shared(Permission::Administrative);
        let inner = FakeSurface {
            state: std::rc::Rc::clone(&state),
            tree: fixture_tree(),
        };
        let mut adapter = PortAdapter::new(inner, "tier1-live-port", |port: &FakeSurface| {
            port.capture_image()
        });

        assert_eq!(adapter.surface_name(), "tier1-live-port");
        assert_eq!(
            adapter.permission(),
            Permission::Administrative,
            "权限必须透传 (否则纵深防御的第二道闸门会被绕过)"
        );
        assert_eq!(adapter.tree().len(), 3);
        let image = adapter.capture_image().expect("闭包提供像素");
        assert_eq!(image.size().width, 200);
        assert!(adapter.capture_png().is_ok());
        assert_eq!(
            adapter
                .read_property("track-0-fader", "value")
                .expect("属性"),
            "value=1"
        );
        adapter
            .dispatch_key_press(yeban_ui_test_port::port::KeyCode::Tab)
            .expect("注入");
        assert_eq!(
            state.borrow().calls,
            ["key_press:Tab".to_owned()],
            "委托必须真的落到内层端口"
        );

        // 越权的注入仍然被内层端口的闸门挡住（适配器不改变任何权限语义）。
        let read_only = PortAdapter::new(
            FakeSurface {
                state: shared(Permission::ReadOnly),
                tree: fixture_tree(),
            },
            "read-only",
            |port: &FakeSurface| port.capture_image(),
        );
        let mut read_only = read_only;
        assert!(matches!(
            read_only.dispatch_key_press(yeban_ui_test_port::port::KeyCode::Tab),
            Err(PortError::PermissionDenied { .. })
        ));
    }

    /// 判据 6（ADR-0001 **D48** / `[UI-A11Y-002]`）: 适配器的两个新钩子**不编造事实** ——
    /// 没有 IME 状态就没有（`None`，服务层据此报 `-32005`），给不出影响预览就没有
    /// （`Ok(None)` ⇒ 线上 `effect: null`）。一个"默认返回 `false` / `{}`"的实现会把
    /// "这个执行面没这能力"伪装成"有，只是值是空的"。
    #[test]
    fn port_adapter_never_invents_ime_state_or_preview_effects() {
        use crate::testing::{FakeSurface, fixture_tree, shared};
        use yeban_ui_test_port::port::Permission;

        let adapter = PortAdapter::new(
            FakeSurface {
                state: shared(Permission::Administrative),
                tree: fixture_tree(),
            },
            "tier1-live-port",
            |port: &FakeSurface| port.capture_image(),
        );
        // `PortAdapter` 只转发 `UiTestPort`；IME 状态与 dryRun 预览由**接线方**
        // （`yeban-app` 的 `LiveAdminSurface`）提供，因此这里必须是"没有"。
        assert!(
            adapter.ime_state().is_none(),
            "适配器不得凭空造一个 IME 状态"
        );
        let arguments = PreviewArguments::default();
        assert!(arguments.is_empty());
        assert_eq!(arguments.len(), 0);
        assert!(arguments.text("view").is_none());
        assert!(arguments.number("x").is_none());
        assert!(arguments.is_true("dryRun").is_none());
        assert!(
            adapter
                .preview_effect(crate::methods::METHOD_FORCE_SAVE, &arguments)
                .expect("默认实现不报错")
                .is_none(),
            "适配器不得凭空造一个影响预览"
        );
    }
}
