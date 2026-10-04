<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/png/yeban-dark-256.png">
    <source media="(prefers-color-scheme: light)" srcset="assets/brand/png/yeban-light-256.png">
    <img src="assets/brand/png/yeban-light-256.png" alt="夜半 (Yeban)" width="120" height="120">
  </picture>
</p>

<p align="center">
  <strong>简体中文</strong> · <a href="README.md">English</a>
  &nbsp;|&nbsp;
  <a href="https://yeban.wangda.today">官网</a> ·
  <a href="docs/README.md">文档索引</a> ·
  <a href="docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md">架构规范</a> ·
  <a href="docs/DEV_WORKFLOW.md">开发工作流</a> ·
  <a href="docs/CI_CD.md">CI/CD</a>
</p>

---

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

---

## 🚀 快速开始

> **环境要求 → 构建 → 运行**，顺序如下。下面的命令都可以直接复制粘贴。

### 环境要求

| | |
| :--- | :--- |
| **Rust** | **1.99.0** —— 由 [`rust-toolchain.toml`](rust-toolchain.toml) 钉死，`rustup` 会自动安装；`rustfmt` + `clippy` 组件同样被钉住。 |
| **操作系统** | macOS（Apple Silicon / Intel）、Linux（X11 或 Wayland）、Windows。 |
| **Linux 构建依赖** | `pkg-config libfontconfig1-dev libfreetype-dev libxkbcommon-dev libwayland-dev libx11-dev libgl1-mesa-dev libasound2-dev`（Debian/Ubuntu 包名；Slint / winit / cpal 链接时需要）。 |
| **macOS 构建依赖** | Xcode Command Line Tools：`xcode-select --install`。 |
| **显示器** | 打开主窗口需要真实显示器；`--headless` 不需要（CI 用的就是它）。 |

### 构建

```bash
git clone https://github.com/gradetwo/yeban.git
cd yeban
cargo build --release                     # 构建整个工作区（Slint + winit + cpal，首次需要几分钟）
cargo build --release -p yeban-app        # 只构建桌面 GUI 二进制
```

### 运行

```bash
cargo run --release -p yeban-app                     # 打开桌面主窗口（需要显示器）
cargo run --release -p yeban-app -- --headless       # 无需显示器：打印 headless ok 后退出 0
cargo run --release -p yeban-app -- --help           # 全部开关 + 组合语义 + 退出码
cargo run --release -p yeban-app -- --version        # 真实版本（读 Cargo.toml）
```

下面这些**都不需要显示器**，在无头机器上可放心跑：

```bash
cargo run --release -p yeban-app -- --save-as demo.yeban                      # 把内置演示工程写成 .yeban
cargo run --release -p yeban-app -- --open demo.yeban --headless              # 打开它，并打印真的读到了什么
cargo run --release -p yeban-app -- --open demo.yeban --save-as copy.yeban    # 另存为：原子替换，归档保真
cargo run --release -p yeban-app -- --open demo.yeban --export-elements e.txt # 导出语义元素清单给脚本/AI
cargo run --release -p yeban-app -- --dump-elements                           # 同一份清单打到 stdout
cargo run --release -p yeban-app -- --project-sample filled --save-as f.yeban  # 换一个内置样本
cargo run --release -p yeban-app -- --print-shortcuts                         # 快捷键策略表
SLINT_BACKEND=headless cargo run --release -p yeban-app -- --headless          # 规范里点名的命令行
```

MCP（Intent API v2）服务：

```bash
cargo run --release -p yeban-mcp                                                    # stdio 形态
cargo run --release -p yeban-mcp --features mcp-http -- --enable-mcp-http           # 仅环回 HTTP（127.0.0.1 + 动态端口）
```

**CLI 的承诺**（实测行为，不是愿望）：`--open` 失败会退出 **3** 并给出容器的原始原因，**绝不**退化成"打开成空工程"；
`--save-as` 是**原子替换**（临时文件 → `fsync` → `rename`），失败时旧文件一字未改；没有 `--open` 时 `--save-as`
存的是**内置演示工程**并在输出里明说；未知开关退出 **2** 并打印用法。退出码：`0` 成功 · `1` 界面路径 ·
`2` 用法错误 · `3` 打开失败 · `4` 保存失败 · `5` 导出失败。

### 测试

```bash
cargo test --workspace                                  # 单元 + 集成判据（约 1,100 条）
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

> **诚实状态**：实时引擎**现在已经能产生音频**了（它确定性地合成工程里的 MIDI 音符 —— 三个音符的夹具实测
> **102 398 / 102 400 个非零样本**、两次运行逐位相同，且实时窗口的**零分配**约束仍然成立）。
> 它还**不做**：滤波器/音色参数、母线限制器、声相定律、走带控制，这些判据里也**还没有声卡路径**；
> 主窗口启动时用的仍是演示工程。"到底什么算已验证、什么仍是 `PENDING`"请看
> [`docs/ledger/gate-status.md`](docs/ledger/gate-status.md)。

### 参与开发

本仓库由**多条并行工作线**推进（git worktree + 一文件一写者），且**只有 CI 的判决算数**。
第一次提交前请读 [`docs/DEV_WORKFLOW.md`](docs/DEV_WORKFLOW.md)；在笔记本上请用本地门禁而不是全量构建：

```bash
bash scripts/gates/run-gates.sh light        # fmt + 14 条机械红线守卫 + 文档 + 许可清单
bash scripts/dev/cargo-local.sh test -p yeban-model   # 刻意拒绝 --workspace（本机不跑重活）
```

## 📚 核心系统架构与设计规范索引

| 规范文档 | 核心职责与涵盖内容 | 规范版本 |
| :--- | :--- | :---: |
| [**系统架构与拓扑设计规范 (ARCHITECTURE)**](docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md) | 双 MCP 运行拓扑闭环、`.yeban.lock` OS 排他锁、960 PPQ AST、`BTreeMap` 确定性状态、`RoutingGraph` 单一声学权威、实时音频退役回收队列、内部 PDC 延迟补偿、5.0ms 时延预算、RF64 写入器、Zip-Slip 安全防御、Ops Log 撤销树 (`ARCH-*`, `MODEL-*`, `MCP-*`) | `v1.0.0-rev1` (起步自 `v0.0.1`) |
| [**工程重构实施路线图 (ROADMAP)**](docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md) | Phase -1 开源合规大扫除、Phase 0 九大技术 Spike 准入验证、Phase 1~4 能力切片推进蓝图、RSK-01 ~ RSK-34 风险登记册、质量门禁体系 (`ROAD-*`, `MUST-GATE-*`, `BASELINE-*`) | `v1.0.0-rev1` (起步自 `v0.0.1`) |
| [**行业深度调研与开源战略 (BENCHMARK)**](docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md) | 关键事实核验状态表、六大顶级 DAW 解构、Web DAW 物理局限剖析、Slint 选型决策论证、Rust 音频军火库盘点、ASIO 隔离审计、AI 权重合规、工业融合架构蓝图与 L1/L2 确定性分级契约 | `v1.0.0-rev1` (起步自 `v0.0.1`) |
| [**桌面 UI/UX 布局与交互设计规范 (UI/UX)**](docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md) | Slint 声明式视网膜高清自适应网格、Session/Arrangement 双视图同构、10 万音符虚拟化钢琴卷帘、A/B 盲听（2048 采样预滚）、色盲安全视觉 Diff、无头视觉回归动态区域遮罩、UI MCP 三级权限分层 (`UI-*`) | `v1.0.0-rev1` (起步自 `v0.0.1`) |

---

## 🏛️ 开源治理与法律合规体系

| 文件路径 | 法律地位与治理内容 |
| :--- | :--- |
| [**`LICENSE`**](LICENSE) | **GNU General Public License v3.0** 完整文本，附带 **GPLv3 §7 允许的 CLAP 专有商业插件动态加载豁免条款**与用户原创内容豁免。 |
| [**`LEGAL.md`**](LEGAL.md) | 第三方商业商标非附属声明（Ableton, FL Studio 等）、Slint GPLv3/商业双轨授权合规提示、Steinberg VST3 3.8.0+ MIT 路径合规声明与开源权重/采样版权守则。 |
| [**`TRADEMARK.md`**](TRADEMARK.md) | “夜半 / Yeban” 项目商标使用指南与第三方商标非附属声明。 |
| [**`GOVERNANCE.md`**](GOVERNANCE.md) | 社区治理结构（BDFL + 维护者团队）、重大架构决策流程（RFC）与 AI Agent 自主治理红线。 |
| [**`AGENTS.md`**](AGENTS.md) | 自主 AI Agent 研发执行规则、单一权威事实源、绝对禁止项清单 (DoD) 与需求编号字典。 |
| [**`THIRD_PARTY_LICENSES.md`**](THIRD_PARTY_LICENSES.md) | 第三方依赖库、字体（Inter/Fira Code）、图标（Lucide）、权重与乐理数据集许可审计明细表。 |
| [**`deny.toml`**](deny.toml) | `cargo-deny` 自动化合规门禁配置（严格白名单限制，阻断不兼容协议与专有依赖入库）。 |
| [**`CONTRIBUTING.md`**](CONTRIBUTING.md) | DCO 1.1 开发者签名规范、代码风格指南、GPLv3 源码可追溯分发（`Cargo.lock` 跟踪、`cargo vendor` 离线依赖、`.slint` 声明式源文件打包）与实时音频线程零分配红线。 |
| [**`CODE_OF_CONDUCT.md`**](CODE_OF_CONDUCT.md) | 基于 Contributor Covenant 2.1 规范的开源社区行为准则。 |
| [**`SECURITY.md`**](SECURITY.md) | 安全脆弱性报告通道、跨进程共享内存（POSIX shm）崩溃隔离边界声明与实时边界安全。 |
| [**`assets/manifest.json`**](assets/manifest.json) | 工程内置字体、图标、乐理词典等静态资产的 SHA-256 校验与元数据注册表。 |
| [**`assets/models/MANIFEST.json`**](assets/models/MANIFEST.json) | AI 伴奏/人声分离等神经网络权重元数据清单（许可协议、参数量、SHA-256 与外部下载源）。 |
| [**`assets/samples/ATTRIBUTION.md`**](assets/samples/ATTRIBUTION.md) | 内置 323 款原声乐器采样素材开源许可全量 Attribution 审计清单（严格锁定 CC0 / CC-BY / MIT 协议，零专有商业样本混入）。 |

---

## 🔗 项目信息

| | |
| :--- | :--- |
| 官网 (中英双语 / 深浅色自适应) | <https://yeban.wangda.today> — 源码在 **`website` 分支**，用 wrangler 部署到 Cloudflare Workers |
| 联系邮箱 | <yeban@wangda.today> |
| 仓库 | <https://github.com/gradetwo/yeban> |
| 开发工作流 | [`docs/DEV_WORKFLOW.md`](docs/DEV_WORKFLOW.md) — 一树一线、本机轻量 / CI 全量 |
| CI/CD | [`docs/CI_CD.md`](docs/CI_CD.md) — 自动档 + 手动档、判决怎么读回来 |
| 判例与账本 | [`docs/adr/`](docs/adr) · [`docs/DEVELOPMENT_LEDGER.md`](docs/DEVELOPMENT_LEDGER.md) |

---

## 🤖 纯 Rust 工作区架构与 AI Agent 闭环开发流水线

```text
yeban/
├── Cargo.toml                      # Workspace 统一清单（members = crates/*, spikes/*）
│                                   #   └─ 依赖版本的唯一事实源 [workspace.dependencies]
├── Cargo.lock                      # [强制版本控制] 100% 可重现构建与 GPLv3 源码追溯
├── rust-toolchain.toml             # L1 确定性: 钉死 Rust 1.99.0
├── README.md / AGENTS.md           # 人类导航 / AI Agent 执行契约与红线清单
├── LICENSE / LEGAL.md / TRADEMARK.md / GOVERNANCE.md / NOTICE.md / SECURITY.md
│                                   # 治理与法务文件（Agent 不得修改，见 AGENTS.md §2 红线 1）
├── THIRD_PARTY_LICENSES.md         # 第三方全量依赖与资产合规审计表
├── deny.toml                       # cargo-deny 自动化开源许可拦截配置
├── CONTRIBUTING.md / CODE_OF_CONDUCT.md
├── schemas/                        # JSON Schema 机器校验契约 (工程/操作/MCP/资产)
├── assets/
│   ├── brand/                      # 品牌母版 yeban.svg + 10 个变体 + PNG 产物
│   ├── fonts/ models/ samples/     # 字体、权重元数据、采样 Attribution 登记表
│   └── manifest.json
├── scripts/
│   ├── dev/                        # cargo-local.sh / worktree.sh / ci-verdict.sh / changed-crates.py
│   ├── gates/                      # run-gates.sh / validate_schemas.py
│   ├── guards/                     # policy_check.py — 14 条机械红线守卫
│   └── brand/                      # render-logo.sh
├── docs/
│   ├── YEBAN_*.md                  # 四份 Normative 规范
│   ├── DEV_WORKFLOW.md / CI_CD.md  # 操作手册
│   ├── adr/                        # 规范冲突与缺口的裁决留痕
│   ├── ledger/                     # 历史代码复用审计等账本附件
│   ├── DEVELOPMENT_LEDGER.md       # 测量账本 + pending 清单
│   └── skills/                     # yeban-dev-workflow
├── spikes/                         # Phase 0 九大 Spike，各自独立成 crate（可并行）
│   └── spike-01-cpal-latency … spike-09-snapshot-exchange
├── .github/workflows/              # ci.yml(自动) / gates-manual.yml(手动) / site-deploy.yml
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

> `crates/` 与 `spikes/` 是 **glob 成员**：新增 crate 不需要改动根 `Cargo.toml`，
> 因此多条并行工作线永远不会争抢同一个文件；代价是这两个目录下的每个子目录都必须是合法 crate
> （守卫 G09 会拦住"建了目录忘了清单"）。


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
