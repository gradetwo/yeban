# ADR-0036 — 鼓与旋律的每小节音头数：默认从**拍号**导出，并开放 `onsetsPerBar`

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ADR-0017`（D1：`includeDrums` /
  `includeMelody` 与 16 分网格常量）、`ADR-0028`（`drumDensity`：按 `hit.accent` 筛点，
  ⛔ 不改网格分辨率）、`ADR-0033`（`voiceSubdivision`，**每跨度**口径；D2 曾把
  `onsetsPerBar` 这个词留给**每小节**口径，本票就是那个每小节口径）、`ADR-0035`
  （`voiceSubdivisions`，逐声部）；`ARCH-DET-001`、`ADR-0016`（缺省逐字节兼容契约）
- **实现落点**: `crates/yeban-mcp/src/domain/section_build.rs`
  （`THEORY_ONSETS_PER_BAR`、`theory_drum_pattern`、`theory_melody`、`PlanOptions`、
  `plan_with_options` 的参数消费校验）、`crates/yeban-mcp/src/tools.rs`、
  `schemas/mcp-tools.schema.json`、`crates/yeban-theory`（**只读其既有公开面，本票不改它**）
- **日期**: 2026-10-28

## 背景（先测后改，AGENTS.md §6）

审计 `G8` 的残余登记写的是「旋律/鼓的每小节音头数仍是常量
`THEORY_ONSETS_PER_BAR = 16`」（`section_build.rs`）。测量之后，这条残余**比登记得更重**：
它不只是一处"不可调"，而是**28 个风格预设的鼓与旋律现在根本生成不出来**。

实测链条（每条都是现读，⛔ 推断）：

1. **常量确实无条件传给 theory**：`section_build.rs` 里 `THEORY_ONSETS_PER_BAR` 的
   三处消费点（`rule.drum_pattern(bars, THEORY_ONSETS_PER_BAR)`、
   `swung_styled_drum_pattern(..., THEORY_ONSETS_PER_BAR, ...)`、
   `rule.rhythm_grid(theory_bars(bars)?, THEORY_ONSETS_PER_BAR)`）都**没有**按拍号折算。
2. **theory 的界是"该拍号一小节有几个 16 分格位"**：`cells_per_bar(meter) =
   ticks_per_bar / GRID_CELL_TICKS`，`GRID_CELL_TICKS = 240`（960 PPQ 下的 16 分音符），
   既有单测逐条钉住（`rhythm.rs:746-752`）：4/4 → **16**、3/4 → **12**、2/4 → **8**、
   6/8 → **12**、7/8 → **14**、5/4 → **20**；
   `build_weighted_grid` 在 `wanted > slots` 时
   `Err(TheoryError::ProgressionTooDense { degrees, slots })`（`rhythm.rs:663`），
   鼓侧同一条界（既有单测 `drum.rs:1477` 钉住
   `styled_drum_pattern(Meter::MARCH /*2/4*/, 1, 9, …)` ⇒ `ProgressionTooDense{degrees:9, slots:8}`）。
   ⭐ 那条界的名字本身就在说"这不是夹紧、是拒绝"：`an_over_dense_request_is_an_error_not_a_silent_clamp`。
3. **哪些预设受影响（现读 `GENRES`）**：`awk '/^pub static GENRES/,/^\];/'` 数 `id:` 行
   = **182** 条流派；同一区间数 `meter:` 行按值分组 =
   `4/4` **154**、`3/4` **13**、`2/4` **8**、`7/8` **4**、`6/8` **3**
   ⇒ 非 4/4 共 **28** 条，它们的格位数（8 / 12 / 14）**都小于 16**。
4. **这 182 条全都暴露在 MCP 的风格预设面上**：既有判据
   `section_build.rs:2065` 对 `GENRES` 逐条 `preset_parts(rule.id).expect(...)`
   ⇒ ⛔ 存在"28 条不在预设面上"的豁免。
5. **症状是响亮失败，不是静默降级**：`theory_invalid`（`section_build.rs:590`）把
   `TheoryError` 包成 `BuildCode::InvalidParameterRange`，载荷带
   `{"stylePreset", "theoryStep", "theoryError"}`。⇒ 没有违反"不得静默"的红线，
   但创作面上 `stylePreset:"waltz" + includeDrums:true` 这一类调用**今天必失败**。

`ADR-0017` 把 16 写死时的口径是"4/4 的 16 分网格"；它**没有**登记"其他拍号怎么办"，
而流派表后来扩到了 182 条 ⇒ 这是一处**契约缺口随数据增长暴露**的既有缺陷，
不是本票新发明的需求。

## 决定

### D1 缺省值改为「该拍号一小节的 16 分格位数」，⛔ 常量 16

`default_onsets_per_bar(meter) = cells_per_bar(meter)`（`None` 时按既有病态拍号路径处理，
本票不新增分支）。

- 对 **154 条 4/4** 预设：`cells_per_bar = 16` ⇒ 与今天**逐字节相同**
  （`ADR-0016` 的缺省兼容契约由这条不变量守住，判据 1 直接钉它）。
- 对 **28 条**非 4/4 预设：把"必失败"变成"生成得出来"。⚠ 这不是行为变更而是
  **缺陷修复** —— 那 28 条在旧默认下没有任何输出可言，逐字节兼容无从谈起。
- ⛔ 写成"取 `min(16, cells)`"：那种写法在 5/4（20 格）下会把默认压到 16，
  等于给 4/4 之外的拍号另立一套隐含口径，且与 D3 的"界从 theory 读回"重复。
  默认就是**整张网格铺满**，与 4/4 今天的 16 是同一条规则的两个实例。

### D2 新可选整数参数 `onsetsPerBar`（单位 = 每小节音头数）

- **名字沿用 `ADR-0033` D2 腾出来的那个词**：当时 ⛔ 用它给"每跨度切分"命名，
  理由是它已经是错误载荷里**每小节**口径的键。本票正是那个每小节口径的参数
  ⇒ **一个词只有一种单位**，`voiceSubdivision`（每跨度）与 `onsetsPerBar`（每小节）
  分属两条互不换算的轴，说明文字里各自写死单位。
- **作用域**：只作用于 `materialSource: "theory"` 的**鼓组**与**旋律**两条生成路径。
  ⛔ 不动和声声部（那是 `ADR-0033` / `ADR-0035` 的地盘：和声按 `ChordSpan` 走，
  与每小节网格不是同一个量）。
- 下界 `1`；上界**不在本层写常量**（见 D3）。

### D3 上界从 theory 的错误里读回，⛔ 手抄拍号表

显式给了 `onsetsPerBar` 而它超过该拍号的格位数时，theory 返回
`ProgressionTooDense { degrees, slots }`。本票**把这个 `slots` 转发进载荷**：

```json
{ "stylePreset": "...", "onsetsPerBar": 17, "maxOnsetsPerBar": 16,
  "meter": "4/4", "hint": "上界 = 该拍号一小节的 16 分格位数" }
```

⛔ 在 MCP 层维护第二张"拍号 → 格位数"表：`G18b` 那条教训（守卫的语义不能靠手抄名单）
的同族错误 —— 流派表一改拍号，手抄表就悄悄错。界只有**一个来源**（theory），
读回路径是"调用 → 收 `Err` → 取 `slots`"，因此新参数**不需要**预先做静态区间校验；
非整数 / `< 1` 这类**形状**错误仍在 domain 层当场拒（`INVALID_PARAMETER_RANGE`，
与 `voiceSubdivision` 同口径）。

### D4 「给了必须被消费」（`ADR-0028` / `ADR-0033` 同一条口径）

- `onsetsPerBar` 与 `materialSource: "theory"` 不同给 ⇒ `INVALID_PARAMETER_RANGE` + `hint`；
- `includeDrums: false` **且** `includeMelody: false` 时给 `onsetsPerBar` ⇒ 同码拒绝
  （两条消费路径都不在场，参数无处落地）；⚠ `drumDensity` 只要求鼓在场，
  本参数**两条路任一在场即可**，因为它同时管鼓与旋律。

### D5 确定性

音头数进入 id 种子的方式**不变**（沿用 `ADR-0017` 的既有种子形状，本票不追加维度）；
同一 `(stylePreset, bars, onsetsPerBar)` ⇒ 同一份载荷。⚠ 缺省路径的 id 因此**逐字节不变**
（D1 的 4/4 不变量覆盖了它）。

## 判据（随代码一起提交）

1. **缺省逐字节不变（4/4）**：`onsetsPerBar` 不给 vs 给 `16`，对 `synthwave`（4/4）
   的鼓与旋律**逐音符相同**；且与"改动前"的输出比较靠 D1 的算术事实（`cells_per_bar(4/4)=16`），
   ⛔ 判据里手抄一份期望网格。
2. **28 条非 4/4 预设从失败变成功**：对 `GenreLibrary::all()` 里每条**非 4/4** 流派
   （名单从 `GenreLibrary::all()` + `cells_per_bar` **现读**，⛔ 手抄 `["waltz", ...]`），
   `includeDrums: true` + `includeMelody: true` 必须 `Ok`。
   ⚠ 口径修正（本票草案写的是"每小节音头数等于 `cells_per_bar(meter)`"，那**不是可观察量**：
   音头数是**用到的**格位数，可以小于格位数，而摇摆（swing）还会把击点挪离 240 的整数倍 ⇒
   判据⛔ 从音符 tick 反推网格）。可观察的两条替代：
   **给 `onsetsPerBar = cells` 与不给 ⇒ 整个规划逐字节相同**（钉住"缺省就是这个数"），
   **给 `onsetsPerBar = 16` ⇒ 响亮拒绝**（钉住"旧的常量今天在这条流派上必然失败"）。
3. **界从 theory 读回**：给一个超过格位数的 `onsetsPerBar` ⇒ `INVALID_PARAMETER_RANGE`，
   载荷的 `maxOnsetsPerBar` 必须等于 `cells_per_bar(rule.meter_value())`（同一表达式，
   ⛔ 写死的数），且 `meter` 字段与该流派一致。
4. **形状错误**：`0` ⇒ `INVALID_PARAMETER_RANGE`（带 `range`），⛔ 静默夹紧、⛔ 静默当"没给"。
   ⚠ 分层实测口径：**负数 / 非整数 / 字符串在工具面就被拦下**
   （`arg_u64` 的"必须是非负整数"与 `ParamSpec::accepts("integer")`），
   所以纯领域层只能量到 `0` 与"超出 `u32` 宽度"这两行 —— 判据按这条如实分写，
   ⛔ 假装领域层会返回 `ParamSpec` 的拒绝。
5. **作用域**：`onsetsPerBar` 只改鼓与旋律 —— 和声声部逐字节不变（与 `ADR-0033`
   判据 4 的对称形式：那边是"切分不动鼓/旋律"，这边是"网格不动和声"）。
6. **耦合拒绝**：D4 两条各一例（`materialSource: "project"`；两条消费路径都不在场）。
7. **与 `voiceSubdivisions` 正交**：同给两个参数时两者都生效且互不折算
   （和声按跨度切分、鼓/旋律按小节网格），⛔ 一个参数把另一个顶掉。
8. **两个声明面同源**：`tools.rs` 的 `ParamSpec` 与 `schemas/mcp-tools.schema.json`
   的 `yeban_propose_section` 都必须有 `onsetsPerBar`，且 `minimum` 一致。

## 不在本票范围（登记，不发明）

1. **每小节的音头**分布本身（哪些落在反拍、切分型）—— 需要节奏型数据表，
   是 `ADR-0033` D4 第 2 条「非均匀节奏型」的同一件事。
2. **重音/力度随网格密度变化** —— 会动力度面（`ADR-0026` 分级、`fault_vocabulary`
   冻结面），本票固定"力度口径不动"。
3. **`6/8` 的复合拍是否应以"拍"而非"16 分格"为默认单位** —— 乐理口径待人类裁决
   （D1 现在统一取 16 分格位，与 theory 的 `cells_per_bar` 一致；本票不自立第二套）。
4. **旋律 `MelodyConstraints::DEFAULT` 的其他维度**（音域、跳进上限）不动。

## 后果

- 正面：创作面上 28 条流派（含 `waltz`、`minuet`、`baroque_suite` 这类 3/4 与 2/4 舞曲）
  第一次能一次调用拿到"配器骨架 + 鼓 + 旋律"；`G8` 的粒度轴补齐到 4 个参数
  （`drumDensity` 筛点、`voiceSubdivision`/`voiceSubdivisions` 切和声、`onsetsPerBar` 定网格）。
- 成本：动了**缺省值的来源**（常量 → 拍号函数）。风险被 D1 的不变量压住：
  4/4 那一档逐字节不变，非 4/4 那一档在旧行为下本来就是错误路径。
  ⚠ 判据 2 若在某条流派上仍红，说明该流派的鼓组/旋律登记本身有问题，
  那是**下一条缺陷**，⛔ 把默认值改回常量来掩盖。
