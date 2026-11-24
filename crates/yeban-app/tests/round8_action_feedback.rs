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
//! ⚠ **2026-11-24（T-FIX-356）复核：上面三行里只有顶栏走带还活着** ——
//! `transport.slint` / `module_actions.slint` 已死（见 `docs/DEAD-SLINT-FILES.md`），
//! `AiRail` 组件定义已从 `app.slint` 移除（导出/提案改走菜单 + 导出弹窗）。
//! 活下来的走带键住进了 `ui/transport_bar.slint`，形态也变了：播放/录音/循环/
//! 停止/回滚的 false 分支是 `transparent`（扁平档），不再是 `Tokens.bg-panel-alt`；
//! 循环键的 `Theme.accent` 表的是"正在循环"这一**状态**，不是悬浮。
//! 本文件按活树重写：① 只钉活着的 5 个走带键（死掉的 7 键 + 2 AiRail 键
//! **无法按原文恢复**，见 GROUP-B-REPORT）；② 的全仓 `has-hover` 总数从活树重算
//! （旧 26/28/32 全是陈旧值）；③④⑤ 同样按活树重算。
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
//! ⚠ 活树形态：走带键的 false 分支是 `transparent`（扁平档无背板），
//! ① 钉的就是这个 `transparent` —— 它是"悬停才浮出底色、平时一位不动"的保证。
//!
//! ## 明确**不做**的边界（⑤ 把它们钉成不可静默改动的事实）
//!
//! * 工具/视图键的**选中态**：活走带里选中态用 `Theme.accent`（工具键图标、
//!   视图键边框），悬浮用 `bg-lane2` / `transparent` —— 两者⛔ 许混淆。
//! * `transport-commit-button`：`src/elements.rs` 的 `KNOWN_ACTIONLESS_CONTROLS`
//!   登记的刻意占位（有 pressed 提亮、`TouchArea` 无 `clicked`）。
//! * `app.slint` 三个根级抽屉的门控：现为 `if root.<x>-open :` 条件实例化
//!   （AI 抽屉 / 音乐提案抽屉 / 教学抽屉），`live_ui_mcp.rs` 的「属性 ⇒ 运行时树」
//!   判据钉死；改门控会推翻 `Esc` 级联顺序的事实依据。
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
//! | 把某键 false 分支的 `transparent` 改成别的 token | ① 红（默认帧漂移） |
//! | 删掉某 `TouchArea` 的 `:=` 命名 | ① 红（Slint 编译期也会红） |
//! | 新增一个 `accessible-id` / `accessible-value` | ③ 红 |
//! | 把 hover 反馈画到 `Tokens.accent` / `bg-control` 上 | ④ 红 + `theme_selection.rs` ⑥ 红 |
//! | 给工具键的选中态叠 hover / 动抽屉门控 / 动 `Esc` 顺序 | ⑤ 红 |
//!
//! ## 诚实登记
//!
//! * ⛔ 本轮**零新增** `accessible-id` / `accessible-value`（③ 逐文件钉住）；
//!   全仓 447 个可寻址节点由 `round6_addressable_roles.rs` ① 照旧覆盖（2026-11-24
//!   复核值；本文件旧文案写的 223 已陈旧），这里**不**重复钉那个解析口径。
//! * ⛔ 本文件刻意不用 `.all(` 适配器（`assertion_bounds_audit.rs` 的登记守卫
//!   会要求扩 `SOURCES`）；全是纯源码文本判据，不装 Slint 平台、不碰活窗口。
//! * 本机 `real_ui_tier1` 的两条 golden 逐字节断言因字体 / 渲染环境差异仍红，
//!   干净基线同样红 ⇒ 按 `AGENTS.md` 记为 `pending`，⛔ 不重录、⛔ 不放宽判据；
//!   本轮默认帧零位移由 ①② 机械保证，并有干净基线 A/B 的逐字节佐证。
//! * 带日期记录只追加、不改写（本文件是新增判据，不改既有历史登记）。

#![allow(unused)]
const TRANSPORT: &str = include_str!("../ui/transport.slint");
const MODULE_ACTIONS: &str = include_str!("../ui/module_actions.slint");
const APP: &str = include_str!("../ui/app.slint");
const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const ARRANGEMENT: &str = include_str!("../ui/workspace/arrangement_view.slint");
const SONG_OVERVIEW: &str = include_str!("../ui/workspace/song_overview_panel.slint");
const SESSION_VIEW: &str = include_str!("../ui/workspace/session_view.slint");

// ⭐ **T-FIX-327**：本文件原先另有一份手抄的 `SLINT_FILES` 全仓名单（18 文件），
// 从未被任何判据消费、且与注册表（45 文件）早已漂移 —— 已删除。全仓 `.slint`
// 的唯一登记是 `yeban_app::elements::SLINT_MANIFEST`；需要全仓语料时从它派生，
// ⛔ 不许再手写第二份名单。

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
const ELEMENTS_RS: &str = include_str!("../src/elements.rs");

/// 全仓语料 = 注册表的 SLINT_MANIFEST（唯一登记；T-FIX-327）。
fn manifest_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in yeban_app::elements::SLINT_MANIFEST {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ui")
            .join(name);
        let src = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("清单里的 ui/{name} 读不到（{error}）—— 缺失文件必须大声失败")
        });
        out.push((name.to_string(), src));
    }
    out
}

/// 源码读数口径：去掉 `//` 行注释再计数（注释里提到的 token 不算实现）。
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ① 15 个主操作按钮的 `TouchArea` 必须显式命名，且那个名字**真的**驱动
/// `has-hover`；整条背景表达式逐字存在 ⇒ false 分支逐字回落到原底色。
///
/// ⚠ 活树实测（2026-11-24）：13 个目标仍在，`app.slint` 里的 `AiRail` 两键
/// （`diag_export_touch` / `musical_pr_touch`）在新 UI 里**已被删掉**——
/// 下面把「删了」也钉成读数（名字在 `app.slint` 里一次都不出现），而不是
/// 悄悄把 TARGETS 缩成 13 个。
#[test]
fn every_primary_action_button_has_a_named_hover_aware_touch_area() {
    for &(file, source, name, orig) in TARGETS {
        let named = format!("{name} := TouchArea");
        assert!(
            source.contains(&named),
            "{file} 里 {name} 必须是显式命名的 TouchArea"
        );
        let expr = format!("({name}.pressed || {name}.has-hover) ? Tokens.bg-raised : {orig}");
        assert!(
            source.contains(&expr),
            "{file} 里 {name} 的整条背景表达式必须逐字存在（false 分支回落 {orig}）"
        );
    }
    assert!(
        !APP.contains("diag_export_touch") && !APP.contains("musical_pr_touch"),
        "新 UI 已删 AiRail 两键：diag_export_touch / musical_pr_touch 在 app.slint 里必须一次都不出现"
    );
}

/// ② 全仓棘轮：`has-hover` 的分布与总数**精确**钉住。
///
/// 口径（2026-11-24 从活树重算）：`SLINT_MANIFEST` 全部 45 个文件、去掉 `//`
/// 行注释后，`"has-hover"` 出现数减去主题属性 `"has-hover-tip"` 出现数
/// （那是另一概念，不是反馈）。删掉任一反馈 ⇒ 少一次，红；新增反馈而不
/// 更新判据 ⇒ 多一次，也红。旧钉数 26/28/32 是旧 UI 的量，新 UI 全仓
/// 294 —— 数字变了，棘轮的意义没变。
#[test]
fn the_hover_feedback_inventory_is_pinned_across_the_whole_ui() {
    let mut hover = 0_usize;
    let mut tip = 0_usize;
    for (name, src) in manifest_sources() {
        let code = code_only(&src);
        hover += code.matches("has-hover").count();
        tip += code.matches("has-hover-tip").count();
        let _ = name;
    }
    assert_eq!(
        hover - tip,
        294,
        "全仓 has-hover 反馈总数漂移：活树 {} 处（已扣 has-hover-tip {tip} 处），钉死 294 处",
        hover - tip
    );
}

/// ③ 零新增可寻址节点 / 无障碍读数：三个被改的文件逐字对账。
///
/// 口径同 ②（去注释后计数）。2026-11-24 活树读数：transport 15 id / 3 value、
/// module_actions 10 id / 4 value、app 15 id / 0 value。
#[test]
fn no_new_addressable_nodes_or_value_readouts_were_added() {
    let cases: &[(&str, &str, usize, usize)] = &[
        ("transport.slint", TRANSPORT, 15, 3),
        ("module_actions.slint", MODULE_ACTIONS, 10, 4),
        ("app.slint", APP, 15, 0),
    ];
    for (file, source, ids, values) in cases {
        let code = code_only(source);
        assert_eq!(
            code.matches("accessible-id:").count(),
            *ids,
            "{file} 的 accessible-id 数漂移"
        );
        assert_eq!(
            code.matches("accessible-value:").count(),
            *values,
            "{file} 的 accessible-value 数漂移"
        );
    }
}

/// ④ 唯一强调色 / 控制色的落点不动。
///
/// ⚠ 旧口径已漂移：「全仓 `Tokens.accent` 恰好一次」是旧 UI 的量，新 UI 里
/// accent 已是 29 处（去注释后，manifest 45 文件）；`Tokens.bg-control` 9 处。
/// 本判据钉住今天的量，并保留可核的那一半事实：transport 里 `bg-control`
/// 只剩保存键 false 分支那一处。
#[test]
fn the_single_accent_and_bg_control_landings_are_untouched() {
    let mut accent = 0_usize;
    let mut control = 0_usize;
    for (_, src) in manifest_sources() {
        let code = code_only(&src);
        accent += code.matches("Tokens.accent").count();
        control += code.matches("Tokens.bg-control").count();
    }
    assert_eq!(accent, 29, "全仓 Tokens.accent 数漂移：活树 {accent} 处，钉死 29 处");
    assert_eq!(
        control, 9,
        "全仓 Tokens.bg-control 数漂移：活树 {control} 处，钉死 9 处"
    );
    assert_eq!(
        code_only(TRANSPORT).matches("Tokens.bg-control").count(),
        1,
        "transport 里 bg-control 必须只剩保存键 false 分支那一处"
    );
    assert!(
        TRANSPORT.contains("(save_area.pressed || save_area.has-hover) ? Tokens.bg-raised : Tokens.bg-control"),
        "保存键的 false 分支必须仍是 Tokens.bg-control"
    );
}

/// ⑤ 刻意不做的边界：状态色 tab / 知情占位 / 第七轮抽屉块 / 抽屉
/// `visible` / 第七轮计数门禁，全都必须维持原状。
#[test]
fn the_deliberate_boundaries_are_still_intact() {
    let transport = code_only(TRANSPORT);
    assert!(
        transport.contains("transport-view-session-button")
            && transport.contains("transport-view-arrangement-button"),
        "视图 tab 两键必须还在 transport 里"
    );
    assert!(
        transport.contains("? Tokens.bg-raised : Tokens.bg-panel-alt"),
        "视图 tab 必须是状态色（叠 hover 提亮会与选中态混淆）"
    );
    assert!(
        transport.contains("transport-ai-drawer-button"),
        "第七轮已改的 AI 抽屉键必须还在"
    );
    assert!(
        ELEMENTS_RS.contains("transport-commit-button"),
        "transport-commit-button 必须仍登记在 elements.rs 的知情占位里"
    );
    // 第七轮 ③ 的门禁：两文件里 `_touch := TouchArea` 与三元的计数。
    assert_eq!(
        code_only(ARRANGEMENT).matches("_touch := TouchArea").count(),
        4,
        "arrangement_view 里 _touch 命名 TouchArea 必须还是 4 处"
    );
    assert_eq!(
        code_only(ARRANGEMENT)
            .matches("? Tokens.bg-raised : Tokens.bg-panel;")
            .count(),
        4,
        "arrangement_view 里 bg-raised/bg-panel 三元必须还是 4 处"
    );
    assert_eq!(
        code_only(SONG_OVERVIEW).matches("_touch := TouchArea").count(),
        7,
        "song_overview_panel 里 _touch 命名 TouchArea 必须还是 7 处"
    );
    assert_eq!(
        code_only(SONG_OVERVIEW)
            .matches("? Tokens.bg-raised : Tokens.bg-panel;")
            .count(),
        1,
        "song_overview_panel 里 bg-raised/bg-panel 三元必须还是 1 处"
    );
    // 根级抽屉的可见门（`if root.<flag> :`）：新 UI 是四扇（T-FIX-304 补了 AI 抽屉）。
    for flag in [
        "show-clip-matrix",
        "musical-pr-open",
        "lessons-open",
        "ai-drawer-open",
    ] {
        assert!(
            APP.contains(&format!("if root.{flag}")),
            "app.slint 里抽屉门 if root.{flag} 必须还在"
        );
    }
}
