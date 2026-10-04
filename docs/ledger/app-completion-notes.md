# app-completion 工作线备注 —— 卷帘 tick 位置 / 轨道色标 / 真实 `.yeban` 打开

- **工作线**: `line/app-completion`（worktree `.worktrees/app-completion`，基线 main `0901dcd`）
- **日期**: 2026-10-06
- **任务来源**: `docs/ledger/app-binding-notes.md` §8 的未实现项 **#1（音符 tick 位置）**、
  **#4（轨道色标）**、**#8（`.yeban` 容器加载）** —— 三件"让界面完全由工程驱动"的收尾。
- **授权的文件**: `crates/yeban-app/**`（含 `src/*.rs`、`ui/**`、`tests/**`）+ 本文件。
  根级共享文件、其它 `crates/**`、`schemas/**`、`docs/adr/**`、法务文件**全部未动**。
- **红线遵守**: 未改 `main.rs` 的事件循环（③ 只提供**可被调用的入口**，接线留给集成者）；
  `host.rs` 仍是**唯一**注入点（ADR-0001 **D28**）；位置换算仍**全部整数**
  （`[MODEL-AST-001]` / `ARCH-DET-001`）；投影层仍**零 Slint 依赖**。

---

## 1. 交付物 ↔ 规范 ID

| 交付物 | 规范 ID | 本线做了什么 |
| :--- | :--- | :--- |
| `crates/yeban-app/src/bridge.rs` | `MODEL-AST-001` `MODEL-AST-002` `MODEL-AST-005` `ARCH-DET-001` `UI-NOTE-001` `UI-NOTE-002` `UI-A11Y-004` `UI-TEST-001` | ① 新增 `NoteView` + `pitch_lane` / `pitch_lane_y`：音符 x / y / 宽 / 车道由 `start_tick` / `pitch` **整数**派生；② 新增 `RgbColor` / `parse_hex_color` / `track_color_or_default`（**唯一**一处颜色语法）+ `TrackView::color_rgb` / `color_hex`；`canonical_lines()` 带上音符几何与规范化色标。**零 Slint 依赖** |
| `crates/yeban-app/src/open.rs`（**新建**） | `ARCH-SEC-003` `ARCH-SEC-004` `MUST-GATE-006` `MUST-GATE-007` `MODEL-AST-002` `MODEL-ISO-001` | ③ 纯函数 `open_project_archive` / `open_project_bytes` + 入口 `open_project_file` / `open_project_archive_file`；`OpenError` **原样**携带 `ContainerError`；4 GiB 文件上限（ZIP32 边界）+ 可注入上限 `ProjectOpenOptions` |
| `crates/yeban-app/src/host.rs` | `MODEL-AST-002` `UI-NOTE-002` `UI-A11Y-004` `ARCH-TOP-002` | 唯一注入点新增 5 个数组：`note-positions` / `note-ys` / `note-widths` / `track-colors` / `track-color-labels`；`RgbColor → slint::Color` 的**唯一**转换 |
| `crates/yeban-app/src/elements.rs` | `UI-TEST-001` `UI-MCP-001` | 新增语义族 `track-{i}-color-swatch`（角色 `image`，标签携带规范化 `#RRGGBB`）—— 色标因此**在控件树里可读** |
| `crates/yeban-app/ui/console/piano_roll.slint` | `UI-NOTE-002` `UI-TEST-001` | 音符 `x/y/width` 改为消费注入数组（删掉 `76px * note_index` / `14px * note_index + 6px` 的**索引布局**）；力度泳道 x 与音符 x 对齐（同一套换算，没有第二套） |
| `crates/yeban-app/ui/workspace/arrangement_view.slint` | `UI-A11Y-004` `UI-TEST-001` | 新增 `track-{i}-color-swatch`（消费 `track-colors[i]` 与 `track-color-labels[i]`） |
| `crates/yeban-app/ui/app.slint` `ui/console/console_tabs.slint` | `UI-GRID-001` | 新增 5 个数组属性并单向转发（默认一律空数组，界面里没有第二份数据） |
| `crates/yeban-app/src/test_port_adapter.rs` | `UI-TEST-001` `UI-MCP-002` `MUST-GATE-015` | Tier-1 判据追加：色标标签 == 投影色标（含缺色回退）、音符几何"x 随 tick 严格增 / y 随音高严格减" |
| `crates/yeban-app/tests/open_project_file.rs`（**新建**） | `ARCH-SEC-003` `MUST-GATE-006` `MUST-GATE-007` | **只用公开 API** 的集成判据：磁盘上的真容器可被打开；截断 / 超限 / 炸弹被拒绝 |
| 本文件 | — | 字段映射、判据清单、注入记录、未实现项、needs |

**本线没有碰**：`src/main.rs`（事件循环 / CLI 接线）、`ui/console/mixer_console.slint`、
`ui/console/device_rack.slint`、`ui/dialogs/**`、`ui/sidebar.slint`、`ui/status_bar.slint`、
`ui/transport.slint`、`Cargo.toml` / `Cargo.lock`（**没有新增任何依赖**）。

---

## 2. ③ `.yeban` 打开：错误映射表（**如实转达**，绝不吞成空工程）

容器 API 来自 `yeban_model::container`（main 已有）。本线**不重写**任何一条防御，只做三件事：
文件尺寸上限、错误如实映射、上限可注入。

| 输入（真容器上做的字节修改 / 参数） | 容器裁决（`ContainerError`） | app 侧结果（`OpenError`） | 判据 |
| :--- | :--- | :--- | :--- |
| 合法容器 | — | `Ok(ProjectArchive)`，`project` / `history_dag` / `assets` 全保真 | `open::tests::real_container_round_trips_through_the_open_entry`、`tests/open_project_file.rs` |
| 截断 1 / 2 / 10 / 22 / ⅓ / ½ / 末 1 字节 | `EocdNotFound` / `TruncatedArchive` 一族 | `Err(Container(_))` | `truncated_containers_error_instead_of_opening_an_empty_project` |
| 空输入 / `b"not a zip at all"` | `EocdNotFound` | `Err(Container(_))` | 同上 |
| 把 `project.json` 的 method 改成 8（deflate） | `UnsupportedCompression { index: 0, method: 8 }` | 同上，**精确变体** | `tampered_containers_report_the_container_verdict_verbatim` |
| 把条目名改成 `../proj.json`（Zip-Slip） | `ParentDirSegment { name }` | 同上 | 同上 |
| 翻转 `project.json` 数据区 1 字节 | `CrcMismatch { index, name, .. }` | 同上 | 同上 |
| `max_entry_bytes = 4`（炸弹阈值注入） | `EntryTooLarge { declared, max }` | 同上 | 同上 |
| `max_entries = 1` | `TooManyEntries { found: 3, max: 1 }` | 同上 | 同上 |
| 声明解压体积改成 1 MB（压缩后几百字节） | `ExpansionRatioExceeded` | 同上 | 同上 |
| `uncompressed` 改成 `0xFFFFFFFF` | `UnsupportedZip64` | 同上 | 同上 |
| `assets/00`（非规范 CAS 名） | `InvalidAssetName { name }` | 同上 | `structurally_wrong_containers_report_precise_errors` |
| 多出 `notes.txt` | `UnexpectedContainerEntry { name }` | 同上 | 同上 |
| 缺 `project.json` | `MissingProjectJson` | 同上 | 同上 |
| `project.json` 是坏 JSON | `InvalidProjectJson { detail }` | 同上（**带底层描述**） | 同上 |
| 文件字节数 > `max_file_bytes` | —（在读之前拦下） | `FileTooLarge { path, len, max }` | `file_entry_reports_success_limits_and_missing_files` |
| 路径不存在 | — | `Io { path, source }` | 同上 |
| `Display` | — | 必须含容器**原文**（`容器被拒绝: … per-entry limit`） | `open_errors_are_self_describing` |

**上限的取值不是拍脑袋**：`.yeban` 是 ZIP32（ZIP64 与多卷被容器明确拒绝），
所以偏移 / 尺寸都是 `u32` ⇒ 超过 4 GiB 的文件不可能是合法 `.yeban`，
`MAX_PROJECT_FILE_BYTES = 1 << 32`。`metadata` 判定之外再用**实际读到的长度**二次判定
（不让"读进来再说"发生）。

**两个返回形态是显式的，不是静默丢弃**：`open_project_file` 只给工程文档；
需要 `history.dag` / `assets` 的调用方用 `open_project_archive_file` / `open_project_archive`
（判据证明它们全保真）。`[MODEL-ISO-001]` 的三层状态因此没有被混在一起。

---

## 3. 字段映射（① 卷帘 tick 位置 / ② 轨道色标）

### 3.1 ① `MidiNote` → `NoteView` → `.slint` → 控件树

| 模型字段 | 投影字段（`bridge::NoteView`） | 换算（**整数**） | `.slint` 属性 | 控件树 ID / 断言 |
| :--- | :--- | :--- | :--- | :--- |
| `MidiNote::id` | `id`（= `note_ulids[i]`） | — | `note-ulids[i]` | `note-{ulid}-rect` |
| `ClipPoolEntry::id` | `clip_id` | — | —（可回溯用） | `canonical_lines()` 的 `clip=` |
| `MidiNote::start_tick` | `start_tick` / `x` | `x = tick_to_px(start_tick, tpp)`（`tick / tpp` 向下取整） | `note-positions[i]` | Tier-1：x 随 tick **严格递增** |
| `MidiNote::duration_ticks` | `end_tick`（`checked_add`）/ `duration_ticks` / `width` | `width = max(1, tick_to_px(end) − tick_to_px(start))` | `note-widths[i]` | `canonical_lines()` 的 `end=` / `width=` |
| `MidiNote::pitch` | `pitch` / `row` / `y` | `row = PITCH_LANE_COUNT−1 − min(pitch−PITCH_LANE_BASE, 15)`；`y = 14px × row + 6px` | `note-ys[i]` | Tier-1：y 随音高**严格递减** |
| `MidiNote::velocity` | `velocity` / `velocity_normalized` | `÷127`（夹到 0–127） | `note-velocities[i]` | `velocity-{i}-bar` 柱高 |

**口径（与 clips 完全同源，没有第二套换算）**：
`tick_to_px` / `px_to_tick` 就是 `app-binding` 建立的那两个函数；音符只是多了一个调用点。
音高是**钳制**而不是绕回：`pitch < 60` 落最下面一条车道、`pitch ≥ 76` 落最上面一条
（真正的视口滚动 / 裁剪是 `[UI-NOTE-001]`，仍是未实现项）。

### 3.2 ② `TrackV3::color` → 投影 → `.slint` → 控件树

| 模型字段 | 投影字段 | 规则 | `.slint` | 控件树 |
| :--- | :--- | :--- | :--- | :--- |
| `TrackV3::color = Some("#f7e6b0")` | `color_rgb = (247,230,176)`、`color_hex = "#F7E6B0"` | 解析 + **规范化大写** | `track-colors[i]`（色块 `background`）| `track-{i}-color-swatch.label` 含 `#F7E6B0` |
| `Some("#AbC")` | `(170,187,204)` / `"#AABBCC"` | CSS 3 位短写展开 | 同上 | 同上 |
| `Some("不是颜色")` / `Some("#12345")` / `Some("")` | `DEFAULT_TRACK_COLOR` / `"#2C3A63"` | **显式回退** | 同上 | 同上（标签是回退色） |
| `None` | `DEFAULT_TRACK_COLOR` / `"#2C3A63"` | **显式回退** | 同上 | 同上 |

**颜色语法是规范缺口**：`TrackV3::color` 的模型文档只写"界面色标"，没有规定格式。
本线的裁决（**登记为 needs**，建议提升为 ADR）：接受可选的 `#` + 3 或 6 个 ASCII 十六进制字符
（大小写不敏感，3 位按 CSS 规则展开）；其余一律**拒绝并回退**，绝不猜颜色。
回退色 = `ui/tokens.slint` 的 `Tokens.line-strong`（`#2c3a63`）—— 一个中性石板色，
"缺色"与"品牌色"在视觉上可区分。两处各写一份十六进制值，由
`bridge::tests::slint_text_contracts_for_colors_and_pitch_lanes` **直接读 `tokens.slint`** 对账。

**色标在控件树里可读**的落点：`track-{i}-color-swatch`（`accessible-role: image`），
其 `accessible-label` = `"轨道色标 " + track-color-labels[i]`。于是同一条断言在**两侧**都成立：
纯 Rust 侧（`elements::track_color_swatches_carry_the_projected_hex`）与 Tier-1 运行时树侧
（`project_projection_reaches_the_control_tree_and_the_pixels`）。

---

## 4. 本机**真跑**了什么（严格区分本机 vs CI）

本机 harness 在仓库**之外** `/Users/crow/work/music/.app-completion-harness/`，
被验证的对象**不是复制品**：`lib.rs` 用 `#[path]` 指向仓库原件
`crates/yeban-app/src/{bridge,scene,elements,open}.rs`（`open.rs` 也在内，因为它零 Slint）。

```bash
WT=/Users/crow/work/music/yeban/.worktrees/app-completion
H=/Users/crow/work/music/.app-completion-harness
source "$WT/scripts/dev/local-env.sh"
bash "$WT/scripts/dev/cargo-local.sh" build -p yeban-model --locked      # 1) 先让依赖可用
DEPS="$WT/target/debug/deps"
CARGO_MANIFEST_DIR="$WT/crates/yeban-app" rustc --edition 2024 --test -D warnings \
  --crate-name yeban_app_local -L dependency="$DEPS" \
  --extern yeban_model="$DEPS/libyeban_model-610f97ad4a2723a8.rlib" "$H/lib.rs" -o "$H/run"
"$H/run"                              # 45 passed
# 集成判据（只用公开 API）：先编一个名为 yeban_app 的 rlib，再编 tests/open_project_file.rs
bash "$H/run_integration.sh" && "$H/run_integration"          # 2 passed
```

### 4.1 本机**真跑**（47 条判据全绿；全都是零 Slint 的纯逻辑）

| # | 判据 | 类别 | 本机实测 |
| :-- | :--- | :--- | :--- |
| 1 | `two_projections_of_the_same_project_are_byte_identical` | 确定性 | 演示 / filled / 空三工程两两逐字节相同（含新增的音符几何行） |
| 2 | `tick_to_pixel_round_trips_in_both_directions` | 时钟 | 既有，保持绿 |
| 3 | `empty_project_projects_to_an_empty_view` | 空工程 | 追加 `notes` / `note_positions` / `note_rows` 全空 |
| 4 | `absurd_tick_ranges_error_instead_of_overflowing` | 溢出 | 既有，保持绿 |
| 5 | `demo_projection_reproduces_the_scene_constants` | 夹具 | 既有，保持绿 |
| 6 | `filled_project_maps_every_model_field_the_view_consumes` | 字段映射 | 既有，保持绿 |
| 7 | `positions_are_integer_derived_and_bars_are_equidistant` | 位置 | 既有，保持绿 |
| 8 | `note_ulids_come_from_the_clip_pool_in_a_deterministic_order` | 顺序 | 既有，保持绿 |
| 9 | `tempo_and_time_signature_projection_covers_the_boundaries` | 边界 | 既有，保持绿 |
| **10** | `note_positions_are_integer_derived_from_ticks` | **① 口径** | `tpp ∈ {1,3,7,30,32,120,960}`（含非 2 的幂）逐音符 `x == tick/tpp`、`width == max(1, Δpx)`；`tick_to_px(30,30)==1`、`tick_to_px(7,7)==1`、`tick_to_px(6,7)==0` |
| **11** | `note_rows_follow_pitch_monotonically_with_clamping` | **① 音高** | 全 `u8` 音高 0–255 车道都在 `0..16`；窗口内逐档减 1；`pitch=0/59 → 15`、`pitch=75+ → 0`；与投影逐音符一致 |
| **12** | `note_positions_are_monotone_and_round_trip_across_zoom_levels` | **① 缩放** | 7 档缩放下：`tpp` 增 ⇒ x 单调不增；`px_to_tick(x) ≤ start < px_to_tick(x+1)` 恒成立 |
| **13** | `absurd_note_ticks_error_and_out_of_range_pitches_do_not_panic` | **① 边界** | `start=u64::MAX-1, dur=u64::MAX` → `Err(TickOverflow)`；音高 `0`/`255`、时值 `0` 不 panic（车道钳制 + 宽度 1px 地板） |
| **14** | `note_parallel_arrays_agree_with_the_rich_projection` | **① 一致性** | 身份 / 力度 / x / y / 宽 / 车道 6 个平行数组与 `notes` 逐项相等 |
| **15** | `track_colors_parse_and_missing_or_illegal_colors_fall_back` | **② 解析/回退** | 合法 6/3 位与大小写、裸 `22aa88` 通过；13 种非法形态（含空白、`#RRGGBBAA`、`rgb(...)`、中文）全部回退；`DEFAULT_TRACK_COLOR.to_hex() == DEFAULT_TRACK_COLOR_HEX` |
| **16** | `slint_text_contracts_for_colors_and_pitch_lanes` | **②/① 文本层耦合** | 回退色 == `tokens.slint` 的 `line-strong`；`piano_roll.slint` 含 16 条车道 / `14px * lane_index` / 音符直接取注入数组；**非注释代码里不得再有 `76px * note_index`** |
| 17 | `elements::registry_follows_the_projected_project` | 模型→注册表 | 既有，保持绿（追加的族不破坏"两个工程 ID 集合必须不同"） |
| **18** | `elements::track_color_swatches_carry_the_projected_hex` | **② 控件树侧** | 色标元素数 == 轨道数；`track-0-color-swatch` 标签含 `#FF8800`（filled）；演示 3 条缺色轨道标签都是 `#2C3A63` |
| 19 | `elements::slint_accessible_ids_and_registry_cover_each_other` | 双向契约 | 13 个 `.slint` 的 `accessible-id` 模板 ↔ 注册表互相覆盖（新增 swatch 族后仍绿） |
| 20 | `elements::model_driven_families_scope_is_exact` | 收窄口径 | 既有，保持绿 |
| 21 | `elements::per_track_semantic_families_are_complete` | 族完整性 | 追加 `color-swatch` 后 6 族 × 轨道数齐全 |
| 22 | `open::tests::real_container_round_trips_through_the_open_entry` | **③ 成功路径** | `write_project_container` 造真容器 → `open_project_archive` → 工程逐字段相等 + `history_dag == b"dag-bytes"` + 资产字节保真 |
| 23 | `open::tests::truncated_containers_error_instead_of_opening_an_empty_project` | **③ 截断** | 7 种截断长度 + 空输入 + 非 ZIP 全部 `Err`，且**不等于**空工程 |
| 24 | `open::tests::tampered_containers_report_the_container_verdict_verbatim` | **③ 篡改** | deflate / Zip-Slip / CRC / 炸弹 / 条目数 / 膨胀比率 / ZIP64 七种精确错误码（见 §2 表） |
| 25 | `open::tests::structurally_wrong_containers_report_precise_errors` | **③ 结构** | `InvalidAssetName` / `UnexpectedContainerEntry` / `MissingProjectJson` / `InvalidProjectJson{detail}` |
| 26 | `open::tests::file_entry_reports_success_limits_and_missing_files` | **③ 入口** | 真磁盘文件成功；注入 8 字节上限 → `FileTooLarge`；缺文件 → `Io`；容器上限仍生效 |
| 27 | `open::tests::open_errors_are_self_describing` | **③ 错误文本** | `Display` 带容器原文；`Io` 暴露 `source()` |
| 28 | `tests/open_project_file.rs`（2 条） | **③ 公开 API** | 只用 `yeban_app::open::*`：真容器从磁盘打开（工程/历史/资产全保真）；截断 / 超限 / 炸弹被拒绝 |
| 29 | 其余 `scene` / `input` / `elements` 既有判据 | — | 全部保持绿（本机 harness 共 **45 + 2**） |

### 4.2 变异测试：**3 处注入 → 3 处变红 → 全部还原**

变异只作用在 harness 里的**副本**（`$H/mut/<name>/`），仓库文件全程未被改动；
每个变异跑完都重跑仓库原件确认恢复为绿（§4.3）。

| # | 注入 | 变红的判据 | 结果 |
| :-- | :--- | :--- | :--- |
| A | `tick_to_px` 改成**浮点累加**（`acc += 1.0 / tpp` 逐 tick） | `tick_to_pixel_round_trips_in_both_directions`、`positions_are_integer_derived_and_bars_are_equidistant`、`absurd_tick_ranges_error_instead_of_overflowing`、**`note_positions_are_integer_derived_from_ticks`**、**`note_positions_are_monotone_and_round_trip_across_zoom_levels`**（**5 红**） | 红 ✓ |
| B | `track_view` 忽略工程 `color`、**硬编码** `DEFAULT_TRACK_COLOR` | `track_colors_parse_and_missing_or_illegal_colors_fall_back`、`elements::track_color_swatches_carry_the_projected_hex`（**2 红**） | 红 ✓ |
| C | `open_project_archive` 把容器错误**吞掉**、返回空工程 | `truncated_containers_error_instead_of_opening_an_empty_project`、`tampered_containers_report_the_container_verdict_verbatim`、`structurally_wrong_containers_report_precise_errors`、`file_entry_reports_success_limits_and_missing_files`、`open_errors_are_self_describing`（**5 红**） | 红 ✓ |

> 注入 A 的第一版写成 `tick * (1.0/tpp)` 再 `floor` —— 结果**假绿**：IEEE-754 下
> `30.0 × (1.0/30.0)` 恰好舍入回 `1.0`。这本身是一条教训：**"浮点"不是判据的靶子，
> "累加"才是**（位置漂移来自累加，不来自一次乘除）。改回累加后 5 条判据立刻变红，
> 与 `app-binding` §4.2 的注入 A 同源。

### 4.3 本机"意外真跑"到的那一半（如实登记，不冒充 CI）

按纪律本机不编译含 Slint 的 crate，因此 `run-gates.sh crate yeban-app` 会 SKIP。
但 `scripts/dev/cargo-local.sh` 只拦 `--workspace` / `--all`，所以下面这条**真的跑完了**：

```bash
bash scripts/dev/cargo-local.sh clippy -p yeban-app --all-targets --locked -- -D warnings
# Finished `dev` profile in 18.72s，退出码 0（零告警）
```

它的**额外价值**（不是替代 CI）：`build.rs` 里的 `slint_build::compile("ui/app.slint")`
在这一步**真的执行了**，生成物
`target/debug/build/yeban-app-*/out/app.rs` 里能看到新增绑定
（`set_note_positions` / `set_note_ys` / `set_note_widths` / `set_track_colors` /
`set_track_color_labels`），因此：

- **`.slint` 语法与语义检查通过**（新增的 `[color]` 数组、色块、音符数组下标全部合法）；
- **Rust ↔ `.slint` 的属性名与类型对齐**（`-D warnings` 下没有未定义 setter）；
- **`src/test_port_adapter.rs` 与 `tests/**` 的类型检查通过**（我新增的 Tier-1 判据能编过）。

**仍然只有 CI 能给出读数的**（本机**没有**验证）：
Tier-1 运行时控件树的**内容**、`track-{i}-color-swatch` 到底在不在树里、
音符 `bounds` 的真实像素、截图指纹 / 颜色数 / SSIM、`cargo test` 的**运行**结果
（`cargo test` 需要 Slint 完整 codegen，属本机禁跑的重活）。

> **后续**：这一批已由 CI run 37235793566 读出（全绿），逐条数字见 §5.1 ——
> 其中 `track-{i}-color-swatch` 在运行时树里、标签等于投影的 `#RRGGBB`（缺色为回退色）、
> 音符 `x` 每步恰好 +32 px、`y` 随音高严格递减，全部有实测。
> `cargo-local.sh clippy` 这条本机捷径**不能**替代它们（它不做 codegen、不跑光栅化）。

---

## 5. CI 判决

> 未读到的判决一律记 `pending`。判决由 `scripts/dev/ci-verdict.sh line/app-completion --watch` 读回
> （退出码 0 ⇒ 判决 SHA == 分支 tip）。

| 轮 | commit | run | 结论 | 说明 |
| :-- | :--- | :--- | :--- | :--- |
| 1（代码净判决） | `b9b0bab` | [37235793566](https://github.com/gradetwo/yeban/actions/runs/37235793566) | **success（本线的净判决）** | `checks` / `lockfile` / `plan` / `deny` 全绿；**`rust (yeban-app)` = success（3m41s）**：`cargo clippy -p yeban-app --all-targets --locked -- -D warnings` 零告警，`cargo test -p yeban-app --all-targets --locked` 四个目标是 **`67 passed`**（lib：`bridge`/`scene`/`elements`/`open`/`input`，本线新增 8 条）、**`0 passed`**（bin）、**`2 passed`**（本线新增的集成目标 `tests/open_project_file.rs`）、**`9 passed`**（Tier-1，含本线追加的色标与音符几何断言）、**`4 passed`**（`tests/live_ui_mcp.rs`）。`rust (workspace 全量)` 被 `plan` **跳过**（受影响集合只有 `yeban-app` + 一份 docs 文件）—— 因此"全量腿也绿"**不**由本线这一轮证明（§7 needs-6）。 |
| 2（docs-only） | `8283230` | [37236114455](https://github.com/gradetwo/yeban/actions/runs/37236114455) | **success** | 只改本文件 ⇒ `plan` 判"受影响集合为空"，两条 `rust` 腿都按设计**跳过**（`-`，不是 failure）；`checks` / `lockfile` / `deny` 绿。这一轮**没有**代码读数（设计如此）。 |

### 5.0 本文件自身提交的读数纪律

- **代码判决锚定 `b9b0bab` / run 37235793566**（那是唯一一轮 `rust (yeban-app)` 真跑的判决）。
- 本文件后续的 docs-only 提交不再逐轮追记判决 —— 它们按设计不触发 `rust` 腿，
  逐轮记只会让表格无限增长（`app-binding` §6.3 有同款结论）。本轮追记所在的提交
  自身也是 docs-only：它的判决只可能覆盖 `checks` / `lockfile` / `deny` / `plan`，
  不能当作代码读数。


### 5.1 CI 实测数字（run 37235793566 的 `rust (yeban-app)` 腿）

```text
running 67 tests  →  test result: ok. 67 passed; 0 failed     # lib（含 open.rs 的 6 条）
running 2 tests   →  test result: ok. 2 passed; 0 failed      # tests/open_project_file.rs（公开 API）
running 9 tests   →  test result: ok. 9 passed; 0 failed      # Tier-1（含本线追加的断言）
running 4 tests   →  test result: ok. 4 passed; 0 failed      # tests/live_ui_mcp.rs
[model-binding] 运行时控件树 82 条; track-*-header=3, section-*-card=2, clip-*-header=2
[model-binding] 工程字段 TrackV3::color[0]=Some("#FF8800") -> 控件树 track-0-color-swatch.label="轨道色标 #FF8800"
[model-binding] 工程字段 TrackV3::color[1]=Some("#3366FF") -> 控件树 track-1-color-swatch.label="轨道色标 #3366FF"
[model-binding] 工程字段 TrackV3::color[2]=None         -> 控件树 track-2-color-swatch.label="轨道色标 #2C3A63"
[model-binding] 工程字段 MidiNote(start=0,    pitch=60) -> 控件树 note-…0100-rect bounds=(68, 907)
[model-binding] 工程字段 MidiNote(start=960,  pitch=64) -> 控件树 note-…0101-rect bounds=(100, 851)
[model-binding] 工程字段 MidiNote(start=1920, pitch=67) -> 控件树 note-…0102-rect bounds=(132, 809)
[model-binding] 工程字段 MidiNote(start=2880, pitch=72) -> 控件树 note-…0103-rect bounds=(164, 739)
[model-binding] 状态 A (filled_project): 1920x1080, 非黑 2073600 (100%), 颜色 2831 种, PNG 6222418 字节, 指纹 214fd0608f81b3a3
[model-binding] 状态 B (demo_project):   1920x1080, 非黑 2073600 (100%), 颜色 3086 种, PNG 6222418 字节, 指纹 5daadb8b3ea09869
[model-binding] 切换后运行时控件树 101 条; 工程驱动的树 82 条 —— 两者必须不同
控件树计数: 注册表 190 条 / 运行时 101 条 / 运行时有而注册表无 0 条 / 注册表有而运行时无 89 条
关键单例覆盖率: 39/39 = 100%（硬下限 90%）
重复族实测: track-*-header=6, note-*-rect=7, clip-*-header=3, velocity-*-bar=6, section-*-card=4, tab-*-button=3, sidebar-item-*=8, piano-roll-tool-*-button=5
[D24] 汉字墨迹 648 px (下限 150);  状态 A 两次截图逐字节相同: true
[UI-MCP-003] 抖动未遮罩 SSIM=0.991552 / 遮罩后 1.000000; 静态回归未遮罩 0.636225 / 遮罩后 0.636252
```

| 证据 | 数字 | 它排除了什么 |
| :--- | :--- | :--- |
| **色标真的到了运行时树** | `track-0-color-swatch.label == "轨道色标 #FF8800"`、`#3366FF`、缺色的 `#2C3A63` | 排除了"颜色只进了 Rust 没进 `.slint`"与"缺色没有回退路径" |
| **色标族规模 == 轨道规模** | 运行时树 79 → **82**（filled 3 轨）/ 95 → **101**（演示 6 轨）；注册表 184 → **190** | 排除了"色块只画了第一个"或"色块被裁剪掉" |
| **音符 x 来自 tick** | `start = 0/960/1920/2880` ⇒ `x = 68/100/132/164`（**每步恰好 +32 px** = 960 ÷ 30） | 排除了"x 还是索引布局"（索引布局的步长是 76px 且与 tick 无关） |
| **音符 y 来自音高** | `pitch = 60/64/67/72` ⇒ `y = 907/851/809/739`（**严格递减**） | 排除了索引布局（索引布局下 y 与音高**同向**递增） |
| **既有契约未被破坏** | 注册表 190 / 运行时 101 / 未登记 **0** / 单例覆盖 **39/39 = 100%** / 重复族计数与既有实测一致 | 本线只**增加**了一个族，没有动任何既有语义 ID |
| **像素仍是确定的** | 同一状态连续两次截图**逐字节相同** `true`；D24 汉字墨迹 648 px | 光栅化确定性、CJK 字体判据都没退化 |
| **换工程 ⇒ 换像素** | 指纹 `214fd0608f81b3a3`(filled) ≠ `5daadb8b3ea09869`(demo) | 树变了、像素也变了 |

> 与 `app-binding` 的指纹对比：本线**故意**改变了两张演示截图的字节
> （音符从索引布局换成 tick 定位、每轨多一个色块），因此
> `app-main-window-arrangement-full-1920x1080.png` 与状态 B 的指纹不再是
> `ef5972f3ac60466f` —— 这是"位置真的来自工程"的**代价**，也是它的证据。
> artifact `ui-screenshots-yeban-app` 里可以人眼复核这四张图。


---

## 6. 未实现项（如实登记，**不是**静默降级）

| # | 未实现 | 现状 | 归属 / 阻塞 |
| :-- | :--- | :--- | :--- |
| 1 | **卷帘视口裁剪 / 滚动** | 音高窗口固定 16 条车道（`60..75`），窗口外音高**钳制**进两端；`[UI-NOTE-001]` 的 R-Tree / 裁剪未做 | `yeban-render` + 视口状态（`MODEL-ISO-001` 的第二层） |
| 2 | **卷帘的作用域** | 注入的仍是**片段池的全部 MIDI 音符**，不是"当前编辑片段"；音符的 `clip_id` 已进投影，为将来的作用域过滤留了抓手 | "当前片段"属会话运行态，`app-binding` §8.2 已登记 |
| 3 | **音符时值的可视化宽度** | `width` 已按整数像素差给出（有 1px 地板），但**最小可视宽度 ≤ 3px 时不画**（`[UI-NOTE-001]` 的裁剪语义）未做 —— 亚像素宽的音符可能仍在树里但看不见 | 同上 |
| 4 | **段落 / 场景色标** | `SectionV3::color` / `SceneV3::color` 已进投影，但 `.slint` 仍未消费（本线只做**轨道**色标，按任务书范围） | 同轨道色标，复用 `parse_hex_color` 即可（一条后续线的小活） |
| 5 | **混音台通道条** | `mixer_console.slint` 仍是 `for track_index in 6` + 内联 `FADER_LEVELS`；默认视图里它隐藏（`console-tab = 0`），因此不在运行时树里 | `[ARCH-UI-002]` 电平 SPSC；`app-binding` needs-1 |
| 6 | **推子 / 声相 / 音量拖拽回写** | 投影只**读**；没有 UI → 模型方向（`Op` 日志 / 引擎调度） | `yeban-engine` + `yeban-model::Op` |
| 7 | **自动化曲线 / 宏 / 设备链** | `automation_lanes` / `macros` / `devices` 仍未进视图；`device_rack.slint` 用 `scene::DEVICE_NAMES` 常量 | `[UI-NOTE-004]`；设备机架应由 `devices` 驱动 |
| 8 | **走带位置 / 当前分支** | 仍是 `SESSION_TIMECODE` / `SESSION_BRANCH_NAME` 占位常量（不属 `YebanProjectV1`） | `yeban-engine::EngineSnapshot` + `CommitGraph` |
| 9 | **`.yeban` 打开接进 CLI / 会话** | 入口是**纯函数 + 公开入口**（`yeban_app::open`），**没有**接进 `main.rs` 的事件循环 —— 按任务书要求（红线 6 与"控制面/事件循环共用线程"是独立裁决） | 集成者 / 控制面线；见 §7 needs |
| 10 | **`.yeban` 的写出方向** | 本线只做"打开"；`write_project_container` 在容器层有，但 app 侧没有"另存 / 保存"入口（那要接 Op 日志与 `ARCH-SEC-004` 原子落盘） | `yeban-mcp::domain::store` + 会话层 |
| 11 | **颜色语法未进规范** | 模型只说"界面色标"；本线取"可选 `#` + 3/6 位十六进制"（CSS 惯例），其余回退 | needs-2（建议 ADR） |

---

## 7. needs（需要别人做 / 需要人类裁决）

1. **`.yeban` 打开接进 CLI**（`--open <path>`）：入口已在 `yeban_app::open`（零 Slint、
   有 27 条本机判据覆盖），但**接线必须避开**红线 6 与"控制面/事件循环共用线程"的裁决 ——
   需要集成者决定它落在 `main.rs` 的参数解析、还是活窗口装配那条路径（`live_surface.rs`）。
2. **`TrackV3::color` 的语法规范缺口**：建议在 ADR-0001 增一条 D 编号裁决，把
   "可选 `#` + 3/6 位 ASCII 十六进制；大小写不敏感；3 位按 CSS 展开；其余拒绝并回退到
   `Tokens.line-strong`"写成契约（含"回退必须可判据化"）。本线已按此实现并两侧对账。
3. **`docs/ledger/app-binding-notes.md` 的未实现项 #1 / #4 / #8 可以关掉**：
   音符 tick 位置与轨道色标**已实现**，`.yeban` 打开的**入口**已交付（接线仍是 #9 的边界）。
   建议集成者在合并时更新那三条，并在 `docs/DEVELOPMENT_LEDGER.md` 记一笔。
4. **段落 / 场景色标**（§6 未实现项 4）：是同一套 `parse_hex_color` 的复用，
   一条小线即可完成；本线按任务书范围只做轨道色标。
5. **`ui-shell-notes.md` 的 pending**：卷帘的 `.slint` 内联数据债在本线之后只剩
   `white-key-pattern` / `tool-names`（规范级常量），可以据此更新。
6. **`rust (workspace 全量)` 腿**：本线只动 `crates/yeban-app/**` + 本文件，
   按 `changed-crates.py` 的推导大概率**只**触发 `rust (yeban-app)` 矩阵腿；
   "全量腿也绿"这条不由本线证明（与 `app-binding` §9.6 同一处境）。

## 8. TODO(hoist)

1. **hoist → ADR-0001**：`app-binding` 已建议一条"投影层零 Slint + 整数位置 + `Result` 溢出"
   的裁决；本线**追加两条**可复用约束：
   ① **界面色标的解析必须只有一处**（投影层），且非法输入必须有**文档化的回退值**；
   ② **容器 / 归档类错误必须原样上报**，任何"吞掉错误换一个空对象"的实现路径都是缺陷
   （本线用注入 C 证明了它会被 5 条判据抓住）。
2. **hoist → `docs/DEV_WORKFLOW.md`**：本机验证"含 Slint 的 crate"的**第三条**手段 ——
   除了切纯模块 + `rustc --test`（`app-binding` 那一条），还可以用
   `cargo-local.sh clippy -p <crate> --all-targets -D warnings`：
   它在**不跑 `cargo test`**（不做完整 codegen）的前提下，让 `build.rs` 真跑一遍
   Slint 编译器并做全目标类型检查 —— 是"`.slint` 能不能编译 / 属性名对不对"的
   最便宜机械证据（本线 18.7s）。
3. **hoist → 判据写作**：`slint_text_contracts_for_colors_and_pitch_lanes` 的形态
   （读 `.slint` 原文做**非注释行**的正 / 负向断言）值得作为"投影 ↔ `.slint` 文本耦合"
   的标准手法写进方法论文档（本仓库已有 `SLINT_MANIFEST`、`component_paths_point_at_real_slint_files`
   两个先例，本线补上"负向断言必须排除注释"这一条，因为负向断言第一版就被自己的注释绊红了）。
