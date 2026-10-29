//! 流派矩阵（MCP 侧）：**每一条登记的流派**用自己的默认值规划一个段落。
//!
//! 单位 = **流派条目**（`GenreLibrary::all()`，现读 182 条 ⇒ 读数由判据自己数，⛔ 抄数字）。
//!
//! 由来（`G8d` 的教训，2026-10-28 补覆盖）：那一次实测出 MCP 侧一个**常量 16**
//! 让 `GENRES` 里 **28 条非 4/4** 流派的鼓/旋律根本生成不出来（theory 报
//! `ProgressionTooDense`）⇒ 这类"登记了却生成不出来"的缺陷必须有全库判据兜住。
//!
//! ⚠ **本文件不是第一个全库遍历**（纠正初版注释里"只有手挑夹具"的错误说法）：
//! `tests/onsets_per_bar.rs` 的 `every_non_common_time_genre_generates_at_its_own_grid`
//! 已经全库遍历，但它**跳过 4/4**、且断言的是"缺省网格 ≡ 该流派自己的 `cells_per_bar`"；
//! `verify/section_pure.rs` 的 `every_theory_genre_maps_to_a_voice_count_from_its_own_chords`
//! 全库比 `preset_parts().len()`，但**不规划**，且那是本机脚手架（不进本 crate 的 test 腿）。
//! 本判据补的差额是三者各自都不覆盖的那一面：**全库（含 4/4）× 走完整规划路径 ×
//! 三类生成物（和声声部 / 鼓 / 旋律）都非空**。
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
        harmony_seed: None,
        material_clip_ids: None,
        part_tracks: None,
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
    // R93：上面那条 `assert_eq` 在**两者都为零**时同样通过 ⇒ 被扫集合必须另有下界，
    // 否则流派表被清空后本判据会变成恒绿。
    assert!(
        genres.len() >= 100,
        "扫描面异常：登记流派只剩 {} 条",
        genres.len()
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

/// ⭐ 全库**确定性**判据（2026-10-28 补覆盖）：同一份入参的两次规划必须**逐字节相同**。
///
/// 差额（先对照存量，规则 29）：`tests/tools_e2e.rs` 的
/// `propose_section_creates_a_real_section_and_is_deterministic` 只钉了**一个夹具**
/// （`synthwave` / 8 小节 / **`project` 材料路径**），而 theory 生成路径（鼓 ＋ 旋律 ＋ 声部连接）
/// 的全库确定性**没有判据**。单位 = **流派条目**；两次规划之间不碰任何全局状态。
///
/// ⛔ 与"跨进程/跨平台确定性"混为一谈：本判据在**同一个进程内**连调两次，
/// 能抓的是每次调用自己引入的非常量（时间、`HashMap` 迭代序、全局计数器、随机源）；
/// 跨平台那条靠 CI 的两条跨平台腿各自跑同一份判据（名字在日志里逐名可见）。
#[test]
fn every_registered_genre_plans_identically_twice() {
    let project = filled_project();
    let genres = GenreLibrary::all();
    assert!(
        genres.len() >= 100,
        "扫描面异常：登记流派只剩 {} 条",
        genres.len()
    );

    let mut unstable: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for rule in genres {
        let first = plan_with_options(&project, "Chorus", rule.id, BARS, None, options());
        let second = plan_with_options(&project, "Chorus", rule.id, BARS, None, options());
        match (first, second) {
            (Ok(first), Ok(second)) => {
                compared += 1;
                if first != second {
                    unstable.push(rule.id.to_owned());
                }
            }
            (Err(first), Err(second)) => {
                // 失败也必须是同一份报文（`ADR-0036` 起各票都钉"原样转发"）。
                compared += 1;
                if first.domain_code() != second.domain_code() {
                    unstable.push(format!("{}（失败码漂移）", rule.id));
                }
            }
            (first, second) => unstable.push(format!(
                "{}（一次成功一次失败: {:?} / {:?}）",
                rule.id,
                first.err(),
                second.err()
            )),
        }
    }

    assert!(
        unstable.is_empty(),
        "同一份入参的两次规划必须逐字节相同，漂移 {} 条: {:?}",
        unstable.len(),
        unstable
    );
    // R93：以上在"流派表为空/全部一次都没比"时同样通过 ⇒ 另钉比较过的条目数。
    assert!(
        compared >= 100,
        "真正比过的流派只有 {compared} 条 —— 扫描面异常"
    );
}
