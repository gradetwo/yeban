//! T-FIX-408 判据：录音中退出 / 关窗必须先结束 take 并落盘。
//!
//! 修前：`app-quit` 只 `hide() + quit_event_loop()`；窗口关闭直接 `HideWindow`。
//! 录音线程随进程消失，`Recorder` 的 session 只在 `stop()` 里写 WAV，因此文件从未存在。
//!
//! 本文件钉住四件事：
//! 1. 菜单退出走 `finish_recording_before_exit`，且不再误调 CLI 的 `invoke_record_wav`；
//! 2. 窗口关闭同样先收尾，收尾失败就 `KeepWindowShown`；
//! 3. `main.rs` 在事件循环返回后再兜一次；
//! 4. 收尾函数本身调用 `Recorder::stop()`（唯一写盘点），失败时返回 false。
#![allow(unused)]

const HOST: &str = include_str!("../src/host.rs");
const MAIN: &str = include_str!("../src/main.rs");

fn head_slice(s: &str, limit: usize) -> &str {
    let mut end = s.len().min(limit);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// ① 菜单退出：先收录音，再判脏；旧 `invoke_record_wav`（CLI 测试音）不得再出现。
#[test]
fn app_quit_finishes_recording_before_dirty_check() {
    let arm = HOST
        .split_once(r#""app-quit" =>"#)
        .expect("缺 app-quit 分派臂")
        .1;
    let arm = head_slice(arm, arm.find("other =>").unwrap_or(arm.len()).min(1200));
    assert!(
        arm.contains("finish_recording_before_exit(&ui)"),
        "app-quit 未走录音收尾：录音线程仍会随进程消失"
    );
    assert!(
        arm.contains("if !finish_recording_before_exit(&ui)"),
        "app-quit 未在收尾失败时拦下退出"
    );
    assert!(
        !arm.contains("invoke_record_wav()"),
        "app-quit 仍误调 CLI 的 record-wav 测试音入口，不会结束交互录音"
    );
    let stop_at = arm.find("finish_recording_before_exit").unwrap();
    let dirty_at = arm.find("current_is_dirty").unwrap();
    assert!(stop_at < dirty_at, "必须先收录音，再判未保存改动");
}

/// ② 窗口关闭：先收录音，失败保持窗口；不能直接 HideWindow 带走 take。
#[test]
fn window_close_finishes_recording_and_blocks_on_failure() {
    let close = HOST
        .split_once("on_close_requested(")
        .expect("缺窗口 close 拦截")
        .1;
    let close = head_slice(close, close.find("});").unwrap_or(close.len()).min(1600));
    assert!(
        close.contains("finish_recording_before_exit(&ui)"),
        "窗口关闭未走录音收尾"
    );
    assert!(
        close.contains("slint::CloseRequestResponse::KeepWindowShown"),
        "收尾失败未保持窗口"
    );
}

/// ③ 事件循环返回后最后兜一次：覆盖平台直接结束循环、close 回调未走的路径。
#[test]
fn main_has_last_chance_recording_stop() {
    assert!(
        MAIN.contains("host::finish_recording_before_exit(&ui)"),
        "main.rs 事件循环返回后未兜底停录"
    );
}

/// ④ 收尾函数本身必须调用 `Recorder::stop()`（唯一写盘点），且失败返回 false。
#[test]
fn exit_finish_calls_recorder_stop_and_reports_failure() {
    let func = HOST
        .split_once("pub fn finish_recording_before_exit")
        .expect("缺 finish_recording_before_exit")
        .1;
    let func = head_slice(func, 2200);
    assert!(func.contains(".stop()"), "收尾函数未调用 Recorder::stop()");
    assert!(func.contains("Err(error)"), "收尾函数未处理停录失败");
    assert!(
        func.contains("false"),
        "收尾失败未给调用方 false（退出会被错误放行）"
    );
}
