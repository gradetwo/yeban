//! Toast 文案可读性的源码层看守（report-3 §7 #10）。
//!
//! 第四轮实测：`ToastBanner` 的气泡固定 310×34px，气泡内 Text 只有
//! `overflow: elide`、没有换行，控制室 toast（含 DIM/MONO/SIDE 四个字段）
//! 被单行截成「· ...」，用户无法据此确认监听状态。
//!
//! 修复是最小零布局改动：给该 Text 加 `wrap: word-wrap`（两行 10px 字仍在
//! 34px 气泡内），elide 仅作超出两行后的兜底。默认帧不渲染 toast ⇒ 不动
//! golden 基准。`wrap` + `overflow: elide` 同块用法在 `ui/sidebar.slint:310`
//! 已有编译通过的先例。
//!
//! 本判据只读 `.slint` 源码文本（⛔ 不编译 Slint），把「ToastBanner 的
//! toast 文案 Text 必须允许换行」钉住：有人把 wrap 删回单行 elide 即变红。

#![allow(unused)]

const MODALS: &str = include_str!("../ui/modals.slint");

/// 取 `export component ToastBanner` 的组件体（到文件末尾，它是最后一个组件）。
fn toast_banner_body() -> &'static str {
    let start = MODALS
        .find("export component ToastBanner")
        .expect("modals.slint 必须有 ToastBanner");
    &MODALS[start..]
}

/// **判据 ①**：ToastBanner 的 toast 文案 Text 必须 `wrap: word-wrap`。
#[test]
fn toast_banner_text_wraps_instead_of_single_line_elide() {
    let body = toast_banner_body();
    assert!(
        body.contains("text: root.toast-text;"),
        "ToastBanner 必须渲染 root.toast-text"
    );
    assert!(
        body.contains("wrap: word-wrap;"),
        "ToastBanner 的 toast 文案必须允许换行，否则长文案被截成「· ...」"
    );
}

/// **判据 ②**：换行与 elide 兜底同时保留（与 ui/sidebar.slint:310 同一口径）。
#[test]
fn toast_banner_keeps_elide_as_overflow_fallback() {
    let body = toast_banner_body();
    assert!(
        body.contains("overflow: elide;"),
        "超出两行后仍需 elide 兜底，避免文字溢出气泡"
    );
}
