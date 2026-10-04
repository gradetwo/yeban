# `tools-domain` 工作线台账：十工具领域落地、判据与未接线清单

- **台账类型**：交付映射 / 逐工具状态表 / 判据证据 / 未决项（**不是规范**）
- **工作线**：`line/tools-domain`（worktree `yeban/.worktrees/tools-domain`）
- **所有者目录**：`crates/yeban-mcp/**`（本文件是唯一新增的文档）
- **规范来源**：
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.1 / §7.2（`ARCH-SEC-002`、`MCP-TOOL-001..010`）、
    §6.1/§6.2（`ARCH-OPS-001/002`）、§5.3/§5.4（`ARCH-SEC-003/004` 原子落盘）、§0.2（`ARCH-SEC-001` `.yeban.lock`）
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M4-003/004`、`MUST-GATE-010`
  - `schemas/mcp-tools.schema.json`（**承重**契约：工具名 / `dryRun` / `idempotencyKey` / 错误码联集 20）
  - `schemas/project.schema.json`（工程文件权威契约）、`schemas/ops.schema.json`（`Op` 信封）
  - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md`（D12 `Op` 补两变体、D13 `McpProposal`
    外部标签、D21 通配路径豁免、**D25 契约联集 20 值 + 承重根**）
  - 前置工作线台账：`docs/ledger/mcp-core-notes.md`（本线接手它的 **P1**：十工具领域实现未接线）

> 本文件回答四个问题：**十个工具各自到底做到哪一步**、**`dryRun`/幂等怎么被证明**、
> **哪些东西明确没做**、**需要谁裁决什么**。

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件 | 规范 ID | 说明 |
| :--- | :--- | :--- |
| `src/domain/mod.rs` | `MCP-TOOL-001..010`、`ARCH-OPS-001/002` | `Domain`（活跃工程 + 提交图谱 + 提案记录 + 注入时钟）、`Plan`（只读规划）、`plan`/`apply`/`execute`/`preview` |
| `src/domain/error.rs` | `MCP-TOOL-001..010`、ADR-0001 D25 | 领域失败 ↔ 契约错误码的**唯一**映射（`ModelError` 穷举、`io::ErrorKind`）；`Fault` 把"领域失败"与"实现级状况"分成两条出口 |
| `src/domain/store.rs` | `ARCH-SEC-004`、`ARCH-SEC-001` | 工程文件读取、**原子落盘**（同目录临时文件 + `sync_all` + `rename`）、`.yeban.lock` 原子创建/释放、SHA-256 摘要 |
| `src/domain/view.rs` | `MCP-TOOL-004` | 字段选择器白名单 + 分页（实体索引**不含音符**） |
| `src/domain/notes.rs` | `MCP-TOOL-006` | `NoteOp`（4 种 `kind`）→ `Op` 编译、音域/发声数校验 |
| `src/domain/section.rs` | `MCP-TOOL-005` | 风格预设 → 段落 + 声部音轨骨架；**DFS 着色判环**（模型层不判环） |
| `src/domain/macros.rs` | `MCP-TOOL-007` | 宏旋钮 + 按 `MacroMapping` 级联 S 曲线自动化点 |
| `src/domain/proposal.rs` | `MCP-TOOL-005/006/007/009/010`、`ARCH-OPS-002` | 提案记录（状态、基于哪个提交、op 清单、合并/拒绝留痕） |
| `src/domain/render.rs` | `MCP-TOOL-008` | 渲染参数校验（`format` 白名单 + 走模型层 `SampleRate::from_hz`）；渲染本体**未接线** |
| `src/domain/ids.rs` | `MODEL-AST-001` 精神 | 确定性夹具身份（FNV-1a 128 → Crockford Base32 ULID），让 `dryRun` 预览与真调用逐字节一致 |
| `src/dispatch.rs` | `MCP-TOOL-001..010` | `Dispatcher` 持有 `Domain`；`dryRun` 走只读 `domain::preview`；幂等缓存查询仍在执行之前 |
| `tests/tools_e2e.rs` | `MCP-TOOL-001..010`、`MUST-GATE-010` | **25 条端到端判据**（真实文件系统 + 真实 JSON-RPC 管线） |

**依赖图变化（集成者需要知道）**：`crates/yeban-mcp/Cargo.toml` 新增一条 `yeban-model.workspace = true`
（根 `[workspace.dependencies]` 早已登记，**未改根 `Cargo.toml`**）。连带：
`Cargo.lock`（1 条依赖边）与 `docs/ledger/dependency-licenses.md`（机器再生成，`Cargo.lock` 摘要变化）。
未引入任何新的第三方 crate ⇒ `deny.toml` 与许可政策不变。

---

## 2. 十个工具的实现状态表（**逐条如实**）

图例：**真做** = 领域语义完整；**半做** = 真的做了一部分，另一部分明确缩水；**未接线** = 只做校验/scope/dryRun。

| 规范 ID | 工具 | 状态 | 真做的部分 | 缩水/未接线的部分 |
| :--- | :--- | :--- | :--- | :--- |
| `MCP-TOOL-001` | `yeban_open_project` | **半做** | 真读盘 + 版本门 + `validate()`；`.yeban.lock` 原子创建（排他）；重复打开同一路径幂等（`alreadyOpen`）；打开别的工程 → `CONFLICT`；失败一律不改会话 | **没有** `fcntl(F_SETLK)`/`LockFileEx` OS 建议锁、**没有** `SHARED_READ` 多读者、**没有**心跳与陈旧锁抢占（探测 PID 需要 `libc`）。工程文件是**裸 JSON** 而不是 `ARCH-SEC-003` 的 ZIP 容器（见 §5 needs-3） |
| `MCP-TOOL-002` | `yeban_save_project` | **真做** | `ARCH-SEC-004` 三阶段原子落盘（同目录临时文件 + `File::sync_all` + `rename`）；失败**不破坏原文件**且清理临时文件；`ENOSPC`/`StorageFull`/`QuotaExceeded` → `DISK_FULL`；无改动默认跳过、`force: true` 强制落盘；只读会话拒绝落盘 | 未做"CAS 资产池"（裸 JSON 里没有资产字节）；`DISK_FULL` 只有**映射判据**（用合成 `io::Error` 钉住），没有真把磁盘写满（做不到，见 §5 boundary-2） |
| `MCP-TOOL-003` | `yeban_close_project` | **真做** | `saveFirst`（缺省 `true`）先原子保存再释放；`Drop` 释放 `.yeban.lock`；已关闭再关 → `NO_ACTIVE_PROJECT`；只读会话不尝试保存 | — |
| `MCP-TOOL-004` | `yeban_query_project` | **真做** | 字段选择器**白名单**（24 个，非法 → `INVALID_FIELD_SELECTOR` + 可选清单）；分页 `limit`/`offset`（默认 100 / 上限 1000 且夹紧上报）；实体索引只带 `{kind,id,name}` ⇒ 响应体积与音符数**解耦** | 选择器语法是白名单，不是自由 JSONPath（这是有意的，见 `view.rs` 模块头） |
| `MCP-TOOL-005` | `yeban_propose_section` | **半做** | 4 个风格预设（未知 → `STYLE_NOT_FOUND`）；`bars` 1..=64（越界 → `OUT_OF_RANGE`）；调式写法校验；段落起点接在最后一个段落之后；每个声部一条 MIDI 音轨；**整批 op 在克隆体上模拟后才建提案**；现有路由图成环 → `CYCLE_DETECTED` | §7.2 的"**声部连接**"与"配器骨架里的**片段**"没有产出：`ARCH-OPS-001` 的 `Op` 全集**没有** `AddClip`/`RemoveClip` 与 `AddRoutingNode`，而 `Op::ConnectRouting` 的前置条件要求两端已在 `routing_graph.nodes` 里 ⇒ **表达不出来**。响应里用 `data.unwired = ["clipPoolEntries","routingEdges"]` 明示（见 §5 needs-1） |
| `MCP-TOOL-006` | `yeban_edit_notes` | **真做** | 4 种 `NoteOp`（`add`/`delete`/`move`/`velocity`）→ `Op` 编译；`Delete` 的 `previous_note` 从当前文档读取；音域 `0..=127`（含平移后）→ `OUT_OF_RANGE`；**发声数峰值 > 32** → `OUT_OF_RANGE`（带 peak/limit）；音符不存在 → `ENTITY_NOT_FOUND`；产物是提案（可逆、可审查） | `NoteOp` 的 JSON 形状没有契约（§5 needs-2）；"发声数"上限 32 是本地常量 |
| `MCP-TOOL-007` | `yeban_set_macro` | **半做** | 音轨 → `TRACK_NOT_FOUND`；宏下标 → `INDEX_OUT_OF_BOUNDS`（带 `macroCount`）；值域 `0.0..=1.0` 且有限 → `OUT_OF_RANGE`；`Op::SetMacro` 的 `old_val` 从文档读；按每个 `MacroMapping` 展开 2 个 S 曲线自动化点（起点 → 一小节后） | **级联点的物理量纲**：`MacroMapping` 只有 `depth`，参数真实值域（dB/Hz/%）住在设备层 ⇒ 写的是**归一化值**，响应里 `normalized: true`（见 §5 needs-4） |
| `MCP-TOOL-008` | `yeban_render_master` | **未接线（另一半明确没做）** | `format` 白名单（`wav`/`rf64`/`bw64`）；`sampleRate` 走**模型层** `SampleRate::from_hz` 允许集合；`normalize` 缺省 `false`；无活跃工程 → `NO_ACTIVE_PROJECT`；`dryRun` **真的**做完全部校验并披露渲染器未接线 | **渲染本体**（触发 `yeban-render`、返回产物哈希与路径）没有接线 ⇒ 通过校验后返回 JSON-RPC `-32005`（`data.validated = true` + `data.request`）。`BUSY` 在当前单线程同步架构下**不可达**（登记，不是遗漏） |
| `MCP-TOOL-009` | `yeban_merge_proposal` | **真做** | 提案 op 包成**单一原子 `Op::Batch`** 施加；失败（含基线漂移）→ `CONFLICT`（带 `baseCommitMoved`）；合并后 `append` 一条主分支提交（`OpOrigin::McpProposal`）；幂等：已合并再合并 → `alreadyMerged` + `appliedOps = 0`；已拒绝的提案 → `CONFLICT` | `CommitGraph` **没有**"多父合并提交"的 API（`Commit.parents` 是 `Vec` 但 `append` 永远写单父），因此合并提交在主分支上只有一个父；提案分支与合并提交的对应关系由本线的 `Proposal` 记录承担（见 §5 needs-5） |
| `MCP-TOOL-010` | `yeban_reject_proposal` | **真做** | 未知提案 → `PROPOSAL_NOT_FOUND`（带 `proposalId`）；已拒绝再拒 → `alreadyRejected`（幂等）；已合并再拒 → `CONFLICT`；**记录保留**（`status`/`resolvedAt`/`resolution` = 拒绝原因）⇒"拒绝也必须可追溯"；拒绝**不改工程字节** | 未真正"释放无用内存快照"（`CommitGraph` 没有 GC API） |

**一句话**：**8 个真做 / 2 个半做（`open_project`、`propose_section`、`set_macro` 中的三处缩水）/ 1 个未接线的一半（`render_master` 的渲染器）**。
上一线的 **P1**（"十工具一律 `-32005 NOT_IMPLEMENTED`"）**关闭**：现在唯一可能返回 `-32005` 的路径是
`yeban_render_master` **参数校验通过之后**的那一半。

---

## 3. `dryRun` 与 `idempotencyKey` 的证据（不是"我实现了"，是"这个判据证明了它"）

### 3.1 `dryRun` 不改状态：两层保证

**结构性（编译期）**：`dryRun` 走的是 [`domain::plan`]，签名是

```rust
pub fn plan(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault>   // &Domain, 不是 &mut
```

⇒ 借用检查器**不允许**它改动任何状态。这不是"我记得不要写"，而是"写不了"。

**运行期（判据）**：`tests/tools_e2e.rs::dry_run_leaves_the_project_bytes_and_commit_count_untouched`
对 7 个工具各做一次 `dryRun`，每次都断言**四个量**逐项不变：

| 判据 | 断言 |
| :--- | :--- |
| 工程内容 | 前后 `serde_json::to_string_pretty(YebanProjectV1)` **逐字节相同**（`assert_eq!(bytes_after, bytes_before)`） |
| 内容摘要 | SHA-256 前后相同 |
| 提交图谱 | `Domain::commit_count()` 前后相同（`CommitGraph` 一条提交都没多） |
| 提案表 | `Domain::proposal_count()` 前后相同 |
| 幂等缓存 | `Dispatcher::idempotency_len()` 仍为 0（`dryRun` 不写缓存） |

**注入证据**：把 `dryRun` 分支改成"先 `domain::execute` 再返回预览"（注入 A），
`dry_run_leaves_the_project_bytes_and_commit_count_untouched` **变红**
（`24 passed; 1 failed`）。还原后 25/25 全绿。

**语义诚实性**：`dryRun` 做**真**领域校验，因此"没有活跃工程"时它返回**带内**领域失败
（`NO_ACTIVE_PROJECT`）而不是伪造成功。判据
`dry_run_reports_why_it_would_fail_instead_of_pretending_success` 对 6 个工具钉住这一点；
判据 `dry_run_preview_matches_what_the_real_call_commits` 钉住"预览里的 op == 真调用提交的 op"（逐字节）。

### 3.2 `idempotencyKey` 真的幂等：两层保证

**结构性**：`dispatch.rs` 的管线顺序是 `⑤ dryRun 短路 → ⑥ 幂等缓存查询 → ⑦ domain::execute`。
命中缓存就**永远到不了第 ⑦ 步**，因此"不重复施加"是顺序保证的，不是靠函数内部自觉。

**运行期**：两条端到端判据各用**两种不同的可观测量**证明：

| 判据 | 用什么量证明"只施加一次" |
| :--- | :--- |
| `same_idempotency_key_applies_exactly_once` | `Domain::proposal_count()`（提案表条数）**与** `Domain::commit_count()`（提交数）在第二次调用后都不变；第二次响应 `replayed == true` 且主体与首次逐字节相同；**不同 key** 视为新请求（两条计数各 +1） |
| `idempotent_merge_does_not_apply_the_batch_twice` | 第二次同键合并后 `commit_count()` 不变**且** `project_bytes()`（工程逐字节文本）不变 ⇒ `Op::Batch` 没有被施加两次 |

另有一条领域层幂等（不走缓存也成立）：已合并的提案再合并返回 `alreadyMerged: true, appliedOps: 0`。

**注入证据**：把幂等查询改成永不命中（注入 B / B′），

- `--lib` 目标：`dispatch::tests::idempotency_replays_the_same_result_with_the_current_id`、`dispatch::tests::distinct_keys_do_not_collide_and_cache_is_ordered` 变红；
- `--test tools_e2e` 目标（**必须单独跑**：cargo 在 lib 目标红掉后不会继续跑集成目标，这是 L12/L15 那条"先确认你读的是什么"的纪律）：
  `same_idempotency_key_applies_exactly_once`、`idempotent_merge_does_not_apply_the_batch_twice` 变红。

还原后两侧全绿。

### 3.3 原子保存的证据

`tests/tools_e2e.rs::save_into_a_read_only_directory_fails_with_io_error_and_keeps_the_original`：

1. 先造一份**原文件**并打开；
2. 真合并一条提案（把"未保存标记"变成真）；
3. `chmod 0555` 父目录（临时文件创建必然失败）；
4. 断言保存返回 `IO_ERROR`、**原文件逐字节不变**、目录里**没有 `.tmp-` 残留**；
5. 恢复权限后同一次保存成功，且**磁盘字节 == 内存工程**、未保存标记清掉。

`error::code_for_io` 的 `DISK_FULL` 分支用合成 `io::Error` 钉住
（`StorageFull` / `QuotaExceeded` / 裸 `ENOSPC=28`，Linux 与 macOS 同号）。

**注入证据**：在 `write_then_replace` 里加一句"先原地截断目标文件"（注入 C），
`save_into_a_read_only_directory_fails_with_io_error_and_keeps_the_original` **变红**
（返回了 `IO_ERROR`，但原文件已经被截断成 0 字节）。还原后全绿。

### 3.4 错误码纪律的证据

判据 ⑥ 分成两半：

- `implementation_error_code_catalog_equals_the_contract_enum_exactly`：`ErrorCode::SCHEMA_CONTRACT`
  与**直接读 `schemas/mcp-tools.schema.json`** 得到的 `enum` **集合完全相等**（双向包含 + 计数 20）；
- `every_emitted_error_code_is_inside_the_contract_enum`：跑一遍 60+ 次真实调用，
  收集**实际产出**的每一个 `ToolResponse.error.code`，断言每个都是契约 enum 的成员（实测覆盖 13 个不同的码）。

**注入证据**：把 `SCHEMA_CONTRACT` 里的 `DISK_FULL` 换成重复的 `BUSY`（模拟契约/实现漂移，注入 D/D′），
`--lib` 变红 `tools::tests::error_code_catalog_covers_the_union_contract_exactly` 与
`samples::tests::dry_run_response_sample_is_a_contract_valid_tool_response`；
`--test tools_e2e` 变红 `implementation_error_code_catalog_equals_the_contract_enum_exactly`。还原后全绿。

### 3.5 提案可逆的证据（复用模型层，不写第二套）

`proposal_ops_are_reversible_through_the_model_inverse`：合并一条提案 → 工程真的变了
（`assert_ne!(after, before)`）→ 对 `Proposal::ops` 逐条调用 **`StampedOp::apply_inverse`**
（即 `Op::invert` 的落点）→ 序列化字节回到合并前（`assert_eq!` 逐字节）。

本 crate 的 `src/` 里**没有任何**自定义 `invert`：`grep -rn "fn invert" crates/yeban-mcp/src`
**零命中**。这条判据因此同时钉住"逆操作只有一份事实源"。

---

## 4. 本机验证 vs 交给 CI

### 4.1 本机（Apple M2，全部真跑过）

```bash
bash scripts/gates/run-gates.sh crate yeban-mcp        # exit 0
bash scripts/gates/run-gates.sh light                  # exit 0
bash scripts/dev/cargo-local.sh test -p yeban-mcp      # 192 = lib 152 + bin 0 + contract 15 + tools_e2e 25 + doc 0
bash scripts/dev/cargo-local.sh test -p yeban-mcp --features mcp-http   # 192 条（同上）
bash scripts/dev/cargo-local.sh clippy -p yeban-mcp --all-targets -- -D warnings            # exit 0
bash scripts/dev/cargo-local.sh clippy -p yeban-mcp --all-targets --features mcp-http -- -D warnings  # exit 0
bash scripts/dev/cargo-local.sh fmt --all --check      # exit 0
bash scripts/dev/cargo-local.sh run -p yeban-mcp --example export_mcp_samples -- --out target/schema-samples
python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples   # exit 0（11 份实例全过 + 2 份 .meta. 显式 skip）
```

`crate yeban-mcp` **没有被 SKIP**：`yeban-model` 不在 `run-gates.sh` 的重依赖正则名单里，
而它依赖表里确实只有 `serde`/`serde_json`/`sha2`/`thiserror`/`ulid`（纯 Rust）。
因此本线的 `clippy -D warnings` 与全部测试都是**本机实测**，不是"留给 CI"。

### 4.2 注入 → 变红 → 还原（**每条都真做过，退出码为准**）

方法：把 `dispatch.rs` / `store.rs` / `tools.rs` 备份到 `/tmp/td-drill/`，
用 Python 精确替换（`assert t.count(old) == 1`，防止注入无效 —— 即 L3 的纪律），
跑 `cargo-local.sh test`，`grep -E "^test .* FAILED"` 记录红掉的判据名，再从备份还原并跑一遍全绿。

| # | 注入 | 变红的判据（实测） | 还原 |
| :--- | :--- | :--- | :--- |
| **A** | `dryRun` 分支先执行 `domain::execute` | `dry_run_leaves_the_project_bytes_and_commit_count_untouched`（e2e：`24 passed; 1 failed`） | ✅ 25/25 |
| **B / B′** | 幂等缓存查询改成永不命中 | **lib**：`idempotency_replays_the_same_result_with_the_current_id`、`distinct_keys_do_not_collide_and_cache_is_ordered`；**e2e（单独跑）**：`same_idempotency_key_applies_exactly_once`、`idempotent_merge_does_not_apply_the_batch_twice` | ✅ 全绿 |
| **C** | `write_then_replace` 先原地截断目标文件 | `save_into_a_read_only_directory_fails_with_io_error_and_keeps_the_original`（e2e：`24 passed; 1 failed`） | ✅ 25/25 |
| **D / D′** | `SCHEMA_CONTRACT` 的 `DISK_FULL` 换成重复的 `BUSY` | **lib**：`error_code_catalog_covers_the_union_contract_exactly`、`dry_run_response_sample_is_a_contract_valid_tool_response`；**e2e（单独跑）**：`implementation_error_code_catalog_equals_the_contract_enum_exactly` | ✅ 全绿 |

**这一轮本身的方法论收获**（值得写进账本）：第一遍跑注入 B/D 时**只看到 lib 目标红**——
`cargo test` 在第一个失败的目标之后就停了，因此集成目标**根本没跑**。
如果就此写成"e2e 判据也红了"，那就是 L12/L15 同族的假读数。第二遍显式 `--test tools_e2e` 才拿到真结论。

### 4.3 交给 CI 的部分

- `rust (workspace)` 全量腿：本机**不能**跑（`--workspace` 被 `cargo-local.sh` 直接拒绝）。
- `checks` 腿的 `jsonschema` 对账：本机**能**跑（上面 4.1 已跑，`exit 0`），CI 上是同一命令。
- 跨架构/基准/fuzz：本线**没有**新增打点，不涉及。

---

## 5. 边界 / needs / pending / TODO(hoist)

### 已知边界（**不是**缺陷，是本线明确没证明的事）

| # | 边界 | 说明 |
| :--- | :--- | :--- |
| boundary-1 | `Op` 全集的**表达力缺口**已实测，但只在本线内部规避 | 见 needs-1 / needs-5 |
| boundary-2 | `DISK_FULL` 只有**映射判据** | 真把磁盘写满需要 root 挂 tmpfs；本机与 CI 都不做。判据用合成 `io::Error`（`StorageFull`/`QuotaExceeded`/`ENOSPC`）钉住映射函数 |
| boundary-3 | `.yeban.lock` 只有"原子创建 + 存在即拒" | 没有 `fcntl`/`LockFileEx` 建议锁、没有心跳/陈旧锁抢占。**崩溃会留下永久锁**（规范允许 CLI `--force-unlock`，本线未实现） |
| boundary-4 | 工程文件是**裸 JSON** | `ARCH-SEC-003` 要求 ZIP 容器（`project.json` + `history.dag` + `assets/{sha256}`，含 Zip-Slip 与解压炸弹防御）。任务书指定用 `YebanProjectV1` JSON；容器属于别的所有者 |
| boundary-5 | `CommitGraph` 只有**单父** `append` | 提案分支由 `genesis` 建成孤立根提交；合并提交在主分支上单父。`Commit.parents` 是 `Vec` 但无 API 可写多父 |
| boundary-6 | `yeban_render_master` 的 `BUSY` 不可达 | 领域状态单线程同步 ⇒ 没有"已有渲染在跑"的窗口。接线到真正异步渲染管线时才可达 |
| boundary-7 | 表格里给 `yeban_render_master` 的声明错误码之外，本线会多产几个**契约内**的码 | 例：`yeban_save_project` 无活跃工程 → `NO_ACTIVE_PROJECT`（§7.2 那一行没列它）。这些码**全在** D25 的 20 值联集内，逐条有判据（§3.4） |

### needs（需要裁决 / 需要别的所有者接线）

| # | 项 | 性质 | 建议 |
| :--- | :--- | :--- | :--- |
| **needs-1** | `ARCH-OPS-001` 的 `Op` 全集缺 **`AddClip`/`RemoveClip`** 与 **`AddRoutingNode`/`RemoveRoutingNode`** | **规范缺口**（实测：`Op::AddClipPlacement` 的前置条件要求片段已在 `clip_pool`；`Op::ConnectRouting` 要求两端已在 `routing_graph.nodes`，而没有任何变体能把它们放进去） | 要落地 §7.2 的"章节配器骨架 + 声部连接"，必须先按 ADR 补 4 个变体（并同步 `schemas/ops.schema.json` 的 `op.oneOf`）。本线**不改**规范/schema，只在响应里上报 `unwired` |
| **needs-2** | `NoteOp` 的 JSON 形状**没有契约** | 规范缺口（`schemas/mcp-tools.schema.json` 把 `ops` 声明为无约束数组；§7.2 只写 `Vec<NoteOp>`） | 本线定义了 `add`/`delete`/`move`/`velocity` 四种 `kind`（见 `domain/notes.rs` 模块头）。建议集成者把它固化成 schema `$defs`，否则第二个实现会漂移 |
| **needs-3** | `.yeban` **ZIP 容器**（`ARCH-SEC-003`）归谁 | 分工不清 | 容器 = `project.json` + `history.dag` + CAS 资产。本线只做 `project.json` 那半。建议明确给 `yeban-model` 或 `yeban-app` 所有，并把 MCP 层的读写换成容器 API |
| **needs-4** | 宏级联的**物理量纲** | 跨层缺口 | `MacroMapping.depth` 是归一化的；参数 min/max 住在设备/参数层而 `ParameterValue` 不携带它们。本线写归一化值并标 `normalized: true`；设备层提供值域解析后必须改成 `false` |
| **needs-5** | `CommitGraph` 缺 **`create_branch(parent)`** 与 **多父合并提交** API | 模型层 API 缺口 | 现状：`fork_anonymous` 强制 `anon-` 前缀；`append` 永远单父。建议补两个 API，让"Musical PR"的 DAG 关系由 `CommitGraph` 而不是 MCP 层的 `Proposal` 记录承担 |
| **needs-6** | 风格预设表 / 音阶表 / 发声数上限 32 / `bars` 上限 64 | 本地决策（规范未给） | 机制是承重的（未知 → `STYLE_NOT_FOUND` / `OUT_OF_RANGE`），表的内容可替换：接线 `yeban-theory`/`yeban-services` 的预设库时只改 `domain/section.rs` 的常量 |
| **needs-7** | 工程文件的**扩展名/锁文件命名** | 本地决策 | 锁文件 = `<工程文件名>.lock`（`demo.yeban` → `demo.yeban.lock`）。规范 §0.2 只写 `.yeban.lock` 这个后缀 |

### pending

| # | 项 | 状态 |
| :--- | :--- | :--- |
| P1（承接 `mcp-core` 台账） | 十工具领域实现未接线 | **本轮关闭**（唯一残留 = 渲染器本体，见 `MCP-TOOL-008`） |
| P2（承接） | 冷启动 ≤ 20ms 未测量 | **仍 pending**（本线不新增打点） |
| P4 / P5 / P10（承接） | ADR-0001 D25 的两处取舍（联集 20 值、4 个 schema 原有码是否全留）待人类追认 | **仍 pending** |
| P6（承接） | `.yeban.lock` 的 OS 建议锁 / 心跳 / 陈旧锁抢占 | **仍 pending**（本线把"原子创建 + 存在即拒"做实了，见 boundary-3） |
| P7（承接） | 形态 A 内嵌进 `yeban-app` | **能力已就绪、仍未接线**：本线新增 `Dispatcher::domain_mut()` 与 `Domain::open_in_memory()`，app 侧只要注入自己的 `YebanProjectV1` 即可（含 `Domain::set_now_ms` 注入时钟） |

### TODO(hoist)

**没有。** 本线**没有**新增任何第三方依赖：只用根 `[workspace.dependencies]` 里早已登记的
`yeban-model`（它自己用 `serde`/`serde_json`/`sha2`/`thiserror`/`ulid`）。
`tempfile` 也**没有**引入 —— 端到端判据用 `std::env::temp_dir()` 下的唯一子目录 + `Drop` 清理。

---

## 6. 一句话总结

> 上一线留下的 **P1** 关闭：十个工具现在**真的做事**。`dryRun` 不改状态由**借用检查器**
> （`plan(&Domain)`）保证、由**逐字节 + 提交数**判据证明；幂等由**管线顺序**
> （缓存查询在执行之前）保证、由"提案数/提交数/工程字节只前进一次"证明；
> 原子保存由"只读目录下失败且原文件逐字节不变"证明；逆操作**只有一份事实源**
> （`yeban-model` 的 `Op::invert`）。四条注入各自让对应的判据变红，还原后全绿。
> 明确没做的一半只有一处 —— `yeban_render_master` 的渲染本体，且它在**参数校验通过之后**
> 才返回 `-32005`。实测出的三处**规范/模型层缺口**（`Op` 全集缺 4 个变体、`NoteOp` 无契约、
> `CommitGraph` 缺分支/多父 API）已登记为 needs，本线**不改**规范与 schema。
