//! ⭐ **点编曲标尺 ⇒ 走带光标到那一拍** —— 端到端判据。
//!
//! ## 被补上的缺口
//!
//! `ui/workspace/arrangement_view.slint` 的小节 / 拍标尺带（`arrangement-ruler`）此前
//! **没有任何点击源**：它声明了 `accessible-role: text` 与语义 ID，画了小节线，但点上去
//! 一位都不动。宿主侧的走带定位只有一条路 —— `host.rs` 里 `engine.seek(0)`（`#227` 的
//! "连按 `Space` 回起点"），因此**除了 0 以外没有任何地方可以让光标去**。
//!
//! 本片把它接成一条真实的指针链：
//!
//! ```text
//! 真实指针事件（本文件的注入）→ Slint 命中测试 → 标尺带的零像素 TouchArea
//!   → ruler-seek(x) → host::ruler_seek_tick → EngineHost::seek → TransportCommand::SeekTicks
//!   → apply_transport（走带光标 / 时间码的唯一写者，读数取自引擎而不是界面）
//! ```
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（点在第 3 条小节线上 ⇒ 到第 3 小节） | 在投影给出的那条线的**列**上按下 ⇒ 引擎 `position_ticks == 3 × bar_length_ticks`，界面镜面 `playhead-ticks` 同值，动作日志末条是 `SeekTicks(该值)` |
//! | ②（落点与点击同列，往返成立） | `bridge::tick_to_px(引擎位置, 缩放) == 点击列` ⇒ "画在哪一列"与"停在哪一列"是同一个整数网格（不多不少一列） |
//! | ③（缩放换了，网格还是那一个） | 换成 `60 tick/px` 并重投影后，改点第 5 条小节线 ⇒ 仍旧落到 `5 × bar_length_ticks`（换算用的是**投影的**缩放，不是常量 30） |
//! | ④（点出网格右缘 ⇒ 夹在右缘） | 缩到 `960 tick/px`（最粗），点在网格右缘之右的**窗口内真实坐标** ⇒ 位置 == `bar_positions.len() × bar_length_ticks`，⛔ 不把光标送去没画网格的地方 |
//! | ⑤（非有限的读数 ⇒ 一位不动） | 直接调回调送 `NaN` ⇒ 位置不变、动作日志**长度不变**（⛔ 不把 NaN 当成"往右无穷远"） |
//! | ⑥（定位不改走带状态） | 播放中点标尺 ⇒ `state` 仍是 `Playing`（seek 只搬位置，⛔ 不起播也不停播） |
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ **不证明"小节 / 拍吸附"**：吸附语义在 Normative 规范里没有定义（`DESKTOP_UI_UX`
//!   那份 2026-10-06 起是参考件、非绑定），本片如实做**逐 tick** 定位；该缺口登记为
//!   `docs/ledger/app-ruler-seek-notes.md` 的 **needs R1** 待人类裁决。
//! - ⛔ **不证明"出声"**：`tests/rewind_sound.rs` 已经钉住"回 0 之后必须真的发声"，
//!   本文件只断言**控制面**位置与日志（同一个 `EngineHost::seek` 入口，⛔ 不新造第二条）。
//! - ⛔ **不证明"不改工程"**：这里没有 `UndoPort`（走带定位是会话运行态，
//!   `MODEL-ISO-001` 第二层）；接线代码里一次 `commit_ops` 都不调，本文件用 ⑥ 的
//!   "状态不变"覆盖可见后果。
//! - 没有真实音频设备：`ProductionLoop::start_with_audio_source(.., 0, None)` 与控制面
//!   驱动的 `pump` 让 `seek` 在 CI 的无显卡/无声卡环境里同样生效。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个建窗口的 `#[test]` 只建
//! **一个** `LivePort`（与 `tests/rewind_sound.rs`、`tests/splitter_drag.rs` 同款）。

use slint::Model as _;
use std::cell::RefCell;
use std::rc::Rc;
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EngineHost;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_engine::ring::TransportCommand;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);
/// 标尺带的语义 ID（`elements.rs` 已注册，本片**不**新增节点）。
const RULER: &str = "arrangement-ruler";
/// 标尺带的纵向中点：带高 22px ⇒ `11.0` 必然落在带内（横向才是本片的变量）。
const RULER_MID_Y: f64 = 11.0;

/// 生产形状的夹具：真引擎（`ProductionLoop`）＋ 真活窗口 ＋ **产品那两个**接线函数。
struct Rig {
    /// 持有它 ⇒ `EngineRebuild::collector` 的消费端一直在（与 `main.rs` 同一装配）。
    _production: ProductionLoop,
    engine: Rc<RefCell<EngineHost>>,
    /// 与 `main.rs` 传给 `wire_clip_scope` / `wire_scene_launch` 的**同一种**活投影句柄。
    view: Rc<RefCell<ViewState>>,
    port: LivePort<MainWindow>,
    project: yeban_model::YebanProjectV1,
}

impl Rig {
    fn new() -> Self {
        let project = demo_project();
        let production =
            ProductionLoop::start_with_audio_source(&project, 0, None).expect("产品装配建引擎");
        assert!(production.has_engine(), "引擎必须活着");
        let engine = production.engine_handle();
        // `src/main.rs` 的同一行：新引擎默认自由跑，而界面初始态是停住。
        engine.borrow_mut().stop();

        let view = ViewState::from_project(&project).expect("演示工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");

        let live_view = Rc::new(RefCell::new(view));
        // ⭐ 与产品装配**同一对**函数：`main.rs` 的 `wire_callbacks` 调的就是这两个。
        host::wire_transport(port.ui(), Rc::clone(&engine), None, None);
        host::wire_ruler_seek(port.ui(), Rc::clone(&engine), Rc::clone(&live_view));
        host::apply_transport(port.ui(), engine.borrow().transport());

        let mut rig = Self {
            _production: production,
            engine,
            view: live_view,
            port,
            project,
        };
        // 控件树先抓一次：`dispatch_pointer_down` 按**缓存的**包围盒换算窗口坐标。
        rig.port
            .refresh_tree(None)
            .expect("必须能抓控件树（标尺带的几何来自这一次抓取）");
        rig
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    /// 投影画出的第 `index` 条小节线在**标尺带局部坐标系**里的列（= 界面自己的读数）。
    fn bar_line_px(&self, index: usize) -> f64 {
        // `ModelRc` 上没有 `.rows()` / `.row()`（本文件上一版因此编译失败）。既有口径：
        // 行数走 UFCS `slint::Model::row_count(&…)`（`host.rs:4406`、`live_surface.rs:1002`），
        // 逐项走 `.iter().nth(…)`（`host.rs:4356` 的 `read_strings`，`iter()` 是 `Model`
        // trait 的方法 ⇒ 顶部那句 `use slint::Model as _;` 由它消费）。
        let model = self.ui().get_bar_positions();
        let rows = slint::Model::row_count(&model);
        assert!(
            index < rows,
            "投影必须画出第 {index} 条小节线（实得 {rows} 条）"
        );
        f64::from(model.iter().nth(index).expect("逐项必有值"))
    }

    /// 引擎的走带位置（**事实源**，不是界面镜面）。
    fn position(&self) -> u64 {
        self.engine.borrow().transport().position_ticks
    }

    fn journal_len(&self) -> usize {
        self.engine.borrow().transport_journal().len()
    }

    /// 动作日志的末条命令。
    fn last_command(&self) -> Option<TransportCommand> {
        self.engine
            .borrow()
            .transport_journal()
            .last()
            .map(|record| record.command)
    }

    /// 在标尺带上真实按下一次：`(局部 x, 带中点)` ⇒ 走命中测试，⛔ 不直接调回调。
    fn click_ruler(&mut self, local_x: f64) {
        self.port
            .dispatch_pointer_down(RULER, local_x, RULER_MID_Y, PointerButton::Left)
            .expect("Interactive 权限下标尺带必须可按下");
        self.port
            .dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
    }

    /// 换横向缩放并**重投影**（与 `wire_clip_scope` 的 fresh 路径同一手法）：
    /// 缩放是视图态 ⇒ 走带与工程都不动，只有网格的像素口径变。
    fn set_zoom(&mut self, ticks_per_pixel: u64) {
        if !host::set_roll_ticks_per_pixel(self.ui(), ticks_per_pixel) {
            return;
        }
        let fresh = host::project_with_view_state(&self.project, self.ui())
            .expect("演示工程在新缩放下必须能投影");
        let scroll = self.ui().get_roll_scroll_x();
        host::apply_view(self.ui(), &fresh, f64::from(WINDOW.width) as f32, scroll);
        *self.view.borrow_mut() = fresh;
    }
}

/// ① + ②：点在第 3 条小节线上 ⇒ 引擎真的到第 3 小节起点，且落点与点击同列。
#[test]
fn clicking_the_third_bar_line_seeks_to_that_bar() {
    let mut rig = Rig::new();
    let bar_ticks = rig.view.borrow().bar_length_ticks;
    let column = rig.bar_line_px(3);

    let journal_before = rig.journal_len();
    rig.click_ruler(column);

    let expected = 3 * bar_ticks;
    assert_eq!(
        rig.position(),
        expected,
        "点在第 3 条小节线（第 {column:.0} 列）必须落到第 3 小节起点"
    );
    assert_eq!(
        rig.ui().get_playhead_ticks(),
        i32::try_from(expected).expect("夹具的 tick 必须在 i32 内"),
        "界面镜面必须回写引擎读数（`apply_transport` 是唯一写者）"
    );
    assert_eq!(
        rig.last_command(),
        Some(TransportCommand::SeekTicks(expected)),
        "必须发既有的 `SeekTicks`，⛔ 不新造第二条定位通路"
    );
    assert_eq!(
        rig.journal_len(),
        journal_before + 1,
        "一次按下 = 一条走带命令（⛔ 不在拖动 / 释放里重复发）"
    );

    // ②：往返 —— 落点换算回像素必须正好是点下去的那一列（同一整数网格）。
    let zoom = host::roll_ticks_per_pixel(rig.ui());
    let back = yeban_app::bridge::tick_to_px(rig.position(), zoom).expect("落点必须能换算回像素");
    assert_eq!(
        f64::from(back),
        column,
        "点在第 {column:.0} 列就必须停在第 {column:.0} 列（实际停在了第 {} 列）",
        back
    );
}

/// ③：缩放换了 ⇒ 点击的列换了，但**同一格小节**仍是同一个 tick（换算用投影的缩放）。
#[test]
fn the_click_follows_the_projection_zoom_not_a_constant() {
    let mut rig = Rig::new();
    let bar_ticks = rig.view.borrow().bar_length_ticks;

    rig.set_zoom(60);
    let column = rig.bar_line_px(5);
    rig.click_ruler(column);

    assert_eq!(
        rig.position(),
        5 * bar_ticks,
        "在 60 tick/px 下点第 5 条小节线必须仍落到第 5 小节起点（点击列 = {column:.0}）"
    );
    let back = yeban_app::bridge::tick_to_px(rig.position(), 60).expect("落点必须能换算回像素");
    assert_eq!(
        f64::from(back),
        column,
        "换了缩放之后，画的那一列与停下的那一列仍必须相同"
    );
}

/// ④：点出网格右缘 ⇒ 夹在右缘（最后一条小节线右边那一整格的末端）。
#[test]
fn a_click_right_of_the_drawn_grid_clamps_to_its_right_edge() {
    let mut rig = Rig::new();
    // 最粗的一档：网格只有几十列宽，于是"右缘之右"是一个**窗口内的真实坐标**（不是编的数）。
    rig.set_zoom(yeban_app::bridge::MAX_TICKS_PER_PIXEL);
    let span = rig.view.borrow().bar_positions.len() as u64 * rig.view.borrow().bar_length_ticks;
    let edge_column =
        f64::from(u32::try_from(span / yeban_app::bridge::MAX_TICKS_PER_PIXEL).unwrap());
    assert!(
        edge_column < f64::from(WINDOW.width) - 168.0,
        "夹具前置：网格右缘必须落在标尺带内，否则这一条测不到夹紧（右缘列 = {edge_column:.0}）"
    );

    rig.click_ruler(edge_column + 40.0);
    assert_eq!(
        rig.position(),
        span,
        "点出网格右缘 40px ⇒ 停在右缘那一 tick（实得 {}），⛔ 不 seek 到没画网格的地方",
        rig.position()
    );
}

/// ⑤：非有限的指针读数 ⇒ **不响应**（位置与动作日志一位都不动）。
#[test]
fn a_non_finite_pointer_reading_moves_nothing() {
    let mut rig = Rig::new();
    rig.click_ruler(rig.bar_line_px(2));
    let position = rig.position();
    let journal = rig.journal_len();

    // 命中测试不会给出 NaN，但回调是**公共入口**（产品路径与判据共用同一个注册点）⇒
    // 它必须对畸形读数如实"什么都不做"，而不是当成"往右无穷远"。
    rig.ui().invoke_ruler_seek(f32::NAN);
    rig.ui().invoke_ruler_seek(f32::INFINITY);

    assert_eq!(rig.position(), position, "NaN / Inf 之后位置必须一位不动");
    assert_eq!(
        rig.journal_len(),
        journal,
        "NaN / Inf 之后必须一条命令都没发"
    );
}

/// ⑥：定位只搬位置 ⇒ 走带状态不变（播放中点标尺不会起播也不会停播）。
#[test]
fn seeking_from_the_ruler_keeps_the_transport_state() {
    let mut rig = Rig::new();
    rig.engine.borrow_mut().play();
    assert!(
        rig.engine.borrow().transport().state.is_running(),
        "夹具前置：必须在播放中"
    );

    rig.click_ruler(rig.bar_line_px(4));

    assert!(
        rig.engine.borrow().transport().state.is_running(),
        "点标尺之后必须仍在播放 —— seek 只搬位置（`TransportCommand::SeekTicks` 的既有语义）"
    );
    assert_eq!(
        rig.position(),
        4 * rig.view.borrow().bar_length_ticks,
        "播放中定位同样生效"
    );
}

// ===========================================================================
// 纯函数侧：夹紧 / 拒绝的三条口径（不建窗口，⛔ 不需要平台）
// ===========================================================================

/// 左端 = 工程起点：负的 x ⇒ 0（夹紧，⛔ 不回绕）。
#[test]
fn a_negative_column_seeks_to_the_start() {
    assert_eq!(host::ruler_seek_tick(-250.0, 30, 65_280), Some(0));
    assert_eq!(host::ruler_seek_tick(-0.5, 30, 65_280), Some(0));
}

/// 一个像素列覆盖 `[p×tpp, (p+1)×tpp)` ⇒ 取整方向与 `tick_to_px` 互逆。
#[test]
fn the_column_is_truncated_like_the_projection_division() {
    // 同一列里的任意小数都得到同一个 tick（点在一列的左半边与右半边不该差一个 tick）。
    assert_eq!(host::ruler_seek_tick(100.0, 30, u64::MAX), Some(3_000));
    assert_eq!(host::ruler_seek_tick(100.9, 30, u64::MAX), Some(3_000));
    assert_eq!(host::ruler_seek_tick(101.0, 30, u64::MAX), Some(3_030));
    // 缩放为 0 是外力写坏的属性：按 `max(1)` 处理而⛔ 不 panic。
    assert_eq!(host::ruler_seek_tick(100.0, 0, u64::MAX), Some(100));
}

/// 右端 = 网格右缘；非有限 ⇒ `None`（不响应）。
#[test]
fn a_column_past_the_grid_clamps_and_a_non_finite_one_is_refused() {
    assert_eq!(host::ruler_seek_tick(1e6, 30, 65_280), Some(65_280));
    assert_eq!(host::ruler_seek_tick(f32::NAN, 30, 65_280), None);
    assert_eq!(host::ruler_seek_tick(f32::INFINITY, 30, 65_280), None);
    assert_eq!(host::ruler_seek_tick(f32::NEG_INFINITY, 30, 65_280), None);
}

/// 网格右缘：溢出 ⇒ 饱和（真实工程到不了；⛔ 不回绕成小数）。
#[test]
fn the_grid_right_edge_is_the_line_count_times_one_bar() {
    assert_eq!(host::ruler_span_ticks(17, 3_840), 65_280);
    assert_eq!(host::ruler_span_ticks(0, 3_840), 0);
    assert_eq!(host::ruler_span_ticks(usize::MAX, u64::MAX), u64::MAX);
}
