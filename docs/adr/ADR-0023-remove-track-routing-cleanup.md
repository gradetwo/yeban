# ADR-0023 — 取走音轨连带清理路由图（`removeTrack` 的孤儿节点与悬空边）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-006`（`yeban_edit_notes`）；`MODEL-AST-004`（`RoutingGraph` 是
  声学连接的唯一真理源）；`crates/yeban-mcp/src/domain/notes.rs`
  （`routing_cleanup_ops`）、`crates/yeban-mcp/src/domain/mod.rs`
  （`plan_edit_notes`）；`Op::DisconnectRouting` / `Op::RemoveRoutingNode` /
  `Op::RemoveTrack` / `Op::Batch`；台账缺口 **G2**
  （`/tmp/yeban-issues-20261027`：删轨后 `render_master` 整体失败）
- **日期**: 2026-10-27

## 背景

模型层的 `Op::RemoveTrack` 只摘音轨本体（`YebanProjectV1::remove_track` 只从
`tracks` 删除）。`RoutingGraph` 刻意不校验"节点身份必须对应一条存在的音轨"
（拓扑自洽与全局可达性不是 `RoutingGraph::validate` 的职责，见
`routing_graph_validate_is_silent_about_topology_and_global_reachability`），
于是删轨之后的文档**仍然合法**，孤儿节点与悬空边留在图里。

实测（G2 工单，origin/main `36843ca9`）：删 6 条轨后 `routing_graph` 节点/边
**原封不动**（6 个孤儿节点 + 6 条悬空边），`yeban_render_master` 整体失败：

```json
{"code": "RENDER_FAILED",
 "message": "路由图里的源节点没有对应的音轨，无法知道该渲染什么",
 "data": {"node": "01M5X5GN4M3WAWPSFBFS1KKTZ3"}}
```

修复负担落在调用方：每条轨要 `disconnectRouting` → `removeRoutingNode` 两步
（6 条轨 = 12 轮提案合并），而"取走音轨"本来就是一个动作。

## 决定

### D1 由**工具层**承担清理，模型语义不动

`Op::RemoveTrack` 仍然是"精确删掉这一条实体"，**不引入级联删除**：模型层的每个
`Op` 都是显式、可逆、单义的；把路由副作用藏进 `RemoveTrack::apply` 会让
`apply_inverse` 与 `validate` 的因果链变成隐式的，也会改变既有工程的字节等价性
（任何直接构造 `RemoveTrack` 的调用方都会突然多删东西）。

清理在 `yeban-mcp` 的 `edit_notes` 编译之后、提案创建之前完成：
`notes::routing_cleanup_ops(project, track_id)` 从**当前文档**读出该轨在
`routing_graph` 里的边与节点，返回 `Vec<Op>`。

### D2 顺序承重：断边 → 移除节点 → 取走音轨

`Op::RemoveRoutingNode` 的前置条件要求**没有任何边引用该节点**
（`ModelError::RoutingNodeInUse`），所以引用该轨的边必须先断。`plan_edit_notes`
把清理 op **插在** `RemoveTrack` 之前：

1. `DisconnectRouting { edge_id, previous_edge }` —— 端点命中该轨的每条边
   （`source_node` **或** `destination_node`，两个方向都算）；
2. `RemoveRoutingNode { node }` —— 该轨在图里的节点（不在图里则跳过）；
3. `RemoveTrack { track_id, previous_track }` —— 编译层照旧产出的一条。

### D3 一次提交、一次撤销

`propose_draft` 用 `Op::Batch` 模拟整批；合并时 `merge_batch` 提交**恰好一条**
`Op::Batch`（`Batch::apply` 顺序施加，`invert` 反向逐条反转）⇒ 一次
`yeban_undo` 把边、节点、音轨复原（`DisconnectRouting` 自带
`previous_edge`，`AddRoutingNode` 只用身份，`RemoveTrack` 自带整条音轨）。

**逐字节**复原的前提是 `routing_graph.nodes` 按字典序（模型的既有纪律：
`AddRoutingNode` 用 `partition_point` 二分插入，判据
`routing_node_ops_keep_the_node_list_sorted` 钉住，而 `RoutingGraph::validate`
**不**查有序）。演示工程与工具面新建的工程满足该前提 ⇒ `tools_e2e.rs` 的
`yeban_undo` 断言是逐字节比较；`filled_project` 把主总线放在 `nodes` 末尾
（历史夹具，不满足有序），因此那一处的域内判据比**内容**（节点集与边表逐条相同），
不冒充逐字节。

### D4 不在图里的音轨 ⇒ 清理为空

`routing_cleanup_ops` 在该轨既无边也无节点时返回空 `Vec`：`removeTrack` 保持
"提案里恰好一条 `RemoveTrack`"的旧形状，既有调用方与既有响应字节不变。

## 诚实的边界（不假装做过）

1. **只清路由图**：`TrackV3::folder_id` 若指向被取走的音轨，模型
   `YebanProjectV1::validate` 会报 `TrackNotFound` ⇒ 提案**响亮失败**。工具面
   没有能设置 `folder_id` 的形态，因而也**没有**能安全级联改父的形态；这条登记为
   已知边界，不做静默改写。
2. **不检测"图仍然可达"**：清理只保证不留下引用被删音轨的边/节点，不重连、
   不替代音轨。若删掉的是唯一的中继节点，主总线可达性由下游渲染的既有错误报出。
3. **实时侧不受影响**：清理发生在工具面编译期，走带与引擎快照不经过这里。

## 判据

- `crates/yeban-mcp/src/domain/notes.rs`：
  `routing_cleanup_disconnects_edges_before_removing_the_node`（顺序、双向过滤、
  撤销载荷、整批反转后节点集与边表逐条相同）、
  `routing_cleanup_is_empty_for_a_track_outside_the_graph`（旧形状不变）。
- `crates/yeban-mcp/tests/tools_e2e.rs`：
  `edit_notes_remove_track_deletes_one_track_and_undo_restores_it` 增加 G2 断言 ——
  提案 op 序列为"断边 → 移除节点 → 取走音轨"，合并后 `routing_graph` 与 `tracks`
  同步减少，**`yeban_render_master` 成功**（修复前为 `RENDER_FAILED`），撤销后图与
  工程逐字节复原。
