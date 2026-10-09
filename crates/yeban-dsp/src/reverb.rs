//! 立体声混响：Freeverb 拓扑 + 合成器音色需要的控制量。[ARCH-RT-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/reverb.rs`（含其全部回归测试）。
//!
//! 每声道 8 个带阻尼的梳状滤波器 + 4 个全通扩散器，经典立体声展宽，加上被替换掉的
//! Soundpipe 混响没有的控制：高频**阻尼**、**预延迟**与立体声**宽度**。延迟线是
//! 刻意静态的——反馈环里的移动抽头会在延迟变短时注入能量，那会让尾巴"泵"起来
//! 而不是衰减。
//!
//! 延迟线在设置采样率时分配一次，因此渲染循环本身从不分配 [ARCH-RT-001]。
//!
//! ## 与来源的差异
//!
//! 1. [`Reverb::set_sample_rate`] 被明确标注为本类型**唯一**的分配入口，
//!    必须在音频回调之外调用；
//! 2. 存储真实采样率（来源在 `set_params` 里用 `44_100.0 * sr_scale` 反推，
//!    当采样率落在 `[22.05 kHz, 96 kHz]` 之外时那个反推值是错的）；
//! 3. `process` 增加"未配置即直通"的守卫：来源在从未调用 `set_sample_rate`
//!    时会在 `self.pre[0][self.pre_index]` 上越界 panic，而实时路径上不允许；
//! 4. `process` 用 `zip` 而不是按下标写 `right[i]`（同上，长度不等就会 panic）；
//! 5. 立体声展宽与前置延迟的事件顺序、参数映射常量（0.70…0.94 反馈、
//!    0.05…0.77 阻尼、`WET_GAIN`）逐一保持与来源相同，既有音色不变；
//! 6. **新增** [`Reverb::reset`] 与 [`Reverb::latency_samples`]（＋公共常量
//!    [`REVERB_LATENCY`]）—— 来源没有这两项，见下一节。两者都是**纯增量**：
//!    `process` 的逐样本路径一个字都没改，既有音色与既有读数不变。
//!
//! ## 器件形状与延迟上报 [ARCH-PDC-001]
//!
//! 本类型与 `compressor` / `channel_strip` / `limiter` / `polysynth` / `drums` /
//! `convolution_reverb` 同形：
//! `new` / `set_sample_rate` / `set_params` / `params` / `is_configured` /
//! `is_active` / `process` / `reset` / `latency_samples`。
//!
//! 后两项是本线按 [ARCH-PDC-001] 补齐的。规范原文（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`
//! 第 398 行）要求"每个插件与内置设备必须精确上报其引入的处理延迟
//! （`DeviceDefinition::latency_samples`）"。Freeverb 的预延迟**只**推迟送进梳状组的
//! 湿信号，干路逐样本即时 ⇒ 本器件的净延迟是 **0**，见 [`Reverb::latency_samples`]。
//! 口径与 [`crate::convolution_reverb`] 模块文档 §2.1 相同。
//!
//! [`Reverb::reset`] 是"清历史、不动配置"的唯一入口：停带、定位或换工程时，
//! 反馈环里的尾巴必须有一个定义明确的去处（否则同一段输入在第二次渲染里与第一次
//! 不同）。它逐样本零分配。
//!
//! ## 5. 非有限**输入样本**（本轮补齐的成员）
//!
//! [`Reverb::set_params`] 早已把非有限**参数**回落成缺省值，理由写在那个函数的
//! 文档里（"会把整个梳状组的反馈变成 `NaN` 并静默毒化整条混响总线"）。
//! **输入样本走的是同一条路**，而本条此前缺失：`NaN`／`±∞` 写进预延迟线后，
//! `input = (dl + dr) · 0.5 · 0.015` 立刻把它们送进 8 条梳状线 ＋ 4 个全通，
//! 而反馈环里 `NaN · damp = NaN`、`∞ · feedback = ∞` ⇒ **一次**坏样本之后
//! 湿路永久非有限，连输入恢复干净也回不来（`reset()` 是唯一出路）。
//!
//! ⇒ [`Reverb::process`] 在**每样本进入递归之前**过 `math::finite_or_zero`：
//! `NaN` 与 `±∞` 归 `0.0`，**有限样本逐位不变**。因此
//! 1. 既有音色、既有读数、既有判据一个比特都不改；
//! 2. `NaN` 样本与"同一位置的 `0.0` 样本"从此**逐位同解**（这正是
//!    `tests/non_finite_never_enters_a_recursion.rs` 钉住的形态）。
//!
//! ⚠ 本守卫与"未配置／湿路不可闻即直通"的短路**不冲突**：[`Reverb::process`] 的
//! 提前返回发生在守卫之前，因此旁通状态仍是**逐位直通**（含非有限样本）。
//! 这不是漏洞：没有样本进入递归，就没有东西可毒化。
//!
//! ⚠ 守卫**不**覆盖"有限但极大"的输入（`3e38` 仍可能溢出成 `∞`）。那条归调用方的
//! 电平口径管，见 `math::finite_or_zero` 的文档。

use crate::math::finite_or_zero;

/// 梳状调音，单位样本 @44.1 kHz（Freeverb 的原始数字）。
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];

/// 全通调音，单位样本 @44.1 kHz。
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];

/// 右声道偏移量，用来展宽立体声像。
const STEREO_SPREAD: usize = 23;

/// 96 kHz 缩放与调制余量之后最长的梳状延迟。
const COMB_MAX: usize = 4096;

/// 全通延迟上限。
const ALLPASS_MAX: usize = 1536;

/// 预延迟缓冲：100 ms @96 kHz。
const PREDELAY_MAX: usize = 9600;

/// 本器件引入的**处理延迟**（帧）。`0` = 不参与 [ARCH-PDC-001] 的补偿量。
///
/// 取 `0` 的理由与 [`crate::convolution::CONV_LATENCY`] 同款：本器件的预延迟
/// **只**推迟湿路，干路用的是当前样本 ⇒ 整机对输入没有净延迟。
/// ⚠ 这意味着"预延迟"**不是** `ARCH-PDC-001` 的延迟，[`Reverb::latency_samples`]
/// 不把它算进去。
pub const REVERB_LATENCY: usize = 0;

/// 湿路径的输出微调。梳状组加四级全通扩散器有很高的谐振增益
///（在梳谐振处可达 ~25×），因此原始求和远热于干信号；这一项把全湿拉回大约单位增益。
const WET_GAIN: f32 = 0.4;

/// 从 44.1 kHz 调到最高支持采样率的缩放系数上限。
const SR_SCALE_MAX: f32 = 96_000.0 / 44_100.0;

/// 单条梳状线（内部类型，不对外暴露）。
struct Comb {
    /// 在 [`Reverb::set_sample_rate`] 里分配一次，渲染循环内不再触碰。
    buf: Vec<f32>,
    len: usize,
    index: usize,
    store: f32,
    damp: f32,
    feedback: f32,
}

impl Comb {
    const fn new() -> Self {
        Self {
            buf: Vec::new(),
            len: 1,
            index: 0,
            store: 0.0,
            damp: 0.2,
            feedback: 0.84,
        }
    }

    fn setup(&mut self, base_len: usize, sr_scale: f32) {
        let scaled = (base_len as f32 * sr_scale) as usize;
        self.len = scaled.clamp(64, COMB_MAX - 8);
        if self.buf.len() != self.len {
            self.buf = vec![0.0; self.len];
        } else {
            self.buf.fill(0.0);
        }
        self.index = 0;
        self.store = 0.0;
    }

    /// 清空这条延迟线的**历史**（缓冲内容 ＋ 一极点阻尼状态 ＋ 读写头）。
    ///
    /// 长度与系数（`len` / `damp` / `feedback`）不动 —— 清的是历史，不是配置。
    /// 未配置时 `buf` 是空 `Vec`，`fill` 是空操作（不 panic、不分配）。
    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.index = 0;
        self.store = 0.0;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let out = self.buf[self.index];
        // 反馈路径里的一极点阻尼。
        self.store = out * (1.0 - self.damp) + self.store * self.damp;
        self.buf[self.index] = input + self.store * self.feedback;
        self.index += 1;
        if self.index >= self.len {
            self.index = 0;
        }
        out
    }
}

/// 单个全通扩散器（内部类型）。
struct Allpass {
    buf: Vec<f32>,
    len: usize,
    index: usize,
}

impl Allpass {
    const fn new() -> Self {
        Self {
            buf: Vec::new(),
            len: 1,
            index: 0,
        }
    }

    fn setup(&mut self, base_len: usize, sr_scale: f32) {
        self.len = ((base_len as f32 * sr_scale) as usize).clamp(32, ALLPASS_MAX - 1);
        if self.buf.len() != self.len {
            self.buf = vec![0.0; self.len];
        } else {
            self.buf.fill(0.0);
        }
        self.index = 0;
    }

    /// 清空这条扩散器的**历史**（缓冲内容与读写头）。长度不动。
    ///
    /// 未配置时 `buf` 是空 `Vec`，`fill` 是空操作（不 panic、不分配）。
    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.index = 0;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        /// 全通反馈系数（Freeverb 的固定值）。
        const FEEDBACK: f32 = 0.5;
        let buffered = self.buf[self.index];
        let out = -input + buffered;
        self.buf[self.index] = input + buffered * FEEDBACK;
        self.index += 1;
        if self.index >= self.len {
            self.index = 0;
        }
        out
    }
}

/// 混响参数：除 `predelay` 外全部归一化到 0..1。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbParams {
    /// 衰减长度：映射到梳状反馈 0.72…0.985。
    pub size: f32,
    /// 尾巴里的高频阻尼。
    pub damp: f32,
    /// 湿/干平衡。
    pub mix: f32,
    /// 湿信号的立体声宽度。
    pub width: f32,
    /// 预延迟（秒，0…0.1）。
    pub predelay: f32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            size: 0.45,
            damp: 0.35,
            mix: 0.25,
            width: 0.8,
            predelay: 0.012,
        }
    }
}

/// Freeverb 拓扑的立体声混响。
pub struct Reverb {
    combs: [[Comb; 8]; 2],
    allpass: [[Allpass; 4]; 2],
    pre: [Vec<f32>; 2],
    pre_len: usize,
    pre_index: usize,
    sample_rate: f32,
    sr_scale: f32,
    params: ReverbParams,
    configured: bool,
}

impl Reverb {
    /// 构造一个**未配置**的实例（直通）。缓冲区在 [`Self::set_sample_rate`] 里分配。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            // 逐项写出，让上层可以把整个混响放进 `static`。
            combs: [
                [
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                ],
                [
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                ],
            ],
            allpass: [
                [
                    Allpass::new(),
                    Allpass::new(),
                    Allpass::new(),
                    Allpass::new(),
                ],
                [
                    Allpass::new(),
                    Allpass::new(),
                    Allpass::new(),
                    Allpass::new(),
                ],
            ],
            pre: [Vec::new(), Vec::new()],
            pre_len: 1,
            pre_index: 0,
            sample_rate: 48_000.0,
            sr_scale: 1.0,
            params: ReverbParams {
                size: 0.45,
                damp: 0.35,
                mix: 0.25,
                width: 0.8,
                predelay: 0.012,
            },
            configured: false,
        }
    }

    /// 按 `sample_rate` 分配全部延迟线 [ARCH-RT-001]。
    ///
    /// **这是本类型唯一会分配的方法**，必须在音频回调之外调用一次
    /// （引擎初始化时）。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sample_rate = crate::math::sanitise_sample_rate(sample_rate);
        self.sample_rate = sample_rate;
        self.sr_scale = (sample_rate / 44_100.0).clamp(0.5, SR_SCALE_MAX);
        self.pre_len = ((self.params.predelay * sample_rate) as usize).clamp(1, PREDELAY_MAX - 1);
        if self.pre[0].len() != PREDELAY_MAX {
            self.pre = [vec![0.0; PREDELAY_MAX], vec![0.0; PREDELAY_MAX]];
        } else {
            self.pre[0].fill(0.0);
            self.pre[1].fill(0.0);
        }
        self.pre_index = 0;
        for (channel, combs) in self.combs.iter_mut().enumerate() {
            let spread = if channel == 1 { STEREO_SPREAD } else { 0 };
            for (index, comb) in combs.iter_mut().enumerate() {
                comb.setup(COMB_TUNING[index] + spread, self.sr_scale);
            }
        }
        for (channel, allpasses) in self.allpass.iter_mut().enumerate() {
            let spread = if channel == 1 { STEREO_SPREAD } else { 0 };
            for (index, allpass) in allpasses.iter_mut().enumerate() {
                allpass.setup(ALLPASS_TUNING[index] + spread, self.sr_scale);
            }
        }
        self.configured = true;
    }

    /// 是否已经过 [`Self::set_sample_rate`]。
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.configured
    }

    /// 设置参数（分配零次；可以逐块调用）。
    ///
    /// 退化输入（`NaN`/`±inf`）会被替换成默认值：它们会把整个梳状组的反馈变成
    /// `NaN` 并静默毒化整条混响总线，而实时路径上无法报错，只能就地回落。
    pub fn set_params(&mut self, params: ReverbParams) {
        /// 非有限值一律回落到这个默认参数集。
        const FALLBACK: ReverbParams = ReverbParams {
            size: 0.45,
            damp: 0.35,
            mix: 0.25,
            width: 0.8,
            predelay: 0.012,
        };
        let sanitise = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        let params = ReverbParams {
            size: sanitise(params.size, FALLBACK.size).clamp(0.0, 1.0),
            damp: sanitise(params.damp, FALLBACK.damp).clamp(0.0, 1.0),
            mix: sanitise(params.mix, FALLBACK.mix).clamp(0.0, 1.0),
            width: sanitise(params.width, FALLBACK.width).clamp(0.0, 1.0),
            predelay: sanitise(params.predelay, 0.0).clamp(0.0, 0.1),
        };
        self.params = params;
        // 0.70…0.94 是稳定区间的映射（0.96 是带调制混响的稳定极限）；
        // 阻尼下限让环路在 DAMP = 0 时仍有损耗。
        let feedback = 0.70 + params.size * 0.24;
        let damp = 0.05 + params.damp * 0.72;
        for combs in self.combs.iter_mut() {
            for comb in combs.iter_mut() {
                comb.feedback = feedback;
                comb.damp = damp;
            }
        }
        if self.configured {
            // 绝不在这里清空预延迟线：参数是平滑的，否则拖一次旋钮就会
            // 在块边界上把尾巴擦掉。
            self.pre_len =
                ((params.predelay * self.sample_rate) as usize).clamp(1, PREDELAY_MAX - 1);
            if self.pre_index >= self.pre_len {
                self.pre_index = 0;
            }
        }
    }

    /// 当前参数。
    #[must_use]
    pub const fn params(&self) -> ReverbParams {
        self.params
    }

    /// 湿信号是否可闻（`mix > 1e-4`）。
    ///
    /// ⚠ 未配置时本方法**仍**按 `mix` 回答（既有判据
    /// `an_unconfigured_reverb_is_a_passthrough` 钉住这条）；它说的是"这个参数
    /// 设置会不会出声"，不是"这台实例现在会不会处理"。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.params.mix > 1e-4
    }

    /// 本器件引入的**处理延迟**（帧），恒为 [`REVERB_LATENCY`] = `0`
    /// [ARCH-PDC-001]。
    ///
    /// 预延迟**不在**这个数里：`process` 里干路用的是**当前**样本 `l` / `r`，
    /// 预延迟线只喂梳状组 ⇒ 整机对输入没有净延迟。判据
    /// `reverb::tests::latency_is_zero_and_the_predelay_delays_only_the_wet_path`。
    ///
    /// 口径与 [`crate::convolution_reverb::ConvolutionReverb::latency_samples`] 相同
    /// （那里也把预延迟排除在 PDC 之外）。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        REVERB_LATENCY
    }

    /// 清空全部延迟线的**历史**：16 条梳状、8 条全通、两条预延迟。
    ///
    /// 契约（由判据
    /// `reverb::tests::reset_reproduces_a_freshly_built_device_bit_for_bit` 钉住）：
    /// `reset()` 之后处理任意块，与一台**刚用同样参数装配好**的实例
    /// （`new` → `set_sample_rate` → `set_params`）逐位相同。
    ///
    /// **配置不动**：采样率、参数、各条线的长度与系数都保持不变 —— 清的只是历史。
    /// 与 [`Self::set_params`] 的取舍不同：那里**故意不清**预延迟线（参数是平滑的，
    /// 块边界上擦掉尾巴会听见），而 `reset()` 是调用方显式要求"从现在起当它没响过"。
    ///
    /// **逐样本零分配**（只对已分配的缓冲做原地写，不经堆），可以在实时线程上调用
    /// [ARCH-RT-001]。未配置时是空操作（空 `Vec` 的 `fill` 不分配也不 panic）。
    pub fn reset(&mut self) {
        for combs in self.combs.iter_mut() {
            for comb in combs.iter_mut() {
                comb.reset();
            }
        }
        for allpasses in self.allpass.iter_mut() {
            for allpass in allpasses.iter_mut() {
                allpass.reset();
            }
        }
        for line in &mut self.pre {
            line.fill(0.0);
        }
        self.pre_index = 0;
    }

    /// 立体声块的原地湿/干混合。
    ///
    /// 未配置时直通；长度取 `left`/`right` 的较短者；全程零分配。
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mix = if self.params.mix.is_finite() {
            self.params.mix.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if !self.configured || self.pre[0].is_empty() || mix <= 1e-4 {
            return;
        }
        let dry = 1.0 - mix;
        let width = if self.params.width.is_finite() {
            self.params.width.clamp(0.0, 1.0)
        } else {
            0.0
        };
        for (left_slot, right_slot) in left.iter_mut().zip(right.iter_mut()) {
            // 入口守卫：一个非有限样本写进预延迟线或梳状组就会**永久**毒化反馈环
            // （`NaN·0.84 = NaN`、`∞·0.84 = ∞`），`reset()` 之外没有出路，
            // 而实时路径上无法报错 ⇒ 就地回落成 `0.0`。有限样本逐位不变，
            // 既有音色与既有读数一个比特都不改。见模块文档 §5。
            let l = finite_or_zero(*left_slot);
            let r = finite_or_zero(*right_slot);

            // 预延迟（普通环形缓冲，两声道等长）。
            let dl = self.pre[0][self.pre_index];
            let dr = self.pre[1][self.pre_index];
            self.pre[0][self.pre_index] = l;
            self.pre[1][self.pre_index] = r;
            self.pre_index += 1;
            if self.pre_index >= self.pre_len {
                self.pre_index = 0;
            }

            // Freeverb 把两个声道求和后送进每个"箱体"（近似单声道输入），
            // 这正是该算法尾巴稳定、密集的来源。
            let input = (dl + dr) * 0.5 * 0.015;
            let mut wet_l = 0.0;
            let mut wet_r = 0.0;
            for comb in self.combs[0].iter_mut() {
                wet_l += comb.process(input);
            }
            for comb in self.combs[1].iter_mut() {
                wet_r += comb.process(input);
            }
            for allpass in self.allpass[0].iter_mut() {
                wet_l = allpass.process(wet_l);
            }
            for allpass in self.allpass[1].iter_mut() {
                wet_r = allpass.process(wet_r);
            }

            // 宽度：0 塌成单声道，1 保持两个箱体完全分离。
            let mid = (wet_l + wet_r) * 0.5 * WET_GAIN;
            let side = (wet_l - wet_r) * 0.5 * width * WET_GAIN;
            let out_l = mid + side;
            let out_r = mid - side;

            *left_slot = l * dry + out_l * mix;
            *right_slot = r * dry + out_r * mix;
        }
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn impulse_response(params: ReverbParams, seconds: f32) -> (Vec<f32>, Vec<f32>, f32) {
        let mut verb = Reverb::new();
        verb.set_sample_rate(SR);
        verb.set_params(params);
        let n = (seconds * SR) as usize;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        l[0] = 1.0;
        r[0] = 1.0;
        verb.process(&mut l, &mut r);
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
        (l, r, peak)
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// 一段**确定性**激励，原地写进一对声道缓冲（同一个 `quantum` 给同一段样本）。
    ///
    /// 没有 RNG 对象、没有分配：本票的两条新判据要给多台实例喂**同一段**输入。
    fn excite(left: &mut [f32], right: &mut [f32], quantum: u64, scale: f32) {
        for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let t = i as f32 * 0.011;
            *l = scale * (quantum as f32 * 0.37 + t).sin();
            *r = scale * (quantum as f32 * 0.29 - t).cos();
        }
    }

    #[test]
    fn impulse_tail_decays_and_stays_bounded() {
        let (l, _, peak) = impulse_response(
            ReverbParams {
                mix: 1.0,
                ..Default::default()
            },
            4.0,
        );
        assert!(peak <= 1.0, "reverb overshot: {peak}");
        let head = rms(&l[4800..9600]); // 0.1–0.2 s
        let tail = rms(&l[l.len() - 4800..]); // 3.9–4.0 s
        assert!(head > 0.0, "no tail at all");
        assert!(tail < head * 0.5, "tail did not decay: {head} -> {tail}");
        assert!(l.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn damping_darkens_the_tail() {
        let bright = impulse_response(
            ReverbParams {
                mix: 1.0,
                damp: 0.0,
                size: 0.8,
                ..Default::default()
            },
            2.0,
        )
        .0;
        let dark = impulse_response(
            ReverbParams {
                mix: 1.0,
                damp: 1.0,
                size: 0.8,
                ..Default::default()
            },
            2.0,
        )
        .0;
        // 平均绝对斜率是廉价的亮度代理：更暗的尾巴相邻样本间移动更少。
        let slope = |buf: &[f32]| {
            buf.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / buf.len() as f32
        };
        assert!(
            slope(&dark[24_000..]) < slope(&bright[24_000..]),
            "damping did not darken the tail"
        );
    }

    #[test]
    fn width_controls_the_stereo_spread() {
        let (l, r, _) = impulse_response(
            ReverbParams {
                mix: 1.0,
                width: 1.0,
                ..Default::default()
            },
            1.0,
        );
        let diff: f32 = l.iter().zip(&r).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 0.01, "stereo tanks are identical");
        let (l0, r0, _) = impulse_response(
            ReverbParams {
                mix: 1.0,
                width: 0.0,
                ..Default::default()
            },
            1.0,
        );
        let diff0: f32 = l0.iter().zip(&r0).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff0 < diff * 0.1, "width 0 should collapse to mono");
    }

    #[test]
    fn pre_delay_holds_the_tail_back() {
        let (early, _, _) = impulse_response(
            ReverbParams {
                mix: 1.0,
                predelay: 0.0,
                size: 0.5,
                ..Default::default()
            },
            1.0,
        );
        let (late, _, _) = impulse_response(
            ReverbParams {
                mix: 1.0,
                predelay: 0.05,
                size: 0.5,
                ..Default::default()
            },
            1.0,
        );
        let first_ms = |buf: &[f32]| {
            buf.iter()
                .position(|s| s.abs() > 1e-4)
                .map(|i| i as f32 / 48.0)
                .unwrap_or(f32::MAX)
        };
        assert!(first_ms(&late) > first_ms(&early) + 20.0);
    }

    #[test]
    fn sustained_tones_do_not_blow_up() {
        // 任何频率的稳定正弦都不得把梳状组泵起来：带调制的反馈延迟是这事
        // 经典出错的地方。
        for freq in [55.0f32, 110.0, 220.0, 440.0, 880.0, 1760.0, 3520.0] {
            let mut verb = Reverb::new();
            verb.set_sample_rate(SR);
            verb.set_params(ReverbParams {
                mix: 1.0,
                size: 1.0,
                damp: 0.0,
                width: 1.0,
                predelay: 0.012,
            });
            let n = 48_000 * 4;
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            for i in 0..n {
                let v = (core::f32::consts::TAU * freq * i as f32 / SR).sin() * 0.5;
                l[i] = v;
                r[i] = v;
            }
            verb.process(&mut l, &mut r);
            let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
            let sec = 48_000;
            let head = rms(&l[sec..sec * 2]);
            let tail = rms(&l[sec * 3..]);
            assert!(peak.is_finite(), "{freq} Hz produced non-finite output");
            assert!(peak < 8.0, "{freq} Hz spiked to {peak}");
            // 稳定音会进入稳态，因此尾巴可能与头部同电平；但它绝不能持续增长。
            assert!(
                tail < head * 1.25,
                "{freq} Hz does not settle: {head} -> {tail}"
            );
        }
    }

    #[test]
    fn long_input_never_blows_up() {
        let mut verb = Reverb::new();
        verb.set_sample_rate(SR);
        verb.set_params(ReverbParams {
            mix: 1.0,
            size: 1.0,
            damp: 0.0,
            ..Default::default()
        });
        let mut l = vec![0.3f32; 48_000];
        let mut r = vec![-0.3f32; 48_000];
        verb.process(&mut l, &mut r);
        assert!(l.iter().all(|s| s.is_finite() && s.abs() < 4.0));
        assert!(r.iter().all(|s| s.is_finite() && s.abs() < 4.0));
    }

    /// **判据（新写，可红）**：预延迟以**秒**计，与采样率无关。
    ///
    /// 观测方式：第一个非零输出出现在"预延迟 + 最短梳延迟"处，而梳延迟本身也被
    /// 采样率缩放，因此绝对值不能直接断言。可断言的是**预延迟带来的那段增量**：
    /// 50 ms 的预延迟必须比 0 ms 的预延迟晚 50 ms 才开始出声，且在 48 kHz 与
    /// 96 kHz 下都成立。
    ///
    /// 把 [`Reverb::set_sample_rate`] 里 `predelay * sample_rate` 换成
    /// `predelay * 48_000.0`，96 kHz 那一半立即变红。
    #[test]
    fn predelay_is_measured_in_seconds_at_any_sample_rate() {
        let first_ms = |sample_rate: f32, predelay: f32| {
            let mut verb = Reverb::new();
            verb.set_sample_rate(sample_rate);
            verb.set_params(ReverbParams {
                mix: 1.0,
                predelay,
                size: 0.5,
                ..Default::default()
            });
            let seconds = 0.5;
            let n = (seconds * sample_rate) as usize;
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            l[0] = 1.0;
            r[0] = 1.0;
            verb.process(&mut l, &mut r);
            let first = l
                .iter()
                .position(|s| s.abs() > 1e-4)
                .expect("reverb produced no output at all");
            first as f32 / sample_rate * 1000.0
        };
        for sample_rate in [48_000.0f32, 96_000.0] {
            let none = first_ms(sample_rate, 0.0);
            let fifty = first_ms(sample_rate, 0.05);
            let added = fifty - none;
            assert!(
                (added - 50.0).abs() < 2.0,
                "at {sample_rate} Hz a 50 ms predelay added {added} ms (0 ms -> {none} ms)"
            );
            // 预延迟 0 时立刻有输出，证明上面测的确实是预延迟而不是别的延迟。
            assert!(
                none < 40.0,
                "the comb bank is late even at predelay 0: {none} ms"
            );
        }
    }

    /// **判据（新写，可红）**：未配置的混响是直通，且不得 panic。
    ///
    /// 来源在从未调用 `set_sample_rate` 时会索引一个空 `Vec` 而 panic。
    /// 若把 `process` 里的 `!self.configured || self.pre[0].is_empty()` 守卫删掉，
    /// 本测试立刻 panic 变红。
    #[test]
    fn an_unconfigured_reverb_is_a_passthrough() {
        let mut verb = Reverb::new();
        assert!(!verb.is_configured());
        assert!(verb.is_active(), "default mix should be audible");
        let mut left = [0.5f32, -0.25, 0.125];
        let mut right = [0.1f32, 0.2, -0.3];
        verb.process(&mut left, &mut right);
        assert_eq!(left, [0.5, -0.25, 0.125]);
        assert_eq!(right, [0.1, 0.2, -0.3]);
        // mix = 0 时即使是已配置实例也必须是逐位直通。
        verb.set_sample_rate(SR);
        verb.set_params(ReverbParams {
            mix: 0.0,
            ..Default::default()
        });
        assert!(!verb.is_active());
        let mut left = [0.5f32, -0.25];
        let mut right = [0.1f32, 0.2];
        verb.process(&mut left, &mut right);
        assert_eq!(left, [0.5, -0.25]);
        assert_eq!(right, [0.1, 0.2]);
        // 长度不等的块不得 panic（只处理较短的部分）。
        verb.set_params(ReverbParams {
            mix: 1.0,
            ..Default::default()
        });
        let mut long = [0.0f32; 8];
        let mut short = [0.0f32; 3];
        verb.process(&mut long, &mut short);
        assert!(long.iter().all(|v| v.is_finite()));
        assert!(short.iter().all(|v| v.is_finite()));
        // 退化参数不得产生 NaN：非有限值回落到默认值，混响仍然必须活下来。
        verb.set_params(ReverbParams {
            size: f32::NAN,
            damp: f32::INFINITY,
            mix: 0.5,
            width: f32::NAN,
            predelay: f32::NAN,
        });
        assert!(verb.is_active());
        let mut left = vec![0.2f32; 4096];
        let mut right = vec![0.2f32; 4096];
        verb.process(&mut left, &mut right);
        assert!(
            left.iter().all(|v| v.is_finite()),
            "degenerate parameters poisoned the reverb bus"
        );
        assert!(right.iter().all(|v| v.is_finite()));
        assert!(
            (verb.params().mix - 0.5).abs() < 1e-6,
            "the finite knob was dropped"
        );
    }

    /// **判据（新写，可红）**：`reset()` 之后逐位等于**刚用同样参数装配好**的实例。
    ///
    /// 量什么：三台同源实例（同样的采样率与参数）在**同一个探测块**上的输出比特。
    /// `used` 先被 8 个块激励（留下尾巴）然后 `reset()`；`fresh` 从未处理过任何输入；
    /// `stale` 与 `used` 经历**完全相同**的输入但**不** `reset()`（反向对照）。
    ///
    /// 判据：① 左/右声道 `used` 的每一个比特等于 `fresh`；② `stale` 与 `fresh`
    /// 至少 `512` 个样本不同 —— 第二条证明这个探测块**测得出**尾巴，因此第一条
    /// 不是"尾巴本来就没有"的假绿。
    ///
    /// 注入：删掉 [`Reverb::reset`] 里梳状与全通的 `reset()` 循环（只留预延迟清零）
    /// ⇒ 断言 ① 立即变红（输出里还留着 8 个块的尾巴）；删掉预延迟清零那一句
    /// 同样变红（预延迟线的残余把 `stale` 的历史带进湿路）。
    #[test]
    fn reset_reproduces_a_freshly_built_device_bit_for_bit() {
        /// 每个块的帧数。`0.02 s` 的预延迟在 48 kHz 下是 `960` 帧，`8 × 4096`
        /// 帧足够让两条预延迟线与最长的梳状线都绕过好几圈。
        const FRAMES: usize = 4_096;
        /// 反向对照的**最小**差异样本数（左声道）。实测远大于它。
        const MIN_STALE_DIFFERENCES: usize = 512;

        let params = ReverbParams {
            size: 0.85,
            damp: 0.2,
            mix: 0.6,
            width: 0.7,
            predelay: 0.02,
        };
        let build = || {
            let mut verb = Reverb::new();
            verb.set_sample_rate(SR);
            verb.set_params(params);
            verb
        };

        let mut used = build();
        let mut stale = build();
        let mut fresh = build();

        // 激励：`used` 与 `stale` 吃**同样**的 8 个块。
        let mut left = vec![0.0f32; FRAMES];
        let mut right = vec![0.0f32; FRAMES];
        for quantum in 0..8 {
            excite(&mut left, &mut right, quantum, 0.5);
            used.process(&mut left, &mut right);
            excite(&mut left, &mut right, quantum, 0.5);
            stale.process(&mut left, &mut right);
        }
        // 覆盖度自检：激励之后尾巴必须还在（否则反向对照没有意义）。
        assert!(
            left.iter().any(|v| v.abs() > 1e-3),
            "激励块没有产生任何输出 ⇒ 本判据测的是空壳"
        );

        used.reset();

        // 探测块：三台处理同一段输入。
        let mut probe_left = vec![0.0f32; FRAMES];
        let mut probe_right = vec![0.0f32; FRAMES];
        excite(&mut probe_left, &mut probe_right, 99, 0.4);

        let mut used_left = probe_left.clone();
        let mut used_right = probe_right.clone();
        used.process(&mut used_left, &mut used_right);

        let mut fresh_left = probe_left.clone();
        let mut fresh_right = probe_right.clone();
        fresh.process(&mut fresh_left, &mut fresh_right);

        let mut stale_left = probe_left.clone();
        let mut stale_right = probe_right.clone();
        stale.process(&mut stale_left, &mut stale_right);

        // ① 正向：逐位相同。
        for (i, (out, want)) in used_left.iter().zip(&fresh_left).enumerate() {
            assert_eq!(
                out.to_bits(),
                want.to_bits(),
                "样本 {i}: reset 之后左声道必须逐位等于新建实例"
            );
        }
        for (i, (out, want)) in used_right.iter().zip(&fresh_right).enumerate() {
            assert_eq!(
                out.to_bits(),
                want.to_bits(),
                "样本 {i}: reset 之后右声道必须逐位等于新建实例"
            );
        }

        // ② 反向对照：不 reset 的实例必须与新建实例**明显**不同。
        let mut differences = 0usize;
        for (out, want) in stale_left.iter().zip(&fresh_left) {
            if out.to_bits() != want.to_bits() {
                differences += 1;
            }
        }
        assert!(
            differences >= MIN_STALE_DIFFERENCES,
            "不 reset 的实例与新建实例只有 {differences}/{FRAMES} 个样本不同 ⇒ \
             这个探测块测不出尾巴，判据 ① 的绿没有判别力"
        );
        // 反向对照在右声道上同样成立（防止判据只在一个声道上有牙）。
        let right_differences = stale_right
            .iter()
            .zip(&fresh_right)
            .filter(|(out, want)| out.to_bits() != want.to_bits())
            .count();
        assert!(
            right_differences >= MIN_STALE_DIFFERENCES,
            "右声道只有 {right_differences}/{FRAMES} 个样本不同 ⇒ 判据只在一个声道上有牙"
        );
        eprintln!(
            "[yeban-dsp] reverb::reset 读数（单位：个样本）: FRAMES={FRAMES} 采样率={SR} Hz \
             预延迟={} 帧 reset 后与新建实例的比特差异=0；不 reset 的对照差异 左={differences} \
             右={right_differences}（门限 {MIN_STALE_DIFFERENCES}）",
            (params.predelay * SR) as usize
        );
    }

    /// **判据（新写，可红）**：延迟上报恒为 `0`，且预延迟**只**推迟湿路。
    ///
    /// 量什么：① [`Reverb::latency_samples`] 的读数（单位：帧）；② 预延迟
    /// `0.0 s` 与 `0.05 s` 下，单位脉冲输出的**第 0 帧**（比特）与湿路首次超过
    /// `1e-4` 的样本下标（单位：帧）。
    ///
    /// 判据：① 读数恒为 `REVERB_LATENCY = 0`；② 两种预延迟下第 0 帧都**恰好**是
    /// `1.0`（`1.0 · dry + 0.0 · mix`，干路用当前样本）；③ `50 ms` 的预延迟把湿路
    /// 的首次可闻输出推后**超过** `2 000` 帧（`50 ms @48 kHz` = `2 400` 帧）
    /// —— ② 与 ③ 一起才说明"干路即时、湿路被推迟"。
    ///
    /// 注入：让预延迟推迟**整个**输入（干路也进延迟线）⇒ 断言 ② 变红；
    /// 把 `set_sample_rate` 里的 `predelay * sample_rate` 写死成 `0.0` ⇒ 断言 ③ 变红。
    #[test]
    fn latency_is_zero_and_the_predelay_delays_only_the_wet_path() {
        /// 观测长度（帧）：`50 ms` 预延迟 ＋ 梳状组的最长首达都装得下。
        const FRAMES: usize = 9_600;
        /// 湿/干平衡。取 `0.5` 而不是接近 `0` 的值：湿路的**首个**到达只有
        /// `脉冲 · 0.015 · WET_GAIN · mix` 量级，`mix` 太小时它落在 `1e-4`
        /// 门限以下，判据测到的就不是"首个到达"而是"积累到可闻"。
        const MIX: f32 = 0.5;

        let probe = |predelay: f32| {
            let mut verb = Reverb::new();
            verb.set_sample_rate(SR);
            verb.set_params(ReverbParams {
                size: 0.5,
                damp: 0.35,
                // 大于 `process` 的 `1e-4` 静音守卫 ⇒ 湿路真的参与运算。
                mix: MIX,
                width: 0.8,
                predelay,
            });
            assert_eq!(verb.latency_samples(), REVERB_LATENCY);
            assert_eq!(verb.latency_samples(), 0, "预延迟不是 PDC 延迟");
            let mut left = vec![0.0f32; FRAMES];
            let mut right = vec![0.0f32; FRAMES];
            left[0] = 1.0;
            right[0] = 1.0;
            verb.process(&mut left, &mut right);
            // 干路即时：第 0 帧 = `1.0 · dry + 0.0 · mix`，湿项在那时还是 0。
            assert_eq!(
                left[0].to_bits(),
                (1.0f32 - MIX).to_bits(),
                "预延迟 {predelay} s: 干路被推迟了（第 0 帧不是干项）"
            );
            assert_eq!(right[0].to_bits(), (1.0f32 - MIX).to_bits());
            // 湿路首次可闻输出（跳过第 0 帧的干项；之后输入恒为 0，剩下的只有湿路）。
            let first = left[1..]
                .iter()
                .position(|s| s.abs() > 1e-4)
                .map(|i| i + 1)
                .expect("湿路一直没有出声");
            (left[0], first)
        };

        let (dry_none, wet_none) = probe(0.0);
        let (dry_fifty, wet_fifty) = probe(0.05);
        assert_eq!(dry_none.to_bits(), dry_fifty.to_bits());
        assert!(
            wet_fifty > wet_none + 2_000,
            "50 ms 的预延迟只把湿路推后了 {} 帧（{wet_none} → {wet_fifty}）",
            wet_fifty - wet_none
        );
        assert!(wet_none < 1_920, "预延迟 0 时湿路来晚了：{wet_none} 帧");
        eprintln!(
            "[yeban-dsp] reverb::latency 读数（单位：帧）: latency_samples()={} \
             第 0 帧干项=0x{:08x}（两种预延迟相同）；湿路首次可闻 预延迟 0 s → {wet_none} 帧、\
             0.05 s → {wet_fifty} 帧，增量 {} 帧",
            REVERB_LATENCY,
            dry_none.to_bits(),
            wet_fifty - wet_none
        );
    }
}
