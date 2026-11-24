//! **双击复位**（推子 → `0 dB`、声相 → 中央）—— **文本层**判据。
//!
//! ## 本文件要证的句子
//!
//! 调音台的四块命中面（通道条推子 / 通道条声相 / 主控推子 / 主控声相）上那个
//! `double-clicked` **不是一个画了但不动的手势**（[`ADR-0052`] 那一类）：
//! 每一面各自绑**自己**的复位回调，回调一路转发到 `MainWindow`，宿主四条臂都汇到
//! **同一个**落点 [`host::commit_mixer_reset`]，而且**先**清掉在手的拖拽手势**再**提交。
//!
//! 仓内已有同族先例：底部控制台 splitter 的双击回默认档位（`ui/app.slint` 的
//! `double-clicked => { … root.console-height-reset(); }`，宿主 `host.rs` 的
//! `ui.on_console_height_reset`）。本票把同一条手势给混音台的推子 / 声相。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① | `mixer_console.slint` 里**恰好**四块命中面带复位臂；每一块**往上**读得到它**自己**的组件头（推子 = 共享 `Fader`、声相 = `TouchArea`）与 `x/y/width/height`（几何一位没动），**往下**读得到**它自己**那条回调（⛔ 串面：把声相臂贴到推子矩形上 ⇒ 红）。⚠ 形态已变（2026-11-24 复核）：推子的双击**不在** `mixer_console` 里 —— 它走共享 `Fader` 组件的 `reset-requested`（`ui/controls/fader.slint:138` 把 `double-clicked` 映射成 `reset-requested`）；`mixer_console` 代码面的 `double-clicked` 只剩两处（两个声相） |
//! | ② | 四个回调在 `mixer_console` / `console_tabs` / `app` **三个**文件里各**声明一次**；`console_tabs` 各**转发一次**；`app.slint` 按实例转发 —— 轨道两条各 **3** 次、主控推子 **1** 次、主控声相 **0** 次（⚠ **已知缺口读数**：活混音台 `UnifiedLowerDock` 根本没有主控声相控件，`MixerConsole` 只在 `if false` 兼容块里实例化 ⇒ `mixer-master-pan-reset` 在活界面上没有路；补控件或接受缺口是待裁项，⛔ 本票捏造转发） |
//! | ③ | 宿主四条 `on_*_reset` 臂各**恰好一次**，每臂里 `end_mixer_gesture` 出现在 `commit_mixer_reset` **之前**，且写的是**自己**那一格（`MixerResetField::Volume` / `::Pan`） |
//! | ④ | 中性值**只有一处**定义：`commit_mixer_reset` 定义一次、被调用四次；`new_val: 0.0` 在整个 `host.rs` 里**恰好一次** ⇒ 界面上不存在第二份"默认值"常量与它分叉 |
//!
//! ## ⛔ 本文件不证的东西（正对照在哪）
//!
//! 文本层证的是"接线存在且唯一"，**不证**"双击真的改了工程"。后者由
//! `tests/live_ui_mcp.rs` 的两条活体判据负责（`a_double_click_reset_moves_the_fader_and_pan_to_neutral`
//! / `a_double_click_reset_leaves_no_sticky_gesture`）：它们驱动**活的** `MainWindow`
//! 从权威工程读回数值。两半不可互相代替 —— 只看文本会在"回调声明了但宿主没接"这种
//! 状态下全绿，只看活体则钉不住"中性值只有一处定义"。
//!
//! ## 变异表（每条都实测过能变红）
//!
//! | 注入 | 文件 | 预期 |
//! | :--- | :--- | :--- |
//! | 把声相命中面的 `root.mixer-pan-reset(track_index);` 换成 `root.mixer-fader-reset(track_index);` | `ui/console/mixer_console.slint` | ① 红（第 2 面的调用不符） |
//! | 删掉 `console_tabs.slint` 里 `mixer-master-pan-reset()` 的转发臂 | `ui/console/console_tabs.slint` | ② 红（该文件转发计数 1 → 0） |
//! | 删掉某一臂里的 `end_mixer_gesture(&ui, &gesture);` | `src/host.rs` | ③ 红（找不到清零点，`commit` 之前无 `end`） |
//! | 把某一臂的 `MixerResetField::Pan` 改成 `::Volume` | `src/host.rs` | ③ 红（写错格）＋活体判据 ② 红（声相没动） |
//! | 在 `commit_mixer_reset` 之外再写一处 `new_val: 0.0` | `src/host.rs` | ④ 红（计数 1 → 2） |
//! | 给活混音台补一个主控声相控件并转发 `mixer-master-pan-reset` | `ui/lower_dock.slint` + `ui/app.slint` | ② 红（`app.slint` 转发计数 0 → 1 —— 修掉缺口时必须同步更新这张钉数） |

#![allow(unused)]
const MIXER_CONSOLE: &str = include_str!("../ui/console/mixer_console.slint");
const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const APP: &str = include_str!("../ui/app.slint");
const HOST: &str = include_str!("../src/host.rs");
const FADER: &str = include_str!("../ui/controls/fader.slint");

/// 返回 `source` 里含 `needle` 的**行号**（1 基，按文档顺序）。
///
/// 数行而不是数子串：命中面与回调的**配对**关系要看"哪一块矩形上面写着什么"，
/// 全文 `matches()` 会把四块矩形混成一堆无上下文的计数。
fn line_numbers(source: &str, needle: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(needle))
        .map(|(nth, _)| nth + 1)
        .collect()
}

/// 取包含 `needle` 的那一行所在的组件块（从最近的组件头行起，到花括号配平止）。
///
/// 组件头只认 `Fader {` / `TouchArea {`（本文件命中面的两种形态）；行注释按 `//`
/// 截断后再数花括号（这些块里的字符串字面量不含花括号与 `//`）。
fn enclosing_component<'a>(lines: &[&'a str], needle: &str) -> Vec<&'a str> {
    let at = lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("源码里找不到 `{needle}`"));
    let mut start = at;
    loop {
        let head = lines[start].trim_start();
        if head.starts_with("Fader {") || head.starts_with("TouchArea {") {
            break;
        }
        assert!(
            start > 0,
            "回调 `{needle}` 上方找不到 Fader/TouchArea 组件头"
        );
        start -= 1;
    }
    let mut depth = 0_i32;
    let mut end = start;
    for (index, line) in lines.iter().enumerate().skip(start) {
        let code = line.split("//").next().unwrap_or("");
        depth += code.matches('{').count() as i32;
        depth -= code.matches('}').count() as i32;
        end = index;
        if depth == 0 {
            break;
        }
    }
    assert!(
        depth == 0,
        "组件块从第 {} 行起没有闭合",
        start + 1
    );
    lines[start..=end].to_vec()
}

/// 命中面 `(x, y, width, height)` 四行 ＋ 它应当调用的那条回调 ＋ 复位臂 ＋ 组件形态。
type HitRect<'a> = ([&'a str; 4], &'a str, &'a str, &'a str);

/// 文档顺序：通道条推子 → 通道条声相 → 主控推子 → 主控声相
/// （与 `mixer_console.slint` 里两个组件的排布同序）。
///
/// ⚠ 推子两行的臂是 `reset-requested =>`（共享 `Fader` 组件把双击映射过来），
/// 声相两行的臂才是 `double-clicked =>`（`TouchArea` 直绑）。
const TRACK_AND_MASTER_HIT_RECTS: [HitRect<'_>; 4] = [
    (
        ["x: 22px;", "y: 54px;", "width: 48px;", "height: 116px;"],
        "root.mixer-fader-reset(track_index);",
        "reset-requested =>",
        "Fader",
    ),
    (
        [
            "x: Tokens.space-3;",
            "y: 18px;",
            "width: 36px;",
            "height: 18px;",
        ],
        "root.mixer-pan-reset(track_index);",
        "double-clicked =>",
        "TouchArea",
    ),
    (
        ["x: 24px;", "y: 54px;", "width: 48px;", "height: 116px;"],
        "root.mixer-master-fader-reset();",
        "reset-requested =>",
        "Fader",
    ),
    (
        [
            "x: Tokens.space-3;",
            "y: 18px;",
            "width: 36px;",
            "height: 18px;",
        ],
        "root.mixer-master-pan-reset();",
        "double-clicked =>",
        "TouchArea",
    ),
];

/// ① 四块命中面各自绑对自己的回调，几何一位没动。
/// 源码读数口径：去掉 `//` 行注释再计数（注释里提到的回调名不算接线）。
const MIXER_CONSOLE: &str = include_str!("../ui/console/mixer_console.slint");
const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const APP: &str = include_str!("../ui/app.slint");
const HOST_RS: &str = include_str!("../src/host.rs");
/// 共享推子控件：推子两面的双击先翻成 `reset-requested`（见 ①）。
const FADER: &str = include_str!("../ui/controls/fader.slint");

fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ① 四块命中面各自绑对自己的回调，几何一位没动。
///
/// 新 UI 形态（`Fader` 共享控件落地后）：推子两面不再直连 `double-clicked`，
/// 而是 `Fader` 控件的 `TouchArea.double-clicked` 翻成 `reset-requested` 再由
/// 实例臂转发；声相两面仍是 `TouchArea` 直连 `double-clicked`。两条链都钉住。
#[test]
fn each_mixer_hit_rect_binds_its_own_double_click_reset() {
    let fader = code_only(FADER);
    assert!(
        fader.contains("callback reset-requested();"),
        "Fader 控件必须声明 reset-requested 回调"
    );
    assert_eq!(
        fader.matches("double-clicked").count(),
        1,
        "Fader 控件里双击翻复位必须恰好一处"
    );
    assert!(
        fader.contains("root.reset-requested();"),
        "Fader 的 double-clicked 臂必须转发 reset-requested"
    );

    let src = code_only(MIXER_CONSOLE);
    assert_eq!(
        src.matches("double-clicked").count(),
        2,
        "混音台里直连 double-clicked 的必须是声相两面（推子走 Fader.reset-requested）"
    );
    assert_eq!(
        src.matches("reset-requested =>").count(),
        2,
        "混音台里经 reset-requested 复位的必须是推子两面"
    );
    for (rect, callback) in TRACK_AND_MASTER_HIT_RECTS {
        let lines: Vec<&str> = src.lines().collect();
        let call = lines
            .iter()
            .position(|line| line.contains(callback))
            .unwrap_or_else(|| panic!("混音台里找不到复位调用 {callback}"));
        // 往上找到所属的组件块：推子是 `Fader {`，声相是 `TouchArea {`。
        let mut start = call;
        while start > 0
            && !(lines[start].contains("Fader {") || lines[start].contains("TouchArea {"))
        {
            start -= 1;
        }
        assert!(
            lines[start].contains("Fader {") || lines[start].contains("TouchArea {"),
            "复位调用 {callback} 必须在 Fader / TouchArea 块内"
        );
        let block = lines[start..=call].join("\n");
        for geo in rect {
            assert!(
                block.contains(geo),
                "命中面 {callback} 的几何 {geo} 必须在同一块内（一位没动）"
            );
        }
        assert!(
            block.contains("double-clicked") || block.contains("reset-requested"),
            "命中面 {callback} 的块内必须有双击臂（double-clicked / reset-requested）"
        );
    }
}

/// ② 四个回调从组件一路转发到 `MainWindow`，每一段**恰好**一次。
///
/// 实测口径（2026-11-24 活树）：`app.slint` 里有三处 `ConsoleTabs` 实例，
/// 故推子/声相转发各 3 次；主控推子只在第三处转发 1 次；⚠ 主控声相在
/// `app.slint` 里**一次转发都没有**（`root.mixer-master-pan-reset(` 命中 0）——
/// 宿主臂存在但从 `app.slint` 到不了，是新 UI 留下的断链读数，钉住它而不是
/// 假装「各转发一次」。
#[test]
fn the_four_reset_callbacks_forward_to_the_main_window_exactly_once_each() {
    let cases: &[(&str, &str, usize, usize, usize)] = &[
        // (回调, 签名, console_tabs 转发, app 转发, mixer_console 调用)
        ("mixer-fader-reset", "mixer-fader-reset(int)", 1, 3, 1),
        ("mixer-pan-reset", "mixer-pan-reset(int)", 1, 3, 1),
        ("mixer-master-fader-reset", "mixer-master-fader-reset()", 1, 1, 1),
        ("mixer-master-pan-reset", "mixer-master-pan-reset()", 1, 0, 1),
    ];
    for (name, signature, tabs_forward, app_forward, console_call) in cases {
        let decl = format!("callback {signature};");
        assert_eq!(
            code_only(MIXER_CONSOLE).matches(&decl).count(),
            1,
            "mixer_console 里 {name} 必须声明一次"
        );
        assert_eq!(
            code_only(CONSOLE_TABS).matches(&decl).count(),
            1,
            "console_tabs 里 {name} 必须声明一次"
        );
        assert_eq!(
            code_only(APP).matches(&decl).count(),
            1,
            "app 里 {name} 必须声明一次"
        );
        let forward = format!("root.{name}(");
        assert_eq!(
            code_only(CONSOLE_TABS).matches(&forward).count(),
            *tabs_forward,
            "console_tabs 里 {name} 的转发次数"
        );
        assert_eq!(
            code_only(APP).matches(&forward).count(),
            *app_forward,
            "app 里 {name} 的转发次数"
        );
        assert_eq!(
            code_only(MIXER_CONSOLE).matches(&forward).count(),
            *console_call,
            "mixer_console 里 {name} 的调用次数"
        );
    }
}

/// ③ 宿主四条臂：先清手势、后提交，而且写的是自己那一格。
#[test]
fn every_host_reset_arm_clears_the_gesture_before_committing_its_own_field() {
    let host = code_only(HOST_RS);
    let arms: &[(&str, &str)] = &[
        ("ui.on_mixer_fader_reset(", "MixerResetField::Volume"),
        ("ui.on_mixer_pan_reset(", "MixerResetField::Pan"),
        ("ui.on_mixer_master_fader_reset(", "MixerResetField::Volume"),
        ("ui.on_mixer_master_pan_reset(", "MixerResetField::Pan"),
    ];
    for (arm, field) in arms {
        assert_eq!(
            host.matches(arm).count(),
            1,
            "宿主里 {arm} 必须恰好一条臂"
        );
        let start = host.find(arm).expect("上面刚数过一次");
        let rest = &host[start..];
        let end = rest
            .find("end_mixer_gesture")
            .expect("臂里必须清手势");
        let commit = rest
            .find("commit_mixer_reset(")
            .expect("臂里必须提交复位");
        assert!(
            end < commit,
            "臂 {arm} 必须先清手势再提交（end@{end} commit@{commit}）"
        );
        assert!(
            rest[commit..commit + 400].contains(field),
            "臂 {arm} 提交的必须是自己那一格 {field}"
        );
    }
}

/// ④ 中性值只有一处定义：落点唯一、`0.0` 只出现一次。
#[test]
fn the_neutral_value_is_defined_in_exactly_one_place() {
    let host = code_only(HOST_RS);
    assert_eq!(
        host.matches("fn commit_mixer_reset(").count(),
        1,
        "commit_mixer_reset 必须只定义一次"
    );
    assert_eq!(
        host.matches("commit_mixer_reset(").count(),
        5,
        "commit_mixer_reset 出现 5 次 = 1 定义 + 4 条臂各调用一次"
    );
    assert_eq!(
        host.matches("new_val: 0.0").count(),
        1,
        "new_val: 0.0 在整个 host.rs 里必须恰好一次"
    );
}
