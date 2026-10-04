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
"$H/run"          # 31 passed
```

### 4.1 本机实测数字（`$H/run`，31 条判据全绿）

| 判据 | 类别 | 实测 |
| :--- | :--- | :--- |
| `two_projections_of_the_same_project_are_byte_identical` | 确定性 | demo/filled/empty 三个工程各投影两次，`canonical_bytes()` **逐字节相同**；指纹 `27fbb4701a135db0` / `c4cf9c2a3015b26b` / `0b4a22b4be528f95`（两次运行完全一致） |
| `tick_to_pixel_round_trips_in_both_directions` | 时钟 | `tpp ∈ {1,5,30,32,60,120,960}` × `px ∈ {0,1,2,29,30,31,1000,65535}` 双向往返全部等值；`tick_to_px(30,30) == 1` |
| `empty_project_projects_to_an_empty_view` | 空工程 | `YebanProjectV1::default()` → 0 轨道 / 0 剪辑 / 0 段落 / 0 场景 / 0 音符；标尺仍 16 条；与 `ViewState::empty()` 字节相同 |
| `absurd_tick_ranges_error_instead_of_overflowing` | 溢出 | `start=u64::MAX-1, dur=u64::MAX` → `Err(TickOverflow)`（不 wrap 不 panic）；`px_to_tick(u32::MAX,u64::MAX)` → `Err`；`tick_to_px(u64::MAX,1)` → `Err`；`tpp=0` → `Err`；分母 0 → `Err` |
| `demo_projection_reproduces_the_scene_constants` | 夹具 | 演示工程 `validate()==Ok`；投影**逐字**复现 `TRACK_NAMES` / `NOTE_ULIDS` / `CLIP_ULIDS` / `SECTION_NAMES` / `SCENE_NAMES` / `FADER_DB_LABELS` 与四个计数 |
| `filled_project_maps_every_model_field_the_view_consumes` | 字段映射 | `title`/`bpm`/`time_signature`/主总线分离/每轨 8 个字段/剪辑起止与车道/段落/场景/4 个音符身份/力度=100÷127 |
| `positions_are_integer_derived_and_bars_are_equidistant` | 位置 | 每个剪辑/段落的 x == `tick_to_px(start)`；小节线相邻差恒 = 128px；段落 x == 前面宽度之和 |
| `note_ulids_come_from_the_clip_pool_in_a_deterministic_order` | 顺序 | 音符身份按键序、无重复；音频片段不贡献音符 |
| `tempo_and_time_signature_projection_covers_the_boundaries` | 边界 | `999.00` / `20.00` / `7/8`（`bar_length_ticks=3360`）/ `NaN` → `bpm_millis=0` |
| `elements::registry_follows_the_projected_project` | **模型→注册表** | 演示工程 6 轨 ⇒ 有 `track-5-header`；`filled_project` 3 轨 ⇒ 无 `track-3-header`；`track-0-header.label` 含 `Lead`；**全表无一个标签含 `鼓`**；演示音符 ULID 全部不在 filled 注册表里 |
| `elements::slint_accessible_ids_and_registry_cover_each_other` | 双向契约 | 13 个 `.slint` 的 `accessible-id` 模板 ↔ 184 条注册表互相覆盖（**改完 .slint 仍然绿**） |
| `elements::model_driven_families_scope_is_exact` | **负向断言的收窄口径** | 8 个投影驱动族全部判为"模型驱动"；9 个仍是静态的部件（侧栏 / 混音台 / 设备机架 / 对话框 / 状态栏 / 走带 / 卷帘容器）全部**不**被判为模型驱动 |
| 其余 19 条 | — | scene / elements 的既有判据（断点表、ULID 形状、排序稳定性、动态区覆盖、`.slint` 清单 = ADR-0001 D2 …）全部保持绿 |

**注册表规模未变**：`ElementRegistry::from_view(&ViewState::demo())` = **184 条 / 14 个动态区**
（与 `docs/ledger/app-introspect-notes.md` §6.1 的实测一致），因此那套 Tier-1 判据的分母不变。

### 4.2 变异测试：**4 处注入 → 4 处变红 → 全部还原**

变异只作用在 harness 里的**副本**（`$H/mut/<name>/`），仓库文件全程未被改动；
每次变异后都重新跑仓库原件确认恢复为绿。

| # | 注入（任务建议的形态） | 变红的判据 |
| :-- | :--- | :--- |
| A | `tick_to_px` 改成**浮点累加**（`acc += 1.0/tpp` 逐 tick） | `tick_to_pixel_round_trips_in_both_directions`、`positions_are_integer_derived_and_bars_are_equidistant`（**2 红**） |
| B | 投影**忽略传入的工程**、改读 `demo_project()`（= "界面读 demo() 而不是工程"） | `filled_project_maps…`、`empty_project_projects…`、`note_ulids_come_from…`、`absurd_tick_ranges…`、`tempo_and_time_signature…`、`scene::from_project_propagates_projection_errors`、`scene::demo_scene_is_projected…`、`elements::registry_follows_the_projected_project`（**8 红**） |
| C | 轨道名**截断成首字符**（字段映射漂移） | `filled_project_maps…`、`demo_projection_reproduces_the_scene_constants`、`elements::registry_follows_the_projected_project`（**3 红**） |
| D | 规范字节里混进易变数据（调用序号） | `two_projections_of_the_same_project_are_byte_identical`、`empty_project_projects_to_an_empty_view`（**2 红**） |

> 变异 B/C 在 `elements::registry_follows_the_projected_project` 上变红，是 CI 侧核心判据
> （`project_projection_reaches_the_control_tree_and_the_pixels`）在**纯 Rust 侧的对应物**：
> 它断言的是"注册表/标签携带工程数据、且不含演示名"，与控件树判据同源。

### 4.3 本机**没有**验证的（交给 CI，逐条说清）

- `host.rs` / `main.rs` / `test_port_adapter.rs` 的**编译正确性**（含 Slint ⇒ 本机禁止编译）；
- `.slint` 的语法与语义（`for name[i] in model` / `.length` / `[length]` 数组属性的**编译**）；
- Tier-1 光栅化的真实像素（尺寸 / 非黑占比 / 颜色数 / PNG 字节 / 两次截图是否逐字节相同）；
- 运行时控件树里到底有哪些 ID、`track-{i}-header` 的 `label` 是不是工程名；
- `clippy --all-targets -D warnings`（含 `clippy::all` 里 rustc 抓不到的族）。

### 4.4 本机为**不能编译的那一半**补的两个静态证据

1. **`.slint` 属性/回调名静态对账**（`$H/slint_propcheck.py`）：对 13 个 `.slint` 的
   **73 个实例化点赋值**逐一核对"目标组件是否声明了该属性"（内建属性白名单外）。
   结果：**全部通过**。该脚本自身做过变异（把 `bar-positions` 写成 `bar-position`）⇒ 立刻报红。
2. **Rust setter ↔ `.slint` 属性名对账**：`host.rs` 里 22 个 `ui.set_*` 调用
   全部能在 `MainWindow` 找到同名 kebab-case 属性（无多无缺）；`ui/` 下**已无内联演示 ULID**
   （`grep 01J8Z5Q0R7K3M9X2V4B6N8P` 无命中）。

---

## 5. 模型数据到达像素的证据（字段 → 控件树 ID → 截图区域）

> 这一节里的**数字**由 CI 的 `rust (yeban-app)` 腿产出（判据把实测值 `observe()` 到
> stderr 与 `target/ui-test-port/app-introspect-observations.txt`）。判决读到后回填。

| 工程字段（`filled_project()`） | 控件树 ID | 断言 | 截图证据 |
| :--- | :--- | :--- | :--- |
| `tracks[0].name == "Lead"` | `track-0-header` | `label` 含 `Lead`，`role == list-item` | 状态 A（§6 数字）里轨道包头列第 1 行 |
| `tracks[1].name == "Bass"` | `track-1-header` | `label` 含 `Bass` | 同上第 2 行 |
| `tracks[2].name == "Aux Reverb"` | `track-2-header` | `label` 含 `Aux Reverb` | 同上第 3 行 |
| `sections["Intro"].name` | `section-0-card` | `label` 含 `Intro` | 画布顶部 20px 章节条左端 |
| `sections["Drop"].name` | `section-1-card` | `label` 含 `Drop` | 同一条的右半 |
| 摆放身份 `01J8ZQ…0050` / `…0051` | `clip-{placement_id}-header` | ID **由工程身份拼出**、可在树里寻址 | 车道 0 / 1 上的剪辑块包头 |
| `MidiNote::id`（4 个） | `note-{ulid}-rect` | 每个工程音符身份都能寻址 | 卷帘网格内的金色方块 |
| `MidiNote::velocity/127` | `velocity-{i}-bar` | 4 根柱与音符逐个对齐 | 卷帘底部力度泳道 |
| `bpm == 128.0` | （`transport-bpm-field` 的 `accessible-value`） | `get_bpm_display() == "128.00"` | 顶栏 BPM 字段 |
| `title == "Yeban Model Core Sample"` | — | `get_window_title()` 等于它 | 窗口标题 |
| **反证**：`scene::TRACK_NAMES`（`鼓`/`贝斯`…） | 由投影驱动的族（`track-*` / `section-*` / `clip-*` / `note-*` / `velocity-*` / `session-track-*` / `scene-launch-*` / `slot-*`） | **这些族的标签里一个演示名都不许出现** | 整张截图 |
| **反证**：`scene::NOTE_ULIDS` | `note-{演示ULID}-rect` | **不得出现** | 卷帘 |

> 负向断言**按族收窄**（`is_model_driven_id`）：侧栏资源库的静态条目里有
> `Sub Bass 低频` / `Night Pad 夜色铺底`，混音台通道条仍用演示名 —— 对整棵树做全局负向断言会
> **假红**。这一点在写判据时踩到过一次，已收窄并写进判据注释（§6 的未实现项同时登记了原因）。

「同一活窗口上换回演示工程 ⇒ 截图逐字节不同 + `track-5-header` 出现 / `Lead` 消失」
是同一判据里的第二条独立证据（同一进程、同一后端、同一字体 ⇒ 排除了环境差异）。

---

## 6. CI 判决

> 未读到的判决一律记 `pending`。

（待第 1 轮 run 读回后回填。）

---

## 7. 边界（本线**不做**什么）

1. **不做 UI→模型的写入**：本线只做单向投影 + 注入。任何反方向的编辑都必须经过
   `yeban-model` 的 `Op` 与 `yeban-engine` 的调度，本线不提供"读回 UI 状态"的入口。
2. **不给位置做浮点**：包括"为了好看"的居中、动画插值 —— 那属于渲染层，不属于投影层。
3. **不发明 `[MODEL-AST-006]`**：规范里该编号缺号（`AGENTS.md` §4.1），本线不使用。
4. **不改根级共享文件**：`Cargo.toml` / `.github/**` / `scripts/**` / `deny.toml` /
   `docs/DEVELOPMENT_LEDGER.md` / `docs/adr/**` / `schemas/**` 全部未动；
   `Cargo.lock` 与 `docs/ledger/dependency-licenses.md` 因依赖图变化按纪律重生成（§1）。
5. **不改其它 crate**：`yeban-model` 只读；`yeban-ui-test-port` 只作为 dev-dependency 使用。
6. **不声称"界面好看"**：本线证明的是"数据真的从模型流到了像素"，不是视觉质量。

---

## 8. 未实现项（如实登记，**不是**静默降级）

| # | 未实现 | 现状 | 归属 / 阻塞 |
| :-- | :--- | :--- | :--- |
| 1 | **MIDI 音符的 tick 位置渲染** | `note-{ulid}-rect` 的**身份与力度**已来自工程；x/y 仍是"第 i 个音符"的索引布局（`76px * i`），不是 `start_tick`/`pitch` 换算 | `[UI-NOTE-001]` 视口裁剪 + `[UI-NOTE-002]` 坐标双向映射；需要 `yeban-render` 的像素管线与当前编辑片段的作用域 |
| 2 | **卷帘的作用域** | 注入的是**片段池的全部 MIDI 音符**，不是"当前编辑片段" | 需要会话运行态里的"当前片段"（`MODEL-ISO-001`），本线不发明 |
| 3 | **混音台通道条** | `mixer_console.slint` 仍是 `for track_index in 6` + 内联 `FADER_LEVELS`；**默认视图里它是隐藏的**（`console-tab = 0`），所以不在运行时树里 | `[ARCH-UI-002]` 电平 SPSC；推子位置属会话运行态 |
| 4 | **轨道色标** | `TrackV3::color` 已进投影（`TrackView::color`），但 `.slint` 未消费：Slint 侧需要 `[color]` 数组，而十六进制字符串→`Color` 的解析要么在 Rust 侧做（多一层转换点），要么引入 Slint 的颜色解析 API（未核验） | 留给 UI 视觉线；本线不在无编译条件下冒险 |
| 5 | **自动化曲线 / 宏 / 设备链** | `TrackV3::automation_lanes` / `macros` / `devices` 全部未进视图；`device_rack.slint` 用 `scene::DEVICE_NAMES` 常量 | `[UI-NOTE-004]`；设备机架应由 `devices` 驱动 |
| 6 | **走带位置 / 当前分支** | `timecode` / `branch_name` 是**占位常量**（`SESSION_TIMECODE` / `SESSION_BRANCH_NAME`），因为它们在 `YebanProjectV1` 里**不存在**（会话运行态 / 提交图谱） | `yeban-engine::EngineSnapshot` + `yeban-model::commit::CommitGraph` |
| 7 | **拖拽 / 吸附 / 循环选区编辑** | 仍是静态几何 | `[UI-NOTE-003]` 工具矩阵 + 指针捕获状态机（规范 §6.1） |
| 8 | **`.yeban` 容器加载** | `--project-sample` 只切换**内置**工程（`demo_project()` / `filled_project()`）；没有"打开文件"路径 | `.yeban` 容器读写属 `yeban-services`；本线不引入 `serde_json` 到 app |
| 9 | **预览 `.slint` 不再自足** | 数组默认值改成 `[]` 之后，单独预览 `app.slint` 会看到空轨道/空卷帘 | 这是"只有一个事实源"的**代价**，与 `docs/ledger/ui-shell-notes.md` pending 4 的原计划一致（"改成 Rust 侧 ModelRc 单向注入, `.slint` 里只留空数组"） |
| 10 | **`slint::platform::set_platform` 每线程仅一次** | 三条 Tier-1 判据各自新建 `LivePort`；libtest 默认一测一线程（CI 实测通过）。若有人用 `--test-threads=1` 跑，第 2 条起会拿到 `PlatformUnavailable` | 既有约束（本线之前就有 2 条）；已在此登记 |

---

## 9. needs（需要别人做 / 需要人类裁决）

1. **`mixer_console.slint` 的轨道驱动**：把 `for track_index in 6` 换成投影注入后，
   `FADER_LEVELS` 必须换成 `[ARCH-UI-002]` 的 SPSC 电平；这是**另一条线**（engine + mixer UI）。
   本线只登记，不动它。
2. **`TrackV3::color` 的消费方式**：`[color]` 数组 + Rust 侧十六进制解析，还是 Slint 侧解析？
   需要一次 ADR 级的取舍（建议 Rust 侧，理由：解析失败必须可判据化）。
3. **当前编辑片段的作用域**（§8.2）：`YebanProjectV1` 里没有"选中片段"，
   需要 `SessionRuntimeState`（`MODEL-ISO-001` 的第二层）—— 属模型/引擎线。
4. **`docs/ledger/ui-shell-notes.md` 的 pending 4/5 可以关掉**：
   "数组属性未从 Rust 注入"与"演示 ULID / 轨道名在两处重复"**已由本线解决**
   （`ui/` 下已无内联演示 ULID；arrangement/session/piano-roll 的规模由工程决定）。
   建议集成者在合并时更新那两条。
5. **`docs/DEVELOPMENT_LEDGER.md`**：建议把"UI 完全无模型绑定"的条目改成
   "默认视图已由 `YebanProjectV1` 驱动；混音台与设备链待接"。

## 10. TODO(hoist)

1. **hoist → ADR-0001**：本线的"投影层必须零 Slint 依赖 + 整数位置 + `Result` 溢出语义"
   是一条可复用的架构约束（任何 UI 线都会遇到），建议提升为一条 D 编号裁决。
   要点：① 位置一律整数 tick 派生；② 投影返回 `Result` 而不是饱和；
   ③ 注入实现只能有一份（`host.rs`），`main.rs` 与判据共用。
2. **hoist → `docs/DEV_WORKFLOW.md`**：本机验证"含 Slint 的 crate"的正确姿势 ——
   把**纯计算模块**切出来，用 `rustc --edition 2024 --test -D warnings` + `#[path]` 指向仓库原件；
   外加两个**文本层**静态对账（`.slint` 属性名、Rust setter ↔ `.slint` 属性）。
   这已经是第 3 条线（`ui-test-port` / `render-master` / 本线）用同一手法，值得写成方法。
3. **hoist → `docs/ledger/app-introspect-notes.md` §6.1 的数字**：注册表仍是 **184 条 / 14 动态区**，
   但它的**构造方式**已从"硬编码常量"变成"投影构造"。数字没变，语义变了 —— 建议在那份 notes 里注明。
