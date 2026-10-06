# ADR-0005 — GUI 进程里唯一可变的工程权威（选项 (a) 的追认、投影注入面与 `.yeban.lock` 写者边界）

- **状态**: **Proposed（待人类裁决）**
- **决定者**: 集成者（Agent）在负责人授权范围内提出并**追认已落地的实现**；「为什么 `ROAD-M4-008` 需要的那个"新的设计裁决"至今没有 ADR」「投影要不要像素级证据」「宿主句柄要不要按读/写分型」这三族口径的最终裁决归负责人
- **关联**: 账本行 = `docs/ledger/phase-status.md:106`（`ROAD-M4-008`，**该行今天已是「已完成」**，见背景 (a2)）；
  工作线台账 = `docs/ledger/m4-008-authority-notes.md`（§1 结构证据、§6.5/§7.5/§8.5/§9.5 逐片"剩下什么"）；
  裁决先例 = `ADR-0001` `D10`（规范缺口写进 `docs/adr/` 而不是由实现者自选）、`D28`（纯函数投影 + 唯一注入点 + 整数位置）、`D29`（`ui/*` method 与 scope 归属；**六级 scope 里没有"界面状态写"**）、`D43`（缺键响亮失败）；
  规范 = `[MCP-DUAL-001]`（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:643`）、`[ARCH-SEC-002]` §7.2（六级 scope，`:602-615`）、`[ARCH-RT-001]` / `MUST-GATE-001`、`MODEL-ISO-001`；
  门禁 = `MUST-GATE-001`（`docs/ledger/gate-status.md:18`）、`MUST-GATE-008`（`:25`）、`MUST-GATE-009`（`:26`）、`MUST-GATE-015`（`:32`）；实时台账 = `docs/ledger/gate-rt-zero-alloc-notes.md`；
  触点 = `crates/yeban-app/src/{main,host,undo,live_surface,reproject,mcp_mount,save_action,save,project_lock,elements,bridge}.rs`、`crates/yeban-mcp/src/domain/mod.rs`、`crates/yeban-mcp/src/transport/http.rs`、`crates/yeban-app/ui/transport.slint`、`crates/yeban-app/tests/{live_ui_mcp,production_reprojection,single_writer_session,in_process_mcp_lock}.rs`、`crates/yeban-app/tests/golden/linux/`
- **日期**: 2026-10-07

> **测量口径**（`AGENTS.md` §6）：本文每个数字都带**对象 + 单位 + 位置**；`file:line` 都在本机读过。
> 凡属「名字命中」而不是「已接线能力」的，一律照实说成名字。
> 凡属**被引用文档里的警告**（例如实时台账 §5.2 的"这不是系统调用级拦截"），本文照引为**证据**而不是转述成结论。
> `#[test]` 的**属性行数**与**执行条数**不是同一个量（feature 门控），本文凡写条数都注明是哪一个。

## 背景

### (a) 账本那一行点名的是什么（逐字引用它自己的话）

`docs/ledger/phase-status.md:106`（`ROAD-M4-008`「双 MCP 服务器协同自测闭环」）的**证据列**里，
把缺口写成两半，并给出结构性原因与"这不是再写点代码"的判断：

> **已做到（UI 侧闭环）**：`crates/yeban-app/tests/live_ui_mcp.rs` 在 CI 上真跑"建立 LivePort → `ui/tree` 拉元素树 JSON →
> 按语义 ID 查节点 → `ui/screenshot` 取 Framebuffer PNG"…… **未做到**：闭环的**域侧那一半** ——
> "AI Agent 注入**编曲意图**"经 `yeban-mcp` 驱动、与 UI 侧在**同一个** CI 用例里协同，没有证据……
>
> **结构性原因**：`mcp_mount::session_dispatcher` 走的是 `Domain::open_in_memory(path, project.clone(), true)`，
> 而界面侧的权威可变实例是 `UndoPort`（持 `RefCell<UndoSession>`）与 `live_surface::LiveSurface.project` ——
> 三者**各持一份 `YebanProjectV1` 克隆**；`Domain` 又刻意不实现 `Clone`、自己持有 `Active::project`，
> **没有任何"外部可变工程"注入点**。因此要关掉这半条必须做一个**新的设计裁决**
> （谁是唯一可变权威：让界面从 `Domain` 投影，还是把 `Domain` 的 `apply` 接到宿主 sink
> 并把 UI 重投影编组进 Slint 事件循环），并连带决定写会话取 `ExclusiveWrite` 后与 GUI 自身保存路径的锁语义。

同一行的备注列补充了后半段历史（`Domain::apply_revision`、`HttpServer::host_domain`、`InProcessMcp::project_authority()`
是"选项 (a) 的第一片"），并明确写着"**锁与写者边界一位没动，且这是刻意的**"。

### (a2) ⚠ 先说一条会改变本文读法的实测：那一行今天已经是「已完成」

本文是**在缺口被关掉之后**写的。不给这条，后面每一句都会被误读成"在描述一个还开着的洞"。

| 事实 | 读数 | 命令 / 位置 |
| :--- | :--- | :--- |
| `ROAD-M4-008` 行的**状态格**（第 3 格） | **`已完成`** | 解析 `docs/ledger/phase-status.md:106` 的 `\|` 分格（7 格，`cells[3].strip() == "**已完成**"`） |
| 是谁翻的 | 提交 **`0763bc8`**（`feat(m4-008): a single-writer session — the mounted control plane owns the project document`） | `git log --oneline -S "本行升级为「已完成」" -- docs/ledger/phase-status.md` |
| 第二片是谁 | 提交 **`a2fd2b1`**（`feat(mcp,app): the GUI's write entry points land on the single mutable authority [ROAD-M4-008]`） | `git log --oneline -S "第二片：GUI 的写入口也落到该权威上" -- docs/ledger/phase-status.md` |
| Phase 4 逐行统计 | 11 条 = 已完成 **7** / 部分 **4** / PENDING **0** | `docs/ledger/phase-status.md:121`；逐行解析 `## 6. Phase 4` 段得 `M4-001..005, 008, 009` = 已完成、`M4-006, 007, 010, 011` = 部分 |
| 工作线台账自己的话 | "本片之后如实剩下的（因此 `ROAD-M4-008` **可以离开「部分」**）" | `docs/ledger/m4-008-authority-notes.md:684`（§9.5） |

⇒ **那一行**描述的两件事都已经落地：① "界面从 `Domain` 投影"（选项 (a) 第一片）；
② "`Domain` 的 `apply` 接到宿主 sink + 把 UI 重投影编组进 Slint 事件循环"（第 (b) 项，`reproject.rs`）；
外加 ③ 单一写者会话（§9）与 ④ 写会话与 GUI 保存路径共用同一把 `.yeban.lock`（§7）。
因此本文的定位**不是**"提请裁决一个新洞怎么补"，而是三件事：

1. **追认**：那个被账本点名"必须做"的设计裁决，是**在代码里分五个提交做掉的**，
   至今**没有一份 ADR**（`docs/adr/` 只有 `ADR-0001..0004`，见 `docs/adr/README.md` 索引表的 4 行）。
   本 ADR 把它补成可复核、可推翻的记录；
2. **登记仍未裁决的口径**（锁的可达性、投影要不要像素证据、宿主句柄要不要分型、无活跃工程时界面怎么办）；
3. **登记仍未闭环的证据**（账本自己的措辞："**本提交的 CI 判决待集成者推送后读回**"，`phase-status.md:106` 末尾）。

### (b) 规范的空隙（因此按 `ADR-0001 D10` 写在这里，不改 Normative 正文）

- `[MCP-DUAL-001]`（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:643`）规定"领域 MCP 与 UI 测试适配层是
  **两个** JSON-RPC 端点"、"AI Agent 注入编曲意图 → 自测闭环"。它规定了**两端点**与**闭环**，
  但**没有任何一处**规定"两个端点服务的是不是同一份工程"、也没有规定"谁是唯一可变权威"。
- `[ARCH-SEC-002]` §7.2（`:602-615`）给出**六级** scope：`ui:read` / `ui:screenshot` / `ui:inject` /
  `app:save` / `app:reload-engine` / `app:admin` —— **没有"界面状态写"**这一级。
  `ADR-0001 D29` 已把它登记为待人类裁决，并让 `ui/switch_main_view` 暂挂 `app:admin`。
- `MUST-GATE-008`（`gate-status.md:25`）规定 `.yeban.lock` 的 OS 建议锁与 `PROJECT_LOCKED`，
  并记录"**仍待**: 把该门禁接进 `ci.yml` 的受影响集合（现在是手动档）"。
  它**没有**规定"写会话存活时，同一个进程里另一条保存路径该被拒还是该委派"。
- `MUST-GATE-001`（`gate-status.md:18`）规定实时回调四元组为零，但**没有**规定"工程变更的投影允许在哪一线程上分配内存"。

### (c) 逐条核对那一行的结构性说法（`file:line`；**含已过期的**）

| # | 那一行的说法 | 判定 | 证据（本机实测） |
| :--- | :--- | :--- | :--- |
| 1 | `mcp_mount::session_dispatcher` 走 `Domain::open_in_memory(path, project.clone(), true)` | **结构真、字面过时** | `crates/yeban-app/src/mcp_mount.rs:873` 仍是 `open_in_memory(path.to_path_buf(), `**`project.clone()`**`, read_only)`；第三个实参**不再是字面 `true`**，而是 `:653` 的 `source.session_read_only()`（`:283-285`：`File`/`InMemory` ⇒ `true`，**`WritableFile` ⇒ `false`**）⇒ "只读克隆"的说法对**只读形态**成立，对**生产 GUI 今天的形态**不成立 |
| 2 | 界面侧权威可变实例是 `UndoPort`（`undo.rs` 持 `RefCell<UndoSession>`） | **已过期** | `crates/yeban-app/src/undo.rs:262` 是 `session: RefCell<`**`UndoBackend`**`>`；`UndoBackend` 在 `:246-251`，两个变体 `Local(Box<UndoSession>)` / **`Authority(ProjectAuthorityHandle)`**；`grep -c "RefCell<UndoSession>" crates/yeban-app/src/undo.rs` = **0**（命中数 = 匹配该模式的行数）。挂载时端口**不持有任何工程副本**（`main.rs:197` `UndoPort::from_authority`） |
| 3 | `live_surface::LiveSurface.project`（当时 `:277,412`）另持一份 | **字段真、行号过时、且不在产品进程里** | 字段在 `crates/yeban-app/src/live_surface.rs:322`；但 `src/live_surface.rs` **只被测试目标** `#[path]` 装入（`crates/yeban-app/tests/live_ui_mcp.rs:30`；`Cargo.toml:207` 的注释写着它依赖 `yeban-ui-mcp`/`yeban-ui-test-port` 两个 **dev-dependency**）⇒ 产品二进制里**没有**这个字段。且它在 `:319-322` 的文档里已自我声明为**投影缓存**（由修订号驱动，唯一刷新点是 `:509` `sync_authority`） |
| 4 | `Domain` 刻意不实现 `Clone`、自己持有 `Active::project` | **真** | `crates/yeban-mcp/src/domain/mod.rs:147`（"**刻意不实现 `Clone`**：它内涵 `.yeban.lock` 的 RAII 守卫与提交图谱，克隆会产出两个独立的写者"）、`:150` `pub struct Domain`；`struct Active` 在 `:100`、`project: YebanProjectV1` 在 `:105`；全文件 `impl Clone for Domain` **0 处** |
| 5 | "**没有任何**'外部可变工程'注入点" | **已过期** | 注入点是**有**的，只是形状是"句柄"而不是"外部可变工程"：`Domain::apply_revision`（`domain/mod.rs:234`）、`HttpServer::host_domain`（`transport/http.rs:581`，签名 `impl FnOnce(&crate::domain::Domain) -> R`，**没有 `&mut`** ⇒ 结构上不是第二个写者）、`HttpServer::apply_host_action`（`:606`）、`HttpServer::host_save_project`（`:642`）、`HttpServer::set_project_revision_sink`（`:674`）、`InProcessMcp::project_authority`（`mcp_mount.rs:699`） |
| 6 | 那个 `ProjectAuthorityHandle` 是**只读**的 | **已过期（且是一条仍在仓库里的过期文档）** | `mcp_mount.rs:467-471` 的句柄今天还有 `apply_host`（`:518`，GUI 写入口）与 `save_to`（`:545`，**落盘**写入口）；而 `:697` 的文档仍写着"而且它**只能读**" —— **这句与 `:518`/`:545` 直接矛盾**，是第二片/§9 之后未同步的文档漂移（本文只登记，`crates/**` 不在本 ADR 的授权范围内） |
| 7 | "阻塞不是'缺显示'：`live_ui_mcp.rs` 已用 Tier-1 testing backend 无头跑真 `MainWindow`，缺的是这条单实例接线" | **真，且这条接线已接上** | 判据 `live_ui_mcp.rs::an_mcp_mutation_reaches_the_live_ui_projection_through_the_single_authority`（`:1704`）与生产侧 `production_reprojection.rs::a_session_side_mutation_reaches_the_production_window_through_the_runtime_hook`（`:103`） |

### (d) 今天进程里到底有几份 `YebanProjectV1`（实测：按"存储位置"数，不按"类型出现次数"数）

**口径**（`AGENTS.md` §6.5：不把一种计数当成另一种计数）：下表数的是
`grep -rn "project: YebanProjectV1\|project: yeban_model::YebanProjectV1\|: YebanProjectV1," crates/yeban-app/src/*.rs`
命中的**存储位置**，加上跨 crate 的 `Domain::Active::project`；**不含**函数参数与返回的临时克隆。

| # | 存储位置 | 谁拥有 | 产品 GUI 里活着吗 | 可变吗 | 权威吗 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| 1 | `crates/yeban-mcp/src/domain/mod.rs:105`（`Active::project`） | 控制面 `Domain`（`open_in_memory` 收下 `mcp_mount.rs:873` 的那份克隆） | **是**（生产 `run_gui` 用 `SessionSource::WritableFile`，`main.rs:373`） | **是**（唯一可变入口 `domain::apply`，`mod.rs:1979`） | **是** —— 编辑的唯一权威 |
| 2 | `crates/yeban-app/src/host.rs:67`（`SaveStatus.project`） | `host::wire_save`（`:476`）装进 `on_save_project` 闭包的那份 | **是**（`main.rs:247-251` 用 `loaded.archive.project.clone()` 装进去，活到窗口销毁） | 否（`:485-499` 只读它去构造一次 `SaveRequest`） | **否、且挂载时不被使用**：`:474` 与 `save_action.rs:142-161` 明写"有权威时 `SaveRequest::project` **不被使用**，字节由权威自己产出" |
| 3 | `crates/yeban-model/src/container/mod.rs:182`（`ProjectArchive::project`，经 `crates/yeban-app/src/open.rs:288` 的 `LoadedProject.archive`） | `run_gui` 的 `loaded` 局部量 | 是（**早期**：初始投影 `main.rs:139` + 引擎装载 `:213`；最后一次使用是 `:251` 的克隆，之后 NLL 即可回收） | 否（只读输入） | 否 |
| 4 | `crates/yeban-mcp/src/undo_session.rs:726`（`UndoSession.project`） | `UndoBackend::Local(Box<UndoSession>)`（`undo.rs:246-249`） | **只在"没挂控制面"时**（默认构建 / 运行期开关关着 / 拿不到锁 ⇒ `main.rs:198`、`:205`、`:331`） | 是 | **那时**是（那时进程里**没有**第二个写者） |
| 5 | `crates/yeban-app/src/live_surface.rs:322`（`LiveSurface.project`） | 测试装配（`#[path]` 装入 `live_ui_mcp.rs`） | **否**（dev-dependency 决定的，`Cargo.toml:207`） | 否（`apply_project` 是唯一写入点，`:459`） | 否（**投影缓存**，刷新由 `:509` `sync_authority` 依修订号驱动） |

**⇒ 结论（产品 GUI 进程，挂载成功时）：持久存活的 `YebanProjectV1` 是 **2** 份**（#1 权威 + #2 保存回执的只读副本），
**#3 是启动期的有界局部量**，**#4 只在没挂载时取代 #1**，**#5 根本不在产品进程里**。
账本那一行"三者各持一份克隆"描述的是 **`WritableFile` 出现之前**的形态；
今天"三份里有一份（`LiveSurface.project`）不在产品进程、一份（`UndoPort`）已经改成句柄"。

**每次读写会产生临时克隆**（不计入上表，但值得一提）：`ProjectAuthorityHandle::project()`
（`mcp_mount.rs:479`）返回 `Option<YebanProjectV1>`（**一份快照**），`host.rs:498` 每次保存前
`port.project.clone()`。它们都是**函数内的有界临时值**，不是第二个权威。

### (e) 域侧 `apply` 改了自己那份之后，界面看得见吗？

**挂载时：看得见，而且是事件驱动的**（这正是"闭环"落地的那一半）：

```text
会话侧一次真的改了工程的请求
  └─ yeban-mcp 的 HttpServer::respond：释放分发器锁之后、写出响应之前
       └─ ProjectRevisionSink::notify(新修订号)        （transport/http.rs:459,674）
            └─ AuthorityMirror::event_loop_sink 的闭包（跑在服务线程上，reproject.rs:198）
                 └─ slint::invoke_from_event_loop(...)  ← 唯一的跨线程手段
                      └─ UI 线程：sync_weak → sync_now（reproject.rs:148,182）
                           └─ ViewState::from_project → host::apply_view（host.rs:119）
```

**没挂载时**：`Domain` 不存在，界面画的是 #4（`UndoSession`），没有会话侧改动这回事。

**#2（`SaveStatus.project`）永远看不见域侧改动** —— 但它是**刻意的**：挂载时保存**根本不读它**
（`save_action.rs:161` 先判 `authority`，`Some` ⇒ 只走 `ProjectAuthorityHandle::save_to`，本地路径**不可达**）。
⇒ **不存在"界面缓存陈旧 ⇒ 写错内容"这条路**，这是 §9 的"为什么不是把字节交给宿主"那一节逐字记下的设计理由。

### (f) 代码强制的约束（选项只能在它们之内）

| 约束 | 出处 | 对选项的限制 |
| :--- | :--- | :--- |
| **投影是纯函数、注入点只有一处、位置是整数** | `ADR-0001 D28`（`ADR-0001:295-308`）；实现在 `bridge.rs` + `host.rs:119` | 任何"第二处注入"或"在 `.slint` 里算"的方案**先行出局** |
| **实时回调四元组为 0**（`allocations`/`deallocations`/`lock_blocking`/`lock_waits`，另加 `io_requests`/`io_ops`） | `MUST-GATE-001`（`gate-status.md:18`）；实测 22/22、**27 764 量子 + 1 063 次快照交换**、本机 3.22 s；台账 `gate-rt-zero-alloc-notes.md:27-40` | 任何"把权威或投影放到音频线程上"的方案**先行出局** |
| **该门禁只看得见"经由边界"的锁与 I/O** | `gate-rt-zero-alloc-notes.md:288-296`（§5.2 原文："**这不是系统调用级拦截，也没有内核钩子**……真实防线是**两半**：约定 + 判据"） | 不能拿"四元组为 0"当"新写一把裸锁也安全"的证明 —— 这是**引用文档自己的警告**，照引 |
| **`.yeban.lock` 跨进程互斥 + `PROJECT_LOCKED`**，且**只读者共存**是一条独立腿 | `MUST-GATE-008`（`gate-status.md:25`）；实测裁决：把已有的 `SessionSource::File` 翻成写形态 ⇒ `in_process_mcp_lock.rs` **3 条红**（"只读…必须是共享读者 / left: Some(ExclusiveWrite) / right: Some(SharedRead)"），见 `phase-status.md:106` 末段 | 任何"把所有形态都改成排他写"的方案会**拆掉这条腿** |
| **默认 release 不链接网络监听代码** | `MUST-GATE-009`（`gate-status.md:26`）+ `AGENTS.md` §2 红线 6 | 任何"让默认构建也挂 `yeban-mcp`"的方案**先行出局** |
| **写入确定性** | `ARCH-DET-001` | 任何让写入顺序依赖锁调度的方案**先行出局** |

## 闭环判据（今天已存在的那一条 + 它还没断言的东西）

**已存在的同案闭环判据**：`crates/yeban-app/tests/live_ui_mcp.rs:1704`
`an_mcp_mutation_reaches_the_live_ui_projection_through_the_single_authority`（**一个**用例、**一个**活窗口）。
它断言的是（逐步用上一步的实读值）：

| 步 | 动作 | 断言 |
| :--- | :--- | :--- |
| 1 | 用 `build_live_ui_from_authority` 装配（该入口**不接受**工程参数，`live_surface.rs:1054`） | 起点：音量泳道在树里、目标泳道 `track-0-automation-pan-lane` **不在** |
| 2 | **域 MCP**：真环回 socket + 真 `Bearer` 令牌发 `tools/call yeban_edit_automation` | `status == success`、`applied == true` |
| 3 | 从宿主口读**同一会话** | 施加修订号**恰好 +1**；权威工程里**真的**多了那条泳道 |
| 4 | 重投影 | `AuthoritySync::Reprojected { revision }`（**不是** `Unchanged`） |
| 5 | 再读**同一个**活窗口的运行时控件树 | 新元素在树里、节点数增加、`accessible-label` **逐字等于权威工程的投影** |
| 6 | 一次**只读** `tools/call yeban_query_project` | 修订号不动、`Unchanged`、树长度一位不变 |
| 7 | **UI MCP** 端到端 `ui/tree` / `ui/node` | `probe.node.id == lane_id`、标签逐字相等、`ui/tree` 的**线上文本**里含该标签 |

⇒ **两个 MCP 面在同一个用例里被真的驱动了**，且"域侧意图 → 界面跟随"是一条**因果链**（步 2 的响应蕴含步 3 的读数）。
这就是 `ROAD-M4-008` 要的那条"同一个 CI 用例"。

**它还没断言的东西（本文的实测缺口，值得负责人知道）**：

1. **它停在元素树/标签层，没有停在像素层。** `ui/screenshot` 只在**另一条**判据里被断言
   （`live_ui_mcp.rs:75` `live_control_plane_reads_the_project_backed_window_end_to_end`：尺寸非零、非全黑、
   与"直接从窗口抓的帧"逐字节同一、连续两次指纹相同）。
   判据 17 的 `plane.probe(...expecting_size(...))` 会抓帧，但它断言的是 `node.id`/`node.label`/`tree_json`，
   **没有**断言"这一帧与改动前那一帧的差异恰好落在新泳道的包围盒里"。
2. **Golden 在同一个用例里帮不上忙**：`crates/yeban-app/tests/golden/linux/` 有 **5** 张 PNG（另加 `MANIFEST.txt`），
   `MUST-GATE-015`（`gate-status.md:32`）要求它们由 Tier-1 软光栅化产出；而 `test_port_adapter.rs:174` 的 `assert_matches_golden`
   在**平台目录不存在**时只打印"视觉回归**未被判定**（不等于通过）"并 `return`。
   本仓**只有** `linux/`，所以本机 macOS 上这条**什么都不判**。
3. **签名口径**：同案闭环的域侧调用走的是**生产 RunMode**（`mcp_mount.rs:870` 的
   `Dispatcher::new(token, ScopeSet::all(), RunMode::Production)`），而生产模式下 `ui:inject` 是**硬禁**的
   （`MUST-GATE-009`；`live_ui_mcp.rs:1144` 的报错文本 `需要 {required}, 当前 ui:read,ui:screenshot` 是它的现场读数）。
   ⇒ 闭环**不能**靠"往界面注入事件"来实现，只能靠"读界面（`ui:read`/`ui:screenshot`）+ 写域（域 MCP）"。

**⇒ 一个"更闭合"的同案判据要断言什么（若负责人要像素证据）**：在**同一个**用例里抓**两帧**
（域侧 `tools/call` 之前 / 重投影之后），断言 ① 两帧**不是逐字节相同**（否则"界面没跟着动"也会绿）；
② 差异的**包围盒**落在新泳道的投影几何内（`bridge.rs` 的 `lane y/height` 派生，`D28` 的整数口径）；
③ 差异外区域**逐字节相同**（排除"整屏重排"这种假跟随）；④ 指纹可复现（同一次运行两次抓帧同指纹）。
—— 这条**不需要**动 golden（它比较同一次运行的**两帧**，不与提交过的基准比），因此**不需要** Golden 再生成，
也**不受**"平台目录不存在就跳过"的影响。

## 裁决一览

| # | 问题 | 建议 | 归负责人？ |
| :-- | :--- | :--- | :--- |
| Q1 | GUI 进程里谁是唯一可变权威 | **A（界面从 `Domain` 投影）+ B（`apply` 经宿主 sink 通知、`invoke_from_event_loop` 编组）**；**否决** C（共享 `Arc<Mutex<YebanProjectV1>>` 单一存储） | **是**（追认这一选择） |
| Q2 | 写会话取 `ExclusiveWrite` 后与 GUI 保存路径的锁语义 | **单一持锁者 + 宿主保存动作**（GUI 保存**委派**给会话，不并存两条路径） | **是**（"只读会话存活时 GUI 本地保存是否该被拒"是 UX 口径） |
| Q3 | 重投影发生在哪、实时路径上不许做什么 | **纯投影（`bridge::from_project`）+ 唯一注入点（`host::apply_view`），由服务线程通知、UI 线程执行**；音频线程**全程不碰**权威 | 否（`D28` + `MUST-GATE-001` 强制） |
| Q4 | 同案闭环判据要不要**像素级**证据 | **要一条差分像素断言**（同案两帧对比，不碰 golden）；是否**升格为硬要求**归负责人 | **是** |
| Q5 | 已落地的 `apply_revision` + `host_domain` + `ProjectAuthorityHandle` 够不够当底座 | **够**（写入口仍只有 `apply` / `host_save_project` 两个）；但 `mcp_mount.rs:697` 的"只能读"**必须改**，是否把句柄**按读/写分型**归负责人 | **是**（分型那半） |
| Q6 | `UndoPort` 的 `Local`/`Authority` 二元性保不保留 | **保留**（"永远挂一个 `Domain`"会违反红线 6 ⇒ 否决） | 否（红线 6 强制） |
| Q7 | 权威**没有活跃工程**时界面怎么办 | **保留最后一帧**（`ProjectionOutcome::NoActiveProject`，不假装清空） | **是**（UX 口径） |
| Q8 | 只读（`SharedRead`）会话存活时，GUI 的**本地**保存路径要不要可达 | **保持被拒**（fail-closed）；要不要给一条"降级为只读会话"的提示归负责人 | **是** |

## 决定（Proposed）

### Q1 — GUI 进程里谁是**唯一可变**的工程权威？

**问题**：账本那一行把选择写成两条路："让界面从 `Domain` 投影"，还是"把 `Domain` 的 `apply` 接到宿主 sink
并把 UI 重投影编组进 Slint 事件循环"。今天代码**两条都做了**（前者 = 第一片/第二片，后者 = 第 (b) 项）。
第三个可能性（任务点名要求评价）是**共享 `Arc<Mutex<YebanProjectV1>>` 式单一存储**。

**选项与代价**：

- **A. 界面从 `Domain` 投影（域是所有者）** —— 触点：`live_surface.rs`（`build_live_ui_from_authority` `:1054`、
  `sync_authority` `:509`）、`undo.rs`（`UndoBackend` `:246-251`）、`main.rs`（`:197` `from_authority`）。
  **schema：不要。锁语义：不要**（投影只读权威）。**golden：理论上要**（新增可视元素会改 5 张基准），
  但本项**没有**新增可视元素 —— 它改的是"这份界面从哪取工程"。**代价**：产品二进制必须在挂载时先建控制面
  （`main.rs:186`），失败路径要处理（`MountError::Locked` ⇒ `main.rs:387`）。**已落地**。
- **B. `Domain::apply` 驱动宿主 sink，UI 重投影编组进 Slint 事件循环** —— 触点：`transport/http.rs`
  （`ProjectRevisionSink` `:459`、`set_project_revision_sink` `:674`）、`reproject.rs`（`:198` `event_loop_sink`、
  `:148` `sync_now`、`:224` `install`）、`main.rs:292`。**schema：不要。锁语义：不要。golden：不要。**
  **代价**：新增一条跨线程跳（服务线程 → UI 线程），因此必须处理"平台没有 event loop proxy"
  （`reproject.rs` 顶部文档：`Err(NoEventLoopProvider)` ⇒ 打 stderr 并**不**把"已投影"记成真话）。
  **与 A 不是互斥项，是 A 的"界面怎么知道"那一半** —— 两者一起才是闭环。**已落地**。
- **C. 共享 `Arc<Mutex<YebanProjectV1>>` 式单一存储（两边锁同一份工程）** —— 触点：`Domain`（放弃 `Active::project`）、
  `bridge`、`host`、`undo`、`mcp_mount`、**以及所有写工具**；schema 不变。
  **代价（三条，前两条是结构性的）**：
  ① **它把 `Domain` 的"不可克隆"不变量抹掉** —— `mod.rs:147` 的注释说得很直白：`Domain` 内涵
  `.yeban.lock` 的 RAII 守卫与提交图谱，"克隆会产出两个独立的写者"。把工程掏出来放进 `Arc<Mutex<..>>`
  会让"写者"重新变成一个**可以有两个句柄**的东西，而当前设计用类型把这件事变成**不可表达**；
  ② **它把锁带进每一条读路径**：当前设计里唯一的那把锁是 `HttpServer` 内部的 `Mutex<Dispatcher>`，
  **只被服务线程持有**（`respond` 在释放它之后才通知，见 `reproject.rs` 顶部数据流），
  宿主侧读权威走的是 `host_domain` 这一个口；换成共享存储后，"投影读一份快照"与"写一次工程"
  会在同一把锁上竞争，而 `ARCH-DET-001`（写入确定性）要求写入字节不依赖调度顺序；
  ③ **它是唯一有可能把锁带进音频路径的形状**：`MUST-GATE-001` 的四元组里 `lock_blocking` / `lock_waits` 必须是 0
  （`gate-status.md:18` 实测 27 764 量子全 0）。今天音频线程**完全不碰**权威（`render_block` 只调 `rt_probe::quantum_enter()`），
  这是一种**结构性**而非"约定"的隔离；共享存储会让"有人在音频线程上读一下工程"从"做不到"变成"做得到"。
  ⇒ **否决**。
  **注意区分**：本仓**确实**用了 `Arc` + `Mutex`（`ProjectAuthorityHandle` 包着 `Arc<HttpServer>`，`mcp_mount.rs:467-471`），
  但共享的是**服务句柄**，不是**工程**；工程仍然只有 `Domain` 里的那一份，且那份 `Domain` 在一个
  `Mutex<Dispatcher>` 后面、**只被服务线程碰**。所以准确的裁决是：
  **"共享一个不可变修订号 + 一个句柄"可以；"共享一个可变工程"不可以。**

**代码强制**：`MUST-GATE-001`（四元组 + §5.2 的边界警告）、`ARCH-DET-001`、`domain/mod.rs:147` 的不可克隆不变量、
`D28`（唯一注入点）。

**建议**：**A + B（今天的样子），否决 C**。
理由：① A 让"界面画的是哪一份工程"与"控制面读写的是哪一份工程"**在构造上**不能分叉
（`build_live_ui_from_authority` **不接受**工程参数，`live_surface.rs:1054`）；② B 让刷新**事件驱动**
而不是"每条路径都记得调钩子"（后者的弱点账本已点名）；③ C 用三条结构性代价换来的只是"少一次快照克隆"，
而那一次克隆在 UI 线程上、不在音频路径上。
**需负责人确认**：是否**追认**这一选择（它已在五个提交里落地，但从来没有一份 ADR 记录过它，
而账本那一行明写"必须做一个**新的设计裁决**"）。

### Q2 — 写会话取 `ExclusiveWrite` 后，与 GUI 自身的保存路径是什么锁语义？

**问题**：GUI 打开真工程文件时用 `SessionSource::WritableFile`（`main.rs:373`）⇒ 会话取
`LockMode::ExclusiveWrite`（`mcp_mount.rs:262-266`）且 `read_only = false`（`:283-285`）。
而 GUI 自己的保存路径（`save.rs` 的 `save_project_file` / `save_archive_file`）**也**取 `ExclusiveWrite`
（第三片）。`flock` 在 **fd 粒度**仲裁 ⇒ **同进程另一个 fd 也冲突**。那么"人在 GUI 按保存"与
"AI 经控制面读写"怎么同时成立？

**选项与代价**：

- **A. 保持两条路径并存、各自取锁，GUI 保存被拒（fail-closed）** —— 触点：`save.rs`（`SaveError::Locked`）、
  `save_action.rs:161-191`。**schema：不要。锁语义：要（两个取锁点）。golden：不要。**
  **代价**：控制面存活时用户在界面上按保存会**失败**（真二进制 `--save-as` ⇒ 退出 **4**，
  判据 `in_process_mcp_lock.rs` / `cli_contract.rs`）。诚实但不友好。
- **B. 单一持锁者 + 宿主保存动作：会话持锁、GUI 保存**委派**给它** —— 触点：`domain/mod.rs` 的
  `host_save_project` + `HostSaveOutcome`、`transport/http.rs:642`、`mcp_mount.rs:545` `save_to`、
  `save_action.rs:161-176`（`Some(authority)` ⇒ **只**走 `save_to`，本地路径**不可达**）、`main.rs:247` `wire_save`。
  **schema：不要。锁语义：要（GUI 保存不再自己取锁，而是委派）。golden：不要。**
  **代价**：`ui/force_save` 这条**测试装配**的 UI 命令仍然不进产品路径（它需要 dev-dependency）；
  产品路径拿到的是**同一个落点策略**（`save_action::dispatch_save`），不是那条命令。
  且"字节由权威自己产出"⇒ 界面缓存陈旧也不会写错内容。**已落地**（§9 + §10）。
- **C. 会话保持只读、GUI 独占写者（`WritableFile` 之前的样子）** —— **代价**：
  AI 侧**永远不能落盘**（`yeban_save_project` ⇒ `IO_ERROR`），而 `[MCP-DUAL-001]` 的"注入编曲意图"闭环
  在"改动必须能被持久化"这一层是残的。**否决**（但**只读形态 `SessionSource::File` 保留**，
  用于"只想让 AI 看"的场景）。
- **D. 去掉 `.yeban.lock` / 换成进程内锁** —— **否决**：直接削弱 `MUST-GATE-008`
  （跨进程互斥 + `PROJECT_LOCKED` + `SIGKILL` 自愈）。

**必须存活的 `MUST-GATE-008` 不变量（逐条）**：
1. **同一工程文件上任意时刻至多一个 `ExclusiveWrite` 持有者**，跨进程**且**同进程（fd 粒度也冲突）；
2. **`ExclusiveWrite` ⊥ `SharedRead`** —— 排他者存活时连只读形态也挂不上（判据实测：`PROJECT_LOCKED`）；
3. **`SharedRead` 之间共存** —— 这条是**独立的一腿**，不许为了"单一写者"把它拆掉
   （实测裁决：把已有的 `SessionSource::File` 翻成写形态 ⇒ `in_process_mcp_lock.rs` **3 条红**）；
4. **崩溃自愈** —— 持有者被 `SIGKILL` 后不留残留锁，且**一个字节都不写**地失败而不是半写。

**代码强制**：`MUST-GATE-008`（`gate-status.md:25`，含"仍待：接进 `ci.yml` 受影响集合"这条**未完成**的登记）、
`store::write_project_atomic` 是唯一的原子落盘入口、`read_only` **只闸落盘**不闸内存变更
（`mcp_mount.rs:461`）。

**建议**：**B**（今天的样子），并**把 A 作为不变量保留在"只读形态"上**。
理由：只有 B 让"谁写了这份文档"的答案**只有一个**；A 让用户看到失败而不是静默地写一份旧字节。
**需负责人确认**：只读（`SharedRead`）会话存活时，GUI 的**本地**保存路径被拒（fail-closed）是否是可接受的产品行为
（今天就是这样）—— 见 Q8。

### Q3 — 重投影发生在哪？实时路径上**不许**做什么？

**问题**：投影是纯函数（`bridge.rs`），注入点只有一处（`host.rs:119` `apply_view`）。一次**从别的线程来的**
域侧变更如何到达它，才不违反 `MUST-GATE-001`？

**选项与代价**：

- **A. 通知（服务线程）→ marshal 到 UI 线程 → 读权威快照 → 投影 → 注入** —— 触点：`reproject.rs` 全部、
  `transport/http.rs:674`、`main.rs:292`。**schema：不要。锁语义：不要**（读权威用的是 `host_domain`，
  唯一的那把 `Mutex<Dispatcher>` 在 `respond` 释放**之后**才通知）。**golden：不要。**
  **代价**：`invoke_from_event_loop` 在没有 event loop proxy 的平台上返回 `Err` ⇒ 必须**出声**
  （`reproject.rs` 顶部：打一行 stderr，且**不**把"已投影"记成真话，下一次通知再试）。
  **已落地**。
- **B. 在实时路径上投影 / 定时器 / 后台线程轮询** —— **否决**：① 实时路径禁止分配（`MUST-GATE-001` 四元组的
  `allocations == 0` 实测 27 764 量子）；② 本仓 DoD 明写"没有判据的能力不算交付"，
  而一个定时器的**调度**没有任何无头判据能界定（§7.5 第 2 条逐字记着这条理由）；
  ③ 轮询会让"界面是不是被查询刷新的"变成不可判定 —— 而判据 17 的第 6 步正是靠
  "只读调用 ⇒ `Unchanged` ⇒ 树长度一位不变"来排除它。
- **C. 在音频线程上读权威** —— **否决**：见 Q1-C 第 ③ 条。

**代码强制**：`D28` 第 1/3 条（投影零 Slint 依赖、`.slint` 零算术）、`MUST-GATE-001` 与其 §5.2 边界警告、
`reproject.rs` 的类型事实（观察者是 `Fn(u64)`，**拿不到** `Domain`/`Dispatcher`/任何工具入口；
持的是 `WeakProjectAuthorityHandle` 以切断 `HttpServer → 观察者 → Arc<HttpServer>` 引用环）。

**建议**：**A**。理由：它让"音频线程碰不到权威"成为**结构事实**而不是约定；它让刷新**可被无头判据确定性驱动**
（通知发生在响应字节写出**之前** ⇒ "客户端收到响应"蕴含"通知已发生"，判据随后跑掉事件循环里那一条排队调用即可，
不需要 sleep / 轮询 / 时钟）。
**不归负责人**（`D28` 与 `MUST-GATE-001` 已强制）。

### Q4 — 同案闭环判据要不要**像素级**证据？

**问题**：判据 17 在**元素树/标签**层闭合（步 7 走 `ui/tree`/`ui/node`），`ui/screenshot` 只在判据 1 里被断言。
"界面**看起来**变了"在像素层今天**没有**同案证据。

**选项与代价**：

- **A. 维持元素树/标签层**（今天的样子） —— 触点：无。**schema/锁/golden：都不要。**
  **代价**：一条把新元素插进树却**画不出来**的回归（例如几何算错落进裁剪区）不会让判据 17 变红；
  它只能被 golden 或判据 1 那类覆盖，而 golden 在 macOS 上**什么都不判**。
- **B. 同案**两帧差分**（不碰 golden）** —— 触点：`live_ui_mcp.rs` 的判据 17 加 3-4 条断言
  （两帧不同 / 差异包围盒落在新泳道几何内 / 差异外逐字节相同 / 指纹可复现）。
  **schema：不要。锁语义：不要。golden：不要**（比较同一次运行的两帧，不与提交过的基准比）。
  **代价**：断言必须容忍抗锯齿与文本重排的邻近像素 ⇒ 包围盒要带一个**命名**的容差，
  且容差本身要有一条判据钉住（否则它会变成"什么都容得下"）。
- **C. 把该场景加进 5 张 golden** —— **代价**：`MUST-GATE-015` 的再生成程序（手动档 `gates-manual.yml` 的
  `goldens` 档 ⇒ 5 张 PNG 全变 + `MANIFEST.txt` 重写）；且本机 macOS **判不了**，
  必须等 CI；再生成一次的成本按 `ADR-0003` 的"批准后的再生成程序"付。**它证的东西比 B 多**
  （真的与基准比），但**贵**且**平台受限**。

**代码强制**：`MUST-GATE-015`（golden 必须由 Tier-1 软光栅化产出）；
`test_port_adapter.rs::assert_matches_golden` 的"平台目录不存在 ⇒ 未被判定"分支；`D28`（几何来自投影的整数派生）。

**建议**：**B**（同案差分像素），把 **C 留给负责人**：C 是"要不要为这一条再付一次 5 张基准的再生成"，
那是产品级的成本决定。
**需负责人确认**：同案闭环判据的**达标线**是元素树/标签（A，今天）、差分像素（B，建议）还是 golden 基准（C）。

### Q5 — 已落地的 `apply_revision` + `host_domain` + `ProjectAuthorityHandle` 切片够不够当底座？

**问题**：账本把这一片描述为"**只读**的 `ProjectAuthorityHandle`"。实测它**今天不是只读的**。

**选项与代价**：

- **A. 保持现状（句柄 = 权威的宿主门面；"只读"只适用于 `host_domain`）** —— 触点：`mcp_mount.rs:467-590`。
  **schema/锁/golden：都不要。** 两个写方法仍然**收敛**：`apply_host`（`:518`）→ `HttpServer::apply_host_action`
  → `domain::apply`（推进点仍只有 `apply` 一处，`mod.rs:1979`）；`save_to`（`:545`）→ `host_save_project`
  → 与 `yeban_save_project` **同一个** `read_only` 门与**同一个** `store::write_project_atomic`。
  **代价**：**类型不表达权限** —— 一个只读挂载给出的句柄在类型上与写会话的句柄**同型**，
  "这是不是第二个写者"只能靠运行时读 `is_writable()`（`:553`）判断；且 `:697` 的文档**仍然是错的**。
- **B. 按读/写分型（`ReadOnlyAuthority` / `WritableAuthority`）** —— 触点：`mcp_mount.rs` 的句柄族、
  `undo.rs`（`Authority(..)` 持有哪种）、`live_surface.rs`、`main.rs`；**没有 schema/锁/golden 代价。**
  **代价**：挂载握手要按 `SessionSource` 决定返回哪种类型（`WritableFile` ⇒ 写型，`File`/`InMemory` ⇒ 只读型），
  调用点要处理两个类型或用枚举；收益是"只读挂载拿到写口"变成**不可表达**，
  与 `Domain` 不可克隆是同一种设计手法。
- **C. 回退到"只读"**（把 `apply_host`/`save_to` 拿掉） —— **代价**：第二片与 §9 全部回退，
  GUI 写入口重新变成第二份权威。**否决**。

**代码强制**：`domain/mod.rs:147` 的"用类型消除两个写者"的先例；`MUST-GATE-009`（`read_only` 门与
"默认产物拒绝网络能力"是两条不同的保证，不能混）。

**建议**：**A 作为今天的地基（够用），B 作为后续切片（更严）**。
理由：A 的"够用"是有证据的 —— 两个写方法都收敛到既有的唯一入口，且 `read_only` 门与
`yeban_save_project` 是同一个（判据 `single_writer_session.rs:366` 的
`the_host_save_action_is_refused_on_a_read_only_session` 直证"宿主保存**不是**绕过 `read_only` 的后门"）。
但**至少**要先修 `mcp_mount.rs:697` 的"只能读"（它今天是一句与 `:518`/`:545` 矛盾的假话）。
**需负责人确认**：句柄要不要**按读/写分型**（那是一条会改宿主 API 的裁决）。

### Q6 — `UndoPort` 的 `Local`/`Authority` 二元性保不保留？

**问题**：`run_gui` 挂上控制面就用 `Authority`，没挂上就用 `Local`（`main.rs:197-205`）。
而 `#[path]` 共享的 `undo_session.rs` 在两侧是**两个不同的类型**，因此"把同一个实例交给两边"在类型系统里不成立
（`m4-008-authority-notes.md` §1.2 第 5 条）。

**选项与代价**：

- **A. 保留二元性**（今天的样子） —— 触点：`undo.rs:246-251`、`main.rs:197`。
  **schema/锁/golden：都不要。** **代价**：两条代码路径要各自被判据覆盖（判据 18/19 覆盖 `Authority`；
  `undo_wiring_ui.rs` 一族覆盖 `Local`）。合理性：**没挂载时进程里没有第二个写者**，
  所以 `Local` 不违反"唯一权威"。
- **B. 永远挂一个 `Domain`**（默认构建也挂，让端口恒为 `Authority`） —— **否决**：
  默认 release 会链接 `yeban-mcp`/网络监听代码，直接撞 `AGENTS.md` §2 红线 6 与 `MUST-GATE-009`
  （`:26` 的产物级判据：默认构建 + `--enable-mcp-http` 必须**非零退出并点名 feature**）。
- **C. 统一两个 `UndoSession` 类型**（消掉 `#[path]` 造成的双实例化） —— **代价**：
  真加一条 `yeban-app → yeban-mcp` 依赖边（默认树 `grep -c "^yeban-mcp v"` 会由 **0** 变 **1**），
  或新建第三个 crate 承接；**与红线 6 冲突**。**否决（在本项范围内）**。

**代码强制**：红线 6 + `MUST-GATE-009`；`undo.rs` 的 `#[path]` 手法（`m4-008-authority-notes.md` §1.2 第 5 条）。

**建议**：**A**。**不归负责人**（红线 6 已强制）。

### Q7 — 权威**没有活跃工程**（`yeban_close_project` 之后）时，界面怎么办？

**问题**：`ProjectionOutcome::NoActiveProject`（`reproject.rs:68`）今天**不动界面**（保留最后一帧）。

**选项与代价**：

- **A. 保留最后一帧，如实报告 `NoActiveProject`** —— 触点：`reproject.rs:148-180`、`live_surface.rs:201-219`。
  **schema/锁/golden：都不要。** **代价**：界面会显示一份**已经不在会话里**的工程 ⇒
  用户可能对着旧画面编辑（编辑会失败，因为权威没有活跃工程）。
- **B. 清空界面（回到"没有工程"的空态）** —— **代价**：需要一条"空态"的投影路径
  （今天 `build_live_ui_from_authority` 在 `authority.project() == None` 时**报错**
  `LiveWiringError`/`Display` 文案，`live_surface.rs:201-219`），而空态的**视觉**是一组新几何 ⇒
  **golden 可能要动**。

**代码强制**：`D28`（唯一注入点）；`ProjectionOutcome` 是**五值枚举**而不是 `bool`
（`reproject.rs:68-90` 的理由：五种结局必须能分开，不许把"没投影"和"投影失败"混成一个 `false`）。

**建议**：**A**（今天的样子）。**归负责人**：这是 UX 口径（"关掉工程后画面留不留"），不是工程能替的。

### Q8 — 只读（`SharedRead`）会话存活时，GUI 的**本地**保存路径要不要可达？

**问题**：第三片把 GUI 保存路径也纳入同一把锁。只读会话取 `SharedRead`，`save_project_file`
取 `ExclusiveWrite` ⇒ **同进程也冲突** ⇒ GUI 保存在只读会话存活时被拒（fail-closed）。

**选项与代价**：

- **A. 保持被拒**（今天的样子） —— **schema/锁/golden：都不要。**
  **代价**：一个"只让 AI 看"的会话会挡住用户保存；用户看到的是一条**保存失败**消息（不是静默写旧字节）。
- **B. 允许本地保存（例如把只读会话降级/临时释放）** —— **代价**：
  要么放宽 `MUST-GATE-008` 的排他性（**禁止**），要么引入"会话暂停"这种今天不存在的状态，
  且会让"谁写了这份文档"重新变成两个答案。**否决**。
- **C. 被拒时给一条**可操作**的提示**（"AI 会话占用中，先让它停机再保存"） —— **代价**：纯 UX，零不变量风险；
  今天 `apply_save_outcome`（`host.rs:509`）已把拒绝原因写进 `save_status` 且屏读器可达。

**代码强制**：`MUST-GATE-008` 的四条不变量（见 Q2）。

**建议**：**A + C**（保持 fail-closed，并把提示写清楚）。**归负责人**：产品口径（"AI 会话在时用户不该能存"
是不是可接受）。

## 待负责人裁决（本 ADR 不代答）

| 裁决 | 为什么不归实现者 | 实现者在此期间会做什么 |
| :--- | :--- | :--- |
| **Q1**（追认选项 (a)/(b)、否决 (c)） | 账本明写"必须做一个**新的设计裁决**"；这是产品与架构层面的所有权裁定，不是实现细节。实现者已按建议落地，但**没有授权**把它写成"已裁决" | 按 A+B 不动；本 ADR 保持 `Proposed` |
| **Q2 的后半**（只读会话存活时 GUI 本地保存被拒是否可接受） | 是 UX/产品口径（用户会不会被自己的 AI 会话挡住保存） | 保持 fail-closed，并把原因写进 `save_status` |
| **Q4**（同案闭环要不要像素证据；要不要为它再动 5 张 golden） | "证到什么程度算证到"与"再付一次 golden 再生成"是成本决定 | 按建议 B（同案两帧差分，零 golden 代价）实现；C 留下来 |
| **Q5**（宿主句柄要不要按读/写分型） | 会改宿主公开 API 与挂载握手；且"只读挂载能不能拿到写口"是安全口径 | 先只修 `mcp_mount.rs:697` 的过期文档；分型留待裁决 |
| **Q7**（无活跃工程时界面留帧还是清空） | UX 口径 | 保持留帧（`NoActiveProject`） |
| **Q8**（只读会话时本地保存被拒的产品口径） | 产品口径 | 保持被拒 + 提示 |
| **Q6 / Q3** 的**机制** | **不归负责人**：红线 6（默认不链接网络）与 `D28`/`MUST-GATE-001` 已强制；两个选项里只有一个合法 | 按建议 A 不动 |

**登记动作**：把上述待裁决项登记为新的 `HD-nn` 会牵动 `docs/ledger/human-decisions.md` 的表头计数
与**四份活文档**的 HD 区间（`docs/README.md`、`docs/ledger/phase-status.md`、`docs/ledger/feature-alignment.md`
+ 清单自身），由 `scripts/gates/check_decisions.py` 机械对账。
那是**集成者 / 负责人**的登记动作；本 ADR 只登记在**索引表**（`docs/adr/README.md`），
**不擅自改动那些计数**（与 `ADR-0003` / `ADR-0004` 的「登记状态」同一处置）。

## 代价（必写）

1. **本文是"事后追认"，因此它的可推翻性有一条特殊成本**：选项 A+B 已经落地并被 **6 个测试目标（文件）** 里的判据
   覆盖（`live_ui_mcp.rs`、`production_reprojection.rs`、`single_writer_session.rs`、`in_process_mcp_lock.rs`、
   `in_process_mcp.rs`、`production_save_ui.rs`）。负责人若改判选项 (c)，代价是**重做**：
   `Domain` 的不可克隆不变量、`ProjectAuthorityHandle` 的全部方法、`reproject.rs` 一整条链，
   以及上面这些判据的**全部**（不是"加一个开关"）。
2. **"闭环"这个词今天有明确定义，但没到像素层**：判据 17 在元素树/标签层闭合；`ui/screenshot` 的像素断言只在**另一条**判据里。
   若负责人要 Q4-C（golden），代价是**一次性 5 张 Linux PNG 再生成** + `MANIFEST.txt` 重写，
   且本机 macOS **判不了**（必须等 CI）。
3. **一处与仓库里过期文档的矛盾被登记**：`crates/yeban-app/src/mcp_mount.rs:697` 写"而且它**只能读**"，
   而同一类型的 `:518`（`apply_host`）与 `:545`（`save_to`）是写口。本 ADR **不改** `crates/**`
   （授权范围只有 `docs/adr/`），因此这句话在本文落地后**仍然在仓库里**。
4. **一处活文档的不一致被登记、不改**：`docs/ledger/feature-alignment.md:194` 的**首列状态词**仍写「部分」，
   而同一格正文已写"`docs/ledger/phase-status.md` §6 `ROAD-M4-008` 已升为「已完成」"。
   本 ADR 不改 `docs/ledger/**`（活文档不在授权范围内）；`check_feature_alignment.py` 对它是绿的
   （它只数三方是否齐全，不判完成度词）。
5. **一条仍未闭环的证据**：`phase-status.md:106` 末句自己写着"**本提交的 CI 判决待集成者推送后读回**"。
   本文**不声称**任何 CI 判决；`WritableFile` 那条提交的 CI 读回是集成者的动作。
6. **本文不实现任何东西**：只新增 `docs/adr/ADR-0005-*.md` 与 `docs/adr/README.md` 的一行索引。
   无 Rust、无 `.slint`、无 schema、无 golden、无台账、无 `docs/DEVELOPMENT_LEDGER.md` 改动。

## 备选方案及其代价（逐条否决的理由）

1. **共享 `Arc<Mutex<YebanProjectV1>>` 单一存储**（Q1-C）：抹掉 `Domain` 的不可克隆不变量、
   把锁带进每条读路径、且是唯一可能把锁带进音频路径的形状。**否决**（三条都是结构性的）。
2. **在实时路径上投影 / 定时器 / 后台轮询**（Q3-B）：违反 `MUST-GATE-001` 四元组，
   且定时器的调度**没有任何无头判据能界定**（违反本仓 DoD "没有判据的能力不算交付"）。**否决**。
3. **会话保持只读、GUI 独占写者**（Q2-C）：AI 侧永远不能落盘，`[MCP-DUAL-001]` 的闭环在持久化层是残的。**否决**
   （只读形态 `SessionSource::File` 单独保留）。
4. **去掉 `.yeban.lock` / 换进程内锁**（Q2-D）：直接削弱 `MUST-GATE-008`。**否决**。
5. **把已有的只读形态翻成写形态**（而不是新增 `WritableFile`）：实测 ⇒ `in_process_mcp_lock.rs`
   **3 条红**（`left: Some(ExclusiveWrite) / right: Some(SharedRead)`），拆掉"只读者共存"那条腿。**否决**。
6. **默认构建也挂 `Domain`，让 `UndoPort` 恒为 `Authority`**（Q6-B）：撞红线 6 与 `MUST-GATE-009` 的产物级判据。**否决**。
7. **统一两个 `UndoSession` 类型**（Q6-C）：要真加 `yeban-app → yeban-mcp` 依赖边（默认树命中 0 → 1）。**否决（本项范围内）**。
8. **回退宿主句柄到"只读"**（Q5-C）：第二片与 §9 全部回退，GUI 写入口重新变成第二份权威。**否决**。
9. **允许只读会话存活时本地保存**（Q8-B）：要么放宽排他性（禁止），要么引入"会话暂停"新状态。**否决**。
10. **加 `#[ignore]` 或放宽任何门禁**：任务与 `AGENTS.md` §2 都禁止。**不做**。

## 切片顺序（一片一交付；标明是否需要人类裁决）

> 顺序理由：**先修一句假话（零风险、零裁决）→ 再把"闭环"证到像素层（零 golden 代价、零裁决）→
> 再问负责人要口径**。前三片**都不等任何人类裁决**，因此这个目标不会单点阻塞在负责人身上。

| 片 | 交付（一个可提交的改动） | 依赖的裁决 | 验收判据 | 需要人类裁决？ |
| :--- | :--- | :--- | :--- | :--- |
| **S0** | 修 `crates/yeban-app/src/mcp_mount.rs:697` 的过期文档（"而且它**只能读**" ⇒ 如实写"读投影口 + 两个写口（`apply_host`/`save_to`），两者都收敛到唯一入口"） | **Q5 的"至少修文档"那半**（`ADR-0004` 的 Q5 建议里已含） | `grep -n "只能读" crates/yeban-app/src/mcp_mount.rs` ⇒ **0 命中**；`cargo clippy -p yeban-app --all-targets -- -D warnings` 零告警 | **不需要**（纯文档纠错；不改行为、不改 API） |
| **S1** | 给判据 17 加**同案差分像素**断言（两帧不同 / 差异包围盒落在新泳道几何内 / 差异外逐字节相同 / 指纹可复现），容差写成**命名常量**并另用一条判据钉住它 | **Q4 的建议 B** | 新断言在**本机**可变红（先红后还原：把 `apply_view` 的注入摘掉 ⇒ 两帧逐字节相同 ⇒ 红；把包围盒容差放大到全屏 ⇒ 红），且**不碰** 5 张 golden | **不需要**（零 golden 代价、零 schema、零锁语义；容差由判据钉住） |
| **S2** | 在 `WritableFile` 那条提交上**读回 CI 判决**并登记（`gh run list` 的 `headSha` 必须是那个提交；planner 必须真的选中 `yeban-app`；目标作业耗时非零且日志里有 `test result:` 行） | 无（纯证据动作） | 一次成功 run 的 run id + 目标作业的 `test result:` 行 | **不需要**（集成者 / 工作线动作；`AGENTS.md` §6.3 的三条防伪） |
| **S3** | `ProjectAuthorityHandle` **按读/写分型**（`ReadOnlyAuthority` / `WritableAuthority`），让"只读挂载拿到写口"在类型上不可表达 | **Q5** | 新判据：只读形态（`SessionSource::File` / `InMemory`）**拿不到**写型句柄（编译期即不可表达 + 一条运行时判据）；`single_writer_session.rs` / `in_process_mcp_lock.rs` 原样重跑全绿 | **需要**（改宿主公开 API 与挂载握手） |
| **S4** | 只读会话存活时 GUI 保存的**提示**文案（`apply_save_outcome` 的 `SaveOutcome::Failed` 里点名"哪个形态占着锁、下一步怎么办"） | **Q8** | 判据：`save_status` 文本含占锁形态与可操作建议；屏读器可达（`accessible-label` 由 `.slint` 取该属性） | **需要**（产品口径） |
| **S5** | 无活跃工程（`yeban_close_project`）时的界面口径（留帧 / 清空）+ 判据 | **Q7** | 若选"清空"：一条新判据 + **可能是 5 张 golden**（新几何）；若选"留帧"：一条判据钉住"留帧且 `NoActiveProject` 可观察" | **需要**（UX 口径；选"清空"会连带 golden 成本） |

**S0 / S1 / S2 三片合计零人类裁决** ⇒ 即使负责人不裁决 Q4/Q5/Q7/Q8，
"过期文档"与"闭环证到像素层"这两件今天就能推进，且 S2 能把"最后一条未闭环的证据"补上。

## 可推翻性

本裁决**可被推翻**。负责人若选上表任一备选，本文件**就地更新**（不另开一份）：
把该备选写进对应问题的「建议」，并同步改写「代价」与「切片顺序」。

- 若负责人裁**Q1 = C（共享 `Arc<Mutex<..>>`）**：`Domain` 的不可克隆不变量、`ProjectAuthorityHandle` 的全部方法、
  `reproject.rs` 一整条链与 S1 全部作废，且必须重开 `MUST-GATE-001` 的覆盖论证（因为"锁进音频路径"从
  "做不到"变成"做得到"）。
- 若负责人裁**Q4 = A（维持元素树层）**：S1 取消。
- 若负责人裁**Q5 = B（分型）**：S3 由"可选"升为"必做"，且 `mcp_mount.rs:697` 的文档在 S3 里一并改写（S0 变为其前置）。
- 若负责人裁**Q7 = B（清空界面）**：S5 要按 `ADR-0003` 的「批准后的再生成程序」重做 **5 张** Linux PNG。
- 任何改判都会让"已经落地并被判据覆盖"的那部分成本**再发生一次**（如果 S1/S3 已按本裁决落地过）。

## 登记状态（供后续集成者接手）

本 ADR 登记在 `docs/adr/README.md` 的索引表（状态 `Proposed（待人类裁决）`）。

`docs/ledger/human-decisions.md` 的逐行计数与 `HD-01..HD-NN` 区间由 `scripts/gates/check_decisions.py`
机械对账（该守卫**只读 `docs/adr/ADR-0001-*.md`** 的 `D<n>` 段落，见脚本 `:53`；本文件用的是 `Q<n>`，
因此**不参与**它的对账）。把本 ADR 的待裁决项登记为新的 `HD-nn` 会牵动**四份活文档**——
那是集成者 / 负责人的登记动作，本 ADR **不擅自改动那些计数**（与 `ADR-0003` / `ADR-0004` 同一处置）。

本 ADR **不改任何状态**（单位已注明，读数取自本机）：

- Phase 4 的 **11 条**阶段要求：已完成 **7** / 部分 **4** / PENDING **0**（`docs/ledger/phase-status.md:121`；
  `ROAD-M4-008` 本身在 `:106` 是**已完成**）；
- 门禁表的 **21 条**：已接线 **19** / 部分 **0** / PENDING **2**（`docs/ledger/gate-status.md`，
  `python3 scripts/gates/check_gate_status.py` 的读数）；
- `docs/ledger/human-decisions.md` 的 **52 项**：已裁决 **48** / 未决 **4**（`HD-48`、`HD-49`、`HD-50`、`HD-52`）；
- `docs/ledger/phase-status.md` 全表 **47 项**：已完成 **18** / 部分 **23** / PENDING **6**；
- `docs/ledger/feature-alignment.md` 的三方对齐矩阵不变（73 行功能 / 17 个 MCP 工具 / 14 条 ui 方法）。
