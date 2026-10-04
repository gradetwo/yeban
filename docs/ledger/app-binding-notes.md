# app-binding 工作线备注 —— 界面不再渲染演示数据，而是由 `YebanProjectV1` 驱动

- **工作线**: `line/app-binding`（worktree `.worktrees/app-binding`，基线 main `80d3022`）
- **日期**: 2026-10-05
- **目标（原始口径）**: 让 `crates/yeban-app` 的界面不再渲染"演示数据"，而是真的由
  `YebanProjectV1` 经一个**纯函数投影层**驱动 —— 从"骨架好看"变成"能装工程"。
- **授权的文件**: `crates/yeban-app/**`（含 `src/*.rs`、`ui/*.slint`、`Cargo.toml`、`tests/**`）+ 本文件。
  `Cargo.lock` 与 `docs/ledger/dependency-licenses.md` 因**依赖图变化**按纪律重生成。
- **本机纪律**: 全程 `scripts/dev/cargo-local.sh`；**没有**在本机编译 Slint
  （`run-gates.sh crate yeban-app` 会按设计自动 SKIP）。纯计算那一半（`bridge` / `scene` /
  `elements`）在本机用 `rustc --edition 2024 --test -D warnings` **真跑**（§4）。

---

## 1. 交付物 ↔ 规范 ID

| 交付物 | 规范 ID | 本线做了什么 |
| :--- | :--- | :--- |
| `crates/yeban-app/src/bridge.rs`（**新建**） | `MODEL-AST-001` `MODEL-AST-002` `MODEL-AST-005` `MODEL-ISO-001` `ARCH-DET-001` `UI-GRID-001` `UI-GRID-002` `UI-TEST-001` | **纯函数** `YebanProjectV1 → ViewState` 投影层，零 Slint 依赖；960 PPQ **整数**位置换算（`tick/px` 双向 + `checked_*`）；演示工程夹具 `demo_project()`；9 条本机可跑判据 |
| `crates/yeban-app/src/host.rs`（**新建**） | `MODEL-AST-002` `UI-GRID-001` `UI-TEST-001` `ARCH-TOP-002` | **唯一**的 Slint 注入实现（`apply_view` / `build_main_window`）；`main.rs` 与全部 UI 判据共用它 |
| `crates/yeban-app/src/scene.rs` | `UI-GRID-002` `MODEL-AST-001` `MODEL-ISO-001` | 演示常量**降级为判据锚点**；`demo()` = `from_view(&ViewState::demo())`；新增 `from_project(&YebanProjectV1) -> Result`；走带时间码 / 分支名显式登记为**会话运行态**占位 |
| `crates/yeban-app/src/elements.rs` | `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` | 语义注册表改由投影构造（`ElementRegistry::from_view`）；**条目数仍是 184 / 动态区仍是 14**（与 app-introspect 实测一致）；新增判据 `registry_follows_the_projected_project` |
| `ui/app.slint` `ui/workspace/arrangement_view.slint` `ui/workspace/session_view.slint` `ui/console/console_tabs.slint` `ui/console/piano_roll.slint` | `UI-GRID-001` `UI-GRID-002` `UI-TEST-001` `UI-MCP-002` | 数组属性默认值改**空**；`for x in <字面整数>` → `for name[i] in root.<数组>`；`accessible-item-count` 用 `.length`；arrangement / session 的演示内联数据**全部删除** |
| `crates/yeban-app/src/main.rs` | `ARCH-UI-003` `UI-TEST-003` | 三条路径（GUI / `--headless` / `--dump-elements`）共用同一条投影数据流；新增 `--project-sample <default\|filled>` |
| `crates/yeban-app/src/test_port_adapter.rs` | `MODEL-AST-002` `UI-TEST-001` `MUST-GATE-015` `UI-MCP-001/002/003` | 新增**本线最核心判据** `project_projection_reaches_the_control_tree_and_the_pixels`；原 8 条判据改为由投影构造窗口/注册表 |
| `crates/yeban-app/Cargo.toml` + `Cargo.lock` | ADR-0001 **D21**（成员间 path 依赖的 `*` 豁免） | `yeban-app → yeban-model = { workspace = true }`（不写 version）；`Cargo.lock` 只多一行 `yeban-model` |
| `docs/ledger/dependency-licenses.md` | 门禁 `license_inventory.py --check` | 只改了 `Cargo.lock` 的 SHA-256 前 16 位（外部包仍是 **618** 个 ⇒ 依赖许可集合未变） |

**本线没有碰**：`ui/console/mixer_console.slint`、`ui/console/device_rack.slint`、
`ui/dialogs/**`、`ui/sidebar.slint`、`ui/status_bar.slint`、`ui/transport.slint`（见 §6 未实现项）。

---

## 2. 投影字段映射表（模型 → 视图 → `.slint` → 被判据断言的语义 ID）

> 「投影层字段」是 `bridge::ViewState` 的字段；「`.slint`」是注入落点；
> 「控件树 ID」是 `[UI-TEST-001]` 语义寻址的入口。**每一行的值都来自工程**，没有一行来自界面常量。

| `YebanProjectV1` 字段 | 投影层字段 | `.slint` 属性 | 控件树 ID / 断言 |
| :--- | :--- | :--- | :--- |
| `title` | `ViewState::title` | `MainWindow.window-title` → `Window.title` | `port.ui().get_window_title()` |
| `bpm` | `bpm` / `bpm_display`(`{:.2}`) / `bpm_millis` | `MainWindow.bpm-display` → `Transport.bpm-display` | `transport-bpm-field` 的 `accessible-value`；`get_bpm_display()` |
| `time_signature` | `time_signature_(numerator/denominator/display)` + `bar_length_ticks` | `MainWindow.bar-positions`（小节线 x） | `arrangement-ruler` 的子元素几何 |
| `id` | `project_id` | —（只进判据与 `--headless` 输出） | `canonical_lines()` 的 `project id=` 行 |
| `tracks`（**排除** `master_bus_track_id`） | `tracks: Vec<TrackView>` | `MainWindow.track-names` / `track-volumes` / `track-mutes` / `track-solos` | `track-{i}-header` / `-mute-button` / `-solo-button`（角色 / 标签 / `accessible-checked`） |
| `tracks[*].name` | `TrackView::name` | `track-names[i]` | `track-{i}-header.label == "轨道 {name}"` |
| `tracks[*].volume_db` | `volume_db` / `volume_display`(`{:.1}`) | `track-volumes[i]`（`UiMonoText`） | 与 `scene::FADER_DB_LABELS` 逐字对账 |
| `tracks[*].mute` / `solo` | `mute` / `solo` | `track-mutes[i]` / `track-solos[i]` | `track-{i}-mute-button` / `-solo-button` 的 `accessible-checked` |
| `tracks[*].kind` | `kind`（`midi`/`audio`/`aux-return`/`master`） | —（待接图标位） | `canonical_lines()` 的 `kind=` |
| `tracks[*].color` | `color: Option<String>` | —（见 §6 未实现项） | `canonical_lines()` 的 `color=` |
| `tracks[*].pan` | `pan_millis`（`i32`） | —（推子/声相未接） | `canonical_lines()` 的 `pan_millis=` |
| `tracks[*].clips[*]`（`ClipPlacement`） | `clips: Vec<ClipView>` | `clip-ulids` / `clip-labels` / `clip-positions` / `clip-widths` / `clip-lanes` | `clip-{placement_id}-header`（**ID 段 = 工程的摆放身份**） |
| `ClipPlacement::start_tick` / `duration_ticks` | `start_tick` / `end_tick`(`checked_add`) / `duration_ticks` / `x` / `width` | `clip-positions[i]` / `clip-widths[i]` | 几何断言 + `x == tick_to_px(start)` |
| `ClipPlacement::clip_id` → `ClipPoolEntry.name` | `clip_id` / `clip_name` / `label`（`"{轨道} · {片段}"`） | `clip-labels[i]` | `clip-{ulid}-header.label` |
| `ClipContent`（Midi/Audio） | `content`（`"midi"`/`"audio"`） | —（波形占位块） | `canonical_lines()` 的 `content=`（经 `clip_id` 归并） |
| `sections`（`SectionV3`） | `sections: Vec<SectionView>` | `section-names` / `section-positions` / `section-widths` | `section-{i}-card`（ID 序 = `BTreeMap` 键序） |
| `SectionV3::name` / `start_tick` / `end_tick` | `name` / `start_tick` / `end_tick` / `x` / `width` | `section-names[i]` 等 | `section-{i}-card.label == "章节 {name}"` |
| `scenes`（`SceneV3`） | `scenes: Vec<SceneView>` | `scene-names` | `scene-launch-{i}-button` / `slot-{t}-{s}-cell` / `session-track-{t}-header` |
| `SceneV3::tempo` | `tempo: Option<f64>` | —（Session 量化未接） | `canonical_lines()` 的 `tempo=` |
| `clip_pool[*].content.notes[*].id`（`BTreeMap` 键序） | `note_ulids: Vec<String>` | `MainWindow.note-ulids` → `ConsoleTabs` → `PianoRoll.note-ulids` | `note-{ulid}-rect`（**ID 段 = 工程的 `MidiNote::id`**） |
| `MidiNote::velocity`（0–127） | `note_velocities: Vec<f32>`（÷127） | `note-velocities` → `PianoRoll.velocities` | `velocity-{i}-bar` 的柱高 |
| `TimeSignature`（分子/分母） | `bar_length_ticks` = `960×分子×4÷分母` | `bar-positions` | 标尺小节线等距 |
| —（`[MODEL-ISO-001]` 会话运行态） | — | `timecode` / `branch-name` = `SESSION_TIMECODE` / `SESSION_BRANCH_NAME` | **占位**，有常量与文档标注 |

### 位置的整数口径（`[MODEL-AST-001]` / `ARCH-DET-001`）

`tick_to_px(tick) = tick / ticks_per_pixel`（**整数除法**，向下取整），
`px_to_tick(px) = px.checked_mul(ticks_per_pixel)`。默认 `ticks_per_pixel = 30`
（4/4 一小节 = 3840 tick = 128px）。**30 不是 2 的幂**：`1.0/30.0` 在二进制浮点里不精确，
所以任何"浮点算位置"的实现都会在 `tick_to_px(30, 30)` 上算出 0 而不是 1 ——
§4 的变异 A 正是靠这一点变红。投影里**没有一处**做位置累加。

---

## 3. `.slint` 的改动清单（哪些是**必须**的，哪些是顺带的一致性修复）

| 文件 | 改动 | 为什么必须 |
| :--- | :--- | :--- |
| `app.slint` | 新增 17 个 `in property`（`window-title` / `track-names` / `track-volumes` / `track-mutes` / `track-solos` / `scene-names` / `section-names` / `section-positions` / `section-widths` / `clip-ulids` / `clip-labels` / `clip-positions` / `clip-widths` / `clip-lanes` / `bar-positions` / `note-ulids` / `note-velocities`）并转发给 `SessionView` / `ArrangementView` / `ConsoleTabs`；`title: root.window-title` | 数组必须由 Rust 侧 `ModelRc` 注入，否则界面只能读内联常量；`window-title` 让标题也来自工程 |
| `workspace/arrangement_view.slint` | 三个数组默认值 → `[]`；`for x in 6/3/4` → `for name[i] in root.<数组>`；`accessible-item-count` → `.length`；x/width → 注入的 `[length]`；静音/独奏 `accessible-checked` → 工程的 `mute`/`solo`；音量文本 → 注入 | **核心**：轨道名/轨道数/剪辑块/段落标记从此由工程决定 |
| `workspace/session_view.slint` | `tracks`/`scenes` 默认值 → `[]`；循环规模用数组长度；Scene Launch 列与 Back-to-Arrangement 的 x 由 `root.tracks.length` 算出（上一版**写死** `128px * 6`，轨道数一变就错位） | 同上；顺带修掉一个真实的错位缺陷 |
| `console/console_tabs.slint` | 新增 `note-ulids` / `note-velocities` 两个**透传**属性并转发给 `PianoRoll` | 默认可见的控制台页（卷帘）此前渲染内联演示 ULID |
| `console/piano_roll.slint` | `note-ulids` / `velocities` 默认值 → `[]`；两个循环改为遍历 `root.note-ulids` | `note-{ulid}-rect` 的 `{ulid}` 段必须来自工程（`[UI-TEST-001]`） |

**未改**（如实登记）：`mixer_console.slint`（`for track_index in 6` 仍是字面量，通道条未接）、
`device_rack.slint`、`dialogs/**`、`sidebar.slint`、`status_bar.slint`、`transport.slint`
（后四者本来就是规范级常量或静态部件）。

**双向契约没有被削弱**：`elements.rs` 的
`slint_accessible_ids_and_registry_cover_each_other`（注册表 ↔ `.slint` 的 `accessible-id`
模板互相覆盖）在本机 harness 里**仍然绿**（§4）。

---

## 4. 本机**真跑过**的判据（vs 交给 CI 的）

本机不编译 Slint，但投影层那一半可以像 `ui-test-port` / `render` 两条线那样单独编译执行。
harness 在 **`/Users/crow/work/music/.app-binding-harness/`（仓库之外）**，
被验证的对象**不是复制品**：`lib.rs` 用 `#[path]` 指向仓库原件
`crates/yeban-app/src/{bridge,scene,elements}.rs`。

```bash
WT=/Users/crow/work/music/yeban/.worktrees/app-binding
H=/Users/crow/work/music/.app-binding-harness
source "$WT/scripts/dev/local-env.sh"
# 1) 先让 yeban-model 可用（无重依赖 crate，本机允许）
bash "$WT/scripts/dev/cargo-local.sh" build -p yeban-model --locked
# 2) 用 rustc 直接编译仓库原件（零 Slint 编译，秒级）
DEPS="$WT/target/debug/deps"
CARGO_MANIFEST_DIR="$WT/crates/yeban-app" rustc --edition 2024 --test -D warnings \
  --crate-name yeban_app_local -L dependency="$DEPS" \
  --extern yeban_model="$DEPS/libyeban_model-610f97ad4a2723a8.rlib" "$H/lib.rs" -o "$H/run"

> **追加（集成者代记，第 7 轮）**：本节第 8 条**已过时** —— `line/app-cli` 已落地"打开文件路径"（`crates/yeban-app/src/open.rs` 的 `open_project_document_file` 与 CLI 的 `--open`），并且**仍然**不需要 `serde_json`（裸 JSON 用"内存包一层容器再交权威读取器"的手法处理）。原结论只保留后半句。
