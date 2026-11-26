//! T-FIX-571：相位对齐对话框的用户输入必须回写宿主根属性。
//!
//! 判据在**活窗口**上走真实命中测试与键盘字符注入（⛔ 不直接 `invoke_*` 回调，
//! 也不读 `.slint` 源码冒充行为）：
//!
//! | # | 判据 | 证的事 |
//! | :--- | :--- | :--- |
//! | ① | 基准轨 / 目标轨输入后根属性等于键入文本 | 四行实例绑定不是单向 |
//! | ② | maxLagMs 键入 `75` 后根属性等于 `75`，运行按它测量 | 参数通路到宿主运行路径 |
//! | ③ | maxLagMs 非数字 ⇒ 运行报原文错误 | 既有错误语义保留 |
//! | ④ | apply 复选框点击后根属性翻面，运行只测量不提交 | 开关通路 |
//! | ⑤ | 再点 apply ⇒ 运行测量并提交补偿 | 打开态真的提交 |
//! | ⑥ | 关掉再打开对话框，根属性仍是用户改后的值 | 不出现「改了又变回初值」 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_export_dialog.rs` 同款）。

use std::rc::Rc;

use slint::ComponentHandle as _;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::phase_align_gui;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::demo_project;
use yeban_model::{ClipPlacement, EntityId, YebanProjectV1};
use yeban_ui_test_port::port::{Permission, PointerButton};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Size, UiTestPort as _};

const NOW_MS: u64 = 1_760_000_000_000;
const LAG_SAMPLES: i32 = 2_400;

/// 从 `source` 轨复制全部摆放与设备链到 `target`，只加精确采样平移。
fn copy_placements_with_offset(
    project: &mut YebanProjectV1,
    source: EntityId,
    target: EntityId,
    offset_samples: i32,
) {
    let (placements, devices) = {
        let source_track = &project.tracks[&source];
        (
            source_track.clips.values().cloned().collect::<Vec<_>>(),
            source_track.devices.clone(),
        )
    };
    let target_track = project.tracks.get_mut(&target).expect("目标轨必须在工程里");
    target_track.clips.clear();
    for placement in placements {
        let id = EntityId::new();
        target_track.clips.insert(
            id,
            ClipPlacement {
                id,
                offset_samples: Some(offset_samples),
                ..placement
            },
        );
    }
    target_track.devices = devices;
}

fn track_id_by_name(project: &YebanProjectV1, name: &str) -> EntityId {
    project
        .tracks
        .values()
        .find(|track| track.name == name)
        .unwrap_or_else(|| panic!("演示工程没有 {name} 轨"))
        .id
}

/// 语义 ID 中心真实按下 + 释放（命中测试，⛔ 不直接调回调）。
fn click_center(port: &mut LivePort<MainWindow>, element_id: &str) {
    let bounds = port
        .tree()
        .find_by_id(element_id)
        .unwrap_or_else(|| panic!("控件树里必须有语义 ID `{element_id}`"))
        .bounds
        .unwrap_or_else(|| panic!("元素 `{element_id}` 必须有几何包围盒"));
    port.dispatch_pointer_down(
        element_id,
        f64::from(bounds.width) / 2.0,
        f64::from(bounds.height) / 2.0,
        PointerButton::Left,
    )
    .unwrap_or_else(|error| panic!("`{element_id}` 必须可按下：{error:?}"));
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");
    port.refresh_tree(None).expect("点击后必须能重抓树");
}

/// 点击输入框后逐字符键入（平台按键事件，⛔ 不直接写根属性）。
fn type_chars(port: &mut LivePort<MainWindow>, element_id: &str, text: &str) {
    click_center(port, element_id);
    for ch in text.chars() {
        port.window().type_char(ch);
    }
    port.refresh_tree(None).expect("键入后必须能重抓树");
}

#[test]
fn phase_align_dialog_inputs_write_back_and_drive_measurement() {
    // 两轨同源、目标轨晚 LAG_SAMPLES：测量值可精确断言（渲染侧 `phase_align_auto` 同款夹具）。
    let mut project = demo_project();
    let drums = track_id_by_name(&project, "鼓");
    let lead = track_id_by_name(&project, "主音");
    copy_placements_with_offset(&mut project, drums, lead, LAG_SAMPLES);
    // 轨名改成 ASCII，键入不依赖 IME（判据只看数值，不看名字）。
    project.tracks.get_mut(&drums).expect("鼓轨").name = "Ref".to_owned();
    project.tracks.get_mut(&lead).expect("主音轨").name = "Main".to_owned();

    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut port = LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 主窗口");
    host::apply_view(port.ui(), &view, 1280.0, 0.0);

    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<t571-phase-align>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let ui = port.ui().clone_strong();
    phase_align_gui::wire_phase_align_dialog(&ui, Some(Rc::clone(&undo)), None);

    ui.invoke_open_phase_align();
    assert!(ui.get_phase_align_open(), "打开回调必须真的打开对话框");
    // 打开时宿主会按选中轨播种 reference/target；本判据从空输入开始，
    // 清空的是**根属性**（单向/双向两种实现都会同步到本地显示），随后只用键鼠改写。
    ui.set_phase_align_reference("".into());
    ui.set_phase_align_target("".into());
    ui.set_phase_align_max_lag_ms("".into());
    ui.set_phase_align_apply(true);
    port.refresh_tree(None).expect("打开后必须能重抓树");

    // ① 基准轨 / 目标轨：键入即回写根属性（旧实现仍为空串）。
    type_chars(&mut port, "phase-align-reference-input", "Ref");
    assert_eq!(
        ui.get_phase_align_reference().as_str(),
        "Ref",
        "基准轨输入必须实时写回根属性"
    );
    type_chars(&mut port, "phase-align-target-input", "Main");
    assert_eq!(
        ui.get_phase_align_target().as_str(),
        "Main",
        "目标轨输入必须实时写回根属性"
    );

    // ② maxLagMs：非数字必须报原文错误（既有语义不因绑定方向改变而丢）。
    type_chars(&mut port, "phase-align-max-lag-input", "abc");
    assert_eq!(
        ui.get_phase_align_max_lag_ms().as_str(),
        "abc",
        "maxLagMs 输入必须实时写回根属性"
    );
    click_center(&mut port, "phase-align-run-button");
    let error = ui.get_phase_align_status().to_string();
    assert!(
        error.contains("参数 `maxLagMs` 类型非法, 期望 `number`"),
        "非数字 maxLagMs 必须报原文错误，实际状态: {error}"
    );

    // ③ maxLagMs：键入 75 ms（=3600 采样，覆盖 2400 采样的 lag；且未越过 ±4800 门）。
    ui.set_phase_align_max_lag_ms("".into());
    type_chars(&mut port, "phase-align-max-lag-input", "75");
    assert_eq!(
        ui.get_phase_align_max_lag_ms().as_str(),
        "75",
        "maxLagMs 输入必须实时写回根属性"
    );

    // ④ 关掉 apply ⇒ 只测量，工程不变。
    let before: Vec<i32> = [drums, lead]
        .iter()
        .map(|id| project.tracks[id].delay_samples.unwrap_or(0))
        .collect();
    click_center(&mut port, "phase-align-apply-checkbox");
    assert!(
        !ui.get_phase_align_apply(),
        "点击 apply 复选框必须把根属性翻成 false"
    );
    click_center(&mut port, "phase-align-run-button");
    let status = ui.get_phase_align_status().to_string();
    assert!(
        status.contains("只测量（未改工程）") && status.contains("lag 2400 采样"),
        "apply 关时必须按键入的 maxLagMs 测出 2400 且不提交，实际状态: {status}"
    );
    let measured = undo.try_project().expect("撤销端口里有工程");
    for id in [drums, lead] {
        assert_eq!(
            measured.tracks[&id].delay_samples.unwrap_or(0),
            before[if id == drums { 0 } else { 1 }],
            "apply 关时 `{}` 的延迟不得变化",
            measured.tracks[&id].name
        );
    }

    // ⑤ 再打开 apply ⇒ 测量并提交补偿；提交后延迟真的变化。
    click_center(&mut port, "phase-align-apply-checkbox");
    assert!(
        ui.get_phase_align_apply(),
        "再点 apply 必须把根属性翻回 true"
    );
    click_center(&mut port, "phase-align-run-button");
    let status = ui.get_phase_align_status().to_string();
    assert!(
        status.contains("已测量并补偿") && status.contains("提交"),
        "apply 开时必须测量并提交补偿，实际状态: {status}"
    );
    let applied = undo.try_project().expect("撤销端口里有工程");
    let after_ref = applied.tracks[&drums].delay_samples.unwrap_or(0);
    let after_main = applied.tracks[&lead].delay_samples.unwrap_or(0);
    assert!(
        after_ref != before[0] || after_main != before[1],
        "提交补偿后至少一条轨的 delay_samples 必须变化: before={before:?} after=({after_ref}, {after_main})"
    );

    // ⑥ 关掉再打开：用户改后的值必须还在根属性里（旧实现会回到初值）。
    ui.set_phase_align_open(false);
    ui.set_phase_align_open(true);
    assert_eq!(
        ui.get_phase_align_reference().as_str(),
        "Ref",
        "重开后基准轨不得回到初值"
    );
    assert_eq!(
        ui.get_phase_align_target().as_str(),
        "Main",
        "重开后目标轨不得回到初值"
    );
    assert_eq!(
        ui.get_phase_align_max_lag_ms().as_str(),
        "75",
        "重开后 maxLagMs 不得回到初值"
    );
    assert!(ui.get_phase_align_apply(), "重开后 apply 不得回到初值");
}
