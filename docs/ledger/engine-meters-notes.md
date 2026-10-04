# `yeban-engine` 电平生产侧台账：口径、判据、实测证据与待办（电平工作线）

- **台账类型**：电平口径 / 判据清单 / 实测证据 / 未决项（**不是规范**）
- **记录时刻**：2026-10-05（第 1 轮；本机 `windows-write` 沙箱 + 受限 rustup）
- **工作线**：`line/engine-meters`（worktree `yeban/.worktrees/engine-meters`）
- **所有者目录**：`crates/yeban-engine/**`（本台账是唯一新增的文档文件）
- **落地范围**：新增 `crates/yeban-engine/src/level.rs`；改写 `src/meter.rs` 的生产/消费侧；
  改写 `src/rt.rs` 的 `render_block`；新增 `crates/yeban-engine/tests/meter_rt_contract.rs`
- **规范来源**：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 的 `ARCH-UI-002`（第 188 行原文）、
  `ARCH-RT-001`、`ARCH-DET-001/002`、`ARCH-TOP-002`；
  `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` 的 `ROAD-M2-007`、`ROAD-M2-008`；
  `MUST-GATE-001`（实时回调零分配）

> 本文件回答五个问题：**电平口径到底是什么（含时间常数与 dBFS 换算）**、
> **每个量子到底发布几条**、**UI 侧"丢弃旧帧只取最新"是怎么保证的**、
> **每条判据怎么变红（含 4 条注入实测）**、**哪些东西明确没做、需要谁裁决什么**。

---

## 0. 这一线改了什么（一句话）

改之前：`render_block` 对**每一条** `tracks()` 里的节点（含主总线）都调
`MeterFrame::measure(id, quantum, block.left())`。由于轨道渲染是占位静音，发布出去的
四类值恒为 `0.0` —— 电平通道"只是计数"。

改之后：

1. **真实口径**：峰值、峰值保持（20 dB/s 指数释放）、块 RMS、平滑 RMS（τ=300 ms 一阶低通）、
   dBFS 换算、`NaN`/`±∞` 钳位全部实现在零依赖的 `src/level.rs`，每节点状态由
   `meter::MeterBank` 在实时线程上以定长数组持有；
2. **真的逐轨 + 母线**：每轨先在 `track_scratch`（复用的 `[f32; 128]`）里渲染，测**单声道、
   声相之前**的电平，再汇入立体声母线块；母线单独测**立体声联动**电平；
3. **口径修正**：主总线本身也是 `TrackV3`、通常就在 `tracks()` 里。旧代码会把母线
   **当成普通轨再计一次**（同一 `EntityId` 每量子两条帧，其中"轨道那条"测的是整块母线）。
   现在母线只出母线那一条：**每量子帧数 = 非母线轨数 + 1**；
4. **发布契约可判**：每量子**恰好一次** `publish`（`bulk_push_calls` / `meter_bulk_publishes`
   可断言），批次 = 本量子全部帧；
5. **UI 侧"只取最新"成为 collector 的能力**：新增 `MeterCollector::drain_latest`
   （抽干积压 + 每节点留最新一帧），`MeterBoard` 的覆盖判据抽成纯函数 `level::supersedes`。

### 0.1 ⚠ 必须如实说明的边界

- **引擎仍然不能发声**。`render_track_into` 目前写静音（声部合成/采样播放属于
  `yeban-sfz` / `yeban-dsp` 的后续切片，见 `docs/ledger/engine-rt-notes.md` §5.1）。
  因此**端到端**（`process_quantum`）发布出来的电平恒为静音。
- 所谓"真实电平"指的是：**计量计算与接线是真的**（每轨一个独立 tap、弹道状态跨量子连续、
  `NaN` 钳位、母线立体声联动、每量子一次批量发布），而"喂进去的信号"目前是占位静音。
  正弦/单调/`NaN` 三类判据打在 `level.rs` 与 `MeterBank::measure` 上 ——
  后者就是 `render_block` 每轨调用的**同一个函数**（不是另写一份"测试专用"实现）。
- 母线汇流 `sum_into_bus` 目前是**等增益写两声道**（mono → L/R 各 +x），
  **不是**声相定律。真正的等功率声相、发送/辅助汇流、PDC 对齐属于混音台切片。

---

## 1. 电平口径（判据要钉住的就是这张表）

全部实现在 `crates/yeban-engine/src/level.rs`（**零依赖**：不引用 cpal/rtrb/yeban-model，
也不引用本 crate 其它模块 ⇒ 可用 `rustc --test` 单独跑，见 §5.1）。

| 量 | 定义 | 单位 / 备注 |
| :--- | :--- | :--- |
| 输入钳位 | `NaN → 0.0`；`±∞` 与越界有限值 `→ ±16.0` | 4× 满量程 = **+24.08 dBFS**；保证后续全是有限数 |
| 峰值 `peak` | 本量子内 `max abs(x)`（钳位后） | 线性，`1.0` = 0 dBFS |
| 峰值保持 `peak_hold` | `max(peak, peak_hold × release)` | 每量子乘子；默认释放 **20 dB/s** |
| RMS `rms` | 本量子 `sqrt(mean(x²))`，不跨量子 | 线性 |
| 平滑 RMS `rms_smoothed` | 对**均方**做一阶低通：`ms ← c·ms + (1−c)·mean(x²)`，再开方 | 默认 **τ = 300 ms** |
| dBFS | `20·log10(幅度)`；`幅度 ≤ 0` ⇒ **负无穷** | `MeterFrame::peak_dbfs/rms_dbfs` |
| 静音下限 | `level::SILENCE_FLOOR_DBFS = −120.0` | UI 柱高用有限值，替代 `-∞` |
| 母线口径 | **立体声联动**：峰值取 L/R 最大绝对值；均方 = `(Σl² + Σr²) / (2n)` | 长度不等按较短者，不 panic |
| 轨道口径 | **单声道、声相之前**（`track_scratch`） | 多声道独立电平未实现（§7） |

### 1.1 时间常数的折算与理由

`quanta_per_second = sample_rate / block_frames`（规范默认 48 000 / 128 = **375**）。
系数按每量子折算，在**快照切换**时用 `MeterBank::set_quanta_per_second` 重算一次
（只做 `powf`/`exp`，零分配/零锁/零 I/O；状态保留）：

```text
peak_release   = 10^(−decay_dB_per_sec / 20 / qps)       // 20 dB/s @375Hz ⇒ 0.993880
rms_coeff      = exp(−1 / (τ · qps))                     // τ=300ms @375Hz ⇒ 0.991151
```

- **20 dB/s**：业界峰值表常见的回落速率（1 秒回落一个数量级）。参考实现下 375 个量子后
  保持值恰好 = 起始的 1/10，判据 `peak_hold_decays_by_the_documented_rate` 断言偏差 < 0.05 dB。
- **τ = 300 ms**：接近 VU 的积分观感。判据 `smoothed_rms_follows_the_documented_time_constant`
  断言"τ 秒后达到阶跃目标的 `1−1/e`"（容差 0.01）以及长时间收敛到块 RMS（容差 0.005）。
- 这两个数是**选择**而不是规范硬性数字，所以以公开常量 + 文档形式给出，
  允许后续按 UI 观感调整（调整必须同步改 §4 的判据，否则就是"表和代码各说各话"）。

### 1.2 为什么输入要钳位（这不是 [ARCH-DSP-001] 的去爆音）

实时路径可能拿到 `NaN`/`±∞`（数值爆炸、上游 bug、未初始化读入）。若原样进入电平，
`NaN` 参与比较恒为假 ⇒ 峰值保持会**永久卡死**在 `NaN`，UI 曲线再也回不来。
所以 `level::sanitize_sample` 把所有非有限/越界值钳到有界范围，`MeterFrame::is_sane`
给 UI 一个廉价的断言口。**这与 [ARCH-DSP-001]（语音偷取淡出、参数 5 ms 平滑、
A/B 交叉淡化）无关**，本线不做任何音频 DSP。

---

## 2. 每个量子发布几条（实测证据）

### 2.1 语义

- 发布**恰好一次**/量子：`MeterPublisher::publish` 每量子只被调用一次，
  `push_partial_slice` **恰好一次**（批量契约 [ROAD-M2-007]）；
- 批次内容 = `非母线轨（按 EntityId 升序） ++ 母线一条`；
- 母线**不重复计量**（即使它在 `tracks()` 里）；
- `EngineStats::meter_bulk_publishes` == "有快照的量子数"；`meter_frames` == 写进队列的帧数。

### 2.2 实测数字（本机真跑，见 §5.1 的口径）

`filled_project()` 夹具：`tracks()` = 主总线 + lead + bass + aux = 4 条 ⇒ **非母线轨 3 + 母线 1 = 4 帧/量子**。

```text
[meter-rt] S1 汇总: quanta=10242 publishes=10242 frames=40968 capacity_drops=0
```

- `publishes == quanta == 10242`（**每量子恰好一次**；若改成每轨一次，同一命令实测
  `publishes=40968` ⇒ 见 §5.2 注入 4）；
- `frames == 4 × 10242 == 40968`（**帧数 = 非母线轨数 + 1**，且 0 丢弃）；
- `meter_capacity_drops == 0`（容量充足）；
- 单元判据 `rt::tests::bus_is_metered_once_even_when_master_is_in_the_track_map` 直接断言
  节点序列 = 非母线轨升序 + 母线，且母线只出现一次。

### 2.3 与旧语义的差异（说明为什么改）

| | 旧 | 新 |
| :--- | :--- | :--- |
| 每量子帧数 | `tracks().len() + 1`（母线在 `tracks()` 里时**重复**计量） | 非母线轨数 + 1 |
| 每轨来源 | 全部测 `block.left()`（整块母线，逐轨相同） | 各自 `track_scratch`（声相前单声道） |
| 每轨值 | 无状态（每次新建检测器） | 有状态（峰值保持/平滑 RMS 跨量子连续） |
| 批量次数 | 1/量子（已经是对的） | 1/量子（保持，并用计数器钉住） |

---

## 3. UI 侧"丢弃旧帧、只取最新"

### 3.1 两条消费路径

| API | 行为 | 结构性契约 |
| :--- | :--- | :--- |
| `MeterCollector::tick(scratch)` | FIFO：一次 `pop_partial_slice` 搬走**前缀**（旧语义，保留） | 每 tick **恰好一次**批量读 |
| `MeterCollector::drain_latest(scratch)` | **抽干整个积压**，每节点只留 `quantum` 最大的一帧 | `bulk_pop_calls` 每轮 +`⌈积压 / DRAIN_CHUNK⌉`（积压正好整除时多一次空读） |

`drain_latest` 是这一线新增的：它让"丢弃旧帧、只取最新"从 `MeterBoard` 的义务变成
**collector 自身的能力**，并且顺手消灭了一个真实的危险 —— 旧写法下 UI 每次只搬
`scratch.len()` 条，若每 UI 帧生产量大于搬运量，队列会**长期全满**，
而 `rtrb` 满时丢的是**最新**帧 ⇒ UI 永远看不到最新。抽干积压后这个正反馈不存在了。

### 3.2 覆盖判据（纯函数）

`MeterBoard::ingest` 只在 `level::supersedes(candidate, held)`（即 `candidate >= held`）时覆盖。
`>=` 而非 `>`：同量子重复投递幂等，不会回退。于是**乱序/重放/迟到**的旧帧永远顶不回新值。

### 3.3 有损语义（诚实口径）

队列真满时 `rtrb` 写入的是**前缀**、丢的是**最新**帧。这不是 bug 而是取舍
（[ARCH-UI-002] 要求"绝不阻塞音频线程"），但必须**可观测**：

- `MeterPublisher::dropped()` > 0 = "UI 已经落后到会丢最新帧"的健康告警；
- 每帧带 `quantum`，`MeterBoard::latest_quantum()` 与音频线程当前量子一比就知道新鲜度；
- 判据 S6（`tests/meter_rt_contract.rs`）钉住：容量 8、送 20 帧 ⇒ 写入 8、`dropped == 12`、
  UI 看到的最大量子是 **7**（而不是 19）⇒ "落后"是可被 UI 检测的事实，而不是静默的谎言。

---

## 4. 判据清单（每条都写清"怎么变红"）

单元判据在 `src/level.rs`（14 条）/`src/meter.rs`（12 条）/`src/rt.rs`（11 条）；
运行期判据在 `tests/meter_rt_contract.rs`（`harness = false`，7 个场景）与既有的
`tests/rt_zero_alloc.rs`。

| # | 判据 | 测试名（文件） | 注入什么会变红 |
| :-: | :--- | :--- | :--- |
| m1 | 满幅正弦 ⇒ 峰值 ≈ 0 dBFS；块 RMS ≈ −3.01 dBFS | `full_scale_sine_peaks_at_zero_dbfs`（`level.rs`） | 峰值取 `sum` 而不是 `max abs`；RMS 不除以 n |
| m2 | 半幅正弦 ⇒ −6.02 dBFS | `half_scale_sine_is_minus_six_dbfs` | 漏掉 `20·log10` 里的 20 |
| m3 | 全零/空块 ⇒ 峰值 0、dBFS 负无穷、**无 NaN** | `zero_input_is_negative_infinity_or_floor_never_nan` | `rms = sum/0`（`0/0 = NaN`）不改 |
| m4 | 不同幅度**单调** | `levels_are_monotonic_in_amplitude` | 用 `min` 代替 `max` 求峰值 |
| m5 | `NaN`/`±∞`/1e30 输入 ⇒ 读数有限且有界 | `nan_and_infinity_inputs_never_produce_nan_levels`、`sanitize_clamps_and_never_returns_nan`、`meter_bank_sanitizes_hostile_input_to_finite_levels` | **删掉 `sanitize_sample`** ⇒ 实测 `peak: inf`（注入 3） |
| m6 | 峰值保持 1 秒恰好回落 20 dB | `peak_hold_decays_by_the_documented_rate` | `peak_release = 1.0`（不释放）；写成线性 dB 而不是幅度乘子 |
| m7 | 平滑 RMS 在 τ 秒达到 `1−1/e`、长时间收敛 | `smoothed_rms_follows_the_documented_time_constant` | `rms_coeff = 0`（不平滑） |
| m8 | 立体声联动（单声道满幅 = 0 dBFS；RMS 按两声道平均） | `stereo_reading_is_channel_linked`、`meter_bank_bus_is_stereo_linked_and_silence_stays_finite` | 只看左声道；均方只用单声道分母 |
| m9 | `supersedes`：同量子/更新接受，更旧拒绝 | `supersedes_accepts_equal_and_newer_but_not_older` | 改成 `>`（同量子重复投递被拒）或恒真 |
| m10 | `MeterBank` 状态绑定节点身份（重排不丢状态） | `meter_bank_keeps_per_node_state_across_reordering` | 槽位改成"按调用下标对齐"（重排即重置） |
| m11 | `drain_latest` 批量灌入后每节点只留最后一帧 | `drain_latest_returns_the_newest_frame_per_node_and_empties_the_queue` | 覆盖判据改成**顺序取**（注入 2：实测取到 quantum=0） |
| m12 | 迟到旧帧不得回退 UI | `lagging_consumer_never_treats_a_stale_frame_as_fresh`、`board_keeps_only_the_latest_frame_per_node_and_ignores_stale_updates` | `MeterBoard::ingest` 无条件插入 |
| m13 | `scratch` 装不下时仍然抽干队列（防积压） | `drain_latest_respects_scratch_capacity_without_losing_the_rest` | `unique == len` 时 `return`（队列残留） |
| m14 | 每量子发布帧数 = 非母线轨 + 1；母线不重复 | `bus_is_metered_once_even_when_master_is_in_the_track_map`（`rt.rs`） | 去掉 `*id == master` 的过滤 |
| m15 | 每量子**恰好一次**批量发布 | `null_path_renders_silence_and_counts_quanta`、`meter_frames_are_published_per_track_with_bulk_api`、`S1/S2`（`meter_rt_contract.rs`） | 改成每轨单独 `publish`（注入 4：实测 `publishes=40968` vs `quanta=10242`） |
| m16 | 静音输入 ⇒ 全帧有限、峰值 0、dBFS 负无穷 | `silent_input_yields_finite_silent_frames`、`S3` | 删钳位；用 `0/0` 求 RMS |
| m17 | **窗口内零分配/零释放**（带真实电平） | `S1`（`tests/meter_rt_contract.rs`）与 `rt_zero_alloc.rs` | 在 `render_block` 里 `Vec::with_capacity(1)` ⇒ 实测 `allocations=10000`（注入 1） |
| m18 | `drain_latest` 自身零分配 | `S7` | collector 内部 `Vec` 临时缓冲 |
| m19 | 队列溢出可观测（`dropped`、可见 quantum 落后） | `S6` | 删掉 `dropped` 计数；把满队列行为改成"阻塞等待" |
| m20 | 容量耗尽不 panic、不扩容 | `oversized_track_set_is_counted_and_never_panics`、`rt::...` | 用 `scratch_meters[produced]` 直接索引（越界 panic） |

> **关于"出队耗时 < 0.05 ms"**：沿用本 crate 既有取舍（`engine-rt-notes.md` §4）——
> 本机明文禁止重依赖编译、CI runner 抖动大，墙钟断言不可标定。因此用**结构性**判据
> （每 tick/每量子恰好一次批量 API、抽干轮数上界）+ **运行期分配计数**替代。

---

## 5. 本机真跑 vs 交给 CI（严格区分）

### 5.1 本机（M2，Apple Silicon）**真的跑过**的

| 命令 | 结果 | 说明 |
| :--- | :--- | :--- |
| `rustc --edition 2024 --test -D warnings crates/yeban-engine/src/level.rs` | ✅ **14 passed** | 纯计算抽成零依赖模块的**全部**判据（正弦/dBFS/单调/NaN/衰减/时间常数/取最新） |
| `cargo check -p yeban-engine --no-default-features --all-targets` | ✅ | **不编译 cpal**（feature `device` 关闭，ADR-0001 D19 的离线构建形态） |
| `cargo clippy -p yeban-engine --no-default-features --all-targets -- -D warnings` | ✅ 0 告警 | 同上 |
| `cargo test -p yeban-engine --no-default-features --all-targets` | ✅ **74 passed + 2 个 `harness = false` 目标 ok** | 见 §5.3 的 105 个测量窗口 |
| `bash scripts/gates/run-gates.sh light` | ✅ | fmt + 13 条守卫 + 文档门禁（47 个 md / 121 链接）+ 许可清单对账 |
| 4 条注入 → 变红 → 字节级还原 | ✅ | 见 §5.2；还原后用 `cmp` 逐文件确认 identical |

> ⚠ **本机没有编译默认 feature（`device` = cpal）**。`src/device.rs` 与 `rt.rs` 里那条
> `NullBackend: Send` 的 feature 门控断言**只由 CI 判定**。本线对这两处的改动为零，
> 但"整仓默认构建是否绿"仍然只能由 CI 回答。

### 5.2 注入 → 变红 → 还原（4 条，全部字节级还原）

| 注入 | 改法 | 实测红点 |
| :--- | :--- | :--- |
| 1（分配） | `render_block` 里 `let _probe = Vec::<u8>::with_capacity(1);` | `rt_zero_alloc`：`allocations=10000 deallocations=10000`（窗口 10,000 量子），退出码 1；`meter_rt_contract`：每个 256 量子窗口 `allocations=256`，`FAIL: [S1] …一票否决` |
| 2（取最新） | `drain_latest` 里覆盖判据改成 `if false && supersedes(...)` | 单元 `drain_latest_returns_the_newest_frame_per_node_and_empties_the_queue` FAILED；集成 `S4a 取到的必须是最后一个量子, 实际 0`、`S4b 迟到的旧帧不得更新 UI`、`S6 最新可见量子应为 7, 实际 0` |
| 3（钳位） | `sanitize_sample` 开头 `return sample;` | `rustc` 单跑 level.rs：`2 failed`；cargo：`3 failed`（含 `meter_bank_sanitizes_hostile_input_to_finite_levels`）；集成 `S5 … peak: inf` |
| 4（发布次数） | 单次批量 `publish` 改成每轨一次 | 4 条 `rt.rs` 单测 FAILED；集成 `S1 publishes=40968`（quanta=10242）、`S2 期望 7, 实际 28` |

还原判据：`cmp` 对 `src/level.rs`、`src/meter.rs`、`src/rt.rs`、`src/lib.rs`、
`tests/meter_rt_contract.rs`、`Cargo.toml` 逐文件比对备份 ⇒ 全部 identical；
`grep -rn INJECT crates/yeban-engine` ⇒ 0 命中（L24：改完立刻 grep 复核）。

### 5.3 零分配窗口的实测数字

- `tests/rt_zero_alloc.rs`（既有）：10,000 量子 + 63 次快照交换 ⇒ `allocations=0 deallocations=0`，
  退出码 0（**加了真实电平计算后仍然 0**）；
- `tests/meter_rt_contract.rs`（新）：40 个窗口 × 256 量子 = 10,242 量子（其中 10 次快照交换），
  每个窗口 `allocations=0 deallocations=0`；汇总 `quanta=10242 publishes=10242 frames=40968`；
  另有 `drain_latest x2048` 窗口 `allocations=0 deallocations=0`；
- 本次 cargo 运行共打印 **105** 行 `allocations=0 deallocations=0`（`grep -c` 实测）。

### 5.4 交给 CI 的（本机未跑）

- `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace --all-targets`
  的**默认 feature** 形态（含 cpal / Slint / symphonia 等重依赖）；
- `cargo deny check`（本机只跑了 light 档的许可清单对账）；
- 跨平台（x86_64 Linux runner）的电平数值复现 —— `log10`/`powf`/`exp` 的 1 ulp 差异
  只影响观测量，不影响 [MUST-GATE-002] 的音频 digest。

### 5.5 CI 判决（**只有 CI 的判决算数**）

| 轮 | run id | 结论 | 关键读数 |
| :-: | :--- | :--- | :--- |
| 1 | `37235815404`（commit `cdf58cd`） | ✅ **全绿** | `checks` ✅ 31s（fmt + 13 守卫 + schema + 文档链接 48 文件 / 121 链接 + 许可清单）、`lockfile` ✅ 19s、`deny` ✅ 44s、`plan` ✅ 6s、**`rust (yeban-engine)` ✅ 49s**（默认 feature，含 cpal/device：`clippy --all-targets -D warnings` + `test --all-targets`）。`rust (workspace 全量)` 被 `plan` 按受影响集合跳过（本次只动 `yeban-engine` + 文档）。 |

CI（x86_64 Linux）上与代码同源的关键读数（从 job 日志抓取，与本机完全一致）：

```text
test result: ok. 84 passed; 0 failed           ← 默认 feature(含 device 的 10 条)
[meter-rt] S1 汇总: quanta=10242 publishes=10242 frames=40968 capacity_drops=0
[ARCH-UI-002] ok: 真峰值/RMS 计量 + 每量子一次批量发布 + UI 取最新 + 10,000 量子零分配
[MUST-GATE-001] ok: 10,000 量子 + 63 次快照交换，实时窗口内零分配零释放
（allocations=0 deallocations=0 共 105 行）
```

> 注：本文件**追加 CI 记录**的那次提交是纯文档提交，其 `plan` 会按受影响集合
> **跳过 rust 腿**（L23 的形状）。因此"代码被验证过"的锚点是 `cdf58cd` / run
> `37235815404`，不是之后的文档提交；读判决时必须同时回答"哪个 SHA"与"哪条腿真的跑了"。

---

## 6. 判据覆盖到的结构性契约（"每量子恰好一次批量发布"等）

| 契约 | 计数器 | 判据 |
| :--- | :--- | :--- |
| 每量子恰好一次电平批量写 | `MeterPublisher::bulk_push_calls` / `EngineStats::meter_bulk_publishes` | m15 |
| 每 tick 恰好一次电平批量读 | `MeterCollector::bulk_pop_calls` | m15、`meter_bulk_contract_is_one_call_per_quantum_and_per_ui_tick` |
| 每块恰好一次事件批量出队 | `EngineStats::event_bulk_pops` | 既有 `events_are_drained_once_per_block_and_applied` |
| 发布帧数 = 非母线轨 + 1 | `EngineStats::meter_frames` | m14、m15 |
| 容量耗尽不 panic/不扩容 | `EngineStats::meter_capacity_drops` = 批次放不下 + `MeterBank::capacity_drops` | m20 |

---

## 7. 未实现项（**不要误读为已做**）

1. **UI 侧消费**（混音台通道条、Slint Property 更新、60Hz 定时器接线）不在本线地盘：
   `MeterBoard` 已经能"只取最新"，但**没有任何 UI 在消费它**；
2. **多声道独立电平**：母线是立体声联动（L/R 合并成一个读数），
   声道各自的电平、以及 >2 声道的口径都没有；
3. **真正的超采样峰值表 / 真峰值（true-peak）**：现在测的是样本峰值，
   没有 4× 过采样，因此**会漏掉采样点之间的过冲**（intersample peak）；
   `LUFS`/`ITU-R BS.1770` 的响度计量也未实现；
4. **母线汇流不是混音**：`sum_into_bus` 等增益写两声道，没有 pan law、发送/辅助增益、
   mute/solo 应用、PDC 对齐（这些是混音台切片）；
5. **轨道渲染仍是占位静音**：`render_track_into` 是唯一接入点，等待声部合成切片；
6. **峰值保持的"钉住时间"未做**：只有指数释放，没有"保持 N ms 再释放"的 UI 观感层；
7. **`NaN` 钳位没有计数上报**：钳掉了多少样本不可观测（只在 `MeterFrame::is_sane` 上体现）；
8. **`dropped` 没有接到 UI 告警**：`MeterPublisher::dropped` 只在发布侧可读，
   collector 侧没有对应的"我落后了"信号；
9. **RT 线程优先级 [ROAD-M2-001]** 仍未实现（沿用 `engine-rt-notes.md` 的 N3）；
10. **`MeterBank` 的容量淘汰策略是最久未计量（LRU）**，不是"按轨道优先级保留"；
    超过 255 条非母线轨时被淘汰的节点电平会重新起跳（会计数，不会静默）。

## 8. needs（需要人类/集成者/其它线裁决）

| ID | 内容 | 归属 |
| :--- | :--- | :--- |
| MN1 | **把纯计算上移到 `yeban-dsp`**：`level.rs` 的 `LevelDetector`/`dbfs`/`dbfs_clamped`/`sanitize_sample`/`supersedes`/`LevelReading` 是零依赖纯计算，符合 [ARCH-DSP-*] 的归属。本线按要求**只做最小实现并留在 `yeban-engine` 内部**（不改别的 crate）。上移后 `yeban-engine` 改为依赖 `yeban-dsp`，`src/level.rs` 退化为 re-export（或直接删除）。 | `yeban-dsp` 线 + 集成者 |
| MN2 | **`docs/ledger/gate-status.md` 的证据行需要补**：`MUST-GATE-001` 现在只写 `tests/rt_zero_alloc.rs`；本线新增 `crates/yeban-engine/tests/meter_rt_contract.rs`（同一个分配器、但跑"带真实电平的量子"）。状态**不变**（仍为"部分"：零锁等待与零阻塞 I/O 的运行期断言仍缺），但证据应并列两个文件。 | 集成者（本线不改共享台账） |
| MN3 | **是否把有损 SPSC 换成"每节点单槽最新值"（seqlock/原子快照）**：现在的语义是"队列满丢**最新**帧 + UI 用 quantum 判断新鲜度"。若产品要求"UI 永远显示最新"，单槽方案更直接；代价是偏离 [ARCH-UI-002] 的 SPSC 措辞。**本线没有改架构**，只把丢失变成可观测。 | 人类/架构裁决（可能触发 ADR） |
| MN4 | **真峰值/响度计量的口径**：若要做 4× 过采样真峰值或 LUFS，需要确定算法与容差（并决定是否引入依赖）。 | 人类/架构裁决 |
| MN5 | **UI 消费切片**（`MeterBoard` → Slint Property、60Hz 定时器、`drain_latest` vs `tick` 的取舍）。 | `yeban-app` 线 |

## 9. CI 改动需求

**没有。** 本线**没有**往 `.github/workflows/ci.yml` 加任何步骤（那是集成者的文件）。
新增的 `[[test]] name = "meter_rt_contract", harness = false` 会由既有的
`cargo test --all-targets`（per-crate 腿与 workspace 腿各一处）自动构建并执行 ——
与 `rt_zero_alloc` 完全同一条路径（`ci.yml` 第 196 / 255 行）。

## 10. 复用来源（Reuse provenance）

本线代码是**新写**的：

- 峰值保持 + 一阶平滑 RMS 是标准计量弹道（教科书形式），按 [ARCH-UI-002] 的措辞实现；
- `MeterCollector`/`MeterPublisher`/`MeterBoard` 的 SPSC 与批量契约沿用 `src/ring.rs`
  已有的模式（同仓库、同作者、同一许可），**没有复制其它项目的代码**；
- `dbfs`/`sanitize`/`supersedes` 是十行以内的纯函数；
- 未引入任何新依赖，因此**不新增** `THIRD_PARTY_LICENSES.md` 条目，也不需要 hoist。

---

## 追加（集成者代记）：**MN1 已由 `line/dsp-level` 关闭**

- **MN1「把电平纯计算上移到 `yeban-dsp`」→ 已完成**：`crates/yeban-dsp/src/meter.rs`（新，含 285 条冻结位模式）+
  `loudness.rs`（BS.1770-4 K 加权 + 无门限 LUFS 最小子集）；`crates/yeban-engine/src/level.rs` 退化为纯 `pub use`。
  **调用方改动量为 0**，engine 侧的实时状态（`MeterFrame`/`MeterBank`/SPSC/`MeterBoard`）与"每量子一次批量发布"留在原处。
- **engine 没有第二份实现的机械证据**：源码扫描（`include_str!` + 实现记号）+ 类型/函数地址比较（`fn_addr_eq`）+ 行为冻结表。
- **结构性读数与本文档上一轮**逐字相同**：`quanta=10242 publishes=10242 frames=40968 capacity_drops=0`、
  105 行 `allocations=0 deallocations=0`。⇒ 搬迁**没有**改变实时侧行为。
- 该线另外新增了 4× 真峰值与 LUFS 最小子集，并在 CI 上被**自己写的逐位判据**抓到 1 ULP 的跨架构差异
  （`f32::log10` 不保证正确舍入）⇒ 已升级为 **ADR-0001 D32**（按运算类别分策）。
- 仍未做（转记）：峰值保持"钉住时间"（须独立类型，不得改 `LevelDetector` 否则破坏逐位契约）、
  多声道独立电平、UI 侧消费、真峰值倍数（4× 在 0.4·fs 欠读 0.44 dB）、其它采样率的 K 加权系数。

