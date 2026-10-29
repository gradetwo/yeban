//! 缺口补全（审计 `N4`）：`yeban_propose_section` 的 `progression` 此前只收
//! **罗马数字**（`Progression::parse`），而 theory 的 `Chord::from_symbol`
//! （"Dm7" 一类绝对符号）一直没有接进走向 —— 本票把 `theory_spans` 补成
//! **双语法**：全串能按级数解析就走罗马路径（既有行为逐字节不变），失败再按
//! [`yeban_theory::progression::expand_chord_symbols`] 逐词解释成绝对和弦符号
//! （符号自带根音/性质 ⇒ 不依赖调式，`ChordSpan.degree` 为 `None`）。
//!
//! 时值分配复用 [`Progression::expand`] 的三条规则（≥小节数时每小节一个循环、
//! 否则按拍、退 16 分、过密报错）⇒ 判据 ⛔ 另写一份分配表。
//!
//! ⚠ 判定顺序是"**整体**先罗马"而⛔ 逐词：罗马数字里不可能出现字母根音
//! （`I`/`V`/`X`/`L`/`i`/`v`/`x`/`l` 都不在 `A..=G` 音名集里）⇒ 两种语法的
//! 合法串**无交集**，不存在"某串既可当级数又可当符号"的歧义；混合写法
//! （`"I-Dm7"`）两边都失败 ⇒ 判据 3 把**两个**错误都钉进载荷。
//!
//! ⚠ `degree` 诚实性：符号来源的区段没有"原始级数" ⇒ `ChordSpan.degree`
//! 一律 `None`（⛔ 拿调内最近级数冒充 —— 那样 `"E7"` 在 C 大调里会被贴上
//! 一个它根本不归属的级数标签）。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options,
};
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::chord::Chord;

/// 夹具的小节数。
const BARS: u64 = 4;
/// 夹具的风格预设：3 声部（`Bass` / `Alto` / `Soprano`）。
const STYLE: &str = "synthwave";
/// 夹具的调：C 自然小调（符号路径不依赖它，但 `scale` 实参仍在场）。
const KEY: &str = "C minor";
/// 符号走向夹具：Dm7 / G7 / Cmaj7，4 小节 ⇒ 第 4 小节循环回 Dm7。
const SYMBOLS: &str = "Dm7-G7-Cmaj7";

fn plan(progression: &str) -> SectionPlan {
    try_plan(progression).unwrap_or_else(|fault| {
        panic!("theory 规划（progression={progression:?}）必须成功: {fault:?}")
    })
}

fn try_plan(progression: &str) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        &filled_project(),
        "Chorus",
        STYLE,
        BARS,
        Some(KEY),
        PlanOptions {
            material_source: Some("theory"),
            progression: Some(progression),
            include_drums: None,
            include_melody: None,
            drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
            melody_lower: None,
            melody_upper: None,
            melody_max_leap: None,
            harmony_max_voice_jump: None,
            swing_permille: None,
            drum_backbeat: None,
            drum_style: None,
            melody_seed: None,
            harmony_seed: None,
            material_clip_ids: None,
            part_tracks: None,
            beat_grouping: None,
        },
    )
}

/// 和声声部的全部音符（`Op::AddClip` 里 `"{section} · {part}"` 命名的 MIDI 片段，
/// ⛔ 只看 voice count 个声部 —— 鼓/旋律键本夹具不给）。
fn harmony_notes(section_plan: &SectionPlan) -> Vec<(u64, u8)> {
    let mut notes = Vec::new();
    for op in &section_plan.ops {
        let Op::AddClip { clip } = op else { continue };
        let ClipContent::Midi { notes: table } = &clip.content else {
            continue;
        };
        for note in table.values() {
            notes.push((note.start_tick, note.pitch));
        }
    }
    notes.sort_unstable();
    notes
}

/// 判据 1：符号走向真的展开 —— 每小节的和声音符全部落在该小节那个
/// **符号和弦**的构成音里（按 4/4 = 3840 tick/小节分桶）。
#[test]
fn symbol_progression_voices_the_named_chords_per_bar() {
    let section_plan = plan(SYMBOLS);
    let chords = ["Dm7", "G7", "Cmaj7", "Dm7"]
        .iter()
        .map(|text| Chord::from_symbol(text).unwrap())
        .collect::<Vec<_>>();
    let tone_sets: Vec<Vec<u8>> = chords
        .iter()
        .map(|chord| {
            chord
                .pitch_classes()
                .iter()
                .map(|pc| pc.semitones())
                .collect()
        })
        .collect();
    let notes = harmony_notes(&section_plan);
    assert!(!notes.is_empty(), "和声声部必须有音符落点");
    let mut per_bar: BTreeMap<u64, usize> = BTreeMap::new();
    for (start, pitch) in &notes {
        let bar = start / 3840;
        *per_bar.entry(bar).or_default() += 1;
        assert!(
            tone_sets[bar as usize].contains(&(pitch % 12)),
            "第 {bar} 小节的音高 {pitch} 不在 {} 的构成音里",
            ["Dm7", "G7", "Cmaj7", "Dm7"][bar as usize]
        );
    }
    assert_eq!(
        per_bar.len(),
        4,
        "3 个和弦铺 4 小节 ⇒ 第 4 小节循环回首个符号"
    );
}

/// 判据 2：符号路径与级数路径在同小节数下展开出**同一条**时值骨架
/// （时值分配是 `expand` 的共享规则 ⇒ 起点/时值序列必须逐项相同）。
#[test]
fn symbol_and_roman_paths_share_the_timeline() {
    let symbol_notes = harmony_notes(&plan("Dm7-G7-Cmaj7"));
    let roman_notes = harmony_notes(&plan("i-VI-III-VII"));
    let timeline = |notes: &[(u64, u8)]| {
        notes
            .iter()
            .map(|(start, _)| *start)
            .collect::<BTreeSet<u64>>()
    };
    assert_eq!(
        timeline(&symbol_notes).into_iter().collect::<Vec<_>>(),
        timeline(&roman_notes).into_iter().collect::<Vec<_>>(),
        "两种语法的起点序列必须逐项相同（同一套分配规则）"
    );
}

/// 判据 3：两种语法都不认的写法 ⇒ `INVALID_PARAMETER_RANGE`，载荷**同时**
/// 带 `romanError` 与 `symbolError`（⛔ 只报后失败的那个）。
#[test]
fn mixed_or_invalid_progression_reports_both_parse_errors() {
    let fault = try_plan("I-Dm7").expect_err("混合写法两边都拒");
    assert_eq!(
        fault.domain_code(),
        Some(BuildCode::InvalidParameterRange),
        "混合写法必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
    );
    let BuildFault::Domain { data, .. } = &fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    let roman = data["romanError"].as_str().unwrap_or_default().to_owned();
    let symbol = data["symbolError"].as_str().unwrap_or_default().to_owned();
    assert!(!roman.is_empty(), "必须带罗马路径的错误: {data}");
    assert!(!symbol.is_empty(), "必须带符号路径的错误: {data}");
    assert_eq!(data["progression"], Value::from("I-Dm7"));

    let fault = try_plan("VIII").expect_err("非法级数也两边都拒（V 不是音名）");
    let BuildFault::Domain { data, .. } = &fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    assert!(
        !data["romanError"].as_str().unwrap_or_default().is_empty()
            && !data["symbolError"].as_str().unwrap_or_default().is_empty()
    );
}
