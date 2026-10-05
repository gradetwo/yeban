# op-origin-mcp —— 给 MCP **直接编辑**一个如实的作者标签（`OpOrigin::McpEdit`）

> 工作线：`line/op-origin-mcp`（worktree `.worktrees/op-origin-mcp`，基线 `ddd637c`）。
> 上游 need 来源：`line/mcp-tools-expansion`（`ADR-0001` **D46**）留下的 needs-1
> —— 「`OpOrigin` 缺 `McpEdit`：MCP 的直接编辑借用了 `AutomationRecord` / `Import` 作为作者（来源标签不准）」。
> 本文件由本线维护；`docs/DEVELOPMENT_LEDGER.md` 与 `docs/ledger/feature-alignment.md` **不归本线**。

---

## 1. 结论（一段话）

模型 `OpOrigin` 新增 **`McpEdit { agent_name: String }`**，MCP 的两个**直接编辑**工具
（`yeban_edit_automation` / `yeban_import_audio`）的提交来源从借来的
`AutomationRecord` / `Import` 改成它；`agent_name` 取既有的 `domain::AGENT_NAME`
（即 `"yeban-mcp"`，与 `UndoState.author` 同源）。除"来源标签更准"外**零语义变化**：
`Op` 的施加/逆操作、撤销重做、`dryRun`、幂等、错误码全部一字未改。
第三个 D46 工具 `yeban_query_engine_state` 是**只读**的，**没有** `origin` 站点（§2.3 复核）。

`OpOrigin` **参与 serde 且随 `StampedOp` 进 `CommitGraph` → `history.dag`**（持久化面），
所以新增变体**是契约变更**；而 `schemas/**` 本线禁改 ⇒ 用**显式欠账清单**
`PENDING_CONTRACT_ORIGINS` 机械钉住漂移，契约侧请求见 §7 的 needs-1。

---

## 2. 受影响调用点清单（全 40 处匹配点，逐一分类）

改动前 `grep -rn 'OpOrigin::' crates/ | wc -l` = **40**（本线复核过）：

| 文件 | 命中数 | 是否受影响 | 说明 |
| :--- | :--- | :--- | :--- |
| `crates/yeban-model/src/ops.rs` | 15 | **受影响** | 枚举定义、`StampedOp::user_ui`、origin 的 3 条判据（本线扩到 5 条） |
| `crates/yeban-model/src/samples.rs` | 8 | 否 | 样本导出仍用 `UserUi` / `McpProposal`，字节不变（§4.2 实测） |
| `crates/yeban-mcp/src/undo_session.rs` | 6 | 否 | `UserUi` 站点（测试夹具）+ 一条文档注释 |
| `crates/yeban-mcp/src/domain/mod.rs` | 3 | 否 | `McpProposal`（提案/合并）+ 代理名文档 |
| `crates/yeban-mcp/src/domain/automation.rs` | 2 | **受影响（借用点 1）** | 提交来源 + 响应来源块的注释 |
| `crates/yeban-app/src/undo.rs` | 2 | 否 | UI 侧 `UserUi` |
| `crates/yeban-model/src/commit.rs` | 1 | 否 | `commit_graph_serde_round_trip` 夹具用 `UserUi` |
| `crates/yeban-mcp/tests/undo_wiring.rs` | 1 | 否 | 测试夹具用 `UserUi` |
| `crates/yeban-mcp/src/domain/proposal.rs` | 1 | 否 | 提案详情用 `McpProposal` |
| `crates/yeban-mcp/src/domain/import_audio.rs` | 1 | **受影响（借用点 2）** | 提交来源 |

### 2.1 借用点 1：`yeban_edit_automation`（`crates/yeban-mcp/src/domain/automation.rs`）

| | 改动前 | 改动后 |
| :--- | :--- | :--- |
| `CommitRequest.origin` | `OpOrigin::AutomationRecord`（第 723 行） | `OpOrigin::McpEdit { agent_name: super::AGENT_NAME.to_owned() }`（第 726 行） |
| 响应 `data.origin.kind` | `"AutomationRecord"` | `"McpEdit"` |
| 响应 `data.origin.note` | 「模型 OpOrigin 没有 `McpEdit` 变体 (needs-1); 借用最接近的 AutomationRecord」 | 「MCP 直接编辑 (不创建提案) ⇒ OpOrigin::McpEdit; 契约 origin.oneOf 尚未承认该分支 (needs)」 |
| 响应键集合 | `{kind, author, note}` | **逐键相同**（只改值，不增删键；`dryRun` 与真做共用同一段代码） |
| 模块文档 | 「操作来源（`OpOrigin`）的**诚实缺口**」 | 「操作来源（`OpOrigin`）已**如实**（不再是借用）」 |

### 2.2 借用点 2：`yeban_import_audio`（`crates/yeban-mcp/src/domain/import_audio.rs`）

| | 改动前 | 改动后 |
| :--- | :--- | :--- |
| `CommitRequest.origin` | `OpOrigin::Import`（第 496 行） | `OpOrigin::McpEdit { agent_name: super::AGENT_NAME.to_owned() }`（第 506 行） |
| 响应体 | 无 `origin` 块 | **无 `origin` 块**（不新增字段 ⇒ 返回语义不变） |
| 模块文档 | 只说落点是 `Op::AddClip` | 新增「操作来源：`OpOrigin::McpEdit`（不再借 `Import`）」小节 |

> 刻意的**不对称**：只有 `automation.rs` 的响应里本来就有 `origin` 块。
> 给 `import_audio` 补一个 `origin` 块会**改变返回语义**（任务书禁改），所以不做 ——
> 它的作者标签只落在**落盘日志**里，由 §5 的判据 ① 在 `history.dag` 上观测。

### 2.3 第三个 D46 工具复核：`yeban_query_engine_state` **没有** `origin` 站点

```text
$ grep -rn 'CommitRequest\|OpOrigin::' crates/yeban-mcp/src/domain/engine_state.rs
（零命中；该模块只有只读读数与 `Domain::set_engine_readings` 镜像）
```

它的 `sideEffect` 是 `read-only`（`tools::TOOLS` 的注册表声明），`plan` 只拿 `&Domain`
（`extension_audit::scan_dry_run_entry_points` 的签名清单里就有它）。
⇒ 三个 D46 工具里**只有两个**有作者标签可修，"三处借来源"这个说法要更正为**两处**。

### 2.4 改完后的 grep 证据（不得残留借用）

```text
$ grep -rn 'origin: OpOrigin::\(AutomationRecord\|Import\)' crates/
（只剩两条**测试夹具**命中；生产区零命中 —— 生产区再无'直接编辑借别的档'的提交站点：
 crates/yeban-mcp/tests/extension_tools.rs:1293  origin: OpOrigin::AutomationRecord,   ← 守卫的反向注入夹具
 crates/yeban-mcp/tests/extension_tools.rs:1299  origin: OpOrigin::Import,             ← 同上
 crates/yeban-mcp/src/domain/extension_audit.rs:525 origin: OpOrigin::AutomationRecord, ← 单测注入夹具）

$ grep -rn 'origin: OpOrigin::' crates/yeban-mcp/src crates/yeban-app/src
crates/yeban-mcp/src/undo_session.rs:1135  origin: OpOrigin::UserUi,        （测试夹具）
crates/yeban-mcp/src/undo_session.rs:1201  origin: OpOrigin::UserUi,        （测试夹具）
crates/yeban-mcp/src/undo_session.rs:1312  origin: OpOrigin::UserUi,        （测试夹具）
crates/yeban-mcp/src/undo_session.rs:1422  origin: OpOrigin::UserUi,        （测试夹具）
crates/yeban-mcp/src/undo_session.rs:1548  origin: OpOrigin::UserUi,        （测试夹具）
crates/yeban-mcp/src/domain/automation.rs:726  origin: OpOrigin::McpEdit {   ← 借用点 1（已改）
crates/yeban-mcp/src/domain/mod.rs:2111        origin: OpOrigin::McpProposal { ← 提案合并（本来就对）
crates/yeban-mcp/src/domain/import_audio.rs:506 origin: OpOrigin::McpEdit {  ← 借用点 2（已改）
crates/yeban-app/src/undo.rs:264               origin: yeban_model::OpOrigin::UserUi, （UI 侧）
```

`crates/yeban-mcp/src/domain/extension_audit.rs` 里剩下的
`OpOrigin::AutomationRecord` / `OpOrigin::Import` 字面量**不是提交站点**，
而是守卫常量（`BORROWED_ORIGIN_NEEDLES`）与两条**注入夹具**（守卫的"反向证明"）。
机器上由守卫 ④ 区分：它只看每个文件**生产区**里 `origin:` 的右值 + 注释行以外的借用字面量。

---

## 3. serde / 持久化路径证据（为什么这是**契约**变更）

| 环节 | 事实 | 位置 |
| :--- | :--- | :--- |
| 类型 | `OpOrigin` 派生 `Serialize, Deserialize`，外部标签（unit = 纯字符串，带载荷 = `{"Tag":{...}}`） | [`crates/yeban-model/src/ops.rs`](../../crates/yeban-model/src/ops.rs) 第 48 行起 |
| 信封 | `StampedOp { origin, timestamp, op }` 同样派生序列化 | 同文件 `StampedOp` |
| 提交 | `Commit.ops: Vec<StampedOp>` | `crates/yeban-model/src/commit.rs` |
| 图谱 | `CommitGraph { commits, branches, depths }` 派生序列化 | 同文件 |
| 落盘 | `history.dag = serde_json::to_vec(graph)`，写进 `.yeban` 容器 | `crates/yeban-mcp/src/domain/store.rs`（`container_bytes`） |
| 读回 | `read_project_container(...).history_dag` → `CommitGraph` | `crates/yeban-model/src/container/mod.rs` |

⇒ 新增变体会**出现在持久化字节里**（`history.dag`）：旧读者（不认识 `McpEdit`）
反序列化会失败。这是**向后不兼容的读取边界**，必须由契约承认 + 由人类评估兼容策略。
本线**没有**自行决定任何兼容策略（既不 `#[serde(alias)]`，也不降级），只如实上报。

---

## 4. 契约影响结论

### 4.1 `schemas/ops.schema.json` 的 `origin.oneOf` 现状（原文）

```json
"origin": {
  "oneOf": [
    { "type": "string", "enum": ["UserUi", "MidiInput", "UndoRedo", "AutomationRecord", "Import", "Migration"] },
    { "type": "object", "required": ["McpProposal"], "additionalProperties": false,
      "properties": { "McpProposal": { "type": "object", "required": ["proposal_id", "agent_name"], "additionalProperties": false, ... } } }
  ]
}
```

`McpEdit` 的线上形态是 `{"McpEdit":{"agent_name":"yeban-mcp"}}` ⇒ 被
`additionalProperties: false` 与"只认 `McpProposal`"两条同时**拒绝**。
**这是实打实的契约漂移**，不是"多一个更宽松的分支"。

### 4.2 处置：不改 `schemas/**`，用**显式欠账清单**把漂移钉死

`schemas/**` 由契约线独占（任务书禁改）⇒ 本线只改枚举，并在
`crates/yeban-model/src/ops.rs` 的测试模块里登记：

```rust
const PENDING_CONTRACT_ORIGINS: [&str; 1] = ["McpEdit"];
```

判据 `origin_variants_match_ops_schema_origin_one_of` 断言
「枚举里的**对象标签** − 契约里的**对象标签**」**恰好等于**这份清单，并且：

| 情形 | 结果 |
| :--- | :--- |
| 契约补上 `McpEdit` 分支 | 差集变空 ≠ 清单 ⇒ **红**，报文指名「请把 PENDING_CONTRACT_ORIGINS 清空」 |
| 枚举再多个对象标签而没登记 | 差集 ≠ 清单 ⇒ 红 |
| 契约多出一个枚举没有的对象分支 | 反向差集非空 ⇒ 红 |
| 单元变体集合（6 个）与契约 enum | **原判据原样保留**（逐字比对） |

同一族的先例是 `Op` 的 `PENDING_CONTRACT_OPS`（`Op` 29 个变体那条棘轮）。

### 4.3 「新增变体不改变既有字节」的**实测**（不是推理）

用**非测试**入口导出四份规范样本，改动前后逐字节比对：

```text
$ bash scripts/dev/cargo-local.sh run -p yeban-model --example export_schema_samples -- --out /tmp/op-origin-before
$ bash scripts/dev/cargo-local.sh run -p yeban-model --example export_schema_samples -- --out /tmp/op-origin-after
$ diff <(shasum -a 256 /tmp/op-origin-before/*.json | sed 's#before#X#') \
       <(shasum -a 256 /tmp/op-origin-after/*.json  | sed 's#after#X#')
（无差异 ⇒ SAMPLES_BYTE_IDENTICAL）
```

| 样本 | 改动前 = 改动后（sha256） |
| :--- | :--- |
| `ops.default.json` | `d331a363621468c9135216ba00332e8ddc40f380b0a00360840ed07e55d14069` |
| `ops.filled.json` | `9dcf3e5c98070062c6ede3613df4b40cd00ec88579fa794041d2b2cb00b82a4d` |
| `project.default.json` | `4cf38f55e27d9ced3a011b4065d2113f211809003b4b312e677fc15e0e7bf725` |
| `project.filled.json` | `960f03a2e0eda1073d9b9bb05fcc787ff24b0e76ce3a487d087e31f61198b486` |

机器侧的钉子：`origin_wire_shape_is_frozen_byte_for_byte` 把**每个**变体的线上字符串
逐字节冻住（含 `McpProposal` 的双键载荷），并用**源码扫描**证明冻结表覆盖枚举全集
（见 §5 判据 ②/④）。

---

## 5. 判据与注入

### 5.1 判据（①..⑥，本机真跑的那一半标 ✅）

| # | 判据 | 用例 | 本机 |
| :--- | :--- | :--- | :--- |
| ① | MCP 直接编辑产生的 `Op` 作者是**新变体**（落盘 `history.dag`，且**有见证**） | `crates/yeban-mcp/tests/extension_tools.rs::direct_edits_are_authored_by_the_mcp_edit_origin_in_the_persisted_log` | ❌ CI（`yeban-mcp` 含重依赖） |
| ① | 同样的性质在**源码文本**上判定（含反向注入） | 同文件 `direct_edit_origins_in_production_sources_are_the_mcp_edit_variant`；纯函数 `extension_audit::scan_direct_edit_origins` | ✅ 裸 `rustc` 脚手架（§5.2） |
| ② | 既有 7 个变体的**语义逐项不变**（一条覆盖全体：线上字节冻结表） | `yeban-model::ops::tests::origin_wire_shape_is_frozen_byte_for_byte` | ✅ |
| ② | 既有单元变体仍与契约 enum 一一对应；`McpProposal` 载荷键不变 | `origin_variants_match_ops_schema_origin_one_of`、`samples::every_unit_origin_variant_is_in_the_contract_enum`、`samples::ops_origin_shapes_match_the_origin_one_of` | ✅ |
| ③ | 逆向/撤销路径不因新变体失配（来源不参与 op 语义） | `yeban-model::ops::tests::the_new_origin_variant_never_changes_an_op_or_its_inverse`；MCP 侧既有 `automation_write_lands_in_the_project_and_undo_restores_byte_for_byte`（CI） | ✅ / CI |
| ④ | 序列化往返 + **逐字节稳定** | `origin_variants_round_trip`（覆盖全集）、`origin_wire_shape_is_frozen_byte_for_byte`、§4.3 的样本 sha256 实测 | ✅ |
| ⑤ | 来源 → 显示/审计映射：新变体必须有**明确**显示 | `automation.rs` 的响应 `data.origin{kind:"McpEdit",author:"yeban-mcp",note}`；判据在 ① 的落盘用例里同时断言 `kind`/`author`/`assert_ne!(kind,"AutomationRecord")` | CI |
| ⑥ | 门禁 | `bash scripts/gates/run-gates.sh light` + `crate yeban-model`（本机）；CI 判决 | ✅ / CI |

**穷尽性保护（判据 ③ 的"编译器要不要我补分支"那一问）—— 如实回答：没有。**
`OpOrigin` 目前**没有任何穷举 `match`**（全仓库 40 处匹配点里没有一处对 `origin` 做
`match`；它只被 serde 派生消费，撤销路径只看 `op` 本体）。因此新增变体**不会**逼编译器
报"缺分支"。补偿措施有两条，都写进了判据：

1. **模型侧**：`the_new_origin_variant_never_changes_an_op_or_its_inverse` 直接证明
   "换来源标签不改变施加/逆操作"（正面证据，而不是"编译过了所以没事"）；
2. **枚举全集棘轮**：`origin_wire_shape_is_frozen_byte_for_byte` 用**源码扫描**
   （`declared_origin_variant_names`，取 `depth == 1` 的标识符）证明冻结表覆盖枚举全集
   —— 谁再加一个变体而没补冻结行，立刻红。这是 Rust 无反射下能拿到"全集"的唯一办法，
   与 `Op::name()` 那条穷举 `match` 同族（`every_op_variant_is_declared_in_the_contract`）。

### 5.2 注入实验（红 → 还原 → 绿）

| # | 注入 | 期望与实测 |
| :--- | :--- | :--- |
| I1 | 把 `automation.rs` 的 `origin` 退回 `OpOrigin::AutomationRecord`（真实源码上跑守卫 ④） | **红**（原文：`automation.rs:726 直接编辑的来源标签必须是 OpOrigin::McpEdit, 实际是 OpOrigin::AutomationRecord,` + `仍在借 OpOrigin::AutomationRecord` + 缺 `agent_name: super::AGENT_NAME`）⇒ 还原 ⇒ `CLEAN` |
| I2 | 给既有变体 `UndoRedo` 加 `#[serde(rename = "UndoRedoRenamedByInjection")]` | **红**（原文：`assertion left == right failed: UndoRedo 的线上字节变了 —— 既有来源变体的语义不许被顺手改动`）⇒ 还原 ⇒ 7 条 origin 判据全绿 |
| I3 | 守卫 ④ 的**内置**注入夹具（`OpOrigin::AutomationRecord` / `OpOrigin::Import` 两条） | 在 `extension_audit.rs` 的单测里断言"必须抓住" ⇒ 证明守卫不空转 |
| I4 | （CI 侧，未在本机跑）把 MCP 写路径退回旧变体 | 由 §5.1 判据 ① 的**见证**抓住：先证明日志里有 `SetAutomationPoint` / `AddClip` 本体，再断作者 ⇒ 不可能在空集合上变绿 |

I1/I2 的本机命令：

```text
$ source scripts/dev/local-env.sh
$ rustc --edition 2024 -D warnings /tmp/guard_probe.rs -o /tmp/guard_probe   # 脚手架上 #[path] 引入真实 extension_audit.rs
$ /tmp/guard_probe "$PWD"           # CLEAN
$ rustc --edition 2024 --test -D warnings crates/yeban-mcp/src/domain/extension_audit.rs -o /tmp/ea_test && /tmp/ea_test
running 9 tests ... test result: ok. 9 passed; 0 failed
```

---

## 6. 本机 vs CI（**严格区分**）

| 项目 | 本机（M2） | CI |
| :--- | :--- | :--- |
| `bash scripts/gates/run-gates.sh light` | ✅ 改动前后均 `exit=0` | ✅ |
| `bash scripts/gates/run-gates.sh crate yeban-model` | ✅ `exit=0`（107 → **109** 条判据；origin 过滤 5 → **7**） | ✅ |
| 样本四份 sha256 前后逐字节相同 | ✅ **真跑过**（§4.3） | ✅（`validate_schemas.py` 另作对账） |
| 裸 `rustc --test` 跑 `extension_audit.rs`（含守卫 ④ 的 2 条新判据） | ✅ **真跑过**（9 条） | ✅（`tests/extension_tools.rs` 在真源码上再跑一遍） |
| 注入 I1 / I2 → 红 → 还原 | ✅ **真跑过** | —（注入是实验，不进仓库） |
| `cargo test -p yeban-mcp`（含 §5.1 判据 ① 的**落盘日志见证**） | ❌ **本机不做**（`heavy-deps.py` 判它含 `rayon`/`symphonia`；`run-gates.sh crate yeban-mcp` 会 SKIP 并指向 CI） | ✅ 判决唯一来源 |

⇒ 运行期那几条（尤其落盘日志的见证）**只有 CI 判决算数**；本机的绿只覆盖
"格式 + 机械守卫 + 零依赖纯逻辑 + 文本级守卫 + 模型判据 + 样本字节"。

---

## 7. needs（交集成者 / 人类裁决，本线**不**自己开）

1. **契约需要一个 `McpEdit` 分支**。请求原文（`schemas/ops.schema.json` 的 `origin.oneOf`）：

   > 给 `origin.oneOf` 增加第三个分支：
   > `{"type":"object","required":["McpEdit"],"additionalProperties":false,`
   > `"properties":{"McpEdit":{"type":"object","required":["agent_name"],`
   > `"additionalProperties":false,"properties":{"agent_name":{"type":"string"}}}}}`。
   > 同时必须**放宽**原对象分支的 `additionalProperties: false` 语义（`oneOf` 各分支互斥，
   > 加第三个分支即可；**不要**给第二个分支加 `McpEdit` 属性）。

   集成者补完后，把 `crates/yeban-model/src/ops.rs` 的 `PENDING_CONTRACT_ORIGINS`
   清成空数组（判据会**主动报红**提醒，不会静默腐烂）。

2. **兼容策略由人类裁决**（本线刻意没做）：`history.dag` 是可持久化面，旧版本读者遇到
   `{"McpEdit":...}` 会**反序列化失败**。可选项（都不在本线权限内）：
   ① 接受破坏性变更（当前状态，计划内的 schema 演进）；② 在**下一次**版本闸门
   （`schema_version` / `VersionGate`）里一并升版；③ 旧读者侧加 `#[serde(other)]` 降级
   ——**没有**人类裁决前本线不实现任何一条。

3. **规范正文同步**：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §6.1 的
   `OpOrigin` 代码块只列 7 个变体（本线**禁改** `docs/YEBAN_*.md`）。
   ⇒ 需要集成者把 `McpEdit { agent_name: String }` 补进那段 Normative 代码块，
   否则规范与实现漂移（本文件的表就是给那次同步用的对照）。
