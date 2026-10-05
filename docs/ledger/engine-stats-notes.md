# 引擎健康读数进 `EngineStats` —— 字段表 / 判据 / 注入 / needs

- **台账类型**：接口读数表 / 判据映射 / **实测读数** / 注入记录 / 本机真跑与 CI 的严格区分 / needs（**不是规范**）
- **工作线**：`line/engine-stats`（worktree `yeban/.worktrees/engine-stats`，分支 `line/engine-stats`，基线 main `c53b7f3`）
- **所有者目录**：`crates/yeban-engine/**`（本文件是唯一新增的共享区文档）
- **上游接缝（上一批线交给集成者的 needs，本线落地其中两条）**：
  - `docs/ledger/gate-snapshot-churn-notes.md` 的 **N2**：`stash_events > 0` ⇒ 读者停止切换快照、
    控制侧"追上新 revision"**静默失败**、而控制面完全看不见（只在引擎内部计数）⇒ 要求"暴露进 `EngineStats` 并让控制面能降速"；
  - 同线的 **N5**：`RetireQueue::release_thread()` / `foreign_drains()` 只在测试探针里可见 ⇒ 要求进 `EngineStats`；
  - 同线的 **N6**：app 侧 60Hz 循环自己的"间隔 / 积压"读数没有与引擎侧判据对账 ⇒ 本线只写 **needs**（app 侧接线由 app 线做）。
- **规范来源**：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2（`ARCH-RT-002`：无锁退役队列 + 主线程 Drop）、
  `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.1 第 12 条（`MUST-GATE-012`）、
  `AGENTS.md` §2 红线 7（回调内零分配 / 零释放 / 零锁）与 §5（本机不编译重依赖）。

> 本文件回答六个问题：**新增了哪些读数（含义 / 单位 / 单调性 / 重置语义）**、
> **"追不上"怎么判定（实测数值）**、**释放线程归属与 `gate-snapshot-churn` 的读数怎么逐项对上**、
> **读这些读数是不是真的无锁零分配**、**判据怎么变红（3 组注入，含一条"无效注入"的实测）**、
> **app 侧应当怎么消费（needs）**。

---

## 1. 结论（一句话 + 交付清单）

> 把"控制面必须能看见的引擎健康读数"收进了 `EngineStats`：**`snapshot_stash_events`**（追不上）、
> 退役队列的 **`retire_pending` / `retire_drained` / `retire_drain_calls` / `retire_pruned`**、
> 以及 **`release_thread_is_main` / `foreign_drains`**（释放发生在哪个线程）。
> 判定不是"只有计数"：`is_snapshot_lagging()`（累计、粘滞）与
> `stash_events_since_last_read(&previous)`（采样窗口增量）+ `is_snapshot_lagging_since(&previous)`。
> `EngineStats` 保持**只读快照 / 无锁 / 零分配**：2000 次读取的窗口实测
> `alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0`。
> "追不上"用**既有 harness**（容量 1 的退役队列 + 控制面不排空）制造出来：
> `stash_events = 1`、`revision` 停在 3 而槽里已经是 6（**控制侧追不上的静默失败被读出来了**）。
> 与 `gate-snapshot-churn`（50 000 次交换压测）的读数**逐项对账**：`pending` / `drained` /
> `drain_calls` / `pruned` / `release_thread_is_main` / `foreign_drains` / `stash` 全部为**等号**。

| 文件 | 变更 | 规范 ID 与 needs | 说明 |
| :--- | :--- | :--- | :--- |
| `crates/yeban-engine/src/snapshot.rs` | 改（**+360 / −32**） | `MUST-GATE-012` `ARCH-RT-002` | 新增 `RetireAccounting`（跨线程原子记账）与 `RetireProducer`（薄包装，把记账自动带给渲染驱动）；`RetireQueue` 的计数改为读写该共享记账；新增 `SnapshotSlot::pruned()`；新增 3 条库单测（饱和 / 量规自愈） |
| `crates/yeban-engine/src/rt.rs` | 改（**+398 / −6**） | `MUST-GATE-012` `ARCH-RT-002` `ARCH-TOP-002` | `EngineStats` 新增 7 个字段 + 4 个判定方法；`stats()` 把它们一次交齐；`EngineRuntime::new` 的生产端类型换成 `RetireProducer`（调用点**一个字未改**）；新增 6 条库单测（判据 ① ② ③ ⑤ ⑥ + 反例） |
| `crates/yeban-engine/tests/rt_zero_alloc.rs` | 改（**+86 / −1**） | `MUST-GATE-001` | 新增判据 **⑫**（每量子读一次 `stats()` 的窗口四元组全 0）与 **⑫b**（注入口径实测） |
| `crates/yeban-engine/tests/snapshot_retire_churn.rs` | 改（**+91 / −1**） | `MUST-GATE-012` | 新增 `RetireMirror` 逐项对账：音频线程退出循环后读一次 `EngineStats`，与队列 / 槽的权威读数**逐项相等**（判据 ③ 的"逐项一致"） |
| `docs/ledger/engine-stats-notes.md` | **新建** | — | 本文件 |

**零新增依赖**：`crates/yeban-engine/Cargo.toml`、根 `Cargo.toml`、`Cargo.lock`、`deny.toml` **一个字未动**。
**没有**改 `crates/yeban-app/**`（app 侧只写 needs）、`schemas/**`、`.github/**`、`scripts/**`、
`docs/YEBAN_*.md`、`docs/adr/**`、`docs/DEVELOPMENT_LEDGER.md`、
`docs/ledger/{gate-status,phase-status,feature-alignment,human-decisions}.md`、法务文件。

### 1.1 设计决定：为什么生产端要换成 `RetireProducer`

退役队列的记账（`pending` / `drained` / `foreign_drains` / 释放线程归属）原先住在 `RetireQueue` 的
**私有字段**里，而 `RetireQueue` 由**控制线程**持有 —— 渲染驱动（`EngineRuntime`）根本读不到它。
要把它读进 `EngineStats`，只有三条路：

| 方案 | 代价 |
| :--- | :--- |
| 给 `EngineRuntime::new` 加一个 `Arc<RetireAccounting>` 参数 | **改签名 ⇒ 必须同时改 `crates/yeban-app`**（本线禁改） |
| 让控制面自己调 `runtime.attach_accounting(...)` | app 侧多一行接线 ⇒ 本次不接线时读数是**撒谎的 0** |
| **把 `rtrb::Producer` 包一层 `RetireProducer`（与队列共享同一个 `Arc`）** | 只改**引擎侧**；`retire_channel` 的每个调用点**源码级不变**（它只是把返回值继续传下去） |

选第三条：`EngineRuntime::new(&slot, retire, ...)` 的实参类型变了，但**每一个调用点都照旧编译**
（app / 示例 / 全部测试都是 `let (retire, queue) = retire_channel(..)` 再把 `retire` 传下去）。
于是 **app 不改一行**也能立刻读到真实读数，而不是"接上之前读 0"。

---

## 2. 新增字段表（`EngineStats`）

| 字段 | 类型 | 含义 | 单位 | 谁写（源） | 单调性 / 重置语义 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `snapshot_stash_events` | `u64` | 退役队列满 ⇒ 读者把旧快照寄存进 `stash` 并**停止切换快照**的累计次数 | 次 | 音频线程（`SnapshotReader::retire_or_stash`，`saturating_add`） | **单调不减（饱和）**；无重置 |
| `retire_pending` | `u64` | 退役队列**当前**待回收条数（跨线程镜像） | 条 | 音频线程 push 成功（`pushed` +1）；**由 `pushed.saturating_sub(drained)` 之差得到**（⚠ 不再覆写） | **量规**（可升可降）；这就是"明确的重置语义" |
| `retire_drained` | `u64` | 累计真正出队并 `Drop` 的旧快照条数（`= RetireQueue::dropped()`） | 条 | 控制线程 `drain`（饱和 CAS） | 单调不减（饱和）；无重置 |
| `retire_drain_calls` | `u64` | 累计**非空** `drain` 调用次数（空转不算 ⇒ 是"60Hz 循环真的在排空"的结构性判据） | 次 | 控制线程 `drain`（饱和 CAS） | 单调不减（饱和）；无重置 |
| `retire_pruned` | `u64` | 累计由写者侧 `SnapshotSlot::prune()` 释放的强引用条数（退役回收的**另一条**路径） | 条 | 控制线程 `prune`（饱和 CAS） | 单调不减（饱和）；无重置 |
| `release_thread_is_main` | `bool` | **首个**执行退役释放的线程是否就是**创建退役队列的那个线程**（= 控制线程 / 释放归属线程） | 布尔 | 控制线程首个非空 `drain` | 只可能 `true → false`；**尚无 drain 时为 `true`**（vacuously：没有观测到外来释放） |
| `foreign_drains` | `u64` | 在"首个释放线程"**之外**发生的非空 `drain` 次数 | 次 | 控制线程 `drain`（饱和 CAS） | 单调不减（饱和）；无重置 |

### 2.1 判定（不是只有计数）

| 判定 | 语义 | 何时用 | 控制面动作 |
| :--- | :--- | :--- | :--- |
| `is_snapshot_lagging()` | **累计 / 粘滞**：自进程开始至少发生过一次 stash（`snapshot_stash_events > 0`） | 起手、诊断 | 降速发布 + 立刻排空 |
| `stash_events_since_last_read(&previous)` | **增量**：自上次读取以来新增的 stash 次数（饱和减法，读到更旧基线给 `0`） | **60Hz 循环每帧** | `> 0` ⇒ 本帧就降速 |
| `is_snapshot_lagging_since(&previous)` | 上面 `> 0` 的布尔形式 | 同上 | 同上 |
| `has_retire_backlog()` | **量规**：队列仍有待回收（可自行回落，与粘滞的 lagging 不同） | 排空/对账 | 排空 |

**为什么必须"增量"而不只是累计**：累计量无法区分"十分钟前抖过一次"与"现在正在抖"。
控制面每帧读一次 `stats()`、把上一帧留着，就能得到"**这 16.7 ms 内**引擎有没有被退役积压逼停"。

**控制面应当据此降速**（写进本节，作为 app 线的消费契约）：

1. `stash_events_since_last_read(&prev) > 0` ⇒ **本帧降低 `SnapshotSlot::publish` 频率**（或暂停 `reload`）；
2. 同一帧立刻 `RetireQueue::drain(全部)` + `SnapshotSlot::prune()`（排空是唯一能让读者恢复切换的动作）；
3. 若连续 N 帧仍 `> 0` ⇒ UI 提示"引擎拓扑更新被推迟"（`is_snapshot_lagging()` 会一直是 `true`，所以提示要基于**增量**，否则会永远挂着）；
4. 注意：`retire_pending` 是**量规**，它回落**不代表**曾经没有 lagging（累计量才是那个事实）。

### 2.2 饱和与量规的边界（判据 ⑥ 的对象）

| 量 | 大数行为 | 为什么这样选 |
| :--- | :--- | :--- |
| `snapshot_stash_events` / `retire_drained` / `retire_drain_calls` / `retire_pruned` / `foreign_drains` | **饱和**（`u64::MAX` 停住，不回绕） | 回绕会把"释放过多少"变成**谎报 0**；饱和是可判定的边界 |
| `retire_pending` | `push` 成功用一次 `fetch_add`（占用以队列容量为界 ⇒ 不可能回绕）；**由 `pushed.saturating_sub(drained)` 之差得到**（不再覆写）| 渲染路径上只花一条原子指令；⚠ **原「覆写 ⇒ 镜像自愈漂移」的说法已被证伪**：覆写窗口（`note_drain` 的 `remaining` 求值 → `pending.store`）之间落进的 `+1` 会被**永久盖掉**（见账本第 82 轮 `MUST-GATE-012` 的根治）|
| 既有 `quanta` / `events_applied` / `meter_frames` … | `wrapping_add`（u64 全宽） | 它们是"有没有在跑"的结构性计数；`DEFAULT_BLOCK_FRAMES = 128` ⇒ 48 kHz 下**每秒恰好 375 个量子**，回绕需要 **> 10 亿年**（**编译期断言** `rt.rs` 的 `const _`，实测通过） |

---

## 3. "追不上"的实测读数（判据 ②，本机真跑）

夹具：**既有** `rig_with_retire_capacity(1)`（退役队列容量 1，其余与既有 `rig()` 同）+ 控制面**不排空**。
命令（本工作树的脚本；`cargo-local.sh` 会打印它实际用的工作区）：

```bash
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --lib
```

| 阶段 | 实测读数 | 解释 |
| :--- | :--- | :--- |
| 正常（`stash == 0`） | `snapshot_stash_events=0`、`is_snapshot_lagging()=false`、`stash_events_since_last_read=0` | 判据 ① 的形态 |
| 发布 2..=6 且不排空（容量 1） | `snapshot_stash_events=1`、`is_snapshot_lagging()=true`、`stash_events_since_last_read(&healthy)=1`、`is_snapshot_lagging_since(&healthy)=true`、`retire_pending=1`、`snapshot_switches=2`、`revision()==Some(3)`（**槽里已经是 6**） | 队列满 ⇒ 读者寄存并**停止切换**；控制侧"追上最新 revision"静默失败，`stash_events` 是**唯一**的可见信号 |
| 控制面降速：`drain(1)` 后再跑 1 个量子 | `revision()==Some(6)`（追上了）、`snapshot_stash_events=2`、`retire_drained=1` | **容量 1 时"追上"本身还要再寄存一次**（读者必须交回滞留的那一份）⇒ 降速 = 排空 **+** 延后发布，不是"排一次就够" |
| 读到更旧基线 | `stash_events_since_last_read` 给 `0`（不是 `u64` 回绕） | 饱和减法语义 |

> 这组读数就是 `gate-snapshot-churn` 注入 I2（关掉 60Hz 排空）暴露的那个行为：
> 那次注入让墙钟从 6 s 拉到 228 s，因为控制侧每轮有 8 次"5 秒握手超时"。
> 现在控制面**不必**靠超时发现这件事 —— 读 `stash_events_since_last_read` 即可。

---

## 4. 释放线程归属：与 `gate-snapshot-churn` 的读数**逐项对照**

### 4.1 50 000 次交换压测里的对账（`tests/snapshot_retire_churn.rs`）

音频线程**退出渲染循环之后**读一次 `EngineStats`（此刻主线程正阻塞在 `join` 上 ⇒ 两边处于**同一静止时刻**），
主线程随后把镜像与队列 / 槽的权威读数**逐项**比等号。本机真跑（`bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test snapshot_retire_churn`）：

| 读数项 | `EngineStats` 镜像（音频线程侧读） | 队列 / 槽的权威读数（控制线程侧读） | 结果 |
| :--- | :--- | :--- | :--- |
| 队列占用 | `stats_mirror(pending=513)` | `queue.pending()=513`（join 之后立刻读） | **相等** |
| 累计回收条数 | `stats_mirror(drained=9999)` | `queue.dropped()=9999` | **相等** |
| 非空 drain 次数 | `stats_mirror(drain_calls=4002)` | `queue.drain_calls()=4002` | **相等** |
| 写者侧回收条数 | `stats_mirror(pruned=9999)` | `slot.pruned()=9999` | **相等** |
| 释放线程归属 | `stats_mirror(release_thread_is_main=true)` | `queue.release_thread() == Some(main_thread)` ⇒ `true` | **相等** |
| 外线程 drain | `stats_mirror(foreign_drains=0)` | `queue.foreign_drains()=0` | **相等** |
| 寄存次数 | `stats_mirror(stash=0)` | `EngineRuntime::snapshot_stash_events()=0` | **相等** |

五轮的镜像读数（同一轮内每一项**逐字相等**，此处摘一轮的打印行）：

```text
轮「高频交换」: swaps=10000 publishes=10512 quanta=21029 ... switches=10512 stash=0
  ... created=10513 released=10512 live=1 queue_pending=0 slot_pending=0
  release_thread_is_main=true foreign_drains=0
  stats_mirror(pending=513 drained=9999 drain_calls=4002 pruned=9999 release_thread_is_main=true foreign_drains=0 stash=0)
```

⚠ **打印行的口径说明（防止误读）**：同一行里的 `queue_pending=0` / `drain_calls=4xxx` 是**轮末**
（归属窗口 drain/prune 之后）的读数，而 `stats_mirror(...)` 是**音频线程退出时**的读数 ——
两者本来就应该不同（中间还发生了两次归属窗口的 drain）。等号比较发生在"join 之后立刻"那一刻，
这是判据里写死的事实，不是"看起来差不多"。整条压测本机 **6.0 s**（`wall_ms=6010`，
50 000 次交换 + 2 560 次尾段积压 + 105 000+ 量子），音频线程 `alloc=0 dealloc=0 快照析构=0`，
对账等式 `创建 52565 == 释放 52560 + 存活 5` 成立。

### 4.2 反例（也是注入 E2 的靶子）

正常路径上 `release_thread_is_main` 恒为 `true` ⇒ 单靠压测**抓不住**"把这个布尔量写死"。
因此本线加了一条**单线程可构造的反例**（`stats_report_a_foreign_release_thread_while_foreign_drains_stays_zero`）：

| 步骤 | 实测读数 |
| :--- | :--- |
| 队列在主线程建；首个非空 `drain` 交给**另一个线程** | `queue.release_thread() != Some(main)`、`release_thread_is_main=false`、**`foreign_drains == 0`** |
| 之后主线程再 `drain` 一次 | `foreign_drains == 1`（相对"首个释放线程"这一次是外来）、`release_thread_is_main` 仍为 `false`（不自愈） |

> 这条反例顺带证明 `release_thread_is_main` **不是** `foreign_drains == 0` 的别名：
> 首个 `drain` 就跑在外线程时，`foreign_drains` 仍然是 `0`，只有归属布尔量抓得住它。

---

## 5. 零分配 / 无锁的实测（判据 ④）

新增的读数全是**原子量**（`AtomicU64` / `AtomicBool` 的 `load`/`store`/CAS 与一个不可变的 `ThreadId`），
所以"读 `EngineStats`"不该引入锁、分配或 I/O。沿用既有零分配探针窗口
（`tests/rt_zero_alloc.rs` 的**按线程**武装计数分配器 + `rt_probe` 的见证型锁探针与 I/O 边界），
新增判据 **⑫**：窗口里 **2000 个量子，每量子读一次 `stats()`**。本机真跑读数：

```text
判据 ⑫ PASS 控制面读取 EngineStats：每量子读一次，四元组仍全 0（无锁 / 零分配 / 零 I-O）
  ⑫统计面读取；四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0]；
  子窗口=1 探针经过=2000（期望 2000）试探成功=2000 试探失败=0；
  窗口内读了 2000 次 stats()（含退役队列原子量）；最后 quanta=2002 retire_pending=0
  release_thread_is_main=true foreign_drains=0

判据 ⑫b PASS 注入口径：`Vec::new()` **不分配**（无效注入），`Vec::with_capacity(1)` 分配 1 次（有效注入）
  `Vec::new()` 窗口[alloc=0 dealloc=0 ...]；`Vec::with_capacity(1)` 窗口[alloc=1 dealloc=1 ...]
```

四元组全 0 **且**探针位置见证成立（`探针经过=2000` = 本窗口量子数、`试探成功=2000`）⇒
这 2000 次读取发生在**确证被执行到**的路径上，`alloc=0` 是**测出来的 0**，不是"这段代码没跑"。

**渲染路径上的新增成本**（如实登记）：`SnapshotReader::retire_or_stash` 在 push 成功时多做一次
`AtomicU64::fetch_add`（一条 RMW，无锁无分配），且只在**快照切换**时发生（不是每个量子）。
判据 ②（`tests/rt_zero_alloc.rs` 的快照交换场景）与 `[MUST-GATE-012]` 的音频线程窗口都仍然是 `alloc=0 dealloc=0`。

---

## 6. 判据清单（8 条，含本机实测）

| # | 判据 | 落在哪 | 本机实测 |
| :-: | :--- | :--- | :--- |
| ① | 正常播放 ⇒ `stash == 0` 且新读数自洽（两条回收路径都在动） | `rt.rs::stats_report_a_healthy_retire_pipeline_during_normal_playback` | `switches=4 drained=4 pruned=4 pending=0 stash=0 lagging=false` |
| ② | **制造追不上**（读者慢于写者）⇒ `stash > 0` 且判定为 lagging | `rt.rs::stats_flag_snapshot_lagging_when_the_reader_cannot_keep_up` + `rt.rs::stash_delta_readings_never_underflow_on_a_stale_baseline` | §3 的表；`stash=1`、`revision` 停在 3（槽里 6） |
| ③ | `release_thread_is_main == true` / `foreign_drains == 0` 在正常路径成立，且与 `gate-snapshot-churn` 逐项一致 | `rt.rs::stats_retire_readings_match_the_queue_item_by_item` + `rt.rs::stats_report_a_foreign_release_thread_while_foreign_drains_stays_zero` + `tests/snapshot_retire_churn.rs` 的 `RetireMirror` 对账 | §4：50 000 次交换下 7 项**逐项相等**；反例给出 `false / 0` 的组合 |
| ④ | 读数**无锁零分配**（沿用既有零分配探针窗口） | `tests/rt_zero_alloc.rs` 判据 **⑫** + **⑫b** | §5：2000 次读取四元组全 0 |
| ⑤ | `stats()` 是**快照**：引擎继续跑 ⇒ 第二次读数不回退（除明确的重置语义） | `rt.rs::stats_are_snapshots_and_never_go_backwards_between_reads` | 同一瞬间两次读**完全相等**；累计量单调不减；`retire_pending` 量规明确回落 3 → 0 |
| ⑥ | 计数**不饱和/不溢出**（大数行为写明并断言） | `snapshot.rs::retire_counters_saturate_instead_of_wrapping`、`snapshot.rs::slot_pruned_counter_saturates_instead_of_wrapping`、`snapshot.rs::retire_pending_mirror_is_overwritten_by_drain_and_self_heals`、`rt.rs` 的编译期 `const _` | `u64::MAX - 1` 预置后 drain ⇒ 停在 `u64::MAX`（不是回绕成 1）；`quanta` 回绕需 > 10 亿年（编译期断言） |
| ⑦ | 既有判据全绿（尤其 `snapshot_retire_churn`、`transport_rt_zero_alloc`） | 整个 crate 的门禁 | 144 条库单测 + 5 个 `harness = false` 运行期目标 + 全部集成目标；`snapshot_retire_churn` 6.0 s 全绿 |
| ⑧ | 门禁：`run-gates.sh light` + `run-gates.sh crate yeban-engine`（本机 `--no-default-features` 真跑） | 脚本 | 两条都 `EXIT=0`（见 §8） |

新增单测共 **9 条**（`rt.rs` 6 条 + `snapshot.rs` 3 条），另有 2 个既有 `harness = false` 目标各加 1 条判据（⑫、以及压测里的对账块）。

---

## 7. 注入记录（3 组：E1 / E2 / E3，其中 E3 含一条"无效注入"的实测）

> 命名刻意用 **E1/E2/E3**（engine-stats），避免与 `gate-snapshot-churn` 的 I1~I4、
> `rt_zero_alloc` 自己的 I1~I4 混淆。
> 每组都：改一处 → 跑同一条命令 → 记录**原始红行** → 还原 → 用 `sha256sum -c` 确认**逐字节相同** → 复跑确认绿。
> 还原之后 `grep -rn "注入 I1\|注入 I2\|注入 I3" crates/` 只剩下**文档里**提到注入的行（本线的三处标记已全部消失）。

### E1 —— 把"追不上"判定写死为 `false`（`rt.rs::EngineStats::is_snapshot_lagging`）

**实测红（2 条判据）**：

```text
test rt::tests::stats_flag_snapshot_lagging_when_the_reader_cannot_keep_up ... FAILED
thread 'rt::tests::stats_flag_snapshot_lagging_when_the_reader_cannot_keep_up' panicked at crates/yeban-engine/src/rt.rs:1313:9:
累计判定必须为真
test rt::tests::stash_delta_readings_never_underflow_on_a_stale_baseline ... FAILED
thread 'rt::tests::stash_delta_readings_never_underflow_on_a_stale_baseline' panicked at crates/yeban-engine/src/rt.rs:1355:9:
assertion failed: lagging.is_snapshot_lagging()
test result: FAILED. 142 passed; 2 failed; 0 ignored
```

### E2 —— 把 `release_thread_is_main` 写死为 `true`（`snapshot.rs::RetireAccounting::release_thread_is_owner`）

**实测红（1 条：反例判据）**：

```text
test rt::tests::stats_report_a_foreign_release_thread_while_foreign_drains_stays_zero ... FAILED
thread '...' panicked at crates/yeban-engine/src/rt.rs:1445:9:
归属不成立
test result: FAILED. 0 passed; 1 failed; 0 ignored
```

⚠ **这条注入教了一件事（如实记下）**：同一时刻

```text
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --lib stats_retire_readings_match_the_queue_item_by_item
  ⇒ ok（正常路径判据**跟着一起撒谎**：镜像与队列两侧都被写死成 true，等号仍然成立）

bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test snapshot_retire_churn
  ⇒ [MUST-GATE-012] ok（50 000 次交换压测**照样全绿**，`release_thread_is_main=true foreign_drains=0`）
```

也就是说：**"正常路径恒为 true"的读数，单靠正常路径判据无法证伪** ——
必须有一条**反例**判据（本线的 `stats_report_a_foreign_release_thread_...`）才有判别力。
这与 `gate-snapshot-churn` 的教训同族（"注入要打在被保护的路径上"）。

### E3 —— 在**读取路径**里加一次分配（`rt.rs::EngineRuntime::stats`）

**E3a（题干给的写法）`Vec::<u8>::new()`：实测 ⑫ 仍然 PASS —— 这是**无效注入**。**

```text
判据 ⑫ PASS 控制面读取 EngineStats：每量子读一次，四元组仍全 0（无锁 / 零分配 / 零 I-O）
  ⑫统计面读取；四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0]；...
判据汇总: 24 / 24 通过
```

原因不是判据坏了，而是 **`Vec::new()` 根本不分配**（容量 0、悬垂指针，不碰分配器）——
本线为此在 `rt_zero_alloc.rs` 里加了 **⑫b** 把这条实测**钉在判据里**
（`Vec::new()` 窗口 `alloc=0`；`Vec::with_capacity(1)` 窗口 `alloc=1 dealloc=1`），
免得后人照着"加一次 `Vec::new()`"做无效注入还以为判据没牙。

**E3b（真正有效的写法）`Vec::<u8>::with_capacity(1)`：实测 ⑫ 变红。**

```text
判据 ⑫ FAIL 控制面读取 EngineStats：每量子读一次，四元组仍全 0（无锁 / 零分配 / 零 I-O）
  ⑫统计面读取；四元组[alloc=2000 dealloc=2000 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0]；...
判据汇总: 23 / 24 通过
EXIT=1
```

`alloc=2000` = 每量子 1 次（窗口 2000 个量子）⇒ 读取路径的分配分量**真的有判别力**，
且变红时其余分量仍为 0（归因干净：不是锁、不是 I/O）。

---

## 8. 本机真跑 vs CI 的**严格区分**

| 项目 | 本机（M2，`aarch64-apple-darwin`） | CI（x86_64 Linux） |
| :--- | :--- | :--- |
| `bash scripts/gates/run-gates.sh light` | ✅ **真跑** `EXIT=0` | ✅（`checks` job） |
| `bash scripts/gates/run-gates.sh crate yeban-engine` | ✅ **真跑** `EXIT=0`（脚本打印 `NOTE … --no-default-features 变体零重依赖, 本机用该变体真跑 (D19)`） | ✅（**默认 feature**，含 cpal） |
| `cargo-local test -p yeban-engine --no-default-features` | ✅ **真跑**：144 条库单测 + 5 个 `harness = false` 目标 + 全部集成目标 | ✅（默认 feature，`--workspace --all-targets`） |
| `cargo-local clippy -p yeban-engine --all-targets --no-default-features -- -D warnings` | ✅ **真跑**：0 警告 | ✅（默认 feature） |
| `-p yeban-engine` 的**默认 feature**（cpal）构建 | ⛔ **未跑**（AGENTS.md §5 明文禁止） | ✅ |
| 跨架构 / L2 对账 | ⛔ 不在本线范围 | 由 workspace 全量测试覆盖 |

**CI 判决**：见 §10 的 needs（提交后由 `bash scripts/dev/ci-verdict.sh line/engine-stats` 读回；
**读回之前本行不得写成"通过"**）。

---

## 9. 仍然**做不到**的部分（本线没有证明的）

1. **没有 cpal 真实回调线程上的同族判据**：与前序各线相同的边界（本机不编译 cpal）。
   `EngineStats` 的读数本身与线程身份无关，但"cpal 回调线程上读它也无锁零分配"没有被测。
2. **没有在 `yeban-app` 的真实 60Hz 循环里驱动**：`needs` N6 仍在 app 线（本线只写 needs，禁改 app）。
   本线证明的是**读数与判定**存在且可信，不是"app 已经在降速"。
3. **`RetireAccounting` 的内存序是"诊断级"的**：`pending` 用 `Release`/`Acquire` 是为了
   让读者看到计数不至于离谱，但**没有**用 loom/Miri 机械化验证（新增依赖需裁决）。
   它的用途是**健康读数**，不是同步原语 —— 控制面不应据它做安全性决策。
4. **`release_thread_is_main` 的 `true` 是 vacuous 真**：尚无任何非空 `drain` 时它也是 `true`。
   也就是"还没释放过"与"释放都发生在归属线程"读数相同 —— 需要区分时请同时看 `retire_drain_calls`。
5. **压测里的镜像对账只在"音频线程退出、主线程阻塞在 join"那一刻做**：
   并发运行中的等号对账不可能（两个读数天然在不同时刻）；这是刻意选的静止时刻。

---

## 10. needs（交给集成者 / 后续工作线）

- **N-A1（app 线，必须）**：`yeban-app::engine_host` 的 60Hz 循环应当**每帧**读一次
  `runtime.stats()`，保留上一帧，用 `stash_events_since_last_read(&prev)` 判定"这一帧引擎被退役积压逼停过"；
  命中时**降速**：降低 `SnapshotSlot::publish` / `reload` 频率、立刻 `retire.drain(RETIRE_CAPACITY)` +
  `slot.prune()`，并把"引擎拓扑更新被推迟"上报 UI（不要用累计的 `is_snapshot_lagging()` 做提示，它会一直挂着）。
- **N-A2（app 线）**：读数可以在 60Hz 循环里**每帧读**（判据 ④ 实测无锁零分配），
  不要为它加锁、也不要把它拷进需要分配的容器里（那会把控制面的成本转成实时路径的风险）。
- **N-A3（app 线，健康监控）**：`release_thread_is_main == false` 或 `foreign_drains > 0` ⇒
  退役释放**没有集中在控制线程**上。app 侧应把 `drain`/`prune` 固定在同一个 60Hz 循环线程里
  （当前 `EngineHost::tick` 就是这样；本线只登记读数，未改 app）。
- **N-A4（app 线，对应上游 N6）**：app 侧 60Hz 循环应同时上报**自己的**"本帧间隔（量子数 / 墙钟）"与
  "排空前的 `retire_pending`"，与引擎侧的 `retire_drain_calls` / `snapshot_stash_events` 做端到端对账 ——
  只有这样才能证明"控制面 60Hz 真的跟得上"，而不只是"引擎侧判据说它跟得上"。
- **N-INT1（集成者）**：`docs/ledger/gate-status.md` 的 `MUST-GATE-012` 行可以补一句"观测面 N2/N5 已落地（`EngineStats`）"，
  并把本线的 CI 判决 run id 写进去（读回后才写"通过"）。
- **N-INT2（集成者）**：建议在 `docs/DEVELOPMENT_LEDGER.md` 记一条教训：
  **"`Vec::new()` 不是分配 —— 用它做零分配注入是无效注入"**（本线 E3a 实测 24/24 仍然全绿），
  与既有的"注入要打在被保护的路径上"是同族。

---

## 11. 复跑命令（本机真跑）

```bash
# 判据 ① ② ③ ⑤ ⑥（库单测；含追不上、逐项对账、饱和）
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --lib

# 判据 ④（零分配/无锁；harness = false，以退出码判定）
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test rt_zero_alloc

# 判据 ③（与 gate-snapshot-churn 逐项对账；~6 s，50 000 次交换）
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test snapshot_retire_churn

# 本 crate 全量（轻量变体，不编译 cpal）
bash scripts/gates/run-gates.sh crate yeban-engine

# 轻量门禁（零编译）
bash scripts/gates/run-gates.sh light
```

> **集成者更正（第 82 轮）**：本文件原写的「`drain` 后覆写为真实剩余 ⇒ 覆写让镜像自愈漂移」是**错的** —— 覆写窗口会永久盖掉窗口内落进的 `+1`，这正是 `MUST-GATE-012` 两次 CI 红（`镜像=512 权威=513`）的根因。现实现改为 `pushed`（`fetch_add`）+ `pending() = pushed.saturating_sub(drained)`，**按构造无丢 `+1` 的窗口**。本文件已按新实现更正两处字段描述；app 侧**不要**再依赖「下一次 `drain` 会自愈 `retire_pending`」（N3）。
