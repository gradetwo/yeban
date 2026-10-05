# schema-ratchet 工作线 notes —— **契约 `required` ⇔ 实现必需性** 的常设棘轮

> 工作线: `line/schema-ratchet`（基线 `main` `113d8a4`，已 merge `origin/main` `0ef7eb2`）
> 拥有: `crates/yeban-model/**`；**只读** `schemas/**`（本线一只手指都没碰它 —— 全部临时注入都已还原并复核 sha256）
> 新增: `crates/yeban-model/tests/schema_ratchet.rs`、本文件
> 交付使命（来自 `line/model-schema-d43` 上报的 needs）: 把「契约（`schemas/project.schema.json`）
> 的 `required` 与实现的必需性对账」做成**常设判据**，让这一类漂移**不可能再发生**。

---

## 0. 一句话结论

**契约与实现的必需性现在被 9 条机械判据双向钉住**，这 9 条**直接读契约产物**
（`schemas/project.schema.json`）与**实现本身**（serde 反序列化行为），
**一个 `required` 名都不手抄** —— 契约改动一次，判据当场重判一次。

对着**最终形态**（`main` 已合并契约收紧，`schemas/project.schema.json` sha256
`5eb3362ab3fb7efc9ee879a173eff30ded48b560ba01e73524114002f0f47e59`，根 `required` **18 键**）
实测：**9/9 绿**，覆盖 **134 条 `required` 路径**（其中必达 91 条）、**5 个 `oneOf` 组**、
§5.4 的 18 项豁免 **18/18 被契约声明、0 项被 `required`**、
**「实现要求但契约没要求」= 0 条**（判据 ⑥ 现在是硬等式，没有过渡豁免）。

本轮的三条关键实测（都是**当场跑过的命令**，见 §5、§7）：

| 事实 | 机械证据 |
| :--- | :--- |
| **"schema 比实现更紧"能被抓** | 把豁免字段 `notes[*].probability` 加进 `required` ⇒ `validate_schemas.py` **exit 1**；本线判据 ⑤+② **红**并点名路径 |
| **"schema 比实现更松"原来发现不了** | 根 `required` 删掉 `rng_seed` ⇒ `validate_schemas.py` **exit 0（没发现）**，而本线判据 ⑥ **红**并点名 `#.rng_seed` |
| **棘轮自己会红** | 实现侧给 `TrackV3::solo_safe` 加回 `#[serde(default)]` ⇒ 判据 ③ **红**，逐 track 实例点名 `#.tracks.*.solo_safe` |

---

## 1. 判据清单（`crates/yeban-model/tests/schema_ratchet.rs`，9 条 + 1 条 `#[ignore]`）

| # | 判据（函数名） | 方向 | 测的是什么 | 最终形态的覆盖量 |
| :-- | :--- | :--- | :--- | :--- |
| ① | `fixture_round_trips_and_is_rich_enough_for_the_ratchet` | — | 夹具 JSON 往返相等 + `validate() == Ok` + **够富**（tracks/clip_pool/sections/scenes/assets/edges/devices/macros/lanes/clips/notes 全非空） | 12 项非空断言 |
| ② | `direction_a_every_contract_required_path_is_emitted_by_the_writer` | **A** | 契约每一条 `required` 路径（递归）都必须出现在 `serde_json::to_value(filled_project())` 里 | **134 条路径** |
| ③ | `direction_b_every_contract_required_path_is_required_by_the_reader` | **B** | 逐个 `required` 路径**删键**（键不存在，不是 `null`）⇒ `from_value::<YebanProjectV1>` 必须失败 | **109 条唯一路径**有实例探针 |
| ④ | `direction_b_d43_exempt_paths_tolerate_missing_keys` | B 反面 | §5.4 的 18 项豁免逐个"删键 ⇒ **必须读成功**" | 13/18 落盘并实测（删 18 个键），5/18 夹具里本来不落盘 |
| ⑤ | `contract_required_is_disjoint_from_the_d43_exemption_set` | §5.4 | 18 项豁免**绝不**能被契约任何 `required` 收下（否则写入器输出被判非法） | 18/18 已声明、**0/18 被 required** |
| ⑥ | `reader_required_paths_are_frozen_debt_in_the_contract` | **B′** | "实现要求但契约没要求"（schema 更松 ⇒ 坏文件能过门禁）——`CONTRACT_TIGHTENING_PENDING` 已清空 ⇒ **硬等式，0 条** | 18 条候选，真正判到 13 条，**债务 0** |
| ⑦ | `contract_self_check_refs_resolve_and_required_names_are_declared` | 契约自身 | `$ref` 可解析；每个 `required` 名都在**同一对象**的 `properties` 里；无本遍历器不支持的 JSON Schema 形态 | 134 条 required，**0 条未声明**、0 个坏 `$ref`、0 个不支持形态 |
| ⑧ | `ref_resolver_and_required_name_check_have_teeth` | 契约自身 | 用**合成 schema** 证明 ⑦ 不是永真判据（坏 `$ref`、拼错的 required、`$ref` 目标里的 required 都要被抓） | 5 断言 |
| ⑨ | `no_vacuous_required_paths_in_the_ratchet` | 反空洞 | 每一条「必达」的 `required` 都被夹具真实触达；每个「必达」`oneOf` 组至少一条分支被触达 | 91 条必达全部有探针；5 组全被触达；未选中分支 11 条（备选，合法） |
| ⑩ | `print_contract_ratchet_table_for_the_notes`（`#[ignore]`） | — | 生成 §6 引用的**真实错误文本**表；不进 CI | — |

---

## 2. 两个方向**各自怎么判**（关键代码结构）

### 2.1 方向 A：契约不能比实现更紧

> 病灶 A1（历史）：契约 `required` 了 `tracks` / `clip_pool` / `routing_graph`，
> 实现却 `#[serde(default)]` 静默兜底 ⇒ 一份被截断的文件"读成功"，整首曲子音轨被清空。
> 病灶 A2（反向过紧）：契约一旦收紧过头（例如把 §5.4 的 `folder_id` / `notes[*].probability`
> 收进 `required`）⇒ **本写入器自己的输出就会被 schema 判为非法**。

判法 = **契约 × 序列化结果**的双路递归（`walk_instance`，`crates/yeban-model/tests/schema_ratchet.rs:491`）：

```text
walk_instance(schema, instance, path, pointer):
    1. `$ref` → 解析后继续（同一个实例）
    2. `oneOf`/`anyOf`（外部标签枚举：`content` 的 `Midi`/`Audio`、`target` 的 5 个变体）：
         分支"适用" = type 相容 且 分支自己的 required 全在实例里      (branch_applies)
         · 逐适用分支继续递归（**有标签的分支：标签名只走一次**，见下）
         · **一个适用分支都没有** ⇒ 记 unmatched_one_of（红）
    3. `required` × 实例对象：
         · 键在   ⇒ 生成删键探针（供方向 B）
         · 键不在 ⇒ 记 missing（**方向 A 违约**）
         · 实例有 required 却不是对象 ⇒ 记 type_mismatch（无法判定 ⇒ 红，不许静默）
    4. `properties`         → 按名字下去（可能没值：那就是"可选子对象"，继续传 None）
       `additionalProperties` → **每一个** map 条目（`tracks[*]` / `clip_pool[*]` / `notes[*]` …）
       `items`               → **每一个**数组元素（`edges[*]` / `devices[*]` / `params[*]` / `lanes[*]` …）
```

于是 `tracks[*].{solo_safe,devices,macros,automation_lanes,clips}` 这类"**每一个** track 对象
都必须有"的 `required` 会被**逐对象**检查（不是只看根），而
`clip_pool[*].content` 的两支按「**至少一支**满足」处理 —— 一条 `Midi` 条目不会因为缺
`Audio` 的 `asset` 被判违约（这正是整合者提醒的第 2 点，方向 A 的语义与他的口径一致）。

**修过的一个真缺陷**：有标签分支最初把标签名走了两遍（`content.Midi.Midi.notes`）。
判据 ③ 用「实例指针」删键，所以它**照样跑**；但判据 ⑥ 要把 schema 路径落到实例上，
双写路径解析不到实例 ⇒ **⑥ 会静默空转**。修法：标签名只走一次
（`walk_schema` / `walk_instance` 各一处，并加了 `tagged_branch_spec`）。
这类"判据绿但没真判"的洞，正是判据 ⑨ + ⑥ 的 `unjudged`/`judged` 反空洞断言要挡的。

**条件性（重要）**：`slide.oneOf[1].duration_ticks` 这类"可选子对象的内部必需字段"
只在父值真的出现时才可判；父值整份文档都没出现时它不构成违约（这也正是 JSON Schema 的语义）。
这类路径由 ⑨ 如实打印，**不假装判过**。

### 2.2 方向 B：契约不能比实现更松（坏文件不许过门禁）

> 病灶（**已被 `line/model-no-compat` 接手者量化**）：实现要求 `rng_seed`、`tracks[*].solo_safe`，
> 契约却不要求 ⇒ 一份缺这些字段的文件**能通过 schema 门禁**（`validate_schemas.py` exit 0），
> 却在 `from_value` 时炸掉。

判法 = **删字段法**（`delete_key`，`crates/yeban-model/tests/schema_ratchet.rs:254`）：

```rust
// 对每一条 required 路径（已由 walk_instance 落成"具体对象指针 + 具体键"）:
let mut document = fixture();                                    // 完整填充的工程
assert!(delete_key(&mut document, &probe.pointer, &probe.key));   // 删的是"键不存在"
if serde_json::from_value::<YebanProjectV1>(document).is_ok() {
    drift.push(path);                                            // ⇒ 实现其实不要求它 ⇒ 红
}
```

**边界：测的是"键不存在"，不是"键是 `null`"** —— 写在 `delete_key` 的文档注释里：
本写入器对语义可选字段用 `Option::None` + `skip_serializing_if`，落盘形态是**键消失**；
`#[serde(default)]` 的兜底也只在**键不存在**时生效。写 `null` 是另一回事
（`Option<T>` 会把 `null` 读成 `None`，那就无法区分"作者显式写了 `null`"与"键被截断"）。
因此**只删键**。

### 2.3 方向 B′：实现要求但契约连 `required` 都没有（**补齐"更松"这一侧**）

判据 ③ 只能判"契约要求了 ⇒ 实现也必须要求"。它的反面需要另一台机器：
契约里**声明为属性但没进 `required`** 的每个路径，逐个删键 ——
删掉后**读失败**就说明"实现要求它、契约没要求它"（`crates/yeban-model/tests/schema_ratchet.rs:1039`）。

`CONTRACT_TIGHTENING_PENDING` 在契约收紧落地后**已清空** ⇒ 判据 ⑥ 现在是**硬等式**：
任何"实现要求但契约没要求"都当场红，**没有登记豁免的余地**。
（过渡期它曾是 §5.1/§5.2 的 12 项登记表，断言写成 `debt ⊆ PENDING`，那样收紧线落地后
判据不会变成跨线的定时炸弹。历史见 `git log -p` 的 `e5a37bd`。）

---

## 3. 豁免集与 `model-no-compat-notes.md` **§5.4** 的逐项对应

豁免集是显式常量 `D43_EXEMPT_REQUIRED_PATHS`（`crates/yeban-model/tests/schema_ratchet.rs:95`），
带 §5.4 的行号级表格注释。18 项 = 3 + 1 + 1 + 2 + 1 + 2 + 1 + 7，
**全部以 notes §5.4 为准**，本线**没有**改动任何一项：

| # | 本线常量里的路径 | §5.4 的对象 / 字段 | 最终形态实测 |
| :-- | :--- | :--- | :--- |
| 1 | `#.author` | 根 `author` | 已声明、**非** required；夹具落盘、删键可读 ✅ |
| 2 | `#.metadata.description` | `metadata.description` | 已声明、非 required；删键可读 ✅ |
| 3 | `#.metadata.tags` | `metadata.tags` | 已声明、非 required；删键可读 ✅ |
| 4 | `#.tracks.*.devices[].params[].unit` | `devices[*].params[*].unit` | 已声明、非 required；2 个键实测删键可读 ✅ |
| 5 | `#.tracks.*.automation_lanes[].domain` | `automation_lanes[*].domain` | 已声明、非 required；删键可读 ✅ |
| 6 | `#.tracks.*.folder_id` | `tracks[*].folder_id` | 已声明、非 required；夹具里不落盘（判据 ① 整份往返覆盖） |
| 7 | `#.tracks.*.color` | `tracks[*].color` | 已声明、非 required；2 个键实测删键可读 ✅ |
| 8 | `#.sections.*.color` | `sections[*].color` | 已声明、非 required；删键可读 ✅ |
| 9 | `#.scenes.*.tempo` | `scenes[*].tempo` | 已声明、非 required；删键可读 ✅ |
| 10 | `#.scenes.*.color` | `scenes[*].color` | 已声明、非 required；夹具里不落盘 |
| 11 | `#.routing_graph.edges[].gain_db` | `routing_graph.edges[*].gain_db` | 已声明、非 required；删键可读 ✅ |
| 12 | `#.clip_pool.*.content.Midi.notes.*.probability` | `MidiNote::probability` | 已声明、非 required；删键可读 ✅（**注入 A 的主角**） |
| 13 | `…notes.*.ratchet` | `MidiNote::ratchet` | 同上 ✅ |
| 14 | `…notes.*.micro_timing_ticks` | `MidiNote::micro_timing_ticks` | 同上 ✅ |
| 15 | `…notes.*.slide` | `MidiNote::slide` | 已声明、非 required；夹具里不落盘 |
| 16 | `…notes.*.pitch_bend_curve` | `MidiNote::pitch_bend_curve` | 同上 |
| 17 | `…notes.*.syllable` | `MidiNote::syllable` | 已声明、非 required；4 个键实测删键可读 ✅ |
| 18 | `…notes.*.phonemes` | `MidiNote::phonemes` | 已声明、非 required；夹具里不落盘 |

**反例（§5.4 原文点名）**：`clip_pool[*].content.Audio.gain_db` **不在**豁免表里 ——
那里是裸 `f32`，所以它**是**必需的。判据 ③ 删它，实测报
``missing field `gain_db` ``（§6.2）。

**"某个被豁免的字段其实实现是必需的"怎么办**：判据 ④ 会当场红并点名那个路径
（错误文本："若它其实是必需的，说明 §5.4 清单有误 —— 把清单报给集成者，不要改豁免集换绿"）。
**最终形态实测：18/18 全部"删键可读" ⇒ §5.4 清单无误，无一项需要上报。**

**为什么这张手写清单不会变成"第二份事实源"**：它是**可被证伪**的 ——
⑤ 断言它**不得**出现在契约 `required` 里、④ 断言删掉它**必须**读成功。
清单若说谎（把必需字段当豁免、或反过来），这两条立刻红。整合者的教训
（"审计器必须读产物，不能读清单"）在这里是成立的：判据的**事实源是契约文件 + `from_value` 行为**，
清单只承担"§5.4 说了哪 18 项"这一件事，且被双向反证。

---

## 4. 契约自身的完整性（判据 ⑦⑧）

| 检查 | 实现 | 最终形态 |
| :--- | :--- | :--- |
| 每个 `$ref` 都能解析（`#/...` 本地指针，`Value::pointer` 自带 `~0`/`~1` 反转义；外部 URL 显式判为**不可解析**） | `resolve_ref`（`:301`）+ 判据 ⑦ | 契约当前不用 `$ref` ⇒ 0 条；注入坏 `$ref` 立刻红（§5.1 D2） |
| 每个 `required` 名都在**同一对象**的 `properties` 里 | `walk_schema`（`:313`）记 `RequiredFact::declared` | **134/134 全部已声明**；拼错的 required 当场红（§5.1 D1） |
| 不许出现遍历器不支持的 JSON Schema 形态（`allOf` / `patternProperties` / `if` / `not` / `unevaluatedProperties` …） | `SchemaFacts::unsupported` | 0 条。**出现即红**：覆盖不完整必须显式扩展遍历器，不许漏判 |
| 上述三条**不是永真** | 判据 ⑧ 用合成 schema 实测（含 `$ref` 目标内的 required） | 5/5 断言通过 |

---

## 5. 注入记录（注入 → 变红 → 还原，全部**本机当场实测**）

命令一律为（本工作树自己的脚本，账本 **L30**）：

```bash
cd /Users/crow/work/music/yeban/.worktrees/schema-ratchet
bash scripts/dev/cargo-local.sh test -p yeban-model --test schema_ratchet
python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples   # 需要 samples
```

### 5.1 对**最终形态**（合并后的契约）的注入 —— 本轮的正式证据

| # | 注入 | 变红 | 真实输出（逐字，节选） | 外部对照 |
| :-- | :--- | :--- | :--- | :--- |
| **A** | `clip_pool…content.oneOf[0].Midi.notes.additionalProperties.required += "probability"`（**收紧一个 §5.4 豁免项**） | ⑤ + ② | `【§5.4 违约】…: ["#.clip_pool.*.content.Midi.notes.*.probability"]`<br>`【方向 A 违约】…: {"#.clip_pool.*.content.Midi.notes.*.probability"}` | `validate_schemas.py` **exit 1**：`project.filled.json: 违反 project.schema.json @ clip_pool/…/content: {'Midi': {'notes': {…}}} is not valid under any of the given schemas` |
| **B** | 根 `required -= "rng_seed"`（**契约更松**） | ⑥ | `【方向 B′ 违约】出现了**新的**「实现要求但契约没要求」的字段…:`<br>`["#.rng_seed"]` | `validate_schemas.py` **exit 0 / 「契约校验通过 (4 份 schema)」= 没发现**（这正是本线补的洞） |
| **C** | `TrackV3::solo_safe` 上**加回** `#[serde(default)]`（**实现放松一个契约必需的字段**） | ③ | `【方向 B 违约】…`<br>`#.tracks.*.solo_safe（实例 /tracks/01J8ZQ…0001 的键 \`solo_safe\`）删掉后**仍然读成功**`（4 条 track 各一条） | — |
| **D1** | `time_signature.required` 里 `"numerator"` → `"numenator"`（拼写错） | ⑦ + ② + ⑥ | `这些 \`required\` 名在**同一对象**的 \`properties\` 里根本不存在…: ["#.time_signature.numenator"]`<br>`【方向 A 违约】…: {"#.time_signature.numenator"}` | — |
| **D2** | `properties.title += "$ref": "#/$defs/Nope"` | ⑦ | `契约里的 \`$ref\` 无法解析（改契约的人请修，本线只读它）: ["#/$defs/Nope"]` | — |

**A 与 B 是一对（也是本轮最有价值的一对）**：它们机械地复现了
①「schema 更紧」**能被** `validate_schemas.py` 抓到、②「schema 更松」**抓不到** ——
而本线判据 ⑥ 在 ② 上**红**。这两条注入就是"我们能抓更松"的机械证据。

还原复核（每一次注入之后）：

```text
$ shasum -a 256 schemas/project.schema.json
5eb3362ab3fb7efc9ee879a173eff30ded48b560ba01e73524114002f0f47e59  schemas/project.schema.json
$ git diff --stat schemas/ crates/yeban-model/src/     # 空输出
$ python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples
契约校验通过 (4 份 schema)。                              # exit 0
$ bash scripts/dev/cargo-local.sh test -p yeban-model --test schema_ratchet | tail -3
test result: ok. 9 passed; 0 failed; 1 ignored
```

### 5.2 对**过渡期**（旧 schema，`main` `113d8a4` 的 11 键）的注入 —— 历史证据

这些注入发生在本线第一次提交（`e5a37bd`，对着**收紧前**的契约）之时，用来证明
"判据在契约收紧之前就已经在工作"，并证明它**没有**把过渡期的差异误判成"棘轮太严"：

| # | 注入 | 变红 | 输出（节选） |
| :-- | :--- | :--- | :--- |
| 1 | `tracks…required += "folder_id"`（收紧一个 §5.4 豁免项） | ⑤ + ② | `["#.tracks.*.folder_id"]`（两条判据） |
| 2 | `TrackV3::mute` 加回 `#[serde(default)]` | ③ | `#.tracks.*.mute（实例 /tracks/…0001 的键 \`mute\`）删掉后**仍然读成功**` × 4 |
| 3 | `tracks…required -= "solo"`（契约更松） | ⑥ | `["#.tracks.*.solo"]` |
| 4 | `time_signature.required` 拼成 `"numenator"` | ⑦ + ② + ⑥ | `["#.time_signature.numenator"]` |
| 5 | `properties.title += "$ref": "#/$defs/Nope"` | ⑦ | `["#/$defs/Nope"]` |

两次总共 **10 次注入、8 类判据变红、全部还原**（`schemas/**` 每次都以 sha256 复核回到原值）。

---

## 6. 真实错误文本样例

### 6.1 方向 B（最终形态，109 条唯一路径，105 条 `missing field` + 4 条标签枚举）—— 节选

```text
#.metadata                                                             missing field `metadata`
#.transport                                                            missing field `transport`
#.assets.*.license                                                     missing field `license`
#.tracks.*.solo_safe                                                   missing field `solo_safe`
#.tracks.*.devices[].id                                                missing field `id`
#.tracks.*.macros[].mappings[].depth                                   missing field `depth`
#.tracks.*.clips.*.loop_config.enabled                                 missing field `enabled`
#.sections.*.id                                                        missing field `id`
#.scenes.*.name                                                        missing field `name`
#.routing_graph.edges[].kind                                           missing field `kind`
#.clip_pool.*.content.Midi.notes.*.velocity                            missing field `velocity`
#.clip_pool.*.content.Audio.gain_db                                    missing field `gain_db`
#.clip_pool.*.content.Midi                                             invalid value: map, expected map with a single key
#.clip_pool.*.content.Audio                                            invalid value: map, expected map with a single key
#.tracks.*.automation_lanes[].target.TrackVolume                       invalid value: map, expected map with a single key
#.tracks.*.automation_lanes[].target.TrackVolume.track_id              missing field `track_id`
```

最后四条是**外部标签枚举**的标签键被删掉时的形态（`ClipContent` / `AutomationTarget`）：
`invalid value: map, expected map with a single key` —— 同样是"响亮失败"，不是静默兜底。

### 6.2 §5.4 豁免（13/18 落盘，18 个键）—— 节选：**必须读成功**

```text
#.author                                                               Ok(缺键可读)  [ / author]
#.metadata.description                                                 Ok(缺键可读)  [/metadata / description]
#.tracks.*.devices[].params[].unit                                     Ok(缺键可读)  [/tracks/…0002/devices/0/params/0 / unit]
#.tracks.*.automation_lanes[].domain                                   Ok(缺键可读)  [/tracks/…0002/automation_lanes/0 / domain]
#.sections.*.color                                                     Ok(缺键可读)  [/sections/…0070 / color]
#.scenes.*.tempo                                                       Ok(缺键可读)  [/scenes/…0080 / tempo]
#.routing_graph.edges[].gain_db                                        Ok(缺键可读)  [/routing_graph/edges/2 / gain_db]
#.clip_pool.*.content.Midi.notes.*.probability                         Ok(缺键可读)  [/clip_pool/…0010/content/Midi/notes/…0103 / probability]
#.clip_pool.*.content.Midi.notes.*.syllable                            Ok(缺键可读)  [×4 个音符]
#.tracks.*.folder_id / #.scenes.*.color / …notes.*.{slide,pitch_bend_curve,phonemes}
                                                                       (夹具里不落盘; 由整份往返覆盖)
```

### 6.3 方向 B′（最终形态）：**债务 0**

```text
[schema-ratchet] 方向 B′: 契约声明为可选属性 18 条（真正判到 13 条，夹具里不落盘 5 条）;
                 其中「实现要求但契约没要求」 0 条: {}; 已登记过渡上限 0 条
```

⇒ **方向 ③（契约 required ⇒ 实现 required）：0 漂移。**
⇒ **方向 B′（实现 required ⇒ 契约 required）：0 漂移**（收紧后，`#.rng_seed`、`#.tracks.*.solo_safe`
等 12 项全部由契约接住）。
过渡期（收紧前）的实测是 **2 条漂移**：`#.rng_seed`、`#.tracks.*.solo_safe` ——
判据当时就点到了路径级，正是本轮收紧要修的那一类。

---

## 7. 最终形态的实测数字（把契约换成 main 的收紧版之后）

```text
$ git merge origin/main            # 无冲突；schemas/project.schema.json sha256 5eb3362a…
$ bash scripts/dev/cargo-local.sh test -p yeban-model --test schema_ratchet -- --nocapture --test-threads=1
[schema-ratchet] §5.4 豁免集: 18 项, 其中 18 项已被契约声明为属性、0 项被 required
[schema-ratchet] §5.4 豁免: 13/18 条落盘并删键验证成功（共 18 个键）; 缺席 5/18: [...]
[schema-ratchet] 覆盖: 契约 required 路径 134 条（其中必达 91 条）; oneOf 组 5 个; 未被夹具选中的分支 11 条
[schema-ratchet] 方向 B′: 契约声明为可选属性 18 条（真正判到 13 条，夹具里不落盘 5 条）; 其中「实现要求但契约没要求」 0 条: {}; 已登记过渡上限 0 条
test result: ok. 9 passed; 0 failed; 1 ignored
```

未选中的 11 条分支（**合法**：分支是备选，不是违例，⑨ 只打印不判红）：
`#.clip_pool.*.content.Midi.notes.*.slide.oneOf[0/1]`、`#.tracks.*.automation_lanes[].domain.oneOf[0]`、
`#.tracks.*.automation_lanes[].target.{TrackPan,SendGain,DeviceParam,Macro}`、
`#.tracks.*.macros[].mappings[].target.{TrackVolume,TrackPan,SendGain,Macro}`。

另外，契约收紧没有破坏"写入器自己的输出仍合法"：
`cargo-local.sh test -p yeban-model --lib export_schema_samples_to_target` 产出 4 份样本后，
`python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples`
= **`契约校验通过 (4 份 schema)`，exit 0**（与整合者的复跑一致，本线独立复现）。

---

## 8. 本机真跑 vs CI：严格区分

### 8.1 本机真跑（**已绿**，本工作树 `/Users/crow/work/music/yeban/.worktrees/schema-ratchet`）

| 命令 | 结果 |
| :--- | :--- |
| `bash scripts/gates/run-gates.sh crate yeban-model` | ✅ **`门禁通过 (mode=crate)`**（**merge `origin/main` `0ef7eb2` 之后复跑**）；fmt → 14 条守卫全 ok → 规范 ID 审计（39 个 ID 全在册）→ 文档检查 → 许可清单（679 行，无漂移）→ `clippy --all-targets -D warnings` 零告警 → test |
| `cargo test -p yeban-model`（经 `cargo-local.sh`，**合并后复跑**） | ✅ `107 + 28 + 50 + 16 + 8 + 9 = 218 passed / 0 failed`（+2 `#[ignore]`：`no_compat.rs` 的错误文本生成器 + 本文件 ⑩） |
| `bash scripts/gates/run-gates.sh light` | ✅ `门禁通过 (mode=light)`（65 个 markdown、198 个相对链接、许可清单一致） |
| `python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples` | ✅ exit 0（4 份 schema + 4 份样本） |
| 10 次注入（§5） | ✅ 全部按预期变红，且**全部还原**（`schemas/**` sha256 与 `git status` 双向复核） |

> `cargo-local.sh` 打印 `工作区=/Users/crow/work/music/yeban/.worktrees/schema-ratchet` ——
> 这是"跑的是本工作树、不是主仓"的机械证据（账本 **L30** 的实测事故就是主仓绝对路径）。

### 8.2 本机**未能**跑（**不是绿**，按 pending 处理）

| 项 | 原因 | 谁来做 |
| :--- | :--- | :--- |
| `run-gates.sh full`（`--workspace` / `cargo deny` / Windows 分支） | 本机纪律禁止全量构建 | **CI** |
| 下游 6 个 crate（`yeban-app` / `yeban-render` / `yeban-engine` / `yeban-mcp` / `yeban-decode` / `yeban-ui-mcp`）的编译与测试 | 全部含重依赖 | **CI** |

---

## 9. 局限（诚实声明，全部也写在测试文件头部）

1. **只覆盖 `YebanProjectV1`**（`schemas/project.schema.json`）—— 不覆盖容器、`ops.schema.json`、
   `mcp-tools.schema.json` 等其它契约。同类棘轮值得在 `ops.schema.json` 上再做一个
   （`samples.rs::op_variants_match_ops_schema_exactly` 已覆盖枚举变体名，但没覆盖 `required`）。
2. **只判 `required` 的存在性/必需性**，不做完整 JSON Schema 校验（类型/枚举/区间由
   `scripts/gates/validate_schemas.py` 与 CI 的 jsonschema 承担）。两者是互补的：
   本判据管"必需性对账"，jsonschema 管"取值合法性"。
3. **条件性 required 只能条件性判**：`slide.oneOf[1].duration_ticks` 这类"可选子对象的内部必需字段"，
   在整份夹具里一个 `slide` 都没有时无法判定 —— ⑨ **如实打印**（不假装判过）。
4. **方向 B′ 只能判契约已经声明的属性**：契约里连 `properties` 都没有的字段
   （收紧前正是这种形态：`metadata` / `devices` / `macros` / `clips` / `sections` / `scenes` / `assets` …）
   在 B′ 里**看不见** —— 那是"契约缺失"，只能靠契约线补属性/补 required（补完立刻由 ③ 接管）。
   收紧之后这一类已经全部被 ③ 覆盖（134 条路径）。
5. **`oneOf` 的"至少一支满足"是方向 A 的语义**，不是完整校验：`{"type":"null"}` 这种无 `required`
   的分支由 `branch_applies` 的 **type 相容**排除，而不是靠 required 排除。
6. **`$defs` 里的模板只在被 `$ref` 引用到时才检查**；外部 `$ref` 显式判为"不可解析"（红），不假装能解析。
7. **`tracks[*]` 里"空集合"不等于"没覆盖"**：某些 track 的 `devices` 是空数组，只要全局
   ≥1 个设备，`devices[*].params[*].unit` 之类的路径就有探针；判据 ⑨ 要求的是**每条必达路径全局 ≥1 探针**，
   不是"每个实例都有"。若某个**必需的集合**整体为空（例如 `tracks = {}`），⑨ 会红。

---

## 10. needs / pending

### needs（需要别人动手）

1. ✅ **已解决（整合者已做）**：`schemas/project.schema.json` 的收紧已合并进 `main`
   （`29b988d` merge，CI run **37248121161** = success；本线已 `git merge origin/main` 到 `0ef7eb2`）。
2. **契约线的所有人**：**不要**把 §5.4 的 18 项任何一项写进任何 `required`（判据 ⑤ 会红，判据 ② 也会红）。
   实测：把 `notes[*].probability` 收进 `required` ⇒ 本线红 **且** `validate_schemas.py` exit 1。
3. **契约线的所有人**：也不要**把 `required` 里的字段删掉**（判据 ⑥ 会红）——
   实测：删掉根 `required` 里的 `rng_seed` ⇒ `validate_schemas.py` **exit 0（没发现）**、
   判据 ⑥ **红**（`["#.rng_seed"]`）。
4. **后续工作线**：给 `schemas/ops.schema.json` 做同款的 `required` 棘轮（本判据的 walker 可直接复用：
   `walk_schema` / `walk_instance` / `deletion_targets` / `resolve_ref` 都与具体文档类型无关）。
5. **审计器纪律（整合者转述的教训）**：审计器必须读**产物**（契约文件 + 反序列化行为），
   不能读**清单**。本判据遵守这一条：`required` 集合 100% 从契约文件现读；
   唯一的两个手写常量是 §5.4 豁免集（由 ④⑤ 双向反证，且以 notes §5.4 为准）
   与**已清空**的过渡登记表。

### pending

| 项 | 状态 |
| :--- | :--- |
| 本分支最终 tip 的 CI 判决 | 见 §12（`scripts/dev/ci-verdict.sh line/schema-ratchet`） |
| `line/model-schema-d43` 进 `main` | ✅ 已完成（`29b988d`，run 37248121161 success） |

---

## 11. 修改文件与净行数

| 文件 | 变更 | 行数（相对 `origin/main`） |
| :--- | :--- | :--- |
| `crates/yeban-model/tests/schema_ratchet.rs` | **新增**（9 条判据 + 1 条 `#[ignore]` + 两套 walker + 路径工具 + §5.4 豁免常量 + 已清空的过渡登记常量） | **+1374** |
| `docs/ledger/schema-ratchet-notes.md` | **新增**（本文件） | **+358**（回填判决后略有增加） |
| `schemas/**` / 根 `Cargo.toml` / `Cargo.lock` / `.github/**` / `scripts/**` / `deny.toml` / `docs/DEVELOPMENT_LEDGER.md` / `docs/adr/**` / 其它 `crates/**` | **零改动**（`git status` + `git diff --stat` 复核） | 0 |

零新增依赖（只用已有的 `serde_json` / `serde` + 标准库）。

---

## 12. 提交与 CI 判决

- 提交 1（代码 + 初版 notes）：`e5a37bd`（对着**收紧前**的契约；`run-gates.sh crate` / `light` 均绿）。
- 合并：`6425252` merge `origin/main` `0ef7eb2`（把契约收紧与 `main` 的其它 6 个提交收进来，无冲突）。
- 提交 2（本文件改成**最终形态**口径 + 退役过渡登记表 + §5.1 的 A/B/C/D 注入实测）。
- **CI 判决**（`bash scripts/dev/ci-verdict.sh line/schema-ratchet`）见下方回填。

```text
（判决回填区：run id / 结论 / 哪些腿跑了）
```
