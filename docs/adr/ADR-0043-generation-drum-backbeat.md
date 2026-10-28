# ADR-0043 — 生成鼓的军鼓落点：把 `GenreRule::drum_pattern` 里写死的 `default_backbeat(meter)` 实参开放为 `drumBackbeat`（缺省 = 不覆盖派生值 ⇒ 契约⛔ 写 `default`，且⛔ 写 `maximum`）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定，与 `ADR-0041` 同族同口径）**
- **决定者**: 集成者（Agent，参考通行配器里"backbeat = 军鼓落在第几拍"的口径自行裁决）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ADR-0016`（缺省逐字节兼容契约）、
  `ADR-0017`（鼓声部 opt-in 与 `materialSource:"theory"` 前置）、`ADR-0028`/`ADR-0036`
  （`drumDensity` / `onsetsPerBar` —— 同处鼓线但是**另外两根**轴）、
  `ADR-0041`（`swingPermille`：本票的**形状模板**，但两键的**界**不同支，见 D2）、
  `ARCH-DET-001`
- **实现落点**: `crates/yeban-mcp/src/domain/section_build.rs`（`PlanOptions`、`backbeat_bound`、
  `DrumRulers`、`theory_drum_pattern`、`drum_contents`、`plan_with_options` 的消费校验）、
  `crates/yeban-mcp/src/domain/mod.rs`（`arg_u64` 读取）、`crates/yeban-mcp/src/tools.rs`、
  `schemas/mcp-tools.schema.json`；`crates/yeban-theory`（**只读其既有公开面，本票不改它**）
- **日期**: 2026-10-28

## 背景（先测后改，AGENTS.md §6）

**这一票在数什么**：`backbeat` 每取一个值时，鼓组型里的**击点条数**（单位＝条）、
其中**军鼓条数**（单位＝条）、以及军鼓落在的**拍序号集合**（单位＝第 0 基的拍号，已去重）。
测法＝仓库外一次性探针 `rustc --extern yeban_theory=target/debug/deps/libyeban_theory-*.rlib`
（⛔ 重依赖、⛔ 不提交），夹具 = MCP 生成路径的实际形状（4 小节、`onsets_per_bar = 每拍 4 格`、
`DrumStyle::Metric`、`permille = None`、`grouping = None`）。

`theory` 侧早就有这一维，只是 MCP 把它**写死**了：`GenreRule::drum_pattern`
（`crates/yeban-theory/src/genre.rs:515`）逐字节等于

```rust
swung_styled_drum_pattern(self.meter_value(), bars, onsets_per_bar,
                          self.swing_permille()?, None,
                          default_backbeat(self.meter_value()), self.drum_style)
```

而 `crates/yeban-theory/src/drum.rs:258/266` 公开了 `pub type Backbeat = u8` 与
`default_backbeat(meter)`（偶数拍取中点、奇数拍取中间那拍）。MCP 的两个调用点
（`section_build.rs` 的 `Some` 分支与 `Ok(None)` 兜底分支）都直接写 `default_backbeat(...)`
⇒ 与 `G8i` **完全同形**的一处"派生值不可覆盖"。⚠ `ADR-0041` 的非范围 `N3` 登记的是
`grouping`，**不包括** `backbeat`（现读该 ADR 的 `N1..N4` 四条）⇒ 本票不在任何已开票的非范围里。

另外一条**已在 theory 文档里登记**的理由（`drum.rs:31`）：`pending 3` 要的"每条流派打什么样的
鼓点序列"只登记了**底鼓落点**这一维（`DrumStyle`），"军鼓位置仍走 `Backbeat` 参数" ⇒
把 `backbeat` 开放为入参，正是那半片数据缺口的**调用方一侧**；流派表那一半（给每条流派登记
一个 `backbeat`）**不在本票边界**（见 `N1`）。

实测（拍号 × `backbeat`，4 小节）：

| 拍号 | 拍数 | `bb=0` | `bb=1` | 派生值（缺省） | 其余合法值 | `bb=拍数+1` |
|---|---|---|---|---|---|---|
| 4/4 | 4 | `Ok(None)` | 总 96／军鼓 16／`{0,1,2,3}` | `2` → 88／8／`{0,2}` | `3` → 88／8／`{0,3}`；`4` → 84／4／`{0}` | `Ok(None)` |
| 3/4 | 3 | `Ok(None)` | 总 68／军鼓 12／`{0,1,2}` | `2` → 64／8／`{0,2}` | `3` → 60／4／`{0}` | `Ok(None)` |
| 2/4 | 2 | `Ok(None)` | 总 48／军鼓 8／`{0,1}` | `1` → 48／8／`{0,1}` | `2` → 44／4／`{0}` | `Ok(None)` |
| 5/4 | 5 | `Ok(None)` | 总 108／军鼓 20／`{0,1,2,3,4}` | `3` → 96／8／`{0,3}` | `2` → 100／12；`4` → 96／8／`{0,4}`；`5` → 92／4 | `Ok(None)` |
| 6/8 | 2 | `Ok(None)` | 总 56／军鼓 8／`{0,1}` | `1` → 56／8／`{0,1}` | `2` → 52／4／`{0}` | `Ok(None)` |

五条读数（第 1、3 条**推翻**"照抄 `ADR-0041` 的判据"的写法）：

1. **本键会加／筛击点**（总数 96 ↔ 88 ↔ 84）—— 与摇摆那根轴（恒 88 条）**相反** ⇒
   `G8i` 判据 2 那句"鼓轨 id 集合逐条不变"在此**必须换成**"非军鼓声部逐条不变 + 军鼓条数随值变"。
2. **合法界随拍号移动**（`1..=拍数`）⇒ schema⛔ 写 `maximum`（写了就会对 2/4 报出 5 这种"契约说能、
   本层拒"的矛盾），也⛔ 写 `default`（派生值随拍号移动：4/4→2、3/4→2、2/4→1、5/4→3、6/8→1）。
   这是本票与 `ADR-0041`（界是 theory 常量对）的**结构性差别**。
3. **军鼓条数不是单调读数**（5/4：`bb=3` 与 `bb=4` 都得 8 条，但落点分别是 `{0,3}` 与 `{0,4}`）⇒
   判据必须钉**拍序号集合**，只数条数会漏掉"把整除写乘法/取模写反"这一类。
4. **theory 对越界返回 `Ok(None)`**（`bb=0`、`bb=拍数+1`），⛔ 错误、⛔ 静默钳制（`drum.rs:80`
   与 `validate_swing_permille` 同一口径）⇒ 本层必须**先判**并把 `field` 带进载荷，否则用户看到的
   是兜底分支那句"流派 X 的拍号推不出可用的鼓组型"（不点名本键）。
5. **`bb=1` 的语义是"每一拍都打"**（`beat.is_multiple_of(1)` 恒真），⛔ 不是"不打军鼓"；
   想表达"不要军鼓"是另一根轴（本票⛔ 发明，见 `N2`）。

## 决定

- **D1 — 新增可选键 `drumBackbeat: integer`，界 = `1..=该流派拍号的拍数`，缺省 = 不覆盖派生值**
  ⇒ 键缺席必须**逐字节**产出同一份 `SectionPlan`（`ADR-0016`）。单位 = **拍序号**（第 0 基），
  语义 = "军鼓打在能被它整除的那些拍的起点"（`drum.rs` 语义 5），⛔ 小节号、⛔ tick、⛔ 十六分格位。
  ⚠ 契约里⛔ `default`、⛔ `maximum`（读数 2）；`minimum: 1` 仍登记（那一支不随拍号移动）。
- **D2 — 界校验函数带拍号入参**：`backbeat_bound(raw: Option<u64>, meter: Meter) -> Result<Option<Backbeat>, BuildFault>`，
  用 `yeban_theory::rhythm::felt_beats_per_bar(meter)` 现读上界 ⇒ 本层⛔ 另立一张「拍号 → 拍数」表
  （与 `ADR-0041` D2 的"界复用 theory 公开面"同一条理由，只是这里公开的是**函数**而⛔ 常量对）。
  `u8::try_from` 挡 `>255`，`1..=felt` 挡 `0` 与超拍数 ⇒ 一条表达式覆盖三种越界，载荷同形状
  （`field` + `value` + `range: [1, felt]`）。
- **D3 — 消费校验取 `ADR-0041` D3 那一支**：`materialSource:"theory"` **且** `includeDrums:true`，
  两个报文分别点名 `materialSource` / `includeDrums`，⛔ 与旋律/和声那两支折成同一个 `is_some()`。
- **D4 — `DrumRulers` 加**第 5 个字段 `backbeat: Option<Backbeat>`（`ADR-0041` D4 的那个结构），
  ⛔ 再走"加第 9 个位置参数"——`density`/`onsets_per_bar`/`ladder`/`swing`/`backbeat` 里
  现在**有三对**同类型的整数，命名实参是唯一能让"传错顺序"编译失败的形状。
- **D5 — 接线只换那一个实参，并保持**"两键都不给 ⇒ 走 `rule.drum_pattern`"这一支：
  `(swing, backbeat) = (None, None)` 时**原样**调用薄封装（缺省兼容是**结构**保证）；
  任一为 `Some` 时走 `swung_styled_drum_pattern(..., swing.or(rule.swing_permille()?), None,
  backbeat.unwrap_or(default_backbeat(meter)), style)` —— `or` 那两支分别等于薄封装内部读的东西，
  因此"只给一个键"不会顺带改掉另一根轴。`Ok(None)` 兜底分支取**同一对**解算值，⛔ 分叉。
- **D6 — 可行性判定⛔ 上移**："该落点在此拍号/格位下产不产得出鼓点"仍由 theory 决定并经既有
  `theory_grid_invalid(..., "swung_styled_drum_pattern", ...)` 原样转发（⛔ 抄表）；
  本层只判 D2 那条形状（读数 4 要求本层先拒，否则报文不点名本键）。
- **D7 — ⛔ 新增故障词**：拒绝载荷⛔ `reason` 键；`Some` 分支的 `theoryStep` **复用**本文件已有的
  `"swung_styled_drum_pattern"` ⇒ `fault_vocabulary` 词表与规范面摘要不变。

## 非范围（登记，⛔ 补齐）

- `N1` **给每条流派登记一个 `backbeat`**（`GENRES` 数据面，theory 侧要加字段）⇒ 本票只开放
  **调用方**那一侧；"哪些流派本该打 1、哪些打 2"是数据裁决，⛔ 本票发明。
- `N2` **`DrumVoice` 级的开关**（关掉军鼓／只留踩镲）：本键⛔ 能表达"不打"（读数 5）。
- `N3` **军鼓的音色与力度**：GM 键 38 与 `accent` 判据不变，本票⛔ 碰（实测只改落点与条数）。
- `N4` **`grouping`（复合拍拍内分组）**：仍是 `ADR-0041` 的 `N3`，与本票是另一根轴。

## 判据（`crates/yeban-mcp/tests/section_drum_backbeat.rs`，6 条）

1. 键缺席 ≡ 给派生值（夹具 `synthwave` 4/4 ⇒ `2`）⇒ `SectionPlan` **逐字节**相同（读数 1 的
   实测等价；实现上走的是薄封装那一支）。顺带钉住夹具形状（鼓轨多条、军鼓在册、和声声部在场）。
2. 给 `1` 与给 `4` ⇒ **非军鼓**击点（底鼓／踩镲／吊镲）的 `(音色, 起点, 时值, 力度)` 逐条与缺省跑相同、
   **军鼓条数改变**（8 → 16 / 4）、军鼓拍集合分别是 `{0,1,2,3}` 与 `{0}`（读数 1、3）；
   和声各声部与旋律与缺省跑**逐字节**相同（D3 的消费面）。
3. 越界（`0` / `5` / `256` / `100000`）⇒ `INVALID_PARAMETER_RANGE`，载荷点名 `field:"drumBackbeat"`
   + `value` + `range:[1,4]`（`4` = 现读 `felt_beats_per_bar(synthwave 的 4/4)`），⛔ `reason` 键；
   两个端点 `1`/`4` 必须放行（读数 4：本层先拒，⛔ 让 theory 的 `Ok(None)` 变成"拍号推不出鼓组型"）。
4. 耦合：脱离 `materialSource:"theory"` ⇒ 拒且 `hint` 点名 `materialSource`；`includeDrums:false`
   ⇒ 拒且 `hint` 点名 `includeDrums`；⚠ **反钉**：`includeMelody:false` 时必须放行。
5. 拍号移动那一支（读数 2）：另一拍号的夹具（3/4 或 6/8 那条流派）上，同一个值的**合法性**与
   `range` 载荷随 `felt_beats_per_bar` 移动 ⇒ 钉住"本层⛔ 抄表、界来自 theory 函数"。
6. 两个声明面（`tools.rs` 的 `param` 与 schema 的 property）同源：类型 `integer`、`minimum = 1`；
   ⭐ 且 schema **⛔ 存在 `default` 键、⛔ 存在 `maximum` 键**（D1：两者都随拍号移动）。
