# `M4-008` 工作线台账 —— UI↔领域**唯一可变权威**（`open-questions.md` 问题 6 选项 (a)：第一片 + 第二片）

> 授权：`docs/ledger/human-decisions.md` 头部声明它是**唯一**需要人类裁决的清单，其条目按**建议**列
> 在负责人"按你的建议来"的长期授权下执行（**不是** Agent 自放行）。本行按
> `docs/ledger/open-questions.md` 问题 6 的 **(a) 建议**执行。
> §1–§5 是选项 (a) 的**第一片**（投影口）；§6 是**第二片**（GUI 的**写**入口也落到该权威上）。
> 问题 6 与 `ROAD-M4-008` 因此**仍是「部分」** —— 如实剩下的那一步写在 §6.4。

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
