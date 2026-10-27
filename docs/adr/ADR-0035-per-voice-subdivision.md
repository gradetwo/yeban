# ADR-0035 — 逐声部节奏粒度（`voiceSubdivisions`）：均匀切分按**声部名**给值

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ARCH-DET-001`（L1 确定性）、
  `ADR-0016`（缺省逐字节兼容契约）、`ADR-0028`（音域 + `drumDensity`）、
  `ADR-0033`（`voiceSubdivision`，本票直接接它的 **D4 第 1 条**："每声部独立粒度……
  需要'按声部名/序给值'的形状选择，留给后续票"）；
  `crates/yeban-mcp/src/domain/section_build.rs`（`theory_contents`、`PlanOptions`、
  `plan_with_options`、`PARTS_THREE` / `PARTS_FOUR`）、`crates/yeban-mcp/src/tools.rs`、
  `schemas/mcp-tools.schema.json`
- **日期**: 2026-10-28

## 背景

`ADR-0033` 落地的是一条**全局**均匀切分：`voiceSubdivision: N` 把**每个**和声声部的
每个 `ChordSpan` 切成 N 条。实测（`section_build.rs:1322` `theory_contents`）该参数
被原样传进 3/4 声部的循环 ⇒ 三个声部**必然同粒度**。

审计 G8 的原始诉求与此不符。它举的形状是"**bass 切 8、pad 持续**"（`ADR-0033` D4
第 1 条的例句）—— 那是 ostinato 铺底 + 长音旋律线的最小形状，也是编曲里最常见的
和声节奏：低音走脉冲、内声部填缝、高声部保持。今天的调用方要拿到它，只能
生成后再对不想要的那几条轨逐音符 `edit_notes`（而 `edit_notes` 走提案，两轮往返）。

代码事实（本票开工前实测）：

| 事实 | 证据 |
| --- | --- |
| 声部名是**闭合小集合**：3 声部 `["Bass","Alto","Soprano"]`、4 声部 `["Bass","Tenor","Alto","Soprano"]` | `section_build.rs:161`、`:164` |
| 声部在 `theory_contents` 里按**下标** `0..voice_count` 迭代，下标即 `preset_parts` 的顺序 | `section_build.rs:1349`、`plan_with_options:1641` |
| 每个声部的 id 种子已含 `{voice}` 下标 ⇒ 改一个声部的粒度**不会**与另一个声部撞身份 | `section_build.rs:1361-1365` |
| `voice_count` 只数**和声声部**（鼓/旋律在其后 `push`，不在这段循环里） | `section_build.rs:1690-1748` |
| `PlanOptions` 是生成时临时结构，⛔ 不落 `yeban-model` | `section_build.rs:1084-1101` |

⇒ 缺口不在能力，在**给值的形状**：一个数 ⇒ 所有声部；需要"按声部给数"。

## 决定

### D1 新可选对象参数 `voiceSubdivisions`（按声部名给值；缺省 = 逐字节不变）

`yeban_propose_section` 新增可选对象参数：

```json
{"voiceSubdivision": 4, "voiceSubdivisions": {"Soprano": 1}}
```

- 键 = **声部名**（`"Bass"` / `"Tenor"` / `"Alto"` / `"Soprano"`），值 = **整数 1..=16**，
  单位与 `ADR-0033` 完全一致：每个 `ChordSpan` 切成几条。
- **组合规则（唯一一条，⛔ 没有第三种解释）**：出现在表里的声部用表里的值；
  **没出现的声部沿用 `voiceSubdivision`**（其缺省为 1）。于是
  `voiceSubdivision:4 + {"Soprano":1}` = `Bass/Alto 切 4、Soprano 持续`；
  只给 `{"Bass":8}` = `Bass 切 8、其余 1`。
- 展开仍在 `theory_contents` 内做，逐声部取自己的 N：`step`、"最后一条吸收余数"、
  力度恒 `THEORY_VELOCITY` 全部沿用 `ADR-0033` D1 —— 本票⛔ 改任何一条算式，
  只把"一个 N"换成"每声部一个 N"。
- 作用面与 `ADR-0033` 同：只作用于 `materialSource:"theory"` 的**和声声部**；
  鼓（已有 `drumDensity`）、旋律（`THEORY_ONSETS_PER_BAR` 网格，见 D4）**不受影响**。

**为何是对象、⛔ 数组或字符串**：数组（`[8,2,1]`）把语义绑死在**下标**上，而声部数随
流派预设变（3 或 4）—— 同一个数组在两个流派里指不同的声部，是"看起来能用、实际会指错"
的形状。字符串（`"Bass=8,Alto=2"`）要把一门小语法写进参数说明并自己解析。对象按名字给值
对两种预设都成立，且 `additionalProperties` 类守卫在本仓已是现成口径。

**落地形状**（`PlanOptions.voice_subdivisions: Option<&'a Value>`，⛔ 预解析好的表）：
原始对象被**原样转发**到领域层，由 `voice_subdivision_table` 一次展开成与和声声部同长的
`Vec<u64>`。理由是"一个文件同一时刻只有一个写者"的参数版：`D2` 那三行拒绝都要求载荷
点名是**哪个声部**，而只有领域层同时握着该预设的声部清单 —— 在实参读取层预解析成
`BTreeMap<String, u64>` 会把"非整数"那一行推到 `domain/mod.rs`，同一张错误表就分成两个
写者，两份载荷形状也必然分叉。`&Value` 让 `PlanOptions` 保住 `Copy`（既有 21 处夹具
逐字面构造它，⛔ 改成 `.clone()` 或 `..Default::default()`）。

### D2 非法形状一律响亮，且**点名是哪个声部**

| 输入 | 结果 |
| --- | --- |
| 键不是该流派的声部名（含 `"Drums"` / `"Melody"`） | `INVALID_PARAMETER_RANGE`，载荷带 `part` 与 `allowedParts`（= `preset_parts(stylePreset)` 的实测清单） |
| 值不是整数，或越出 `1..=16` | `INVALID_PARAMETER_RANGE`，载荷带 `part` / `value` / `range: [1, 16]`（⚠ 落地时把 `voiceSubdivisions[part]` 改写成了 `value`：JSON 里带变量键要靠手工拼 `Map`，而 `part` 已经把键名点出来了，回显值不需要再嵌一层） |
| 整个参数不是对象（数组 / 字符串 / 数字 / `null`） | 领域层 ⇒ `INVALID_PARAMETER_RANGE`（带期望形状的 `hint`）；⚠ **工具面**上这一行由 `ParamSpec::accepts` 先接手（`json_type: "object"`），报的是参数类型错误而不是领域码 —— 两条路都响亮，判据在**纯领域层**量这一行 |
| 给了它但 `materialSource` ⛔ 是 `"theory"` | `INVALID_PARAMETER_RANGE` + `hint`（与 `voiceSubdivision`、`drumDensity` 的"给了不消费的参数"同口径） |
| 空对象 `{}` | **合法**：没有任何覆盖 ⇒ 与"不给该键"逐字节相同 |

⛔ 静默忽略未知声部名：`"Drums"` 是调用方最可能误写的键（它确实是个轨，只是不由本参数
管），把它读成"没有覆盖"会让调用方以为 bass 铺底已经生效。

### D3 缺省逐字节不变（含 id 种子，按**声部**判定）

`ADR-0033` D3 的种子规则原样保留，只是把条件从"整个请求"下放到"每个声部"：
某声部 `N == 1` ⇒ 两段式种子 `"theory-note:{section}:{voice}:{chord}"`；
`N > 1` ⇒ 三段式 `"…:{chord}:{part}"`。
⇒ 不给 `voiceSubdivisions` 时逐字节不变（全部声部落回 `voiceSubdivision`，其缺省为 1）；
给了但某个声部没列出且 `voiceSubdivision` 缺省时，**那个声部的音符身份与今天完全相同**。
这条是 `ARCH-DET-001` 与"缺省兼容契约"的交点：粒度改变⛔ 得连带翻动没被改的声部的 id。

### D4 不在本票范围（登记，不发明）

1. **旋律的每小节 onset 网格**：旋律走 `rhythm_grid(_, THEORY_ONSETS_PER_BAR=16)`
   （`section_build.rs:1493`），把 16 变成可传值是**另一个参数**，且它与
   `melody_over_chords` 的选音相互耦合（判据 `melody_stays_on_the_genre_grid_and_in_range_in_a_dag`
   钉着网格对齐）—— 另立一票。
2. **切分内的重音模式**（哪些切点有力差）= `ADR-0033` D4 第 3 条，会改力度面，
   与 `ADR-0026` 分级都要重新核对 ⇒ 本票继续**力度恒 96**。
3. **非均匀节奏型表**（`ADR-0033` D4 第 2 条）= 需要新数据表与规范口径，⛔ 不发明。
4. **段级调号 / 调内收拢的转调 / 平滑声部连接** = `ADR-0034` D4，三条待人类裁决，
   与本票无关（本票⛔ 读 `key_signature`）。

## 判据（落地时随代码一起提交）

| # | 判据 | 它量什么 | 钉住的决定 |
| --- | --- | --- | --- |
| 1 | `voice_subdivisions_absent_is_byte_identical_to_today` | 不给该键 ⇒ 工程摘要与 `ADR-0033` 那条路径逐字节相同 | D3 |
| 2 | `voice_subdivisions_apply_to_the_named_voice_only` | `{"Bass":4}` ⇒ 只有 bass 轨每跨度 4 条，其余声部仍 1 条 | D1 |
| 3 | `voice_subdivisions_compose_over_the_uniform_knob` | `voiceSubdivision:4 + {"Soprano":1}` ⇒ 逐声部 `4/4/1` | D1 组合规则 |
| 4 | `voice_subdivisions_preserve_every_span_without_overlap_per_voice` | 每个被切的声部各自跨度总时值守恒、无缝无叠 | D1（沿用 D33 算式） |
| 5 | `voice_subdivisions_unknown_part_names_the_allowed_parts` | `{"Drums":2}` ⇒ `INVALID_PARAMETER_RANGE` + `part` + `allowedParts` | D2 |
| 6 | `voice_subdivisions_out_of_range_and_non_integer_are_loud` | `{"Bass":0}` / `{"Bass":17}` / `{"Bass":2.5}` ⇒ `INVALID_PARAMETER_RANGE` 带 `range` | D2 |
| 7 | `voice_subdivisions_require_theory_material_source` | 与 `materialSource` 的耦合：非 theory 给了 ⇒ `hint` 拒绝 | D2 |
| 8 | `voice_subdivisions_keep_untouched_voices_note_ids_stable` | 没被改的声部音符 id 与缺省跑逐字节相同 | D3 |
| 9 | `voice_subdivisions_empty_object_is_the_default` | `{}` == 不给 | D2 |
| 10 | `voice_subdivisions_are_deterministic_across_two_calls` | 同一请求两次 ⇒ 同一份载荷（`ARCH-DET-001`），且对象里键的书写顺序不参与身份 | D1 |
| 11 | `voice_subdivisions_is_declared_on_both_param_surfaces` | `tools.rs` 的 `ParamSpec` 与契约 JSON 都登记该键，上界等于领域层的 `MAX_VOICE_SUBDIVISION`（⛔ 两份数字） | D1/D2 |

⚠ 判据里**所有**声部清单都从 `preset_parts(style)` 读回来比对（`allowedParts` 那条尤其
如此）—— 手抄一份 `["Bass","Alto","Soprano"]` 会让"清单来自预设"这件事在声部表被改动时
变成空判据（`G18b` 那条"守卫的语义不能靠手抄名单"的教训，同族）。
