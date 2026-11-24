//! 第八轮 `[PATCH UI-FEEDBACK-02]` 常驻门禁：把「按下 / 悬浮可见」从第七轮
//! 的 10 个控制键推广到 **15 个用户最高频的主操作按钮**，并把反馈覆盖率
//! 钉成全仓棘轮。
//!
//! ## 为什么这一轮不是「复制第七轮」
//!
//! 第七轮 `[PATCH UI-FEEDBACK-01]` 修的是三件点状的事：两个切换态不可见、
//! 总览面板 6 键 + 4 个缩放键无反馈、`Esc` 级联顺序。修完之后，用户日常
//! **真正会去点**的按钮仍有一大片悬浮 / 按下时纹丝不动。
//!
//! ⚠ **2026-11-24（T-FIX-356）复核：旧文案的 15 键里只有 4 键还活着** ——
//! `transport.slint` / `module_actions.slint` 已死（见 `docs/DEAD-SLINT-FILES.md`：
//! 从未被 import），`AiRail` 组件定义已从 `app.slint` 移除（导出/提案改走菜单 +
//! 导出弹窗）。活下来的走带键住进了 `ui/transport_bar.slint`，形态也变了：
//! 播放/录音/循环/停止/回滚的 false 分支是 `transparent`（扁平档），不再是
//! `Tokens.bg-panel-alt`；循环键的 `Theme.accent` 表的是"正在循环"这一**状态**，
//! 不是悬浮。本文件按活树重写，只钉活着的 5 个走带键。
//!
//! ## 旧 15 键的去向（逐键对账，2026-11-24 活树实测）
//!
//! | 旧键 | 去向 |
//! | :--- | :--- |
//! | `play_area` / `stop_area` / `rec_area`（死 `transport.slint`） | 活着：`transport_bar.slint` 的 `btn_play` / `btn_stop` / `btn_rec`（同名 `accessible-id` 由 T-FIX-122 落到活键） |
//! | `revert_area`（回滚） | 活着：`btn_rewind`（"返回工程起点 / 归零"，同一语义、改名） |
//! | `save_area` / `toggle_area`（视图切换） | **死**：无活按钮等价物，**无法按原文恢复** |
//! | `proposal_area`（AI 徽章）/ `metronome_area` | **死**：功能改走菜单/抽屉（节拍器只剩 `app.slint` 的 `MenuItem`，见 ⑤），按钮形态的悬浮反馈已不存在，**无法按原文恢复** |
//! | 5 个 `module_actions.slint` 键（导出 WAV / 导入 MIDI / 装载 SFZ / 录音 / MIDI 输入） | **死**：文件已不在活树中，**无法按原文恢复** |
//! | 2 个 `AiRail` 键（导出诊断包 / 审查提案） | **死**：组件定义已从 `app.slint` 移除，**无法按原文恢复** |
//! | —— | 新增：`btn_loop`（循环开关，旧 15 键里没有；状态色见 ①④） |
//!
//! 所谓「用户操作感受最明显」不是相加，而是**一致性**：第七轮之后，鼠标扫过
//! 顶栏会出现「有的键亮、有的键不亮」的参差。本轮把主操作键补齐，
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
//! | 把循环键的 `root.is-looping ? Theme.accent` 改成 hover 驱动 | ① 红（状态与悬浮混淆）+ ④ 红 |
//! | 新增一个 `accessible-id` / `accessible-value` | ③ 红 |
//! | 把 hover 反馈画到 `Tokens.accent` / `bg-control` 上 | ④ 红 + `theme_selection.rs` ⑥ 红 |
//! | 给工具键的选中态叠 hover / 动抽屉门控 / 动 `Esc` 顺序 | ⑤ 红 |
//!
//! ## 诚实登记
//!
//! * ⛔ 本轮**零新增** `accessible-id` / `accessible-value`（③ 逐文件钉住）；
//!   全仓 448 个可寻址节点由 `round6_addressable_roles.rs` ① 照旧覆盖（2026-11-24
//!   复核值；`arrangement_ruler_seek` 收尾时 +1，见该文件），这里**不**重复钉那个解析口径。
//! * ⛔ 本文件刻意不用 `.all(` 适配器（`assertion_bounds_audit.rs` 的登记守卫
//!   会要求扩 `SOURCES`）；全是纯源码文本判据，不装 Slint 平台、不碰活窗口。
//! * 本机 `real_ui_tier1` 的两条 golden 逐字节断言因字体 / 渲染环境差异仍红，
//!   干净基线同样红 ⇒ 按 `AGENTS.md` 记为 `pending`，⛔ 不重录、⛔ 不放宽判据；
//!   本轮默认帧零位移由 ①② 机械保证，并有干净基线 A/B 的逐字节佐证。
//! * 带日期记录只追加、不改写（本文件是新增判据，不改既有历史登记）。
//! * ② 的语料是注册表 `SLINT_MANIFEST`（45 文件，唯一登记；T-FIX-327），
//!   其中 4 个死文件（`transport.slint` / `module_actions.slint` /
//!   `workspace/arrangement_view.slint` / `workspace/song_overview_panel.slint`）
//!   仍贡献 `has-hover` 26 处（死文本冻结值）；② 除钉全仓 300 外另钉
//!   活文件子集 274 ⇒ 死文本的量被显式隔离，删死文件时两处钉数同步可算。
//! * ⚠ **T-FIX-356 重写说明**：上一版（`8e1cb0ed`）的 ①③④⑤ 是对着死文件断言的
//!   （13 个"目标"在死文本里恒成立，活界面改什么都不红）。本版按活树重写，
//!   死文件 `include_str!` 常量已全部删除（`TRANSPORT` / `MODULE_ACTIONS` /
//!   `ARRANGEMENT` / `SONG_OVERVIEW`）；旧 15 键里 11 键无法按原文恢复，
//!   见上表（协调人口径"7 + 2"少计了 `save_area` 与 `toggle_area` ——
//!   两者确无活按钮等价物，见 GROUP-B-REPORT）。

#![allow(unused)]
/// 活走带（5 个主操作键的唯一活文件）。
const TRANSPORT_BAR: &str = include_str!("../ui/transport_bar.slint");
const APP: &str = include_str!("../ui/app.slint");
const ELEMENTS_RS: &str = include_str!("../src/elements.rs");

// ⭐ **T-FIX-327**：本文件原先另有一份手抄的 `SLINT_FILES` 全仓名单（18 文件），
// 从未被任何判据消费、且与注册表（45 文件）早已漂移 —— 已删除。全仓 `.slint`
// 的唯一登记是 `yeban_app::elements::SLINT_MANIFEST`；需要全仓语料时从它派生，
// ⛔ 不许再手写第二份名单。

/// 死文件（`docs/DEAD-SLINT-FILES.md`：从未被 import）：② 的活文件子集钉数
/// 排除它们；本文件其余判据⛔ 不读死文本。
const DEAD_FILES: [&str; 4] = [
    "transport.slint",
    "module_actions.slint",
    "workspace/arrangement_view.slint",
    "workspace/song_overview_panel.slint",
];

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

/// 源码读数口径：去掉 `//` 行注释、`/* */` 块注释与 `"…"` 字符串字面量再计数
/// （注释 / 文案里提到的 token 不算实现）。`has-hover-tip` 是主题属性，
/// 与反馈的 `has-hover` 是另一概念，② 里单独扣掉。
fn code_only(source: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len() {
        if source[index..].starts_with("//") {
            match source[index..].find('\n') {
                Some(offset) => index += offset,
                None => break,
            }
        } else if source[index..].starts_with("/*") {
            match source[index + 2..].find("*/") {
                Some(offset) => index += offset + 4,
                None => break,
            }
        } else if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() && bytes[index] != b'"' {
                if bytes[index] == b'\\' {
                    index += 1;
                }
                index += 1;
            }
            index += 1;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// ① 活走带 5 键：`Rectangle` 与 `TouchArea` 必须显式命名，且悬浮片段的
/// false 分支逐字是 `transparent`（扁平档无背板）；循环键的 `Theme.accent`
/// 必须是 `root.is-looping` 驱动的**状态**，不是悬浮。
///
/// `(矩形名, TouchArea 名, 悬浮片段)` —— 悬浮片段逐字存在 ⇒ false 分支回落
/// `transparent`，删掉任一键的反馈即红。
const LIVE_BUTTONS: &[(&str, &str, &str)] = &[
    (
        "btn_rewind",
        "btn_rewind_touch",
        "btn_rewind_touch.has-hover ? Tokens.bg-lane2 : transparent",
    ),
    (
        "btn_stop",
        "btn_stop_touch",
        "btn_stop_touch.has-hover ? Tokens.bg-lane2 : transparent",
    ),
    (
        "btn_play",
        "btn_play_touch",
        "btn_play_touch.has-hover ? Tokens.skin-meter-low.with-alpha(0.16) : transparent",
    ),
    (
        "btn_rec",
        "btn_rec_touch",
        "btn_rec_touch.has-hover ? Tokens.skin-arm.with-alpha(0.16) : transparent",
    ),
    (
        "btn_loop",
        "btn_loop_touch",
        "btn_loop_touch.has-hover ? Tokens.bg-lane2 : transparent",
    ),
];

#[test]
fn every_live_transport_button_has_a_named_hover_aware_touch_area() {
    let bar = code_only(TRANSPORT_BAR);
    for &(rect, touch, hover) in LIVE_BUTTONS {
        assert!(
            bar.contains(&format!("{rect} := Rectangle")),
            "活走带里 {rect} 必须是显式命名的 Rectangle"
        );
        assert!(
            bar.contains(&format!("{touch} := TouchArea")),
            "活走带里 {touch} 必须是显式命名的 TouchArea"
        );
        assert!(
            bar.contains(hover),
            "活走带里 {rect} 的悬浮片段必须逐字存在（false 分支回落 transparent）：{hover}"
        );
    }
    // 循环键：`Theme.accent` 是"正在循环"的状态色 —— 三处（背景 / 边框宽 /
    // 图标色）全由 `root.is-looping` 驱动，⛔ 不是 hover。
    assert_eq!(
        bar.matches("root.is-looping ? Theme.accent").count(),
        3,
        "循环键的 Theme.accent 必须恰好 3 处、且全由 root.is-looping 驱动"
    );
    assert!(
        bar.contains(
            "background: root.is-looping ? Theme.accent.with-alpha(0.22) : \
             (btn_loop_touch.has-hover ? Tokens.bg-lane2 : transparent);"
        ),
        "循环键的背景必须逐字是：状态 ? Theme.accent : (悬浮 ? bg-lane2 : transparent)"
    );
    // 4 个活键带无障碍 id（回滚键没有）；id 字面量是字符串，须对原文查
    // （code_only 会剥掉字符串字面量）。
    for id in [
        "transport-stop-button",
        "transport-play-button",
        "transport-record-button",
        "transport-loop-button",
    ] {
        assert!(
            TRANSPORT_BAR.contains(&format!("accessible-id: \"{id}\"")),
            "活走带里必须有 accessible-id {id}"
        );
    }
}

/// ② 全仓棘轮：`has-hover` 的分布与总数**精确**钉住。
///
/// 口径（2026-11-24 从活树重算）：`SLINT_MANIFEST` 全部 45 个文件、去掉注释
/// 与字符串字面量后，`"has-hover"` 原始 300 处，其中主题属性 `"has-hover-tip"`
/// 6 处 ⇒ 反馈净值 294。删掉任一反馈 ⇒ 少一次，红；新增反馈而不更新判据 ⇒
/// 多一次，也红。旧钉数 26/28/32 是旧 UI 的量。
/// 另钉活文件子集：45 文件减 4 死文件 ⇒ 274 处（死文本 26 处被显式隔离）。
#[test]
fn the_hover_feedback_inventory_is_pinned_across_the_whole_ui() {
    let mut hover = 0_usize;
    let mut tip = 0_usize;
    let mut live_hover = 0_usize;
    for (name, src) in manifest_sources() {
        let code = code_only(&src);
        let count = code.matches("has-hover").count();
        hover += count;
        tip += code.matches("has-hover-tip").count();
        if !DEAD_FILES.contains(&name.as_str()) {
            live_hover += count;
        }
    }
    assert_eq!(
        hover, 300,
        "全仓 has-hover 原始总数漂移：活树 {hover} 处，钉死 300 处"
    );
    assert_eq!(
        tip, 6,
        "全仓 has-hover-tip（主题属性、不是反馈）漂移：活树 {tip} 处，钉死 6 处"
    );
    assert_eq!(
        hover - tip,
        294,
        "全仓 has-hover 反馈净值漂移：活树 {} 处，钉死 294 处",
        hover - tip
    );
    assert_eq!(
        live_hover, 274,
        "活文件 has-hover 子集漂移：活树 {live_hover} 处，钉死 274 处（死文本 26 处已排除）"
    );
    assert_eq!(
        code_only(TRANSPORT_BAR).matches("has-hover").count(),
        44,
        "活走带 transport_bar.slint 里 has-hover 必须还是 44 处"
    );
    assert_eq!(
        code_only(APP).matches("has-hover").count(),
        5,
        "app.slint 里 has-hover 必须还是 5 处（含 has-hover-tip 1 处）"
    );
}

/// ③ 零新增可寻址节点 / 无障碍读数：两个活文件逐字对账。
///
/// 口径同 ②（去注释与字符串后计数）。2026-11-24 活树读数：transport_bar
/// 10 id / 2 value、app 15 id / 0 value。死文件的 id/value ⛔ 不钉
/// （死文本里的数字与活界面无关）。
#[test]
fn no_new_addressable_nodes_or_value_readouts_were_added() {
    let cases: &[(&str, &str, usize, usize)] = &[
        ("transport_bar.slint", TRANSPORT_BAR, 10, 2),
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

/// ④ 强调色 / 控制色的落点不动。
///
/// ⚠ 旧口径已漂移：「全仓 `Tokens.accent` 恰好一次」是旧 UI 的量，新 UI 里
/// accent 已是 29 处（去注释与字符串后，manifest 45 文件）；`Tokens.bg-control`
/// 9 处。本判据钉住今天的量，并保留可核的那一半事实：活走带里
/// `Theme.accent` 37 处全是状态色（循环/播放/录音等），`Tokens.accent` 与
/// `bg-control` 都是 0 处 —— 悬浮反馈⛔ 许画到强调色/控制色上。
#[test]
fn the_single_accent_and_bg_control_landings_are_untouched() {
    let mut accent = 0_usize;
    let mut control = 0_usize;
    for (_, src) in manifest_sources() {
        let code = code_only(&src);
        accent += code.matches("Tokens.accent").count();
        control += code.matches("Tokens.bg-control").count();
    }
    assert_eq!(
        accent, 29,
        "全仓 Tokens.accent 数漂移：活树 {accent} 处，钉死 29 处"
    );
    assert_eq!(
        control, 9,
        "全仓 Tokens.bg-control 数漂移：活树 {control} 处，钉死 9 处"
    );
    let bar = code_only(TRANSPORT_BAR);
    assert_eq!(
        bar.matches("Theme.accent").count(),
        37,
        "活走带里 Theme.accent 必须还是 37 处（状态色）"
    );
    assert_eq!(
        bar.matches("Tokens.accent").count(),
        0,
        "活走带里 Tokens.accent 必须是 0 处（悬浮反馈⛔ 许画到强调色上）"
    );
    assert_eq!(
        bar.matches("bg-control").count(),
        0,
        "活走带里 bg-control 必须是 0 处（保存键的 bg-control false 分支随死文件走了）"
    );
}

/// ⑤ 刻意不做的边界：选中态与悬浮的分离 / 知情占位 / 抽屉门控 / 死键的
/// 活去向，全都必须维持原状。
#[test]
fn the_deliberate_boundaries_are_still_intact() {
    let bar = code_only(TRANSPORT_BAR);
    // 播放/录音键的背景以**状态**开头（`root.is-playing` / `root.is-recording`），
    // 悬浮只在扁平档的内层分支 —— 选中态与悬浮⛔ 许混淆。
    assert!(
        bar.contains("background: root.is-playing"),
        "播放键的背景必须以 root.is-playing 状态开头"
    );
    assert!(
        bar.contains("background: root.is-recording"),
        "录音键的背景必须以 root.is-recording 状态开头"
    );
    assert!(
        ELEMENTS_RS.contains("transport-commit-button"),
        "transport-commit-button 必须仍登记在 elements.rs 的知情占位里"
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
    // 死键的活去向（按钮形态已死，功能改走菜单/抽屉 —— 钉住去向，别假装按钮还在）：
    // 节拍器只剩菜单项；AI 协作只剩抽屉门。
    assert!(
        APP.contains("command-execute(\"transport-metronome\")"),
        "节拍器开关必须还在 app.slint 的菜单里（按钮形态已死，功能未死）"
    );
}
