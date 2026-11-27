//! T-FIX-821：停住态点场景必须起播；已在播放时行为不变。
//!
//! 旧实现：`wire_scene_launch` 只排队，`PendingSceneLaunch` 到点只 retune＋计数，
//! 走带仍 Stopped ⇒ 用户看到“已排队”但播放头不动。
//!
//! 本判据走生产接线：`build_main_window` ＋ `wire_scene_launch` 的
//! `invoke_scene_launch`；控制驱动直接推运行时量子，观察真实 `TransportState`。

use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_model::LaunchQuantization;
use yeban_ui_test_port::inspect::install_testing_backend;

const QUANTA_FOR_TWO_BARS: usize = 1_500;

struct Fixture {
    ui: yeban_app::ui::MainWindow,
    engine: Rc<RefCell<EngineHost>>,
}

fn fixture() -> Fixture {
    install_testing_backend();
    let mut project = yeban_model::samples::filled_project();
    project.transport.launch_quantization = LaunchQuantization::Bar;
    let view = ViewState::from_project(&project).expect("filled_project 必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut engine = EngineHost::new();
    engine.reload(&project, 0).expect("引擎必须能建起来");
    // 与 GUI 生产路径一致：`reload` 之后由控制面显式停住（`main.rs` 同款）。
    engine.stop();
    let engine = Rc::new(RefCell::new(engine));
    let view = Rc::new(RefCell::new(view));
    let ui = host::build_main_window(&view.borrow(), &scene).expect("生产窗口");
    host::wire_scene_launch(&ui, Rc::clone(&engine), view);
    Fixture { ui, engine }
}

impl Fixture {
    fn is_running(&self) -> bool {
        self.engine.borrow().transport().state.is_running()
    }

    fn applied(&self) -> u64 {
        let mut engine = self.engine.borrow_mut();
        engine
            .runtime_for_criteria()
            .expect("控制驱动档：运行时归宿主所有")
            .stats()
            .scene_launches_applied
    }

    /// 推到场景真的生效（或预算耗尽），并顺带断走带状态。
    fn pump_until_applied(&self, target: u64) -> usize {
        let mut block = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * 2];
        let mut engine = self.engine.borrow_mut();
        let runtime = engine
            .runtime_for_criteria()
            .expect("控制驱动档：运行时归宿主所有");
        let mut steps = 0;
        while runtime.stats().scene_launches_applied < target && steps < QUANTA_FOR_TWO_BARS {
            runtime.process_quantum(&mut block, 2);
            steps += 1;
        }
        steps
    }
}

#[test]
fn stopped_scene_launch_starts_transport_at_the_quantised_point() {
    let fx = fixture();
    assert!(!fx.is_running(), "夹具必须从 Stopped 开始");

    fx.ui.invoke_scene_launch(0);

    // 回执如实：停住态点场景 ⇒ 已开始（量化点起播），不是旧的“已排队”。
    let status = fx.ui.get_scene_launch_status();
    assert!(
        status.contains("已开始"),
        "停住态点场景必须回执“已开始”，实际={status}"
    );
    assert!(
        !fx.is_running(),
        "量化点之前走带不该已经跑起来（起播在量化点发生）"
    );

    let steps = fx.pump_until_applied(1);
    assert!(steps > 0, "量化点必须真的被推到（预算内生效）");
    assert_eq!(
        fx.applied(),
        1,
        "场景必须恰好生效一次；实际 {}",
        fx.applied()
    );
    assert!(fx.is_running(), "量化点之后 transport.state 必须是 Running");

    // 已在播放 ⇒ 行为不变：只排队，不改变既有走带状态。
    fx.ui.invoke_scene_launch(0);
    let status = fx.ui.get_scene_launch_status();
    assert!(
        status.contains("已排队"),
        "已在播放时回执必须仍是“已排队”，实际={status}"
    );
    let steps = fx.pump_until_applied(2);
    assert!(steps > 0, "第二次量化点必须被推到");
    assert!(
        fx.is_running(),
        "已在播放时点场景不得停走带（量化点后仍 Running）"
    );
    assert_eq!(fx.applied(), 2, "第二次场景也必须恰好生效一次");
}
