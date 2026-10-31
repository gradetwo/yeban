# ADR-0058 — AI 提案抽屉的**焦点模型**（行间键盘移动 `U-09`）与「放弃」的**键位**（`U-10`）

- **状态**: **Proposed**（待人类批准；⛔ 集成者代签 —— **本票不改代码**，只把两处规范缺口登记成可批可驳的选项）
- **决定者**: 待人类。两票各自选一（可不同批），推荐组合写在「决定」一节
- **关联**: `~/zimmer-ui-mode-report.md` §4.1（`U-09`）/ §4.2（`U-10`）；
  `[UI-A11Y-001]`（扫描码热键）、`[UI-A11Y-003]`（屏读动作与点击源同落点）、
  `[UI-A11Y-004]`（无障碍树与焦点）、`[MODEL-AST-002]`（抽屉读数由投影派生）、
  `ADR-0005`（界面⛔ 持有第二份权威）、`ADR-0052`（"画了但无动作"控件的处置）、
  `ADR-0054`（同类：一张表里同一个语义只有一个口径）
- **落点（批准后）**: `crates/yeban-app/src/input.rs`（`Focus` 枚举 / `InputContext::resolve_logical`
  的策略表 / `Action` 词表）、`crates/yeban-app/src/host.rs`（`apply_action` 的对应臂，
  形状照 `AcceptAiSuggestion` 那条）、`crates/yeban-app/ui/dialogs/musical_pr_drawer.slint`
  （仅 `D1` 需要：焦点来源）

## 背景（先测后改，`AGENTS.md` §6）

Normative 两份文档（架构 / 路线图）**没有**规定：抽屉里的提案行如何用键盘移动，也没有规定
「放弃」应该占哪一格。`YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` 自 2026-10-06 起是
**Advisory（非绑定）**。⇒ 两处都是**规范缺口**，按 `AGENTS.md` §5.7 登记，不自行选一种落地。

### 现读事实（本票的每一条都来自代码，不是推断）

| # | 事实 | 出处（符号级，行号会腐烂） |
| :--- | :--- | :--- |
| 1 | 焦点模型只有 **3** 态：`MainCanvas` / `TextInput` / `Other` —— **没有**抽屉态 | `input.rs` 的 `enum Focus` |
| 2 | 按键策略表的顺序：① `F5`/`F6` 全局 → ② `TextInput` 整块放行（合成态 `ConsumedByIme`）→ ③ `Cmd/Ctrl` → ④ `Alt` → ⑤ `Shift` → ⑥ 裸键。第 ⑥ 步里**只有 `Tab` 看焦点**（`self.focus == Focus::MainCanvas`），其余格子（含裸 `↑/↓`）**在 `Focus::Other` 下同样生效** | `input.rs` 的 `InputContext::resolve_logical` |
| 3 | 裸 `↑/↓` = `NudgeSelection { step: Semitone }`；`Shift+↑/↓` = 整八度；`Alt+↑/↓` = `SelectTrackBy(∓1)`；`Cmd/Ctrl+Alt+↑/↓` = `NudgeMixerChannel { Volume }` —— 方向键**四格已占**，其中 `Alt+Shift+方向` 与 `Cmd/Ctrl+Alt+Shift+方向` 明确**放行**（表里有 `if !modifiers.shift` / `(key, false, true)` 的守卫） | 同上（第 ③④⑤⑥ 步） |
| 4 | 采纳**已有**键位：`Shift+Enter` → `Action::AcceptAiSuggestion`；宿主臂只 `invoke` 抽屉的 `accept()` 回调，且带两个前置（`musical_pr_open ∧ musical_pr_resolve_available`），缺一 ⇒ **返回 false（不消费）** | `input.rs` 第 ⑤ 步、`host.rs` 的 `apply_action` |
| 5 | 放弃**只有**可点入口 `musical-pr-reject-button`；`host.rs` 的注释逐字写着"拒绝只有可点等价物"。落点 `resolve_proposal(Reject { id: selected })` 与 `wire_proposal_drawer` 的 `on_reject_ai_proposal` 已是同一条 | `host.rs` 的 `wire_proposal_drawer` |
| 6 | 抽屉的**移动目标已存在**：行 ID `musical-pr-proposal-{i}-item`、输入 `selected-proposal-id`、回调 `select-proposal(string)`；"点一行 = **只**写那一行的身份再重注入"（不合并） | `musical_pr_drawer.slint` / `host.rs` |
| 7 | 裸 `Esc` = `Action::Cancel`，其顺序是「导出弹窗 → **抽屉开着就收起**（`U-11`，2026-10-31 落地）→ 取消在手拖动」—— 一个键今天已担两义 | `input.rs` 第 ⑥ 步、`host.rs` 的 `Action::Cancel` 臂 |
| 8 | 裸 `[` / `]` 已经是抽屉语义（`AuditionMain` / `AuditionProposal`）⇒ "提案"这件事在**裸键区**已有先例 | `input.rs` 第 ⑥ 步 |
| 9 | 报告的复现状态：本轮抽屉行数为 **0** ⇒ `U-09` / `U-10` **均未被行为复现**，两条都是"代码事实仍立 / 文档自承仍立"（§4.1 / §4.2） | `~/zimmer-ui-mode-report.md` §4 |

⚠ 第 9 条限定了本票的证据边界：**没有**用户可见的复现，只有表 1–8 的代码事实。

## 决定

### `U-09`：抽屉内行间键盘移动

**推荐 = D2（先给一格未占的修饰组合，不动焦点模型）**；`D1` 是完整答案但需要一整条焦点工程。

| 选项 | 内容 | 代价（必须有） |
| :--- | :--- | :--- |
| **D1** | 加第四态 `Focus::ProposalList`：焦点落在抽屉列表时裸 `↑/↓` = 上/下一条提案，其余焦点一律不变 | ① 需要一个**真实的焦点来源**（今天 `Focus` 由 Slint 侧上报，列表行不是可聚焦控件 ⇒ 要新增 `FocusScope` 与上报接线）；② 焦点模型是**跨视图**的（卷帘 / 编排 / 混音 / 各弹窗都归它），只为抽屉加一态会留下"这一处有焦点语义、别处没有"的不一致；③ 要定"抽屉开着但焦点仍在画布"时谁赢 —— 这一格今天的实际答案是**裸 `↑/↓` 改音符半音**，改判等于改一次既有语义；④ 判据要覆盖焦点进出（不是只测一次按键） |
| **D2** ⭐ | 不加焦点态，在**当前放行**的格里选一格（现读候选：`Alt+Shift+↑/↓` —— 第 ④ 步的 `Alt+↑/↓` 带 `!shift` 守卫，故其 `Shift` 变体空着）⇒ 新绑 `Action::SelectAdjacentProposal { direction }`，落点复用 `select-proposal` 回调 | ① 与列表控件的通行习惯（裸方向键移动选中行）不一致，键盘-only 用户要靠文档与 `ADR-0001` 式的键位表；② 若将来落地 D1，这格要么保留成别名（两格同义，是长期负担），要么撤（撤是一次 breaking change）；③ 夹紧口径要新定（首条之上 / 末条之下是夹紧还是回绕），**这一小格仍需人类点头** |
| **D3** | 抽屉开着就劫持裸 `↑/↓`（不看焦点） | ⛔ 不推荐：抽屉开着时"半音微调选中音符"这条既有语义会**静默换义**，与 `ADR-0054` 立的"同一语义只有一个口径"相反，也正是报告 §4.1 点出的那处冲突；且用户常常边看抽屉边改音符 |
| **D4** | 关掉本票，接受"行间移动只能点" | 代价是把键盘-only 用户挡在外面（`[UI-A11Y-004]` 的方向），但**与今天的真实暴露面一致**：本轮抽屉行数为 0（事实 9），缺口尚未被任何用户路径命中 |

### `U-10`：「放弃」的键位

**推荐 = D2（`Shift+Delete`），且与采纳 `Shift+Enter` 同族**。

| 选项 | 内容 | 代价（必须有） |
| :--- | :--- | :--- |
| **D1** | `Shift+Backspace` → `Action::RejectAiSuggestion`，宿主臂照 `AcceptAiSuggestion` 的形状（同两个前置 ⇒ 不满足就 `return false`，⛔ 不吞键），落点 `invoke_reject_ai_proposal()` | 与裸 `Backspace` = `DeleteSelection` 只差一个 `Shift`，而这两件事**作用域不同**（前者写工程的删音符、后者只翻记录：领域侧 `apply_reject` **绝不**删除那条提案）⇒ 相邻键位会被读成同一族，误触代价不对称 |
| **D2** ⭐ | `Shift+Delete` → 同上 | 同 D1 的"相邻键位"代价（裸 `Delete` 也是 `DeleteSelection`）。选它而非 D1 的理由：**macOS 键盘的 `Delete` 键就是 Backspace**，`Shift+Delete` 在两个平台上是同一颗物理键的同一个修饰；而 PC 全尺寸键盘上 `Shift+Backspace` 与 `Shift+Delete` 会分裂成两个动作位 |
| **D3** | `Alt+Backspace`（第 ④ 步里 Backspace 放行） | 与 macOS 的 `Option+Delete`（删一词）文本编辑习惯撞 —— 虽然文本框在第 ② 步就整块放行、实际不冲突，但读起来像"抢了编辑键" |
| **D4** | 让裸 `Esc` 兼"放弃" | ⛔ 不推荐：事实 7 —— `Esc` 今天已担两义（收起弹窗 / 收起抽屉 / 取消拖动），再加一义会把刚落的 `U-11` 反过来重定义；"收起"与"放弃提案"是两种结果（后者翻记录、留痕），一键两义正是报告 §4.2 点名"没有规范裁决"的那处 |
| **D5** | 不修，维持"只有 `musical-pr-reject-button`" | 事实 5 的不对称继续存在：采纳有键、放弃没有 —— 键盘-only 用户能合、不能弃（只能收起抽屉，语义不同） |

### 批准后怎么落（两票共同）

1. 只动 `input.rs` 的表 + `host.rs` 的臂 + `main.rs`/`cli.rs` 的键位表登记，**零像素**：
   不新增可见控件 ⇒ `[UI-MCP-003]` 的 Linux 基准**不需要**重录（⛔ `gate=goldens`）；
2. 判据照既有形状：`tests/ai_drawer_key_ui.rs` 已钉"开抽屉 ⇒ Esc 消费、再 Esc 放行"，
   新格同样要钉**正反两向**（前置不满足 ⇒ 不消费；满足 ⇒ 恰好一条回调），
   ⛔ 只测"按了有用"（那读不出误吞键）；
3. `D1(U-09)` 若被选中，焦点上报那条**单独成票**，不与键位混在一次提交里 ——
   焦点模型改动会影响所有视图的按键归属，判据面完全不同。

## 推翻条件

- 人类若落地**完整焦点模型**（跨视图定义"焦点在哪个面板 ⇒ 哪一族键归它"），`U-09` 的 D2 应作废，
  改走 D1，并把 D2 那格撤掉；撤之前要先给它一个替代位（⛔ 留一格空着）。
- 若将来引入"多提案并行待决"的 UI（今天的模型是点选**一条**再处置），
  `U-10` 的任何单键语义都要重议（一键到底放弃哪一条 = 焦点问题，退化成 `U-09` 未解的形态）。
- 若 `[UI-A11Y-004]` 被解释为"列表类控件必须可用裸方向键遍历"，则 D2/D4 全部出局，只剩 D1。

## 与本报告其余条目的关系

`U-05` / `U-07` / `U-11` 已按负责人任务落地（判据见 `docs/ledger/app-zimmer-ui-notes.md` §1）；
`U-12`（自动化泳道不覆盖主总线）与 `N-UI-2`（循环重复无 UI 表示）各自还需要一条产品裁决，
⛔ 本票不代答；`U-13` 的三个控件已由 `ADR-0052` / `HD-72` 登记且未决。
