//! 文档化的**可选实参**必须至少被一条判据给过**非缺省值**（`ADR-0028` 的教训，规则 32 的机械化）。
//!
//! 由来（2026-10-28，`drumDensity`）：`ADR-0028` 落地后很长一段时间里，
//! `crates/yeban-mcp/tests/*.rs` 的 **14** 处 `drum_density` **全部**是 `None`（管线占位）——
//! 也就是说 `section_build.rs` 里那个稀疏筛点**没有任何判据看着**，而**看目录**完全看不出来：
//! 同族的 `swingPermille` / `drumStyle` / `drumBackbeat` / `beatGrouping` / `melodySeed`
//! 各有自己的判据文件（缺口补在 `tests/section_drum_density.rs`）。
//!
//! 单位 = **文档化可选键**（取自 `schemas/mcp-tools.schema.json` 的
//! `definitions.DocumentedToolArguments.$defs.<tool>.properties`，⛔ 本文件另抄一份键名表）。
//!
//! ## 判据力（先说清楚它**证明**什么、⛔ 证明什么）
//!
//! - 证明：每个登记的可选键都在测试里出现过一次**非缺省取值**（`k: <非 None>` 或 `"k": <非 null>`）。
//! - ⛔ 证明那个值真的到了消费点：`k: given.k` 这类建造器管线也只是"某个非 `None` 的值从这里过"。
//!   ⇒ 本判据挡的是"**整键零覆盖**"（14/14 全是 `None` 那一类），⛔ 挡"值语义写错"。
//! - ⛔ 数 `__` 字面量 `Some(`：同目录判据大多经建造器给值，构造点写的是 `k: given.k` ⇒
//!   `Some(` 在有覆盖的键上同样是 0（实测：本判据初版就是这么错的，见台账 16:03 的处方修正）。
//!
//! ## 敏感性（规则 30：先把真缺陷的预映像喂给它）
//!
//! 预映像（`section_drum_density.rs` 落地**之前**的测试树）上，本判据的等价算法在 21 个键里
//! **恰好红 1 个**：`drumDensity`（0 处非缺省）；其余 20 个键各 ≥ 1 ⇒ ⛔ 误伤面。
//! 现读最小读数 = **1**（`drumDensity` / `drumStyle` / `drumBackbeat` / `beatGrouping` 四个键）。
//!
//! ## 扫描面（⛔ 本判据**不**覆盖扩展工具；且扩展侧的词面读数会**假绿**）
//!
//! 只扫 `DocumentedToolArguments`（3 个工具 / 21 个键）。`ExtensionToolArguments.$defs`
//! （15 个工具 / 81 个属性）未纳入，两条**实测**理由：
//!
//! 1. `src` 单元测试也是判据（`yeban_query_engine_state.since` 的断言在
//!    `src/domain/engine_state.rs:649`）⇒ 只读 tests 会假红；
//! 2. 换成 tests ＋ src 全扫更糟：**跨工具同名键碰撞**把别的工具命中算给本工具
//!    （`yeban_render_sfz_wav.sampleRate` 的 199 处命中全在 `yeban_render_master`
//!    一族的调用里），**非法值**（拒绝用例）也算命中（`toneHz` 唯一一处是
//!    `yeban_record_wav` 的 `-5`）⇒ 2026-10-28 的手工全扫给出"零覆盖 0"的**假绿**。
//!    真相是：`yeban_render_sfz_wav.velocity` 那时**根本没到达渲染器**（解析＋回显的
//!    空操作，R142），`toneHz` 从未被给过有效值，`sampleHz` 只有输出被断言。
//!
//! 三处缺口现由 `tests/sfz_tools.rs` 的语义判据兜住
//! （`the_sample_hz_argument_is_honoured_and_bad_values_are_rejected` /
//! `the_render_velocity_override_selects_regions_and_scales_the_peak` /
//! `the_render_sample_rate_and_tone_are_honoured`）。
//!
//! ⚠ 结论：扩展参数轴**不能靠词面扫描**（规则 32 的假阳性一侧）。要机械化必须按
//! "**哪个调用的**实参"归类（解析调用点），本文件不做 —— 这项 rot 风险如实登记。
//!
//! ⭐ **2026-10-30 加了第二条轴**（`every_documented_creation_key_is_registered_with_an_effect_criterion`）：
//! 上面这条只数"键有没有被给过非缺省值"，新那条登记"**改了它的值 ⇒ 产出真的变**"的证据判据。
//! 两轴**共用**同一份键全集派生（`documented_keys()`）与同一份测试源读取（`test_sources()`），
//! ⛔ 各数各的键（第 44 条）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// 文档化可选键的登记处（`ToolCall` 的特有实参走注册表，这一节登记**新增**键的形状）。
const SCHEMA: &str = "schemas/mcp-tools.schema.json";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/yeban-mcp 之上应当有仓库根")
        .to_path_buf()
}

/// `materialSource` → `material_source`（`PlanOptions` 的字段名形态）。
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// 本键在**给定文本**里被给过非缺省值的次数。
///
/// 两种形态各数一遍：Rust 字段（`drum_density: Some("sparse")` / `drum_density: density`）
/// 与 JSON 实参（`"drumDensity": "sparse"`）。`None` / `null` ⛔ 算数（那正是缺口的样子）。
fn non_default_hits(text: &str, camel: &str, snake: &str) -> usize {
    let mut hits = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        // ① Rust 字段形态：`<snake>:` 之后的值⛔ 是 `None`。
        if let Some(at) = line.find(&format!("{snake}:")) {
            let rest = line[at + snake.len() + 1..]
                .trim()
                .trim_end_matches(',')
                .trim();
            if !rest.is_empty() && rest != "None" {
                hits += 1;
            }
        }
        // ② JSON 实参形态：`"<camel>":` 之后的值⛔ 是 `null`。
        if let Some(at) = line.find(&format!("\"{camel}\":")) {
            let rest = line[at + camel.len() + 3..]
                .trim()
                .trim_end_matches(',')
                .trim();
            if !rest.is_empty() && rest != "null" {
                hits += 1;
            }
        }
    }
    hits
}

/// **键全集的唯一派生处**（本文件两条判据共用一把尺：第 44 条 ⛔ 第二份真相）。
///
/// 对象＝ `schemas/mcp-tools.schema.json` 里 `DocumentedToolArguments.$defs` 的每个工具
/// 的 `properties` 键（现读，字典序去重），单位＝个 `(tool, camelKey)` 对。
/// R93：集合是**动态**的 ⇒ 下界在这里钉，schema 被清空时两条判据一起红。
fn documented_keys() -> Vec<(String, String)> {
    let root = repo_root();
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(root.join(SCHEMA)).expect("schema 必须可读"))
            .expect("schema 必须是合法 JSON");
    let tools = schema["definitions"]["DocumentedToolArguments"]["$defs"]
        .as_object()
        .expect("`DocumentedToolArguments.$defs` 必须是对象");
    assert!(
        !tools.is_empty(),
        "`DocumentedToolArguments.$defs` 为空 —— 扫描面异常"
    );
    let mut keys: Vec<(String, String)> = Vec::new();
    for (tool, spec) in tools {
        let props = spec["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("`{tool}` 必须登记 properties"));
        for (camel, _) in props {
            keys.push((tool.clone(), camel.clone()));
        }
    }
    keys.sort();
    keys.dedup();
    assert!(
        keys.len() >= 20,
        "文档化可选键只剩 {} 个 —— 扫描面异常（现读 29）",
        keys.len()
    );
    keys
}

/// **测试源集合的唯一读取处**（覆盖面数"键出没出现过"，效果面数"判据函数在不在"）。
fn test_sources() -> Vec<(PathBuf, String)> {
    let tests_dir = repo_root().join("crates/yeban-mcp/tests");
    let mut sources: Vec<(PathBuf, String)> = Vec::new();
    for entry in fs::read_dir(&tests_dir).expect("tests 目录必须可读") {
        let path = entry.expect("目录项必须可读").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        sources.push((
            path.clone(),
            fs::read_to_string(&path).expect("测试源必须可读"),
        ));
    }
    assert!(
        sources.len() >= 20,
        "扫到的测试源只剩 {} 个 —— 扫描面异常",
        sources.len()
    );
    sources.sort();
    sources
}

#[test]
fn every_documented_optional_argument_is_given_a_non_default_value_somewhere() {
    let keys: Vec<(String, String, String)> = documented_keys()
        .into_iter()
        .map(|(tool, camel)| {
            let snake = snake_case(&camel);
            (tool, camel, snake)
        })
        .collect();

    let sources = test_sources();

    let mut uncovered: Vec<String> = Vec::new();
    let mut readings: Vec<(String, usize)> = Vec::new();
    for (tool, camel, snake) in &keys {
        let hits: usize = sources
            .iter()
            .map(|(_, text)| non_default_hits(text, camel, snake))
            .sum();
        readings.push((camel.clone(), hits));
        if hits == 0 {
            uncovered.push(format!("{tool}.{camel}"));
        }
    }

    assert!(
        uncovered.is_empty(),
        "这些文档化的可选键在 `crates/yeban-mcp/tests/*.rs` 里**一次非缺省值都没有**给过 \
         （＝整键零覆盖：`ADR-0028` 的 `drumDensity` 就是这样漏了整整一轮）: {:?}\n\
         逐个读数（键, 非缺省出现次数）: {:?}",
        uncovered,
        readings
    );
}

/// 给定文本里有没有名为 `name` 的**函数定义**（`fn NAME(`，带标识符边界，⛔ 子串命中）。
///
/// 右侧边界由 needle 自带的 `(` 保证（`NAME_x(` ⛔ 命中 `NAME(`）；左侧只需排除
/// `myfn NAME(` 这类粘连（R119 同型）。
fn defines_fn(text: &str, name: &str) -> bool {
    let needle = format!("fn {name}(");
    let mut search = 0_usize;
    while let Some(found) = text[search..].find(&needle) {
        let at = search + found;
        search = at + 1;
        let prev_is_ident = text[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !prev_is_ident {
            return true;
        }
    }
    false
}

/// 效果面登记表（**第二条轴**）：每个文档化键要么绑定一条"**改它的值 ⇒ 产出真的变**"
/// （或把"缺省 ⇒ 与今天逐字节相同"钉住）的判据，要么**显式登记为缺口**。
///
/// # 为什么是人工登记表而⛔ 词法扫描
///
/// 上一条判据只证明"某处给过非缺省值"，而本文件头部（第 40-47 行）自己就写明那⛔ 等于
/// 证明键被 honour。先试过词面自动认（判据名含键的 snake_case 全名 **且** 含效果词），
/// 实测只认出 **5 / 29** —— `drumBackbeat` 的证据函数叫
/// `the_backbeat_moves_only_the_snare_line`，词面必然漏（`AGENTS.md` §6.3 第 3 条的同型
/// 陷阱：⛔ 用词面判断交付）。于是**语义由人核（逐个读函数体），机械只钉三件事**：
///
/// 1. **两向集合相等**：schema 的键全集 ＝ 已证名单 ∪ 缺口名单（新键进 schema 而没登记 ⇒ 红；
///    键被删而登记表没删 ⇒ 红）；
/// 2. **登记的判据名逐个真实存在**（`fn NAME(` 现读）：判据被改名／删掉 ⇒ 红，
///    ⛔ 让登记表退化成死名册；
/// 3. **缺口⛔ 悄悄补齐**：已证与缺口两名单⛔ 重叠，且每行已证必须给出**非空**判据名
///    （空集⛔ 充当"已证"）。
///
/// # 读数（2026-10-30，逐函数体核过；对象＝键，单位＝个键）
///
/// **29** 个键 ＝ **29** 已证 ＋ **0** 缺口。本票落地时读出的两个缺口（`propose.materialSource`
/// 只有耦合校验、`edit_notes.mergeCommitMessage` 无判据读回合并提交的信息）已由同线的
/// `material_source_switches_the_material_and_theory_ignores_the_clip_pool` 与
/// `transpose_clip_expands_into_one_move_per_note_in_a_single_batch` 补齐，⛔ 表退化成死名册：
/// 两侧都由"登记的判据名 `fn NAME(` 真实存在"那条拦着。
///
/// ⭐ **缺省侧**（"不给这个键 ⇒ 与今天逐字节相同"那一半）此前有四处没钉子，现由同线
/// 第三片补齐：`propose.includeDrums` / `propose.includeMelody` ⇒
/// `absent_include_flags_are_byte_identical_to_explicit_false`（该文件的 `Given` 两位
/// 自 `bool` 放宽成 `Option<bool>` —— 此前⛔ 表达得出"不给"）、`propose.progression` ⇒
/// `absent_progression_is_byte_identical_to_the_registered_first_pick`、
/// `edit_notes.autoMerge` ⇒ `absent_auto_merge_on_edit_notes_proposes_without_merging`。
/// ⚠ 其余 **25** 行仍按**任一侧**判据登记即算已证（如 `propose.melodyUpper` 只有缺省侧
/// 那一条）—— 本表⛔ 断言"两侧都钉过"，两侧状态只在上面这四个键上逐行核过。
#[test]
fn every_documented_creation_key_is_registered_with_an_effect_criterion() {
    /// 已证名单：键 → 证据判据名（只取一条代表性判据；同名判据⛔ 要求在同一文件里）。
    const EFFECT_FACE: [(&str, &str, &[&str]); 29] = [
        (
            "yeban_edit_notes",
            "autoMerge",
            &[
                "transpose_clip_expands_into_one_move_per_note_in_a_single_batch",
                "absent_auto_merge_on_edit_notes_proposes_without_merging",
            ],
        ),
        (
            "yeban_edit_notes",
            "create",
            &["edit_notes_create_and_place_in_one_proposal_is_atomic_and_undoable"],
        ),
        (
            "yeban_edit_notes",
            "mergeCommitMessage",
            &["transpose_clip_expands_into_one_move_per_note_in_a_single_batch"],
        ),
        (
            "yeban_edit_notes",
            "placement",
            &["needs_6_placement_puts_pool_material_on_the_timeline_and_the_master_hears_it"],
        ),
        (
            "yeban_propose_section",
            "beatGrouping",
            &["the_grouping_moves_the_kick_and_the_accents_not_the_grid"],
        ),
        (
            "yeban_propose_section",
            "autoMerge",
            &["auto_merge_lands_in_one_call"],
        ),
        (
            "yeban_propose_section",
            "drumBackbeat",
            &["the_backbeat_moves_only_the_snare_line"],
        ),
        (
            "yeban_propose_section",
            "drumDensity",
            &["sparse_keeps_only_the_accented_hits_of_every_registered_genre"],
        ),
        (
            "yeban_propose_section",
            "drumStyle",
            &["four_on_the_floor_only_adds_kicks_on_beat_starts"],
        ),
        (
            "yeban_propose_section",
            "harmonyMaxVoiceJump",
            &["a_tight_jump_moves_only_the_harmony_pitches"],
        ),
        (
            "yeban_propose_section",
            "harmonySeed",
            &["harmony_seed_is_deterministic_and_swaps_the_progression_pick"],
        ),
        (
            "yeban_propose_section",
            "includeDrums",
            &[
                "drums_generate_on_project_source",
                "absent_include_flags_are_byte_identical_to_explicit_false",
            ],
        ),
        (
            "yeban_propose_section",
            "includeMelody",
            &[
                "melody_generates_over_project_material_with_explicit_scale",
                "absent_include_flags_are_byte_identical_to_explicit_false",
            ],
        ),
        (
            "yeban_propose_section",
            "materialClipIds",
            &["named_materials_feed_parts_in_declared_order"],
        ),
        (
            "yeban_propose_section",
            "materialSource",
            &["material_source_switches_the_material_and_theory_ignores_the_clip_pool"],
        ),
        (
            "yeban_propose_section",
            "melodyLower",
            &["a_narrow_register_moves_only_the_melody_pitches"],
        ),
        (
            "yeban_propose_section",
            "melodyMaxLeap",
            &["a_tight_leap_moves_only_the_melody_pitches"],
        ),
        (
            "yeban_propose_section",
            "melodySeed",
            &["a_different_seed_changes_pitches_and_velocity_never_the_grid"],
        ),
        (
            "yeban_propose_section",
            "melodyUpper",
            &["absent_register_is_byte_identical_to_the_published_window"],
        ),
        (
            "yeban_propose_section",
            "mergeCommitMessage",
            &["merge_commit_message_passes_through"],
        ),
        (
            "yeban_propose_section",
            "onsetsPerBar",
            &["grid_moves_only_drums_and_melody"],
        ),
        (
            "yeban_propose_section",
            "partTracks",
            &["reused_tracks_get_placements_without_new_tracks"],
        ),
        (
            "yeban_propose_section",
            "progression",
            &[
                "symbol_progression_voices_the_named_chords_per_bar",
                "absent_progression_is_byte_identical_to_the_registered_first_pick",
            ],
        ),
        (
            "yeban_propose_section",
            "swingPermille",
            &["the_displacement_grows_monotonically_with_the_permille"],
        ),
        (
            "yeban_propose_section",
            "velocityMax",
            &["absent_ladder_is_byte_identical_to_the_published_two_rungs"],
        ),
        (
            "yeban_propose_section",
            "velocityMin",
            &["the_two_rungs_land_on_exactly_the_registered_accent_tiers"],
        ),
        (
            "yeban_propose_section",
            "voiceSubdivision",
            &["voice_subdivision_partitions_the_span_without_loss"],
        ),
        (
            "yeban_propose_section",
            "voiceSubdivisions",
            &["voice_subdivisions_apply_to_the_named_voice_only"],
        ),
        (
            "yeban_render_master",
            "contributions",
            &["contributions_match_an_independent_mute_render"],
        ),
    ];
    /// 缺口名单：**今天为空**。⛔ 把它删掉 —— 这张空表是"新键进 schema 而没人登记"
    /// 那一类的接收端（`declared == universe` 靠它把未证的键接住），也是"已证的键被
    /// 悄悄降级"时唯一会红的位置。补一片时的写法：把行从 `EFFECT_FACE` 搬进来、
    /// 两侧长度各自 +1/-1，判据 3 的"两名单⛔ 重叠"会拦住只搬不改的情况。
    const GAPS: [(&str, &str, &str); 0] = [];

    let universe: std::collections::BTreeSet<(String, String)> =
        documented_keys().into_iter().collect();
    let mut declared: std::collections::BTreeSet<(String, String)> = EFFECT_FACE
        .iter()
        .map(|(tool, camel, _)| (tool.to_string(), camel.to_string()))
        .collect();
    declared.extend(
        GAPS.iter()
            .map(|(tool, camel, _)| (tool.to_string(), camel.to_string())),
    );
    assert_eq!(
        declared, universe,
        "效果面登记表与 schema 的键全集两向不符 => 新键没登记，或删掉的键还留在表里"
    );
    assert!(
        EFFECT_FACE.len() >= 20,
        "已证键只剩 {} 行 => 登记表被整体掏空（R93：⛔ 把已证批量搬进缺口名单来让它绿）",
        EFFECT_FACE.len()
    );

    let sources = test_sources();
    for (tool, camel, names) in EFFECT_FACE {
        assert!(
            !GAPS.iter().any(|row| row.0 == tool && row.1 == camel),
            "键 `{tool}.{camel}` 同时出现在已证与缺口两张名单 => 登记口径自相矛盾"
        );
        assert!(
            !names.is_empty(),
            "键 `{tool}.{camel}` 登记的判据名是空集 => 空集⛔ 充当『已证』"
        );
        for &name in names {
            assert!(
                sources
                    .iter()
                    .any(|(_, text)| defines_fn(text.as_str(), name)),
                "键 `{tool}.{camel}` 登记的判据 `{name}` 在 `crates/yeban-mcp/tests/*.rs` 里\
                 找不到 `fn {name}(` => 判据被改名或删掉了，登记表成了死名册"
            );
        }
    }
}
