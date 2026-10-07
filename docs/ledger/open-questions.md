# 待决清单（唯一入口）

> 本文件是**唯一**的待决入口：每项都给出**可一个字回答**的选项、**我已有的建议**、以及**你一选我会立刻做什么**。
> 详细的勘察与理由在 `docs/DEVELOPMENT_LEDGER.md` 第 361–373 轮；已裁决的历史在 `docs/ledger/human-decisions.md`。
> **已收回的提问**：`HD-49`（"要不要自托管 runner"）—— 早已由 `HD-38` = B 与 `ADR-0001 D50` 裁决为**不投入**，其门控的四项
> （`BASELINE-003`、`ROAD-M0-003`、`ROAD-M0-006`、`ROAD-M4-006`）因此是**依裁决 PENDING**，不是因疏忽。
> 其中 `BASELINE-003` 的**处置**仍是 **PENDING**，但理由**已变**：负责人 2026-10-07 的 `HD-59` 把正式口径定为
> **绘制回调耗时（不含呈现）p99 ≤ 2 ms**（墙钟帧周期降级为**环境读数**），参考机实测 p99 **2.440–3.049 ms**
> ⇒ 该门禁是**已量到正式读数、读数未达标**。保持 PENDING、**不放松门限、也不从表里移除**，发布说明必须照写，
> **不得**写成"等硬件"、也**不得**写成"有意挂起"。
> **2026-10-07 注（`HD-49` 的条目本身）**：上句的「已收回」指**自托管 runner 预算**这一问（`HD-38`/`D50`）；本条
> 曾记 `HD-49` **仍未决**（它只问 `BASELINE-003` 单条的判决口径），其后列出过"仍记未决"的三条理由。**该条现已
> 关闭**：负责人当日的 `HD-59` 口径裁决回答了它问的"判决口径" ⇒ `human-decisions.md` 的 `HD-49` 行已落 `✅`
> （其"未决"历史保留在该行）。**项目级**的那一半 —— `BASELINE` 系列（001/002/004/005）在没有参考机 runner 时
> 按什么口径记 —— **仍未裁**，登记为 **`HD-58`**。逐条理由见 `human-decisions.md` 那两行 2026-10-07 的注。
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

## 5b. Logic Pro 导出方向 —— ✅ **已裁决（负责人，选项 A）**

- 背景（账本第 389–391 轮）：负责人把"参考 `.als`"（问题 5）**改指到 Logic Pro**，并点名 groove 的调研与实现。
  当时给出三个字母：**(A)** 新增一个 Logic Pro 导出器、`.als` 原样保留；**(B)** 用 Logic 替换 `.als` 的定位；
  **(C)** 只把 Logic 当优先项、`.als` 仍是实验性首片。三者的工作量相差很大，故没有替负责人选。
- **裁决：选项 A。** ⇒ 新能力 = Logic Pro (`.logicx`) **导出器**，gate 在**非默认** feature
  `experimental-logic-export` 后面；`crates/yeban-render/src/als.rs` 与 `experimental-als-export`
  **一位没动**（`.als` 原样保留）。
- **已落地（本切片，`ROAD-M4-011`）**：`crates/yeban-render/src/logic.rs`（纯内存、确定性的
  `project_data` / `build_bundle`：自研分块 `ProjectData` + bplist00 `MetaData.plist` 三件 + 映射损失表）、
  app 出口 `crates/yeban-app/src/export_logic.rs`（建 bundle 目录 + 同一份原子落盘）+ CLI `--export-logic <dir>`
  （`crates/yeban-app/src/cli.rs`，`logic-losses:` / `logic-loss:` 逐条打到 stdout），
  判据在 render 侧 6 条（含**可选**的本机演示工程头部核对，演示工程不存在即 skip）+ app 侧单测 2 条 +
  真二进制 B7d/B15。
- **打开结论（实测，勿外推）**：本机 **Logic Pro 12.2** 能打开**供体拼接**产物（负责人实测，2026-10-06）；
  自研（无供体）写入器的产物被拒绝过两次（账本第 403、405 轮），结论不覆盖其它 Logic 版本或其它机器。
  被验证的还有：结构与本机实测字节布局、以及 groove 的写入器/读取器一致；
  `MetaData.plist` 是标准 bplist00（独立用 `plistlib` / `plutil` 对账）。

## 6. `M4-008` —— UI↔领域**唯一可变权威**
- 结构：三份拷贝（`Domain` 独占且刻意不实现 `Clone`；`undo::UndoPort` 的 `RefCell<UndoSession>`；`live_surface::LiveSurface`）；
  进程内控制面的会话是**只读克隆** ⇒ 挂载今天绝不是第二个写者。
- **(a) 建议**：UI 从 `Domain` **投影** ⇒ 构造上只有一个写者，锁的故事保持简单；改动最大。
- (b) 保留拷贝 + `Domain::apply` 挂宿主 sink ⇒ 改动小，但**仍有三处权威**，正确性依赖每条路径都记得调钩子。
- (c) 让挂载会话可写并取 `ExclusiveWrite` ⇒ 控制面成为真写者，GUI 保存路径须让位；委派运行刻意拒绝（会在没有单一权威接线时造出影子写者）。
- **选 (a) 我做**：`Domain` 居中 + 两处改投影 + 加"经 MCP 变更 ⇒ UI 投影跟随"判据 + 重跑锁判据证明 `MUST-GATE-008` 仍成立。

### 执行状态：🟡 **第 (b) 项已落地（2026-10-06），仍是「部分」** —— GUI 的**保存路径**已纳入同一把 `.yeban.lock`，**生产窗口也有了运行期重投影钩子**；缺的只剩"单一写者会话"

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

**第三片（2026-10-06）：GUI 的保存路径也纳入同一把 `.yeban.lock`**

- **锁的同一份源码**：新增 `crates/yeban-app/src/project_lock.rs`，用 `#[path = "../../yeban-mcp/src/domain/lock.rs"]` 把领域 MCP 的锁实现装进 app（与 `undo.rs` 装 `undo_session.rs` 同款）⇒ 不写第二份锁协议，**也不新增 `yeban-mcp` 依赖边**（默认树里 `yeban-mcp` 命中仍为 **0**，加 feature 才 1）。代价只有一条**直接边**：那份源码用 `serde_json`（它**早就在**默认依赖图里：`yeban-model` → `serde_json`；默认树"唯一 name+version 集合"改动前后各 **296** 个、diff 为空，`Cargo.lock` 未变）。
- **两条工程保存入口各取一次 `ExclusiveWrite`**：`crates/yeban-app/src/save.rs` 的 `save_project_file`（`ui/force_save` 的落点）与 `save_archive_file`（`--save-as` 的落点）；拿不到锁 ⇒ 新增的 `SaveError::Locked`，**一个字节都不写**。`write_file_atomically` **仍然不取锁**：它写的是任意产物（元素清单 / `.mid` / `.als`），对它们取工程锁是**假保护**（`song.mid.lock` 与任何工程都不互斥）。改动前的写入口计数（除 `save.rs`）：`save_project_file(` **1**、`save_archive_file(` **2**（1 生产 + 1 单测）、`write_file_atomically(` **3**（全是导出）—— 取锁位置因此只有一处。
- **判据（有牙，且能失败）**：`save.rs::a_held_project_lock_refuses_the_save_without_touching_the_file`（lib 单测：持锁 ⇒ 拒绝 + 旧文件逐字节未变 + 释放后同一份保存必须成功 + 不留残留锁）；`tests/cli_contract.rs::save_as_is_refused_while_the_project_lock_is_held`（**默认构建**的真二进制：持锁 ⇒ 退出 4、stderr 含"拒绝写入"、目标字节不变；释放 ⇒ 退出 0 且内容真的换了）；`tests/in_process_mcp_lock.rs` 新增两条 —— `…the_gui_save_paths_are_refused_while_another_process_holds_the_project_exclusively`（真子进程排他持有 ⇒ `save_project_file` 与 `--save-as` **两条**都被拒；`SIGKILL` 持有者后保存成功且不留残留锁）与 `…a_mounted_read_only_session_refuses_the_gui_save_on_the_same_file`（本进程的只读会话持 `SharedRead` 时 GUI 保存同样被拒 = fail-closed，登记事实）。
- **负向实测两条（先红后还原，两个取锁点各测一次）**：① 摘掉 `save_project_file` 的取锁 ⇒ 单测 `持锁时保存必须被拒: SaveReport { … }`（`FAILED. 0 passed; 1 failed`）+ `in_process_mcp_lock` `FAILED. 3 passed; 2 failed`，而 B14 **保持绿**；② 摘掉 `save_archive_file` 的取锁 ⇒ B14 `left: 0 / right: 4` 且 `FAILED. 0 passed; 1 failed`，`in_process_mcp_lock` `left: Some(0) / right: Some(4)`，而判据 ④ **保持绿**。两条已还原（`grep -rn "NEGATIVE MEASUREMENT" crates/` = 0）。
- **`read_only` / 锁模式仍然一位没改 —— 这一次是实测后的裁决**：临时把 `mcp_mount.rs` 翻成 `ExclusiveWrite` + `open_in_memory(.., false)` 跑一遍，`in_process_mcp.rs::the_round_trip_stops_leaving_nothing_listening` 立刻红在 `left: String("success") / right: "error"`（**控制面真的拿到了落盘路径**），`in_process_mcp_lock.rs` 三条红（`Some(ExclusiveWrite)` vs `Some(SharedRead)`，含"另一个只读形态不能共存"），合计 `FAILED. 2 passed; 3 failed` ⇒ 翻转换来的是"控制面能落盘 + GUI 保存仍被自己的锁挡住 + `MUST-GATE-008` 的共享读语义没了"。⇒ 本片**明确不翻**，把"单一写者会话（宿主保存动作）"登记为下一步；完整读数见 `m4-008-authority-notes.md` §7.4。
- **运行期重投影（选项 (a) 剩下第 2 件事）：已由第 (b) 项关闭（2026-10-06）** —— 见 `m4-008-authority-notes.md` §8。`crates/yeban-mcp/src/transport/http.rs` 的 `HttpServer` 新增 `set_project_revision_sink`（观察者类型 `Fn(u64)`，拿不到 `Domain`），`respond` 在释放分发器锁之后、**写出响应之前**、且**只在本次请求真的推进了 `apply_revision`** 时通知一次；`crates/yeban-app/src/reproject.rs`（`cfg(in-process-mcp)`）的 `AuthorityMirror` 读权威（只读口）→ 投影 → `host::apply_view` 注入活窗口，生产驱动是 `slint::invoke_from_event_loop`（**不是**定时器、**不是**后台线程）；`run_gui` 在进事件循环之前装它，装不上**出声**。判据 `crates/yeban-app/tests/production_reprojection.rs::a_session_side_mutation_reaches_the_production_window_through_the_runtime_hook` 在**生产构造入口**建出来的窗口上跑：真环回 socket 改工程 ⇒ 钩子跑之前窗口**一位不动** ⇒ 跑掉事件循环里那一条排队调用 ⇒ 窗口泳道 1→2 条、标签逐字等于权威工程的投影；只读调用不刷新。四条负向实测（不装观察者 / 不注入 / 持强句柄成环 / 不 marshal 直接在服务线程上做）都会红，已还原。**登记一个元问题**：`docs/DEVELOPMENT_LEDGER.md:8962` 写的"the existing `invoke_from_event_loop` path already does for host calls"与实测不符 —— 改动前 `crates/` 全树 `invoke_from_event_loop` 调用点 **0** 个（该词只出现在 `docs/**` 与上游 slint 源码里）；那条路是本次第一次接起来的（该文件由集成者独占，本行不改它）。

**没做到（因此本项仍是「部分」，不是「已关闭」）**

1. **"单一写者会话"未建**（第三片 §7.4）：GUI 的保存落点与控制面会话在**同一个**工程文件上互斥（`flock` 在 fd 粒度仲裁，同进程另一个 fd 也冲突）⇒ 控制面存活时 GUI 的保存被**拒绝**（fail-closed）。今天不可观测（生产 `run_gui` 从不给 `ui/force_save` 配 `save_path`），但要让"人在 GUI 存"与"AI 经控制面读写"同时成立，必须给宿主加一个**保存动作**让会话成为唯一写者。注意这一条是**磁盘写者**的边界，与内存权威无关：内存里的唯一可变权威已经只有一个（第二片做的）。
2. ~~生产 GUI 仍没有运行期重投影~~ ⇒ **已关闭（第 (b) 项，2026-10-06）**：现在 `crates/yeban-app/src/reproject.rs` 是产品路径上的钩子，由控制面 `respond` 的通知驱动、经 `slint::invoke_from_event_loop` 在 UI 线程上执行；判据跑在**生产构造入口** `host::build_main_window` 建出来的窗口上。详见 `m4-008-authority-notes.md` §8。
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
| `ROAD-M4-008` | UI 侧闭环真跑；依赖边与运行态挂载已落地（`4971549`）；跨形态锁已成立（`db1a667`）；**选项 (a) 第一片（2026-10-06）**：`Domain` 施加修订号 + 宿主只读投影口 `HttpServer::host_domain` + `build_live_ui_from_authority` / `LiveUi::sync_authority`；**选项 (a) 第二片（2026-10-06）**：`Plan::Host` + `apply_host_action` + `HttpServer::apply_host_action` 把 GUI 的**写**入口（撤销族 / 卷帘铅笔）落到那一个 `Domain` 上（`UndoPort` 的 `RefCell<UndoSession>` 命中 1→0）；**选项 (a) 第三片（2026-10-06）**：`src/project_lock.rs` 以 `#[path]` 共享 `yeban-mcp` 的锁实现，`save_project_file` / `save_archive_file` 各取一次 `ExclusiveWrite`（拿不到 ⇒ `SaveError::Locked`，一个字节都不写），判据 B14 + `in_process_mcp_lock.rs` 两条 + lib 单测，两条负向实测（两个取锁点各一条）都会红 | **"单一写者会话"未建**（GUI 保存与控制面会话在同一文件上互斥 ⇒ 控制面存活时 GUI 保存被拒；要同时成立须给宿主加保存动作，第三片 §7.4 有实测依据）；~~生产窗口没有运行期重投影~~ ⇒ **已关闭（第 (b) 项，2026-10-06）**：`crates/yeban-app/src/reproject.rs` + `HttpServer::set_project_revision_sink` + `run_gui` 的安装点；判据 `tests/production_reprojection.rs`（真环回 socket + 生产窗口 + 事件循环弹出排队调用 + 四条负向实测） | **无需新裁决** —— 方向已是**问题 6 (a)**（本项上方「执行状态」），剩下的是**执行**：建"单一写者会话"（宿主保存动作）—— 运行期重投影已由第 (b) 项关闭 |
| `ROAD-M4-010` | V1/V2 Web 包袱已彻底删除；汇合项（`P4_Gate` 四个入边）| **它必然被其它项拖住**，不可单独提前判 | **无需单独裁决** —— 随 `M4-006/007/008` 与两项 PENDING 的处置而自然收口 |

**读法**：上表四行里，`M4-006` 与 `M4-010` **不需要你新增裁决**（前者依既有 `D50` 裁决为长期 PENDING，后者是汇合项）；
**问题 1**（`N2` 快捷键）、**问题 2**（响度传输）与**问题 3**（`D47`/`.als` 出口）**已关闭**
（分别按建议 (1)/(a)/(a) 落地：GUI 绑逻辑键 + 无头端口判据；既有控制面的 `since` 游标 +
`InProcessMcp::engine_readings_handle` 宿主注入口 + 真 socket 判据；CLI `--export-als` + 损失表可见）；
真正需要你的只剩 **问题 5**（参考 `.als`）以及与 Phase 4 并列的 **问题 4**（`MUST-GATE-014` 素材）
—— **问题 6 的裁决已经是 (a)**（本轮交出**第三片**：GUI 的保存路径也纳入同一把 `.yeban.lock`，并实测裁决"仍不翻 `read_only`"，见上方「执行状态」），它现在缺的是**执行**而不是**你选一个字母**。

---

## 7. Logic 导出器 · 多轨需要一份新供体 —— ✅ **唯一仍开放的问题**

**背景（一段话）**：Logic 导出器的**单轨**能力已被实测证明：本机 **Logic Pro 12.2** 能打开供体拼接产物（负责人，2026-10-06）。但**多轨**不行。原因是当前 MIT 供体 `F0_baseline` 是**紧凑形态**：参考规范 §10.6.3 要求的两张表在它里面**不存在**（所需偏移超出它的 10,756 字节 `gnoS`），且它的 363 条混音条带里**没有一条** UUID 全零（355 条是 `ee…` 占位符）。真实工程里**有**全零条带（`Swing!` 147/549、`ocean eyes` 814/1162、`MONTERO` 327/1089），所以**规范没错，是这份供体的形态不同**。

**请你选一项**：

| 选项 | 内容 | 成本 | 后果 |
| :--- | :--- | :--- | :--- |
| **(A) 推荐** | 用 **Logic Pro 12.2** 新建工程 ⇒ 轨道加到 **8** 条左右 ⇒ **删到只剩 1 条** ⇒ 保存 ⇒ 把路径告诉我 | 约一分钟 | 得到"N 通道预留混音器"，即有最多 N−1 个空槽 ⇒ 槽位激活**可以实施**并**可复跑** |
| (B) | 提供**另一个 MIT 供体**（带全零 UUID 条带）| 取决于你 | 同上，但需再核对许可与形态 |
| (C) | 明确"**多轨暂不做**" | 零 | 我把这条记入台账为**依裁决不做**，并保持单轨能力 |

**说明**：选项 (A) 的供体是**你自己的作品**，不受第三方版权约束，我会照 `assets/logic-donor/` 的既有做法写明来源。

**为什么需要它**：参考规范 §10.6.1 的原话是"轨道**无法**从零合成"——新建通道会触发 Logic 的混音器与环境扩展，并重排整个 `OCuA` 块。规范给的方法就是"(A)"那条：建 N 条、删到只剩一条、保存。

**不要做的**：⛔ **不要**为此做第五次 Logic 测试。当前产物与上一次的**记录数、顺序、被改记录集合、根版本码、`gnoS` 主体全部相同**，只差登记文本与被映射区域的名字和音符。所以第五次测试测的是同一批字节。

