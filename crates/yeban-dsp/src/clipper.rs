//! 母带软削波器（Clipper）：软膝削波 + 过采样抗混叠。
//!
//! 设计来源：第五份问题报告 C-1（`research_notes/yeban-issues5-20261115`）。
//! 它是**普通插入器件**（与 [`crate::limiter`] 同层），可进任意轨插入链；
//! 母带链只是首个使用点（链序 Strip → Tape → Clipper → Limiter）。
//!
//! ## 算法
//!
//! ```text
//! v = x · 10^(drive_db/20)
//! a = |v|，C = 10^(ceiling_db/20)，S = C · 10^(−knee_db/20)   // 膝起点
//! a ≤ S        ⇒ y = v                                   // 膝以下线性（逐位 = v）
//! S < a，knee>0 ⇒ y = sign(v) · [ S + (C−S)·(1 − exp(−(a−S)/(C−S))) ]
//! S < a，knee=0 ⇒ y = sign(v) · C                        // 硬削极限
//! out = (1−mix)·x + mix·y
//! ```
//!
//! 膝以上曲线在 `a = S` 处值与斜率都与线性段相等（一阶连续），且**渐近**到
//! `C`、永不越过 `C`（`mix = 1` 时）。`drive` 把瞬态推过膝点，crest 因此下降；
//! 削波产生的谐波在过采样域产生、抽取前被抗镜像滤波去掉（见下）。
//!
//! ## 过采样
//!
//! 复用 [`crate::oversample::Oversampler2x`] 级联：1× = 直算，2× = 一级往返，
//! 4× = 两级，8× = 三级。干湿混合在**过采样域**做（干路与湿路同走一条往返，
//! 相位对齐，⛔ 不与未延迟的原干声相混 —— 那会梳状滤波）。
//! 往返延迟与磁带饱和同族：设备作者须把过采样往返延迟计入模型层
//! `latency_samples` 声明 [ARCH-PDC-001]；本器件不替模型层推导它。
//!
//! ## 实时安全
//!
//! 缓冲在 [`Clipper::configure`] 一次性分配（定长块 [`CLIPPER_CHUNK_FRAMES`]
//! 帧分块处理），`process` 内零分配。入口非有限样本回落 `0.0`
//! （过采样滤波器有记忆，NaN 会污染记忆，与 `tape.rs` 同一条理由）。

use crate::math::{db_to_gain, finite_or_zero};
use crate::oversample::Oversampler2x;

/// 分块帧数（`process` 内部按此分块，缓冲在 `configure` 按它分配）。
pub const CLIPPER_CHUNK_FRAMES: usize = 1024;

/// 削波器参数（用户域：dB / 比值 / 倍数）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipperParams {
    /// 输入驱动（dB，0..=24）。缺省 6 dB：典型母带削 crest 的中点量级。
    pub drive_db: f32,
    /// 削波渐近线（dBFS，−6..=0）。缺省 −1 dB，与 limiter 天花板对齐。
    pub ceiling_db: f32,
    /// 软膝宽度（dB，0..=6）。0 = 近硬削；6 = 膝起点在 ceiling 之下 6 dB。
    pub knee_db: f32,
    /// 干湿比（0..=1）。缺省 1（全湿）。
    pub mix: f32,
    /// 过采样倍数（1 / 2 / 4 / 8）。缺省 4。
    pub oversample: f32,
}

impl ClipperParams {
    /// 缺省参数。
    pub const DEFAULT: Self = Self {
        drive_db: 6.0,
        ceiling_db: -1.0,
        knee_db: 3.0,
        mix: 1.0,
        oversample: 4.0,
    };

    /// 缺省参数。
    #[must_use]
    pub const fn new() -> Self {
        Self::DEFAULT
    }

    /// 钳制到合法域；非有限值回落缺省；过采样吸附到最近的 1/2/4/8（对数中点）。
    #[must_use]
    pub fn sanitised(self) -> Self {
        let finite_or = |value: f32, fallback: f32, low: f32, high: f32| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                fallback
            }
        };
        let oversample = if !self.oversample.is_finite() {
            Self::DEFAULT.oversample
        } else if self.oversample < std::f32::consts::SQRT_2 {
            1.0
        } else if self.oversample < 2.0 * std::f32::consts::SQRT_2 {
            2.0
        } else if self.oversample < 4.0 * std::f32::consts::SQRT_2 {
            4.0
        } else {
            8.0
        };
        Self {
            drive_db: finite_or(self.drive_db, Self::DEFAULT.drive_db, 0.0, 24.0),
            ceiling_db: finite_or(self.ceiling_db, Self::DEFAULT.ceiling_db, -6.0, 0.0),
            knee_db: finite_or(self.knee_db, Self::DEFAULT.knee_db, 0.0, 6.0),
            mix: finite_or(self.mix, Self::DEFAULT.mix, 0.0, 1.0),
            oversample,
        }
    }

    /// 过采样级数（0 = 1×，1 = 2×，2 = 4×，3 = 8×）。
    #[must_use]
    pub fn oversample_stages(self) -> usize {
        match self.sanitised().oversample as u32 {
            1 => 0,
            2 => 1,
            4 => 2,
            _ => 3,
        }
    }

    /// 本参数是否**恒等**（`mix == 0` ⇒ 全干直通，运行态不建器件）。
    #[must_use]
    pub fn is_identity(self) -> bool {
        self.sanitised().mix == 0.0
    }
}

impl Default for ClipperParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 纯曲线（无过采样、无干湿）：驱动 + 软膝渐近削波。
///
/// `params` 内部先 `sanitised()`；非有限输入回落 `0.0`。
/// `mix = 1` 时输出幅度严格 `< ceiling` 线性值（渐近，可达但不越）。
#[must_use]
pub fn clipper_shape(params: ClipperParams, x: f32) -> f32 {
    let params = params.sanitised();
    let v = finite_or_zero(x) * db_to_gain(params.drive_db);
    let ceiling = db_to_gain(params.ceiling_db);
    let a = v.abs();
    if a <= 0.0 {
        return 0.0;
    }
    let knee_start = ceiling * db_to_gain(-params.knee_db);
    if a <= knee_start {
        return v;
    }
    if params.knee_db <= 0.0 {
        return ceiling.copysign(v);
    }
    let width = ceiling - knee_start;
    let shaped = knee_start + width * (1.0 - libm::expf(-(a - knee_start) / width));
    shaped.copysign(v)
}

/// 曲线 + 干湿（无过采样）：`oversample = 1` 的 `process` 与本函数逐位一致。
#[must_use]
pub fn clipper_shape_mixed(params: ClipperParams, x: f32) -> f32 {
    let params = params.sanitised();
    let x = finite_or_zero(x);
    let wet = clipper_shape(params, x);
    (1.0 - params.mix) * x + params.mix * wet
}

/// 软削波器（立体声；每声道一组过采样级联状态）。
pub struct Clipper {
    params: ClipperParams,
    stages_l: [Oversampler2x; 3],
    stages_r: [Oversampler2x; 3],
    buf_a: Vec<f32>,
    buf_b: Vec<f32>,
    scratch: Vec<f32>,
    ready: bool,
}

impl Default for Clipper {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipper {
    /// 构造一个**未配置**的实例（直通）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            params: ClipperParams::DEFAULT,
            stages_l: [Oversampler2x::new(); 3],
            stages_r: [Oversampler2x::new(); 3],
            buf_a: Vec::new(),
            buf_b: Vec::new(),
            scratch: Vec::new(),
            ready: false,
        }
    }

    /// 配置（分配缓冲并清状态）。**只能在音频回调之外调用** [ARCH-RT-001]。
    pub fn configure(&mut self, sample_rate: f32) {
        let _ = sample_rate;
        let max_len = 8 * CLIPPER_CHUNK_FRAMES;
        self.buf_a = vec![0.0; max_len];
        self.buf_b = vec![0.0; max_len];
        // scratch 须容下最坏情形：downsample 的 OS_TAPS-1 + 2*4096。
        self.scratch = vec![0.0; 16_384];
        self.reset();
        self.ready = true;
    }

    /// 是否已配置。
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.ready
    }

    /// 写参数（`sanitised()` 后存入；不清时间状态，与 `TapeSaturation::set_params` 同纪律）。
    pub fn set_params(&mut self, params: ClipperParams) {
        self.params = params.sanitised();
    }

    /// 当前参数（已 `sanitised`）。
    #[must_use]
    pub const fn params(&self) -> ClipperParams {
        self.params
    }

    /// 本级是否执行：`mix > 0`（读已 sanitised 的值）。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.params.mix > 0.0
    }

    /// 清空全部时间状态（过采样历史），不碰参数。
    pub fn reset(&mut self) {
        for stage in self.stages_l.iter_mut().chain(self.stages_r.iter_mut()) {
            stage.reset();
        }
    }

    /// 原地处理一个立体声块（取两声道较短者）。未配置时直通。
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.ready {
            return;
        }
        let frames = left.len().min(right.len());
        let mut offset = 0;
        while offset < frames {
            let n = (frames - offset).min(CLIPPER_CHUNK_FRAMES);
            let (l, r) = (
                &mut left[offset..offset + n],
                &mut right[offset..offset + n],
            );
            // 两声道共用 buf/scratch ⇒ 逐声道串行处理（状态各自独立）。
            let Self {
                stages_l,
                stages_r,
                buf_a,
                buf_b,
                scratch,
                params,
                ..
            } = &mut *self;
            process_channel(params, stages_l, buf_a, buf_b, scratch, l);
            process_channel(params, stages_r, buf_a, buf_b, scratch, r);
            offset += n;
        }
    }

    /// 处理单声道（实时插入链的单声道路径用左声道状态组）。
    pub fn process_mono(&mut self, mono: &mut [f32]) {
        if !self.ready {
            return;
        }
        let mut offset = 0;
        while offset < mono.len() {
            let n = (mono.len() - offset).min(CLIPPER_CHUNK_FRAMES);
            let slice = &mut mono[offset..offset + n];
            let Self {
                stages_l,
                buf_a,
                buf_b,
                scratch,
                params,
                ..
            } = &mut *self;
            process_channel(params, stages_l, buf_a, buf_b, scratch, slice);
            offset += n;
        }
    }
}

/// 单声道一小块：过采样往返 + 高率域曲线/干湿。
fn process_channel(
    params: &ClipperParams,
    stages: &mut [Oversampler2x; 3],
    buf_a: &mut [f32],
    buf_b: &mut [f32],
    scratch: &mut [f32],
    data: &mut [f32],
) {
    let params = params.sanitised();
    let stage_count = params.oversample_stages();
    if stage_count == 0 {
        for slot in data.iter_mut() {
            *slot = clipper_shape_mixed(params, *slot);
        }
        return;
    }
    let n = data.len();
    buf_a[..n].copy_from_slice(data);
    let mut cur = n;
    // 上采样级联：a → b → a …（用一对缓冲乒乓；每级后交换角色靠 copy 回 a 简化）
    for stage in stages.iter_mut().take(stage_count) {
        stage.upsample(&buf_a[..cur], &mut buf_b[..cur * 2], scratch);
        buf_a[..cur * 2].copy_from_slice(&buf_b[..cur * 2]);
        cur *= 2;
    }
    for slot in buf_a[..cur].iter_mut() {
        *slot = clipper_shape_mixed(params, *slot);
    }
    for stage in stages.iter_mut().take(stage_count).rev() {
        stage.downsample(&buf_a[..cur], &mut buf_b[..cur / 2], scratch);
        buf_a[..cur / 2].copy_from_slice(&buf_b[..cur / 2]);
        cur /= 2;
    }
    debug_assert_eq!(cur, n);
    data.copy_from_slice(&buf_a[..n]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(params: ClipperParams) -> Clipper {
        let mut c = Clipper::new();
        c.configure(48_000.0);
        c.set_params(params);
        c
    }

    fn mag(data: &[f32], freq: f32, rate: f32) -> f32 {
        let k = core::f32::consts::TAU * freq / rate;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, &x) in data.iter().enumerate() {
            re += x * (k * i as f32).cos();
            im -= x * (k * i as f32).sin();
        }
        (re * re + im * im).sqrt() / data.len() as f32
    }

    /// 膝以下线性：drive 0、输入在膝起点之下 ⇒ 输出逐位等于输入（1× 路径）。
    #[test]
    fn below_the_knee_is_linear() {
        let params = ClipperParams {
            drive_db: 0.0,
            ceiling_db: 0.0,
            knee_db: 6.0,
            mix: 1.0,
            oversample: 1.0,
        };
        // 膝起点 = 1.0 · 10^(−6/20) ≈ 0.501；0.25 在其下。
        assert_eq!(clipper_shape(params, 0.25), 0.25);
        assert_eq!(clipper_shape(params, -0.25), -0.25);
        let mut c = configured(params);
        let (mut l, mut r) = (vec![0.25f32; 64], vec![-0.25f32; 64]);
        // 断言判据的规模界：块定长 64（`.all(...)` 的根是 `l` / `r`）。
        assert_eq!(l.len(), 64);
        assert_eq!(r.len(), 64);
        c.process(&mut l, &mut r);
        assert!(l.iter().all(|v| *v == 0.25));
        assert!(r.iter().all(|v| *v == -0.25));
    }

    /// 渐近 ceiling：任意大输入输出严格不超过 ceiling 线性值，且足够接近。
    #[test]
    fn the_curve_asymptotes_to_the_ceiling_without_crossing_it() {
        let params = ClipperParams::DEFAULT; // ceiling −1 dB
        let ceiling = db_to_gain(-1.0);
        for x in [0.9f32, 1.0, 2.0, 10.0, 100.0, 1.0e4] {
            let y = clipper_shape(params, x);
            assert!(y <= ceiling, "输入 {x} 输出 {y} 越过 ceiling {ceiling}");
            assert!(y >= -ceiling);
            let y_neg = clipper_shape(params, -x);
            assert!(y_neg >= -ceiling && y_neg <= ceiling);
        }
        let y = clipper_shape(params, 100.0);
        assert!(y > ceiling * 0.99, "大输入应贴近渐近线：{y} vs {ceiling}");
    }

    /// 五参数边界钳制 + 非有限回落 + 过采样吸附。
    #[test]
    fn params_are_clamped_to_their_domains() {
        let p = ClipperParams {
            drive_db: 99.0,
            ceiling_db: 3.0,
            knee_db: -2.0,
            mix: 2.0,
            oversample: 3.0,
        }
        .sanitised();
        assert_eq!(p.drive_db, 24.0);
        assert_eq!(p.ceiling_db, 0.0);
        assert_eq!(p.knee_db, 0.0);
        assert_eq!(p.mix, 1.0);
        assert_eq!(p.oversample, 4.0, "3 吸附到 4（对数中点 2.828 之上）");
        let p = ClipperParams {
            drive_db: -5.0,
            ceiling_db: -99.0,
            knee_db: 99.0,
            mix: -1.0,
            oversample: 1.2,
        }
        .sanitised();
        assert_eq!(p.drive_db, 0.0);
        assert_eq!(p.ceiling_db, -6.0);
        assert_eq!(p.knee_db, 6.0);
        assert_eq!(p.mix, 0.0);
        assert_eq!(p.oversample, 1.0);
        let p = ClipperParams {
            drive_db: f32::NAN,
            ceiling_db: f32::INFINITY,
            knee_db: f32::NAN,
            mix: f32::NAN,
            oversample: f32::NAN,
        }
        .sanitised();
        assert_eq!(p, ClipperParams::DEFAULT);
        assert_eq!(ClipperParams::DEFAULT.oversample_stages(), 2);
    }

    /// mix = 0 全干直通；mix = 0.5 时输出在干与湿之间。
    #[test]
    fn mix_zero_is_passthrough_and_half_mix_blends() {
        let dry = ClipperParams {
            mix: 0.0,
            oversample: 1.0,
            ..ClipperParams::DEFAULT
        };
        assert!(dry.is_identity());
        assert_eq!(clipper_shape_mixed(dry, 0.8), 0.8);
        let half = ClipperParams {
            mix: 0.5,
            oversample: 1.0,
            ..ClipperParams::DEFAULT
        };
        let wet = clipper_shape(half, 0.8);
        let blended = clipper_shape_mixed(half, 0.8);
        assert!((blended - (0.8 + wet) * 0.5).abs() < 1.0e-6);
    }

    /// 过采样抗混叠判据：19 kHz 正弦被强驱动硬削时，三次谐波 57 kHz 在
    /// 48 kHz 基带折回 9 kHz。4× 路径的 9 kHz 折回分量须显著低于 1× 路径。
    #[test]
    fn oversampling_reduces_aliasing_of_clipped_harmonics() {
        let rate = 48_000.0f32;
        let len = 8192usize;
        let input: Vec<f32> = (0..len)
            .map(|i| 0.9 * (core::f32::consts::TAU * 19_000.0 * i as f32 / rate).sin())
            .collect();
        let run = |oversample: f32| -> Vec<f32> {
            let params = ClipperParams {
                drive_db: 12.0,
                ceiling_db: -1.0,
                knee_db: 0.0,
                mix: 1.0,
                oversample,
            };
            let mut c = configured(params);
            let mut l = input.clone();
            let mut r = input.clone();
            // 分多块喂入让过采样历史稳定，取后半段量折回分量。
            c.process(&mut l, &mut r);
            l[len / 2..].to_vec()
        };
        let alias_1x = mag(&run(1.0), 9_000.0, rate);
        let alias_4x = mag(&run(4.0), 9_000.0, rate);
        assert!(alias_1x > 0.01, "1× 夹具必须真的产生折回分量：{alias_1x}");
        assert!(
            alias_4x < alias_1x * 0.5,
            "4× 折回分量 {alias_4x} 应显著低于 1× 的 {alias_1x}"
        );
    }

    /// 过采样往返不毁低频小信号：1 kHz、膝以下幅度，过 4× 后 RMS 接近原值。
    #[test]
    fn oversampled_path_preserves_a_below_knee_tone() {
        let rate = 48_000.0f32;
        let len = 8192usize;
        let input: Vec<f32> = (0..len)
            .map(|i| 0.2 * (core::f32::consts::TAU * 1_000.0 * i as f32 / rate).sin())
            .collect();
        let params = ClipperParams {
            drive_db: 0.0,
            oversample: 4.0,
            ..ClipperParams::DEFAULT
        };
        let mut c = configured(params);
        let mut l = input.clone();
        let mut r = input.clone();
        c.process(&mut l, &mut r);
        let rms = |d: &[f32]| (d.iter().map(|v| v * v).sum::<f32>() / d.len() as f32).sqrt();
        let ratio = rms(&l[len / 2..]) / rms(&input[len / 2..]);
        assert!(
            (ratio - 1.0).abs() < 0.05,
            "4× 往返后 1 kHz 小信号 RMS 比 {ratio}"
        );
    }

    /// 复位后与全新实例逐位相同；未配置直通；非有限输入不污染输出。
    #[test]
    fn reset_reproduces_a_fresh_device_and_non_finite_input_is_safe() {
        let params = ClipperParams::DEFAULT;
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.37).sin() * 0.9).collect();
        let mut used = configured(params);
        let (mut l, mut r) = (input.clone(), input.clone());
        used.process(&mut l, &mut r);
        used.reset();
        let (mut l1, mut r1) = (input.clone(), input.clone());
        used.process(&mut l1, &mut r1);
        let mut fresh = configured(params);
        let (mut l2, mut r2) = (input.clone(), input.clone());
        fresh.process(&mut l2, &mut r2);
        assert_eq!(l1, l2);
        assert_eq!(r1, r2);

        let mut unconfigured = Clipper::new();
        let (mut l, mut r) = (vec![0.5f32; 8], vec![0.5f32; 8]);
        // 断言判据的规模界：块定长 8（`.all(...)` 的根是 `l`）。
        assert_eq!(l.len(), 8);
        unconfigured.process(&mut l, &mut r);
        assert!(l.iter().all(|v| *v == 0.5));

        let mut c = configured(ClipperParams {
            oversample: 1.0,
            ..ClipperParams::DEFAULT
        });
        let (mut l, mut r) = (vec![f32::NAN, f32::INFINITY, 0.1], vec![0.0; 3]);
        c.process(&mut l, &mut r);
        assert!(l.iter().all(|v| v.is_finite()));
    }
}
