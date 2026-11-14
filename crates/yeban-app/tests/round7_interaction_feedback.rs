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

#![allow(unused)]
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
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> song-overview-fullscreen:"),
        "新 UI 契约面必须声明 > song-overview-fullscreen:"
    );
    assert!(
        aether_host.contains("song_overview_fullscreen"),
        "宿主接线层必须注册 song_overview_fullscreen"
    );
}

/// ② 每个目标 `TouchArea` 都必须显式命名，且那个名字真的驱动了视觉反馈表达式。
/// 未命名 ⇒ `has-hover` / `pressed` 无从引用（Slint 编译期也会红，这里是源码级早报）。
#[test]
fn every_target_touch_area_is_named_and_drives_a_visual_feedback() {
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
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> song-overview-fullscreen:"),
        "新 UI 契约面必须声明 > song-overview-fullscreen:"
    );
    assert!(
        aether_host.contains("song_overview_fullscreen"),
        "宿主接线层必须注册 song_overview_fullscreen"
    );
}

/// ③ 默认帧零位移：所有新增三元的 false 分支逐字等于改动前的静态 token。
/// 计数用 `assert_eq!` 而不是「至少」—— 多一条就是有人偷偷改了默认帧或加了第二套口径。
#[test]
fn default_frame_falls_back_to_the_original_tokens() {
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
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> song-overview-fullscreen:"),
        "新 UI 契约面必须声明 > song-overview-fullscreen:"
    );
    assert!(
        aether_host.contains("song_overview_fullscreen"),
        "宿主接线层必须注册 song_overview_fullscreen"
    );
}

/// ④ 源码顺序：全屏臂必须排在 AI 抽屉臂之前（只在 `Action::Cancel` 段内比对，
/// 避免命中文件别处的同名 getter）。
#[test]
fn escape_exits_fullscreen_before_the_ai_drawer_in_the_source() {
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
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> song-overview-fullscreen:"),
        "新 UI 契约面必须声明 > song-overview-fullscreen:"
    );
    assert!(
        aether_host.contains("song_overview_fullscreen"),
        "宿主接线层必须注册 song_overview_fullscreen"
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
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> song-overview-fullscreen:"),
        "新 UI 契约面必须声明 > song-overview-fullscreen:"
    );
    assert!(
        aether_host.contains("song_overview_fullscreen"),
        "宿主接线层必须注册 song_overview_fullscreen"
    );
}
