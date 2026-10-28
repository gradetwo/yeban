# ADR-0046 — 鼓网格的拍内分组：把 `swung_styled_drum_pattern` 里写死的 `grouping = None` 开放为 `beatGrouping`（缺省 = 不覆盖拍号自身的度量层级 ⇒ 契约⛔ 写 `default`、⛔ 写 `maximum`、⛔ 写 `maxItems`）

- 状态：**Accepted（集成者依 2026-10-25 人类常设授权裁定；暂记缺口 `G8n`）**
- 日期：2026-10-28
- 关联：`ADR-0041`（N3 明写"把 `grouping` 入参化"是下一条轴）、`ADR-0036`（`onsetsPerBar` 从拍号导出）、
  `ADR-0043`/`ADR-0044`（同族鼓线两键：落点与底鼓口径）、`ADR-0016`（键缺席逐字节兼容）、
  `AGENTS.md` §6（先测后改）与 §6.3（"绿"的防伪）、规则第 19 条（先 grep 消费点）

## 背景（先测后改，AGENTS.md §6）

**本节在数什么**：① 那个实参在 theory 里是什么（单位 = 处，`grep` 现读）；② 换它会改动鼓轨的哪一维
（单位 = 每小节的击点条数 / `(拍序号)` 落点集合 / 重音档条数）；③ 合法形状随什么移动（单位 = 拍号）。
全部现读，⛔ 凭记忆。

1. **两处写死，且都是 `None`**：`crates/yeban-theory/src/genre.rs:525` 的 `GenreRule::drum_pattern`
   （薄封装）把第 5 个实参固定为 `None`；`crates/yeban-mcp/src/domain/section_build.rs` 的
   `theory_drum_pattern` 在显式路径与兜底路径**各自**再写一次 `None`。理论侧的机制早就存在 ——
   `swung_styled_drum_pattern(meter, bars, onsets, permille, grouping, backbeat, style)` 的第 5 实参
   与 `rhythm::BeatGrouping` 全套（构造、`is_group_start`、`metric_weight_grouped`）都已公开且有测试；
   缺的只是**调用方入口**。
2. **它改的不是网格，是分派与重量**：`grouping` 一变，`GridReading { weight, group_start }` 两个读数
   一起变（`drum.rs` 的 `place_grouped_voices`：`weight = max(metric_weight_in, metric_weight_grouped)`，
   `group_start = grouping.is_group_start(beat)`）⇒ **底鼓（Metric 口径）与吊镲的落点**、以及
   `hit.accent`（`= weight >= STRONG_BEAT_WEIGHT`）跟着变；`accent` 是**格级**读数 ⇒ 同格的所有鼓件共享它。
   ⚠ 与 `ADR-0044`（只动底鼓）与 `ADR-0043`（动军鼓落点）都不同轴。
3. **实测读数**（本机一次性探针，`yeban-theory` 零依赖、构造上允许；1 小节、全格点、`DrumStyle::Metric`、
   缺省反拍；跑完即删）：

   | 拍号 | 分组 | 总击点 | 底鼓拍 | 吊镲拍 | 军鼓拍（重音） | 踩镲重音格 | 重音档合计 |
   | :--- | :--- | ---: | :--- | :--- | :--- | :--- | ---: |
   | 4/4 | 缺省 | 22 | `{0,2}` | `{0,2}` | `{0,2}`（`[1,1]`） | 0、8 | **8** |
   | 4/4 | `[4]` | 20 | `{0}` | `{0}` | `{0,2}`（`[1,1]`） | 0、8 | 6 |
   | 4/4 | `[2,2]` | 22 | `{0,2}` | `{0,2}` | `{0,2}`（`[1,1]`） | 0、8 | 8（= 缺省，逐字节） |
   | 4/4 | `[1,1,1,1]` | 26 | `{0,1,2,3}` | `{0,1,2,3}` | `{0,2}`（`[1,1]`） | 0、4、8、12 | **14** |
   | 3/4 | 缺省 | 16 | `{0}` | `{0}` | `{0,2}`（`[1,0]`） | 0 | 4 |
   | 3/4 | `[3]` | 16 | `{0}` | `{0}` | `{0,2}`（`[1,0]`） | 0 | 4（= 缺省，逐字节） |
   | 3/4 | `[1,1,1]` | 20 | `{0,1,2}` | `{0,1,2}` | `{0,2}`（`[1,1]`） | 0、4、8 | 11 |
   | 6/8 | 缺省 | 18 | `{0,1}` | `{0,1}` | `{0,1}` | 0、6 | 8 |
   | 6/8 | `[1,1]` | 18 | `{0,1}` | `{0,1}` | `{0,1}` | 0、6 | 8（= 缺省，逐字节） |
   | 6/8 | `[2]` | 16 | `{0}` | `{0}` | `{0,1}` | 0、6 | 6 |
   | 5/4 | 缺省 / `[5]` | 24 | `{0}` | `{0}` | `{0,2}` | 0 | 4（两条逐字节） |
   | 5/4 | `[3,2]` | 26 | `{0,3}` | `{0,3}` | `{0,2}` | 0、12 | 8 |
   | 5/4 | `[2,3]` | 26 | `{0,2}` | `{0,2}` | `{0,2}` | 0、8 | 7（⚠ 与 `[3,2]` **不同**：落点与重音都随切法移动） |
   | 7/8 | 缺省 / `[7]` | 18 | `{0}` | `{0}` | `{0,2}` | — | 4 |
   | 7/8 | `[2,2,3]` | 22 | `{0,2,4}` | `{0,2,4}` | `{0,2}` | — | 11 |

   三条从表里读出来的硬事实：① **踩镲的条数与起点恒不变**（每小节 = 格点数，本键⛔ 改网格）；
   ② **重音档条数会变**（4/4 8→14/小节）⇒ 只比网格会漏；
   ③ 缺省 ≡ 该拍号"复现分组"（4/4 `[2,2]`、3/4 `[3]`、6/8 `[1,1]`、5/4 `[5]`、7/8 `[7]`）**逐字节**，
   换一条才改读数 —— 这正是 `rhythm.rs` 的 `BeatGrouping` 文档承诺的那条关系，本票的兼容性判据靠它。
   ⚠ `rhythm.rs` 明写 **8/8 没有复现分组**（内置口径只在半小节给强拍，任何分组都把每组起点判成强拍）
   ⇒ 在 8/8 上"键缺席 ≡ 给某条"无 X 可选；本票判据只用 4/4 与 3/4，⛔ 声称跨拍号通用。
4. **军鼓"不变"是 4/4 读数、⛔ 全局承诺**：3/4 给 `[1,1,1]` 时军鼓第二击的重音从 0 变 1
   （读数 3 表：`[1,0]` ↔ `[1,1]`），因为 `accent` 是格级读数。判据只在 4/4 夹架上钉军鼓全等，
   并把这条差写进文件头的免责声明。

## 决定

- **D1 — 新增可选键 `beatGrouping: array<integer>`，缺省 = 不覆盖拍号自身的度量层级** ⇒ 键缺席必须
  **逐字节**产出同一份 `SectionPlan`（`ADR-0016`）。单位 = **拍数序列**（逐项 = 该组的拍数；
  ⛔ tick、⛔ "每小节打几下"、⛔ 小节号）。契约里 ⛔ `default`（缺省那条随拍号移动，写死任何一个数都是撒谎）。
- **D2 — 合法性唯一判者是 theory 的 `BeatGrouping::new`**：本层只做 `u64 → u8` 的**类型面**转换
  （`> 255` 无法表达），语义面（非空、逐项 `>= 1`、总和 == `felt_beats_per_bar(meter)`）由
  `BeatGrouping::new` 判、本层⛔ 重算。拒绝（`INVALID_PARAMETER_RANGE`，`field` + `value`，
  语义面另加 `beatsPerBar`，⛔ `reason`、⛔ `range`）。⛔ 静默夹紧、⛔ 降级成缺省。
  也要**先于** theory 判掉：`BeatGrouping::new` 返回 `Option`（⛔ 错误），放下去只会落到兜底分支
  那句不点名本键的"拍号推不出鼓组型"。
- **D3 — 消费耦合与 `swingPermille`/`drumBackbeat`/`drumStyle` 同一支**：要求
  `materialSource:"theory"` **且** `includeDrums:true`，两个报文分别点名 `materialSource` / `includeDrums`；
  ⚠ **反钉**：`includeMelody:false` 时必须放行（本键⛔ 读旋律）。
- **D4 — `DrumRulers` 加第 7 个字段 `grouping: Option<&'a [u64]>`**（前六个：`density`/`onsets_per_bar`/
  `ladder`/`swing`/`backbeat`/`style`），结构因此带生命周期参数 `DrumRulers<'a>`；⛔ 再走"加第 8 个
  位置参数"（`theory_drum_pattern` 现在 4 个参数，两处都仍 < 7）。字段存**原始读数**而⛔ `BeatGrouping`：
  引用的 `&[u8]` 无处安放（入参是 `&[u64]`），且"没给"要与"给了但非法"分开 —— 前者放行、后者拒绝。
- **D5 — 契约的形状面**：`type: array`、`items: {type: integer, minimum: 1}`、`minItems: 1`；
  ⛔ 逐项 `maximum`、⛔ 数组级 `maxItems`、⛔ `default` —— 三者的上界都是**该拍号的拍数**
  （4/4 = 4、3/4 = 3、6/8 = 2），写死一个数字会对窄拍号报出"契约说能、本层拒"的矛盾。
  `items` 由 `tools.rs` 的 `beat_grouping_items()` 生成 ⇒ 广播面（`tools/list` 的 `inputSchema`）
  与契约文件同源，⛔ 手抄第二份。
- **D6 — 接线只换那一个实参**：`theory_drum_pattern` 的两处 `None` 换成 `grouping`；薄封装那一支
  的**触发条件**从三元组扩成四元组 `(swing, backbeat, style, grouping)`，四者全 `None` 才走
  `GenreRule::drum_pattern`（= 接线前的路径）。若有人只把 `grouping` 的实参换掉而忘记扩元组，
  缺省路径会改走显式分支 ⇒ 判据 1 红（等价性是实测，⛔ 定义）。
- **D7 — ⛔ 新增故障词**：复用既有 `InvalidParameterRange` + `field`/`value`/`beatsPerBar`/`hint`
  载荷形状，步骤名复用已有 `swung_styled_drum_pattern` ⇒ `fault_vocabulary` 词表与规范面摘要不变。

## 非范围（登记，⛔ 补齐）

- `N1` **8/8 的复现分组**：`rhythm.rs` 明写没有（读数 3 末尾）。要么改内置口径，要么接受 8/8 上
  "给任何分组都改读数" —— 那是拍号语义的裁决，⛔ 本票发明。
- `N2` **把分组写进流派登记表**：`rhythm.rs` 明写"分组是作品或地区的选择，不是拍号写出来的"，
  ⛔ 替任何流派猜一个（与 `onset`/`drumStyle` 同源理由）。
- `N3` **分组对非底鼓/吊镲鼓件的约束**：`accent` 是格级读数 ⇒ 3/4 的 `[1,1,1]` 会顺带改军鼓力度档
  （读数 4）。要让"分组只动底鼓与吊镲"成立就得拆 `accent` 的语义 —— 那是 `drum.rs` 的读数模型，
  ⛔ 本票顺手改。
- `N4` **旋律动机**：`GAPS.md:139` 五轴里唯一未交付那一轴的半片，仍卡在 `§0.6.3` 的人类裁决。
- `N5` **分组粒度 = 拍（`felt_beats_per_bar`），⛔ 细分到复合拍内部**：`rhythm.rs` 的
  `BeatGrouping` 只切**拍**，而 6/8 的 `felt_beats_per_bar` = `numerator / 3` = 2 ⇒ `[2]` 与 `[1,1]`
  是**全部**合法形状，`3+3`（两个复合拍）与 `2+2+2`（三个二拍）在 felt=2 的网格上**不可区分**。
  要表达后者得先让 `rhythm` 的分组单位下沉到八分音符 —— 那是拍号语义的裁决（与 `N1` 同族），
  ⛔ 本票改。

## 判据（`crates/yeban-mcp/tests/section_drum_grouping.rs`，6 条）

1. `absent_grouping_is_byte_identical_to_the_derived_split` —— 键缺席 ≡ 给 `[2,2]`（4/4 的复现分组）
   ⇒ 整份 `SectionPlan` **逐字节**相同（D6 的结构等价性）；顺带钉夹具形状（三件鼓件都在册、和声声部在场）。
2. `the_grouping_moves_the_kick_and_the_accents_not_the_grid` —— `[1,1,1,1]` ⇒ 踩镲 `(起点, 时值, 音高)`
   逐条不变、军鼓形状全等（⚠ 4/4 读数）、底鼓落点 `{0,2}` → `{0,1,2,3}`、重音档 8→14 条/小节、
   和声与旋律逐字节不变；并断言夹具**真的**用到下档力度（否则重音读数无从对照）。
3. `illegal_groupings_are_rejected_naming_the_key` —— 空表 / `[0]` / `[3]` / `[3,2]` / `[5]` / `[1,1,1]`
   ⇒ 拒绝并带 `beatsPerBar: 4`；`[300]` ⇒ 类型面拒绝（⛔ `beatsPerBar`）；`[4]`/`[2,2]`/`[1,1,1,1]` 放行。
4. `the_grouping_key_requires_the_theory_source_and_the_drum_part` —— 脱离 theory 与
   `includeDrums:false` 各一条拒绝（`hint` 点名对应键）；⚠ 反钉 `includeMelody:false` 必须放行。
5. `the_legal_shape_moves_with_the_meter` —— 3/4 夹具上 `[3,2]`/`[4]` 被拒且 `beatsPerBar: 3`、
   `[2,1]`/`[3]`/`[1,1,1]` 放行；两个拍号上键缺席都放行；**并在 3/4 上再钉一次**"缺省 ≡ `[3]`"
   （⛔ 把 `[2,2]` 当跨拍号常量）。
6. `drum_grouping_is_declared_on_both_param_surfaces_without_default_or_maximum` —— `tools.rs` 的
   `array_param("beatGrouping", …)` 与契约 JSON 的 `properties.beatGrouping` 同时在场，
   `items.type = integer`、`items.minimum = 1`、`minItems = 1`，且 ⛔ `default`/`maxItems`/`items.maximum`。

### 待 CI 回答的两件事 —— **已由 CI 回答（2026-10-28，两条都成立）**

- **① MCP 路径下"8→14 条/小节"逐值成立。** 判据 2 在 `yeban-mcp` 里**不是** theory 直调：它走
  `plan_with_options`（`onsets_per_bar` 由流派格点导出、力度由 `ADR-0037` 的阶梯给出），断言
  `accent_count(base) == 8 * bars` 与 `accent_count(moved) == 14 * bars`。线 run **`40674443906`**
  （head `1ac8816f`，`rust (yeban-mcp)` 785 s 真跑 / 119 个 `test result:` / passed 2,352 / `FAILED` 0）
  里该判据逐名 `... ok` ⇒ 读数与 theory 探针一致，⛔ 需要按夹具改写条数。
- **② `waltz`（3/4）夹具在该 `progression` 下规划成功。** 判据 5 在 `waltz` 上先跑通 `plan(...)`
  才谈断言（`[3,2]`/`[4]` 被拒且 `beatsPerBar: 3`；`[2,1]`/`[3]`/`[1,1,1]` 放行；缺省 ≡ `[3]`），
  同一 run 里 `... ok` ⇒ 夹具可用，⛔ 需要换 3/4 夹具、⛔ 放宽主张。
- ⚠ 口径（第 22 条）：两条读数都来自**判据名 ＋ 文件路径**（`Running tests/section_drum_grouping.rs`
  在该日志里出现 **2** 次 — 见 `tests/section_drum_grouping.rs`）的交叉核对，⛔ 仅凭整体 `success`。
- ⏳ 主干侧替代判决：merge **`b42f4cc0`** 的 run `40679452708`（碰 `crates/**` ＋ `schemas/**`
  ⇒ `workspace_wide: true` ⇒ 全量腿必真跑）。
