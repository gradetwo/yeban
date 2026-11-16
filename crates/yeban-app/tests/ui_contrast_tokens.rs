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
