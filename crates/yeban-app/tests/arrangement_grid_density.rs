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
//!
//! ## 七版增补（Wang 2026-11-17 15:49 + 15:51 亲定）
//!
//! * 钢琴卷帘播放头同步改纯白 `#ffffff`（判据 ⑦ 钉 `piano_roll.slint`
//!   落点与对最深行底 `#161b26` 的比值）。
//! * 编曲区行底分界线提亮定值 `#5a6672`：对最不利行底 `#181719`
//!   实算 3.04:1，钉在目标窗 3.0–3.1 内，且相对亮度须低于拍线
//!   `#5d6a76`（判据 ⑦）。
//! * 轨道选中边框统一纯白、选中底改定值（行底 `#131a20`／头底
//!   `#16232b`，判据在 `selection_highlight_uses_accent.rs` 换钉）；
//!   本文件判据 ⑦ 另钉小节线/拍线对新选中行底的比值不破既有下限。
//!
//! ## 八版增补（Wang 2026-11-17 16:08 + 16:09 亲定，照 Logic Pro）
//!
//! * 选中改 Logic 式：纯白边框作废，选中只由提亮承载——行底
//!   `#0f1218`、头底 `#293645`（Logic 亮度比 1.80 线性提亮；判据在
//!   `selection_highlight_uses_accent.rs` 换钉）；本文件判据 ⑦ 另钉
//!   小节线/拍线对新选中行底的实算比值（小节线 4.83、拍线 3.38）。
//! * 行底分界线再深一档：`#5a6672` → `#4e5964`，对最不利行底
//!   `#181719` 实算 2.50:1，钉在目标窗 2.3–2.7 内，仍暗于拍线
//!   `#5d6a76`（判据 ⑦）。`#181719` 不再是选中行底，八版起只作
//!   保守最亮参考底保留（实算比值取最不利情形）。
//! * 片段底改纯色：编曲区片段块外层底改为不透明 `#090b10`（判据 ⑦
//!   后新立判据钉住）；Clip 矩阵槽底、下坞轨道徽章底范围外，只列
//!   清单不动。
//! * 片段内标记改白色系实色（16:23 追加）：波形峰值柱与 MIDI 音符条
//!   改纯白 `#ffffff`（对最不利片段实色底实算 14.71:1），波形基准线
//!   与 MIDI 分界线改灰实色 `#748392`（弱于纯白、分主次）。
//! * 和弦块底实色化（16:23 追加）：五块和弦底（原 α≈0.09）与边框
//!   （原 α≈0.21）全换合成等效实色（叠和弦轨带底 `#0d1017` 的呈现
//!   色），hover 同法；和弦名文字对新实底实算 7.56–8.79:1。
//!
//! ## 九版增补（Wang 2026-11-17 16:43 亲定，照 Logic Pro 区域）
//!
//! * 片段底改轨道色纯色本体：八版「合成等效实色」= 0.12×轨道色 + 0.88×近黑，呈现为轨道色混黑、暗且脏，废。底由投影层 `bridge::clip_base_color` 算（`blk.base-color`）：纯色不混黑；仅当白色标记对纯色底不足 3.05 时按统一规则只降 HSL 明度（每步 0.02，色相/饱和不动）到达标。判据 ⑧ 换钉落点 + 全调色板普查（产品自带/自分派轨道色 + 默认色 + 白，逐色钉落码值并实算三项下限）。
//! * 同色边框去除：未选中片段 `trk.color` α0.55 边框整圈去掉（宽 0）；选中片段 2px 纯白边框保留（判据 ⑧）。
//! * 片段内文字/导线改投影推导：文字 `blk.text-color`（近白→纯白→纯黑，第一达 4.5 者）、导线 `blk.guide-color`（灰 `#748392` 达 3.0 则灰，否则黑/白取高者）；纯白标记沿用八版（判据 ⑨）。
//! * 和弦块同口径改纯色：五块底 = 和弦色本体，hover = 同色明度 ±0.04（方向取文字仍 >= 4.5 的一侧），边框 = 同色更深一档实色（-0.10 / hover -0.05）；文字按底色选近白/纯白（判据 ⑨）。
//!
//! ## 十版增补（Wang 2026-11-17 18:28 亲定，照 Logic Pro 截图逐像素实测）
//!
//! * 标记改同色相派生色：纯白 `#FFFFFF` 标记太刺眼（Logic 波形色
//!   不是纯白也不是固定色）。波形峰值柱/MIDI 音符条/波形基准线/
//!   MIDI 分界线改由投影层 `bridge::clip_marker_color` 派生：轨道色
//!   HSV（H 不变、V = 0.99、S × 1/3），即底色的浅色版（Logic 实测
//!   对：蓝 `#4A80CE`→`#C6DEFC`、紫 `#6F3CCC`→`#D7C4FB`、品红
//!   `#9724C2`→`#E6B3FB`，对底 2.9–5.2:1）。判据 ⑧ 普查逐色钉
//!   派生落码值（精确值断言取代九版导线 >= 3.0 地板——派生规则本身
//!   不承诺该地板，Logic 实测下限 2.9）；判据 ⑨ 纯白落点 4 → 2
//!   （只剩播放头线/菱形头，Wang 亲定纯白不动）。片段名文字维持
//!   九版规则（近白优先，>= 4.5），与 Logic 分工一致。和弦块内无
//!   波形/音符类标记（只有名称文字），本项不波及。
//! * 车道底抬升：未选中车道底 `#090b10`（相对亮度 0.0033）→
//!   `#2e2e2e`（0.0273，Logic 编曲区底实测同值），只落车道容器，
//!   `Theme.bg-canvas` 本体与标尺带 `Theme.bg-panel` 不动（标尺带
//!   与车道底不共用令牌）。线色按新底重算、比率目标不变：拍线
//!   `#828f9c`（对最不利底 = 选中行 3.21，窗 3.15–3.30；对未选中
//!   行 4.11）、小节线 `#a7b0ba`（4.82 / 6.18，下限 4.5）、分界线
//!   `#6d7d8c`（2.50，窗 2.3–2.7；对未选中行 3.21，仍暗于拍线）。
//!   选中行底维持八版相对提亮比 1.80：`#383f4c`（相对亮度
//!   0.0492 = 1.80 × 0.0273）；选中头底 `#293645` 不动（头底基色
//!   `#1d2733` 未变）。九版起充当保守最亮参考底的 `#181719` 退役
//!   ——新底组（选中行）本身即最不利底。
//!
//! ## 十一版增补（Wang 2026-11-17 18:51 + 18:52 + 18:53 亲定）
//!
//! * 车道底回蓝调抬亮：十版中性灰 `#2e2e2e` 与全应用深蓝色系
//!   （轨道头 `#1d2733`、标尺带）差异太大 ⇒ 改回九版 `#090b10`
//!   的色相/饱和（HSV 222.9° / 0.4375）只抬明度：定值 `#1d2333`
//!   （相对亮度 0.0170，目标带 0.014–0.020，仍低于十版 0.0273）。
//!   落点仍只落车道容器，`Theme.bg-canvas` 本体与标尺带不动。
//! * 选中行底维持相对提亮比 1.80 按新基底重算：`#273146`
//!   （相对亮度 0.0307 = 1.80 × 0.0170）；选中头底 `#293645`
//!   不动。线色按新底重算、比率目标不变：拍线 `#74808b`（对最
//!   不利底 = 选中行 3.22，窗 3.15–3.30；对未选中行 3.88）、
//!   小节线 `#979fa8`（4.86 / 5.85，下限 4.5）。
//! * 分界线改暗线（18:52，推翻八版以来亮线方向）：取比车道底
//!   更暗的同色系深色 `#040508`（相对亮度 0.0015 < 两底），暗线
//!   对两底实算 1.30（未选中）/ 1.57（选中）。⚠ Wang 原目标窗
//!   2.3–2.7 在暗方向物理不可达：WCAG 对比含 +0.05 眩光项，
//!   车道底亮度 0.0170 时纯黑线对未选中底上限仅 1.34:1；判据按
//!   实算紧窗换钉（1.28–1.32 / 1.54–1.59）并断言方向（分界线
//!   相对亮度低于两底）。层级按绝对对比仍 分界线 < 拍线 < 小节线。
//! * 十版派生标记（`bridge::clip_marker_color`）原样保留，判据
//!   ⑧ 不动；片段名文字规则、播放头纯白、选中片段 2px 纯白边
//!   均不动。

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
    // 网格五色齐备；四版以前的金色（#a3804f / #c6a47c）清零；
    // 十版起旧车道线色（#748392 / #5d6a76）清零（已按新底重算）；
    // 十一版起十版底与线色全组清零（底回蓝调重算）。
    for color in ["#83909e", "#6e7f8f", "#66788a", "#979fa8", "#74808b"] {
        assert!(arrangement.contains(color), "网格色 {color} 缺失");
    }
    for gold in ["#a3804f", "#c6a47c"] {
        assert!(!arrangement.contains(gold), "金色 {gold} 仍在网格里");
    }
    for retired in [
        "#748392", "#5d6a76", "#2e2e2e", "#383f4c", "#a7b0ba", "#828f9c", "#6d7d8c",
    ] {
        assert!(
            !arrangement.contains(retired),
            "旧底/旧线色 {retired} 仍在（十一版已按新底重算）"
        );
    }

    // 比值下限（实算，不钉字面）：标尺三色对标尺带 #1d2733——小节级
    // >= 4.5、拍级与细分级 >= 3.0（细分 3.0 是五版定值的硬下限）。
    // 标尺带与车道底不共用令牌，十版不动。
    assert!(contrast_ratio("#83909e", "#1d2733") >= 4.5);
    assert!(contrast_ratio("#6e7f8f", "#1d2733") >= 3.0);
    assert!(contrast_ratio("#66788a", "#1d2733") >= 3.0);
    // 车道两色（十一版按新底重算）：最不利底 = 选中行底 #273146
    // （新底组里最亮）；未选中行底 #1d2333 次之。
    assert!(contrast_ratio("#979fa8", "#273146") >= 4.5);
    assert!(contrast_ratio("#74808b", "#273146") >= 3.0);
    assert!(contrast_ratio("#979fa8", "#1d2333") >= 4.5);
    assert!(contrast_ratio("#74808b", "#1d2333") >= 3.0);
    // 拍线目标窗（六版定，比率目标不变）：对最不利底的比值须落在
    // 3.15–3.30——下限仍是上面的 >= 3.0（不放宽），窗断言额外钉住
    // 「淡」的幅度，防止日后悄悄调回更亮。
    let lane_beat = contrast_ratio("#74808b", "#273146");
    assert!(
        (3.15..=3.30).contains(&lane_beat),
        "车道拍线比值 {lane_beat:.3} 不在目标窗 3.15–3.30"
    );
}

/// 从 `src[..end]` 里最后一个 `{marker}` 之后读出 px 数值（`.slint`
/// 几何常量提取，无正则，手写扫描）。
fn last_px_before(src: &str, end: usize, marker: &str) -> f32 {
    let head = &src[..end];
    let idx = head.rfind(marker).expect("标记必须存在");
    let rest = &head[idx + marker.len()..];
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().expect("标记之后必须是数值")
}

/// 从 `src[from..]` 里第一个 `{marker}` 之后读出 px 数值。
fn first_px_after(src: &str, from: usize, marker: &str) -> f32 {
    let rest = &src[from..];
    let idx = rest.find(marker).expect("标记必须存在");
    let tail = &rest[idx + marker.len()..];
    let num: String = tail
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().expect("标记之后必须是数值")
}

/// ⑥ 编号净空（六版，Wang 15:06 终判）：编号区与刻度行分区，编号区底
/// 与每档刻度顶之间 >= 2px 净空；三档刻度分级比例（满行 : 1/2 : 1/4）
/// 不变。钉的是 `arrangement.slint` 里的几何常量本身，⛔ 不靠目视。
#[test]
fn ruler_numbers_keep_a_clear_zone_above_the_ticks() {
    let arrangement = include_str!("../ui/arrangement.slint");

    // 标尺带高：`// Ruler (24px)` 注释之后第一个 height。
    let ruler_at = arrangement.find("// Ruler (24px)").expect("标尺注释在");
    let band = first_px_after(arrangement, ruler_at, "height: ");

    // 编号区高：编号 Text 的 text 行之前最近的 height。
    let text_at = arrangement
        .find("text: \"\" + (root.bar-start-index + i + 1);")
        .expect("编号 Text 在");
    let number_zone = last_px_before(arrangement, text_at, "height: ");

    // 三档刻度：各自 background 行之前最近的 height（刻度高）与
    // `parent.height - Npx`（刻度底距带底的距离，即刻度行上沿位置）。
    let tick = |color: &str| {
        let at = arrangement
            .find(&format!("background: {color};"))
            .unwrap_or_else(|| panic!("刻度色 {color} 在"));
        (
            last_px_before(arrangement, at, "height: "),
            last_px_before(arrangement, at, "parent.height - "),
        )
    };
    let (bar_h, bar_from_bottom) = tick("#83909e");
    let (beat_h, beat_from_bottom) = tick("#6e7f8f");
    let (sub_h, sub_from_bottom) = tick("#66788a");

    // 分级比例不变：小节满刻度行、拍 1/2、细分 1/4。
    assert_eq!(beat_h * 2.0, bar_h, "拍刻度须为小节刻度高的一半");
    assert_eq!(sub_h * 4.0, bar_h, "细分刻度须为小节刻度高的四分之一");

    // 净空不变量：每档刻度顶（自带顶起算）与编号区底之间 >= 2px。
    for (name, from_bottom) in [
        ("小节", bar_from_bottom),
        ("拍", beat_from_bottom),
        ("细分", sub_from_bottom),
    ] {
        let tick_top = band - from_bottom;
        assert!(
            tick_top - number_zone >= 2.0,
            "{name}刻度顶 {tick_top} 与编号区底 {number_zone} 之间净空不足 2px"
        );
    }
}

/// ⑦ 十一版（Wang 2026-11-17 18:51 + 18:52 + 18:53）：车道底改回
/// 蓝调 `#1d2333`（相对亮度带 0.014–0.020）、选中行底按 1.80 提亮比
/// 重算、分界线改暗线（相对亮度低于两底）、线色按新底重算后比值
/// 不破下限；卷帘播放头改白断言保留（七版）。
#[test]
fn rev11_divider_and_selected_row() {
    let arrangement = include_str!("../ui/arrangement.slint");

    // 车道底落点：车道容器必须是十一版定值 #1d2333，且其实算相对
    // 亮度落入 Wang 18:53 亲定的目标带 0.014–0.020。
    assert!(
        arrangement.contains("background: #1d2333;"),
        "车道容器底必须用十一版定值 #1d2333"
    );
    let base_lum = relative_luminance("#1d2333");
    assert!(
        (0.014..=0.020).contains(&base_lum),
        "车道底相对亮度 {base_lum:.4} 不在目标带 0.014–0.020"
    );
    // 选中行底：相对亮度比维持八版 1.80（对未选中行底 #1d2333 实算）。
    let lift = relative_luminance("#273146") / base_lum;
    assert!(
        (1.79..=1.81).contains(&lift),
        "选中行提亮比 {lift:.3} 不在 1.80 ± 0.01"
    );

    // 分界线落点：「行底分界线」注释之后第一个 background 必须是定值。
    let divider_at = arrangement.find("行底分界线").expect("分界线注释在");
    let rest = &arrangement[divider_at..];
    let bg_idx = rest.find("background: ").expect("分界线 background 在");
    let bg_val: String = rest[bg_idx + "background: ".len()..]
        .chars()
        .take_while(|c| *c != ';')
        .collect();
    assert_eq!(
        bg_val.trim(),
        "#040508",
        "分界线必须用十一版暗线定值 #040508"
    );

    // 分界线方向（十一版改暗线，Wang 18:52）：相对亮度必须低于
    // 未选中行底与选中行底（暗于背景），且低于车道拍线。
    let divider_lum = relative_luminance("#040508");
    assert!(
        divider_lum < base_lum && divider_lum < relative_luminance("#273146"),
        "分界线 #040508 必须暗于两行底（暗线方向）"
    );
    assert!(
        divider_lum < relative_luminance("#74808b"),
        "分界线必须暗于车道拍线 #74808b"
    );
    // 分界线比值（暗线对亮底方向）：对未选中行底实算 1.30（窗
    // 1.28–1.32）、对选中行底实算 1.57（窗 1.54–1.59）。⚠ Wang 原
    // 目标窗 2.3–2.7 在暗方向物理不可达（纯黑对未选中底上限
    // 1.34:1），此处按实算紧窗钉住。
    let divider_lane = contrast_ratio("#040508", "#1d2333");
    assert!(
        (1.28..=1.32).contains(&divider_lane),
        "分界线对未选中行底比值 {divider_lane:.3} 不在实算窗 1.28–1.32"
    );
    let divider_sel = contrast_ratio("#040508", "#273146");
    assert!(
        (1.54..=1.59).contains(&divider_sel),
        "分界线对选中行底比值 {divider_sel:.3} 不在实算窗 1.54–1.59"
    );

    // 选中行底 #273146 下，小节线/拍线对它的实算比值不破既有下限
    // （4.5 / 3.0，与判据 ⑤ 同口径），并钉住实算值本身（小节线
    // 4.86、拍线 3.22，窗内浮动只容舍入）。
    assert!(contrast_ratio("#979fa8", "#273146") >= 4.5);
    assert!(contrast_ratio("#74808b", "#273146") >= 3.0);
    let bar_sel = contrast_ratio("#979fa8", "#273146");
    assert!(
        (4.83..=4.89).contains(&bar_sel),
        "小节线对选中行底比值 {bar_sel:.3} 不在实算窗 4.83–4.89"
    );
    let beat_sel = contrast_ratio("#74808b", "#273146");
    assert!(
        (3.15..=3.30).contains(&beat_sel),
        "拍线对选中行底比值 {beat_sel:.3} 不在实算窗 3.15–3.30"
    );
    // 层级（对选中行底的绝对对比）：分界线 < 拍线 < 小节线。
    assert!(
        divider_sel < beat_sel && beat_sel < bar_sel,
        "层级须 分界线 < 拍线 < 小节线（实算 {divider_sel:.3} / {beat_sel:.3} / {bar_sel:.3}）"
    );

    // 卷帘播放头：`piano-roll-playhead` 元素向上的最近一处 background
    // 必须是纯白；白色对卷帘最深行底 #161b26（黑键行）>= 17。
    let piano_roll = include_str!("../ui/console/piano_roll.slint");
    let ph_at = piano_roll
        .find("accessible-id: \"piano-roll-playhead\"")
        .expect("卷帘播放头在");
    let head = &piano_roll[..ph_at];
    let bg_rel = head.rfind("background: ").expect("播放头 background 在");
    let ph_val: String = head[bg_rel + "background: ".len()..]
        .chars()
        .take_while(|c| *c != ';')
        .collect();
    assert_eq!(ph_val.trim(), "#ffffff", "卷帘播放头必须纯白 #ffffff");
    assert!(contrast_ratio("#ffffff", "#161b26") >= 17.0);
}

/// RGB → HSL（与 `bridge::clip_base_color` 的压暗规则对账用：
/// 压暗只许降明度，色相/饱和须守恒）。
fn rgb_to_hsl(hex: &str) -> (f32, f32, f32) {
    let h = hex.trim_start_matches('#');
    let chan =
        |i: usize| u8::from_str_radix(&h[i..i + 2], 16).expect("合法十六进制") as f32 / 255.0;
    let (r, g, b) = (chan(0), chan(2), chan(4));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let hue = if (max - r).abs() < f32::EPSILON {
        let mut x = (g - b) / d;
        if g < b {
            x += 6.0;
        }
        x / 6.0
    } else if (max - g).abs() < f32::EPSILON {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (hue, s, l)
}

/// ⑧ 十版片段底纯色 + 去同色边框 + 标记同色相派生（底/边框为九版
/// Wang 2026-11-17 16:43 亲定，标记为十版 18:28 亲定）：
/// 片段块底必须是投影层算好的轨道色纯色 `blk.base-color`（八版
/// 「近黑底 + 0.12 叠层」混黑做法清零）；未选中片段的同色边框
/// （`trk.color` α0.55）去掉（宽 0），选中片段 2px 纯白边框保留；
/// 文字/导线/标记走投影字段。调色板普查逐色钉落码值：底/文字
/// 沿九版下限实算（白对底 >= 3.0、文字 >= 4.5），标记/导线改钉
/// 派生精确值（十版规则不承诺九版导线 >= 3.0 地板，精确值断言
/// 取代之，强度不降）。
#[test]
fn rev10_clip_background_is_pure_track_color() {
    use yeban_app::bridge::{
        RgbColor, clip_base_color, clip_marker_color, clip_text_color, track_color_or_default,
    };

    let arrangement = include_str!("../ui/arrangement.slint");
    let clips_at = arrangement
        .find("for blk in root.timeline-blocks")
        .expect("片段循环在");
    let rest = &arrangement[clips_at..];
    let bg_idx = rest.find("background: ").expect("片段块 background 在");
    let bg_val: String = rest[bg_idx + "background: ".len()..]
        .chars()
        .take_while(|c| *c != ';')
        .collect();
    assert_eq!(
        bg_val.trim(),
        "blk.base-color",
        "片段块底必须是轨道色纯色 blk.base-color（九版），不得混黑/透底"
    );
    // 八版混黑两件套清零：0.12 叠层与 α0.55 同色边框。
    for old in ["trk.color.with-alpha(0.12)", "trk.color.with-alpha(0.55)"] {
        assert!(!arrangement.contains(old), "八版片段做法 {old} 仍在");
    }
    // 边框：未选中宽 0（无边框）、选中 2px；边框色只剩纯白（选中指示）。
    assert!(
        arrangement.contains("? 2px : 0px"),
        "未选中片段边框宽必须为 0（同色边框已去除）"
    );
    // 文字与导线走投影字段（名字/增益/标注/空片段 4 处文字、2 处导线）。
    assert_eq!(
        arrangement.matches("color: blk.text-color;").count(),
        4,
        "片段内文字应为 4 处走 blk.text-color"
    );
    assert_eq!(
        arrangement.matches("background: blk.guide-color;").count(),
        2,
        "片段内导线应为 2 处走 blk.guide-color"
    );
    assert_eq!(
        arrangement.matches("background: blk.marker-color;").count(),
        2,
        "片段内标记应为 2 处走 blk.marker-color（峰值柱/音符条）"
    );

    // 调色板普查：产品自带/自分派的全部轨道色（自动分派 6 色、夹具
    // 3 色）、默认色与白色（最不利），逐色按 bridge 规则钉落码值。
    // （原色、落码底、文字、标记）——标记与导线同出
    // `clip_marker_color`（导线字段由 host 同值填充，接线计数在上），
    // 数值由派生式离线算出后钉死，bridge 规则若漂移此处即红。
    // 下限实算只保留仍成立的两项：白对底 >= 3.0（底压暗规则的
    // 兜底性质，十版不变）、文字 >= 4.5；标记改精确值断言（十版
    // 派生规则对底实算 2.22–7.96，与 Logic 实测带 2.9–5.2 同量级，
    // 不再钉 >= 3.0 地板）。
    let census: [(&str, &str, &str, &str); 11] = [
        ("#2C3A63", "#2C3A63", "#F1F5F9", "#CEDAFC"),
        ("#FF8800", "#E07800", "#000000", "#FCD5A8"),
        ("#33AAFF", "#0095FF", "#000000", "#B9E0FC"),
        ("#66DD88", "#24A449", "#000000", "#CFFCDC"),
        ("#DD66CC", "#DD66CC", "#000000", "#FCCFF6"),
        ("#FFD166", "#C28800", "#000000", "#FCEDCA"),
        ("#8C8CFF", "#8282FF", "#000000", "#D7D7FC"),
        ("#3366FF", "#3366FF", "#FFFFFF", "#B9CAFC"),
        ("#22AA88", "#20A281", "#000000", "#B9FCEC"),
        ("#F7E6B0", "#B48E12", "#000000", "#FCF7E4"),
        ("#FFFFFF", "#8F8F8F", "#000000", "#FCFCFC"),
    ];
    for (orig, want_base, want_text, want_marker) in census {
        let track = track_color_or_default(Some(orig));
        let base = clip_base_color(track);
        assert_eq!(base.to_hex(), want_base, "{orig} 的落码底色不符");
        assert_eq!(
            clip_text_color(base).to_hex(),
            want_text,
            "{orig} 的文字色不符"
        );
        assert_eq!(
            clip_marker_color(track).to_hex(),
            want_marker,
            "{orig} 的标记派生色不符"
        );
        // 下限用本文件的 WCAG 式实算（不靠 bridge 自证）。
        let base_hex = base.to_hex();
        assert!(
            contrast_ratio("#ffffff", &base_hex) >= 3.0,
            "{orig} 底 {base_hex} 上白色对比不足 3.0（底压暗规则兜底）"
        );
        assert!(
            contrast_ratio(want_text, &base_hex) >= 4.5,
            "{orig} 底 {base_hex} 上文字对比不足 4.5"
        );
        // 压暗守恒：落码与原色同色相、同饱和（只降明度）；未压暗者
        // 落码 == 原色。
        let (h0, s0, l0) = rgb_to_hsl(orig);
        let (h1, s1, l1) = rgb_to_hsl(&base_hex);
        assert!(
            (h0 - h1).abs() < 0.01 && (s0 - s1).abs() < 0.02 && l1 <= l0 + 0.004,
            "{orig} → {base_hex} 不是只降明度（色相 {h0:.3}→{h1:.3}、饱和 {s0:.3}→{s1:.3}）"
        );
    }
    // 默认色走同一入口（缺失/非法回退色也是调色板成员）。
    assert_eq!(
        clip_base_color(RgbColor::new(0x2c, 0x3a, 0x63)).to_hex(),
        "#2C3A63"
    );
}

/// ⑨ 片段内标记与和弦块纯色（和弦为九版 Wang 2026-11-17 16:43，
/// 标记为十版 18:28，十一版仅换车道小节线重算值）：波形峰值柱/
/// MIDI 音符条改走投影派生色 `blk.marker-color`、片段内导线走
/// `blk.guide-color`（同出派生函数）；纯白只剩播放头线/菱形头
/// 2 处（Wang 亲定播放头纯白不动）；车道小节线改十一版重算值
/// `#979fa8` 2 处、旧值 `#a7b0ba` 与 `#748392` 清零；五块和弦底
/// 为和弦色纯色本体、hover/边框为同色明度档实色，八版合成等效
/// 实色与更早的 alpha 字面量全清零（沿九版）。
#[test]
fn rev11_clip_markers_and_chord_blocks_are_pure() {
    let arrangement = include_str!("../ui/arrangement.slint");

    // 片段内旧透画标记清零（本文件内三处原为轨道色 α0.85/0.20/0.15）。
    for old in [
        "trk.color.with-alpha(0.85)",
        "trk.color.with-alpha(0.20)",
        "trk.color.with-alpha(0.15)",
    ] {
        assert!(!arrangement.contains(old), "片段内透画标记 {old} 仍在");
    }
    // 纯白落点计数：十版只剩播放头线/菱形头 2 处（峰值柱/音符条
    // 已改走 blk.marker-color，判据 ⑧ 钉计数 2）。
    assert_eq!(
        arrangement.matches("background: #ffffff;").count(),
        2,
        "纯白实色落点应为 2 处（播放头线/菱形头；片段标记已改派生色）"
    );
    assert_eq!(
        arrangement.matches("background: #979fa8;").count(),
        2,
        "#979fa8 落点应为 2 处（车道小节线两处）"
    );
    assert!(
        !arrangement.contains("#748392") && !arrangement.contains("#a7b0ba"),
        "旧小节线色 #748392 / #a7b0ba 应清零（十一版已按新底重算为 #979fa8）"
    );
    // 派生标记对最不利落码底（白色轨道：派生 #fcfcfc 对压暗底
    // #8f8f8f）实算 3.15 >= 3.0；其余调色板底由判据 ⑧ 的普查
    // 逐色钉精确值看守。
    assert!(contrast_ratio("#fcfcfc", "#8f8f8f") >= 3.0);

    // 和弦块：旧 alpha 字面量（底 α0x18/0x35、边框 α0x35/0x70）清零。
    for old in [
        "#1b283818",
        "#1b283835",
        "#2d203818",
        "#2d203835",
        "#182c2618",
        "#182c2635",
        "#34261618",
        "#34261635",
        "#38769135",
        "#38769170",
        "#705e8835",
        "#705e8870",
        "#34786435",
        "#34786470",
        "#9a6a3835",
        "#9a6a3870",
    ] {
        assert!(!arrangement.contains(old), "和弦块 alpha 字面量 {old} 仍在");
    }
    // 八版合成等效实色（混黑）清零。
    for old in [
        "#0e121a", "#10151e", "#162530", "#203d4d", "#10121a", "#14131e", "#22202e", "#383249",
        "#0e1318", "#0f161a", "#152627", "#1e3e39", "#111217", "#151517", "#2a231e", "#4b3825",
    ] {
        assert!(!arrangement.contains(old), "八版和弦块混黑实色 {old} 仍在");
    }
    // 九版纯色齐备：四色底（Cm 两块同色）+ hover 明度档 + 边框深档。
    for pure in [
        "#387691", "#326a82", "#2a586c", "#31677f", "#705e88", "#7a6694", "#57496a", "#645479",
        "#347864", "#2e6a58", "#255446", "#2c6655", "#9a6a38", "#8b6033", "#75502a", "#875d31",
    ] {
        assert!(arrangement.contains(pure), "和弦块纯色 {pure} 缺失");
    }
    // 和弦名文字对纯色底（含 hover 底）>= 4.5（本文件 WCAG 式实算）。
    for (text, base) in [
        ("#f1f5f9", "#387691"),
        ("#f1f5f9", "#326a82"),
        ("#f1f5f9", "#705e88"),
        ("#f1f5f9", "#7a6694"),
        ("#f1f5f9", "#347864"),
        ("#f1f5f9", "#2e6a58"),
        ("#ffffff", "#9a6a38"),
        ("#ffffff", "#8b6033"),
    ] {
        assert!(
            contrast_ratio(text, base) >= 4.5,
            "和弦文字 {text} 对底 {base} 对比不足 4.5"
        );
    }
}
