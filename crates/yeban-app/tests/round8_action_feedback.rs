//! 第八轮 `[PATCH UI-FEEDBACK-02]` 常驻门禁：把「按下 / 悬浮可见」从第七轮
//! 的 10 个控制键推广到 **15 个用户最高频的主操作按钮**，并把反馈覆盖率
//! 钉成全仓棘轮。
//!
//! ## 为什么这一轮不是「复制第七轮」
//!
//! 第七轮 `[PATCH UI-FEEDBACK-01]` 修的是三件点状的事：两个切换态不可见、
//! 总览面板 6 键 + 4 个缩放键无反馈、`Esc` 级联顺序。修完之后，用户日常
//! **真正会去点**的按钮仍有一大片悬浮 / 按下时纹丝不动：
//!
//! | 区域 | 文件 | 本轮补齐 | 改法 |
//! | :-- | :-- | :-- | :-- |
//! | 顶栏走带 | `transport.slint` | 播放 / 停止 / 录音 / 回滚 / 保存 / AI 徽章 / 节拍器 / 视图切换，8 键 | 原本只有 `pressed`，扩成 `pressed \|\| has-hover` |
//! | 右栏模块入口 | `module_actions.slint` | 导出 WAV / 导入 MIDI / 装载 SFZ / 录音 / MIDI 输入，5 键 | 匿名 `TouchArea` ⇒ 命名 + 底色反馈 |
//! | 右栏 AI 协作栏 | `app.slint` 的 `AiRail` **组件定义** | 导出诊断包 / 审查提案，2 键 | 同上；常驻右栏与 compact 抽屉是两个实例 ⇒ 改一次两处生效 |
//!
//! 所谓「用户操作感受最明显」不是相加，而是**一致性**：第七轮之后，鼠标扫过
//! 顶栏会出现「有的键亮、有的键不亮」的参差。本轮把 15 个主操作键补齐，
//! 并把「全仓 `has-hover` 出现次数」钉成精确计数 ⇒ 以后谁删掉一处反馈、
//! 或新增按钮却不给反馈，判据先红。
//!
//! ## 默认帧零位移（golden 不重录的前提）
//!
//! 每个新增表达式的**最后一个分支逐字等于改动前的静态 token**：
//! `has-hover=false` / `pressed=false` 时渲染出的颜色与改前完全相同。
//! ① 用「整条表达式逐字存在」逐键钉住，② 用全仓精确计数钉住总量 ⇒
//! 谁把默认帧颜色改了、或悄悄删了一处反馈，都会红。
//!
//! ## 明确**不做**的边界（⑤ 把它们钉成不可静默改动的事实）
//!
//! * `transport-view-session-button` / `transport-view-arrangement-button`：
//!   背景是**状态色**（`? Tokens.bg-raised : Tokens.bg-panel-alt`）。叠加 hover
//!   提亮会与「当前选中态」混淆 —— 这一族第七轮已认作「切换态已有可见区分」。
//! * `transport-commit-button`：`src/elements.rs` 的 `KNOWN_ACTIONLESS_CONTROLS`
//!   登记的刻意占位（有 pressed 提亮、`TouchArea` 无 `clicked`）。
//! * `transport-ai-drawer-button`：第七轮已改，且 `round7_interaction_feedback.rs`
//!   ① 钉着它那一块的 `: Tokens.bg-panel-alt;` 与 `color:` 整串。
//! * `arrangement_view.slint` / `song_overview_panel.slint`：第七轮 ③ 的
//!   `_touch := TouchArea` / `? Tokens.bg-raised : Tokens.bg-panel;` 精确计数门禁。
//! * `app.slint` 三个根级抽屉的 `visible`：`live_ui_mcp.rs` 的「属性 ⇒ 运行时树」
//!   判据钉死；且第七轮 `host.rs` 注释把「compact 抽屉可与全屏同开」当作
//!   `Esc` 级联顺序的事实依据 ⇒ 改 visible 会推翻上一轮。
//! * `Action::Cancel` 加臂关提案 / 宏抽屉：要新造 callback，与 `host.rs`
//!   「不发明这条回调」及「唯一写者」契约冲突。
//! * 键盘焦点环 / Tab 陷阱：要补 `FocusScope`，会改 Tab 轮转表并撞
//!   `export_dialog_button_focus.rs`。
//!
//! 三路只读盘点（覆盖层闭环 / 状态不可见 / 焦点与反馈闭环）后亲自复核，
//! 上述边界均属**刻意如此**或**已有防护**，⛔ 不把空操作伪装成缺口。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | 删掉某键的 `has-hover` | ① 红 + ② 红 |
//! | 把某键 false 分支改成别的 token | ① 红（默认帧漂移） |
//! | 删掉某 `TouchArea` 的 `:=` 命名 | ① 红（Slint 编译期也会红） |
//! | 新增一个 `accessible-id` / `accessible-value` | ③ 红 |
//! | 把 hover 反馈画到 `Tokens.accent` / `bg-control` 上 | ④ 红 + `theme_selection.rs` ⑥ 红 |
//! | 给视图 tab 叠 hover / 动抽屉 `visible` | ⑤ 红 |
//!
//! ## 诚实登记
//!
//! * ⛔ 本轮**零新增** `accessible-id` / `accessible-value`（③ 逐文件钉住）；
//!   全仓 223 个可寻址节点由 `round6_addressable_roles.rs` ① 照旧覆盖，
//!   这里**不**重复钉那个解析口径。
//! * ⛔ 本文件刻意不用 `.all(` 适配器（`assertion_bounds_audit.rs` 的登记守卫
//!   会要求扩 `SOURCES`）；全是纯源码文本判据，不装 Slint 平台、不碰活窗口。
//! * 本机 `real_ui_tier1` 的两条 golden 逐字节断言因字体 / 渲染环境差异仍红，
//!   干净基线同样红 ⇒ 按 `AGENTS.md` 记为 `pending`，⛔ 不重录、⛔ 不放宽判据；
//!   本轮默认帧零位移由 ①② 机械保证，并有干净基线 A/B 的逐字节佐证。
//! * 带日期记录只追加、不改写（本文件是新增判据，不改既有历史登记）。

const TRANSPORT: &str = include_str!("../ui/transport.slint");
const MODULE_ACTIONS: &str = include_str!("../ui/module_actions.slint");
const APP: &str = include_str!("../ui/app.slint");
const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const ARRANGEMENT: &str = include_str!("../ui/workspace/arrangement_view.slint");
const SONG_OVERVIEW: &str = include_str!("../ui/workspace/song_overview_panel.slint");
const SESSION_VIEW: &str = include_str!("../ui/workspace/session_view.slint");

/// 全仓 `.slint` 清单：与 `round6_addressable_roles.rs::SLINT_FILES`、
/// `src/elements.rs::SLINT_MANIFEST` 同一份 19 文件口径（编译期固定）。
/// ② 的「全仓 `has-hover` 总数」用它做主体扫描，⛔ 不用 `read_dir` 空转成绿。
const SLINT_FILES: &[(&str, &str)] = &[
    ("app.slint", APP),
    ("transport.slint", TRANSPORT),
    ("status_bar.slint", include_str!("../ui/status_bar.slint")),
    ("sidebar.slint", include_str!("../ui/sidebar.slint")),
    ("tokens.slint", include_str!("../ui/tokens.slint")),
    (
        "workspace/session_view.slint",
        include_str!("../ui/workspace/session_view.slint"),
    ),
    ("workspace/arrangement_view.slint", ARRANGEMENT),
    ("workspace/song_overview_panel.slint", SONG_OVERVIEW),
    ("console/console_tabs.slint", CONSOLE_TABS),
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
    (
        "dialogs/musical_pr_drawer.slint",
        include_str!("../ui/dialogs/musical_pr_drawer.slint"),
    ),
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
    ("module_actions.slint", MODULE_ACTIONS),
];

/// 本轮 15 个目标按钮：`(文件, 源码, TouchArea 名, 默认帧原底色)`。
/// `orig` 是**改动前那一行的静态 token**，false 分支必须逐字等于它。
const TARGETS: &[(&str, &str, &str, &str)] = &[
    (
        "transport.slint",
        TRANSPORT,
        "play_area",
        "Tokens.bg-panel-alt",
    ),
    (
        "transport.slint",
        TRANSPORT,
        "stop_area",
        "Tokens.bg-panel-alt",
    ),
    (
        "transport.slint",
        TRANSPORT,
        "rec_area",
        "Tokens.bg-panel-alt",
    ),
    (
        "transport.slint",
        TRANSPORT,
        "revert_area",
        "Tokens.bg-panel-alt",
    ),
    (
        "transport.slint",
        TRANSPORT,
        "save_area",
        "Tokens.bg-control",
    ),
    (
        "transport.slint",
        TRANSPORT,
        "proposal_area",
        "Tokens.bg-panel-alt",
    ),
    (
        "transport.slint",
        TRANSPORT,
        "metronome_area",
        "Tokens.bg-panel-alt",
    ),
    (
        "transport.slint",
        TRANSPORT,
        "toggle_area",
        "Tokens.bg-panel-alt",
    ),
    (
        "module_actions.slint",
        MODULE_ACTIONS,
        "export_wav_touch",
        "Tokens.bg-panel-alt",
    ),
    (
        "module_actions.slint",
        MODULE_ACTIONS,
        "import_midi_touch",
        "Tokens.bg-panel-alt",
    ),
    (
        "module_actions.slint",
        MODULE_ACTIONS,
        "load_sfz_touch",
        "Tokens.bg-panel-alt",
    ),
    (
        "module_actions.slint",
        MODULE_ACTIONS,
        "record_wav_touch",
        "Tokens.bg-panel-alt",
    ),
    (
        "module_actions.slint",
        MODULE_ACTIONS,
        "midi_in_touch",
        "Tokens.bg-panel-alt",
    ),
    ("app.slint", APP, "diag_export_touch", "Tokens.bg-panel-alt"),
    ("app.slint", APP, "musical_pr_touch", "Tokens.bg-panel-alt"),
];

/// ① 15 个主操作按钮的 `TouchArea` 必须显式命名，且那个名字**真的**驱动
/// `has-hover`；整条背景表达式逐字存在 ⇒ false 分支逐字回落到原底色。
#[test]
fn every_primary_action_button_has_a_named_hover_aware_touch_area() {
    assert_eq!(TARGETS.len(), 15, "本轮目标按钮数必须与交付说明一致");
    for (file, src, name, orig) in TARGETS {
        assert!(
            src.contains(&format!("{name} := TouchArea")),
            "{file}: `{name}` 必须显式命名，否则 `has-hover` / `pressed` 无从引用"
        );
        let expression =
            format!("({name}.pressed || {name}.has-hover) ? Tokens.bg-raised : {orig};");
        assert!(
            src.contains(&expression),
            "{file}: `{name}` 的反馈表达式整条必须逐字存在 ⇒ 默认帧（无悬浮、无按下）\
             回落到原底色 `{orig}`；缺了就是有人改了默认帧或删了反馈"
        );
    }
}

/// ② 全仓棘轮：`has-hover` 的分布与总数**精确**钉住本轮基线。
/// 删掉任一反馈 ⇒ 少一次，红；新增反馈而不更新判据 ⇒ 多一次，也红。
/// 这同时把第七轮那 11 处反馈一起钉住（⛔ 不允许「补新的、丢旧的」）。
#[test]
fn the_hover_feedback_inventory_is_pinned_across_the_whole_ui() {
    let cases: &[(&str, &str, usize)] = &[
        ("app.slint", APP, 2),
        ("console/console_tabs.slint", CONSOLE_TABS, 1),
        (
            "console/mixer_console.slint",
            include_str!("../ui/console/mixer_console.slint"),
            3,
        ),
        ("module_actions.slint", MODULE_ACTIONS, 5),
        ("transport.slint", TRANSPORT, 9),
        ("workspace/arrangement_view.slint", ARRANGEMENT, 4),
        ("workspace/song_overview_panel.slint", SONG_OVERVIEW, 6),
        ("workspace/session_view.slint", SESSION_VIEW, 2),
    ];
    for (file, src, want) in cases {
        assert_eq!(
            src.matches("has-hover").count(),
            *want,
            "{file}: `has-hover` 次数与本轮基线不符 ⇒ 反馈被删或新增未登记"
        );
    }
    let mut total = 0usize;
    for (_, src) in SLINT_FILES {
        total += src.matches("has-hover").count();
    }
    // ⭐ 第九轮 `[PATCH UI-FEEDBACK-03]`：曲风预设键 + 返回编曲各补 1 处 ⇒ 26→28。
    // ⭐ 第十轮 `[PATCH UI-FEEDBACK-04]`：Session 场景激发键 1 + Mixer Tab1 三键 3 ⇒ 28→32。
    assert_eq!(
        total, 32,
        "全仓 `has-hover` 总数应为 32（第七轮 11 + 第八轮 15 + 第九轮 2 + 第十轮 4）；\
         改了要同步复核清单、解析口径与交付说明"
    );
    assert_eq!(SLINT_FILES.len(), 19, "判据清单仍须是 19 个 `.slint`");
}

/// ③ 零新增可寻址节点 / 无障碍读数：三个被改的文件逐字对账。
/// `TouchArea` 命名**不**进运行时控件树（只有 `accessible-id` 进树），
/// 因此本轮对 `ui/tree` 的读数一位不动。
#[test]
fn no_new_addressable_nodes_or_value_readouts_were_added() {
    assert_eq!(
        MODULE_ACTIONS.matches("accessible-id:").count(),
        10,
        "module_actions.slint 的 10 个可寻址 ID 一位未动"
    );
    assert_eq!(
        MODULE_ACTIONS.matches("accessible-value:").count(),
        4,
        "module_actions.slint 的 4 条读数一位未动（⛔ 不造第二份读数）"
    );
    assert_eq!(
        APP.matches("accessible-id:").count(),
        8,
        "app.slint 的 8 个可寻址 ID 一位未动"
    );
    assert_eq!(
        APP.matches("accessible-value:").count(),
        3,
        "app.slint 的 3 条读数一位未动"
    );
    assert_eq!(
        TRANSPORT.matches("accessible-id:").count(),
        15,
        "transport.slint 的 15 个可寻址 ID 一位未动"
    );
    assert_eq!(
        TRANSPORT.matches("accessible-value:").count(),
        4,
        "transport.slint 的 4 条读数一位未动"
    );
}

/// ④ 唯一强调色 / 控制色的落点不动：本轮只用 `Tokens.bg-raised` 做反馈，
/// ⛔ 不碰 `Tokens.accent`（`theme_selection.rs` ⑥ 钉死全仓恰好一次、
/// 且落在录音键 ● 的 `color:`）与 `Tokens.bg-control`（只住 transport 保存键）。
#[test]
fn the_single_accent_and_bg_control_landings_are_untouched() {
    assert!(
        TRANSPORT.contains(
            "background: (save_area.pressed || save_area.has-hover) \
             ? Tokens.bg-raised : Tokens.bg-control;"
        ),
        "保存键的反馈表达式必须仍以 `bg-control` 收尾（`bg-control` 全仓唯一落点）"
    );
    assert_eq!(
        TRANSPORT.matches("Tokens.bg-control").count(),
        2,
        "transport.slint 里 `Tokens.bg-control` 仍是 1 处代码 + 1 处说明注释"
    );
    assert_eq!(
        MODULE_ACTIONS.matches("Tokens.bg-control").count(),
        0,
        "模块入口不许用 `bg-control`"
    );
    assert_eq!(
        APP.matches("Tokens.bg-control").count(),
        0,
        "AI 协作栏不许用 `bg-control`"
    );
    assert_eq!(
        MODULE_ACTIONS.matches("Tokens.accent").count(),
        0,
        "模块入口不许把 hover 画成唯一强调色"
    );
    assert_eq!(
        APP.matches("Tokens.accent").count(),
        0,
        "AI 协作栏不许把 hover 画成唯一强调色"
    );
}

/// ⑤ 刻意不做的边界：状态色 tab / 知情占位 / 第七轮抽屉块 / 三个抽屉
/// `visible` / 第七轮计数门禁，全都必须维持原状。改这些不是「顺手」，
/// 而是要连带改本判据、`live_ui_mcp.rs` 与第七轮叙述 —— 棘轮的意义就在这里。
#[test]
fn the_deliberate_boundaries_are_still_intact() {
    assert!(
        TRANSPORT.contains(
            "background: !root.arrangement-view ? Tokens.bg-raised : Tokens.bg-panel-alt;"
        ) && TRANSPORT.contains(
            "background: root.arrangement-view ? Tokens.bg-raised : Tokens.bg-panel-alt;"
        ),
        "两个视图 tab 的背景仍是**状态色**：叠加 hover 会与选中态混淆"
    );
    assert!(
        TRANSPORT
            .contains("background: commit_area.pressed ? Tokens.bg-raised : Tokens.bg-panel-alt;"),
        "`transport-commit-button` 仍是 `KNOWN_ACTIONLESS_CONTROLS` 登记的刻意占位"
    );
    assert!(
        TRANSPORT.contains("background: root.ai-drawer-open ? Tokens.gold"),
        "第七轮的 AI 抽屉展开态金底必须原样保留"
    );
    assert!(
        APP.contains("visible: root.compact && root.ai-drawer-open;")
            && APP.contains("visible: root.musical-pr-open;")
            && APP.contains("visible: root.macro-panel-open;"),
        "三个根级抽屉的 visible 表达式未被本轮抑制（运行时树判据 + 第七轮叙述）"
    );
    assert_eq!(
        ARRANGEMENT.matches("_touch := TouchArea").count(),
        4,
        "第七轮钉住的 4 个缩放键命名未被动过"
    );
    assert_eq!(
        SONG_OVERVIEW
            .matches("? Tokens.gold : Tokens.line;")
            .count(),
        4,
        "第七轮 3 条 + 第九轮曲风预设键 1 条 hover 金边 false 分支未被动过"
    );
}
