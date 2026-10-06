# 三方对齐矩阵（系统 / UI / MCP，70 行）

> **一句话定位**：这份文件回答**唯一**一个问题 ——
> **"某个功能在系统侧实现了没有、在 UI 侧暴露了没有、在 MCP 侧暴露了没有，三者错位在哪、为什么、谁来补、什么时候补"**。

## 0. 分工声明（四张表互不复制，能引用就引用）

| 表 | 唯一负责回答 | 权威来源 | 机械守卫 |
| :--- | :--- | :--- | :--- |
| **本表** `docs/ledger/feature-alignment.md` | **三方暴露**：系统 / UI / MCP 各自有没有、错位在哪 | `crates/*/src/lib.rs` 的 `IMPLEMENTED_SPEC_IDS`、`schemas/mcp-tools.schema.json`、`crates/yeban-ui-mcp/src/methods.rs`、`docs/ledger/*-notes.md` | `scripts/gates/check_feature_alignment.py` |
| `docs/ledger/gate-status.md` | **发布门禁**过没过（`MUST-GATE-*` / `BASELINE-*`） | 路线图 §5 | `scripts/gates/check_gate_status.py` |
| `docs/ledger/phase-status.md` | **阶段项**做没做完（`ROAD-*`，46 项） | 路线图 §3 | `scripts/gates/check_phase_status.py` |
| `docs/ledger/human-decisions.md` | **待人类裁决**（`HD-01..HD-42`） | `docs/adr/ADR-0001-*` + 负责人追认 | `scripts/gates/check_decisions.py` |

引用规则（这是四张表不互相矛盾的关键）：

- 某个功能的**完成度**（"部分做到哪一步"）**不在本表复制**：本表只记**暴露面**三档（有 / 部分 / 无）与**错位**。
  某项进度若有 `ROAD-*` 编号，本表在"系统"列**引用**它；状态请去 `phase-status.md` 读。
- 某项卡在门禁上时，本表**只写门禁 ID**；门禁状态请去 `gate-status.md` 读。
- 某项卡在人类裁决上时，本表**只写 `HD-*` 编号**；选项与后果在 `human-decisions.md`。
- 本表的**独有内容**只有三样：① 同一功能在三侧的**载体**（精确到 crate / 文件 / 元素 ID / 工具参数）；
  ② **"系统有而 UI 或 MCP 没有"**（以及反方向）的错位与理由；③ 与任何一条 `ROAD-*` 都不等价的"暴露缺口"。

## 1. 对齐分类汇总（由守卫逐行对账）

分类的判定**完全机械**（`scripts/gates/check_feature_alignment.py` 的 `classify()`），规则如下：

- 系统列首词 ∈ `已实现` / `部分` / `计划` / `无`；UI 列与 MCP 列首词 ∈ `有` / `部分` / `无`。
- `暴露(x)` 定义为 `x ∈ {有, 部分}`；`系统有(x)` 定义为 `x ∈ {已实现, 部分}`。
- 判定顺序：`计划` 或三侧全空 ⇒ **仅计划**；`系统=无` 且任一侧暴露 ⇒ **UI 或 MCP 独有**；
  `系统有` 且两侧都暴露 ⇒ **三方齐全**；只有 UI 暴露 ⇒ **系统+UI**；只有 MCP 暴露 ⇒ **系统+MCP**；否则 ⇒ **仅系统**。

- 三方齐全：26 行
- 系统+UI（MCP 无）：9 行
- 系统+MCP（UI 无）：15 行
- 仅系统：9 行
- 仅计划（系统也未实现）：8 行
- UI 或 MCP 独有（系统没有）：5 行
- **合计：72 行**

> ⚠ 只改表格不改这一节 ⇒ `check_feature_alignment.py` 立刻变红（第 4 条判据）。数字要么能被命令复核，要么别写。

## 2. 三档标记、状态词与"无"的三件套（守卫机械判定）

每行的形状（列内用**全角** `｜` 分隔，因此单元格里不会出现半角竖线）：

```text
| 功能 | 系统标记｜证据 | UI标记｜载体 | MCP标记｜工具·参数 | 原因：…；计划：…；状态：<状态词> |
```

1. **"有"必须写清载体**：系统列写 crate / 文件 / 规范 ID；UI 列写元素 ID 前缀 / `.slint` 文件 / `ui/*` 方法名；
   MCP 列写**工具名 + 参数名**（`ui/*` 方法同样写参数名）。
2. **"无 / 部分"必须写清三件事**：`原因：…；计划：…；状态：…`，三段按此次序、各自非空。
   三者缺一 ⇒ 守卫变红（"无"最容易被写成一句"以后再说"）。
3. **状态词只许七个**：`三方齐全` / `PENDING` / `未到期` / `有意不做` / `人类决策中` / `待接线` / `未核查`。
4. **不许乐观**：任何"有"若无法指出具体载体，写 `无｜未核查`，**不写"应该有"**。本表有 1 行是 `未核查`（见 §16）。
5. `三方齐全` 只在**三侧都暴露**时使用（系统=已实现 ∧ UI=有 ∧ MCP=有）；守卫会反向检查这一点。

**六个分类标签下的独有含义**（读表时别混）：

| 分类 | 含义 | 为什么要单列 |
| :--- | :--- | :--- |
| 三方齐全 | 三侧都能引用到 | 正常的收口状态 |
| 系统+UI（MCP 无） | 引擎/模型做了、界面也做了、**没有工具能碰** | AI Agent 够不着这些能力，是 MCP 工具集的**覆盖缺口** |
| 系统+MCP（UI 无） | 引擎/模型做了、工具也做了、**界面没有入口** | 人类用户够不着这些能力，是 UI 的**入口缺口** |
| 仅系统 | 只有 crate 内部有 | 尚未暴露给任何外部使用者 |
| 仅计划（系统也未实现） | 三侧都没有，但**计划里有** | 区分"还没轮到"与"漏了" |
| UI 或 MCP 独有（系统没有） | 界面或工具侧有东西，但系统侧**没有对应实现** | ⚠ 这一类必须逐条解释：可能是界面在演假数据，也可能是系统侧分工在别处 |

## 3. 分组 A —— 数据模型与工程

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| 960 PPQ 整数时钟 + `EntityId(Ulid)` + 全 `BTreeMap` 确定性 AST | 已实现｜`crates/yeban-model/src/ids.rs:26`（`PPQ=960`）、`ids.rs:41`（`EntityId`）、`crates/yeban-model/src/project.rs:57`（全 `BTreeMap`）；规范 ID `MODEL-AST-001..005,007` | 有｜`crates/yeban-app/src/bridge.rs`（`YebanProjectV1` → `ViewState` 单向投影）+ `crates/yeban-app/src/elements.rs` 的 `MODEL_DRIVEN_FAMILIES`（9 族） | 有｜`yeban_query_project`（`limit`/`offset`/`fields`，`crates/yeban-mcp/src/tools.rs:457`） | 状态：三方齐全 |
| `YebanProjectV1` 契约与 `schema_version` 版本信封（不匹配即拒） | 已实现｜`crates/yeban-model/src/project.rs:9,1503,1601`（`check_readable`）+ `schemas/project.schema.json`；`ROAD-M1-001` | 有｜`crates/yeban-app/src/bridge.rs` 的 `from_project` | 有｜`yeban_open_project`（`path`/`readOnly`）、`yeban_query_project`（`fields`） | 状态：三方齐全 |
| 三层状态物理隔离（`MODEL-ISO-001`：持久化 / 会话运行态 / 本机配置） | 部分｜持久化层已实现（`crates/yeban-model/src/project.rs:57`）；`SessionRuntimeState` 与 `LocalMachineConfig` **两个类型不存在**（`grep -rn "struct SessionRuntimeState\|struct LocalMachineConfig" crates/` 命中 0，见 `docs/ledger/phase-status.md` §3 `ROAD-M1-001`） | 无｜占位常量 `SESSION_TIMECODE` / `SESSION_BRANCH_NAME` 在 `crates/yeban-app/src/scene.rs` | 无｜十个领域工具里没有会话运行态检索面 | 原因：第二层类型未定义（模型/引擎线缺口）；计划：`yeban-model` / `yeban-engine` 出 `SessionRuntimeState` 后，UI 占位常量与 MCP 查询面**同一次**接；状态：PENDING |
| 声学路由唯一真理源 `RoutingGraph`（`folder_id` 只做 UI 折叠） | 已实现｜`crates/yeban-model/src/project.rs:1376` + `validate()`；文档口径 `project.rs:1115`；契约 `schemas/project.schema.json` 的 `routing_graph`；`ROAD-M1-002` | 无｜没有路由/连线视图（`folder_id` 明文"仅用于界面层树状折叠"，`project.rs:1115`） | 部分｜`yeban_propose_section`（`sectionName`/`stylePreset`/`bars`/`scale`/`dryRun`）会判环路 `CYCLE_DETECTED`，但**声部连接不产出**：`crates/yeban-mcp/src/domain/section.rs:109` 报 `data.unwired = ["clipPoolEntries","routingEdges"]` | 原因：MCP 侧仍按"`Op` 全集缺 `AddClip`/`AddRoutingNode`"的**旧假设**实现（`crates/yeban-mcp/src/domain/section.rs:8-16`），而 `crates/yeban-model/src/ops.rs:190,197` 已有该变体（`HD-12` 已裁决、提交 `4190651` 落地）；计划：`yeban-mcp` 线改用 `Op::AddClip` / `Op::AddRoutingNode` 并删 `unwired` 上报；状态：PENDING |
| 领域操作日志 + `CommitGraph` 撤销树（匿名分叉 / 命名分支 / 每 256 次快照） | 已实现｜`crates/yeban-model/src/ops.rs:122`（29 个 `Op` 变体）；`crates/yeban-model/src/commit.rs:39`（`SNAPSHOT_INTERVAL=256`）、`commit.rs:342`（`fork_anonymous`）；`ROAD-M1-003` | 部分｜载体 `crates/yeban-app/ui/dialogs/undo_tree_modal.slint` + `elements.rs` 的 `undo-tree-*` 与 `transport-commit-button` / `-revert-button` / `-branch-button`；但 `crates/yeban-app/src/main.rs:141` 的 `wire_callbacks` **只打 stderr**（`main.rs:135-140` 明文"故意什么都不做"） | 无｜十工具与 `ui/*` 14 条方法都没有撤销 / 重做 / 提交图谱面 | 原因：UI→模型写入方向被显式推迟（避免制造"UI 已经通了"的假象）；计划：`yeban-app` 事件循环接 `yeban-model::Op` 归约；状态：待接线 |
| AI 提案分支（Musical PR）的建 / 合 / 拒 | 已实现｜`crates/yeban-model/src/commit.rs:342` + `crates/yeban-mcp/src/domain/proposal.rs` | 部分｜`crates/yeban-app/ui/dialogs/musical_pr_drawer.slint` + `elements.rs` 的 `musical-pr-accept-button` / `musical-pr-reject-button` / `musical-pr-drawer`；回调 `accept-ai-proposal` / `reject-ai-proposal`（`crates/yeban-app/ui/app.slint:179-180`）在 `main.rs:148-149` **未接线** | 有｜`yeban_propose_section`（`sectionName`/`stylePreset`/`bars`/`scale`/`dryRun`）、`yeban_merge_proposal`（`proposalId`/`commitMessage`）、`yeban_reject_proposal`（`proposalId`/`reason`） | 原因：UI 回调未接线（同一根因）；计划：`yeban-app` 接控制面，或按 `ROAD-M4-001` 引入与 `yeban-mcp` 的依赖边；状态：待接线 |

## 4. 分组 B —— 操作与撤销

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| 10,000 步随机操作序列守恒（`proptest` + 逆操作） | 已实现｜`crates/yeban-model/src/ops.rs:1620`（`CI_SEQUENCE_STEPS = 10_000`）；判据 `state_tree_is_conserved_under_reverse_undo`（`ops.rs:3364`） | 无｜UI 层没有属性测试（守恒性是模型层财产，跨层重复会造第二份口径） | 无｜MCP 层同样没有（理由同上） | 原因：**有意不做** —— 这条判据的消费者是 `MUST-GATE-010`，不是界面或工具；计划：无；状态：有意不做 |
| **执行**撤销 / 重做（可逆能力对外可调用） | 已实现｜生产入口 `crates/yeban-model/src/commit.rs:505`（`CommitGraph::undo`）、`:521`（`undo_with`，游标由调用方保存）、`:533`（`op.apply_inverse(doc)?`，在 `#[cfg(test)]`（`commit.rs:568`）**之前** ⇒ 是生产代码）；撤销原语 `crates/yeban-model/src/ops.rs:112,954`；时延判据 `BASELINE-004`（`docs/ledger/gate-status.md:44`：p99 0.084 µs，`undo_batch2` 1.834 µs） | 部分｜**只有展示，没有操作**：`crates/yeban-app/ui/dialogs/undo_tree_modal.slint` 的**唯一** callback 是 `close`（`grep -oE 'callback [a-z-]+'` ⇒ 只有 `close`）；`crates/yeban-app/ui/app.slint:166,177,484,486` 只有 `undo-tree-open` 展示态；元素已登记（`crates/yeban-app/src/elements.rs:818,825,833,841`）；快捷键解析在 `crates/yeban-app/src/input.rs:369-370`（`Action::Undo`/`Redo`），但派发在 `crates/yeban-app/src/main.rs:146` 未接线 | 无｜十工具里没有撤销 / 重做；`crates/yeban-ui-mcp/src/methods.rs` 的 14 条方法里也没有 | 原因：三侧各自只做到"模型侧可逆"，**没有谁负责把它接到面上** —— `grep -rn "UndoCursor" crates/` 只命中 `crates/yeban-model/src/commit.rs` 与 `crates/yeban-model/src/lib.rs` ⇒ `yeban-model` 之外**零调用者**；计划：新开一条线**同时**接 UI（`undo_tree_modal` 真的触发 `undo_with`）与 MCP（工具或 `ui/*` 方法）；状态：PENDING |

## 5. 分组 C —— 容器与文件

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| `.yeban` ZIP 容器 + CAS 资产池（`assets/{sha256}`） | 已实现｜`crates/yeban-model/src/container/mod.rs:59,511`；判据 `crates/yeban-model/tests/container_roundtrip.rs`；`ROAD-M1-004` | 部分｜CLI 路径在 `crates/yeban-app/src/open.rs` 与 `crates/yeban-app/src/save.rs`；**GUI 路径无打开/保存入口**（`main.rs` 事件循环未接，`docs/ledger/app-cli-notes.md` §7 #6） | 有｜`yeban_open_project`（`path`/`readOnly`）、`yeban_save_project`（`force`）、`yeban_close_project`（`saveFirst`） | 原因：GUI 路径未接（`crates/yeban-app/src/main.rs:135`）；计划：`yeban-services` / 会话层给出权威"当前工程路径"，再接入事件循环；状态：待接线 |
| Zip-Slip 与解压炸弹上限（四道闸门） | 已实现｜`crates/yeban-model/src/container/path.rs` + `crates/yeban-model/src/container/mod.rs:89`（`ContainerLimits`）；判据 `crates/yeban-model/tests/container_adversarial.rs`；`MUST-GATE-006` / `MUST-GATE-007` | 无｜UI 层没有自己的解压路径（容器定位是模型层财产） | 有｜经 `yeban_open_project`（`path`）隐式强制（两侧共用 `read_project_container`） | 原因：**有意不做** —— 第二份容器实现 = 第二份被绕过的防御面；计划：无；状态：有意不做 |
| `.yeban.lock` 排他锁（原子创建 + OS 建议锁 + 崩溃残留可接管） | 已实现｜`crates/yeban-mcp/src/domain/lock.rs`（`std::fs::File::try_lock`，Unix 上是 `flock(2)`、Windows 上是 `LockFileEx`）；判据 `crates/yeban-mcp/tests/lock_advisory.rs`；台账 `docs/ledger/lock-advisory-notes.md`；`MUST-GATE-008` | 无｜`grep -rn "yeban.lock\|try_lock" crates/yeban-app/src/` 命中 0 —— GUI 路径不持锁 | 有｜`yeban_open_project`（`path`/`readOnly`）持锁，冲突返回 `PROJECT_LOCKED` | 原因：GUI 目前按"单实例"假设，未接锁；计划：`yeban-services` / 会话层统一持锁者；状态：未到期（多实例是 v1.1+ 范围） |
| 非容器文件精确拒绝（容器是唯一格式） | 已实现｜app 侧 `OpenError::NotAYebanContainer`（`crates/yeban-app/src/open.rs`）；MCP 侧 `crates/yeban-mcp/src/domain/store.rs:293` 同语义；判据 `store.rs:902`（同一份 JSON 在容器里仍能打开） | 有｜CLI `--open <path>`（`crates/yeban-app/src/cli.rs`，`docs/ledger/app-no-compat-notes.md`） | 有｜`yeban_open_project`（`path`）对非容器返回 `FILE_NOT_FOUND` 或 `IO_ERROR`（`docs/ledger/mcp-no-compat-notes.md` §3） | 状态：三方齐全 |
| 保存落盘：临时文件 → `fsync` → `rename`，以及 `history.dag` 的内容 | 部分｜原子替换两处都有（`crates/yeban-mcp/src/domain/store.rs:44-46`、`crates/yeban-app/src/save.rs:205`）；但 `history.dag` 由 `yeban_save_project` / `ui/force_save` 写成**空字节**（`docs/ledger/app-mixer-notes.md` §7 #7），权威内容在 `crates/yeban-model/src/commit.rs` | 有｜`ui/force_save`（无参数，`Scope::AppSave`） | 有｜`yeban_save_project`（`force`） | 原因：容器布局要求该条目存在，但提交图谱序列化未被任一入口调用；计划：`yeban-model` 提供 DAG 序列化面，再由两个保存入口同一次接；状态：PENDING |

## 6. 分组 D —— 自动化

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| 参数自动化平滑（`ARCH-DSP-001`，τ≈5ms 一阶低通 + 吸附） | 部分｜原语已实现 `crates/yeban-dsp/src/smoothing.rs`（`ParamSmoother`）；音频路径**无消费者**（`grep -rn "ParamSmoother" crates/*/src` 除该文件外命中 0）；`docs/ledger/phase-status.md` §4 `ROAD-M2-006` 引 `D44①`"参数平滑仍未实现"；自动化**录制**（`OpOrigin::AutomationRecord` 在 `crates/yeban-model/src/ops.rs:48`）无写入路径 | 无｜无平滑时间常数相关控件（自动化泳道的写模式角标只显示模式） | 无｜十个工具都没有自动化面；`yeban_set_macro`（`trackId`/`macroIndex`/`value`）只写目标值，不暴露平滑参数 | 原因：设备链参数求值接口未落地（`docs/ledger/mcp-render-notes.md` needs-3）；计划：engine 线出参数求值 / 自动化曲线求值接口；状态：PENDING |
| `yeban_edit_automation` | 已实现｜`crates/yeban-model` 的 `automation_value_at`（**唯一求值入口**）+ `Op::SetAutomationPoint` / `Op::SetAutomationLane` | 有｜`crates/yeban-app/src/automation.rs`（泳道 → 折线投影，含细采样） | 有｜`crates/yeban-mcp/src/domain/automation.rs`（读写同一条泳道；写走 `Op`，可逆） | 原因：AI 侧此前只能看见自动化而不能读写，泳道求值只有界面在用；计划：由 `ADR-0001 D46` 裁决的工具集扩张落地；注：求值必须走 `automation_value_at`（禁第二份求值，判据有牙）；状态：三方齐全 |
| `yeban_query_engine_state` | 已实现｜`SessionRuntimeState`（`MODEL-ISO-001` 第 2 层）+ `project.audio_config.sample_rate` + `TrackV3::devices`；缓冲帧数住在 `yeban-engine` | 有｜`crates/yeban-app/src/engine_host.rs`（走带 / seek / play）+ `meters.rs` | 有｜`crates/yeban-mcp/src/domain/engine_state.rs`（只读；缓冲走宿主注入的镜像） | 原因：引擎运行态此前只在界面/宿主进程内可读，AI 侧看不见；计划：D46 落地；注：形态 B（stdio 二进制）没有引擎进程 ⇒ `bufferFrames` 只能为 null（已如实写在契约与台账）；状态：三方齐全 |
| `yeban_import_audio` | 已实现｜`yeban-decode`（解码 + `PcmBudget`）+ `Op::AddClip` | 无｜`crates/yeban-app/src` 里没有音频导入路径（`decode_path` 与 `yeban_decode` 在 app 侧均零命中；`bridge.rs` 的音频片段是夹具假哈希） | 有｜`crates/yeban-mcp/src/domain/import_audio.rs`（走既有 CAS 池 + `Op`） | 原因：UI 侧从来没有导入音频文件的入口，`yeban-decode` 此前只被 MCP 的渲染片段路径消费；计划：UI 接一条导入动作（同一个 `Op::AddClip` + 同一份 CAS 池）；状态：待接线 |
| `yeban_export_diagnostics`（诊断包导出，**D56**） | 已实现｜`crates/yeban-engine/src/diagnostics.rs`（采集 env/git/状态/配置 + zip + 逐项 sha256 + 脱敏）与 `crates/yeban-mcp/src/domain/diagnostics.rs`（领域接线） | 有｜`app.slint` 的 `diagnostics-export-action` → `host.rs` 的 `on_export_diagnostics` → `UiAction::ExportDiagnostics` | 有｜`yeban_export_diagnostics`（可选 `outDir`，缺省写当前目录） | 原因：—；计划：—；状态：三方齐全 |
| 撤销 / 重做（`yeban_undo` / `yeban_redo` + UI 入口） | 已实现｜`crates/yeban-model/src/commit.rs:505,525`（`undo` / `undo_with`，**生产代码**）+ `ops.rs` 的逆操作原语；`BASELINE-004` 有单步撤销时延判据（p99 0.084 µs）；游标属 `MODEL-ISO-001` 的**会话运行态**（`SessionRuntimeState::undo_cursor`，**不落盘**） | 有｜`crates/yeban-app/src/undo.rs` + `ui/dialogs/undo_tree_modal.slint` 的动作 + `Cmd+Z` 派发（D45 接线） | 有｜`yeban_undo` / `yeban_redo`（`schemas/mcp-tools.schema.json` 已登记；共用 `crates/yeban-mcp/src/undo_session.rs` 的**同一份**实现，含 `dryRun`） | 原因：此前**模型有生产级实现、域外零调用者**（`UndoCursor` 只在 `yeban-model` 内出现；UI 弹窗唯一 callback 是 `close`；MCP/ui-mcp 里 `undo` / `redo` 0 命中）；计划：已由 `ADR-0001 D45` 裁决「UI+MCP 两侧同接、共用同一实现」并由本线落地；状态：三方齐全 |
| 自动化泳道（多泳道 + 曲线 + 单位轴标签） | 已实现｜`crates/yeban-model/src/automation.rs`；接线在 `crates/yeban-app/src/automation.rs`（`project_lanes_at_cursor`） | 有｜`crates/yeban-app/ui/workspace/arrangement_view.slint:53-72`（9 个平行数组由 `src/automation.rs` 单一事实源派生）+ `crates/yeban-app/src/elements.rs:503` 的 `track-{i}-automation-{key}-lane` | 无｜十工具与 `ui/*` 14 条方法都没有自动化泳道的读写面（AI 只能经 `ui/tree` 读到标签文本） | 原因：`schemas/mcp-tools.schema.json` 的 10 个工具**枚举是规范定的**，里面没有自动化工具；计划：**需要人类裁决**是否扩工具集（`HD-*` 报给负责人）；状态：人类决策中 |
| 宏与级联映射（`MacroMapping`） | 已实现｜`crates/yeban-model/src/ops.rs` 的 `Op::SetMacro` + `crates/yeban-mcp/src/domain/macros.rs` | 无｜`TrackV3::macros` 未进视图（`crates/yeban-app/src/bridge.rs` 无 macros 投影，`docs/ledger/app-mixer-notes.md` §7 #4） | 有｜`yeban_set_macro`（`trackId`/`macroIndex`/`value`） | 原因：UI 侧设备机架仍由演示常量驱动（`crates/yeban-app/ui/console/device_rack.slint` + `scene::DEVICE_NAMES`）；计划：`[UI-NOTE-004]` 设备机架改由 `TrackV3::devices` / `macros` 驱动；状态：PENDING |

## 7. 分组 E —— 音频引擎

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| cpal 设备宿主 + 纯函数格式协商 + `NullBackend` | 已实现｜`crates/yeban-engine/src/device.rs`；`ROAD-M0-001` | 有｜`ui/reload_engine`（无参数，`Scope::AppReloadEngine`） | 无｜十工具没有设备面；引擎重载只存在于 UI 控制面 | 原因：`MCP-TOOL-001..010` 是规范枚举，里面没有设备/引擎工具；计划：**需要人类裁决**是否扩工具集；状态：人类决策中 |
| OS 高优先级实时线程调度 | 无｜`crates/yeban-engine/src/device.rs:17` 与 `src/lib.rs:73` 明文"`[ROAD-M2-001]` 实时线程优先级未实现" | 无｜无对应控件 | 无｜无对应工具 | 原因：cpal 的 `realtime` feature 不覆盖 macOS / Linux-ALSA，自研需新 `libc` 依赖 + 平台 `unsafe` 审计（`docs/ledger/engine-rt-notes.md` §5.2 needs N3）；计划：依赖图裁决通过后落地；状态：人类决策中 |
| 引擎内部机制：无锁 SPSC 环 + 批量 API + 快照原子交换 + FTZ/DAZ | 已实现｜`crates/yeban-engine/src/ring.rs`、`block.rs`（`bulk_push_calls` / `bulk_pop_calls`）、`snapshot.rs`（`RetireQueue`）、`fpu.rs:57`（`enable_ftz_daz`）；判据 `crates/yeban-engine/tests/rt_zero_alloc.rs`、`meter_rt_contract.rs`；`ROAD-M2-002/003/007` | 无｜UI 线程只经 `crates/yeban-app/src/meters.rs` 消费电平队列 | 无｜无工具 | 原因：**有意不做** —— 这些是实时内部机制，暴露成外部接口会扩大红线 7 与红线 6 的攻击面；计划：无；状态：有意不做 |
| PDC 内部延迟补偿（Kahn 拓扑 + 关键路径 + 环形延迟 + 对齐不变式） | 部分｜`crates/yeban-engine/src/graph.rs:175,196`（`PdcPlan::compute`）、`latency.rs`；判据含 `pdc_alignment_invariant_holds_for_diamond`、`compensated_branches_line_up_sample_exactly`；**未接进** `EngineRuntime::render_block` 的实际混音路径（`docs/ledger/phase-status.md` §4 `ROAD-M2-004`） | 无｜无 PDC 显示 | 无｜`yeban_render_master` 只消费 `latency_samples`，没有 PDC 工具 | 原因：`CompensationBank` 目前"只构造 + 测试里用"；计划：engine 线接入混音路径，并在**同一提交**里回填 `LatencyTable`（`DEVELOPMENT_LEDGER.md` `D44②`）；状态：待接线 |
| 录音（`ARCH-REC-*`）与自动保存（`ARCH-SYS-*`） | 无｜`grep -rin "struct Recorder" crates/ --include=*.rs` 命中 0、`"fn record"` 的 2 处命中都在 `crates/yeban-engine/examples/measure_latency.rs:243,262`（那是 `cpal::StreamInstant` 时延采样器，**不是** DAW 录音）；`grep -rin "autosave" crates/ --include=*.rs` 命中 0 —— 两个需求族在 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 里有条目、**代码里没有** | 无｜`transport-record-button` 在 `crates/yeban-app/ui/transport.slint` 里是纯视觉按钮（`main.rs:141-151` 无 `record` 回调）；无自动保存提示控件 | 无｜十工具没有录音面，也没有自动保存/快照工具 | 原因：完全未实现（`ARCH-REC-*` / `ARCH-SYS-*` 无代码）；计划：`yeban-engine` 出录音路径（`ROAD-M2-*` 之后）、`yeban-services` 出自动保存定时器；状态：未到期 |
| Voice stealing 淡出 + PolySynth 发声 | 部分｜`crates/yeban-engine/src/synth.rs`（PolySynth）；判据 `crates/yeban-engine/tests/steal_fade.rs`（实测硬窃取台阶 0.555190 → 淡出 0.000031）；但**引擎不发声** —— 轨道渲染是占位静音（`docs/ledger/app-mixer-notes.md` §7 #1） | 无｜无声部 / 乐器面板 | 无｜`DeviceKind` 没有 SFZ 变体（`docs/ledger/mcp-render-notes.md` pending），`yeban_edit_notes`（`trackId`/`clipId`/`ops`/`idempotencyKey`）只写 MIDI 事件 | 原因：声部合成切片未接实时路径 + 323 款素材未入库（`HD-31`）；计划：`yeban-sfz` 接入合成路径 + 人类采集素材；状态：PENDING |

## 8. 分组 F —— 混音与电平

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| VU / 峰值电平 SPSC 链路（有损队列 + 抽干取最新） | 已实现｜生产端 `crates/yeban-engine/src/meter.rs:583`（`meter_channel`）；消费端 `crates/yeban-app/src/meters.rs:304`（`MeterRuntime` / `drain_latest`）；台账 `docs/ledger/engine-meters-notes.md`；`ROAD-M2-008` | 有｜`crates/yeban-app/ui/console/mixer_console.slint` + `crates/yeban-app/src/elements.rs` 的 `track-{i}-meter` / `mixer-master-meter` | 无｜无电平工具；`ui/property`（`elementId`/`name`）只支持 10 个属性（`crates/yeban-ui-test-port/src/inspect.rs:217`：`role`/`label`/`id`/`type`/`x`/`y`/`width`/`height`/`opacity`/`valid`），**读不到电平数值** | 原因：`ControlNode` 没有数值字段（`crates/yeban-ui-test-port/src/tree.rs:120` 只有 `id`/`role`/`label`/`bounds`/`dynamic_region`/`parent`）；计划：扩 `ControlNode` + `property_of` 清单（`docs/ledger/app-mixer-notes.md` §7 #2）；状态：PENDING |
| 通道条：音量 / 声相 / 静音 / 独奏 / 主控 | 部分｜模型字段齐（`crates/yeban-model/src/project.rs:1118-1137` 的 `TrackV3`：`volume_db` / `pan` / `mute` / `solo`）；但 `Op` 全集**没有**混音写入变体（`crates/yeban-model/src/ops.rs` 的 29 个变体里只有 `SetParam` / `SetRoutingGain`，没有 `SetTrackVolume` / `SetPan` / `SetMute` / `SetSolo`） | 有｜`crates/yeban-app/ui/console/mixer_console.slint` + `elements.rs` 的 `track-{i}-fader` / `-mute-button` / `-solo-button` / `-channel-strip` 与 `mixer-master-*` 全套；推子位置由 `volume_db` 驱动，但点击/拖动**不改工程**（`docs/ledger/app-mixer-notes.md` §7 #3） | 无｜十工具没有混音面；`yeban_set_macro`（`trackId`/`macroIndex`/`value`）只能经宏间接影响参数 | 原因：① `ARCH-OPS-001` 的 `Op` 全集缺混音写入变体；② UI 回调未接线；计划：先补 `Op` 变体（ADR 裁决），再让 `yeban-app` 事件循环接上；状态：PENDING |
| 真峰值（8×/16× 过采样）与 LUFS 测量 | 已实现｜`crates/yeban-dsp/src/loudness.rs`；台账 `docs/ledger/dsp-loudness-notes.md`（BS.1770-4 系数由同一原型推导、复现到 3.3e-16）；`HD-26` / `HD-27` 已裁决 | 无｜无电平 / LUFS 显示控件（`dB [-60.0, 12.0]` 文本只出现在自动化轴标签） | 部分｜`yeban_render_master` 有 `normalize`，**没有** LUFS 或真峰值目标参数；响应里带 `dither` 字段 | 原因：规范 §7.2 给 `yeban_render_master` 的参数表只有 `format` / `sampleRate` / `normalize` / `path`，没有响度目标位；计划：**需要人类裁决**是否扩参数表；状态：人类决策中 |
| 60Hz 主线程电平心跳 | 无｜`crates/yeban-app/src/engine_host.rs:39` 明文"本切片**没有 60Hz 主线程心跳**，`reload` 末尾主动 `drain` 一次即可" | 部分｜`MeterRuntime::poll` / `drain_latest` 已实现（`crates/yeban-app/src/meters.rs:304`）；无定时器驱动（`grep -rn "Timer" crates/yeban-app/src/*.rs` 命中 0） | 无｜无工具 | 原因：队列、丢帧语义、"只取最新"都做了并有判据，缺的只是**驱动它的定时器**；计划：`yeban-app` 事件循环加 60Hz 心跳（`ROAD-M2-008` 的剩余半条）；状态：待接线 |

## 9. 分组 G —— 解码与重采样

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| `symphonia` 解码 → 内容寻址 PCM 资产 | 已实现｜`crates/yeban-decode/src/decode.rs:97`（`decode_bytes`）、`asset.rs`；台账 `docs/ledger/decode-core-notes.md`；硬边界"解码永不在实时回调路径上"（`ARCH-TOP-002`） | 无｜无音频导入控件 | 部分｜只在 `yeban_render_master`（`format`/`sampleRate`/`normalize`/`path`）的音频片段路径上被消费（`crates/yeban-mcp/src/domain/render.rs:1379`）；**没有**独立导入工具 | 原因：规范 §7.2 的十个工具里没有音频导入位；计划：**需要人类裁决**是否扩工具集；状态：人类决策中 |
| `rubato` sinc 重采样（44.1 / 48 / 96 kHz 互转） | 已实现｜`crates/yeban-decode/src/resample.rs:83`（`resample_interleaved`，BlackmanHarris2 / 256 / 1024）；按 `D32` **不**作跨架构位级承诺 | 无｜无采样率选择控件（采样率是渲染参数，界面不重复暴露） | 有｜`yeban_render_master` 的 `sampleRate`（`crates/yeban-mcp/src/tools.rs:523-528`） | 原因：**有意不做** —— 避免在界面里造第二个采样率事实源；计划：无；状态：有意不做 |
| 解码尺寸上限与容器 / 位深 / 声道矩阵 | 已实现｜`crates/yeban-decode/src/limits.rs`；台账 `docs/ledger/decode-limits-notes.md`（上限口径写死并判据钉住） | 无｜无 | 部分｜渲染路径按同一上限拒绝（`crates/yeban-mcp/src/domain/render.rs` 的容器/编解码矩阵），但**没有**独立导入面可显式报上限 | 原因：上限只在"真的有导入入口"时才有意义；计划：与音频导入工具同一次落地；状态：PENDING |

## 10. 分组 H —— SFZ 与乐器

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| SFZ v2 零拷贝解析器（三级作用域 / `#define` / `#include` 沙箱） | 已实现｜`crates/yeban-sfz/src/parser.rs:941,951`（`parse_text` / `parse_sources`）；台账 `docs/ledger/sfz-core-notes.md`；`MUST-GATE-011`（部分） | 无｜无乐器加载控件 | 无｜无工具 | 原因：`yeban-sfz` **没有任何工作区内消费者**（`grep -rn "yeban-sfz" crates/*/Cargo.toml` 只命中自身与根清单登记）；计划：engine 线在合成路径接入 `yeban-sfz`；状态：PENDING |
| 固定容量声部池与确定性窃取（默认 512 声部） | 已实现｜`crates/yeban-sfz/src/voice_pool.rs:214`（`VoicePool`）；`ARCH-RT-004` | 无｜无 | 无｜无 | 原因：与上一条同一根因（无消费者）；计划：随 `yeban-sfz` 接线一起暴露；状态：PENDING |
| 设备机架与设备链（`DeviceKind` / `TrackV3::devices`） | 部分｜`TrackV3::devices` 在 `crates/yeban-model/src/project.rs`；**没有** `DeviceKind` 的 SFZ 变体（`docs/ledger/mcp-render-notes.md` 的 pending 段） | 部分｜`crates/yeban-app/ui/console/device_rack.slint` 仍用 `scene::DEVICE_NAMES` 演示常量 + `device-plugin-host-placeholder-card` 占位卡 | 无｜无设备工具 | 原因：设备链 DSP / 自动化求值接口未落地（`docs/ledger/mcp-render-notes.md` needs-3）；计划：`[UI-NOTE-004]` 设备机架改由工程驱动 + engine 参数面；状态：PENDING |
| 外部插件宿主（CLAP / VST3 + 崩溃看门狗） | 无｜`crates/yeban-plugin-host/src/lib.rs` 是 12 行纯文档占位（`grep -c "pub fn\|pub struct"` 命中 0） | 部分｜`crates/yeban-app/ui/console/device_rack.slint` 的 `device-plugin-host-placeholder-card`（占位卡，不是真宿主） | 无｜无工具 | 原因：`[v2.0.0]` 范围；计划：v2.0.0 插件宿主线；状态：未到期 |

## 11. 分组 I —— 离线渲染

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| Rayon 多核分层并行母带渲染 + 主总线字典序单线程归约 | 已实现｜`crates/yeban-render/src/render.rs:676,706`（`rayon::ThreadPoolBuilder` + `par_iter_mut`，按 `RoutingGraph` 分层）；确定性归约 `crates/yeban-render/src/sum.rs:59,81`（`fixed_order` / `reduce_ordered`）；判据 `crates/yeban-render/tests/l1_digest_contract.rs`；`ROAD-M4-004/005` | 无｜`grep -rn "render\|export\|bounce" crates/yeban-app/ui/` 无导出控件 | 有｜`yeban_render_master`（`format`/`sampleRate`/`normalize`/`path`） | 原因：GUI 导出面未做（无菜单、无对话框）；计划：`yeban-app` 或控制面接 `yeban-render`；另注 `BASELINE-001` 的"≥100× 实时"读数（单线程 106.3× / Rayon 135.9×）**只能算数量级**，达标判定要在规范指定参考机上复跑（`HD-38`）；状态：PENDING |
| RF64 / BW64 写入器 + BEXT 元数据 | 已实现｜`crates/yeban-render/src/rf64.rs`（bext 固定前缀 602 字节，v1/v2 读写）；`ROAD-M4-006` | 无｜无格式选择控件 | 有｜`yeban_render_master` 的 `format`（白名单 `wav` / `rf64` / `bw64`，`crates/yeban-mcp/src/domain/render.rs:183`） | 原因：**有意不做** —— 格式是渲染参数，界面不造第二份白名单；计划：无；状态：有意不做 |
| TPDF 抖动与位深量化 | 已实现｜`crates/yeban-render/src/dither.rs` + `rng.rs`（`dither_rng_for` 由 `project.rng_seed` 派生） | 无｜无 | 部分｜渲染路径**恒开**（`crates/yeban-mcp/src/domain/render.rs:168,694`，响应里报 `dither = "TPDF (ARCH-FMT-001)"`），**没有**开关参数 | 原因：`ARCH-FMT-001` 要求抖动是渲染固有步骤，**不该**由调用方关掉；计划：无；状态：有意不做 |
| MIDI 0/1 导出（SMF，`midly` 编码 + 自研 VLQ 回读） | 已实现｜`crates/yeban-midi/src/midi.rs`（含 `MThd`/`MTrk` 字节级独立核对）；**两个出口**：app CLI `yeban-app --export-midi <path>`（`crates/yeban-app/src/export_midi.rs` + 判据 `tests/cli_contract.rs` B12/B13；CI run 37254761445 ✓）与 MCP 只读工具 `yeban_export_midi` | 无｜GUI 仍无导出控件（**有意**：D47 把 GUI 导出定为 CLI/离线语义） | 有｜`yeban_export_midi`（无特有参数；只读，base64 回传字节 + `sha256` + 计数；`crates/yeban-mcp/src/domain/export_midi.rs` 复用 `yeban_midi::export::export_from_project`） | 原因：UI 侧仍无导出控件（`ADR-0001 D47` 把 GUI 导出定为离线/CLI 语义）；MCP 侧按本线直接指令新增**只读**工具（不落盘 —— 落盘出口仍是 app CLI）；计划：UI 侧维持 D47 的离线语义，AI 侧的字节获取由 `yeban_export_midi` 覆盖；状态：PENDING |
| 实验性 `.als` 导出（`experimental-als-export`） | 无｜`grep -rn "experimental-als-export" Cargo.toml crates/*/Cargo.toml` 命中 0；`grep -rin "export_als\|AlsExport" crates/ --include=*.rs` 命中 0（`docs/ledger/phase-status.md` §6 `ROAD-M4-007`） | 无｜无 | 无｜无 | 原因：完全未实现；前置能力已在（`crates/yeban-midi/src/midi.rs` + 音频片段进母带的通路）；计划：`[ARCH-FMT-002]` 线，需先有 `flate2` 特性门；状态：未到期 |

## 12. 分组 J —— 界面与交互

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| 四区工作区布局（走带 / 侧栏 / 编曲区 / 底部多标签控制台） | 无｜布局是 `.slint` 层财产，`crates/**` 里没有布局数据模型（`UI-GRID-*` 只规定几何，不要求暴露） | 有｜`crates/yeban-app/ui/app.slint:266,295`（装配点）+ `ui/transport.slint` + `ui/sidebar.slint` + `ui/workspace/` + `ui/console/console_tabs.slint` | 无｜`ui/tree`（`prefix`/`source`/`dynamicOnly`）只能**读**布局，不能改 | 原因：**有意不做** —— 布局不是可编程契约面（改布局的需求应由 `UI-GRID-*` 的规范修订承担，而不是一个 MCP 方法）；计划：无；状态：有意不做 |
| Session / Arrangement 双视图切换 | 已实现｜视图枚举 `crates/yeban-app/src/input.rs:219-225`（`View::Session` / `View::Arrangement`）；`F5` / `F6` 在 `input.rs:351-352` | 有｜`crates/yeban-app/ui/workspace/session_view.slint` + `ui/workspace/arrangement_view.slint` + `transport-view-toggle-button` / `-view-session-button` / `-view-arrangement-button` | 有｜`ui/switch_main_view`（`view` ∈ `arrangement` / `session`，白名单见 `crates/yeban-ui-mcp/src/methods.rs:220-252`） | 状态：三方齐全 |
| 语义元素 ID 注册表 + 控件树内省（含无障碍角色与标签） | 已实现｜`crates/yeban-app/src/elements.rs`（`is_well_formed_id`、`MODEL_DRIVEN_FAMILIES` 9 族）；`crates/yeban-ui-test-port/src/tree.rs:120`（`ControlNode`）、`src/inspect.rs:119`（`node_from_handle`）；`ARCH-UI-004` / `UI-TEST-001` | 有｜`crates/yeban-app/ui/**/*.slint` 的 `accessible-id`（实测 87 处）与 `accessible-role` / `accessible-label` / `accessible-item-index` | 有｜`ui/methods`（无参数）、`ui/tree`（`prefix`/`source`/`dynamicOnly`）、`ui/node`（`elementId`）、`ui/coverage`（`ids`） | 状态：三方齐全 |
| 响应式属性读取 | 部分｜只支持 10 个属性（`crates/yeban-ui-test-port/src/inspect.rs:217`）：`role` / `label` / `id` / `type` / `x` / `y` / `width` / `height` / `opacity` / `valid` | 部分｜滑块 `value`、开关 `checked`、文本 `text` 等**读不到**（`crates/yeban-ui-test-port/src/tree.rs:120` 的 `ControlNode` 没有这些字段） | 有｜`ui/property`（`elementId`/`name`）—— 但受上面的属性清单限制 | 原因：上游 `ElementHandle` 未暴露值字段，本线不编造（`docs/ledger/app-mixer-notes.md` §7 #2）；计划：扩 `ControlNode` + `property_of` 清单；状态：PENDING |
| 指针 / 键盘事件注入 | 已实现｜`crates/yeban-ui-test-port/src/port.rs`（`Operation::DispatchPointer` / `DispatchKey`、三级 `Permission`，默认 `ReadOnly`）；`UI-TEST-002` | 有｜所有 `TouchArea` 与快捷键动作（`crates/yeban-app/src/input.rs`） | 有｜`ui/dispatch_pointer_down`（`elementId`/`xOffset`/`yOffset`/`button`）、`ui/dispatch_pointer_move`（`x`/`y`）、`ui/dispatch_pointer_up`（`button`）、`ui/dispatch_key_press`（`keyCode`） | 状态：三方齐全 |
| 走带（播放 / 停止 / 录音 / BPM / 时间码 / 分支） | 部分｜**只有配置数据，没有走带引擎**：`crates/yeban-model/src/project.rs:378`（`TransportConfig`：bpm / metronome / count-in / launch）；`grep -rn "fn stop" crates/ --include=*.rs` 命中 0；`crates/yeban-engine/src/device.rs:327` 的 `fn play` 是**启动音频流**，不是 DAW 走带（`docs/ledger/engine-sound-notes.md` 明文"无走带控制，默认从 tick 0 起滚"）；录音与自动保存另见分组 E | 部分｜`crates/yeban-app/ui/transport.slint` 的 `playing` 属性与 `callback toggle-play()`（11 个 property/callback）+ `elements.rs` 的 `transport-play-button` / `-stop-button` / `-record-button` / `-bpm-field` / `-timecode` / `-branch-button`；`toggle-play` 在 `crates/yeban-app/src/main.rs:142` 只打 stderr | 无｜十工具无走带工具；只能经 `ui/dispatch_pointer_down`（`elementId`/`xOffset`/`yOffset`/`button`）盲点注入，且注入后同样不生效 | 原因：引擎侧走带未实现（`ROAD-M2-*` 的剩余工作），UI 控件先占位；加上走带位置属会话运行态（`[MODEL-ISO-001]` 第二层，未定义）；计划：Phase 2 剩余工作 + 会话层出 `SessionRuntimeState`；状态：PENDING |
| 卷帘视口裁剪 / **坐标映射与吸附** / 工具状态机（`[UI-NOTE-001/002/003]`） | 部分｜**投影层已落地**：裁剪 `notes_visible_in` + 六数组 `VisibleNotes`、视口边界属性 `roll-min/max-tick`/`roll-min/max-pitch`、按 x 的二分子集索引（`crates/yeban-app/src/bridge.rs`）；**仍缺**批量绘制路径；**工具状态机已实现**（三列矩阵 + `active-tool` 三级镜像 + `UiAction::SelectTool` 进入唯一下发点，均有判据与守卫 —— 第 557–563/607/608 轮更正旧文）；**R-Tree 有意未采用**（改用按 x 排序的二分索引，零新依赖）。⚠ 本节曾引 `piano_roll.slint:4,11` 的"本骨架没有实现"，**该源码注释已于第 608 轮更正**（引用保留于此作为历史） | 部分｜`piano-roll-grid` / `piano-roll-keys` / `piano-roll-tool-{name}-button` / `note-{ulid}-rect` / `velocity-{i}-bar` 元素齐；五个工具是 **UI 状态**，不是能作用到模型的编辑操作 | 无｜无工具 | 原因：虚拟化与编辑语义未实现（`docs/ledger/phase-status.md` §5 的 `ROAD-M3-001` / `ROAD-M3-002`）；计划：`yeban-render` 像素管线 + 会话运行态（当前编辑片段）；状态：PENDING | **第 563 轮补（工具状态）**：`active-tool` 已**镜像到 MainWindow**（宿主可读，判据在 `test_port_adapter.rs`），`UiAction::SelectTool` 已进入**唯一下发点** `undo.rs::dispatch_key` 并有判据 —— 但**快捷键本身仍不可用**，因为它卡在 `needs N2`（Slint 的 `KeyEvent` **没有物理码**，而输入层基于 `PhysicalKey`；见 `app-projection-notes.md` N2 与账本第 257 轮）。
| **全键盘音符操控**（`[UI-NOTE-005]`：方向键按网格平移 / `Alt` 1 tick 微调 / 上下半音 / `Shift+上下` 八度 / `Shift+左右` 改时值 / `Space`·`Enter` 试听） | 计划｜规范 §3.5 定义，**尚未实现**：`grep -rn "Arrow" crates/yeban-app/src/input.rs` **无命中**，`grep -rni "nudge" crates/yeban-app/src/*.rs` **无命中**（2026-10-05 实测；**刻意不用带半角竖线的 alternation** —— 那会把表格单元格切开） | 无｜卷帘没有键盘分支（同上第一次 grep 的结果） | 无｜无对应工具 | 原因：规范 §3.5 未接线，且**此前未被任何表追踪**（账本第 240 轮）；计划：待负责人决定是否纳入 Phase 4（未纳则保持本行如实为"未实现"）；状态：PENDING（未实现：两条 grep 均无命中；是否纳入 Phase 4 由负责人决定） |
| 截图（Tier-1 软件光栅化）与动态区域遮罩 | 已实现｜`crates/yeban-ui-test-port/src/render.rs:6,10`（自研 `Platform` → `MinimalSoftwareWindow` → `SoftwareRenderer`，明文禁 `i-slint-backend-testing` 出图）；`src/mask.rs`（`apply_masks` / `mask_is_effective` / `mask_rects_from_tree`）；`MUST-GATE-015` | 有｜真实窗口实例 `crates/yeban-app/src/live_surface.rs`；动态区标记在 `crates/yeban-app/src/elements.rs` | 有｜`ui/screenshot`（`maskDynamic`/`maxBytes`）、`ui/dynamic_regions`（无参数） | 状态：三方齐全 |
| 强制执行保存 与 重载音频引擎（两条 UI 控制面管理动作） | 已实现｜保存 `crates/yeban-app/src/save.rs:205`（`write_file_atomically`）；重载 `crates/yeban-app/src/engine_host.rs:174`（`reload`）+ `crates/yeban-engine/src/device.rs` | 有｜`ui/force_save`（无参数，`Scope::AppSave`）、`ui/reload_engine`（无参数，`Scope::AppReloadEngine`） | 部分｜`yeban_save_project`（`force`）覆盖"强制保存"；**引擎重载没有领域工具** | 原因：引擎重载不属 `MCP-TOOL-001..010` 的枚举；计划：**需要人类裁决**是否扩工具集（`docs/ledger/app-mixer-notes.md` §6 needs-2 与 #7）；状态：人类决策中 |

## 13. 分组 K —— 无障碍与视觉回归

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| 无障碍语义属性（角色 / 标签 / 序） | 已实现｜`crates/yeban-app/src/elements.rs:91`（`ElementKind::accessible_role`）；`crates/yeban-ui-test-port/src/inspect.rs:71`（`role_name`）；`UI-A11Y-003` | 有｜`accessible-id` / `accessible-label` / `accessible-item-index`（例：`crates/yeban-app/ui/console/piano_roll.slint:68-70`；自动化泳道标签带单位与写模式，`crates/yeban-app/src/automation.rs:377`） | 有｜`ui/tree`（`prefix`/`source`/`dynamicOnly`，节点含 `role` / `label`）、`ui/property`（`name` = `role` 或 `label`） | 状态：三方齐全 |
| 扫描码热键与 IME 合成态防护（`UI-A11Y-001/002`） | 已实现｜`crates/yeban-app/src/input.rs`（`PhysicalKey` 扫描码解析、`Resolution::ConsumedByIme`、`InputContext`） | 有｜`ui/property {"name":"isComposing"}`（实测两状态可区分，`UI-A11Y-002`）；载体是 app 侧**真的** `InputContext`（`crates/yeban-app/src/input.rs:290`），非影子变量 | 部分｜`ui/dispatch_key_press`（`keyCode`）能注入按键，但**合成态无法经协议设置** —— `crates/yeban-ui-mcp/src/methods.rs` 的 14 条方法里没有 IME 状态位 | 原因：**曾经的**原因是「UI 控制面没有 `dryRun`、IME 位不可观测」；计划：已由 `ADR-0001 D48` 裁决并由 `line/ui-mcp-dryrun-ime` 落地（CI run 37255336054 = success）；状态：待接线（**仅剩**：Slint 平台的 IME 事件源尚未接进 `InputContext` —— 载体与驱动点已就位，属 app 的 Slint 接线切片） |
| 无障碍测试（读屏用例 `TEST-SPEC-005` / WCAG 对比度 `TEST-SPEC-006`） | 无｜`grep -rln -i "wcag\|screen.reader" crates/` 命中 0（`docs/ledger/phase-status.md` §5 `ROAD-M3-006`） | 无｜无 | 无｜无 | 原因：完全未实现；计划：`[UI-A11Y-004]` 对比度预算 + `TEST-SPEC-005/006`，需人类批准基准图样；状态：PENDING |
| 分平台 Golden 基准图集与 SSIM 比对 | 部分｜`crates/yeban-ui-test-port/src/golden.rs`（强制 `tests/golden/<platform>/<name>.png` 路径形状、分平台严禁混用）+ `src/ssim.rs`（阈值 0.98）；但 `ls tests/golden` ⇒ `No such file or directory`（`ROAD-M0-008` / `ROAD-M3-007`） | 无｜截图一律落在 `target/ui-test-port/`，**不进仓库** | 部分｜`ui/screenshot`（`maskDynamic`/`maxBytes`）出 PNG 与像素证据，**不做**基准比对 | 原因：基准图需人类批准（1920×1080 stored deflate 下 ≈6.2 MB/张，逼近红线 9 的 10 MB；`docs/ledger/app-introspect-notes.md` needs-4/7）；计划：人类批图样与尺寸策略；状态：人类决策中 |

## 14. 分组 L —— MCP 服务与安全

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| stdio 传输与独立二进制 | 已实现｜`crates/yeban-mcp/src/transport/stdio.rs:39,71`（`serve_lines` / `serve_stdio`）+ `crates/yeban-mcp/Cargo.toml:15`（`[[bin]] name = "yeban-mcp"`）；`ROAD-M4-002` | 有｜`crates/yeban-ui-mcp/src/transport/stdio.rs` | 有｜JSON-RPC 方法 `tools/list`、`tools/call`（`crates/yeban-mcp/src/dispatch.rs:52,55`） | 原因：三方齐全，但有一条**判据覆盖缺口**：`grep -rn "CARGO_BIN_EXE_yeban-mcp" crates/` 命中 0 ⇒ "独立进程 + stdio"这条链在 CI 里没有端到端判据（`ROAD-M4-002`）；计划：仿 `crates/yeban-app/tests/cli_contract.rs:35` 的子进程级契约测试；状态：三方齐全 |
| 环回 HTTP 传输（绑 `127.0.0.1` + Bearer Token） | 已实现｜`crates/yeban-mcp/src/transport/http.rs` 的 `HttpServer::bind_loopback`（绑 `127.0.0.1:0` 后回读 `local_addr()` 断言环回）+ `BearerToken`；`MUST-GATE-009` | 部分｜`crates/yeban-ui-mcp/src/transport/http.rs`，由 `ui-mcp-http` feature + 运行时 `--enable-ui-mcp-http` **两道开关**控制，默认关 | 有｜feature `mcp-http`（`crates/yeban-mcp/Cargo.toml`） | 原因：默认关是红线 6 的硬要求（`AGENTS.md` §2）；计划：发行版不开，仅开发/测试按需开；状态：有意不做 |
| 六级 scope 鉴权（`ARCH-SEC-002`） | 已实现｜`crates/yeban-mcp/src/security.rs:129`（`Scope`，`Scope::ALL.len() == 6`）+ `authorize` | 有｜`crates/yeban-ui-mcp/src/methods.rs` 每条方法声明 `scope`（`UiRead` / `UiScreenshot` / `UiInject` / `AppSave` / `AppAdmin` / `AppReloadEngine`） | 有｜`tools/call` 前做 scope 检查（`crates/yeban-mcp/src/dispatch.rs:1` 模块头的流程） | 状态：三方齐全 |
| 三级权限分层（`[UI-MCP-001]` ReadOnly / Interactive / Administrative） | 已实现｜`crates/yeban-ui-test-port/src/port.rs`（`Permission` 默认 `ReadOnly`、`Operation` 到权限的映射） | 有｜`crates/yeban-ui-mcp/src/methods.rs` 的 `port_operation` 字段（由判据 `port_operation_tier_matches_scope` 钉住两张表不许漂移） | 有｜同上传导（`ui/*` 的执行面再判一次） | 状态：三方齐全 |
| `dryRun` 与 `idempotencyKey` 两个协议特性 | 已实现｜`crates/yeban-mcp/src/tools.rs:37,40`（`DRY_RUN_PARAM` / `IDEMPOTENCY_KEY_PARAM`）+ `crates/yeban-mcp/src/dispatch.rs:10,11,34`（dryRun 短路、相同键复用首次结果） | 有｜`ui/property {"name":"isComposing"}`（实测两状态可区分，`UI-A11Y-002`）；载体是 app 侧**真的** `InputContext`（`crates/yeban-app/src/input.rs:290`），非影子变量 | 有｜`yeban_propose_section`（`dryRun`）、`yeban_edit_notes`（`idempotencyKey`），两者同时属于 `COMMON_PARAMS` | 原因：**曾经的**原因是「UI 控制面没有 `dryRun`、IME 位不可观测」；计划：已由 `ADR-0001 D48` 裁决并由 `line/ui-mcp-dryrun-ime` 落地（CI run 37255336054 = success）；状态：待接线（**仅剩**：Slint 平台的 IME 事件源尚未接进 `InputContext` —— 载体与驱动点已就位，属 app 的 Slint 接线切片） |
| 能力发现（工具目录与方法目录） | 已实现｜`crates/yeban-mcp/src/tools.rs:612`（`catalog()`，供 `tools/list`） | 有｜`ui/methods`（无参数，`Scope::UiRead`；返回 `methodCount` = 14，`crates/yeban-ui-mcp/src/methods.rs:585`） | 有｜`tools/list` | 状态：三方齐全 |
| 进程内嵌入 `yeban-app`（形态 A） | 无｜`grep -n "yeban-mcp" crates/yeban-app/Cargo.toml` 命中 0 ⇒ **没有依赖边**（`docs/ledger/phase-status.md` §6 `ROAD-M4-001`） | 部分｜控制面能力在 `crates/yeban-app/src/live_surface.rs`（`build_live_ui` / `LiveAdminSurface`），但只作为 dev-dependency 的测试目标，发行路径 `main.rs` 一行未动 | 部分｜`Dispatcher::domain_mut()` 与 `Domain::open_in_memory()` 已就绪（`docs/ledger/tools-domain-notes.md` P7 明文"能力已就绪、仍未接线"） | 原因：app 不依赖 `yeban-mcp`；计划：集成者拍板非默认 feature + `ci.yml` 步骤（`docs/ledger/live-port-notes.md` §7.2）；状态：PENDING |
| 双 MCP 协同自测闭环（`MCP-DUAL-001` / `ROAD-M4-008`） | 部分｜UI 侧闭环**真跑**（`crates/yeban-app/tests/live_ui_mcp.rs`：`ui/tree` → `ui/node` → `ui/screenshot`，CI run 37233532606）；域侧那一半没有（`grep -rn "yeban_mcp\|yeban-mcp" crates/yeban-app/tests/` 命中 0） | 有｜同上（同一份用例） | 部分｜域侧十工具的端到端在 `crates/yeban-mcp/tests/tools_e2e.rs`，但**不与** UI 侧在同一用例里协同 | 原因：需要 app 依赖 `yeban-mcp`（同上一条）；计划：与上面那条同一提交落地；状态：PENDING |
| 错误码契约（21 值，`ADR-0001 D25` 联集） | 已实现｜`crates/yeban-mcp/src/tools.rs:121`（`ErrorCode::ALL`，21 项）+ `schemas/mcp-tools.schema.json` 的 `error.code.enum` | 有｜UI 控制面自有 JSON-RPC 码（`crates/yeban-ui-mcp/src/lib.rs` 的 `ELEMENT_NOT_FOUND` / `CAPTURE_FAILED` / `GEOMETRY_UNAVAILABLE`，落在 `-32000..=-32099` 服务自定义区） | 有｜`ToolResponse.error.code` | 原因：三方齐全，但 `ADR-0001 D25` 仍标 `Proposed`，等人类追认（`docs/ledger/mcp-core-notes.md` P4 / P10）；计划：负责人追认；状态：三方齐全 |
| `.yeban.lock` 跨形态互斥（进程内 HTTP 与独立 stdio 访问同一工程） | 部分｜`crates/yeban-mcp/src/domain/lock.rs` 的跨进程建议锁已有判据；但"进程内嵌 HTTP 与 stdio 互斥同一工程文件"这一**跨形态**端到端无判据（`docs/ledger/phase-status.md` §2 `ROAD-M0-007`） | 无｜无 | 部分｜`yeban_open_project`（`path`/`readOnly`）返回 `PROJECT_LOCKED` | 原因：域侧 HTTP 形态没嵌进 app（同上"进程内嵌入"那条）；计划：同一提交落地；状态：PENDING |
| MCP 冷启动 ≤ 20ms（形态 B 目标） | 部分｜启动路径已实现（`crates/yeban-mcp/src/bin/yeban-mcp.rs:217` 调 `serve_stdio`）；耗时**未测量**（`docs/ledger/mcp-core-notes.md` P2：没有 `iai-callgrind` 打点，也没有 CI 侧启动耗时判据） | 无｜未核查 —— 本次建表**没有**核对 `yeban-ui-mcp` 侧是否存在启动耗时判据 | 无｜未核查 —— 同上 | 原因：无打点设施、无 CI 判据；计划：需要固定硬件 + `criterion` / `iai-callgrind`（`HD-38` 的自托管 runner 预算未批）；状态：未核查 |

## 15. 分组 M —— 插件与外部协同

| 功能 | 系统（实现/计划 + 证据） | UI 暴露（有/无 + 载体） | MCP 暴露（有/无 + 工具·参数） | 缺口：原因 / 计划 / 状态 |
| :--- | :--- | :--- | :--- | :--- |
| CLAP / VST3 插件宿主与崩溃看门狗 | 无｜`crates/yeban-plugin-host/src/lib.rs` 是 12 行纯文档占位（`pub fn` / `pub struct` 命中 0） | 部分｜`crates/yeban-app/ui/console/device_rack.slint` 的 `device-plugin-host-placeholder-card`（占位卡，不是真宿主） | 无｜无工具 | 原因：`[v2.0.0]` 范围（`crates/yeban-plugin-host/src/lib.rs` 的 crate 文档自带 `[v2.0.0]` 标记）；计划：v2.0.0 插件宿主线；状态：未到期 |
| 外部软件联动（如 iZotope RX）与 `notify` 热重载（≤500ms） | 无｜`crates/yeban-services/src/lib.rs` 是 10 行纯文档占位（`pub fn` / `pub struct` 命中 0）；`ARCH-EXT-*` 无代码 | 无｜无 | 无｜无 | 原因：完全未实现；计划：`yeban-services` 线（`ARCH-EXT-*`）；状态：未到期 |
| MLS Ping 声学脉冲延迟校准 | 无｜`crates/yeban-services/src/lib.rs` 只有文档占位；`grep -rn -i "mls\|ping" docs/ledger/audio-latency-notes.md` 命中 0 | 无｜无 | 无｜无 | 原因：完全未实现（与 `BASELINE-005` 的"机器就绪、门禁仍 PENDING"是两件事）；计划：`yeban-services` 线；状态：未到期 |
| ASIO 可选特性与反向 VST3 打包（`yeban-vst`） | 无｜`grep -rn -i asio crates/*/Cargo.toml Cargo.toml` 命中 0（`docs/ledger/phase-status.md` §1 `ROAD-M-1-004`）；`crates/yeban-vst/src/lib.rs` 是 11 行占位且 `[dependencies]` 为空（`ROAD-M-1-003`） | 无｜无 | 无｜无 | 原因：法务确认未做（`HD-34`），且没有 `asio` 特性可标 `unverified`；反向打包属 `[v2.0.0]`；计划：人类法务签字 + v2.0.0；状态：人类决策中 |

## 16. 建表过程中发现的**具体错位**（本表最有价值的部分）

> 每条都是"当场可复核"的：命令写在证据里。这些不是"表没填好"，而是**三侧真的不一致**。

### 错位 1（**有能力、有判据、零调用者**，本表最高优先）：撤销 / 重做

- **功能**：撤销 / 重做 —— 已实现、已有性能判据，但**没有任何界面或协议能调用它**。
- **系统 = 有（完整且已判据覆盖）**：
  - 生产入口 `crates/yeban-model/src/commit.rs:505`（`CommitGraph::undo`）、`:521`（`undo_with`，游标由调用方保存）；
  - `commit.rs:533` 的 `op.apply_inverse(doc)?` 位于 `#[cfg(test)]`（`commit.rs:568`）**之前** ⇒ 是生产代码；
  - 撤销原语 `crates/yeban-model/src/ops.rs:112`、`:954`（`apply_inverse`），`ops.rs` 里逆操作相关共 **10** 处；
  - 时延判据 `BASELINE-004`：`docs/ledger/gate-status.md:44` 记 p99 **0.084 µs**（20,000 次/op，本机 M2 `--release`），`undo_batch2` 1.834 µs，目标 200 µs。
- **UI = 部分（只有"展示"，没有"操作"）**：
  - `crates/yeban-app/ui/dialogs/undo_tree_modal.slint` 的**唯一** callback 是 `close`（`grep -oE 'callback [a-z-]+'` ⇒ 只有 `close`）；
  - `crates/yeban-app/ui/app.slint:166,177,484,486` 只有 `undo-tree-open` 展示态与 `toggle-undo-tree`；
  - 元素已登记（`crates/yeban-app/src/elements.rs:818`（`undo-tree-modal`）、`:825`、`:833`、`:841`）；
  - 快捷键解析在 `crates/yeban-app/src/input.rs:369-370`（`Action::Undo` / `Redo`），派发在 `main.rs:146` 未接线。
- **MCP = 无**：`grep -cin 'undo\|redo' crates/yeban-mcp/src/tools.rs schemas/mcp-tools.schema.json` ⇒ **0**；`crates/yeban-ui-mcp/src/methods.rs` ⇒ **0**。
- **最硬的一条证据**：`grep -rn "UndoCursor" crates/` 只命中 `crates/yeban-model/src/commit.rs` 与 `crates/yeban-model/src/lib.rs`
  ⇒ **`yeban-model` 之外没有任何 crate 调 `undo` / `undo_with`**。
  （注：`crates/yeban-mcp/src/domain/mod.rs:2060` 与 `crates/yeban-mcp/src/domain/macros.rs:271` 的 `apply_inverse` 都落在
  `#[cfg(test)]`（`domain/mod.rs:1634`）之内；`crates/yeban-model/src/samples.rs:746` 与 `examples/bench_undo.rs` 也不是生产调用路径。）
- **影响**：这是最危险的一类错位 —— 从进度表、门禁表、控件树、覆盖率断言**四个视角看它都是"有"**：
  `BASELINE-004` 有实测数字、`undo-tree-modal` 在运行时树里、`ui/coverage` 全绿。
  但用户按 `Cmd+Z` 什么都不会发生，AI Agent 也没有任何方法撤销一次误操作
  （只能靠再发一次 `yeban_edit_notes` 反向补偿，那会**多留一条提交**，与"撤销"语义不同）。
- **建议处置**：新开一条线**同时**接两侧 —— UI 侧让 `undo_tree_modal` 的采纳动作真的调 `CommitGraph::undo_with`（游标存会话态），
  MCP 侧加工具或 `ui/*` 方法暴露同一入口（**两侧共用同一个 `undo_with`，不要各写一份**）。
  落地前，任何"撤销已完成"的措辞都应改成"模型侧可逆已完成，尚无调用入口"。

### 错位 2（**代码级**）：`yeban_propose_section` 仍在为一个**已经能表达**的能力上报 `unwired`

- **功能**：`yeban_propose_section` 的"章节配器骨架 + 声部连接"。
- **证据**：
  - 报告缺口的代码：`crates/yeban-mcp/src/domain/section.rs:8-16`（模块头断言"没有 `AddClip` / `RemoveClip` / `AddRoutingNode` / `RemoveRoutingNode` 变体"）与 `section.rs:109-110`（响应里报 `data.unwired = ["clipPoolEntries","routingEdges"]`）。
  - 反证：`crates/yeban-model/src/ops.rs:190`（`AddClip`）、`:197`（`RemoveClip`）、以及 `AddRoutingNode` / `RemoveRoutingNode`（29 个变体中的四个）**都已经存在**，落地提交 `4190651`（`git merge-base --is-ancestor 4190651 HEAD` ⇒ 真）。
  - 裁决依据：`docs/ledger/human-decisions.md:36`（`HD-12` = `ADR-0001 D27`，已裁决"追认并回写"，状态 ✅ 2026-10-04）。
  - 台账也停在旧结论：`docs/ledger/tools-domain-notes.md:55,231` 与 `docs/ledger/mcp-render-notes.md:325`（needs-4）。
- **影响**：① 规范 §7.2 要求的"声部连接"**至今没产出**，而阻塞理由已经不成立 —— 这是"假阻塞"；
  ② AI Agent 拿到 `unwired` 会以为模型表达力不足，转而去绕路（例如直接改工程文件），绕开 `Op` 日志的撤销语义；
  ③ 这张表若不点出来，下一轮还会有人按旧台账判断"做不了"。
- **建议处置**：`yeban-mcp` 线改 `domain/section.rs` 用 `Op::AddClip` + `Op::AddRoutingNode` 构造骨架与连接，
  删掉 `unwired` 上报与模块头旧假设；同一次提交回写 `tools-domain-notes.md` 的 `MCP-TOOL-005` 行、
  needs-1、boundary-1 与 `mcp-render-notes.md` 的 needs-4。**不由本线改 `crates/**`**（红线：一文件一写者）。

### 错位 3（**判据覆盖**）：十个 MCP 工具的"独立进程 + stdio"这条链没有端到端判据

- **功能**：形态 B（独立 Native CLI + stdio + `.yeban.lock`），`ROAD-M4-002` 标"已完成"。
- **证据**：`crates/yeban-mcp/Cargo.toml:15` 有 `[[bin]] name = "yeban-mcp"`；`crates/yeban-mcp/src/bin/yeban-mcp.rs:217` 真的调 `serve_stdio`；
  但 `grep -rn "CARGO_BIN_EXE_yeban-mcp" crates/` 命中 **0**。
  对照：同仓库的 app CLI **有**子进程级契约测试 —— `crates/yeban-app/tests/cli_contract.rs:35` 的 `env!("CARGO_BIN_EXE_yeban-app")`。
- **影响**：`ROAD-M4-002` 的"已完成"建立在"传输函数单测 + 域分发直调 + 锁"三件上，
  **"二进制真的能被 spawn 起来并说 stdio"** 不在 CI 覆盖内 ⇒ 打包/入口回归会静默溜过。
- **建议处置**：仿 `cli_contract.rs` 加一条 `tests/bin_contract.rs`（spawn `yeban-mcp`、喂一行 `tools/list`、断言 stdout 是合法 JSON-RPC）。

### 错位 4（**文档级**）：三份台账仍说"自动化曲线 / 宏 / 设备链都没进视图"，而自动化泳道**已经进视图了**

- **功能**：自动化泳道（投影 + 曲线 + 控件树可读）。
- **证据**：`crates/yeban-app/ui/workspace/arrangement_view.slint:53-72`（9 个平行数组由 `src/automation.rs` 单一事实源派生）、
  `crates/yeban-app/src/elements.rs:503`（`track-{i}-automation-{key}-lane`）、`crates/yeban-app/src/bridge.rs:526,678`。
  仍写旧结论的地方：`docs/ledger/app-binding-notes.md` §8 #5、`docs/ledger/app-completion-notes.md` §6 #7；
  `docs/ledger/app-automation-ui-notes.md` §9 needs-2 已经**请求**关掉这半句，但**没有回写**。
- **影响**：读台账的人会以为"自动化在 UI 侧完全没暴露"，从而重复规划已经做完的工作
  （正是本表要防的一类误判）。
- **建议处置**：集成者在下一轮把上述两处改成"自动化曲线已接；`macros` / `devices` 仍未进视图"，
  并保留"设备机架仍用演示常量"那半句。

### 错位 5（**三方覆盖缺口**）：MIDI 0/1 导出**系统有、UI 没有、MCP 没有** —— 三侧只有一侧知道它存在

> ✅ **已裁决并落地一半（2026-10-05）**：`ADR-0001 **D47**` 把 MIDI 导出的**唯一出口**定为 `yeban-app --export-midi`（离线批处理语义），**有意不扩** `render_master` 的参数面 —— `line/app-export-midi` 已交付（`export_midi.rs` 829 行 + CLI + B12/B13 真二进制判据，run **37254761445** = success）。⇒ 本节对 MIDI 的判断改为：**系统有出口、UI/MCP 有意不加**；`.als` 导出（`ROAD-M4-007`）仍 PENDING。

- **功能**：SMF 导出（`midly` 编码 + 自研 VLQ 回读，`ARCH-FMT-001 §5.5`）。
- **证据**：`crates/yeban-midi/src/midi.rs`（模块头逐条写明能力与边界）；
  `grep -rn "yeban_render::midi\|render::midi" crates/*/src crates/*/tests` 命中 **0** ⇒ **零消费者**；
  `grep -rn "render\|export\|bounce" crates/yeban-app/ui/` 无导出控件；十工具枚举里没有 MIDI 导出。
- **影响**：这是一个"实现了但谁也用不上"的能力 —— 不是 bug，但是**投入没有出口**；
  同时它是 `.als` 导出（`ROAD-M4-007`）的唯一前置能力，不接就等于前置能力也闲置。
- **建议处置**：要么在 `yeban_render_master` 的 `format` 白名单里加 `mid`（需人类裁决参数扩张），
  要么在 app CLI 加一个 `--export-midi <path>` 批处理入口（不触碰红线 6）。**不要**两边都各造一份。

### 错位 6（**三方覆盖缺口**）：`yeban-` 里两个**成品 crate 零消费者**

- **功能**：`yeban-theory`（乐理与流派规则引擎）、`yeban-sfz`（SFZ v2 采样器 + 声部池）。
- **证据**：`grep -rn "yeban-theory" crates/*/Cargo.toml Cargo.toml` 只命中自身与根 `[workspace.dependencies]` 的登记行；
  `grep -rn "yeban-sfz" crates/*/Cargo.toml Cargo.toml` 同样；
  `crates/yeban-mcp/src/domain/section.rs:45` 甚至明文"表的内容是可替换的：换成 `yeban-theory` / `yeban-services` 的预设库时……"
  —— 也就是说 MCP 的章节预设**自己写了一张 4 行常量表**，而 `yeban-theory/src/genre.rs` 存在却没被调用。
- **影响**：`yeban_propose_section` 的 `stylePreset` / `scale` 走的是 MCP 本地常量（`STYLE_PRESETS` 4 项、`MODES` 12 项），
  与 `yeban-theory` 的规则库**可能给出不同结果** —— 这是"第二份实现"的隐患，也是规范 §7.2 的语义稀释。
- **建议处置**：由 `yeban-mcp` 或 `yeban-engine` 线接 `yeban-theory`（`STYLE_PRESETS` → 规则库），
  并在接线时把 `docs/ledger/tools-domain-notes.md` 的 needs-6（"本地决策，表内容可替换"）标为关闭。

### 错位 7（**三档中的"部分"已成常态**）："系统有 + UI 有控件"但**回调一律未接线**

- **功能**：走带播放、撤销树、AI 提案采纳/拒绝、声学诊断（四类共 9 个回调）。
- **证据**：`crates/yeban-app/src/main.rs:141-151` 的 `wire_callbacks` 把 9 个回调全部指向 `trace()`，
  而 `trace()`（`main.rs:154-156`）只打印"`ui callback ... (未接线: 等待 yeban-engine / yeban-model)`"。
- **影响**：从**控件树与截图**看，界面上有按钮、有标签、有语义 ID，`ui/coverage` 也全绿；
  但点下去什么都不发生。⇒ 任何"以控件树存在为证据"的对齐断言都会把这一批判成"UI 已暴露"。
  本表因此对这批一律记 `部分`，并在"缺口"列点名 `main.rs` 的行号。
- **建议处置**：这**不是**缺陷（理由写在 `main.rs:135-140`，是为了不制造"UI 已经通了"的假象），
  但需要一次**专门**的接线切片；在此之前，任何"UI 暴露度"的统计都应把"控件存在"与"回调接线"分开计。

### 错位 8（**反方向**）：`ui/*` 与 `yeban_*` 的功能面**几乎不相交**

- **证据**：`crates/yeban-ui-mcp/src/methods.rs` 的 14 条方法里，与领域状态有关的只有 `ui/force_save`（存盘）与 `ui/switch_main_view`（视图）；
  其余 12 条全是"读 UI / 注入事件"。
  反方向：十工具里没有任何一条能读 UI（`crates/yeban-ui-mcp/Cargo.toml` 的依赖注释明文"依赖方向：`yeban-ui-mcp` → {`yeban-mcp`, `yeban-ui-test-port`}，**不**依赖 `yeban-app`（会成环）…… `yeban-model` 没有被引用 —— 本线不碰领域状态"）。
- **影响**：今天"双 MCP"实际是**两个互不相识的端点**：领域 MCP 不知道界面长什么样，UI 控制面不知道工程里有什么。
  `ROAD-M4-008`（双 MCP 协同闭环）的"AI 改模型 → 界面跟着变"这半条因此没有载体。
- **建议处置**：与错位 2 / `ROAD-M4-001` 同一件事 —— 需要 `yeban-app` 引入非默认 feature 的 `yeban-mcp` 依赖边，
  由集成者在 `ci.yml` 加步骤（`.github/**` 集成者独占）。

## 17. 本机 vs CI 的严格区分

- 本表**全部内容**是**本机**在 worktree `feature-alignment`（基线 `92a31c7`）上**读代码 + grep + 读台账**得到的静态事实，
  **没有任何一条**来自"我觉得"。
- 本机**跑过**的（可复跑）：
  - `python3 scripts/gates/check_feature_alignment.py`（本表自己的守卫，含 4 条注入 → 变红 → 还原记录，
    见 `scripts/gates/run-gates.sh` 的 `gate_docs` 与 `.github/workflows/ci.yml` 的 `checks` job）；
  - 表里引用的每一条 `grep` / `ls` 命令（分别写在对应行的证据列里）。
- 本机**没有跑**的（因此本表**不**声称）：任何 `cargo build` / `cargo test` / `cargo clippy`（本机纪律：
  含 Slint / cpal / symphonia 的重依赖一律交给 CI，`AGENTS.md` §5.1-5.2）。因此：
  - "**系统有**"的判据是**源码在**（文件、类型、字符串、测试名），**不是**"CI 上跑绿了"；
  - "**UI 有**"的判据是 `accessible-id` / 回调 / 元素注册表**在**，**不是**"真渲染出来过"；
  - "**MCP 有**"的判据是方法名 + 参数**在注册表/契约里**，**不是**"端到端调通过"。
- **只有 CI 的判决算数**（`docs/CI_CD.md` §3）。本表相关的 CI 判决由 `bash scripts/dev/ci-verdict.sh line/feature-alignment` 读回；
  **未读回之前一律记 `pending`**。

## 18. 本表的维护纪律

1. **改一行必须同时改 §1 的汇总计数** —— 否则 `check_feature_alignment.py` 立刻红（第 4 条判据）。
2. **不许发明名字**：表里出现的每一个 `yeban_*` 工具名与 `ui/*` 方法名，守卫都会去 `crates/` 里 grep；
   凭空发明的名字直接红（与 `scripts/gates/spec_id_audit.py` 的"不得发明 ID"同族）。
3. **不许漏名字**：`schemas/mcp-tools.schema.json` 的每一个工具、`crates/yeban-ui-mcp/src/methods.rs` 的每一条方法，
   **必须**在本表里作为一行或被某一行的 UI / MCP 列点名。
4. **"无 / 部分"必须写三件套**（`原因：…；计划：…；状态：…`）；
   优化"以后再说""待补充"这类空话会被守卫判为"没写清"。
