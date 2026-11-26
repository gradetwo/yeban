//! T-FIX-687：鼓机**选风格即生成预听 loop**。
//!
//! 判据在真实 Tier-1 活窗口里点 `drum-gen-style-1`（真指针按下/释放），断言：
//! 风格与摇摆立即更新、弹窗不关、现有生成通道恰好新增一条已摆放的 MIDI 片段，
//! 其音符数等于 theory 用该风格 + 当前四参数生成的结果（证"真的生成了，不是只改下标"）。
//! 另有一条响亮失败：越界风格下标给 error，不生成。

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::EntityId;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_700_000_000_000;

struct Fixture {
    port: LivePort<MainWindow>,
    undo: Rc<UndoPort>,
}

impl Fixture {
    fn new() -> Self {
        let project = filled_project();
        let undo = Rc::new(UndoPort::new(
            UndoSession::open("<drum-style-preview>", "yeban-app", project.clone(), NOW_MS)
                .expect("撤销会话必须能打开"),
        ));
        let view = ViewState::from_project(&project).expect("filled_project 必须能投影");
        let scene = DemoScene::from_view(&view);
        let view = Rc::new(RefCell::new(view));
        let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window(&view.borrow(), &scene)
        })
        .expect("Tier-1 平台 + 生产窗口构造入口");
        host::wire_drum_gen_dialog(port.ui(), Some(Rc::clone(&undo)));
        port.ui().invoke_open_drum_gen();
        assert!(port.ui().get_drum_gen_open(), "打开回调必须真的开窗");
        Self { port, undo }
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    fn click_style(&mut self, index: i32) {
        self.port.refresh_tree(None).expect("点击前重抓几何");
        let id = format!("drum-gen-style-{index}");
        let bounds = self
            .port
            .tree()
            .find_by_id(&id)
            .unwrap_or_else(|| panic!("控件树里必须有 {id}"))
            .bounds
            .expect("风格行必须有几何");
        assert!(bounds.width > 0 && bounds.height > 0, "{id} 几何为零");
        self.port
            .dispatch_pointer_down(&id, 4.0, 4.0, PointerButton::Left)
            .expect("风格行必须可按下");
        self.port
            .dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
    }
}

fn clip_ids(project: &yeban_model::YebanProjectV1) -> BTreeSet<EntityId> {
    project.clip_pool.keys().copied().collect()
}

/// 点第二行风格 ⇒ 立即生成一条已摆放、可走带预听的 loop；弹窗留在原地。
#[test]
fn picking_a_style_generates_one_placed_pattern_immediately() {
    let mut fx = Fixture::new();
    let ui = fx.ui().clone_strong();
    let before = clip_ids(&fx.undo.project());
    let styles = yeban_theory::factory_drum_styles().expect("出厂预置");
    let expected_swing = styles[1].swing_permille;

    fx.click_style(1);
    assert_eq!(ui.get_drum_gen_style_index(), 1, "风格下标必须立刻更新");
    assert_eq!(
        ui.get_drum_gen_swing(),
        expected_swing.to_string(),
        "摇摆必须先播种为该风格定值，再按它生成"
    );
    assert!(
        ui.get_drum_gen_open(),
        "选风格生成的是预听 loop，弹窗必须留着继续挑"
    );
    assert_eq!(ui.get_drum_gen_error(), "", "成功不得留错误");

    let after = fx.undo.project();
    let new_clips: Vec<&yeban_model::ClipPoolEntry> = after
        .clip_pool
        .values()
        .filter(|entry| !before.contains(&entry.id))
        .collect();
    assert_eq!(new_clips.len(), 1, "一次选风格必须恰好生成一条片段");
    let new_id = new_clips[0].id;
    let notes = new_clips[0].content.notes().expect("鼓机片段必须是 MIDI");

    // 预听的前提是它在时间线上（`drum_gen_ops` 的 AddClipPlacement），不是只躺在池里。
    assert!(
        after.tracks.values().any(|track| track
            .clips
            .values()
            .any(|placement| placement.clip_id == new_id)),
        "新 loop 必须已摆放，才能被现有走带预听"
    );

    let expected = yeban_theory::generate_drum_pattern(
        &styles[1],
        &yeban_theory::DrumGenParams {
            complexity: 50,
            intensity: 80,
            swing_permille: Some(expected_swing),
            seed: after.rng_seed,
            bars: 4,
            fill: ui.get_drum_gen_fill(),
        },
    )
    .expect("该风格 + 当前参数必须能生成");
    assert_eq!(notes.len(), expected.len(), "必须用该风格真的生成 pattern");
}

/// 生成失败要响亮：越界风格下标写 error，且不新增半成品片段。
#[test]
fn an_out_of_range_style_refuses_loudly_without_generating() {
    let fx = Fixture::new();
    let ui = fx.ui().clone_strong();
    let before = clip_ids(&fx.undo.project());
    ui.invoke_drum_gen_style_picked(9_999);
    let error = ui.get_drum_gen_error();
    assert!(
        error.contains("越界") && error.contains("9999"),
        "越界风格必须点名失败，实际 {error:?}"
    );
    let after = fx.undo.project();
    assert_eq!(clip_ids(&after), before, "失败路径不得留下半成品片段");
}
