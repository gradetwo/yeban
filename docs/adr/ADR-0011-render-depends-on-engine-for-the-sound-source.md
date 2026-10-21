# ADR-0011 — 让 `yeban-render` 依赖 `yeban-engine`（`default-features = false`），把引擎音源接进 MCP 导出

- **状态**: **Accepted（负责人 2026-10-21 逐字裁决「统一」）**
- **决定者**: 负责人（Agent 提出并实施）
- **关联**: `docs/adr/ADR-0001` **D19**（本次裁决**扩展**它，见下）；
  `scripts/gates/check_mcp_dependency_direction.py` 的 `(yeban-mcp, yeban-render)` 例外（本 ADR **不新增**例外）；
  勘察报告 ＝ 任务单 `#250`（事实源）；差量读数 ＝ `#253` 的 `C1` 判据
- **日期**: 2026-10-21

## 背景

同一份工程有两条导出路径，而 MIDI 音源不同：

- CLI 与界面走 `EngineTrackSource` → `SynthEngine`（加法波表 `HOLLOW` ＋ ADSR ＋ 双振荡器）；
- MCP（agent 用）走自己的 `MidiSynthSource`（**纯正弦** ＋ 线性 5 ms/10 ms 包络）。

`#253` 用 `C1` 判据量出了差量，并且量出了**其余一切逐位相同**：

| 夹具 | 结果 |
| :--- | :--- |
| 纯音频（float32） | ⭐ `masterDigest` **逐位相同**（`0/64` 差异） |
| 纯音频（16-bit，`#254` 修完归一化后） | ⭐ 同样**逐位相同** |
| 纯 MIDI | ⚠ 差 `63/64` 字符；**峰值只差 `0.1512%`**、max `0.59` 满幅、RMS `−16.79 dBFS` |
| 混合 | 差 `58/64`，且与纯 MIDI 只差 `0.83%`（音频部分逐位抵消） |

⇒ 差量**只是一个变量**：MIDI 音源。

`MidiSynthSource` 自己的注释把理由写成"与块边界/线程数构造性无关"。那是真的，但**不是它存在的理由** ——
`SynthEngine` 自己就有逐位确定性判据（`synth_render.rs` 的两次独立装配、`block_length_extremes.rs` 的九种块长）。
真正的理由是**依赖方向**：`yeban-mcp` 不能依赖 `yeban-engine`。

## 决定

### D1 负责人的裁决

负责人 2026-10-21 对"要不要把两条音源统一"逐字回答 **「统一」** ⇒ 让 MCP 也用引擎音色。

### D2 路线：`yeban-render` 长出音源

给 `yeban-render` 加一条 `yeban-engine = { workspace = true, default-features = false }` 依赖，
把引擎音源适配器放进 `yeban-render`，`yeban-mcp` 经**已登记**的 `(yeban-mcp, yeban-render)` 例外使用它。

**不新增任何依赖例外。** `yeban-mcp` 仍然不直接依赖 `yeban-engine`。

### D3 这条边**超出 `ADR-0001` D19 的原意**

`D19` 逐字授权的是"`yeban-render` 以 `yeban-engine = { …, default-features = false }` 依赖它，
**于是离线渲染拿到同一份算法**而不编译 `cpal`"，上下文是**PDC 算法**。

本 ADR 用同一条边**装音源**，那是 `D19` 没有讨论过的用途。**记录在案，不追认 `D19`**。

### D4 实测代价（`#250` 的读数，非估算）

| 项 | 实测 |
| :--- | :--- |
| 依赖闭包差 | ⭐ **只有 `yeban-engine v0.0.1` 一行** |
| 新增第三方包 | ⭐ **0** |
| `cpal`（单编 `-p yeban-mcp`） | ⭐ **不编**（`default-features = false`） |
| `cpal`（`--workspace` 全量） | ⚠ **照编**（`resolver = "3"` 会统一特性；今天也为 `yeban-app` 编） |

⚠ **最后一行是条件，不是细节**：本 ADR **不**声称"整仓不编 `cpal`"。

### D5 后果

- ✅ MCP 导出的 MIDI 音色与界面/CLI **一致** ⇒ `C1` 的 B/C 夹具应从"已知不同"变成"相同"。
- ⚠ **MCP 的既有钉死常量会变**：`masterDigest`、文件 `sha256`、`clippedSamples`、`sources[].kind` 的取值。
  三件套（旧值 → 新值 → 原因）必须逐条写。
- ⚠ **`unsupported` 的 `sfz*` 家族要重算**：引擎自带 SFZ 支路 ⇒ 现有 16 项里若干要退出。
  ⛔ 不得为了"清单变短"而删掉仍然为真的键（第 324 条）。
- ⚠ **`MidiSynthSource` 退役**：它是本 ADR 要取代的东西，留着就是第二份音源。
- ⚠ **导出会变慢**：引擎音色比纯正弦重。这是负责人已知并接受的代价。

### D6 不在本 ADR 范围

- **实时侧**：`yeban-engine` 的实时路径**不动**（它本来就用引擎音色）。
- **`--sfz` 旁路与最小音源**（`track_notes_in_frames`）：它们**已经**吃 `ratchet`/`micro_timing`/`probability`/循环，
  且**不走** `MidiSynthSource` ⇒ 不在本 ADR 的替换面里。
- **容器／抖动／采样率**的差异：那是 `#250` 列的另外几条，**刻意不合并**（判据只比 `masterDigest`）。

## 未决

1. 引擎音源适配器落在 `yeban-render` 的**哪一层**（新模块 vs `render.rs` 内）。
2. `SnapshotSlot`／`arm_from_snapshot` 的**推进契约**：`EngineTrackSource` 是**有状态**的（`advance(frames)`），
   而今天的 `MidiSynthSource` 是"绝对帧的纯函数"⇒ 这把"块无关"从**构造性**降级为**契约性**（靠调用顺序保证）。
   `yeban-render` 的 `render_node` 每节点按块顺序推进，CLI 已如此 ⇒ 语义成立，但必须写进判据。
3. 块长：`yeban-render` 的 `L1_BLOCK_SIZE = 128`、引擎的 `DEFAULT_BLOCK_FRAMES = 128`，今天一致 ⇒ 需要一条判据钉住它。
