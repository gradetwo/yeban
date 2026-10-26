# ADR-0021 — 母带渲染真的读 `tempo_map`（分段常速回放）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MODEL-AST-002`（960 PPQ 与时间轴）、`ARCH-DET-001`（L1 确定性）、
  `ARCH-DET-002`（累加顺序）、`MCP-TOOL-008`（`yeban_render_master`）；
  `crates/yeban-model/src/time_math.rs`（`TempoMap`）、
  `crates/yeban-model/src/project.rs`（`YebanProjectV1::tempo()`）、
  `crates/yeban-engine/src/{snapshot,audio_clip,synth}.rs`、
  `crates/yeban-render/src/{engine_source,automation}.rs`、
  `crates/yeban-mcp/src/domain/{render,render_math,render_clip_math,sfz}.rs`；
  `crates/yeban-engine/tests/tempo_map_schedule.rs`
- **日期**: 2026-10-27

## 背景

`project.tempo_map` 与 `project.time_signature_map` 早已能写入：
`Op::SetTempoPoint` / `Op::SetTimeSignaturePoint`、MCP 的 `setTempoPoint` /
`setTimeSignaturePoint`、MIDI 导入的速度点位采纳都会落表。但渲染器与引擎的
tick → 帧换算**只乘 `project.bpm`** ⇒ 变速工程在母带里**静默无效**：表写进去了、
`yeban_query_project` 也读得到，导出的音频却按单一速度铺满整首。

这是"创作音乐功能缺口"，不是精度问题：agent 用 `setTempoPoint` 做半速/双速段落，
得到的母带与请求不符，而且没有任何错误或警告。

## 决定

### D1 `TempoMap` 是时间换算的**唯一**模型层投影

`yeban_model::time_math::TempoMap { base_bpm, points: BTreeMap<u64, f64> }`：

- `base_bpm` 取自 `YebanProjectV1::bpm`（tick 0 的速度）；
- 点是**分段常速**：`bpm_at_tick(t) = points.range(..=t).next_back()`，
  没有点 ⇒ `base_bpm`；
- `YebanProjectV1::tempo()` 是唯一构造入口（`base_bpm = bpm`，点表 clone）。

### D2 正向：**先累加未舍入的帧数，最后只舍入一次**

`frames_at_tick(tick)`：把 `[0, tick)` 按速度点切成段，每段用
`delta_ticks × 60 × sr / (bpm × ppq)` 求**未舍入**的帧数并累加，最后 `round` 一次。
不逐段舍入：`N` 个速度点最多引入 `N` 帧的漂移，且漂移量随点表长度变化 ——
那不是"分段常速"，是"分段取整"。

**空表必须与 `ticks_to_frames` 逐位相同**（直接委托给同一个函数）：
既有工程、既有母带摘要常量一位都不许动。

### D3 反向：扫描分段 + 段内取整

`ticks_at_frame(frame)` 找出该帧落在哪一段，段内用
`frame_delta × bpm × ppq / (60 × sr)` 向下取整后与段 tick 宽度取 `min`；
空表与既有 `yeban-render::automation::tick_at_frame` 的算式**同一条、同一个顺序**
（该函数保留为恒定速度的适配器，判据仍钉着它）。

### D4 离线专用入口；实时走带**不改**

- 新增 `EngineSnapshot::from_project_with_tempo(project, revision, &TempoMap)`；
- `from_project` / `from_project_with_limiter` / `from_project_with_latencies`
  **继续**按 `project.bpm` 恒定速度投影；
- `yeban-render::EngineTimbre::{from_project, from_project_armed}` 改走
  `from_project_with_tempo(project, 1, &project.tempo())` ⇒ **MCP 母带渲染**拿到
  分段常速；空表与接线前逐位相同；
- `TrackGainAutomation` / `TrackPanAutomation` / 设备参数自动化的
  `bpm: f64` 字段换成 `tempo: TempoMap`，块起点 tick 走
  `ticks_at_frame(...).unwrap_or(0)`（空表 ⇒ 与旧 `tick_at_frame` 逐位相同）。

理由：实时走带的**位置推进**是一帧一帧的绝对帧，节拍器、循环边界、UI 拍格都按
恒定 BPM 计算。只在快照里放开变速会让"听到的时间"与"走带显示的时间"分叉，
比两边都恒定更糟。实时侧对齐是**登记的后续项**（见 D5）。

### D5 引擎与 MCP 的覆盖范围

- `project_schedules`：音符起止、rattach 子脉冲、循环副本全部走
  `TickTimebase::sample_at`；弯音/滑音**局部时间**以该音符首次出现的绝对 tick 为
  锚点做 `frames_at_tick(anchor + local) − frames_at_tick(anchor)`（空表仍走
  `tick_to_sample`，逐位不变）。
- `project_audio_clips_with_tempo`：摆放起止、循环区间、`base_end` 全部走同一时基。
- MCP `render.rs`：总帧数缺省推导、采样器 `note_frame_span`、音频片段
  `clip_frame_span`、循环展开的四次 tick→帧换算全部改读 `project.tempo()`。
- MCP `sfz.rs::collect_notes`（`yeban_render_sfz_wav` 的排程）同样改读。
- `render_math::{frames_at_tick, note_frame_span_with_tempo}` 与
  `render_clip_math::clip_frame_span_with_tempo` 是新的纯逻辑入口；旧签名保留为
  `TempoMap::constant(bpm)` 的包装（验证脚手架 `verify/render_pure.rs` 与既有
  单元判据因此继续有效）。

## 诚实的边界（不假装做过）

1. **实时走带尚未变速**：`EngineRuntime` 的位置推进、节拍器
   （`MetronomePlan`）、循环边界、UI 拍格仍按 `project.bpm` 恒定速度。
   `yeban-app` 的 `render_core` / `engine_host` 仍调用 `from_project`。
2. **`time_signature_map` 仍不可闻**：本仓库没有"拍号影响 tick 时间"的语义
   （拍号只影响拍格显示与节拍器重音），因此不存在需要接的 tick→帧路径。
   若将来接入节拍器重音，另立切片。
3. **`yeban_import_audio` 的 `durationTicks` 推导**仍按 `project.bpm` 单速
   （`frames_to_ticks_ceil`）。在变速工程里导入音频且不显式给时长时，
   推导值与摆放起点的速度段无关 —— 登记为待办，本轮不改。
4. **弯音锚点**：变速点落在**单个音符内部**时，弯音曲线的局部帧间距按音符
   **首次出现**处的速度段换算，而不是逐点按绝对位置重算。这是"一个音符共享一份
   形状表"（`#311b`）的既有结构决定的；差一个变速点落在音符内部的情形，
   登记为已知近似。

## 判据

- `crates/yeban-engine/tests/tempo_map_schedule.rs`：
  T1 空表与 `from_project` 逐项相同；T2 变速点后音符 tick 1920 ⇒ 36 000 帧
  （恒定 120 BPM 是 48 000）；T3 音频摆放同样移动；T4 端到端 36 000 帧前完全静音。
- `crates/yeban-model/src/time_math.rs`：空表逐位等价、分段常速、正反互换、
  病态点拒绝。
- `crates/yeban-mcp/verify/render_pure.rs`：既有 53 条纯逻辑判据（旧签名包装）
  继续在裸 `rustc` 下通过。
- 既有引擎/渲染测试是**空表逐位不变**的回归闸：任何"空表也改了旧路径"的改动
  都会让母带摘要常量变红。
