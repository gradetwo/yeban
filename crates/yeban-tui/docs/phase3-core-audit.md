# 夜半 TUI 三期 · app 核心能力审计

基线：main `291fc4fc6`。审计对象：二期收尾时登记的五项缺失能力。
审计面：`crates/yeban-model/src/ops.rs` 的 `Op` 全集、`crates/yeban-app/src/undo.rs`
的 `UndoPort`、`crates/yeban-app/src/host.rs` 的 GUI 实际调用、
`crates/yeban-app/src/input.rs` 的共享键位策略表、`crates/yeban-app/src/cli.rs`
的无头入口。
裁决口径：**已有** = 核心 op / 命令面可表达且有现存消费者，TUI 只需接线；
**缺失** = 需要新 op 或新核心机制，TUI 线不私自发明，写成规格另行立线。

结论先行：**五项全部「已有」**。本期不需要新 `Op` 变体，不动 `Cargo.lock`。

---

## 1. 片段拆分（在播放头处 split clip）—— 已有

证据：

- 模型层没有 `SplitClip` 变体，也不需要。拆分可由既有 op 组合完整表达。
- GUI 的实际实现：`crates/yeban-app/src/host.rs` 的
  `split_selected_clip_at_playhead`（`Cmd/Ctrl+T` 落点）。组合形状：
  `clip_duration_ops_for`（`SetClipPlacementDuration` 缩短左半）
  ＋跨切分点音符的 `SetNoteTiming`（左侧截断）与 `DeleteNote`（移交右半）
  ＋`Op::AddClip`（右半新池条目，名后缀 ` (Part 2)`）
  ＋`Op::AddClipPlacement`（右半摆放，继承 loop/muted/offset）。
  一次 `UndoPort::commit_ops` 提交，一步撤销。
- 键位：共享策略表 `Action::SplitAtPlayhead`（`input.rs`，`Cmd/Ctrl+T`，
  解析器判据钉住 meta/ctrl 两路）。TUI 二期未接此 Action，按键只落日志。

缺口（仅接法，非核心）：组合逻辑写在 host.rs 的私有 Slint 耦合函数里，
TUI 不能直接调用。

TUI 接法：把 op 组合抽成 host.rs 的 pub 纯函数
`clip_split_ops_for(project, placement_ulid, split_tick) -> Option<(Vec<Op>, String)>`，
GUI 原函数改调它（行为逐字不变，含退化切分拒绝：切分点必须严格落在摆放内部）；
TUI 在片段网格焦点下以「光标处片段 × 播放头 tick」调用，`Ctrl+T` 走共享解析。

## 2. 片段复制 / 粘贴 —— 已有

证据：

- 剪贴板模型：`crates/yeban-app/src/host.rs` 的 thread-local
  `CLIP_CLIPBOARD: RefCell<Option<(ClipPoolEntry, ClipPlacement)>>`
  （进程内快照：池条目 ＋ 源摆放）。
- 复制：`clipboard_copy_selected`（host.rs）。粘贴：`clipboard_paste`（host.rs）
  ＝ `Op::AddClip`（片段换新身份、每个音符换新身份）＋ `Op::AddClipPlacement`
  （落点 = 选中轨 × 播放头，拒主总线），一次提交一步撤销。
  剪切：`clipboard_cut` ＝ 复制 ＋ `clip_delete_ops_for` 同批提交。
- 键位：共享策略表 `Action::CopyClipboard / PasteClipboard / CutClipboard`
  （`input.rs`，`Cmd/Ctrl+C/V/X`，ADR-0076 B3）。TUI 二期未接这三个 Action。

TUI 接法：TUI 自持同形状的进程内剪贴板（条目 ＋ 摆放快照），粘贴组合逐项照
`clipboard_paste`。两处形态差异如实登记：复制源 = 光标处片段（GUI 是界面选中的
片段）；粘贴落点 = 选中轨 × 小节光标 tick（GUI 是播放头；TUI 的 `:seek` 同步
移动播放头与光标，光标即 TUI 的编辑点）。

## 3. 片段改名 —— 已有

证据：

- `Op::SetClipName { clip_id, old_name, new_name }`（`crates/yeban-model/src/ops.rs`）。
  只写 `ClipPoolEntry::name`，不动池键与任何摆放（名字是纯展示注记）。
- 现存消费者：`crates/yeban-mcp/src/domain/notes.rs` 的 `NoteOp::SetClipName`
  （`yeban_edit_automation` 的 `setClipName` 形态）。GUI 未接（bridge/host 无调用点），
  但核心通路完整、有判据。

TUI 接法：片段网格焦点下 `i` 进入 Insert 改名，目标改为光标处片段的池条目，
提交 `Op::SetClipName`。轨道改名保持轨道/检查器焦点下的既有行为不变。

## 4. 音符级编辑 —— 已有

证据：

- 模型 op 全套（`crates/yeban-model/src/ops.rs` 的 `Op` 枚举）：
  `AddNote`、`DeleteNote`（自带 `previous_note` 撤销载荷）、`MoveNote`（增量）、
  `ModifyNoteVelocity`、`SetNoteTiming`、`SetNoteExpression`、
  `SetNotePitchShape`、`SetNotePitchBendPoint`。
- GUI 钢琴卷帘的实际调用面在 host.rs：`note_identity_for`（音符身份解析唯一口）、
  `split_ops_for`（切音符）、`duplicate_ops_for`（复制音符）、
  `nudge_delta_for`（`[UI-NOTE-005]` 微移换算：Grid = 240 tick、Tick = 1、
  Semitone = ±1、Octave = ±12）与多处 `Op::AddNote` 构造点。
- 读取面：音符住 `ClipPoolEntry::content = ClipContent::Midi { notes }`；
  `YebanProjectV1::note(clip_id, note_id)`（`crates/yeban-model/src/project.rs`）。
- 键位：共享策略表把方向键解析为 `Action::NudgeSelection`
  （←/→ = Grid、↑/↓ = Semitone、Shift+↑/↓ = Octave、Alt+←/→ = Tick）。
  TUI 二期未接此 Action。

TUI 接法（从简从实）：片段网格焦点下裸 `n`（共享表 PassThrough、Vi 层未占）
打开音符列表视图，对象 = 光标处 MIDI 片段。视图内：`j/k` 选音符；
`h/l` 与 ←/→ 移时间（`MoveNote`，一格 240 tick，复用 `nudge_delta_for`）；
`J/K` 与 ↑/↓ 移音高；`a` 加音符（`AddNote`：新身份、起点 = 小节光标、
音高 = 选中音符音高或 60、时值 = 一拍、力度 = 100）；
`Delete` 删音符（`DeleteSelection` 的视图内落点）；`<`/`>` 改时值
（`SetNoteTiming`，一拍步进，下限一拍）；`-`/`+` 改力度
（`ModifyNoteVelocity`，±10，夹 1..=127）。
每笔一次 `commit_ops`，一步撤销。播放走既有走带（Space），不复刻卷帘。
落点安排：`j/k/h/l` 走 Vi 动作层按视图分流；方向键走 `NudgeSelection` 分发
按视图分流；`J/K`、`a`、`<`、`>` 在共享解析 PassThrough 后、Vi 层之前按视图
拦截；`-`/`+` 在共享表是缩放，本期在 `ZoomIn/ZoomOut` 分发里按视图分流为
力度（与二期 `DeleteSelection` 按焦点分流同一手法），视图外行为不变。

## 5. 池条目删除 —— 已有（片段池）

证据：

- `Op::RemoveClip { clip_id, previous_clip }`（`crates/yeban-model/src/ops.rs`）。
  前置条件：条目存在且**没有任何摆放引用它**，否则模型硬拒
  `ModelError::ClipInUse`——引用中的条目删不掉，不会留下悬空摆放。
  撤销载荷 `previous_clip` 自包含，删除可撤销。
- 现存消费者：`crates/yeban-mcp/src/domain/notes.rs` 的 `NoteOp::RemoveClip`
  （唯一的池级形态）。GUI 无池管理面。

附注（采样/资产池）：`Op::RemoveAsset` 只删 `project.assets` 的索引项，
资产字节归容器层（`ProjectArchive`）所有。TUI 无资产浏览面，本期不接，
不在本项范围内。

TUI 接法：`:pool` 命令打开片段池列表视图（条目名、内容类型、引用数）；
`j/k` 选条目，`Delete` 删（`Op::RemoveClip`）。被引用时模型拒绝，
TUI 如实报拒绝原因与引用数，不代删摆放。

---

## 键位冲突核查（新键先行核对）

- `Ctrl+T / C / V / X`：共享策略表既有 Action（SplitAtPlayhead /
  CopyClipboard / PasteClipboard / CutClipboard），TUI 本期只是补上分发落点，
  不新增绑定，与 `input.rs` 判据同向。
- 方向键：共享表既有 `NudgeSelection`，本期只在音符视图内接落点，
  视图外保持二期的未接状态。
- `Delete`：共享表 `DeleteSelection`，二期已在片段网格焦点接删摆放；
  本期在音符/池视图内接各自落点，同一「删当前选中对象」语义。
- 裸 `n`：共享表 PassThrough、Vi 状态机未占，TUI 新增（开音符视图）。
- 裸 `a`、`<`、`>`：共享表 PassThrough，仅在音符视图内拦截。
  裸 `-`/`+`：共享表是缩放；本期在分发层按视图分流（音符视图内 = 力度，
  视图外 = 缩放，行为不变）。
- `Tab` 焦点循环保持三窗格不变（K6 判据不改）。
