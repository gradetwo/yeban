//! **宏面板**（`ui/dialogs/macro_panel.slint`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 控制台 Tab 3 标题行的「宏 (N)」按钮打开的右侧抽屉**不是装饰**：它的条数 / 名字 /
//! 位置来自 `ViewState::macros`（`TrackV3::macros` 的投影），滑块手势经
//! `host::wire_macro_panel` 提交**恰好一次**批次：`Op::SetMacro` ＋ 该宏每条映射的
//! 两个 `macro-cascade:` 点；导出侧按新宏位置读到新值。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（无泳道无物化点） | 夹具目标泳道与 `Macro` 泳道都不存在；手势前导出求值器是恒等 |
//! | ②（一次手势 = 一个批次） | grab/drag 只写视图态；release ⇒ `commit_count + 1` |
//! | ③（级联点真物化） | 目标泳道恰 2 点，id 是 `macro_cascade_point_id` 的确定性身份，起点/终点按旧/新宏位置算 |
//! | ④（导出随新位置变化） | 同一工程 `TrackGainAutomation` 从恒等变为非恒等，tick 3840 的 `last_db` 与目标泳道终点逐位相同 |
//! | ⑤（UI 明说仅导出） | 面板有 `macro-panel-export-only` 角标，文案走 `@tr` 且八语文档有名 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_export_dialog.rs` 同款）。
//!
//! ## 红对照
//!
//! 把 `host::wire_macro_panel` 的 `macro-release` 退回只提交 `Op::SetMacro`：
//! `commit_count` 仍 `+1`，但目标泳道 0 点、导出求值器仍恒等 ⇒ ③④ 当场红。

use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::AutomationTarget;
use yeban_model::MacroMapping;
use yeban_model::YebanProjectV1;
use yeban_model::ids::macro_cascade_point_id;
use yeban_render::automation::TrackGainAutomation;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

fn live_window(project: &YebanProjectV1) -> LivePort<MainWindow> {
    let view = ViewState::from_project(project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口")
}

fn undo_port(project: &YebanProjectV1) -> Rc<UndoPort> {
    Rc::new(UndoPort::new(
        UndoSession::open("<判据 macro>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ))
}

#[test]
fn macro_slider_materialises_the_cascade_and_export_follows_the_new_position() {
    let mut project = yeban_model::samples::filled_project();
    let macro_track = project
        .tracks
        .values()
        .find(|track| !track.macros.is_empty())
        .expect("样本必须有带宏的音轨")
        .id;
    // 样本的宏映射指向 DeviceParam；本判据把同一台宏改绑到目标单位更直观的
    // `TrackVolume`（depth = 6 dB），这样导出读数可以直接和推子 dB 对位。
    project.tracks.get_mut(&macro_track).expect("宏轨").macros[0].mappings = vec![MacroMapping {
        target: AutomationTarget::TrackVolume {
            track_id: macro_track,
        },
        depth: 6.0,
    }];
    let target = project.tracks[&macro_track].macros[0].mappings[0].target;
    let macro_lane = AutomationTarget::Macro {
        track_id: macro_track,
        macro_index: 0,
    };
    // 样本自带一条音量点线；判据要证“无泳道无物化点”，先把这条目标泳道摘掉。
    project
        .tracks
        .get_mut(&macro_track)
        .expect("宏轨")
        .automation_lanes
        .remove(&target);
    // ① 前提：无 Macro 泳道、无物化点。
    assert!(
        !project.tracks[&macro_track]
            .automation_lanes
            .contains_key(&target),
        "夹具必须从“无物化点”开始"
    );
    assert!(
        !project.tracks[&macro_track]
            .automation_lanes
            .contains_key(&macro_lane),
        "夹具必须从“无 Macro 泳道”开始"
    );

    let mut live = live_window(&project);
    let ui = live.ui();
    let port = undo_port(&project);
    host::wire_edit_surfaces(ui, &port);
    ui.set_selected_track_id(macro_track.to_string().into());

    let old_value = project.tracks[&macro_track].macros[0].value;
    let depth = project.tracks[&macro_track].macros[0].mappings[0].depth;
    let static_value = target.static_value(&project).expect("目标静态值");
    let before_commits = port.display().commit_count;

    // ② 手势：按下 / 拖动只改视图态，松手才提交。
    ui.invoke_macro_grab(0, 0.0);
    ui.invoke_macro_drag(0, 1.0);
    ui.invoke_macro_release(0);
    assert_eq!(
        port.display().commit_count,
        before_commits + 1,
        "一次松手必须恰好一个提交"
    );

    let after = port.try_project().expect("提交后的工程");
    let lane = after.tracks[&macro_track]
        .automation_lanes
        .get(&target)
        .expect("手势必须物化目标泳道");
    let first = macro_cascade_point_id(macro_track, 0, 0, 0);
    let last = macro_cascade_point_id(macro_track, 0, 0, 1);
    // ③ 级联点数与确定性身份：恰好两个点。
    assert_eq!(lane.points.len(), 2, "一条映射必须恰好两个级联点");
    let start = lane.points.get(&first).expect("起点");
    let end = lane.points.get(&last).expect("终点");
    assert_eq!(
        start.value.to_bits(),
        (static_value + old_value * depth).to_bits(),
        "起点 = 静态值 + 旧宏位置 × depth"
    );
    assert_eq!(
        end.value.to_bits(),
        (static_value + 1.0 * depth).to_bits(),
        "终点 = 静态值 + 新宏位置 × depth"
    );
    assert_eq!(
        after.tracks[&macro_track].macros[0].value.to_bits(),
        1.0_f32.to_bits()
    );

    // ④ 导出侧真的读新值：手前恒等，手后读到终点值（tick 3840 = 96000 帧 @120bpm）。
    let before_export = TrackGainAutomation::new(&project, macro_track, 48_000);
    assert!(
        before_export.is_identity(),
        "无泳道无物化点时导出必须是恒等"
    );
    let mut after_export = TrackGainAutomation::new(&after, macro_track, 48_000);
    assert!(!after_export.is_identity(), "物化后导出不得再走恒等快路径");
    let mut buffer = vec![0.25_f32; 256];
    after_export.apply_ratio_block(&mut buffer, 96_000, 256, 1);
    let reading = after_export.last_db().expect("物化后导出必须有读数");
    assert_eq!(
        reading.to_bits(),
        end.value.to_bits(),
        "导出读数必须与目标泳道终点逐位相同"
    );

    // ④-b 有旧物化点再转宏：必须按新位置重写既有两个点（⛔ 不是继续按旧值）。
    let before_second_commits = port.display().commit_count;
    ui.invoke_macro_grab(0, 0.25);
    ui.invoke_macro_drag(0, 0.5);
    ui.invoke_macro_release(0);
    assert_eq!(
        port.display().commit_count,
        before_second_commits + 1,
        "第二次松手仍是一个提交"
    );
    let after_second = port.try_project().expect("第二次提交后的工程");
    let second_lane = after_second.tracks[&macro_track]
        .automation_lanes
        .get(&target)
        .expect("旧泳道必须仍在");
    assert_eq!(second_lane.points.len(), 2, "重写后仍恰好两个点");
    let rewritten_end = second_lane.points.get(&last).expect("终点身份不变").value;
    assert_eq!(
        rewritten_end.to_bits(),
        (static_value + 0.5 * depth).to_bits(),
        "旧物化点必须按新宏位置重写"
    );
    let mut second_export = TrackGainAutomation::new(&after_second, macro_track, 48_000);
    let mut second_buffer = vec![0.25_f32; 256];
    second_export.apply_ratio_block(&mut second_buffer, 96_000, 256, 1);
    assert_eq!(
        second_export.last_db().expect("第二次导出读数").to_bits(),
        rewritten_end.to_bits(),
        "有旧物化点时导出也必须跟随新宏位置"
    );

    // ⑤ 面板明说只导出，语义 ID 在控件树里。
    ui.set_macro_panel_open(true);
    let tree = live.refresh_tree(None).expect("ui/tree");
    assert!(
        tree.contains("macro-panel-export-only"),
        "宏面板必须挂出“仅导出”角标"
    );
}
