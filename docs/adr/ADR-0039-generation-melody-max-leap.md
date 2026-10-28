# ADR-0039 — 生成旋律的跳进上界：把 `MelodyConstraints.max_leap` 开放为 `melodyMaxLeap`（缺省逐字节 = 12）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent，参考 Logic Pro / 通用旋律生成器的 "range & leap" 与 stepwise-leap
  约束口径自行裁决）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ADR-0016`（缺省逐字节兼容契约）、
  `ADR-0017` D1（旋律声部的引入）、`ADR-0038`（**同一根轴的音域半片**，其非范围 `N2`
  逐字写着"跳进上界 `melodyMaxLeap`……本票不开"⇒ 本票就是那条 `N2` 的兑现票）、
  `ADR-0037`（力度阶梯：同一族"给了必须被消费"的耦合口径）、`ARCH-DET-001`
- **实现落点**: `crates/yeban-mcp/src/domain/section_build.rs`（`PlanOptions`、`melody_leap`、
  `MelodyRulers`、`melody_contents`、`plan_with_options` 的消费校验）、
  `crates/yeban-mcp/src/domain/mod.rs`（`arg_u64` 读取）、`crates/yeban-mcp/src/tools.rs`、
  `schemas/mcp-tools.schema.json`；`crates/yeban-theory`（**只读其既有公开面，本票不改它**
  —— 沿用 `ADR-0028`/`ADR-0038` 的模块边界纪律）
- **日期**: 2026-10-28

## 背景（先测后改，AGENTS.md §6）

`ADR-0038` 把旋律的**音高窗口**接成了入参，同时把**跳进上界**记为非范围 `N2`，理由是
"单位不同（相邻两音的半音距离，不是音域）"。那条理由⛔ 说明这个键**是不是一个能动的杠杆**
—— 如果它接进去只改变一个音都算白费。本轮先量（本机一次性 `rustc --test` 探针，直链
`yeban-theory` 的 rlib ⇒ ⛔ 重依赖、⛔ 不提交；输入＝`GenreLibrary::try_get("synthwave")`、
主音 `C`、`bars=4`、`onsets_per_bar=8`、`seed=7`、窗口固定 `48..=84`，只换
`MelodyConstraints.max_leap`；单位＝**onset 个数 / MIDI 音高值 / 相邻两音的半音距离**）：

| `max_leap` | onset 数 | 实测最大跳进 | 出现的不同音高（个） |
|---|---|---|---|
| 12（出厂） | 32 | 12 | 13 ＝ `{51,55,56,58,60,62,63,65,67,70,72,74,77}` |
| 5 | 32 | 5 | 8 ＝ `{58,60,62,63,65,67,68,70}` |
| 2 | 32 | 2 | 3 ＝ `{67,68,70}` |
| 0 | 32 | 0 | **1** ＝ `{67}`（同音反复） |

三条读数各自的含义：

1. **onset 数恒 32** ⇒ 跳进上界⛔ 不筛点、⛔ 不改时值 —— 与音域那一片（`ADR-0038` D1 的
   "⛔ 不筛点"）同一条形状，判据可以照抄。
2. **实测最大跳进恰等于入参上界**（12/5/2/0 四档全打满）⇒ 这是硬约束而⛔ 软偏好，
   且在出厂值上就被用满（缺省旋律里真的存在 12 半音的跳进）⇒ 收紧它是可听见的变化，
   这一票有实际价值。
3. **`max_leap = 0` 把整条旋律压成单音而 onset 数不变** ⇒ "合法但极端"的边界形状与
   `melodyLower == melodyUpper` 完全同型（`ADR-0038` 已把那条写进判据），本票照此写。

theory 侧的公开面现读：`crates/yeban-theory/src/melody.rs:87`
`pub struct MelodyConstraints { pub lower: u8, pub upper: u8, pub max_leap: u8 }`、
`MELODY_MAX_LEAP = 12`（`:68`）、`melody.rs:33-37` 明文"相邻两音的半音距离恒 ≤ `max_leap`
（默认 12）……窗口里一个音阶音都没有时返回 `NoFeasibleVoicing`，**不**静默放宽上界"。
MCP 侧此前唯一消费点写死 `max_leap: MELODY_MAX_LEAP`（`section_build.rs:1828`）。

## 决定

- **D1 — 新增两个可选键中的这一个：`melodyMaxLeap: integer`，取值 `0..=127`，缺省 `12`**
  （= theory 的 `MELODY_MAX_LEAP`）⇒ 两态语义：键缺席与键给 `12` 必须**逐字节**产出同一份
  `SectionPlan`（`ADR-0016` 的缺省兼容契约，判据 1 钉住）。单位 = **相邻两音的半音距离**，
  ⛔ MIDI 音高值、⛔ 八度号 —— 文档串必须自己把这三者分清，因为它与 `melodyLower`/
  `melodyUpper` 只差一个词。
- **D2 — 上界取 `127` 而⛔ 取"看起来更合理"的 12 或 24**：音高全域是 `0..=127`，
  两个音的半音距离在任何输入下都⛔ 可能超过 127 ⇒ `0..=127` 是**恰好的**界（既没有
  永远打不到的合法值，也不需要为本层另立一张表）。大于窗口宽度的取值 = "实际不设限"，
  这是**语义**而⛔ 错误，与 theory 的 `max_leap`⛔ 参与 `MelodyConstraints::validate`
  同一条事实。`0` 合法（同音反复，读数 3）。
- **D3 — 与音域两键共用同一条消费校验**：`melodyMaxLeap` 的唯一读取点也是旋律生成路径
  （`melody_contents`），因此要求与 `materialSource:"theory"` **且** `includeMelody:true` 同给，
  缺一即拒（`INVALID_PARAMETER_RANGE` + `hint` 点名要补的键）。⚠ 本票把这条校验的**条件表达式**
  从两处重复的 `lower.is_some() || upper.is_some()` 折成一个 `register_given`（三键）——
  措辞随之点名三个键，但**码、载荷形状与 `hint` 里的关键词不变** ⇒ `ADR-0038` 的判据 4
  照旧通过（实测：它只断言码、⛔ `reason` 键、以及 `hint` 含 `materialSource`/`includeMelody`）。
- **D4 — 界只判一次**：`0..=127` 在 MCP 侧的 `melody_leap` 判（复用 `melody_register` 里
  那条 `u8::try_from` + `filter` 的同一个校验器，提到模块级 ⇒ 两个键⛔ 两个写者）；
  ⛔ 在本层预判"这个跳进上界在当前窗口里可不可行" —— `max_leap = 0` 且窗口里只有一个
  音阶音时 theory 自己会给答案，越界窗口时给 `NoFeasibleVoicing`，原样转发
  （`theoryStep:"melody_over_chords"`）。沿用 `ADR-0036` D3 / `ADR-0038` D3 的同一条理由。
- **D5 — ⛔ 新增故障词**：拒绝载荷⛔ 带 `reason` 键，形状与音域两键逐字同族 ⇒
  `fault_vocabulary` 的词表与规范面摘要不变（`ADR-0038` D4 的口径直接继承）。
- **D6 — `MelodyRulers` 加第三个字段 `leap: u8`**（⛔ 又加一个函数参数）：该结构体的存在理由
  就是 `clippy::too_many_arguments` 在本函数是承重信号（`ADR-0038` 的第一批就是红在这里），
  捆成结构体后新键的接入成本 = 一个字段，且 `leap` 与两个 `(u8, u8)` **类型不同** ⇒
  力度档 / 音域 / 跳进三者调错顺序会编译失败而⛔ 静默通过。

## 非范围（登记，⛔ 补齐）

- `N1` **逐声部跳进表**（和声各声部走 `voice_leading` 的自己的 `max_leap`，本票只碰旋律线）。
- `N2` **`melodyMaxLeap` 与音域的联合可行性预校验** —— 见 D4，那是 theory 的判定。
- `N3` `genre_melody*` 门面的跳进参数（沿用 `ADR-0038` N4，本票⛔ 打开那四条门面）。
- `N4` **旋律动机**（`ADR-0038` N1 原样挂着：theory `melody.rs:16` 明文"旋律型 (motif)、
  句法、终止式一律不做"⇒ 产品范围决定，已登记待人类裁决）。

## 判据（`crates/yeban-mcp/tests/section_melody_leap.rs`，6 条）

1. 键缺席 ≡ 给 `12` ⇒ `SectionPlan` 逐字节相同；且缺省旋律的实测最大跳进 = 12（读数 2 的
   "缺省就打满"这一半在这里变成判据）。
2. 收紧到 `2`：只动旋律音高 —— 和声与鼓**逐字节**相同、旋律 id/起点/时值逐条对齐、
   onset 数不变、相邻跳进 ≤ 2、力度仍取阶梯两档。
3. 越界（`128`）⇒ `INVALID_PARAMETER_RANGE`，载荷点名 `field:"melodyMaxLeap"` + `value` +
   `range:[0,127]`，⛔ `reason` 键。
4. 耦合：⛔ 脱离 `materialSource:"theory"`、⛔ 脱离 `includeMelody:true`（含 `hint` 点名要补的键）。
5. `0` 合法 ⇒ 整条旋律压成单音、onset 数不变（读数 3）。
6. 两个声明面（`tools.rs` 的 `param` 与 schema 的 property）同源：类型 `integer`、
   `minimum 0`、`maximum 127`、`default 12`。⚠ **本条同时改掉 `ADR-0038` 判据 6 里
   "两个声明面都⛔ 出现 `melodyMaxLeap`" 的反向钉** —— 那条钉的是"本票不开"这个事实，
   本票开了它就必须翻转，⛔ 留着当假绿。
