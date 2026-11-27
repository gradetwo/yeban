//! T-FIX-816：WAV 导出的**失败**必须把实测行改成失败态。
//!
//! 问题：失败回执已经写进 `module-status`，但 `export-wav-measurement` 只在成功时
//! 更新；空工程导出失败后它仍显示导出前的旧读数（或“尚未导出”），看起来像旧测量
//! 或什么事都没发生。
//!
//! 本判据走生产窗口的**真回调**：空工程 ⇒ `invoke_export_wav` ⇒ 失败回执必须为
//! `outcome=error`，且实测行必须以“未测 / 导出失败”开头并带同一条错误原文。
//! 红对照：把失败分支的实测行写回去掉 ⇒ 本判据红（实测行仍是旧的成功读数）。
//!
//! 平台约束：`slint::platform::set_platform` 是线程局部的 ⇒ 本文件只放一个 `#[test]`。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::host::{ModulePaths, wire_module_actions_with};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::inspect::install_testing_backend_with_event_loop;

fn drain_event_loop() {
    slint::quit_event_loop().expect("带事件循环的后端必须提供 proxy");
    slint::run_event_loop().expect("无头事件循环必须正常返回");
}

fn receipt_message(receipt: &str) -> String {
    let marker = "message=\"";
    let Some(start) = receipt.find(marker).map(|i| i + marker.len()) else {
        return String::new();
    };
    let tail = &receipt[start..];
    let end = tail.rfind('"').unwrap_or(tail.len());
    tail[..end].to_owned()
}

#[test]
fn empty_project_export_failure_updates_the_measurement_row() {
    install_testing_backend_with_event_loop();

    // 合法空工程：无音轨、无片段、无主总线实体 ⇒ 导出必须失败，不能产出空白 WAV。
    let project = YebanProjectV1::default();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("t-fix-816", "yeban-app", project.clone(), 1).expect("打开撤销会话"),
    ));
    let view = ViewState::from_project(&project).expect("空工程也必须能投影成空视图");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 0).expect("生产窗口");
    let mut paths = ModulePaths::from_env();
    paths.export_wav = std::env::temp_dir().join(format!(
        "yeban-t816-{}-{}.wav",
        std::process::id(),
        yeban_model::EntityId::new()
    ));
    let _midi = wire_module_actions_with(
        &ui,
        Some(Rc::clone(&port)),
        paths,
        Rc::new(yeban_app::file_picker::NoPicker),
    );

    // 打开弹窗：实测行先被重置成“尚未导出”，再模拟上一次成功导出的旧读数。
    ui.invoke_open_export_wav();
    ui.set_export_wav_measurement(
        "实测 · 真峰值 -3.0 dBFS · 积分响度 -14.0 LUFS · LRA 1.0 LU".into(),
    );

    ui.invoke_export_wav();
    assert!(
        host::wait_for_pending_export(),
        "空工程导出也必须留下一个可 join 的工作线程"
    );
    drain_event_loop();

    let receipt = ui.get_module_status().to_string();
    assert!(
        receipt.starts_with("module:action=export-wav:outcome=error:"),
        "空工程导出必须失败并写失败回执；实际={receipt}"
    );
    assert!(
        !ui.get_module_status_succeeded(),
        "失败回执的成功位必须为 false"
    );

    let message = receipt_message(&receipt);
    assert!(
        !message.is_empty(),
        "失败回执必须带 message 原文；实际={receipt}"
    );
    let measurement = ui.get_export_wav_measurement().to_string();
    assert!(
        measurement.starts_with("未测 · 导出失败"),
        "失败后实测行必须显示失败态，而不是旧读数；实际={measurement}"
    );
    assert!(
        measurement.contains(&message),
        "实测行必须与回执同一条失败原因；measurement={measurement} message={message}"
    );
    assert!(
        !measurement.contains("真峰值"),
        "失败后不得保留旧的成功测量；实际={measurement}"
    );
}
