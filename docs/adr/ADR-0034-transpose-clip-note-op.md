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
{"kind": "transposeClip", "deltaPitch": -5}
```

（`trackId` / `clipId` 在**工具顶层**，⛔ 不在 op 对象里 —— 与其余读音符的形态同一条
寻址口径；`clipId` 因此进入 `ENVELOPE_CLIP_KINDS`。）

- 语义：把该片段 `notes` 表里**每一条**音符在**同一条 `Op::Batch`** 内变成
  `Op::MoveNote { delta_tick: 0, delta_pitch: }`；起点/时值/velocity/`note_id` 一位不动。
- 展开在**服务端**做（`compile`），调用方⛔ 不需要知道音符 id，也不必先查询。
- `deltaPitch` 是 `i8`（半音，可负）；越出 `0..=127` 的音符在 `apply` 前置校验里
  响亮失败（沿用 `ops.rs:1221-1228` 的既有拒绝），⇒ **整批原子**，不会留下"半个片段转好了"。
- 空音符表 ⇒ **展开成 0 条 `Op`、⛔ 不是错误**（判据 6 钉的是 `compile` 返回
  `Ok(vec![])` 这一层；"0 条 op 的提案在工具层回执长什么样"由既有的空 `ops` 口径承重，
  本票不另立，也⛔ 顺手给它编一个错误码）。
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

> ⚠ **本节于落地那一次提交就地改正**（原判据 1…6 是设计期的**草案名**，且有两处把取值猜错）：
> ① 原判据 5 写 `commitModel == "direct"` —— `yeban_edit_notes` 是**提案类**，`ADR-0032` 的
> 映射给的是 `proposal`，带 `autoMerge` 时是 `proposalAutoMerged`（本票不新增取值，
> 但必须自述对的那一个）；② 原判据 6 把"`deltaPitch: 128`"归到 `INVALID_PARAMETER_RANGE` ——
> `read_i8` 的口径是**形状**错误落 `INVALID_PARAMETER_RANGE`、**区间**错误落 `OUT_OF_RANGE`
> （与逐条 `move` 同一份算式，⛔ 不是第二条口径）。下面是与 `tests/transpose_clip.rs`
> **逐字同名**的七条。

| # | 判据（`crates/yeban-mcp/tests/transpose_clip.rs`） | 钉住 |
| :--- | :--- | :--- |
| 1 | `transpose_clip_expands_into_one_move_per_note_in_a_single_batch` | D1+D2：一条形态 → N 条 `Op`，`data.merge.appliedOps == N`、`data.autoMerged`、`commitModel == "proposalAutoMerged"`（一个批 = 一次提交） |
| 2 | `every_note_moves_in_pitch_and_nothing_moves_in_time` | D1：逐条 `pitch + 2`；`start_tick` / `duration_ticks` / `velocity` / 身份集合一位不动（`deltaTick` 恒 0）；D2 的"半音级"（⛔ 读谱调） |
| 3 | `one_undo_step_reverts_the_whole_transposition` | D2：**一次** `yeban_undo` 把整批 N 条还原（⛔ 需要 N 步） |
| 4 | `an_out_of_range_note_rejects_the_whole_batch_without_partial_apply` | D3：`OUT_OF_RANGE` + `data.noteId` 点名，失败前后逐音符读数**完全相同**（⛔ 部分应用） |
| 5 | `the_payload_names_no_note_and_rejects_every_illegal_shape` | D1/D5：载荷里⛔ 不出现 `noteId`（给了 ⇒ `unknownTransposeClipField` + `supportedFields`）；缺键 / `2.5` ⇒ `INVALID_PARAMETER_RANGE`；`128` ⇒ `OUT_OF_RANGE`；两张发布清单都含本形态 |
| 6 | `an_empty_midi_clip_expands_to_zero_ops_and_is_not_an_error` | D5：空音符表 ⇒ `compile` 返回 `Ok(vec![])`（合法，⛔ 不是错误码） |
| 7 | `the_form_is_published_on_both_surfaces` | D1 的发布面：`ops.items.oneOf` 分支封闭（`additionalProperties: false`）、`x-envelope.clipId: "required"`、`required == [kind, deltaPitch]`、`±128` 界、描述写明"整片段 / 每个 / 展开 / 半音" |

