# ADR-0029 — 提案类工具新增 opt-in `autoMerge`：一次调用完成「提案 + 合并」

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`MCP-TOOL-006`（`yeban_edit_notes`）、
  `MCP-TOOL-001..010`（契约工具面）、`ARCH-OPS-002`（提交图谱：合并提交多父语义不变）、
  `crates/yeban-mcp/src/domain/mod.rs`（`propose_draft` / `Plan::Propose` / `apply_propose` / `apply_merge`）、
  `schemas/mcp-tools.schema.json`
- **日期**: 2026-10-27

## 背景

2026-10-27 的 MCP 创作链路审计（G13）全程计数：`edit_notes` / `propose_section` /
`set_macro` 永远写隔离提案，落盘必须再走 `yeban_list_proposals` +
`yeban_merge_proposal` —— 一次作曲约 **25 轮**纯往返税。工具面唯一的直接提交是
`yeban_edit_automation`（`appliedInOfflineRender: true`），证明"同一进程内直接施加"
在现有安全模型下是成立的。

同时 `plan_merge` 读代码实测：`commitMessage` **本来就是可选的**
（缺省 `"merge proposal {id}"`，`mod.rs` `plan_merge`），GAPS 里"必填报 -32602"
是 schema 层的 `required` 造成的表象——本 ADR 顺带把 schema 与代码口径对齐。

## 决定

### D1 形态：三个提案类工具加可选布尔 `autoMerge`（缺省 `false`）

`yeban_propose_section` / `yeban_edit_notes` / `yeban_set_macro` 接受
`"autoMerge": true`。缺省行为**逐字节不变**（不写该键的既有调用方零感知；
判据钉住"响应形状与今天相同"）。

**不做**的事：不新增第 25 个工具、不改 `yeban_merge_proposal` 的语义、
不引入"批量合并"（GAPS 要的是砍掉往返，不是砍掉提案机制本身——
提案隔离 + 多父合并提交是 `ARCH-OPS-002` 的既有契约）。

### D2 实现：复用同一施加链，⛔ 不写第二条合并路径

`propose_draft` 读 `arguments.autoMerge` 放进 `Plan::Propose { auto_merge }`；
`apply_inner` 的 Propose 臂先走 `apply_propose`（建分支、写提案提交、登记记录），
随后**以同一把锁内**从 `domain.proposals` 取回刚登记的 `Proposal` 记录，
调用**现有的** `apply_merge`（同一预演、同一 `commit_merge`、同一多父图谱语义）。
⇒ "autoMerge 合并的"与"你手动 merge 的"是同一份代码，不可能漂移。

### D3 回执：两段式，如实拼合

成功时响应 = 提案回执 ⊕ 合并回执：`pendingMerge: false`、新增
`autoMerged: true`、保留 `proposal` 投影并附上 `merge` 对象
（`mergeTool` 字段在 autoMerge 路径下改为 `"applied-in-process:autoMerge"`，
⛔ 不再教调用方去调 `yeban_merge_proposal`——那一步已经发生）。

### D4 失败语义：提案已建就是已建，⛔ 不假装回滚

若第二步合并失败（理论上仅当基线在 plan/apply 之间被宿主移动），
返回**结构化错误**且 `data` 里如实给 `proposalCreated: true` + `proposalId` +
失败原因 —— 提案保持 `Open`，调用方仍可手动 merge 或 reject。
不引入"补偿删除"：图谱与提案记录都是追加事实，删了反而造假。

### D5 `dryRun` 与 autoMerge 的交叉

`dryRun: true` + `autoMerge: true` ⇒ **不落盘、不合并**，预览里如实标
`wouldAutoMerge: true`。dryRun 的承诺（一位都不改）优先级更高。

### D6 commitMessage 口径对齐

三个工具的 `autoMerge` 路径接受可选 `mergeCommitMessage`（缺省同
`plan_merge` 的 `"merge proposal {id}"`）；`yeban_merge_proposal` schema 的
`commitMessage` 从 `required` 挪到可选（与代码缺省一致），留 `description`
建议显式书写。

## 后果

- 长曲构建从"每段 2~3 次调用"压到 1 次；提案审计轨迹（隔离分支 + 多父合并提交）
  **一条不少**，因为走的就是既有链路。
- 判据：`auto_merge_lands_in_one_call`（真 MCP 调用后 `query_project` 可见内容、
  `list_proposals` 状态为 Merged）、`absent_auto_merge_is_byte_identical_to_today`
  （缺省响应与既有判据形状一致）、`failed_auto_merge_keeps_proposal_open`
  （D4 的失败面）、dryRun 交叉面（D5）。
- schema/`tools.rs`/`SELECTORS` 无关；`mcp-tools.schema.json` 加三个可选键 +
  merge 工具 `commitMessage` 转可选（D6），contract 对账按既有机制走。
