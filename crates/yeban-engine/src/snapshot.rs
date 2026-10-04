//! 不可变引擎快照、原子交换槽与退役回收队列。
//! [ARCH-RT-002, ROAD-M2-002, ARCH-TOP-002]
//!
//! ## 规范要求
//!
//! [ARCH-RT-002] 原文（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2）：
//!
//! > 音频线程内使用原子指针读取最新的不可变 `Arc<EngineSnapshot>`；若检测到拓扑版本更新，
//! > 音频线程将持有的旧快照 **move** 进预先分配的无锁队列
//! > `rtrb::Producer<Arc<EngineSnapshot>>`；**主线程以 60Hz 轮询从回收队列出队并负责旧快照
//! > 的 Drop 析构**，彻底杜绝音频线程内发生任何内存释放（Zero Free）。
//!
//! 因此本模块把职责切成三块：
//!
//! | 角色 | 线程 | 做的事 |
//! | :--- | :--- | :--- |
//! | [`SnapshotSlot::publish`] | Model 线程（非实时） | 构造新快照、原子换指针、把旧 `Arc` 移入待回收清单 |
//! | [`SnapshotReader`] | cpal 回调线程 | 无锁读指针、`Arc` 克隆、把旧 `Arc` push 进退役队列 |
//! | [`RetireQueue::drain`] + [`SnapshotSlot::prune`] | 主线程 60Hz | 出队并 **Drop**；释放写者侧待回收清单 |
//!
//! ## 无锁指针交换的**内存回收安全证明**（本模块唯一的 `unsafe` 依据）
//!
//! 裸指针方案的唯一危险是"读者 load 到指针、还没来得及把引用计数 +1，写者就把对象释放了"。
//! 本模块用一个**纪元握手**（epoch handshake）关掉这个窗口，只依赖 Acquire/Release：
//!
//! ```text
//! 写者 publish():                        读者 begin_block():
//!   anchor = new                            epoch = slot.epoch.load(Acquire)   (1)
//!   ptr.store(new, Release)                 ptr   = slot.ptr.load(Acquire)     (2)
//!   e = epoch.fetch_add(1, AcqRel) + 1      if ptr != held { increment_strong_count; from_raw }
//!   pending.push((e, old))                  读者 end_block():
//!                                             slot.reader_done.store(epoch, Release)  (3)
//! 写者侧 prune():
//!   done = reader_done.load(Acquire)
//!   drop 掉所有 e <= done 的待回收项
//! ```
//!
//! **论证**：
//!
//! 1. 写者先 `ptr.store` 再 `epoch.fetch_add`（程序序）。读者若在 (1) 读到纪元 `e`，
//!    则 `ptr.store(Release)` **happens-before** 这次 `fetch_add`（写者程序序），
//!    而读者以 `Acquire` 读到了 `fetch_add` 的结果，因此 `ptr.store` 也 happens-before
//!    读者的 (1)。由写读一致性，读者其后的 (2) 不可能读到比 `ptr.store` 更早的值：
//!    **读者在纪元 `e` 的块里绝不会解引用被 `e` 标记淘汰的那个快照**。
//! 2. `reader_done` 只写一次"我完成了纪元 `e` 的块"，且读者是**单线程顺序**
//!    执行块：一旦 `done >= e`，所有更早的块都已完整结束，其中对旧指针的
//!    `increment_strong_count` 也已完成。
//! 3. 读者以 `Release` 写 (3)、写者以 `Acquire` 读 `done` ⇒ 读者在 (3) 之前做的一切
//!    （包括引用计数 +1）happens-before 写者 `prune` 里的 `drop`。因此读者**自己的**
//!    `Arc` 与写者 anchor 的引用计数都已被抬升，`drop` 不会打断任何在途读者。
//! 4. 锚（`anchor`）始终持有当前快照的一个强引用 ⇒ `slot.ptr` 永不悬垂，
//!    任何时刻新建的读者都能安全地克隆它。
//!
//! 只要读者**每个块**都调用 [`SnapshotReader::end_block`]，`prune` 就总能在两个块周期内
//! 释放写者侧清单。读者线程若停止（关流），[`Drop`] 会把 `reader_done` 置为 `u64::MAX`
//! 并解除 attached 标记，`prune` 随即可以清空全部待回收项。
//!
//! ## 边界（本切片没有证明的）
//!
//! - **`unsafe` 未做形式化验证**：上面的论证是纸面推导 + 单线程顺序假设，
//!   `Miri`/`loom` 的机械化验证列入 notes 的 needs 清单。当前不引入 `loom`（新依赖）
//!   是为了不扩大依赖图。
//! - **只支持单个读者**：`SnapshotReader` 的"单线程顺序块"前提是安全证明的一部分。
//!   多读者需要换成真正的 hazard pointer 数组。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use thiserror::Error;
use yeban_dsp::math::note_to_hz;
use yeban_model::{
    BlockSize, ClipContent, EntityId, ModelError, RoutingGraph, TrackKind, TrackV3, YebanProjectV1,
};

use crate::graph::{LatencyTable, PdcError, PdcPlan};
use crate::synth::{
    MAX_NOTES_PER_TRACK, NoteSchedule, ScheduledNote, tick_to_sample, velocity_gain,
};

/// 退役队列默认容量（条）。一帧 60Hz 内被替换的快照远不会超过这个数。
pub const DEFAULT_RETIRE_CAPACITY: usize = 32;

/// 快照投影错误。
#[derive(Debug, Error, PartialEq)]
pub enum SnapshotError {
    /// PDC 拓扑计算失败（成环 / master 不在图里 / 边端点缺失）。
    #[error(transparent)]
    Pdc(#[from] PdcError),

    /// 模型层校验失败。
    #[error(transparent)]
    Model(#[from] ModelError),

    /// 工程没有可用的主总线节点 —— 无法确定 PDC 的关键路径终点。
    ///
    /// 触发条件：`YebanProjectV1::master_bus_track_id` 是空 ULID，
    /// 或者它不在 `routing_graph.nodes` 里。
    #[error("engine snapshot: project has no usable master bus node")]
    NoMasterBus,
}

/// 音频线程需要的单轨参数（`TrackV3` 的**只读投影**）。
///
/// 故意不搬运 `name`/`color`/`clips` 等界面或编辑期数据：音频线程只需混音参数，
/// 少复制一个 `String` 就少一处 `Drop` 出现在快照里 [ARCH-RT-001]。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackParams {
    kind: TrackKind,
    volume_db: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    solo_safe: bool,
    device_count: u32,
    latency_samples: u32,
}

impl TrackParams {
    /// 从模型层音轨投影（**只读**，不改动模型）。
    ///
    /// `latency_samples` 由调用方提供，因为 [ARCH-PDC-001] 要求的
    /// `DeviceDefinition::latency_samples` 在 `yeban-model` 里还不存在
    /// （见 [`crate::graph`] 模块文档的规范缺口）。
    #[must_use]
    pub fn from_track(track: &TrackV3, latency_samples: u32) -> Self {
        Self {
            kind: track.kind,
            volume_db: track.volume_db,
            pan: track.pan,
            mute: track.mute,
            solo: track.solo,
            solo_safe: track.solo_safe,
            device_count: u32::try_from(track.devices.len()).unwrap_or(u32::MAX),
            latency_samples,
        }
    }

    /// 音轨类型。
    #[must_use]
    pub const fn kind(&self) -> TrackKind {
        self.kind
    }

    /// 音量 (dB)。
    #[must_use]
    pub const fn volume_db(&self) -> f32 {
        self.volume_db
    }

    /// 声相 -1.0..=1.0。
    #[must_use]
    pub const fn pan(&self) -> f32 {
        self.pan
    }

    /// 静音。
    #[must_use]
    pub const fn is_muted(&self) -> bool {
        self.mute
    }

    /// 独奏。
    #[must_use]
    pub const fn is_soloed(&self) -> bool {
        self.solo
    }

    /// 独奏安全。
    #[must_use]
    pub const fn is_solo_safe(&self) -> bool {
        self.solo_safe
    }

    /// 设备链长度。
    #[must_use]
    pub const fn device_count(&self) -> u32 {
        self.device_count
    }

    /// 本轨自身处理延迟（采样点）。
    #[must_use]
    pub const fn latency_samples(&self) -> u32 {
        self.latency_samples
    }
}

/// 不可变引擎快照：音频线程在每个渲染量子边界读取的唯一权威运行态
/// [ARCH-RT-002, ROAD-M2-002]。
///
/// - **全部字段私有 + 只有 getter** ⇒ 构造后无法修改，可以安全地 `Arc` 共享给音频线程
///   而不需要任何锁；
/// - 由模型层数据**投影**而来（[`EngineSnapshot::from_project`]），不持有 `&mut` 模型引用；
/// - `Send + Sync`（字段都是 `Send + Sync`），因此 `Arc<EngineSnapshot>` 可以跨线程传递。
#[derive(Clone, Debug, PartialEq)]
pub struct EngineSnapshot {
    revision: u64,
    sample_rate: u32,
    block_frames: usize,
    channels: u16,
    master: EntityId,
    tracks: BTreeMap<EntityId, TrackParams>,
    /// 每轨的音符调度表（`BTreeMap` ⇒ 迭代顺序确定 [MODEL-AST-003]）。
    ///
    /// 实时侧只读它、不构造它：tick → 样本的换算、`probability` 触发判定、
    /// 力度/音量增益全部在**控制线程**算完（见 [`crate::synth`] 模块文档 §1）。
    schedules: BTreeMap<EntityId, NoteSchedule>,
    /// 当前快照里已调度的音符总条数（诊断/判据用）。
    scheduled_notes: usize,
    /// 因 [`MAX_NOTES_PER_TRACK`] 容量上限而被丢弃的音符条数（构造期计数）。
    note_schedule_drops: u64,
    pdc: PdcPlan,
}

impl EngineSnapshot {
    /// 从工程投影出一个快照 [ROAD-M2-002]。
    ///
    /// `revision` 是模型层的提交版本号（单调递增），用于让 UI/日志判断"音频线程是否
    /// 已经追上"。
    ///
    /// 节点延迟**自动**从模型读取（[`LatencyTable::from_project`]，即
    /// `DeviceDefinition::latency_samples` 之和）[ARCH-PDC-001]。
    /// 需要注入测量值/构造合成场景时用 [`from_project_with_latencies`](Self::from_project_with_latencies)。
    ///
    /// # Errors
    ///
    /// - [`SnapshotError::NoMasterBus`]：工程没有可用的主总线节点；
    /// - [`SnapshotError::Pdc`]：路由图成环 / 边端点缺失；
    /// - [`SnapshotError::Model`]：`RoutingGraph::validate()` 失败。
    pub fn from_project(project: &YebanProjectV1, revision: u64) -> Result<Self, SnapshotError> {
        let latencies = LatencyTable::from_project(project);
        Self::from_project_with_latencies(project, revision, &latencies)
    }

    /// 同 [`from_project`](Self::from_project)，但延迟表由调用方显式提供。
    ///
    /// 用途：离线对账时注入实测延迟、测试时构造"只有一条支路有延迟"的合成场景。
    /// 生产路径应当用 [`from_project`](Self::from_project)，避免出现第二个延迟事实源。
    ///
    /// # Errors
    ///
    /// 同 [`from_project`](Self::from_project)。
    pub fn from_project_with_latencies(
        project: &YebanProjectV1,
        revision: u64,
        latencies: &LatencyTable,
    ) -> Result<Self, SnapshotError> {
        let master = project.master_bus_track_id;
        if master.is_nil() || !project.routing_graph.nodes.contains(&master) {
            return Err(SnapshotError::NoMasterBus);
        }
        project.routing_graph.validate()?;

        let mut tracks: BTreeMap<EntityId, TrackParams> = BTreeMap::new();
        for (id, track) in &project.tracks {
            tracks.insert(*id, TrackParams::from_track(track, latencies.get(id)));
        }
        let block = project.audio_config.block_size.frames() as usize;
        let (schedules, dropped) = project_schedules(project);
        Self::from_parts(
            revision,
            project.audio_config.sample_rate.hz(),
            block,
            2,
            master,
            tracks,
            &project.routing_graph,
            latencies,
        )
        .map(|snapshot| snapshot.with_schedules(schedules, dropped))
    }

    /// 低层构造：显式给出全部字段（离线渲染器与测试用）。
    ///
    /// **不含**音符调度表（构造出的是静音快照）；需要发声时用
    /// [`with_schedules`](Self::with_schedules) 附上，或直接用
    /// [`from_project`](Self::from_project)（它会把工程的 MIDI 摆放投影成调度表）。
    ///
    /// # Errors
    ///
    /// 同 [`from_project`](Self::from_project)（不含 `NoMasterBus` 的 nil 判断，
    /// 但仍要求 `master` 在 `routing_graph.nodes` 里）。
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        revision: u64,
        sample_rate: u32,
        block_frames: usize,
        channels: u16,
        master: EntityId,
        tracks: BTreeMap<EntityId, TrackParams>,
        routing: &RoutingGraph,
        latencies: &LatencyTable,
    ) -> Result<Self, SnapshotError> {
        let pdc = PdcPlan::compute(routing, master, latencies)?;
        Ok(Self {
            revision,
            sample_rate,
            block_frames,
            channels,
            master,
            tracks,
            schedules: BTreeMap::new(),
            scheduled_notes: 0,
            note_schedule_drops: 0,
            pdc,
        })
    }

    /// 附上音符调度表（构造期**允许分配**：这一步在控制线程上）。
    ///
    /// `dropped` 是构造调度表时因容量上限丢弃的音符条数
    /// （见 [`crate::synth::MAX_NOTES_PER_TRACK`]）。
    #[must_use]
    pub fn with_schedules(
        mut self,
        schedules: BTreeMap<EntityId, NoteSchedule>,
        dropped: u64,
    ) -> Self {
        self.scheduled_notes = schedules.values().map(NoteSchedule::len).sum();
        self.note_schedule_drops = dropped;
        self.schedules = schedules;
        self
    }

    /// 模型层提交版本号。
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// 采样率 (Hz)——来自 `audio_config.sample_rate`，本 crate 不再另立事实源。
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 渲染量子帧数（[ARCH-DET-001] 的 L1 固定块长，默认 128）。
    #[must_use]
    pub const fn block_frames(&self) -> usize {
        self.block_frames
    }

    /// 输出通道数。
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.channels
    }

    /// 主总线节点身份。
    #[must_use]
    pub const fn master(&self) -> EntityId {
        self.master
    }

    /// 音轨参数表（`BTreeMap` ⇒ 迭代顺序确定 [MODEL-AST-003]）。
    #[must_use]
    pub const fn tracks(&self) -> &BTreeMap<EntityId, TrackParams> {
        &self.tracks
    }

    /// 单轨参数。
    #[must_use]
    pub fn track(&self, id: &EntityId) -> Option<&TrackParams> {
        self.tracks.get(id)
    }

    /// 全部音轨的音符调度表（`BTreeMap` ⇒ 迭代顺序确定 [MODEL-AST-003]）。
    #[must_use]
    pub const fn schedules(&self) -> &BTreeMap<EntityId, NoteSchedule> {
        &self.schedules
    }

    /// 单轨的音符调度表（没有该轨时为 `None`）。
    #[must_use]
    pub fn schedule(&self, id: &EntityId) -> Option<&NoteSchedule> {
        self.schedules.get(id)
    }

    /// 本快照已调度的音符总条数。
    #[must_use]
    pub const fn scheduled_notes(&self) -> usize {
        self.scheduled_notes
    }

    /// 构造调度表时因容量上限丢弃的音符条数。
    #[must_use]
    pub const fn note_schedule_drops(&self) -> u64 {
        self.note_schedule_drops
    }

    /// PDC 计划（拓扑序 + 每节点补偿延迟）。
    #[must_use]
    pub const fn pdc(&self) -> &PdcPlan {
        &self.pdc
    }

    /// 模型层 `BlockSize` 枚举是否与本快照的块长一致（L1 契约自检）。
    #[must_use]
    pub fn block_size_matches_enum(&self) -> bool {
        BlockSize::from_frames(self.block_frames as u32)
            .map(|size| size.frames() as usize == self.block_frames)
            .unwrap_or(false)
    }
}

/// 把工程的 MIDI 摆放投影成"每轨一份已调度音符表"[ROAD-M2-005, ROAD-M2-006]。
///
/// 这是**控制线程**上的纯投影（允许分配、允许超越函数），实时侧只读结果：
///
/// ```text
/// TrackV3.clips ─► ClipPlacement ─► clip_pool[clip_id] 为 Midi ─► BTreeMap<EntityId, MidiNote>
///   │  起点 tick = placement.start_tick + note.start_tick + micro_timing_ticks
///   │  ratchet 把时值等分成 N 个脉冲（整数除法，余数不补）
///   │  probability 用 MidiNote::triggers(project.rng_seed) 做**确定性**判定
///   ▼  tick → sample（一次 f64 换算，IEEE 精确类）
/// ScheduledNote { start_sample, end_sample, phase_inc, freq_hz, gain }
/// ```
///
/// 语义裁决（本切片明确采取的口径，未在规范里定义的都登记在
/// `docs/ledger/engine-sound-notes.md` 的 needs）：
///
/// 1. **音符时值被裁剪到摆放区间** `[start_tick, start_tick + duration_ticks)`；
/// 2. **`loop_config` 不展开**（一个摆放只播一遍），坐标语义待裁决；
/// 3. **力度 0 仍然进调度表**，由 `velocity_gain(0) == 0.0` 让它静音 ——
///    这样"力度 0 不发声"是**增益路径**的判据，而不是"被调度器丢掉"的巧合；
/// 4. **静音/独奏**在构造期折算成增益门（`track_is_audible`）：
///    不可闻的轨道照常触发声部，但增益恒为 0 ⇒ 输出逐位为 0；
/// 5. **`ClipContent::Audio` 不产生声音**（采样播放/SFZ 尚未接入，见 notes）；
/// 6. 每轨超过 [`MAX_NOTES_PER_TRACK`] 的音符被丢弃并计数。
fn project_schedules(project: &YebanProjectV1) -> (BTreeMap<EntityId, NoteSchedule>, u64) {
    let sample_rate = project.audio_config.sample_rate.hz();
    #[allow(clippy::cast_precision_loss)]
    let sample_rate_f32 = sample_rate as f32;
    let samples_per_tick = crate::synth::samples_per_tick(project.bpm, sample_rate);
    let any_solo = project.tracks.values().any(|track| track.solo);
    let mut schedules: BTreeMap<EntityId, NoteSchedule> = BTreeMap::new();
    let mut dropped = 0u64;

    for (id, track) in &project.tracks {
        let audible =
            crate::synth::track_is_audible(track.mute, track.solo, track.solo_safe, any_solo);
        let gain = if audible {
            crate::synth::track_gain(track.volume_db)
        } else {
            0.0
        };
        let mut notes: Vec<ScheduledNote> = Vec::new();

        for placement in track.clips.values() {
            if placement.muted {
                continue;
            }
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let ClipContent::Midi { notes: pool } = &entry.content else {
                continue;
            };
            let placement_start = i128::from(placement.start_tick);
            let placement_end = placement_start + i128::from(placement.duration_ticks);

            for note in pool.values() {
                // 确定性概率触发：种子来自工程（`rng_seed`），与调用次数/顺序无关
                // [MODEL-AST-005, ARCH-DET-001]。
                if !note.triggers(project.rng_seed) {
                    continue;
                }
                let ratchet = u64::from(note.ratchet.unwrap_or(1).clamp(1, 16));
                let step = (note.duration_ticks / ratchet).max(1);
                let offset =
                    i128::from(note.start_tick) + i128::from(note.micro_timing_ticks.unwrap_or(0));

                for pulse in 0..ratchet {
                    #[allow(clippy::cast_possible_wrap)]
                    let raw_start = placement_start + offset + (pulse * step) as i128;
                    let start_tick = raw_start.clamp(0, placement_end).min(i128::from(u64::MAX));
                    let end_tick = (start_tick + i128::from(step))
                        .min(placement_end)
                        .min(i128::from(u64::MAX));
                    if end_tick <= start_tick {
                        continue;
                    }
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let (start_tick, end_tick) = (start_tick as u64, end_tick as u64);
                    let (Some(start_sample), Some(end_sample)) = (
                        tick_to_sample(start_tick, samples_per_tick),
                        tick_to_sample(end_tick, samples_per_tick),
                    ) else {
                        continue;
                    };
                    if end_sample <= start_sample {
                        continue;
                    }
                    if notes.len() >= MAX_NOTES_PER_TRACK {
                        dropped = dropped.saturating_add(1);
                        continue;
                    }
                    notes.push(ScheduledNote::new(
                        start_sample,
                        end_sample,
                        note.pitch,
                        note.velocity,
                        note_to_hz(f32::from(note.pitch)),
                        velocity_gain(note.velocity) * gain,
                        sample_rate_f32,
                    ));
                }
            }
        }

        // 排序 ⇒ 实时侧的单调游标成立（`BTreeMap` 顺序不等于时间顺序）。
        // 稳定排序 + 全序 key ⇒ 迭代顺序与平台无关 [MODEL-AST-003]。
        notes.sort_by_key(|note| (note.start_sample(), note.pitch(), note.end_sample()));
        schedules.insert(*id, NoteSchedule::from_sorted(notes));
    }

    (schedules, dropped)
}

/// 退役回收队列的消费端（**主线程**持有）。
///
/// 音频线程把旧快照 push 进来；主线程以 60Hz `drain` 并在这里真正 **Drop**
/// [ARCH-RT-002]。因此"释放快照"这件事永远不会发生在音频线程上。
#[derive(Debug)]
pub struct RetireQueue {
    consumer: rtrb::Consumer<Arc<EngineSnapshot>>,
    drained: u64,
    drops: u64,
}

impl RetireQueue {
    /// 当前排队待回收的条数。
    #[must_use]
    pub fn pending(&self) -> usize {
        self.consumer.slots()
    }

    /// 累计已回收（Drop）的快照数。
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.drops
    }

    /// 累计 drain 调用次数。
    #[must_use]
    pub const fn drain_calls(&self) -> u64 {
        self.drained
    }

    /// 一次性出队至多 `max` 条并 **Drop**，返回实际回收条数。
    ///
    /// 使用 `rtrb` 的**批量** API（`Consumer::read_chunk` + `IntoIterator`），
    /// 一次调用搬空整批 [ROAD-M2-007]。本函数在主线程调用，允许发生真正的 `dealloc`。
    pub fn drain(&mut self, max: usize) -> usize {
        let available = self.consumer.slots();
        let take = available.min(max);
        if take == 0 {
            return 0;
        }
        self.drained = self.drained.saturating_add(1);
        let chunk = match self.consumer.read_chunk(take) {
            Ok(chunk) => chunk,
            // slots() 与 read_chunk 之间没有别的消费者，这里理论上不可达；
            // 真发生了也只是"这一轮少回收一点"，下一轮再来 —— 不 panic。
            Err(_) => return 0,
        };
        let mut count = 0usize;
        for snapshot in chunk {
            // 主线程负责 Drop [ARCH-RT-002]：这是整个设计的目的所在。
            drop(snapshot);
            count += 1;
        }
        self.drops = self.drops.saturating_add(count as u64);
        count
    }
}

/// 建立退役队列：`Producer` 给音频线程，[`RetireQueue`] 给主线程。
#[must_use]
pub fn retire_channel(capacity: usize) -> (rtrb::Producer<Arc<EngineSnapshot>>, RetireQueue) {
    let (producer, consumer) = rtrb::RingBuffer::<Arc<EngineSnapshot>>::new(capacity.max(1));
    (
        producer,
        RetireQueue {
            consumer,
            drained: 0,
            drops: 0,
        },
    )
}

/// 快照原子交换槽的写者侧（Model 线程持有）。
///
/// 读者通过 [`SnapshotReader::attach`] 在册，并在回调线程内读 `ptr`。
pub struct SnapshotSlot {
    /// 当前快照指针（Release 写 / Acquire 读）。
    ptr: AtomicPtr<EngineSnapshot>,
    /// 当前快照的**锚**：只要槽活着，`ptr` 就永不悬垂。
    anchor: Mutex<Arc<EngineSnapshot>>,
    /// 纪元：每次 publish 后 +1。读者用它证明"我不会再碰被淘汰的快照"。
    epoch: AtomicU64,
    /// 读者已完整结束的最大纪元。
    reader_done: AtomicU64,
    /// 是否存在在册读者。没有读者时待回收项可以立即释放。
    reader_attached: AtomicBool,
    /// 已被 `ptr` 淘汰、但在读者确认之前必须保持存活的强引用。
    pending: Mutex<Vec<(u64, Arc<EngineSnapshot>)>>,
    published: AtomicU64,
}

// 线程安全说明：`SnapshotSlot` 的每个字段都是 `Send + Sync` —— `AtomicPtr`/`AtomicU64`/
// `AtomicBool` 是无条件 `Send + Sync` 的原子类型，`Mutex<T>` 在 `T: Send` 时是 `Send + Sync`。
// 因此**不需要**手写 `unsafe impl`；`mod tests` 里的 `assert_send_sync::<SnapshotSlot>()`
// 把这一点钉在编译期。
impl SnapshotSlot {
    /// 用初始快照建立交换槽。
    #[must_use]
    pub fn new(initial: EngineSnapshot) -> Arc<Self> {
        let anchor = Arc::new(initial);
        let raw = Arc::as_ptr(&anchor).cast_mut();
        Arc::new(Self {
            ptr: AtomicPtr::new(raw),
            anchor: Mutex::new(anchor),
            epoch: AtomicU64::new(0),
            reader_done: AtomicU64::new(0),
            reader_attached: AtomicBool::new(false),
            pending: Mutex::new(Vec::new()),
            published: AtomicU64::new(1),
        })
    }

    /// 发布一个新快照（**Model 线程**调用）。
    ///
    /// 旧快照的强引用被移入待回收清单，由 [`prune`](Self::prune) 在读者确认之后释放。
    /// 写者路径允许 `Mutex`（它不是实时线程）[ARCH-TOP-002]。
    pub fn publish(&self, snapshot: EngineSnapshot) {
        self.publish_arc(Arc::new(snapshot));
    }

    /// 以 `Arc` 形式发布（避免调用方多余的一次 `Arc::new`）。
    pub fn publish_arc(&self, snapshot: Arc<EngineSnapshot>) {
        let new_raw = Arc::as_ptr(&snapshot).cast_mut();
        let old = {
            let mut anchor = lock(&self.anchor);
            let old = std::mem::replace(&mut *anchor, snapshot);
            // Release: 与读者的 Acquire load 配对；且必须早于 `epoch.fetch_add`。
            self.ptr.store(new_raw, Ordering::Release);
            old
        };
        let retired_at = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        lock(&self.pending).push((retired_at, old));
        self.published.fetch_add(1, Ordering::Release);
    }

    /// 当前快照的指针（仅用于诊断/测试；正常读者走 [`SnapshotReader`]）。
    #[must_use]
    pub fn current_ptr(&self) -> *const EngineSnapshot {
        self.ptr.load(Ordering::Acquire)
    }

    /// 当前快照的 `Arc` 克隆（**非实时路径**：它会在写者锁上短暂等待）。
    #[must_use]
    pub fn current(&self) -> Arc<EngineSnapshot> {
        Arc::clone(&lock(&self.anchor))
    }

    /// 纪元计数（每次 publish +1）。
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// 累计 publish 次数。
    #[must_use]
    pub fn published(&self) -> u64 {
        self.published.load(Ordering::Acquire)
    }

    /// 写者侧待回收清单长度（诊断用；不是实时路径）。
    #[must_use]
    pub fn pending_len(&self) -> usize {
        lock(&self.pending).len()
    }

    /// 释放所有已被读者确认淘汰的强引用，返回释放条数（**主线程 60Hz** 调用）。
    ///
    /// 这是"内存回收"发生的地方；音频线程只 `push`，从不 `drop` [ARCH-RT-002]。
    pub fn prune(&self) -> usize {
        let mut pending = lock(&self.pending);
        let before = pending.len();
        if !self.reader_attached.load(Ordering::Acquire) {
            // 没有在册读者 ⇒ 没有任何在途指针解引用 ⇒ 全部可以立即释放。
            pending.clear();
            return before;
        }
        let done = self.reader_done.load(Ordering::Acquire);
        pending.retain(|(retired_at, _)| *retired_at > done);
        before - pending.len()
    }
}

/// 快照原子交换槽的读者侧（**cpal 回调线程**持有）。
///
/// 使用方式（每个渲染量子一次，见 [`crate::rt`]）：
///
/// ```ignore
/// if let Some(snapshot) = reader.begin_block() {
///     let frames = snapshot.block_frames();
///     // … 只读访问, 零分配、零锁 …
/// }
/// reader.end_block();
/// ```
pub struct SnapshotReader {
    slot: Arc<SnapshotSlot>,
    retire: rtrb::Producer<Arc<EngineSnapshot>>,
    held: Option<Arc<EngineSnapshot>>,
    /// 当前持有快照的**地址**（不保存裸指针：裸指针 `*const T` 是 `!Send`，
    /// 而整个读者必须能 move 进 cpal 的回调线程）。地址比较与指针比较语义相同，
    /// 且 `*const T as usize` 是安全转换。
    held_addr: usize,
    /// 退役队列满时的临时寄存位（仍有强引用 ⇒ 不会在音频线程释放）。
    stash: Option<Arc<EngineSnapshot>>,
    local_epoch: u64,
    switches: u64,
    stash_events: u64,
}

impl SnapshotReader {
    /// 在册化一个读者。**必须在音频线程启动前调用一次**。
    #[must_use]
    pub fn attach(slot: &Arc<SnapshotSlot>, retire: rtrb::Producer<Arc<EngineSnapshot>>) -> Self {
        // 先复位进度再登记: 顺序无关紧要（见模块文档的证明），
        // 但"先保守后激进"更不容易出错。
        slot.reader_done.store(0, Ordering::Release);
        slot.reader_attached.store(true, Ordering::Release);
        let held = Arc::clone(&lock(&slot.anchor));
        let held_addr = Arc::as_ptr(&held) as usize;
        Self {
            slot: Arc::clone(slot),
            retire,
            held: Some(held),
            held_addr,
            stash: None,
            local_epoch: 0,
            switches: 0,
            stash_events: 0,
        }
    }

    /// 本块开始时调用：必要时切换到最新快照；返回当前快照的只读引用。
    ///
    /// 实时安全：一次 `Acquire` 载入 +（仅在换快照时）一次引用计数 +1 与一次 `push`。
    /// **没有分配、没有锁、没有阻塞** [红线 7]。
    ///
    /// 若退役队列满（`stash` 也被占用），本块**放弃切换**、继续用旧快照：
    /// 宁可晚一个块切拓扑，也绝不在音频线程释放内存 [ARCH-RT-002]。
    pub fn begin_block(&mut self) -> Option<&Arc<EngineSnapshot>> {
        self.local_epoch = self.slot.epoch.load(Ordering::Acquire);
        self.flush_stash();

        let current = self.slot.ptr.load(Ordering::Acquire);
        let unchanged = current as usize == self.held_addr;
        if !current.is_null() && !unchanged && self.stash.is_none() {
            // SAFETY: `current` 来自 `Arc::as_ptr`（写者 anchor 持有的那个 `Arc`），
            // 且模块文档证明了以下不变式：
            //   1. anchor 始终持有当前快照的强引用 ⇒ 在本次 `begin_block` 期间
            //      `current` 指向的分配一定活着（引用计数 >= 1）；
            //   2. 本块读到的纪元 `e` 之后，写者绝不会把 `current` 判定为"可淘汰"，
            //      因为 `prune` 要求 `reader_done >= retired_at`，而读者要到 `end_block`
            //      才会把 `reader_done` 推到 `e`；
            //   3. 因此 `increment_strong_count` + `from_raw` 是"把已存在的强引用
            //      再复制一份"，与 `Arc::increment_strong_count` 官方文档给出的
            //      `as_ptr` → `increment_strong_count` → `from_raw` 用法完全一致，
            //      不会产生悬垂或二次释放。
            unsafe {
                Arc::increment_strong_count(current);
                let new = Arc::from_raw(current);
                let old = self.held.replace(new);
                self.held_addr = current as usize;
                self.switches = self.switches.saturating_add(1);
                if let Some(old) = old {
                    self.retire_or_stash(old);
                }
            }
        }
        self.held.as_ref()
    }

    /// 本块结束时调用：向写者公布"我已完整结束纪元 `e` 的块"。
    ///
    /// `fetch_max` 保证进度单调，即使块与块之间发生乱序重排也不会回退。
    pub fn end_block(&mut self) {
        self.slot
            .reader_done
            .fetch_max(self.local_epoch, Ordering::Release);
    }

    /// 当前持有的快照。
    #[must_use]
    pub fn held(&self) -> Option<&Arc<EngineSnapshot>> {
        self.held.as_ref()
    }

    /// 累计完成的快照切换次数。
    #[must_use]
    pub const fn switches(&self) -> u64 {
        self.switches
    }

    /// 因退役队列满而暂时寄存的次数（> 0 说明主线程 drain 不及时）。
    #[must_use]
    pub const fn stash_events(&self) -> u64 {
        self.stash_events
    }

    /// 把旧快照推进退役队列；队列满则寄存到 `stash`（下次块边界重试）。
    fn retire_or_stash(&mut self, old: Arc<EngineSnapshot>) {
        match self.retire.push(old) {
            Ok(()) => {}
            Err(rtrb::PushError::Full(arc)) => {
                self.stash = Some(arc);
                self.stash_events = self.stash_events.saturating_add(1);
            }
        }
    }

    /// 冲刷寄存位（块边界调用，属于实时路径）。
    fn flush_stash(&mut self) {
        if let Some(pending) = self.stash.take() {
            match self.retire.push(pending) {
                Ok(()) => {}
                Err(rtrb::PushError::Full(arc)) => {
                    self.stash = Some(arc);
                }
            }
        }
    }
}

impl Drop for SnapshotReader {
    fn drop(&mut self) {
        // 读者退场: 之后不会再有指针解引用 ⇒ 允许写者释放全部待回收项。
        self.slot.reader_done.store(u64::MAX, Ordering::Release);
        self.slot.reader_attached.store(false, Ordering::Release);
        // 寄存位里的旧快照在这里释放 —— 此时已经不在音频线程上（读者已 Drop），
        // 因此不违反"音频线程零释放"。
        self.stash = None;
    }
}

/// 取写者侧锁；中毒时取回内部数据（写者路径上宁可继续也不 panic [红线 7 的精神]）。
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use yeban_model::{RoutingEdge, RoutingKind};

    fn simple_project() -> YebanProjectV1 {
        let master = EntityId::new();
        let track = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![track, master],
            ..RoutingGraph::default()
        };
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: track,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        let mut tracks = BTreeMap::new();
        tracks.insert(
            track,
            TrackV3 {
                id: track,
                volume_db: -6.0,
                ..TrackV3::default()
            },
        );
        YebanProjectV1 {
            master_bus_track_id: master,
            routing_graph: routing,
            tracks,
            ..YebanProjectV1::default()
        }
    }

    fn snapshot_with(frames: usize, revision: u64) -> EngineSnapshot {
        let master = EntityId::new();
        let mut routing = RoutingGraph::default();
        routing.nodes.push(master);
        EngineSnapshot::from_parts(
            revision,
            48_000,
            frames,
            2,
            master,
            BTreeMap::new(),
            &routing,
            &LatencyTable::new(),
        )
        .expect("单节点图合法")
    }

    #[test]
    fn project_projection_carries_track_params_and_pdc() {
        let project = simple_project();
        let snapshot = EngineSnapshot::from_project(&project, 7).expect("投影成功");
        assert_eq!(snapshot.revision(), 7);
        assert_eq!(snapshot.sample_rate(), 48_000);
        assert_eq!(snapshot.block_frames(), 256, "BlockSize 默认是 256");
        assert!(snapshot.block_size_matches_enum());
        assert_eq!(snapshot.master(), project.master_bus_track_id);
        assert_eq!(snapshot.tracks().len(), 1);
        let params = snapshot
            .track(&project.tracks.keys().next().copied().unwrap())
            .expect("轨道参数已投影");
        assert_eq!(params.volume_db(), -6.0);
        assert_eq!(params.pan(), 0.0);
        assert!(!params.is_muted());
        assert_eq!(params.device_count(), 0);
        assert_eq!(snapshot.pdc().total_latency(), 0);
        assert_eq!(snapshot.pdc().compensation(&snapshot.master()), Some(0));
    }

    #[test]
    fn project_without_master_node_is_rejected_explicitly() {
        // 默认空工程的 master_bus_track_id 是 nil ULID
        let project = YebanProjectV1::default();
        assert_eq!(
            EngineSnapshot::from_project(&project, 0),
            Err(SnapshotError::NoMasterBus)
        );

        // master id 存在但不在路由节点里 → 同样是 NoMasterBus
        let mut project = simple_project();
        let master = project.master_bus_track_id;
        project.routing_graph.nodes.retain(|node| *node != master);
        assert_eq!(
            EngineSnapshot::from_project(&project, 0),
            Err(SnapshotError::NoMasterBus)
        );
    }

    #[test]
    fn cyclic_routing_propagates_pdc_error() {
        let a = EntityId::new();
        let b = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![a, b],
            ..RoutingGraph::default()
        };
        for (source, destination) in [(a, b), (b, a)] {
            let id = EntityId::new();
            routing.edges.insert(
                id,
                RoutingEdge {
                    id,
                    source_node: source,
                    destination_node: destination,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            );
        }
        let project = YebanProjectV1 {
            master_bus_track_id: a,
            routing_graph: routing,
            ..YebanProjectV1::default()
        };
        match EngineSnapshot::from_project(&project, 0) {
            Err(SnapshotError::Pdc(PdcError::Cycle { nodes })) => {
                let expected: BTreeSet<EntityId> = [a, b].into_iter().collect();
                assert_eq!(nodes.into_iter().collect::<BTreeSet<_>>(), expected);
            }
            other => panic!("预期 Cycle 错误, 实际 {other:?}"),
        }
    }

    /// 判据 (a)：**退役队列不泄漏** —— 旧快照在 drain 之后引用计数归零。
    ///
    /// 用 `Weak::upgrade()` 而不是 `Arc::strong_count`，因为前者是"对象是否还活着"的
    /// 直接证据：`strong_count == 0` 也可能只是暂时读到了别的计数。
    #[test]
    fn retired_snapshots_are_dropped_after_drain_and_reference_count_hits_zero() {
        let (mut producer, mut queue) = retire_channel(8);
        let snapshot = Arc::new(snapshot_with(128, 1));
        let weak = Arc::downgrade(&snapshot);

        // 音频线程侧的动作：把旧快照 move 进队列（不清引用）
        producer.push(Arc::clone(&snapshot)).expect("队列有空间");
        assert_eq!(Arc::strong_count(&snapshot), 2);
        drop(snapshot); // 音频线程不再持有它
        assert!(weak.upgrade().is_some(), "队列还持有强引用");

        assert_eq!(queue.pending(), 1);
        assert_eq!(queue.dropped(), 0);
        assert_eq!(queue.drain(8), 1, "主线程抽取 1 条");
        assert!(
            weak.upgrade().is_none(),
            "drain 之后强引用必须归零 —— 否则就是泄漏"
        );
        assert_eq!(queue.dropped(), 1);
        assert_eq!(queue.pending(), 0);
        assert_eq!(queue.drain(8), 0, "空队列 drain 返回 0");
    }

    /// 判据 (a) 的边界：队列满时音频线程**不释放**内存，而是寄存到 `stash`，
    /// 等主线程腾出空间后再推入 —— 并且最终仍会归零。
    #[test]
    fn full_retire_queue_never_frees_on_the_audio_thread() {
        let (producer, mut queue) = retire_channel(1);
        let slot = SnapshotSlot::new(snapshot_with(128, 0));
        // 初代快照的唯一强引用由 anchor 持有；这里只留一个 Weak 用于事后判断死活。
        let initial_weak = Arc::downgrade(&slot.current());
        let mut reader = SnapshotReader::attach(&slot, producer);

        let mut weaks = Vec::new();
        for revision in 1..=4u64 {
            slot.publish(snapshot_with(128, revision));
            let held = reader.begin_block().expect("块内总有快照");
            weaks.push(Arc::downgrade(held));
            reader.end_block();
        }
        assert!(reader.switches() >= 1, "至少完成一次快照切换");
        assert!(
            reader.stash_events() > 0,
            "容量 1 的退役队列必然触发寄存 —— 这正是「音频线程不 free」的直接证据"
        );
        // **关键断言**: 主线程 drain 之前, 所有音频线程曾经持有的快照都还活着
        // ⇒ 音频线程上没有发生任何 Drop [ARCH-RT-002]。
        assert!(
            initial_weak.upgrade().is_some(),
            "主线程抽取之前, 被替换的初代快照必须仍然存活"
        );
        for weak in &weaks {
            assert!(weak.upgrade().is_some(), "音频线程不得释放任何快照");
        }

        // 主线程以 60Hz 节奏抽干
        let mut drained = 0;
        for _ in 0..16 {
            drained += queue.drain(4);
            slot.prune();
        }
        assert!(drained >= 1, "主线程必须真的回收到了东西, 实际 {drained}");
        assert!(queue.dropped() >= 1);

        // 回收之后: 初代快照（既不在 anchor、也不再被读者持有）必须归零
        assert!(
            initial_weak.upgrade().is_none(),
            "drain + prune 之后初代快照的强引用必须归零 —— 否则就是泄漏"
        );
        // 读者当前持有的那个仍必须活着（它自己拿着强引用）
        assert!(reader.held().is_some());
    }

    /// 判据 (a) 的第二面：**写者侧**待回收清单也必须归零（否则 `publish` 会无限泄漏）。
    #[test]
    fn writer_pending_list_is_pruned_after_reader_progress() {
        let slot = SnapshotSlot::new(snapshot_with(128, 0));
        let (producer, mut queue) = retire_channel(64);
        let mut reader = SnapshotReader::attach(&slot, producer);

        for revision in 1..=5u64 {
            slot.publish(snapshot_with(128, revision));
        }
        assert_eq!(slot.pending_len(), 5, "5 次 publish 淘汰了 5 个快照");

        // 在读者推进之前, 一项都不许释放
        assert_eq!(slot.prune(), 0, "读者还没确认, 写者不能释放任何一项");

        // 读者跑两个块边界
        let _ = reader.begin_block();
        reader.end_block();
        let _ = reader.begin_block();
        reader.end_block();

        let freed = slot.prune();
        assert!(
            freed >= 4,
            "读者推进后写者应释放绝大多数待回收项, 实际 {freed}"
        );
        assert!(slot.pending_len() <= 1, "残余至多一项(最新的那次淘汰)");

        // 读者 push 进队列的那个旧快照也必须被主线程回收掉
        let mut reclaimed = 0;
        for _ in 0..8 {
            reclaimed += queue.drain(8);
            slot.prune();
        }
        assert!(reclaimed > 0, "退役队列里的条目必须被主线程取走并 Drop");
        assert_eq!(slot.pending_len(), 0, "写者侧清单最终必须清空");
    }

    /// 读者退场后，写者可以立即释放全部待回收项（否则关流时会泄漏）。
    #[test]
    fn dropping_the_reader_releases_writer_side_backlog() {
        let slot = SnapshotSlot::new(snapshot_with(128, 0));
        let (producer, _queue) = retire_channel(64);
        {
            let mut reader = SnapshotReader::attach(&slot, producer);
            for revision in 1..=3u64 {
                slot.publish(snapshot_with(128, revision));
            }
            // 读者还**没有**跑完任何块 ⇒ reader_done == 0 ⇒ 写者一项都不许释放。
            assert_eq!(slot.pending_len(), 3);
            assert_eq!(slot.prune(), 0, "读者在册且未完成任何块时不许释放");

            // 跑一个块：读者现在持有最新快照, 但这个测试关心的是"退场后清空"
            let _ = reader.begin_block();
            reader.end_block();
            assert!(slot.pending_len() >= 1);
        }
        // 读者 Drop ⇒ reader_attached = false ⇒ 所有待回收项立即可以释放
        let before = slot.pending_len();
        assert!(before >= 1, "退场时应当还有待回收项可释放, 实际 {before}");
        assert_eq!(slot.prune(), before);
        assert_eq!(slot.pending_len(), 0, "读者退场 ⇒ 全部待回收项可释放");
    }

    /// 快照是真正的"不可变 + 可共享"：多个 `Arc` 克隆看到同一份数据，
    /// 且跨线程发送是合法的（`Send + Sync` 编译期断言）。
    #[test]
    fn snapshot_is_immutable_shareable_and_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        fn assert_send<T: Send>() {}
        assert_send_sync::<EngineSnapshot>();
        assert_send_sync::<SnapshotSlot>();
        assert_send_sync::<Arc<EngineSnapshot>>();
        // 读者必须能 move 进 cpal 的回调闭包（`D: Send + 'static`）——
        // 这正是"裸指针字段会让它变成 !Send"那个坑的编译期判据。
        assert_send::<SnapshotReader>();

        let snapshot = Arc::new(snapshot_with(128, 3));
        let clone = Arc::clone(&snapshot);
        let handle = std::thread::spawn(move || clone.revision());
        assert_eq!(handle.join().expect("线程不 panic"), 3);
        assert_eq!(snapshot.revision(), 3);
        // 只有 getter、没有 setter：字段全私有，编译期即不可变。
        assert_eq!(snapshot.channels(), 2);
    }

    /// 判据 (i)：**延迟的唯一来源是模型的 `DeviceDefinition::latency_samples`**
    /// [ARCH-PDC-001]，`yeban-engine` 不再自造第二个事实源。
    ///
    /// 构造两条并联支路：`heavy` 的设备链有 32 采样延迟，`light` 没有；
    /// PDC 必须把 `light` 补 32 采样，`heavy` 补 0。
    #[test]
    fn pdc_reads_device_latency_samples_from_the_model() {
        use yeban_model::{DeviceDefinition, DeviceKind};

        let heavy = EntityId::new();
        let light = EntityId::new();
        let master = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![heavy, light, master],
            ..RoutingGraph::default()
        };
        for (source, destination) in [(heavy, master), (light, master)] {
            let id = EntityId::new();
            routing.edges.insert(
                id,
                RoutingEdge {
                    id,
                    source_node: source,
                    destination_node: destination,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            );
        }
        let mut tracks = BTreeMap::new();
        tracks.insert(
            heavy,
            TrackV3 {
                id: heavy,
                devices: vec![DeviceDefinition {
                    kind: DeviceKind::ExternalEffect,
                    latency_samples: 32,
                    ..DeviceDefinition::default()
                }],
                ..TrackV3::default()
            },
        );
        tracks.insert(
            light,
            TrackV3 {
                id: light,
                ..TrackV3::default()
            },
        );
        let project = YebanProjectV1 {
            master_bus_track_id: master,
            routing_graph: routing,
            tracks,
            ..YebanProjectV1::default()
        };

        let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影成功");
        assert_eq!(
            snapshot.pdc().total_latency(),
            32,
            "L_max 由 32 采样的设备决定"
        );
        assert_eq!(snapshot.pdc().compensation(&heavy), Some(0));
        assert_eq!(snapshot.pdc().compensation(&light), Some(32));
        // 投影出来的轨道参数也带上该延迟（供实时侧做诊断/UI）
        assert_eq!(
            snapshot.track(&heavy).map(|p| p.latency_samples()),
            Some(32)
        );
        assert_eq!(snapshot.track(&light).map(|p| p.latency_samples()), Some(0));
    }

    /// 旁通设备的延迟**不计入**（它不在信号路径上）。
    #[test]
    fn bypassed_devices_do_not_contribute_latency() {
        use yeban_model::{DeviceDefinition, DeviceKind};

        let track = EntityId::new();
        let master = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![track, master],
            ..RoutingGraph::default()
        };
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: track,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        let mut tracks = BTreeMap::new();
        tracks.insert(
            track,
            TrackV3 {
                id: track,
                devices: vec![
                    DeviceDefinition {
                        kind: DeviceKind::ExternalEffect,
                        latency_samples: 64,
                        bypassed: true,
                        ..DeviceDefinition::default()
                    },
                    DeviceDefinition {
                        kind: DeviceKind::InternalEffect,
                        latency_samples: 8,
                        ..DeviceDefinition::default()
                    },
                ],
                ..TrackV3::default()
            },
        );
        let project = YebanProjectV1 {
            master_bus_track_id: master,
            routing_graph: routing,
            tracks,
            ..YebanProjectV1::default()
        };
        let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影成功");
        assert_eq!(
            snapshot.pdc().total_latency(),
            8,
            "只有未旁通的那个设备贡献延迟"
        );
    }

    #[test]
    fn block_frames_that_are_not_a_spec_enum_value_are_reported() {
        let snapshot = snapshot_with(200, 0);
        assert!(!snapshot.block_size_matches_enum());
        let ok = snapshot_with(64, 0);
        assert!(ok.block_size_matches_enum());
    }
}
