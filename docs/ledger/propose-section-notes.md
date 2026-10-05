# `line/propose-section` — 拆掉 `yeban_propose_section` 的**代码级假阻塞**

- **台账类型**：交付映射 / 判据证据 / 本地决策与 needs（**不是规范**）
- **工作线**：`line/propose-section`（worktree `yeban/.worktrees/propose-section`，分支 `line/propose-section`，基线 `8529b31`）
- **所有者目录**：`crates/yeban-mcp/**`（本文件 + `tools-domain-notes.md` / `mcp-render-notes.md` 的
  **仅 `MCP-TOOL-005` 相关行**）
- **规范来源**：
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2（`MCP-TOOL-005`：`yeban_propose_section` =
    "在隔离分支 `ai/proposal-{ulid}` 创建章节配器骨架与声部连接"）、§6.1/§6.2（`ARCH-OPS-001/002`）
  - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` **D27**（= `HD-12`：`Op` 23 → 27，
    `AddClip`/`RemoveClip`/`AddRoutingNode`/`RemoveRoutingNode` 追认）、**D43**（没有兼容包袱 ⇒ 旧假设该删就删）、
    **D25**（错误码联集 20 值，不许发明新码）、**D12**（同族的两个删除变体）
  - `schemas/ops.schema.json`（27 变体契约）、`schemas/mcp-tools.schema.json`（工具契约）
  - 承接：`docs/ledger/tools-domain-notes.md` 的 **needs-1** / **boundary-1**、
    `docs/ledger/mcp-render-notes.md` 的 **needs-4**

> **一句话**：`yeban_propose_section` 曾经在响应里报
> `data.unwired = ["clipPoolEntries","routingEdges"]`，理由是"`Op` 全集没有 `AddClip`/`AddRoutingNode`"。
> 那个理由在 `ADR-0001` **D27**（2026-10-04 追认）之后**已经不成立**，但代码与台账都停在旧结论上。
> 本线把那段过期自我限制删掉，用**模型里真实存在**的 `Op` 真的生成骨架与声部连接，
> 并让 `unwired` 变成**从真实 op 推导**的量（而不是一句会腐烂的声明）。

---

## 1. 改了什么（文件 ↔ 职责）

| 文件 | 变化 | 说明 |
| :--- | :--- | :--- |
| `crates/yeban-mcp/src/domain/section_build.rs` | **新增** | 真实的骨架生成器：段落 / 片段池条目 / 摆放 / 音轨 / 路由节点 / 路由边。**零重依赖**（只用 `yeban-model` + `serde_json` + `std`），因此本机可用裸 `rustc --edition 2024 --test` 真跑 |
| `crates/yeban-mcp/src/domain/section.rs` | **重写为适配层** | 只剩两件事：`BuildFault` → `Fault`（`error_code` 穷举映射）、`Op` → JSON。**删掉**模块头里"`Op` 全集没有 AddClip/AddRoutingNode"的两条断言与 `SectionPlan::preview()` 里写死的 `unwired` 字符串 |
| `crates/yeban-mcp/src/domain/mod.rs` | 少量接线 | ① 注册 `pub mod section_build`；② `draft_unwired` 从**真实 `opKinds`** 推导（不再写死）；③ `Plan::Propose` 预览与提案响应新增 `willCreate`（从同一份 `ops` 派生，"将要做什么"与"真的做了什么"不可能漂移） |
| `crates/yeban-mcp/tests/tools_e2e.rs` | **判据更新 + 新增** | 旧断言（`unwired == ["clipPoolEntries","routingEdges"]`）反转成 `unwired == []`；新增 5 条端到端判据（骨架增量 / 连接方向与类型 / 逐字节逆操作 / dryRun 预览 / 缺件错误 / 幂等 / 输入→输出） |
| `crates/yeban-mcp/verify/section_pure.rs` | **新增** | 本机验证脚手架：`#[path]` 引入**真实源文件**（`section_build.rs` + `ids.rs`）+ 3 条独立口径的对抗性判据 |

**没有**新增任何第三方依赖、**没有**改 `Cargo.toml` / `Cargo.lock` / `schemas/**` / 其它 `crates/**`。

---

## 2. `Op` 的选择与理由（先读再写，不假设）

先读 `crates/yeban-model/src/ops.rs`（**只读**）确认 27 个变体里到底有什么、每个变体的
`precondition` 要求哪些**真实存在**的 id：

| §7.2 的能力 | 用的 `Op` | 它自己声明的前置条件（实测） | 本线的顺序处置 |
| :--- | :--- | :--- | :--- |
| 段落 | `SetSection { old_section: None, .. }` | 新身份不得已存在（旧值必须与文档一致） | 批次第一条 |
| 配器骨架里的**片段** | `AddClip { clip }` | 身份不得已在 `clip_pool` 里 | **先于**摆放 |
| 声部音轨 | `AddTrack { track }` | `track.validate()` 且身份不重复 | 先于摆放 |
| 片段**摆放** | `AddClipPlacement { track_id, placement }` | ① 音轨存在；② `clip_pool` 里**已有**该片段 | 在 `AddClip` + `AddTrack` **之后** |
| **声部连接**（节点） | `AddRoutingNode { node }` | 节点不得已在 `routing_graph.nodes` 里 | 在 `ConnectRouting` **之前**，且**只在缺失时**加（含主总线） |
| **声部连接**（边） | `ConnectRouting { edge }` | 两端**都已在** `routing_graph.nodes` 里；边身份不重复 | 最后 |

生成顺序（`opKinds` 的真实形状）：

```text
SetSection · AddClip ×N · AddTrack ×N · AddClipPlacement ×N · AddRoutingNode ×N(+1) · ConnectRouting ×N
```

**为什么必须是这个顺序**：`Op::Batch::precondition` 返回 `Ok(())`，子操作的前置条件是在
`Batch::commit` 的探针克隆体上**按顺序**逐条检查的（`ops.rs:1428`"原子性：在克隆体上整体应用
成功后再一次性提交"）。把摆放排在 `AddClip` 之前会直接 `ClipNotFound`。

**可逆性（`ARCH-OPS-001/002`）**：`Op::Batch` 的逆 = 子操作逆的**逆序**
（`ops.rs:1208-1219`）。逆序恰好满足"先删边再删节点、先删摆放再删片段"的前置条件，
因此整批是**逐字节**可回退的（证据见 §6）。

**方向与类型**：声部 → 主总线的边一律 `RoutingKind::TrackToBus`、`gain_db: None`
（`None` = 单位增益，是语义而不是"未填"；与模型规范样本 `samples.rs` 的 `lead → master` 同口径）。

---

## 3. 生成的骨架长什么样（真实响应，贴原文）

一次性取证程序（**本机真跑**，`#[path]` 指向真实源文件）在
`samples::filled_project()` + `Chorus/synthwave/8 小节/C minor` 上的输出：

```text
### opKinds = ["SetSection", "AddClip", "AddClip", "AddClip", "AddClip",
              "AddTrack", "AddTrack", "AddTrack", "AddTrack",
              "AddClipPlacement" ×4, "AddRoutingNode" ×4, "ConnectRouting" ×4]
### unwired = []
### 计数: sections 2->3  tracks 4->8  clip_pool 2->6  routing.nodes 4->8  routing.edges 3->7
```

段落的真实增量（`+sections`）：

```json
{ "id": "2RKN37DGB9ECRWKXY71V3P162T", "name": "Chorus", "start_tick": 15360, "end_tick": 46080 }
```

片段池条目的真实增量（`+clip_pool`，节选一个；**内容来自工程里已有的真实材料**，不是凭空造音乐）：

```json
{ "id": "1NSNZM3BPGY9QJX38MA5CAHA0M", "name": "Chorus · Pad",
  "content": { "Midi": { "notes": {
    "6EDJHSJH7FJPV2WB47SKVE3PZ4": { "id": "6EDJHSJH7FJPV2WB47SKVE3PZ4", "start_tick": 0,
      "duration_ticks": 480, "pitch": 60, "velocity": 100, "syllable": "do" },
    "6EDJHSJHFFJPV2WB47SKVE3Q8Z": { "id": "6EDJHSJHFFJPV2WB47SKVE3Q8Z", "start_tick": 960,
      "duration_ticks": 480, "pitch": 64, "velocity": 100, "ratchet": 2, "syllable": "re" },
    "6EDJHSJHQFJPV2WB47SKVE3QJT": { "…": "…", "pitch": 67, "micro_timing_ticks": -12, "syllable": "mi" },
    "6EDJHSJHZFJPV2WB47SKVE3QWN": { "…": "…", "pitch": 72, "probability": 0.75, "syllable": "fa" }
  } } } }
```

音轨与摆放的真实增量（`+tracks`）：

```text
3RAF3ED192DTER880GV8C943P1 name=Chorus · Pad kind=Midi placements=1
   {"id":"7V2VZ9R9EJXPEVNRWHME5PT485","clip_id":"1NSNZM3BPGY9QJX38MA5CAHA0M",
    "start_tick":15360,"duration_ticks":30720,
    "loop_config":{"enabled":false,"start_tick":0,"end_tick":0},"muted":false}
03F77TVR2Y3F8S6Y203Y051ERB name=Chorus · Lead  … 3CD16HJ9S2DTER880DWGHCTC9M name=Chorus · Arp
6JBB7SPWTY3F8S6VBXB3CDKSYD name=Chorus · Bass
```

声部连接的真实增量（`+routing_graph`）：

```text
nodes: 03F77TVR2Y3F8S6Y203Y051ERB  3CD16HJ9S2DTER880DWGHCTC9M
       3RAF3ED192DTER880GV8C943P1  6JBB7SPWTY3F8S6VBXB3CDKSYD
edges:
{"id":"1STANJ9QSF4FBM4R2CXG4BR5Z8","source_node":"03F77TVR…","destination_node":"01J8ZQ00000000000000000001","kind":"TrackToBus"}
{"id":"6778E7GTMQJ5ZBB48023483DF6","source_node":"3RAF3E…","destination_node":"01J8ZQ00000000000000000001","kind":"TrackToBus"}
{"id":"12ZNZ7CJVMY2HQPTDQTY11WCZ5","source_node":"3CD16H…","destination_node":"01J8ZQ00000000000000000001","kind":"TrackToBus"}
{"id":"6ANA3GQ525VZFQCNVH6PGACS8A","source_node":"6JBB7S…","destination_node":"01J8ZQ00000000000000000001","kind":"TrackToBus"}
```

> 主总线身份是样本工程的 `01J8ZQ00000000000000000001`；4 条边的目标都是它，方向都是"声部 → 主总线"。

### 3.1 材料从哪来（**本地裁决**，规范没给）

`yeban_propose_section` 的输入里**没有**任何"材料"参数（§7.2 只给
`sectionName/stylePreset/bars/scale`），而每个声部必须有一条**真实**片段池条目
（`AddClipPlacement` 的前置条件要求片段已在池里）。本线的口径：

1. **材料** = `clip_pool` 里 `ClipContent::Midi` 且**至少一个音符**的条目，按 `BTreeMap` 键序（确定序）；
   空 MIDI 片段**不是**材料（它不携带音乐内容）；
2. 第 `i` 个声部取 `materials[i % materials.len()]`（轮转；声部数可以多于材料数）；
3. 每个声部拿一条**新**片段池条目（`AddClip`），内容 = 材料音符的**逐字段拷贝**，
   只有两处不同：音符身份派生为 `note:{clip_id}:{原身份}`（因此不同片段不共享音符身份）、
   音高按 `scale` 做**等音类移调**（`delta = (tonic_pc - 材料最低音的音级) mod 12`，
   `pitch + delta > 127` 时**减一个八度**折叠 ⇒ 音级不丢、音域恒在 `0..=127`）；
   不给 `scale` 时 `delta = 0`（原样拷贝）；
4. **片段池里没有可用材料 ⇒ `CLIP_NOT_FOUND`**，`data` 里带
   `missing / acceptedContent / clipPoolEntries / clipPoolEntryCount / requiredParts / why`。
   **不造 id、不假装成功、不退回 `unwired`。**

`scale` 真的进输出：`C minor` 与 `D minor` 的产物逐音符相差 2 个半音且保音级（判据
`section_name_style_bars_and_scale_really_change_the_output` 与端到端
`propose_section_outputs_track_the_inputs`）。

---

## 4. `unwired` 现在是什么

| 位置 | 之前 | 现在 |
| :--- | :--- | :--- |
| `domain/section.rs` 模块头 | 断言"`Op` 全集没有 `AddClip`/`AddRoutingNode`，因此表达不出来" | **删除**（D43：过期假设该删就删，不写"保留 + 记条件"） |
| `SectionPlan::preview()` | 写死 `"unwired": ["clipPoolEntries","routingEdges"]` + 一段 reason | 整个死代码函数**删除**（它只服务于那条过期声明） |
| `domain/mod.rs::draft_unwired` | 写死 `vec!["clipPoolEntries","routingEdges"]` | **从提案真实的 `opKinds` 推导**：`AddClip` 缺席 ⇒ `clipPoolEntries`；`AddRoutingNode`/`ConnectRouting` 缺席 ⇒ `routingEdges` |
| `SectionPlan::unwired` | （字段不存在） | 同一函数从 `ops` 推导，随规划一起返回 |
| 响应 `data.unwired` | `["clipPoolEntries","routingEdges"]` | **`[]`**（判据断言它不含这两个键） |

**为什么"推导"比"写空数组"强**：`[]` 是一句会腐烂的声明；推导则让
"代码真的做了什么"与"响应声称缺什么"**永远同一件事**。判据
`unwired_is_derived_from_the_real_ops` 同时钉住两个方向：真做 ⇒ 报空；把某条相位摘掉 ⇒
`unwired` **自己**把对应键报回来（§8 的注入 1/2 就是这么变红的）。

**仍然真实存在的缺口**（不藏在 `unwired` 里，而是**明确错误**）：
`clip_pool` 里没有可用材料 ⇒ `CLIP_NOT_FOUND`。理由很硬：没有任何 MCP 工具能创建片段池条目
（`yeban_edit_notes` 需要**已存在**的 `clipId`）。⇒ 登记为 **needs-2**（见 §9）。

---

## 5. 判据清单（22 条本机真跑 + 6 条 CI-only）

### 5.1 本机**真跑**（`rustc --edition 2024 --test -D warnings`，25 passed / 0 failed）

`section_build.rs` 自带 19 条（`#[cfg(test)] mod tests`，随 `cargo test -p yeban-mcp` 在 CI 同样执行）
+ `verify/section_pure.rs` 3 条 + `ids.rs` 既有 3 条 = **25 passed; 0 failed**
（`0 warnings`，因为编译命令带 `-D warnings`；`clippy-driver -D clippy::all` 另跑，exit 0）。

| # | 判据（本机真跑） | 对应要求 |
| :--- | :--- | :--- |
| 1 | `skeleton_and_voice_routing_really_exist_after_applying` | ① sections/clip_pool/tracks/摆放 的**具体增量**；② 路由节点集合 + 每条边的 source/destination/kind/gain |
| 2 | `batch_inverse_restores_the_project_byte_for_byte` | ③ `Op::Batch` 的逆 ⇒ `serde_json::to_string` 逐字节回到调用前 |
| 3 | `planning_never_mutates_the_project` | ④（只读半边）规划与失败路径都不动工程 |
| 4 | `op_summary_matches_the_real_delta` | ④（清单半边）`willCreate` 的数 == 真的增量 |
| 5 | `missing_material_is_a_clear_clip_not_found` | ⑤ 空池 / 只有音频 / 只有空 MIDI ⇒ `ClipNotFound` + `data.missing`，不是 panic、不是空工程 |
| 6 | `missing_master_bus_is_a_clear_track_not_found` | ⑤（同类）无主总线 ⇒ `TrackNotFound` |
| 7 | `a_second_identical_batch_cannot_be_applied_twice` | ⑥ 模型侧：同请求两次 ⇒ 同一批 op；第二次被拒且**原子回滚** |
| 8 | `section_name_style_bars_and_scale_really_change_the_output` | ⑦ 章节名 / 风格（声部数+名字）/ 小节数 / 调式（移调口径独立算一遍）**各自**影响输出 |
| 9 | `unwired_is_derived_from_the_real_ops` | ⑧ op 在 ⇒ `unwired` 空；摘掉 `AddClip`/`AddRoutingNode`/`ConnectRouting` ⇒ 对应键**自己**回来 |
| 10 | `plan_is_deterministic_and_applies_cleanly` | §7.2 的确定性 + 整批可施加 |
| 11 | `a_master_bus_missing_from_the_node_table_is_added_once` / `an_existing_master_bus_node_is_not_added_twice` | `AddRoutingNode` 前置条件（不许重复身份、缺了必须补） |
| 12 | `parts_round_robin_over_the_available_materials` / `material_content_is_preserved_except_pitch_and_identity` | 材料轮转 + 内容逐字段保真（只有身份与音高可动） |
| 13 | `cycle_detection_finds_and_reports_cycles` / `a_cyclic_project_is_refused_before_arranging` | 模型层不判环，本层在**创建骨架之前**判（真 DFS 着色） |
| 14 | `bars_and_scale_are_validated` / `unknown_preset_is_style_not_found` / `note_names_and_pitch_classes_are_aligned` | 参数校验与两张表的对齐 |
| 15 | `routing_edges_are_never_cyclic`（脚手架） | 生成的连接图是 **DAG**、无自环（独立口径） |
| 16 | `skeleton_ids_are_never_nil_and_are_distinct`（脚手架） | 新身份不得 nil / 撞车 / 复用源材料音符身份；路由节点必须是真实音轨身份 |
| 17 | `missing_material_reports_the_typed_fault`（脚手架） | 缺材料是**类型化**的领域失败（不是字符串、不是 panic） |

### 5.2 CI-only（本机编不动整个 crate）

`run-gates.sh crate yeban-mcp` 在本机 **SKIP**（`yeban-mcp` 传递依赖
`yeban-render`/`yeban-decode` ⇒ rayon/hound/midly/symphonia/rubato，政策如此），因此下面这些
**只有 CI 判决算数**：

| # | 判据 | 对应要求 |
| :--- | :--- | :--- |
| i | `section.rs::the_plan_wires_clips_and_routing_and_reports_no_unwired` | 适配层：`unwired` 为空 + 五个 `Op` 变体都在 |
| ii | `section.rs::missing_material_is_clip_not_found` / `every_build_code_maps_into_the_contract_enum` | `BuildCode` → **契约内**错误码（D25 不许发明新码，7 个类别逐个断言） |
| iii | `tools_e2e.rs::propose_section_creates_a_real_section_and_is_deterministic` | ⑧ 响应 `data.unwired == []`（**"把 unwired 加回来 ⇒ 红"的 CI 侧机械保护**）+ 确定性身份 + `willCreate` 计数 |
| iv | `tools_e2e.rs::propose_section_merge_writes_the_skeleton_and_the_voice_routing` | ①② 在**真实 JSON-RPC 管线 + 真容器**上的增量、方向、类型 |
| v | `tools_e2e.rs::propose_section_ops_are_reversible_byte_for_byte` | ③ 用**线载荷**里的 stamped op 走 `apply_inverse` ⇒ 逐字节回退 |
| vi | `tools_e2e.rs::propose_section_dry_run_previews_without_touching_bytes` | ④ dryRun：工程字节 / 提交数 / 提案数都不变，且 `preview.willCreate` 给清单 |
| vii | `tools_e2e.rs::propose_section_without_usable_material_is_a_clear_clip_not_found` | ⑤ 端到端 `CLIP_NOT_FOUND` + `data.missing` + 不留提案 + 不改字节 |
| viii | `tools_e2e.rs::propose_section_same_idempotency_key_does_not_duplicate_the_skeleton` | ⑥ 同键第二次 `replayed: true`、同一条提案、合并后只有 4 个声部 |
| ix | `tools_e2e.rs::propose_section_outputs_track_the_inputs` | ⑦ 端到端：调式移调（保音级）/ 声部名 / 小节跨度 / 摆放时值 |
| x | 既有 `reject_keeps_a_traceable_record_and_blocks_merging`、`idempotent_merge_does_not_apply_the_batch_twice`、`dry_run_previews_match_what_the_real_call_commits`、`proposal_ops_are_reversible_through_the_model_inverse`、`every_emitted_error_code_is_inside_the_contract_enum` | ⑨ 提案线（propose → merge/reject）语义**未被破坏** |

---

## 6. 逆操作逐字节回退的证据（本机真跑原文）

```text
before   sha256=0218fa7d620d48e23363f3a49621d8af647f6abb10f8423feeaebafce057265b bytes=4978
after    sha256=0e76906b199683d9d924a0886fa0523df6339c6c6e1dfcbaced6ebe6d8e4ec59 bytes=10691
inverted sha256=0218fa7d620d48e23363f3a49621d8af647f6abb10f8423feeaebafce057265b bytes=4978
byte_exact_restore = true
```

口径：`Op::Batch { ops }.apply(&mut doc)` → `apply_inverse(&mut doc)` →
`serde_json::to_string` **逐字节**等价（不是"字段相等"）。CI 侧另有
`propose_section_ops_are_reversible_byte_for_byte`（走**线上 JSON → stamped op** 的路径，
证明线载荷与内存载荷一致）。

---

## 7. 本机真跑 vs CI（**严格区分**）

| 项目 | 本机 | 命令 / 读数 |
| :--- | :--- | :--- |
| `yeban-model` 构建（给脚手架链接） | ✅ 真跑 | `bash scripts/dev/cargo-local.sh build -p yeban-model` → `Finished dev profile … in 9.46s`（只有 serde/serde_json/sha2/thiserror/ulid，纯 Rust） |
| 骨架生成器 22 条判据 | ✅ 真跑 | `rustc --edition 2024 --test -D warnings crates/yeban-mcp/verify/section_pure.rs --extern yeban_model=… --extern serde_json=… -L dependency=target/debug/deps` → **25 passed; 0 failed**（含 `ids.rs` 既有 3 条） |
| clippy（生成器这一层） | ✅ 真跑 | `clippy-driver --edition 2024 --test -D warnings -D clippy::all …` → **exit 0，零输出** |
| 机械红线 + 文档 + 许可 | ✅ 真跑 | `bash scripts/gates/run-gates.sh light` → `门禁通过 (mode=light)`，exit 0 |
| `crate yeban-mcp` 的 clippy + 全套测试 | ❌ **本机不跑** | `bash scripts/gates/run-gates.sh crate yeban-mcp` → `SKIP yeban-mcp 含重依赖, 本机不编译 (交给 GitHub CI)` |
| 格式化 | ✅ 真跑 | `bash scripts/dev/cargo-local.sh fmt --all --check` → exit 0 |
| 注入 → 变红 → 还原 | ✅ 真跑 | 见 §8 |
| 端到端判据（`tests/tools_e2e.rs`）+ `src/**` 的单元判据 | ⏳ **CI** | 需要 `yeban-render`/`yeban-decode` 的完整编译 |

> `cargo fmt --all --check` 会**解析**全部 Rust 文件（因此能抓住语法级错误），
> 但**不能**替代类型检查 —— 适配层 / 接线 / 端到端判据的类型正确性由 CI 判决。

---

## 8. 注入 ▸ 变红 ▸ 还原（3 次，本机真跑原始读数）

方法：`section_build.rs` 先备份到 `/tmp/ps-drill/`，用 Python 精确替换
（`assert t.count(old) == 1`，防注入无效 —— L3 纪律），重编译 + 重跑脚手架，
`grep -E "^test .* FAILED"` 记录红判据名，再从备份还原并用 **sha256 比对**确认逐字节还原。

| # | 注入 | 变红（实测读数） | 还原 |
| :--- | :--- | :--- | :--- |
| **1** | 骨架生成改成**空操作**（不加 `AddClip`） | `10 passed; 15 failed` —— 含 `skeleton_and_voice_routing_really_exist_after_applying`、`unwired_is_derived_from_the_real_ops`（`unwired` **自己**报回 `clipPoolEntries`）、`skeleton_ids_are_never_nil_and_are_distinct`、`op_summary_matches_the_real_delta`、`routing_edges_are_never_cyclic` 等 | ✅ `25 passed; 0 failed`，sha256 `e00772…` 与备份一致 |
| **2** | **连接生成整段删掉**（不加路由节点、不加路由边） | `20 passed; 5 failed` —— `skeleton_and_voice_routing_really_exist_after_applying`、`unwired_is_derived_from_the_real_ops`（报回 `routingEdges`）、`routing_edges_are_never_cyclic`、`a_master_bus_missing_from_the_node_table_is_added_once`、`an_existing_master_bus_node_is_not_added_twice` | ✅ 同上 |
| **3** | 把 `unwired` **加回来**（`unwired_for_section_op_kinds` 恒返回两个旧键） | `24 passed; 1 failed` —— 精确红一条：`unwired_is_derived_from_the_real_ops` | ✅ 同上 |

**注入 3 的双侧保护说明**（诚实区分）：本机能证明的是**本地**那一条判据红；
CI 侧对应的机械保护是 `tools_e2e.rs::propose_section_creates_a_real_section_and_is_deterministic`
里的 `assert_eq!(first["data"]["unwired"], json!([]))`，它**本机跑不了**（重依赖），
由 CI 判决。

---

## 9. 边界 / needs / pending

### 已知边界（本线明确**没做**或**没证明**的事）

| # | 边界 | 说明 |
| :--- | :--- | :--- |
| boundary-A | **调式只用了主音音级，没有用调式音阶结构** | `scale` 目前只决定移调的音级锚点 ⇒ `D dorian` 与 `D minor` 的**产物逐字节相同**（`tonic_pc` 都是 2）。要真正按调式生成/过滤，需要 `yeban-theory` 的音阶语义（见 needs-1）。这是本线**如实登记**的缩水，不藏在 `unwired` 里 |
| boundary-B | 声部音轨**没有设备/乐器** | 骨架只有音轨 + 片段 + 连接，没有 `InsertDevice`（那个变体**存在**，但"给哪个声部装什么乐器"是编排决策，规范没给）。因此骨架还发不出声 |
| boundary-C | 骨架**不在 Session View 建场景** | §7.2 只说"章节配器骨架与声部连接"，没有要求 `SceneV3`。`SetScene`（`old_scene: None`）表达得了，本线没做 |
| boundary-D | 材料是**拷贝**而不是引用 | 每个声部拿到独立片段（音符身份派生），因此改一个声部不影响别的声部 —— 这是编排骨架想要的行为，但意味着 `clip_pool` 会长 N 条 | 
| boundary-E | 装配方案本身没有规范依据 | "轮转分配材料 + 等音类移调 + 八度折叠"是**本地裁决**（§7.2 只给四个入参）。机制（未知预设 `STYLE_NOT_FOUND`、缺材料 `CLIP_NOT_FOUND`、不可逆则不许提交）是承重的；**具体分配算法**可替换 |

### needs（需要裁决 / 需要别的所有者接线）

| # | 项 | 性质 | 建议 |
| :--- | :--- | :--- | :--- |
| **needs-1** | **风格/声部表 + 调式语义是否应从 `yeban-theory` 取** | 集成者要求登记（本线**没有**接线，也**不建议**本线接线） | 现状：风格/声部来源仍是 `section.rs`（原 `:45`，现 `section_build.rs`）的**本地 4 行表**；调式语义只有 `NOTE_PITCH_CLASSES` 与 `MODES` 字符串白名单。`yeban-theory` 里**有**对应的真语义：`yeban_theory::genre::{GenreLibrary::get(id)/ids()/search()}`、`GenreRule::primary_scale(tonic)`、`GenreRule::sketch(…)`、`GenreRule::chords(tonic)`、`yeban_theory::scale::{Scale::parse, ScaleKind, Scale::pitch_classes, Scale::contains, Scale::degree_of, Scale::diatonic_triads}`、`yeban_theory::progression::{Progression::parse, Progression::expand}`、`yeban_theory::voice_leading::*`。**具体缺的语义**：① 预设 id → 声部名的映射（`GenreLibrary::ids()` 的 id 与 `STYLE_PRESETS` 的四个名字未必同名）；② "调式 → 可用音级集合"（判据要用的正是 `Scale::pitch_classes/contains`）；③ 声部数/音域/密度的编排提示。**为什么本线不做**：接线要改 `crates/yeban-mcp/Cargo.toml` 的依赖边，而 `yeban-theory` 目前**零消费者** ⇒ 会动 `Cargo.lock`（本线禁改），且 `GenreRule` 的语义（流派规则/出处）与"声部名预设"是否对得上必须先读清楚。**留给集成者排期。** |
| **needs-2** | **`clip_pool` 的"材料创建"没有工具** | 真实能力缺口 | 空池工程做不了配器（本线如实报 `CLIP_NOT_FOUND`）。`Op::AddClip` 在模型层已可表达，但**没有任何 MCP 工具**让 Agent 把一条片段放进池子（`yeban_edit_notes` 需要已存在的 `clipId`；`yeban_save_project` 只能保存既有内容）。建议：要么在 §7.2 层面给 `yeban_edit_notes`/新工具一个"创建片段"的形状，要么明确"配器前必须由人/导入路径准备材料" |
| **needs-3** | `ADR-0001` **D27** 的"问题陈述"里仍写着"那条工具只能返回 `data.unwired = [...]`" | 文档时效 | 那是**历史问题陈述**（裁决本身已把 4 个变体追认进全集），但今天读起来像是现状。`docs/adr/**` 本线**不改**（禁改），建议集成者按 D43 就地改写为"已由 `line/propose-section` 接线" |
| **needs-4** | `willCreate` 只汇总**六类结构实体** | 本地决策 | `summarize_ops` 覆盖 `SetSection/AddTrack/AddClip/AddClipPlacement/AddRoutingNode/ConnectRouting`；`AddNote`（音符）与自动化点仍只在 `ops` 全量载荷里。若下游要"按实体类预览音符增量"，需要扩展（`AddNote` 的载荷格式已可表达） |
| **needs-5** | `MAX_BARS` / `MAX_PARTS` / 四个预设 / 色板 / `NOTE_PITCH_CLASSES` | 本地决策（承接 `tools-domain-notes` needs-6） | 机制承重、内容可替换；接线理论库时只改 `section_build.rs` 的常量与材料装配函数 |

### pending

| # | 项 | 状态 |
| :--- | :--- | :--- |
| P-1 | `tools-domain-notes.md` 的 **needs-1** / **boundary-1**、`mcp-render-notes.md` 的 **needs-4** | **本线关闭**（同一次提交内回写） |
| P-2 | CI 判决 | 推送后由 `scripts/dev/ci-verdict.sh` 读回（本文件 §10） |

---

## 10. CI 判决（读到什么写什么）

- 代码 + 台账提交：`pending`（推送后回填 commit / run id）
- 本机读数**不是**判决：本机 25 passed + light 门禁通过只说明"生成器这一层与机械红线是绿的"，
  `yeban-mcp` 的 clippy 与端到端判据必须等 CI。

---

## 11. 文件清单与净行数

| 文件 | 状态 | 净增/减（`git diff --numstat`，见提交） |
| :--- | :--- | :--- |
| `crates/yeban-mcp/src/domain/section_build.rs` | 新增 | 生成器 + 19 条判据 |
| `crates/yeban-mcp/src/domain/section.rs` | 重写 | 适配层（旧模块头断言与 `preview()` 删除） |
| `crates/yeban-mcp/src/domain/mod.rs` | 改 | 模块注册 + `draft_unwired` 推导 + `willCreate` |
| `crates/yeban-mcp/tests/tools_e2e.rs` | 改 | 1 条反转 + 5 条新增 + 3 个夹具 |
| `crates/yeban-mcp/verify/section_pure.rs` | 新增 | 本机脚手架（`#[path]` 引真实源 + 3 条独立判据） |
| `docs/ledger/tools-domain-notes.md` | 改 | 仅 `MCP-TOOL-005` 相关行 |
| `docs/ledger/mcp-render-notes.md` | 改 | 仅 needs-4 一行 |
| `docs/ledger/propose-section-notes.md` | 新增 | 本文件 |
