//! ⭐ **流式采样率转换**（T-FIX-72）：设备采样率 ≠ 引擎（工程）采样率时，在实时
//! 通路里把一条率的交错样本流连续地换到另一条率，速度与音高保持正确。
//!
//! ## 为什么在本仓自己实现（⛔ 不引流式重采样依赖）
//!
//! - 离线腿已有 `yeban-decode` 的 rubato 高精 sinc（整段、允许分配）；实时腿的
//!   硬契约是**回调稳态零分配 / 零锁 / 零 I/O**（红线 7 / `MUST-GATE-001`），
//!   需要一个状态与缓冲在**打开设备时预分配**、推进时只做定长乘加的实现，
//!   仓内没有现成的这种形态；
//! - 本实现是**多相窗化 sinc**（Blackman 窗、逐相位行归一化 ⇒ 直流精确），
//!   相位推进用**精确有理数**（步长 = 约分后的 `in_rate / out_rate`，`u64`
//!   分子累加）⇒ 长跑**零漂移**由构造保证，不是靠浮点近似。
//!
//! ## 推进模型（push）与历史深度不变式
//!
//! 调用方逐帧 [`RateConverter::push_frame`]：先把已就绪的输出排进 `out`
//! （排空尝试），再把新帧写进环形历史，然后再排一次。就绪条件是输出位置的
//! 整个窗（[`RESAMPLER_TAPS`] 帧）都已进历史。
//!
//! ⚠ **不变式**：两次「排空尝试」之间最多追加 [`MAX_PUSH_BURST`] 帧 —— 输出腿
//! 按 128 帧量子推进（每帧都带一次排空尝试），输入腿逐帧推进，都满足。
//! 历史深度 [`HISTORY_FRAMES`] = 窗长 + 一整个量子 + 余量 ⇒ 待排输出的窗
//! 永远落在驻留窗内（推导见 `push_frame` 的注释）。
//!
//! ## 延迟
//!
//! 恒定延迟 = [`RESAMPLER_TAPS`] / 2 个**输入**帧（窗的后半未到齐前不发输出）。
//! 内容对齐不偏移：第 `k` 个输出帧对应的输入位置恒为 `k × in_rate / out_rate`
//! （精确有理），延迟只影响"何时交付"，不影响"交付的是哪个时刻的声音"。

use core::f64::consts::PI;

/// 多相表的相位行数（分数位置量化到 1/256 帧）。
pub const RESAMPLER_PHASES: usize = 256;
/// 窗化 sinc 的总抽头数（窗长，单位：输入帧）。
pub const RESAMPLER_TAPS: usize = 32;
/// 两次排空尝试之间允许追加的最大帧数（= 引擎渲染量子）。
pub const MAX_PUSH_BURST: usize = 128;
/// 环形历史深度（帧）：窗长 + 一个量子 + 余量。
pub const HISTORY_FRAMES: usize = RESAMPLER_TAPS + MAX_PUSH_BURST + 96;

/// 一台流式采样率转换器（单向：`in_rate` → `out_rate`，通道数不变）。
///
/// 构造（[`RateConverter::new`]）是**唯一**分配点：系数表与历史缓冲都在这里
/// 按最终尺寸建好；此后 [`RateConverter::push_frame`] 只做定长乘加与索引运算，
/// ⛔ 不分配、不加锁、无 I/O —— 可以直接放进音频回调的推进链。
#[derive(Debug)]
pub struct RateConverter {
    /// 输入采样率 (Hz)。
    in_rate: u32,
    /// 输出采样率 (Hz)。
    out_rate: u32,
    /// 通道数（输入与输出相同；转换逐通道独立、用同一组系数）。
    channels: usize,
    /// 约分后的步长分子：每个输出帧前进 `step_num / step_den` 个输入帧。
    step_num: u64,
    /// 约分后的步长分母。
    step_den: u64,
    /// 多相系数表：`RESAMPLER_PHASES × RESAMPLER_TAPS`，行 = 分数相位，
    /// 行内逐抽头归一化（行和 = 1 ⇒ 直流信号逐位通过）。
    coeffs: Box<[f32]>,
    /// 环形历史：`HISTORY_FRAMES × channels` 交错；槽 = 绝对帧号 mod 深度。
    history: Box<[f32]>,
    /// 已推入的输入帧总数（绝对计数）。
    received: u64,
    /// 下一个输出帧的位置分子：其输入位置 = `next_out_num / step_den`。
    next_out_num: u64,
    /// 已发出的输出帧总数。
    emitted: u64,
}

/// `u64` 最大公约数（构造期用，非实时路径）。
const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

impl RateConverter {
    /// 构造一台 `in_rate` → `out_rate` 的转换器。
    ///
    /// 返回 `None` 的情形（调用方据此走直通/报错，⛔ 不许静默造一台假的）：
    /// 任一采样率为 0、通道数为 0、或 `in_rate == out_rate`（等率不需要转换，
    /// 调用方必须走逐位直通路径）。
    #[must_use]
    pub fn new(in_rate: u32, out_rate: u32, channels: u16) -> Option<Self> {
        if in_rate == 0 || out_rate == 0 || channels == 0 || in_rate == out_rate {
            return None;
        }
        let channels = usize::from(channels);
        let divisor = gcd(u64::from(in_rate), u64::from(out_rate));
        let step_num = u64::from(in_rate) / divisor;
        let step_den = u64::from(out_rate) / divisor;

        // 系数表（构造期，允许浮点超越函数；用 `libm` 保持跨平台一致
        // [ARCH-DET-001]）。截止频率按**输入率**归一：升采样取 1（输入自身的
        // 带限），降采样取 out/in（抗混叠）。
        let cutoff = (f64::from(out_rate) / f64::from(in_rate)).min(1.0);
        let half = (RESAMPLER_TAPS / 2) as f64;
        let mut coeffs = vec![0.0f32; RESAMPLER_PHASES * RESAMPLER_TAPS];
        for phase in 0..RESAMPLER_PHASES {
            let frac = phase as f64 / RESAMPLER_PHASES as f64;
            let mut row = [0.0f64; RESAMPLER_TAPS];
            let mut sum = 0.0f64;
            for (tap, slot) in row.iter_mut().enumerate() {
                // 抽头 tap 读输入帧 floor(x) - TAPS/2 + 1 + tap，它与输出位置
                // x 的距离 d = tap - TAPS/2 + 1 - frac。
                let d = tap as f64 - half + 1.0 - frac;
                let sinc = if d == 0.0 {
                    cutoff
                } else {
                    libm::sin(PI * cutoff * d) / (PI * d)
                };
                // Blackman 窗（|d| < half 时非零，端点恰为 0）。
                let w = if d.abs() < half {
                    0.42 + 0.5 * libm::cos(PI * d / half) + 0.08 * libm::cos(2.0 * PI * d / half)
                } else {
                    0.0
                };
                *slot = sinc * w;
                sum += *slot;
            }
            // 逐行归一化 ⇒ 行和 = 1：直流（恒定）信号逐位通过，增益不漂移。
            let base = phase * RESAMPLER_TAPS;
            for (tap, value) in row.iter().enumerate() {
                coeffs[base + tap] = (value / sum) as f32;
            }
        }

        Some(Self {
            in_rate,
            out_rate,
            channels,
            step_num,
            step_den,
            coeffs: coeffs.into_boxed_slice(),
            history: vec![0.0f32; HISTORY_FRAMES * channels].into_boxed_slice(),
            received: 0,
            next_out_num: 0,
            emitted: 0,
        })
    }

    /// 输入采样率 (Hz)。
    #[must_use]
    pub const fn in_rate(&self) -> u32 {
        self.in_rate
    }

    /// 输出采样率 (Hz)。
    #[must_use]
    pub const fn out_rate(&self) -> u32 {
        self.out_rate
    }

    /// 通道数。
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.channels as u16
    }

    /// 恒定交付延迟（输入帧）：窗后半长。
    #[must_use]
    pub const fn latency_input_frames(&self) -> u64 {
        (RESAMPLER_TAPS / 2) as u64
    }

    /// 已推入的输入帧总数。
    #[must_use]
    pub const fn received(&self) -> u64 {
        self.received
    }

    /// 已发出的输出帧总数。
    #[must_use]
    pub const fn emitted(&self) -> u64 {
        self.emitted
    }

    /// 下一个输出帧是否已就绪（它的整个窗都已进历史）。
    fn next_ready(&self) -> bool {
        if self.received == 0 {
            return false;
        }
        let i0 = self.next_out_num / self.step_den;
        // 就绪 ⟺ 最大抽头帧号（i0 + TAPS/2）≤ 已收到的最后一帧，
        // 即"所需输入帧数"（i0 + TAPS/2 + 1）不超过已收到的帧数。
        let frames_needed = i0 + (RESAMPLER_TAPS / 2) as u64 + 1;
        frames_needed <= self.received
    }

    /// 把已就绪的输出帧排进 `out`（从 `*filled` 帧处起），排到 `out` 满或
    /// 无就绪输出为止。返回本次排出的帧数。
    fn drain_ready(&mut self, out: &mut [f32], filled: &mut usize) -> usize {
        let channels = self.channels;
        let capacity = out.len() / channels;
        let mut drained = 0usize;
        while *filled < capacity && self.next_ready() {
            let i0 = self.next_out_num / self.step_den;
            let rem = self.next_out_num % self.step_den;
            let phase = (rem * RESAMPLER_PHASES as u64 / self.step_den) as usize;
            let row = &self.coeffs[phase * RESAMPLER_TAPS..(phase + 1) * RESAMPLER_TAPS];
            let base = *filled * channels;
            for channel in 0..channels {
                let mut acc = 0.0f32;
                for (tap, weight) in row.iter().enumerate() {
                    // 抽头 tap 的绝对帧号（可为负 ⇒ 历史尚不存在 ⇒ 0）。
                    let index = i0 as i64 - (RESAMPLER_TAPS / 2) as i64 + 1 + tap as i64;
                    let sample = if index < 0 {
                        0.0
                    } else {
                        let slot = (index as u64 % HISTORY_FRAMES as u64) as usize;
                        self.history[slot * channels + channel]
                    };
                    acc += weight * sample;
                }
                out[base + channel] = acc;
            }
            *filled += 1;
            drained += 1;
            self.emitted += 1;
            self.next_out_num += self.step_num;
        }
        drained
    }

    /// ⭐ **推进一帧**：先排空已就绪输出，再把 `frame`（`channels` 个交错样本）
    /// 写进历史，最后再排一次。
    ///
    /// `out` 是完整的输出缓冲，`*filled` 是其中已写好的帧数（输入/输出）。
    /// 返回本次总共排出的帧数。`out` 已满时仍会收下 `frame`（待排输出留在
    /// 状态里，下一次调用先排它们）—— 历史深度不变式（模块文档）保证这期间
    /// 待排输出的窗不被覆盖：两次排空尝试之间最多追加 [`MAX_PUSH_BURST`] 帧，
    /// 而每次 `push_frame` 都做两次排空尝试。
    ///
    /// 实时契约：零分配、零锁、零 I/O，只做定长乘加 [红线 7]。
    pub fn push_frame(&mut self, frame: &[f32], out: &mut [f32], filled: &mut usize) -> usize {
        debug_assert_eq!(
            frame.len(),
            self.channels,
            "推入帧的宽度必须等于构造时的通道数"
        );
        let mut drained = self.drain_ready(out, filled);
        let slot = (self.received % HISTORY_FRAMES as u64) as usize;
        let base = slot * self.channels;
        self.history[base..base + self.channels].copy_from_slice(frame);
        self.received += 1;
        drained += self.drain_ready(out, filled);
        drained
    }

    /// 把一整块交错输入（`frames × channels`）逐帧推进，输出排进 `out`。
    /// 返回本次排出的输出帧数。语义与逐帧 [`RateConverter::push_frame`] 相同。
    pub fn push_block(&mut self, block: &[f32], out: &mut [f32], filled: &mut usize) -> usize {
        let channels = self.channels;
        let frames = block.len() / channels;
        let mut drained = 0usize;
        for index in 0..frames {
            let frame = &block[index * channels..(index + 1) * channels];
            drained += self.push_frame(frame, out, filled);
        }
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_or_invalid_rates_have_no_converter() {
        assert!(
            RateConverter::new(48_000, 48_000, 2).is_none(),
            "等率必须直通"
        );
        assert!(RateConverter::new(0, 44_100, 2).is_none());
        assert!(RateConverter::new(48_000, 0, 2).is_none());
        assert!(RateConverter::new(48_000, 44_100, 0).is_none());
        assert!(RateConverter::new(48_000, 44_100, 2).is_some());
    }

    #[test]
    fn dc_signal_passes_through_unchanged() {
        // 行归一化的直接读数：恒定 0.5 进去，稳态后恒定 0.5 出来（容差只留
        // f32 系数量化）。
        let mut converter = RateConverter::new(48_000, 44_100, 1).expect("异率");
        let mut out = [0.0f32; 512];
        let mut filled = 0usize;
        for _ in 0..4_096 {
            filled = 0;
            converter.push_frame(&[0.5], &mut out, &mut filled);
        }
        // 最后一次调用排出的帧必须全是 0.5（窗已全为 0.5 的稳态）。
        assert!(filled > 0, "稳态后每次推进都应有输出");
        for sample in &out[..filled] {
            assert!((sample - 0.5).abs() < 1e-6, "直流必须逐位通过: {sample}");
        }
    }
}
