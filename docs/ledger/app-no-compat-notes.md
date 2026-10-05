# app-no-compat 工作线台账 —— 删除裸 JSON 兼容读路径（`.yeban` 容器是**唯一**工程格式）

- **台账类型**：删除清单 / 判据反转记录 / 注入记录 / 本机真跑与 CI 区分 / 边界与 needs（**不是规范**）
- **工作线**：`line/app-no-compat`（worktree `yeban/.worktrees/app-no-compat`，基线 main `632b0c0`）
- **日期**：2026-10-04
- **授权**：`docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` **D43** ——
  「1.0.0 之前不存在历史包袱 / 旧版本兼容；发现问题或更优解**直接推翻**」。
  D43 第 1 条**点名**了本条路径：`yeban-app` 的 `DocumentFormat::ProjectJson`
  （同一件事在界面侧的复制品，代码里的实际标识符是 `DocumentFormat::BareProjectJson`）——
  「**凡是"为了读旧文件/旧格式"而存在的分支，一律删除**，而不是"保留 + 记删除条件"」。
- **相关规范 (Normative)**：`[ARCH-SEC-003]`（容器）、`[ARCH-SEC-004]`（原子落盘）、
  `[MUST-GATE-006]`（Zip-Slip）、`[MUST-GATE-007]`（解压炸弹）、`[MODEL-AST-002]`、
  D30（容器读法歧义用"拒绝"或"明示"消除）。
- **地盘**：`crates/yeban-app/**` + 本台账。**没有**改其它 `crates/**`（尤其**没有**碰
  `crates/yeban-mcp/**` —— 那边的同一条 `bare-json` 路径由另一条线处理）、根 `Cargo.toml` /
  `Cargo.lock`（**零新增依赖**）、`.github/**`、`scripts/**`、`deny.toml`、
  `docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`、
  `spikes/**`、法务文件、`README*.md`。
- 本台账**读过但未改** `docs/ledger/store-container-notes.md` §4（裸 JSON 兼容的删除条件）：
  它描述的是 **`yeban-mcp` 的 store**，归那条线；D43 已经把 §4 的三条删除条件一次性作废
  （不是"满足条件"，是**不再需要条件**）。

---

## 1. 删了什么（逐项）

| # | 删除项 | 位置 | 为什么它"只服务兼容" |
| :-- | :--- | :--- | :--- |
| 1 | `DocumentFormat` 枚举（`Container` / `BareProjectJson`）与 `as_str()` | `src/open.rs` | 兼容路径存在时才有"形态"可选；删掉后只剩一个取值，单变体枚举只会招人再塞一个变体回来 ⇒ 替换为常量 `DOCUMENT_FORMAT = "yeban-container"` |
| 2 | `open_project_document_file` 的**裸 JSON 分派**（判定顺序第 3 步） | `src/open.rs` | 这就是兼容读路径本体 |
| 3 | `looks_like_bare_json`（首个非空白字节是 `{`） | `src/open.rs` | 只用于"要不要走裸 JSON 分支"的猜测 |
| 4 | `wrap_bare_project_json`（在内存里包最小容器再读） | `src/open.rs` | 兼容路径的成套机制（`project.json` + 空 `history.dag`） |
| 5 | `OpenError::NotAContainerNorJson` | `src/open.rs` | 名字与语义都建立在"也可能是裸 JSON"上；改为 `NotAYebanContainer`（**仍然**携带容器原裁决，不丢精确原因） |
| 6 | `OpenedProject::format` 字段 | `src/open.rs` | 报告读法的字段；只有一个读法 ⇒ 保留 `format=` 输出但取值来自常量 |
| 7 | `ProjectSource::File::format` 字段 | `src/cli.rs` | 同上（`opened:` / `saved: … from=` 两处输出不再有第二种取值） |
| 8 | 模块文档"裸 JSON 为什么可以顺便打开" | `src/open.rs` | 换成 D43 说明 + 新判定顺序 |
| 9 | `--open` 用法里"接受 `.yeban` 容器或裸 `project.json`" | `src/cli.rs`（`usage_text`） | 用户可见的兼容承诺 |
| 10 | `lib.rs` / `main.rs` 的同类表述 | `src/lib.rs` / `src/main.rs` | 同上 |
| 11 | 断言"裸 JSON **能**打开"的判据（3 条） | `src/open.rs`、`src/cli.rs`、`tests/cli_contract.rs` | **反转**，不是删除 —— 见 §3、§4 |

### 1.1 行数（`git diff --numstat` 实测；生产代码按"去掉 `#[cfg(test)]` 段"拆分）

| 类别 | 文件 | 增 / 删 | 净值 |
| :--- | :--- | :--- | :--- |
| **生产代码** | `src/open.rs` | +54 / −125 | **−71** |
| **生产代码** | `src/cli.rs` | +16 / −20 | **−4** |
| **生产代码** | `src/lib.rs` / `src/main.rs` | +4 / −3 | +1 |
| **生产代码小计** | | | **−74 行** |
| 判据（测试） | `src/open.rs` | +127 / −68 | +59 |
| 判据（测试） | `src/cli.rs` | +29 / −18 | +11 |
| 判据（测试） | `tests/cli_contract.rs` | +75 / −23 | +52 |
| **判据小计** | | | **+122 行** |
| 文档 | `docs/ledger/app-cli-notes.md`（修订） | +51 / −25 | +26 |
| 文档 | `docs/ledger/app-completion-notes.md`（修订） | +7 / −0 | +7 |
| 文档 | 本台账（新增） | +231 / −0 | +231 |

**口径说明（防止误读）**：这条工作线的**生产代码净删 74 行**（兼容分支、形态枚举、错误变体、
封装函数、文档承诺整段消失）。判据与文档净增是**任务本身的要求**（判据要**反转并扩容**，
notes 是新交付物）—— 不是"没删掉"。全仓 `git diff --cached --stat` 为
`8 files changed, 594 insertions(+), 282 deletions(-)`；**没有任何一份未打算删的文件出现净删**
（逐文件核对：唯一的删除集中在 `src/open.rs`（−193）与 `docs/ledger/app-cli-notes.md`（−25，
是我自己改写的 §5.1 旧文本），见 §6 的提交前检查）。

### 1.2 **保留**的东西（D43 不解除的部分）

- 容器侧**全部**安全闸门与错误分类：Zip-Slip（`ParentDirSegment`）、炸弹（`EntryTooLarge` /
  `ExpansionRatioExceeded`）、条目数、单条目上限、ZIP64 / 多卷、CRC、压缩法、缺件
  （`MissingProjectJson`）、**容器内**的坏 `project.json`（`InvalidProjectJson`）；
- "读之前按 `metadata` 判上限 + 读回后再判"的 `read_capped`（TOCTOU 二次判定）；
- `has_zip_signature` 这个**判断本身**（见 §2）：它不再服务兼容，只服务诊断分档。

---

## 2. 判定顺序（新的契约）与"看起来像 ZIP"那一条为什么不算补丁

`open_project_document_file` 现在只有三步：

1. 按**容器**读；成功 ⇒ `Ok`（`history.dag` / 资产池保真）；
2. 失败且文件**有 ZIP 结构**（前 4 字节是 `PK\x03\x04` / `PK\x05\x06` / `PK\x07\x08`）
   ⇒ **原样上报容器裁决**（`OpenError::Container`）。截断 / 被篡改的 `.yeban` 因此永远拿到
   精确错误码，而不是被糊成"不是我们的文件"；
3. 否则 ⇒ `OpenError::NotAYebanContainer`：`` `…` 不是 `.yeban` 容器 (容器裁决: …)``。

第 2 步的签名检查**曾被写成兼容补丁**（旧注释：「保证"看起来像 ZIP 的东西"绝不会掉进裸 JSON
分支」）。裸 JSON 分支删掉之后这个**目的**消失了；判断被保留是因为它还有一个**与兼容无关**的
用途：把诊断分成"**你给的不是 `.yeban`**"（第 3 步）与"**你的 `.yeban` 坏了**"（第 2 步）。
截断的 `.yeban` 前 4 字节仍是 `PK\x03\x04`，因此落在第 2 步 —— 判据 27 专门钉住这一点。
函数因此改名 `looks_like_zip` → `has_zip_signature`，文档重写，不留旧措辞。

---

## 3. 裸 JSON 现在被**明确拒绝**的证据

**纯函数/单元路径**（`open::tests::document_entry_rejects_a_bare_project_json_document`）：
`Err(OpenError::NotAYebanContainer { .. })`，`error.container() == Some(&ContainerError::EocdNotFound)`，
`Display` 必含 ``不是 `.yeban` 容器`` 与 `end-of-central-directory`。

**真进程**（`tests/cli_contract.rs::open_rejects_a_bare_project_json_with_a_precise_reason`，
本机用同构探针二进制真跑；CI 用 `CARGO_BIN_EXE_yeban-app`）：

```text
$ yeban-app --open /tmp/project.json --headless      # 裸 JSON = 从真容器里取出的那一份 project.json
(stderr) yeban-app: 打开 `/tmp/project.json` 失败: `/tmp/project.json` 不是 `.yeban` 容器 (容器裁决: end-of-central-directory record not found)
stdout: <空>
exit=3
```

对照（**不是**泛化的"未知格式"、**不是**空工程）：

| 输入 | 出口 | 退出码 |
| :--- | :--- | :--- |
| 真 `.yeban` 容器 | `Ok`，`format=yeban-container` | `0` |
| 裸 `project.json`（内容完全合法） | `NotAYebanContainer` + `EocdNotFound` | `3` |
| 空文件 / `not a zip at all` / 64 字节 `0xAB` | `NotAYebanContainer` + `EocdNotFound` | `3` |
| 截断的 `.yeban`（前 4 字节 `PK\x03\x04`） | `Container(…)` + 精确容器裁决 | `3` |
| 多一个条目（`notes.txt`）的 ZIP | `Container(UnexpectedContainerEntry)` | `3` |
| 缺 `project.json` 的容器 | `Container(MissingProjectJson)`（**不是**空工程） | `3` |
| 容器**内部**的坏 JSON | `Container(InvalidProjectJson { detail })` | `3` |

"内容合法"这件事有一条**强对照**判据：同一份 JSON 字节包进真容器后能正常打开
（判据 26 / B10 的后半段）—— 因此拒绝的是**容器边界**，不是"这份工程内容不行"。

---

## 4. 判据反转清单 + 注入→变红→还原

### 4.1 反转（**不是删除**）

| 旧判据（断言"能打开"） | 新判据（断言"必须被拒绝"） | 新断言 |
| :--- | :--- | :--- |
| `open::tests::document_entry_accepts_a_bare_project_json_document` | `open::tests::document_entry_rejects_a_bare_project_json_document` | `Err(NotAYebanContainer)` + `EocdNotFound` + 精确文案；同份 JSON 进容器仍能打开 |
| `cli::tests::open_accepts_a_bare_project_json_document_and_labels_it`（39） | `cli::tests::open_rejects_a_bare_project_json_document_with_a_precise_reason`（39） | `exit_code() == 3` + 文案 + `format=` 常量唯一 + 用法不再提裸 JSON |
| `cli_contract::open_accepts_a_bare_project_json_and_labels_the_format`（B10） | `cli_contract::open_rejects_a_bare_project_json_with_a_precise_reason`（B10） | 真进程 `code == 3`、stdout 空、stderr 精确、容器对照仍 `0` |
| `open::tests::document_entry_reports_a_container_as_a_container`（断言 `format` 字段） | `open::tests::document_entry_opens_a_real_container` | 去掉形态字段，保留归档全保真 |
| `open::tests::non_container_non_json_input_is_rejected_with_the_container_verdict` | `open::tests::non_container_input_is_rejected_precisely_without_an_empty_project` | 空/垃圾/随机/坏 JSON 四例 + "有 ZIP 结构"分档 + 缺件容器**不是**空工程 |
| `cli_contract::broken_inputs_exit_non_zero_without_panicking`（B4） | 同名（**扩容**） | 追加空文件 / 随机字节两例；`既不是` → ``不是 `.yeban` 容器`` |

### 4.2 任务书点名的 6 条判据 ↔ 实现

| # | 要求 | 判据 |
| :-- | :--- | :--- |
| ① | 真容器仍能打开（回归） | `open::tests::document_entry_opens_a_real_container`、`real_container_round_trips_through_the_open_entry`、B3、`tests/open_project_file.rs` |
| ② | 裸 `project.json` 被拒绝且原因精确 | 判据 26 / 39 / B10 |
| ③ | 随机字节 / 空文件 / 截断文件各自明确错误 | 判据 28 / B4（空、垃圾、随机、截断） |
| ④ | 篡改容器（CRC / 中央目录）仍被拒 | 判据 19 (c) 数据区 CRC ⇒ `CrcMismatch`；(h) 中央目录 CRC ⇒ `LocalCentralMismatch`；(a)(b)(d)–(g) 压缩法 / Zip-Slip / 炸弹 / 上限 / ZIP64 |
| ⑤ | `--save-as` 写出的字节仍能被 `--open` 读回（闭环） | 判据 33 / B5（工程 + 归档逐字段等价） |
| ⑥ | 不存在"打开成空工程"的路径（缺件容器报错） | 判据 28 末段（只有 `history.dag` ⇒ `MissingProjectJson`，且断言 `!= Some(empty_project)`）；判据 18 的逐长度截断 |

### 4.3 注入 → 变红 → 还原（**2 条，全部真做过**）

方法：`cp crates/yeban-app/src/open.rs /tmp/app-no-compat-backup/open.rs`，注入后用**两条探针**
（`run.sh` = 零 Slint 单元判据；`verify.sh` = `tests/cli_contract.rs` 打在本机同构探针二进制上）
重跑，记录红点，再还原并用 `md5` / `diff -q` 逐字节确认。

| # | 注入 | 改法 | 实测红点 |
| :-- | :--- | :--- | :--- |
| **A** | **把裸 JSON 分支偷偷加回来**（任务书建议 1） | 在 `NotAYebanContainer` 那一支里：首个非空白字节是 `{` ⇒ `write_container` 包最小容器 + `read_project_container` 读回来返回 `Ok` | 单元 **92 passed / 3 failed**：`open::tests::document_entry_rejects_a_bare_project_json_document`、`open::tests::non_container_input_is_rejected_precisely_without_an_empty_project`、`cli::tests::open_rejects_a_bare_project_json_document_with_a_precise_reason`；真进程 **11 passed / 1 failed**：`open_rejects_a_bare_project_json_with_a_precise_reason` |
| **B** | **把容器错误吞掉返回空工程**（任务书建议 2） | 把该支直接改成 `Ok(OpenedProject { archive: 空工程, .. })` | 单元 **91 / 4 failed**（多红 `cli::tests::broken_inputs_fail_with_exit_code_three_and_a_precise_reason`）；真进程 **10 / 2 failed**（多红 `broken_inputs_exit_non_zero_without_panicking`） |

还原复核：`md5 -q crates/yeban-app/src/open.rs` = `8f5af788e23b57a46ceacb26a363d499`（与注入前一致）；
`diff -q` identical；`grep -rn INJECT crates/yeban-app/` = **0 命中**。
（第三条建议"把 `format=` 写死"在当前设计下**不可判**：容器是唯一格式 ⇒ 这个值**按定义**只有一个，
所以没有采用；取更可判的两条。）

---

## 5. "容器是唯一格式"后的 CLI 语法表变化

| 项 | 旧 | 新 |
| :--- | :--- | :--- |
| `--open <path>` 的输入集合 | `.yeban` 容器 **或**裸 `project.json` | **只** `.yeban` 容器 |
| `--open` 对非容器文件 | 裸 JSON ⇒ 打开并标 `format=project-json`；其余 ⇒ `NotAContainerNorJson` | 一律 ⇒ `NotAYebanContainer`（"不是 `.yeban` 容器" + 容器原裁决），退出码 `3` |
| `opened:` 行的 `format=` | `yeban-container` \| `project-json` | **只有** `yeban-container`（常量 `DOCUMENT_FORMAT`） |
| `--help` 的 `--open` 说明 | "接受 `.yeban` 容器或裸 `project.json`" | "`.yeban` 容器是**唯一**工程格式（D43）；非容器文件被明确拒绝" |
| `--help` 退出码 `3` 说明 | "容器拒绝：…" | 追加 "**不是 `.yeban` 容器**"，并补 "缺件" |
| 退出码 | `0..5` **不变** | `0..5` 不变（这是一次**收紧**，不是新增码） |
| `--save-as` / `--export-elements` / `--version` / `--headless` / `SLINT_BACKEND=headless` | — | **一行未改** |

用法文本里的兼容承诺同时被一条判据钉住：`usage_text()` 不得含 `裸` 或 `project-json`
（`cli::tests` 判据 39 与真进程 B1 各断言一次）。

---

## 6. 本机真跑 vs CI（严格区分）

### 6.1 本机**真跑**（`rustc --edition 2024 --test -D warnings`，零 Slint；探针在
`/tmp/app-no-compat-harness/`，**仓库之外**，用 `#[path]` 指向仓库原件 —— 不是复制品）

| 步骤 | 命令 | 实测（退出码直接读，**没有**把门禁管道给 `head`/`tail`） |
| :--- | :--- | :--- |
| 零 Slint 半边单元判据（cli/bridge/scene/elements/input/open/save） | `bash /tmp/app-no-compat-harness/run.sh` | ✅ `RUN_EXIT=0` → **95 passed; 0 failed** |
| `tests/cli_contract.rs` 12 条，被测进程 = 本机同构探针二进制 | `bash /tmp/app-no-compat-harness/verify.sh` | ✅ `VERIFY_EXIT=0` → **12 passed; 0 failed** |
| clippy（`-D warnings -D clippy::all`，直接调 `clippy-driver`、不经管道） | 见 `clippy.sh` 的等价直调 | ✅ `CLIPPY_DIRECT_EXIT=0`，零输出 = 零告警 |
| 门禁 light | `bash scripts/gates/run-gates.sh light` | ✅ `GATES_EXIT=0` → `门禁通过 (mode=light)`（fmt / 红线守卫 / 文档链接 / 依赖许可清单漂移全绿） |

### 6.2 交给 CI 判（本机**不能**判）

- **真二进制**的 argv→stdout/stderr/**退出码**（`tests/cli_contract.rs` 的 12 条，含反转后的 B10
  与扩容后的 B4）：本机跑的是同构探针进程，能证明**逻辑**，不能证明**链接后的产物**；
- 含 **Slint** 的一切：`main.rs` 的 GUI 分发、`host::build_main_window`、`.slint` 语法与类型
  （`run-gates.sh crate yeban-app` 在本机 SKIP，这是纪律不是失败）；
- `cargo clippy -p yeban-app --all-targets -D warnings`（**含** Slint 目标）与 `cargo fmt --all --check` 的 CI 版。

### 6.3 CI 判决

- **run `37245680897`（branch `line/app-no-compat`，commit `6a43a44`）= ✅ success**（读回方式：
  `bash scripts/dev/ci-verdict.sh --watch line/app-no-compat`）。逐 job：
  - ✅ `lockfile`（17s）、✅ `checks`（fmt / 红线守卫 / schema，38s）、✅ `deny`（cargo-deny，45s）、
    ✅ `plan`（受影响集合，7s）；
  - ✅ **`rust (yeban-app)`（ID 111563025107，4m6s）** —— 原始日志实测：
    lib 单元判据 `test result: ok. 110 passed; 0 failed`；
    `Running tests/cli_contract.rs` ⇒ `test result: ok. 12 passed; 0 failed`
    （**这就是本线"真二进制退出码 + 反转后的 B10 + 扩容后的 B4"的判据**，
    本机那一半用的是同构探针，只有这里才是链接后的产物）；
  - `rust (workspace 全量)` / `windows` 按受影响集合推导 **skip**（0s），符合"只跑受影响集合"。
- 说明：本线**只推了一次**（代码 + 文档同一提交），因此不存在"连续推送取消上一轮 run"的风险
  （L23/L26）。

---

## 7. 边界 / needs / pending

### 7.1 边界（如实登记）

- **`yeban-mcp` 的同一条路径不在本条线**：`crates/yeban-mcp/src/domain/store.rs` 的
  `looks_like_container` / `ProjectFormat` / `bare-json` 由另一条线按 D43 处理。
  在那条线落地之前，**MCP 侧仍然能读裸 JSON** —— 这是**已知的暂时不一致**，不是静默降级。
- **`.yeban` 以外的导入格式**（`.mid` / `.als` / 音频）仍然没有；本条线只删兼容。

### 7.2 needs

| # | 需要谁 | 事项 |
| :-- | :--- | :--- |
| needs-1 | 集成者 / 文档 | README / 官网不要再写"可以打开散落的 `project.json`"；`app-cli-notes.md` §6 的诚实说明已加"非容器被明确拒绝"一条 |
| needs-2 | 另一条线（`yeban-mcp`） | 按 D43 删掉 store 的裸 JSON 读路径，让"容器是唯一格式"在**进程边界**上也成立 |
| needs-3 | 集成者 | `docs/DEVELOPMENT_LEDGER.md` / `gate-status.md` 若要登记"`--open` 对非容器文件精确拒绝"，请引用本台账；本线**不改**这些共享文件 |

### 7.3 pending

- ~~CI 判决 run id~~ **已读回**：run `37245680897` = ✅ success（见 §6.3）。
- `docs/ledger/app-cli-notes.md` 里 app-cli 线**历史**的 CI run `37242779089` 与本线无关，
  仅作为那份台账的历史证据保留。
