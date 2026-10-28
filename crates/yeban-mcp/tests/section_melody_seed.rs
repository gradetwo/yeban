//! `ADR-0045`：`yeban_propose_section` 的 `melodySeed` —— 旋律**取音种子**从
//! `melody_contents` 里写死的实参 `section_seed(section_id)` 换成可给的入参，
//! 缺省 ⇒ 逐字节不变（`ADR-0016`）。
//!
//! 单位口径 = **SplitMix64 的 64 位输入**（一个标识量，⛔ 数量）⇒ 本键**没有自然上界**：
//! 界就是 `arg_u64` 的类型面本身（`0..=u64::MAX`）。因此契约里⛔ `maximum`、⛔ `default`
//! （缺省那条 `section_seed(section_id)` 随段落身份移动，写死任何一个数都是撒谎），
//! `minimum: 0` 仍登记（那一支不随任何东西移动）。
//!
//! ⚠ **换种子换的是音高 + 由音高派生的力度档，⛔ 格位**（`ADR-0045` 读数 2）：
//! `melody.rs` 里种子只经 `pick_within_leap` → `derive_index` 取一个音高，而 MCP 的力度
//! 阶梯按 `chord_tone` 选档，`chord_tone = chord_mask[pitch % 12]` 是**音高**的函数
//! ⇒ 力度档会跟着换。所以判据 3 **分两张表**：格位表 `(start, dur)` 必须逐条不变，
//! 音高表必须变，力度只要求"仍落在阶梯两档里"（⛔ 把力度当不变量、⛔ 把整条形状表拿去比对）。
//!
//! ⚠ **可行性也随种子移动**（`pick_within_leap` 依赖上一音 `previous`）⇒ `NoFeasibleVoicing`
//! 出现与否⛔ 是种子无关量。本文件把"某些种子会报错"当作**要登记的事实**：取样固定为一组
//! 种子（⛔ 为了让判据绿而只挑不报错的那些），报错者必须逐条核对是 theory 的原样转发。
//!
//! ⚠ **分层如实**（与 `ADR-0036`..`ADR-0044` 的同一口径）：`-1` / `1.5` / `"7"` 三类
//! 非正整数在**工具面**被拦（`ParamSpec::accepts("integer")` 拦下浮点与字符串，
//! 负数过类型闸后由 `arg_u64` 的 `as_u64` 拦下）⇒ 判据 4 用 `Dispatcher` 真跑一次调用，
//! ⛔ 只用 `json_type == "integer"` 代替（那只会漏掉 `-1` 那一支）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{ErrorCode, TOOLS};
use yeban_model::{ClipContent, EntityId, Op, samples::filled_project};

/// 夹具的小节数。
const BARS: u64 = 4;
/// 夹具的风格预设：4/4（⇒ 缺省网格 16）且登记了旋律走向。
const STYLE: &str = "synthwave";
/// 夹具的调：C 自然小调（窗口宽 ⇒ 多数种子有解）。
const KEY: &str = "C minor";
/// 缺省阶梯的两档（本文件的夹具不给力度键）。
const LADDER: [u8; 2] = [96, 112];
/// 判据 3 的固定取样（⛔ 按结果挑选）：两个端点、两个小整数、两个大整数。
const SEEDS: [u64; 6] = [0, 1, 42, 7_000_000_000, 0xdead_beef_cafe, u64::MAX];

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`，按**音符 id** 索引。
type Note = (u64, u64, u8, u8);

/// 一次请求的可给项。
#[derive(Clone, Copy)]
struct Given {
    material_source: Option<&'static str>,
    include_melody: bool,
    include_drums: bool,
    melody_seed: Option<u64>,
}

impl Given {
    fn theory() -> Self {
        Self {
            material_source: Some("theory"),
            include_melody: true,
            include_drums: true,
            melody_seed: None,
        }
    }

    fn seed(mut self, value: Option<u64>) -> Self {
        self.melody_seed = value;
        self
    }

    fn material_source(mut self, value: Option<&'static str>) -> Self {
        self.material_source = value;
        self
    }

    fn melody(mut self, value: bool) -> Self {
        self.include_melody = value;
        self
    }

    fn drums(mut self, value: bool) -> Self {
        self.include_drums = value;
        self
    }
}

fn try_plan(given: Given) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        &filled_project(),
        "Chorus",
        STYLE,
        BARS,
        Some(KEY),
        PlanOptions {
            material_source: given.material_source,
            progression: None,
            include_drums: Some(given.include_drums),
            include_melody: Some(given.include_melody),
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
            melody_seed: given.melody_seed,
        },
    )
}

fn plan(given: Given) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "theory 规划（melodySeed={:?}）必须成功: {fault:?}",
            given.melody_seed
        )
    })
}

/// 按**声部名**索引的音符表（声部名取 `"{section} · {part}"` 的后半）。
fn voices(section_plan: &SectionPlan) -> BTreeMap<String, BTreeMap<String, Note>> {
    let mut map: BTreeMap<String, BTreeMap<String, Note>> = BTreeMap::new();
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
        let table = map.entry(part).or_default();
        for note in notes.values() {
            table.insert(
                note.id.to_canonical_string(),
                (
                    note.start_tick,
                    note.duration_ticks,
                    note.pitch,
                    note.velocity,
                ),
            );
        }
    }
    map
}

/// 某声部**排序后的格位表**（`(起点, 时值)`，⛔ 含音高 —— 那是另一张表）。
fn grid(table: &BTreeMap<String, Note>) -> Vec<(u64, u64)> {
    let mut values: Vec<(u64, u64)> = table.values().map(|note| (note.0, note.1)).collect();
    values.sort_unstable();
    values
}

/// 某声部**排序后的音高表**（含重复 ⇒ 是"多重集"读数）。
fn pitch_bag(table: &BTreeMap<String, Note>) -> Vec<u8> {
    let mut values: Vec<u8> = table.values().map(|note| note.2).collect();
    values.sort_unstable();
    values
}

/// 从规划结果里取出**段落身份**（`Op::SetSection` 的那一个）。
fn section_id_of(section_plan: &SectionPlan) -> EntityId {
    section_plan
        .ops
        .iter()
        .find_map(|op| match op {
            Op::SetSection { section_id, .. } => Some(*section_id),
            _ => None,
        })
        .expect("规划结果必须带一条 `SetSection`")
}

/// ⭐ `ADR-0017` D3 / `ADR-0045` D5 的**派生种子**（与 `section_seed` 同一算法：
/// ULID 的低 64 位与高 64 位亦或）。判据 1 用它把"键缺席 ≡ 给派生值"钉成可测的等式。
fn derived_seed(section_id: EntityId) -> u64 {
    let raw: u128 = section_id.as_ulid().0;
    #[allow(clippy::cast_possible_truncation)]
    let low = raw as u64;
    #[allow(clippy::cast_possible_truncation)]
    let high = (raw >> 64) as u64;
    low ^ high
}

/// 失败载荷的 `data`。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 断言一条拒绝：码 = `INVALID_PARAMETER_RANGE`，且载荷⛔ 带 `reason` 键（D6：⛔ 新词）。
fn assert_shape_reject(fault: &BuildFault, expected: &str) {
    assert_eq!(
        fault.domain_code(),
        Some(BuildCode::InvalidParameterRange),
        "{expected} 必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
    );
    let payload = data(fault);
    assert!(
        payload.get("reason").is_none(),
        "{expected}：载荷⛔ 带 `reason` 键（否则本票会撑大故障词表）: {payload}"
    );
}

// ---------------------------------------------------------------------------
// 工具面（`Dispatcher`）夹具：判据 4 要真跑一次调用，纯领域层量不到类型闸。
// ---------------------------------------------------------------------------

fn dispatcher_with_project() -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("melody-seed"), filled_project(), false)
        .expect("注入规范工程");
    dispatcher
}

fn unique_path(tag: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "yeban-mcp-melody-seed-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ))
        .join("demo.yeban")
}

/// 一次 `tools/call`，**不拆包**：类型闸的拒绝落在 JSON-RPC 层（`-32602`），
/// 而 `arg_u64` 的拒绝落在带内（`ToolResponse`）⇒ 两个落点都要能断言。
/// 口径先例：`tests/export_stems.rs` 判据 7、`tests/effect_contract.rs` 判据 ⑤。
fn call_raw(dispatcher: &mut Dispatcher, auth: &str, arguments: Value) -> yeban_mcp::Response {
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "yeban_propose_section", "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    outcome.response.expect("必须有响应")
}

/// 带内读数（`ToolResponse` 的 JSON）；协议层报错时直接 panic，把两侧区分开。
fn in_band(response: yeban_mcp::Response, expected: &str) -> Value {
    assert!(
        response.error.is_none(),
        "{expected}：拒绝必须走**带内**（ToolResponse），⛔ 协议层错误码: {:?}",
        response.error
    );
    response.result.expect("带内响应")
}

/// 协议层读数（`-32602` 那一支）：返回报文文本，供"点名哪个键"的断言。
fn protocol_error(response: yeban_mcp::Response, expected: &str) -> String {
    assert!(
        response.result.is_none(),
        "{expected}：类型闸拦下后不该带 result"
    );
    let error = response
        .error
        .unwrap_or_else(|| panic!("{expected}：必须是 JSON-RPC 层错误"));
    assert_eq!(
        error.code, -32602,
        "{expected}：坏类型走既有的 `INVALID_PARAMS`（⛔ 新码）"
    );
    format!(
        "{} {}",
        error.message,
        serde_json::to_string(&error.data).expect("错误 data 必须可序列化")
    )
}

/// 基线必填实参（`sectionName` / `stylePreset` / `bars` + theory + 旋律）。
fn base_args(seed: Value) -> Value {
    json!({
        "sectionName": "Chorus",
        "stylePreset": STYLE,
        "bars": BARS,
        "scale": KEY,
        "materialSource": "theory",
        "includeMelody": true,
        "includeDrums": false,
        "melodySeed": seed,
        "dryRun": true,
    })
}

/// ⭐ 判据 1（`ADR-0045` D1/D5）：**键缺席 ≡ 给出 `derived_seed(section_id)`** ⇒ 整份
/// `SectionPlan` 逐字节相同；顺带钉夹具形状（旋律在场、多条音符、和声与鼓在场）。
#[test]
fn absent_seed_is_byte_identical_to_the_derived_seed() {
    let absent = plan(Given::theory());
    let id = section_id_of(&absent);
    let explicit = plan(Given::theory().seed(Some(derived_seed(id))));
    assert_eq!(
        absent,
        explicit,
        "键缺席必须逐字节等于给 `{{\"melodySeed\": {}}}`（`ADR-0016` 的缺省兼容契约 + D5 的结构保证）",
        derived_seed(id)
    );
    let table = voices(&absent);
    assert!(
        table.contains_key("Melody"),
        "夹具必须有旋律轨: {:?}",
        table.keys().collect::<Vec<_>>()
    );
    assert!(
        table["Melody"].len() > 1,
        "夹具的缺省旋律必须有多条音符: {}",
        table["Melody"].len()
    );
    assert!(
        table.contains_key("Drums"),
        "夹具必须有鼓轨（判据 3 要证它⛔ 被种子影响）"
    );
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        assert!(
            table.contains_key(*part),
            "和声声部 `{part}` 必须在场: {:?}",
            table.keys().collect::<Vec<_>>()
        );
    }
}

/// ⭐ 判据 2（`ARCH-DET` L1/L2）：同一份入参跑两次 ⇒ 逐字节相同。
/// ⚠ 钉的是**值相等**（两次独立规划的字节），⛔ 写成"两次调用返回同一个对象"。
#[test]
fn the_seed_is_idempotent_per_section() {
    for seed in [None, Some(7), Some(u64::MAX)] {
        let first = plan(Given::theory().seed(seed));
        let second = plan(Given::theory().seed(seed));
        assert_eq!(
            first, second,
            "melodySeed={seed:?}：同一份入参的两次规划必须逐字节相同"
        );
    }
}

/// ⭐ 判据 3（D1 + 读数 2）：换 `melodySeed` ⇒ **格位表逐条不变、音高表改变**，
/// 力度只要求仍取阶梯两档（`chord_tone` 由音高派生 ⇒ 力度档**允许**跟着换）；
/// 和声与鼓两轨与缺省跑**逐条相同**（消费面 = 只有旋律读它）。
/// ⚠ 取样是固定的 `SEEDS`（⛔ 按结果挑种子）：其中报错的那些必须核对为 theory 的原样转发。
#[test]
fn a_different_seed_changes_pitches_and_velocity_never_the_grid() {
    let base = plan(Given::theory());
    let base_table = voices(&base);
    let base_melody = &base_table["Melody"];
    let base_grid = grid(base_melody);
    let base_pitches = pitch_bag(base_melody);

    let mut changed = 0_usize;
    let mut forwarded = 0_usize;
    let mut observed: Vec<(u64, usize, Vec<u8>)> = Vec::new();
    for seed in SEEDS {
        let shifted = match try_plan(Given::theory().seed(Some(seed))) {
            Ok(plan) => plan,
            Err(fault) => {
                // 可行性随种子移动（读数 2 末尾）⇒ 这一支必须**如实登记**为原样转发。
                assert_shape_reject(&fault, &format!("melodySeed={seed} 的 theory 失败"));
                let payload = data(&fault);
                assert_eq!(
                    payload["theoryStep"],
                    json!("melody_over_chords"),
                    "melodySeed={seed}：失败必须点名 theory 的阶段（原样转发，⛔ 本层判定种子好坏）: {payload}"
                );
                forwarded += 1;
                continue;
            }
        };
        let shifted_table = voices(&shifted);
        assert_eq!(
            base_table.keys().collect::<Vec<_>>(),
            shifted_table.keys().collect::<Vec<_>>(),
            "melodySeed={seed}：声部集合必须不变"
        );
        for part in preset_parts(STYLE).expect("夹具必须在册") {
            assert_eq!(
                base_table[*part], shifted_table[*part],
                "melodySeed={seed}：和声声部 `{part}` 必须与缺省跑逐条相同（种子⛔ 读和声）"
            );
        }
        assert_eq!(
            base_table["Drums"], shifted_table["Drums"],
            "melodySeed={seed}：鼓声部必须与缺省跑逐条相同（`ADR-0045` N1）"
        );
        let shifted_melody = &shifted_table["Melody"];
        let shifted_grid = grid(shifted_melody);
        assert_eq!(
            base_grid, shifted_grid,
            "melodySeed={seed}：**格位表**（起点 + 时值）必须逐条相同 —— 网格来自 `rhythm_grid`，⛔ 读种子"
        );
        assert_eq!(
            base_melody.len(),
            shifted_melody.len(),
            "melodySeed={seed}：音符条数必须不变（不筛点）"
        );
        for (id, (start, duration, _, velocity)) in base_melody {
            let (shift_start, shift_duration, _, shift_velocity) = shifted_melody[id];
            assert_eq!(
                (start, duration),
                (&shift_start, &shift_duration),
                "melodySeed={seed}：旋律音符 `{id}` 的起点与时值不得因种子改变"
            );
            assert!(
                LADDER.contains(velocity) && LADDER.contains(&shift_velocity),
                "melodySeed={seed}：旋律音符 `{id}` 的力度 {velocity}→{shift_velocity} 必须仍取阶梯两档 {LADDER:?}"
            );
        }
        let shifted_pitches = pitch_bag(shifted_melody);
        observed.push((seed, shifted_pitches.len(), shifted_pitches.clone()));
        if shifted_pitches != base_pitches {
            changed += 1;
        }
    }
    // 夹具必须有可观察性：至少一个取样种子真的换了音高（否则本条是空判据）。
    assert!(
        changed > 0,
        "SEEDS 里必须至少有一个种子真的改变音高多重集（否则本票是空判据）。实测 {} 个报错 / 取样: {observed:?}",
        forwarded
    );
    // ⛔ 声称"每个种子都给不同形状"（那是代理测量 320 组，⛔ 是 MCP 路径的证据）⇒ 只登记计数。
    println!(
        "melodySeed 取样读数：{} 个成功（其中 {changed} 个改变音高表）、{} 个由 theory 报错转发",
        observed.len(),
        forwarded
    );
}

/// ⭐ 判据 4（读数 4 + D1）：非正整数在**工具面**就被拒且报文点名 `melodySeed`
/// （`1.5` / `"7"` 过 `ParamSpec::accepts`，`-1` 过类型闸后由 `arg_u64` 的 `as_u64` 拦下）；
/// 两个端点 `0` 与 `u64::MAX` 必须放行（本键⛔ 有人为上界）。
#[test]
fn non_integer_or_negative_is_rejected_naming_the_key() {
    let mut dispatcher = dispatcher_with_project();
    let auth = format!("Bearer {}", dispatcher.expected_token().expose());

    // 浮点与字符串 ⇒ `ParamSpec::accepts("integer")` 在**协议层**先拒（`-32602`），
    // 请求根本走不到领域层（与 `export_stems.rs` 判据 7 的"两个合法落点"同形）。
    for bad in [json!(1.5), json!("7")] {
        let response = call_raw(&mut dispatcher, &auth, base_args(bad.clone()));
        let detail = protocol_error(response, &format!("melodySeed={bad} 的类型闸"));
        assert!(
            detail.contains("melodySeed"),
            "报文必须点名是哪个键: {detail}"
        );
    }

    // 负数**过**类型闸（`accepts` 用 `as_i64` 认它）⇒ 由领域入口的 `arg_u64`（`as_u64`）
    // 拦下，落点是**带内** `INVALID_PARAMETER_RANGE`（`ADR-0045` D6：⛔ 新码）。
    let response = call_raw(&mut dispatcher, &auth, base_args(json!(-1)));
    let result = in_band(response, "`melodySeed: -1` 的 `arg_u64` 拒绝");
    assert_eq!(
        result["status"],
        json!("error"),
        "`melodySeed: -1` 必须被拒: {result}"
    );
    assert_eq!(
        result["error"]["code"].as_str(),
        Some(ErrorCode::InvalidParameterRange.as_str()),
        "`melodySeed: -1` 必须走既有码 `INVALID_PARAMETER_RANGE`: {result}"
    );
    let text = serde_json::to_string(&result["error"]).expect("序列化错误块");
    assert!(
        text.contains("melodySeed"),
        "拒绝必须点名是哪个键（`arg_u64` 的报文形状）: {result}"
    );

    // 两个端点必须放行（界 = 类型面本身，⛔ 人为上界）。
    for good in [json!(0), json!(u64::MAX)] {
        let response = call_raw(&mut dispatcher, &auth, base_args(good.clone()));
        assert!(
            response.error.is_none(),
            "melodySeed={good} 是界内取值，⛔ 在协议层被拒: {:?}",
            response.error
        );
        let result = response.result.expect("带内响应");
        assert_ne!(
            result["status"].as_str(),
            Some("error"),
            "melodySeed={good} 是界内取值，⛔ 被当成越界拒绝: {result}"
        );
    }
    // 纯领域层的同一读数：两个端点都给得出规划（0..=u64::MAX 全程放行）。
    for raw in [0u64, u64::MAX] {
        try_plan(Given::theory().seed(Some(raw)))
            .unwrap_or_else(|fault| panic!("melodySeed={raw} 在界内，必须放行: {fault:?}"));
    }
}

/// ⭐ 判据 5（D3 的消费面）：脱离 `materialSource:"theory"` ⇒ 拒且 `hint` 点名
/// `materialSource`；`includeMelody:false` ⇒ 拒且 `hint` 点名 `includeMelody`；
/// ⚠ **反钉**：`includeDrums:false` 时必须放行（本键⛔ 读鼓）。
#[test]
fn the_seed_key_requires_the_theory_source_and_the_melody_part() {
    for material_source in [None, Some("project")] {
        let fault = try_plan(
            Given::theory()
                .material_source(material_source)
                .seed(Some(7)),
        )
        .expect_err("`melodySeed` 不得脱离 materialSource:\"theory\" 单独给");
        assert_shape_reject(&fault, "`melodySeed` 脱离 theory");
        assert!(
            data(&fault)["hint"]
                .as_str()
                .is_some_and(|text| text.contains("materialSource")),
            "载荷必须指出要补 materialSource: {}",
            data(&fault)
        );
    }
    let fault = try_plan(Given::theory().seed(Some(7)).melody(false))
        .expect_err("`melodySeed` 不得在没有旋律声部时静默通过");
    assert_shape_reject(&fault, "`melodySeed` 脱离 includeMelody");
    assert!(
        data(&fault)["hint"]
            .as_str()
            .is_some_and(|text| text.contains("includeMelody")),
        "载荷必须指出要补 includeMelody: {}",
        data(&fault)
    );
    // 反钉：要鼓而⛔ 要鼓不是本键的前提；⛔ 鼓在场才能给本键。
    try_plan(Given::theory().seed(Some(7)).drums(false)).unwrap_or_else(|fault| {
        panic!("`melodySeed` 与鼓无关：includeDrums:false 必须放行（D3 的反钉）: {fault:?}")
    });
}

/// ⭐ 判据 6（D1/D7）：两个声明面同源在场，契约里⛔ `default`、⛔ `maximum`、
/// `minimum: 0` 在册，且两侧的说明文字⛔ 出现"随机"（D7：它是确定性取音的输入）。
#[test]
fn melody_seed_is_declared_on_both_param_surfaces_without_default_or_maximum() {
    let spec = TOOLS
        .iter()
        .find(|tool| tool.name == "yeban_propose_section")
        .expect("注册表里必须有 `yeban_propose_section`");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("仓库根目录");
    let text = std::fs::read_to_string(repo.join("schemas").join("mcp-tools.schema.json"))
        .expect("读契约 schema");
    let contract: Value = serde_json::from_str(&text).expect("契约必须是合法 JSON");
    let declared = &contract["definitions"]["DocumentedToolArguments"]["$defs"]["yeban_propose_section"]
        ["properties"];

    let param = spec
        .param("melodySeed")
        .expect("参数表必须声明 `melodySeed`");
    assert_eq!(param.json_type, "integer", "`melodySeed` 必须是整数参数");
    assert!(!param.required, "缺省 = 派生种子 ⇒ 必须可选");
    assert!(
        param.doc.contains("ADR-0045") && param.doc.contains("确定性"),
        "参数说明必须写清出处与确定性语义: {}",
        param.doc
    );
    assert!(
        !param.doc.contains("随机"),
        "D7：参数说明⛔ 把本键说成\"随机\"（它是确定性取音的输入）: {}",
        param.doc
    );

    let property = &declared["melodySeed"];
    assert!(
        !property.is_null(),
        "契约 JSON 必须登记 `melodySeed` 的属性: {text}"
    );
    assert_eq!(property["type"], json!("integer"));
    assert_eq!(
        property["minimum"],
        json!(0),
        "下界 0 必须登记（`arg_u64` 真实拒绝的那一支）"
    );
    assert!(
        property.get("maximum").is_none(),
        "契约⛔ 写 `maximum`：本键没有自然上界，写死一个数会造出\"契约说能、本层拒\"的矛盾: {property}"
    );
    assert!(
        property.get("default").is_none(),
        "契约⛔ 写 `default`：缺省是随段落身份移动的派生值: {property}"
    );
    let doc = property["description"].as_str().expect("description");
    assert!(
        !doc.contains("随机"),
        "D7：契约说明⛔ 出现\"随机\"这类词: {doc}"
    );
    assert!(
        doc.contains("ADR-0045") && doc.contains("音高"),
        "契约说明必须写清出处与\"换音高\"的语义: {doc}"
    );
}
