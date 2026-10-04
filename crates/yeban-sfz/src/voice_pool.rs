//! 预分配声部池与确定性声部窃取 (Voice Stealing)。
//!
//! 规范来源 (Normative): `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2
//! [ARCH-RT-001]（预分配声部池，默认 512 / 可配 1024）与 [ARCH-RT-004]：
//!
//! > 当多音轨音符并发超过声部池上限时，激活确定性声部窃取算法：优先窃取处于
//! > Release 阶段尾部、振幅能量最低（<-60dBFS）或最早被触发的声音；窃取瞬间对被终止
//! > 声部强制应用 3ms 快速指数衰减微淡出包络，彻底杜绝爆音。
//!
//! 路线图 `ROAD-M2-006` 还要求 5.0ms 升余弦自愈平滑 —— 那是**渲染器**的职责；
//! 本模块只负责「谁被窃取、何时、以什么包络」，因此**不依赖 `yeban-dsp`**（避免跨线耦合）。
//!
//! # 能量注入
//!
//! 「振幅能量」由调用方通过 [`VoicePool::set_level`] / [`VoicePool::refresh_levels`]
//! （闭包注入）写入瞬时电平（dBFS）。本模块不猜测电平，只看注入值。
//!
//! # 与规范的一处刻意偏离（需要人类裁决，见 notes）
//!
//! 固定容量池没有「额外 slack 槽位」同时容纳「正在淡出的旧声部」和「刚开始的新声部」。
//! 因此本实现采用**立即接管**：`note_on` 超限时窃取 victim 的槽位给新声部，并把
//! victim 句柄交还调用方，由渲染器对 victim 尚未播完的残余输出应用
//! [`StealFade`]（3ms 指数衰减）。池内另提供 [`VoicePool::retire`] 路径：
//! 声部结束/被窃取后进入 `retiring` 状态，`process` 走完 3ms 淡出即把槽位**归零**
//! 并标记可复用。

use crate::error::SfzError;

/// 默认声部容量 (`ARCH-RT-001`)。
pub const DEFAULT_VOICE_CAPACITY: usize = 512;
/// 最大可配置声部容量 (`ARCH-RT-001`)。
pub const MAX_VOICE_CAPACITY: usize = 1024;
/// 窃取淡出时长 (ms)，`ARCH-RT-004` 规定的 3ms。
pub const STEAL_FADE_MILLIS: f32 = 3.0;
/// 指数淡出终点增益，对应约 -60 dBFS。
pub const STEAL_FADE_FLOOR: f32 = 1.0e-3;
/// 「能量过低」判定阈值 (dBFS)，`ARCH-RT-004` 的 `-60dBFS`。
pub const SILENT_DBFS: f32 = -60.0;
/// 淡出采样数上限（防止非法采样率制造天文数字）。
const MAX_FADE_SAMPLES: u32 = 1 << 20;

/// 3ms 指数淡出包络（`ARCH-RT-004`）。
///
/// `gain_at(0) == 1.0`，`gain_at(samples) == 0.0`，中间按 `FLOOR^(n/N)` 指数衰减。
/// 采样数按采样率计算：48kHz 下为 144 采样点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StealFade {
    samples: u32,
}

impl StealFade {
    /// 按采样率构造（非法采样率回退 48kHz；结果永远落在 `1..=MAX_FADE_SAMPLES`）。
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let safe_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            48_000.0
        };
        let raw = (safe_rate * STEAL_FADE_MILLIS / 1000.0).ceil();
        let samples = if raw.is_finite() && raw >= 1.0 {
            (raw as u32).min(MAX_FADE_SAMPLES)
        } else {
            1
        };
        Self { samples }
    }

    /// 淡出总采样数（恒 ≥ 1）。
    #[must_use]
    pub fn samples(self) -> u32 {
        self.samples
    }

    /// 第 `sample_index` 个采样的包络增益。
    #[must_use]
    pub fn gain_at(self, sample_index: u32) -> f32 {
        if sample_index == 0 {
            return 1.0;
        }
        if sample_index >= self.samples {
            return 0.0;
        }
        let exponent = f64::from(sample_index) / f64::from(self.samples);
        f64::from(STEAL_FADE_FLOOR).powf(exponent) as f32
    }

    /// 给定已播放采样数，淡出是否已经结束。
    #[must_use]
    pub fn is_complete(self, elapsed: u32) -> bool {
        elapsed >= self.samples
    }
}

/// 声部包络阶段。由渲染器通过 [`VoicePool::set_stage`] 更新。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceStage {
    /// 起音阶段。
    Attack,
    /// 延音阶段（按住）。
    Sustain,
    /// 释音阶段（note-off 之后）。
    Release,
}

/// 声部句柄：`index` + `generation`，`generation` 让被回收过的槽位上的旧句柄失效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoiceHandle {
    /// 槽位下标（`0..capacity`）。
    pub index: u32,
    /// 槽位代数；每次分配 / 回收都会递增。
    pub generation: u32,
}

/// 一个槽位的只读快照（用于诊断与测试；`active == false` 时其余字段均为零值）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceInfo {
    /// 槽位下标。
    pub index: usize,
    /// 是否被占用。
    pub active: bool,
    /// 是否正在走 3ms 淡出（[`VoicePool::retire`] 之后）。
    pub retiring: bool,
    /// 触发音符。
    pub note: u8,
    /// 触发力度。
    pub velocity: u8,
    /// 包络阶段。
    pub stage: VoiceStage,
    /// 注入的瞬时电平 (dBFS)。
    pub level_db: f32,
    /// 触发序号（越大越晚触发）。
    pub order: u64,
    /// 剩余淡出采样数。
    pub fade_remaining: u32,
    /// 剩余窃取淡入采样数。
    pub fade_in_remaining: u32,
}

/// `note_on` 的判决结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteOnOutcome {
    /// 有空闲槽位，新声部直接开始。
    Started(VoiceHandle),
    /// 池已满：按确定性规则窃取了 `victim`，新声部在同一槽位立即开始。
    ///
    /// 渲染器应当用 [`StealFade`] 对 `victim` 的残余输出做 3ms 指数淡出。
    Stolen {
        /// 被窃取的旧声部句柄（已失效）。
        victim: VoiceHandle,
        /// 新声部句柄。
        started: VoiceHandle,
    },
}

impl NoteOnOutcome {
    /// 新声部的句柄。
    #[must_use]
    pub fn started(self) -> VoiceHandle {
        match self {
            Self::Started(handle) => handle,
            Self::Stolen { started, .. } => started,
        }
    }

    /// 被窃取的旧声部句柄（若有）。
    #[must_use]
    pub fn victim(self) -> Option<VoiceHandle> {
        match self {
            Self::Started(_) => None,
            Self::Stolen { victim, .. } => Some(victim),
        }
    }
}

/// 一个声部槽位。全部是 `Copy` 字段：回收 = 直接覆盖，**不触碰堆**。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Slot {
    generation: u32,
    active: bool,
    retiring: bool,
    note: u8,
    velocity: u8,
    stage: VoiceStage,
    level_db: f32,
    order: u64,
    fade_remaining: u32,
    fade_in_remaining: u32,
}

impl Slot {
    /// 归零的空闲槽位（`ARCH-RT-004`：「淡出后声部状态归零可复用」）。
    fn free(generation: u32) -> Self {
        Self {
            generation,
            active: false,
            retiring: false,
            note: 0,
            velocity: 0,
            stage: VoiceStage::Attack,
            level_db: f32::NEG_INFINITY,
            order: 0,
            fade_remaining: 0,
            fade_in_remaining: 0,
        }
    }
}

/// 固定容量的预分配声部池。
///
/// 容量在构造时确定；`note_on` / `process` / `retire` / `finish` 全部**零堆分配**，
/// 可在实时音频回调里调用 [ARCH-RT-001]。
#[derive(Debug, Clone)]
pub struct VoicePool {
    slots: Vec<Slot>,
    order_counter: u64,
    steal_count: u64,
    last_stolen: Option<VoiceHandle>,
    fade: StealFade,
}

impl Default for VoicePool {
    fn default() -> Self {
        Self::build(DEFAULT_VOICE_CAPACITY, 48_000.0)
    }
}

impl VoicePool {
    /// 以指定容量与采样率构造；容量必须落在 `1..=MAX_VOICE_CAPACITY`。
    pub fn new(capacity: usize, sample_rate: f32) -> Result<Self, SfzError> {
        if capacity == 0 || capacity > MAX_VOICE_CAPACITY {
            return Err(SfzError::InvalidVoiceCapacity {
                requested: capacity,
                max: MAX_VOICE_CAPACITY,
            });
        }
        Ok(Self::build(capacity, sample_rate))
    }

    /// 内部构造：容量已经过校验 / 夹取，**不会失败**（因此 `Default` 不需要 `unwrap`）。
    fn build(capacity: usize, sample_rate: f32) -> Self {
        let capacity = capacity.clamp(1, MAX_VOICE_CAPACITY);
        Self {
            slots: (0..capacity).map(|_| Slot::free(1)).collect(),
            order_counter: 0,
            steal_count: 0,
            last_stolen: None,
            fade: StealFade::new(sample_rate),
        }
    }

    /// 容量（声部上限）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// 当前占用的槽位数（含正在淡出的）。
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.active).count()
    }

    /// 当前空闲槽位数。
    #[must_use]
    pub fn free_count(&self) -> usize {
        self.capacity() - self.active_count()
    }

    /// 累计发生过的窃取次数。
    #[must_use]
    pub fn steal_count(&self) -> u64 {
        self.steal_count
    }

    /// 最近一次被窃取的声部句柄。
    #[must_use]
    pub fn last_stolen(&self) -> Option<VoiceHandle> {
        self.last_stolen
    }

    /// 当前使用的 3ms 淡出包络。
    #[must_use]
    pub fn steal_fade(&self) -> StealFade {
        self.fade
    }

    /// 更新采样率（只影响此后开始的淡出）。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.fade = StealFade::new(sample_rate);
    }

    /// 触发一个音符。池满时按 [ARCH-RT-004] 的确定性规则窃取。
    ///
    /// **零分配**。窃取顺序见 [`VoicePool::select_victim`]。
    pub fn note_on(&mut self, note: u8, velocity: u8, level_db: f32) -> NoteOnOutcome {
        if let Some(index) = self.slots.iter().position(|slot| !slot.active) {
            return NoteOnOutcome::Started(self.activate(index, note, velocity, level_db, 0));
        }
        // 池已满：确定性窃取。（容量 ≥ 1 保证此处必然存在 active 槽位。）
        let index = self.select_victim();
        let victim = self.slots.get(index).map_or(
            VoiceHandle {
                index: 0,
                generation: 0,
            },
            |slot| VoiceHandle {
                index: index as u32,
                generation: slot.generation,
            },
        );
        let fade_in = self.fade.samples();
        let started = self.activate(index, note, velocity, level_db, fade_in);
        self.steal_count = self.steal_count.wrapping_add(1);
        self.last_stolen = Some(victim);
        NoteOnOutcome::Stolen { victim, started }
    }

    /// 确定性窃取目标（**同一输入序列永远同一结果** [ARCH-DET-001]）。
    ///
    /// 排序键（字典序，取最小者）：
    /// 1. 层级：`0` = 正在淡出或处于 Release，`1` = 注入电平 < [`SILENT_DBFS`]，`2` = 其它；
    /// 2. 注入电平最小者；
    /// 3. 触发序号最早者；
    /// 4. 槽位下标最小者（彻底打破平局，保证全序）。
    ///
    /// 全程线性扫描、零分配。
    #[must_use]
    pub fn select_victim(&self) -> usize {
        let mut best: Option<(u8, f32, u64, u32)> = None;
        let mut best_index = 0usize;
        for (index, slot) in self.slots.iter().enumerate() {
            if !slot.active {
                continue;
            }
            let key = victim_key(slot, index);
            let take = match best {
                None => true,
                Some(current) => key < current,
            };
            if take {
                best = Some(key);
                best_index = index;
            }
        }
        best_index
    }

    /// 立即设置某个活跃声部的瞬时电平 (dBFS)（能量注入）。
    pub fn set_level(&mut self, handle: VoiceHandle, level_db: f32) -> Result<(), SfzError> {
        self.slot_mut(handle)?.level_db = sanitize_level(level_db);
        Ok(())
    }

    /// 用闭包批量刷新所有活跃声部的瞬时电平（能量注入，零分配）。
    pub fn refresh_levels(&mut self, mut level_of: impl FnMut(usize) -> f32) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.active {
                slot.level_db = sanitize_level(level_of(index));
            }
        }
    }

    /// 设置包络阶段。
    pub fn set_stage(&mut self, handle: VoiceHandle, stage: VoiceStage) -> Result<(), SfzError> {
        let slot = self.slot_mut(handle)?;
        slot.stage = stage;
        slot.retiring = false;
        Ok(())
    }

    /// note-off：进入 Release 阶段（真正回收由渲染器调用 [`VoicePool::retire`] 或
    /// [`VoicePool::finish`] 触发）。
    pub fn note_off(&mut self, handle: VoiceHandle) -> Result<(), SfzError> {
        self.set_stage(handle, VoiceStage::Release)
    }

    /// 开始 3ms 指数淡出。淡出走完后 `process` 会把槽位归零并标记可复用。
    pub fn retire(&mut self, handle: VoiceHandle) -> Result<StealFade, SfzError> {
        let fade = self.fade;
        let slot = self.slot_mut(handle)?;
        slot.retiring = true;
        slot.stage = VoiceStage::Release;
        slot.fade_remaining = fade.samples();
        slot.fade_in_remaining = 0;
        Ok(fade)
    }

    /// 立即释放槽位（渲染器确认包络已结束时使用）。槽位状态归零、代数递增。
    pub fn finish(&mut self, handle: VoiceHandle) -> Result<(), SfzError> {
        let slot = self.slot_mut(handle)?;
        let next = slot.generation.wrapping_add(1);
        *slot = Slot::free(next);
        Ok(())
    }

    /// 推进 `frames` 个采样：推进窃取淡入，并回收淡出已完成的槽位。
    ///
    /// **零分配**，实时安全。
    pub fn process(&mut self, frames: u32) {
        for slot in &mut self.slots {
            if !slot.active {
                continue;
            }
            slot.fade_in_remaining = slot.fade_in_remaining.saturating_sub(frames);
            if slot.retiring {
                slot.fade_remaining = slot.fade_remaining.saturating_sub(frames);
                if slot.fade_remaining == 0 {
                    let next = slot.generation.wrapping_add(1);
                    *slot = Slot::free(next);
                }
            }
        }
    }

    /// 查询句柄对应的活跃声部。
    #[must_use]
    pub fn voice(&self, handle: VoiceHandle) -> Option<VoiceInfo> {
        let slot = self.slots.get(handle.index as usize)?;
        if !slot.active || slot.generation != handle.generation {
            return None;
        }
        Some(voice_info(handle.index as usize, slot))
    }

    /// 按下标读取槽位快照（含空闲槽位，便于断言「归零可复用」）。
    #[must_use]
    pub fn voice_at(&self, index: usize) -> Option<VoiceInfo> {
        self.slots.get(index).map(|slot| voice_info(index, slot))
    }

    fn slot_mut(&mut self, handle: VoiceHandle) -> Result<&mut Slot, SfzError> {
        match self.slots.get_mut(handle.index as usize) {
            Some(slot) if slot.active && slot.generation == handle.generation => Ok(slot),
            _ => Err(SfzError::StaleVoiceHandle),
        }
    }

    /// 激活一个槽位（内部：容量不变量保证 `index` 有效）。
    fn activate(
        &mut self,
        index: usize,
        note: u8,
        velocity: u8,
        level_db: f32,
        fade_in: u32,
    ) -> VoiceHandle {
        self.order_counter = self.order_counter.wrapping_add(1);
        let order = self.order_counter;
        let Some(slot) = self.slots.get_mut(index) else {
            // 不可达：容量 ≥ 1 且 index 来自空闲槽位 / select_victim。
            // 返回一个永远不会命中的 stale 句柄（代数 0 不会被分配）。
            return VoiceHandle {
                index: 0,
                generation: 0,
            };
        };
        let generation = slot.generation.wrapping_add(1);
        *slot = Slot {
            generation,
            active: true,
            retiring: false,
            note: note.min(127),
            velocity: velocity.min(127),
            stage: VoiceStage::Attack,
            level_db: sanitize_level(level_db),
            order,
            fade_remaining: 0,
            fade_in_remaining: fade_in,
        };
        VoiceHandle {
            index: index as u32,
            generation,
        }
    }
}

fn voice_info(index: usize, slot: &Slot) -> VoiceInfo {
    VoiceInfo {
        index,
        active: slot.active,
        retiring: slot.retiring,
        note: slot.note,
        velocity: slot.velocity,
        stage: slot.stage,
        level_db: slot.level_db,
        order: slot.order,
        fade_remaining: slot.fade_remaining,
        fade_in_remaining: slot.fade_in_remaining,
    }
}

/// 窃取排序键：`(层级, 电平, 触发序号, 槽位下标)`。
fn victim_key(slot: &Slot, index: usize) -> (u8, f32, u64, u32) {
    let tier = if slot.retiring || slot.stage == VoiceStage::Release {
        0
    } else if slot.level_db < SILENT_DBFS {
        1
    } else {
        2
    };
    (tier, slot.level_db, slot.order, index as u32)
}

/// 把非有限电平压成 `-inf`，保证排序键是全序（确定性）。
fn sanitize_level(level_db: f32) -> f32 {
    if level_db.is_finite() {
        level_db
    } else {
        f32::NEG_INFINITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_never_exceeded() {
        let mut pool = VoicePool::new(8, 48_000.0).expect("valid capacity");
        for note in 0..200u8 {
            pool.note_on(note % 128, 100, -12.0);
            assert!(
                pool.active_count() <= pool.capacity(),
                "active {} > capacity {}",
                pool.active_count(),
                pool.capacity()
            );
        }
        assert_eq!(pool.capacity(), 8);
        assert_eq!(pool.active_count(), 8);
        assert_eq!(pool.free_count(), 0);
    }

    #[test]
    fn steal_order_is_deterministic_across_runs() {
        let script: Vec<(u8, f32, VoiceStage)> = vec![
            (60, -6.0, VoiceStage::Attack),
            (61, -20.0, VoiceStage::Sustain),
            (62, -70.0, VoiceStage::Sustain),
            (63, -12.0, VoiceStage::Release),
        ];
        let run = || {
            let mut pool = VoicePool::new(4, 48_000.0).expect("valid capacity");
            let mut log = Vec::new();
            for (note, level, stage) in &script {
                let outcome = pool.note_on(*note, 100, *level);
                if let Some(handle) = outcome.victim() {
                    log.push((handle.index, handle.generation));
                }
                if let NoteOnOutcome::Started(handle) = outcome {
                    pool.set_stage(handle, *stage).expect("live handle");
                }
            }
            // 第 5 个音符必然触发窃取：Release 的 63（tier 0）。
            let outcome = pool.note_on(64, 100, -3.0);
            log.push((outcome.victim().expect("stolen").index, 0));
            (log, pool.steal_count(), pool.last_stolen())
        };
        let first = run();
        let second = run();
        assert_eq!(first, second);
        // 明确的期望：victim 是 Release 的那个槽位（下标 3）。
        assert_eq!(first.0, vec![(3, 0)]);
    }

    #[test]
    fn quiet_voice_is_stolen_before_loud_ones() {
        let mut pool = VoicePool::new(3, 48_000.0).expect("valid capacity");
        pool.note_on(60, 100, -6.0);
        pool.note_on(61, 100, -70.0);
        pool.note_on(62, 100, -6.0);
        let outcome = pool.note_on(63, 100, -6.0);
        assert_eq!(outcome.victim().map(|handle| handle.index), Some(1));
    }

    #[test]
    fn earliest_voice_is_stolen_when_all_else_equal() {
        let mut pool = VoicePool::new(3, 48_000.0).expect("valid capacity");
        pool.note_on(60, 100, -6.0);
        pool.note_on(61, 100, -6.0);
        pool.note_on(62, 100, -6.0);
        let outcome = pool.note_on(63, 100, -6.0);
        assert_eq!(outcome.victim().map(|handle| handle.index), Some(0));
    }

    #[test]
    fn retire_fade_zeroes_the_slot_and_makes_it_reusable() {
        let mut pool = VoicePool::new(2, 48_000.0).expect("valid capacity");
        let handle = pool.note_on(60, 100, -6.0).started();
        let fade = pool.retire(handle).expect("live handle");
        assert_eq!(fade.samples(), 144, "3ms @ 48kHz");
        assert!(pool.voice(handle).expect("still active").retiring);

        pool.process(fade.samples() - 1);
        assert!(pool.voice(handle).is_some(), "fade not finished yet");
        pool.process(1);
        assert!(pool.voice(handle).is_none(), "stale after reclamation");

        let zeroed = pool.voice_at(handle.index as usize).expect("slot exists");
        assert!(!zeroed.active);
        assert_eq!(zeroed.note, 0);
        assert_eq!(zeroed.order, 0);
        assert_eq!(zeroed.fade_remaining, 0);
        assert_eq!(zeroed.level_db, f32::NEG_INFINITY);

        let reused = pool.note_on(72, 100, -3.0);
        assert_eq!(reused.started().index, handle.index);
        assert!(matches!(reused, NoteOnOutcome::Started(_)));
    }

    #[test]
    fn steal_fade_envelope_decays_exponentially_to_silence() {
        let fade = StealFade::new(48_000.0);
        assert_eq!(fade.samples(), 144);
        assert_eq!(fade.gain_at(0), 1.0);
        assert_eq!(fade.gain_at(144), 0.0);
        let mut previous = 1.0f32;
        for index in 1..fade.samples() {
            let gain = fade.gain_at(index);
            assert!(gain < previous, "envelope must be strictly decreasing");
            previous = gain;
        }
        assert!(fade.gain_at(143) < 0.002);
        assert!(fade.is_complete(144));
        assert!(!fade.is_complete(143));
        // 非法采样率回退，不会 panic / 不会产生 0 采样淡出。
        assert!(StealFade::new(f32::NAN).samples() >= 1);
        assert!(StealFade::new(-1.0).samples() == 144);
    }

    #[test]
    fn stale_handles_are_rejected() {
        let mut pool = VoicePool::new(2, 48_000.0).expect("valid capacity");
        let handle = pool.note_on(60, 100, -6.0).started();
        pool.finish(handle).expect("live handle");
        assert!(matches!(
            pool.set_level(handle, -9.0),
            Err(SfzError::StaleVoiceHandle)
        ));
        assert!(pool.voice(handle).is_none());
    }

    #[test]
    fn invalid_capacity_is_a_result_not_a_panic() {
        assert!(matches!(
            VoicePool::new(0, 48_000.0),
            Err(SfzError::InvalidVoiceCapacity { requested: 0, .. })
        ));
        assert!(matches!(
            VoicePool::new(MAX_VOICE_CAPACITY + 1, 48_000.0),
            Err(SfzError::InvalidVoiceCapacity { .. })
        ));
        assert_eq!(VoicePool::default().capacity(), DEFAULT_VOICE_CAPACITY);
    }

    #[test]
    fn injected_non_finite_level_does_not_break_determinism() {
        let run = || {
            let mut pool = VoicePool::new(2, 48_000.0).expect("valid capacity");
            pool.note_on(60, 100, -6.0);
            pool.note_on(61, 100, f32::NAN);
            pool.note_on(62, 100, -6.0)
        };
        assert_eq!(run(), run());
        assert_eq!(run().victim().map(|handle| handle.index), Some(1));
    }
}
