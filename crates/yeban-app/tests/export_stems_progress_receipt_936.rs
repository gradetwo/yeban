//! T-FIX-936：分轨导出在途进度与终局回执 —— GUI 端到端臂。
//!
//! 单判据（事件循环后端每进程只能装一次 ⇒ 本文件只能有一个 `#[test]`；
//! 与 `gui_export_asset_pool.rs` 同款 `install_testing_backend_with_event_loop`）。
//!
//! 臂 A（建目录失败）：USERPROFILE 根下放一个文件挡住 `Documents/Yeban/stems`
//! —— 点菜单 ⇒ 同步落在常驻错误回执（reason ＋ 下一步），不 spawn 工作线程。
//! 臂 B（成功）：换干净的 USERPROFILE 根 —— 点菜单 ⇒ 同步读到起始在途文案；
//! 飞行中轮询＋排空事件循环，抓到一条带 `done/total` 的计数读数；工作线程
//! join 后再排空，终局读到常驻成功回执（`outcome=ok` ＋ `dir=`），提示条带目录。
//!
//! 不需要事件循环的判据（进度回调本身／在途写点直接写入／九语文案覆盖）在
//! `export_stems_progress_units_936.rs`。
//!
//! ⚠ 读数说明：臂 B 的在途计数读数必须在事件循环排空后才落到属性上 ——
//! 工作线程的进度回调经 `upgrade_in_event_loop` 投递，测试侧
//! `quit_event_loop` ＋ `run_event_loop` 逐轮排空才能观测到。起始写点是
//! spawn 前的同步写，不用排空。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::host::{ModulePaths, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::inspect::install_testing_backend_with_event_loop;

const NOW_MS: u64 = 1_700_000_000_000;

/// 把本测试进程的 profile 指到给定根（调用时现读，臂间可换根）。
fn point_profile_at(root: &PathBuf) {
    // SAFETY: 本文件单判据、臂间串行，没有别的 USERPROFILE 读者。
    unsafe { std::env::set_var("USERPROFILE", root) };
}

/// 排空一次事件循环：先挂 quit，再跑一轮，把工作线程已投递的闭包执行掉。
fn drain_event_loop() {
    slint::quit_event_loop().expect("带事件循环的后端必须提供 proxy");
    slint::run_event_loop().expect("无头事件循环必须正常返回");
}

fn receipt_field<'a>(receipt: &'a str, key: &str) -> Option<&'a str> {
    receipt
        .split(':')
        .find_map(|part| part.strip_prefix(&format!("{key}=")))
}

#[test]
fn stems_export_progress_and_terminal_receipt() {
    install_testing_backend_with_event_loop();
    let mut failures: Vec<String> = Vec::new();

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<936-repro>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let ui = host::build_main_window_with_console_tab(&view, &scene, 0).expect("生产窗口");
    let _ = host::wire_module_actions_with(
        &ui,
        Some(Rc::clone(&undo)),
        ModulePaths::default(),
        Rc::new(yeban_app::file_picker::NoPicker),
    );
    host::wire_aether_actions(
        &ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::clone(&view_cell),
    );
    assert!(
        undo.try_project().is_some(),
        "复现前提：撤销端口必须握着活跃工程"
    );
    ui.show().expect("活窗口必须能 show");

    // ── 臂 A：建目录失败（同步常驻错误回执） ──────────────────────────
    let blocked_root = std::env::temp_dir().join(format!("yeban-936-mkdir-{}", std::process::id()));
    let blocked_stems = blocked_root.join("Documents").join("Yeban").join("stems");
    std::fs::create_dir_all(blocked_stems.parent().expect("stems 有父目录")).expect("建挡路父目录");
    std::fs::write(&blocked_stems, b"blocking file").expect("放挡路文件");
    point_profile_at(&blocked_root);
    assert_eq!(
        host::stems_output_dir(),
        blocked_stems,
        "臂 A 前提：输出目录必须指到被挡住的那份"
    );
    ui.invoke_command_execute("export-stems".into());
    let receipt_a = ui.get_module_status().to_string();
    let text_a = ui.get_module_status_text().to_string();
    println!("[936-A] receipt={receipt_a} text={text_a}");
    if !ui.get_module_status_succeeded() {
        // 失败档本就是 false，上面不断言；只记一条读数。
    } else {
        failures.push(format!(
            "臂 A：建目录失败必须落在失败档，读到 succeeded=true：{receipt_a}"
        ));
    }
    if !receipt_a.contains("stems-create-output-dir-failed") {
        failures.push(format!("臂 A：回执必须点名建目录失败码：{receipt_a}"));
    }
    if !text_a.contains("无法创建") || !text_a.contains("没有开始") {
        failures.push(format!("臂 A：人话行必须说清没开始＋原因：{text_a}"));
    }
    if !text_a.contains("可写") && !text_a.contains("换一个位置") {
        failures.push(format!("臂 A：人话行必须给下一步：{text_a}"));
    }
    if !ui.get_toast_is_error() {
        failures.push(format!(
            "臂 A：建目录失败的提示条必须标错误档：is_error={}",
            ui.get_toast_is_error()
        ));
    }

    // ── 臂 B：成功（同步起始 → 飞行中计数 → 常驻成功回执） ──────────
    let ok_root = std::env::temp_dir().join(format!("yeban-936-ok-{}", std::process::id()));
    point_profile_at(&ok_root);
    let out_dir = host::stems_output_dir();
    assert!(!out_dir.exists(), "臂 B 前提：输出目录必须从干净状态起");
    let started = Instant::now();
    ui.invoke_command_execute("export-stems".into());
    // 起始写点是 spawn 前的同步写：工作线程还没回来就必须可见。
    let start_text = ui.get_module_status_text().to_string();
    let start_receipt = ui.get_module_status().to_string();
    println!("[936-B-start] text={start_text} receipt={start_receipt}");
    if start_text != "正在导出分轨…" {
        failures.push(format!(
            "臂 B：点菜单后必须同步读到起始在途文案，读到：{start_text}"
        ));
    }
    if !start_receipt.contains("outcome=in-progress") {
        failures.push(format!(
            "臂 B：点菜单后回执必须同步标 in-progress：{start_receipt}"
        ));
    }
    if ui.get_toast_message().as_str().contains("分轨完成")
        || ui.get_toast_message().as_str().contains("正在导出分轨")
    {
        failures.push(format!(
            "臂 B：起始写点⛔ 不得碰提示条（toast 4 秒就收走）：{}",
            ui.get_toast_message()
        ));
    }

    // 飞行中轮询＋排空：抓一条带 done/total 的计数读数。
    let mut counted: Option<(String, String, Duration)> = None;
    let mut terminal_early = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        drain_event_loop();
        let text = ui.get_module_status_text().to_string();
        let receipt = ui.get_module_status().to_string();
        if receipt.contains("outcome=ok") {
            terminal_early = true;
            println!("[936-B] 终局先于计数读数到达：{text} / {receipt}");
            break;
        }
        if receipt.contains("outcome=in-progress")
            && receipt_field(&receipt, "done").is_some_and(|d| d != "0")
            && text.contains('/')
        {
            counted = Some((text, receipt, started.elapsed()));
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    match &counted {
        Some((text, receipt, elapsed)) => {
            println!("[936-B-counted] after {elapsed:?} text={text} receipt={receipt}");
        }
        None => {
            if terminal_early {
                failures.push(
                    "臂 B：终局回执先于任何计数读数到达 —— 飞行中进度没被观测到（导出太快或排空没跟上）"
                        .to_string(),
                );
            } else {
                failures.push(
                    "臂 B：60 秒内既没抓到计数读数、终局也没到 —— 工作线程可能卡住".to_string(),
                );
            }
        }
    }

    // 工作线程 join 后再排空一次，终局回执落袋。
    if !host::wait_for_pending_export() {
        failures.push("臂 B：必须有一个刚结束的导出工作线程可 join".to_string());
    }
    drain_event_loop();
    let receipt_b = ui.get_module_status().to_string();
    let text_b = ui.get_module_status_text().to_string();
    let toast_b = ui.get_toast_message().to_string();
    println!(
        "[936-B-terminal] after {:?} receipt={receipt_b} text={text_b} toast={toast_b}",
        started.elapsed()
    );
    if !ui.get_module_status_succeeded() {
        failures.push(format!("臂 B：导出必须成功：{receipt_b}"));
    }
    if receipt_field(&receipt_b, "outcome") != Some("ok") {
        failures.push(format!("臂 B：终局回执必须标 outcome=ok：{receipt_b}"));
    }
    if receipt_field(&receipt_b, "dir").is_none_or(|d| d.is_empty()) {
        failures.push(format!("臂 B：终局回执必须带输出目录 dir=：{receipt_b}"));
    }
    if !out_dir.join("stems-manifest.json").exists() {
        failures.push(format!(
            "臂 B：分轨 manifest 必须落盘：{}",
            out_dir.display()
        ));
    }
    if !toast_b.contains(&out_dir.display().to_string()) {
        failures.push(format!("臂 B：成功提示条必须带输出目录：{toast_b}"));
    }

    let _ = std::fs::remove_dir_all(&blocked_root);
    let _ = std::fs::remove_dir_all(&ok_root);
    assert!(
        failures.is_empty(),
        "T-FIX-936 判据失败（{} 条）：\n{}",
        failures.len(),
        failures.join("\n")
    );
}
