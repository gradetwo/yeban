# 工具可用性与 feature 门控（tool-availability）

本文件回答三个问题：MCP 工具在**哪种形态**里能用、被哪个 **feature** 门控、
哪些工具还依赖**宿主注入的 provider**。每条都给出代码出处，与代码不一致时以代码为准。

工具注册表的唯一事实源是 `crates/yeban-mcp/src/tools.rs` 的 `TOOLS`
（现读：`TOOL_COUNT = 44` ＝ `DOCUMENTED_TOOL_COUNT 10` ＋ `EXTENSION_TOOL_COUNT 34`；
⛔ 不要引用任何文档里手抄的旧数字，工具数以这三个常量与注册表为准）。

---

## 1. 两种形态，同一份注册表

出处：`crates/yeban-mcp/src/lib.rs` 模块文档、`crates/yeban-mcp/src/transport/mod.rs`。

| 形态 | 是什么 | 入口 |
| :--- | :--- | :--- |
| **形态 A（库形态）** | `yeban-app` 进程内托管，AI Agent 在运行中的 DAW 上工作 | `crates/yeban-app/src/mcp_mount.rs` 挂载；传输为环回 HTTP（仅 `127.0.0.1`）或进程内直调 |
| **形态 B（二进制形态）** | 独立 `yeban-mcp` 二进制走 stdio 批处理，无 GUI 环境 | `crates/yeban-mcp/src/bin/yeban-mcp.rs` |

两种形态共用同一份 `TOOLS` 注册表与同一个 `Domain`。差异只有两个：
① 形态 A 挂载时宿主会注入 provider（见 §3），形态 B 没有注入方；
② 形态 A 的 HTTP 监听多一道运行时开关（见 §2）。

## 2. feature 门控（`yeban-app`）

出处：`crates/yeban-app/Cargo.toml` 的 `[features]`、`crates/yeban-app/src/cli.rs`
的 `*_FEATURE` 常量与 `ParseError::*NotCompiled`。

### 2.1 缺省构建里有什么

- `default = ["in-process-mcp"]`。**2026-11-12 负责人裁决（ADR-0072）**：
  `in-process-mcp` 默认开；它转发 `yeban-mcp/mcp-http`，即形态 A 的监听代码
  **编译进**缺省构建。`--no-default-features` 仍可整组关掉。
- ⚠ 编译进来 ≠ 在监听。运行时还有**第二道开关**：不给 `--enable-mcp-http`
  也不设 `YEBAN_MCP_HTTP=1` 就不监听（`mcp_mount.rs` / `transport`）。
  `yeban-mcp` 自己的 `default = []`：`mcp-http` **绝不**进 `yeban-mcp` 的
  default（MUST-GATE-009），只能由宿主显式转发打开。
- 被 `in-process-mcp` 门控的 CLI 出口在缺省构建里**直接可用**；在
  `--no-default-features` 构建里调用它们 ⇒ 点名 feature 的用法错误
  （退出码 2，⛔ 不静默忽略）：`SongOverviewNotCompiled` /
  `ApplyOpsNotCompiled` / `ExportStemsNotCompiled`。

### 2.2 缺省关的 feature（给了开关先看这里）

出处：`crates/yeban-app/Cargo.toml` `[features]` 与
`scripts/guards/policy_check.py` 的 `FORBIDDEN_DEFAULT_FEATURES`。

| feature | 开关 | 缺省 | 缺省构建里调用的后果 |
| :--- | :--- | :--- | :--- |
| `experimental-als-export` | `--export-als` | 关 | `AlsNotCompiled` 用法错误（退出码 2），点名 feature |
| `experimental-logic-export` | `--export-logic` | 关 | `LogicNotCompiled`，同上 |
| `experimental-logic-import` | `--inspect-logic` / `--import-logic` | 关 | `InspectLogicNotCompiled` / `ImportLogicNotCompiled`，同上 |
| `ui-mcp-http` | `--enable-ui-mcp-http` | 关 | `UiMcpHttpNotCompiled`，同上 |
| `midi-input` | `--midi-in` | 关 | 物理 MIDI 输入入口不编译 |
| `ui-test-port` | `--ui-test-mode` | 关 | UI 测试口不编译 |

## 3. 依赖宿主 provider 的工具（只有两个）

出处：`crates/yeban-mcp/src/domain/mod.rs` 的 `Domain` 字段与
`set_song_overview_provider` / `set_phase_align_auto_provider`；
实现侧 `crates/yeban-mcp/src/domain/song_overview.rs` /
`crates/yeban-mcp/src/domain/phase_align_auto.rs`。

| 工具 | 依赖的 provider trait | 宿主实现（形态 A） | 注入点 |
| :--- | :--- | :--- | :--- |
| `get_song_overview` | `SongOverviewProvider` | `RealSongOverviewProvider`（`crates/yeban-app/src/song_overview_provider.rs`） | `Domain::set_song_overview_provider`，`mcp_mount.rs` 挂载时调用 |
| `yeban_auto_align_tracks` | `PhaseAlignAutoProvider` | `RealPhaseAlignAutoProvider`（`crates/yeban-app/src/phase_align_auto_provider.rs`） | `Domain::set_phase_align_auto_provider`，同上 |

**为什么是注入而不是直接实现**：`yeban-mcp` 是轻量 crate，依赖方向由
`scripts/gates/check_mcp_dependency_direction.py` 机械守卫——它不得拖
`yeban-dsp` 音频分析栈进来。分析实现住在 `yeban-dsp` 一侧，只能由宿主
（`yeban-app`）以 provider 形式注入。没有任何工具会自己写这两个字段。

**无 provider 时的契约（两个工具同口径）**：调用 ⇒ `CONFLICT` 错误，
`data.reason = "providerUnavailable"`（常量 `PROVIDER_UNAVAILABLE_REASON`），
并带 `providerAttached: false` 与指向形态 A 的 `hint`。
⛔ **绝不返回估算值、占位值或假测量**——没有 provider 就是响亮地不可用。

**推论**：形态 B（独立 stdio 二进制）在当前主干没有任何注入方 ⇒ 上表两个工具
在形态 B 里恒报 `providerUnavailable`；其余 42 个工具不依赖 provider，
两种形态行为一致。

## 4. CLI 导出开关 ↔ MCP 工具对应表

出处：`crates/yeban-app/src/cli.rs`（开关字面值与 `*_FEATURE` 常量）、
`crates/yeban-app/src/lib.rs` 的模块门控、各出口模块文档。

| CLI 开关 | 做什么 | MCP 对应 | feature 门 |
| :--- | :--- | :--- | :--- |
| `--export-wav` | 母带 WAV 导出（`export_wav.rs`） | `yeban_render_master`（同一渲染口径；母带缺省 24-bit） | 无（常开） |
| `--export-midi` | SMF 导出（字节由 `yeban-render` 编码器产出，app 只做薄封装） | `yeban_export_midi`（响应契约 `{encoding:"base64", content, bytes, sha256}`） | 无（常开） |
| `--export-elements` / `--dump-elements` | 无头元素树导出 | 无对应工具（UI 内省面） | 无（常开） |
| `--export-song-overview` | 整曲总览 JSON；内容与 `get_song_overview` 响应 `data` **逐字节相同** | `get_song_overview`（`format:"json"`） | `in-process-mcp`（缺省开） |
| `--export-song-overview-html` | 同一份总览的单文件 HTML 报告 | `get_song_overview`（`format:"html"`） | `in-process-mcp`（缺省开） |
| `--export-stems` | 分轨导出；后端与工具同一份（`stems::export` / `stems::write`），值是输出目录（不存在时写盘时创建） | `yeban_export_stems` | `in-process-mcp`（缺省开） |
| `--apply-ops` | 批量执行 ops 文件；文件格式就是 `yeban_edit_notes` 的实参对象，执行管线（`parse_ops` / `compile` / 提交）住在 `yeban-mcp`；`--dry-run` 是它的修饰器 | `yeban_edit_notes` | `in-process-mcp`（缺省开） |
| `--edit-samples` | 采样库批处理编辑（`sample_editor.rs`） | 无对应工具 | 无（常开） |
| `--slice-audio` | 音频切片批处理（`audio_slicer.rs`；`tape` 变速变调 / `shift` 移调时长不变 / `stretch` 拉伸音高不变） | 无对应工具 | 无（常开） |
| `--export-lyrics` | 歌词时间轴导出成 LRC（纯模型数据 `YebanProjectV1::lyrics` ⇒ LRC 文本，**不经过 MCP**） | 无对应工具 | 无（常开；明确不需要 `in-process-mcp`） |
| `--export-als` | Ableton `.als` 导出 | 无对应工具 | `experimental-als-export`（缺省关） |
| `--export-logic` | Logic Pro `.logicx` 导出 | 无对应工具 | `experimental-logic-export`（缺省关） |

一行总结：**缺省构建里，全部 MCP 工具与上表除最后两行外的 CLI 出口都可用**；
两个 provider 工具只在形态 A 里有真数据；`.als` / `.logicx` 两个实验导出
需要重编译时显式开 feature。
