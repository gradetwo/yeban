# ADR-0014 — MIDI 导入与导出弯音归属与线格式转换

- **状态**: Proposed (依 Logic Pro 与 MIDI 1.0 通行规范裁决)
- **决定者**: Agent (依据人类常设授权："遇到需要裁决的、参考 Logic Pro 自己裁决")
- **关联**: `MODEL-AST-005`（`MidiNote::pitch_bend_curve`）；`ADR-0012`（弯音曲线坐标、形状与校验）；
  `crates/yeban-midi/src/midi.rs`（`parse_smf`）；
  `crates/yeban-midi/src/import.rs`（`plan_midi_import`）；
  `crates/yeban-midi/src/export.rs`（`export_midi`）；
  `docs/ledger/handover-latest.md` §0.5.8 第 8 项「弯音归属」。
- **日期**: 2026-10-26

## 背景

夜半模型层已在 `MidiNote` 中支持 `pitch_bend_curve: Vec<(u64, i16)>`，并在 `ADR-0012` 中严格立法：
- `tick` 是**音符局部**坐标（`0` = 音符起点 `start_tick`）；
- `cents` 相对音高，闭区间 `[-1200, 1200]`；
- tick 严格升序，线性插值，末点之后保持末值。
且母带渲染与 DSP 实时引擎均已对其进行物理求值（`#311b`）。

然而在 MIDI 文件互操作层（`yeban-midi`）：
1. 导入解析侧（`parse_smf`）：收到 `MidiMessage::PitchBend` 仅记录 `aux_events.record(MidiAuxEventKind::PitchBend)` 并全量报丢，报文为「a channel pitch bend applies to every sounding note; there is no honest per-note attribution rule」；
2. 导出侧（`export_midi`）：注释显式声明 `pitch_bend_curve` 不导出。

由于现代数字音频工作站（如 Logic Pro）在单音旋律或独奏轨中大量使用弯音，全量丢失导致音乐表现力严重折损。

## 决定

### D1: 解析层读出 14-bit 弯音事件

在 `crates/yeban-midi/src/midi.rs` 中：
1. 新增 `ParsedPitchBend`：
   ```rust
   pub struct ParsedPitchBend {
       pub track: usize,
       pub channel: u8,
       pub tick: u64,
       pub cents: i16,
   }
   ```
2. 标准 MIDI 默认弯音范围为 ±2 半音（±200 cents），中点为 `8192`（0x2000）：
   $$\text{cents} = \left(\frac{(v - 8192) \times 200 + 4096}{8192}\right) \text{ 截断并限制在 } [-1200, 1200]$$
3. 在 `ParsedMidi` 中新增 `pub pitch_bends: Vec<ParsedPitchBend>`。

### D2: 导入归属逻辑 —— 单发声「唯一才归属」

在 `crates/yeban-midi/src/import.rs` 的 `plan_midi_import` 中：
1. 遵循歌词归属（`attach_lyrics`）已确立的「唯一才归属」原则：
   对于轨 $T$、通道 $C$ 上的弯音事件 $(T, C, \text{tick}, \text{cents})$：
   查找在当前轨与通道上处于鸣响区间（`note.start_tick <= tick < note.end_tick`）的音符集合。
2. 若鸣响音符数**恰为 1**（单发声旋律线）：
   - 该弯音事件唯一归属于该音符；
   - 局部坐标计算：`rel_tick = scale_ticks(bend.tick, ppq) - scale_ticks(note.start_tick, ppq)`；
   - 插入该音符的 `pitch_bend_curve`，遵循 `ADR-0012`（保持 tick 严格升序；同 tick 覆盖更新）；
3. 若鸣响音符数为 0（空档期）或 >1（和弦复音复用单通道）：
   - 无法诚实判定归属，记入丢失（`dropped`）；
4. 统计与分桶：`read == attached + dropped` 守恒断言。

### D3: 导出层支持弯音序列写回

在 `crates/yeban-midi/src/export.rs` 的 `export_midi` 中：
1. 遍历音轨音符，当音符拥有非空 `pitch_bend_curve` 时：
   对每个 `(note_tick, cents)`：
   - 绝对 tick：`abs_tick = note.start_tick + note_tick`；
   - 换算为 14-bit MIDI bend：
     $$v = \left(8192 + \frac{\text{cents} \times 8192}{200}\right) \text{ clamp 至 } [0, 16383]$$
   - 插入 `MidiMessage::PitchBend` 事件；
2. 若曲线末点弯音不为 0 cents，在音符结束绝对时刻 `note.start_tick + note.duration_ticks` 插入一条中点（8192，即 0 cents）复位弯音事件，防止弯音泄漏到通道后续音符。

## 影响

- 兼容性：所有既有无弯音 MIDI 文件导入导出逐字节等价；
- 确定性：归属算法为纯函数，跨架构和操作系统保持一致；
- 往返保真度：带弯音的单音音符在夜半与标准 DAW 之间实现高保真互操作。
