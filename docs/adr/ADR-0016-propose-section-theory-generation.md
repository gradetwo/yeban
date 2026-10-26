# ADR-0016 — `yeban_propose_section` 理论和声骨架生成支持（`materialSource: 'theory'`）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-*`（`yeban_propose_section` 契约）；`schemas/mcp-tools.schema.json`；
  `crates/yeban-mcp/src/domain/section.rs`（`plan` 的 `CLIP_NOT_FOUND` 分支）与
  `crates/yeban-mcp/src/domain/section_build.rs`（声部音轨与路由装配）；
  `crates/yeban-theory/src/progression.rs`（`Progression::expand`）、
  `crates/yeban-theory/src/voice_leading.rs`（`realize` / `realize_three_voices`）、
  `crates/yeban-theory/src/genre.rs`（`GenreRule::progression_at` / `progression_for`）；
  `crates/yeban-mcp/src/domain/ids.rs`（`deterministic_id`）；
  `MODEL-AST-001`（960 PPQ 整数网格）、`ARCH-DET-*`（声学确定性契约）
- **日期**: 2026-10-27

## 背景

`yeban_propose_section` 的既有语义是**从工程材料池装配段落**：它在
`project.clip_pool` 里挑选"至少含一个音符的 MIDI 片段"作为和声材料，再按流派
规则铺成声部音轨。当 `clip_pool` 里没有可用材料时，`plan` 返回契约内的
`CLIP_NOT_FOUND`（`data.missing = "usableClipPoolEntries"`）。

这在**从零创作**的场景里形成了死锁：Agent 被要求"写一段副歌"，但一个新建工程
的 `clip_pool` 为空，工具必然报 `CLIP_NOT_FOUND`；而"先造材料再装配"要求 Agent
自己逐音符手写和声，绕开了 `yeban-theory` 已有的和弦展开与声部连接能力，也无
任何质量与确定性保障。缺口本质是：**工具只有"装配已有材料"一条路径，没有
"从乐理生成骨架"这条路径**。

`yeban-theory` 一侧的件已经齐备且是纯函数：`Progression::expand` 把罗马数字走向
在给定调式（`Scale`）上展开成 `Vec<ChordSpan>`，`voice_leading::realize` 以束搜索
把和弦序列连接成满足音域与跳进约束的声部进行。缺的只是 MCP 层把这两件接进
`yeban_propose_section` 的**决策**——用哪个参数开启、走向从哪来、生成的音符如何
满足确定性红线。规范对此**从未定义**，故立此 ADR。

## 决定

### D1 参数：`materialSource` 与 `progression`

`yeban_propose_section` 的入参新增两个**可选**字段：

1. **`materialSource`**：字符串枚举，取值 `'project'`（**缺省**）或 `'theory'`。
   - `'project'` 保持"从 `clip_pool` 装配"的既有路径；
   - `'theory'` 是**显式 opt-in** 的理论生成路径，不选它就不会凭空生成任何材料。
2. **`progression`**：可选字符串，罗马数字走向（如 `"i VI III VII"`），由
   `Progression::parse` 解析。
   - 提供时按**自定义走向**展开；
   - 缺省时采用**流派首选走向**：从 [`GenreRule`] 登记的 `typical_progressions`
     取首选条目（`progression_at(0)`），不引入任何随机源。

两个字段均为**协议层扩展**（可选、带缺省值），故同步更新
`schemas/mcp-tools.schema.json`，schema 仍是工具契约的唯一权威定义。

### D2 向后兼容性契约

- `materialSource` 为 `'project'`（或缺省）时，检查顺序、错误码、生成的 Ops
  序列与字节效果**100% 保持原样**——包括 `clip_pool` 为空时仍返回
  `CLIP_NOT_FOUND`（`data.missing = "usableClipPoolEntries"`）。既有测试
  （如 `missing_material_is_clip_not_found`）不得改动期望。
- `materialSource` 为 `'theory'` 且 `clip_pool` **有**可用材料时，同样走理论
  生成路径（`'theory'` 的语义是"材料来源"，不是"缺料兜底"）。
- `materialSource` 为 `'theory'` 且 `clip_pool` 为空（或有料，同上）时：以目标
  段落调式构造 `Scale`，用 `yeban_theory::progression::Progression::expand`
  展开走向，再用 `voice_leading::realize`（三声部配置走
  `realize_three_voices`）得到声部进行，生成 **3 或 4 个声部**的确定性 MIDI
  片段，随后进入与既有路径**相同**的音轨/摆放/路由装配（见 D4）。
- 声部数沿用 `section_build` 既有政策：取该流派全部典型走向中和弦构成音数的
  最大值（三和弦 ⇒ 3，七和弦 ⇒ 4）。
- 理论层的失败（走向解析失败、音域内无可行声位等 `TheoryError`）必须映射为
  契约内错误码（`INVALID_PARAMETER_RANGE` 族），**不得**让 `TheoryError` 泄漏成
  传输层异常。

### D3 确定性与 ID 生成规则

生成的每一个 MIDI 音符必须满足：

1. **整数网格**：`start_tick` / `duration_ticks` 全部落在 960 PPQ（`MODEL-AST-001`）
   的整数刻度上；和弦跨度按小节整数划分，音符时值不出现非整 tick。
2. **ID 派生**：音符、片段、音轨等一切 `EntityId` 一律经
   `domain::ids::deterministic_id` 从**稳定的语义种子**（如
   `"theory-note:<section>:<part>:<bar>:<voice>:<index>"`）派生，禁止随机 UUID。
3. **无浮点、无随机**：音高来自 `Pitch::midi_number()` 的整数读数；声部连接由
   `realize` 的束搜索给出**确定解**（同输入同输出）。生成路径上不得出现
   `rand`、墙钟、哈希迭代序（`HashMap`）等非确定源——这是红线 4（确定性状态）
   与 `ARCH-DET-*` 在工具层的延伸。
4. **往返可逆**：同一工程、同一参数重复调用产生**逐字节相同**的 Ops 序列与
   提交后工程哈希（CAS 读数一致）；生成的片段经 `yeban_export_midi` 导出再
   `yeban_import_midi` 导入后音符集合不变。

### D4 声部音轨与路由

保持既有装配语义，**不新增**路由政策：

- 三声部配置命名 `Bass / Alto / Soprano`，四声部配置命名
  `Bass / Tenor / Alto / Soprano`（低 → 高，与 theory 声部序一致；沿用
  `section_build.rs` 的 `PARTS_THREE` / `PARTS_FOUR` 词表）；
- 每个声部一条 `Op::AddTrack` + `Op::AddClipPlacement`；
- 连接沿用 `Op::AddRoutingNode` + `Op::ConnectRouting`，**声部 → 主总线**，
  与既有 `'project'` 路径逐一同构；工程没有主总线音轨时仍报 `TRACK_NOT_FOUND`。

## 后果

- ✅ 从零创作解阻塞：Agent 在空工程上调用 `yeban_propose_section`
  （`materialSource: 'theory'`）即可得到可用的和声骨架，不再需要逐音符手写材料。
- ✅ 兼容性零风险：缺省路径逐字节不变（D2 第一条），存量工程与存量测试不受影响。
- ⚠ 契约面扩大：`schemas/mcp-tools.schema.json` 需登记两个新参数；`TOOLS` 文档
  与工具说明须同步措辞，否则 schema 与实现漂移（测量纪律第 3 条的教训）。
- ⚠ `progression` 缺省值的"首选"取 `typical_progressions[0]` 是一个**配器政策**
  而非 theory 数据本身给出的裁决——theory 只登记了走向集合，"第 0 条即首选"是
  本 ADR 定义的读数，如实登记，不假装是 theory 给的。
- ⚠ 理论骨架是**起点不是成品**：生成的是块状和声节奏（每和弦跨满分配的小节），
  旋律化、节奏加花仍由后续工具（`yeban_edit_automation`、音符级编辑）完成，
  本 ADR 不承诺音乐风格质量。

## 未决

1. `'theory'` 路径是否复用 `clip_pool`（把生成片段**登记入池**供后续段落引用），
   还是每次内联生成、不落池 —— 落池会改 `clip_pool` 字节，需单独一票。
2. `progression` 与 `scale` 不兼容（如走向含降六级而调式不含）时的错误措辞与
   `data` 字段口径。
3. 生成片段的力度（velocity）政策：theory 层无力度概念，本实现取固定值；
   是否需要 `dynamics` 参数另议。
4. 四声部以上（七和弦叠替和弦外音）的声部数上限是否放开。
