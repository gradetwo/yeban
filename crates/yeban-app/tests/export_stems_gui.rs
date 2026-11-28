//! **分轨导出的 GUI 入口不再是 stub**（ADR-0076 B5）。
//!
//! 修前：`app.slint` 写「导出分轨 (Stems, 请用 CLI)」，`dispatch_aether_command` 里
//! 只 `invoke_open_export_wav()` + toast「CLI 可用 --export-stems」⇒ GUI 没有分轨导出。
//!
//! 修法：`on_command_execute` 特判 `export-stems`（与 `place-audio-clip` 同款）：
//! 输出目录 = WAV 导出路径同级 `stems/` → `create_dir_all` → 工作线程调
//! `crate::export_stems::export_stems_with_progress`（与 CLI / MCP **同一份**后端，
//! 带进度回调版；无回调的 `export_stems` 只是它的薄 wrapper）→ 回 UI 写回执。
//! 前置已核：`in-process-mcp` 默认开、`rfd` 已是依赖、`ModulePaths: Clone`。
//!
//! ⚠ 纯 `include_str!` 判据（本机编不了 Slint）；行为由 CI 的 yeban-app 腿真跑。

const HOST: &str = include_str!("../src/host.rs");
const APP: &str = include_str!("../ui/app.slint");

#[test]
fn export_stems_gui_calls_the_real_backend() {
    assert!(
        HOST.contains(r#""export-stems" =>"#),
        "on_command_execute 必须有 export-stems 特判"
    );
    assert!(
        HOST.contains("crate::export_stems::export_stems_with_progress("),
        "GUI 必须调与 CLI/MCP 同一份 export_stems 后端（带进度回调版）"
    );
    assert!(
        HOST.contains("std::thread::spawn"),
        "渲染必须在工作线程（否则 UI 冻结，同 P1-A）"
    );
    assert!(
        HOST.contains("upgrade_in_event_loop"),
        "结果必须回 UI 线程写回执"
    );
}

#[test]
fn export_stems_menu_no_longer_says_use_cli() {
    assert!(
        !APP.contains("导出分轨 (Stems, 请用 CLI)"),
        "菜单不得再让用户「请用 CLI」"
    );
    assert!(APP.contains("导出分轨 (Stems)..."), "菜单应有真实入口");
}
