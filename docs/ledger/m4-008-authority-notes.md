# `M4-008` 工作线台账 —— UI↔领域**唯一可变权威**（`open-questions.md` 问题 6 选项 (a)：第一片 + 第二片 + 第三片 + 第 (b) 项）

> 授权：`docs/ledger/human-decisions.md` 头部声明它是**唯一**需要人类裁决的清单，其条目按**建议**列
> 在负责人"按你的建议来"的长期授权下执行（**不是** Agent 自放行）。本行按
> `docs/ledger/open-questions.md` 问题 6 的 **(a) 建议**执行。
> §1–§5 是选项 (a) 的**第一片**（投影口）；§6 是**第二片**（GUI 的**写**入口落到该权威上）；
> §7 是**第三片**（GUI 的**保存路径**纳入同一把 `.yeban.lock`，并**实测**裁决"仍不翻 `read_only`"）；
> §8 是**第 (b) 项**（生产窗口的**运行期重投影钩子** —— `run_gui` 不再只画打开时那一版）。
> 问题 6 与 `ROAD-M4-008` 因此**仍是「部分」**：§7.5 的两件事里，第 2 件（运行期重投影）
> 已由 §8 关闭，第 1 件（**单一写者会话**：宿主保存动作）仍在 —— 逐条见 §8.5。

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

## 4. 第一片结束时剩下的那一半（**已由第二片部分关掉**，见 §6）

> 本节保留第一片的原始判断（历史不删）。第二片之后哪些成立、哪些不成立，逐条对照见 §6.4。

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

---

## 6. 第二片（2026-10-06）：GUI 的**写**入口也落到唯一可变权威上

> 本片只做 §4 三件事里的第 1 件（写入口），并**按授权刻意停在**第 2 件之前：
> `read_only` 与锁模式**一位没改**（§6.4 说明为什么"顺手翻过去"就是影子写者）。
> 因此问题 6 与 `ROAD-M4-008` **仍是「部分」**。

### 6.1 结构性证据（**改动前**实测；命中数 = "匹配该模式的行数"，命令逐条给出）

| # | 说法 | 改动前读数 | 命令 |
| :--- | :--- | :--- | :--- |
| 1 | GUI 的撤销端口**持自己的一份会话**（影子写者的载体） | `RefCell<UndoSession>` 命中 **1**（`undo.rs:214`） | `grep -c "RefCell<UndoSession>" crates/yeban-app/src/undo.rs` |
| 2 | `#[path]` 决定两侧是**两个类型**（"共享同一实例"在类型系统里不成立） | 命中 **1**（`undo.rs:33`） | `grep -n "path = " crates/yeban-app/src/undo.rs` |
| 3 | 挂载会话**只读**（`read_only = true`） | `open_in_memory(.., true)` 命中 **1** | `grep -n "open_in_memory" crates/yeban-app/src/mcp_mount.rs` |
| 4 | `Domain` 的**唯一**可变入口 | `pub fn apply` 命中 **1** | `grep -c "^pub fn apply(" crates/yeban-mcp/src/domain/mod.rs` |
| 5 | `Domain` 刻意不实现 `Clone` | `impl Clone for Domain` 命中 **0** | `grep -c "impl Clone for Domain" crates/yeban-mcp/src/domain/mod.rs` |

**改动后**（同一条命令）：

| # | 说法 | 读数 |
| :--- | :--- | :--- |
| 1 | `RefCell<UndoSession>`（影子形态）已不存在 | **0** |
| 1b | 新后端 `UndoBackend::{Local, Authority}` 的权威变体 | `UndoBackend::Authority` 命中 **8** |
| 3 | `read_only` 仍是 `true` | **1**（`mcp_mount.rs:734`，未改） |

### 6.2 本片做了什么

| 文件 | 改动 | 为什么 |
| :--- | :--- | :--- |
| `crates/yeban-mcp/src/domain/mod.rs` | 新增 `HostAction`（`Undo`/`Redo`/`Commit`）、`HostOutcome`、`Plan::Host` 变体与 `apply_host_action`；`op`/`commit_delta`/`mutates_project`/`project_after`/`describe`/`apply_inner` 的匹配臂补齐 | 宿主动作与工具动作在**同一处**汇合：`apply(&mut Domain, Plan)`。`apply_revision` 的推进因此仍只有**一个**位置 |
| `crates/yeban-mcp/src/transport/http.rs` | `HttpServer::apply_host_action` | 与 `host_domain` 共用**同一个** `Mutex<Dispatcher>`、**同一个** `Domain`；没有第二个端口 / 令牌 / 通道；对外 JSON-RPC 面一位没变 |
| `crates/yeban-app/src/mcp_mount.rs` | `ProjectAuthorityHandle::{undo_display, graph, apply_host}`；类型文档与模块文档改写为"读口 + 写口" | GUI 的读（显示态 / 图谱 / 投影）与写（撤销 / 提交）都从**这一个**句柄走 |
| `crates/yeban-app/src/undo.rs` | `UndoPort` 内部改为 `UndoBackend::{Local(Box<UndoSession>), Authority(ProjectAuthorityHandle)}`；新增 `from_authority` / `try_project`；`display`/`project`/`graph`/`fingerprint`/`commit_ops`/`undo`/`redo` 全部委派 | 挂载了控制面时端口**不再持有任何工程 / 图谱 / 游标副本**；没有控制面时仍是本地会话（那时进程里没有第二个写者） |
| `crates/yeban-app/src/main.rs` | `run_gui` 把控制面**挂载点提前**到构造端口之前；挂上了用 `UndoPort::from_authority`，没挂上用 `open_undo_session` | "谁是权威"由**挂载结果**决定，不由调用点随手决定 |
| `crates/yeban-app/src/host.rs` | 三处 `port.project()` → `port.try_project()`（`None` 时出声跳过，不 panic 在 Slint 回调里） | 权威会话可能被控制面 `yeban_close_project` 关掉工程 |
| `crates/yeban-app/tests/live_ui_mcp.rs` | 判据 18 / 19 + `project_note_ids` 夹具 | 见 §6.3 |

### 6.3 判据（有牙，且能失败）

**判据 18** `live_ui_mcp.rs::a_gui_action_and_a_session_action_share_one_projection_through_the_authority`

| 步 | 谁写 | 实读 |
| :--- | :--- | :--- |
| 1 | 以权威装配真实界面；由**同一个**句柄构造 `UndoPort`（不持会话） | 起点 `track-0-automation-pan-lane` 不在运行时树里；端口的显示态逐字段 == 权威的 `undo_display()` |
| 2 | **会话侧**：真客户端 + 真令牌在环回 socket 上发 `yeban_edit_automation` | 施加修订号 `r → r+1`；`sync_authority = Reprojected`；泳道进树 |
| 3 | **GUI 侧**：`host::wire_undo` 接的真实 Slint 回调 `undo-step` | 修订号 `r+1 → r+2`（**同一个权威**）；端口 `display().undone == 1`；动作日志 `changed_project()` |
| 4 | 投影跟随 | `Reprojected`；泳道离开运行时树 |
| 5 | **会话侧**：`yeban_redo` | 修订号 `r+2 → r+3`；同一条泳道回到同一投影 |

**判据 19** `live_ui_mcp.rs::a_gui_pencil_edit_lands_in_the_same_authority_as_the_session`

| 步 | 动作 | 实读 |
| :--- | :--- | :--- |
| 1 | 以权威装配界面；`host::wire_roll_edit` 接**同一个**句柄构造的端口 | 起点音符身份集（样本工程 4 个） |
| 2 | 用**与宿主同一条解析**（`host::pencil_op_for`）找一个真落在片段内的点击位置 | `Some(AddNote)` |
| 3 | **GUI 事件源**：`app.slint` 的 `clicked` 回调 | 权威工程音符 **4 → 5**（恰好 +1）；修订号 `r → r+1` |
| 4 | `sync_authority` + 运行时树 | 新元素 `note-{ulid}-rect` 在树里（新 ULID 由前后差集算出，不硬编码） |
| 5 | **会话侧**：真环回 socket 发 `yeban_undo` | 音符身份集回到起点；新元素离开同一棵树 |

### 6.3.1 负向实测（先红后还原；两条方向不同，因此两条都做了）

| # | 临时改哪里 | 红在哪 | 实读 |
| :--- | :--- | :--- | :--- |
| ① | `UndoPort::from_authority` 改成**另开一份 `UndoSession`**（即改动前的影子形态） | 判据 18 步 3 | `assertion left == right failed: GUI 的写入必须落在**同一个**权威上 … left: 1 / right: 2`；`test result: FAILED. 0 passed; 1 failed; 0 ignored; 19 filtered out` |
| ② | `UndoPort::commit_ops` 的权威分支改成直接返回 `Err`（即 GUI 的提交没落到权威上） | 判据 19 步 3 | `assertion left == right failed: GUI 铅笔画一次必须恰好在**权威工程**里加一个音符（起点 4 个，现在 4 个） left: 0 / right: 1`；`test result: FAILED. 0 passed; 1 failed; 0 ignored; 19 filtered out` |

两条都已还原（`grep -rn "NEGATIVE MEASUREMENT" crates/` = **0** 命中）。

### 6.4 为什么**不**翻 `read_only` / 锁模式（本片在此明确停下）

第一片与第二片之间，"只读"的**准确含义**被查清了：`read_only` 只闸**落盘**
（`plan_save` 在 `read_only` 时返回 `IO_ERROR`），**不闸内存里的工程变更** ——
`yeban_undo` / `yeban_edit_automation` 在 `read_only = true` 的会话上一直能改工程
（判据 17 就是这么写的）。因此第 1 步**不需要**放开它，本片也确实没放。

把 `read_only` 改成 `false` 会**新开一条落盘路径**：控制面的 `yeban_save_project` 可以写
`SessionSource::File` 所指的那个工程文件。而 app GUI 自己的保存路径
（`ui/force_save` → `src/save.rs`、`--save-as`）**不取** `.yeban.lock`（模块文档的"诚实边界"
一直这么登记）。两条会同时写同一个文件的路 = **影子写者** —— 正是问题 6 选项 (c) 被拒的理由。
而且 `SessionSource::lock_mode()` 即便改成 `ExclusiveWrite` 也**挡不住 GUI 的保存路径**（它压根不取锁），
所以"翻了锁就等于排他"是假的。

⇒ 本片**停在这里**：`read_only = true`、`LockMode::SharedRead` 一位没动；
`in_process_mcp_lock.rs`（`MUST-GATE-008` 的跨形态互斥）原样重跑 **3 passed**。
下一片若要翻，前提是先把 GUI 的保存路径也纳入同一把锁。

### 6.5 第二片之后如实剩下的（因此 `ROAD-M4-008` 仍是「部分」）

1. **落盘写者边界未合并**（§6.4）：GUI 保存路径不取 `.yeban.lock` ⇒ `read_only` 不能翻、
   锁模式不能重裁决。这是"同一份工程的两个磁盘写者"这一条，**不是**内存权威的问题（内存权威已唯一）。
2. **生产 GUI 仍没有运行期重投影**：`run_gui` 建窗口走的是 `host::build_main_window`（初值来自
   `loaded.archive.project` —— 一份**只读**输入，不是权威），而唯一的"依修订号重投影"实现
   （`live_surface::LiveUi::sync_authority`）住在**测试目标专用**的 `src/live_surface.rs` 里
   （dev-dependency 决定它不进产品二进制）。后果：GUI **自己**的动作会从权威重投影
   （`host::refresh_undo_window` / `wire_roll_edit` 都用 `port.try_project()`），
   但**会话侧的改动不会自动刷新生产窗口**。补它需要在产品路径上接一条周期性刷新，
   而 `run_gui` 今天没有任何定时器 —— 加一个**无法无头判定**的定时器违反本仓 DoD
   （"没有判据的能力不算交付"），因此本片不做，如实登记。
3. **启动时仍有两份 `YebanProjectV1` 值**（`loaded.archive.project` 与 `Domain` 里那一份克隆）：
   前者只作**初始投影输入**、从不被写，因此它不构成第二个**可变**权威；
   要让产品窗口也从 `Domain` 投影（`build_live_ui_from_authority` 进产品路径）是第三片的事。

### 6.6 本机验证原始读数（第二片，2026-10-06）

```text
bash scripts/dev/cargo-local.sh fmt --check                                        # exit 0
bash scripts/dev/cargo-local.sh check -p yeban-app                                 # exit 0
bash scripts/dev/cargo-local.sh check -p yeban-app --features in-process-mcp       # exit 0
bash scripts/dev/cargo-local.sh tree -p yeban-app -e normal --locked | grep -c yeban-mcp                      # 0
bash scripts/dev/cargo-local.sh tree -p yeban-app -e normal --locked --features in-process-mcp | grep -c yeban-mcp  # 1
bash scripts/dev/cargo-local.sh clippy -p yeban-app --all-targets -- -D warnings                            # exit 0
bash scripts/dev/cargo-local.sh clippy -p yeban-app --all-targets --features in-process-mcp -- -D warnings # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-app --tests                          # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-app --tests --features in-process-mcp # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-mcp --tests                          # exit 0
```

`--features in-process-mcp` 档里与本片直接相关的四行：

```text
tests/in_process_mcp.rs:      test result: ok. 3 passed; 0 failed; 0 ignored
tests/in_process_mcp_lock.rs: test result: ok. 3 passed; 0 failed; 0 ignored   ← MUST-GATE-008 跨形态互斥原样重跑
tests/live_ui_mcp.rs:         test result: ok. 20 passed; 0 failed; 0 ignored  ← 含本片判据 18 / 19（默认档是 17）
tests/undo_wiring_ui.rs:      test result: ok. 10 passed; 0 failed; 0 ignored
```

---

## 7. 第三片（2026-10-06）：GUI 的**保存路径**也纳入同一把 `.yeban.lock`

> 本片做选项 (a) 剩下的第 1 件事（落盘写者边界），并把第 2 件事（翻 `read_only`）
> **实测后拒绝**：翻过去会让控制面真的落盘（§7.4 的原始读数），而 GUI **自己**的保存
> 路径会被**本进程自己**的锁挡住 ⇒"翻一下"不是一次机械改动，它需要"单一写者会话"
> 这个新设计。第 3 件事（生产路径的运行期重投影）仍然不做，理由见 §7.5。
> 因此问题 6 与 `ROAD-M4-008` **仍是「部分」**。

### 7.1 结构性证据（**改动前**实测；命中数 = "匹配该模式的行数"，命令逐条给出）

| # | 说法 | 改动前读数 | 命令 |
| :--- | :--- | :--- | :--- |
| 1 | app 侧**一行取锁代码都没有** | `acquire(` 命中 **0** | `grep -rn "acquire(" crates/yeban-app/src/ \| wc -l` |
| 2 | **写工程**的入口只有两条，且都在 `save.rs` 里收口 | `save_project_file(` 除 `save.rs` 外命中 **1**（`live_surface.rs:566` = `ui/force_save`）；`save_archive_file(` 命中 **2**（`cli.rs:1542` = 生产 `--save-as`，`cli.rs:2058` = 同文件单测） | `grep -rn "save_project_file(\|save_archive_file(" crates/yeban-app/src/ \| grep -v "src/save.rs"` |
| 3 | 共用的原子写入面**不都是工程** | `write_file_atomically(` 除 `save.rs` 外命中 **3**：`cli.rs:1505`（`--export-elements`）、`export_midi.rs:52`（`--export-midi`）、`export_als.rs:115`（`--export-als`） | 同上一行的命令（换函数名） |
| 4 | `feature-alignment.md` 那行"GUI 路径不持锁"的机械证据**从一开始就数错了东西** | `grep -rn "yeban.lock\|try_lock" crates/yeban-app/src/` 命中 **21**（全是注释：`mcp_mount.rs` 16 / `main.rs` 3 / `undo.rs` 2） | 该命令本身（零编译） |
| 5 | "共享 `yeban-mcp` 源码"有先例可循 | `#[path` 声明命中 **1**（`undo.rs:33` 装 `undo_session.rs`） | `grep -rn "#\[path" crates/yeban-app/src/` |

**第 4 条是顺手修掉的元问题**：那一列想表达的"GUI 不取锁"其实该用**取锁调用**计数
（第 1 行：0 命中），而写成"`yeban.lock` 这个词出现几次"之后，任何**注释**都会让它变绿 ——
它当时写"命中 0"，但第二片加的模块文档早已把命中推到 21。本片因此把那一格改成
**可复核的取锁调用计数**。

### 7.2 本片做了什么

| 文件 | 改动 | 为什么 |
| :--- | :--- | :--- |
| `crates/yeban-app/src/project_lock.rs`（新） | `#[path = "../../yeban-mcp/src/domain/lock.rs"] pub mod lock;` | **同一份源码**装进 app（与 `undo.rs` 装 `undo_session.rs` 同款）⇒ 不写第二份锁协议；也不新增 `yeban-mcp` 依赖边（MUST-GATE-009 的编译期开关一位没动） |
| `crates/yeban-app/src/save.rs` | `save_project_file` / `save_archive_file` 各取一次 `LockMode::ExclusiveWrite`（私有 `acquire_write_lock`）；新增 `SaveError::{Locked, LockUnsupported}` + `LockHold`；`write_file_atomically` **仍然不取锁** | 这两条是**唯一**写工程容器的入口 ⇒ 取锁位置只有一个，不存在"某条保存路径忘了取"。导出目标（清单 / `.mid` / `.als`）不是工程文档，对它们取工程锁是**假保护** |
| `crates/yeban-app/Cargo.toml` | `serde_json` 由 dev-dependency 改为普通依赖（一条**直接边**，包集合不变） | 共享进来的那份源码用 `serde_json` 写锁元数据；它**早就在**默认依赖图里（`yeban-model` → `serde_json`） |
| `crates/yeban-app/src/lib.rs` | `pub mod project_lock;` | 锁模块的出入口 |
| `crates/yeban-app/src/save.rs`（单测） | 判据 8 | 本机最快的"持锁 ⇒ 拒绝"读数 |
| `crates/yeban-app/tests/cli_contract.rs` | 判据 B14 | **默认构建**的真二进制判据（CI 的默认腿就会跑到它） |
| `crates/yeban-app/tests/in_process_mcp_lock.rs` | 判据 ③①②（两条） | 跨进程（真 `Command` 子进程持有）与同进程（挂载会话持有）两条腿 |

**没有动的东西（刻意的）**：`read_only = true`、`SessionSource::lock_mode()`（仍 `SharedRead`）、
`acquire_session_lock`、`crates/yeban-mcp/**`（一个字节没改）、`in-process-mcp` /
`experimental-als-export` 的 feature 定义、`Cargo.lock`（实测未变，见 §7.6）。

### 7.3 判据（有牙，且能失败）

| # | 判据 | 证什么 | 关键断言 |
| :--- | :--- | :--- | :--- |
| 8 | `save.rs::a_held_project_lock_refuses_the_save_without_touching_the_file`（lib 单测） | 保存路径**真的**取同一把锁 | 持锁 ⇒ `SaveError::Locked`（点名工程与锁文件）+ 旧文件逐字节未变 + **释放后同一份保存必须成功** + 成功不留残留 `.yeban.lock` |
| B14 | `cli_contract.rs::save_as_is_refused_while_the_project_lock_is_held`（**默认构建**、真二进制） | 命令行保存路径（`--save-as`）与别的持有者争同一把锁 | 持锁 ⇒ 退出 **4** + stderr 含"拒绝写入" + 目标逐字节未变（第二次保存写的是**另一份**工程 ⇒ "写没写"在字节上看得见）+ 释放后退出 **0** 且内容**真的换了** |
| ③ | `in_process_mcp_lock.rs::the_gui_save_paths_are_refused_while_another_process_holds_the_project_exclusively`（feature，**跨进程**） | 别的形态（真子进程）排他持锁时，**两条** GUI 保存路径都被拒 | `save_project_file` ⇒ `SaveError::Locked`（`holderMetadata == "available"`）；真二进制 `--save-as` ⇒ 退出 4；`SIGKILL` 持有者后保存成功且不留残留锁 |
| ④ | `in_process_mcp_lock.rs::a_mounted_read_only_session_refuses_the_gui_save_on_the_same_file`（feature，**同进程**） | "本进程的只读会话"与"GUI 的排他写"互斥（fail-closed） | 挂载（`SharedRead`）时 `save_project_file` ⇒ `SaveError::Locked` 且字节未变；`stop()` 后同一条保存成功 |

判据 ④ **不是"想要的结局"，而是登记事实**：它把"长命的共享读者与短命的排他写者互斥"
变成可失败的断言，也是 §7.4 拒绝翻转的实测依据。

### 7.3.1 负向实测（先红后还原；两个取锁点各测一次，因为它们是**两条**独立接线）

| # | 临时改哪里 | 红在哪 | 实读 |
| :--- | :--- | :--- | :--- |
| ① | `save_project_file` 里的 `acquire_write_lock(path)?` 摘掉 | 判据 8 / 判据 ④ / 判据 ③ 的 2a 步 | 判据 8：`持锁时保存必须被拒: SaveReport { … bytes: 7683 … }`，`test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 188 filtered out`；`in_process_mcp_lock`：`test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out`（③ 与 ④ 同时红）。**B14 保持绿**（它走 `save_archive_file`）⇒ 两个取锁点确实是**两条**接线，不是一条 |
| ② | `save_archive_file` 里的 `acquire_write_lock(path)?` 摘掉 | B14 / 判据 ③ 的 2b 步 | B14：`assertion `left == right` failed: 被别的持有者持锁时 --save-as 必须拒绝（退出 4）; stderr= left: 0 right: 4`，`test result: FAILED. 0 passed; 1 failed; … 18 filtered out`；`in_process_mcp_lock`：`left: Some(0) / right: Some(4)`，`test result: FAILED. 4 passed; 1 failed; …`（判据 ④ 保持绿） |

两条都已还原（`grep -rn "NEGATIVE MEASUREMENT" crates/` = **0**；`grep -c "acquire_write_lock(path)?" crates/yeban-app/src/save.rs` = **2**）。

### 7.4 为什么**仍然不**翻 `read_only` / 锁模式（这次是**实测**的裁决）

第二片拒绝翻转的理由是"GUI 保存路径不取锁"（§6.4）。本片把那一条**关掉了**，
因此翻转必须重新裁决一次 —— **实测**（临时改 `mcp_mount.rs` 三处：`lock_mode` →
`ExclusiveWrite`、`acquire_lock(path, true)` → `(path, false)`、`open_in_memory(.., true)`
→ `(.., false)`，跑完即还原）得到：

| 判据 | 翻转后的实读 | 说明 |
| :--- | :--- | :--- |
| `in_process_mcp.rs::the_round_trip_stops_leaving_nothing_listening` | `left: String("success") / right: "error"`（`:359`），`test result: FAILED. 2 passed; 1 failed; 0 ignored` | **控制面真的拿到了落盘路径**：`yeban_save_project` 从被拒变成成功 —— 这就是"新开一条磁盘写者"的实测证据 |
| `in_process_mcp_lock.rs::the_mounted_control_plane_excludes_writers_and_shares_with_readers` | `left: Some(ExclusiveWrite) / right: Some(SharedRead)`（`:375`） | 共享读的语义没了：另一个只读形态**不能再共存**（`MUST-GATE-008` 的第二条腿） |
| `in_process_mcp_lock.rs::a_mount_is_refused_while_another_form_holds_the_project_exclusively` | `left: Some(ExclusiveWrite) / right: Some(SharedRead)`（`:320`） | 同一个断言（同一份语义） |
| `in_process_mcp_lock.rs::a_mounted_read_only_session_refuses_the_gui_save_on_the_same_file` | `left: Some(ExclusiveWrite) / right: Some(SharedRead)`（`:519`） | 而 GUI 自己的保存**仍然是拒绝**（`flock` 在 fd 粒度仲裁，同进程另一个 fd 也冲突）⇒ 翻转**不会**让 GUI 能存，只会让控制面能存 |
| 合计 | `test result: FAILED. 2 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out` | 三条红、两条绿（跨进程那两条与本裁决无关） |

**裁决：不翻。** 三条理由，逐条都有上面的读数支撑：

1. **翻转换来的是"控制面能落盘"，代价是"GUI 不能保存"** —— 同一个工程文件上，
   会话是长命持有者、GUI 保存是短命写者，两者在*同一个进程*里抢同一把 `flock`，
   GUI 必然输。要让两者都成立，前提是**一个持有者**（会话）成为唯一写者，
   也就是给宿主加一个**保存动作**（`HostAction::Save` 之类）并让 GUI 的 `ui/force_save`
   走它 —— 那是新的接口，不是"把布尔翻一下"。
2. **它会拆掉 `MUST-GATE-008` 的第二条腿**：`SharedRead` 的语义是"只读者共存、写者被挡"，
   现有判据 ② 就靠"另一个只读进程能共存"证明挂载取的不是排他锁。翻成 `ExclusiveWrite`
   之后，第二个 app 实例 / 独立 stdio `yeban-mcp` 的**只读**访问会被挡在门外 ——
   而"AI 只读分析"正是本形态的主要用途。
3. **本片的目标（GUI 保存路径纳入同一把锁）不依赖翻转**：控制面保持只读时，
   GUI 保存仍然与它争同一把锁（判据 ③：跨进程持有者挡住两条保存路径；
   判据 ④：本进程的只读会话也挡住）。也就是说**排他性已经成立**，
   翻转不是它的前提，而是"要不要把控制面变成写者"这个**另一个**问题。

### 7.5 第三片之后如实剩下的（因此 `ROAD-M4-008` 仍是「部分」）

1. **"单一写者会话"未建**（§7.4 第 1 条）：`ui/force_save` 的落点
   （`save_project_file`）与控制面会话在同一个工程文件上互斥 ⇒ 控制面存活时 GUI 的
   保存被**拒绝**（fail-closed，判据 ④）。今天不可观测：`ui/force_save` 的 `save_path`
   只由 `LiveWiringOptions` 给，而**生产 `run_gui` 从不构造它**（`live_surface.rs` 只在
   测试目标里被 `#[path]` 装进去）⇒ 生产 GUI 今天没有保存入口。要真的让"人在 GUI 存"
   与"AI 经控制面读/写"同时成立，需要一个宿主保存动作（会话持锁、GUI 委派），
   本片**不做**，设计要点写在 §7.4 第 1 条。
2. **生产窗口仍没有运行期重投影**（第二片 §6.5 第 2 条原样成立，本片不动）：
   本片实测复核了两条事实 —— `crates/yeban-app/src/` + `ui/` 里
   `Timer|start_repeated|invoke_from_event_loop` 命中 **0**（`run_gui` 真的没有任何定时器），
   而"依修订号重投影"的唯一实现 `LiveUi::sync_authority` 只存在于
   `src/live_surface.rs`（dev-dependency 决定它不进产品二进制）。⇒ **本片没有可无头判定的
   实现路径**：要么把 `yeban-ui-test-port` / `yeban-ui-mcp` 拉进产品依赖图（红线，不动），
   要么加一个**没有任何无头判据能界定其调度**的定时器（本仓 DoD 明文拒绝）。
   因此如实**不做**，与第二片同一结论、同一条理由。
3. **`UndoPort` 会话与 `Domain` 会话仍是两个类型**（结构性，第二片 §6.5 第 3 条）。

### 7.6 本机验证原始读数（第三片，2026-10-06）

```text
bash scripts/dev/cargo-local.sh fmt                                                       # exit 0
bash scripts/dev/cargo-local.sh check -p yeban-app                                        # exit 0
bash scripts/dev/cargo-local.sh check -p yeban-app --features in-process-mcp              # exit 0
bash scripts/dev/cargo-local.sh tree -p yeban-app -e normal --locked | grep -c yeban-mcp                      # 0
bash scripts/dev/cargo-local.sh tree -p yeban-app -e normal --locked --features in-process-mcp | grep -c yeban-mcp  # 1
bash scripts/dev/cargo-local.sh clippy -p yeban-app --all-targets -- -D warnings                            # exit 0
bash scripts/dev/cargo-local.sh clippy -p yeban-app --all-targets --features in-process-mcp -- -D warnings # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-app --tests                          # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-app --tests --features in-process-mcp # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-mcp --tests                           # exit 0
```

默认档（`test -p yeban-app --tests`）里与本片直接相关的两行：

```text
tests/cli_contract.rs:        test result: ok. 19 passed; 0 failed; 0 ignored   ← 含本片判据 B14
src/lib.rs (unittests):       test result: ok. 189 passed; 0 failed; 0 ignored  ← 含本片判据 8
```

`--features in-process-mcp` 档：

```text
tests/in_process_mcp.rs:      test result: ok. 3 passed; 0 failed; 0 ignored
tests/in_process_mcp_lock.rs: test result: ok. 5 passed; 0 failed; 0 ignored   ← MUST-GATE-008 的 3 条 + 本片判据 ③ ④
tests/live_ui_mcp.rs:         test result: ok. 20 passed; 0 failed; 0 ignored
```

**依赖图对账（"包集合一个没变"的机械证据，零编译）**：

```text
# 默认树里「唯一的 name vX.Y.Z 集合」：改动前(HEAD 清单) 296 个 / 改动后 296 个，diff 为空
git show HEAD:crates/yeban-app/Cargo.toml > crates/yeban-app/Cargo.toml   # 临时换回旧清单
cargo-local.sh tree -p yeban-app -e normal --locked | grep -oE "[a-z0-9_-]+ v[0-9][0-9.a-z-]*" | sort -u | wc -l   # 296
# …（换回新清单）…                                                                                                    # 296
git status --short Cargo.lock    # 空 ⇒ Cargo.lock 未变（serde_json 早在 yeban-app 的锁定依赖表里）
```

---

## 8. 第 (b) 项（2026-10-06）：生产窗口的**运行期重投影钩子**

> 本片做选项 (a) 剩下的第 2 件事。它**不是**定时器、**不是**后台线程：
> 通知点在控制面**服务完一次请求之后**（`HttpServer::respond`，释放分发器锁之后、
> 写出响应字节之前），跨线程那一跳走 `slint::invoke_from_event_loop`。
> 因此 `ROAD-M4-008` 的第 (b) 项**关闭**；本项整体仍是「部分」，因为第 1 件事
> （"单一写者会话"）与它无关且仍未建（§8.5）。

### 8.1 结构性证据（**改动前**实测；命中数 = "匹配该模式的行数"，命令逐条给出）

| # | 说法 | 改动前读数 | 命令 |
| :--- | :--- | :--- | :--- |
| 1 | **改动前** `crates/` 里**一个** `invoke_from_event_loop` 调用点都没有 | **0** | `grep -rn "invoke_from_event_loop" crates/ --include=*.rs \| wc -l` |
| 2 | `run_gui` 里**没有**任何定时器 / 事件循环投递 | **0** | `grep -rnE "Timer\|start_repeated\|invoke_from_event_loop" crates/yeban-app/src/ crates/yeban-app/ui/ \| wc -l` |
| 3 | 生产窗口的构造点只有一处 | `host::build_main_window` 命中 **1**（`main.rs:139`） | `grep -n "build_main_window" crates/yeban-app/src/main.rs` |
| 4 | `live_surface.rs` **不在**产品 lib 里（只在测试目标里 `#[path]` 装进去） | `grep -c "live_surface" crates/yeban-app/src/lib.rs` = **0** | 该命令本身；装它的只有 `tests/live_ui_mcp.rs:30` |
| 5 | 判据今天**直接调** `sync_authority()`（没有事件循环参与） | 命中 **7**，全在 `tests/live_ui_mcp.rs` | `grep -rn "sync_authority()" crates/yeban-app/tests/live_ui_mcp.rs \| wc -l` |
| 6 | 宿主侧对权威的强句柄 | `ProjectAuthorityHandle`（`mcp_mount.rs`，内含 `Arc<HttpServer>`） | `grep -n "pub struct ProjectAuthorityHandle" crates/yeban-app/src/mcp_mount.rs` |

**第 1 条是必须登记的元问题**：`docs/DEVELOPMENT_LEDGER.md` 第 8962 行写着"the existing
`invoke_from_event_loop` path already does for host calls"（即"这条路已经存在，复用它即可"）。
**实测把这个前提推翻了**：改动前 `crates/` 全树 0 个调用点；`invoke_from_event_loop` 这个词
在本仓只出现在 `docs/**` 与 `.worktrees/.cargo-home/` 里那份**上游 slint 源码**中。
因此本片不是"复用一条已有的路"，而是**第一次把这条路接起来**。
（`DEVELOPMENT_LEDGER.md` 由集成者独占，本片不改它；这一条在此登记并随报告上呈。）

**第 2 条与"为什么不能加定时器"的关系**：无头判据用的 Tier-1 平台
（`crates/yeban-ui-test-port/src/render.rs` 的 `Tier1Platform`）只实现了
`create_window_adapter`，其余走上游默认实现 ⇒ `new_event_loop_proxy()` 返回 `None`、
`run_event_loop()` 返回 `Err(NoEventLoopProvider)`。因此在这个平台上
**定时器根本不会被推进**（没有循环去调 `update_timers_and_animations`），
`invoke_from_event_loop` 也只会返回 `Err`。定时器仍然**不可判定**，本片一个都没加。

### 8.2 本片做了什么

| 文件 | 改动 | 为什么 |
| :--- | :--- | :--- |
| `crates/yeban-mcp/src/transport/http.rs` | 新增 `ProjectRevisionSink` 类型别名 + 私有 `RevisionSink` 字段（`Debug` 只报 `installed`）、`HttpServer::set_project_revision_sink`；`respond` 在**同一把分发器锁**里前后各读一次 `apply_revision`，只在**真的前进**时、**释放锁之后、写出响应之前**通知 | "一次请求真的改过工程"成为**精确读数**（不是两次独立采样）；通知时序对客户端确定 ⇒ 判据不需要 sleep/轮询。观察者是 `Fn(u64)`，**拿不到** `Domain` ⇒ 结构上不是第二个写者 |
| `crates/yeban-app/src/mcp_mount.rs` | `InProcessMcp::set_project_revision_sink`（服务已停机 ⇒ 返回 `false`，不假装装上）；`ProjectAuthorityHandle::downgrade()` + 新类型 `WeakProjectAuthorityHandle` | 观察者装在 `HttpServer` **内部**；若它（或它 marshal 的闭包）持强句柄就形成 `HttpServer → 观察者 → Arc<HttpServer>` 引用环 ⇒ 停机后监听口不关。弱句柄切断这条环（§8.3.1 负向实测 ③ 就是这个环的实测证据） |
| `crates/yeban-app/src/reproject.rs`（新，`cfg(in-process-mcp)`） | `AuthorityMirror`（窗口弱引用 + 权威弱句柄 + `projected` 修订号）+ `ProjectionOutcome` 五态 + `sync_now` / `sync_weak` / `event_loop_sink` / `install` | 生产路径的**唯一**重投影钩子：读权威（只读口）→ `ViewState::from_project` → `host::apply_view` 注入活窗口。`event_loop_sink` 是生产驱动（`invoke_from_event_loop`） |
| `crates/yeban-app/src/main.rs` | `run_gui` 在进事件循环之前 `AuthorityMirror::install(&ui, mount)`，并打一行"重投影: 已装 / **未装**"的报告 | 装了才叫"生产路径上有钩子"；**未装时出声**（服务已停机），不留一个"看起来装了"的空钩子。返回值持到事件循环结束 |
| `crates/yeban-app/src/lib.rs` | `pub mod reproject;`（同一个 `cfg`） | 默认构建里这个模块**不存在** ⇒ 产品依赖图一位没变 |
| `crates/yeban-ui-test-port/src/inspect.rs` | `install_testing_backend_with_event_loop()`（转发上游 `init_integration_test_with_mock_time`） | 无头判据要**真的**跑 `invoke_from_event_loop` 这一跳，就必须有一个**带事件循环**的无头平台。上游那一档（`threading: true`）每进程只能装一次 ⇒ 判据目标里只能有一个 `#[test]`（文件头写明了） |
| `crates/yeban-app/tests/production_reprojection.rs`（新） | 判据（下方 §8.3） | 这条判据是"第 (b) 项闭合"与"没闭合"的分界线 |

**没有动的东西（都是刻意的）**：`read_only = true`、`SessionSource::lock_mode()`（仍 `SharedRead`）、
`acquire_session_lock`、`undo.rs` 的任何一行、`Domain` 的任何一行、`Domain::apply` 的推进点、
`crates/yeban-app/Cargo.toml` 与 `Cargo.lock`（实测未变，见 §8.6）、
`in-process-mcp` / `experimental-als-export` 的 feature 定义、`MUST-GATE-009` 的四条。

### 8.3 判据（有牙，且能失败）

`crates/yeban-app/tests/production_reprojection.rs::a_session_side_mutation_reaches_the_production_window_through_the_runtime_hook`
（`--features in-process-mcp`；文件里**只有这一个** `#[test]`，理由见 §8.2 的平台约束）

| 步 | 动作 | 实读 |
| :--- | :--- | :--- |
| 1 | 用**生产构造入口** `host::build_main_window` 建窗口（工程取自权威） | 窗口的自动化泳道 1 条，`track-0-automation-pan-lane` **不在** |
| 2 | `AuthorityMirror::install` 把观察者装到控制面上 | 起点"已投影修订号" == 权威当前修订号（0） |
| 3 | **会话侧**：真环回 socket + 真令牌 + 真客户端发 `yeban_edit_automation`（`TrackPan` 写一个点） | `status = success`；权威修订号 **0 → 1** |
| 4 | 钩子**还没跑**，读同一个活窗口 | 窗口泳道仍是那 **1** 条（旧投影）—— 会话侧的改动**没有**同步溜进 UI 线程 |
| 5 | 同一时刻读**权威工程**的投影 | 权威**已经**有那条新泳道 ⇒ 第 4 步是真的缺口，不是巧合 |
| 6 | `quit_event_loop()` + `run_event_loop()`（无头平台 FIFO 弹出那一条排队调用） | 钩子在 **UI 线程**上执行（没有 sleep / 轮询 / 超时） |
| 7 | 再读同一个窗口 | 泳道 **1 → 2** 条，新标签 `Lead · 声相 自动化 0.250` **逐字等于**权威工程的投影；`projected_revision` == 1 |
| 8 | 一次**只读** `yeban_query_project` + 再跑一轮事件循环 | 修订号不动 ⇒ `respond` 里 `advanced == false` ⇒ 观察者**根本没被调用**；窗口泳道逐字不变 |
| 9 | 直接再驱动一次钩子本体（`sync_now`） | `Unchanged`（幂等：已经投影过这一版） |
| 10 | `drop(authority)`（判据自己那个强句柄）后 `mount.stop()` | 监听口真的关了（`TcpStream::connect_timeout` 失败）⇒ 观察者持的是**弱**句柄，没有引用环 |

**为什么第 4 步与第 8 步是"牙齿"而不是装饰**：第 4 步把"marshal 到 UI 线程"与"在服务线程上
直接重投影"区分开；第 8 步把"事件驱动"与"每次请求都刷一遍"区分开。两者都不是恒真的断言。

### 8.3.1 负向实测（先红后还原；四条方向各不相同）

| # | 临时改哪里 | 红在哪 | 实读（原文） |
| :--- | :--- | :--- | :--- |
| ① | `AuthorityMirror::install` 里**不装观察者**（`set_project_revision_sink` 不调用） | 步 7（判据 `:199`） | ``钩子跑过之后生产窗口里必须有 `track-0-automation-pan-lane` 的标签 `Lead · 声相 自动化 0.250`: ["Lead · 音量 自动化 -6.0 dB · 录制臂 触碰"]``，`test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out` |
| ② | `sync_now` 算出投影但**不注入**（摘掉 `host::apply_view`） | 步 7（判据 `:199`） | 与 ① 同一行、逐字相同（事件循环**真的**跑过了，只是没画） |
| ③ | `event_loop_sink` 里让观察者持一个**强** `ProjectAuthorityHandle`（引用环） | 步 10（判据 `:261`） | `停机之后不该还有东西在监听 127.0.0.1:50874：钩子若持有强句柄就会形成 HttpServer → 观察者 → Arc<HttpServer> 的引用环，这里会变红` |
| ④ | `event_loop_sink` **不 marshal**，直接在服务线程上调 `sync_weak()` | 步 7（判据 `:199`） | 与 ① 同一行、逐字相同 —— 因为上游 `Weak::upgrade()` 在**非创建线程**上返回 `None`（`i-slint-core/api.rs:1171-1177`），所以"在错误的线程上重投影"是一次**确定的**空操作，不是一个偶发 panic |

四条都在**最终代码状态**上复跑过一次（lint 修复之后），四条都红、四条都还原：每次还原后
`cmp` 与基线逐字节相同、`shasum -a 256 crates/yeban-app/src/reproject.rs`
= `8d73bc1c8201c6416ee9b0a7fd62709772ff75b49a3a88639f0dc5b3e10a0391`（四次都一样），
`grep -rn "NEGATIVE MEASUREMENT" crates/` = **0**。

### 8.4 为什么这个形态是**可判定**的（而不是"又一个无法判定的定时器"）

| 问题 | 答案 |
| :--- | :--- |
| 谁在什么时点通知？ | 控制面**服务完一次请求**之后（`HttpServer::respond`）。定义好的、由请求到达驱动 —— 不是时钟 |
| 跨线程那一跳会不会丢？ | 不会：`slint::invoke_from_event_loop` 把闭包排进平台的事件循环队列；生产平台（`backend-winit`）实现了 proxy |
| 判据怎么**确定**它已经排进去了？ | 通知发生在**写出响应字节之前** ⇒ "客户端读到响应"蕴含"已经排进队列"。判据因此不需要 sleep / 轮询 / 超时 |
| 判据怎么**确定**那一跳会跑？ | 判据自己压一条 `Quit` 再进 `run_event_loop()`；无头平台是 FIFO 队列 ⇒ 先跑重投影、再看到 `Quit`，循环必然返回（不会 park 在空队列上） |
| 没有事件循环的形态怎么办？ | `invoke_from_event_loop` 返回 `Err(NoEventLoopProvider)` ⇒ **出声**打一行 stderr，并且**不**推进 `projected`（下一次通知会再试）。**没有**静默降级 |

### 8.5 第 (b) 项之后如实剩下的（因此 `ROAD-M4-008` 仍是「部分」）

1. **"单一写者会话"未建**（§7.4 的第 1 条，本片未动）：GUI 的保存落点
   （`save_project_file`）与控制面会话在**同一个**工程文件上互斥 ⇒ 控制面存活时 GUI 保存被拒
   （fail-closed，判据 ④）。要同时成立须给宿主加一个**保存动作**并让 GUI 的 `ui/force_save`
   走它。注意这一条是**磁盘写者**的边界，与内存权威无关。
2. ~~生产窗口仍没有运行期重投影~~ ⇒ **本片关闭**。逐条对照 §7.5 第 2 条：
   `run_gui` 今天**有**钩子了（`AuthorityMirror::install`），它由 `respond` 的通知驱动、
   经 `invoke_from_event_loop` 在 UI 线程上执行；判据在**生产构造入口**建出来的窗口上
   端到端跑通（§8.3）。**不是**定时器，因此不触碰"无头判据界定不了调度"那条 DoD。
3. **`UndoPort` 会话与 `Domain` 会话仍是两个类型**（结构性，§6.5 第 3 条，本片未动）。
4. **启动时仍有两份 `YebanProjectV1` 值**（§6.5 第 3 条）：`loaded.archive.project` 是
   建窗口的**只读初值**、从不被写，因此不是第二个**可变**权威；本片让"之后"由权威驱动，
   "启动那一次"仍复用 `host::build_main_window`。

### 8.6 本机验证原始读数（第 (b) 项，2026-10-06）

```text
bash scripts/dev/cargo-local.sh fmt                                    # exit 0
bash scripts/dev/cargo-local.sh fmt --check                            # exit 0
bash scripts/dev/cargo-local.sh check -p yeban-app                     # exit 0
bash scripts/dev/cargo-local.sh check -p yeban-app --features in-process-mcp          # exit 0
bash scripts/dev/cargo-local.sh clippy -p yeban-app --all-targets -- -D warnings      # exit 0
bash scripts/dev/cargo-local.sh clippy -p yeban-app --all-targets --features in-process-mcp -- -D warnings  # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-app --tests                             # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-app --tests --features in-process-mcp    # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-mcp --tests                              # exit 0（重跑；首轮 lock_advisory 一条偶发红，见下）
bash scripts/gates/run-gates.sh light                                  # exit 0，末行 `门禁通过 (mode=light)`
```

与本片直接相关的判据行（`--features in-process-mcp` 档）：

```text
tests/production_reprojection.rs:  test result: ok. 1 passed; 0 failed; 0 ignored
```

默认档里这个目标存在但 0 用例（feature 关着：`mcp_mount` / `reproject` 两个模块都不在编译里）：

```text
tests/production_reprojection.rs:  test result: ok. 0 passed; 0 failed; 0 ignored
```

**一条偶发红（如实登记，不是本片引入）**：首轮 `test -p yeban-mcp --tests` 里
`tests/lock_advisory.rs::another_process_cannot_open_a_project_we_hold_exclusively`
红在 `握手标记必须是 JSON: EOF while parsing a value at line 1 column 0: `（`test result: FAILED. 13 passed; 1 failed`）。
它是本仓**已登记**的百年老 flake（`docs/DEVELOPMENT_LEDGER.md` 第 279 / 288 轮：
"one occurrence in seven observations"），且与本片无关 —— 那条断言量的是**子进程的握手行**，
发生在任何 HTTP 请求（也就是 `respond` 里那个通知点）之前。随后单跑 3 次 + 整套重跑 1 次全绿
（`14 passed` ×3，整套 11 个目标全 ok）。

**依赖图对账（"包集合一个没变"的机械证据，零编译）**：

```text
# 默认树里「唯一的 name vX.Y.Z 集合」：本轮改动前 296 个 / 改动后 296 个
bash scripts/dev/cargo-local.sh tree -p yeban-app -e normal --locked --prefix none \
  | grep -oE "^[a-zA-Z0-9_-]+ v[0-9][0-9.a-zA-Z-]*" | sort -u | wc -l    # 296
# 清单与锁文件逐字节未动（空输出）：
git status --short -- Cargo.toml Cargo.lock crates/yeban-app/Cargo.toml \
  crates/yeban-mcp/Cargo.toml crates/yeban-ui-test-port/Cargo.toml
```
