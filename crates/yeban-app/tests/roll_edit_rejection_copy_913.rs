//! T-FIX-913：选区/音符拖动三条拒绝文案的九语闭集（纯函数单元判据）。
//!
//! 只断言 [`yeban_app::host`] 的三条文案函数，不碰运行时 UI：
//! - 九种语言每条都有非空文案（`Language::ALL` 就是事实源）；
//! - 中文句逐字等于规范句（旧字面量作前缀 ＋ 工程未变 ＋ 可执行下一步）；
//! - 三条文案在同一种语言里互不相同（不是同一份复制）。

use yeban_app::cli::Language;
use yeban_app::host;

const NUDGE_ZH: &str =
    "模型拒绝：选区移动越界或身份已失效，工程没有变化。请重新选择后再移动，或缩小移动幅度后重试。";
const IDENTITY_ZH: &str =
    "音符拖动被拒：音符身份已失效（可能已被删除），工程没有变化。请重新选择音符后再拖动。";
const BOUNDS_ZH: &str = "音符拖动被拒：已到音高上限/下限或已到时间轴边界，工程没有变化。请缩小拖动幅度，或把音符拖回边界内再松手。";

#[test]
fn roll_rejection_copy_covers_all_nine_languages() {
    assert_eq!(Language::ALL.len(), 9, "源语言＋八个目标语言");
    for lang in Language::ALL {
        let nudge = host::roll_nudge_rejected_message(lang);
        let identity = host::roll_note_drag_identity_message(lang);
        let bounds = host::roll_note_drag_bounds_message(lang);
        assert!(!nudge.trim().is_empty(), "{lang:?} 缺微移拒绝文案");
        assert!(!identity.trim().is_empty(), "{lang:?} 缺身份失效文案");
        assert!(!bounds.trim().is_empty(), "{lang:?} 缺边界拒绝文案");
        assert_ne!(nudge, identity, "{lang:?} 微移与身份文案不得相同");
        assert_ne!(identity, bounds, "{lang:?} 身份与边界文案不得相同");
        assert_ne!(nudge, bounds, "{lang:?} 微移与边界文案不得相同");
    }
}

#[test]
fn roll_rejection_copy_zh_cn_pins_the_spec_sentences() {
    assert_eq!(
        host::roll_nudge_rejected_message(Language::ZhCn),
        NUDGE_ZH,
        "微移拒绝中文句必须逐字等于规范句"
    );
    assert_eq!(
        host::roll_note_drag_identity_message(Language::ZhCn),
        IDENTITY_ZH,
        "身份失效中文句必须逐字等于规范句"
    );
    assert_eq!(
        host::roll_note_drag_bounds_message(Language::ZhCn),
        BOUNDS_ZH,
        "边界拒绝中文句必须逐字等于规范句"
    );
    // 旧字面量作前缀：T-FIX-581 的逐字钉点与清理前缀继续命中。
    assert!(
        NUDGE_ZH.starts_with("模型拒绝：选区移动越界或身份已失效"),
        "微移文案必须保留旧字面量作前缀"
    );
    assert!(
        IDENTITY_ZH.starts_with("音符拖动被拒：音符身份已失效（可能已被删除）"),
        "身份文案必须保留旧字面量作前缀"
    );
    assert!(
        BOUNDS_ZH.starts_with("音符拖动被拒：已到音高上限/下限或已到时间轴边界"),
        "边界文案必须保留旧字面量作前缀"
    );
    // 每条都说清三件事：发生了什么、工程没有变化、下一步可做什么。
    for (name, text) in [
        ("微移", NUDGE_ZH),
        ("身份", IDENTITY_ZH),
        ("边界", BOUNDS_ZH),
    ] {
        assert!(
            text.contains("工程没有变化"),
            "{name}文案必须说清工程没有变化: {text}"
        );
        assert!(
            text.contains('，'),
            "{name}文案必须以逗号分隔原因与下一步: {text}"
        );
    }
    assert!(
        NUDGE_ZH.contains("重新选择") && NUDGE_ZH.contains("缩小移动幅度"),
        "微移文案的下一步必须是重新选择 / 缩小幅度: {NUDGE_ZH}"
    );
    assert!(
        IDENTITY_ZH.contains("重新选择音符后再拖动"),
        "身份文案的下一步必须是重新选择后再拖动: {IDENTITY_ZH}"
    );
    assert!(
        BOUNDS_ZH.contains("缩小拖动幅度") && BOUNDS_ZH.contains("拖回边界内"),
        "边界文案的下一步必须是缩小幅度 / 拖回边界内: {BOUNDS_ZH}"
    );
}

#[test]
fn roll_rejection_copy_english_carries_the_same_three_facts() {
    let nudge = host::roll_nudge_rejected_message(Language::EnUs);
    let identity = host::roll_note_drag_identity_message(Language::EnUs);
    let bounds = host::roll_note_drag_bounds_message(Language::EnUs);
    for (name, text) in [("nudge", nudge), ("identity", identity), ("bounds", bounds)] {
        assert!(
            text.contains("unchanged"),
            "{name} English copy must state the project is unchanged: {text}"
        );
    }
    assert!(
        nudge.contains("Select the notes again"),
        "nudge English next step must be re-select: {nudge}"
    );
    assert!(
        identity.contains("Select the note again"),
        "identity English next step must be re-select: {identity}"
    );
    assert!(
        bounds.contains("Drag a shorter distance"),
        "bounds English next step must be a shorter drag: {bounds}"
    );
}
