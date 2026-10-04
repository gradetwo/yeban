# 夜半 (Yeban) 专业桌面 DAW 架构与系统设计规范 (Slint + Pure Rust 原生版)

> **项目全称**：夜半 (Yeban) / Yeban DAW  
> **开源许可证**：GNU General Public License v3.0 (GPLv3) 带 CLAP 插件加载附加许可 (GPLv3 §7)  
> **规范版本**：`v3.0-rev6` (2026-10-04)  
> **Depends-on**：ARCHITECTURE v3.0-rev6, LEGAL.md  

> [!IMPORTANT]
> ### 🌟 夜半 (Yeban) 核心工程宪章与研发准则 (Core Mandates)
> 1. **开源与许可协议 (GPLv3)**：本项目在 GitHub 全面开源，遵循 **GNU General Public License v3.0 (GPLv3)** 协议，附带 GPLv3 §7 允许的 CLAP 专有插件动态加载例外条款。第三方基于本项目 Fork 并闭源分发须自行向 SixtyFPS GmbH 获取 Slint 商业许可；
> 2. **原生桌面技术栈 (Slint + Rust)**：纯血 **Slint 响应式矢量 GUI + 纯 Rust 低时延实时音频引擎**。坚决**不做任何 Web / Wasm / AudioWorklet 版本**，彻底摆脱浏览器沙盒与 JavaScript GC 爆音；
> 3. **AI Agent 自主研发范式 (Autonomous AI Agent Development)**：本套设计文档专供 **自主 AI Agent** 消费、理解、编码实施与自动化回归自测，是机器可执行的权威工程基准；
> 4. **零人力工时评估 (Zero Human Staffing Estimation)**：**彻底废除所有传统软件工程的人工人力、人月、人天及工时评估**。全周期以可机械化度量的能力切片（Capability Slices）和八重自动化质量门禁（Quality Gates）为唯一推进与验收标尺。

> **修订记录 (Revision Log)**：  

> - `v3.0-rev6` (2026-10-04)：**系统闭环、数据模型与实时安全深度修正 (依据全量专家评审)**。  
>   1. **双 MCP 进程与状态拓扑闭环 (P0-1)**：确立 `yeban-mcp` 双重形态（活会话内嵌 Streamable HTTP 服务 + 独立批处理 CLI 二进制），引入 `.yeban.lock` 项目文件锁与 Model 唯一写者 Actor 并发模型，彻底解决进程间状态不同步与并发冲突；  
>   2. **Slint 无头与内省三层兜底体系 (P0-2)**：修正 Rust API 规范（`slint::testing::*`），建立自定义 `Platform + SoftwareRenderer` 无头截图、`i-slint-backend-testing` CI 断言与自研 `yeban-ui-mcp` 适配层的三层工程兜底；  
>   3. **路由单一事实源 (P0-3)**：确立 `RoutingGraph` 为唯一声学路由权威，`parent_folder_id` 更名为 `folder_id`（仅负责 UI 折叠），彻底废除 `parent_group_id` 与重复的 `TrackV3.sends`；  
>   4. **确定性集合全面 BTreeMap 化 (P0-4)**：持久化 AST 集合全面替换为 `BTreeMap`，彻底消除 `HashMap` 随机迭代序对 Bit-Exact 渲染与 Git 状态可复现性的破坏；  
>   5. **实时音频安全硬伤修复 (P0-5)**：引入 `EngineSnapshot` 退役回收队列（延迟至主线程释放，彻底消除音频线程 drop 堆内存）、多生产者 SPSC 块边界统一聚合、VU 计量 SPSC 轮询解耦与统一 FTZ/DAZ 浮点初始化；  
>   6. **补齐关键子系统**：新增内部插件延迟补偿架构（PDC 总章 §3.4）、声部窃取策略、端到端 5.0ms 延迟分解预算表、RF64/BW64 + BEXT 广播级 WAV 写入器、ZIP 导入防 zip-slip 路径穿越保护；将 `.als` 导出降级为实验性并清理专有格式（NKI/EXS24/RVC）。  
> - `v3.0-rev5` (2026-10-04)：GPLv3 开源合规深度落地、品牌重塑为“夜半 (Yeban)”，确立 CLAP §7 例外条款与 Slint 双授权声明。  
> - `v3.0-rev4` (2026-10-04)：UI 与引擎深度物理分层解耦。  
> - `v3.0-rev3` (2026-10-04)：重大架构转型，彻底放弃 Web 方案，确立 Slint GUI + Pure Rust 原生桌面路线。  
> - `v3.0-rev2` (2026-10-04)：依据设计评审完成数据模型解耦与确定性分层。  
> - `v3.0-rev1` (2026-10-04)：初始版本。

> **定位**：以 REAPER 7、Bitwig Studio 5 与 Ableton Live 12 为工业级设计参考与架构借鉴的现代化桌面数字音频工作站（DAW）。  
> **设计哲学**：零历史包袱，采用 **Slint 响应式矢量界面 + 纯 Rust 低时延实时音频引擎**，兼顾 **AI Agent 意图生成与双 MCP 闭环** 与 **人类专业级音乐制作体验（硬件级低时延、零 GC 爆音、120 FPS 丝滑响应）**。

---

## 目录
0. [系统总体架构与双 MCP 拓扑闭环](#0-系统总体架构与双-mcp-拓扑闭环)
1. [Slint 响应式矢量 GUI 与低时延事件架构](#1-slint-响应式矢量-gui-与低时延事件架构)
2. [核心数据模型规范 (960 PPQ AST & BTreeMap 确定性状态)](#2-核心数据模型规范-960-ppq-ast--btreemap-确定性状态)
3. [实时音频引擎与内部 PDC 拓扑 (cpal, SPSC & PDC)](#3-实时音频引擎与内部-pdc-拓扑-cpal-spsc--pdc)
4. [离线/无头渲染管线与声学自愈 (Rayon & De-clicking)](#4-离线无头渲染管线与声学自愈-rayon--de-clicking)
5. [声学确定性契约与全格式持久化 (.yeban, RF64, MIDI)](#5-声学确定性契约与全格式持久化-yeban-rf64-midi)
6. [编曲时光机、领域操作日志与版本图谱架构 (Ops Log, Commit DAG & Musical PR)](#6-编曲时光机领域操作日志与版本图谱架构-ops-log-commit-dag--musical-pr)
7. [Yeban Intent API v2 意图协议与 Agent-Computer Interface (ACI)](#7-yeban-intent-api-v2-意图协议与-agent-computer-interface-aci)
8. [纯 Rust Cargo Workspace 代码架构规划](#8-纯-rust-cargo-workspace-代码架构规划)
9. [面向未来的预留接口与解耦规范 (Extensibility & Future-Proofing)](#9-面向未来的预留接口与解耦规范-extensibility--future-proofing)
10. [关键音频子系统与生产级保障规范 (Critical Audio Subsystems)](#10-关键音频子系统与生产级保障规范-critical-audio-subsystems)

---

## 0. 系统总体架构与双 MCP 拓扑闭环

### 0.1 进程与线程拓扑规范 (Process & Threading Topology)

为了彻底解决“外部 Agent 无法 attach 正在运行的 DAW 会话”以及“双进程同时打开工程导致数据损坏”的根本架构矛盾，夜半 (Yeban) 确立了严格的**进程与线程运行拓扑**：

```
+──────────────────────────────────────────────────────────────────────────────────────────────────────────+
|                                  进程形态 1: 主 DAW 桌面进程 (yeban-app)                                  |
|                                                                                                          |
|  [线程 1: UI / 事件循环线程]                 [线程 2: Model 唯一写者 Actor]                              |
|  - Slint 声明式矢量渲染 (120 FPS)           - 独占持有 yeban-model 权威 AST                              |
|  - 拖拽本地交互草稿                          - 处理 OpRequest 队列，生成提交与回退日志                     |
|  - 60Hz 轮询 drain 计量 SPSC 队列更新电平    - 发布不可变 EngineSnapshot 并通知 UI 增量更新                |
|             │                                              ▲                                             |
|             │ (提交用户编辑 Op)                              │ (提交 AI 生成 Op)                           |
|             ▼                                              │                                             |
|  [线程 3: 内嵌活会话 MCP 服务 (yeban-mcp 嵌入模式)] ─────────┘                                             |
|  - 仅绑定 127.0.0.1 (端口可通过环境变量配置，默认 9316)                                                     |
|  - 启动生成随机 Token 鉴权文件 (~/.yeban/session.token)，提供 Streamable HTTP / WebSocket 传输              |
|  - AI Agent attach 当前活会话，调用意图工具直接作用于当前工程内存，即刻触发 UI 局部重绘与音频热更新          |
|                                                                                                          |
|  [线程 4: cpal 实时音频回调线程 (RT Priority)]                                                           |
|  - 多生产者无锁 SPSC 队列块边界聚合 (UI 参数、midir MIDI 输入、插件消息)                                  |
|  - 原子指针交换接入最新 EngineSnapshot；旧快照 move 进退役队列 (由主线程异步回收释放)                       |
|  - 零 malloc / 零 free / 零系统调用；批量处理 (bulk_push / bulk_pop)；FTZ/DAZ 模式生效                     |
|                                                                                                          |
|  [线程 5+: 后台磁盘 I/O、Rayon 离线任务与 CAS 资产管理]                                                    |
+──────────────────────────────────────────────────────────────────────────────────────────────────────────+

+──────────────────────────────────────────────────────────────────────────────────────────────────────────+
|                               进程形态 2: 独立批处理 CLI 进程 (yeban-mcp 独立二进制)                        |
|  - 纯无头、无 GUI 依赖、冷启动 ≤ 20ms                                                                     |
|  - 标准 stdio JSON-RPC 传输 (供外部编排调度器直接 spawn 唤起)                                             |
|  - 独占打开指定 .yeban 工程文件，执行乐理编排、音符处理与极速多核母带导出 (yeban-render)                    |
|  - 退出时保存工程并释放项目文件锁                                                                        |
+──────────────────────────────────────────────────────────────────────────────────────────────────────────+
```

### 0.2 工程文件互斥锁规范 (.yeban.lock)

为杜绝主 DAW 进程与独立 CLI 进程同时写入同一个工程导致文件损坏：
1. **锁文件生成**：任何进程打开 `.yeban` 工程目录或归档容器时，必须在工程同级目录创建 `.yeban.lock`。
2. **锁内容协议**：JSON 编码记录持有者元数据：
   ```json
   {
     "pid": 48215,
     "hostname": "studio-workstation",
     "app_version": "0.3.0",
     "started_at": 1791093600,
     "last_heartbeat": 1791093612
   }
   ```
3. **心跳与陈旧锁抢占 (Heartbeat & Stale Detection)**：
   - 进程每 3 秒刷新一次 `last_heartbeat`；
   - 若检测到锁已存在且 `last_heartbeat` 距当前时间 < 15 秒且对应 PID 活跃，则**拒绝并发打开并报错**；
   - 若 `last_heartbeat` 超过 15 秒或 PID 已不存在（判定为宿主异常崩溃遗留陈旧锁），允许弹出接管提示或由 CLI 参数 `--force-unlock` 安全夺取锁并记录恢复日志。

### 0.3 UI 表现层与 Rust 音频引擎深度解耦

夜半 (Yeban) 的核心架构特征在于 **UI 表现层与 Rust 引擎层的完全解耦**：
- `yeban-model`、`yeban-dsp`、`yeban-theory`、`yeban-render`、`yeban-engine` 等纯 Rust 引擎 crate 本身**绝不依赖任何 GUI 框架或窗口系统库**（零 Slint 引用、零 X11/Wayland/Windows 依赖）。
- 在无图形界面（Headless）环境下，引擎层可以常态化运行，执行音乐逻辑、处理工程状态、运行离线极速渲染，完全不启动任何窗口系统。
- Slint 的职责仅为“当且仅当需要 GUI 交互或 UI 视觉验证时，将引擎状态投影并渲染呈现”。

#### 系统分层职责与无头模式行为规范矩阵

| 层级 | 对应 Crate | 核心职责 | 无头模式行为 | 依赖关系 |
| :--- | :--- | :--- | :--- | :--- |
| **音频引擎层** | `yeban-model`, `yeban-dsp`, `yeban-theory`, `yeban-render`, `yeban-engine` | 权威数据 AST、DSP 算法、乐理走向、离线并行渲染、声卡物理 I/O | **始终独立常驻运行**，与任何 GUI 无关 | 严禁依赖 Slint / 窗口库 |
| **领域 MCP 服务层** | `yeban-mcp` | 暴露音乐意图 API（支持 stdio 批处理模式与 HTTP 活会话 attach 模式） | **始终可用**，可独立为 CLI 或内嵌于 DAW 进程 | 依赖引擎层，零 UI 依赖 |
| **UI 交互表现层** | `crates/yeban-app` | 渲染钢琴卷帘、通道条、调音台、自动化包络等视觉界面 | 生产批处理模式下**跳过初始化**；测试模式以 headless 后端或自定义离屏平台运行 | 依赖引擎层与 Slint |
| **UI 自动化测试层** | `yeban-ui-mcp` / `i-slint-backend-testing` | 元素树审查、无头截图、模拟按键/拖拽测试 | **按需启动**（仅在 UI 自动化测试/视觉验收时激活） | 依托 Slint 公开 API 或测试后端 |

### 0.4 Slint 三层无头与自动化测试兜底体系 (Three-Tier Fallback)

为彻底化解对未正式发布或不稳定上游特性的依赖风险，夜半确立**三层工程兜底保障体系**：
1. **Tier 1 兜底：自定义无头离屏截图 (Custom Platform + SoftwareRenderer)**  
   利用 Slint 官方稳定支持的 `slint::platform::Platform` 与 `SoftwareRenderer` 机制，将 UI 树直接光栅化绘制至内存 Framebuffer 并编码导出 PNG 截图，无需物理显示器，不受 X11/Wayland 环境限制。
2. **Tier 2 兜底：基于官方测试后端直接断言 (In-Memory Testing Backend)**  
   在 CI/CD 单元与集成测试中，直接调用 Slint 官方 Rust API `slint::testing::init_integration_test_backend()` 初始化测试后端，通过 `slint::testing::send_mouse_click()`、`slint::testing::send_keyboard_char()` 注入事件并直接读取组件模型属性断言，完全无需 MCP 网络层中转。
3. **Tier 3 兜底：自研 UI 适配内省服务 (`yeban-ui-mcp`)**  
   若需要外部 AI Agent 动态内省活界面，由项目实现自研轻量适配模块：基于 Slint 公开的组件层级与模型 API，通过 `slint::invoke_from_event_loop` 将元素树坐标几何与属性以 JSON-RPC 格式通过本地 HTTP 服务暴露。

### 核心设计原则 (RFC 2119 规范)
1. **纯血原生单二进制 (MUST)**：全系统基于 Rust 编写，通过 Cargo Workspace 统一构建，彻底剔除 Node.js、V8、Wasm 运行时与浏览器沙盒层，交付极小体积（< 25MB）与极快启动（目标 ≤ 100ms）的原生桌面应用。
2. **单一真实数据源 (MUST)**：权威工程状态 100% 归属于 `crates/yeban-model`。Slint 界面仅持有只读投影，拖拽交互在主线程形成瞬态本地草稿，松手后向数据核心提交原子领域操作（`Op`）。
3. **UI 与引擎物理级解耦 (MUST)**：`yeban-model`、`yeban-dsp`、`yeban-theory`、`yeban-render`、`yeban-engine` 严禁引入任何 Slint 或窗口系统依赖。所有引擎功能必须在完全不初始化 Slint 的无头环境下 100% 正常运行。
4. **实时声学绝对安全 (MUST)**：音频回调线程内严禁执行任何 `malloc`/`free` 堆分配、互斥锁（Mutex）、文件读写、日志输出、动态派发开销与系统阻塞调用。主线程与音频线程间唯一通信渠道为无锁 SPSC 环形队列，旧快照通过退役回收队列延迟到主线程释放。
5. **意图驱动人机协同 (SHOULD)**：通过原生 `yeban-mcp` 向 AI 暴露乐理与曲式编排意图工具，所有 AI 产出均隔离于独立分支，经人类制作人审核后方可合并。
6. **开源合规声明 (MUST)**：本项目整体采用 GPLv3 许可证。UI 层依赖的 Slint 框架采用 GPLv3/商业双授权模式。基于本项目 fork 并希望以非 GPLv3 兼容许可证分发的第三方，须自行向 SixtyFPS GmbH 获取 Slint 商业许可。本项目不提供 Slint 商业许可的任何担保或转授。加载专有 CLAP 插件遵循本项目根目录 `LICENSE` 中的 GPLv3 §7 附加许可例外条款。
7. **默认安全 (Safe-by-default) 哲学 (MUST)**：核心 crate 一律强制声明 `#![forbid(unsafe_code)]`。unsafe 仅允许存在于操作系统硬件音频驱动（cpal/ASIO FFI）、POSIX 共享内存操作与沙盒进程边界，所有 unsafe 代码块必须配有详细的 Safety 不变量前置条件与 Miri / ASAN 自动化测试守卫。

---

## 1. Slint 响应式矢量 GUI 与低时延事件架构

### 1.1 为什么选择 Slint 构筑专业 DAW 界面
- **保留模式（Retained Mode）与局部脏矩形渲染**：不同于即时模式（Immediate Mode，如 egui）每帧强制全屏重绘导致的巨大 CPU 浪费，Slint 仅在响应式属性（Properties）发生变动时触发受影响区域重绘，静态界面下 CPU 占用接近 0%；
- **原生机器码直编**：`.slint` 声明式布局直接通过 `slint-build` 静态编译为 Rust 结构体与原生绘制指令，无脚本引擎开销；
- **硬件加速图形流水线**：底层接入 FemtoVG / Skia / OpenGL 硬件渲染后端，完美支持高刷新率（120 FPS+）显示器与高分屏（Retina / 4K）DPR 缩放。

### 1.2 界面主线程与音频管线无锁解耦模型

```
[Slint UI 主事件循环]           [midir 物理 MIDI 线程]       [沙盒插件 IPC 线程]
        │                               │                           │
        ▼ (UI 交互参数 SPSC)              ▼ (MIDI 事件 SPSC)           ▼ (插件控制 SPSC)
+───────────────────────────────────────────────────────────────────────────────────+
|                 多生产者 SPSC 队列组 (各生产者独占一条无锁 SPSC 环形缓冲)            |
+───────────────────────────────────────────────────────────────────────────────────+
                                        │
                                        │ 块边界聚合出队 (耗时 < 0.05ms，零系统调用)
                                        ▼
                       [cpal 实时音频回调线程 (RT Priority)]
                                        │
                                        │ 1. 原子指针交换获取不可变 EngineSnapshot
                                        │ 2. 旧快照 move 进退役队列 (主线程异步释放，零 free)
                                        │ 3. 实时多轨求和，计算真峰值与 RMS 电平
                                        ▼
                       [VU 计量回传无锁环形队列 (Meter SPSC)]
                                        │
                                        │ UI 线程 60Hz 定时器轮询出队 (或 invoke_from_event_loop)
                                        ▼
                            [Slint VU 响应式属性更新]
                                        │
                                        ▼
                            [Slint 局部脏矩形硬件加速重绘]
```

### 1.3 Slint 无头模式运行机制与内省自动化安全规范

为支持 AI Agent 自动化开发、CI/CD 自动化验证与无窗口服务器渲染，系统提供多层次的无头运行与测试集成：

1. **环境与 Feature 配置**：
   - 编译期按需激活：`--features "ui-test-mcp,slint/renderer-skia"`（针对 `yeban-app`，避免污染全 workspace）；
   - 运行期无头配置：
     ```bash
     SLINT_BACKEND=headless \
     YEBAN_UI_MCP_PORT=9315 \
     cargo test -p yeban-app --features ui-test-mcp
     ```
   - 支持通过 Skia 软件后端在内存离屏 Framebuffer 渲染高保真截图供视觉断言。

2. **UI 元素树内省协议与安全隔离**：
   - **安全绑定铁律**：UI 内省与事件模拟接口**默认关闭**；启用时**仅绑定 `127.0.0.1` 本地回环地址**，绝对禁止监听 `0.0.0.0`；
   - **会话鉴权**：启动时生成高熵随机 Token 写入 `~/.yeban/session.token`（文件权限 `0600`），所有 HTTP JSON-RPC 请求必须携带 `Authorization: Bearer <token>`；
   - **权限分级**：划分为只读审查（`ReadOnly`：树遍历与截图）与交互注入（`Interactive`：模拟点击与按键），默认生产环境严禁开启交互注入；
   - **元素树审查**：AI Agent 可通过 JSON-RPC 查询控件树（Widget Tree），提取坐标、尺寸、可见性及自定义绑定状态（如推子电平、音符方块包围盒）。

3. **CI/CD 原生 Rust 自动化测试 (`i-slint-backend-testing`)**：
   - 官方测试后端 `i-slint-backend-testing` 提供 Rust 原生 API `slint::testing::init_integration_test_backend()`；
   - 单元测试与 CI 流水线通过 `slint::testing::send_mouse_click()` 与 `slint::testing::send_keyboard_char()` 在内存中直接分发事件，断言组件属性同步，零物理显示器依赖，零 MCP 网络协议中间层，极度稳定快速。

---

## 2. 核心数据模型规范 (960 PPQ AST & 三层状态物理隔离)

### 2.1 三层状态物理隔离原则
1. **`ProjectDocument`（持久化文档层）**：包含音乐作品完整乐理 AST、轨道、片段、设备参数、路由关系与提交历史。完整保存于本地 `.yeban`（兼容 `.groove`）工程容器文件中。
2. **`SessionRuntimeState`（挥发性运行时状态层）**：包含当前播放头 Tick、isPlaying、任务进度、插件进程 PID、视窗打开状态等。**严禁持久化存入 Commit**。
3. **`LocalMachineConfig`（本机配置层）**：包含本机声卡物理端口绑定、外部编辑器绝对路径、云端 API Token。敏感凭据存入系统安全密钥链（OS Keychain），工程内仅存引用指针。

### 2.2 核心 Rust 数据结构定义 (crates/yeban-model)

```rust
// crates/yeban-model/src/project.rs
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 工业级时钟基准：一拍 (四分音符) = 960 Ticks
pub const PPQ: u64 = 960;

/// 全局实体统一使用有序 ULID 包装类型 (128-bit，天然时间序，Copy/Ord，类型安全)
#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub ulid::Ulid);

impl EntityId {
    pub fn new() -> Self {
        Self(ulid::Ulid::new())
    }
}

pub type AssetHash = String; // SHA-256 哈希

/// 顶层工程文档 (ProjectDocument) - 唯一权威持久化结构
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YebanProjectV3 {
    pub schema_version: u32, // 固定为 3
    pub id: EntityId,
    pub sample_rate: u32, // 工程基准采样率 (默认 48000)
    pub audio_config: ProjectAudioConfig,
    pub rng_seed: u64, // 确定性随机发生器种子 (确保 probability 等算法跨机位级一致)
    pub pan_law: PanLaw, // 声相衰减律
    pub metadata: ProjectMetadata,
    pub transport: TransportConfig,
    pub sections: BTreeMap<EntityId, SectionV3>,
    pub tracks: BTreeMap<EntityId, TrackV3>,
    pub master_bus_track_id: EntityId,
    pub routing_graph: RoutingGraph, // 唯一声学路由事实源
    pub scenes: BTreeMap<EntityId, SceneV3>,
    pub clip_pool: BTreeMap<EntityId, ClipPoolEntry>,
    pub assets: BTreeMap<AssetHash, AssetMetadata>,
}

/// 兼容性类型别名
pub type GrooveProjectV3 = YebanProjectV3;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectMetadata {
    pub title: String,
    pub artist: Option<String>,
    pub genre_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectAudioConfig {
    pub sample_rate: u32,
    pub bit_depth: BitDepth,
    pub base_buffer_size: u32,
    pub input_latency_samples: u32,
    pub output_latency_samples: u32,
    pub pdc_enabled: bool,
    pub metronome_enabled: bool,
    pub count_in_bars: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitDepth {
    Int16,
    Int24,
    Float32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanLaw {
    Negative3dBEqualPower, // -3dB 等功率正弦余弦曲线 (默认)
    Negative4_5dB,
    Negative6dBLinear,
    ZeroDBSinCos,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TransportConfig {
    pub tempo: f32, // 默认 BPM (20.0 ~ 300.0)
    pub time_signature: (u8, u8), // 如 (4, 4)
    pub tempo_track: Vec<TempoPoint>,
    pub meter_track: Vec<MeterPoint>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TempoPoint {
    pub tick: u64,
    pub bpm: f32,
    pub curve: CurveType,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MeterPoint {
    pub tick: u64,
    pub numerator: u8,
    pub denominator: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum CurveType {
    Jump,
    Linear,
    Exponential,
    Bezier { tension: f32 },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrackV3 {
    pub id: EntityId,
    pub order: String, // Fractional Index 排序键 (如 "a0", "a1")
    pub name: String,
    pub kind: TrackKind,
    pub color: String,
    pub muted: bool,
    pub solo: bool,
    pub solo_safe: bool,
    pub volume_db: Option<f32>, // None 表示 -inf dB
    pub pan: f32,               // -1.0 ~ +1.0
    
    pub folder_id: Option<EntityId>, // 视觉目录树折叠父节点 (严禁参与声学总线路由)
    pub is_collapsed: bool,

    pub instrument: Option<InstrumentDefinition>,
    pub insert_devices: Vec<DeviceDefinition>,
    
    pub arrangement_clips: Vec<ClipPlacement>,
    pub session_slots: BTreeMap<EntityId, Option<EntityId>>, // SceneId -> clipPoolId
    pub take_lanes: Vec<TakeLane>,
    pub macros: Vec<MacroParameter>,
    pub automation_lanes: Vec<AutomationLane>,
    pub follow_section_transpose: bool,

    pub armed: bool,
    pub monitoring: MonitoringMode,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitoringMode {
    Off,
    In,   // 直接穿透监听输入信号
    Auto, // 走带停止或录音激活时监听输入，播放时监听音轨已有内容
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum TrackKind {
    Instrument,
    Audio,
    Group,
    Return,
    Master,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SectionV3 {
    pub id: EntityId,
    pub name: String,
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub color: Option<String>,
    pub order: String,
    pub transpose_semitones: i8,
    pub repeat_count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SceneV3 {
    pub id: EntityId,
    pub name: String,
    pub order: String,
    pub launch_quantization: LaunchQuantization,
    pub follow_action: Option<FollowAction>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchQuantization {
    Global,
    None,
    Bars8,
    Bars4,
    Bars2,
    Bar1,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FollowAction {
    pub chance: f32,
    pub target: FollowActionTarget,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowActionTarget {
    Next,
    Previous,
    First,
    Last,
    Any,
    Other,
    Stop,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TakeLane {
    pub id: EntityId,
    pub name: String,
    pub is_active: bool,
    pub clips: Vec<ClipPlacement>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MacroParameter {
    pub index: u8, // 0~7
    pub name: String,
    pub value: f32, // 0.0 ~ 1.0
    pub mappings: Vec<MacroMapping>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MacroMapping {
    pub target_device_id: EntityId,
    pub parameter_id: EntityId,
    pub min_value: f32,
    pub max_value: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AutomationLane {
    pub id: EntityId,
    pub target: AutomationTarget,
    pub points: Vec<AutomationPoint>,
    pub is_visible: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum AutomationTarget {
    TrackVolume,
    TrackPan,
    DeviceParam { device_id: EntityId, param_id: EntityId },
    Macro { macro_index: u8 },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AutomationPoint {
    pub tick: u64,
    pub value: f32,
    pub curve: CurveType,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeviceDefinition {
    pub id: EntityId,
    pub name: String,
    pub kind: DeviceKind,
    pub enabled: bool,
    pub parameters: BTreeMap<EntityId, f32>,
    pub latency_samples: u32, // 该效果器引入的内部算法延迟 (供 PDC 自动补偿)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum DeviceKind {
    InternalEq,
    InternalCompressor,
    InternalTruePeakLimiter { lookahead_samples: u32, oversampling: u8 },
    ClapPlugin { plugin_id: String },
    Vst3Plugin { class_id: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstrumentDefinition {
    pub id: EntityId,
    pub name: String,
    pub kind: InstrumentKind,
    pub polyphony: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum InstrumentKind {
    PolySynth,
    Sfz { sfz_asset_hash: AssetHash },
    ClapPlugin { plugin_id: String },
    Vst3Plugin { class_id: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClipPoolEntry {
    pub id: EntityId,
    pub name: String,
    pub color: Option<String>,
    pub content_offset_ticks: u64,
    pub loop_config: LoopConfig,
    pub lineage: Option<AiGenerationMetadata>,
    pub content: ClipContent,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoopConfig {
    pub enabled: bool,
    pub start_tick: u64,
    pub end_tick: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ClipContent {
    Midi(ExpressiveMidiData),
    Audio(AudioClipData),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClipPlacement {
    pub placement_id: EntityId,
    pub clip_pool_id: EntityId,
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub is_muted: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoutingGraph {
    pub edges: Vec<RoutingEdge>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoutingEdge {
    pub id: EntityId,
    pub source_track_id: EntityId,
    pub destination_track_id: EntityId,
    pub kind: RoutingKind,
    pub send_gain_db: Option<f32>,
    pub destination_device_id: Option<EntityId>, // 侧链目标
    pub pre_fader: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum RoutingKind {
    Output,
    Send,
    Sidechain,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExpressiveMidiData {
    pub notes: BTreeMap<EntityId, MidiNote>,
    pub cc_events: Vec<ContinuousControllerEvent>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MidiNote {
    pub id: EntityId,
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub pitch: u8,
    pub velocity: u8,
    pub probability: Option<f32>, // 0.0 ~ 1.0 (结合 rng_seed + note.id 确定性计算)
    pub ratchet: Option<u8>,
    pub micro_timing: Option<i16>, // -30 ~ +30 Ticks (微时序偏移)
    pub slide: Option<SlideConfig>,
    pub pitch_bend_curve: Vec<(u64, i16)>, // (tick, cents)
    pub syllable: Option<String>,
    pub phonemes: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SlideConfig {
    pub to_pitch: u8,
    pub duration_ticks: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AudioClipData {
    pub asset_hash: AssetHash,
    pub source_start_frame: u64,
    pub source_duration_frames: u64,
    pub fade_in_ticks: u64,
    pub fade_out_ticks: u64,
    pub pitch_shift_semitones: f32,
    pub warping: WarpConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WarpConfig {
    pub enabled: bool,
    pub mode: WarpMode,
    pub original_bpm: f32,
    pub markers: Vec<TransientMarker>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum WarpMode {
    Repitch,
    Beats,
    Complex,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TransientMarker {
    pub id: EntityId,
    pub source_sample_frame: u64,
    pub target_tick: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AssetMetadata {
    pub hash: AssetHash,
    pub file_name: String,
    pub byte_size: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub total_frames: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AiGenerationMetadata {
    pub proposal_id: EntityId,
    pub model_tag: String,
    pub prompt_summary: String,
    pub seed: u64,
    pub timestamp: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContinuousControllerEvent {
    pub tick: u64,
    pub controller_number: u8,
    pub value: u8,
}
```

---

## 3. 实时音频引擎与内部 PDC 拓扑 (cpal, SPSC & PDC)

### 3.1 cpal 硬件声卡适配与平台事实规范

为确保跨平台音频驱动落地的真实性，系统对各平台底层声卡接口进行严格的分级适配：

1. **Linux 平台规范**：
   - `cpal` 采用成熟稳定的 **ALSA** 后端与可选的 **JACK** 后端。在现代 Linux 发行版（Ubuntu 24.04+、Fedora 40+、Arch）中，由系统的 **`pipewire-alsa`** 与 **`pipewire-jack`** 插件自动接管并转发至 PipeWire 统一多媒体图，无需原生 PipeWire 后端即可达成极低延迟；V3.x 可评估接入 `pipewire-rs` 原生绑定。
   - 调度策略：依靠 Linux 系统级 `rlimits`（配置 `/etc/security/limits.d/audio.conf` 赋予 `rtprio 99`、`memlock unlimited`）或接入 `RTKit` (RealtimeKit DBus 服务) 确保实时线程不被抢占。
2. **Windows 平台规范**：
   - 默认采用 WASAPI 后端；低延迟监听评估引入 `wasapi` crate 实现独占模式（WASAPI Exclusive Mode）与事件驱动缓冲调度；
   - **ASIO 策略**：由于 Steinberg ASIO SDK 存在严格的非自由再分发许可限制，**官方默认构建与发布包禁用 ASIO、不捆绑 ASIO SDK**。项目保留可选编译特性 `--features asio`，允许用户自备 ASIO SDK 源码在本地自行编译；
   - 调度策略：音频回调首帧调用 Windows 多媒体类计划程序服务（MMCSS：`AvSetMmThreadCharacteristicsW("Pro Audio", &task_index)`）。
3. **macOS 平台规范**：
   - 直连 Apple CoreAudio 低延迟 HAL 驱动，CoreAudio 渲染线程由操作系统内核自动赋予最高实时约束；支持制作人选择在系统“音频 MIDI 设置”中预先构建的硬件聚合设备（Aggregate Device）。
4. **底层物理时延测量策略**：
   - `cpal` 的 `OutputStreamTimestamp` 仅反映流缓冲区时间戳，无法代表物理声卡回路延迟；
   - 系统通过底层原生系统 API 实测回路延迟：macOS 查询 CoreAudio `kAudioDevicePropertyLatency` 与 `kAudioStreamPropertyLatency`；Windows 查询 WASAPI `IAudioClient::GetStreamLatency`；Linux 借助 PipeWire/JACK 硬件回环测试。

### 3.2 零分配音频回调与快照退役回收队列

1. **零堆分配 (Zero Malloc) 铁律**：
   - 预分配声部池（Voice Pool，默认 512 声部，可配置至 1024 声部）；
   - 在音频回调执行的 `process()` 循环内，**严禁执行任何堆内存分配、释放或系统调用**；
   - 批量数据交换强制使用 `rtrb::Consumer::read_chunk` / `bulk_pop` 与 `rtrb::Producer::write_chunk` / `bulk_push` 接口，配合栈上预分配的定长临时缓冲区（如 `[f32; 128]`）。
2. **EngineSnapshot 退役回收队列 (Retire Queue)**：
   - 音频线程内使用原子指针读取最新的不可变 `Arc<EngineSnapshot>`；
   - 若检测到拓扑版本更新，音频线程将持有的旧快照 **move** 进预先分配的无锁队列 `rtrb::Producer<Arc<EngineSnapshot>>`；
   - **主线程以 60Hz 轮询从回收队列出队并负责旧快照的 Drop 析构**，彻底杜绝音频线程内发生任何内存释放（Zero Free）！
3. **CPU 浮点模式统一 (FTZ / DAZ)**：
   - 音频线程启动时统一显式开启 Flush-To-Zero (FTZ) 与 Denormals-Are-Zero (DAZ) 模式（x86 设置 MXCSR 寄存器 bits 15 与 6；ARM64 设置 FPCR 寄存器 FZ 位），彻底根除微弱次正规数引起的 CPU 计算指令周期暴增 100 倍与跨架构浮点不一致。
4. **声部窃取策略 (Voice Stealing Protocol)**：
   - 当多音轨音符并发超过声部池上限时，激活确定性声部窃取算法：优先窃取处于 Release 阶段尾部、振幅能量最低（<-60dBFS）或最早被触发的声音；
   - 窃取瞬间对被终止声部强制应用 3ms 快速指数衰减微淡出包络，彻底杜绝爆音。

### 3.3 升余弦平滑与等功率交叉淡化规范
- **循环点微平滑窗**：在小节末端应用 64 采样点升余弦微窗，消除采样波形不连续产生的杂音：
  $$w(n) = \frac{1}{2} \left[1 - \cos\left(\frac{\pi n}{N - 1}\right)\right], \quad n \in [0, N - 1], \; N = 64$$
- **A/B 盲听等功率瞬切**：快捷键 `[` / `]` 触发主线与 AI 提案分支盲听对比时，以 30ms 等功率正弦/余弦窗口在下一拍精确下拍瞬切，音乐走带连续不中断。盲听期间双分支并发渲染，预滚缓冲锁定为 128 采样点。

### 3.4 内部插件延迟补偿架构 (Internal Plugin Delay Compensation, PDC)

为确保真峰值限制器（BS.1770-4 规范需要 4× 过采样滤波与前瞻 Lookahead 缓冲）、外部沙盒商业插件及模拟硬件回路在总线混音时不发生低频相位干涉抵消，系统确立全链路 PDC 架构：

1. **延迟上报规范**：每个插件与内置设备必须精确上报其引入的处理延迟（`DeviceDefinition::latency_samples`）；
2. **DAG 关键路径拓扑分析**：在非实时线程构建 `EngineSnapshot` 时，分析 `RoutingGraph` 中从每个信号源到 Master 总线的所有声学通路，计算各并行分支的累积延迟，确定最长延迟关键路径 $L_{\max}$；
3. **自动补偿对齐 (Delay Alignment)**：
   - 对于累积延迟为 $L_i$ 的并行分支，在进入总线求和节点前自动插入 $D_i = L_{\max} - L_i$ 采样点的环形延迟缓冲（PDC Delay Line）；
   - 实时音频引擎与 Rayon 离线母带渲染器完全共用同一套 PDC 算法，确保主干声部、鼓组并行压缩（New York Compression）与侧链在任何时候绝对相位对齐！

### 3.5 专业监听延迟预算分解表 (@48kHz, 64 Samples Buffer)

| 处理阶段 | 采样点数 | 物理时延 (毫秒) | 硬件与算法职责 |
| :--- | :---: | :---: | :--- |
| **物理声卡输入缓冲** | 64 | **1.33 ms** | ADC 模拟转数字并写入 DMA 环形缓冲 |
| **内部 DSP 拓扑调度** | - | **0.50 ms** | 声部合成、通道条 EQ/压缩与 PDC 对齐计算 |
| **物理声卡输出缓冲** | 64 | **1.33 ms** | DAC 数字转模拟硬件流水线缓冲 |
| **系统驱动与总线余量** | - | **1.84 ms** | 操作系统底层音频管线与驱动调度冗余 |
| **端到端回路总时延** | **-** | **5.00 ms** | **达成专业乐手即兴演奏与录音监听的物理黄金标准** |

---

## 4. 离线/无头渲染管线与声学自愈 (Rayon & De-clicking)

### 4.1 多核并行离线母带渲染器 (`crates/yeban-render`)
- **Rayon 工作窃取拓扑**：针对 32 条及以上音轨，系统基于 `RoutingGraph` 的无环有向图拓扑排序，按音轨依赖层级生成并行工作单元，由 Rayon 线程池多核并行渲染各轨乐器合成器、插件效果链及侧链调制；
- **串行归约确定性铁律 (Serial Reduction)**：在汇聚到父总线及 Master 母带总线时，所有并行音频缓冲区的加法求和严格按照音轨排序键（基于 `EntityId` 的确定性字典序）执行单线程固定顺序串行累加。此举彻底消除了并行多线程由于浮点加法非结合律 `(a + b) + c != a + (b + c)` 带来的最低有效位（LSB）随机漂移；
- **纯 Rust 声学自愈与去爆音 (De-clicking)**：
  - **语音偷取 (Voice Stealing) 与切音平滑**：当合成器复音数超限触发语音偷取或音符急停时，分配器对被偷取声部强制施加 5.0ms 升余弦窗（Raised-Cosine Window: $w(t) = \frac{1}{2}(1 - \cos(\pi t / T))$）或五次多项式平滑窗快速淡出，消除波形瞬断引起的直流阶跃与爆音；
  - **参数自动化平滑滤波**：所有瞬变自动化事件经过单极点低通滤波（$y[n] = (1 - \alpha) x[n] + \alpha y[n-1]$，时间常数 $\tau \approx 5\text{ms}$），根除阶跃断崖引起的咔嗒杂音；
  - **A/B 试听交叉淡化**：30ms 等功率淡入淡出（Equal-Power Crossfade: $\cos / \sin$ 曲线），自动对齐至最近的节拍网格或零交叉点。

### 4.2 性能基准达标指标
- **基准渲染速度目标**：32 轨基准合成参考工程 A 在基准硬件（Apple M2 Pro 12-core / AMD Ryzen 7 7840HS 8-core/16-thread）上的离线母带渲染导出速度目标达成 **≥ 100× 真实时间**（180 秒整曲立体声导出 ≤ 1.8 秒完成）；
- **CI 性能回归门禁**：基于 `iai-callgrind`（精确指令周期与缓存命中分析）与 `criterion` 在基准测试流水线持续监控指令开销，离线渲染性能衰减超过 5% 即阻断 CI 合并。

---

## 5. 声学确定性契约与全格式持久化 (.yeban, .als, MIDI, RF64)

### 5.1 全平台声学确定性分级契约 (Determinism Contract L1 / L2)

夜半 (Yeban) 摒弃不严谨的“绝对跨架构 100% Bit-Exact”口号，确立分级声学确定性工程契约：

- **L1 级确定性（同平台同编译器位级一致，Bit-Exact）**：
  - 条件：相同操作系统与 CPU 架构、锁定 Rust 编译器版本、固定基准指令集（如 `target-cpu=x86-64-v3` 或 Apple Silicon）、统一启用纯 Rust `libm` 数学库、禁用编译器硬件 FMA 融合乘加展开（`-C target-feature=-fma` 或显式严格算子顺序）、固定统一处理块大小（128 采样点）、使用固定种子 PRNG（`rand_xoshiro`）；
  - 保证：离线母带渲染器与实时录音渲染达成立体声 24-bit / 32-bit Float PCM 样本 **100.000% 位级哈希全同 (Bit-Exact)**，杜绝任何偶发随机性。
- **L2 级确定性（跨平台/跨 CPU 架构确定性）**：
  - 条件：x86_64 与 AArch64 跨架构场景下；
  - 保证：因不同芯片微架构的底层 SIMD 矢量化实现微弱差异，系统保证峰值样本绝对误差阈值 $\max(|s_1[n] - s_2[n]|) < 1.0 \times 10^{-6}$（相当于低于 -120 dBFS），人耳与频谱仪完全不可闻，经跨平台 CI runner 自动化对账拦截。

### 5.2 广播级 RF64 / BW64 自研写入器
- 原生支持 EBU Tech 3306 / ITU-R BS.2088 规范的 RF64 / BW64 格式，突破传统标准 RIFF WAV 的 4GB 文件体积极限，无惧数十轨长时录音母带；
- 完整内嵌广播级 `bext` (Broadcast Extension) 元数据块（录制起始时间码、响度元数据 EBU R128、工程 ULID 全局唯一标识）；
- 内置高质量 TPDF（Triangular Probability Density Function）高精抖动算法，在将 32-bit Float 降采样为 24-bit / 16-bit 整数导出时，消除量化非线性截断失真。

### 5.3 标准 `.yeban` 归档容器与安全性防护
- 标准 ZIP 容器存储 `project.json`（BTreeMap 保证键排序字典序确定）、`history.dag`（版本提交树）与 `assets/{sha256}`（CAS 资产池）；
- **Zip-Slip 路径遍历防御 (MUST)**：解包与导入 `.yeban` 归档时，必须对每个 Zip 内部条目的目标路径进行 `canonicalize()` 规范化检查，严禁包含 `..`、绝对根路径或跨卷符号链接，杜绝路径穿越任意文件覆盖漏洞；
- **解压炸弹防御 (Decompression Bomb, MUST)**：限制单个解压条目体积上限（≤ 2 GB），同时限制整体压缩膨胀比率上限（最大不超过 100:1），超限立即拒绝解包并报错，防范恶意归档造成的 DoS 内存耗尽攻击；
- 保持对历史 `.groove` 扩展名的直接识别与无缝迁移解包。

### 5.4 实验性 Ableton Live Set (`.als`) 导出 (`experimental-als-export`)
- 标记为实验性特性，基于 `flate2` 直接生成 Gzip 压缩的 XML 工程结构，兼容 Ableton Live 11/12 子集；
- **映射损失对照表 (Mapping Loss Table)**：
  | 夜半 (Yeban) 实体 | Ableton Live 映射目标 | 转换行为与保真度 | 降级与兜底策略 |
  | :--- | :--- | :--- | :--- |
  | **MIDI 轨 / 音符 / 力度 / 弯音** | `MidiTrack` / `Clip` / `Notes` | 100% 无损对齐，960 PPQ 转 Live 内部时钟 | 无损失 |
  | **通道条音量、声像、静音、独奏** | `MixerDevice` / `Volume`, `Pan`, `Speaker` | 映射至 Live 通道条参数曲线 | 无损失 |
  | **音频剪辑 (Audio Clip)** | `AudioTrack` / `AudioClip` | 相对路径引用，对齐时间轴起止与淡入淡出 | 导出时自动将 CAS 资产打包至工程目录 |
  | **夜半原生内建合成器 (SubSynth 等)** | 暂无等价内置乐器 | 无法直接等价映射至 Live 原生合成器 | 自动提供“音频冻结 (Audio Freeze)”将该轨烘焙为广播级 WAV 导入 |
  | **任意拓扑侧链与复杂路由图** | Ableton 侧链路由 (Live 限制较多) | 直线立体声输出正常映射，复杂反馈网络无法表达 | 降级为主立体声总线输出，输出详细映射警告日志 |

### 5.5 标准 MIDI 0/1 导出
- 基于 `midly` 库实现零堆分配的高速 SMF 序列化导出，支持 Tempo Map 拍速标记、拍号变更与多通道独立分轨导出。

### 5.6 专有格式清理与开源透明
- 明确清理并移除 NKI、EXS24、RVC 等封闭专有私有格式直接解析器，采样资产严格基于公开 SFZ v2 规范与开放标准 PCM WAV/FLAC 资源，确保全链条开源合规与格式透明。

---

## 6. 编曲时光机、领域操作日志与版本图谱架构 (Ops Log, Commit DAG & Musical PR)

### 6.1 操作来源与完整领域操作日志 (`OpOrigin` & `Op`)

```rust
// crates/yeban-model/src/ops.rs

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum OpOrigin {
    UserUi,
    MidirInput,
    McpProposal { proposal_id: EntityId, agent_name: String },
    UndoRedo,
    AutomationPlayback,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StampedOp {
    pub origin: OpOrigin,
    pub timestamp: u64,
    pub op: Op,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Op {
    // 1. 音符级操作
    AddNote { track_id: EntityId, clip_id: EntityId, note: MidiNote },
    DeleteNote { track_id: EntityId, clip_id: EntityId, note_id: EntityId, previous_note: MidiNote },
    MoveNote { track_id: EntityId, clip_id: EntityId, note_id: EntityId, delta_tick: i64, delta_pitch: i8 },
    ModifyNoteVelocity { track_id: EntityId, clip_id: EntityId, note_id: EntityId, old_vel: u8, new_vel: u8 },
    
    // 2. 片段与音轨级操作
    AddClip { track_id: EntityId, clip: ClipV3 },
    RemoveClip { track_id: EntityId, clip_id: EntityId, previous_clip: ClipV3 },
    MoveClip { track_id: EntityId, clip_id: EntityId, old_start_tick: u64, new_start_tick: u64 },
    AddTrack { track: TrackV3 },
    RemoveTrack { track_id: EntityId, previous_track: TrackV3 },
    
    // 3. 路由图操作 (唯一声学真理源)
    ConnectRouting { edge: RoutingEdge },
    DisconnectRouting { source: RoutingEndpoint, destination: RoutingEndpoint, previous_edge: RoutingEdge },
    SetRoutingGain { source: RoutingEndpoint, destination: RoutingEndpoint, old_gain: f32, new_gain: f32 },

    // 4. 设备链与宏参数操作
    InsertDevice { track_id: EntityId, slot_index: usize, device: DeviceDefinition },
    RemoveDevice { track_id: EntityId, slot_index: usize, previous_device: DeviceDefinition },
    SetParam { target: AutomationTarget, old_val: f32, new_val: f32 },
    SetMacro { track_id: EntityId, macro_index: usize, old_val: f32, new_val: f32 },

    // 5. 自动化与曲式操作
    SetAutomationPoint { target: AutomationTarget, point_id: EntityId, old_point: Option<AutomationPoint>, new_point: AutomationPoint },
    RemoveAutomationPoint { target: AutomationTarget, point_id: EntityId, previous_point: AutomationPoint },
    SetSection { section_id: EntityId, old_section: Option<SectionV3>, new_section: SectionV3 },
    SetScene { scene_id: EntityId, old_scene: Option<SceneV3>, new_scene: SceneV3 },

    // 6. 原子批处理
    Batch { ops: Vec<Op>, description: String },
}
```

### 6.2 版本提交与 DAG 图谱 (`Commit`)

```rust
// crates/yeban-model/src/commit.rs

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Commit {
    pub id: EntityId,
    pub parents: Vec<EntityId>,
    pub branch_id: String,
    pub author: String,
    pub message: String,
    pub created_at: u64,
    pub rng_seed: u64,
    pub ops: Vec<StampedOp>,
    pub snapshot_ref: Option<ContentHash>, // 每 256 次提交保存全量快照，其余时间重放 ops
}
```

- **匿名分支分叉机制**：制作人按 `Cmd+Z` 回退数步后执行新编辑，系统自动从回退点长出匿名历史分支（Fork），被撤销的操作 100% 永久保全；
- **单步可逆性**：每个 `Op` 自带反向逻辑，单步撤销耗时 ≤ 0.2ms；
- **宏观 AI 提案一键撤销**：批准 AI 生成的全曲提案后，合并动作封装为单一原子 `Op::Batch`，按一次 `Cmd+Z` 整套 AI 变更瞬间回退。

### 6.3 撤销语义状态转换表 (Undo Semantics State Table)

| 当前交互上下文 | 用户动作 | 状态机响应行为 | 撤销树内部拓扑演进 |
| :--- | :--- | :--- | :--- |
| **主分支连续编辑** | `Cmd+Z` (Undo) | 执行当前指针处 `Op` 的逆操作，还原界面与模型 | 撤销栈指针减 1，历史记录保留 |
| **已回退数步状态** | 用户输入新编辑 | 自动切断前向重做链，生成匿名分支 (Fork) | 从当前指针派生新 Branch，原分叉作为只读孤岛永久保全 |
| **跨 Commit 边界** | 连续 `Cmd+Z` 跨过快照 | 跨越上一个 Commit，继续逆向重放上一 Commit 的 ops | 跨 Commit 树级溯源，零数据丢失 |
| **已接受 AI 提案** | 立即按 `Cmd+Z` | 执行 `Op::Batch` 的反向操作集合 | 整体提案原子回滚至合并前状态，提案分支保留供再次比对 |

---

## 7. Yeban Intent API v2 意图协议与双 MCP 协同体系

### 7.1 双 MCP 进程拓扑与网络安全规范 (Live Session Attach vs Headless CLI)

为消除“独立 stdio 进程无法通知运行中 GUI 刷新”与“双进程并发修改导致工程损坏”的核心矛盾，夜半 (Yeban) 将 `yeban-mcp` 设计为兼具库与独立可执行文件的双形态架构：

1. **形态 A：运行态会话挂载 (In-Process Live Attach, yeban-app 内嵌)**：
   - `yeban-app` (Slint GUI 桌面进程) 启动时，直接在进程内初始化 `yeban-mcp` 服务模块；
   - 暴露 **Streamable HTTP JSON-RPC** 传输通道，**强制且仅绑定** 本地环回地址 `127.0.0.1:<PORT>`（默认 9316 或自动分配空闲端口）；
   - **安全默认设计 (Safe-by-Default)**：启动时在 `~/.yeban/session.token` 生成高熵随机 Token（权限锁定为 0600），所有外部 Agent 连接必须携带 `Authorization: Bearer <TOKEN>` 请求头，杜绝未授权本地进程探测；
   - AI Agent（如 Antigravity / Claude Code / Cursor）连接该接口后，直接向应用内部的 `yeban-model` Actor 发送意图命令，触发实时原子更新，并通过内存直连通知 Slint UI 触发局部脏矩形重绘，实现真正的“AI 边改、制作人边看、声卡边响”的实时人机协同。
2. **形态 B：无头批处理与 CI 离线模式 (Standalone Headless CLI, yeban-mcp 二进制)**：
   - 编译为原生纯 Rust 独立命令行工具 `yeban-mcp`，基于标准输入输出 (stdio) 交互；
   - 适用于无 GUI 环境的批量转码、算法编曲流水线与 CI/CD 自动化母带渲染；
   - 运行时独占打开 `.yeban` 工程文件，利用多核 Rayon 极速渲染后退出。
3. **并发排他文件锁 (`.yeban.lock`)**：
   - 无论是形态 A 还是形态 B，打开 `.yeban` 工程时必须原子创建 `.yeban.lock` 排他锁文件（写入当前进程 PID、机器标识与心跳时间戳）；
   - 其它进程尝试打开已锁定的工程将立即返回 `PROJECT_LOCKED` 错误，彻底杜绝多进程并发读写导致工程破坏。

### 7.2 完整意图工具集规范 (yeban-mcp 核心工具)

所有工具均支持 `dryRun: bool`（只读模拟校验）与 `idempotencyKey: String`（幂等重放校验）：

| 工具名称 | 输入参数概要 | 领域语义与副作用 | 幂等性与错误码 |
| :--- | :--- | :--- | :--- |
| `yeban_open_project` | `path: String, readOnly: bool` | 校验排他锁并打开指定 `.yeban` 工程 | 幂等；`PROJECT_LOCKED`, `FILE_NOT_FOUND` |
| `yeban_save_project` | `force: bool` | 将内存权威状态与 CAS 资产原子刷盘 | 幂等；`IO_ERROR`, `DISK_FULL` |
| `yeban_close_project` | `saveFirst: bool` | 保存并释放当前工程与 `.yeban.lock` | 幂等；`NO_ACTIVE_PROJECT` |
| `yeban_query_project` | `limit: u32, offset: u32, fields: Vec<String>` | 分页拉取工程稀疏视图，防止海量音符撑爆 Agent 上下文 | 幂等只读；`INVALID_FIELD_SELECTOR` |
| `yeban_propose_section` | `sectionName: String, stylePreset: String, bars: u32, scale: String, dryRun: bool` | 在隔离分支 `ai/proposal-{ulid}` 创建章节配器骨架与声部连接 | 幂等；`STYLE_NOT_FOUND`, `CYCLE_DETECTED` |
| `yeban_edit_notes` | `trackId: EntityId, clipId: EntityId, ops: Vec<NoteOp>, idempotencyKey: String` | 在指定片段执行音符增删改，自动进行音域与发声数合法性校验 | 幂等；`CLIP_NOT_FOUND`, `OUT_OF_RANGE` |
| `yeban_set_macro` | `trackId: EntityId, macroIndex: usize, value: f32` | 调节乐器宏旋钮，触发级联平滑自动化展开 | 幂等；`TRACK_NOT_FOUND`, `INDEX_OUT_OF_BOUNDS` |
| `yeban_render_master` | `format: String, sampleRate: u32, normalize: bool` | 触发 `yeban-render` 多核并行导出广播级 WAV 并返回哈希与路径 | 幂等；`RENDER_FAILED`, `BUSY` |
| `yeban_merge_proposal` | `proposalId: EntityId, commitMessage: String` | 将审查通过的 AI 提案分支以原子 `Op::Batch` 合并至主分支并广播重绘 | 幂等；`PROPOSAL_NOT_FOUND`, `CONFLICT` |
| `yeban_reject_proposal` | `proposalId: EntityId, reason: String` | 拒绝并归档指定提案分支，释放无用内存快照 | 幂等；`PROPOSAL_NOT_FOUND` |

### 7.3 双 MCP 自动化开发与自测闭环 (Dual MCP Architecture)

夜半 (Yeban) 依托 Slint 的无头基础设施，构建无需实体屏幕的端到端自测验收流水线：

```
+-----------------------------------------------------------------------------------------+
|                        夜半 (Yeban) 双 MCP 服务器协同与自测闭环                           |
+-----------------------------------------------------------------------------------------+
|                                                                                         |
|       [AI Coding / Composing Agent (如 Antigravity / Claude Code / Cursor)]              |
|                             │                                   │                       |
|                             │ 1. 意图编曲与参数操作              │ 3. 视觉审查与事件注入 |
|                             │ (HTTP JSON-RPC :9316)             │ (HTTP JSON-RPC :9315) |
|                             ▼                                   ▼                       |
|              +-----------------------------+     +-----------------------------+        |
|              |    领域 MCP: yeban-mcp      |     |   UI 测试 MCP: slint/mcp    |        |
|              |  - 内嵌于 yeban-app 进程     |     |  - 内嵌于 yeban-app         |        |
|              |  - 音乐意图与乐理规则       |     |  - 远程 UI 元素树内省       |        |
|              |  - 多核并行渲染导出         |     |  - 软件光栅化无头截图输出   |        |
|              +-----------------------------+     +-----------------------------+        |
|                             │                                   │                       |
|                             │ 2. 状态原子提交                   │ 4. 界面重绘与状态投影 |
|                             ▼                                   ▼                       |
|              +-----------------------------------------------------------------+        |
|              |               夜半权威工程数据核心 (yeban-model)                 |        |
|              |   - 960 PPQ AST, Ops Log 撤销树, EngineSnapshot 调度快照         |        |
|              +-----------------------------------------------------------------+        |
|                                                                                         |
+-----------------------------------------------------------------------------------------+
```

#### 双 MCP 自动化自测工作流：
1. **意图注入 (Step 1)**：AI Agent 通过 HTTP JSON-RPC 调用内嵌 `yeban-mcp` 的 `yeban_propose_section`，向工程数据核心提交 16 小节钢琴伴奏；
2. **状态投影与局部重绘 (Step 2)**：`yeban-model` 变更通知 Slint UI（以 `SLINT_BACKEND=headless` 运行），主界面在内存 Framebuffer 触发局部脏矩形更新；
3. **视觉布局内省与无头截图 (Step 3)**：AI Agent 通过 HTTP 访问 Slint 测试 MCP（端口 `9315`），拉取钢琴卷帘的元素树 JSON，核验音符方块的位置与数量；随后调用截图接口获取 Framebuffer PNG 图像；
4. **自主断言与修复闭环 (Step 4)**：AI 视觉模型比对截图，若发现音符方块渲染重叠或边框截断，Agent 立即通过修改 Slint 代码或调用 `yeban_edit_notes` 修正，无需物理显示器介入即可在 CI/CD 中完成端到端自测验收。

---

## 8. 纯 Rust Cargo Workspace 代码架构规划

全工程统一为规范的纯 Rust Workspace，根目录无任何 Node/pnpm/V8 依赖，严格保证 UI 与引擎的物理解耦：

```
yeban/
├── Cargo.toml                      # Workspace 统一清单
├── Cargo.lock                      # [强制版本控制] 确保 100% 可重现构建与 GPLv3 源码追溯
├── LICENSE                         # GPLv3 全文 + GPLv3 §7 CLAP 插件例外条款
├── LEGAL.md                        # 商标免责声明、Slint 双授权政策与合规政策
├── CONTRIBUTING.md                 # DCO 贡献协议、代码门禁与 cargo vendor 分发规范
├── SECURITY.md                     # 安全政策与漏洞披露通道
├── assets/                         # 323 款原声乐器采样图谱与出厂预设
│   └── samples/
│       ├── ATTRIBUTION.md          # 采样资产开源许可与署名清单
│       └── manifest.json
│
└── crates/
    ├── yeban-app/                  # Slint GUI 主程序与桌面窗口宿主 (内嵌 yeban-mcp HTTP 服务)
    │   ├── Cargo.toml              # features = ["slint/mcp", "slint/renderer-skia"]
    │   ├── ui/                     # .slint 声明式矢量界面定义文件 (源码随发布包分发)
    │   │   ├── app.slint           # 主窗口网格与多标签导轨
    │   │   ├── transport.slint     # 走带控制器与时间码显示
    │   │   ├── pianoroll.slint     # 虚拟化钢琴卷帘交互组件
    │   │   └── mixer.slint         # 通道条与多通道调音台
    │   └── src/                    # Slint 属性绑定与后台事件调度 (支持无头跳过 run_event_loop)
    │
    ├── yeban-model/                # [引擎核心] 960 PPQ 数据模型、ULID、Op 操作日志与撤销树 (零 GUI 依赖)
    ├── yeban-theory/               # [引擎核心] 159 种流派规则、和弦走向展开与声部连接算法 (零 GUI 依赖)
    ├── yeban-dsp/                  # [引擎核心] 纯数学 DSP 库 (真峰值限制器, SSL 压缩, 通道条, 808/909) (零 GUI 依赖)
    ├── yeban-engine/               # [引擎核心] cpal 声卡直驱、音频实时调度与 SPSC 环形队列 (零 GUI 依赖)
    ├── yeban-sfz/                  # [引擎核心] 零拷贝 SFZ v2 词法解析器与预分配语音池 (零 GUI 依赖)
    ├── yeban-decode/               # [引擎核心] 基于 symphonia 的全格式解码与 rubato 重采样 (零 GUI 依赖)
    ├── yeban-render/               # [引擎核心] Rayon 多核并行离线母带渲染器与 RF64/MIDI 导出 (零 GUI 依赖)
    ├── yeban-mcp/                  # [业务服务] 兼具库 (供 yeban-app 内嵌) 与独立二进制 (stdio 批处理) (零 GUI 依赖)
    ├── yeban-services/             # [V3.1 外部协同] 外部专业音频软件 (iZotope RX) 联动与 Ping 延迟校准
    ├── yeban-plugin-host/          # [V4.0 崩溃隔离宿主] 跨进程独立崩溃隔离商业插件宿主 (clack + POSIX shm)
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs              # 宿主进程间通信中枢
    │       ├── shm_bridge.rs       # POSIX shm 环形音频帧交换
    │       └── sandbox_worker.rs   # 独立子进程插件加载与崩溃看门狗
    └── yeban-vst/                  # [V4.0 反向插件] 基于 nih-plug 将 yeban-dsp 反向打包为 VST3/CLAP 插件
```

> **`Cargo.lock` 纳入版本控制铁律 (MUST)**：因工作区包含可直接分发的可执行程序（`crates/yeban-app` 与 `crates/yeban-mcp`），`Cargo.lock` 必须强制纳入 Git 跟踪管理，确保构建 100% 可重现，满足 GPLv3 源码追溯与可验证分发要求。

---

## 9. 面向未来的预留接口与解耦规范 (Extensibility & Future-Proofing)

### 9.1 跨进程崩溃隔离商业插件宿主 (`yeban-plugin-host`) [V4.0]
- **架构澄清：崩溃隔离宿主 (Crash Isolation Host)**：
  - 本模块定位为**进程级崩溃隔离宿主**，旨在防范第三方专有插件发生段错误（SIGSEGV）、空指针异常或内存泄漏时拖垮 DAW 宿主主进程；
  - **明确声明**：该模块**非操作系统级安全沙箱 (Not an OS Security Sandbox)**，不防范恶意插件对宿主环境发起提权反弹 Shell 攻击，仅提供进程边界防护与稳定性看门狗；
- **插件格式支持策略**：
  - `clack` 作为 CLAP 格式的首选宿主实现（MIT/Apache-2.0，零 C++ FFI 开销）。加载专有商业 CLAP 插件遵循本项目根目录 `LICENSE` 中的 GPLv3 §7 附加许可例外条款；
  - VST3 格式通过 `vst3-sys` FFI 绑定支持，锁定基于 MIT 许可的官方 VST3 SDK 3.8.0+。两者共用同一跨进程隔离基础设施，格式差异仅在插件加载与参数适配层处理；
- **POSIX `shm_open` / Windows MMF 共享内存通信**：宿主与插件独立 worker 进程间通过共享内存交换 32-bit Float 音频采样块与 MIDI 事件，往返时延 **< 0.3ms**；
- **绝对崩溃防护与热重启**：第三方插件段错误崩溃仅导致独立 worker 退出，主工程 100% 稳定运行，界面弹出原地一键热重启并恢复崩溃前参数。

### 9.2 外部专业桌面软件双向热重载 (iZotope RX / Melodyne) [V3.1]
- 制作人右键选中音频选区选择“在 iZotope RX 中编辑”，系统导出广播级 BWF 临时文件并启动跨平台文件监听（`notify` crate）；外部软件按 `Cmd+S` 时，系统在 10ms 内校验哈希、裁去保护区并以新 Take 泳道热重载回时间轴。

### 9.3 模拟硬件效果器回路与一键 Ping 自动延迟校准 (Ping ADC) [V3.1]
- 插入 `ExternalHardwareInsert` 设备，发射 MLS（最大长度序列）声学脉冲测算往返样本延迟，内核自动**超前延迟其他所有并行数字音轨（PDC 自动延迟补偿）**，确保绝对零相位抵消。

### 9.4 Linux Wayland 窗口定位降级规范
- 在 Linux Wayland 环境下，因 `xdg-shell` 协议严格禁止客户端自主设定绝对屏幕坐标，弹出的菜单、右键列表及浮动插件窗口通过 `xdg-positioner` 与 Slint/winit subsurface 子表面机制进行相对定位；若底层合成器不支持复杂子表面，系统平滑降级为工作区内部平铺悬浮窗。

---

## 10. 关键音频子系统与生产级保障规范 (Critical Audio Subsystems)

### 10.1 音频与 MIDI 硬件录音系统
1. **音频录制与前置延迟补偿**：点击录音 Arm 按钮后，音频线程直接从声卡物理输入捕获 PCM 流并写入预分配的无锁环形缓冲区，由后台 I/O 线程刷盘至 CAS 临时暂存；录音停止后，系统依据测得的物理声卡输入延迟与缓冲区大小自动前移音频片段起止点，实现微秒级物理对齐；
2. **物理 MIDI 输入直连**：基于 `midir` 库直连 USB MIDI 键盘与鼓垫，击键即时推入 SPSC 队列触发软音源发声，记录时间戳直接映射至 960 PPQ 整数 Tick。

### 10.2 采样率自适应重采样 (`rubato`)
- 当用户声卡工作在 44.1kHz / 96kHz 而工程设定为 48kHz 时，系统通过 `rubato` 库在输入输出端自动激活 140dB SNR 高精多相重采样滤波，消除音调失真。

### 10.3 音轨冻结 (Freeze) 与分轨母带导出 (Stem Export)
- 支持一键将复杂乐器轨与效果链快速离线烘焙为单个 32-bit Float 广播级 WAV 音频文件，释放实时 CPU 负荷。

### 10.4 弹性拉伸 (Time-Stretching) 算法
- 采用宽松商业友好的 **`signalsmith-stretch`**（MIT 协议）纯 Rust/C++ 绑定，实现高质量瞬态保留、共振峰平移与速度弹性拉伸。

### 10.5 自动保存、紧急草稿与崩溃恢复
- 后台线程每 60 秒将操作日志快照原子写入本地临时目录；若遇意外断电，应用重新打开时自动检测并弹窗恢复未提交草稿。
\n