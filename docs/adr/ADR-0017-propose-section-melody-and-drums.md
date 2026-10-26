# ADR-0017 — `yeban_propose_section` 旋律与鼓组理论生成扩展（`includeMelody` 与 `includeDrums`）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section` 契约）；`schemas/mcp-tools.schema.json`；
  `crates/yeban-mcp/src/domain/section.rs`、`crates/yeban-mcp/src/domain/section_build.rs`；
  `crates/yeban-theory/src/drum.rs`（`DrumPattern` / `DrumVoice` / `swung_styled_drum_pattern`）、
  `crates/yeban-theory/src/melody.rs`（`melody_over_chords` / `MelodyConstraints`）、
  `crates/yeban-theory/src/genre.rs`（`GenreRule::drum_pattern` / `GenreRule::rhythm_grid`）；
  `crates/yeban-mcp/src/domain/ids.rs`（`deterministic_id`）；
  `MODEL-AST-001`（960 PPQ 整数网格）、`ARCH-DET-*`（声学确定性契约）、
  `WALL ③` / `WALL ④`（`creation_acceptance.rs` 旋律与鼓点生成缺口）
- **日期**: 2026-10-27

## 背景

在 `ADR-0016`（`materialSource: 'theory'`）落地后，`yeban_propose_section` 已经能够
在空工程上从零展开罗马数字和弦走向并生成确定性的 3/4 声部配器骨架（`Bass / Alto / Soprano`
或 `Bass / Tenor / Alto / Soprano`）。

然而，一首完整的现代音乐段落（Intro / Verse / Chorus / Drop）不仅包含和声声位连接，
还需要节奏骨架（鼓组）与旋律线条（主音/副旋律）。在 `crates/yeban-mcp/tests/creation_acceptance.rs`
的墙清单中明确记录了两大创作痛点：
- **WALL ③**：旋律自动生成（`melody_over_chords`）未接进任何 MCP 工具，AI 只能手写旋律音符或调用低阶 `yeban_edit_notes`；
- **WALL ④**：鼓组自动生成（`styled_drum_pattern`）未接进任何 MCP 工具，AI 只能手写鼓点。

在 `yeban-theory` 中，两项生成能力早已完整实现且全部为确定性纯函数：
1. `GenreRule::drum_pattern(bars, onsets_per_bar)` / `swung_styled_drum_pattern`：依据流派拍号、摇摆比例与鼓组风格生成网格击点，并映射到通用 MIDI（GM）鼓键（底鼓 36、军鼓 38、踩镲 42、吊镲 51）；
2. `melody_over_chords(&key, &spans, &grid, constraints, seed)`：在目标调式与和弦跨度（`ChordSpan`）上，依据流派节奏网格生成符合音域与最大跳进约束的旋律线条。

缺口仅在于 MCP 层 `yeban_propose_section` 尚未提供 opt-in 入口把旋律与鼓组作为扩展声部包含进段落提案中。

## 决定

### D1 入参：`includeDrums` 与 `includeMelody`

为 `yeban_propose_section` 新增两个**可选布尔参数**：

1. **`includeDrums`**（`boolean`，缺省 `false`）：
   - 当为 `true` 时，在段落中自动追加一条鼓组打击乐音轨 `"{section_name} · Drums"`；
   - 鼓点内容由 `GenreRule::drum_pattern`（或 `swung_styled_drum_pattern`）生成；
   - 击点映射为标准的 GM 打击乐音符：
     - `DrumVoice::Kick` -> MIDI pitch 36
     - `DrumVoice::Snare` -> MIDI pitch 38
     - `DrumVoice::HiHat` -> MIDI pitch 42
     - `DrumVoice::Ride` -> MIDI pitch 51
   - 时值固定为 120 ticks（1/32 音符），力度按 `accent` 分级（重音 112，普通 96）；
   - 该音轨通过 `Op::AddRoutingEdge` 直连主总线（`master_bus_track_id`），保证整图为 DAG。

2. **`includeMelody`**（`boolean`，缺省 `false`）：
   - 当为 `true` 时，在段落中自动追加一条旋律主音乐轨 `"{section_name} · Melody"`；
   - 旋律内容由 `melody_over_chords` 基于当前段落展开的和弦区段（`ChordSpan`）、目标调式与流派节奏网格生成；
   - 音高符合 `MelodyConstraints::DEFAULT`（60..=84，中央 C 到高八度 C，最大跳进 7 半音）；
   - 时值与起点与网格 onset 严格对齐，力度按 `chord_tone` 分级（和弦音 112，经过音 96）；
   - 该音轨通过 `Op::AddRoutingEdge` 直连主总线，保证整图为 DAG。

### D2 兼容性契约（逐字节向后兼容）

- `includeDrums` 与 `includeMelody` 缺省均为 `false`（或 `None`）；
- 当两项均未指定或为 `false` 时，`plan` 产出的 `SectionPlan`、操作列表 `ops`、声部音轨与身份与原有逻辑**逐字节相同**；
- 若 `materialSource: "project"` 且显式指定 `includeDrums: true` 或 `includeMelody: true`，返回 `INVALID_PARAMETER_RANGE` 并给出明确的 `hint`："旋律与鼓组理论生成仅在 materialSource: 'theory' 时支持"。

### D3 确定性身份派生与整图无环

- 鼓组音符身份：`deterministic_id(&format!("theory-drum:{section_id}:{hit_index}:{voice}"))`；
- 旋律音符身份：`deterministic_id(&format!("theory-melody:{section_id}:{note_index}"))`；
- 音轨与摆放身份遵循既有的 `deterministic_id(&format!("part:{section_id}:{index}:{part_name}"))` 体系；
- 路由边身份为 `deterministic_id(&format!("edge:{section_id}:{index}:{part_name}:{bus}"))`；
- 施加后通过 `detect_cycle` 校验全局路由图无环。

### D4 错误处理契约

- 理论生成中的任何错误（走向无法展开、节奏网格非法、和弦音生成失败等）统一捕获并映射为 `BuildCode::InvalidParameterRange`（MCP 错误码 `INVALID_PARAMETER_RANGE`）；
- 错误负载中附带 `theoryStep`（如 `"GenreRule::drum_pattern"` 或 `"melody_over_chords"`）与 `theoryError` 详细说明，严禁泄漏未经处理的底层 `TheoryError`。
