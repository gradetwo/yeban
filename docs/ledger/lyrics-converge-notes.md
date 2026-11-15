# 歌词汇聚工作线台账（`line/lyrics-converge`）

- **台账类型**：小设计说明 + 交付映射（**不是规范**）。
- **工作线**：`line/lyrics-converge`，基线 `origin/main` `52a9c3149`。
- **所有者目录**：`crates/yeban-model/src/lyrics.rs`、`crates/yeban-app/src/lyrics_export.rs`。

## 1. 问题

`--export-lyrics` 只读工程歌词时间轴 `YebanProjectV1::lyrics`。
`MidiNote::syllable` 不汇聚。
只用音节写词的工程导出结果是 `lines=0`。
完整歌词导不出来。
外评 C 条已核实属实（见 2026-11-15 记忆日志）。

## 2. 设计（实现前定稿，实现照此执行）

### 2.1 汇聚来源与优先级

汇聚有两个来源。

1. 工程歌词时间轴 `project.lyrics`。
   它是行级权威来源。
   它原样进入导出。
   它不被改写。
2. 音符音节 `MidiNote::syllable`。
   它只经片段摆放进入汇聚。
   未被任何摆放引用的片段不汇聚。
   原因是未摆放的片段不在工程时间轴上发声。

优先级只有一条：时间轴高于音节汇聚。
冲突取舍见 §2.3。

存储口径不变。
时间轴仍是唯一落盘的歌词结构。
汇聚只发生在导出时。
汇聚不写回工程。
`lyrics_to_lrc` 的既有语义不变。
汇聚走新入口 `converged_lyric_lines` 与 `lyrics_to_lrc_converged`。

### 2.2 音节的收集与排序规则

收集顺序按以下规则执行。

1. 音轨按显示顺序遍历。
   显示顺序由 `YebanProjectV1::display_track_ids` 给出。
   主总线不在显示顺序内，不汇聚。
2. 每条轨内，摆放按 `(start_tick, id)` 升序遍历。
3. 静音摆放跳过。
   静音摆放不发声，不导出它的音节。
4. 轨的静音与独奏状态不参与判定。
   导出是文档导出，不是试听混音。
5. 摆放只取 MIDI 片段。
   片段缺失时跳过该摆放。
   合法工程不会出现片段缺失。
6. 片段内每个音符按 `ClipPlacement::note_copies` 展开。
   循环摆放的重复遍次各自产生一次出现。
   每次出现的绝对 tick 是 `placement.start_tick + copy_start_tick`。
   计算用饱和加法。
7. `micro_timing_ticks` 不参与汇聚位置。
   它是演奏偏移，不是歌词位置。
8. `probability` 与 `ratchet` 不参与汇聚。
   音节是已写下的词，不是触发结果。

### 2.3 分行规则

音节本身没有行边界。
本设计不发明小节或段落边界。
分行只用一条时间规则。

1. 同一摆放内，出现按 `(绝对 tick, 音符 id)` 升序排列。
2. 相邻两个出现合成一行，当且仅当后一个出现的起点不晚于
   前一个出现的终点加 `LYRIC_LINE_GAP_TICKS`。
   `LYRIC_LINE_GAP_TICKS` 是 960 tick，即一拍。
   前一个出现的终点是它的起点加时值。
3. 间隔超过一拍时断行。
   新行的起始 tick 是该行第一个出现的绝对 tick。
4. 一行内的音节用一个空格连接。
   该口径与时间轴既有文本一致，如 `昨日 你 信誓旦旦`。
5. 不同摆放不合并成一行。
   不同音轨不合并成一行。

### 2.4 空音节规则

以下音节视为无音节：`None`、空串、纯空白串。
无音节的音符不产生出现。
无音节的音符不产生空行。
无音节的音符不参与分行间隔计算。
有内容的音节先去掉首尾空白再使用。

### 2.5 与时间轴冲突时的取舍

1. 音节汇聚行与任一时间轴行起始 tick 完全相同时，丢弃该汇聚行。
   时间轴在该时刻已有权威整行。
   再导出一行会产生同一时间戳的重复行。
2. 起始 tick 不完全相同时，两者都保留。
   两者可能不是同一份词。
   静默丢弃会丢失信息。
3. 文本相同不构成冲突。
   副歌在时间轴中本来就会以相同文本重复出现。
4. 合并后按起始 tick 升序排列。
   同一 tick 时时间轴行排在汇聚行之前。
   其余同 tick 行保持汇聚时的收集顺序。
   排序是稳定排序，结果确定。

### 2.6 输出格式不变的承诺

1. LRC 格式不变。
   头标签仍是 `[ti:]`、`[ar:]`、`[by:Yeban]`、`[offset:0]`。
   给出总时长时才写 `[length:]`。
   行格式仍是 `[mm:ss.cc]文本`。
2. `--export-lyrics` 开关不变。
   回执前缀仍是 `exported-lyrics:`。
   回执行字段顺序仍是 `path`、`bytes`、`lines`。
3. `lines` 的计数口径扩展为汇聚后的实际写出行数。
   它仍是实测读数，不从入参抄。
4. 无时间轴且无音节的工程仍只导出头标签。
   它的回执仍是 `lines=0`。
5. 只有时间轴的工程导出逐字节不变。
   前提是该工程没有带音节的已摆放音符。
   有带音节音符时，新增行就是本线的预期行为变化。

## 3. 实现映射

| 文件 | 内容 |
| :--- | :--- |
| `crates/yeban-model/src/lyrics.rs` | 新增 `LYRIC_LINE_GAP_TICKS`、`converged_lyric_lines`、`lyrics_to_lrc_converged`；`lyrics_to_lrc` 与汇聚导出共用同一份 LRC 渲染 |
| `crates/yeban-app/src/lyrics_export.rs` | `export_lyrics` 改走汇聚导出；回执 `lines` 取汇聚行数 |
| `crates/yeban-model/tests/lyrics.rs` | 模型层判据：仅音节、两者并存且同 tick 时间轴优先、空音节不产生行、间隔断行 |
| `crates/yeban-app/tests/lyrics_export.rs` | 端到端判据：仅音节工程导出完整歌词、两者并存、空音节不产生空行 |

## 4. 不做事项

- 不新增 MCP 工具。
- 不新增警告码。
- 不新增模型字段。
- 不改 LRC 解析 `parse_lrc`。
- 不改时间轴的校验规则 `validate_lyrics`。
