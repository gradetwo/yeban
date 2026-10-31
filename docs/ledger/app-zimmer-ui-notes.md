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
| `U-13` 三个"画了但无动作"的控件 | 报告只给结论未点名；现读即 `KNOWN_ACTIONLESS_CONTROLS` 的 **3** 条（`ai-rail-intent-button` / `transport-commit-button` / `transport-branch-button`），处置已由 **`HD-72` / `ADR-0052`** 登记且**未决**（维持现状 is 当前处置）⇒ ⛔ 本线不自行选产品姿态 |

## 3. 后续切片：编排行区**纵向滚动**（`N-UI-1` 的产品面）

`N-UI-1`（"控件树只含视口内条目"）本身是测试能力缺口，但这批探针同时暴露了一个
**产品缺陷**：编排行区没有滚动 ⇒ 轨道多到超出可视高后，后面的行**无法到达**
（报告的真实工程 28 轨只有前 ~8 行可用）。

| 落点 | 改动 | 规范 ID | 判据 |
| :--- | :--- | :--- | :--- |
| `ui/workspace/arrangement_view.slint` | 五个行锚定层（包头 / 车道 / 剪辑 / 拖动面 / 自动化带）包进 `Flickable`（`content-width: self.width` 只滚纵向；子元素 y 减 `rows-top`；⚠ 必须写 `content-*`，`viewport-*` 是 `@deprecated` 别名，绑定挂在别名上会被上游 flickable pass 的自动生成内容高顶掉 ⇒ 滚轮恒被忽略）＋ 新输入 `sheet-height` | `UI-NOTE-001` | `tests/arrangement_vertical_scroll.rs` |
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

## 4. 本片第一票红与根因（run `41208446082`，macOS 腿）

`wheel_scroll_moves_rows_up_by_the_delta` 报 `left: 90 / right: -310`（首行一位不动）、
`scrolling_to_the_end_clamps_and_reveals_the_last_row` 报"滚到底后首行仍可达"——**两条红的
形态相同：滚轮根本没生效**，不是"滚了但幅度不对"。

逐段排除后定位到 `.slint` 的属性名：

- 上游 `Flickable::is_allowed_scroll_direction`（`items/flickable.rs`）要求
  `content_height > 自身 height`，否则 `process_wheel_event` 直接 `EventIgnored` 并**转发给父级**
  ⇒ 内容高不够时，滚轮是**静默无效**的（不报错、不警告）。
- `viewport-width/height` 在 1.18.1 是 `content-*` 的 `@deprecated` 别名
  （`builtin_elements.rs`），而 `passes/flickable.rs` 的 `fixup_geometry` 用
  `is_binding_set("content-height")` 判断"有没有显式绑定"来决定是否自动生成内容高；
  该 pass 跑在 `passes/remove_aliases.rs`（`passes.rs` 第 173 行 vs 第 247 行）**之前** ⇒
  写在别名上的绑定它读不到，于是装上自动绑定（自动值 = 容器自身高，且 `for` 循环子元素
  不参与计算 = 上游 issue #407），随后别名折叠时显式值可能被顶掉。
- 排除项（都核过源码/现读代码）：`PointerScrolled` 的 `Cancelled` 相位确实**直接 set**、
  无动画（`api.rs` → `flickable.rs`）；宿主注入链 `host.rs → bridge.rs → app.slint → arrangement_view.slint`
  逐级都在；界面内没有任何 `scroll-event` 处理器 ⇒ `TouchArea` 对滚轮返回 `Reject`，不会吞事件。

⇒ 修法是**把绑定写回规范名 `content-*`**，并加判据⑥把"内容高注入到位且高出窗口"钉成前置：
以后这一环再断，日志会直接指出是注入断了还是容器没接住，而不再是"y 不动"这种两解的读数。

另有一处判据松绑：交集行数下限 `>= 8` 改 `>= 5`——滚动 400px 后**顶部**若干行会被裁出树，
交集大小由"可视行数 − 滚掉的行数"决定，8 是照抄未滚动时的读数，不是语义要求。

### 第二处缺陷：滚轮符号（同一个红里藏着的第二个原因）

`ensure_in_bound`（`items/flickable.rs`）把 `content-x/y` 夹在 **`[视口尺寸 − 内容尺寸, 0]`**，
而 `process_wheel_event` 用的是 `new_pos = current + delta`、**不取反**。⇒ 静止位
（`content = 0`）上**正的 delta 被夹回 0，一位都不动**。判据②③原先写的是
`delta_y = +400 / +100_000` 期望"看下面的行"，方向本身就不成立。

修法是给判据两个具名助手（`scroll_rows_down(amount)` 送 `-amount`、`scroll_rows_up(amount)`
送 `+amount`），把符号收在一处；这与真实滚轮的符号一致（向下滚 = 负 delta）。

第二票红（run `41247139184`，`rust (yeban-app)` / macos / windows 三腿）红在 **clippy 编译期**
（`f32: TryFrom<u32>` 不存在 —— `Size.height` 是 u32），目标二进制根本没执行 ⇒ 按 §6.3 第 4 条
这**不是**判据失败，只是没跑到；改 `as f32` 后与符号修复合成同一票重推。

教训（可复用）：**上游的夹紧区间是有方向的**。写滚动判据前先看 `ensure_in_bound` 的
`min/max`，别按"正数=往下"的直觉设符号；直觉那一版会退化成"什么都没发生"，与"容器不可滚"
在日志里长得一模一样。

教训（可复用）：**用 `@deprecated` 别名绑定属性，等于把一个显式值交给上游 pass 的执行顺序去猜**。
凡上游 pass 会按属性名自动补绑定的元素（`Flickable` 的内容尺寸即其一），一律写规范名。

## 5. 本机真跑 vs CI

- 本机：`rustfmt --check`、守卫脚本。Slint 重依赖编译与判据执行一律交 CI（`AGENTS.md` §5）。
- 判决只认 CI 读回；未读回一律 `pending`，⛔ 写成"通过"。判绿需确认目标二进制**真的执行过**
  （`Running tests/` 计数 + 具名 `test ... ok`），⛔ 把 fail-fast 的空白当证据。

## 6. 第二、三票红：一处编译期红 + 一处"写死的滚动量"（run `41247139184` / `41247717524`）

**第二票（`41247139184`，sha `d103af6c`）红在编译期，不是判据**：三条腿（`rust (yeban-app)` /
macos / windows）的 `clippy (-D warnings)` 步骤报 4 个
`error[E0277]: the trait bound f32: TryFrom<u32> is not satisfied`
（`WINDOW.height` 是 `u32`，判据里用了 `f32::try_from(...)`）。按 `AGENTS.md` §6.3 第 4 条口径，这只能记作
**"目标二进制没跑到"** —— 该 run 的日志里根本没有 `Running tests/arrangement_vertical_scroll.rs`
这一行，⛔ 不能读成"滚动判据失败"。修法是照仓库既有写法（`tests/asset_pool_panel.rs`）用 `as f32`。
⚠ 顺带记一条我自己的错：`f32::from_i32` **不存在**（标准库只给 `i8/i16/u8/u16 → f32` 的 `From`，
`i32 → f32` 有损所以没有）—— 写之前该现读，不该按"对称一定有"的直觉。

**第三票（`41247717524`，sha `f1337217`）第一次让判据真的跑了**，读回的口径是"数 `Running tests/`
= 5 个二进制、目标二进制在其中、再数具名 `test … ok`"：

| 读数 | 值 |
| :--- | :--- |
| `Running tests/arrangement_vertical_scroll.rs` | 出现（该二进制真的执行） |
| 4 条判据 | **3 passed / 1 failed** |
| 红的那条 | `wheel_scroll_moves_rows_up_by_the_delta`：`两次快照必须有一段交集才读得出位移（交集只有 1 行）` |

前两票的根因（内容高别名 + 滚轮符号）**至此被证明已修**：`scrolling_to_the_end_clamps_and_reveals_the_last_row`
绿 ⇒ 滚一下确实把末行进树、把首行裁出；`short_projects_do_not_scroll` 绿 ⇒ 不溢出时偏移恒 0。

**第三票的根因是我自己写死的常数**：判据②的探针取了固定 400px，而我把"行区视口高"算成了
`窗口 900 − 行区上沿 42 = 858`。真实视口不是这么来的 —— 编排区下面还有底部控制台。反推实测：
交集 = 1 行 ⇒ `视口高 − 400 ≈ 一个行距` ⇒ 行区视口高约 **456px**（首行在窗口内 y=90，故行区底约 546）。
固定 400px 因此几乎吃满整个视口，"两次快照的交集"退化成一行，位移样本读不出来。

修法：判据②的 delta **不再写死**，改取"两个行距"（行距 = 树里第 1 行与第 0 行的 y 差，同一份投影），
并把"滚动前行区里至少 5 行"钉成前置条件 —— 于是这条红将来只可能是"方向/夹紧/投影"坏了，
不会再是"探针选得太长"。

教训（可复用）：**滚动探针的量必须由被测量本身导出，不能凭窗口尺寸推视口尺寸**。
一个固定的像素探针同时依赖视口高与内容高，而这两者都不是测试说了算的（布局归宿主）。
