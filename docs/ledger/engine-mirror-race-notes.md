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
- **本轮交付形态（集成者降档指令）**：**只提交本文件**。代码改动（修法 + 判据 + 见证）
  已在本机完成并全绿，按指令 `git stash` 留在本工作树，**下一轮推**：
  - stash：`stash@{0} On line/engine-mirror-race: engine-mirror-race: 记账修法+判据+见证（本机已绿，下一轮推）`
  - 同一份补丁的仓库外副本：`/Users/crow/work/music/emr-engine-changes.patch`（495 行）
  - 该补丁的 `git diff --stat`：3 文件、**+332 / −30**（`src/snapshot.rs` +246 段、`tests/snapshot_retire_churn.rs` +110 段、`src/rt.rs` 6 行）

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

## 2. 修法（已在本机完成，按降档指令 stash，下一轮推）

**不去动判据的比较时刻，改记账**（因为比较时刻本来就是对的）：

| 位置 | 改动 | 理由 |
| :--- | :--- | :--- |
| `RetireAccounting` | `pending` 字段 ⇒ `pushed`（成功入队数，`fetch_add`）；`pending()` = `pushed.saturating_sub(drained)` | 两个操作数都**只增不减** ⇒ 不存在"读旧值再覆写"的步骤 ⇒ 不存在能丢 `+1` 的窗口 |
| `note_drain` | 去掉 `remaining` 参数与 `store`；只 `saturating_bump(drained, taken)` | 覆写本身就是缺陷来源（它只在**下一次** drain 才自愈） |
| `drain()` 的 `Err` 分支 | 不再记 `drain_calls` | 与字段语义（**非空** drain 次数）一致；该分支按注释不可达 |
| `RetireProducer::push` | 注释钉死顺序：**先入环、后记账** | 反过来会让 drain 在 `+1` 落地前消费掉它 ⇒ 永久多记（`saturating_sub` 夹不回那一笔） |
| `rt.rs` / `snapshot.rs` 文档 | "覆写 ⇒ 漂移自愈"改成"两个单调量之差，静止点上恒等" | 原文档描述的是一个**错误**的设计承诺 |
| 新增库单测 | `retire_pending_mirror_is_the_difference_of_two_monotonic_counters`（替换旧的"覆写自愈"单测）+ `pending_mirror_stays_exact_at_every_quiescent_point_while_drain_races_push`（400 个静止点样本的并发见证） | 旧单测把缺陷当成契约钉住了，必须换口径 |

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
（注入 I1 时它同时报 `... 入队/出队账不平：pushed=10511 − drained=9999 ≠ 精确待回收=513`）。

---

## 5. 判据清单（下一轮随代码一起推）与注入记录

### 5.1 判据（本线新增/强化的部分）

| 编号 | 判据 | 强度 |
| :--- | :--- | :--- |
| ① | 高频交换轮在**静止点**上镜像 == 权威（`pending`/`drained`/`drain_calls`/`pruned`/`foreign_drains`/归属布尔逐项等号） | 既有（`line/engine-stats`），保留不动 |
| ①′ | 静止点上 `pushed − drained == 精确待回收`（结构等式） | **新增** |
| ② | 静止点前后两次全套读数逐项相等 + `Arc::strong_count(accounting) == 1` | **新增** |
| ③ | 见证：精确待回收 ≥ 尾段积压；`drained`/`drain_calls`/`pruned` 严格 > 0 | **新增** |
| ④ | 注入"记账少记一次" ⇒ 判据红；"多记一次" ⇒ 反向红 | 注入（见 5.2） |
| ⑤ | 既有 `MUST-GATE-012` 全部判据仍绿（50 000 次交换 / 音频线程零分配零释放 / 主线程归属 / 零泄漏等式） | 既有，**一个字未改** |
| ⑥ | `run-gates.sh light` + `run-gates.sh crate yeban-engine`（本机自动走 `--no-default-features`） | 门禁 |
| ⑦ | 库内并发见证：400 个 epoch 的"drain 与 push 真并发 ⇒ 静止点上必须等号"（含非平凡自检） | **新增**（`src/snapshot.rs` 单测） |

### 5.2 注入记录（"注入 → 变红 → 还原 → 复绿"）

| 注入 | 内容 | 结果（原始输出） |
| :--- | :--- | :--- |
| **I1 记账少记一次** | `note_push` 的第一条不记账（`AtomicBool` 一次性闸门） | **红**：`[MUST-GATE-012] FAIL: 轮「高频交换」: EngineStats 镜像与队列读数不一致：retire_pending 镜像=512 权威=513`（与 CI 原文逐字相同）+ `静止点上入队/出队账不平：pushed=10511 − drained=9999 ≠ 精确待回收=513`；`EXIT=1` |
| **I2 记账多记一次** | `note_push` 第一条记 2 | **待跑**（下一轮与代码一起给原始日志） |
| **I3 覆写式竞态（放大窗口）** | `note_drain` 里等一次真 push 落进窗口再"覆写为旧读数"（忠实再现第一版缺陷 + 模拟 CI 的抢占） | **待跑**（下一轮给原始日志） |

I1 还原后 `sha256sum src/snapshot.rs` 与注入前**逐字节相同**：
`71a11d0134ca42d7d36c1e87dedcdf38d951ba944f56997c6df5c036d163026a`；
还原后复跑 `snapshot_retire_churn` = `EXIT=0`。

### 5.3 连跑结果（"不再抖"这件事必须如实登记）

| 对象 | 代码版本 | 次数 | 结果 |
| :--- | :--- | :--- | :--- |
| `snapshot_retire_churn` | **未修**（基线 HEAD） | 连跑 6 | 6/6 绿（`EXIT=0`）—— **本机没复现** |
| `snapshot_retire_churn` | 修好后 | 连跑 5 | **5/5 绿**（`EXIT=0`） |
| `cargo test -p yeban-engine --no-default-features --lib` | 修好后 | 连跑 3 | **3/3 绿**（每次 145 passed） |
| `cargo clippy -p yeban-engine --all-targets --no-default-features -- -D warnings` | 修好后 | 1 | `CLIPPY_EXIT=0` |

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
   本机能给的只有"按构造不存在该窗口"+"静止点等式在 400 个并发样本上成立"。
2. I3（忠实再现覆写式竞态）**还没跑**：它会引入一次刻意的 `spin`/抢占放大，
   属于"注入"而非产品代码，下一轮给原始日志后从工作树里彻底删掉。
3. `docs/ledger/engine-stats-notes.md` 里"`drain` 后**覆写**为真实剩余 ⇒ 漂移自愈"
   （§2 字段表、§2.2 边界表）现在是**错的**；本线下一轮改代码时一并修正那两行，
   本轮按降档指令**不动它**。
4. 判据 ② 的"双读"是控制线程上的读；它证明的是"没有写者"，不是"音频线程真的停了"
   —— 后者由 `join` + 生产端强引用计数共同承担。
5. 真实 cpal 回调线程不在本机验证范围内（AGENTS §5：本机不编译重依赖）。

---

## 8. needs（交给集成者 / 下一条线）

| 编号 | needs | 为什么 |
| :--- | :--- | :--- |
| **N1** | 下一轮**立刻**把 stash 里的修法 + 判据推上去（`stash@{0}`，或 `/Users/crow/work/music/emr-engine-changes.patch`） | 本轮只交了 notes；`main` 上的 `MUST-GATE-012` 仍然是红的 |
| **N2** | 修 `docs/ledger/engine-stats-notes.md` 里两处"覆写自愈"的描述（§2 字段表 `retire_pending` 行、§2.2 边界表） | 它们描述的是缺陷，不是契约 |
| **N3** | app 侧（`yeban-app`）**不要**依赖"镜像会被下一次 drain 自愈"这一行为；`retire_pending` 是**量规**，静止点上才与精确读等号 | 60Hz 循环的降速决策只看 `stash` 增量，不看 `pending` |
| **N4** | 把"同一门禁连跑 N 次"登记为**可选**判据（不是硬门槛）：本机连跑不能替代 CI 判决，但能在 CI 上抓"低频漂移"类缺陷 | 本次缺陷就是低频的（2/5 轮次） |
| **N5** | 判据"镜像 == 权威"的**静止点**定义应当写进 `MUST-GATE-012` 的台账口径（本文件 §3），后续工作线复用 | 避免下一条线再自己发明口径 |
