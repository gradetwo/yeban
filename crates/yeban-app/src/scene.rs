//! 演示场景数据 —— UI 骨架的**假数据源**。
//!
//! 规范来源 (Normative):
//! - `[UI-GRID-002]` UI/UX 规范 §1.2: ≥1920 全展开 / 1366–1919 折叠 —— 断点判定在
//!   [`DemoScene::is_compact`] 里落成可测的纯函数。
//! - `[UI-TEST-001]` §12.2: `note-{ulid}` / `clip-{ulid}` 里的 ULID 必须真实合法
//!   (26 字符 Crockford Base32), 否则语义 ID 就成了编造的字符串。
//!
//! ## 这不是模型层
//!
//! 这里的常量是**一次性演示数据**, 不是 `yeban-model` 的 `YebanProjectV1`, 也不是
//! `yeban-engine` 的 `EngineSnapshot`。用它的唯一目的是: 让 UI 骨架能离屏渲染出
//! "有内容"的画面, 让元素注册表有真实的成员可以断言。
//!
//! ## 与 `.slint` 的字面量重复 (known debt)
//!
//! `ui/workspace/arrangement_view.slint` 与 `ui/console/piano_roll.slint` 里各自内联了
//! 一份同样的 ULID 数组 / 轨道名数组 —— 因为 Slint 的数组属性需要**默认值**才能在
//! 没有宿主注入时独立渲染。这份重复是临时的: `yeban-model` 落地后改成 Rust 侧
//! `ModelRc` 单向注入, `.slint` 里只留空数组。见 `docs/ledger/ui-shell-notes.md` 的 pending。

/// 演示轨道数。与 `ui/` 下所有 `for … in 6` 循环的边界一致。
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

/// 轨道名 (演示数据)。
pub const TRACK_NAMES: [&str; TRACK_COUNT] = ["鼓", "贝斯", "铺底", "主音", "弦乐", "打击"];

/// 场景名 (演示数据)。
pub const SCENE_NAMES: [&str; SCENE_COUNT] = ["Intro", "Verse", "Chorus", "Drop"];

/// 章节卡片名 (演示数据)。
pub const SECTION_NAMES: [&str; SCENE_COUNT] = ["Intro", "Verse", "Chorus", "Outro"];

/// 音符实体的演示 ULID。
///
/// 逐字必须与 `ui/console/piano_roll.slint` 的 `note-ulids` 默认值一致 ——
/// [`crate::elements::ElementRegistry`] 就是用这一组常量生成 `note-{ulid}-rect` 的。
pub const NOTE_ULIDS: [&str; NOTE_COUNT] = [
    "01J8Z5Q0R7K3M9X2V4B6N8P1A2",
    "01J8Z5Q0R7K3M9X2V4B6N8P1A3",
    "01J8Z5Q0R7K3M9X2V4B6N8P1B0",
    "01J8Z5Q0R7K3M9X2V4B6N8P1C7",
    "01J8Z5Q0R7K3M9X2V4B6N8P1D4",
    "01J8Z5Q0R7K3M9X2V4B6N8P1E1",
];

/// 剪辑实体的演示 ULID。必须与 `ui/workspace/arrangement_view.slint` 的 `clip-ulids` 一致。
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

/// 设备链里的设备卡名 (演示数据)。
pub const DEVICE_NAMES: [&str; 3] = ["EQ 4 段", "Compressor", "Space Reverb"];

/// 演出用的推子位置 (0.0–1.0 归一化) —— 只喂给 `.slint` 的 `levels` 默认值。
pub const FADER_LEVELS: [f32; TRACK_COUNT] = [0.72, 0.55, 0.48, 0.62, 0.35, 0.40];

/// 推子分贝显示值 (演示数据; 真实换算归模型层)。
pub const FADER_DB_LABELS: [&str; TRACK_COUNT] = ["-3.2", "-6.0", "-8.4", "-4.8", "-12.0", "-10.6"];

/// 一个控制台标签: `name` 进语义 ID, `label` 进可读标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleTab {
    /// 语义 ID 片段, 必须是小写连字符串 (`piano-roll` / `mixer` / `devices`)。
    pub name: &'static str,
    /// 人类可读标签 (进 `accessible-label`)。
    pub label: &'static str,
}

/// UI 骨架的演示数据集合。
#[derive(Debug, Clone, Copy)]
pub struct DemoScene {
    /// 窗口标题。
    pub title: &'static str,
    /// 顶栏时间码显示值。
    pub timecode: &'static str,
    /// 顶栏 BPM 显示值 (字符串: 格式化是模型层的活)。
    pub bpm_display: &'static str,
    /// 顶栏分支名。
    pub branch_name: &'static str,
    /// 启动时是否直接进 Arrangement 视图 (规范的默认视图是线性编曲)。
    pub arrangement_by_default: bool,
    /// 演示视口宽度 (逻辑像素) —— 1920 落在 `[UI-GRID-002]` 的全展开档。
    pub viewport_width: u32,
    /// 演示视口高度 (逻辑像素)。
    pub viewport_height: u32,
}

impl DemoScene {
    /// 构造演示场景。全部数值都是常量, 因此**没有任何 I/O 与分配**, 可随时调用。
    #[must_use]
    pub const fn demo() -> Self {
        Self {
            title: "夜半 Yeban",
            timecode: "001.01.000",
            bpm_display: "120.00",
            branch_name: "main",
            arrangement_by_default: true,
            viewport_width: BREAKPOINT_FULL_HD,
            viewport_height: 1080,
        }
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
/// 存在的意义: 演示 ULID 是**手写常量**, 手写就会写错。把校验写成可测函数, 而不是
/// 让编造的字符串混进语义 ID —— 那会让 `note-{ulid}-rect` 这个约定形同虚设。
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
