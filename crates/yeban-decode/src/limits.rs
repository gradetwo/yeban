//! 解码预算与长度算术 —— **零第三方依赖**的纯逻辑层。
//!
//! 为什么单独一层：这些判据决定"畸形输入会不会把进程吃掉"，它们必须在**不编译
//! symphonia / rubato** 的前提下就能被逐条跑红跑绿。本模块只用 `core`/`alloc`/`std`，
//! 因此可以用
//! `rustc --edition 2024 --test` 把本文件单独编成测试二进制在本机执行
//! （见 `docs/ledger/decode-core-notes.md` §7 的验证范围声明）。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.3 [ARCH-SEC-003]：单条目的
//!   "≤ 2 GB"口径被本模块沿用为**输入字节上限**；
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.1 [ARCH-DET-001]：重采样必须
//!   "同输入 → 同输出"，所以长度契约只用**精确有理数整数运算**表达，不引入任何浮点
//!   或超越函数（`libm` 都不需要）；
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §10.2 [ARCH-DSP-002]：
//!   44.1k/48k/96k 互转的输入输出长度关系。
//!
//! 边界: 本模块**不做**任何 I/O、不持有缓冲、不知道 symphonia 的存在。它只回答
//! "这个尺寸/这个长度是否在预算内"。

use std::error::Error;
use std::fmt;

/// 单个输入资产的字节上限：2 GiB。
///
/// 口径直接沿用 [ARCH-SEC-003] 对归档单条目的 2 GB 上限 —— 同一个数字只在一个
/// 地方被裁决，避免"归档允许 2 GB、解码器允许 20 GB"这种自相矛盾。
pub const MAX_INPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 解码后**交织 f32 PCM** 的字节上限：2 GiB。
///
/// 换算成时长（`帧数 = 字节 / (声道 × 4)`）：
/// - 48 kHz 立体声 ⇒ 约 93 分钟；
/// - 96 kHz 立体声 ⇒ 约 46 分钟；
/// - 96 kHz 8 声道 ⇒ 约 11.6 分钟。
///
/// 超出即 [`LimitViolation::TooManyFrames`]，**绝不**"先分配再说"。
pub const MAX_PCM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 声道数上限（畸形文件常用"声明 65535 声道"制造乘法溢出/OOM）。
pub const MAX_CHANNELS: u16 = 64;

/// 采样率上限：768 kHz（DXD 级别之上再留一倍余量）。
pub const MAX_SAMPLE_RATE: u32 = 768_000;

/// 交织 `f32` 样本的总数上限（由 [`MAX_PCM_BYTES`] 换算）。
pub const MAX_INTERLEAVED_SAMPLES: u64 = MAX_PCM_BYTES / 4;

/// 重采样所用的 sinc 滤波器长度（抽头数）。
///
/// 与 `rubato` 构造参数里的字面量 `256` 是同**一个**被钉死的值：字面量写在调用点
/// 是为了避免与上游参数位宽（`usize` vs `u32`）耦合，这里的常量负责让判据能引用它、
/// 并在判据里与上游参数对上（见 `resample.rs` 的配置判据）。
pub const SINC_LEN: u64 = 256;

/// 长度契约的相对容差：0.1%（单位 ppm 的千分之一，见 [`LEN_TOLERANCE_PPM`]）。
pub const LEN_TOLERANCE_PPM: u64 = 1_000;

/// 长度契约的绝对容差下限（帧）。防止"短片段 + 相对容差"退化成零容差。
pub const MIN_LEN_TOLERANCE_FRAMES: u64 = 8;

/// "短片段"阈值（帧）：理想输出帧数低于此值时，重采样器被裁掉的前置延迟
/// 相对整个片段不再可忽略，契约额外放宽 [`SINC_LEN`] 帧。
///
/// 这不是"放水"，而是把 `rubato` 的真实语义写进来：延迟被裁掉之后剩下的不是
/// 一个整数帧的舍入误差，而是滤波器长度量级的边界效应。
pub const SHORT_CLIP_THRESHOLD_FRAMES: u64 = 4_096;

/// 声明的帧数与解出的帧数的默认容差：**0 帧**。
///
/// 本 crate 只启用无损编解码（PCM / FLAC）与 Ogg Vorbis（按包解，无整段延迟字段）；
/// 对 PCM/FLAC 而言容器声明的帧数必须与解出的帧数**逐帧相等**，不给容差。
pub const DURATION_TOLERANCE_FRAMES: u64 = 0;

/// 解码循环里允许的"连续不推进"包数上限。
///
/// 存在的理由是一个**实测到的上游行为**，不是假想的防御：RIFF 解封装用**声明的**
/// `data` 块长度算 `data_end_pos`（源码 `symphonia-format-riff-0.6.1/src/common.rs:475`
/// 与 `wave/mod.rs:145`），`next_packet` 又只按 `data_end_pos - pos` 判断"还有多少块"
/// （`common.rs:404-408`）。于是一个"`data` 头完整、但数据体被截断"的 WAV 会让
/// `pos` 停住不动、每轮都返回**空包**，而 `read_boxed_slice` 对 EOF 是**截短返回**
/// 而不是报错（`symphonia-core-0.6.1/src/io/mod.rs:553-584`）。
/// 结果是"解析器不报错、也不推进"——没有这道闸门，解码线程会**永远转下去**。
///
/// 1024 这个数字对合法输入是极宽松的：WAV / FLAC 的音频包之间最多夹几个非音频包，
/// 不可能连续 1024 个包一帧音频都不出。而任何"不推进"的病态输入都会在 1024 轮内被拒。
pub const MAX_IDLE_PACKETS: u32 = 1_024;

/// "解码不推进"计数器。
///
/// 纯逻辑、零依赖，因此**闸门本身的行为可以在本机单独跑**（见 `#[cfg(test)]`）。
/// CI 上另有一条集成路径证明它真的被接进了解码循环。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IdleGuard {
    idle: u32,
}

impl IdleGuard {
    /// 全新的计数器（0 次不推进）。
    #[must_use]
    pub const fn new() -> Self {
        Self { idle: 0 }
    }

    /// 记一次"这一轮没有产出样本"，返回 `true` 表示**已经超过** [`MAX_IDLE_PACKETS`]。
    #[must_use]
    pub fn bump(&mut self) -> bool {
        self.idle = self.idle.saturating_add(1);
        self.idle > MAX_IDLE_PACKETS
    }

    /// 归零（这一轮真的产出了样本）。
    pub fn reset(&mut self) {
        self.idle = 0;
    }

    /// 当前连续不推进的包数。
    #[must_use]
    pub fn idle(self) -> u32 {
        self.idle
    }
}

/// 预算/尺寸违规。全部是"输入不可信"导致的，一律返回而非 panic。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitViolation {
    /// 输入字节数超过上限。
    InputTooLarge {
        /// 实际（或声明）的字节数。
        bytes: u64,
        /// 生效的上限。
        limit: u64,
    },
    /// 声道数为 0。
    ZeroChannels,
    /// 声道数超过上限。
    TooManyChannels {
        /// 声明的声道数。
        channels: u16,
        /// 生效的上限。
        limit: u16,
    },
    /// 采样率为 0。
    ZeroSampleRate,
    /// 采样率超过上限。
    SampleRateTooHigh {
        /// 声明的采样率。
        rate: u32,
        /// 生效的上限。
        limit: u32,
    },
    /// 交织样本总数超过预算。
    TooManyFrames {
        /// 帧数。
        frames: u64,
        /// 声道数。
        channels: u16,
        /// `frames × channels` 的交织样本数。
        samples: u64,
        /// 生效的样本数上限。
        limit: u64,
    },
    /// `frames × channels` 在 `u64` 里溢出。
    LayoutOverflow {
        /// 帧数。
        frames: u64,
        /// 声道数。
        channels: u16,
    },
    /// 向分配器申请缓冲被拒绝（`Vec::try_reserve` 失败）。
    AllocationRefused {
        /// 本次想要追加的样本数。
        samples: u64,
    },
}

impl fmt::Display for LimitViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooLarge { bytes, limit } => {
                write!(f, "input is {bytes} bytes, over the {limit}-byte cap")
            }
            Self::ZeroChannels => f.write_str("stream declares zero channels"),
            Self::TooManyChannels { channels, limit } => {
                write!(
                    f,
                    "stream declares {channels} channels, over the {limit} cap"
                )
            }
            Self::ZeroSampleRate => f.write_str("stream declares a zero sample rate"),
            Self::SampleRateTooHigh { rate, limit } => {
                write!(f, "stream declares {rate} Hz, over the {limit} Hz cap")
            }
            Self::TooManyFrames {
                frames,
                channels,
                samples,
                limit,
            } => write!(
                f,
                "{frames} frames x {channels} channels = {samples} interleaved samples, \
                 over the {limit}-sample budget"
            ),
            Self::LayoutOverflow { frames, channels } => write!(
                f,
                "frame/channel product overflows: {frames} frames x {channels} channels"
            ),
            Self::AllocationRefused { samples } => {
                write!(f, "allocator refused a buffer for {samples} more samples")
            }
        }
    }
}

impl Error for LimitViolation {}

/// 重采样长度契约：解出的输出帧数必须落在 `[min, max]` 闭区间内。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LenContract {
    /// 允许的最小输出帧数。
    pub min: u64,
    /// 允许的最大输出帧数。
    pub max: u64,
    /// `⌊输入帧数 × 输出率 / 输入率⌋`。
    pub ideal_floor: u64,
    /// `⌈输入帧数 × 输出率 / 输入率⌉`。
    pub ideal_ceil: u64,
}

/// 长度契约违约。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LenContractViolation {
    /// 采样率为 0，比例无定义。
    UndefinedRatio {
        /// 输入采样率。
        in_rate: u32,
        /// 输出采样率。
        out_rate: u32,
    },
    /// 实际输出帧数落在契约区间之外。
    OutsideBounds {
        /// 实际输出帧数。
        produced: u64,
        /// 生效的契约。
        contract: LenContract,
    },
}

impl fmt::Display for LenContractViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndefinedRatio { in_rate, out_rate } => {
                write!(f, "resample ratio undefined: {in_rate} Hz -> {out_rate} Hz")
            }
            Self::OutsideBounds { produced, contract } => write!(
                f,
                "resampler produced {produced} frames, outside the contract {}..={} \
                 (ideal {}..={})",
                contract.min, contract.max, contract.ideal_floor, contract.ideal_ceil
            ),
        }
    }
}

impl Error for LenContractViolation {}

/// 校验输入字节数是否在预算内。
///
/// # Errors
///
/// 超过 `limit` 时返回 [`LimitViolation::InputTooLarge`]。
pub fn check_input_len(bytes: u64, limit: u64) -> Result<(), LimitViolation> {
    if bytes > limit {
        return Err(LimitViolation::InputTooLarge { bytes, limit });
    }
    Ok(())
}

/// 校验 `frames × channels`，溢出或超预算都返回错误。
///
/// # Errors
///
/// 见 [`LimitViolation`] 的变体说明。
pub fn interleaved_samples(frames: u64, channels: u16) -> Result<u64, LimitViolation> {
    frames
        .checked_mul(u64::from(channels))
        .ok_or(LimitViolation::LayoutOverflow { frames, channels })
}

/// 校验一次解码的布局是否在预算内。
///
/// # Errors
///
/// 声道数/采样率不合法，或 `frames × channels` 超过 `pcm_bytes_limit / 4` 时返回错误。
pub fn check_layout(
    channels: u16,
    sample_rate: u32,
    frames: u64,
    pcm_bytes_limit: u64,
) -> Result<(), LimitViolation> {
    if channels == 0 {
        return Err(LimitViolation::ZeroChannels);
    }
    if channels > MAX_CHANNELS {
        return Err(LimitViolation::TooManyChannels {
            channels,
            limit: MAX_CHANNELS,
        });
    }
    if sample_rate == 0 {
        return Err(LimitViolation::ZeroSampleRate);
    }
    if sample_rate > MAX_SAMPLE_RATE {
        return Err(LimitViolation::SampleRateTooHigh {
            rate: sample_rate,
            limit: MAX_SAMPLE_RATE,
        });
    }
    let samples = interleaved_samples(frames, channels)?;
    let limit = pcm_bytes_limit / 4;
    if samples > limit {
        return Err(LimitViolation::TooManyFrames {
            frames,
            channels,
            samples,
            limit,
        });
    }
    Ok(())
}

/// 计算重采样的长度契约（精确有理数运算，零浮点）。
///
/// 语义：输出帧数 ≈ `输入帧数 × out_rate / in_rate`，容差为
/// `max(理想值 × 0.1%, 8 帧)`；当理想输出帧数小于 [`SHORT_CLIP_THRESHOLD_FRAMES`]
/// 时再额外放宽 [`SINC_LEN`] 帧（被裁掉的前置延迟相对短片段不可忽略）。
///
/// 返回 `None` 表示任一采样率为 0（比例无定义）。
#[must_use]
pub fn resample_len_contract(
    input_frames: u64,
    out_rate: u32,
    in_rate: u32,
) -> Option<LenContract> {
    if in_rate == 0 || out_rate == 0 {
        return None;
    }
    let num = u128::from(out_rate);
    let den = u128::from(in_rate);
    let ideal = u128::from(input_frames) * num;
    let floor = ideal / den;
    let ceil = ideal.div_ceil(den);
    let relative = floor * u128::from(LEN_TOLERANCE_PPM) / 1_000_000;
    let mut slack = relative.max(u128::from(MIN_LEN_TOLERANCE_FRAMES));
    if floor < u128::from(SHORT_CLIP_THRESHOLD_FRAMES) {
        slack += u128::from(SINC_LEN);
    }
    Some(LenContract {
        min: u64::try_from(floor.saturating_sub(slack)).ok()?,
        max: u64::try_from(ceil + slack).ok()?,
        ideal_floor: u64::try_from(floor).ok()?,
        ideal_ceil: u64::try_from(ceil).ok()?,
    })
}

/// 校验重采样实际输出的帧数是否满足长度契约。
///
/// # Errors
///
/// 采样率为 0，或 `produced` 落在 [`resample_len_contract`] 给出的区间之外时返回错误。
pub fn check_resampled_len(
    input_frames: u64,
    out_rate: u32,
    in_rate: u32,
    produced: u64,
) -> Result<LenContract, LenContractViolation> {
    let contract = resample_len_contract(input_frames, out_rate, in_rate)
        .ok_or(LenContractViolation::UndefinedRatio { in_rate, out_rate })?;
    if produced < contract.min || produced > contract.max {
        return Err(LenContractViolation::OutsideBounds { produced, contract });
    }
    Ok(contract)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn size_caps_are_pinned_to_the_documented_numbers() {
        // 判据 (尺寸上限口径): 改这个数字必须同时改 notes，否则这里先红。
        assert_eq!(MAX_INPUT_BYTES, 2 * GIB);
        assert_eq!(MAX_PCM_BYTES, 2 * GIB);
        assert_eq!(MAX_INTERLEAVED_SAMPLES, 2 * GIB / 4);
        assert_eq!(MAX_CHANNELS, 64);
        assert_eq!(MAX_SAMPLE_RATE, 768_000);
        assert_eq!(SINC_LEN, 256);
        // 判据 (MUST-GATE-011 防挂死): 不推进包数的闸门必须存在且非零。
        assert_eq!(MAX_IDLE_PACKETS, 1_024);
    }

    #[test]
    fn input_byte_budget_is_enforced() {
        assert_eq!(check_input_len(0, MAX_INPUT_BYTES), Ok(()));
        assert_eq!(check_input_len(MAX_INPUT_BYTES, MAX_INPUT_BYTES), Ok(()));
        assert_eq!(
            check_input_len(MAX_INPUT_BYTES + 1, MAX_INPUT_BYTES),
            Err(LimitViolation::InputTooLarge {
                bytes: MAX_INPUT_BYTES + 1,
                limit: MAX_INPUT_BYTES,
            })
        );
    }

    #[test]
    fn layout_budget_rejects_degenerate_declarations() {
        assert_eq!(
            check_layout(0, 48_000, 1, MAX_PCM_BYTES),
            Err(LimitViolation::ZeroChannels)
        );
        assert_eq!(
            check_layout(65, 48_000, 1, MAX_PCM_BYTES),
            Err(LimitViolation::TooManyChannels {
                channels: 65,
                limit: 64
            })
        );
        assert_eq!(
            check_layout(2, 0, 1, MAX_PCM_BYTES),
            Err(LimitViolation::ZeroSampleRate)
        );
        assert_eq!(
            check_layout(2, 768_001, 1, MAX_PCM_BYTES),
            Err(LimitViolation::SampleRateTooHigh {
                rate: 768_001,
                limit: 768_000
            })
        );
    }

    #[test]
    fn layout_budget_rejects_an_asset_over_the_pcm_cap() {
        // 正常的一秒立体声 48k 通过。
        assert_eq!(check_layout(2, 48_000, 48_000, MAX_PCM_BYTES), Ok(()));
        // 恰好用满预算通过；再多一帧就被拒绝。
        let frames = MAX_INTERLEAVED_SAMPLES / 2;
        assert_eq!(check_layout(2, 48_000, frames, MAX_PCM_BYTES), Ok(()));
        match check_layout(2, 48_000, frames + 1, MAX_PCM_BYTES) {
            Err(LimitViolation::TooManyFrames {
                frames: f,
                channels,
                limit,
                ..
            }) => {
                assert_eq!(f, frames + 1);
                assert_eq!(channels, 2);
                assert_eq!(limit, MAX_INTERLEAVED_SAMPLES);
            }
            other => panic!("expected TooManyFrames, got {other:?}"),
        }
    }

    #[test]
    fn layout_product_overflow_is_detected_not_wrapped() {
        assert_eq!(
            interleaved_samples(u64::MAX, 2),
            Err(LimitViolation::LayoutOverflow {
                frames: u64::MAX,
                channels: 2
            })
        );
        assert_eq!(interleaved_samples(4, 2), Ok(8));
    }

    #[test]
    fn zero_rate_has_no_length_contract() {
        assert!(resample_len_contract(1_000, 0, 48_000).is_none());
        assert!(resample_len_contract(1_000, 48_000, 0).is_none());
        assert_eq!(
            check_resampled_len(1_000, 0, 48_000, 0),
            Err(LenContractViolation::UndefinedRatio {
                in_rate: 48_000,
                out_rate: 0
            })
        );
    }

    #[test]
    fn length_contract_always_contains_the_ideal_output() {
        // 判据 (ARCH-DSP-002 长度关系): 三个规范采样率两两互转，理想值必在区间内。
        let rates = [44_100u32, 48_000, 96_000];
        for &in_rate in &rates {
            for &out_rate in &rates {
                for &frames in &[0u64, 1, 7, 1_000, 44_100, 48_000, 96_000, 480_000] {
                    let c =
                        resample_len_contract(frames, out_rate, in_rate).expect("non-zero rates");
                    assert!(
                        c.min <= c.ideal_floor,
                        "contract {c:?} excludes its own floor"
                    );
                    assert!(
                        c.ideal_ceil <= c.max,
                        "contract {c:?} excludes its own ceil"
                    );
                    assert!(c.min <= c.max);
                    // 理想值本身必须被接受。
                    assert_eq!(
                        check_resampled_len(frames, out_rate, in_rate, c.ideal_floor),
                        Ok(c.clone())
                    );
                    assert_eq!(
                        check_resampled_len(frames, out_rate, in_rate, c.ideal_ceil),
                        Ok(c)
                    );
                }
            }
        }
    }

    #[test]
    fn length_contract_pins_the_three_normative_conversions() {
        // 48000 @48k -> 44.1k 恰好 44100 帧（无舍入）。
        let c = resample_len_contract(48_000, 44_100, 48_000).unwrap();
        assert_eq!((c.ideal_floor, c.ideal_ceil), (44_100, 44_100));
        assert_eq!(c.min, 44_100 - 44);
        assert_eq!(c.max, 44_100 + 44);
        // 44100 @44.1k -> 48k 恰好 48000 帧。
        let c = resample_len_contract(44_100, 48_000, 44_100).unwrap();
        assert_eq!((c.ideal_floor, c.ideal_ceil), (48_000, 48_000));
        assert_eq!(c.min, 48_000 - 48);
        assert_eq!(c.max, 48_000 + 48);
        // 48000 @48k -> 96k 恰好翻倍。
        let c = resample_len_contract(48_000, 96_000, 48_000).unwrap();
        assert_eq!((c.ideal_floor, c.ideal_ceil), (96_000, 96_000));
    }

    #[test]
    fn length_contract_rejects_an_untrimmed_delay() {
        // 判据 (预填充/延迟语义): 若前置延迟没有被裁掉，输出会多出滤波器长度量级的帧。
        let in_frames = 48_000u64;
        let produced_without_trim = 44_100 + SINC_LEN; // sinc_len/2 量级的延迟在此之上
        assert!(
            check_resampled_len(in_frames, 44_100, 48_000, produced_without_trim).is_err(),
            "a {SINC_LEN}-frame delay must fall outside the contract"
        );
        // 而正确裁剪后的结果在区间内。
        assert!(check_resampled_len(in_frames, 44_100, 48_000, 44_100).is_ok());
    }

    #[test]
    fn idle_guard_trips_exactly_at_the_cap_and_resets_on_progress() {
        // 判据 (MUST-GATE-011 防挂死): 闸门必须真的会在上限处跳闸，且"有推进就清零"。
        let mut guard = IdleGuard::new();
        assert_eq!(guard.idle(), 0);
        for _ in 0..MAX_IDLE_PACKETS {
            assert!(!guard.bump(), "must not trip before the cap");
        }
        assert_eq!(guard.idle(), MAX_IDLE_PACKETS);
        assert!(guard.bump(), "must trip once past the cap");
        guard.reset();
        assert_eq!(guard.idle(), 0);
        assert!(!guard.bump());
        // 计数饱和而不是回绕（否则一个超长挂死又会从 0 开始）。
        let mut saturated = IdleGuard::new();
        for step in 0..(MAX_IDLE_PACKETS + 10) {
            assert_eq!(
                saturated.bump(),
                step >= MAX_IDLE_PACKETS,
                "step {step} tripped at the wrong moment"
            );
        }
        assert_eq!(saturated.idle(), MAX_IDLE_PACKETS + 10);
    }

    #[test]
    fn short_clips_get_an_explicit_delay_allowance() {
        // 短片段：理想输出 100 帧 < 4096，容差里必须含 SINC_LEN。
        let c = resample_len_contract(100, 44_100, 48_000).unwrap();
        assert!(c.max - c.ideal_ceil >= SINC_LEN);
        // 长片段：额外延迟放宽必须**消失**，容差只剩 0.1% 相对项。
        // 取理想输出 ≈ 199_369 帧（0.1% = 199 帧，仍小于 SINC_LEN），这样
        // "是否多给了 SINC_LEN" 可以从差值上直接读出来。
        let c = resample_len_contract(217_000, 44_100, 48_000).unwrap();
        let relative = c.ideal_floor * LEN_TOLERANCE_PPM / 1_000_000;
        assert!(
            relative < SINC_LEN,
            "test fixture must sit below the sinc allowance"
        );
        assert_eq!(c.max - c.ideal_ceil, relative);
        assert!(c.max - c.ideal_ceil < SINC_LEN);
    }
}
