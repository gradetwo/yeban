# 夜半 (Yeban) 自主 AI Agent 研发执行规则与契约守则 (Agent Rules & DoD)

> **适用对象**：所有参与夜半 (Yeban DAW) 项目代码生成、重构、审查与测试的自主 AI Agent（如 Antigravity / Cursor / Claude Code 等）。  
> **核心原则**：纯 Rust 原生桌面 + Slint 响应式 GUI + 100% 自动化测试与质量门禁驱动 + 零人工人力评估。

---

## 1. 权威规范来源 (Mandatory Sources of Truth)

所有 Agent 在进行任何任务时，必须以以下文件为单一权威事实源：

1. **架构与系统设计核心 (Normative)**：[`GROOVE_V3_ARCHITECTURE_AND_SYSTEM_DESIGN.md`](file:///home/crow/work/agy/review/GROOVE_V3_ARCHITECTURE_AND_SYSTEM_DESIGN.md)
2. **工程重构路线图与门禁 (Normative)**：[`GROOVE_V3_ENGINEERING_IMPLEMENTATION_ROADMAP.md`](file:///home/crow/work/agy/review/GROOVE_V3_ENGINEERING_IMPLEMENTATION_ROADMAP.md)
3. **桌面 UI/UX 交互设计规范 (Normative)**：[`GROOVE_V3_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md`](file:///home/crow/work/agy/review/GROOVE_V3_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md)
4. **法律合规与许可政策 (Mandatory Legal)**：[`LICENSE`](file:///home/crow/work/agy/review/LICENSE), [`LEGAL.md`](file:///home/crow/work/agy/review/LEGAL.md), [`TRADEMARK.md`](file:///home/crow/work/agy/review/TRADEMARK.md)
5. **安全策略 (Mandatory Security)**：[`SECURITY.md`](file:///home/crow/work/agy/review/SECURITY.md)
6. **行业调研与生态融合 (Informative / Research Only)**：[`GROOVE_V3_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md`](file:///home/crow/work/agy/review/GROOVE_V3_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md)（*注：仅供选型与背景参考，不得直接作为硬性实现规范*）

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

- `ARCH-TOP-*`：进程与线程拓扑规范需求
- `ARCH-SEC-*`：网络、权限与归档安全需求
- `ARCH-RT-*`：实时音频线程零分配与时延预算需求
- `ARCH-DET-*`：L1/L2 声学确定性契约需求
- `ARCH-PDC-*`：内部插件延迟补偿架构需求
- `MODEL-*`：960 PPQ 数据模型与 Ops Log 需求
- `MCP-*`：意图服务与工具集接口需求
- `UI-GRID-*`：Slint 视口与网格布局需求
- `UI-TEST-*`：UI 自动化与无头内省需求
- `UI-A11Y-*`：全键盘操作与无障碍需求
- `MUST-GATE-*`：硬性发布门禁检查项
- `BASELINE-*`：基准性能达标线
