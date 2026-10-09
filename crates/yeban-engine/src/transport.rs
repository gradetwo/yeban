//! **确定性走带状态机**（960 PPQ 整数 tick）+ RT → 控制侧的原子读数镜面。
//! [ARCH-RT-001, ARCH-DET-001, ROAD-M2-001, MODEL-ISO-001]
//!
//! ## 为什么它住在 `yeban-engine` 而且是"纯状态机"
//!
//! - 播放头位置按 [MODEL-ISO-001] 属于**会话运行态**（第二层），**不得**进入
//!   `YebanProjectV1`，也不得进入不可变的 [`EngineSnapshot`](crate::snapshot::EngineSnapshot)
//!   —— 快照是模型投影，携带挥发性走带状态会让"同一工程 ⇒ 同一快照"这条不变量失效。
//! - 本模块**零 GUI 依赖、零 cpal 依赖**：它只用 `core::sync::atomic` 与
//!   [`TransportCommand`]（`ring.rs` 的 `Copy` 事件载荷），因此在
//!   `--no-default-features` 下就能编译并跑全部判据（本机可跑的那一侧）。
//!
//! ## tick 转换公式（本模块的全部数学）
//!
//! ```text
//! 960 PPQ ⇒ 1 四分音符 = 960 tick
//! 每帧推进的 tick 数 r = bpm / 60 × PPQ / sample_rate = bpm × 16 / sample_rate
//! ```
//!
//! 实时侧**不用** `f64`：速度在**控制面**一次性量化成整数有理数
//!
//! ```text
//! tick_num = bpm_micro × PPQ          （bpm_micro = round(bpm × 1_000_000)）
//! tick_den = 60 × 1_000_000 × sample_rate
//! ⇒ r = tick_num / tick_den           （与上面的 f64 式在 1e-6 BPM 内相等）
//! ```
//!
//! 位置推进是**整数精确的带余除法**（余数就是 r 的小数部分，逐帧精确携带）：
//!
//! ```text
//! total      = remainder + frames × tick_num          （u128，整数）
//! Δtick      = total / tick_den                       （整数除法，向下取整）
//! remainder  = total % tick_den                       （< tick_den，精确的相位余数）
//! position  += Δtick
//! ```
//!
//! ### 为什么整数精确
//!
//! 1. **没有浮点加法律**：`position` 是整数，`remainder` 是整数，`tick_num`/`tick_den`
//!    是整数。整个推进路径上只有整数乘/加/除/取余 —— 它们由语言语义**完全确定**
//!    （ADR-0001 D32 的"IEEE 精确类"里最严格的一档：整数运算没有架构差异可言）。
//! 2. **不累加误差**：每一帧的分数部分都被**精确地**放进 `remainder`，而不是每帧
//!    丢掉一次小数。因此 N 帧之后的位置**恰好**是 `floor(Σ frames_i × tick_num / tick_den)`
//!    = `floor(N × tick_num / tick_den)` —— 与"一次性算 N 帧"逐位相同。
//!    浮点累加（`position += frames as f64 * r`）做不到这一点：每一步都舍入，
//!    N 步的误差是 O(√N) 级的随机游走（注入实验见台账）。
//! 3. **不随采样率漂移**：`sample_rate` 只出现在 `tick_den` 里，而且**没有任何常量按
//!    48 kHz 硬编码**（判据 `positions_do_not_assume_48khz` 用 44.1 kHz 逐点核对）。
//!    更换采样率只是换一个分母 ⇒ 位置仍然等于该采样率下的 `floor(N×r)`。
//!
//! ### 速度变化为什么不跳变
//!
//! [`Transport::arm`] 在快照边界重算 `tick_num`/`tick_den`，并把旧的相位余数**精确换算**到
//! 新分母（`remainder × den_new / den_old`，向下取整，误差 < 1/den_new 个 tick）。
//! `position_ticks` **一个整数都不动** ⇒ BPM 变化那一瞬间位置连续（判据
//! `tempo_change_never_jumps_the_position`）。
//!
//! ## 实时安全（红线 7）
//!
//! [`Transport`] 的每一个方法都只做整数运算与分支：**不分配、不释放、不加锁、不做 I/O**。
//! 控制侧要发命令时走**既有**的无锁 SPSC 批量通道（[`crate::ring`] 的
//! `EngineEvent::Transport`），实时侧在量子边界出队时调用 [`Transport::apply`]。
//! 反向的读数（RT → UI）走 [`TransportMirror`]：**原子量 + 版本号 seqlock**，
//! 写者（RT）只写不读、从不等待；读者（UI/控制面）在版本号变化时重读。
//!
//! 判据 `transport_rt_zero_alloc`（`harness = false` 的计数型分配器）实测窗口内
//! `allocations == 0 && deallocations == 0`。

use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use yeban_model::PPQ;
use yeban_model::project::{DEFAULT_BPM, MAX_BPM, MIN_BPM};

use crate::ring::TransportCommand;

/// 速度量化分母（"微 BPM"）：进实时侧之前 `bpm` 被量化成 `1/1_000_000` BPM。
///
/// 量化误差 < 1e-6 BPM（相对误差 < 5e-8），远小于任何可听差异；换来的是实时侧
/// **完全没有浮点**。`DEFAULT_BPM`/`MIN_BPM`/`MAX_BPM` 都远大于 1e-6 ⇒ 量化不会把
/// 合法速度压成 0。
pub const BPM_SCALE: u64 = 1_000_000;

/// 走带状态。
///
/// `Recording` 已**预留**但本切片不产生它：录音路径（`ARCH-REC-*`）还没有实现
/// （见 `docs/ledger/feature-alignment.md` 的分组 E），因此枚举里有这一档、
/// 状态机里没有入口 —— 不假装支持录音，也不为将来重新设计状态表示。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransportState {
    /// 停住：时钟不推进、输出静音、位置保留（见 [`TransportCommand::Stop`]）。
    #[default]
    Stopped,
    /// 播放中：每量子按转换公式推进 tick。
    Playing,
    /// 录音中（**预留**；本切片没有任何入口能到达这一档）。
    Recording,
}

impl TransportState {
    /// 稳定的机器可读名字（日志 / 判据 / MCP 报告用；不随枚举顺序变化）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Playing => "playing",
            Self::Recording => "recording",
        }
    }

    /// 该状态是否在推进时钟。
    #[must_use]
    pub const fn is_running(self) -> bool {
        matches!(self, Self::Playing | Self::Recording)
    }

    /// 原子量编码（`u8`）。
    ///
    /// `pub(crate)`：跨线程镜像（[`crate::stats_mirror`]）要存同一个状态 ⇒
    /// **复用这一张表**，不复制第二份编码（复制出来的第二份会漂移）。
    /// `match` 是穷举的 ⇒ 新增枚举变体会在这里**编译失败**。
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Stopped => 0,
            Self::Playing => 1,
            Self::Recording => 2,
        }
    }

    /// 原子量解码：未知字节按 `Stopped`（保守：宁可显示停住，也不假装在播）。
    pub(crate) const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Playing,
            2 => Self::Recording,
            _ => Self::Stopped,
        }
    }
}

/// 速度的**有理数**表示：`tick_num / tick_den` = 每帧推进的 tick 数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tempo {
    tick_num: u64,
    tick_den: u64,
}

impl Tempo {
    /// 由 `bpm` + 采样率算出有理数（控制面语义：**这里**允许一次 `f64` 量化）。
    fn from_bpm(bpm: f64, sample_rate: u32) -> Self {
        let bpm_micro = quantise_bpm(bpm);
        let sample_rate = if sample_rate == 0 {
            48_000
        } else {
            sample_rate
        };
        Self {
            tick_num: bpm_micro.saturating_mul(PPQ),
            tick_den: 60u64
                .saturating_mul(BPM_SCALE)
                .saturating_mul(u64::from(sample_rate)),
        }
    }

    /// 回到 `f64` 的 BPM（只用于报告/判据；实时路径上不调用）。
    ///
    /// `r = tick_num/tick_den`（每帧 tick 数）⇒ `bpm = r × 60 × sample_rate / PPQ`。
    /// 注意 `sample_rate` 是**独立**的一项：`tick_den` 里含采样率，所以
    /// `tick_num/tick_den` 不能单独还原 BPM（这是第一版写错的地方，判据
    /// `invalid_tempo_inputs_fall_back_deterministically` 当场抓到）。
    fn bpm(self, sample_rate: u32) -> f64 {
        if self.tick_den == 0 {
            return DEFAULT_BPM;
        }
        #[allow(clippy::cast_precision_loss)]
        let bpm = self.tick_num as f64 * 60.0 * f64::from(sample_rate)
            / (self.tick_den as f64 * PPQ as f64);
        bpm
    }

    /// `frames` 帧包含多少个 tick（向下取整；纯函数，判据直接钉住它）。
    fn ticks_for(self, frames: u64) -> u64 {
        if self.tick_den == 0 {
            return 0;
        }
        let total = u128::from(frames) * u128::from(self.tick_num);
        u64::try_from(total / u128::from(self.tick_den)).unwrap_or(u64::MAX)
    }

    /// tick → 帧（**四舍五入**，与投影层 `synth::tick_to_sample` 的 `.round()` 同口径）。
    fn frames_for(self, tick: u64) -> u64 {
        if self.tick_num == 0 {
            return 0;
        }
        let num = u128::from(tick) * u128::from(self.tick_den);
        let half = u128::from(self.tick_num) / 2;
        u64::try_from((num + half) / u128::from(self.tick_num)).unwrap_or(u64::MAX)
    }
}

/// `bpm` → 微 BPM 整数（非有限值退回 [`DEFAULT_BPM`]，越界钳到模型范围）。
fn quantise_bpm(bpm: f64) -> u64 {
    let bpm = if bpm.is_finite() {
        bpm.clamp(MIN_BPM, MAX_BPM)
    } else {
        DEFAULT_BPM
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let micro = (bpm * BPM_SCALE as f64).round() as u64;
    micro.max(1)
}

/// 一条走带命令在实时侧产生的**后果**（`Copy`：出队路径上不产生任何 Drop）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportEffect {
    /// 命令没有改变任何状态（重复 `Play`/重复 `Stop`）。
    None,
    /// 状态变成了 `StateChanged`。
    StateChanged(TransportState),
    /// 位置被定位：实时侧必须把合成器播放头对齐到 `frames`，
    /// 否则"定位到 tick t"只改数字、不出该处的声音。
    Seeked {
        /// 目标 tick（960 PPQ）。
        tick: u64,
        /// 该 tick 对应的绝对帧（四舍五入，与投影层同口径）。
        frames: u64,
    },
}

/// 走带状态机（**实时侧持有的可变状态**；由 [`crate::rt::EngineRuntime`] 拥有）。
///
/// 默认是 [`Transport::free_running`]（Playing）：本切片接入之前，`process_quantum`
/// 的语义就是"快照一发布就从 tick 0 起滚"，因此**未收到任何走带命令时行为逐位不变**
/// （既有判据全绿，见台账 §5）。要"加载即停住"的控制面发一条 `Stop` 即可
/// （`yeban-app::engine_host` 就是这么做的），不存在第二套状态源。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transport {
    state: TransportState,
    position_ticks: u64,
    position_frames: u64,
    /// 相位余数（`< tick_den`）：tick 的小数部分，逐帧精确携带 ⇒ 不漂移。
    remainder: u64,
    tempo: Tempo,
    sample_rate: u32,
    commands_applied: u64,
    quanta_played: u64,
    ticks_advanced: u64,
    last_quantum_ticks: u64,
}

impl Transport {
    /// **自由跑**的走带：构造即 `Playing`，位置 `tick 0`。
    ///
    /// 这是 `EngineRuntime` 的默认值，用来保持"接入走带之前"的行为不变。
    #[must_use]
    pub fn free_running(sample_rate: u32, bpm: f64) -> Self {
        Self::new(TransportState::Playing, sample_rate, bpm)
    }

    /// **停住**的走带：构造即 `Stopped`，位置 `tick 0`。
    #[must_use]
    pub fn stopped(sample_rate: u32, bpm: f64) -> Self {
        Self::new(TransportState::Stopped, sample_rate, bpm)
    }

    fn new(state: TransportState, sample_rate: u32, bpm: f64) -> Self {
        let sample_rate = if sample_rate == 0 {
            48_000
        } else {
            sample_rate
        };
        Self {
            state,
            position_ticks: 0,
            position_frames: 0,
            remainder: 0,
            tempo: Tempo::from_bpm(bpm, sample_rate),
            sample_rate,
            commands_applied: 0,
            quanta_played: 0,
            ticks_advanced: 0,
            last_quantum_ticks: 0,
        }
    }

    /// 快照边界武装：按新快照的采样率与速度重算有理数。
    ///
    /// **位置与状态都不动**（`position_ticks` 逐位保留）⇒ 换快照 / 改 BPM 不跳变；
    /// 相位余数按新旧分母精确换算（误差 < 1 个 tick 的 `1/tick_den`）。
    pub fn arm(&mut self, sample_rate: u32, bpm: f64) {
        let sample_rate = if sample_rate == 0 {
            48_000
        } else {
            sample_rate
        };
        let next = Tempo::from_bpm(bpm, sample_rate);
        if next == self.tempo && sample_rate == self.sample_rate {
            return;
        }
        // 余数换分母：remainder_new = floor(remainder_old × den_new / den_old)。
        // 用 u128 中间量避免溢出（den 量级 ~1e13，remainder < den）。
        if self.tempo.tick_den != 0 && self.remainder != 0 {
            let scaled = u128::from(self.remainder) * u128::from(next.tick_den)
                / u128::from(self.tempo.tick_den);
            self.remainder = u64::try_from(scaled)
                .unwrap_or(0)
                .min(next.tick_den.saturating_sub(1));
        }
        self.tempo = next;
        self.sample_rate = sample_rate;
    }

    /// 在量子边界应用一条走带命令（**实时路径**：只有分支与整数运算）。
    pub fn apply(&mut self, command: TransportCommand) -> TransportEffect {
        self.commands_applied = self.commands_applied.saturating_add(1);
        match command {
            TransportCommand::Play => {
                if self.state == TransportState::Playing {
                    TransportEffect::None
                } else {
                    self.state = TransportState::Playing;
                    TransportEffect::StateChanged(self.state)
                }
            }
            // `Stop` 与 `Pause` 在本状态机里是**同一件事**：停住并**保留位置**。
            // "回到起始点"不是停止的语义，而是 `Stop` 之后的一条 `SeekTicks(0)`
            // （UI 的停止按钮就是这么发的，见 `yeban-app::engine_host::stop_and_rewind`）。
            TransportCommand::Stop | TransportCommand::Pause => {
                if self.state == TransportState::Stopped {
                    TransportEffect::None
                } else {
                    self.state = TransportState::Stopped;
                    TransportEffect::StateChanged(self.state)
                }
            }
            TransportCommand::SeekTicks(tick) => {
                self.seek(tick);
                TransportEffect::Seeked {
                    tick,
                    frames: self.position_frames,
                }
            }
        }
    }

    /// 定位到 `tick`（位置**恰好**是 `tick`，相位余数清零）。
    ///
    /// 状态不变：播放中定位 ⇒ 从新位置继续播；停住时定位 ⇒ 下次 `Play` 从新位置起。
    pub fn seek(&mut self, tick: u64) {
        self.position_ticks = tick;
        self.position_frames = self.tempo.frames_for(tick);
        self.remainder = 0;
        self.last_quantum_ticks = 0;
    }

    /// 推进 `frames` 帧，返回本量子推进的 tick 数。
    ///
    /// **非 Running 状态恒返回 0 且位置不动** —— 这是判据 ⑩（负向判据）的被测对象。
    pub fn advance_frames(&mut self, frames: u64) -> u64 {
        if !self.state.is_running() || frames == 0 || self.tempo.tick_den == 0 {
            self.last_quantum_ticks = 0;
            return 0;
        }
        let total =
            u128::from(self.remainder) + u128::from(frames) * u128::from(self.tempo.tick_num);
        let denominator = u128::from(self.tempo.tick_den);
        let ticks = u64::try_from(total / denominator).unwrap_or(u64::MAX);
        self.remainder = u64::try_from(total % denominator).unwrap_or(0);
        self.position_ticks = self.position_ticks.saturating_add(ticks);
        self.position_frames = self.position_frames.saturating_add(frames);
        self.quanta_played = self.quanta_played.saturating_add(1);
        self.ticks_advanced = self.ticks_advanced.saturating_add(ticks);
        self.last_quantum_ticks = ticks;
        ticks
    }

    /// 当前状态。
    #[must_use]
    pub const fn state(&self) -> TransportState {
        self.state
    }

    /// 是否在推进时钟。
    #[must_use]
    pub const fn is_playing(&self) -> bool {
        self.state.is_running()
    }

    /// 当前绝对位置（960 PPQ 整数 tick）。
    #[must_use]
    pub const fn position_ticks(&self) -> u64 {
        self.position_ticks
    }

    /// 当前绝对位置（帧；与位置 tick **同步**推进，同一量子内两者描述同一个瞬间）。
    #[must_use]
    pub const fn position_frames(&self) -> u64 {
        self.position_frames
    }

    /// 当前相位余数（tick 的小数部分，`< tick_den`）。诊断/判据用。
    #[must_use]
    pub const fn remainder(&self) -> u64 {
        self.remainder
    }

    /// 本量子推进的 tick 数（最近一次 [`Self::advance_frames`] 的返回值的副本）。
    #[must_use]
    pub const fn last_quantum_ticks(&self) -> u64 {
        self.last_quantum_ticks
    }

    /// 武装进去的速度（`f64` BPM，量化到 1e-6 后的值）。诊断/判据用。
    #[must_use]
    pub fn bpm(&self) -> f64 {
        self.tempo.bpm(self.sample_rate)
    }

    /// 武装进去的采样率。
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 累计应用的走带命令条数（含幂等命令）。
    #[must_use]
    pub const fn commands_applied(&self) -> u64 {
        self.commands_applied
    }

    /// 累计推进过的量子数（只在 Running 时自增）。
    #[must_use]
    pub const fn quanta_played(&self) -> u64 {
        self.quanta_played
    }

    /// 累计推进过的 tick 数（= 位置的首 tick 起点之外的全部增量）。
    #[must_use]
    pub const fn ticks_advanced(&self) -> u64 {
        self.ticks_advanced
    }

    /// `frames` 帧对应多少 tick（**判据用的纯函数**，与 `advance_frames` 同一份有理数）。
    #[must_use]
    pub fn ticks_for_frames(&self, frames: u64) -> u64 {
        self.tempo.ticks_for(frames)
    }

    /// `tick` 对应哪一帧（四舍五入；seek 与投影层对齐用的同一份有理数）。
    #[must_use]
    pub fn frames_for_tick(&self, tick: u64) -> u64 {
        self.tempo.frames_for(tick)
    }

    /// 从当前位置到 `tick` **还差多少帧**（向上取整；整数精确）。
    ///
    /// 语义：这是 [`Self::advance_frames`] 的逆运算 —— 返回值是使
    /// `position_ticks >= tick` 成立的**最小**帧数 `f`。也就是说，调用方在
    /// 当前位置推进 `f` 帧，播放头就**恰好**第一次碰到 `tick`。
    ///
    /// 用途：节拍器要把咔哒声放在**采样点精确**的拍边界上（[`crate::metronome`]），
    /// 而"拍"是 tick 栅格上的整数点。实时侧因此只做整数乘/除/取余：
    /// **没有**浮点、没有超越函数、没有分配/锁/I-O [MUST-GATE-001]。
    ///
    /// 推导（与 [`Self::advance_frames`] 同一份有理数）：
    ///
    /// ```text
    /// advance_frames(f) ⇒ position_ticks += floor((remainder + f × tick_num) / tick_den)
    /// 要求 floor((remainder + f × tick_num) / tick_den) >= tick − position_ticks
    /// ⇔ remainder + f × tick_num >= Δtick × tick_den
    /// ⇔ f >= (Δtick × tick_den − remainder) / tick_num
    /// ⇒ f = ceil(…)
    /// ```
    ///
    /// - `tick <= position_ticks` ⇒ `0`（已经到达或越过；不回绕、不返回负数）；
    /// - 速度有理数退化（`tick_num == 0` 或 `tick_den == 0`）⇒ `u64::MAX`
    ///   （"永远到不了"的显式表示，而不是猜一个帧数）。
    #[must_use]
    pub fn frames_until_tick(&self, tick: u64) -> u64 {
        let position = self.position_ticks;
        if tick <= position {
            return 0;
        }
        if self.tempo.tick_num == 0 || self.tempo.tick_den == 0 {
            return u64::MAX;
        }
        // `tick > position` ⇒ `delta >= 1` ⇒ `target >= tick_den > remainder`
        // ⇒ 下面的减法恒不下溢（`remainder` 恒 `< tick_den`，见 `advance_frames`）。
        let delta = u128::from(tick - position);
        let target = delta * u128::from(self.tempo.tick_den);
        let numerator = target - u128::from(self.remainder);
        u64::try_from(numerator.div_ceil(u128::from(self.tempo.tick_num))).unwrap_or(u64::MAX)
    }

    /// 把当前读数发布到原子镜面（**实时路径**：只有原子写）。
    pub fn publish(&self, mirror: &TransportMirror) {
        mirror.store(self);
    }
}

/// 控制侧读到的走带读数（`Copy`，一致的一帧）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportReading {
    /// 状态。
    pub state: TransportState,
    /// 绝对位置（960 PPQ tick）。
    pub position_ticks: u64,
    /// 绝对位置（帧）。
    pub position_frames: u64,
    /// 实时侧累计应用的走带命令条数。
    pub commands_applied: u64,
    /// 实时侧累计推进的量子数。
    pub quanta_played: u64,
    /// 实时侧累计推进的 tick 数。
    pub ticks_advanced: u64,
}

impl TransportReading {
    /// 一个"还没有任何引擎"的中性读数（`EngineHost` 在 `reload` 之前用它）。
    #[must_use]
    pub const fn cold() -> Self {
        Self {
            state: TransportState::Stopped,
            position_ticks: 0,
            position_frames: 0,
            commands_applied: 0,
            quanta_played: 0,
            ticks_advanced: 0,
        }
    }
}

/// RT → 控制侧的走带读数镜面：原子量 + 版本号（seqlock）。
///
/// - 写者只有一个（实时线程），**只写不读**，因此永远不会被读者阻塞；
/// - 读者（UI/控制面）拿到的是**同一瞬间**的状态与位置（版本号配对），
///   不会出现"状态说 Playing、位置还是上一量子"的撕裂读数；
/// - 没有锁、没有分配：`TransportMirror::new()` 是唯一会分配的地方
///   （由控制线程在打开设备之前调用）。
#[derive(Debug)]
pub struct TransportMirror {
    version: AtomicU64,
    state: AtomicU8,
    position_ticks: AtomicU64,
    position_frames: AtomicU64,
    commands_applied: AtomicU64,
    quanta_played: AtomicU64,
    ticks_advanced: AtomicU64,
}

impl Default for TransportMirror {
    fn default() -> Self {
        Self::new()
    }
}

impl TransportMirror {
    /// 建一个全零的镜面（`Stopped` / tick 0）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: AtomicU64::new(0),
            state: AtomicU8::new(TransportState::Stopped.code()),
            position_ticks: AtomicU64::new(0),
            position_frames: AtomicU64::new(0),
            commands_applied: AtomicU64::new(0),
            quanta_played: AtomicU64::new(0),
            ticks_advanced: AtomicU64::new(0),
        }
    }

    /// **实时侧**发布一个读数：奇数版本 → 写字段 → 偶数版本（`Release`）。
    fn store(&self, transport: &Transport) {
        self.version.fetch_add(1, Ordering::Release);
        self.state.store(transport.state.code(), Ordering::Relaxed);
        self.position_ticks
            .store(transport.position_ticks, Ordering::Relaxed);
        self.position_frames
            .store(transport.position_frames, Ordering::Relaxed);
        self.commands_applied
            .store(transport.commands_applied, Ordering::Relaxed);
        self.quanta_played
            .store(transport.quanta_played, Ordering::Relaxed);
        self.ticks_advanced
            .store(transport.ticks_advanced, Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    /// 控制侧读取一个**一致**的读数。
    ///
    /// 版本号奇数 = 写者正在写 ⇒ 重读。写者从不等待读者，因此这里不可能死锁；
    /// 单线程（判据/控制面显式驱动）时第一次就能配对成功。
    #[must_use]
    pub fn read(&self) -> TransportReading {
        loop {
            let before = self.version.load(Ordering::Acquire);
            if before & 1 == 1 {
                core::hint::spin_loop();
                continue;
            }
            let reading = TransportReading {
                state: TransportState::from_code(self.state.load(Ordering::Relaxed)),
                position_ticks: self.position_ticks.load(Ordering::Relaxed),
                position_frames: self.position_frames.load(Ordering::Relaxed),
                commands_applied: self.commands_applied.load(Ordering::Relaxed),
                quanta_played: self.quanta_played.load(Ordering::Relaxed),
                ticks_advanced: self.ticks_advanced.load(Ordering::Relaxed),
            };
            if self.version.load(Ordering::Acquire) == before {
                return reading;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 ①：48 kHz、**每个量子 960 帧**下，播 N 个量子后的位置**恰等于**
    /// `floor(N × 960 × tick_num / tick_den)`（逐点断言，不是"约等于"）。
    ///
    /// 960 帧在 120 BPM / 48 kHz 下是 38.4 tick ⇒ 第 1 个量子末位置必须是 38（不是 38.4、
    /// 也不是 39）。
    #[test]
    fn positions_are_exact_integer_ticks_at_48khz() {
        let mut transport = Transport::free_running(48_000, 120.0);
        assert_eq!(transport.position_ticks(), 0);
        let mut previous = 0u64;
        let mut expected_positions = Vec::new();
        for quantum in 1..=8u64 {
            let ticks = transport.advance_frames(960);
            // 公式：floor(N × 960 × 1.152e11 / 2.88e12) = floor(N × 38.4)
            let numerator = u128::from(quantum) * 960 * 115_200_000_000u128;
            let want = u64::try_from(numerator / 2_880_000_000_000u128).unwrap();
            assert_eq!(
                transport.position_ticks(),
                want,
                "量子 {quantum}: 位置必须是公式的整数值"
            );
            assert_eq!(ticks, want - previous, "量子 {quantum}: 本量子增量");
            previous = want;
            expected_positions.push(want);
        }
        assert_eq!(
            expected_positions,
            vec![38, 76, 115, 153, 192, 230, 268, 307],
            "逐点读数：960 帧 = 38.4 tick ⇒ 位置序列就是 38/76/115/…"
        );
        assert_eq!(
            transport.position_ticks(),
            307,
            "8 个 960 帧量子 = 307.2 tick ⇒ 307"
        );
        assert_eq!(transport.position_frames(), 8 * 960);
        assert_eq!(transport.quanta_played(), 8);
    }

    /// 判据 ②：**不许**按 48 kHz 硬编码 —— 44.1 kHz 下位置仍然等于同一个"帧 → tick"公式。
    #[test]
    fn positions_do_not_assume_48khz() {
        for (sample_rate, denominator) in [
            (44_100u32, 2_646_000_000_000u128),
            (48_000, 2_880_000_000_000),
            (96_000, 5_760_000_000_000),
        ] {
            let mut transport = Transport::free_running(sample_rate, 120.0);
            for quantum in 1..=4u64 {
                transport.advance_frames(960);
                let want =
                    u64::try_from(u128::from(quantum) * 960 * 115_200_000_000u128 / denominator)
                        .unwrap();
                assert_eq!(
                    transport.position_ticks(),
                    want,
                    "{sample_rate} Hz 第 {quantum} 个量子"
                );
            }
        }
        // 44.1 kHz 的显式读数：960 帧 = 41.79… tick ⇒ 41 / 83 / 125 / 167。
        let mut transport = Transport::free_running(44_100, 120.0);
        let mut trajectory = Vec::new();
        for _ in 0..4 {
            transport.advance_frames(960);
            trajectory.push(transport.position_ticks());
        }
        assert_eq!(trajectory, vec![41, 83, 125, 167]);
    }

    /// 判据 ③：`stop` 之后再 `play` **从停住的位置继续** —— 不回 0、不跳一格。
    #[test]
    fn stop_then_play_resumes_from_the_frozen_position() {
        let mut transport = Transport::free_running(48_000, 120.0);
        for _ in 0..10 {
            transport.advance_frames(128);
        }
        let stopped_at = transport.position_ticks();
        assert!(stopped_at > 0);

        assert_eq!(
            transport.apply(TransportCommand::Stop),
            TransportEffect::StateChanged(TransportState::Stopped)
        );
        // 停住：再推 100 个量子，位置一个 tick 都不动（判据 ⑩ 的同一件事）。
        for _ in 0..100 {
            assert_eq!(transport.advance_frames(128), 0);
        }
        assert_eq!(transport.position_ticks(), stopped_at, "停止不得改变位置");

        assert_eq!(
            transport.apply(TransportCommand::Play),
            TransportEffect::StateChanged(TransportState::Playing)
        );
        assert_eq!(
            transport.position_ticks(),
            stopped_at,
            "Play 的瞬间位置必须还是停住的位置（不是 0、不是网格对齐值）"
        );
        transport.advance_frames(128);
        assert_eq!(
            transport.position_ticks(),
            stopped_at + transport.ticks_for_frames(128),
            "恢复播放后从停住的位置**继续**"
        );
        assert_ne!(stopped_at + transport.ticks_for_frames(128), 0);
    }

    /// 判据 ④：`seek(t)` 之后位置的**起点恰是 t**，并且从 t 起按公式推进。
    #[test]
    fn seek_sets_the_exact_origin() {
        let mut transport = Transport::free_running(48_000, 120.0);
        transport.advance_frames(5_000);
        assert_eq!(
            transport.apply(TransportCommand::SeekTicks(1_920)),
            TransportEffect::Seeked {
                tick: 1_920,
                frames: 48_000
            },
            "1920 tick @120BPM/48k = 48000 帧（与投影层 tick_to_sample 的 round 同口径）"
        );
        assert_eq!(transport.position_ticks(), 1_920);
        assert_eq!(transport.position_frames(), 48_000);
        assert_eq!(transport.remainder(), 0, "定位清相位余数");

        transport.advance_frames(960);
        assert_eq!(transport.position_ticks(), 1_920 + 38);
        // 停住时定位也必须生效（下次 Play 从 t 起）。
        transport.apply(TransportCommand::Stop);
        transport.apply(TransportCommand::SeekTicks(7));
        assert_eq!(transport.position_ticks(), 7);
        transport.advance_frames(9_999);
        assert_eq!(transport.position_ticks(), 7, "停住时推进无效");
        transport.apply(TransportCommand::Play);
        assert_eq!(transport.position_ticks(), 7, "Play 从 seek 的位置起");
    }

    /// 判据 ⑤：同一操作序列 ⇒ tick 轨迹**逐位相同**（两次运行的整条轨迹向量相等）。
    #[test]
    fn identical_command_sequences_yield_bit_identical_trajectories() {
        fn run() -> Vec<(u64, u64)> {
            let mut transport = Transport::stopped(44_100, 133.7);
            let mut trace = Vec::new();
            transport.apply(TransportCommand::Play);
            for step in 0..64u64 {
                if step == 17 {
                    transport.apply(TransportCommand::Stop);
                }
                if step == 23 {
                    transport.apply(TransportCommand::Play);
                }
                if step == 40 {
                    transport.apply(TransportCommand::SeekTicks(1_234));
                }
                transport.advance_frames(if step % 3 == 0 { 128 } else { 960 });
                trace.push((transport.position_ticks(), transport.remainder()));
            }
            trace
        }
        assert_eq!(run(), run(), "同一输入序列必须给出逐位相同的 tick 轨迹");
    }

    /// 判据：`frames_until_tick` 是 `advance_frames` 的**精确逆**（最小帧数）。
    ///
    /// 三件可证伪的事：
    /// 1. 整除速度（120 / 128 BPM @48 kHz）给出**字面**帧数（960 tick = 24000 / 22500 帧）；
    /// 2. 不整除速度（133.7 BPM）满足"最小性"：推进 `f` 帧到了、推进 `f-1` 帧没到；
    /// 3. 已到达/越过的目标返回 0（不回绕）。
    #[test]
    fn frames_until_tick_is_the_exact_inverse_of_advance_frames() {
        assert_eq!(
            Transport::free_running(48_000, 120.0).frames_until_tick(960),
            24_000,
            "120 BPM: 960 tick = 1 秒 = 48000 帧的一半"
        );
        assert_eq!(
            Transport::free_running(48_000, 128.0).frames_until_tick(960),
            22_500,
            "128 BPM: 60 × 48000 / 128"
        );
        assert_eq!(
            Transport::free_running(48_000, 128.0).frames_until_tick(0),
            0
        );

        // 最小性（不整除速度）：这是"贪心 +1"与"精确逆"的分界。
        for bpm in [133.7, 99.9, 187.3] {
            let mut transport = Transport::free_running(48_000, bpm);
            let frames = transport.frames_until_tick(960);
            assert!(frames > 0);
            let mut early = Transport::free_running(48_000, bpm);
            early.advance_frames(frames - 1);
            assert!(
                early.position_ticks() < 960,
                "{bpm} BPM: 推进 {} 帧就到了 ⇒ 不是最小帧数",
                frames - 1
            );
            transport.advance_frames(frames);
            assert_eq!(
                transport.position_ticks(),
                960,
                "{bpm} BPM: 推进 {frames} 帧必须**恰好**到达 960 tick"
            );
            assert_eq!(transport.frames_until_tick(960), 0, "到达之后是 0");
            assert_eq!(transport.frames_until_tick(959), 0, "越过的目标也是 0");
        }

        // 44.1 kHz 不按 48 kHz 硬编码：960 tick @120 BPM = 22050 帧。
        assert_eq!(
            Transport::free_running(44_100, 120.0).frames_until_tick(960),
            22_050
        );
    }

    /// 判据 ⑩（负向）：`Stop` 状态下的推进路径**不产生任何 tick 推进**。
    #[test]
    fn stopped_transport_never_advances() {
        let mut transport = Transport::stopped(48_000, 120.0);
        assert_eq!(transport.state(), TransportState::Stopped);
        for _ in 0..1_000 {
            assert_eq!(transport.advance_frames(128), 0);
        }
        assert_eq!(transport.position_ticks(), 0);
        assert_eq!(transport.position_frames(), 0);
        assert_eq!(transport.quanta_played(), 0, "停住时量子计数也不许动");
        assert_eq!(transport.ticks_advanced(), 0);
        // 幂等：重复 Stop 不产生状态变化（但命令本身计入 commands_applied）。
        assert_eq!(
            transport.apply(TransportCommand::Stop),
            TransportEffect::None
        );
        assert_eq!(
            transport.apply(TransportCommand::Pause),
            TransportEffect::None
        );
        assert_eq!(transport.commands_applied(), 2);
    }

    /// 判据 ⑪：BPM 变化**不跳变**（位置逐位保留），而且之后按新速度推进。
    #[test]
    fn tempo_change_never_jumps_the_position() {
        let mut transport = Transport::free_running(48_000, 120.0);
        for _ in 0..37 {
            transport.advance_frames(128);
        }
        let before = transport.position_ticks();
        let remainder_before = transport.remainder();
        assert!(
            remainder_before > 0,
            "夹具必须留下非零相位，否则这条判据会永真"
        );

        transport.arm(48_000, 140.0);
        assert_eq!(
            transport.position_ticks(),
            before,
            "改 BPM 的那一瞬间位置必须逐位不变"
        );
        // `tick_den = BPM_SCALE × 60 × sample_rate` **不含 bpm** ⇒ 改 BPM 只换分子，
        // 相位余数**逐位保留**（连换算都不需要）。这一条把它钉住。
        assert_eq!(
            transport.remainder(),
            remainder_before,
            "改 BPM 不换分母 ⇒ 相位余数必须逐位保留"
        );
        assert!((transport.bpm() - 140.0).abs() < 1e-6);

        let ticks = transport.advance_frames(128);
        // 140 BPM / 48 kHz：每帧 140×16/48000 tick ⇒ 128 帧 = 5.973 tick；
        // 再加上带过来的 0.44 tick 相位 ⇒ 本量子推进 **6**（"余数不丢"的直接后果）。
        assert_eq!(ticks, 6, "改速度后按新速度推进, 且相位余数参与进位");
        assert_eq!(transport.position_ticks(), before + 6);
        // 采样率变化**会**换分母 ⇒ 余数按分母比例换算，但位置仍然一位不动。
        let before = transport.position_ticks();
        transport.arm(44_100, 140.0);
        assert_eq!(transport.position_ticks(), before);
        assert_eq!(transport.sample_rate(), 44_100);
        assert!((transport.bpm() - 140.0).abs() < 1e-6);
    }

    /// 判据：整数带余除法**与一次性整段计算逐位相同**（这正是"不累加误差"的证明）。
    #[test]
    fn stepping_matches_one_shot_arithmetic_exactly() {
        for bpm in [20.0, 120.0, 133.7, 999.0] {
            for sample_rate in [44_100u32, 48_000, 96_000] {
                let mut stepped = Transport::free_running(sample_rate, bpm);
                let mut one_shot = Transport::free_running(sample_rate, bpm);
                let mut total_frames = 0u64;
                for step in 0..257u64 {
                    let frames = 1 + (step % 128);
                    stepped.advance_frames(frames);
                    total_frames += frames;
                }
                one_shot.advance_frames(total_frames);
                assert_eq!(
                    stepped.position_ticks(),
                    one_shot.position_ticks(),
                    "{bpm} BPM / {sample_rate} Hz: 分步与整段必须逐位相同"
                );
            }
        }
    }

    /// 判据：镜面读数是**一致的一帧**（状态与位置来自同一个量子），且默认中性。
    #[test]
    fn mirror_publishes_a_consistent_reading() {
        let mirror = TransportMirror::new();
        assert_eq!(mirror.read(), TransportReading::cold());
        let mut transport = Transport::free_running(48_000, 120.0);
        transport.advance_frames(128);
        transport.publish(&mirror);
        let reading = mirror.read();
        assert_eq!(reading.state, TransportState::Playing);
        assert_eq!(reading.position_ticks, transport.position_ticks());
        assert_eq!(reading.position_frames, transport.position_frames());
        assert_eq!(reading.quanta_played, 1);
        assert_eq!(reading.commands_applied, 0);

        transport.apply(TransportCommand::Stop);
        transport.publish(&mirror);
        let reading = mirror.read();
        assert_eq!(reading.state, TransportState::Stopped);
        assert_eq!(reading.commands_applied, 1);
        // 版本号必须是偶数（写者不落在"写一半"的状态上）。
        assert_eq!(mirror.version.load(Ordering::Relaxed) % 2, 0);
    }

    /// 判据：非法 BPM / 采样率的**兜底**是显式且确定的（不 panic、不产生 0 分母）。
    #[test]
    fn invalid_tempo_inputs_fall_back_deterministically() {
        for bpm in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1.0, 1e9] {
            let mut transport = Transport::free_running(0, bpm);
            assert_eq!(transport.sample_rate(), 48_000);
            assert!(transport.bpm() >= MIN_BPM && transport.bpm() <= MAX_BPM);
            transport.advance_frames(128);
            assert!(transport.position_ticks() < 1_000, "钳位之后不得爆炸");
        }
        assert_eq!(Transport::free_running(48_000, f64::NAN).bpm(), 120.0);
        assert_eq!(Transport::free_running(48_000, 5.0).bpm(), MIN_BPM);
        assert_eq!(Transport::free_running(48_000, 5_000.0).bpm(), MAX_BPM);
    }

    /// 判据：**同一条命令的返回值也幂等** —— 状态没变时 `apply` 必须报
    /// [`TransportEffect::None`]，只有真的换了状态才报 `StateChanged`。
    ///
    /// 量什么：`Transport::apply` 的返回值（枚举，逐变体比较）与状态（枚举）。
    ///
    /// ## 为什么单独立一条
    ///
    /// `EngineRuntime::render_block` 只消费 [`TransportEffect::Seeked`]（它去调
    /// `synth.seek`），`StateChanged` 被调用点丢弃 ⇒ 这条契约在**音频与位置读数上
    /// 不可观测**，`tests/idempotency_and_channel_consistency.rs` 的 ⑤-3（整段样本 +
    /// 位置读数逐位相同）因此对它是盲的。本票（engine-28）注入实测：把 `Play` 分支的
    /// `if self.state == TransportState::Playing` 改成 `if false`（⇒ 重复 `Play`
    /// 也报 `StateChanged`），全量 24 个目标全绿。
    ///
    /// ⚠ 覆盖边界：本判据钉的是**公开 API 的返回值**（`Transport` / `TransportEffect`
    /// 都是 `pub`，是 `yeban-engine` 对控制面的契约），不是 RT 路径上的行为 ——
    /// 后者由 ⑤-3 覆盖。
    #[test]
    fn reapplying_a_command_reports_no_state_change() {
        let mut transport = Transport::free_running(48_000, 120.0);
        assert_eq!(
            transport.state(),
            TransportState::Playing,
            "前提：自由跑的构造状态是 Playing"
        );
        assert_eq!(
            transport.apply(TransportCommand::Play),
            TransportEffect::None,
            "已经在播放 ⇒ 重复 Play 不得报状态变更"
        );
        assert_eq!(
            transport.apply(TransportCommand::Play),
            TransportEffect::None,
            "重复两次同样不得报状态变更"
        );
        assert_eq!(
            transport.apply(TransportCommand::Stop),
            TransportEffect::StateChanged(TransportState::Stopped),
            "真的换了状态 ⇒ 必须报 StateChanged"
        );
        assert_eq!(
            transport.apply(TransportCommand::Stop),
            TransportEffect::None,
            "已经停住 ⇒ 重复 Stop 不得报状态变更"
        );
        assert_eq!(
            transport.apply(TransportCommand::Pause),
            TransportEffect::None,
            "`Pause` 与 `Stop` 在本状态机里是同一件事 ⇒ 停住时同样是 None"
        );
        assert_eq!(
            transport.apply(TransportCommand::Play),
            TransportEffect::StateChanged(TransportState::Playing),
            "从停住恢复播放 ⇒ 必须报 StateChanged"
        );
        assert_eq!(
            transport.apply(TransportCommand::Pause),
            TransportEffect::StateChanged(TransportState::Stopped),
            "播放中 Pause ⇒ 必须报 StateChanged"
        );
        // 覆盖度：七条命令真的都被施加过（返回值口径不影响命令计数）。
        assert_eq!(transport.commands_applied(), 7);
    }

    /// 判据：`seek(t)` 必须**清相位余数** —— 定位之后的 tick 轨迹不得带着定位前的零头。
    ///
    /// 量什么：`advance_frames` 若干帧之后的 `position_ticks`（整数 tick）与
    /// `remainder`（`tick_den` 的分数，无量纲整数）。`tick_num = 120_000_000 × 960`、
    /// `tick_den = 1_000_000 × 60 × 48_000` ⇒ **1 帧 = 0.04 tick**。
    ///
    /// ## 为什么单独立一条
    ///
    /// 既有判据 `seek_sets_the_exact_origin` 确实断言了 `remainder() == 0`，但它在
    /// seek **之前**推进的是 5 000 帧 = **恰好 200 tick**（余数本来就是 0）
    /// ⇒ 那条断言对本条契约是**空转**的。本票（engine-28）注入实测：删掉
    /// `Transport::seek` 里的 `self.remainder = 0;`，全量 24 个目标全绿。
    ///
    /// 构造：先推进 128 帧（= 5.12 tick ⇒ 余数 0.12 tick），再 `SeekTicks(100)`，
    /// 然后推进 **24 帧 = 0.96 tick**：清零之后不足 1 tick（位置不动）；带着陈旧
    /// 0.12 tick 的零头会凑成 **1.08 tick** ⇒ 多跳 1 tick。最后再推进 1 帧
    /// （= 1.00 tick 恰好进位）作为"计数真的在走"的见证。
    #[test]
    fn seek_clears_the_phase_remainder() {
        let mut transport = Transport::free_running(48_000, 120.0);
        transport.advance_frames(128);
        assert_eq!(transport.position_ticks(), 5, "128 帧 = 5.12 tick");
        assert_eq!(
            transport.remainder(),
            345_600_000_000,
            "前提：推进 128 帧之后必须留下非零的相位余数（0.12 tick）"
        );

        transport.apply(TransportCommand::SeekTicks(100));
        assert_eq!(transport.position_ticks(), 100, "定位到 tick 100");
        assert_eq!(
            transport.remainder(),
            0,
            "定位必须清相位余数（陈旧零头会污染之后的 tick 计数）"
        );

        transport.advance_frames(24);
        assert_eq!(
            transport.position_ticks(),
            100,
            "24 帧 = 0.96 tick 不得进位（带陈旧 0.12 tick 的零头会凑成 1.08 ⇒ 多跳 1 tick）"
        );
        transport.advance_frames(1);
        assert_eq!(
            transport.position_ticks(),
            101,
            "再推 1 帧 = 1.00 tick 恰好进位一次（见证计数真的在走）"
        );
    }
}
