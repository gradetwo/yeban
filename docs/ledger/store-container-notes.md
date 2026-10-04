# `store-container` 工作线台账：把 `.yeban` 归档容器接进 `yeban-mcp` 的保存/加载

- **台账类型**：交付映射 / 格式对照 / 错误映射表 / 判据证据 / 未决项（**不是规范**）
- **工作线**：`line/store-container`（worktree `yeban/.worktrees/store-container`，基线 `0901dcd`）
- **所有者目录**：`crates/yeban-mcp/**`（本文件是唯一新增的文档）
- **规范来源**：
  - [`../YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`](../YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md)
    §5.3 `[ARCH-SEC-003]`（ZIP 容器 `project.json` + `history.dag` + `assets/{sha256}`）、
    §5.3 `[ARCH-SEC-004]`（原子落盘）、§0.2 `[ARCH-SEC-001]`（`.yeban.lock`）、
    §6.1/§6.2 `[ARCH-OPS-002]`（提交图谱）、§5.2 `[ARCH-DET-001]`、§7.1/§7.2 `[MCP-TOOL-001/002]`
  - [`../YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`](../YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md)
    `[MUST-GATE-006]`（Zip-Slip）、`[MUST-GATE-007]`（解压炸弹）、`[MUST-GATE-008]`（建议锁）
  - [`../adr/ADR-0001-workspace-topology-and-version-pinning.md`](../adr/ADR-0001-workspace-topology-and-version-pinning.md)
    **D25**（工具级错误码联集 20 值，唯一来源）、**D30**（容器裁决与"下一个能力切片"）、**D31**（锁）
  - 前置台账：[`container-notes.md`](container-notes.md)（字节层）、
    [`tools-domain-notes.md`](tools-domain-notes.md)（保存/打开的既有语义与 `needs-3`）、
    [`lock-advisory-notes.md`](lock-advisory-notes.md)（锁语义）

> 本文件回答六个问题：**改了哪些文件对应哪条规范**、**落盘格式改动前/后长什么样**、
> **容器错误怎么映射进契约错误码**、**裸 JSON 兼容路径何时可以删**、
> **每条判据怎么变红（含注入记录）**、**哪些东西明确没做**。

---

## 0. 一句话结论

`yeban_save_project` 现在写出的是 **`ARCH-SEC-003` 的 ZIP 容器**（前 4 字节 `PK\x03\x04`，
条目 = `project.json` + `history.dag` + `assets/{sha256}`），
`yeban_open_project` 按 **ZIP 魔数**判定并读回容器（`history.dag` 与 CAS 资产池都恢复），
**`ARCH-SEC-004` 的三阶段原子落盘一字未改**（唯一入口 `store::write_project_atomic`）。
容器线的 **`needs-2`（"把 MCP 的读写换成容器 API"）关闭**；裸 JSON 只保留为**有删除条件的读兼容路径**。
16 条新判据全部本机真跑，5 次注入中 **4 次让判据变红**、1 次（去掉 `fsync`）**如实记录为"本机不可观测"**。

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件 | 规范 ID | 本轮的改动 |
| :--- | :--- | :--- |
| [`../../crates/yeban-mcp/src/domain/store.rs`](../../crates/yeban-mcp/src/domain/store.rs) | `ARCH-SEC-003`、`ARCH-SEC-004`、`ARCH-OPS-002`、`MUST-GATE-006/007` | `load_project`（ZIP 魔数判定：容器优先 / 裸 JSON 兼容）、`ProjectFormat`、`LoadedProject`、`container_bytes`（工程 + DAG + CAS 池 → 容器字节）、`ContainerRejection` 五分类 + `container_fault`（45 个容器错误 → `IO_ERROR` 的唯一映射）、`write_project_atomic` 入参 `&str → &[u8]`（协议不变） |
| [`../../crates/yeban-mcp/src/domain/mod.rs`](../../crates/yeban-mcp/src/domain/mod.rs) | `MCP-TOOL-001/002`、`ARCH-OPS-002`、`ARCH-SEC-003` | `Active` 增会话 CAS 池 `assets: BTreeMap<AssetHash, Vec<u8>>`；`Plan::Open(Box<OpenRequest>)` 携带形态/历史/资产；`Plan::Save.bytes: Vec<u8>`（容器字节）；`reset_history(SessionSeed)` 恢复 `history.dag`；`Domain::{put_asset, asset, asset_hashes, asset_count}`；`Plan::planned_commit_count`（预览不再用 `+1` 近似历史恢复） |
| [`../../crates/yeban-mcp/tests/container_store.rs`](../../crates/yeban-mcp/tests/container_store.rs) | `ARCH-SEC-003/004`、`ARCH-OPS-002`、`MUST-GATE-006/007`、`MODEL-AST-007` | **16 条端到端判据**（§5） |
| [`../../crates/yeban-mcp/tests/tools_e2e.rs`](../../crates/yeban-mcp/tests/tools_e2e.rs) | `ARCH-SEC-004`、`MCP-TOOL-002` | **2 条既有判据按新语义改写**（见 §2.6，公开改写、不是静默变绿） |

**改动到的共享文件**：**无**。根 `Cargo.toml`、`.github/**`、`scripts/**`、`deny.toml`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`、其它 `crates/**`、
`spikes/**`、法务文件全部未动。**`docs/ledger/gate-status.md` 也未改**：本轮没有让任何
`MUST-GATE-*` 的状态发生变化（`MUST-GATE-006/007` 在容器线就已 `已接线`，`ARCH-SEC-004` 不在该表内）。

**依赖图**：**零变化**（没有新增/删除任何依赖边）。因此 `Cargo.lock`、
`docs/ledger/dependency-licenses.md` 都不需要再生成 —— 这也与 `git status` 一致。

---

## 2. 格式：改动前 / 改动后对照

### 2.1 保存（`yeban_save_project`）

| | 改动前 | 改动后 |
| :--- | :--- | :--- |
| 落盘字节 | `serialize_project(project)`：**美化 JSON + 结尾换行**（UTF-8 文本） | `write_project_container(project, history.dag, assets)`：**标准 ZIP（只写 `stored`）** |
| 前 4 字节 | `{` 或 `\n`（视美化格式） | `PK\x03\x04`（`store::CONTAINER_MAGIC`） |
| 提交图谱 | **不落盘**（打开即重建一条根提交） | `history.dag` = `serde_json::to_vec(CommitGraph)`，打开时恢复 |
| CAS 资产 | **不落盘**（裸 JSON 里只有 `project.assets` 这份元数据索引） | `assets/{sha256}` 逐条写出（条目名 = 字节的 SHA-256，写出前重算校验） |
| 原子落盘 | 同目录 `.name.tmp-{ulid}` + `create_new` + `write_all` + `sync_all` + `rename` | **一字未改**，只是把"写什么字节"从 `&str` 换成 `&[u8]` |
| 响应新增字段 | — | `format: "yeban-container"`、`assets`、`historyCommits` |
| `plan_save` 预览新增 | — | `format`、`containerEntries`、`assets`、`historyCommits`，`bytes` 语义变为**容器字节数** |

### 2.2 加载（`yeban_open_project`）

```text
load_project(path)
  ├─ bytes.starts_with(b"PK\x03\x04")  ⇒ read_project_container(bytes, ContainerLimits::default())
  │                                       ⇒ check_readable() → validate() → decode_history_dag()
  └─ 否则                               ⇒ UTF-8 → serde_json → check_readable() → validate()
```

| | 改动前 | 改动后 |
| :--- | :--- | :--- |
| 判定方式 | 只有一条路径：UTF-8 + JSON | **ZIP 魔数**（4 字节）决定走哪条；扩展名/大小都不参与 |
| 版本门 + `validate()` | `check_readable()` → `validate()` | **同一对调用、同一顺序、同一错误映射**（兼容 ≠ 放宽） |
| 容器安全 | 不适用 | `MUST-GATE-006/007` 在**内容解读之前**生效（模型层的判定顺序即契约） |
| 响应新增字段 | — | `format`、`historyRestored`、`historyCommits`、`assets` |

### 2.3 `history.dag` 的口径（本线的裁决）

`history.dag` = `serde_json::to_vec(&CommitGraph)`（**紧凑 JSON**）。
依据：模型层 [`CommitGraph`](../../crates/yeban-model/src/commit.rs) **已经是**
`Serialize + Deserialize`，而且它内部的三个集合全是 `BTreeMap`（`ARCH-OPS-002` / `MODEL-AST-003`），
因此序列化字节本身是确定的。**不另造一套私有格式** —— 两份格式必然漂移，
而"漂移"在这种"打开旧工程"的路径上表现为**静默丢历史**。

读取侧的三条裁决（判据 16 钉住）：

| 输入 | 行为 | 为什么 |
| :--- | :--- | :--- |
| 空条目 / 零提交图谱 | 视为"没有历史" ⇒ 建一条根提交 | 与"首次打开"语义一致 |
| **合法 JSON 但没有 `main` 分支** | `IO_ERROR`（打开即拒绝） | 这种"半个历史"会让后续**每一次** `yeban_propose_*` 撞 `CONFLICT`；在打开处拒绝比留给下一个工具踩更诚实 |
| **非法 JSON** | `IO_ERROR`（不静默忽略） | 忽略等于把用户的历史悄悄丢掉 |

### 2.4 `assets/{sha256}` 的口径（本线的裁决）

| 环节 | 行为 |
| :--- | :--- |
| 放资产 | `Domain::put_asset(bytes)` —— SHA-256 由**字节算出**（不接受调用方声明的哈希），返回 [`AssetHash`] |
| 保存 | 会话 CAS 池（`BTreeMap<AssetHash, Vec<u8>>`，键序 = 哈希升序）整体交给 `write_project_container`，它**逐条重算** SHA-256 并要求等于条目名 |
| 打开 | `read_project_container` 已经**重算**每个 `assets/*` 的 SHA-256；本层再把它们装回会话池 |
| 再保存 | 池原样写回（判据 9 的第三段：不是"读进来但写不回去"） |

**与 `project.assets`（元数据索引）的关系（boundary-1）**：`YebanProjectV1.assets` 是
`BTreeMap<AssetHash, AssetMetadata>`（只有 `original_path`/`byte_len`/`license` 等**元数据**，
**没有字节**）。容器里的 `assets/{sha256}` 是**字节**。本线的裁决是：
**会话 CAS 池是"容器里有什么"的唯一事实源**；两者的一致性（"索引里的每个 hash 都有字节"、
"没有多余字节"）**本轮不做交叉校验**。理由：模型层没有 blob 存储，MCP 层也不该凭空造一个；
真正的资产落盘属于应用层/资产库，接线时必须补一条"索引 ↔ 池"的对账判据。

### 2.5 内容摘要与落盘字节**解耦**（有意为之）

- `projectDigest` / `saved_digest` / 未保存标记：仍然是
  **规范化 JSON**（`serialize_project`：美化 + 结尾换行）的 SHA-256；
- 落盘字节：容器（`project.json` 里是**紧凑** JSON）。

因此同一个工程，`projectDigest` 与 `project.json` 的 SHA-256 **不相等**，这是**口径不同**而不是 bug。
好处：`changed`（未保存标记）、`dryRun` 预览、幂等比较这些"工程内容"语义没有跟着落盘格式改；
"容器字节变了"与"工程内容变了"可以分别讨论。

### 2.6 被改写的既有判据（**公开改写，不是静默变绿**）

保存格式从"裸 JSON"变成"容器"**必然**让两条断言"磁盘文本 == 内存工程的美化 JSON"的判据失效。
它们被**明确改写**（并在原地留注释指向容器判据），不是被绕过：

| 判据 | 原断言 | 新断言 |
| :--- | :--- | :--- |
| `tools_e2e.rs::save_into_a_read_only_directory_fails_with_io_error_and_keeps_the_original` | 保存成功后 `fs::read_to_string(path) == project_bytes(domain)` | 保存成功 ⇒ `format == "yeban-container"`、磁盘字节 ≠ 原裸 JSON、**前 4 字节是 `PK\x03\x04``；"逐字节相同"的往返由 `container_store.rs` 判据 2 承担 |
| `tools_e2e.rs::open_save_close_round_trip_preserves_bytes_on_disk` | 同上 | 保存后磁盘字节 ≠ 原裸 JSON 且以 ZIP 魔数开头；完整容器往返在判据 2/9 |

其余 23 条 `tools_e2e` 判据**未改**（它们的夹具是裸 JSON ⇒ 走兼容路径，语义不变）。

---

## 3. 容器错误 → 契约错误码（**只用一个已存在的码**）

`ADR-0001 D25` 的 20 值联集是**工具级错误码的唯一来源**。容器层
[`ContainerError`](../../crates/yeban-model/src/container/mod.rs) 的 **45 个变体**（实测计数；
`container-notes.md` §1 里写的 "38 个" 是**该文档的陈旧计数**，不是代码缺陷）
**全部**映射到 `ErrorCode::IoError`（`IO_ERROR`），分类与规范 ID 进 `error.data`：

| 分类 `data.category` | 规范 ID `data.specId` | 覆盖的 `ContainerError` 变体（45 个，穷举匹配、无 `_` 兜底） |
| :--- | :--- | :--- |
| `path-traversal` | `MUST-GATE-006` | `EmptyEntryName`、`EntryNameTooLong`、`AbsoluteEntryPath`、`ParentDirSegment`、`CurrentDirSegment`、`EmptyPathSegment`、`DirectoryEntryUnsupported`、`BackslashInEntryName`、`ColonInEntryName`、`NulInEntryName`、`ControlCharInEntryName`、`TrailingDotOrSpaceSegment`、`WindowsReservedName`、`DuplicateEntryName`、`SymlinkEntryUnsupported`（15） |
| `archive-bomb` | `MUST-GATE-007` | `EntryTooLarge`、`EntryActualTooLarge`、`ArchiveTooLarge`、`ExpansionRatioExceeded`、`TooManyEntries`（5） |
| `unsupported-container-feature` | `ARCH-SEC-003` | `UnsupportedMultiDisk`、`UnsupportedZip64`、`UnsupportedCompression`、`UnsupportedDataDescriptor`、`EncryptedEntryUnsupported`（5） |
| `malformed-container` | `ARCH-SEC-003` | `EocdNotFound`、`TruncatedArchive`、`CentralDirectoryOutOfBounds`、`CentralDirectorySizeMismatch`、`TruncatedCentralDirectory`、`BadCentralDirectorySignature`、`EntryNameNotUtf8`、`LocalHeaderOutOfBounds`、`BadLocalHeaderSignature`、`LocalCentralMismatch`、`TruncatedEntryData`、`StoredSizeMismatch`、`CrcMismatch`（13） |
| `container-layout` | `ARCH-SEC-003` | `MissingProjectJson`、`MissingHistoryDag`、`InvalidProjectJson`、`UnexpectedContainerEntry`、`InvalidAssetName`、`AssetHashMismatch`、`ContainerSerialization`（7） |

**为什么一律 `IO_ERROR` 而不是"更贴切"的码**（例如 `PERMISSION_DENIED`）：
契约里**没有**"Zip-Slip"/"解压炸弹"/"不支持的压缩法"这些**容器内部**的码，也不该有
（`D25` 的联集是**工具级领域码**）。把归档问题伪装成领域语义会让调用方按领域语义重试一件
永远不会成功的事。**诊断信息走 `data`，不挤进 `code`**：

```json
{"error":{"code":"IO_ERROR","message":"`/tmp/x.yeban` 不是可接受的 .yeban 容器 [path-traversal / MUST-GATE-006]: ...",
          "data":{"category":"path-traversal","specId":"MUST-GATE-006",
                  "containerError":"ParentDirSegment { name: \"../../xy\" }","path":"/tmp/x.yeban"}}}
```

`store::container_rejection` 是**穷举** `match`（没有 `_`）：容器线若新增一个变体，
**本 crate 编译失败**，强迫做一次显式分类决定 —— 与 `error::code_for_model` 同一条纪律。

---

## 4. 裸 JSON 兼容路径：策略与**删除条件**

### 4.1 策略

| 问题 | 裁决 |
| :--- | :--- |
| 读 | **兼容**：没有 ZIP 魔数就按"UTF-8 + `serde_json` + 版本门 + `validate()`"读。响应里 `format: "bare-json"` **如实披露**，调用方永远知道自己在兼容路径上 |
| 写 | **不兼容**：保存一律产出容器（判据 7 钉住："写一律产出容器，读才需要兼容"） |
| 为什么保留 | 本仓库**全部既有夹具**（`tools_e2e.rs`、`lock_advisory.rs`、`samples` 导出探针）与任何"容器接线之前写出的 `.yeban`"都是裸 JSON。直接不兼容 = 一次性废掉既有判据与用户文件，而"读旧、写新"是零代价的迁移路径 |
| 为什么不是永久承诺 | 兼容路径是**双份解析逻辑**（两份失败模式、两份错误消息）；它只该活到"没有裸 JSON 输入"为止 |

### 4.2 删除条件（**三条同时满足**才可删）

1. **没有生产者**：仓库里不再有任何测试/脚本/示例**写出**裸 JSON 工程文件
   （`grep -rn "to_string_pretty(&project)" crates/yeban-mcp/tests` 与
   `crates/yeban-model/src/samples.rs` 的导出路径都已改成容器或删掉）；
2. **没有消费者**：`schemas/project.schema.json` 仍然是工程文档契约（它不会变），
   但**不再有**"外部工具直接写裸 JSON 给 MCP 打开"的用法（需要人类确认；
   `tools-domain-notes.md` 的 `needs-3` 把这条记为分工问题）；
3. **有一次迁移窗口**：至少一个发布周期里，打开裸 JSON 时返回**显式的迁移提示**
   （现在只有 `format: "bare-json"`，没有警告级提示）。

删的时候要一起删：`store::load_bare_json`、`ProjectFormat::BareJson`、
`tests/container_store.rs::bare_json_projects_still_open_on_the_compat_path`、
`tools_e2e.rs` 的裸 JSON 夹具（改成 `store::container_bytes` 写夹具），
以及本节的"删除条件"本身。

---

## 5. 判据清单（16 条）与注入实验（5 次）

### 5.1 判据（全部在 [`tests/container_store.rs`](../../crates/yeban-mcp/tests/container_store.rs)）

| # | 判据 | 钉住的事实 |
| :--- | :--- | :--- |
| 1 | `save_writes_a_standard_container_with_the_zip_magic` | 前 4 字节 `PK\x03\x04`；模型层容器读取器解开恰好 `project.json` + `history.dag` |
| 2 | `save_then_open_round_trips_the_project_byte_for_byte` | 保存 → 关闭 → 重开 ⇒ 工程**逐字节**相同（规范化 JSON）+ 摘要相同 |
| 3 | `save_then_open_restores_the_commit_graph_from_history_dag` | 3 条提交（根/提案/合并）与主分支头跨打开存活 |
| 4 | `two_forced_saves_of_the_same_session_are_byte_identical` | 同输入两次保存字节相同（`ARCH-DET-001`） |
| 5 | `save_into_a_read_only_directory_keeps_the_original_container_bytes` | 失败 ⇒ `IO_ERROR`、原容器**逐字节不变**、无 `.tmp-` 残留；恢复权限后成功 |
| 6 | `save_replaces_the_target_inode_instead_of_truncating_it` | 落盘是"新文件 + `rename`"（inode 变化），不是原地截断写 |
| 7 | `bare_json_projects_still_open_on_the_compat_path` | 兼容路径活着且**披露形态**；写一律产出容器 |
| 8 | `container_project_json_is_accepted_by_the_project_schema` | 从磁盘容器里**原样**取出 `project.json`，交给 `validate_schemas.py --samples-dir`（Python jsonschema **独立实现**）通过；并断言脚本**真的**校验了这一份（防空转） |
| 9 | `assets_round_trip_with_content_addressing` | `assets/{sha256}` 条目名 = 字节 SHA-256；模型层 `read_project_container` 再验一遍；关闭重开后池一致；再保存仍写回 |
| 10 | `truncated_container_is_refused_instead_of_loading_half_a_project` | 截断 ⇒ `IO_ERROR`，不留下活跃工程 |
| 11 | `crc_tampered_container_is_refused_instead_of_loading_it` | 篡改数据字节 ⇒ `IO_ERROR` + `malformed-container` |
| 12 | `zip_slip_entry_name_is_refused_with_a_contract_error_code` | 合法容器里把条目名换成 `../../xy`（local + central 两处）⇒ `IO_ERROR` + `path-traversal` |
| 13 | `unsupported_compression_is_refused_with_a_contract_error_code` | 压缩法改成 `8`(deflate) ⇒ `IO_ERROR` + `unsupported-container-feature`，诊断指名 `UnsupportedCompression` |
| 14 | `container_without_history_dag_is_refused_as_a_layout_error` | 缺 `history.dag` ⇒ `IO_ERROR` + `container-layout` |
| 15 | `every_container_rejection_stays_inside_the_contract_enum` | 5 类畸形容器逐个打开，错误码**全部**在 `ErrorCode::SCHEMA_CONTRACT`（20 值）内 |
| 16 | `corrupt_history_dag_is_refused_instead_of_silently_dropping_history` | 坏 `history.dag` JSON / 无 `main` 分支 ⇒ 打开即 `IO_ERROR` |

**判据 8 的独立核验**：另外手工验证过"这份脚本真的会红" ——
把一份 `{"schema_version":1,"bpm":1.0}` 当 `project.bogus.json` 交给同一命令，
脚本 `exit 1` 并逐条列出缺失的必填键。⇒ 判据 8 的"绿"不是空转。

### 5.2 注入实验（**5 次；4 次变红、1 次如实记为不可观测**）

方法：`src/domain/{store,mod}.rs` 备份到 `/tmp/store-container-backup/`，
用 Python 定点替换（每次 `assert t.count(old) == 1`，防止注入无效 —— L3/L24 的纪律），
跑 `cargo-local.sh test -p yeban-mcp`，记录红掉的判据名，再从备份还原并校验 SHA-256 一致。

| # | 注入 | 实测变红的判据 | 还原 |
| :--- | :--- | :--- | :--- |
| **A** | `write_then_replace` 改成**原地截断写**（丢掉 tmp+rename） | `save_replaces_the_target_inode_instead_of_truncating_it`（inode 相同：`left: 190168577 right: 190168577`）、`save_into_a_read_only_directory_keeps_the_original_container_bytes`（只读目录下保存**居然成功**，`status:"success"` 而判据要求 `error`）→ `14 passed; 2 failed` | ✅ 16/16 |
| **B** | 保存字节从容器换回**裸 JSON**（`plan_save` 改调 `serialize_project`） | 7 条：`save_writes_a_standard_container_with_the_zip_magic`、`save_then_open_round_trips_the_project_byte_for_byte`、`save_then_open_restores_the_commit_graph_from_history_dag`、`bare_json_projects_still_open_on_the_compat_path`、`container_project_json_is_accepted_by_the_project_schema`、`assets_round_trip_with_content_addressing`、`save_into_a_read_only_directory_keeps_the_original_container_bytes` → `9 passed; 7 failed` | ✅ 16/16 |
| **C** | `decode_history_dag` 把 JSON 解析错误**吞掉**（当作"没有历史"） | `corrupt_history_dag_is_refused_instead_of_silently_dropping_history` → `15 passed; 1 failed` | ✅ 16/16 |
| **D** | `write_then_replace` 里**去掉 `file.sync_all()?`** | **零条变红**（`155 + 16 + 15 + 13 + 25` 全绿）——**如实记录**：断电/崩溃后的持久性在进程内**不可观测**，见 §9 boundary-3 | ✅ 16/16 |
| **E** | `looks_like_container` 恒返回 `false`（永远走裸 JSON 兼容路径） | 9 条：`corrupt_history_dag_*`、`crc_tampered_*`、`container_without_history_dag_*`、`assets_round_trip_*`、`unsupported_compression_*`、`zip_slip_*`、`bare_json_projects_*`、`save_then_open_round_trips_*`、`save_then_open_restores_*` → `7 passed; 9 failed` | ✅ 16/16 |

还原校验：

```text
$ shasum -a 256 crates/yeban-mcp/src/domain/{store,mod}.rs /tmp/store-container-backup/*.rs
9ae69f2f6ca3c864ed0312a62b5cd2a900408a3fa2e8b9cfe92317fe0168a705  crates/yeban-mcp/src/domain/store.rs
7852d17a08a6841e7942222f505b0b6e4bf10c71237b99f63add97a414bc7a52  crates/yeban-mcp/src/domain/mod.rs
7852d17a08a6841e7942222f505b0b6e4bf10c71237b99f63add97a414bc7a52  /tmp/store-container-backup/mod.rs
9ae69f2f6ca3c864ed0312a62b5cd2a900408a3fa2e8b9cfe92317fe0168a705  /tmp/store-container-backup/store.rs
```

---

## 6. 实测证据（不是"我实现了"，是"这个命令输出了什么"）

### 6.1 保存 → 打开往返 + 资产哈希（端到端，实测）

探针（真实的 `Domain::open_in_memory` → `put_asset` → `domain::execute("yeban_save_project")`
→ 真文件系统；探针源码用后即删，不留在仓库里）：

```text
{"data":{"assets":1,"atomic":true,"bytes":9931,"forced":true,"format":"yeban-container",
         "historyCommits":1,"path":".../yeban-container-probe/probe.yeban",
         "projectDigest":"ea132d8f425207f0388fe79867d41bf10fa7941efca6ebb0a485f09cfbb8c1bd",
         "saved":true,"skipped":false},"status":"success"}
BYTES=9931  MAGIC=[50, 4B, 03, 04]
ASSET=c8f5d0341d54d951a71b136e6e2afcb14d11ed8489a7ae126a8fee0df6ecf193
ASSET_SHA=c8f5d0341d54d951a71b136e6e2afcb14d11ed8489a7ae126a8fee0df6ecf193
```

### 6.2 第三方工具（`unzip` 6.00 / Info-ZIP Apple 修改版）**真的**读得出来

```text
$ unzip -l probe.yeban
  Length      Date    Time    Name
---------  ---------- -----   ----
     4903  01-01-1980 00:00   project.json
      494  01-01-1980 00:00   history.dag
     4096  01-01-1980 00:00   assets/c8f5d0341d54d951a71b136e6e2afcb14d11ed8489a7ae126a8fee0df6ecf193
---------                     -------
     9493                     3 files

$ unzip -t probe.yeban
    testing: project.json             OK
    testing: history.dag              OK
    testing: assets/c8f5d034…f193     OK
No errors detected in compressed data of probe.yeban.

$ unzip -p probe.yeban assets/c8f5d034…f193 | shasum -a 256
c8f5d0341d54d951a71b136e6e2afcb14d11ed8489a7ae126a8fee0df6ecf193  -
```

**最后一行是本线最强的正向证据**：`unzip` 解出的资产字节，其 SHA-256 **等于条目名里携带的 CAS 键**
（`MUST-GATE-006/007` 之外，`MODEL-AST-007` 的内容寻址由**第三方工具**端到端验证）。

`history.dag` 的内容（真实字节，截断）：

```text
{"commits":{"01M44CHZ0NSZF93VAGDNZMXBE3":{"id":"01M44CHZ0NSZF93VAGDNZMXBE3","parents":[],
"branch_id":"main","author":"yeban-mcp","message":"open /var/folders/…/probe.yeban",…}},…}
```

### 6.3 本机真跑（Apple M2，`run-gates.sh crate yeban-mcp` **不是 SKIP**）

```text
$ bash scripts/gates/run-gates.sh crate yeban-mcp          # exit 0
  lib          155 passed   (基线 152 → +3: store.rs 的容器往返/兼容/截断/分类判据)
  container_store 16 passed (本轮新增)
  contract        15 passed
  lock_advisory   13 passed
  tools_e2e       25 passed (其中 2 条按新语义改写, 见 §2.6)
  合计 224 条
$ bash scripts/dev/cargo-local.sh clippy -p yeban-mcp --all-targets -- -D warnings   # exit 0
$ bash scripts/dev/cargo-local.sh fmt -p yeban-mcp --check                          # exit 0
$ python3 scripts/gates/validate_schemas.py --samples-dir <容器里的 project.json>    # exit 0（判据 8 内置）
```

### 6.4 未在本机跑（交给 CI）

- `rust (workspace 全量)`：`--workspace` 被 `cargo-local.sh` 直接拒绝（本机纪律）。
- `cargo deny` / `lockfile`：本线**没有**改依赖图，理论上不受影响；判决仍以 CI 为准。
- 跨架构/基准/fuzz：本线**没有**新增打点，不涉及。

---

## 7. CI 判决

- 分支：`line/store-container`
- 读取方式：`bash scripts/dev/ci-verdict.sh line/store-container`（**会校验判决 SHA == 分支 tip**）

| 轮次 | run id | 头部 | 结论 |
| :--- | ---: | :--- | :--- |
| 第 1 轮（容器接线 + 16 条判据 + 5 次注入） | 见提交信息 / `ci-verdict.sh` 读数 | 见下 | 见下 |

> 本文件自身是**文档改动**：记录判决的这一次提交会再前进一格。它只改
> `docs/ledger/store-container-notes.md`，不触碰任何 `crates/**`，因此不影响 §6 的任何判据；
> 但按纪律仍然读回判决。

---

## 8. 边界 / needs / pending / TODO(hoist)

### 已知边界（**不是**缺陷，是本线明确没证明的事）

| # | 边界 | 说明 |
| :--- | :--- | :--- |
| boundary-1 | `project.assets`（元数据索引）与容器 `assets/{sha256}`（字节）**不做交叉校验** | 见 §2.4。模型层没有 blob 存储；会话 CAS 池是"容器里有什么"的唯一事实源。真资产库接线时必须补"索引 ↔ 池"对账判据 |
| boundary-2 | 跨进程/跨会话的**字节确定性**不成立 | 提交身份是 `EntityId::new()`（ULID，随机，`CommitDraft` 由调用方提供 id 是**有意**的设计）。判据 4 因此断言的是**同一会话内**两次落盘逐字节相同；容器写入器本身的确定性由容器线的 `write_is_deterministic` 承担 |
| boundary-3 | **`fsync` 的移除在本机不可观测**（注入 D 零变红） | 断电/崩溃后的持久性是**进程外**性质，`std::fs` 没有可注入的 fsync 探针，本机与 CI 都无法用判据钉住"真的 fsync 了"。本线的证据是"三阶段协议**只有一个**入口 `write_project_atomic`，且它在 `rename` 之前调用 `sync_all`"这条结构事实 + 代码审查。要机械钉住需要 `strace`/`dtruss` 级别的系统调用观测（登记为 pending） |
| boundary-4 | 只测到"资产 = 4 KiB 随机字节" | 没有测大资产（GB 级）与真实音频（FLAC/WAV）。容器线的上限判据用"声明 2 GB + 实际小字节"钉住阈值本身 |
| boundary-5 | `put_asset` 只进**内存**池 | 没有磁盘级 CAS（`assets/{sha256}` 落盘池）、没有 GC/去重策略、没有"引用计数"；一次会话里放进池但从不被工程引用的资产**照样会被写进容器**（池是权威） |
| boundary-6 | 兼容路径读裸 JSON 时**没有**迁移提示（只有 `format: "bare-json"`） | 见 §4.2 的删除条件 3 |
| boundary-7 | 打开容器时**不恢复**提案表（`proposals`） | `Proposal` 记录是 MCP 会话态（`ARCH-OPS-002` 的 DAG 才是持久化层）；容器里只有 `history.dag`。`ai/proposal-*` 分支与提交都在 DAG 里，记录不在 |

### needs（需要裁决 / 需要别的所有者接线）

| # | 项 | 性质 | 建议 |
| :--- | :--- | :--- | :--- |
| needs-1 | 容器里的**资产索引 ↔ 池**一致性（boundary-1） | 跨层缺口 | 资产库/应用层落地后，在 `load_project` / `container_bytes` 加双向对账（含"索引里有但容器没有"与"容器有但索引没有"两侧），并同步 `assets.manifest.schema.json` 的口径 |
| needs-2 | 裸 JSON 兼容路径的**删除时点**（§4.2 三条） | 需要人类裁决 | 建议在 v1.0 冻结后一个发布周期内删除，并把它写成 ADR |
| needs-3 | `container-notes.md` §1 的 "38 个变体"应为 **45** | 文档陈旧 | 该文件属容器线/集成者；本线不擅改，只在此登记实测计数 |
| needs-4 | `history.dag` 是否需要**版本字段** | 规范缺口 | 现在是裸 `CommitGraph` JSON；`Op`/`Commit` 变体扩张时旧 `history.dag` 的可读性靠 `serde` 默认值。建议将来加一层 `{"version":1,"graph":{…}}` 信封（会让本线的判据 3/16 需要同步） |

### pending

| # | 项 | 状态 |
| :--- | :--- | :--- |
| P1 | 容器线的 `needs-2`（MCP 读写换成容器 API） | **本轮关闭** |
| P2 | `fsync` 的机械观测（boundary-3） | **仍 pending**（需要系统调用级追踪；本机与 CI 都不做） |
| P3 | 真资产库（磁盘 CAS）接线 | **仍 pending**（needs-1 / boundary-5） |
| P4 | `tools-domain-notes.md` 的 `boundary-4`（"工程文件是裸 JSON"） | **本轮过时**：写已经是容器；读仍兼容裸 JSON。读该台账时以本文件为准 |

### TODO(hoist)

**没有。** 本线**没有**新增任何第三方依赖：容器读写住在 `yeban-model`
（零新增依赖的手写 ZIP 子集），`yeban-mcp` 只是调用它；仍未引入 `tempfile`/`zip`/`flate2`。

---

## 9. 一句话总结

> `yeban_save_project` 写出的字节现在是 **`ARCH-SEC-003` 的 ZIP 容器**（`unzip -l/-t/-p` 本机实测可读，
> 资产 SHA-256 == 条目名），`yeban_open_project` 按 **ZIP 魔数**判定并恢复 `history.dag` 与 CAS 资产池；
> **`ARCH-SEC-004` 的三阶段原子落盘一字未改**，并由"只读目录下失败且原容器逐字节不变"+
> "inode 变化"两条判据证明。容器层的 45 个错误**全部**映射到 `IO_ERROR`（`D25` 联集内），
> 分类与规范 ID 进 `data`，**不发明新码**。裸 JSON 只保留为**有删除条件**的读兼容路径。
> 5 次注入里 4 次让判据变红（原地写 2 条 / 裸 JSON 7 条 / 吞掉 history 错误 1 条 / 关掉容器分派 9 条），
> 第 5 次（去掉 `fsync`）**零变红**，如实登记为本机不可观测的边界。

---

## 10. 本线修改/新增文件的绝对路径清单

```text
/Users/crow/work/music/yeban/.worktrees/store-container/crates/yeban-mcp/src/domain/store.rs            (修改)
/Users/crow/work/music/yeban/.worktrees/store-container/crates/yeban-mcp/src/domain/mod.rs              (修改)
/Users/crow/work/music/yeban/.worktrees/store-container/crates/yeban-mcp/tests/container_store.rs       (新增, 16 条判据)
/Users/crow/work/music/yeban/.worktrees/store-container/crates/yeban-mcp/tests/tools_e2e.rs             (修改: 2 条判据按新语义改写)
/Users/crow/work/music/yeban/.worktrees/store-container/docs/ledger/store-container-notes.md            (本文件)
```
