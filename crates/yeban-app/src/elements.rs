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
use crate::scene::{CONSOLE_TABS, DEVICE_NAMES, EQ_BANDS, TOOL_NAMES};

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
    /// 侧栏资源 / 控制台标签 / 卷帘工具矩阵 / 设备机架 / 两个对话框 —— 它们要么是
    /// 规范级常量（工具矩阵、EQ 频段），要么还没有对应的模型实体（设备链）。
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
            "AI 编曲提案待审查",
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
            "硬件声卡采样率与性能状态",
            true,
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
        }
        registry.add(
            "session-back-to-arrangement-button",
            ElementKind::Button,
            "workspace/session_view.slint",
            "返回编曲 (Back to Arrangement)",
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

        // ------------------------------------------------------------ 设备机架
        registry.add(
            "device-rack",
            ElementKind::Region,
            "console/device_rack.slint",
            "设备效果链",
            false,
        );
        registry.add(
            "device-rack-eq-curve",
            ElementKind::Image,
            "console/device_rack.slint",
            "4 段 EQ 频响曲线 (占位)",
            true,
        );
        for band in EQ_BANDS {
            registry.add(
                &format!("device-rack-eq-{band}-knob"),
                ElementKind::Slider,
                "console/device_rack.slint",
                &format!("{band} 频段增益"),
                false,
            );
        }
        for (device_index, name) in DEVICE_NAMES.iter().enumerate() {
            registry.add(
                &format!("device-{device_index}-card"),
                ElementKind::ListItem,
                "console/device_rack.slint",
                &format!("设备 {name}"),
                false,
            );
            registry.add(
                &format!("device-{device_index}-bypass-switch"),
                ElementKind::Switch,
                "console/device_rack.slint",
                &format!("旁通 {name}"),
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
/// （侧栏资源库 / 设备机架 / 两个对话框 / 状态栏 / 走带）的静态标签是**已知的未实现项**
/// （见 `docs/ledger/app-binding-notes.md`）—— 对它们做全局断言会假红，
/// 例如侧栏里有 `Sub Bass 低频`。
///
/// `mixer-` 是 app-mixer 工作线新加的一族：调音台的**通道条**（`track-{i}-*`）本来就在
/// `track-` 族里，但**主控通道条**（`mixer-master-*`）不共享那个前缀 —— 它的名字 / 音量 /
/// 声相 / 静音 / 独奏 / 色标同样来自工程，所以必须有自己的族，否则"主控显示的是不是工程"
/// 就没有机械检查（`ui/console/mixer_console.slint` 的 `mixer-master-strip` 子树）。
pub const MODEL_DRIVEN_FAMILIES: [&str; 9] = [
    "track-",
    "section-",
    "clip-",
    "note-",
    "velocity-",
    "session-track-",
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
    const SLINT_MANIFEST: [&str; 13] = [
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
        "dialogs/musical_pr_drawer.slint",
        "dialogs/undo_tree_modal.slint",
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

        let registry = registry();

        // 方向 1: 注册表 → .slint
        for meta in registry.iter() {
            assert!(
                patterns.iter().any(|(_, segments, is_template)| {
                    id_matches_pattern(segments, *is_template, &meta.id)
                }),
                "注册表里的 `{}` 在 ui/ 的任何 .slint 里都找不到对应的 accessible-id 模板",
                meta.id
            );
        }

        // 方向 2: .slint → 注册表
        for (file, segments, is_template) in &patterns {
            assert!(
                registry
                    .iter()
                    .any(|meta| id_matches_pattern(segments, *is_template, &meta.id)),
                "{file} 里的 accessible-id 模板 {segments:?} (template={is_template}) 在注册表里一个成员都没有"
            );
        }
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
            "clip-01J8Z5Q0R7K3M9X2V4B6N8P1F9-header",
            "note-01J8Z5Q0R7K3M9X2V4B6N8P1A2-rect",
            "velocity-0-bar",
            "session-track-0-header",
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
        name.starts_with("track-") || name.starts_with("master-")
    }

    /// 读出 `Component { … }` 块里**第一层**的属性赋值名（跳过嵌套元素与回调）。
    ///
    /// 只做一层缩进无关的花括号深度计数：本仓库这几个实例化块里没有同名嵌套组件，
    /// 因此"深度 1 上的 `name:`"就是该组件的属性/回调赋值。
    fn assigned_properties(source: &str, component: &str) -> Vec<String> {
        let header = format!("{component} {{");
        let mut out = Vec::new();
        let mut inside = false;
        let mut depth = 0_i32;
        for line in source.lines() {
            let trimmed = line.trim();
            if !inside {
                if trimmed == header {
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
            .expect("读 mixer_console.slint");

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
}
