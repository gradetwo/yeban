//! `ADR-0040`（`ADR-0039` 非范围 `N1` 的兑现票）：`yeban_propose_section` 的
//! `harmonyMaxVoiceJump` —— **和声声部在相邻两个和弦之间**的跳进上界，从 theory 的出厂常量
//! `VoicingConstraints::{THREE,FOUR}_VOICES.max_voice_jump`（现读同为 12）换成可给的入参，
//! 缺省 ⇒ 逐字节不变。
//!
//! 单位口径 = **半音距离**（`0..=127`），⛔ 不是 MIDI 音高值、⛔ 不是八度号、⛔ 不是
//! `melodyMaxLeap`（那一个管**旋律线**里相邻两音，本键管**同一和声声部**在前后两个和弦里的音高差）。
//! 上界为什么是 `127`：单个声部的两个音都在 `0..=127` 内 ⇒ 半音距离在任何输入下都不可能超过
//! 127（`ADR-0040` D2）；本层⛔ 另立一张表。
//!
//! ⚠ **这把尺子与旋律那两票有一处实测差异，判据因此⛔ 照抄**：缺省 12 在真实进行上只打到
//! `2..=3` 半音（本机探针现读：`i-VI-III-VII` 上 `Bass=2 / Alto=1 / Soprano=3`），⛔ 旋律那条
//! "缺省就打满 12"。所以本文件⛔ 写"实测跳进 == 入参"，只写 **≤ 入参** ＋
//! "足够紧的值确实改变输出"（判据 2）与"比实际所需更松的取值不改变输出"（同一条，读数 4）。
//!
//! ⚠ **可行性边界随进行移动**（`ADR-0040` D4，实测：`1` 与 `0` 在 `i-VI-III-VII` 与
//! `i-vii-i-vii` 上都无解，而 `0` 在 `i-i-i-i`（同和弦反复）上有解）⇒ 本层⛔ 预判、⛔ 把
//! "某个值必红/必绿"钉成常量；判据 5 因此**两个方向都要量**（同一个 `0` 一处绿一处红），
//! 那才是"唯一来源是 theory 的 `NoFeasibleVoicing`"的机械证据。
//!
//! ⚠ **消费口径比旋律三键松**（D3）：和声声部在 theory 路径上恒在场、恒被消费 ⇒ 只要求
//! `materialSource:"theory"`，⛔ **不**要求 `includeMelody` / `includeDrums`。判据 4 把这条
//! 松口径**反钉**成"两个可选声部都不给时必须放行" —— 否则"把旋律那条更严的要求扩到本键"
//! 这类改动会安静地通过。
//!
//! ⚠ 分层如实：`非整数` / `负数` 在**工具面**就被拦下（`arg_u64` 与 `ParamSpec::accepts("integer")`），
//! 纯领域层量不到 ⇒ 本文件把它写成判据 6 里"`json_type` 必须是 integer"（与 `ADR-0036`..
//! `ADR-0039` 判据 6 同一条分工）。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::voice_leading::VoicingConstraints;

/// 夹具的小节数。
const BARS: u64 = 4;
/// 夹具的风格预设：3 声部（`Bass` / `Alto` / `Soprano`）。
const STYLE: &str = "synthwave";
/// 夹具的调：C 自然小调。
const KEY: &str = "C minor";
/// 夹具的进行：缺省实测跳进 `Bass=2 / Alto=1 / Soprano=3` ⇒ 收到 `2` 真的会改音高。
const PROGRESSION: &str = "i-VI-III-VII";
/// 同和弦反复的进行：任何上界（含 `0`）都有解 ⇒ 判据 5 的"绿"那一半。
const STATIC: &str = "i-i-i-i";
/// 会红的那档上界（半音距离）：实测把 `Soprano` 的 3 半音压到 1，改掉 2 条音符。
const TIGHT: u64 = 2;
/// 比实际所需更松的一档（半音距离）：实测与缺省逐条相同（读数 4）。
const LOOSE: u64 = 5;
/// theory 的出厂上界（⛔ 在本文件再抄一个 `12` —— 判据 6 用它核契约里的 `default`）。
const PUBLISHED: u64 = VoicingConstraints::THREE_VOICES.max_voice_jump as u64;
/// 和声音符的力度（`ADR-0037` 阶梯的下档：`theory_contents` 对所有和声音取 `ladder.0`）。
const HARMONY_VELOCITY: u8 = 96;

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`，按**音符 id** 索引。
type Note = (u64, u64, u8, u8);

/// 一次请求的可给项（缺省 = theory + 鼓 + 旋律 + 夹具进行，本键不给）。
#[derive(Clone, Copy)]
struct Given {
    material_source: Option<&'static str>,
    progression: &'static str,
    include_drums: bool,
    include_melody: bool,
    harmony_max_voice_jump: Option<u64>,
}

impl Given {
    fn theory() -> Self {
        Self {
            material_source: Some("theory"),
            progression: PROGRESSION,
            include_drums: true,
            include_melody: true,
            harmony_max_voice_jump: None,
        }
    }

    fn jump(mut self, value: Option<u64>) -> Self {
        self.harmony_max_voice_jump = value;
        self
    }

    fn progression(mut self, value: &'static str) -> Self {
        self.progression = value;
        self
    }

    fn material_source(mut self, value: Option<&'static str>) -> Self {
        self.material_source = value;
        self
    }

    fn drums(mut self, value: bool) -> Self {
        self.include_drums = value;
        self
    }

    fn melody(mut self, value: bool) -> Self {
        self.include_melody = value;
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
            progression: Some(given.progression),
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
            harmony_max_voice_jump: given.harmony_max_voice_jump,
            swing_permille: None,
            drum_backbeat: None,
        },
    )
}

fn plan(given: Given) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "theory 规划（harmonyMaxVoiceJump={:?}）必须成功: {fault:?}",
            given.harmony_max_voice_jump
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

/// 某声部**按起点排序后**相邻两音的最大半音距离（单位 = 半音距离）。
///
/// ⚠ 必须先按起点排序：音符表按 **id** 索引（id 是确定性哈希，⛔ 时间序），直接用迭代器
/// 顺序会把两条不相邻的和弦当作一对来量。
fn max_jump(table: &BTreeMap<String, Note>) -> u64 {
    let mut starts: Vec<(u64, u8)> = table.values().map(|note| (note.0, note.2)).collect();
    starts.sort_unstable();
    starts
        .windows(2)
        .map(|pair| pair[0].1.abs_diff(pair[1].1) as u64)
        .max()
        .unwrap_or(0)
}

/// 失败载荷的 `data`（拒绝类判据都要读它）。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 断言一条拒绝：码 = `INVALID_PARAMETER_RANGE`，且载荷⛔ 带 `reason` 键
/// （`ADR-0040` D7 继承 `ADR-0038` D4 ⇒ `fault_vocabulary` 的词表与规范面摘要不变）。
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

/// ⭐ 判据 1（`ADR-0040` D1）：**键缺席 ≡ 给出厂上界**，整个规划逐字节相同
/// （`SectionPlan: PartialEq`）。⚠ ⛔ 断言"缺省实测跳进 == 出厂值"（读数 2：缺省 12 在真实
/// 进行上只打到 3，那条断言会红）；这里改钉"夹具确实存在 > `TIGHT` 的跳进"，否则判据 2 是空判据。
#[test]
fn absent_jump_is_byte_identical_to_the_published_bound() {
    let absent = plan(Given::theory());
    let explicit = plan(Given::theory().jump(Some(PUBLISHED)));
    assert_eq!(
        absent, explicit,
        "键缺席必须逐字节等于给 `{{\"harmonyMaxVoiceJump\": {PUBLISHED}}}`（`ADR-0016` 的缺省兼容契约）"
    );
    let table = voices(&absent);
    let mut observed: Vec<(String, u64)> = Vec::new();
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        let harmony = &table[*part];
        // 夹具必须真的产出跨多个和弦的声部（否则判据 2/5 都是空判据）。
        assert!(
            harmony.len() > 1,
            "夹具的和声声部 `{part}` 必须有多于一层和弦: {harmony:?}"
        );
        let jump = max_jump(harmony);
        assert!(
            jump <= PUBLISHED,
            "缺省和声声部 `{part}` 的跳进 {jump} 必须 ≤ 出厂上界 {PUBLISHED}"
        );
        for (_, _, _, velocity) in harmony.values() {
            assert_eq!(
                *velocity, HARMONY_VELOCITY,
                "缺省和声音符 `{part}` 的力度必须取阶梯下档 {HARMONY_VELOCITY}"
            );
        }
        observed.push(((*part).to_owned(), jump));
    }
    let tightest = observed.iter().map(|(_, jump)| *jump).max().unwrap_or(0);
    assert!(
        tightest > TIGHT,
        "夹具的缺省和声必须真的出现 > {TIGHT} 的跳进，否则'收紧到 {TIGHT}'是空判据: {observed:?}"
    );
}

/// ⭐ 判据 2（D1/D5 的映射面 + "只作用于和声"）：
/// * 收紧到 `TIGHT` ⇒ 实测跳进 ≤ 请求上界、**至少一层和声的音高被改**（防空判据）；
/// * 放宽到 `LOOSE`（比实际所需更松）⇒ 与缺省**逐条相同**（读数 4：上界⛔ 保证打满，
///   "设了没变化"在这里是**正确语义**，把它钉住才能挡住"为了看起来有效而静默改写音高"）；
/// * 两档下：旋律与鼓与缺省跑**逐字节**相同、和声声部 id / 起点 / 时值逐条对齐、
///   和弦层数不变（⛔ 不筛和弦、⛔ 不改时值）、和声力度仍取阶梯下档。
#[test]
fn a_tight_jump_moves_only_the_harmony_pitches() {
    let base = plan(Given::theory());
    let base_table = voices(&base);

    for (bound, expect_change) in [(TIGHT, true), (LOOSE, false)] {
        let shifted = plan(Given::theory().jump(Some(bound)));
        let shifted_table = voices(&shifted);
        assert_eq!(
            base_table.keys().collect::<Vec<_>>(),
            shifted_table.keys().collect::<Vec<_>>(),
            "上界 {bound}：声部集合必须不变"
        );
        for part in ["Melody", "Drums"] {
            assert_eq!(
                base_table[part], shifted_table[part],
                "上界 {bound}：声部 `{part}` 必须与缺省跑逐条相同（本键只作用于和声声部）"
            );
        }
        let mut moved: Vec<String> = Vec::new();
        for name in preset_parts(STYLE).expect("夹具必须在册") {
            let (base_voice, shifted_voice) = (&base_table[*name], &shifted_table[*name]);
            assert_eq!(
                base_voice.keys().collect::<Vec<_>>(),
                shifted_voice.keys().collect::<Vec<_>>(),
                "上界 {bound}：和声声部 `{name}` 的音符 id 集合必须⛔ 不受跳进上界影响（不筛和弦）"
            );
            for (id, (start, duration, pitch, velocity)) in base_voice {
                let (shift_start, shift_duration, shift_pitch, shift_velocity) = shifted_voice[id];
                assert_eq!(
                    (start, duration),
                    (&shift_start, &shift_duration),
                    "上界 {bound}：和声音符 `{id}`（声部 `{name}`）的起点与时值不得因跳进上界改变"
                );
                assert_eq!(
                    *velocity, HARMONY_VELOCITY,
                    "上界 {bound}：缺省跑的和声音符 `{id}` 力度本就该是 {HARMONY_VELOCITY}"
                );
                assert_eq!(
                    shift_velocity, HARMONY_VELOCITY,
                    "上界 {bound}：和声音符 `{id}` 的力度必须仍是阶梯下档 {HARMONY_VELOCITY}"
                );
                if *pitch != shift_pitch {
                    moved.push(format!("{name}:{id}"));
                }
            }
            let observed = max_jump(shifted_voice);
            assert!(
                observed <= bound,
                "上界 {bound}：声部 `{name}` 的实测跳进 {observed} 必须 ≤ 请求上界（theory 明文⛔ 静默放宽）"
            );
        }
        if expect_change {
            assert!(
                !moved.is_empty(),
                "上界 {bound}：必须真的改变至少一层和声的音高（否则正例是空判据）"
            );
        } else {
            assert!(
                moved.is_empty(),
                "上界 {bound} 比夹具实际所需更松 ⇒ 输出必须与缺省逐条相同（读数 4），却改了: {moved:?}"
            );
        }
    }
}

/// ⭐ 判据 3（D1/D2 的形状面）：越界（`128` / 超出 `u8` 宽度）⇒ 点名是哪个键，
/// 载荷带 `value` + `range`，⛔ `reason` 键。`127` 在界内 ⇒ 必须放行；`0` 也在界内 ⇒
/// ⛔ 被**参数层**拒（它在夹具进行上会转成判据 5 的 theory 转发，那是另一条码路径）。
#[test]
fn out_of_range_is_rejected_naming_the_key() {
    for raw in [128u64, 1_000, u64::from(u8::MAX) + 1] {
        let fault = try_plan(Given::theory().jump(Some(raw)))
            .expect_err("越界不得静默夹紧（`ADR-0036` D3 的同一条口径）");
        assert_shape_reject(&fault, &format!("`harmonyMaxVoiceJump` 越界 ({raw})"));
        let payload = data(&fault);
        assert_eq!(
            payload["field"],
            json!("harmonyMaxVoiceJump"),
            "载荷必须点名是哪个键: {payload}"
        );
        assert_eq!(
            payload["value"],
            json!(raw),
            "载荷必须回显被拒的值: {payload}"
        );
        assert_eq!(
            payload["range"],
            json!([0, 127]),
            "载荷必须给出界（`0..=MIDI_PITCH_MAX`）: {payload}"
        );
    }
    // `127` = "实际不设限"，是语义而⛔ 越界 ⇒ 必须放行。
    try_plan(Given::theory().jump(Some(127)))
        .expect("`harmonyMaxVoiceJump` = 127 在 0..=127 内，必须放行");
    // `0` 同样⛔ 是参数层的拒绝：它要么成功，要么走 theory 转发（载荷⛔ 带 `field`）。
    if let Err(fault) = try_plan(Given::theory().jump(Some(0))) {
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "`harmonyMaxVoiceJump` = 0 只能成功或由 theory 转发: {fault:?}"
        );
        assert!(
            data(&fault).get("field").is_none(),
            "`0` 在界内 ⇒ ⛔ 参数层的拒绝（这条只能是 theory 的可行性判定）: {}",
            data(&fault)
        );
    }
}

/// ⭐ 判据 4（D3 的消费面，**两向都钉**）：本键⛔ 脱离 `materialSource:"theory"`（含 `hint`
/// 点名要补的键）；同时 `includeMelody:false` **且** `includeDrums:false` 时给本键**必须放行**
/// —— 和声声部恒在场，⛔ 把旋律三键那条更严的要求扩散过来（扩过来会多造一个文档里没写的拒绝）。
#[test]
fn the_harmony_key_requires_only_the_theory_source() {
    for material_source in [None, Some("project")] {
        let fault = try_plan(
            Given::theory()
                .material_source(material_source)
                .jump(Some(TIGHT)),
        )
        .expect_err("`harmonyMaxVoiceJump` 不得脱离 materialSource:\"theory\" 单独给");
        assert_shape_reject(&fault, "`harmonyMaxVoiceJump` 脱离 theory");
        assert!(
            data(&fault)["hint"]
                .as_str()
                .is_some_and(|text| text.contains("materialSource")),
            "载荷必须指出要补 materialSource: {}",
            data(&fault)
        );
    }
    // 反钉：两个可选声部都不给 ⇒ 必须放行（只剩和声声部，本键就在消费它）。
    for given in [
        Given::theory().jump(Some(TIGHT)).melody(false),
        Given::theory().jump(Some(TIGHT)).melody(false).drums(false),
    ] {
        try_plan(given).unwrap_or_else(|fault| {
            panic!(
                "`harmonyMaxVoiceJump` ⛔ 要求旋律/鼓声部在场（drums={} melody={}）: {fault:?}",
                given.include_drums, given.include_melody
            )
        });
    }
}

/// ⭐ 判据 5（D4）：可行性唯一来源是 theory，且**边界随进行移动** ⇒ 本层⛔ 抄那张
/// 「进行 × 上界」表。同一个 `0`：在夹具的进行（`PROGRESSION`）上由 theory 判无解并**原样转发**
/// （`theoryStep:"voice_leading::realize"` + `stylePreset` + `theoryError`，载荷⛔ `field` ⇒
/// 不是参数层的拒绝），在同和弦反复的进行（`STATIC`）上必须**成功**且和弦不动。
#[test]
fn feasibility_is_forwarded_from_theory_and_moves_with_the_progression() {
    for (progression, bound) in [(PROGRESSION, 0u64), (PROGRESSION, 1), ("i-vii-i-vii", 0)] {
        let fault = try_plan(Given::theory().progression(progression).jump(Some(bound)))
            .expect_err("夹具的进行上该上界无解（实测），必须报错而⛔ 静默放宽或产出空和声");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "theory 的无解必须转发成 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        let payload = data(&fault);
        assert_eq!(
            payload["theoryStep"],
            json!("voice_leading::realize"),
            "转发必须点名 theory 的哪一步失败: {payload}"
        );
        assert_eq!(
            payload["stylePreset"],
            json!(STYLE),
            "转发必须带上风格预设: {payload}"
        );
        assert!(
            payload["theoryError"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "转发的 `theoryError` 必须是可读的非空字符串: {payload}"
        );
        assert!(
            payload.get("field").is_none(),
            "这是可行性判定而⛔ 参数越界 ⇒ 载荷⛔ 带 `field`: {payload}"
        );
        assert_shape_reject(&fault, &format!("`{progression}` 上界 {bound} 无解"));
    }
    // 同一个 `0` 在同和弦反复上进行必须绿（读数 3 的另一半）⇒ 钉住"⛔ 把某个值钉成必红"。
    let repeated = plan(Given::theory().progression(STATIC).jump(Some(0)));
    let table = voices(&repeated);
    for name in preset_parts(STYLE).expect("夹具必须在册") {
        assert_eq!(
            max_jump(&table[*name]),
            0,
            "`{STATIC}` 上界 `0` 必须放行且声部 `{name}` 完全不动"
        );
    }
}

/// ⭐ 判据 6：两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON），且契约的 `default` 与
/// theory 的**两份**出厂值（3 / 4 声部）是同一组数字 —— 本层⛔ 第三个写者（`ADR-0040` D1/D5）；
/// 类型必须是 integer（工具面的"非整数 / 负数"由这道类型闸承担 —— 见文件头）。
#[test]
fn jump_bound_is_declared_on_both_param_surfaces() {
    assert_eq!(
        VoicingConstraints::THREE_VOICES.max_voice_jump,
        VoicingConstraints::FOUR_VOICES.max_voice_jump,
        "`ADR-0040` D5 的接线前提：3 与 4 声部出厂值必须同数（现读实测），否则'键缺席 ≡ 一个值'不成立"
    );
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
        .param("harmonyMaxVoiceJump")
        .expect("参数表必须声明 `harmonyMaxVoiceJump`");
    assert_eq!(
        param.json_type, "integer",
        "`harmonyMaxVoiceJump` 必须是整数参数"
    );
    assert!(!param.required, "缺省 = 出厂上界 ⇒ 必须可选");
    assert!(
        param.doc.contains("ADR-0040") && param.doc.contains("半音距离"),
        "参数说明必须写清单位与出处: {}",
        param.doc
    );
    let property = &declared["harmonyMaxVoiceJump"];
    assert!(
        !property.is_null(),
        "契约 JSON 必须登记 `harmonyMaxVoiceJump` 的属性: {text}"
    );
    assert_eq!(property["type"], json!("integer"));
    assert_eq!(
        property["minimum"],
        json!(0),
        "下界必须是 0（和弦不动合法）"
    );
    assert_eq!(
        property["maximum"],
        json!(127),
        "上界必须与 `MIDI_PITCH_MAX` 同一组数字（半音距离⛔ 可能超过音高全域宽度）: {property}"
    );
    assert_eq!(
        property["default"],
        json!(PUBLISHED),
        "缺省必须与 theory 的出厂值同源（⛔ 两份数字）: {property}"
    );
}
