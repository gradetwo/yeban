//! **T-FIX-434 / ISS-MWVWXV9Y-15FA68** 回归判据：打开音源库不得在 UI 线程同步扫全库。
//!
//! 用注入的慢扫描接缝把工作线程按住，证明：
//! 1. `on_open_sound_library` 立刻返回，期间事件循环仍能跑别的排队调用；
//! 2. 扫描完成后才发布 `sound-installed-*`；
//! 3. 扫描在飞时重开只登记 pending，旧结果作废，最终只接受新 generation；
//! 4. 整体扫描失败走红色状态，不伪装成空库。
//!
//! ⚠ 单 `#[test]`：Slint 平台每线程只能装一次。

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::Model;
use yeban_app::bridge::ViewState;
use yeban_app::host::{self, SoundInstalledScanHook, SoundInstalledSnapshot};
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::inspect::install_testing_backend_with_event_loop;

fn snapshot(name: &str) -> SoundInstalledSnapshot {
    SoundInstalledSnapshot {
        ids: vec![format!("user:{name}")],
        names: vec![name.to_owned()],
        locations: vec![format!("/tmp/{name}")],
        paths: vec![PathBuf::from(format!("/tmp/{name}"))],
        sizes: vec!["1.0 KiB".into()],
        deletable: vec![true],
        doctor_needed: vec![false],
        corrupt_names: Vec::new(),
        quarantine_failed: Vec::new(),
    }
}

fn drain_event_loop() {
    slint::quit_event_loop().expect("无头平台必须提供事件循环 proxy（否则本判据不成立）");
    slint::run_event_loop().expect("无头事件循环必须正常返回");
}

#[test]
fn opening_the_sound_library_keeps_the_ui_thread_off_the_disk_scan() {
    install_testing_backend_with_event_loop();
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口");

    // 慢扫描接缝：第 1 次调用按住等测试放行（旧 generation）；
    // 第 2 次立刻返回新快照；第 3 次返回整体失败。
    let calls = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx): (Sender<usize>, Receiver<usize>) = std::sync::mpsc::channel();
    let (release_tx, release_rx): (Sender<()>, Receiver<()>) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let started_tx = Arc::new(Mutex::new(started_tx));
    let scanner: SoundInstalledScanHook = {
        let calls = Arc::clone(&calls);
        let started_tx = Arc::clone(&started_tx);
        let release_rx = Arc::clone(&release_rx);
        Arc::new(move |_roots: &yeban_app::sound_library::SoundRoots| {
            let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
            started_tx
                .lock()
                .expect("started 通道")
                .send(call)
                .expect("测试仍持有 started 接收端");
            match call {
                1 => {
                    release_rx
                        .lock()
                        .expect("release 通道")
                        .recv_timeout(Duration::from_secs(10))
                        .expect("测试必须放行第一条慢扫描");
                    Ok(snapshot("stale"))
                }
                2 => Ok(snapshot("fresh")),
                _ => Err("模拟音源目录不可读".to_owned()),
            }
        })
    };
    host::wire_sound_library_dialog_with_installed_scan(&ui, None, scanner);

    // ① 打开回调必须在扫描完成前返回。
    let started_at = std::time::Instant::now();
    ui.invoke_open_sound_library();
    assert!(
        started_at.elapsed() < Duration::from_millis(150),
        "打开回调不得等待扫描（实测 {:?}）",
        started_at.elapsed()
    );
    assert!(ui.get_sound_library_open(), "面板必须已经打开");
    assert_eq!(
        ui.get_sound_library_status().as_str(),
        "正在扫描音源库…",
        "没有快照时必须明说正在扫描"
    );
    assert_eq!(ui.get_sound_installed_names().iter().count(), 0);

    // ② 扫描没完成时，事件循环仍能跑另一条排队调用。
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(5)), Ok(1));
    let (marker_tx, marker_rx) = std::sync::mpsc::channel();
    slint::invoke_from_event_loop(move || {
        marker_tx.send(()).expect("测试仍持有 marker 接收端");
    })
    .expect("无头平台必须接受 invoke_from_event_loop");
    drain_event_loop();
    assert!(
        marker_rx.recv_timeout(Duration::from_secs(2)).is_ok(),
        "扫描期间事件循环必须继续推进心跳"
    );

    // ③ 扫描在飞时关掉再打开：只登记 pending，⛔ 不起第二条并发扫描。
    ui.invoke_close_sound_library();
    ui.invoke_open_sound_library();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "在飞期间重开不得并发起第二条扫描"
    );

    // 放行旧 generation：它的结果必须被丢弃，随后自动补扫新 generation。
    release_tx.send(()).expect("放行第一条扫描");
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(5)), Ok(2));
    let fresh = wait_for_installed_name(&ui, "fresh");
    assert_eq!(
        fresh,
        vec!["fresh".to_owned()],
        "旧 generation 的 stale 结果不得覆盖界面"
    );
    assert_eq!(ui.get_sound_library_status().as_str(), "");

    // ④ 整体失败必须红字，⛔ 不伪装成空库。
    ui.invoke_close_sound_library();
    ui.invoke_open_sound_library();
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(5)), Ok(3));
    wait_for_scan_error(&ui);
    assert!(
        ui.get_sound_library_status().as_str().contains("扫描失败"),
        "整体失败必须进入状态行（现读：{}）",
        ui.get_sound_library_status()
    );
    assert!(
        ui.get_sound_library_status_is_error(),
        "整体失败必须是红色状态"
    );
}

/// 排空事件循环直到界面里出现指定库名（最多 3 秒；异步工作线程用）。
fn wait_for_installed_name(ui: &yeban_app::ui::MainWindow, want: &str) -> Vec<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        drain_event_loop();
        let names: Vec<String> = ui
            .get_sound_installed_names()
            .iter()
            .map(|name| name.to_string())
            .collect();
        if names.iter().any(|name| name == want) {
            return names;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "等待 {want} 发布超时（现读 {names:?}）"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 排空事件循环直到状态行变成失败（最多 3 秒）。
fn wait_for_scan_error(ui: &yeban_app::ui::MainWindow) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        drain_event_loop();
        if ui.get_sound_library_status_is_error()
            && ui.get_sound_library_status().as_str().contains("扫描失败")
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "等待扫描失败状态超时（现读 {}）",
            ui.get_sound_library_status()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
