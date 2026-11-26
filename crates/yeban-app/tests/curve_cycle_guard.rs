//! T-FIX-715 装配判据（源码文本级）：曲线切换入口与 ops 不变量。
//!
//! 本机按 AGENTS.md §5 不跑 Slint 重依赖构建，故以源码文本判据核对接线与
//! 「只改 curve、tick/value 不变、可撤销、新建点仍 Linear」。像素/GUI 实测交 CI。

const BRIDGE: &str = include_str!("../src/bridge.rs");
const ARR: &str = include_str!("../ui/arrangement.slint");
const APP: &str = include_str!("../ui/app.slint");
const HOST: &str = include_str!("../src/host.rs");

fn cycle_fn() -> &'static str {
    let start = BRIDGE
        .find("pub fn cycle_automation_curve_ops(")
        .expect("bridge 必须有曲线切换 op 生成器");
    let tail = &BRIDGE[start..];
    let end = tail.find("/// 将录制的一批").unwrap_or(tail.len());
    &tail[..end]
}

#[test]
fn ops_only_change_curve_and_carry_old_point_for_undo() {
    let f = cycle_fn();
    assert!(
        f.contains("curve: next_curve"),
        "新点必须写切换后的曲线: {f}"
    );
    assert!(f.contains("tick: existing.tick"), "tick 必须不变: {f}");
    assert!(f.contains("value: existing.value"), "value 必须不变: {f}");
    assert!(
        f.contains("old_point: Some(*existing)"),
        "必须带 old_point，撤销才能回到原曲线: {f}"
    );
    for curve in [
        "CurveType::Linear =>",
        "CurveType::Exponential =>",
        "CurveType::Logarithmic =>",
        "CurveType::SCurve =>",
    ] {
        assert!(f.contains(curve), "四态循环缺少分支 {curve}: {f}");
    }
}

#[test]
fn new_points_stay_linear() {
    // 新建点两处（点击新增 + 录制新增）仍恒 Linear，未被曲线切换污染。
    let count = BRIDGE
        .matches("curve: yeban_model::CurveType::Linear,")
        .count();
    assert!(
        count >= 2,
        "新建点缺省仍须 Linear（点击 + 录制两处），实测 {count} 处"
    );
}

#[test]
fn ui_entry_and_host_wiring_present() {
    assert!(
        ARR.contains("callback automation-lane-cycle-curve(length, length);"),
        "编曲视图缺少曲线切换回调"
    );
    assert!(ARR.contains("event.modifiers.control"), "缺少修饰键判定");
    assert!(
        ARR.contains("root.automation-lane-cycle-curve("),
        "修饰键单击未接入曲线切换"
    );
    assert!(
        APP.contains("callback automation-lane-cycle-curve(length, length);")
            && APP.contains("automation-lane-cycle-curve(x, y) =>"),
        "app.slint 未转发曲线切换回调"
    );
    assert!(
        HOST.contains("pub fn wire_automation_lane_cycle_curve(")
            && HOST.contains("wire_automation_lane_cycle_curve(ui, port);"),
        "宿主未接线曲线切换回调"
    );
    assert!(
        HOST.contains("cycle automation point curve"),
        "提交说明缺失"
    );
}
