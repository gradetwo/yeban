# ADR-0022 — 离线渲染逐轨武装；母带回执点名静音源

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-008`（`yeban_render_master`）、`ARCH-DET-001`（L1 确定性）、
  `ARCH-RT-001`（实时回调零分配 / 槽位内存预算）、`ARCH-RT-002`（快照边界）；
  `crates/yeban-engine/src/synth.rs`（`MAX_TRACK_SLOTS`、`arm_from_snapshot`）、
  `crates/yeban-render/src/engine_source.rs`、
  `crates/yeban-mcp/src/domain/render.rs`（`silent_sources_warning`）、
  `crates/yeban-mcp/tests/render_master.rs`
- **日期**: 2026-10-27

## 背景

2026-10-27 的 MCP 工具面审计（G1）实测：一个 28 轨工程渲染"成功"，回执里
**12 条轨 `peak = 0.0`**，音频里也没有它们，而 `warnings[]` 是空数组。

根因不是混音，是**离线源与实时槽位预算被错误地绑在一起**：

- `SynthEngine::arm_from_snapshot` 把快照里**全部**轨道投影进
  `MAX_TRACK_SLOTS = 16` 个槽位；
- 离线渲染的 `EngineTrackSource` **每个源独享一台引擎**，本来只渲染自己那条轨，
  却也调用 `arm_from_snapshot` 塞入整份快照；
- 工程超过 16 条轨时，按快照顺序排在第 17 条起的源**没有槽位**，
  `render_track` 整轨输出数字静音（`track_drops` 只是计数，没有读者）。

⇒ 工程规模一过 16，母带就**静默丢轨**。这是"创作音乐功能缺口"里最坏的一类：
用户（agent）得到的回执说成功，听感却少了整条轨。

## 决定

### D1 新增单轨武装入口 `SynthEngine::arm_track_from_snapshot`

只把**一条** `track` 投影进槽位；其余快照状态（采样率、solo/mute 门控）语义与
`arm_from_snapshot` **逐条对齐**：

- 同一 `begin_snapshot` 路径、同一 `align_cursors`、同一 `set_track_gain_scales`；
- `any_solo` 仍按**整份快照**判定（逐轨武装不得改变 solo 门控口径：别的轨 solo 时
  本轨照样被静音，那是**正确**的静音，不是丢轨）；
- `track` 不在快照的 `tracks()` 里、或它就是 master 母线 ⇒ 什么都不武装
  （`render_track` 输出静音，与全量武装时该轨拿不到槽位的语义相同）；
- 空快照 ⇒ `SnapshotError::NoMasterBus`（与 `arm_from_snapshot` 同一条拒绝）。

### D2 离线源改走单轨武装 ⇒ 源数与槽位上限解耦

`EngineTrackSource::new` 由 `arm_from_snapshot(&snapshot)` 改为
`arm_track_from_snapshot(&snapshot, track)`。一个源只占一个槽 ⇒
**工程有多少源都不受 16 限制**；这条路径上 `track_drops` 恒为 0。

**实时侧不改**：`MAX_TRACK_SLOTS = 16` 的内存预算是红线 7（回调内禁分配）与
`ARCH-RT-001` 的既成口径，本切片一个字节都不动。

### D3 母带回执新增 `warnings[].code = "silentSources"`

条件（三条同时成立才点名）：

1. `audible` —— `mute` / `solo` 门控下本来就应当静音的源**不**报；
2. `peak == 0.0` —— 源节点输出一个非零样本都没有；
3. `notes > 0 || audio_clips > 0` —— 它**确实排了**会发声的内容（空轨不点名）。

每条点名 `node` / `track` / `kind` / `notes` / `audioClips` / `notesSilenced`，
并给一段可执行的排查提示（solo 门控、推子/声相为 0、路由边断开、采样器缺样本）。
`blocking: false`：**不改样本、不改 `bytes`、不改 `sha256`** —— 渲染结果本身不变，
只是把"看起来成功、其实没声"这件事变成可读的读数。

与既有 `exportClipping` 同走 `data.warnings[]`（与提案响应的 `#175`/`#178`
同名同义）；两条都不出现时仍是空数组。

### D4 不自动修静音源

工具**不**替用户删内容、**不**替用户改路由。静音可能是用户故意的（内容在窗口外、
被 solo 门控），自动"修复"会改写工程语义。点名 + 提示是上限。

## 代价（不假装做过）

1. **实时播放仍受 16 槽限制**：`EngineRuntime` 的槽位预算没有变，>16 轨的实时
   试听仍只有前 16 条轨出声。本切片只修**离线渲染**（`yeban_render_master` 与
   `yeban_render_sfz_wav` 共用的逐轨源路径）。实时扩容涉及预分配内存与红线 7，
   登记为后续切片，不在本轮暗中改动。
2. **`silentSources` 是窗口判据**：内容排在渲染窗口**之外**的轨同样 `peak == 0.0`
   且被点名（判据测试 `a_source_that_renders_silence_in_the_window_is_named_in_a_warning`
   正是这个形状）。这是**有意的**：回执描述的是"这段母带里有没有它"，提示语让
   agent 检查摆放是否越界；它不声称"这条轨坏了"。
3. **不能区分"信号相消"**：若一条轨的输出恰好与别轨逐样本反相相消，其**源节点**
   `peak` 仍非零（源在混音前采样），因此不点名；母带总 `peak` 另有读数。
4. **G4 仍需单独修**：SFZ 全部样本缺失时 `yeban_load_sfz` 仍报告"加载成功"。
   本警告会在**渲染**时点名该源（`notes > 0` 且 `peak == 0`），但加载面的
   成功/失败口径是另一个切片。

## 判据

- `crates/yeban-render/src/engine_source.rs`：
  - `every_track_source_sounds_beyond_the_sixteen_slot_cap`：24 个源逐条渲染，
    每条 `peak > 0`（修复后全绿）；
  - `full_snapshot_arming_is_what_silenced_the_seventeenth_track`（**带牙**的控制组）：
    手工按旧路径全量武装 ⇒ `track_drops() > 0` 且最后一条轨 `peak == 0.0`。
    没有它，"全部出声"可能只是夹具没触发缺陷。
- `crates/yeban-mcp/tests/render_master.rs`：
  - `every_source_in_a_twenty_four_track_project_sounds`：25 条源（夹具自带 Lead
    + 24 轨）全部 `peak > 0`，`warnings[]` 无 `silentSources`；
  - `a_source_that_renders_silence_in_the_window_is_named_in_a_warning`：
    恰一条 `silentSources` 点名 Pad（`count == 1`、`blocking == false`、
    `notes > 0`）；把 Pad 静音后该警告消失（门控静音**不**点名）。
- **确定性**：`PolySynth` 无 RNG，每个槽的合成器独立构造 ⇒ 槽位下标不影响音频；
  既有母带摘要常量（`DEFAULT_MASTER_DIGEST` / `DEFAULT_MASTER_SHA256`）不变。
