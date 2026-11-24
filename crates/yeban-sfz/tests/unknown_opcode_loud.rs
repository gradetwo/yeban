//! ⭐ **未知 opcode 响亮判据**（Wang 2026-11-18 开发规则「默认不能静默」
//! 的首个适用面）。
//!
//! 旧行为：装载器不认识的 opcode 名被**静默忽略**（个别既有测试甚至把
//! 「像未知 opcode 一样忽略」钉为预期）。本判据钉新口径：
//!
//! * 每个不认识的 opcode 名在装载报告里恰出一条
//!   [`Warning::UnknownOpcode`]（按名去重，跨 region 不重复刷）；
//! * 认得的名字（静态、调制、家族形态、别名、`<control>` 特殊名）
//!   **不得**误报；
//! * `<curve>` / `<effect>` 段里不被认领的名字同样响亮；
//! * 无 `sample` 被整体丢弃的 region 不逐名扫（其丢弃已有
//!   `RegionWithoutSample` 告警；持久作用域的名字在文件尾扫尾仍会被报）。

use yeban_sfz::{ParseLimits, Warning, parse_text};

/// 一份文本里全部未知 opcode 告警（顺序即装载报告顺序）。
fn unknown_notes(text: &str) -> Vec<Warning> {
    parse_text(text, &ParseLimits::default())
        .unwrap_or_else(|error| panic!("`{text}` 必须可装载：{error}"))
        .warnings()
        .iter()
        .filter(|warning| matches!(warning, Warning::UnknownOpcode { .. }))
        .cloned()
        .collect()
}

#[test]
fn an_unknown_opcode_warns_exactly_once_per_name() {
    // 同一个不认识的名字在多条 region 里出现 ⇒ 恰一条（按名去重）。
    let text = "<region> sample=a.wav frobnicate=3\n<region> sample=b.wav frobnicate=9";
    assert_eq!(
        unknown_notes(text),
        vec![Warning::UnknownOpcode {
            line: 1,
            name: "frobnicate".to_string(),
        }]
    );
    // 两个不同的不认识名字 ⇒ 两条。
    let two = unknown_notes("<region> sample=a.wav frobnicate=3 zanzibar=1");
    assert_eq!(two.len(), 2, "不同名字各记一条，实得 {two:?}");
    // 告警文案逐字钉。
    assert_eq!(
        two[0].to_string(),
        "line 1: unknown opcode `frobnicate` ignored"
    );
}

#[test]
fn an_unknown_opcode_in_a_persistent_scope_still_warns_once() {
    // `<global>` 里的不认识名字被每条 region 的扫尾看到，但只报首次。
    let text = "<global> frobnicate=1\n<region> sample=a.wav\n<region> sample=b.wav";
    let notes = unknown_notes(text);
    assert_eq!(notes.len(), 1, "实得 {notes:?}");
    // 没有任何 region 时，文件尾扫尾仍要报（残表不许静默）。
    let notes = unknown_notes("<global> frobnicate=1");
    assert_eq!(notes.len(), 1, "无 region 的残表也必须响亮，实得 {notes:?}");
    // `<control>` 里的不认识名字在文件尾扫尾报告。
    let notes = unknown_notes("<control> frobnicate=1\n<region> sample=a.wav");
    assert_eq!(notes.len(), 1, "实得 {notes:?}");
}

#[test]
fn known_opcodes_never_warn_as_unknown() {
    // 一份把各家族都用上一遍的文本：静态、声像族与调制、门控、触发、
    // 交叉淡化、标签、EG、LFO、`<control>` 特殊名 —— 全部是认得的名字，
    // 未知 opcode 告警必须为 0（本判据同时是消费登记的假阳性守卫）。
    let text = "\
<control> default_path=samples/ label_cc7=Gate set_cc11=64 note_offset=1
<global> pan=10 position=-5 width=80 volume=3 tune=0 lokey=0 hikey=127
<region> sample=a.wav key=60 pan_oncc10=20 position_oncc11=-30 width_cc12=40 \
ampeg_attack=0.01 ampeg_release=0.5 pitch_oncc1=100 cutoff=1200 cutoff_oncc74=200 \
locc64=1 hicc64=127 on_locc32=0 on_hicc32=127 xfin_locc20=0 xfout_hicc21=127 \
xf_keycurve=power eg2_pitch_oncc140=50 amplfo_depth=2 amplfo_freqcc3=1 \
lfo1_freq=2 lfo1_pitch=100 lfo1_freq_oncc9=50 sw_lokey=36 sw_hikey=38 \
group=1 off_by=2 loop_mode=one_shot seq_length=2 rt_decay=3";
    let notes = unknown_notes(text);
    assert!(notes.is_empty(), "认得的名字不得误报未知，实得 {notes:?}");
}

#[test]
fn malformed_family_forms_surface_as_unknown_instead_of_vanishing() {
    // 无法表示的家族形态（调制解析返回 `None` 的拼写）旧行为是被家族
    // 遍历静默吞掉；新口径：名字没被任何一处认领 ⇒ 按未知 opcode 响亮。
    let notes = unknown_notes("<region> sample=a.wav pan_oncc=5 width_stepcc3=2");
    assert_eq!(notes.len(), 2, "实得 {notes:?}");
}

#[test]
fn unknown_names_in_curve_and_effect_sections_warn() {
    let notes = unknown_notes("<curve> curve_index=7 v000=0 v127=1 frobnicate=2");
    assert_eq!(
        notes.len(),
        1,
        "<curve> 里的不认识名字必须响亮，实得 {notes:?}"
    );
    let notes = unknown_notes("<effect> bus=main frobnicate=2");
    assert_eq!(
        notes.len(),
        1,
        "<effect> 里的不认识名字必须响亮，实得 {notes:?}"
    );
    // 规范的曲线点与 effect 名不误报。
    assert!(unknown_notes("<curve> curve_index=7 v000=0 v042=0.5 v127=1").is_empty());
    assert!(unknown_notes("<effect> bus=main type=lshelf effect1=20").is_empty());
}

#[test]
fn loop_count_and_loop_crossfade_variants_warn_as_unknown() {
    // ⭐ T-FIX-282：`loop_count` / `loop_crossfade` 的 smoothcc/stepcc/oncc/curvecc
    // 变体在规范里不存在（两页的 Modulation 表都没有这些行），本仓不实现它们 ⇒
    // 它们必须按未知 opcode 响亮（⛔ 不静默吃掉）；而两个**本体**名字是认得的，
    // 不得误报（本体由解析接线，见 `instrument.rs` 的循环段）。
    let notes = unknown_notes(
        "<region> sample=a.wav loop_count=3 loop_crossfade=0.01 \
         loop_count_smoothcc1=1 loop_count_stepcc1=1 loop_count_oncc1=1 loop_count_curvecc1=1 \
         loop_crossfade_smoothcc1=1 loop_crossfade_stepcc1=1 loop_crossfade_oncc1=1 loop_crossfade_curvecc1=1",
    );
    assert_eq!(notes.len(), 8, "八个未做变体各记一条，实得 {notes:?}");
    for note in &notes {
        let Warning::UnknownOpcode { name, .. } = note else {
            panic!("只应有未知 opcode 告警，实得 {note:?}");
        };
        assert!(
            name.starts_with("loop_count_") || name.starts_with("loop_crossfade_"),
            "告警名必须是变体，实得 {name}"
        );
    }
}

#[test]
fn a_discarded_region_does_not_report_its_own_names() {
    // 无 `sample` 的段整体丢弃（`RegionWithoutSample` 已响亮）：其表内
    // 名字不逐名报未知 —— 包括认得的 `key` 与不认识的名字都不报；
    // 但同一名字出现在建成的 region 里时照常报。
    let notes = unknown_notes("<region> key=60 frobnicate=1");
    assert!(notes.is_empty(), "丢弃段的名字不逐名报，实得 {notes:?}");
    let notes = unknown_notes("<region> key=60\n<region> sample=a.wav frobnicate=1");
    assert_eq!(notes.len(), 1, "实得 {notes:?}");
}
