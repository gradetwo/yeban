# ADR-0052 — 界面上「画了但今天**没有任何动作**」的三个控件：维持钉死／诚实置灰／移除／补能力，是产品姿态裁决，而三个控件各缺**不同的一层**（没有共同修法）

- **状态**: **Proposed**（待人类批准；⛔ 集成者代签。本票**不改任何代码、不改任何 `.slint`、不重录任何基准**，
  只把 `crates/yeban-app` 的常驻判据**点名登记**的三个空壳控件背后的裁决写成可批可驳的选项）
- **决定者**: 待人类裁决（集成者提出）
- **关联**: `ADR-0051`（同族"登记现状、⛔ 自行扩面"的写法，且本票 `D5` 与它第 2 条撞同一件事）、
  `ADR-0005`（界面⛔ 持有第二份权威）、`HD-56`（负责人 2026-10-07「最高规则」：1.0.0 之前**基准可重录**，
  代价＝手动档 `gates-manual.yml` 的 `gate=goldens` ＋ 人工复核）、`HD-51`（UI/UX 稿降级为 Advisory）、
  `UI-A11Y-003`（屏读动作与点击源同落点）、`UI-TEST-001`（语义 ID ↔ `.slint` 双向契约）、
  `ARCH-OPS-002`（提交图谱契约）、`MODEL-ISO-001`（持久层／视图态划界）、`ROAD-M4-008`（单一可变权威）
- **实现落点**: 批 `D1` ⇒ **零代码**。批 `D2`／`D3` ⇒ `crates/yeban-app/ui/**` ＋ `crates/yeban-app/src/`
  （注册表与三条判据）＋ **5 张 Linux 基准重录**。批 `D4` ⇒ 先 `crates/yeban-mcp`（新 `HostAction` 臂 ＋
  撤销语义），再 `crates/yeban-app`（键的点击源／转发／宿主唯一写者）。批 `D5` ⇒ `crates/yeban-app`（卡片文案
  与已有的提案入口对齐），⚠ 与 `ADR-0051` 同批才有意义。

## 背景（先测后改，`AGENTS.md` §6）

**这个数字在数什么**：单位＝**个**，对象＝`crates/yeban-app/src/elements.rs` 的
`KNOWN_ACTIONLESS_CONTROLS`（`:4208`，声明为 `[&str; 3]`）里登记的、**有交互角色却没有点击源**的控件 —— 也就是
"看起来能按、按下去什么都不发生"的集合。该集合由常驻判据
`controls_without_any_click_source_are_exactly_the_known_empty_ones` 逐名钉死（扫描全部 `ui/**` 的代码文本，
把六个交互角色 `button` / `tab` / `switch` / `checkbox` / `radio-button` / `combobox` 的元素块与
`clicked` 的落点做包含关系配对）。

现读（2026-10-31，`main` = `f9e1e7f0`）：

| 控件（语义 ID） | 画在哪 | 缺的是**哪一层** | 现读证据 |
| --- | --- | --- | --- |
| `ai-rail-intent-button` | `crates/yeban-app/ui/app.slint:120`，卡片标签逐字「用自然语言描述编曲意图」（`:121`） | **产品入口不存在** | `.slint` 侧**没有**任何 `callback`；`grep -rn intent crates/yeban-app/src/host.rs` ⇒ **0 行**；全仓 `自然语言` 在 `crates/*/src` ＋ `crates/*/ui` 里共 **2 行**（就是上面那个标签与注册表文案）⇒ "自然语言 → 生成"这条链在代码里**一处都不存在**（⛔ 没有解析器、⛔ 没有吃自由文本的生成入口） |
| `transport-commit-button` | `crates/yeban-app/ui/transport.slint` 的 `commit_area`（**有** `TouchArea`、**没有** `clicked`） | **会话状态层不存在** | 界面每一次编辑**当场**提交：`crates/yeban-app/src/host.rs` 里 `.commit_ops(` 的真实调用点 **33** 处（例 `:1591` 音量 / `:1727` 声像 / `:2134` 通用开关臂）⇒ 会话里**不存在**"攒一批待提交的编辑"这个状态；而权威对**空** op 批次直接 `return Ok(None)`（`crates/yeban-mcp/src/domain/mod.rs:1387`）⇒ 这个键**没有可作用的对象** |
| `transport-branch-button` | 同文件（该控件**连** `TouchArea` **都没有**，判据第 ④ 条钉死了这一点） | **领域操作面不存在**（⚠ 模型层**已有**原语，见下） | `Domain::active_branch()` **只读**（`crates/yeban-mcp/src/domain/mod.rs:555`）、`UndoState::branch()` **只读**（`crates/yeban-mcp/src/undo_session.rs:182`）、`HostAction` 今天**只有 3 臂**（`crates/yeban-mcp/src/domain/mod.rs:859` 的 `pub enum HostAction`：`Undo` / `Redo` / `Commit`）⇒ 没有"切换活跃分支"的动作面 |

⚠ **本票对既有判据文档的一处更正**（登记、⛔ 擅自改写 —— 那是别的切片的活文档）：
`crates/yeban-app/src/elements.rs` 的 `the_remaining_transport_document_actions_are_not_wired_today` 把分支键写成
"切换活跃分支的 API **一处都不存在**"。这句在**领域／宿主面**成立；在**模型面不成立** ——
`crates/yeban-model/src/commit.rs` 今天就有四个分支原语：`create_branch`（`:492`）、`fork_anonymous`（`:570`）、
`branch_head`（`:652`）、`branch_count`（`:787`），而 `create_branch` 在仓库里的唯一**领域**消费者是提案路径
（`crates/yeban-mcp/src/domain/mod.rs:4391`：建在 `draft.base_commit` 上的隔离分支 `ai/proposal-{ulid}`；
语义见 `crates/yeban-mcp/src/domain/proposal.rs` 的模块文档）。⇒ 分支键缺的**不是模型能力**，
是**领域操作面 ＋ 撤销语义**，这直接改变了 `D4` 的代价表（比"从零造分支"低一档）。

## 现状是**被机械钉住**的，但钉子不会自己拔

| 常驻判据 | 钉什么 |
| --- | --- |
| `controls_without_any_click_source_are_exactly_the_known_empty_ones` | 空壳集合与 `KNOWN_ACTIONLESS_CONTROLS` **必须逐名相等** ⇒ 新增一个"看起来能按"的控件会红；修好一个却忘了从清单删也会红 |
| `the_remaining_transport_document_actions_are_not_wired_today` | `commit` / `branch` 两键今天⛔ 转发、⛔ 声明回调、`commit_area` ⛔ `clicked`、分支块⛔ 点击源；**反向**钉回滚键 `revert-to-last-commit` **必须仍然接着**（2026-10-30 `line/revert-button` 接线后翻过来的）；第 ⑤ 条钉三个 `transport-*` ID 仍在注册表 ⇒ "顺手删控件"也会红 |
| `every_clickable_control_binds_a_matching_accessibility_action` | 只约束**有**点击源的控件（`accessible-action-default` 与 `clicked` 同落点）⇒ 它管不到"有名字、没动作"这一类，这正是上一条补的缺口 |

⇒ 缺口**不会悄悄扩大**，但也**不会被主动消掉**：判据自己写着"谁接上了就一并更新本判据"，它钉的是**今天**，
不是路线。要不要在 1.x 之前把这三个空壳消掉（以及按什么顺序消）＝ **产品／优先级裁决**，⛔ 集成者能自定的机械活。

## 五条路线（⛔ 本票不推荐任何一条）

- **`D1` 维持现状**：三个空壳原样留着，由上面三条判据逐名钉住"今天确实不可用"。零代码、零基准改动。
  代价：用户每天看到三个按下去没有反应的键，其中意图卡片**明确承诺**了一件事（「用自然语言描述编曲意图」）
  而仓库**完全没有**兑现它的那一层。
- **`D2` 诚实置灰 ＋ 可见说明**：`enabled: false` ＋ 一句写明缺哪一层的话（照 `ADR-0051` 那句
  `SIDEBAR_EMPTY_PRESETS_REASON` 的写法）。⛔ **只**改 `accessible-label` —— 本仓判据
  `musical_pr_drawer_declares_no_demo_proposals` 已写明"只写在注释或 `accessible-label` 里用户看不见"。
  ⚠ 这是**用户可见**改动 ⇒ `crates/yeban-app/tests/golden/linux/` 的 **5** 张基准（现读 `ls *.png` 计数）
  确定性过期 ⇒ 必须走 `gates-manual.yml` 的 `gate=goldens` 重录 ＋ 人工复核（`HD-56` 已把"基准绝对冻结"
  降为条件，所以这条路**可走**，但代价是人类复核工时）。
- **`D3` 移除三个控件**：连同注册表条目与 `KNOWN_ACTIONLESS_CONTROLS` 的三条一起删（ID ↔ `.slint` 是双向契约，
  只删一边会红）。最诚实，代价同 `D2`（5 张基准），并且**放弃**这三个位置代表的需求本身
  （编辑暂存 / 分支导航 / 自然语言意图）—— 将来要做时要重新画、重新登记。
- **`D4` 只接**真能接**的那一条（分支键）**：在领域面新增 `HostAction::SwitchBranch { name }`（模型原语已在），
  键的 `clicked` 与 `accessible-action-default` 同落点，`branch-name` 那块只读显示随之变成切换后的读数。
  ⚠ 批它必须**同时**回答撤销语义：切换活跃分支后 `Undo` / `Redo` 作用在**哪条链**上、`UndoState` 报的
  "可撤销步数"跨分支怎么算、时光机画的是当前分支的 `ancestry` 还是全图 —— 这些是
  `ARCH-OPS-002` ＋ `MODEL-ISO-001` 的**契约问题**，⛔ 一行接线。另两键保持 `D1`。
- **`D5` 换角色而不删**：把意图卡片改成**已存在**的提案入口的界面壳（`yeban_propose_section` 的 `stylePreset`）。
  ⚠ 两条硬约束：① 那条路径落的是**隔离分支** `ai/proposal-{ulid}`，只有 `yeban_merge_proposal` 才并回主线
  ⇒ "点一下卡片"的产品语义＝**提交一条待审查的 AI 提案**，这正是 `ADR-0051` 第 2 条在防的事，必须与它同批；
  ② **提交键⛔ 改成"落盘"** —— 落盘在仓库里**已经有**接了线的控件
  （`crates/yeban-app/ui/transport.slint:439` 的 `transport-save-button` → `host::wire_save` →
  `save_action::dispatch_save`，状态行 `status-bar-save-status`），再让提交键承担同一语义是**第二个入口做同一件事**。

## 为什么⛔ 机械替换（三条，全部现读）

1. **三个空壳缺的是三层不同的东西**（产品入口 / 会话状态层 / 领域操作面＋撤销契约）。
   有共同修法的只有"处置"类（`D2`／`D3`），而**怎么处置是产品姿态**：留着（承认在做）／
   置灰（承认没有但保留位置）／删掉（这一版不做）。三种姿态在这个仓库里都还没有被人类批过。
2. **任何用户可见的处置都要重录 5 张 Linux 基准 ＋ 人工复核**。按 `HD-56` 这不是禁令而是**代价**，
   但代价落在人类工时上 ⇒ ⛔ 集成者自行垫付（`ADR-0051` 与 `line/honesty` 系列都按这条口径写）。
3. **`D4` 触碰撤销契约与持久层划界**；**`D5` 撞 `ADR-0051` 的提案语义**，且提交键的替代角色已被 `transport-save-button` 占走。
   ⇒ 三条里没有任何一条是"顺手做完"的规模。

## 闭环与判据（无论批哪条都要一并落）

- 批 `D1` ⇒ 三条判据**不动**，本票按"裁决＝维持现状"关闭；三个空壳继续被逐名点名。
- 批 `D2` ⇒ `KNOWN_ACTIONLESS_CONTROLS` **仍然**留三条（置灰⛔ 等于有动作，`clicked` 依然不存在），
  但要**新增**一条运行时判据：置灰状态与那句说明文字必须真的出现在无头控件树里
  （`[UI-TEST-001]` 的语义寻址 ＋ `[UI-A11Y-003]`），并按 `gate=goldens` 重录 5 张。
- 批 `D3` ⇒ 语义注册表与两条判据的期望集合要**同步收缩**（⛔ 只删 `.slint` 不删登记 ⇒ 双向契约会红），
  重录 5 张；`the_remaining_transport_document_actions_are_not_wired_today` 的第 ⑤ 条（三个 ID 必须仍在注册表）
  要随之改写。
- 批 `D4` ⇒ 按 `controls_without_any_click_source_are_exactly_the_known_empty_ones` 文档要求的动作
  把分支那条**从清单删掉**（"修好一个 ⇒ 删掉它"），`the_remaining_transport_document_actions_are_not_wired_today`
  收缩成只管 `commit` 一键；跨分支的 `Undo`／`Redo`／`branch-name` 读数要在 `crates/yeban-mcp` 的判据族里落。
- 批 `D5` ⇒ 卡片文案与语义要同步三处：`crates/yeban-app/src/elements.rs` 的注册表文案、
  `crates/yeban-app/src/test_port_adapter.rs` 里"意图生成卡片必须可见"与卡片宽度两条读数、以及 5 张基准。

## 影响面

- `D1`：只有本票 ＋ 账本两行。
- `D2`／`D3`／`D5`：`crates/yeban-app`（`ui/**` ＋ `src/elements.rs` ＋ `src/test_port_adapter.rs`）
  ＋ **5 张 Linux 基准手动重录 ＋ 人工复核**。⛔ 触引擎 crate、⛔ 动持久层。
- `D4`：`crates/yeban-mcp`（`HostAction` 新臂 ＋ `Plan::Host` 施加臂 ＋ 撤销语义）＋ `crates/yeban-app`
  （键的点击源／转发／宿主唯一写者）＋ 判据族 ⇒ 建议**另开一票**；本票只负责定"要不要消、按什么顺序消"。

## 本票不做的事

- ⛔ 改任何 `.slint`、任何判据、任何基准 —— 三条路线的用户可见部分都要人类先批姿态；
- ⛔ 替 `the_remaining_transport_document_actions_are_not_wired_today` 更正那句"API 一处都不存在"
  （上文的更正只写在本票里，等该判据的 owning 切片随自己的改动一并更正，避免两个面各说一套）；
- ⛔ 把"接分支"当 UI 接线做完 —— 它需要撤销语义的答案，而那个答案⛔ 由集成者代签。
