//! **辅助文字的对比度下限**（用户 2026-11-16 反馈 + 外部评审「文字对比度过低」）。
//!
//! 修前：8 个界面文件里 100 处 `color: #94a3b8`（深灰底上的暗灰字），
//! 在 1080P/2K 下辨识度低，需费力才能看清。
//!
//! 修法：`color:` 后的 `#94a3b8` 统一提亮为 `#b6c2d2`（深底上对比度显著提升）；
//! `background:` / `stroke:` 上的装饰线**不动**（它们不是文字，提亮反而抢注意力）。
//!
//! ⚠ 纯 `include_str!` 判据；像素效果由 5 张 golden 基准看守（本改动触发重录）。

const FILES: [(&str, &str); 8] = [
    ("left_panel", include_str!("../ui/left_panel.slint")),
    ("lower_dock", include_str!("../ui/lower_dock.slint")),
    ("arrangement", include_str!("../ui/arrangement.slint")),
    ("transport_bar", include_str!("../ui/transport_bar.slint")),
    ("modals", include_str!("../ui/modals.slint")),
    ("clip_matrix", include_str!("../ui/clip_matrix.slint")),
    ("control_room", include_str!("../ui/control_room.slint")),
    (
        "console/piano_roll",
        include_str!("../ui/console/piano_roll.slint"),
    ),
];

#[test]
fn muted_text_labels_are_brighter_than_94a3b8() {
    let mut bad = Vec::new();
    for (name, src) in FILES {
        for (i, line) in src.lines().enumerate() {
            if line
                .split_once("color:")
                .is_some_and(|(_, rest)| rest.contains("#94a3b8"))
            {
                bad.push(format!("{name}:{}", i + 1));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "这些行的 color: 仍是低对比度 #94a3b8（应提亮为 #b6c2d2）：{bad:?}"
    );
}

#[test]
fn decorative_strips_may_keep_the_old_tone() {
    // 装饰线（background/stroke）不在提亮范围：至少保留一处，证明没有误改装饰色。
    let left = FILES[0].1;
    assert!(
        left.contains("background: #94a3b8;") || left.contains("#94a3b8"),
        "装饰线不应被一并提亮（避免抢文字注意力）"
    );
}

// ===========================================================================
// 第十轮补丁：`#64748b` 文字色（实测最亮底 3.61:1，低于 AA 4.5）
// ===========================================================================

/// 相对亮度（WCAG 2.1 定义：sRGB 通道 → 线性 → 0.2126R+0.7152G+0.0722B）。
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

/// 两色对比度（WCAG：`(L1+0.05)/(L2+0.05)`，亮者作分母）。
fn contrast_ratio(a: &str, b: &str) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// 判据 1：8 个界面文件里**不再有** `color: #64748b`（它是最亮面板底上唯一不过线的文字色）。
#[test]
fn no_text_color_64748b_left() {
    let mut bad = Vec::new();
    for (name, src) in FILES {
        for (i, line) in src.lines().enumerate() {
            if line
                .split_once("color:")
                .is_some_and(|(_, rest)| rest.contains("#64748b"))
            {
                bad.push(format!("{name}:{}", i + 1));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "这些行的文字色仍是 #64748b（应改为 ≥AA 的 #7c8aa0）：{bad:?}"
    );
}

/// 判据 2：把"替代色"钉在**真实算出来的比值**上，而不是字面匹配。
///
/// 背景取 UI 里最亮的三种面板底（`#161b28` 卡片 / `#111827` 面板 / `#0b1220` 窗口），
/// 只要其中任意一个不过 4.5:1 就红——这条以后换任何颜色都会先被算一遍。
#[test]
fn muted_text_meets_wcag_aa_on_every_panel_background() {
    const MUTED_TEXT: &str = "#7c8aa0";
    for bg in ["#161b28", "#111827", "#0b1220"] {
        let ratio = contrast_ratio(MUTED_TEXT, bg);
        assert!(
            ratio >= 4.5,
            "{MUTED_TEXT} 在 {bg} 上只有 {ratio:.2}:1，低于 WCAG AA 的 4.5:1"
        );
    }
    // 同时钉住"旧值确实不过线"，防止有人把这条判据顺手改成恒真。
    assert!(
        contrast_ratio("#64748b", "#161b28") < 4.5,
        "#64748b 在 #161b28 上本就超过 4.5:1，本判据的前提失效，需重新评估"
    );
}

