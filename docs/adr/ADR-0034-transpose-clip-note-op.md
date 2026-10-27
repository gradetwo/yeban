# ADR-0034 — 片段级移调（`transposeClip`）：转调走已有 `MoveNote`，⛔ 不新增 op 变体

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定；D4 三条仍为待人类裁决的规范缺口）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-006`（`yeban_edit_notes`）、`MODEL-AST-002`（`YebanProjectV1`）、
  `ARCH-DET-001`（L1 确定性）、`ARCH-OPS-001`（领域操作日志）、`ADR-0032`（回执自述提交模型）、
  `crates/yeban-mcp/src/domain/notes.rs`（`OP_KINDS` / `parse_one` / `compile`）、
  `crates/yeban-model/src/ops.rs`（`Op::MoveNote`、`Op::Batch`）
- **日期**: 2026-10-27

## 背景

审计 G18 的后半句：**"每段可换 scale，但段间无过渡/转调辅助"**。
本片先测后改（测量纪律 §6「先 grep 交付，再下结论」），实测结论是**工具面已经有移调的
原子能力，缺的是"整片一次算完"的粒度**：

| 事实 | 证据 |
| --- | --- |
| MIDI 移调 op **已存在**：`Op::MoveNote { track_id, clip_id, note_id, delta_tick: i64, delta_pitch: i8 }` | `yeban-model/src/ops.rs:188-199` |
| 它是全集中**唯一**的相对变换 op（就地改音高、保留 `note_id`、`apply`/`inverse`/前置条件齐备） | `ops.rs:176-187`、`:2882`（apply）、`:2297`（inverse）、`:1343`（前置） |
| 音域硬校验 `0..=127`，越界拒 | `ops.rs:1221-1228`；MCP 侧 `OUT_OF_RANGE` `notes.rs:8187` |
| MCP **已暴露**：`yeban_edit_notes` 的 `ops[].kind:"move"` + `deltaPitch` | `notes.rs:4332-4336`、编译 `:8177-8215`；item schema `op_schema.rs:401-407` |
| N 条 op **已经**折叠成恰好一条 `Op::Batch`（一步撤销） | `ops.rs:1163`、`domain/mod.rs:1972-1981`（`merge_batch`） |
| **没有** clip 级"整表替换音符"的 op：唯一写全音符表的是 `Op::AddClip`，且仅创建 | `ops.rs:409`、`notes.rs:10006`；`setClipNotes` / `replaceClip` **未找到** |

⇒ 今天的真实摩擦不是"不能转调"，而是**调用方必须自己数音符**：先 `query_project` 取该
片段 `notes` 的**全部 noteId**，再逐条写 `{"kind":"move","noteId":…,"deltaPitch":d}`。
一个 40 音的片段就是 40 个 op —— 载荷、往返、以及"漏一个音就转调不彻底"的错法风险
都在调用方身上。这与 `G13`（编辑→合并两轮往返）同族：**能力在，粒度不在**。

## 决定

### D1 新增一个 **NoteOp kind**（不是新 `Op` 变体）：`transposeClip`

`yeban_edit_notes` 的 `ops[]` 接受

```json
{"kind": "transposeClip", "trackId": "…", "clipId": "…", "deltaPitch": -5}
```

- 语义：把该片段 `notes` 表里**每一条**音符在**同一条 `Op::Batch`** 内变成
  `Op::MoveNote { delta_tick: 0, delta_pitch: }`；起点/时值/velocity/`note_id` 一位不动。
- 展开在**服务端**做（`compile`），调用方⛔ 不需要知道音符 id，也不必先查询。
- `deltaPitch` 是 `i8`（半音，可负）；越出 `0..=127` 的音符在 `apply` 前置校验里
  响亮失败（沿用 `ops.rs:1221-1228` 的既有拒绝），⇒ **整批原子**，不会留下"半个片段转好了"。
- 空音符表 ⇒ `appliedOps` 为 0 的成功回执（不是错误）：合法且可核对。
- **⛔ 不新增 `Op` 枚举变体**。理由：`Op::MoveNote` 已完备（apply/inverse/前置/撤销都有），
  本票要的只是"一次描述、逐音展开"的**表达层**。加 `Op` 变体要动
  `ops.rs:155/1252/1343/2297/2882`、`schemas/ops.schema.json` 的 `oneOf`（`:634` 要求覆盖全变体）、
  `op_schema.rs`、`notes.rs` 的 `OP_KINDS/parse_one/compile` —— 换来的语义与 `MoveNote` 重复。

### D2 只保证**十二平均律位移**，⛔ 不 promises "换调后仍在调内"

`transposeClip` 是**色高**（chromatic）变换：`deltaPitch = 目标主音 - 原主音 (mod 12)` 由
调用方自己算。生成期已有的 `shift_bytes`（八度折叠，`section_build.rs:789`）与
`snap_into_scale`（`:761`）**不**接进 edit 期，因为"把 F 大调的旋律挪到 D 小调时每个音
该落到哪个调内音"没有唯一答案 —— 那是**规范缺口**，⛔ 私自选一套听感规则（AGENTS.md §5.7）。
需要调内收拢的调用方，今天就有一条不假的路：转调后再逐音 `edit_notes` 修正。

### D3 段级**没有**地方存新调号：本片如实不动

实测 `SectionV3 { id, name, start_tick, end_tick, color }`（`yeban-model/src/project.rs:1373-1385`）
**无** `scale` / `key` 字段；调号只有工程级 `YebanProjectV1::key_signature`（`:1656`，
`KeySignature{sharps_flats,is_minor}` `:137`），可写入口只有 `Op::SetKeySignature`（`ops.rs:921`）
/ MCP `setKeySignature`（`notes.rs:1946`）。而 `yeban_propose_section` 的 `scale` **只在生成期消费**
（`section_build.rs:1520/1602`、id 种子 `:1185/1649`），⛔ 不持久化。
⇒ 本票**不**动数据模型，也⛔ 悄悄写全局 `keySignature`（那会把"转调"和"改谱面调号"两件事混成一件）。
段级 key 字段要不要存在 **D4**。

### D4 登记，不发明

1. **段级调号**（`SectionV3.key`）= 模型/格式变更，需人类批准（新 ADR）。
   注：`Op` enum 是 serde **外标签**且 `SCHEMA_VERSION=1`（`project.rs:71`，
   `check_readable:1789` 只挡"读端过旧"）⇒ 加变体本身向后兼容、不破既有 `.yeban`、
   无需版本升；`MODEL-AST-007` 的 CAS 是 `AssetHash` 内容寻址（`ids.rs:120`、
   `project.rs:1702`），与 op 变体格式无关。也就是说"技术上不难"，但它是**规范**问题而非实现问题。
2. **调内收拢的转调**（`transposeToKey`，D2）= 需要一套规范映射。
3. **跨段过渡/共同音写法的 smooth voice-leading** = 需要 `yeban-theory` 侧的公开面，另一票。

## 判据（落地时随代码一起提交）

1. `transpose_clip_covers_every_note_without_ids`：夹具片段含 N 个音符，一条
   `transposeClip` ⇒ 响应的 `appliedOps == N`，且**逐音** `pitch + delta` 正确、
   `startTick` / `durationTicks` / `velocity` / `note_id` **逐字节不变**。
2. `transpose_clip_is_one_undo_step`：一次 `yeban_undo` 回到转调前的工程字节
   （与转调前逐字节相同），证明它落在同一条 `Op::Batch` 上。
3. `transpose_clip_out_of_range_rejects_the_whole_batch`：任一音越 `0..=127` ⇒
   `OUT_OF_RANGE` 且工程⛔ 无半程修改（`appliedOps` 为 0 / 未落盘）。
4. `transpose_clip_needs_no_note_ids`：调用载荷里⛔ 不出现任何 `noteId`（与"必须先查再逐条写"
   的旧形状划清界限）；空音符片段 ⇒ `success` + `appliedOps: 0`。
5. `transpose_clip_receipt_self_describes`：回执 `data.commitModel == "direct"`
   （`ADR-0032` 的既有映射，本票不新增取值）。
6. `delta_pitch_bounds_are_loud`：非整数 / 超出 `i8` ⇒ `INVALID_PARAMETER_RANGE`。
