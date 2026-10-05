# `[MUST-GATE-001]` 闭环台账 —— 实时回调零分配 / 零释放 / **零锁等待** / **零阻塞 I/O**

- **台账类型**：探针设计 / 判据映射 / 实测读数 / 注入记录 / **覆盖范围的诚实边界** / 本机真跑与 CI 的严格区分 / needs（**不是规范**）
- **工作线**：`line/gate-rt-zero-alloc`（worktree `yeban/.worktrees/gate-rt-zero-alloc`，基线 main `330bf85`）
- **所有者目录**：`crates/yeban-engine/**`（本文件是唯一新增的共享区文档；`docs/ledger/{gate-status,phase-status,feature-alignment,human-decisions}.md` 与 `docs/DEVELOPMENT_LEDGER.md` 由集成者改）
- **规范来源**：
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.1 第 1 条（`MUST-GATE-001`，逐字见下）
  - `AGENTS.md` §2 红线 7（实时回调内零分配 / 零释放 / 零锁等待 / 零阻塞 I/O）+ §5（本机不编译重依赖）
  - `docs/DEVELOPMENT_LEDGER.md` 的 L6 / L17（门禁不许管道、不许用 `;` 接提交）、L22（全局计数器必须排除别的线程）、L24 / L26 / L28（补丁必须复核）、L30（本机真跑必须可回答"跑的哪个树"）、L31（提交信息每句"已完成"都要有当场命令）
- **上游接缝**：`crates/yeban-engine/tests/rt_zero_alloc.rs`（本目标的既有 10 000 量子 + 63 次交换版本）、
  `tests/meter_rt_contract.rs`（电平路径的运行期零分配）、`tests/transport_rt_zero_alloc.rs`（走带）、
  `tests/synth_rt_zero_alloc.rs`（合成 + 混音链）、`tests/snapshot_retire_churn.rs`（按线程武装的分配器先例）

> 本文件回答六个问题：**规范那四个词各自变成了哪条判据**、**探针凭什么不是空转（"有牙"的证据）**、
> **六个场景的实测四元组**、**判据怎么变红（4 组注入 + 3 组盲区证据）**、
> **覆盖范围到哪为止（哪些 I/O 与锁路径不在覆盖内）**、**本机真跑与 CI 判决的严格区分**。

---

## 0. 规范原文（逐字）

> **[MUST-GATE-001] 音频线程零安全违规 (Zero Glitch / Zero Alloc)**：实时音频回调函数内严禁出现任何堆内存分配（`malloc`）、
> 堆释放（`free`/`drop`）、文件/网络 I/O 或互斥锁争用，**CI 运行时通过内存分配 Hook 进行严格断言**。

---

## 1. 结论（一句话 + 交付清单）

> 规范这句话现在**四个分量各自都有运行期判据**：六个场景（纯渲染 / 快照交换 / 走带 / 电平计量 /
> 自动化求值 / 限制器混音链，合计 **27 764 个量子** + 1 063 次快照交换）里，
> **`allocations == 0`、`deallocations == 0`、`lock_blocking == 0`、`lock_waits == 0`、
> `io_requests == 0`、`io_ops == 0` 同时成立**，且**每一次读数都附带"探针确实被跑到过"的见证**
> （`quanta_visits == 该窗口的量子数`、`lock_try_successes == quanta_visits`）。
> 探针的"牙"用 3 组正对照（非 RT 路径上阻塞加锁 / 争用等待 / 真实 I/O）+ 4 组注入（各自打红后**逐字节还原**）证明。
> 本机（M2）**整条判据 22/22 通过，实测 3.22 秒**（含构建）。

| 文件 | 行数 | 规范 ID | 说明 |
| :--- | ---: | :--- | :--- |
| `crates/yeban-engine/src/rt_probe.rs`（**新建**） | 596 | `MUST-GATE-001` `ARCH-RT-001` | 实时路径的**可插桩边界**：见证型锁探针 `RtLockProbe` + 唯一诊断/I-O 出口 `diag` + 线程窗口计数 + `foreign_*` 归属桶 |
| `crates/yeban-engine/src/rt.rs`（+14） | 1128 | `MUST-GATE-001` `ARCH-RT-001` | `render_block` 开头调 `rt_probe::quantum_enter()`（每量子一次见证）；三处电容耗尽 + 无快照分支改走 `rt_probe::diag(...)` |
| `crates/yeban-engine/src/snapshot.rs`（+6） | 1660 | `MUST-GATE-001` `ARCH-RT-002` | 退役队列满（`retire_or_stash`）走 `rt_probe::diag(SnapshotRetireStash)` |
| `crates/yeban-engine/src/lib.rs`（+2） | 147 | `MUST-GATE-001` | `pub mod rt_probe;` + 模块地图一行 |
| `crates/yeban-engine/tests/rt_zero_alloc.rs`（**重写**） | 1335 | `MUST-GATE-001` `ARCH-RT-001` | 六场景 × 四元组（`harness = false`）+ 22 条判据 + 探针正对照 + 线程归属对账 |

**零新增依赖**：根 `Cargo.toml`、`Cargo.lock`、`deny.toml`、`crates/**`（本 crate 之外）、`scripts/**`、`.github/**` 一个字未动。

---

## 2. 探针设计

### 2.1 为什么探针住在**产品代码**里

| 读法 | 证据 | 谁给 |
| :--- | :--- | :--- |
| "代码里没写锁/没写 I/O" | `grep -n "Mutex\|println" crates/yeban-engine/src/rt.rs` | 人/审查（**不是**运行期判据） |
| "**运行期确实没有**" | 探针在**同一条被执行到的路径**上读数为 0 | `src/rt_probe.rs` |

集成判据（`tests/*.rs`）只能看到 crate 的**公共面**，无法把自己的仪器插进 `render_block`。
一个装在测试里、与实时路径无关的计数器读到 0，只证明"没人碰过那个计数器"。因此探针必须在 `src/` 里，
以公共 API 暴露读数。

### 2.2 锁探针：见证（witness）而不是哑计数器

`RtLockProbe` 包住一个**真的** `std::sync::Mutex<()>`，把三种加锁行为分开计数：

| 方法 | 语义 | 计数 |
| :--- | :--- | :--- |
| `lock()` | **阻塞式**加锁（红线 7 禁止实时路径使用） | 阻塞尝试 +1；若已被别的线程持有则等待 +1，然后真的等 |
| `try_lock_once()` | **非阻塞**试探（实时路径上允许） | 成功 / 失败分开计数 |

实时路径上的探针位置是 `EngineRuntime::render_block` 的**开头**：每个量子调一次 `rt_probe::quantum_enter()`，
它在窗口内做两件事 —— 记一次"探针位置被执行到"（`quanta_visits`），并对内置的 `rt_path_lock()` **非阻塞**试探一次
（`try_lock`，成功次数记进 `lock_try_successes`）。于是判据可以同时钉住：

1. **位置见证**：`quanta_visits == 本窗口处理的量子数` ⇒ 探针确实在被执行的代码里（不是"那段代码根本没跑"）；
2. **能力见证**：`lock_try_successes == quanta_visits` ⇒ 这条路径**真的能操作那把锁**
   ⇒ 同一窗口里 `lock_blocking == 0 && lock_waits == 0` 是**测出来的 0**，不是"没有仪器所以没有违规"。

### 2.3 I/O 探针：单一出口 + 计数型后端

实时路径上原本没有任何日志/文件/网络调用，因此本线**建**了一个出口：`rt_probe::diag(RtDiagEvent)`。
它接在三个"真的发生异常"的位置（都是实时路径上的边界事件）：

| 调用点 | 事件 | 触发条件 |
| :--- | :--- | :--- |
| `rt.rs` 三处电容耗尽分支 | `MeterCapacityDrop` | 计量节点数超过定长缓冲 / 电平槽被淘汰 |
| `rt.rs` 无快照分支 | `NoSnapshot` | 写者尚未发布任何快照 |
| `snapshot.rs::retire_or_stash` | `SnapshotRetireStash` | 退役队列满 ⇒ 旧快照寄存 |

`diag` 分三层计数：进入边界（`io_requests`）、**真的转交给已安装 sink**（`io_ops`）、外线程对照（`foreign_io_*`）。
判据要求**前两层都为 0**（连"想在实时路径上产生一条诊断"都不许）。
默认（发布构建）没有 sink ⇒ 纯计数；判据安装一个**计数型见证后端**（真的写临时文件 + 真的 `stderr` 打印），
用它证明这个边界**有牙**。

### 2.4 两层计数器：线程窗口 + 进程总量 + 外线程桶

| 层 | 存储 | 语义 |
| :--- | :--- | :--- |
| **窗口** | `thread_local!` + `const` 初始化（无惰性分配、无析构器） | 只在 `watch_current_thread()` 打开的**那个线程**上累加 |
| **总量** | `AtomicU64` | 进程内所有线程的累计（用于"别人的操作确实发生过"的对照） |
| **外线程** | `AtomicU64` | 来自**非声明 RT 线程**的操作（`declare_rt_thread()`），防"把别人的操作算到 RT 头上" |

窗口是**线程局部**的 ⇒ 判据 ⑪ 可以在实时窗口**跨**一个外线程活动的同时保持读数无歧义
（外线程做 3 次阻塞加锁 + 2 次真实 I/O：实时窗口四元组仍全 0，而 `foreign_*` 桶分别是 3 / 2、见证 sink 实写 2 次）。

### 2.5 成本（为什么可以留在产品代码里）

| 情形 | 每量子成本 |
| :--- | :--- |
| 未武装（发布构建的常态） | 一次线程局部 `Cell::get`（约 1 ns；无分配、无原子、无系统调用） |
| 已武装（判据窗口内） | 同上 + 三次线程局部读写 + 一次 `Mutex::try_lock`（无争用时是单次 CAS）+ 一次 `Relaxed` 原子加 |

这份成本表**不是靠文字自证的**：10 000 量子窗口里 `quantum_enter` 每量子都被执行
（`quanta_visits == 10000`），而**同一个窗口**的分配计数为 0 ⇒ 探针自身的开销被**其它探针**证明是实时安全的（互相作证）。

### 2.6 "有牙"的证据（正对照 + 见证，全部为实测读数）

| # | 正对照 | 实测读数（本机 M2） |
| :-: | :--- | :--- |
| ⑦a | 非 RT（控制侧）路径上 3 次**阻塞**加锁 | `lock_blocking=3 lock_waits=0`，其余分量 0 |
| ⑦b | 锁**已被别的线程持有**时阻塞加锁（对照探针） | `lock_blocking=1 lock_waits=1`（等待被观察到，不只是尝试） |
| ⑦c | 非 RT 路径上 3 次 `diag` | `io_requests=3 io_ops=3`；sink `emits 2 -> 5`；见证文件里该事件行数 `0 -> 3` |
| ⑦d | 同一窗口的分配分量 | `alloc=0 dealloc=0`（真实文件/控制台 I/O **不计入堆分配** ⇒ 注入 I2 变红只能归因于 I/O） |
| 见证 | 六场景每个窗口 | `quanta_visits == 期望量子数`、`lock_try_successes == quanta_visits`、`lock_try_failures == 0` |

> 关键的一条：把 `render_block` 开头的 `quantum_enter()` **删掉**（注入 I4）后，
> 所有 0 仍然是 0（因为没有仪器就没有读数），但**见证判据立刻变红**：
> `探针经过=0（期望 10000）`。这就是"探针不许空转"的机械保证。

---

## 3. 六个场景的实测四元组（本机 M2，冻结代码）

命令（**本机真跑**；L30：`cargo-local.sh` 会打印它实际用的工作区）：

```bash
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test rt_zero_alloc
```

`--no-default-features` 是纪律要求（`yeban-engine` 默认 feature 含 cpal，本机不编译重依赖）。

| # | 场景 | 量子 | 子窗口 | 探针经过 | 试探成功 | alloc | dealloc | lock_blocking | lock_waits | io_requests | io_ops |
| :-: | :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ① | 纯渲染（`filled_project`） | 10 000 | 1 | 10 000 | 10 000 | 0 | 0 | 0 | 0 | 0 | 0 |
| ② | 快照交换（63 逐步 + 1 000 高频） | 1 063 | 1 063 | 1 063 | 1 063 | 0 | 0 | 0 | 0 | 0 | 0 |
| ③ | 走带（200 轮命令 + 2 000 播放 + 501 停住） | 2 701 | 203 | 2 701 | 2 701 | 0 | 0 | 0 | 0 | 0 | 0 |
| ④ | 电平计量（每 5 量子一次 UI 抽干） | 10 000 | 2 000 | 10 000 | 10 000 | 0 | 0 | 0 | 0 | 0 | 0 |
| ④d | UI 侧抽干 1 000 次（独立窗口） | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| ⑤ | 自动化求值（每量子一批 `SetParam`） | 2 000 | 2 000 | 2 000 | 2 000 | 0 | 0 | 0 | 0 | 0 | 0 |
| ⑥ | 限制器/混音链（滤波+声相+限制+窃取） | 2 000 | 1 | 2 000 | 2 000 | 0 | 0 | 0 | 0 | 0 | 0 |
| ⑪ | 实时窗口**跨外线程**（3 锁 + 2 真实 I/O） | 400 | 1 | 400 | 400 | 0 | 0 | 0 | 0 | 0 | 0 |

合计 **27 764 个量子**、1 063 次快照交换；**每个场景的四元组全 0**。

各场景的**覆盖度证据**（防"窗口里什么都没跑"的假绿，详情来自判据 detail 行）：

| # | 覆盖度读数 |
| :-: | :--- |
| ① | `quanta=10001`（含预热 1 个）、`event_bulk_pops=10001`（每量子一次批量出队）；母线未触发限制器（`reductions=0`） |
| ② | `发布=1063 实际切换=1063 主线程回收=1063 退役队列满寄存=0`（高频段每量子一发一换） |
| ③ | `走带命令=202 推进量子=2151 状态=Stopped 停住窗口位置 18259 -> 18259`（时钟真的冻结） |
| ④ | `每量子批量发布=10001 抽到帧=40004 非有限帧=0`（3 轨 + 母线 = 4 帧/量子） |
| ⑤ | 自动化值域 `-12.000 .. 2.994`（曲线真的在动）、`实时侧应用=2000` |
| ⑥ | `限制器压过=256098 最大压限=0.8878 声部窃取=27 触发音符=83 NaN=0 右声道非零=0` |
| ⑪ | 外线程：`foreign 加锁=3 foreign 诊断=2 见证 sink 实写=2`；外线程自己的窗口 `lock=0 io=0` |

完整日志（**定义性的一次，提交前在最终代码上重跑，未注入**，节选）：

```text
[MUST-GATE-001] 判据 S PASS 仪器自检：分配器在窗口内能看见 1 次分配 + 1 次释放（有判别力）
             自检窗口[alloc=1 dealloc=1 ...]
[MUST-GATE-001] 判据 T PASS 探针自身的陷阱已登记：新锁首触可能分配一次（平台相关，读数只打印），温暖之后恒为 0
             冷窗口[alloc=1 dealloc=0 ...]（macOS 实测应为 alloc=1 bytes=64）暖窗口[alloc=0 dealloc=0 ...]（必须为 0）
[MUST-GATE-001] 判据 ⑩ PASS ARMED 窗口语义：...
             窗口外 1 000 次分配后空窗口[alloc=0 ...]；窗口内 1 次分配 1 alloc/1 dealloc；未武装窗口 lock_blocking=0 io_requests=0（总量已 +1 / +1）
[MUST-GATE-001] 判据 ① PASS ... ①纯渲染；四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0]；子窗口=1 探针经过=10000（期望 10000）试探成功=10000 试探失败=0；quanta=10001 ...
[MUST-GATE-001] 判据 ⑥ PASS ... 限制器压过=256098 最大压限=0.8878 声部窃取=27 触发音符=83 NaN=0 右声道非零=0
[MUST-GATE-001] 判据 ⑦a PASS ... 窗口四元组[alloc=0 dealloc=0 lock_blocking=3 lock_waits=0 io_requests=0 io_ops=0]
[MUST-GATE-001] 判据 ⑦b PASS ... 握手(held=true joined=true released=true) 争用窗口四元组[alloc=0 dealloc=0 lock_blocking=1 lock_waits=1 io_requests=0 io_ops=0]
[MUST-GATE-001] 判据 ⑦c PASS ... 窗口四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=3 io_ops=3]；sink 计数 emits 2 -> 5 bytes 48 -> 168；见证文件里该事件行数 0 -> 3
[MUST-GATE-001] 判据 ⑪c PASS 线程归属：... foreign 加锁=3（要求 3）foreign 诊断=2（要求 2）见证 sink 实写=2（要求 2）；外线程窗口 lock=0 io=0（要求 0/0）
[MUST-GATE-001] 判据汇总: 22 / 22 通过
[MUST-GATE-001] ok: 六场景（纯渲染 / 快照交换 / 走带 / 电平计量 / 自动化 / 混音链）四元组全 0；探针有牙（正对照 + 注入）；线程归属与外线程活动已对账
```

---

## 4. 判据清单与注入记录

### 4.1 判据清单（`tests/rt_zero_alloc.rs`，22 条，全部 PASS）

| # | 标签 | 判据 | 变红方式 |
| :-: | :--- | :--- | :--- |
| 1 | ① | 纯渲染 10 000 量子：四元组全 0 + 见证成立 | 注入 I1/I2/I3/I4 |
| 2 | ①c | 覆盖度：10 000 量子确实被处理（`quanta` / `event_bulk_pops`） | 减少压测规模 |
| 3 | ② | 快照交换 63 + 1 000：四元组全 0 | 注入 I1/I2/I3 |
| 4 | ②c | 覆盖度：实际切换 ≥ 1 000 且旧快照在主线程回收 | 不排空退役队列 |
| 5 | ③ | 走带 Play/Stop/Seek：四元组全 0 | 注入 I1/I2/I3 |
| 6 | ③c | 覆盖度：命令被应用、推进过、停住窗口时钟冻结 | 删掉停住分支的冻结 |
| 7 | ④ | 电平计量 10 000 量子：四元组全 0 | 注入 I1/I2/I3 |
| 8 | ④c | 覆盖度：每量子恰好一次批量发布、UI 抽到帧且全部有限 | 每轨单独 publish |
| 9 | ④d | UI 侧抽干 1 000 次：零分配零释放 | 在 collector 里分配临时 `Vec` |
| 10 | ⑤ | 自动化求值 2 000 量子：四元组全 0 | 注入 I1/I2/I3 |
| 11 | ⑤c | 覆盖度：曲线值在动 + 事件真的被实时侧出队 | 发布事件但不驱动量子 |
| 12 | ⑥ | 限制器/混音链 2 000 量子：四元组全 0 | 注入 I1/I2/I3 |
| 13 | ⑥c | 覆盖度：限制器真的压过、窃取真的发生、右声道静音、无 NaN | 摘掉限制器 / 关掉声相 |
| 14 | S | 仪器自检：窗口内 1 次分配 + 1 次释放必须被看见 | 关掉 `ARMED` 开关 |
| 15 | T | 新锁首触可能分配一次（只打印读数），温暖之后恒为 0 | 去掉温暖步骤 |
| 16 | ⑩ | ARMED 窗口语义：窗口外不计数、窗口内有牙、未武装不进窗口 | 把线程局部武装改成无条件累加 |
| 17 | ⑦a | 锁探针有牙：非 RT 路径 3 次阻塞加锁被计数 | 把计数删掉 |
| 18 | ⑦b | 等待有牙：锁被别人持有时记成 1 次等待 | 把等待计数删掉 |
| 19 | ⑦c | I/O 探针有牙：3 次诊断 = 3 次真实写（计数 + 字节 + 文件内容三重证据） | 把 sink 调用删掉 |
| 20 | ⑦d | 见证 sink 零分配（注入变红的归因干净） | 在 sink 里分配 |
| 21 | ⑪ | 实时窗口**跨外线程**活动：四元组仍全 0 | 把窗口计数改成进程全局 |
| 22 | ⑪c | 线程归属：外线程操作落进 `foreign_*` 桶且真的发生（不是空转） | 去掉 `foreign_*` 记账 |

⑧（RT 回调里 `Mutex::lock()` ⇒ 锁判据红）与 ⑨（RT 回调里写文件/打印 ⇒ I/O 判据红）由注入 I1 / I2 实测覆盖；
⑫（门禁）见 §7。

### 4.2 四组注入（注入 → 变红 → **逐字节还原** → 复绿）

注入只落在**本线拥有**的文件 `crates/yeban-engine/src/rt.rs`。
每次注入前把 `rt.rs` 复制到 `/tmp/rt.rs.frozen`，还原后用 `sha256sum -c` 逐文件校验（**四次都是 4/4 OK**）。

- `rt.rs` 冻结哈希：`sha256 = ccd0c2d0a35428f29501de043af9fca508fcb88b941b19449c30efbac2a6f5cf`
  （注入前 / 每次还原后 / 全部注入跑完之后**逐字节相同**）
- 这四组记录是在**最终冻结代码上重跑**的（`rt_probe.rs` 经 `cargo fmt` 重排之后，见 §10 的四个哈希）：
  每次注入只改 `rt.rs` 一行，跑同一条命令，记录红行，再 `cp` 还原并校验。

| # | 注入（`rt.rs::render_block` 开头，紧随 `quantum_enter()`） | 判据结果 | 原始红行（节选） |
| :-: | :--- | :--- | :--- |
| I1 | `drop(rt_probe::rt_path_lock().lock());` | **15 / 22**，6 个场景 + ⑪ 的**锁**分量红 | `①纯渲染；四元组[alloc=0 dealloc=0 lock_blocking=10000 lock_waits=0 ...]` |
| I2 | `rt_probe::diag(RtDiagEvent::NoSnapshot);` | **14 / 22**，6 个场景 + ⑪ 的 **I/O** 分量红 | `四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=10000 io_ops=10000]`（见证 sink 实写 10 000+ 行，证明 I/O 真的发生） |
| I3 | `let injected = Vec::<u8>::with_capacity(1); black_box(&injected);` | **15 / 22**，6 个场景的**分配/释放**分量红 | `四元组[alloc=10000 dealloc=10000 lock_blocking=0 ...]` |
| I4 | **删掉** `rt_probe::quantum_enter();` | **15 / 22**，见证（位置）判据红 | `探针经过=0（期望 10000）试探成功=0`（四元组仍显示 0 —— 正是"没有仪器就没有读数"） |

还原后复跑：**22 / 22 通过**（见 §7 的本机真跑记录）。

### 4.3 三组盲区证据（**注入后判据不变红**，如实登记；它们不是判据的牙）

这三条不是"判据失败"，而是**覆盖边界的存在性证明**。它们把 §5 的"不在覆盖内"从**声明**变成**实测**。

| # | 注入 | 结果 | 含义 |
| :-: | :--- | :--- | :--- |
| G1 | `eprintln!("G1");`（绕过边界，每量子一次） | **22 / 22 全绿**，日志里实测 **28 171 行** `G1` | 裸打印**完全看不见**：`io_requests=0 io_ops=0 alloc=0`。这就是"不是系统调用级拦截"的代价 |
| G2a | `static G2A_RAW_MUTEX: std::sync::Mutex<()>` + 每量子 `lock()`（首次触碰发生在预热量子 = 窗口外） | **22 / 22 全绿** | 裸 `std::sync::Mutex` **完全看不见**（`lock_blocking=0`、`alloc=0`）—— 这是锁覆盖面的**最坏情形** |
| G2b | 每量子 `let m = std::sync::Mutex::new(()); drop(m.lock());`（首次触碰发生在窗口**内**） | **15 / 22**，**分配**分量红：`alloc=10000 dealloc=10000 lock_blocking=0` | 裸锁**只能**被分配探针"顺带"抓到：macOS 上每个 `Mutex` 实例的首次加锁都分配一次 64 字节（§6）。在别的平台上这条"顺带"可能不存在 |

---

## 5. 覆盖范围的诚实说明（**必须和读数一起读**）

### 5.1 覆盖矩阵

| 分量 | 覆盖的路径 | 不覆盖的路径（**已知盲区**） |
| :--- | :--- | :--- |
| 堆分配 / 释放 | 进程内**所有**走 `#[global_allocator]` 的分配/释放/重分配（除本判据自己的线程外，按线程武装） | `mmap` 直映射、OS 缺页、`malloc` 的 arena 扩张（无 Hook 可见的 `alloc` 调用）；系统库内部自己的分配 |
| 锁 | 走 `rt_probe::RtLockProbe` 的加锁（尝试 / 等待分开计数）；实时路径上的见证试探 | **裸** `std::sync::Mutex` / `RwLock` / `Condvar` / `Once` / parking_lot 等直接调用（G2a 实测：完全看不见）；系统库内部的锁（cpal / CoreAudio / 分配器） |
| 阻塞 I/O | 走 `rt_probe::diag` 的诊断/I-O（唯一出口） | **裸** `println!` / `eprintln!` / `std::fs::*` / socket（G1 实测：看不见）；cpal 与 OS 内部的日志；未来的采样器磁盘流式读（尚未实现） |

### 5.2 一句话边界

> **这不是系统调用级拦截，也没有内核钩子。** 锁探针只看得见"经由本边界"的加锁，I/O 探针只看得见
> "经由 `rt_probe::diag`"的诊断。真实防线是**两半**：
> ① **约定**：实时路径上的诊断与锁必须走 `rt_probe`（`render_block`、`retire_or_stash` 的调用点就是这样接的）；
> ② **判据**：一旦走了边界，就必须为 0（本台账的四元组）。
> "新写一把裸锁 / 裸打印" 这种改动由**源码形状**那一半兜住（代码审查 + `scripts/guards/policy_check.py`），
> 本模块补的是"运行期真的发生了"这一半。两半都在时才是闭环；只报后半就是拔高。

### 5.3 其它如实登记的边界

1. **"实时线程"的身份**来自 `declare_rt_thread()`（判据自己声明的主线程），不是 cpal 的真实回调线程 ——
   本机与 CI 都不编译 cpal（AGENTS.md §5），真实回调线程上的读数**尚未取得**（见 needs N3）。
2. **窗口边界**：分配器与探针窗口都是"第一个量子之前预热、最后一个量子之后关闭"；
   预热把一次性路径（FPU 武装、首份快照武装、波表/包络模板、**探针锁的首次加锁**）排除在判据之外 ——
   排除是**显式**的（§6 记录了它排除掉的那一笔 64 字节分配），不是"看不见就算了"。
3. **⑤ 自动化只覆盖到"事件出队"**：实时侧目前对 `SetParam` 只计数（引擎还没有"参数目标表"），
   因此"自动化真的改变了声音"**未被本判据覆盖**（needs N1）。
4. **不在覆盖内的还有**：`rtrb` 队列的原子操作（那是无锁 SPSC，不是锁）、
   FTZ/DAZ 控制寄存器写入（一次性的、非 I/O 的系统调用语义）、
   以及走带读数镜面的原子写。

---

## 6. 一条顺带挖出来的平台事实：**新锁的首次加锁会分配**

诊断 ① 的第一次失败时发现：10 000 量子窗口 `allocations=1`，来源**不是引擎代码，而是探针自己**
（`quantum_enter` 的见证 `try_lock` 是该 `Mutex` 实例的首次加锁）。用独立程序（`rustc -O`，不在本仓库内）实测：

```text
mutex#0 first try_lock: alloc=1 bytes=64
mutex#1 first try_lock: alloc=1 bytes=64
...
same mutex 1st: alloc=1 bytes=64
same mutex 2nd: alloc=0 bytes=0
other thread 1st: alloc=1 bytes=64
other thread 2nd: alloc=0 bytes=0
```

结论（macOS / Apple M2，`rustc 1.92` 工具链）：

1. **每个 `std::sync::Mutex` 实例的首次加锁都会堆分配一次 64 字节**，同一实例第二次为 0（跨线程也只在首次分配）；
2. 因此**任何在实时路径上引入并首次触碰的锁都会分配** —— 这既是威胁（探针自己会制造它要测的违规），
   也是一条**部分**兜底（G2b 实测：裸锁的首次触碰会被分配探针抓住）；
3. 处置：`rt_probe::declare_rt_thread()` 在窗口之外调用 `warm_up()`（对实时路径那把锁先试探一次），
   判据自己新建的 `RtLockProbe` 也一律先 `try_lock_once()` 一次再开窗口；
   判据 `T` 把这条事实**登记为可观测读数**（冷窗口 / 暖窗口分别打印，断言的是"温暖之后恒为 0"这条与平台无关的不变式）。

---

## 7. 本机真跑 vs CI（严格区分）

| 项目 | 状态 | 证据 |
| :--- | :--- | :--- |
| **本机真跑**（M2，`--no-default-features`） | ✅ 已完成 | `bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test rt_zero_alloc` ⇒ **22/22 通过**，退出码 0，实测 **3.22 s**（含构建）；`cargo-local.sh` 打印的工作区 = `…/.worktrees/gate-rt-zero-alloc`（L30 要求的可回答性） |
| 本机全量 engine 目标 | ✅ 已完成 | `bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features` ⇒ 退出码 0（既有 4 个 `harness=false` 目标与 libtest 单元判据全绿，含 `rt_probe` 的 4 条单元判据） |
| 本机 clippy | ✅ 已完成 | `bash scripts/dev/cargo-local.sh clippy -p yeban-engine --no-default-features --all-targets -- -D warnings` ⇒ 退出码 0 |
| 本机 fmt | ✅ 已完成 | `bash scripts/dev/cargo-local.sh fmt --all --check` ⇒ 退出码 0 |
| 本机门禁（轻档 + crate 档） | ✅ 已完成 | `bash scripts/gates/run-gates.sh light` 与 `… crate yeban-engine` ⇒ 退出码 0（crate 档自动用 `--no-default-features` 轻量变体，D19） |
| **CI 判决** | ⏳ 见提交后的 `ci-verdict.sh` 读数 | CI 上跑的是**默认 feature** 全量（含 cpal 编译）与全部 4 个 `harness=false` 目标；**只有 CI 的判决算数**（SKILL「Honesty rules」） |

**本机做不到、也不许假装做到的**：本机不编译 cpal（AGENTS.md §5），因此
① 真实设备回调线程上的读数、② 默认 feature 下的 `yeban-engine` 全量构建，
本机都不产生判决 —— 它们只由 CI 覆盖。

---

## 8. needs（给集成者与后续线）

| # | needs | 性质 | 建议处置 |
| :-: | :--- | :--- | :--- |
| N1 | **实时侧的参数目标表**：⑤ 自动化场景只覆盖到"事件出队"（`SetParam` 目前只计数、不改 DSP） ⇒ "自动化真的改变了声音"未被覆盖 | 实现缺口 | 由后续线做"参数槽 → DSP 系数"映射；届时 ⑤ 的判据可加"输出随自动化变化"的断言 |
| N2 | **真实 cpal 回调线程上的探针读数**：本模块的"RT 线程"是判据自己声明的主线程 | 环境缺口 | 在有声卡的机器/CI 上跑 `device` feature 的窄判据；或在 `NullBackend` 的真实线程上开窗口 |
| N3 | **裸锁 / 裸 I/O 的盲区**（§4.3 G1 / G2a 实测）：只在"必须走边界"的约定下成立 | 结构性风险 | 若要**强制**，需要源码形状守卫（禁止 `std::sync::Mutex` / `println!` 出现在 `rt.rs` 的调用树里）或平台级拦截；属人类裁决范围（不要偷偷加 `libc` 依赖） |
| N4 | **`gate-status.md` 的 `MUST-GATE-001` 行**（本线无权改，见文首纪律） | 集成者动作 | 建议替换为下面 §8.1 的文本 |
| N5 | **`docs/DEVELOPMENT_LEDGER.md`**：本轮的教训（"探针自己会制造它要测的违规"：新锁首触 64 字节；"见证是探针不空转的唯一机械保证"）值得收进账本 | 集成者动作 | 由集成者按 L 编号追加；本线不写共享账本 |

### 8.1 建议的 `gate-status.md` 行（供集成者逐字替换）

```text
| `MUST-GATE-001` | 实时回调零分配/零释放/零 I/O/零锁 | **已接线** | **四个分量都有运行期判据**（`crates/yeban-engine/tests/rt_zero_alloc.rs`，`harness=false`，22 条判据 / 六场景 / 27 764 量子 + 1 063 次快照交换）：计数型全局分配器（按线程武装）断言 `allocations == 0 && deallocations == 0`；**新建探针边界** `crates/yeban-engine/src/rt_probe.rs`（`RtLockProbe` 见证型锁 + 唯一诊断/I-O 出口 `diag`）断言 `lock_blocking == 0 && lock_waits == 0 && io_requests == 0 && io_ops == 0`，并逐窗口附"探针被跑到过"的见证（`quanta_visits == 量子数`、`lock_try_successes == quanta_visits`）。探针的牙：3 组正对照（非 RT 阻塞加锁 / 争用等待 / 真实文件+控制台 I/O）+ 4 组注入（锁 / I/O / 分配 / 摘掉探针，各自打红后逐字节还原）。**诚实边界（实测）**：不是 syscall 级拦截 —— 裸 `eprintln!`（实测 28 171 行）与"窗口外已暖过的裸 `std::sync::Mutex`"**判据不变红**；⑤ 自动化只覆盖到事件出队（实时侧尚未改 DSP）。本机 `--no-default-features` 实测 22/22、3.35 s；CI 判决见 run id。判据与读数见 `docs/ledger/gate-rt-zero-alloc-notes.md` |
```

---

## 9. 复现命令（逐条都跑过）

```bash
# ① 四元组判据（本机真跑，零重依赖变体）
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test rt_zero_alloc

# ② 全部 engine 目标（含既有 4 个 harness=false 目标与单元判据）
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features

# ③ 静态检查（本机）
bash scripts/dev/cargo-local.sh clippy -p yeban-engine --no-default-features --all-targets -- -D warnings
bash scripts/dev/cargo-local.sh fmt --all --check

# ④ 门禁（轻档 + crate 档；**绝不管道**，L6/L17）
bash scripts/gates/run-gates.sh light && bash scripts/gates/run-gates.sh crate yeban-engine

# ⑤ 反馈判决（只有 CI 的判决算数）
bash scripts/dev/ci-verdict.sh line/gate-rt-zero-alloc
```

## 10. 证据文件的 sha256（冻结代码，提交前复核）

| 文件 | sha256 |
| :--- | :--- |
| `crates/yeban-engine/src/rt.rs` | `ccd0c2d0a35428f29501de043af9fca508fcb88b941b19449c30efbac2a6f5cf` |
| `crates/yeban-engine/src/snapshot.rs` | `e9765f1348a9b15476c1d77385853c200bdf918e37a8ac0418a21bbd347bdff9` |
| `crates/yeban-engine/tests/rt_zero_alloc.rs` | `86dd3ef653b5ed933b35a6f6145628cedec46a8d1aef582a1298f511461f159a` |
| `crates/yeban-engine/src/rt_probe.rs` | `790e25a17149b73b32c5ac4d4bcba9f39092f2a5d38a1bd09f622a1c6b5b1bc3` |

（`rt.rs` 的哈希在**四次注入的每次还原之后**都用 `sha256sum -c` 校验为同一个值。）
