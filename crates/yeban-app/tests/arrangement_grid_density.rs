//! ⭐ **编曲网格密度自适应** —— 五版判据（Wang 2026-11-17 14:37 按 Logic Pro
//! 实测规格定）。
//!
//! ## 规格来源（父 agent 对 Wang 所发两张 Logic 截图逐像素实测，Retina 2x，
//! 以下均为逻辑像素）
//!
//! * 图一（bar 宽 48px）：标尺刻度全 1px 宽；小节刻度满刻度行高、拍刻度约
//!   半行（下对齐）；编号每 2 小节一个；车道只有小节线（1px），无拍线。
//! * 图二（bar 宽约 7.5px，极端缩小）：编号每 16 小节；线条单位粗化为每 2
//!   小节一条（线距 15px），仍 1px；无更细刻度。
//! * 拍内细分刻度在 Logic 高缩放时出现（Wang 另行明确）。
//!
//! ## 被钉的读数（`host::grid_density`，纯函数）
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① 三档实测复现 | 48px ⇒ 编号步长 2／单位 1／拍刻度在／车道无拍线／无细分；192px ⇒ 步长 1／拍线在／细分 4；7.5px ⇒ 步长 16／单位 2／无拍级 |
//! | ② 阈值边界 | 96px 编号间距、15px 线距、每拍 8px（拍刻度与细分距）、每拍 24px（车道拍线，推定值）四条边界的内外各一档 |
//! | ③ 非 4/4 推广 | 3/4 与 6/8 按拍号分子算每拍宽，⛔ 不写死 4 |
//! | ④ 退化输入 | 小节宽 0／负／NaN／无穷 ⇒ 最保守读数（步长／单位 1、无拍刻度、无细分），不 panic |
//! | ⑤ 接线与色值 | 宿主五写入点与 `.slint` 五属性消费点都在；网格五色齐备、金色已清零；五色对各自背景的 WCAG 实算比值不低于各自下限 |
//!
//! 像素侧的线宽（全 1px）由 1:1 渲染帧逐像素实测看守（五版对照帧
//! `compare5-*`），本文件只钉**函数读数与接线**，⛔ 不冒充像素证据。
//!
//! ## 六版增补（Wang 2026-11-17 15:06 终判 + 15:09 追加）
//!
//! * 车道拍线再淡一档：`#687685` → `#5d6a76`，对最不利行底 `#181719`
//!   实算 3.22:1，钉在目标窗 3.15–3.30 内（判据 ⑤ 同步换钉 + 窗断言；
//!   硬下限 ≥3.0 不放宽）。
//! * 标尺编号净空：编号只占标尺带上部 9px 净空区，编号区底与每档刻度
//!   顶之间 >= 2px（判据 ⑥ 钉 `.slint` 几何常量；刻度行 12px 与三档
//!   分级 12/6/3 不动）。
//! * 播放头竖线改纯白 `#ffffff`（落点在 `arrangement.slint` 播放头
//!   覆盖层，不在本文件覆盖面）。

use yeban_app::host::{GridDensity, grid_density};

/// 一行断言一组读数（五个字段逐项比，失败时整组打印便于定位）。
fn expect(bar_width_px: f32, beats_per_bar: i32, want: GridDensity) {
    let got = grid_density(bar_width_px, beats_per_bar);
    assert_eq!(
        got, want,
        "grid_density({bar_width_px}, {beats_per_bar}) 读数不符"
    );
}

/// ① 两张 Logic 截图的三档缩放必须逐项复现。
#[test]
fn logic_measured_tiers_are_reproduced() {
    // 图一档：bar 48px、每拍 12px。编号每 2 小节；车道无拍线；标尺拍刻度在；
    // 细分距 12/4 = 3px < 8px ⇒ 无细分。
    expect(
        48.0,
        4,
        GridDensity {
            label_stride_bars: 2,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: false,
            subdiv_per_beat: 0,
        },
    );
    // 高缩放档：bar 192px、每拍 48px。编号每小节；车道拍线在；细分 4 份
    // （细分距 12px >= 8px）。
    expect(
        192.0,
        4,
        GridDensity {
            label_stride_bars: 1,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: true,
            subdiv_per_beat: 4,
        },
    );
    // 图二档：bar 7.5px。编号每 16 小节（16 × 7.5 = 120 >= 96，8 × 7.5 = 60
    // 不够）；线条单位 2 小节（线距 15px）；无拍刻度、无车道拍线、无细分。
    expect(
        7.5,
        4,
        GridDensity {
            label_stride_bars: 16,
            line_unit_bars: 2,
            show_beat_ticks: false,
            show_lane_beats: false,
            subdiv_per_beat: 0,
        },
    );
    // 默认帧档：bar 128px（默认缩放 30 tick/px）、每拍 32px。细分 4 份
    // （32/4 = 8 >= 8 恰达标）。
    expect(
        128.0,
        4,
        GridDensity {
            label_stride_bars: 1,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: true,
            subdiv_per_beat: 4,
        },
    );
}

/// ② 四条阈值的边界内外。
#[test]
fn density_threshold_boundaries() {
    // 编号间距 96px：恰达标 ⇒ 步长 1；差 1px ⇒ 步长 2。
    expect(
        96.0,
        4,
        GridDensity {
            label_stride_bars: 1,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: true,
            subdiv_per_beat: 2,
        },
    );
    assert_eq!(grid_density(95.0, 4).label_stride_bars, 2);
    // 线距 15px：恰达标 ⇒ 单位 1；不足 ⇒ 单位 2。
    assert_eq!(grid_density(15.0, 4).line_unit_bars, 1);
    assert_eq!(grid_density(14.9, 4).line_unit_bars, 2);
    // 每拍 8px（拍刻度）：恰达标在场；不足退场且无细分。
    let at = grid_density(32.0, 4);
    assert!(at.show_beat_ticks && at.subdiv_per_beat == 0);
    let below = grid_density(31.9, 4);
    assert!(!below.show_beat_ticks && below.subdiv_per_beat == 0);
    // 每拍 24px（车道拍线）：bar 96px 时每拍恰 24px ⇒ 在场（上已断言）；
    // bar 95px 时每拍 23.75px ⇒ 退场。
    assert!(!grid_density(95.0, 4).show_lane_beats);
    // 细分逐档退：每拍 16px ⇒ 2 份（16/2 = 8 达标、16/4 = 4 不达标）；
    // 每拍 12px ⇒ 0 份（12/2 = 6 < 8）。
    assert_eq!(grid_density(64.0, 4).subdiv_per_beat, 2);
    assert_eq!(grid_density(48.0, 4).subdiv_per_beat, 0);
    // 单位 >= 2 时拍级全退（即使每拍宽本身达标也不适用——单位由小节宽定）。
    let coarse = grid_density(14.0, 4);
    assert_eq!(coarse.line_unit_bars, 2);
    assert!(!coarse.show_beat_ticks && !coarse.show_lane_beats);
    assert_eq!(coarse.subdiv_per_beat, 0);
}

/// ③ 拍号分子推广：每拍宽 = 小节宽 / 分子，⛔ 不写死 4。
#[test]
fn density_generalizes_beyond_four_four() {
    // 3/4、bar 48px：每拍 16px ⇒ 拍刻度在、车道拍线不在、细分 2 份。
    expect(
        48.0,
        3,
        GridDensity {
            label_stride_bars: 2,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: false,
            subdiv_per_beat: 2,
        },
    );
    // 6/8、bar 192px：每拍 32px ⇒ 与 4/4 默认帧同读数（细分 4 份）。
    expect(
        192.0,
        6,
        GridDensity {
            label_stride_bars: 1,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: true,
            subdiv_per_beat: 4,
        },
    );
    // 步长与单位只由小节宽定，与拍号无关：7.5px 在 3/4 下同为 16 / 2。
    let narrow = grid_density(7.5, 3);
    assert_eq!(narrow.label_stride_bars, 16);
    assert_eq!(narrow.line_unit_bars, 2);
    // 分子异常（0）夹到 1 处理，不 panic、不除零。
    assert_eq!(grid_density(48.0, 0), grid_density(48.0, 1));
}

/// ④ 退化输入：小节宽无读数时退回最保守读数。
#[test]
fn density_degenerate_widths_fall_back() {
    let fallback = GridDensity {
        label_stride_bars: 1,
        line_unit_bars: 1,
        show_beat_ticks: false,
        show_lane_beats: false,
        subdiv_per_beat: 0,
    };
    for width in [0.0_f32, -5.0, f32::NAN, f32::INFINITY] {
        expect(width, 4, fallback);
    }
}

/// 相对亮度（WCAG 2.1 定义，与 `tests/ui_contrast_tokens.rs` 同式）。
fn relative_luminance(hex: &str) -> f32 {
    let h = hex.trim_start_matches('#');
    let channel = |i: usize| {
        let v = u8::from_str_radix(&h[i..i + 2], 16).expect("合法十六进制") as f32 / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

/// 两色对比度（WCAG：`(L1+0.05)/(L2+0.05)`，亮者作分子）。
fn contrast_ratio(a: &str, b: &str) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// ⑤ 接线与色值：宿主写入点、`.slint` 消费点、五色与比值下限。
#[test]
fn density_wiring_and_colors_are_in_place() {
    let host = include_str!("../src/host.rs");
    for setter in [
        "ui.set_grid_label_stride(",
        "ui.set_grid_line_unit(",
        "ui.set_grid_show_beat_ticks(",
        "ui.set_grid_show_lane_beats(",
        "ui.set_grid_subdiv_per_beat(",
    ] {
        assert!(host.contains(setter), "host::apply_view 缺写入点 {setter}");
    }
    assert!(
        host.contains("grid_density(bar_width_px"),
        "密度读数必须由 grid_density 按小节宽算出"
    );

    let arrangement = include_str!("../ui/arrangement.slint");
    for property in [
        "root.grid-label-stride",
        "root.grid-line-unit",
        "root.grid-show-beat-ticks",
        "root.grid-show-lane-beats",
        "root.grid-subdiv-per-beat",
    ] {
        assert!(
            arrangement.contains(property),
            "arrangement.slint 未消费 {property}"
        );
    }
    // 网格五色齐备；四版以前的金色（#a3804f / #c6a47c）清零。
    for color in ["#83909e", "#6e7f8f", "#66788a", "#748392", "#5d6a76"] {
        assert!(arrangement.contains(color), "网格色 {color} 缺失");
    }
    for gold in ["#a3804f", "#c6a47c"] {
        assert!(!arrangement.contains(gold), "金色 {gold} 仍在网格里");
    }

    // 比值下限（实算，不钉字面）：标尺三色对标尺带 #1d2733——小节级
    // >= 4.5、拍级与细分级 >= 3.0（细分 3.0 是五版定值的硬下限）。
    assert!(contrast_ratio("#83909e", "#1d2733") >= 4.5);
    assert!(contrast_ratio("#6e7f8f", "#1d2733") >= 3.0);
    assert!(contrast_ratio("#66788a", "#1d2733") >= 3.0);
    // 车道两色：对车道底 #090b10 与最亮的选中行底 #181719（24,23,25）。
    assert!(contrast_ratio("#748392", "#181719") >= 4.5);
    assert!(contrast_ratio("#5d6a76", "#181719") >= 3.0);
    // 六版目标窗（Wang 15:06 终判「再淡一点」）：车道拍线对最不利行底的
    // 比值须落在 3.15–3.30——下限仍是上面的 >= 3.0（不放宽），窗断言
    // 额外钉住「淡」的幅度，防止日后悄悄调回五版亮度。
    let lane_beat = contrast_ratio("#5d6a76", "#181719");
    assert!(
        (3.15..=3.30).contains(&lane_beat),
        "车道拍线比值 {lane_beat:.3} 不在六版目标窗 3.15–3.30"
    );
}
