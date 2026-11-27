//! T-FIX-837：停止取消未生效的场景启动；正常到点生效时回执改为“已生效”。
//!
//! 红对照（旧实现）：`pending_launch` 在 `Stopped` 下仍按量子递减 ⇒ 点场景后按停止、
//! 再推 800 个量子，`scene_launches_applied` 变成 1、BPM 被场景 tempo 改写。
//! 本判据要求取消后两者都不动，回执明确写“已取消：停止”。

use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::{EditMark, EngineHost};
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_model::LaunchQuantization;
use yeban_ui_test_port::inspect::install_testing_backend;

const QUANTA_FOR_TWO_BARS: usize = 1_500;
const QUANTA_AFTER_STOP: usize = 800;
const SCENE_TEMPO: f64 = 91.0;

struct HostFixture {
    ui: yeban_app::ui::MainWindow,
    engine: Rc<RefCell<EngineHost>>,
}

fn project_with_scene_tempo(
    quantization: LaunchQuantization,
) -> yeban_model::project::YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    project.transport.launch_quantization = quantization;
    project
}

fn host_fixture(quantization: LaunchQuantization) -> HostFixture {
    install_testing_backend();
    let project = project_with_scene_tempo(quantization);
    let mut view = ViewState::from_project(&project).expect("filled_project 必须能投影");
    // 场景 tempo 与工程 BPM 不同 ⇒ “应用了场景”与“BPM 没动”可以逐位区分。
    view.scenes[0].tempo = Some(SCENE_TEMPO);
    let scene = DemoScene::from_view(&view);
    let mut engine = EngineHost::new();
    engine.reload(&project, 0).expect("引擎必须能建起来");
    // 与生产装配一致：reload 之后由控制面显式停住。
    engine.stop();
    let engine = Rc::new(RefCell::new(engine));
    let view = Rc::new(RefCell::new(view));
    let ui = host::build_main_window(&view.borrow(), &scene).expect("生产窗口");
    host::wire_transport(&ui, Rc::clone(&engine), None, None);
    host::wire_scene_launch(&ui, Rc::clone(&engine), Rc::clone(&view));
    HostFixture { ui, engine }
}

impl HostFixture {
    fn applied(&self) -> u64 {
        self.engine
            .borrow_mut()
            .runtime_for_criteria()
            .expect("控制驱动档：运行时归宿主所有")
            .stats()
            .scene_launches_applied
    }

    fn bpm(&self) -> f64 {
        self.engine
            .borrow_mut()
            .runtime_for_criteria()
            .expect("控制驱动档：运行时归宿主所有")
            .transport()
            .bpm()
    }

    fn has_pending(&self) -> bool {
        self.engine.borrow().has_pending_scene_launch() == Some(true)
    }

    fn pump(&self, quanta: usize) {
        let mut block = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * 2];
        let mut engine = self.engine.borrow_mut();
        let runtime = engine
            .runtime_for_criteria()
            .expect("控制驱动档：运行时归宿主所有");
        for _ in 0..quanta {
            runtime.process_quantum(&mut block, 2);
        }
    }
}

#[test]
fn stop_cancels_pending_scene_launch_without_applying_or_retuning() {
    let fx = host_fixture(LaunchQuantization::Off);
    let bpm_before = fx.bpm();
    assert!(!fx.has_pending(), "夹具从没有 pending 开始");

    fx.ui.invoke_scene_launch(0);
    assert!(fx.has_pending(), "点场景后必须真的排上 pending");
    assert_eq!(fx.applied(), 0, "量化点之前不许生效");
    let status = fx.ui.get_scene_launch_status();
    assert!(
        status.contains("已开始"),
        "停住态点场景回执应为已开始，实际={status}"
    );

    // 取消之前一个量子都不推：此时旧实现也还没机会应用。
    fx.ui.invoke_stop();
    assert!(
        !fx.has_pending(),
        "停止返回时 pending 必须已经被清掉，实际仍挂着"
    );

    // 取消后推 800 个量子：旧实现会在这里把 pending 走到生效并改写 BPM。
    // ⚠ 先断引擎读数，再断回执：这样红对照首先打在“延迟应用”这个产品缺陷上。
    fx.pump(QUANTA_AFTER_STOP);
    assert_eq!(fx.applied(), 0, "停止必须取消 pending，不能延迟应用");
    assert_eq!(fx.bpm(), bpm_before, "未生效的场景不许改写 BPM");
    assert!(!fx.has_pending(), "取消后 pending 必须清空");
    let status = fx.ui.get_scene_launch_status();
    assert!(
        status.contains("已取消：停止"),
        "按停止后回执必须明确写取消，实际={status}"
    );
    assert_eq!(
        fx.ui.get_current_scene_index(),
        -1,
        "启动被取消 ⇒ 当前场景高亮必须退回"
    );
}

struct LoopFixture {
    production: ProductionLoop,
    ui: yeban_app::ui::MainWindow,
    view: Rc<RefCell<ViewState>>,
    engine: Rc<RefCell<EngineHost>>,
}

fn loop_fixture(quantization: LaunchQuantization) -> LoopFixture {
    install_testing_backend();
    let project = project_with_scene_tempo(quantization);
    let mut view = ViewState::from_project(&project).expect("filled_project 必须能投影");
    view.scenes[0].tempo = Some(SCENE_TEMPO);
    let scene = DemoScene::from_view(&view);
    let production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    engine.borrow_mut().stop();
    let view = Rc::new(RefCell::new(view));
    let ui = host::build_main_window(&view.borrow(), &scene).expect("生产窗口");
    host::wire_scene_launch(&ui, Rc::clone(&engine), Rc::clone(&view));
    LoopFixture {
        production,
        ui,
        view,
        engine,
    }
}

impl LoopFixture {
    fn tick(&mut self) {
        let view = self.view.borrow();
        let _ = self
            .production
            .tick(&self.ui, &view, EditMark::new(None, 0), None);
    }

    fn applied(&self) -> u64 {
        self.engine
            .borrow_mut()
            .runtime_for_criteria()
            .expect("控制驱动档：运行时归宿主所有")
            .stats()
            .scene_launches_applied
    }

    fn pump_until_applied(&self, target: u64) {
        let mut block = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * 2];
        let mut engine = self.engine.borrow_mut();
        let runtime = engine
            .runtime_for_criteria()
            .expect("控制驱动档：运行时归宿主所有");
        for _ in 0..QUANTA_FOR_TWO_BARS {
            if runtime.stats().scene_launches_applied >= target {
                break;
            }
            runtime.process_quantum(&mut block, 2);
        }
    }
}

#[test]
fn a_scene_that_reaches_its_quantised_point_updates_the_receipt_to_effective() {
    let mut fx = loop_fixture(LaunchQuantization::Bar);
    fx.ui.invoke_scene_launch(0);
    let status = fx.ui.get_scene_launch_status();
    assert!(
        status.contains("已开始"),
        "停住态点场景回执应为已开始，实际={status}"
    );

    fx.pump_until_applied(1);
    assert_eq!(fx.applied(), 1, "场景必须恰好生效一次");
    assert!(
        fx.engine.borrow().transport().state.is_running(),
        "到点必须起播"
    );

    // “已生效”走生产心跳的同一个读口：真实计数前进才改回执。
    fx.tick();
    let status = fx.ui.get_scene_launch_status();
    assert!(
        status.contains("已生效"),
        "正常到点生效后回执必须更新为已生效，实际={status}"
    );
}
