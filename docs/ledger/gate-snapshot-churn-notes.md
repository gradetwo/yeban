# `[MUST-GATE-012]` 闭环台账 —— 高频交换压测 / 音频线程零释放 / 主线程 60Hz 释放 / 零泄漏对账

- **台账类型**：判据映射 / 实测读数 / 注入记录 / 本机真跑与 CI 的严格区分 / **仍未证明的部分** / needs（**不是规范**）
- **工作线**：`line/gate-snapshot-churn`（worktree `yeban/.worktrees/gate-snapshot-churn`，基线 main `f12f226`）
- **所有者目录**：`crates/yeban-engine/**`（本文件是唯一新增的共享区文档；`docs/ledger/{gate-status,phase-status,feature-alignment,human-decisions}.md` 与 `docs/DEVELOPMENT_LEDGER.md` 由集成者改）
- **规范来源**：
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.1 第 12 条（`MUST-GATE-012`）
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2（`ARCH-RT-002`：无锁退役队列 + 主线程 Drop）
  - `AGENTS.md` §2 红线 7（实时回调内零分配 / 零释放 / 零锁）+ §5（本机不编译重依赖）
- **上游接缝**：`docs/ledger/engine-rt-notes.md`（退役队列的原始设计与 needs）、
  `docs/ledger/transport-engine-notes.md`（走带 harness）、`crates/yeban-engine/tests/rt_zero_alloc.rs`（`MUST-GATE-001` 的 63 次交换版本）

> 本文件回答六个问题：**规范那三句话各自变成了哪条判据**、**实测读数是什么**、
> **判据怎么变红（5 组注入）**、**"释放只在主线程"是结构证明还是调度证明**、
> **本机真跑与 CI 判决的严格区分**、**还剩什么没做到**。

---

## 1. 结论（一句话 + 交付清单）

> 规范的三句话现在各自都有**运行期判据**：**50 000 次高频快照交换**（5 轮 × 10 000，
> 另有 2 560 次尾段积压，共 52 560 次发布、105 141 个渲染量子 = **280.4 秒**等价音频时长）下，
> 音频线程窗口内**堆释放 0 次、堆分配 0 次、快照析构 0 次**；
> 全部旧快照在**主线程**的 60Hz 节拍循环里释放（纯 drain 窗口实测
> **出队 == 全局释放 == 主线程释放**，每轮 512 ~ 514 条）；
> **零泄漏对账等式成立**：`创建 52 565 == 释放 52 560 + 存活 5`。
> 本机（M2）**整条判据实测 6.0 ~ 6.2 秒**。

| 文件 | 规范 ID | 说明 |
| :--- | :--- | :--- |
| `crates/yeban-engine/tests/snapshot_retire_churn.rs`（**新建**，933 行） | `MUST-GATE-012` `ARCH-RT-002` `ARCH-RT-001` | 高频交换压测主判据（`harness = false`）：真音频线程 + 按线程武装的计数型分配器 + 逐线程快照析构探针 + 零泄漏对账 |
| `crates/yeban-engine/src/snapshot.rs`（+252 行） | `MUST-GATE-012` `ARCH-RT-002` | ① `pub mod release_probe`（线程局部快照析构计数 + 进程总数）；② `impl Drop for EngineSnapshot` 的唯一记账点；③ `RetireQueue::release_thread()` / `foreign_drains()`（释放线程可观测） |
| `crates/yeban-engine/src/rt.rs`（+12 行） | `MUST-GATE-012` `ARCH-RT-002` | `EngineRuntime::snapshot_stash_events()`：把"退役队列满 ⇒ 读者停止切换"变成可读读数（判据要求压测期间恒为 0） |
| `crates/yeban-engine/Cargo.toml`（+10 行） | `MUST-GATE-001` | 新增 `[[test]] name = "snapshot_retire_churn" harness = false`（**零新增依赖**：`Cargo.lock`、根 `Cargo.toml`、`deny.toml` 一个字未动） |

---

## 2. 规范三句话 → 判据映射（逐句可测）

| # | 规范里的词 | 判据（`tests/snapshot_retire_churn.rs`） | 仪器 | 本机实测 |
| :-: | :--- | :--- | :--- | :--- |
| ① | 音频线程**无任何堆释放** | 音频线程窗口内 `deallocations == 0` | 计数型全局分配器（**线程局部**武装，`thread_local!` + `try_with`） | 每轮 `0`，5 轮合计 `0` |
| ② | （更强的对照）无任何堆分配 | 同窗口 `allocations == 0`（**单独报告**；若 > 0 必须点名来源） | 同一个分配器 | 每轮 `0` |
| ③ | 零泄漏 | 等式 `创建数 == 释放数 + 存活数`，且 `queue.pending() == 0 && slot.pending_len() == 0` | `release_probe::total()` 差值 + `Arc::strong_count` | `52565 == 52560 + 5` ✅ |
| ④ | 旧快照**在主线程**释放 | (a) 音频线程上快照析构数 `== 0`（**整个线程生命周期**，含读者退场）；(b) 纯 drain 窗口 `出队数 == 全局释放数 == 主线程释放数`；(c) `RetireQueue::release_thread() == 主线程` 且 `foreign_drains == 0` | 逐线程释放探针 + 分配器 + 队列记账 | `drain(drained=512 released=512 watched=512)`、`release_thread_is_main=true`、`foreign_drains=0` |
| ⑤ | 主线程 **60Hz 循环**排空 | 节拍 = 每 5 个量子（13.3 ms，标称 75 Hz）排空一次；实测**平均**间隔 ≤ 6 个量子 = 16.0 ms ≤ 16.67 ms；轮末 `pending == 0` | 音频时钟 `AUDIO_QUANTA` + `tick_triggers` | `tick_triggers≈4000/轮`，均值 `5.26` 个量子 = **14.0 ms** |
| ⑥ | 压测时长与交换次数**实测打印** | 每轮打印 `swaps/publishes/quanta/audio_s/wall_ms`，并断言总量 ≥ 50 000 | `println!` + 断言 | 见 §3 |
| ⑦ | 与**走带**交互（播放中交换） | 第 5 轮在 `TransportCommand::Play` + 周期 `SeekTicks` 下做同样的 10 000 次交换，仍断言音频线程 0 释放 / 0 分配 | 借 `transport_rt_zero_alloc` 的既有 harness（`event_channel` + 量子驱动） | `transport_commands=21`、`transport_quanta=21038`、0 释放 |
| ⑧ | 门禁 | `run-gates.sh light` + `-p yeban-engine`（clippy `-D warnings` + test，`--no-default-features` 轻量变体） | 脚本 | 见 §8 |

另外两条"防假绿"判据（不在规范原文里，但没有它们前 7 条都可能是空转）：

- **覆盖率自检**：音频线程的 `switches` 必须 ≥ 请求的交换次数（实测 `switches == publishes`，
  即**每一次发布都被音频线程观察到并真的换了一次快照**）；`drained_during_churn > 0`；
  `prune/drain` 归属窗口必须真的清掉了 ≥ 尾段积压条数。
- **仪器自检**（"一个永远不会红的测量工具比没有测量更糟"）：先证明分配器能看见
  `alloc=1 dealloc=1`、释放探针能看见"本线程释放 1 个"；再由 5 组注入证明判据真的会红（§4）。

---

## 3. 实测读数（本机 M2，`main = f12f226` 基线 + 本线改动）

命令（**本机真跑**，工作树内脚本；L30：`cargo-local.sh` 会打印它实际用的工作区）：

```bash
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test snapshot_retire_churn
```

完整日志（**定义性的一次，提交前在最终代码上重跑，未注入**）：

```text
[MUST-GATE-012] 规模: 5 轮 × 10000 次交换 + 每轮 512 次尾段积压 = 52560 次发布（目标 ≥ 50 000）
[MUST-GATE-012] 仪器自检: 分配器 alloc=1 dealloc=1
[MUST-GATE-012] 仪器自检: 释放探针 本线程=1 全局=1
[MUST-GATE-012] 轮「高频交换」: swaps=10000 publishes=10512 quanta=21027 audio_s=56.1 wall_ms=1203 audio_ms=1199 \
  audio_alloc=0 audio_dealloc=0 audio_releases=0 switches=10512 stash=0 \
  drain_calls=4001 drained(churn)=9999 pruned(churn)=9999 max_pending_before_drain=3 \
  max_quanta_between_drains=6 tick_triggers=4000 backlog=513 \
  prune(pruned=513 released=0 watched=0 alloc=0) drain(drained=513 released=513 watched=513 alloc=0) \
  created=10513 released=10512 live=1 queue_pending=0 slot_pending=0 release_thread_is_main=true foreign_drains=0
（其余 4 轮读数同族：quanta 21027~21030、wall_ms 1182~1212、drain_calls 4001~4002、
 归属窗口 512~514 条、max_pending_before_drain=3、音频线程全部 0/0/0）
[MUST-GATE-012] 汇总: rounds=5 swaps=50000 (+2560 尾段积压) wall_ms=6008 audio_time_s=280.4 \
  音频线程 alloc=0 dealloc=0 快照析构=0；创建=52565 释放=52560 存活=5；主线程归属窗口释放=2566
[MUST-GATE-012] 对账等式: 创建 52565 == 释放 52560 + 存活 5 ⇒ 成立（零泄漏）
[MUST-GATE-012] ok: 50000 次高频交换 / 280.4 秒等价音频时长，音频线程堆释放 0 次、快照析构 0 次；\
  全部旧快照在主线程 60Hz 循环中释放；零泄漏对账成立
```

| 读数 | 值 | 怎么来的 |
| :--- | :--- | :--- |
| 快照交换次数 | **50 000**（5 轮 × 10 000）+ 2 560 次尾段积压 = **52 560 次发布** | `SnapshotSlot::publish` 计数 = `1 + scheduled` |
| 真实切换次数 | **52 560**（= 发布数，`switches == publishes`） | 逐次"发布→等音频线程观察到该 revision"握手（`LAST_REVISION`） |
| 渲染量子数 | **105 141** | `AUDIO_QUANTA`（音频线程逐量子自增） |
| 等价音频时长 | **280.4 秒**（105 141 × 128 帧 ÷ 48 kHz） | 音频时钟换算 |
| 墙钟耗时 | **6 008 ms**（整条判据；含 5 次线程创建与 5 轮对账） | 每轮 `wall_ms` 相加；整进程 `time` 实测 **6.2 s** |
| 音频线程堆分配 | **0**（每轮 0） | 计数型全局分配器，窗口内线程局部武装 |
| 音频线程堆释放 | **0**（每轮 0） | 同一分配器 `deallocations` |
| 音频线程快照析构 | **0**（整线程生命周期，含读者退场） | `release_probe::released_by_current_thread()` |
| 主线程纯 drain 窗口 | `出队 == 全局释放 == 主线程释放`（每轮 512 ~ 514 条） | 拆成 prune 窗口 + drain 窗口，见 §5 |
| 60Hz 节拍 | `tick_triggers ≈ 4 000/轮`，均值间隔 **≈ 5.26 个量子 = 14.0 ms** | `AUDIO_QUANTA` 音频时钟；上限断言 6 个量子 = 16.0 ms ≤ 16.67 ms |
| 排空前最大积压 | **3 条**（`queue.pending()`） | 每拍实测 |
| 退役队列满寄存 | **0** | `EngineRuntime::snapshot_stash_events()` |
| 零泄漏对账 | `创建 52 565 == 释放 52 560 + 存活 5` ✅ | `release_probe::total()` 差值 + `Arc::strong_count(&slot.current()) - 1` |

**CI 时长考量**：本机 ~6 s；即使 CI runner 慢 5 倍也只有 ~30 s，且只在 `yeban-engine` 这一个
测试目标里跑（`harness = false`，不占 libtest 汇总）。因此**没有**为了省时间把 50 000 次调小
（判据里有 `total_swaps >= 50_000` 的硬断言，任何"调小"都会同时把判据削弱 ⇒ 不许出现）。

---

## 4. 注入记录（5 组，全部"注入 → 变红 → 还原 → 复绿"）

每组都：改一处 → 跑同一条命令 → 记录**原始红行** → `cp` 还原 → 复跑确认 `EXIT=0`。
下面 5 组读数全部来自**冻结代码**（`cargo fmt` + clippy `-D warnings` 之后）的一次完整重跑，
还原后 `sha256sum` 与注入前**逐字节相同**（`tests/snapshot_retire_churn.rs` =
`8dcadf16…6f19`，`src/snapshot.rs` = `a2987e58…375d`）。注入只落在**本线拥有**的文件
（`src/snapshot.rs`、`tests/snapshot_retire_churn.rs`），**没有**动 `crates/**` 里的别的 crate、
`scripts/**`、`.github/**`。提交前用 `grep -n "注入"` 复核过：只剩下**文档里**提到注入的行，
**没有**任何注入代码残留。

### I1a —— 音频线程就地 `drop` 旧快照（不推进退役队列）

改 `SnapshotReader::retire_or_stash` 为 `drop(old)`。**实测红（3 条同族）**：

```text
轮「高频交换」: ... audio_dealloc=0 audio_releases=0 switches=10512 stash=0 drain_calls=0 \
  drained(churn)=0 pruned(churn)=9999 ... prune(pruned=513 released=513 watched=513 alloc=0) \
  drain(drained=0 released=0 watched=0 alloc=0) release_thread_is_main=false foreign_drains=0
FAIL: 退役队列的释放线程不是主线程：release_thread=None（主线程=ThreadId(1)）
FAIL: drain 窗口一次释放都没发生 —— 这条判据是空转（假绿）
FAIL: 归属窗口清掉的不够多：prune=513 drain=0，尾段积压是 512 条 —— 压测没造出积压
FAIL: 60Hz 循环一次都没排空 —— 排空路径未被覆盖
（本轮共 20 项失败；整条判据墙钟 6 009 ms）
```

⚠ **这条注入教会我一件事（如实记下）**：`audio_dealloc` 与 `audio_releases` **仍然是 0** ——
因为 `publish` 把旧 anchor 也塞进了**写者侧待回收清单**（`SnapshotSlot::pending`），
所以"读者手里的那一份引用"不是最后一个强引用：就地 `drop` **不会**在音频线程产生 `dealloc`。
也就是说本设计对"音频线程零释放"有**两条独立的保险**：
① 读者推进退役队列；② 写者侧清单在读者确认前一直持有强引用。
I1a 变红靠的是"**退役队列被绕过**"（结构判据），不是"音频线程释放了内存"。

### I1b —— 音频线程就地 `drop` **且**写者侧不留强引用（真正的"音频线程释放"）

在 I1a 之上把 `publish_arc` 的 `pending.push` 换成 `drop(old)`。**实测红（规范点名的那两条）**：

```text
轮「高频交换」: ... audio_alloc=0 audio_dealloc=105120 audio_releases=10512 ... drain_calls=0
汇总: ... 音频线程 alloc=0 dealloc=525600 快照析构=52560 ...
FAIL: 音频线程窗口内发生了 105120 次堆释放 —— [MUST-GATE-012] 一票否决
FAIL: 音频线程上释放了 10512 个快照 —— 旧快照必须经退役队列交回主线程
FAIL: 退役队列的释放线程不是主线程：release_thread=None（主线程=ThreadId(1)）
FAIL: drain 窗口一次释放都没发生 —— 这条判据是空转（假绿）
```

注意 `dealloc=105120 ≈ 10×10512`：每个快照的内部容器（`BTreeMap`/`Vec`）也在同一线程被释放 ——
这正是"释放快照"在真实系统里的代价，也正是规范禁止它出现在音频线程上的原因。
（本轮共 30 项失败；**对账等式仍然成立**：`52565 == 52560 + 5` —— 没有泄漏，
只是释放**发生在了错误的线程**上。这正是判据①/④ 与判据③ 的分工。）

### I2 —— 关掉 60Hz 排空（`QUANTA_PER_60HZ_TICK = u64::MAX`）

**实测红（71 项失败）**：

```text
轮「高频交换」: ... switches=4097 stash=1 drain_calls=1 drained(churn)=0 tick_triggers=0 \
  backlog=4096 prune(pruned=4106 released=10 watched=10 alloc=0) drain(drained=4096 released=4096 watched=4096 alloc=0) \
  created=10513 released=4106 live=1 queue_pending=0
FAIL: 60Hz 节拍只跑了 0 拍，但音频时钟走过 834247 个量子 ⇒ 平均间隔 > 6 个量子（16.0 ms）
FAIL: 发生了 1 次退役队列满寄存（容量 4096）—— 高频压测被容量打折
FAIL: 零泄漏对账等式破：创建 10513 ≠ 释放 4106 + 存活 1
FAIL: 音频线程只切换了 4097 次快照，压测要求 ≥ 10000 次
FAIL: 音频线程在 5s 内没有观察到 revision 4099（交换握手失败）  …（×8/轮）
汇总: 对账等式: 创建 52565 == 释放 20530 + 存活 5 ⇒ 不成立   （整条判据墙钟 227.4 s，共 70 项失败）
```

三个附带发现（都是真的、都值得记）：
1. **泄漏真的会被对账抓到**：容量 4096 的队列满了以后，约 6 400 个快照停在写者侧清单里
   （读者不再前进 ⇒ `prune` 按 `reader_done` 不敢放）⇒ 等式**当场破**。这就是判据 ③ 的价值。
2. **设计的背压是"停切换"而不是"释放"**：队列满时 `begin_block` **放弃切换**、继续用旧快照
   （红线 7 优先）。于是控制侧的"追上新 revision"会**静默地**永远追不上 —— 注入后每轮
   8 次 5 秒握手超时把墙钟从 6 s 拉到 228 s。产品侧的教训见 needs N2。
3. 即使在这种病态下，**释放仍然全部发生在主线程**（`drain(512…)` 的 `released == watched`），
   音频线程的 `dealloc`/`析构` 仍是 0 —— 说明"零释放"不是靠"跑得快"，而是结构性的。

### I3 —— 让**别的线程**执行 `drain`

在归属 drain 窗口里用 `std::thread::scope` 把 `queue.drain(DRAIN_ALL)` 挪到另一个线程。**实测红**：

```text
轮「高频交换」: ... drain(drained=513 released=513 watched=0 alloc=4) foreign_drains=1
FAIL: 有 1 次 `drain` 发生在释放线程之外 —— 释放没有集中在主线程
FAIL: drain 窗口：出队 513 条、全局释放 513 次、主线程释放 0 次 —— 三者必须相等
FAIL: 主线程 60Hz 排空窗口内有分配：prune=0 drain=4 —— 排空路径必须零分配
```

（本轮共 15 项失败；`drain(drained=513 released=513 watched=0 alloc=4)`。）

⚠ **这条注入推翻了我第一版判据**：最初的实现把 `drain + prune` 放在**同一个**窗口里，
实测 `released == watched == 513`（因为队列与写者侧清单是同一个快照的两条强引用，
`drain` 放手后 `prune` 才放手 ⇒ 释放"漂"到了 prune 上）⇒ 注入**抓不住**。
现在拆成 **prune 窗口**（清写者侧清单，实测 `released=0`）与 **drain 窗口**（此时队列里每条
都是最后一个强引用 ⇒ `出队 == 全局释放 == 主线程释放`），这条注入才变成硬红。

### I4 —— 回调窗口里分配一次（`Vec::with_capacity(1)`）

**实测红**：

```text
轮「高频交换」: ... audio_alloc=21050 audio_dealloc=21050 audio_releases=0 ...
FAIL: 音频线程窗口内发生了 21050 次堆释放 —— [MUST-GATE-012] 一票否决
FAIL: 音频线程窗口内发生了 21050 次堆分配 —— `process_quantum` 调用树必须零分配
```

（本轮共 10 项失败；分配与释放各 21050 次 = 每量子一次：分配器窗口的**判别力**
在真实调用路径上被再证一次。）

---

## 5. "释放只在主线程"是**哪一类**证据（如实回答）

**调度证明（真测量），不是结构推断。** 三条独立证据：

1. **逐线程析构计数**：`EngineSnapshot::drop` 里记本线程的析构次数（`release_probe`，
   线程局部 `Cell<u64>`，无锁零分配）。判据在**音频线程自己的窗口里**读它 ⇒
   实测 `0`（且窗口一直覆盖到读者退场）。这是"这个线程没有释放这些对象"的直接测量。
2. **纯 drain 归属窗口**：主线程先 `prune`（实测 `released=0`）/再 `drain`，
   在 drain 窗口里 `出队 512 == 全局释放 512 == 主线程释放 512`。
   "全局"用进程级原子量计数、与"主线程"用线程局部量计数，两者**在窗口内逐一相等** ⇒
   这 512 次释放**全部**发生在执行 drain 的那个线程上。注入 I3（换线程 drain）当场变红。
3. **队列自身的记账**：`RetireQueue::release_thread()` 是**第一个非空 `drain` 的线程**
   `== 主线程`；`foreign_drains == 0`（在别的线程上发生过非空 drain 的次数）。

⚠ 必须说清楚的边界：这三条证明的是"**执行 drain/prune 的那个线程就是主线程，且释放就在那里发生**"。
它**没有**证明"`yeban-app` 的 60Hz 循环真的每 16.67 ms 跑一次"（那个循环在 app 侧，
不在本 crate；本线只把它按**音频时钟**模拟成"每 5 个量子一拍"）。也没有证明"音频线程"就是
cpal 的真实回调线程（见 §7）。

**窗口边界（两个窗口刻意不同）**：

| 窗口 | 覆盖范围 | 对应规范句 |
| :--- | :--- | :--- |
| 分配器（`alloc`/`dealloc`） | **回调路径**：预热之后、最后一个量子之前关窗 | 「音频线程无任何堆释放」（回调内） |
| 快照析构（`release_probe`） | 音频线程**整个生命周期**，含 `EngineRuntime` 析构（读者退场） | 「所有旧快照均在主线程释放」——这句话没有关流例外 |

分配器窗口不含读者退场，是因为 `EngineRuntime` 析构会释放 SPSC 环自身的内存
（`rtrb` 的生产/消费端缓冲）—— 那是**关流路径**的正当释放，不是回调行为。
而"读者手里的旧快照"在退场时**不会**被释放（实测 `audio_releases=0`）：它仍被 anchor 或
写者侧清单持有。

---

## 6. 60Hz 节拍的标定（一条实测教训）

节拍 = **每 5 个量子**（128 帧 × 5 ÷ 48 kHz = **13.3 ms**，标称 75 Hz），
而不是 `16.67 ms ÷ 2.667 ms = 6.25` 个量子。理由是"**检查粒度**"：

- 节拍检查只能落在**渲染量子边界**上，而且主线程在每个交换之间会离开自旋等待去构造快照
  （那段时间音频线程照跑，节拍看不到）；
- 实测（标称 6 个量子时）：`tick_triggers=3336` 覆盖 `21037` 个量子 ⇒
  **均值 6.31 个量子 = 16.8 ms**，比 60 Hz（16.67 ms）**略慢**；
- 标称 5 个量子时：实测均值 **5.26 个量子 = 14.0 ms**，于是
  "实际排空频率不低于 60 Hz"是**测出来的**。

**刻意没有**"相邻两次节拍最大间隔"的硬判据：实测这个量在同一台机器上（还有别的构筑在跑）
在 5 ~ 67 个量子之间跳（67 个量子 = 179 ms），而同一时刻 `max_pending_before_drain` 始终只有 3 ——
因为主线程被 OS 抢占时**同时也停止了发布**，所以"单拍被拉长"量的是**墙上时钟的调度**，
不是排空循环。它照常打印（可观测），判据只钉**平均速率**这条与调度无关的量，
以及 `stash_events == 0` / `pending` 有界 / 轮末 `pending == 0` 这三条硬性质。

---

## 7. 本机真跑 vs CI 的**严格区分**

| 项目 | 本机（M2，`aarch64-apple-darwin`） | CI（x86_64 Linux） |
| :--- | :--- | :--- |
| `cargo test -p yeban-engine --no-default-features --test snapshot_retire_churn` | ✅ **真跑**（定义性读数为 §3） | CI 用**默认 feature**（含 cpal）跑 `--workspace --all-targets` ⇒ 同一测试目标也会跑到，但**编译路径不同** |
| `cargo clippy -p yeban-engine --all-targets --no-default-features -- -D warnings` | ✅ **真跑**（见 §8） | ✅（默认 feature） |
| `bash scripts/gates/run-gates.sh light` | ✅ **真跑**（见 §8） | ✅（`checks` job） |
| `-p yeban-engine` 的**默认 feature**（cpal）构建 | ⛔ **未跑**（AGENTS.md §5 明文禁止：cpal 是重依赖） | ✅ |
| cpal 真实回调线程上的同族判据 | ⛔ **做不到**（见 §7 needs N3） | ⚠ 也**没有**：现有判据用的是 `std::thread` |
| 跨架构（L2 对账） | ⛔ 不在本线范围 | 由 CI 的 workspace 全量测试覆盖 |

**本次 CI 判决**：`pending`（提交推送后用 `bash scripts/dev/ci-verdict.sh line/gate-snapshot-churn`
读回；读回之前本行**不得**写成"通过"）。见 §10 的 needs N1（由集成者把 run id 写进
`docs/ledger/gate-status.md`）。

---

## 8. 门禁（本机真跑记录）

| 命令 | 结果 | 读数 |
| :--- | :--- | :--- |
| `bash scripts/gates/run-gates.sh light` | ✅ EXIT=0 | fmt + 机械红线守卫 + 文档契约（含链接/ID/门禁状态表）+ 依赖许可清单漂移 |
| `bash scripts/dev/cargo-local.sh clippy -p yeban-engine --all-targets --no-default-features -- -D warnings` | ✅ EXIT=0 | 0 警告 |
| `bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features` | ✅ | 131 条库单测（含本线新增 2 条）+ 5 个 `harness = false` 运行期目标 + 全部集成目标 |
| `bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test snapshot_retire_churn` | ✅ EXIT=0 | §3 的读数；整条判据 **6.0 ~ 6.2 s** |

新增的 2 条库单测（`src/snapshot.rs`）：
`drain_records_the_release_thread_and_flags_foreign_drains`（释放线程 / `foreign_drains` 语义）
与 `release_probe_counts_every_snapshot_drop_on_the_dropping_thread`（逐线程归账；
其中 `total()` 只断言"≥ 1"，因为 libtest 并行跑别的测试 —— **精确对账**只在
`harness = false` 的单判据进程里做，这一点在测试注释里写明了）。

---

## 9. 仍然**做不到**的部分 / 本判据没有证明的

1. **"音频线程"是 `std::thread`，不是 cpal 的真实回调线程**。本机按纪律不编译 cpal，
   所以判据用一个真的 OS 线程承载 `EngineRuntime`（同一个 `process_quantum` /
   `SnapshotReader` 代码路径，即生产回调里跑的那份代码）。**线程身份是替身，
   代码路径是真的**。真回调线程上的同族判据见 needs N3。
2. **"60Hz 循环"是按音频时钟模拟的**（每 5 个量子一拍），不是墙上时钟定时器；
   app 侧真正的 60Hz 循环（`yeban-app::engine_host`）没有在本次判据里被驱动。
   本判据证明的是**队列/释放的所有权语义**与**排空节奏能跟上交换速率**。
3. **没有机械化内存序验证**：`begin_block`/`publish`/`prune` 的 Acquire/Release 论证仍是
   模块文档里的纸面推导 + 运行期判据；没有 Miri / loom / TSan（引入它们是新依赖，需裁决）。
4. **释放探针本身是生产代码**（`impl Drop for EngineSnapshot` 里两个计数）⇒ 它是"自指"的仪器。
   缓解：① 主要仪器（计数型分配器）**不依赖探针**，两条独立证据必须同时为 0；
   ② 两条仪器都有自检与注入证据（§4）；③ 探针出错会先破对账等式。
5. **单拍最大间隔不受约束**（只报告）：那是 OS 调度，不是本循环的节拍。
6. **没有跨架构**：x86_64 Linux 上的行为由 CI 的 workspace 全量测试覆盖，本文件不声称跑过。
7. **判据规模不可下调**：`total_swaps >= 50_000` 是硬断言；本机 ~6 s，无需为省时间削弱它。

---

## 10. needs（交给集成者 / 后续工作线）

- **N1（集成者，必须）**：把 `docs/ledger/gate-status.md` 的 `MUST-GATE-012` 行从
  **部分** 改为 **已接线/闭环**，理由指向本文件 §3 的读数与 §4 的注入；
  并把 CI 判决 **run id** 写进去（本线提交后用 `ci-verdict.sh` 读回后才能写"通过"）。
  同时建议在 `docs/DEVELOPMENT_LEDGER.md` 记一条教训：**"注入要打在被保护的路径上"**——
  I1a（就地 `drop`）之所以没让 `dealloc` 变红，是因为写者侧清单还持有一份强引用（§4）。
- **N2（engine / app）**：`stash_events > 0` 意味着**读者停止切换快照**（设计选择：绝不阻塞、
  绝不在音频线程释放），控制侧"追上新 revision"会**静默**失败。建议：
  ① 把 `snapshot_stash_events()` 接进 `EngineStats` / UI 可观测面；
  ② 控制面在 `stash_events` 增长时降速发布或提示"引擎拓扑更新被推迟"。
- **N3（engine-rt / 集成者）**：在**真实 cpal 回调线程**上做同族判据（CI 上跑，或由人类解除
  本机 cpal 编译限制）。可以直接复用 `tests/snapshot_retire_churn.rs` 的
  `release_probe` + 线程局部分配器写法，把音频线程换成 cpal 的 stream 回调。
- **N4（人类裁决）**：`begin_block`/`publish`/`prune` 的 Miri / loom 机械化验证（新依赖）。
- **N5（engine）**：`RetireQueue::release_thread()` / `foreign_drains()` 现在是 crate 内可读；
  是否要进 `EngineStats`（给 app/UI 一个"释放线程是否单一"的健康读数）请集成者裁决。
- **N6（app）**：`yeban-app::engine_host` 的 60Hz 排空循环应当有自己的"排空间隔/积压"读数，
  与本判据的音频时钟节拍做端到端对账（本线只覆盖引擎侧的队列语义）。

---

## 11. 复跑命令 / 修改文件与净行数

```bash
# 判据本体（本机真跑；~6 s）
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test snapshot_retire_churn

# 本 crate 的 clippy + 全部测试（轻量变体，不编译 cpal）
bash scripts/gates/run-gates.sh crate yeban-engine

# 轻量门禁（零编译）
bash scripts/gates/run-gates.sh light
```

| 文件 | 变更 | 净行数（`git diff --numstat` / `wc -l`） |
| :--- | :--- | :--- |
| `crates/yeban-engine/src/snapshot.rs` | 改 | **+252 / −0**（`release_probe` 模块 + `impl Drop` + 队列释放线程记账 + 2 条单测） |
| `crates/yeban-engine/src/rt.rs` | 改 | **+12 / −0**（`snapshot_stash_events()`） |
| `crates/yeban-engine/Cargo.toml` | 改 | **+10 / −0**（`[[test]] harness = false`） |
| `crates/yeban-engine/tests/snapshot_retire_churn.rs` | **新建** | **933 行** |
| `docs/ledger/gate-snapshot-churn-notes.md` | **新建** | **374 行**（本文件） |

**没有**新增依赖（`Cargo.lock`、根 `Cargo.toml`、`deny.toml` 未动）；**没有**改
`crates/**`（本 crate 之外）、`schemas/**`、`.github/**`、`scripts/**`、
`docs/YEBAN_*.md`、`docs/adr/**`、`docs/DEVELOPMENT_LEDGER.md`、
`docs/ledger/{gate-status,phase-status,feature-alignment,human-decisions}.md`、法务文件。
