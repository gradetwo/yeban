# `mixer-horizontal-scroll` 工作线台账 —— 调音台通道条的横向可达（N-UI-1 的混音面）

- **台账类型**：交付映射 / 判据清单 / 边界与未决（**不是规范**）
- **工作线**：`line/mixer-horizontal-scroll`（worktree `yeban/.worktrees/mixer-horizontal-scroll`，基线 origin/main `b008fd49`）
- **所有者目录**：`crates/yeban-app/ui/console/mixer_console.slint` + `crates/yeban-app/tests/mixer_horizontal_scroll.rs` + 本文件
- **本机纪律**：只跑了 `rustfmt --edition 2024`（新测试文件与 `.slint` 注释）；**没有**在本机编译 Slint
  —— 构建 / clippy / 判据全部交给 GitHub Actions。
- **规范来源**：`[UI-TEST-001..003]`（语义 ID 寻址 / 事件注入 / 无头运行）、`[UI-GRID-002]`（响应式断点）、
  `[ARCH-UI-005]`（无头控件树内省）、`[ROAD-M3-002]`（滚动手势）
- **缺口来源**：负责人报告 `~/zimmer-ui-mode-report.md` §5.1（N-UI-1）与 §7（混音台），非 MCP 部分

---

## 0. 一句话

改之前：通道条按 `8 + 104px * i` 平铺、三块侧面板的位置也跟着轨道数 `n` 走，**没有任何横向容器**
⇒ 轨道一多，第 K 条之后的通道条与**全部侧面板**冲出窗口，既看不到也点不到。
报告在 1440 档实测"树里只有 `mixer-master-*` 加 `track-0`..`track-13`"，并留下一句
**"第 14 条以后的混音通道对真实用户是否可达仍未验证"** —— 本票把这条"仍未验证"变成判据。

改之后：横向流程里的全部（导出 MIDI 面板 / 打开工程面板 / 素材池 + 通道条 `for` 循环）包进一个
`Flickable`，`content-width` 由宿主同一组几何常量给出；主控通道条留在容器**外面**（钉在
`root.width - 112`，属于视口而不是内容）。演示工程（6 轨）内容不溢出 ⇒ 偏移恒 0 ⇒ 默认帧、
golden、`tests/pane_no_overlap.rs` 的 73 控件预算逐位不变。

---

## 1. 缺陷的算术（报告读数与本票夹具）

| 量 | 口径 | 1440 档 / 15 轨夹具 |
| :--- | :--- | :--- |
| 通道条左沿 | `8 + 104 × i` | 第 14 条起于 `8 + 104×14 = 1464`（报告写的"1456"是**不含 `space-4`** 的 `104×14`） |
| 滚动视口宽 | `混音台宽 − 112` | `1440 − 112 = 1328` |
| 内容宽 | `8 + 104n + (宽档 438 / 窄档 216)` | `8 + 1560 + 438 = 2006` |
| 最大偏移 | `内容宽 − 视口宽` | `678` ⇒ 单次滚动 400 落在区间内（判据⑦把它钉成前置条件） |

顺带修掉的边角：此前最右的通道条会画到主控条**右侧**露出一小截（14 轨档下第 13 条跨 1432px 边界，
靠主控条自己不透明才没盖住它）。视口右缘收在 `parent.width - 112px` 之后，这类"从主控条右边探出来"
的像素不再存在。

## 2. 改了什么

- `crates/yeban-app/ui/console/mixer_console.slint`：新增横向 `Flickable` 容器（原 261..894 行整体缩进
  进容器；文件 1144 → 1173 行，花括号配平已机械核对：容器深度 1、主控条 `Rectangle` 深度 1）。
  **没有**改任何既有控件的 x/y/宽/高表达式，也没有新增语义 ID。
- `crates/yeban-app/tests/mixer_horizontal_scroll.rs`：新增判据文件（5 个 `#[test]`）。
- 本文件。

`content-*` 而不是 `viewport-*`：后者是 `@deprecated` 别名，而上游 `flickable` 编译 pass 跑在
`remove_aliases` **之前**，它按名字检查 `content-width` 有没有显式绑定来决定是否自动生成内容宽
（自动值 = 容器自身高/宽）；绑定挂在别名上时它读不到，于是装上自动绑定，我们的显式值在合并时
被丢弃 ⇒ 内容宽恒等于视口宽 ⇒ **滚轮被静默忽略并转发给父级**。这条在编排行区那一票红里实测过，
留痕 `docs/ledger/app-zimmer-ui-notes.md`（run `41208446082`）。

## 3. 判据映射

判据全部从**运行时控件树的快照交集**读，⛔ 不假设控制台在窗口原点、⛔ 不硬编码视口宽。

| 判据 | `#[test]` | 证的缺口 |
| :--- | :--- | :--- |
| ① + ② | `wheel_scroll_moves_strips_left_and_reveals_later_tracks` | 滚动前末条不可寻址；向右滚 400 ⇒ 交集里每条恰好左移 400 且更大序号进入树；滚回 400 ⇒ 快照逐位复原 |
| ③ + ④ | `scrolling_to_the_end_clamps_and_reveals_the_last_strip_and_the_panels` | 滚到底 ⇒ 末条通道与其**推子**可达、导出 MIDI 按钮回到视口、首条被裁出；再滚一次逐位不变（夹紧） |
| ⑤ | `vertical_wheel_does_not_scroll_the_console` | `content-height: self.height` ⇒ 纵向 delta 被上游拒收，混音台一位不动 |
| ⑥ | `scrolling_does_not_move_the_master_strip` | 主控通道条在容器外 ⇒ 起点/滚动后/到底时 x 逐位相同，且未滚动时恰为 `root.width − 112` |
| ⑦ | `Rig::assert_overflows`（前三条的前置） | 内容宽高出视口且最大偏移 > 单次滚动量 —— 没有它，滚轮语义根本不成立（防伪：把"注入断了"与"容器没接住滚轮"分得开） |
| ⑧ | `few_tracks_do_not_scroll` | 演示工程 6 轨内容不溢出 ⇒ 偏移恒 0，默认帧不漂移 |

符号：上游把 `content-x` 夹在 `[视口宽 − 内容宽, 0]` 且 `new_pos = current + delta` **不取反**
⇒ 静止位上正的 `delta_x` 被夹回 0。判据因此用 `scroll_strips_right`（送**负** delta）/
`scroll_strips_left` 两个具名助手把符号收在一处。

## 4. 边界（不许当成已证）

- ⛔ **没有滚动条**：裸 `Flickable` 不生成滚动条（上游只有 `ScrollView` 会加），本片只接滚轮。
  产品面是否需要可见的横向滚动条 / 通道条计数，待负责人裁决。
- ⛔ **普通鼠标滚轮横滚不了**：判据⑤证的恰恰是"纵向不动混音台"。横滚需要触控板的 `delta_x`，
  或非 macOS 下按住 Shift 的滚轮（`items/flickable.rs` 的 `scroll_delta` 仅在 Shift 且非 macOS
  时交换两轴）。**键盘入口（如 `PageUp/PageDown` 或 `[`/`]` 跳通道）本片没做**，也未曾登记为已做。
- ⛔ **触摸拖动平移未证**：`interactive` / `mouse-drag-pan-enabled` 都用上游默认值（两者默认 `true`）。
  内容不溢出时拖动是恒 no-op ⇒ 既有夹具的指针手势一位不变；溢出后"在空白处横拖 = 平移"是
  期望行为，但判据没有覆盖触摸/鼠标拖动手势。
- ⛔ **不改宿主注入**：调音台的横向偏移由容器自己持有（与编排行区的 `sheet-height` 注入不同），
  因此本片不新增 `MainWindow` 属性。副作用：MCP 面读不到"混音台滚到哪了"—— 若要把它纳入
  `[MCP-DUAL-001]` 的对照面，需要另开一票。
- ⚠ **判据②的"逐位复原"依赖取消相位的直接 set**：上游 `PointerScrolled` 走 `TouchPhase::Cancelled`
  ⇒ 无动画、下一次抓树即可读几何。若上游改成带动量的 `flicked`，交集位移仍成立但"逐位复原"要重核。

## 5. 判决状态

| 提交 | 内容 | CI run | 判决 |
| :--- | :--- | :--- | :--- |
| `73a2db88` | `.slint` 容器 + 判据文件 | `41250324373` | **绿**（读回于 2026-10-31）：`rust (yeban-app)`（job `124682358510`）`Running tests/` = **274**，`mixer_horizontal_scroll` 二进制在其中，5 条具名判据各 **2 档 ok**，`test result: ok. 5 passed; 0 failed`；`macos` / `windows` 两腿同为 success，deny / checks / plan / lockfile 全绿 ⇒ 非空绿 |

按 `[AGENTS.md §6.3]` 的口径：判绿必须确认 `rust (yeban-app)` / `macos` / `windows` 三条腿里
`Running tests/` 覆盖了 `mixer_horizontal_scroll` 这个二进制，并找到 5 条具名 `test … ok`；
编译期红（如上一票的 `E0277`）只能记作"没跑到"，不得记作判据失败或通过。
