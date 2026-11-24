//! T-FIX-64（2026-11-19 走查残留收尾）的源码层看守。
//!
//! 三条判据都只读 `.slint` 源码文本（⛔ 不编译 Slint），与
//! `ui_toast_legibility.rs` 同一门手艺：钉住「修过的形态不许悄悄退回去」。
//!
//! | # | 判据 | 退回即红的形态 |
//! | :-- | :--- | :--- |
//! | ① | `app.slint` 的提示条 Timer 触发时必须**清文案** | 只清 `show-floating-toast`（横幅可见条件是「有文案」，旧写法自动收走从未生效） |
//! | ② | 走查点名的两处字形残留不得复现 | `startup_chooser.slint` / `song_overview_panel.slint` 里再出现 `▾` / `▸` 字面量（UI 字体无此字形，必须画 Path 三角） |
//!
//! ⚠ 2026-11-24（T-FIX-356）：判据 ②b 已随死文件 `ui/workspace/song_overview_panel.slint`
//! 一起删除——它守的那把折叠键只活在那个死文件里，活树里没有对应物可重定向（
//! `grep "总览（已折叠）"` 在活文件里零命中）。删的不是"放宽"，是"守的对象不存在了"；
//! ②a（启动选择器，活文件）与 ①③ 一位未动。
//! | ③ | 下坞混音面 MASTER 条之前必须有可伸缩空格 | MASTER 紧跟末条通道左排、右沿留空（走查 #13） |

#![allow(unused)]

const APP: &str = include_str!("../ui/app.slint");
const STARTUP_CHOOSER: &str = include_str!("../ui/dialogs/startup_chooser.slint");
// ⚠ `SONG_OVERVIEW_PANEL` 常量已随判据 ②b 删除（死文件，见 `docs/DEAD-SLINT-FILES.md`）。
const LOWER_DOCK: &str = include_str!("../ui/lower_dock.slint");

/// 判据 ①：自动收走必须清文案 —— Timer 的 `triggered` 块里要有
/// `root.toast-message = "";`，且计时条件要看文案本身（只看显位的话，
/// 一批只写文案的入口永远等不到收走）。
#[test]
fn the_toast_timer_really_dismisses_by_clearing_the_message() {
    let timer_start = APP
        .find("interval: 4s;")
        .expect("app.slint 必须有 4 秒提示条 Timer");
    let timer_block = &APP[timer_start..APP.len().min(timer_start + 600)];
    assert!(
        timer_block.contains("root.toast-message = \"\";"),
        "Timer 触发必须清文案，否则横幅（可见条件 = 有文案）永远收不走"
    );
    assert!(
        timer_block.contains("running: root.toast-message != \"\""),
        "Timer 的 running 条件必须看文案，只看 show-floating-toast 会漏掉只写文案的入口"
    );
}

/// 判据 ②a：启动选择器的详情折叠箭头必须是画出来的 Path，不许再用字形。
#[test]
fn startup_chooser_draws_its_collapse_arrow() {
    assert!(
        !STARTUP_CHOOSER.contains('▾') && !STARTUP_CHOOSER.contains('▸'),
        "startup_chooser 不得再出现 ▾ / ▸ 字面量（UI 字体无此字形）"
    );
    assert!(
        STARTUP_CHOOSER.contains("M 0 1.5 L 7 1.5 L 3.5 5.5 Z"),
        "折叠箭头必须是与 left_panel 同一门手艺的 7×7 Path 三角"
    );
}

/// 判据 ③：MASTER 条与通道组之间必须隔着可伸缩空格，MASTER 才钉得住右沿。
#[test]
fn the_lower_dock_master_strip_is_spaced_to_the_right_edge() {
    let master_at = LOWER_DOCK
        .find("Dedicated Master 2-Bus Channel Strip")
        .expect("lower_dock 必须有 MASTER 通道条");
    let before = &LOWER_DOCK[master_at.saturating_sub(1600)..master_at];
    assert!(
        before.contains("horizontal-stretch: 1;"),
        "MASTER 条之前 1600 字节内必须有 horizontal-stretch: 1 的空格，否则它跟排在通道后面、右沿留空"
    );
}
