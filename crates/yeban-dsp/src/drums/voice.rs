//! 4 个鼓机音色的**合成配方**（信号发生层）[ARCH-RT-001, ARCH-DET-001]。
//!
//! 本文件只管"一个槽位怎么发声"：波形、包络、带通、噪声、clap 的 burst 定时。
//! 槽位分配、确定性窃取与 3 ms 淡出在 [`super`]（`drums/mod.rs`）。
//!
//! ## 1. 只组合既有器件，不新写滤波器
//!
//! 全部信号发生都来自 `yeban-dsp` 已有的公开面：
//!
//! | 用途 | 复用的既有器件 | `file:line` |
//! | :--- | :--- | :--- |
//! | 幅度/音高/噪声/尾巴包络 | [`crate::envelope::Adsr`]（含 `STEAL_RELEASE_SECONDS`） | `envelope.rs:52` / `:31` |
//! | 白噪声与确定性 RNG | [`crate::noise::Rng`]（xorshift32，显式播种） | `noise.rs:24` / `:34` |
//! | 带通（`LP(hi) ∘ (x − LP(lo))⁴`） | [`crate::filter::LadderFilter`] | `filter.rs:64` / `:93` |
//!
//! **带通不是新滤波器**：它是既有 `LadderFilter` 的组合（见 [`Band`] 与
//! [`HIGHPASS_STAGES`]）。`shaping::Biquad`（`shaping.rs:252`）是私有类型，
//! 本 crate 的公共面里没有带通；组合既有公开器件与
//! `channel_strip`（`channel_strip.rs:1`）的"组合既有器件"纪律同形。
//! ⚠ 第一版用的是 `LP(hi) − LP(lo)`，实测低频裙边太缓（452 Hz 只压到 −14 dB）
//! ⇒ 被 [`Band`] 的文档记录并换掉。
//!
//! ## 2. 逐样本路径上没有除法、没有超越函数（除正弦本身）
//!
//! - **相位是 `u32` 定点**：`2³²` 一周期。整数累加无漂移、不受 FTZ/DAZ 影响
//!   ⇒ 逐位确定，与块切分构造性无关（同 `polysynth` 的相位口径）；
//! - **音高下滑是增量域的线性插值**：`inc = inc_lo + (inc_hi − inc_lo)·p`
//!   （`p` 是 1 → 0 的音高包络）。`inc` 对频率是线性的，因此"插值增量"与
//!   "插值频率再算增量"在浮点意义下只差一次舍入 —— 而它**省掉了每帧的除法**；
//! - **正弦**用 [`sine_from_phase`]：`phase >> 8` 后转 `f32`。`2²⁴` 以内的整数在
//!   `f32` 里**精确**，因此这一步不引入舍入；`f32::sin` 是本 crate 已有的做法
//!   （`math::exp2` 调 `f32::exp2`、`filter.rs:102` 调 `tan`、`shaping.rs` 调
//!   `sin_cos`），跨平台一致性口径与它们相同：**不是本模块新引入的依赖**；
//! - **方波**直接看相位最高位，没有超越函数。
//!
//! ## 3. 数值安全（不许 NaN / Inf）
//!
//! 逐项论证：
//!
//! 1. 参数进不了本文件：全部经 [`super::DrumKitParams::sanitised`] 钳到有限合法域；
//! 2. `Adsr` 的值域：`envelope.rs:222-258` 的每一支都把 `value` 留在 `[0, 1]`
//!    （Attack 到 1 就切段、Decay 是 `sustain + (value − sustain)·coef`、Release
//!    在 `< 1e-4` 时归零）⇒ 只要 `sustain ∈ [0,1]` 且 `value` 从有限数出发，
//!    它恒有限。本模块一律 `sustain = 0.0`；
//! 3. `increment_for` 的输出恒在 `[1, 2³¹]`（`clamp(0, 0.5) × 2³²`，非有限走 `1.0`）
//!    ⇒ `as u32` 不会回绕，也不会得 0（0 增量会让相位卡死）；
//! 4. `sine_from_phase` 的输入是 `u32`、输出是 `[-1, 1]` 的 `sin`；
//! 5. 方波支路是 `±1` 的 6 项和乘 `1/6` ⇒ `[-1, 1]`；
//! 6. `Rng::next_bipolar` 是 `[-1, 1)`；
//! 7. [`Band::process`] 的每个阶段都是"有限数之差"或 `LadderFilter` 的输出；
//!    `filter.rs:166` 的 `bounded_saturate` 把每个滤波器输出收在 `[-1, 1]`
//!    ⇒ 四级高通的中间值在 `[-2, 2]` 内（级联的界是 `2^4`，但实测远小于它，
//!    且**恒有限**）；
//! 8. 槽位输出是有限值的乘加 ⇒ 有限；总和是有限值之和 ⇒ 有限。
//!
//! 判据：`mod.rs` 的单测 `hostile_parameters_never_produce_a_non_finite_sample`
//! 用"每个字段都是 `NaN`/`±∞`"的鼓组把 5 个音色各渲染 4 800 帧，
//! 断言非有限样本数为 **0**；`drums_render.rs` 的 R1 再断言 5 个音色各自
//! 一击的样本全部有限。

use crate::envelope::Adsr;
use crate::filter::LadderFilter;
use crate::math::sanitise_sample_rate;
use crate::noise::Rng;

use super::DrumKitParams;

/// 踩镲的方波支路条数（经典 808 踩镲的做法是"多条非谐方波"）。
pub(super) const HAT_PARTIALS: usize = 6;

/// 踩镲 6 条方波的**频率比值**。
///
/// 选法（**本模块声明**，不是从硬件反推的）：取**前 6 个素数的平方根**
/// `√2, √3, √5, √7, √11, √13`。
///
/// 这条选法给出的性质是**可判定的**，而且判据真的在测它
/// （`drums/mod.rs` 的 `the_hat_ratios_are_mutually_inharmonic`）：
///
/// 1. 6 个比值两两不同；
/// 2. **任意两个比值的商都不落在 `1..=8` 的整数上**（容差 `5e-3`）⇒ 没有哪条
///    方波的基频正好落在另一条的某个低次谐波上 ⇒ 6 条支路没有共同基频，
///    合成谱是密集的非谐谱，这正是"金属味"要的东西。
///
/// ⚠ 反例（本模块第一版踩到的）：`√1..√9` 里 `√9 / √1 = 3` 是整数 ⇒ 第 2 条
/// 性质不成立。第一版还写了一条"与 `p/q`（`p, q ≤ 8`）相差大于 0.02"的断言，
/// 而这条断言对任何 6 个实数都**不可能**普遍成立（`√2/√3 = 0.8165` 距 `3/4`
/// 只有 0.0665、`√2/√5 = 0.6325` 距 `2/3` 只有 0.0342）⇒ 那条断言被删掉，
/// 换成上面这条可满足、且**真的**在测"无共同基频"的性质。
///
/// 规范、ADR 与既有代码都**没有**给出这组比值（见 `drums/mod.rs` 的"规范原文"）。
/// 踩镲 6 条方波的**频率比值**（见下方常量 `HAT_RATIOS` 的定义）。
pub(super) const HAT_RATIOS: [f32; HAT_PARTIALS] = [
    // 只有 √2 在 `core::f32::consts` 里有常量（clippy 的 `approx_constant`
    // 会拒绝手写的那一个）⇒ 第一项用常量、其余用字面量，注释写明出处。
    core::f32::consts::SQRT_2, // √2
    1.732_050_8,               // √3
    2.236_068,                 // √5
    2.645_751_3,               // √7
    3.316_624_8,               // √11
    3.605_551_3,               // √13
];

/// 带通的谐振旋钮值。取 `0.0`：带通是"两个低通之差"，端点由截止频率定，
/// 不需要谐振；打开谐振会在端点造出两个尖峰，那不是"带通噪声"。
pub(super) const BANDPASS_RESONANCE: f32 = 0.0;

/// 带通的驱动旋钮值。取 `0.0`（`LadderFilter::configure` 的驱动底部是单位增益，
/// `filter.rs:111`）⇒ 通带透明，噪声不被额外塑形。
pub(super) const BANDPASS_DRIVE: f32 = 0.0;

/// `f32` 能精确表示一切整数上界的下一档：`2²⁴`。
const TWO_POW_24: f32 = 16_777_216.0;

/// `2³²`（一个周期对应的定标）。
const TWO_POW_32: f32 = 4_294_967_296.0;

/// 半个周期（方波的符号分界）。
const HALF_PHASE: u32 = 0x8000_0000;

/// `u32` 定点相位 → 正弦值。
///
/// 先右移 8 位再转 `f32`：`phase >> 8` 最大 `2²⁴ − 1`，而 `2²⁴` 以内的整数在
/// `f32` 里**精确** ⇒ 这一步没有舍入。相位分辨率因此是 `2⁻²⁴` 周期（约 −144 dB）。
#[inline]
#[must_use]
pub(super) fn sine_from_phase(phase: u32) -> f32 {
    let turns = (phase >> 8) as f32 * (1.0 / TWO_POW_24);
    (turns * core::f32::consts::TAU).sin()
}

/// 频率 → 相位增量（`2³²` 定标，`f32`）。
///
/// 输出恒在 `[1, 2³¹]`：
/// - `turns` 先钳到 `[0, 0.5]`（Nyquist）⇒ 乘积最多 `2³¹`；
/// - 非有限输入（含 `NaN`：`f32::clamp` 对 `NaN` 是恒等，`clamp.rs` 行为见
///   `compressor.rs:319` 的同类注释）走 `1.0`；
/// - 下界 `1.0` 而不是 `0.0`：0 增量会让相位永久卡死（静音但不是"无声地正确"）。
#[inline]
#[must_use]
pub(super) fn increment_for(freq_hz: f32, sample_rate: f32) -> f32 {
    let turns = (freq_hz / sample_rate).clamp(0.0, 0.5);
    let increment = turns * TWO_POW_32;
    if increment.is_finite() {
        increment.max(1.0)
    } else {
        1.0
    }
}

/// 高通的级数：`x − LP(lo)` 级联这么多次。
///
/// `x − LP(lo)` 是"四极低通的**补**"，它的阻带斜率**不是** 24 dB/oct：在低频处
/// `LP(lo) → 1` 而相位 `φ → 0`，差值 `|1 − e^{jφ}| ≈ |φ|`，而四极低通在
/// `f ≪ lo` 处的相位是 `≈ 4·arctan(f/lo)` ⇒ 它是**一阶（6 dB/oct）**高通。
/// 因此级联 4 次得到 24 dB/oct 的近似。
///
/// 本机实测（`LP(hi)( hp^N( x ) )` 的正弦稳态峰值，48 kHz，
/// `lo = 6000`、`hi = 14000`）：
///
/// | 频率 | `N = 1` | `N = 2` | `N = 4` |
/// | :--- | ---: | ---: | ---: |
/// | 452 Hz | 0.322 | 0.104 | **0.0107** |
/// | 1 kHz | 0.601 | 0.361 | 0.130 |
/// | 2 kHz | 0.984 | 0.968 | 0.937 |
/// | 4 kHz | 1.000 | 1.000 | 1.000 |
/// | 8 kHz | 0.900 | 0.810 | 0.656 |
/// | 12 kHz | 0.593 | 0.351 | 0.124 |
/// | 14 kHz | 0.499 | 0.249 | 0.062 |
const HIGHPASS_STAGES: usize = 4;

/// 带通：`LP(hi)` ∘ `hp` 级联 [`HIGHPASS_STAGES`] 次，`hp(v) = v − LP(lo)(v)`。
///
/// # 为什么不是 `LP(hi) − LP(lo)`（本模块第一版的做法，实测被否）
///
/// | 频率 | `LP(14000) − LP(6000)` | `LP(8000) − LP(700)` |
/// | :--- | ---: | ---: |
/// | 452 Hz | **0.193** | 1.284 |
/// | 1 kHz | 0.414 | 1.026 |
/// | 2 kHz | 0.739 | 0.882 |
/// | 4 kHz | 0.965 | 0.668 |
/// | 6 kHz | 0.921 | 0.432 |
/// | 10 kHz | 0.581 | 0.131 |
///
/// 452 Hz 处只有 −14 dB ⇒ 一个"6–14 kHz 带通"实际上把 452 Hz **放过去了**，
/// 而 6 条方波的能量恰好集中在它们的基频（452–1154 Hz，幅度 0.21，而 3 kHz
/// 以上的谐波只有 0.02–0.07）⇒ 踩镲听起来是一条 452 Hz 的嗡嗡声。
/// 本机实测：那一版的谱重心只有 **1585.7 Hz**（`base_hz = 320`、
/// `highpass_hz = 6000`）。
///
/// ⇒ 现在用级联高通（见 [`HIGHPASS_STAGES`] 的表：452 Hz 从 0.322 降到 0.0107，
/// −39 dB）。五只 [`LadderFilter`] 全部是**既有公开器件**（`filter.rs:64`），
/// 没有新写滤波器 —— 与 `channel_strip`（`channel_strip.rs:1`）的"组合既有器件"
/// 纪律同形。
///
/// ⚠ `highpass_hz` 是**高通级的低通拐点**，不是带通下沿：由 `N = 4` 的表可见
/// `lo = 6000` 时的有效通带约 **2–9 kHz**。参数文档与判据都按这个口径写。
#[derive(Clone, Copy, Debug)]
pub(super) struct Band {
    /// 各级高通用的低通（截止 `lo`，**各自独立的状态**）。
    low: [LadderFilter; HIGHPASS_STAGES],
    /// 上沿低通（截止 `hi`）。
    high: LadderFilter,
}

impl Band {
    /// 空带通：系数取 [`LadderFilter::new`] 的默认值，状态归零。
    pub(super) const IDLE: Self = Self {
        low: [LadderFilter::new(); HIGHPASS_STAGES],
        high: LadderFilter::new(),
    };

    /// 只换系数、**保留状态**（`set_params` 作用在在响的一击上时用）。
    ///
    /// 触发时的状态归零由 [`Generator::new`] 从 [`Band::IDLE`] 构造保证。
    pub(super) fn reconfigure(&mut self, sample_rate: f32, lo_hz: f32, hi_hz: f32) {
        for filter in &mut self.low {
            filter.configure(sample_rate, lo_hz, BANDPASS_RESONANCE, BANDPASS_DRIVE);
        }
        self.high
            .configure(sample_rate, hi_hz, BANDPASS_RESONANCE, BANDPASS_DRIVE);
    }

    /// 过一个样本。
    #[inline]
    pub(super) fn process(&mut self, input: f32) -> f32 {
        let mut value = input;
        for filter in &mut self.low {
            value -= filter.process(value);
        }
        self.high.process(value)
    }
}

/// 一个槽位的**信号发生器状态**。
///
/// 容量对 4 个音色取并集：kick 用 `pitch_env` + `body_env` + `phases[0]`，
/// snare 用 `body_env`（音调）+ `tail_env`（噪声）+ `phases[0..2]` + `rng`，
/// 踩镲用 `body_env` + `phases[0..6]`，拍手用 `burst_env` + `tail_env` + `rng`。
/// 未用到的字段保持 [`Adsr`] 的 Idle（值 0）⇒ 它们不发声，也不产生 `NaN`。
///
/// 取并集（而不是 `enum` 套 `struct`）的理由：整个结构是 `Copy` 值类型，
/// 槽位池因此也是定长数组，`render` / `trigger` 零分配
/// （与 `polysynth::PolyVoice` 的同一取舍）。
#[derive(Clone, Copy, Debug)]
pub(super) struct Generator {
    /// [`super::DrumVoice`] 的 `u8` 判别值。`u8` 而不是枚举，
    /// 是为了让 `Generator` 保持 `Copy` 且 `const` 可构造。
    voice: u8,
    /// 相位累加器（`2³²` 一周期）。
    phases: [u32; HAT_PARTIALS],
    /// 静态相位增量（snare 的 2 个音调振荡器 / 踩镲的 6 条方波）。
    incs: [f32; HAT_PARTIALS],
    /// kick 的**尾巴**（音高包络走完后）相位增量。
    inc_lo: f32,
    /// kick 的**起始**相位增量（音高包络 = 1 时）。
    inc_hi: f32,
    /// snare 的音调分量电平。
    tone_level: f32,
    /// snare 的噪声分量电平。
    noise_level: f32,
    /// **音色电平**（`KickParams::level` 等，`0..=1`）。
    ///
    /// 放在发生器里而不是折进 `Slot::gain`：`set_params` 改音色电平时，
    /// 在响的一击必须跟着变（折进 `gain` 就只对**新**触发生效）。
    voice_level: f32,
    /// 白噪声源（每槽一份，触发时按确定性种子播种）。
    rng: Rng,
    /// 带通（两级高通 + 一级低通，见 [`Band`]）。
    band: Band,
    /// kick：音高下滑包络。
    pitch_env: Adsr,
    /// kick 幅度 / snare 音调分量 / 踩镲整体幅度。
    body_env: Adsr,
    /// snare 噪声分量 / clap 长尾巴。
    tail_env: Adsr,
    /// clap 的短促串包络（每个 onset 重触发一次）。
    burst_env: Adsr,
    /// 距触发的帧数（clap 的 burst 定时用整数帧，确定性）。
    frame: u64,
    /// clap 已经发出的 onset 个数（从 0 起）。
    burst_index: u32,
    /// clap 的 onset 总数（`>= 1`）。
    bursts: u32,
    /// clap 的 onset 间隔（帧，`>= 1`）。
    spacing_frames: u64,
    /// 是否已进入窃取/闭镲淡出。淡出期间**不再**重触发 clap 的 onset ——
    /// 否则包络会在淡出中途回升，那就是一次咔哒。
    choking: bool,
}

impl Generator {
    /// 空发生器：全部包络 Idle、相位与滤波器状态归零。
    pub(super) const IDLE: Self = Self {
        voice: 0,
        phases: [0; HAT_PARTIALS],
        incs: [0.0; HAT_PARTIALS],
        inc_lo: 1.0,
        inc_hi: 1.0,
        tone_level: 0.0,
        noise_level: 0.0,
        voice_level: 0.0,
        rng: Rng::new(1),
        band: Band::IDLE,
        pitch_env: Adsr::new(),
        body_env: Adsr::new(),
        tail_env: Adsr::new(),
        burst_env: Adsr::new(),
        frame: 0,
        burst_index: 0,
        bursts: 1,
        spacing_frames: 1,
        choking: false,
    };

    /// 触发一个音色：把 `kit` 解析成系数，并把"本帧就发声"的包络 gate_on。
    ///
    /// `seed` 是白噪声的确定性种子（由 [`super::DrumMachine::trigger`] 从
    /// 音色与触发序号推出）⇒ 同一次触发序列给同一段噪声 [ARCH-DET-001]。
    pub(super) fn new(voice: u8, kit: &DrumKitParams, sample_rate: f32, seed: u32) -> Self {
        let mut generator = Self {
            voice,
            rng: Rng::new(seed),
            ..Self::IDLE
        };
        generator.reconfigure(kit, sample_rate);
        match voice {
            super::VOICE_KICK => {
                generator.pitch_env.gate_on();
                generator.body_env.gate_on();
            }
            super::VOICE_SNARE => {
                generator.body_env.gate_on();
                generator.tail_env.gate_on();
            }
            super::VOICE_CLOSED_HAT | super::VOICE_OPEN_HAT => {
                generator.body_env.gate_on();
            }
            _ => {
                // clap：第 0 个 onset 立刻发出。
                generator.gate_clap_onset();
            }
        }
        generator
    }

    /// 音色的 `u8` 判别值。
    #[must_use]
    pub(super) const fn voice(&self) -> u8 {
        self.voice
    }

    /// 当前**输出幅度包络**（用作窃取优先级里的"振幅能量"）。
    ///
    /// 单位与包络同量纲（`0..=1`，无量纲）。kick/踩镲是 `body_env`；
    /// snare 与 clap 是两段里较大的那一段（它们的两个分量各自独立衰减）。
    #[must_use]
    pub(super) fn level(&self) -> f32 {
        match self.voice {
            super::VOICE_KICK | super::VOICE_CLOSED_HAT | super::VOICE_OPEN_HAT => {
                self.body_env.value()
            }
            super::VOICE_SNARE => self.body_env.value().max(self.tail_env.value()),
            _ => self.burst_env.value().max(self.tail_env.value()),
        }
    }

    /// 本槽位是否已经发声完毕（可以回收）。
    ///
    /// clap 的 onset 序列没有走完时**永不**返回 `true`：onset 之间的包络可能
    /// 落到 Idle，但下一拍还要重触发（否则槽位会被中途回收、拍手少一拍）。
    #[must_use]
    pub(super) fn is_finished(&self) -> bool {
        match self.voice {
            super::VOICE_KICK | super::VOICE_CLOSED_HAT | super::VOICE_OPEN_HAT => {
                !self.body_env.is_active()
            }
            super::VOICE_SNARE => !self.body_env.is_active() && !self.tail_env.is_active(),
            _ => {
                self.burst_index + 1 >= self.bursts
                    && !self.burst_env.is_active()
                    && !self.tail_env.is_active()
            }
        }
    }

    /// 进入淡出（窃取/闭镲 choke）：每个包络都用 [ARCH-RT-004] 的 3 ms
    /// 指数释放，并**停止** clap 的 onset 重触发。
    pub(super) fn begin_fade(&mut self) {
        self.choking = true;
        self.pitch_env.start_steal_fade();
        self.body_env.start_steal_fade();
        self.tail_env.start_steal_fade();
        self.burst_env.start_steal_fade();
    }

    /// 按新的 `kit` 重算"参数 → 系数"的映射，**保留全部状态值**
    /// （相位、RNG、包络电平、滤波器状态、`frame` 都不动）。
    ///
    /// 调用点保证：只对 `fade_remaining == 0` 的槽位调用
    /// （见 [`super::DrumMachine::set_params`]）—— 否则会把淡出的 3 ms release
    /// 覆盖回正常释放，等于取消淡出。
    pub(super) fn reconfigure(&mut self, kit: &DrumKitParams, sample_rate: f32) {
        let sample_rate = sanitise_sample_rate(sample_rate);
        match self.voice {
            super::VOICE_KICK => {
                let params = &kit.kick;
                self.voice_level = params.level;
                self.inc_lo = increment_for(params.tune_hz, sample_rate);
                self.inc_hi = increment_for(params.glide_hz, sample_rate);
                self.pitch_env
                    .set_params(0.0, params.glide_s, 0.0, params.glide_s);
                self.body_env
                    .set_params(params.attack_s, params.decay_s, 0.0, params.decay_s);
            }
            super::VOICE_SNARE => {
                let params = &kit.snare;
                self.voice_level = params.level;
                self.incs = [0.0; HAT_PARTIALS];
                self.incs[0] = increment_for(params.tone_hz, sample_rate);
                self.incs[1] = increment_for(params.tone_hz * params.tone_ratio, sample_rate);
                self.tone_level = params.tone_level;
                self.noise_level = params.noise_level;
                self.band.reconfigure(
                    sample_rate,
                    params.noise_highpass_hz,
                    params.noise_lowpass_hz,
                );
                self.body_env.set_params(
                    params.attack_s,
                    params.tone_decay_s,
                    0.0,
                    params.tone_decay_s,
                );
                self.tail_env.set_params(
                    params.attack_s,
                    params.noise_decay_s,
                    0.0,
                    params.noise_decay_s,
                );
            }
            super::VOICE_CLOSED_HAT | super::VOICE_OPEN_HAT => {
                let params = &kit.hihat;
                self.voice_level = params.level;
                let decay_s = if self.voice == super::VOICE_CLOSED_HAT {
                    params.closed_decay_s
                } else {
                    params.open_decay_s
                };
                for (index, increment) in self.incs.iter_mut().enumerate() {
                    *increment = increment_for(params.base_hz * HAT_RATIOS[index], sample_rate);
                }
                self.band
                    .reconfigure(sample_rate, params.highpass_hz, params.lowpass_hz);
                self.body_env
                    .set_params(params.attack_s, decay_s, 0.0, decay_s);
            }
            _ => {
                let params = &kit.clap;
                self.voice_level = params.level;
                self.band
                    .reconfigure(sample_rate, params.highpass_hz, params.lowpass_hz);
                self.bursts = params.bursts;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let spacing = (params.burst_spacing_s * sample_rate) as u64;
                self.spacing_frames = spacing.max(1);
                self.burst_env.set_params(
                    params.attack_s,
                    params.burst_decay_s,
                    0.0,
                    params.burst_decay_s,
                );
                self.tail_env
                    .set_params(params.attack_s, params.decay_s, 0.0, params.decay_s);
            }
        }
    }

    /// gate_on 当前 `burst_index` 对应的那一个 onset。
    ///
    /// 第 `bursts − 1` 个 onset（最后一个）走**长尾巴** `tail_env`，
    /// 其余走**短促串** `burst_env` ⇒ 听感是"3 次短促 + 1 次带尾巴"。
    fn gate_clap_onset(&mut self) {
        if self.burst_index + 1 >= self.bursts {
            self.burst_env.reset();
            self.tail_env.reset();
            self.tail_env.gate_on();
        } else {
            self.burst_env.reset();
            self.burst_env.gate_on();
        }
    }

    /// 推进一个样本并返回它。**实时路径**：零分配、零锁、零 I/O。
    #[inline]
    pub(super) fn next(&mut self) -> f32 {
        let value = match self.voice {
            super::VOICE_KICK => self.kick(),
            super::VOICE_SNARE => self.snare(),
            super::VOICE_CLOSED_HAT | super::VOICE_OPEN_HAT => self.hihat(),
            super::VOICE_CLAP => self.clap(),
            _ => 0.0,
        };
        self.frame = self.frame.saturating_add(1);
        value * self.voice_level
    }

    /// 底鼓：一条**下滑正弦**。
    ///
    /// `f(t) = tune_hz + (glide_hz − tune_hz)·p(t)`，`p` 是一极点包络
    /// （`Adsr`，`sustain = 0`）⇒ `p(t) = e^{−6t/glide_s}`。
    /// 幅度是 `body_env`（同一个一极点口径）。相位取正弦**再**推进
    /// ⇒ 第 0 帧的相位正好是 0，正弦从 0 出发（不是从某个随机相位起跳）。
    #[inline]
    fn kick(&mut self) -> f32 {
        let glide = self.pitch_env.process(true);
        let increment = self.inc_lo + (self.inc_hi - self.inc_lo) * glide;
        let value = sine_from_phase(self.phases[0]);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let step = increment as u32;
        self.phases[0] = self.phases[0].wrapping_add(step);
        value * self.body_env.process(true)
    }

    /// 军鼓：**两个固定频率正弦 + 带通白噪声**。
    ///
    /// 两个正弦的比值是 `tone_ratio`（默认 1.784 ≈ 330/185，即经典军鼓的两个
    /// 桥接 T 频率）。音调分量走 `body_env`、噪声分量走 `tail_env`：两条包络
    /// 各自衰减，因此可以分别对账（见 `tests/drums_render.rs` 的 S2/S3）。
    ///
    /// ⚠ 两个正弦**没有**互相耦合，也**没有**桥接 T 的 Q —— 见
    /// `drums/mod.rs` 的未实现清单第 7 条。
    #[inline]
    fn snare(&mut self) -> f32 {
        let first = sine_from_phase(self.phases[0]);
        let second = sine_from_phase(self.phases[1]);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let step0 = self.incs[0] as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let step1 = self.incs[1] as u32;
        self.phases[0] = self.phases[0].wrapping_add(step0);
        self.phases[1] = self.phases[1].wrapping_add(step1);
        let tone = (first + second) * 0.5 * self.tone_level * self.body_env.process(true);
        let white = self.rng.next_bipolar();
        let noise = self.band.process(white) * self.noise_level * self.tail_env.process(true);
        tone + noise
    }

    /// 踩镲：**6 条非谐方波 → 带通 → 幅度包络**。
    ///
    /// 方波的谐波含量是"奇次全给"，6 条非谐比值的奇次谐波叠在一起就是密集的
    /// 金属谱；带通（默认 6–14 kHz）把低频的"音高感"切掉，只留金属噪声。
    ///
    /// ⚠ 真实 808 踩镲的金属振荡器是**移位寄存器 + 6 位计数器**，本模块用的是
    /// 6 条**连续**方波（未实现清单第 9/10 条）。
    #[inline]
    fn hihat(&mut self) -> f32 {
        let mut sum = 0.0f32;
        for index in 0..HAT_PARTIALS {
            sum += if self.phases[index] < HALF_PHASE {
                1.0
            } else {
                -1.0
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let step = self.incs[index] as u32;
            self.phases[index] = self.phases[index].wrapping_add(step);
        }
        let mixed = sum * (1.0 / HAT_PARTIALS as f32);
        self.band.process(mixed) * self.body_env.process(true)
    }

    /// 拍手：**带通白噪声 × 多个短促 onset 的包络**。
    ///
    /// onset 定时用**整数帧**（`frame` 与 `spacing_frames`），因此与块切分无关。
    /// 最后一个 onset 走长尾巴 `tail_env`，其余的每个都 `reset` 后重触发
    /// `burst_env` ⇒ 包络在 onset 处**从 0 起跳**，onset 之间没有台阶。
    #[inline]
    fn clap(&mut self) -> f32 {
        if !self.choking {
            while self.burst_index + 1 < self.bursts {
                let next_at = u64::from(self.burst_index + 1).saturating_mul(self.spacing_frames);
                if self.frame < next_at {
                    break;
                }
                self.burst_index += 1;
                self.gate_clap_onset();
            }
        }
        let white = self.rng.next_bipolar();
        let noise = self.band.process(white);
        let envelope = self
            .burst_env
            .process(true)
            .max(self.tail_env.process(true));
        noise * envelope
    }
}
