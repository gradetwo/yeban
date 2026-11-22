//! **播放跟进（Catch）端到端判据** —— T-FIX-110 的接线面。
//!
//! # 本文件要证的句子
//!
//! 纯决策 [`follow_scroll`](yeban_app::host) 在 `host.rs` 的单元判据里已经钉住四种情形；
//! 本文件证的是**它真的被接上**：`.slint` 算的可见宽确实到了宿主手里，且
//! [`ProductionLoop::tick`](yeban_app::host::ProductionLoop::tick) 在真窗口上真的会翻页。
//!
//! # 判据
//!
//! | # | 断言 | 弄红它的改动 |
//! | :--- | :--- | :--- |
//! | ① | `arrangement-timeline-px` > 0 且 **小于** 窗口宽 | 回调不接线 / 直接把窗口宽当可见宽（后者会让播放头在包头底下消失 240px 才翻页） |
//! | ② | 播放中播放头越过右界 ⇒ 偏移**增大**，且播放头回到可见区内、落在左起 10% 处 | 删掉 `tick` 里的跟进调用 / 提前量改成 0（贴边） |
//! | ③ | 用户手动改偏移 ⇒ 之后再越界也**不动** | 去掉 `observe` 的脱开判定（抢视图） |
//! | ④ | 停住 ⇒ 不动；重新起播 ⇒ 立刻跟过去 | 去掉上升沿重挂（脱开一次就永久失效） |
//!
//! # 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 本文件只有一个 `#[test]`
//! （与 `tests/arrangement_ruler_seek.rs`、`tests/production_meter_leg.rs` 同款）。

#![allow(unused)]
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EditMark;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{Permission, Size, UiTestPort as _};

const WINDOW: Size = Size::new(1440, 900);

/// 生产形状的夹具：真引擎 + 真活窗口（与 `main.rs` 同一个建窗入口）。
struct Rig {
    production: ProductionLoop,
    view: ViewState,
    port: LivePort<MainWindow>,
}

impl Rig {
    fn new() -> Self {
        let project = demo_project();
        let production =
            ProductionLoop::start_with_audio_source(&project, 0, None).expect("产品装配建引擎");
        production_engine_stopped(&production);
        let view = ViewState::from_project(&project).expect("演示工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let mut port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");
        // 一次渲染 pass：`arrangement-timeline-px` 的绑定要真的被求值过。
        port_refresh(&mut port);
        Self {
            production,
            view,
            port,
        }
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    /// 可见宽（`.slint` 算好、经回调写上根属性的那一格）。
    fn timeline_px(&self) -> f32 {
        self.ui().get_arrangement_timeline_px()
    }

    fn scroll_x(&self) -> f32 {
        self.ui().get_roll_scroll_x()
    }

    fn playhead_px(&self) -> f32 {
        self.ui().get_playhead_px()
    }

    /// 一跳（`run_gui` 的 16ms 定时器体）。
    fn tick(&mut self) {
        // 字段级拆借（`MainWindow` 不是 `Clone`）：`tick` 要 `&mut production`，
        // 而窗口句柄活在同级的 `port` 里。
        let Self {
            production,
            view,
            port,
        } = self;
        let _ = production.tick(port.ui(), view, EditMark::new(None, 0), None);
    }

    fn play(&mut self) {
        self.production.engine_handle().borrow_mut().play();
    }

    fn stop(&mut self) {
        self.production.engine_handle().borrow_mut().stop();
    }

    fn seek(&mut self, ticks: u64) {
        self.production.engine_handle().borrow_mut().seek(ticks);
    }

    /// 模拟"用户自己把视图滚到别处"。
    ///
    /// 走的是**产品那条路**：滚轮 / 滚动条 / 滑块最终都落到
    /// `reproject_with_scroll(ui, port, next)` ⇒ `apply_view` ⇒ `set_roll_scroll_x`。
    /// 本文件没有 `UndoPort`（滚轮回调要求它），所以直接写同一个属性 —— 跟进判定看的是
    /// "偏移还是不是自己写的那一个"，与谁写的无关。
    fn user_scrolls_to(&mut self, px: f32) {
        self.ui().set_roll_scroll_x(px);
    }
}

/// 引擎新起一代默认"自由跑"，而界面初始态是停住（`main.rs` 同一行）。
fn production_engine_stopped(production: &ProductionLoop) {
    production.engine_handle().borrow_mut().stop();
}

/// 强制一次渲染 pass（`.slint` 的 `length` 绑定要真的被求值过，才有几何可读）。
///
/// ⚠ 不拿控件树当"已布局"的证据：新 UI 重构后视觉元素不再进无障碍树
/// （`tests/automation_labels_follow_playhead.rs` 的注释记着这一条）。
fn port_refresh(port: &mut LivePort<MainWindow>) {
    let _ = port.refresh_tree(None).expect("必须能抓控件树");
}

#[test]
fn playback_pages_the_arrangement_and_backs_off_when_the_user_scrolls() {
    let mut rig = Rig::new();

    // ── ① 几何真的从 `.slint` 到了宿主，且**不是**窗口宽 ────────────────────────
    let timeline = rig.timeline_px();
    let window_w = f32::from(u16::try_from(WINDOW.width).expect("窗口宽"));
    assert!(
        timeline > 0.0,
        "`arrangement-timeline-px` 必须是正数（实得 {timeline}）：0 表示 \
         `timeline-width-changed` 那条回调根本没接上，跟进会永久失效"
    );
    assert!(
        timeline < window_w,
        "可见宽必须**扣掉**包头与侧栏（实得 {timeline} ≥ 窗口宽 {window_w}）——\
         否则播放头会在包头底下消失一大段才翻页"
    );

    // 起播：上升沿重挂 Catch。这一跳播放头还在 0（可见），所以偏移一位不动。
    let before = rig.scroll_x();
    rig.play();
    rig.tick();
    assert_eq!(
        rig.scroll_x(),
        before,
        "界内不许动（起播这一跳的播放头在 0，可见）"
    );

    // ── ② 播放头越过右界 ⇒ 翻页，播放头回到左起 10% ───────────────────────────
    // 位置取一个**肯定在视口外**的 tick：可见宽 1000px 时，`4 × 1000px` 处的 30 tick/px
    // 换算必然出界。这里用可见宽现算，不写死像素。
    let far_ticks = u64::try_from((timeline * 4.0) as i64).expect("正数") * 30;
    rig.seek(far_ticks);
    rig.tick();
    let after = rig.scroll_x();
    assert!(
        after > before,
        "播放头出界后偏移必须增大（before={before}, after={after}）"
    );
    let expected = (rig.playhead_px() - timeline * 0.10).max(0.0);
    assert!(
        (after - expected).abs() <= 1.0,
        "新左界必须是「播放头 − 可见宽的 10%」（实得 {after}，期望 {expected}）"
    );
    let on_screen = rig.playhead_px() - after;
    report_line(&format!(
        "[follow] 可见宽={timeline}px 播放头={}px 偏移 {}→{} 屏上 x={on_screen}",
        rig.playhead_px(),
        before,
        after
    ));
    assert!(
        (0.0..=timeline).contains(&on_screen),
        "翻页后播放头必须回到可见区（屏上 x={on_screen}，可见区 0..={timeline}）"
    );
    assert!(
        (on_screen - timeline * 0.10).abs() <= 1.0,
        "翻页后播放头应落在左起 10% 处（实得 {on_screen}）"
    );

    // ── ③ 用户手动滚开 ⇒ 不再抢视图 ─────────────────────────────────────────
    // 用户滚到**别处**（滚回时间轴头部）：必须与跟进刚写的那个值不同，
    // 否则"偏移没变"本来就该判定为"用户没动过"。
    let user_px = 0.0;
    rig.user_scrolls_to(user_px);
    rig.tick(); // 这一跳发现偏移被改 ⇒ 脱开
    rig.seek(far_ticks * 4);
    rig.tick();
    assert_eq!(
        rig.scroll_x(),
        user_px,
        "用户滚开之后，跟进不许再动偏移（Logic 的 Catch 脱开）"
    );

    // ── ④ 停住不跟进；重新起播立刻恢复 ─────────────────────────────────────
    rig.stop();
    rig.tick();
    rig.seek(far_ticks * 8);
    rig.tick();
    assert_eq!(
        rig.scroll_x(),
        user_px,
        "停住时视图不许自己动（哪怕播放头在视口外）"
    );

    rig.play();
    rig.tick();
    let resumed = rig.scroll_x();
    report_line(&format!(
        "[follow] 用户滚到 {user_px} 后脱开；重播后偏移 {user_px}→{resumed}"
    ));
    assert!(
        resumed > user_px,
        "重新起播必须重新挂上 Catch（user_px={user_px}, resumed={resumed}）"
    );
    let expected = (rig.playhead_px() - rig.timeline_px() * 0.10).max(0.0);
    assert!(
        (resumed - expected).abs() <= 1.0,
        "重播后的落点同一条式子（实得 {resumed}，期望 {expected}）"
    );
}
