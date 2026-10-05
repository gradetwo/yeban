# mcp-no-compat 工作线台账 —— 删除 `yeban-mcp` 的裸 JSON 兼容读路径（`.yeban` 容器是**唯一**工程格式）

- **台账类型**：删除清单 / 判据反转记录 / 注入记录 / 本机真跑与 CI 严格区分 / 能力矩阵处理 / 边界与 needs（**不是规范**）
- **工作线**：`line/mcp-no-compat`（worktree `yeban/.worktrees/mcp-no-compat`，基线 main `42442af`）
- **日期**：2026-10-04
- **授权**：`docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` **D43** ——
  「1.0.0 之前不存在历史包袱 / 旧版本兼容；发现问题或更优解**直接推翻**」。
  D43 第 1 条**点名**了本条路径：`yeban-mcp` 的裸 JSON 兼容读路径（`bare-json`）——
  「**凡是"为了读旧文件/旧格式"而存在的分支，一律删除**，而不是"保留 + 记删除条件"」。
- **相关规范 (Normative)**：`[ARCH-SEC-003]`（容器）、`[ARCH-SEC-004]`（原子落盘）、
  `[ARCH-OPS-002]`（提交图谱）、`[MUST-GATE-006]`（Zip-Slip）、`[MUST-GATE-007]`（解压炸弹）、
  `[MUST-GATE-008]`（建议锁）、`ADR-0001` **D25**（错误码联集 20 值，不许发明新码）、
  **D30**（容器读法歧义用"拒绝"或"明示"消除）。
- **地盘**：`crates/yeban-mcp/**` + 本台账 + `docs/ledger/**` 的相关修订。
  **没有**改根 `Cargo.toml` / `Cargo.lock`（**零新增依赖**）、`.github/**`、`scripts/**`、
  `deny.toml`、`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`、
  `spikes/**`、其它 `crates/**`、法务文件、`README*.md`。
- **与 app 侧的关系**：`yeban-app` 的同一条路径已由 `line/app-no-compat` 删除（合并于 main）。
  本线**读它的做法并保持一致**：错误命名（`NotAYebanContainer` ↔ `not_a_container_fault`）、
  报告字段（形态只剩一个常量取值）、判据**反转而不是删除**、强对照（同内容进容器仍可读）。
  见 [`app-no-compat-notes.md`](app-no-compat-notes.md)。

---

## 0. 一句话结论

`yeban-mcp` 的**裸 JSON 兼容读路径已被整段删除**（`store::load_bare_json`、`ProjectFormat` 枚举、
`looks_like_container` 的分派用途、dryRun 的 `container` 布尔）。
`.yeban` 容器现在是 MCP 侧**唯一**的工程格式：非容器文件（裸 `project.json` / 随机字节 / 空文件）
得到的是**一个精确的错误**（`IO_ERROR` + `不是 .yeban 容器` + 容器原裁决），
**不是**"打开成空工程"，**不是**泛化的"未知格式"。
容器侧**全部**安全闸门（Zip-Slip / 炸弹 / 上限 / CRC / ZIP64 / 多卷 / 缺件 / 容器内坏 JSON）
与错误分类一个都没松；`ARCH-SEC-004` 的原子落盘协议**一字未改**；`error.code` 仍只用 D25 联集里的
`IO_ERROR`，**没有新增任何错误码**。

---

## 1. 删了什么（逐项 + 净行数）

| # | 删除项 | 位置 | 为什么它"只服务兼容" |
| :-- | :--- | :--- | :--- |
| 1 | `load_bare_json`（UTF-8 → `serde_json` → 版本门 → `validate()`） | `src/domain/store.rs` | 兼容读路径**本体**（35 行函数整段消失） |
| 2 | `ProjectFormat` 枚举（`Container` / `BareJson`）与 `as_str()` / `is_container()` | `src/domain/store.rs` | 兼容路径存在时才有"形态"可选；删掉后只剩一个取值 ⇒ 替换为常量 `DOCUMENT_FORMAT`（与 app 侧同名常量一致） |
| 3 | `LoadedProject::format` 字段 | `src/domain/store.rs` | 报告读法的字段；只有一个读法 ⇒ 从结构体里去掉，响应里的 `format` 来自常量 |
| 4 | `OpenRequest::format` 字段 | `src/domain/mod.rs` | 同上（"形态"不再需要在计划里传递） |
| 5 | dryRun 预览里的 `"container": format.is_container()` 布尔 | `src/domain/mod.rs` | 形态枚举**带出来的**第二个披露字段；app 侧报告里只有 `format=` 一个字段 ⇒ 对称地删掉 |
| 6 | `looks_like_container`（`bytes.starts_with(CONTAINER_MAGIC)` 的分派用途） | `src/domain/store.rs` | 旧用途是"决定要不要掉进裸 JSON 分支"——**为兼容而写的补丁式判断**。删除分派后不能整个删掉"有没有 ZIP 结构"这个判断，因为它还有一个**与兼容无关**的用途（诊断分档），因此改名为 `has_zip_signature`（三签名）并重写文档 |
| 7 | `load_container` 的两档分派（`map_err(container_fault)` 无分档） | `src/domain/store.rs` | 改成 `has_zip_signature` 两档：有 ZIP 结构 ⇒ `container_fault`（"你的 `.yeban` 坏了"），没有 ⇒ `not_a_container_fault`（"你给的不是 `.yeban` 容器"） |
| 8 | 断言"裸 JSON **能**打开 / 披露 `bare-json`"的判据（1 条集成 + 1 条单元） | `tests/container_store.rs`、`src/domain/store.rs` | **反转**，不是删除 —— 见 §4 |
| 9 | 非容器夹具（写裸 JSON 再打开） | `tests/tools_e2e.rs`、`tests/lock_advisory.rs`、`tests/container_store.rs` | 全部改成写**真容器**（`write_project_container` + 空 `history.dag`）；否则夹具本身就成了"兼容路径的唯一消费者" |
| 10 | 模块文档的"裸 JSON 是有删除条件的兼容路径"整段 | `src/domain/store.rs`、`src/domain/mod.rs`、`src/domain/render.rs` | 换成 D43 说明 + 新的判定顺序 + `audioClips` 触发条件的收窄说明 |

### 1.1 行数（`git diff --numstat` 实测；生产代码按"去掉 `#[cfg(test)]` 段"用 diff hunk 拆分）

| 类别 | 文件 | 增 / 删 | 净值 |
| :--- | :--- | :--- | :--- |
| **生产代码** | `src/domain/store.rs` | +100 / −103 | **−3** |
| **生产代码** | `src/domain/mod.rs` | +17 / −24 | **−7** |
| **生产代码** | `src/domain/render.rs`（仅文档措辞） | +13 / −8 | +5 |
| **生产代码小计** | | +130 / −135 | **−5 行** |
| 判据（测试） | `src/domain/store.rs`（`#[cfg(test)]` 段） | +175 / −18 | +157 |
| 判据（测试） | `tests/container_store.rs` | +253 / −48 | +205 |
| 判据（测试） | `tests/tools_e2e.rs` | +41 / −28 | +13 |
| 判据（测试） | `tests/lock_advisory.rs` | +9 / −3 | +6 |
| 判据（测试） | `tests/render_audio_clips.rs`（仅措辞） | +4 / −4 | 0 |
| **判据小计** | | +482 / −101 | **+381 行** |
| 文档 | `docs/ledger/store-container-notes.md`（修订 §4 等） | +60 / −29 | +31 |
| 文档 | `docs/ledger/audio-render-notes.md`（needs-7 状态更新） | +11 / −3 | +8 |
| 文档 | `docs/ledger/mcp-render-notes.md`（一行订正） | +1 / −1 | 0 |
| 文档 | `docs/ledger/human-decisions.md`（HD-20 标"已完成"） | +1 / −1 | 0 |
| 文档 | 本台账（新增） | +358 / −0 | +358 |

**口径说明（防止误读）**：**生产代码净 −5 行**。这个数字比"感觉上删掉的量"小，
原因是同一次改动里**新写了大量文档**（模块头的 D43 判定顺序、`has_zip_signature` 的三签名理由、
`not_a_container_fault` 的"为什么不发明新码"整节），它们与被删的 60 余行代码在同一个文件里对抵。
判据与文档净增是**任务本身的要求**（判据要**反转并扩容**，notes 是新交付物）——
不是"没删掉"。全仓 `git diff --cached --stat` 见 §7 的提交前检查；**没有任何一份未打算删的
文件出现净删**（唯一的删除集中在 `src/domain/store.rs`、`src/domain/mod.rs` 与本线新增/修订的文档）。

### 1.2 **保留**的东西（D43 不解除的部分）

- 容器侧**全部**安全闸门与错误分类：Zip-Slip（`ParentDirSegment` 家族）、炸弹（`EntryTooLarge` /
  `EntryActualTooLarge` / `ArchiveTooLarge` / `ExpansionRatioExceeded` / `TooManyEntries`）、
  条目数、单条目上限、ZIP64 / 多卷 / 加密 / data descriptor、CRC、压缩法、
  缺件（`MissingProjectJson` / `MissingHistoryDag`）、**容器内**的坏 `project.json`
  （`InvalidProjectJson`）、资产哈希不符（`AssetHashMismatch`）；
- "读之前按 `metadata` 判上限 + 读回后再判"的**两道**文件大小闸门（TOCTOU 二次判定）；
- `has_zip_signature` 这个**判断本身**（见 §2）：它不再服务兼容，只服务诊断分档；
- 版本门（`check_readable`）与结构校验（`validate()`）的**调用顺序与错误映射**
  （兼容删了，安全与诚实一条都没减）；
- `history.dag` 的三条读取裁决（空 ⇒ `None`；无 `main` 分支 ⇒ 拒绝；非法 JSON ⇒ 拒绝）；
- `ARCH-SEC-004` 的三阶段原子落盘（唯一入口 `write_project_atomic`）与 `MUST-GATE-008` 的锁语义。

### 1.3 错误码：**没有新增，也没有死分类**

- `ADR-0001 D25` 的 20 值联集是唯一来源。删除后**没有任何** `ContainerError` 变体变得不可达：
  容器读取器现在覆盖**每一个**文件（裸 JSON 也先经过它，只是报 `EocdNotFound`），
  写出路径（`container_bytes` → `write_project_container`）仍覆盖写侧的变体。
  因此 `store::container_rejection` 的**穷举** `match`（无 `_` 兜底）保持原样，
  `error::code_for_model` 的穷举 match 也**未改**（它是 D25 的编译期护栏）。
- 新增的 `not_a_container_fault` 同样只出口 `ErrorCode::IoError`；分类/规范 ID/容器原裁决走
  `data.{category, specId, containerError}` —— **不发明新码**。
- 注意：`ContainerError` 是 `yeban-model` 的类型，本线**无权**改它（`其它 crates/**` 禁改）；
  若将来模型层删变体，本 crate 的穷举 match 会**编译失败**——那正是想要的行为。

---

## 2. 判定顺序（新契约）与"看起来像 ZIP"那一条为什么不算补丁

```text
load_project(path)
  ├─ 不是普通文件                                  ⇒ FILE_NOT_FOUND
  ├─ metadata().len() / read() 后实际长度 > 上限    ⇒ IO_ERROR（category=archive-bomb，不读进内存）
  └─ load_container(path, bytes, len, limits)
       ├─ read_project_container 成功 ⇒ check_readable() → validate() → decode_history_dag()
       ├─ 失败且 bytes 有 ZIP 结构   ⇒ container_fault        【"你的 .yeban 坏了"】
       └─ 失败且 bytes 没有 ZIP 结构 ⇒ not_a_container_fault  【"你给的不是 .yeban 容器"】
```

第 3 步的签名检查**曾被写成兼容补丁**（旧注释：「保证"看起来像 ZIP 的东西"绝不会掉进裸 JSON
分支」）。裸 JSON 分支删掉之后这个**目的**消失了；判断被保留是因为它还有一个**与兼容无关**的
用途：把诊断分成"**你给的不是 `.yeban`**"与"**你的 `.yeban` 坏了**"。截断的 `.yeban` 前 4 字节
仍是 `PK\x03\x04`，因此落在"坏了"那一档，拿到的是**精确的容器裁决**（判据 10 钉住）。
函数因此改名 `looks_like_container` → `has_zip_signature`，签名从 1 个扩到 3 个
（local header / EOCD / data descriptor），文档重写，不留旧措辞 —— 与 app 侧
`has_zip_signature` **逐条同义**。

---

## 3. 非容器被**精确拒绝**的证据

**纯函数/单元路径**（`store.rs`，本机 `rustc --edition 2024 --test` 真跑；payload 由直接调用
`not_a_container_fault` 打印，**载荷逐字节如下**）：

```json
{"error":{"code":"IO_ERROR",
          "data":{"category":"malformed-container","containerError":"EocdNotFound",
                  "path":"/tmp/bare.yeban","specId":"ARCH-SEC-003"},
          "message":"`/tmp/bare.yeban` 不是 `.yeban` 容器 (容器裁决: end-of-central-directory record not found)"},
 "status":"error"}
```

对照（**"你的 `.yeban` 坏了"**那一档，`container_fault`）：

```json
{"error":{"code":"IO_ERROR",
          "data":{"category":"malformed-container","containerError":"EocdNotFound",
                  "path":"/tmp/cut.yeban","specId":"ARCH-SEC-003"},
          "message":"`/tmp/cut.yeban` 不是可接受的 .yeban 容器 [malformed-container / ARCH-SEC-003]: end-of-central-directory record not found"},
 "status":"error"}
```

**工具层（`yeban_open_project`）**：`tests/container_store.rs` 的
`bare_json_project_is_refused_precisely` / `non_container_inputs_are_refused_without_an_empty_project`
断言 `status=error`、`error.code=IO_ERROR`、`data.category=malformed-container`、
`data.specId=ARCH-SEC-003`、`data.containerError=EocdNotFound`、文案含
``不是 `.yeban` 容器`` 与 `end-of-central-directory`，并且 **`active_project()` 为 `None`**、
**调用方的文件逐字节未被改写**。

输入与出口对照（**不是**泛化的"未知格式"、**不是**空工程）：

| 输入 | 出口 | `containerError` |
| :--- | :--- | :--- |
| 真 `.yeban` 容器 | `Ok`，`format=yeban-container` | — |
| 裸 `project.json`（内容完全合法） | `not_a_container_fault` | `EocdNotFound` |
| 空文件 | `not_a_container_fault` | `EocdNotFound` |
| 64 字节 `0xAB` | `not_a_container_fault` | `EocdNotFound` |
| `not a zip at all` | `not_a_container_fault` | `EocdNotFound` |
| `{not json` | `not_a_container_fault` | `EocdNotFound` |
| 截断的 `.yeban`（前 4 字节 `PK\x03\x04`） | `container_fault`（精确容器裁决） | `EocdNotFound` / `TruncatedArchive` |
| 只改 central directory 条目名 | `container_fault` | `LocalCentralMismatch { index: 0, name: "qroject.json" }` |
| 翻转数据字节 | `container_fault` | `CrcMismatch { .. }` |
| 只有 `history.dag` 的容器 | `container_fault`（**不是空工程**） | `MissingProjectJson` |
| 容器**内部**的坏 `project.json` | `container_fault` | `InvalidProjectJson { detail }` |

**强对照**（"内容合法"这件事的证据）：同一份 JSON 字节**原样包进真容器**后仍能打开，
`projectDigest` 与裸 JSON 文本的 SHA-256 一致（单元判据
`bare_json_is_refused_as_not_a_container_and_the_same_json_in_a_container_still_opens`
与集成判据 7 各断言一次）—— 因此拒绝的是**容器边界**，不是"这份工程内容不行"。

---

## 4. 与 app 侧的语义一致性证据

| 维度 | `yeban-app`（`app-no-compat`，已合并） | `yeban-mcp`（本线） | 一致性 |
| :--- | :--- | :--- | :--- |
| "不是容器"的错误 | `OpenError::NotAYebanContainer { path, container }`，`Display` = `` `{path}` 不是 `.yeban` 容器 (容器裁决: {container}) `` | `store::not_a_container_fault(path, error)`，`message` = `` `{path}` 不是 `.yeban` 容器 (容器裁决: {error}) `` | **同一模板、同一含义**：暴露容器层的**原裁决**，不吞成"无法识别" |
| 形态报告字段 | 删掉 `OpenedProject::format` 字段，报告 `format=` 取值来自常量 `DOCUMENT_FORMAT = "yeban-container"` | 删掉 `LoadedProject::format` / `OpenRequest::format`，响应 `format` 取值来自常量 `store::DOCUMENT_FORMAT = "yeban-container"` | **同名常量、同一取值**；形态枚举整体删除 |
| "看起来像 ZIP"的补丁判断 | 改名 `looks_like_zip` → `has_zip_signature`（三签名），只服务"不是我们的文件" vs "我们的文件坏了" | `looks_like_container` → `has_zip_signature`（三签名），用途相同 | **同义**；截断容器两侧都落"坏了"档、都拿到精确容器裁决 |
| 判据反转方式 | 断言"能打开" ⇒ 断言"必须被拒绝 + 精确文案"（`cli_contract.rs`，退出码 3） | 断言"能打开 / `format=bare-json`" ⇒ 断言"必须被拒绝 + 精确 payload"（`container_store.rs` 判据 7、`store.rs` 单元判据） | **反转而不是删除** |
| 强对照 | 同一份 JSON 包进真容器仍能打开 | 同一份 JSON 包进真容器仍能打开（`projectDigest` 相同） | **同一条强对照** |
| 缺件容器 | `Container(MissingProjectJson)`，**不是**空工程 | `container_fault` + `MissingProjectJson`，**不是**空工程 | **同义** |
| 退出码 / 错误码 | CLI 退出码 `3`（容器拒绝），`0..5` 不变 | 契约 `IO_ERROR`（D25 联集），**没有新增码** | 各自协议的**同一收紧**（不是新增类别） |

两侧的差异只在**协议外壳**（CLI 退出码 vs MCP `ToolResponse.error.code`），
**语义判定逐条对应**。

---

## 5. 能力矩阵那一格：`audioClips` 的处理（如实保留）

`docs/ledger/audio-render-notes.md` §needs-7 指出：`audioClips` 那一格的**自然消失条件**是
「裸 JSON 分派删除 **且** `open_in_memory` 不再产生无载荷会话」。

**本线只删前者，因此回答是：这一格**不能删键**，触发条件**收窄**。**

- 第 1 个条件**已成立**：本文档第 1 节就是那个删除。
- 第 2 个条件**没有成立**：`Domain::open_in_memory` 仍在（
  `crates/yeban-mcp/tests/render_audio_clips.rs` 判据 9 与规范样本工程
  `yeban_model::samples::filled_project()` 的唯一会话种子路径就是它）。
- 代码里**唯一**还能造出"索引里有声明、会话 CAS 池里没有字节"的路径，现在就是
  `open_in_memory`（`src/domain/render.rs` 的 `resolve_asset` 返回 `Ok(None)` 的唯一情形）。
- 因此本线**如实保留** `audioClips` 键与那条分支，只把文档措辞从"裸 JSON 兼容路径与内存注入夹具"
  收窄为"内存注入的会话"；`audio-render-notes.md` §3.4 / needs-7 追加了状态更新，
  `mcp-render-notes.md` 的一处旧措辞也订正了。
- **没有**为了"矩阵好看"提前删键：删掉它会把一个"渲染必然失败"的工程交出去，
  违反 D43 保留的那条红线（**诚实性**：真不支持的必须在响应里如实说）。

---

## 6. 判据反转清单 + 注入→变红→还原

### 6.1 反转（**不是删除**）

| 旧判据（断言"能打开"） | 新判据（断言"必须被拒绝"） | 新断言 |
| :--- | :--- | :--- |
| `container_store::bare_json_projects_still_open_on_the_compat_path`（7） | `container_store::bare_json_project_is_refused_precisely`（7） | `IO_ERROR` + `malformed-container` + `EocdNotFound` + 文案含``不是 `.yeban` 容器``/`end-of-central-directory`；无活跃工程；原文件未改；**同字节进真容器仍 `Ok` 且 `projectDigest` 一致** |
| `store::tests::bare_json_is_still_readable_on_the_compat_path` | `store::tests::bare_json_is_refused_as_not_a_container_and_the_same_json_in_a_container_still_opens` | 同上 + 强对照（`write_container` 用**同一份 JSON 字节**） |
| `store::tests::corrupt_json_is_refused_with_an_io_error`（只断言 `IO_ERROR`） | `store::tests::empty_random_and_garbage_files_are_refused_precisely`（**扩容**） | 空 / 随机 / 垃圾 / 坏 JSON 四例都得到 `not_a_container_fault` + 精确 payload；真容器反面对照仍 `Ok` |
| `container_store::truncated_container_...`（只断言 `IO_ERROR`） | 同名（**扩容**） | 追加 `category=malformed-container`、`specId`、`containerError` 是字符串、文案是"不是可接受的 `.yeban` 容器" |
| `store::tests::oversized_files_...` 的"宽松上限 ⇒ `data.category` 为 null" | 同名（**改写**） | 宽松上限下现在走 `not_a_container_fault`（**有** category），因此改判 `data.fileBytes` 为 null（证明拒绝来自**文件大小**闸门） |
| 夹具：`Scratch::write_project` / `open_bare_json` 写裸 JSON | `Scratch::write_project` / `open_container` / `container_fixture` 写**真容器** | 生产写出路径同形（`write_project_container` + 空 `history.dag`） |

### 6.2 任务书点名的 8 条判据 ↔ 实现

| # | 要求 | 判据（本文件内编号） |
| :-- | :--- | :--- |
| ① | 容器读写闭环仍绿（回归） | `container_store` 1 / 2 / 3 / 4 / 9；`store::container_round_trip_preserves_the_project_and_the_history` |
| ② | 裸 JSON ⇒ 精确拒绝 | `container_store` 7；`store::bare_json_is_refused_as_not_a_container_and_the_same_json_in_a_container_still_opens`；`store::empty_random_and_garbage_files_are_refused_precisely` |
| ③ | 随机字节 / 空文件 / 截断各有明确错误 | `container_store` 17（空/随机/垃圾/裸 JSON）+ 10（截断）；`store::empty_random_...` + `store::truncating_...` |
| ④ | 篡改容器（CRC / 中央目录）仍被拒 | `container_store` 11（数据字节 ⇒ `CrcMismatch`）+ **18（central directory 条目名 ⇒ `LocalCentralMismatch`）**；`container_store` 12/13/14 覆盖 Zip-Slip / deflate / 缺件 |
| ⑤ | 炸弹 / 上限闸门仍生效（一个都不许松） | `store::container_bomb_gates_still_fire_after_the_compat_path_is_gone`（真容器 + 紧上限 ⇒ **容器层** `EntryTooLarge`，载荷**无** `fileBytes` ⇒ 不是 I/O 层那道）、`store::oversized_files_are_refused_before_they_are_read_into_memory`（I/O 层两道）、`container_store` 15（5 类畸形容器逐个都在 D25 联集内） |
| ⑥ | 不存在"打开成空工程"的路径 | `container_store` 17 末段（只有 `history.dag` ⇒ `MissingProjectJson`，且 `active_project()` 为 `None`）、14（缺 `history.dag`）、15（容器内坏 `project.json`）、16（坏/无 `main` 的 `history.dag`） |
| ⑦ | `yeban_save_project` 写出的字节是容器且能被 `yeban_open_project` 读回 | `container_store` 1（前 4 字节 + 模型层可读）、2（逐字节往返）、3（`history.dag` 恢复）、4（两次落盘逐字节相同）、9（资产往返） |
| ⑧ | 权限 / 锁的行为未受影响（`PROJECT_LOCKED` 仍绿） | `tools_e2e::save_into_a_read_only_directory_fails_with_io_error_and_keeps_the_original`、`tools_e2e::save_refuses_a_read_only_session`、`tools_e2e::opening_a_locked_project_is_project_locked_and_leaves_it_alone`、`lock_advisory.rs` 14 条（夹具已换成真容器） |

### 6.3 注入 → 变红 → 还原（**3 条，全部本机真跑**）

方法：`cp crates/yeban-mcp/src/domain/store.rs /tmp/mcp-no-compat-backup/store.rs`，
注入后用**唯一**探针 `/tmp/mcp-store-harness/run.sh`（`rustc --edition 2024 --test -D warnings`
指向**仓库原件**；无重依赖）重跑 `store.rs` 的单元判据，记录红点，再还原并用 `md5` / `diff -q` 逐字节确认。

| # | 注入 | 改法 | 实测红点 |
| :-- | :--- | :--- | :--- |
| **A** | **把裸 JSON 分支偷偷加回来**（任务书建议 1） | 在 `not_a_container` 那一支：首个非空白字节是 `{` 且能反序列化成 `YebanProjectV1` ⇒ `Ok(LoadedProject)` | **56 passed / 1 failed**：`store::tests::bare_json_is_refused_as_not_a_container_and_the_same_json_in_a_container_still_opens`（集成判据 7 的裸 JSON 例同理——它跑在 CI 上） |
| **B** | **把容器错误吞成"空工程"**（任务书建议 2） | 容器裁决那一支直接 `Ok(LoadedProject { project: filled_project(), .. })` | **55 passed / 2 failed**：`store::tests::truncating_a_container_is_an_io_error_not_a_half_project`、`store::tests::container_bomb_gates_still_fire_after_the_compat_path_is_gone` |
| **C** | **把某个安全闸门关掉**（任务书建议 3） | 去掉 `load_project_with_limits` 里**两道**文件大小闸门（`metadata` 与 `read` 后各一道） | **56 passed / 1 failed**：`store::tests::oversized_files_are_refused_before_they_are_read_into_memory` |

还原复核（**逐字节**）：

```text
$ md5 -q crates/yeban-mcp/src/domain/store.rs
c74af318bf0582f7099053b03b2db0fc        # 与注入前一致
$ diff -q /tmp/mcp-no-compat-backup/store.rs crates/yeban-mcp/src/domain/store.rs   # 无输出 = identical
$ grep -rn INJECT crates/yeban-mcp/     # 0 命中
$ bash /tmp/mcp-store-harness/run.sh    # test result: ok. 57 passed; 0 failed
```

（注入 C 需要顺带把 `max_file` / `declared` 改成 `_` 前缀才能通过 `-D warnings` 的编译，
否则红点是"编译失败"而不是判据失败 —— 这一步已如实处理，红点是判据红。
第三建议里"把 `format=` 写死"在当前设计下**不可判**：容器是唯一格式 ⇒ 这个值**按定义**只有一个，
因此采用了上面更可判的三条。）

---

## 7. 本机真跑 vs CI（**严格区分**）

### 7.1 本机**真跑**

| 步骤 | 命令（原样） | 实测（退出码直接读，**没有**把门禁管道给 `head`/`tail`） |
| :--- | :--- | :--- |
| `store.rs` 的单元判据（**执行的**） | `bash /tmp/mcp-store-harness/run.sh` | ✅ **57 passed; 0 failed**（其中 `domain::store::tests::*` 15 条，含全部新增/反转判据；探针用 `#[path]` 指向**仓库原件**，不是复制品） |
| 拒绝 payload 逐字节取证 | `/tmp/mcp-store-harness/msg`（同目录第二个探针） | ✅ 输出见 §3（`NOT_A_CONTAINER=...` / `BROKEN_CONTAINER=...` / `format const = yeban-container`） |
| 模型层 `ContainerError` 变体取证 | `/tmp/mcp-probe/probe`（只链接 `yeban-model`） | ✅ bare/empty/random/garbage ⇒ `EocdNotFound`；half-truncate ⇒ `EocdNotFound`；CD 名字 ⇒ `LocalCentralMismatch`；数据翻转 ⇒ `CrcMismatch`；紧上限 ⇒ `EntryTooLarge`；只 history.dag ⇒ `MissingProjectJson`；只 project.json ⇒ `MissingHistoryDag`；deflate ⇒ `UnsupportedCompression`；Zip-Slip ⇒ `ParentDirSegment` |
| **类型检查**（不链接、不执行） | `CARGO_TARGET_DIR=<主仓 target> bash scripts/dev/cargo-local.sh check -p yeban-mcp --all-targets` | ✅ 0 错误 0 警告（lib + 5 个集成测试目标；用注入语法错误证明测试目标**确实**被检查） |
| 格式化 | `bash scripts/dev/cargo-local.sh fmt -p yeban-mcp -- --check` | ✅ 通过 |
| 门禁 light | `bash scripts/gates/run-gates.sh light` | 见 §7.3 |

**关于 `check --all-targets` 的诚实说明**：它把 `CARGO_TARGET_DIR` 指向**主仓库已缓存的**
`target/`，只增量检查本 crate 与目标（实测 5.5s / 0.4s）。第三方重依赖（symphonia / rayon）用的是
主仓已缓存产物；**没有**在本机跑 `cargo test -p yeban-mcp`（链接集成测试需要 symphonia 的 rlib ⇒
会触发重依赖编译，政策禁止）、**没有** `--workspace`、**没有** benchmark / fuzz。
换句话说：**集成测试在本机只被"类型检查"，没有被"执行"** —— 它们的**真执行**在 CI。

### 7.2 交给 CI 判（本机**不能**判）

- **集成测试的真执行**：`tests/container_store.rs`（含反转后的判据 7、新增 17/18）、
  `tests/tools_e2e.rs`、`tests/lock_advisory.rs`、`tests/render_audio_clips.rs`
  —— 本机没有链接它们（原因见上），因此这些判据的**绿/红一律以 CI 为准**；
- `cargo clippy -p yeban-mcp --all-targets -D warnings` 与 `cargo fmt --all --check` 的 CI 版；
- 含 symphonia / rayon 的链接与执行（判据 5/8/9 的真实音频路径）；
- Windows 腿（`gates-manual` 的 `windows` 门禁）；
- 本机 `run-gates.sh crate yeban-mcp` 会 **SKIP**（传递重依赖，政策如此，不是失败）。

### 7.3 门禁与 CI 判决

（提交后回填：`run-gates.sh light` 读数、`git diff --cached --stat`、CI run id 与逐 job 结果。）

---

## 8. 边界 / needs / pending

### 8.1 边界（如实登记）

- **`yeban-app` 侧不在本线**：它已由 `line/app-no-compat` 删除并合并（§4 是对齐证据，
  本线**没有改** `crates/yeban-app/**`）。
- **MCP 的非容器拒绝是"进程内"语义**：MCP 没有进程退出码（那是 CLI 的），
  对应物是 `ToolResponse.error.code = IO_ERROR`。两侧"收紧"的方向一致，载体按各自协议。
- **`.yeban` 以外的导入格式**（`.mid` / `.als` / 音频）仍没有；本线只删兼容。
- **`open_in_memory` 仍在** ⇒ `audioClips` 键保留（§5）；删键是**另一条线的条件**。

### 8.2 needs

| # | 需要谁 | 事项 |
| :-- | :--- | :--- |
| needs-1 | 集成者 / 文档 | `README*` / 官网不要再写"可以打开散落的 `project.json`"（app 线已提过 needs-1，本线让它在**进程边界上也成立**） |
| needs-2 | 集成者 | `docs/DEVELOPMENT_LEDGER.md` / `gate-status.md` 若要登记"`yeban_open_project` 对非容器文件精确拒绝"，请引用本台账；本线**不改**这些共享文件 |
| needs-3 | 裁决（跨线） | `audioClips` 键的最终归属仍挂在 `audio-render-notes.md` needs-7：只有 `open_in_memory` 也收掉之后才可删键（§5） |
| needs-4 | app 线 / 集成者 | `crates/yeban-app/tests/cli_contract.rs` 与本线判据现在是**同一条纪律**；若将来再引入任何"读旧格式"的分支，两侧应当成对处理 |

### 8.3 pending

- CI 判决 run id：**待回填**（§7.3）。
- 本机**未执行**的集成测试绿：**待 CI**（§7.2）——本机只做了类型检查。

---

## 9. 修改文件清单

```text
crates/yeban-mcp/src/domain/store.rs             (修改: 删兼容读路径/ProjectFormat/load_bare_json; 加 has_zip_signature/not_a_container_fault/DOCUMENT_FORMAT; 判据反转+扩容)
crates/yeban-mcp/src/domain/mod.rs               (修改: OpenRequest::format 与 container 布尔删除; format 取常量)
crates/yeban-mcp/src/domain/render.rs            (修改: audioClips 触发条件措辞收窄为"内存注入")
crates/yeban-mcp/tests/container_store.rs        (修改: 夹具写真容器; 判据 7 反转; 新增 17/18; 10 扩容)
crates/yeban-mcp/tests/tools_e2e.rs              (修改: 夹具写真容器; 内联裸 JSON 夹具改真容器)
crates/yeban-mcp/tests/lock_advisory.rs          (修改: 夹具写真容器)
crates/yeban-mcp/tests/render_audio_clips.rs     (修改: 措辞)
docs/ledger/mcp-no-compat-notes.md               (新增: 本台账)
docs/ledger/store-container-notes.md             (修订: §4 删除条件 → 删除记录; §0/§2.2/§5/§7/§8/§9 加 D43 标记)
docs/ledger/audio-render-notes.md                (修订: §3.2/§3.4/needs-7 的状态更新)
docs/ledger/mcp-render-notes.md                  (修订: 一行措辞订正)
docs/ledger/human-decisions.md                   (修订: HD-20 标"已完成")
```

共享文件（`Cargo.toml` / `Cargo.lock` / `.github/**` / `scripts/**` / `deny.toml` /
`docs/DEVELOPMENT_LEDGER.md` / `docs/adr/**` / `docs/YEBAN_*.md` / `schemas/**` /
其它 `crates/**` / `spikes/**` / 法务文件 / `README*.md`）：**一个都没改**。
依赖图：**零变化**（因此 `Cargo.lock`、`docs/ledger/dependency-licenses.md` 不需要重新生成）。
