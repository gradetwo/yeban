# `M4-008` 工作线台账 —— UI↔领域**唯一可变权威**（`open-questions.md` 问题 6 选项 (a) 第一片）

> 授权：`docs/ledger/human-decisions.md` 头部声明它是**唯一**需要人类裁决的清单，其条目按**建议**列
> 在负责人"按你的建议来"的长期授权下执行（**不是** Agent 自放行）。本行按
> `docs/ledger/open-questions.md` 问题 6 的 **(a) 建议**执行。
> 本片是选项 (a) 的**第一片**；问题 6 与 `ROAD-M4-008` 因此**仍是「部分」** ——
> 如实剩下的那一半写在 §4。

---

## 1. 授权与结构证据（先核实，再动手）

### 1.1 授权

| 事实 | 读数 | 命令 |
| :--- | :--- | :--- |
| `human-decisions.md` 的条目行数 | **49** | `grep -cE '^\| \`HD-[0-9]+\`' docs/ledger/human-decisions.md` |
| 其中已裁决 | **47** | `python3 scripts/gates/check_decisions.py` ⇒ `[ok] human-decisions.md: 49 项(其中 47 项已裁决)` |
| 头部自称 | "**49 项中 47 项已裁决**" | 与上面两条**逐数吻合**（不再信任任何转述数字） |
| 授权语 | "每一项按其**建议**列执行 …… 这**不是**"Agent 自己放行"" | `docs/ledger/human-decisions.md` 头部 |

### 1.2 结构性事实（改动前实测；命中数是"**匹配该模式的行数**"，命令逐条给出）

| # | 说法 | 命中数 | 命令 |
| :--- | :--- | :--- | :--- |
| 1 | `Domain` 持有自己的工程且**刻意不实现 `Clone`** | `struct Active` 1 处、`pub struct Domain` 1 处（`:100` / `:150`），全文件 `impl Clone for Domain` **0 处** | `grep -rn "pub struct Domain\|struct Active\|impl Clone for Domain" crates/yeban-mcp/src/domain/mod.rs` |
| 2 | `undo::UndoPort` 用 `RefCell<UndoSession>` 持一份 | `UndoSession` 命中 5 行；`session: RefCell<UndoSession>` 在 `crates/yeban-app/src/undo.rs:214` | `grep -c "UndoSession" crates/yeban-app/src/undo.rs` |
| 3 | `live_surface::LiveAdminSurface` 另持一份工程 | `project` 命中 **25** 行；字段在 `crates/yeban-app/src/live_surface.rs:277`（改后加了文档，行号随之下移） | `grep -c "project" crates/yeban-app/src/live_surface.rs` |
| 4 | 进程内控制面的会话是**只读克隆** | `open_in_memory(path.to_path_buf(), project.clone(), true)` —— `crates/yeban-app/src/mcp_mount.rs` 的 `session_dispatcher` | `grep -n "open_in_memory\|read_only" crates/yeban-app/src/mcp_mount.rs` |
| 5 | `#[path]` 共享的 `undo_session.rs` 在两侧是**两个类型** | `crates/yeban-app/src/undo.rs:33` 的 `#[path = "../../yeban-mcp/src/undo_session.rs"]` | `grep -n "path = " crates/yeban-app/src/undo.rs` |

**第 5 条是本项剩下工作量的结构性原因**：`#[path]` 引入的是**同一个源文件**，
但在 `yeban_app` 与 `yeban_mcp` 里各实例化一次 ⇒ `crate::undo::undo_session::UndoSession`
与 `yeban_mcp::undo_session::UndoSession` **是两个不同的类型**，"把同一个实例交给两边"
在类型系统里就不成立。因此"同一个权威"只能在**投影**层面达成（本片做的），
或另做一次把 GUI 写入口整体搬到 `Domain` 的重构。

### 1.3 改动前的基线（同一台机器）

```text
cargo tree -p yeban-app -e normal --locked                                  # grep -c yeban-mcp = 0
cargo tree -p yeban-app -e normal --locked --features in-process-mcp         # grep -c yeban-mcp = 1
bash scripts/dev/cargo-local.sh check -p yeban-app                           # exit 0
```

---

## 2. 本片做了什么（唯一可变权威 + 投影）

| 文件 | 改动 | 为什么 |
| :--- | :--- | :--- |
| `crates/yeban-mcp/src/domain/mod.rs` | `Domain::apply_revision`（字段 + 访问器）、`Plan::mutates_project`、`apply` 在**唯一可变入口**推进修订号（仅在施加**成功**时） | "权威改过工程"第一次成为**可读的事实** —— 界面刷新因此是事件驱动的，而不是"每条路径都记得调钩子"（选项 (b) 的弱点） |
| `crates/yeban-mcp/src/transport/http.rs` | `HttpServer::host_domain`：**只借出 `&Domain`** 的宿主口 | 分发器在 `Mutex` 里（`serve_once` 之后外部借不到），宿主又必须在挂载**之后**读同一个会话。签名里没有 `&mut` ⇒ 这个口子**结构上不可能**成为第二个写者 |
| `crates/yeban-app/src/mcp_mount.rs` | `InProcessMcp::project_authority()` + 只读的 `ProjectAuthorityHandle{project(), apply_revision()}` | 与既有的 `EngineReadingsHandle` 同款（同一个 `Arc<HttpServer>`、同一个 `Mutex`、同一个会话），**没有**第二套机制/端口/令牌 |
| `crates/yeban-app/src/live_surface.rs` | `build_live_ui_from_authority`（**不接受工程参数**）、`LiveUi::sync_authority`、`AuthoritySync` 三态枚举；`LiveAdminSurface.project` 的角色改写为**投影缓存** | 选项 (a) 的实质：工程的唯一来源是控制面正在服务的那一个 `Domain`；重投影只在修订号前进时发生 |
| `crates/yeban-app/tests/live_ui_mcp.rs` | 判据 17（下方 §3） | 这条判据是"闭合"与"没闭合"的分界线 |

**没有动的东西（都是刻意的）**：`read_only = true`、`SessionSource::lock_mode()`（仍 `SharedRead`）、
`acquire_session_lock`、`yeban-app/Cargo.toml` 的依赖与 features、`undo.rs` 的任何一行。

---

## 3. 判据（有牙，且能失败）

### 3.1 主判据

`crates/yeban-app/tests/live_ui_mcp.rs::an_mcp_mutation_reaches_the_live_ui_projection_through_the_single_authority`

| 步 | 动作 | 实读 |
| :--- | :--- | :--- |
| 1 | 以权威装配真实 `MainWindow`（`build_live_ui_from_authority`，无工程参数） | 起点：控件树 84 个节点；只有 `track-0-automation-volume-lane`，`track-0-automation-pan-lane` 不在 |
| 2 | **真客户端 + 真令牌 + 真环回 socket** 发 `yeban_edit_automation`（在 `TrackPan` 上写一个点） | `status = success`、`applied = true` |
| 3 | 从**宿主口**读同一个会话 | 施加修订号 **0 → 1**；工程里真的多了 `track-0-automation-pan-lane` |
| 4 | `LiveUi::sync_authority` | `Reprojected { revision: 1 }`（不是 `Unchanged`） |
| 5 | 同一个活窗口的运行时控件树 | **84 → 85** 个节点；新元素标签 = `Lead · 声相 自动化 0.250`，且**逐字等于权威工程的投影** |
| 6 | 一次只读 `yeban_query_project`（同一个 socket） | 修订号**不动** ⇒ `AuthoritySync::Unchanged`，控件树节点数不变（界面不是被查询刷新的） |
| 7 | `ui/tree` / `ui/node` 端到端 | 控制面服务的就是这**一个**更新后的活窗口 |

### 3.2 配套（推进口径的机械守卫）

`crates/yeban-mcp/src/domain/mod.rs::only_plans_that_can_change_the_project_advance_the_apply_revision`
—— 只读一侧（`yeban_query_project` / `yeban_export_midi`）：修订号与**工程字节**都逐字不变；
写一侧（`yeban_edit_automation` 写一个点）：恰好 +1 且字节真的变了。

### 3.3 负向实测（先红后还原；两条都做了，因为两种错法的方向相反）

| # | 临时改哪里 | 红在哪 | 实读 |
| :--- | :--- | :--- | :--- |
| ① | `Domain::apply` 的修订号推进改成 `let mutates = false`（即权威不再报告"我改过工程"） | 步 3 | `assertion left == right failed: 一次改工程的施加必须恰好推进一个修订号 / left: 0 / right: 1`，`test result: FAILED. 0 passed; 1 failed` |
| ② | `LiveAdminSurface::sync_authority` 里摘掉 `self.apply_project(&project)?;`（即界面不重投影） | 步 5（**UI 断言本身**） | `重投影之后运行时控件树里必须有 track-0-automation-pan-lane（界面没跟着权威走）`，`test result: FAILED. 0 passed; 1 failed` |

两条都已还原（`grep -rn "NEGATIVE MEASUREMENT" crates/` = 0 命中）。

---

## 4. 如实剩下的那一半（因此本项**不是**已关闭）

1. **生产 GUI 还不是投影**：`src/main.rs` 的 `run_gui` 没有运行期重投影（只在启动时 `host::apply_view` 一次），
   而它的写入口是 `undo::UndoPort` 的 `RefCell<UndoSession>` —— **它仍是一份权威**（§1.2 第 5 条）。
   要让 GUI 也"从 `Domain` 投影"，必须把 `Cmd+Z` / 卷帘编辑这些**写**入口改走
   `ProjectAuthorityHandle`，并在 Slint 侧接一条周期性 `sync_authority`（与电平消费同款）。
2. **因此 `read_only` 还不能放开**：只要 GUI 仍持自己的会话，把挂载会话改成可写就是**两个写者**
   （影子副本）。问题 6 的选项 (c) 已明文拒绝这件事，理由正是"会在没有单一权威接线时造出影子写者"。
   本片据此**拒绝**了"顺手把会话改成可写 + 取 `ExclusiveWrite`"，锁与写者边界**一位没动**。
3. **`UndoPort` 会话与 `Domain` 会话仍是两份**（结构性，见 §1.2 第 5 条）。

`ROAD-M4-008` 的"双 MCP 协同"因此也仍是「部分」：**同一个用例里两者真的协同**这一条已经成立
（§3.1 步 2 与步 7：域 MCP 经真 socket 改工程 ⇒ UI 侧 `ui/tree`/`ui/node` 读到新元素），
但那条协同**不经过生产 GUI**。

---

## 5. 本机验证原始读数（2026-10-06）

```text
bash scripts/dev/cargo-local.sh fmt --check                                        # exit 0
bash scripts/dev/cargo-local.sh check -p yeban-app                                 # exit 0
bash scripts/dev/cargo-local.sh check -p yeban-app --features in-process-mcp       # exit 0
cargo tree -p yeban-app -e normal --locked                      | grep -c yeban-mcp  # 0
cargo tree -p yeban-app -e normal --locked --features in-process-mcp | grep -c yeban-mcp  # 1
bash scripts/dev/cargo-local.sh clippy -p yeban-app --all-targets -- -D warnings                            # exit 0
bash scripts/dev/cargo-local.sh clippy -p yeban-app --all-targets --features in-process-mcp -- -D warnings # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-app --tests                          # exit 0（in_process_mcp* 两条目标 0 用例 = feature 关着的直接读数）
bash scripts/dev/cargo-local.sh test -p yeban-app --tests --features in-process-mcp # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-mcp --tests                          # exit 0
```

`--features in-process-mcp` 档里与本片直接相关的三行：

```text
tests/in_process_mcp.rs:      test result: ok. 3 passed; 0 failed; 0 ignored
tests/in_process_mcp_lock.rs: test result: ok. 3 passed; 0 failed; 0 ignored   ← MUST-GATE-008 跨形态互斥原样重跑
tests/live_ui_mcp.rs:         test result: ok. 18 passed; 0 failed; 0 ignored  ← 含本片判据 17（默认档是 17）
```
