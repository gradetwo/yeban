# `live-port` 工作线台账 —— **活的 `MainWindow` 上的 UI 控制面**（`ui-mcp` × `yeban-app`）

- **台账类型**：交付映射 / 接线图 / 契约实测 / 判据清单 / 未决项（**不是规范**）
- **工作线**：`line/live-port`（worktree `yeban/.worktrees/live-port`，基线 main `d9a4e9d`）
- **所有者目录**：`crates/yeban-app/**` + `crates/yeban-ui-mcp/**`（本台账是唯一新增的共享区文档）
- **本机纪律**：全程 `scripts/dev/cargo-local.sh`；**没有**在本机编译 Slint
  （`run-gates.sh crate yeban-app|yeban-ui-mcp` 会按设计 SKIP 本机档位）。
  零 Slint 的那一半在本机用 `rustc --edition 2024 --test -D warnings` **真跑**，
  另外用 `clippy-driver` 直接驱动本机 clippy（见 §5.3，这是本线新拿到的手法）。
- **规范来源**：
  - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §12.2 `[UI-TEST-001]`、
    §12.3 `[UI-MCP-001]`、§12.4 `[UI-TEST-002]`、§12.5 `[UI-MCP-002]`/`[UI-MCP-003]`
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §1.3 `[ARCH-UI-003]`、§7 `[ARCH-UI-004]`
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `MUST-GATE-015`
  - `docs/adr/ADR-0001`：**D18**（`SLINT_BACKEND=headless` 不存在）、**D22**（debug info 构建期打开）、
    **D28**（投影层零 Slint + 唯一注入点）、**D29**（控制面方法名与 scope）
- **上游接缝**：`docs/ledger/ui-mcp-notes.md` §8 needs-3、`docs/ledger/app-binding-notes.md`、
  `docs/ledger/app-introspect-notes.md`、`docs/ledger/ui-test-port-notes.md`

> 本文件回答五个问题：**我交付了什么对应哪条规范**、**谁持有谁（含 `Send` 边界）**、
> **端到端怎么被证明的（数字）**、**哪些是本机真跑哪些只有 CI 能判**、**还剩什么没做**。

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件 | 规范 ID | 说明 |
| :--- | :--- | :--- |
| `crates/yeban-ui-mcp/src/live.rs`（**新建**，1331 行） | `ARCH-UI-004` `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` `MUST-GATE-015` | **真实界面上的控制面装配点** `ControlPlane`（把 256-bit token + 六级 scope + 三级权限映射收在一处）+ **端到端读数** `probe`（`ui/tree` → 按语义 ID 找节点 → `ui/node` → `ui/screenshot`）+ `ScreenshotProbe`（base64/字节数/指纹/**IHDR** 四者自洽校验）+ 纯查询辅助（`find_semantic_node` / `find_family_member` / `family_member_count` / `png_ihdr_size`）+ 14 条本机判据 |
| `crates/yeban-ui-mcp/src/lib.rs` | 同上 | 模块地图补一行 + `pub use live::{…}`（13 个公开项） |
| `crates/yeban-ui-mcp/src/surface.rs` | `MUST-GATE-015` | 只改文档：把"`UiSurface` 的实现者"指向 `crate::live::ControlPlane`（原先那处 intra-doc 链接指向一个不存在的 `crate::live`，现在它存在了） |
| `crates/yeban-app/src/live_surface.rs`（**新建**，271 行） | `ARCH-UI-003` `ARCH-UI-004` `MODEL-AST-002` `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` `MUST-GATE-015` ADR-0001 D28 | **app 侧唯一的接线点**：`build_live_ui(project, permission)` 把 `YebanProjectV1` → 投影 → 注册表 → `LivePort::new`（= 装 Tier-1 平台 + `host::build_main_window` + `show()` + 抓运行时树）→ 直接抓一帧留作对照 → `PortAdapter` 升格为 `UiSurface`；`LiveUi::into_control_plane` 把权限**同时**映射到端口三级与六级 scope |
| `crates/yeban-app/src/registry_tree.rs`（**新建**，45 行） | `UI-TEST-001` `UI-MCP-002` | `ElementRegistry` → `ControlTree` 的**唯一**转换实现（纯函数、零 Slint）；被 `live_surface.rs` 与本机探针**同一个文件**复用，因此不存在第二份实现 |
| `crates/yeban-app/tests/live_ui_mcp.rs`（**新建**，369 行） | `ARCH-SEC-002` `UI-TEST-001` `UI-TEST-002` `UI-MCP-001` `UI-MCP-002` `MUST-GATE-015` | **CI 上真的执行**的 4 条端到端判据（见 §4.1） |
| `crates/yeban-app/Cargo.toml` | `AGENTS.md` §2 红线 6 / `MUST-GATE-009` | 新增 `[dev-dependencies] yeban-ui-mcp`（**不**进 release 图，§3.3 有核验命令） |
| `Cargo.lock` + `docs/ledger/dependency-licenses.md` | 门禁 `license_inventory.py --check` | 各改 1 行：lock 里多一条 dev 边（`yeban-app → yeban-ui-mcp`），清单里 `Cargo.lock` 的 SHA-256 前 16 位 `f4bebbf3dd1a02a6 → 43b1899b59117994`。外部包数仍是 **618**，没有新增任何外部依赖 |

**没有新增任何外部依赖**（`serde_json` / `serde` / `thiserror` 早已在根
`[workspace.dependencies]`；`yeban-mcp` / `yeban-ui-test-port` 是同工作区 crate）。
**没有改**：根 `Cargo.toml`、`.github/**`、`scripts/**`、`deny.toml`、`schemas/**`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、其它 `crates/**`、
`spikes/**`、法务文件。

### 1.1 为什么 `ui-mcp` 侧没有再发明一遍"两行接线"

`ui-mcp-notes.md` §8 needs-3 给的是**闭包注入**形态：

```rust
let port = yeban_ui_test_port::render::LivePort::new(size, permission, Some(&registry), build)?;
let surface = yeban_ui_mcp::PortAdapter::new(port, "tier1-live-port", |port| {
    port.window().capture().map_err(|e| yeban_ui_test_port::port::PortError::Capture { message: e.to_string() })
});
```

本线**原样采用**它，只补两件它没做的事（都在 `yeban-app` 侧，没有越界改 `ui-test-port`）：

1. 把"怎么从活窗口拿像素"写成**具名函数** `capture_tier1`，于是执行面类型可以直接写出来
   （`pub type LiveSurface = PortAdapter<LivePort<MainWindow>, fn(&LivePort<MainWindow>) -> Result<Rgb8Image, PortError>>`），
   `LiveUi` 也就能持有它并交给判据；
2. 补上 `ElementRegistry → ControlTree` 的转换（原本只存在于 `src/test_port_adapter.rs`
   的私有函数里，接线方拿不到）。现在它是 `registry_tree.rs` 的公开纯函数，
   **判据与本机探针共用同一个文件**。

`ui-test-port` / `ui-mcp` 的**公开 API 一行都没改**（`PortAdapter` 已经是公开的；
`UiService::new` 也已公开）。本线唯一"新增公开构造点"在 `yeban-ui-mcp::live::ControlPlane`
—— 它存在的原因是**接线方不该自己拼 token 与 scope**（拼错一套就是一条静默的越权路径）。

---

## 2. 接线图：谁持有谁，以及 `Send` 边界

```text
                    YebanProjectV1 （唯一事实源）
                          │
        bridge::from_project ──> ViewState ──> elements::ElementRegistry
             │                                        │
             │                                        └─ registry_tree::control_tree_from_registry
             │                                                    │
             │                                                    ▼
             │                                            ControlTree（静态注册表，无几何）
             └─ scene::from_view ──> DemoScene（视口 1920×1080 + 会话运行态占位）
                                        │
                                        ▼
  LiveControlPlane ──owns──> ControlPlane ──owns──> Box<dyn UiSurface>
       │                        │  （token + ScopeSet + RunMode）        │
       │                        │                                       ▼
       │                        │                            PortAdapter<LivePort<MainWindow>, fn(..)>
       │                        │                                       │ owns
       │                        │                        ┌──────────────┴───────────────┐
       │                        │                        ▼                              ▼
       │                        │              LivePort<MainWindow>            （捕获闭包 = capture_tier1
       │                        │                     │ owns                      → Tier1Window::capture）
       │                        │        ┌────────────┴────────────┐
       │                        │        ▼                         ▼
       │                        │  MainWindow              Tier1Window(Rc<MinimalSoftwareWindow>)
       │                        │  （host::build_main_window，**唯一**注入点 D28）
       │                        └── ui/* 方法落到 UiService::handle_line（原始 JSON-RPC 文本入口）
       └──owns──> Rgb8Image（装配时**直接**抓的一帧，用于"发出去的图 == 窗口的图"这条断言）
```

### 2.1 `Send` 边界（**整条链都不是 `Send`**，这是刻意的）

- `MainWindow` 与 `MinimalSoftwareWindow` 都是 `Rc` 语义；Slint 的平台上下文是**线程局部**的
  （上游 `MinimalSoftwareWindow` 的回归测试注释原文：*"Each test runs on its own thread, so the
  thread-local global context is unset here."*）。⇒ `LivePort<T>` / `PortAdapter` / `ControlPlane`
  **都不是 `Send`**，`Box<dyn UiSurface>` 也**没有**加 `Send` 约束（改了它就会把真实接线排除掉）。
- 与之配套的是 `yeban-ui-mcp` 的环回 HTTP 是**单线程串行 `accept`**（`transport/http.rs` 文件头
  写明这条取舍）：一个执行面、一个线程、一条串行队列。
- **为什么不去"修"这个 `Send`**：要跨线程就得把窗口状态复制一份出来，于是出现**两份事实源**
  —— 一份给渲染、一份给控制面，`ui/screenshot` 与 `ui/tree` 就可能来自不同的帧。
  本仓库对"第二份事实源"的容忍度是零（`[MODEL-AST-002]` 只允许单向投影）。
- **线程纪律（给后来者）**：`slint::platform::set_platform` 每个**线程**只能成功一次
  ⇒ 同一个 `#[test]` 里不能建第二个 `LiveUi`。本线的 4 条 CI 判据因此各建一个（libtest 默认
  一测一线程）。若有人用 `--test-threads=1`，第 2 条起会拿到 `PlatformUnavailable`
  —— 这一条在 `app-binding-notes.md` §8.10 已登记，本线沿用。

---

## 3. 端到端判据：`建立 LivePort → ui/tree → 按语义 ID 查节点 → ui/screenshot`

### 3.1 CI 侧（`crates/yeban-app/tests/live_ui_mcp.rs`，4 条）

| # | 判据 | 断言什么 |
| ---: | :--- | :--- |
| 1 | `live_control_plane_reads_the_project_backed_window_end_to_end` | 用 `yeban_model::samples::filled_project()` 驱动真实窗口：`ui/tree` 可解析（`count == nodes.len()`）→ `track-{i}-header` 的 `label == "轨道 {工程轨道名}"`（**逐字**）→ 族规模 == 工程规模（3 轨道 / 2 段落 / 2 剪辑摆放）→ 每个剪辑摆放 ULID 与每个音符 ULID 都能按语义 ID 寻址 → 投影驱动的族里**一个演示夹具名字都没有** → 截图像素尺寸 == 视口尺寸且 IHDR 一致、非全黑、颜色数 ≥ 8、PNG 字节数 == 解出的字节数、默认遮罩真的生效 → **未遮罩帧的指纹 == 装配时直接抓的那一帧的指纹**（最强的一条：AI 拿到的像素就是窗口产出的像素）→ 连续两次读数逐字节相同 |
| 2 | `live_control_plane_coverage_matches_the_projected_registry` | `ui/coverage` 两个方向：`unknownAtRuntime` **必须为空**；`missingAtRuntime` **非空且 < 全集**（`visible: false` 的分支不进运行时树，[ARCH-UI-005]） |
| 3 | `production_plane_hard_denies_injection_on_the_live_window` | 生产模式 + 真实执行面：`ui/dispatch_key_press` ⇒ `403` 且 `error.data.kind == "forbidden-in-production"`；随后只读路径照常可用 |
| 4 | `interactive_test_mode_plane_injects_into_the_live_window` | 测试模式 + `Interactive`：`ui/methods` 报 `surface = tier1-live-port` / `mode = test` / `injectAllowed = true`；`ui/dispatch_pointer_move` 落到真实窗口（200 + 回显坐标）；注入后仍能截图 |

判据 1 里"标签 == 工程轨道名"这条的**权威数字**来自 `app-binding-notes.md` §5.1/§5.3 的实测
（`track-0-header.label="轨道 Lead"`，CI run 37229660272）—— 本线把它从"app 层的判据"
**搬到了控制面这条路上**：同一个标签现在是**经 JSON-RPC 拿到的**。

### 3.2 本机侧（`/Users/crow/work/music/.live-port-harness/`，仓库之外）

探针**不是复制品**：`probe_check.rs` 用 `#[path]` 指向仓库原件
`crates/yeban-app/src/registry_tree.rs`，并链接仓库原件的
`bridge.rs` / `scene.rs` / `elements.rs`（`app_lib.rs`）与**整个** `yeban-ui-mcp` lib
（`rustc --test` 直接编 `crates/yeban-ui-mcp/src/lib.rs`）。

```bash
cd /Users/crow/work/music/.live-port-harness && bash run.sh     # (1)…(5) 五步全绿
bash clippy.sh                                                  # 零 Slint 部分的 clippy
bash mutate.sh                                                  # 3 处注入 → 变红 → 还原
```

唯一差别是**执行面**：本机是 `FakeWindow`（`Rgb8Image` + `yeban-ui-test-port` 手写的**真 PNG
编码器**，合成几何），CI 是 `LivePort<MainWindow>`（Tier-1 软件光栅化 + 真布局几何 + `.slint`
的 `accessible-label`）。**`probe` 是同一段代码**。

#### 本机实测数字（`bash run.sh`）

```text
=== (3) yeban-ui-mcp 单元判据 (--test)
test result: ok. 66 passed; 0 failed            # 原 52 条 + 本线新增 14 条

=== (5) 工程数据 → 注册表 → 控制面 probe（端到端, 假窗口 + 真 PNG 编码器）
[harness] 工程: 轨道 3 条 / 段落 2 条 / 剪辑 2 条 / 音符 4 个 / bpm 128.00
[harness] 注册表 132 条 (11 个动态区) → 运行时树 132 条
[live-port] surface=tier1-live-port 树 132 节点 (source=runtime) 查 `track-0-header`
            -> role=list-item label="轨道包头 Lead" (期望含 "Lead")
[live-port] ui/screenshot: 1920x1080 maskDynamic=true regions=11 maskEffective=true
            非黑 2070144/2073600 (99.8%) 颜色 4 种 PNG 6222418 字节 (解码 6222418 字节)
            指纹 f5208f0ac7ba8a54 IHDR 1920x1080
[harness] 未遮罩: 指纹 4b3d3e4965884a5e 非黑 2073600 (100%) 颜色 3 种
[harness] ui/coverage: 注册表 132 / 运行时 132 / 缺失 0 / 未登记 0
[harness] 不存在的 ID (probe 层): ElementNotInTree { tree 132 个节点 }
[harness] 不存在的 ID (服务层): HTTP 400 / 码 Some(-32006)
```

**读法（每条都排除了什么）**：

| 数字 | 排除了什么 |
| :--- | :--- |
| 注册表 **132** 条 / **11** 个动态区（演示工程是 184 / 14） | "注册表还是硬编码的演示常量" —— 它跟着**工程规模**走（3 条非主总线轨道） |
| `track-0-header.label` 含 `Lead` | 标签里是**工程数据**（`Lead` 是 `filled_project()` 的轨道名） |
| 族规模 3 / 2 / 2 与 `view.tracks/sections/clips` 逐项相等 | "控件树里的族规模 == 工程规模"（不是界面常量） |
| 工程驱动的族里没有 `scene::TRACK_NAMES`（`鼓`/`贝斯`…） | "界面读的是工程还是 `demo()`" |
| 遮罩帧指纹 `f5208f0ac7ba8a54` ≠ 未遮罩 `4b3d3e4965884a5e` | 遮罩**真的改了发出去的字节**（不是"证据说遮了、图没遮"） |
| `IHDR 1920x1080` == `width/height` 字段 | 证据字段与**真 PNG 字节**一致（尺寸不是回显出来的） |
| PNG **6222418** 字节（= stored deflate 的 1920×1080 定长） | 与 CI 侧 app-binding 的实测**逐字相同**，说明编码路径同一条 |
| 不存在的 ID：probe 层 `ElementNotInTree`、服务层 `-32006` | 两层都**不返回空成功**（§12.2） |

**诚实边界（本机 ≠ CI 的三处）**：

1. 标签前缀不同：本机拿到的是**注册表**标签 `轨道包头 Lead`（`elements.rs`），
   CI 拿到的是 `.slint` 的 `accessible-label` `轨道 Lead`（`arrangement_view.slint:136`）。
   两者都含工程轨道名，**"逐字等于 `.slint` 标签"这一条只有 CI 能判**（本机没有 Slint）。
2. 几何是合成的（`runtime_tree_with_geometry`），因此 `missing_at_runtime == 0`；
   CI 上 `visible: false` 的分支不进树 ⇒ 缺失**必然非空**。
3. 像素是 `Rgb8Image` 手绘的（真 PNG 编码器），**不是**软件光栅化的产物
   ⇒ `[MUST-GATE-015]` 的"真正被渲染过"只有 CI 能判（这正是 `app-introspect` 那条线已有的能力）。

### 3.3 **CI 上的实测数字（真实界面，run 37233109787）**

> 这是本工作线的**核心证据**：同一个 `probe` 代码，执行面换成 `LivePort<MainWindow>`
> 之后在真实界面上的读数。取自 `rust (workspace 全量)` 腿的 `test --workspace` 日志
> （`tests/live_ui_mcp.rs`：`running 4 tests` → **`4 passed`**）。

```text
[live-port] surface=tier1-live-port 树 79 节点 (source=runtime)
            查 `track-0-header` -> role=list-item label="轨道 Lead" (期望含 "Lead")
[live-port] ui/screenshot: 1920x1080 maskDynamic=true regions=6 maskEffective=true
            非黑 2054848/2073600 (99.1%) 颜色 2642 种 PNG 6222418 字节 (解码 6222418 字节)
            指纹 91e06ec5d83fadd3 IHDR 1920x1080
[live-port] 未遮罩帧: 指纹 a360802d81461bee 非黑 2073600 (100%) 颜色 2825 种; 与窗口直抓帧一致
[live-port] ui/coverage: 注册表 132 / 运行时 79 / 注册表有而运行时无 53 / 运行时有而注册表无 0
[live-port] ui/methods: service="yeban-ui-mcp" surface="tier1-live-port" mode="test" injectAllowed=true
[live-port] 截图产物: target/ui-test-port/live-port-filled-project-unmasked-1920x1080.png
[live-port] 遮罩帧产物: target/ui-test-port/live-port-filled-project-masked-1920x1080.png
[live-port] 控件树产物: target/ui-test-port/live-port-control-tree.json
```

| 证据 | 数字 | 它排除了什么 |
| :--- | :--- | :--- |
| 控件树里的**工程数据** | `track-0-header.label == "轨道 Lead"`（`Lead` 是 `filled_project()` 的轨道名；`aria` 前缀 `轨道 ` 来自 `arrangement_view.slint`） | "控件树读的是演示常量" |
| 族规模 == 工程规模 | `track-*-header=3` / `section-*-card=2` / `clip-*-header=2`（与 `view.tracks/sections/clips` 逐项相等） | "界面里还写着 `for … in 6`" |
| 语义 ID 可寻址 | 79 个运行时节点；4 个**工程音符 ULID** 与 2 个**工程剪辑摆放 ULID** 逐个可查 | "`{ulid}` 段来自界面字面量" |
| **AI 拿到的像素 == 窗口的像素** | 未遮罩帧指纹 **`a360802d81461bee`** == 同一轮 `[model-binding]` 判据里**直接** `port.window().capture()` 的指纹；也 == `app-binding-notes.md` §5.1 记录的同一工程/同一视图的 Tier-1 指纹 | "控制面自己渲染了第二份像素"（两条独立代码路径给出同一串字节） |
| 遮罩真的改字节 | 遮罩帧 `91e06ec5d83fadd3` ≠ 未遮罩 `a360802d81461bee`；`maskEffective=true`、6 个动态区（与 `app-introspect-notes.md` §6.1 实测的"运行时动态区 6 个"**一致**） | "证据说遮了、发出去的图没遮" |
| `[MUST-GATE-015]` 两条门槛 | 1920×1080（IHDR 也是 1920×1080）、非黑 **99.1%**（未遮罩 100%）、颜色 **2642** 种、PNG **6222418** 字节（stored deflate 定长） | "尺寸为零 / 全黑帧" |
| `visible: false` 的过滤 | 注册表 132 / 运行时 79 / 缺失 **53** / **未登记 0** | "注册表有而运行时无"是**必然**的（`[ARCH-UI-005]`），且运行时里没有未登记 ID |
| 事件注入这条路 | `ui/methods` 报 `mode="test"` / `injectAllowed=true`（判据 4 在同一轮里 200 通过了 `ui/dispatch_pointer_move`） | "测试模式也注不进去" |
| 生产硬禁 | 判据 3 在同一轮里拿到 `403` + `kind == "forbidden-in-production"` | "真实执行面上能绕过 §12.3" |

**artifact**：`ui-screenshots-workspace`（CI 上传 30 天）里新增
`live-port-filled-project-unmasked-1920x1080.png`、`live-port-filled-project-masked-1920x1080.png`、
`live-port-control-tree.json` —— 人眼可复核。

### 3.4 独立复核（**仓库之外**，Pillow 11.3.0，对 CI artifact 重算）

CI 的 artifact（`ui-screenshots-workspace`）下载到仓库之外（`/tmp/lp-art/`）之后，
用 Python + Pillow **重新解码**那两张 PNG，并对着 `live-port-control-tree.json` 逐条核对。
这一步的价值是：**证据不再只来自我自己的断言代码**（判据里也可能写错），
而是来自一个独立解码器 + 树 JSON 里的真实几何。

```text
live-port-filled-project-unmasked-1920x1080.png: 6222418 字节, 魔数 OK, IHDR 1920x1080, fnv1a64=a360802d81461bee
live-port-filled-project-masked-1920x1080.png:   6222418 字节, 魔数 OK, IHDR 1920x1080, fnv1a64=91e06ec5d83fadd3
颜色数: 未遮罩 2825 / 遮罩 2642          （与 CI 日志逐字相同）
非黑像素: 未遮罩 2073600 / 遮罩 2054848  （与 CI 日志逐字相同）
6 个动态区: 未遮罩图里都不是全黑 / 遮罩图里**每一个都全黑**  ⇒ [UI-MCP-002] PASS
track-0-header 区域 (240,90,408,144): 背景 #151d38, 墨迹 913 px, 该区域 211 种颜色
对照（工程里不存在的"第 4 行", 240,258,408,312）: 只有 1 种颜色
```

**"截图哪个区域"的答案（来自树里的真实几何）**：

| 语义 ID | 帧内矩形（1920×1080） | 标签 |
| :--- | :--- | :--- |
| `track-0-header` | `x=240, y=90, w=168, h=54` | `轨道 Lead`（**判据查的就是它**） |
| `track-1-header` / `track-2-header` | `y=146` / `y=202`（同列） | `轨道 Bass` / `轨道 Aux Reverb` |
| `section-0-card` / `section-1-card` | `y=48`，`x=408` / `x=664`（`w=256, h=20`） | `章节 Intro` / `章节 Drop` |
| `clip-01J8ZQ…0050-header` / `…0051-header` | `(408,94)` / `(472,150)` | `剪辑 Lead · Clip` / `剪辑 Bass · Kick` |
| 4 个 `note-01J8ZQ…0100..0103-rect` | `(68,697)` … `(296,739)`（`66×12`） | 工程里 `MidiNote::id` 的前 4 个 |
| 6 个动态区（被置黑） | `arrangement-playhead (768,68,1,563)`、`piano-roll-playhead (236,691,1,365)`、`status-bar-chord (236,1056,96,24)`、`status-bar-device (1652,1056,256,24)`、`status-bar-selection (8,1056,220,24)`、`transport-timecode (224,8,128,32)` —— 面积合计 **18752 px = 0.9043%** | 与 `app-introspect-notes.md` §6.1 的实测**同一个数** |

> `track-0-header` 区域里 913 px 墨迹、211 种颜色 ⇒ 那里**真的画了字**（"轨道 Lead"），
> 而"工程里没有第 4 条轨道"的那一行只有 1 种颜色（纯背景）——
> 这就是"界面规模由**工程**决定"在最外层可被人眼复核的形态。

---

## 4. 两半的判据清单

### 4.1 `crates/yeban-ui-mcp/src/live.rs` 的 14 条（本机 `rustc --test` 真跑）

| # | 判据 | 覆盖 |
| ---: | :--- | :--- |
| 1 | `probe_reads_a_project_backed_node_and_a_real_png_end_to_end` | 端到端四步 + 线格式键序（字母序）+ 证据链 |
| 2 | `probe_reports_a_missing_semantic_id_as_an_explicit_error` | 树层 `ElementNotInTree` + 服务层 `-32006/400` |
| 3 | `probe_rejects_a_label_that_does_not_carry_the_project_data` | 标签不含工程文本 ⇒ `LabelMismatch`（**说出两边**） |
| 4 | `an_all_black_frame_is_rejected_with_capture_failed` | `[MUST-GATE-015]` 全黑 ⇒ `-32008/500` |
| 5 | `probe_checks_the_screenshot_size_against_the_expected_one` | `SizeMismatch`（拿 IHDR 比，不是回显） |
| 6 | `two_probes_of_the_same_surface_are_byte_identical` | 确定性（树/节点/截图三份字节） |
| 7 | `production_hard_denies_injection_before_the_surface_is_touched` | 生产硬禁 403 + **执行面零调用**；测试模式真落到执行面 |
| 7b | `call_result_carries_the_machine_readable_denial_kind` | `kind`（`forbidden-in-production` / `element-not-found`） |
| 8 | `unauthenticated_calls_are_rejected_before_parsing` | 401 + `id=null`（鉴权先于解析） |
| 9 | `coverage_reports_both_directions_through_the_live_plane` | `ui/coverage` 两个方向 |
| 10 | `pure_lookup_helpers_report_the_exact_failure` | 纯查询辅助的**精确**失败形状 |
| 11 | `png_ihdr_reader_reads_the_bytes_not_the_evidence_fields` | IHDR 读法独立于编码器（喂假字节也能读） |
| 12 | `scope_table_follows_the_port_permission_tiers` | 三级权限 → 六级 scope，逐级断言 |
| 13 | `the_assembly_point_exposes_capabilities_and_the_surface_name` | 装配点 = `ui/methods` 的形状 + 令牌可用 |

### 4.2 本机**没有**验证的（**只有 CI 能判**，逐条说清）

- `live_surface.rs` / `tests/live_ui_mcp.rs` / `registry_tree.rs`（在 app 的 lib 图里）的**编译正确性**；
- `clippy --workspace --all-targets -- -D warnings` 对**上面三个文件**的结论
  （本机 clippy 只覆盖零 Slint 部分，见 §5.3）；
- Tier-1 软件光栅化的**真实像素**：尺寸 / 非黑占比 / 颜色数 / PNG 字节 / 指纹；
- 运行时控件树里到底有哪些 ID、`track-0-header` 的 `.slint` 标签是不是 `轨道 Lead`；
- `visible: false` 的过滤（因此 `missingAtRuntime` 的量级）；
- `slint::platform::set_platform` 在 4 个测试线程上的行为（每个 `#[test]` 一次）。

---

## 5. 本机真跑 vs 交给 CI（严格区分）

### 5.1 本机**真跑**的五步（`bash run.sh`，全绿）

| 步 | 目标 | 结果 |
| ---: | :--- | :--- |
| (1) | `yeban-ui-test-port` 的零 Slint 子集（`image`/`tree`/`mask`/`png`/`port`，`#[path]` 指仓库原件）→ 真 rlib | 编译通过 |
| (2) | `yeban-ui-mcp` 的 **lib** 目标，feature **OFF** 与 **ON** 各一遍（模拟默认 release 与 `ui-mcp-http`） | `-D warnings` 零告警 |
| (3) | `yeban-ui-mcp` 的**单元判据**（`--test`，含 `live.rs`） | **66 passed; 0 failed** |
| (4) | `yeban-app` 的纯计算那一半（`bridge`/`scene`/`elements`）→ 真 rlib | `-D warnings` 零告警 |
| (5) | 工程数据 → 注册表 → 控制面 `probe` 端到端（假窗口 + 真 PNG 编码器） | **1 passed**，数字见 §3.2 |

外加（同一批依赖，零 Slint）：

```text
bash scripts/gates/run-gates.sh light            # 门禁通过 (mode=light)
cargo metadata --locked                          # exit 0
cargo tree -p yeban-app -e normal --locked | grep -c 'yeban-ui-mcp'   # 0  ← 红线 6 的证据
cargo tree -p yeban-app -e dev,normal --locked | grep 'yeban-ui-mcp'  # 测试图里才有
```

### 5.2 注入 → 变红 → 还原（**3 处，全部真做过**）

方法（`bash mutate.sh`）：备份 → **精确文本替换**（替换次数必须恰好 1，否则脚本自己报错，
避免"注入没生效"被当成"判据没变红"）→ 重新编译并真跑 → 记录红掉的**判据名** →
从备份还原 → `sha256` 逐字节确认复原 → 再跑一遍全绿。

| # | 注入 | 变红的判据（**实测**） | 还原 |
| :--- | :--- | :--- | :--- |
| **A** | `registry_tree.rs`：轨道包头标签改读**演示常量**（`scene::TRACK_NAMES[0]`） | `project_data_reaches_the_control_plane_end_to_end`（本机端到端）—— 同一断言在 CI 判据里**逐字存在**（`probe.node.label == "轨道 {工程轨道名}"`） | ✅ `sha256 ec6166c8…` |
| **B** | `surface.rs`：`[MUST-GATE-015]` 的"非全黑"判定被架空 | `surface::tests::screenshot_evidence_requires_a_non_black_non_empty_frame`、`live::tests::an_all_black_frame_is_rejected_with_capture_failed`、`service::tests::screenshot_returns_a_real_non_black_png_with_evidence`（**3 红**） | ✅ `sha256 df633f54…` |
| **C** | `live.rs`：`find_semantic_node` 找不到时**返回第一个节点**（"按语义 ID 查找"变成"随便给一个"） | `live::tests::pure_lookup_helpers_report_the_exact_failure`、`live::tests::probe_reports_a_missing_semantic_id_as_an_explicit_error`（**2 红**） | ✅ `sha256 46fd23a3…` |

**方法学留痕（注入 B 的额外收获）**：架空服务侧的"非全黑"门槛之后，
`live::tests::an_all_black_frame_is_rejected_with_capture_failed` 并没有"静默变绿"，
而是以另一种方式变红 —— `ScreenshotProbe::parse` 的**第二条防线**
（`nonBlackPixels` 必须落在 `1..=宽×高`）抓住了它。也就是说这条链是**两层**的：
服务侧拒绝 + 证据链自洽；注入一层还有另一层。

**注入写法的一条纪律**：注入 B 没有写成 `if false`（那会触发 `unreachable_code`，
在 `-D warnings` 下变成**编译失败**）—— 编译错误不是判据
（`ui-mcp-notes.md` §5 的教训）。改成 `&& image.size().width == 4242`
（运行时恒假、编译期不折叠）。

### 5.3 本机 clippy（本线新拿到的手法，**只覆盖零 Slint 部分**）

**`clippy-driver` 可以像 `rustc` 一样直接被驱动**。`cargo clippy -p yeban-ui-mcp` 在本机跑不了
是因为 cargo 要先编译依赖图里的 Slint；而本 crate 自己的代码零 Slint —— 把依赖换成探针 rlib
之后，**本 crate 的全部代码都能被真 clippy 检查**：

```bash
bash /Users/crow/work/music/.live-port-harness/clippy.sh
#   ui-mcp lib (feature OFF)      → 零告警
#   ui-mcp lib (feature ON)       → 零告警
#   ui-mcp 单元判据 (--test)      → 零告警   ← 含本线 14 条新判据
#   app 的 registry_tree.rs       → 零告警
```

**边界（明说）**：`live_surface.rs` 与 `tests/live_ui_mcp.rs` 需要 `yeban-app` 的 lib
（含 Slint），因此**不在**本机 clippy 的覆盖内 —— 那两块的 clippy 结论只有 CI 能给。
这也意味着"本机 clippy 零告警"**不能**替代 CI 的
`cargo clippy --workspace --all-targets -- -D warnings`。

### 5.4 与 CI 的分工（一句话）

> **本机证明**：控制面管线 / 线格式 / 证据链 / 授权顺序 / 错误码 / 工程 → 注册表 → 节点这一条链。
> **CI 证明**：上面全部 **+** 像素真的来自 Tier-1 软件光栅化的活窗口、控件树里有 `.slint` 的
> `accessible-label` 与真实几何、`visible: false` 的过滤真的生效。
> 两处的 `probe` 是**同一段代码**，差别只有执行面。

---

## 6. CI 判决

> 未读到的判决一律记 `pending`。读取方式：`bash scripts/dev/ci-verdict.sh line/live-port`。

| 轮 | commit | run id | 结论 |
| :--- | :--- | :--- | :--- |
| 1 | `9ddac1a` | [37232856705](https://github.com/gradetwo/yeban/actions/runs/37232856705) | **failure**：`plan` / `checks` / `lockfile` / `deny` 全绿；`rust (workspace 全量)` 的 **clippy 死了 1 条**（本线的）：`crates/yeban-app/tests/../src/live_surface.rs:215` `error: method \`scene\` is never used`。⇒ `test --workspace` **没有跑到**本线的 4 条端到端判据（这一轮没有代码读数）。`rust (${{ matrix.crate }})` 按计划跳过（0s）。 |
| 2 | `9403282` | [37233109787](https://github.com/gradetwo/yeban/actions/runs/37233109787) | **failure（唯一的红点不是本线的）**：`plan` / `checks` / `lockfile` / `deny` 绿；`rust (workspace 全量)` 的 **clippy 零告警**（本线第 1 轮那条死代码已删），`test --workspace` 跑到了本线的全部判据 —— **`tests/live_ui_mcp.rs`：`running 4 tests` → `4 passed`（10.07s）**，读数见 §3.3。红的唯一一条是 **`crates/yeban-engine/tests/rt_zero_alloc.rs:121`**：`[MUST-GATE-001] 10_000 quanta: allocations=9 deallocations=3` ⇒ `left: 9 / right: 0`。该文件由 **origin/main 的 `384ea92`** 引入（不是本线；`git diff --name-only 384ea92 HEAD` 里没有 `crates/yeban-engine/**`）。主分支自己那一轮（run 37232665652 @ `384ea92`）走的是 **per-crate 矩阵腿**、该判据**过**；本线这一轮因为是全量腿（`Cargo.lock` 命中 `ROOT_TRIGGERS`）而把它放进了 `--workspace` 的供给里，于是**同一个测试二进制给出了不同结果** —— 见 §6.2（这是给集成者的 needs，不是本线的红点）。 |
| 3 | `c16375b` | [37233532606](https://github.com/gradetwo/yeban/actions/runs/37233532606) | **success（本线的净判决）**：`plan` / `checks` / `lockfile` / `deny` / **`rust (workspace 全量)`** 全绿（`rust (${{ matrix.crate }})` 按计划跳过）。`clippy --workspace --all-targets -- -D warnings` **零告警**（含本线那两个含 Slint 的文件）；`test --workspace --all-targets` 里 `tests/live_ui_mcp.rs` = **`running 4 tests` → `ok. 4 passed`（8.95s）**，`yeban_ui_mcp` 的 lib 单元判据 = **`running 66 tests` → `ok. 66 passed`**（与本机零 Slint 探针**同一个数字**）。第 2 轮那条 engine 红点在这一轮**过了**（`rt_zero_alloc`：`ok. 1 passed; 0 failed; 1 ignored`）⇒ 它是**负载/供给敏感的抖动**，与本线无关（§6.2）。 |
| 4 | `pending` | `pending` | `pending`（只改本 notes 的提交；因为 `Cargo.lock` 仍在 diff 范围内，计划器会**再跑一次全量腿**，因此这一轮也有代码读数） |

### 6.2 第 2 轮的读数：本线全绿，唯一的红点在 **main 自己**的新判据上

本线这一轮的**全部**读数都是绿的：

```text
clippy --workspace --all-targets -- -D warnings     # 零告警（第 1 轮那条死代码已删）
Running tests/live_ui_mcp.rs (target/debug/deps/live_ui_mcp-20496409d54ffd09)
running 4 tests
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.07s
```

红色的唯一一条**不在本线的 diff 里**：

```text
Running tests/rt_zero_alloc.rs (target/debug/deps/rt_zero_alloc-c32d96ca0f652cd9)
test real_time_path_allocates_and_frees_nothing ... FAILED
[MUST-GATE-001] 10_000 quanta: allocations=9 deallocations=3
panicked at crates/yeban-engine/tests/rt_zero_alloc.rs:121:5: 实时回调窗口内发生了 9 次堆分配
```

判定"是不是我的错"的方法（与 `app-introspect-notes.md` §6 的读法相同）：

| 事实 | 出处 |
| :--- | :--- |
| 该文件由 origin/main 的 `384ea92` 引入，不是本线 | `git log --oneline -1 -- crates/yeban-engine/tests/rt_zero_alloc.rs` → `384ea92 test(engine): 补上 MUST-GATE-001 缺失的运行期断言` |
| 本线的提交**没有**碰 `crates/yeban-engine/**` | `git diff --name-only 384ea92 HEAD` → 只有本线那 10 个文件 |
| main 自己那一轮是**矩阵腿**（该判据单独跑）⇒ 过 | run 37232665652 @ `384ea92` = success |
| 本线这一轮是**全量腿**（`Cargo.lock` 命中 `ROOT_TRIGGERS`）⇒ 同一个测试二进制报 9 次分配 | 本 run 的 `test --workspace` |

⇒ 结论：**同一个测试二进制在两条腿里给出不同结果**，这是一个与"谁改了什么"无关的
供给/环境差异（或该计数型全局分配器判据在负载下不稳）。**它归 engine 线 / 集成者**，
本线不改别的 crate（`AGENTS.md` §2 与工作线纪律）。
本线在 §8 把它登记成 needs（带两个 run id），并在第 3 轮 rebase 到 `afb6204` 后重跑 ——
若它不再红，说明"本线全绿 + 该红点不可复现"；若它仍红，那也已经有充分证据说它与本线无关。

### 6.3 第 3 轮（净判决）：本线全绿，而且**读数与第 2 轮逐字相同**

```text
Running tests/live_ui_mcp.rs (...)
running 4 tests
[live-port] surface=tier1-live-port 树 79 节点 (source=runtime) 查 `track-0-header` -> role=list-item label="轨道 Lead"
[live-port] ui/screenshot: 1920x1080 maskDynamic=true regions=6 maskEffective=true
            非黑 2054848/2073600 (99.1%) 颜色 2642 种 PNG 6222418 字节 指纹 91e06ec5d83fadd3 IHDR 1920x1080
[live-port] 未遮罩帧: 指纹 a360802d81461bee 非黑 2073600 (100%) 颜色 2825 种; 与窗口直抓帧一致
[live-port] ui/coverage: 注册表 132 / 运行时 79 / 缺失 53 / 未登记 0
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.95s
Running unittests src/lib.rs (yeban_ui_mcp-…)
running 66 tests
test result: ok. 66 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.47s
Running tests/rt_zero_alloc.rs (…)
test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.11s
```

**三条读数值得单独写下来**：

1. **确定性跨 run**：第 2 轮与第 3 轮（不同 commit、不同 runner 实例）的 `live-port` 读数
   **逐字相同** —— 同一个 `a360802d81461bee` / `91e06ec5d83fadd3` / 2642 / 2825 / 2054848。
   这与 `app-introspect-notes.md` §6.4 的"across-run 指纹只有单点数据"那条 pending
   **在本线这一组上补了一个数据点**（同一字体环境 = 同一 CI 镜像）。
2. **本机 == CI 的那一半**：`yeban_ui_mcp` 的 66 条，本机零 Slint 探针与 CI 的 workspace 腿
   **同一个数字**。也就是说"零 Slint 那一半"的本机真跑不是近似 —— 它是同一批判据。
3. **第 2 轮的 engine 红点不可复现**：同一台 runner 镜像、同一条全量腿，
   `rt_zero_alloc` 在第 2 轮报 `allocations=9`、在第 3 轮 `ok`。
   ⇒ 那是**负载敏感的抖动**（计数型全局分配器判据），不是"谁改了什么"。
   两个读数都留在 §6.2 与本节，归 engine 线/集成者（§8 needs-6）。

### 6.1 第 1 轮的两条读数（都值得写下来）

1. **计划器的行为被证实**：本线改了 `Cargo.lock`，而 `scripts/dev/changed-crates.py` 的
   `ROOT_TRIGGERS` 含 `Cargo.lock` ⇒ `workspace_wide = true` ⇒ 走
   **`rust (workspace 全量)`** 单腿（`clippy --workspace --all-targets` +
   `test --workspace --all-targets`），per-crate 矩阵腿被**跳过**（0s）。
   因此本线的 4 条端到端判据会在**全量腿**里被执行 —— 与提交前的推导一致。
2. **红点是"只有 CI 能抓"的那一类，而且是本线的**：`LiveControlPlane::scene()` 这个访问器
   **没有任何调用方**（判据只用 `viewport()`），在 `-D warnings` 下是 `dead_code`。
   它与 `app-introspect-notes.md` §6 的 `TOKEN_BG_PANEL_ALT is never used` 是同一族：
   `src/live_surface.rs` 含 Slint ⇒ 本机探针**看不见**它（本机 clippy 只覆盖零 Slint 部分，
   见 §5.3），只有 CI 的 `clippy --workspace --all-targets` 能抓。
   **修法**：不是给访问器加 `#[allow]`，也不是让别的代码"顺手调用一下"，
   而是**删掉它**（有用的 API 才留）—— 判据只用到 `viewport()`，
   这已经由 `LiveControlPlane::viewport()` 覆盖。

---

## 7. 边界（本线**不做**什么）

1. **不改 `ui-test-port` / 任何其它 crate**：`LivePort` / `PortAdapter` 的公开 API 一行未动
   （本线只在 `yeban-app` 与 `yeban-ui-mcp` 里工作）。
2. **不把控制面接进发行版的运行路径**：`src/main.rs` 的事件循环一行未动。
   理由：`ui-mcp` 在默认 release 构建里**不能**被打开（红线 6），而"GUI 事件循环 + 串行控制面
   在同一个线程上"是一个独立的设计问题（谁阻塞谁），不属于本线。
   `[ARCH-UI-003]` 因此**没有被触碰**：`host::build_main_window` 本来就不跑事件循环，
   Tier-1 路径是 `set_platform` + `MainWindow::new()` + `show()`，**没有** `run_event_loop`。
3. **不发明第二个注入点**：真实窗口仍然只由 `host::build_main_window` 构造（D28）。
4. **不做 PNG 解码**（沿用 `ui-mcp` 的边界）：`ScreenshotProbe` 只读 PNG 的 **IHDR 8 个字节**
   做尺寸对账，不解码像素；跨进程的图-图比对仍需要人类先裁决基准图库。
5. **不实现管理动作**：`ui/switch_main_view` / `ui/force_save` / `ui/reload_engine`
   在真实执行面上仍然返回 `-32005 NOT_IMPLEMENTED`（`LivePort` 的三个 `*_impl` 如实拒绝）
   —— 本线**没有**把它们变成假的成功。判据 4 只证明事件注入这条路的接线。
6. **不碰 `tests/` 与 `src/test_port_adapter.rs` 的既有判据**：那条线的 9 条判据原样保留
   （本线只是新增了 `registry_tree.rs`，与它内部的私有转换函数**重复 9 行** —— 见 §9 TODO）。

---

## 8. needs（需要别人做 / 需要人类裁决）

1. **【集成者】若要发行版真的能开控制面**：给 `yeban-app` 加一个**非默认** feature
   （例如 `ui-control-plane = ["dep:yeban-ui-mcp"]`）+ `main.rs` 的一个开关，
   并在 `ci.yml` 里加一条**带该 feature** 的 `clippy/test`（`.github/**` 是集成者独占）。
   本线**没有**做这件事：它会把 `ui-mcp` 拉进 release 依赖图（红线 6 要求默认关闭），
   而且"控制面与 GUI 事件循环如何共用线程"需要一个显式的设计裁决。
2. **【人类裁决】`docs/ledger/dependency-licenses.md` 又被本线重新生成**
   （只有 `Cargo.lock` 的 SHA-256 一行变化）。多线并行时这是冲突热点。
   （第 3 轮 rebase 到 `afb6204` 之后**无冲突**：`384ea92..afb6204` 没有改 `Cargo.lock`，
   因此本线的锁与清单原样有效，已由 `run-gates.sh light` 的 `license_inventory.py --check` 复核。）
3. **【人类裁决】`ControlPlane` 的公开构造点是否要收窄**：目前
   `read_only` / `interactive_for_tests` / `with_scopes` 三个都是公开的。
   `interactive_for_tests` 的名字是刻意的（`ui:inject` 在生产模式硬禁），
   但"库里有没有一条通往测试模式的公开路"值得一次裁决。
4. **【集成者】`docs/DEVELOPMENT_LEDGER.md`**：建议把
   "UI 控制面尚未在真实界面上被端到端证明"这条 pending 改成
   "已由 `line/live-port` 在 `LivePort<MainWindow>` 上端到端证明（见
   `docs/ledger/live-port-notes.md` §3）"，并把 `ui-mcp-notes.md` §8 needs-3（app 侧两行接线）
   标为**已完成**。
5. **【集成者】`docs/ledger/ui-mcp-notes.md` §8 needs-3 可以关掉**：那两行已经落地在
   `crates/yeban-app/src/live_surface.rs`（并且是**唯一**一处）。
6. **【集成者 / engine 线】`[MUST-GATE-001]` 的计数型全局分配器判据在**全量腿**下报红**：
   `crates/yeban-engine/tests/rt_zero_alloc.rs`（origin/main `384ea92` 新增）在
   run [37233109787](https://github.com/gradetwo/yeban/actions/runs/37233109787) 的
   `test --workspace --all-targets` 里实测 `allocations=9 deallocations=3`
   （`rt_zero_alloc.rs:121` ⇒ `left: 9 / right: 0`），而同一判据在 main 自己的
   per-crate 矩阵腿（run 37232665652 @ `384ea92`）里是**过**的。触发本线这一轮走全量腿的
   原因是本线的 `Cargo.lock` 变化命中 `scripts/dev/changed-crates.py` 的 `ROOT_TRIGGERS`。
   两个 run id 都在这里，便于复核"同一个测试二进制在两条腿里结果不同"。
   **第 3 轮（run 37233532606）同一条腿里它 `ok`（`1 passed`）⇒ 确认是负载敏感的抖动**，
   但"一个全局分配器计数判据在负载下会红"本身就是一条需要 engine 线处理的债。
   本线**不改** `crates/yeban-engine/**`（别的 crate 不是本线的地盘）。
7. **【人类裁决】`--test-threads=1` 下的平台约束**：4 条 CI 判据各装一次平台，
   这是 libtest 默认（一测一线程）下的正确用法。要不要在 CI 里显式钉住默认线程数，
   或把这条约束变成一条判据（例如检测"同线程第二次 `set_platform`"并给出可读的错误），
   需要一次取舍（既有债务，本线只登记）。

---

## 9. pending（未证实的、已知的债）

1. **本线 CI 是否真的绿** —— 见 §6；未读到判决前一律记 `pending`。
2. **`registry_tree.rs` 与 `src/test_port_adapter.rs` 的私有 `registry_to_tree` 重复 9 行**
   （唯一实现 vs 判据自持的拷贝）。本线**没有**去改那条线的文件以降低风险；
   建议集成者在合并时把 `test_port_adapter.rs` 改成
   `use crate::registry_tree::control_tree_from_registry;`（需要给那个测试目标加一句
   `#[path = "registry_tree.rs"] mod registry_tree;`）。
3. **`live_surface.rs` 的 `#[path = "registry_tree.rs"]` 嵌套包含**：靠 rustc 的
   "`#[path]` 相对**当前文件所在目录**"语义（本机用一个 3 文件最小探针**实测**过这个语义，
   见 §5.1 的第 (5) 步装置）。它不是不稳定特性，但对读者是**不常见**的写法 —— 已在文件头写明。
4. **本机没有覆盖的编译面**（§4.2）：`live_surface.rs` / `tests/live_ui_mcp.rs` 的
   clippy 与编译正确性只由 CI 判。
5. **真实 GL/物理窗口后端仍未验证**：本线走的仍然是
   `set_platform` + `MinimalSoftwareWindow`（`[MUST-GATE-015]` 要求的正是它）。
   `renderer-femtovg` + winit 的真实屏幕渲染仍未在任何地方被验证（与 `app-introspect` 同一债务）。
6. **`ui/screenshot` 与 SSIM 的关系没有新增方法**（沿用 `ui-mcp` 的 needs-5）：
   本线只把"发出去的 PNG == 窗口的帧"钉住，不做跨进程图-图比对。

---

## 10. TODO(hoist)

1. **hoist → `docs/DEV_WORKFLOW.md`**：本机验证"含 Slint 的 crate"的**第三条**手法 ——
   `clippy-driver` 直接驱动（`bash .live-port-harness/clippy.sh` 的形态）。
   前两条（抽纯计算模块 + `#[path]` 指仓库原件；文本层 `.slint` 属性对账）已由
   `ui-test-port` / `app-binding` 两条线登记。本线证明**clippy 也能在零 Slint 的那一半上真跑**，
   代价是"含 Slint 的那几个文件仍然只能由 CI 判"。
2. **hoist → `docs/DEV_WORKFLOW.md`**：给"判据必须挂在 CI 会执行的目标上"补一条推论 ——
   本线的 4 条判据走的是**自动发现**的 `tests/*.rs`（与 `real_ui_tier1.rs` 同款），
   因此不需要任何人记得加 CI 步骤。**"不需要记得"比"记得加"更可靠。**
3. **hoist → ADR-0001 D18 一族**：`ui/screenshot` 的线上键序是**字母序**
   （`serde_json::Value` = `BTreeMap`），与 `UiTree::to_json()` 的字段声明序**不同**。
   两者都逐字节稳定，但任何"把线格式与 `to_json()` 直接比字节"的判据都会**假红**。
   建议把这条写进 D18（它已经收集了 Slint/序列化一族的实测边界）。
4. **hoist → `docs/DEVELOPMENT_LEDGER.md` 的必读命令表**：
   `cargo tree -p yeban-app -e normal --locked`（红线 6 的机械证据）
   与 `cargo tree -p yeban-app -e dev,normal --locked`（测试图的证据）这一对，
   建议作为"给 app 加 dev-dependency"时的标准核验命令。

---

## 11. 修改文件绝对路径清单

```text
新增:
  /Users/crow/work/music/yeban/.worktrees/live-port/crates/yeban-ui-mcp/src/live.rs
  /Users/crow/work/music/yeban/.worktrees/live-port/crates/yeban-app/src/live_surface.rs
  /Users/crow/work/music/yeban/.worktrees/live-port/crates/yeban-app/src/registry_tree.rs
  /Users/crow/work/music/yeban/.worktrees/live-port/crates/yeban-app/tests/live_ui_mcp.rs
  /Users/crow/work/music/yeban/.worktrees/live-port/docs/ledger/live-port-notes.md   （本文件）

修改:
  /Users/crow/work/music/yeban/.worktrees/live-port/crates/yeban-ui-mcp/src/lib.rs
  /Users/crow/work/music/yeban/.worktrees/live-port/crates/yeban-ui-mcp/src/surface.rs   （只改文档）
  /Users/crow/work/music/yeban/.worktrees/live-port/crates/yeban-app/Cargo.toml
  /Users/crow/work/music/yeban/.worktrees/live-port/Cargo.lock                            （+1 行 dev 边）
  /Users/crow/work/music/yeban/.worktrees/live-port/docs/ledger/dependency-licenses.md    （机器再生成）

仓库之外（过程产物 / 探针，**不入库**）:
  /Users/crow/work/music/.live-port-harness/{run.sh,clippy.sh,mutate.sh,pure_lib.rs,app_lib.rs,
      probe_check.rs,registry_tree_check.rs,backup/,*.log}
  /Users/crow/work/music/yeban/.worktrees/live-port/target/ui-test-port/  （截图与控件树 JSON）
```
