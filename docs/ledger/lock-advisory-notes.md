# 工作线 `lock-advisory` 台账 —— `MUST-GATE-008` 操作系统级建议锁

- **所有者目录**：`crates/yeban-mcp/**`（本文件与 `src/domain/lock.rs`、`tests/lock_advisory.rs` 是本线新增）
- **分支 / 基线**：`line/lock-advisory`，基于 main `d9a4e9d`
- **规范来源**：
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` **[MUST-GATE-008]**：
    "打开工程时必须成功施加 OS 建议锁并原子创建锁文件，并发读写立即抛出 `PROJECT_LOCKED`"
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` **§0.2 [ARCH-SEC-001]**（`.yeban.lock` 五条：原子创建 /
    OS 建议锁 / 双模式 / 内容元数据协议 / 心跳与陈旧抢占）
  - `schemas/mcp-tools.schema.json`（`PROJECT_LOCKED` 是契约错误码之一）+ `docs/adr/ADR-0001` **D25**（联集 20 值）
  - 前置台账：`docs/ledger/tools-domain-notes.md` 的 **boundary-3 / P6**（"`.yeban.lock` 只有原子创建 + 存在即拒，
    **崩溃会留下永久锁**"）与 **needs-7**（锁文件命名 `<工程文件名>.lock`）

> 本文件回答四个问题：**规范要什么、std 到底有什么、内核实测行为是什么、哪些做不到**。

---

## 1. 先核验再写码：stable Rust 里到底有没有文件锁

任务书要求"先核验再写"。核验材料是**本机工具链自带的 rustdoc**
（`rustc 1.99.0 (b940084d7 2026-09-28)`，sysroot `~/.rustup/toolchains/stable-aarch64-apple-darwin`，
`share/doc/rust/html/std/fs/`），不是记忆、不是网页。

| 问题 | 核验结论 | 出处（可复核） |
| :--- | :--- | :--- |
| stable 里有文件锁吗？ | **有** | `std/fs/struct.File.html` |
| 方法叫什么？ | `File::try_lock()`、`File::try_lock_shared()`、`File::lock()`、`File::lock_shared()`、`File::unlock()` | 同上（`method.*` 锚点） |
| 哪个版本稳定的？ | 全部标注 **`1.89.0`** | 每个方法签名前的版本徽标 |
| 返回什么？ | `try_*` → `Result<(), TryLockError>`；`TryLockError::{WouldBlock, Error}`，"`WouldBlock` = 被别的句柄/进程持有" | `std/fs/enum.TryLockError.html` |
| Unix 上底层是什么？ | **`flock(2)`**（`LOCK_EX`/`LOCK_SH` + `LOCK_NB`）—— **不是** `fcntl(F_SETLK)` | `try_lock` / `lock_shared` 的 "Platform-specific behavior" 段 |
| Windows 上底层是什么？ | **`LockFileEx`**（`LOCKFILE_EXCLUSIVE_LOCK` + `LOCKFILE_FAIL_IMMEDIATELY`）；且**append-only 句柄锁不住** | 同上 |
| 工具链够新吗？ | `rust-toolchain.toml` 钉 `1.99.0` ≥ 1.89.0 ⇒ **可用** | 仓库文件 |

**⇒ 决定：零新增依赖。** 没有引入 `libc`，也没有引入 `windows-sys`。
`crates/yeban-mcp/Cargo.toml` **一个字都没改**，依赖图不变，
因此 `Cargo.lock` 与 `docs/ledger/dependency-licenses.md` 都不需要重新生成
（本机跑了 `python3 scripts/gates/license_inventory.py` 验证：内容与 HEAD **逐字节相同**，
`git status` 不显示该文件）。

### 1.1 `flock(2)` vs `fcntl(F_SETLK)`：为什么选了前者

规范 §0.2 第 2 条字面写的是 `fcntl(F_SETLK)`。本线**刻意偏差**，理由是两条实测出来的语义差别：

| | `fcntl(F_SETLK)`（POSIX record lock） | `flock(2)`（本实现） |
| :--- | :--- | :--- |
| 进程死亡（含 `SIGKILL`）自动释放 | 是 | 是 |
| **关闭 fd** 时释放 | **否**（锁属于 (进程, inode)，`close` 不放） | 是 |
| **同一进程内多个 fd 互相争用** | **否**（同进程对同一 inode 的 fcntl 锁互不冲突） | **是**（实测 S1） |

第三行是决定性的：`fcntl` 拦不住"同一进程开两次同一个工程"（任务书场景 ①）。
`F_GETLK` 的"查询被谁持有"能力 `flock` 没有 —— 本实现用**锁文件内容**里的持有者元数据
（PID / hostname / lock_mode / project_path）承担同一职责，且不需要 `unsafe`。

---

## 2. 内核行为实测（不是"看起来对"）

写码前先跑了一个一次性探针（`/tmp/lockprobe2`，`std::fs::File` + `std::process::Command`），
测的是**本机 macOS/aarch64 的真实内核行为**。原始输出：

```text
S1 same-proc ex/ex : a=Ok(()) b=Err("\"WouldBlock\"")       # 同进程两个 fd 真的互斥
S2 shared/shared   : c=Ok(()) d=Ok(())                       # 共享锁真的可共存
S3 shared-then-ex  : f=Err("\"WouldBlock\"")                 # 读者在场时写者真的被挡
S4 rdonly shared   : g=Ok(()) h=Ok(())                       # 只读句柄也能拿共享锁
S4 rdonly exclusive: i=Err("\"WouldBlock\"")                 # 只读句柄拿不到排他锁
S5 parent-holds-ex, child ex   : CHILD ex => Err("\"WouldBlock\"")   # 跨进程真的互斥
S5 parent-holds-ex, child shared: CHILD sh => Err("\"WouldBlock\"")  # 跨进程读者也被挡
S6 killed child status = ExitStatus(unix_wait_status(9))     # 子进程被 SIGKILL
S6 after SIGKILL, parent try_lock = Ok(())                   # 内核真的自动释放了锁
S6 lock file still exists = true                             # 但锁文件仍在磁盘上!
S6 takeover rewrite ok, len=12                               # 于是"接管并重写"是可行的
```

S6 的最后两行就是本门禁要修的那个病的**机理**：
**崩溃 ⇒ 内核释放建议锁 + 磁盘留下锁文件**。
旧实现用"文件存在"当占用证据 ⇒ 永久锁死；
新实现用"能不能拿到建议锁"当证据 ⇒ 自动可接管。

---

## 3. 落地设计

### 3.1 文件与规范 ID 映射

| 文件 | 规范 ID | 内容 |
| :--- | :--- | :--- |
| `crates/yeban-mcp/src/domain/lock.rs`（新） | `MUST-GATE-008`、`ARCH-SEC-001` | `LockMode`、`LockMetadata`、`LockGuard`、`LockError`、平台层、`acquire`、`read_metadata` |
| `crates/yeban-mcp/src/domain/store.rs`（改） | `ARCH-SEC-001` | 锁机制**全部委托**给 `lock.rs`；只保留"`LockError` → 契约错误码"的唯一映射；`lock_path` 等再导出 |
| `crates/yeban-mcp/src/domain/mod.rs`（改） | `MCP-TOOL-001/003` | 打开/关闭路径接新锁；响应新增 `lockMode` / `advisoryLock` / `tookOverStaleLock` |
| `crates/yeban-mcp/tests/lock_advisory.rs`（新） | `MUST-GATE-008` | 15 条判据（含跨进程与 SIGKILL 崩溃自愈） |
| `crates/yeban-mcp/tests/tools_e2e.rs`（改） | `MCP-TOOL-001..010` | 把"造一个锁文件"改成"**真的持有建议锁**"（旧写法在新语义下是崩溃遗留，会被合法接管） |

### 3.2 加锁协议（原子创建 + 建议锁，两者都要成功）

```text
open_lock_file:
    create_new(true) 成功          -> (file, created=true)     # 原子创建 (O_CREAT|O_EXCL)
    AlreadyExists -> open(read+write) -> (file, created=false) # 文件本来就在
    (Windows 注意: append-only 句柄锁不住, 所以一律 read+write)

took_over_stale_lock = (!created) && mode == ExclusiveWrite

try_lock()/try_lock_shared()  失败 -> 返回 Err, **不删锁文件**（失败者不该动别人的文件）
                              成功 -> 排他模式重写元数据 (set_len(0)+write+sync_all)
                                     失败 -> 撤掉这次加锁并删掉锁文件 (不留"我的锁+别人的内容")
```

**为什么先创建后加锁是安全的**（崩溃/竞态矩阵）：

| 崩溃/竞态点 | 磁盘状态 | 下一个打开者 | 判定 |
| :--- | :--- | :--- | :--- |
| `create_new` 之前 | 无锁文件 | 自己创建 + 加锁 | 可打开 |
| `create_new` 后、`try_lock` 前 | **空锁文件** | 能拿到锁（无持有者） | 可打开 + 接管 — **正确** |
| `try_lock` 后、写元数据前 | 空文件 + 活锁 | `WouldBlock` | `PROJECT_LOCKED` |
| 写元数据中（截断后/写完前） | **半截 JSON** | `WouldBlock` | `PROJECT_LOCKED`（元数据不参与判定） |
| 持有中崩溃（含 `SIGKILL`） | 完整元数据 + **无锁** | 能拿到锁 | 可打开 + **接管重写** |
| 正常排他 `Drop` | 文件被删 | 无锁文件 | 可打开 |

**关键性质：任何一行都不会永久锁死。** 三种残骸（空文件 / 半截 JSON / 完整元数据但无人持有）
处置完全相同 —— 拿得到建议锁就接管。判据 `a_corrupted_lock_file_is_still_takeable`
逐一把这三种残骸都跑了一遍。

### 3.3 `PROJECT_LOCKED` 映射（不发明新码）

| OS 层事实 | 出口 | 判据 |
| :--- | :--- | :--- |
| `WouldBlock`（同进程另一 fd / 跨进程 / 读者挡写者 / 写者挡读者） | `ToolResponse.error.code = PROJECT_LOCKED` | `a_second_open_in_the_same_process_is_project_locked`、`another_process_cannot_open_a_project_we_hold_exclusively` |
| 平台无建议锁 | JSON-RPC 实现级 `-32005`（`Fault::Impl`），**不是**契约码 | 见 §5 pending P3（cfg 注入，本机无真机） |
| `io::Error` | 复用既有 `code_for_io` → `IO_ERROR` / `FILE_NOT_FOUND` / `DISK_FULL` | `domain::error` 既有判据 |

`PROJECT_LOCKED` 的 `data` 载荷（可观察 + 可排查）：

```json
{
  "lockFile": ".../demo.yeban.lock",
  "holder": "<锁文件原文>",
  "holderPid": 48215,
  "holderMode": "ExclusiveWrite",
  "heartbeatAgeSecs": 0,
  "staleHeartbeatSecs": 15,
  "advisoryLockHeld": true
}
```

`advisoryLockHeld` 是刻意留的**可断言字段**：它把"占用来自内核建议锁"这件事
从散文变成判据（`assert_locked` 每次都断言它）。

### 3.4 双模式：`SHARED_READ` / `EXCLUSIVE_WRITE`

- **读者与写者必须锁同一个 inode** ⇒ 读者也锁**锁文件**，
  因此旧行为"只读打开不创建锁文件"被**有意改变**：只读打开现在会创建并共享锁住 `.yeban.lock`。
  旧行为在语义上等于"只读打开完全不受保护"（写者照样能改），是更糟的选择。
- 共享读者**不删**锁文件（删了会制造 "检查存在 → 加锁" 的 TOCTOU 窗口，
  且无法唤醒已经在等另一个 inode 的竞争者）。读者留下的文件**没有持有者 ⇒ 可接管**，
  所以它不是永久锁（判据 `an_exclusive_guard_releases_the_file_and_is_not_a_takeover` 的后半段）。
- 排他持有者 `Drop` 时：显式 `unlock()` + 关闭 fd + 删除锁文件（保持旧的"关闭即清理"直觉）。

### 3.5 心跳与陈旧阈值（规范 §0.2 第 5 条的**收窄**）

规范要求 3 秒续约 / 15 秒陈旧 / `kill(pid, 0)` 二次确认后允许接管。本实现：

- 常量 `HEARTBEAT_INTERVAL_SECS = 3`、`STALE_HEARTBEAT_SECS = 15` **按规范钉住**，并有判据；
- 但**心跳不参与任何判定**：
  - 锁被持有 ⇒ 一律 `PROJECT_LOCKED`，**无论心跳多陈旧**；
  - 锁没有被持有 ⇒ 一律可接管，**无论心跳多新鲜**。
- 理由（单调性）：内核在进程死亡时释放 `flock` ⇒ "建议锁被持有"是"持有者活着"的**充分**证据；
  反向不成立（持有者可能活着但心跳陈旧）。只凭心跳接管会强抢一个被 `SIGSTOP`
  或调度饿死的**活**持有者的锁 —— 宁可拒绝，也不误抢。
- `heartbeatAgeSecs` 只出现在 `PROJECT_LOCKED` 的诊断载荷里，供 UI 提示"持有者可能已卡死"。
- 规范允许的 `--force-unlock` **没有实现**：接管是**隐式**的（拿到锁即接管），
  残留锁不需要"强制"开关。登记为 pending P2。

---

## 4. 判据（15 条，全部本机真跑）

`cargo test -p yeban-mcp` 全量：**152 单元 + 15 contract + 13 lock_advisory + 25 tools_e2e 全绿**。

| # | 判据 | 测什么 |
| :--- | :--- | :--- |
| ① | `lock_file_is_sibling_named_after_the_project_file` | `<工程文件名>.lock` |
| ②③ | `a_second_open_in_the_same_process_is_project_locked` | 同进程第二个 `Domain` → `PROJECT_LOCKED`；独占期间只读也被拒 |
| ④ | `an_exclusive_guard_releases_the_file_and_is_not_a_takeover` | 关闭释放（`releasedLock=true` + 文件被删）；重新打开不是接管 |
| ⑤ | `a_lock_file_without_a_live_holder_is_not_occupied` | **仅凭文件存在不算被占用**（崩溃可接管） |
| ⑥ | `taking_over_a_stale_lock_rewrites_the_metadata` | 接管**重写** PID / 时间戳 / 工程路径 |
| ⑦ | `a_corrupted_lock_file_is_still_takeable` | 空文件 / 半截 JSON / 非 JSON 三种残骸都可接管 |
| ⑧⑨ | `shared_readers_coexist_and_block_a_writer` | 读者共存；读者挡写者；第三个读者仍可入 |
| ⑩ | `another_process_cannot_open_a_project_we_hold_exclusively` | **跨进程**双向互斥（真的另一个 PID） |
| ⑪ | `another_process_can_share_the_read_lock_or_be_blocked_by_a_writer` | 跨进程共享读共存；跨进程写者被读者挡 |
| ⑫⑬ | `a_sigkilled_holder_releases_the_advisory_lock_and_can_be_taken_over` | `SIGKILL` 后锁文件仍在 + 立刻可接管 + 陈旧 PID 被覆盖 |
| ⑭ | `the_only_contract_code_for_lock_contention_is_project_locked` | 只用 `PROJECT_LOCKED`；联集仍是 20 值 |
| ⑮ | `an_unsupported_platform_is_refused_through_the_implementation_exit` | 平台不支持走 JSON-RPC `-32005`（**映射判据**；真机判据见 P3） |

### 4.1 跨进程是怎么"真做"的（不是 `sleep` 猜时间）

`tests/lock_advisory.rs` 用 `std::process::Command` **重新执行本测试二进制**，
子进程里跑的是**真实的 `Dispatcher` + `yeban_open_project`**（和 `yeban-mcp` 二进制同一条代码路径）：

```text
父: Command::new(current_exe()) .args(["cross_process_child_holder","--nocapture"])
    .env(YEBAN_LOCK_TEST_CHILD=1) .env(...PROJECT) .env(...READY) .env(...MODE)
子: 真实 Dispatcher 打开工程 -> 把**真实响应 JSON** 写进握手文件 -> sleep 等被杀
父: 轮询握手文件（不是 sleep 猜时间）-> 解析出子进程看到的响应 -> 断言
父: Child::kill() (= Unix SIGKILL) + wait() 收尸（不留僵尸）
```

`cross_process_child_holder` 在没有环境变量时**立刻返回**，
所以它在正常 `cargo test` 里是一条空判据（不会卡住 60 秒）。

**做不到的部分（明说，不含糊）**：
- `#[cfg(unix)]` 之外的平台**没有**跨进程判据（`Child::kill()` 在 Windows 上语义不同，本机也无法验证）。
- `UnsupportedPlatform` 路径**没有真机判据**（本机是 macOS，走的是 Unix 分支）。

### 4.2 注入 → 变红 → 还原（4 轮，全部实测）

| 注入 | 改了什么 | 变红的判据（实测） | 还原 |
| :--- | :--- | :--- | :--- |
| **INJ-1** | `acquire()` 开头加 `if lock_file.exists() { return Err(WouldBlock) }`（= **旧实现**：只看文件存在） | **6 条**：⑤ `a_lock_file_without_a_live_holder_is_not_occupied`（"无持有者的锁文件必须可接管"）、⑥、⑦、⑧（两个读者不再共存）、⑪、⑫（"崩溃后必须能接管"） | `diff` 与备份逐字节相同 ✅ |
| **INJ-2** | `LockGuard::drop` 里排他分支不 `remove_file`（只释放建议锁） | ④ `an_exclusive_guard_releases_the_file_and_is_not_a_takeover`（"关闭工程后锁文件必须被删除"） | 同上 ✅ |
| **INJ-3** | `rewrite_metadata` 直接 `return Ok(())`（拿锁但不重写内容） | ⑥ `taking_over_a_stale_lock_rewrites_the_metadata`、⑬ `a_sigkilled_holder_...`（断言在 `lock_advisory.rs:604`） | 同上 ✅ |
| **INJ-4** | 去掉 `plan_open` 的"模式升级"防线（`already_open && read_only != is_read_only`） | `tools_e2e::save_refuses_a_read_only_session`（"读者在场时写者必须被拒" —— 写请求被放行） | 同上 ✅ |

每次注入后都重跑对应测试二进制并记录**精确失败行号与断言文本**，
还原后重跑全量测试确认回到全绿。**注**：注入只发生在工作树内，从未推送。

---

## 5. needs / pending / TODO

### pending

| # | 项 | 状态 |
| :--- | :--- | :--- |
| **P1** | **Windows 分支（`LockFileEx`）从未编译过、从未跑过** | **still pending**。代码不写 `unsafe`、不自己调 `windows-sys`，只是把 `File::try_lock*` 在 `#[cfg(windows)]` 下接线 —— 因此不存在"看起来对但从未编译"的手写 FFI。但**"接线正确"这件事本机无法证明**。需要：CI 增 `windows-latest` job，或人工在 Windows 上跑 `cargo test -p yeban-mcp --test lock_advisory`。**在没有这项证据前，不得宣称 Windows 已验证。** |
| **P2** | `ARCH-SEC-001` §0.2 第 5 条的**心跳抢占**（3s 续约 / `kill(pid, 0)` / `--force-unlock`） | **still pending**（本线**有意收窄**，见 §3.5：建议锁是单调的活体证据，心跳不能覆盖它）。若要实现，需要 `libc::kill` 或 `OpenProcess` ⇒ 需要显式依赖，且必须解决"心跳陈旧但持有者活着"的误抢风险。 |
| **P3** | 非 Unix / 非 Windows 的 `UnsupportedPlatform` | **没有真机判据**。代码路径存在（`#[cfg(not(any(unix, windows)))]`），语义是"显式拒绝，绝不静默放过"。验证手段只能是交叉编译到 wasm 之类的目标（本机与 CI 都不做）。 |
| **P4** | `F_GETLK` 风格的"查询被谁持有" | 已用锁文件元数据替代（`read_metadata`）。`flock` 无此查询是**平台事实**，不是遗漏。 |
| 承接 `tools-domain` P6 | `.yeban.lock` 的 OS 建议锁 / 心跳 / 陈旧锁抢占 | **OS 建议锁 + 陈旧锁接管：本轮关闭**；心跳部分见 P2。 |

### needs（需要别的所有者裁决/接线）

| # | 项 | 建议 |
| :--- | :--- | :--- |
| **needs-A** | **只读打开的行为改变**：旧行为"只读打开不创建锁文件"，新行为"只读打开创建并共享锁住 `.yeban.lock`" | 语义上必须如此（读者与写者要锁同一个 inode）。若产品上不接受"看一个工程会留下一个锁文件"，替代方案是让读者也删文件 —— 但那会引入 TOCTOU 窗口，**本线不建议**。请人类确认取舍。 |
| **needs-B** | `docs/ledger/tools-domain-notes.md` 的 boundary-3 / P6 与 `MCP-TOOL-001` 状态行现在**过时了** | 本线按纪律**不改**该文件（`docs/ledger/*` 中它由前一条线拥有）。请集成者把 "**没有** `fcntl(F_SETLK)`/`LockFileEx` OS 建议锁、**没有** `SHARED_READ` 多读者" 更新为已落地。 |
| **needs-C** | `ARCH-SEC-001` §0.2 第 2 条的**字面偏差**（`flock` 而非 `fcntl`） | 建议由集成者写进 `docs/adr/` 或直接修订规范措辞为"得到 OS 建议锁（Unix 可用 `flock(2)` 或 `fcntl(F_SETLK)`，须满足：进程死亡自动释放 + 同进程多 fd 互相争用）"。本线不擅自改 Normative 文档。 |

### boundary（明确做不到的观测）

| # | 边界 | 说明 |
| :--- | :--- | :--- |
| boundary-A | Windows 真机验证 | 见 P1。 |
| boundary-B | `UnsupportedPlatform` 真机 | 见 P3。 |
| boundary-C | 心跳判据 | 心跳**当前不是判据**，因此没有"心跳超时 ⇒ 接管"的判据；有常量判据把规范值钉住。 |
| boundary-D | `DISK_FULL` 类的锁失败 | 与前置线一致：只做映射判据，不真把磁盘写满。 |

---

## 6. 本机真跑 vs 交给 CI

| 项 | 本机（Apple M2, macOS/aarch64） | CI |
| :--- | :--- | :--- |
| `cargo clippy -p yeban-mcp --all-targets -- -D warnings` | ✅ 真跑（0 告警） | `ci.yml` 全量 |
| `cargo test -p yeban-mcp`（152 + 15 + 12 + 25） | ✅ 真跑（全绿） | 全量 |
| **跨进程判据**（本文件 §4.1） | ✅ **真跑**（真起子进程，SIGKILL 真崩溃） | 真跑（同机同平台） |
| `run-gates.sh crate yeban-mcp` | ✅ **真编译真跑**（`yeban-mcp` 无重依赖，**不是 SKIP**） | — |
| `run-gates.sh light` | ✅ 全绿 | — |
| Windows / wasm 平台 | ❌ 做不到 | 需要 `windows-latest` job（P1） |
| 全量 workspace / benchmark / fuzz | ❌ 按纪律不跑 | 交 CI |

---

## 追加（`line/store-container` 代记）：**Windows 强制锁**与"持锁期间谁能读锁文件"

> 本节由 `line/store-container`（`crates/yeban-mcp/**` 的当前所有者）追加，**不修改**上文任何原结论。
> 上文 P1 / boundary-A 说"Windows 分支从未编译过/跑过" —— 那仍然是**当时的**事实；本节记录的是
> 集成者新增的 `windows` 手动门禁**第一次真跑**之后发生的事。

### 实测：`windows` 门禁第一次执行就抓到真实缺陷（CI run `37235205697`）

```text
thread 'domain::store::tests::exclusive_lock_is_atomic_and_released_on_drop' panicked at store.rs:314:
读锁元数据: Os { code: 33, kind: Uncategorized,
  message: "The process cannot access the file because another process has locked a portion of the file." }
```

**根因（设计假设不成立，不是测试写错）**：

| | Unix（`flock(2)`） | Windows（`LockFileEx`） |
| :--- | :--- | :--- |
| 锁的性质 | **建议锁**：不阻止其它句柄 `read`/`write` | **强制锁**：锁的是**字节区间**，其它句柄（**含同一进程的另一个句柄**）的读写被 OS 拒绝 |
| 持锁期间**另一个句柄**读锁文件 | 成功 | **失败**（`ERROR_LOCK_VIOLATION` / `os error 33`） |
| 通过**持锁句柄**写元数据 | 成功 | 成功（锁的所有者可以读写自己的区间） |
| 持有者诊断的可靠来源 | 磁盘内容 **或** 守卫访问器 | **只能**是守卫访问器（内存里那一份） |

### 已落地的三条修法（`crates/yeban-mcp/src/domain/lock.rs`）

1. `LockGuard` **携带它写入的元数据** + `LockGuard::holder()`：诊断与判据**不再**"持锁再读文件"，
   两个平台行为一致（Unix 侧还少一次 I/O）；
2. **"读持有者信息"发生在尝试加锁之前**：`LockFileSnapshot` 先读一次，
   `LockError::WouldBlock { snapshot }` 把这份快照带给调用方（对**别人的**持有者，Windows 上
   这份快照同样是"读不到" —— 那是 OS 强制的，不是我们能绕的）；
3. **容忍"读不到"**：不 panic、不把打开判失败；`PROJECT_LOCKED` 载荷新增
   `holderMetadata ∈ {available, unavailable-on-this-platform, unavailable}`。
   "我看不到持有者"与"没有持有者"是两件事，糊成一个 `null` 才是缺陷。

配套：`Domain::lock_holder()`（会话层出口）；`store::locked_fault_with_snapshot()`；
判据 `tests/lock_advisory.rs::holder_metadata_while_locked_is_platform_specific`
（Unix 断言可读、Windows 断言被拒，**两边都断言，不删也不跳过**）。

### 对上文 pending 的影响

| # | 上文的结论 | 现在 |
| :--- | :--- | :--- |
| **P1** | Windows 分支从未编译/跑过 | **门禁已存在并已真跑过一次**（抓到本节缺陷）。修复后的代码由集成者复跑 `gates-manual` 的 `windows` 门禁复核；**在拿到那次读数之前，本项仍是 pending** |
| **boundary-A** | Windows 真机验证 | 同 P1。另新增一条平台事实：**跨进程**读持有者信息在 Windows 上不可用（`LockFileEx` 强制锁）⇒ 记录为 `needs-5`（`docs/ledger/store-container-notes.md` §8） |
| **boundary-C / D** | 心跳不是判据 / `DISK_FULL` 只有映射判据 | **不变** |
