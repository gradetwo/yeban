//! [`EngineStats`] 的**跨线程只读镜像**：控制面在音频线程之外读取引擎健康读数。
//! [ARCH-TOP-002]、[ARCH-RT-001]、[ROAD-M2-001]
//!
//! # 1. 为什么需要它（出处照抄，不是本模块发明的需求）
//!
//! [`EngineRuntime`] 一旦挂到设备回调上就**归音频线程所有**（`yeban-app` 的
//! `EngineHost::open_device` 把运行时移进 cpal 的回调闭包）。此后控制线程拿不到
//! `&EngineRuntime` ⇒ [`EngineRuntime::stats`] 不可达。这一条已被两处**已提交**的
//! 登记如实写下：
//!
//! | 出处 | 原文（照抄） |
//! | :--- | :--- |
//! | `yeban-app` 的 `engine_host.rs` 模块文档 | 设备腿活跃时 `EngineRuntime` 归 cpal 回调线程所有 ⇒ `EngineHost::engine_stats` 返回 `None`（引擎累计统计不再可读）…… 需要一个"跨线程只读的 `EngineStats` 镜像"才能补上 —— **本票不做**（登记为 needs，不发明第二份统计来源） |
//! | `docs/ledger/feature-alignment.md` 的 "cpal 设备宿主" 行 | ③ **新登记的边界**：设备腿活跃时 `EngineHost::engine_stats()` 返回 `None`（`EngineRuntime` 已在音频线程上）…… 要补一条跨线程只读统计镜像才行，本票不发明第二份统计来源 |
//!
//! 两处都把修法指向**本 crate**（"不发明第二份统计来源"＝ 不要由 app 自己再数一遍，
//! 而是由引擎把**同一份**读数搬到跨线程可读的地方）。本模块就是那条搬运。
//!
//! ⚠ 产品形态上这不是可选项：`yeban-app` 的 `main.rs` 在启动时调用 `open_device`
//! ⇒ **默认运行形态就是设备腿**，而设备腿正是统计不可读的那一种。
//!
//! # 2. 形状：一个字段一个原子量，没有锁、没有队列、没有分配
//!
//! [`EngineStats`] 的**每一个字段**在 [`EngineStatsMirror`] 里都有一个对应的原子量。
//! 写入发生在实时路径（每量子一次 [`EngineStatsMirror::publish`]），读取发生在任意
//! 线程（[`EngineStatsMirror::read`]）。
//!
//! | 字段类型 | 承载 | 说明 |
//! | :--- | :--- | :--- |
//! | `u64` | `AtomicU64` | 直接存 |
//! | `f32` | `AtomicU32` + `to_bits` | 保持逐位（含 `-0.0` 与次正规数） |
//! | `Option<f32>` | 一个 `AtomicU64`：高 32 位 = 存在标记，低 32 位 = 位模式 | 存在性与值在**同一个原子量**里 ⇒ 不会读到"有值但值是上一个"的交错 |
//! | `bool` | `AtomicBool` | 直接存 |
//! | `Option<FtzDazOutcome>` | `AtomicU8` 编码 | `0` = 缺失，`1` = `Applied`，`2` = `Unsupported` |
//! | `TransportState` | `AtomicU8` 编码 | [`crate::transport::TransportState::code`] 的稳定编码 |
//!
//! **零分配 / 零锁 / 零阻塞 I/O / 零日志**（[红线 7]、[MUST-GATE-001]）：
//! `publish` 只做原子存、`read` 只做原子取，两者都不碰堆、不加锁、不做系统调用、
//! 不打印。`read` 返回的 [`EngineStats`] 是栈上的 `Copy` 值（无 `Drop`）。
//!
//! # 3. 一致性口径（**诚实边界**）
//!
//! 1. **内存序一律 `Relaxed`**。理由是这些量都是**诊断读数**：没有一个跨字段不变式
//!    被读者依赖（不存在"两个数必须同时取"的契约）。需要"某一量子上的精确一致读数"
//!    的地方是**同线程**的 [`EngineRuntime::stats`]，不是本镜像；
//! 2. **一次 `read` 可能横跨多个量子**（各字段各自原子取，可能来自不同的 `publish`）。
//!    这是刻意的取舍：换来的是"读者永不阻塞写者、写者永不阻塞读者"。凡是要作判据的
//!    等号，都必须取**静止点**（写者已停、线程已 join）——那时 `read()` 与
//!    [`EngineRuntime::stats`] **逐字段相等**（判据见 `tests/synth_rt_zero_alloc.rs`
//!    的场景 ⑮ 与 `tests/rt_zero_alloc.rs` 的静止点口径）；
//! 3. **单调字段不会回退**：计数类字段（`quanta` / `retire_*` / `*_frames` 等）在源侧
//!    只增不减 ⇒ 读者看到的序列单调不减。量规类字段（`retire_pending`）可升可降，
//!    与源侧同口径；
//! 4. **"还没发布过"与"发布了一个全零读数"不可区分**：镜像的初值就是引擎的**冷值**
//!    （全零，且 `ftz = None`、`transport_state = Stopped`、`release_thread_is_main = true`）。
//!    要区分请读 `quanta`（真实引擎处理过量子就 `> 0`）。
//!
//! ⚠ **冷值不等于 `EngineStats::default()`**：`release_thread_is_main` 的派生默认值是
//! `false`，而它的语义初值是 `true`（"尚无释放"是**空真**，不是"释放跑在外线程" ——
//! 与 [`crate::snapshot::RetireAccounting`] 的同名字段同一个约定）。若镜像照抄派生默认值，
//! 控制面在第一个量子之前会读到一条**假警报**。判据
//! [`tests::a_cold_mirror_reads_the_engine_cold_value`] 钉住这个区别。
//!
//! # 4. 写入点
//!
//! [`EngineRuntime`] 在**构造期**建好镜像（那时允许分配），把 `Arc` 的一份克隆交给
//! 控制面（[`EngineRuntime::stats_mirror`]），此后每次
//! [`EngineRuntime::process_quantum`] 结束时发布一次。构造期还会发布一次**初值**
//! （与 [`crate::transport::TransportMirror`] 的"先发布初值"同一个理由：控制面在
//! 第一个量子之前就能读到与引擎实际状态相符的数，而不是冷值）。

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};

use crate::fpu::FtzDazOutcome;
use crate::rt::EngineStats;
use crate::transport::TransportState;

/// `Option<f32>` 打包后的"存在"位（高 32 位为 `1`）。
const SOME_FLAG: u64 = 1 << 32;

/// `Option<FtzDazOutcome>` 的编码：缺失。
const FTZ_NONE: u8 = 0;
/// `Option<FtzDazOutcome>` 的编码：[`FtzDazOutcome::Applied`]。
const FTZ_APPLIED: u8 = 1;
/// `Option<FtzDazOutcome>` 的编码：[`FtzDazOutcome::Unsupported`]。
const FTZ_UNSUPPORTED: u8 = 2;

/// `Option<f32>` → `(存在标记, 位模式)` 打包。`None` ⇒ `0`。
#[must_use]
const fn pack_optional_f32(value: Option<f32>) -> u64 {
    match value {
        Some(value) => SOME_FLAG | (value.to_bits() as u64),
        None => 0,
    }
}

/// 打包 → `Option<f32>`。存在标记不是 `1` 时一律回落 `None`（含"读到撕裂值"的情形）。
#[must_use]
const fn unpack_optional_f32(packed: u64) -> Option<f32> {
    if packed & SOME_FLAG == SOME_FLAG {
        Some(f32::from_bits(packed as u32))
    } else {
        None
    }
}

/// [`Option<FtzDazOutcome>`] → `u8`。`match` 是穷举的 ⇒ 新增枚举变体会**编译失败**。
#[must_use]
const fn encode_ftz(value: Option<FtzDazOutcome>) -> u8 {
    match value {
        None => FTZ_NONE,
        Some(FtzDazOutcome::Applied) => FTZ_APPLIED,
        Some(FtzDazOutcome::Unsupported) => FTZ_UNSUPPORTED,
    }
}

/// `u8` → [`Option<FtzDazOutcome>`]。未知编码回落 `None`（不发明第三种状态）。
#[must_use]
const fn decode_ftz(code: u8) -> Option<FtzDazOutcome> {
    match code {
        FTZ_APPLIED => Some(FtzDazOutcome::Applied),
        FTZ_UNSUPPORTED => Some(FtzDazOutcome::Unsupported),
        _ => None,
    }
}

/// [`TransportState`] → `u8`。
///
/// 编码表**只有一份**：它住在 [`TransportState::code`]（本模块不复制那张表）。
#[must_use]
const fn encode_transport(value: TransportState) -> u8 {
    value.code()
}

/// `u8` → [`TransportState`]。未知编码回落 [`TransportState::Stopped`]
/// （与 [`TransportState::from_code`] 同口径：**不**发明状态）。
#[must_use]
const fn decode_transport(code: u8) -> TransportState {
    TransportState::from_code(code)
}

/// [`EngineStats`] 的**跨线程只读镜像**。
///
/// 一个字段一个原子量；`publish` 逐字段存、`read` 逐字段取。零分配、零锁、零 I/O
/// （见模块文档 §2）。构造期分配一次（`Arc` 与结构体本身），此后音频线程**只写**。
///
/// ⚠ 本类型**不**派生 `Clone`：共享靠 `Arc`（[`EngineRuntime::stats_mirror`] 返回克隆
/// 的 `Arc`），而不是复制一份状态 —— 复制出来的第二份会立刻漂移。
pub struct EngineStatsMirror {
    quanta: AtomicU64,
    events_applied: AtomicU64,
    snapshot_switches: AtomicU64,
    event_bulk_pops: AtomicU64,
    meter_frames: AtomicU64,
    meter_bulk_publishes: AtomicU64,
    meter_capacity_drops: AtomicU64,
    ftz: AtomicU8,
    rendered_samples: AtomicU64,
    scheduled_notes: AtomicU64,
    note_schedule_drops: AtomicU64,
    track_drops: AtomicU64,
    notes_triggered: AtomicU64,
    voice_steals: AtomicU64,
    limiter_gain_reductions: AtomicU64,
    limiter_max_reduction_bits: AtomicU32,
    limiter_current_reduction_bits: AtomicU32,
    insert_gain_reductions: AtomicU64,
    insert_max_reduction_db_bits: AtomicU32,
    insert_strip_frames: AtomicU64,
    insert_reverb_frames: AtomicU64,
    insert_reverb_rate_rejects: AtomicU64,
    insert_convolution_frames: AtomicU64,
    insert_convolution_rejects: AtomicU64,
    param_gain_frames: AtomicU64,
    param_master_gain_frames: AtomicU64,
    param_gain_rejects: AtomicU64,
    param_unmapped_events: AtomicU64,
    param_capacity_drops: AtomicU64,
    pdc_unarmed_nodes: AtomicU64,
    pdc_clamped_frames: AtomicU64,
    pdc_processed_blocks: AtomicU64,
    metronome_clicks: AtomicU64,
    drum_hits: AtomicU64,
    quanta_per_second: AtomicU64,
    transport_state: AtomicU8,
    position_ticks: AtomicU64,
    position_frames: AtomicU64,
    transport_commands: AtomicU64,
    transport_quanta: AtomicU64,
    snapshot_stash_events: AtomicU64,
    retire_pending: AtomicU64,
    retire_drained: AtomicU64,
    retire_drain_calls: AtomicU64,
    retire_pruned: AtomicU64,
    release_thread_is_main: AtomicBool,
    foreign_drains: AtomicU64,
}

impl EngineStatsMirror {
    /// 引擎**冷值**的镜像（`const` ⇒ 可在静态量与常量里用；不分配）。
    ///
    /// 冷值 = 全零，但 `release_thread_is_main = true`（空真，见模块文档 §3 第 4 条）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            quanta: AtomicU64::new(0),
            events_applied: AtomicU64::new(0),
            snapshot_switches: AtomicU64::new(0),
            event_bulk_pops: AtomicU64::new(0),
            meter_frames: AtomicU64::new(0),
            meter_bulk_publishes: AtomicU64::new(0),
            meter_capacity_drops: AtomicU64::new(0),
            ftz: AtomicU8::new(FTZ_NONE),
            rendered_samples: AtomicU64::new(0),
            scheduled_notes: AtomicU64::new(0),
            note_schedule_drops: AtomicU64::new(0),
            track_drops: AtomicU64::new(0),
            notes_triggered: AtomicU64::new(0),
            voice_steals: AtomicU64::new(0),
            limiter_gain_reductions: AtomicU64::new(0),
            limiter_max_reduction_bits: AtomicU32::new(0),
            limiter_current_reduction_bits: AtomicU32::new(0),
            insert_gain_reductions: AtomicU64::new(0),
            insert_max_reduction_db_bits: AtomicU32::new(0),
            insert_strip_frames: AtomicU64::new(0),
            insert_reverb_frames: AtomicU64::new(0),
            insert_reverb_rate_rejects: AtomicU64::new(0),
            insert_convolution_frames: AtomicU64::new(0),
            insert_convolution_rejects: AtomicU64::new(0),
            param_gain_frames: AtomicU64::new(0),
            param_master_gain_frames: AtomicU64::new(0),
            param_gain_rejects: AtomicU64::new(0),
            param_unmapped_events: AtomicU64::new(0),
            param_capacity_drops: AtomicU64::new(0),
            pdc_unarmed_nodes: AtomicU64::new(0),
            pdc_clamped_frames: AtomicU64::new(0),
            pdc_processed_blocks: AtomicU64::new(0),
            metronome_clicks: AtomicU64::new(0),
            drum_hits: AtomicU64::new(0),
            quanta_per_second: AtomicU64::new(0),
            // 初值 = `EngineStats::default()` 的走带状态。这里写 `Stopped` 而不是
            // `TransportState::default()` 是因为本函数是 `const`（`Default::default`
            // 不是 `const`）。两者**必须**同值，漂移由判据
            // `a_cold_mirror_reads_the_default_stats` 钉死。
            transport_state: AtomicU8::new(encode_transport(TransportState::Stopped)),
            position_ticks: AtomicU64::new(0),
            position_frames: AtomicU64::new(0),
            transport_commands: AtomicU64::new(0),
            transport_quanta: AtomicU64::new(0),
            snapshot_stash_events: AtomicU64::new(0),
            retire_pending: AtomicU64::new(0),
            retire_drained: AtomicU64::new(0),
            retire_drain_calls: AtomicU64::new(0),
            retire_pruned: AtomicU64::new(0),
            release_thread_is_main: AtomicBool::new(true),
            foreign_drains: AtomicU64::new(0),
        }
    }

    /// 把一份**权威读数**逐字段存进镜像（实时路径：每量子一次；零分配零锁零 I/O）。
    ///
    /// 全部存操作都是 `Relaxed`（理由见模块文档 §3 第 1 条）。
    pub fn publish(&self, stats: &EngineStats) {
        self.quanta.store(stats.quanta, Ordering::Relaxed);
        self.events_applied
            .store(stats.events_applied, Ordering::Relaxed);
        self.snapshot_switches
            .store(stats.snapshot_switches, Ordering::Relaxed);
        self.event_bulk_pops
            .store(stats.event_bulk_pops, Ordering::Relaxed);
        self.meter_frames
            .store(stats.meter_frames, Ordering::Relaxed);
        self.meter_bulk_publishes
            .store(stats.meter_bulk_publishes, Ordering::Relaxed);
        self.meter_capacity_drops
            .store(stats.meter_capacity_drops, Ordering::Relaxed);
        self.ftz.store(encode_ftz(stats.ftz), Ordering::Relaxed);
        self.rendered_samples
            .store(stats.rendered_samples, Ordering::Relaxed);
        self.scheduled_notes
            .store(stats.scheduled_notes, Ordering::Relaxed);
        self.note_schedule_drops
            .store(stats.note_schedule_drops, Ordering::Relaxed);
        self.track_drops.store(stats.track_drops, Ordering::Relaxed);
        self.notes_triggered
            .store(stats.notes_triggered, Ordering::Relaxed);
        self.voice_steals
            .store(stats.voice_steals, Ordering::Relaxed);
        self.limiter_gain_reductions
            .store(stats.limiter_gain_reductions, Ordering::Relaxed);
        self.limiter_max_reduction_bits
            .store(stats.limiter_max_reduction.to_bits(), Ordering::Relaxed);
        self.limiter_current_reduction_bits
            .store(stats.limiter_current_reduction.to_bits(), Ordering::Relaxed);
        self.insert_gain_reductions
            .store(stats.insert_gain_reductions, Ordering::Relaxed);
        self.insert_max_reduction_db_bits
            .store(stats.insert_max_reduction_db.to_bits(), Ordering::Relaxed);
        self.insert_strip_frames
            .store(stats.insert_strip_frames, Ordering::Relaxed);
        self.insert_reverb_frames
            .store(stats.insert_reverb_frames, Ordering::Relaxed);
        self.insert_reverb_rate_rejects
            .store(stats.insert_reverb_rate_rejects, Ordering::Relaxed);
        self.insert_convolution_frames
            .store(stats.insert_convolution_frames, Ordering::Relaxed);
        self.insert_convolution_rejects
            .store(stats.insert_convolution_rejects, Ordering::Relaxed);
        self.param_gain_frames
            .store(stats.param_gain_frames, Ordering::Relaxed);
        self.param_master_gain_frames
            .store(stats.param_master_gain_frames, Ordering::Relaxed);
        self.param_gain_rejects
            .store(stats.param_gain_rejects, Ordering::Relaxed);
        self.param_unmapped_events
            .store(stats.param_unmapped_events, Ordering::Relaxed);
        self.param_capacity_drops
            .store(stats.param_capacity_drops, Ordering::Relaxed);
        self.pdc_unarmed_nodes
            .store(stats.pdc_unarmed_nodes, Ordering::Relaxed);
        self.pdc_clamped_frames
            .store(stats.pdc_clamped_frames, Ordering::Relaxed);
        self.pdc_processed_blocks
            .store(stats.pdc_processed_blocks, Ordering::Relaxed);
        self.metronome_clicks
            .store(stats.metronome_clicks, Ordering::Relaxed);
        self.drum_hits.store(stats.drum_hits, Ordering::Relaxed);
        self.quanta_per_second.store(
            pack_optional_f32(stats.quanta_per_second),
            Ordering::Relaxed,
        );
        self.transport_state
            .store(encode_transport(stats.transport_state), Ordering::Relaxed);
        self.position_ticks
            .store(stats.position_ticks, Ordering::Relaxed);
        self.position_frames
            .store(stats.position_frames, Ordering::Relaxed);
        self.transport_commands
            .store(stats.transport_commands, Ordering::Relaxed);
        self.transport_quanta
            .store(stats.transport_quanta, Ordering::Relaxed);
        self.snapshot_stash_events
            .store(stats.snapshot_stash_events, Ordering::Relaxed);
        self.retire_pending
            .store(stats.retire_pending, Ordering::Relaxed);
        self.retire_drained
            .store(stats.retire_drained, Ordering::Relaxed);
        self.retire_drain_calls
            .store(stats.retire_drain_calls, Ordering::Relaxed);
        self.retire_pruned
            .store(stats.retire_pruned, Ordering::Relaxed);
        self.release_thread_is_main
            .store(stats.release_thread_is_main, Ordering::Relaxed);
        self.foreign_drains
            .store(stats.foreign_drains, Ordering::Relaxed);
    }

    /// 读出一份 [`EngineStats`]（任意线程；零分配零锁零 I/O）。
    ///
    /// 一次调用内各字段各自原子取 ⇒ 可能横跨多个 `publish`。要作等号判据请在
    /// **静止点**读（模块文档 §3 第 2 条）。
    #[must_use]
    pub fn read(&self) -> EngineStats {
        EngineStats {
            quanta: self.quanta.load(Ordering::Relaxed),
            events_applied: self.events_applied.load(Ordering::Relaxed),
            snapshot_switches: self.snapshot_switches.load(Ordering::Relaxed),
            event_bulk_pops: self.event_bulk_pops.load(Ordering::Relaxed),
            meter_frames: self.meter_frames.load(Ordering::Relaxed),
            meter_bulk_publishes: self.meter_bulk_publishes.load(Ordering::Relaxed),
            meter_capacity_drops: self.meter_capacity_drops.load(Ordering::Relaxed),
            ftz: decode_ftz(self.ftz.load(Ordering::Relaxed)),
            rendered_samples: self.rendered_samples.load(Ordering::Relaxed),
            scheduled_notes: self.scheduled_notes.load(Ordering::Relaxed),
            note_schedule_drops: self.note_schedule_drops.load(Ordering::Relaxed),
            track_drops: self.track_drops.load(Ordering::Relaxed),
            notes_triggered: self.notes_triggered.load(Ordering::Relaxed),
            voice_steals: self.voice_steals.load(Ordering::Relaxed),
            limiter_gain_reductions: self.limiter_gain_reductions.load(Ordering::Relaxed),
            limiter_max_reduction: f32::from_bits(
                self.limiter_max_reduction_bits.load(Ordering::Relaxed),
            ),
            limiter_current_reduction: f32::from_bits(
                self.limiter_current_reduction_bits.load(Ordering::Relaxed),
            ),
            insert_gain_reductions: self.insert_gain_reductions.load(Ordering::Relaxed),
            insert_max_reduction_db: f32::from_bits(
                self.insert_max_reduction_db_bits.load(Ordering::Relaxed),
            ),
            insert_strip_frames: self.insert_strip_frames.load(Ordering::Relaxed),
            insert_reverb_frames: self.insert_reverb_frames.load(Ordering::Relaxed),
            insert_reverb_rate_rejects: self.insert_reverb_rate_rejects.load(Ordering::Relaxed),
            insert_convolution_frames: self.insert_convolution_frames.load(Ordering::Relaxed),
            insert_convolution_rejects: self.insert_convolution_rejects.load(Ordering::Relaxed),
            param_gain_frames: self.param_gain_frames.load(Ordering::Relaxed),
            param_master_gain_frames: self.param_master_gain_frames.load(Ordering::Relaxed),
            param_gain_rejects: self.param_gain_rejects.load(Ordering::Relaxed),
            param_unmapped_events: self.param_unmapped_events.load(Ordering::Relaxed),
            param_capacity_drops: self.param_capacity_drops.load(Ordering::Relaxed),
            pdc_unarmed_nodes: self.pdc_unarmed_nodes.load(Ordering::Relaxed),
            pdc_clamped_frames: self.pdc_clamped_frames.load(Ordering::Relaxed),
            pdc_processed_blocks: self.pdc_processed_blocks.load(Ordering::Relaxed),
            metronome_clicks: self.metronome_clicks.load(Ordering::Relaxed),
            drum_hits: self.drum_hits.load(Ordering::Relaxed),
            quanta_per_second: unpack_optional_f32(self.quanta_per_second.load(Ordering::Relaxed)),
            transport_state: decode_transport(self.transport_state.load(Ordering::Relaxed)),
            position_ticks: self.position_ticks.load(Ordering::Relaxed),
            position_frames: self.position_frames.load(Ordering::Relaxed),
            transport_commands: self.transport_commands.load(Ordering::Relaxed),
            transport_quanta: self.transport_quanta.load(Ordering::Relaxed),
            snapshot_stash_events: self.snapshot_stash_events.load(Ordering::Relaxed),
            retire_pending: self.retire_pending.load(Ordering::Relaxed),
            retire_drained: self.retire_drained.load(Ordering::Relaxed),
            retire_drain_calls: self.retire_drain_calls.load(Ordering::Relaxed),
            retire_pruned: self.retire_pruned.load(Ordering::Relaxed),
            release_thread_is_main: self.release_thread_is_main.load(Ordering::Relaxed),
            foreign_drains: self.foreign_drains.load(Ordering::Relaxed),
        }
    }
}

impl Default for EngineStatsMirror {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for EngineStatsMirror {
    /// 打印**读出来**的那一份读数（不是 39 个原子量的内部状态）。
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EngineStatsMirror")
            .field("stats", &self.read())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::TransportState;

    /// 一份**逐字段互不相同**的读数：任何"漏存一个字段"都会在回读时暴露。
    ///
    /// ⚠ 这个函数是**漂移闸门**：它用**完整**的结构体字面量构造 [`EngineStats`]
    /// ⇒ 以后往 `EngineStats` 加字段会让这里**编译失败**，逼作者同时把镜像补齐
    /// （这比"注释里写一句请记得同步"可靠）。
    fn sentinel_stats() -> EngineStats {
        EngineStats {
            quanta: 1,
            events_applied: 2,
            snapshot_switches: 3,
            event_bulk_pops: 4,
            meter_frames: 5,
            meter_bulk_publishes: 6,
            meter_capacity_drops: 7,
            ftz: Some(FtzDazOutcome::Unsupported),
            rendered_samples: 9,
            scheduled_notes: 10,
            note_schedule_drops: 11,
            track_drops: 12,
            notes_triggered: 13,
            voice_steals: 14,
            limiter_gain_reductions: 15,
            limiter_max_reduction: 0.25,
            limiter_current_reduction: 0.125,
            insert_gain_reductions: 17,
            insert_max_reduction_db: -6.5,
            insert_strip_frames: 19,
            insert_reverb_frames: 20,
            insert_reverb_rate_rejects: 21,
            insert_convolution_frames: 27,
            insert_convolution_rejects: 28,
            param_gain_frames: 40,
            param_master_gain_frames: 44,
            param_gain_rejects: 41,
            param_unmapped_events: 42,
            param_capacity_drops: 43,
            pdc_unarmed_nodes: 22,
            pdc_clamped_frames: 23,
            pdc_processed_blocks: 24,
            metronome_clicks: 25,
            drum_hits: 26,
            quanta_per_second: Some(375.5),
            transport_state: TransportState::Playing,
            position_ticks: 29,
            position_frames: 30,
            transport_commands: 31,
            transport_quanta: 32,
            snapshot_stash_events: 33,
            retire_pending: 34,
            retire_drained: 35,
            retire_drain_calls: 36,
            retire_pruned: 37,
            release_thread_is_main: false,
            foreign_drains: 39,
        }
    }

    /// 判据 ①：整份读数逐字段往返（`PartialEq` 是整体等号）。
    #[test]
    fn every_field_round_trips_through_the_mirror() {
        let mirror = EngineStatsMirror::new();
        let published = sentinel_stats();
        mirror.publish(&published);
        assert_eq!(mirror.read(), published, "镜像回读必须与发布值逐字段相同");
    }

    /// 判据 ②：冷镜像读出的是**引擎的冷值**（不是 `EngineStats::default()`）。
    ///
    /// 唯一一处刻意偏离派生默认值的字段是 `release_thread_is_main`：派生默认 `false`
    /// 读作"释放跑在外线程"（假警报），而语义初值是 `true`（空真）。两处都断言，
    /// 这样"改回派生默认值"会立刻变红。
    #[test]
    fn a_cold_mirror_reads_the_engine_cold_value() {
        let cold = EngineStats {
            release_thread_is_main: true,
            ..EngineStats::default()
        };
        assert_eq!(EngineStatsMirror::new().read(), cold);
        // `Default::default()` 与 `new()` 必须同一条路径（否则两个初值会漂移）。
        assert_eq!(EngineStatsMirror::default().read(), cold);
        // 反面对照：派生默认值**不是**冷值（这一条证明上面那条断言有牙）。
        assert_ne!(
            EngineStats::default().release_thread_is_main,
            cold.release_thread_is_main
        );
    }

    /// 判据 ③：`publish` 是**覆写**（不是累加）：第二次发布把全部字段拉到新值。
    #[test]
    fn publishing_again_overwrites_gauges_and_counters() {
        let mirror = EngineStatsMirror::new();
        mirror.publish(&sentinel_stats());
        let lower = EngineStats {
            quanta: 0,
            ftz: Some(FtzDazOutcome::Applied),
            quanta_per_second: None,
            transport_state: TransportState::Stopped,
            release_thread_is_main: true,
            ..EngineStats::default()
        };
        mirror.publish(&lower);
        assert_eq!(mirror.read(), lower);
    }

    /// 判据 ④：`Option<f32>` 的存在性与值在**同一个原子量**里
    /// ⇒ `None` 不会被读成"上一个值"。
    #[test]
    fn optional_f32_presence_is_packed_with_its_value() {
        let mirror = EngineStatsMirror::new();
        mirror.publish(&EngineStats {
            quanta_per_second: Some(-0.0),
            ..EngineStats::default()
        });
        // `-0.0` 与 `+0.0` 数值相等但位模式不同 ⇒ 用位模式断言"逐位保真"。
        let read = mirror.read().quanta_per_second.expect("必须存在");
        assert_eq!(read.to_bits(), (-0.0f32).to_bits(), "-0.0 必须逐位保留");
        mirror.publish(&EngineStats {
            quanta_per_second: None,
            ..EngineStats::default()
        });
        assert_eq!(mirror.read().quanta_per_second, None);
    }

    /// 判据 ⑤：非有限 `f32` 也逐位往返（镜像**不**做净化 —— 净化是源侧的事）。
    #[test]
    fn non_finite_floats_are_preserved_bit_for_bit() {
        let mirror = EngineStatsMirror::new();
        mirror.publish(&EngineStats {
            limiter_max_reduction: f32::NAN,
            insert_max_reduction_db: f32::NEG_INFINITY,
            ..EngineStats::default()
        });
        let read = mirror.read();
        assert!(read.limiter_max_reduction.is_nan());
        assert_eq!(read.insert_max_reduction_db, f32::NEG_INFINITY);
    }

    /// 判据 ⑥：`Option<FtzDazOutcome>` 三个状态各自往返（编码表是穷举 `match`）。
    #[test]
    fn ftz_outcome_round_trips_every_state() {
        for value in [
            None,
            Some(FtzDazOutcome::Applied),
            Some(FtzDazOutcome::Unsupported),
        ] {
            assert_eq!(decode_ftz(encode_ftz(value)), value);
            let mirror = EngineStatsMirror::new();
            mirror.publish(&EngineStats {
                ftz: value,
                ..EngineStats::default()
            });
            assert_eq!(mirror.read().ftz, value);
        }
    }

    /// 判据 ⑦：走带状态三种取值各自往返；未知编码回落 `Stopped`（不发明状态）。
    #[test]
    fn transport_state_round_trips_and_falls_back_to_stopped() {
        for state in [
            TransportState::Stopped,
            TransportState::Playing,
            TransportState::Recording,
        ] {
            assert_eq!(decode_transport(encode_transport(state)), state);
        }
        // 0xFF 不在编码表里 ⇒ 回落 `Stopped`（而不是 panic 或发明第四种状态）。
        assert_eq!(decode_transport(0xFF), TransportState::Stopped);
    }
}
