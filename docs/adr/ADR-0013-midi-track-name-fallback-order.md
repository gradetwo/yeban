# ADR-0013 — 标准 MIDI 文件（SMF）多轨导入音轨名回退机制与损失读数

- **状态**: **Approved** (经 2026-10-25 常设授权裁决并完全落地 D1/D2/D3)
- **决定者**: 人类负责人（根据 `AGENTS.md` §5.7 规范缺口裁决原则）
- **关联**: `[ARCH-EXT-002]`（MIDI 互操作与导入规范）；`crates/yeban-midi/src/midi.rs`（SMF 解析器）；`crates/yeban-midi/src/import.rs`（多轨拆分规划器 `plan_midi_import`）；`docs/ledger/handover-latest.md` §0.5.8 优先级 7；
- **日期**: 2026-10-26

## 背景

在标准 MIDI 文件（SMF 格式 1）中，每条音符轨道（`MTrk`）的命名通常通过元事件声明。MIDI 1.0 规范定义了以下三种相关的文本元事件：
1. `FF 03` (`MetaMessage::TrackName`)：Sequence/Track Name（音序/音轨名称）
2. `FF 04` (`MetaMessage::InstrumentName`)：Instrument Name（乐器名称，如 "Violin I", "Grand Piano"）
3. `FF 08` (`MetaMessage::ProgramName`)：Program Name（音色/程序名称）

### 现状与缺口

在 `yeban-midi` 当前的实现中：
1. **解析层**（`crates/yeban-midi/src/midi.rs`）：
   - `MetaMessage::TrackName` 既记录计数，又将解析后的结构体 `ParsedTrackName { track, tick, text }` 存入 `ParsedMidi::track_names`；
   - `MetaMessage::InstrumentName` 与 `MetaMessage::ProgramName` 仅做 `aux_events.record()` 计数，其文本载荷当场被丢弃，并未保存在 `ParsedMidi` 中。
2. **多轨拆分规划层**（`crates/yeban-midi/src/import.rs`）：
   - 当 `split_tracks: true` 为含有音符的后续轨道新建音轨时，仅查找该轨的 `track_names`（`FF 03`）。若缺失，则直接回退为机器合成的 `format!("Track {}", track_index + 1)`；
   - 导致大量真实世界中仅使用 `FF 04`（乐器名）标记轨道的制谱/编曲软件导出的 MIDI 文件失去有意义的轨名。
3. **损失审计与判据**：
   - 当前 `STORABLE_KINDS` 为 5 类（`Tempo`, `TimeSignature`, `TrackName`, `Marker`, `KeySignature`）；
   - `InstrumentName` 与 `ProgramName` 当前被判定为无落点（`dropped == read`）。若将其部分或全部采纳为轨名，涉及损失统计与过时判据的联动演进。

---

## 决定建议（Proposed Decisions）

### D1 多轨拆分时的轨名回退链顺序

在 `plan_midi_import` 拆分多轨模式中，新建音轨的名字按照以下优先级顺序确定：
1. **首选 `FF 03` (TrackName)**：若当前 `MTrk` 含有非空、严格 UTF-8 的轨名，优先采用；
2. **次选 `FF 04` (InstrumentName)**：若当前 `MTrk` 无有效 `FF 03`，但含有非空、严格 UTF-8 的乐器名，采用该乐器名；
3. **末选 `FF 08` (ProgramName)**：若前两者均无，采用程序音色名；
4. **兜底机器名**：若三者皆无，保持原有的 `format!("Track {}", track_index + 1)`。

### D2 文本有效性与防冲突原则

- **严格 UTF-8 与非空**：非合法 UTF-8 字节序列或空字符串不视作有效名字，不参与回退匹配。
- **单轨内冲突处置**：若单条 `MTrk` 内出现多条互不相同且非空的同类候选名字（例如两个不同的 InstrumentName），规划器**绝不随意挑选**，判定为有歧义，回退至下一个优先级或机器兜底名。

### D3 损失读数（Losses）精确记账契约

- **采纳记账**：只有当某个 `InstrumentName` 或 `ProgramName` 确实被采纳为工程音轨名（`TrackV3::name`）时，其对应的 `dropped` 计数才减 1（即记录为已落地）；
- **全损保障**：未被采纳的元事件（如被前序优先级覆盖、跨事件冲突、或仅存在于首轨但首轨未采纳），继续如实报告 `dropped`，绝不冒充已处理。
- `STORABLE_KINDS` 适时扩充以反映新落点，更新相关单元测试断言。

### D4 目标音轨（首轨）保持保守

- 导入的第一个音轨直接修改工程既有的落点音轨（`destination_track`）。为保障确定性与最小惊奇原则，首轨的轨名采纳继续遵循现行 `adopt_track_name` 契约（全文件去重、恰有一个全局明确名字时才改写工程轨名），不引入复杂的跨文件乐器名覆盖，将修改范围严格收敛在拆分出的新轨道上。
