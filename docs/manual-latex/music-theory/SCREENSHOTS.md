# T-DOC-003 Music Theory — screenshot inventory

Management captures the real UI shots and replaces the placeholders under
`music-theory/figures/` using the same names. Nine chapters, nine unique
screenshot names per language; no duplicate names across chapters.

| Name | Chapter | Chinese caption | English caption |
|---|---|---|---|
| `piano-roll-pitch-labels` | CH01 | 钢琴卷帘音高标签与钢琴键盘对照 | Piano-roll pitch labels against the piano keyboard |
| `piano-roll-rhythm-grid` | CH02 | 钢琴卷帘节奏网格与拍号 | Piano-roll rhythm grid and time signature |
| `piano-roll-interval-two-notes` | CH03 | 卷帘上的两个音与音程行距 | Two notes and their interval spacing in the piano roll |
| `piano-roll-scale-ladder` | CH04 | 卷帘中的音阶阶梯 | Scale ladder in the piano roll |
| `piano-roll-c-major-triad` | CH05 | C 大三和弦在卷帘中的叠置 | C major triad stacked in the piano roll |
| `piano-roll-ii-v-i` | CH06 | ii–V–I 进行在卷帘中的排列 | ii–V–I progression laid out in the piano roll |
| `piano-roll-four-bar-question-answer` | CH07 | 四小节问句与答句 | Four-bar question and answer |
| `arrangement-aaba-structure` | CH08 | 编曲区 AABA 段落结构 | AABA section structure in the arrangement view |
| `piano-roll-staff-mapping` | CH09 | 谱面音名与卷帘行号对照 | Staff note names mapped to piano-roll rows |

Notes:

- All placeholders use the shared `\screenshot{name}{caption}` command from
  `common/yeban-manual.sty`; the caption and `fig:<name>` label are generated
  by that command.
- `piano-roll-staff-mapping` is the CH09 cross-reference image requested by
  Management; it must show a staff note name next to the matching piano-roll row.
- No screenshot names overlap with the other four volumes at this baseline.
