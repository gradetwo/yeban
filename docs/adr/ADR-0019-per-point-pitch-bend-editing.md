# ADR-0019 — `yeban_edit_notes` 支持逐点编辑弯音曲线（`setPitchBendPoint`，消除 WALL ⑤）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-006`（`yeban_edit_notes` 契约）；`schemas/ops.schema.json`；
  `crates/yeban-model/src/ops.rs`（`Op::SetNotePitchBendPoint`）；
  `crates/yeban-mcp/src/domain/notes.rs`、`crates/yeban-mcp/src/op_schema.rs`；
  `MODEL-AST-001`（960 PPQ 整数网格）、`MODEL-AST-005`（`MidiNote`）；
  `ADR-0012`（弯音曲线契约：`cents` ±1200、tick 严格升序）；
  `WALL ⑤`（`creation_acceptance.rs`：弯音曲线只能整体写的缺口）
- **日期**: 2026-10-27

## 背景

`crates/yeban-mcp/tests/creation_acceptance.rs` 的墙清单里，**WALL ⑤** 记录：

> 主音的颤音曲线只能整体写（`add.note.pitchBendCurve` 或 `setNotePitchShape` 的
> `newPitchBendCurve`），**没有**逐点增删的 op；改一个点要整条重写。

实测确认：写侧只有两个入口，且都要求调用方**重传整条曲线** ——

1. `yeban_edit_notes` 的 `add.note.pitchBendCurve`（只能给新音符）；
2. `yeban_edit_notes` 的 `setNotePitchShape.newPitchBendCurve`（`Op::SetNotePitchShape`
   的撤销载荷是整条旧曲线）。

因此"改第 3 个点"必须先 `yeban_query_project` 读回整条曲线、本地改一个点、再整条回写；
"删一个点"更要自己拼出"不含该 tick"的整条曲线。这是**读回-重传**往返，且并发的两次
整条重写会互相覆盖（后写者获胜），对 AI 创作颤音/Vibrato 这类**逐点微调**场景成本很高。

## 决定

### D1 新增模型变体 `Op::SetNotePitchBendPoint`

```rust
SetNotePitchBendPoint {
    track_id: EntityId,
    clip_id: EntityId,
    note_id: EntityId,
    tick: u64,               // 与 pitch_bend_curve 的键同一坐标系（音符局部/绝对同形）
    old_cents: Option<i16>,  // None = 该 tick 原先没有点
    new_cents: Option<i16>,  // None = 删除该 tick 的点
}
```

状态转移（与 `Op::SetTempoPoint` 同形）：

| `old_cents` | `new_cents` | 语义 |
| :--- | :--- | :--- |
| `None` | `Some(cents)` | 插入新点（该 tick 必须**没有**点） |
| `Some(old)` | `Some(new)` | 改值 |
| `Some(old)` | `None` | 删除 |
| `None` | `None` | 无意义 ⇒ `ModelError::OpStateMismatch` |

`precondition` 从**文档现值**核对 `old_cents`（不一致 ⇒ `OpStateMismatch`），并把
`Some(cents)` 限制在 `±PITCH_BEND_CENTS_MAX_ABS`（1200）内，越界 ⇒
`ModelError::PitchBendCentsOutOfRange { value, tick }`（沿用 `ADR-0012` 的值域）。
`commit` 用 `binary_search_by_key` 定位后原地改/删/插，⛔ 不重排、⛔ 不覆盖相邻点；
升序由插入位置保证。`invert` 只交换两个 `Option<i16>`。

### D2 新增 `yeban_edit_notes` op kind `setPitchBendPoint`

```json
{"kind": "setPitchBendPoint", "noteId": "<26 字符 ULID>", "tick": 120, "cents": 300}
{"kind": "setPitchBendPoint", "noteId": "<26 字符 ULID>", "tick": 240, "cents": null}
```

- 字段白名单**恰好**四条：`kind` / `noteId` / `tick` / `cents`；其它键 ⇒
  `INVALID_PARAMETER_RANGE`（`reason = "unknownSetPitchBendPointField"`，
  载荷带 `supportedFields`）；
- `cents` **必填**：整数音分，或 `null` 表示删除。⚠ **缺键 ≠ null**（缺键报
  `INVALID_PARAMETER_RANGE`，不静默当成删除）；非整数 ⇒ `INVALID_PARAMETER_RANGE`
  （`reason = "centsMustBeInteger"`）；越出 ±1200 ⇒ `OUT_OF_RANGE`；
- `old_cents` 由 `compile` 从**当前文档**的 `pitch_bend_curve` 查得，⛔ 不信调用方声明；
- 删除一个**不存在**的点（`old_cents` 与 `new_cents` 都是 `None`）⇒
  `ENTITY_NOT_FOUND`（`reason = "pitchBendPointNotFound"`，载荷带 `tick` 与 `hint`），
  与 `setTempoPoint` 的 `tempoPointNotFound` 同口径；
- 本 kind 属**音符级**：`is_note_level()` 与 `NOTE_LEVEL_KINDS` 收录它也收
  `reads_envelope_clip()`（`compile` 需要读该片段的音符）。

### D3 兼容性与可逆性

- 不改变 `add.note.pitchBendCurve` 与 `setNotePitchShape` 的任何行为 ⇒ 旧请求**逐字节**
  不变；
- `Op::SetNotePitchBendPoint` 自带 `old_cents` 撤销载荷，`yeban_undo` 一次即逐字节复原；
- 提案仍是"`yeban_edit_notes` 提案 + `yeban_merge_proposal`"模型，与同族音符级 op 一致。

### D4 机械读数同步（本次一并改钉）

- `schemas/ops.schema.json`：`SetNotePitchBendPoint` 分支（`gen-ops-schema-branches.py`
  生成，`--check` 进 CI）；
- `domain/notes.rs`：`OP_KINDS` 46 → 47、`ENVELOPE_CLIP_KINDS` 8 → 9、
  `NOTE_LEVEL_KINDS` 7 → 8；解析表 46 → 47 行、编译表 47 → 48 行（kind → `Op` 变体仍**单射**）；
- `tests/op_reachability.rs`：`EXPECTED_SITES` 54 → 55；
- `tests/fault_vocabulary.rs`：新增 `centsMustBeInteger` / `pitchBendPointNotFound`
  ⇒ reason 159 → 161、规范行 720 → 728、摘要重钉
  `c5056c369e24f34a11ab7349d5af96f4a552f6431c1ebe9a8b18353c6635514d`
  （由判据自己报出后照抄），站点普查 219/216/208 → 221/218/210。
  `unknownSetPitchBendPointField` **不进**黄金表：它作为参数传给 `reject_unknown_keys`，
  扫描器按动态站点处理（与 `unknownSetTempoPointField` 同待遇）。

## 代价

1. **多一个 `Op` 变体**：持久化 AST 的枚举多一支，`ops.schema.json` 多一个 `oneOf` 分支；
   所有"kind → `Op` 单射"的 ratchet 必须同步（本次已改钉）。
2. **逐点插入可能在两处并发编辑时互相踩点**：本 op 只保证"该 tick 的旧值未变"，
   ⛔ 不提供整条曲线的乐观锁。两个不同 tick 的插入互不影响；同一 tick 的第二次改值会
   `OpStateMismatch`（这正是"先读后写"的检测点），调用方需重读后重试。
3. **依赖 `ADR-0012` 的 ±1200 值域**：若人类日后推翻 `ADR-0012` 改值域，本 op 的边界
   校验随之调整（单一常量 `PITCH_BEND_CENTS_MAX_ABS` 已收口）。
4. **删除语义用 `null`**，与"缺键"在 JSON 层可区分（显式 `null` vs 无键），要求调用方
   写 `"cents": null` 而不是省略；错误信息里给出该提示。
