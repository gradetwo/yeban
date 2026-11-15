# ADR-0074 — `未修复问题清单-0f181bd.md` 复核裁决：双层键路与待实现项（Proposed）

- **状态**: **Proposed（待人类批准）** — 依 `AGENTS.md §5.7`，本 ADR 登记
  「规范缺口 / 两套事实源冲突」的复核证据与建议裁决，**不擅自选定语义、
  不私自发明答案**。
- **决定者**: 待人类责任人裁定
- **关联**: `~/work/qoder/未修复问题清单-0f181bd.md`（2026-11-16，基线 `0f181bd`）；
  `ADR-0073`（report-3 ROI 裁决）；规范 `AGENTS.md §5.7/§6.3/§6.4`；
  代码事实源 `crates/yeban-app/ui/app.slint`、`src/input.rs`、`src/host.rs`、
  `src/cli.rs`、`src/live_surface.rs`
- **日期**: 2026-11-16

## 背景

`未修复问题清单-0f181bd.md` 是别人对 `0f181bd` 的一份逐控件复核，列出
P0×3 / P1×8 / P2×3 / P3×3。当前 tip 已 rebase 到 `origin/main`（`bb5e479e`，
tree 与 `0f181bd` 一致），故清单的代码事实对当前 tip 成立。本轮逐条复核后
分三类：**确定性可修（已落地）**、**双层键路语义冲突（需裁决）**、
**能力缺失 / 环境限制（需立项或真机）**。

### 根因：键盘存在两套事实源

当前代码里同一个物理键被**两层各绑一次**，且没有仲裁：

1. `ui/app.slint:840` 的 `FocusScope.key-pressed` **无条件最先**调用
   `root.trigger-shortcut(event.text, event)`；
2. `src/host.rs:19036` 的 `on_trigger_shortcut` → `ui.invoke_key_action(...)`
   → `src/input.rs` 解析表（`--print-shortcuts` 主表 = 这一层的"官方口径"）；
3. 回到 `app.slint:841-936`，同一个 key-handler **又**按 `event.text` 跑一遍
   界面直达分支并 `return accept`。

于是解析表消费过的键，界面层**再补做一次**。此外 `src/host.rs:17077`
（`wire_aether_callbacks`）与 `:19036`（`wire_aether_actions`）**两次**给
`ui.on_trigger_shortcut(...)` 赋值；Slint 后写覆盖前写 ⇒ 17077 的
「空格 / r / c / m / 1-4 / b 白名单」处理器是**死代码**（清单 P2-1）。

清单点名的三条后果（当前代码逐字复核一致）：

| 键 | 解析表 (`input.rs`) | 界面层 (`app.slint`) | 净效果 |
|---|---|---|---|
| `Ctrl+T` | `:1344 Character('t') → SplitAtPlayhead` | `:877-880 track-add-requested()` | 切一刀 **且** 加一轨（P0-3） |
| `Tab` | `:1451 → ToggleView` | `:850-854 show-clip-matrix 取反 + zone-toggle("clipmatrix")` | 换主视图 **且** 开 Clip 矩阵（P1-1） |
| `Ctrl+Alt+M` | `:1312 → ToggleConsoleMaximize` | `:885-888 console-maximize-toggle()` | 最大化→立刻还原，净零（P1-2） |

⚠ **语义冲突的核心**：`app.slint:874-876` 的注释明确写「Ctrl/Cmd+T
单一语义 = 新建轨道 …… 界面这一路是 Ctrl+T 的唯一入口」，而 `input.rs:1344`
把它绑成 `SplitAtPlayhead`，且 `logic_pro_workflow_shortcuts_resolve_correctly`
判据把这条解析钉死。两份文档各写一半，互相矛盾。**哪一层是唯一事实源，
是产品/规范裁决，不是 Agent 可私自决定的**（§5.7）。

## 决定（本轮实际落地）

### D1 — 四项零语义裁量修复（已在 `line/ui-quickfix` 落地，CI 待判）

| 清单项 | 修复 |
|---|---|
| P0-2 EQ CURVE 空框 | 接上**已存在**的真实数据 `MainWindow.eq-curve-svg`（`bridge::calculate_eq_curve_svg` → `host.rs:540/4600` → `device_rack.slint:568` 同一路）。`Path` 仅在曲线 ≠ 缺省 `"M 0 50 L 420 50"` 时 `visible`，0dB 基准线 `Rectangle` 保留 ⇒ **缺省帧逐像素不变，不动 5 张 Linux golden**。已接真实数据故去掉「示意」标注 |
| P1-3 chip 尾字被吃 | `和弦·收起/展开`、`轨道·菜单` 去掉会在窄框被截断的中点 `·` |
| P1-4 波形长句被裁成「波形」 | 占位文案改 `visible: parent.width >= 88px`，窄片段不再显示半截误导文案；完整语义进 `accessible-label` |
| P2-2 键位表两段式读不出覆盖 | `--print-shortcuts` 明确列出「Tab / Ctrl+T / Ctrl+Alt+M 双层都绑」的警示 |

配套：`ui_display_honesty.rs` 判据 ②/⑤ 随演进同步更新，新增
`ui_unfixed_list_regression.rs` 5 条源码层判据（含 golden 保护断言）。

⛔ BUS COMP / 机架卡片 / 通道条 EQ 三处**仍未接**真实参数（无对应投影属性），
继续保持「示意」标注，不臆造曲线。

### D2 — 双层键路冲突：登记三条候选路线，不代选

| 路线 | 做法 | 代价 / 风险 |
|---|---|---|
| **R1（清单"根治建议"）** | `trigger-shortcut` 改 `-> bool`，`on_trigger_shortcut` 透出 `invoke_key_action` 的 consumed 返回值，`app.slint:840` 改 `if (root.trigger-shortcut(...)) { return accept; }` —— 解析表消费掉的键界面层不再补做 | ⚠ **改变可见语义**：Ctrl+T 从「加轨」变「切分」（与 `app.slint:874` 注释及前几轮修复结论相反）、Tab 从「开 Clip 矩阵」变「换主视图」（菜单「视图 → 显隐 Clip 激发矩阵 (Tab)」随即名不副实）。需同步改注释、菜单文案、`input.rs` 判据，并重录受影响的 golden |
| **R2（界面层唯一）** | 移除/合并 `host.rs:19036` 的 `on_trigger_shortcut`，让 `app.slint` key-handler 成为唯一入口；解析表只作为 `--print-shortcuts` 的文档 | ⚠ 推翻 `--print-shortcuts` 作为官方口径的现状；`ui/dispatch_key_chord` → `invoke_key_action` 的测试链路会失去生产对应物；`input.rs` 大量判据失效 |
| **R3（维持现状 + 显式标注）** | 不改行为，把三条双绑键写进键位表（= D1 的 P2-2），并把 17077 的死代码删除或标注为"故意未接线" | 零行为风险，但 P0-3/P1-1/P1-2 仍是缺陷；仅适合作为裁决前的临时态 |

关联硬约束（清单 P2-3，当前代码复核一致）：
- `ui/dispatch_key_chord` → `live_surface.rs:1289-1298` `invoke_key_action`，
  **只跑解析表，不经 `app.slint:839` key-handler**；
- `ui/dispatch_key_press` → 真平台 `WindowEvent::KeyPressed`，但 Slint 1.18.1
  的 `KeyPressed` **无修饰位字段**。
⇒ 「带修饰位的平台事件」在现有测试端口**结构上不可表达**，所以
Ctrl+T / Tab / Ctrl+Alt+M 这类双层缺陷**无法被当前判据测到**。补法（给测试后端
加 `KeyChordModifiers` 或让 `dispatch_key_press` 收修饰位）是独立能力切片，
不在本轮。

### D3 — 其余项：登记，不臆造

| 清单项 | 判定 | 建议裁决 / 后续 |
|---|---|---|
| P0-1 音频波形无真实峰值 | **能力缺失**（非假显示）：`arrangement.slint:1156-1185` 注释自证「按 block 的离线分析缓存尚未接入 timeline-blocks 投影」 | 立项接入 peak 分析缓存 → 投影属性 → 波形绘制；需真机素材与 golden 重录 |
| P1-5 复制/粘贴/剪切 | `app.slint` 0 处、`host.rs` 无对应命令臂 | 需定义剪贴板模型 + 选中片段/音符语义，独立切片 |
| P1-6 菜单无「退出」 | `app.slint` 0 处 | 需 `Window.request_close` 接线 + 未保存确认策略裁决 |
| P1-7 导出分轨仅 CLI | 文案已诚实标注「请用 CLI」 | GUI 入口需文件对话框族（与 `ADR-0073` Q2 的 7 个 `wire_*` 同源），独立切片 |
| P1-8 预备拍无菜单入口 | `Ctrl+Alt+D → ToggleCountIn` 已在解析表，菜单 7 组无对应项 | 补一个 `command-execute` 臂 + 菜单项即可（低风险，但需确认 transport 状态投影） |
| P2-1 死代码 `on_trigger_shortcut` | 见 D2 | 随 R1/R2/R3 一并处置；若 R3 则至少删除或标注 |
| P3-1 截图白底 / 底栏消失 | **未定论**：同尺寸两次抓取内容区不同（1345×605 vs 1444×699），`pane_no_overlap` 18 条断言全绿 | 需真机前台人眼观察，⛔ 不当缺陷上报也⛔ 不忽略 |
| P3-2 窄片段名 elide | 设计选择 | 低优先打磨 |
| P3-3 约 130 个测试目标/真人注入未覆盖 | 环境限制 | CI 已真跑 report-3 §1 的 14 个目标（`43744117695`，0 FAILED）；其余按 CI 覆盖，不臆断 |

## 建议的人类裁决点

1. **D2 选 R1 / R2 / R3**：核心是「Ctrl+T 到底是加轨还是切分」「Tab 是换视图
   还是开 Clip 矩阵」。选定后由实现票统一 `input.rs` / `app.slint` / 菜单文案 /
   `--print-shortcuts` / 判据，并评估 golden 重录。
2. **P0-1 波形峰值**是否立项（涉及离线分析 + 投影 + 视觉回归）。
3. **P1-5/6/7/8** 各自是否立项，以及剪贴板语义、退出确认策略。
4. **P2-3 测试端口修饰位**是否立项（决定双层键缺陷以后能否被自动测到）。

## 后果

- 正向：清单里 4 项确定性缺陷已修且有回归判据；双层键路的**冲突本身**被
  显式登记，避免后续 Agent 按任一单方面注释"修一半"造成新的不一致；
  `--print-shortcuts` 不再隐瞒三条双绑键。
- 负向 / 限制：P0-3 / P1-1 / P1-2 / P2-1 **本轮未改行为**（那是语义裁决）；
  P0-1 / P1-5/6/7/8 / P3 未实现。它们需要人类裁决、独立能力切片或真机验证，
  不能靠本机源码扫描臆造。
