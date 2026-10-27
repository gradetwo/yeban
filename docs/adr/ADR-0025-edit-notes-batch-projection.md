# ADR-0025 — `yeban_edit_notes` 的批内投影：路由建边可与建节点同一批（消除 G7）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-006`（`yeban_edit_notes` 契约）；`MODEL-AST-004`（`RoutingGraph`）；
  `ARCH-OPS-001`（领域操作日志：提案 = 有序 `Op` 批次）；
  `crates/yeban-mcp/src/domain/notes.rs`（`compile` 的批内投影）、
  `crates/yeban-mcp/src/domain/mod.rs`（`propose_draft` 的 `Op::Batch` 模拟，**未改**）；
  先例 `ADR-0018`（创建与摆放同批原子化——那是**单 op** 的两半，本票是**跨 op** 的顺序可见性）；
  缺口来源 `/tmp/yeban-issues-20261027` 审计 G7（2026-10-27，在 origin/main `36843ca9` 实测）
- **日期**: 2026-10-27

## 背景

审计 G7 实测：一次 `yeban_edit_notes` 的 `ops[]` 里放 `addTrack + addRoutingNode +
connectRouting` 三联，被 `ENTITY_NOT_FOUND`（`reason = "routingNodeNotFound"`）挡下——
`compile()` 的各 match 臂一律拿**批前**文档做预检，同批更早 op 即将新建的节点不可见。
而提案真正落地前要过的 `propose_draft` 模拟用的是 `Op::Batch`，它**本来就按顺序**把每条
op 施加到克隆工程上：也就是说**合并语义支持批内链式依赖，只有编译期预检不支持**。
代价是每给一条新轨接路由都多一整轮提案→合并往返（两轮），这与 `ADR-0018` 消除的
WALL ① 同型，只是发生在路由图。

## 决定

### D1 `compile()` 读**批内逐条投影**，判据仍在模拟

`compile()` 开头克隆一次文档；每编译出一条 `Op`，**最佳努力**把它施加到克隆上
（`let _ = compiled_op.apply(&mut project)`），后续 op 的存在性预检与撤销载荷都读这份
投影。于是：

- `addRoutingNode → connectRouting` 同批即可（节点算"已在 `nodes` 里"）；
- 撤销载荷取自**该条在序列中的当时值**，与合并期重放同一口径（此前"同批两条
  `setRoutingGain` 打同一条边"这类序列也会载荷错位）；
- 施加失败**不在编译期报错**：预检只是给 `reason` + `hint` 的友好层，真判据仍由
  `propose_draft` 的 `Op::Batch` 模拟按原契约码报（`from_model("提案模拟", …)`）。
  ⛔ 不新增错误面、不改任何既有错误码。

### D2 顺序仍然是语义

投影按 op 顺序推进 ⇒ **反序**（先 `connectRouting` 后 `addRoutingNode`）在编译期仍以
`routingNodeNotFound` 响亮拒绝（判据测试钉住），hint 更新为"放在同批 `ops[]` 的更早位置
也算"。

### D3 已知不彻底之处（登记，不扩大）

两条既有的**循环前**绑定不动：顶层音轨 `track` 与片段池 `entry`（`removeClip` 的
`previous_clip`、顶层轨 mute/solo 的旧值仍读批前状态）。它们是 `ADR-0018` 之前的旧
信封语义，改动会牵动"顶层 `trackId`/`clipId` 何时必填"一整族判据——与本票的
G7 痛点无关，**不在本票范围**。若将来要同批链式使用顶层信封，另立 ADR。

## 代价

- 每次 `compile()` 多一次工程克隆（`propose_draft` 本来也克隆，同量级）；
- 每条 op 多一次 `apply` 尝试（纯内存，控制线程；⛔ 不涉实时路径）。

## 判据测试

- `routing_chain_in_one_batch_sees_nodes_created_by_earlier_ops`（`notes.rs` 测试模块）：
  正序一批编译通过 + `Op::Batch` 施加后边真的进图 + 工程仍合法；
  注入① 投影退回批前校验 ⇒ 第一条断言红；注入② 投影无视顺序 ⇒ 反序断言红。

## 编号说明

`ADR-0024` 已被并行线 `line/sfz-load-loud`（G4）占用；本线取 `0025`。
后续并行线顺延（G6 预留 `0026`）。
