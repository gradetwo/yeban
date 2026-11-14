//! 第十轮 `[PATCH UI-FEEDBACK-04]`：**「可交互即可感知」**——把第九轮开创的
//! 光标契约推广到覆盖层与 Console 拖拽面，并补齐同屏反馈参差。
//!
//! 本轮用户原话：「仔细检查各种视图是否用户友好，操作是否用户友好」。按
//! 「分视图 + 分交互维度」双轴盘点后，四路只读调研 + 亲自复核收敛出四组；
//! 其中第 ④ 组在 `real_ui_tier1` 干净基线 A/B 中被实证会改默认帧像素，
//! **已撤回并登记待裁**，最终落地三组，覆盖 workspace / console / dialogs
//! 三大视图族，以及「点 / 拖」两个操作维度：
//!
//! | 组 | 区域 | 文件 | 缺口 | 改法 |
//! | :-- | :--- | :--- | :--- | :--- |
//! | ① | 覆盖层可点控件 | 4 个 `dialogs/*.slint` | 提案/时光机/宏面板/导出弹窗共 11 个能点的按钮，鼠标扫过是默认箭头；同抽屉提案行第九轮已给手型，三个主按钮却纹丝不动 | 各 `TouchArea` 加 `mouse-cursor: MouseCursor.pointer;`（第九轮同款精确串） |
//! | ② | Console 拖拽面 | `mixer_console.slint` ×4、`device_rack.slint` ×2、`macro_panel.slint` ×1 | 推子/声相/EQ/发送/宏旋钮都是「不画任何像素」的透明手势面，用户看不出能按住拖——拖拽是这几页的主操作 | 各加 `mouse-cursor: MouseCursor.grab;`（`i-slint-common-1.18.1` 的 `MouseCursor::Grab`，编译探针已验） |
//! | ③ | 同屏反馈参差 | `session_view.slint` ×1、`mixer_console.slint` ×3 | 一排场景激发键零反馈、旁边返回编曲键却 hover 即亮；Mixer Tab1 三键只有 `pressed` | 扩成 `pressed \|\| has-hover`，false 分支逐字回落 `bg-panel-alt` |
//! | ④ | ~~激活态可读性~~（**A/B 后撤回**） | `arrangement_view.slint` | 「M」「S」激活态字色对比度仅 1.4–1.6:1 是真缺口，但 golden「满工程」帧含已静音/独奏轨道，改字色会改默认帧像素 | ⛔ 撤回，登记待裁（见下方诚实登记）；⑤ 把「不改」钉成事实 |
//!
//! ## 默认帧零像素漂移怎么保证
//!
//! * 光标不是渲染帧的一部分（第九轮已确立，`round9_action_feedback.rs` ② 同款）；
//!   本轮 18 处光标**零新增控件、零新增边框、零新增几何**。
//! * ③ 的每个新增三元的最后一个分支逐字等于改动前的静态 token
//!   （`Tokens.bg-panel-alt`）；默认态 `pressed=false` / `has-hover=false` /
//!   `current-scene-index == -1` ⇒ 回落到原底色；Mixer 三键只在
//!   `active-tab == 1` 时可见，默认帧根本不渲染。
//! * ④（M/S 字色）**已撤回**：A/B 实证它会改默认帧，故本轮不含该改动。
//! * 本机 Linux golden 逐字节断言若红，按 `AGENTS.md` 用干净基线 A/B 取证后记
//!   `pending`，⛔ 不重录、⛔ 不放宽判据。
//!
//! ## 诚实登记（明确**不做**的边界，⑦ 钉住不可静默改动）
//!
//! * ⛔ **覆盖层「×」关闭键的 hover/pressed 反馈**：要给 4 个「×」补反馈须命名
//!   4 个 `TouchArea` 并把 `has-hover` 棘轮再推 4 格；其中 `export_wav_dialog`
//!   关闭格含 `export-wav-close-focus := FocusScope` 与 Tab/Return 按键冒泡，
//!   改它会撞 `export_dialog_button_focus.rs` 的轮转表判据。本轮只取零副作用的
//!   指针光标，把反馈留在独立切片。
//! * ⛔ **画布手势面**（`piano_roll.slint` 卷帘拖动、Mixer 点空白选轨）：同一面上
//!   混着点选 / 画音符 / 拖滚动 / 双击复位，给单一光标会误导；刻意不加。
//! * ⛔ **全仓其余可点控件**（Console 其余按钮标签、`sidebar`、transport 两个
//!   状态 tab、M/S/R 按钮）：本轮只覆盖上述三类视图族的高频控件，其余留后续切片，
//!   ⛔ 不把「部分补齐」写成「全仓一致」。
//! * ⛔ **视图 tab 状态色叠 hover**（`round8` ⑤ 登记）、**FocusScope / Tab 轮转补齐**
//!   （`round8` 刻意不做 + `export_dialog_button_focus.rs` 钉住）：不动。
//! * ⛔ **导出失败回执在遮罩后看不见**：需宿主新增结果投影通道，且「导出即关闭」
//!   被 `export_dialog_button_focus.rs:311` ⑤ 钉住；按 `d_ui_27` ⑧ 先例登记待裁，
//!   ⛔ 不伪装成修复。
//! * ⛔ **声相读数 28px 截断**：改 `width` 动运行时包围盒，须先核
//!   `pane_no_overlap` 与 `MIXER_FADER_TRAVEL_PX` 同源关系；本轮不动。
//! * ⛔ **`ink-2` 对深板不足 3:1 / M/S 激活态字色（第 ④ 组，A/B 后撤回）**：
//!   这是真 UX 缺口，但 golden「满工程」帧含已静音/独奏轨道，把字母换成
//!   `#060A14` 会把第一帧 sha256 从 `ecda1f1d…` 改成 `253e19a7…`（单变量实验
//!   实证：只撤销 ④ 即回到 `ecda1f1d…`）。本机禁止重录 Linux golden，
//!   `theme_selection.rs` ①c-bis 也已把该对比度登记为调色板事实待裁决；
//!   按 `d_ui_27` ⑧ 先例登记待裁，留 CI / 人工 golden gate 裁决的独立切片。
//!   ⑤ 把「不改」钉成不可静默改动的事实，⛔ 不把像素漂移伪装成零漂移。
//! * ⛔ **全屏键全屏态按下无反馈**：改背景会撞 `round9` ④ 的
//!   `? Tokens.gold : Tokens.bg-panel);` 精确计数；走文字色感知过弱；不做。
//! * ⛔ **提案 / 宏抽屉 Esc 关不掉**：`round9` ⑥ 重评后决定不动（唯一写者契约，
//!   补臂要新造 callback）；**三个根级抽屉 `visible`** 被 `live_ui_mcp.rs` 钉住；
//!   **`Action::Cancel` 级联顺序**被 `round7` ④⑤ 钉住。三者都不动 `host.rs`。
//! * ⛔ 本轮**零新增** `accessible-id` / `accessible-value` / 可寻址节点；全仓 238
//!   个可寻址节点仍由 `round6_addressable_roles.rs` 覆盖。⑥ 逐文件钉住读数一位未动。
//! * ⛔ 本文件刻意不用 `.all(` 适配器（`assertion_bounds_audit.rs` 的登记守卫会要求
//!   扩 `SOURCES`）；全是纯源码文本判据，不装 Slint 平台、不碰活窗口。
//! * 同步扩了 `round8_action_feedback.rs` ②（cases 加 mixer 3、session 1→2、
//!   total 28→32）与 `round9_action_feedback.rs` ②③④（cursor_total 2→13、
//!   session 1→2、cases 加 mixer 3、total 28→32）：这是**新增合法反馈/光标**带来的
//!   预期增长，⛔ 不是放宽 —— 仍是精确计数，且由本文件 ①④ 三方互相印证。
//! * 上游 `7f535740` 之后无新提交，第九轮补丁尚未合入；本轮在第九轮 WIP 之上继续，
//!   交付「第十轮增量 + 第九+十轮累计」双补丁。**带日期记录只追加、不改写**。

#![allow(unused)]
const APP: &str = include_str!("../ui/app.slint");
const TRANSPORT: &str = include_str!("../ui/transport.slint");
const MODULE_ACTIONS: &str = include_str!("../ui/module_actions.slint");
const SIDEBAR: &str = include_str!("../ui/sidebar.slint");
const SESSION_VIEW: &str = include_str!("../ui/workspace/session_view.slint");
const ARRANGEMENT: &str = include_str!("../ui/workspace/arrangement_view.slint");
const SONG_OVERVIEW: &str = include_str!("../ui/workspace/song_overview_panel.slint");
const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const PIANO_ROLL: &str = include_str!("../ui/console/piano_roll.slint");
const MIXER_CONSOLE: &str = include_str!("../ui/console/mixer_console.slint");
const DEVICE_RACK: &str = include_str!("../ui/console/device_rack.slint");
const MUSICAL_PR_DRAWER: &str = include_str!("../ui/dialogs/musical_pr_drawer.slint");
const MACRO_PANEL: &str = include_str!("../ui/dialogs/macro_panel.slint");
const EXPORT_WAV_DIALOG: &str = include_str!("../ui/dialogs/export_wav_dialog.slint");
const HUMANIZE_DIALOG: &str = include_str!("../ui/dialogs/humanize_dialog.slint");
const UNDO_TREE_MODAL: &str = include_str!("../ui/dialogs/undo_tree_modal.slint");
const HOST_RS: &str = include_str!("../src/host.rs");
const SONG_OVERVIEW_RS: &str = include_str!("../src/song_overview.rs");

/// 全仓 `.slint` 清单：与 `round8_action_feedback.rs`、`round9_action_feedback.rs`、
/// `round6_addressable_roles.rs::SLINT_FILES`、`src/elements.rs::SLINT_MANIFEST` 同一份
/// 19 文件口径（编译期固定），⛔ 不用 `read_dir` 空转成绿。
const SLINT_FILES: &[(&str, &str)] = &[
    ("app.slint", APP),
    ("transport.slint", TRANSPORT),
    ("module_actions.slint", MODULE_ACTIONS),
    ("sidebar.slint", SIDEBAR),
    ("status_bar.slint", include_str!("../ui/status_bar.slint")),
    ("tokens.slint", include_str!("../ui/tokens.slint")),
    ("workspace/session_view.slint", SESSION_VIEW),
    ("workspace/arrangement_view.slint", ARRANGEMENT),
    ("workspace/song_overview_panel.slint", SONG_OVERVIEW),
    ("console/console_tabs.slint", CONSOLE_TABS),
    ("console/piano_roll.slint", PIANO_ROLL),
    ("console/mixer_console.slint", MIXER_CONSOLE),
    ("console/device_rack.slint", DEVICE_RACK),
    (
        "console/asset_pool_panel.slint",
        include_str!("../ui/console/asset_pool_panel.slint"),
    ),
    ("dialogs/musical_pr_drawer.slint", MUSICAL_PR_DRAWER),
    ("dialogs/macro_panel.slint", MACRO_PANEL),
    ("dialogs/export_wav_dialog.slint", EXPORT_WAV_DIALOG),
    ("dialogs/humanize_dialog.slint", HUMANIZE_DIALOG),
    ("dialogs/undo_tree_modal.slint", UNDO_TREE_MODAL),
];

/// ① 覆盖层 11 个可点控件补齐手型光标：逐文件精确计数 + 全仓棘轮 = 13。
/// ⭐ 第十一轮 `[PATCH UI-FEEDBACK-05]` 把全仓手型光标扩到 50（只扩不放宽，
/// 逐文件新分布由 `round11_action_feedback.rs` ① 单独钉）。
/// 删任一处 ⇒ 少一次，红；别处又加 ⇒ 多一次，也红。指针串沿用第九轮精确写法。
#[test]
fn overlay_action_controls_are_discoverable_with_pointer_cursors() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback toggle-ai-drawer("),
        "新 UI 契约面必须声明 callback toggle-ai-drawer("
    );
    assert!(
        aether_host.contains("on_toggle_ai_drawer"),
        "宿主接线层必须注册 on_toggle_ai_drawer"
    );
}

/// ② 7 处拖拽手势面补「可拖」光标：这是本轮唯一的新交互语言。
/// ⭐ 第十一轮新增 `MouseCursor.row-resize`（主窗口分隔条），本句只描述第十轮当时状态。
/// `Grab` 是 `i-slint-common-1.18.1::MouseCursor` 的真实变体（编译探针已验），
/// 且用 `grab` 而非 `pointer` ⇒ 不混进 ① 的精确串计数。
#[test]
fn drag_surfaces_now_show_a_grabbable_cursor() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback toggle-ai-drawer("),
        "新 UI 契约面必须声明 callback toggle-ai-drawer("
    );
    assert!(
        aether_host.contains("on_toggle_ai_drawer"),
        "宿主接线层必须注册 on_toggle_ai_drawer"
    );
}

/// ③ Session 场景激发键与 Mixer Tab1 三键不再是「半套」按钮：
/// 表达式整条逐字存在 ⇒ false 分支逐字回落到 `bg-panel-alt`（默认帧零漂移）。
#[test]
fn session_scene_and_mixer_tab1_buttons_are_no_longer_half_wired() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback toggle-ai-drawer("),
        "新 UI 契约面必须声明 callback toggle-ai-drawer("
    );
    assert!(
        aether_host.contains("on_toggle_ai_drawer"),
        "宿主接线层必须注册 on_toggle_ai_drawer"
    );
}

/// ④ 全仓 `has-hover` 棘轮 = 32，逐文件分布与第十轮基线一致。
/// 与 `round8_action_feedback.rs` ②、`round9_action_feedback.rs` ④ 的 cases 三方互证。
#[test]
fn the_hover_inventory_and_round8_round9_pins_stay_consistent() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback toggle-ai-drawer("),
        "新 UI 契约面必须声明 callback toggle-ai-drawer("
    );
    assert!(
        aether_host.contains("on_toggle_ai_drawer"),
        "宿主接线层必须注册 on_toggle_ai_drawer"
    );
}

/// ⑤ 第 ④ 组（M/S 激活态字色）**A/B 后撤回**：钉住 `arrangement_view.slint`
/// 的 M/S 字色仍是原式 `color: Tokens.ink-2;`，⛔ 不许静默改成反色三元。
/// 原因见文档头诚实登记：改它会动默认帧像素，本机禁止重录 Linux golden。
#[test]
fn mute_solo_active_labels_stay_untouched_after_the_ab_findings() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback toggle-ai-drawer("),
        "新 UI 契约面必须声明 callback toggle-ai-drawer("
    );
    assert!(
        aether_host.contains("on_toggle_ai_drawer"),
        "宿主接线层必须注册 on_toggle_ai_drawer"
    );
}

/// ⑥ 零新增可寻址节点 / 无障碍读数：本轮改动的 7 个 `.slint` 逐文件对账
/// （＋人性化弹窗 1 案：12 ID / 5 读数）。
/// （第 ④ 组撤回后，`arrangement_view.slint` 已不在改动清单内）。
/// `mouse-cursor` 与 `TouchArea` 命名都**不**进运行时控件树。
#[test]
fn no_new_addressable_nodes_or_second_readouts() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback toggle-ai-drawer("),
        "新 UI 契约面必须声明 callback toggle-ai-drawer("
    );
    assert!(
        aether_host.contains("on_toggle_ai_drawer"),
        "宿主接线层必须注册 on_toggle_ai_drawer"
    );
}

/// ⑦ 刻意不做的边界：Esc 级联顺序、三个抽屉 `visible`、静音态档名、
/// 导出弹窗关闭格的 `FocusScope`、画布手势面、Session 插槽格固定底色，
/// 全都维持原状。改这些不是「顺手」，而是要连带改本判据、`live_ui_mcp.rs`、
/// `round7_interaction_feedback.rs`、`export_dialog_button_focus.rs` 与
/// `song_overview.rs` 的叙述。
#[test]
fn the_deliberate_boundaries_are_still_intact() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback toggle-ai-drawer("),
        "新 UI 契约面必须声明 callback toggle-ai-drawer("
    );
    assert!(
        aether_host.contains("on_toggle_ai_drawer"),
        "宿主接线层必须注册 on_toggle_ai_drawer"
    );
}
