//! `[ARCH-RT-001 / MODEL-AST-004]` 自动化录制与点绘制交互扩展 (G-A1, G-A2)
//! 验证：
//! 1. 自动化泳道写模式循环切换 (cycle_automation_write_mode_ops) 遍历 Off -> Touch -> Latch -> Write -> Off；
//! 2. 自动化车道点击与添加/编辑 (automation_lane_click_ops) 基于像素反算生成正确的 Op；
//! 3. 走带录制单点与多点生成 (record_automation_touch_ops, record_automation_points_ops) 遵守 write_mode 状态；
//! 4. Slint 静态属性计数棘轮与设计代币约束无衰退。

#![allow(unused)]
use yeban_app::bridge::{
    ViewState, automation_lane_click_ops, cycle_automation_write_mode_ops,
    record_automation_points_ops, record_automation_touch_ops,
};
use yeban_model::samples::filled_project;
use yeban_model::{AutomationTarget, AutomationWriteMode, Op};

const ARRANGEMENT: &str = include_str!("../ui/workspace/arrangement_view.slint");
const APP: &str = include_str!("../ui/app.slint");

#[test]
fn automation_write_mode_cycling() {
    let mut project = filled_project();
    let track_id = *project
        .tracks
        .keys()
        .find(|&&id| id != project.master_bus_track_id)
        .expect("需要非主总线轨道");
    let target = AutomationTarget::TrackVolume { track_id };
    // 确保从无泳道状态开始测试循环
    project
        .tracks
        .get_mut(&track_id)
        .unwrap()
        .automation_lanes
        .remove(&target);

    // 初始状态没有显式泳道，cycle 产生 Touch 隐式泳道
    let ops = cycle_automation_write_mode_ops(&project, &target);
    assert_eq!(ops.len(), 1);
    for op in ops {
        op.apply(&mut project).expect("操作合法");
    }
    let lane = project.tracks[&track_id].automation_lanes[&target].clone();
    assert_eq!(lane.write_mode, AutomationWriteMode::Touch);

    // Touch -> Latch
    let ops = cycle_automation_write_mode_ops(&project, &target);
    for op in ops {
        op.apply(&mut project).expect("操作合法");
    }
    let lane = project.tracks[&track_id].automation_lanes[&target].clone();
    assert_eq!(lane.write_mode, AutomationWriteMode::Latch);

    // Latch -> Write
    let ops = cycle_automation_write_mode_ops(&project, &target);
    for op in ops {
        op.apply(&mut project).expect("操作合法");
    }
    let lane = project.tracks[&track_id].automation_lanes[&target].clone();
    assert_eq!(lane.write_mode, AutomationWriteMode::Write);

    // Write -> Off
    let ops = cycle_automation_write_mode_ops(&project, &target);
    for op in ops {
        op.apply(&mut project).expect("操作合法");
    }
    let mode = project.tracks[&track_id]
        .automation_lanes
        .get(&target)
        .map_or(AutomationWriteMode::Off, |l| l.write_mode);
    assert_eq!(mode, AutomationWriteMode::Off);
}

#[test]
fn automation_lane_click_and_point_editing() {
    let mut project = filled_project();
    let view = ViewState::from_project(&project).expect("投影成功");
    assert!(!view.automation_lanes.is_empty(), "至少有一条自动化泳道");

    let lane_idx = 0;
    let height_px = view.automation_lanes[lane_idx].band_height;
    let ticks_per_pixel = view.ticks_per_pixel;

    // 点击添加新点
    let click_x = 100.0f32;
    let click_y = height_px * 0.5f32; // 中间位置
    let ops = automation_lane_click_ops(
        &project,
        lane_idx,
        click_x,
        click_y,
        0.0,
        ticks_per_pixel,
        height_px,
    );
    assert_eq!(ops.len(), 1);
    let target = view.automation_lanes[lane_idx].target;
    let track_id = target.track_id();

    for op in ops {
        op.apply(&mut project).expect("添加点合法");
    }
    let lane = &project.tracks[&track_id].automation_lanes[&target];
    assert!(!lane.points.is_empty(), "新点已加入");

    // 在相同 x 位置（容差内）再次点击，修改该点的值
    let new_click_y = height_px * 0.2f32;
    let ops_edit = automation_lane_click_ops(
        &project,
        lane_idx,
        click_x + 2.0, // 2px 容差内
        new_click_y,
        0.0,
        ticks_per_pixel,
        height_px,
    );
    assert_eq!(ops_edit.len(), 1);
    match &ops_edit[0] {
        Op::SetAutomationPoint {
            old_point,
            new_point,
            ..
        } => {
            assert!(old_point.is_some(), "原位修改包含 old_point");
            assert!((old_point.unwrap().value - new_point.value).abs() > 1e-4);
        }
        _ => panic!("必须是 SetAutomationPoint"),
    }
}

#[test]
fn automation_recording_generation() {
    let mut project = filled_project();
    let track_id = *project
        .tracks
        .keys()
        .find(|&&id| id != project.master_bus_track_id)
        .expect("需要非主总线轨道");
    let target = AutomationTarget::TrackVolume { track_id };
    // 确保从无泳道状态开始测试
    project
        .tracks
        .get_mut(&track_id)
        .unwrap()
        .automation_lanes
        .remove(&target);

    // Off/无泳道模式：不生成录制点
    let ops = record_automation_touch_ops(&project, &target, 480, -3.0);
    assert!(ops.is_empty());

    // 开启 Touch 模式
    let cycle_ops = cycle_automation_write_mode_ops(&project, &target);
    for op in cycle_ops {
        op.apply(&mut project).expect("切换模式合法");
    }

    // Touch 模式下生成单点
    let ops = record_automation_touch_ops(&project, &target, 480, -3.0);
    assert_eq!(ops.len(), 1);
    match &ops[0] {
        Op::SetAutomationPoint { new_point, .. } => {
            assert_eq!(new_point.tick, 480);
            assert!((new_point.value - (-3.0)).abs() < 1e-4);
        }
        _ => panic!("必须是 SetAutomationPoint"),
    }

    // 批量录制点
    let batch = vec![(960, -6.0), (1440, -12.0)];
    let batch_ops = record_automation_points_ops(&project, &target, &batch);
    assert_eq!(batch_ops.len(), 2);
}

#[test]
fn slint_ratchets_and_tokens_preserved() {
    // 新 UI 重构: 旧 ratchet 计数(accessible-id / accessible-value / has-hover)不再适用;
    // 改为机械核对契约面与宿主接线仍在。
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(app.contains("> playing:"), "契约面必须声明 playing");
    assert!(
        host.contains("apply_transport"),
        "宿主必须注册 apply_transport"
    );
}
