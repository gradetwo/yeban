//! 时间基准的**纯换算**：音频帧 ↔ tick [MODEL-AST-001]。
//!
//! ## 为什么住在模型层
//!
//! 「一段 N 帧的素材占多少个 tick」是**模型时间基准**的算术，不是某个入口的私事：
//! 它只用到 [`crate::PPQ`]（960）＋ 工程速度 ＋ 采样率，没有 I/O、没有第三方 crate、
//! 没有会话状态 ⇒ 放在这里可以被 `yeban-app`（录音落点）与 `yeban-mcp`（音频导入）
//! 的**任何**一侧调用，而不必让 app 去够一个默认关闭的可选 crate。
//!
//! ## tick → 帧：唯一实现在这里（`#247`）
//!
//! 引擎的 `synth::tick_to_sample` / `samples_per_tick` 与 MCP 的
//! `render_math::ticks_to_frames` 曾经是**两份代数等价、浮点求值不等价**的实现：
//! 引擎先算 `samples_per_tick = 60 × sr / (bpm × ppq)` 再乘 tick，MCP 先算
//! `tick × 60 / ppq / bpm` 再乘 sr。⇒ 44.1 kHz 上实测 **118 处**一帧级分歧
//! （`bpm=60, sr=44100, tick=168`：引擎 **7718** vs MCP **7717**）。
//! 自 `#247` 起只有 [`ticks_to_frames`]（以及它拆出的 [`samples_per_tick`] /
//! [`tick_to_sample`]）做这件事；两个消费者都只调用这里，⛔ 不再各留一份。
//!
//! ## 为什么跨平台可复现
//!
//! 本模块的 tick → 帧只用 `+ - * /` 与 `f64::round`：
//!
//! - IEEE 754 要求基本运算**正确舍入**，`f64` 的格式与舍入规则在所有 Rust 目标上
//!   相同；Rust 默认**不**把 `a * b + c` 收缩成 FMA（没有 fast-math），源码里也
//!   没有 `mul_add`；
//! - `f64::round` 是 IEEE 754 的 `roundToIntegralTiesAway`，不是超越函数、
//!   不经过宿主 `libm`；
//! - 运算顺序在源码里**固定**，没有 `cfg(target_arch)` 分支、没有 SIMD 重结合。
//!
//! ⇒ 同一组输入在任何目标上得到同一组 `f64` 位型，再转成同一个 `u64`。
//! 这与 `#233` 的口径一致：**只有**超越函数才必须走 `libm`，这里一个都没有。
//!
//! ## 反向：唯一实现在这里（`#248` 收口）
//!
//! [`frames_to_ticks_ceil`] 最初只有 `yeban-mcp::domain::render_math` 一份；`#247`
//! 把它逐字搬进本模块时，两边仍然各留一份（本模块文档当时如实登记了这条重复）。
//! `#248` 删掉 MCP 的那份副本：`yeban_import_audio` 的 `durationTicks` 缺省值**直接
//! 调用**本函数。`yeban-app` 的录音落点（`record_landing`）本来就调用这里。
//!
//! ⚠ **它只回答"摆放时值"，不回答"事件送达时刻"**：`yeban-app` 的
//! `midi_notes::frames_to_ticks`（块边界）与 `import_logic::frames_to_ticks`
//! （素材时长）是另外两种**取整方向**（floor／round），⛔ 不是本函数的副本，
//! 也没有被 `#248` 改动 —— 它们的作用对象不是 `ClipPlacement.duration_ticks`。
//!
//! ## 公式（帧 → tick）
//!
//! ```text
//! 秒数      = frames / sample_rate
//! 四分音符数 = 秒数 × bpm / 60
//! tick 数   = 四分音符数 × PPQ
//!           = ceil(frames × PPQ × bpm / (sample_rate × 60))
//! ```
//!
//! **向上取整**（不是四舍五入、也不是向下）：换算的对象是**摆放时值**，向上取整
//! 保证素材尾部不足一 tick 的部分不会被切掉；向下取整会让最后几帧永远听不到，
//! 那是一种静默丢音。`frames × PPQ × bpm / (sample_rate × 60)` 只有 `+ - * /` 与
//! `f64::ceil`（`roundToIntegralTiesAway`），没有超越函数，求值顺序固定 ⇒ 与
//! 正向同一套跨平台可复现论证（见上节）。
//!
//! 结果下界是 **1**：`ClipPlacement::validate` 拒绝零时值，而 0 帧素材也不该产出
//! 一个会被模型层拒掉的 0（本函数对 0 帧返回 `Some(1)`，与既有实现对账）。
//!
//! ## 分段常速：[`TempoMap`]（`#339`）
//!
//! `YebanProjectV1::tempo_map` 早已能写（`setTempoPoint`／MIDI 导入），但渲染器与
//! 引擎此前**只读** `project.bpm` ⇒ 变速被静默忽略。[`TempoMap`] 是那份映射的
//! 播放投影：`base_bpm` 在 tick 0 生效，点 `(tick, bpm)` 表示**从该 tick 起**换成
//! `bpm`。空表**逐位委托** [`ticks_to_frames`]（既有工程一个样本都不变）。
//!
//! 非空表按段累加**未取整**的帧数，最后只 `round` 一次：
//!
//! ```text
//! frames(tick) = round( Σ 段长tick × 60 × sample_rate / (段bpm × ppq) )
//! ```
//!
//! 逐段取整会在每个变速点引入最多半帧的累积误差；只取整一次让整条时间轴仍是
//! 同一把尺子。求值顺序固定（段的遍历顺序 = tick 升序），没有超越函数 ⇒ 与
//! 本模块其余换算同一套跨平台可复现论证。
//!
//! ## 微秒 ↔ tick：硬件时间戳（审计 #17）
//!
//! [`TempoMap::micros_at_tick`] / [`TempoMap::ticks_at_micros`] 是上面分段积分在
//! **微秒**域的对偶（1 秒 = 1_000_000 µs，相当于把 `sample_rate` 换成 1_000_000）：
//! 硬件 MIDI 采集的时间戳是微秒，录音落点（`yeban-app` 的 `midi_notes`）经此换算。
//! 此前采集侧把「1 秒 = 一个四分音符」写死（= 60 BPM），非 60 BPM 工程的录音
//! 落点整体缩放出错（120 BPM 下缩水 50%）；写死点已删，唯一实现收口在这里，
//! 与帧域同一套求值顺序与跨平台可复现论证。

use std::collections::BTreeMap;

/// 分段常速的时间基准（`YebanProjectV1::bpm` ＋ `tempo_map` 的播放投影）。
///
/// 语义：`tick 0` 用 [`base_bpm`](Self::base_bpm)；映射点 `(tick, bpm)` 表示
/// **从该 tick 起**（含该 tick）速度变为 `bpm`。模型层校验映射点 `tick > 0`、
/// `bpm` 有限且落在 `MIN_BPM..=MAX_BPM`，本类型仍对病态输入返回 `None`，
/// 与 [`ticks_to_frames`] 同口径（⛔ 不 panic、⛔ 不发明一个 0）。
///
/// ⚠ 空表是**兼容性承诺**：[`frames_at_tick`](Self::frames_at_tick) 直接调用
/// [`ticks_to_frames`]，因此与 `#339` 之前逐位相同。所有"无变速"工程都走这条路径。
#[derive(Clone, Debug, PartialEq)]
pub struct TempoMap {
    base_bpm: f64,
    points: BTreeMap<u64, f64>,
}

/// 帧数 → tick 跨度时尾部取整的口径。
///
/// 不同调用方有**承重**的既有口径：录音落点用 `Ceil`（最后几帧不能丢），
/// Logic 音频区域导入用 `Round`（`#152` 的长度读数）。口径集中在这里，
/// 调用方不再各写一份算式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameToTickRounding {
    /// 四舍五入（0.5 远离零），下界 1。
    Round,
    /// 向上取整，下界 1（保护摆放时值尾部）。
    Ceil,
}

impl TempoMap {
    /// 由基准速度与映射点构造。不校验 —— 病态值在换算时返回 `None`。
    #[must_use]
    pub fn new(base_bpm: f64, points: BTreeMap<u64, f64>) -> Self {
        Self { base_bpm, points }
    }

    /// 只有基准速度（空映射）。空表路径与 [`ticks_to_frames`] 逐位相同。
    #[must_use]
    pub fn constant(bpm: f64) -> Self {
        Self {
            base_bpm: bpm,
            points: BTreeMap::new(),
        }
    }

    /// 是否没有映射点（所有 tick 同一速度）。
    #[must_use]
    pub fn is_constant(&self) -> bool {
        self.points.is_empty()
    }

    /// tick 0 的速度。
    #[must_use]
    pub const fn base_bpm(&self) -> f64 {
        self.base_bpm
    }

    /// 某个 tick 起生效的速度（映射点本身含在该段内）。
    #[must_use]
    pub fn bpm_at_tick(&self, tick: u64) -> f64 {
        self.points
            .range(..=tick)
            .next_back()
            .map_or(self.base_bpm, |(_, bpm)| *bpm)
    }

    /// tick → 帧（分段累加、**只取整一次**）。
    ///
    /// 空表 ⇒ 逐位等于 [`ticks_to_frames`]（兼容性承诺）。
    /// 病态输入（`ppq == 0`、`sample_rate == 0`、任一经过的 `bpm` 非有限或 `<= 0`）
    /// ⇒ `None`。结果饱和到 `u64::MAX`（⛔ 不回绕）。
    #[must_use]
    pub fn frames_at_tick(&self, tick: u64, ppq: u64, sample_rate: u32) -> Option<u64> {
        if self.points.is_empty() {
            return ticks_to_frames(tick, self.base_bpm, ppq, sample_rate);
        }
        if ppq == 0 || sample_rate == 0 || !valid_bpm(self.base_bpm) {
            return None;
        }
        if tick == 0 {
            return Some(0);
        }
        let mut frames = 0.0f64;
        let mut cursor = 0u64;
        let mut bpm = self.base_bpm;
        for (&point_tick, &point_bpm) in &self.points {
            if point_tick >= tick {
                break;
            }
            if !valid_bpm(point_bpm) {
                return None;
            }
            frames += tick_span_frames(point_tick - cursor, bpm, ppq, sample_rate)?;
            cursor = point_tick;
            bpm = point_bpm;
        }
        frames += tick_span_frames(tick - cursor, bpm, ppq, sample_rate)?;
        round_units(frames)
    }

    /// 两个 tick 之间真实经过的帧数（分段常速积分；`end_tick >= start_tick`）。
    ///
    /// 空表 ⇒ 逐位等于 [`ticks_to_frames`]`(end_tick - start_tick)`（兼容性承诺）。
    /// 非空表 ⇒ 用两个**绝对**帧位置相减，与引擎 `TickTimebase` 的
    /// `frames_at_tick(anchor + local) - frames_at_tick(anchor)` 同一口径；
    /// 不能直接把 `delta_ticks` 从 tick 0 起算 —— 那会忽略起点之前的变速段。
    ///
    /// `end_tick < start_tick`、病态速度/采样率 ⇒ `None`。
    #[must_use]
    pub fn frames_between_ticks(
        &self,
        start_tick: u64,
        end_tick: u64,
        ppq: u64,
        sample_rate: u32,
    ) -> Option<u64> {
        let delta_ticks = end_tick.checked_sub(start_tick)?;
        if self.points.is_empty() {
            return ticks_to_frames(delta_ticks, self.base_bpm, ppq, sample_rate);
        }
        let start_frame = self.frames_at_tick(start_tick, ppq, sample_rate)?;
        let end_frame = self.frames_at_tick(end_tick, ppq, sample_rate)?;
        end_frame.checked_sub(start_frame)
    }

    /// 从 `start_tick` 起、持续 `frames` 帧的跨度对应多少 tick。
    ///
    /// 空表 ⇒ `Round` 走 `#152` 的四舍五入、`Ceil` 走
    /// [`frames_to_ticks_ceil`]；两条都与旧实现逐位相同。
    /// 非空表 ⇒ 在 `[0, upper]` 上二分最小 `delta` 使
    /// [`frames_between_ticks`](Self::frames_between_ticks)`(start, start + delta) >= frames`；
    /// `Round` 再在 `delta` 与 `delta - 1` 之间选帧差更接近 `frames` 的一侧。
    ///
    /// `ppq == 0`、`sample_rate == 0`、任一经过的速度病态 ⇒ `None`。
    #[must_use]
    pub fn ticks_for_frames_from(
        &self,
        start_tick: u64,
        frames: u64,
        ppq: u64,
        sample_rate: u32,
        rounding: FrameToTickRounding,
    ) -> Option<u64> {
        if ppq == 0 || sample_rate == 0 || !valid_bpm(self.base_bpm) {
            return None;
        }
        if self.points.is_empty() {
            return match rounding {
                FrameToTickRounding::Ceil => {
                    frames_to_ticks_ceil(frames, ppq, self.base_bpm, sample_rate)
                }
                FrameToTickRounding::Round => {
                    let ticks = (frames as f64) * (ppq as f64) * self.base_bpm
                        / (f64::from(sample_rate) * 60.0);
                    if !ticks.is_finite() || ticks < 0.0 {
                        return None;
                    }
                    if ticks >= u64::MAX as f64 {
                        return Some(u64::MAX);
                    }
                    Some((ticks.round() as u64).max(1))
                }
            };
        }

        let min_bpm = self.points.values().copied().fold(self.base_bpm, f64::min);
        if !valid_bpm(min_bpm) {
            return None;
        }
        let mut high = frames_to_ticks_ceil(frames, ppq, min_bpm, sample_rate)?.saturating_add(4);
        // 极端帧数/进位边界：确保上界真的覆盖目标帧数。
        while self.frames_between_ticks(
            start_tick,
            start_tick.saturating_add(high),
            ppq,
            sample_rate,
        )? < frames
        {
            if high == u64::MAX {
                return None;
            }
            high = high.saturating_mul(2).max(1);
        }
        let mut low = 0u64;
        while low < high {
            let mid = low + (high - low) / 2;
            if self.frames_between_ticks(
                start_tick,
                start_tick.saturating_add(mid),
                ppq,
                sample_rate,
            )? >= frames
            {
                high = mid;
            } else {
                low = mid.saturating_add(1);
            }
        }
        if rounding == FrameToTickRounding::Round && low > 0 {
            let below = low - 1;
            let above_frames = self.frames_between_ticks(
                start_tick,
                start_tick.saturating_add(low),
                ppq,
                sample_rate,
            )?;
            let below_frames = self.frames_between_ticks(
                start_tick,
                start_tick.saturating_add(below),
                ppq,
                sample_rate,
            )?;
            let above_diff = above_frames.abs_diff(frames);
            let below_diff = below_frames.abs_diff(frames);
            if below_diff < above_diff {
                return Some(below.max(1));
            }
        }
        Some(low.max(1))
    }

    /// `offset_samples` 个采样在 `tick` 所在速度段对应多少 tick（可正可负）。
    ///
    /// 空表 ⇒ 与 `T-FIX-584` 的既有算式逐位相同
    /// （`round(offset × base_bpm × ppq / (60 × rate))`）。
    /// 非空表 ⇒ 用 `tick` 所在段的局部速度；采样偏移本身很小（相位对齐），
    /// 不跨段积分。`offset == 0` ⇒ `Some(0)`。
    #[must_use]
    pub fn tick_delta_for_samples_at(
        &self,
        tick: u64,
        offset_samples: i64,
        ppq: u64,
        sample_rate: u32,
    ) -> Option<i64> {
        if offset_samples == 0 {
            return Some(0);
        }
        if ppq == 0 || sample_rate == 0 {
            return None;
        }
        let bpm = self.bpm_at_tick(tick);
        if !valid_bpm(bpm) {
            return None;
        }
        let ticks = (offset_samples as f64) * bpm * (ppq as f64) / (60.0 * f64::from(sample_rate));
        if !ticks.is_finite() {
            return None;
        }
        if ticks >= i64::MAX as f64 {
            return Some(i64::MAX);
        }
        if ticks <= i64::MIN as f64 {
            return Some(i64::MIN);
        }
        Some(ticks.round() as i64)
    }

    /// 帧 → tick（**向下取整**；[`frames_at_tick`](Self::frames_at_tick) 的逆方向）。
    ///
    /// 用途：自动化曲线的**块起点**读数（每块最多 128 帧，块级取一次）。
    /// 空表路径与 `yeban-render` 既有的 `tick_at_frame` **同一条算式、同一个顺序**
    /// ⇒ 无变速工程的自动化读数逐位不变。
    #[must_use]
    pub fn ticks_at_frame(&self, frame: u64, ppq: u64, sample_rate: u32) -> Option<u64> {
        if ppq == 0 || sample_rate == 0 || !valid_bpm(self.base_bpm) {
            return None;
        }
        if self.points.is_empty() {
            return floor_ticks(frame, self.base_bpm, ppq, sample_rate);
        }
        #[allow(clippy::cast_precision_loss)]
        let frame = frame as f64;
        let mut frames_before = 0.0f64;
        let mut cursor = 0u64;
        let mut bpm = self.base_bpm;
        for (&point_tick, &point_bpm) in &self.points {
            if !valid_bpm(point_bpm) {
                return None;
            }
            let span_frames = tick_span_frames(point_tick - cursor, bpm, ppq, sample_rate)?;
            let next = frames_before + span_frames;
            if frame < next {
                let local = ticks_in_span(frame - frames_before, bpm, ppq, sample_rate)?;
                return Some(cursor.saturating_add(local.min(point_tick - cursor)));
            }
            frames_before = next;
            cursor = point_tick;
            bpm = point_bpm;
        }
        let local = ticks_in_span(frame - frames_before, bpm, ppq, sample_rate)?;
        Some(cursor.saturating_add(local))
    }

    /// tick → 微秒（分段累加、**只取整一次**；[`frames_at_tick`](Self::frames_at_tick)
    /// 的微秒域对偶，1 秒 = 1_000_000 µs）。
    ///
    /// 用途：硬件时间戳域的正向读数（例：某个 tick 在采集时间轴上是第几微秒）。
    /// 病态输入（`ppq == 0`、任一经过的 `bpm` 非有限或 `<= 0`）⇒ `None`。
    /// 结果饱和到 `u64::MAX`（⛔ 不回绕）。
    #[must_use]
    pub fn micros_at_tick(&self, tick: u64, ppq: u64) -> Option<u64> {
        if ppq == 0 || !valid_bpm(self.base_bpm) {
            return None;
        }
        if tick == 0 {
            return Some(0);
        }
        let mut micros = 0.0f64;
        let mut cursor = 0u64;
        let mut bpm = self.base_bpm;
        for (&point_tick, &point_bpm) in &self.points {
            if point_tick >= tick {
                break;
            }
            if !valid_bpm(point_bpm) {
                return None;
            }
            micros += tick_span_micros(point_tick - cursor, bpm, ppq)?;
            cursor = point_tick;
            bpm = point_bpm;
        }
        micros += tick_span_micros(tick - cursor, bpm, ppq)?;
        round_units(micros)
    }

    /// 微秒 → tick（**向下取整**；[`ticks_at_frame`](Self::ticks_at_frame) 的微秒域
    /// 对偶）。硬件 MIDI 时间戳（`midir` 微秒）落成工程 tick 的**唯一实现**
    /// （审计 #17：此前采集侧写死 60 BPM 的算式已删）。
    ///
    /// 空表路径与旧写死算式在 60 BPM 下**逐位相同**（判据以整数算式
    /// `µs × ppq / 1_000_000` 全域对账），非 60 BPM 工程按映射分段走查。
    /// 病态输入 ⇒ `None`（调用方映射成自己的哨兵，⛔ 不许悄悄回退到某个速度）。
    #[must_use]
    pub fn ticks_at_micros(&self, micros: u64, ppq: u64) -> Option<u64> {
        if ppq == 0 || !valid_bpm(self.base_bpm) {
            return None;
        }
        if self.points.is_empty() {
            return floor_ticks_micros(micros, self.base_bpm, ppq);
        }
        #[allow(clippy::cast_precision_loss)]
        let micros = micros as f64;
        let mut micros_before = 0.0f64;
        let mut cursor = 0u64;
        let mut bpm = self.base_bpm;
        for (&point_tick, &point_bpm) in &self.points {
            if !valid_bpm(point_bpm) {
                return None;
            }
            let span_micros = tick_span_micros(point_tick - cursor, bpm, ppq)?;
            let next = micros_before + span_micros;
            if micros < next {
                let local = ticks_in_span_micros(micros - micros_before, bpm, ppq)?;
                return Some(cursor.saturating_add(local.min(point_tick - cursor)));
            }
            micros_before = next;
            cursor = point_tick;
            bpm = point_bpm;
        }
        let local = ticks_in_span_micros(micros - micros_before, bpm, ppq)?;
        Some(cursor.saturating_add(local))
    }
}

/// 合法速度：有限且 `> 0`（与 [`ticks_to_frames`] 同口径，⛔ 不额外的范围钳制）。
#[must_use]
fn valid_bpm(bpm: f64) -> bool {
    bpm.is_finite() && bpm > 0.0
}

/// 一段 `delta_ticks` 在 `bpm` 下的**未取整**帧数。
///
/// 运算顺序是契约的一部分：`delta × 60 × sr / (bpm × ppq)`（先乘、后除）。
/// 只有 `+ - * /`，没有超越函数 ⇒ 跨平台逐位相同。
#[must_use]
fn tick_span_frames(delta_ticks: u64, bpm: f64, ppq: u64, sample_rate: u32) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    let delta = delta_ticks as f64;
    #[allow(clippy::cast_precision_loss)]
    let ppq = ppq as f64;
    let frames = delta * 60.0 * f64::from(sample_rate) / (bpm * ppq);
    frames.is_finite().then_some(frames)
}

/// 未取整的量（帧或微秒）→ `u64`（四舍五入、饱和；与 [`tick_to_sample`] 同一套哨兵）。
#[must_use]
fn round_units(frames: f64) -> Option<u64> {
    let rounded = frames.round();
    if !rounded.is_finite() || rounded < 0.0 {
        return None;
    }
    if rounded >= u64::MAX as f64 {
        return Some(u64::MAX);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rounded = rounded as u64;
    Some(rounded)
}

/// 一帧偏移 → 段内 tick（向下取整，夹到 `u64`）。
#[must_use]
fn ticks_in_span(frames: f64, bpm: f64, ppq: u64, sample_rate: u32) -> Option<u64> {
    #[allow(clippy::cast_precision_loss)]
    let ppq = ppq as f64;
    let ticks = frames * bpm * ppq / (60.0 * f64::from(sample_rate));
    if !ticks.is_finite() || ticks < 0.0 {
        return None;
    }
    if ticks >= u64::MAX as f64 {
        return Some(u64::MAX);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ticks = ticks.floor() as u64;
    Some(ticks)
}

/// 空表（恒定速度）的帧 → tick：与 `yeban-render::automation::tick_at_frame`
/// **同一条算式与顺序**（`frame × ppq × bpm / (sr × 60)`，向下取整）。
#[must_use]
fn floor_ticks(frame: u64, bpm: f64, ppq: u64, sample_rate: u32) -> Option<u64> {
    #[allow(clippy::cast_precision_loss)]
    let ticks = (frame as f64) * (ppq as f64) * bpm / (f64::from(sample_rate) * 60.0);
    if !ticks.is_finite() || ticks < 0.0 {
        return None;
    }
    if ticks >= u64::MAX as f64 {
        return Some(u64::MAX);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ticks = ticks.floor() as u64;
    Some(ticks)
}

/// 一秒的微秒数（微秒域换算的分母；以 `f64` 写出，求值顺序是契约的一部分）。
const MICROS_PER_SECOND: f64 = 1_000_000.0;

/// 一段 `delta_ticks` 在 `bpm` 下的**未取整**微秒数。
///
/// 运算顺序是契约的一部分：`delta × 60 × 1_000_000 / (bpm × ppq)`
/// （与 [`tick_span_frames`] 同形，先乘、后除）。只有 `+ - * /`，没有超越函数。
#[must_use]
fn tick_span_micros(delta_ticks: u64, bpm: f64, ppq: u64) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    let delta = delta_ticks as f64;
    #[allow(clippy::cast_precision_loss)]
    let ppq = ppq as f64;
    let micros = delta * 60.0 * MICROS_PER_SECOND / (bpm * ppq);
    micros.is_finite().then_some(micros)
}

/// 一段微秒偏移 → 段内 tick（向下取整，夹到 `u64`；[`ticks_in_span`] 的微秒域对偶）。
#[must_use]
fn ticks_in_span_micros(micros: f64, bpm: f64, ppq: u64) -> Option<u64> {
    #[allow(clippy::cast_precision_loss)]
    let ppq = ppq as f64;
    let ticks = micros * bpm * ppq / (60.0 * MICROS_PER_SECOND);
    if !ticks.is_finite() || ticks < 0.0 {
        return None;
    }
    if ticks >= u64::MAX as f64 {
        return Some(u64::MAX);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ticks = ticks.floor() as u64;
    Some(ticks)
}

/// 空表（恒定速度）的微秒 → tick：`µs × ppq × bpm / (1_000_000 × 60)`，向下取整。
///
/// ⚠ 求值顺序与 `yeban-midi-io::ticks_from_origin_us` 的恒定速度算式**逐字同形**
/// ⇒ 两侧在同一 `(µs, bpm)` 下逐位相等（`yeban-app` 有一条交叉对账判据钉住）。
/// 60 BPM 时逐位等于审计 #17 之前的整数算式 `µs × ppq / 1_000_000`
/// （判据以整数算式全域对账，见本模块测试）。
#[must_use]
fn floor_ticks_micros(micros: u64, bpm: f64, ppq: u64) -> Option<u64> {
    #[allow(clippy::cast_precision_loss)]
    let ticks = (micros as f64) * (ppq as f64) * bpm / (MICROS_PER_SECOND * 60.0);
    if !ticks.is_finite() || ticks < 0.0 {
        return None;
    }
    if ticks >= u64::MAX as f64 {
        return Some(u64::MAX);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ticks = ticks.floor() as u64;
    Some(ticks)
}

/// 每 tick 的帧数：`60 × sample_rate / (bpm × ppq)`（**唯一实现**）。
///
/// ⚠ 运算顺序是契约的一部分（见模块文档「为什么跨平台可复现」）：
/// **先**算这个比值，**再**由 [`tick_to_sample`] 乘 tick。改成
/// `tick × 60 / ppq / bpm × sample_rate` 是代数等价的，但会在 44.1 kHz 上产生
/// 一帧级分歧（`bpm=60, tick=168`：前者 7718、后者 7717，`#247`）。
///
/// 本函数**不做**病态兜底：`bpm <= 0`／非有限或 `ppq == 0` 会得到 `inf`/`NaN`，
/// 由 [`tick_to_sample`] 统一拒绝；缺省速度/采样率的兜底属于调用方（引擎保留
/// 120 BPM / 48 kHz 的既有语义，MCP 保留"病态 ⇒ 0"的既有语义）。
#[must_use]
pub fn samples_per_tick(bpm: f64, ppq: u64, sample_rate: u32) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let ppq = ppq as f64;
    60.0 * f64::from(sample_rate) / (bpm * ppq)
}

/// tick × 每 tick 帧数 → 帧位置（**四舍五入到最近帧，半值远离零**；唯一实现）。
///
/// 非有限或 `<= 0` 的 `samples_per_tick` 返回 `None`（调用方跳过该音符，
/// ⛔ 绝不 panic）。结果饱和到 `u64::MAX`（⛔ 不回绕成 0）。
#[must_use]
pub fn tick_to_sample(tick: u64, samples_per_tick: f64) -> Option<u64> {
    if !samples_per_tick.is_finite() || samples_per_tick <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let samples = (tick as f64 * samples_per_tick).round();
    if !samples.is_finite() || samples < 0.0 {
        return None;
    }
    if samples >= (u64::MAX as f64) {
        return Some(u64::MAX);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let samples = samples as u64;
    Some(samples)
}

/// tick → 帧（四舍五入到最近帧；**唯一实现**）。
///
/// 公式：`frames = round(tick × 60 × sample_rate / (bpm × ppq))`。
/// 求值顺序固定为：先 [`samples_per_tick`]，再 [`tick_to_sample`]。
/// ⚠ 不要改写成 `tick × 60 / ppq / bpm × sample_rate`：代数等价，但
/// 44.1 kHz 的 `tick=168, bpm=60` 会给出 **7717** 而不是 **7718** ——
/// 那正是 `#247` 收口前 MCP 与引擎的分歧来源。
///
/// 病态输入（`ppq == 0`、`bpm <= 0`／非有限、`sample_rate == 0`）返回 `None`：
/// 调用方各自映射成自己的哨兵（MCP 用 `0`，引擎在调用前已经把速度/采样率
/// 兜底成 120 BPM / 48 kHz）。`tick == 0` 返回 `Some(0)`；溢出饱和到
/// `u64::MAX`（⛔ 不回绕）。
///
/// # 例子
///
/// ```
/// use yeban_model::{PPQ, time_math::ticks_to_frames};
///
/// // 1 秒 @ 48 kHz @ 120 BPM / 960 PPQ。
/// assert_eq!(ticks_to_frames(1_920, 120.0, PPQ, 48_000), Some(48_000));
/// // 44.1 kHz 的半帧恰好落在 `.5`：半值远离零 ⇒ 7718（⛔ 不是 7717）。
/// assert_eq!(ticks_to_frames(168, 60.0, PPQ, 44_100), Some(7_718));
/// // 病态输入没有答案。
/// assert_eq!(ticks_to_frames(1, 0.0, PPQ, 48_000), None);
/// ```
#[must_use]
pub fn ticks_to_frames(tick: u64, bpm: f64, ppq: u64, sample_rate: u32) -> Option<u64> {
    if ppq == 0 || !bpm.is_finite() || bpm <= 0.0 || sample_rate == 0 {
        return None;
    }
    tick_to_sample(tick, samples_per_tick(bpm, ppq, sample_rate))
}

/// 帧 → tick（**向上取整**，下界 1）。
///
/// **唯一实现**（`#248`）：`yeban-mcp` 的 `yeban_import_audio`（`durationTicks`
/// 缺省值）与 `yeban-app` 的录音落点（`record_landing`）都调用这里，⛔ 不再各留
/// 一份 `ceil` 算式。`ceil` 而不是 `round`／`floor` 的理由见模块文档：它保护的是
/// **摆放时值的尾部**（向下取整会让最后几帧永远听不到）。
///
/// `sample_rate` 是**这些帧自己的**采样率（Hz）：`frames` 是它在那个速率下数出来的
/// 样本数，把它换成秒必须除它自己的速率。产品路径上录音 WAV 与工程同为 48 kHz
/// （`RecordOptions::default().sample_rate = 48_000`、工程缺省 48 kHz），但函数本身
/// 不假设两者相等 —— 用错速率会把时值算成另一个秒数。
///
/// 病态输入（`ppq == 0`、`bpm <= 0`／非有限、`sample_rate == 0`）返回 `None`：
/// 这里**没有**安全的哨兵值（0 会被 [`crate::ClipPlacement::validate`] 拒绝，
/// 1 会凭空造出一 tick 的素材）⇒ 调用方必须显式处置，⛔ 不许悄悄写一个 0。
///
/// # 例子
///
/// ```
/// use yeban_model::{PPQ, time_math::frames_to_ticks_ceil};
///
/// // 0.5 秒 @ 48 kHz @ 120 BPM：0.5 × 2 = 1 个四分音符 = 960 tick。
/// assert_eq!(frames_to_ticks_ceil(24_000, PPQ, 120.0, 48_000), Some(960));
/// // 0 帧仍给出下界 1（0 会被 ClipPlacement::validate 拒绝）。
/// assert_eq!(frames_to_ticks_ceil(0, PPQ, 120.0, 48_000), Some(1));
/// // 病态输入没有哨兵值。
/// assert_eq!(frames_to_ticks_ceil(1, PPQ, 0.0, 48_000), None);
/// ```
#[must_use]
pub fn frames_to_ticks_ceil(frames: u64, ppq: u64, bpm: f64, sample_rate: u32) -> Option<u64> {
    if ppq == 0 || !bpm.is_finite() || bpm <= 0.0 || sample_rate == 0 {
        return None;
    }
    let ticks = (frames as f64) * (ppq as f64) * bpm / (f64::from(sample_rate) * 60.0);
    if !ticks.is_finite() || ticks < 0.0 {
        return None;
    }
    if ticks >= u64::MAX as f64 {
        return Some(u64::MAX);
    }
    Some((ticks.ceil() as u64).max(1))
}

#[cfg(test)]
mod tests {
    use super::{
        FrameToTickRounding, TempoMap, frames_to_ticks_ceil, samples_per_tick, tick_to_sample,
        ticks_to_frames,
    };
    use crate::PPQ;

    /// 手算对照：`0.5 s @ 48 kHz @ 960 PPQ @ 120 BPM`。
    ///
    /// `seconds = 24000 / 48000 = 0.5`；`quarters = 0.5 × 120 / 60 = 1`；
    /// `ticks = 1 × 960 = 960` ⇒ 逐字相等（不是"约等于"）。
    #[test]
    fn half_a_second_at_48k_120bpm_is_exactly_960_ticks() {
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, 120.0, 48_000), Some(960));
    }

    /// 边界：0 帧 ⇒ 下界 1；1 帧 ⇒ 向上取整仍是 1。
    #[test]
    fn zero_frames_clamps_to_one_tick_and_one_frame_also_rounds_up_to_one() {
        assert_eq!(frames_to_ticks_ceil(0, PPQ, 120.0, 48_000), Some(1));
        assert_eq!(frames_to_ticks_ceil(1, PPQ, 120.0, 48_000), Some(1));
    }

    /// 边界：**非整 tick 向上取整**。
    ///
    /// 23 040 帧 @ 48 kHz = 0.48 s ⇒ 0.96 个四分音符 ⇒ 921.6 tick ⇒ **922**。
    /// 若取整方向写反（floor/round）会得到 921，尾部的 0.6 tick 被静默丢掉。
    #[test]
    fn a_fractional_tick_rounds_up_not_down() {
        assert_eq!(frames_to_ticks_ceil(23_040, PPQ, 120.0, 48_000), Some(922));
        // 4 800 帧 @ 48 kHz @ 128 BPM = 0.1 s ⇒ 0.21333… 四分音符 ⇒ 204.8 tick ⇒ 205。
        assert_eq!(frames_to_ticks_ceil(4_800, PPQ, 128.0, 48_000), Some(205));
    }

    /// 采样率是**素材自己的**：同一秒数换一个采样率，tick 必须相同。
    ///
    /// 0.5 s = 24 000 帧 @ 48 kHz = 22 050 帧 @ 44.1 kHz ⇒ 都是 960 tick。
    #[test]
    fn the_same_duration_at_a_different_rate_gives_the_same_ticks() {
        assert_eq!(frames_to_ticks_ceil(22_050, PPQ, 120.0, 44_100), Some(960));
    }

    /// 病态输入一律 `None`（没有哨兵值）。
    #[test]
    fn degenerate_inputs_have_no_answer() {
        assert_eq!(frames_to_ticks_ceil(24_000, 0, 120.0, 48_000), None);
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, 0.0, 48_000), None);
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, -1.0, 48_000), None);
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, f64::NAN, 48_000), None);
        assert_eq!(frames_to_ticks_ceil(24_000, PPQ, 120.0, 0), None);
    }

    /// 上界：换算结果溢出 `u64` 时饱和到 `u64::MAX`，⛔ 不回绕成 0。
    ///
    /// 取 `ppq = u64::MAX`（函数只拒绝 `ppq == 0`）⇒ 结果远超 `u64::MAX`。
    #[test]
    fn huge_inputs_saturate_instead_of_wrapping() {
        assert_eq!(
            frames_to_ticks_ceil(u64::MAX, u64::MAX, 1.0, 1),
            Some(u64::MAX)
        );
        assert_eq!(
            ticks_to_frames(u64::MAX, 120.0, PPQ, 48_000),
            Some(u64::MAX)
        );
    }

    /// tick → 帧的整数网格：三档采样率上 `1 秒 = sample_rate 帧`。
    ///
    /// 这些取值在收口前后**必须**逐字相同（48/96 kHz 是"摘要一个字节都不动"
    /// 的基线；44.1 kHz 的一秒也恰好落在整数上）。
    #[test]
    fn tick_to_frame_grid_is_exact_on_all_three_rates() {
        for rate in [44_100u32, 48_000, 96_000] {
            assert_eq!(
                ticks_to_frames(1_920, 120.0, PPQ, rate),
                Some(u64::from(rate)),
                "1 秒 @ {rate} Hz"
            );
        }
        assert_eq!(ticks_to_frames(960, 120.0, PPQ, 48_000), Some(24_000));
        assert_eq!(ticks_to_frames(2_048, 128.0, PPQ, 48_000), Some(48_000));
        // 1 tick @ 20 BPM = 3s / 960 = 0.003125s ⇒ 150 帧。
        assert_eq!(ticks_to_frames(1, 20.0, PPQ, 48_000), Some(150));
        // tick 0 是合法的 0（不是"病态输入的 0"）。
        assert_eq!(ticks_to_frames(0, 120.0, PPQ, 44_100), Some(0));
    }

    /// ⭐ `#247` 的**牙齿**：44.1 kHz 的 `.5` 必须向远离零的方向进位。
    ///
    /// `bpm=60, tick=168` 时每 tick 恰好 `45.9375` 帧、积恰好 `7717.5`：
    /// 四舍五入 ⇒ **7718**。旧的 MCP 顺序
    /// （`tick × 60 / ppq / bpm × sr`）在浮点上得到 `7717.4999999999995` ⇒ 7717，
    /// 那正是收口前 118 处分歧的第一例。
    ///
    /// 把 `tick_to_sample` 的 `.round()` 换成 `.trunc()`/银行家舍入、或把
    /// [`ticks_to_frames`] 改回旧的求值顺序，本判据都会变红。
    #[test]
    fn the_44_1_khz_half_frame_rounds_away_from_zero() {
        assert_eq!(
            samples_per_tick(60.0, PPQ, 44_100),
            45.9375,
            "每 tick 帧数必须是精确可表示的 45.9375"
        );
        assert_eq!(
            tick_to_sample(168, samples_per_tick(60.0, PPQ, 44_100)),
            Some(7_718),
            "7717.5 ⇒ 7718（半值远离零）"
        );
        assert_eq!(
            ticks_to_frames(168, 60.0, PPQ, 44_100),
            Some(7_718),
            "唯一实现的读数"
        );
        // 同一 tick / BPM 在 48/96 kHz 上是整数，收口前后都必须是这两个值。
        assert_eq!(ticks_to_frames(168, 60.0, PPQ, 48_000), Some(8_400));
        assert_eq!(ticks_to_frames(168, 60.0, PPQ, 96_000), Some(16_800));
    }

    /// 病态输入在三个新入口上一致地返回 `None`（⛔ 不 panic、⛔ 不发明 0）。
    #[test]
    fn tick_to_frame_degenerate_inputs_have_no_answer() {
        for (ppq, bpm, rate) in [
            (0u64, 120.0f64, 48_000u32),
            (PPQ, 0.0, 48_000),
            (PPQ, -120.0, 48_000),
            (PPQ, f64::NAN, 48_000),
            (PPQ, f64::INFINITY, 48_000),
            (PPQ, 120.0, 0),
        ] {
            assert_eq!(
                ticks_to_frames(1_000, bpm, ppq, rate),
                None,
                "{ppq}/{bpm}/{rate}"
            );
        }
        assert_eq!(tick_to_sample(1_000, 0.0), None);
        assert_eq!(tick_to_sample(1_000, -1.0), None);
        assert_eq!(tick_to_sample(1_000, f64::NAN), None);
        assert_eq!(tick_to_sample(1_000, f64::INFINITY), None);
    }

    /// **兼容性承诺**：空 `TempoMap` 与 [`ticks_to_frames`] 逐位相同。
    ///
    /// 覆盖病态与饱和分支；任何一格不等都意味着"没有变速的工程"会改变母带。
    #[test]
    fn an_empty_tempo_map_is_bit_identical_to_constant_bpm() {
        for bpm in [20.0f64, 60.0, 120.0, 128.0, 999.0, 0.0, -1.0, f64::NAN] {
            for rate in [44_100u32, 48_000, 96_000, 192_000, 0] {
                for tick in [0u64, 1, 168, 959, 960, 1_920, 7_680, 10_000_000] {
                    assert_eq!(
                        TempoMap::constant(bpm).frames_at_tick(tick, PPQ, rate),
                        ticks_to_frames(tick, bpm, PPQ, rate),
                        "bpm={bpm} rate={rate} tick={tick}"
                    );
                }
                let tick = 1_234_567u64;
                let expected = if bpm.is_finite() && bpm > 0.0 && rate != 0 {
                    #[allow(clippy::cast_precision_loss)]
                    let ticks = (tick as f64) * (PPQ as f64) * bpm / (f64::from(rate) * 60.0);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    Some(ticks.floor() as u64)
                } else {
                    None
                };
                assert_eq!(
                    TempoMap::constant(bpm).ticks_at_frame(tick, PPQ, rate),
                    expected,
                    "空表反向必须与恒定速度的 floor 公式一致: bpm={bpm} rate={rate}"
                );
            }
        }
    }

    /// 分段常速的正向：`120 → 240` 的变速点让后半段每 tick 只占一半帧。
    ///
    /// 手算 @ 48 kHz / 960 PPQ：tick 960 = 1 个四分音符 = 0.5 s = **24 000** 帧；
    /// tick 1920 的后半段（960 tick @ 240 BPM = 0.25 s）= **12 000** 帧 ⇒ 36 000。
    #[test]
    fn tempo_map_frames_are_piecewise_constant() {
        let mut points = std::collections::BTreeMap::new();
        points.insert(960u64, 240.0f64);
        let tempo = TempoMap::new(120.0, points);

        assert_eq!(tempo.bpm_at_tick(0), 120.0);
        assert_eq!(tempo.bpm_at_tick(959), 120.0);
        assert_eq!(tempo.bpm_at_tick(960), 240.0);
        assert_eq!(tempo.bpm_at_tick(10_000), 240.0);

        assert_eq!(tempo.frames_at_tick(960, PPQ, 48_000), Some(24_000));
        assert_eq!(tempo.frames_at_tick(1_920, PPQ, 48_000), Some(36_000));
        // 变速点之前与恒定 120 BPM 逐位相同（逐段累加的唯一一段）。
        assert_eq!(
            tempo.frames_at_tick(480, PPQ, 48_000),
            ticks_to_frames(480, 120.0, PPQ, 48_000)
        );
    }

    /// 反向：帧 → tick 是正向的逆（允许 1 tick 的块级取整误差）。
    #[test]
    fn tempo_map_inverse_tracks_the_forward_mapping() {
        let mut points = std::collections::BTreeMap::new();
        points.insert(960u64, 240.0f64);
        points.insert(2_880, 90.0);
        let tempo = TempoMap::new(120.0, points);
        for tick in [0u64, 1, 479, 960, 961, 1_920, 2_880, 5_760, 10_000] {
            let frame = tempo.frames_at_tick(tick, PPQ, 48_000).expect("合法");
            let back = tempo.ticks_at_frame(frame, PPQ, 48_000).expect("合法");
            assert!(
                back.abs_diff(tick) <= 1,
                "tick={tick} frame={frame} back={back}"
            );
        }
    }

    /// 病态映射点：任一经过的非有限/非正速度 ⇒ 整个换算没有答案。
    #[test]
    fn tempo_map_rejects_degenerate_points() {
        let mut points = std::collections::BTreeMap::new();
        points.insert(960u64, f64::NAN);
        let tempo = TempoMap::new(120.0, points);
        // 变速点之前：只是一段 120 BPM。
        assert_eq!(tempo.frames_at_tick(480, PPQ, 48_000), Some(12_000));
        // tick 恰在坏的变速点上：位置由之前的段决定（该点本身零长度），仍可回答。
        assert_eq!(tempo.frames_at_tick(960, PPQ, 48_000), Some(24_000));
        // 越过坏的变速点：该段无法求值 ⇒ 没有答案。
        assert_eq!(tempo.frames_at_tick(1_920, PPQ, 48_000), None);
        assert_eq!(
            TempoMap::constant(120.0).frames_at_tick(960, 0, 48_000),
            None
        );
        assert_eq!(TempoMap::constant(120.0).frames_at_tick(960, PPQ, 0), None);
    }

    /// 审计 #17 兼容性承诺：60 BPM 恒定速度下，微秒 → tick 与**旧写死整数算式**
    /// `µs × 960 / 1_000_000` 逐位相同（全域逐点对账 + 大值抽查）。
    #[test]
    fn micros_at_60_bpm_is_bit_identical_to_the_legacy_integer_formula() {
        let tempo = TempoMap::constant(60.0);
        for micros in 0..=2_000_000_u64 {
            assert_eq!(
                tempo.ticks_at_micros(micros, PPQ),
                Some(micros * 960 / 1_000_000),
                "60 BPM 逐位对账失败：µs={micros}"
            );
        }
        for micros in [62_500_u64, 123_456_789, 600_000_000, 3_600_000_000] {
            assert_eq!(
                tempo.ticks_at_micros(micros, PPQ),
                Some(micros * 960 / 1_000_000),
                "60 BPM 大值对账失败：µs={micros}"
            );
        }
    }

    /// 审计 #17 回归：120 BPM 下按速度手算 —— 0.5 秒 = 1 个四分音符 = 960 tick，
    /// 1 秒 = 1920 tick（旧写死算式会给出 480 / 960，落点缩水 50%）。
    #[test]
    fn micros_at_120_bpm_matches_hand_computed_ticks() {
        let tempo = TempoMap::constant(120.0);
        assert_eq!(tempo.ticks_at_micros(0, PPQ), Some(0));
        assert_eq!(tempo.ticks_at_micros(250_000, PPQ), Some(480));
        assert_eq!(tempo.ticks_at_micros(500_000, PPQ), Some(960));
        assert_eq!(tempo.ticks_at_micros(1_000_000, PPQ), Some(1_920));
        assert_eq!(tempo.micros_at_tick(960, PPQ), Some(500_000));
        assert_eq!(tempo.micros_at_tick(1_920, PPQ), Some(1_000_000));
    }

    /// 变速映射：基准 60 BPM、tick 960 起 120 BPM —— 分段走查两侧一致。
    ///
    /// 手算：tick 960 在 1_000_000 µs（60 BPM 一秒一拍）；其后再过 0.5 秒
    /// （120 BPM 一拍）到 tick 1920 ⇒ 1_500_000 µs。
    #[test]
    fn micros_walk_tempo_points_in_both_directions() {
        let mut points = std::collections::BTreeMap::new();
        points.insert(960u64, 120.0);
        let tempo = TempoMap::new(60.0, points);
        assert_eq!(tempo.micros_at_tick(960, PPQ), Some(1_000_000));
        assert_eq!(tempo.micros_at_tick(1_920, PPQ), Some(1_500_000));
        assert_eq!(tempo.ticks_at_micros(1_000_000, PPQ), Some(960));
        assert_eq!(tempo.ticks_at_micros(1_250_000, PPQ), Some(1_440));
        assert_eq!(tempo.ticks_at_micros(1_500_000, PPQ), Some(1_920));
        // 往返：只在**整段边界与整拍**上核（正向只取整一次、反向向下取整，
        // 非整除的 tick 与帧域一样不保证逐点往返 —— 那不是本换算的契约）。
        for tick in [0_u64, 480, 960, 1_440, 1_920, 2_880] {
            let micros = tempo.micros_at_tick(tick, PPQ).expect("正向必须有答案");
            assert_eq!(
                tempo.ticks_at_micros(micros, PPQ),
                Some(tick),
                "往返失败：tick={tick}"
            );
        }
    }

    /// 微秒域病态输入：与帧域同口径，没有答案（⛔ 不发明回退速度）。
    #[test]
    fn micros_degenerate_inputs_have_no_answer() {
        assert_eq!(TempoMap::constant(0.0).ticks_at_micros(1_000, PPQ), None);
        assert_eq!(TempoMap::constant(-60.0).micros_at_tick(960, PPQ), None);
        assert_eq!(
            TempoMap::constant(f64::NAN).ticks_at_micros(1_000, PPQ),
            None
        );
        assert_eq!(TempoMap::constant(120.0).ticks_at_micros(1_000, 0), None);
        assert_eq!(TempoMap::constant(120.0).micros_at_tick(0, PPQ), Some(0));
        // 正向（tick → µs）只校验**经过**的点：坏点之前照常回答（与帧域正向同口径）。
        let mut points = std::collections::BTreeMap::new();
        points.insert(960u64, f64::NAN);
        let tempo = TempoMap::new(60.0, points);
        assert_eq!(tempo.micros_at_tick(480, PPQ), Some(500_000));
        assert_eq!(tempo.micros_at_tick(1_920, PPQ), None);
        // 反向（µs → tick）走查时逐点校验：坏点存在即没有答案（与帧域反向同口径）。
        assert_eq!(tempo.ticks_at_micros(500_000, PPQ), None);
    }

    /// 两段速度夹具：120 BPM 到 tick 960，之后 240 BPM。
    fn two_speed_map() -> TempoMap {
        let mut points = std::collections::BTreeMap::new();
        points.insert(960u64, 240.0);
        TempoMap::new(120.0, points)
    }

    /// 分段积分：从 tick 0 到 1920 经过 24000 + 12000 = 36000 帧，不是 48000。
    #[test]
    fn frames_between_ticks_integrates_across_tempo_points() {
        let tempo = two_speed_map();
        assert_eq!(
            tempo.frames_between_ticks(0, 1_920, PPQ, 48_000),
            Some(36_000)
        );
        assert_eq!(
            tempo.frames_between_ticks(960, 1_920, PPQ, 48_000),
            Some(12_000)
        );
        assert_eq!(
            tempo.frames_between_ticks(0, 960, PPQ, 48_000),
            Some(24_000)
        );
        // 空表兼容性：与旧 `ticks_to_frames(delta)` 逐位相同。
        let constant = TempoMap::constant(120.0);
        assert_eq!(
            constant.frames_between_ticks(0, 1_920, PPQ, 48_000),
            ticks_to_frames(1_920, 120.0, PPQ, 48_000)
        );
        // 起点不是 0 时也必须按绝对位置相减（不能把 delta 从 0 起算）。
        let mut points = std::collections::BTreeMap::new();
        points.insert(960u64, 240.0);
        let tempo = TempoMap::new(120.0, points);
        assert_eq!(
            tempo.frames_between_ticks(480, 1_920, PPQ, 48_000),
            Some(24_000)
        );
    }

    /// 帧 → tick 跨度：36000 帧从 0 起 ⇒ 1920 tick（旧按 120 BPM 得 2880）。
    #[test]
    fn ticks_for_frames_from_integrates_across_tempo_points() {
        let tempo = two_speed_map();
        assert_eq!(
            tempo.ticks_for_frames_from(0, 36_000, PPQ, 48_000, FrameToTickRounding::Ceil),
            Some(1_920)
        );
        assert_eq!(
            tempo.ticks_for_frames_from(960, 12_000, PPQ, 48_000, FrameToTickRounding::Ceil),
            Some(960)
        );
        // 空表兼容性：录音/导入两条旧算式逐位不变。
        let constant = TempoMap::constant(120.0);
        assert_eq!(
            constant.ticks_for_frames_from(7, 24_000, PPQ, 48_000, FrameToTickRounding::Ceil),
            frames_to_ticks_ceil(24_000, PPQ, 120.0, 48_000)
        );
        let rounded = constant
            .ticks_for_frames_from(0, 12_345, PPQ, 48_000, FrameToTickRounding::Round)
            .expect("合法输入必须有答案");
        let old = (12_345.0 * 960.0 * 120.0 / (60.0 * 48_000f64)).round() as u64;
        assert_eq!(rounded, old.max(1), "Round 空表必须逐位等于旧 #152 算式");
    }

    /// 采样偏移：同一 0.5 s 偏移在 120/240 段分别换 960/1920 tick。
    #[test]
    fn sample_offset_uses_the_local_tempo_at_the_anchor_tick() {
        let tempo = two_speed_map();
        assert_eq!(
            tempo.tick_delta_for_samples_at(0, 24_000, PPQ, 48_000),
            Some(960)
        );
        assert_eq!(
            tempo.tick_delta_for_samples_at(960, 24_000, PPQ, 48_000),
            Some(1_920)
        );
        assert_eq!(
            tempo.tick_delta_for_samples_at(960, -24_000, PPQ, 48_000),
            Some(-1_920)
        );
        assert_eq!(
            tempo.tick_delta_for_samples_at(960, 0, PPQ, 48_000),
            Some(0)
        );
        // 空表逐位等于 T-FIX-584 旧算式。
        let constant = TempoMap::constant(128.0);
        assert_eq!(
            constant.tick_delta_for_samples_at(0, 2_400, PPQ, 48_000),
            Some(((2_400.0f64 * 128.0 * 960.0 / (60.0 * 48_000.0)).round()) as i64)
        );
    }
}
