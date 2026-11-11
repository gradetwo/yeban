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
    assert!(
        SONG_OVERVIEW.contains("profile_touch := TouchArea"),
        "预设键的 `TouchArea` 必须显式命名，否则 `has-hover` / `pressed` 无从引用"
    );
    assert!(
        SONG_OVERVIEW
            .contains("background: profile_touch.pressed ? Tokens.bg-raised : Tokens.bg-panel;"),
        "预设键按下提亮的 false 分支必须逐字回落到 `bg-panel`"
    );
    assert!(
        SONG_OVERVIEW
            .contains("border-color: profile_touch.has-hover ? Tokens.gold : Tokens.line;"),
        "预设键悬浮金边的 false 分支必须逐字回落到 `line`"
    );
    assert!(
        SONG_OVERVIEW.contains("accessible-id: \"song-overview-profile-button\";"),
        "预设键的可寻址 id 一位未动（本轮不新增节点）"
    );
    assert!(
        !SONG_OVERVIEW.contains("background: Tokens.bg-panel;\n        border-radius: 8px;\n        border-width: 1px;\n        border-color: Tokens.line;\n        accessible-role: button;\n        accessible-id: \"song-overview-profile-button\";"),
        "改动前的「恒定底色 + 恒定边框」块必须已被替换掉"
    );
}

/// ② 段落标签点击区必须**可发现**：`mouse-cursor: pointer` 落在
/// `target-cycle` 那块 `TouchArea` 内；光标不进渲染帧 ⇒ 零像素位移，
/// 且仍是「不进无障碍树」的纯指针控件（⛔ 不新增 `accessible-id`）。
#[test]
fn section_label_click_target_is_discoverable_without_new_pixels() {
    let start = SONG_OVERVIEW
        .find("for _[section-index] in root.section-names : TouchArea {")
        .expect("Phase 2 段落标签点击区必须存在");
    let tail = &SONG_OVERVIEW[start..];
    let end = tail
        .find("clicked => { root.target-cycle(section-index); }")
        .expect("段落标签点击区必须仍落 `target-cycle`");
    let block = &tail[..end];
    assert!(
        block.contains("mouse-cursor: MouseCursor.pointer;"),
        "段落标签 8 px 点击区必须给手型光标，否则用户根本看不出这里能点"
    );
    assert!(
        !block.contains("accessible-id"),
        "段落标签仍是纯指针控件，不进无障碍树（⛔ 不新增可寻址节点）"
    );
    assert!(
        MUSICAL_PR_DRAWER.contains("mouse-cursor: MouseCursor.pointer;"),
        "手型光标不是本轮新发明的视觉语言：提案行早有同款先例"
    );
    let mut cursor_total = 0usize;
    for (_, src) in SLINT_FILES {
        cursor_total += src.matches("mouse-cursor: MouseCursor.pointer;").count();
    }
    // ⭐ 第十轮 `[PATCH UI-FEEDBACK-04]`：覆盖层 11 个可点控件补齐手型光标 ⇒ 2→13。
    // 统计口径仍是精确串 `mouse-cursor: MouseCursor.pointer;`；本轮新增的
    // `MouseCursor.grab`（可拖光标）不在此串内，由 `round10_action_feedback.rs` 单独钉。
    // ⭐ 第十一轮 `[PATCH UI-FEEDBACK-05]`：全仓剩余 37 个高频可点控件收官 ⇒ 13→50。
    // `MouseCursor.row-resize`（分隔条）同样不在此串内，由 `round11` 单独钉。
    // 带日期记录只追加、不改写，上面两行原文保留。
    // 人性化 UI 线（`line/humanize-ui-entry`）：人性化弹窗新增 3 个可点控件（关闭 + 两枚应用）⇒ 50→53。
    assert_eq!(
        cursor_total, 53,
        "全仓手型光标应为 53 处（第九轮 2 + 第十轮 11 + 第十一轮 37 + 人性化弹窗 3）；改了要同步复核清单与交付说明"
    );
}

/// ③ 「返回编曲」不再是全仓唯一只有 `pressed` 的半套真按钮。
#[test]
fn back_to_arrangement_button_is_no_longer_half_wired() {
    assert!(
        SESSION_VIEW.contains(
            "background: (back_area.pressed || back_area.has-hover) ? Tokens.bg-raised : Tokens.bg-void;"
        ),
        "返回编曲的反馈表达式必须逐字存在，false 分支逐字回落到 `bg-void`"
    );
    assert!(
        !SESSION_VIEW
            .contains("background: back_area.pressed ? Tokens.bg-raised : Tokens.bg-void;"),
        "改动前的「只按下不悬浮」半套写法必须已消失"
    );
    // ⭐ 第十轮 `[PATCH UI-FEEDBACK-04]`：场景激发键又补 1 处 ⇒ 1→2。
    assert_eq!(
        SESSION_VIEW.matches("has-hover").count(),
        2,
        "session_view.slint 共 2 处反馈（第九轮返回编曲 1 + 第十轮场景激发 1）"
    );
    assert!(
        SESSION_VIEW.contains("background: Tokens.bg-panel;"),
        "`fabricated_state_honesty.rs` 钉住的 Session 插槽格底色一位未动"
    );
}

/// ④ 全仓 `has-hover` 棘轮 = 32（第十轮 `[PATCH UI-FEEDBACK-04]` 由 28 扩钉），
/// 并复核第七/八轮扩钉值与本轮一致。
/// 删任一反馈 ⇒ 少一次，红；新增反馈不登记 ⇒ 多一次，也红。
#[test]
fn the_hover_inventory_and_round7_round8_pins_stay_consistent() {
    let cases: &[(&str, &str, usize)] = &[
        ("app.slint", APP, 2),
        ("transport.slint", TRANSPORT, 9),
        ("workspace/session_view.slint", SESSION_VIEW, 2),
        ("workspace/arrangement_view.slint", ARRANGEMENT, 4),
        ("workspace/song_overview_panel.slint", SONG_OVERVIEW, 6),
        (
            "console/console_tabs.slint",
            include_str!("../ui/console/console_tabs.slint"),
            1,
        ),
        (
            "console/mixer_console.slint",
            include_str!("../ui/console/mixer_console.slint"),
            3,
        ),
        (
            "module_actions.slint",
            include_str!("../ui/module_actions.slint"),
            5,
        ),
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
    assert_eq!(
        total, 32,
        "全仓 `has-hover` 总数应为 32（第七轮 11 + 第八轮 15 + 第九轮 2 + 第十轮 4）"
    );
    assert_eq!(SLINT_FILES.len(), 19, "判据清单仍须是 19 个 `.slint`");

    // round7 ③ 的三条扩钉值：预设键补反馈后 3→4 / 3→4 / 1→2。
    assert_eq!(
        SONG_OVERVIEW
            .matches("? Tokens.gold : Tokens.line;")
            .count(),
        4,
        "七轮金边 3 条 + 预设键 1 条"
    );
    assert_eq!(
        SONG_OVERVIEW.matches("? Tokens.bg-raised").count(),
        4,
        "七轮按下提亮 3 条 + 预设键 1 条"
    );
    assert_eq!(
        SONG_OVERVIEW
            .matches("? Tokens.bg-raised : Tokens.bg-panel;")
            .count(),
        2,
        "折叠键 + 预设键两条按下提亮的 false 分支都是 `bg-panel`"
    );
    assert_eq!(
        SONG_OVERVIEW
            .matches("? Tokens.gold : Tokens.bg-panel);")
            .count(),
        2,
        "胶囊与全屏键的状态三元原样保留"
    );
    assert_eq!(
        ARRANGEMENT.matches("_touch := TouchArea").count(),
        4,
        "第七轮钉住的 4 个缩放键命名未被动过"
    );
}

/// ⑤ 零新增可寻址节点 / 无障碍读数：被改的两个文件逐字对账。
/// `TouchArea` 命名与 `mouse-cursor` 都**不**进运行时控件树。
#[test]
fn no_new_addressable_nodes_or_second_readouts() {
    assert_eq!(
        SONG_OVERVIEW.matches("accessible-id:").count(),
        11,
        "总览面板的 11 个可寻址 ID 一位未动（含 Phase 2 预设键）"
    );
    assert_eq!(
        SONG_OVERVIEW.matches("accessible-value:").count(),
        6,
        "总览面板的 6 条读数一位未动（⛔ 不造第二份读数）"
    );
    assert!(
        SONG_OVERVIEW.contains("accessible-value: root.profile;"),
        "预设键读数仍与可见 text 同源，不是第二份真相源"
    );
    assert_eq!(
        SESSION_VIEW.matches("accessible-id:").count(),
        9,
        "session_view.slint 的 9 个可寻址 ID 一位未动"
    );
    assert_eq!(
        SESSION_VIEW.matches("accessible-value:").count(),
        2,
        "session_view.slint 的 2 条读数一位未动"
    );
    assert_eq!(
        APP.matches("accessible-id:").count(),
        8,
        "app.slint 未改，8 个可寻址 ID 佐证本轮没顺手动根组件"
    );
    for (file, src) in [
        ("song_overview_panel.slint", SONG_OVERVIEW),
        ("session_view.slint", SESSION_VIEW),
    ] {
        assert_eq!(
            src.matches("Tokens.accent").count(),
            0,
            "{file}: 本轮反馈不许画成唯一强调色（`theme_selection.rs` ⑥）"
        );
        assert_eq!(
            src.matches("Tokens.bg-control").count(),
            0,
            "{file}: 本轮反馈不许用 `bg-control`（只住 transport 保存键）"
        );
    }
}

/// ⑥ 刻意不做的边界：Esc 级联顺序、三个抽屉 `visible`、静音态档名，
/// 全都维持原状。改这些不是「顺手」，而是要连带改本判据、`live_ui_mcp.rs`、
/// `round7_interaction_feedback.rs` 与 `song_overview.rs` 的叙述。
#[test]
fn the_deliberate_boundaries_are_still_intact() {
    let cancel = HOST_RS
        .find("Action::Cancel => {")
        .expect("`apply_action` 必须有 `Action::Cancel` 臂");
    // `HOST_RS` 在 Windows 检出是 CRLF：每行多 1 字节，`cancel + 6000` 会落在
    // 多字节字符中间（主干 CI run 42733188432 Windows 腿：末字节 559649 落在
    // '据' 内，`cancel + 6000` 不是字符边界）。把窗口末端进到下一个字符
    // 边界，最多补 2 字节，内容只多不少；LF 下 6000 本已是边界，行为不变。
    let rest = &HOST_RS[cancel..];
    let mut end = 6000.min(rest.len());
    while end < rest.len() && !rest.is_char_boundary(end) {
        end += 1;
    }
    let segment = &rest[..end];
    let fullscreen = segment
        .find("song_overview_toggle_fullscreen")
        .expect("全屏臂必须在 `Action::Cancel` 段内");
    let drawer = segment
        .find("invoke_toggle_ai_drawer")
        .expect("AI 抽屉臂必须在 `Action::Cancel` 段内");
    assert!(
        fullscreen < drawer,
        "Esc 仍是「先退全屏、再收 AI 抽屉」（第七轮 [PATCH UI-FEEDBACK-01]）"
    );
    assert!(
        APP.contains("visible: root.compact && root.ai-drawer-open;")
            && APP.contains("visible: root.musical-pr-open;")
            && APP.contains("visible: root.macro-panel-open;"),
        "三个根级抽屉的 visible 未被抑制（`live_ui_mcp.rs` 运行时树判据）"
    );
    assert!(
        SONG_OVERVIEW_RS.contains("profile: TensionProfile::Default.as_str().to_owned(),"),
        "静音态恒为缺省档是刻意的「不留旧工程数据」语义；本轮重评后不推翻"
    );
    assert!(
        !HOST_RS.contains("invoke_close_musical_pr")
            && !HOST_RS.contains("invoke_close_macro_panel"),
        "⛔ 没有为关提案 / 宏抽屉新造 callback（唯一写者契约；需独立切片）"
    );
}
