//! 第七轮 `[PATCH UI-FEEDBACK-01]` 常驻门禁：把「用户操作感受最明显」的两类改进
//! 钉成不可回归的判据。
//!
//! ## 为什么这一轮不是「补接线」
//!
//! 前六轮收敛的是「底层能力已有、只缺接线」与零像素一致性。本轮按用户新定标改为
//! 优先**人眼与手能直接感到**的缺陷，复核后确认两类真缺口：
//!
//! | 类 | 缺口 | 落点 |
//! | :-- | :-- | :-- |
//! | A 状态不可见 | `console-maximize-button` 有 `accessible-checked`、`transport-ai-drawer-button` 有 `accessible-expanded`，但人眼看到的板 / 字恒定 ⇒ 用户点完不知道当前处于哪个态 | 金底 = 当前态 + 文字反色 |
//! | A 反馈缺失 | 总览面板 6 个控制键、4 个缩放 ± 键的 `TouchArea` 只有 `clicked`，悬浮 / 按下一位不动 | 命名 `TouchArea` + `has-hover` 金边 / `pressed` 板提亮 |
//! | B Esc 抢先 | `Action::Cancel` 级联里 AI 抽屉臂在全屏臂之前，而 compact 抽屉是**根级覆盖层**（`visible: root.compact && root.ai-drawer-open`，不含 `!song-overview-fullscreen`）、可与全屏同开 ⇒ 组合态按 `Esc` 退不出全屏 | 全屏臂提到抽屉臂之前 |
//!
//! ## 默认帧零位移（golden 不重录的前提）
//!
//! 每个新增三元的**最后一个分支逐字等于原静态值**：`has-hover=false` / `pressed=false`
//! / 状态为默认时，渲染出的颜色与改动前完全相同。③ 用精确计数钉住这些回落分支，
//! 谁把默认帧颜色改了就红 —— 这是本轮不需要重录 Linux golden 的机械保证。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | 把某键的 false 分支改成别的 token | ③ 红（默认帧漂移） |
//! | 删掉某 `TouchArea` 的 `:=` 命名 | ② 红（反馈表达式悬空 ⇒ 编译期也会红） |
//! | 去掉状态色三元、退回静态板 | ① 红 |
//! | 把全屏臂挪回抽屉臂之后 | ④ 源码顺序红 + ⑤ 活体红 |
//! | 组合态按 `Esc` 只关抽屉 | ⑤ 红 |
//!
//! ⛔ 本文件**不**重复钉可寻址节点总数（`round6_addressable_roles.rs`）与
//! `accessible-value:` 计数（`song_overview_panel.rs`）：本轮零新增可寻址节点、
//! 零新增无障碍读数，已有判据照旧覆盖。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `ai_drawer_key_ui.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;

const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const TRANSPORT: &str = include_str!("../ui/transport.slint");
const SONG_OVERVIEW: &str = include_str!("../ui/workspace/song_overview_panel.slint");
const ARRANGEMENT: &str = include_str!("../ui/workspace/arrangement_view.slint");
const HOST_RS: &str = include_str!("../src/host.rs");

/// 把 `//` 注释与 `"..."` 字符串内容替换为等长空格（保留花括号结构），供块配平使用。
/// 四个目标 `.slint` 文件都没有 `/* */` 块注释（已核），故只处理行注释。
fn mask(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0usize;
    let mut in_comment = false;
    let mut in_string = false;
    while i < bytes.len() {
        let byte = bytes[i];
        if in_comment {
            if byte == b'\n' {
                in_comment = false;
            } else {
                out[i] = b' ';
            }
        } else if in_string {
            if byte == b'"' {
                in_string = false;
            } else {
                out[i] = b' ';
            }
        } else if byte == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            in_comment = true;
            out[i] = b' ';
        } else if byte == b'"' {
            in_string = true;
        }
        i += 1;
    }
    String::from_utf8(out).expect("等长字节替换后仍是合法 UTF-8")
}

/// 以 `accessible-id` 为锚，返回它所在的那一层元素块（向前找最近的未配平 `{`，
/// 向后配平到匹配的 `}`）。与 `round6_addressable_roles.rs` 同源，但这里只用于
/// 把断言收窄到「这个按钮」，避免邻块的同名 token 造成假绿。
fn enclosing_block<'a>(src: &'a str, masked: &str, id: &str) -> &'a str {
    let pos = src.find(id).unwrap_or_else(|| panic!("缺可寻址 id `{id}`"));
    let bytes = masked.as_bytes();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut i = pos;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'}' => depth += 1,
            b'{' => {
                if depth == 0 {
                    start = i;
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let mut level = 0i32;
    let mut end = bytes.len();
    for (j, byte) in bytes.iter().enumerate().skip(start) {
        match *byte {
            b'{' => level += 1,
            b'}' => {
                level -= 1;
                if level == 0 {
                    end = j + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    &src[start..end]
}

/// ① 切换态必须**画给人看**，且默认帧逐字回落到原来的底色 / 字色。
#[test]
fn toggle_buttons_show_their_state_without_changing_the_default_frame() {
    let maximize = enclosing_block(CONSOLE_TABS, &mask(CONSOLE_TABS), "console-maximize-button");
    assert!(
        maximize.contains("root.console-maximized ? Tokens.gold"),
        "最大化键必须把「当前态」画给人看（金底）: {maximize}"
    );
    assert!(
        maximize.contains(": Tokens.bg-panel-alt;"),
        "默认帧（未最大化、未悬浮、未按下）必须逐字回落到原底色 `bg-panel-alt`: {maximize}"
    );
    assert!(
        maximize.contains("color: root.console-maximized ? #060A14 : Tokens.ink-1;"),
        "金底上的字必须反色，默认帧仍是 `ink-1`: {maximize}"
    );

    let drawer = enclosing_block(TRANSPORT, &mask(TRANSPORT), "transport-ai-drawer-button");
    assert!(
        drawer.contains("root.ai-drawer-open ? Tokens.gold"),
        "AI 抽屉键必须把「展开态」画给人看（金底）: {drawer}"
    );
    assert!(
        drawer.contains(": Tokens.bg-panel-alt;"),
        "默认帧（未展开、未悬浮、未按下）必须逐字回落到原底色 `bg-panel-alt`: {drawer}"
    );
    assert!(
        drawer.contains("color: root.ai-drawer-open ? #060A14 : Tokens.gold;"),
        "金底上的「AI」必须反色，默认帧仍是 `gold`: {drawer}"
    );
}

/// ② 每个目标 `TouchArea` 都必须显式命名，且那个名字真的驱动了视觉反馈表达式。
/// 未命名 ⇒ `has-hover` / `pressed` 无从引用（Slint 编译期也会红，这里是源码级早报）。
#[test]
fn every_target_touch_area_is_named_and_drives_a_visual_feedback() {
    let cases: [(&str, &str, &str); 9] = [
        ("console_tabs.slint", CONSOLE_TABS, "mx_touch"),
        ("transport.slint", TRANSPORT, "drawer_area"),
        ("song_overview_panel.slint", SONG_OVERVIEW, "pill_touch"),
        ("song_overview_panel.slint", SONG_OVERVIEW, "collapse_touch"),
        (
            "song_overview_panel.slint",
            SONG_OVERVIEW,
            "fullscreen_touch",
        ),
        ("arrangement_view.slint", ARRANGEMENT, "zoom_h_out_touch"),
        ("arrangement_view.slint", ARRANGEMENT, "zoom_h_in_touch"),
        ("arrangement_view.slint", ARRANGEMENT, "zoom_v_out_touch"),
        ("arrangement_view.slint", ARRANGEMENT, "zoom_v_in_touch"),
    ];
    for (file, src, name) in cases {
        assert!(
            src.contains(&format!("{name} := TouchArea")),
            "{file}: `{name}` 必须显式命名，否则 `has-hover` / `pressed` 无从引用"
        );
        assert!(
            src.contains(&format!("{name}.has-hover")) || src.contains(&format!("{name}.pressed")),
            "{file}: `{name}` 必须至少驱动一个视觉反馈表达式"
        );
    }
}

/// ③ 默认帧零位移：所有新增三元的 false 分支逐字等于改动前的静态 token。
/// 计数用 `assert_eq!` 而不是「至少」—— 多一条就是有人偷偷改了默认帧或加了第二套口径。
#[test]
fn default_frame_falls_back_to_the_original_tokens() {
    assert_eq!(
        SONG_OVERVIEW
            .matches("? Tokens.gold : Tokens.line;")
            .count(),
        3,
        "胶囊 / 折叠键 / 全屏键三条 hover 金边的 false 分支必须都是 `line`"
    );
    assert_eq!(
        SONG_OVERVIEW.matches("? Tokens.bg-raised").count(),
        3,
        "胶囊 / 折叠键 / 全屏键三条按下提亮"
    );
    assert_eq!(
        SONG_OVERVIEW
            .matches("? Tokens.bg-raised : Tokens.bg-panel;")
            .count(),
        1,
        "折叠键按下的 false 分支必须是 `bg-panel`"
    );
    assert_eq!(
        SONG_OVERVIEW
            .matches("? Tokens.gold : Tokens.bg-panel);")
            .count(),
        2,
        "胶囊的状态三元与全屏键的状态三元必须原样保留（各自一个 `gold : bg-panel`）"
    );
    assert_eq!(
        ARRANGEMENT
            .matches("? Tokens.bg-raised : Tokens.bg-panel;")
            .count(),
        4,
        "四个缩放 ± 键的 false 分支必须都是 `bg-panel`"
    );
    assert_eq!(
        ARRANGEMENT.matches("_touch := TouchArea").count(),
        4,
        "四个缩放 ± 键各自命名了 `TouchArea`"
    );
}

/// ④ 源码顺序：全屏臂必须排在 AI 抽屉臂之前（只在 `Action::Cancel` 段内比对，
/// 避免命中文件别处的同名 getter）。
#[test]
fn escape_exits_fullscreen_before_the_ai_drawer_in_the_source() {
    let cancel = HOST_RS
        .find("Action::Cancel => {")
        .expect("`apply_action` 必须有 `Action::Cancel` 臂");
    let tail = &HOST_RS[cancel..];
    let fullscreen = tail
        .find("if ui.get_song_overview_fullscreen()")
        .expect("级联里必须有全屏臂");
    let drawer = tail
        .find("if ui.get_ai_drawer_open()")
        .expect("级联里必须有抽屉臂");
    assert!(
        fullscreen < drawer,
        "全屏臂必须排在 AI 抽屉臂之前（组合态下 `Esc` 先退全屏）; \
         fullscreen@{fullscreen} drawer@{drawer}"
    );
}

/// ⑤ 活体：构造「全屏 + 抽屉同开」的组合态，按一次 `Esc`。
///
/// ⚠ 夹具边界（如实登记）：`build_live_ui_with` 装的是 `host::wire_keys`，**没有**
/// `song_overview::wire_song_overview` ⇒ `invoke_song_overview_toggle_fullscreen()`
/// 在本夹具里是 no-op，`song-overview-fullscreen` 不会被翻转。真正「`Esc` 退出全屏」
/// 的端到端断言由 `song_overview_panel.rs`（产品装配腿）承担。
///
/// 因此这里用**抽屉有没有被关**来判级联选中的是哪一臂：全屏臂在前 ⇒ 抽屉不动；
/// 抽屉臂在前 ⇒ `ai-drawer-open` 被翻成 false。`invoke_toggle_ai_drawer` 在本夹具
/// 有产品同一个注册者（`ai_drawer_key_ui.rs` 已证），故这个观测是真的。
#[test]
fn escape_combination_prefers_fullscreen_in_the_live_window() {
    let project = filled_project();
    let ui = live::build_live_ui_with(
        &project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 端口");
    let window = slint::ComponentHandle::clone_strong(ui.ui());

    // 组合态前置：全屏开着（直接置视图态属性；`Action::Cancel` 只读它的 getter），
    // 抽屉开着（走唯一写者 `toggle-ai-drawer`，与按钮 / 快捷键同一条回调）。
    window.set_song_overview_fullscreen(true);
    window.invoke_toggle_ai_drawer();
    assert!(window.get_song_overview_fullscreen(), "前置：全屏态");
    assert!(window.get_ai_drawer_open(), "前置：抽屉展开");

    assert!(
        window.invoke_key_action("\u{1b}".into(), false, false, false, false),
        "组合态下 `Escape` 必须被消费（有可收尾的面板）"
    );
    assert!(
        window.get_ai_drawer_open(),
        "组合态按一次 `Esc` 必须**先**选全屏臂 ⇒ 抽屉留待第二次 `Esc`；\
         若 `ai-drawer-open` 翻成 false，说明抽屉臂又抢在了全屏之前"
    );
}
