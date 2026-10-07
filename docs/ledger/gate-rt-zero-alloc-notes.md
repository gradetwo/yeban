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
> **CI 判决（已读回）**：`line/gate-rt-zero-alloc` 的 run **37270170716 = success**，
> `rust (yeban-engine)` 腿（job id 111635332023）在 **Linux** 上同一判据 **22 / 22 通过**，
> 六个场景的四元组与 macOS **逐项相同**（见 §3.1）。

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

## 3.1 本机（macOS / M2）与 CI（Linux）的读数对照

两边的判据都是**同一条命令的同一个目标**（CI 上是 `cargo test --all-targets` 里的 `rt_zero_alloc` 目标），
因此这张对照表同时是"判据不依赖平台"的证据。

| 读数 | 本机 macOS / M2 | CI Linux（run 37270170716，engine 腿） | 差异解读 |
| :--- | :--- | :--- | :--- |
| 判据汇总 | 22 / 22 通过 | **22 / 22 通过** | 一致 |
| ① 纯渲染 10 000 量子四元组 | `0 / 0 / 0 / 0 / 0 / 0` | `0 / 0 / 0 / 0 / 0 / 0` | 一致 |
| ③ 走带（命令 202 / 推进 2151 / tick 18259） | 同上 | 同上（**逐项相同**） | 一致（走带是整数有理数推进，确定性） |
| ⑥ 限制器（压过 256 098 / 最大压限 0.8878 / 窃取 27 / 触发 83 / NaN 0） | 同上 | 同上（**逐项相同**） | 一致（DSP 走 IEEE 精确类，见 `MUST-GATE-003` 的口径） |
| ⑦b 争用等待 | `held=true joined=true released=true lock_waits=1` | 同（`lock_waits=1`，其中 `joined=true`） | 一致（250 ms 固定持锁在 GitHub runner 上足够） |
| ⑦c I/O 有牙 | `io_requests=3 io_ops=3`、sink `emits 2 -> 5`、文件行数 `0 -> 3` | 同（**逐项相同**） | 一致 |
| 判据 T 的**冷窗口** | `alloc=1`（§6 的 64 字节陷阱） | **`alloc=0`** | **平台差异**：该陷阱在本 CI runner 上不出现 ⇒ 只打印不断言是正确选择 |
| libtest 单元判据 | 135 passed | 145 passed | CI 跑的是**默认 feature**（含 cpal），多出的 10 条是设备侧判据 |
| 墙钟 | 3.22 s（含构建） | 整腿 **1 分 6 秒**（含 cpal 编译与本 crate 全部目标） | 规模差异，非判据差异 |

**结论**：四元组判据本身与平台无关；唯一出现平台差异的是**探针自身**的首触开销（§6），
而它的处置（窗口外温暖 + 只打印冷读数）在两边都成立。

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
| **CI 判决** | ✅ **run 37270170716 = success**（`line/gate-rt-zero-alloc`，tip `3e24d97`） | 六条腿全绿：`checks` 46 s / `plan` 7 s / `deny` 51 s / `lockfile` 18 s / **`rust (yeban-engine)` 1 m 6 s** / `rust (yeban-app)` 2 m 50 s（`windows` 与 `rust (workspace 全量)` 按路径过滤跳过）。engine 腿里本判据 **22 / 22 通过**，Linux 读数与 macOS 逐项对照见 §3.1；CI 上跑的是**默认 feature**（含 cpal）与全部 4 个 `harness = false` 目标 |

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
| `MUST-GATE-001` | 实时回调零分配/零释放/零 I/O/零锁 | **已接线** | **四个分量都有运行期判据**（`crates/yeban-engine/tests/rt_zero_alloc.rs`，`harness=false`，22 条判据 / 六场景 / 27 764 量子 + 1 063 次快照交换）：计数型全局分配器（按线程武装）断言 `allocations == 0 && deallocations == 0`；**新建探针边界** `crates/yeban-engine/src/rt_probe.rs`（`RtLockProbe` 见证型锁 + 唯一诊断/I-O 出口 `diag`）断言 `lock_blocking == 0 && lock_waits == 0 && io_requests == 0 && io_ops == 0`，并逐窗口附"探针被跑到过"的见证（`quanta_visits == 量子数`、`lock_try_successes == quanta_visits`）。探针的牙：3 组正对照（非 RT 阻塞加锁 / 争用等待 / 真实文件+控制台 I/O）+ 4 组注入（锁 / I/O / 分配 / 摘掉探针，各自打红后逐字节还原）。**诚实边界（实测）**：不是 syscall 级拦截 —— 裸 `eprintln!`（实测 28 171 行）与"窗口外已暖过的裸 `std::sync::Mutex`"**判据不变红**；⑤ 自动化只覆盖到事件出队（实时侧尚未改 DSP）。本机 `--no-default-features` 实测 22/22、3.22 s；**CI run 37270170716 = success**（`rust (yeban-engine)` 腿 22/22，Linux 与 macOS 读数逐项对照见 notes §3.1）。判据与读数见 `docs/ledger/gate-rt-zero-alloc-notes.md` |
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
| `crates/yeban-engine/src/rt.rs` | `bf37d1a06c2e0782e0f60fdaed95f51e0337c2719cf866c6b858d3a8a395059b` |
| `crates/yeban-engine/src/snapshot.rs` | `e9765f1348a9b15476c1d77385853c200bdf918e37a8ac0418a21bbd347bdff9` |
| `crates/yeban-engine/tests/rt_zero_alloc.rs` | `d90e0abe4f3f0f1d11f613f61745e99991b7f80b63a937716614f6ce5a73fe51` |
| `crates/yeban-engine/src/rt_probe.rs` | `790e25a17149b73b32c5ac4d4bcba9f39092f2a5d38a1bd09f622a1c6b5b1bc3` |

（`rt.rs` 的哈希在**四次注入的每次还原之后**都用 `sha256sum -c` 校验为同一个值。）
⚠ **2026-10-07 重测（`3502353` 改动了 `rt.rs` 与 `tests/rt_zero_alloc.rs` 两个文件）**：
本表 `rt.rs` 一行原记 `ccd0c2d0a35428f29501de043af9fca508fcb88b941b19449c30efbac2a6f5cf`、
`tests/rt_zero_alloc.rs` 一行原记 `86dd3ef653b5ed933b35a6f6145628cedec46a8d1aef582a1298f511461f159a`
（那一刻的真话，原值保留在这里）；上面两行的新值用
`shasum -a 256 crates/yeban-engine/src/rt.rs crates/yeban-engine/tests/rt_zero_alloc.rs` 实测。
本表另外两行（`snapshot.rs` / `rt_probe.rs`）**本提交未触及**，未重测。新增文件
`crates/yeban-engine/tests/pdc_mix_path.rs` 的哈希见 §17。

---

# 第 611 轮补（扩场景）：**六场景 → 十场景**

> 本节由一次**扩场景**切片追加（任务的原话是"实时零分配扩场景"）。
> **本切片没有修改 `docs/ledger/gate-status.md` 的任何一行**，也没有改 `MUST-GATE-001`
> 或 `MUST-GATE-012` 的状态：`MUST-GATE-001` 本来就是 **已接线**（已接线是最高状态，
> 只能被削弱、不能被"推进"），因此本轮是**给一个已经接线的门禁加证据，而不是移动它**。
> 本切片碰过的文件只有两个：`crates/yeban-engine/tests/rt_zero_alloc.rs`（判据）与本文（记录）。

## 11. 四个新场景：各自覆盖什么、为什么那条路径**可能**分配、怎么检出分配

器件**完全复用**：计数型全局分配器、`window()`、`Scenario`、`Report` 都是本文件
原有的机制，**没有新增 `unsafe`**（`#[global_allocator]` 仍然只有本文件里那一个
`CountingAllocator`，定义在 `tests/` 下 ⇒ 不进任何发行二进制；见 §15）。
判定口径不变：窗口内四元组（`alloc` / `dealloc` / `lock_blocking` / `lock_waits` /
`io_requests` / `io_ops`）**逐项为 0**，外加"探针真的被跑到过"的见证
（`quanta_visits == 期望量子数`、`lock_try_successes == quanta_visits`）。

| # | 场景 | 覆盖的实时路径 | 为什么那条路径**可能**分配 | 分配怎么被检出 |
| :-: | :--- | :--- | :--- | :--- |
| ⑬ | 回调缓冲长度边界：`[1,2,3,127,128,129,1024,1025]` 帧 × 25 轮 + 每轮一次 129 样本（非帧对齐） | `process_quantum` 的 `while offset < total_frames` 切块 + 逐帧交错拷贝 + `AudioBlock::set_frames` 的短块路径 + 尾部残余样本契约 | 那是**唯一**把"设备给的任意长度交错缓冲"切成整量子的地方；一个自然的实现会 `to_vec()`/`resize()`，或为 `<128` 帧的尾块另建缓冲 | 同一个计数分配器；再加**哨兵检查**：帧对齐缓冲里不许残留哨兵，非对齐缓冲必须**恰好**残留 1 个 |
| ⑭ | 采样率 × 项目声明 `block_size` 全组合（5×5×2 = 50 次切换，2 000 量子） | `render_block` 的**重新武装**分支：`MeterBank::set_quanta_per_second`、`SynthEngine::begin_snapshot`（按新采样率重算频率/相位）、`Transport::arm` | 换快照时若重建 `Vec`/`BTreeMap`（调度表、声相表、PDC 计划）就会分配；既有场景②只改 `revision`（内容相同）⇒ 这条分支的"内容真的变了"那一半从未被覆盖 | 同一个计数分配器；覆盖度判据同时钉住"武装值逐项等于 `采样率/128`"（50 项）以证明重新武装**真的发生了** |
| ⑮ | **播放中**「`Op` 层编辑 / 撤销 → 重新发布快照」2 000 轮 + 主线程排空退役队列 | `Op::apply` / `Op::apply_inverse`（真实撤销载荷）→ `EngineSnapshot::from_project` → 原子发布 → 实时侧"新修订 ⇒ 切换 + 旧快照入退役队列" | 撤销交互的组合路径：一边出声、一边被模型改写、一边换拓扑。既有场景②不播放，场景③不发布 ⇒ 这个组合从未被覆盖 | 同一个计数分配器；同时断言"编辑与撤销都真的应用过"（各 1 000 次）与"旧快照只在主线程释放" |
| ⑯ | 满批事件洪峰：`SCRATCH_EVENTS` = 128 条/量子 × 500 量子（参数 + 音符 + 走带混排） | `EventReceiver::drain_with` 把整批搬进 `[EngineEvent; SCRATCH_EVENTS]` 栈数组的**满块边界** | 一个自然的实现会 `Vec::from_iter`/`extend`，或在"批比缓冲大"时分配溢出缓冲；既有场景⑤每量子只发 **1** 条 ⇒ 满块边界从未被覆盖 | 同一个计数分配器；同时断言 64 000 条**全部**被通道接受并被实时侧计数 |

### 11.1 实测读数（本机 M2，`--no-default-features`，冻结代码）

命令（L30：`cargo-local.sh` 会打印它实际用的工作区）：

```bash
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test rt_zero_alloc
```

| # | 量子 | 子窗口 | 探针经过 | 试探成功 | alloc | dealloc | lock_blocking | lock_waits | io_requests | io_ops |
| :-: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ⑬ | 625 | 1 | 625 | 625 | 0 | 0 | 0 | 0 | 0 | 0 |
| ⑭ | 2 000 | 50 | 2 000 | 2 000 | 0 | 0 | 0 | 0 | 0 | 0 |
| ⑮ | 2 000 | 2 000 | 2 000 | 2 000 | 0 | 0 | 0 | 0 | 0 | 0 |
| ⑯ | 500 | 500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 0 |

覆盖度证据（逐条来自判据 detail 行，防"窗口里什么都没跑"的假绿）：

| # | 覆盖度读数（原文） |
| :-: | :--- |
| ⑬ | `长度集合=[1, 2, 3, 127, 128, 129, 1024, 1025] × 25 轮 + 每轮一次 129 样本；共喂入 60975 帧 / 625 个量子；帧对齐缓冲残留哨兵=0`；⑬c：`非对齐 129 样本的残余哨兵=25（要求 25，即每轮恰好 1 个）；单次 1025 帧 ⇒ 量子数=9（要求 9）` |
| ⑭ | `切换 50 次（5 采样率 × 5 声明缓冲 × 2 轮）；武装的量子/秒=5 个不同值；走带位置 5 -> 6932 tick；合成样本=256128`；⑭c：`武装值逐项比对=true（要求 true；50 项）` |
| ⑮ | `编辑=1000 撤销=1000 发布=2000；实际切换=2000 主线程回收=2000；走带位置=10245 tick 状态=Playing；释放线程=主线程?true 外线程排空=0 退役满寄存=0` |
| ⑯ | `每量子 128 条 × 500 量子 = 尝试 64000 条，通道接受 64000 条；实时侧应用=64001 批量出队=501；通道容量=1024` |

**判据总数：24 → 32**（`判据汇总: 32 / 32 通过`，退出码 0；⚠ 后续的 PDC 接线切片
（`3502353`）再把它推到 **34**，见 §17）。其中 ⑮ 顺带把
`MUST-GATE-012`（退役队列）关心的两条读数也钉进了播放场景：`release_thread_is_main=true`、
`foreign_drains=0`、`退役满寄存=0`——但**门禁 012 的行没有被本切片改动**。

### 11.2 哪些跑在**真回调**、哪些只是**测试里的替身**（这条最容易含混，必须写明）

| 场景 | 跑在哪 | 与规范那句的关系 |
| :--- | :--- | :--- |
| ⑬⑭⑮⑯（以及既有的①~⑥、⑪、⑫） | **判据自己声明的主线程**调用 `EngineRuntime::process_quantum` —— 即**与 cpal 回调同一份代码路径**，但**不是** cpal 的真实回调线程 | 覆盖"回调函数体"，**不覆盖**"设备把回调跑在哪个线程上" |
| 真 cpal 回调 | `crates/yeban-engine/src/device.rs:390-391`（`build_output_stream` 的闭包只调 `process_quantum`；⚠ 行号已漂移，更正见本行第 3 格：闭包现于 `:427-429`） | 这条路径**从未被运行期分配判据覆盖过**：它需要 `device` feature（cpal）**且**一台有声卡的机器；本机按纪律不编译重载，托管 runner 无音频设备。**【后续更正（2026-10-08，设备腿 `5d528b7` 之后）】**：上文那句「从未被运行期分配判据覆盖过」作为**结论已假** ✗ —— 场景 **⑳**（`crates/yeban-engine/tests/rt_zero_alloc.rs` 的 `scenario_device_callback_body`，2 000 次调用）现在覆盖**设备回调体函数** `device::render_callback` —— cpal 建流的闭包、`NullBackend::render` 与判据调的是**同一个函数**（闭包体 `crates/yeban-engine/src/device.rs:428` 调它；它只有一行 `process_quantum`）⇒ "往回调体里加分配 / 锁 / 阻塞 I/O / 日志"会在 ⑳ 变红。⚠ ⑳ **仍然不覆盖** cpal 的**闭包本身**与**流**（`build_output_stream` → `play`），也**不覆盖**控制面（配置协商与错误回调计数）—— ⑳ 覆盖的是**回调体**，**不是**"设备路径已被覆盖" ✗。**行号更正**：闭包现于 `crates/yeban-engine/src/device.rs:427-429`，旧记 `:390-391`。**与本格上半句仍然一致的部分**：真 cpal 回调线程 / 设备开流关流确实需要 `device` feature（cpal）且一台有声卡的机器；本机按纪律不编译重载，托管 runner 无音频设备。 |
| `NullBackend`（`device.rs:414`，同一份 `EngineRuntime`；⚠ 行号更正见本行第 3 格：结构体现于 `device.rs:452`） | `device.rs` 的 10 条单元判据（含 `null_backend_drives_the_same_render_path_without_a_device`） | 它是**替身**（在内存里驱动渲染），**不是**真实设备；而且 `device.rs` 里**没有**任何分配断言（`grep -n alloc crates/yeban-engine/src/device.rs` ⇒ 0 行）⇒ 它没有给本门禁提供证据。**【后续更正（2026-10-08，设备腿 `5d528b7` 之后）】**：① 行号更正 —— `NullBackend` 结构体现于 `crates/yeban-engine/src/device.rs:452`，旧记 `:414`；② 上面那条 `grep` 现在**不是 0 行而是 5 行**（`grep -n alloc crates/yeban-engine/src/device.rs` ⇒ `13`、`17`、`374`、`788`、`789`），但 5 行命中的**全部是文件名 `rt_zero_alloc.rs` 里的字串 `alloc`**，**不是**分配断言 ⇒ "`device.rs` 里没有任何分配断言"这个**结论仍然成立** ✓；③ 结论"它没有给本门禁提供证据"**仍然成立** —— ⑳ 直接调用 `device::render_callback`，**不**经 `NullBackend::render`。 |

⇒ 一句话：**本轮的四个新场景都不在真回调线程上，它们扩的是"回调函数体"的覆盖集，不是"回调宿主"的覆盖集。**

## 12. 注入 I5–I8：**每个新场景各自能被单独打红**

方法照 §4.2：把 `crates/yeban-engine/src/rt.rs` 复制到 `/tmp/rtbak/rt.rs.orig`，
每次只改一处，跑同一条命令，记录红行，最后 `cp` 还原并**用 `cmp` + `sha256` 双证**逐字节相同
（不用 `git checkout`，理由见 AGENTS.md §6.2）。四次注入后 `git diff -- crates/yeban-engine/src/` **为空**。

`rt.rs` 冻结哈希 `sha256 = 80893dce9e361aa4eff69634f4357ad78b537307ee72d558de2898d70ecfebe3`
（注入前 / 每次还原后**逐字节相同**。⚠ 这是**第 611 轮那一刻**的 `rt.rs`；
`3502353`（PDC 接线）之后的新值见 §17。）

| # | 注入点（`rt.rs`） | 判据结果 | 原始红行（节选，逐字） |
| :-: | :--- | :--- | :--- |
| I5 | `process_quantum` 切块循环里，当 `frames < DEFAULT_BLOCK_FRAMES` 时 `Vec::<u8>::with_capacity(1)` | **31 / 32**，**只有 ⑬** 红 | `[MUST-GATE-001] FAIL 判据 ⑬ …: ⑬回调缓冲长度边界；四元组[alloc=175 dealloc=175 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0]；…探针经过=625（期望 625）…` —— 175 = 25 轮 × 7 个短块（1/2/3/127/64/129 的尾块/1025 的尾块），**与手算逐项吻合** |
| I6 | 重新武装分支里，当 `current.sample_rate() != 48_000` 时分配一次 | **31 / 32**，**只有 ⑭** 红 | `FAIL 判据 ⑭ …: ⑭采样率/声明缓冲切换；四元组[alloc=40 dealloc=40 …]` —— 40 = 4 个非 48 kHz 采样率 × 5 个声明缓冲 × 2 轮 |
| I7 | 重新武装分支里，当有轨 `volume_db() > 0.0`（= 编辑已生效）时分配一次 | **31 / 32**，**只有 ⑮** 红 | `FAIL 判据 ⑮ …: ⑮播放中编辑/撤销；四元组[alloc=1000 dealloc=1000 …]；…编辑=1000 撤销=1000…` —— 1000 = "编辑已应用"的那一半 |
| I8 | `events.drain_with` 闭包里，当 `applied == SCRATCH_EVENTS` 时分配一次 | **31 / 32**，**只有 ⑯** 红 | `FAIL 判据 ⑯ …: ⑯满批事件洪峰；四元组[alloc=500 dealloc=500 …]；…每量子 128 条 × 500 量子…` —— 500 = 每量子恰好一次 |

**没有"注入了却不变红"的情形**：四次注入各自命中且**只**命中它针对的那个新场景
（这正是把注入条件绑到"该场景独有的事实"上的目的：⑬ 的短块、⑭ 的非 48 kHz、
⑮ 的编辑值、⑯ 的满批）。还原后复跑：**32 / 32 通过**。

## 13. 边界登记（**实测**，不是推断）：两条会到达 `diag` 边界的实时路径

§2.3 说 `rt_probe::diag` 是实时路径上**唯一**的诊断/IO 出口，判据要求 `io_requests == 0`。
但代码里有**两条**会**主动**调用它的路径。本节把"它们真的会被触发、并且真的会被判据抓到"
从推断变成实测 —— **它们不在本判据的覆盖集内**（纳入就会变红）。

### 13.1 退役队列欠容 ⇒ `SnapshotRetireStash`

探法（临时，已还原）：把判据的 `Rig::new` 里退役通道容量 64 改成 **2**，其它一字不动。

```text
26 / 32 通过
FAIL 判据 ② …: ②快照交换；四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=125 io_ops=125]；…发布=1063 实际切换=315 主线程回收=314 退役队列满寄存=125
FAIL 判据 ⑭ …: 四元组[alloc=0 dealloc=0 … io_requests=1 io_ops=1]；…武装的量子/秒=1 个不同值
FAIL 判据 ⑮ …: 四元组[alloc=0 dealloc=0 … io_requests=400 io_ops=400]；…实际切换=802 主线程回收=801 退役队列满寄存=400
```

### 13.2 电平计量容量不足 ⇒ `MeterCapacityDrop`

探法（临时，已还原）：把 `crates/yeban-engine/src/meter.rs` 的 `SCRATCH_METERS` 由 256 改成 **2**。

```text
24 / 32 通过
FAIL 判据 ① …: 四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=10000 io_ops=10000]；…quanta=10001 …
FAIL 判据 ⑬ …: 四元组[alloc=0 dealloc=0 … io_requests=625 io_ops=625]；…共喂入 60975 帧 / 625 个量子…
```

### 13.3 结论（必须与"全 0"一起读）

1. **分配分量仍然是 0**（两次探法的红行里 `alloc=0 dealloc=0`）⇒ 两条溢出路径**不会**
   在实时线程上分配/释放；"零堆释放"这条设计目标（[ARCH-RT-002]）成立。
2. 但 `io_requests > 0`，且在**装了见证 sink 的这个判据二进制里** `io_ops > 0` 也成立
   （sink 真的写文件 + 真的 `stderr()` 打印）⇒ 判据按 `io_requests == 0` 判红。
3. 因此准确的说法是：**"实时路径在非溢出条件下不产生诊断事件"**。
   "实时路径上从不产生诊断事件"**不成立**，本判据的覆盖集**不含**这两条溢出路径。
   **【`2352f9f` 之后补，本句已过期】**：那两条溢出路径**已改走纯计数入口** `rt_probe::note_suppressed`（从不读 `SINK`）⇒ 本判据的覆盖集**现已包含**它们 —— 新增场景 ⑱（退役队列容量 1、控制面故意不排空）与 ⑲（300 轨 > `SCRATCH_METERS−1`），判据 **34 → 38**，两条都在**见证型写文件 sink 装着**时断言 `io_requests == 0 && io_ops == 0`（且 `suppressed_diag_events > 0`）。本节上面两次注入的红行是**那一刻的真话**，保留不改；关闭记录见 §16 的 N6 行。
4. 生产影响**有界但非零**：发布构建**不安装 sink** ⇒ `diag` 退化为两次 `Relaxed` 原子自增
   （无系统调用、无分配、不等待），红线 7 的"零阻塞 I/O"未破；
   但只要有人给发布构建装了写文件的 sink（例如某个诊断开关），这两条路径就会变成
   **实时线程上的真实文件/控制台写**。⇒ 记 needs N6。

## 14. 归属更正（记录在案，防止下一个读者踩同一个坑）

**本切片收到的任务把"实时零分配扩场景"这句话配到了 `MUST-GATE-012` 上；那个配对是错的。**
治理目标给的是**两份列表**（六条门禁 ID / 五条短语），把两份列表**按位一一对应**就会得到错误的配对。
正确映射（以`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` 的规范定义为准）：

| 短语 | 门禁 | 规范定义（逐字，`…ROADMAP.md`） |
| :--- | :--- | :--- |
| **实时零分配扩场景** | **`MUST-GATE-001`** | `[MUST-GATE-001] 音频线程零安全违规 (Zero Glitch / Zero Alloc)`（`:380`）；规范背书 `ARCH-RT-001` |
| **快照退役高频压测** | **`MUST-GATE-012`** | `[MUST-GATE-012] 音频退役回收队列零泄漏`（`:391`）；规范背书 `ARCH-RT-002` |

在本切片**之前**，"实时零分配扩场景"这句话在全仓只出现过**一处**（`grep -rn` 命中 1 行）：
`docs/DEVELOPMENT_LEDGER.md:2047`，而它在那一行里指的正是 **`MUST-GATE-001`**
（"…然后开 `MUST-GATE-001`（实时零分配扩场景…）"）。（本文现在也引用了这句话，所以 `grep -c "实时零分配扩场景" docs/**/*.md` 的读数已经变了 ——
**行数是会随文本变化的，别把这一刻的计数当永久事实**。）
⇒ 本轮的工作属于 **001**；`MUST-GATE-012` 的行与状态**未被本切片改动**。

## 15. 本轮的冻结哈希、依赖图度量与复现命令

被改动的文件**只有** `crates/yeban-engine/tests/rt_zero_alloc.rs`（外加本文，属记录）。
`crates/yeban-engine/src/**` 在全部注入与探法之后**逐字节还原**：`git diff -- crates/yeban-engine/src/` 为空。

| 文件 | sha256 |
| :--- | :--- |
| `crates/yeban-engine/tests/rt_zero_alloc.rs`（本轮冻结值） | `04debc585b8b5d59cc241ef01f4f2beafb897566e4340ea047c796f00a2ba639` |
| `crates/yeban-engine/src/rt.rs`（注入前/还原后；本切片**未**改） | `80893dce9e361aa4eff69634f4357ad78b537307ee72d558de2898d70ecfebe3` |
| `crates/yeban-engine/src/meter.rs`（探法前/还原后；本切片**未**改） | `74d19d20df78a3719a1bf624d000946cb0db049c9607af54d940a59bb96cff1a` |
| `crates/yeban-engine/src/snapshot.rs`（本切片**未**改） | `22e3af3a9b1471058f6f995c0e151a02cf12a332fcaed8f93d3ed72ef3b8c574` |

⚠ 本表 `crates/yeban-engine/src/rt.rs` 与 `crates/yeban-engine/tests/rt_zero_alloc.rs` 两行是
**第 611 轮那一刻**的冻结值；`3502353`（PDC 接线）改动了这两个文件，当前值见 §17。
`src/meter.rs` 与 `src/snapshot.rs` 两行未被本提交触及（§17 未重测）。

**依赖图度量**（先说口径）：*`cargo tree -p <crate> -e normal --locked --prefix none`
里**互不相同的 `name version` 条目数**（单位 = 依赖条目，不是行数 —— AGENTS.md §6.5）*：

| crate | 度量（本切片之后；⚠ 口径见下面的更正） |
| :--- | ---: |
| `yeban-engine`（默认 feature，含 cpal） | **75** |
| `yeban-engine`（`--no-default-features`，本机变体） | **56** |
| `yeban-model`（默认 feature） | **34** |

⚠ **口径更正（2026-10-07 重测，`3502353`）**：上表三个值（75 / 56 / 34）**不是**本节
声称的"互不相同的 `name version` 条目数"，而是下面旧命令 `… | sort -u | wc -l` 数出来的
**原始输出行数**（cargo 把已展示过的子树重渲染成 `(*)`，各占一行；再加上 `cargo-local.sh`
打印横幅那一行）。按本节**写明的口径**（`cargo tree -p <crate> -e normal --locked --prefix none`
里互不相同的 `name version` 条目）重测当前树（`3502353`，未改任何清单），读数是
**`yeban-engine` 默认 57 / `--no-default-features` 44 / `yeban-model` 29**。命令：

```bash
bash scripts/dev/cargo-local.sh tree -p <crate> -e normal --locked --prefix none \
  | tail -n +2 | sed 's/ (\*)$//' | sort -u | wc -l
```

旧值保留在此（那是那一刻的真话，不改写）；`tail -n +2` 是为了丢掉 `cargo-local.sh` 的横幅，
`sed 's/ (\*)$//'` 是为了把 cargo 的重渲染行折回同一个 `name version` 条目。

"之后"与"之前"**由结构证明相同**，而不是又量一遍：本切片没有碰任何清单 ——
`git diff --name-only HEAD -- '*Cargo.toml' 'Cargo.lock'` **输出为空**。
（本切片只改 `tests/**` 与 `docs/ledger/*-notes.md`，两者都不在依赖图里。）
`crates/yeban-engine/Cargo.toml` 也**未**新增 `[dev-dependencies]`：计数型分配器住在
`tests/rt_zero_alloc.rs` 里（`#[global_allocator]` + 文件内 `unsafe impl GlobalAlloc`），
**本来就是既有的**，没有引入任何新依赖、也没有往 `src/` 加 `unsafe`。

```bash
# ① 本判据（本机真跑，零重依赖变体）
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test rt_zero_alloc   # 32/32, exit 0

# ② 全部 engine 目标
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features                        # exit 0

# ③ 静态检查
bash scripts/dev/cargo-local.sh fmt --all --check
bash scripts/dev/cargo-local.sh check  -p yeban-engine --all-targets --no-default-features
bash scripts/dev/cargo-local.sh clippy -p yeban-engine --all-targets --no-default-features -- -D warnings

# ④ 依赖图度量（口径 = 互不相同的 name version 条目；必须剥掉 cargo 的 " (*)" 与 cargo-local 的横幅）
bash scripts/dev/cargo-local.sh tree -p yeban-engine -e normal --locked --prefix none \
  | tail -n +2 | sed 's/ (\*)$//' | sort -u | wc -l
# ⚠ 旧版这里只写 `| sort -u | wc -l`：那数的是原始输出行（含 (*) 重渲染与横幅），
#    第 611 轮因此把 57 记成 75。
git diff --name-only HEAD -- '*Cargo.toml' 'Cargo.lock'   # 必须为空 ⇒ 依赖图与 HEAD 相同

# ⑤ 门禁
bash scripts/gates/run-gates.sh light
```

## 16. 本轮新增的 needs

| # | needs | 性质 | 建议处置 |
| :-: | :--- | :--- | :--- |
| N6 | ~~**两条溢出路径会到达 `diag` 边界**（§13 实测：退役队列欠容 / 电平容量不足）⇒ "实时路径上从不产生诊断事件"不成立；发布构建无 sink 时它只是原子自增，但一旦有人给发布构建装了写文件的 sink，那两条路径就是实时线程上的真实写~~ ⇒ **已关闭**：**负责人裁决 = 选项 A，已落地**（commit `2352f9f`）—— 五处 `rt_probe::diag` 调用点全部改走**纯计数**入口 `rt_probe::note_suppressed`（`rt_probe.rs:483`），它**从不读 `SINK`**；`diag` 保留为**非实时路径**与判据正对照的 I/O 边界 | 结构 + 裁决（**已关闭**） | **判据从 34 条扩到 38 条**：新增场景 ⑱（退役队列容量 1、控制面故意不排空）与 ⑲（300 轨 > `SCRATCH_METERS−1`），两条都在**见证型写文件 sink 装着**时断言完整零四元组**（含 `io_requests == 0 && io_ops == 0`）**且 `suppressed_diag_events > 0`。**注入 I9**（把 `rt_probe::diag` 加回电平溢出分支）⇒ **仅 ⑲ 红**：`四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=100 io_ops=100]`（`37 / 38 通过`）—— `alloc=0` 把它归因到 **I/O** 而非分配；还原后 `cmp` 无差异 + sha256 `ad8b283b0feb87891205ccb02a8004824e19cb97ea748455ef9f99f0595f5358`（`src/rt.rs`，两侧同值）+ `38 / 38 通过`。**计数仍可离线观察**：`rt_probe::totals()` / `suppressed_by_kind(event)`，以及既有的 `EngineStats::{meter_capacity_drops, snapshot_stash_events}`（经 `EngineRuntime::stats()`）⇒ 此前那句"任何零 I/O 的声明都必须写明不含溢出路径"**随之解除** |
| N7 | **真 cpal 回调线程 / 设备开流-关流**仍未覆盖（§11.2）：需要 `device` feature + 有声卡的机器；`NullBackend` 是替身且无分配断言 | 环境缺口 | 在有声卡的参考机上跑一条 `device` feature 的窄判据（开流→推若干缓冲→关流），把分配计数开在回调线程上；或裁决"以 `process_quantum` 的覆盖为准"并写明 |
| N8 | **插件（VST3/CLAP）路径不存在**：`yeban-plugin-host` / `yeban-vst` 是故意空的骨架（v2.0.0 阶段，`AGENTS.md` 附录 C.4） | 不是缺口 | **不要**为凑场景去提前实现它们；等 v2.0.0 的宿主真正落地后再加场景 |

---

## 17. 第 612 轮补（`ROAD-M2-004` PDC 接线）：`32 → 34` 条判据与场景 ⑰

> 本节由 `3502353`（把 PDC 计划接进实时混音路径）追加。本切片**只**改判据与登记：
> `MUST-GATE-001` 仍是 **已接线**（最高状态，不给一个已接线的门禁"升级"），
> `docs/ledger/gate-status.md` 的任何一行都**没有被本切片改动**。
> 改动文件（`git show --stat 3502353`）：`crates/yeban-engine/src/graph.rs`、
> `crates/yeban-engine/src/rt.rs`、新增 `crates/yeban-engine/tests/pdc_mix_path.rs`、
> `crates/yeban-engine/tests/rt_zero_alloc.rs`。

**冻结哈希（本次重测）** —— 命令：
`shasum -a 256 crates/yeban-engine/src/rt.rs crates/yeban-engine/tests/rt_zero_alloc.rs crates/yeban-engine/tests/pdc_mix_path.rs`：

| 文件 | sha256 |
| :--- | :--- |
| `crates/yeban-engine/src/rt.rs` | `bf37d1a06c2e0782e0f60fdaed95f51e0337c2719cf866c6b858d3a8a395059b` |
| `crates/yeban-engine/tests/rt_zero_alloc.rs` | `d90e0abe4f3f0f1d11f613f61745e99991b7f80b63a937716614f6ce5a73fe51` |
| `crates/yeban-engine/tests/pdc_mix_path.rs`（新增） | `045e33d8fc50f94154172b0f9227ab77d689173d4a9fc4cdd7d87cfed203c145` |

本文件此前记录过这两个文件的另外几个值（§4.2 与 §10 的 `ccd0c2d0…`、§10 的 `86dd3ef6…`、
§12 与 §15 的 `80893dce…`、§15 的 `04debc58…`），它们都是**各自那一刻**的 rt.rs /
rt_zero_alloc.rs。本次**不改写**那些历史行，只在上文加了指针，并在这里给出当前值。
`src/meter.rs`、`src/snapshot.rs`、`src/rt_probe.rs` 未被 `3502353` 触及，**未重测**。

**判据总数 32 → 34**：`[MUST-GATE-001] 判据汇总: 34 / 34 通过`（本机 M2，
`--no-default-features`，退出码 0）。本文件的净增是场景 ⑰ 的 `⑰` 与 `⑰c` 两条
（32 → 34）；新文件 `tests/pdc_mix_path.rs` 的 2 条**不属于本文件**，另跑
`test result: ok. 2 passed; 0 failed`。

### 17.1 场景 ⑰：覆盖什么

| # | 场景 | 覆盖的实时路径 | 为什么那条路径**可能**分配 | 分配怎么被检出 |
| :-: | :--- | :--- | :--- | :--- |
| ⑰ | PDC 补偿延迟线：2 000 量子稳态 + 1 000 量子跨快照**重新武装**（32 → 96 帧） | `render_block` 快照边界的 `CompensationBank::rearm`（`set_delay` 分支）与逐轨 `apply` 的逐样本环形延迟读写（`ROAD-M2-004` 接线新增） | 一个自然的实现会按计划重建 `Vec`（`from_plan`）或 `truncate` 旧延迟线（`Drop` ⇒ `dealloc`）；`rearm` 只写节点键 + `set_delay`，所以必须证明这条路径真的零分配 | 同一个计数分配器；覆盖度判据 ⑰c 另钉住"武装延迟逐节点等于计划、至少一条 > 0、窗口里真有样本流过、重新武装后等于新计划、未武装/被钳均为 0" |

### 17.2 ⑰ 的实测四元组与见证值（本机 M2，`--no-default-features`）

| # | 量子 | 子窗口 | 探针经过 | 试探成功 | alloc | dealloc | lock_blocking | lock_waits | io_requests | io_ops |
| :-: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ⑰ | 3 000 | 2 | 3 000 | 3 000 | 0 | 0 | 0 | 0 | 0 | 0 |

覆盖度见证（判据 ⑰c 的 detail 行，逐字）：`装配差异=[]`、`武装>0 节点=1`、`延迟和=32`、
`窗口非零样本=88552`、`重新武装后差异=[]`、`延迟和=96`、`延迟线真的处理过=3001`、
`未武装节点=0`、`被钳帧数=0`。判据 ⑰ 的四元组读数逐字：
`四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0]`。

复现命令：

```bash
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features \
  --test rt_zero_alloc --test pdc_mix_path    # 判据汇总: 34 / 34 通过；pdc_mix_path 2 passed
shasum -a 256 crates/yeban-engine/src/rt.rs crates/yeban-engine/tests/rt_zero_alloc.rs \
  crates/yeban-engine/tests/pdc_mix_path.rs
```

### 17.3 本节新增/仍然有效的边界（必须和"全 0"一起读）

1. **跑的是回调体，不是 cpal 回调线程**：与 §11.2 同口径 —— 判据自己声明的主线程调
   `EngineRuntime::process_quantum`。
2. **引擎仍没有真实设备链延迟**：`DeviceDefinition::latency_samples` 是**上报值**；
   `pdc_mix_path` 的慢支路自身延迟按 `graph.rs` 既有判据
   `compensated_branches_line_up_sample_exactly` 的同样方式模拟。
3. **`MAX_PDC_DELAY_FRAMES = 8192` / `PDC_SLOTS = 16` 是实现边界，不是规范常数**：
   `ARCH-PDC-001` / `ARCH-PDC-002` 对池容量**没有上限**，而 `MUST-GATE-001` 禁止回调内分配
   ⇒ 预分配池必然有一条容量线。超出由 `EngineStats::pdc_unarmed_nodes` /
   `pdc_clamped_frames` 计数（**可观察，不静默**）。
4. **`D44②` 仍未回填**：限制器自身的 33 帧前瞻没有写进 `LatencyTable` ⇒ master 输出带
   `L_max + 33`。这是**故意不吞掉**的欠债，不是本切片修好的东西。
5. **`yeban-render` 侧那份重复 `pdc.rs` 的退役仍 pending**（ADR-0001 D19）。
6. **规范对四点沉默**：环容量上限、电平取样点的位置、全局 `L_max` 预滚是否应计入上报延迟、
   以及实时池装不下计划时的行为 —— 本切片对前两点做了**登记**（取样点选在电平之后、
   容量线可观察），没有发明规范答案。
