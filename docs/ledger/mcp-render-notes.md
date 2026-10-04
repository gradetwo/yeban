# mcp-render 工作线台账：`yeban_render_master` 的真渲染接线

- **台账类型**：交付映射 / **能力矩阵** / 判据与注入证据 / 未决项（**不是规范**）
- **工作线**：`line/mcp-render`（worktree `yeban/.worktrees/mcp-render`）
- **所有者目录**：`crates/yeban-mcp/**`（本文件是唯一新增的文档；另按任务书改了
  `docs/ledger/tools-domain-notes.md` 的 `MCP-TOOL-008` 那一行）
- **规范来源**：
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2（`MCP-TOOL-008` 的参数与错误码）、
    §4（离线渲染管线）、§5.3/§5.4（`ARCH-SEC-003/004` 容器与原子落盘）、§5（`ARCH-FMT-001`）
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M4-004/005/006`、`BASELINE-001`
  - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md`
    **D7**（未具备条件一律 PENDING）、**D19**（engine/render 的 feature 切分）、
    **D25**（错误码联集 20 值）、**D26**（`symphonia`/`rubato` 与规范 API 落差）、
    **D30**（`.yeban` 容器）
  - `schemas/mcp-tools.schema.json`（承重契约：工具名 / `dryRun` / `idempotencyKey` / 错误码 20）
  - 前置台账：`docs/ledger/tools-domain-notes.md`（本线接手它的"唯一未接线的一半"，即 008）
    与 `docs/ledger/render-master-notes.md`（`yeban-render` 侧的边界：`latencies` 的唯一来源、
    `AudioSource` 是注入的、本机不编译）
  - `docs/ledger/store-container-notes.md`（**保存**走 `.yeban` 容器；渲染产物是**另一个文件**）

> 本文件回答：**`yeban_render_master` 现在到底渲染了什么**、**什么明确没渲染**、
> **实测数字是多少**、**哪些读数来自本机、哪些只能来自 CI**、**需要谁裁决什么**。

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件（相对 `crates/yeban-mcp/`） | 内容 | 规范 ID |
| :--- | :--- | :--- |
| `src/domain/render.rs` | **参数校验 + 真渲染**：路由图探针剪枝、`RenderPlan` 编译/执行、MIDI 合成源、母带增益与峰值归一化、24-bit 抖动、RIFF/RF64/BW64 + `bext` 编码、路径规则与两条护栏、实测数字载荷 | `MCP-TOOL-008`、`ROAD-M4-004/005`、`ARCH-DET-002`、`ARCH-PDC-001`、`ARCH-FMT-001` |
| `src/domain/render_math.rs` | **零第三方依赖**纯逻辑：tick→帧、峰值/归一化、线性包络、毫秒→帧、Hinnant 日历、缺省输出名 | 同上（本机可单独执行） |
| `src/domain/mod.rs` | `Plan::RenderMaster { artifact }`；`plan_render_master`（只读渲染）；`apply_render`（原子落盘）；`apply` 去掉不再需要的 `&ToolCall` 参数 | `MCP-TOOL-008`、`ARCH-SEC-004` |
| `src/tools.rs` | `MCP-TOOL-008` 新增可选参数 `path`；错误码列仍只写规范表格那一行（见 §4） | `MCP-TOOL-008` |
| `src/dispatch.rs` | 幂等/`dryRun` 相关判据更新（幂等**不再**依赖 `-32005` 分支） | `MCP-TOOL-001..010` |
| `src/transport/http.rs` | 501 那条判据改成"渲染已接线、采样率不一致 ⇒ 带内 `RENDER_FAILED`" | `MCP-TOOL-008` |
| `src/domain/store.rs` | **未改**：`write_project_atomic`（tmp + `sync_all` + rename）被渲染复用 | `ARCH-SEC-004` |
| `tests/render_master.rs` | **12 条端到端判据**（尺寸自洽、确定性+钉死哈希、归一化、`dryRun`、幂等、`IO_ERROR`、错误码、路径护栏、非静音、延迟来源、format、unsupported） | `MCP-TOOL-008`、`MUST-GATE-002` |
| `verify/render_pure.rs` | **非 cargo 目标**的本机零依赖脚手架（`#[path]` 引入真实 `render_math.rs`） | — |
| `Cargo.toml` | 新增 `yeban-render.workspace = true`、`libm.workspace = true`（**未改根 `Cargo.toml`**，**未引入新的第三方包**） | — |
| `crates/yeban-render/**` | **一个字节都没改**（本线按任务书"用已有的导出能力"） | — |

依赖方向：`yeban-mcp → yeban-render → {yeban-model, yeban-dsp, libm, sha2, rayon, midly, hound}`。
新增的两条依赖边都已在根 `[workspace.dependencies]` 登记，`Cargo.lock` 里**没有新包**
（`deny` / 许可清单不因此漂移；`D19` 关于"render 消费 engine 时 `default-features = false`"
的前提在这里不成立：`yeban-render` **不**依赖 `yeban-engine`）。

---

## 2. 能力矩阵（**真做 / 半做 / 未接线**）

### 2.1 真做（有判据钉住）

| 能力 | 落点 | 判据 |
| :--- | :--- | :--- |
| 参数白名单与模型层采样率集合 | `render::validate` | `src/domain/render.rs` 单元判据 + e2e `every_impossible_request_...` |
| 输出路径规则（显式 `path` / 缺省 `<stem>.master.<format>`） | `RenderRequest::output_path` | 单元 `default_output_path_follows_the_documented_rule` + e2e `dry_run_...` |
| 输出路径两条护栏（工程文件本身 / `.yeban.lock`） | `guard_output_path` | 单元 + e2e `output_path_may_not_be_the_project_or_the_lock_file` |
| 帧数推导（tick→帧，四舍五入）与 1 小时上限 | `render_math::ticks_to_frames` | 纯逻辑 17 条（含整数有理数对账） |
| 路由图剪枝 / 源节点集合 | 探针编译 `RenderPlan::compile_with_latencies`（frames=1） | e2e 全绿 + `sources[].audible` |
| MIDI 合成源（正弦基波 + 线性 5ms/10ms 包络） | `MidiSynthSource` | e2e `the_master_is_not_silent_and_panning_moves_the_image` |
| 音符字段：`start_tick`/`duration_ticks`/`pitch`(等程律)/`velocity`/`micro_timing_ticks` | `build_source` | 同上 + `master_is_not_silent` 的 RMS 断言 |
| placement `start_tick` / `muted`、音轨 `volume_db`/`pan`(等功率 −3 dB)/`mute`/`solo`(源轨规则) | `build_source` | e2e 声相左右不对称 + 居中对称 |
| 设备延迟**唯一来源** `DeviceDefinition::latency_samples` → PDC | `track_latencies`（`yeban-render`） | e2e `longest_path_latency_comes_from_the_device_definitions`（32+16=48，旁通不计） |
| 边 `gain_db` / 固定顺序串行归约 | `yeban-render`（复用） | `yeban-render` 自己的判据 + 本线端到端 |
| Master 轨 `volume_db` / `mute` | `build` 后乘 | 端到端字节自洽 |
| TPDF 抖动 + 24-bit 量化（种子 = 工程 `rng_seed` × Master 身份） | `dither::quantize` + `rng::dither_rng_for` | e2e 确定性 + **钉死常量哈希** |
| RIFF / RF64 / BW64 + `bext`（工程 ULID、真实 `CodingHistory`） | `rf64`（复用） | e2e 三种容器 + 独立头部长度对账 |
| 峰值归一化（目标满量程；全零信号为恒等） | `render_math::normalize_gain` | 纯逻辑 + e2e（±1e-6） |
| **原子落盘**（同目录 tmp + `sync_all` + rename） | 复用 `store::write_project_atomic` | e2e 只读目录 ⇒ `IO_ERROR` 且旧产物逐字节不变、无残留 |
| 实测可观测性（12 个字段） | `RenderArtifact::details` | e2e `one_second_...` 逐项断言 |
| 幂等（同 `idempotencyKey` 不重复渲染） | 复用 `dispatch` 第 6 步缓存 | e2e：inode/`mtime` 不变 + 删产物后重放**不重建** |
| `dryRun`（同一**只读**渲染路径，不落盘） | `plan_render_master` + `Plan::describe` | e2e：目录**完全为空**、工程字节不变、预览给出路径/帧数/实测 SHA-256 |

### 2.2 半做（真做了一半，另一半**明确登记**）

| 项 | 真做的 | 缩水的（口径写在响应 `unsupported` 里） |
| :--- | :--- | :--- |
| 音源 | 确定性的正弦基波 + 线性包络 | 不是 SFZ 采样器、没有多振荡器/滤波/效果器；**本模型版本的 `DeviceKind` 里也没有 SFZ 变体** ⇒ `sfzSampler` 登记为"能力不在本切片" |
| 设备链 | `latency_samples` 进了 PDC（相位对齐是真的） | 设备**参数**（cutoff/resonance…）没有求值 ⇒ `deviceChainDsp` |
| 外部插件 | 延迟照算 | 没有宿主、没有参数 ⇒ `externalPlugins` |
| 自动化 | 工程里的自动化泳道被**如实登记** | 曲线没有求值（连静态值也不代偿） ⇒ `automationLanes` |
| 循环 | placement 第一遍被渲染 | `loop_config` 的**重复**没有展开 ⇒ `clipLoopRepetition` |
| 音频片段 | 摆放的时序被计入母带长度 | `ClipContent::Audio` 的 CAS 资产字节**没有被消费**（`yeban-decode` 不在本线地盘） ⇒ `audioClips`（静音 + 登记） |
| `solo` | 源轨规则：`any_solo ⇒ 仅 solo || solo_safe 发声` | 辅助返回**总线**不参与 solo 判定（登记在 §7 边界） |
| 包络 | 固定 5 ms 起音 / 10 ms 释音（线性） | 工程模型里没有 ADSR 字段可读；这是**本地常量**（`ATTACK_MS`/`RELEASE_MS`） |
| 位深/声道/采样率 | 24-bit 立体声；采样率必须与工程一致 | 重采样器未接线 ⇒ 不一致时 `RENDER_FAILED`（**不是**猜一个）；位深/声道不可配（规范表格也没有这两个参数） |
| `bext` 响度 | 写哨兵 `UNKNOWN`（沿用 `yeban-render` 的裁决） | 归一化后的 **sample peak** 报了；**true peak / LUFS** 没有测量（真峰值需要过采样，`TruePeak` 在 `yeban-dsp`，本线未接） ⇒ 绝不写假 LUFS |
| 性能 | 管线是真的、并行是真的 | **没有** `criterion` 打点 ⇒ 与 `BASELINE-001` 无关（见 §7） |

### 2.3 未接线（明确不在本切片）

| 项 | 为什么 |
| :--- | :--- |
| 重采样（`ARCH-DSP-002`、ADR-0001 D26 的 `rubato`） | 不在本线地盘；请求采样率 ≠ 工程采样率时返回 `RENDER_FAILED`（`data.unwired = "resampler"`） |
| 侧链**键控**语义 | `yeban-render` 的既有边界：侧链边按普通音频边处理；出现即登记 `sidechainRouting` |
| Master 轨 `pan` | 总线节点的声相无法在源侧表达；出现即登记 `masterPan` |
| 音频解码（WAV/FLAC/MP3 资产） | `yeban-decode` 是别的所有者；因此音频片段只登记不渲染 |
| `BUSY` | 领域状态单线程同步 ⇒ "已有渲染在跑"的窗口不存在（**登记，不是遗漏**） |
| 更细的工程时间轴（自动化驱动的长度、`transport` 循环） | 母带长度 = 源节点上可闻 placement/音符的最大末端 tick（确定，可解释） |

---

## 3. 三个"口径"（避免下游误读）

### 3.1 输出路径规则

```text
arguments.path 存在        → 原样使用（空串/纯空白 ⇒ INVALID_PARAMETER_RANGE）
否则                       → <工程文件所在目录>/<工程文件 stem>.master.<format>
                              /x/demo.yeban + format=wav  ⇒ /x/demo.master.wav
                              /x/no-ext     + format=rf64 ⇒ /x/no-ext.master.rf64
                              /             (没有 stem)   ⇒ ./master.master.<format>
```

两条护栏：不得等于**工程文件本身**、不得等于**`.yeban.lock`**（都用
`canonicalize` 兜住"同一文件的不同写法"）。

### 3.2 `normalize` 的口径

- **峰值归一化**，作用域 = **抖动/量化之前的浮点母带**（在 Master 增益之后）；
- 目标 = `1.0`（满量程）；`gain = 1/peak`；
- **全零信号**（`peak == 0`）：`gain = 1.0`（恒等），响应里
  `normalize.applied = false` + `normalize.note` 说明"全零信号"。
  把 0 放大到任何目标都是在制造不存在的信号，因此刻意**不**报"成功归一化"；
- 容差与理由：f32 的除法与乘法各一次舍入 ⇒ 实测 `|peak_after − 1.0| < 1e-6`
  （相对误差上界 ~2⁻²⁴）。判据在 `tests/render_master.rs`；
- **抖动在归一化之后**：全零**浮点**母带经 TPDF 抖动后，文件里会出现 ±1 LSB 的量化噪声
  —— 这是正确的 dither 行为（判据断言 "≤ 2 LSB" 而不是"逐样本为 0"）。

### 3.3 实测数字（1 秒 48 kHz 立体声夹具）

夹具：120 BPM / 960 PPQ / 1920 tick、单条 `A4` 音符（velocity 100）、
工程 `rng_seed = 0x594542414E000001`、固定注入时钟 `1_760_000_000_000` ms、
固定 ULID（无熵源 ⇒ **完全确定**）。

| 量 | 值 |
| :--- | ---: |
| `frames` / `channels` / `bitDepth` | 48 000 / 2 / 24 |
| `blocks`（128 帧一块） | 375 |
| `payloadBytes` | 288 000（= 48000×2×3） |
| `headerBytes`（RIFF） | 694（12 + `fmt ` 24 + `bext` 650 + `data` 头 8） |
| `bytes`（RIFF） | 288 694 |
| `bytes`（RF64 / BW64） | 288 730（多 36 字节 `ds64`） |
| `masterDigest`（浮点样本位型 SHA-256） | `b242d510d581541732134d3c0e233a11d6045ffb65535675adc73501fb28eefb` |
| 文件 `sha256`（RIFF） | `b9472fcd20086efd4953d457dde26169d1374f592bc90a0cf7b81304e0b69cc4` |
| 文件 `sha256`（RF64） | `5427fabd34c65cebb732dc61e69163c6d82e72a155773f211d8c3a64f7c2c54b` |
| 文件 `sha256`（BW64） | `c51db1def06ed2e0312aecfc0ab119dfa0d9fb547ad60cd8b45ba2242610adc2` |
| `longestPathFrames`（无设备夹具） | 0 |
| `durationSeconds` | 1.0 |

`masterDigest` 与容器无关（三种格式相同）—— 这本身是一条判据
（`the_requested_format_changes_the_container_but_not_the_audio`）。
前三个哈希已**钉进判据**（`two_renders_of_the_same_project_are_byte_identical`）：
夹具确定 ⇒ 它们是跨机器可比的常量（L1 位级一致的可比形式）。

---

## 4. 错误映射表（**不发明新码**，ADR-0001 D25 联集内）

| 状况 | 出口 | 载荷要点 |
| :--- | :--- | :--- |
| `format` 不在白名单 | `INVALID_PARAMETER_RANGE` | `data.supportedFormats` |
| `sampleRate` 不在模型集合 / 非整数 | `INVALID_PARAMETER_RANGE` | 模型层错误信息 |
| `normalize` 非布尔、`path` 非非空字符串 | `INVALID_PARAMETER_RANGE` | 实参形状 |
| 输出路径 = 工程文件 / 锁文件 | `INVALID_PARAMETER_RANGE` | `outputPath` / `projectPath` / `lockFile` |
| 没有活跃工程 | `NO_ACTIVE_PROJECT` | — |
| 采样率 ≠ 工程采样率（重采样未接线） | `RENDER_FAILED` | `unwired: "resampler"`、`requestedSampleRate`、`projectSampleRate`、`specId: ARCH-DSP-002` |
| 0 帧（没有可渲染的 MIDI 内容） | `RENDER_FAILED` | `endTick`、`sourceNodes`、`hint` |
| 帧数超上限（1 小时 @ 48 kHz） | `RENDER_FAILED` | `frames`、`maxFrames` |
| 路由图非法 / Master 不在图里 / 有环 | `RENDER_FAILED` | `renderError`（`RenderError` 的 Debug）、`context` |
| 源节点没有对应音轨 / 摆放引用缺失片段 | `RENDER_FAILED` | 相关身份 |
| 容器编码失败 | `RENDER_FAILED` | `rf64Error` |
| 输出目录不存在 / 不可写 / 磁盘满 | `IO_ERROR` / `DISK_FULL`（`store` 的既有映射） | `kind`、`osError` |
| 缺必填参数 / 参数类型不符 | JSON-RPC `-32602`（**契约层**，在领域之前） | `detail` |

`src/tools.rs` 里 `MCP-TOOL-008` 的 `errors` 列**仍然只写规范表格那一行**
（`RENDER_FAILED` / `BUSY`）：那一列的口径是"表格里列了什么"，而
`tools.rs::per_tool_error_codes_cover_every_documented_code_exactly` 要求
"全部工具声明的并集 == 表格的 16 个"。实现真实产出的额外码
（`NO_ACTIVE_PROJECT`/`INVALID_PARAMETER_RANGE`/`IO_ERROR`/`DISK_FULL`）**全在 D25 联集内**，
由 `tests/tools_e2e.rs::every_emitted_error_code_is_inside_the_contract_enum` 兜住
（与 `tools-domain-notes.md` 的 `boundary-7` 同一处置）。

---

## 5. 判据清单

### 5.1 端到端（`crates/yeban-mcp/tests/render_master.rs`，12 条）

| # | 判据 | 断言的可观测量 |
| :--- | :--- | :--- |
| 1 | `one_second_of_stereo_master_is_header_plus_frames_times_channels_times_depth` | 文件字节 == **独立手算**的头 + 帧×声道×位深/8（+偶数补位）；`parse_container` 回读；`bext` 携带工程 ULID |
| 2 | `two_renders_of_the_same_project_are_byte_identical` | 两次渲染逐字节相同 + **两个钉死的 SHA-256 常量** |
| 3 | `normalize_hits_full_scale_and_leaves_all_zero_signal_alone` | 归一化后峰值 ∈ 1.0±1e-6、文件真的变了；全零信号 `applied=false` 且文件 ≤ 2 LSB |
| 4 | `dry_run_writes_nothing_and_says_where_it_would_write` | 目录**完全为空**、工程字节不变、预览给出路径/帧数/实测 SHA-256 |
| 5 | `the_same_idempotency_key_does_not_render_twice` | `mtime`/inode 不变 + 删产物后重放**不重建** + 不同键真渲染 |
| 6 | `read_only_output_directory_is_an_io_error_and_keeps_the_previous_file` | `IO_ERROR`、旧产物逐字节不变、无 `.tmp-` 残留 |
| 7 | `every_impossible_request_returns_a_contract_error_code` | 契约内错误码 + `data.unwired` + 0 帧 + `NO_ACTIVE_PROJECT` |
| 8 | `output_path_may_not_be_the_project_or_the_lock_file` | 护栏在写盘**之前**生效 |
| 9 | `the_master_is_not_silent_and_panning_moves_the_image` | 左/右 RMS（全左声相不对称、居中对称） |
| 10 | `longest_path_latency_comes_from_the_device_definitions` | 0 / 48（32+16，旁通 999 不计）+ `deviceChainDsp` 登记 |
| 11 | `the_requested_format_changes_the_container_but_not_the_audio` | 三种容器前缀 + 负载逐字节相同 + `masterDigest` 相同 |
| 12 | `unsupported_features_are_named_and_counted_only_when_present` | 有循环才登记 `clipLoopRepetition`；干净工程 `unsupported == []` |

### 5.2 纯逻辑（`verify/render_pure.rs` + `render_math.rs` 单元，本机 `rustc --test`）

17 条：整数有理数版 tick→帧对账（200+ 样本）、400 年日历往返（28 571 天）、
峰值/归一化暴力对账、显式分段包络对账（上千组）、输出命名、`bext` 时间戳单调与补零、
毫秒换算自洽、病态输入返回 0、包络端点与"极短音符不被静音"、全零归一化恒等。

### 5.3 改写的旧判据（`-32005` 那一版已退役）

| 文件 | 旧 | 新 |
| :--- | :--- | :--- |
| `src/domain/mod.rs` | `render_master_validates_before_it_reports_not_wired`（好参数 ⇒ `-32005`） | `render_master_validates_first_then_really_writes_a_master`（坏参数 ⇒ 带内；好参数 ⇒ 真文件 + 90 000 帧 + SHA-256） |
| `src/dispatch.rs` | `the_only_implementation_level_code_left_is_the_unwired_renderer` | `no_tool_is_left_at_an_implementation_level_error`（好参数带内成功、坏参数带内失败、不再有 `validated` 字段） |
| `src/dispatch.rs` | `idempotency_replays_the_same_result_with_the_current_id`（用 `-32005` 当载荷） | `idempotency_replays_the_same_result_without_rendering_twice`（产物 `mtime` 不变 + 删产物不重建） |
| `src/transport/http.rs` | `unwired_renderer_maps_to_501_only_after_parameter_validation` | `render_master_is_in_band_over_http_and_never_answers_501` |
| `tests/tools_e2e.rs` | `not_implemented_never_appears_in_a_tool_response` / `no_tool_answers_with_a_blanket_not_implemented` | 同名前者的新断言（`dryRun` 带内成功 + `wired: true`）；后者要求**任何工具都不再有 JSON-RPC 层错误** |

---

## 6. 本机真跑 vs CI（严格区分）

### 6.1 本机**真的跑了**什么

| 命令 | 读数 |
| :--- | :--- |
| `rustc --edition 2024 --test -D warnings -W missing_docs crates/yeban-mcp/verify/render_pure.rs` | **17 通过 / 0 失败**（真实 `render_math.rs`） |
| `clippy-driver --edition 2024 --test -D warnings -D clippy::all -D rust_2018_idioms crates/yeban-mcp/verify/render_pure.rs` | **0 告警** |
| `bash scripts/dev/cargo-local.sh fmt --all --check` | 通过 |
| `bash scripts/gates/run-gates.sh light` | 通过（fmt + 13 条红线守卫 + 文档链接 + 许可清单漂移） |
| `cargo metadata --offline`（重新生成 `Cargo.lock`） | 仅 +2 行：`yeban-mcp` 的依赖表加 `libm` / `yeban-render`（**没有新包**） |
| `python3 scripts/gates/license_inventory.py` + `--check` | 重新生成后通过（679 行；`libm` 的"直接依赖方"多一个 `yeban-mcp`） |

### 6.2 **本机替代性验证**（必须如实说明，别当成"CI 已绿"）

`yeban-mcp` 现在传递依赖 `rayon`/`hound`/`midly`。按本机纪律（`AGENTS.md` §5.2、
`docs/DEV_WORKFLOW.md`）我**没有**在本机编译它们，也就**没有**跑
`run-gates.sh crate yeban-mcp` / `cargo test -p yeban-mcp`。

替代方案（**仓库外的临时脚手架**，`/tmp/tcroot/pkg/mcp-typecheck`，不入库）：

- 一个 scratch 包用**绝对路径**把 `crates/yeban-mcp/src/lib.rs` 当作 crate root
  （因此 `crate::…` 路径、内嵌 `#[cfg(test)]`、5 个集成测试目标全都是**真实源文件**）；
- 它把依赖 `yeban-render` 指向一个**测试替身**：
  - `dither.rs` / `rf64.rs` / `rng.rs` 用 `#[path]` 引入**真实源文件**（它们零重依赖）；
  - `render.rs` 是手写的**简化调度器**（剪枝 + 源节点 + 固定顺序求和 + 由
    `latencies` 推 `longest_path` + 位级摘要），**没有** rayon 线程池与 PDC 延迟线；
- `[lints]` 复制工作区的 `clippy::all`/`rust_2018_idioms` 设置，因此 `clippy -D warnings`
  与 CI 同源。

| 命令（在 scratch 包里） | 读数 |
| :--- | :--- |
| `cargo check --all-targets` | 通过（lib + bin + 5 个集成目标） |
| `cargo clippy --all-targets -- -D warnings` | **0 告警**（这一轮抓出并修掉了 4 个真问题：未使用的 `call` 参数、`format!` 位置参数缺失、`field_reassign_with_default`、`chunks_exact_to_as_chunks`） |
| `cargo test --lib` | **174 通过 / 0 失败** |
| `cargo test --test render_master` | **12 通过 / 0 失败** |
| `cargo test --test tools_e2e` | **25 通过 / 0 失败** |
| `cargo test --test contract / container_store / lock_advisory` | **16 / 15 / 14 通过** |
| 合计 | **256 通过 / 0 失败** |

**替身的边界（不许当成真渲染器的证据）**：

1. 真实调度器（Rayon 并行 + `ARCH-DET-002` 的固定顺序归约 + **PDC 延迟线**）
   **从未在本机执行**；
2. 本机替身对"**单源** ⇒ Master"这一类夹具在语义上与真实实现等价
   （源节点 → 单个入边、增益 1.0、无补偿延迟），因此 §3.3 的字节读数是**可以**在
   CI 上复现的**预期值**；但这条"等价"是**论证**，不是本机实测；
3. 跨架构（`MUST-GATE-003`）与 `BASELINE-001` 的性能读数**不在本线**。

### 6.3 注入 → 变红 → 还原（**5 条，全部真做过**）

方法：把 `render.rs`/`render_math.rs`/`dispatch.rs`/`store.rs` 备份到 `/tmp/inj-backup/`，
用 Python 精确替换并**断言锚点恰好命中 1 次**（否则属于"注入无效"，账本 L3），
跑测试、记录红掉的判据名，再从备份还原并复跑全绿。**每个 cargo 目标单独跑**
（第一处失败后 cargo 不会继续跑下一个目标 —— L12/L15 的教训）。

| # | 注入 | 变红的判据（实测） | 还原 |
| :--- | :--- | :--- | :--- |
| **I1** | 渲染结果改成固定静音（`output.samples.fill(0.0)`） | e2e：`the_master_is_not_silent_and_panning_moves_the_image`、`normalize_hits_full_scale_and_leaves_all_zero_signal_alone`（`10 passed; 2 failed`） | ✅ 12/12 |
| **I2** | `normalize` 改成空操作（`normalize_gain` 恒返回 1.0） | **零依赖脚手架**：`normalize_of_silence_is_identity`、`normalization_hits_the_target_within_one_ulp_scale`、`peak_and_normalization_agree_with_brute_force`（`14 passed; 3 failed`）；**e2e**：`normalize_hits_full_scale_and_leaves_all_zero_signal_alone` | ✅ 17/17 + 12/12 |
| **I3** | 关掉幂等缓存（查询前 `self.idempotency.clear()`） | **lib**：`distinct_keys_do_not_collide_and_cache_is_ordered`、`idempotency_replays_the_same_result_without_rendering_twice`；**e2e**：`the_same_idempotency_key_does_not_render_twice`；**tools_e2e**（单独跑）：`same_idempotency_key_applies_exactly_once`、`idempotent_merge_does_not_apply_the_batch_twice` | ✅ 174/12/25 |
| **I4** | 原子写改原地写（`create(true).truncate(true).open(target)`） | **e2e**：`read_only_output_directory_is_an_io_error_and_keeps_the_previous_file`；**tools_e2e**：`save_into_a_read_only_directory_fails_with_io_error_and_keeps_the_original` | ✅ 174/12/25 |
| **I5**（额外） | 抖动种子掺入时间熵 | **e2e**：`two_renders_of_the_same_project_are_byte_identical`（**钉死哈希**同时红）、`the_requested_format_changes_the_container_but_not_the_audio` | ✅ 12/12 |

**这 5 条证明的事**：① "母带一定是静音"会被抓；② "归一化是空操作"会被抓；
③ "幂等缓存关掉"会被抓；④ "原子写退化成原地写"会被抓；⑤ "引入熵源"会被抓
（因此确定性判据不是空判据）。

---

## 7. 与 `BASELINE-001` 的关系

**没有关系（如实说）**：本线**没有**产出任何性能读数。`BASELINE-001` 的门槛是
"32 轨参考工程 A ≥ 100× 实时"，需要 `criterion` + 参考硬件，属于别的所有者的 manual 档。
本线只跑 1 秒夹具的功能判据，**不声明**任何吞吐/实时倍数。
`gate-status.md` 的 `BASELINE-001` 与 `MUST-GATE-002` 状态**不需要**因为本线改动而变化：
- `MUST-GATE-002`（同平台 bit-exact）仍是**部分**：本线新增的**钉死哈希**判据是
  "同平台 + 同一实现"的位级判据（跨机器可比的**常量**），但"跨机器/跨架构 SHA-256 全同"
  这条门禁仍不存在（`MUST-GATE-003` 依然 PENDING，需两条真跑的架构）。

---

## 8. needs / pending / TODO(hoist)

### needs（需要裁决或别的所有者接线）

| # | 项 | 性质 | 建议 |
| :--- | :--- | :--- | :--- |
| **needs-1** | **音频片段渲染**（`ClipContent::Audio` 的 CAS 资产） | 跨所有者 | 需要 `yeban-decode` 的解码接口（WAV 之外还有 FLAC/MP3 的 D26 落差）。本线只登记 `audioClips` 并静音，**绝不**假装渲染了 |
| **needs-2** | **重采样**（`sampleRate` ≠ 工程采样率） | `ARCH-DSP-002` / D26 | 接 `rubato` 后把 `RENDER_FAILED` 换成真重采样。当前行为是**响亮的拒绝**，不是静默错音高 |
| **needs-3** | **设备链 DSP / 自动化求值** | 引擎层 | 需要 engine 线的参数求值与自动化曲线求值接口；本线只把 `latency_samples` 接进 PDC |
| **needs-4** | `Op` 全集缺 `AddClip`/`AddRoutingNode`（承接 `tools-domain` 的 needs-1） | 规范缺口 | 与 `yeban_propose_section` 的"配器骨架"同一个缺口；本线未触碰 |
| **needs-5** | `bext` v2 响度哨兵 / 工程 ULID 的规范落点（承接 `render-master` 的 needs） | 需要人类裁决（EBU Tech 3285 PDF 不可机读） | 本线沿用 `Loudness::UNKNOWN` 与 `OriginatorReference`，并把 `CodingHistory` 换成**真实**参数（不再写 `<sample_rate>` 字面量） |
| **needs-6** | `yeban-mcp` 的**性能回归**（`BASELINE-001` 之外：单次渲染时延） | 未打点 | 本线没有 `criterion`；若要门禁化，需要固定硬件 |
| **needs-7** | `run-gates.sh crate yeban-mcp` 在本机会编译 `rayon/hound/midly` | 门禁口径 | 建议给 `run-gates.sh` 的 `heavy_deps_of` 增加"**传递**重依赖"识别（现在只看本 crate 清单），否则本线每次本机门禁都会真编译重依赖 —— 与 AGENTS.md §5.2 的纪律冲突 |

### pending

| # | 项 | 状态 |
| :--- | :--- | :--- |
| P1（承接 `tools-domain`） | "十工具领域实现未接线" | **本轮关闭**：`yeban_render_master` 的渲染本体已接线，**本 crate 的工具路径上不再有任何实现级出口** |
| P8 | `BUSY` 不可达 | 仍 pending（单线程同步架构；异步渲染管线落地后才可达） |
| pending | `sfzSampler` | 本模型的 `DeviceKind` **没有** SFZ 变体；实现 SFZ 播放需要 `yeban-sfz` 接线（别的所有者） |
| pending | `noteRatchet` 展开 / `probability` 确定性触发（`rng_seed` 已就绪）/ 弯音曲线 / 滑音 / 歌词-音素歌声合成 | 都已**登记**为 `unsupported`，未实现；`probability` 的确定性 PRNG 基础设施（`yeban_dsp::noise::Rng` + `rng::dither_seed`）已在，接线成本低 |
| pending | placement 循环的第二遍及以后 | 当前只渲染第一遍；展开需要"循环次数 × 内容"的长度语义（工程模型里有 `LoopConfig`，缺的是渲染语义） |

### TODO(hoist)

**没有。** 本线只新增两条**已在根 `[workspace.dependencies]` 登记**的依赖边
（`yeban-render`、`libm`），**未改**根 `Cargo.toml`，**未引入**新的第三方包
（`Cargo.lock` 的包集合不变 ⇒ `deny`/许可清单不漂移）。

---

## 9. CI 判决（读到什么写什么）

读取方式：`bash scripts/dev/ci-verdict.sh line/mcp-render`

| 轮次 | run id | 头部 | 结论 |
| :--- | ---: | :--- | :--- |
| 第 1 轮 | 见提交信息 / `ci-verdict.sh` 读数 | — | 见下文补记 |

补记待填：本轮推送后由 `ci-verdict.sh` 读回。

---

## 10. 修改文件的绝对路径清单

```text
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/Cargo.toml
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/src/domain/render.rs
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/src/domain/render_math.rs   (新增)
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/src/domain/mod.rs
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/src/domain/error.rs
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/src/tools.rs
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/src/dispatch.rs
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/src/transport/http.rs
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/src/lib.rs
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/tests/render_master.rs     (新增)
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/tests/tools_e2e.rs
/Users/crow/work/music/yeban/.worktrees/mcp-render/crates/yeban-mcp/verify/render_pure.rs     (新增)
/Users/crow/work/music/yeban/.worktrees/mcp-render/docs/ledger/mcp-render-notes.md            (新增, 本文件)
/Users/crow/work/music/yeban/.worktrees/mcp-render/docs/ledger/tools-domain-notes.md          (仅 008 那一行 + 一条转记)
```

**未触碰**：根 `Cargo.toml`、`Cargo.lock`、`.github/**`、`scripts/**`、`deny.toml`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`、
`crates/yeban-render/**`（零改动）、其它 `crates/**`、`spikes/**`、法务文件。
