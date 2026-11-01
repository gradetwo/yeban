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

## 7. 第三票的读数补全：三条腿同一个断言 ⇒ 一个根因，不是平台差异

下载三条腿的作业日志现读（`gh api repos/gradetwo/yeban/actions/jobs/<id>/logs`，run `41247717524`）：

| 腿 | `Running tests/arrangement_vertical_scroll.rs` | 4 条判据 | 红的 panic 位置与文本 |
| :--- | :--- | :--- | :--- |
| `rust (yeban-app)` | 出现 | 3 passed / 1 failed | `arrangement_vertical_scroll.rs:246`：`两次快照必须有一段交集才读得出位移（交集只有 1 行）` |
| macos | 出现（job `124674574880` 第 5759 行） | 3 passed / 1 failed | 同一行号、同一句文本 |
| windows | 出现（job `124674574889` 第 5635 行） | 3 passed / 1 failed | 同一行号、同一句文本 |

⇒ **一次根因（写死的 400px 探针）在三个平台上一致复发**，⛔ 读成"平台相关的视口高不同"。
另外两腿的尾部 `error: 1 target failed: -p yeban-app --test arrangement_vertical_scroll` 就是这一条，
其余二进制全绿（各 `test result: ok.`）—— 也就是说 macos / windows 这两票**没有**别的账要还。

## 8. 推送侧的一条操作教训：git-over-HTTPS 会假成功

本片 `9d46e7e3` 一度**推不上去**：`git push` 报 `GnuTLS recv error (-110)` 或
`Failed to connect to github.com port 443 after 133xxx ms`，而 `gh api`（走 `api.github.com`）一直正常
⇒ 判"推没推上去"⛔ 靠 `gh api` 通不通，要现读 `gh api repos/gradetwo/yeban/branches/<线名> --jq '.commit.sha'`
跟本地 `git rev-parse HEAD` 对（本仓库第一次就是靠这条读回发现远端还停在 `f1337217`）。
更阴的一次：把 push 丢到后台包装脚本里，包装脚本 **exit 0** 而输出文件里是一行 `fatal:`
⇒ 判推送只认 `gh api` 的远端 sha，⛔ 认包装脚本的退出码。

可用的绕法（实测）：`git -c http.version=HTTP/1.1 push`（默认 HTTP/2 的连接在这台机器上会卡在 443）。
⛔ 把 `http.version` 写进 git config（改配置属人的决定），只在单次调用上带 `-c`。

## 9. 报告 §10 的一条"仍未验证"读成了事实：编排的**横向**可达只有缩放，没有平移

报告 §10 写"视口外的剪辑块对真实用户是否可达仍未验证"。本票现读代码后的口径（⛔ 未做行为实验）：

| 问题 | 现读事实 | 出处（符号级） |
| :--- | :--- | :--- |
| 编排视图有没有横向偏移输入？ | **没有**。`arrangement_view.slint` 的输入里与 x 相关的只有投影算好的 `clip-positions` / `section-positions` / `bar-positions` 与标量 `ticks-per-pixel`；**没有** `scroll-x` / `view-offset` 一类的 in 属性 | `arrangement_view.slint` 的属性表 |
| 那"看后面的小节"靠什么？ | 只有**缩放**：`Shift+Z` = `ZoomToFit`、`Z` = `ZoomToSelection`，目标值由 `bridge::ticks_per_pixel_to_fit` 一次算出（唯一换算＋夹紧），再重投影 | `input.rs` 第 ⑥ 步、`host.rs` 的 `zoom_action` |
| 缩放的界 | `MIN_TICKS_PER_PIXEL = 1`（最大放大）… `MAX_TICKS_PER_PIXEL = 960`（一个四分音符 1px，最小放大）；文档**明写**夹紧后果："若 `ceil(span / px) > MAX`，本函数给出上界值，**而不是假装装得下**" | `bridge.rs` 的三个常量与其文档 |
| 默认档有多窄？ | 默认 `DEFAULT_TICKS_PER_PIXEL = 30` ⇒ 一个 4/4 小节（3840 tick）= **128px**。行区可视宽**本票未现读**（它随窗口档与侧栏状态变），因此只给公式：可见小节数 ≈ `行区宽 px / 128` | 同上 |

⇒ 结论分两句，⛔ 混成一句：

1. **中等长度工程不是缺口**：按最小放大一格 960 tick/px，横向覆盖 = `px × 960` tick，
   工程跨度小于它时 `Shift+Z` 就能全部装进视口 ⇒ "视口外的剪辑"在默认档下**可达**（靠缩放）。
2. **但没有任何平移入口**：想要"停在第 40 小节、放大到能编辑音符密度"的用法，
   今天只能反复缩放、没有左右滚动条 / 滚轮横向 / 拖动平移。这与 `N-UI-1` 的**纵向**缺口
   （本线已修）是两种病：纵向是"行压根到不了"，横向是"能到但看不细"。

⛔ 本票不顺手补横向平移：报告把它记在**未验证**而不是**缺口**里，
而"横向平移的落点归宿主偏移、还是归 Flickable 的 `content-width`"是一处会撞上
`app-zimmer-ui-notes` §3 那条"⛔ 双重计算"教训的**设计选择** ⇒ 待人类点票再动。

## 10. 纵向滚动票的收盘判决与落地（run `41250318956`）

`line/arrangement-scroll` 的第三票（前两票的根因见 §4 与 §6）于 sha `9d46e7e3` 判**绿**，
按 `[AGENTS.md §6.3]` 的口径逐腿读回：

| 腿 | 读数 |
| :--- | :--- |
| `rust (yeban-app)`（job `124682339791`） | `Running tests/` = **272**，其中 `arrangement_vertical_scroll.rs` 出现 **2 次**（单特性 + 合并特性两档），4 条具名判据各 `ok`，`test result: ok. 4 passed; 0 failed` ×2 ⇒ 非空绿 |
| `macos` / `windows` | success |
| deny / checks / plan / lockfile | success |

落地：`worktree.sh land arrangement-scroll` ⇒ 合并提交 `fec6bd1b`（6 文件，+579/−8，
新建 `tests/arrangement_vertical_scroll.rs`）。落地后 `post_merge_checks` 的两处 FAIL
（`Cargo.lock` 漂移 / 许可清单漂移）与本批**无关**——`git diff --name-only` 里没有任何
`Cargo.*` 或许可文件，两处 FAIL 与本机 1.99.0 pin 拉不到的环境漂移同源。

## 11. 报告缺口对最新 main 的收盘核账（2026-10-31）

`~/zimmer-ui-mode-report.md` 是**落地前**的快照。逐条对**当前 main 的代码**现读后，
四条"已复现"缺口全部已由既有工作线闭合（⛔ 本票不重开、⛔ 不新接线）：

| 报告缺口 | 现状 | 现读出处（符号级） |
| :--- | :--- | :--- |
| **U-05** 段落卡片不可点 | **已闭合** | `host::wire_section_seek` 注册 `on_section_seek`（⇒ `commit_seek(.., "section-seek")`），产品路径 `main.rs` 与无头路径 `live_surface.rs` 各调一次 |
| **U-07** 标尺定位三残缺 | **已闭合** | `wire_ruler_seek` 三条臂：`on_ruler_seek`（点击自由定位，`ruler_seek_tick` 带 `bar` 参数）、`on_ruler_seek_bar`（"定位到第 N 小节"，补 R2 键盘/屏读入口）、`ruler_step_tick`（相邻小节步进，补 R3）；R1 的"线间点击"保留自由定位，小节精确导航改由后两条臂承担（既有设计选择，非本票新造） |
| **U-11** Esc 不关抽屉 | **已闭合** | `host.rs` `Action::Cancel` 臂：`if ui.get_ai_drawer_open() { ui.invoke_toggle_ai_drawer(); return true; }`，注释逐字点名 `U-11`；与走带按钮 / `Cmd/Ctrl+Alt+A` 共用 `toggle-ai-drawer` 单写者 ⇒ 键盘/鼠标/屏读不分叉 |
| **U-16** 运行时无打开入口 | **已闭合** | `host::wire_open_project` + `PortOpenProject: OpenProjectRuntime`，`input.rs` 的 `Cmd/Ctrl+O => Action::OpenProject` ⇒ `ui.invoke_open_project()`（第 ⑩ 票落地的运行时打开切片） |

**仍未闭合的三条，全部卡在人类裁决 / ADR，⛔ 本票不发明答案**：
U-12（主总线自动化泳道被投影过滤 —— 是否该放开是产品决策）、N-UI-2（循环重复的 UI 表示）、
U-09/U-10（抽屉行间键盘移动 / "放弃"快捷键 —— 已登记 `ADR-0058`，状态 `Proposed`，待人类批准）。

⇒ 报告里**所有非 MCP 的可复现 UI 缺口**在 main 上已无待接线项；剩下的是决策依赖项，不属于本票范围。

## 12. 报告 §7/§8/§9/§10 的余下项收盘（补 §11 未覆盖的三条）

§11 收了 U-05/U-07/U-11/U-16 四条；报告还点名 U-08 / U-13，以及 §7 的混音通道可达与 §10 的
"视口外可达"清单。逐条对**最新 main** 现读后的口径：

| 项 | 定性 | 依据 |
| :--- | :--- | :--- |
| **U-08** | **非功能缺口，且本票不猜** | 报告逐字："文档措辞问题，**不影响标尺功能**"＋"未在本轮复现"。探针既未复现、又无确切措辞缺陷原文 ⇒ 改动＝发明；⛔ 不动。若确属根文档措辞，`docs/**` 归集成者 |
| **U-13** | **卡人类裁决** | "三个画了但无动作的控件"——这三个控件**该做什么**是产品决策（`ADR-0052` / `HD-72` 未决）⇒ ⛔ 不接线，等裁决 |
| **§7 混音第 14 条以后通道可达** | **已闭合（本线）** | `line/mixer-horizontal-scroll`（落地 `4c0d3153`）给了通道条的宿主横向偏移 ⇒ 后段通道可滚入视口；判据 `scrolling_to_the_end_clamps_and_reveals_the_last_strip_and_the_panels` 在 run `41250324373` 绿 |
| **§10 第 8 条以后轨道行可达** | **已闭合（本线）** | `line/arrangement-scroll`（落地 `fec6bd1b`）行区 `Flickable` 纵向滚动；判据 `…reveals_the_last_row` 在 run `41250318956` 绿 |
| **§10 Intro 以外段落卡片可达** | **已闭合** | `wire_section_seek`（同 §11 U-05）：卡片点击 ⇒ 定位段起点，配合编排行区滚动可达后段卡片 |
| **§10 Session / 钢琴卷帘标尺点击** | **口径为「单标尺覆盖」** | 现读：`console/piano_roll.slint` 与 `workspace/session_view.slint` **各自都没有独立标尺** ⇒ 报告那两条"仍未验证"由编曲**唯一标尺**（`wire_ruler_seek`）覆盖，不是漏接线 |

⇒ 报告**全部**非 MCP UI 项现读完毕：**可复现功能缺口全部已落地**，**余下全部卡人类裁决**（U-09/U-10/U-12/U-13、N-UI-2）或属**非功能措辞**（U-08）。自主可做的 UI 功能切片 = **零**，与集成者在 `handover-latest.md` 记的"自主可做的下一批 = 零"一致。

## 13. 人类裁决落账 + sidebar 基准重录核账（2026-11-01）

**基准重录核账（`line/sidebar-library`）**：手动档 `gate=goldens` run `41254412881`
conclusion=**success**，`goldens` 作业实跑 23:56:23Z–00:00:37Z（≈4m14s，非空绿）。
产物 `golden-linux` 的 **5 张 PNG sha256 与仓库内已钉死基准逐张相等**（单位＝张；对象＝
`crates/yeban-app/tests/golden/linux/` 全部 5 文件）⇒ sidebar 线**没有移动任何被基准覆盖的像素**，
基准零变更可入库，`MANIFEST.txt` 无需改写。此后只剩常驻判据 run `41253471992` 的判决决定合流。

**人类裁决（2026-11-01，负责人经 AskUserQuestion 亲裁，本票只登记不代签）**：

| 项 | 裁决 | 本线动作 |
| :--- | :--- | :--- |
| **U-13** 三个空壳控件 | **A 维持现状**（= `ADR-0052` 的 `D1`，零代码、零 `.slint`、零基准重录） | ⛔ 不改三控件、不改注册表；三控件继续由 `KNOWN_ACTIONLESS_CONTROLS` 逐名钉死。`ADR-0052` 状态 Proposed→Approved(D1) 由该文档提出方（集成者）翻转，本线不碰其文件 |
| **U-12** 主总线自动化泳道 | **待裁**——放开过滤是产品决策，无裁决不可施工 | 登记为 pending，不实现 |
| **N-UI-2** 循环重复的 UI 表示 | **待裁**——是否要界面上的重复标记属交互设计决策 | 登记为 pending，不实现 |
| **U-09 / U-10** 抽屉行间键盘移动 / 「放弃」键 | **待裁**——焦点模型与键位已写进 `ADR-0058`（状态仍 `Proposed`），等负责人批准 | 登记为 pending，不实现 |

⇒ zimmer 报告面的 UI 项至此**全部收口**：可复现功能缺口全部已落地（§11/§12），U-13 有了人类姿态裁决
（维持现状），其余四项**逐条登记为待人类裁决**——自主可做的 UI 功能切片仍为**零**，且不虚构。

**登记升级（同批）**：`U-12` 与 `N-UI-2` 从"本台账一句话"升格为**可批可驳的正式票**——
`ADR-0059`（主总线自动化泳道：A 维持排除／B 放开可编辑／C 只读显示；现读 `automation.rs` 的
`filter(!= master_bus_track_id)` 与模块注释"判据断言跳过是有意的"）与 `ADR-0060`（循环重复 UI 表示：
A 维持／B ×N 徽标／C 区间分割／D 可编辑循环区；现读 `yeban-model/src/loop_plan.rs` 的
`ClipPlacement::loop_repeat()` 为唯一真相、三处共用），均 `Proposed`、零代码、已入 `docs/adr/README.md` 索引。
U-09/U-10 的正式票即既有 `ADR-0058`；U-13 的正式票即既有 `ADR-0052`（待其提出方按本批裁决翻 `Accepted`）。

## 14. §10「视口外剪辑块可达」的代码事实收盘（2026-11-01）

**这个数字在数什么**：单位＝**个**，对象＝编曲剪辑层注入界面的集合大小对**视口**的依赖。
现读（`main`，符号级）：`bridge.rs` 的 `clip_ulids / clip_labels / clip_positions /
clip_widths / clip_ys / clip_heights` 全部是 `self.clips.iter()` **无过滤臂** —— 与轨道行/音符
那两处的**视口裁剪**不同（那些有 `visible_*_scoped(scroll_x, viewport_width, ..)` 族），
**剪辑层没有**同名裁剪。⇒ 报告 §10 第 4 条"视口外的剪辑块对真实用户是否可达"在**树可达性**
口径上答案是**可达**（N-UI-1 的窗口化不覆盖剪辑块）；"滚轮滚动行为"那一格仍属**未行为验证**，
且编曲行区 `Flickable` 注释自证为**纵向专用**（`content-width: self.width`），横向口径
按第 199 轮教训归宿主 `scroll-x`（⛔ 双重计算）。本条只登记代码事实，⛔ 不据此发明缺陷。

## 15. 判决读回落账（2026-11-01，sidebar 合流前夜）

| 票 | run / headSha | 读数 | 口径 |
| :--- | :--- | :--- | :--- |
| main（混音落地批） | `41253377080` / `4c0d3153` | **success**，真腿全跑：lockfile/plan/checks/deny/macos/**rust (yeban-app)**/windows | 本批代码判决 |
| main（docs 批） | `41256461256` / `f6753cc8` | **success**，但 4 跑 5 跳 —— 纯文档批按计划空绿 | ⛔ 只算门禁绿，代码判决承上一行 |
| sidebar goldens 重录 | `41254412881` | **success** 且 5 张 PNG **sha256 逐张相等** | 基准零变更（§13 已记） |
| sidebar 判据第一批 | `41253471992` / `1da6ddd5` | **failure**：`391 passed; 1 failed` —— 抽屉单注入判据把 sidebar 本地同名声明 `empty-reason:` 误读为第二注入路径 | 根因修复 `d5ceab0d`：判据只数**实例绑定行**（豁免含 `property <` 的声明行；`MusicalPrDrawer` 全仓唯一实例化在 `app.slint`，真注入仍被抓） |
| sidebar 重判 | `41256416978` / `d5ceab0d` | **success**（判决直读 GitHub）；判据门计数 `Running_tests=276`、`sidebar_category=2`、`sidebar_item=2`、`result_FAILED=0` ⇒ 收紧后的单注入判据在 yeban-app 与 windows 两腿全绿；合流 `983080d2` 已推送并远程确认 | 落地批 main run 读回与拆树见 §16（自动落账） |

## 15.1 合流流水线的两处判据耦合缺陷（在飞期间热修，2026-11-01）

流水线 step3 原判据把「工作线全量验证」耦合到**作业名** `rust (workspace`：
本 run 的受影响集合计划把全量腿**跳过**（分支只触 `yeban-app`），目标二进制实际跑在
`rust (yeban-app)` 腿 —— 若不改，一次**真绿**会被空日志的跳过腿误判为 `exit 2` 拒落。
热修：step3 改为拼接**所有** `rust (` 腿日志后再套同一判据（目标二进制在场 ⇒ 仍然可证伪；
⛔ 放宽为"看整体 conclusion"）。step6 原判据取 `--limit 1` 的"最新 main run"，与并发推送
存在竞态；改按**精确 headSha** 定位。教训与 §15 判据误红同族：**任何自动化判据都不许耦合
作业名/最新性这类会被计划裁剪与并发移动的表面坐标，要耦合就耦合内容（二进制名、SHA）。**

## 16. sidebar 合流收盘（自动落账 2026-11-01，数值现读自流水线日志）

- sidebar 重判 run `41256416978` / `d5ceab0d`：**success**（判据收紧后）。
- 合流：`worktree.sh land sidebar-library` ⇒ 本地 main tip `983080d2`（与 §15 文档批同车推送）。
- 落地批 main run `41259186362`：**success**（该 run 含 crates 改动 ⇒ 真腿；空绿不适用）。
- 拆树：`worktree.sh rm sidebar-library --purge`（留档 tag `line-archive/sidebar-library`）。
- 至此 zimmer 报告非 MCP 面**全部收盘**：可复现缺口已落地，U-13=A 维持现状，
  U-09/U-10/U-12/N-UI-2 挂 `ADR-0058/0059/0060` 待人类批准（均 Proposed、零代码）。

## 17. D-UI-4：UI 新建的 MIDI 轨自带一台内置乐器（`ADR-0065`，Proposed，2026-11-01）

**这个数字在数什么**：单位＝**个**，对象＝活窗口里 `instrument-install-*-button` 的实例数。
现读（`--ui-test-mode` 控制面，`ids("instrument-install-")`）= **0**，面板原因串逐字
`INSTRUMENT_INSTALL_NO_BUILTIN_REASON`；根因是清单按**样本字节是否存在**过滤，而仓内
音频样本 **0 个**（`ADR-0055` 同一读数）。⇒ 只用鼠标/键盘的用户建出的 MIDI 轨
`TrackV3::default()` ⇒ `devices` 为空 ⇒ 引擎侧 `DeviceTone::is_source()` 判不出音源 ⇒
`bypass()` ⇒ **画满音符仍然静音**（路由边、电平表、推子全部"看起来在工作"）。

**落点**：`src/bridge.rs`（`BUILTIN_INSTRUMENT_NAME` / `BUILTIN_INSTRUMENT_PARAMS` /
`builtin_instrument_device()` ＋ `track_create_ops` 对 `NewTrackKind::Midi` 追加一条
`Op::InsertDevice{slot_index: 0}`）、`src/host.rs`（手势表同步）。⛔ 不新增控件：
不想要就走**既有** `device-{i}-remove-button`；一次点击仍**恰好一次** `commit_ops`
（同一批 Op 多一条 ⇒ Undo 一步复原）。

**判据**：`tests/track_routing_crud.rs` ①（设备数/种类/非旁通/`latency_samples==0`/含音源键/
九个键名逐条过引擎 `is_consumed_instrument_param`）＋ 新增 ①b（音频轨 `devices` 仍为空）。

**被本票改写的既有假设（如实登记）**：`tests/device_clip_creation.rs` 判据 ⑦
"建轨 → 装 EQ" 之后设备链原为 `[EQ]`，现为 `[内置合成器, EQ]`（`device_insert_ops`
一直是**追加到链尾**，`slot_index = len`）⇒ 断言改为数量 2 ＋ 顺序，⛔ 不是把判据放宽。

**残留风险（不在本票解决）**：默认音色的九个**值**写在 `bridge.rs`，是引擎
`CONSUMED_INSTRUMENT_PARAMS` 之外的第二份表（键名有判据钉住，**值**没有契约位）；
上提到共享契约属账本 `S6`（那份契约的家今天还在 `yeban-mcp`）。

**判决读回**（2026-11-01 现读 `gh run view`）：run `41286061770` / headSha `fd8768f2` /
`conclusion = success`；9 作业里 `deny` `lockfile` `plan` `checks` `macos` `rust (yeban-app)`
`windows` 全部 success，`e2e` 与 `rust (workspace 全量)` 为 **skipped**（本批不触发，⛔ 不计为绿）。
`rust (yeban-app)` 腿的 `test result:` 行 **287** 条（＝测试二进制数）、累计
**2197 passed**、`... FAILED` / `panicked` **0** 条。
⚠ 该腿日志**没有** `Running tests/…` 这种行（计数为 0，见 §6.4 第 4 条要防的正是"判据名没出现"），
因此"目标二进制真的跑了"这判断不靠 `Running` 行，而是逐条点名读回五条判据：
`creating_a_midi_track_commits_once_and_wires_it_to_the_master`（判据 ①）、
`creating_an_audio_track_leaves_the_device_chain_empty`（①b 防回归：音频轨仍空链）、
`the_builtin_devices_take_every_default_from_the_shared_contract`（⑨ 键名对账引擎）、
`inserting_a_reverb_commits_once_and_installs_the_reverb_params`（判据 ⑦ 改写后 `[内置合成器, EQ]`）、
`creating_an_empty_clip_commits_once_and_places_four_bars_at_zero`（片段长度未被本票带偏）——
五条在 `test (合并特性)` 步骤内均逐字读到 `... ok`。

## 18. D-UI-8：UI 控制面装配里**保存入口未接线**（2026-11-01）

**这个数字在数什么**：单位＝**字节 / 个**，对象＝点击 `transport-save-button` 之后
①工程文件在磁盘上的**字节数与轨数**、②控件树里 `status-bar-save-status` 这个**节点是否存在**。
两者都是"点了到底有没有用"的可证伪读数，⛔ 不是"回执字符串好不好看"。

**症状**：第二首（只用人类手势）录到保存环节时，点「保存」与按 Ctrl+S **都不产生任何可见后果**。
比报错更糟的是它**连失败都不报**：`.slint` 里 `status-bar-save-status` 的可见条件是
`visible: root.save-status != ""` ⇒ `save-status` 空串时那个节点**根本不存在**，
而控制面 `ui.value()` 把 `-32006 element-not-found` **咽成 `None`**。于是从外面看
"点了没回复"和"回复是空"长得一模一样 —— 这是**沉默缺陷**，不是错误。

**根因**（读代码定位，非从症状倒推）：`host::wire_save` 是 `save-project` 回调的**唯一登记点**，
而它只被 `run_gui` 调用。`main.rs` 对 `--enable-ui-mcp-http` 的处理是**直接**进
`ui_mcp_serve::run` ⇒ `run_gui` 在该进程里**永不执行** ⇒ 控制面进程用的是
`build_live_ui_with` 这条装配路径，而它从来没调用 `wire_save`。
真实 GUI 会话没事、只有控制面装配的窗口坏 —— 这解释了为什么此前所有判据都没抓到它。

**修复形状**：`live_surface.rs` 的装配末尾补一次 `host::wire_save(&window, host::SaveStatus{
target: options.save_path, project}, options.undo.clone(),
std::rc::Rc::new(yeban_app::file_picker::NoPicker), None)`。⛔ 不新增控件、不新增方法。
`NoPicker` 是既有的"无桌面选择器"实现：控制面进程没有弹框可用，给它一个**返回 `None`**
的选择器正是既有语义。

**判据**：`tests/d_ui_8_serve_save_gesture.rs` 两条。
① `the_control_plane_assembly_registers_the_save_callback`（**默认档**可编译）＝源码文本断言：
`live_surface.rs` 里出现 `host::wire_save(` / `options.undo.clone()` /
`yeban_app::file_picker::NoPicker`，且 `ui_mcp_serve.rs` 里出现 `build_live_ui_with`
（钉住"控制面确实走这条装配"这个前提，否则 ① 会随重构静默失效）。
② `a_real_click_on_the_save_button_lands_the_live_project_on_disk`
（`#[cfg(feature = "ui-mcp-http")]`）＝**真指针点击**：种子容器 → `open_project_document_file`
→ `serve_wiring` → `build_live_ui_with`；先断言开局 `status-bar-save-status` **缺席**、
`get_save_status()` 为空串（把"沉默"写成前置条件），再建一条 MIDI 轨（轨数 +1），点保存按钮，
断言回执以「已保存」开头、`get_save_succeeded()` 为真、节点由缺席变存在、
**磁盘字节变化**且读回容器的轨数 = 点击前 +1、`history_dag` 字节 > 0。

**第一票红与热修**（run `41296475771` / 手动档 `gate=all-features crate=yeban-app`）：
**4 个编译错误**，全部是同一条作用域规则 —— 判据 ② 的两个 `use`
（`yeban_model::container::read_project_container` / `ContainerLimits`）写在 `#[test] fn`
**函数作用域**里，而读回容器的代码在**模块级助手** `on_disk` 中 ⇒ 助手看不见。
⚠ 这条缺陷之所以能躲过默认档 CI：`#[cfg(feature = "ui-mcp-http")]` 的腿**默认档根本不编译**。
热修＝助手内改**全路径限定**，⛔ 不是把 import 提升到模块级（那会让默认档也依赖该 feature 的名字）。

**判决读回**（2026-11-01 现读 `gh run view`）：
- 手动档 run `41297680457` / headSha `9d4023d1`：**success**。11 作业里只有
  `all-features (yeban-app)` 为 success，其余（fps / determinism / bench / inventory /
  goldens / fuzz / windows / arm 对账 / arm 收据 / pending）全部 **skipped** ⇒ ⛔ 不计为绿。
  该腿 `test result: ok` **145** 行、累计 **1175 passed**、`... FAILED` **0**、`panicked` **0**；
  `Running … tests/d_ui_8_serve_save_gesture.rs` 在该腿日志中**在场**（此工作流的日志形状
  与自动档不同：`^[[1m^[[92m     Running^[[0m tests/…` 里的转义是**字面的 `^[[` 两字符**，
  直接 `grep 'Running tests/'` 记 0 是**形状**造成的、不是"没跑到"——计数须用
  `grep -cE 'Running.*tests/.*\.rs'`，现读 **141**）。两条判据逐字读到 `... ok`，回执逐字：
  `[D-UI-8] 真指针点击「保存」：权威 7 → 8 轨；磁盘 8001 → 10668 字节、7 → 8 轨、
  history_dag 1845 字节；回执="已保存 10668 字节 → /tmp/yeban-d-ui-8-save-…/serve.yeban
  （本地原子写）"；控件树里 status-bar-save-status 已由缺失变为存在`。
- 自动档 CI（push `9d4023d1`）run `41297665481`：**success**（9 作业：deny / plan / lockfile /
  checks / `rust (yeban-app)` / windows / macos 全 success，`rust (workspace 全量)` 与 `e2e`
  **skipped** ⇒ ⛔ 不计为绿）。`rust (yeban-app)` 腿现读：`test result: ok` **289** 行、
  累计 **2199 passed**、`... FAILED` **0**；`d_ui_8_serve_save_gesture` 在该腿日志中出现
  **2** 次（二进制名 + 判据行）⇒ 目标二进制真跑了，⛔ 不是"判据名缺失"那种顺序空白。

- ⚠ 修复本体 `1349e571` 自己那票 run `41296326041` 被同 ref 的并发组**取消**（`cancelled`）。
  如实登记：**该票什么都没判**；其内容（fix + 判据）由 `9d4023d1` 这一批（同车 + 判据热修）覆盖。

**残留缺口**（不在本票解决，如实登记）：接线修的是**控制面装配路径**。一个**没有 `--open`
启动的真实 GUI 会话**里，`NoPicker` 返回 `None` ⇒ 「保存」按钮仍是没有目标可写；
另存为/新建后首存的选择器路径属另一切片，⛔ 本片不私自发明文件对话框语义。

## 19. rebase 到 main（51 票之后）的碰撞与分工收盘（2026-11-01）

**这个数字在数什么**：单位＝**个**，对象＝`line/zimmer-ui-write-surface` 的 9 支票在
`git rebase origin/main` 里**真正发生内容冲突**的文件数，以及 main 已经覆盖 / 未覆盖的
本线修复数。来源＝逐支 `git status` 的 `UU` 计数与 `git show origin/main:<file> | grep -c` 的现读。

**为什么要单开一节**：负责人要求"先 pull 到最新再 rebase，让第二首曲子用上新落地的修复"。
main 在这期间**独立落了 `ADR-0057` 选项 A**（测试面才建撤销会话，生产面恒 `None`），它修的就是
`D-UI-1` 的**同一个症状**。于是"我的补丁还剩什么"必须现读，⛔ 不能靠"我写过所以还在"。

**分工现读**（main 侧那份文件逐字核对）：

| 本线的修复 | main 是否已覆盖 | 证据（现读） |
| :--- | :--- | :--- |
| `D-UI-1` 核心：serve 侧不再 `undo: None` | **已覆盖**（同一形状，闸在 `run`、装配是纯函数） | `origin/main` 的 `ui_mcp_serve.rs:155` 起 `let test_undo = if options.ui_test_mode && env YEBAN_ALLOW_UI_INJECT == "1"`，`:189` `undo: test_undo` |
| 服务端端口的**资产池**（`#132` 那条边界） | **未覆盖** | `git show origin/main:.../ui_mcp_serve.rs \| grep -c replace_assets` = **0**（GUI 那条装配在 `main.rs` 一直做，服务端漏） |
| `D-UI-8`：控制面装配里**保存入口**未接线 | **未覆盖** | `git show origin/main:.../live_surface.rs \| grep -c "host::wire_save("` = **0** |
| `D-UI-3`：修饰键 chord 注入 | **未覆盖**（main 无此方法） | `origin/main` 的 `methods.rs` 声明长度行逐字 `MethodSpec; 17`（本线现读 **18**） |
| `D-UI-4`：建 MIDI 轨即挂内置乐器 | **未覆盖** | main 侧 `bridge.rs` 无 `builtin_instrument_device` |

⇒ rebase 后本线剩下的是**真增量**，⛔ 不是重复劳动；`D-UI-1` 那一支从"我自己造一份配方"
改写成"**把 main 那道闸开出来的端口喂进装配**"（`serve_wiring` 变纯装配 + `run` 侧
`replace_assets` 补池），会话配方收敛到 `undo::open_undo_session → session_from_archive`
一条（`main.rs` 与本档调**同一个**入口）。

**改号**（按仓内既有惯例"后落地的一方改号"，先例 `c5e83a46` / `f0499302`）：
`ADR-0061`（chord）→ **`ADR-0064`**；`ADR-0062`（内置乐器）→ **`ADR-0065`**。
main 在这期间把 `0061/0062/0063` 分别给了乒乓延迟／磁带饱和／侧链键输入。
引用面逐条改：`README.md` 表两行、`bridge.rs`、`device_clip_creation.rs`、
`app-zimmer-ui-notes.md` §17 标题、`feature-alignment.md`（5 处）与 `phase-status.md`（1 处）
—— ⚠ 本节的 4 处是**改号收尾时的漏网**：第一批只改了 6 个文件，`grep -rn "ADR-0061" docs/ledger`
现读仍有 5＋1 处才补上。⛔ main 的磁带／乒乓票在 `yeban-render`／`yeban-engine`／`yeban-dsp`
里的 `ADR-0061/0062` 引用一位未动（同名不同票，按语义甄别而不是全局替换）。
两处号位说明写进 ADR 正文（blockquote），裁决内容一字未改。

**融合收尾的一处陈旧读数**：`live_surface.rs` 的 `save_now` 文档注释在本线改名成
`savedFromUndoPort` 而 main 的字段叫 `writtenFromUndoPort` ⇒ rebase 保留 main 的字段名后
注释变成**指向一个不存在的读数**，已就地改正（⛔ 判据只钉字符串，注释里的名字不在射程内，
这正是"注释腐烂"的形状）。

**静态复核**（本机不编译 Slint，§5 纪律 ⇒ 把默认档那两条**读源码**判据的每个断言逐条
`grep -F` 复刻一遍，单位＝**个断言**）：`d_ui_1` 判据 ① 的 11 条断言全过（含 `undo,\n
        proposal_source` 那一支析取），`d_ui_8` 判据 ① 的 4 条全过；
`methods.rs` 的三处机械读数自洽（声明 `MethodSpec; 18` = 条目 18 = 两处 `METHOD_COUNT` 断言 18），
`mutating: true` **11** / `false` **7**，`service.rs:2724` 写死的 `supporting.len() == 11` 仍然成立
⇒ 干跑没有把"计数漂移"带进新 base。`cargo fmt` 以 `rustfmt +stable --edition 2024 --check`
对本票触及的 `.rs` 逐文件跑过（无输出）。

**判决**：`pending` —— 推送后按 §6.3 逐票读回（自动档 `CI` + 手动档
`gates-manifest gate=all-features crate=yeban-app` 的 `ui-mcp-http` 腿），⛔ 不写"通过"。

**备份**：rebase 前的 tip 记在 `backup/pre-rebase-zimmer-ui` = `82cd7166`（⛔ 不删，
新分支若判红要能逐字回到旧形状）。
