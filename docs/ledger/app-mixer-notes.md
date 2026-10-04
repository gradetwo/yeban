# `app-mixer` 工作线台账 —— 混音台通道条由工程驱动并消费真实电平 + 三个管理动作真接线

- **台账类型**：交付映射 / 接线图 / 判据清单 / 实测证据 / 未决项（**不是规范**）
- **工作线**：`line/app-mixer`（worktree `yeban/.worktrees/app-mixer`，基线 main `b014e8f`）
- **所有者目录**：`crates/yeban-app/**` + `crates/yeban-ui-mcp/**`（本台账是唯一新增的共享区文档）
- **本机纪律**：全程 `scripts/dev/cargo-local.sh`；**没有**在本机编译 Slint
  （`run-gates.sh crate yeban-app|yeban-ui-mcp` 会按设计 SKIP 本机档位）。
  零 Slint 的那一半在本机用 `rustc --edition 2024 --test -D warnings` **真跑**，
  并用 `clippy-driver -D clippy::all` 真跑 clippy（手法沿用 `live-port` 线，见 §5）。
- **规范来源**：
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`：§1.2 `[ARCH-UI-002]`（电平 SPSC 解耦原文）、
    §5.3 `[ARCH-SEC-003]`/`[ARCH-SEC-004]`（容器与原子落盘）、§7 `[ARCH-UI-004]`、`[ARCH-RT-002]`、`[ARCH-TOP-002]`
  - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §12.2 `[UI-TEST-001]`、
    §12.3 `[UI-MCP-001]`（三级权限，含 Administrative 三件事）、§12.5 `[UI-MCP-002]`
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M2-007`/`ROAD-M2-008`、`MUST-GATE-015`
  - `docs/adr/ADR-0001`：**D18**（无头/测试 API 以上游为准）、**D19**（引擎 feature 与 cpal 边界）、
    **D22**（debug info 构建期打开）、**D28**（投影层零 Slint + 唯一注入点）、**D29**（方法名与 scope）、
    **D30**（`.yeban` 容器、原子落盘归属 `yeban-mcp`）
- **上游接缝**：`docs/ledger/engine-meters-notes.md`（**电平生产侧**，§7 MN5 把 UI 消费指给了本线）、
  `docs/ledger/app-binding-notes.md` §2/§8（投影字段映射表 + 未实现项）、
  `docs/ledger/app-completion-notes.md`（tick / 色标 / `.yeban` 打开）、
  `docs/ledger/live-port-notes.md`（`live_surface.rs` 接法与 D28 的唯一注入点）、
  `docs/ledger/ui-mcp-notes.md`（方法注册表 / 六个 scope / 管理动作当时为何只转发）

> 本文件回答六个问题：**改了什么对应哪条规范**、**谁持有谁（含线程边界）**、
> **电平到底有没有被界面消费（哪个属性 → 哪个数值）**、**三个管理动作真的做了什么**、
> **每条判据怎么变红（含 7 条注入实测）**、**还剩什么没做、需要谁裁决什么**。

---

## 0. 一句话

改之前：`mixer_console.slint` 是 `for track_index in 6` + 内联的 `levels`/`db-labels` 演示数组
（通道条数被刻死在 6，而且"推子位置"与"电平柱高"是**同一个数**）；`ui/switch_main_view` /
`ui/force_save` / `ui/reload_engine` 在真实执行面上如实返回 `-32005`。

改之后：

1. **通道条由工程驱动**：规模 = `root.track-names.length`（演示工程 6 条、`filled_project()` 3 条），
   名字 / 音量 / 声相 / 静音 / 独奏 / 色标全部来自 `YebanProjectV1` 的投影；
2. **推子 ≠ 电平**：推子位置来自 `TrackV3::volume_db`（`bridge::volume_fraction`），
   电平柱高与 dBFS 文本来自引擎发布的 `MeterFrame`（`meters::peak_fraction` / `display_dbfs`）；
3. **电平真的被消费**：`MeterCollector::drain_latest` → `MeterBoard::ingest`（`supersedes`）→
   `host::apply_meters` → `.slint`；`track-{i}-meter` 的 `accessible-label` 携带
   `"峰值 <x> RMS <y> dBFS"`，因此"UI 消费了电平"是**控件树可读**的事实；
4. **三个管理动作真接线**：切视图（真改 `arrangement-view` 并回读）、强制保存
   （`[ARCH-SEC-004]` 的临时文件 → `sync_all` → 原子重命名，写出可被读回的 `.yeban`）、
   重载引擎（新 `EngineSnapshot` + 新 SPSC + 真推 N 个量子，并把**新**消费端交还 UI）；
5. **如实报告结果**：三个动作的执行面回执经 `UiSurface::take_admin_report` 进
   `result.report`（`generation` / `quanta` / `meterFrames` / `bytes` / `arrangementView` …），
   失败**不吞**（没有主总线的工程重建引擎 ⇒ 明确报错）。

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件 | 规范 ID | 说明 |
| :--- | :--- | :--- |
| `crates/yeban-app/src/meters.rs`（**新建**，约 480 行） | `ARCH-UI-002` `ARCH-TOP-002` `ROAD-M2-008` `MODEL-AST-002` | 电平**消费侧**：`MeterRuntime`（60Hz 抽帧腿：`drain_latest` → `MeterBoard`）→ `MeterSnapshot`（每轨 dBFS 文本 + 0–1 柱高）；dBFS/NaN 口径与显示下限的纯函数；**零 Slint**，本机 `rustc --test` 真跑 9 条判据 |
| `crates/yeban-app/src/engine_host.rs`（**新建**，约 340 行） | `ARCH-RT-002` `ROAD-M2-002` `ARCH-UI-002` `ARCH-TOP-002` ADR-0001 D19 | `ui/reload_engine` 的落地：`EngineSnapshot::from_project` → 新 `SnapshotSlot`/退役队列/事件通道/**电平队列** → `EngineRuntime` 真推 N 个量子 → 交还 `MeterCollector`；报告 `quanta`/`meterBulkPublishes`/`meterFrames`；**零 `cpal::*`** |
| `crates/yeban-app/src/save.rs`（**新建**，约 300 行） | `ARCH-SEC-003` `ARCH-SEC-004` `MODEL-AST-002` | `ui/force_save` 的落地：`write_project_container`（**不重写**容器规则）→ 同目录 `.tmp-{ulid}` → `sync_all` → 原子 `rename`（+ Unix 目录刷盘）；零 Slint，本机 5 条判据 |
| `crates/yeban-app/src/live_surface.rs`（改写，290 → 约 560 行） | `ARCH-UI-002` `ARCH-UI-004` `UI-MCP-001` `UI-MCP-002` `UI-TEST-001` `MUST-GATE-015` `ARCH-SEC-004` ADR-0001 D19/D28 | app 侧**唯一**接线点：新增 `LiveAdminSurface`（三个管理动作 + 电平消费 + 换工程）、`build_live_ui_with(project, &LiveWiringOptions)`（控制台 Tab / 保存路径 / 量子数）、`LiveUi::{tree_snapshot, capture, adopt_meter_collector, pump_meters, apply_project}` |
| `crates/yeban-app/ui/console/mixer_console.slint`（重写，221 → 约 420 行） | `ARCH-UI-002` `UI-TEST-001` `UI-MCP-002` `MODEL-AST-002` | `for track_name[i] in root.track-names`；通道条 6 个字段 + 电平（dBFS 文本 + 柱高）全部注入；`track-{i}-meter` 的 `accessible-label` 带 dBFS；主控通道条同源 |
| `crates/yeban-app/ui/app.slint` / `ui/console/console_tabs.slint` | `MODEL-AST-002` `UI-TEST-001` | 新增 22 个注入属性（`track-meter-*` / `master-*` / `track-pans` / `track-volume-fractions`）并两级转发到 `MixerConsole` |
| `crates/yeban-app/src/host.rs` | `ARCH-UI-002` `MODEL-AST-002` ADR-0001 D28 | `apply_view` 补投影字段 + **把电平重置成"与当前工程等长的静音"**；新增 `apply_meters`（60Hz 写入面）；`build_main_window_with_console_tab`（混音台可见的构造形态，**仍是唯一注入实现**） |
| `crates/yeban-app/src/bridge.rs` | `MODEL-AST-001` `ARCH-DET-001` `UI-A11Y-004` | 新增 `TrackView::pan_display` / `volume_fraction`、`ViewState::{track_pans, track_volume_fractions}`、纯函数 `pan_display` / `volume_fraction`（`FADER_MIN_DB`/`FADER_MAX_DB`）；+1 条判据 |
| `crates/yeban-app/src/elements.rs` | `UI-TEST-001` `UI-MCP-002` | 调音台新增 3×N+3 条注册表条目（静音 / 独奏 / 色标，含主控）；`MODEL_DRIVEN_FAMILIES` 8 → 9（新增 `mixer-`）；+3 条**文本层**判据（`.slint` 属性转发对账、host setter 对账、混音台驱动契约） |
| `crates/yeban-app/src/test_port_adapter.rs` | `UI-TEST-001` `MUST-GATE-015` | 色标族计数**收窄**到 arrangement（排除 `-mixer-`）—— 调音台现在也有一份色标，不收窄会假红 |
| `crates/yeban-ui-mcp/src/surface.rs` | `MUST-GATE-015` `UI-MCP-001` | 新增 `AdminReport` / `ReportValue` 与 `UiSurface::take_admin_report`（**默认 `None`** ⇒ `PortAdapter` 与既有假面一行都不用改） |
| `crates/yeban-ui-mcp/src/service.rs` | `UI-MCP-001` `ARCH-SEC-002` | 三个管理动作的结果挂上 `result.report`（唯一挂载点 `attach_admin_report`）；+2 条判据 |
| `crates/yeban-ui-mcp/src/live.rs` | `UI-MCP-001` | 新增 `ControlPlane::administrative_for_tests`（作用域集合走 `scopes_for_permission` 唯一映射表） |
| `crates/yeban-ui-mcp/src/methods.rs` | `UI-TEST-001` D29 | `ui/switch_main_view` 的 `view` 变成**白名单参数** `["arrangement", "session"]`（非法值 ⇒ `-32602`，不再漏到执行面） |
| `crates/yeban-ui-mcp/src/testing.rs` | — | 假执行面新增"预置回执"能力（`Fixture::report`），让"回执挂载"这条路径在本机有判据 |
| `crates/yeban-app/tests/live_ui_mcp.rs` | `ARCH-SEC-002` `ARCH-SEC-004` `ARCH-UI-002` `UI-MCP-001` `UI-TEST-001` `MUST-GATE-015` | **新增 8 条端到端判据**（混音台 3 + 管理动作 4 + 失败路径 1），与既有 4 条同住一个测试目标 |
| `crates/yeban-app/Cargo.toml` | ADR-0001 D19/D21 | 新增 `yeban-engine = { workspace = true }`（**默认 feature**；app 里不出现任何 `cpal::*`） |
| `Cargo.lock` + `docs/ledger/dependency-licenses.md` | 门禁 `license_inventory.py --check` | lock 多一条普通边（`yeban-app → yeban-engine`）；清单只有 `Cargo.lock` 摘要哈希一行变化（`43b1899b59117994` → `f32c4ee22d451f35`）。外部包数仍是 **618** |

**没有新增任何外部依赖**：`yeban-engine` 是同工作区 crate（`yeban-app` 只是第一次直接依赖它），
因此 `cargo deny` / 许可白名单 / 供应商清单都不受影响。

**没有改**：根 `Cargo.toml`、`.github/**`、`scripts/**`、`deny.toml`、`schemas/**`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、其它 `crates/**`、
`spikes/**`、法务文件、`crates/yeban-app/src/main.rs`（事件循环一行未动，红线 6 + D28）。

---

## 2. 接线图与线程 / 所有权边界

```text
                    YebanProjectV1 （唯一事实源）
                          │
        bridge::from_project ──> ViewState ──> elements::ElementRegistry
             │                        │                  │
             │                        │                  └─ registry_tree ──> ControlTree（静态注册表）
             │                        └─ scene::from_view ──> DemoScene（视口 1920×1080 + 会话运行态占位）
             ▼
   host::apply_view / apply_meters （**唯一**注入实现，D28）
             │
             ▼
   ┌─────────────────────── LiveAdminSurface ────────────────────────┐
   │ inner: PortAdapter<LivePort<MainWindow>, fn(..)>                │  ← 零 Slint 的 UiSurface
   │ window: MainWindow（clone_strong，Rc 语义的**同一个**窗口）      │
   │ registry: ControlTree（重抓树时重新注入动态区标记）              │
   │ project + view（换工程时一起换；投影是纯函数，两份必然逐字节相同）│
   │ save_path / save_epoch                                          │
   │ meters: MeterRuntime ── owns ──> MeterCollector + MeterBoard     │  ← **UI 线程**
   │ engine: EngineHost ─── owns ──> SnapshotSlot + EngineRuntime     │  ← 引擎侧（今天由控制面驱动）
   │ report: Option<AdminReport>                                     │
   └──────────────────────────────────────────────────────────────────┘
             ▲
             │ Box<dyn UiSurface>
   LiveControlPlane ──owns──> ControlPlane ──owns──> UiService（token + ScopeSet + RunMode）
             └──owns──> Rgb8Image（装配时直接抓的一帧，用于"发出去的图 == 窗口的图"）

   线程边界（[ARCH-TOP-002] / [ARCH-UI-002]）：

     引擎侧（今天：控制面/测试线程显式驱动；明天：cpal 回调）
        EngineRuntime::process_quantum ──每量子一次──> MeterPublisher
                                                          │  rtrb SPSC（meter_channel 建的**唯一**队列）
                                                          ▼
     UI 线程（60Hz 腿）                          MeterCollector
        MeterRuntime::poll ── drain_latest（抽干 + 每节点留最新）──> MeterBoard（supersedes）
        Host::apply_meters ── 写 Slint 属性（O(轨道数) 拷贝）
```

**边界纪律（三条，都是"不许做"）**：

1. **不另造队列**：app 侧没有 `VecDeque` / 第二个缓存 / `Mutex`。"取最新"是引擎
   `MeterCollector::drain_latest` + `MeterBoard::ingest`（`level::supersedes`）的能力
   （`docs/ledger/engine-meters-notes.md` §3）。
2. **不复制窗口状态**：为了"能跨线程"去复制一份 `MainWindow` 就会造出**两份事实源**，
   `ui/tree` 与 `ui/screenshot` 可能来自不同的帧。`Send` 边界就是这条链的边界
   （与 `live-port-notes.md` §2.1 同口径）。
3. **不改 `main.rs` 的事件循环**：本线只给"可调用的入口 / 接口"（`live_surface.rs`），
   发行路径一行未动。要把控制面装进发行版，需要非默认 feature + CI 步骤（集成者独占）。

### 2.1 引擎的"量子"到底是什么（**实测踩到的坑，值得记住**）

`EngineRuntime::process_quantum(output, channels)` 会把**任意长度**的输出缓冲按
`DEFAULT_BLOCK_FRAMES`（= **128** 帧，`[ARCH-DET-001]`）切片，每片一个量子、每片一次批量发布。
而快照里 `ProjectAudioConfig::block_size` 是**模型层的声明值**（演示工程 = `Frames256`）。
第一版按"快照声明的块长"分配输出缓冲 ⇒ 一次调用推进了 **2** 个量子
（实测 `meter_bulk_publishes = 2 × quanta`，本机探针当场抓到）。现在的写法是
`vec![0.0; DEFAULT_BLOCK_FRAMES * channels]`，一次调用 = 一个量子。
详见 §6 needs-1（`rt.rs` 的弹道折算用的是**快照声明值**，两者的差异属 engine 线裁决）。

---

## 3. 字段映射表（`YebanProjectV1` / `MeterFrame` → 投影 → `.slint` → 控件树）

| 来源 | 投影字段 | `.slint` 属性 | 控件树落点（可断言） |
| :--- | :--- | :--- | :--- |
| `tracks`（排除主总线） | `ViewState::tracks` | `MainWindow.track-names` → `MixerConsole.track-names`（`for track_name[i] in …`） | 通道条数 = `track-{i}-channel-strip` 族规模；`accessible-label == "通道条 {name}"` |
| `tracks[*].name` | `TrackView::name` | 同上 | 同上 |
| `tracks[*].volume_db` | `volume_display`（`{:.1}`） | `track-volumes[i]` | `track-{i}-fader` 的 `accessible-label == "轨道 {name} 推子"`；`accessible-value` 字符串 = `"{db} dB"` |
| `tracks[*].volume_db` | `volume_fraction`（0–1，`FADER_MIN_DB..FADER_MAX_DB`） | `track-volume-fractions[i]` | 推子帽的 `y`（几何；与电平柱高**无关**） |
| `tracks[*].pan` | `pan_millis` + `pan_display`（`C/L50/R30`） | `track-pans[i]` | `MixerConsole` 里的 `PAN …` 文本（非语义 ID，见 §7 未实现项） |
| `tracks[*].mute` / `solo` | `mute` / `solo` | `track-mutes[i]` / `track-solos[i]` | `track-{i}-mixer-mute-button` / `-solo-button` 的 `accessible-checked`（运行时树**不投影** checked，见 §7） |
| `tracks[*].color` | `color_rgb`（已在投影层解析 + 回退） | `track-colors[i]`（`[color]`） | `track-{i}-mixer-color-swatch` 的 `accessible-label == "轨道色标 {#RRGGBB}"` |
| `master_bus_track_id` 指向的那条 | `ViewState::master` | `master-name` / `master-volume-display` / `master-volume-fraction` / `master-pan` / `master-mute` / `master-solo` / `master-color` / `master-color-label` | `mixer-master-strip` / `mixer-master-fader` / `mixer-master-mute-button` / `mixer-master-solo-button` / `mixer-master-color-swatch` |
| **`MeterFrame::peak`**（引擎，线性幅度） | `ChannelMeter::peak_dbfs`（`dbfs_clamped(SILENCE_FLOOR_DBFS)`）→ `peak_label`（`{:.1}`） | `track-meter-peaks[i]` | `track-{i}-meter` 的 `accessible-label` 里 `"峰值 {label} "`；`accessible-value == "{label} dBFS"` |
| **`MeterFrame::rms_smoothed`** | `ChannelMeter::rms_dbfs` → `rms_label` | `track-meter-rmss[i]` | 同一个 `accessible-label` 里的 `"RMS {label} dBFS"` |
| **`MeterFrame::peak`** | `ChannelMeter::level`（`peak_fraction`，0–1） | `track-meter-levels[i]`（`[float]`） | 电平柱的 `y` / `height`（几何，**遮罩区** `dynamic_region = true`） |
| 母线帧（`MeterFrame.node == master`） | `MeterSnapshot::master*` | `master-meter-peak` / `-rms` / `-level` | `mixer-master-meter` 的 `accessible-label` / `accessible-value` |

**"没有电平"与"电平是静音"是同一种形态**：两者都写成有限值 `-120.0`（`METER_FLOOR_DBFS`，
re-export 自引擎的 `SILENCE_FLOOR_DBFS`）。这是刻意的 —— 界面上不存在"空字符串 / `NaN` /
上一帧残留"这三种第三形态（`ChannelMeter::silent` 是唯一的构造点）。

---

## 4. 三个管理动作：真的做了什么 + 副作用怎么观测

| 动作 | scope | 真的做了什么（代码落点） | 副作用证据（判据里的断言） |
| :--- | :--- | :--- | :--- |
| `ui/switch_main_view` | `app:admin` | `apply_main_view`：`MainWindow.arrangement-view = (view == "arrangement")` → **回读** → `refresh_tree` | ① 回执 `report.arrangementView` 是**回读值**（不是回显入参）；② 重抓树之后 `workspace-session-canvas` 与 `workspace-arrangement-canvas` 互换（`ui/tree` 与 `ui/node` 两条路都看到）；③ 非法视图名 ⇒ `-32602` 且视图不变 |
| `ui/force_save` | `app:save` | `save_now` → `save::save_project_file`：`write_project_container` → 同目录 `.tmp-{ulid}` → `write_all` → `sync_all` → `rename`（+ Unix 目录刷盘） | ① 磁盘上真的出现文件，大小 == 回执 `report.bytes`；② `open_project_file(path)` 读回的工程与写出的**逐字段相同**；③ `report.saveEpoch` 第二次调用 +1；④ 目录里不留临时文件；⑤ 未配置路径 / 生产模式 ⇒ **不写任何文件** |
| `ui/reload_engine` | `app:reload-engine` | `reload_engine_now` → `EngineHost::reload`：`EngineSnapshot::from_project` → 新 `SnapshotSlot`/退役队列/事件通道/**电平队列** → `EngineRuntime` 推 N 个量子 → 新 `MeterCollector` 交给 `MeterRuntime::adopt` | ① 回执 `report.generation` 1 → 2（每次换代 +1）、`quanta`/`meterBulkPublishes`/`meterFrames` 是引擎 `stats()` 原值（`publishes == quanta`、`frames == quanta × (非母线轨 + 1)`）；② **界面侧**：`track-0-meter` 的标签从注入的 `0.0` 变回下限 `-120.0`（旧读数作废 == 真的换了队列）；③ 没有主总线的工程 ⇒ 明确报错且代数不推进、旧引擎保留 |

**为什么"回执"是必须的**（`UiSurface::take_admin_report`）：§12.3 的三个动作在**执行面**上
做了什么，只有执行面知道；服务层合成一个 `{"accepted": true}` 既证明不了副作用，也让"失败被吞"
与"成功"长得一样。默认实现返回 `None`，因此 `PortAdapter` 与既有假执行面的载荷**逐字节不变**
（判据 `admin_reports_are_attached_and_absent_when_the_surface_has_none` 两个方向都钉住）。

**诚实边界**：`PortError` 没有"动作已接线但失败"这一档，因此管理动作的**失败**仍映射到
`-32005`（`kind = "not-implemented"`），消息里带**精确**原因。这是已知的映射收窄，见 §6 needs-2。

---

## 5. 本机真跑 vs 交给 CI（严格区分）

### 5.1 本机**真的跑过**的（仓库之外的探针，被验证的是仓库原件）

harness 在 `/Users/crow/work/music/.app-mixer-harness/`（**仓库之外**，不入库），
用 `#[path]` 指向仓库原件 —— **不是复制品**：

| 步骤 | 命令 | 结果 |
| :--- | :--- | :--- |
| app 零 Slint 半边（bridge/scene/elements/meters/engine_host/save/open） | `bash .app-mixer-harness/run.sh` | ✅ **69 passed; 0 failed**（`rustc --edition 2024 --test -D warnings -D missing_docs`） |
| app 零 Slint 半边的 clippy | `bash .app-mixer-harness/clippy.sh` | ✅ **零告警**（`clippy-driver -D clippy::all -D warnings`） |
| `yeban-ui-mcp` 零 Slint 半边（含 `transport/http`） | `bash .app-mixer-harness/run-uimcp.sh` | ✅ **68 passed; 0 failed**（含 `--cfg feature="ui-mcp-http"`） |
| `yeban-ui-mcp` 的 clippy（lib OFF / lib ON / 单元判据 三步） | `bash .app-mixer-harness/clippy-uimcp.sh` | ✅ **三步零告警** |
| 门禁 | `bash scripts/gates/run-gates.sh light` | ✅ **门禁通过 (mode=light)** |
| 注入 → 变红 → 还原 | 见 §5.3 | ✅ **7 条，全部字节级还原**（`cmp` 逐文件 identical；`grep -rn INJECT crates/` 零命中） |

> `missing_docs` 与 `clippy::all` 都按**真实 crate 的 lint 级别**在本机跑了：前者一开始就被
> 探针用来审我的三个新模块（补了探针的 `#![deny(missing_docs)]`），后者用 `clippy-driver`
> 直接驱动。**这一步立刻抓到一个只有非测试库目标才会暴露的真错误**：
> `service.rs` 的 `use crate::surface::{… ReportValue …}` 在 `--test` 下没问题，
> 但在 lib 目标里是 `unused import` ⇒ CI 的 `clippy -D warnings` 必红（见 §5.3 的说明）。

### 5.2 本机**没有**验证的（**只有 CI 能判**，逐条说清）

- `.slint` 的**语法与类型**（`for name[i] in model`、`[color]` / `[float]` 数组默认值 `[]`、
  22 个属性的两级转发、`100px * root.track-meter-levels[i]` 的 `length` 运算）；
- `host.rs` / `live_surface.rs` / `main.rs` / `test_port_adapter.rs` 的**编译正确性**（含 Slint）；
- 运行时控件树的**真实内容**（`track-{i}-meter` 的标签是不是那个 dBFS、混音台可见时
  `unknownAtRuntime` 是不是 0、切视图之后树是不是真的换了）；
- Tier-1 的真实像素（非黑占比 / 颜色数 / PNG 字节 / 指纹）；
- `clippy -p yeban-app --all-targets`（含 Slint 目标）与 `yeban-ui-mcp --features ui-mcp-http` 的组合。

**本机为"不能编译的那一半"补的三条文本层判据**（都真跑，都能变红）：

1. `elements::forwarded_mixer_properties_exist_in_the_target_components`：
   `app.slint → ConsoleTabs → MixerConsole` 的 22 个注入属性**双向**逐一对应
   （少一个 = 某一级转发断了；多一个 = 死属性）。注入 4 的同族会让它红。
2. `elements::host_setters_match_the_declared_slint_properties`：
   `host.rs` 里每一个 `ui.set_x_y(...)` 都能在 `MainWindow` 找到 `x-y`，
   且 `track-*`/`master-*` 每个声明都被某个 setter 写过。
3. `elements::mixer_console_is_driven_by_the_projection_and_the_meter_arrays`：
   混音台必须包含 `for track_name[track_index] in root.track-names` 与 8 处投影/电平下标，
   且**非注释行**里不许再出现 `for track_index in 6` / `db-labels` / `FADER_LEVELS` / `0.72`。

### 5.3 注入 → 变红 → 还原（**7 条，全部真做过**）

方法：把源文件备份到 `/tmp/app-mixer-backup/`，注入后用本机探针重跑，记录**红掉的判据名**，
再从备份还原并 `cmp` 逐文件确认 identical（§5.1 最后一行）。

| # | 注入（任务建议的形态） | 改法 | 实测红点 |
| :-- | :--- | :--- | :--- |
| 1 | **电平显示硬编码常量** | `meters::display_dbfs` 无条件返回 `"0.0"` | `meters::{the_display_floor_is_the_engine_floor, nan_frames_never_render_as_nan, snapshot_joins_frames_by_identity_not_by_index, strip_count_and_names_follow_the_projected_project, adopt_resets_the_board_so_a_new_engine_starts_silent}`（**5 红**，64 passed / 5 failed） |
| 2 | **"取最新"改成队列顺序取** | `MeterRuntime::poll` 用 `collector.tick` + 只喂前 1 条 | `meters::{poll_uses_the_drain_latest_semantics_not_the_fifo_one, a_small_scratch_still_drains_the_backlog, snapshot_joins_frames_by_identity_not_by_index}`（**3 红**） |
| 3 | **按队列顺序取第一条**（不按身份对齐） | `snapshot` 里的 `find` 恒返回 `held.first()` | `meters::snapshot_joins_frames_by_identity_not_by_index`（**1 红**） |
| 4 | **界面不再消费电平数组** | `mixer_console.slint` 的电平柱改成 `100px * 0.5` | `elements::mixer_console_is_driven_by_the_projection_and_the_meter_arrays`（**1 红**） |
| 5 | **注册表漏登记**（双向契约破裂） | 删掉 `track-{i}-mixer-mute-button` 的注册表条目 | `elements::{slint_accessible_ids_and_registry_cover_each_other, model_driven_families_scope_is_exact}`（**2 红**） |
| 6 | **管理动作不再如实报告** | `service::attach_admin_report` 把回执丢掉 | `service::admin_reports_are_attached_and_absent_when_the_surface_has_none`（**1 红**） |
| 7 | **`view` 白名单被撤掉** | `methods::VIEW.allowed = None` | `service::switch_main_view_rejects_a_view_name_outside_the_whitelist`（**1 红**） |

**注入 6/7 的附加价值（"只有非测试库目标才会暴露"的第二次战果）**：为了跑 clippy 的
lib 目标，我把探针扩成 lib(OFF)/lib(ON)/单元判据三步 —— 第一步立刻抓到
`ReportValue` 的 `unused import`（本机 `--test` 形态看不见）。这与 `ui-mcp-notes.md` §7.1
第 1 轮 CI 的红点是**同一个族**：修法是把测试专用导入挪进 `mod tests`，而不是加 `#[allow]`。

---

## 6. needs（需要人类 / 其它线裁决）

1. **【engine 线 / 人类裁决】快照声明的 `block_size` 与运行时量子长度是两个概念**：
   `EngineRuntime::process_quantum` 固定按 `DEFAULT_BLOCK_FRAMES`（128）切片，而
   `render_block` 折算电平弹道系数时用的是**快照声明的** `current.block_frames()`
   （演示工程 = 256）⇒ `quanta_per_second` 会差 2×，峰值保持/平滑 RMS 的时间常数随之偏。
   本线**不改其它 crate**，只把两侧的差异写成判据（`engine_host::the_runtime_quantum_is_the_det_block_not_the_declared_block_size`）
   并在此登记。需要 engine 线决定"以谁为准"。
2. **【人类裁决】管理动作的失败码**：`PortError` 的五个变体里没有"动作已接线但失败"这一档，
   因此 Admin 上下文统一把 `Rejected` 映射到 `-32005 NOT_IMPLEMENTED`（消息精确、`kind` 误导）。
   要不要按 D29 的家族扩张新增 `-32010 ACTION_FAILED`（或让执行面能区分"未接线"与"执行失败"），
   属错误码家族的扩张，本线不自造。
3. **【app 侧 / 集成者】`force_save` 的落点**：本线用 `LiveWiringOptions::save_path` 显式注入
   （测试用临时目录）。真实的"当前工程路径"属于会话运行态（`MODEL-ISO-001` 第二层），
   需要 `yeban-services` / 会话层给一个权威来源；在那之前 `save_path = None` ⇒ 如实报错。
4. **【集成者】`docs/ledger/dependency-licenses.md` 再次被重新生成**（只有 lock 摘要一行）。
   多线并行时这是冲突热点。
5. **【集成者】`docs/ledger/engine-meters-notes.md` §7 MN5 / §8 MN5**："UI 消费切片"已由本线
   完成（`crates/yeban-app/src/meters.rs`），可以在那份 notes 里标为**已完成**。
6. **【集成者】`docs/ledger/gate-status.md`**：`MUST-GATE-015` 的证据行可以补上
   "混音台通道条（`track-{i}-meter` 的 dBFS 标签）也进入了运行时控件树断言"；
   `ARCH-UI-002` 的 UI 半边现在有真实消费者（本文件 §4/§5）。
7. **【人类裁决】`ui/switch_main_view` 的 scope 仍是 `app:admin`**（D29 已登记的待裁决项）：
   本线**没有**改动它，只把 `view` 参数收成白名单。六级 scope 里仍没有"界面状态写"这一级。

---

## 7. 未实现项（如实登记，**不是**静默降级）

| # | 未实现 | 现状 | 归属 / 阻塞 |
| :-- | :--- | :--- | :--- |
| 1 | **引擎不发声** | 轨道渲染是占位静音 ⇒ 引擎实际发布的电平恒为下限；本线的"真实电平"是指**计量与接线**是真的（口径、SPSC、每量子一次批量发布），判据用**注入的已知帧**证明消费链路 | `yeban-sfz` / `yeban-dsp` 的声部合成切片 |
| 2 | **声相 / 静音 / 独奏的"数值"在控件树里读不到** | 运行时 `ControlNode` 只有 `id/role/label/bounds/dynamic_region/parent`，**没有** `accessible-checked` / `accessible-value`；`ui/property` 的清单里也没有它们。因此静音/独奏的证据是**标签 + 像素**，声相的证据是标签（`PAN L50`） | `yeban-ui-test-port` 的 `ControlNode` 扩展（不是本地盘）；本线**不编造** checked 字段 |
| 3 | **推子 / 声相的拖动** | 推子帽位置由 `volume_db` 驱动，但拖拽（指针捕获 + Op 日志）未接；静音/独奏按钮有 `TouchArea` 语义但点击不改工程 | `[UI-NOTE-003]` + `yeban-model` 的 `Op`（UI→模型方向本线**故意不做**） |
| 4 | **设备链 / 自动化 / 宏** | `device_rack.slint` 仍用演示常量；`TrackV3::devices` / `automation_lanes` / `macros` 未进视图 | `[UI-NOTE-004]`；设备机架应由 `devices` 驱动 |
| 5 | **指针/键盘注入之后不自动重抓树** | 注入走的是 `LivePort` 的窗口事件；若一次点击会改变可见分支（例如点 `tab-mixer-button`），运行时树要等下一次 `pump_meters` / 管理动作 / 换工程才更新。**没有**自动刷新是刻意的：`ui/tree` 的字节稳定性判据依赖"没有别的写入者" | 需要一次裁决：注入后是否强制 `refresh_tree`（代价：每次注入一次全树内省） |
| 6 | **多声道独立电平 / 真峰值 / LUFS** | 母线是立体声联动、轨道是声相前单声道；过期沿 `engine-meters-notes.md` §7 的 2/3 | engine 线 |
| 7 | **`.yeban` 的 `history.dag` 内容** | `force_save` 写出**空字节**的 `history.dag`（容器布局要求该条目存在）；提交图谱的权威内容属 `yeban-model::commit` | `yeban-model` / 提交图谱切片 |
| 8 | **没有开声卡** | `EngineHost` 不调用 `yeban_engine::device`；`process_quantum` 由控制面/测试线程显式驱动（它是 cpal 回调会调的**同一个函数**，但"由回调驱动"未接线） | 红线 6 + D19 的设备裁决；需要非默认 feature 与集成者的 CI 步骤 |
| 9 | **控制面不进发行路径** | `main.rs` 事件循环一行未动（`live-port-notes.md` §7.2 的边界沿用） | 集成者（`.github/**` 独占） |

---

## 8. pending（未证实的、已知的债）

1. **本线的 CI 判决** —— 见 §9；未读回来之前一律记 `pending`。
2. **含 Slint 的编译从未在本机发生过** ⇒ `.slint` 的语法/类型、`host.rs` / `live_surface.rs`
   的编译正确性**只由 CI 判**（§5.2 列了逐条）。
3. **`.slint` 的 `accessible-value` 类型**：本线沿用既有写法（一律**字符串**），
   因为仓库里 21 处既有用法都是字符串且 CI 绿。数值本身通过 `accessible-label` 暴露
   （`"… 峰值 -6.0 RMS -6.0 dBFS"`），因此不依赖 `accessible-value` 的类型。
4. **`--test-threads=1` 下的平台约束**：新增的 8 条判据与既有 4 条各装一次 Tier-1 平台
   （libtest 默认一测一线程）。若有人用 `--test-threads=1`，第 2 条起会拿到 `PlatformUnavailable`
   （既有债务，`app-binding-notes.md` §8.10 已登记）。
5. **本机没有覆盖的编译面**：`live_surface.rs` 与 `tests/live_ui_mcp.rs` 的 clippy 只能由 CI 判
   （它们含 Slint）。本机的 clippy 只覆盖零 Slint 半边。

---

## 9. CI 判决（读到什么写什么，**只有 CI 的判决算数**）

| 轮 | commit | run id | 结论 | 关键读数 |
| :-- | :--- | :--- | :--- | :--- |
| — | （待填） | — | **pending** | 推送后由 `bash scripts/dev/ci-verdict.sh line/app-mixer` 读回 |

---

## 10. TODO(hoist)

1. **hoist → ADR-0001 D28 一族**：执行面**回执**（`AdminReport`）是"唯一注入点"的补集 ——
   界面数据"只进不出"，但控制面的管理动作必须能**如实报告副作用**。这条形状（默认 `None`
   的可选 trait 方法 + 服务层唯一挂载点）值得提升为一条裁决：*"控制面方法的副作用必须由执行面
   交出结构化回执，而不是由服务层合成 `accepted`"*。
2. **hoist → `docs/DEV_WORKFLOW.md`**：本机验证"含 Slint 的 crate"的**第四条**手法 ——
   把**探针的 lint 级别对齐真实 crate**（`#![deny(missing_docs)]` + `clippy-driver -D clippy::all`
   + lib 目标的 feature OFF/ON 两侧）。前三条已由 `ui-test-port` / `app-binding` / `live-port`
   登记；本条的价值是实测到的（§5.1 的红字）。
3. **hoist → `docs/DEV_WORKFLOW.md`**：`EngineRuntime::process_quantum` 的"一次调用 = `output.len()/channels/128` 个量子"
   这条 API 语义值得写进"给引擎加消费者"的注意事项（本线第一版就踩了，见 §2.1）。
4. **hoist → `docs/ledger/engine-meters-notes.md`**：`SILENCE_FLOOR_DBFS` 现在有了第二个
   真消费者（`crates/yeban-app/src/meters.rs` 的 `METER_FLOOR_DBFS` re-export）。
   "显示下限与钳位下限必须是同一个常量"这条纪律值得在那里记一笔。

---

## 11. 修改文件绝对路径清单

```text
新增:
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/src/meters.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/src/engine_host.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/src/save.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/docs/ledger/app-mixer-notes.md   （本文件）

修改:
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/Cargo.toml
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/src/lib.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/src/bridge.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/src/elements.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/src/host.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/src/live_surface.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/src/test_port_adapter.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/tests/live_ui_mcp.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/ui/app.slint
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/ui/console/console_tabs.slint
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-app/ui/console/mixer_console.slint
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-ui-mcp/src/lib.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-ui-mcp/src/live.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-ui-mcp/src/methods.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-ui-mcp/src/service.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-ui-mcp/src/surface.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/crates/yeban-ui-mcp/src/testing.rs
  /Users/crow/work/music/yeban/.worktrees/app-mixer/Cargo.lock                            （+1 条普通依赖边）
  /Users/crow/work/music/yeban/.worktrees/app-mixer/docs/ledger/dependency-licenses.md    （机器再生成，1 行）

仓库之外（过程产物 / 探针，**不入库**）:
  /Users/crow/work/music/.app-mixer-harness/{lib.rs,uimcp_stub.rs,run.sh,run-uimcp.sh,clippy.sh,clippy-uimcp.sh,run,uimcp_tests,backup/}
  /tmp/app-mixer-backup/            （注入实验的字节级备份）
  /Users/crow/work/music/yeban/.worktrees/app-mixer/target/    （截图 / 控件树 JSON / 编译产物）
```
