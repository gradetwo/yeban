//! 本机零重依赖验证脚手架 —— `yeban_propose_section` 的**真**骨架生成器 [MCP-TOOL-005]。
//!
//! 它**不是** cargo 目标（不在 `tests/` 下）：`yeban-mcp` 一旦依赖
//! `yeban-render` / `yeban-decode`（rayon/hound/midly/symphonia/rubato）就无法在本机编译
//! （`AGENTS.md` §5 的本机纪律），而"骨架长什么样、声部连到哪、逆操作能不能逐字节回退"
//! 这几件事只依赖 `yeban-model` + `serde_json`。
//!
//! ```text
//! bash scripts/dev/cargo-local.sh build -p yeban-model
//! bash scripts/dev/cargo-local.sh build -p yeban-theory   # `line/theory-wiring` 新增的依赖边
//! DEPS=target/debug/deps
//! YM=$(ls $DEPS/libyeban_model-*.rlib | head -1)
//! JS=$(ls $DEPS/libserde_json-*.rlib | head -1)
//! YT=$(ls $DEPS/libyeban_theory-*.rlib | head -1)
//! rustc --edition 2024 --test -D warnings crates/yeban-mcp/verify/section_pure.rs \
//!   --extern yeban_model=$YM --extern serde_json=$JS --extern yeban_theory=$YT \
//!   -L dependency=$DEPS -o /tmp/section_pure
//! /tmp/section_pure
//! ```
//!
//! `--extern yeban_theory=…` 不是可选项：本文件用 `#[path]` 引入的
//! `src/domain/section_build.rs` 现在就 `use yeban_theory::…`（`ADR-0001` D49 的接线），
//! 少这一个 `--extern` 连编译都过不去 —— 这也是判据 ⑥「依赖边真的存在」的**编译级**证据。
//!
//! 关键点：本文件用 `#[path]` 引入**真实源文件**（`src/domain/section_build.rs` 与
//! `src/domain/ids.rs`），不是抄一份会漂移的副本 —— 本机跑过的就是 CI 跑的那一份代码。
//! `section_build.rs` 自带的 `#[cfg(test)] mod tests` 会随 `--test` 一起编译执行，
//! 因此 ①–⑧ 的主体判据在本机**真的**跑；本文件在此之上加三条**独立口径**的对抗性判据：
//!
//! - `routing_edges_are_never_cyclic`：生成的连接图必须是 DAG（自环/回边直接红）；
//! - `skeleton_ids_are_never_nil_and_are_distinct`：任何新身份都不得是 nil、
//!   同一批里不得撞车、派生音符身份不得复用源材料身份（"凭空造 id" 最危险的形态
//!   就是造出 nil、撞车或与既有实体同名）；
//! - `missing_material_reports_the_typed_fault`：缺材料必须是**类型化**的领域失败。
//!
//! `ADR-0001` **D49**（`line/theory-wiring`）之后再加三条**独立口径**的接线判据 ——
//! 它们全部**直接问 theory**（`GenreLibrary` / `ScaleKind` / `ChordSpan`），不读本模块的
//! 中间量，因此"把 theory 的查询换成硬编码常量"这类注入会在这里变红：
//!
//! - `dorian_and_minor_differ_exactly_by_the_sixth`：`D dorian` / `D minor` 的逐音输出
//!   （写死期望值）；
//! - `every_theory_genre_maps_to_a_voice_count_from_its_own_chords`：182 条流派逐条比对
//!   "声部数 = theory 给的和弦构成音数"；
//! - `the_preset_catalogue_is_the_theory_library`：候选清单**就是** `GenreLibrary::ids()`。
//!
//! `ADR-0016`（`materialSource: "theory"`）再加三条判据：
//!
//! - `theory_is_explicitly_opt_in_and_project_path_is_unchanged`：缺省仍是 `'project'`
//!   （空池 ⇒ `CLIP_NOT_FOUND`），显式 `'project'` 逐字节等于缺省路径；
//! - `theory_generates_a_deterministic_skeleton_on_an_empty_pool`：空池上生成 3/4 声部，
//!   起点/时值对齐 theory 自己展开的 `ChordSpan`、力度固定 96、重复调用逐字节相同、
//!   声部连接是 DAG；
//! - `theory_honours_custom_progression_and_scale`：`progression` / `scale` 真的进展开
//!   （时值与身份写死断言），非法走向映射成 `INVALID_PARAMETER_RANGE`。
//!
//! `ADR-0017`（`includeDrums` / `includeMelody`）再加四条判据：
//!
//! - `drums_are_gm_mapped_and_route_to_master_in_a_dag`：鼓点逐击对齐 theory 的
//!   `DrumPattern`（GM 键 / 起点 / 力度 / 时值），且直连主总线、整图 DAG；
//! - `melody_stays_on_the_genre_grid_and_in_range_in_a_dag`：旋律起点/时值逐 onset
//!   对齐 `rhythm_grid`、音高在默认音域与目标音阶内、力度按 `chord_tone` 分级；
//! - `drums_and_melody_are_opt_in_and_legacy_plans_are_byte_identical`：缺省 / 显式
//!   `false` 逐字节等于既有行为，`true` 只追加两条声部；
//! - `drums_and_melody_require_the_theory_source`：`'project'` 路径上给任一项 ⇒
//!   `INVALID_PARAMETER_RANGE`（带指出修复方式的 `hint`）。
//!
//! `ADR-0028`（`drumDensity` / 3 声部低音区）再加四条判据：
//!
//! - `drum_density_normal_is_byte_identical_to_absent`：显式 `"normal"` 逐字节等于键缺席；
//! - `sparse_drums_are_exactly_the_accented_hits`：`sparse` 逐击等于 theory 鼓组型里
//!   登记为重音的子集（theory 侧独立重算），计数严格更小、身份是 normal 的真子集；
//! - `drum_density_requires_include_drums_and_rejects_unknown_values`：脱离
//!   `includeDrums:true` 拒绝；未知取值拒绝并列出 `accepted`；
//! - `three_voice_bass_sits_in_the_low_register`：最低声部全部落在 C2–C3，声部按音区成梯次。

// 本文件是 `rustc --test` 的 **crate root**, 不是库; 被包含模块的公开 API 在这里
// 没有"外部消费者", 因此 `dead_code` 会误报。真实判定由 CI 的
// `cargo clippy -p yeban-mcp --all-targets -- -D warnings` 在 lib crate 上执行。
#![allow(dead_code)]

#[path = "../src/domain/ids.rs"]
mod ids;
#[path = "../src/domain/section_build.rs"]
mod section_build;

use std::collections::BTreeMap;

use yeban_model::{EntityId, Op, RoutingGraph, TrackKind, samples::filled_project};

use section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, detect_cycle, plan, plan_with_options,
};

/// 把一份规划的整批 op 施加到样本工程的克隆体上。
fn applied(
    project: &yeban_model::YebanProjectV1,
    plan: &SectionPlan,
) -> yeban_model::YebanProjectV1 {
    let mut after = project.clone();
    Op::Batch {
        ops: plan.ops.clone(),
        description: "harness".to_owned(),
    }
    .apply(&mut after)
    .expect("整批必须能施加");
    after
}

/// 空 `clip_pool` 的工程夹具（`ADR-0016` 的从零创作场景）。
///
/// 只清 `clip_pool` 会留下悬空摆放（`validate()` 以 `ClipNotFound` 拒绝），
/// 因此音轨上的摆放必须一起清掉。
fn empty_pool_project() -> yeban_model::YebanProjectV1 {
    let mut project = filled_project();
    project.clip_pool.clear();
    for track in project.tracks.values_mut() {
        track.clips.clear();
    }
    project.validate().expect("空材料池夹具必须合法");
    project
}

#[test]
fn routing_edges_are_never_cyclic() {
    let project = filled_project();
    let plan = plan(&project, "Chorus", "synthwave", 4, Some("C minor")).expect("规划");
    let after = applied(&project, &plan);

    // 声部连接不得引入任何环（判据独立于 section_build 内部的 DFS 实现）。
    for (index, edge_id) in plan.routing_edge_ids.iter().enumerate() {
        let edge = after
            .routing_graph
            .edges
            .get(edge_id)
            .unwrap_or_else(|| panic!("第 {index} 条边必须存在"));
        assert_ne!(edge.source_node, edge.destination_node, "自环: {edge:?}");
    }
    let mut edges: BTreeMap<EntityId, yeban_model::RoutingEdge> = BTreeMap::new();
    for (id, edge) in &after.routing_graph.edges {
        if plan.routing_edge_ids.contains(id) {
            edges.insert(*id, *edge);
        }
    }
    let graph = RoutingGraph {
        nodes: after.routing_graph.nodes.clone(),
        edges,
    };
    assert!(
        detect_cycle(&graph).is_none(),
        "声部连接必须是 DAG: {:?}",
        detect_cycle(&graph)
    );
}

#[test]
fn skeleton_ids_are_never_nil_and_are_distinct() {
    let project = filled_project();
    let plan = plan(&project, "Chorus", "orchestral_film_score", 2, None).expect("规划");
    // **新建实体**的身份：段落 / 片段池条目 / 摆放 / 音轨 / 路由边。
    let mut minted: Vec<EntityId> = vec![plan.section_id];
    minted.extend(plan.part_track_ids.iter().copied());
    minted.extend(plan.part_clip_ids.iter().copied());
    minted.extend(plan.placement_ids.iter().copied());
    minted.extend(plan.routing_edge_ids.iter().copied());
    for id in &minted {
        assert!(!id.is_nil(), "身份不得是 nil");
    }
    for (index, id) in minted.iter().enumerate() {
        assert!(
            !minted[index + 1..].contains(id),
            "同一批里的身份必须互不相同: {id}"
        );
    }
    // 路由节点表里的身份**必须**是声部音轨本身（+ 可能的主总线）——
    // 不得凭空造出第三类节点身份。
    let bus = project.master_bus_track_id;
    for node in &plan.added_routing_nodes {
        assert!(
            plan.part_track_ids.contains(node) || *node == bus,
            "路由节点必须是真实音轨身份: {node}"
        );
    }
    for (index, node) in plan.added_routing_nodes.iter().enumerate() {
        assert!(
            !plan.added_routing_nodes[index + 1..].contains(node),
            "路由节点不得重复: {node}"
        );
    }

    // 生成出来的**音符**身份也不得与材料里的音符撞车（逐片段独立）。
    let after = applied(&project, &plan);
    let source_ids: Vec<EntityId> = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.keys().copied())
        .collect();
    for clip_id in &plan.part_clip_ids {
        let notes = after.clip_pool[clip_id]
            .content
            .notes()
            .expect("生成的片段必须是 MIDI");
        assert!(!notes.is_empty(), "生成的片段必须带材料");
        for id in notes.keys() {
            assert!(
                !source_ids.contains(id),
                "派生音符身份不得复用源材料的身份: {id}"
            );
        }
    }
}

#[test]
fn missing_material_reports_the_typed_fault() {
    let mut project = filled_project();
    project.clip_pool.clear();
    let fault = plan(&project, "Chorus", "lo_fi_hip_hop", 4, None).expect_err("无材料");
    assert_eq!(fault.domain_code(), Some(BuildCode::ClipNotFound));
    assert!(matches!(fault, BuildFault::Domain { .. }));
}

/// 判据 ①（独立口径）：调式语义真的进了输出，且 `D dorian` / `D minor` 的差异**恰好**
/// 是第六级 —— 期望值写死，不由 `section_build` 的任何函数算出来。
#[test]
fn dorian_and_minor_differ_exactly_by_the_sixth() {
    // 材料: D4(62) / Bb4(70) / B4(71) / F4(65) / D5(74)，最低音音级 = 2 ⇒ 目标 D 时移调 = 0。
    let mut project = filled_project();
    let material_id = project
        .clip_pool
        .values()
        .find(|entry| entry.content.notes().is_some())
        .map(|entry| entry.id)
        .expect("样本必须有 MIDI 材料");
    let entry = project.clip_pool.get_mut(&material_id).expect("材料");
    let mut notes = BTreeMap::new();
    for (index, (pitch, tick)) in [(62u8, 0u64), (70, 480), (71, 960), (65, 1440), (74, 1920)]
        .into_iter()
        .enumerate()
    {
        let id = ids::deterministic_id(&format!("harness-note:{index}"));
        notes.insert(id, yeban_model::MidiNote::new(id, tick, pitch, 480));
    }
    let Some(slot) = entry.content.notes_mut() else {
        panic!("材料必须是 MIDI");
    };
    *slot = notes;
    project.validate().expect("夹具必须合法");

    let pitches = |scale: &str| -> Vec<u8> {
        let plan = plan(&project, "Verse", "lo_fi_hip_hop", 2, Some(scale)).expect("规划");
        let after = applied(&project, &plan);
        let mut values: Vec<u8> = after.clip_pool[&plan.part_clip_ids[0]]
            .content
            .notes()
            .expect("MIDI")
            .values()
            .map(|note| note.pitch)
            .collect();
        values.sort_unstable();
        values
    };
    assert_eq!(pitches("D dorian"), vec![62, 65, 69, 71, 74], "D dorian");
    assert_eq!(pitches("D minor"), vec![62, 65, 70, 70, 74], "D minor");
}

/// 判据 ②（独立口径）：182 条 theory 流派逐条比对 —— 声部数必须等于
/// **theory 自己展开的**和弦构成音数。把 `preset_parts` 换成硬编码常量即红。
#[test]
fn every_theory_genre_maps_to_a_voice_count_from_its_own_chords() {
    use yeban_theory::genre::GenreLibrary;
    use yeban_theory::pitch::PitchClass;

    let mut seen: Vec<usize> = Vec::new();
    for rule in GenreLibrary::all() {
        let scale = rule.primary_scale(PitchClass::C).expect("theory 音阶");
        let expected = rule
            .typical_progressions
            .iter()
            .map(|text| yeban_theory::progression::Progression::parse(text).expect("theory 走向"))
            .flat_map(|progression| progression.chords(&scale))
            .map(|chord| chord.pitch_classes().len())
            .max()
            .expect("至少一个和弦");
        let parts = section_build::preset_parts(rule.id).expect("合法预设");
        assert_eq!(parts.len(), expected, "流派 {}", rule.id);
        if !seen.contains(&expected) {
            seen.push(expected);
        }
    }
    seen.sort_unstable();
    assert_eq!(seen, vec![3, 4], "theory 全库只给出 3 与 4 两种构成音数");
}

/// 判据 ③（独立口径）：候选清单**就是** theory 的流派 ID 清单。
#[test]
fn the_preset_catalogue_is_the_theory_library() {
    use yeban_theory::genre::GenreLibrary;

    assert_eq!(section_build::available_presets(), GenreLibrary::ids());
    let mut project = filled_project();
    project.clip_pool.clear();
    let fault = plan(&project, "Chorus", "yeban_unknown_style", 4, None).expect_err("未知风格");
    assert_eq!(fault.domain_code(), Some(BuildCode::StyleNotFound));
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败");
    };
    assert_eq!(
        data["availablePresets"],
        serde_json::json!(GenreLibrary::ids())
    );
}

/// `ADR-0016` D2：`'theory'` 是**显式** opt-in。
///
/// - 缺省路径（`materialSource` 缺省）在空池上仍然报 `CLIP_NOT_FOUND`；
/// - 显式 `'project'` 忽略 `progression`，且逐字节等于缺省路径；
/// - 未知来源是契约内错误，并回显已接受取值。
#[test]
fn theory_is_explicitly_opt_in_and_project_path_is_unchanged() {
    let empty = empty_pool_project();
    let fault = plan(&empty, "Chorus", "synthwave", 4, None).expect_err("缺省路径必须仍报缺料");
    assert_eq!(fault.domain_code(), Some(BuildCode::ClipNotFound));

    let project = filled_project();
    let default = plan(&project, "Chorus", "synthwave", 4, None).expect("缺省路径");
    let explicit = plan_with_options(
        &project,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("project"),
            progression: Some("I-V"),
            include_drums: None,
            include_melody: None, drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("显式 project");
    assert_eq!(
        default, explicit,
        "`'project'` 必须忽略 progression 并逐字节等于缺省路径"
    );

    let fault = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("improvisation"),
            progression: None,
            include_drums: None,
            include_melody: None, drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect_err("未知材料来源");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败");
    };
    assert_eq!(data["materialSource"], "improvisation");
    assert_eq!(data["accepted"], serde_json::json!(["project", "theory"]));
}

/// `ADR-0016` D2/D3/D4（独立口径）：空 `clip_pool` 上 `'theory'` 真的生成
/// 3/4 声部骨架。期望的和弦跨度由 theory **自己**展开（写死 4/4 + 4 小节），
/// 逐音断言起点/时值/力度与和弦音归属；同一请求逐字节可重复，
/// 且生成结果与 `clip_pool` 内容无关（理论路径不读材料池）。
#[test]
fn theory_generates_a_deterministic_skeleton_on_an_empty_pool() {
    use yeban_theory::genre::GenreLibrary;
    use yeban_theory::pitch::PitchClass;
    use yeban_theory::progression::{Meter, Progression};

    let empty = empty_pool_project();
    let filled = filled_project();
    for (style, voice_count) in [("synthwave", 3usize), ("funk", 4)] {
        let options = PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: None,
            include_melody: None, drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        };
        let planned =
            plan_with_options(&empty, "Chorus", style, 4, None, options).expect("理论生成");
        assert_eq!(planned.part_track_ids.len(), voice_count, "{style} 的声部数");
        assert_eq!(planned.part_clip_ids.len(), voice_count);
        assert_eq!(planned.placement_ids.len(), voice_count);
        assert_eq!(planned.routing_edge_ids.len(), voice_count);
        assert!(
            planned.unwired.is_empty(),
            "两条材料路径的装配必须同构: {:?}",
            planned.unwired
        );

        // 确定性：同一请求重复调用逐字节相同；空池与满池产出同一份 op 序列
        // （理论路径**不得**读取 `clip_pool`）。
        let repeated =
            plan_with_options(&empty, "Chorus", style, 4, None, options).expect("重复调用");
        assert_eq!(planned, repeated, "{style}: 同一请求必须产出同一份规划");
        let from_filled = plan_with_options(&filled, "Chorus", style, 4, None, options)
            .expect("满池理论生成");
        assert_eq!(
            planned.ops, from_filled.ops,
            "{style}: 理论路径不得读取 clip_pool"
        );

        // 独立口径：期望的 `ChordSpan` 由 theory 展开（C 调、4/4、4 小节）。
        let rule = GenreLibrary::get(style).expect("theory 流派");
        let scale = rule.primary_scale(PitchClass::C).expect("theory 音阶");
        let spans = Progression::parse(rule.progression_at(0).expect("首选走向"))
            .expect("theory 走向")
            .with_meter(Meter {
                numerator: 4,
                denominator: 4,
            })
            .with_bars(4)
            .expect("4 小节")
            .expand(&scale)
            .expect("展开");
        assert!(!spans.is_empty());

        let after = applied(&empty, &planned);
        after.validate().expect("施加后必须合法");
        let mut minted_notes: Vec<EntityId> = Vec::new();
        for (index, (clip_id, track_id)) in planned
            .part_clip_ids
            .iter()
            .zip(planned.part_track_ids.iter())
            .enumerate()
        {
            let notes = after.clip_pool[clip_id]
                .content
                .notes()
                .expect("理论片段必须是 MIDI");
            assert_eq!(notes.len(), spans.len(), "{style} 声部 {index} 的音符数");
            minted_notes.extend(notes.keys().copied());
            let mut by_onset: Vec<(u64, u64, u8, u8)> = notes
                .values()
                .map(|note| {
                    (
                        note.start_tick,
                        note.duration_ticks,
                        note.pitch,
                        note.velocity,
                    )
                })
                .collect();
            by_onset.sort_unstable();
            for (note, span) in by_onset.iter().zip(spans.iter()) {
                assert_eq!(note.0, span.start_tick, "{style}: 起点必须对齐 ChordSpan");
                assert_eq!(
                    note.1, span.duration_ticks,
                    "{style}: 时值必须对齐 ChordSpan"
                );
                assert_eq!(note.3, 96, "{style}: 理论路径的力度固定为 96");
                let chord_tones: Vec<u8> = span
                    .chord
                    .pitch_classes()
                    .iter()
                    .map(|pitch_class| pitch_class.semitones())
                    .collect();
                assert!(
                    chord_tones.contains(&(note.2 % 12)),
                    "{style}: 音高 {} 不是和弦构成音 {chord_tones:?}",
                    note.2
                );
            }
            let track = after.tracks.get(track_id).expect("声部音轨");
            assert_eq!(track.kind, TrackKind::Midi);
            assert!(track.name.starts_with("Chorus · "), "{}", track.name);
        }
        // D3：派生音符身份不得是 nil，也不得撞车。
        for (index, id) in minted_notes.iter().enumerate() {
            assert!(!id.is_nil(), "{style}: 理论音符身份不得是 nil");
            assert!(
                !minted_notes[index + 1..].contains(id),
                "{style}: 理论音符身份不得撞车: {id}"
            );
        }

        // D4：每个声部一条 `声部 → 主总线` 的边，整图必须是 DAG。
        let bus = empty.master_bus_track_id;
        let mut added_edges: BTreeMap<EntityId, yeban_model::RoutingEdge> = BTreeMap::new();
        for (index, edge_id) in planned.routing_edge_ids.iter().enumerate() {
            let edge = after
                .routing_graph
                .edges
                .get(edge_id)
                .unwrap_or_else(|| panic!("{style}: 第 {index} 条边必须存在"));
            assert_eq!(edge.source_node, planned.part_track_ids[index]);
            assert_eq!(edge.destination_node, bus);
            assert_eq!(edge.kind, yeban_model::RoutingKind::TrackToBus);
            assert_eq!(edge.gain_db, None);
            added_edges.insert(*edge_id, *edge);
        }
        let graph = RoutingGraph {
            nodes: after.routing_graph.nodes.clone(),
            edges: added_edges,
        };
        assert!(
            detect_cycle(&graph).is_none(),
            "{style}: 声部连接必须是 DAG: {:?}",
            detect_cycle(&graph)
        );
    }
}

/// `ADR-0016` D1/D3：`progression` 覆盖与 `scale` 都真的进 theory 展开。
///
/// 4/4 + 4 小节的自定义走向 ⇒ 每个和弦恰好一小节（3840 tick），起点写死；
/// `A minor` 的 `i` 级必须给出 A/C/E 三个音级。非法走向映射成契约内错误，
/// `data.theoryStep` 指出失败发生在 theory 的哪一步。
#[test]
fn theory_honours_custom_progression_and_scale() {
    let empty = empty_pool_project();
    let options = PlanOptions {
        material_source: Some("theory"),
        progression: Some("i-VII-VI-VII"),
        include_drums: None,
        include_melody: None, drum_density: None,
        voice_subdivision: None,
        voice_subdivisions: None,
        onsets_per_bar: None,
        velocity_min: None,
        velocity_max: None,
    };
    let planned = plan_with_options(&empty, "Chorus", "synthwave", 4, Some("A minor"), options)
        .expect("自定义走向");
    assert_eq!(planned.part_track_ids.len(), 3);
    let after = applied(&empty, &planned);
    for clip_id in &planned.part_clip_ids {
        let notes = after.clip_pool[clip_id].content.notes().expect("MIDI");
        let mut typed: Vec<(u64, u64, u8)> = notes
            .values()
            .map(|note| (note.start_tick, note.duration_ticks, note.pitch))
            .collect();
        typed.sort_unstable();
        assert_eq!(
            typed.iter().map(|note| (note.0, note.1)).collect::<Vec<_>>(),
            vec![(0, 3840), (3840, 3840), (7680, 3840), (11520, 3840)],
            "4/4 + 4 小节 ⇒ 每和弦一小节"
        );
        let first_degree = typed[0].2 % 12;
        assert!(
            [9, 0, 4].contains(&first_degree),
            "A minor 的 i 级必须是 A/C/E, 实际音级 {first_degree}"
        );
    }

    // 流派首选走向（synthwave 的 `i-VI-III-VII`）是另一份规划：
    // 走向必须真的进身份种子，而不是仅仅算一遍丢掉。
    let default = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        Some("A minor"),
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: None,
            include_melody: None, drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("默认走向");
    assert_ne!(planned.section_id, default.section_id);
    assert_ne!(planned.part_clip_ids, default.part_clip_ids);
    assert_eq!(planned.part_track_ids.len(), default.part_track_ids.len());

    let fault = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: Some("not-a-degree"),
            include_drums: None,
            include_melody: None, drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect_err("非法走向");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败");
    };
    assert_eq!(data["theoryStep"], "Progression::parse");
    assert!(data["theoryError"].is_string());
}

/// `ADR-0017` D1：鼓组声部是标准 GM 打击乐映射。
///
/// 期望击点由 **theory 自己**生成（`GenreRule::drum_pattern`），逐击比对
/// `(tick, GM 键, 力度, 时值)`，因此"换一个鼓件表/换一个时值"会立刻变红。
/// 同时验证该声部直连主总线且整图仍是 DAG。
#[test]
fn drums_are_gm_mapped_and_route_to_master_in_a_dag() {
    use yeban_theory::genre::GenreLibrary;

    let empty = empty_pool_project();
    let options = PlanOptions {
        material_source: Some("theory"),
        progression: None,
        include_drums: Some(true),
        include_melody: None, drum_density: None,
        voice_subdivision: None,
        voice_subdivisions: None,
        onsets_per_bar: None,
        velocity_min: None,
        velocity_max: None,
    };
    let planned = plan_with_options(&empty, "Chorus", "synthwave", 4, None, options)
        .expect("鼓组生成");
    assert_eq!(planned.part_track_ids.len(), 4, "3 个和声声部 + Drums");
    assert!(planned.unwired.is_empty(), "装配必须同构: {:?}", planned.unwired);

    let after = applied(&empty, &planned);
    let index = 3;
    let track = after
        .tracks
        .get(&planned.part_track_ids[index])
        .expect("鼓组音轨");
    assert_eq!(track.name, "Chorus · Drums");
    assert_eq!(track.kind, TrackKind::Midi);

    let notes = after.clip_pool[&planned.part_clip_ids[index]]
        .content
        .notes()
        .expect("鼓组片段必须是 MIDI");
    let mut actual: Vec<(u64, u8, u8, u64)> = notes
        .values()
        .map(|note| {
            (
                note.start_tick,
                note.pitch,
                note.velocity,
                note.duration_ticks,
            )
        })
        .collect();
    actual.sort_unstable();

    let rule = GenreLibrary::get("synthwave").expect("theory 流派");
    let pattern = rule
        .drum_pattern(4, 16)
        .expect("鼓组型")
        .expect("4/4 有鼓组型");
    let mut expected: Vec<(u64, u8, u8, u64)> = pattern
        .hits()
        .iter()
        .map(|hit| {
            (
                hit.tick,
                hit.voice.gm_key(),
                if hit.accent { 112 } else { 96 },
                120,
            )
        })
        .collect();
    expected.sort_unstable();
    assert_eq!(
        actual, expected,
        "GM 键 / 起点 / 力度 / 时值必须逐击对齐 theory 的鼓组型"
    );
    for (_, pitch, _, _) in &actual {
        assert!(
            [36, 38, 42, 51].contains(pitch),
            "非标准 GM 打击乐键: {pitch}"
        );
    }

    // 每个声部（含 Drums）都直连主总线 ⇒ 整图 DAG。
    let bus = empty.master_bus_track_id;
    assert_eq!(planned.routing_edge_ids.len(), planned.part_track_ids.len());
    let mut added = BTreeMap::new();
    for (edge_index, edge_id) in planned.routing_edge_ids.iter().enumerate() {
        let edge = after
            .routing_graph
            .edges
            .get(edge_id)
            .unwrap_or_else(|| panic!("第 {edge_index} 条边必须存在"));
        assert_eq!(edge.source_node, planned.part_track_ids[edge_index]);
        assert_eq!(edge.destination_node, bus);
        assert_eq!(edge.kind, yeban_model::RoutingKind::TrackToBus);
        added.insert(*edge_id, *edge);
    }
    let graph = RoutingGraph {
        nodes: after.routing_graph.nodes.clone(),
        edges: added,
    };
    assert!(
        detect_cycle(&graph).is_none(),
        "鼓组接入后整图必须仍是 DAG: {:?}",
        detect_cycle(&graph)
    );

    // 同一请求 ⇒ 同一份规划（身份与时值都由 `section_id` 确定性派生）。
    let repeated = plan_with_options(&empty, "Chorus", "synthwave", 4, None, options)
        .expect("重复调用");
    assert_eq!(planned, repeated);
}

/// `ADR-0017` D1/D3：旋律落在流派节奏网格与既有和弦跨度上。
///
/// 起点/时值逐 onset 对齐 theory 的 `rhythm_grid`；音高在
/// `MelodyConstraints::DEFAULT` 音域内且属于目标音阶；力度按 `chord_tone`
/// 分 112/96。同样验证 DAG。
#[test]
fn melody_stays_on_the_genre_grid_and_in_range_in_a_dag() {
    use yeban_theory::genre::GenreLibrary;
    use yeban_theory::pitch::PitchClass;
    use yeban_theory::progression::{Meter, Progression};
    use yeban_theory::{MELODY_LOWER_BOUND, MELODY_UPPER_BOUND};

    let empty = empty_pool_project();
    let planned = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: None,
            include_melody: Some(true), drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("旋律生成");
    assert_eq!(planned.part_track_ids.len(), 4, "3 个和声声部 + Melody");
    assert!(planned.unwired.is_empty(), "装配必须同构: {:?}", planned.unwired);

    let after = applied(&empty, &planned);
    let index = 3;
    let track = after
        .tracks
        .get(&planned.part_track_ids[index])
        .expect("旋律音轨");
    assert_eq!(track.name, "Chorus · Melody");
    assert_eq!(track.kind, TrackKind::Midi);

    let notes = after.clip_pool[&planned.part_clip_ids[index]]
        .content
        .notes()
        .expect("旋律片段必须是 MIDI");
    let mut actual: Vec<(u64, u64, u8, u8)> = notes
        .values()
        .map(|note| {
            (
                note.start_tick,
                note.duration_ticks,
                note.pitch,
                note.velocity,
            )
        })
        .collect();
    actual.sort_unstable();

    // 独立口径：网格与和弦跨度都由 theory 重新算一遍。
    let rule = GenreLibrary::get("synthwave").expect("theory 流派");
    let grid = rule.rhythm_grid(4, 16).expect("节奏网格");
    let hits = grid.hits();
    let total_ticks = grid.total_ticks();
    assert_eq!(actual.len(), hits.len(), "每个网格 onset 恰好一个旋律音");

    let scale = rule.primary_scale(PitchClass::C).expect("theory 音阶");
    let spans = Progression::parse(rule.progression_at(0).expect("首选走向"))
        .expect("theory 走向")
        .with_meter(Meter {
            numerator: 4,
            denominator: 4,
        })
        .with_bars(4)
        .expect("4 小节")
        .expand(&scale)
        .expect("展开");
    let scale_classes: Vec<u8> = scale
        .pitch_classes()
        .iter()
        .map(|pitch_class| pitch_class.semitones())
        .collect();

    for (onset, (note, hit)) in actual.iter().zip(hits).enumerate() {
        assert_eq!(note.0, hit.tick, "起点必须对齐网格 onset");
        let expected_duration = hits
            .get(onset + 1)
            .map_or(total_ticks, |next| next.tick)
            - hit.tick;
        assert_eq!(note.1, expected_duration, "时值必须对齐相邻 onset");
        assert!(
            (MELODY_LOWER_BOUND..=MELODY_UPPER_BOUND).contains(&note.2),
            "音高 {} 越出 MelodyConstraints::DEFAULT 音域",
            note.2
        );
        let pitch_class = note.2 % 12;
        assert!(
            scale_classes.contains(&pitch_class),
            "旋律音 {pitch_class} 不属于目标音阶 {scale_classes:?}"
        );
        let span = spans
            .iter()
            .find(|span| span.start_tick <= note.0 && note.0 < span.end_tick())
            .expect("每个旋律音都必须落在某个和弦跨度里");
        let chord_classes: Vec<u8> = span
            .chord
            .pitch_classes()
            .iter()
            .map(|pitch_class| pitch_class.semitones())
            .collect();
        let expected_velocity = if chord_classes.contains(&pitch_class) {
            112
        } else {
            96
        };
        assert_eq!(
            note.3, expected_velocity,
            "第 {onset} 个音的力度必须按 chord_tone 分级"
        );
    }

    let bus = empty.master_bus_track_id;
    let mut added = BTreeMap::new();
    for (edge_index, edge_id) in planned.routing_edge_ids.iter().enumerate() {
        let edge = after
            .routing_graph
            .edges
            .get(edge_id)
            .unwrap_or_else(|| panic!("第 {edge_index} 条边必须存在"));
        assert_eq!(edge.source_node, planned.part_track_ids[edge_index]);
        assert_eq!(edge.destination_node, bus);
        added.insert(*edge_id, *edge);
    }
    let graph = RoutingGraph {
        nodes: after.routing_graph.nodes.clone(),
        edges: added,
    };
    assert!(
        detect_cycle(&graph).is_none(),
        "旋律接入后整图必须仍是 DAG: {:?}",
        detect_cycle(&graph)
    );

    let repeated = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: None,
            include_melody: Some(true), drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("重复调用");
    assert_eq!(planned, repeated, "同一 section_id ⇒ 同一条旋律");
}

/// `ADR-0017` D2：两项缺省/`false` 时逐字节等于既有行为；两项同时为 `true`
/// 只追加两条声部，不改变既有声部的身份。
#[test]
fn drums_and_melody_are_opt_in_and_legacy_plans_are_byte_identical() {
    let empty = empty_pool_project();
    let baseline = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: None,
            include_melody: None, drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("理论生成");

    for (drums, melody) in [(None, None), (Some(false), Some(false))] {
        let legacy = plan_with_options(
            &empty,
            "Chorus",
            "synthwave",
            4,
            None,
            PlanOptions {
                material_source: Some("theory"),
                progression: None,
                include_drums: drums,
                include_melody: melody, drum_density: None,
                voice_subdivision: None,
                voice_subdivisions: None,
                onsets_per_bar: None,
                velocity_min: None,
                velocity_max: None,
            },
        )
        .expect("缺省/显式 false");
        assert_eq!(baseline, legacy, "缺省与显式 false 必须逐字节相同");
    }

    let full = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: Some(true),
            include_melody: Some(true), drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("鼓组 + 旋律");
    assert_eq!(
        full.part_track_ids.len(),
        baseline.part_track_ids.len() + 2,
        "只追加 Drums / Melody 两条声部"
    );
    assert_eq!(
        full.part_track_ids[..baseline.part_track_ids.len()],
        baseline.part_track_ids[..],
        "既有和声声部的身份不得改变"
    );
    assert_ne!(baseline.ops, full.ops, "追加声部必须真的出现在 op 序列里");

    // `'project'` 路径同样逐字节不变。
    let project = filled_project();
    let default = plan(&project, "Chorus", "synthwave", 4, None).expect("缺省路径");
    let explicit_false = plan_with_options(
        &project,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: None,
            progression: None,
            include_drums: Some(false),
            include_melody: Some(false), drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("显式 false");
    assert_eq!(default, explicit_false);
}

/// `ADR-0017` D2：`'project'` 路径上给 `includeDrums` / `includeMelody` 是
/// 契约内错误，并带指出修复方式的 `hint`。
#[test]
fn drums_and_melody_require_the_theory_source() {
    let project = filled_project();
    for source in [None, Some("project")] {
        for (drums, melody) in [(Some(true), None), (None, Some(true))] {
            let fault = plan_with_options(
                &project,
                "Chorus",
                "synthwave",
                4,
                None,
                PlanOptions {
                    material_source: source,
                    progression: None,
                    include_drums: drums,
                    include_melody: melody, drum_density: None,
                    voice_subdivision: None,
                    voice_subdivisions: None,
                    onsets_per_bar: None,
                    velocity_min: None,
                    velocity_max: None,
                },
            )
            .expect_err("工程材料路径不得生成鼓组/旋律");
            assert_eq!(
                fault.domain_code(),
                Some(BuildCode::InvalidParameterRange),
                "source={source:?} drums={drums:?} melody={melody:?}"
            );
            let BuildFault::Domain { message, data, .. } = fault else {
                panic!("必须是领域失败");
            };
            assert_eq!(
                message,
                "旋律与鼓组理论生成仅在 materialSource: 'theory' 时支持"
            );
            assert_eq!(data["hint"], "请指定 materialSource: \"theory\"");
        }
    }
}

/// `ADR-0028` D2：显式 `"normal"` 与键缺席在鼓路径上必须**逐字节相同**。
#[test]
fn drum_density_normal_is_byte_identical_to_absent() {
    let empty = empty_pool_project();
    let absent = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: Some(true),
            include_melody: None,
            drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("缺省鼓");
    let explicit = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: Some(true),
            include_melody: None,
            drum_density: Some("normal"),
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("显式 normal");
    assert_eq!(absent, explicit, "显式 `\"normal\"` 必须与键缺席逐字节相同");
}

/// `ADR-0028` D2：`sparse` 击点**逐击**等于 theory 鼓组型里登记为重音的子集 ——
/// 期望值由 theory 侧独立重算（`drum_pattern` 过滤 `accent`），⛔ 不读本模块中间量；
/// 另要求计数**严格更小**（若某流派全为重音，该判据变红并提示，而不是静默通过）。
#[test]
fn sparse_drums_are_exactly_the_accented_hits() {
    use yeban_theory::genre::GenreLibrary;

    let empty = empty_pool_project();
    let normal = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: Some(true),
            include_melody: None,
            drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("normal 鼓");
    let sparse = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: Some(true),
            include_melody: None,
            drum_density: Some("sparse"),
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("sparse 鼓");

    // 鼓轨/片段身份不变 —— 密度档只改音符集合。
    assert_eq!(
        normal.part_track_ids, sparse.part_track_ids,
        "鼓轨身份不得随密度档改变"
    );
    assert_eq!(
        normal.part_clip_ids, sparse.part_clip_ids,
        "鼓片段身份不得随密度档改变"
    );

    let after_normal = applied(&empty, &normal);
    let after_sparse = applied(&empty, &sparse);
    let notes_normal = after_normal.clip_pool[&normal.part_clip_ids[3]]
        .content
        .notes()
        .expect("normal 鼓片段必须是 MIDI");
    let notes_sparse = after_sparse.clip_pool[&sparse.part_clip_ids[3]]
        .content
        .notes()
        .expect("sparse 鼓片段必须是 MIDI");

    let mut expected: Vec<(u64, u8, u8, u64)> = GenreLibrary::get("synthwave")
        .expect("theory 流派")
        .drum_pattern(4, 16)
        .expect("鼓组型")
        .expect("4/4 有鼓组型")
        .hits()
        .iter()
        .filter(|hit| hit.accent)
        .map(|hit| (hit.tick, hit.voice.gm_key(), 112, 120))
        .collect();
    expected.sort_unstable();
    let mut actual: Vec<(u64, u8, u8, u64)> = notes_sparse
        .values()
        .map(|note| (note.start_tick, note.pitch, note.velocity, note.duration_ticks))
        .collect();
    actual.sort_unstable();
    assert_eq!(
        actual, expected,
        "sparse 必须逐击等于 theory 登记的重音击点（力度 112、时值 120）"
    );
    assert!(
        notes_sparse.len() < notes_normal.len(),
        "sparse 必须真的更稀疏: {} vs {}",
        notes_sparse.len(),
        notes_normal.len()
    );
    for id in notes_sparse.keys() {
        assert!(
            notes_normal.contains_key(id),
            "sparse 音符身份必须来自 normal 全集（hit_index 沿用原索引）: {id}"
        );
    }
}

/// `ADR-0028` D2：`drumDensity` 只有鼓轨在场时才有意义；未知取值必须列出
/// `accepted` 清单（不静默回退 `normal`）。
#[test]
fn drum_density_requires_include_drums_and_rejects_unknown_values() {
    let empty = empty_pool_project();
    for drums in [None, Some(false)] {
        let fault = plan_with_options(
            &empty,
            "Chorus",
            "synthwave",
            4,
            None,
            PlanOptions {
                material_source: Some("theory"),
                progression: None,
                include_drums: drums,
                include_melody: None,
                drum_density: Some("sparse"),
                voice_subdivision: None,
                voice_subdivisions: None,
                onsets_per_bar: None,
                velocity_min: None,
                velocity_max: None,
            },
        )
        .expect_err("drumDensity 不得脱离 includeDrums:true 单独给");
        assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
        let BuildFault::Domain { data, .. } = fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(
            data["hint"],
            "请同时指定 includeDrums: true（鼓密度只作用于生成的鼓轨）"
        );
    }
    let fault = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: Some(true),
            include_melody: None,
            drum_density: Some("double"),
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect_err("未知密度档必须拒绝");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败");
    };
    assert_eq!(
        data["accepted"],
        serde_json::json!(["normal", "sparse"]),
        "拒绝必须列出合法取值"
    );
    assert_eq!(data["drumDensity"], serde_json::json!("double"));
}

/// `ADR-0028` D1：3 声部理论段的最低声部必须坐在 C2–C3（单位：MIDI 音高值
/// 36–48，实测落点 47–48），且声部按音区成梯次 —— 修正前该声部实测 67–72。
#[test]
fn three_voice_bass_sits_in_the_low_register() {
    let empty = empty_pool_project();
    let planned = plan_with_options(
        &empty,
        "Chorus",
        "synthwave",
        4,
        None,
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: None,
            include_melody: None,
            drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
        },
    )
    .expect("3 声部理论段");
    assert_eq!(planned.part_track_ids.len(), 3);
    let after = applied(&empty, &planned);
    let bass = after.clip_pool[&planned.part_clip_ids[0]]
        .content
        .notes()
        .expect("bass 片段必须是 MIDI");
    assert!(!bass.is_empty(), "bass 声部必须有音符");
    for note in bass.values() {
        assert!(
            (36..=48).contains(&note.pitch),
            "bass 音高 {} 越出 C2–C3 区间",
            note.pitch
        );
    }
    let alto = after.clip_pool[&planned.part_clip_ids[1]]
        .content
        .notes()
        .expect("内声部片段必须是 MIDI");
    let high = after.clip_pool[&planned.part_clip_ids[2]]
        .content
        .notes()
        .expect("高声部片段必须是 MIDI");
    let bass_max = bass.values().map(|note| note.pitch).max().expect("非空");
    let alto_min = alto.values().map(|note| note.pitch).min().expect("非空");
    let alto_max = alto.values().map(|note| note.pitch).max().expect("非空");
    let high_min = high.values().map(|note| note.pitch).min().expect("非空");
    assert!(
        bass_max < alto_min && alto_max < high_min,
        "声部按音区成梯次: bass≤{bass_max} < alto {alto_min}..{alto_max} < high≥{high_min}"
    );
}
