# ADR-0018 — `yeban_edit_notes` 支持在 `create: true` 时同时指定 `placement`（消除 WALL ①）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-006`（`yeban_edit_notes` 契约）；`schemas/mcp-tools.schema.json`；
  `crates/yeban-mcp/src/domain/mod.rs`（`plan_edit_notes`）、`crates/yeban-mcp/src/domain/notes.rs`；
  `MODEL-AST-001`（960 PPQ 整数网格）、`MODEL-AST-002`（`Op::AddClip` 与 `Op::AddClipPlacement`）；
  `WALL ①`（`creation_acceptance.rs` 新建片段与摆放必须分两次调用提案合并的缺口）
- **日期**: 2026-10-27

## 背景

在 `crates/yeban-mcp/tests/creation_acceptance.rs` 的墙清单中，排在首位最高痛点的正是 **WALL ①**：
- **WALL ①**：创建片段（`create: true`）与摆放（`placement`）此前被硬性互斥检查（`placementIsNotCreation`）阻断，同给会直接报 `INVALID_PARAMETER_RANGE`；
- 导致 AI 在为一个新轨道创作材料并放置到时间轴上时，必须进行两轮完整的提案往返（第 1 轮：新建材料放入片段池；第 2 轮：引用该片段并放置到轨道上）；在构建一个包含 4 条轨道的简单工程时，仅建材料和摆放就需要整整 **8 次提案与合并往返**。

在兄弟工具 `yeban_import_audio` 中，早已有成熟且经过严格验证的单次多操作模式：传入 `trackId` 时，在同一次提案中同时生成 `Op::AddClip` 与 `Op::AddClipPlacement`。
`Op::AddClipPlacement` 本身并不要求目标片段在调用前的静态工程中已存在，只要在同一批操作中紧跟在 `Op::AddClip` 之后施加，模型的执行和反转（Undo）即可天然保证原子性与可逆性。

因此，消除 `placementIsNotCreation` 这一人为限制、允许 `create: true` 与 `placement` 同给，具有极高的工程价值与体验收益。

## 决定

### D1 放行 `create: true` 与 `placement` 同给

在 `yeban_edit_notes` 中，当 `create: true` 时，若调用方同时提供了 `placement` 对象，予以放行并合成原子提案：
1. 提取并编译 `ops` 中的音符（保持仅允许 `NoteOp::Add` 规则）；
2. 构造 `Op::AddClip`；
3. 解析 `placement` 对象（复用 `parse_placement_for_create`）：
   - `placement.kind` 必须是 `"add"` 或省略（缺省为 `"add"`）；若显式传入 `"move"` 或 `"remove"`，报 `INVALID_PARAMETER_RANGE`（`reason = "createPlacementKindMustBeAdd"`）；
   - 目标音轨为顶层 `trackId`（必须能在 `project.tracks` 中找到，否则报 `TRACK_NOT_FOUND`）；
   - `startTick` 缺省为 0；
   - `durationTicks` 缺省由新建片段中全部音符的 `start_tick + duration_ticks` 的最大值自动计算；若片段中没有音符且未显式指定 `durationTicks`，报 `INVALID_PARAMETER_RANGE`（`reason = "durationNotDerivable"`）；
   - 支持可选的 `placementId`、`muted`、`loopEnabled`、`loopStartTick`、`loopEndTick`；
   - 检查 `placementId` 未被音轨已有摆放占用（否则报 `CONFLICT` / `placementAlreadyExists`）；
4. 构造 `Op::AddClipPlacement` 并追加到编译操作列表中：
   ```rust
   compiled = vec![
       Op::AddClip { clip_id, clip_name, content: ClipContent::Midi(MidiClip { notes }) },
       Op::AddClipPlacement { track_id, placement },
   ];
   ```
5. 提案标题格式化为：`新建并摆放 MIDI 片段: {clip_name} ({note_count} 个音符) → 音轨 {track_name}`。

### D2 兼容性契约（100% 逐字节向后兼容）

- 当 `create: true` 但未提供 `placement`（或为 `null`）时，保留原有的纯材料新建逻辑，产出的 `compiled` 操作集仅包含 `Op::AddClip`，行为与既有实现**逐字节相同**；
- 当 `create: false` 时，既有的 `placement` 行为（`add` / `move` / `remove`）不受任何影响。

### D3 撤销与幂等性

- `Op::AddClipPlacement` 的逆操作是删除该摆放，`Op::AddClip` 的逆操作是删除该片段池条目；
- 一次 `yeban_undo` 自动逆序执行逆操作，先摘摆放再摘材料，整机状态精确逐字节复原。
