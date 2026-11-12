# ADR-0015 — MIDI CC 到轨道自动化泳道映射

- **状态**: Proposed (依 Logic Pro 与 MIDI 1.0 通行规范裁决)
- **决定者**: Agent (依据人类常设授权："遇到需要裁决的、参考 Logic Pro 自己裁决")
- **关联**: `MODEL-AST-002`（`AutomationTarget`、`AutomationLane`、`AutomationPoint`）；
  `crates/yeban-midi/src/midi.rs`（`parse_smf`、`MidiExport`）；
  `crates/yeban-midi/src/import.rs`（`plan_midi_import`）；
  `crates/yeban-midi/src/export.rs`；
  `docs/ledger/handover-latest.md` §0.5.8 第 9 项「CC -> 自动化」。
- **日期**: 2026-10-26

## 背景

在夜半模型层中，每条音轨（`TrackV3`）拥有独立的自动化泳道集合：
`pub automation_lanes: BTreeMap<AutomationTarget, AutomationLane>`。
其中，原生支持的目标包括：
1. `AutomationTarget::TrackVolume { track_id }`：单位为分贝 (`Decibels`)，固有值域 `[-60.0, 12.0]` dB；
2. `AutomationTarget::TrackPan { track_id }`：单位为双极归一化 (`Bipolar`)，固有值域 `[-1.0, 1.0]`。

但在目前的 MIDI 导入导出互操作层（`yeban-midi`）中：
- 导入解析侧（`parse_smf`）：收到 `MidiMessage::Controller` 仅记录 `aux_events.record(MidiAuxEventKind::ControlChange)` 并全量报丢（`dropped == read`），丢弃原因为「no CC-to-automation mapping: automation lanes only target TrackVolume/TrackPan/SendGain/DeviceParam/Macro」；
- 导出侧（`MidiExport`）：仅导出音符、歌词与弯音，不写出任何 CC 事件。

在数字音频工作站（如 Logic Pro、Ableton Live、Cubase）中，MIDI 通道音量（CC 7）与通道声相（CC 10）是最核心的轨道混音与包络控制标准。将 CC 7 与 CC 10 分别映射到音轨的音量与声相自动化泳道，是填补音乐创作缺口、实现高保真工程交换的必要步骤。

## 决定

### D1: 解析层读出 CC 控制改变事件

在 `crates/yeban-midi/src/midi.rs` 中：
1. 新增 `ParsedControlChange`：
   ```rust
   #[derive(Clone, Debug, PartialEq, Eq)]
   pub struct ParsedControlChange {
       pub track: usize,
       pub channel: u8,
       pub tick: u64,
       pub controller: u8,
       pub value: u8,
   }
   ```
2. 在 `ParsedMidi` 中新增字段：
   ```rust
   pub control_changes: Vec<ParsedControlChange>,
   ```
3. 在 `parse_smf` 遇到 `MidiMessage::Controller { controller, value }` 时，将事件推入 `control_changes`，同时保留 `aux_events.record(MidiAuxEventKind::ControlChange)` 计数。

### D2: 导入归属与数值换算映射

在 `crates/yeban-midi/src/import.rs` 的 `plan_midi_import` 中：
1. 识别目标控制器：
   - `controller == 7` (Channel Volume) 映射到目标音轨的 `AutomationTarget::TrackVolume { track_id }`；
   - `controller == 10` (Pan) 映射到目标音轨的 `AutomationTarget::TrackPan { track_id }`；
   - 其余控制器（CC 1 调制、CC 64 延音踏板等模型中尚未建立持久化字段的控制器）不映射，保持 `dropped` 并提供精确原因。
2. 音轨归属：
   - 若 `options.split_tracks == true`，按 `track_destinations` 查找 CC 所在 `MTrk` 对应的 `track_id`；
   - 否则统一映射到 `dest_track_id`。
3. 数值换算（遵循标准声学与 MIDI GM 规范）：
   - **CC 7 音量转分贝**：
     MIDI 标准 GM 音量公式为 $gain = (v / 127.0)^2$。换算成分贝：
     $$\text{dB} = \begin{cases} -60.0, & v = 0 \\ \operatorname{clamp}\left(40.0 \times \log_{10}(v / 127.0), -60.0, 12.0\right), & v > 0 \end{cases}$$
     - 当 $v = 127$ 时，$\text{dB} = 0.0$ dB；
     - 当 $v = 100$（MIDI 规范默认音量）时，$\text{dB} \approx -4.15$ dB；
     - 当 $v = 0$ 时，$\text{dB} = -60.0$ dB。
   - **CC 10 声相转双极标量**：
     MIDI 声相范围 $0 \dots 127$，中点为 64：
     $$pan = \begin{cases} \frac{v - 64.0}{64.0}, & v \le 64 \\ \frac{v - 64.0}{63.0}, & v > 64 \end{cases}$$
     - 当 $v = 0$ 时，$pan = -1.0$（极左）；
     - 当 $v = 64$ 时，$pan = 0.0$（居中）；
     - 当 $v = 127$ 时，$pan = 1.0$（极右）。
4. 泳道生成与去重：
   - 将换算后的点以 `scale_ticks(cc.tick, ppq)` 挂入对应轨道的 `AutomationLane`；
   - 同一 tick 内若有多个相同目标的 CC 事件，后到者覆盖先前者，保持点按 tick 严格递增；
   - 曲线类型为 `CurveType::Linear`；
   - 泳道通过 `Op::SetAutomationLane` 挂载到音轨。
5. 损失守恒与分桶：
   - `MidiAuxEventKind::ControlChange` 计入 `STORABLE_KINDS`；
   - 映射成功的 CC 7 与 CC 10 事件计入 `attached`；
   - 未映射的 CC 事件计入 `dropped`；
   - 满足 `read == attached + dropped` 守恒断言。

### D3: 导出层写出 CC 7 与 CC 10

在 `crates/yeban-midi/src/midi.rs` 中：
1. `RawEvent` 引入 `ControlChange`:
   ```rust
   ControlChange {
       tick: u64,
       channel: u8,
       controller: u8,
       value: u8,
   }
   ```
2. 排序全序 (`tie_break`):
   在同一 tick 中，`ControlChange` 排在 `Lyric` 之后、`NoteOn` 之前，确保在音符发声前控制器状态已就绪。
3. 导出逆向换算：
   - `TrackVolume` (dB) $\to$ CC 7：
     $$v = \operatorname{round}\left(127.0 \times 10^{\frac{\text{dB}}{40.0}}\right).\operatorname{clamp}(0, 127)$$
   - `TrackPan` $\to$ CC 10：
     $$v = \operatorname{round}\left(\text{if } pan \le 0.0 \{ 64.0 + pan \times 64.0 \} \text{ else } \{ 64.0 + pan \times 63.0 \}\right).\operatorname{clamp}(0, 127)$$
4. 逐点输出为 `TrackEventKind::Midi { channel, message: MidiMessage::Controller { controller, value } }`。

### D4: 其余全部控制器进各自的 `MidiCc` 泳道（2026-11-12 追加）

D2 只给了 CC 7 / CC 10 落点，其余控制器（CC 1 调制、CC 11 表情、CC 64 踏板…）
仍在导入时丢弃。`AutomationTarget::MidiCc { track_id, controller }` 自 `#341`
起已存在（值域原始 `0..=127`，渲染侧 `CcProbe` 逐块求值进引擎），缺的只是
导入/导出两端的接线：

1. 导入（`plan_cc_lanes`）：CC 7 / CC 10 仍走 D2 的声学换算（向后兼容）；
   其余控制器号一律进各自的 `MidiCc` 泳道，值取**原始 7 位值**（⛔ 不换算、
   不归一化 —— 渲染侧与 SFZ 的 `*_onccN` / `loccN` / `hiccN` 本来就以原始值
   为域，归一化只会引入舍入）。音轨归属、同 tick 后到者覆盖、并入已有泳道、
   `CurveType::Linear` 与 D2 同口径；`attached` 数全部映射事件，
   `ControlChange` 的 `dropped` 自此恒为 0。
2. 导出（`track_control_changes`）：`MidiCc` 泳道逐点写回同号 CC
   （四舍五入后钳 `0..=127`；`read_enabled = false` 不写，与 D3 同口径）。
   CC 7 / CC 10 不走这里（它们只活在音量/声相泳道里，⛔ 不双写）。
   整数值往返恒等；手画的非整数点按就近格取整。

代价：`MidiCc` 泳道是按控制器号分的"一号一条"，文件里 20 个控制器号即 20 条
泳道 —— 这是模型既有的寻址方式（`AutomationTarget` 以控制器号为身份），
不是本决定新增的。

## 影响

- 表现力：MIDI 文件中包含的音量渐变（Volume swell）与左右声相摆动能够无缝还原为夜半 DAW 的原生混音包络；
- 确定性与可逆性：CC 7 与 CC 10 在夜半 DAW 与标准 DAW 之间实现高精度往返转换；
- 兼容性：未包含 CC 自动化泳道的现有工程导出字节保持不变。
- D4 之后：导入不再丢弃任何 CC（踏板、表情、调制数据全部进泳道）；CC64 的
  保持语义由 ADR-0071 另行裁决（D4 只解决"数据进泳道"，不解决"踏板保持"）。
