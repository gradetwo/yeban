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

## 20. D-UI-9：状态栏的保存回执**人看得见、自动化读不出**（2026-11-01）

**这个数字在数什么**：单位＝**个字符**，对象＝活窗口里 `ui/property("status-bar-save-status", …)`
两种属性名各自读回的长度。现读：`label` → **11** 个字符（`最近一次保存工程的结果`，静态句），
`value` → 修复前为 `null`（该节点没声明绑定状态）。修复后能读回多少字符**尚未实测**
（要等含本票的二进制跑一次活体，或等判据 ③ 的 CI 回执）⇒ 这里⛔ 不写数字。

**症状**（第二首曲子的收尾撞上）：真指针按「保存」后磁盘确实写了（容器 1187 → 510,010 字节，
`mtime` 前进），脚本却判红 —— 因为断言写的是"回执必须以『已保存』开头"，而它读到的是那格
**静态**的 `accessible-label`。这不是脚本笨：是控制面**给不出**人看见的那句话。

**根因**（两条，都不在命中测试也不在权限）：
1. `ui/status_bar.slint` 的回执文本只绑在 `text: root.save-status`，`accessible-label` 是
   恒定的说明句（屏读器念它，⛔ 不该把它当读数）；
2. 控制面的属性清单（`yeban-ui-test-port::inspect::property_of`）支持
   `role/label/id/type/x/y/width/height/opacity/valid/value/checked` —— **没有**"读 `text` 内容"
   这一格；而它下面那层 `ElementHandle`（`i-slint-backend-testing` 1.18.1）逐条枚举
   （`search_api.rs:607..803`）确实**没有** `accessible_name()`，⛔ 不能凭空实现。

⇒ 与 `engine_error_banner.rs` / `sfz_ui_selector.rs` 用的是**同一格既有通道**：把要给人读的话
再声明成 `[ARCH-UI-004]` 的绑定状态 `accessible-value`。修复＝ `status_bar.slint` 一行
（`accessible-value: root.save-status;`）＋为什么写在该行上方。⛔ 不新增方法、不新增属性名、
不动端口与协议 —— 于是 `ui/property` 与 `ui/node` 的 `value` **同时**通（两者同源于
`ElementHandle::accessible_value()`，见 `inspect.rs:186`）。

**判据**：`d_ui_8_serve_save_gesture.rs` 加**判据 ③**（手动档 `all-features`）—— 同一次点击之后，
`ui/property(…, "value")` 与 `ui/node` 的 `node.value` 都必须**逐字等于** Rust 侧的
`window.get_save_status()`；走的是与创作脚本**同一个** JSON 行协议（⛔ 留 Rust 后门 = 判据绿而脚本仍读不出）。
反证：摘掉那行 `accessible-value` ⇒ 读回 `""`，红在 ③ 的第一格。

**创作侧同步**：`compose_ui.py` 的 `stage_save` 从 `label` 改读 `value`，并把"读不到"的报文
分写成两种（① 二进制早于本修复：节点在树里但 `value` 为 `null`；② 保存腿没接：节点整个不在树里），
⛔ 不再把两种原因混成一句"D-UI-8 回来了"。

**判决**：`pending` —— 本票推送后按 §6.3 逐票读回（自动档 + 手动档 `all-features (yeban-app)`）。
创作用的 arm64 二进制必须由**含本票**的构建分支产出（⛔ 拿 rebase 前的 `9db61c4b` 那份跑，
`stage_save` 会红在 ① 那种情形）。

## 21. rebase 之后的**第二处**融合断裂，与 D-UI-9 判据 ③ 的活体读数（2026-11-01）

**这个数字在数什么**：单位＝**个 JSON 数组元素**，对象＝`yeban_render::automation::APPLIED_DEVICE_PARAM_NAMES`
（= 渲染响应里 `data.automation.appliedDeviceParamNames` 的长度）。现读：**78**。
`origin/main` 与本线上那一版判据硬编码的期望值：**70**。差的 8 个是磁带饱和那张票
（`ADR-0062`）带进来的 `tape_drive` / `saturation_drive` / `tape_mix` / `saturation_mix` /
`tape_trim_db` / `saturation_trim_db` / `tape_hf_hz` / `saturation_hf_hz`。

**症状**：手动档 `all-features (yeban-app)`（run `41303198552`，headSha `7f108ea4`）**failure**，
但 §19 那处编译断裂已经消失：作业里 `error[E…]` **0** 格，测试真的跑起来了。唯一一格红——

```
test every_gain_eq_filter_field_gives_the_same_master_in_both_exports ... FAILED
assertion `left == right` failed: MCP 名单必须逐条来自 yeban-render（…）
  left: 78   right: 70
```

**根因（⛔ 不是本票造成）**：`git diff --stat origin/main HEAD -- crates/yeban-app/tests/device_param_automation_export.rs`
= 空（逐字节同 `main`）⇒ 红在 **main 自己身上**：`ADR-0062` 给名册加了 8 个拼写时，同一份名册的
**两个读数点只有一个跟着改了** —— `crates/yeban-mcp/tests/render_audio_clips.rs` 改成 78，
`crates/yeban-app/tests/device_param_automation_export.rs` 留着手抄的 70。本线 rebase 上来才看得见。
`yeban-render/src/automation.rs:39` 的模块注释已经写"78 个拼写 = 43 个字段"⇒ **只有测试在说谎**，
能力面没问题。

**修法**（按判据自己那句话，⛔ 不再手抄第二个数）：断言从"比长度"改成"比内容"——
把响应数组与 `yeban_render::automation::APPLIED_DEVICE_PARAM_NAMES` **逐字同序同集**比对。
这同时把旧写法**看不见**的一类回归纳入射程：长度对、内容换掉。名册今后再加拼写，本判据不用动。
`crates/yeban-mcp/src/domain/render.rs:2020` 的注释仍写"70 个拼写 = 41 通道条（含压缩 14）＋ 6 混响
＋ 8 卷积 ＋ 15 延迟"⇒ **登记，不在本线改**（那是 `yeban-mcp` 的文件，一线一写者；且它是注释不是判据，
⛔ 判据红不红与它无关）。

**判据 ③ 的活体读数**（本票真正要的东西，§20 遗留的那格 `pending`）：
同一份日志里 `Running^[[0m tests/d_ui_8_serve_save_gesture.rs` 之后是
`running 2 tests` ⇒ 两条都 `ok`，`test result: ok. 2 passed; 0 failed`。判据 ③ **不是第三个 test
函数**，而是并进了 ② 那一次真指针点击里（`d_ui_8_serve_save_gesture.rs:241-276`，seq 52/53），
所以它随 ② 一起为绿 ⇒ **D-UI-9 的修复在二进制里生效**：`ui/property(…, "value")` 与
`ui/node` 的 `node.value` 都逐字等于 `window.get_save_status()`。

⚠ **同一批的其余二进制一位没跑**：`cargo test` fail-fast（§6.3 第 4 条），红排在第 **24** 个
`Running tests/`（上一次全绿的手动档 run `41297680457` 读过 **145** 个）。本票的判据 ③
字母序在红之前（`d_ui_8` < `device_param`），所以**它的绿有效**，
但这一票整批评不了"全绿"—— 由 `d3f3250f` 的重跑（run `41305782409`）补。

**判决读回**（逐票，单位＝**个 run 的 conclusion**）：

| 提交 | 自动档 `CI` | 手动档 `all-features (yeban-app)` |
| :--- | :--- | :--- |
| `4c89225b`（D-UI-9） | `41302641177` = **cancelled**（被下一票的 push 并发取消）⇒ 无判决 | 无本票自己的 dispatch；判据 ③ 的绿读在 `7f108ea4` 那份（含本票） |
| `7f108ea4`（融合热修） | `41303177658` = **cancelled**（被 `d3f3250f` 的 push 取消；9 作业，`windows`/`macos` 两格的"失败"是取消传导，不是判据红） | `41303198552` = **failure**：唯一红＝上面那格手抄个数；`error[E…]` 0 格 |
| `d3f3250f`（本票） | `41305769056` = **failure**，但红的只有 `windows`／`macos` 两格（同一时间 `main` 自己也红在这两格）；`rust (yeban-app)` = **success**、`checks`/`deny`/`lockfile`/`plan` 全绿 | `41305782409` = **success** |

**判决**：`d3f3250f` 的**判据腿已读回为绿**（自动档 `rust (yeban-app)` success + 手动档
`all-features (yeban-app)` success）；整票 `conclusion` 仍是 failure，只因 `windows`/`macos`
两格**与 main 同源**的红 —— 那不是本票造成的（下方 §21.1 现读自 `main` 的 run）。
§20 里那句"修复后能读回多少字符尚未实测"由**创作用活体**补：第二首曲子在 `dist6`
（CI 产物 `480b3a8e`，`sha256` 与 `SHA256SUMS` 逐字节相符，`--help` 输出与 CI 的 `help.txt` 全同）
上重跑，`stage_save` 现读 `value` 的字符数写进 §22。

### 21.1 同一格判据在 `main` 上是红的（2026-11-01 现读，单位＝**个名单元素**）

`main` tip `f3621406b` 的自动档 run `41313635970` = **failure**，红的五格里有一格是
`rust (yeban-app)`（本线同名的格子是**绿**的）。日志里的 panic 头两行：

```
thread 'every_gain_eq_filter_field_gives_the_same_master_in_both_exports' panicked at
  crates/yeban-app/tests/device_param_automation_export.rs:942:9:
assertion `left == right` failed: MCP 名单必须逐条来自 yeban-render（70 个拼写：…）
  left: 80    right: 70
```

⇒ 本票（`d3f3250f`）改的正是这一格，且**改法在 `main` 的最新名册上依然成立**：断言比的是
**内容**，不是长度 ⇒ 名册从 70 → 78 → **80**（`tape_asym` / `saturation_asym` 又被 `main`
合进来两个拼写）都不需要动判据。⚠ **这也把 §21 开头那句"现读 78"变成了陈旧值**：
78 是本线 rebase 那一刻 `yeban-render` 的读数，`main` 现在的是 80 ⇒ **名册数只能现读，
⛔ 不许把任何一个数抄进结论**（§6 测量纪律的第 3 条同类）。

顺带一句口径：`main` 红的另外四格 `windows` / `macos` / `rust (yeban-dsp)` / `rust (yeban-engine)`
里，`windows`/`macos` 本线也红（同源），`yeban-dsp`/`yeban-engine` 本线的自动档**根本没跑这两格**
（受影响集合把本线划在它们之外）⇒ ⛔ 不得据此说"main 的问题与本线无关"，只能说
"本线的 `yeban-app` 判据腿绿"。

## 22. D-UI-10：卷帘一次滚动把界面**抹回载入态** —— 现场、根因、修法、判据（2026-11-01）

**这个数字在数什么**：单位＝**次自愈**，对象＝创作脚本 `compose_ui.py` 里
`heal_scene()` 记下的"现场读数与期望不符 ⇒ 按 `Ctrl+Shift+Z` 找回"事件；
现场读数的三元组 = `(轨道行数, 剪辑数, 速度)`，全部现读自 UI 控制面
（`ui/tree` 的 `track-header*` / `clip-header*` 计数 + `transport-tempo` 的读数）。

### 22.1 现场（第二首歌，模拟人类 UI 模式，二进制 = `dist6` / commit `480b3a8e`）

铅笔在卷帘里连按十几下之后，`ui/tree` 的三个计数**同时**回到播种那份空工程：

| 批次 | 自愈次数 | 每一次的现场 | 找回方式 |
| :--- | :--- | :--- | :--- |
| `rerun6` | **3** | 全部 = `(0, 0, '120.00')`（期望 6 轨） | 各 **1** 发 Redo ⇒ `(6, n, '20.00')` |
| `rerun7` | **6** | 全部 = `(0, 0, '120.00')`（期望 6 轨） | 各 **1** 发 Redo ⇒ `(6, n, '20.00')` |

`rerun7` 的六次标签（现读自 `out/ui/rerun7.log`）：`erase roll`、`song:脉冲低音 bar8`、
`song:脉冲低音 bar11`、`song:重音打击#0`、`song:固定音型 bar11`、`song:铜管齐奏#0`。
⚠ `state.json` 的 `scene_heals` 数组里有 **9** 条，前 3 条是 `rerun6` 留下的**同一批**
（`erase roll` / `bar8` / `bar11`）⇒ 数组长度 ⛔ 不等于本轮次数，次数以日志为准（§6 第 5 条）。

三个特征把这条判成**静默重投影**，而不是"编辑丢了"：
① 归零的是**整屏**（轨道 + 剪辑 + 速度三样一起），不是单个音符；
② `app.log` 对此**一个字不写**；
③ **一发** Redo 就把**整段会话**带回来 ⇒ 撤销栈一位没动，工程对象也没动 ——
坏的只有**界面上那份投影**。两次独立批次在同一点（`脉冲低音 bar8`，一次越界拖拽之后）
塌掉 ⇒ **确定性**，不是偶发。

### 22.2 根因（`crates/yeban-app/src/host.rs`，修复前的代码逐行）

`build_main_window_with_console_tab` 里装的 `scroll-requested` handler 长这样（⛔ 摘要有失真，
这是 `git show 5e9dc6a2` 的 `-` 行原文）：

```rust
let scroll = std::rc::Rc::new(std::cell::RefCell::new(0.0_f32));
…
    let view_snapshot = std::rc::Rc::clone(&view_snapshot);   // ← 装配那一刻的投影
    ui.on_scroll_requested(move |delta| {
        let next = { let mut current = scroll.borrow_mut(); *current = advance_scroll(*current, delta); *current };
        apply_view(&ui, &view_snapshot, ui.window().size().width as f32, next);
    });
```

两处耦合起来才致命：

1. **快照冻结**：闭包抓的是 `Rc<ViewState>`，此后**没有任何路径**更新它 ⇒ 每次滚动都
   把那份**旧投影**整份 `apply_view` 回去（bpm、轨道、剪辑、音符一起回写）。
2. **偏移双份所有权**：宿主在 `Rc<RefCell<f32>>` 里自己存了一份偏移，而 `apply_view`
   写的是界面属性 `roll-scroll-x`（`host.rs:314` 的 `set_roll_scroll_x`）⇒ 两份会互相盖，
   重新接线时还要手工播种。

产品路径上 `wire_roll_edit` 随后会**重装** handler，抓的是"接线那一刻"的工程 ——
而创作时那一刻工程**还是空的**（先装配、后播种/画音符），于是症状正是
"画了几个音符之后随手一拖，界面回到载入态"。触发门槛只有**拖拽阈值 4px**，
所以"随手"两个字是字面意义的。

### 22.3 修法（同一提交 `5e9dc6a2`）

新增唯一的派发器 `install_roll_scroll(ui, fallback_view, port)`（`host.rs:11996`），两条腿：

```rust
let next = advance_scroll(ui.get_roll_scroll_x(), delta);      // 偏移唯一事实源 = 界面属性
match port.as_ref().and_then(|p| p.try_project())
          .and_then(|project| project_with_view_state(&project, &ui).ok()) {
    Some(refreshed) => apply_view(&ui, &refreshed, width, next),  // 端口在场 ⇒ 用**此刻**的工程
    None            => apply_view(&ui, &fallback_view, width, next),
}
```

- ⛔ 不再存宿主侧偏移镜像 ⇒ 顺带消掉"双重计算"这一类回归的土壤；
- 投影**每次重新取自端口**（与 `reproject_roll` 同一条链），端口缺席/无活跃工程才如实退回
  快照（⛔ 不在 Slint 回调里 panic，与 `install_roll_click` 的 `fallback_view` 同一语义）；
- **两个装配点都装**：`build_main_window_with_console_tab`（`host.rs:12273`，无端口）与
  `wire_roll_edit`（`host.rs:10865`，带端口）⇒ 产品进程与无头面不可能只接一半。

### 22.4 判据（`crates/yeban-app/tests/roll_edit_tools.rs:4363`）

`a_scroll_drag_after_a_pencil_edit_redraws_the_live_project_not_the_wired_snapshot`
—— 走**生产装配顺序**（`assemble()` 里就调 `wire_roll_edit`，⛔ 不是测试自己拼的半边）：

1. 铅笔点一下 ⇒ 工程 `note_count` +1，且 `note-{ULID}-rect` **在树里**（拖动前先确认，
   免得视口裁剪把"本来就看不见"误读成"被抹掉了"）；候选 x 只取 `60 ≤ x ≤ grid_width − 60`
    ⇒ 保证那 60px 的拖动不会把音符推出可视区；
2. 注入 `down → move×3 → up` 水平拖 60px，断言 `|Δroll-scroll-x| > 0.5`
   —— 这一条是**前置**：不成立就说明本判据根本没驱动 `scroll-requested`，后面的断言都是空的；
3. **核心**：那次滚动之后，刚画下的音符 `{ULID}-rect` **必须仍在树里**；
4. 滚动手势 ⛔ 不改工程：`note_count` 与拖动前**逐只相同**。

**怎么变红**：把 handler 换回"用冻结快照 `apply_view`"，第 3 步立刻红（音符整批消失），
第 4 步仍绿 —— 正是本缺陷的签名：**工程没错，投影错了**。

### 22.5 顺带量到的两件事（都进创作者报告）

1. **保存回执可读（D-UI-9 的活体读数，§20/§21 遗留的 `pending` 到此收盘）**：
   `ui.value("status-bar-save-status")` 现读
   `已保存 215740 字节 → /tmp/zimmer_creative/out/ui/zimmer_ui.yeban（本地原子写）`
   = **66 个字符**（单位＝**个 Unicode 码点**，Python `len()`），磁盘同一次
   **1187 → 215,740 字节**、`mtime` 递增 ⇒ 回执里的字节数与磁盘**逐字节对齐**，
   ⛔ 不是"写的是装配快照"。
2. **混音台推子的度量口径**（本机现读，⛔ 规范没写）：`track-N-fader` 的包围盒是
   **22×16 px**；一次拖动的位移换算 = **0.75 dB / 像素**（现测 `6.0 → 3.0`（4px）
   `→ −3.0`（8px）`→ −18.0`（20px）`→ −12.0`（−8px）），`dy > 0`（向下）= **变轻**；
   推子是**相对**手势 ⛔ 不是绝对定位 ⇒ **手柄的视觉位置与 dB 读数不对应**。
   后果落在创作上：`stage_mixer` 第一版按"从底部上拖 `height × 比例`"设电平，
   六条推子**全部顶到 +6 dB 上限**（读数 `6.0/6.0/6.0/5.8/5.3/6.0`）＋ 总线也 `6.0 dB`
   ⇒ 导出 WAV `peak = 0.0 dBFS`、集成 `+0.09 LUFS`（削波）。
   **根因分两半**：harness 的错（该闭环读 dB 而不是猜像素）＋ 界面的缺
   （⛔ 无数值输入、⛔ 无削波/安全余量提示、22×16px 上 0.75 dB/px 根本点不准）。
   修复后的 `stage_mixer` = 读当前 dB → `dy = (have − want) / 0.75` → 拖 → 再读 → 修正（≤4 轮），
   并把总线目标设为 **0 dB**。

### 22.6 第二首歌 `rerun7` 的创作读数（供报告与对照）

| 指标 | 单位 | 现读 |
| :--- | :--- | :--- |
| 音符总数（工程侧） | 只 | **172** |
| 音符逐只回读比对 | 次 | 172，⛔失配 **0** |
| 时长改写命中 | 次 | `dur_presses = 0`、`duration_changed = 0` ⇒ **本轮没走成时值臂**（见下） |
| 工具切换回退点击 | 次 | 31（`tool_click_fallback`） |
| 盲拖（越出可读区） | 次 | 4（`blind`） |
| 音高折半（超界钳制） | 只 | 122 |
| 保存回执 | 字符 | 66（`已保存 215740 字节 …`） |
| 导出 WAV 墙钟 | ms | **2,516**（44.1 kHz / 24 bit / 180.0 s，47,628,068 字节） |
| 场景自愈 | 次 | 6 |

⚠ `dur_presses = 0` 与 §22.5 的第 2 条同属"人类面太窄"这一族：时值只有 `Shift+←/→`
一条写入路径（一档 = 一个吸附网格），而本批次**根本没触发它** ⇒ 登记为待查，
⛔ 不得写成"时值功能正常"。

**判决**：`5e9dc6a2` 自动档 `41314892958`、手动档 `41314963972` **在飞** ⇒ 本条记 `pending`；
读回时必须确认 `roll_edit_tools` 二进制出现在 `Running tests/` 名单里且有 `test result:` 行
（§6.3 第 4 条：fail-fast 之下"名字缺失"既不是失败也不是通过）。

## 23. D-UI-11 与 D-UI-13 的收盘（判决现读，2026-11-02）

两票都在合流前的 `line/zimmer-ui-write-surface` 上，本分支查不到 per-commit run ⇒ 用
`gh run list --branch line/zimmer-ui-write-surface` 现读（单位＝**个 run 的结论**）：

| 提交 | 内容 | 本提交自己的 run | 结论 |
| :--- | :--- | :--- | :--- |
| `708ed3f4` | serve 那份装配漏接选轨 / 行高拖拽 / 撤销三条 | 无（同批推送，tip 才有 run） | 由 `05c54275` 的 run 覆盖 |
| `8ca460ba` | 卷帘命中测试不分作用域 | `41369170925` | **failure**（修复前的红形态，下一枚热修接手） |
| `6809d13e` / `a95851e2` | 同票的 `collapsible_if` / `for_kv_map` 两处 clippy 红 | `41369885862` / `41370538851` | **failure** ⇒ `a95851e2` 的手动档 `41371903917` **success** |
| `9157dcb2` | D-UI-11 判据 ⑤ 改为本行内拖拽 | `41363718543` | **failure**（同上，红在判据前提） |
| `05c54275` | 批尖（D-UI-1 判据读盘归一行尾） | `41393391415` | **success** ⇒ 整批（含两票）到此有判决 |

⚠ 中间那些红**不是**"最后一条绿了所以前面不用看"：`collapsible_if` 与 `for_kv_map` 都是在
`-D warnings` 下直接判红的编译期错误，写代码时就该现读 `clippy` 输出而不是等 CI（本节第 4 行
两枚热修 = 两次往返 ≈ 26 分钟墙钟）。

## 24. D-EX-1 的四次读回与"空绿"陷阱（2026-11-02）

| run | 触发 | 时长 | 结论 | 这句话能说 / 不能说 |
| :--- | :--- | :--- | :--- | :--- |
| `41436916743` | push `df64c9da` | — | **failure** | 红在 `deny` 一条腿（RUSTSEC-2026-0433 / pulldown-cmark 0.13.5）⇒ 与本片代码无关，但⛔ 不能当"已过" |
| `41446828680` | push `607fd495`（升 0.13.6） | — | **failure** | 红在 `checks` 的**依赖许可清单漂移**一条 ⇒ 升版本必须同批重生成清单 |
| `41460622795` | push `343461a4`（重生成清单） | **1m12s** | success | ⚠ **空绿**（§6.3 第 2 条）：纯 `docs/**` 改动 ⇒ plan 返回空集合，rust 腿全 `-`/`0s`，不构成任何验证 |
| `41461978083` | 手动档（workflow_dispatch，树 = `343461a4`） | **48m43s** | **success** | D-EX-1 的代码在这一棵树里被真跑过 ⇒ 本票判决以这一条为准 |

读回时另按 §6.3 第 4 条核过：`gui_export_limiter` 这个名字确实出现在该 run 的
`Running tests/` 名单里（⚠ 直接 `grep -c "Running tests/"` 会因 ANSI 转义把计数打成 **0**，
匹配二进制名才拿到真读数）。

## 25. D-UI-14：左栏搜索框此前是**假的**（`83d151c3`，判决 pending）

现场：`ui/sidebar.slint` 那一格挂着 `accessible-role: text-input` 与语义 ID
`sidebar-search-field`，可寻址、可被屏读念出，但它是**一个占位矩形** ⇒ 敲进去什么都不动。
修法与判据逐条写在提交正文（两向绑定、过滤只在 Rust 一处、空态分两档、钳位分母 = 过滤后的
条数、⑨ 文本层取证）。⛔ 不换 `placeholder-text`：那会让默认帧像素搬家 ⇒ 5 张 Linux 基准整批重录。

判决：`83d151c3` 自己**没有** per-commit run（与 `5d80c42c` 同批推送，只有批尖有 run，L32）；
批尖 `5d80c42c` 的 `41472437095` = **failure** ⇒ 本票判据从未被执行（见 §26）⇒ 记 `pending`，
以 `f0a57180` 的 run `41476987648` 为准。

## 26. D-UI-15 的第一票红：一个**签名生命周期**（`5d80c42c` → 热修 `f0a57180`）

现场：`41472437095` 三条 `yeban-app` 腿（`rust` / `windows` / `macos`）的
`clippy (-D warnings)` 全挂，报错一条：

```
error[E0106]: missing lifetime specifier
   --> crates/yeban-app/src/host.rs:11896:34
    = help: this function's return type contains a borrowed value, but the signature
            does not say whether it is borrowed from `project` or `placement_ulid`
```

根因：本片把 `clip_placement_for` 从"只回一个 `start_tick` 标量"改成**回借用**
（`Option<(EntityId, EntityId, &ClipPlacement)>`，让拖动与删除共用同一个身份解析点），
但签名里有两个输入生命周期 ⇒ 返回值的生命周期无法省略。`41436916743` 那一族是环境红，
这一条是**我写的代码**红。

⚠ 代价被低估的那一半：错误在 **lib** 编译期 ⇒ 同一批里三个判据文件（`sidebar_search.rs`、
`delete_selected_clip.rs`、以及本票之后的 `clip_mute_toggle.rs`）**从未被编译，更未执行**。
所以这一票既不是"判据失败"也不是"判据通过"，是**零读数**（§6.3 第 4 条的形状）。

教训登记：把函数改成回借用时，⛔ 不要假设"省略式能推出来"——只有**一个**输入生命周期时才推得出。
本机不编译 Slint ⇒ 这一类形状只能在 CI 撞上，所以每票落地前先做一次
"这个签名有几个输入生命周期、返回值借的是哪一个"的自检。

## 27. D-UI-16：剪辑能选、能拖、能删，就是不能单独静音（`e48bf59c`，判决 pending）

缺口证据（现读命令，单位＝**个匹配**）：切片之前 `grep -rn SetClipPlacementMuted crates/yeban-app/src`
= **空**，而同一条 `Op` 在 `yeban-model/src/ops.rs` 有 apply / inverse / 前置条件，
在 `yeban-engine/src/snapshot.rs`、`yeban-render/src/{als,logic}.rs`、
`yeban-mcp/src/domain/notes.rs` 都是**真读数** ⇒ 引擎与 MCP 早已诚实，只有界面那一半没接。

切片形状（与 `D-UI-15` 同一族口径：⛔ 不加新按钮 ⇒ 不动元素注册表、⛔ 默认帧逐像素不变）：
键 `Cmd/Ctrl+M`（实测该格此前是第 3 步的 `_ => PassThrough`）＋ `host::clip_mute_ops_for`
（全 app 唯一构造点）＋ `apply_view` 注入 `clip-mutes` ＋ 包头底色
`bg-raised(#1e2733) → bg-void(#0e1216)` ＋ 读屏标签后缀"（静音）"。
与轨静音的分工是**作用域**：裸 `m` 动 `selected-track-id`，本键动 `selected-clip-ulid`。

机械副作用现读补账（⛔ 不等 CI 报红再改）：`APPLY_VIEW_WRITE_SITE_BUDGET` **100 → 101**、
`PER_FRAME_ARRAY_REWRITE_BUDGET` **57 → 58**、快捷键表 `SHORTCUTS` **46 → 47**
（判据 B11b 同时改条目数与那句分解文案）。

判决：本票与 §25 / §26 同批，`f0a57180` 的 run `41476987648` **在飞** ⇒ 三票全部记 `pending`；
读回时必须逐条确认 `sidebar_search` / `delete_selected_clip` / `clip_mute_toggle` 三个二进制
都出现在 `Running tests/` 名单里且各有 `test result:` 行。

## 28. `41476987648` 读回：第二枚**我写的代码**红（`clippy::clone_on_copy`），仍是零读数

读法（run 尚未整体结束 ⇒ `gh run view --log-failed` 直接拒绝："logs will be available when it is
complete"）⇒ 改走 **per-job API**：`gh api --allow-escape-sequences repos/gradetwo/yeban/actions/
jobs/<job-id>/logs`，先 `sed 's/\x1b\[[0-9;]*[a-zA-Z]//g'` 去 ANSI 再 `grep`。⚠ 这条"整体在飞时
拿不到日志"是本轮新学：判在飞 run 的单票结论要用 `gh run view --json jobs` 的**作业级**
`conclusion`，日志只能等 run 结束后取（或按作业 id 取）。

作业级现读（`f0a57180`）：`lockfile` / `checks` / `plan` / `deny` = success；
**`rust (yeban-app)` = failure**、**`windows (...)` = failure**；`macos` 排队；
`rust (workspace 全量)` / `e2e` 被 plan 跳过（⛔ 那两格的 success/skipped 不构成任何验证，§6.3 第 2 条）。

失败内容是**一条** clippy 硬错，linux 与 windows 同源（非平台抖动）：

```
error: using `clone` on type `ClipPlacement` which implements the `Copy` trait
   --> crates/yeban-app/src/host.rs:11950:29
11950 |         previous_placement: placement.clone(),
```

根因是 §26 那枚修复的**副作用**：`clip_placement_for` 改成回 `&'p ClipPlacement` 之后，
`clip_delete_ops_for` 里那句 `placement.clone()` 的对象从"非 Copy 的引用目标"变成了
"**Copy 类型的 clone**" ⇒ `clippy::all` 在 `-D warnings` 下判硬错。改法 = `*placement`
（`ClipPlacement`  derive `Copy`，语义还是一份快照，撤销载荷一位不少）。

⚠ 与 §26 同一半代价，且这次更贵：错误仍在 **lib** 编译期 ⇒ `sidebar_search` /
`delete_selected_clip` / `clip_mute_toggle` 三个判据二进制**第二次**从未被编译。
两连红的共同形状是"**改签名/改类型后没有重读调用点**"——本机不编译 Slint 时，这类
错误的唯一防线是把"这一改动会让哪些调用点的类型变掉"逐条列出来看一遍，而不是等 CI。

本轮顺手做了该自检（覆盖 §26 / §28 两处改动波及的全部调用点）：
`clip_placement_for` 的六个使用点里只有 11950 一处用到整份摆放，其余只读标量字段；
三个新判据文件则逐符号核过 `#[test]` 之外的**未使用**导入 / 助手 / 绑定（0 处），
并逐一现读它们依赖的 API 形状：`ActionOutcome::changed()`（`undo.rs:221`）、
`UndoDisplay::commit_count`（`yeban-mcp/src/undo_session.rs:343`）、
`Rgb8Image::pixel(u32,u32)->Option<Rgb>` / `pixels()` / `Rect::new(i32,i32,u32,u32)`（`image.rs`）、
`LiveWiringOptions` 有 `impl Default`（`live_surface.rs:196`）且 `undo: Some(..)` 那一路会
真的接 `wire_clip_scope` / `wire_clip_drag` / `wire_track_select`、
`.slint` 侧 `key-action(string,bool,bool,bool,bool)->bool` / `clip-select(string)` /
`track-select(int)` / `in property <[bool]> clip-mutes` / `clip-{ulid}-header` 全部在场。

## §29 run `41479416456`（tip `ff0be316`）读回 —— **第三次零读数**，根因移进判据文件

逐作业结论（`gh run view 41479416456 --json conclusion,headSha,jobs`，`headSha` 逐位 = `ff0be316`）：

| 作业 | 结论 |
|---|---|
| `lockfile` / `plan` / `checks` / `deny` | success |
| `rust (yeban-app)` | **failure** |
| `windows (…)` | **failure** |
| `macos (…)` | **failure**（这一腿这次真跑了，不再是从未开始） |
| `rust (workspace 全量)` / `e2e` | skipped ⇒ 空绿（§6.3-2） |

在飞的 run 没有 `--log-failed`，取法同 §28：`gh run view --json jobs` 拿三个 failure 的
`databaseId`（`125393605895` / `…5945` / `…6031`）⇒ `gh api --allow-escape-sequences
repos/gradetwo/yeban/actions/jobs/<id>/logs` ⇒ 剥 ANSI ⇒ `/tmp/log-41479416456-app.log`。

**判决内容（单位：`error` 行数 = 2，二者同在一个二进制里）**：`lib` 这次编译过了
（日志里 `Compiling yeban-app v0.0.1` 之后没有 lib 侧错误），红的是
`crates/yeban-app (test "clip_mute_toggle")`：

1. `unused_parens` @ `tests/clip_mute_toggle.rs:346` —— `.map(|v| (if *v { '1' } else { '0' }))`，
   `-D warnings` 下是硬错；改法 = 去掉那对括号。
2. `E0716` temporary dropped while borrowed @ 同文件 `301` ——
   `let muted_after: Vec<&String> = placements(&project_after).iter()….collect();`
   `placements()` 回的是**拥有所有权的 `Vec<(String, ClipPlacement)>`**，`.iter()` 借的是那个
   临时值，它在本语句结束即释放而借用活到下一句 `assert_eq!`。改法 = 先把 `placements(..)`
   绑成一个局部量再迭代。

后果（这是本节的重点，不是第 1 条那种小事）：`grep -c "Running tests/" = 0` ⇒ 编译期失败，
测试阶段**整体没有开始**，D-UI-14 / 15 / 16 三个判据**第三次**零读数，判决仍是 `pending`。

⚠ §28 末尾那句自检结论是**假的**："三个新判据文件则逐符号核过……（0 处）"。它核的是
"未使用的导入 / 助手 / 绑定"，而真正致命的是**表达式的生命周期**与 **`clippy::all` 的形状类
lint**（这两类都不在"有没有用到"这个清单里）。教训要这样记：**"我检查过 X" 不等于"我检查过
判据文件"**，X 是自检清单里那一小截，写进账本时必须把 X 的名字写出来，否则下一次会拿着
半张清单当全绿。

本轮改的流程（本机跑不了 `cargo clippy -p yeban-app` —— Slint 依赖，AGENTS.md §5.2；CI 是
唯一编译器）：推送前对**每一个从未编译过的判据文件**做一次并行子代理静态审计，口径固定四项 ——
①每个被调符号对照**真实定义**核名字/元数/参数与返回类型/是否 `pub`（跨 `tests/*.rs` 的私有
助手不可达：每个集成测试文件是自己的 crate）；②`use` 与助手是否全部被用（`unused_imports` /
`dead_code` 都是硬错）；③`clippy::all` 清单逐条过（工作区 `Cargo.toml:115-118` 实测
`all = "deny"`，`float_cmp` 之类 `restriction` 不在其中、cast 类是 `pedantic` 也不在其中）；
④哑断言（同一份读数自己比自己、`assert_eq!(X, X)`、缺按键前快照的"其余逐值不变"）。

这套审计在**推送之前**抓到 7 处会被 CI 判红的真错，全部已改：

| 文件:位置 | 类 | 改法 |
|---|---|---|
| `clip_duration.rs:428-437` | 编译错 E0369 | `Option<f32>` 与 `f32` 比较（`.unwrap_or_default()` 收尾）⇒ 整段换成前后两列按下标逐值对账 |
| `clip_duration.rs:434` / `:594-598` / `:533-539` / `:846-852` | `clippy::eq_op` ＋ 零信息 | 四处"自己比自己"的占位断言删除，改为按键前快照（`widths_before` / `duration_before`）对账 |
| `clip_duration.rs:395-417` | 判据自身假阳性 | 过滤器写成"时长 ≠ A 的**原值**"，把本来就不等于 A 的 B（夹具 960 ≠ 3840）也捞进"改动"⇒ 改成**同一行前后**对比（`changed_rows == vec![a_row]`） |
| `clip_duration.rs:691-693` | 编译错（格式串） | `"…（实测 {:?"` 占位符没有闭合；改为先绑 `duration_now` 再写 `{duration_now:?}` |
| `clip_duration.rs:199` / 两个文件的 `UiAction` 导入 | `dead_code` / `unused_imports` | 删除已无人调用的 `width_at` 助手与随之失效的导入 |
| `delete_selected_clip.rs:522-545`、`clip_mute_toggle.rs:598-622` | **必红**的运行时前提 | 见 §29.1 |
| `sidebar_search.rs:287-291` | 哑断言 | 原来敲的是夹具里**没绑任何 Action** 的字母 ⇒ 焦点走没走 `active-tool` 都不动；改为敲数字 `3`（工具矩阵第 3 行）两个方向都可证，敲完立刻退格以保住后面每一臂的过滤前提 |

### §29.1 一条值得单列的界面事实：`UndoPort::perform` **不**重投影

`crates/yeban-app/src/undo.rs` 里端口自己写着"端口碰不到 Slint"（`UiAction::SelectTool`
那一臂的注释是同一处证据）。因此
`port.perform(UiAction::Undo)` 之后读 `window.get_clip_ulids()` / `clip-mutes` 读到的是
**撤销之前那一帧**的界面属性 —— 三个判据文件原先都在这里断言"画面跟得上权威"，那是
**断言了一个不存在的行为**，下一次真跑必然红（不是空转，是假红里最坏的一种：实现没错，判据错）。

唯一能让界面跟着撤销的入口是**键路径**：`invoke_key_action(..)` → `on_key_action` →
`apply_action` 的 `Action::Undo | Action::Redo` 那一臂（`host.rs:9446-9455`）→ 末尾
`refresh_undo_window(ui, port, true)`（`reproject = matches!(action, Undo | Redo)`）→
`project_with_view_state` + `apply_view`。走带区的 ↻ 与弹窗的"撤销一步"两个按钮回调
（`host.rs:10081` / `10124`）也是"先 `port.perform` 再手动 `refresh_undo(.., true)`"，
即**重投影是调用方的责任，不是端口的**。三个判据的撤销臂已统一改为键路径，并顺手把
**redo 之后画面也跟上**这一条从"没断言"补成断言（`clip_duration` ⑦ / `delete_selected_clip` ⑥ /
`clip_mute_toggle` ⑦）。这条纪律将来适用于任何"撤销/重做后读界面"的判据。

## §30 D-EX-2 登记：**导出那 2.5 秒 UI 线程不处理任何事件**（只登记，本切片一行代码没改）

> 来源：创作轮 §22.6 的那一行「导出 WAV 墙钟 = 2,516 ms」；本票只补"它为什么是缺陷"与"缺哪两个裁决"。

**结构现读（三处证据，全部 `crates/yeban-app/src/host.rs`）**：

| 位置 | 事实 |
| :--- | :--- |
| `7925` | `ui.on_export_wav(move \|\| { … })` —— 导出弹窗「导出」键的宿主回调 |
| `7990` | 回调体内**同步**调用 `crate::export_wav::export_wav_with_sfz_file(…)`，拿返回值直接拼回执 |
| `7930` / `8067` | `Instant::now()` 与 `module_trace("export-wav", started, …)` ⇒ 计时点就在这条同步调用的两侧 |

**指标（数什么：一次导出从按下到回执写出的墙钟，单位 ms）**：§22.6 现读 **2,516 ms**
（44.1 kHz / 24 bit / 180.0 s 音频 / 47,628,068 字节）。这段时间 `on_export_wav` 不返回 ⇒
Slint 事件循环这一路停摆：期间没有任何点击、按键、重投影能进来。

**对照：本仓库已经有两处「长活不占 UI 线程」的成例**，导出是**第三处长活却没用这个模式**：

| 长活 | 中间态 | 工作线程 |
| :--- | :--- | :--- |
| 乐器采样体检 `host.rs:3241` → `3248` | 在 `spawn` **之前**同步写「检查中…：<名字>」 | `std::thread::spawn` |
| MIDI 采集 `host.rs:8421` → `8450` | 同上纪律 | `std::thread::spawn` |
| **导出 WAV `host.rs:7990`** | ⛔ 无（回执只在渲染完之后出现） | ⛔ 无（UI 线程自己渲） |

**未证的部分（登记，⛔ 不补齐，遵 §6.4-4）**：我没有测「停摆期间的点击是否被丢弃还是排队到
渲染完再处理」，也没有测「导出期间**声音**是否照常」——后者取决于渲染路径有没有与音频线程
争同一把工程锁，我没有读回来。因此本票**不**写成"导出会丢点击 / 导出会把播放打断"。

**要人类裁决才动得的两件事**（擅自选边就是私自发明答案）：

1. **进度反馈的形态**：「导出中…」是加一个界面属性 + 弹窗按钮禁用态，还是复用 `module-status`
   那条既有读数？前一种会改**默认帧像素** ⇒ `tests/golden/**` 整批重录（同一族代价见
   `ui/sidebar.slint` 里搜索框那段注释），需要人类批准重录。
2. **渲在哪、怎么汇合**：沿用上面那两条成例的「spawn + UI 线程排空」，还是接进已有的 worker
   槽抽象（`host.rs:2693` / `8542` 那两个 `JoinHandle` 槽位）；并且**同一目标文件并发两次导出**
   允不允许（现有纪律里没有这一条的答案）。

**处置**：`pending`，等裁决。创作轮里它的实际代价是"点导出＝冻 2.5 秒、期间界面看起来死了"，
属于**可用性缺陷但不是错误结果**——回执与文件本身是对的，所以不阻塞 D-EX-1 那一批。

## §31 死控件普查的结果：**没有新的死键**，但**文本框全都摸不到键盘**

普查口径（一次只读扫描，`.slint` 侧与 `src/` 侧**双向**核验才算）：
(A) 声明了按钮外观却没有点击源；(B) 点击源调用的回调走到 `app.slint` 就断了；
(C) 渲染字面量却冒充实时读数。

**(A) 三处，全部是**已登记**的那三条** —— `src/elements.rs:4551` 的
`KNOWN_ACTIONLESS_CONTROLS [&str;3]` 逐条对得上：`ai-rail-intent-button`
（`app.slint:119`）、`transport-commit-button`（`transport.slint:378`，有
`commit_area := TouchArea` 却**空体**）、`transport-branch-button`（`transport.slint:347`，
`elements.rs:3603` 原话"今天不得有点击源"）。⇒ **没有遗漏的死键**，这三条继续等人类裁决
（见本文件前面的 pending 清单）。

**(B) 空。** 每个被 `clicked` / `accessible-action-default` 触发的回调，要么在 `app.slint`
转发到一个 `src/` 里有 `on_` 处理器的 `MainWindow` 回调，要么落到宿主会读的视图态属性上。

**(C) 一处，但它不是假控件**：`ui/status_bar.slint:131` 的 `chord: "(none)"` 在
`src/elements.rs:379` 登记为**动态区**（第五位 `true`），且 `status_bar.slint:127-129` 自己写着
"`chord` 则永远停在无读数形态（**能力不存在**）" ⇒ 与 `device_rack.slint` 那两处
"(占位)" / "[v2.0.0 · 未实现]" 同族：如实报"没有读数"。⚠ 若要补这一格，
`yeban-theory::chord::Chord::from_voicing`（`crates/yeban-theory/src/chord.rs:571`）就是现成的
反向识别（音集 → 符号），⛔ 不必新造算法；但 `yeban-app` 现在**不依赖** `yeban-theory`
（`Cargo.toml` 只有 `yeban-model`），且"哪些音符算进来"（播放头？未静音轨？选区？）
规范里没有答案 ⇒ 那是**要 ADR 的语义决定**，不是接线。本票不动它。

**⭐ 真正的缺口在这里：文本框只能靠鼠标点。** 现读链条：

1. `ui/app.slint:725` `forward-focus: key-handler` ⇒ 窗口建立时焦点在快捷键层；
2. 焦点在画布时 `Tab` = `Action::ToggleView`（`src/input.rs:1418` 的既有判据逐字如此，
   规范 §2.2 也把 `Tab` 限定在画布）；
3. `key-pressed` 只有 in `key-action` 返回 `false` 时才 `reject`（`app.slint:728-733`），
   而注释逐字写着 **`accept` ⇒ "Slint 也不做 `Tab` 焦点轮转"**（`app.slint:716`）。

⇒ 焦点一旦落在快捷键层，`Tab` 被 DAW 吃掉、永不轮转 ⇒ **`bpm-input`（`transport.slint:281`）、
搜索框（`sidebar.slint` 的 `sidebar-search-input`）、导出弹窗那 4 个框，纯键盘用户一个都进不去**。
仓库里唯一的程序性回焦成例是时光机那一对（`app.slint:2237` `undo_tree_trap.focus()` /
`2240` `key-handler.focus()`）⇒ **有办法补**（键位 → 转发 → `.focus()`），
D-UI-18 就是把搜索框接进键盘可达面。

## §32 D-UI-19 导出弹窗的键盘捕获（模态第一次真的持有键盘）＋ 两条只登记的缺口

**要补的句子**：`[UI-A11Y-003]` §7.3 第 1 条 —— 模态面板内焦点必须闭环，`Tab` 不得逃出，
`Esc` 必须收尾。§31 那条链在弹窗这一侧的表现是：那四格（路径 / 采样率 / 位深 / 秒数）
**只能靠鼠标点**，而键盘整个停在 `key-handler` 上 —— 弹窗盖住整帧却一个键也接不到。

**交付**（`ui/dialogs/export_wav_dialog.slint` · `ui/app.slint` · `src/host.rs` ·
判据 `tests/export_dialog_key_capture.rs`；`tests/export_dialog_modal_closure.rs` 只改说明）：

1. **捕获点在祖先，不在兄弟**：`export-trap := FocusScope` 包住整块面板（不是时光机那种
   兄弟覆盖层）。理由实测自 `i-slint-core-1.18.1/window.rs:1324-1348`：键事件从焦点件
   **只往上冒给祖先** ⇒ 兄弟 `FocusScope` 永远接不到 `TextInput` 拒绝的键。
2. **`Tab` / `Shift+Tab` 由模态自己轮转**：`focus-on-tab-navigation: false` 是**必需**的 ——
   内建那段（`window.rs:1350-1370`）只在"没人 accept"时跑，且按**可见**筛，而此刻左栏搜索框
   与 BPM 框同样可见 ⇒ 焦点会跳出模态。`Ctrl/Alt/Meta + Tab` 一律 `reject`，与内建同口径。
   `Shift+Tab` 在注入侧写作 `Character('\u{19}')`（`KeyCode` **没有** Backtab 变体）。
3. **`Esc` 落既有那条回调**：`root.close()` ⇒ 宿主 `on_close_export_wav`，⛔ 不是第二份关闭逻辑
   （`export_dialog_modal_closure.rs` ⑥ 钉的就是"这个形状只有一处"）。
4. **进入/退出的焦点转移写在宿主**：打开站点在同一个闭包里调 `invoke_focus_export_wav_dialog()`；
   关闭站点**只在 `was_open` 时**调 `invoke_focus_shortcut_layer()` —— 否则"本来就没开"的
   关闭路径会把快捷键层抢过来（那是一条真实的越权改焦点）。
5. **读数是派生的**：`out property <int> focused-field` 由四件的 `has-focus` 三元链算出，
   ⛔ 没有新增可寻址控件、⛔ 没有第二处 setter。

**⚠ 读数语义**（判据里逐字写了，别误读）：那个整数是**四格文本框**的 `has-focus`，
**不是 trap 的** —— `move_focus`（`window.rs:1591-1629`）把 `has-focus` 发在**目标件**上，
trap 自己那条恒为假。又因 `.slint` 的 `item.focus()` 生成
`set_focus_item(.., FocusReason::Programmatic)`（`i-slint-compiler-1.18.1/generator/rust.rs:4541`）
⇒ 既**绕过可见性判定**（这就是 `focus-on-*` 两个 false 仍进得去的原因），也
**不 select-all**（那段只在 `TabNavigation` 里全选，`items/text.rs:1243-1246`）⇒ 判据 ③ 才敢
断言"插入一个字符后长度恰好 +1、去掉那一个字符等于种子串"。

**判决**：`pending` —— 本机不编译 `yeban-app`（Slint 重依赖），CI 是唯一编译器。

**同一轮新发现、本票⛔ 不顺手改的两条**：

* **`D-UI-20` 弹窗里的"按钮"没有一格可聚焦 ⇒ `Return` 触发不了导出。** 现读
  `export_wav_dialog.slint`：`export-wav-close-button`（`accessible-role: button`，159 行）、
  `export-wav-limiter-switch`（`switch`，330 行）、`export-wav-run-button`（`button`，401 行）
  全是 `Rectangle` + `TouchArea` 的手绘件 —— 没有 `Button`/`FocusScope` 容器 ⇒ 收不到焦点
  ⇒ 键盘既走不到"确认导出"也走不到那个开关。
  **⚠ 本条登记的"要先实测可聚焦容器会不会与 `TouchArea` 抢命中测试"于同日实测完毕，答案是
  不会**：`FocusScope::input_event_filter_before_children` 无条件 `ForwardEvent`
  （`items/input_items.rs:657-665`）⇒ 孩子先收到点击；`FocusScope::input_event`（667-688）
  只在 `enabled && focus-on-click && Pressed && !has-focus` 时才吞。Slint 自家 `Button`
  就是这个形状：`forward-focus` 到一个内含 `FocusScope` 的 `StateLayer`，后者
  `key-pressed(event) => { if (event.text == Key.Space || event.text == Key.Return) { … } }`
  （`i-slint-compiler-1.18.1/widgets/material/components.slint:77-88`）⇒ **`Key.Return`
  / `Key.Space` 是 `.slint` 里的合法写法**（本仓此前只用 `\u{...}` 转义，因为 `"\t"` 不编译），
  原始码位为 Return = `\u{a}`、Space = `\u{20}`（`i-slint-common/key_codes.rs`）。
  ⇒ 剩下的边界只有一个：**焦点可见性**。给这三格加 `FocusScope` 不改一个像素，
  但"看得见焦点在哪"要画焦点环 ⇒ 那会动两条已判绿的 Golden 基准 ⇒ 归人类/集成者裁决。

## §33 判据从未编过：两枚编译红 ＋ 一条**全局**门禁红（2026-11-03）

run `41512495951`（tip `2f75ab73`，D-UI-18 ＋ D-UI-19 那一批）**红**，三个作业各自红在
不同的地方 —— 逐条读回：

| 作业 | 判决 | 根因 |
| :--- | :--- | :--- |
| `rust (yeban-app)` | 红 | `export_dialog_key_capture.rs:409` `error[E0277]` —— `match_indices` 给的是 `&usize`，我拿它跟整数比 ⇒ 已在 `2f75ab73` 之后热修（`let offset = *offset;`） |
| `macos` | 红 | `search_focus_key.rs:112` `error[E0425] cannot find value 'meta'` —— 我把循环第四个元组元素绑成了 `_meta`，而第 119 行用的是 `meta`。⚠ **这条 D-UI-18 的判据从写下起从未在任何平台上编译成功过** |
| `deny (cargo-deny 开源合规)` | 红 | ⛔ **不是本线的改动**：`error[yanked]: detected yanked crate (try 'cargo update -p async-executor')`，`Cargo.lock:30` 的 `async-executor 1.14.0` 在 crates.io 被撤回。同一 sha 在 18:54 那票（`41504236343`）deny 还是绿的 ⇒ **上游撤回**，与代码无关 |

**为什么第二条是在 macos 而不是在 linux 发现的**（这是本仓库结构性的盲区，不是运气）：
`cargo clippy --all-targets` 与 `cargo test --all-targets` 都按**目标**并行编译，
但**第一个失败的目标就让作业退出**（`warning: build failed, waiting for other jobs to finish…`
之后就是 `exit code 101`）。linux 先撞见字母序靠前的 `export_dialog_key_capture`，
于是 `search_focus_key` 的错误**一位都看不见**；macos 那条腿（编译顺序不同）才把它爆出来。
⇒ 与 §6.3-4 的 `Running tests/` 截断是**同一件事的编译期版本**：
**一票红只覆盖它第一个撞上的目标**，⛔ 不许把"只看到一个错"读成"只有一个错"。

**本轮学到的机械核验（写判据的当场就要做，⛔ 不等 CI）**：

1. **每个 `get_/set_/invoke_` 名字都对着 `.slint` 的界面表面过一遍** —— 用脚本把
   `ui/app.slint` 里的 `property` / `function` / `callback` 声明提成集合，再把判据里用到的
   访问器逐个下划线转连字符去查（本轮两枚判据共 9 ＋ 8 个访问器，全部命中；
   编译错误**不在名字上**，在类型与绑定上 ⇒ 说明这道核验不能替代编译，只能缩小失败面）。
2. **`for (a, b, c, d) in [...]` 里每个绑定都必须被用到**：`-D warnings` 下
   `unused_variables` 会红，而 `_x` 又把 `x` 变不存在 —— 唯一的正解是**真的用它**，
   ⛔ 不是加下划线把问题藏起来。
3. **迭代器给的索引常是引用**（`match_indices`、`iter().enumerate()` 配合 `.filter(|(i, _)| …)`
   ⇒ `&usize`），跨平台编译器不会替本机"看起来对"背书。

**`async-executor` 这条不在本线的处置权限内**：`Cargo.lock` 是根级共享文件（多线并发写它
必然互相覆盖），而"要不要接一个更新的 semver 兼容版本"是供应链裁决 ⇒ **只上报，不自行
`cargo update`**。在这一条解决之前，任何线的 `deny` 作业都会红 ⇒ **"tip 绿"目前不可达**，
合入 `main` 的默认门禁（`MUST-GATE` 要求 `cargo deny check` 通过）因此**卡住**，
与本线的代码质量无关。
* **`D-UI-21` 模态遮罩不吞鼠标 ⇒ 弹窗开着时点击仍穿到下层。** 遮罩就是组件根那枚
  `Rectangle`（`background: mask-black.with-alpha(0.65)`），而 `Rectangle` 对鼠标事件返回
  `EventIgnored`（Slint 里只有 `TouchArea` 会吞）⇒ 整帧遮罩**只挡眼睛不挡手**；
  `D-UI-19` 新加的 trap 也帮不上 —— 它 `focus-on-click: false`，`FocusScope::input_event`
  在这种情况下同样 `EventIgnored`（实测 `items/input_items.rs`）。时光机同款：
  `undo_tree_modal.slint` 全文只有三处 `TouchArea`（✕ 钮、DAG 节点、"撤销一步"钮，
  149 / 197 / 306 行），**没有**全屏遮罩层。⇒ 现在"弹窗开着还能改下层"是真实行为，
  `export_dialog_modal_closure.rs` 的路由一节因此不再声称自己钉的是哪一条 `Esc` 路径。
  修它 = 遮罩层加 `TouchArea` 吞掉点击，但那会改变"点遮罩不关面板"的既有语义、
  且牵动两条已判绿的视觉回归基准 ⇒ 单独一票。

## §34 D-UI-20：弹窗里那三格手绘控件第一次能拿到键盘、能被 `Return` 触发（2026-11-03）

**要补的句子**：§32 登记的 `D-UI-20` —— 「确认导出」钮、「✕」钮、限制器开关三格全是
`Rectangle` + `TouchArea` 的手绘件，没有 `FocusScope` ⇒ 收不到焦点 ⇒ 键盘**既走不到导出，
也走不到那个开关**。`D-UI-19` 让模态第一次持有键盘之后，这条缺口从"看不见"变成"看得见但按不到"：
`Tab` 已经在弹窗里闭环了，闭环的却只有四格文本框。

**交付**（`ui/dialogs/export_wav_dialog.slint` · `ui/app.slint` · 判据
`tests/export_dialog_button_focus.rs`（新）· `tests/export_dialog_key_capture.rs`（4 格 → 7 格迁移）；
`src/host.rs` **一位未动**）：

1. **三枚零视觉 `FocusScope`**：`export-wav-close-focus`（195 行）、`export-wav-limiter-focus`（401）、
   `export-wav-run-focus`（478）各自铺满所在格子（`x:0; y:0; width/height: parent`），
   `key-pressed` 只认 `Key.Return || Key.Space` ⇒ 分别落到 `root.run()` /
   `root.limiter = !root.limiter` / `root.close()`。⛔ 没有第二份动作实现：✕ 走的是**既有的**
   `close()`（判据 ⑩ 数 `root.close()` = 4 条落点：无障碍 + 鼠标 + 键盘 + trap 的 `Esc`）。
2. **轮转从四格扩到七格**：`focus-next-control()` / `focus-previous-control()`（570 / 589 行）
   两端都绕回，`-1`（焦点不在模态内）时分别从首格 / 末格起步；「序号 → 控件」只有
   `focus-control(index)`（549 行）**一张表**，加减法不许自己 `.focus()`。
3. **四格文本框新增 `accepted => { root.run(); }`**（258 / 291 / 324 / 361 行）——
   `TextInput.accepted` 只在非只读单行时触发并消费 `Return`。
4. **读数第二条**：`out property <int> focused-control`（532 行，`MainWindow` 侧 2284 行**纯转发**）
   与既有 `focused-field` 从**同一批** `has-focus` 推导。⛔ 不复用 `focused-field` 的理由已写进
   注释：那条的值域被 `D-UI-19` 判据按"四格"钉死，改值域＝悄悄放宽既有判据。

**两条界面决定**（`[UI-A11Y-003]` 未规定，本票定的不是默认值而是裁决，故留痕）：

* **`Return` 在任一文本框里＝提交导出**（不是"编辑完这一格"）。四格 + 三钮共 7 个停止点里
  有 7 条路通向同一个 `run()`；
* **`Tab` 顺序把 ✕ 放在最后一格**（`0` 路径 → `1` 采样率 → `2` 位深 → `3` 秒数 →
  `4` 限制器 → `5` 导出 → `6` ✕）。这样"打开即落在路径框"（`focus-first-field()`，543 行）
  与"末档是取消"两件事不打架；⛔ 没有把 ✕ 提到前面去迎合"对话框第一格是取消"的另一种惯例。

**实测引证**（决定形状的四条，全部来自 `~/.cargo` 里 vendored 的 1.18.1 源码）：

| 事实 | 出处 | 对本票的意义 |
| :--- | :--- | :--- |
| `FocusScope::input_event_filter_before_children` 无条件返回 `ForwardEvent` | `i-slint-core-1.18.1/items/input_items.rs:657-665` | 新加的 scope **永不抢** `TouchArea` 的点击 |
| `focus-on-click: false` 时 `input_event` 对 `Pressed` 返回 `EventIgnored` | 同上 667-688 | 鼠标路径逐字节不变（本票最怕的回归面） |
| `focus_event` 只门控 `TabNavigation` / `PointerClick` | 同上 | 程序化 `.focus()` **必达**，不受可见性判定影响 |
| `Key` 是**内置命名空间**，无需 import | `widgets/cupertino/button.slint:155`、`widgets/material/checkbox.slint:162` | `.slint` 里可以写 `Key.Return` / `Key.Space`，⛔ 不必再手敲 `\u{000a}` |

**为什么不牵动已判绿的基准**：三枚 scope **不带 `accessible-id`** ⇒ 寻址面仍是 10（判据 ⑩ 断言），
`export-wav-dialog` 的 `visible` 默认 `false` ⇒ 默认帧与 `tests/golden/linux/` 五张基准一位不改；
scope 声明在各格 `TouchArea` **之前** ⇒ 命中测试层序也不变。运行时树那侧同理：
`export_dialog_modal_closure.rs` 断言的是"关闭态 id 集合相等"，弹窗关着时新件根本不在树里。

**本票⛔ 不管、如实登记的三条边界**：

1. **没有焦点环** —— 加 scope 不改一个像素，但"看得见焦点在哪"要画环 ⇒ 动 Golden ⇒ 归人类裁决；
2. **点击不改变焦点**（三格都 `focus-on-click: false`）。焦点入口只有两个：`Tab` 轮转与宿主打开时
   那一次 `focus-first-field()`。鼠标用户不受影响（钮照点），但"点一下钮再用 `Return`"这条路**不存在**；
   这条与 `D-UI-21`（遮罩不吞鼠标）**必须一起判** —— 单独给 scope 开 `focus-on-click` 会改变
   `TouchArea` 与点击归属的现状；
3. **模态持有键盘期间 `Cmd/Ctrl` 组合键到不了宿主**（trap 在祖先，`Ctrl/Alt/Meta + Tab` 一律 `reject`
   但其余 chord 不冒泡）。`[UI-A11Y-001]` 的热键表在弹窗内是**暂停**的，这是模态的定义而非缺陷，
   但"弹窗里按 `Cmd+S` 存盘"这类预期落空 ⇒ 记此以免日后当 bug 查。

**判据**：`export_dialog_button_focus.rs` ①–⑨ 为活体（`Tab` 到第 5 格按 `Return` ⇒ **真落盘一个 WAV**
并核 receipt 的 `frames`；第 6 格按 `Return` ⇒ 走既有 `close()` 且**不**落盘；⑨ 在钮上按 `Esc`
证明模态仍持有键盘），⑩ 为源码纪律（14 项计数 + 两条读数无写者）。`export_dialog_key_capture.rs`
的 ④⑤ 已从 4 格迁到 7 格，并新增一条交叉断言：焦点落在 `4/5/6` 这三格时 `focused-field` **必须**是 `-1`
⇒ 两条读数不许互相矛盾。

**判决**：`pending` —— 本机不编译 `yeban-app`（Slint 重依赖），CI 是唯一编译器。

### 同批带上的一枚判据口径修正（`search_focus_key.rs`，run `41516839490` 教的）

那票红在 `:188`：`the_focus_route_has_exactly_one_implementation` 数
`ui.invoke_focus_sidebar_search()` 得 2，期望 1 —— 第二处是 `host.rs:9383` 的**说明注释**里
逐字写了这个调用（带括号，长得就像调用点）。

* ⚠ **修法是给计数定口径，⛔ 不是删掉那句注释来喂判据**。新增 `code_only()`：先按行剥掉
  `//` 开头的行，再数 ⇒ 口径写进判据头与函数文档，读日志的人不必猜"这个 2 是代码还是注释"。
* 这条口径**适用于本仓所有源码纪律型判据**（§34 的 ⑩ 也一律走 `code_only()`）：
  **注释不是实现站点**。这是"字面量计数"这类判据的通用失效模式，与 §6.4-1「参考文档的警告是证据」
  并列 —— 前者防"实现偏离已写的规矩"，这条防"判据把文字当代码"。
* ⚠ 那票红的**覆盖**后果（§6.3-4）：`cargo test` fail-fast，`search_focus_key` 之后的
  **47 / 153** 个测试二进制一位都没跑（`ui_export_dialog`、`sidebar_search`、`undo_tree_*`、
  `sfz_*`、`volume_automation_export`…）。⇒ 在 `0c89f750` 上这些的判决是**未知**，⛔ 不是绿。
  本票把它们一起推上去，正是为了拿到这份读数。

### 落地方式（本机 git 传输坏死的处置，留痕以免下轮误判为"未推送"）

`git push` / `git fetch` 在本机一律 `GnuTLS recv error (-110)` 或挂死，而 `gh` / `curl` 正常 ⇒
本线的提交经 GitHub git-data API 重建对象后**非强制**移动 ref（`/tmp/api_push3.py`：按
`rev-list 远端tip..本地HEAD` 逐条建 commit，最后一个 tree 必须等于本地 HEAD 的 tree 才动 ref）。
由此**本地 sha 与远端 sha 不同**（`54036a5c` ↔ `0c89f750`，两者 tree 同为 `45a4cc83`，
差异只有 committer 日期被规范化成 `Z` 与末尾空行）。⇒ 读判决时**认 tree，不认 sha**；
待 git 传输恢复后用 `git fetch && git reset --hard origin/line/zimmer-export-limiter`
把本地指针对齐（破坏性操作，需人类确认后再做，本票不擅自执行）。

## §35 run `41522031319` 读回：**第四次零读数**，根因是我写的闭包 borrows（2026-11-03）

远端 tip `1f771c1d`（tree 同本地 `6eac4231`）。各作业结论：`checks` / `plan` / `lockfile` **绿**；
`deny` **红**（还是集成者那枚 `Cargo.lock` 里的 yanked `async-executor 1.14.0`，与本线无关，⛔ 我不动锁文件）；
`windows` 与 `rust (yeban-app)` **红**；`macos` 当时仍在跑；`rust (workspace 全量)` 与 `e2e` **skipped**（§6.3-2 的空绿，不算验证）。

`rust (yeban-app)`（作业 `125540651732`）的日志里，单位＝**行条数**：

| 数什么 | 值 |
| :--- | ---: |
| `Running tests/` 出现的行（= 编译并跑起来的测试二进制数） | **0** |
| `test result:` 出现的行（= 真正给出的判决数） | **0** |
| `error[E0502]` 出现的行 | **17** |
| 涉及的源文件（去重） | 只有 `tests/export_dialog_button_focus.rs` |

⇒ **D-UI-20 的判据从未编过**，`0c89f750` 那一批未跑的 47/153 依旧未跑。这是第四次零读数，
而且根因与前三次都不同（前三次：`E0425` 少 import、注释里的字面量喂了计数、`&usize` 比整数）。

**缺陷本身**：`:180-181` 写了 `let control = || ui.ui().get_export_wav_focused_control();`。
闭包按 `&ui` 捕获端口，而 `LiveUi::pump_meters(&mut self)` / `dispatch_key_press(&mut self)`
要 `&mut ui` ⇒ 之后每一次按键注入都与这个仍然活着的不可变借用冲突（17 处，全是同一枚闭包的账）。
`ui.ui()` 自己是 `&self`（`live_surface.rs:1739`），所以问题**不在读属性**，在**闭包把借用延续了整段**。

* **修法**：删掉两枚闭包，改成与已判绿的 `tests/export_dialog_key_capture.rs`（D-UI-19）**逐字同款**的
  内联读法 `ui.ui().get_export_wav_focused_control()`（`control()` 12 处、`field()` 5 处全部替换）。
  ⛔ 不是"把断言改松"，⛔ 不是删掉某条判据 —— 判据数量、口径、期望值一位未动。
* ⚠ **generalisable 的规矩**：本机对 `yeban-app` 唯一的语法神谕是 `cargo fmt`（它只做**解析**，
  §33 就靠它兜住字面量），而**借用检查不在其中** ⇒ `cargo fmt` 绿⛔ 不等于能编。
  因此新增一条自查：**判据文件里不许写捕获端口的闭包**；要复用读数就内联，或写成
  `fn xxx(port: &mut Port)` 这种**显式形参**的自由函数（`tests/d_ui_21_modal_mask_swallows_clicks.rs` 用的是后者）。

## §36 D-UI-21：遮罩**第一次真的**吞掉鼠标（导出弹窗 ＋ 时光机，2026-11-03）

§32/§34 登记的"点遮罩会穿到下层"这条，今天闭环。缺陷的机制只有一句：两份模态的遮罩都只是
一层 `Rectangle` 背景，而 **`Rectangle` 对鼠标事件返回 `EventIgnored`**；Slint 里只有 `TouchArea`
会拦 —— `i-slint-core-1.18.1/items/input_items.rs` 的 `Pressed` 那一支只要 `enabled` 就返回
`GrabMouse`，**与有没有写 `clicked` 无关**（这条是实测，不是推断：正因为无关，才能加一枚"只吞不做"的遮罩）。

### 交付

1. `ui/dialogs/export_wav_dialog.slint:115-120` —— 组件根 `Rectangle` 的**第一个**孩子：
   全尺寸、无回调、无 `accessible-id` 的 `TouchArea { x:0; y:0; width: parent.width; height: parent.height; }`。
2. `ui/dialogs/undo_tree_modal.slint:92` —— 同一枚，位置同理（声明在面板 `undo-tree-panel` **之前**）。
3. `crates/yeban-app/tests/d_ui_21_modal_mask_swallows_clicks.rs` —— 两条 `#[test]`：
   活体 ①–⑥ ＋ 源码纪律 ⑦（不开窗口）。

### 为什么"只吞不做"是对的语义，而不是偷懒

既有的关面板路径只有两条：✕ 与 `Esc`（`export-wav-dialog.slint` 的 `close` 回调，宿主
`on_close_export_wav`）。若给遮罩补一条 `clicked => { root.close(); }`，就把"点外面不关"改成了
"点外面就关" —— 那是**交互裁决**，不属于本票（本票的职责只有一个：下层收不到）。
所以 ③ 同时钉两条：**属性不翻**（吞掉了）且**弹窗仍开着**（没顺手关）。

### 为什么它不改任何像素、也不改运行时树

* 无 `background` / `border` ⇒ `TouchArea` 本身不可见；
* ⛔ 无 `accessible-id` ⇒ 装配时与运行时的可寻址面一位不变 ⇒ `tests/golden/linux/` 的五张基准、
  `tests/ui_export_dialog.rs` 的寻址断言、`tests/export_dialog_modal_closure.rs` 的闭合 id 集全部不受影响；
* 声明在面板**之前** = Slint 命中测试从最上层往下走时，面板及其里的控件**仍在遮罩之上**
  ⇒ ④「点弹窗内的 ✕ 必须真的关弹窗」证的正是这条顺序（把遮罩移到面板之后 ⇒ 弹窗把自己也挡聋，④ 会红）。

### 判据（①–⑥ 是同一条链，不是六个独立断言）

`VICTIM = "console-maximize-button"`：底部控制台右上角的 `⤢`，它的点击跟一枚**纯视图态布尔**
`console-maximized`（`tests/console_maximize.rs` 已证这条路是活的）。

| | 证的事 |
| :--- | :--- |
| ① 对照臂 | 弹窗**关着**时真点击 ⇒ 翻 `true`，再点 ⇒ 翻回 `false`。⛔ 这条不成立时 ③ 的"没翻"什么也不证 |
| ② 前提几何 | 弹窗开着 ⇒ 遮罩与受害者**同时在**运行时树里，且受害者**中心不在** `export-wav-panel` 的矩形内 ⇒ 挡住它的是遮罩，⛔ 不是"控件不在场"、⛔ 不是"面板压着它" |
| ③ 吞 | 同一坐标再点 ⇒ 属性**仍是** `false`，且弹窗仍开着（**修前这一条是红的**） |
| ④ 面板仍可点 | 点 `export-wav-close-button` ⇒ 弹窗关闭 |
| ⑤ 关掉后复活 | 再点同一处 ⇒ 翻 `true` ⇒ 证 ③ 是遮罩，⛔ 不是回调坏了 |
| ⑥ 时光机同款 | `set_undo_tree_open(true)` 期间点 ⇒ 不翻（属性停在 `true`）；关掉 ⇒ 翻回来 |
| ⑦ 源码纪律 | 两份模态各有**恰好一枚**"无 `clicked` 的全尺寸 `TouchArea`"，且它排在面板 id 之前；`TouchArea {` 恒 4、`clicked =>` 恒 3、`accessible-id:` 恒 10 / 7 —— 一律走 §34 的 `code_only()` 口径 |

每次点击之前**重抓控件树**：`dispatch_pointer_down` 用的是端口里缓存的包围盒，而 ① 之后控制台
真的变高了（会移动）。偏移 `(2.0, 2.0)` 与 `console_maximize.rs` / `undo_tree_close_button.rs` 逐字同款。

### 边界（如实登记，⛔ 不假装覆盖）

1. **`Esc` / 键盘那条路径不在本票** —— 模态持键盘已由 D-UI-19/20 覆盖，本票只管鼠标。
2. **点遮罩不关面板**：见上，属交互裁决。若人类要改成"点外面就关"，那是新的一票（要同时改 ③ 的期望）。
3. **其他两份覆盖层没带遮罩拦截**：`ai-rail` / `musical-pr-drawer` / `macro-panel` 不是模态
   （它们没有持键盘），本票⛔ 顺手给它们加"点不进下层"——那会改变非模态面板的行为，属另一票。
4. **鼠标悬停态（本票⛔ 未测，只登记推断）**：命中测试是**同一套**从上往下的顺序，所以遮罩那枚
   `TouchArea` 成为最上层的鼠标敏感项后，下层的 `hovered` 状态理论上不再点亮 ⇒ "弹窗开着时
   下层按钮不会再高亮"。这一条**没有判据**（本票的 ①–⑥ 全读属性，不读 hover），
   属同机制的推论，若将来要钉需另开一票并先确认没有基准依赖 hover 态。

**判决**：`pending` —— 本机不编译 `yeban-app`；`cargo fmt` / `cargo fmt --check` 均 `rc=0`
（只能证语法与格式，见 §35），守卫脚本本地全绿，⑦ 的四个计数已在本地用真实源码模拟过
（`dialog: TouchArea 4 / clicked 3 / ids 10 / 遮罩在面板之前 = true`，`time_machine: 4 / 3 / 7 / true`）。

## §37 `D-UI-22` 编号落定：从画布进文本框的键位是**规范冲突**，⛔ 不是漏接线（2026-11-03）

§31 那句"文本框全都摸不到键盘"从今天起有编号、有票、⛔ 没有代码。

* **为什么不直接接线**：`Tab` 在画布上已被 `[UI-A11Y-001]` 键位表（`DESKTOP_UI_UX:300`）钉成
  "视图切换"，而 `[UI-A11Y-003]` §7.3 第 1/4 条要求"全界面 Tab 序列"并把轮转列为 CI **MUST**。
  两句对**同一个键**说相反的话 ⇒ 满足一句必然违反另一句，这是产品裁决（§5.7：⛔ 不私自发明答案）。
* 留痕：`docs/adr/ADR-0067-keyboard-entry-into-text-fields.md`（`Proposed`，三条路线 D1/D2/D3，
  建议 D1 = `D-UI-18` 专属 chord 那条成例的推广；README 索引已同步，G20 本地绿）。
* ⚠ **修掉一处腐烂的行号**（§6.1-1 那条"别写行号"的教训第三次生效）：§31 里那句
  `app.slint:717` 的注释在 D-UI-19/20 之后落在 `:716`。⇒ 本票新增的引用一律**同时给出锚点**
  （"`key-action` 返回值那段注释"），行号只作辅助。
* **可达性现状**（本票现读，逐条对过 grep）：搜索框 ✅ ／ 导出弹窗七格 ✅ ／ 时光机 ✅ ／
  `bpm-input` ⛔ 仅鼠标 ／ `device-*` 文本框 ⛔ 仅鼠标。后两条是 D1 的工单范围。

## §38 `D-UI-23`：预备拍开关**关不掉、也持久化不了** —— 落点从"换一代引擎"搬到"改文档"（2026-11-03）

症状（§31 那一族"键白按"的一个新形状）：`Cmd/Ctrl+Alt+D` 按第二下，预备拍**还是开着**；
关掉之后保存工程，下次打开又是开的。界面上一切看起来"响应了"。

* **事实源错位**（根因，⛔ 不是漏了一条 `commit`）：那条臂算"下一个值"读的是
  `EngineCommandContext::project` —— 装配期**冻结**的那份 `Rc<YebanProjectV1>`（`main.rs`
  只在**换工程**时经 `after` 重装配）。冻结快照永不因按键而变 ⇒ 每一下都算出同一个"下一个值"
  ⇒ **单向置位**，不是开关；而文档一位没改 ⇒ **没有东西可持久化**。
  ⚠ 唯一的读者 `record_wav::count_in_frames` 读的**也是**那份冻结快照 ⇒ 换的那一代引擎里
  `count_in_bars` 仍是旧值，录音前照样空转。
* **换代是纯代价**：`metronome::set_count_in_bars` 走 `apply_transport_config` ⇒
  先 `stop` → `reload` → `seek` → 恢复走带（位置还会漂移几个 tick，见 `MAX_RESTORE_DRIFT_TICKS`）。
  而实时引擎⛔ **不实现**预备拍 —— `crates/yeban-engine/src/lib.rs` 逐字把"预备拍
  （`transport.count_in_bars`）"列在**仍未实现**那一栏。⇒ 停走带的代价付了，收益为零。
  ⇒ **判据由此改写**：`action_needs_engine` 的入选门槛是「**实时引擎自己读这个字段**」，
  ⛔ 不是「字段住在 `TransportConfig` 里」。节拍器留下（它的计划在快照投影期进快照），预备拍摘出。
* **修复形状**（三条性质同时拿到）：落点搬到 `apply_action` 的**文档级提交** ——
  `Op::SetTransportConfig { old_config, new_config }`（`old_config` 从 `UndoPort::try_project()`
  现读，模型对陈旧 `old_config` 会以 `OpStateMismatch` 拒绝 ⇒ 不是自己拼一份"我以为的当前值"）。
  ⇒ ⭐ 可撤销、⭐ 会持久化、⭐ 下一个读者读到的就是它。
  ⚠ `metronome_enabled` 与 `launch_quantization` **逐字带着走**（整体替换那条 op 的代价就在兄弟字段上：
  顺手把节拍器的运行态镜面写进文档 = 造出第二个事实源）。
* **取数三档**（新函数 `host::record_count_in(undo, frozen, sample_rate)`，单位：**帧**）：
  端口在场 ⇒ 读**活文档**（端口在场但读不出工程 ⇒ 如实 0，⛔ 不拿冻结那份冒充现状）；
  无端口（判据 / 无头装配）⇒ 照旧退回装配期那份，⛔ 不改既有形态；两份都没有 ⇒ 0。
  抽成一个函数不是为了节拍器开旁路，而是**优先级只写一次**、判据能**逐档问它**
  （⛔ 不必起线程拉设备源才能观测）。
* **真的拆掉了**：`metronome::set_count_in_bars` 删除（内核 `apply_transport_config` 保留，
  `set_enabled` 还在用）⇒ ⛔ 不留一枚没人调的 `pub fn`（判据 ⑨ 从源码现数：代码里 0 处）。
  ⚠ 顺带一处 clippy 陷阱：摘掉预备拍后 `apply_engine_command` 的判据 ① 只剩一条动作，
  写成单元素 `for` 在 `-D warnings` 下**直接红**（`clippy::single_element_loop`）⇒ 改成裸块。
  集合奇偶由 `src/test_port_adapter.rs` 的 `only_engine_commands_take_the_engine_borrow`
  从**源码文本**对账（注释先剥掉 ⇒ 注释不是实现站点）。
* **判据**：`crates/yeban-app/tests/d_ui_23_count_in_writes_the_document.rs`，四例九档 ——
  ① 前提读数（`count_in_bars = 2` **小节**、`bpm = 128.0`、分子 4 ⇒ 180 000 **帧** @ 48 kHz）
  ② 第一下真的改文档（2→0 小节、提交恰好 +1 **条**）③ 开关两极性（0→1→0，合计 +3 条）
  ④ 兄弟字段与界面镜面 `metronome-on` 一位不动 ⑤ 重投影不许画歪六格读数（两档**非自比较**对照）
  ⑥ `Cmd+Z` 放回 1 小节 + redo 回 0 ⑦ `undo = None` 的装配 ⇒ 返回 `false` 不消费
  （对照臂：同一装配里数字键照常换工具，否则 `false` 可能只是"这键整体没接"）
  ⑧ 取数三档逐档 ⑨ 源码出处纪律（`Op::SetTransportConfig {` 构造点 1 处、`record_count_in(`
  定义+唯一调用 2 处、`Action::ToggleCountIn` 在 `host.rs` 代码里 1 处、`set_count_in_bars` 0 处）。
* **如实登记的边界**（⛔ 不假装整条链通了）：
  1. **实时播放里仍然听不到预备拍** —— 引擎不实现它（上面那一栏）。本票修的是
     "UI 的开关 ⇒ 文档 ⇒ 读得到文档的读者"，⛔ 不是"走带时会数拍子"；那是**引擎功能请求**。
  2. **界面上没有预备拍那一格** —— `ui/**/*.slint` 里 `count_in` / `countIn` / `预备拍`
     **0 命中**（不加限定的 `count` 子串有 91 处，全是 `accessible-item-count` /
     `routing-count-label` / `undo-commit-count`，没有一处承载 `count_in_bars`）
     ⇒ 拨完开关用户**看不见**状态。本票⛔ 不新增控件（加控件 = 改像素 = 金帧重录 = 人类裁决），
     这条与 §37（`ADR-0067`）一起等 UI 面裁决。
  3. 端口在场但会话被控制面关掉（`try_project() == None`）⇒ 0 帧 + 一行 stderr + 不消费；
     判据造不出这一档（那个 `None` 只在 `UndoBackend::Authority` 那一支，夹具是 `Local` ⇒ 恒 `Some`），
     故只登记不验。

**判决**：`pending` —— 本机不编译 `yeban-app`（Slint 是重依赖）。本地只证到
`cargo fmt -p yeban-app -- --check` `rc=0`；`git diff --numstat`：`cli.rs 4/2`、`host.rs 112/27`、
`input.rs 4/2`、`metronome.rs 11/22` ＋ 判据文件一枚。⚠ 本票**没有** `.slint` 改动 ⇒
不需要金帧重录；`wire_record` / `wire_record_at` 的签名**未变**（`undo` 参数本来就在）⇒
既有调用点一处不改。CI 回来前不落"通过"。

## §39 `D-UI-24`：重投影**不再抹掉**正在敲的 BPM 草稿（编辑保护只写在一个地方）（2026-11-03）

症状（§31 那一族的另一个形状，⛔ 不是"键白按"而是"字白打"）：在走带条的速度框里敲到一半
（例：`128.00` 里删掉尾数打成 `13`），**点一下卷帘里的剪辑**（或 AI 会话侧改一次工程），
框里那串**立刻**变成权威读数 `128.00` —— 用户看到的是"打的字没了"。

* **根因**（一条无条件写的语句，⛔ 不是缺一条 `commit`）：`host::apply_view` 每次重投影都
  **无条件** `set_bpm_display(权威读数)`，而它正是"权威 → 界面"那一面的**唯一注入点**。
* **为什么点击会触发**（前提取证，判据 ⑥ 从源码现数）：画布那三层
  （`ui/workspace/arrangement_view.slint` / `ui/console/piano_roll.slint` /
  `ui/workspace/session_view.slint`）里 `FocusScope` **0** 处、`.focus(` **0** 处 ⇒
  点剪辑**不会**让文本框失焦，焦点一直留在 `bpm-input` 里；而那一次编辑会提交 ⇒
  `refresh_undo_window` 重投影 ⇒ 盖掉。第二条触发腿是 `reproject.rs`（会话侧 `tools/call`
  改工程后重投影同一个活窗口），两条都**不需要 AI 在场**才成立。
* **修复形状**（三个"只有一处"）：门 = `MainWindow` 的一格 `in property <bool>
  bpm-field-editing`；**读者只有 `apply_view` 那一句**；**写者只有两个** —— `wire_input`
  把 `bpm-input` 自己在 `changed has-focus` 里发出的**那条既有事件**镜像过去（与
  `[UI-A11Y-002]` §7.2 喂 `InputContext::set_focus` 的是**同一次**通知 ⇒ ⛔ 不新造事件源、
  ⛔ 不轮询焦点系统），加上 `wire_bpm_commit` 进门**清零**。判据 ⑦ 把这四个数钉死。
* **为什么它是 `in` ⛔ 不是派生 `out`**（与 `shortcut-layer-focused` / `export-wav-focused-field`
  不同形的原因）：`bpm-input` 声明在**子组件** `Transport` 内部，主窗口层的绑定取不到那个标识
  ⇒ 只能由宿主把已存在的事件搬过来。
* **既有两条口径一位不动**（本票⛔ 不改变"提交后写什么"）：回车**本身解决草稿** ——
  清零之后 `wire_bpm_commit` 里那一个 `refresh_undo_window` 照常写：非法 / 同值 ⇒ 写回
  权威读数（`tests/ui_bpm_field.rs` ③/④"⛔ 不留冒充权威的草稿"），合法 ⇒ 归一化 `{:.2}`。
  ⚠ 清零的语义是"这串草稿已有裁决"，⛔ 不是"焦点离开了框"（判据 ⑤ 在**没有**失焦事件的
  情况下断言门已假）。
* **判据**：`crates/yeban-app/tests/d_ui_24_bpm_draft_survives_reprojection.rs`，七例 ——
  ① 对照臂（没在编辑 ⇒ 注入照常写，保护⛔ 不许外溢成整次注入不跑）② 编辑中 ⇒ 草稿活着
  **且同一次注入的标题那格照改**（⛔ 不能让"没被盖"混成"注入没执行"那种假绿）
  ③ 由**真改文档**的一次按键（`Cmd/Ctrl+Alt+D` ⇒ 预备拍 +1 条提交）引发的重投影⛔ 不吃草稿
  ④ 失焦后下一次注入写回权威 ⇒ ⛔ 造出永久分叉 ⑤ 回车两支（拒 `"9999"` / 收 `"140"`）
  都归一化 + 门清零 ⑥ 前提取证（三层画布 0 处 `FocusScope` / 0 处 `.focus(`）
  ⑦ 出处纪律（`set_bpm_display(` = **1**、`get_bpm_field_editing(` = **1**、
  `set_bpm_field_editing(` = **2**、`app.slint` 代码行里 `bpm-field-editing` = **1**，
  `text <=> root.bpm-display;` 那一句双向绑定仍在）。⚠ 四个数都在提交前**现读源码**核过
  （`code_only` 先剥 `//` 行 ⇒ 注释⛔ 算实现站点）。
* **如实登记的边界**（⛔ 不假装整类缺陷修完了）：
  1. ⚠ **本条第一版写错了，由 §40 从上游源码改正**（原文声称"调音台 SFZ 路径格同样会被
     重投影盖掉"）。改正后的事实：`mixer_console.slint:738` 的 `sampler-path-field` 是
     **单向**绑定（`text: root.track-sampler-paths[track_index]`），而单向绑定在用户**第一次
     按键**时就被打断（`i-slint-core-1.18.1/properties.rs:1029-1043`：`Property::set` 在默认
     `intercept_set == false` 的绑定上 `remove_binding`）⇒ 它**不会**被盖掉，它得的是**反面**
     缺陷：**从此再也不跟随权威**（浏览按钮填不进框、撤销/AI 侧改动显示不出来）。
     那一格本票一位没动，⛔ 也不能照本票的形状修（Slint 的 `<=>` 拒绝下标左值 ⇒ §40）。
  2. 左栏搜索框**没有**这个缺陷，且⛔ 不该有门：`sidebar-search` 的唯一写者是输入框自己的
     两向绑定，宿主刻意⛔ 不 `set_sidebar_search`（`host.rs:5795-5797` 原话：再写一次就与
     绑定互为回路）。导出弹窗那四格⛔ 不在 `apply_view` 的注入面上 ⇒ 与本票无关。
  3. 回车之后门已被清零 ⇒ **同一次聚焦会话里**再打第二串草稿仍然暴露在同一触发下（要再点一次
     才会重新置门）。这条只在判据 ⑤ 之后成立，本票⛔ 不把它说成已闭合。
  4. 判据 ③ 借预备拍键只当一个**真的会改文档并重投影**的漏斗用，⛔ 不是主张"文本框聚焦时
     DAW 热键照常触发"—— 真实键盘下字符键归文本框，那是 `ADR-0067`（`D-UI-22`）未决的
     规范冲突，无头注入绕过的是**派发**那一层。
     ⛔⛔ **这一条在写下当天就被判决证伪**（见 §43）：无头注入⛔ 绕过不了任何一层，
     `invoke_key_action` 与真窗口共用**同一个** `resolve_logical`。

**判决**：`pending` —— 本机不编译 `yeban-app`（Slint 是重依赖）。本地只证到
`cargo fmt -p yeban-app -- --check` `rc=0`，加上 ⑦ 那四个数与 ⑥ 那六个 0 的**现读对账**。
`git diff --numstat`：`src/host.rs 32/1`、`ui/app.slint 23/0` ＋ 判据文件一枚。
⚠ `app.slint` 那 23 行**只有一格属性声明**（`in property <bool>`，画不出任何东西、
不改任何坐标）⇒ 本票**零像素改动**，不需要金帧重录。

## §40 取证：全仓库**六个** `TextInput`，只有调音台 SFZ 那格是单向绑定 —— 它得的缺陷与 `D-UI-24` **反面**（2026-11-03）

数什么（单位：个）：`grep` 全部 `ui/**/*.slint` 里 `:= TextInput` 的**控件数**与其 `text`
的绑定形态 ⇒ **6** 个输入框：`bpm-input` / 导出弹窗四格（`path` `sample-rate` `bit-depth`
`seconds`）/ `sidebar-search-input` = **5** 个 `text <=> …`（双向），
`mixer_console.slint:738` 的 `sampler-path-field` = **1** 个 `text: root.track-sampler-paths[track_index]`（**单向**）。
⇒ 它是全仓库唯一的例外，而本仓早已把"单向绑定会被第一次按键打断"写成教训
（`transport.slint:30`、`transport.slint:285`、`sidebar.slint:214`、`[PATCH export-wav-dialog-binding]`）。

* **用户可见的两条害**（都由"绑定已死 ⇒ 这格不再跟随权威"推出）：
  1. **浏览按钮填不进框**：`wire_mixer_sampler_browse`（`host.rs:2312-2335`）只
  `set_track_sampler_paths(...)`，⛔ 不提交、不改文档（口径写在 `host.rs:2297`）。用户若**先敲过
  一个字**再点"…"选中文件，那一次的数组重写进不到已断绑定的这一格 ⇒ 框里仍是旧草稿，
  看起来"浏览没反应"。
  2. ⭐ **骗 AI**（比视觉问题重）：`TextInput` 的 `accessible-value` 自动 == `text`
  （上游 `lower_accessibility.rs`，本仓 `mixer_console.slint:715-719` 逐字记着这条），
  而 `text` 停在草稿上 ⇒ 控制面 / 屏读器读 `track-{i}-sampler-path` 读到的是**非权威**的串，
  与文档里的 `TrackV3::sampler` 分叉。撤销一次 SFZ 绑定、或 AI 侧改路径，界面上那格都不动。
* **上游证据**（版本钉 `1.18.1`，本机读 `~/.cargo/registry`，⛔ 不需要编译）：
  1. **单向绑定确实会被打断**：`i-slint-core-1.18.1/properties.rs:1029-1044`
     `Property::set` 先问绑定的 `intercept_set`，默认绑定返回假（`:353`）⇒ `remove_binding()`（`:685`）；
     双向绑定自己 `intercept_set == true` 因此活得下来（`properties/two_way_binding.rs:21-24`）。
     按键那条路就是普通 `set`：`items/text.rs:1679-1681` `set_text_internal` 里 `self.text.set(text)`。
  2. ⛔ **修法不能是"改成 `<=>` 就完事"**：`i-slint-compiler-1.18.1/passes/resolving.rs:3169-3211`
     只接受 `PropertyReference` / `StructFieldAccess` / `RepeaterModelReference` 三种左值，
     下标 `arr[i]` 解成 `Expression::ArrayIndex` ⇒ 落到 `:3257-3261` 那条原话诊断
     `Two-way bindings can only target property references`。**这是一堵编译错误的墙**，
     ⛔ 一个"写法没写好"的问题。
  3. 上游**唯一**支持的"逐行可编辑"两向形态是绑到 `for` 的**行数据**（`TwoWayBinding::ModelData`），
     回写走 `model_set_row_data`（`generator/rust.rs:926` 与 `:4120`；`generator/cpp.rs:716`）
     ⇒ 要它跟随权威，就得把被迭代的那个模型从 `[string]` 换成**带字段的 struct 模型**
     （现状：`mixer_console.slint:503` `for track_name[track_index] in root.track-names`）。
* **为什么它⛔ 是 `D-UI-24` 的复制品**：那一票的症状是"写得太勤"（每次重投影都盖掉草稿），
  这一格的症状是"**再也不写**"（断绑定之后权威进不来）。反过来，一旦真把两向链接到行数据上，
  宿主每投影换整份模型 ⇒ **又变回盖掉**那一族 ⇒ 这一票要同时成立，**前置**是"带身份的焦点上报"
  （`host.rs` 里那条注释原话：失焦⛔ 能硬编码成"回到画布"，得改成问焦点系统当前是谁）。
  两路 `changed has-focus` 的**先后顺序**没有上游保证（`sidebar.slint:77-82` 登记的就是这一条）
  ⇒ 身份必须进通知（只有 `id` 对得上的失焦才准清零），⛔ 是一个布尔位能表达的。
* **需要的设计裁决**（⛔ 由 Agent 私自定，属状态分层决策）：调音台 SFZ 那一格的事实源怎么放 ——
  **A** 把 `track-names` 升成 struct 模型（`{ name, sampler-path }`），行数据成为**瞬态草稿镜像**
  （权威仍是文档），配带身份的焦点门 ⇒ 唯一能同时拿到"跟随权威"＋"可手敲"的形状，工作量最大
  （`.slint` 里 `track_name` 的每一处引用要改、宿主注入要造 struct 数组、焦点通道要接第二路）；
  **B** 收掉可编辑面：那一格改**只读**显示，路径只经"浏览 / SET" ⇒ 症状消失，代价是用户不能手敲粘贴；
  **C** 维持现状，把上面两条害按已知缺陷登记（本 §）。
  ⇒ 本票⛔ 动一行代码，等 A/B/C 的裁决；一有裁决这就是**下一票**的形状。

**判决**：`pending`（本条是**取证与登记**，⛔ 有代码改动）—— 证据全部来自本机现读：`ui/**/*.slint`
的 `TextInput` 清单、`host.rs` 的两个函数、以及 `~/.cargo/registry` 里 `1.18.1` 的上游源码。
⛔ 本机没有编译任何东西，因此"改 `<=>` 会红"这一条是**读上游诊断得出的**，⛔ 是 CI 实测的；
若将来有人按 A/B/C 动手，第一手判据应当是**先量出这一格当前的跟随行为**（敲一个字 → 改权威 →
读 `track-{i}-sampler-path` 的 `accessible-value`），把结论写在判据里再谈修法。

---

## §41 `D-UI-26`：界面上**印出来的和弦**里有一把根本⛔ 存在 —— 修文案，⛔ 补键（2026-11-03）

**症状**（用户可见的那一面）：AI 协作栏抽屉的按钮把键位写在 `accessible-label` 里 ——
`"AI 协作栏抽屉 (Cmd/Ctrl + 反斜杠)"`。屏读器会念这句，用户照着按 `⌘\` / `Ctrl+\`：**什么都没发生**。
⇒ 这是"按钮点了没反应"同族、却更难发现的一族缺陷：⛔ 缺像素、⛔ 缺回调，只缺一行文案与键表的 correspondence。

**根因**（现读 `src/input.rs`）：第 3 步 `if modifiers.command()` 那张匹配表里**从未**有过
`'\\'` 这一臂 ⇒ `Cmd/Ctrl + \` 落到 `_ => Resolution::PassThrough`。抽屉真正的键盘入口是
`Cmd/Ctrl+Alt+A`（`ADR-0064` 第 4 条点名；端到端判据 = `tests/ai_drawer_key_ui.rs`，
注入的是 `("a", false, true, …)`）。⇒ **修法是⭐ 把文案改成真绑的那一把**，⛔ 是给 `\` 补一臂
（那会造出**第二把**键干同一件事 —— 键位是界面决定，⛔ 由 Agent 顺手发明）。
同一句假承诺还在 `ui/app.slint:2117` 与 `ui/transport.slint:505` 的**注释**里各留了一份
（注释⛔ 承诺，但它会教下一个读代码的人相信那把键存在）⇒ 一并改成 `Cmd/Ctrl+Alt+A`。

**取证时挖出来的第二件事：承诺有**两个**源。** `src/elements.rs` 的
`registry.add(id, kind, file, name, ..)` 里那份 `name` 也带键位（`yeban-ui-test-port` 的控件树
读数，屏读与 AI 都从这里取），实测有 **3** 格携带和弦：`transport-metronome-button`
`"节拍器 (Cmd/Ctrl + Alt + N)"`、`musical-pr-accept-button` `"采纳提案 (Shift+Enter)"`、
`undo-tree-undo-button` `"撤销一步 (Cmd+Z)"` —— 三者都**逐字等于**对应 `.slint` 里的那条字面
（今天一致 ⇒ 不是缺陷，但它是**下一个** `D-UI-26` 的温床：改 label 而忘改登记名，界面与控件树
就各说一把键）。因此本判据把两边一起扫，并把"不许漂移"钉成一条（⑤）。
⚠ Rust 侧只扫 `registry.add(..)` 参数**区间**：`elements.rs` 的**散文**里也出现
`Shift+Enter`（实测第 3964 行那条 assert 消息），把散文当承诺会让守卫变成假红机器 ——
同一区间外还有一处 `'Enter`'` 会命中"键名不在词表"，正是靠区间口径排除的。

**清点口径**（单位：**一次印刷** = 一条 `Chord`；⛔ 是"不同的键"）：
17 个 `.slint`（编译期 `include_str!`）扫出 **6** 处印刷，登记表扫出 **3** 处，合计 **9** 处、
**4** 把不同的和弦 —— `Cmd/Ctrl + Alt + N` ×2、`Cmd/Ctrl + Alt + A` ×1、`Cmd+Z` ×3、
`Shift+Enter` ×3（次数含**两源**合并）。写坏了的键位提示（修饰词接不到 `+ 键名`）：**0** 处；
词表不认识的键名：**0** 处。磁盘上 `.slint` 文件 **17** 个 == 清单 **17** 项。

**判据**（`crates/yeban-app/tests/d_ui_26_labelled_chords_are_bound.rs`，**8** 条 `#[test]`）：
①多重集 == `PINNED_PROMISES`（原话与次数都钉住 ⇒ 新增承诺必须同步扩表）；②每一把喂
`InputContext::resolve_logical`（画布焦点）都出**期望的那个** `Action`，`Cmd/Ctrl` 的 `ctrl`
与 `meta` **两条腿都验**（`Modifiers::command()` 就是 `ctrl \|\| meta`）；③`Cmd/Ctrl+\` 仍 ⇒
`PassThrough` 且扫出的和弦⛔ 含 `'\'`（假承诺⛔ 许回来）；④AI 抽屉 label 逐字 == 真绑那把、
登记名那一格⛔ 带和弦、且 transport 里承诺 `'a'` 的键位**恰好一处**；⑤带和弦的登记名必须与某条
`.slint` 字面逐字相同；⑥磁盘 == 嵌入清单（`read_dir` 只用于这一条对账，口径同
`tests/assertion_bounds_audit.rs`）；⑦"承诺写全"棘轮（incomplete/unknown 必须为空集）；
⑧同一把和弦在 `Focus::TextInput` 下必须仍 ⇒ `PassThrough`（那是 `ADR-0067` 登记的**现状**，
⛔ 让 ② 的绿被读成"任何焦点下都生效"）。

**本机怎么验的（⚠ 这条决定判决的可信边界）**：`AGENTS.md` §5 ⇒ 本机⛔ 编译 `yeban-app`。
所以我把这个判据文件**原样**搬进一个零依赖的桩 crate（`ui/`、`src/elements.rs` 用符号链接指向真实
文件，只把 `input` 的类型形状照抄、键表只放钉住的那几臂），`cargo test` **8 条全绿**，并用
**变异检验**证明它⛔ 空转：删掉桩表里 `(Character('a'), false, true)` 那一臂 ⇒ ② 立刻红在
`ui/transport.slint:711 印着 Cmd/Ctrl + Alt + A（整条字面 "AI 协作栏抽屉 (Cmd/Ctrl + Alt + A)"）
—— Ctrl 形态在键表里解析成 PassThrough`。⇒ 桩证明的是**扫描器与对账逻辑**（词表、区间口径、
多重集、逐字相同、清单完整性）；**真键表那几臂**是现读 `src/input.rs` 第 3/5 步核出来的，
⛔ 经 CI 实测。本地另加 `cargo fmt -p yeban-app -- --check` `rc=0`。

**判决**：`pending` —— 交 CI。`git diff --numstat`：`tests/d_ui_26_labelled_chords_are_bound.rs 761/0`
（新判据一枚）＋ `ui/app.slint 1/1`、`ui/transport.slint 8/2`。
⚠ 界面侧改的是 `accessible-label` 的**文本**与三处**注释**：无坐标、无尺寸、无颜色变化 ⇒
**零像素改动**，⛔ 需要金帧重录（`[UI-MCP-003]` 那条不触发）；但它**确实**改变无障碍树里的
`accessible-name`，所以任何按名字断言的既有判据都应当跟着改 —— 实测仓库内按这句 label 断言的
只有本文件（`elements.rs` 的登记名⛔ 带键位，因此它⛔ 是第二处要同步的文本）。

---

## §42 `D-UI-26` 的姊妹族：括号里印出来的**裸键**也是承诺 —— 守卫从 8 条扩到 10 条（2026-11-03）

**为什么这一票要补**：§41 那枚判据只认**带修饰词的和弦**（解析器从修饰词起跳）。同一条"念出来却
什么都不会发生"的缺陷，另一半分长这样：`ui/transport.slint:673` 的 label 写
`切换 Session / Arrangement (Tab, 仅画布聚焦时)` —— 括号里是**一把裸键**。守卫⛔ 覆盖它 ⇒ 有人
把这把键改掉或删掉那一臂，判据一声不响。本票把这一族一并钉住，⛔ 另开一票（同一句承诺、同一张
键表、同一个解析器）。

**现读的四个裸键承诺**（单位：**一次印刷** = 一条 `BarePromise`；`.slint` 与 `elements.rs` 各一份）：
`Space` ×2（`播放 / 暂停 (Space)`）、`F5` ×2（`Session 触发矩阵 (F5)`）、`F6` ×2
（`Arrangement 时间轴 (F6)`）、`Tab` ×2（`切换 Session / Arrangement (Tab, 仅画布聚焦时)`）
⇒ **8 处、4 把**。逐条对 `src/input.rs`：第 1 步 `F5`⇒`ShowView(Session)`、`F6`⇒`ShowView(Arrangement)`
（**全局**，连合成态都放行）；第 6 步裸键 `Space`⇒`PlayPause`、`Tab`⇒ **只在 `Focus::MainCanvas`**
`ToggleView`，其余焦点 `PassThrough` 交给焦点系统 ⇒ 那句 `仅画布聚焦时` 的限定语**是真的**
（本票⛔ 造缺陷，是把已经对的行为钉住）。

**新增两条判据**（⑨/⑩，口径对称）：
* ⑨ 裸键多重集 == 钉值清单（`Space`/`F5`/`F6`/`Tab` 各 2 次），且每把在**画布**焦点下解析出钉住的
  `Action`；
* ⑩ 文案里那句**焦点限定语**必须就是键表的行为，两半分各挡一种空绿：
  带限定语的 ⇒ 在 `Other`（推子/旋钮/标签）与 `TextInput` 下必须**真的⛔ 吞键**（挡"文案说只在画布，
  实现却哪儿都吞"）；⛔ 带限定语的 ⇒ 在 `MainCanvas` **与** `Other` 两条腿上都必须兑现（挡"只验画布，
  于是'哪儿都生效'的承诺被读成'只在画布生效'"）。

**解析口径**（防假红）：只认 `( … )` 括号组里**逗号前第一节**恰好等于词表键名的；组长 >32 字符按散文
处理。⚠ `BARE_KEY_WORDS` 刻意**⛔ 收中文键名** —— `删除` / `空格` / `回车` 在中文文案里首先是**动词**
（`撤销一步`、`删除选中` 这类 label 处处都是），把它们当裸键名会把散文读成承诺 ⇒ 假红；中文键名只在
**和弦**里认（`Cmd+删除` 那种写法才成立）。`Shift+Enter` 这类和弦⛔ 与裸键族重叠（第一节是
`Shift+Enter`，⛔ 在裸键词表里）。

**顺带把 ⑤ 补硬**：原来只要求"**带和弦**的登记名"与 `.slint` 逐字相同；登记名里另外那 4 句裸键提示
同样是对 AI/屏读的承诺 ⇒ 现在 ⑤ 管**全部 7 句**（3 和弦 ＋ 4 裸键），并加了"一句都没扫到 ⇒ 红"
的空集防线（口径：⛔ 让守卫靠空集转正）。

**本机怎么验的**（同 §41 那条边界：本机⛔ 编译 `yeban-app`，判据逻辑搬进零依赖桩 crate 跑）：
**10 条全绿**；且 ⑩ 做了两半分各自的**变异检验** —— 把桩表里 `Tab` 的 `MainCanvas` 门去掉 ⇒ ⑩ 红在
`ui/transport.slint:673 … 键表却在 Other 焦点下也吞了这把键`；把 `Space` 加上 `MainCanvas` 门 ⇒ ⑩ 红在
`ui/transport.slint:123 … 这是对「哪儿都生效」的承诺，但键表在 Other 焦点下解析成 PassThrough`。
⇒ 这两条绿⛔ 是白给的。`cargo fmt -p yeban-app -- --check` `rc=0`。

**判决**：`pending`（交 CI）。`git diff --numstat`：`tests/d_ui_26_labelled_chords_are_bound.rs 236/15`
—— ⛔ `.slint` 改动、⛔ 像素改动，纯判据扩展（文件现为 **10** 条 `#[test]`，§41 里那句"8 条"记的是
当时那一枚提交）。

---

## §43 `D-UI-24` 判决读回：**7 条里 ③ 红**，红在我自己的注入假设上（2026-11-03）

**读数对象**：run `41537282778`（`headSha=26a23966`，即 §39 那一枚提交）。单位：**作业（job）**
与**测试二进制里的 `test result:` 行**（§6.3 第 2、4 条：跳过的作业⛔ 不是绿，判据名缺失 = 没跑到）。

| 作业 | 结论 | 是不是本票的证据 |
| :--- | :--- | :--- |
| `rust (yeban-app)` | **failure** | ✅ 是 —— 目标二进制**跑到了**（日志里有 `Running tests/d_ui_24_bpm_draft_survives_reprojection.rs` ＋ `test result:` 行） |
| `windows (…) / macos (…)` | failure | ✅ 同一条判据、同一行号 |
| `deny (cargo-deny 开源合规)` | failure | ❌ 与本票无关，见 §44 |
| `rust (workspace 全量)` / `e2e (…)` | **skipped** | ⛔ 空绿，不作任何验证 |

`rust (yeban-app)` 里那一条的原文（三条腿逐字一致，仅路径分隔符不同）：

```
running 7 tests
…
test a_document_edit_that_re_projects_does_not_lose_the_draft ... FAILED
thread '…' panicked at crates/yeban-app/tests/d_ui_24_bpm_draft_survives_reprojection.rs:266:5:
预备拍键必须被消费（否则本条没有发生重投影，②的"没变"就成了无源之水）
test result: FAILED. 6 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

⇒ **①②④⑤⑥⑦ 六条拿到了真的绿读数**（含本票的正 面 ② 与两条纪律 ⑥⑦）；红的只有 ③。

### 根因：§39 边界 4 那句"无头注入绕过的是派发那一层"是**错的**

现读链条（三条都是代码事实，⛔ 推测）：

1. `src/host.rs` `wire_input` 的 `on_ime_focus_changed` 把 `focused_now = true` 落成
   `InputContext::set_focus(Focus::TextInput)`；
2. `src/input.rs` `resolve_logical` 的**第 1 步**只放行 `F5`/`F6`，**第 2 步**是
   `if self.focus == Focus::TextInput { return PassThrough }` —— 带 `Cmd`/`Ctrl` 的组合排在
   **第 3 步**，因此焦点在文本框里时**一把也到不了**；
3. `invoke_key_action` 不是旁路：它经 `wire_keys` 调的正是**同一个** `resolve_logical`。
   判据先把焦点置进文本框、再从同一张表要一次"消费"⇒ 必然返回未消费，`Op` 一条也没提交。

⚠ 这条教训的形状正是 `AGENTS.md` §6.3 第 4 条的反面：**我⛔ 是把判据名缺失当成"没跑到"，
而是把一个从未被任何一条腿执行过的假设写进了判据**。本机⛔ 编译 `yeban-app` ⇒ 这条判据从写下到
CI 之间**一次都没有运行过**；它红得准确，红的是我自己那句前提。

### 改法：③ 换到**鼠标**那条腿（同一缺陷在真窗口里的形状）

`mixer-mute-toggle(int)`（调音台通道条的 M 键）→ `host::wire_mixer_switch` →
**恰好一条** `Op::SetTrackMute` → `refresh_undo_window(.., true)` → `apply_view`。它不受键表管，
所以"焦点在框里"这件事⛔ 会把它一起挡掉 —— 而这正是 §39 开头登记的触发面 ①（"画布上点一下"）。

非空转的**现场证据**换了落点：`track-mutes` 那一列**只由 `apply_view` 写**（`src/host.rs` 的
`ui.set_track_mutes(...)` 是唯一写者），所以"那一格翻了"就等于"那一次注入执行到了"，
⛔ 不是"回调在四处早退里返回了所以草稿没被盖"那种假绿。四条断言：文档那一轨翻了 ⇒
界面那一列跟着翻 ⇒ 提交条数 `+1` ⇒ 框里**仍是草稿**。

顺带**新增一条钉值**（单位：**行**）：`track-ids` / `track-mutes` 两列各 `3` 条
（`filled_project()` 的通道条数，与 `tests/mixer_keyboard_switch.rs` 同一钉法）—— 判据按下标索引，
夹具换轨数时必须**先红在这里**，⛔ 让下标悄悄落到另一条轨上。

**判决**：本条改动**待 CI**（本机⛔ 编译）。`git diff --numstat`：
`tests/d_ui_24_bpm_draft_survives_reprojection.rs 81/25`。§39 的修法（`apply_view` 那一句门）
**一位未动** —— 判决⛔ 曾要求改它，红的只有判据 ③ 的注入方式。

## §44 取证：`deny` 那条腿现在是**全仓库红**，`async-executor v1.14.0` 被上游 yank（2026-11-03）

**读数对象**：两条 run 的 `deny (cargo-deny 开源合规)` 作业。单位：**作业结论**。

- 本线 run `41537282778`（`26a23966`）：`advisories FAILED, bans ok, licenses ok, sources ok`；
- `main` 的 run `41542884885`（`61ff8825`，**别的线**刚推的"nightfall v14 落地点补齐"）：
  同一作业同样 `failure`，而它⛔ 含我一行代码。

失败原文（`deny` 自己给的处置建议）：

```
error[yanked]: detected yanked crate (try `cargo update -p async-executor`)
```

依赖链是**现读**的：`async-executor v1.14.0` ← `accesskit_unix v0.22.1` / `zbus v5.19.0`
← `i-slint-backend-winit v1.18.1` ← `slint v1.18.1` ⇒ **Slint 的传递依赖**，本仓库⛔ 直接声明它。

**处置**（⛔ 本线自行改依赖）：

1. 本批三枚提交（`af28382e` / `611b0ee3` / 本条）的 `git diff --name-only` 实测只含
   `.slint` / `.rs` / `.md` ⇒ **零**个 `Cargo.toml` / `Cargo.lock` 改动，红与它们无因果；
2. `Cargo.lock` 是根级共享文件（`AGENTS.md` §5.6 归集成者），且 yanked crate 属**供应链信号**
   （`MUST-GATE` 之一）⇒ 一次 `cargo update -p async-executor` 该由谁来跑、要不要连着
   重验金帧，是**人类/集成者**的决定，留在此处等裁决，⛔ 在本线偷偷 resolve；
3. 因此**本线的 `deny` 腿预期继续红**：读回判决时必须把"这一枚提交的测试腿"与
   "仓库级 `deny` 红"分开记（§6.3 第 2 条：一条红的作业不污染另一条的读数，
   但⛔ 不能再把整批写成绿）。

⚠ 另一处噪声：同一作业日志早期还有一行
`error: override toolchain '1.99.0-x86_64-unknown-linux-musl' is not installed`，
它之后 `deny` 仍然跑出了完整报告 ⇒ 那是该 step 的**工具链回退**，⛔ 是本次失败的原因。

**第三个独立数据点**：本线更早的 run `41525041679`（`D-UI-21` 那一枚提交）现在读回来也是
**只有** `deny` 一条红，测试腿（Linux / Windows / macOS）**全绿** —— 现读非成功作业清单：
`deny` failure ＋ `rust (workspace 全量)` / `e2e` 两条 `skipped`。⇒ "yanked 的传递依赖把
`deny` 腿打红、⛔ 动摇任何测试判决"这条判断有第三例支撑。

## §45 `D-UI-24` ③ 重写后的判决读回：run `41545628710` —— 三条腿同一枚 `E0308`，测试腿**一个二进制都没跑到**（2026-11-03）

**先说指标**：数的是同一作业日志里 `Running tests/` 出现的**次数**（单位：次）与
`test result:` 出现的**行数**（单位：行）—— 二者即"`cargo test` 真的开跑了几批判据"的现场证据
（§6.3 第 2、4 条）。`rust (yeban-app)` 腿实测 **0 次 / 0 行**。

| 作业 | 结论 | 现场 |
|---|---|---|
| `lockfile` / `plan` / `checks` | success | fmt 与红线守卫都过 ⇒ ⛔ 类型层的事 |
| `deny (cargo-deny)` | failure | §44 那条仓库级红，与本判决无因果 |
| `rust (yeban-app)` | failure | **clippy 阶段**退出码 101，`Running tests/` = 0 |
| `windows` / `macos` | failure | 同一条 `E0308`，同一行号 |
| `e2e` / `rust (workspace 全量)` | skipped | 空绿，⛔ 计入任何覆盖 |

**根因**（一枚，三条腿共享）：我在 §43 里改写的 `③` 用了自造的 helper

```rust
fn string_column(model: &slint::ModelRc<String>) -> Vec<String>   // ✗ CI 判的
```

而 `get_track_ids()` 的元素类型是 `slint::SharedString` ⇒ `expected &ModelRc<String>,
found &ModelRc<SharedString>`。

**为什么只有这一条红，而同一段里的 `bool_column` 没事**：`bool_column` 是从
`tests/mixer_keyboard_switch.rs:146` **原样抄**的（类型自带）；`string_column` 是我**发明**的
（凭"界面那一列是字符串"推断签名）。同一文件里"抄来的"与"发明的"并排时，红的是发明的这一条。
本仓库对同一个属性**早有现成读法**：`tests/d_ui_11_serve_header_gestures.rs:412` 的
`track_ids()` 直接 `.iter().map(|id| id.to_string())`，让类型由属性的 getter 自己决定。

**新登记的测量规则**（§5 的推论，⛔ 靠自觉）：本机⛔ 能编译 Slint ⇒ **类型层错误只有 CI 会抓**。
所以任何**新增的"读界面某一列"**，落笔前必须先 `grep` 同一个属性的**已有读法**并照抄；
⛔ 从签名形状推断元素类型。

**对同批另两枚的影响**（如实切开，⛔ 连坐也⛔ 免罪）：`af28382e` / `611b0ee3`（`D-UI-26`）
在本轮**只有"编译过"这一条证据**（clippy 对那两个文件未报错，因为它先把整个 target 图跑完才退出），
**运行时判据一条都没执行** ⇒ 它们的 10 条守卫仍是 `pending`，⛔ 得写成"绿"也⛔ 得写成"红"。

**改法**：签名换成 `&slint::ModelRc<slint::SharedString>`，并把"`get_track_ids()` 返回
`SharedString`、这一条是 CI 教的"写进 helper 的文档注释，防下一轮再猜。
`cargo fmt -p yeban-app -- --check` `rc=0`。`git diff --numstat`：
`tests/d_ui_24_bpm_draft_survives_reprojection.rs 5/2`。判决：`pending`（重跑）。

## §46 取证：`git push | tail` 的退出码是 `tail` 的 —— 一次**假成功推送**被 `git ls-remote` 抓回（2026-11-03）

**事实**（本轮实测）：把推送命令接上 `| tail -5` 后，工具报 **exit code 0**，而 stderr 里其实是

```text
fatal: Failed to connect to github.com port 443 after 133929 ms: Connection timed out
```

管道的退出码默认取**最后一条命令**（这里是 `tail`），于是网络失败被包装成"成功"。

**怎么抓回的**：不采信包装层的判决，去**远端**现读 —— `git ls-remote origin line/zimmer-export-limiter`
仍返回 `26a23966…`（推送前的值）⇒ 本地"已推送"的信念与远端事实不符。随后
⛔ 接管道重跑，并用 `; echo "rc=$?"` 把退出码打在**命令本身**上，真成功才是
`26a23966..82818d20`。

**通用规则**：
1. 推送 / 上传 / 任何"改变远端状态"的命令⛔ 接管道；要截输出就在命令上直接取 `$?`，或 `set -o pipefail`；
2. **判决从远端读回**（`git ls-remote` 的 ref、`gh run list` 的 `headSha`），⛔ 从本地命令的退出码推断；
3. 本轮还叠了一层风险：那条推送是**后台任务**，"completed (exit code 0)" 更容易被当成权威 ⇒
   后台任务的 exit code 只说明**包装脚本**没崩。

⚠ 另两次纯网络失败（`GnuTLS recv error (-110)`、`HTTP2 framing layer error`）是同一原因的
不同表现，绕过方式是 `git -c http.version=HTTP/1.1`；它们**都**被 `ls-remote` 现读判决，⛔ 靠猜。

## §47 `D-UI-27`：状态栏那句**快捷键提示**是一句承诺 —— 守卫落地，并实测出一处能力层过承诺（2026-11-03）

### 一、先把上一轮的判决读回（run `41548239565` = `b5a3084a` 自己的 run）

**先说指标**：与 §45 同两条口径 —— 同一作业日志里 `Running tests/` 的**次数**（单位：次）与
`test result:` 的**行数**（单位：行）。`rust (yeban-app)` 腿实测 **0 次 / 0 行**（与 §45 同型）。

| 作业 | 结论 | 现场 |
|---|---|---|
| `lockfile` / `plan` / `checks` | success | fmt 与红线守卫过 |
| `deny (cargo-deny)` | failure | §44 那条仓库级红，**根因未变**：`error[yanked]: detected yanked crate (try cargo update -p async-executor)`，末行 `advisories FAILED, bans ok, licenses ok, sources ok` |
| `rust (yeban-app)` | failure | **clippy 阶段**退出码 101，唯一的 error 是 `d_ui_26_labelled_chords_are_bound.rs:431` 的 `clippy::manual_pattern_char_comparison` |
| `windows` / `macos` | failure | 同一条 lint |
| `e2e` / `rust (workspace 全量)` | skipped | 空绿 |

**两条分开的读数，⛔ 混成一条**：
1. ⭐ **§45 那枚 `E0308` 确实修好了** —— 本轮日志里**只有这一处** error，且它在 `d_ui_26` 而非
   `d_ui_24`；clippy 会把 target 图跑完再退出，所以"`d_ui_24` 没报错"就是"`d_ui_24` 编译过"的证据。
2. ⛔ **但 `d_ui_24`/`d_ui_26` 的判据仍一条没跑** —— 编译过 ≠ 跑过（§6.3 第 4 条）。
   所以 §39/§41/§42/§43/§45 里的 `pending` **原样保留**，本轮⛔ 能把任何一条写成绿。

**新 lint 为什么本机看不见**：`AGENTS.md` §5 禁止本机编译 Slint ⇒ `cargo clippy -p yeban-app
--all-targets -D warnings` 本机⛔ 跑，`rust 1.99.0` 的新 lint 只有 CI 会报。
**规则（§45 那条的姊妹）**：在测试文件里写**手写字符比较**（`.split(|c: char| c == ',' || …)`、
`.any(|c| c == …)`）之前，直接写成 clippy 要的形式 —— `split([',', '，', '、'])`；
⛔ 指望本机 lint 兜住。**一枚 lint 会打死同腿所有测试二进制**（fail-fast ⇒ 一整批判据零读数），
代价与一枚类型错误同级。

**改法**：`d_ui_26:431` 的手写比较换成 char 数组 pattern（语义等价：同一个 `split`，同一组分隔符）。

### 二、本片：`D-UI-27` 守卫 `tests/d_ui_27_status_bar_tip_promises_are_bound.rs`

**缺口是什么**：`ui/status_bar.slint:132` 的 `tip` 默认值是一句**画在屏幕上的**用户承诺

```text
空格 播放/暂停 · Tab 切视图 (画布聚焦时) · 1–5 工具 · [ ] A/B 盲听
```

`app.slint` 的 `StatusBar {` 实例⛔ 传 `tip`、`host.rs` 的 `set_tip(` 命中 **0 处**（现读）
⇒ 用户看到的逐字就是这个字面量。而 `D-UI-26` 的守卫只认两种形状（**带修饰的和弦**、
**括号里的裸键**），这句话里 **四段承诺一段都不是**那两种：中文键名 `空格`、括号里装**限定语**
⛔ 键名的 `Tab … (画布聚焦时)`、数字区间 `1–5`（连接号是 en-dash `U+2013`）、方括号键 `[ ]`。
⇒ 承诺与键表的一致性今天**零机械防线**：顺手加一把 `Q 量化` 就是第二个"念出来却什么都不发生"。

**八条判据**：① 清点（段数 4 / 承诺 9 把 / 词表不认识的段 0，身份串＝写法+键+原话+限定语四元组逐字对账）
② 兑现·画布 ③ **限定语双向对账（读解析出来的文案，⛔ 读钉死表）** ④ 文本域口径（现状 `PassThrough`，
⛔ 算兑现，`ADR-0067` 未决）⑤ 画的==扫的（`text: root.tip;` 在 + 实例⛔ 传 `tip:` + 宿主 `set_tip(` = 0）
⑥ 一段⛔ 两个键名来源 ⑦ 语义 ID `status-bar-shortcut-tip` 仍在登记面 ⑧ **能力层读数**（见下）。

### 三、实测出来的过承诺：`[ ]` 两把键**已绑、无落地实现**（⑧ 把它从注释升级成判据名）

`host::action_has_implementation` 今天只把 `AuditionMain` / `AuditionProposal` 判 `false`
（其余承诺全 `true`）—— `[ARCH-RT-005]` 要求的"主线与提案并发渲染 + 30ms 等功率瞬切 + 2048 采样预滚"
在仓库里一处都不存在 ⇒ 按 `[` 今天唯一能发生的事是⛔ 任何东西。这与 `D-UI-26` 的 `Cmd/Ctrl+\`
**同族而低一层**：那把键⛔ 绑（修法是删文案），这两把**绑了却死**。

⛔ 本切片自行修：两个方向都归人类 —— (A) 补 A/B 试听能力，或 (B) 删 `· [ ] A/B 盲听` 那一段
（**用户可见**文本 ⇒ `tests/golden/linux/` 基准过期 ⇒ `gates-manual.yml` 的 `gate=goldens` + 人工复核，`HD-56`）。
本片做的是**把它钉成读数**：⑧ 断言"死键集合 == `{'[', ']'}`"，两边任何一边动都会红
（实现→⑧ 红，逼着回来升级承诺并重录基准；顺手删文案→⑧ 的另一半与 ① 一起红）。

### 四、桩方法与六条变异检验的读数（本机拿得到的最强证据）

`src/input.rs` 本身是**零依赖状态机**（文件里 `use` 语句 **0 条**）⇒ 本机把它原样搬进
`/tmp/d27check`（`input.rs` 截到内部 `#[cfg(test)]` 之前，共 1320 行；`host.rs` 桩 = 逐字抄
`action_has_implementation` 的函数体；四个 `include_str!` 指向真文件的**副本**），
`cargo test` **真跑八条判据**：

- 基线：**8 passed / 0 failed**。
- M1 删掉文案里的 `(画布聚焦时)` ⇒ **③ + ① 红**（6 passed / 2 failed）。
  ⚠ 这一条改写了本票的设计：③ 最初读**钉死表**里的限定语，M1 之下它**绿着说谎**；
  改成读**解析出来的**文案后 M1 才按承诺的方式红。**"文案↔键表"的对账必须读文案**。
- M2 加一段 `· Q 量化` ⇒ **① + ③ + ⑥ 红**（5 passed / 3 failed；③ 是先拒绝在未解析文本上推理）。
- M3 `app.slint` 的实例开始传 `tip:` ⇒ **⑤ 红**。
- M4 把桩改成"全 `true`"（模拟 A/B 被实现）⇒ **⑧ 红**。
- M5 宿主里加一处 `window.set_tip(…)` ⇒ **⑤ 红**（`assertion left==right failed: … 现读 1 处`）。
- M6 从 `elements.rs` 抹掉 `status-bar-shortcut-tip` ⇒ **⑦ 红**。

⛔ 把桩的绿当判决：桩里 `yeban_app` 被换成 `d27check`，是**另一份编译单元**；键表/宿主 API
的签名漂移只有 CI 看得见（§45 的 `SharedString` 就是它教的）。桩只证**本文件逻辑与断言方向**。

### 五、顺手关掉一条误登记

上一轮的"候选 D-UI-27：顶栏「提交」按钮与「分支」格有标签无点击源"**⛔ 是本线的活** ——
它已被 `docs/adr/ADR-0052-actionless-drawn-controls-disposition.md`（`Proposed`）登记，
覆盖 `ai-rail-intent-button` / `transport-commit-button` / `transport-branch-button` 三格
（各自缺的层不同：无产品入口 / 无暂存编辑态 / 无域级切分支动作面，而模型侧
`CommitGraph::fork_anonymous`、`create_branch` 早已存在）。编号因此让给本票（状态栏提示守卫），
候选项在任务表里改判为"**已登记，待裁 ⛔ 实现**"。

`cargo fmt -p yeban-app -- --check` `rc=0`（整个 crate 干净）。判决：`pending`（交 CI）。

---

## §48 判决读回：run `41550904195` —— **一枚 clippy lint 吃掉三条腿**，于是把 CI 同款 `clippy 1.99` 搬进本机零依赖桩（2026-11-03）

### 一、读数（§6.3 的口径：先数作业真跑了什么）

指标：run `41550904195`（`headSha 6167f3b2`）各作业的 `conclusion`，与 `rust (yeban-app)` 日志里
`Running tests/` 的**次数**（单位：次）和 `test result:` 的行数（单位：行）。

| 作业 | conclusion | 实跑测试二进制数 |
| :--- | :--- | ---: |
| `plan` / `checks (fmt / 红线守卫 / schema)` / `lockfile` | success | — |
| `rust (yeban-app)` | **failure** | **0** |
| `windows (… / yeban-app / …)` | **failure** | 0 |
| `macos (… / yeban-app / …)` | **failure** | 0 |
| `deny (cargo-deny)` | failure | —— 与本章无关，仍是 §44 那个被 `yank` 的 `async-executor` |
| `e2e` / `rust (workspace 全量)` | skipped | —— ⛔ 是绿（§6.3 ②） |

日志里的原文（三条腿逐字同判）：

```text
error: the loop variable `index` is only used to index `chars`
   --> crates/yeban-app/tests/d_ui_27_status_bar_tip_promises_are_bound.rs:313:18
   = note: `-D clippy::needless-range-loop` implied by `-D clippy::all`
error: could not compile `yeban-app` (test "d_ui_27_status_bar_tip_promises_are_bound") due to 1 previous error
##[error]Process completed with exit code 101.
```

⇒ `Running tests/` = **0**、`test result:` = **0**：**`D-UI-24` / `D-UI-26` / `D-UI-27` 到今天仍然一条运行时读数都没有**（连续第三个 run）。
`checks` 腿的 `cargo fmt --check` 是绿的 ⇒ 这⛔ 是格式问题，是 **lint**（`clippy::needless_range_loop`，
`for index in open..chars.len()` 只为索引 `chars`）。修法就是 §47 那条改写：`windows()/position()`
定位 + `for &ch in &chars[open..]`。

### 二、真正的教训：**一枚 lint ⇒ 三条腿 ⇒ 整批零读数**，而 lint 本机⛔ 看不见？—— 看得见的

`§47` 已经记过"手写 char 比较会招 `manual_pattern_char_comparison`"，本票又踩第二枚。两次都是
**同一个通用 lint 家族**，都⛔ 需要 Slint，都本机看不见 —— 因为我只在 `stable`（`clippy 0.1.96`）上跑过，
而 CI 是 **`rust-1.99.0`**。实测本机 `rustup` 里**装着 1.99.0 工具链**：

```console
$ cargo +1.99.0 clippy --version
clippy 0.1.99 (b940084d7e 2026-09-28)
```

⇒ **本机就能跑 CI 同款 lint**。桩的做法（§47 的桩，这次改成**脚本可复现**，`/tmp/dcheck`）：

1. `src/input.rs` = 真源码截到内部 `#[cfg(test)]` 之前（1319 行，逐字）；
2. `src/host.rs` = 从 `src/host.rs:9164` 起的 `action_has_implementation`，尾部用**第一个顶格 `}`** 定位
   （⚠ 别按花括号计数：函数体注释里成对的 `{`/`}` 会把终点推到 **+820 行**，实测把整段宿主塞进桩里换来满屏 `E0432`）；
3. 把 `crates/yeban-app/ui/**/*.slint` 整个复制进桩（`include_str!("../ui/…")` 原样可解析），
   `src/{host,elements}.rs` 复制进 `fixtures/`；
4. 测试副本里 `use yeban_app::` → `use dcheck::`，`read_dir("ui")` → `read_dir(env!("CARGO_MANIFEST_DIR"))`。

**先核验这台机器真的能抓 lint**（⛔ 拿"没报错"当"通过"）：把 §47 的旧写法原样塞回桩再跑 ——

```text
error: the loop variable `index` is only used to index `chars`
error: could not compile `dcheck` (test "d_ui_27") due to 1 previous error
```

同一条 lint、同一条信息（`rust-1.96.0` 的 index 页与 CI 的 `rust-1.99.0` 一致）⇒ 预检器**有效**。
改回后：`cargo +1.99.0 clippy --tests --all-targets -- -D warnings` 干净，
`cargo +1.99.0 test` **`d_ui_26` 10/10、`d_ui_27` 8/8 真的执行了**。

⛔ 把桩的绿当判决（§47 同一句）：桩是另一份编译单元，`SharedString` 那类签名漂移只有 CI 看得见；
桩这次覆盖的是**通用 lint + 本文件逻辑** —— 而这两样正是连吃三个 run 的东西。

### 三、新到的工作纪律（写下来，⛔ 靠自觉）

> **任何新增/改动的 `.rs` 测试文件，若能塞进零依赖桩，就先在桩里跑 `cargo +1.99.0 clippy --tests -D warnings` 再推。**
> 塞不进去的（`d_ui_21/23/24` 那类要 `bridge/ui/undo/scene` 的），⛔ 本机预检 ⇒ 推之前**逐条自查 lint 面**：
> `for i in 0..v.len()` 只为索引、`.split(|c: char| c == …)`、`&v[..]` 手抄切片这类形状一律改写成迭代器形式。

`cargo fmt -p yeban-app -- --check` `rc=0`。判决：本条改写交 CI 读回（`pending`）。
