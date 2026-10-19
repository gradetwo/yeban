# ADR-0010 — 片段循环的坐标语义：`start_tick`/`end_tick` 是**摆放相对**坐标，重复按区间长度平铺

- **状态**: **Accepted（集成者裁定；负责人 2026-10-15 授权"需要决策的事、你自己择优选择"）**
- **决定者**: 集成者（Agent）
- **关联**: `docs/ledger/engine-sound-notes.md` 的 needs **N1**（本 ADR 关闭它）；
  `crates/yeban-model/src/project.rs` 的 `LoopConfig`（`:936`）与 `ClipPlacement::loop_config`（`:1079`）；
  `crates/yeban-mcp/src/domain/render.rs` 的既有实现（`loop_plan` / `note_copies`，`#241`）；
  `crates/yeban-engine/src/snapshot.rs` 的 `project_schedules`（`:1108`）；
  台账 `R956`（`#241` 的实现与读数）与 `R961`（`#245` 勘察）
- **日期**: 2026-10-20

## 背景

`N1` 逐字问的是：`ClipPlacement::loop_config` 的 `start_tick` / `end_tick` 到底是
**clip 局部**坐标还是**时间轴**坐标？规范只写"循环配置"，未定义坐标语义，因此引擎
**选择不展开**（一个摆放只播一遍），以免**发明语义**；`N1` 同时写明"**需要一条裁决
（写进 ADR）才能实现**"。

`#241` 已经在 `yeban-mcp` 的导出里实现了展开并钉住了读数，但那条语义**只存在于代码
与台账**里。若引擎直接照它实现而不写这条 ADR，那就是"**照实现抄实现**"——`N1`
要的正是**一条可引用的裁决**，而不是又一次从代码反推。

## 决定

### D1 坐标

`loop_config.start_tick` 与 `end_tick` 是**摆放相对**坐标：从该 `ClipPlacement`
自身的起点 `placement.start_tick` 起算的 tick 偏移（与 `MidiNote::start_tick` 同一
参照系）。

⚠ 这一条**不是**新发明的口径：`#241` 的实现（`render.rs:2126` 的 `loop_plan`
文档）与 `engine-sound-notes.md:219` 的待裁问题逐字都指向它；本 ADR 只是**把它写成
可引用的决定**。

### D2 何时重复

设 `L = end_tick − start_tick`（区间长度）、`D = placement.duration_ticks`。

- `enabled == false` ⇒ **不重复**（单遍）。
- `enabled == true` 且 `start_tick < end_tick` 且 **`end_tick ≥ D`** ⇒ **不重复**（单遍，
  **逐位等于** `enabled == false`）。⭐ 这一条是"恰好覆盖"与"越过末端"的兜底，
  它让"覆盖整段"的循环配置**不改变任何字节**。
- `enabled == true` 且 `start_tick < end_tick < D` ⇒ 区间 `[start_tick, end_tick)`
  按 `L` 平铺重复，重复次数 `N = ceil((D − start_tick) / L)`。
- `end_tick ≤ start_tick` ⇒ **不重复**（非法区间按"无循环"处理，不猜测作者意图）。

### D3 每一遍的内容

- 每一遍以 `end_tick` 为**硬切边界**：音符尾巴裁到 `end_tick`；最后一遍再裁到 `D`。
⇒ 各遍内容**逐遍相同**（这是"重复"的可测定义）。
- `start_tick > 0` 时，区间**之前**的内容在时间轴开头**播一遍**（pickup），不进循环。
- 起音 `≥ end_tick` 的音符**不参与**循环演奏（循环区间就是被演奏的内容）。

### D4 音频片段

音频片段用**同一份** `D1–D3` 的规划；每一遍以素材内的 `source_offset` 指向循环区间
那一段。⚠ 引擎侧今天**没有** `source_offset` 字段（`AudioClipSpan` 只有
`start/end/gain/asset/pcm`），所以音频循环在引擎侧**尚未实现**——那是本 ADR 之后
的一期工作，不是新语义。

### D5 语义的唯一落点

`D1–D3` 是**纯整数**判定，**必须只实现一份**，落在 `yeban-model`（因为**引擎与 MCP
是两个消费者**，且二者都已依赖 model）。`yeban-mcp` 现有的本地实现应当在落地期
改为调用 model，不得保留第二份。

⚠ 这一条是 `R961` 的机械结论：把语义放 `yeban-render` 会让 `yeban-engine` 新增一条
今天不存在的层间依赖边，并把 WAV/SMF/线程池拖进实时 crate 的依赖闭包。

### D6 时间轴换算不在本 ADR 范围

从 tick 换算到帧在引擎（`synth.rs:2153`）与 MCP（`render_math.rs:32`）是**两份实现**，
在 44.1 kHz 上实测有 **118 处**一帧级分歧。那是 **D6 之外的另一个缺口**，本 ADR
**不**声称两者帧级相同；"同一 tick 表"是本 ADR 的验收口径，"同一帧表"需要先把
换算收口成一份。

## 后果

- ✅ `N1` **关闭**：坐标语义有了可引用的裁决，引擎可以照此实现。
- ⚠ **会变的**：`enabled && end_tick < D` 的工程**音频会变**（正是修复目标）。
  `migrate_groove` 导入的工程属于这一类（它主动写 `enabled, 0..natural`）。
- ✅ **不会变的**：无循环字段、`enabled == false`、`end_tick ≥ D`（含恰好覆盖）的工程——
  **一个字节都不动**。`filled_project` 的 `0..3840` 恰等于 `duration_ticks: 3840`，属于此列。
- ⚠ `yeban-engine` 有 `MAX_NOTES_PER_TRACK = 8192`（`synth.rs:155`），展开后副本计入
  `note_schedule_drops`；MCP 无逐轨上限。**两者上限不同**，判据必须显式声明该读数。
- ⚠ `.als` / `.logic` 导出在"循环区间 ≠ 摆放跨度"时**仍记 loss**（`als.rs:890`／
  `logic.rs:4449`）；本 ADR 不改变它们，那是独立口径。

## 未决

1. 音频片段的引擎实现（`source_offset` ＋ 按遍展开）—— 二期。
2. tick→帧两份实现是否收口 —— 另一票，会改 MCP 既有摘要。
3. 大 N 的上限：引擎 8192/轨 vs MCP 无上限，谁对齐谁。
4. `--sfz` 旁路与最小音源（`track_notes_in_frames`）今天连 `ratchet` / `micro_timing` /
   `probability` 都不吃 —— 同族缺口，是否同票补。
