//! **内容面去透明判据**（Wang 2026-11-17 定「按建议全改」）。
//!
//! 口径：内容面半透明一律落为纯色不透明；动态色（轨道色/总线色）用
//! `color.mix(所依底色, 原 alpha)` 取等效合成实色，视觉呈现不变、只是不再
//! 透叠。选中不靠轨道色、也不靠彩色边框，靠底色提亮一档承载（Logic 式）。
//! 瞬时反馈类（模态 scrim、hover、拖拽预览、框选、置灰）不在本判据范围。
//!
//! ⚠ 纯 `include_str!` 判据；像素级效果由出帧目视看守。

const CLIP_MATRIX: &str = include_str!("../ui/clip_matrix.slint");
const LOWER_DOCK: &str = include_str!("../ui/lower_dock.slint");
const TRANSPORT_BAR: &str = include_str!("../ui/transport_bar.slint");
const MODALS: &str = include_str!("../ui/modals.slint");
const DEVICE_RACK: &str = include_str!("../ui/console/device_rack.slint");

/// 旧的白 @ 低 alpha 发丝线字面量（改后五文件内不得再出现）。
const HAIRLINE_LITERALS: [&str; 6] = [
    "#ffffff08",
    "#ffffff0a",
    "#ffffff10",
    "#ffffff14",
    "#ffffff18",
    "#ffffff25",
];

#[test]
fn clip_matrix_slots_are_solid_track_color_mixes() {
    assert!(
        CLIP_MATRIX.contains("trk.color.mix(Theme.bg-panel, 0.18)"),
        "有片段槽底必须是轨道色等效合成实色（mix 0.18）"
    );
    assert!(
        CLIP_MATRIX.contains("trk.color.mix(Theme.bg-panel, 0.5)"),
        "有片段槽边框必须是轨道色等效合成实色（mix 0.5）"
    );
    assert!(
        CLIP_MATRIX.contains("#000000.mix(Theme.bg-panel, 0.145)"),
        "空槽底必须是等效合成实色"
    );
    assert!(
        !CLIP_MATRIX.contains("with-alpha(0.18)")
            && !CLIP_MATRIX.contains("with-alpha(0.5)")
            && !CLIP_MATRIX.contains("#00000025"),
        "Clip 矩阵槽不得再用半透明底/边框"
    );
}

#[test]
fn clip_matrix_selection_is_brightness_not_track_color() {
    assert!(
        CLIP_MATRIX.contains("selected-track-id ? Theme.bg-surface"),
        "选中行底必须用统一提亮一档的面板色（bg-surface）"
    );
    assert!(
        !CLIP_MATRIX.contains("selected-track-id ? trk.color"),
        "选中行不得再用轨道色透染"
    );
}

#[test]
fn lower_dock_badges_are_solid_mixes() {
    assert!(
        LOWER_DOCK.contains("root.selected-track-color.mix(Theme.bg-panel, 0.15)"),
        "当前轨徽章底必须是等效合成实色"
    );
    assert!(
        LOWER_DOCK.contains("trk.color.mix(#080a10, 0.18)"),
        "轨名徽章底必须是等效合成实色"
    );
    assert!(
        LOWER_DOCK.contains("Theme.track-bus.mix(#080a10, 0.20)"),
        "总线徽章底必须是等效合成实色"
    );
}

#[test]
fn lower_dock_strip_selection_is_brightness_not_accent_border() {
    assert!(
        LOWER_DOCK.contains("selected-track-id ? #0f1117 : #080a10"),
        "通道条选中必须靠底色提亮一档（#0f1117）承载"
    );
    assert!(
        !LOWER_DOCK.contains("selected-track-id ? Theme.accent : #191f2c"),
        "通道条选中不得再用 accent 色边框"
    );
}

#[test]
fn hairline_literals_are_gone_from_dock_transport_modals() {
    for (name, src) in [
        ("lower_dock", LOWER_DOCK),
        ("transport_bar", TRANSPORT_BAR),
        ("modals", MODALS),
    ] {
        for lit in HAIRLINE_LITERALS {
            assert!(
                !src.contains(lit),
                "{name} 不得再用白 alpha 发丝线字面量 {lit}（换实色 line 令牌）"
            );
        }
    }
    assert!(
        TRANSPORT_BAR.contains("Tokens.line-strong") && MODALS.contains("Tokens.line"),
        "发丝线必须换成实色 line/line-strong 令牌"
    );
    assert!(
        !MODALS.contains("#00000030") && MODALS.contains("#12161f"),
        "命令面板分类 chip 底必须是等效合成实色 #12161f"
    );
}

#[test]
fn device_rack_eq_fill_is_solid() {
    assert!(
        DEVICE_RACK.contains("#d4a017.mix(Tokens.bg-shell, 0.125)"),
        "EQ 频响填充必须是金色等效合成实色（mix bg-shell 0.125）"
    );
    assert!(
        !DEVICE_RACK.contains("#d4a01720"),
        "EQ 频响填充不得再用半透明金"
    );
}
