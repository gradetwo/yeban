# 撤销接线（UI + MCP 两侧，共用同一实现）—— 台账 [ADR-0001 D45, D46, D25, MODEL-ISO-001, ARCH-OPS-002]

工作线 `line/undo-wiring`（worktree `.worktrees/undo-wiring`，基线 `b1d783b`）。
人类负责人裁决 **D45**（逐字）："撤销入口 UI+MCP 两侧同接，共用同一实现。"

## 0. 一句话结论

模型侧那个生产级撤销（`CommitGraph::undo_with`）现在**有了两侧调用者**：
MCP 侧是工具 `yeban_undo` / `yeban_redo`，UI 侧是 `Cmd+Z` / `Cmd+Shift+Z` /
`Cmd+Shift+H` 与时光机弹窗里的"撤销一步"按钮。两侧**编译的是同一份源码**
（`crates/yeban-mcp/src/undo_session.rs`，UI 侧用 `#[path]` 引入），
因此"人按 `Cmd+Z`"与"AI 发 `yeban_undo`"改的是**同一串字节** —— 由判据逐字节钉住。

**没有**动 `crates/yeban-model/**`（一行都没动）、**没有**动根 `Cargo.toml` /
`Cargo.lock`（零新增依赖）、**没有**动 `crates/yeban-ui-mcp/**`、
**没有**动 `.github/**` 与 `scripts/**`。

## 1. 两侧入口的具体行号

| 侧 | 入口 | 行号（本提交） |
| :--- | :--- | :--- |
| MCP | 工具注册（`yeban_undo` / `yeban_redo` 两条 `ToolSpec`） | `crates/yeban-mcp/src/tools.rs:605`、`:622`；`TOOL_COUNT` 在 `:61` |
| MCP | 工具名 → 计划（`plan` 分发） | `crates/yeban-mcp/src/domain/mod.rs:1064`、`:1065` |
| MCP | 只读规划 `plan_undo` / `plan_redo` | `crates/yeban-mcp/src/domain/mod.rs:1100`、`:1118` |
| MCP | 施加 `apply_undo` / `apply_redo` | `crates/yeban-mcp/src/domain/mod.rs:1480`、`:1516` |
| MCP | 契约错误码映射（`INDEX_OUT_OF_BOUNDS` 等） | `crates/yeban-mcp/src/domain/mod.rs:920`（`undo_refusal_to_fault`） |
| MCP | **唯一执行者**：`undo_session::undo` → `graph.undo_with(..)` | `crates/yeban-mcp/src/undo_session.rs:624`、`:634` |
| MCP | 重做（正向 `Op::apply`） | `crates/yeban-mcp/src/undo_session.rs:651` |
| MCP | 契约登记（工具名 + 扩展参数约束） | `schemas/mcp-tools.schema.json` 的 `definitions.ToolCall.properties.name.enum` 与 `definitions.ExtensionToolArguments` |
| UI | 时光机弹窗：动作回调 + 模型读数属性 | `crates/yeban-app/ui/dialogs/undo_tree_modal.slint:33`（`can-undo`）、`:41`（`callback undo-step()`）、按钮元素 ID 在 `:174`（`undo-tree-undo-button`） |
| UI | MainWindow：注入面 + 回调 + 转发 | `crates/yeban-app/ui/app.slint:170`（`undo-can-undo`）、`:190`（`callback undo-step()`）、`:503` 起（转发进弹窗） |
| UI | 界面 → 会话的**唯一入口** | `crates/yeban-app/src/undo.rs:274`（`UndoPort::perform`） |
| UI | 键盘链（策略表 → 动作 → 工程回退） | `crates/yeban-app/src/undo.rs:102`（`perform_key`）、`:84`（`dispatch_key`） |
| UI | Slint 接线（两条回调 + 显示态注入） | `crates/yeban-app/src/host.rs:325`（`wire_undo`）、`:295`（`apply_undo`） |
| UI | `main.rs` 里的调用点 | `crates/yeban-app/src/main.rs:164`（建会话测）、`:180`（`wire_undo`）、`:182`（`apply_undo`） |
| UI | 语义元素登记 | `crates/yeban-app/src/elements.rs:851`（`undo-tree-undo-button`） |
| 判据 | MCP 侧端到端 | `crates/yeban-mcp/tests/undo_wiring.rs`（17 条） |
| 判据 | UI 侧端到端 | `crates/yeban-app/tests/undo_wiring_ui.rs`（10 条） |
| 判据 | 两侧各跑一遍的共享判据 | `crates/yeban-mcp/src/undo_session.rs` 的 `#[cfg(test)]`（17 条） |

行号取法：`grep -n` 在本提交的树上实测（`grep -n 'name: "yeban_undo"' …` 一类），
不是凭记忆写的。

## 2. "共用同一实现"的机械证据（三层）

### 2.1 文件级：一份源码，两个 crate

| 侧 | 引入方式 | 编译它的 crate |
| :--- | :--- | :--- |
| MCP | `crates/yeban-mcp/src/lib.rs` 的 `pub mod undo_session;` | `yeban-mcp` |
| UI | `crates/yeban-app/src/undo.rs` 的 `#[path = "../../yeban-mcp/src/undo_session.rs"] pub mod undo_session;` | `yeban-app` |

`#[path]` 引入**同一份源码**是本仓库既有手法（先例：`crates/yeban-render/verify/pure_modules.rs`、
`crates/yeban-mcp/verify/section_pure.rs`、`crates/yeban-app/tests/real_ui_tier1.rs`）。
因此共享模块的 `#[cfg(test)]` 在**两个 crate 里各跑一遍** —— 这不是巧合，是"同一份实现"的直接可观测后果
（实测：`yeban-app` 的测试清单里出现 `undo::undo_session::tests::*`）。

### 2.2 grep 级 + 结构级：生产代码里只有一个撤销实现

判据 `undo_session::tests::no_second_undo_implementation_exists_in_the_workspace`
（纯函数 `scan_second_undo_implementations` + 真实文件扫描，见 `undo_session.rs:999`）在
两个 crate 里各断言一次：生产区（`#[cfg(test)]` 属性**行**之前、注释行不计）不得出现
`apply_inverse` / `structural_inverse` / `invert(`（自写反向应用）与
`.undo_with(` / `.undo(`（绕开共享入口直调模型）。

本提交树上的实测读数（脚本口径与判据一致）：

```
生产区命中计数: apply_inverse=0  structural_inverse=0  invert(=1  .undo_with(=1  .undo(=0
```

- `.undo_with(` 的 1 处 = `undo_session.rs:634`（共享实现里唯一一处）；
- `invert(` 的 1 处 = `undo_session.rs:395` 的 `op_is_applied`：**只读**地构造逆操作来
  判断"这条 op 是否已作用在文档上"（用于把游标从"文档 + 图谱"推导出来），**不施加**任何东西。

### 2.3 集成者预审的 16 处 `apply_inverse`：逐条分类

`grep -rn 'apply_inverse' crates/yeban-mcp/src crates/yeban-app/src` 在本提交树上共 **16** 行
（集成者那次看到 15 行，是因为管道里的 `grep -v test` 又滤掉了 `undo_session.rs:1665`
—— 那行是判据里的**夹具字符串**，`diff` 已核。逐条如下：

| # | 位置 | 类别 | 本线新增？ |
| :--- | :--- | :--- | :--- |
| 1 | `undo_session.rs:32` | 注释：解释"撤销由模型内的 `op.apply_inverse` 完成" | 新增（文档） |
| 2 | `undo_session.rs:977` | 注释：解释判据为什么豁免测试区 | 新增（文档） |
| 3 | `undo_session.rs:991` | 注释：判据规则 1 的说明 | 新增（文档） |
| 4 | `undo_session.rs:997` | 注释：判据规则的补充说明 | 新增（文档） |
| 5 | `undo_session.rs:1012` | **生产代码**：判据的**匹配词表字符串**（不是反向应用） | 新增（守卫本身） |
| 6 | `undo_session.rs:1637` | 判据代码：对行做 `contains` 过滤 | 新增（判据） |
| 7 | `undo_session.rs:1649` | 判据夹具字符串："看似等价的独立实现"样例 | 新增（判据） |
| 8 | `undo_session.rs:1653` | 判据断言字符串 | 新增（判据） |
| 9 | `undo_session.rs:1662` | 注释：反例 3 的说明 | 新增（文档） |
| 10 | `undo_session.rs:1665` | 判据夹具字符串（`#[cfg(test)]` 区内） | 新增（判据） |
| 11 | `undo_session.rs:1672` | 判据夹具字符串（注释样例） | 新增（判据） |
| 12 | `domain/section.rs:246` | 既有：`#[cfg(test)]`（`section.rs:103` 之后）提案骨架可逆性判据 | 未改动 |
| 13 | `domain/mod.rs:20` | 既有：模块文档（"逆操作一律由 `Op` 提供"） | 未改动 |
| 14 | `domain/mod.rs:2455` | 既有：`#[cfg(test)]`（`:2020` 之后）`edit_notes` 可逆性判据 | 未改动 |
| 15 | `domain/macros.rs:271` | 既有：`#[cfg(test)]`（`:169` 之后）宏可逆性判据 | 未改动 |
| 16 | `domain/section_build.rs:1537` | 既有：`#[cfg(test)]`（`:1075` 之后）骨架可逆性判据 | 未改动 |

⇒ **生产侧真正"施加逆操作"的地方只有一处**：`undo_session.rs:634` 的
`graph.undo_with(..)`（模型内 `op.apply_inverse`）。本线新增的 `apply_inverse`
**只有**两条：一条是判据自己的匹配词表（第 5 条），其余全是注释与判据夹具字符串。
因此**没有任何一处"自己拼的撤销"**，不需要按 D45 改。

为什么本线能保证这一点而不是靠自觉：`undo_session::commit` **不需要**回滚 ——
它先在克隆体上施加整批（`Op::Batch` 自己也是原子试跑），提交写成之后才替换权威工程。
第一版实现曾在提交失败路径上写 `batch.apply_inverse(project)`（合法的自愈），
被这条守卫当场拒绝，于是改成"克隆体 + 最后替换" ⇒ 连那一处也消失了。

### 2.4 行为级：两侧同一串字节

判据 ⑧（两份，互补）：
`crates/yeban-mcp/tests/undo_wiring.rs::the_ui_path_and_the_mcp_path_agree_byte_for_byte`
与 `crates/yeban-app/tests/undo_wiring_ui.rs::the_ui_path_matches_the_shared_implementation_byte_for_byte`。
两侧都用同一个 `undo_session::project_fingerprint`（模型容器字节的 SHA-256）与
同一个夹具 `undo_session::wiring_fixture`（一个确定性的"改音符力度"脚本）：

- MCP 路径：真工具 `yeban_edit_notes` → `yeban_merge_proposal` → `yeban_undo`；
- UI 路径：`UndoSession::commit`（`OpOrigin::UserUi`）→ `UndoPort::perform(UiAction::Undo)`；
- 断言：两条路径的工程 JSON **`serde_json::to_string` 逐字节相同**，且都等于原始工程。

## 3. 游标为什么不落盘（逐字节证据）

`UndoCursor` 由模型**刻意**不实现 `Serialize`/`Deserialize`
（`crates/yeban-model/src/commit.rs:178` 的文档："属于会话运行态，不是持久化文档层"）。
本线的会话态 `UndoState`（游标 + 活跃分支）因此也不可能出现在 `.yeban` 里。

判据 ⑦的实测证据（`crates/yeban-mcp/tests/undo_wiring.rs::the_cursor_never_reaches_the_project_container`）：

1. 撤销前后 `history.dag` 的**字节完全相同**（`serde_json::to_vec(graph)` 逐字节比较）；
2. 工程文档的**键集合**逐一相同（`serde_json::to_value(project)` 的键序列比较）——
   没有多出任何字段，且工程 JSON 文本里不含 `cursor` / `Cursor` / `undone` / `redo`；
3. 容器字节级：`project.json` 变了（少了被撤销的那次变更），`history.dag` 与资产逐字节不变；
   读回容器后再比较一次 `history_dag`。

**强化（本线发现并处置的一个真缺口）**：游标不落盘 ⇒ "撤销 → 保存 → 退出 → 重开"之后，
文档停在被撤销后的状态而 `history.dag` 里那条提交还在 ⇒ 重开后的第一次 `Cmd+Z`
会去撤销一条**本来就没应用**的 op（模型会如实拒绝 `OpStateMismatch`，用户看到的是"撤销坏了"）。
处置**不是**把游标序列化，而是**推导**它：`UndoState::align_with`
（`undo_session.rs:229`）只读地从（文档, 图谱）算出"头上已经有几步没被应用"
（判据：`op.invert(doc)` 的前置条件成立 ⇔ 该 op 已生效；`Op::Batch` 递归到子 op）。
于是重开之后：`undone = 1`（推导出来的）、`redo` 仍然可用，而工程里一个字节都没有游标。
判据：`the_cursor_is_derivable_from_the_document_and_the_graph` 与
`saved_after_an_undo_reopens_to_the_same_bytes`。

## 4. MCP 工具面（D45 + D46）

- **工具名**：`yeban_undo`、`yeban_redo`（登记在 `schemas/mcp-tools.schema.json` 的
  `ToolCall.name.enum`，并各有一条实参约束：`definitions.ExtensionToolArguments.$defs.<tool>`，
  经 `allOf` 的 `if/then` 挂到 `ToolCall` 上）。
- **`specId` 为什么不写 `MCP-TOOL-011/012`**：四份规范里 `MCP-TOOL-` 族只有 `001..010`，
  而本线**无权**改 `docs/YEBAN_*.md`。`AGENTS.md` §4.1 立过先例（`MODEL-AST-006` 缺号
  ⇒ 不得凭空发明编号）。因此扩展工具的 `specId` 是 `MCP-TOOL-EXT-UNDO` /
  `MCP-TOOL-EXT-REDO`（**形态上就不是编号**），并有判据钉住"规范族恰好 001..010、
  扩展 ID 不得伪装成编号"。schema 侧同样逐字段对账（`extension_argument_constraints_match_the_registry`）。
- **参数**：`steps`（integer，可选，缺省 1，`minimum: 1`）；`dryRun` / `idempotencyKey`
  由 `COMMON_PARAMS` 提供（与其余十个工具完全同义）。
- **`steps: 0`**：**显式**拒绝（`INVALID_PARAMETER_RANGE`）—— 不含糊地替调用方猜"等价于 1"。
- **无历史可撤**：`INDEX_OUT_OF_BOUNDS`（ADR-0001 **D25** 的联集 20 值内，**不发明新码**）。
  语义说明：D25 的联集里没有 `NO_HISTORY`；"请求的步数超出可回退深度"就是索引/计数越界。
  载荷带 `reason: "no-history"`，好让调用方区分是哪一类越界。
  其余拒绝映射：`NoBranch` / `NotAtCommitBoundary` / `Model` → `CONFLICT`；`Serialization` → `IO_ERROR`。
- **`dryRun`**：走 `domain::plan`（拿到 `&Domain`，借用检查器保证只读），
  预览内容由 `undo_session::simulate_undo` / `simulate_redo` 在**克隆体**上算出 ——
  与真做**同一个函数**，因此"预览说的"与"真做的"不可能漂移（判据 ⑥ 逐字段比较摘要）。
- **行为**：整批（`Op::Batch`，如 `yeban_propose_section` 那批）算**一步**；
  撤销后继续编辑按模型既有的 `CommitGraph::fork_anonymous` 派生匿名分支
  （原分支头保留为只读孤岛），因此被撤销的 op **不可能**被再撤一次。

## 5. UI 面（D45 的另一半）

- `undo_tree_modal.slint`：新增 `can-undo` / `undo-depth` / `undone-depth` /
  `commit-count` / `branch-name` 五个**模型读数**属性；新增回调 `undo-step()`
  （"撤销一步"按钮，语义元素 ID `undo-tree-undo-button`）；顶部状态行与按钮文案
  全部由这些读数拼出，弹窗**不自己推断**能不能撤销。6 节点 DAG 图形仍是硬编码骨架
  （见 §9 未实现项）。
- `app.slint`：新增 `undo-can-undo` / `undo-can-redo` / `undo-depth` / `undo-undone` /
  `undo-commit-count` 五个注入面属性与 `callback undo-step()`，并把读数转发给弹窗。
- `host.rs`：`apply_undo`（唯一写这些属性的地方，与 `apply_transport` 同纪律）与
  `wire_undo`（`toggle-undo-tree` + `undo-step` 两条回调 → 同一个 `UndoPort::perform`）。
  撤销之后**重新投影**（`ViewState::from_project` → `apply_view`），因此界面画的是回退后那一版。
- `undo.rs`：`UndoPort` 是界面 → 会话的**唯一入口**，每次动作写一条 `UiActionRecord`
  （动作名 + 后果 + 游标前后 + **工程指纹前后** + 提交数前后）。判据据此断言
  "点了它 ⇒ 工程真的回退了一版"，而不是"控件树里有这个元素"。
- `main.rs`：`--open` / 样本装载之后建会话（打开 = 新会话），注入显示态并接线；
  报告行如实打印"可撤销 N 步 · 已撤销 M 步 · 分支 X · 实现 = undo_session"。
- **"编辑入口"边界（如实登记）**：本线**只**接撤销。`UndoPort::commit_ops` 是留给
  未来编辑面的入口（本线未接任何产生 `Op` 的交互），因此生产会话的初始撤销深度是 **0**，
  界面显示"无历史可撤"；判据通过该入口注入一个真 `Op` 来断言"点了它真的回退一版"。

**UI 动作真的执行撤销的实测输出**（本机真跑，判据断言；不是控件树证据）：

```
cargo test -p yeban-app --test undo_wiring_ui
test a_click_really_rolls_the_project_back ... ok
  动作日志: action="undo" outcome=Changed{steps:1, undone_total:1, op_kinds:["Batch"]}
  fingerprint_before=Some("<编辑过的那一版>") fingerprint_after=Some("<原始那一版>")
  undone_before=0 -> undone_after=1 ; commits_before=2 -> commits_after=2（撤销不动图谱）
test the_cmd_z_chain_reaches_the_model ... ok
  Cmd+Z ⇒ Changed{1}（工程回到原始）; Cmd+Shift+Z ⇒ Changed{1}（回到编辑过的那一版）
  Cmd+Shift+H ⇒ DisplayOnly（只开弹窗）; 文本框聚焦 + Cmd+Z ⇒ None（策略表说 PassThrough，留给文本框）
test the_ui_path_matches_the_shared_implementation_byte_for_byte ... ok
  UI 路径撤销后的工程指纹 == 原始工程指纹（同一串字节）
```

## 6. 判据清单（14 条要求 → 落点）

| # | 要求 | 落点 | 本机 |
| :--- | :--- | :--- | :--- |
| ① | 单步撤销逐字节回到上一版（真夹具，比较 `serde_json::to_string`） | `undo_wiring.rs::one_undo_returns_to_the_previous_bytes`；`undo_session.rs::single_undo_returns_to_the_previous_bytes` | ✅ 真跑 |
| ② | 连续 N 步逐步回退 | `consecutive_undos_step_back`；`consecutive_undos_step_back_byte_by_byte` | ✅ |
| ③ | redo 逐字节回到撤销前 | `redo_returns_to_the_bytes_before_the_undo`（两份） | ✅ |
| ④ | 批量（`Op::Batch`）算**一步** | `a_proposal_batch_counts_as_one_step`；`a_batch_counts_as_exactly_one_step` | ✅ |
| ⑤ | 无历史 ⇒ 明确错误码 + 工程不变 | `undo_without_history_is_an_explicit_domain_error`；`undo_without_history_refuses_and_leaves_the_project_untouched` | ✅ |
| ⑥ | `dryRun=true` ⇒ 状态一位不变 + 预览与真做一致 | `dry_run_does_not_touch_state_and_matches_the_real_undo`；`the_read_only_simulation_matches_the_real_undo_and_redo` | ✅ |
| ⑦ | 游标不落盘（字节级 + 键集合） | `the_cursor_never_reaches_the_project_container`；`the_cursor_is_never_persisted`；`the_cursor_is_derivable_from_the_document_and_the_graph` | ✅ |
| ⑧ | UI 路径与 MCP 路径逐字节相同 | `the_ui_path_and_the_mcp_path_agree_byte_for_byte`（MCP 侧）；`the_ui_path_matches_the_shared_implementation_byte_for_byte`（UI 侧） | ✅ |
| ⑨ | UI 动作**真的**接线（动作日志 / 注入点断言） | `a_click_really_rolls_the_project_back`；`a_ui_action_really_rolls_the_project_back_one_version`；`the_cmd_z_chain_reaches_the_model`；`the_keyboard_chain_really_rolls_the_project_back` | ✅ |
| ⑩ | 撤销不破坏既有不变量（`validate()`） | `the_project_still_validates_after_undo`；`the_project_still_validates_after_a_ui_undo`；`the_project_still_validates_after_undo_and_redo` | ✅ |
| ⑪ | 撤销后再保存、再打开 ⇒ 与内存态一致 | `saved_after_an_undo_reopens_to_the_same_bytes`（两侧各一条）+ `save_and_reopen_after_an_undo_matches_the_in_memory_state` | ✅ |
| ⑫ | 撤销不越过"工程打开"边界 | `undo_does_not_cross_a_project_open_boundary`（含"打开带历史的工程"与"匿名分支泄漏"两种强形态）；`a_new_project_open_starts_a_fresh_undo_session`；`undo_never_crosses_a_project_open_boundary` | ✅ |
| ⑬ | 幂等 / 并发安全 | `the_same_idempotency_key_never_undoes_twice`；`a_refusal_leaves_the_session_usable`；`repeated_undo_actions_accumulate_in_the_log` | ✅ |
| ⑭ | 门禁：`light` + 涉及 crate 的测试/clippy | 见 §8（light 14 步中 1 步红，已在 §9 记为 needs） | 部分 |

附加判据（超出 14 条要求）：工具列表描述符（`the_two_new_tools_respect_the_existing_pipeline`）、
契约形状与错误码（`undo_refusals_are_in_band_not_implementation_errors`）、
撤销后继续编辑派生匿名分支（`editing_after_an_undo_forks_an_anonymous_branch`）、
显示态来自模型读数（`the_display_state_comes_from_the_model_readings`）、
投影可用性（`the_projection_after_a_ui_undo_is_the_rolled_back_project`）、
唯一调用点（`the_tool_path_uses_the_model_inverse_entry_point_only`）。

## 7. 注入记录（4 条：注入 ⇒ 变红 ⇒ 还原 ⇒ 变绿）

每条都**实际改了源码并真跑**，红点原文照抄如下（`…` 为省略）。
还原之后重跑同一命令 ⇒ `test result: ok`。

### 注入 ① 在 MCP 侧写一份"看似等价"的独立实现（故意差一点）⇒ 变红

注入：新增 `crates/yeban-mcp/src/domain/undo_alt.rs`（自写 `for stamped in ops.iter().skip(1) { stamped.apply_inverse(project) }`
—— 既绕开共享实现，又少撤最新那一条），并把 `apply_undo` 改成调它。

红点 A（静态守卫，**两个 crate 各红一次**）：

```
thread 'undo_session::tests::no_second_undo_implementation_exists_in_the_workspace' panicked:
生产代码里出现了第二份撤销实现:
…/crates/yeban-mcp/src/domain/undo_alt.rs:19 出现 `apply_inverse` —— 生产代码里不许有第二份反向应用实现
test result: FAILED. 0 passed; 1 failed
```

红点 B（行为判据，端到端真的走错了实现）：`one_undo_returns_to_the_previous_bytes`
（`left: Number(0) right: 1`）、`a_proposal_batch_counts_as_one_step`、
`consecutive_undos_step_back`、`redo_returns_to_the_bytes_before_the_undo`
（`INDEX_OUT_OF_BOUNDS no-redo`）、`the_cursor_never_reaches_the_project_container`
（`left: 0 right: 1`）、`the_same_idempotency_key_never_undoes_twice`、
`editing_after_an_undo_forks_an_anonymous_branch`、`a_refusal_leaves_the_session_usable`
—— 共 **8 条**红（`test result: FAILED. 14 passed; 3 failed` 与 `14 passed; 5 failed` 两批合计）。
还原后：`test result: ok. 17 passed`（MCP 端到端）与守卫 `ok`。

### 注入 ② 把游标"落盘"（追加到工程字节）⇒ 变红

注入：`UndoSession::project_bytes` 在容器字节后追加 `format!("undone={}", cursor)`。

红点：`undo_session` 里 **8 条**红，其中

```
the_cursor_is_never_persisted ... FAILED（工程字节里不该出现 `undone`）
single_undo_returns_to_the_previous_bytes ... FAILED（逐字节回不到上一版）
consecutive_undos_step_back_byte_by_byte / a_batch_counts_as_exactly_one_step /
the_read_only_simulation_matches_the_real_undo_and_redo /
undo_then_commit_forks_so_discarded_ops_cannot_be_undone_again /
save_and_reopen_after_an_undo_matches_the_in_memory_state /
the_cursor_is_derivable_from_the_document_and_the_graph ... FAILED
```

还原后：`test result: ok. 17 passed`。

### 注入 ③ 把一批 op 拆成 N 条（批不再算"一步"）⇒ 变红

注入：`undo_session::commit` 里把 `vec![StampedOp::new(.., batch)]` 改成
`request.ops.iter().map(|op| StampedOp::new(.., op))`。

红点（模型侧 3 条 + 工具面 3 条）：

```
a_batch_counts_as_exactly_one_step ... FAILED
the_read_only_simulation_matches_the_real_undo_and_redo ... FAILED
single_undo_returns_to_the_previous_bytes ... FAILED
--- 工具面 ---
a_proposal_batch_counts_as_one_step ... FAILED（left: 16, right: 1）
one_undo_returns_to_the_previous_bytes ... FAILED（left: "ModifyNoteVelocity", right: "Batch"）
dry_run_does_not_touch_state_and_matches_the_real_undo ... FAILED
```

还原后：`test result: ok. 17 passed`。

### 注入 ④ 打开工程时不重置会话态（游标 / 活跃分支跟到新工程上）⇒ 变红

注入：删掉 `Domain::reset_history` 里的 `self.undo = UndoState::new(AGENT_NAME);`。

第一版注入**没有**变红 —— 因为打开路径上还有一层自愈（`align_with` 从新文档重新推导游标）。
这是**判据不够强**，不是实现没问题：真正的泄漏是**活跃分支**。
于是把判据 ⑫ 加强成"打开一个**带历史**的工程 + 在上一个工程上撤销后继续编辑（派生匿名分支）"，
再注入 ⇒ 变红：

```
undo_does_not_cross_a_project_open_boundary ... FAILED
打开新工程必须把活跃分支拉回 main:
{"error":{"code":"CONFLICT","data":{"branch":"anon-01M458QACP0AR9M39K5KXY3A9J"},
 "message":"提交图谱里没有分支 `anon-…`"},"status":"error"}
```

还原后：`test result: ok. 17 passed`。

## 8. 本机真跑 vs CI（严格区分）

**本机真跑的**（全部通过；命令与实测时长如下。"缓存来源"= 复用主仓
`/Users/crow/work/music/yeban/target` 的已建缓存，只增量重编本工作树的成员 crate；
这条用法由集成者本轮明文授权："允许复用已建缓存、增量有界 …… 不允许本机做 Slint 的首次全量构建"）。

| 命令（均带 `CARGO_TARGET_DIR=主仓/target`） | 实测 | 结果 |
| :--- | :--- | :--- |
| `cargo-local.sh check -p yeban-app --locked`（**未改动**的树先探一次成本） | 17.9 s wall | 复用缓存成功（未触发 Slint 首次全量构建） |
| `cargo-local.sh test -p yeban-mcp --locked` | 8.3 s wall | 10 个测试二进制全绿（含 17 条新端到端 + 224 条 lib） |
| `cargo-local.sh test -p yeban-app --locked` | 21.2 s wall | 8 个目标全绿（163 lib + 14 + 15 + 2 + 11 + 10；含 Slint testing-backend 的 `real_ui_tier1`） |
| `cargo-local.sh test -p yeban-app --features ui-test-port --locked` | （同上，全绿） | 含 `[[test]] test_port_adapter` |
| `cargo-local.sh clippy -p yeban-mcp --all-targets --locked -- -D warnings` | 1.1 s（增量） | **0 告警** |
| `cargo-local.sh clippy -p yeban-app --all-targets --locked -- -D warnings` | 2.0 s（增量） | **0 告警** |
| `cargo-local.sh check -p yeban-model --locked` | 4.9 s wall | 未改动模型（只读，验证基线可编） |
| `bash scripts/gates/run-gates.sh light` | 1.2 s | **14 步中 13 步绿**；`feature-alignment` 红（见 §9 needs-1） |
| `python3 scripts/gates/validate_schemas.py --samples-dir <导出目录>` | 由 `contract.rs` 真跑 | 合法样本全过、故意违法样本变红（schema 扩展被 jsonschema 4.24.0 接受） |

**只有 CI 能判的**（本机**没有**做，也不该做）：`cargo clippy --workspace --all-targets`、
`cargo test --workspace --all-targets`、`cargo deny check`、跨平台（windows/macos 矩阵）、
fuzz、benchmark。本次判决以 `scripts/dev/ci-verdict.sh line/undo-wiring` 读回的 run id 为准
（本线**只推一次**：判决读数写在推完之后的汇报里，不另开回填提交）。

**如实登记的新增编译成本**：本线新增 3 个源文件（共享实现 1 710 行、UI 端口 650 行、
两份端到端判据 1 252 行）⇒ 增量重编 `yeban-mcp` lib+tests 约 **8 s**、
`yeban-app` lib+tests 约 **21 s**（Slint 重依赖全部来自已建缓存；未新增任何依赖，
`Cargo.lock` 与 `Cargo.toml` 的 diff 为**空**）。

## 9. 未实现项 / needs / 边界

| 编号 | 事项 | 为什么不在本线 | 类型 |
| :--- | :--- | :--- | :--- |
| needs-1 | `feature-alignment.md` 需要点名 `yeban_undo` / `yeban_redo` | 该文件在本线的**禁改清单**里（集成者独占）。而 `scripts/gates/check_feature_alignment.py` 的判据 1 要求 schema 里的每个工具名都出现在该表里 ⇒ 本线的 `light`（以及 CI 的 checks job）会因此红 | 跨线耦合（**本线唯一的红**） |
| needs-2 | `history.dag` 的公开编解码面 | `yeban-app` 没有 `serde_json` 依赖（零新增依赖），而 `history.dag` = `serde_json::to_vec(CommitGraph)`。⇒ UI 侧目前**不能**从容器恢复提交图谱（只能用模型对象）。建议把 `yeban-mcp/src/domain/store.rs::decode_history_dag` 那套口径提到 `yeban-model`（或给 app 加一条依赖边，走 D51） | 需要裁决 |
| needs-3 | OS 键事件源 | `undo::perform_key` 已经把"策略表 → 动作 → 工程回退"整条链做完并可判据化，但本进程**还没有**把键盘事件喂进 `input.rs` 的事件源（Slint 不暴露物理扫描码；`[UI-A11Y-001]` 要求扫描码绑定） | 既有缺口（非本线引入） |
| 未实现 | 生产侧"编辑入口" | 本线只接撤销；`UndoPort::commit_ops` 已就绪，但没有产生 `Op` 的交互 ⇒ 生产会话初始撤销深度为 0（界面显示"无历史可撤"） | 已登记（对齐表同族） |
| 未实现 | 时光机 **DAG 图形** | 6 个节点仍是硬编码示意；真实版本树要画匿名分支/合并（`ARCH-OPS-002`）。本线接线的是"状态行 + 撤销一步按钮" | 已登记 |
| 未实现 | 多步撤销的 UI 交互（点节点回滚） | 需要"节点 ↔ 撤销步数"的映射与选择语义（含分叉/合并），本线只做了 `UndoMany(steps)` 这一层动作与判据 | 已登记 |
| 未实现 | redo 栈容量策略 | 本线**不需要** redo 栈：重做 = 把游标往回一格的 op 按正向 `Op::apply` 再打一次（游标本身就是栈指针）。容量/裁剪策略在"无限撤销历史"那一层，本线不引入 | 有意不做 |
| 未实现 | 计时器/时钟注入 | 会话的 `now_ms` 由调用方注入（模型纪律）；UI 侧用 `SystemTime`，MCP 侧沿用既有 `set_now_ms` | 有意不做 |
| 边界 | 焦点闭环（`[UI-A11Y-003]` §7.3） | 弹窗仍未做 Focus Trap（文件头已登记），本线未扩大该缺口 | 既有 |

**needs-1 的精确修法**（集成者 10 秒可关）：在 `docs/ledger/feature-alignment.md`
的"执行撤销 / 重做"那一行里点名两个工具并更新三侧标记与状态，例如把该行的
"计划：新开一条线**同时**接 UI（…）与 MCP（工具或 `ui/*` 方法）"改成
"MCP 工具 `yeban_undo` / `yeban_redo` 与 UI 的 `Cmd+Z` / 时光机按钮**共用**
`crates/yeban-mcp/src/undo_session.rs`（`#[path]` 同一份源码）"，
并把 UI/MCP 两列改成 `有`、状态改成 `三方齐全`（同时按 `check_feature_alignment.py`
的口径更新 §1 汇总计数）。这两处一改，本线的 `light` 即全绿。

## 10. 修改文件与净行数

```
 crates/yeban-app/src/elements.rs                  |   10 +
 crates/yeban-app/src/host.rs                      |   83 +-
 crates/yeban-app/src/lib.rs                       |    1 +
 crates/yeban-app/src/main.rs                      |   54 +-
 crates/yeban-app/src/undo.rs                      |  650 ++++++++   (新增)
 crates/yeban-app/tests/undo_wiring_ui.rs          |  351 +++++    (新增)
 crates/yeban-app/ui/app.slint                     |   18 +
 crates/yeban-app/ui/dialogs/undo_tree_modal.slint |   68 +-
 crates/yeban-mcp/src/domain/mod.rs                |  470 +++++-
 crates/yeban-mcp/src/lib.rs                       |    1 +
 crates/yeban-mcp/src/tools.rs                     |   99 +-
 crates/yeban-mcp/src/undo_session.rs              | 1710 ++++++++  (新增)
 crates/yeban-mcp/tests/contract.rs                |   74 +-
 crates/yeban-mcp/tests/tools_e2e.rs               |   12 +-
 crates/yeban-mcp/tests/undo_wiring.rs             |  901 +++++    (新增)
 schemas/mcp-tools.schema.json                     |   95 +-
 16 files changed, 4519 insertions(+), 78 deletions(-)   （净 +4441）
```

没有出现在清单里的关键文件（= **没动**）：`crates/yeban-model/**`、
`crates/yeban-ui-mcp/**`、`Cargo.toml`（根与成员）、`Cargo.lock`、`scripts/**`、
`.github/**`、`deny.toml`、`docs/YEBAN_*.md`、`docs/DEVELOPMENT_LEDGER.md`、
`docs/adr/**`、`docs/ledger/{gate-status,phase-status,feature-alignment,human-decisions}.md`、法务文件。

## 11. 与裁决 / 规范 ID 的对应

- **D45**（撤销两侧同接、共用同一实现）：§1 §2 §5 与全部判据；
- **D46**（工具集是起点不是上限，扩充必须同步 schema）：§4；
- **D25**（错误码联集 20 值，不发明新码）：§4 的 `INDEX_OUT_OF_BOUNDS` 映射；
- **MODEL-ISO-001**（三层状态隔离）：§3（游标不落盘 + 可推导）；
- **ARCH-OPS-001/002**（领域操作日志与提交图谱、撤销后继续编辑派生匿名分支）：§4 §7 注入④；
- **`MCP-TOOL-EXT-*`**：扩展工具的 `specId`（**不是**规范编号，理由见 §4）。
