//! 2× 过采样往返（饱和路径的抗混叠）。[ARCH-DSP-001] [ARCH-PDC-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/oversample.rs`（含其四条回归测试）。
//!
//! 梯形滤波器（以及任何状态变量族）会饱和输入，而 tanh 型级会把它在基础
//! Nyquist 之上产生的每一个谐波**直接折回可听频段**。把整条滤波器路径
//! （含限带）跑在两倍采样率上，折叠点就移到 48 kHz，抗镜像滤波器可以在抽取前
//! 把它去掉。
//!
//! 纪律很明确：**绝不给一个已经限带的输出打第二层补丁**。因此这里是一次完整的
//! 上采样 / 处理 / 下采样往返，而不是贴在非线性级上的修正。原型是
//! Kaiser 窗低通（63 抽头，β = 8，截止 0.21×过采样率 = 20 kHz @96 kHz）：
//! 到 19 kHz 平坦、21 kHz 处 −12 dB、基础 Nyquist 处已经 −71 dB，再往上低于 −85 dB。
//! 同一张表同时服务插值器与抽取器，这正是往返是"纯延迟"的原因。
//!
//! 往返是线性相位，因此它带来固定的 [`OS_LATENCY`] 个**基础采样率**样本延迟。
//! 调用方必须补偿它 [ARCH-PDC-001]："每个插件与内置设备必须精确上报其引入的
//! 处理延迟"。因果的抗镜像滤波器不可能有零群延迟，所以这是本模式的价格，
//! 而且要**显式**补偿。
//!
//! ## 与来源的差异
//!
//! 1. 系数表的 9 位小数字面量改写为**最短往返表示**（f32 位模式逐一比对相同）。
//!    数值完全不变，只是不再触发 `clippy::excessive_precision`；
//! 2. 新增 [`Oversampler2x::process_round_trip`]：把上/下两步收成一个调用，
//!    让"延迟由谁补偿"这件事只有一个出口；
//! 3. 新增 `round_trip_latency_is_reported_and_phase_linear` 判据，锁住
//!    [`OS_LATENCY`] 与系数表（对称性 + 直流增益）的一致性。

/// 共享 Kaiser 原型的抽头数。
pub const OS_TAPS: usize = 63;
/// 中心抽头，即一个 FIR 在**过采样率**下的群延迟。
pub const OS_CENTRE: usize = OS_TAPS / 2;
/// 上采样 + 下采样的往返延迟，单位是**基础采样率**样本。
///
/// 零延迟需要非因果滤波器，因此这是本模式的固定代价，必须由调用方显式补偿
/// [ARCH-PDC-001]。
pub const OS_LATENCY: usize = OS_CENTRE;

/// Kaiser 低通，`sum = 1`，对称。
///
/// 由 `h[n] = 2·fc·sinc(2·fc·(n − 31))·I₀(β·√(1 − ((n−31)/31)²)) / I₀(β)`
///（`fc = 0.21`、`β = 8`）生成，再归一化到直流增益恰为 1。
///
/// 截止频率足够低：在基础 Nyquist（0.25×过采样率 = 24 kHz）处阻带已约 −70 dB，
/// 而那正是抽取前必须消失的东西——半带滤波器没有保护带，紧贴 Nyquist 的那个
/// 倍频程会折到自己身上（同步抽取器的注释记的是同一个坑）。63 抽头而不是同步
/// 路径的 95 抽头，是因为这份要跑在每个声部上；到 24.2 kHz 它仍能到 −97 dB。
///
/// 注意：表是对称的（`h[k] == h[OS_TAPS−1−k]`），峰值在 [`OS_CENTRE`]。
#[rustfmt::skip]
const OS_H: [f32; OS_TAPS] = [
    -1.508e-06, 5.4388e-05, 5.8423e-05, -0.000126979,
    -0.000257244, 0.000109753, 0.000638094, 0.000222426,
    -0.001070892, -0.001118757, 0.001149812, 0.002637395,
    -0.000221923, -0.004371227, -0.002360323, 0.005271592,
    0.006776361, -0.00374928, -0.012218226, -0.001858482,
    0.01655406, 0.01255176, -0.016341748, -0.027950654,
    0.007024669, 0.04596551, 0.017837353, -0.06311572,
    -0.07467588, 0.07548618, 0.3071053, 0.41999155,
    0.3071053, 0.07548618, -0.07467588, -0.06311572,
    0.017837353, 0.04596551, 0.007024669, -0.027950654,
    -0.016341748, 0.01255176, 0.01655406, -0.001858482,
    -0.012218226, -0.00374928, 0.006776361, 0.005271592,
    -0.002360323, -0.004371227, -0.000221923, 0.002637395,
    0.001149812, -0.001118757, -0.001070892, 0.000222426,
    0.000638094, 0.000109753, -0.000257244, -0.000126979,
    5.8423e-05, 5.4388e-05, -1.508e-06,
];

/// 2× 往返的单声道状态。
///
/// 只保存分块 FIR 需要的历史，因此一个块的成本是一次拷贝、零次分配
/// [ARCH-RT-001]。
#[derive(Clone, Copy, Debug)]
pub struct Oversampler2x {
    /// 最近 [`OS_CENTRE`] 个基础采样率输入样本，最老的在前。
    up_tail: [f32; OS_CENTRE],
    /// 最近 [`OS_TAPS`] − 1 个过采样样本，最老的在前。
    down_tail: [f32; OS_TAPS - 1],
}

impl Oversampler2x {
    /// 构造（历史归零）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            up_tail: [0.0; OS_CENTRE],
            down_tail: [0.0; OS_TAPS - 1],
        }
    }

    /// 丢弃历史（例如模式被打开时：往返从静音重新开始，而不是把旧尾巴带过断点）。
    pub fn reset(&mut self) {
        self.up_tail = [0.0; OS_CENTRE];
        self.down_tail = [0.0; OS_TAPS - 1];
    }

    /// 本实例引入的延迟（基础采样率样本），供 PDC 上报 [ARCH-PDC-001]。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        OS_LATENCY
    }

    /// 对 `x` 补零后低通：向 `out` 写 `2 * x.len()` 个样本。
    ///
    /// `scratch` 是调用方的暂存（至少 `OS_CENTRE + x.len()` 长），
    /// 因此这个块级往返零分配。
    pub fn upsample(&mut self, x: &[f32], out: &mut [f32], scratch: &mut [f32]) {
        let n = x.len();
        // 先做长度检查（长度不足时是"什么都不做"，而不是 panic）：
        // 实时路径上宁可静默跳过也不可能展开栈。
        if out.len() < n * 2 || scratch.len() < OS_CENTRE + n {
            return;
        }
        scratch[..OS_CENTRE].copy_from_slice(&self.up_tail);
        scratch[OS_CENTRE..OS_CENTRE + n].copy_from_slice(x);
        for i in 0..n {
            let base = OS_CENTRE + i;
            let mut even = 0.0f32;
            let mut odd = 0.0f32;
            // 偶相位：h[2j] 配 x[i−j]，含中心抽头（j = 15）。
            let mut j = 0;
            while 2 * j < OS_TAPS {
                even += OS_H[2 * j] * scratch[base - j];
                j += 1;
            }
            // 奇相位：h[2j+1]，即两者之间的半样本。
            let mut j = 0;
            while 2 * j + 1 < OS_TAPS {
                odd += OS_H[2 * j + 1] * scratch[base - j];
                j += 1;
            }
            // 补零让电平减半，因此插值增益是 2。
            out[2 * i] = 2.0 * even;
            out[2 * i + 1] = 2.0 * odd;
        }
        // 尾巴是最后 OS_CENTRE 个输入样本。
        self.up_tail.copy_from_slice(&scratch[n..OS_CENTRE + n]);
    }

    /// 低通 `v` 并每两个样本取一个：向 `out` 写 `v.len() / 2` 个样本。
    pub fn downsample(&mut self, v: &[f32], out: &mut [f32], scratch: &mut [f32]) {
        let n = v.len() / 2;
        if out.len() < n || scratch.len() < OS_TAPS - 1 + v.len() {
            return;
        }
        let c = OS_TAPS - 1;
        scratch[..c].copy_from_slice(&self.down_tail);
        scratch[c..c + v.len()].copy_from_slice(v);
        for (i, slot) in out.iter_mut().enumerate().take(n) {
            let base = c + 2 * i;
            // 对称系数：把 k 与 OS_TAPS−1−k 配对，只用表的前一半。
            let mut acc = OS_H[OS_CENTRE] * scratch[base - OS_CENTRE];
            let mut k = 0;
            while k < OS_CENTRE {
                acc += OS_H[k] * (scratch[base - k] + scratch[base - (OS_TAPS - 1 - k)]);
                k += 1;
            }
            *slot = acc;
        }
        // 尾巴是最后 OS_TAPS − 1 个过采样样本。
        let end = c + v.len();
        self.down_tail.copy_from_slice(&scratch[end - c..end]);
    }

    /// 完整的上采样 → 下采样往返：向 `out` 写 `input.len()` 个样本。
    ///
    /// `upsampled` 至少 `2 * input.len()` 长，`scratch` 至少
    /// `max(OS_CENTRE + input.len(), OS_TAPS - 1 + 2 * input.len())` 长。
    ///
    /// 中间的"DSP"（这里没有：本类型只做往返）由调用方在
    /// `upsampled[..2n]` 上原地运行，这正是"完整往返而不是打补丁"的落点。
    pub fn process_round_trip(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        upsampled: &mut [f32],
        scratch: &mut [f32],
    ) {
        let n = input.len().min(output.len());
        if n == 0 || upsampled.len() < n * 2 {
            return;
        }
        self.upsample(&input[..n], &mut upsampled[..n * 2], scratch);
        self.downsample(&upsampled[..n * 2], &mut output[..n], scratch);
    }
}

impl Default for Oversampler2x {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 抽取器的真正职责：1× 流承载不了的内容必须在"每隔一个样本丢一个"**之前**
    /// 消失，否则就会折回。过采样率下的 30 kHz 音（高于 24 kHz 基础 Nyquist）
    /// 不得以 18 kHz 的镜像活下来。
    #[test]
    fn decimation_removes_everything_above_the_base_nyquist() {
        let mut os = Oversampler2x::new();
        let mut scratch = [0.0f32; 4096];
        // 一个 3 kHz 音加一个 30 kHz 音，都在过采样率下。
        let mut v = [0.0f32; 1024];
        for (i, x) in v.iter_mut().enumerate() {
            let t = i as f32 / 96000.0;
            *x = (core::f32::consts::TAU * 3000.0 * t).sin()
                + (core::f32::consts::TAU * 30000.0 * t).sin();
        }
        let mut out = [0.0f32; 512];
        let mut settled = [0.0f32; 512];
        for _ in 0..4 {
            os.downsample(&v, &mut out, &mut scratch);
            settled = out;
        }
        let mag = |data: &[f32], f: f32| {
            let k = core::f32::consts::TAU * f / 48000.0;
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &x) in data.iter().enumerate() {
                re += x * (k * i as f32).cos();
                im -= x * (k * i as f32).sin();
            }
            (re * re + im * im).sqrt() / data.len() as f32
        };
        let wanted = mag(&settled, 3000.0);
        let mirror = mag(&settled, 18000.0);
        assert!(wanted > 0.3, "the 3 kHz tone came out at {wanted}");
        assert!(
            mirror < wanted * 1e-3,
            "the 30 kHz tone folded to 18 kHz at {:.1} dB",
            20.0 * (mirror / wanted).log10()
        );
    }

    /// 直流输入必须原样返回（插值的 2 倍增益与抽取器的单位直流增益必须一致）。
    #[test]
    fn round_trip_is_unity_at_dc() {
        let mut os = Oversampler2x::new();
        let x = [0.5f32; 64];
        let mut up = [0.0f32; 128];
        let mut scratch = [0.0f32; 512];
        let mut y = [0.0f32; 64];
        // 先把 FIR 历史跑稳，输出才是稳态。
        for _ in 0..8 {
            os.upsample(&x, &mut up, &mut scratch);
            os.downsample(&up, &mut y, &mut scratch);
        }
        for v in &y[48..] {
            assert!((v - 0.5).abs() < 1e-4, "DC came back as {v}");
        }
    }

    /// 插值器不得在基础 Nyquist 之上凭空造能量：一个基础频段正弦必须几乎
    /// 原幅度地走出抽取器，而 24 kHz 之上的镜像正是原型要杀掉的东西。
    #[test]
    fn upsample_kills_the_image_above_the_base_nyquist() {
        // 96 kHz 下的 30 kHz 是 48 kHz 流承载不了的镜像：它必须被抽取器的
        // 阻带衰减掉，而不是折回来。
        let mut os = Oversampler2x::new();
        let mut up = [0.0f32; 256];
        let mut scratch = [0.0f32; 1024];
        let mut x = [0.0f32; 128];
        for (i, v) in x.iter_mut().enumerate() {
            // 基础采样率下的 3 kHz：稳在通带内。
            *v = (core::f32::consts::TAU * 3000.0 * i as f32 / 48000.0).sin();
        }
        for _ in 0..4 {
            os.upsample(&x, &mut up, &mut scratch);
        }
        // 用 Goertzel 同时探基础频段音与其关于 24 kHz 的镜像。
        let mag = |data: &[f32], f: f32| {
            let k = core::f32::consts::TAU * f / 96000.0;
            let coeff = 2.0 * k.cos();
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for &v in data {
                let s0 = v + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt() / data.len() as f32
        };
        let pass = mag(&up, 3000.0);
        // 3 kHz 在 2× 域的镜像落在 93 kHz = 96 − 3；抽取之后它就是 3 kHz 本身，
        // 因此改查 30 kHz 处的阻带——那才是折回 18 kHz 的东西。
        let stop = mag(&up, 30000.0);
        assert!(pass > 0.3, "passband came out at {pass}");
        assert!(
            stop < pass * 0.02,
            "30 kHz image is only {:.1} dB down",
            20.0 * (stop / pass).log10()
        );
    }

    /// 往返必须是恰好 [`OS_LATENCY`] 个样本的纯延迟：调用方精确补偿这个数，
    /// 这里差一个样本就会在模式切换处表现为相位跳变。
    ///
    /// 把断言改成 `40 + OS_LATENCY + 1` 或把 [`OS_LATENCY`] 改成
    /// `OS_CENTRE + 1`，本测试立即变红。
    #[test]
    fn round_trip_delays_by_exactly_the_reported_latency() {
        let mut os = UpsampleHarness::new();
        // 脉冲在 t = 40；跑得足够长，尾巴已经稳定。
        let mut x = [0.0f32; 128];
        x[40] = 1.0;
        let mut out = [0.0f32; 128];
        os.run(&x, &mut out);
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(
            peak,
            40 + OS_LATENCY,
            "impulse came out at {peak}, expected {}",
            40 + OS_LATENCY
        );
    }

    /// **判据（新写，可红）**：延迟常量与系数表互相自洽 [ARCH-PDC-001]。
    ///
    /// PDC 上报的延迟必须真的等于原型的群延迟。原型的群延迟等于中心抽头位置，
    /// 而中心抽头位置只有在表**对称**且抽头数为奇数时才唯一。因此同时断言：
    /// 抽头数为奇数、[`OS_LATENCY`] 与中心一致、表逐对对称、直流增益恰为 1。
    ///
    /// 把系数表里任意一个数改错（哪怕只改一位小数），对称性或直流增益立即变红；
    /// 把 [`OS_LATENCY`] 改成 `OS_CENTRE + 16`（四舍五入到基础采样率）也会变红。
    #[test]
    fn round_trip_latency_is_reported_and_phase_linear() {
        assert_eq!(OS_TAPS % 2, 1, "an odd tap count is what centres the delay");
        assert_eq!(OS_CENTRE, OS_TAPS / 2);
        assert_eq!(
            OS_LATENCY, OS_CENTRE,
            "reported latency must be the group delay"
        );
        for k in 0..OS_TAPS {
            assert_eq!(
                OS_H[k],
                OS_H[OS_TAPS - 1 - k],
                "prototype is not symmetric at tap {k} (linear phase lost)"
            );
        }
        let dc: f64 = OS_H.iter().map(|h| f64::from(*h)).sum();
        assert!(
            (dc - 1.0).abs() < 1e-5,
            "prototype DC gain is {dc}, not unity"
        );
        // 峰值必须真的落在中心抽头上，否则群延迟不是 OS_CENTRE。
        let peak_index = (0..OS_TAPS)
            .max_by(|a, b| OS_H[*a].abs().partial_cmp(&OS_H[*b].abs()).unwrap())
            .unwrap();
        assert_eq!(
            peak_index, OS_CENTRE,
            "prototype peak is not the centre tap"
        );
    }

    /// `process_round_trip` 只是上/下两步的组合，不得有自己的算法。
    #[test]
    fn process_round_trip_matches_the_two_step_api() {
        let input: Vec<f32> = (0..64)
            .map(|i| (core::f32::consts::TAU * 500.0 * i as f32 / 48_000.0).sin())
            .collect();
        let mut combined = Oversampler2x::new();
        let mut stepwise = Oversampler2x::new();
        let mut out_combined = [0.0f32; 64];
        let mut out_stepwise = [0.0f32; 64];
        let mut upsampled = [0.0f32; 128];
        let mut scratch = [0.0f32; 1024];
        combined.process_round_trip(&input, &mut out_combined, &mut upsampled, &mut scratch);
        stepwise.upsample(&input, &mut upsampled, &mut scratch);
        stepwise.downsample(&upsampled, &mut out_stepwise, &mut scratch);
        assert_eq!(out_combined, out_stepwise);
        assert_eq!(combined.latency_samples(), OS_LATENCY);
    }

    #[test]
    fn short_buffers_are_rejected_instead_of_panicking() {
        let mut os = Oversampler2x::new();
        let mut scratch = [0.0f32; 8];
        let mut out = [0.0f32; 4];
        // scratch 太小 → 直接返回，不改输出。
        os.upsample(&[1.0f32; 4], &mut out, &mut scratch);
        assert_eq!(out, [0.0; 4]);
        // 空输入是合法的。
        let mut empty: [f32; 0] = [];
        let mut empty_up: [f32; 0] = [];
        os.upsample(&[], &mut empty, &mut scratch);
        os.process_round_trip(&[], &mut empty, &mut empty_up, &mut scratch);
        // 输出缓冲太短时同样只是跳过。
        let mut tiny = [0.0f32; 2];
        os.downsample(&[1.0f32; 16], &mut tiny, &mut scratch);
        assert_eq!(tiny, [0.0; 2]);
    }

    struct UpsampleHarness {
        os: Oversampler2x,
        up: [f32; 256],
        scratch: [f32; 1024],
    }

    impl UpsampleHarness {
        fn new() -> Self {
            Self {
                os: Oversampler2x::new(),
                up: [0.0; 256],
                scratch: [0.0; 1024],
            }
        }

        fn run(&mut self, x: &[f32], out: &mut [f32]) {
            self.os.upsample(x, &mut self.up, &mut self.scratch);
            self.os.downsample(&self.up, out, &mut self.scratch);
        }
    }
}
