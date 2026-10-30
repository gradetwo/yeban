//! **走带光标跟随引擎读数** —— 端到端判据（[UI-NOTE-001] / [UI-MCP-002]）。
//!
//! ## 本文件要证的句子
//!
//! 编曲视图与卷帘的两条 `playhead` **不是一个停在假位置的装饰**：`host::apply_transport`
//! 把引擎的 `TransportReading::position_ticks` 写成窗口属性 `playhead-ticks`，两条光标
//! 用 `tick ÷ ticks_per_pixel`（与投影里音符 / 小节线同一条整数除法）派生 x ⇒
//! 播放推进时光标跟着走，无障碍值 `accessible-value` 是**同一刻的时间码**，
//! 而不是旧版写死的 `"3"`（旧几何：`168px + 120px * 3` / `56px + 60px * 3`）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（装配态） | tick 0 ⇒ 编曲光标 x = 画布左沿 + 168px、卷帘光标 x = 卷帘左沿 + 56px；两条 `accessible-value` 都 = 窗口时间码 |
//! | ②（真读数 → 真几何） | `apply_transport` 写第 3 小节第 2 拍 ⇒ 重抓树后两条光标都精确移动 `tick ÷ tpp` 像素（⛔ 不是只看属性） |
//! | ③（回 0） | 再写 `TransportReading::cold()` ⇒ 两条光标回原位、值回 `001.01.000`（位置不是累加的） |
//! | ④（文本层接线） | `.slint` 里假位置 / 假值已删除、两条 x 都读 `playhead-ticks`；`app.slint` 原样转发；`host.rs` 只有一个 `set_playhead_ticks` 写者 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/revert_button.rs` / `tests/console_maximize.rs` 同款）。

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_engine::transport::{TransportReading, TransportState};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Rect, Size};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);

fn live_window() -> (LivePort<yeban_app::ui::MainWindow>, ViewState) {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    (port, view)
}

/// 重抓控件树之后取一个节点的几何与无障碍值。
fn node(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) -> (Rect, Option<String>) {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    let node = tree
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"));
    (
        node.bounds
            .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒")),
        node.value.clone(),
    )
}

fn bounds_of(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) -> Rect {
    node(port, id).0
}

#[test]
fn both_playheads_follow_the_engine_transport_reading() {
    let (mut port, view) = live_window();

    // ---- ① 装配态：tick 0 ⇒ 两条光标都在时间轴左沿，值 = 时间码 ----
    let canvas = bounds_of(&mut port, "workspace-arrangement-canvas");
    let roll = bounds_of(&mut port, "piano-roll");
    let (arrangement0, arrangement_value0) = node(&mut port, "arrangement-playhead");
    let (roll0, roll_value0) = node(&mut port, "piano-roll-playhead");
    assert_eq!(
        arrangement0.x,
        canvas.x + 168,
        "tick 0 的编曲光标 x 必须 = 画布左沿 + 168px（旧版是假的 + 360px）"
    );
    assert_eq!(
        arrangement0.y,
        canvas.y + 20,
        "编曲光标上沿必须钉在画布内 y=20"
    );
    assert_eq!(arrangement0.width, 1, "编曲光标必须是 1px 竖线");
    assert_eq!(
        roll0.x,
        roll.x + 56,
        "tick 0 的卷帘光标 x 必须 = 卷帘左沿 + 56px（旧版是假的 + 180px）"
    );
    assert_eq!(roll0.y, roll.y + 28, "卷帘光标上沿必须钉在卷帘内 y=28");
    assert_eq!(roll0.width, 1, "卷帘光标必须是 1px 竖线");

    let timecode0 = port.ui().get_timecode().to_string();
    assert_eq!(timecode0, "001.01.000", "装配态时间码必须是起点");
    assert_eq!(
        arrangement_value0.as_deref(),
        Some(timecode0.as_str()),
        "无障碍值必须就是走带时间码（⛔ 不是写死的 \"3\"）"
    );
    assert_eq!(
        roll_value0.as_deref(),
        Some(timecode0.as_str()),
        "卷帘光标的无障碍值必须与编曲读同一份时间码"
    );

    // ---- ② 引擎读数推进 ⇒ 两条光标按同一映射移动 ----
    let tpp = u64::try_from(port.ui().get_roll_ticks_per_pixel()).expect("缩放必须是正数");
    assert_eq!(
        tpp, view.ticks_per_pixel,
        "前提：窗口缩放读数与投影的 ticks_per_pixel 是同一个值"
    );
    // 第 3 小节第 2 拍（4/4 夹具）：bar = 4 拍 × 960 tick。
    let tick = 2 * view.bar_length_ticks + view.bar_length_ticks / 4;
    let expected_shift = i32::try_from(tick / tpp).expect("像素位移必须落在 i32 内");
    assert!(expected_shift > 0, "前提：位移必须为正: {expected_shift}px");

    let reading = TransportReading {
        state: TransportState::Stopped,
        position_ticks: tick,
        ..TransportReading::cold()
    };
    host::apply_transport(port.ui(), reading);
    assert_eq!(
        port.ui().get_playhead_ticks(),
        i32::try_from(tick).expect("tick 必须落在 i32 内"),
        "`playhead-ticks` 必须逐字等于引擎读数"
    );

    let (arrangement1, arrangement_value1) = node(&mut port, "arrangement-playhead");
    let (roll1, roll_value1) = node(&mut port, "piano-roll-playhead");
    assert_eq!(
        arrangement1.x,
        arrangement0.x + expected_shift,
        "编曲光标必须精确移动 tick ÷ tpp = {expected_shift}px"
    );
    assert_eq!(
        roll1.x,
        roll0.x + expected_shift,
        "卷帘光标必须用同一条映射移动 {expected_shift}px"
    );
    assert_eq!(arrangement1.y, arrangement0.y, "纵向位置不动");
    assert_eq!(roll1.y, roll0.y, "纵向位置不动");

    let expected_timecode = yeban_app::bridge::timecode_for_ticks(view.timecode_grid(), tick);
    assert_ne!(expected_timecode, timecode0, "前提：时间码必须真的变了");
    assert_eq!(
        port.ui().get_timecode(),
        expected_timecode,
        "时间码与光标必须来自同一次读数"
    );
    assert_eq!(
        arrangement_value1.as_deref(),
        Some(expected_timecode.as_str()),
        "编曲光标的无障碍值必须跟着读数走"
    );
    assert_eq!(
        roll_value1.as_deref(),
        Some(expected_timecode.as_str()),
        "卷帘光标的无障碍值必须跟着读数走"
    );

    // ---- ③ 回到 0 ⇒ 两条光标回原位（位置不是累加的） ----
    host::apply_transport(port.ui(), TransportReading::cold());
    assert_eq!(port.ui().get_playhead_ticks(), 0);
    let (arrangement2, arrangement_value2) = node(&mut port, "arrangement-playhead");
    let (roll2, roll_value2) = node(&mut port, "piano-roll-playhead");
    assert_eq!(
        (arrangement2.x, arrangement2.y),
        (arrangement0.x, arrangement0.y)
    );
    assert_eq!((roll2.x, roll2.y), (roll0.x, roll0.y));
    assert_eq!(arrangement_value2.as_deref(), Some(timecode0.as_str()));
    assert_eq!(roll_value2.as_deref(), Some(timecode0.as_str()));

    // ---- ④ 文本层：假位置 / 假值已删除、同一个写者 ----
    let arrangement = include_str!("../ui/workspace/arrangement_view.slint");
    assert!(
        arrangement.contains("root.playhead-ticks / max(1, root.ticks-per-pixel)"),
        "编曲光标 x 必须由 `playhead-ticks` 派生: {arrangement}"
    );
    assert!(
        !arrangement.contains("120px * 3"),
        "写死的假位置 `120px * 3` 必须已删除"
    );
    assert!(
        !arrangement.contains("accessible-value: \"3\""),
        "写死的假无障碍值 \"3\" 必须已删除"
    );

    let roll_source = include_str!("../ui/console/piano_roll.slint");
    assert!(
        roll_source.contains("root.playhead-ticks / max(1, root.ticks-per-pixel)"),
        "卷帘光标 x 必须由 `playhead-ticks` 派生: {roll_source}"
    );
    assert!(
        roll_source.contains("- (root.scroll-x * 1px)"),
        "卷帘光标必须减去与音符位置同一个滚动读数"
    );
    assert!(
        !roll_source.contains("60px * 3"),
        "写死的假位置 `60px * 3` 必须已删除"
    );

    let app = include_str!("../ui/app.slint");
    assert!(
        app.contains("in-out property <int> playhead-ticks: 0;"),
        "`app.slint` 必须声明走带光标位置属性"
    );
    assert_eq!(
        app.matches("playhead-ticks: root.playhead-ticks;").count(),
        2,
        "编曲与卷帘两条光标都必须拿同一份 `playhead-ticks`"
    );

    let source = include_str!("../src/host.rs");
    assert_eq!(
        source.matches("ui.set_playhead_ticks(").count(),
        1,
        "`playhead-ticks` 必须**恰好**有一个写者（`apply_transport`）"
    );
}
