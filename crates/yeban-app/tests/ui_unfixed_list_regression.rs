//! **`未修复问题清单-0f181bd.md` 里可确定性修复项的回归判据**。
//!
//! 该清单的 P0-3 / P1-1 / P1-2 / P2-1 是**双层键路的语义冲突**
//! （`input.rs` 解析表 vs `app.slint` key-handler 各绑一次），依 `AGENTS.md §5.7`
//! 需人类裁决「哪一层是唯一事实源」，本轮**不擅自改动**；只把冲突如实写进
//! `--print-shortcuts`（判据 ④）并留痕 ADR。
//!
//! 本判据钉住的是清单里**零语义裁量、可确定性修复**的四项：
//! - ① P0-2 `EQ CURVE` 接上**已存在**的真实数据 `MainWindow.eq-curve-svg`
//!   （`bridge::calculate_eq_curve_svg` → `host.rs` → `device_rack.slint` 同一路），
//!   缺省 `"M 0 50 L 420 50"` 与旧 0dB 基准线逐像素一致 ⇒ 不动 5 张 golden。
//! - ② P1-3 chip 文案去掉会在窄框里被吃掉尾字的 `·`（`和弦·收起` → `和弦收起` 等）。
//! - ③ P1-4 波形占位长句在窄片段上改为**按宽度隐藏**，不再显示被裁成「波形」的
//!   半截误导文案；完整语义保留在 `accessible-label`。
//! - ④ P2-2 `--print-shortcuts` 明确列出「双层都绑」的三条键。
//!
//! 纯 `include_str!` 源码文本判据：零 GUI / 零音频 / 零重依赖，`rustc --test` 可直跑。

const APP: &str = include_str!("../ui/app.slint");
const LEFT_PANEL: &str = include_str!("../ui/left_panel.slint");
const ARRANGEMENT: &str = include_str!("../ui/arrangement.slint");
const CLI: &str = include_str!("../src/cli.rs");

/// 去掉整行注释后取代码行，避免注释里的示例文案被误判为实现。
fn code_lines(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ① EQ CURVE 卡片接的是**真实参数曲线**，不是常量折线 / 空框。
#[test]
fn eq_curve_thumbnail_is_wired_to_the_real_computed_svg() {
    let app = code_lines(APP);
    let left = code_lines(LEFT_PANEL);
    // MainWindow → ZoneBrowserAndInspector 的属性传递
    assert!(
        app.contains("eq-curve-svg: root.eq-curve-svg;"),
        "MainWindow 必须把 eq-curve-svg 传给左侧检查器"
    );
    // 组件侧声明 + 真正用于 Path.commands
    assert!(
        left.contains("in property <string> eq-curve-svg"),
        "ZoneBrowserAndInspector 必须声明 eq-curve-svg 入参"
    );
    assert!(
        left.contains("commands: root.eq-curve-svg;"),
        "EQ CURVE 卡片的 Path 必须消费真实曲线，⛔ 不是画死几何"
    );
    assert!(
        left.contains("viewbox-width: 420;") && left.contains("viewbox-height: 100;"),
        "Path 必须带与 device_rack 相同的 viewbox，否则缺省基准线会被裁到框外"
    );
    // 缺省帧必须逐像素不变：Path 在缺省基准线时隐藏，0dB Rectangle 仍在。
    assert!(
        left.contains(r#"visible: root.eq-curve-svg != "M 0 50 L 420 50";"#),
        "缺省曲线时 Path 必须隐藏，否则 5 张 Linux golden 会被改动"
    );
    assert!(
        left.contains("background: root.selected-track-color.with-alpha(0.35);"),
        "0dB 基准线 Rectangle 必须保留（缺省帧唯一可见的绘制）"
    );
    // 已接真实数据 ⇒ 不能再自称「示意」（诚实性）
    assert!(
        !left.contains("EQ CURVE · 示意"),
        "接了真实曲线还标「示意」= 新的假显示"
    );
}

/// ② chip 文案不含会在窄框里被截断的间隔号 `·`。
#[test]
fn chip_labels_avoid_the_truncation_prone_middle_dot() {
    let arr = code_lines(ARRANGEMENT);
    for label in ["和弦收起", "和弦展开", "轨道菜单"] {
        assert!(
            arr.contains(label),
            "chip 文案 `{label}` 缺失（清单 P1-3：旧 `·` 版本尾字被吃掉）"
        );
    }
    for legacy in ["和弦·收起", "和弦·展开", "轨道·菜单"] {
        assert!(
            !arr.contains(legacy),
            "旧 chip 文案 `{legacy}` 又回来了，窄片段会再被裁成「轨道 ·展」"
        );
    }
}

/// ③ 波形占位长句按片段宽度隐藏，且完整语义进无障碍标签。
#[test]
fn waveform_placeholder_label_is_width_gated() {
    let arr = code_lines(ARRANGEMENT);
    assert!(
        arr.contains("visible: parent.width >= 88px;"),
        "窄片段必须隐藏整句，⛔ 不显示被裁成「波形」的半截误导文案（清单 P1-4）"
    );
    assert!(
        arr.contains("峰值分析尚未接入") && arr.contains("accessible-label:"),
        "隐藏后完整语义必须保留在 accessible-label（读屏器可读）"
    );
    // 文案本身仍在（ui_display_honesty 判据 ② 仍绿）
    assert!(arr.contains("波形待分析 · 示意"));
}

/// ④ ADR-0074 R1 收敛后：解析表键不再出现在「界面直达键」段（改由主表覆盖）。
#[test]
fn shortcut_table_no_longer_lists_parsed_keys_as_ui_direct() {
    let cli = code_lines(CLI);
    for gone in [
        "#   Ctrl+T",
        "#   X               打开底部混音控制台",
        "#   P               打开底部钢琴卷帘",
        "#   C               循环播放开关",
    ] {
        assert!(
            !cli.contains(gone),
            "R1 后 {gone:?} 由解析表唯一执行，不应再在「界面直达键」段单列"
        );
    }
    // 纯界面键仍在
    assert!(cli.contains("#   F7 / F8 / F9"));
}

/// ⑤ 修复不得把此前清掉的七处假几何画回来。
#[test]
fn fabricated_geometry_does_not_return() {
    const FAKES: [&str; 7] = [
        "M 0 12 L 6 4 L 12 14 L 18 3 L 26 15 L 34 2",
        "M 0 12 L 6 18 L 12 13 L 18 20 L 26 13 L 34 21",
        "M 0 28 Q 20 25, 35 12 T 70 18 T 100 24",
        "M 0 20 Q 35 18, 55 12 T 100 16",
        "M 0 36 Q 40 12, 80 34 T 160 18 T 240 32",
        "M 0 48 Q 55 42, 110 20 T 220 34",
        "M 0 18 Q 20 8, 38 16 T 55 12 T 72 20",
    ];
    let sources = [code_lines(ARRANGEMENT), code_lines(LEFT_PANEL)];
    for fake in FAKES {
        for src in &sources {
            assert!(
                !src.contains(fake),
                "假几何 `{fake}` 又被画回来了（显示诚实性回归）"
            );
        }
    }
}
