# ADR-0051 — 左侧资源栏「预置」分类：模型层**没有预置载体**（`preset` 零命中），而 theory 里躺着 182 条**只读**流派预设；要不要把那个库当作这个分类的事实源 ＝ 语义裁决，非机械活

- **状态**: **Proposed**（待人类批准；⛔ 集成者代签。本票**不改任何代码**，只把 `line/sidebar-library`
  落地时**故意留空**的那一格登记成可批可驳的裁决）
- **决定者**: 待人类裁决（集成者提出）
- **关联**: `ADR-0049`／`ADR-0050`（同族"登记现状、⛔ 自行扩面"的写法）、`ADR-0005`（界面⛔ 持有第二份权威）、
  `MODEL-AST-003`（持久化实体集合必须用 `BTreeMap` 保迭代顺序确定性）、`MODEL-ISO-001`（三层状态隔离）、
  `MUST-GATE-002`／`ARCH-DET-001`（改持久化 AST ⇒ L1 bit-exact 基准哈希要重录）、
  `UI-GRID-001`（顶栏 / 左栏结构）、任务单 `#133`（侧栏条目接真实投影那片）
- **实现落点**: 批 `D1` ⇒ **零代码**（现状即裁决）。批 `D2`／`D4` ⇒
  `crates/yeban-app/src/bridge.rs` 的 `sidebar_category_content` 分类 2 那一臂 ＋
  一个**当前不存在**的消费端（见"为什么⛔ 机械替换"第 2 条）。批 `D3` ⇒
  `crates/yeban-model`（新实体 + 新 `Op` 变体 + 迁移）＋ 该 crate 的幂等/对账判据族。

## 背景（先测后改，AGENTS.md §6）

**这两个数字在数什么**：① 单位＝**行**，对象＝`crates/yeban-model/src/` 全文（不计大小写）里出现
`preset` 这个子串的行；② 单位＝**条**，对象＝`crates/yeban-theory/src/genre.rs` 里
`pub static GENRES: &[GenreRule] = &[` 这个数组**字面量内部**的 `GenreRule {` 条目。

现读（2026-10-30，`main` = `b46c760c`）：

| 问题 | 读数 | 位置 / 命令口径 |
| --- | --- | --- |
| 模型层有「预置」这个概念吗 | **0 行** | `grep -rin preset crates/yeban-model/src \| wc -l` ⇒ `0` |
| `Op` 枚举里有预置相关的变体吗 | **0 行** | `grep -n Preset crates/yeban-model/src/ops.rs \| wc -l` ⇒ `0` |
| 有没有**只读**的、成规模的"预置样"库 | **182 条** | `genre.rs:537` 的 `GENRES` 数组（`:537–:2937`），条目数按 `GenreRule {` 字面量在数组体内计数 |
| 这个库有没有对外入口 | 有 | `GenreLibrary::all() -> &'static [GenreRule]`（`genre.rs:3011`）、`GenreLibrary::len()`（`:3017`）、`GenreLibrary::get(id)` |
| 每条有哪些可读字段 | — | `GenreRule`（`genre.rs:77`）：`id`（小写蛇形，**就是** MCP 的 `stylePreset` 键）／`name_zh`／`name_en`／`default_bpm_range`／`meter`／`typical_progressions` |
| 谁在消费它 | 控制面 | `yeban_propose_section` 的 `stylePreset: String`（规范 `MCP-TOOL-005`，`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:634`）→ `domain/section_build.rs:582` 的 `GenreLibrary::get(...)`，未知 ⇒ `STYLE_NOT_FOUND`（`section_build.rs:355,585`） |
| **界面**有没有消费流派的地方 | **没有** | `grep -rn 风格 crates/yeban-app/src crates/yeban-app/ui` 的命中**逐条读过**：全是 Slint 的**编译期界面风格**（`host.rs:10586`、`cli.rs:245`）与**命名风格**（`elements.rs:2180`）两类，⛔ 一处音乐流派语义 |

与本票直接相关的现状：`line/sidebar-library`（tip `2fee8bce`）把三个分类的数据源改成投影，其中
分类 2「预置」**如实返回空**，并把理由原样写到界面上（`bridge.rs` 的
`SIDEBAR_EMPTY_PRESETS_REASON = "模型层尚无预置载体（无 preset 实体、无对应 Op 变体），此分类暂无条目"`）。
⇒ 用户看到的不是"空列表 + 沉默"，而是一句**说清缺哪一层**的话。本票登记的就是这句话背后的裁决。

## 「预置」到底指什么，规范**没有**回答

唯一的出处是 `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md:84` 左栏那一行的
「预置模板」四个字 —— 而该文档于 2026-10-06 经负责人裁决**从 Normative 降级为 Advisory**
（`AGENTS.md` §1 第 3 条明文：*"参考设计，**非绑定**，不得直接作为硬性实现规范"*）。
⛔ 任何一处规范定义过：预置是**乐器 patch**（一层音色参数）、**采样/循环包**，还是
`GenreLibrary` 那种**流派模板**。三种读法会导出三种完全不同的落地（D2 vs D3 vs D5）。

⇒ 私自挑一种 ＝ 发明答案（`AGENTS.md` §5.7）。本票因此只登记，不代签。

## 五条可选路线（⛔ 本票不建议任何一条）

- **`D1` 维持现状**：分类 2 恒空 + 那句逐字理由。零代码、零风险，代价是界面上永远有一格
  "有标签、没有内容"的空位，而它**恰好**是三种读法里最像"能用但没做完"的那种。
- **`D2` 把 182 条流派投影成「预置」**：`sidebar_category_content(view, 2)` 直接读
  `GenreLibrary::all()` 的 `name_zh`（**零 IO、零扫描、天然只读且确定序**，比分类 0 那条
  库根扫描便宜得多）。⚠ 单独批 **D2 会造出假控件**，见下面第 2 条 —— 批它必须**同时**批消费端。
- **`D3` 模型层新增预置载体**：新实体 + 新 `Op` 变体 + `BTreeMap` 收录（`MODEL-AST-003`）
  + 归档迁移 + 幂等/跨架构对账判据全套。这是"预置 = 可保存可撤销的音色/内容包"这种读法的**正解**，
  也是本票里唯一能让「预置」这一格真正变成**可写功能**的路线。代价：改持久化 AST ⇒ L1 bit-exact
  基准哈希要重录（`MUST-GATE-002`），⛔ 不是集成者能顺手做完的量级。
- **`D4` ＝ `D2` + 改名**：分类标签改成「风格预设」，让**界面词与仓库里真的存在的东西对齐**
  （顺带消掉"预置到底指哪种"的歧义源头）。它与 `D2` 同批才有意义：改了名仍然要回答"点了会怎样"。
- **`D5` 删掉这个分类**：左栏两格（乐器 / 采样）。与 advisory 参考稿冲突，但**不违反任何绑定规范**
  —— 那条"预置模板"来自降级文档。若裁决认为三种读法都不该在这个版本落地，D5 是最诚实的收口。

## 为什么⛔ 机械替换（三条硬约束，全部现读）

1. **"预置"的语义未被任何绑定规范定义**（上一节）。D2 / D3 / D5 分别对应三种读法，
   选错一种不是"实现偏保守"，而是**替人类做了一个没人批过的产品决定**。
2. **列表必须配一个真的消费端，否则就是假控件。** 本仓库明文纪律：*"把动作吞掉却什么都不做，
   比不处理更糟"*（`crates/yeban-app/src/main.rs` 的 `wire_callbacks` 文档）。今天 UI 侧
   **一个**消费流派的 `UiAction` 都没有（背景表第 7 行）。而 `GenreLibrary` 现在唯一的消费者
   `yeban_propose_section` 落的是**隔离分支** `ai/proposal-{ulid}`
   （`crates/yeban-mcp/src/domain/proposal.rs` 模块文档：提案⛔ 直接改工程，只有
   `yeban_merge_proposal` 才并回主分支）⇒ "在侧栏点一下流派"等于"提交一条待审查的 AI 提案"，
   这是**产品语义**，⛔ 是代码细节。
3. **`D3` 触碰持久化状态与确定性基准。** 新实体要进 `YebanProjectV1`、新 `Op` 要进归约与撤销链，
   还要动 `MODEL-ISO-001` 的三层划界（预置显然是**持久层**，⛔ 视图态）；改完 L1 摘要与 golden
   基准全部要重录。按 §5 的执行环境纪律，这类改动只能整票批准、按切片推进，⛔ 与侧栏投影混在一片。

## 闭环与判据（无论批哪条都要落）

- 现有钉住现状的两条机械读数：`crates/yeban-app/src/bridge.rs` 的
  `SIDEBAR_EMPTY_PRESETS_REASON`（空态那句逐字理由）＋ 分类 2 恒返回空 items 的
  `sidebar_category_content`。批 `D1` ⇒ 两者不动，本票按"裁决＝维持"关闭；
  批 `D2`／`D4` ⇒ `sidebar_category_content(view, 2).items` 的**条数判据**要从"0 条"改成
  "`GenreLibrary::len()` 条"，且判据必须**自己调** `GenreLibrary::all()` 取数（⛔ 抄一个 182 进测试）；
  批 `D3` ⇒ 判据改口径 + 该 crate 的持久化幂等/对账族要覆盖新 `Op`。
- ⚠ 无论哪条：**消费端没落地之前⛔ 允许 `D2` 单独批**，否则本票会亲手把一句诚实的空态文案
  换成一个可点而无效的列表 —— 那正是 `line/honesty` 系列判据在防的东西。

## 影响面

- `D1`：零代码，只有本票与账本两行。
- `D2`／`D4`：`crates/yeban-app`（`bridge.rs` 一臂 + `ui/sidebar.slint` 的分类标签 + 判据两处）；
  ⛔ 触引擎 crate，⛔ 动持久层 ⇒ 风险最低，但**功能上仍是一半**（没有消费端）。
- `D3`：`crates/yeban-model` 为主，连带 `yeban-mcp` 的领域面（要有一条读写预置的工具才自洽）、
  迁移与基准重录 —— 建议**单独开票**，本票只负责先定语义。
- `D5`：`crates/yeban-app`（`SIDEBAR_CATEGORIES` 由 3 项改 2 项 + `ui/sidebar.slint` 分类标签 +
  判据与注册表同步），⛔ 留"有格无名"的空位。
