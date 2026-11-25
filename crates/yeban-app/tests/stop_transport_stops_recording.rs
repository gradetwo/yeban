//! ⭐ T-FIX-348：停止按钮 / 空格 / 录音键**三条入口**都必须结束正在进行的 take。
//!
//! 修前 `on_stop` 与 `on_toggle_play` 只动 `EngineHost`，录音器仍在后台采集；界面走带
//! 已停、`recording` 仍为真，WAV 继续变长。本判据走**生产同一份** `wire_transport_*`
//! 装配，用脚本化源（⛔ 不碰声卡）逐条入口断言：
//! ① `Recorder::is_recording() == false`；② `ui.get_recording() == false`；
//! ③ 停后 WAV 字节不再增长；④ 新 take 已作为一条摆放落地进工程。

#![allow(unused)]
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::record_device::InputSourceFactory;
use yeban_app::record_wav::{InputSource, RecordOptions, RecordPlan};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, YebanProjectV1};

const NOW: u64 = 1_760_000_000_000;
const WINDOW: yeban_ui_test_port::Size = yeban_ui_test_port::Size::new(1440, 900);
const PLAYHEAD_TICK: u64 = 960;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-348-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn options() -> RecordOptions {
    RecordOptions {
        seconds: 1.0,
        sample_rate: 48_000,
        channels: 2,
        capacity_frames: 8192,
        drain_batch: 512,
        ..RecordOptions::default()
    }
}

fn fixture_project() -> YebanProjectV1 {
    let mut project = filled_project();
    project.transport.count_in_bars = 0;
    for track in project.tracks.values_mut() {
        track.clips.clear();
    }
    project
}

struct ToneSource {
    channels: u16,
    frame: u64,
}

impl InputSource for ToneSource {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let channels = usize::from(self.channels.max(1));
        for index in 0..frames {
            let value = ((self.frame + index as u64) % 32) as f32 / 64.0;
            for channel in 0..channels {
                out[index * channels + channel] = value;
            }
        }
        self.frame += frames as u64;
        frames
    }

    fn label(&self) -> &'static str {
        "t-fix-348-tone-source"
    }
}

struct ToneFactory;

impl InputSourceFactory for ToneFactory {
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        Ok(Box::new(ToneSource {
            channels: options.channels.max(1),
            frame: 0,
        }))
    }

    fn label(&self) -> &'static str {
        "t-fix-348-tone-factory"
    }
}

struct Rig {
    _tier1: yeban_ui_test_port::render::Tier1Window,
    ui: yeban_app::ui::MainWindow,
    engine: Rc<RefCell<EngineHost>>,
    port: Rc<UndoPort>,
    recorder: Rc<RefCell<yeban_app::record_wav::Recorder>>,
    path: PathBuf,
    _production: host::ProductionLoop,
    _scratch: Scratch,
}

fn assemble(tag: &str) -> Rig {
    let tier1 =
        yeban_ui_test_port::render::Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let project = fixture_project();
    let view = ViewState::from_project(&project).expect("装配时投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口构造入口");
    tier1
        .resize(WINDOW)
        .expect("窗口尺寸定下来之后再注入一次布局");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("stop-transport", "yeban-app", project.clone(), NOW)
            .expect("打开撤销会话"),
    ));
    let production = host::ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let _ = engine.borrow_mut().stop();
    let scratch = Scratch::new(tag);
    let path = scratch.dir.join("take.wav");
    let recorder = host::wire_transport_with_record(
        &ui,
        Rc::clone(&engine),
        Some(Rc::new(fixture_project())),
        Some(Rc::clone(&port)),
        host::RecordWiring {
            plan: RecordPlan {
                path: path.clone(),
                options: options(),
            },
            factory: Rc::new(ToneFactory),
            outcome: Rc::new(host::UiRecordOutcome),
            playhead_tick: Rc::new(|| PLAYHEAD_TICK),
        },
    );
    Rig {
        _tier1: tier1,
        ui,
        engine,
        port,
        recorder,
        path,
        _production: production,
        _scratch: scratch,
    }
}

impl Rig {
    fn placement_count(&self) -> usize {
        self.port
            .project()
            .tracks
            .values()
            .map(|track| track.clips.len())
            .sum()
    }

    fn start_take(&self) {
        let before = self.placement_count();
        self.ui.invoke_toggle_record();
        assert!(self.ui.get_recording(), "起录必须成功（工厂能开）");
        assert!(self.recorder.borrow().is_recording(), "录音器必须真的在录");
        std::thread::sleep(Duration::from_millis(90));
        assert_eq!(self.placement_count(), before, "未停录前不许落地");
    }

    fn assert_stopped(&self, before_placements: usize, label: &str) {
        assert!(
            !self.recorder.borrow().is_recording(),
            "{label}: Recorder 必须停止采集"
        );
        assert!(!self.ui.get_recording(), "{label}: 界面 recording 必须归零");
        assert_eq!(
            self.placement_count(),
            before_placements + 1,
            "{label}: 新 take 必须作为一条摆放落地进工程"
        );
        let bytes = std::fs::metadata(&self.path)
            .unwrap_or_else(|error| panic!("{label}: 读不到 WAV: {error}"))
            .len();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            std::fs::metadata(&self.path).expect("再读 WAV").len(),
            bytes,
            "{label}: 停后 WAV 字节不许继续增长"
        );
    }
}

#[test]
fn stop_button_space_and_record_key_all_end_the_current_take() {
    let rig = assemble("all-entries");

    // ① 停止按钮：起录（走带停着）⇒ 停止。
    rig.start_take();
    let before = rig.placement_count();
    rig.ui.invoke_stop();
    assert!(
        !rig.engine.borrow().transport().state.is_running(),
        "停止按钮仍必须停走带"
    );
    rig.assert_stopped(before, "停止按钮");

    // ② 空格：先播放，再起录 ⇒ 空格停走带时也必须结束 take。
    rig.ui.invoke_toggle_play();
    assert!(
        rig.engine.borrow().transport().state.is_running(),
        "前置：空格必须先把走带播起来"
    );
    rig.start_take();
    let before = rig.placement_count();
    rig.ui.invoke_toggle_play();
    assert!(
        !rig.engine.borrow().transport().state.is_running(),
        "空格仍必须停走带"
    );
    rig.assert_stopped(before, "空格");

    // ③ 录音键：原入口不能被新接线破坏。
    rig.start_take();
    let before = rig.placement_count();
    rig.ui.invoke_toggle_record();
    rig.assert_stopped(before, "录音键");
}
