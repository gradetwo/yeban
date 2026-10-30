//! `审计 N1`：`includeDrums` / `includeMelody` 放开到 `'project'` 材料源 ——
//! 鼓轨不读和声材料（流派 `GenreRule` 自带网格/音色/重音）⇒ 两源同口径；
//! 旋律需要和弦跨度 ⇒ 逐小节从和声材料片段的 voicing 判出（`material_spans`），
//! 音阶判不出来 ⇒ 要求显式 `scale`。
//!
//! 夹具材料：`filled_project` 的 MIDI 片段第一小节是 `{60,64,67,72}` =
//! Cmaj7 四音类（`Chord::from_voicing` 判得出）；后三小节无相交音符 ⇒
//! 判据 2 顺带守住"延音沿用上一小节"那条语义。
//!
//! ⭐ 判据 4（2026-10-30 补）把本片从**打开那面**扩到**缺省那面**：两位键
//! "不给"与"给 `false`"必须逐字节同形 —— 这是效果面登记表读出的四处缺省钉之一。

use std::collections::BTreeMap;

use serde_json::Value;
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options,
};
use yeban_model::music::MidiNote;
use yeban_model::{ClipContent, EntityId, Op, samples::filled_project};

const BARS: u64 = 4;
const STYLE: &str = "synthwave";
/// GM 打击乐键（底鼓 36 / 军鼓 38 / 踩镲 42 / 吊镲 51）。
const GM_DRUM_KEYS: [u8; 4] = [36, 38, 42, 51];

#[derive(Clone, Copy, Default)]
struct Given {
    material_source: Option<&'static str>,
    scale: Option<&'static str>,
    /// ⭐ `None` = 调用方**不给这个键**（缺省钉判据要吃这一支）；此前是 `bool`，
    /// 于是本文件把两位一律包成 `Some(..)` —— 缺省形状在全库一次都没跑过。
    include_drums: Option<bool>,
    include_melody: Option<bool>,
    swing_permille: Option<u64>,
    melody_seed: Option<u64>,
}

fn try_plan(
    project: &yeban_model::YebanProjectV1,
    given: Given,
) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        project,
        "Chorus",
        STYLE,
        BARS,
        given.scale,
        PlanOptions {
            material_source: given.material_source,
            progression: None,
            include_drums: given.include_drums,
            include_melody: given.include_melody,
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
            swing_permille: given.swing_permille,
            drum_backbeat: None,
            drum_style: None,
            melody_seed: given.melody_seed,
            harmony_seed: None,
            material_clip_ids: None,
            part_tracks: None,
            beat_grouping: None,
        },
    )
}

fn plan(project: &yeban_model::YebanProjectV1, given: Given) -> SectionPlan {
    try_plan(project, given).unwrap_or_else(|fault| panic!("规划必须成功: {fault:?}"))
}

/// 按声部名取音符表 `(起点, 时值, 音高, 力度)`。
fn part_notes(section_plan: &SectionPlan, part: &str) -> Vec<(u64, u64, u8, u8)> {
    section_plan
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::AddClip { clip } if clip.name.ends_with(part) => {
                let ClipContent::Midi { notes } = &clip.content else {
                    panic!("{part} 必须是 MIDI 片段");
                };
                Some(
                    notes
                        .values()
                        .map(|n| (n.start_tick, n.duration_ticks, n.pitch, n.velocity))
                        .collect(),
                )
            }
            _ => None,
        })
        .next()
        .unwrap_or_else(|| panic!("必须存在 `{part}` 声部的片段"))
}

fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 一次规划的**可比**读数。
///
/// ⛔ 直接比较 `SectionPlan`：它派生了 `PartialEq`，但 `part_track_ids` /
/// `part_clip_ids` 里的 `EntityId` 由 `EntityId::new()` 每次随机生成 ⇒ 两次规划
/// 永远不相等（那是身份的随机性，⛔ 本片要说的事）。这里取的是"缺省即与今天
/// 逐字节相同"真正覆盖的四件：段落身份、节拍几何、op 条数、逐片段的名字与音符表。
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlanShape {
    section_id: String,
    start_tick: u64,
    end_tick: u64,
    ticks_per_bar: u64,
    ops: usize,
    /// 每个 `AddClip` 渲染成 `名字:(起点,时值,音高,力度),…` 后拼接成一个串：
    /// "逐字节相同"这条断言因此**字面**成立，且⛔ 为此再引入一层嵌套元组类型。
    clips: String,
}

fn shape(section_plan: &SectionPlan) -> PlanShape {
    let mut clips = String::new();
    for op in &section_plan.ops {
        let Op::AddClip { clip } = op else {
            continue;
        };
        clips.push_str(&clip.name);
        clips.push(':');
        if let ClipContent::Midi { notes } = &clip.content {
            let mut rows: Vec<String> = notes
                .values()
                .map(|n| {
                    format!(
                        "({},{},{},{})",
                        n.start_tick, n.duration_ticks, n.pitch, n.velocity
                    )
                })
                .collect();
            rows.sort();
            clips.push_str(&rows.join(","));
        }
        clips.push(';');
    }
    PlanShape {
        section_id: section_plan.section_id.to_canonical_string(),
        start_tick: section_plan.start_tick,
        end_tick: section_plan.end_tick,
        ticks_per_bar: section_plan.ticks_per_bar,
        ops: section_plan.ops.len(),
        clips,
    }
}

/// 判据 1：`'project'` 源上 `includeDrums` 生成 GM 鼓轨；鼓键
/// （`swingPermille`）在两源同口径被消费 ⇒ 同给时换一版网格。
#[test]
fn drums_generate_on_project_source() {
    let project = filled_project();
    let straight = plan(
        &project,
        Given {
            include_drums: Some(true),
            ..Given::default()
        },
    );
    let hits = part_notes(&straight, "Drums");
    assert!(!hits.is_empty(), "鼓轨必须有击点");
    assert!(
        hits.iter().all(|hit| GM_DRUM_KEYS.contains(&hit.2)),
        "全部击点必须是 GM 打击乐键 {GM_DRUM_KEYS:?}: {hits:?}"
    );
    // `swingPermille` 在 project 源上同样被消费（换一版 ⇒ 网格起点变）。
    let swung = plan(
        &project,
        Given {
            include_drums: Some(true),
            swing_permille: Some(660),
            ..Given::default()
        },
    );
    assert_ne!(
        hits.iter().map(|hit| hit.0).collect::<Vec<_>>(),
        part_notes(&swung, "Drums")
            .iter()
            .map(|hit| hit.0)
            .collect::<Vec<_>>(),
        "swing=660 必须推移鼓点起点（否则 swingPermille 没被消费）"
    );
}

/// 判据 2：`'project'` 源上 `includeMelody` —— 显式 `scale` 在场时
/// 从材料判出 Cmaj7 跨度并生成旋律；`scale` 缺省 ⇒ 响亮拒（音阶判不出）。
#[test]
fn melody_generates_over_project_material_with_explicit_scale() {
    let project = filled_project();
    let fault = try_plan(
        &project,
        Given {
            include_melody: Some(true),
            ..Given::default()
        },
    )
    .expect_err("`project` 源上 includeMelody 缺省 `scale` 必须拒绝");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    assert!(data(&fault).get("reason").is_none());
    assert!(data(&fault).get("hint").is_some());

    let section_plan = plan(
        &project,
        Given {
            scale: Some("C major"),
            include_melody: Some(true),
            ..Given::default()
        },
    );
    let melody = part_notes(&section_plan, "Melody");
    assert!(!melody.is_empty(), "旋律必须有音符");
    assert!(
        melody.iter().all(|note| (48..=84).contains(&note.2)),
        "旋律音高必须落在缺省窗口 48..=84: {melody:?}"
    );
    // 确定性：同参数两次 ⇒ 逐条同形。
    let again = plan(
        &project,
        Given {
            scale: Some("C major"),
            include_melody: Some(true),
            ..Given::default()
        },
    );
    assert_eq!(melody, part_notes(&again, "Melody"));
}

/// 判据 3：材料片段逐小节判不出 voicing（<3 个音类）⇒ `InvalidParameterRange`
/// 带 `bar` 与 `hint`；⛔ 静默换和弦、⛔ 用邻近和弦充数。
#[test]
fn unvoiced_material_is_a_loud_failure() {
    let mut project = filled_project();
    // 把片段池里 MIDI 片段改残：第 0 小节只剩两个音类。
    for entry in project.clip_pool.values_mut() {
        if let ClipContent::Midi { notes } = &mut entry.content {
            let mut sparse = BTreeMap::new();
            for (index, pitch) in [60_u8, 64].iter().enumerate() {
                let id = EntityId::new();
                sparse.insert(id, MidiNote::new(id, index as u64 * 960, *pitch, 480));
            }
            *notes = sparse;
        }
    }
    let fault = try_plan(
        &project,
        Given {
            scale: Some("C major"),
            include_melody: Some(true),
            ..Given::default()
        },
    )
    .expect_err("判不出 voicing 的材料必须响亮拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    assert_eq!(data(&fault).get("bar"), Some(&Value::from(0)));
    assert!(data(&fault).get("reason").is_none());
    assert!(data(&fault).get("hint").is_some());
}

/// 判据 4（2026-10-30 补）：两位**不给键**时的缺省形状 —— `ADR-0017` D1 说
/// "缺省（`None` / `false`）与既有行为逐字节相同"，消费点 `section_build.rs:2622`
/// 的 `options.include_drums == Some(true)` 就是那句话的实现。本判据把这句话钉成
/// 读数：4 种"关"的写法（⛔ / `false` 的任意组合）⇒ 同一个形状；反向再钉两位
/// 各自 `true` 时形状**必须**移动 ⇒ 上面的相等⛔ 是"两边都零差异"的空绿。
///
/// ⭐ 此前本文件的 `Given` 两位是 `bool`，于是全库⛔ 出现过一次"不给键"的调用
/// —— 效果面登记表（`documented_arg_coverage.rs`）因此把这两行记作**只证了打开这面**。
#[test]
fn absent_include_flags_are_byte_identical_to_explicit_false() {
    let project = filled_project();
    let baseline = shape(&plan(
        &project,
        Given {
            include_drums: Some(false),
            include_melody: Some(false),
            ..Given::default()
        },
    ));

    let off: [(&str, Given); 4] = [
        ("两位都不给", Given::default()),
        (
            "只给 drums=false",
            Given {
                include_drums: Some(false),
                ..Given::default()
            },
        ),
        (
            "只给 melody=false",
            Given {
                include_melody: Some(false),
                ..Given::default()
            },
        ),
        (
            "两位都给 false",
            Given {
                include_drums: Some(false),
                include_melody: Some(false),
                ..Given::default()
            },
        ),
    ];
    for (name, given) in off {
        assert_eq!(
            baseline,
            shape(&plan(&project, given)),
            "`{name}` 必须与显式双 false 逐字节同形（缺省 = 关）"
        );
    }

    // 反向：两位各自 true 都真的改形状（鼓轨 / 旋律轨各追加一条声部）。
    let with_drums = plan(
        &project,
        Given {
            include_drums: Some(true),
            ..Given::default()
        },
    );
    assert_ne!(
        baseline,
        shape(&with_drums),
        "includeDrums=true 必须移动形状，否则上面的相等是空读"
    );
    let with_melody = plan(
        &project,
        Given {
            scale: Some("C major"),
            include_melody: Some(true),
            ..Given::default()
        },
    );
    assert_ne!(
        baseline,
        shape(&with_melody),
        "includeMelody=true 必须移动形状，否则上面的相等是空读"
    );
}
