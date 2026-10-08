//! **节拍器咔哒声**：构造期算好点击波形与拍栅格，实时侧只做"比对位置 + 乘加"。
//! [ARCH-DET-001, MUST-GATE-001, MODEL-AST-002]
//!
//! ## 为什么分成"构造期"与"逐样本"两半
//!
//! [MUST-GATE-001] 禁止音频回调里做任何分配/释放/加锁/阻塞 I/O，[ADR-0001 D32]
//! 禁止把**超越函数类**（`sin`/`cos`/`exp`/`powf`）放进逐样本路径。
//! 咔哒声恰好同时踩到这两条：
//!
//! | 阶段 | 线程 | 允许做的事 | 本模块的产物 |
//! | :--- | :--- | :--- | :--- |
//! | 构造期 | 控制线程 | 分配、`sin`、任意浮点 | [`ClickWave`]（`sin` 波形 + 二次衰减包络，峰值归一到 `1.0`）与 [`MetronomePlan`] 的拍栅格 |
//! | 逐样本 | 音频线程 | 只读快照 + IEEE 精确类乘加 | [`render_quantum`]：整数 tick 比对 + 一次乘 + 两次加 |
//!
//! 与声相表（[`crate::snapshot::TrackParams::pan_gains`]）、主总线推子
//! （[`crate::snapshot::EngineSnapshot::master_gain`]）是同一个形状：**超越函数只在构造期**。
//!
//! ## 拍栅格为什么用 tick 而不是帧
//!
//! 走带是**唯一**的时钟事实源（[`crate::transport`]）：位置是 960 PPQ 的整数 tick，
//! 由整数有理数 `tick_num / tick_den` 逐帧精确推进。拍点是 tick 栅格上的整数点
//! （一拍 = `PPQ × 4 / 分母` tick），因此：
//!
//! - **改 BPM 不移动拍栅格**（tick 与速度无关）⇒ 只有咔哒声的**帧位置**随速度变；
//! - 帧位置由 [`Transport::frames_until_tick`] 反算（整数 `div_ceil`），
//!   因此咔哒声落在**采样点精确**的拍边界上，而不是"最近的量子边界"；
//! - 没有任何浮点累加 ⇒ 长曲子里不漂移 [ARCH-DET-001]。
//!
//! ## 混音位置
//!
//! 调用点在 [`crate::rt::EngineRuntime::render_block`] 的步骤 3a'：
//! **母线求和之后、主总线推子与母线限制器之前**。理由见该调用点的注释。
//!
//! ## 关掉时逐位不变
//!
//! 快照里 `metronome_enabled = false` ⇒ `EngineSnapshot::metronome()` 是 `None`
//! ⇒ 实时侧 `armed_metronome_enabled = false` ⇒ **整段咔哒声代码不被执行**
//! ⇒ 输出与接线前**逐字节相同**（这是本切片的硬判据，见 `tests/metronome_render.rs`）。

use yeban_model::PPQ;
use yeban_model::TimeSignature;

use crate::block::{AudioBlock, DEFAULT_BLOCK_FRAMES};
use crate::transport::Transport;

/// 咔哒声的基频 (Hz)。
///
/// 构造期用它算 `sin` 波形；实时侧只看到采样值。
pub const CLICK_HZ: f32 = 1_000.0;

/// 咔哒声时长（毫秒）。
///
/// 4 ms 在 48 kHz 上是 192 帧：短到不掩盖音乐，长到听得清音高。
pub const CLICK_MS: u32 = 4;

/// 咔哒声波形的**最大**长度（帧）。
///
/// 192 kHz × 4 ms = 768 ⇒ 1024 留裕度。定长数组是实时安全的前提
/// （[`ClickWave`] 随快照 `Arc` 共享，音频线程不再分配）。
pub const MAX_CLICK_FRAMES: usize = 1_024;

/// 强拍（小节第一拍）增益。
pub const STRONG_GAIN: f32 = 0.5;

/// 弱拍增益。
///
/// 取 `STRONG_GAIN / 2` ⇒ 强弱只差 **−6 dB**，且比值 `2.0` 在 `f32` 里**精确**
/// （两个值都是 2 的幂），判据因此可以逐位断言。
pub const WEAK_GAIN: f32 = 0.25;

/// 单个量子内最多触发的拍数。
///
/// 处理量子是 128 帧（[ARCH-DET-001]），而模型允许的最短一拍是
/// `PPQ × 4 / 32 = 120` tick：在 `MAX_BPM = 999` 下也 ≥ 2400 帧
/// ⇒ 真实上限是 **1**。这里取 4 只是纵深：超出部分**不静默**地少打一拍
/// （下一量子的陈旧对齐会把游标拉回拍栅格，绝不补打一串）。
pub const MAX_BEATS_PER_QUANTUM: usize = 4;

/// 空闲游标：不小于任何合法 `click_len` ⇒ "当前没有咔哒声在响"。
const IDLE_CURSOR: usize = usize::MAX;

/// 构造期算好的**咔哒声波形**（单声道，峰值归一到 `1.0`）。
///
/// - 首样本与末样本都是 `0.0`：`sin(0) = 0`，包络在末点归零 ⇒ 起止都不产生阶跃
///   （"受保护的短包络"的字面含义）；
/// - 峰值**恰好** `1.0`（最大样本除以它自己 ⇒ IEEE 精确）；
/// - 立体声两声道加同一个值（咔哒声没有声相）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClickWave {
    samples: [f32; MAX_CLICK_FRAMES],
    len: usize,
}

impl ClickWave {
    /// 按采样率构造：`len = sample_rate × CLICK_MS / 1000`（钳到 `1..=MAX_CLICK_FRAMES`）。
    ///
    /// `sample_rate == 0` 按 48 kHz（与 [`crate::transport::Transport::arm`] 同兜底口径）。
    /// `sin` 与除法都只在这里发生 —— 实时侧拿不到这个函数。
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = if sample_rate == 0 {
            48_000
        } else {
            sample_rate
        };
        let len = usize::try_from(u64::from(sample_rate) * u64::from(CLICK_MS) / 1_000)
            .unwrap_or(MAX_CLICK_FRAMES)
            .clamp(1, MAX_CLICK_FRAMES);
        let mut samples = [0.0f32; MAX_CLICK_FRAMES];
        let span = (len - 1).max(1) as f32;
        let mut peak = 0.0f32;
        for (index, sample) in samples[..len].iter_mut().enumerate() {
            // 二次衰减包络 `(1 - t)²`：t = 0 ⇒ 1，t = 1 ⇒ **恰好** 0。
            let t = index as f32 / span;
            let envelope = (1.0 - t) * (1.0 - t);
            let phase = core::f32::consts::TAU * CLICK_HZ * index as f32 / sample_rate as f32;
            *sample = phase.sin() * envelope;
            peak = peak.max(sample.abs());
        }
        if peak > 0.0 {
            for sample in &mut samples[..len] {
                *sample /= peak;
            }
        }
        Self { samples, len }
    }

    /// 有效样本数（帧）。
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// 波形是否为空（**恒为 `false`**；`len` 被钳到 `≥ 1`）。
    ///
    /// 存在的理由：`clippy::len_without_is_empty`。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 有效样本（实时侧只读；长度 = [`Self::len`]）。
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        &self.samples[..self.len]
    }
}

/// 构造期算好的**节拍器计划**：点击波形 + 拍栅格（tick）。
///
/// 它随 [`crate::snapshot::EngineSnapshot`] 走（`Arc` 共享、不可变），
/// 因此音频线程读到的波形与拍栅格**来自同一份快照**（换快照时两者一起换）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetronomePlan {
    click: ClickWave,
    ticks_per_beat: u64,
    beats_per_bar: u64,
}

impl MetronomePlan {
    /// 按采样率与拍号构造。
    ///
    /// ## 一拍是多少 tick（这是一个**口径**，不是规范原文）
    ///
    /// 一拍 = 拍号**分母**所表示的音符：`4 / 分母` 个四分音符
    /// ⇒ `ticks_per_beat = PPQ × 4 / 分母`。
    ///
    /// - `4/4` ⇒ `960` tick（四分音符）；
    /// - `6/8` ⇒ `480` tick（八分音符）；
    /// - `3/2` ⇒ `1920` tick（二分音符）。
    ///
    /// `TimeSignature` 里**没有**"附点/beat grouping"这一维（模型层没有这个字段）
    /// ⇒ `6/8` 按八分音符打 6 下，而**不是**按附点四分打 2 下。这是如实的选择，
    /// 不是遗漏；要改口径必须先给模型加字段（本切片不改模型）。
    ///
    /// 分母来自模型的合法集合 `{1,2,4,8,16,32}` ⇒ 除法**恒精确**（`3840 / d`）。
    /// 非法输入（0）在这里兜底成 1，绝不产生 0 分母。
    #[must_use]
    pub fn new(sample_rate: u32, time_signature: TimeSignature) -> Self {
        let denominator = u64::from(time_signature.denominator.max(1));
        let ticks_per_beat = (PPQ * 4 / denominator).max(1);
        Self {
            click: ClickWave::new(sample_rate),
            ticks_per_beat,
            beats_per_bar: u64::from(time_signature.numerator.max(1)),
        }
    }

    /// 点击波形（构造期算好；实时侧只查表）。
    #[must_use]
    pub const fn click(&self) -> &ClickWave {
        &self.click
    }

    /// 一拍是多少 tick（`PPQ × 4 / 分母`）。
    #[must_use]
    pub const fn ticks_per_beat(&self) -> u64 {
        self.ticks_per_beat
    }

    /// 一小节多少拍（= 拍号分子）。
    #[must_use]
    pub const fn beats_per_bar(&self) -> u64 {
        self.beats_per_bar
    }
}

/// 实时侧的节拍器状态（**挥发性运行态**：像走带位置一样不进快照）。
///
/// 只有 4 个字段，全是整数/`f32`：不持有波形（波形从当前快照读），
/// 因此这个结构体可以随 `EngineRuntime` 一起 `Copy`/重置，不涉及任何分配。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetronomeVoice {
    /// 当前咔哒声播放到第几个样本（`IDLE_CURSOR` = 没有在响）。
    cursor: usize,
    /// 当前咔哒声的增益（强/弱拍，触发时决定）。
    gain: f32,
    /// 下一个要触发的拍（tick；`0` = 从 tick 0 起的下拍）。
    next_beat_tick: u64,
    /// 累计触发过的咔哒声次数（诊断/覆盖度判据用）。
    clicks: u64,
}

impl Default for MetronomeVoice {
    fn default() -> Self {
        Self::new()
    }
}

impl MetronomeVoice {
    /// 空闲的开始状态：没有咔哒声在响、下一拍是 tick 0。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cursor: IDLE_CURSOR,
            gain: 0.0,
            next_beat_tick: 0,
            clicks: 0,
        }
    }

    /// 关掉咔哒声：丢掉正在衰减的尾巴（`clicks` 与 `next_beat_tick` 保留）。
    ///
    /// 走带停住与"快照里关掉节拍器"都必须走这里：前者要求停住即静音，
    /// 后者要求"关掉"与"从未开过"**逐位**相同（残留 4 ms 尾巴会破坏这条）。
    pub fn silence(&mut self) {
        self.cursor = IDLE_CURSOR;
        self.gain = 0.0;
    }

    /// 把下一拍对齐到 `position_ticks` **之后（含）**的第一个拍点。
    ///
    /// 用在定位（`SeekTicks`）上：不然从第 8 小节跳回第 1 小节之后，
    /// 咔哒声会一直等到第 9 小节的强拍才再响。
    pub fn resync(&mut self, position_ticks: u64, ticks_per_beat: u64) {
        self.next_beat_tick = next_beat_at_or_after(position_ticks, ticks_per_beat);
    }

    /// 累计触发过的咔哒声次数。
    #[must_use]
    pub const fn clicks(&self) -> u64 {
        self.clicks
    }

    /// 下一个要触发的拍（tick）。
    #[must_use]
    pub const fn next_beat_tick(&self) -> u64 {
        self.next_beat_tick
    }

    /// 当前咔哒声游标是否空闲（诊断用；`true` = 没有在响）。
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        self.cursor == IDLE_CURSOR
    }
}

/// `position_ticks` 之后（含）的第一个拍点 tick。
///
/// `ticks_per_beat == 0` 时原样返回位置（不发明栅格、不除零）。
fn next_beat_at_or_after(position_ticks: u64, ticks_per_beat: u64) -> u64 {
    if ticks_per_beat == 0 {
        return position_ticks;
    }
    position_ticks
        .div_ceil(ticks_per_beat)
        .saturating_mul(ticks_per_beat)
}

/// 该拍是强拍（小节第一拍）还是弱拍。
fn accent_gain(tick: u64, ticks_per_beat: u64, beats_per_bar: u64) -> f32 {
    if ticks_per_beat == 0 || beats_per_bar == 0 {
        return WEAK_GAIN;
    }
    if (tick / ticks_per_beat).is_multiple_of(beats_per_bar) {
        STRONG_GAIN
    } else {
        WEAK_GAIN
    }
}

/// 把一个量子的咔哒声混进**已经汇流**的立体声母线块。
///
/// ## 这个函数就是"逐样本路径"
///
/// 它只做四件事：整数比较、一次 `f32` 乘、两次 `f32` 加、整数自增。
/// **没有**分配、释放、锁、I/O、日志、除法、超越函数 [MUST-GATE-001, ADR-0001 D32]。
/// 唯一的整数除法在**每拍一次**的拍点扫描里（`frames_until_tick` 与
/// [`accent_gain`]），不在逐样本循环里。
///
/// ## 覆盖的行为
///
/// - 走带停住 ⇒ [`MetronomeVoice::silence`] 并**立即返回**（停住即静音，不留尾巴）；
/// - `voice.next_beat_tick` 落在播放头**之后**时（停住期间过拍、或定位回退）
///   ⇒ 就地重新对齐，绝不补打一串；
/// - 咔哒声跨量子延续：游标是状态，本量子放不下的尾巴在下一个量子继续。
pub fn render_quantum(
    voice: &mut MetronomeVoice,
    plan: &MetronomePlan,
    transport: &Transport,
    block: &mut AudioBlock<DEFAULT_BLOCK_FRAMES>,
    frames: usize,
) {
    if frames == 0 {
        return;
    }
    if !transport.is_playing() {
        voice.silence();
        return;
    }

    let ticks_per_beat = plan.ticks_per_beat();
    let beats_per_bar = plan.beats_per_bar();
    let position = transport.position_ticks();
    if voice.next_beat_tick < position {
        // 陈旧对齐（停住期间时间没走、或上一份快照的速度不同）：拉回拍栅格。
        voice.resync(position, ticks_per_beat);
    }

    // --- 1) 拍点扫描（**每拍一次**，不是每样本一次）---
    let mut starts = [0usize; MAX_BEATS_PER_QUANTUM];
    let mut gains = [0.0f32; MAX_BEATS_PER_QUANTUM];
    let mut count = 0usize;
    let mut tick = voice.next_beat_tick;
    while count < MAX_BEATS_PER_QUANTUM {
        let offset = transport.frames_until_tick(tick);
        if offset >= frames as u64 {
            break;
        }
        starts[count] = usize::try_from(offset).unwrap_or(usize::MAX);
        gains[count] = accent_gain(tick, ticks_per_beat, beats_per_bar);
        count += 1;
        tick = tick.saturating_add(ticks_per_beat);
    }
    voice.next_beat_tick = tick;

    // --- 2) 逐样本：比对当前位置 + 混一个受保护的短包络 ---
    let click = plan.click().samples();
    let click_len = click.len();
    let (left, right) = block.stereo_mut();
    let usable = frames.min(left.len()).min(right.len());
    let mut next_start = 0usize;
    for frame in 0..usable {
        if next_start < count && starts[next_start] == frame {
            voice.cursor = 0;
            voice.gain = gains[next_start];
            voice.clicks = voice.clicks.wrapping_add(1);
            next_start += 1;
        }
        if voice.cursor < click_len {
            let sample = click[voice.cursor] * voice.gain;
            left[frame] += sample;
            right[frame] += sample;
            voice.cursor += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(numerator: u8, denominator: u8) -> TimeSignature {
        TimeSignature {
            numerator,
            denominator,
        }
    }

    /// 波形：峰值**恰好** `1.0`，首末样本**恰好** `0.0`（受保护的短包络）。
    #[test]
    fn click_wave_peaks_at_one_and_starts_ends_at_zero() {
        for sample_rate in [44_100u32, 48_000, 96_000] {
            let wave = ClickWave::new(sample_rate);
            let samples = wave.samples();
            let peak = samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
            assert_eq!(peak, 1.0, "{sample_rate} Hz: 峰值必须归一化到 1.0");
            assert_eq!(samples[0], 0.0, "首样本必须恰好为 0（sin(0) × 1）");
            assert_eq!(
                *samples.last().expect("非空"),
                0.0,
                "末样本必须恰好为 0（包络 (1-t)² 在 t=1 归零）"
            );
            assert_eq!(
                wave.len(),
                (sample_rate as usize * CLICK_MS as usize) / 1_000,
                "{sample_rate} Hz: 长度 = 采样率 × 4 ms"
            );
        }
        // 非法采样率兜底：不 panic、不产生 0 长度。
        assert_eq!(ClickWave::new(0).len(), 192);
    }

    /// 拍栅格：`PPQ × 4 / 分母`（4/4 = 960、6/8 = 480、3/2 = 1920）。
    #[test]
    fn grid_follows_the_time_signature_denominator() {
        for (numerator, denominator, ticks_per_beat) in [
            (4u8, 4u8, 960u64),
            (6, 8, 480),
            (3, 2, 1_920),
            (5, 4, 960),
            (7, 16, 240),
        ] {
            let plan = MetronomePlan::new(48_000, signature(numerator, denominator));
            assert_eq!(
                plan.ticks_per_beat(),
                ticks_per_beat,
                "{numerator}/{denominator}"
            );
            assert_eq!(plan.beats_per_bar(), u64::from(numerator));
        }
        // 非法分母不除零。
        assert_eq!(
            MetronomePlan::new(48_000, signature(4, 0)).ticks_per_beat(),
            3_840
        );
    }

    /// 强弱拍判定：只有小节第一拍是强拍。
    #[test]
    fn only_the_first_beat_of_a_bar_is_strong() {
        assert_eq!(accent_gain(0, 960, 4), STRONG_GAIN);
        assert_eq!(accent_gain(960, 960, 4), WEAK_GAIN);
        assert_eq!(accent_gain(2_880, 960, 4), WEAK_GAIN);
        assert_eq!(accent_gain(3_840, 960, 4), STRONG_GAIN, "下一小节第一拍");
        assert_eq!(accent_gain(0, 0, 4), WEAK_GAIN, "退化栅格不除零");
    }

    /// 对齐：`ceil` 到拍栅格；已对齐时**原地**（不跳到下一拍）。
    #[test]
    fn resync_lands_on_the_bar_grid() {
        assert_eq!(next_beat_at_or_after(0, 960), 0);
        assert_eq!(next_beat_at_or_after(1, 960), 960);
        assert_eq!(next_beat_at_or_after(960, 960), 960);
        assert_eq!(next_beat_at_or_after(961, 960), 1_920);
        assert_eq!(next_beat_at_or_after(7, 0), 7);
    }

    /// 咔哒声**真的**落在拍边界上：一个 128 帧量子里的偏移 == 拍点的绝对帧。
    #[test]
    fn clicks_land_on_the_exact_beat_frame() {
        // 128 BPM / 48 kHz / 4-4 ⇒ 一拍 22500 帧（= 960 tick）。
        let plan = MetronomePlan::new(48_000, signature(4, 4));
        let transport = Transport::free_running(48_000, 128.0);
        let mut voice = MetronomeVoice::new();
        let mut block = AudioBlock::<DEFAULT_BLOCK_FRAMES>::new();
        // 第 1 个量子：tick 0 是拍点 ⇒ 偏移 0。
        block.set_frames(DEFAULT_BLOCK_FRAMES);
        render_quantum(
            &mut voice,
            &plan,
            &transport,
            &mut block,
            DEFAULT_BLOCK_FRAMES,
        );
        assert_eq!(voice.clicks(), 1, "tick 0 的首拍必须触发");
        assert_eq!(block.left()[0], 0.0, "首样本是包络零点");
        assert!(block.left()[1] != 0.0, "第 2 个样本起必须真的出声");
        // 音高：一个周期 48 帧（1000 Hz @48 kHz）⇒ 峰值落在第 12 个样本附近。
        // 波形峰值在构造期归一化到**恰好** 1.0 ⇒ 乘上强拍增益仍是精确的 `f32`。
        // ⚠ 4 ms 的咔哒声（192 帧 @48 kHz）比一个量子的 128 帧长 ⇒ 本量子只放得下前半。
        let click_len = plan.click().len().min(block.left().len());
        let peak = block.left()[..click_len]
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert_eq!(peak, STRONG_GAIN, "强拍峰值 = STRONG_GAIN");
        assert_eq!(block.left()[12], block.right()[12], "左右同值（无声相）");
    }

    /// 停住 ⇒ 立即静音（不留 4 ms 尾巴）且不再触发。
    #[test]
    fn stopped_transport_is_silent_and_does_not_retrigger() {
        let plan = MetronomePlan::new(48_000, signature(4, 4));
        let mut transport = Transport::free_running(48_000, 128.0);
        let mut voice = MetronomeVoice::new();
        let mut block = AudioBlock::<DEFAULT_BLOCK_FRAMES>::new();
        block.set_frames(DEFAULT_BLOCK_FRAMES);
        render_quantum(
            &mut voice,
            &plan,
            &transport,
            &mut block,
            DEFAULT_BLOCK_FRAMES,
        );
        assert_eq!(voice.clicks(), 1);
        assert!(!voice.is_idle(), "咔哒声正在响");

        transport.apply(crate::ring::TransportCommand::Stop);
        block.silence();
        block.set_frames(DEFAULT_BLOCK_FRAMES);
        render_quantum(
            &mut voice,
            &plan,
            &transport,
            &mut block,
            DEFAULT_BLOCK_FRAMES,
        );
        assert_eq!(voice.clicks(), 1, "停住不许再触发");
        assert!(voice.is_idle(), "停住必须丢掉尾巴");
        assert_eq!(block.left()[0], 0.0);
    }
}
