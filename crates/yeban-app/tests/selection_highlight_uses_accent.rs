//! **选中轨道高亮不得与轨道身份色撞色**（用户 2026-11-16 反馈）。
//!
//! 修前：选中态的行底色/边框用的是 `trk.color`（轨道自身颜色），而轨道色条/片段
//! 也是同一个色 ⇒ 「选中」与「这条轨是什么色」混在一起，看不出哪条被选中。
//!
//! 第一代修法（2026-11-16）：选中态统一用 `Theme.accent` 做底色与边框；
//! 轨道色只保留在左侧色条上（身份标识）。
//!
//! 第二代修法（七版，Wang 2026-11-17 15:51 亲定）：选中边框一律纯白
//! `#ffffff`（行边框与左右轨道头边框三处同改），选中底色改定值——行底
//! `#131a20`、头底 `#16232b`（均由现行 accent 合成帧上实测色向白混合
//! 3% 而来），与轨道色、accent **双脱钩**。本文件随之换钉：原「必须用
//! accent」的三条断言改为「必须用纯白/定值、且不得再用 accent」；
//! 「不得用 trk.color」与「色条保留轨道色」两条原样保留（不放宽）。
//!
//! ⚠ 纯 `include_str!` 判据；像素级效果由对照帧看守（七版 compare7-*），
//! 本改动会触发 golden 基准重录（`gates-manual` 的 `goldens` 档）。

const ARR: &str = include_str!("../ui/arrangement.slint");

#[test]
fn selected_track_highlight_uses_white_not_track_color() {
    assert!(
        !ARR.contains("selected-track-id ? trk.color"),
        "选中高亮不得再用 trk.color（与轨道身份色撞色 ⇒ 看不出选中）"
    );
    assert!(
        !ARR.contains("selected-track-id ? Theme.accent"),
        "选中高亮不得再用 Theme.accent（七版口径：与 accent 亦脱钩）"
    );
    assert!(
        ARR.contains("selected-track-id ? #ffffff"),
        "选中边框必须统一纯白 #ffffff（行边框与轨道头边框）"
    );
    assert!(
        ARR.contains("selected-track-id ? #131a20"),
        "选中行底必须用定值 #131a20"
    );
    assert!(
        ARR.contains("selected-track-id ? #16232b"),
        "选中轨道头底必须用定值 #16232b"
    );
}

#[test]
fn track_color_strip_is_preserved_as_identity() {
    assert!(
        ARR.contains("background: trk.color;"),
        "左侧色条必须保留轨道色（身份标识），否则选中后认不出是哪条轨"
    );
}
