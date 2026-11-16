//! **选中轨道高亮不得与轨道身份色撞色**（用户 2026-11-16 反馈）。
//!
//! 修前：选中态的行底色/边框用的是 `trk.color`（轨道自身颜色），而轨道色条/片段
//! 也是同一个色 ⇒ 「选中」与「这条轨是什么色」混在一起，看不出哪条被选中。
//!
//! 修法：选中态统一用 `Theme.accent`（低饱和强调色）做底色与边框；
//! 轨道色只保留在左侧 `5px` 色条上（身份标识）。可读性来自「强调色边框 + 轨道色条」
//! 两种语义分离。
//!
//! ⚠ 纯 `include_str!` 判据；像素级效果由 5 张 golden 基准（`tests/golden/linux`）看守，
//! 本改动会触发基准重录（`gates-manual` 的 `goldens` 档）。

const ARR: &str = include_str!("../ui/arrangement.slint");

#[test]
fn selected_track_highlight_uses_accent_not_track_color() {
    assert!(
        !ARR.contains("selected-track-id ? trk.color"),
        "选中高亮不得再用 trk.color（与轨道身份色撞色 ⇒ 看不出选中）"
    );
    assert!(
        ARR.contains("selected-track-id ? Theme.accent.with-alpha(0.08)"),
        "行底色必须用统一强调色（Theme.accent）"
    );
    assert!(
        ARR.contains("selected-track-id ? Theme.accent.with-alpha(0.70)"),
        "行边框必须用统一强调色"
    );
    assert!(
        ARR.contains("selected-track-id ? Theme.accent.with-alpha(0.80)"),
        "轨道头边框必须用统一强调色"
    );
}

#[test]
fn track_color_strip_is_preserved_as_identity() {
    assert!(
        ARR.contains("background: trk.color;"),
        "左侧色条必须保留轨道色（身份标识），否则选中后认不出是哪条轨"
    );
}
