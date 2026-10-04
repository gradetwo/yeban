# 夜半 (Yeban) 自主 AI Agent 研发执行规则与契约守则 (Agent Rules & DoD)

> **适用对象**：所有参与夜半 (Yeban DAW) 项目代码生成、重构、审查与测试的自主 AI Agent（如 Antigravity / Cursor / Claude Code 等）。  
> **核心原则**：纯 Rust 原生桌面 + Slint 响应式 GUI + 100% 自动化测试与质量门禁驱动 + 零人工人力评估。

---

## 1. 权威规范来源 (Mandatory Sources of Truth)

所有 Agent 在进行任何任务时，必须以以下文件为单一权威事实源：

1. **架构与系统设计核心 (Normative)**：[`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`](docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md)
2. **工程重构路线图与门禁 (Normative)**：[`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`](docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md)
3. **桌面 UI/UX 交互设计规范 (Normative)**：[`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md`](docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md)
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
6. **发行特性安全红线**：官方默认 release 构建中严禁默认开启 `mcp-http`、`ui-mcp`、`asio`、`experimental-vst3` 或 `experimental-als-export`，上述特性仅限在特定开发或测试配置中按需开启；
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
- `ARCH-FMT-*`：RF64/BW64 与实验性 `.als` 导出（001–002）
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
- `ROAD-M4-001..010`：Phase 4 MCP / 离线母带 / 分发与门禁切流
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
