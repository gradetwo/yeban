//! `[ARCH-RT-001 / MODEL-AST-004]` 侧链、动态控制与瞬态塑形器扩展 (G-S1, G-D4, G-D5, G-D6)
//! 验证：
//! 1. 侧链循环切换与断开生成合法原子 Op 批次；
//! 2. 压缩器比率循环切换与设备插入生成合法 Op 批次；
//! 3. 限制器与瞬态塑形器读数正常生成；
//! 4. Slint 静态属性计数棘轮与设计代币约束无衰退。

#![allow(unused)]
use yeban_app::bridge::{
    BuiltinEffect, ViewState, compressor_cycle_ratio_ops, project_compressor_info,
    project_limiter_info, project_shaper_info, project_sidechain_info, sidechain_cycle_ops,
};
use yeban_model::EntityId;
use yeban_model::YebanProjectV1;
use yeban_model::samples::filled_project;

const DEVICE_RACK: &str = include_str!("../ui/console/device_rack.slint");
const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const APP: &str = include_str!("../ui/app.slint");

use yeban_model::{Op, RoutingKind};

/// filled 工程里第一条非主总线轨（与 `bridge.rs` 的 `sidechain_and_dynamics_projection_and_ops`
/// 单元判据同一口径）。
fn first_display_track(project: &YebanProjectV1) -> EntityId {
    *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("样本工程必须有非主总线轨")
}

/// 1. 侧链循环切换与断开生成合法原子 Op 批次。
#[test]
fn sidechain_cycling_and_connecting_creates_valid_ops() {
    let mut project = filled_project();
    let track = first_display_track(&project);
    let candidates: Vec<EntityId> = project
        .tracks
        .keys()
        .copied()
        .filter(|&id| id != track && id != project.master_bus_track_id)
        .collect();
    assert!(!candidates.is_empty(), "循环切换需要至少一个候选键源");

    // 初始无侧链。
    let (label, avail) = project_sidechain_info(&project, project.tracks.get(&track));
    assert_eq!(label, "侧链键源: 无 (点击切换)");
    assert!(avail);

    // 第一轮：连上第一个候选（恰好一条 ConnectRouting；Op 全部合法可应用）。
    let ops = sidechain_cycle_ops(&project, track);
    assert!(!ops.is_empty());
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Op::ConnectRouting { .. }))
            .count(),
        1,
        "第一轮必须恰好连一条侧链边"
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(op, Op::DisconnectRouting { .. })),
        "初始无边时不该有断开"
    );
    for op in &ops {
        op.apply(&mut project).expect("侧链 Op 必须能应用");
    }
    let edge = project
        .routing_graph
        .edges
        .values()
        .find(|edge| edge.kind == RoutingKind::Sidechain && edge.destination_node == track)
        .expect("连上之后必须有一条入边");
    assert_eq!(edge.source_node, candidates[0]);
    let (label, _) = project_sidechain_info(&project, project.tracks.get(&track));
    assert!(label.contains("侧链键源: 轨"), "连上之后读数必须点名键源轨");
    assert!(label.contains("(点击切换)"));

    // 完整循环：再切 (候选数) 次回到无（每个选择恰好出现一次）。
    for _ in 0..candidates.len() {
        let ops = sidechain_cycle_ops(&project, track);
        assert!(!ops.is_empty());
        for op in &ops {
            op.apply(&mut project).expect("侧链 Op 必须能应用");
        }
    }
    let (label, avail) = project_sidechain_info(&project, project.tracks.get(&track));
    assert_eq!(label, "侧链键源: 无 (点击切换)");
    assert!(avail);
    assert!(
        !project
            .routing_graph
            .edges
            .values()
            .any(|edge| { edge.kind == RoutingKind::Sidechain && edge.destination_node == track }),
        "完整一轮之后入边必须断干净"
    );
}

/// 2. 压缩器比率循环切换与设备插入生成合法 Op 批次；动态读数正常生成。
#[test]
fn compressor_cycling_and_dynamics_readout() {
    let mut project = filled_project();
    let track = first_display_track(&project);

    // 比率循环：[2, 4, 8, 20]，四轮回到起点（若该轨本来就没有比率参数，
    // 则第一轮是插入压缩器，插完再循环同样四轮回起点）。
    let mut ops = compressor_cycle_ratio_ops(&project, track);
    assert!(!ops.is_empty());
    if ops.iter().any(|op| matches!(op, Op::InsertDevice { .. })) {
        for op in &ops {
            op.apply(&mut project).expect("插入压缩器必须成功");
        }
        ops = compressor_cycle_ratio_ops(&project, track);
        assert!(!ops.is_empty());
    }
    let first = ops
        .iter()
        .find_map(|op| match op {
            Op::SetParam { new_val, .. } => Some(*new_val),
            _ => None,
        })
        .expect("比率循环必须是一条 SetParam");
    assert!(
        [2.0f32, 4.0, 8.0, 20.0].contains(&first),
        "下一比率必须在 [2, 4, 8, 20] 里，实得 {first}"
    );
    // 先落到表值上再记起点：插入的缺省比率未必在表里，四轮循环只保证「表值 → 表值」。
    for op in &ops {
        op.apply(&mut project).expect("比率 Op 必须能应用");
    }
    let start_ratio = project
        .tracks
        .get(&track)
        .expect("轨")
        .devices
        .iter()
        .flat_map(|d| d.params.iter())
        .find(|p| p.name == "ratio")
        .expect("循环前轨上必须已有 ratio 参数")
        .value;
    for _ in 0..4 {
        let ops = compressor_cycle_ratio_ops(&project, track);
        assert_eq!(ops.len(), 1, "比率循环每轮恰好一条 SetParam");
        for op in &ops {
            op.apply(&mut project).expect("比率 Op 必须能应用");
        }
    }
    let end_ratio = project
        .tracks
        .get(&track)
        .expect("轨")
        .devices
        .iter()
        .flat_map(|d| d.params.iter())
        .find(|p| p.name == "ratio")
        .expect("ratio 参数")
        .value;
    assert!(
        (end_ratio - start_ratio).abs() < 0.01,
        "四轮比率循环必须回到起点（{start_ratio} → {end_ratio}）"
    );

    // 三路动态读数：压缩器 / 限制器 / 瞬态塑形器。
    let (comp_info, comp_avail) = project_compressor_info(&project, project.tracks.get(&track));
    assert!(comp_info.contains("压缩器:"), "压缩器读数：{comp_info}");
    assert!(comp_avail);
    let (lim_info, lim_avail) = project_limiter_info(&project, project.tracks.get(&track));
    assert!(lim_info.contains("限制器:"), "限制器读数：{lim_info}");
    assert!(lim_avail);
    let (shaper_info, shaper_avail) = project_shaper_info(&project, project.tracks.get(&track));
    assert!(
        shaper_info.contains("瞬态塑形:"),
        "塑形器读数：{shaper_info}"
    );
    assert!(shaper_avail);

    // 无轨 / 主总线两档空态。
    assert_eq!(
        project_sidechain_info(&project, None),
        ("侧链键源: 无可显示的音轨".to_owned(), false)
    );
    assert_eq!(
        project_sidechain_info(&project, project.tracks.get(&project.master_bus_track_id)),
        ("侧链键源: 主总线不可侧链".to_owned(), false)
    );
    assert_eq!(
        project_compressor_info(&project, None),
        ("压缩器: 无可显示的音轨".to_owned(), false)
    );
    assert_eq!(
        project_limiter_info(&project, None),
        ("限制器: 无可显示的音轨".to_owned(), false)
    );
    assert_eq!(
        project_shaper_info(&project, None),
        ("瞬态塑形: 无可显示的音轨".to_owned(), false)
    );
}

/// 3. 限制器与瞬态塑形器读数正常生成；内置压缩器契约（分组 / 显示名 / 参数）。
#[test]
fn builtin_effect_compressor_contract() {
    assert_eq!(BuiltinEffect::Eq.group_id(), "eq");
    assert_eq!(BuiltinEffect::Eq.display_name(), "EQ");
    assert_eq!(BuiltinEffect::Reverb.group_id(), "reverb");
    assert_eq!(BuiltinEffect::Reverb.display_name(), "Reverb");
    assert_eq!(BuiltinEffect::Compressor.group_id(), "dynamics");
    assert_eq!(BuiltinEffect::Compressor.display_name(), "压缩器");

    let device = yeban_app::bridge::builtin_effect_device(BuiltinEffect::Compressor)
        .expect("dynamics 分组必须在效果器契约里");
    assert_eq!(device.name, "压缩器");
    assert!(
        device.params.iter().any(|p| p.name == "ratio"),
        "内置压缩器必须带 ratio 参数"
    );
    assert!(
        device
            .params
            .iter()
            .any(|p| p.name == "threshold_db" || p.name == "threshold"),
        "内置压缩器必须带阈值参数"
    );

    // 没有压缩器的轨：第一轮是插入内置压缩器（不是空手而归）。
    let mut project = filled_project();
    let track = first_display_track(&project);
    project.tracks.get_mut(&track).expect("轨").devices.clear();
    let ops = compressor_cycle_ratio_ops(&project, track);
    assert_eq!(ops.len(), 1, "无压缩器的轨第一轮恰好一条插入 Op");
    assert!(
        matches!(ops[0], Op::InsertDevice { .. }),
        "无压缩器的轨第一轮必须是 InsertDevice"
    );
    for op in &ops {
        op.apply(&mut project).expect("插入必须成功");
    }
    assert!(
        project
            .tracks
            .get(&track)
            .expect("轨")
            .devices
            .iter()
            .flat_map(|d| d.params.iter())
            .any(|p| p.name == "ratio"),
        "插入后轨上必须有 ratio 参数"
    );
}

/// 4. Slint 静态属性计数棘轮与设计代币约束无衰退。
#[test]
fn slint_ratchets_are_strictly_preserved() {
    // 四路读数的默认值逐字钉死（界面未投影时的空态文案）。
    for (property, literal) in [
        ("sidechain-key-label", "侧链键源: 无 (点击切换)"),
        (
            "compressor-info",
            "压缩器: 阈值 -18.0 dB · 比率 4:1 · GR 0.0 dB",
        ),
        ("limiter-info", "限制器: 阈值 -0.1 dBTP · 状态 正常"),
        ("shaper-info", "瞬态塑形: 启动 +0.0 · 维持 +0.0 (未激活)"),
    ] {
        assert!(
            DEVICE_RACK.contains(&format!(
                "in property <string> {property}: @tr(\"{literal}\");"
            )),
            "device_rack 里 {property} 的默认值必须逐字是 {literal}"
        );
    }
    for property in [
        "sidechain-available",
        "compressor-available",
        "limiter-available",
        "shaper-available",
    ] {
        assert!(
            DEVICE_RACK.contains(&format!("in property <bool> {property}: false;")),
            "device_rack 里 {property} 的默认值必须是 false"
        );
    }
    // 两条循环回调各声明一次、各转发一次；四路读数各绑一次文本。
    for callback in [
        "callback sidechain-cycle();",
        "callback compressor-cycle-ratio();",
    ] {
        assert_eq!(
            DEVICE_RACK.matches(callback).count(),
            1,
            "device_rack 里 {callback} 必须恰好一次"
        );
    }
    for call in ["root.sidechain-cycle();", "root.compressor-cycle-ratio();"] {
        assert_eq!(
            DEVICE_RACK.matches(call).count(),
            1,
            "device_rack 里 {call} 必须恰好一次"
        );
    }
    for property in [
        "sidechain-key-label",
        "compressor-info",
        "limiter-info",
        "shaper-info",
    ] {
        assert!(
            DEVICE_RACK.contains(&format!("text: root.{property};")),
            "device_rack 里 {property} 必须绑到一行文本上"
        );
    }
    // 宿主接线：两条回调各注册一次；apply_view 把四路读数写进窗口。
    let host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert_eq!(
        host.matches("on_sidechain_cycle").count(),
        1,
        "宿主必须注册 on_sidechain_cycle 一次"
    );
    assert_eq!(
        host.matches("on_compressor_cycle_ratio").count(),
        1,
        "宿主必须注册 on_compressor_cycle_ratio 一次"
    );
    for setter in [
        "set_sidechain_key_label",
        "set_compressor_info",
        "set_limiter_info",
        "set_shaper_info",
    ] {
        assert!(
            host.contains(setter),
            "宿主 apply_view 必须把 {setter} 写进窗口"
        );
    }
}

/// 5. ViewState 含动态与侧链属性，且与桥接函数对同一条展示轨算出同一份读数。
#[test]
fn viewstate_contains_dynamics_and_sidechain_properties() {
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("样本工程必须能投影");
    // 展示轨 = 四路读数与 ViewState 逐字对得上的那条非主总线轨。
    let display = project
        .tracks
        .values()
        .filter(|t| t.id != project.master_bus_track_id)
        .find(|t| {
            let (label, avail) = project_sidechain_info(&project, Some(t));
            label == view.sidechain_key_label && avail == view.sidechain_available
        })
        .expect("ViewState 的侧链读数必须来自某一条真实展示轨");
    let (comp_info, comp_avail) = project_compressor_info(&project, Some(display));
    assert_eq!(view.compressor_info, comp_info);
    assert_eq!(view.compressor_available, comp_avail);
    let (lim_info, lim_avail) = project_limiter_info(&project, Some(display));
    assert_eq!(view.limiter_info, lim_info);
    assert_eq!(view.limiter_available, lim_avail);
    let (shaper_info, shaper_avail) = project_shaper_info(&project, Some(display));
    assert_eq!(view.shaper_info, shaper_info);
    assert_eq!(view.shaper_available, shaper_avail);
}
