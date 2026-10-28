# ADR-0041 — 生成鼓的摇摆比例：把 `GenreRule::drum_pattern` 里写死的 `permille` 实参开放为 `swingPermille`（缺省 = 不覆盖该流派登记值 ⇒ 契约⛔ 写 `default`）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent，参考 Logic Pro / 通行节奏制作里"swing = 反拍后移的比例"的口径自行裁决）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ADR-0016`（缺省逐字节兼容契约）、
  `ADR-0017`（鼓/旋律声部的 opt-in 与 `materialSource:"theory"` 前置）、
  `ADR-0028`（`drumDensity` 的耦合口径 = "要鼓轨在场"，本票取同一支）、
  `ADR-0036`（`onsetsPerBar` —— 与本票同处鼓网格、但是**另一根**轴）、
  `ADR-0037`（力度阶梯：本票⛔ 动力度）、`ADR-0040`（和声跳进：那是一处"只要求材料源"的
  **较松**口径，本票是"多要求一个声部在场"的较紧一支）、`ARCH-DET-001`
- **实现落点**: `crates/yeban-mcp/src/domain/section_build.rs`（`PlanOptions`、`swing_permille_bound`、
  `DrumRulers`、`theory_drum_pattern`、`drum_contents`、`plan_with_options` 的消费校验）、
  `crates/yeban-mcp/src/domain/mod.rs`（`arg_u64` 读取）、`crates/yeban-mcp/src/tools.rs`、
  `schemas/mcp-tools.schema.json`；`crates/yeban-theory`（**只读其既有公开面，本票不改它**）
- **日期**: 2026-10-28

## 背景（先测后改，AGENTS.md §6）

`G8` 那条多轴缺口到此已核销五根轴（密度、声部节奏、音域、力度、两处跳进），剩下的是鼓的
**微时值**：MCP 侧调用 theory 拿鼓组型时，唯一没被开放的那个量就是摇摆比例 ——
`GenreRule::drum_pattern`（`crates/yeban-theory/src/genre.rs:515`）逐字节等于

```rust
swung_styled_drum_pattern(self.meter_value(), bars, onsets_per_bar,
                          self.swing_permille()?, None,
                          default_backbeat(self.meter_value()), self.drum_style)
```

即整条鼓线上"流派登记值不可覆盖"这一处硬编码（`swing_permille()` 读的是 `GENRES` 表里的
`swing: Option<f32>` 字段）。本轮先量（本机一次性探针，路径依赖 `yeban-theory`，⛔ 重依赖、
⛔ 不提交；夹具 = MCP 生成路径的实际形状：`synthwave` 4/4、4 小节、每小节 16 格位；
单位＝**条（个 hit）/ tick（起点位移）/ 千分比（档位）/ 条（流派）**）：

| 读数 | 值 |
|---|---|
| `synthwave` 的登记值 | `swing: None` ⇒ `swing_permille()` = `None`（**⛔ 不是 `Some(500)`**） |
| `None` 路径 vs 显式 `Some(500)` | 88 条 hit 的 `(音色, 起点, 重音)` **逐条相同** |
| 条数（个）随档位 | `500 / 580 / 660 / 700 / 1000` 全部恒 **88** ⇒ 本键⛔ 不筛点 |
| 音色与重音（个不同） | 全部恒 **0** ⇒ 本键⛔ 不改 GM 键、⛔ 不改 `accent`（因而⛔ 不改力度） |
| 位移和（tick，对 `500` 档） | `500`→0、`580`→1216、`660`→2432、`700`→3072、`1000`→7648（**单调不减**） |
| 单条最大位移（tick） | `580`→38、`660`→76、`700`→96、`1000`→239（受影响的恒为 32 条反拍） |
| 越界 | `499` ⇒ `SwingOutOfRange{value:499}`、`1001` ⇒ `SwingOutOfRange{value:1001}`（theory **⛔ 静默钳制**） |
| `GENRES` 的登记分布 | 182 条：**148 条不登记**、**34 条登记**，折算档位集合恰为 `{540,560,580,600,620,640,660}` ⇒ **⛔ 一条登记 `500`** |
| 三条非平直流派与平直档的位移和（tick） | `bossa_nova(540)`→608、`lo_fi_hip_hop(560)`→896、`bebop(580)`→1216（条数仍恒 88） |

四条读数各自的含义（第 1、3 条**推翻**了"照抄 melodyMaxLeap 那票"的写法）：

1. **缺省不是一个数字**（末两行）：148 条走"未登记"那一支、34 条各有各的值 ⇒
   契约里写 `default: 500` 会把 34 条说成 148 条那样，那是**错的契约**（与前面四票的
   `default` 字段是**唯一一处**结构性差别）。
2. **`None` 与 `Some(500)` 在夹具上逐条相同** ⇒ "键缺席 ≡ 平直档"这条判据**可写**，
   但它是**实测等价**而⛔ 定义 —— 本层实现⛔ 依赖它（缺席时结构上仍走 `None` 分支）。
3. **位移与档位单调**（第 5 行）⇒ 判据除了"至少一条变了"还要量**方向与强度一致**，
   否则把千分比当成 tick 数或百分数传下去（同类型、同编译结果）照样绿。
4. **theory 已经响亮拒绝越界**（`SwingOutOfRange`）⇒ 本层判**形状**、theory 判**可行性**，
   两件事⛔ 混成一条；而 `ProgressionTooDense`（顺手实测：`operetta` 在 16 格位下
   `{degrees:16, slots:12}`）属 `onsetsPerBar` 那根轴（`ADR-0036`），本票⛔ 碰它。

## 决定

- **D1 — 新增可选键 `swingPermille: integer`，取值 `500..=1000`，缺省 = 不覆盖该流派登记值**
  ⇒ 键缺席必须**逐字节**产出同一份 `SectionPlan`（`ADR-0016`）。单位 = **千分比**
  （一对格位里第二个 onset 的后移比例，`500` = 平直），⛔ 百分数、⛔ tick 数、⛔ "每小节推多快"。
  ⚠ 本键⛔ 登记 `default`（读数 1）：缺省语义是"读流派表"，其唯一写者是 `GENRES`。
- **D2 — 界复用 theory 公开的那对常量** `SWING_PERMILLE_STRAIGHT`（500）/ `SWING_PERMILLE_MAX`（1000）
  ⇒ 本层⛔ 出现第三个写者（`ADR-0039` D4 那条 `midi_pitch_bound` 的处置在此**不适用**：
  那是 `0..=127` 一支，本键的界与单位都不同，故另立 `swing_permille_bound` 一个函数、
  ⛔ 把两类界折进同一个校验器 —— 折在一起会让报文里的 `range` 与文档不符）。
  `u16::try_from` 挡 `>65535`，`contains` 挡 `500` 以下与 `1000` 以上 ⇒ 一条表达式覆盖两种越界。
- **D3 — 消费校验取"较紧"的一支**：`materialSource:"theory"` **且** `includeDrums:true`。
  理由是消费点（鼓轨）与 `drumDensity` 完全同一个 ⇒ 同一口径；与 `ADR-0040` D3 的和声键
  是**不同**一支（那一个恒在场）。⛔ 把三处口径（旋律三键 / 和声键 / 本键）折成同一个
  `is_some()` 表达式：折在一起必然把某一支的要求扩散到别的键上，造出一个文档里没写的拒绝。
  材料源那一支由 `ADR-0017` 既有检查 + 本票一条显式检查共同覆盖（两支报文都点名 `materialSource`）。
- **D4 — `drum_contents` 的参数收成 `DrumRulers { density, onsets_per_bar, ladder, swing }`**：
  加第 8 个参数会撞 `clippy::too_many_arguments`，而它在本仓是**承重信号**（`G8f` 第一批
  Run `40609755301` 就烧在这条上，8/7）⇒ ⛔ 用 `#[allow]` 压掉。另一条更硬的理由与
  `MelodyRulers` 同形：这几个数里存在同类型的组合，写成命名字段后"调错实参顺序"编译失败。
- **D5 — 接线只换那一个实参**：`swing = None` 时**原样**调用 `rule.drum_pattern(bars, onsets)`
  （⇒ 缺省兼容是**结构**保证，⛔ "两个调用恰好算得一样"这种运气）；`Some(permille)` 时显式调用
  `swung_styled_drum_pattern(..., Some(permille), None, default_backbeat(meter), style)`
  —— 与薄封装的差别只在 `permille`。`Ok(None)` 兜底分支保持既有形状（其 permille 取
  `swing.or(登记值)` ⇒ 缺席路径逐字节不变）。
- **D6 — 可行性判定⛔ 上移到本层**：越界（`499`/`1001`）由本层的形状检查先拒（读数 4），
  "该比例在此拍号/格位下产不产得出鼓点"仍由 theory 决定，转发经既有
  `theory_grid_invalid(style, "swung_styled_drum_pattern", …)` 原样带出（⛔ 抄一张「流派 × 比例」表）。
- **D7 — ⛔ 新增故障词**：拒绝载荷⛔ 带 `reason` 键；`Some` 分支的 `theoryStep` **复用**本文件
  已有的字符串 `"swung_styled_drum_pattern"`（兜底分支就在用它）⇒ `fault_vocabulary` 的词表
  与规范面摘要不变。

## 非范围（登记，⛔ 补齐）

- `N1` **旋律 onset 网格的摇摆**：`GenreRule::rhythm_grid` 同样读 `swing_permille()`，本票
  ⛔ 接它 ⇒ 判据 2 里"旋律声部逐字节不变"是**当下的范围声明**，⛔ "音乐上旋律该不该跟着摆"的
  主张。要不要一根键管两层（鼓 + 旋律网格）需要新的裁决。
- `N2` **和声声部的微时值**（`voice_leading` 那条线上没有任何 swing 概念）。
- `N3` **`grouping`（复合拍的拍内分组）实参**：theory 侧是 `Option<BeatGrouping>`，MCP 两处调用
  都写死 `None` ⇒ 入参化它是 6/8 那类拍号的**另一根**轴（与 `ADR-0036` 非范围 3 相邻）。
- `N4` **逐对 / 逐小节的 swing 曲线**（theory 只有单个千分比，字段本身也是单个 `Option<f32>`）。

## 判据（`crates/yeban-mcp/tests/section_drum_swing.rs`，6 条）

1. 键缺席 ≡ 给 `500` ⇒ `SectionPlan` 逐字节相同（读数 2 的实测等价）；顺带钉住夹具形状
   （鼓轨多条、和声声部在册、鼓的力度只取阶梯两档 `{96,112}`）。
2. 给 `700` ⇒ 鼓轨的 **id 集合 / 音高 / 时值 / 力度**逐条与缺省跑相同（⛔ 不筛点、⛔ 不改音色、
   ⛔ 不改时值、⛔ 不改力度）、**至少一条起点被推移**（防空判据）；和声各声部与旋律与缺省跑
   **逐字节**相同（D3/N1 的消费面）。
3. 越界（`0` / `499` / `1001` / `65536` / `100000`）⇒ `INVALID_PARAMETER_RANGE`，载荷点名
   `field:"swingPermille"` + `value` + `range:[500,1000]`，⛔ `reason` 键；两个端点 `500`/`1000`
   必须放行。⚠ 这里 `499`/`1001` 的拒绝**必须**带 `field`（= 本层的参数判定）而⛔ 是 theory 的转发。
4. 耦合：脱离 `materialSource:"theory"` ⇒ 拒且 `hint` 点名 `materialSource`；
   `includeDrums:false` ⇒ 拒且 `hint` 点名 `includeDrums`；
   ⚠ **反钉**：`includeMelody:false` 时必须放行（⛔ 把旋律那条更严的要求扩散过来）。
5. 位移强度读数：`{500,580,700,1000}` 四档相对缺省跑的鼓轨起点位移和（tick）
   **恰为 0 / 严格递增**（读数 3）—— 这条才是"接错实参"（千分比 ↔ tick）能被抓到的地方。
6. 两个声明面（`tools.rs` 的 `param` 与 schema 的 property）同源：类型 `integer`、
   `minimum = SWING_PERMILLE_STRAIGHT`、`maximum = SWING_PERMILLE_MAX`；
   ⭐ 且 schema **⛔ 存在 `default` 键**（D1：缺省随流派移动 ⇒ 固定数字是错的契约）。
