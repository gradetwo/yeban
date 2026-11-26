//! **T-FIX-429 / ISS-MWVUALT9-BF166D** 回归判据：预设 / 等级循环不得在 UI 回调里同步跑完整曲。
//!
//! 用注入的慢分析接缝把工作线程按住，证明：
//! 1. `on_song_overview_profile_cycle` 立即返回，并且事件循环还能跑别的排队调用；
//! 2. 旧 generation 的结果不覆盖界面，最新一代才写面板；
//! 3. 完成后面板状态清空“分析中…”，预设档落到最新一次点击的值。
//!
//! ⚠ 单 `#[test]`：Slint 平台每线程只能装一次。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::song_overview::{self, SongOverviewAnalysisHook, SongOverviewAnalysisJob};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::inspect::install_testing_backend_with_event_loop;

fn drain_event_loop() {
    slint::quit_event_loop().expect("无头平台必须提供事件循环 proxy（否则本判据不成立）");
    slint::run_event_loop().expect("无头事件循环必须正常返回");
}

fn wait_until_idle(state: &Rc<RefCell<song_overview::SongOverviewLive>>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        drain_event_loop();
        let in_flight = {
            let core = Arc::clone(&state.borrow().core);
            let core = core
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            core.in_flight
        };
        if !in_flight {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "等待分析工作线程完成超时（in_flight 仍为 true）"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn profile_cycle_returns_before_render_and_latest_generation_wins() {
    install_testing_backend_with_event_loop();
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口");
    let live_view = Rc::new(RefCell::new(view));
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<song-overview-async>", "yeban-app", project, 0).expect("打开"),
    ));

    let calls = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx): (Sender<usize>, Receiver<usize>) = std::sync::mpsc::channel();
    let (release_tx, release_rx): (Sender<()>, Receiver<()>) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let started_tx = Arc::new(Mutex::new(started_tx));
    let hook: SongOverviewAnalysisHook = {
        let calls = Arc::clone(&calls);
        let started_tx = Arc::clone(&started_tx);
        let release_rx = Arc::clone(&release_rx);
        Arc::new(move |job: &SongOverviewAnalysisJob| {
            let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
            started_tx
                .lock()
                .expect("started 通道")
                .send(call)
                .expect("测试仍持有 started 接收端");
            release_rx
                .lock()
                .expect("release 通道")
                .recv_timeout(Duration::from_secs(10))
                .expect("测试必须放行这条慢分析");
            song_overview::compute_song_overview(job)
        })
    };
    let engine = Rc::new(RefCell::new(EngineHost::new()));
    let state = song_overview::wire_song_overview_with_analysis_hook(
        &ui,
        engine,
        Rc::clone(&live_view),
        Some(Rc::clone(&undo)),
        hook,
    );

    // ① 预设回调必须在后台分析完成前返回；期间事件循环仍能跑排队的别的调用。
    let started_at = Instant::now();
    ui.invoke_song_overview_profile_cycle();
    assert!(
        started_at.elapsed() < Duration::from_millis(150),
        "预设循环不得等待完整曲分析（实测 {:?}）",
        started_at.elapsed()
    );
    assert_eq!(
        ui.get_song_overview_analysis_status().as_str(),
        "分析中…",
        "发起后台任务时必须把状态写成分析中"
    );
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(5)), Ok(1));
    let (marker_tx, marker_rx) = std::sync::mpsc::channel();
    slint::invoke_from_event_loop(move || {
        marker_tx.send(()).expect("测试仍持有 marker 接收端");
    })
    .expect("无头平台必须接受 invoke_from_event_loop");
    drain_event_loop();
    assert!(
        marker_rx.recv_timeout(Duration::from_secs(2)).is_ok(),
        "后台分析期间事件循环必须继续推进心跳"
    );

    // ② 第一代仍在飞时再点一次：只增代号，旧代结果必须被丢弃。
    ui.invoke_song_overview_profile_cycle();
    let latest_generation = {
        let core = Arc::clone(&state.borrow().core);
        let core = core
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        core.generation
    };
    release_tx.send(()).expect("放行第一代分析");
    assert_eq!(
        started_rx.recv_timeout(Duration::from_secs(5)),
        Ok(2),
        "第一代结果作废后必须接着跑最新一代"
    );
    release_tx.send(()).expect("放行最新一代分析");
    wait_until_idle(&state);

    // ③ 最新一代的结果才落地；状态行清空；档位是第二次点击后的值。
    let core = Arc::clone(&state.borrow().core);
    let core = core
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(core.generation, latest_generation);
    assert!(!core.in_flight, "最新一代完成后必须落回 in_flight=false");
    assert_eq!(
        core.profile,
        song_overview::next_profile(song_overview::next_profile(
            yeban_dsp::song_overview::TensionProfile::Default
        )),
        "两次点击必须停在第二个档位"
    );
    assert!(core.data.total_bars > 0, "结果必须是非空总览");
    drop(core);
    assert_eq!(
        ui.get_song_overview_analysis_status().as_str(),
        "",
        "完成后必须清掉分析中状态"
    );
    assert!(
        !ui.get_song_overview_profile().is_empty(),
        "预设按钮显示名必须由结果写回"
    );
}
