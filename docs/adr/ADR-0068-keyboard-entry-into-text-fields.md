# ADR-0068 — 从画布进入文本框的键位未定（缺口 D-UI-22：`Tab` 的两种规范互相冲突）

- **状态**: **Proposed（待人类批准；⛔ Agent 不代签 —— 本票零代码，也没有待撤回的补丁）**
  —— 三条路线都能实现，但它们**互不相同地**违反一份规范：选哪一条是产品裁决，
  不是工程裁决。本票只把冲突、代价与既有成例摆出来。
- **决定者**: 待人类裁决
- **关联**: 缺口 `D-UI-22`（`docs/ledger/app-zimmer-ui-notes.md` §31 登记的"文本框只能靠鼠标点"，
  本票给它编号）；已闭合的姊妹票 `D-UI-18`（左栏搜索框：走的是**专属 chord**，即下面 D1 的成例）、
  `D-UI-19` / `D-UI-20`（导出弹窗：走的是**宿主打开时那一次程序性取焦** ＋ 模态内自建轮转）；
  `crates/yeban-app/ui/app.slint:725-726`（`forward-focus: key-handler` ＋ `key-handler := FocusScope`）、
  `app.slint:716`（注释逐字："accept ⇒ Slint 也不做 `Tab` 焦点轮转"）、
  `crates/yeban-app/src/input.rs:702`（`Action::ToggleView`）、`input.rs:1262`（画布持焦时 `Tab` 解析成它）、
  `input.rs:924`（该变体的文档逐字写明这个事实）、`input.rs:1402`（既有判据"删掉 TextInput 分支会变红"）、
  `crates/yeban-app/ui/app.slint:2237`（`focus-sidebar-search()`，D1 的成例）与
  `app.slint:2247-2251`（时光机那一对 `undo_tree_trap.focus()` / `key-handler.focus()`，程序性取焦的第二个成例）、
  `crates/yeban-app/ui/transport.slint` 的 `bpm-input`（今天**没有**任何键盘入口，见下"证据边界"）
- **日期**: 2026-11-03

## 背景（全部现读，⛔ 不采信报告措辞）

两份规范对**同一个键**说了**两句相反的话**：

1. `[UI-A11Y-001]` 的键位表（`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md:300`）：
   > **Tab** ｜ 视图切换 ｜ Session 触发矩阵 与 Arrangement 时间轴切换 ｜ **仅在主工作区聚焦时响应**
2. `[UI-A11Y-003]` §7.3 第 1 条（同文件 `:326`）：
   > 全界面焦点流转**遵循严密的 Tab 序列**，弹窗与模态对话框内自动实现焦点闭环（Focus Trap），
   > 关闭时焦点精准复位至触发控件
   并且 §7.3 第 4 条把"**模拟 Tab 焦点轮转**并断言焦点顺序"写成 CI **MUST** 用例。

代码今天按 **(1)** 办：焦点落在快捷键层（窗口建立时 `forward-focus: key-handler` 就落在这里）时，
`key-pressed` 把这一键交给 `input::resolve_logical`，那里解析成 `Action::ToggleView` ⇒
返回 `accept` ⇒ Slint 的**内建 `Tab` 轮转根本不发生**（`app.slint:716` 的注释逐字如此）。

⇒ **纯键盘用户在画布上没有任何入口进入文本框**。§31 的链条今天仍成立，只是范围小了：

| 敲入控件 | 键盘可达？ | 靠什么 |
| :--- | :--- | :--- |
| `sidebar-search-input` | ✅ | `D-UI-18` 的专属 chord（`Cmd/Ctrl+F`）→ `app.slint:2237` 的 `public function` |
| 导出弹窗四框 ＋ 三格按钮 | ✅ | `D-UI-19/20`：宿主打开时程序性取焦，模态内自建七格轮转 |
| 时光机模态 | ✅ | `app.slint:2247-2251` 那一对程序性取焦成例 |
| **`bpm-input`（`transport.slint`）** | ⛔ **仅鼠标** | 全仓 grep `focus.*bpm` 在 `.slint` / `src/` 侧**只有** `test_port_adapter.rs:2576` 一条注释 ⇒ **没有键盘入口** |
| **混音面板的采样器路径框等 `device-*` 文本框** | ⛔ 仅鼠标 | 同上：没有任何 `public function focus-…` 指向它们 |

⚠ **证据边界（§6.4）**：`bpm-input` 的"摸不到"是**代码读出来的**（没有取焦入口、`Tab` 被吃掉），
本轮**没有**开窗复现"按 `Tab` 后 `bpm-input` 的 `has-focus` 仍为 `false`"这条判据。
若批准 D1/D2/D3 任一档，实现票必须**先**补这条活体判据，⛔ 不许从这张表直接宣布缺陷已证。

⚠ 规范地位：`[UI-A11Y-001]` 那张表出自 `YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md`，
该文档于 **2026-10-06 经负责人裁决自 Normative 降级为 Advisory**（`AGENTS.md` §1 第 3 条）。
⇒ 两句冲突里，**advisory 的那句不是硬约束**，但 `Tab`=`ToggleView` 已经被
`input.rs:1402` 那条**既有判据**钉成事实 —— 改键位是**行为变更**，会牵动快捷键策略表与它的判据。

## 三条路线

### D1 — 每个敲入控件配一条专属 chord（`D-UI-18` 的成例推广）

给 `bpm-input`（以及后续每个 `device-*` 文本框）加一条宿主侧 `public function focus-xxx()`，
由快捷键策略表里**新增的一条 chord** 触发（例：`Cmd/Ctrl+B` 之类未占位）。

* ✅ **零规范冲突**：⛔ 不动 `Tab` 的画布语义 ⇒ `input.rs:1402` 与 `[UI-A11Y-001]` 那句都原样成立。
* ✅ 与仓库既有成例逐字同款（`D-UI-18` 就是这么做的，判据 `tests/search_focus_key.rs` 已判绿）。
* ✅ 实现面小：一条 `Action` 变体 ＋ 一条 `.slint` 函数 ＋ 一条判据。
* ⚠ **代价**：可聚焦控件有多少个，chord 就要占多少条 —— 键盘地图会越长越难记，
  而且**它永远不满足** §7.3 第 1 条那句"全界面 Tab 序列"（规范字面仍在欠债，需写明"本 DAW 用
  显式入口替代全局 Tab 轮转"这条裁量）。

### D2 — `Tab` 改为"进入焦点链"，视图切换挪到别的键

画布持焦时 `Tab` 不再解析成 `ToggleView`，而是 `reject` 给 Slint（或宿主自建轮转），
视图切换改绑例如 `Shift+Tab` 或 `Cmd/Ctrl+Tab`。

* ✅ 正面满足 `[UI-A11Y-003]` §7.3 第 1/4 条（"Tab 序列"＋"CI 轮转用例"）。
* ⛔ **改变既有键位语义**：`[UI-A11Y-001]` 那一行要改（Advisory，可改，但要留痕）；
  `input.rs:1262` 的解析、`:1402` 的既有判据、以及任何把 `Tab` 当视图切换的判据全部重判。
* ⛔ 视图切换是创作者的**高频**键（触发矩阵 ↔ 时间轴），把它换到带修饰的位置是**可用性倒退**，
  这一条属人类裁量，Agent 不代评。

### D3 — 让 Slint 的内建全局轮转接管 `Tab`

把 `key-pressed` 对 `Tab` 一律 `reject`，交给 Slint 按"可见"筛的遍历。

* ✅ 最省事，且天然覆盖"所有可见可聚焦元素"。
* ⛔ **焦点集不受控**：此刻左栏搜索框、BPM 框、混音面板里每一个 `TextInput` 都可见 ⇒
  轮转会走进**当前折叠/滚动位置之外**的控件（ Slint 不做"按面板作用域筛"）。
* ⛔ 会推翻 `D-UI-19/20` 的实现前提：那两票的整份理由是"模态必须**自己**闭环"
  （`focus-on-tab-navigation: false`），D3 等于把闭环交回内建遍历 ⇒
  `export_dialog_modal_closure` / `export_dialog_key_capture` / `export_dialog_button_focus`
  三条判据连同 §32/§34 的边界判定**一并重判**。

## 建议（⛔ 不是决定）

**D1**，并且把 `[UI-A11Y-003]` §7.3 第 1 条那句"全界面 Tab 序列"在本仓库解释成
"**每个敲入控件都有一条键盘入口，模态内部 Tab 自闭合**"，把这个解释登记进 §7.3 的实现注记里。
理由：它不改任何既有键位、不引入不可控焦点集、不动已判绿的三份模态判据，且有 `D-UI-18` 的成例。

但**这条解释本身也是对一句 MUST 的裁量**，⇒ 必须人类批准后才写进账本；
Agent ⛔ 不得用"Advisory 文档"这一句把 MUST 抹掉。

## 无论选哪一档都残留的两件事（不在本票内解决）

1. **焦点在按钮/格上看不见**（`D-UI-20` 边界 2）：补焦点描边会改像素 ⇒ 牵动 `tests/golden/linux/`
   五张基准，属人类/集成者裁决。
2. **关闭模态时"复位至触发控件"**（`D-UI-20` 边界 1）：今天只能复位到快捷键层，
   因为触发方 `module-export-wav-button` 没有焦点身份（不是可聚焦控件）。
