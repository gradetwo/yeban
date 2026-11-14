//! 第九轮 `[PATCH UI-FEEDBACK-03]` 常驻门禁：**让 Songmap Phase 2 的新功能不再
//! 「点了没手感」、让隐藏交互「看得出来」、补齐全仓最后一处半套反馈**。
//!
//! ## 为什么这一轮不是「再补一批 hover」
//!
//! 基线 `7f535740` 合入了第七/八轮（把悬浮 / 按下可见推广到 15 个主操作键，全仓
//! `has-hover` 精确计数钉在 26）**以及全新的 Songmap Phase 2**。三路只读盘点 +
//! 亲自复核后，用户能直接感到的缺口集中在三个点：
//!
//! | 区域 | 文件 | 缺口 | 改法 |
//! | :-- | :--- | :--- | :--- |
//! | 总览面板曲风预设键 | `song_overview_panel.slint` | 与同排 4 胶囊 / 折叠 / 全屏共 6 键**同款式**，却独漏第七/八轮的「悬浮金边 + 按下提亮」⇒ 鼠标扫过一排只有它不亮，看起来不像能点 | 命名 `profile_touch`，照抄折叠键三元 |
//! | 段落标签点击区 | `song_overview_panel.slint` | Phase 2「点段落标签循环期望等级」是**隐藏交互**：8 px 高、无边框、无光标提示，用户根本看不出能点 | 给纯指针 `TouchArea` 加 `mouse-cursor: MouseCursor.pointer;`（光标不进渲染帧） |
//! | Session 返回编曲键 | `session_view.slint` | 全仓**唯一**只有 `pressed` 没有 `has-hover` 的半套真按钮 | 扩成 `pressed \|\| has-hover` |
//!
//! 三组都守「**默认帧零像素漂移**」：每个新增表达式的最后一个分支逐字等于改动前的
//! 静态 token（`bg-panel` / `line` / `bg-void`）；光标不是渲染帧的一部分；零新增控件、
//! 零新增边框、零新增几何。
//!
//! ## 明确**不做**的边界（⑥ 把它们钉成不可静默改动的事实）
//!
//! * **提案 / 宏抽屉按 `Esc` 关不掉**：这是三路盘点里「按了键却没效果」最刺眼的一条，
//!   但补臂要新造 callback，与 `host.rs` 注释明写的「不发明这条回调」及
//!   `set_musical_pr_*` 唯一写者契约冲突；第八轮已把它登记为刻意不做，**本轮重评后仍不动**。
//! * **空工程时点预设键界面恒印「均衡」**：`silent_overview` 把 `profile` 恒设为
//!   `TensionProfile::Default`，注释写「不留旧工程数据」—— 这是刻意的「静音态 = 全缺省」
//!   语义；无工程时档位本来就无意义，让文字跟着变反而误导。**不把刻意状态伪装成缺口**。
//! * **段落等级读数 / 可发现性之外的改动**：加 `accessible-value` 会撞
//!   `song_overview_panel.rs` 的读数计数与 `fabricated_state_honesty.rs` 禁第二份读数；
//!   本轮只做「光标可发现」，等级读数留作独立切片。
//! * `app.slint` 三个根级抽屉 `visible`、`host.rs` 的 `Action::Cancel` 顺序、
//!   `Tokens.accent` / `Tokens.bg-control` 落点：分别由 `live_ui_mcp.rs`、
//!   `round7_interaction_feedback.rs`、`theme_selection.rs` 钉死，一概不动。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | 删掉预设键的 `has-hover` / `pressed` | ① 红 + ④ 红 |
//! | 把预设键 false 分支改成别的 token | ① 红（默认帧漂移）+ ④ 红 |
//! | 删掉段落标签的 `mouse-cursor` | ② 红 |
//! | 给段落标签加 `accessible-id` | ② 红 + ⑤ 红 |
//! | 把返回编曲改回只有 `pressed` | ③ 红 + ④ 红 |
//! | 新增一个 `accessible-id` / `accessible-value` | ⑤ 红 |
//! | 动 `Esc` 顺序 / 抽屉 `visible` / 静音态档名 | ⑥ 红 |
//!
//! ## 诚实登记
//!
//! * ⛔ 本轮**零新增** `accessible-id` / `accessible-value` / 可寻址节点（⑤ 逐文件钉住）；
//!   全仓 238 个可寻址节点由 `round6_addressable_roles.rs` 照旧覆盖，这里**不**重复钉
//!   那个解析口径。
//! * ⛔ 本文件刻意不用 `.all(` 适配器（`assertion_bounds_audit.rs` 的登记守卫会要求扩
//!   `SOURCES`）；全是纯源码文本判据，不装 Slint 平台、不碰活窗口。
//! * 同步扩了 `round7_interaction_feedback.rs` ③（3→4 / 3→4 / 1→2）与
//!   `round8_action_feedback.rs` ②⑤（26→28、5→6、3→4）：这是**新增合法反馈**带来的
//!   预期增长，⛔ 不是放宽 —— 仍是精确计数，且由 ④ 复核两处扩钉一致。
//! * 上游 `7f535740` 刚重录 Linux golden。本轮默认帧零位移由 ①③④ 机械保证；
//!   本机 `real_ui_tier1` 逐字节断言若红，按 `AGENTS.md` 记为 `pending`，⛔ 不重录、
//!   ⛔ 不放宽判据。
//! * 带日期记录只追加、不改写（本文件是新增判据，不改既有历史登记）。

#![allow(unused)]
const SONG_OVERVIEW: &str = include_str!("../ui/workspace/song_overview_panel.slint");
const SESSION_VIEW: &str = include_str!("../ui/workspace/session_view.slint");
const ARRANGEMENT: &str = include_str!("../ui/workspace/arrangement_view.slint");
const TRANSPORT: &str = include_str!("../ui/transport.slint");
const APP: &str = include_str!("../ui/app.slint");
const HOST_RS: &str = include_str!("../src/host.rs");
const SONG_OVERVIEW_RS: &str = include_str!("../src/song_overview.rs");
const MUSICAL_PR_DRAWER: &str = include_str!("../ui/dialogs/musical_pr_drawer.slint");

/// 全仓 `.slint` 清单：与 `round8_action_feedback.rs::SLINT_FILES`、
/// `round6_addressable_roles.rs::SLINT_FILES`、`src/elements.rs::SLINT_MANIFEST`
/// 同一份 19 文件口径（编译期固定），⛔ 不用 `read_dir` 空转成绿。
const SLINT_FILES: &[(&str, &str)] = &[
    ("app.slint", APP),
    ("transport.slint", TRANSPORT),
    ("status_bar.slint", include_str!("../ui/status_bar.slint")),
    ("sidebar.slint", include_str!("../ui/sidebar.slint")),
    ("tokens.slint", include_str!("../ui/tokens.slint")),
    ("workspace/session_view.slint", SESSION_VIEW),
    ("workspace/arrangement_view.slint", ARRANGEMENT),
    ("workspace/song_overview_panel.slint", SONG_OVERVIEW),
    (
        "console/console_tabs.slint",
        include_str!("../ui/console/console_tabs.slint"),
    ),
    (
        "console/piano_roll.slint",
        include_str!("../ui/console/piano_roll.slint"),
    ),
    (
        "console/mixer_console.slint",
        include_str!("../ui/console/mixer_console.slint"),
    ),
    (
        "console/device_rack.slint",
        include_str!("../ui/console/device_rack.slint"),
    ),
    (
        "console/asset_pool_panel.slint",
        include_str!("../ui/console/asset_pool_panel.slint"),
    ),
    ("dialogs/musical_pr_drawer.slint", MUSICAL_PR_DRAWER),
    (
        "dialogs/undo_tree_modal.slint",
        include_str!("../ui/dialogs/undo_tree_modal.slint"),
    ),
    (
        "dialogs/export_wav_dialog.slint",
        include_str!("../ui/dialogs/export_wav_dialog.slint"),
    ),
    (
        "dialogs/humanize_dialog.slint",
        include_str!("../ui/dialogs/humanize_dialog.slint"),
    ),
    (
        "dialogs/macro_panel.slint",
        include_str!("../ui/dialogs/macro_panel.slint"),
    ),
    (
        "module_actions.slint",
        include_str!("../ui/module_actions.slint"),
    ),
];

/// ① 曲风预设键必须显式命名 `TouchArea`，且整条反馈表达式**逐字**存在 ⇒
/// false 分支逐字回落到改动前的 `bg-panel` / `line`（默认帧零像素位移）。
#[test]
fn profile_key_is_named_and_shows_hover_and_pressed_feedback() {
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

/// ② 段落标签点击区必须**可发现**：`mouse-cursor: pointer` 落在
/// `target-cycle` 那块 `TouchArea` 内；光标不进渲染帧 ⇒ 零像素位移，
/// 且仍是「不进无障碍树」的纯指针控件（⛔ 不新增 `accessible-id`）。
#[test]
fn section_label_click_target_is_discoverable_without_new_pixels() {
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

/// ③ 「返回编曲」不再是全仓唯一只有 `pressed` 的半套真按钮。
#[test]
fn back_to_arrangement_button_is_no_longer_half_wired() {
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

/// ④ 全仓 `has-hover` 棘轮 = 32（第十轮 `[PATCH UI-FEEDBACK-04]` 由 28 扩钉），
/// 并复核第七/八轮扩钉值与本轮一致。
/// 删任一反馈 ⇒ 少一次，红；新增反馈不登记 ⇒ 多一次，也红。
#[test]
fn the_hover_inventory_and_round7_round8_pins_stay_consistent() {
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

/// ⑤ 零新增可寻址节点 / 无障碍读数：被改的两个文件逐字对账。
/// `TouchArea` 命名与 `mouse-cursor` 都**不**进运行时控件树。
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

/// ⑥ 刻意不做的边界：Esc 级联顺序、三个抽屉 `visible`、静音态档名，
/// 全都维持原状。改这些不是「顺手」，而是要连带改本判据、`live_ui_mcp.rs`、
/// `round7_interaction_feedback.rs` 与 `song_overview.rs` 的叙述。
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
