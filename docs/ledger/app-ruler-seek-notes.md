# `ruler-seek` 工作线台账 —— **点编曲标尺 ⇒ 走带光标真的到那一拍**

- **台账类型**：交付映射 / 转换公式与论证 / 判据清单 / 本机真跑与 CI 的严格区分 / 未实现项与 needs（**不是规范**）
- **工作线**：`line/ruler-seek`（worktree `yeban/.worktrees/ruler-seek`，基线 main `2860f88d`）
- **所有者目录**：`crates/yeban-app/**`（本文件是唯一新增的共享区文档）
- **规范来源**：`[ROAD-M3-001]`（Slint 工作区装配）、`[UI-NOTE-002]`（坐标双向映射：像素 ↔ tick 的
  同一网格）、`[MODEL-ISO-001]`（走带定位 = 会话运行态，⛔ 不进工程）、`[ARCH-TOP-002]`（线程拓扑：
  本片的回调只在 UI 线程，⛔ 不触"实时音频回调零分配"红线）、`ADR-0004` Q5（横向缩放归宿主视图态）；
  缺口登记：`feature-alignment.md` 错位 7「控件存在 ≠ 回调接线」的**标尺**那一条。

---

## 1. 落地清单（文件 ↔ 规范 ID ↔ 说明）

| 文件 | 规范 ID | 说明 |
| :--- | :--- | :--- |
| `crates/yeban-app/ui/workspace/arrangement_view.slint` | `UI-NOTE-002` | 标尺带（`arrangement-ruler`）加**零像素 `TouchArea`**（第一个子元素、不带 `accessible-id`）＋ `callback ruler-seek(length)`：只上报 `mouse-x`，⛔ 界面不做位置算术（与轨道头拖拽面 / `clip-drag-*` 同一条纪律） |
| `crates/yeban-app/ui/app.slint` | 同上 | root 侧同名回调 + 视图实例化块里逐字透传（与 `clip-select` 同一处） |
| `crates/yeban-app/src/host.rs` | `UI-NOTE-002` `MODEL-ISO-001` | `ruler_seek_tick`（px→tick 的**唯一**换算落点，纯函数）、`ruler_span_ticks`（网格右缘）、`wire_ruler_seek`（**唯一**注册者；`try_borrow_mut` 降级 + 先放借再写界面，`#212` 同款两道防线） |
| `crates/yeban-app/src/main.rs` | `ARCH-TOP-002` | `wire_callbacks` 里调用一次：引擎 `Rc` 与 `wire_transport` **同一个**、投影 `Rc` 与 `wire_clip_scope` **同一个** |
| `crates/yeban-app/tests/arrangement_ruler_seek.rs`（**新建**） | 同上 | 六条端到端 + 四条纯函数判据（见 §3） |

## 2. 换算口径（每一条都是决定）

- **像素列 → tick**：`px_to_tick(floor(max(x, 0)), ticks_per_pixel)`，`ticks_per_pixel` 读
  [`host::roll_ticks_per_pixel`]（= 投影 `tick_to_px` 的同一个来源）⇒ 一个列 `p` 覆盖
  `[p×tpp, (p+1)×tpp)`，且 `tick_to_px(结果) == p` 恒成立（判据 ② 钉住往返）。
- **右界**：`bar_positions.len() × bar_length_ticks` —— 用**投影自己的**两个读数，⛔ 不在宿主重算
  `bar_count` 公式（那是 `bridge` 的单一事实源）。点出右缘 ⇒ 夹在右缘（⛔ 不把光标送去没画网格的地方）。
- **左界**：负 x ⇒ `0`（夹紧，`u64` 没有负数）。非有限（NaN / ±Inf）⇒ **不响应**，一条命令都不发。
- **不吸附**：⚠ 见 §5。

## 3. 判据（`tests/arrangement_ruler_seek.rs`）

| # | 名字 | 证的事 |
| :--- | :--- | :--- |
| ①② | `clicking_the_third_bar_line_seeks_to_that_bar` | 真实指针事件 → 命中测试 → 回调 → `SeekTicks`：引擎位置 == `3 × bar_length_ticks`、镜面 `playhead-ticks` 同值、日志恰加一条、落点换算回像素 == 点击列 |
| ③ | `the_click_follows_the_projection_zoom_not_a_constant` | 换 60 tick/px 并重投影后点第 5 条线 ⇒ 仍是 `5 × bar_length_ticks`（证明用的是投影的缩放，不是常量 30） |
| ④ | `a_click_right_of_the_drawn_grid_clamps_to_its_right_edge` | 960 tick/px 下网格只有几十列宽 ⇒ "右缘之右"是**窗口内真实坐标**，位置落在右缘 tick |
| ⑤ | `a_non_finite_pointer_reading_moves_nothing` | `NaN` / `Inf` ⇒ 位置与日志一位不动 |
| ⑥ | `seek_from_the_ruler_keeps_the_transport_state` | 播放中定位 ⇒ 仍在播放（seek 只搬位置） |
| 纯函数 | `a_negative_column_seeks_to_the_start` / `the_column_is_truncated_like_the_projection_division` / `a_column_past_the_grid_clamps_and_a_non_finite_one_is_refused` / `the_grid_right_edge_is_the_line_count_times_one_bar` | 左夹紧 / 截断方向 / 右夹紧与拒绝 / 溢出饱和 |

## 4. 本机真跑 vs CI

- **本机真跑**：`rustfmt --check`（三份 `.rs`）＋ `scripts/gates/check_feature_alignment.py` ＋
  `check_test_criteria_presence.py`（302 个 `tests/*.rs` 全绿）。
- **本机没跑**（`AGENTS.md` §5 禁止：Slint 重依赖编译）：`cargo clippy --all-targets -D warnings`、
  本片六条判据、无头 golden。**未读回的判决一律记 `pending`**，不写成"通过"。

## 5. 未实现项与 needs（⛔ 不私自发明答案）

- **needs R1｜小节 / 拍吸附**：参考规范 `DESKTOP_UI_UX` §4.2 提到标尺，但那份 2026-10-06 起是
  **参考件、非绑定**；Normative 两份文档里**没有**"点击标尺应吸附到小节/拍"的规定。因此本片如实做
  **逐 tick** 定位（点在哪一列就停在哪一列）。若负责人要的是"点标尺 = 跳到最近小节线"，
  需要一条 ADR 裁决网格（小节 / 拍 / 既有 `ROLL_SNAP_GRID_TICKS`）与越界方向，届时只改
  `ruler_seek_tick` 一处（判据 ②③ 会立即指出网格变了）。
- **needs R2｜无障碍侧的定位**：标尺带是 `accessible-role: text`（不是控件），本片只接指针 ⇒
  屏读器 / 键盘**没有**"定位到第 N 小节"的入口。既有键盘路径只有 `Home`/`Space×2` 回起点。
  要不要给它一个 `accessible-action-default`（会新增一个 a11y 节点 ⇒ golden 与元素树要重录）
  属另一片，⛔ 不在本片顺手加。
- **needs R3｜拖过标尺连续定位**：本片只在 `down` 上发一条命令（`move` 不发 ⇒ 不与
  轨道头拖拽 / 剪辑拖动抢手势）。"按住拖动扫定位"要不要做，同样待裁决。
