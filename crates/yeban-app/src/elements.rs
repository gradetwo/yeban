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
/// （侧栏资源库 / 混音台 / 设备机架 / 两个对话框）的静态标签是**已知的未实现项**
/// （见 `docs/ledger/app-binding-notes.md`）—— 对它们做全局断言会假红，
/// 例如侧栏里有 `Sub Bass 低频`、混音台通道条仍用演示轨道名。
pub const MODEL_DRIVEN_FAMILIES: [&str; 8] = [
    "track-",
    "section-",
    "clip-",
    "note-",
    "velocity-",
    "session-track-",
    "scene-launch-",
    "slot-",
];

/// 该语义 ID 是否属于 [`MODEL_DRIVEN_FAMILIES`]（即"应当携带工程数据"的部件）。
#[must_use]
pub fn is_model_driven_family(id: &str) -> bool {
    MODEL_DRIVEN_FAMILIES
        .iter()
        .any(|prefix| id.starts_with(prefix))
}

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
        ] {
            assert!(registry.contains(id), "判据清单里的 `{id}` 不在注册表里");
            assert!(is_model_driven_family(id), "`{id}` 应当被判为投影驱动");
        }
        for id in [
            "sidebar-item-0",
            "sidebar-category-0-button",
            "mixer-console",
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
}
