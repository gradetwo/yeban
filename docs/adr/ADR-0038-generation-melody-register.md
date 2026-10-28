# ADR-0038 — 生成旋律的音域：把 `MelodyConstraints` 的音高窗口开放为 `melodyLower` / `melodyUpper`（缺省逐字节 = 48 / 84）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent，参考 Logic Pro 的 Melodic Sampler / "Range" 与 DAW 通用的
  note-range 口径自行裁决）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ADR-0016`（缺省逐字节兼容契约）、
  `ADR-0017` D1（旋律声部的引入，其文档串逐字写着"音域 `MelodyConstraints::DEFAULT`"）、
  `ADR-0028`（**同一根轴的前半**：和声声部音域 ⇒ 自构 `SECTION_THREE_VOICES`，并在"没做的"
  第 1 条把"更多音域旋钮"推给下一票）、`ADR-0036`（`onsetsPerBar`：同一生成路径的网格轴）、
  `ADR-0037`（力度阶梯：同一族"给了必须被消费"的耦合口径）；`ARCH-DET-001`
- **实现落点**: `crates/yeban-mcp/src/domain/section_build.rs`（`PlanOptions`、`melody_register`、
  `melody_contents`、`plan_with_options` 的消费校验）、`crates/yeban-mcp/src/domain/mod.rs`
  （`arg_u64` 读取）、`crates/yeban-mcp/src/tools.rs`、`schemas/mcp-tools.schema.json`；
  `crates/yeban-theory`（**只读其既有公开面，本票不改它** —— 沿用 `ADR-0028` D1 的模块边界纪律）
- **日期**: 2026-10-28

## 背景（先测后改，AGENTS.md §6）

审计 `G8` 的登记原文（`GAPS.md:139`）五个轴里，**音域**这一轴此前只闭合了一半：
`ADR-0028` D1 交付的是**和声声部**的音域（`VoicingConstraints` / `SECTION_THREE_VOICES`），
而**旋律声部**的音高窗口从来是 theory 出厂常量。台账 `## 2026-10-28：G8 闭合度更正`
把它登记为 `G8f`（⛔ 未开工）。本票兑现 `G8f` 的**音域半片**。

实测链条（每条现读，⛔ 推断；测于 `main` = `cfaf79cd`，单位＝MIDI 音高值）：

1. **旋律音域是一个硬编码常量，且只有一个消费点**：`section_build.rs:1750`
   `melody_contents` 里 `MelodyConstraints::DEFAULT` —— 整个 `yeban-mcp` 里
   `melody_over_chords` / `genre_melody*` 只出现这一处（`grep` 实测：`section_build.rs`
   之外只有 `tools.rs:753` 的文档串提到它）。
2. **theory 侧的约束是公开可构的**：`crates/yeban-theory/src/melody.rs:87`
   `pub struct MelodyConstraints { pub lower: u8, pub upper: u8, pub max_leap: u8 }`；
   出厂三常数 `MELODY_LOWER_BOUND = 48`（`:62`）、`MELODY_UPPER_BOUND = 84`（`:65`）、
   `MELODY_MAX_LEAP = 12`（`:68`），且 `lib.rs:176` 的 `pub use` 已把三者与
   `MelodyConstraints` 一并重导出（⇒ MCP 侧的 import 路径有两个合法写法，本票取
   `yeban_theory::{…}` 这一条，与 `verify/section_pure.rs:742` 的既有用法一致）。
3. **音高窗口是被保证的语义**：`melody.rs:27-28` 明文"候选音高恒在 `lower..=` `upper`
   之内，且其音级必须属于 `Scale`"。⇒ 音域参数不需要 MCP 侧再做一次逐音校验。
4. **窄音域的真实形状**（零依赖探测工程 `/tmp/yeban_probe_register`，C 自然小调
   `i-VI-VII-i` 4 小节、`metric_grid(COMMON, 4, 16)` ⇒ 网格 **64** 个 onset，种子 7）：

   | `(lower, upper)` | 结果 | 落点区间 | 不同音高数 |
   |---|---|---|---|
   | `(48, 84)`＝缺省 | `Ok` | 48..=84 | 19 |
   | `(60, 60)` | `Ok` | 60..=60 | 1 |
   | `(61, 61)` | **`Err`** `no feasible voicing…` | — | — |
   | `(48, 49)` | `Ok` | 48..=48 | 1 |
   | `(80, 84)` | `Ok` | 80..=84 | 3 |
   | `(48, 55)` | `Ok` | 48..=55 | 5 |
   | `(72, 79)` | `Ok` | 72..=79 | 5 |
   | `(120, 127)` | `Ok` | 120..=127 | 5 |

   三条读数决定本票的形状：**每条 `Ok` 的音高都落在请求的窗口内**（保证成立）；
   **onset 数恒为 64**（音域⛔ 不筛点、不改变时值 ⇒ 音符 id 集合与缺省跑逐条对齐）；
   **(61,61) 这类"窗口里没有音阶音"的组合由 theory 拒绝**，MCP 侧无法预先算出（那要抄
   一张"调式 × 音域"可行性表 —— 正是 `ADR-0036` D3 拒绝的那类第二写者）。
5. **既有校验的边界**：`MelodyConstraints::new`（`melody.rs:108-119`）只判
   `lower > upper` ⇒ `VoiceRangeInvalid`、`upper > 127` ⇒ `VoiceRangeOutOfMidi`，
   `max_leap` 刻意不校验（`0` 是合法语义）。MCP 层的入参界另有其数：
   `yeban_model::music::MIDI_PITCH_MAX = 127`（`music.rs:28`，与力度那条
   `MIDI_VELOCITY_MAX` 同一模块、同一口径）。

## 决定

### D1 两个可选整数键 = 旋律音高窗口的两端

`PlanOptions.melody_lower: Option<u64>` / `melody_upper: Option<u64>`，工具面
`yeban_propose_section` 新增两个可选参数 `melodyLower` / `melodyUpper`：

- **单位 = MIDI 音高值**（⛔ 不是"第几个八度"、⛔ 不是半音偏移量），取值 `0..=127`，
  界取自 `MIDI_PITCH_MAX`（⛔ 手抄 `127`）。
- 缺省：`melodyLower` ＝ [`MELODY_LOWER_BOUND`]（48）、`melodyUpper` ＝
  [`MELODY_UPPER_BOUND`]（84）—— 两个键都不给 ⇒ 逐字节维持既有输出。
- `melodyLower <= melodyUpper` 在**本层**判（`min > max` ⇒ `INVALID_PARAMETER_RANGE`，
  载荷点名 `field:"melodyLower"` 并回显两值 + `hint`）。
- **`max_leap` 不在本票**：它的单位是"相邻两音的半音距离"，与"音高窗口"不是同一个量
  （`ADR-0036` D2 的"一个词只有一种单位"），归入非范围 `N2`。

### D2 缺省兼容契约（`ADR-0016`）

两键都不给 ≡ 给 `{"melodyLower": 48, "melodyUpper": 84}`，整个 `SectionPlan` 逐字节相同。
本票⛔ 改任何缺省数值 —— `ADR-0028` D1 已经付过一次"低音声部音级变化 ⇒ 重录读数"的代价，
同一根轴不许在缺省路径上再动一次。

### D3 消费校验：给了必须被消费（比力度那一票更严）

`melodyLower` / `melodyUpper` 只被**旋律生成路径**读取（和声声部走 `ADR-0028` 的
`VoicingConstraints`，鼓⛔ 有固定 GM 音高），因此同一次调用必须同时满足：

1. `materialSource: "theory"`（外部素材路径的音高由调用方给出，本层不覆盖）；
2. `includeMelody: true`（键缺省 = `false` ⇒ 给了音域却没有旋律声部 ＝ 参数被静默忽略）。

任一不满足 ⇒ `INVALID_PARAMETER_RANGE` + `hint`（点名要补的键）。这与 `ADR-0037` D4 的
"⛔ 不要求鼓/旋律在场"是**同一规则的两种取值**，差别在事实本身：力度下档被和声声部
**恒**消费，音域窗口则没有任何"恒消费"的路径。

### D4 拒绝载荷⛔ 带 `reason` 键

沿用 `onsetsPerBar` / `velocityMin` 那一族的形状（`field` / `value` / `range` / `hint`），
**故意不写** `reason` 键 ⇒ `fault_vocabulary` 的黄金表（`GOLDEN_REASONS: [&str; 203]`）
与规范面摘要（207 个 reason / 906 行 / `4890ba25…`）逐字节不变。
"窗口里没有音阶音"这一类失败**不在本层预校验**，而是照 `ADR-0036` D3 的做法
调用 theory、把 `NoFeasibleVoicing` 原样转发进既有的 [`theory_invalid`] 形状
（`InvalidParameterRange` + `stylePreset` / `theoryStep` / `theoryError`）。

## 非范围（登记，⛔ 补齐）

- `N1` **旋律动机／主题变形**：theory 里⛔ 没有任何 motive 概念
  （`melody.rs:16` 逐字写着"旋律型 (motif)、句法、终止式一律不做"）⇒ `GAPS.md:139`
  的"无旋律动机"这一轴**不在本票**，且它是**产品范围决定**（要新建 theory 侧的动机模型），
  已登记待人类裁决，⛔ 自行发明。
- `N2` **跳进上界 `melodyMaxLeap`**：`MelodyConstraints.max_leap` 同样可以从入参给
  （theory 侧零成本），但它不是"音域"⇒ 不进本票，避免一根轴上挂两个单位。
- `N3` **逐声部音域表**：本票只管旋律一条声部；和声声部的音域是 `ADR-0028` 的
  `SECTION_THREE_VOICES`（仍是编译期常量，⛔ 入参化）。
- `N4` **`genre_melody*` 系列工具**：`yeban-theory` 的公开门面另有
  `genre_melody_for_with` 等带 `constraints` 入参的变体，本票⛔ 不新增工具、
  不改这些门面。

## 判据（`crates/yeban-mcp/tests/section_melody_range.rs`）

1. 两键都不给 ≡ 给 `48`/`84` ⇒ `SectionPlan` 逐字节相同，且缺省旋律音高全部落在 `48..=84`。
2. 给 `{60, 60}` / `{48, 55}` ⇒ **音符 id 集合、起点、时值、力度**与缺省跑逐条相同，
   只有**音高**改变且恒落在请求窗口内（把"音域⛔ 不筛点、⛔ 不改时值"钉住）。
3. 越界（`128` / 超出 `u8` 宽度）点名是哪个键；倒序 ⇒ `lower > upper` 拒绝并回显两值；
   `lower == upper` 合法（单音窗口）。
4. 耦合：非 `theory` 或缺 `includeMelody: true` ⇒ 拒绝并点名要补的键；正例：给了窗口
   就必须真的改变音高。
5. 不可行窗口 ⇒ `INVALID_PARAMETER_RANGE`，载荷里 `theoryStep` 指向旋律阶段
   （转发 theory 的判定，⛔ 本层抄可行性表）。
6. 两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON 的 `minimum`/`maximum`/`default`
   与领域层常量是同一组数字）。
