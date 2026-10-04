# `yeban-engine` API 核验台账、判据与待办（实时引擎工作线）

- **台账类型**：API 事实核验 / 判据清单 / 未决项（**不是规范**）
- **记录时刻**：2026-10-05
- **工作线**：`line/engine-rt`（worktree `yeban/.worktrees/engine-rt`）
- **所有者目录**：`crates/yeban-engine/**`（本台账是唯一新增的文档文件）
- **落地范围**：`block` / `fpu` / `graph` / `ring` / `snapshot` / `meter` / `rt` / `device` 八个模块
- **规范来源**：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3、§0.1；
  `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M2-001..004`、`ROAD-M2-007..008`

> 本文件回答四个问题：**我用到的 API 到底是什么签名（以及从哪读来的）**、
> **每条判据怎么变红**、**哪些东西明确没做**、**需要谁裁决什么**。

---

## 1. 依赖与 API 事实核验（**先查证再写代码**）

### 1.1 依赖登记与版本

| 依赖 | 版本 | 来源 | 是否已登记在根 `[workspace.dependencies]` |
| :--- | :--- | :--- | :--- |
| `cpal` | `0.18.2` | 根登记（`default-features = false`） | ✅ |
| `rtrb` | `0.4.0` | 根登记（`default-features = false`） | ✅ |
| `thiserror` | `2.0.21` | 根登记 | ✅ |
| `yeban-model` | `0.0.1`（path） | 根登记为**纯 path** | ⚠️ 见 §1.3 |

**没有新增任何未登记依赖，因此没有需要 hoist 的新 crate。**
唯一的 `TODO(hoist)` 是内部 crate 的版本补登记（§1.3）。

### 1.2 核验过的 API 签名（逐条给出出处）

核验方法：从 crates.io 静态包下载**真实源码**（不是文档站转述），逐行读 `src/**`。
因此下面的签名是源码级事实。

**`rtrb` 0.4.0**（`rtrb-0.4.0/src/{lib.rs,chunks.rs}`，
crates.io `https://crates.io/api/v1/crates/rtrb/0.4.0`，
文档 `https://docs.rs/rtrb/0.4.0/rtrb/`）：

| 项 | 签名（源码原文） | 用途 |
| :--- | :--- | :--- |
| `Producer::push` | `pub fn push(&mut self, value: T) -> Result<(), PushError<T>>` | 退役队列（`Arc` 非 `Copy`） |
| `Producer::push_partial_slice` | `pub fn push_partial_slice<'a>(&mut self, slice: &'a [T]) -> (&'a [T], &'a [T]) where T: Copy` | 事件/电平**批量写** |
| `Producer::push_entire_slice` | `pub fn push_entire_slice(&mut self, slice: &[T]) -> Result<(), ChunkError> where T: Copy` | （未用；整批要么全成功） |
| `Producer::{slots, cached_slots, is_full, is_abandoned, buffer}` | `-> usize / bool / &RingBuffer<T>` | 容量与背压判断 |
| `Consumer::pop_partial_slice` | `pub fn pop_partial_slice<'a>(&mut self, slice: &'a mut [T]) -> (&'a mut [T], &'a mut [T]) where T: Copy` | **每块一次**批量读 |
| `Consumer::read_chunk` | `pub fn read_chunk(&mut self, n: usize) -> Result<ReadChunk<'_, T>, ChunkError>` | 退役队列批量出队（`Arc` 非 `Copy`） |
| `ReadChunk` 的 `IntoIterator` | `type Item = T`；"When the iterator is dropped, all iterated slots are made available for writing again" | move 出 `Arc` 并 Drop |
| `ChunkError` | 唯一变体 `TooFewSlots(usize)` | 批量的失败模式 |
| `PushError` | `Full(T)` / `Abandoned(T)` | 满队列时**拿回**元素（不在音频线程丢） |
| `Consumer::{pop, peek, slots, is_empty, is_abandoned, buffer}` | `-> T / &T / usize / bool` | — |
| `RingBuffer::new(capacity)` | `capacity` **原样**成为 `RingBuffer::capacity()`（源码实测：**没有** 2 的幂取整） | 队列建立 |

**`cpal` 0.18.2**（`cpal-0.18.2/src/{lib.rs,traits.rs,error.rs,sample_format.rs,platform/mod.rs}`，
crates.io `https://crates.io/api/v1/crates/cpal/0.18.2`，
文档 `https://docs.rs/cpal/0.18.2/cpal/`）：

> ⚠️ **0.18 与 0.15/0.16 的 API 有实质差异**，凭记忆写会编译失败。实测差异如下。

| 项 | 0.18.2 签名 | 与旧版的差异 |
| :--- | :--- | :--- |
| `DeviceTrait::build_output_stream` | `fn build_output_stream<T, D, E>(&self, config: StreamConfig, data_callback: D, error_callback: E, timeout: Option<Duration>) -> Result<Self::Stream, Error>`，`D: FnMut(&mut [T], &OutputCallbackInfo) + Send + 'static` | **多了 `timeout: Option<Duration>` 参数**；且 `config` 按值传入 |
| `DeviceTrait` 超 trait | `PartialEq + Eq + Hash + Debug + Display`，新增必填方法 `description()` 与 `id()` | 旧版没有 `id()`/`description()`；**没有 `name()`**，设备名走 `Display` |
| `DeviceTrait::default_output_config` | `-> Result<SupportedStreamConfig, Error>` | 一致 |
| `HostTrait` | `fn devices(&self) -> Result<Self::Devices, Error>`（**返回 `Result`**）；`fn default_output_device(&self) -> Option<Self::Device>`；`fn is_available() -> bool`（无 `self`） | `devices()` 由 `Iterator` 变成 `Result<Iterator>` |
| `SupportedStreamConfigRange` | `pub fn new(channels, min_sample_rate, max_sample_rate, buffer_size, sample_format)` + 同名 getter；`try_with_sample_rate` / `try_with_standard_sample_rate` | **`new` 与 getter 都是 `pub`** ⇒ 能力协商可以脱离真实设备单测（本工作线据此设计 `device::negotiate`） |
| `StreamConfig` | `{ channels: ChannelCount, sample_rate: SampleRate, buffer_size: BufferSize }` | 一致 |
| `BufferSize` | `Default \| Fixed(FrameCount)`（`FrameCount = u32`） | 一致 |
| `SupportedBufferSize` | `Range { min, max } \| Unknown` | 一致 |
| `SampleFormat` | `#[non_exhaustive]` 枚举（`F32, F64, I16, …`） | **`non_exhaustive`** ⇒ 任何 `match` 必须带通配分支 |
| `Error` | 结构体（`ErrorKind` + `Option<Cow<'static, str>>`），实现 `Display + std::error::Error` | 旧版是枚举 |
| `StreamTrait` | `play(&self)` / `pause(&self)` / `buffer_size(&self)` / `now(&self)` | 建流后**默认停止**，必须显式 `play()` |
| `cpal::default_host()` | `-> Host`（动态分派枚举） | 一致 |
| `realtime` feature | 只覆盖 `wasapi` / `aaudio` / `pipewire` / `jack`（`#[cfg(feature="realtime")]` 出现在这些 host 的 `stream.rs`），**macOS 与 Linux-ALSA 无任何开关** | 见 §4 needs |

**`std` 原子量**（`https://doc.rust-lang.org/std/sync/atomic/`）：
`AtomicPtr::{new, load, store}`、`AtomicU64::{load, store, fetch_add, fetch_max}`、
`AtomicBool::{load, store}`；所用 `Ordering` 为 `Acquire`/`Release`/`AcqRel`。
另用 `Arc::{as_ptr, increment_strong_count, from_raw}`（官方文档给出的
`as_ptr` → `increment_strong_count` → `from_raw` 三元组用法）。

**`ulid` / `EntityId`**：`yeban_model::EntityId` 是 `Copy + Ord + Hash`（16 字节 ULID），
因此所有 SPSC 载荷都能是 `Copy`（`rtrb` 批量 API 的前提）。

### 1.3 ⚠️ 根 `[workspace.dependencies]` 的纯 path 登记与 `deny.toml` 冲突（**发现**）

- 根把内部 crate 登记为**纯 path**（`yeban-model = { path = "crates/yeban-model" }`，**无 `version`**）；
- Cargo 因此给成员依赖解析出 `req = "*"`；
- `deny.toml` 的 `[bans] wildcards = "deny"` 会把 `*` 判为 **MUST-GATE-004 失败**。

本工作线是**第一个**跨成员依赖（`crates/yeban-engine/Cargo.toml` 是唯一出现
`yeban-*.workspace` 的地方），因此这个冲突第一次暴露。实测：

```text
error[wildcard]: found 2 wildcard dependencies for crate 'yeban-engine'
   ┌─ crates/yeban-engine/Cargo.toml:28:25   yeban-model.workspace = true
   ┌─ Cargo.toml:87:16                       yeban-engine = { path = "crates/yeban-engine" }
```

**本线的处置**（根 `Cargo.toml` / `deny.toml` 都属于集成者独占，不得改）：

```toml
yeban-model = { path = "../yeban-model", version = "0.0.1" }   # 显式版本, 无通配
```

本地实测 `cargo-deny --all-features check` 因此变为 **exit 0**（只有既有的
`multiple-versions = "warn"` 重复版本告警；cpal 引入了约 10 组新的重复版本，全部是 warn）。

**TODO(hoist)**：请集成者在根 `[workspace.dependencies]` 给内部 crate 补上
`version = "0.0.1"`（一行），本行即可还原为 `yeban-model.workspace = true`。
这是**规范/基础设施层面的裁决点**，不由本工作线单独决定。

---

## 2. 落地清单（文件 → 规范 ID）

| 文件 | 内容 | 规范 ID |
| :--- | :--- | :--- |
| `src/block.rs` | `AudioBlock<FRAMES>`（栈上 `[f32; 128]` × 2）、`ScratchBuffer<N>`、编译期块长断言 | [ARCH-DET-001] [ARCH-RT-001] [ROAD-M2-007] |
| `src/fpu.rs` | `enable_ftz_daz` / `disable_ftz_daz` / `ftz_daz_enabled`（x86 MXCSR bit15+bit6、aarch64 FPCR bit24、其它架构安全空实现） | [ARCH-RT-003] [ROAD-M2-003] |
| `src/graph.rs` | `PdcPlan::compute`（Kahn 拓扑排序 + 关键路径 + `D_i` 分配）、`LatencyTable`、`DelayLine`、`CompensationBank` | [ARCH-PDC-001] [ARCH-PDC-002] [ARCH-DET-002] [ROAD-M2-004] |
| `src/ring.rs` | `event_channel`（`EventSender`/`EventReceiver`）、`EngineEvent`/`ParamAddress`/`TransportCommand`，批量契约计数器 | [ARCH-RT-001] [ROAD-M2-007] |
| `src/snapshot.rs` | `EngineSnapshot`（不可变投影）、`SnapshotSlot`（原子指针 + 锚 + 纪元握手）、`SnapshotReader`、`RetireQueue`、`retire_channel` | [ARCH-RT-002] [ROAD-M2-002] [ARCH-TOP-002] |
| `src/meter.rs` | `meter_channel`（高容量、有损）、`MeterFrame::measure`、`MeterBoard`（UI 侧最新值面板） | [ARCH-UI-002] [ROAD-M2-008] |
| `src/rt.rs` | `EngineRuntime::process_quantum`（**不依赖 cpal** 的回调逻辑）、`EngineStats` | [ARCH-TOP-002] [ARCH-RT-001] [ARCH-RT-003] |
| `src/device.rs` | `negotiate`（纯函数能力协商）、`open_output`、`OutputStreamHandle`、`NullBackend`、`enumerate_output_devices` | [ROAD-M2-001] [ARCH-TOP-002] |

---

## 3. 判据（每条都说明"怎么变红"）

判据代码全部是 `#[cfg(test)]` 内联单测，由 CI 的
`cargo clippy -p yeban-engine --all-targets -- -D warnings` + `cargo test -p yeban-engine` 执行。
**本机（M2）不编译重依赖，因此这些测试的记录一律是"交给 CI 判定"** —— 见 §5 的执行记录。

| # | 判据 | 测试名（文件） | 注入什么会让它变红 |
| :-: | :--- | :--- | :--- |
| a1 | **退役队列不泄漏**：旧快照经主线程 `drain` 后强引用归零 | `retired_snapshots_are_dropped_after_drain_and_reference_count_hits_zero`（`snapshot.rs`） | 把 `RetireQueue::drain` 改成"出队但不 Drop"（例如 `let _ = chunk.into_iter().count()` 保留元素）→ `Weak::upgrade()` 仍为 `Some` |
| a2 | **音频线程零释放**：退役队列满时旧快照被寄存而不是被 Drop（主线程 drain 前所有旧快照都还活着） | `full_retire_queue_never_frees_on_the_audio_thread`（`snapshot.rs`） | 把 `retire_or_stash` 的 `Full` 分支改成 `drop(arc)` → "drain 之前全部存活"的断言立刻红 |
| a3 | **写者侧清单不泄漏**：读者推进纪元后 `prune` 必须清空 `pending` | `writer_pending_list_is_pruned_after_reader_progress`、`dropping_the_reader_releases_writer_side_backlog`（`snapshot.rs`） | 把 `prune` 的 `retain` 条件改成恒 `true`（即从不释放）→ `pending_len()` 不降；或改成恒 `false`（提前释放）→ 需要读者在册的断言红 |
| b | **成环图返回明确错误而不是死循环** | `cyclic_graph_returns_error_and_never_hangs`（`graph.rs`） | 把 Kahn 的"剩余节点"分支删掉改成 `debug_assert!` 后继续 → 返回 `Ok` 而不是 `Cycle`；或改成 `loop {}` → 测试挂死（超时） |
| c1 | **PDC 定义性不变量**：`∀v ∈ Reachable(master): arrival(v) + D(v) == L_max` | `pdc_alignment_invariant_holds_for_diamond`（`graph.rs`） | 把 `D(v)` 写成 `L_max - L(v)`（漏掉节点自身延迟）→ 长支路 `D(long)` 变成 20 而不是 0 |
| c2 | **PDC 行为面**：两条并联支路的脉冲在**同一采样点**到达 | `compensated_branches_line_up_sample_exactly`（`graph.rs`） | 把 `DelayLine` 的 `read` 初始偏移从 `capacity - delay` 改成 `delay` → 延迟线方向反转，两条支路错开 |
| d1 | **事件 SPSC 批量契约**：每块**恰好一次**批量 API（结构性，不依赖墙钟） | `bulk_contract_is_exactly_one_pop_call_per_block`（`ring.rs`） | 把 `drain_with` 改成 `while let Ok(e) = consumer.pop()` 逐条循环 → `bulk_pop_calls` 变成 1 但 `events_drained` 只增 1（或按实现循环 120 次）→ 断言红 |
| d2 | **电平 SPSC 批量契约**：音频每量子一次批量写、UI 每 tick 一次批量读 | `meter_bulk_contract_is_one_call_per_quantum_and_per_ui_tick`（`meter.rs`） | 同上，改成逐条 push/pop |
| e1 | **FTZ/DAZ 开关不 panic 且幂等**（全架构） | `enable_ftz_daz_never_panics_and_is_idempotent`（`fpu.rs`） | 把 `enable` 改成 `unimplemented!()` 或让它对未知架构 panic |
| e2 | **支持的架构上确实置位且可逆** | `ftz_daz_bits_are_really_set_on_supported_arches`（`fpu.rs`，`cfg(x86_64/aarch64)`） | 把 `_mm_setcsr(csr \| FTZ \| DAZ)` 改成 `_mm_setcsr(csr)` → 读回 `Some(false)`；把清位改成只清 FTZ 不清 DAZ → `enabled()` 读回 `Some(false)` 失败 |
| f | **无设备环境下开流允许失败但绝不 panic** | `opening_the_default_output_device_never_panics`（`device.rs`） | 把 `ok_or(NoDefaultOutputDevice)?` 改成 `.unwrap()` → CI 上 panic |
| g | **不打开真实设备也能覆盖完整回调路径** | `null_backend_drives_the_same_render_path_without_a_device` + `rt.rs` 的 5 条测试 | 让 `NullBackend` 绕过 `EngineRuntime`（例如直接填静音）→ `quanta`/`event_bulk_pops`/`meter_frames` 计数断言红 |
| h1 | **能力协商可在无设备环境单测** | `negotiate_*` 4 条（`device.rs`） | 把协商里的"退回标准采样率"分支删掉 → `negotiate_falls_back_...` 红 |
| h2 | **要求独占模式时明确失败而不是假装成功** | `requiring_exclusive_mode_fails_explicitly_instead_of_pretending`（`device.rs`） | 把 `RequireExclusive` 分支删掉 → 返回 `Ok` |

> **关于"耗时 < 0.05ms"**：任务书允许在无法稳定测量时改用结构性断言。本机是 M2 开发机且
> **明文禁止在本机跑重依赖编译/测试**，墙钟断言无法在本机反复标定，写在 CI 上也会因
> runner 抖动而变成脆弱判据。因此判据 (d1)/(d2) 改为**结构性断言**："每块只调用一次批量 API"
> —— 用 `bulk_pop_calls == 块数` 而不是时间来钉住批量契约。
> 这正是任务书 §3 给出的备选路径，在此如实记录。

---

## 4. 边界 / needs / pending

### 4.1 明确**没有**做到的（不要误读）

1. **不能发声**：`process_quantum` 只做"事件出队 → 快照切换 → 清空输出块 → 电平上报"。
   声部合成与通道条要等 `yeban-sfz` / `yeban-dsp` 的后续切片接入。
   测试里"输出全零"是**设计事实**，不是"引擎工作正常"的证据。
2. **实时线程优先级 [ROAD-M2-001] 未实现**。实测 cpal 0.18 的 `realtime` feature
   只在 `wasapi` / `aaudio` / `pipewire` / `jack` 的 host 代码里出现
   （`grep -rn 'feature = "realtime"' cpal-0.18.2/src/`），
   **macOS 与 Linux-ALSA 路径没有对应开关**。自己实现需要 `pthread_setschedparam`
   （新 `libc` 依赖）或 macOS 的 `thread_policy_set`（新依赖 + 平台 `unsafe`）。
3. **独占模式**：cpal 0.18 没有 API。`ShareMode::RequireExclusive` 返回明确错误，
   `PreferExclusive` 降级为共享并**如实记录**在 `NegotiatedConfig::share_mode`。
4. **只支持 `f32`**：协商到非 `f32` 一律 `Err(UnsupportedSampleFormat)`，
   不做静默格式转换（新样本路径会破坏 [ARCH-DET-001] 的确定性论证）。
5. **只支持单个读者**：`SnapshotReader` 的安全证明依赖"单线程顺序执行块"（§`snapshot.rs` 模块文档）。
   多读者需要真正的 hazard pointer 数组。
6. **`unsafe` 未做形式化验证**：快照回收的纪元握手是纸面推导
   （Acquire/Release + 单读者顺序 + read-read 一致性），没有跑 `Miri`/`loom`。
   引入 `loom` 会扩大依赖图，属于需要裁决的动作。
7. **`fpu` 的 32 位 `x86` 分支**在 CI 上没有对应 target，因此只保证"能通过语法/名称解析"，
   没有实测。CI 的 `ubuntu-latest` 是 `x86_64`。

### 4.2 needs（需要人类/集成者裁决）

| ID | 内容 | 阻塞点 |
| :--- | :--- | :--- |
| N1 | 根 `[workspace.dependencies]` 给内部 crate 补 `version = "0.0.1"`（§1.3） | 根文件由集成者独占；不补则我的 `version = "0.0.1"` 是唯一合规写法 |
| N2 | `[ARCH-PDC-001]` 要求 `DeviceDefinition::latency_samples`，但 `yeban-model` **没有**该字段 | 模型层由 `line/model-core` 拥有；本线只能提供 `LatencyTable` 显式入口，`from_tracks` 暂时全零 |
| N3 | [ROAD-M2-001] RT 优先级方案（`realtime` feature? 新 `libc` 依赖? 平台分支?） | 依赖图裁决 + 平台 `unsafe` 审计 |
| N4 | 是否允许引入 `loom`（或 `miri` 流程）来机械化验证 §`snapshot.rs` 的无锁回收 | 新依赖 = 根清单改动 |
| N5 | 独占模式（WASAPI Exclusive）要不要引 `wasapi` crate（规范 §3.1 提到） | 依赖图裁决 |

### 4.3 pending（后续切片）

- `src/graph.rs`：把 `CompensationBank` 接进 `EngineRuntime::render_block` 的实际混音路径
  （现在只构造、只在测试里用）。
- `src/rt.rs`：把 `EngineEvent::SetParam` / `NoteOn` / `NoteOff` / `Transport` 接到实际渲染
  （现在是计数占位）；接一阶低通参数平滑（τ≈5ms，[ARCH-DSP-001]）。
- `src/meter.rs`：`MeterBoard` 接 Slint Property 更新（属于 `yeban-app` 的地盘）。
- `src/snapshot.rs`：`PdcPlan` 的拓扑序在 `render_block` 里尚未用于"按序串行归约"
  （[ARCH-DET-002] 的落地要等混音真接入）。

---

## 5. 本机执行记录（**只记录真的跑过的**）

| 命令 | 结果 | 备注 |
| :--- | :--- | :--- |
| `bash scripts/dev/cargo-local.sh fmt --all` | ✅ | 语法解析通过（rustfmt 会拒绝非法语法） |
| `bash scripts/gates/run-gates.sh light` | ✅（提交前重跑） | fmt + 12 条守卫 + 文档门禁 + 许可清单对账；**文档门禁在本工作树里是空跑**，见下面的缺口 2 |
| `cargo metadata --format-version 1`（经 `local-env.sh`） | ✅ | 重新生成 `Cargo.lock`（+153 行，无其它改动） |
| `python3 scripts/gates/license_inventory.py` | ✅ | 重新生成清单（外部包 579 → 591） |
| `python3 scripts/gates/license_inventory.py --check` | ✅ | 与依赖图一致（648 行） |
| `cargo-deny --all-features check`（预编译二进制 0.20.2） | ✅ exit 0 | 只有既有的 `multiple-versions = warn` 告警 |
| `cargo clippy -p yeban-engine` / `cargo test -p yeban-engine` | ⛔ **未跑** | 明文禁止：`cpal` 是重依赖，本机不编译（AGENTS.md §5） |

### ⚠️ 门禁缺口（**发现，需要集成者修 `scripts/**`**）

`scripts/gates/run-gates.sh` 的重依赖检测正则要求依赖名后面**紧跟 `=`**：

```bash
HEAVY_RE='^[[:space:]]*(...|cpal|...)[[:space:]]*='
```

而工作区继承写法是 `cpal.workspace = true`（`cpal` 后面是 `.` 而不是 `=`），
**正则匹配不到**。实测：

```bash
$ grep -E '^[[:space:]]*(slint|...|cpal|...)[[:space:]]*=' crates/yeban-engine/Cargo.toml
$ echo $?
1     # 无匹配 ⇒ heavy_deps_of() 返回空 ⇒ gate_crate 不会 SKIP
```

后果：本机跑 `run-gates.sh crate yeban-engine` **不会**像设计预期那样跳过，
而会在 M2 上真的去编译 cpal（正是用户硬性禁止的事）。
本线因此**没有**执行该命令。建议把正则放宽为同时接受 `cpal = …` 与 `cpal.workspace = …`
（例如 `^[[:space:]]*(name)([[:space:]]*=|\.)`）—— 但 `scripts/**` 属于集成者独占，
本线只报告、不修改。

### ⚠️ 门禁缺口 2（**发现，需要集成者修 `scripts/**`**）

`scripts/gates/check_docs_links.py` 的 `markdown_files()` 把 `.worktrees` 放进了 `SKIP_DIRS`：

```python
SKIP_DIRS = {".git", "target", ..., ".worktrees", ...}
def markdown_files():
    for path in REPO.rglob("*.md"):
        if any(part in SKIP_DIRS for part in path.parts):   # ← 相对路径判断用了**绝对**路径分量
            continue
```

但在工作树里 `REPO` 本身就是 `/Users/crow/work/music/yeban/.worktrees/engine-rt`，
于是 `path.parts` 里必然含 `.worktrees` —— **所有** markdown 文件都被跳过。实测输出：

```text
note: 扫描 0 个 markdown 文件，检查 0 个相对链接
```

后果：本机跑 `run-gates.sh light` 时"文档链接门禁"是**空跑**（一条永远不会红的判据），
只有集成者合并到 main 之后才会真正检查。本线的对策是**手工**核验：
`docs/ledger/engine-rt-notes.md` 里**没有**任何 `](...)` 相对链接（全用反引号写路径），
因此不存在链接腐烂风险。建议把 `SKIP_DIRS` 的判断改成"相对 `REPO` 的路径分量"。
同上，`scripts/**` 属于集成者独占，本线只报告、不修改。

---

## 6. 复用来源（Reuse provenance）

本 crate 的代码是**新写**的：

- `graph.rs` 的 PDC 算法按规范 §3.4 的文字实现（拓扑排序 + 环形延迟线是教科书算法）；
- `block.rs` 的接口形状参考了兄弟 crate `crates/yeban-dsp/src/block.rs`（同一仓库、同一作者、
  同一许可），但**没有复制任何函数体**：那边是标量混音原语，这边是容器与块长契约；
- `device.rs` / `snapshot.rs` 的 cpal / rtrb 用法全部来自 §1.2 核验过的官方 API 文档与源码。

因此**不新增** `THIRD_PARTY_LICENSES.md` 条目。
