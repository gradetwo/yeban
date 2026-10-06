# 夜半 (Yeban) 自主 AI Agent 研发执行规则与契约守则 (Agent Rules & DoD)

> **适用对象**：所有参与夜半 (Yeban DAW) 项目代码生成、重构、审查与测试的自主 AI Agent（如 Antigravity / Cursor / Claude Code 等）。  
> **核心原则**：纯 Rust 原生桌面 + Slint 响应式 GUI + 100% 自动化测试与质量门禁驱动 + 零人工人力评估。

---

## 1. 权威规范来源 (Mandatory Sources of Truth)

所有 Agent 在进行任何任务时，必须以以下文件为单一权威事实源：

1. **架构与系统设计核心 (Normative)**：[`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`](docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md)
2. **工程重构路线图与门禁 (Normative)**：[`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`](docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md)
3. **桌面 UI/UX 交互设计规范 (Advisory / Reference Only —— 2026-10-06 经负责人裁决自 Normative 降级)**：[`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md`](docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md)（*注：参考设计，**非绑定**，不得直接作为硬性实现规范*）
4. **法律合规与许可政策 (Mandatory Legal)**：[`LICENSE`](LICENSE), [`LEGAL.md`](LEGAL.md), [`TRADEMARK.md`](TRADEMARK.md)
5. **安全策略 (Mandatory Security)**：[`SECURITY.md`](SECURITY.md)
6. **行业调研与生态融合 (Informative / Research Only)**：[`docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md`](docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md)（*注：仅供选型与背景参考，不得直接作为硬性实现规范*）
7. **操作手册与判例 (Operational)**：[`docs/DEV_WORKFLOW.md`](docs/DEV_WORKFLOW.md)（怎么干活）、[`docs/CI_CD.md`](docs/CI_CD.md)（CI/CD 与判决怎么读）、[`docs/adr/`](docs/adr)（规范冲突时的裁决留痕）、[`docs/DEVELOPMENT_LEDGER.md`](docs/DEVELOPMENT_LEDGER.md)（测量账本与 pending 清单）

> 以上路径均为**仓库内相对路径**。历史文档曾用 `file:///home/crow/work/agy/review/GROOVE_V3_*` 这类绝对路径
> 指向已被重命名/移动的文件；本次已统一修正为 `docs/YEBAN_*`。

---

## 2. 绝对禁止项清单 (Forbidden Actions - Hard Redlines)

AI Agent 严禁执行以下操作，违者将被自动化 CI 与代码审查机制一票否决：

1. **法务文件红线**：严禁在未经人类责任人明确书面指示下修改 `LICENSE`、`LEGAL.md`、`SECURITY.md`、`TRADEMARK.md`、`GOVERNANCE.md` 与 `NOTICE.md`；
2. **依赖许可红线**：严禁引入未通过 `cargo deny check` 的依赖库，严禁引入任何带有非商业限制（如 CC-BY-NC）、专有不可再分发或不兼容 GPLv3 的第三方依赖；
3. **物理架构解耦红线**：严禁向 `yeban-model`、`yeban-dsp`、`yeban-theory`、`yeban-render`、`yeban-engine`、`yeban-sfz` 等引擎层 crate 引入任何 GUI（Slint/winit/OpenGL/Qt）相关依赖；
4. **确定性状态红线**：严禁在持久化 AST 核心实体集合中使用 `HashMap` 或 `HashSet`，必须强制使用 `BTreeMap` 以保障跨进程、跨重启迭代顺序的一致性；
5. **网络监听安全红线**：所有 MCP 及调试服务严禁绑定 `0.0.0.0`，必须且仅能绑定环回地址 `127.0.0.1`；
6. **发行特性安全红线**：官方默认 release 构建中严禁默认开启 `mcp-http`、`ui-mcp`、`asio`、`experimental-vst3`、`experimental-als-export` 或 `experimental-logic-export`，上述特性仅限在特定开发或测试配置中按需开启；
7. **实时音频安全红线**：实时音频回调函数内严禁出现任何堆内存分配（`malloc`/`Box::new`/`Vec::push` 等）、堆释放（`drop`/`dealloc`）、互斥锁等待（`Mutex::lock`）或阻塞式系统调用（文件/网络 I/O、控制台打印等）；
8. **内存安全红线 (Safe-by-Default)**：在 `yeban-model`、`yeban-theory`、`yeban-dsp`、`yeban-render` 中强制启用 `#![forbid(unsafe_code)]`；`unsafe` 仅限出现在已审计的底层驱动与 FFI 边界中，且必须附带详尽的 `// SAFETY:` 注释证明；
9. **资产与权重合规红线**：严禁提交任何大于 10MB 的未注册二进制文件，严禁提交任何未在 `assets/manifest.json` 或 `assets/models/MANIFEST.json` 中登记许可证与 SHA-256 的音频样本或神经网络权重。

---

## 3. 完成的定义 (Definition of Done - DoD)

任何由 AI Agent 实施的能力切片（Capability Slice）或代码提交，只有在满足以下全部条件时方可判定为“完成”：

1. **编译无警告**：代码在 Rust stable 工具链下编译通过，`cargo clippy --workspace --all-targets -- -D warnings` 零告警，`cargo fmt --check` 格式化通过；
2. **自动化测试 100% 通过**：单元测试、属性测试（`proptest`）与集成测试全部绿灯；
3. **需求规范 ID 闭环**：本次代码变更必须明确对应并标注所实现的规范 ID（例如 `ARCH-RT-001`, `MODEL-002`），更新对应的测试映射用例；
4. **基准性能无衰退**：基准测试（`criterion` / `iai-callgrind`）打点波动在允许阈值内（衰退不得超过 3%）；
5. **开源合规审计通过**：`cargo deny check` 100% 通过，无未授权协议或已知漏洞报告；
6. **UI 变更双重验证**：涉及 UI 的修改必须包含稳定的自动化元素 ID，并通过无头控件树 JSON 断言与无头截图比对（动态区域已遮罩）。

---

## 4. 规范需求 ID 命名字典 (Requirement ID Dictionary)

> 本节原为架构文档的节选，已按 **实测** 补全四份规范中真实存在的全部 ID 族
> （测法：对 `docs/YEBAN_*.md` 全文正则提取 ID 前缀并计数，见 `docs/DEVELOPMENT_LEDGER.md`）。

### 4.1 架构与数据模型

- `ARCH-TOP-*`：进程与线程拓扑规范需求（001–003）
- `ARCH-SEC-*`：网络、权限与归档安全需求（`.yeban.lock`、MCP 安全默认、ZIP 容器安全、原子落盘）
- `ARCH-RT-*`：实时音频线程零分配、快照退役回收、FTZ/DAZ、声部抢占、A/B 预滚（001–005）
- `ARCH-DET-*`：L1/L2 声学确定性契约需求（001–002）
- `ARCH-PDC-*`：内部插件延迟补偿与时延预算（001–002）
- `ARCH-DSP-*`：去爆音、重采样、冻结/分轨、弹性拉伸（001–004）
- `ARCH-UI-*`：局部脏矩形、电平 SPSC 解耦、无头配置、元素树内省安全、Testing Backend 使用约束（001–005）
- `ARCH-SLINT-*`：Slint 上游能力核验与三层自研兜底（001）
- `ARCH-OPS-*`：领域操作日志与提交图谱（001–002）
- `ARCH-FMT-*`：RF64/BW64 与实验性 DAW 互操作导出（`.als` / `.logicx`）（001–002）
- `ARCH-PLUG-*` / `ARCH-REC-*` / `ARCH-EXT-*` / `ARCH-SYS-*`：插件宿主 / 录音 / 外部协同 / 自动保存
- `MODEL-ISO-001`：持久化 / 会话运行态 / 本机配置三层状态物理隔离
- `MODEL-AST-001..005, 007`：960 PPQ 与 `EntityId`、`YebanProjectV1`、`BTreeMap` 确定性、`RoutingGraph`、`MidiNote`、CAS 哈希
  （**注意：`MODEL-AST-006` 在规范中缺号**，实现时不得凭空发明该编号）

### 4.2 服务与 UI

- `MCP-TOOL-001..010`：十个 `yeban_*` 工具（权威定义见 `schemas/mcp-tools.schema.json`）
- `MCP-DUAL-001`：双 MCP 自测闭环
- `UI-GRID-001..004`：Slint 视口、响应式断点与折叠、局部脏矩形、120 FPS
- `UI-NOTE-001..005`：卷帘视口裁剪、坐标双向映射、工具矩阵、编曲辅助、全键盘音符操控
- `UI-TEST-001..003`：语义 Element ID 寻址、事件注入、无头运行
- `UI-A11Y-001..004`：扫描码热键、IME `is_composing` 防护、无障碍树与焦点、色盲友好与 AAA 对比
- `UI-MCP-001..003`：三级权限分层、动态区域遮罩、分平台 Golden 与 SSIM ≥ 0.98

### 4.3 工程推进与验收

- `ROAD-M-1-001..006`：Phase -1 开源合规与仓库大扫除
- `ROAD-M0-001..009`：Phase 0 九大 Spike（目录见 `spikes/`）
- `ROAD-M1-001..006`：Phase 1 数据模型 / Ops Log / 容器 / 迁移 / 属性测试
- `ROAD-M2-001..008`：Phase 2 实时引擎 / PDC / SFZ / 声部池 / 电平表
- `ROAD-M3-001..007`：Phase 3 Slint 工作区 / 虚拟化卷帘 / A-B 盲听 / 无障碍 / 视觉回归
- `ROAD-M4-001..011`：Phase 4 MCP / 离线母带 / 实验性 DAW 互操作导出（`.als`、`.logicx`）/ 分发与门禁切流
- `MUST-GATE-001..015`：硬性发布门禁（一票否决）
- `BASELINE-001..006`：基准性能达标线
- `TEST-SPEC-001..006`：测试规范（属性幂等、模糊、跨架构对账、无头视觉回归、读屏用例、WCAG 对比度）
- `RSK-01..38`：风险登记册

---

## 5. 执行环境纪律 (Execution Environment Discipline)

用户的硬性要求：**开发测试过程中避免在本机跑高耗 CPU 任务**。本机是 Apple M2 开发/编辑环境。

1. **本机允许**：`cargo fmt`；**无重依赖** crate 的 `clippy` / `test`；全部守卫与 schema 脚本。
2. **本机禁止**：`--workspace` 全量构建、Slint / cpal / symphonia 等重依赖编译、基准测试、
   模糊测试、跨架构确定性对账。这些一律交给 GitHub Actions。
3. **纪律由脚本机械执行，不靠自觉**：`scripts/dev/cargo-local.sh` 直接拒绝 `--workspace` / `--all`；
   `scripts/gates/run-gates.sh crate <name>` 见到重依赖就跳过并指向 CI。
4. **CI/CD 双模式**：自动档 `ci.yml`（push/PR）+ 手动档 `gates-manual.yml`（`workflow_dispatch`）。
   详见 [`docs/CI_CD.md`](docs/CI_CD.md)。
5. **只有 CI 的判决算数**：本地绿是参考。判决必须用 `scripts/dev/ci-verdict.sh` 读回来，
   未读取的判决一律记为 `pending`，不得写成"通过"。
6. **多线并行**：一树一线（`scripts/dev/worktree.sh`），一个文件同一时刻只有一个写者。
   根级共享文件（`Cargo.toml`、`.github/**`、`scripts/**`、`docs/DEVELOPMENT_LEDGER.md`）
   由集成者独占。详见 [`docs/DEV_WORKFLOW.md`](docs/DEV_WORKFLOW.md)。
7. **规范缺口与冲突**：不得私自发明答案，也不得擅自改写 Normative 文档；把裁决写进
   [`docs/adr/`](docs/adr) 并标注 `Proposed`，由人类批准。

## 6. 测量纪律 (Measurement Discipline)

本节来自第 384-394 轮的实测教训。违反本节会产生"看起来有"的错误结论。

1. **先说指标，再跑命令。** 在运行测量命令**之前**，用一句话写出这个数字在数什么（对象 + 单位）。
   例："数 `cargo tree --prefix none` 里有几个**不同的 `name version` 行**"。
   只接受"单位已被你命名"的数字。**无单位的数字不许进结论。**
2. **删除与移动之前先核验目标路径。** 回退（`git checkout -- <path>`）的**基线**必须是你想回到的那个提交。
   用 `diff -q` 核对**正确的基线**，不要对你正要替换的东西核验（那是自我确认）。
   第 355 轮曾用"含缺陷的提交"当基线，于是"验证通过"了一个仍含缺陷的文件，代价是三轮。
3. **不要用词面判断交付。** 目标或台账里的**词**常与**交付形态**不同：
   `ui/*` 的 `dryRun` 在控制面而非 `.slint` 标记层；响度是 `yeban_query_engine_state` 的**字段**而非工具；
   `device` 是 `yeban_query_engine_state` 的**只读设备链**（宏工具 `yeban_set_macro` 承载的是 `track.macros[]`，不是设备对象），可写的只有 `yeban_edit_automation` 的 `DeviceParam` 目标，**工具面没有设备 CRUD**。**先 `grep` 交付，再下结论。**
4. **文件大小用 `stat -f%z`（字节），不要用 `du -h`。** `du` 报的是磁盘占用，与内容长度不同。
   第 390 轮因此把 5.4 MiB 报成 908 KB。
5. **数"条目"时不要用 `grep -c` 数"行"。** 条目与行不是一一对应。
   第 394 轮因此把 25 条损失登记报成 18。
6. **一次推送 = 一批改动**：tip run 的判决即整批的判决（L32）。
   **未读回的判决等于没有判决**：每个提交都必须读回 `conclusion`，并确认 `steps` 数不为 0。

### 6.3 "绿"的三条防伪规则（第 969-970 轮，两次真实误判换来的）

1. **一次推送 ≠ 一个 run。每个提交各自有一个 run。** 所以"tip 绿"只覆盖 tip。判"某提交绿"必须读**该提交自己**的 run（`gh run list` 里的 `headSha`）。曾据 tip 绿声称整批绿，事后三个提交全是红的。
2. **跳过的作业不是绿。** 本仓库 CI 的规划步骤对纯文档改动返回 `{"crates": [], "workspace_wide": false}`，于是所有 rust 作业显示为 `-` 且耗时 `0s`，整体结论却是 success。这种绿是**空绿**，不构成任何验证。判绿必须确认**目标作业真的执行过**（耗时非 0，且日志里有 `test result:` 行）。
3. **"字节数相同"通常不蕴含任何东西。** 本仓库的 PNG 写入器用**存储式 deflate**，长度为 `raw + ceil(raw/65535)×5 + 63`，所以 1920×1080 恒为 6,222,418 字节。判据报"基准 N 字节 / 当前 N 字节"时，大小相等**不代表内容相同**。曾据此推断"偶发"，实为**确定性的基准过期**。要判内容，必须下载产物比**哈希**或**逐像素**，并给出差异包围盒与前几个差异像素的颜色。

### 6.4 参考文档的警告是证据（第 416 轮换来的）

参考文档里**明确警告某种做法会损坏文件**时，那条警告**就是证据**。

1. **不要实现被警告的做法。** 若仍要偏离，**在测量结果回来之前**写下偏离的理由。曾有一片读到"原地改写会留下陈旧字节并**损坏**文件，正确做法是**重设记录大小**"这条警告，却仍实现了原地改写；产出的文件**正好以文档描述的方式损坏**，表现为用户可见的缺陷（第二条轨道没有音符）。
2. **第一处侥幸成功不是成功。** 同一片里第一条轨道"能用"，只是因为改写的名字长度**恰好**对齐；第二条轨道名字长 6 字节就坏了。**同一个代码路径在两组输入上表现不同时，先怀疑运气，不要怀疑输入。**
3. **"缺件"假设要有字节证据。** 猜测"缺少某条记录"（摆放事件）是错的：那条记录**完好且与供体逐字节相同**。定位缺陷应当**逐记录对比一份构造上正确的参照**，而不是从症状推断缺了什么。
4. **未证的语义要登记，不要补齐。** 供体音符序列里的 `b0`/`b1` 头行编码每条音区的 MIDI 通道，未逆向出来 ⇒ 只登记 `NOTE_EVENT_SHAPE_CAVEAT`，**不发明**这些字节。
