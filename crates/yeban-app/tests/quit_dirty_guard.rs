//! T-FIX-208 判据：退出 / 关窗前的「未保存改动」保护（数据丢失级）。
//!
//! 修前：app-quit 与窗口关闭都直接 `quit_event_loop()` / 关窗，全仓无工程脏标记，
//!       单次误按即可丢掉全部未保存编辑。
//!
//! 本文件钉住五件事，每条都是「修前必红」：
//! | # | 判据 | 修前为什么红 |
//! | :-- | :--- | :--- |
//! | 1 | `dirty_guard` 纯逻辑模块 + 单测存在 | 模块不存在 |
//! | 2 | 保存成功（Saved 分支）才 `mark_saved` | 保存点从不被记录 |
//! | 3 | app-quit 臂先判 `current_is_dirty`，dirty 弹三选 | 直接 hide + quit |
//! | 4 | Windows 三选 overlay 用 quit-confirm-open + 三 callback | 属性 / callback 都不存在 |
//! | 5 | 产品装配调 `wire_quit_guard` + 窗口 close 拦截 | 无装配 |
//!
//! ⚠ 纯 `include_str!` 判据 + 纯逻辑单测（本机不编 Slint 全量）；
//!   head 比对的行为由 `dirty_guard.rs` 的 7 个单测真跑覆盖。
#![allow(unused)]

const HOST: &str = include_str!("../src/host.rs");
const MAIN: &str = include_str!("../src/main.rs");
const LIB: &str = include_str!("../src/lib.rs");
const APP: &str = include_str!("../ui/app.slint");
const GUARD: &str = include_str!("../src/dirty_guard.rs");

/// 截到 `limit` 字节以内、且不切断 UTF-8 字符的切片（中文多字节安全）。
fn head_slice(s: &str, limit: usize) -> &str {
    let mut end = s.len().min(limit);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// ① 纯逻辑模块存在、被 lib 声明、含 head 比对与保存点记账。
#[test]
fn dirty_guard_module_exists_and_is_declared() {
    assert!(
        LIB.contains("pub mod dirty_guard;"),
        "lib.rs 未声明 dirty_guard 模块"
    );
    assert!(
        GUARD.contains("pub struct SavedHead"),
        "缺 SavedHead 保存点结构"
    );
    assert!(
        GUARD.contains("pub fn mark_saved"),
        "缺 mark_saved 记账入口"
    );
    assert!(
        GUARD.contains("pub fn current_is_dirty"),
        "缺 current_is_dirty 判定"
    );
    assert!(
        GUARD.contains("current != *self.0.borrow()"),
        "脏判定必须走 head 指针比对（不是另设 dirty 布尔）"
    );
    // 行为单测真跑（撤销回保存点不脏 / 失败保存不挪点等）。
    assert!(
        GUARD.contains("fn undo_back_to_saved_point_is_clean"),
        "缺撤销回保存点的判据"
    );
    assert!(
        GUARD.contains("fn failed_save_does_not_move_the_point"),
        "缺失败保存的判据"
    );
}

/// ② 保存成功才记账 —— 失败 / 取消绝不动保存点。
#[test]
fn saved_head_is_marked_only_on_save_success() {
    let anchor = "if matches!(outcome, crate::save_action::SaveOutcome::Saved { .. })";
    assert!(HOST.contains(anchor), "保存回调缺「仅 Saved 分支」的门槛");
    let after = HOST.split_once(anchor).expect("缺 Saved 门槛").1;
    let window = head_slice(after, after.find("});").unwrap_or(after.len()).min(600));
    assert!(
        window.contains("crate::dirty_guard::mark_saved("),
        "Saved 分支未 mark_saved：保存成功没有前移保存点"
    );
}

/// ③ app-quit 臂先判脏：dirty 弹三选，干净才走原出口（且保留 quit_event_loop）。
#[test]
fn app_quit_interposes_dirty_check() {
    let arm = HOST
        .split_once(r#""app-quit" =>"#)
        .expect("缺 app-quit 分派臂")
        .1;
    let arm = head_slice(arm, arm.find("other =>").unwrap_or(arm.len()).min(800));
    assert!(
        arm.contains("crate::dirty_guard::current_is_dirty("),
        "app-quit 未判脏：仍能一键丢掉未保存改动"
    );
    assert!(
        arm.contains("set_quit_confirm_open(true)"),
        "dirty 时未弹三选框"
    );
    assert!(
        HOST.contains("slint::quit_event_loop()"),
        "干净路径仍必须走 quit_event_loop（既有出口判据，全局口径）"
    );
}

/// ④ Slint 侧三选框：属性 + 三 callback + overlay 三按钮。
#[test]
fn slint_has_quit_confirm_triple_choice() {
    assert!(
        APP.contains("in-out property <bool> quit-confirm-open"),
        "缺 quit-confirm-open 属性"
    );
    assert!(
        APP.contains("callback quit-save-and-exit()"),
        "缺「保存并退出」callback"
    );
    assert!(
        APP.contains("callback quit-discard-and-exit()"),
        "缺「直接退出」callback"
    );
    assert!(APP.contains("callback quit-cancel()"), "缺「取消」callback");
    // overlay 三按钮各自发一个 callback。
    assert!(
        APP.contains("root.quit-save-and-exit()"),
        "overlay 无保存并退出按钮"
    );
    assert!(
        APP.contains("root.quit-discard-and-exit()"),
        "overlay 无直接退出按钮"
    );
    assert!(APP.contains("root.quit-cancel()"), "overlay 无取消按钮");
    // Esc = 取消，优先级最高。
    let esc = APP
        .split_once("if (event.text == Key.Escape)")
        .expect("缺 Esc 处理")
        .1;
    let esc = head_slice(
        esc,
        esc.find("startup-chooser-open")
            .unwrap_or(esc.len())
            .min(400),
    );
    assert!(esc.contains("root.quit-cancel()"), "Esc 未取消退出三选");
}

/// ⑤ 产品装配 + 窗口关闭拦截 + 线程槽安装。
#[test]
fn product_assembly_wires_quit_guard() {
    assert!(
        HOST.contains("pub fn wire_quit_guard"),
        "缺 wire_quit_guard 装配函数"
    );
    assert!(
        MAIN.contains("host::wire_quit_guard(&ui, &undo_port)"),
        "main.rs 未装配退出保护"
    );
    assert!(
        HOST.contains("crate::dirty_guard::install("),
        "未安装保存点线程槽（app-quit / 保存记账无法共享真源）"
    );
    assert!(
        HOST.contains("on_close_requested("),
        "窗口关闭按钮未拦截（只挡了菜单退出）"
    );
    assert!(
        HOST.contains("slint::CloseRequestResponse::KeepWindowShown"),
        "dirty 关窗未保持窗口（会丢改动）"
    );
    // 初值 = 装配那一刻的 head（从文件打开的工程不误弹）。
    assert!(
        HOST.contains("saved.mark_saved(port.display().head)"),
        "保存点初值未设为当前 head（启动瞬间会误弹）"
    );
}
