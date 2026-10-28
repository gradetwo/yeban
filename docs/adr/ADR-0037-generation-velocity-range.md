# ADR-0037 — 生成内容的力度：把既有的两级映射开放为 `velocityMin` / `velocityMax`（缺省逐字节 = 96 / 112）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent，参考 Logic Pro 的 "Velocity Range" 口径自行裁决）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ADR-0016`（缺省逐字节兼容契约）、
  `ADR-0017` **D1**（鼓组重音与旋律和弦音 112、其余 96 —— 本票参数化的**就是**这条登记）、
  `ADR-0028`（`drumDensity`：按 `hit.accent` **筛**点，⛔ 不改力度）、
  `ADR-0033`（`voiceSubdivision`，其 D 段逐字写着"力度全部沿用 `THEORY_VELOCITY`（96）。
  ⛔ 本票不引入重音模式"—— 力度被**显式推迟**，本票是那张欠条的兑现）、
  `ADR-0035`／`ADR-0036`（逐声部与每小节网格，同一族"生成粒度"轴）；`ARCH-DET-001`
- **实现落点**: `crates/yeban-mcp/src/domain/section_build.rs`（`PlanOptions`、`theory_contents`、
  `drum_contents`、`melody_contents`、`plan_with_options` 的消费校验）、
  `crates/yeban-mcp/src/domain/mod.rs`（`arg_u64` 读取）、`crates/yeban-mcp/src/tools.rs`、
  `schemas/mcp-tools.schema.json`；`crates/yeban-theory`（**只读其既有公开面，本票不改它**）
- **日期**: 2026-10-28

## 背景（先测后改，AGENTS.md §6）

审计 `G8` 的登记原文（缺口文档 `GAPS.md:139`）逐字为「**无鼓密度/力度、无声部节奏、无旋律动机、无音域控制**」（五个轴）与
`NOTES.md:106` 第 4 条「鼓/声部的节奏与力度不可调——ostinato、定音鼓、BRAAAM 全部手写音符 + 循环」。
密度一族的四片（`ADR-0028`／`ADR-0033`／`ADR-0035`／`ADR-0036`）已落地，只核销了
**音域、鼓密度、声部节奏**三轴 ⇒ **力度**与**旋律动机**两轴从未交付，而
`ADR-0033` D4 第 3 条与 `ADR-0035` 非范围第 2 条都**明文**把力度推给"下一票"
（"故本票固定'力度恒 96'"）。因此 `G8` 记为"消除"是**过度申报**：本票兑现力度那张欠条，
台账的 `G8` 闭合度同步改正，旋律动机／`MelodyConstraints` 那轴登记为 **`G8f`**（⛔ 未开工，
见台账 `## 2026-10-28：G8 闭合度更正` 第 5 条读数）。

实测链条（每条现读，⛔ 推断；测于 `main` = `e1311434`）：

1. **力度是两个硬编码常量**：`section_build.rs:172` `const THEORY_VELOCITY: u8 = 96;`、
   `:175` `const ACCENT_VELOCITY: u8 = 112;`。`crates/yeban-theory/src/` 里
   `velocity` 出现 **0** 次 ⇒ 力度赋值全部发生在 MCP 层，theory 不参与。
2. **三级消费点**：和声 `:1544` 恒 `96`；鼓 `:1642-1646` = `hit.accent ? 112 : 96`；
   旋律 `:1707-1711` = `note.chord_tone ? 112 : 96`。
3. **`accent` 不是力度概念**：`crates/yeban-theory/src/drum.rs:251` `pub accent: bool`，
   由 `weight >= STRONG_BEAT_WEIGHT`（`:723`）导出 —— 它是**节拍强度**，不携带任何动态值。
   ⇒ 今天可达的力度面恰好是**两级阶梯**，没有第三级。
4. **入参面没有任何力度入口**：`PlanOptions`（`section_build.rs:1144-1177`）共 8 个字段
   （`material_source`／`progression`／`include_drums`／`include_melody`／`drum_density`／
   `voice_subdivision`／`voice_subdivisions`／`onsets_per_bar`），
   `schemas/mcp-tools.schema.json` 的 `yeban_propose_section` 属性集与之镜像，
   并把"重音 112 其余 96"写死在**输出描述**里（`:504`／`:509`／`:525`）。
5. **手工绕行确实存在**：`yeban_edit_notes` 的 `kind: "velocity"`（读 `notes.rs:4394-4397`，
   写 `:8333-8342`）可逐轨逐音改，但那是**合并之后**的第二次往返，且 `setNoteExpression`
   覆盖 probability/ratchet/microTiming 而**不含** velocity。

## 决定

### D1 两个可选整数入参 `velocityMin` / `velocityMax`，取值 `0..=127`

把既有的**两级阶梯**直接铺到调用者给的区间上：

| 既有 tier | 今天的值 | 本票后的值 |
| :--- | :--- | :--- |
| 和声声部（无 tier 区分） | 96 | `velocityMin` |
| 鼓 `hit.accent == false` / 旋律 `chord_tone == false` | 96 | `velocityMin` |
| 鼓 `hit.accent == true` / 旋律 `chord_tone == true` | 112 | `velocityMax` |

⛔ 不新增第三级、⛔ 不发明新的重音模式（`ADR-0033` 那条推迟**继续有效**：本票只把已有的
两级**换成调用者给的值**，tier 的判定口径仍逐字来自流派登记）。

### D2 缺省 = `velocityMin: 96` / `velocityMax: 112` ⇒ 逐字节维持既有输出

两个键都不给 ⇒ 走的就是今天那两个常量，因此 `ARCH-DET-001` 的跨重启/跨架构一致性与既有
goldens **不受影响**（`ADR-0016` 的缺省逐字节兼容契约）。⚠ 只给其中一个键是合法的：
另一半沿用缺省值（例：`{ "velocityMax": 127 }` ⇒ 阶梯 `96 / 127`）。

### D3 唯一的交叉约束是 `velocityMin <= velocityMax`，越界与倒序都**响亮拒绝**

拒绝口径沿用 `onsetsPerBar` 那一族（`section_build.rs:1863-1877`）：
`BuildCode::InvalidParameterRange` + 载荷 `{"velocityMin", "velocityMax", "range": [0, 127]}`
或 `{"hint": …}`。⚠ 载荷里**不写** `reason` 键 —— 那族拒绝本来就没有 `reason`，
`fault_vocabulary` 的词表与规范面摘要因此**不变**（⛔ 为了少改判据而发明新词，也⛔ 反过来
为了改判据而给旧词换名字）。倒序（`min > max`）被拒的理由：那是"反重音"，属于演奏法
而非力度范围，本票不替它编语义（见 N2）。

### D4 只允许与 `materialSource: "theory"` 同给；⛔ 不要求鼓/旋律在场

力度两档同时作用于**和声、鼓、旋律**三条生成路径，所以"theory 在场"即足够
（与 `voiceSubdivision` 同口径，⛔ 沿用 `onsets_per_bar` 那条更严的"必须有消费方"）。
工程材料路径（`materialSource: "project"`）的力度来自材料本身 ⇒ 给了本票参数就是
**未被消费的入参**，按"给了必须被消费"的既有纪律拒绝。

## 非范围（登记，⛔ 补齐）

- **N1 逐声部力度表**（如 `{"Bass": 72, "Soprano": 108}`）不做：先要 D1 的两档口径稳定，
  而且它会与 `ADR-0035` 的逐声部形状纠缠成"同一张表两个写者"。
- **N2 反重音／力度曲线**（crescendo、按小节渐强、按拍位的 velocity curve）不做：
  theory 层没有"力度包络"概念，做了就是在本票里发明新语义。
- **N3 生成后自动改力度**（把 `edit_notes` 的 `velocity` op 折进同一次提案）不做：
  那是 `G13`（往返压缩）的口径，与本票的入参面正交。

## 判据（可观察量，⛔ 写"应该可用"）

1. **缺省逐字节**：同一 `section_id` 下不传两个键 ⇒ 每条生成音符的 velocity ∈ {96, 112}
   且与传 `{ "velocityMin": 96, "velocityMax": 112 }` 时**逐音相同**（含音符 id）。
2. **映射正确**：传 `{ "velocityMin": 40, "velocityMax": 100 }` ⇒ 和声**全** 40；
   鼓与旋律恰好等于既有 tier 集合（`accent`／`chord_tone` 为 100，其余 40），
   且**音符条数与 id 序列**与缺省跑一次完全相同（力度⛔ 参与 id 派生 —— 现读
   `deterministic_id` 的种子只含 section/voice/index，本判据把这条也钉住）。
3. **拒绝面**：越界（`128`）、非整数、倒序、`materialSource` 非 theory 各自
   `InvalidParameterRange` 并点名是哪个键。
4. **词表不变**：`fault_vocabulary` 的出现数／词表／规范面摘要与落地前**同一组数字**
   （本票⛔ 新增 `reason` 词）。
