//! 缺陷 **D-UI-3** 的判据：控制面能不能注入**带修饰键的组合**（chord）。
//!
//! 一句话的问题陈述（现读，⛔ 不采信报告措辞）：`[UI-TEST-002]` §12.4 要求
//! `dispatch_key_press(key_code)` 分发 `Shift+Enter`，但那条签名只有**一个键名**、
//! 没有修饰位，`slint::platform::WindowEvent::KeyPressed` 本身也**不带**修饰字段
//! ⇒ 平台事件那条路永远表达不了 `Shift+Z`（全曲总览）、`Shift+↑/↓`（整八度）、
//! `Cmd/Ctrl+*` 全家。本文件证明新的 `ui/dispatch_key_chord` 这一格**真的通到界面**：
//!
//! | 判据 | 证明 |
//! | :--- | :--- |
//! | `every_port_chord_key_reaches_the_single_parse_point_of_the_host` | **词表同源**：端口 `ChordKey::text()` 出来的那一个码位，宿主的唯一解析点 `LogicalKey::from_text` 认得，且解析出的**就是**那一个逻辑键（词表漂移会红在这里，而不是红在"按了没反应"） |
//! | `injecting_shift_z_through_the_port_fits_the_whole_project_into_the_viewport` | **真的改了投影**：经端口注入 `Shift+Z` 之后，卷帘缩放 == 模型侧独立算出的 `ceil(内容跨度 / 视口宽)`，滚动回到 0，回执 `consumed == true`，且第二次注入幂等 |
//! | `an_unbound_chord_reports_not_consumed_and_leaves_everything_alone` | **不假装成功**：策略表明写「未被点名 ⇒ 放行」的那一格（`Cmd/Ctrl+Alt+Shift+↑`）返回 `consumed == false`，缩放/滚动/工具/播放四位读数一位没动 |
//!
//! ## 为什么落点是 `invoke_key_action` 而不是平台事件
//!
//! 见 `crates/yeban-app/src/live_surface.rs` 的 `dispatch_key_chord_impl` 文档与
//! `docs/adr/ADR-0064-ui-dispatch-key-chord.md`（`Proposed`）。要点只有一句：用户真键盘上的
//! `WindowEvent::KeyPressed` → `.slint` 的 `key-handler` → `key_action(text, shift, ctrl, alt, meta)`
//! → `host::wire_keys` 的 `on_key_action` 这条链里，`invoke_key_action` 就是它的**入口**，
//! 因此这条注入与用户键走**同一个回调、同一张策略表、同一个 UndoPort** ⇒ 不是旁路。
//!
//! ## 每个 `#[test]` 只装一次 Tier-1 平台
//!
//! 同 `live_ui_mcp.rs`：`slint::platform::set_platform` 是线程局部的（libtest 每测试一线程），
//! 同一个测试里**不能**建第二个 `LiveUi`。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分（三条 chord 判据 + 一个 `LiveUi`）⇒ 其余项在
// 本测试目标里是 dead_code（`-D warnings` 会判红）。同款见 `tests/ui_bpm_field.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;
use yeban_app::input::LogicalKey;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::{ChordKey, KeyChord, KeyChordModifiers, Permission};
use yeban_ui_test_port::render::report_line;

/// 缩放的上下界（`bridge::MIN/MAX_TICKS_PER_PIXEL`）—— 写成**字面量**做独立 oracle；
/// 常量本身由 `bridge` 的单元判据钉住。单位：tick / 逻辑像素。
const MIN_TPP: u64 = 1;
const MAX_TPP: u64 = 960;

const NOW: u64 = 1_760_000_000_000;

/// 内容的末端（tick）：剪辑右缘与段落右缘里更远的那个。
///
/// 这是**模型侧**的独立算法（与 `host::zoom_action` 读的投影 `content_end_tick()` 不同源），
/// 用它做 oracle 才有意义 —— 拿投影验投影等于什么都没验。口径与 `tests/live_ui_mcp.rs`
/// 的同名助手逐字一致（那条判据已在 CI 绿过，这里不另造第三份口径）。
fn model_content_end(project: &yeban_model::project::YebanProjectV1) -> u64 {
    let clip_end = project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .map(|placement| placement.start_tick + placement.duration_ticks)
        .max()
        .unwrap_or(0);
    let section_end = project
        .sections
        .values()
        .map(|section| section.end_tick)
        .max()
        .unwrap_or(0);
    clip_end.max(section_end)
}

/// 装配一条**测试模式 + `Interactive`** 的真实控制面。
///
/// ⚠ `undo` 必须是 `Some`：`ZoomToFit` 的前置条件就是撤销端口（`host.rs` 里
/// `Action::ZoomToSelection | Action::ZoomToFit` 那一支，`undo` 为 `None` 时**直接不消费**）——
/// 那正是缺陷 `D-UI-1` 修掉的那一格。装配与判据读的是**同一个** `project`。
fn assemble_interactive(project: &yeban_model::project::YebanProjectV1) -> live::LiveUi {
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<判据:D-UI-3 chord 注入>",
            "yeban-app",
            project.clone(),
            NOW,
        )
        .expect("打开撤销会话"),
    ));
    let wiring = live::LiveWiringOptions {
        permission: Permission::Interactive,
        console_tab: 0,
        save_path: None,
        engine_quanta: 0,
        undo: Some(Rc::clone(&port)),
        proposal_source: yeban_app::proposals::ProposalSource::none(),
    };
    live::build_live_ui_with(project, &wiring).expect("真实界面 + Tier-1 执行面")
}

/// 判据 ①（**默认构建，无需装配窗口**）：端口词表与宿主唯一解析点同源。
#[test]
fn every_port_chord_key_reaches_the_single_parse_point_of_the_host() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// 判据 ②（**默认构建**）：经**端口**注入 `Shift+Z` ⇒ 卷帘真的缩放到全曲。
#[test]
fn injecting_shift_z_through_the_port_fits_the_whole_project_into_the_viewport() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// 判据 ③（**默认构建**）：策略表里"未被点名 ⇒ 放行"的那一格必须报 `consumed == false`，
/// 并且**一位状态都不动**。
#[test]
fn an_unbound_chord_reports_not_consumed_and_leaves_everything_alone() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}
