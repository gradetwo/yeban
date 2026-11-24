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
// ⭐ T-FIX-218：`sidebar.slint` 已删（旧侧栏组件，无人 import、不参与编译），
// 此处原先的 `SIDEBAR` 常量随之删除。
// ⚠ 死文件 `include_str!` 已按简报清除：`transport.slint` / `module_actions.slint` /
// `workspace/arrangement_view.slint` / `workspace/song_overview_panel.slint`（见
// `docs/DEAD-SLINT-FILES.md`）。依赖它们的断言已重定向到活源（④⑤）或随文件删除（①⑦），
// 全仓总数仍按 `SLINT_MANIFEST` 语料算（含死文件在盘内容——那是语料，不是活 UI）。
const SESSION_VIEW: &str = include_str!("../ui/workspace/session_view.slint");
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

/// 全仓 `.slint` 语料：由 `yeban_app::elements::SLINT_MANIFEST`（45 文件）派生，
/// 内容按名读盘 —— 文件缺失即 `panic`（⛔ 不空转成绿）。手抄 `SLINT_FILES` 名单已随
/// T-FIX-327 删除；此处是它唯一的替代品。
fn slint_sources() -> Vec<(&'static str, String)> {
    let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    yeban_app::elements::SLINT_MANIFEST
        .iter()
        .map(|name| {
            let source = std::fs::read_to_string(ui_dir.join(name))
                .unwrap_or_else(|e| panic!("读 ui/{name} 失败：{e}"));
            (*name, source)
        })
        .collect()
}

/// 按注册表名取一份活源（④⑤ 的重定向行用）。
fn source(name: &str) -> String {
    slint_sources()
        .into_iter()
        .find(|(file, _)| *file == name)
        .map(|(_, src)| src)
        .unwrap_or_else(|| panic!("清单里必须有 {name}"))
}

// ⭐ **T-FIX-327**：本文件原先另有一份手抄的 `SLINT_FILES` 全仓名单（18 文件），
// 从未被任何判据消费、且与注册表（45 文件）早已漂移 —— 已删除。全仓 `.slint`
// 的唯一登记是 `yeban_app::elements::SLINT_MANIFEST`；需要全仓语料时从它派生，
// ⛔ 不许再手写第二份名单。

/// ① 覆盖层 11 个可点控件补齐手型光标：逐文件精确计数 + 全仓棘轮 = 13。
/// ⭐ 第十一轮 `[PATCH UI-FEEDBACK-05]` 把全仓手型光标扩到 50（只扩不放宽，
/// 逐文件新分布由 `round11_action_feedback.rs` ① 单独钉）。
/// 删任一处 ⇒ 少一次，红；别处又加 ⇒ 多一次，也红。指针串沿用第九轮精确写法。
#[test]
fn overlay_action_controls_are_discoverable_with_pointer_cursors() {
    let cases: &[(&str, &str, usize)] = &[
        ("dialogs/musical_pr_drawer.slint", MUSICAL_PR_DRAWER, 4), // 提案行 1 + 本轮 3
        ("dialogs/undo_tree_modal.slint", UNDO_TREE_MODAL, 4),
        ("dialogs/macro_panel.slint", MACRO_PANEL, 1),
        ("dialogs/export_wav_dialog.slint", EXPORT_WAV_DIALOG, 4),
        ("dialogs/humanize_dialog.slint", HUMANIZE_DIALOG, 3), // 人性化弹窗：关闭 + 两枚应用
                                                               // ⚠ `workspace/song_overview_panel.slint` 是死文件（无活源可重定向）：该行随文件一起走
                                                               // （原 1 处手型光标，休眠钉见 `round11_action_feedback.rs` ① 的同款行）。
    ];
    for (file, src, want) in cases {
        assert_eq!(
            src.matches("mouse-cursor: MouseCursor.pointer;").count(),
            *want,
            "{file}: 手型光标数量与第十轮基线不符 ⇒ 删了或又新增未登记"
        );
    }
    // 光标只补可发现性，⛔ 不替换任何按钮的行为落点。
    assert!(MUSICAL_PR_DRAWER.contains("clicked => { root.accept(); }"));
    assert!(MUSICAL_PR_DRAWER.contains("clicked => { root.reject(); }"));
    assert!(UNDO_TREE_MODAL.contains("clicked => { root.undo-step(); }"));
    assert!(UNDO_TREE_MODAL.contains("clicked => { root.redo-step(); }"));
    assert!(EXPORT_WAV_DIALOG.contains("clicked => { root.run(); }"));
    assert!(EXPORT_WAV_DIALOG.contains("clicked => { root.limiter = !root.limiter; }"));
    // 手型不是本轮新发明的视觉语言：第九轮两处先例仍在。
    assert!(MUSICAL_PR_DRAWER.contains("mouse-cursor: MouseCursor.pointer;"));

    let mut pointer_total = 0usize;
    for (_, src) in slint_sources() {
        pointer_total += src.matches("mouse-cursor: MouseCursor.pointer;").count();
    }
    assert_eq!(
        pointer_total, 231,
        "全仓手型光标（`MouseCursor.pointer;` 拼法）= 231；多一处就是别处也改了光标"
    );
    assert_eq!(
        slint_sources().len(),
        45,
        "判据清单仍须是注册表 45 个 `.slint`"
    );
}

/// ② 7 处拖拽手势面补「可拖」光标：这是本轮唯一的新交互语言。
/// ⭐ 第十一轮新增 `MouseCursor.row-resize`（主窗口分隔条），本句只描述第十轮当时状态。
/// `Grab` 是 `i-slint-common-1.18.1::MouseCursor` 的真实变体（编译探针已验），
/// 且用 `grab` 而非 `pointer` ⇒ 不混进 ① 的精确串计数。
#[test]
fn drag_surfaces_now_show_a_grabbable_cursor() {
    let cases: &[(&str, &str, usize)] = &[
        ("console/mixer_console.slint", MIXER_CONSOLE, 2), // 通道推子/声相 + 主控两处 → 新 UI 剩 2
        ("console/device_rack.slint", DEVICE_RACK, 0), // EQ 面 + 发送滑块改用 pointer，grab 归零
        ("dialogs/macro_panel.slint", MACRO_PANEL, 1), // 宏旋钮手势面
    ];
    for (file, src, want) in cases {
        assert_eq!(
            src.matches("mouse-cursor: MouseCursor.grab;").count(),
            *want,
            "{file}: 可拖光标数量与第十轮基线不符"
        );
    }
    let mut grab_total = 0usize;
    let mut cursor_total = 0usize;
    for (_, src) in slint_sources() {
        grab_total += src.matches("mouse-cursor: MouseCursor.grab;").count();
        cursor_total += src.matches("mouse-cursor:").count();
    }
    assert_eq!(grab_total, 6, "全仓可拖光标恰好 6 处");
    assert_eq!(
        cursor_total, 272,
        "全仓光标恰好 272 处；多一处就是别处也改了光标"
    );
    // 拖拽手势面本身仍是「不画任何像素」的静态矩形，⛔ 没改成会渲染的控件。
    assert!(MIXER_CONSOLE.contains("它们是**静态矩形**"));
    // ⚠ "位置**不随数值动**"这句随新 UI 从 device_rack 搬进 mixer_console（:955）：同一主张，现址再钉。
    assert!(MIXER_CONSOLE.contains("位置都**不随数值动**"));
    // 画布手势面刻意不加光标（语义混合：点选/画音符/拖滚动/双击复位）。
    // ⚠ 新 UI 给卷帘的八度升降键各补 1 处手型光标（`round11` ④ 同款）：整文件零光标的前提已
    // 不存在，钉新现实 —— 恰好 2 处且都在八度按钮里，画布手势面本身仍零光标。
    assert_eq!(
        PIANO_ROLL.matches("mouse-cursor:").count(),
        2,
        "卷帘光标恰好 2 处（两个八度升降键）"
    );
    for marker in [
        "oct-down-touch := TouchArea {",
        "oct-up-touch := TouchArea {",
    ] {
        assert!(
            PIANO_ROLL.contains(marker),
            "八度升降键 `{marker}` 必须留在源文件里"
        );
    }
}

/// ③ Session 场景激发键与 Mixer Tab1 三键不再是「半套」按钮：
/// 表达式整条逐字存在 ⇒ false 分支逐字回落到 `bg-panel-alt`（默认帧零漂移）。
#[test]
fn session_scene_and_mixer_tab1_buttons_are_no_longer_half_wired() {
    // ⚠ 表达式随新 UI 包进 `texture-physical` 三元：逻辑（`pressed || has-hover || 当前场景`）
    // 与 false 分支回落 `bg-panel-alt` 一位未动，逐字钉新写法。
    assert!(
        SESSION_VIEW.contains(
            "background: Tokens.texture-physical ? ((scene_touch.pressed || scene_touch.has-hover || root.current-scene-index == scene_index) ? Tokens.raised-brush : Tokens.panel-alt-brush) : ((scene_touch.pressed || scene_touch.has-hover || root.current-scene-index == scene_index) ? Tokens.bg-raised : Tokens.bg-panel-alt);"
        ),
        "场景激发键反馈表达式必须逐字存在，false 分支逐字回落 `bg-panel-alt`"
    );
    assert!(
        SESSION_VIEW.contains("scene_touch := TouchArea {"),
        "场景激发键的 `TouchArea` 必须显式命名，否则 has-hover 无从引用"
    );
    assert!(
        !SESSION_VIEW.contains(
            "background: root.current-scene-index == scene_index ? Tokens.bg-raised : Tokens.bg-panel-alt;"
        ),
        "改动前的「零反馈」写法必须已消失"
    );
    for name in ["midi_export_area", "open_project_area", "new_project_area"] {
        let expression = format!(
            "background: ({name}.pressed || {name}.has-hover) ? Tokens.bg-raised : Tokens.bg-panel-alt;"
        );
        assert!(
            MIXER_CONSOLE.contains(&expression),
            "Mixer Tab1 的 `{name}` 反馈表达式整条必须逐字存在"
        );
        let old = format!("background: {name}.pressed ? Tokens.bg-raised : Tokens.bg-panel-alt;");
        assert!(
            !MIXER_CONSOLE.contains(&old),
            "Mixer Tab1 的 `{name}` 改动前「只按下不悬浮」的半套写法必须已消失"
        );
    }
}

/// ④ 全仓 `has-hover` 棘轮 = 32，逐文件分布与第十轮基线一致。
/// 与 `round8_action_feedback.rs` ②、`round9_action_feedback.rs` ④ 的 cases 三方互证。
#[test]
fn the_hover_inventory_and_round8_round9_pins_stay_consistent() {
    // ⚠ 死文件行按简报重定向/删除：`transport.slint`（9）→ 活源 `transport_bar.slint`（44），
    // `workspace/arrangement_view.slint`（4）→ 活源 `arrangement.slint`（82）；
    // `module_actions.slint`（5）与 `workspace/song_overview_panel.slint`（9）无活源，
    // 该行删除（其计数仍进下面的全仓总数——总数按语料算，不按本表）。
    let live_transport_bar = source("transport_bar.slint");
    let live_arrangement = source("arrangement.slint");
    let cases: &[(&str, &str, usize)] = &[
        ("app.slint", APP, 5),
        ("transport_bar.slint", &live_transport_bar, 44),
        ("console/console_tabs.slint", CONSOLE_TABS, 1),
        ("console/mixer_console.slint", MIXER_CONSOLE, 3),
        ("workspace/session_view.slint", SESSION_VIEW, 4),
        ("arrangement.slint", &live_arrangement, 82),
    ];
    for (file, src, want) in cases {
        assert_eq!(
            src.matches("has-hover").count(),
            *want,
            "{file}: `has-hover` 次数与第十轮基线不符 ⇒ 反馈被删或新增未登记"
        );
    }
    let mut total = 0usize;
    for (_, src) in slint_sources() {
        total += src.matches("has-hover").count();
    }
    assert_eq!(total, 301, "全仓 `has-hover` 总数应为 301");
    assert_eq!(
        slint_sources().len(),
        45,
        "判据清单仍须是注册表 45 个 `.slint`"
    );
}

/// ⑤ 第 ④ 组（M/S 激活态字色）**A/B 后撤回**：钉住 `arrangement_view.slint`
/// 的 M/S 字色仍是原式 `color: Tokens.ink-2;`，⛔ 不许静默改成反色三元。
/// 原因见文档头诚实登记：改它会动默认帧像素，本机禁止重录 Linux golden。
#[test]
fn mute_solo_active_labels_stay_untouched_after_the_ab_findings() {
    // ⚠ 本判据原先钉死文件 `workspace/arrangement_view.slint` 的 M/S 字色（8 处
    // `color: Tokens.ink-2;`、0 反色三元），无活源可逐字重定向：活源 `arrangement.slint`
    // 的 M/S/R 按钮已改用另一套配色（Studio 色：`trk.muted ? Tokens.bg-lane2 …`）。
    // 按简报把判据重定向到活源、钉**同一条禁令**（撤回的反色三元不得回来）＋活方案逐字存在。
    let live = source("arrangement.slint");
    assert_eq!(
        live.matches("? #060A14 : Tokens.ink-2;").count(),
        0,
        "⛔ 反色字面量不得出现在活编曲 M/S 字色里（A/B 撤回的改动不得回来）"
    );
    assert!(
        !live.contains("color: root.track-mutes[track_index] ?")
            && !live.contains("color: root.track-solos[track_index] ?"),
        "⛔ M/S 字色反色三元必须不存在（A/B 实证会改默认帧，已撤回）"
    );
    assert!(
        live.contains("color: trk.muted ? Tokens.bg-lane2 : (m-btn-touch.has-hover ? Tokens.skin-text-strong : Tokens.skin-text-body);"),
        "活 M 键字色原式逐字存在"
    );
    assert!(
        live.contains("background: trk.soloed ? Tokens.skin-mute : (s-btn-touch.has-hover ? Tokens.skin-chip : Tokens.bg-panel);"),
        "活 S 键底色原式逐字存在"
    );
    assert!(
        !live.contains("Tokens.accent") && !live.contains("Tokens.bg-control"),
        "④ 撤回后仍不许引入唯一强调色 / 控制色（`theme_selection.rs` ⑥）"
    );
}

/// ⑥ 零新增可寻址节点 / 无障碍读数：本轮改动的 7 个 `.slint` 逐文件对账
/// （＋人性化弹窗 1 案：12 ID / 5 读数）。
/// （第 ④ 组撤回后，`arrangement_view.slint` 已不在改动清单内）。
/// `mouse-cursor` 与 `TouchArea` 命名都**不**进运行时控件树。
#[test]
fn no_new_addressable_nodes_or_second_readouts() {
    let cases: &[(&str, &str, usize, usize)] = &[
        ("dialogs/musical_pr_drawer.slint", MUSICAL_PR_DRAWER, 9, 1),
        ("dialogs/undo_tree_modal.slint", UNDO_TREE_MODAL, 8, 0),
        ("dialogs/macro_panel.slint", MACRO_PANEL, 9, 2),
        ("dialogs/export_wav_dialog.slint", EXPORT_WAV_DIALOG, 12, 5),
        ("dialogs/humanize_dialog.slint", HUMANIZE_DIALOG, 12, 5),
        ("console/mixer_console.slint", MIXER_CONSOLE, 16, 3),
        ("console/device_rack.slint", DEVICE_RACK, 58, 0),
        ("workspace/session_view.slint", SESSION_VIEW, 9, 2),
    ];
    for (file, src, ids, values) in cases {
        assert_eq!(
            src.matches("accessible-id:").count(),
            *ids,
            "{file}: 可寻址 ID 一位未动"
        );
        assert_eq!(
            src.matches("accessible-value:").count(),
            *values,
            "{file}: 读数一位未动（⛔ 不造第二份读数）"
        );
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
    let mut readout_total = 0usize;
    for (_, src) in slint_sources() {
        readout_total += src.matches("accessible-value:").count();
    }
    // ⚠ T-FIX-356 合并重钉：63→64 —— 同票 Group B 给标尺带补了
    // `accessible-value: root.playhead-timecode;`（`ui/arrangement.slint`，U-07 R2）；
    // 合入前实测 left 64。
    assert_eq!(readout_total, 64, "全仓读数总数 64");
}

/// ⑦ 刻意不做的边界：Esc 级联顺序、三个抽屉 `visible`、静音态档名、
/// 导出弹窗关闭格的 `FocusScope`、画布手势面、Session 插槽格固定底色，
/// 全都维持原状。改这些不是「顺手」，而是要连带改本判据、`live_ui_mcp.rs`、
/// `round7_interaction_feedback.rs`、`export_dialog_button_focus.rs` 与
/// `song_overview.rs` 的叙述。
#[test]
fn the_deliberate_boundaries_are_still_intact() {
    let cancel = HOST_RS
        .find("Action::Cancel => {")
        .expect("`apply_action` 必须有 `Action::Cancel` 臂");
    // `HOST_RS` 在 Windows 检出是 CRLF：每行多 1 字节，`cancel + 6000` 会落在
    // 多字节字符中间（支 CI run 42777244371 Windows 腿：末字节 565114 落在
    // '据' 内，`cancel + 6000` 不是字符边界；与第九轮判据同型，见
    // `round9_action_feedback.rs` 同款修法）。把窗口末端进到下一个字符
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
    // ⚠ 三个抽屉的可见性随新 UI 从 `visible:` 属性改成条件实例化（`if root.*-open`）：
    // 仍是"未被抑制"同一条禁令，改钉三个 `if` 门。
    assert!(
        APP.contains("if root.ai-drawer-open : Rectangle {")
            && APP.contains("if root.musical-pr-open : musical_pr_drawer := MusicalPrDrawer {")
            && APP.contains("if root.macro-panel-open : macro_panel := MacroPanel {"),
        "三个根级抽屉的条件实例化门未被抑制（`live_ui_mcp.rs` 运行时树判据）"
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
    assert!(
        EXPORT_WAV_DIALOG.contains("export-wav-close-focus := FocusScope {"),
        "⛔ 导出弹窗关闭格的 FocusScope 未被命名改动（本轮不补 × 的 hover 反馈）"
    );
    // ⚠ 卷帘光标恰好 2 处（两个八度升降键；② 同款钉）—— 画布手势面本身仍零光标。
    assert_eq!(
        PIANO_ROLL.matches("mouse-cursor:").count(),
        2,
        "⛔ 卷帘画布手势面刻意不加光标（语义混合），八度键除外"
    );
    assert!(
        SESSION_VIEW.contains("background: Tokens.bg-panel;"),
        "`fabricated_state_honesty.rs` 钉住的 Session 插槽格底色一位未动"
    );
    // ⚠ transport 两个状态 tab 的断言随死文件 `ui/transport.slint` 一起删除：
    // 视图 tab（`view_session_area` / `view_arr_area`）只活在那个死文件里，活树里没有
    // 对应物可重定向（`transport_bar.slint` 里没有这些 tab）。该条禁令的休眠钉见
    // `round11_action_feedback.rs` 的死文件行；此处不再假装它还在活树里。
}
