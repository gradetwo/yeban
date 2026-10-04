//! 演示场景数据 —— 界面外壳的**夹具**，现在由 [`crate::bridge`] 的投影层产出。
//!
//! 规范来源 (Normative):
//! - `[UI-GRID-002]` UI/UX 规范 §1.2: ≥1920 全展开 / 1366–1919 折叠 —— 断点判定在
//!   [`DemoScene::is_compact`] 里落成可测的纯函数。
//! - `[UI-TEST-001]` §12.2: `note-{ulid}` / `clip-{ulid}` 里的 ULID 必须真实合法
//!   (26 字符 Crockford Base32), 否则语义 ID 就成了编造的字符串。
//! - `[MODEL-AST-002]` `YebanProjectV1` 是唯一权威工程结构。
//! - `[MODEL-ISO-001]` 三层状态物理隔离: **走带位置**（`timecode`）与**当前分支**
//!   (`branch_name`) 属于会话运行态 / 提交图谱，**不在** `YebanProjectV1` 里。
//!   本模块因此明确把它们标成占位常量，而不是假装它们是工程字段。
//!
//! ## 这一版与上一版的区别（这是本工作线的核心变更）
//!
//! 上一版里 [`TRACK_NAMES`] / [`NOTE_ULIDS`] / [`CLIP_ULIDS`] / [`SECTION_NAMES`] 是
//! **界面的唯一数据源** —— 一组 `&'static str`。现在它们只是[**判据锚点**]：
//! 真正的数据源是 [`crate::bridge::demo_project`] 返回的一个 `YebanProjectV1`，
//! 由 [`DemoScene::demo`] → [`DemoScene::from_view`] 投影出来。
//!
//! `bridge::tests::demo_projection_reproduces_the_scene_constants` 逐字钉住
//! "常量的值 == 演示工程投影出来的值"；`elements.rs` 的注册表也改为由投影构造。
//! 于是"演示数据"与"真实工程数据"走的是**同一条代码路径** ——
//! 换成 `yeban_model::samples::filled_project()` 时没有任何分支切换。
//!
//! ## 与 `.slint` 的字面量重复：已消除（本线）
//!
//! 上一版 `ui/workspace/arrangement_view.slint` 与 `ui/piano_roll.slint` 里各自内联了
//! 一份同样的 ULID / 轨道名数组作为**属性默认值**（Slint 数组属性需要默认值才能独立渲染）。
//! 本线把 arrangement / session 两处的默认值改成**空数组**，由 Rust 侧经
//! `src/host.rs` 单向注入 —— 重复只剩 `piano_roll.slint` 一处（见 notes 的未实现项）。

use crate::bridge::{BridgeError, ViewState};

/// 演示轨道数。与演示工程 `demo_project()` 的非主总线轨道数一致（有判据钉住）。
pub const TRACK_COUNT: usize = 6;

/// 演示场景数 (Session View 的行数)。
pub const SCENE_COUNT: usize = 4;

/// 演示音符数。
pub const NOTE_COUNT: usize = 6;

/// 演示剪辑数。
pub const CLIP_COUNT: usize = 3;

/// `[UI-GRID-002]` 全展开断点: ≥1920 逻辑像素宽 → 左栏 240px + 右栏常驻 280px。
pub const BREAKPOINT_FULL_HD: u32 = 1920;

/// `[UI-GRID-002]` 折叠区间下界: 1366–1919 逻辑像素宽 → 左栏 36px 图标导轨 + 右栏变抽屉。
pub const BREAKPOINT_MIN: u32 = 1366;

/// 演示轨道名 —— **现在只是判据锚点**：值由 `bridge::demo_project()` 的投影复现。
pub const TRACK_NAMES: [&str; TRACK_COUNT] = ["鼓", "贝斯", "铺底", "主音", "弦乐", "打击"];

/// 场景名 —— 同上（判据锚点，事实源是演示工程的 `scenes`）。
pub const SCENE_NAMES: [&str; SCENE_COUNT] = ["Intro", "Verse", "Chorus", "Drop"];

/// 章节卡片名 —— 同上（事实源是演示工程的 `sections`）。
pub const SECTION_NAMES: [&str; SCENE_COUNT] = ["Intro", "Verse", "Chorus", "Outro"];

/// 音符实体的演示 ULID —— 事实源是演示工程 MIDI 片段里的 `MidiNote::id`。
///
/// [`crate::elements::ElementRegistry`] 用它们生成 `note-{ulid}-rect`，而注册表现在由
/// 投影构造，因此这组常量与工程里的音符身份**逐字对账**。
pub const NOTE_ULIDS: [&str; NOTE_COUNT] = [
    "01J8Z5Q0R7K3M9X2V4B6N8P1A2",
    "01J8Z5Q0R7K3M9X2V4B6N8P1A3",
    "01J8Z5Q0R7K3M9X2V4B6N8P1B0",
    "01J8Z5Q0R7K3M9X2V4B6N8P1C7",
    "01J8Z5Q0R7K3M9X2V4B6N8P1D4",
    "01J8Z5Q0R7K3M9X2V4B6N8P1E1",
];

/// 剪辑实体的演示 ULID —— 事实源是演示工程里 `ClipPlacement::id`（时间轴摆放身份）。
pub const CLIP_ULIDS: [&str; CLIP_COUNT] = [
    "01J8Z5Q0R7K3M9X2V4B6N8P1F9",
    "01J8Z5Q0R7K3M9X2V4B6N8P1G6",
    "01J8Z5Q0R7K3M9X2V4B6N8P1H3",
];

/// 底部控制台的三个标签。
///
/// `name` 就是语义 ID 里的 `{tab_name}` —— `[UI-TEST-001]` 要求它是稳定的小写连字符串,
/// 因此这里同时是「Rust 侧注册表」与「`.slint` 侧 `tab-names` 默认值」的事实源。
pub const CONSOLE_TABS: [ConsoleTab; 3] = [
    ConsoleTab {
        name: "piano-roll",
        label: "钢琴卷帘",
    },
    ConsoleTab {
        name: "mixer",
        label: "调音台",
    },
    ConsoleTab {
        name: "devices",
        label: "设备效果链",
    },
];

/// 卷帘工具矩阵 (`[UI-NOTE-003]` 规范 §3.3): 1 选择 / 2 铅笔 / 3 剪刀 / 4 力度 / 5 橡皮擦。
pub const TOOL_NAMES: [&str; 5] = ["select", "pencil", "knife", "velocity", "eraser"];

/// 4 段 EQ 的频段名 (`[UI-NOTE-004]` 规范 §5.1)。
pub const EQ_BANDS: [&str; 4] = ["low", "low-mid", "high-mid", "high"];

/// 设备链里的设备卡名。
///
/// **已知债**：它**没有**接到 `TrackV3::devices` 上（设备机架尚未由模型驱动，
/// 见 `docs/ledger/app-binding-notes.md` 的未实现项）。
pub const DEVICE_NAMES: [&str; 3] = ["EQ 4 段", "Compressor", "Space Reverb"];

/// 演出用的推子位置 (0.0–1.0 归一化)。
///
/// **这是会话运行态而不是工程状态**：真实数值来自 `[ARCH-UI-002]` 的无锁 Meter SPSC
/// （由 `yeban-engine` 每 60Hz 推送），不落在 `YebanProjectV1` 里。演示期先用常量。
pub const FADER_LEVELS: [f32; TRACK_COUNT] = [0.72, 0.55, 0.48, 0.62, 0.35, 0.40];

/// 推子分贝显示值 —— 事实源是演示工程每轨的 `TrackV3::volume_db`（有判据逐字对账）。
pub const FADER_DB_LABELS: [&str; TRACK_COUNT] = ["-3.2", "-6.0", "-8.4", "-4.8", "-12.0", "-10.6"];

/// `[MODEL-ISO-001]` 走带位置占位：它属于**会话运行态**，不是 `YebanProjectV1` 的字段。
///
/// 真实值来自 `yeban-engine` 的 `EngineSnapshot`（960 PPQ 整数 tick → 时间码），
/// 本工作线无权发明那个映射，因此保留字面量并显式登记。
pub const SESSION_TIMECODE: &str = "001.01.000";

/// `[MODEL-ISO-001]` 当前分支占位：分支头住在 `yeban-model::commit::CommitGraph`，
/// 不在工程文档里。接上提交图谱之前保留 `main`。
pub const SESSION_BRANCH_NAME: &str = "main";

/// 一个控制台标签: `name` 进语义 ID, `label` 进可读标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleTab {
    /// 语义 ID 片段, 必须是小写连字符串 (`piano-roll` / `mixer` / `devices`)。
    pub name: &'static str,
    /// 人类可读标签 (进 `accessible-label`)。
    pub label: &'static str,
}

/// 界面外壳的场景参数（窗口标题 / 走带显示 / 视口尺寸）。
///
/// 字段拆成两类，**不混用**（`[MODEL-ISO-001]`）：
/// - **工程派生**：`title` / `bpm_display` —— 由 [`Self::from_view`] 从投影取；
/// - **会话运行态 / 本机视口**：`timecode` / `branch_name` / `viewport_*` —— 占位或本机探测。
#[derive(Debug, Clone, PartialEq)]
pub struct DemoScene {
    /// 窗口标题（来自 `YebanProjectV1::title`）。
    pub title: String,
    /// 顶栏时间码显示值（会话运行态占位，见 [`SESSION_TIMECODE`]）。
    pub timecode: String,
    /// 顶栏 BPM 显示值（来自工程 `bpm`，格式化由投影层完成）。
    pub bpm_display: String,
    /// 顶栏分支名（提交图谱占位，见 [`SESSION_BRANCH_NAME`]）。
    pub branch_name: String,
    /// 启动时是否直接进 Arrangement 视图 (规范的默认视图是线性编曲)。
    pub arrangement_by_default: bool,
    /// 演示视口宽度 (逻辑像素) —— 1920 落在 `[UI-GRID-002]` 的全展开档。
    pub viewport_width: u32,
    /// 演示视口高度 (逻辑像素)。
    pub viewport_height: u32,
}

impl DemoScene {
    /// 构造演示场景：**由投影层产出**（`bridge::demo_project()` → `ViewState`）。
    ///
    /// 无 I/O、无分配以外的副作用；夹具非法时 panic（属编程错误）。
    ///
    /// # Panics
    ///
    /// 仅当 [`crate::bridge::demo_project`] 被改成非法工程时 panic。
    #[must_use]
    pub fn demo() -> Self {
        Self::from_view(&ViewState::demo())
    }

    /// 由一个**投影结果**构造外壳场景。这是界面侧唯一的取数口径。
    #[must_use]
    pub fn from_view(view: &ViewState) -> Self {
        Self {
            title: view.title.clone(),
            timecode: SESSION_TIMECODE.to_owned(),
            bpm_display: view.bpm_display.clone(),
            branch_name: SESSION_BRANCH_NAME.to_owned(),
            arrangement_by_default: true,
            viewport_width: BREAKPOINT_FULL_HD,
            viewport_height: 1080,
        }
    }

    /// 由一个**真实工程**构造外壳场景（真实路径：`from_project(&project)`）。
    ///
    /// # Errors
    ///
    /// 投影失败（越界 tick / 非法拍号）时冒泡 [`BridgeError`]。
    pub fn from_project(
        project: &yeban_model::project::YebanProjectV1,
    ) -> Result<Self, BridgeError> {
        Ok(Self::from_view(&ViewState::from_project(project)?))
    }

    /// `[UI-GRID-002]` 响应式断点判定: 视口宽度是否落在「折叠」档。
    ///
    /// 规范原文:
    /// - `≥ 1920×1080`: 全展开 (左栏 240px, 右栏常驻 280px);
    /// - `1366×768 ～ 1919×1079`: 左栏默认收起为 36px 图标导轨, 右栏自动折叠为抽屉。
    ///
    /// 规范没有规定 `< 1366` 时怎么办。本实现取**保守解**: 仍然折叠 (与 1366–1919 同档),
    /// 因为"更窄的屏幕要求更多折叠"是唯一不会破坏可用性的方向。这一条属于规范缺口,
    /// 已登记在 `docs/ledger/ui-shell-notes.md` 的 needs 里等人类裁决。
    #[must_use]
    pub const fn is_compact(viewport_width: u32) -> bool {
        viewport_width < BREAKPOINT_FULL_HD
    }

    /// 演示场景自身的断点判定结果 (main.rs 用它初始化 `MainWindow::compact`)。
    #[must_use]
    pub const fn compact(&self) -> bool {
        Self::is_compact(self.viewport_width)
    }
}

impl Default for DemoScene {
    fn default() -> Self {
        Self::demo()
    }
}

/// 校验一个字符串是不是**真正的** ULID 文本形式。
///
/// 规则来自 `MODEL-AST-001`/ADR-0001 D6 的实测结论: 26 个字符, 字母表是 Crockford Base32
/// (`0123456789ABCDEFGHJKMNPQRSTVWXYZ`, 大小写不敏感, 不含 `I`/`L`/`O`/`U`)。
///
/// 存在的意义: 语义 ID 里的 `{ulid}` 段来自**实体身份**（现在由投影给出），而身份是运行时
/// 生成的。把校验写成可测函数，而不是让编造的字符串混进语义 ID —— 那会让
/// `note-{ulid}-rect` 这个约定形同虚设。
#[must_use]
pub fn is_ulid_text(value: &str) -> bool {
    if value.len() != 26 {
        return false;
    }
    value.bytes().all(|byte| {
        let upper = byte.to_ascii_uppercase();
        upper.is_ascii_digit()
            || (upper.is_ascii_uppercase() && !matches!(upper, b'I' | b'L' | b'O' | b'U'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_ulids_are_real_ulid_text() {
        for ulid in NOTE_ULIDS.iter().chain(CLIP_ULIDS.iter()) {
            assert!(
                is_ulid_text(ulid),
                "演示 ULID `{ulid}` 不是合法的 26 字符 Crockford Base32"
            );
        }
    }

    #[test]
    fn ulid_checker_rejects_the_obvious_wrong_shapes() {
        // 25 / 27 字符
        assert!(!is_ulid_text("01J8Z5Q0R7K3M9X2V4B6N8P1A"));
        assert!(!is_ulid_text("01J8Z5Q0R7K3M9X2V4B6N8P1A22"));
        // 含 Crockford 排除字符 I / L / O / U
        assert!(!is_ulid_text("01J8Z5Q0R7K3M9X2V4B6N8P1AI"));
        assert!(!is_ulid_text("01J8Z5Q0R7K3M9X2V4B6N8P1AL"));
        assert!(!is_ulid_text("01J8Z5Q0R7K3M9X2V4B6N8P1AO"));
        assert!(!is_ulid_text("01J8Z5Q0R7K3M9X2V4B6N8P1AU"));
        // 非 ASCII 数字 / 连字符
        assert!(!is_ulid_text("01J8Z5Q0R7K3M9X2V4B6N8P1-2"));
        // 小写是允许的 (规范要求大小写不敏感)
        assert!(is_ulid_text("01j8z5q0r7k3m9x2v4b6n8p1a2"));
    }

    #[test]
    fn ulids_are_distinct() {
        let mut all: Vec<&str> = NOTE_ULIDS
            .iter()
            .chain(CLIP_ULIDS.iter())
            .copied()
            .collect();
        all.sort_unstable();
        let before = all.len();
        all.dedup();
        assert_eq!(before, all.len(), "演示 ULID 有重复 —— 元素 ID 会因此撞车");
    }

    /// `[UI-GRID-002]` 断点判据。三档边界逐个钉死, 不靠"看起来对"。
    #[test]
    fn compact_breakpoint_matches_the_spec_table() {
        // ≥1920: 全展开
        assert!(!DemoScene::is_compact(1920));
        assert!(!DemoScene::is_compact(2560));
        assert!(!DemoScene::is_compact(3840));
        // 1366–1919: 折叠
        assert!(DemoScene::is_compact(1919));
        assert!(DemoScene::is_compact(1600));
        assert!(DemoScene::is_compact(1366));
        // <1366: 规范未定义, 实现取保守解 (仍折叠) —— 见函数文档
        assert!(DemoScene::is_compact(1365));
        assert!(DemoScene::is_compact(1024));
    }

    #[test]
    fn demo_scene_starts_expanded() {
        let scene = DemoScene::demo();
        assert_eq!(scene.viewport_width, BREAKPOINT_FULL_HD);
        assert!(!scene.compact(), "1920 宽的演示视口必须落在全展开档");
        assert!(scene.arrangement_by_default);
    }

    /// 判据: 外壳场景的**工程派生字段**确实来自投影（而不是另一份常量）。
    #[test]
    fn demo_scene_is_projected_from_the_demo_project() {
        let view = ViewState::demo();
        let scene = DemoScene::demo();
        assert_eq!(scene.title, view.title);
        assert_eq!(scene.bpm_display, view.bpm_display);
        assert_eq!(scene.title, "夜半 Yeban");
        assert_eq!(scene.bpm_display, "120.00");
        // 会话运行态字段明确是占位（[MODEL-ISO-001]），不得被当成工程字段。
        assert_eq!(scene.timecode, SESSION_TIMECODE);
        assert_eq!(scene.branch_name, SESSION_BRANCH_NAME);
        // `from_view` 是同一个口径：换一个工程 ⇒ 换一个标题 / BPM。
        let filled = yeban_model::samples::filled_project();
        let filled_scene = DemoScene::from_project(&filled).expect("投影");
        assert_eq!(filled_scene.title, "Yeban Model Core Sample");
        assert_eq!(filled_scene.bpm_display, "128.00");
        assert_ne!(filled_scene.title, scene.title);
    }

    /// 判据: 非法工程经 `from_project` 返回 `Err`，而不是 panic。
    #[test]
    fn from_project_propagates_projection_errors() {
        let mut project = yeban_model::samples::default_project();
        project.time_signature.denominator = 0;
        assert_eq!(
            DemoScene::from_project(&project),
            Err(BridgeError::ZeroTimeSignatureDenominator)
        );
    }

    #[test]
    fn console_tab_names_are_well_formed() {
        for tab in CONSOLE_TABS {
            assert!(!tab.name.is_empty());
            assert!(
                tab.name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "控制台标签名 `{}` 必须是小写连字符串",
                tab.name
            );
            assert!(!tab.label.is_empty());
        }
    }
}
