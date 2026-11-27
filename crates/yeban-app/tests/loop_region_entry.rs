//! **循环区间入口**（T-FIX-889）—— 端到端判据。
//!
//! ## 前提（ISS-MWYH9VNB-56D6D9，T-FIX-616 真实 UI 走查）
//!
//! 编曲 UI 只有一个循环开/关开关（`transport-loop-button`），没有设置循环区间的
//! 入口：`cycle-start-ticks` / `cycle-end-ticks` 作为窗口属性存在，但无控件可写；
//! 既有判据 `cycle_band_no_fake_range.rs` 只靠直写根属性注入区间。本单在走带栏
//! 加「按选区设循环」按钮（`transport-set-loop-from-selection-button`）＋区间读数
//! （`transport-loop-range-readout`），宿主按选中剪辑写两个 tick 属性。
//!
//! ## 本文件要证的句子（一个活窗口，全部走控件树动作）
//!
//! | 臂 | 证的事 |
//! | :--- | :--- |
//! | ① 入口在场 | 控件树里有新按钮与新读数；按钮有几何（宽随文案走，⛔ 不塌成 0 宽） |
//! | ② 无选区响亮拒绝 | 选区为空时点按钮：tick 一位不动、读数仍 `"—"`、错误级提示条出现（`toast-is-error`），⛔ 不静默吞掉 |
//! | ③ 真实 UI 入口写区间 | 控件树动作选中片段 → 控件树动作点按钮 ⇒ `cycle-start/end-ticks` == 该片段的起止 tick，`cycle-enabled` 与 `is-looping` 同开，读数 == `"{start} - {end}"`，画带几何条件（`is-looping && cycle-end-px > cycle-start-px`，与 `arrangement.slint` 的 `cycle-range-band` 逐字同款）成立；成功提示条出现且非错误级 |
//!
//! 选片段也走控件树动作（`clip-{ulid}-header` 的默认无障碍动作），⛔ 不走指针命中：
//! 这样本判据不依赖自动化泳道 TouchArea 的遮挡裁决（那是 T-FIX-888 的范围）；
//! 指针路径由 `clip_marquee_selection.rs` 另行看守。
//!
//! ## 语言前提
//!
//! 读数是语言无关的纯数字（`"{start} - {end}"`，未设时 `"—"`）；本文件**不**断言
//! 任何中文文案 —— 无选区臂只断言提示条非空＋错误级，成功臂只断言提示条非空＋非错误
//! 级。按钮文案与 tooltip 的 8 语翻译由 `i18n_coverage.rs` 另行看守（.po 同步）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 建窗口的判据只有一条
//! （①②③ 共用一个活窗口）；纯函数部分（`clip_selection_tick_span` /
//! `loop_range_is_valid`）由 `bridge.rs` / `host.rs` 的单测看守，不在本文件建窗。
//!
//! ## 黄金基线说明
//!
//! 本单在走带栏加了一个可见按钮＋一个读数 ⇒ 默认帧逐像素变化，`tests/golden/linux/`
//! 的基准确定性过期（重构期 golden 比对已禁用，见 `test_port_adapter.rs`）；
//! 合入后走 `gates-manual.yml` 的 `gate=goldens` ＋人工复核重录，⛔ 本线自行决定。
//! 本文件只留帧指纹（尺寸 / 非黑 / 颜色数 / FNV）当证据，不与旧基准比对。

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::host::{self};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::inspect::find_by_accessible_id;
use yeban_ui_test_port::render::{LivePort, golden_evidence, report_line};
use yeban_ui_test_port::{Permission, Size};

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;
/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);
/// 新按钮的语义 ID（`transport_bar.slint`，`elements.rs` 已登记）。
const SET_LOOP_BUTTON: &str = "transport-set-loop-from-selection-button";
/// 新读数的语义 ID（同上）。
const LOOP_READOUT: &str = "transport-loop-range-readout";

/// 触发一个按钮的**默认无障碍动作**（与鼠标点击是逐字同一落点）。
fn press(window: &MainWindow, id: &str) {
    find_by_accessible_id(window, id)
        .unwrap_or_else(|| panic!("控件树里没有 `{id}` —— 入口不在当前可见分支里（或 ID 改了）"))
        .invoke_accessible_default_action();
}

/// 画带几何条件（与 `arrangement.slint` 的 `cycle-range-band` 逐字同款，见
/// `cycle_band_no_fake_range.rs` 的同名辅助函数）：开关开着，且结束 px 严格大于
/// 开始 px（`cycle-*-px = cycle-*-ticks / max(1, roll-ticks-per-pixel)`）。
/// ⚠ 活带在 `arrangement.slint`（app.slint:11 进口的那个 `ArrangementViewport`），
/// ⛔ 不是 `ui/workspace/arrangement_view.slint` 里的同名带（那份只进 elements.rs
/// 的注册表元数据，不进活窗口；它的门是 `cycle-enabled`，活带是 `is-looping`）。
fn band_visible(ui: &MainWindow) -> bool {
    let tpp = ui.get_roll_ticks_per_pixel().max(1);
    let start_px = ui.get_cycle_start_ticks() / tpp;
    let end_px = ui.get_cycle_end_ticks() / tpp;
    ui.get_is_looping() && end_px > start_px
}

#[test]
fn set_loop_from_selection_writes_cycle_range_and_shows_band() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    // 演示工程第一条片段：MIDI 轨 0 上从 0 开始的那条（`samples.rs` 的 P1F9，
    // 0..3840）。期望值全部从投影现读，⛔ 不在判据里手抄 tick 数。
    let first = view
        .clips
        .iter()
        .find(|clip| clip.start_tick == 0)
        .expect("演示工程必须有从 0 开始的片段")
        .clone();
    let (want_start, want_end) = (first.start_tick, first.end_tick);
    assert!(want_end > want_start, "夹具片段必须有正长度");

    let scene = DemoScene::from_view(&view);
    let mut port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 主窗口");
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<loop-region-entry>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开"),
    ));
    let live_view = Rc::new(RefCell::new(view.clone()));
    host::wire_clip_scope(port.ui(), Rc::clone(&live_view), Some(Rc::clone(&undo)));

    // ① 入口在场：树里有新按钮与新读数；按钮宽随文案走（⛔ 不塌成 0 宽）。
    // ⚠ `refresh_tree` 要 `&mut port`，必须在 `let ui = port.ui()` 之前做 ——
    // `ui` 一旦借住，`port` 就借不出 `&mut` 了（`clip_selection.rs` 同款次序）。
    let tree = port.refresh_tree(None).expect("必须能抓控件树");
    let button = tree
        .find_by_id(SET_LOOP_BUTTON)
        .unwrap_or_else(|| panic!("控件树里没有 `{SET_LOOP_BUTTON}`"));
    let button_bounds = button
        .bounds
        .unwrap_or_else(|| panic!("`{SET_LOOP_BUTTON}` 必须有几何包围盒"));
    assert!(
        button_bounds.width > 60,
        "按钮宽必须随文案撑开（中文 6 字 + 内边距 ≈ 92px），实得 {}px —— 塌了就是布局没接上",
        button_bounds.width
    );
    assert!(
        tree.find_by_id(LOOP_READOUT).is_some(),
        "控件树里必须有 `{LOOP_READOUT}`"
    );

    // 从这里起只读窗口属性（`ui` 是共享借用，`port.window().capture()` 同款）。
    let ui = port.ui();

    // 前提态：默认无区间、无读数、开关关着、提示条空着。
    assert_eq!(ui.get_cycle_start_ticks(), 0, "默认 cycle-start 必须是 0");
    assert_eq!(ui.get_cycle_end_ticks(), 0, "默认 cycle-end 必须是 0");
    assert!(!ui.get_cycle_enabled(), "默认 cycle-enabled 必须是 false");
    assert!(!ui.get_is_looping(), "默认 is-looping 必须是 false");
    assert_eq!(
        ui.get_cycle_range_readout().as_str(),
        "—",
        "默认读数必须是 \"—\""
    );
    assert!(
        !band_visible(ui),
        "默认态无真实区间 ⇒ 不画带（cycle_band 口径）"
    );
    assert!(ui.get_toast_message().is_empty(), "前提：提示条默认空");

    // ② 无选区响亮拒绝：点按钮 ⇒ 一位不动 + 错误级提示条常驻。
    press(ui, SET_LOOP_BUTTON);
    assert_eq!(ui.get_cycle_start_ticks(), 0, "无选区时不许写 cycle-start");
    assert_eq!(ui.get_cycle_end_ticks(), 0, "无选区时不许写 cycle-end");
    assert!(!ui.get_cycle_enabled(), "无选区时不许开 cycle-enabled");
    assert_eq!(
        ui.get_cycle_range_readout().as_str(),
        "—",
        "无选区时读数不动"
    );
    assert!(
        !ui.get_toast_message().is_empty(),
        "无选区必须响亮拒绝（提示条非空）"
    );
    assert!(ui.get_toast_is_error(), "无选区拒绝必须是错误级（常驻）");
    assert!(
        !ui.get_module_status_text().is_empty(),
        "模块回执也必须同时写"
    );

    // ③ 控件树动作选中片段（`clip-{ulid}-header` 的默认动作 = 单击选中的同一落点）。
    let clip_header = format!("clip-{}-header", first.placement_id);
    press(ui, &clip_header);
    let selected: Vec<String> = ui
        .get_selected_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(
        selected,
        vec![first.placement_id.clone()],
        "点包头必须选中这一条片段"
    );

    // ③ 控件树动作点「按选区设循环」⇒ 区间 / 开关 / 读数 / 画带条件全数就位。
    press(ui, SET_LOOP_BUTTON);
    assert_eq!(
        ui.get_cycle_start_ticks(),
        i32::try_from(want_start).expect("演示 tick 在 i32 内"),
        "cycle-start 必须是选中片段的起点"
    );
    assert_eq!(
        ui.get_cycle_end_ticks(),
        i32::try_from(want_end).expect("演示 tick 在 i32 内"),
        "cycle-end 必须是选中片段的终点"
    );
    assert!(ui.get_cycle_enabled(), "设区间后 cycle-enabled 必须开");
    assert!(ui.get_is_looping(), "设区间后 is-looping 必须同步开");
    assert_eq!(
        ui.get_cycle_range_readout().as_str(),
        format!("{want_start} - {want_end}"),
        "读数必须是 \"{{start}} - {{end}}\"（语言无关的纯数字）"
    );
    assert!(band_visible(ui), "有真实区间 + 开关开 ⇒ 循环带可见");
    assert!(!ui.get_toast_message().is_empty(), "成功也必须有可见回执");
    assert!(!ui.get_toast_is_error(), "成功回执必须是信息级（非错误）");

    // 帧证据：尺寸非零且非全黑（`[MUST-GATE-015]`）；指纹进 stderr 日志。
    // `YEBAN_DUMP_FRAME_LOOP_REGION=<path>` 置了就把这一帧写盘，供人工看带与读数。
    let image = port.window().capture().expect("设循环后的帧必须能抓");
    let probe = golden_evidence(&image).expect("[MUST-GATE-015] 尺寸非零且非全黑");
    if let Ok(path) = std::env::var("YEBAN_DUMP_FRAME_LOOP_REGION") {
        let bytes = yeban_ui_test_port::png::encode_rgb8(&image);
        std::fs::write(&path, bytes).expect("写帧");
        report_line(&format!("[loop-region-entry] 帧已写盘：{path}"));
    }
    report_line(&format!(
        "[loop-region-entry] 区间 {} - {}，读数 \"{}\", {}",
        want_start,
        want_end,
        ui.get_cycle_range_readout().as_str(),
        probe.summary()
    ));
}
