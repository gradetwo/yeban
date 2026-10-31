# `zimmer-ui` 工作线台账 —— 按 `~/zimmer-ui-mode-report.md` 修**非 MCP** 的 UI 缺口

- **台账类型**：报告条目 ↔ 代码落点 ↔ 判据 ↔ 未做项（**不是规范**）
- **工作线**：`line/console-tab-key`（worktree `yeban/.worktrees/console-tab-key`）
- **来源**：`~/zimmer-ui-mode-report.md`（2026-10-31 探针报告）＋ `~/zimmer-20261031-package.zip`
  （**后者全为 MCP 面** ⇒ 本线不动，留给 MCP 线）
- **规范来源**：`[UI-NOTE-002]`（像素 ↔ tick 同一网格）、`[UI-A11Y-003]`（屏读动作与点击源同落点）、
  `[UI-TEST-001]`（语义 ID 寻址）、`[MODEL-ISO-001]`（视图态 / 会话运行态 / 持久层隔离）；
  裁决：`docs/adr/ADR-0054-ruler-bar-snap.md`（Proposed）

---

## 1. 本批修复（文件 ↔ 规范 ID ↔ 判据）

| 报告条目 | 落点 | 规范 ID | 判据 |
| :--- | :--- | :--- | :--- |
| `U-05` 段落卡片只显示、不可点 | `ui/workspace/arrangement_view.slint`（卡片 `accessible-action-default` + 零像素 `TouchArea` → `section-seek(index)`）、`ui/app.slint`（透传）、`src/host.rs`（`wire_section_seek`：查 `view.sections[index].start_tick` → `commit_seek`）、`src/main.rs`（装配一次） | `UI-NOTE-002` `UI-A11Y-003` `UI-TEST-001` `MODEL-ISO-001` | `tests/arrangement_section_seek.rs`：① 真实指针点 `section-{i}-card` ⇒ 引擎位置 == 该段 `start_tick`、日志末尾 `SeekTicks`；② 无障碍默认动作同落点。⚠ 卡片**不选中**、不改工程（定位 = 会话运行态） |
| `U-07 R1` 线间点击不吸附 | `src/host.rs` `ruler_seek_tick`：`px → tick → bridge::snap_tick(tick, bar_length_ticks) → min(span)` | `UI-NOTE-002` | `tests/arrangement_ruler_seek.rs` ⑦ + 纯函数（吸附 / 平局向更晚） |
| `U-07 R2` 屏读 / 键盘无"第 N 小节" | `arrangement_view.slint`：标尺 `accessible-role: slider` + `increment/decrement/set-value`；`src/host.rs` `ruler_step_tick` / `ruler_bar_tick` | `UI-A11Y-003` | ⑨ + 纯函数（±1 小节夹紧 / 1 起的第 N 小节 / 非数字零命令） |
| `U-07 R3` 拖动不定位 | `arrangement_view.slint`：`TouchArea.moved` 与 `down` 落同一条 `ruler-seek` | `UI-NOTE-002` | ⑧（按下 + 移动 + 松手 ⇒ 日志恰加两条） |
| `U-11` Esc 不关闭提案抽屉 | `src/host.rs` `Action::Cancel`：导出弹窗之后、手势取消之前，抽屉开着 ⇒ `invoke_toggle_ai_drawer()`；`src/input.rs` 文档同步（⛔ 仍不是"放弃提案"） | `UI-A11Y-003` | `tests/ai_drawer_key_ui.rs`：`escape_closes_the_open_ai_drawer_and_then_passes_through`（开 ⇒ Esc 消费并收起 ⇒ 再 Esc 放行） |

装配纪律：`U-07` 的引擎 `Rc` 与 `wire_transport` 同一个、投影 `Rc` 与 `wire_clip_scope` 同一个；
`U-05` 复用逐段 `start_tick`（投影的单一事实源），⛔ 界面算术、⛔ `commit_ops`。

**像素面**：本批全部新增元素都是零视觉的（`TouchArea` / 无障碍属性 / 回调），
默认帧的绘制属性一位未动 ⇒ `[UI-MCP-003]` 的 Linux 基准**不需要**重录（⛔ `gate=goldens`）。
角色 `text → slider` 只改无障碍树，不改像素。

## 2. 本批**不改**的条目（现状已登记，等裁决 / 属别的线）

| 条目 | 为什么本线不接 |
| :--- | :--- |
| `U-09` 抽屉内无行间键盘移动 | `Focus` 枚举只有 3 态（主画布 / 文本输入 / 其他），方向键已绑音符微调 ⇒ 需要一条**焦点模型**裁决（新焦点态 / 与音符微调的优先级），⛔ 在缺口报告层自行发明 |
| `U-10` 抽屉"放弃"无键盘快捷键 | `Escape` 已承担"取消在手拖动 / 关面板"；"一键两义"（放弃提案 vs 收尾）没有规范裁决，且拒提案的既有唯一入口是 `musical-pr-reject-button` |
| `U-12` 自动化泳道不覆盖主总线 | 投影**有意**过滤主总线（`ADR-0004` 的泳道归属）；要做需要一条"主总线自动化如何编辑 / 如何投影"的产品裁决 |
| `U-16` 运行时没有打开入口 | 本线已有 `Cmd/Ctrl+O` 切片（`554eaaa4`，复用既有打开入口）；报告探针走测试口的打开路径 ⇒ 产品内打开的最终证明在 `Cmd/Ctrl+O` 的判据与读回，不在本台账重复 |
| `N-UI-1` 控件树只含视口内条目 | 测试能力缺口（窗口化控件树），不是产品缺陷；⛔ 在 UI 线顺手改测试口。它暴露的**产品面**（行区无滚动 ⇒ 后面的行不可达）已在 §3 单独落地 |
| `N-UI-2` 循环重复没有 UI 表示 | 需要先回答"循环区域画成什么"（`ADR-0010` 只定坐标语义），属视图设计缺口 |

## 3. 后续切片：编排行区**纵向滚动**（`N-UI-1` 的产品面）

`N-UI-1`（"控件树只含视口内条目"）本身是测试能力缺口，但这批探针同时暴露了一个
**产品缺陷**：编排行区没有滚动 ⇒ 轨道多到超出可视高后，后面的行**无法到达**
（报告的真实工程 28 轨只有前 ~8 行可用）。

| 落点 | 改动 | 规范 ID | 判据 |
| :--- | :--- | :--- | :--- |
| `ui/workspace/arrangement_view.slint` | 五个行锚定层（包头 / 车道 / 剪辑 / 拖动面 / 自动化带）包进 `Flickable`（`viewport-width: self.width` 只滚纵向；子元素 y 减 `rows-top`）＋ 新输入 `sheet-height` | `UI-NOTE-001` | `tests/arrangement_vertical_scroll.rs` |
| `ui/app.slint` | 根属性 `arrangement-sheet-height` 转发 | — | 同上 |
| `src/bridge.rs` / `src/host.rs` | `ViewState::track_sheet_height()`（最后一行底沿 − 行区上沿）→ `apply_row_geometry` 注入 | `MODEL-AST-002`（投影派生，⛔ 界面算术） | 同上 |

判据覆盖：① 30 轨时末行**不在控件树里**（测试口按可见性裁剪；不可寻址 ⇒ 不可达）；
② 滚轮 `delta_y > 0` ⇒ 两次快照交集的**每行**恰好上移该 delta，且更大索引的行进入树中
（**方向语义**由此钉死；再滚 −400 快照复原）；③ 滚到底夹紧且末行进入树里、y 落在行区；
④ 标尺 / 章节卡片不动；⑤ 演示工程（6 轨）内容不高 ⇒ 偏移恒 0（默认帧 / golden 不漂移）。

⚠ **判据写法教训**（run `41203913356` 三平台同红）：控件树的 `bounds` 不是"离屏也读得到"——
`ElementHandle::is_visible()` 对被子孙裁剪链盖掉的元素返回 false，**视口外的行根本不进树**
（`track-19-header 必须可寻址` panic）。所以判据不能读"末行的 y"，要用"是否可寻址"表达
可达性、用两次快照交集表达位移；夹具行数也要让最大偏移（内容高 − 可视高）**大于**判据的
滚动量，否则 `ensure_in_bound` 把"上移 delta"夹成"上移到头"。

边界：只接**滚轮**（上游 `PointerScrolled` 的 `Cancelled` 相位直接 set、无动画 ⇒ 判据
可立即读几何）；⛔ 横向（仍归宿主的 `scroll-x`，见账本第 199 轮"双重计算"教训）；
⛔ 拖动滚动条 / 触摸惯性。`Flickable` 没有 `accessible-id` ⇒ 控件树不新增节点。

## 4. 本机真跑 vs CI

- 本机：`rustfmt --check`、守卫脚本。Slint 重依赖编译与判据执行一律交 CI（`AGENTS.md` §5）。
- 判决只认 CI 读回；未读回一律 `pending`，⛔ 写成"通过"。判绿需确认目标二进制**真的执行过**
  （`Running tests/` 计数 + 具名 `test ... ok`），⛔ 把 fail-fast 的空白当证据。
