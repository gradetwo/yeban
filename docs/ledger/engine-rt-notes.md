# `yeban-engine` API 核验台账、PDC 公共契约、判据与待办（实时引擎工作线）

- **台账类型**：API 事实核验 / 公共契约 / 判据清单 / 未决项（**不是规范**）
- **记录时刻**：2026-10-05（第 2 轮；对应 CI run 37221166111 的红点复盘 + 集成者 ADR-0001 D19/D21 落地）
- **工作线**：`line/engine-rt`（worktree `yeban/.worktrees/engine-rt`）
- **所有者目录**：`crates/yeban-engine/**`（本台账是唯一新增的文档文件）
- **落地范围**：`block` / `fpu` / `graph` / `ring` / `snapshot` / `meter` / `rt` / `device` 八个模块
- **规范来源**：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3、§0.1；
  `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M2-001..004`、`ROAD-M2-007..008`

> 本文件回答五个问题：**我用到的 API 到底是什么签名（以及从哪读来的）**、
> **PDC 模块的公共契约是什么（供 `yeban-render` 机械替换）**、
> **每条判据怎么变红**、**哪些东西明确没做**、**需要谁裁决什么**。

---

## 1. 依赖与 API 事实核验（**先查证再写代码**）

### 1.1 依赖登记与特性切分

| 依赖 | 版本 | 来源 | 备注 |
| :--- | :--- | :--- | :--- |
| `cpal` | `0.18.2` | 根登记（`default-features = false`） | **optional**，由 feature `device` 打开 |
| `rtrb` | `0.4.0` | 根登记（`default-features = false`） | 无条件 |
| `thiserror` | `2.0.21` | 根登记 | 无条件 |
| `yeban-model` | `0.0.1`（path） | 根登记 | `workspace = true` |

**没有新增任何未登记依赖，因此没有需要 hoist 的新 crate。**

按 [ADR-0001 D19]，设备 I/O 与 PDC 算法在**cargo feature 层面**分开：

```toml
[features]
default = ["device"]
device = ["dep:cpal"]
```

| 构建 | 编译进产物的模块 | 用途 |
| :--- | :--- | :--- |
| `yeban-engine`（默认） | 全部（含 `device`） | 实时引擎 |
| `yeban-engine` + `default-features = false` | `block` `fpu` `graph` `ring` `snapshot` `meter` `rt` | `yeban-render` 离线渲染复用同一份 PDC 算法，**不编译 cpal** |

代码级门控只有两处：`#[cfg(feature = "device")] pub mod device;`（`lib.rs`）与
`rt.rs` 里那条断言 `NullBackend: Send` 的测试。`graph.rs` **零 cpal 引用**（已逐行核验：
除 `device.rs` 外，`cpal` 只出现在文档正文里）。

### 1.2 核验过的 API 签名（逐条给出出处）

核验方法：从 crates.io 静态包下载**真实源码**（不是文档站转述），逐行读 `src/**`；
`std` 的原子量与内联汇编语义对照官方文档。

**`rtrb` 0.4.0**（`rtrb-0.4.0/src/{lib.rs,chunks.rs}`，
crates.io API `https://crates.io/api/v1/crates/rtrb/0.4.0`，
文档 `https://docs.rs/rtrb/0.4.0/rtrb/`）：

| 项 | 签名（源码原文） | 用途 |
| :--- | :--- | :--- |
| `Producer::push` | `pub fn push(&mut self, value: T) -> Result<(), PushError<T>>` | 退役队列（`Arc` 不是 `Copy`） |
| `Producer::push_partial_slice` | `pub fn push_partial_slice<'a>(&mut self, slice: &'a [T]) -> (&'a [T], &'a [T]) where T: Copy` | 事件/电平**批量写** |
| `Producer::slots` / `is_full` / `is_abandoned` / `buffer` | `-> usize` / `bool` / `bool` / `&RingBuffer<T>` | 背压与容量 |
| `Consumer::pop_partial_slice` | `pub fn pop_partial_slice<'a>(&mut self, slice: &'a mut [T]) -> (&'a mut [T], &'a mut [T]) where T: Copy` | **每块一次**批量读 |
| `Consumer::read_chunk` | `pub fn read_chunk(&mut self, n: usize) -> Result<ReadChunk<'_, T>, ChunkError>` | 退役队列批量出队 |
| `ReadChunk` 的 `IntoIterator` | `type Item = T`；"When the iterator is dropped, all iterated slots are made available for writing again" | move 出 `Arc` 并 Drop |
| `ChunkError` | 唯一变体 `TooFewSlots(usize)` | 批量失败模式 |
| `PushError` | **唯一变体** `Full(T)`（0.4 没有 `Abandoned`） | 满队列时**拿回**元素 |
| `RingBuffer::new(capacity)` | `capacity` **原样**成为 `RingBuffer::capacity()`（源码实测：**没有** 2 的幂取整） | 队列建立 |

**`cpal` 0.18.2**（`cpal-0.18.2/src/{lib.rs,traits.rs,error.rs,sample_format.rs,platform/mod.rs}`，
crates.io API `https://crates.io/api/v1/crates/cpal/0.18.2`，
文档 `https://docs.rs/cpal/0.18.2/cpal/`）：

> ⚠️ **0.18 与 0.15/0.16 的 API 有实质差异**，凭记忆写会编译失败。实测差异如下。

| 项 | 0.18.2 签名 | 与旧版的差异 |
| :--- | :--- | :--- |
| `DeviceTrait::build_output_stream` | `fn build_output_stream<T, D, E>(&self, config: StreamConfig, data_callback: D, error_callback: E, timeout: Option<Duration>) -> Result<Self::Stream, Error>`，`D: FnMut(&mut [T], &OutputCallbackInfo) + Send + 'static` | **多了 `timeout` 参数**；`config` 按值传入 |
| `DeviceTrait` 超 trait | `PartialEq + Eq + Hash + Debug + Display`，新增必填 `description()` 与 `id()` | 旧版没有；**也没有 `name()`**，设备名走 `Display` |
| `HostTrait::devices` | `fn devices(&self) -> Result<Self::Devices, Error>` | 由 `Iterator` 变成 `Result<Iterator>` |
| `HostTrait::default_output_device` | `-> Option<Self::Device>` | 一致 |
| `SupportedStreamConfigRange` | `pub fn new(channels, min_sample_rate, max_sample_rate, buffer_size, sample_format)` + 同名 getter + `try_with_standard_sample_rate` | **`new`/getter 都是 `pub`** ⇒ 能力协商可脱离真实设备单测（`device::negotiate` 据此设计） |
| `StreamConfig` / `BufferSize` / `SupportedBufferSize` | `{channels, sample_rate, buffer_size}` / `Default \| Fixed(u32)` / `Range{min,max} \| Unknown` | 一致 |
| `SampleFormat` | `#[non_exhaustive]` 枚举（`F32, F64, I16, …`） | 任何 `match` 必须带通配分支 |
| `cpal::Error` | 结构体（`ErrorKind` + `Option<Cow<'static, str>>`），实现 `Display + std::error::Error` | 旧版是枚举 |
| `StreamTrait::play/pause` | `&self` | 建流后**默认停止**，必须显式 `play()` |
| `realtime` feature | `#[cfg(feature = "realtime")]` 只出现在 `wasapi` / `aaudio` / `pipewire` / `jack` 的 host 代码里；**macOS 与 Linux-ALSA 无开关** | 见 §5.2 needs |

**`std` 内联汇编与原子量**（`https://doc.rust-lang.org/reference/inline-assembly.html`、
`https://doc.rust-lang.org/std/sync/atomic/`）：
`AtomicPtr::{new, load, store}`、`AtomicU64::{load, store, fetch_add, fetch_max}`、
`AtomicBool::{load, store}`；`Ordering` 用 `Acquire`/`Release`/`AcqRel`。
`Arc::{as_ptr, increment_strong_count, from_raw}` 用的是官方文档给出的
`as_ptr` → `increment_strong_count` → `from_raw` 三元组。

### 1.3 🚨 第 1 轮 CI 学到的两条硬事实（**都已写进代码注释**）

**(a) `core::arch::x86_64::_mm_getcsr` / `_mm_setcsr` 自 Rust 1.75 起 deprecated。**
第 1 轮 CI 在 `fpu.rs` 报了 5 条 `use of deprecated function ... use inline assembly instead`，
在 `-D warnings` 下直接失败。教训：**"官方推荐用法"也是 API 事实，必须核验** ——
当时我对着 docs.rs 读了签名，却没有读 deprecation 属性。
现改为内联汇编 `stmxcsr [mem]` / `ldmxcsr [mem]`（与 stdarch 的提示一致），
并且刻意**不给 `ldmxcsr` 标 `nomem`/`readonly`**：该汇编没有输出操作数，
若声明成"无内存副作用"，LLVM 有权把设置 FTZ/DAZ 的指令当死代码删掉。

**(b) `std::mem::replace(&mut opt, Some(x))` 会被 `clippy::mem_replace_option_with_some` 拒绝**
（`clippy::all` 属于工作区 deny 组）。改为 `opt.replace(x)`。

另有一条 `unused import`：`rt.rs` 里 `yeban_model::EntityId` 只在测试里用，已把导入移进 `mod tests`。

### 1.4 根 `[workspace.dependencies]` 的通配版本：本线撞上、集成者已按 D21 放行

历史（保留证据，避免后人重踩）：根把内部 crate 登记为**纯 path**（无 `version`），
Cargo 因此给成员依赖解析出 `req = "*"`，被 `deny.toml` 的 `[bans] wildcards = "deny"` 判为
MUST-GATE-004 失败。本线是**第一个**跨成员依赖，第 1 轮实测：

```text
error[wildcard]: found 2 wildcard dependencies for crate 'yeban-engine'
   ┌─ crates/yeban-engine/Cargo.toml:28:25   yeban-model.workspace = true
   ┌─ Cargo.toml:87:16                       yeban-engine = { path = "crates/yeban-engine" }
```

当时的临时处置是写 `yeban-model = { path = "../yeban-model", version = "0.0.1" }`。
**集成者随后在 main 按 ADR-0001 D21 加了 `allow-wildcard-paths = true`（只放行带 `path` 的 `*`），
本线已还原为 `yeban-model.workspace = true`，临时版本号已删除。**

> ⚠️ **给 `line/render-master` 的提醒**：D21 之前另有一条绕过写法是
> `yeban-engine = { path = "../yeban-engine", version = "0.0.1" }`。D21 之后**不要**再那样写 ——
> D21 明确否决了"给内部条目补 version"（`^0.0.1` 不匹配 `0.1.0`，升版会连锁改一堆内部依赖）。
> 正确写法就是 `yeban-engine = { workspace = true, default-features = false }`。

---

## 2. PDC 公共契约（**给 `yeban-render` 机械替换用的精确签名**）

模块路径：`yeban_engine::graph`（**无条件编译**，不引用 cpal；`default-features = false` 时可用）。
类型都实现 `Clone + Debug`（`PdcError` 另有 `PartialEq + Eq`；`LatencyTable`/`PdcPlan` 另有
`Default + PartialEq + Eq`）。`EntityId` / `RoutingGraph` / `TrackV3` / `YebanProjectV1` 都来自
`yeban_model`（渲染线已有该依赖）。

```rust
pub enum PdcError {
    UnknownMaster { master: EntityId },
    DanglingEdge  { node: EntityId },
    Cycle         { nodes: Vec<EntityId> },   // 按键升序, 确定性
}

pub struct LatencyTable;                       // Default + Clone + Debug + PartialEq + Eq
impl LatencyTable {
    pub fn new() -> Self;
    pub fn set(&mut self, node: EntityId, latency_samples: u32);
    pub fn get(&self, node: &EntityId) -> u32;          // 未登记 = 0
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn iter(&self) -> impl Iterator<Item = (&EntityId, &u32)>;
    pub fn from_project(project: &YebanProjectV1) -> Self;
    pub fn from_tracks(tracks: &BTreeMap<EntityId, TrackV3>) -> Self;
}

pub struct PdcPlan;                            // Default + Clone + Debug + PartialEq + Eq
impl PdcPlan {
    pub fn compute(graph: &RoutingGraph, master: EntityId, latencies: &LatencyTable)
        -> Result<Self, PdcError>;
    pub fn order(&self) -> &[EntityId];                          // 全图拓扑序（确定性）
    pub fn latency(&self, node: &EntityId) -> Option<u32>;        // L(v)
    pub fn arrival(&self, node: &EntityId) -> Option<u32>;        // L(v)+own(v)；不可达 master 时 None
    pub fn compensation(&self, node: &EntityId) -> Option<u32>;   // D(v)=L_max-arrival(v)；不可达时 None
    pub fn total_latency(&self) -> u32;                          // L_max
    pub fn excluded(&self) -> &[EntityId];                       // 不可达 master 的节点（按键升序）
    pub fn compensated_len(&self) -> usize;
}

pub struct DelayLine;                          // Clone + Debug
impl DelayLine {
    pub fn new(max_delay_samples: usize) -> Self;          // 预分配 max+1 格
    pub fn capacity(&self) -> usize;
    pub fn delay(&self) -> usize;
    pub fn set_delay(&mut self, samples: usize) -> usize;  // 钳到 capacity-1，不 panic
    pub fn reset(&mut self);
    pub fn is_passthrough(&self) -> bool;                  // delay == 0
    pub fn process_in_place(&mut self, buf: &mut [f32]);
    pub fn process(&mut self, input: &[f32], output: &mut [f32]);   // 按较短者工作
}

pub struct CompensationBank;                   // Clone + Debug
impl CompensationBank {
    pub fn from_plan(plan: &PdcPlan) -> Self;              // 构造期分配，处理期零分配
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn max_capacity(&self) -> usize;
    pub fn apply(&mut self, node: &EntityId, buf: &mut [f32]) -> bool;   // 无该节点时 false
    pub fn line(&self, node: &EntityId) -> Option<&DelayLine>;
}
```

**语义与不变量（替换时必须保持）**：

1. `L(v) = max over 前驱 p of arrival(p)`，`arrival(v) = L(v) + own_latency(v)`，
   `L_max = arrival(master)`；
2. **定义性不变量**：`∀ v ∈ Reachable(master): arrival(v) + D(v) == L_max`，
   其中 `D(v) = L_max - arrival(v)`；
3. 不可达 master 的节点进 `excluded()`，**没有**补偿语义（`arrival`/`compensation` 返回 `None`）；
4. 成环 ⇒ `Err(PdcError::Cycle)`，**不死循环**；
5. 节点自身延迟**唯一来源**：`DeviceDefinition::latency_samples`
   （未旁通设备饱和求和；`0` = 未上报，不做推断）。

---

## 3. 落地清单（文件 → 规范 ID → feature）

| 文件 | 内容 | 规范 ID | feature |
| :--- | :--- | :--- | :--- |
| `src/block.rs` | `AudioBlock<FRAMES>`（栈上 `[f32;128]` × 2）、`ScratchBuffer<N>`、编译期块长断言 | [ARCH-DET-001] [ARCH-RT-001] [ROAD-M2-007] | 无条件 |
| `src/fpu.rs` | `enable_ftz_daz` / `disable_ftz_daz` / `ftz_daz_enabled`（x86 内联汇编 MXCSR bit15+bit6、aarch64 FPCR bit24、其它架构安全空实现） | [ARCH-RT-003] [ROAD-M2-003] | 无条件 |
| `src/graph.rs` | `PdcPlan::compute`（Kahn + 关键路径 + `D_i`）、`LatencyTable`、`DelayLine`、`CompensationBank` | [ARCH-PDC-001] [ARCH-PDC-002] [ARCH-DET-002] [ROAD-M2-004] | 无条件（D19 要求） |
| `src/ring.rs` | `event_channel`、`EngineEvent`/`ParamAddress`/`TransportCommand`、批量契约计数器 | [ARCH-RT-001] [ROAD-M2-007] | 无条件 |
| `src/snapshot.rs` | `EngineSnapshot`、`SnapshotSlot`（原子指针 + 锚 + 纪元握手）、`SnapshotReader`、`RetireQueue` | [ARCH-RT-002] [ROAD-M2-002] [ARCH-TOP-002] | 无条件 |
| `src/meter.rs` | `meter_channel`（高容量有损）、`MeterFrame::measure`、`MeterBoard` | [ARCH-UI-002] [ROAD-M2-008] | 无条件 |
| `src/rt.rs` | `EngineRuntime::process_quantum`（**不依赖 cpal**）、`EngineStats` | [ARCH-TOP-002] [ARCH-RT-001] [ARCH-RT-003] | 无条件 |
| `src/device.rs` | `negotiate`（纯函数协商）、`open_output`、`OutputStreamHandle`、`NullBackend` | [ROAD-M2-001] [ARCH-TOP-002] | `device` |

---

## 4. 判据（每条都说明"怎么变红"）

判据都是 `#[cfg(test)]` 内联单测，由 CI 的
`cargo clippy -p yeban-engine --all-targets -- -D warnings` + `cargo test -p yeban-engine` 执行。
**本机（M2）不编译重依赖，因此这些测试一律"交给 CI 判定"**（§6 有本机真实执行记录）。

| # | 判据 | 测试名（文件） | 注入什么会让它变红 |
| :-: | :--- | :--- | :--- |
| a1 | **退役队列不泄漏**：`drain` 之后强引用归零 | `retired_snapshots_are_dropped_after_drain_and_reference_count_hits_zero`（`snapshot.rs`） | `drain` 改成"出队但不 Drop"（例如只 `count()` 不消费）⇒ `Weak::upgrade()` 仍为 `Some` |
| a2 | **音频线程零释放**：队列满时只寄存不 Drop（drain 前全部存活） | `full_retire_queue_never_frees_on_the_audio_thread`（`snapshot.rs`） | `retire_or_stash` 的 `Full` 分支改成 `drop(arc)` ⇒ 断言红 |
| a3 | **写者侧清单归零** | `writer_pending_list_is_pruned_after_reader_progress`、`dropping_the_reader_releases_writer_side_backlog` | `prune` 的 `retain` 恒真（从不释放）⇒ `pending_len()` 不降；恒假 ⇒ "读者在册时不许释放"断言红 |
| b | **成环返回明确错误，不死循环** | `cyclic_graph_returns_error_and_never_hangs`（`graph.rs`） | 删掉未归约分支 ⇒ 返回 `Ok`；改成 `loop {}` ⇒ 挂死（超时） |
| c1 | **PDC 定义性不变量** `arrival(v)+D(v)==L_max` | `pdc_alignment_invariant_holds_for_diamond`（`graph.rs`） | `D(v)` 写成 `L_max - L(v)`（漏掉节点自身延迟）⇒ 长支路 `D` 变 20 |
| c2 | **PDC 行为面**：并联支路脉冲落在**同一采样点** | `compensated_branches_line_up_sample_exactly`（`graph.rs`） | 环形延迟线的 `read` 初始偏移由 `capacity - delay` 改成 `delay` ⇒ 方向反转、错开 |
| d1 | **事件 SPSC 批量契约**：每块**恰好一次**批量 API（结构性） | `bulk_contract_is_exactly_one_pop_call_per_block`（`ring.rs`） | 改成 `while let Ok(e) = pop()` 逐条循环 ⇒ `bulk_pop_calls` 与块数不再相等 |
| d2 | **电平 SPSC 批量契约** | `meter_bulk_contract_is_one_call_per_quantum_and_per_ui_tick` | 同上 |
| e1 | **FTZ/DAZ 开关不 panic 且幂等**（全架构） | `enable_ftz_daz_never_panics_and_is_idempotent`（`fpu.rs`） | 让它对未知架构 panic |
| e2 | **支持的架构上确实置位且可逆** | `ftz_daz_bits_are_really_set_on_supported_arches`（`fpu.rs`，x86_64/aarch64） | 汇编改成 `write_mxcsr(csr)`（不置位）⇒ 读回 `false`；这条同时是"`ldmxcsr` 是否被当死代码删掉"的判据 |
| f | **无设备环境下开流允许失败但绝不 panic** | `opening_the_default_output_device_never_panics`（`device.rs`） | `ok_or(...)?` 改成 `unwrap()` ⇒ CI 上 panic |
| g | **不打开真实设备也能覆盖完整回调路径** | `null_backend_drives_the_same_render_path_without_a_device` + `rt.rs` 的 6 条 | 让 `NullBackend` 绕过 `EngineRuntime` ⇒ `quanta`/`event_bulk_pops`/`meter_frames` 计数断言红 |
| h1 | **能力协商可无设备单测** | `negotiate_*` 4 条（`device.rs`） | 删掉"退回标准采样率"分支 ⇒ `negotiate_falls_back_...` 红 |
| h2 | **要求独占模式时明确失败** | `requiring_exclusive_mode_fails_explicitly_instead_of_pretending` | 删掉 `RequireExclusive` 分支 ⇒ 返回 `Ok` |
| i1 | **延迟唯一来源是模型的 `DeviceDefinition::latency_samples`** | `pdc_reads_device_latency_samples_from_the_model`（`snapshot.rs`） | `LatencyTable::from_project` 改成返回空表 ⇒ `L_max` 变 0、`compensation(&light)` 变 `Some(0)` |
| i2 | **旁通设备不计入延迟** | `bypassed_devices_do_not_contribute_latency`（`snapshot.rs`）、`latency_table_sums_model_device_latency_and_skips_bypassed`（`graph.rs`） | 去掉 `if !device.bypassed` ⇒ 延迟变 4168 / 64 |
| j | **`EngineRuntime: Send`**（编译期；cpal 回调闭包要求 `D: Send + 'static`） | `runtime_is_send_so_it_can_move_into_the_audio_callback`（`rt.rs`） | 把 `held_addr: usize` 改回裸指针字段 `*const EngineSnapshot` ⇒ 编译失败（裸指针 `!Send`） |

> **关于"出队耗时 < 0.05ms"**：任务书允许在无法稳定测量时改用结构性断言。本机明文禁止
> 重依赖编译/测试，墙钟断言既无法在本机标定、在 CI 上也会因 runner 抖动而脆弱，
> 因此判据 (d1)/(d2) 采用**结构性**形式："每块只调用一次批量 API"
> （`bulk_pop_calls == 块数`）。理由与取舍在此如实记录。

---

## 5. 边界 / needs / pending

### 5.1 明确**没有**做到的（不要误读）

1. **不能发声**：`process_quantum` 只做"事件出队 → 快照切换 → 清空输出块 → 电平上报"。
   测试里"输出全零"是**设计事实**，不是引擎正常的证据。
2. **实时线程优先级 [ROAD-M2-001] 未实现**：cpal 0.18 的 `realtime` feature 只在
   `wasapi`/`aaudio`/`pipewire`/`jack` 出现，macOS 与 Linux-ALSA 路径没有开关。
   自研需要 `pthread_setschedparam`（新 `libc` 依赖）或 macOS `thread_policy_set`。
3. **独占模式**：cpal 0.18 无 API。`RequireExclusive` 返回明确错误；`PreferExclusive`
   降级为共享并如实记录在 `NegotiatedConfig::share_mode`。
4. **只支持 `f32`**：非 `f32` 明确报错，不做静默格式转换（会引入未验证的样本路径）。
5. **只支持单个读者**：快照回收的安全证明依赖"单线程顺序执行块"。
6. **`default-features = false` 时没有 `NullBackend`**（它的配置类型含 cpal 类型）。
   离线侧需要驱动时直接调 `yeban_engine::rt::EngineRuntime::process_quantum` 喂缓冲。
7. **快照回收的 `unsafe` 只有纸面证明**：没跑 `Miri`/`loom`。
8. **32 位 `x86` 分支**在 CI 上没有对应 target，只保证名称解析通过。

### 5.2 needs（需要人类/集成者裁决）

| ID | 内容 | 状态 |
| :--- | :--- | :--- |
| N1 | 根内部 path 依赖的通配豁免 | ✅ **已解决**（ADR-0001 D21 `allow-wildcard-paths = true`）；本线已还原为 `workspace = true` |
| N2 | `ARCH-PDC-001` 的 `DeviceDefinition::latency_samples` | ✅ **已解决**（main `8f40290`）；本线改为从模型读取 |
| N3 | [ROAD-M2-001] RT 优先级方案（`realtime` feature？新 `libc`？平台分支？） | **待裁决**（依赖图 + 平台 `unsafe` 审计） |
| N4 | 是否引入 `loom`/`miri` 流程来机械化验证无锁回收 | **待裁决**（新依赖 = 根清单改动） |
| N5 | 独占模式是否引 `wasapi` crate（规范 §3.1 提到） | **待裁决** |

### 5.3 pending（后续切片）

- `graph::CompensationBank` 尚未接进 `EngineRuntime::render_block` 的实际混音路径
  （现在只构造 + 测试里用）。接入时注意：[ARCH-DET-002] 要求汇合处按 `EntityId`
  字典序**串行**累加，`PdcPlan::order()` 已提供确定性拓扑序。
- `EngineEvent` 的 `SetParam`/`NoteOn`/`NoteOff`/`Transport` 目前只计数，未接渲染；
  接入时配一阶低通参数平滑（τ≈5ms，[ARCH-DSP-001]）。
- `MeterBoard` 未接 Slint Property 更新（属于 `yeban-app` 的地盘）。
- **`yeban-render` 侧那份最小同构 PDC 必须在合并后退役**（ADR-0001 D19 写明由集成者
  保证合并顺序）⇒ 本文件 §2 就是给那次替换用的契约。

---

## 6. 本机与 CI 执行记录（**只记录真的跑过的**）

### 6.1 本机（M2，只做零编译的事）

| 命令 | 结果 | 备注 |
| :--- | :--- | :--- |
| `bash scripts/dev/cargo-local.sh fmt --all` | ✅ | rustfmt 会拒绝非法语法 ⇒ 顺带是语法检查 |
| `bash scripts/gates/run-gates.sh light` | ✅ | fmt + 12 条守卫 + 文档门禁 + 许可清单对账；**文档门禁在本工作树里是空跑**（见 §6.3 缺口 2） |
| `check_docs_links.py` 的 `LINK_RE` **手工**跑在本文件上 | ✅ 0 命中 | 空跑的工作树门禁的替代自查（第 1 轮就是漏了这一步） |
| `cargo metadata --format-version 1`（经 `local-env.sh`） | ✅ | 重新生成 `Cargo.lock` |
| `python3 scripts/gates/license_inventory.py` / `--check` | ✅ | 外部依赖包数 579 → 591，与依赖图一致 |
| `cargo-deny --all-features check`（预编译二进制 0.20.2） | ✅ 四项全 ok | `advisories ok, bans ok, licenses ok, sources ok` |
| `cargo clippy/test -p yeban-engine` | ⛔ **未跑** | 明文禁止：`cpal` 是重依赖（AGENTS.md §5） |

### 6.2 CI

| 轮 | run id | 结论 | 红点与处置 |
| :-: | :--- | :--- | :--- |
| 1 | `37221166111` | ❌ | `checks`：文档链接门禁报本文件第 262 行有一个"链接"——其实是正文里写的 markdown 链接**语法示例**，被 `LINK_RE` 抓到 ⇒ 已改写措辞，本文件不再出现该字符组合。<br>`rust (yeban-engine)`：clippy 5 类错误（1 条 unused import、5 条 deprecated intrinsic、1 条 `mem_replace_option_with_some`）⇒ 已全部修掉（§1.3）。<br>其余 job 全绿：`deny` ✅（含新的 cpal/alsa 依赖）、`lockfile` ✅、`plan` ✅、另外 21 条 rust 腿 ✅。 |
| 2 | `37221884009`（commit `3b09dc1`） | ✅ **全绿** | `plan` ✅ / `checks` ✅（fmt + 12 条守卫 + schema + **文档链接** + 许可清单对账）/ `lockfile` ✅ / `deny` ✅（cpal+alsa 许可齐备）/ `rust (workspace 全量)` ✅ 3m57s —— 即 `clippy --workspace --all-targets -- -D warnings` + `test --workspace` 全通过，**本 crate 的 60 条单测（`grep -c '#\[test\]'` 实测；含 FTZ/DAZ 汇编判据 e2、PDC 不变量 c1/c2、退役回收 a1/a2/a3、延迟来源 i1/i2）在 x86_64 Linux 上实测通过**。 |

> 第 2 轮把 21 条 per-crate 矩阵腿换成了**一条 `rust (workspace 全量)`**（集成者改的 CI 形态）：
> 因为本次是 rebase 后的 force-push，`plan` 推导出工作区全量。判决更强（整仓 clippy+test），
> 但也就无法从矩阵腿名字直接看出"只有 yeban-engine 被验"；本行如实记录。

### 6.3 ⚠️ 两个门禁缺口（**发现，需要集成者修 `scripts/**`**）

**缺口 1 —— 重依赖检测认不出工作区继承写法。**
`run-gates.sh` 的重依赖正则是 `(…|cpal|…)[[:space:]]*=`，而实际写法是
`cpal = { workspace = true, optional = true }`（`cpal` 后面不是 `=`）。实测在该 crate 的
`Cargo.toml` 上 `grep` 退出码为 1（无匹配）⇒ `gate_crate` 不会 SKIP，
会在 M2 上真的编译 cpal（正是用户硬性禁止的事）。本线因此**没有**执行该命令。

**缺口 2 —— 工作树里文档链接门禁是空跑（并且真的让我多花了一轮 CI）。**
`check_docs_links.py` 的 `markdown_files()` 用**绝对**路径分量过滤 `SKIP_DIRS`，
而 `SKIP_DIRS` 含 `.worktrees`；工作树里 `REPO` 自身就在 `.worktrees/` 下 ⇒
所有 markdown 被跳过。实测输出 `note: 扫描 0 个 markdown 文件`。
第 1 轮 CI 的文档门禁是在 CI（非工作树）里才第一次真正扫到本文件并变红。
**临时对策**：本线改为用该脚本的 `LINK_RE` 正则手工扫自己的文件（§6.1）。
建议把过滤改成"相对 `REPO` 的路径分量"。

---

## 7. 复用来源（Reuse provenance）

本 crate 的代码是**新写**的：

- `graph.rs` 的 PDC 算法按规范 §3.4 的文字实现（拓扑排序 + 环形延迟线是教科书算法）；
- `block.rs` 的接口形状参考了兄弟 crate `crates/yeban-dsp/src/block.rs`（同仓库、同作者、
  同一许可），但**没有复制任何函数体**；
- `fpu.rs` 的 x86 内联汇编与 stdarch（`https://github.com/rust-lang/stdarch`，
  MIT/Apache-2.0）的 `_mm_getcsr`/`_mm_setcsr` 等价，是按上游 deprecation 提示做的
  两行 `stmxcsr`/`ldmxcsr` 重写；
- `device.rs` / `snapshot.rs` 的 cpal / rtrb 用法全部来自 §1.2 核验过的官方 API 与源码。

因此**不新增** `THIRD_PARTY_LICENSES.md` 条目。
