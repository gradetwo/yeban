# schema-ratchet 工作线 notes —— **契约 `required` ⇔ 实现必需性** 的常设棘轮

> 工作线: `line/schema-ratchet`（基线 `main` `113d8a4`）
> 拥有: `crates/yeban-model/**`；**只读** `schemas/**`（本线一只手指都没碰它）
> 新增: `crates/yeban-model/tests/schema_ratchet.rs`、本文件
> 交付使命（来自 `line/model-schema-d43` 上报的 needs）: 把"契约（`schemas/project.schema.json`）
> 的 `required` 与实现的必需性对账"做成**常设判据**，让这一类漂移**不可能再发生**。

---

## 0. 一句话结论

**契约（契约文件本身）与实现的必需性现在被 9 条机械判据双向钉住**，并且这 9 条
**直接读契约文件**、一个 `required` 名都不手抄 —— 契约改动一次，判据当场重判一次。

在**当前 main 的契约**（`113d8a4`，未收紧）与**`line/model-schema-d43` 的收紧契约**
（`9d60b82`，根 `required` 18 键）两种状态下，这 9 条**都绿**（§7 有实测数字）。
这是本线最关键的一条证据：这台棘轮不会在收紧线合并进 main 的那一刻变成红（不是定时炸弹），
而是**换个契约继续判**，并把判决从 26 条 required 路径扩大到 134 条。

同时它**现在就已经抓到了 2 条真实漂移**（§6.3），正是这轮收紧要修的那一类：

| 漂移路径 | 形态 | 处置 |
| :--- | :--- | :--- |
| `#.rng_seed` | 实现要求（缺键即报错）、契约只声明为属性**没进 required** | §5.1 待办（`line/model-schema-d43` 已修） |
| `#.tracks.*.solo_safe` | 同上 | §5.2 待办（同上） |

---

## 1. 判据清单（`crates/yeban-model/tests/schema_ratchet.rs`，9 条 + 1 条 `#[ignore]`）

| # | 判据（函数名） | 方向 | 测的是什么 | 覆盖量（当前契约 / 收紧后契约） |
| :-- | :--- | :--- | :--- | :--- |
| ① | `fixture_round_trips_and_is_rich_enough_for_the_ratchet` | — | 夹具 JSON 往返相等 + `validate() == Ok` + **够富**（tracks/clip_pool/sections/scenes/assets/edges/devices/macros/lanes/clips/notes 全非空） | 12 项非空断言 |
| ② | `direction_a_every_contract_required_path_is_emitted_by_the_writer` | **A** | 契约每一条 `required` 路径（递归）都必须出现在 `serde_json::to_value(filled_project())` 里 | 26 条路径 / 134 条路径 |
| ③ | `direction_b_every_contract_required_path_is_required_by_the_reader` | **B** | 逐个 `required` 路径**删键**（键不存在，不是 `null`）⇒ `from_value::<YebanProjectV1>` 必须失败 | 26 个唯一路径 / 109 个唯一路径 |
| ④ | `direction_b_d43_exempt_paths_tolerate_missing_keys` | B 反面 | §5.4 的 18 项豁免逐个"删键 ⇒ **必须读成功**" | 13/18 落盘并实测（删 18 个键），5/18 夹具里本来不落盘 |
| ⑤ | `contract_required_is_disjoint_from_the_d43_exemption_set` | §5.4 | 18 项豁免**绝不**能被契约任何 `required` 收下（否则写入器输出被判非法） | 18 项 / 18 项声明、0 项 required |
| ⑥ | `reader_required_paths_are_frozen_debt_in_the_contract` | **B′** | "实现要求但契约没要求"（schema 更松 ⇒ 坏文件能过门禁）只能落在**已登记的收紧待办**里 | 判到 3/4 候选，债务 = 2 条已登记 / 判到 13/18 候选，债务 = **0** |
| ⑦ | `contract_self_check_refs_resolve_and_required_names_are_declared` | 契约自身 | `$ref` 可解析；每个 `required` 名都在**同一对象**的 `properties` 里；无本遍历器不支持的 JSON Schema 形态 | 26 条 / 134 条 required，0 条未声明 |
| ⑧ | `ref_resolver_and_required_name_check_have_teeth` | 契约自身 | 用**合成 schema** 证明 ⑦ 不是永真判据（坏 `$ref`、拼错的 required、`$ref` 目标里的 required 都要被抓） | 5 断言 |
| ⑨ | `no_vacuous_required_paths_in_the_ratchet` | 反空洞 | 每一条「必达」的 `required` 都被夹具真实触达；每个「必达」`oneOf` 组至少一条分支被触达 | 26/26 必达 / 91 必达 + 5 组，未选中分支 11 条（备选，合法） |
| ⑩ | `print_contract_ratchet_table_for_the_notes`（`#[ignore]`） | — | 生成 §6 引用的**真实错误文本**表；不进 CI | — |

---

## 2. 两个方向**各自怎么判**（关键代码结构）

### 2.1 方向 A：契约不能比实现更紧

> 病灶：契约 `required` 了 `tracks` / `clip_pool` / `routing_graph`，实现却 `#[serde(default)]`
> 静默兜底；反过来，契约一旦收紧过头（例如把 §5.4 的 `folder_id` 收进 `required`），
> **本写入器自己的输出就会被 schema 判为非法**。

判法 = **契约 × 序列化结果**的双路递归（`walk_instance`，`tests/schema_ratchet.rs:491`）：

```text
walk_instance(schema, instance, path, pointer):
    1. `$ref` → 解析后继续（同一个实例）
    2. `oneOf`/`anyOf`：
         分支"适用" = type 相容 且 分支自己的 required 全在实例里     (branch_applies)
         · 对每个适用分支继续递归（有标签的分支：标签名只走一次）
         · **一个适用分支都没有** ⇒ 记 unmatched_one_of（红）
    3. `required` × 实例对象：
         · 键在   ⇒ 生成删键探针（供方向 B）
         · 键不在 ⇒ 记 missing（**方向 A 违约**）
         · 实例有 required 却不是对象 ⇒ 记 type_mismatch（无法判定 ⇒ 红，不许静默）
    4. `properties`   → 按名字下去（可能没值：那就是"可选子对象"，继续传 None）
       `additionalProperties` → 每一个 map 条目（`tracks[*]` / `clip_pool[*]` / `notes[*]` …）
       `items`         → 每一个数组元素（`edges[*]` / `devices[*]` / `params[*]` / `lanes[*]` …）
```

于是 `tracks[*].solo_safe` / `devices` / `macros` 这类"每一个 track 对象都必须有"的
`required` 会被**逐对象**检查（不是只看根），`clip_pool[*].content` 的两支按
"**至少一支**满足"处理 —— 一条 `Midi` 条目不会因为缺 `Audio` 的 `asset` 被判违约。

**条件性（重要）**：`slide.oneOf[1].duration_ticks` 这类"可选子对象的内部必需字段"
只在父值真的出现时才可判；父值整份文档都没出现时它不构成违约（这也正是 JSON Schema 的语义）。
这类路径由 ⑨ 如实打印，**不假装判过**。

### 2.2 方向 B：契约不能比实现更松（坏文件不许过门禁）

> 病灶：实现要求 `rng_seed`、`tracks[*].solo_safe`，契约却不要求 ——
> 一份缺这些字段的文件**能通过 schema 门禁**，却在 `from_value` 时炸掉。

判法 = **删字段法**（`delete_key`，`tests/schema_ratchet.rs:254`）：

```rust
// 对每一条 required 路径（已由 walk_instance 落成"某个具体对象 + 某个键"）:
let mut document = fixture();                                  // 完整填充的工程
assert!(delete_key(&mut document, &probe.pointer, &probe.key)); // 删的是"键不存在"
if serde_json::from_value::<YebanProjectV1>(document).is_ok() {
    drift.push(path);                                          // ⇒ 实现其实不要求它 ⇒ 红
}
```

**边界：测的是"键不存在"，不是"键是 `null`"** —— 写在 `delete_key` 的文档注释里：
本写入器对语义可选字段用 `Option::None` + `skip_serializing_if`，落盘形态是**键消失**；
`#[serde(default)]` 的兜底也只在**键不存在**时生效。写 `null` 是另一回事
（`Option<T>` 会把 `null` 读成 `None`，那就无法区分"作者显式写了 `null`"与"键被截断"）。
因此**只删键**。

### 2.3 方向 B′：实现要求但契约连 `required` 都没有

方向 ③ 只能判"契约要求了 ⇒ 实现也必须要求"。它的**反面**（契约没要求 ⇒ 坏的更松）
需要另一台机器：契约里**声明为属性但没进 `required`** 的每个路径，逐个删键 ——
删掉后**读失败**就说明"实现要求它、契约没要求它"（`tests/schema_ratchet.rs:1039`）。

这些路径必须全部落在**显式常量** `CONTRACT_TIGHTENING_PENDING`（notes §5.1/§5.2 的 12 项）里。
断言刻意写成 **`debt ⊆ PENDING`（单向包含）而不是相等**：

- 收紧线落地后 `debt` 变空集 ⇒ **依然绿**（不会变成一个跨线的定时炸弹）；
- 任何**新**漂移都不在表里 ⇒ **当场红**并把路径点出来。

---

## 3. 豁免集与 `model-no-compat-notes.md` **§5.4** 的逐项对应

豁免集是一个**显式常量** `D43_EXEMPT_REQUIRED_PATHS`（`tests/schema_ratchet.rs:95`），
带 §5.4 的行号级表格注释。18 项 = 3 + 1 + 1 + 2 + 1 + 2 + 1 + 7：

| # | 本线常量里的路径 | §5.4 的对象 / 字段 | 现状（当前契约 / 收紧后） |
| :-- | :--- | :--- | :--- |
| 1 | `#.author` | 根 `author` | 声明、非 required；夹具落盘、删键可读 ✅ |
| 2 | `#.metadata.description` | `metadata.description` | 收紧前契约无此属性 / 收紧后声明、非 required ✅ |
| 3 | `#.metadata.tags` | `metadata.tags` | 同上 ✅ |
| 4 | `#.tracks.*.devices[].params[].unit` | `devices[*].params[*].unit` | 收紧前无 `devices` / 收紧后声明、非 required ✅ |
| 5 | `#.tracks.*.automation_lanes[].domain` | `automation_lanes[*].domain` | 同上 ✅ |
| 6 | `#.tracks.*.folder_id` | `tracks[*].folder_id` | 声明、非 required；夹具里不落盘（判据 ① 整份往返覆盖） |
| 7 | `#.tracks.*.color` | `tracks[*].color` | 收紧前无此属性 / 收紧后声明、非 required ✅ |
| 8 | `#.sections.*.color` | `sections[*].color` | 同上 ✅ |
| 9 | `#.scenes.*.tempo` | `scenes[*].tempo` | 同上 ✅ |
| 10 | `#.scenes.*.color` | `scenes[*].color` | 夹具里不落盘（同上） |
| 11 | `#.routing_graph.edges[].gain_db` | `routing_graph.edges[*].gain_db` | 声明、非 required；夹具落盘、删键可读 ✅ |
| 12–18 | `#.clip_pool.*.content.Midi.notes.*.{probability, ratchet, micro_timing_ticks, slide, pitch_bend_curve, syllable, phonemes}` | 同左（7 个表现力字段） | 收紧前无 `clip_pool.content` / 收紧后声明、非 required；4 项落盘删键可读，3 项不落盘 |

**反例（§5.4 原文点名）**：`clip_pool[*].content.Audio.gain_db` **不在**豁免表里 ——
那里是裸 `f32`，所以它**是**必需的；判据 ③ 会删它并期望读失败（收紧后契约已实测）。

**若 §5.4 里某一项其实是必需的**：判据 ④ 会当场红并点名那个路径
（"若它其实是必需的，说明 §5.4 清单有误 —— 把清单报给集成者，不要改豁免集换绿"）。
**本轮实测：18 项全部"删键可读"，清单无误。**

---

## 4. 契约自身的完整性（判据 ⑦⑧）

| 检查 | 实现 | 现状 |
| :--- | :--- | :--- |
| 每个 `$ref` 都能解析（`#/...` 本地指针，`Value::pointer` 自带 `~0`/`~1` 反转义；外部 URL 显式判为**不可解析**） | `resolve_ref`（`:301`）+ 判据 ⑦ | 契约当前不用 `$ref` ⇒ 0 条；注入坏 `$ref` 立刻红（§5 注入 5） |
| 每个 `required` 名都在**同一对象**的 `properties` 里 | `walk_schema`（`:313`）记 `RequiredFact::declared` | 26/26、134/134 全部已声明；拼错的 required 当场红（注入 4） |
| 不许出现遍历器不支持的 JSON Schema 形态（`allOf` / `patternProperties` / `if` / `not` / `unevaluatedProperties` …） | `SchemaFacts::unsupported` | 0 条。**出现即红**：覆盖不完整必须显式扩展遍历器，不许漏判 |
| 上述三条**不是永真** | 判据 ⑧ 用合成 schema 实测（含 `$ref` 目标内的 required） | 5/5 断言通过 |

---

## 5. 注入记录（注入 → 变红 → 还原，全部**本机实测**）

命令一律为（本工作树自己的脚本，账本 **L30**）：

```bash
cd /Users/crow/work/music/yeban/.worktrees/schema-ratchet
bash scripts/dev/cargo-local.sh test -p yeban-model --test schema_ratchet
```

| # | 注入 | 位置 | 变红的判据 | 真实输出（逐字） | 还原 |
| :-- | :--- | :--- | :--- | :--- | :--- |
| 1 | `tracks.additionalProperties.required += "folder_id"`（**收紧一个 §5.4 豁免项**） | `schemas/project.schema.json`（临时） | ⑤ + ② | `【§5.4 违约】这些 D43 豁免项被契约 \`required\` 了…: ["#.tracks.*.folder_id"]`<br>`【方向 A 违约】契约 \`required\` 了这些路径，但 filled_project 的序列化结果里**没有**这些键…: {"#.tracks.*.folder_id"}` | ✅ 已还原（sha256 `4f12cc0b…` 一致，`git status` 干净） |
| 2 | `TrackV3::mute` 上**加回** `#[serde(default)]`（**放松一个契约必需的字段**） | `crates/yeban-model/src/project.rs` | ③ | `【方向 B 违约】契约 \`required\` 了这些路径，但实现**不**要求它们…`<br>`#.tracks.*.mute（实例 /tracks/01J8ZQ00000000000000000001 的键 \`mute\`）删掉后**仍然读成功**`（4 条 track 各一条） | ✅ `git checkout -- crates/yeban-model/src/project.rs` |
| 3 | `tracks.additionalProperties.required` 里**删掉 `"solo"`**（**契约更松**） | schema（临时） | ⑥（B′） | `【方向 B′ 违约】出现了**新的**「实现要求但契约没要求」的字段…`<br>`["#.tracks.*.solo"]` | ✅ 已还原 |
| 4 | `time_signature.required` 里把 `"numerator"` 拼成 `"numenator"` | schema（临时） | ⑦ + ② + ⑥ | `这些 \`required\` 名在**同一对象**的 \`properties\` 里根本不存在…: ["#.time_signature.numenator"]` | ✅ 已还原 |
| 5 | `properties.title` 上加 `"$ref": "#/$defs/Nope"` | schema（临时） | ⑦ | `契约里的 \`$ref\` 无法解析（改契约的人请修，本线只读它）: ["#/$defs/Nope"]` | ✅ 已还原 |

> 注入 2 是**实现侧**的注入，注入 1/3/4/5 是**契约侧**的注入 —— 两侧都实测过。
> 5 次注入共 8 条判据变红（②×2、③×1、⑤×1、⑥×2、⑦×2），**全部还原后 9/9 绿**。

---

## 6. 真实错误文本样例

### 6.1 方向 B（当前契约，26 条唯一路径）—— 节选

```text
#.schema_version                                                       missing field `schema_version`
#.tracks                                                               missing field `tracks`
#.clip_pool                                                            missing field `clip_pool`
#.routing_graph                                                        missing field `routing_graph`
#.time_signature.numerator                                             missing field `numerator`
#.tracks.*.mute                                                        missing field `mute`
#.tracks.*.solo                                                        missing field `solo`
```

### 6.2 方向 B（**收紧后的契约**，109 条唯一路径）—— 节选（含两种形态）

```text
#.metadata                                                             missing field `metadata`
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
#.tracks.*.automation_lanes[].target.TrackVolume                       invalid value: map, expected map with a single key
```

最后两条是**外部标签枚举**的标签键被删掉时的形态（`ClipContent` / `AutomationTarget`）：
`invalid value: map, expected map with a single key` —— 也是一条"响亮失败"，不是静默兜底。

### 6.3 方向 B′：本轮**抓到**的漂移（当前契约，未收紧）

```text
#.author                                                               Ok(缺键可读 ⇒ 实现不要求)  [ / author]
#.rng_seed                                                             *** Err ⇒ 实现要求但契约没要求: missing field `rng_seed` ***  [ / rng_seed]
#.tracks.*.folder_id                                                   (夹具里不存在, 无从判定)
#.tracks.*.solo_safe                                                   *** Err ⇒ 实现要求但契约没要求: missing field `solo_safe` ***  [/tracks/01J8ZQ00000000000000000001 / solo_safe]
（…另 3 条 track 同样）
```

⇒ **方向 ③（契约 required ⇒ 实现 required）：当前 0 漂移。**
⇒ **方向 B′（实现 required ⇒ 契约 required）：当前 2 条漂移** —— `#.rng_seed`、`#.tracks.*.solo_safe`，
两条都**已经在** `model-no-compat-notes.md` §5.1/§5.2 的收紧清单里，正是
`line/model-schema-d43` 这一刀要修的；收紧之后再跑，**债务清 0**（§7.2）。
这不是"判据永远绿"，而是"判据把待办点名到路径级"。

---

## 7. 双态证据：**这一轮契约收紧合并前 / 后**，9 条判据都绿

### 7.1 收紧前（本分支 tip 上的 `schemas/project.schema.json`，即 `main` `113d8a4`）

```text
$ bash scripts/dev/cargo-local.sh test -p yeban-model --test schema_ratchet -- --nocapture --test-threads=1
[schema-ratchet] §5.4 豁免集: 18 项, 其中 2 项已被契约声明为属性、0 项被 required
[schema-ratchet] §5.4 豁免: 13/18 条落盘并删键验证成功（共 18 个键）; 缺席 5/18: [...]
[schema-ratchet] 覆盖: 契约 required 路径 26 条（其中必达 26 条）; oneOf 组 0 个; 未被夹具选中的分支 0 条
[schema-ratchet] 方向 B′: 契约声明为可选属性 4 条（真正判到 3 条，夹具里不落盘 1 条）; 其中「实现要求」 2 条: {"#.rng_seed", "#.tracks.*.solo_safe"}
test result: ok. 9 passed; 0 failed; 1 ignored
```

### 7.2 收紧后（把 `origin/line/model-schema-d43`（`9d60b82`）的
`schemas/project.schema.json` **临时换入**、跑完立刻换回；sha256 已复核还原）

```text
[schema-ratchet] §5.4 豁免集: 18 项, 其中 18 项已被契约声明为属性、0 项被 required
[schema-ratchet] 覆盖: 契约 required 路径 134 条（其中必达 91 条）; oneOf 组 5 个; 未被夹具选中的分支 11 条
[schema-ratchet] 方向 B′: 契约声明为可选属性 18 条（真正判到 13 条，夹具里不落盘 5 条）; 其中「实现要求」 0 条: {}
test result: ok. 9 passed; 0 failed; 1 ignored
```

未选中的 11 条分支（**合法**：分支是备选，不是违例，⑨ 只打印不判红）：
`…content.Midi.notes.*.slide.oneOf[0/1]`、`…automation_lanes[].domain.oneOf[0]`、
`…automation_lanes[].target.{TrackPan,SendGain,DeviceParam,Macro}`、
`…macros[].mappings[].target.{TrackVolume,TrackPan,SendGain,Macro}`。

**结论**：契约从 26 条 required 扩到 134 条之后，判据**仍然全绿**（写入器确实写全了、
读取器确实要求了），而覆盖量翻了 5 倍。这就是"常设判据"该有的形态。

---

## 8. 本机真跑 vs CI：严格区分

### 8.1 本机真跑（**已绿**，本工作树 `/Users/crow/work/music/yeban/.worktrees/schema-ratchet`）

| 命令 | 结果 |
| :--- | :--- |
| `bash scripts/gates/run-gates.sh crate yeban-model` | ✅ **`门禁通过 (mode=crate)`**；fmt → 14 条守卫全 ok → 文档检查（64 文件）→ 许可清单（679 行，无漂移）→ `clippy --all-targets -D warnings` 零告警 → test |
| `cargo test -p yeban-model`（经 `cargo-local.sh`） | ✅ `107 + 28 + 50 + 16 + 8 + 9 = 218 passed / 0 failed`（+2 `#[ignore]`，其中 1 条是 `no_compat.rs` 的错误文本生成器、1 条是本文件的 ⑩） |
| `bash scripts/dev/cargo-local.sh fmt -p yeban-model -- --check` | ✅ |
| 5 次注入（§5） | ✅ 5/5 按预期变红，且**全部还原**（schema sha256 与 `git status` 双向复核） |

> `cargo-local.sh` 打印 `工作区=/Users/crow/work/music/yeban/.worktrees/schema-ratchet` ——
> 这是"跑的是本工作树、不是主仓"的机械证据（账本 **L30** 的实测事故就是主仓绝对路径）。

### 8.2 本机**未能**跑（**不是绿**，按 pending / needs 处理）

| 项 | 原因 | 谁来做 |
| :--- | :--- | :--- |
| `run-gates.sh full`（含 `validate_schemas.py` / `cargo deny` / `--workspace`） | 本机纪律禁止全量构建 | **CI** |
| 下游 6 个 crate（`yeban-app` / `yeban-render` / `yeban-engine` / `yeban-mcp` / `yeban-decode` / `yeban-ui-mcp`）的编译与测试 | 全部含重依赖，本机不编译 | **CI** |
| 契约收紧合并后与本线合并后的联合复跑 | 取决于 `line/model-schema-d43` 何时进 `main` | **集成者**（§10 needs-1） |

---

## 9. 局限（诚实声明，全部写在测试文件头部）

1. **只覆盖 `YebanProjectV1`**（`schemas/project.schema.json`）—— 不覆盖容器、`ops.schema.json`、
   `mcp-tools.schema.json` 等其它契约。同类棘轮值得在 `ops.schema.json` 上再做一个
   （`samples.rs::op_variants_match_ops_schema_exactly` 已覆盖枚举变体名，但没覆盖 `required`）。
2. **只判 `required` 的存在性/必需性**，不做完整 JSON Schema 校验（类型/枚举/区间由
   `scripts/gates/validate_schemas.py` 与 CI 的 jsonschema 承担）。
3. **条件性 required 只能条件性判**：`slide.oneOf[1].duration_ticks` 这类"可选子对象的内部必需字段"，
   在整份夹具里一个 `slide` 都没有时无法判定 —— ⑨ **如实打印**（不假装判过）。
4. **方向 B′ 只能判契约已经声明的属性**：契约里连 `properties` 都没有的字段
   （收紧前的 `metadata` / `devices` / `macros` / `clips` / `sections` / `scenes` / `assets` …）
   在 B′ 里**看不见** —— 那是"契约缺失"，只能靠契约线补属性/补 required（补完立刻由 ③ 接管）。
5. **`oneOf` 的"至少一支满足"是方向 A 的语义**，不是完整校验：`{"type":"null"}` 这种无 `required`
   的分支由 `branch_applies` 的 **type 相容**排除，而不是靠 required 排除。
6. **`$defs` 里的模板只在被 `$ref` 引用到时才检查**。

---

## 10. needs / pending

### needs（需要别人动手）

1. **集成者**：`schemas/project.schema.json` 的收紧（`line/model-schema-d43`，`9d60b82`）合并进 `main` 之后，
   请 `git fetch origin main && git merge origin/main` 到本线并**重跑**本判据（§7.2 已预演：仍绿）。
   合并同时请**清空** `CONTRACT_TIGHTENING_PENDING`（`tests/schema_ratchet.rs:123`）——
   判据不会因此变红（单向包含），但常量上的注释与 notes §5.1/§5.2 会变成过期描述，
   ⑩ 的 eprintln 会逐项提醒 "已登记的收紧待办里这些项当前不构成漂移"。
2. **契约线的所有人**：**不要**把 §5.4 的 18 项任何一项写进任何 `required`（判据 ⑤ 会红）；
   `folder_id` / `color` / `unit` / `domain` / `gain_db` / 7 个表现力字段都已被实测为"删键可读"。
3. **后续工作线**：给 `schemas/ops.schema.json` 做同款的 `required` 棘轮（本判据的 walker 可直接复用：
   `walk_schema` / `walk_instance` / `deletion_targets` 都与具体文档类型无关）。

### pending（以"是否读到判决"为准）

| 项 | 状态 |
| :--- | :--- |
| 本分支 CI 判决 | 见 §12（`scripts/dev/ci-verdict.sh line/schema-ratchet`） |
| `line/model-schema-d43` 进 `main` | **pending**：截至本线提交时 `origin/main` 仍是 `113d8a4`（未收紧，根 `required` 11 键） |

### ⚠ 与本线任务书的一处**事实更正**

任务书说 "`schemas/project.schema.json` **刚被收紧**：根 `required` 18 键"。
**实测：不是。** 截至本线开工与提交，`origin/main` = `113d8a4`，其 `schemas/project.schema.json`
根 `required` 仍是 **11 键**、`tracks.additionalProperties.required` 仍是 7 键、
`clip_pool` 仍是空壳（`{"type":"object"}`）；18 键的收紧版**只在** `origin/line/model-schema-d43`
（`9d60b82`）上。本判据因此刻意做成"**读契约文件**"，两种契约下都实测过（§7）。

---

## 11. 修改文件与净行数

| 文件 | 变更 | 行数 |
| :--- | :--- | :--- |
| `crates/yeban-model/tests/schema_ratchet.rs` | **新增**（9 条判据 + 1 条 `#[ignore]` + 两套 walker + 路径工具 + 两个常量） | +1389 |
| `docs/ledger/schema-ratchet-notes.md` | **新增**（本文件） | 见 `git diff --stat` 提交后复核 |
| `schemas/**` / 根 `Cargo.toml` / `Cargo.lock` / `.github/**` / `scripts/**` / `deny.toml` / `docs/DEVELOPMENT_LEDGER.md` / `docs/adr/**` / 其它 `crates/**` | **零改动**（`git status` 复核） | 0 |

零新增依赖（只用已有的 `serde_json` / `serde`（dev 目标可用）+ 标准库）。

---

## 12. 提交与 CI 判决

- 提交 1（代码）：见 `git log`；本机 `run-gates.sh crate yeban-model` 绿、`run-gates.sh light` 绿。
- 提交 2（本文件 + CI 判决回填）：见 `git log`。
- **CI 判决**：`bash scripts/dev/ci-verdict.sh line/schema-ratchet` —— 见下方回填。

```text
（判决回填区：run id / 结论 / 哪些腿跑了）
```
