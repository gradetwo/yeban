//! `审计 N3`：`yeban_propose_section` 的 `harmonySeed` —— 和声**编排种子**。
//! 缺省 ⇒ 走向取流派 `progression_at(0)`、音阶取 `primary_scale`（逐字节不变）；
//! 给了 ⇒ 未指名的槽位改由 `GenreRule::progression_for(seed)` /
//! `GenreRule::scale_for(tonic, seed)` 抽取（同一 `seed` 喂两条互相独立的盐
//! ⇒ 走向与音阶各自换一条登记项）。
//!
//! ⚠ 两个槽位**都**被 `progression` + `scale` 指名时种子无可消费 ⇒ 响亮拒
//! （"给了必须被消费"）；`drumSeed` 是刻意缺席项 —— theory 的鼓生成
//! （`swung_styled_drum_pattern`）全量确定性、⛔ 种子入参，本键⛔ 兼作鼓种子。
//!
//! ⚠ 分层如实：`-1` / `1.5` / `"7"` 的工具面拦截走 `arg_u64` +
//! `ParamSpec::accepts("integer")` —— 与 `melodySeed` 同一调用路径，已由
//! `section_melody_seed.rs` 判据 4 覆盖 ⇒ 本文件⛔ 重复取样工具面类型闸。

use std::collections::BTreeMap;

use serde_json::Value;
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options,
};
use yeban_model::{ClipContent, EntityId, Op, samples::filled_project};

/// 夹具的小节数。
const BARS: u64 = 4;
/// 夹具的风格预设：4/4，登记了多条走向与多个音阶（`seeds_cover_every_registered`）。
const STYLE: &str = "synthwave";

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`。
type Note = (u64, u64, u8, u8);

/// 一次请求的可给项。
#[derive(Clone, Copy, Default)]
struct Given {
    material_source: Option<&'static str>,
    progression: Option<&'static str>,
    scale: Option<&'static str>,
    harmony_seed: Option<u64>,
}

impl Given {
    /// 夹具的基线：`materialSource` 缺省 = `project` ⇒ 本文件除"耦合门"
    /// 用例外全部显式给 `"theory"`。
    fn theory() -> Self {
        Self {
            material_source: Some("theory"),
            ..Self::default()
        }
    }
}

fn try_plan(given: Given) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        &filled_project(),
        "Chorus",
        STYLE,
        BARS,
        given.scale,
        PlanOptions {
            material_source: given.material_source,
            progression: given.progression,
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
            harmony_seed: given.harmony_seed,
            material_clip_ids: None,
            part_tracks: None,
            beat_grouping: None,
        },
    )
}

fn plan(given: Given) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| panic!("theory 规划必须成功: {fault:?}"))
}

/// 按**声部名**索引的音符表（声部名取 `"{section} · {part}"` 的后半）。
fn voices(section_plan: &SectionPlan) -> BTreeMap<String, Vec<Note>> {
    let mut map: BTreeMap<String, Vec<Note>> = BTreeMap::new();
    for op in &section_plan.ops {
        let Op::AddClip { clip } = op else {
            continue;
        };
        let ClipContent::Midi { notes } = &clip.content else {
            continue;
        };
        let part = clip
            .name
            .rsplit(" · ")
            .next()
            .unwrap_or(clip.name.as_str())
            .to_owned();
        let table = map.entry(part).or_default();
        for note in notes.values() {
            table.push((
                note.start_tick,
                note.duration_ticks,
                note.pitch,
                note.velocity,
            ));
        }
    }
    for table in map.values_mut() {
        table.sort_unstable();
    }
    map
}

/// 失败载荷的 `data`。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 判据 1：同一组参数给同一个种子 ⇒ 逐字节同形（确定性）；
/// 且与 `melodySeed` 不同 —— 换的是**和声声部的音高表**（走向条目换了）。
#[test]
fn harmony_seed_is_deterministic_and_swaps_the_progression_pick() {
    let seeds: [u64; 8] = [0, 1, 2, 3, 42, 1_000_003, 0xdead_beef, u64::MAX];
    let mut distinct: Vec<Vec<Note>> = Vec::new();
    for seed in seeds {
        let given = Given {
            harmony_seed: Some(seed),
            ..Given::theory()
        };
        let first = plan(given);
        let second = plan(given);
        assert_eq!(
            voices(&first),
            voices(&second),
            "种子 {seed} 的两次规划必须逐字节同形"
        );
        // 和声声部 = 排除鼓/旋律外全部声部（夹具两个键都不给 ⇒ parts 全是和声）。
        let harmony: Vec<Note> = voices(&first).into_values().flatten().collect();
        if !distinct.contains(&harmony) {
            distinct.push(harmony);
        }
    }
    assert!(
        distinct.len() >= 2,
        "8 个种子里至少要有两个**不同的**和声结果（`seeds_cover_every_registered` 保证种子空间能取到全部登记走向）"
    );
    // 键缺席的基线：两次规划同形（原有不变量，顺带守住）。
    let base_a = plan(Given::theory());
    let base_b = plan(Given::theory());
    assert_eq!(voices(&base_a), voices(&base_b));
}

/// 判据 2：耦合门三类，各自响亮且载荷⛔ 带 `reason` 键（⛔ 新词）。
#[test]
fn harmony_seed_gates_are_loud() {
    // 2a. `materialSource: 'project'` + 种子 ⇒ 材料路径不读走向 ⇒ 拒。
    let fault = try_plan(Given {
        material_source: Some("project"),
        harmony_seed: Some(7),
        ..Given::default()
    })
    .expect_err("project 材料源 + harmonySeed 必须拒绝");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    assert!(data(&fault).get("reason").is_none());
    assert!(data(&fault).get("hint").is_some());

    // 2b. `progression` + `scale` 都指名 + 种子 ⇒ 无可消费槽位 ⇒ 拒。
    let fault = try_plan(Given {
        progression: Some("I-V-vi-IV"),
        scale: Some("C major"),
        harmony_seed: Some(7),
        ..Given::theory()
    })
    .expect_err("两槽位都指名 + harmonySeed 必须拒绝");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    assert!(data(&fault).get("reason").is_none());
    assert!(data(&fault).get("hint").is_some());

    // 2c. 只指名 `progression`（scale 缺席 ⇒ 仍有可消费槽位）⇒ 放行。
    plan(Given {
        progression: Some("I-V-vi-IV"),
        harmony_seed: Some(7),
        ..Given::theory()
    });
}

/// 判据 3：种子抽到的走向/音阶文本**进入段落身份**（`scale_key` =
/// `"{scale}:theory:{progression}"` ⇒ 换种子的每"一版"是**另一个** section id，
/// 音符 id 跟着换）—— 这⛔ 是缺陷而是既有约定：同一文本 ⇒ 同一 id。
/// 断言 = 换种子换来的不同和声结果，其 section id 也**确实**换了（⛔ 撞身份）。
#[test]
fn a_different_pick_is_a_different_section_identity() {
    let id_of = |seed: u64| {
        plan(Given {
            harmony_seed: Some(seed),
            ..Given::theory()
        })
        .ops
        .iter()
        .find_map(|op| match op {
            Op::SetSection { section_id, .. } => Some(*section_id),
            _ => None,
        })
        .expect("规划结果必须带一条 `SetSection`")
    };
    let ids: Vec<EntityId> = [0u64, 1, 2, 3, 42, 1_000_003]
        .iter()
        .map(|seed| id_of(*seed))
        .collect();
    // 同种子两次 ⇒ 同 id（确定性，顺带）。
    assert_eq!(id_of(0), ids[0]);
    // 判据 1 已证有 ≥2 种和声结果 ⇒ 这里只断言"不同结果 ⛔ 挤在同一个身份里"：
    // 6 个种子的身份集合 ≥ 2（走向只有 2 条时上界也是 2，⛔ 判相等数）。
    let mut distinct = ids.clone();
    distinct.sort();
    distinct.dedup();
    assert!(
        distinct.len() >= 2,
        "换种子换来的不同走向必须落在不同段落身份上: {ids:?}"
    );
}
