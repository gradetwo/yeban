//! **BUS COMP 卡片画的是真实压缩传递曲线**（ADR-0076 B2）。
//!
//! 现状（修前）：`left_panel.slint` 的 BUS COMP 只有 0dB 基准线 + 文案「示意」，
//! 与压缩器参数无关 —— 调阈值/比率时曲线区没有任何反馈。
//!
//! 修法：`bridge::calculate_comp_curve_svg(threshold_db, ratio)` 按硬拐点压缩公式
//! `out = in <= t ? in : t + (in - t)/ratio` 画 43 点曲线；参数来自该轨压缩器设备的
//! `threshold_db`/`ratio`（与 `project_compressor_info` 同一份设备）；无压缩器 ⇒ 空串，
//! 卡片保持「示意」标注（诚实，不画常量曲线）。
//!
//! ⚠ 纯 `include_str!` 判据（本机编不了 Slint 重依赖）。

const BRIDGE: &str = include_str!("../src/bridge.rs");
const HOST: &str = include_str!("../src/host.rs");
const APP: &str = include_str!("../ui/app.slint");
const LEFT: &str = include_str!("../ui/left_panel.slint");

#[test]
fn bus_comp_curve_is_computed_from_real_params() {
    assert!(
        BRIDGE.contains("pub fn calculate_comp_curve_svg(threshold_db: f32, ratio: f32) -> String"),
        "必须有按 threshold/ratio 计算的压缩曲线函数"
    );
    assert!(
        BRIDGE.contains("threshold + (input_db - threshold) / ratio"),
        "必须是真实压缩传递函数（硬拐点），⛔ 不是常量曲线"
    );
    assert!(
        BRIDGE.contains("pub fn project_compressor_curve("),
        "必须有从设备参数取数的投影函数"
    );
}

#[test]
fn bus_comp_card_consumes_the_computed_curve() {
    assert!(
        HOST.contains("ui.set_comp_curve_svg(view.compressor_curve_svg().into());"),
        "宿主必须把曲线注入界面属性"
    );
    assert!(
        APP.contains("comp-curve-svg"),
        "MainWindow 必须有 comp-curve-svg 属性"
    );
    assert!(
        LEFT.contains("in property <string> comp-curve-svg"),
        "左栏组件必须接收 comp-curve-svg"
    );
    assert!(
        LEFT.contains("commands: root.comp-curve-svg;"),
        "BUS COMP 卡片的 Path 必须消费真实曲线"
    );
    assert!(
        LEFT.contains("BUS COMP · 示意"),
        "无压缩器时必须仍诚实标「示意」（不能假装有响应）"
    );
}
