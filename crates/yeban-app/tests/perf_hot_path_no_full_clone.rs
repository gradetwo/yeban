//! **性能热路径：拖动预览不得深拷贝整份工程**（性能实测 P2-A）。
//!
//! 背景（`~/work/qoder/性能实测-a87a480.md` §2 P2-A）：`on_clip_drag_move` →
//! `preview_clip_drag` 原本 `port.try_project()`，而 `try_project` 的实现是
//! `session.project().clone()` —— **完整深拷贝**。拖动时 pointer-move 60–120 次/秒
//! ⇒ 每秒 60–120 次全工程克隆；工程变大后就是「越拖越卡」。
//!
//! 修法：`UndoPort::with_project`（`Local` 后端直接借用 `&YebanProjectV1`，
//! ⛔ 不 clone），拖动预览只取那一条 placement 的 `start_tick`。
//!
//! ⚠ 纯 `include_str!` 源码判据（本机编不了 Slint 重依赖）；行为由 CI 的
//! `yeban-app` 腿真跑。

const HOST: &str = include_str!("../src/host.rs");
const UNDO: &str = include_str!("../src/undo.rs");

/// 取某个顶层 `fn` 的函数体（到行首无缩进的 `}` 为止）。
///
/// ⚠ 先把 CRLF 归一到 LF：Windows 检出是 `\r\n}` ⇒ 直接 `find("\n}\n")`
/// 找不到（实测 CI windows 腿因此 panic）。
fn fn_body(src: &str, sig: &str) -> String {
    let normalized = src.lines().collect::<Vec<_>>().join("\n");
    let start = normalized
        .find(sig)
        .unwrap_or_else(|| panic!("必须存在 {sig}"));
    let tail = &normalized[start..];
    let end = tail.find("\n}\n").expect("函数必须有顶层结尾");
    tail[..end].to_string()
}

/// ① 拖动预览走借用式读，不再 clone 整份工程。
#[test]
fn clip_drag_preview_does_not_clone_the_whole_project() {
    let body = fn_body(HOST, "fn preview_clip_drag(");
    assert!(
        body.contains("with_project"),
        "拖动预览必须用 UndoPort::with_project 借用读（P2-A）"
    );
    assert!(
        !body.contains("try_project()"),
        "拖动预览不得再调 try_project()（那会 clone 整份工程，60–120 次/秒；\n\
         注释里提到该名字不算，这里钉的是调用形式）"
    );
}

/// ② 借用式读的实现确实是「借用 + 闭包」，不是又包一层 clone。
#[test]
fn undo_port_with_project_borrows_instead_of_cloning() {
    assert!(
        UNDO.contains("pub fn with_project<R>"),
        "UndoPort 必须提供 with_project 借用式读接口"
    );
    assert!(
        UNDO.contains("Some(f(session.project()))"),
        "Local 后端必须直接借用 session.project()（&YebanProjectV1），⛔ 不加 .clone()"
    );
}

/// ③ T-FIX-431：编曲区横向滚动/滚轮的重投影不再深拷贝整份工程。
///
/// 红对照：把 `reproject_with_scroll` 改回 `port.try_project()`（Local 后端会
/// `session.project().clone()`）⇒ 本判据立刻变红。
#[test]
fn scroll_reprojection_borrows_instead_of_cloning_the_project() {
    let body = fn_body(HOST, "fn reproject_with_scroll(");
    assert!(
        body.contains("with_project"),
        "滚动/滚轮重投影必须用 UndoPort::with_project 借用读（T-FIX-431）"
    );
    assert!(
        !body.contains("try_project()"),
        "滚动/滚轮重投影不得再调 try_project()（那一跳会 clone 整份工程）"
    );
}

/// ④ 滚动/滚轮的三条入口（空车道拖动、卷帘滚轮、编曲区滚轮/滚动条）都不走全量 clone。
#[test]
fn every_scroll_entry_point_avoids_the_full_project_clone() {
    let install_scroll = fn_body(HOST, "fn install_roll_scroll(");
    assert!(
        install_scroll.contains("with_project"),
        "空车道拖动 / 卷帘滚轮必须改用 with_project"
    );
    assert!(
        !install_scroll.contains("try_project()"),
        "install_roll_scroll 的两个滚动回调都不得再 try_project()"
    );

    let wheel = fn_body(HOST, "fn wire_arrangement_wheel(");
    assert!(
        wheel.contains("reproject_with_scroll"),
        "编曲区滚轮必须走同一个滚动重投影落点"
    );
    assert!(
        !wheel.contains("try_project()"),
        "编曲区滚轮不得自己再克隆一份工程"
    );
}
