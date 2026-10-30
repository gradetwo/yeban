//! 语义元素 ID 注册表 —— `[UI-TEST-001]` 与 `[UI-MCP-002]` 的 Rust 侧基础。
//!
//! 规范来源 (Normative):
//! - `[UI-TEST-001]` UI/UX 规范 §12.2「稳定语义元素寻址」: 自动化**必须**基于语义
//!   Element ID, 严禁绝对像素坐标。约定族: `track-{i}-fader` / `note-{ulid}-rect` /
//!   `clip-{ulid}-header` / `tab-{name}-button`。
//! - `[UI-MCP-002]` §12.5「动态区域自动遮罩」: 走带光标 / VU 电平 / 时间码每帧跳变,
//!   截图比对前必须按元素包围盒置黑 —— 因此注册表要能**回答"哪些是动态区域"**。
//! - `[UI-MCP-001]` §12.3: 三级权限分层的第一层 `ReadOnly` 就是"检索控件树 + 读属性",
//!   它消费的正是这份注册表。
//!
//! ## 为什么 .slint 侧还需要一份注册表
//!
//! Slint 1.18.1 **确实**有原生元素标识机制, 但有两个, 且都不是我们要的那一个:
//!
//! 1. `accessible-id` —— `.slint` 里的字符串属性, 语义最接近 "DOM id"。核验出处:
//!    语言参考「Common Properties & Callbacks」的 Accessibility Properties 一节
//!    (<https://docs.slint.dev/latest/docs/slint/reference/common/>), 原文:
//!    "A unique identifier for the element, used to identify widgets for automation and
//!    testing purposes."。`ui/` 下每个关键节点都设了它。
//! 2. 元素 `id` (`foo := Rectangle {}`) —— 由 `i-slint-backend-testing` 的
//!    `ElementHandle::find_by_element_id` 查询, 但它的键是
//!    **`组件名::局部名`** 这种"限定 id", 不是我们想要的业务语义
//!    (出处: <https://docs.rs/i-slint-backend-testing/1.18.1/i_slint_backend_testing/struct.ElementHandle.html>)。
//!
//! 两者都**只在有窗口实例时才存在**。而 yeban-app 的无头路径 (`--headless`) 故意不构造
//! 窗口 (见 `src/main.rs` 的理由), 所以需要一份**纯 Rust、零 Slint 依赖**的注册表:
//! CI 可以在没有显示器、没有渲染后端的情况下断言"元素该在的都在, ID 都合法, 动态区域都登记了"。
//! 有窗口实例时, `yeban-ui-test-port` 用 `accessible_id()` / `find_by_element_id` 交叉核对
//! 这份注册表 (那是它的工作, 不在本文件范围)。
//!
//! ## 红线 4 的精神: 不用 HashMap
//!
//! `AGENTS.md` §2 红线 4 禁止在持久化 AST 里用 `HashMap`/`HashSet`。注册表不是持久化 AST,
//! 但同样是"跨进程、跨重启必须给出同一份迭代顺序"的集合 —— 截图回归与元素清单 diff 都要
//! 稳定的顺序。因此这里用 `BTreeMap`, 并用测试钉死"迭代顺序是排序后的"。

use std::collections::BTreeMap;

use crate::bridge::ViewState;
use crate::scene::{CONSOLE_TABS, TOOL_NAMES};

/// 一个语义元素在无障碍树里的角色。
///
/// 取值与 `.slint` 的 `accessible-role` 枚举**同名同义** —— 核验出处:
/// 语言参考「Built-in Enums → AccessibleRole」
/// (<https://docs.slint.dev/latest/docs/slint/reference/property-types/builtin-enums/>)。
/// 保留这层映射而不是直接暴露 `&'static str`, 是为了让"注册表说它是 slider, `.slint` 里
/// 却写了 button"这类漂移在类型层面就可疑。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ElementKind {
    /// `button`: 可按下并触发一个动作。
    Button,
    /// `slider`: 连续可调值 (推子 / 分割线 / 声像)。
    Slider,
    /// `text-input`: 可编辑文本 (BPM 敲入 / 搜索框)。
    TextInput,
    /// `tab`: 单个标签页。
    Tab,
    /// `tab-list`: 标签页容器。
    TabList,
    /// `list`: 列表容器。
    List,
    /// `list-item`: 列表 / 集合里的一个条目 (音符、剪辑、插槽、通道条)。
    ListItem,
    /// `region`: 有边界的语义区域 (面板、画布)。
    Region,
    /// `progress-indicator`: 进度 / 电平类指示器。
    ProgressIndicator,
    /// `switch`: 开关 (旁通)。
    Switch,
    /// `text`: 只读文本。
    Text,
    /// `combobox`: 下拉选择 (分支选择器)。
    Combobox,
    /// `groupbox`: 分组容器 (通道条)。
    GroupBox,
    /// `image`: 图形 (频响曲线占位)。
    Image,
    /// `complementary`: 地标 —— 补充主内容的分区 (侧栏、AI 协作栏)。
    Complementary,
    /// `content-info`: 地标 —— 应用或内容的信息条 (状态栏)。
    ContentInfo,
    /// `main`: 地标 —— 主内容区 (工作区画布)。每个视图恰好一个。
    Main,
    /// ⭐ `banner`: 地标 —— 横跨全宽、与主内容**并列**的一条通知 (任务单 `#86` 甲)。
    ///
    /// 取 Slint `AccessibleRole::Banner`（上游
    /// `i-slint-common-1.18.1/enums.rs` 的变体，kebab 名 `banner` 已登记在
    /// `yeban_ui_test_port::tree::KNOWN_ROLES`，本单之前**没有**任何元素用它）。
    ///
    /// ⚠ 为什么不用 `content-info`：那个角色已经被状态栏占着，而"一条错误横幅"
    /// 与"状态栏"是**两个**语义节点（判据要能分开寻址，屏读器要能分开念）。
    /// 上游的角色表里**没有** `alert` —— `banner` 是可用的最近似取值，如实登记这一点。
    Banner,
}

impl ElementKind {
    /// 返回 `.slint` `accessible-role` 的字面取值。
    #[must_use]
    pub const fn accessible_role(self) -> &'static str {
        match self {
            Self::Button => "button",
            Self::Slider => "slider",
            Self::TextInput => "text-input",
            Self::Tab => "tab",
            Self::TabList => "tab-list",
            Self::List => "list",
            Self::ListItem => "list-item",
            Self::Region => "region",
            Self::ProgressIndicator => "progress-indicator",
            Self::Switch => "switch",
            Self::Text => "text",
            Self::Combobox => "combobox",
            Self::GroupBox => "groupbox",
            Self::Image => "image",
            Self::Complementary => "complementary",
            Self::ContentInfo => "content-info",
            Self::Main => "main",
            Self::Banner => "banner",
        }
    }
}

/// 一个语义元素的元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementMeta {
    /// 语义 ID, 形如 `track-0-fader` / `note-01J8…-rect`。
    pub id: String,
    /// 无障碍角色。
    pub kind: ElementKind,
    /// 定义它的 `.slint` 文件 (相对 `crates/yeban-app/ui/`), 用于把断言失败映射回源码。
    pub component: &'static str,
    /// 人类可读标签, 对应 `.slint` 的 `accessible-label`。
    pub label: String,
    /// `[UI-MCP-002]` 是否为每帧跳变的高频刷新区域 (截图比对前必须置黑)。
    pub dynamic_region: bool,
}

/// 语义元素注册表: `BTreeMap<id, ElementMeta>`。
///
/// 重复插入同一个 ID 是**编程错误** —— [`Self::insert`] 会返回被顶掉的旧值,
/// [`Self::demo`] 直接用 `expect` 炸掉, 测试会变红。
#[derive(Debug, Clone, Default)]
pub struct ElementRegistry {
    entries: BTreeMap<String, ElementMeta>,
}

impl ElementRegistry {
    /// 按 `ui/` 下 `.slint` 文件里**实际写着**的 `accessible-id` 构建演示注册表。
    ///
    /// 演示注册表 = [`crate::bridge::ViewState::demo`]（演示 `YebanProjectV1` 的投影）
    /// 的注册表。它不是"另一份常量"—— 与真实工程走的是**同一条**路径：
    /// [`Self::from_view`]。
    #[must_use]
    pub fn demo() -> Self {
        Self::from_view(&ViewState::demo())
    }

    /// 由**投影结果**构造注册表：条目数、`{ulid}` 段与标签全部来自工程。
    ///
    /// 覆盖面: 骨架里每一个带 `accessible-id` 的节点。`.slint` 里加了 ID 而这里没加,
    /// `console_tab_buttons_cover_every_declared_tab` 一类的判据抓不住 —— 那种漂移靠
    /// `yeban-ui-test-port` 的有窗实例对账 (见模块文档), 属于已登记的边界。
    ///
    /// 哪些族**不由**工程驱动（如实登记，见 `docs/ledger/app-binding-notes.md`）：
    /// 侧栏资源 / 控制台标签 / 卷帘工具矩阵 / 两个对话框 —— 它们要么是规范级常量
    /// （工具矩阵），要么没有对应的模型实体。
    ///
    /// ⭐ **任务单 `#157` 移出了"设备机架"**：设备卡片现在**由** `ViewState::devices`
    /// 驱动（条目数 / 名字 / 旁通 / 参数读数）⇒ 它不再是静态部件。⚠ 它**没有**被加进
    /// [`MODEL_DRIVEN_FAMILIES`]：那份清单是**前缀**表，加 `"device-"` 会把
    /// `device-rack`（区域本身）与 `device-rack-eq-curve`（占位图）也算成工程驱动；
    /// 设备卡的"数据来自工程"由 `tests/device_rack_projection.rs` 直接钉住。
    #[must_use]
    pub fn from_view(view: &ViewState) -> Self {
        let mut registry = Self::default();

        // ------------------------------------------------------------ 外壳
        registry.add(
            "sidebar",
            ElementKind::Complementary,
            "sidebar.slint",
            "左侧资源浏览器与资产库",
            false,
        );
        registry.add(
            "sidebar-collapse-button",
            ElementKind::Button,
            "sidebar.slint",
            "收起 / 展开左侧资源栏",
            false,
        );
        registry.add(
            "sidebar-search-field",
            ElementKind::TextInput,
            "sidebar.slint",
            "搜索资源",
            false,
        );
        for (index, label) in ["乐器", "采样", "预置"].iter().enumerate() {
            registry.add(
                &format!("sidebar-category-{index}-button"),
                ElementKind::Tab,
                "sidebar.slint",
                &format!("资源分类 {label}"),
                false,
            );
        }
        for index in 0..8 {
            registry.add(
                &format!("sidebar-item-{index}"),
                ElementKind::ListItem,
                "sidebar.slint",
                &format!("资源条目 {index}"),
                false,
            );
        }

        // ------------------------------------------------------------ 顶栏走带
        registry.add(
            "transport-play-button",
            ElementKind::Button,
            "transport.slint",
            "播放 / 暂停 (Space)",
            false,
        );
        registry.add(
            "transport-stop-button",
            ElementKind::Button,
            "transport.slint",
            "停止 (回到起始点)",
            false,
        );
        registry.add(
            "transport-record-button",
            ElementKind::Button,
            "transport.slint",
            "录音",
            false,
        );
        // `ROAD-M4-008` 选项 (a)：产品二进制里的**保存**入口（用户够得到的那个按钮）。
        // 它不改工程内容，只是把"保存"交给宿主（`host::wire_save` → `save_action`）。
        registry.add(
            "transport-save-button",
            ElementKind::Button,
            "transport.slint",
            "保存工程",
            false,
        );
        registry.add(
            "transport-bpm-field",
            ElementKind::TextInput,
            "transport.slint",
            "速度 BPM",
            false,
        );
        registry.add(
            "transport-timecode",
            ElementKind::Text,
            "transport.slint",
            "主时间码",
            true,
        );
        registry.add(
            "transport-branch-button",
            ElementKind::Combobox,
            "transport.slint",
            "当前分支",
            false,
        );
        registry.add(
            "transport-commit-button",
            ElementKind::Button,
            "transport.slint",
            "提交当前编辑为一个 Commit",
            false,
        );
        registry.add(
            "transport-revert-button",
            ElementKind::Button,
            "transport.slint",
            "回滚到上一个 Commit",
            false,
        );
        registry.add(
            "transport-ai-proposal-badge",
            ElementKind::Button,
            "transport.slint",
            // 2026-10-08（诚实性审计）：这里原写 `"AI 编曲提案待审查"` —— 那是一个**没有
            // 数据源的状态声明**，与 `ui/transport.slint` 的假读数同源。徽章现在只声明
            // 它的真实动作（`open-ai-proposals` → 打开提案抽屉），标签随之改为动作描述。
            "打开 AI 编曲提案审核抽屉",
            false,
        );
        registry.add(
            "transport-view-session-button",
            ElementKind::Tab,
            "transport.slint",
            "Session 触发矩阵 (F5)",
            false,
        );
        registry.add(
            "transport-view-arrangement-button",
            ElementKind::Tab,
            "transport.slint",
            "Arrangement 时间轴 (F6)",
            false,
        );
        registry.add(
            "transport-view-toggle-button",
            ElementKind::Button,
            "transport.slint",
            "切换 Session / Arrangement (Tab, 仅画布聚焦时)",
            false,
        );
        registry.add(
            "transport-ai-drawer-button",
            ElementKind::Button,
            "transport.slint",
            "AI 协作栏抽屉",
            false,
        );

        // ------------------------------------------------------------ splitter
        registry.add(
            "splitter-workspace-console",
            ElementKind::Slider,
            "app.slint",
            "主工作区与底部控制台的分割线 (220–600px)",
            false,
        );

        // ------------------------------------------------------------ 状态栏
        registry.add(
            "status-bar",
            ElementKind::ContentInfo,
            "status_bar.slint",
            "状态栏",
            false,
        );
        registry.add(
            "status-bar-selection",
            ElementKind::Text,
            "status_bar.slint",
            "选区信息",
            true,
        );
        registry.add(
            "status-bar-chord",
            ElementKind::Text,
            "status_bar.slint",
            "实时和弦识别",
            true,
        );
        registry.add(
            "status-bar-shortcut-tip",
            ElementKind::Text,
            "status_bar.slint",
            "快捷键提示",
            false,
        );
        // `ROAD-M4-008` 选项 (a)：最近一次保存的结果（宿主注入的**原话**，成功与失败都在）。
        // 与上一格共用同一块 480px（`.slint` 的 `visible` 二选一），是两个语义节点。
        registry.add(
            "status-bar-save-status",
            ElementKind::Text,
            "status_bar.slint",
            "最近一次保存工程的结果",
            true,
        );
        registry.add(
            "status-bar-device",
            ElementKind::Text,
            "status_bar.slint",
            // 台账 R9：标签必须与 `ui/status_bar.slint` 的 `accessible-label` **逐字相同**
            // （判据 `test_port_adapter` 断言运行时 label == 注册表 label）。
            // 原文写"硬件声卡采样率"，而这一格显示的是**工程音频配置**
            // （引擎据此向声卡协商），不是声卡协商回来的读数。
            "音频配置与性能目标",
            true,
        );

        // ⭐ ------------------------------------------------- 引擎错误横幅
        // 任务单 `#86` 甲：状态栏**上方**的一条覆盖层（`ui/engine_error_banner.slint`）。
        //
        // ⚠ 它**不进** `DEFAULT_VIEW_MUST_HAVE`：没有错误时 `.slint` 侧
        // `visible = text != ""` 为假 ⇒ 上游遍历**过滤掉**它 ⇒ 无头控件树里
        // 它根本不出现。它进的是 `test_port_adapter` 的 `DEFAULT_VIEW_MUST_NOT_HAVE`
        // （那条判据正是"默认帧里它不显示"的机械形态）。
        //
        // `dynamic_region = false`：它只在**错误变了**的那一跳重画（不是每帧跳变），
        // 因此金标比对**不**需要遮罩它 —— 与 VU 表 / 走带光标 / 时间码不同。
        registry.add(
            "engine-error-banner",
            ElementKind::Banner,
            // ⚠ 组件住在 `app.slint` 的**内联** `EngineErrorBanner` 里，⛔ 不是自己的
            // 一个 `.slint` 文件：`ADR-0001` **D2** 冻结了 `ui/` 的清单，且由判据
            // `elements::tests::slint_manifest_matches_adr_0001_d2` 机械看守。
            "app.slint",
            // ⚠ 必须与 `.slint` 的 `accessible-label` **逐字相同**
            // （`test_port_adapter` 断言运行时 label == 注册表 label）。
            "引擎错误横幅",
            false,
        );

        // ------------------------------------------------------------ AI 协作栏
        registry.add(
            "ai-rail",
            ElementKind::Complementary,
            "app.slint",
            "AI 协作栏: 意图生成与声学诊断",
            false,
        );
        registry.add(
            "ai-rail-intent-button",
            ElementKind::Button,
            "app.slint",
            "用自然语言描述编曲意图",
            false,
        );
        registry.add(
            "ai-rail-diagnose-button",
            ElementKind::Button,
            "app.slint",
            "声学诊断",
            false,
        );
        registry.add(
            "diagnostics-export-action",
            ElementKind::Button,
            "app.slint",
            "导出诊断包 (D56)",
            false,
        );
        registry.add(
            "ai-rail-musical-pr-button",
            ElementKind::Button,
            "app.slint",
            "打开 AI 编曲提案审核抽屉",
            false,
        );

        // ------------------------------------------------- **模块入口**（五个）
        //
        // 它们住在右栏的下半区（`ui/module_actions.slint` 的 `ModuleActions`，由
        // `app.slint` 的 `AiRail` 实例化 —— 既有三条控件止于 y=204，
        // 因此这些入口**没有**移动任何**更早**的控件；第五条（硬件 MIDI 输入）追加在
        // 四个入口之后，只把两条**空文本**状态行往下让位（空 Text 不产像素）。
        //
        // 标签必须与 `.slint` 的 `accessible-label` **逐字相同**：`test_port_adapter`
        // 的运行时树对账会断言 label 相等。四条回调的宿主实现全在
        // `host::wire_module_actions`（复用 `export_wav` / `import_midi` / `sfz_load` /
        // `record_wav` 四个**已经存在**的模块，⛔ 不另写一套）。
        //
        // ⚠ `module-last-status` 是**动态**的（`dynamic = true`）：它的文本由
        // 宿主写进 `MainWindow.module-status`，每次按下都可能不同
        // ⇒ `[UI-MCP-002]` 的截图遮罩必须按元素包围盒置黑它。
        // 它是 e2e「触发入口 ⇒ 状态文本按预期变化」的**可寻址**落点。
        for (id, label) in [
            (
                "module-export-wav-button",
                "导出 WAV（打开采样率 / 位深 / 时长选项）",
            ),
            (
                "module-import-midi-button",
                "导入 MIDI（把 SMF 的音符导入第一条非主总线音轨）",
            ),
            (
                "module-load-sfz-button",
                "装载 SFZ 乐器（报告区域数并探测一个音符）",
            ),
            (
                "module-record-wav-button",
                "录音（把控制侧测试音录成 24-bit WAV）",
            ),
            // ⭐ 第五条：**硬件** MIDI 输入（`crate::midi_in::capture_messages`）。
            // ⚠ 它与 `module-import-midi-button` 不是同一件事：那条读**文件**（SMF），
            // 这条读**物理端口**（`[ADR-0006]`）。
            // ⭐ 落点 = CLI 的 `--midi-in` **＋** `--midi-in-write-notes`：采集完成后，
            // 配对出的音符写进**第一条非主总线音轨**（规则在 `crate::import_midi::destination`）。
            (
                "module-midi-in-button",
                "硬件 MIDI 输入（采集物理端口事件，并把配对出的音符写进第一条非主总线音轨）",
            ),
        ] {
            registry.add(
                id,
                ElementKind::Button,
                "module_actions.slint",
                label,
                false,
            );
        }
        registry.add(
            "module-actions-title",
            ElementKind::Text,
            "module_actions.slint",
            "模块入口：导出 WAV / 导入 MIDI / 装载 SFZ / 录音 / 硬件 MIDI 输入",
            false,
        );
        // ⭐ 硬件 MIDI 输入的**专属**结果行（与 `module-last-status` 是同一串字节、
        // 同一个写者 ⇒ 两个可寻址的显示位，不是一个第二真相源）。
        // ⚠ 动态区：每次按下都变 ⇒ 截图遮罩要按它的包围盒置黑。
        registry.add(
            "module-midi-in-status",
            ElementKind::Text,
            "module_actions.slint",
            "最近一次硬件 MIDI 输入的采集结果（端口 / 事件数 / 不支持数 / 丢弃事件数 / 抖动 / 错误码）",
            true,
        );
        registry.add(
            "module-last-status",
            ElementKind::Text,
            "module_actions.slint",
            // 标签与 `.slint` 的 `accessible-label` **逐字相同**（`test_port_adapter`
            // 的运行时对账断言 label 相等）。标签是**静态**的（内容默认空串，绑上去会
            // 让节点无名字）；回执原文住在可见 `text`、`MainWindow.module-status` 与
            // `accessible-value` 上（`#236`：三处绑同一个属性）。
            "最近一次模块入口动作的回执（路径 / 帧数 / 来源 / 错误码）",
            // 动态区：回执每次按下都变 ⇒ 截图比对的遮罩要按它的包围盒置黑。
            true,
        );
        // ⭐ 任务单 `#230`：**母带削波警示行**（`module_actions.slint` 的
        // `module-clipping-warning`，`y = 448`）。
        //
        // 它与上面两条状态行不是同一格：只放一行短句（削波样本数 / 建议减多少 dB），
        // 空串 ⇒ `visible: false` ⇒ 默认帧里不在运行时树、不画像素。
        // 动态区：导出不同的工程 / 削波读数不同 ⇒ 截图遮罩按它的包围盒置黑。
        registry.add(
            "module-clipping-warning",
            ElementKind::Text,
            "module_actions.slint",
            // 与 `.slint` 的 `accessible-label` 逐字相同。
            "母带削波警示（削波样本数 / 建议减多少 dB）",
            true,
        );
        // ⭐ 任务单 `#319`／`#320`：**MIDI 文件导入的损失摘要行**（`module_actions.slint`
        // 的 `module-import-losses`，`y = 448` —— 与削波警示**同一槽位**、互斥）。
        // `#320` 起同一行也带**歌词的非零丢失桶**（完整五项在回执 `lyrics_detail`）。
        //
        // 空串 ⇒ `visible: false` ⇒ 默认帧里不在运行时树、不画像素（与 `#230` 同款）。
        // 唯一写者 = `host::apply_module_receipt_with_alerts`：每次动作同时写两条，
        // 没给的那条写空串 ⇒ 任一时刻最多一条可见。
        // 动态区：每次导入的读数不同 ⇒ 截图遮罩按它的包围盒置黑。
        registry.add(
            "module-import-losses",
            ElementKind::Text,
            "module_actions.slint",
            // 与 `.slint` 的 `accessible-label` 逐字相同。
            "最近一次 MIDI 文件导入的损耗摘要（类数 / 事件数 / 歌词丢失 / 音符通道 / 释放力度）",
            true,
        );

        // ⭐ **导出 MIDI**（任务单 `#90` 甲）—— 第六条模块入口。
        //
        // 它与上面五条**同一族**（ID 族 `module-{动作}-button`、同一行回执的
        // `action=export-midi`、同一个宿主写者 `host::wire_module_actions`），但**物理位置
        // 不同**：它住在 `ui/console/mixer_console.slint` 里（控制台 **Tab 1 = 调音台**），
        // 因为 `ai-rail` 在**默认帧**里是可见的静态像素，在那里多一个按钮会让 5 张 Linux
        // 基准确定性过期，而本机是 macOS、`tests/golden/<platform>` 只有 `linux`
        // （⛔ 本机无法按 `[UI-MCP-003]` 重录）。`MixerConsole` 只在 `active-tab == 1` 时
        // 可见 ⇒ 默认帧逐位不变（同一纪律的既有先例：`#86` 乙的「浏览…」按钮）。
        //
        // ⛔ 它不是"第二个导出器"：落点是**已经存在**的
        // `crate::export_midi::export_project_to_file`（与 CLI 的 `--export-midi` 同一个函数）。
        registry.add(
            "module-export-midi-button",
            ElementKind::Button,
            "console/mixer_console.slint",
            // 与 `.slint` 的 `accessible-label` **逐字相同**（运行时树对账会断言相等）。
            "导出 MIDI（把当前工程写成 SMF 1 文件）",
            false,
        );

        // ⭐ **打开工程**（任务单 `#93`）—— **运行期换工程**的可见入口。
        //
        // 与 `module-export-midi-button` **逐条同款**：同一族 ID（`module-{动作}-button`）、
        // 同一个面板行、同一条 `module-status` 回执通道、**同一个物理位置纪律**
        // （控制台 **Tab 1 = 调音台**；`MixerConsole` 只在 `active-tab == 1` 时可见
        // ⇒ **默认帧（Tab 0 卷帘）里没有它** ⇒ 5 张 Linux 基准逐位不变，而本机是 macOS
        // ⇒ 无法按 `[UI-MCP-003]` 重录）。
        //
        // 它补的是 `#86` 乙 如实登记过的那一格：原生对话框那时只接到**已有的**运行期入口
        // （`LiveUi::open_project` / 控制面 `ui/open_project`）上，产品 `run_gui` 里
        // **没有**可点的入口（`--open` 只在启动时读一次）。本条的唯一落点是
        // `host::wire_open_project`：读盘复用 `crate::open::open_project_file`、
        // 换会话复用 `UndoPort::open_project_session`（「打开一个工程 = **新会话**」），
        // ⛔ 没有第二份读盘 / 换工程实现。
        registry.add(
            "module-open-project-button",
            ElementKind::Button,
            "console/mixer_console.slint",
            // 与 `.slint` 的 `accessible-label` **逐字相同**（运行时树对账会断言相等）。
            "打开工程（用文件对话框挑一个 .yeban 容器并就地换掉当前工程）",
            false,
        );

        // ⭐ **新建工程**（任务单 `#129`）—— **从零开始**的可见入口。
        //
        // 与上面那条**逐条同款**：同一族 ID（`module-{动作}-button`）、同一个「工程」面板
        // （就在「打开工程」正下方 `y: 56px`）、同一条 `module-status` 回执通道、同一个
        // 物理位置纪律（控制台 **Tab 1 = 调音台** ⇒ 默认帧里没有它 ⇒ 5 张 Linux 基准
        // 逐位不变）。它补的是人类实测被卡的那一格：GUI 只能"打开"已经存在的 `.yeban`，
        // 不能"新建"。唯一落点是 `host::wire_new_project`：落盘复用
        // `save::save_archive_file`（与 CLI `--save-as` 同一条）、换工程复用
        // `OpenProjectRuntime::open_runtime`（与「打开工程」同一条），⛔ 没有第二份实现。
        registry.add(
            "module-new-project-button",
            ElementKind::Button,
            "console/mixer_console.slint",
            // 与 `.slint` 的 `accessible-label` **逐字相同**（运行时树对账会断言相等）。
            "新建工程（用文件对话框给新工程选一个落点，写出 0 音轨 ＋ 1 主总线的空白容器并就地换过去）",
            false,
        );

        // ------------------------------------------------------------ Session View
        registry.add(
            "workspace-session-canvas",
            ElementKind::Main,
            "workspace/session_view.slint",
            "Session 触发矩阵",
            false,
        );
        for (track_index, track) in view.tracks.iter().enumerate() {
            registry.add(
                &format!("session-track-{track_index}-header"),
                ElementKind::ListItem,
                "workspace/session_view.slint",
                &format!("Session 轨道 {}", track.name),
                false,
            );
            for (scene_index, _scene) in view.scenes.iter().enumerate() {
                registry.add(
                    &format!("slot-{track_index}-{scene_index}-cell"),
                    ElementKind::ListItem,
                    "workspace/session_view.slint",
                    &format!("轨道 {} 的场景插槽 {scene_index}", track.name),
                    false,
                );
            }
        }
        registry.add(
            "scene-launch-column-header",
            ElementKind::Text,
            "workspace/session_view.slint",
            "场景一键激发列",
            false,
        );
        for (scene_index, scene) in view.scenes.iter().enumerate() {
            registry.add(
                &format!("scene-launch-{scene_index}-button"),
                ElementKind::Button,
                "workspace/session_view.slint",
                &format!("激发场景 {}", scene.name),
                false,
            );
            // 场景色标（`SceneV3::color` 的投影形态）：标签携带规范化 `#RRGGBB`，
            // 与轨道色标同一条纪律 —— 控件树里读到的就是投影字段。
            registry.add(
                &format!("scene-{scene_index}-color-swatch"),
                ElementKind::Image,
                "workspace/session_view.slint",
                &format!("场景色标 {}", scene.color_hex),
                false,
            );
        }
        registry.add(
            "session-back-to-arrangement-button",
            ElementKind::Button,
            "workspace/session_view.slint",
            "返回编曲 (Back to Arrangement)",
            false,
        );
        // ⭐ 任务单 `#283`：场景启动的**如实回执**（没有引擎 / 事件环满时非空）。
        // 它不是交互控件（`accessible-role: text`），但必须有登记 —— `ui/` 里每个
        // `accessible-id` 都要被注册表覆盖（判据 `slint_accessible_ids_and_registry_cover_each_other`）。
        registry.add(
            "session-scene-launch-status",
            ElementKind::Text,
            "workspace/session_view.slint",
            "场景启动状态",
            false,
        );
        // ⭐ 任务单 `#190`：画布装不下的轨道列**如实报数**（场景列原先按 `tracks.length`
        // 定位 ⇒ 1440 档第 7 轨起 / 1024 展开态第 4 轨起被推出画布）。只在
        // `轨道数 > 容量` 时在场；注册表按"无条件声明"登记（缺失不算红）。
        registry.add(
            "session-track-truncation-note",
            ElementKind::Text,
            "workspace/session_view.slint",
            "Session 轨道列截断报数（还有几轨未显示）",
            false,
        );

        // ------------------------------------------------------------ Arrangement View
        registry.add(
            "workspace-arrangement-canvas",
            ElementKind::Main,
            "workspace/arrangement_view.slint",
            "Arrangement 线性编曲时间轴",
            false,
        );
        for (section_index, section) in view.sections.iter().enumerate() {
            registry.add(
                &format!("section-{section_index}-card"),
                ElementKind::ListItem,
                "workspace/arrangement_view.slint",
                &format!("章节 {}", section.name),
                false,
            );
            // 段落色标（`SectionV3::color` 的投影形态）：标签携带规范化 `#RRGGBB`，
            // 与轨道 / 场景色标同一条纪律 —— 控件树里读到的就是投影字段。
            registry.add(
                &format!("section-{section_index}-color-swatch"),
                ElementKind::Image,
                "workspace/arrangement_view.slint",
                &format!("段落色标 {}", section.color_hex),
                false,
            );
        }
        registry.add(
            "arrangement-ruler",
            ElementKind::Text,
            "workspace/arrangement_view.slint",
            "小节与拍标尺",
            false,
        );
        registry.add(
            "arrangement-loop-brace",
            ElementKind::Slider,
            "workspace/arrangement_view.slint",
            "循环选区",
            false,
        );
        for (track_index, track) in view.tracks.iter().enumerate() {
            registry.add(
                &format!("track-{track_index}-header"),
                ElementKind::ListItem,
                "workspace/arrangement_view.slint",
                &format!("轨道包头 {}", track.name),
                false,
            );
            registry.add(
                &format!("track-{track_index}-mute-button"),
                ElementKind::Button,
                "workspace/arrangement_view.slint",
                &format!("轨道 {} 静音", track.name),
                false,
            );
            registry.add(
                &format!("track-{track_index}-solo-button"),
                ElementKind::Button,
                "workspace/arrangement_view.slint",
                &format!("轨道 {} 独奏", track.name),
                false,
            );
            // 轨道色标：标签携带**规范化**的 `#RRGGBB`（非法 / 缺失已在投影层回退）。
            // 它让"投影 ↔ 控件树"的色标一致性在纯 Rust 侧与 Tier-1 运行时树两侧都能断言
            // （`[UI-A11Y-004]` 的色标纪律；见 docs/ledger/app-completion-notes.md §2）。
            registry.add(
                &format!("track-{track_index}-color-swatch"),
                ElementKind::Image,
                "workspace/arrangement_view.slint",
                &format!("轨道色标 {}", track.color_hex),
                false,
            );
        }
        for (clip_index, clip) in view.clips.iter().enumerate() {
            registry.add(
                &format!("clip-{}-header", clip.placement_id),
                ElementKind::ListItem,
                "workspace/arrangement_view.slint",
                &format!("剪辑包头 {clip_index}"),
                false,
            );
            // 拖动面（`clip-{ulid}-body`）：与包头**同一取值域**的身份再登记一次。
            // 它是指针手势的输入面（`[UI-TEST-002]` 注入拖动用它寻址），角色 = `region`
            // （`.slint` 里 `accessible-role: region`，零视觉元素）。
            // 标签与 `.slint` 的 `"剪辑拖动面 " + clip-labels[i]` 同构：用剪辑标签而不是
            // 下标 —— 包头用下标是既有口径（本轮不改），拖动面按"身份 + 标签"写。
            registry.add(
                &format!("clip-{}-body", clip.placement_id),
                ElementKind::Region,
                "workspace/arrangement_view.slint",
                &format!("剪辑拖动面 {}", clip.label),
                false,
            );
        }
        registry.add(
            "arrangement-playhead",
            ElementKind::Slider,
            "workspace/arrangement_view.slint",
            "走带光标",
            true,
        );

        // ---- 自动化泳道（`line/app-automation-ui`） ----
        //
        // 每条泳道一个元素：ID 与 `ui/workspace/arrangement_view.slint` 里拼出来的
        // `"track-" + i + "-automation-" + 目标键 + "-lane"` 逐字相同（由
        // `slint_accessible_ids_and_registry_cover_each_other` 双侧对账），标签携带
        // **单位与当前值** —— 于是"AI/判据能读到自动化"不是靠截图猜的。
        //
        // 角色是 `image`（它是曲线的图形载体，与 EQ 频响曲线 / 色标同款）。
        // **不是**动态遮罩区：曲线随工程变化，但不随每帧的走带 / 电平跳变。
        for lane in &view.automation_lanes {
            registry.add(
                &lane.element_id,
                ElementKind::Image,
                "workspace/arrangement_view.slint",
                &lane.label,
                false,
            );
        }

        // ------------------------------------------------------------ 控制台导轨
        registry.add(
            "console-tab-rail",
            ElementKind::TabList,
            "console/console_tabs.slint",
            "底部控制台标签导轨",
            false,
        );
        for tab in CONSOLE_TABS {
            registry.add(
                &format!("tab-{}-button", tab.name),
                ElementKind::Tab,
                "console/console_tabs.slint",
                tab.label,
                false,
            );
        }
        registry.add(
            "console-maximize-button",
            ElementKind::Button,
            "console/console_tabs.slint",
            "最大化 / 还原底部控制台",
            false,
        );

        // ------------------------------------------------------------ 钢琴卷帘
        registry.add(
            "piano-roll",
            ElementKind::Region,
            "console/piano_roll.slint",
            "虚拟化钢琴卷帘",
            false,
        );
        for (tool_index, name) in TOOL_NAMES.iter().enumerate() {
            registry.add(
                &format!("piano-roll-tool-{name}-button"),
                ElementKind::Button,
                "console/piano_roll.slint",
                &format!("卷帘工具 {name} ({})", tool_index + 1),
                false,
            );
        }
        registry.add(
            "piano-roll-status",
            ElementKind::Text,
            "console/piano_roll.slint",
            "卷帘吸附与音域状态",
            false,
        );
        registry.add(
            "piano-roll-keys",
            ElementKind::List,
            "console/piano_roll.slint",
            "钢琴键列",
            false,
        );
        registry.add(
            "piano-roll-grid",
            ElementKind::Region,
            "console/piano_roll.slint",
            "音符网格画布",
            false,
        );
        // `[ROAD-M3-002]` 拖动滚动手势的**输入面**（`[UI-TEST-001]` §12.2 语义 ID）：
        // 它是"Slint 发增量、宿主拥有偏移"那条链的**起点**，无头判据靠这个 ID 按语义寻址注入
        // （`crates/yeban-app/tests/piano_roll_drag_scroll.rs`）。
        registry.add(
            "piano-roll-scroll-area",
            ElementKind::Region,
            "console/piano_roll.slint",
            "卷帘拖动手势面（按住并水平拖动 ⇒ 报告增量）",
            false,
        );
        for (note_index, ulid) in view.note_ulids.iter().enumerate() {
            registry.add(
                &format!("note-{ulid}-rect"),
                ElementKind::ListItem,
                "console/piano_roll.slint",
                &format!("音符 {note_index}"),
                false,
            );
        }
        registry.add(
            "note-suggestion-overlay-rect",
            ElementKind::ListItem,
            "console/piano_roll.slint",
            "AI 建议音符 (待采纳)",
            false,
        );
        registry.add(
            "piano-roll-velocity-lane",
            ElementKind::Region,
            "console/piano_roll.slint",
            "力度泳道",
            false,
        );
        for velocity_index in 0..view.note_ulids.len() {
            registry.add(
                &format!("velocity-{velocity_index}-bar"),
                ElementKind::Slider,
                "console/piano_roll.slint",
                &format!("力度条 {velocity_index}"),
                false,
            );
        }
        registry.add(
            "piano-roll-playhead",
            ElementKind::Slider,
            "console/piano_roll.slint",
            "卷帘走带光标",
            true,
        );

        // ------------------------------------------------------------ 调音台
        registry.add(
            "mixer-console",
            ElementKind::Region,
            "console/mixer_console.slint",
            "多轨调音台总控",
            false,
        );
        for (track_index, track) in view.tracks.iter().enumerate() {
            registry.add(
                &format!("track-{track_index}-channel-strip"),
                ElementKind::GroupBox,
                "console/mixer_console.slint",
                &format!("通道条 {}", track.name),
                false,
            );
            registry.add(
                &format!("track-{track_index}-meter"),
                ElementKind::ProgressIndicator,
                "console/mixer_console.slint",
                &format!("轨道 {} 电平表", track.name),
                true,
            );
            // [UI-TEST-001] 明文点名的约定族之一
            registry.add(
                &format!("track-{track_index}-fader"),
                ElementKind::Slider,
                "console/mixer_console.slint",
                &format!("轨道 {} 推子", track.name),
                false,
            );
            // 声相读出 / 拖动面（`mixer_console.slint` 的 `"PAN " + root.track-pans[i]`）。
            //
            // 为什么它是一个**独立**的语义 ID 而不是复用推子的：`pan` 到 2026-10-07 为止
            // 只活在标签文本里（`docs/ledger/feature-alignment.md` 的"声相读不到"），
            // 而"拖得动 + 读得回"要求它有 `accessible-value`。本切片起它有。
            //
            // ⚠ 与 `track-{i}-fader` **同族但不同名**：两个元素可能同时出现在树上，
            // 重名会让运行时控件树构建失败（`TreeError::DuplicateId`）。
            registry.add(
                &format!("track-{track_index}-pan"),
                ElementKind::Slider,
                "console/mixer_console.slint",
                &format!("轨道 {} 声相", track.name),
                false,
            );
            // 混音台的静音 / 独奏 / 色标用**自己**的语义 ID（`-mixer-` 中缀），
            // 不复用 arrangement 的 `track-{i}-mute-button`：两个视图可能同时可见，
            // 重复 ID 会让运行时控件树构建直接失败（`TreeError::DuplicateId`）。
            registry.add(
                &format!("track-{track_index}-mixer-mute-button"),
                ElementKind::Button,
                "console/mixer_console.slint",
                &format!("轨道 {} 静音 (调音台)", track.name),
                false,
            );
            registry.add(
                &format!("track-{track_index}-mixer-solo-button"),
                ElementKind::Button,
                "console/mixer_console.slint",
                &format!("轨道 {} 独奏 (调音台)", track.name),
                false,
            );
            registry.add(
                &format!("track-{track_index}-mixer-color-swatch"),
                ElementKind::Image,
                "console/mixer_console.slint",
                &format!("轨道色标 {} (调音台)", track.color_hex),
                false,
            );
            // ⭐ **SFZ 采样器绑定**（任务单 `#82`）：路径输入框 ＋ SET / CLR。
            //
            // 三个 ID 是**独立**的语义节点（不是把输入框与按钮揉成一格）：控制面要能
            // ① 读当前绑定（`-sampler-path` 的 `value` = 路径原文）、② 写一段文本
            // （`ElementHandle::set_accessible_value` 落到 `TextInput.text`）、
            // ③ 触发提交 / 解绑（两个按钮的 `accessible-action-default`）。
            // 混起来的后果是"读"与"写"落到同一个节点而看不出是哪一步失败。
            //
            // `-sampler-path` 的**路径原文**进 `accessible-value` 是自动绑定的
            // （Slint 对 `TextInput` 的 `accessible-value` 默认指向 `text`；
            // 上游 `i-slint-compiler-1.18.1/passes/lower_accessibility.rs`）——
            // 因此它不是"我们声称有一个值"，而是这个控件**本来**的可读值。
            registry.add(
                &format!("track-{track_index}-sampler-path"),
                ElementKind::TextInput,
                "console/mixer_console.slint",
                &format!(
                    "轨道 {} SFZ 采样器路径: {}",
                    track.name,
                    track.sampler_path.as_deref().unwrap_or("(未绑定)")
                ),
                false,
            );
            registry.add(
                &format!("track-{track_index}-sampler-set"),
                ElementKind::Button,
                "console/mixer_console.slint",
                &format!("轨道 {} 提交 SFZ 路径", track.name),
                false,
            );
            registry.add(
                &format!("track-{track_index}-sampler-clear"),
                ElementKind::Button,
                "console/mixer_console.slint",
                &format!("轨道 {} 解绑 SFZ 采样器", track.name),
                false,
            );
            // ⭐ **③ 浏览…**（任务单 `#86` 乙）：开一次**原生**文件对话框，把选中的路径
            // **填进** `-sampler-path` 那个输入框（⛔ 不提交 —— 提交仍是 `-sampler-set`）。
            //
            // ⚠ 它是一个**独立**的语义节点而不是给输入框加个后缀：控制面要能分开寻址
            // "开对话框"与"提交"，否则判据分不清"按钮没接"与"对话框没弹"。
            registry.add(
                &format!("track-{track_index}-sampler-browse"),
                ElementKind::Button,
                "console/mixer_console.slint",
                &format!("轨道 {} 浏览 SFZ 文件", track.name),
                false,
            );
        }
        registry.add(
            "mixer-master-strip",
            ElementKind::GroupBox,
            "console/mixer_console.slint",
            "主控通道条",
            false,
        );
        registry.add(
            "mixer-master-meter",
            ElementKind::ProgressIndicator,
            "console/mixer_console.slint",
            "主控电平表",
            true,
        );
        registry.add(
            "mixer-master-fader",
            ElementKind::Slider,
            "console/mixer_console.slint",
            "主控推子",
            false,
        );
        // 主控声相读出 / 拖动面（`mixer_console.slint` 的 `"PAN " + root.master-pan`）。
        //
        // 与 `track-{i}-pan` 同一个角色、同一个理由：`.slint` 的手势只报"哪一类面板 +
        // 本地坐标"，而"主控声相到底是多少"必须能从**控件树**读回来
        // （`ui/property {name:"value"}` 读 `accessible-value`）—— 否则"拖得动"这一半
        // 只能靠窗口属性证明，控制面看不见它。本切片起它有 `accessible-value`。
        registry.add(
            "mixer-master-pan",
            ElementKind::Slider,
            "console/mixer_console.slint",
            "主控声相",
            false,
        );
        registry.add(
            "mixer-master-mute-button",
            ElementKind::Button,
            "console/mixer_console.slint",
            "主控静音",
            false,
        );
        registry.add(
            "mixer-master-solo-button",
            ElementKind::Button,
            "console/mixer_console.slint",
            "主控独奏",
            false,
        );
        registry.add(
            "mixer-master-color-swatch",
            ElementKind::Image,
            "console/mixer_console.slint",
            "主控色标",
            false,
        );

        // ------------------------------------------------------------ 素材池（`#133`）
        // 面板 / 条目数 / 空态是**单例**；行是**投影驱动的动态族**
        // （`for asset_name[row_index] in root.asset-pool-names`）⇒ 条目数随
        // `clip_pool` 变化（本判据的"派生不是硬编码"那一半就在这份循环里）。
        registry.add(
            "asset-pool-panel",
            ElementKind::Region,
            "console/asset_pool_panel.slint",
            "素材池",
            false,
        );
        registry.add(
            "asset-pool-count",
            ElementKind::Text,
            "console/asset_pool_panel.slint",
            &format!("素材 {} 条", view.asset_pool.len()),
            false,
        );
        registry.add(
            "asset-pool-empty",
            ElementKind::Text,
            "console/asset_pool_panel.slint",
            "尚无素材",
            false,
        );
        registry.add(
            "asset-pool-import-button",
            ElementKind::Button,
            "console/asset_pool_panel.slint",
            "导入音频（用文件对话框挑一个 WAV 并作为片段导入）",
            false,
        );
        for (index, row) in view.asset_pool.iter().enumerate() {
            registry.add(
                &format!("asset-pool-row-{index}-row"),
                ElementKind::ListItem,
                "console/asset_pool_panel.slint",
                &row.label,
                false,
            );
        }

        // ------------------------------------------------------------ 设备机架
        //
        // ⭐ 任务单 `#157`：设备机架**不再是静态部件** —— 卡片数 / 标签 / 旁通位全部来自
        // `ViewState::devices`（`TrackV3::devices` 的投影），空链 ⇒ 一块卡片都不登记
        // （界面显示 `device-rack-empty` 的空态）。旧版这里遍历的 `scene::DEVICE_NAMES`
        // 是三条编造的数据，已删除。
        registry.add(
            "device-rack",
            ElementKind::Region,
            "console/device_rack.slint",
            "设备效果链",
            false,
        );
        registry.add(
            "device-rack-track-label",
            ElementKind::Text,
            "console/device_rack.slint",
            &view.device_track_label,
            false,
        );
        registry.add(
            "device-rack-count",
            ElementKind::Text,
            "console/device_rack.slint",
            &format!("{} 台设备", view.devices.len()),
            false,
        );
        registry.add(
            "device-rack-eq-curve",
            ElementKind::Image,
            "console/device_rack.slint",
            "4 段 EQ 频响曲线 (占位)",
            true,
        );
        registry.add(
            "device-eq-low-gain-panel",
            ElementKind::GroupBox,
            "console/device_rack.slint",
            "EQ 低架增益 (eq_low_gain)",
            false,
        );
        // 注册表登记的是 `.slint` **声明过**的元素（不是"此刻可见"的元素）：滑块节点与
        // 空态节点在 `.slint` 里是**无条件声明**的（只有 `visible` 随投影变），因此这里
        // 也**无条件**登记 —— 否则 `slint_accessible_ids_and_registry_cover_each_other`
        // 的方向 2（`.slint` → 注册表）会红。
        registry.add(
            "device-eq-low-gain-slider",
            ElementKind::Slider,
            "console/device_rack.slint",
            "EQ 低架增益",
            false,
        );
        let eq_value_label = if view.device_eq_available() {
            format!("{} dB", view.device_eq_gain_db())
        } else {
            "本轨没有可编辑的 eq_low_gain".to_owned()
        };
        registry.add(
            "device-eq-value",
            ElementKind::Text,
            "console/device_rack.slint",
            &eq_value_label,
            false,
        );
        registry.add(
            "device-rack-empty",
            ElementKind::Text,
            "console/device_rack.slint",
            "无设备（点上方「装 EQ」或「装混响」）",
            false,
        );
        // ⭐ 任务单 `#163`：设备增删 ＋ 建空片段的三个按钮与两句逐字原因。
        // 三者都是**无条件声明**的（只有 `enabled` / `visible` 随投影变）⇒ 这里也
        // 无条件登记（与 `send-create-button` / `track-create-*` 同一条纪律）。
        registry.add(
            "device-insert-actions",
            ElementKind::Region,
            "console/device_rack.slint",
            "设备与片段操作",
            false,
        );
        registry.add(
            "device-insert-eq-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "装 EQ",
            false,
        );
        registry.add(
            "device-insert-reverb-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "装混响",
            false,
        );
        registry.add(
            "clip-create-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "新建空的 MIDI 片段",
            false,
        );
        // 原因格在**可用**时文本是空串（`.slint` 里 `visible: false`）；注册表要求每条都有
        // 非空无障碍名 ⇒ 可用时登记一句"可用"的说明，⛔ 不登记空串（与 `send-create-reason` 同款）。
        let insert_reason_label = if view.device_insert_reason.is_empty() {
            "装设备可用（无不可用原因）".to_owned()
        } else {
            view.device_insert_reason.clone()
        };
        registry.add(
            "device-insert-reason",
            ElementKind::Text,
            "console/device_rack.slint",
            &insert_reason_label,
            false,
        );
        let clip_reason_label = if view.clip_create_reason.is_empty() {
            "建片段可用（无不可用原因）".to_owned()
        } else {
            view.clip_create_reason.clone()
        };
        registry.add(
            "clip-create-reason",
            ElementKind::Text,
            "console/device_rack.slint",
            &clip_reason_label,
            false,
        );
        // ⭐ 任务单 `#180`：**主总线**设备链 —— 设备架面板的第二段（装 / 删）。
        // 三个标量元素（标题 / 条目数 / 原因）与两个按钮是**无条件声明**的 ⇒ 这里无条件
        // 登记；卡片 / 识别读数 / 删除按钮**按条目**登记（与 `.slint` 的 `for … in
        // root.master-device-names` 同索引集）—— 0 台时一个都不登记。
        registry.add(
            "master-device-rack",
            ElementKind::Region,
            "console/device_rack.slint",
            "主总线设备链",
            false,
        );
        registry.add(
            "master-device-track-label",
            ElementKind::Text,
            "console/device_rack.slint",
            &view.master_device_label,
            false,
        );
        registry.add(
            "master-device-rack-count",
            ElementKind::Text,
            "console/device_rack.slint",
            &format!("{} 台设备", view.master_devices.len()),
            false,
        );
        // ⭐ 任务单 `#190`：装不下的主总线设备**如实报数**（段高 260、卡片 `72 + 46n`
        // ⇒ 第 5 张起越过段底被 `clip`）。与计数格互斥，只在 `设备数 > 容量` 时在场；
        // 注册表按"无条件声明"登记（与 `.slint` 的 `if` 同一条口径：运行时缺失不算红）。
        registry.add(
            "master-device-truncation-note",
            ElementKind::Text,
            "console/device_rack.slint",
            "主总线设备链截断报数（还有几台未显示）",
            false,
        );
        registry.add(
            "master-device-insert-eq-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "给主总线装 EQ",
            false,
        );
        registry.add(
            "master-device-insert-reverb-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "给主总线装混响",
            false,
        );
        let master_insert_reason_label = if view.master_device_insert_reason.is_empty() {
            "给主总线装设备可用（无不可用原因）".to_owned()
        } else {
            view.master_device_insert_reason.clone()
        };
        registry.add(
            "master-device-insert-reason",
            ElementKind::Text,
            "console/device_rack.slint",
            &master_insert_reason_label,
            false,
        );
        // 与第一段同一条口径：空态节点是**无条件声明**的（只有 `visible` 随投影变）⇒
        // 这里也无条件登记，否则 `.slint` → 注册表的方向会红。
        registry.add(
            "master-device-rack-empty",
            ElementKind::Text,
            "console/device_rack.slint",
            "无设备（点上方「装 EQ」或「装混响」）",
            false,
        );
        // ⭐ 任务单 `#181`：主总线设备的两个可编辑面 —— 那一格 EQ 面板 / 滑块 / 读数 /
        // 逐字原因都是**无条件声明**的（只有 `visible` 随投影变）⇒ 这里无条件登记。
        registry.add(
            "master-device-eq-panel",
            ElementKind::GroupBox,
            "console/device_rack.slint",
            "主总线 EQ 低架增益 (eq_low_gain)",
            false,
        );
        registry.add(
            "master-device-eq-slider",
            ElementKind::Slider,
            "console/device_rack.slint",
            "主总线 EQ 低架增益",
            false,
        );
        let master_eq_value_label = if view.master_device_eq_available() {
            format!("{} dB", view.master_device_eq_gain_db())
        } else {
            "主总线没有可编辑的 eq_low_gain".to_owned()
        };
        registry.add(
            "master-device-eq-value",
            ElementKind::Text,
            "console/device_rack.slint",
            &master_eq_value_label,
            false,
        );
        let master_eq_reason_label = if view.master_device_eq_reason.is_empty() {
            "主总线 EQ 可用（无不可用原因）".to_owned()
        } else {
            view.master_device_eq_reason.clone()
        };
        registry.add(
            "master-device-eq-reason",
            ElementKind::Text,
            "console/device_rack.slint",
            &master_eq_reason_label,
            false,
        );
        for (index, row) in view.master_devices.iter().enumerate() {
            registry.add(
                &format!("master-device-{index}-card"),
                ElementKind::ListItem,
                "console/device_rack.slint",
                &format!(
                    "主总线设备 {}{}",
                    row.name,
                    if row.bypassed {
                        "（已旁通）"
                    } else {
                        "（生效）"
                    }
                ),
                false,
            );
            // ⭐ 任务单 `#181`：每台一个旁通开关（与展示轨 `device-{i}-bypass-switch`
            // 同一副语义；`.slint` 里它只随 `master-device-names` 的条目数生成）。
            registry.add(
                &format!("master-device-{index}-bypass-switch"),
                ElementKind::Switch,
                "console/device_rack.slint",
                &format!("旁通主总线设备 {}", row.name),
                false,
            );
            // 识别读数在"没有"时 `.slint` 里是空串 + `visible: false`；注册表要求每条都有
            // 非空无障碍名 ⇒ 空串登记一句说明，⛔ 不登记空串（与 `send-create-reason` 同款）。
            let recognised_label = if row.recognised.is_empty() {
                "无已识别参数".to_owned()
            } else {
                row.recognised.clone()
            };
            registry.add(
                &format!("master-device-{index}-recognised"),
                ElementKind::Text,
                "console/device_rack.slint",
                &recognised_label,
                false,
            );
            let unrecognised_label = if row.unrecognised.is_empty() {
                "无未识别参数".to_owned()
            } else {
                row.unrecognised.clone()
            };
            registry.add(
                &format!("master-device-{index}-unrecognised"),
                ElementKind::Text,
                "console/device_rack.slint",
                &unrecognised_label,
                false,
            );
            registry.add(
                &format!("master-device-{index}-remove-button"),
                ElementKind::Button,
                "console/device_rack.slint",
                &format!("移除主总线设备 {}", row.name),
                false,
            );
        }
        // ⭐ 任务单 `#164`：一键装乐器 —— 一格当前乐器读数 ＋ 不可用时的逐字原因 ＋
        // 每个内置乐器一个按钮（按钮名来自投影的 `instrument_names`，与 `.slint` 的
        // `for … in root.instrument-names` 同索引集）。
        registry.add(
            "instrument-label",
            ElementKind::Text,
            "console/device_rack.slint",
            &view.instrument_label,
            false,
        );
        let instrument_reason_label = if view.instrument_install_reason.is_empty() {
            "装乐器可用（无不可用原因）".to_owned()
        } else {
            view.instrument_install_reason.clone()
        };
        registry.add(
            "instrument-install-reason",
            ElementKind::Text,
            "console/device_rack.slint",
            &instrument_reason_label,
            false,
        );
        // ⭐ 任务单 `#211`：点击之后的**运行期回执**（"检查中…" / "已装上" / "装不了：…"）。
        // ⚠ 它不是投影：`ViewState` 不携带它，初始态是空串 ⇒ 注册表给一句稳定的占位
        // （⛔ 不能是空字面量，见 `no_accessible_name_may_be_the_empty_literal`）；
        // 运行时树里的真实文本由控件树的 `label` 读。
        registry.add(
            "instrument-install-status",
            ElementKind::Text,
            "console/device_rack.slint",
            "装乐器的运行期回执（点击后出现；投影不携带）",
            false,
        );
        // ⭐ 任务单 `#196`：采样库的下载入口 ＋ 逐字提示（未安装 / 运行期回执）。
        registry.add(
            "sample-library-download-button",
            ElementKind::Button,
            "console/device_rack.slint",
            if view.sample_library_installed {
                "校验采样库"
            } else {
                "下载采样库"
            },
            false,
        );
        let sample_library_label = if view.sample_library_notice.is_empty() {
            "采样库已安装（无提示）".to_owned()
        } else {
            view.sample_library_notice.clone()
        };
        registry.add(
            "sample-library-status",
            ElementKind::Text,
            "console/device_rack.slint",
            &sample_library_label,
            false,
        );
        for (instrument_index, instrument_name) in view.instrument_names.iter().enumerate() {
            registry.add(
                &format!("instrument-install-{instrument_index}-button"),
                ElementKind::Button,
                "console/device_rack.slint",
                &format!("装乐器 {instrument_name}"),
                false,
            );
        }
        // ⭐ 任务单 `#210`：按钮行**分页**（每页 2 条）⇒ 两个翻页键。
        // ⚠ 它们的**在场**是界面运行态（当前页 > 0 / 还有下一页），投影只能给出"清单有
        // 20+ 条"这件事 ⇒ 注册表无条件登记（与 `.slint` 的模板一一对应）；运行时树里
        // 只有真的在场的那一两个节点。`tools-domain` 的 `ui/coverage` 允许注册表 ⊇ 运行时。
        registry.add(
            "instrument-page-prev-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "上一页乐器",
            false,
        );
        registry.add(
            "instrument-page-next-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "下一页乐器",
            false,
        );
        for row in &view.devices {
            let device_index = row.slot_index;
            registry.add(
                &format!("device-{device_index}-card"),
                ElementKind::ListItem,
                "console/device_rack.slint",
                &format!("设备 {}", row.label()),
                false,
            );
            registry.add(
                &format!("device-{device_index}-bypass-switch"),
                ElementKind::Switch,
                "console/device_rack.slint",
                &format!("旁通 {}", row.name),
                false,
            );
            // 两台参数行**无条件**登记（`.slint` 里每个设备都有这两个 `Text` 节点，
            // 只是 `visible` 绑在读数非空上）。读数本身仍然逐字来自投影。
            registry.add(
                &format!("device-{device_index}-recognised"),
                ElementKind::Text,
                "console/device_rack.slint",
                &format!("设备 {} 的已识别参数", row.name),
                false,
            );
            registry.add(
                &format!("device-{device_index}-unrecognised"),
                ElementKind::Text,
                "console/device_rack.slint",
                &format!("设备 {} 的未识别参数", row.name),
                false,
            );
            // ⭐ 任务单 `#163`：每台一个删除按钮（参数是**设备身份**，⛔ 不是行下标）。
            registry.add(
                &format!("device-{device_index}-remove-button"),
                ElementKind::Button,
                "console/device_rack.slint",
                &format!("移除设备 {}", row.name),
                false,
            );
        }
        registry.add(
            "device-plugin-host-placeholder-card",
            ElementKind::ListItem,
            "console/device_rack.slint",
            "商业插件宿主卡片 (v2.0.0, 未实现)",
            false,
        );

        // ------------------------------------------------------------ 发送条
        //
        // ⭐ 任务单 `#159`：发送条与设备链**同住** Tab 3 的同一面板。行数 / 目的地 /
        // 增益文本全部来自 `ViewState::sends`（`RoutingGraph` 的 `SendToAux` 投影），
        // 空 ⇒ 只登记 `send-empty` 的空态（界面逐字写"无发送"）。
        registry.add(
            "send-strip",
            ElementKind::Region,
            "console/device_rack.slint",
            "发送与轨路由",
            false,
        );
        registry.add(
            "send-count",
            ElementKind::Text,
            "console/device_rack.slint",
            &format!("{} 条发送", view.sends.len()),
            false,
        );
        registry.add(
            "send-empty",
            ElementKind::Text,
            "console/device_rack.slint",
            "无发送",
            false,
        );
        registry.add(
            "send-create-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "新建发送",
            false,
        );
        // `send-create-reason` 在**可用**时文本是空串（`.slint` 里 `visible: false`）；
        // 注册表要求每条都有非空无障碍名 ⇒ 可用时登记一句"可用"的说明，⛔ 不登记空串。
        let reason_label = if view.send_create_reason.is_empty() {
            "新建发送可用（无不可用原因）".to_owned()
        } else {
            view.send_create_reason.clone()
        };
        registry.add(
            "send-create-reason",
            ElementKind::Text,
            "console/device_rack.slint",
            &reason_label,
            false,
        );
        for (send_index, row) in view.sends.iter().enumerate() {
            registry.add(
                &format!("send-{send_index}-row"),
                ElementKind::ListItem,
                "console/device_rack.slint",
                &row.label(),
                false,
            );
            registry.add(
                &format!("send-{send_index}-gain-slider"),
                ElementKind::Slider,
                "console/device_rack.slint",
                &format!("发送 {} 增益", row.destination_name),
                false,
            );
            registry.add(
                &format!("send-{send_index}-gain-value"),
                ElementKind::Text,
                "console/device_rack.slint",
                &row.gain_display,
                false,
            );
            // ⭐ 任务单 `#160`：每一条发送一个删除按钮（参数是**路由边身份**）。
            registry.add(
                &format!("send-{send_index}-remove-button"),
                ElementKind::Button,
                "console/device_rack.slint",
                &format!("删除发送 → {}", row.destination_name),
                false,
            );
        }

        // ------------------------------------------------------------ 宏面板
        //
        // 入口按钮与设备链**同住** Tab 3 的标题行（无条件声明，只有 `visible` 随
        // 第一段宽度变）⇒ 这里**无条件**登记。行数 / 名字 / 位置全部来自
        // `ViewState::macros`（`TrackV3::macros` 的投影）；零宏时画空态。
        registry.add(
            "macro-open-button",
            ElementKind::Button,
            "console/device_rack.slint",
            &format!("打开宏面板（{} 个宏）", view.macros.len()),
            false,
        );
        registry.add(
            "macro-panel",
            ElementKind::Region,
            "dialogs/macro_panel.slint",
            "宏面板",
            false,
        );
        registry.add(
            "macro-panel-title",
            ElementKind::Text,
            "dialogs/macro_panel.slint",
            &format!("宏面板，{}", view.device_track_label),
            false,
        );
        registry.add(
            "macro-panel-close-button",
            ElementKind::Button,
            "dialogs/macro_panel.slint",
            "关闭宏面板",
            false,
        );
        registry.add(
            "macro-empty-state",
            ElementKind::Text,
            "dialogs/macro_panel.slint",
            "这条轨没有宏",
            false,
        );
        for (macro_index, row) in view.macros.iter().enumerate() {
            registry.add(
                &format!("macro-{macro_index}-row"),
                ElementKind::ListItem,
                "dialogs/macro_panel.slint",
                &row.label(),
                false,
            );
            registry.add(
                &format!("macro-{macro_index}-name"),
                ElementKind::Text,
                "dialogs/macro_panel.slint",
                &row.name,
                false,
            );
            registry.add(
                &format!("macro-{macro_index}-value"),
                ElementKind::Text,
                "dialogs/macro_panel.slint",
                &row.value_display,
                false,
            );
            registry.add(
                &format!("macro-{macro_index}-slider"),
                ElementKind::Slider,
                "dialogs/macro_panel.slint",
                &format!("{} 位置", row.name),
                false,
            );
        }

        // ------------------------------------------------------------ 轨 / 路由（⭐ `#160`）
        //
        // 与发送段**同一面板**（`send-strip` 的上半段）。建轨三个按钮 ＋ 删除展示轨；
        // 四者都是**无条件声明**的（只有 `enabled` 随投影变）⇒ 这里**无条件**登记，
        // 否则 `.slint` → 注册表的方向会红（与 `send-create-button` 同一条纪律）。
        // 主总线说明那一行兼作"没有主总线 ⇒ 不能建轨"的逐字原因。
        registry.add(
            "track-routing-panel",
            ElementKind::Region,
            "console/device_rack.slint",
            "轨与路由",
            false,
        );
        registry.add(
            "track-routing-count",
            ElementKind::Text,
            "console/device_rack.slint",
            &view.track_count_label,
            false,
        );
        registry.add(
            "master-bus-label",
            ElementKind::Text,
            "console/device_rack.slint",
            &view.master_bus_label,
            false,
        );
        registry.add(
            "track-create-midi-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "新建 MIDI 轨",
            false,
        );
        registry.add(
            "track-create-audio-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "新建音频轨",
            false,
        );
        registry.add(
            "track-create-aux-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "新建辅助返回轨",
            false,
        );
        registry.add(
            "track-delete-button",
            ElementKind::Button,
            "console/device_rack.slint",
            "删除展示轨",
            false,
        );
        // 与 `send-create-reason` 同款：可用时 `.slint` 里 `visible: false`、文本是空串，
        // 而注册表要求每条都有非空无障碍名 ⇒ 可用时登记一句说明，⛔ 不登记空串。
        let track_delete_reason_label = if view.track_delete_reason.is_empty() {
            "删除展示轨可用（无不可用原因）".to_owned()
        } else {
            view.track_delete_reason.clone()
        };
        registry.add(
            "track-delete-reason",
            ElementKind::Text,
            "console/device_rack.slint",
            &track_delete_reason_label,
            false,
        );

        // ------------------------------------------------------------ 对话框
        registry.add(
            "musical-pr-drawer",
            ElementKind::Region,
            "dialogs/musical_pr_drawer.slint",
            "AI 编曲提案审核抽屉",
            false,
        );
        registry.add(
            "musical-pr-close-button",
            ElementKind::Button,
            "dialogs/musical_pr_drawer.slint",
            "关闭提案抽屉",
            false,
        );
        registry.add(
            "musical-pr-diff-legend",
            ElementKind::Text,
            "dialogs/musical_pr_drawer.slint",
            "视觉差异图例: 新增 / 删除 / 微调",
            false,
        );
        // 空态（零提案时**唯一**出现在抽屉内容区的图元）。注册表的标签就是界面上那句
        // 可见文案 —— 注册表与图元说的是同一句话（`[UI-TEST-001]`）。
        registry.add(
            "musical-pr-empty-state",
            ElementKind::Text,
            "dialogs/musical_pr_drawer.slint",
            "还没有 AI 提案。这条链路尚未接线。",
            false,
        );
        for proposal_index in 0..3 {
            registry.add(
                &format!("musical-pr-proposal-{proposal_index}-item"),
                ElementKind::ListItem,
                "dialogs/musical_pr_drawer.slint",
                &format!("AI 提案 {proposal_index}"),
                false,
            );
        }
        registry.add(
            "musical-pr-accept-button",
            ElementKind::Button,
            "dialogs/musical_pr_drawer.slint",
            "采纳提案 (Shift+Enter)",
            false,
        );
        registry.add(
            "musical-pr-reject-button",
            ElementKind::Button,
            "dialogs/musical_pr_drawer.slint",
            "放弃提案 (Esc)",
            false,
        );
        registry.add(
            "undo-tree-modal",
            ElementKind::Region,
            "dialogs/undo_tree_modal.slint",
            "编曲版本时光机图谱",
            false,
        );
        registry.add(
            "undo-tree-panel",
            ElementKind::Region,
            "dialogs/undo_tree_modal.slint",
            "版本图谱面板",
            false,
        );
        for node_index in 0..6 {
            registry.add(
                &format!("undo-tree-node-{node_index}"),
                ElementKind::ListItem,
                "dialogs/undo_tree_modal.slint",
                &format!("版本节点 {node_index}"),
                false,
            );
        }
        // 2026-10-08（诚实性审计）：图谱节点现在是**真实提交链**的投影（宿主
        // `host::apply_undo` 从 `UndoPort::graph()` 写 `undo-node-labels` /
        // `-node-count` / `-node-hidden`）。链条为空时界面画**空态**、比画布长时画
        // **截断报数** —— 两个状态都各自有一个稳定的语义 ID（`[UI-TEST-001]`）。
        // 为什么注册的节点 ID 仍恰好是 `0..6`：画布只放得下 `host::UNDO_TREE_MAX_NODES`
        // （= 6）个节点，所以运行时**不可能**出现第 7 个 `undo-tree-node-*`
        // （`live_control_plane_coverage_matches_the_projected_registry` 的
        // `unknown_at_runtime` 因此仍然为空）。
        registry.add(
            "undo-tree-empty-state",
            ElementKind::Region,
            "dialogs/undo_tree_modal.slint",
            "还没有任何版本提交。撤销历史为空。",
            false,
        );
        registry.add(
            "undo-tree-truncation-note",
            ElementKind::Region,
            "dialogs/undo_tree_modal.slint",
            "更早的版本未画出（截断报数）",
            false,
        );
        registry.add(
            "undo-tree-close-button",
            ElementKind::Button,
            "dialogs/undo_tree_modal.slint",
            "关闭时光机",
            false,
        );
        // ADR-0001 D45: 弹窗里**真的执行撤销**的那个按钮。
        // 它的"能不能撤"由宿主从模型读数注入（`host::apply_undo`），因此它不是
        // 自成一体的演示部件 —— 但也不是 `track-*` 那类"每个实体一个 ID"的族。
        registry.add(
            "undo-tree-undo-button",
            ElementKind::Button,
            "dialogs/undo_tree_modal.slint",
            "撤销一步 (Cmd+Z)",
            false,
        );
        // 导出 WAV 选项框：默认关闭（`export-wav-open=false`），因此不进运行时控件树，
        // 也不改任何金帧。打开后宿主把 `module-export-wav-button` 的动作接到这里，
        // 真正的写出仍走 `MainWindow.export-wav` → `export_wav::export_wav_with_sfz_file`。
        // 四个输入框的文本**不在 UI 侧解析**：解析只发生在宿主 `parse_wav_export_options`，
        // 非法值回执 `error=invalid-options` 且不落盘（`[UI-TEST-001]`）。
        registry.add(
            "export-wav-dialog",
            ElementKind::Region,
            "dialogs/export_wav_dialog.slint",
            "导出 WAV 对话框",
            false,
        );
        registry.add(
            "export-wav-panel",
            ElementKind::Region,
            "dialogs/export_wav_dialog.slint",
            "导出 WAV 选项面板",
            false,
        );
        registry.add(
            "export-wav-close-button",
            ElementKind::Button,
            "dialogs/export_wav_dialog.slint",
            "关闭导出对话框",
            false,
        );
        registry.add(
            "export-wav-path-input",
            ElementKind::TextInput,
            "dialogs/export_wav_dialog.slint",
            "导出目标路径（留空用默认路径）",
            false,
        );
        registry.add(
            "export-wav-sample-rate-input",
            ElementKind::TextInput,
            "dialogs/export_wav_dialog.slint",
            "采样率（auto / 44100 / 48000 / 96000）",
            false,
        );
        registry.add(
            "export-wav-bit-depth-input",
            ElementKind::TextInput,
            "dialogs/export_wav_dialog.slint",
            "位深（16 / 24 / 32f）",
            false,
        );
        registry.add(
            "export-wav-seconds-input",
            ElementKind::TextInput,
            "dialogs/export_wav_dialog.slint",
            "导出时长（秒，留空用后端默认）",
            false,
        );
        registry.add(
            "export-wav-options-note",
            ElementKind::Text,
            "dialogs/export_wav_dialog.slint",
            "选项说明：留空或 auto 采用后端默认；非法值不落盘",
            false,
        );
        registry.add(
            "export-wav-run-button",
            ElementKind::Button,
            "dialogs/export_wav_dialog.slint",
            "按当前选项导出 WAV",
            false,
        );

        registry
    }

    /// 插入一条元数据; 返回被同一个 ID 顶掉的旧值 (正常情况下应为 `None`)。
    pub fn insert(&mut self, meta: ElementMeta) -> Option<ElementMeta> {
        self.entries.insert(meta.id.clone(), meta)
    }

    /// 便捷插入: 由字段直接构造。ID 重复时 panic —— 重复 ID 是编程错误, 不是运行时状况。
    ///
    /// # Panics
    ///
    /// 当 `id` 已存在时 panic。
    pub fn add(
        &mut self,
        id: &str,
        kind: ElementKind,
        component: &'static str,
        label: &str,
        dynamic_region: bool,
    ) {
        let previous = self.insert(ElementMeta {
            id: id.to_owned(),
            kind,
            component,
            label: label.to_owned(),
            dynamic_region,
        });
        assert!(previous.is_none(), "语义元素 ID 重复: {id}");
    }

    /// 按 ID 精确查找。
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ElementMeta> {
        self.entries.get(id)
    }

    /// 是否登记了该 ID。
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    /// 元素总数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 注册表是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 按 ID 升序遍历全部元素 (顺序稳定, 因为底层是 `BTreeMap`)。
    pub fn iter(&self) -> impl Iterator<Item = &ElementMeta> + '_ {
        self.entries.values()
    }

    /// 按 ID 升序返回全部语义 ID。
    pub fn ids(&self) -> impl Iterator<Item = &str> + '_ {
        self.entries.keys().map(String::as_str)
    }

    /// 返回 ID 以 `prefix` 开头的全部元素 (例如 `"tab-"` 取全部控制台标签)。
    pub fn with_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = &'a ElementMeta> + 'a {
        self.entries
            .values()
            .filter(move |meta| meta.id.starts_with(prefix))
    }

    /// `[UI-MCP-002]` 全部高频刷新区域 (截图比对前必须按包围盒置黑)。
    pub fn dynamic_regions(&self) -> impl Iterator<Item = &ElementMeta> + '_ {
        self.entries.values().filter(|meta| meta.dynamic_region)
    }

    /// 把注册表导出成稳定的行协议, 供 `--headless --dump-elements` 与 CI 断言消费。
    ///
    /// 格式 (每行一个元素, 字段用空格分隔, 顺序固定):
    /// `element <id> role=<accessible-role> kind=<ElementKind> component=<file> dynamic=<true|false> label=<label>`
    ///
    /// 用行协议而不是 JSON: 本 crate 没有 `serde_json` 依赖, 为了一个调试导出引入
    /// 序列化框架不划算。ID 与文件名保证不含空格, `label` 放在最后一段因此可以含空格。
    #[must_use]
    pub fn dump_lines(&self) -> Vec<String> {
        self.entries
            .values()
            .map(|meta| {
                format!(
                    "element {} role={} kind={:?} component={} dynamic={} label={}",
                    meta.id,
                    meta.kind.accessible_role(),
                    meta.kind,
                    meta.component,
                    meta.dynamic_region,
                    meta.label,
                )
            })
            .collect()
    }
}

/// 校验语义 ID 的**形状**: 由小写/大写字母与数字组成的分段, 用单个 `-` 连接。
///
/// 允许大写是因为 `note-{ulid}-rect` / `clip-{ulid}-header` 里的 ULID 是 Crockford Base32
/// (规范要求大小写不敏感, 我们统一写大写)。
///
/// 拒绝: 空串、前导/尾随 `-`、连续 `--`、空格、下划线、任何非 ASCII 字符。
/// 这条校验是"约定"能被机械检查的前提 —— 否则 `note_01J8…_rect` 这种手滑会静默通过。
#[must_use]
pub fn is_well_formed_id(id: &str) -> bool {
    if id.is_empty() || id.starts_with('-') || id.ends_with('-') || id.contains("--") {
        return false;
    }
    id.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// 已经由 [`crate::bridge`] 的投影驱动的语义 ID 族（前缀）。
///
/// 这份清单把"哪些部件的数据来自工程"从注释变成**可断言的事实**：
/// 判据只允许对这些族做"不得出现演示数据"的负向断言；其余部件
/// （侧栏资源库 / 两个对话框 / 状态栏 / 走带）的静态标签是**已知的未实现项**
/// （见 `docs/ledger/app-binding-notes.md`）—— 对它们做全局断言会假红，
/// 例如侧栏里有 `Sub Bass 低频`。
///
/// ⚠ **设备效果链（任务单 `#157`）为什么不在本表**：它的卡片已经由
/// `ViewState::devices` 驱动，但 `device-` 这个前缀还覆盖 `device-rack`（区域本身）与
/// `device-rack-eq-curve`（占位图）—— 把整个前缀判成"工程驱动"会把两件非数据部件也
/// 卷进负向断言。因此设备卡的"数据来自工程"由 `tests/device_rack_projection.rs` 直接钉住
/// （条目数 / 名字 / 旁通位 / 参数读数四项），而不是加一条过宽的前缀。
///
/// `mixer-` 是 app-mixer 工作线新加的一族：调音台的**通道条**（`track-{i}-*`）本来就在
/// `track-` 族里，但**主控通道条**（`mixer-master-*`）不共享那个前缀 —— 它的名字 / 音量 /
/// 声相 / 静音 / 独奏 / 色标同样来自工程，所以必须有自己的族，否则"主控显示的是不是工程"
/// 就没有机械检查（`ui/console/mixer_console.slint` 的 `mixer-master-strip` 子树）。
pub const MODEL_DRIVEN_FAMILIES: [&str; 10] = [
    "track-",
    "section-",
    "clip-",
    "note-",
    "velocity-",
    "session-track-",
    "scene-",
    "scene-launch-",
    "slot-",
    "mixer-",
];

/// 该语义 ID 是否属于 [`MODEL_DRIVEN_FAMILIES`]（即"应当携带工程数据"的部件）。
#[must_use]
pub fn is_model_driven_family(id: &str) -> bool {
    MODEL_DRIVEN_FAMILIES
        .iter()
        .any(|prefix| id.starts_with(prefix))
}

/// 从 `ui/dialogs/musical_pr_drawer.slint` **删掉的**全部假提案字面量（文案 + 置信度）。
///
/// 逐字抄自本票改动前的 HEAD（`git show <parent>:crates/yeban-app/ui/dialogs/
/// musical_pr_drawer.slint` 的 `:18-25`），当时它们是四条属性的**默认值** ——
/// 也就是说没有任何宿主写它们（`no_host_writes_the_musical_pr_proposal_properties`
/// 证明这一点），**用户看到的就是这些编造的数据**。
///
/// 为什么放在 lib 里而不是测试模块里: 两个测试目标（本文件的单元测试与
/// `tests/live_ui_mcp.rs` 的集成测试）要探**同一批**字符串。写第二份就会漂移 ——
/// 补一条字面量却漏改另一份, 判据就有一半永远绿。
#[doc(hidden)]
pub const DEMO_PROPOSAL_LITERALS: [&str; 6] = [
    "Verse 第 3 小节加入上行贝斯线",
    "Chorus 第 2 拍删除重叠和弦音",
    "Drop 段微调铺底音高 -2 半音",
    "94%",
    "88%",
    "76%",
];

/// 从 `ui/dialogs/undo_tree_modal.slint` **删掉的**全部假历史节点名（逐字）。
///
/// 逐字抄自本次改动前的 HEAD（`git show <parent>:crates/yeban-app/ui/dialogs/
/// undo_tree_modal.slint` 的 `:30`），当时它们是 `node-labels` 属性的**默认值** ——
/// 而**没有任何宿主写者**（`git grep -n 'node-labels'` 只命中语义注册表与测试）
/// ⇒ 用户打开时光机看到的就是这六个不存在的版本，其中最后一个还声称有一条
/// "AI 提案"版本。
///
/// 为什么放在 lib 里而不是测试模块里：两个测试目标（`tests/undo_tree_honesty.rs`
/// 的文本层探针与 `tests/live_ui_mcp.rs` 的运行时树探针）要探**同一批**字符串。
/// 写第二份就会漂移。
#[doc(hidden)]
pub const DEMO_UNDO_TREE_LITERALS: [&str; 6] = [
    "c0 初始",
    "c1 鼓组",
    "c2 贝斯",
    "c3 铺底",
    "c4 主音",
    "c5 AI 提案",
];

/// 抽屉的四个数据面属性：`.slint` 名 / Slint 生成的 Rust setter 名（snake_case）配对。
///
/// 判据要同时看两侧的**名称风格** —— 只查一侧会让"换个名字重新注入"漏过去。
#[doc(hidden)]
pub const MUSICAL_PR_PROPERTY_NAMES: [(&str, &str); 4] = [
    ("proposal-count", "proposal_count"),
    ("proposal-labels", "proposal_labels"),
    ("proposal-kinds", "proposal_kinds"),
    ("confidences", "confidences"),
];

#[cfg(test)]
mod tests {
    use super::*;
    // 这些常量现在只被**判据**用到（事实源是 `bridge::demo_project` 的投影），
    // 因此 import 放在 tests 里 —— 放进模块级会撞上 lib 构建的 `unused_imports`。
    use crate::scene::{CLIP_COUNT, CLIP_ULIDS, NOTE_COUNT, NOTE_ULIDS, TRACK_COUNT};

    fn registry() -> ElementRegistry {
        ElementRegistry::demo()
    }

    /// 判据 3 (SKILL 规则 2): **`tab-*-button` 前缀必须覆盖每一个控制台标签**。
    ///
    /// 曾经在故意把 `ui/console/console_tabs.slint` 的 `accessible-id` 前缀改成
    /// `tabx-` 之后变红 (`left: ["piano-roll", "mixer", "devices"]` vs `right: []`)。
    #[test]
    fn console_tab_buttons_cover_every_declared_tab() {
        let registry = registry();
        let mut declared: Vec<&str> = CONSOLE_TABS.iter().map(|tab| tab.name).collect();
        declared.sort_unstable();

        let mut found: Vec<&str> = registry
            .with_prefix("tab-")
            .filter_map(|meta| {
                meta.id
                    .strip_prefix("tab-")
                    .and_then(|rest| rest.strip_suffix("-button"))
            })
            .collect();
        found.sort_unstable();

        assert_eq!(
            found, declared,
            "`tab-{{name}}-button` 必须与 scene::CONSOLE_TABS 一一对应; \
             新增控制台标签时两处都要改"
        );
        assert!(
            !found.is_empty(),
            "`tab-` 前缀一个元素都没匹配到 —— 前缀约定被破坏了"
        );
    }

    /// 判据: 每条轨道的推子 / 包头 / 静音 / 独奏 / 通道条 / 电平表都必须存在。
    #[test]
    fn per_track_semantic_families_are_complete() {
        let registry = registry();
        for track_index in 0..TRACK_COUNT {
            for suffix in [
                "fader",
                "header",
                "mute-button",
                "solo-button",
                "channel-strip",
                "meter",
                "color-swatch",
            ] {
                let id = format!("track-{track_index}-{suffix}");
                assert!(registry.contains(&id), "缺少语义元素 `{id}` [UI-TEST-001]");
            }
        }
        assert_eq!(
            registry
                .with_prefix("track-")
                .filter(|m| m.id.ends_with("-fader"))
                .count(),
            TRACK_COUNT
        );
    }

    /// `note-{ulid}-rect` / `clip-{ulid}-header` 必须逐个对得上演示 ULID 常量,
    /// 而且 ULID 段本身必须是合法的 Crockford Base32。
    #[test]
    fn ulid_families_match_the_scene_constants() {
        let registry = registry();
        for ulid in NOTE_ULIDS {
            let id = format!("note-{ulid}-rect");
            assert!(registry.contains(&id), "缺少 `{id}`");
            assert!(crate::scene::is_ulid_text(ulid));
        }
        for ulid in CLIP_ULIDS {
            let id = format!("clip-{ulid}-header");
            assert!(registry.contains(&id), "缺少 `{id}`");
            assert!(crate::scene::is_ulid_text(ulid));
        }
        // 结构化计数: `note-{26 字符 ULID}-rect` —— 顺带钉死 `{ulid}` 段确实在中间。
        let note_rects = registry
            .iter()
            .filter(|meta| meta.id.starts_with("note-") && meta.id.ends_with("-rect"))
            .filter(|meta| {
                crate::scene::is_ulid_text(&meta.id["note-".len()..meta.id.len() - "-rect".len()])
            })
            .count();
        assert_eq!(
            note_rects, NOTE_COUNT,
            "`note-{{ulid}}-rect` 的成员数必须等于演示音符数"
        );

        let clip_headers = registry
            .iter()
            .filter(|meta| meta.id.starts_with("clip-") && meta.id.ends_with("-header"))
            .filter(|meta| {
                crate::scene::is_ulid_text(&meta.id["clip-".len()..meta.id.len() - "-header".len()])
            })
            .count();
        assert_eq!(
            clip_headers, CLIP_COUNT,
            "`clip-{{ulid}}-header` 的成员数必须等于演示剪辑数"
        );
    }

    /// 全部 ID 形状合法 (没有下划线 / 空格 / 连续连字符), 且**不含绝对像素坐标风格**的名字。
    #[test]
    fn every_id_is_well_formed() {
        let registry = registry();
        for id in registry.ids() {
            assert!(is_well_formed_id(id), "语义 ID `{id}` 形状非法");
        }
        assert!(!registry.is_empty());
    }

    #[test]
    fn id_shape_checker_rejects_the_usual_mistakes() {
        assert!(is_well_formed_id("track-0-fader"));
        assert!(is_well_formed_id("note-01J8Z5Q0R7K3M9X2V4B6N8P1A2-rect"));
        assert!(is_well_formed_id("tab-piano-roll-button"));
        assert!(!is_well_formed_id(""));
        assert!(!is_well_formed_id("-track-0"));
        assert!(!is_well_formed_id("track-0-"));
        assert!(!is_well_formed_id("track--0"));
        assert!(!is_well_formed_id("track_0"));
        assert!(!is_well_formed_id("track 0"));
        assert!(!is_well_formed_id("轨道-0"));
    }

    /// 红线 4 的精神: 迭代顺序必须稳定且排序 (BTreeMap 而非 HashMap)。
    ///
    /// 曾经在把 `entries` 换成 `HashMap` 之后变红 —— 顺序断言会随机失败。
    #[test]
    fn iteration_order_is_sorted_and_stable() {
        let table = registry();
        let ids: Vec<&str> = table.ids().collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "注册表迭代顺序必须是 ID 升序 (BTreeMap)");

        // 连跑两次拿到的是同一份顺序
        let second = registry();
        let again: Vec<&str> = second.ids().collect();
        assert_eq!(ids, again);
    }

    #[test]
    fn duplicate_ids_are_rejected_loudly() {
        let mut registry = ElementRegistry::default();
        registry.add("a-1", ElementKind::Button, "x.slint", "甲", false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            registry.add("a-1", ElementKind::Button, "x.slint", "乙", false);
        }));
        assert!(result.is_err(), "重复语义 ID 必须 panic, 不能静默覆盖");
    }

    /// `[UI-MCP-002]`: 走带光标 / 时间码 / VU 电平表必须被登记为动态区域, 否则无头截图
    /// 比对会因每帧抖动产生 100% 假阳性。
    #[test]
    fn dynamic_regions_cover_every_high_frequency_widget() {
        let registry = registry();
        let dynamic: Vec<&str> = registry
            .dynamic_regions()
            .map(|meta| meta.id.as_str())
            .collect();
        for required in [
            "transport-timecode",
            "arrangement-playhead",
            "piano-roll-playhead",
            "mixer-master-meter",
        ] {
            assert!(
                dynamic.contains(&required),
                "`{required}` 必须登记为动态遮罩区域"
            );
        }
        for track_index in 0..TRACK_COUNT {
            let id = format!("track-{track_index}-meter");
            let meta = registry.get(&id).unwrap_or_else(|| panic!("缺少 `{id}`"));
            assert!(meta.dynamic_region, "`{id}` 必须登记为动态遮罩区域");
        }
    }

    /// 每个元素的 `component` 字段都指向一个**真实存在**的 `.slint` 文件。
    #[test]
    fn component_paths_point_at_real_slint_files() {
        let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        for meta in registry().iter() {
            let path = ui_dir.join(meta.component);
            assert!(
                path.is_file(),
                "`{}` 指向的 `{}` 不存在",
                meta.id,
                path.display()
            );
        }
    }

    // =====================================================================
    // 与 `ui/` 的**双向**契约: 这是本地唯一能做的 "UI 变更双重验证" 前置检查。
    // 真正的无头控件树断言 + 截图比对在 CI 上由 yeban-ui-test-port 完成。
    // =====================================================================

    /// ADR-0001 D2 的 UI 文件清单。多一个/少一个都必须在这里显式改, 否则测试变红 ——
    /// 这样"UI/UX §8 的 11 个文件"就不是一句注释, 而是一条会被机械检查的约定。
    ///
    /// `module_actions.slint`（14 号）是**四个模块入口**那一票加的：导出 WAV / 导入 MIDI /
    /// 装载 SFZ / 录音。它是**新增的独立组件**（不是往 `transport.slint` 里塞）——
    /// 理由是顶栏那三个文档键与录音键的"无点击处理器"被本文件自己的两条判据钉着
    /// （承载 `[ARCH-REC-001]` 的声卡录音缺口），本票不动它们。
    ///
    /// `console/asset_pool_panel.slint`（15 号）是**任务单 `#133`** 加的素材池面板：
    /// 独立组件（住在 `MixerConsole` 里，Tab 1 才可见）⇒ 默认帧像素不变。它与上面
    /// 那一族同一条纪律：UI 加一个文件就必须在清单里显式改。
    const SLINT_MANIFEST: [&str; 17] = [
        "app.slint",
        "transport.slint",
        "status_bar.slint",
        "sidebar.slint",
        // 主题层: 规范缺口, 由本工作线登记 (不在 ADR-0001 D2 的 11 个之内)
        "tokens.slint",
        "workspace/session_view.slint",
        "workspace/arrangement_view.slint",
        "console/console_tabs.slint",
        "console/piano_roll.slint",
        "console/mixer_console.slint",
        "console/device_rack.slint",
        // 素材池面板（任务单 `#133`）。
        "console/asset_pool_panel.slint",
        "dialogs/musical_pr_drawer.slint",
        "dialogs/undo_tree_modal.slint",
        // 导出 WAV 选项框（`[UI-NOTE-004]`）：隐藏模态，默认关闭 ⇒ 金帧不变。
        "dialogs/export_wav_dialog.slint",
        // 宏面板（`[UI-NOTE-004]` 的宏轨那一格）：右侧抽屉，默认关闭 ⇒ 金帧不变。
        "dialogs/macro_panel.slint",
        // 四个模块入口（导出 WAV / 导入 MIDI / 装载 SFZ / 录音）。
        "module_actions.slint",
    ];

    fn ui_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui")
    }

    fn collect_slint_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_slint_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "slint") {
                out.push(path);
            }
        }
    }

    /// `ui/` 下的 `.slint` 文件集合必须**恰好**等于 ADR-0001 D2 的清单。
    #[test]
    fn slint_manifest_matches_adr_0001_d2() {
        let ui = ui_dir();
        let mut found = Vec::new();
        collect_slint_files(&ui, &mut found);
        let mut found: Vec<String> = found
            .iter()
            .map(|path| {
                path.strip_prefix(&ui)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        found.sort();
        let mut expected: Vec<String> = SLINT_MANIFEST.iter().map(|s| (*s).to_owned()).collect();
        expected.sort();
        assert_eq!(found, expected, "`ui/` 的 .slint 清单与 ADR-0001 D2 不一致");
    }

    /// 把一行 `accessible-id:` 的右值拆成**字面段**。
    ///
    /// `.slint` 里语义 ID 有三种写法: 纯字面量 (`"status-bar"`), 以及
    /// `"track-" + i + "-fader"` 这样的模板。模板无法在文本层展开成具体 ID, 但它的
    /// **字面段序列**是确定的 —— 契约判据就建立在字面段上。
    fn id_pattern(line: &str) -> Option<(Vec<String>, bool)> {
        let rhs = line.split_once("accessible-id:")?.1;
        let rhs = rhs.split(';').next().unwrap_or(rhs);
        let segments: Vec<String> = rhs
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        if segments.is_empty() {
            return None;
        }
        // `+` 说明右值里除了字面量还有动态片段 (索引 / 模型元素 / 属性)。
        Some((segments, rhs.contains('+')))
    }

    /// 一个具体语义 ID 是否匹配某个 `accessible-id` 右值。
    ///
    /// 三种形态:
    /// - 纯字面量: 必须完全相等;
    /// - 单段模板 `"sidebar-item-" + i`: 必须是该前缀且后面还有东西;
    /// - 多段模板 `"track-" + i + "-fader"`: 首段前缀 + 末段后缀 + 中间段按序出现。
    fn id_matches_pattern(segments: &[String], is_template: bool, id: &str) -> bool {
        let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
            return false;
        };
        if !is_template {
            return segments.len() == 1 && id == first;
        }
        if !id.starts_with(first.as_str()) {
            return false;
        }
        if segments.len() == 1 {
            return id.len() > first.len();
        }
        if !id.ends_with(last.as_str()) {
            return false;
        }
        let mut cursor = first.len();
        for segment in &segments[1..] {
            match id[cursor..].find(segment.as_str()) {
                Some(offset) => cursor += offset + segment.len(),
                None => return false,
            }
        }
        true
    }

    /// 判据: `ui/**.slint` 与 [`ElementRegistry`] 必须**互相覆盖**。
    ///
    /// - 每个注册表 ID 都要能被某个 `.slint` 模板/字面量匹配上 (注册表不能凭空发明元素);
    /// - 每个 `.slint` 里的 `accessible-id:` 模板/字面量都要有注册表成员 (UI 加了 ID 却
    ///   忘了登记, 以后的自动化测试就会找不到它)。
    ///
    /// 这条判据把"UI 变更必须带稳定语义 ID"从口号变成机械检查 —— `[UI-TEST-001]`。
    #[test]
    fn slint_accessible_ids_and_registry_cover_each_other() {
        let ui = ui_dir();
        let mut files = Vec::new();
        collect_slint_files(&ui, &mut files);
        files.sort();

        let mut patterns: Vec<(String, Vec<String>, bool)> = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file).expect("读取 .slint 失败");
            let rel = file
                .strip_prefix(&ui)
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            for line in text.lines() {
                if let Some((segments, is_template)) = id_pattern(line) {
                    patterns.push((rel.clone(), segments, is_template));
                }
            }
        }
        assert!(
            !patterns.is_empty(),
            "`ui/` 里一个 accessible-id 都没有 [UI-TEST-001]"
        );
        // ⚠ 例外清单不许留"已经不存在的模板"：否则它会静默放行未来同名的空模板。
        for (first, last) in EMPTY_DYNAMIC_TEMPLATES_ALLOWED {
            assert!(
                patterns
                    .iter()
                    .any(|(_, segments, is_template)| *is_template
                        && segments.len() == 2
                        && segments[0] == *first
                        && segments[1] == *last),
                "例外清单里的模板 `{first}<i>{last}` 在 ui/ 里不存在"
            );
        }

        let mut with_master_device = yeban_model::samples::filled_project();
        let master_id = with_master_device.master_bus_track_id;
        let master_device = crate::bridge::builtin_effect_device(crate::bridge::BuiltinEffect::Eq)
            .expect("契约里必须有 EQ 分组");
        with_master_device
            .tracks
            .get_mut(&master_id)
            .expect("filled_project 必须有主总线轨")
            .devices
            .push(master_device);

        let registries = [
            registry(),
            // ⭐ 任务单 `#159`：`demo_project` 里**没有**发送（也没有 `AuxReturn` 轨），
            // 而 `filled_project` 有一条真实的 `SendToAux` 边 ⇒ 发送行的动态模板
            // （`send-{i}-row` / `-gain-slider` / `-gain-value`）只会在后者实例化。
            // 两个注册表都走**同一个** `from_view`，⛔ 不是第二份常量。
            ElementRegistry::from_view(
                &ViewState::from_project(&yeban_model::samples::filled_project())
                    .expect("filled_project 必须能投影"),
            ),
            // ⭐ 任务单 `#180`：`filled_project` 的主总线**没有设备** ⇒ 主总线卡片 /
            // 识别读数 / 删除按钮的动态模板（`master-device-{i}-*`）在上一个注册表里
            // 一个成员都没有。这里给它挂一台真的 EQ（走 `builtin_effect_device`，⛔ 不手写
            // 参数表）⇒ 同一套 `from_view` 就能实例化那四个模板。
            ElementRegistry::from_view(
                &ViewState::from_project(&with_master_device)
                    .expect("挂了主总线设备的 filled_project 必须能投影"),
            ),
        ];

        // 方向 1: 注册表 → .slint
        for registry in &registries {
            for meta in registry.iter() {
                assert!(
                    patterns.iter().any(|(_, segments, is_template)| {
                        id_matches_pattern(segments, *is_template, &meta.id)
                    }),
                    "注册表里的 `{}` 在 ui/ 的任何 .slint 里都找不到对应的 accessible-id 模板",
                    meta.id
                );
            }
        }

        // 方向 2: .slint → 注册表
        for (file, segments, is_template) in &patterns {
            if *is_template && empty_dynamic_template_is_documented(segments) {
                continue;
            }
            assert!(
                registries
                    .iter()
                    .any(|registry| registry.iter().any(|meta| id_matches_pattern(
                        segments,
                        *is_template,
                        &meta.id
                    ))),
                "{file} 里的 accessible-id 模板 {segments:?} (template={is_template}) 在注册表里一个成员都没有"
            );
        }
    }

    /// `accessible-id` 模板允许**今天 0 个成员**的**逐条**例外（⛔ 不是静默放行）。
    ///
    /// ⭐ 任务单 `#165`：`instrument-install-<i>-button` 的成员来自**按样本字节过滤后的**
    /// 内置乐器清单。仓内 `assets/samples/**` 有 1398 份 SFZ / **0 个样本字节** ⇒ 可装清单
    /// 为空 ⇒ 控件树里一个按钮都没有。模板仍在 `.slint`（`for … in root.instrument-names`），
    /// 样本入库后同一模板会实例化 —— 这是"**今天确实没有实例**"，⛔ 不是"UI 加了 ID
    /// 却忘了登记"。
    ///
    /// ⚠ 例外必须**逐条**写理由，并且 `.slint` 里必须真有这个模板
    /// （同一判据的上半段机械检查）。
    const EMPTY_DYNAMIC_TEMPLATES_ALLOWED: &[(&str, &str)] = &[(
        "instrument-install-",
        "-button", // #165：清单按样本字节过滤，今天为空 ⇒ 0 个按钮
    )];

    /// 这个模板是不是在 [`EMPTY_DYNAMIC_TEMPLATES_ALLOWED`] 里登记过。
    fn empty_dynamic_template_is_documented(segments: &[String]) -> bool {
        EMPTY_DYNAMIC_TEMPLATES_ALLOWED.iter().any(|(first, last)| {
            segments.len() == 2 && segments[0] == *first && segments[1] == *last
        })
    }

    /// 判据: 注册表由**投影**驱动 —— 换一个工程, 三族结构随之改变, 且标签携带工程数据。
    ///
    /// 这条判据是"界面不再渲染演示数据"在**纯 Rust 侧**的对应物：它不需要 Slint,
    /// 因此本机就能跑（Tier-1 的像素版判据在 `test_port_adapter.rs`，只有 CI 能跑）。
    #[test]
    fn registry_follows_the_projected_project() {
        let filled_project = yeban_model::samples::filled_project();
        let filled_view = ViewState::from_project(&filled_project).expect("投影");
        let demo = ElementRegistry::from_view(&ViewState::demo());
        let filled = ElementRegistry::from_view(&filled_view);

        assert!(
            demo.contains("track-5-header"),
            "演示工程有 6 条非主总线轨道"
        );
        assert!(
            !filled.contains("track-3-header"),
            "filled_project 只有 3 条非主总线轨道 —— 第 4 条不该存在"
        );
        assert!(filled.contains("track-0-header"));
        assert_ne!(
            demo.ids().collect::<Vec<_>>(),
            filled.ids().collect::<Vec<_>>(),
            "两个工程的语义 ID 集合必须不同（否则注册表没有跟着投影走）"
        );

        let lead = filled.get("track-0-header").expect("track-0-header");
        assert!(
            lead.label.contains("Lead"),
            "标签必须来自工程: {}",
            lead.label
        );
        for meta in filled.iter() {
            assert!(
                !meta.label.contains("鼓"),
                "`{}` 的标签里出现了演示夹具的轨道名 `鼓`: {}",
                meta.id,
                meta.label
            );
        }

        // 剪辑 / 段落 / 音符三族同样来自工程身份。
        let placement = filled_view.clips[0].placement_id.clone();
        assert!(filled.contains(&format!("clip-{placement}-header")));
        let section = filled_view.sections[0].id.clone();
        assert!(!section.is_empty(), "段落身份必须非空");
        assert!(filled.contains("section-0-card"));
        assert_eq!(filled_view.sections.len(), 2);
        for ulid in &filled_view.note_ulids {
            assert!(filled.contains(&format!("note-{ulid}-rect")));
        }
        for ulid in NOTE_ULIDS {
            assert!(
                !filled.contains(&format!("note-{ulid}-rect")),
                "演示音符 `{ulid}` 不该出现在 filled_project 的注册表里"
            );
        }
    }

    /// 判据: **轨道色标在投影与控件树两侧一致**（app-completion ②）。
    ///
    /// 三条断言：
    /// 1. 每条非主总线轨道都有 `track-{i}-color-swatch`，角色是 `image`；
    /// 2. 它的标签里带**规范化**的 `#RRGGBB`，且与 `ViewState::track_color_labels()` 逐项相等；
    /// 3. 缺色 / 非法色的轨道用的是回退色（不是空串、不是演示色）。
    #[test]
    fn track_color_swatches_carry_the_projected_hex() {
        let filled =
            ViewState::from_project(&yeban_model::samples::filled_project()).expect("投影");
        let registry = ElementRegistry::from_view(&filled);
        assert_eq!(
            registry
                .with_prefix("track-")
                // 只看 **arrangement** 的色标：app-mixer 之后调音台也有一份
                // `track-{i}-mixer-color-swatch`（同一个投影字段，不同的语义 ID —— 两个
                // 视图可能同时可见，重复 ID 会让运行时控件树构建失败）。不收窄计数会翻倍。
                .filter(|meta| meta.id.ends_with("-color-swatch"))
                .filter(|meta| !meta.id.contains("-mixer-"))
                .count(),
            filled.tracks.len(),
            "色标元素数必须等于轨道数"
        );
        // 调音台色标与 arrangement 色标**逐个对齐**（同一个 `TrackView::color_hex`）。
        for (index, label) in filled.track_color_labels().iter().enumerate() {
            let mixer = registry
                .get(&format!("track-{index}-mixer-color-swatch"))
                .unwrap_or_else(|| panic!("缺少 track-{index}-mixer-color-swatch"));
            assert_eq!(mixer.kind, ElementKind::Image, "调音台色标角色必须是 image");
            assert_eq!(mixer.component, "console/mixer_console.slint");
            assert!(
                mixer.label.contains(label),
                "调音台色标标签必须携带同一个投影色标: {} vs {label}",
                mixer.label
            );
        }
        let labels = filled.track_color_labels();
        for (index, label) in labels.iter().enumerate() {
            let meta = registry
                .get(&format!("track-{index}-color-swatch"))
                .unwrap_or_else(|| panic!("缺少 track-{index}-color-swatch"));
            assert_eq!(meta.kind, ElementKind::Image, "色标角色必须是 image");
            assert_eq!(meta.component, "workspace/arrangement_view.slint");
            assert!(
                meta.label.contains(label),
                "色标标签必须携带投影的规范化色标: {} vs {label}",
                meta.label
            );
            assert!(
                label.starts_with('#') && label.len() == 7,
                "色标形态: {label}"
            );
        }
        // filled_project 的 Lead 轨是 `#FF8800`；演示工程的空色轨必须回退。
        assert_eq!(labels[0], "#FF8800");
        let demo = ElementRegistry::demo();
        let demo_view = ViewState::demo();
        for (index, track) in demo_view.tracks.iter().enumerate() {
            let meta = demo
                .get(&format!("track-{index}-color-swatch"))
                .unwrap_or_else(|| panic!("缺少演示色标 {index}"));
            let expected = track.color_hex.clone();
            assert!(meta.label.contains(&expected));
            if track.color.is_none() {
                assert_eq!(
                    expected,
                    crate::bridge::DEFAULT_TRACK_COLOR_HEX,
                    "缺色轨道必须回退到文档常量"
                );
            }
        }
    }

    /// 判据: `MODEL_DRIVEN_FAMILIES` 恰好覆盖投影驱动的族，且**不**覆盖仍是静态的部件。
    ///
    /// 这条判据存在的原因：判据本身曾用"整棵树的标签都不含演示名"做负向断言，
    /// 而侧栏里有 `Sub Bass 低频`、混音台通道条仍用演示轨道名 ⇒ **假红**。
    /// 收窄的口径必须可测，否则下一次还会踩。
    #[test]
    fn model_driven_families_scope_is_exact() {
        let registry = registry();
        for id in [
            "track-0-header",
            "section-0-card",
            "section-0-color-swatch",
            "clip-01J8Z5Q0R7K3M9X2V4B6N8P1F9-header",
            "note-01J8Z5Q0R7K3M9X2V4B6N8P1A2-rect",
            "velocity-0-bar",
            "session-track-0-header",
            "scene-0-color-swatch",
            "scene-launch-0-button",
            "slot-0-0-cell",
            // app-mixer: 调音台通道条 / 电平表 / 主控通道条都由投影或引擎电平驱动。
            "track-0-channel-strip",
            "track-0-meter",
            "track-0-mixer-mute-button",
            "mixer-console",
            "mixer-master-strip",
            "mixer-master-meter",
        ] {
            assert!(registry.contains(id), "判据清单里的 `{id}` 不在注册表里");
            assert!(is_model_driven_family(id), "`{id}` 应当被判为投影驱动");
        }
        for id in [
            "sidebar-item-0",
            "sidebar-category-0-button",
            "device-rack",
            "musical-pr-drawer",
            "undo-tree-modal",
            "status-bar",
            "transport-bpm-field",
            "piano-roll",
        ] {
            assert!(registry.contains(id), "判据清单里的 `{id}` 不在注册表里");
            assert!(
                !is_model_driven_family(id),
                "`{id}` 仍是静态部件, 不该被判为投影驱动"
            );
        }
        assert!(!is_model_driven_family(""));
    }

    #[test]
    fn dump_lines_reproduce_the_registry() {
        let registry = registry();
        let lines = registry.dump_lines();
        assert_eq!(lines.len(), registry.len());
        assert!(lines.iter().all(|line| line.starts_with("element ")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("element track-0-fader role=slider"))
        );
    }

    // =====================================================================
    // app-mixer 工作线：混音台的**文本层**契约（本机不编译 Slint, 这几条是能真跑的那一半）
    // =====================================================================

    /// 读出 `.slint` 里声明的 `in property <...> name: …` 的属性名。
    ///
    /// 三种可见性都要认：`in property`（注入面）、`in-out property`（宿主可写可读，
    /// 例如 `console-tab` / `timecode`）、`out property`（`tokens.slint` 的只读令牌）。
    fn declared_properties(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        for line in source.lines() {
            let trimmed = line.trim();
            let rest = trimmed
                .strip_prefix("in-out property ")
                .or_else(|| trimmed.strip_prefix("in property "))
                .or_else(|| trimmed.strip_prefix("out property "));
            let Some(rest) = rest else {
                continue;
            };
            let Some((_, after)) = rest.split_once('>') else {
                continue;
            };
            if let Some((name, _)) = after.split_once(':') {
                out.push(name.trim().to_owned());
            }
        }
        out
    }

    /// 该属性是否属于**注入面**（`track-*` / `master-*` 两族）。
    ///
    /// 只对这两族做"两侧逐一对应"的断言：内建属性（`x` / `visible` / `width`…）与
    /// 别的注入面（`note-*` / `clip-*`）各有自己的判据，混在一起会让这条检查失真。
    fn is_injection_property(name: &str) -> bool {
        // ⭐ 任务单 `#180`：`master-device-*` 也是 `master-` 前缀，但它的目标组件是
        // **`DeviceRack`**（Tab 3 设备架的第二段），⛔ 不是 `MixerConsole` ⇒ 不参与
        // 本判据的"app → ConsoleTabs → MixerConsole 两级对齐"口径。它的三级转发由
        // 本测试末尾的**方向 4** 单独钉住（`ConsoleTabs → DeviceRack`），⛔ 不是放行。
        (name.starts_with("track-") || name.starts_with("master-"))
            && !name.starts_with("master-device-")
    }

    /// 读出实例化块里**第一层**的属性赋值名（跳过嵌套元素与回调）。
    ///
    /// 只做一层缩进无关的花括号深度计数：本仓库这几个实例化块里没有同名嵌套组件，
    /// 因此"深度 1 上的 `name:`"就是该组件的属性/回调赋值。
    ///
    /// ## 实例化行的形状（本切片收紧了这条**代理**，不变量本身没变）
    ///
    /// 原先只认 `Component {` 这一种写法。`[UI-NOTE-003]` 的力度臂给 `ConsoleTabs` 起了
    /// 元素名以便从子组件**上报**车道顶沿（`console-tabs := ConsoleTabs {`）⇒ 旧代理
    /// 一行都匹配不到，于是返回**空列表**、让调用方的 `assert!(!forwarded.is_empty())` 以
    /// 一句"app.slint 里必须真的转发注入属性"报出来（实测：本切片第一次全量测试就是这样红的）。
    ///
    /// ⛔ 这里**不**把断言放宽：改为①两种写法都认；②**恰好一次**实例化（0 次 = 代理瞎了，
    /// 2 次 = 第二份实例，两者都必须当场说出来，而不是静默返回残缺列表）。
    fn assigned_properties(source: &str, component: &str) -> Vec<String> {
        let header = format!("{component} {{");
        let mut out = Vec::new();
        let mut instantiated = 0_usize;
        let mut inside = false;
        let mut depth = 0_i32;
        for line in source.lines() {
            let trimmed = line.trim();
            if !inside {
                // `Component {` 或 `<id> := Component {`。
                let named = trimmed
                    .split_once(":=")
                    .is_some_and(|(_, rest)| rest.trim() == header);
                if trimmed == header || named {
                    instantiated += 1;
                    inside = true;
                    depth = 1;
                }
                continue;
            }
            if depth == 1
                && let Some((name, _)) = trimmed.split_once(':')
            {
                let name = name.trim();
                if !name.is_empty() {
                    out.push(name.to_owned());
                }
            }
            depth += i32::try_from(trimmed.matches('{').count()).unwrap_or(0)
                - i32::try_from(trimmed.matches('}').count()).unwrap_or(0);
            if depth <= 0 {
                break;
            }
        }
        assert_eq!(
            instantiated, 1,
            "`{component}` 必须**恰好实例化一次**（0 次 = 匹配代理失效，2 次 = 第二份实例）—— \
             本判据靠它读出转发面，读到空列表会让下游断言失去意义"
        );
        out
    }

    /// 判据（文本层）：混音台通道条由**投影 + 引擎电平**驱动，而不是内联演示数据。
    ///
    /// 这条能在本机 `rustc --test` 真跑 —— 它是"本机不编译 Slint"这条纪律下能拿到的
    /// 最强证据之一（`.slint` 的语法/类型仍只能由 CI 判，见 notes §5）。
    #[test]
    fn mixer_console_is_driven_by_the_projection_and_the_meter_arrays() {
        let source = std::fs::read_to_string(ui_dir().join("console/mixer_console.slint"))
            .expect("读 mixer_console.slint");

        for required in [
            // 规模由工程决定（不再是 `for track_index in 6`）
            "for track_name[track_index] in root.track-names",
            // 投影字段逐个进界面
            "root.track-colors[track_index]",
            "root.track-color-labels[track_index]",
            "root.track-mutes[track_index]",
            "root.track-solos[track_index]",
            "root.track-pans[track_index]",
            "root.track-volumes[track_index]",
            "root.track-volume-fractions[track_index]",
            // 电平：柱高 + dBFS 文本（两条独立的机械证据）
            "100px * root.track-meter-levels[track_index]",
            "root.track-meter-peaks[track_index]",
            "root.track-meter-rmss[track_index]",
            // 主控同源
            "root.master-meter-level",
            "root.master-volume-fraction",
            "root.master-pan",
            "root.master-color",
            // dBFS 必须出现在**控件树可读**的属性里（`[UI-TEST-001]` 只允许语义 ID 寻址；
            // `ui/property` 的属性名是 `value`，取自 Slint 的 `accessible-value`（`b37f6ad`
            // 起进 `property_of` 清单）—— 本文件的 dBFS 文本走 `accessible-label`，
            // `track-{i}-meter` / `mixer-master-meter` 另在 `accessible-value` 里各带一份）。
            "dBFS",
        ] {
            assert!(
                source.contains(required),
                "mixer_console.slint 必须包含 `{required}`（投影 / 电平驱动契约）"
            );
        }

        // 负向断言只看**非注释行**：注释里可以（也应该）提到旧写法作为历史。
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "for track_index in 6",
            "db-labels",
            "FADER_LEVELS",
            "0.72",
            "master-level[0]",
        ] {
            assert!(
                !code.contains(forbidden),
                "mixer_console.slint 的代码里不许再有 `{forbidden}`（内联演示数据）"
            );
        }
    }

    /// 判据（文本层）：混音台的**输入面真的存在**，且像素行程常量与 `.slint` 的几何一致。
    ///
    /// ## 它守的是什么（本次的缺口就是它抓的那一格）
    ///
    /// 本次之前 `mixer_console.slint` 的 `TouchArea` 计数是 **0** —— 推子 / 静音 / 独奏
    /// 全是纯 `Rectangle`，四项能力在界面上**完全不可操作**，而当时的判据（控件树里有
    /// 这些语义 ID、值来自投影）**全绿**。这条判据把"输入面"变成机械事实：
    /// 四个 `TouchArea`（推子 / 声相 / 静音 / 独奏）少一个就红。
    ///
    /// **通道条之后再加的两格（本切片）**：主控通道条的推子与声相
    /// （`mixer-master-fader` / `mixer-master-pan`，六个**不带下标**的回调）。
    /// 它们与通道条那四个**同一口径**：少一个就红；另加一条**负向**断言 ——
    /// 主控那一块里不许出现 `track_index`、也不许复用带下标的通道条回调
    /// （主总线不在 `track-names` / `track-ids` 里，按下标寻址对它不存在）。
    ///
    /// ## 第二半：像素行程常量必须与 `.slint` 的几何**同源**
    ///
    /// `MIXER_FADER_TRAVEL_PX` 与 `MIXER_PAN_TRAVEL_PX`（`src/host.rs`）是"拖 1 像素 =
    /// 数值变多少"的唯一口径；它们必须分别等于 `.slint` 里推子帽位置表达式的像素系数与
    /// 声相拖动面的宽度，否则"推子帽动 1 像素"与"数值动一格"会各说各话。
    #[test]
    fn the_mixer_input_surface_exists_and_matches_the_travel_constants() {
        let source = std::fs::read_to_string(ui_dir().join("console/mixer_console.slint"))
            .expect("读 mixer_console.slint")
            // ⚠ **行尾归一**：本判据下面的几何字面量内嵌 `\n`，而 Windows 工作树（无 `.gitattributes`
            // ＋ `core.autocrlf=true`）里同一文件的字节是 CRLF ⇒ ⛔ 字面量匹配不上 ⇒ 只有 windows 腿红
            // （实测根因）。⭐ 归一放在**读盘处**，判据钉的那几行几何**一个字都没改** ⇒ 牙原样保留。
            .replace("\r\n", "\n");

        // ① 输入面：六个 `TouchArea`（通道条四个各是循环体里的一份；主控两个各出现一次）。
        assert!(
            source.matches("TouchArea").count() >= 6,
            "混音台必须有 ≥ 6 个 `TouchArea`（通道条：推子 / 声相 / 静音 / 独奏；\
             主控：推子 / 声相），实测 {}",
            source.matches("TouchArea").count()
        );
        for (what, marker) in [
            (
                "推子竖直拖",
                "root.mixer-fader-grab(track_index, self.mouse-y)",
            ),
            ("推子收尾", "root.mixer-fader-release(track_index)"),
            (
                "声相水平拖",
                "root.mixer-pan-grab(track_index, self.mouse-x)",
            ),
            ("声相收尾", "root.mixer-pan-release(track_index)"),
            ("静音点击", "root.mixer-mute-toggle(track_index)"),
            ("独奏点击", "root.mixer-solo-toggle(track_index)"),
            // 主控：**不带下标**的六个成员（本切片补的那一格）。
            ("主控推子按下", "root.mixer-master-fader-grab(self.mouse-y)"),
            ("主控推子拖动", "root.mixer-master-fader-drag(self.mouse-y)"),
            ("主控推子收尾", "root.mixer-master-fader-release()"),
            ("主控声相按下", "root.mixer-master-pan-grab(self.mouse-x)"),
            ("主控声相拖动", "root.mixer-master-pan-drag(self.mouse-x)"),
            ("主控声相收尾", "root.mixer-master-pan-release()"),
        ] {
            assert!(
                source.contains(marker),
                "混音台缺少「{what}」的输入面：源码里找不到 `{marker}`"
            );
        }

        // ①b 主控通道条的**负向**断言（"主控回退成不可操作" / "主控按下标寻址"都会被这里抓到）：
        // 主总线不在 `track-names` / `track-ids` 里 ⇒ 主控那一块里**不许**出现轨道下标，
        // 也不许复用带 `int` 下标的通道条回调（那会要求发明一个哨兵下标）。
        let master_block = source
            .split_once("mixer-master-strip")
            .map(|(_, tail)| tail)
            .expect("`mixer-master-strip` 必须在源码里（主控通道条）");
        assert!(
            !master_block.contains("track_index"),
            "主控通道条里不许出现 `track_index` —— 身份→下标的算术只有一处来源（宿主）"
        );
        for forbidden in [
            "mixer-fader-grab(track",
            "mixer-pan-grab(track",
            "mixer-fader-release(track",
            "mixer-pan-release(track",
        ] {
            assert!(
                !master_block.contains(forbidden),
                "主控通道条不许复用带下标的通道条回调：源码里出现 `{forbidden}`"
            );
        }

        // ② 推子行程：`.slint` 的位置表达式里的像素系数就是行程。
        let fader_geometry = "60px + 88px * (1.0 - root.track-volume-fractions[track_index])";
        assert!(
            source.contains(fader_geometry),
            "推子帽的位置表达式是像素行程的唯一来源"
        );
        assert_eq!(
            crate::host::MIXER_FADER_TRAVEL_PX,
            88.0,
            "宿主常量必须等于 `.slint` 的 88px"
        );

        // ③ 声相行程：拖动漫面的宽度就是行程（整段字面量一起钉住，避免"读了半个块"）。
        let pan_surface = "            x: Tokens.space-3;\n            y: 18px;\n            width: 36px;\n            height: 18px;";
        assert!(
            source.contains(pan_surface),
            "声相拖动漫面的几何（宽 36px）必须与 `MIXER_PAN_TRAVEL_PX` 一致"
        );
        assert_eq!(crate::host::MIXER_PAN_TRAVEL_PX, 36.0);

        // ④ 主控推子帽的位置表达式与通道条**同一个像素系数**（88px ⇒ 共用一个行程常量）。
        assert!(
            source.contains("60px + 88px * (1.0 - root.master-volume-fraction)"),
            "主控推子帽的位置表达式必须与 `MIXER_FADER_TRAVEL_PX`（88px）同源"
        );
    }

    /// 判据（文本层）：**转发链两侧对齐** —— `app.slint → ConsoleTabs → MixerConsole`
    /// 的每一个投影 / 电平属性都真的在目标组件里声明了。
    ///
    /// 这是本机对"我写对了 20 多个转发属性名"唯一能做的机械检查（对照 CI 的
    /// `cargo build` 才是最终判决）。它抓的是最真实的手滑：`track-meter-rmss` 写成
    /// `track-meter-rms`、`master-color-label` 漏了一级转发 —— 那类错误在 CI 上表现成
    /// 一句"unknown property"，定位成本远高于这里。
    #[test]
    fn forwarded_mixer_properties_exist_in_the_target_components() {
        let ui = ui_dir();
        let app = std::fs::read_to_string(ui.join("app.slint")).expect("读 app.slint");
        let tabs = std::fs::read_to_string(ui.join("console/console_tabs.slint"))
            .expect("读 console_tabs");
        let mixer = std::fs::read_to_string(ui.join("console/mixer_console.slint"))
            .expect("读 mixer_console");

        let main_declared = declared_properties(&app);
        let tabs_declared = declared_properties(&tabs);
        let mixer_declared = declared_properties(&mixer);

        // 方向 1：app.slint 转发给 ConsoleTabs 的每个注入属性都在 ConsoleTabs 里声明过。
        let mut forwarded: Vec<String> = assigned_properties(&app, "ConsoleTabs")
            .into_iter()
            .filter(|name| is_injection_property(name))
            .collect();
        forwarded.sort();
        forwarded.dedup();
        assert!(!forwarded.is_empty(), "app.slint 里必须真的转发注入属性");
        for name in &forwarded {
            assert!(
                tabs_declared.contains(name),
                "app.slint 转发了 `{name}`, 但 ConsoleTabs 没有声明它"
            );
            assert!(
                main_declared.contains(name),
                "MainWindow 必须声明被转发的 `{name}`"
            );
        }

        // 方向 2：ConsoleTabs 转发给 MixerConsole 的每个注入属性都在 MixerConsole 里声明过。
        let mut to_mixer: Vec<String> = assigned_properties(&tabs, "MixerConsole")
            .into_iter()
            .filter(|name| is_injection_property(name))
            .collect();
        to_mixer.sort();
        to_mixer.dedup();
        assert_eq!(
            to_mixer, forwarded,
            "两级转发的注入属性集合必须一致（少一个 = 某一级断了）"
        );
        for name in &to_mixer {
            assert!(
                mixer_declared.contains(name),
                "console_tabs.slint 把 `{name}` 转发给 MixerConsole, 但后者没有声明它"
            );
        }

        // 方向 3（反向）：MixerConsole 声明的每个注入属性都必须被转发，不许有死属性。
        let mut declared_injection: Vec<String> = mixer_declared
            .iter()
            .filter(|name| is_injection_property(name))
            .cloned()
            .collect();
        declared_injection.sort();
        assert_eq!(
            declared_injection, to_mixer,
            "MixerConsole 声明的注入属性与收到的转发必须一一对应"
        );

        // 方向 4（⭐ 任务单 `#180`，`#181` 扩充）：**主总线设备链**的注入面有**第三级
        // 目标**（`ConsoleTabs → DeviceRack`，Tab 3）。集合与三级声明逐条对齐 ——
        // 少一个就是某级断了，多一个就是界面上多了一个没有宿主的属性。
        let rack = std::fs::read_to_string(ui.join("console/device_rack.slint"))
            .expect("读 device_rack.slint");
        let rack_declared = declared_properties(&rack);
        let mut to_rack: Vec<String> = assigned_properties(&tabs, "DeviceRack")
            .into_iter()
            .filter(|name| name.starts_with("master-device-"))
            .collect();
        to_rack.sort();
        to_rack.dedup();
        let mut wanted: Vec<String> = [
            "master-device-bypassed",
            "master-device-eq-available",
            "master-device-eq-device-id",
            "master-device-eq-gain-db",
            "master-device-eq-max-db",
            "master-device-eq-min-db",
            "master-device-eq-reason",
            "master-device-ids",
            "master-device-insert-available",
            "master-device-insert-reason",
            "master-device-label",
            "master-device-names",
            "master-device-recognised",
            "master-device-unrecognised",
        ]
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
        wanted.sort();
        assert_eq!(
            to_rack, wanted,
            "ConsoleTabs 转给 DeviceRack 的 `master-device-*` 注入面必须恰好是这十四格\
             （`#182` 多了 `master-device-eq-device-id`）"
        );
        for name in &to_rack {
            assert!(
                rack_declared.contains(name),
                "console_tabs.slint 把 `{name}` 转发给 DeviceRack, 但后者没有声明它"
            );
            assert!(
                tabs_declared.contains(name),
                "ConsoleTabs 必须声明被转发的 `{name}`"
            );
            assert!(main_declared.contains(name), "MainWindow 必须声明 `{name}`");
        }
    }

    /// 判据（文本层）：`host.rs` 里每一个 `ui.set_x_y(...)` 都能在 `MainWindow` 找到
    /// 对应的 `in property <…> x-y`（kebab-case）。
    ///
    /// app-binding 线把这个对账放在**仓库之外**的探针脚本里；本线把它变成仓库内的判据，
    /// 因为混音台一口气加了 14 个 setter（`docs/ledger/app-mixer-notes.md` §5）。
    #[test]
    fn host_setters_match_the_declared_slint_properties() {
        let ui = ui_dir();
        let app = std::fs::read_to_string(ui.join("app.slint")).expect("读 app.slint");
        let host = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
        )
        .expect("读 host.rs");

        let declared: Vec<String> = declared_properties(&app);
        assert!(declared.len() > 20, "MainWindow 的注入面异常偏小");
        let mut setters: Vec<String> = host
            .lines()
            .filter_map(|line| line.trim().strip_prefix("ui.set_"))
            .filter_map(|rest| rest.split_once('('))
            .map(|(name, _)| name.trim().replace('_', "-"))
            .collect();
        assert!(!setters.is_empty(), "host.rs 里必须真的有 setter 调用");
        setters.sort();
        setters.dedup();
        for name in &setters {
            assert!(
                declared.contains(name),
                "host.rs 调用了 `ui.set_{}`, 但 MainWindow 没有声明 `{name}`",
                name.replace('-', "_")
            );
        }
        // 反向：每一个**注入面**属性都必须被某个 setter 写过（`track-` / `master-` 两族）。
        for name in declared
            .iter()
            .filter(|name| name.starts_with("track-") || name.starts_with("master-"))
        {
            assert!(
                setters.contains(name),
                "MainWindow 声明了 `{name}` 但没有任何 setter 写它（死属性）"
            );
        }
    }

    // =====================================================================
    // 诚实性契约：AI 提案抽屉里**不许出现编造的提案数据**（文本层 + 属性层）
    //
    // 本机不编译 Slint, 因此这里是本地能真跑的那一半；运行时控件树那一半在
    // `tests/live_ui_mcp.rs` 的 `the_musical_pr_drawer_shows_an_honest_empty_state`。
    // =====================================================================

    /// 去掉 `//` 行注释与行尾注释后的**代码文本**。
    ///
    /// 为什么要去注释: 本票把"什么被删掉了"写进了注释（审计留痕）。探针若把注释也算作
    /// "界面里的字面量", 就会逼着作者**不写**留痕 —— 那与本仓库的纪律相反。
    /// 只处理 `//`: `.slint` 与 `.rs` 在本仓库都不用块注释承载会出现在界面上的字符串。
    fn code_only(source: &str) -> String {
        source
            .lines()
            .map(|line| match line.split_once("//") {
                Some((code, _comment)) => code,
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 取出 `{name} := TouchArea { … }` 的**代码**片段（花括号配平）。
    ///
    /// 返回 `None` = 源文件里没有这个 `TouchArea`。那本身就是要红的事实：
    /// `ui/` ↔ 注册表的双向契约要求它留在源文件里。
    fn touch_area_body(code: &str, name: &str) -> Option<String> {
        let marker = format!("{name} := TouchArea {{");
        let start = code.find(&marker)?;
        let rest = &code[start..];
        let mut depth = 0_i32;
        for (index, ch) in rest.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(rest[..=index].to_owned());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// 判据（**"今天确实不可用"钉死**）：走带区三个**文档级动作**今天都没有接线。
    ///
    /// 三个控制由 `953b85b`（"Slint 主窗口骨架…"）加进走带区。那一票的提交信息逐字写着
    /// "`GUI` 路径注入演示数据并接线回调"与"回调只打 stderr"，并在边界一节写明
    /// "走带 / Op 归约 / AI 采纳 / 撤销栈 / 电平 SPSC / … 均未实现"。
    ///
    /// 本判据把"今天确实不可用"钉成**机械事实**：将来谁真接了、忘了改判据，会红。
    /// 谁正是来接线的，就**必须**一并更新本判据（它钉的是**今天**的状态，不是永久禁令）。
    ///
    /// 三个控制各自卡在哪（代价表与逐条证据见交付报告）。
    /// 行号刻意**不写**：本仓库的守卫口径是"符号是判据、行号是提示"，
    /// 而 `ui/transport.slint` 同一片区域今天还有别的切片在改（行号会漂）。
    ///
    /// - `transport-commit-button`（`ui/transport.slint` 的 `commit_area`）—— **能力不存在**：
    ///   每一次界面编辑已经各自 `commit_ops` 一次（`src/host.rs:1660` 等），
    ///   会话里**没有**"待提交的编辑"这一状态；权威对**空** op 批次明确拒绝
    ///   （`crates/yeban-mcp/src/domain/mod.rs:2152`，`mutates_project` 也按 `!ops.is_empty()`
    ///   判定，`:893` / `:952`）。
    /// - `transport-revert-button`（同文件的 `revert_area`）—— **能力存在但未接线**：
    ///   每次提交恰好携带一条 `Op::Batch`（`crates/yeban-mcp/src/undo_session.rs:445`，
    ///   `:684` 的"本实现保证每次提交恰好一条 op"），因此**一步撤销 = 回退一个 Commit**，
    ///   载体是既有的 `UiAction::Undo`。
    /// - `transport-branch-button`（同文件，**没有** `TouchArea`）—— **能力不存在**：
    ///   没有任何"切换活跃分支"的 API（`HostAction` 只有 `Undo` / `Redo` / `Commit`，
    ///   `domain/mod.rs:669`）；它今天只是一块**只读**显示（`src/host.rs:2708` 写 `branch-name`）。
    ///
    /// **为什么不在本判据里顺手把按钮置灰**：任何**用户可见**的处置（置灰 + 文字说明，
    /// 或移除控件）都会改默认帧 ⇒ `crates/yeban-app/tests/golden/linux/` 的 5 张基准
    /// **确定性**过期，而基准只许按 `gates-manual.yml` 的 `goldens` 档重录 + 人工复核。
    /// 只写 `accessible-label` 不算诚实处置 —— 本文件 `musical_pr_drawer_declares_no_demo_proposals`
    /// 的判据已经写明"只写在注释或 `accessible-label` 里用户看不见"。
    #[test]
    fn the_three_transport_document_actions_are_not_wired_today() {
        let transport = code_only(
            &std::fs::read_to_string(ui_dir().join("transport.slint")).expect("读 transport.slint"),
        );

        // ① `app.slint` 的 `Transport { … }` 实例**没有**转发这三个回调。
        //    查在**声明**之前：一个只声明不转发的回调是空壳，一个 `name => …` 的转发
        //    才是"用户点下去真的会走那条链"。两条各自独立（本判据的注入实测见交付报告）。
        let app =
            code_only(&std::fs::read_to_string(ui_dir().join("app.slint")).expect("读 app.slint"));
        for name in ["commit", "revert", "branch"] {
            for spelling in [format!("{name} =>"), format!("{name}=>")] {
                assert!(
                    !app.contains(&spelling),
                    "`app.slint` 今天不得转发 `{spelling}` —— \
                     谁接上了就一并更新本判据（`the_three_transport_document_actions_are_not_wired_today`）"
                );
            }
        }

        // ② `transport.slint` **没有**为这三个动作声明任何回调。
        //    声明了却接不上（或接了却没人转发）就是假契约。
        for name in ["commit", "revert", "branch"] {
            assert!(
                !transport.contains(&format!("callback {name}")),
                "`transport.slint` 今天不得声明 `callback {name}` —— \
                 谁接上了就一并更新本判据（`the_three_transport_document_actions_are_not_wired_today`）"
            );
        }

        // ③ 两个**已经存在**的 `TouchArea` 里没有 `clicked`。
        //    它们今天只在按下时换底色（`background: commit_area.pressed ? …`），松手什么都不发生。
        for area in ["commit_area", "revert_area"] {
            let body = touch_area_body(&transport, area).unwrap_or_else(|| {
                panic!("`{area}` 必须留在源文件里（`ui/` ↔ 注册表的双向契约要它）")
            });
            assert!(
                !body.contains("clicked"),
                "`{area}` 今天不得有 `clicked` —— 谁接上了就一并更新本判据\n{body}"
            );
        }

        // ④ 分支那块连点击源都没有：`transport-branch-button` 到下一个 `accessible-id`
        //    之间不得出现 `TouchArea`（否则"点得动但没反应"又回来了）。
        let branch_start = transport
            .find("accessible-id: \"transport-branch-button\";")
            .expect("`transport-branch-button` 必须留在源文件里");
        let branch_rest = &transport[branch_start..];
        let branch_end = branch_rest
            .find("accessible-id: \"transport-commit-button\";")
            .expect("`transport-commit-button` 必须紧跟分支控件之后");
        assert!(
            !branch_rest[..branch_end].contains("TouchArea"),
            "`transport-branch-button` 今天不得有点击源（切换分支的 API 一处都不存在）"
        );

        // ⑤ 三个语义 ID **仍然登记**：登记与接线是两件事，本判据只钉接线。
        //    这样"顺手把控件删掉"也会走到这里，逼作者显式改判据（删或接，都要留痕）。
        let registry = registry();
        for id in [
            "transport-branch-button",
            "transport-commit-button",
            "transport-revert-button",
        ] {
            assert!(
                registry.contains(id),
                "`{id}` 必须留在语义注册表里 —— 移除它是另一票（要重录 5 张 Linux 基准）"
            );
        }
    }

    /// 判据（**"录音今天确实不可用"钉死**，`[ARCH-REC-001]` 的缺口）：录音键**没有**点击处理器，
    /// 而且这不是"忘了接线"——**录音能力在仓库里一处都不存在**。
    ///
    /// 与上一条判据（`the_three_transport_document_actions_are_not_wired_today`）同款：
    /// 它钉的是**今天**的状态，不是永久禁令。将来谁真接了录音，**必须**一并更新本判据。
    ///
    /// ## 规范说"要有"，代码说"没有"
    ///
    /// - **规范（Normative）**：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §10.1 第 1 条
    ///   `[ARCH-REC-001]` 逐字要求"点击录音 Arm 按钮后，音频线程直接从声卡物理输入捕获 PCM 流
    ///   并写入预分配的无锁环形缓冲区，由后台 I/O 线程刷盘至 CAS 临时暂存…"——
    ///   规范**定义了**它，实现**一处都没有**。
    /// - **引擎**：`crates/yeban-engine/src/transport.rs` 的 `TransportState::Recording` 是**预留档**，
    ///   注释逐字写着"`Recording` 已**预留**但本切片不产生它：录音路径（`ARCH-REC-*`）还没有实现…
    ///   因此枚举里有这一档、状态机里没有入口"；`crates/yeban-engine/src/lib.rs` 同款。
    ///   `TransportCommand`（`crates/yeban-engine/src/ring.rs`）只有 `Play` / `Stop` / `Pause` /
    ///   `SeekTicks` 四条，**没有**任何一条能到 `Recording`。
    /// - **模型**：`TrackV3`（`crates/yeban-model/src/project.rs`）没有"已挂载 / 输入源"字段；
    ///   `grep -rn 'armed' crates/yeban-model/src` 命中 **0**。唯一的输入设备记录是
    ///   `crates/yeban-model/src/local_config.rs` 的 `AudioPortBinding::input_device` ——
    ///   它是**本机端口绑定**（`MODEL-ISO-001` 第 3 层，永不进工程文件），全仓库**没有**消费者
    ///   （除 model 自己的测试），不是"录音已挂载"。
    /// - **MCP**：`grep -n 'record\|Record' crates/yeban-mcp/src/tools.rs` 命中 **0**
    ///   （17 个工具里没有录音工具）。
    ///
    /// ## 三条独立探针（破坏任何一条都变红）
    ///
    /// 1. `ui/transport.slint` 的 `rec_area`（`TouchArea`）**没有** `clicked`；
    /// 2. 该文件**不声明** `callback record`，`ui/app.slint` 也**不转发** `record =>`
    ///    —— 声明了却接不上（或接了却没人转发）就是假契约；
    /// 3. `transport-record-button` **仍然登记**在语义注册表里（登记与接线是两件事）。
    ///
    /// ## 为什么本判据不顺手把按钮置灰 / 删掉
    ///
    /// 任何**用户可见**的处置都会改默认帧 ⇒ `crates/yeban-app/tests/golden/linux/` 的 5 张基准
    /// **确定性**过期，而基准只许按手动档 `gates-manual.yml` 的 `goldens` 档重录 ＋ 人工复核。
    /// 而且 `transport-record-button` 同时在 `tests/golden` 无关的三处清单里：
    /// 语义注册表、`src/test_port_adapter.rs` 的 `DEFAULT_VIEW_MUST_HAVE` 与 `VISIBLE_SINGLETONS`。
    ///
    /// ## 已知的**假象**（登记，不在本票处置）
    ///
    /// 录音键今天声明 `accessible-checkable: true` ＋ `accessible-checked: false`，而宿主**没有**
    /// 任何写者 ⇒ 屏读器会把它读成"一个永远是关的可勾选按钮"（与 `ffb3dcc` 修掉的 AI 徽章
    /// "假装有状态"同族）。它的运行时可观测面由 `src/test_port_adapter.rs` 的
    /// `pressing_the_record_button_today_changes_nothing` 钉住（`checked` 必须恒为 `Some(false)`）。
    /// 处置它（改成不可勾选，或真接录音）会动 `accessible-checkable` —— 那要么是**能力**裁决，
    /// 要么是**另一票**（可能连带重录基准），因此本票只如实登记。
    #[test]
    fn the_record_button_has_a_click_handler_and_a_wired_record_callback() {
        let transport = code_only(
            &std::fs::read_to_string(ui_dir().join("transport.slint")).expect("读 transport.slint"),
        );

        // ① `rec_area` 留在源文件里，**并且有** `clicked`（本轮接上了 `[ARCH-REC-001]`）。
        //
        // ⚠ 本条**从"不得有 `clicked`"反转成"必须有"**：上一版的注释逐字写着
        // "谁真接了录音就一并更新本判据"（`ui/transport.slint` 同款）。反转的是**能力**，
        // ⛔ 不是把判据放松。
        let body = touch_area_body(&transport, "rec_area").unwrap_or_else(|| {
            panic!("`rec_area` 必须留在源文件里（`ui/` ↔ 注册表的双向契约要它）")
        });
        assert!(
            body.contains("clicked"),
            "`rec_area` 必须有 `clicked` —— 录音键是**动作**键，悬空就是假控件\n{body}"
        );
        assert!(
            body.contains("toggle-record"),
            "按下必须落到 `root.toggle-record()`（⛔ 不是空回调）\n{body}"
        );

        // ② 回调**声明**且**转发**（缺一就是假契约）。
        assert!(
            transport.contains("callback toggle-record()"),
            "`transport.slint` 必须声明 `callback toggle-record()`"
        );
        let app =
            code_only(&std::fs::read_to_string(ui_dir().join("app.slint")).expect("读 app.slint"));
        assert!(
            app.contains("callback toggle-record()"),
            "`app.slint` 必须声明 `callback toggle-record()`（根上的转发口）"
        );
        assert!(
            app.contains("toggle-record =>"),
            "`app.slint` 必须把 `toggle-record` 转给宿主（⛔ 不转发＝界面自己吃掉了按键）"
        );
        // ③ 落盘的路径只有一个真相源：`YEBAN_UI_RECORD_WAV_PATH` 环境变量族（见
        //    `record_wav::default_record_path`）。⛔ 界面不许自己写死一个路径。
        assert!(
            !transport.contains(".wav"),
            "`transport.slint` 不得出现 `.wav` —— 路径由宿主从环境变量族取，界面不认识文件"
        );

        // ③ 语义 ID **仍然登记**：登记与接线是两件事。这样"顺手把控件删掉"也会走到这里，
        //    逼作者显式改判据（删或接，都要留痕；删还要重录 5 张 Linux 基准）。
        assert!(
            registry().contains("transport-record-button"),
            "`transport-record-button` 必须留在语义注册表里 —— 移除它是另一票（要重录基准）"
        );
    }

    /// 判据（**"两键今天不可区分"钉死**）：`Space` 与 `Shift+Space` 在宿主侧共用**一个** match 臂。
    ///
    /// `crates/yeban-app/src/input.rs` 把两键解析成**两个不同**的 `Action`
    /// （`Key::Space` → `Action::PlayPause`、`Shift` + `Key::Space` → `Action::ResumeFromCursor`），
    /// `crates/yeban-app/src/cli.rs` 的 `SHORTCUTS` 表也把它们当成两条规范快捷键
    /// （两条的 `implemented` 都是 `true` ⇒ `--print-shortcuts` 都不打 `(未实现)`）。
    /// 但 `src/host.rs` 的 `apply_action` 里，两键落到**同一个**臂：
    ///
    /// ```text
    /// Action::PlayPause | Action::ResumeFromCursor => { ui.invoke_toggle_play(); true }
    /// ```
    ///
    /// ⇒ 今天**没有**任何可观测差异。本判据把这件事钉成机械事实：
    /// 将来谁做出"从光标续播"的独立语义，这里会红，作者必须一并更新本判据与 `cli.rs` 的表述。
    /// 运行时那一半（两键的引擎读数逐位相等）由 `src/test_port_adapter.rs` 的
    /// `shift_space_and_space_are_the_same_transport_path_today` 见证 —— 两条路径彼此独立。
    ///
    /// **为什么查源码而不是只查运行时**：运行时那句话是"读数相同"，
    /// 它证明不了"同一个臂"（两段等价的代码也会给出相同读数）。源码文本证明的是**耦合本身**。
    #[test]
    fn space_and_shift_space_share_one_host_arm_today() {
        // `include_str!`（而不是运行时读盘）：同一目录下的兄弟文件，编译期就把正文嵌进来，
        // 因此这条源码探针与当前目录无关、也不会因工作目录不同而失效。
        let host = code_only(include_str!("host.rs"));

        // ① 两个 `Action` 在 `host.rs` 的**代码**里各自只出现一次 —— 出现两次就是第二个落点。
        for needle in ["Action::PlayPause", "Action::ResumeFromCursor"] {
            assert_eq!(
                host.matches(needle).count(),
                1,
                "`host.rs` 的代码里 `{needle}` 今天必须**恰好出现一次**（唯一落点）—— \
                 多出来的那一次是第二条走带语义，本判据与 `cli.rs` 的表述都要一起更新"
            );
        }

        // ② 那唯一的落点是**合起来的**一个臂，臂体里只有 `ui.invoke_toggle_play()`。
        let arm_start = host
            .find("Action::PlayPause | Action::ResumeFromCursor")
            .expect("两键必须共用 `Action::PlayPause | Action::ResumeFromCursor` 这一个臂");
        let arm_rest = &host[arm_start..];
        let arm_end = arm_rest
            .find("Action::Undo")
            .expect("走带臂之后必须紧跟撤销族臂（把它当作本臂的右边界）");
        let arm = &arm_rest[..arm_end];
        assert!(
            arm.contains("ui.invoke_toggle_play()"),
            "走带臂必须落到界面已有的 `toggle-play`（`wire_transport` 接到引擎的那一个回调）\n{arm}"
        );
        assert!(
            !arm.contains("stop_and_rewind"),
            "走带臂今天**不回到起点**（`Action::PlayPause` 的文档注释写\"暂停时播放头回起始点\"，\
             与实现不符；`Stop` 才回起点）。若这里出现 `stop_and_rewind`，语义变了 ⇒ 更新本判据\n{arm}"
        );
    }

    /// 判据（**多路径探针：源码文本 + 属性默认值 + 循环规模**）：抽屉里没有任何假提案。
    ///
    /// 三条路径各自独立, 破坏任何一条都变红：
    ///
    /// 1. **全 `ui/` 源码文本**: [`DEMO_PROPOSAL_LITERALS`] 一条都不得出现在 `ui/**` 的
    ///    任何 `.slint` 的**代码**里。只查抽屉那一个文件不够 —— 把假数据搬到 `app.slint`
    ///    再注入, 用户看到的还是一模一样的假界面。
    /// 2. **属性默认值**: 四个数据面属性的默认值必须是 `0` / `[]`。这一条钉的是
    ///    "宿主不写时界面显示什么", 而今天**没有**任何宿主写它们（下一条判据证明）。
    /// 3. **循环规模**: 提案列表必须由**数据**驱动, 不得是固定的 `for … in 3` ——
    ///    固定循环加空数组会在界面上留三张空卡片。
    ///
    /// 还有一条**空态**断言: 零提案时界面必须有一句用户可见的真话,
    /// 而不是一片空白（空白让用户以为界面坏了）。文案必须同时是 `accessible-label`
    /// 与可见 `text` —— 只写一处就是"源码里说真话、用户看不见"的老毛病。
    #[test]
    fn musical_pr_drawer_declares_no_demo_proposals() {
        let drawer_path = ui_dir().join("dialogs/musical_pr_drawer.slint");
        let drawer = std::fs::read_to_string(&drawer_path).expect("读 musical_pr_drawer.slint");

        // ---- 路径 ①: 全 ui/ 的**代码**里不得有假提案字面量 ----
        let ui = ui_dir();
        let mut files = Vec::new();
        collect_slint_files(&ui, &mut files);
        files.sort();
        assert!(!files.is_empty(), "ui/ 下一个 .slint 都没有?");
        let mut hits: Vec<String> = Vec::new();
        for file in &files {
            let rel = file
                .strip_prefix(&ui)
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            let code = code_only(&std::fs::read_to_string(file).expect("读 .slint 失败"));
            for literal in DEMO_PROPOSAL_LITERALS {
                if code.contains(literal) {
                    hits.push(format!("{rel}: `{literal}`"));
                }
            }
        }
        assert!(
            hits.is_empty(),
            "假提案数据回到了界面里（编造的提案文案 / 置信度不许出现在任何 .slint 的代码里）:\n  {}",
            hits.join("\n  ")
        );

        // ---- 路径 ②: 四个属性的**默认值**必须是空的 ----
        for (slint_name, _) in MUSICAL_PR_PROPERTY_NAMES {
            assert!(
                !drawer.contains(&format!("{slint_name}: 3")),
                "`{slint_name}` 的默认值回到了演示常量"
            );
        }
        for literal in [
            "in property <int> proposal-count: 0;",
            "in property <[string]> proposal-labels: [];",
            "in property <[string]> proposal-kinds: [];",
            "in property <[string]> confidences: [];",
        ] {
            assert!(
                drawer.contains(literal),
                "抽屉的数据面必须默认全空, 缺这一行: `{literal}`"
            );
        }

        // ---- 路径 ③: 提案列表由**数据**驱动, 不是固定的三条 ----
        assert!(
            drawer.contains("in root.proposal-kinds :"),
            "提案列表必须由数据（`root.proposal-kinds`）驱动, 不能是固定条数"
        );
        assert!(
            !drawer.contains("for proposal_index in 3"),
            "固定的 `for proposal_index in 3` 必须消失（它会在空数组上留三张空卡片）"
        );

        // ---- 空态: 用户可见的真话（`accessible-label` 与 `text` **两处都要有**）----
        assert!(
            drawer.contains("accessible-id: \"musical-pr-empty-state\";"),
            "零提案时必须有一个语义 ID = `musical-pr-empty-state` 的空态图元"
        );
        assert!(
            drawer.contains("accessible-label: \"还没有 AI 提案。这条链路尚未接线。\";"),
            "空态的 `accessible-label` 必须就是界面上那句话（读屏用户听到的也是它）"
        );
        assert!(
            drawer.contains("text: \"还没有 AI 提案。这条链路尚未接线。\";"),
            "空态必须有一句**可见**的文案 —— 只写在注释或 `accessible-label` 里用户看不见"
        );

        // ---- 零提案时不画"采纳 / 放弃": 一个可点的「采纳」在空列表上是假控件 ----
        assert_eq!(
            drawer
                .matches("if root.proposal-count > 0 : Rectangle")
                .count(),
            2,
            "「采纳」与「放弃」两个按钮都必须挂在 `proposal-count > 0` 的条件上"
        );
        for id in ["musical-pr-accept-button", "musical-pr-reject-button"] {
            assert!(
                drawer.contains(&format!("accessible-id: \"{id}\";")),
                "`{id}` 必须留在源文件里（`ui/` ↔ 注册表的双向契约要它）"
            );
        }
    }

    /// 判据（**属性层的前提**）：没有任何宿主代码写抽屉的四个数据面属性。
    ///
    /// 这条不是顺手加的对账 —— 它是上一条判据**路径 ②** 得以成立的前提:
    /// 只要有任何一处 Rust 写这四个属性, "默认值 = 用户看到的"就不再成立,
    /// 路径 ② 就退化成一句空话。
    ///
    /// 两条子路径：
    /// - **Rust 侧**: `src/**` 的**代码**里不得出现 Slint 生成的 setter
    ///   （`set_proposal_count(` 等）。口径与 `host_setters_match_the_declared_slint_properties`
    ///   同款 —— 都是文本层扫 `.rs`。
    /// - **界面侧**: 除了抽屉自己, `ui/**` 的**代码**里不得出现那四个 kebab-case 属性名。
    ///   在 `app.slint` 里绑一句（`proposal-count: root.…`）就等于接上了注入面。
    ///
    /// 本仓纪律: "把动作吞掉却什么都不做, 比不处理更糟"（见 `host.rs` 的
    /// `action_has_implementation` 文档）。这一条把"今天真的没有数据源"从一句话变成断言。
    #[test]
    fn no_host_writes_the_musical_pr_proposal_properties() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut rs_files = Vec::new();
        collect_files_with_extension(&src, "rs", &mut rs_files);
        rs_files.sort();
        assert!(!rs_files.is_empty(), "src/ 下一个 .rs 都没有?");

        let mut setter_hits: Vec<String> = Vec::new();
        for file in &rs_files {
            let rel = file
                .strip_prefix(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            let code = code_only(&std::fs::read_to_string(file).expect("读 .rs 失败"));
            for (_, rust_name) in MUSICAL_PR_PROPERTY_NAMES {
                if code.contains(&format!("set_{rust_name}(")) {
                    setter_hits.push(format!("{rel}: set_{rust_name}(…)"));
                }
            }
        }
        assert!(
            setter_hits.is_empty(),
            "有宿主代码在写抽屉的提案数据面 ⇒ 界面上的提案不再是'没有数据源':\n  {}",
            setter_hits.join("\n  ")
        );

        let ui = ui_dir();
        let mut slint_files = Vec::new();
        collect_slint_files(&ui, &mut slint_files);
        slint_files.sort();
        let drawer_rel = "dialogs/musical_pr_drawer.slint";
        let mut bind_hits: Vec<String> = Vec::new();
        for file in &slint_files {
            let rel = file
                .strip_prefix(&ui)
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            if rel == drawer_rel {
                continue; // 抽屉自己**声明**这四个属性, 这不是注入。
            }
            let code = code_only(&std::fs::read_to_string(file).expect("读 .slint 失败"));
            for (slint_name, _) in MUSICAL_PR_PROPERTY_NAMES {
                if code.contains(&format!("{slint_name}:")) {
                    bind_hits.push(format!("{rel}: {slint_name}:"));
                }
            }
        }
        assert!(
            bind_hits.is_empty(),
            "抽屉之外有 .slint 在绑定提案数据面 ⇒ 用户看到的不再是默认值:\n  {}",
            bind_hits.join("\n  ")
        );
    }

    /// 收集 `dir` 下（递归）全部扩展名为 `ext` 的文件。
    ///
    /// 与上面的 `collect_slint_files` 同款, 只是扩展名可参数化（本判据还要扫 `src/**` 的 `.rs`）。
    fn collect_files_with_extension(
        dir: &std::path::Path,
        ext: &str,
        out: &mut Vec<std::path::PathBuf>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files_with_extension(&path, ext, out);
            } else if path.extension().is_some_and(|found| found == ext) {
                out.push(path);
            }
        }
    }

    // =======================================================================
    // `[UI-A11Y-003]` §7.3 第 4 条（`TEST-SPEC-005` 用例 ①）：
    // **遍历交互控件, 断言无障碍名称非空**, 并且断言它的无障碍**动作**真的接在
    // 指针点击的同一个落点上。
    //
    // 为什么这一族判据住在纯 Rust 侧: 它们读的是 `.slint` 源码文本, 不需要 Slint
    // 运行时, 因此**本机**（不许编译 Slint 的机器）也能跑。运行时那一半
    // （真控件树的标签非空）住在 `test_port_adapter.rs`, 只有 CI 能跑 —— 两半都在,
    // 判据才既"早"又"真"。
    //
    // ## 为什么"绑定了 `accessible-action-*`"才是判据
    //
    // 上游只在 `.slint` 绑定了 `accessible-action-*` 时才把动作导出给 OS 无障碍树:
    // `i-slint-compiler-1.18.1/generator/rust.rs` 的 `supported_accessibility_actions`
    // 只从 `component.accessible_prop` 里的 `Action*` 项收集; `i-slint-backend-winit`
    // 的 accesskit 适配器只按它 `node.add_action(accesskit::Action::Click)`。
    // 也就是说: **一个只声明了 `accessible-role: button` 的节点, 屏读器读得到标签、
    // 却按不动它。** 这一族判据把"按得动"从口号变成机械检查。
    // =======================================================================

    /// `.slint` 文本里一个声明了 `accessible-role` 的**元素块**（行区间）。
    #[derive(Debug)]
    struct RoleBlock {
        /// 元素开头那一行在文件里的下标。
        start: usize,
        /// 配平 `{ … }` 的那一行在文件里的下标。
        end: usize,
        /// `accessible-role` 的字面取值（`button` / `tab` / …）。
        role: String,
    }

    /// 一行开头的空格数 —— `.slint` 用缩进表达父子关系。
    fn indent_of(line: &str) -> usize {
        line.len() - line.trim_start().len()
    }

    /// 找到全部声明了 `accessible-role` 的元素块。
    ///
    /// 口径（机械, 不猜）：
    /// 1. 某一行含 `accessible-role: <kebab 字面量>;` —— 字面量不是单一 kebab 标识符的
    ///    行不算（那样可以把注释里的示例文本误认成声明, 但注释已在 `code_only` 里剥掉）;
    /// 2. 块首 = 该行**之前**最近的一行, 它更浅缩进且以 `{` 结尾;
    /// 3. 块尾 = 从块首起花括号**首次配平**的那一行。
    fn role_blocks(lines: &[&str]) -> Vec<RoleBlock> {
        let mut out = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let Some((_, rest)) = line.split_once("accessible-role:") else {
                continue;
            };
            let role = rest.trim().trim_end_matches(';').trim().to_owned();
            if role.is_empty() || !role.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
                continue;
            }
            let indent = indent_of(line);
            let start = (0..index).rev().find(|&probe| {
                lines[probe].trim_end().ends_with('{') && indent_of(lines[probe]) < indent
            });
            let Some(start) = start else {
                continue;
            };
            let mut depth = 1_i32;
            let mut end = lines.len() - 1;
            for (probe, text) in lines.iter().enumerate().skip(start + 1) {
                depth += text.matches('{').count() as i32 - text.matches('}').count() as i32;
                if depth <= 0 {
                    end = probe;
                    break;
                }
            }
            out.push(RoleBlock { start, end, role });
        }
        out
    }

    /// 取出 `keyword => { … }` 的全部块体（花括号配平）。
    ///
    /// - 返回 `(块体首行的文件行下标, 空白归一化后的块体文本)`;
    /// - `base_line` 是 `code` 首行在文件里的下标（传整份文件时是 0）;
    /// - 关键字必须**独立成词**（前后不是标识符字符）, 否则 `clicked` 会命中
    ///   `pointer-clicked` 这类更长名字。
    fn arrow_bodies(code: &str, keyword: &str, base_line: usize) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        let mut from = 0_usize;
        while let Some(offset) = code[from..].find(keyword) {
            let at = from + offset;
            from = at + keyword.len();
            let before_ok = code[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '_'));
            if !before_ok {
                continue;
            }
            let rest = code[at + keyword.len()..].trim_start();
            let Some(rest) = rest.strip_prefix("=>") else {
                continue;
            };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('{') else {
                continue;
            };
            let mut depth = 1_i32;
            for (index, ch) in rest.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            let body = rest[..index]
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ");
                            let line = base_line + code[..at].matches('\n').count();
                            out.push((line, body));
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// `ui/**.slint` 的 `(相对路径, 文本)` 清单（按路径排序, 迭代顺序确定）。
    fn slint_sources() -> Vec<(String, String)> {
        let ui = ui_dir();
        let mut files = Vec::new();
        collect_slint_files(&ui, &mut files);
        files.sort();
        files
            .into_iter()
            .map(|file| {
                let rel = file
                    .strip_prefix(&ui)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = std::fs::read_to_string(&file).expect("读取 .slint 失败");
                (rel, text)
            })
            .collect()
    }

    /// 判据（`[UI-A11Y-003]` §7.3 第 4 条 ① / `TEST-SPEC-005`）：**每一个可点击的交互
    /// 控件都必须绑定无障碍动作, 而且落点与指针点击逐字相同。**
    ///
    /// 口径：
    /// - "可点击的交互控件" = 某一行有 `clicked => { … }`, 且它**最内层**的
    ///   `accessible-role` 元素块的角色属于交互族（`button` / `tab` / `switch` /
    ///   `checkbox` / `radio-button` / `combobox` / `list-item`）。用"最内层"是为了
    ///   正确归属**嵌套**点击：轨道头（`list-item`）里包着静音 / 独奏两个按钮,
    ///   那两处点击属于按钮, 不属于轨道头。
    /// - "落点逐字相同" = 无障碍动作块体与点击块体在**空白归一化**后相等。
    ///   这就排除了"挂一个什么都不做的动作"与"挂到另一个回调上"。
    /// - 动作声明必须**在点击源之前**（本文件的既有形状: 无障碍属性与几何 / 颜色一起
    ///   写在元素自己那一块, 子元素在后）—— 这条同时挡住了"借用子元素动作"的假通过。
    ///
    /// 注入证明（破坏 ⇒ 抓字面红行）：见交付报告。摘掉任意一条
    /// `accessible-action-default => { … }` 即红。
    #[test]
    fn every_clickable_control_binds_a_matching_accessibility_action() {
        const INTERACTIVE_ROLES: [&str; 7] = [
            "button",
            "tab",
            "switch",
            "checkbox",
            "radio-button",
            "combobox",
            "list-item",
        ];
        let mut checked = 0_usize;
        for (rel, text) in slint_sources() {
            let code = code_only(&text);
            let lines: Vec<&str> = code.lines().collect();
            let blocks = role_blocks(&lines);
            for (click_line, click_body) in arrow_bodies(&code, "clicked", 0) {
                let block = blocks
                    .iter()
                    .filter(|block| block.start < click_line && click_line <= block.end)
                    .max_by_key(|block| block.start);
                let Some(block) = block else {
                    continue;
                };
                if !INTERACTIVE_ROLES.contains(&block.role.as_str()) {
                    continue;
                }
                let block_text = lines[block.start..=block.end].join("\n");
                let actions: Vec<(usize, String)> = ["default", "expand"]
                    .iter()
                    .flat_map(|name| {
                        arrow_bodies(
                            &block_text,
                            &format!("accessible-action-{name}"),
                            block.start,
                        )
                    })
                    .filter(|(line, _)| *line < click_line)
                    .collect();
                assert!(
                    actions.iter().any(|(_, body)| *body == click_body),
                    "{rel} 第 {} 行: 角色 `{}` 的控件有点击源, 却没有绑**同一个落点**的 \
                     `accessible-action-*`。\n\
                     点击落点: {click_body:?}\n\
                     该元素块里的无障碍动作: {actions:?}\n\
                     宪法依据: `[UI-A11Y-003]` §7.3 第 4 条 ① —— 没有动作绑定, OS 无障碍树里\
                     这个节点读得到、按不动（上游 `supported_accessibility_actions` 只收集\
                     `accessible-action-*`）。",
                    click_line + 1,
                    block.role
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 20,
            "只检查到 {checked} 个可点击控件 ⇒ 判据可能空转（口径或文本解析漂了）"
        );
    }

    /// 判据（`[UI-A11Y-003]` §7.3 第 4 条 ①）：**无障碍名称不得出现空字面量。**
    ///
    /// §7.3 第 4 条 ① 的原话是"遍历主界面所有交互控件并断言无障碍名称非空"。文本层
    /// 能机械抓住的那一类缺陷就是"表达式里写了 `\"\"`"：`sidebar-item-{i}` 曾经写成
    /// `root.compact ? \"\" : root.items[item_index]` ⇒ 折叠断点下 8 个可交互条目在
    /// 无障碍树里全是**无名**节点（屏读器读到 8 个一模一样的空项目）。
    ///
    /// 为什么连"三元表达式的一支"也算违规: §7.3 第 4 条 ① 要求的是名称**非空**,
    /// 不是"在某个断点下非空"; 折叠是视觉选择, 不该抹掉名称。运行时那一半
    /// （两个断点下真控件树的标签都非空）在 `test_port_adapter.rs`。
    #[test]
    fn no_accessible_name_may_be_the_empty_literal() {
        let mut labels = 0_usize;
        for (rel, text) in slint_sources() {
            for (index, line) in code_only(&text).lines().enumerate() {
                let Some((_, rest)) = line.split_once("accessible-label:") else {
                    continue;
                };
                labels += 1;
                assert!(
                    !rest.contains("\"\""),
                    "{rel} 第 {} 行: 无障碍名称里出现空字面量 ⇒ 某个断点下这个交互控件会变成\
                     **无名**节点（`[UI-A11Y-003]` §7.3 第 4 条 ① 要求名称非空）。\n{line}",
                    index + 1
                );
            }
        }
        assert!(
            labels >= 90,
            "只读到 {labels} 条 accessible-label ⇒ 判据可能空转（口径或文本解析漂了）"
        );
    }

    /// 判据：**没有点击源的交互控件必须恰好是这一份清单**（每一个都有登记的理由）。
    ///
    /// 这条判据补的是 [`every_clickable_control_binds_a_matching_accessibility_action`]
    /// 的缺口：那条只约束"有点击源的控件"。一个**新加**的、`accessible-role: button`
    /// 却谁也不接的控件（"看起来能按、按下去什么都不发生"）不会让那条变红。本判据把
    /// "今天确实没有动作"的控件**逐个点名** —— 清单外新增一个, 或者修好一个却忘了
    /// 从清单里删掉, 都会红。
    ///
    /// ⭐ **更正（`[ARCH-REC-001]` 接线之后）**：`"transport-record-button"` **已从本清单移出** ——
    /// 它现在**有**点击源（`rec_area` 的 `clicked` ＋ `accessible-action-default` 同一落点，
    /// 见判据 `the_record_button_has_a_click_handler_and_a_wired_record_callback`）。
    /// 移出它正是本判据文档要求的动作（"修好一个 ⇒ 从清单里删掉它"）。
    ///
    /// 清单里每一条的现状（逐条核对过, 不是凭记忆）：
    ///
    /// | 语义 ID（`.slint` 右值） | 为什么今天没有动作 |
    /// | :--- | :--- |
    /// | `"ai-rail-intent-button"` | 意图生成能力不存在: 没有 `callback`、没有宿主写者 |
    /// | `"transport-branch-button"` | 切换活跃分支的 API 一处都不存在（判据 `the_three_transport_document_actions_are_not_wired_today` 已钉） |
    /// | `"transport-commit-button"` | 同上（会话里没有"待提交的编辑"这一状态） |
    /// | `"transport-revert-button"` | 同上（能力存在但未接线） |
    ///
    /// ⭐ **主控静音 / 独奏已从清单移出**（`8 → 6`）：`"mixer-master-mute-button"` /
    /// `"mixer-master-solo-button"` 现在有真实动作（`host::wire_mixer_master_switch`）。
    ///
    /// ⭐ **任务单 `#283` 移出了一条**：`"scene-launch-" + i + "-button"` 现在**真的**
    /// 可点 —— `TouchArea.clicked` 与 `accessible-action-default` 同一落点
    /// `root.scene-launch(scene_index)` ⇒ `host::wire_scene_launch` ⇒
    /// `EngineHost::queue_scene_launch` ⇒ 引擎在量化点 `transport.retune(场景 tempo)`
    /// ＋ `scene_launches_applied += 1`（判据 `tests/scene_launch_wiring.rs`）。
    /// 按本判据文档的要求（"修好一个 ⇒ 从清单里删掉它"），它不再属于"没有动作"的集合。
    ///
    /// ⭐ **本切片移出了一条**（`8 → 7`）：`"sidebar-category-" + i + "-button"` 现在三个
    /// 标签**都**可点 —— `TouchArea.clicked` 与 `accessible-action-default` 同一落点
    /// `root.select-category(category_index)` ⇒ `app.slint` 原样转发 ⇒ 宿主
    /// `host::wire_view_callbacks` 钳到 `0..=2` 后写 `sidebar-category`（纯视图态：
    /// 选中背景 / 文字色 / `accessible-item-selected` 三处读同一个属性）。
    /// 判据 `tests/sidebar_category.rs`。
    #[test]
    fn controls_without_any_click_source_are_exactly_the_known_empty_ones() {
        const CONTROL_ROLES: [&str; 6] = [
            "button",
            "tab",
            "switch",
            "checkbox",
            "radio-button",
            "combobox",
        ];
        let mut found: Vec<String> = Vec::new();
        for (rel, text) in slint_sources() {
            let code = code_only(&text);
            let lines: Vec<&str> = code.lines().collect();
            let blocks = role_blocks(&lines);
            let clicks = arrow_bodies(&code, "clicked", 0);
            for block in &blocks {
                if !CONTROL_ROLES.contains(&block.role.as_str()) {
                    continue;
                }
                let has_own_click = clicks.iter().any(|(line, _)| {
                    let innermost = blocks
                        .iter()
                        .filter(|probe| probe.start < *line && *line <= probe.end)
                        .max_by_key(|probe| probe.start);
                    innermost.is_some_and(|probe| probe.start == block.start)
                });
                if has_own_click {
                    continue;
                }
                let block_text = lines[block.start..=block.end].join("\n");
                let id = block_text
                    .lines()
                    .find_map(|line| line.split_once("accessible-id:").map(|(_, rest)| rest))
                    .map(|rest| rest.trim().trim_end_matches(';').trim().to_owned())
                    .unwrap_or_else(|| {
                        panic!("{rel}: 角色 `{}` 的元素块没有 accessible-id", block.role)
                    });
                found.push(format!("{rel} :: {id}"));
            }
        }
        found.sort();
        found.dedup();
        assert_eq!(
            found,
            KNOWN_ACTIONLESS_CONTROLS
                .iter()
                .map(|entry| (*entry).to_owned())
                .collect::<Vec<String>>(),
            "没有点击源的交互控件集合与登记不符。\n\
             新增一个 ⇒ 要么给它接一个真动作（`accessible-action-default` + `clicked` 同一个落点），\
             要么把它登记进本判据并写清理由;\n\
             修好一个 ⇒ 从 `KNOWN_ACTIONLESS_CONTROLS` 里删掉它（本判据是活文档）。"
        );
    }

    /// [`controls_without_any_click_source_are_exactly_the_known_empty_ones`] 的期望集合。
    ///
    /// 已排序（`BTreeMap` 口径的确定性: 判据的期望值不依赖文件系统迭代顺序）。
    ///
    /// ⭐ **任务单 `#157` 移出了一条**：`device-{i}-bypass-switch` 现在是**真的**
    /// 可点控件（`TouchArea.clicked` + `accessible-action-default` 同一落点
    /// `root.device-bypass-toggle(root.device-ids[device_index])` ⇒ 一次
    /// `Op::SetDeviceBypassed`）。⭐ `#182` 起落点报的是**设备身份**（⛔ 不是下标）。
    /// 按本判据文档的要求（"修好一个 ⇒ 从清单里删掉它"），它不再属于"没有动作"的集合。
    ///
    /// ⭐ **任务单 `#283` 又移出了一条**（`9 → 8`）：`scene-launch-<i>-button` 现在走
    /// `.slint` 的 `clicked` ＋ `accessible-action-default`（同一落点）⇒ 宿主
    /// `host::wire_scene_launch` ⇒ `EngineHost::queue_scene_launch` ⇒ 引擎在量化点
    /// `transport.retune(场景 tempo)`。它与上一条同型：**接上真动作才移出**，
    /// ⛔ 不是"先删表行"。
    /// ⭐ **主控静音 / 独奏移出了两条**（`8 → 6`）：`"mixer-master-mute-button"` /
    /// `"mixer-master-solo-button"` 现在**真的**可点 —— 主控通道条的那两个按钮走
    /// `TouchArea.clicked` ＋ `accessible-action-default`（同一落点）
    /// `root.mixer-master-mute-toggle()` / `root.mixer-master-solo-toggle()` ⇒
    /// `host::wire_mixer_master_switch` 按主总线身份提交一次 `Op::SetTrackMute` /
    /// `Op::SetTrackSolo`。按本判据文档的要求（"修好一个 ⇒ 从清单里删掉它"），
    /// 它们不再属于"没有动作"的集合。
    ///
    /// ⭐ **控制台最大化按钮再移出一条**（`6 → 5`；本行按两线合并后的实测条数改写，
    /// ⛔ 不是照抄单线提交里的 `8 → 7`）：`console-maximize-button` 现在有
    /// `TouchArea.clicked` ＋ `accessible-action-default`（同一落点
    /// `root.console-maximize-toggle()`）⇒ 宿主 `host::wire_view_callbacks` 写
    /// `console-maximized` —— 与键盘 `Cmd/Ctrl+Alt+M` 的
    /// `Action::ToggleConsoleMaximize` 是**同一个**视图态属性。
    ///
    /// ⭐ **资源栏分类标签再移出一条**（`5 → 4`；同样按合并后的实测条数改写，
    /// ⛔ 不是照抄单线提交里的 `8 → 7`）：`"sidebar-category-" + i + "-button"` 现在三个
    /// 标签**都**可点 —— `TouchArea.clicked` 与 `accessible-action-default` 同一落点
    /// `root.select-category(category_index)` ⇒ `app.slint` 原样转发 ⇒ 宿主
    /// `host::wire_view_callbacks` 钳到 `0..=2` 后写 `sidebar-category`（纯视图态：
    /// 选中背景 / 文字色 / `accessible-item-selected` 三处读同一个属性）。
    /// 判据 `tests/sidebar_category.rs`。
    const KNOWN_ACTIONLESS_CONTROLS: [&str; 4] = [
        "app.slint :: \"ai-rail-intent-button\"",
        "transport.slint :: \"transport-branch-button\"",
        "transport.slint :: \"transport-commit-button\"",
        "transport.slint :: \"transport-revert-button\"",
    ];
}
