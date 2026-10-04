# 夜半 (Yeban) 专业桌面 DAW 架构与系统设计规范 (Slint + Pure Rust 原生版)

> **项目全称**：夜半 (Yeban) / Yeban DAW  
> **开源许可证**：GNU General Public License v3.0 (GPLv3) 带 CLAP 插件加载附加许可 (GPLv3 §7)  
> **规范版本**：`v3.0-rev5` (2026-10-04)  
> **Depends-on**：ARCHITECTURE v3.0-rev5, LEGAL.md  
> **修订记录 (Revision Log)**：  
> - `v3.0-rev5` (2026-10-04)：**GPLv3 开源合规深度落地、品牌重塑为“夜半 (Yeban)”与架构技术准确性修订**。项目命名正式确立为中文“夜半”、英文 "Yeban"；全面采用 GPLv3 许可证并在根目录 `LICENSE` 加入 CLAP 插件加载例外条款（GPLv3 §7 附加许可）；明确 Slint GPLv3 路径的第三方 fork 商业限制；修正 Slint MCP 服务器协议定义（HTTP/JSON-RPC 基于 Protobuf 系统测试内省层）；增加 `rtrb` 批量操作铁律（`bulk_push` / `bulk_pop`）；明确 `cpal` 平台特定延迟查询策略；完善 `yeban-plugin-host` 目录树与 CLAP/VST3 (3.8.0+ MIT) 双宿主实现策略；修正 LaTeX 渲染公式。  
> - `v3.0-rev4` (2026-10-04)：**UI 与引擎深度解耦、无头运行与双 MCP 协同架构落地**。确立纯 Rust 音频引擎层（`yeban-model`、`yeban-dsp`、`yeban-theory`、`yeban-render`）与 Slint GUI 表现层的完全物理隔离；系统性支持 Slint 三类无头运行机制（完全跳过 Slint 初始化、`SLINT_BACKEND=headless` 软件光栅化截图、`i-slint-backend-testing` CI 模拟）；落地“双 MCP 服务器协同架构”（`yeban-mcp` 领域业务意图 API + Slint 内嵌 UI 测试内省 MCP），实现 AI Agent 驱动的无 GUI 批量生产模式与全自动无头 UI 测试闭环。  
> - `v3.0-rev3` (2026-10-04)：**重大技术架构转型**。彻底放弃 Web/Wasm/AudioWorklet/TypeScript/React 路线，全线确立 **Slint GUI + Pure Rust 原生音频引擎** 架构；全工程收敛为纯 Cargo Workspace；解构 Slint 响应式属性与原生音频线程的无锁 SPSC 通信；全面拥抱操作系统共享内存（POSIX shm）实现真实低延迟（<0.3ms）第三方插件崩溃隔离沙盒；系统性补齐录音、硬件声卡直驱、采样率自适应与安全导入规范。  
> - `v3.0-rev2` (2026-10-04)：依据设计评审完成数据模型解耦与确定性分层。  
> - `v3.0-rev1` (2026-10-04)：初始版本。

> **定位**：设计参考 REAPER 7、Bitwig Studio 5 与 Ableton Live 12 等工业级现代化桌面音频工作站（DAW）。  
> **原则**：零历史包袱，彻底废除 V1/V2 双轨割裂；采用 **Slint 原生硬件加速矢量界面 + 纯 Rust 高实时低延迟音频引擎**，兼顾 **AI Agent MCP 极速母带生成（≥100x 真实时间）** 与 **人类专业级音乐创作（硬件级低时延、零 GC 爆音、120 FPS 丝滑响应）**。

---

## 目录
0. [系统总体架构与纯 Rust 原生设计](#0-系统总体架构与纯-rust-原生设计)
1. [Slint 响应式矢量 GUI 与低时延事件架构](#1-slint-响应式矢量-gui-与低时延事件架构)
2. [核心数据模型规范 (960 PPQ AST & 三层状态物理隔离)](#2-核心数据模型规范-960-ppq-ast--三层状态物理隔离)
3. [实时音频引擎与原生 DSP 拓扑 (cpal & SPSC 无锁环形缓冲)](#3-实时音频引擎与原生-dsp-拓扑-cpal--spsc-无锁环形缓冲)
4. [离线/无头渲染管线与声学自愈 (Rayon & De-clicking)](#4-离线无头渲染管线与声学自愈-rayon--de-clicking)
5. [声学确定性契约与全格式持久化 (.yeban, .als, MIDI)](#5-声学确定性契约与全格式持久化-yeban-als-midi)
6. [编曲时光机、领域操作日志与版本图谱架构 (Ops Log, Commit DAG & Musical PR)](#6-编曲时光机领域操作日志与版本图谱架构-ops-log-commit-dag--musical-pr)
7. [Yeban Intent API v2 意图协议与 Agent-Computer Interface (ACI)](#7-yeban-intent-api-v2-意图协议与-agent-computer-interface-aci)
8. [纯 Rust Cargo Workspace 代码架构规划](#8-纯-rust-cargo-workspace-代码架构规划)
9. [面向未来的预留接口与解耦规范 (Extensibility & Future-Proofing)](#9-面向未来的预留接口与解耦规范-extensibility--future-proofing)
10. [关键音频子系统与生产级保障规范 (Critical Audio Subsystems)](#10-关键音频子系统与生产级保障规范-critical-audio-subsystems)

---

## 0. 系统总体架构与纯 Rust 原生设计

```
+----------------------------------------------------------------------------------------------------+
|                         夜半 (YEBAN) 纯 RUST 原生桌面专业 DAW 架构全景                              |
+----------------------------------------------------------------------------------------------------+
|                                                                                                    |
|  [用户交互表现层: Slint GUI (crates/yeban-app)] <--------------------+                             |
|  - 声明式 .slint 响应式属性绑定，编译为本地机器码 (FemtoVG/Skia/GL)    |                             |
|  - 脏矩形局部重绘，空闲 CPU 占用接近 0，120 FPS 视网膜高清自适应       | [Slint 内嵌测试 MCP 服务]    |
|  - Session / Arrangement 双视图同构，30ms 等功率 A/B 盲听切换          | - features = ["slint/mcp"]  |
|  - 【无头模式】：完全跳过初始化，或 headless 软件光栅化渲染            | - 端口: SLINT_MCP_PORT      |
|                                                                        | - UI 树内省、无头截图、     |
|                             │ (主线程事件分发)   ▲ (只读模型投影与电平推送)  交互模拟 (供 CI/Agent)     |
|                             ▼                    │                     +-----------------------------+
|  [数据总线与状态调度中枢 (crates/yeban-model)] ─┘                                                   |
|  - 960 PPQ 整数 Tick 权威 AST，ULID 键控 Map 与 fractional index 排序                                |
|  - 领域操作日志 (Ops Log) 撤销树，三层状态物理隔离 (ProjectDocument / Runtime / LocalConfig)          |
|  - 不可变调度拓扑快照 (EngineSnapshot) 原子指针无锁交换                                               |
|                                                                                                    |
|            │ (原子指针交换与控制消息)                │ (本地 stdio JSON-RPC)                           |
|            ▼                                         ▼                                                |
|  [实时音频引擎 (crates/yeban-engine)]     [领域业务 MCP 服务 (crates/yeban-mcp)]                     |
|  - OS 实时高优先级音频回调线程 (cpal)       - 原生独立二进制，冷启动 ≤ 20ms                            |
|  - SPSC 无锁环形缓冲区 (rtrb / bulk 队列)  - 意图函数调用 (yeban_propose_section 等)                 |
|  - 静态预分配声部池 (零 malloc / 零 GC)     - 提案分支强制隔离 (ai/proposal-*)                         |
|  - 统一 DSP 库 (真峰值限制器, 808/909, SFZ)  - 始终运行，100% 独立于 Slint GUI                         |
|  - cpal 声卡直驱 (CoreAudio / WASAPI / PipeWire)                                                   |
|                                                                                                    |
|            │                                         │                                                |
|            ▼ (POSIX shm 共享内存，<0.3ms 时延)        ▼ (多核并行多轨导出)                             |
|  [商业插件沙盒宿主 (crates/yeban-plugin-host)] [离线母带渲染器 (crates/yeban-render)]                  |
|  - 独立沙盒进程加载 CLAP / VST3 商业插件    - Rayon 多核并行渲染 (≥ 100× 真实时间)                     |
|  - 插件崩溃隔离不闪退，支持原地热重启       - 确定性串行总线归约，Bit-Exact 母带输出                   |
+----------------------------------------------------------------------------------------------------+
```

### 0.1 核心机制：UI 表现层与 Rust 音频引擎深度解耦

夜半 (Yeban) 的核心架构特征在于 **UI 表现层与 Rust 引擎层的完全解耦**：
- `yeban-model`、`yeban-dsp`、`yeban-theory`、`yeban-render`、`yeban-engine` 等纯 Rust 引擎 crate 本身**绝不依赖任何 GUI 框架或窗口系统库**（零 Slint 引用、零 X11/Wayland/Windows 依赖）。
- 在无图形界面（Headless）环境下，引擎层可以常态化运行，执行音乐逻辑、处理工程状态、运行离线极速渲染，完全不启动任何窗口系统。
- Slint 的职责仅为“当且仅当需要 GUI 交互或 UI 视觉验证时，将引擎状态投影并渲染呈现”。

#### 系统分层职责与无头模式行为规范矩阵

| 层级 | 对应 Crate | 核心职责 | 无头模式行为 | 依赖关系 |
| :--- | :--- | :--- | :--- | :--- |
| **音频引擎层** | `yeban-model`, `yeban-dsp`, `yeban-theory`, `yeban-render`, `yeban-engine` | 权威数据 AST、DSP 算法、乐理走向、离线并行渲染、声卡物理 I/O | **始终独立常驻运行**，与任何 GUI 无关 | 严禁依赖 Slint / 窗口库 |
| **领域 MCP 服务层** | `yeban-mcp` | 暴露音乐意图 API（stdio JSON-RPC：音轨、音符、和弦、渲染导出） | **始终运行**，完全独立于 Slint，供 AI Agent 编曲 | 依赖引擎层，零 UI 依赖 |
| **UI 交互表现层** | `crates/yeban-app` | 渲染钢琴卷帘、通道条、调音台、自动化包络等视觉界面 | 无头生产模式下**完全跳过初始化**；测试模式以 headless 后端运行 | 依赖引擎层与 Slint |
| **UI 测试 MCP 层** | Slint 内嵌 MCP Server | UI 元素树远程内省、无头离线截图、模拟按键/拖拽测试 | **按需启动**（仅在 UI 自动化测试/视觉验收时激活） | 内嵌于 Slint 运行时 |

### 0.2 三种无头运行机制实现规范

系统提供三种无头运行路径，覆盖从极速批处理到 UI 自动化测试的全场景：

1. **完全不初始化 Slint (Pure Uninitialized Headless)**：
   - 最轻量、最高性能的无头形态。在 CLI、后台守护进程或自动化任务中，Rust 主程序只调用 `yeban-model`、`yeban-mcp`、`yeban-render` 等引擎 crate，完全跳过 `slint::run_event_loop()` 的调用。
   - Slint 虽被编译进二进制，但在内存中零初始化、零开销、冷启动 ≤ 20ms。适合生产环境 AI Agent 编曲与离线批处理。
2. **Headless 软件渲染后端 (`SLINT_BACKEND=headless`)**：
   - Slint 运行时使用纯软件光栅化渲染器（可配合 `--features slint/renderer-skia` 启用 Skia 软件后端），不连接 X11、Wayland 或 Windows DWM。
   - 界面树完整计算并驻留内存 Framebuffer，支持高保真无头截屏输出，供 AI Agent 检查界面渲染、对齐与色彩。
3. **Testing 专用后端 (`i-slint-backend-testing`)**：
   - 采用 Slint 官方测试后端，通过 `testingInitBackend` 在内存中模拟完整窗口系统与事件循环，专为 CI/CD 管线与单元测试设计。支持在无真实显示器的服务器环境下执行 `show()`、`run_event_loop()` 与事件分发。

### 0.3 系统的两大无头服务形态

基于上述解耦机制，夜半 (Yeban) 支持两种独立的无头运行形态：
- **形态 A：纯 MCP 服务模式 (Pure MCP Service Mode，无 GUI 生产模式)**  
  二进制启动后仅激活音频引擎层与 `yeban-mcp`，Slint 事件循环不启动。AI Agent 通过 stdio JSON-RPC 直接调用乐理生成、音符修改与离线渲染，适用于生产环境后台极速编曲、批处理导出。
- **形态 B：无头 UI 验证测试模式 (Headless UI Testing Mode，双 MCP 闭环测试)**  
  在 CI/CD 或本地自动化验收时，以 `SLINT_BACKEND=headless SLINT_MCP_PORT=9315 cargo run --features slint/mcp` 启动。此时 AI Agent 同时连接双 MCP：通过 `yeban-mcp` 注入音符与工程变更，通过 Slint 内嵌 MCP 获取 UI 元素树与无头截图，形成“意图修改 ➔ 界面重绘 ➔ 视觉内省 ➔ 截图断言”的完整自主验证闭环。

### 核心设计原则 (RFC 2119 规范)
1. **纯血原生单二进制 (MUST)**：全系统基于 Rust 编写，通过 Cargo Workspace 统一构建，彻底剔除 Node.js、V8、Wasm 运行时与浏览器沙盒层，交付极小体积（< 25MB）与极快启动（≤ 100ms）的原生桌面应用。
2. **单一真实数据源 (MUST)**：权威工程状态 100% 归属于 `crates/yeban-model`。Slint 界面仅持有只读投影，拖拽交互在主线程形成瞬态本地草稿，松手后向数据核心提交原子领域操作（`Op`）。
3. **UI 与引擎物理级解耦 (MUST)**：`yeban-model`、`yeban-dsp`、`yeban-theory`、`yeban-render` 严禁引入任何 Slint 或窗口系统依赖。所有引擎功能必须在完全不初始化 Slint 的无头环境下 100% 正常运行。
4. **实时声学绝对安全 (MUST)**：音频回调线程内严禁执行任何 `malloc`/`free` 堆分配、互斥锁（Mutex）、文件读写与系统阻塞调用。主线程与音频线程间唯一通信渠道为无锁单生产者单消费者（SPSC）环形队列。
5. **意图驱动人机协同 (SHOULD)**：通过原生 `yeban-mcp` 向 AI 暴露乐理与曲式编排意图工具，所有 AI 产出均隔离于独立分支，经人类制作人审核后方可合并。
6. **开源合规声明 (MUST)**：本项目整体采用 GPLv3 许可证。UI 层依赖的 Slint 框架采用 GPLv3/商业双授权模式。基于本项目 fork 并希望以非 GPLv3 兼容许可证分发的第三方，须自行向 SixtyFPS GmbH 获取 Slint 商业许可。本项目不提供 Slint 商业许可的任何担保或转授。加载专有 CLAP 插件遵循本项目根目录 `LICENSE` 中的 GPLv3 §7 附加许可例外条款。

---

## 1. Slint 响应式矢量 GUI 与低时延事件架构

### 1.1 为什么选择 Slint 构筑专业 DAW 界面
- **保留模式（Retained Mode）与局部脏矩形渲染**：不同于即时模式（Immediate Mode，如 egui）每帧强制全屏重绘导致的巨大 CPU 浪费，Slint 仅在响应式属性（Properties）发生变动时触发受影响区域重绘，静态界面下 CPU 占用接近 0%；
- **原生机器码直编**：`.slint` 声明式布局直接通过 `slint-build` 静态编译为 Rust 结构体与原生绘制指令，无脚本引擎开销；
- **硬件加速图形流水线**：底层接入 FemtoVG / Skia / OpenGL 硬件渲染后端，完美支持高刷新率（120 FPS+）显示器与高分屏（Retina / 4K）DPR 缩放。

### 1.2 界面主线程与音频管线无锁解耦模型

```
[Slint UI 事件循环 (主线程)]
        │
        │ 1. 旋钮拖拽 / 键盘按键 ➔ 产生实时 MIDI/参数控制事件
        ▼
[SPSC 无锁环形缓冲区 (rtrb / 原子环形队列)]
        │
        │ 2. 原子出队 (耗时 < 0.05ms，零系统调用)
        ▼
[cpal 实时音频回调线程 (高优先级)]
        │
        │ 3. 每 64/128 采样点周期填充硬件缓冲，计算真峰值与 RMS 电平
        ▼
[SPSC 回传无锁环形队列]
        │
        │ 4. 节流推送 VU 电平给 Slint 响应式属性 (60 FPS 平滑动画)
        ▼
[Slint VU 电平表重绘]
```

### 1.3 Slint 无头模式运行机制与内嵌 MCP 远程内省协议

为支持 AI Agent 自动化开发、CI/CD 自动化验证与无窗口服务器渲染，Slint 原生提供了无头模式与远程内省能力：

1. **环境与 Feature 配置**：
   - 编译期激活：`--features slint/mcp`（包含内嵌 MCP 服务器与内省运行时）；
   - 运行期环境变量：
     ```bash
     SLINT_BACKEND=headless \
     SLINT_MCP_PORT=9315 \
     cargo run -p yeban-app --features slint/mcp
     ```
   - 可配置 `--features slint/renderer-skia` 结合 Skia 软件后端提供高保真离线无头截图。

2. **通信协议与内省能力**：
   - Slint 内嵌 MCP 服务器使用 **HTTP/JSON-RPC** 传输层，底层共享基于 **Protobuf** 的系统测试内省层（`IntrospectionState`），支持通过 `ElementHandle` API 进行窗口与元素精确跟踪；
   - **UI 元素树审查**：AI Agent 可远程遍历完整 UI 控件树（Widget Tree），查询元素的坐标、尺寸、透明度、可见性及自定义属性（如音轨名称、推子电平、音符方块边界）；
   - **交互事件模拟**：支持通过 MCP 接口向目标控件分发模拟点击、双击、拖拽与文本输入事件，无缝触发 Slint 回调函数；
   - **无头截屏断言 (Headless Screenshots)**：无需物理显示器即可将当前 Framebuffer 渲染结果直接转储为 PNG 图像，供 AI 视觉模型比对像素级布局差异、检查是否有重叠与排版溢出。

3. **CI/CD Testing 后端自动化测试 (`i-slint-backend-testing`)**：
   - 官方测试后端 `i-slint-backend-testing` 提供 `testingInitBackend`，使测试用例能够在完全没有 X11 / Wayland / Windows 窗口系统的情况下创建窗口、分发真实事件、驱动事件循环并断言属性变化，彻底消除 GUI 测试对物理显示的强依赖。

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
use std::collections::HashMap;

/// 工业级时钟基准：一拍 (四分音符) = 960 Ticks
pub const PPQ: u32 = 960;

/// 全局实体统一使用有序 ULID
pub type EntityId = String;
pub type AssetHash = String; // SHA-256 哈希

/// 顶层工程文档 (ProjectDocument) - 唯一权威持久化结构
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YebanProjectV3 {
    pub schema_version: u32, // 固定为 3
    pub id: EntityId,
    pub metadata: ProjectMetadata,
    pub transport: TransportConfig,
    pub sections: HashMap<EntityId, SectionV3>,
    pub tracks: HashMap<EntityId, TrackV3>,
    pub master_bus_track_id: EntityId,
    pub routing_graph: RoutingGraph,
    pub scenes: HashMap<EntityId, SceneV3>,
    pub clip_pool: HashMap<EntityId, ClipPoolEntry>,
    pub assets: HashMap<AssetHash, AssetMetadata>,
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
pub struct TransportConfig {
    pub tempo: f32, // 默认 BPM (20.0 ~ 300.0)
    pub time_signature: (u8, u8), // 如 (4, 4)
    pub tempo_track: Vec<TempoPoint>,
    pub meter_track: Vec<MeterPoint>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TempoPoint {
    pub tick: u32,
    pub bpm: f32,
    pub curve: CurveType,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MeterPoint {
    pub tick: u32,
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
    pub order: String, // Fractional Index 排序键
    pub name: String,
    pub kind: TrackKind,
    pub color: String,
    pub muted: bool,
    pub solo: bool,
    pub solo_safe: bool,
    pub volume_db: Option<f32>, // None 表示 -inf dB
    pub pan: f32,               // -1.0 ~ +1.0
    
    pub parent_folder_id: Option<EntityId>, // 视觉折叠
    pub parent_group_id: Option<EntityId>,  // 声学总线路由
    pub is_collapsed: bool,

    pub instrument: Option<InstrumentDefinition>,
    pub insert_devices: Vec<DeviceDefinition>,
    pub sends: Vec<SendRoute>,
    
    pub arrangement_clips: Vec<ClipPlacement>,
    pub session_slots: HashMap<u32, Option<EntityId>>, // Slot 序号 -> clipPoolId
    pub take_lanes: Vec<TakeLane>,
    pub macros: Vec<MacroParameter>,
    pub automation_lanes: Vec<AutomationLane>,
    pub follow_section_transpose: bool,
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
pub struct ClipPoolEntry {
    pub id: EntityId,
    pub name: String,
    pub color: Option<String>,
    pub content_offset_ticks: u32,
    pub loop_config: LoopConfig,
    pub lineage: Option<AiGenerationMetadata>,
    pub content: ClipContent,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoopConfig {
    pub enabled: bool,
    pub start_tick: u32,
    pub end_tick: u32,
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
    pub start_tick: u32,
    pub duration_ticks: u32,
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
    pub notes: HashMap<EntityId, MidiNote>,
    pub cc_events: Vec<ContinuousControllerEvent>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MidiNote {
    pub id: EntityId,
    pub start_tick: u32,
    pub duration_ticks: u32,
    pub pitch: u8,
    pub velocity: u8,
    pub probability: Option<f32>, // 0.0 ~ 1.0 (确定性哈希计算)
    pub ratchet: Option<u8>,
    pub micro_timing: Option<i16>, // -120 ~ +120 Ticks
    pub slide: Option<SlideConfig>,
    pub pitch_bend_curve: Vec<(u32, i16)>, // (tick, cents)
    pub syllable: Option<String>,
    pub phonemes: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SlideConfig {
    pub to_pitch: u8,
    pub duration_ticks: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AudioClipData {
    pub asset_hash: AssetHash,
    pub source_start_frame: u64,
    pub source_duration_frames: u64,
    pub fade_in_ticks: u32,
    pub fade_out_ticks: u32,
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
    pub target_tick: u32,
}
```

---

## 3. 实时音频引擎与原生 DSP 拓扑 (cpal & SPSC 无锁环形缓冲)

#### 3.1 cpal 原生硬件声卡直驱架构
- **平台特定优化**：
  - **macOS**：直连 CoreAudio，支持低时延聚合设备；
  - **Windows**：首选专有硬件 ASIO 驱动（利用 `asio-sys`），后备原生 WASAPI 独占模式（Exclusive Mode）；
  - **Linux**：首选现代 PipeWire 原生低延迟流，兼容专业 JACK 实时音频拓扑。
- **线程亲和性与实时优先级**：通过 `thread_priority` 将音频回调线程提升为操作系统最高调度优先级（Real-Time FIFO 优先级），规避系统后台进程抢占导致的欠载爆音。
- **延迟测量策略 (Latency Measurement Strategy)**：`cpal` 不提供设备端到端延迟查询公开 API（`OutputStreamTimestamp` 仅反映音频缓冲区流水线时间戳）。测试门禁中“硬件往返时延 ≤ 5.0ms”的测量须通过平台特定系统 API 或回环测试实现：macOS 使用 CoreAudio `kAudioDevicePropertyLatency`，Windows 使用 WASAPI `IAudioClient::GetStreamLatency`，Linux 使用 PipeWire / JACK 的回环测量工具。`cpal` 自身的缓冲时间戳仅作为参考基线。

### 3.2 零 GC 预分配声部池与 SFZ 静态调度
- 预先静态分配 128 个合成器与采样回放声部（Voice Slot），在音频回调执行的 `process()` 循环内，**严禁执行任何堆内存分配、释放或系统调用**；
- 采样音频流读取：利用多线程后台流式读取环形队列，为每个激活音轨预热首段 2048 采样点，后续大采样由后台磁盘线程分块推入环形缓冲，做到零 I/O 停顿。
- **`rtrb` 批量操作铁律 (MUST)**：在实时音频回调中**严禁使用逐样本的 `push` / `pop` 循环**。必须统一使用 `rtrb::Consumer::read_chunk` / `bulk_pop` 与 `rtrb::Producer::write_chunk` / `bulk_push` 接口，配合栈上预分配的定长临时缓冲区（如 `[f32; 128]`）。每次音频回调仅执行 1-2 次批量原子切片搬移，将系统调用、缓存未命中与分支预测开销降至绝对最低。

### 3.3 升余弦平滑与等功率交叉淡化规范
- **循环点微平滑窗**：在小节末端应用 64 采样点升余弦微窗，消除采样波形不连续产生的杂音：
  $$w(n) = \frac{1}{2} \left[1 - \cos\left(\frac{\pi n}{N - 1}\right)\right], \quad n \in [0, N - 1], \; N = 64$$
- **A/B 盲听等功率瞬切**：快捷键 `[` / `]` 触发主线与 AI 提案分支盲听对比时，以 30ms 等功率正弦/余弦窗口在下一拍精确下拍瞬切，音乐走带连续不中断。

---

## 4. 离线/无头渲染管线与声学自愈 (Rayon & De-clicking)

### 4.1 多核并行极速母带渲染器 (`crates/yeban-render`)
- **Rayon 工作窃取拓扑**：针对 32 条及以上音轨，采用 Rayon 线程池多核并行渲染各轨设备链与声部；
- **串行归约确定性铁律**：主母带总线混音与多轨加法求和严格按照音轨排序键执行**固定顺序串行归约**，规避多线程浮点加法非结合律引起的微小位级漂移，达成绝对的 Bit-Exact 确定性导出。

### 4.2 性能基准达标指标
- 32 轨基准合成参考工程 A 在基准硬件（M2 Pro / Ryzen 7 7840HS）上的离线母带渲染速度达成 **≥ 100× 真实时间**（180 秒曲目 ≤ 1.8 秒完成导出）。

---

## 5. 声学确定性契约与全格式持久化 (.yeban, .als, MIDI)

### 5.1 全平台声学确定性 (Bit-Exact Parity)
在纯 Rust 架构下，离线母带渲染器与本地实时音频引擎使用完全相同的纯 Rust DSP 数学内核（`crates/yeban-dsp`），在统一块大小（128 点）、纯 Rust `libm` 与种子伪随机数约束下，不同操作系统导出 WAV 达到 **100.000% 位级一致**。

### 5.2 工业级格式导出引擎
- **原生 Ableton Live Set (`.als`)**：采用 `flate2` 直接生成 Gzip 压缩的 XML 工程结构，支持一键在 Ableton Live 11/12 中打开；
- **标准 MIDI 0/1**：基于 `midly` 实现零堆分配的高速 SMF 序列化导出；
- **标准 `.yeban` 归档容器**：标准 ZIP 打包 `project.json`、版本提交历史与 CAS 资产池（保持对历史 `.groove` 扩展名的完全解包兼容）。

---

## 6. 编曲时光机、领域操作日志与版本图谱架构 (Ops Log, Commit DAG & Musical PR)

### 6.1 统一操作日志与非线性撤销树

```rust
// crates/yeban-model/src/ops.rs

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Op {
    AddNote { track_id: EntityId, clip_id: EntityId, note: MidiNote },
    DeleteNote { track_id: EntityId, clip_id: EntityId, note_id: EntityId, previous_note: MidiNote },
    MoveNote { track_id: EntityId, clip_id: EntityId, note_id: EntityId, delta_tick: i32, delta_pitch: i8 },
    SetParam { target: AutomationTarget, old_val: f32, new_val: f32 },
    AddTrack { track: TrackV3 },
    RemoveTrack { track_id: EntityId, previous_track: TrackV3 },
    Batch { ops: Vec<Op>, description: String },
}
```

- **匿名分支分叉机制**：制作人按 `Cmd+Z` 回退数步后执行新编辑，系统自动从回退点长出匿名历史分支（Fork），被撤销的操作 100% 永久保全；
- **单步可逆性**：每个 `Op` 自带反向逻辑，单步撤销耗时 ≤ 0.2ms；
- **宏观 AI 提案一键撤销**：批准 AI 生成的全曲提案后，合并动作封装为单一原子 `Op::Batch`，按一次 `Cmd+Z` 整套 AI 变更瞬间回退。

---

## 7. Yeban Intent API v2 意图协议与 Agent-Computer Interface (ACI)

### 7.1 意图驱动接口架构与乐理下沉
所有意图处理与声部连接算法封装于纯 Rust `crates/yeban-theory`，原生独立二进制 `yeban-mcp` 直接响应 JSON-RPC，冷启动 ≤ 20ms：

```json
// 意图调用示例: yeban_propose_section
{
  "name": "yeban_propose_section",
  "description": "在独立提案分支中为指定曲式章节生成配器与乐理内容",
  "inputSchema": {
    "type": "object",
    "properties": {
      "sectionName": { "type": "string" },
      "stylePreset": { "type": "string" },
      "bars": { "type": "integer", "default": 16 },
      "scale": { "type": "string" }
    },
    "required": ["sectionName", "stylePreset"]
  }
}
```

- **提案分支隔离铁律**：AI 生成操作必须在 `ai/proposal-*` 分支发起提交，主分支完全由制作人掌控；
- **交互开销控制**：单次 16 小节段落生成往返 Token 消耗中位数 ≤ 600 Tokens。

### 7.2 核心意图工具集规范 (yeban-mcp stdio 工具)

| 工具名称 | 输入参数 | 领域语义与副作用 | 幂等性保障 |
| :--- | :--- | :--- | :--- |
| `yeban_propose_section` | `sectionName`, `stylePreset`, `bars`, `scale` | 在 `ai/proposal-{ulid}` 分支创建章节配器骨架 | 是 (相同参数生成唯一分支) |
| `yeban_edit_notes` | `trackId`, `clipId`, `ops: Vec<NoteOp>` | 针对指定片段执行音符增删改，自动检测音域合法性 | 是 (基于 ULID 幂等校验) |
| `yeban_set_macro` | `trackId`, `macroIndex`, `value: f32` | 调整宏旋钮，级联平滑插值驱动绑定参数 | 是 |
| `yeban_render_master` | `format`, `sampleRate`, `normalize` | 触发 `yeban-render` 多核并行导出母带并返回路径 | 是 |

### 7.3 双 MCP 服务器协同与 AI Agent 闭环测试架构 (Dual MCP Architecture)

夜半 (Yeban) 首创“领域业务意图 + UI 视觉内省”双 MCP 协同体系，使自主 AI Agent 具备完整的自主开发、自测与交付能力：

```
+-----------------------------------------------------------------------------------------+
|                        夜半 (Yeban) 双 MCP 服务器协同与自测闭环                           |
+-----------------------------------------------------------------------------------------+
|                                                                                         |
|       [AI Coding / Composing Agent (如 Antigravity / Claude Code / Cursor)]              |
|                             │                                   │                       |
|                             │ 1. 意图编曲与参数操作              │ 3. 视觉审查与事件注入 |
|                             │ (stdio JSON-RPC)                  │ (HTTP JSON-RPC :9315) |
|                             ▼                                   ▼                       |
|              +-----------------------------+     +-----------------------------+        |
|              |    领域 MCP: yeban-mcp      |     |   UI 测试 MCP: slint/mcp    |        |
|              |  - 纯 Rust 独立二进制       |     |  - 内嵌于 yeban-app         |        |
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

#### 双 MCP 自动化开发与测试工作流：
1. **意图注入 (Step 1)**：AI Agent 通过 `yeban-mcp` 调用 `yeban_propose_section`，向工程数据核心提交 16 小节钢琴伴奏；
2. **状态投影与局部重绘 (Step 2)**：`yeban-model` 变更通知 Slint UI（以 `SLINT_BACKEND=headless` 运行），主界面在内存 Framebuffer 触发局部脏矩形更新；
3. **视觉布局内省与无头截图 (Step 3)**：AI Agent 通过 HTTP 访问 Slint MCP（端口 `9315`），基于 Protobuf 内省层拉取钢琴卷帘的元素树 JSON，核验音符方块的位置与数量；随后调用截图接口获取 Framebuffer PNG 图像；
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
    ├── yeban-app/                  # Slint GUI 主程序与桌面窗口宿主
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
    ├── yeban-render/               # [引擎核心] Rayon 多核并行离线母带渲染器与 ALS/MIDI 导出 (零 GUI 依赖)
    ├── yeban-mcp/                  # [业务服务] 原生独立二进制 MCP Server (stdio 交互，零 GUI 依赖)
    ├── yeban-services/             # [V3.1 外部协同] 外部专业音频软件 (iZotope RX) 联动与 Ping 延迟校准
    ├── yeban-plugin-host/          # [V4.0 沙盒宿主] 跨进程独立沙盒商业插件宿主 (clack + POSIX shm)
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs              # 宿主进程间通信中枢
    │       ├── shm_bridge.rs       # POSIX shm 环形音频帧交换
    │       └── sandbox_worker.rs   # 独立子进程插件加载与看门狗
    └── yeban-vst/                  # [V4.0 反向插件] 基于 nih-plug 将 yeban-dsp 反向打包为 VST3/CLAP 插件
```

> **`Cargo.lock` 纳入版本控制铁律 (MUST)**：因工作区包含可直接分发的可执行程序（`crates/yeban-app` 与 `crates/yeban-mcp`），`Cargo.lock` 必须强制纳入 Git 跟踪管理，确保构建 100% 可重现，满足 GPLv3 源码追溯与可验证分发要求。

---

## 9. 面向未来的预留接口与解耦规范 (Extensibility & Future-Proofing)

### 9.1 跨进程沙盒化商业插件宿主 (`yeban-plugin-host`) [V4.0]
- **插件格式支持策略**：`clack` 作为 CLAP 格式的首选宿主实现（MIT/Apache-2.0，零 C++ FFI 开销）。VST3 格式通过 `vst3-sys` FFI 绑定支持，锁定基于 MIT 许可的 VST3 SDK 3.8.0+。两者共用同一跨进程沙盒基础设施（`yeban-plugin-host`），格式差异仅在插件加载与进程间协议层处理。跨进程沙盒加载专有 CLAP 插件遵循本项目根目录 `LICENSE` 中的 GPLv3 §7 附加许可例外条款；
- **POSIX `shm_open` / Windows MMF 共享内存**：宿主与插件进程间交换 32-bit Float 音频采样块与 MIDI 事件，往返时延 **< 0.3ms**；
- **绝对崩溃防护与热重启**：第三方插件段错误（SIGSEGV）崩溃仅导致独立 worker 退出，主工程 100% 稳定运行，界面弹出原地热重启按钮。

### 9.2 外部专业桌面软件双向热重载 (iZotope RX / Melodyne) [V3.1]
- 制作人右键选中音频选区选择“在 iZotope RX 中编辑”，系统导出广播级 BWF 临时文件并启动跨平台文件监听（`notify` crate）；外部软件按 `Cmd+S` 时，系统在 10ms 内校验哈希、裁去保护区并以新 Take 泳道热重载回时间轴。

### 9.3 模拟硬件效果器回路与一键 Ping 自动延迟校准 (Ping ADC) [V3.1]
- 插入 `ExternalHardwareInsert` 设备，发射 MLS（最大长度序列）声学脉冲测算往返样本延迟，内核自动**超前延迟其他所有并行数字音轨（PDC 自动延迟补偿）**，确保绝对零相位抵消。

---

## 10. 关键音频子系统与生产级保障规范 (Critical Audio Subsystems)

### 10.1 音频与 MIDI 硬件录音系统
1. **音频录制与前置延迟补偿**：点击录音 Arm 按钮后，音频线程直接从声卡物理输入捕获 PCM 流并写入内存池；录音结束自动依据硬件回环延迟微移音轨，实现与现有伴奏的微秒级物理对齐；
2. **物理 MIDI 输入直连**：基于 `midir` 库直连 USB MIDI 键盘与鼓垫，击键即时推入 SPSC 队列触发软音源发声。

### 10.2 采样率自适应重采样 (rubato)
- 当用户外接声卡工作在 44.1kHz / 96kHz 而工程设定为 48kHz 时，系统通过 `rubato` 库在输入输出端自动激活 140dB SNR 高精多相重采样滤波，消除音调失真。

### 10.3 音轨冻结 (Freeze) 与分轨母带导出 (Stem Export)
- 支持一键将复杂乐器轨与效果链快速离线烘焙为单个 32-bit Float 广播级 WAV 音频文件，释放实时 CPU 负荷。

### 10.4 弹性拉伸 (Time-Stretching) 算法
- 采用宽松商业友好的 **`signalsmith-stretch`**（MIT 协议）纯 Rust/C++ 绑定，实现高质量瞬态保留、共振峰平移与速度弹性拉伸。

### 10.5 自动保存、紧急草稿与崩溃恢复
- 后台线程每 60 秒将操作日志快照原子写入本地临时目录；若遇意外断电，应用重新打开时自动检测并弹窗恢复未提交草稿。\n