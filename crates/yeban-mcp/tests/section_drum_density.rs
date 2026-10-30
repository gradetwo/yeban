//! `ADR-0028`（`drumDensity`）的**语义**判据 —— 2026-10-28 补的覆盖缺口。
//!
//! 由来（先纠正自查时的说法，规则 29）：本键的**拒绝面**早在
//! `tests/param_bounds_guard.rs` 里登记了词表（`("drumDensity", BoundStatus::WordList)`），
//! 但**语义面**现读零覆盖 —— 全仓 `crates/yeban-mcp/tests/*.rs` 里 25 处 `drum_density`
//! **全部**是 `None`（管线占位），没有任何一条判据给过 `"sparse"`／`"normal"`。
//! 也就是说：`section_build.rs:2096` 那个 `if density == Sparse && !hit.accent { continue }`
//! 在此之前**没有判据看着**。本文件的差额就是这个。
//!
//! 口径：单位 = **流派条目**（`GenreLibrary::all()`）；每格跑**两档**
//! （`"normal"` / `"sparse"`）再比。⛔ 用本文件手挑的夹具代替全库
//! （那是 `G8d` 的教训：登记了却生成不出来的缺陷只在全库上现形）。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options,
};
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::GenreLibrary;

/// 矩阵的每格小节数：取最小非零值 —— 本判据量的是"筛点对不对"，⛔ 量"生成多久"。
const BARS: u64 = 1;

/// 只开鼓轨（本键唯一的消费面）：其余 18 个可选键一律 `None`，
/// 于是每流派读的是**自己登记**的 `primary_scale` / `progression_at(0)`。
fn options(density: Option<&'static str>) -> PlanOptions<'static> {
    PlanOptions {
        material_source: Some("theory"),
        progression: None,
        include_drums: Some(true),
        include_melody: None,
        drum_density: density,
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

/// 鼓轨的音符表：`id 规范串 → 力度`（id 由**原始** `hit_index` 派生 ⇒ 两档之间可对照，
/// 这正是 `section_build.rs:2094` 那句注释的承诺）。
fn drum_velocities(section_plan: &SectionPlan) -> BTreeMap<String, u8> {
    let mut map: BTreeMap<String, u8> = BTreeMap::new();
    for op in &section_plan.ops {
        let Op::AddClip { clip } = op else { continue };
        if !clip.name.ends_with(" · Drums") {
            continue;
        }
        let ClipContent::Midi { notes } = &clip.content else {
            continue;
        };
        for note in notes.values() {
            map.insert(note.id.to_canonical_string(), note.velocity);
        }
    }
    map
}

fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// ⭐ 判据 1：全库 × 两档。`sparse` 必须满足三条：
/// ① **非空**（静默的鼓轨不是产出）；② 音符 id **⊂** `normal` 的 id（只筛不发明）；
/// ③ 全部落在**上档**力度（因为留下来的只有重音）。
///
/// ⭐ 判据 2（同一循环里的防空）：⛔ 只钉"是子集"—— 全库若一个点都没被筛掉，
/// "sparse" 这个名字就是假的，而子集关系**恒成立**（R93 同族：空/恒真断言不算判据）。
#[test]
fn sparse_keeps_only_the_accented_hits_of_every_registered_genre() {
    let project = filled_project();
    let genres = GenreLibrary::all();
    assert_eq!(
        genres.len(),
        GenreLibrary::len(),
        "`all()` 与 `len()` 必须同源（⛔ 一条为空而另一条非空）"
    );
    // R93：上面那条在**双零**时同样通过 ⇒ 被扫集合必须另有下界。
    assert!(
        genres.len() >= 100,
        "扫描面异常：登记流派只剩 {} 条",
        genres.len()
    );

    let mut failures: Vec<String> = Vec::new();
    let mut empty_sparse: Vec<String> = Vec::new();
    let mut not_subset: Vec<String> = Vec::new();
    let mut mixed_tiers: Vec<String> = Vec::new();
    let mut planned = 0usize;
    let mut strict_decrease = 0usize;
    let mut sparse_notes_total = 0usize;

    for rule in genres {
        let normal = plan_with_options(
            &project,
            "Chorus",
            rule.id,
            BARS,
            None,
            options(Some("normal")),
        );
        let sparse = plan_with_options(
            &project,
            "Chorus",
            rule.id,
            BARS,
            None,
            options(Some("sparse")),
        );
        let (normal, sparse) = match (normal, sparse) {
            (Ok(normal), Ok(sparse)) => (normal, sparse),
            (Err(fault), _) => {
                failures.push(format!("{}: normal 档失败 {fault:?}", rule.id));
                continue;
            }
            (Ok(_), Err(fault)) => {
                // 登记的拒绝面：流派一个重音都没有 ⇒ 响亮拒绝，且载荷指向本键。
                if fault.domain_code() != Some(BuildCode::InvalidParameterRange) {
                    failures.push(format!("{}: sparse 档失败的码不对 {fault:?}", rule.id));
                    continue;
                }
                let payload = data(&fault);
                let hint = payload["hint"].as_str().unwrap_or_default();
                if !hint.contains("drumDensity") {
                    failures.push(format!(
                        "{}: sparse 档的 `hint` 未点名 `drumDensity`（载荷 {payload}）",
                        rule.id
                    ));
                }
                continue;
            }
        };
        planned += 1;

        let normal_notes = drum_velocities(&normal);
        let sparse_notes = drum_velocities(&sparse);
        if sparse_notes.is_empty() {
            empty_sparse.push(rule.id.to_owned());
            continue;
        }
        sparse_notes_total += sparse_notes.len();

        let normal_ids: BTreeSet<&String> = normal_notes.keys().collect();
        let extra: Vec<&String> = sparse_notes
            .keys()
            .filter(|id| !normal_ids.contains(id))
            .collect();
        if !extra.is_empty() {
            not_subset.push(format!(
                "{}: 多出 {} 条（如 {:?}）",
                rule.id,
                extra.len(),
                extra.first()
            ));
        }

        // ③ 留下来的只有重音 ⇒ 力度只有一档，且它必须等于 `normal` 里的**上档**。
        let tiers: BTreeSet<u8> = sparse_notes.values().copied().collect();
        let normal_top = normal_notes.values().copied().max();
        match (tiers.len(), normal_top) {
            (1, Some(top)) if tiers.contains(&top) => {}
            (1, _) => mixed_tiers.push(format!(
                "{}: sparse 的力度 {:?} ⛔ 等于 normal 的上档 {normal_top:?}",
                rule.id, tiers
            )),
            (n, _) => mixed_tiers.push(format!(
                "{}: sparse 里出现了 {n} 档力度 {:?}（只剩重音 ⇒ 应当只有一档）",
                rule.id, tiers
            )),
        }

        // ② 防空：正常档的点数**严格**多于稀疏档（至少有一条流派上被筛掉过，
        //    这条在循环外另有汇总断言；这里只数）。
        if sparse_notes.len() < normal_notes.len() {
            strict_decrease += 1;
        }
    }

    assert!(
        failures.is_empty(),
        "全库两档规划必须成功（或按登记的拒绝面报 `INVALID_PARAMETER_RANGE`），失败 {} 条: {:?}",
        failures.len(),
        failures
    );
    assert!(
        empty_sparse.is_empty(),
        "`sparse` 生成不出任何鼓点（{} 条）：静默的鼓轨不是产出: {:?}",
        empty_sparse.len(),
        empty_sparse
    );
    assert!(
        not_subset.is_empty(),
        "`sparse` 的音符必须是 `normal` 的子集（只筛、不发明），越界 {} 条: {:?}",
        not_subset.len(),
        not_subset
    );
    assert!(
        mixed_tiers.is_empty(),
        "`sparse` 留下的只有重音 ⇒ 力度必须只有一档且等于 `normal` 的上档，违例 {} 条: {:?}",
        mixed_tiers.len(),
        mixed_tiers
    );
    // R93：以上四条在"两档都返回同一份非空结果"时**同样**通过 ⇒ 必须另钉
    // "筛确实发生过"这一面，否则 `sparse` 退化成 `normal` 时本判据仍然恒绿。
    assert!(
        planned >= 100,
        "两档都规划成功的流派只有 {planned} 条 —— 扫描面异常"
    );
    assert!(
        sparse_notes_total >= 100,
        "全库 `sparse` 合计只有 {sparse_notes_total} 条鼓点 —— 扫描面异常"
    );
    assert!(
        strict_decrease >= 1,
        "全库 **没有一条**流派的 `sparse` 比 `normal` 少（{planned} 条全相等）⇒ \
         `sparse` 已退化成 `normal`，本键名不副实"
    );
}

/// ⭐ 判据 3（消费面，两向都钉，口径与 `swingPermille` / `drumStyle` / `drumBackbeat` 同一支）：
/// 本键的消费路径**只有** theory 生成的鼓轨 ⇒ 工程材料路径上给它就是"给了不消费的参数"。
/// ⚠ 实际承载这条拒绝的是 `includeDrums` 的耦合（`section_build.rs:2309`，先于本键那行），
/// 本判据钉的是**行为**（转义不了），⛔ 钉报文的归属。
#[test]
fn the_density_key_requires_the_theory_source_and_the_drum_part() {
    let project = filled_project();
    // ⭐ 审计 `N1` 正例：`'project'` 源 + 鼓轨在场 ⇒ `drumDensity` 被消费 ⇒ 放行。
    for material_source in [None, Some("project")] {
        let mut given = options(Some("sparse"));
        given.material_source = material_source;
        plan_with_options(&project, "Chorus", "synthwave", BARS, None, given).unwrap_or_else(
            |fault| panic!("project 源 + 鼓轨在场时 `drumDensity` 必须放行 [审计 N1]: {fault:?}"),
        );
    }

    // 鼓轨不在场 ⇒ 本键无人消费，必须响亮拒绝而不是静默忽略。
    let mut no_drums = options(Some("sparse"));
    no_drums.include_drums = Some(false);
    let fault = plan_with_options(&project, "Chorus", "synthwave", BARS, None, no_drums)
        .expect_err("`drumDensity` 不得在没有鼓轨时静默失效");
    assert_eq!(
        fault.domain_code(),
        Some(BuildCode::InvalidParameterRange),
        "必须报 `INVALID_PARAMETER_RANGE`: {fault:?}"
    );
    assert!(
        data(&fault)["hint"]
            .as_str()
            .is_some_and(|text| text.contains("includeDrums")),
        "载荷必须指出要补 includeDrums: {}",
        data(&fault)
    );

    // 反钉：没有旋律声部时必须放行（筛点只被鼓轨消费，与旋律无关）。
    plan_with_options(&project, "Chorus", "synthwave", BARS, None, options(None))
        .unwrap_or_else(|fault| panic!("`drumDensity` 缺省时必须放行: {fault:?}"));
}

/// ⭐ 判据 4：词表外的值必须响亮拒绝，且**两档都放行**（⛔ 只测 `"sparse"`——
/// 那会把 `"normal"` 的解析路径留空）。`accepted` 的取值由本判据现场对账：
/// 里面列出的每一项都必须真的能被规划成功（⛔ 只比字符串）。
#[test]
fn unknown_density_values_are_rejected_with_a_usable_accepted_list() {
    let project = filled_project();
    let fault = plan_with_options(
        &project,
        "Chorus",
        "synthwave",
        BARS,
        None,
        options(Some("dense")),
    )
    .expect_err("`drumDensity: \"dense\"` 不在词表里，必须拒绝");
    assert_eq!(
        fault.domain_code(),
        Some(BuildCode::InvalidParameterRange),
        "必须报 `INVALID_PARAMETER_RANGE`: {fault:?}"
    );
    let payload = data(&fault);
    let accepted: Vec<String> = payload["accepted"]
        .as_array()
        .unwrap_or_else(|| panic!("载荷必须带 `accepted` 列表: {payload}"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("`accepted` 里必须全是字符串: {payload}"))
                .to_owned()
        })
        .collect();
    for word in ["normal", "sparse"] {
        assert!(
            accepted.iter().any(|value| value == word),
            "`accepted` 必须包含 {word:?}（现读 {accepted:?}）"
        );
    }
    // `accepted` 的每一项都必须**真的**能被规划 ⇒ 它是一份可用的候选，⛔ 一句空话。
    for word in &accepted {
        let leaked: &'static str = match word.as_str() {
            "normal" => "normal",
            "sparse" => "sparse",
            other => {
                panic!("`accepted` 里出现了本判据不认识的候选 {other:?} —— 词表变了就更新这条判据")
            }
        };
        plan_with_options(
            &project,
            "Chorus",
            "synthwave",
            BARS,
            None,
            options(Some(leaked)),
        )
        .unwrap_or_else(|fault| panic!("`accepted` 里的 {word:?} 必须放行: {fault:?}"));
    }
}

/// ⭐ 判据 4（**缺省侧**，`ADR-0028`）：不给 `drumDensity` 与显式 `"normal"` **逐点同形**。
///
/// 消费点 `DrumDensity::parse` 的第一臂就是 `None | Some("normal")`，此前全库没有一条判据
/// 把 `None` 那半支与 `"normal"` 比过（判据 1 只比 `normal` ↔ `sparse`）。
/// 全库 × 每流派各跑两次，鼓轨的 `id → velocity` 表必须**逐键相等**；
/// 另钉"真正比过表的流派数"下界（R93：两边都为空时相等是恒真的）。
#[test]
fn absent_density_is_identical_to_explicit_normal_on_every_genre() {
    let project = filled_project();
    let genres = GenreLibrary::all();
    assert!(
        genres.len() >= 100,
        "扫描面异常：登记流派只剩 {} 条",
        genres.len()
    );

    let mut failures: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for rule in genres {
        let absent = plan_with_options(&project, "Chorus", rule.id, BARS, None, options(None));
        let normal = plan_with_options(
            &project,
            "Chorus",
            rule.id,
            BARS,
            None,
            options(Some("normal")),
        );
        match (absent, normal) {
            (Ok(absent), Ok(normal)) => {
                let absent_notes = drum_velocities(&absent);
                let normal_notes = drum_velocities(&normal);
                if absent_notes != normal_notes {
                    failures.push(format!(
                        "{}: 缺省与 normal 的鼓点表不同 ({} vs {} 点)",
                        rule.id,
                        absent_notes.len(),
                        normal_notes.len()
                    ));
                } else if !normal_notes.is_empty() {
                    compared += 1;
                }
            }
            (Err(fault), _) => failures.push(format!("{}: 缺省档失败 {fault:?}", rule.id)),
            (Ok(_), Err(fault)) => failures.push(format!("{}: normal 档失败 {fault:?}", rule.id)),
        }
    }
    assert!(
        failures.is_empty(),
        "缺省钉不符 {} 条: {:?}",
        failures.len(),
        failures
    );
    assert!(
        compared >= 100,
        "真正比过非空鼓点表的流派只有 {compared} 条 —— 判据在空表上恒绿"
    );
}
