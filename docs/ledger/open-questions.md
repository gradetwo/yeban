# 待决清单（唯一入口）

> 本文件是**唯一**的待决入口：每项都给出**可一个字回答**的选项、**我已有的建议**、以及**你一选我会立刻做什么**。
> 详细的勘察与理由在 `docs/DEVELOPMENT_LEDGER.md` 第 361–373 轮；已裁决的历史在 `docs/ledger/human-decisions.md`。
> **已收回的提问**：`HD-49`（"要不要自托管 runner"）—— 早已由 `HD-38` = B 与 `ADR-0001 D50` 裁决为**不投入**，其门控的四项
> （`BASELINE-003`、`ROAD-M0-003`、`ROAD-M0-006`、`ROAD-M4-006`）因此是**依裁决 PENDING**，不是因疏忽。
> 纪律提醒（本会话第 362/364 轮学到）：**问之前先查本目录与 ADR** —— 已经记录在案的判决依然算数，重复追问是噪音。

## 1. `N2` —— 快捷键（Slint 不暴露物理键码）✅ **已关闭（2026-10-06，按建议 (1) 执行）**
- **(1) 建议**：GUI 接受**逻辑键**，无头端口保留**物理码**判据；在 `N2` 行写清端口判据仍覆盖哪些情形。
- (2) 换 GUI 框架 —— 为快捷键层丢弃 Slint 及建在其上的 UI 层，代价与收益不成比例。
- (3) 不经判据发布 —— 与"没有判据的能力不算交付"矛盾。
- **选 (1) 已执行（2026-10-06）**：
  - `crates/yeban-app/src/input.rs` 新增 `LogicalKey`（唯一解析点 `from_text`，词表取自 Slint 的
    `key_codes`：`\t`/`\n`/`\u{1b}`/`\u{7f}`/`\u{f708}`/`\u{f709}`/空格/可打印字符）与
    `InputContext::resolve_logical`；`InputContext::resolve`（物理码入口）改为委托给它 ⇒ **两条入口
    共用同一张策略表**，由判据 `physical_and_logical_entries_resolve_identically` 逐项机械钉住。
  - `crates/yeban-app/ui/app.slint` 加 `forward-focus: key-handler;` 与
    `key-handler := FocusScope { key-pressed(event) => … }`（**唯一**键盘事件源，把 `event.text` +
    四个修饰位交给宿主的 `key-action` 回调）。
  - `crates/yeban-app/src/host.rs` 的 `wire_keys` → `apply_action`：工具 / 双视图 / 侧栏 / 控制台 /
    走带落到**界面已有**的属性与回调；撤销族（`Cmd+Z` / `Cmd+Shift+Z` / `Cmd+Shift+H`）经
    `undo::dispatch_key`（唯一下发点）落到 `UndoPort`。生产 `main.rs` 传真 `UndoPort`；无头执行面
    `live_surface.rs` 传 `None`（那里没有撤销会话 ⇒ 如实 `reject`，不假装撤销了）。
  - **判据**：`crates/yeban-app/tests/live_ui_mcp.rs` 判据 16
    (`a_logical_key_shortcut_from_the_event_source_reaches_the_host_action`)：从无头端口注入逻辑键
    `"3"` ⇒ `active-tool` 真的变成矩阵第 3 行；注入 `Tab` ⇒ 切视图且被消费（`accept`）；未绑定的键与
    无撤销会话的 `Cmd+Z` 如实 `reject`。**负向实测**：临时摘掉 `wire_keys` 这一条立刻变红
    （`left: 1, right: 3`），因此它不是一条恒绿的判据。另有零 Slint 的判据
    `undo.rs::the_logical_key_chain_really_rolls_the_project_back`（逻辑键 `Cmd+Z` 真的回退工程字节）。
  - **端口物理码判据原样保留、一条未改**：`live_surface.rs::physical_key_of` + `key_resolution`
    （`ui/dispatch_key_press` 的 `dryRun.resolution`）、`test_port_adapter.rs` 的 IME 门控判据、
    `undo.rs::perform_key`（物理码整链）。它们仍覆盖逻辑绑定**结构上表达不了**的情形（与布局无关的
    **键位**意图、Shift 改了字符的键如美式 `Shift+1` = `"!"`）—— 逐条写在
    `docs/ledger/app-projection-notes.md` 的 `N2` 行里。

## 2. 响度传输（可选；契约侧已闭合且与传输无关）✅ **已关闭（2026-10-06，按建议 (a) 执行）**
- **(a) 建议**：用**已有的**环回控制面推送 5 个字段（无新进程/端口/鉴权；能力继承挂载的"默认关"）。
- (b) 仅轮询 —— 零构建，但表头在两次轮询之间无法更新。
- (c) 另立通道 —— 预先拒绝：第二套鉴权 + 重复挂载已提供的东西（`MUST-GATE-009` 要防的漂移）。
- **选 (a) 已执行（2026-10-06）**：在既有会话上发布 5 字段 + 一条"客户端收到更新"判据 + 保持默认关。
  - **没有新机制**：同一个方法（`yeban_query_engine_state`）、同一个 token、同一个环回端口、同一个
    分发器。新增的只是一个**可选游标** `since` + 会话上的**单调读数修订号**
    （`engine.readingsRevision` 每次注入 +1）+ 一条 64 条的**有界尾部**。
  - **为什么不是"服务端在同一条连接上主动写"**：`crates/yeban-mcp/src/transport/http.rs` 的形态是
    **一请求一响应**（无 keep-alive / 无 `Transfer-Encoding: chunked` / 无 HTTP/2，响应写完即
    `Connection: close`，见该文件"边界（明确没做）"）。服务端推送需要**第二套机制** ⇒ 按本项
    预备的退路取"**既有查询上的 `since`/游标方案**"，并把这个局限写进契约描述与实现文档。
  - **客户端形状**：`yeban_query_engine_state` 新增可选整数 `since`；带它时响应的
    `data.readingsStream` = `{since, revision, buffered, agedOut, updates[]}`，`updates[]` 里每条
    是 `{revision, sampleRate, bufferFrames, integratedLufs, momentaryLufs, shortTermLufs,
    loudnessRangeLu, truePeakDbfs}`（修订号严格大于 `since`）。**幂等**：客户端不推进游标就
    重复拿到同一条，不是"消费即消失"。**不静默丢**：游标比保留窗口更旧 ⇒ `agedOut: true` +
    空 `updates`（ADR-0001 D23 的同一条纪律）。**缺省不带 `since` 时响应形状与从前逐字段相同**
    （旧客户端的字节不变）。
  - **宿主怎么注入**（这条线补上的那一半）：`InProcessMcp::engine_readings_handle()` →
    `EngineReadingsHandle::set_engine_readings`，走**既有**的、文档写明"只对宿主开放"的
    `Domain::set_engine_readings`。它**不**扩大 JSON-RPC 面：没有任何工具的实参碰得到镜像
    （判据 `tools_never_write_the_engine_mirror` 仍逐位钉住），也没有第二个分发器/端口/令牌。
  - **判据**：`crates/yeban-app/tests/in_process_mcp.rs::a_client_receives_loudness_updates_over_the_in_process_control_plane`
    （真 `TcpStream` + 真令牌）：先证**缺席**（未测量 ⇒ 5 个字段全 `null`、`readingsRevision = 0`、
    缺省调用无增量段），再注入一次真读数后证**在场**（游标 `0` ⇒ 恰好 1 条更新，5 字段逐个回显）、
    **幂等**（同游标重复拿到同一条）、**推进后不再重放**（游标 `1` ⇒ 空、`agedOut=false`）、
    **第二条更新只给新的**（游标 `1` ⇒ 只有修订 2）、以及"清空回未测量"也到得了客户端。
    **负向实测**：临时摘掉 `Domain::set_engine_readings` 里那一行"把读数交给会话"，判据立刻变红
    （`assertion left: Null, right: 128`，`test result: FAILED. 2 passed; 1 failed`；随后已还原）。
  - **`MUST-GATE-009` 一条未动**：`in-process-mcp` 仍不在任何 `default` 里（`scripts/guards/policy_check.py`
    的 `FORBIDDEN_DEFAULT_FEATURES` 点名），默认依赖树里 `yeban-mcp` 命中仍为 **0**，仍只绑环回 +
    动态端口、仍必须 Bearer token、`ui:inject` 在生产模式下仍硬拒。

## 3. `D47` —— MIDI 导出的"唯一出口 = app CLI"是否覆盖实验性 `.als`？ ✅ **已关闭（2026-10-06，按建议 (a) 执行）**
- **(a) 建议（若要让导出器可用）**：加 `--export-als`，与 `--export-midi` 并列，并把损失报告写进日志。
- (b) `.als` 保持 crate 内、实验性 —— 记为"无用户出口，属库能力"，并在 ADR 补**一句澄清**。
- (c) 走 MCP —— 与 `D47` 明文选择 app CLI 矛盾，需**修改** `D47` 而非扩展。
- **选 (a) 已执行（2026-10-06）**：CLI 开关 `--export-als` 与 `--export-midi` 并列（`crates/yeban-app/src/cli.rs`
  + 薄封装 `crates/yeban-app/src/export_als.rs`），映射损失表**逐条**打到 stdout（`als-losses:` / `als-loss:` 行，
  超上限时 `als-losses-truncated: ... and N more` 礼貌截断）；`crates/yeban-app/tests/cli_contract.rs`
  加了"默认档退出 2 并点名 feature"与"带 feature 的正面导出"两条真二进制判据；非默认 feature
  `experimental-als-export` 的门控**未被削弱**（默认构建的依赖树里 `yeban-render → flate2` 这条边**不存在**）。

## 4. `MUST-GATE-014` 素材 —— ✅ **已裁决（负责人，2026-10-06）**
- **裁决**：素材**复用 `groove` 的 R2 镜像**，**不用重新入库**；原则与 `groove` 相同 —— **先从源下载，失败再 fallback 到 R2 镜像**。
- 负责人明示：**这块不用查验了**。⇒ 不再对该块做进一步核验或准备。
- ⇒ 因此本项的形态是"**按需取用 + 不入库**"：门禁仍保持"仓库内 0 字节被校验"，`MUST-GATE-014` 的机制/白名单/登记**已在位**，素材分发**不属于本仓库职责**。
- 现状：机制、白名单、登记、校验入口**全部就绪**；`D54` 已授权复用 `groove` 的选择；过滤后登记 **30 款（27 CC0 + 3 CC-BY）**，
  源为 33 SFZ / 21 505 文件 / 9.371 GiB；**仓库今天校验 0 字节**。
- **(A) 建议**：**分发这 30 款** ⇒ 门禁从"仅机制"推进到**真字节**。
- (B) **不入库** ⇒ 保持 0 字节，并把该理由记录为**已裁决**（`HD-31` 已预见这是正当答案）。

## 5. 参考 `.als`
- 现状：导出器被**刻意**称为 "Ableton-style" 而非"可在 Live 11/12 打开"，因为**仓库里没有参考文件可测**；委派运行拒绝超出可验证范围声称。
- **(A) 建议**：**提供一份参考集**（或你在自己机器上手工打开一次并回报结果）。
- (B) 明确不做 ⇒ 该措辞保持现状，并在行内写清"由数据缺口决定"。
- **选 (A) 我做**：加入 fixture + 写**结构性**判据（**不是**逐字节相等 —— Live 会重写自己的容器）+ 按证据升级措辞。

## 6. `M4-008` —— UI↔领域**唯一可变权威**
- 结构：三份拷贝（`Domain` 独占且刻意不实现 `Clone`；`undo::UndoPort` 的 `RefCell<UndoSession>`；`live_surface::LiveSurface`）；
  进程内控制面的会话是**只读克隆** ⇒ 挂载今天绝不是第二个写者。
- **(a) 建议**：UI 从 `Domain` **投影** ⇒ 构造上只有一个写者，锁的故事保持简单；改动最大。
- (b) 保留拷贝 + `Domain::apply` 挂宿主 sink ⇒ 改动小，但**仍有三处权威**，正确性依赖每条路径都记得调钩子。
- (c) 让挂载会话可写并取 `ExclusiveWrite` ⇒ 控制面成为真写者，GUI 保存路径须让位；委派运行刻意拒绝（会在没有单一权威接线时造出影子写者）。
- **选 (a) 我做**：`Domain` 居中 + 两处改投影 + 加"经 MCP 变更 ⇒ UI 投影跟随"判据 + 重跑锁判据证明 `MUST-GATE-008` 仍成立。

### 执行状态：🟡 **第二片已落地（2026-10-06），仍是「部分」** —— GUI 的**写**入口已落到该权威；缺的是"保存路径也纳入同一把锁"与"生产窗口的运行期重投影"

**已做到（投影口 + 会红的判据）**

- **`Domain` 有了"我改过工程"的可观察读数**：`crates/yeban-mcp/src/domain/mod.rs` 新增 `Domain::apply_revision`（**施加修订号**）与 `Plan::mutates_project`（推进口径：只读变体不推进）。推进点只有一个 —— 唯一可变入口 `Domain::apply`，且只在施加**成功**时 +1。
- **宿主只读口**：`crates/yeban-mcp/src/transport/http.rs` 的 `HttpServer::host_domain` 只借出 `&Domain`（签名里没有 `&mut`）⇒ 这个口子**结构上不可能**成为第二个写者；`crates/yeban-app/src/mcp_mount.rs` 的 `InProcessMcp::project_authority()` 把它包成只读的 `ProjectAuthorityHandle`（`project()` / `apply_revision()`）。
- **界面成为投影（这一条是选项 (a) 的实质）**：`crates/yeban-app/src/live_surface.rs` 新增 `build_live_ui_from_authority` —— 它**不接受任何工程参数**，工程的唯一来源是控制面正在服务的那一个 `Domain`；`LiveUi::sync_authority` 只在修订号前进时重投影（因此不依赖"每条路径都记得调钩子"）。`LiveAdminSurface.project` 的角色从"另一份权威"改写为"**投影缓存**"。
- **判据（有牙，且能失败）**：`crates/yeban-app/tests/live_ui_mcp.rs::an_mcp_mutation_reaches_the_live_ui_projection_through_the_single_authority` —— 真环回 socket + 真令牌发 `yeban_edit_automation`（在从没有泳道的 `TrackPan` 上写一个点）⇒ 修订号 0→1 ⇒ **同一个活窗口**的运行时控件树 84→**85** 个节点、新语义元素 `track-0-automation-pan-lane` 的 `accessible-label` **逐字等于权威工程的投影** ⇒ `ui/tree` / `ui/node` 端到端读到它；随后一次**只读** `yeban_query_project` ⇒ `AuthoritySync::Unchanged`（界面不是被查询刷新的）。配套：`crates/yeban-mcp/src/domain/mod.rs::only_plans_that_can_change_the_project_advance_the_apply_revision` 钉住推进口径（只读：修订号与工程字节逐字不变；写：恰好 +1 且字节真的变了）。
- **负向实测两条（都是先红后还原）**：① 临时摘掉 `apply` 的修订号推进 ⇒ 红在「施加修订号 0→1」（`left: 0, right: 1`，`test result: FAILED. 0 passed; 1 failed`）；② 临时摘掉 `sync_authority` 里的重投影一步 ⇒ 红在控件树断言（`track-0-automation-pan-lane` 不在树里）。因此这条判据不是恒绿。
- **`MUST-GATE-008` 一位没动**：控制面会话仍 `read_only = true`、仍取 `LockMode::SharedRead`，`crates/yeban-app/tests/in_process_mcp_lock.rs` 原样重跑全绿；默认依赖树里 `yeban-mcp` 命中仍为 **0**（加 feature 才 1）。

**第二片（2026-10-06）：GUI 的写入口也落到该权威上**

- **宿主写入口**：`crates/yeban-mcp/src/domain/mod.rs` 新增 `HostAction`（`Undo`/`Redo`/`Commit`）、`HostOutcome` 与 `Plan::Host`；`pub fn apply_host_action(domain, action)` 只做三件事 —— 只读预读 op 种类、把动作包成 `Plan::Host` 交给**唯一可变入口** `apply`、读回显示态。推进 `apply_revision` 与 `sync_session` 仍**只在** `apply` 那一处。`crates/yeban-mcp/src/transport/http.rs` 的 `HttpServer::apply_host_action` 与既有的 `host_domain` 共用**同一个** `Mutex<Dispatcher>`、**同一个** `Domain`：没有第二个端口 / 令牌 / 通道，对外 JSON-RPC 面一位没变。
- **GUI 侧不再持自己的会话**：`crates/yeban-app/src/undo.rs` 的 `UndoPort` 内部改为 `UndoBackend::{Local(Box<UndoSession>), Authority(ProjectAuthorityHandle)}`；`main.rs` 的 `run_gui` **先挂控制面**，挂上了就用 `UndoPort::from_authority`（端口只握句柄，`display`/`project`/`graph`/`fingerprint`/`commit_ops`/`undo`/`redo` 全部委派给权威），没挂上才用本地会话（那时进程里没有第二个写者）。`ProjectAuthorityHandle` 相应扩出 `undo_display` / `graph` / `apply_host`。实测：`grep -c "RefCell<UndoSession>" crates/yeban-app/src/undo.rs` 由 **1 → 0**。
- **判据（有牙，且能失败）**：`crates/yeban-app/tests/live_ui_mcp.rs::a_gui_action_and_a_session_action_share_one_projection_through_the_authority` —— 以权威装配真实界面 + 由**同一个**句柄构造端口；会话侧真 socket 发 `yeban_edit_automation` ⇒ 修订号 `r→r+1`、泳道进树；**GUI 侧**走 `host::wire_undo` 接的真实回调 `undo-step` ⇒ 修订号 `r+1→r+2`（同一个权威）、泳道离树；会话侧再 `yeban_redo` ⇒ `r+2→r+3`、泳道回树。`…::a_gui_pencil_edit_lands_in_the_same_authority_as_the_session` —— GUI 的 `clicked` 回调（卷帘铅笔）让**权威工程**音符 4→5、修订号 +1、新元素 `note-{ulid}-rect` 进树；随后会话侧一次 `yeban_undo` 把音符与元素一起撤掉。
- **负向实测两条（先红后还原）**：① `UndoPort::from_authority` 改回另开一份 `UndoSession` ⇒ 红在判据 18 的 `left: 1 / right: 2`（`test result: FAILED. 0 passed; 1 failed; … 19 filtered out`）；② `commit_ops` 的权威分支改成直接 `Err` ⇒ 红在判据 19 的 `left: 0 / right: 1`（音符 4→4）。两条已还原（`grep -rn "NEGATIVE MEASUREMENT" crates/` = 0）。
- **`read_only` / 锁模式仍然一位没改，且这是刻意的**：实测查清 `read_only` 只闸**落盘**（`plan_save`），不闸内存变更 —— 因此第 1 步不需要放开它。而翻成 `false` 会**新开一条落盘路径**（控制面 `yeban_save_project`），与 GUI 自己的保存路径（`ui/force_save` / `--save-as`）**同时写同一个工程文件**，那就是影子写者（选项 (c) 被拒的理由）；而且 GUI 保存路径**不取** `.yeban.lock`，把锁改成 `ExclusiveWrite` 也挡不住它。⇒ 本片**停在这里**并如实登记。

**没做到（因此本项仍是「部分」，不是「已关闭」）**

1. **落盘写者边界未合并**：GUI 自己的保存路径（`ui/force_save` → `src/save.rs`、`--save-as`）**不取** `.yeban.lock`，因此不能把挂载会话改成 `read_only = false` 并取 `ExclusiveWrite` —— 两条会同时写同一个工程文件的路就是影子写者（选项 (c) 已明文拒绝）。本片据此**拒绝**了"顺手翻过去"。注意这一条是**磁盘写者**的边界，与内存权威无关：内存里的唯一可变权威已经只有一个（第二片做的）。
2. **生产 GUI 仍没有运行期重投影**：`run_gui` 建窗口走 `host::build_main_window`（初值来自 `loaded.archive.project`，一份**只读**输入），而"依修订号重投影"的唯一实现（`live_surface::LiveUi::sync_authority`）住在**测试目标专用**的 `src/live_surface.rs`（dev-dependency ⇒ 不进产品二进制）。后果：GUI **自己**的动作会从权威重投影（`host::refresh_undo_window` / `wire_roll_edit` 都用 `port.try_project()`），但**会话侧的改动不会自动刷新生产窗口**。补它需要在产品路径上接一条周期性刷新，而 `run_gui` 今天没有任何定时器 —— 加一个**无法无头判定**的定时器违反本仓 DoD（"没有判据的能力不算交付"），因此本片不做，如实登记为下一片。
3. **`UndoPort` 与 `Domain` 的会话仍是两个类型**（结构性）：`crates/yeban-app/src/undo.rs` 用 `#[path]` 共享 `undo_session.rs`，而 `#[path]` 引入的是**另一个 crate 里的另一个类型** ⇒ "共享同一个实例"在类型系统里不成立。第二片因此走的是**委派**（`HostAction` → `Domain`），而不是"把同一个 `UndoSession` 交给两边"。

**原始读数与逐条证据**（授权核实、命中数、负向实测的原文、锁判据的三行 `test result:`）：见
[`docs/ledger/m4-008-authority-notes.md`](m4-008-authority-notes.md)。

---

## 附：Phase 4 的四项「部分」各自缺什么，以及对应哪条裁决

> 目的：让"还剩什么"与"要你回哪个字母"一一对应，不留无法追溯的 🟡。

| 项 | 现状证据 | 缺什么 | 对应的裁决 |
| :--- | :--- | :--- | :--- |
| `ROAD-M4-006` | RF64/BW64 写入器 + BEXT 元数据已落地（`crates/yeban-render/src/rf64.rs`）| **32 轨参考工程 ≥100× 实时**的**实测** | **不需要新裁决** —— 依 `HD-38`/`D50`（不投入自托管 runner）该实测**长期 PENDING**；若你改变 `D50`，我按第 361 轮简报的 (a) 执行 |
| `ROAD-M4-007` | `.als` 导出器首片：模块 + 19 条损失表 + 4 条判据；**默认依赖树 0 命中 `flate2`**；**CI 全量腿 success**；**出口已接**：CLI `--export-als` 呈现损失表（问题 3 已关闭） | **参考 `.als`**（用于把"风格级"升级为"可在 Live 打开"）| **问题 5**（提供 / 不做）；出口形态已由 **问题 3**（`D47`）裁决并落地 |
| `ROAD-M4-008` | UI 侧闭环真跑；依赖边与运行态挂载已落地（`4971549`）；跨形态锁已成立（`db1a667`）；**选项 (a) 第一片（2026-10-06）**：`Domain` 施加修订号 + 宿主只读投影口 `HttpServer::host_domain` + `build_live_ui_from_authority` / `LiveUi::sync_authority`；**选项 (a) 第二片（2026-10-06）**：`Plan::Host` + `apply_host_action` + `HttpServer::apply_host_action` 把 GUI 的**写**入口（撤销族 / 卷帘铅笔）落到那一个 `Domain` 上（`UndoPort` 的 `RefCell<UndoSession>` 命中 1→0），判据 18/19 直证"GUI 写与会话写落在**同一个**投影上"，两条负向实测都会红 | **落盘写者边界未合并**（GUI 保存路径不取 `.yeban.lock` ⇒ `read_only` 与 `SharedRead` 不能翻，否则与控制面 `yeban_save_project` 同时写一个文件 = 影子写者）；**生产窗口没有运行期重投影**（`sync_authority` 住在 dev-dependency 的 `live_surface.rs`，`run_gui` 也没有可无头判定的定时器） | **无需新裁决** —— 方向已是**问题 6 (a)**（本项上方「执行状态」），剩下的是**执行**：先把 GUI 保存路径纳入同一把锁，再接产品路径的运行期重投影 |
| `ROAD-M4-010` | V1/V2 Web 包袱已彻底删除；汇合项（`P4_Gate` 四个入边）| **它必然被其它项拖住**，不可单独提前判 | **无需单独裁决** —— 随 `M4-006/007/008` 与两项 PENDING 的处置而自然收口 |

**读法**：上表四行里，`M4-006` 与 `M4-010` **不需要你新增裁决**（前者依既有 `D50` 裁决为长期 PENDING，后者是汇合项）；
**问题 1**（`N2` 快捷键）、**问题 2**（响度传输）与**问题 3**（`D47`/`.als` 出口）**已关闭**
（分别按建议 (1)/(a)/(a) 落地：GUI 绑逻辑键 + 无头端口判据；既有控制面的 `since` 游标 +
`InProcessMcp::engine_readings_handle` 宿主注入口 + 真 socket 判据；CLI `--export-als` + 损失表可见）；
真正需要你的只剩 **问题 5**（参考 `.als`）以及与 Phase 4 并列的 **问题 4**（`MUST-GATE-014` 素材）
—— **问题 6 的裁决已经是 (a)**（本轮交出第一片，见上方「执行状态」），它现在缺的是**执行**而不是**你选一个字母**。
