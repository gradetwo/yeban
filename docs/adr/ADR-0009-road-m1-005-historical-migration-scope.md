# ADR-0009 — `ROAD-M1-005`（历史数据迁移器）的范围：条目本身是②，它预设的输入是③

- **状态**: **Proposed（待人类裁决）**
- **决定者**: 集成者（Agent）提出；口径归负责人
- **关联**: `ROAD-M1-005`（Normative 路线图，`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:194-195`）；
  `ADR-0001` **D3**（`schema_version` 字面值）与 **D43**（1.0.0 之前没有历史包袱，负责人 2026-10-04 授权）；
  `docs/ledger/human-decisions.md` 的 `HD-23`（版本字段只用于拒绝不匹配）与 `HD-56`（最高规则）；
  `crates/yeban-model/src/project.rs`（`SCHEMA_VERSION` / `check_readable`）；
  `crates/yeban-model/tests/no_compat.rs`（判据 ⑥ `…_is_rejected_and_never_migrated`）；
  `docs/ledger/phase-status.md` 的 `ROAD-M1-005` 行；勘察报告 ＝ 任务单 `#105`（本 ADR 的事实源）
- **日期**: 2026-10-16

## 背景（这是规范与规范的冲突，不是实现缺口）

三条规范把同一件事说成了两个方向：

1. `ROAD-M1-005` 逐字：「**历史数据迁移器**：交付 `crates/yeban-model/src/migration/` 模块，
   将历史工程**无损升格**为合法 v1.0.0 AST。」
2. `ADR-0001` **D43**（负责人 2026-10-04 **明确授权**）逐字：「在发布 **1.0.0 正式版**之前，
   我们**不要有任何历史包袱和旧版本兼容之类的需求**……**直接推翻之前的设计和代码就好**」，
   并逐字给出执行口径：「**凡是"为了读旧文件/旧格式"而存在的分支，一律删除**」，
   收尾逐字：「后续所有"要不要为旧格式留一条路"的讨论，**默认答案是"不留"**。」
   而**一个迁移器就是**这样一条"为了读旧文件"的分支。
3. `ADR-0001` **D3** 把两条连在一起：它先判「历史上**从未发布过** `schema_version = 2/3` 的文档，
   所以"迁移兼容"**没有对象**」，随后在代价栏留口：「**若确实存在外部 v3 文档**，需补一个迁移器
   （`ROAD-M1-005` 已预留 `src/migration/`）」。

⇒ ⚠ **本条是两种冲突叠在一起**（`AGENTS.md` §5.7 的第 269 条三分法）：
**条目本身是②（规范与规范冲突）**，**它预设的输入对象是③（规范没覆盖）**。
⚠ 这两问必须**分开**回答：只答其中一个会得到错误的动作（答②会去建一个空注册表，
答③会去解析一个没人承认的格式）。

## 事实（任务单 `#105` 的实测读数，逐条可复核）

1. **Yeban 侧的"历史文档" ＝ 0 份。** `SCHEMA_VERSION = READER_SCHEMA_VERSION = MIN_READER_VERSION = 1`
   （`project.rs:71/74/77`）；`check_readable()` 只有**两条拒绝规则**、**没有任何接受或迁移分支**；
   `ModelError` 只有 2 个版本臂（`SchemaVersionTooNew` / `ReaderTooOld`）。
2. **`OpOrigin::Migration` 的生产者 ＝ 0 个、消费者 ＝ 0 个**（非测试代码口径）：
   5 处代码命中里 1 处是枚举定义、3 处是测试、1 处是 `Display` 表（`crates/yeban-app/src/import_audio.rs:177`），
   外加 1 处契约 `schemas/ops.schema.json:24`。对照：`OpOrigin::MidiInput` 有真生产者
   （`midi_notes.rs:473`、`import_midi.rs:257`）。
3. **`crates/yeban-model/src/migration/` 不存在**（`ls` ⇒ `No such file or directory`；`find -name 'migration*'` ⇒ 0 命中）。
4. **仓库内的旧工程夹具 ＝ 0 个**：`git ls-files | grep -cE '\.groove$'` ⇒ 0；`\.yeban$` ⇒ 0；
   `git grep -n "groove-project"` ⇒ 0 命中；`git grep -n "groove-package"` ⇒ 0 命中。
5. **磁盘上确有旧工程文件，但它们不是 Yeban 文档。** 全盘 `find`（剪掉 `node_modules`/`Library`/`.git` 等）
   命中的 `.groove` ＝ **6 个**，全部在 `/Users/crow/Downloads`。其中：
   - **1 个**是真旧 Groove Web 工程（`假如时光能够倒流.groove`，**19111 字节**，顶层 `format = "groove-project"`，
     `version = 1`（数字））；
   - **5 个**的顶层 `format = "groove-package"`（版本字段是**字符串** `schemaVersion` / `projectVersion`，
     取值如 `"2.34.16-dev"`）。⚠ 这 5 个的形状在**旧 Web 实现**（`groove/src`、`groove/docs`、`groove/mcp`、`*.md`）
     与旧规范树（`/Users/crow/work/groove_v3/`）里**各命中 0 次** ⇒ **没有任何规范描述它们**
     （其 `generator` 自称 `groove-mcp-*-pipeline`）。
6. ⭐ ⭐ **"无损升格"经测量不成立。** 对那 1 个真 `groove-project` v1 做逐字段对照
   （左列 ＝ 真实文件的顶层字段，**24** 个）：**有精确对应物 ＝ 9 个**
   （`id`／`name`／`bpm`／`timeSignature`／`isMetronome`／`isCountIn`／`tags`／`createdAt`／`updatedAt`）；
   **没有精确对应物 ＝ 15 个**（`genreId`／`genreName`／`swing`／`resolution`／`stepCount`／`patterns`／
   `activeSlot`／`songMode`／`songChain`／`sections`／`loopRange`／`effectsRack`／`drumKit`／`isFavorite`／`notes`）。
   9 个里还有 **3 个带语义偏差**：`id` 的旧值含 `_` 且 24 字符，而 `EntityId` 要求 26 字符 Crockford Base32 ULID
   ⇒ **值不可搬运**；`isCountIn` 是 `bool`，而 `count_in_bars: u8` 是**小节数** ⇒ 旧 `false` 只能落成 `0`；
   `sections` 有同名字段但载荷不对应（`SectionV3` 只有 `{id,name,start_tick,end_tick,color}`，
   旧 section 的 `slot`／`bars`／`mute`／`overrides` 全无落点）。
   ⚠ 步进网格那一族最典型的量：8 条 lane × 16 格 ＝ **128** 个步进格（非零 **23**），
   `YebanProjectV1` 里**没有**落点字段（`grep -rin "swing|resolution|step_count|genre|favorite|song_chain|song_mode|active_slot" crates/yeban-model/src/` ⇒ 0 命中）。
7. **正对照（证明上一条的对照方法不是空转）**：`project.bpm = 84`（int）↔ `YebanProjectV1.bpm: f64`（`project.rs:1593`），
   单位同为 BPM，可逐值搬运；`timeSignature = "4/4"` ↔ `TimeSignature { numerator: 4, denominator: 4 }`（`:1595`），
   表示法不同但语义同一。
8. **损失表形状已存在一份，且依赖方向不允许复用**：`yeban_render::als::AlsLoss`
   （`crates/yeban-render/src/als.rs:143`）＋ 两个机器可读前缀（`LOSS_UNMAPPED_PREFIX = "未映射:"` `:120`、
   `LOSS_NOT_EQUIVALENT_PREFIX = "非等价:"` `:123`）。⚠ `yeban-model` **不**依赖 `yeban-render`，
   反向依赖存在（`crates/yeban-render/Cargo.toml:37`）⇒ 住在 `yeban-model/src/migration/`（条目指定路径）
   的迁移器**不能**引用 `AlsLoss`。
9. **旧实现自己的政策与本仓相反**（如实登记，供比较）：旧 Groove Web 的 `.groove` 是**加法兼容**的
   （`project.ts:99-105` 逐字：「The reader treats a missing arrangement as "one clip, no sections" …
   so old files keep working and nothing has to be migrated.」），且它自带 4 个 `migrate*` ＋ 2 个 `normalise*` 读者。

## 裁决（待负责人一句话确认）

### D1 — `ROAD-M1-005` 的处置：**本阶段有意不做，条目保留为路线图记录**

**建议（A）**：把 `ROAD-M1-005` 记作「**本阶段有意不做**」，理由 ＝ 事实 1／2／5／6
（没有 Yeban 旧文档 ⇒ 没有对象；唯一真实旧格式不是 Yeban 文档，且 **15/24 字段无落点** ⇒ "无损升格"不可能）。
**保留** `src/migration/` 这个**路径预留**（`ADR-0001 D3` 已写它），但**不建**模块、**不注册**任何迁移步骤
—— 一个零历史步骤的注册表会把"源 schema"变成**必须编造**的东西（`AGENTS.md` §5.7）。

**选项**：**A**（本条建议）／**B** 保留 `PENDING` 到 1.0.0 之后重评／**C** 把条目措辞改为"只做版本拒绝面"。

**不决定的后果**：台账里 `ROAD-M1-005` 会继续以 `PENDING` 出现在"还剩几项"的计数里，
而它的"未实现"理由既不是排期也不是能力不足，而是**规范不允许**；这会让后续接手者重复本轮勘察。

### D2 — 旧 `.groove` 文件的地位：**不纳入（不是 Yeban 文档）**

**建议（A）**：明确 **`groove-project` v1 与 5 份 `groove-package` 都不是 `ADR-0001` D3 所说的"外部 v3 文档"**
（后者指 `schema_version = 2/3` 的 **Yeban** 文档，实测 0 份）⇒ 不纳入迁移输入。
⚠ 这条**同时**回答了一个风险：若把 `~/Downloads` 的那 1 个文件当迁移夹具入库，
就会把一份**来源与许可未登记**的用户文件变成仓库契约（`AGENTS.md` §2 第 9 条）。

**选项**：**A**（本条建议）／**B** 把 `groove-project` v1 命名为迁移输入并新开条目／**C** 只登记、不处理、不解析。

**不决定的后果**：`~/Downloads` 里那 6 个文件的地位会一直悬着；
每一次"要不要支持旧工程"的讨论都要重新做一遍本轮清点。

### D3 — 损失表形状的归属：**现在不决定（因为 A 之下没有损失表）**

**建议（C）**：**不做**损失表。若将来 D2 选 B（接纳旧格式），再走一次裁决 ——
那时才需要在"A 在 `yeban-model` 新开一份（仓内会有 2 份）"与"B 把迁移器移出 `yeban-model`
以复用 `AlsLoss`（与条目路径冲突）"之间选（事实 8）。

**选项**：A 在 `yeban-model` 新建／B 迁移器移出 `yeban-model`／**C** 不做（本条建议）。

## 与既有裁决的关系

- **不推翻** `ADR-0001 D3`：它判的是"内部版本号没有迁移对象"，本条补的是"**外部**旧格式也不是迁移对象"。
- **复用** `ADR-0001 D43`（负责人授权、且是本 ADR 里优先级最高的口径）与 `HD-23`（版本只用于拒绝不匹配）。
- **不放松**任何东西：本条只关掉一个**未开工**的条目，⛔ 不动判据、⛔ 不动 schema、⛔ 不动红线。
- `crates/yeban-model/tests/no_compat.rs` 判据 ⑥ 的现状（拒绝而不迁移）**与本条一致**，⛔ 不改它。

## 当前处置（不构成单点阻塞）

⛔ 本 ADR **不阻塞**任何在制品：`src/migration/` 今天不存在，也没有任何在制的线依赖它。
在负责人裁决之前，队列里**跳过** `ROAD-M1-005`、继续推进其它功能项；
本条在 `docs/ledger/human-decisions.md` 里登记为 **`HD-64`**。
