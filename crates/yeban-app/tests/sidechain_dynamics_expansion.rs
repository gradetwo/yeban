//! `[ARCH-RT-001 / MODEL-AST-004]` 侧链、动态控制与瞬态塑形器扩展 (G-S1, G-D4, G-D5, G-D6)
//! 验证：
//! 1. 侧链循环切换与断开生成合法原子 Op 批次；
//! 2. 压缩器比率循环切换与设备插入生成合法 Op 批次；
//! 3. 限制器与瞬态塑形器读数正常生成；
//! 4. Slint 静态属性计数棘轮与设计代币约束无衰退。

use yeban_app::bridge::{
    BuiltinEffect, ViewState, compressor_cycle_ratio_ops, project_compressor_info,
    project_limiter_info, project_shaper_info, project_sidechain_info, sidechain_cycle_ops,
};
use yeban_model::EntityId;
use yeban_model::samples::filled_project;

const DEVICE_RACK: &str = include_str!("../ui/console/device_rack.slint");
const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const APP: &str = include_str!("../ui/app.slint");

#[test]
fn sidechain_cycling_and_connecting_creates_valid_ops() {
    let mut project = filled_project();
    let track_ids: Vec<EntityId> = project
        .tracks
        .keys()
        .copied()
        .filter(|&id| id != project.master_bus_track_id)
        .collect();
    assert!(track_ids.len() >= 2, "测试工程至少需要 2 条非主总线轨道");
    let track_a = track_ids[0];
    let _track_b = track_ids[1];

    // 初始状态：无侧链
    let (label, avail) = project_sidechain_info(&project, project.tracks.get(&track_a));
    assert_eq!(label, "侧链键源: 无 (点击切换)");
    assert!(avail);

    // 第一次循环：连接到一个候选轨 (track_b 或其他非 master 轨)
    let ops = sidechain_cycle_ops(&project, track_a);
    assert!(!ops.is_empty(), "循环应产出建边操作");
    for op in ops {
        op.apply(&mut project).expect("侧链边连接必须合法");
    }

    let (label_after, _) = project_sidechain_info(&project, project.tracks.get(&track_a));
    assert!(label_after.contains("侧链键源: 轨"));
    assert!(label_after.contains("(点击切换)"));

    // 再次循环遍历直到回到 None
    let mut current_label = label_after;
    for _ in 0..track_ids.len() + 1 {
        let ops = sidechain_cycle_ops(&project, track_a);
        for op in ops {
            op.apply(&mut project).expect("操作必须合法");
        }
        let (next_label, _) = project_sidechain_info(&project, project.tracks.get(&track_a));
        current_label = next_label;
        if current_label == "侧链键源: 无 (点击切换)" {
            break;
        }
    }
    assert_eq!(current_label, "侧链键源: 无 (点击切换)");
}

#[test]
fn compressor_cycling_and_dynamics_readout() {
    let mut project = filled_project();
    let track_ids: Vec<EntityId> = project
        .tracks
        .keys()
        .copied()
        .filter(|&id| id != project.master_bus_track_id)
        .collect();
    let track_a = track_ids[0];

    // 初始状态：尚无带 ratio 的压缩器设备，循环将自动插入一台内置压缩器
    let ops = compressor_cycle_ratio_ops(&project, track_a);
    assert!(!ops.is_empty(), "没有压缩器时应产出 InsertDevice 操作");
    for op in ops {
        op.apply(&mut project).expect("插入压缩器必须成功");
    }

    let (comp_info, comp_avail) = project_compressor_info(&project, project.tracks.get(&track_a));
    assert!(comp_info.contains("压缩器:"));
    assert!(comp_avail);

    // 有压缩器后，循环将修改 ratio 参数
    let ops2 = compressor_cycle_ratio_ops(&project, track_a);
    assert!(!ops2.is_empty());
    for op in ops2 {
        op.apply(&mut project).expect("更改 ratio 参数必须成功");
    }

    // 限制器与瞬态塑形器读数
    let (limiter_info, lim_avail) = project_limiter_info(&project, project.tracks.get(&track_a));
    assert!(limiter_info.contains("限制器: 阈值 -0.1 dBTP"));
    assert!(lim_avail);

    let (shaper_info, shaper_avail) = project_shaper_info(&project, project.tracks.get(&track_a));
    assert!(shaper_info.contains("瞬态塑形:"));
    assert!(shaper_avail);
}

#[test]
fn builtin_effect_compressor_contract() {
    assert_eq!(BuiltinEffect::Compressor.group_id(), "dynamics");
    assert_eq!(BuiltinEffect::Compressor.display_name(), "压缩器");
}

#[test]
fn slint_ratchets_are_strictly_preserved() {
    // 1. DeviceRack 静态计数棘轮
    assert_eq!(
        DEVICE_RACK.matches("accessible-id:").count(),
        63,
        "device_rack.slint 的 accessible-id 必须严格固定在 63 个"
    );
    assert_eq!(
        DEVICE_RACK.matches("accessible-value:").count(),
        3,
        "device_rack.slint 的 accessible-value 必须严格固定在 3 个"
    );
    assert_eq!(
        DEVICE_RACK
            .matches("mouse-cursor: MouseCursor.pointer;")
            .count(),
        20,
        "device_rack.slint 的 pointer 光标数必须严格固定在 20 个"
    );
    assert_eq!(
        DEVICE_RACK
            .matches("mouse-cursor: MouseCursor.grab;")
            .count(),
        2,
        "device_rack.slint 的 grab 光标数必须严格固定在 2 个"
    );
    assert_eq!(
        DEVICE_RACK.matches("enabled: root.").count(),
        11,
        "device_rack.slint 的 enabled: root. 必须严格固定在 11 个"
    );
    assert_eq!(
        DEVICE_RACK.matches("Tokens.accent").count(),
        0,
        "device_rack.slint 严禁使用 Tokens.accent"
    );

    // 2. ConsoleTabs 静态计数
    assert_eq!(
        CONSOLE_TABS.matches("accessible-id:").count(),
        3,
        "console_tabs.slint 的 accessible-id 保持 3 个"
    );
    assert_eq!(
        CONSOLE_TABS.matches("accessible-value:").count(),
        0,
        "console_tabs.slint 的 accessible-value 保持 0 个"
    );

    // 3. App 根组件计数
    assert_eq!(
        APP.matches("accessible-id:").count(),
        8,
        "app.slint 的 accessible-id 保持 8 个"
    );
    assert_eq!(
        APP.matches("accessible-value:").count(),
        3,
        "app.slint 的 accessible-value 保持 3 个"
    );
    assert_eq!(
        APP.matches("Tokens.accent").count(),
        0,
        "app.slint 严禁新增 Tokens.accent"
    );
}

#[test]
fn viewstate_contains_dynamics_and_sidechain_properties() {
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("工程投影必须成功");
    assert!(!view.sidechain_key_label().is_empty());
    assert!(view.sidechain_available());
    assert!(!view.compressor_info().is_empty());
    assert!(view.compressor_available());
    assert!(!view.limiter_info().is_empty());
    assert!(view.limiter_available());
    assert!(!view.shaper_info().is_empty());
    assert!(view.shaper_available());
}
