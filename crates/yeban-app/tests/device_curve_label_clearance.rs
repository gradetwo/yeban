//! T-FIX-178 回归判据：设备曲线不得穿过「实测曲线」标签。
//!
//! 背景：`lower_dock.slint` 的设备曲线井里，曲线 `Path` 原先从 y=0 开始，
//! 高 42px；标签 `Text` 在 y=3px、8px 字（字形框约 y=3..11px）。
//! EQ 在 +12dB 时 viewbox y=16.7（`bridge::calculate_eq_curve_svg`：
//! y = 50 - gain * 50/18，见 `bridge.rs` 的 `eq_curve_projection_and_svg_generation`
//! 判据，+12dB 低架首点 y=16.7），映射到 42px 高的 Path 上约 7px，
//! 正穿过标签字形框（1x 活渲染实测 24 个曲线像素在框内）；压缩器
//! 在高输入端 viewbox y 可到 0，同样会穿过标签。
//!
//! 修法（T-FIX-178）：曲线 `Path` 下移，顶部留出标签高度，底部不动，
//! 仍守住 T-FIX-112 的 ACTIVE 行间隙。本判据用纯文本几何钉住它：
//! 以 viewbox y=0（钳制后的最坏情况，任何增益/压缩比都到不了更上方）
//! 为准，曲线描边顶缘必须在标签字形框底缘之下；曲线底缘到 ACTIVE 行
//! （y=46px）仍须留至少 4px。
//!
//! 纯 `include_str!` 判据：零 GUI / 零音频 / 零重依赖。

const LOWER_DOCK: &str = include_str!("../ui/lower_dock.slint");

/// 在 `haystack` 里找 `needle: <数字>px` 的第一个数值。
fn px_after(haystack: &str, needle: &str) -> f32 {
    let start = haystack
        .find(needle)
        .unwrap_or_else(|| panic!("lower_dock.slint 里找不到 `{needle}`"));
    let rest = &haystack[start + needle.len()..];
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse::<f32>()
        .unwrap_or_else(|_| panic!("`{needle}` 后不是数字：{rest:?}"))
}

#[test]
fn device_curve_stays_below_the_measured_curve_label() {
    // 曲线 Path 段：从 C2 注释起到标签 Text 之前。
    let curve_start = LOWER_DOCK
        .find("真实设备曲线")
        .expect("lower_dock 必须有真实设备曲线注释");
    let label_start = LOWER_DOCK[curve_start..]
        .find("@tr(\"实测曲线\")")
        .map(|i| curve_start + i)
        .expect("lower_dock 必须有 @tr(\"实测曲线\") 标签");
    let curve = &LOWER_DOCK[curve_start..label_start];
    assert!(
        curve.contains("device-curve-svgs[idx]"),
        "曲线 Path 必须消费 root.device-curve-svgs"
    );
    let path_y = px_after(curve, "y: ");
    let path_height = px_after(curve, "height: ");
    let viewbox_height = px_after(curve, "viewbox-height: ");
    let stroke = px_after(curve, "stroke-width: ");
    assert!(
        (viewbox_height - 100.0).abs() < 0.01,
        "曲线 viewbox 高必须是 100（判据按此换算），实得 {viewbox_height}"
    );

    // 标签字形框：y=3px、8px 字，保守按字高 8px 计，底缘 = 3 + 8 = 11px。
    // 标签的 y 在 @tr 之前、在所属 Text 块之内；font-size 在 @tr 之后、
    // 同一 Text 块之内，所以分两段解析。
    let text_start = LOWER_DOCK[..label_start]
        .rfind("Text {")
        .expect("实测曲线标签须在 Text 块内");
    let label_y = px_after(&LOWER_DOCK[text_start..label_start], "y: ");
    let font_size = px_after(&LOWER_DOCK[label_start..], "font-size: ");
    let label_bottom = label_y + font_size;
    assert!(
        (label_y - 3.0).abs() < 0.01 && (font_size - 8.0).abs() < 0.01,
        "标签须在 y=3px、8px 字（判据按此计字形框），实得 y={label_y} font={font_size}"
    );

    // 最坏情况 viewbox y=0（钳制顶）：描边顶缘仍须在标签底缘之下。
    // 另验 +12dB EQ（viewbox y=16.7）、压缩器高输入端（viewbox y=22.5，阈值
    // -18dB/比率 4）和 0dB（viewbox y=50）。
    for viewbox_y in [0.0f32, 16.7, 22.5, 50.0] {
        let display_y = path_y + viewbox_y / viewbox_height * path_height;
        let stroke_top = display_y - stroke / 2.0;
        assert!(
            stroke_top > label_bottom,
            "viewbox y={viewbox_y} 的曲线描边顶缘 {stroke_top:.1}px 必须在标签字形框底缘 {label_bottom:.1}px 之下（Path y={path_y}px 高={path_height}px）"
        );
    }

    // 底部仍守 T-FIX-112：曲线底缘到 ACTIVE 行（y=46px）至少 4px。
    let active_row_y = 46.0f32;
    let curve_bottom = path_y + path_height;
    assert!(
        active_row_y - curve_bottom >= 4.0 - 0.01,
        "曲线底缘 {curve_bottom:.1}px 到 ACTIVE 行 {active_row_y}px 须留至少 4px"
    );
    assert!(
        LOWER_DOCK[label_start..].contains("y: 46px"),
        "ACTIVE/#N 行须仍在 y=46px"
    );
}
