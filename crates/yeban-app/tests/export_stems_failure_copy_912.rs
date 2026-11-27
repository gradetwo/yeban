//! T-FIX-912：分轨导出两处失败的可执行文案与错误分级（运行时 UI 读数）。
//!
//! 只读 `MainWindow` 的运行时属性，不 grep 源码字符串：
//! - 提示条与 `module-status-text` 必须是完整人话＋下一步；
//! - `module-status-succeeded` / `toast-is-error` 必须是错误档；
//! - 技术原文只许在结构化 `module-status` 的 `detail=` 里，且不在主提示条。

use yeban_app::bridge::ViewState;
use yeban_app::cli::Language;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

fn build_ui() -> LivePort<yeban_app::ui::MainWindow> {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("LivePort 必须装配 MainWindow")
}

#[test]
fn stems_failure_copy_covers_all_nine_languages() {
    assert_eq!(Language::ALL.len(), 9, "源语言＋八个目标语言");
    for lang in Language::ALL {
        let no_project = host::stems_no_project_message(lang);
        let render_failed = host::stems_render_failure_message(lang);
        assert!(!no_project.trim().is_empty(), "{lang:?} 缺无工程文案");
        assert!(!render_failed.trim().is_empty(), "{lang:?} 缺渲染失败文案");
    }
    assert!(host::stems_no_project_message(Language::ZhCn).contains("打开"));
    assert!(host::stems_render_failure_message(Language::ZhCn).contains("重试"));
}

#[test]
fn no_project_and_render_failure_land_as_actionable_errors_with_detail_split() {
    let port = build_ui();
    let ui = port.ui();

    // 先造一个“成功/信息”旧态，证明失败写点会重置分级，不沿用上一条。
    ui.set_module_status_succeeded(true);
    ui.set_toast_is_error(false);
    ui.set_toast_message("上一步完成".into());

    // ---- 臂 1：没有活跃工程 ----
    let no_project = host::stems_no_project_message(Language::ZhCn);
    host::report_module_failure(
        ui,
        "export-stems",
        host::NO_ACTIVE_PROJECT_CODE,
        no_project,
        None,
    );
    assert_eq!(ui.get_module_status_text().as_str(), no_project);
    assert_eq!(ui.get_toast_message().as_str(), no_project);
    assert!(!ui.get_module_status_succeeded(), "错误不得标成成功");
    assert!(ui.get_toast_is_error(), "错误提示条必须是错误档");
    assert!(ui.get_show_floating_toast(), "错误必须在浮位可见");
    let receipt = ui.get_module_status().to_string();
    assert!(
        receipt.contains("action=export-stems"),
        "回执点名动作: {receipt}"
    );
    assert!(receipt.contains("outcome=error"), "回执点名失败: {receipt}");
    assert!(
        receipt.contains("no-active-project"),
        "无工程必须有稳定错误码: {receipt}"
    );
    assert!(
        no_project.contains("打开"),
        "必须提示先打开工程: {no_project}"
    );
    assert!(
        no_project.contains("新建"),
        "必须提示可新建工程: {no_project}"
    );
    assert!(!ui.get_toast_message().contains("no-active-project"));

    // ---- 臂 2：分轨渲染失败，技术原文在结构化回执与日志，不在主提示条 ----
    let render_failed = host::stems_render_failure_message(Language::ZhCn);
    let raw = "render: No such file or directory (os error 2) at /tmp/private/stem.wav";
    host::report_module_failure(
        ui,
        "export-stems",
        "stems-render-failed",
        render_failed,
        Some(raw),
    );
    let toast = ui.get_toast_message().to_string();
    let text = ui.get_module_status_text().to_string();
    let receipt = ui.get_module_status().to_string();
    assert_eq!(toast, render_failed, "主提示条必须只有人话＋下一步");
    assert_eq!(text, render_failed, "人话回执行与提示条同一句");
    assert!(!ui.get_module_status_succeeded());
    assert!(ui.get_toast_is_error());
    assert!(ui.get_show_floating_toast());
    assert!(
        render_failed.contains("没有完成") && render_failed.contains("重试"),
        "必须说清没完成并给重试: {render_failed}"
    );
    assert!(
        render_failed.contains("没有被改动"),
        "必须说清工程/原文件未受影响: {render_failed}"
    );
    assert!(
        render_failed.contains("WAV"),
        "换格式检查必须可达: {render_failed}"
    );
    assert!(
        render_failed.contains("输出目录"),
        "目录可写检查必须可达: {render_failed}"
    );
    assert!(
        !toast.contains("No such file"),
        "技术原文不得进主提示条: {toast}"
    );
    assert!(
        !text.contains("os error"),
        "人话回执行不得进技术原文: {text}"
    );
    assert!(
        !toast.contains("stems-render-failed"),
        "错误码不得进主提示条"
    );
    assert!(receipt.contains("error=stems-render-failed"));
    assert!(
        receipt.contains("detail=") && receipt.contains("No such file"),
        "技术原文必须仍可在结构化回执里查: {receipt}"
    );
}
