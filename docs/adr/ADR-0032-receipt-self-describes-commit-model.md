# ADR-0032 — 每条成功回执自述 `commitModel`：把「这次要不要再合并」变成读数而不是记忆

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-001..010`（契约工具面）、`MCP-TOOL-EXT`（扩展工具面）、`ARCH-OPS-001..002`
  （领域操作日志与提交图谱）、`ADR-0029`（`autoMerge`：本 ADR 的取值之一由它产生）、
  `crates/yeban-mcp/src/domain/mod.rs`（`enum Plan` / `apply_inner` / `propose_draft` / `apply_propose`）、
  `crates/yeban-mcp/src/output_schema.rs`（`COVERED_TOOLS` 五张响应 schema）
- **日期**: 2026-10-27
- **缺口**: GAPS `G17`（两种提交模型并存）

## 背景

2026-10-27 的 MCP 创作链路审计（`G17`，WALL ⑥ 现状）实测：同一次作曲里
`edit_notes` / `propose_section` / `set_macro` 走**提案 + 合并**，而
`yeban_edit_automation` 走**直接提交**（`appliedInOfflineRender: true`，无需合并）。
调用方必须**记住**哪个工具要合并 —— 审计报告原文："小。文档有标，但易错"。

先读清交付（⛔ 不凭词面）：

| 事实 | 实测位置 |
| :--- | :--- |
| 提案家族只有三个工具 | `domain/mod.rs:2361` 的 `propose_draft` 文档行"三个提案类工具共用的收尾"；调用方 `plan_propose_section:2410` / `plan_edit_notes:2548` / `plan_set_macro:2838` |
| 提案 vs 自动合并的判据是**一个 bool** | `Plan::Propose { auto_merge }`（`mod.rs:986`，`ADR-0029`），`apply_propose` 两条臂写 `pendingMerge:true` + `mergeTool:"yeban_merge_proposal"`（`:4124`）与 `pendingMerge:false` + `autoMerged:true`（`:4157`） |
| 所有真实回执只有一个出口 | `apply_inner`（`mod.rs:3566`）＝ 对**每个** `Plan` 变体的唯一 `match`；`dry_run_result`（`dispatch.rs:431`）是另一条、已有 `sideEffect` / `wouldChangeState` 自述 |
| 响应契约是**开集** | `schemas/mcp-tools.schema.json` 的 `definitions.ToolResponse.properties.data` 是 `{"type":"object"}`（`:341`），`additionalProperties:false` 只加在**入参** `arguments` 上 ⇒ 新增读数字段**不需要**改 schema 文件 |

⇒ 缺的不是能力，是**自述**：能力面已经区分得很清楚，但"这次调用属于哪一类"这件事
只存在于文档与调用方的记忆里。

## 决定

### D1 取值：四态，沿「提交」这一条轴，⛔ 不引入第二种提交模型

`commitModel ∈ {"proposal", "proposalAutoMerged", "direct", "none"}`，**每条成功回执**都带
（⛔ 不是"只有提案类工具带" —— 那正是本票要治的"要记住哪个有哪个没有"）：

| 值 | 含义（调用方可据以行动的一句话） | 产生的 `Plan` |
| :--- | :--- | :--- |
| `proposal` | 工程头没动，**必须**再调 `yeban_merge_proposal` 才落盘 | `Propose { auto_merge: false }` |
| `proposalAutoMerged` | 已当场合并，⛔ 不需要也不必再合并 | `Propose { auto_merge: true }` |
| `direct` | 本次调用直接施加/移动工程头，没有"下一步去合并" | `Merge` / `EditAutomation` / `Undo` / `Redo` / `ImportAudio` / `ImportMidi` / `LoadSfz` … |
| `none` | 本次调用不产生任何提交（只读、渲染到文件、会话/容器操作、探针） | `Query` / `EngineState` / `ListProposals` / `RenderMaster` / `RenderSfzWav` / `RecordWav` / `ExportMidi` / `Open` / `Save` / `Close` / `Reject` / `LaunchScene` / `MidiInputProbe` / `Diagnostics` |

两条边界口径，写进 D2 的映射表而**不**再造新值：

- `Plan::Merge`（`yeban_merge_proposal` 本身）⇒ **`direct`**。它就是"合并这一步"，
  回执要说的一句话是"工程当场变了、没有下一步"，而不是再回到 `proposal` 语义里。
- `none` 只说"没有提交"，⛔ 不说"没有副作用" —— 渲染落盘、启动场景、录音都写在
  工程之外，那一条轴由既有字段（`sideEffect` / `appliedInOfflineRender` / 文件路径）继续说。

**本票明确不做**的事（红线，`G17` 原文的诱惑就在这里）：

1. ⛔ 不把 `yeban_edit_automation` 改成提案模型，也不把提案家族改成直接提交 ——
   两种模型**并存是既有契约**（`ARCH-OPS-002` 的提案隔离 + 多父合并提交），
   审计要的是"别让我记"，不是"只留一种"。
2. ⛔ 不开新工具、不动 `required` 入参、不改任何既有键的含义。
3. ⛔ 不进 `reason` 词汇表：`commitModel` 是 `data` 的普通读数键，与故障词表面无关
   （`tests/fault_vocabulary.rs` 的冻结摘要只覆盖带 `"reason"` 的字面站点，本键不触发重钉 ——
   前提是⛔ 不要把它塞进任何含 `"reason"` 的 `json!`）。

### D2 实现：一处推导 ＋ 一处注入，⛔ 逐工具手抄

新增纯函数 `fn commit_model_of(plan: &Plan) -> &'static str`，对**每个** `Plan` 变体一条臂
（`match` 穷尽 ⇒ 以后新增变体**编不过**，这就是防漂移的那道机械防线，与 `ADR-0031` D1
"词汇表是唯一权威面"同一条思路）。注入点放在 `apply_inner` 的成功出口：
`Ok(response)` 时把 `commitModel` 写进 `response.data` 对象。

- ⛔ 不在二十来个 `apply_*` 里各写一遍（那会把"易错"从调用方搬进生产代码）；
- ⛔ 不在 `dry_run_result` 里另算一套（预览已经用 `wouldChangeState` / `sideEffect`
  说清了同一件事，且 `is_dry_run` 在 `dispatch.rs:365` 就短路了，压根不进 `apply_inner`）。

### D3 契约登记：五个已声明响应的工具同时把它写进 `required`

`output_schema.rs` 的 `COVERED_TOOLS`（`:58`，五个：`edit_notes` / `merge_proposal` /
`query_engine_state` / `render_master` / `undo`）各自 `properties` 加
`"commitModel": {"type": "string", "enum": [四条]}` **并**加进 `required`。

口径：`validate_complete`（`output_schema.rs:152`）只要求"**声明的每个字段都必须出现**"，
多余的键由 `validate` 放行 ⇒ 漏声明不会变红。因此本票把 D3 当作**契约义务**而不是
CI 兜底：一个每条回执都存在的键**必须**出现在 `required` 里，否则 schema 就不是
"从真实响应抽出来的"（该模块文档自己立的规矩）。

### D4 `ADR-0029` 的"零感知"就地收窄范围，并把这句话写进判据

`tests/auto_merge.rs` 判据 2（`default_shape_is_unchanged_without_auto_merge`）的字面断言是
"缺省回执里⛔ 不得出现 `autoMerged` 键 / `merge` 键" —— 本票新增的 `commitModel`
**不违反字面**，但违反它当时的口气（"不写该键的调用方逐字节零感知"）。

处理：不改断言的强度，**加一条**同向断言把新键钉成显式读数
（`data["commitModel"] == "proposal"`），并在文档表里把"零感知"改写为
"**`autoMerge` 的两条键形状**零感知"。这样"缺省路径没被 autoMerge 拖走"这条原意
仍然被逐字保护，而"回执多了一个自述键"变成**登记过的**契约变更，⛔ 不是漂移。

## 代价与影响面

- 每条成功回执多一个键 ⇒ 任何对 `data` 做**精确键集合**比较的判据都会红 ——
  这是预期的、且是本票要的读数（改判据而不是改口径）。已知候选：
  `extension_tools.rs`（预览键白名单，实测只覆盖 `preview`，预计不动）、
  `contract.rs`（断言 `inputSchema`，预计不动）、`tools_e2e.rs`（断言**工程字节**，预计不动）。
  ⚠ 逐条以 CI 判决为准，⛔ 不在此预先声称"全绿"。
- `schemas/mcp-tools.schema.json` **零改动**（D1 表末行实测依据）。
- 本票与 `ADR-0030`（分轨导出）都落在 `domain/mod.rs` ⇒ **串行**：本片的实现等在
  `line/stem-export` 落地之后，⛔ 两线不同时写同一个文件。

## 判据（落地时随代码一起提交）

1. `commit_model_is_self_described_per_category`：六次真实调用（提案缺省 / 提案 `autoMerge` /
   `edit_automation` / `merge_proposal` / `query` / `undo`）各自的 `data.commitModel`
   等于 D1 表里的**逐字**取值；`proposal` 那条同时断言 `pendingMerge:true`（两把钥匙不说反话）。
2. `commit_model_never_promises_a_merge_step_that_is_not_needed`：`direct` / `none` 两类回执里
   ⛔ 不得出现 `mergeTool`（把"要记的规矩"变成断言，正是本票的交付）。
3. 映射穷尽性由 `match` 的编译期保证；再加一条 `commit_model_values_are_exactly_four`
   钉住取值集合（改成第五条必须**故意**改判据，与 `tools.rs` 的 `TOOL_COUNT` 同一口径）。
4. `output_schema` 既有双向判据（`:696` / `:721`）自动要求新字段带 `type`；
   `props_176` 的 `validate_complete` 自动要求已声明即出现。

## 落地补记（写代码时实测到的三处偏离，⛔ 不是事后修饰）

1. **D2 的注入点做成 wrapper，不改函数名**：`apply_inner(domain, plan)` 保留原名
   （`ADR-0029` 与 `docs/ledger/m4-008-authority-notes.md` 都按这个名字留痕），本体
   改名为 `apply_plan_model`，`apply_inner` 只剩两行 —— 先 `commit_model_of(&plan)`
   （必须在 `plan` 被 move 之前），再 `.map(|response| response.with_commit_model(model))`。
2. **判据 2 有一条实测例外**：`yeban_list_proposals` 的清单**顶层本来就带**
   `"mergeTool": "yeban_merge_proposal"`（`domain/mod.rs:3188`，`#176` 的既有契约），
   它说的是"清单里那些提案用哪个工具去合"，⛔ 不是"本次调用属于提案模型"。
   按 D1"⛔ 不改任何既有键的含义"，判据**不**把它算作违规，改为另钉
   `list.commitModel == "none"` + `list.mergeTool` 保持原值 —— 两个同名字段各说各的。
3. **D3 的声明方式改成 `finish` 外统一挂一层**（`output_schema::with_commit_model`）：
   `render_master` 的 `json!` 字面量早已贴着 `serde_json` 宏的递归上限（该函数里
   `midiSource` 的来历就是这条），往五份字面量里各加一个键有一处编不过；同时
   取值集合直接引用 `domain::COMMIT_MODEL_VALUES`，⛔ 不留"schema 与实现两份清单"。
4. **预期代价之一被量化**：`tests/payload_budget.rs` 钉的 `[BASELINE-006]` 缺省往返
   响应读数 2,816 B ⇒ 形状上恰好 **+25 B**（`"commitModel"` 13 + `:` 1 + `"proposal"` 10
   + 逗号 1；`serde_json::Map` 有序 ⇒ 插在哪对键之间都只需一个逗号）⇒ 期望值改为
   **2,841 B**（合计 3,037 ≤ 4,096 的预算不变）。⚠ 以 CI 报出的读数为准，不符就改这里。
