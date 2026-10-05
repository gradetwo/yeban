# `engine-mirror-race` —— `retire_pending` 镜像漂移的定位 / 修法 / 静止点口径 / 见证 / 注入

- **台账类型**：缺陷定位记录 / 修法 / 判据口径 / 注入证据 / 本机真跑与 CI 的严格区分 / needs（**不是规范**）
- **工作线**：`line/engine-mirror-race`（worktree `yeban/.worktrees/engine-mirror-race`，分支同名，基线 main `c4d222c`）
- **所有者目录**：`crates/yeban-engine/**`（本文件是唯一新增的共享区文档）
- **CI 原文（两轮都红）**：

  ```text
  [MUST-GATE-012] FAIL: 轮「高频交换」: EngineStats 镜像与队列读数不一致：retire_pending 镜像=512 权威=513
  [MUST-GATE-012] FAIL: 轮「高频交换（复跑）」: 同上
  error: test failed, to rerun pass `-p yeban-engine --test snapshot_retire_churn`
  ```

- **集成者裁定**：这是判据设计问题还是真缺陷，必须先给证据；**不许**用"重跑就好"或放宽断言糊过去。
- **本轮交付**：本文件 + 代码改动（3 文件）**同一次推**。`git diff --stat` = 3 文件、**+332 / −30**
  （`src/snapshot.rs`、`src/rt.rs`、`tests/snapshot_retire_churn.rs`）。
  - 补丁仓库外副本：`/Users/crow/work/music/emr-engine-changes.patch`（**已被本次同推取代，仅作历史**）
  - 之前那次 notes-only 推送的 run `37281143589` = `success`，但
    `rust (workspace 全量)` / `rust (matrix.crate)` / `windows` **skipped steps=0**
    ⇒ **空心绿，不算判决**（`plan` 判受影响 crate 集合为空）。本次带代码，才会真跑 `rust` 腿。

---

## 1. 定位结论（一句话）

> **不是"两处读数不在同一静止时刻"，是镜像真的漂了 —— 而且是永久的。**
> 判据 ① 的比较点本来就是静止点（`join` 之后、任何 `prune`/`drain` 之前），
> 它抓到的是一个**记账缺陷**：`RetireAccounting::note_drain` 的"覆写剩余量"与
> 音频线程的 `note_push`（`+1`）**不是原子的**，窗口里落进来的 `+1` 被**永久**盖掉。

### 1.1 为什么"判据读得不一致"这条假设被排除

`line/engine-stats` 写下的对账段（`tests/snapshot_retire_churn.rs`，基线 HEAD）本身就是同一静止点：

| 步骤 | HEAD 行号 | 事实 |
| :--- | :--- | :--- |
| 音频线程退出渲染循环后读一次 `EngineStats`（镜像） | 434 | `let stats = runtime.stats();` 在 `while` 之外 |
| 主线程等音频线程收摊（`join`） | 553 | 这期间主线程**只**在 `join` 里阻塞 |
| 主线程读队列/槽的权威读数 | 570 | `queue.pending()` 等 —— **在** 553 之后 |
| 两轮之间是否有 `prune`/`drain` | — | **没有**：归属窗口的 `prune`/`drain` 在 622 之后才发生 |

也就是说：镜像读完之后，到权威读数之前，**没有任何** push（生产端随音频线程退出而析构）、
也没有任何 drain。若只是"读的时刻不同"，两者的差应当**可自愈**；而 CI 报的是
`镜像=512 权威=513`：权威比镜像**大 1**，方向是"镜像**少记**了一条"。

### 1.2 漂移的确切窗口（代码级证据）

基线 HEAD `crates/yeban-engine/src/snapshot.rs`：

| 行号 | 代码 | 作用 |
| :--- | :--- | :--- |
| 837-841 | `fn note_push` ⇒ `self.pending.fetch_add(1, Release)` | 音频线程：成功 `push` ⇒ 量规 `+1` |
| 847-851 | `fn note_drain(taken, remaining)` ⇒ `saturating_bump(drain_calls)` / `saturating_bump(drained)` / `self.pending.store(remaining)` | 控制线程：**覆写**量规为"出队后的真实剩余" |
| 1023 | `self.accounting.note_drain(count, self.consumer.slots());` | `remaining` 实参在**调用点**求值（丢弃循环之后） |

交错（这是**唯一的**丢失路径，且与调度无关，只是窗口宽窄问题）：

```text
  控制线程（drain）                                音频线程
  ────────────────────────────────────────────     ─────────────────────────────
  ① 求值 self.consumer.slots() = R
  ② saturating_bump(drain_calls)   ← CAS 循环
  ③ saturating_bump(drained, n)    ← CAS 循环
                                                   ④ inner.push(X)  环里多一条 X
                                                   ⑤ note_push()    pending += 1
  ⑥ pending.store(R)               ← ④⑤ 的 +1 被**永久**盖掉
```

②③ 是两条对**同一条缓存行**的 CAS 循环；在两条线程互抢该缓存行时，
①→⑥ 的窗口是**几百纳秒**量级；若控制线程在这中间被 OS 抢占，窗口直接拉到毫秒级。
⇒ 镜像**永久**少记一条（不是瞬时抖动：没有任何后续动作会把它加回来）。

**为什么只丢"最后一条"级别的一笔**：覆写式量规每个非空 `drain` 都会用真实剩余盖一次
⇒ 第 k 次 drain 造成的漂移会被第 k+1 次 drain **治好**；能活到静止点的只有
**最后一轮 drain** 造成的那一笔。这正是 CI 的形态：5 轮里**部分轮**红、且差值恰好为 1。

### 1.3 与 CI 数字逐项对上

| 量 | 值 | 解释 |
| :--- | :--- | :--- |
| `BACKLOG_PER_ROUND` | 512（HEAD `tests/snapshot_retire_churn.rs:91`） | 尾段"只发布不排空"512 次 |
| 静止点**精确**待回收 | 512 + 1 = **513** | 512 条尾段积压 + 高频段最后一条没被排掉的 |
| 镜像 | **512**（少 1） | 最后一轮 drain 的覆写吞掉了高频段那条的 `+1` |
| CI 报的差值 | `镜像=512 权威=513` | 与"少记一笔"**逐位**吻合 |

本机复跑也观察到同样的两个量级：`backlog=513`（精确）与 `pushed − drained = 513`（修好后）。

---

## 2. 修法（本次随代码一起推）

**不去动判据的比较时刻，改记账**（因为比较时刻本来就是对的）：

| 位置 | 改动 | 理由 |
| :--- | :--- | :--- |
| `RetireAccounting` | `pending` 字段 ⇒ `pushed`（成功入队数，`fetch_add`）；`pending()` = `pushed.saturating_sub(drained)` | 两个操作数都**只增不减** ⇒ 不存在"读旧值再覆写"的步骤 ⇒ 不存在能丢 `+1` 的窗口 |
| `note_drain` | 去掉 `remaining` 参数与 `store`；只 `saturating_bump(drained, taken)` | 覆写本身就是缺陷来源（它只在**下一次** drain 才自愈） |
| `drain()` 的 `Err` 分支 | 不再记 `drain_calls` | 与字段语义（**非空** drain 次数）一致；该分支按注释不可达 |
| `RetireProducer::push` | 注释钉死顺序：**先入环、后记账** | 反过来会让 drain 在 `+1` 落地前消费掉它 ⇒ 永久多记（`saturating_sub` 夹不回那一笔） |
| `rt.rs` / `snapshot.rs` 文档 | "覆写 ⇒ 漂移自愈"改成"两个单调量之差，静止点上恒等" | 原文档描述的是一个**错误**的设计承诺 |
| 新增库单测 | `retire_pending_mirror_is_the_difference_of_two_monotonic_counters`（替换旧的"覆写自愈"单测）+ `pending_mirror_stays_exact_at_every_quiescent_point_while_drain_races_push`（400 个静止点样本的并发见证） | 旧单测把缺陷当成契约钉住了，必须换口径 |
| 库单测的**静止点协议** | 用**代际回执**（`ack: AtomicU64` 只增不减 + 三条命令 `PUSH` / `PARK` / `QUIT`），并把 `PARK` 与 `QUIT` 分成**两个**状态 | 第一版把"停下"与"收工"写成同一个状态值 ⇒ 生产者会在 epoch 边界**退出线程**，下一轮"等环里有货"空转到断言红（本机带负载连跑第 14 次复现 `snapshot.rs:2139:17: 生产者没有开始 push`）。那是**判据自己的状态机缺陷**（集成者在全量跑里也踩到，报 `2140:17`），修法是加回执世代，**不是**放宽断言 |

**没有任何容差**：静止点上要求**等号**；并发窗口里允许瞬时差 1（环里已放进、`+1` 未落地），
那是量规的正常语义 —— 判据比的是**静止点**，不是并发中的某一瞬。

---

## 3. 静止点口径（复用既有口径，只补"证明它是静止的"）

口径**照抄** `line/engine-stats` 的那一招，不另造：

```text
  音频线程退出渲染循环 →（写回 AudioReport / 读一次 EngineStats）
  → 主线程 join 返回 → 此刻读队列与槽的权威读数 → 逐项比等号
```

本线**新增**两件"证明静止"的事（因为"同一静止时刻"这句话在第一版里只是**断言**）：

1. **生产端已不存在**：`Arc::strong_count(queue.accounting()) == 1`。
   音频线程退出时 `EngineRuntime` 析构把 `RetireProducer` 丢了 ⇒ 静止点之后
   **不可能**再有 push。这是**结构事实**，不是"等一会儿大概就停了"。
2. **静止点前后各读一次全套读数**（`pending` / `dropped` / `drain_calls` / `foreign_drains` /
   `slot.pruned()` / `pushed` / 镜像），中间刻意 `spin_loop` × 64 + `yield_now()`；
   两次必须**逐项相等**。真有在飞的 push/drain，这两次就会不同。

---

## 4. 见证（MUST-GATE-001 注入 I4 的三条要求）

| 要求 | 本线的见证 | 实测读数（本机，修好后单跑） |
| :--- | :--- | :--- |
| ① 读到的是**非平凡值**（不是两个 0） | 静止点精确待回收 ≥ 尾段积压 512；且 `drained` / `drain_calls` / `pruned` 都 **> 0** 才允许判绿 | `精确待回收=513 镜像=513 pushed=10512 drained=9999 drain_calls=4003 pruned=9999` |
| ② "静止"是**真的** | 上面 §3 的两件事：`join` + 生产端强引用 == 1 + 双读逐项相等 | `双读相等=true 生产端强引用=1` |
| ③ 没有用容差掩盖缺陷 | 判据是**等号**；注入"记账少记一次" ⇒ 当场红，且红出来的数字与 CI **逐位相同** | 注入 I1 实测：`精确待回收=513 镜像=512 pushed=10511`、`FAIL: 轮「高频交换」: EngineStats 镜像与队列读数不一致：retire_pending 镜像=512 权威=513` |

另有**结构等式**作为第三重见证：静止点上 `pushed − drained == 精确待回收`
（注入 I1 时它同时报 `... 入队/出队账不平：pushed=10511 − drained=10000 ≠ 精确待回收=512`）。

库内的并发见证（`snapshot.rs` 的 `pending_mirror_stays_exact_at_every_quiescent_point_while_drain_races_push`）
每个 epoch 取一个静止点样本、共 **400 个样本**，判据是**等号**，并且自己带非平凡自检
（`pushed_total > 0`、`non_zero_samples > 0`、`max_exact > 0`、`drained > 0`）。

---

## 5. 判据清单与注入记录

### 5.1 判据（本线新增/强化的部分）

| 编号 | 判据 | 强度 |
| :--- | :--- | :--- |
| ① | 高频交换轮在**静止点**上镜像 == 权威（`pending`/`drained`/`drain_calls`/`pruned`/`foreign_drains`/归属布尔逐项等号） | 既有（`line/engine-stats`），保留不动 |
| ①′ | 静止点上 `pushed − drained == 精确待回收`（结构等式） | **新增** |
| ② | 静止点前后两次全套读数逐项相等 + `Arc::strong_count(accounting) == 1` | **新增** |
| ③ | 见证：精确待回收 ≥ 尾段积压；`drained`/`drain_calls`/`pruned` 严格 > 0 | **新增** |
| ④ | 注入"记账少记一次" ⇒ 判据红；"多记一次" ⇒ 反向红；"覆写式竞态" ⇒ 库内见证红 | 注入（见 5.2） |
| ⑤ | 既有 `MUST-GATE-012` 全部判据仍绿（50 000 次交换 / 音频线程零分配零释放 / 主线程归属 / 零泄漏等式） | 既有，**一个字未改** |
| ⑥ | `run-gates.sh light` + `run-gates.sh crate yeban-engine`（本机自动走 `--no-default-features`） | 门禁（实测 EXIT=0，见 5.3） |
| ⑦ | 库内并发见证：400 个 epoch 的"drain 与 push 真并发 ⇒ 静止点上必须等号"（含非平凡自检） | **新增**（`src/snapshot.rs` 单测） |

### 5.2 注入记录（3 组，"注入 → 变红 → 还原 → 复绿"；原始输出在下方）

| 注入 | 内容 | 结果（原始输出摘要） |
| :--- | :--- | :--- |
| **I1 记账少记一次** | `note_push` 的第一条不记账（`AtomicBool` 一次性闸门） | **红（churn）**：`静止点见证「高频交换」: 精确待回收=512 镜像=511 pushed=10511 drained=10000`；`FAIL: 轮「高频交换」: EngineStats 镜像与队列读数不一致：retire_pending 镜像=511 权威=512`；`静止点上入队/出队账不平：pushed=10511 − drained=10000 ≠ 精确待回收=512`；`I1_EXIT=1` |
| **I2 记账多记一次** | `note_push` 第一条记 2 | **红（churn，反方向）**：`精确待回收=514 镜像=515 pushed=10513 drained=9998`；`FAIL: ... retire_pending 镜像=515 权威=514`；`I2_EXIT=1` |
| **I3 覆写式竞态（忠实再现第一版）** | `note_drain` 在窗口里等一次并发 `push` 落进来，然后 `pushed.store(stale)`（= 第一版的 `pending.store(remaining)`） | **红（库内并发见证，第 1 个样本就红）**：`panicked at src/snapshot.rs:2179:13: assertion left == right failed: 静止点上镜像必须与精确读数相等（第 1 个样本）`，`left: 11 right: 12`；`I3_EXIT=101` |

三条注入都**逐一还原**，还原后 `sha256sum src/snapshot.rs` =
`22e3af3a9b1471058f6f995c0e151a02cf12a332fcaed8f93d3ed72ef3b8c574`（与注入前逐字节相同），
且还原后立刻跑出 §5.3 的绿。

⚠ I3 第一版（在 churn harness 上做同样的注入）**没能制造丢失**：churn 的音频线程在
`wait_for_revision` 期间可能已经追上写者、没有在飞的 push，于是"等一次 push"空转；
反而把 harness 拖停，先红的是 60Hz 节拍判据（`音频时钟走过 70007 个量子`）。
⇒ I3 的证据取**库内并发见证**（那里的生产者是**连续 push** 的，窗口里一定有 push）。
这条也说明：**注入打在被保护的路径上**不等于打在被保护的交错上（与 `gate-snapshot-churn-notes.md` §4 同族教训）。

### 5.3 连跑与门禁结果（"不再抖"必须如实登记）

| 对象 | 代码版本 | 次数 / 方式 | 结果 |
| :--- | :--- | :--- | :--- |
| `snapshot_retire_churn` | **未修**（基线 HEAD） | 连跑 6 | 6/6 绿（`EXIT=0`）—— **本机没复现** |
| `snapshot_retire_churn` | 修好后 | 连跑 5 | **5/5 绿** |
| `cargo test -p yeban-engine --no-default-features --lib` | 修好后 | 连跑 3 | **3/3 绿**（每次 145 passed） |
| 库内并发见证单测（`--lib pending_mirror`） | 修好后 + **4 个 CPU 打满进程** | 连跑 **60** | **60/60 绿**（旧协议在同一手法下第 14 次红，见 §2 末行） |
| `cargo test -p yeban-engine --no-default-features`（**全目标**，含 5 个 `harness = false`） | 修好后 + 3 个 CPU 打满进程 | 1 轮 | `test result: ok. 145 passed; 0 failed` + `[MUST-GATE-001] ok` + `[MUST-GATE-012] ok: 50000 次高频交换 ... 零泄漏对账成立`；`FULL_TARGETS_EXIT=0` |
| `run-gates.sh crate yeban-engine`（fmt/guards/docs/licenses + clippy `-D warnings` + 全目标 test） | 修好后 | 1 轮 | **`GATE_CRATE_EXIT=0`** |
| `run-gates.sh light` | 修好后 | 1 轮 | **EXIT=0** |

⚠ **必须如实说明**：本机（M2）**没有**复现 CI 的红 —— 未修版本连跑 6 次全绿。
这不削弱定位结论（1.2 的窗口与 1.3 的数字是代码级 + CI 读数级证据），
但意味着"连跑 N 次全绿"**不能**当作"修好了"的证据；**只有 CI 判决算数**（AGENTS §5.5）。

---

## 6. 本机 vs CI（严格区分）

| 维度 | 本机（Apple M2，debug，`--no-default-features`） | CI（GitHub runner） |
| :--- | :--- | :--- |
| 复现 | **未复现**（未修版本 6 连跑全绿） | **已复现**（两轮判决都红，2 个轮次） |
| 为什么 | 核多、负载低 ⇒ 控制线程在"①→⑥ 窗口"里被抢占的概率低 | runner 核少、负载高 ⇒ 抢占落在窗口里的概率高，窗口从纳秒拉到毫秒 |
| 结论 | 本机绿=参考；**判决以 CI 为准** | 本线的修法必须在 CI 上拿到判决 |

本机命令（工作树内，纪律 L30）：

```bash
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --test snapshot_retire_churn
bash scripts/dev/cargo-local.sh test -p yeban-engine --no-default-features --lib
bash scripts/dev/cargo-local.sh clippy -p yeban-engine --all-targets --no-default-features -- -D warnings
bash scripts/gates/run-gates.sh light
bash scripts/gates/run-gates.sh crate yeban-engine     # 本机自动用 --no-default-features（D19）
```

---

## 7. 仍未证明 / 不确定的部分（如实登记）

1. **本机没复现** ⇒ "修法确实消灭了 CI 上的那次红"要靠 **CI 判决**才能确认；
   本机能给的只有"按构造不存在该窗口"+"静止点等式在 400 个并发样本 + 5 轮 50 000 次交换上成立"。
2. **判据自己的状态机缺陷（已被集成者抓到，已修）**：本线第一版库内见证单测把
   "停下"与"收工"写成同一个状态值，且初值就是"收工" ⇒ 生产者线程可能在 epoch 边界退出，
   下一轮的"等环里有货"空转到 `生产者没有开始 push` 断言红。集成者在全量跑里报
   `snapshot.rs:2140:17`，本机带负载连跑第 14 次复现同一断言（`2139:17`）。
   ⚠ 这**不是**被测对象的漂移（修法本身没被推翻），但它是"本机已绿"这句话的**反例**：
   同类缺陷只在高负载 / 全量并发跑时暴露 ⇒ 判据本身也必须**带负载连跑**过才算见过世面。
3. I3 的证据口径：它只在"生产者**连续 push**"的库内见证上是确定性的；
   在 churn harness 上同样的注入会先撞上 60Hz 节拍判据（见 §5.2 的 ⚠），
   所以 churn 侧仍以 I1/I2 两个**确定性**注入为证。
4. `docs/ledger/engine-stats-notes.md` 里"`drain` 后**覆写**为真实剩余 ⇒ 漂移自愈"
   （§2 字段表、§2.2 边界表）现在是**错的**；本线按降档指令**不动别人的台账**，
   把更正登记为 needs N2。
5. 判据 ② 的"双读"是控制线程上的读；它证明的是"没有写者"，不是"音频线程真的停了"
   —— 后者由 `join` + 生产端强引用计数共同承担。
6. 真实 cpal 回调线程不在本机验证范围内（AGENTS §5：本机不编译重依赖）。
7. **跨工作树的 git stash 是共享的**（同一个 `.git`）：本线在 `git stash pop` 时误弹了
   `line/origin-contract` 的 `stash@{0}`（它的改动被应用进本工作树，产生
   `crates/yeban-model/src/ops.rs` 冲突）。已用 `git restore --staged --worktree` 把该文件
   还原到 HEAD、并把那条 stash **原样保留**（未 drop）。⇒ 建议把"弹 stash 前先核对
   stash 的 `On <branch>` 与文件清单"记进工作流纪律。

---

## 8. needs（交给集成者 / 下一条线）

| 编号 | needs | 为什么 |
| :--- | :--- | :--- |
| **N1** | 集成者把本线这次推的**判决**读回并回填台账（`MUST-GATE-012` 的 gate-status 由集成者独占） | 本线只交 notes + 代码；判决由 CI 给 |
| **N2** | 修 `docs/ledger/engine-stats-notes.md` 里两处"覆写自愈"的描述（§2 字段表 `retire_pending` 行、§2.2 边界表） | 它们描述的是缺陷，不是契约 |
| **N3** | app 侧（`yeban-app`）**不要**依赖"镜像会被下一次 drain 自愈"这一行为；`retire_pending` 是**量规**，静止点上才与精确读等号 | 60Hz 循环的降速决策只看 `stash` 增量，不看 `pending` |
| **N4** | 把"同一门禁连跑 N 次 **+ 带 CPU 负载**"登记为**可选**判据（不是硬门槛）：本机连跑不能替代 CI 判决，但能抓"低频漂移 / 判据自身状态机缺陷"这类只在负载下暴露的问题 | 本次两条缺陷都是低频的：CI 2/5 轮次红；本线判据的协议缺陷要带负载连跑 14 次才现形 |
| **N5** | 判据"镜像 == 权威"的**静止点**定义应当写进 `MUST-GATE-012` 的台账口径（本文件 §3），后续工作线复用 | 避免下一条线再自己发明口径 |
| **N6** | 把"弹 stash 前先核对 `On <branch>` 与文件清单；跨工作树 stash 共享同一个 `.git`"写进工作流纪律 | 本线误弹了 `line/origin-contract` 的 stash（见 §7 第 7 条），差点把别人的改动带进本线的提交 |
