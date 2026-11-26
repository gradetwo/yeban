//! ⭐ T-FIX-569：编曲视图循环指示带的运行时行为判据（真窗口属性读写，非字符串 grep）。
//!
//! # 缺陷
//!
//! 产品只有「整曲循环开关」（`cycle_enabled` / `is_looping`），没有可设循环区间的事实源；
//! Rust 全树从不写 `cycle-start/end-ticks`。旧默认 0..15360 是从死文件
//! `workspace/arrangement_view.slint` 抄来的演示区间，开关一开就在活视图画出假循环带。
//!
//! # 判据（四臂，穿真窗口；红对照见下）
//!
//! | 臂 | 钉住的事实 |
//! | :--- | :--- |
//! | ① 默认无区间 | 根属性 `cycle-start-ticks == 0 && cycle-end-ticks == 0`——演示区间清零 |
//! | ② 开关同源 | `set_cycle_enabled(true)` + 宿主同步后 `is-looping == true`；两端仍相等 ⇒ 画带条件不成立，开关开也不画假带 |
//! | ③ 接线是活的 | 注入真实区间（写根属性）后两端按 px 换算分离，`end > start` 成立；区间回零后再度相等 |
//! | ④ 开关是另一半 | 区间非空但开关关闭时，`is-looping == false` ⇒ 条件不成立 |
//!
//! 画带条件与 `.slint` 逐字同源：`is-looping && cycle-end-px > cycle-start-px`，其中
//! `cycle-*-px = cycle-*-ticks / max(1, roll-ticks-per-pixel)`。判据用真窗口 getter 读回
//! 这些值并按同一公式判定 —— **红对照**：把 `app.slint` 默认改回 15360 ⇒ 臂①红。
//!
//! 平台约束：`Tier1Window::install` 每线程一次 ⇒ 本文件只留一个装平台的 `#[test]`。

#![allow(unused)]

use slint::ComponentHandle as _;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

/// 与 `.slint` 同源的画带条件（泵动真窗口属性后计算）。
fn band_visible(ui: &MainWindow) -> bool {
    let tpp = ui.get_roll_ticks_per_pixel().max(1);
    let start_px = ui.get_cycle_start_ticks() / tpp;
    let end_px = ui.get_cycle_end_ticks() / tpp;
    ui.get_is_looping() && end_px > start_px
}

fn open() -> LivePort<MainWindow> {
    let project = YebanProjectV1::default();
    let view = ViewState::from_project(&project).expect("夹具工程必须能投影");
    let scene = DemoScene::from_view(&view);
    LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 0)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

#[test]
fn cycle_band_has_no_fake_demo_range_and_obeys_switch_and_range() {
    let port = open();
    let ui = port.ui();
    // 显式切编曲视图 + 左置轨道头（默认 persona 可能落在右置/Session 分支）。
    ui.set_arrangement_view(true);
    ui.global::<yeban_app::ui::Theme<'_>>()
        .set_track_headers_on_right(false);
    ui.set_roll_ticks_per_pixel(8);

    // ① 默认无演示区间：两端 tick 必须都是 0（红对照：把 app.slint 默认改回 15360 即红）。
    assert_eq!(
        ui.get_cycle_start_ticks(),
        0,
        "无真实区间时 cycle-start-ticks 默认必须为 0"
    );
    assert_eq!(
        ui.get_cycle_end_ticks(),
        0,
        "旧 0..15360 演示区间必须清零（Rust 全树从不写这两个属性）"
    );
    assert!(!band_visible(ui), "首帧无区间、开关关，循环带不得显示");

    // ② 开关打开但无真实区间 ⇒ 条件仍不成立（开关与区间同源，不画假带）。
    ui.set_cycle_enabled(true);
    host::sync_aether_display(ui);
    assert!(
        ui.get_is_looping(),
        "cycle-enabled 必须同源同步到 is-looping"
    );
    assert_eq!(
        ui.get_cycle_start_ticks(),
        ui.get_cycle_end_ticks(),
        "开关打开也不能凭空造出区间"
    );
    assert!(
        !band_visible(ui),
        "开关开但区间为空，画带条件 is-looping && end>start 必须不成立"
    );

    // ③ 注入真实区间：条件成立（接线活），区间回零后条件再度不成立。
    ui.set_cycle_start_ticks(480);
    ui.set_cycle_end_ticks(1920);
    assert!(
        band_visible(ui),
        "注入真实区间 [480,1920] 后循环带条件必须成立"
    );
    ui.set_cycle_end_ticks(480);
    assert!(!band_visible(ui), "区间退化为零宽后循环带条件必须不成立");

    // ④ 开关关闭：即便区间非空也不画（开关是同一事实源的另一半）。
    ui.set_cycle_end_ticks(1920);
    ui.set_cycle_enabled(false);
    host::sync_aether_display(ui);
    assert!(!ui.get_is_looping(), "关闭循环开关 is-looping 必须同步为假");
    assert!(!band_visible(ui), "开关关闭时即便区间非空也不得画带");
}
