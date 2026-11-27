//! T-FIX-567：总线通道条三件套（声像旋钮 / 左栏总线推子 / 下坞总线推子）
//! **接到主总线单一事实源** —— 不再是"只改界面数字"的第二份假状态。
//!
//! ## 本文件要证的句子
//!
//! 派单证据：三处控件拖动只写本地 `bus-volume-db` / `bus-track-vol` / `bus-pan-norm`
//! （由 app.slint 两个 `<=>` 双向绑定串成一张网），Rust 全树无读写 ⇒ 声像、输出音量
//! 一点不变。主控通道条本身有真实通路（`mixer-master-fader/pan-*` 手势 ⇒ Op 提交）。
//! 修法：删掉第二份假状态，三处控件**读主控投影属性**、**手势走既有主控回调链**。
//!
//! ## 两层判据（不可互相代替）
//!
//! | 层 | 判据 | 读的是谁 |
//! | :--- | :--- | :--- |
//! | A 文本 | `a_fake_bus_state_is_gone_and_three_controls_read_master_source` | `.slint` 源文件（接线存在且唯一、假状态消失） |
//! | A 文本 | `a_master_callback_chain_is_declared_and_forwarded` | 回调声明 / 转发 / 宿主落点 |
//! | B 活窗口 | `master_fader_gesture_writes_project_and_moves_all_readouts` | 真 `MainWindow` + `UndoPort` ⇒ 权威工程真值 + 投影属性 |
//! | B 活窗口 | `master_pan_gesture_and_reset_write_project_and_move_readouts` | 同上（声相与双击归零） |
//!
//! 只看 A 会绿在"文本写了但宿主没接"；只看 B 会绿在"主控通路本来就能动、但三处假控件
//! 没接上去"。两层合起来才钉住"三处控件与主控通道条同源"。
//!
//! ## 变异表（每条都实测能变红）
//!
//! | 注入 | 文件 | 预期 |
//! | :--- | :--- | :--- |
//! | 把 left_panel 声像 TouchArea 的 `mixer-master-pan-grab` 换回 `bus-pan-dragging = true` | left_panel.slint | A1 红（假状态复活）|
//! | 把 app.slint 的 `bus-volume-fraction: root.master-volume-fraction;` 删掉 | app.slint | A1 红（读数不再同源）|
//! | 把 lower_dock 总线推子的 `mixer-master-fader-grab` 删掉 | lower_dock.slint | A1 红（手势不走主控）|
//! | 把 host.rs 的 `set_master_pan_value` 投影删掉 | host.rs | B 红（`master_pan_value` 不随拖动变）|

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use yeban_app::host;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::project::YebanProjectV1;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;

const NOW_MS: u64 = 1_700_000_000_000;

fn ui_src(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ui")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("读 ui/{name}: {e}"))
}

fn host_src() -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"))
        .expect("读 host.rs")
}

/// 带 UndoPort 的活窗口（与产品进程同一个 `wire_mixer_edit` 装配）。
fn live_rig(project: &YebanProjectV1) -> (MainWindow, Rc<UndoPort>, live::LiveUi) {
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "T-FIX-567 总线通道条同源",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let ui = live::build_live_ui_with(
        project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + 端口");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    (window, port, ui)
}

/// 权威工程里主总线的音量 dB / 声相 f32。
fn master_params(project: &YebanProjectV1) -> (f32, f32) {
    let id = project.master_bus_track_id;
    let track = project.tracks.get(&id).expect("主总线必须在工程里");
    (track.volume_db, track.pan)
}

fn master_id(project: &YebanProjectV1) -> EntityId {
    project.master_bus_track_id
}

// ===========================================================================
// A 层：文本（接线存在且唯一、假状态消失）
// ===========================================================================

#[test]
fn a_fake_bus_state_is_gone_and_three_controls_read_master_source() {
    let app = ui_src("app.slint");
    let left = ui_src("left_panel.slint");
    let dock = ui_src("lower_dock.slint");

    // ① 第二份假状态彻底消失（app 的两根 `<=>` 网 + 两个组件的属性声明）。
    assert!(
        !app.contains("bus-volume-db"),
        "app.slint 不得再声明/绑定假状态 bus-volume-db"
    );
    assert!(
        !left.contains("bus-track-vol")
            && !left.contains("bus-pan-norm")
            && !left.contains("bus-dragging"),
        "left_panel 的 bus-track-vol / bus-pan-norm / bus-dragging 假状态必须删除"
    );
    assert!(
        !dock.contains("bus-volume-db") && !dock.contains("bus-dragging"),
        "lower_dock 的 bus-volume-db / bus-dragging 假状态必须删除"
    );

    // ② 三处控件读主控事实源：left_panel 四个输入属性都从 master 注入。
    for line in [
        "bus-volume-fraction: root.master-volume-fraction;",
        "bus-volume-display: root.master-volume-display;",
        "bus-pan-value: root.master-pan-value;",
        "bus-pan-display: root.master-pan;",
    ] {
        assert!(app.contains(line), "app.slint 必须注入同源读数: {line}");
    }
    // 下坞总线推子读 master-volume-db（由 fraction 派生）与 master-volume-display。
    assert!(
        dock.contains("master-volume-db"),
        "lower_dock 总线推子必须读 master-volume-db"
    );

    // ③ 三处手势走主控回调：left_panel 两处 TouchArea 各走 fader / pan 链。
    assert!(
        left.contains("root.mixer-master-fader-grab(self.mouse-y)")
            && left.contains("root.mixer-master-fader-drag(self.mouse-y)")
            && left.contains("root.mixer-master-fader-release()")
            && left.contains("root.mixer-master-fader-reset()"),
        "左栏总线推子必须走 mixer-master-fader-* 手势链"
    );
    assert!(
        left.contains("root.mixer-master-pan-grab(self.mouse-x)")
            && left.contains("root.mixer-master-pan-drag(self.mouse-x)")
            && left.contains("root.mixer-master-pan-release()")
            && left.contains("root.mixer-master-pan-reset()"),
        "左栏声像旋钮必须走 mixer-master-pan-* 手势链"
    );
    // 下坞总线推子走 fader 链（app.slint 已把它的回调转发上去，见下一条判据）。
    assert!(
        dock.contains("root.mixer-master-fader-grab(self.mouse-y)")
            && dock.contains("root.mixer-master-fader-reset()"),
        "下坞总线推子必须走 mixer-master-fader-* 手势链"
    );

    // ④ 读数不是自算：left_panel 不再用 Math 拼 L/C/R，直接显示 master-pan 文案。
    assert!(
        !left.contains("Math.round(Math.abs(root.bus-pan-norm)"),
        "声像读数不得再自算 L/C/R（第二份口径）"
    );
    assert!(
        left.contains("text: root.bus-pan-display;"),
        "声像读数必须显示同源的 bus-pan-display"
    );
}

#[test]
fn a_master_callback_chain_is_declared_and_forwarded() {
    let app = ui_src("app.slint");
    let left = ui_src("left_panel.slint");
    let host = host_src();

    // left_panel 声明 8 条主控回调，app 把每一条转发到 MainWindow。
    for cb in [
        "mixer-master-fader-grab",
        "mixer-master-fader-drag",
        "mixer-master-fader-release",
        "mixer-master-fader-reset",
        "mixer-master-pan-grab",
        "mixer-master-pan-drag",
        "mixer-master-pan-release",
        "mixer-master-pan-reset",
    ] {
        assert!(
            left.contains(&format!("callback {cb}(")) || left.contains(&format!("callback {cb}()")),
            "left_panel 必须声明回调 {cb}"
        );
        assert!(
            app.contains(&format!("{cb}(")),
            "app.slint 必须转发回调 {cb}"
        );
    }

    // 宿主主控臂与复位落点在位（复用既有真实通路，不是新造第二套）。
    for arm in [
        "on_mixer_master_fader_grab",
        "on_mixer_master_fader_release",
        "on_mixer_master_pan_grab",
        "on_mixer_master_pan_release",
        "on_mixer_master_fader_reset",
        "on_mixer_master_pan_reset",
    ] {
        assert!(host.contains(arm), "host.rs 必须含主控回调臂 {arm}");
    }
    assert!(
        host.contains("fn commit_mixer_reset")
            && host.contains("fn preview_master_volume")
            && host.contains("fn preview_master_pan"),
        "主控手势必须汇到既有的投影与复位落点"
    );
    // 声像归一化数值投影存在（三处声像指针/读数同源的数值源）。
    assert!(
        host.contains("set_master_pan_value"),
        "host.rs 必须投影 master-pan-value（声像数值同源源）"
    );
}

// ===========================================================================
// B 层：活窗口（真回调 ⇒ 权威工程真值 + 投影读数一起动）
// ===========================================================================

#[test]
fn master_fader_gesture_writes_project_and_moves_all_readouts() {
    let project = filled_project();
    let mid = master_id(&project);
    let (window, port, _ui) = live_rig(&project);

    let (vol_before, _) = master_params(&port.try_project().unwrap());
    let frac_before = window.get_master_volume_fraction();
    // 起点：grab 记起点指针 y，drag 让指针上移 24px ⇒ 音量增大。
    let y0 = 300.0;
    window.invoke_mixer_master_fader_grab(y0);
    window.invoke_mixer_master_fader_drag(y0 - 24.0);
    // 拖动期预览：投影属性已经跟着动（与主控通道条同一份）。
    let frac_preview = window.get_master_volume_fraction();
    assert!(
        frac_preview > frac_before,
        "拖动期必须预览写 master-volume-fraction（{frac_before} -> {frac_preview}）"
    );
    window.invoke_mixer_master_fader_release();

    let (vol_after, _) = master_params(&port.try_project().unwrap());
    assert!(
        vol_after > vol_before,
        "松手必须把音量写进权威工程（{vol_before} -> {vol_after}）"
    );
    // 提交后重投影：界面读数与工程真值同源（fraction 由同一 volume_fraction 派生）。
    let expected_frac = ((vol_after - (-60.0)) / (6.0 - (-60.0))).clamp(0.0, 1.0);
    let frac_after = window.get_master_volume_fraction();
    assert!(
        (frac_after - expected_frac).abs() < 1e-3,
        "提交后界面 fraction 必须与工程真值同源（{frac_after} vs {expected_frac}）"
    );

    // 双击归零：走到真实 commit_mixer_reset，工程回到 0 dB。
    window.invoke_mixer_master_fader_reset();
    let (vol_reset, _) = master_params(&port.try_project().unwrap());
    assert_eq!(
        vol_reset.to_bits(),
        0.0_f32.to_bits(),
        "双击归零必须把主总线音量写成 0 dB（逐位）"
    );
    let _ = mid;
}

#[test]
fn master_pan_gesture_and_reset_write_project_and_move_readouts() {
    let project = filled_project();
    let (window, port, _ui) = live_rig(&project);

    let (_, pan_before) = master_params(&port.try_project().unwrap());
    let pan_val_before = window.get_master_pan_value();
    let x0 = 200.0;
    window.invoke_mixer_master_pan_grab(x0);
    // 右拖 90px（MIXER_PAN_TRAVEL_PX = 90 附近）⇒ 声相右移。
    window.invoke_mixer_master_pan_drag(x0 + 90.0);
    let pan_val_preview = window.get_master_pan_value();
    assert!(
        pan_val_preview > pan_val_before,
        "拖动期必须预览写 master-pan-value（{pan_val_before} -> {pan_val_preview}）"
    );
    // 文本读数与数值同源（右移 ⇒ R 开头）。
    let pan_text = window.get_master_pan().to_string();
    assert!(
        pan_text.starts_with('R') || pan_text == "C",
        "master-pan 文本必须与右移数值同源，实际: {pan_text}"
    );
    window.invoke_mixer_master_pan_release();

    let (_, pan_after) = master_params(&port.try_project().unwrap());
    assert!(
        pan_after > pan_before,
        "松手必须把声相写进权威工程（{pan_before} -> {pan_after}）"
    );

    // 双击归中：工程声相回到 0.0，数值文本回到 C。
    window.invoke_mixer_master_pan_reset();
    let (_, pan_reset) = master_params(&port.try_project().unwrap());
    assert_eq!(
        pan_reset.to_bits(),
        0.0_f32.to_bits(),
        "双击归中必须把主总线声相写成 0.0（逐位）"
    );
    assert_eq!(
        window.get_master_pan().as_str(),
        "C",
        "归中后 master-pan 文本必须回到 C"
    );
    assert!(
        window.get_master_pan_value().abs() < 1e-6,
        "归中后 master-pan-value 必须回到 0"
    );
}
