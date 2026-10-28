//! 流派矩阵（MCP 侧）：**每一条登记的流派**用自己的默认值规划一个段落。
//!
//! 单位 = **流派条目**（`GenreLibrary::all()`，现读 182 条 ⇒ 读数由判据自己数，⛔ 抄数字）。
//!
//! 由来（`G8d` 的教训，2026-10-28 补覆盖）：那一次实测出 MCP 侧一个**常量 16**
//! 让 `GENRES` 里 **28 条非 4/4** 流派的鼓/旋律根本生成不出来（theory 报
//! `ProgressionTooDense`），而当时 MCP 侧的判据只有少数**手挑夹具**
//! （`synthwave` / `waltz` …）⇒ 同样的回归再犯一次，现有判据一条都不会红。
//! theory 侧早就有全流派矩阵（`tests/properties.rs` 的
//! `every_genre_produces_a_playable_sketch` / `every_genre_produces_a_melody_in_its_own_scale` /
//! `every_registered_genre_produces_a_well_formed_drum_pattern`），缺的正是**穿过
//! MCP 规划层**的那一遍。
//!
//! 口径（关键的一面）：**所有可选键全缺省** —— `scale`／`progression` 都给 `None`，
//! 于是每一流派读的是它**自己登记**的 `primary_scale` / `progression_at(0)`
//! （`theory_inputs` 现读逻辑），而不是本文件挑的一条走向 ⇒ 这条判据问的是
//! "登记了却生成不出来"的那类自相矛盾（若有，就是真缺陷，⛔ 放宽成本判据）。

use std::collections::BTreeMap;

use yeban_mcp::domain::section_build::{PlanOptions, SectionPlan, plan_with_options};
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::GenreLibrary;

/// 矩阵的每格小节数：取最小非零值 —— 本判据量的是"生成得出来吗"，
/// ⛔ 量"生成多久/多长"（那是各票自己的形状判据）。
const BARS: u64 = 1;

/// 全缺省的可选项：只有"theory 材料源 + 鼓 + 旋律"是这条判据的**主题**，
/// 其余 16 个键一律 `None`（任何一个非 `None` 都会把某一支流派专属逻辑带进来）。
fn options() -> PlanOptions<'static> {
    PlanOptions {
        material_source: Some("theory"),
        progression: None,
        include_drums: Some(true),
        include_melody: Some(true),
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
        beat_grouping: None,
        melody_seed: None,
    }
}

/// 声部名 ⇒ 音符条数（声部名取 `"{section} · {part}"` 的后半，与同目录其余判据一致）。
fn note_counts(section_plan: &SectionPlan) -> BTreeMap<String, usize> {
    let mut map: BTreeMap<String, usize> = BTreeMap::new();
    for op in &section_plan.ops {
        let Op::AddClip { clip } = op else { continue };
        let ClipContent::Midi { notes } = &clip.content else {
            continue;
        };
        let part = clip
            .name
            .rsplit(" · ")
            .next()
            .unwrap_or(clip.name.as_str())
            .to_owned();
        *map.entry(part).or_default() += notes.len();
    }
    map
}

#[test]
fn every_registered_genre_plans_with_its_own_defaults() {
    let project = filled_project();
    let genres = GenreLibrary::all();
    assert_eq!(
        genres.len(),
        GenreLibrary::len(),
        "`all()` 与 `len()` 必须同源（⛔ 一条为空而另一条非空）"
    );

    let mut failures: Vec<String> = Vec::new();
    let mut empty_parts: Vec<String> = Vec::new();
    for rule in genres {
        match plan_with_options(&project, "Chorus", rule.id, BARS, None, options()) {
            Ok(section_plan) => {
                let counts = note_counts(&section_plan);
                // 三件最小生成物：和声声部（≥1 条）、鼓、旋律。
                // ⚠ 和声声部名随流派/声部数变化 ⇒ 只要求"存在**非** Drums/Melody 的声部有条"。
                let harmony = counts
                    .iter()
                    .filter(|(part, _)| part.as_str() != "Drums" && part.as_str() != "Melody")
                    .map(|(_, n)| *n)
                    .sum::<usize>();
                let drums = counts.get("Drums").copied().unwrap_or(0);
                let melody = counts.get("Melody").copied().unwrap_or(0);
                if harmony == 0 || drums == 0 || melody == 0 {
                    empty_parts.push(format!(
                        "{}: harmony={harmony} drums={drums} melody={melody}",
                        rule.id
                    ));
                }
            }
            Err(fault) => failures.push(format!("{}: {fault:?}", rule.id)),
        }
    }

    assert!(
        failures.is_empty(),
        "登记的流派必须都能用**自己的默认值**规划出一个段落（失败 {} 条 / 共 {} 条）: {:?}",
        failures.len(),
        genres.len(),
        failures
    );
    assert!(
        empty_parts.is_empty(),
        "规划成功但生成物为空（{} 条）: {:?}",
        empty_parts.len(),
        empty_parts
    );
}
