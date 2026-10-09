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
use crate::instrument::NotePolyphony;

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
    ///
    /// 淡出长度在 `f64` 里算。理由：`f32` 的 `rate * 3 / 1000` 在采样率大于约
    /// `1.13e38` 时先上溢成 `inf`，于是「极大采样率」落进 `else` 分支退化成 **1** 个采样
    /// —— 既与 `MAX_FADE_SAMPLES` 的钳制意图相反，也让长度对采样率**非单调**
    /// （`1e37` 给 1048576，`f32::MAX` 反而给 1）。`f64` 的中间乘积最大约 `1.02e36`，
    /// 恒为有限值，因此钳制重新成为唯一的收口。
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let safe_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            48_000.0
        };
        let raw = (f64::from(safe_rate) * f64::from(STEAL_FADE_MILLIS) / 1000.0).ceil();
        let samples = if raw.is_finite() && raw >= 1.0 {
            // `f64 as u32` 在 Rust 里是饱和转换：超大 `raw` 收成 `u32::MAX` 再被钳住。
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
    /// polyphony group（`group` / `polyphony_group`，缺省 0）。
    ///
    /// 只是 [`VoicePool::apply_note_polyphony`] 的键：同一 group 且同一音高的声部
    /// 才互相计入 `note_polyphony` 限制（<https://sfzformat.com/opcodes/note_polyphony/>）。
    pub group: u32,
    /// 包络阶段。
    pub stage: VoiceStage,
    /// 注入的瞬时电平 (dBFS)。
    pub level_db: f32,
    /// 触发序号（越大越晚触发）。
    pub order: u64,
    /// 剩余淡出采样数。
    ///
    /// 不变量：`retiring == false` 时恒为 `0`；`retiring == true` 时恒 `>= 1`
    /// （只有 [`VoicePool::retire`] / [`VoicePool::apply_note_polyphony`] 会把它设成正数，
    /// 只有 [`VoicePool::process`] 与 [`VoicePool::set_stage`] / [`VoicePool::finish`] 会清它）。
    pub fade_remaining: u32,
    /// 剩余窃取淡入采样数。
    ///
    /// 不变量：`active == false` 时恒为 `0`；否则它由 [`VoicePool::process`] **单调递减**
    /// 到 `0`，到 `0` 之后不再变化 —— 因此它**不需要**像 [`VoiceInfo::fade_remaining`]
    /// 那样由外部显式清零（后者只在 `retiring` 为真时被推进）。[`VoicePool::retire`] 与
    /// [`VoicePool::apply_note_polyphony`] 会把它归零（淡出胜过淡入），
    /// [`VoicePool::set_stage`] 刻意不碰它。
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
    group: u32,
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
            group: 0,
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
    ///
    /// **幂等**：同一个值重复设置与只设置一次状态完全相同。已经在飞的那条淡出
    /// （[`VoiceInfo::fade_remaining`]）**不**按新采样率重算，也不被清零 —— 它的长度是
    /// 窃取 / [`VoicePool::retire`] 那一刻由 [`StealFade`] 定下的。因此本方法只改
    /// [`VoicePool::steal_fade`] 的读数，不触碰任何槽位字段。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.fade = StealFade::new(sample_rate);
    }

    /// 触发一个音符。池满时按 [ARCH-RT-004] 的确定性规则窃取。
    ///
    /// **零分配**。窃取顺序见 [`VoicePool::select_victim`]。
    ///
    /// 等价于 [`VoicePool::note_on_in_group`] 取 `group = 0`（缺省 polyphony group）。
    pub fn note_on(&mut self, note: u8, velocity: u8, level_db: f32) -> NoteOnOutcome {
        self.note_on_in_group(note, velocity, level_db, 0)
    }

    /// 触发一个音符，并记下它的 polyphony group（`group` / `polyphony_group`）。
    ///
    /// `group` 只参与 [`VoicePool::apply_note_polyphony`] 的键；它**不**改变任何
    /// 窃取 / 淡出行为，也不改变返回值。**零分配**。
    pub fn note_on_in_group(
        &mut self,
        note: u8,
        velocity: u8,
        level_db: f32,
        group: u32,
    ) -> NoteOnOutcome {
        if let Some(index) = self.slots.iter().position(|slot| !slot.active) {
            return NoteOnOutcome::Started(
                self.activate(index, note, velocity, level_db, group, 0),
            );
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
        let started = self.activate(index, note, velocity, level_db, group, fade_in);
        self.steal_count = self.steal_count.wrapping_add(1);
        self.last_stolen = Some(victim);
        NoteOnOutcome::Stolen { victim, started }
    }

    /// 施加 `note_polyphony` 限制：结束同一 polyphony group 内**同一音高**的超额在场声部。
    ///
    /// `handle` 是刚触发的新声部（它的 `note` / `group` / `velocity` 已由
    /// [`VoicePool::note_on_in_group`] 记下）。返回被结束的声部数（0 表示无需让位）。
    /// 被结束的声部留在自己的槽位上、状态转为 `retiring` 并带 3 ms 指数淡出
    /// （[`StealFade`]），由渲染器读 [`VoiceInfo::retiring`] 淡化、由
    /// [`VoicePool::process`] 在淡出走完后归零回收 —— 因此**不需要**返回句柄。
    ///
    /// # 规则（语义与裁决见 [`NotePolyphony`]）
    ///
    /// 只要「同键在场数 + 新声部」超过 `limit`，就结束一个**有资格**的在场声部；
    /// 已处于 `retiring` 的声部不计入在场数。资格由 [`NotePolyphony::masks`] 给出
    /// （`self_mask` 为真时只允许结束力度不高于新声部的声部）。没有有资格的声部时停止，
    /// 新声部照常发声。`limit == 0`（[`NotePolyphony::is_unlimited`]）时直接返回 0。
    ///
    /// 被选中的声部是排序键 `(力度, 触发序号, 槽位下标)` 的**最小者**：力度最低者优先，
    /// 同力度取触发最早者，再同则取槽位下标最小者 —— 全序，因此同一输入序列结果唯一
    /// （[ARCH-DET-001]）。
    ///
    /// **零分配**：全程线性扫描，只在已有槽位上写字段。
    pub fn apply_note_polyphony(
        &mut self,
        handle: VoiceHandle,
        policy: NotePolyphony,
    ) -> Result<usize, SfzError> {
        if policy.is_unlimited() {
            return Ok(0);
        }
        let (note, group, velocity, index) = {
            let slot = self.voice(handle).ok_or(SfzError::StaleVoiceHandle)?;
            (slot.note, slot.group, slot.velocity, handle.index as usize)
        };
        let limit = policy.limit as usize;
        let mut standing = self
            .slots
            .iter()
            .enumerate()
            .filter(|(other, slot)| {
                *other != index
                    && slot.active
                    && !slot.retiring
                    && slot.note == note
                    && slot.group == group
            })
            .count();
        let mut ended = 0usize;
        while standing >= limit {
            let Some(victim) = self.polyphony_victim(index, note, group, velocity, policy) else {
                break;
            };
            let fade = self.fade;
            if let Some(slot) = self.slots.get_mut(victim) {
                slot.retiring = true;
                slot.stage = VoiceStage::Release;
                slot.fade_remaining = fade.samples();
                slot.fade_in_remaining = 0;
            }
            standing -= 1;
            ended += 1;
        }
        Ok(ended)
    }

    /// `note_polyphony` 的下一个让位对象：`(力度, 触发序号, 槽位下标)` 最小的**有资格**
    /// 同键在场声部。没有则返回 `None`（新声部照常发声）。零分配。
    fn polyphony_victim(
        &self,
        exclude: usize,
        note: u8,
        group: u32,
        velocity: u8,
        policy: NotePolyphony,
    ) -> Option<usize> {
        let mut best: Option<(u8, u64, usize)> = None;
        for (other, slot) in self.slots.iter().enumerate() {
            if other == exclude
                || !slot.active
                || slot.retiring
                || slot.note != note
                || slot.group != group
                || !policy.masks(slot.velocity, velocity)
            {
                continue;
            }
            let key = (slot.velocity, slot.order, other);
            if best.is_none_or(|current| key < current) {
                best = Some(key);
            }
        }
        best.map(|(_, _, index)| index)
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
    ///
    /// **幂等**：这里是**赋值**而不是累加，同一个值重复写与只写一次得到相同的
    /// [`VoiceInfo::level_db`]（非有限取值同样先经 `sanitize_level` 归一）。
    pub fn set_level(&mut self, handle: VoiceHandle, level_db: f32) -> Result<(), SfzError> {
        self.slot_mut(handle)?.level_db = sanitize_level(level_db);
        Ok(())
    }

    /// 用闭包批量刷新所有活跃声部的瞬时电平（能量注入，零分配）。
    ///
    /// **幂等**（对**纯**闭包而言）：每个槽位都是赋值，同一个纯闭包重复调用得到相同的
    /// 快照。闭包自身若带内部状态（例如自增计数器），重复调用的结果由调用方负责 ——
    /// 本方法只保证「同一个读数 ⇒ 同一个写入」。
    pub fn refresh_levels(&mut self, mut level_of: impl FnMut(usize) -> f32) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.active {
                slot.level_db = sanitize_level(level_of(index));
            }
        }
    }

    /// 设置包络阶段。
    ///
    /// 同时**取消**该声部尚未走完的 `retire` / `note_polyphony` 让位淡出：`retiring`
    /// 归 `false`，淡出计数 [`VoiceInfo::fade_remaining`] 一并清零。
    ///
    /// 两者必须一起复位。只清 `retiring` 会留下「不在淡出、却仍记着剩余淡出采样数」的
    /// 残值，而 [`VoicePool::process`] 只在 `retiring` 为真时推进该计数 —— 残值既不会被
    /// 推进，也不会被回收，快照读出来就是个没有意义的数。
    ///
    /// **幂等**：重复施加同一个 `stage` 是恒等 —— 第二次调用既不再次递增代数，也不复活
    /// 已被取消的淡出（取消之后只能由 [`VoicePool::retire`] 重新武装）。
    pub fn set_stage(&mut self, handle: VoiceHandle, stage: VoiceStage) -> Result<(), SfzError> {
        let slot = self.slot_mut(handle)?;
        slot.stage = stage;
        slot.retiring = false;
        slot.fade_remaining = 0;
        Ok(())
    }

    /// note-off：进入 Release 阶段（真正回收由渲染器调用 [`VoicePool::retire`] 或
    /// [`VoicePool::finish`] 触发）。
    pub fn note_off(&mut self, handle: VoiceHandle) -> Result<(), SfzError> {
        self.set_stage(handle, VoiceStage::Release)
    }

    /// 开始 3ms 指数淡出。淡出走完后 `process` 会把槽位归零并标记可复用。
    ///
    /// **幂等（同一步内）**：两次调用之间没有 [`VoicePool::process`] 时，第二次与第一次的
    /// 状态完全相同 —— 计数器是被**赋值**成完整长度，不是累加。两次调用之间推进过
    /// `process` 时，第二次把计数器重新武装回完整长度（槽位寿命至多延长一个 3ms）：
    /// 这是「重新开始一次淡出」的**动作**语义，与 [`VoicePool::note_on`] 是事件而非赋值
    /// 同一条口径。
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
        group: u32,
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
            group,
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
        group: slot.group,
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
    fn set_stage_cancels_the_pending_fade_and_clears_its_counter() {
        let mut pool = VoicePool::new(2, 48_000.0).expect("valid capacity");
        let handle = pool.note_on(60, 100, -6.0).started();
        pool.retire(handle).expect("live handle");
        assert_eq!(
            pool.voice(handle).expect("still active").fade_remaining,
            pool.steal_fade().samples()
        );

        pool.set_stage(handle, VoiceStage::Sustain)
            .expect("live handle");
        let info = pool.voice(handle).expect("still active");
        assert!(!info.retiring, "set_stage cancels the pending fade");
        assert_eq!(info.stage, VoiceStage::Sustain);
        assert_eq!(
            info.fade_remaining, 0,
            "a cancelled fade must not leave its counter behind"
        );

        // 取消之后 `process` 不再回收这个槽位（渲染器必须显式 finish / retire）——
        // 这条同时证明残值不是「靠 process 兜住」的。
        pool.process(pool.steal_fade().samples());
        assert!(pool.voice(handle).is_some());
        assert_eq!(pool.voice(handle).expect("still active").fade_remaining, 0);

        // 让位淡出（`apply_note_polyphony`）走的是同一条 `retiring` 路径，同样被取消。
        let mut masked = VoicePool::new(4, 48_000.0).expect("valid capacity");
        let victim = masked.note_on_in_group(60, 100, -6.0, 0).started();
        let start = masked.note_on_in_group(60, 100, -6.0, 0).started();
        masked
            .apply_note_polyphony(start, limit(1, true))
            .expect("live handle");
        assert!(masked.voice(victim).expect("still active").retiring);
        masked
            .set_stage(victim, VoiceStage::Release)
            .expect("live handle");
        let cancelled = masked.voice(victim).expect("still active");
        assert!(!cancelled.retiring);
        assert_eq!(cancelled.fade_remaining, 0);
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
    fn the_fade_length_saturates_at_the_clamp_for_huge_sample_rates() {
        // `f32` 中间乘积在 rate > f32::MAX/3 时上溢成 `inf`，旧口径因此把 `f32::MAX`
        // 变成 1 个采样；`f64` 里算之后钳制成为唯一的收口。
        assert_eq!(StealFade::new(f32::MAX).samples(), MAX_FADE_SAMPLES);
        assert_eq!(StealFade::new(1.0e38).samples(), MAX_FADE_SAMPLES);
        assert_eq!(StealFade::new(48_000.0).samples(), 144, "3ms @ 48kHz");
        assert_eq!(StealFade::new(44_100.0).samples(), 133, "ceil(132.3)");
        assert_eq!(StealFade::new(1.0).samples(), 1);

        // 长度对采样率单调不减，且恒落在 `1..=MAX_FADE_SAMPLES`。
        let mut previous = 0u32;
        for rate in [
            1.0f32,
            100.0,
            44_100.0,
            48_000.0,
            1.0e6,
            1.0e9,
            1.0e12,
            1.0e30,
            1.0e37,
            1.13e38,
            1.14e38,
            f32::MAX,
        ] {
            let samples = StealFade::new(rate).samples();
            assert!(
                (1..=MAX_FADE_SAMPLES).contains(&samples),
                "rate {rate} gave {samples}"
            );
            assert!(
                samples >= previous,
                "rate {rate} went backwards: {samples} < {previous}"
            );
            previous = samples;
        }
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

    // ------------------------------------------------------------------
    // note_polyphony / note_selfmask（同音同时发声数限制）
    // ------------------------------------------------------------------

    fn limit(limit: u32, self_mask: bool) -> NotePolyphony {
        NotePolyphony { limit, self_mask }
    }

    #[test]
    fn note_polyphony_ends_the_lower_velocity_voice_by_default() {
        let mut pool = VoicePool::new(8, 48_000.0).expect("valid capacity");
        let first = pool.note_on_in_group(60, 100, -6.0, 0).started();
        let second = pool.note_on_in_group(60, 100, -6.0, 0).started();
        let ended = pool
            .apply_note_polyphony(second, limit(1, true))
            .expect("live handle");
        assert_eq!(ended, 1);
        assert!(pool.voice(first).expect("still active").retiring);
        assert!(
            !pool.voice(second).expect("still active").retiring,
            "the new note always plays"
        );
    }

    #[test]
    fn a_quieter_note_does_not_end_a_louder_one_when_self_mask_is_on() {
        // 规范：缺省 self-mask 下 note_polyphony 是「hint 而不是严格上限」——
        // 严格更低的力度不关掉更高的在场声部，新声部照常发声。
        let mut pool = VoicePool::new(8, 48_000.0).expect("valid capacity");
        let loud = pool.note_on_in_group(60, 100, -6.0, 0).started();
        let quiet = pool.note_on_in_group(60, 40, -6.0, 0).started();
        let ended = pool
            .apply_note_polyphony(quiet, limit(1, true))
            .expect("live handle");
        assert_eq!(
            ended, 0,
            "a strictly quieter note does not mask a louder one"
        );
        assert!(!pool.voice(loud).expect("still active").retiring);
        assert!(!pool.voice(quiet).expect("still active").retiring);
    }

    #[test]
    fn note_selfmask_off_makes_the_limit_strict() {
        let mut pool = VoicePool::new(8, 48_000.0).expect("valid capacity");
        let loud = pool.note_on_in_group(60, 100, -6.0, 0).started();
        let quiet = pool.note_on_in_group(60, 40, -6.0, 0).started();
        let ended = pool
            .apply_note_polyphony(quiet, limit(1, false))
            .expect("live handle");
        assert_eq!(
            ended, 1,
            "selfmask off ends the same pitch regardless of velocity"
        );
        assert!(pool.voice(loud).expect("still active").retiring);
    }

    #[test]
    fn the_note_polyphony_key_is_the_group_plus_the_pitch() {
        let mut pool = VoicePool::new(8, 48_000.0).expect("valid capacity");
        let other_group = pool.note_on_in_group(60, 100, -6.0, 1).started();
        let other_note = pool.note_on_in_group(61, 100, -6.0, 0).started();
        let same_key = pool.note_on_in_group(60, 100, -6.0, 0).started();
        let ended = pool
            .apply_note_polyphony(same_key, limit(1, true))
            .expect("live handle");
        assert_eq!(ended, 0, "a different group or pitch is outside the key");
        assert!(!pool.voice(other_group).expect("still active").retiring);
        assert!(!pool.voice(other_note).expect("still active").retiring);
        assert_eq!(
            pool.voice(same_key).expect("still active").group,
            0,
            "the group is recorded on the slot and read back"
        );
    }

    #[test]
    fn a_zero_note_polyphony_limit_never_ends_a_voice() {
        let mut pool = VoicePool::new(8, 48_000.0).expect("valid capacity");
        let mut last = pool.note_on_in_group(60, 100, -6.0, 0).started();
        for _ in 0..3 {
            last = pool.note_on_in_group(60, 100, -6.0, 0).started();
        }
        assert_eq!(
            pool.apply_note_polyphony(last, NotePolyphony::UNLIMITED)
                .expect("live handle"),
            0
        );
        assert_eq!(pool.active_count(), 4, "all four same-key voices stay");
    }

    #[test]
    fn note_polyphony_reduces_to_the_limit_deterministically() {
        let run = || {
            let mut pool = VoicePool::new(8, 48_000.0).expect("valid capacity");
            // 三个同键声部，力度 100 / 60 / 80；新声部是 80，limit = 2。
            // 只有 60 有资格（60 <= 80），因此被结束的是它 —— 唯一的那个。
            let handles: Vec<VoiceHandle> = [100u8, 60, 80]
                .into_iter()
                .map(|velocity| pool.note_on_in_group(60, velocity, -6.0, 0).started())
                .collect();
            let ended = pool
                .apply_note_polyphony(*handles.last().expect("a voice"), limit(2, true))
                .expect("live handle");
            let retiring: Vec<bool> = handles
                .iter()
                .map(|handle| pool.voice(*handle).expect("still active").retiring)
                .collect();
            (ended, retiring)
        };
        let (ended, retiring) = run();
        assert_eq!(ended, 1);
        assert_eq!(
            retiring,
            vec![false, true, false],
            "the quietest eligible same-key voice goes first"
        );
        assert_eq!(
            run(),
            (ended, retiring),
            "same input sequence ⇒ same result"
        );
    }

    #[test]
    fn retiring_voices_do_not_count_toward_the_limit_twice() {
        let mut pool = VoicePool::new(8, 48_000.0).expect("valid capacity");
        let first = pool.note_on_in_group(60, 100, -6.0, 0).started();
        let second = pool.note_on_in_group(60, 100, -6.0, 0).started();
        assert_eq!(
            pool.apply_note_polyphony(second, limit(1, true))
                .expect("live handle"),
            1
        );
        assert!(pool.voice(first).expect("still active").retiring);

        // 第二次调用（同一新声部）：未 retiring 的同键声部只剩它自己 ⇒ 无可让位者。
        // 若 `retiring` 声部仍计入在场数，这里会再结束一个。
        assert_eq!(
            pool.apply_note_polyphony(second, limit(1, true))
                .expect("live handle"),
            0,
            "already-retiring voices are not counted again"
        );
        assert!(!pool.voice(second).expect("still active").retiring);
    }

    #[test]
    fn masking_a_voice_enters_the_existing_steal_fade_path() {
        let mut pool = VoicePool::new(8, 48_000.0).expect("valid capacity");
        let victim = pool.note_on_in_group(60, 100, -6.0, 0).started();
        let start = pool.note_on_in_group(60, 100, -6.0, 0).started();
        pool.apply_note_polyphony(start, limit(1, true))
            .expect("live handle");
        let info = pool.voice(victim).expect("still active");
        assert_eq!(info.stage, VoiceStage::Release);
        assert_eq!(info.fade_remaining, pool.steal_fade().samples());

        pool.process(pool.steal_fade().samples());
        assert!(
            pool.voice(victim).is_none(),
            "the 3ms fade reclaims the slot"
        );
    }

    #[test]
    fn apply_note_polyphony_rejects_a_stale_handle_and_never_panics() {
        let mut pool = VoicePool::new(2, 48_000.0).expect("valid capacity");
        let handle = pool.note_on_in_group(60, 100, -6.0, 0).started();
        pool.finish(handle).expect("live handle");
        assert!(matches!(
            pool.apply_note_polyphony(handle, limit(1, true)),
            Err(SfzError::StaleVoiceHandle)
        ));
        // 越界句柄同样只是 `Err`，不 panic。
        let far = VoiceHandle {
            index: u32::MAX,
            generation: 1,
        };
        assert!(matches!(
            pool.apply_note_polyphony(far, limit(1, true)),
            Err(SfzError::StaleVoiceHandle)
        ));
        // `limit == 0` 在取句柄**之前**返回，因此连越界句柄也不报错（无锁、无索引）。
        assert_eq!(
            pool.apply_note_polyphony(far, NotePolyphony::UNLIMITED)
                .expect("unlimited short-circuits"),
            0
        );
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

    // ------------------------------------------------------------------
    // 幂等性（类别 5）：同一对象上重复施加同一个值
    // ------------------------------------------------------------------
    //
    // 判定口径：对**同一个池实例**把同一个值写两次，逐槽位快照必须与写一次相同。
    // 快照是 `Vec<Option<VoiceInfo>>`（含空闲槽位，「归零可复用」也因此在判据内）。
    // 事件型入口（`note_on` / `note_on_in_group`）与动作型入口（`retire` + `process`）
    // 的结论见本节的最后两条判据。

    /// 把一个池的每个槽位快照成可比较的向量（含空闲槽位）。
    fn snapshot(pool: &VoicePool) -> Vec<Option<VoiceInfo>> {
        (0..pool.capacity())
            .map(|index| pool.voice_at(index))
            .collect()
    }

    #[test]
    fn a_repeated_same_valued_level_write_leaves_every_slot_unchanged() {
        let mut pool = VoicePool::new(4, 48_000.0).expect("valid capacity");
        let live = pool.note_on(60, 100, -6.0).started();
        pool.note_on(61, 100, -12.0);
        let retiring = pool.note_on(62, 100, -3.0).started();
        pool.retire(retiring).expect("live handle");

        pool.set_level(live, -9.0).expect("live handle");
        let once = snapshot(&pool);
        pool.set_level(live, -9.0).expect("live handle");
        assert_eq!(
            snapshot(&pool),
            once,
            "level is assigned, never accumulated"
        );

        // 逐位口径：`f32` 用 `to_bits`（`PartialEq` 会把 `-0.0` 与 `0.0` 视为相等）。
        assert_eq!(
            pool.voice(live).expect("still active").level_db.to_bits(),
            (-9.0f32).to_bits()
        );

        // 同一个纯闭包重复调用：`refresh_levels` 也是逐槽位赋值。
        pool.refresh_levels(|index| -6.0 - index as f32);
        let once = snapshot(&pool);
        pool.refresh_levels(|index| -6.0 - index as f32);
        assert_eq!(snapshot(&pool), once);
        assert_eq!(
            pool.voice(live).expect("still active").level_db.to_bits(),
            (-6.0f32).to_bits(),
            "the last write is the one that stands, for every active slot"
        );

        // 非有限注入同样幂等：第二次写入与第一次是同一个归一结果。
        pool.set_level(live, f32::NAN).expect("live handle");
        let once = pool.voice(live).expect("still active").level_db;
        pool.set_level(live, f32::NAN).expect("live handle");
        assert_eq!(
            pool.voice(live).expect("still active").level_db.to_bits(),
            once.to_bits()
        );
    }

    #[test]
    fn a_repeated_same_valued_stage_write_leaves_every_slot_unchanged() {
        let mut pool = VoicePool::new(4, 48_000.0).expect("valid capacity");
        let live = pool.note_on(60, 100, -6.0).started();
        let other = pool.note_on(61, 100, -6.0).started();
        pool.retire(other).expect("live handle");

        pool.set_stage(live, VoiceStage::Sustain)
            .expect("live handle");
        let once = snapshot(&pool);
        pool.set_stage(live, VoiceStage::Sustain)
            .expect("live handle");
        assert_eq!(
            snapshot(&pool),
            once,
            "a repeated stage write must not bump the generation or revive a cancelled fade"
        );

        // `note_off` 就是 `set_stage(Release)`（见其实现）：重复两次与一次相同，
        // 包括「第一次把尚未走完的淡出取消掉」这一步。
        pool.retire(live).expect("live handle");
        pool.note_off(live).expect("live handle");
        let cancelled = pool.voice(live).expect("still active");
        assert!(!cancelled.retiring);
        assert_eq!(cancelled.fade_remaining, 0);
        let once = snapshot(&pool);
        pool.note_off(live).expect("live handle");
        assert_eq!(snapshot(&pool), once);

        // 幂等 ≠ 句柄失效：重复写之后句柄仍然有效。
        assert_eq!(
            pool.voice(live).expect("still active").stage,
            VoiceStage::Release
        );
        assert_eq!(pool.active_count(), 2);
    }

    #[test]
    fn a_repeated_same_valued_sample_rate_write_is_a_no_op() {
        let mut pool = VoicePool::new(2, 48_000.0).expect("valid capacity");
        let handle = pool.note_on(60, 100, -6.0).started();
        pool.retire(handle).expect("live handle");
        let in_flight = pool.voice(handle).expect("still active").fade_remaining;
        assert_eq!(in_flight, 144, "3ms @ 48kHz");

        pool.set_sample_rate(96_000.0);
        let once = (
            snapshot(&pool),
            pool.steal_fade(),
            pool.steal_count(),
            pool.last_stolen(),
        );
        assert_eq!(
            pool.voice(handle).expect("still active").fade_remaining,
            in_flight,
            "the rate change must not rescale a fade that is already in flight"
        );
        assert_eq!(pool.steal_fade().samples(), 288, "3ms @ 96kHz");

        pool.set_sample_rate(96_000.0);
        let twice = (
            snapshot(&pool),
            pool.steal_fade(),
            pool.steal_count(),
            pool.last_stolen(),
        );
        assert_eq!(once, twice, "the same rate twice must equal it once");
    }

    #[test]
    fn setting_the_sample_rate_after_construction_equals_constructing_with_it() {
        // 同一个值经两条路径进入：构造参数与 `set_sample_rate`。两条路径后的池必须相同。
        let direct = VoicePool::new(4, 44_100.0).expect("valid capacity");
        let mut late = VoicePool::new(4, 48_000.0).expect("valid capacity");
        late.set_sample_rate(44_100.0);
        assert_eq!(late.steal_fade(), direct.steal_fade());
        assert_eq!(late.steal_fade().samples(), 133, "ceil(44.1kHz * 3ms)");
        assert_eq!(snapshot(&late), snapshot(&direct));
    }

    #[test]
    fn retire_without_an_intervening_process_is_idempotent() {
        let mut pool = VoicePool::new(2, 48_000.0).expect("valid capacity");
        let handle = pool.note_on(60, 100, -6.0).started();
        let first = pool.retire(handle).expect("live handle");
        let once = snapshot(&pool);
        let second = pool.retire(handle).expect("live handle");
        assert_eq!(first, second, "both calls report the same envelope");
        assert_eq!(
            snapshot(&pool),
            once,
            "a repeated retire must assign the counter, not accumulate it"
        );
        assert_eq!(
            pool.voice(handle).expect("still active").fade_remaining,
            first.samples()
        );
        assert!(pool.voice(handle).expect("still active").retiring);
    }

    #[test]
    fn retire_after_a_process_step_re_arms_the_counter_by_design() {
        // 类别 5 的诚实边界：`retire` 是「重新武装一次淡出」的动作，不是「设一个值」。
        // 中途推进过 `process` 之后再 `retire`，计数器回到满值（槽位寿命至多延长一个
        // 3ms），因此它在时间域上**不**幂等 —— 这是刻意的 re-arm 语义，不是缺陷。
        let mut pool = VoicePool::new(2, 48_000.0).expect("valid capacity");
        let handle = pool.note_on(60, 100, -6.0).started();
        let fade = pool.retire(handle).expect("live handle");
        pool.process(100);
        assert_eq!(
            pool.voice(handle).expect("still active").fade_remaining,
            fade.samples() - 100
        );
        pool.retire(handle).expect("live handle");
        assert_eq!(
            pool.voice(handle).expect("still active").fade_remaining,
            fade.samples(),
            "the second retire re-arms the countdown"
        );
    }

    #[test]
    fn process_zero_frames_is_the_identity() {
        // 三个槽位：一个走 Release（tier 0、序号更早）、一个在 retiring、一个是新窃取的。
        let mut pool = VoicePool::new(3, 48_000.0).expect("valid capacity");
        let released = pool.note_on(60, 100, -6.0).started();
        let retiring = pool.note_on(61, 100, -6.0).started();
        pool.note_on(62, 100, -6.0);
        pool.note_off(released).expect("live handle");
        pool.retire(retiring).expect("live handle");
        let outcome = pool.note_on(63, 100, -6.0);
        assert_eq!(
            outcome.victim(),
            Some(released),
            "the earliest tier-0 voice goes first"
        );
        let started = outcome.started();
        assert_eq!(
            pool.voice(started).expect("still active").fade_in_remaining,
            144,
            "a stolen slot starts with a fade-in"
        );
        assert_eq!(
            pool.voice(retiring).expect("still active").fade_remaining,
            144
        );

        let before = snapshot(&pool);
        pool.process(0);
        assert_eq!(
            snapshot(&pool),
            before,
            "zero frames must not advance a counter or reclaim a slot"
        );
        assert!(pool.voice(retiring).is_some());
    }

    #[test]
    fn finish_is_state_idempotent_and_the_second_call_is_a_stale_handle_error() {
        let mut pool = VoicePool::new(2, 48_000.0).expect("valid capacity");
        let handle = pool.note_on(60, 100, -6.0).started();
        pool.finish(handle).expect("live handle");
        let freed = pool.voice_at(handle.index as usize).expect("slot exists");
        let active = pool.active_count();

        assert!(matches!(
            pool.finish(handle),
            Err(SfzError::StaleVoiceHandle)
        ));
        let after = pool.voice_at(handle.index as usize).expect("slot exists");
        assert_eq!(after, freed, "a rejected finish must not touch the slot");
        assert_eq!(pool.active_count(), active);
        assert!(pool.voice(handle).is_none());

        // 代数口径：`VoiceInfo` 不带 `generation`，所以用「重新占用后的新句柄」来读它。
        // 第一次 `finish` 递增 1、重新激活再递增 1 ⇒ 新代数恰好是旧代数 + 2；
        // 若被拒绝的第二次 `finish` 也递增了代数，这里会读到 + 3。
        let reused = pool.note_on(72, 100, -3.0).started();
        assert_eq!(reused.index, handle.index);
        assert_eq!(
            reused.generation,
            handle.generation + 2,
            "the rejected finish must not bump the generation a second time"
        );
    }

    #[test]
    fn a_repeated_note_on_is_an_event_and_not_a_setter() {
        // 类别 5 的表内结论：`note_on` **不**满足幂等，这是事件语义而不是缺陷 ——
        // 同一音符事件重复触发两次就是两个并发声部（两次 note-on 是两件事件）。
        let mut pool = VoicePool::new(4, 48_000.0).expect("valid capacity");
        let once = pool.note_on(60, 100, -6.0).started();
        let after_one = snapshot(&pool);
        let twice = pool.note_on(60, 100, -6.0).started();
        assert_ne!(once.index, twice.index, "two distinct slots");
        assert_ne!(once, twice, "two distinct handles");
        assert!(pool.voice(once).is_some() && pool.voice(twice).is_some());
        assert_eq!(pool.active_count(), 2, "two triggers start two voices");
        assert_ne!(snapshot(&pool), after_one);

        // 池满时同一事件第二次触发的是**窃取**：容量与占用数都不变，victim 确定。
        let mut small = VoicePool::new(1, 48_000.0).expect("valid capacity");
        let first = small.note_on(60, 100, -6.0).started();
        let second = small.note_on(60, 100, -6.0);
        assert_eq!(second.victim(), Some(first));
        assert_eq!(small.capacity(), 1);
        assert_eq!(small.active_count(), 1);
        assert_eq!(small.steal_count(), 1);
    }

    #[test]
    fn the_read_only_entry_points_do_not_change_the_pool() {
        let mut pool = VoicePool::new(3, 48_000.0).expect("valid capacity");
        pool.note_on(60, 100, -6.0);
        pool.note_on(61, 100, -6.0);
        let before = snapshot(&pool);
        let victim = pool.select_victim();
        assert_eq!(pool.select_victim(), victim, "a query is pure");
        assert_eq!(pool.capacity(), 3);
        assert_eq!(pool.active_count(), 2);
        assert_eq!(pool.free_count(), 1);
        assert_eq!(pool.steal_count(), 0);
        assert_eq!(pool.last_stolen(), None);
        assert_eq!(pool.voice_at(1), before[1]);
        assert_eq!(snapshot(&pool), before, "no read-only entry point mutates");
    }

    #[test]
    fn a_repeated_same_valued_polyphony_call_leaves_every_slot_unchanged() {
        // 类别 5 的收口：`apply_note_polyphony` 是 11 个可变公开入口之一，既有判据只按
        // 「第二次返回 0」读过它；这里按**逐槽位快照**判定。
        let mut pool = VoicePool::new(4, 48_000.0).expect("valid capacity");
        let first = pool.note_on_in_group(60, 100, -6.0, 0).started();
        let second = pool.note_on_in_group(60, 100, -6.0, 0).started();
        let other_pitch = pool.note_on_in_group(61, 100, -6.0, 0).started();
        assert_eq!(
            pool.apply_note_polyphony(second, limit(1, true))
                .expect("live handle"),
            1,
            "the earlier same-key voice gives way"
        );
        assert!(pool.voice(first).expect("still active").retiring);
        // 中途推进一小段，让那条让位淡出**不再**是满值：这样「第二次调用是否重新武装」
        // 就成为一个可观测的差别（不推进时它天然看不出来）。
        pool.process(100);
        let once = snapshot(&pool);
        assert_eq!(
            pool.voice(first).expect("still active").fade_remaining,
            pool.steal_fade().samples() - 100
        );

        // 第二次施加同一个值：已让位的声部不再计入在场数 ⇒ 0，且一个字段都不动。
        assert_eq!(
            pool.apply_note_polyphony(second, limit(1, true))
                .expect("live handle"),
            0
        );
        assert_eq!(
            snapshot(&pool),
            once,
            "a repeated polyphony call must be slot identity"
        );
        assert!(!pool.voice(other_pitch).expect("still active").retiring);
        // 与 `retire` 的 re-arm 语义相对：重复调用**不**重新武装那条让位淡出。
        assert_eq!(
            pool.voice(first).expect("still active").fade_remaining,
            pool.steal_fade().samples() - 100,
            "the repeated call must not re-arm the pending fade"
        );
        // 幂等不是句柄失效。
        assert!(pool.voice(second).is_some());
    }

    #[test]
    fn a_repeated_note_on_in_group_is_an_event_and_records_the_group() {
        // `note_on_in_group` 是 `note_on` 之外的第二个事件入口（多一个 group 参数）：
        // 同一事件重复两次仍是**两个**声部，且两次都记下同一个非零 group。
        let mut pool = VoicePool::new(4, 48_000.0).expect("valid capacity");
        let first = pool.note_on_in_group(60, 100, -6.0, 7).started();
        let after_one = snapshot(&pool);
        let second = pool.note_on_in_group(60, 100, -6.0, 7).started();
        assert_ne!(first, second, "two distinct handles");
        assert_ne!(first.index, second.index, "two distinct slots");
        assert_eq!(pool.active_count(), 2, "two triggers start two voices");
        assert_ne!(snapshot(&pool), after_one);
        assert_eq!(pool.voice(first).expect("still active").group, 7);
        assert_eq!(pool.voice(second).expect("still active").group, 7);

        // 文档化的等价：`note_on` 就是 `group = 0` 的 `note_on_in_group`（句柄 + 状态）。
        let mut plain = VoicePool::new(4, 48_000.0).expect("valid capacity");
        let mut grouped = VoicePool::new(4, 48_000.0).expect("valid capacity");
        assert_eq!(
            plain.note_on(60, 100, -6.0),
            grouped.note_on_in_group(60, 100, -6.0, 0)
        );
        assert_eq!(snapshot(&plain), snapshot(&grouped));

        // 不同的 group 是不同的键：这不改变「事件」这个结论，只是让键不同。
        let mut keyed = VoicePool::new(4, 48_000.0).expect("valid capacity");
        let g0 = keyed.note_on_in_group(60, 100, -6.0, 0).started();
        let g1 = keyed.note_on_in_group(60, 100, -6.0, 1).started();
        assert_eq!(
            keyed
                .apply_note_polyphony(g1, limit(1, true))
                .expect("live handle"),
            0,
            "another group is outside the note_polyphony key"
        );
        assert!(!keyed.voice(g0).expect("still active").retiring);
    }

    // ------------------------------------------------------------------
    // 块长度极值（类别 7）与它们在时间推进轴上的幂等收口（类别 5）
    // ------------------------------------------------------------------

    /// 一个含三种声部形态的池：被窃取的（带 144 采样淡入）、retiring 的（带淡出）、
    /// 普通的。返回池、三个声部各自的句柄，以及窃取产生的新句柄。
    fn fade_fixture() -> (VoicePool, [VoiceHandle; 3], VoiceHandle) {
        let mut pool = VoicePool::new(3, 48_000.0).expect("valid capacity");
        let released = pool.note_on(60, 100, -6.0).started();
        let retiring = pool.note_on(61, 100, -6.0).started();
        let plain = pool.note_on(62, 100, -6.0).started();
        pool.note_off(released).expect("live handle");
        pool.retire(retiring).expect("live handle");
        // 池已满：窃取 tier 0 里触发最早的 `released`，新声部带 144 采样淡入。
        let outcome = pool.note_on(63, 100, -6.0);
        assert_eq!(outcome.victim(), Some(released));
        let stolen = outcome.started();
        assert_eq!(
            pool.voice(stolen).expect("still active").fade_in_remaining,
            144,
            "3ms @ 48kHz"
        );
        (pool, [released, retiring, plain], stolen)
    }

    #[test]
    fn process_is_invariant_under_block_splitting() {
        // 类别 7（块长度极值）＋ 类别 5：同一个**总**推进量，切成两块推进与一次推进必须
        // 逐位相同 —— 渲染器的块长由设备决定，池的状态不允许依赖它。
        // 覆盖 1 帧、非 2 的幂（7 / 11 / 100 / 44）、恰好等于淡出长度（144）、
        // 超过淡出长度（1000）与 `u32::MAX`（饱和）。
        for (first, second) in [
            (1u32, 1u32),
            (7, 11),
            (0, 144),
            (144, 0),
            (100, 44),
            (1000, 5),
            (5, 1000),
            (u32::MAX, 1),
            (1, u32::MAX),
            (u32::MAX, u32::MAX),
        ] {
            let total = first.saturating_add(second);
            let (mut split, split_handles, split_stolen) = fade_fixture();
            split.process(first);
            split.process(second);

            let (mut one_shot, one_shot_handles, one_shot_stolen) = fade_fixture();
            one_shot.process(total);

            assert_eq!(
                snapshot(&split),
                snapshot(&one_shot),
                "split ({first}, {second}) must equal one shot {total}"
            );
            for index in 0..3 {
                assert_eq!(
                    split.voice(split_handles[index]).is_some(),
                    one_shot.voice(one_shot_handles[index]).is_some(),
                    "handle {index} liveness, split ({first}, {second})"
                );
            }
            assert_eq!(
                split.voice(split_stolen).is_some(),
                one_shot.voice(one_shot_stolen).is_some(),
                "the stolen voice's liveness, split ({first}, {second})"
            );
            // 代数口径：`VoiceInfo` 不带 `generation`，用「复用同一槽位得到的新句柄」读它。
            // 两条路径的回收次数必须相同，否则这里会差 1。
            assert_eq!(
                split.note_on(72, 100, -3.0).started(),
                one_shot.note_on(72, 100, -3.0).started(),
                "split ({first}, {second}) must reclaim a slot the same number of times"
            );
        }
    }

    #[test]
    fn a_saturated_process_call_is_a_fixed_point_and_never_wraps() {
        // 类别 7：`frames = u32::MAX` 只能是饱和，不能回绕成小数字（回绕会让一个正在
        // 淡出的声部永远不被回收）。类别 5：饱和之后再施加同一个值就是恒等。
        let (mut pool, handles, stolen) = fade_fixture();
        assert!(pool.voice(handles[1]).is_some(), "the fade is in flight");
        assert_eq!(
            pool.voice(stolen).expect("still active").fade_in_remaining,
            144
        );

        pool.process(u32::MAX);
        let saturated = snapshot(&pool);
        assert!(
            pool.voice(handles[1]).is_none(),
            "one saturated block must reclaim the retiring slot"
        );
        assert_eq!(
            pool.voice(stolen).expect("still active").fade_in_remaining,
            0,
            "the fade-in bottoms out at zero, never wraps"
        );
        assert!(pool.voice(handles[2]).is_some(), "a plain voice survives");

        pool.process(u32::MAX);
        assert_eq!(
            snapshot(&pool),
            saturated,
            "a repeated MAX is the identity on the saturated state"
        );
        pool.process(u32::MAX - 1);
        assert_eq!(snapshot(&pool), saturated);
        pool.process(0);
        assert_eq!(snapshot(&pool), saturated);
        assert_eq!(pool.active_count(), 2);
    }
}
