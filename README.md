# 夜半 (Yeban) — 专业桌面数字音频工作站 (DAW)

> **项目全称**：夜半 (Yeban DAW)  
> **开源许可证**：GNU General Public License v3.0 (GPLv3) 附 CLAP 插件加载附加许可 (GPLv3 §7)  
> **技术基座**：Slint 响应式矢量 GUI + 纯 Rust 低时延实时音频引擎  
> **核心定位**：现代化、高确定性、面向 AI 人机共创的纯血桌面原生专业数字音频工作站（以 REAPER 7、Bitwig Studio 5 与 Ableton Live 12 为工业级设计参考与架构借鉴）

---

> [!IMPORTANT]
> ## 🌟 夜半 (Yeban) 四大核心工程宪章与研发准则 (Core Mandates)
> 
> 1. **开源与许可协议 (Open Source under GPLv3)**：  
>    本项目整体在 GitHub 以 **GNU General Public License v3.0 (GPLv3)** 协议开源，并显式附加 GPLv3 §7 允许的 **CLAP 专有商业插件动态加载例外条款**。UI 依赖框架 Slint 采用 GPLv3/商业双轨授权，夜半自身完全合规；第三方基于本项目 Fork 并闭源分发须自行向 SixtyFPS GmbH 获取 Slint 商业许可。
> 
> 2. **原生桌面技术栈 (Slint + Pure Rust Native Engine, Zero Web)**：  
>    彻底抛弃任何 Web / HTML5 Canvas / Wasm / AudioWorklet 方案，构建纯正的 **Slint 响应式矢量桌面 GUI + 纯 Rust 低时延实时音频引擎**。直连操作系统底层声卡驱动（CoreAudio, WASAPI, PipeWire/ALSA），消除 JavaScript GC 爆音与浏览器沙盒瓶颈，达成专业监听级 ≤ 5.0ms 往返回路时延与 120 FPS 丝滑交互。
> 
> 3. **面向自主 AI Agent 研发范式 (Autonomous AI Agent Development)**：  
>    本项目文档与规范系专门面向 **自主 AI Agent（如 Antigravity / Claude Code / Cursor）** 编写的机器可执行工程契约与架构基准。AI Agent 依据完备的强类型数据模型、双 MCP 内省自测流水线（领域编曲 MCP + Slint UI 视觉 MCP）及严格属性测试实施全自主闭环研发。
> 
> 4. **零人力工时评估 (Zero Human Staffing Estimation)**：  
>    **彻底废除所有传统软件工程的人工人力、人月、人天及工时评估**。全生命周期完全以纯机器可量化度量的端到端**能力切片（Capability Slices）**为实施单元，以八重自动化**质量门禁（Quality Gates）**为唯一推进准绳。

---

## 📚 核心系统架构与设计规范索引

| 规范文档 | 核心职责与涵盖内容 | 规范版本 |
| :--- | :--- | :---: |
| [**系统架构与拓扑设计规范 (ARCHITECTURE)**](file:///home/crow/work/agy/review/GROOVE_V3_ARCHITECTURE_AND_SYSTEM_DESIGN.md) | 双 MCP 运行拓扑闭环、`.yeban.lock` OS 排他锁、960 PPQ AST、`BTreeMap` 确定性状态、`RoutingGraph` 单一声学权威、实时音频退役回收队列、内部 PDC 延迟补偿、5.0ms 时延预算、RF64 写入器、Zip-Slip 安全防御、Ops Log 撤销树 (`ARCH-*`, `MODEL-*`, `MCP-*`) | `v1.0.0-rev1` (起步自 `v0.0.1`) |
| [**工程重构实施路线图 (ROADMAP)**](file:///home/crow/work/agy/review/GROOVE_V3_ENGINEERING_IMPLEMENTATION_ROADMAP.md) | Phase -1 开源合规大扫除、Phase 0 九大技术 Spike 准入验证、Phase 1~4 能力切片推进蓝图、RSK-01 ~ RSK-34 风险登记册、质量门禁体系 (`ROAD-*`, `MUST-GATE-*`, `BASELINE-*`) | `v1.0.0-rev1` (起步自 `v0.0.1`) |
| [**行业深度调研与开源战略 (BENCHMARK)**](file:///home/crow/work/agy/review/GROOVE_V3_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md) | 关键事实核验状态表、六大顶级 DAW 解构、Web DAW 物理局限剖析、Slint 选型决策论证、Rust 音频军火库盘点、ASIO 隔离审计、AI 权重合规、工业融合架构蓝图与 L1/L2 确定性分级契约 | `v1.0.0-rev1` (起步自 `v0.0.1`) |
| [**桌面 UI/UX 布局与交互设计规范 (UI/UX)**](file:///home/crow/work/agy/review/GROOVE_V3_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md) | Slint 声明式视网膜高清自适应网格、Session/Arrangement 双视图同构、10 万音符虚拟化钢琴卷帘、A/B 盲听（2048 采样预滚）、色盲安全视觉 Diff、无头视觉回归动态区域遮罩、UI MCP 三级权限分层 (`UI-*`) | `v1.0.0-rev1` (起步自 `v0.0.1`) |

---

## 🏛️ 开源治理与法律合规体系

| 文件路径 | 法律地位与治理内容 |
| :--- | :--- |
| [**`LICENSE`**](file:///home/crow/work/agy/review/LICENSE) | **GNU General Public License v3.0** 完整文本，附带 **GPLv3 §7 允许的 CLAP 专有商业插件动态加载豁免条款**与用户原创内容豁免。 |
| [**`LEGAL.md`**](file:///home/crow/work/agy/review/LEGAL.md) | 第三方商业商标非附属声明（Ableton, FL Studio 等）、Slint GPLv3/商业双轨授权合规提示、Steinberg VST3 3.8.0+ MIT 路径合规声明与开源权重/采样版权守则。 |
| [**`TRADEMARK.md`**](file:///home/crow/work/agy/review/TRADEMARK.md) | “夜半 / Yeban” 项目商标使用指南与第三方商标非附属声明。 |
| [**`GOVERNANCE.md`**](file:///home/crow/work/agy/review/GOVERNANCE.md) | 社区治理结构（BDFL + 维护者团队）、重大架构决策流程（RFC）与 AI Agent 自主治理红线。 |
| [**`AGENTS.md`**](file:///home/crow/work/agy/review/AGENTS.md) | 自主 AI Agent 研发执行规则、单一权威事实源、绝对禁止项清单 (DoD) 与需求编号字典。 |
| [**`THIRD_PARTY_LICENSES.md`**](file:///home/crow/work/agy/review/THIRD_PARTY_LICENSES.md) | 第三方依赖库、字体（Inter/Fira Code）、图标（Lucide）、权重与乐理数据集许可审计明细表。 |
| [**`deny.toml`**](file:///home/crow/work/agy/review/deny.toml) | `cargo-deny` 自动化合规门禁配置（严格白名单限制，阻断不兼容协议与专有依赖入库）。 |
| [**`CONTRIBUTING.md`**](file:///home/crow/work/agy/review/CONTRIBUTING.md) | DCO 1.1 开发者签名规范、代码风格指南、GPLv3 源码可追溯分发（`Cargo.lock` 跟踪、`cargo vendor` 离线依赖、`.slint` 声明式源文件打包）与实时音频线程零分配红线。 |
| [**`CODE_OF_CONDUCT.md`**](file:///home/crow/work/agy/review/CODE_OF_CONDUCT.md) | 基于 Contributor Covenant 2.1 规范的开源社区行为准则。 |
| [**`SECURITY.md`**](file:///home/crow/work/agy/review/SECURITY.md) | 安全脆弱性报告通道、跨进程共享内存（POSIX shm）崩溃隔离边界声明与实时边界安全。 |
| [**`assets/manifest.json`**](file:///home/crow/work/agy/review/assets/manifest.json) | 工程内置字体、图标、乐理词典等静态资产的 SHA-256 校验与元数据注册表。 |
| [**`assets/models/MANIFEST.json`**](file:///home/crow/work/agy/review/assets/models/MANIFEST.json) | AI 伴奏/人声分离等神经网络权重元数据清单（许可协议、参数量、SHA-256 与外部下载源）。 |
| [**`assets/samples/ATTRIBUTION.md`**](file:///home/crow/work/agy/review/assets/samples/ATTRIBUTION.md) | 内置 323 款原声乐器采样素材开源许可全量 Attribution 审计清单（严格锁定 CC0 / CC-BY / MIT 协议，零专有商业样本混入）。 |

---

## 🤖 纯 Rust 工作区架构与 AI Agent 闭环开发流水线

```
yeban/
├── Cargo.toml                      # Workspace 统一清单
├── Cargo.lock                      # [强制版本控制] 确保 100% 可重现构建与 GPLv3 源码追溯
├── README.md                       # 本索引导航与核心宪章
├── LICENSE                         # GPLv3 全文 + GPLv3 §7 CLAP 插件例外条款
├── LEGAL.md                        # 商标免责、Slint 双授权声明与合规政策
├── TRADEMARK.md                    # 商标使用守则
├── GOVERNANCE.md                   # 社区治理与决策流程
├── AGENTS.md                       # 自主 AI Agent 研发执行规则与规范编号体系
├── THIRD_PARTY_LICENSES.md         # 第三方全量依赖与资产合规审计表
├── deny.toml                       # cargo-deny 自动化开源许可拦截配置
├── CONTRIBUTING.md                 # DCO 1.1 协议、代码门禁与 cargo vendor 分发规范
├── SECURITY.md                     # 安全政策与漏洞披露通道
├── schemas/                        # JSON Schema 机器校验契约 (工程/操作/MCP/资产)
├── assets/                         # 字体、模型权重元数据与采样 Attribution
│
└── crates/
    ├── yeban-app/                  # Slint GUI 主程序与桌面窗口宿主 (内嵌 yeban-mcp HTTP 服务)
    ├── yeban-model/                # [引擎核心] 960 PPQ 数据模型、ULID、BTreeMap AST、Op 日志 (零 GUI 依赖)
    ├── yeban-theory/               # [引擎核心] 159 种流派规则、和弦走向展开与声部连接算法 (零 GUI 依赖)
    ├── yeban-dsp/                  # [引擎核心] 纯数学 DSP 库 (真峰值限制器, SSL 压缩, 通道条, 808/909) (零 GUI 依赖)
    ├── yeban-engine/               # [引擎核心] cpal 声卡直驱、内部 PDC、退役回收队列与无锁 SPSC (零 GUI 依赖)
    ├── yeban-sfz/                  # [引擎核心] 零拷贝 SFZ v2 词法解析器与预分配语音池 (零 GUI 依赖)
    ├── yeban-decode/               # [引擎核心] 基于 symphonia 的全格式解码与 rubato 重采样 (零 GUI 依赖)
    ├── yeban-render/               # [引擎核心] Rayon 多核并行离线母带渲染器与 RF64/MIDI 导出 (零 GUI 依赖)
    ├── yeban-mcp/                  # [业务服务] 兼具库 (供 yeban-app 活会话挂载) 与独立二进制 (stdio 批处理)
    ├── yeban-ui-test-port/         # [测试核心] Slint 无头自动化测试端口与语义控件树内省抽屉 (零物理窗口)
    ├── yeban-ui-mcp/               # [测试服务] 基于 JSON-RPC 的 UI 自动化测试与无头截图 MCP 适配层
    ├── yeban-services/             # [v1.1.0 外部协同] 外部专业音频软件 (iZotope RX) 联动与 Ping 延迟校准
    ├── yeban-plugin-host/          # [v2.0.0 崩溃隔离宿主] 跨进程独立崩溃隔离商业插件宿主 (clack + POSIX shm)
    └── yeban-vst/                  # [v2.0.0 反向插件] 基于 nih-plug 将 yeban-dsp 反向打包为 VST3/CLAP 插件
```

### AI Agent 双 MCP 自动化开发与无头自测流水线

AI Agent 在无物理显示器的环境中，通过双 MCP 架构形成自开发、自驱动与自验证的闭环：

```
+─────────────────────────────────────────────────────────────────────────────────────────+
|                       夜半 (Yeban) AI Agent 全自主驱动开发与测试闭环                      |
+─────────────────────────────────────────────────────────────────────────────────────────+
|                                                                                         |
|       [AI Coding / Composing Agent (如 Antigravity / Claude Code / Cursor)]              |
|                             │                                   │                       |
|                             │ 1. 意图编曲与工程操作              │ 3. 视觉审查与事件注入 |
|                             │ (HTTP JSON-RPC 127.0.0.1)         │ (yeban-ui-mcp 127.0.0.1)
|                             ▼                                   ▼                       |
|              +-----------------------------+     +-----------------------------+        |
|              |    领域 MCP: yeban-mcp      |     | UI 测试 MCP: yeban-ui-mcp   |        |
|              |  - 内嵌于 yeban-app 进程     |     |  - 基于 yeban-ui-test-port  |        |
|              |  - 音乐意图与乐理规则       |     |  - 语义控件树遍历与属性断言 |        |
|              |  - 多核并行离线母带导出     |     |  - 软件光栅化无头截图输出   |        |
|              +-----------------------------+     +-----------------------------+        |
|                             │                                   │                       |
|                             │ 2. 状态原子提交                   │ 4. 界面局部重绘与投影 |
|                             ▼                                   ▼                       |
|              +-----------------------------------------------------------------+        |
|              |               夜半权威工程数据核心 (yeban-model)                 |        |
|              |   - 960 PPQ AST, Ops Log 撤销树, EngineSnapshot 调度快照         |        |
|              +-----------------------------------------------------------------+        |
|                                                                                         |
+─────────────────────────────────────────────────────────────────────────────────────────+
```

1. **Step 1: 业务意图注入**：Agent 通过 HTTP JSON-RPC 连接 `127.0.0.1` 动态端口（安全凭据鉴权），调用 `yeban_propose_section` 生成配器骨架；
2. **Step 2: 状态原子生效**：`yeban-model` 单一写者 Actor 提交原子操作日志，通知 Slint 触发局部脏矩形重绘并更新实时声学快照；
3. **Step 3: 视觉内省与无头快照**：Agent 访问 UI 测试 MCP，通过语义 Element ID 遍历元素树 JSON 校验音符包围盒，并捕获无头 Framebuffer 截图；
4. **Step 4: 动态遮罩与自主闭环**：自动化测试引擎对 VU 表与播放头应用动态遮罩后比对 SSIM（基准目标 ≥ 0.98），若发现异常自动生成修复代码，直至全部通过质量门禁体系。
