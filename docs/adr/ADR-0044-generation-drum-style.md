# ADR-0044 — 生成鼓的底鼓口径：把 `GenreRule::drum_pattern` 里最后一个写死的实参 `self.drum_style` 开放为 `drumStyle`（缺省 = 不覆盖流派登记值 ⇒ 契约⛔ 写 `default`；界 = `DrumStyle::ALL` 的公开名字，⛔ 本地字符串表）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定，与 `ADR-0041`／`ADR-0043` 同族同口径）**
- **决定者**: 集成者（Agent，参考通行配器里"four-on-the-floor = 每拍一击的底鼓"的口径自行裁决）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ADR-0016`（缺省逐字节兼容契约）、
  `ADR-0017`（鼓声部 opt-in 与 `materialSource:"theory"` 前置）、`ADR-0028`（`DrumStyle` 这一维**落地**的票：
  枚举、流派登记与 `Metric` 缺省都出自它，本票只开放**调用方**那一侧）、
  `ADR-0041`（`swingPermille`：同一条调用的第三个实参）、`ADR-0043`（`drumBackbeat`：第四个实参；
  本票是**第五个**，也是 `drum_pattern` 参数表里最后一个不可覆盖的）、
  `MCP-TOOL-005` 的 `drumDensity`（⚠ 同为字符串枚举参数，但 `DrumDensity` 是**本层自己**的类型、
  theory 侧⛔ 它 ⇒ 它的本地字面量是唯一写者；`DrumStyle` 的名字在 theory，本票因此**必须**从 `ALL`/`name()` 现读，见 D2）、`ARCH-DET-001`
- **缺口号**: ⭐ **暂记 `G8l`**（纪律第 21 条：`G8j` ＝ `harmonyVoices`／和声音域**两张**票、
  `G8k` ＝ `drumBackbeat`，均现读 `docs/DEVELOPMENT_LEDGER.md`；⛔ 凭记忆递增 ⇒ 开票时再 `grep` 一次缺口表）
- **实现落点**: `crates/yeban-mcp/src/domain/section_build.rs`（`PlanOptions`、`DrumRulers`、
  `theory_drum_pattern`、`drum_contents`、`plan_with_options` 的消费校验）、
  `crates/yeban-mcp/src/domain/mod.rs`（`arg_str` 读取）、`crates/yeban-mcp/src/tools.rs`、
  `schemas/mcp-tools.schema.json`；`crates/yeban-theory`（**只读其既有公开面，本票不改它**）
- **日期**: 2026-10-28

## 背景（先测后改，AGENTS.md §6）

**这一票在数什么**：`drum_style` 取两档时，鼓组型里的**击点条数**（单位＝条）、其中**底鼓条数**（单位＝条）、
底鼓的**起点 tick 集合**（单位＝tick），以及**非底鼓**击点的 `(GM 键, 起点 tick, 重音)` **有序表**（单位＝条）。
测法＝仓库外一次性探针 `rustc --edition 2024 -O --extern yeban_theory=target/debug/deps/libyeban_theory-*.rlib`
（⛔ 重依赖、⛔ 不提交；探针 `/tmp/probe_drum_style.rs`，夹具 = MCP 生成路径的实际形状：4 小节、
`grouping = None`、`backbeat = default_backbeat(meter)`、`permille ∈ {None, 580}`、`onsets_per_bar ∈ {4, 8, 16}`）。

`GenreRule::drum_pattern` 的整条参数表里，前四格已被三张票接成入参（`ADR-0036` 的 `onsets_per_bar`、
`ADR-0041` 的 `permille`、`ADR-0043` 的 `backbeat`、`ADR-0028` 的 `density` 筛法），只剩**最后**一个实参
`self.drum_style` 仍不可覆盖：`swung_styled_drum_pattern(meter, bars, onsets, self.swing_permille()?, None,
default_backbeat(meter), self.drum_style)`（`crates/yeban-theory/src/genre.rs:515`）。MCP 的两个调用点
（`section_build.rs` 的 `Some` 分支与 `Ok(None)` 兜底分支）都直接写 `rule.drum_style`
⇒ 与 `G8i`／`G8k` **完全同形**的第三处"流派登记值不可覆盖"。

⛔ **本票不在任何已开票的非范围里**（现读：`grep -rn "drum_style\|DrumStyle" docs/adr/` 只命中
`ADR-0041:26` 那一句**代码引用**，⛔ 一条非范围登记）；`ADR-0041` 的 `N3` 登记的是 `grouping`，与 `style` 是另一根轴。

实测读数（五条，第 3、4 条**推翻**"照抄 `ADR-0043` 判据"的写法）：

1. **登记分布**（单位＝条流派）：`GENRES` 182 条 ⇒ `Metric` **169** 条、`FourOnTheFloor` **13** 条
   （`disco,boogie,house,deep_house,tech_house,progressive_house,garage_house,techno,minimal_techno,trance,psytrance,hardstyle,amapiano`）
   ⇒ 缺省**不是一个数字**，契约里⛔ 写 `default`（与 `ADR-0041` D5 同支，⛔ 与 `ADR-0040` 那一支不同）。
   夹具 `synthwave` 属 `Metric` 那一支 ⇒ 判据 1 的"缺席 ≡ 登记值"是**实测等价**而⛔ 定义。
2. **本键只动底鼓**：`182 流派 × 3 档 onsets = 546` 组里可测的 **518** 组，**非底鼓**击点的
   `(GM 键, tick, 重音)` 表在两档之间**逐条相同**（不等 = **0** 组）⇒ 判据 2 能把"只改底鼓"钉成
   "别的击点逐条不变"，这是 `ADR-0043` 那条判据的**加强**（那一个键会同时加／筛军鼓，⛔ 有此不变量）。
3. **底鼓集合恒为超集**：`FourOnTheFloor` 的底鼓 tick 集合 ⊇ `Metric` 的（非超集 = **0** 组），但
   ⚠ **"严格更多"⛔ 全局成立**：518 组里有 **6** 组两档**完全相同**
   （`jig`／`irish_trad`／`celtic` 三条 **6/8** 流派 × `onsets ∈ {4, 8}` ⇒ 复合二拍里两个拍起点恒是组起点，
   两档打在同一个集合上）。4/4 夹具上实测 `8 → 16` 条（`onsets` 4/8/16 三档都是这个倍数）。
   ⇒ 判据⛔ 写"给 `four-on-the-floor` 必然更多底鼓"为全平台断言；必须**夹具内**钉"更多"、**6/8 上**钉"逐字节相同"。
4. **与摇摆正交**（这条能挡住"接错实参"那一类缺陷）：`permille = None` 与 `Some(580)` 两档下，
   两档 `drum_style` 的**底鼓 tick 集合各自完全相同**（新增的底鼓恒落在**拍起点**，而摇摆只推移每对格位里
   的**第二个** onset）⇒ 判据可钉"换 style ⛔ 改 swing 的量、换 swing ⛔ 改底鼓落点"。
5. **可行性⛔ 受本键影响**：28 组 `Err(ProgressionTooDense)`（正是 `ADR-0036` 登记的那 28 条非 4/4 流派在
   `onsets=16` 上的行为）在两档下报**同一个错**、`Ok(None)` 的分布也一致 ⇒ "该口径在这拍号/格位下产不产得出鼓点"
   仍由 theory 判定并原样转发（D6），本层⛔ 判、⛔ 抄「拍号 × 口径」表。

`DrumStyle` 的**公开面**足以当界：`pub const ALL: [Self; 2]`（`drum.rs:211`）＋
`pub const fn name(self) -> &'static str`（`:215`，返回 `"metric"` / `"four-on-the-floor"`）
⇒ 合法值集合与显示名**只有 theory 一个写者**。

## 决定

- **D1 — 新增可选键 `drumStyle: string`，界 = `DrumStyle::ALL` 的公开名字，缺省 = 不覆盖流派登记值**
  ⇒ 键缺席必须**逐字节**产出同一份 `SectionPlan`（`ADR-0016`）。语义 = "底鼓落在**哪些**拍的起点"
  （`Metric` = 组起点；`FourOnTheFloor` = 每一个拍起点），⛔ 军鼓落点（那是 `ADR-0043` 的 `drumBackbeat`）、
  ⛔ 每小节几个 onset（`ADR-0036`）、⛔ 筛不筛非重音（`ADR-0028` 的 `drumDensity`）—— 四根轴⛔ 折成一根。
  ⚠ 契约里⛔ `default`（读数 1）；`enum` **可以**登记（那一支⛔ 随拍号移动，与 `ADR-0043` 的 `maximum` 不同）。
- **D2 — 解析器⛔ 自带字面量**：`DrumStyle::ALL.iter().copied().find(|style| style.name() == value)`，
  未知取值 ⇒ `INVALID_PARAMETER_RANGE`（载荷 `field` + `value` + `accepted`，`accepted` 从 `ALL` × `name()`
  **现场生成**）。⚠ 这与既有的 `DrumDensity::parse`（`section_build.rs:1130` 一段，`match` 里手写
  `"normal"`/`"sparse"`）**不是同一族**：`DrumDensity` 是**本层自己的**枚举（theory 侧⛔ 这个类型），
  那份字面量就是它的唯一写者，⛔ 抄；而 `DrumStyle` 的**名字在 theory**（`ALL` ＋ `name()` 公开）⇒
  本票若再手写字面量，theory 改名时会静默变成"契约说能、本层拒"。故本票的判据 6 直接拿 `ALL` 对契约的
  `enum` 核，⛔ 出现第二个写者。
- **D3 — 消费校验取 `ADR-0041`／`ADR-0043` 那一支**：`materialSource:"theory"` **且** `includeDrums:true`，
  两个报文分别点名 `materialSource` / `includeDrums`，⛔ 与旋律/和声那两支折成同一个 `is_some()`；
  ⚠ **反钉**：`includeMelody:false` 时必须放行（底鼓口径只被鼓轨消费）。
- **D4 — `DrumRulers` 加**第 6 个字段 `style: Option<DrumStyle>`（`ADR-0041` D4 建的结构、`ADR-0043` D4 加到 5 个）：
  现在字段里同类型的整数有 `density`/`onsets_per_bar`/`ladder`/`swing`/`backbeat` 三对以上，
  ⛔ 再走"加第 10 个位置参数"—— 命名实参是唯一能让"传错顺序"编译失败的形状。
  ⚠ 类型存**已解析的 `Option<DrumStyle>`** 而⛔ `Option<&str>`：解析只发生在 D2 那一条表达式里，
  结构体里再解析一次就是第二个写者。
- **D5 — 接线只换那一个实参，并把"薄封装那一支"的条件**扩成三键：
  `(swing, backbeat, style) = (None, None, None)` 时**原样**调用 `rule.drum_pattern`（缺省兼容是**结构**保证）；
  任一为 `Some` 时走 `swung_styled_drum_pattern(meter, bars, onsets, swing_resolved, None,
  backbeat_resolved, style.unwrap_or(rule.drum_style))` —— `unwrap_or` 里那一只读的就是薄封装内部读的同一个字段，
  因此"只给 `drumStyle`"⛔ 顺带改掉摇摆或军鼓落点。`Ok(None)` 兜底分支取**同一组**解算值，⛔ 分叉。
- **D6 — 可行性判定⛔ 上移**：读数 5 要求两档 `drum_style` 的失败面**逐字相同** ⇒ 仍由 theory 决定并经既有
  `theory_grid_invalid(..., "swung_styled_drum_pattern", ...)` 原样转发（⛔ 抄表、⛔ 夹紧、⛔ 静默换档）。
- **D7 — ⛔ 新增故障词**：拒绝载荷⛔ `reason` 键；`Some` 分支的 `theoryStep` **复用**
  `"swung_styled_drum_pattern"` ⇒ `fault_vocabulary` 词表与规范面摘要不变（与 `ADR-0041` D7／`ADR-0043` D7 同一条）。

## 非范围（登记，⛔ 补齐）

- `N1` **给每条流派登记更完整的鼓点型数据**（`pending 3` 的数据那一半：`ADR-0028` 只登记了底鼓落点这一维，
  军鼓位置那一半已由 `ADR-0043` 的 `N1` 登记为欠）⇒ 本票只开放**调用方**这一侧，⛔ 给 182 条流派改登记值。
- `N2` **`grouping`（复合拍拍内分组）**：`ADR-0041` 的 `N3`，是**第 5 个**实参、本票⛔ 碰。
  ⚠ 它是这条参数表里最后一个未开放的入参候选，但它是**数组**入参（`BeatGrouping::new(meter, &[u8])`），
  形状与本票不同族。
- `N3` **底鼓的音色与力度**：GM 键 36 与 `accent` 判据不变（读数 2 的实测面只有落点与条数）。
- `N4` **`DrumVoice` 级的开关**（只留踩镲／关掉底鼓）：`ADR-0043` 的 `N2` 同一支，本票⛔ 能表达"不打底鼓"
  （`Metric` 是"组起点打"，⛔ "不打"）。
- `N5` **`drumDensity:"sparse"` × `drumStyle` 的组合语义**：探针只比了**非底鼓**击点的 `accent` 表与
  底鼓 tick 集合，**没**量"新增的那批拍起点底鼓在 sparse 档下会不会被筛掉"（`accent` 读数在新击点上
  是否恒真⛔ 已知）⇒ 判据⛔ 对组合给断言，两档都只在**缺省 density** 下钉；组合语义待实测另计一张。

## 判据（`crates/yeban-mcp/tests/section_drum_style.rs`，6 条）

1. 键缺席 ≡ 显式给**夹具登记值**（`synthwave` ⇒ `Metric`／`"metric"`）⇒ `SectionPlan` **逐字节**相同
   （`ADR-0016`；读数 1 的实测等价，实现上走薄封装那一支）。顺带钉夹具形状：底鼓与踩镲/军鼓都在册、
   ⛔ 一条流派登记值写进本层常量。
2. 给 `"four-on-the-floor"`（4/4 夹具）⇒ ①**非底鼓**击点的 `(GM 键, 起点, 时值, 重音)` 逐条与缺省跑相同
   （读数 2）；②底鼓 tick 集合是缺省的**超集**且**严格更多**（实测 `8 → 16` 条）且新增的那批**恒在拍起点**
   （`tick % ticks_per_beat == 0`，读数 4）；③和声各声部与旋律与缺省跑**逐字节**相同（D3 消费面）；
   ④同一份 `drumStyle` 在 `swingPermille = 500` 与 `580` 两档下的底鼓 tick 集合**相同**（正交读数 ⇒ 挡"接错实参"）。
3. 未知取值（`"four"` / `""` / `"FOUR-ON-THE-FLOOR"` 大小写不同形）⇒ `INVALID_PARAMETER_RANGE`，载荷点名
   `field:"drumStyle"` + `value` + `accepted` 且 `accepted` **恰等于** `DrumStyle::ALL × name()` 现读的两项；
   两个合法值都必须放行（⛔ 夹紧、⛔ 静默降级成 `Metric`）。
4. 耦合：脱离 `materialSource:"theory"` ⇒ 拒且 `hint` 点名 `materialSource`；`includeDrums:false` ⇒ 拒且
   `hint` 点名 `includeDrums`；⚠ **反钉**：`includeMelody:false` 时必须放行。
5. 复合拍那一支（读数 3 的**反例**）：6/8 的夹具流派（`jig`／`irish_trad`／`celtic` 任一条）上两档
   `drum_style` 的 `SectionPlan` **逐字节相同** ⇒ 钉住"⛔ 把'严格更多'写成全平台断言"，
   同时钉"本层⛔ 另判可行性"（同一夹具在 `onsets=16` 上的 `ProgressionTooDense` 两档**同报文**，读数 5）。
6. 两个声明面同源（`tools.rs` 的 `ParamSpec` ⇄ 契约 JSON）：`json_type == "string"`、⛔ `required`；
   契约 property 的 `enum` 恰为现读 `DrumStyle::ALL` 的两个 `name()`（⛔ 两份数字/两份字符串表）、
   ⭐ 且**⛔ 存在 `default` 键**（D1：169/13 的登记分布 ⇒ 一个固定字符串会把两支说成同一支）；
   `doc` 必须含 `ADR-0044` 与"底鼓"。
