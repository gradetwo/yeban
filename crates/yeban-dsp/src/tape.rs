//! 磁带饱和（链级饱和器；设计文档 `docs/adr/ADR-0061`，状态 `Proposed`）。
//!
//! ## 为什么是链级
//!
//! `drive` / `filter_drive` 是 `InternalInstrument` 的滤波参数，由引擎音源投影消费，
//! 采样器轨不走该投影 ⇒ `drive` 对采样器轨无效（Nightfall 采样版 README 的缺口）。
//! 本级跑在插入链里（通道条之后、Freeverb 之前），所有源（波表、采样、音频片段）
//! 都经过它 ⇒ 饱和覆盖采样器轨。
//!
//! ## 信号模型
//!
//! ```text
//! y_sat = pade_tanh(g · x) / g ，其中 g = 1 + 9 · drive
//! y     = (1 - mix) · x + mix · y_sat ，再乘 tape_trim_db 的线性增益
//! ```
//!
//! - 曲线复用 [`crate::math::pade_tanh`]（与滤波驱动同款，不另发明；同一事实只写一处）。
//! - 小信号增益 ≈ 1：`drive` 只改谐波，不改电平（判据钉住）。
//! - 湿路径先过一阶低通（`tape_hf_hz`，模拟磁带高频损失），干路径不滚降；
//!   `tape_hf_hz ≥ 20000` 时滤波器旁通（恒等）。
//! - 2x 过采样往返**固定**（[`crate::oversample::Oversampler2x`]，v1 不设开关）：
//!   饱和谐波在 2x 域产生、下采样低通滤掉 Nyquist 之外的部分 ⇒ 混叠低于
//!   无过采样的直算（判据钉住方向，不编门限数字）。
//! - 磁带磁滞模型 v1 不做（列 v2）。
//!
//! ## 级建成条件（与运行态同口径）
//!
//! `tape_drive > 0` **且** `tape_mix > 0`（两者任一为缺省 ⇒ 不建器件，
//! 不另设 `tape_enabled` 开关 —— 沿压缩 `#303` 口径“写了非缺省参数 ⇒ 级执行”）。
//! [`TapeSaturation::is_active`] 就是这条判定（读的是 `sanitised()` 后的值）。
//!
//! ## 延迟与 PDC
//!
//! 本级执行时引入 [`crate::oversample::OS_LATENCY`]（31 样本 @ 任何采样率，
//! 单位是基础采样率样本）⇒ 设备作者须把该值计入设备的
//! `latency_samples` 声明（[`TapeSaturation::latency_samples`] 即此数；
//! 未武装时级不执行、延迟为 0）。模型层 `latency_samples` 是唯一事实源
//! [ARCH-PDC-001]，本 crate 不替模型层推导它。
//!
//! ## 实时安全
//!
//! - 全部状态是定长数组（过采样历史、滤波器状态），构造期确定；
//!   [`TapeSaturation::process`] 的暂存（上采样缓冲、下采样暂存）**由调用方提供**
//!   （[`Oversampler2x::process_round_trip`] 同款）⇒ 逐块零分配、零锁、零 I/O。
//! - 入口守卫：非有限样本就地回落成 `0.0`（过采样滤波器有记忆，`NaN` 会污染记忆，
//!   与 `delay.rs` 同一条理由）；有限样本逐位不变。
//! - `configure` 是**唯一**的状态初始化入口（采样率＋清零），必须在音频回调之外
//!   调用一次；未 `configure` 时 [`TapeSaturation::process`] 是直通（不改变缓冲）。

use crate::math::{db_to_gain, finite_or_zero, pade_tanh, sanitise_sample_rate};
use crate::oversample::{OS_LATENCY, OS_TAPS, Oversampler2x};

/// `tape_trim_db` 的合法域（dB）。
pub const TAPE_TRIM_MIN_DB: f32 = -24.0;
/// `tape_trim_db` 的合法域（dB）。
pub const TAPE_TRIM_MAX_DB: f32 = 24.0;
/// `tape_hf_hz` 的合法域（Hz）。
pub const TAPE_HF_MIN_HZ: f32 = 2000.0;
/// `tape_hf_hz` 的合法域（Hz）；≥ 该值时湿路低通旁通（恒等）。
pub const TAPE_HF_MAX_HZ: f32 = 20_000.0;

/// 磁带饱和参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapeParams {
    /// 饱和驱动（0..1）。`0` = 不饱和（级不建，见模块文档）。
    pub drive: f32,
    /// 干湿比（0..1）。`0` = 全干（级不建，见模块文档）。
    pub mix: f32,
    /// 输出补偿增益（dB，−24..+24）。
    pub trim_db: f32,
    /// 湿路低通截止频率（Hz，2000..20000；≥ 20000 旁通）。
    pub hf_hz: f32,
}

impl TapeParams {
    /// 缺省：全干、无驱动、无补偿、湿路低通旁通 ⇒ 级不建（恒等）。
    pub const DEFAULT: Self = Self {
        drive: 0.0,
        mix: 0.0,
        trim_db: 0.0,
        hf_hz: TAPE_HF_MAX_HZ,
    };

    /// 缺省参数。
    #[must_use]
    pub const fn new() -> Self {
        Self::DEFAULT
    }

    /// 钳制表（与 [`TapeSaturation::set_params`] 同一次；判据钉死）。
    ///
    /// | 字段 | 合法域 | 低于下界 | 高于上界 | 非有限 |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | `drive` / `mix` | `[0, 1]` | `0` | `1` | **`0`**（下界） |
    /// | `trim_db` | `[−24, +24]` | 下界 | 上界 | **下界** |
    /// | `hf_hz` | `[2000, 20000]` | 下界 | 上界 | **下界** |
    ///
    /// 非有限一律归下界（沿 [`crate::compressor::CompressorParams::sanitised`] 的
    /// 口径：`f32::clamp` 对 `NaN` 是恒等，单靠 `clamp` 会漏一个 `NaN` 进实时路径）。
    #[must_use]
    pub fn sanitised(self) -> Self {
        let finite_or = |value: f32, low: f32, high: f32| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                low
            }
        };
        Self {
            drive: finite_or(self.drive, 0.0, 1.0),
            mix: finite_or(self.mix, 0.0, 1.0),
            trim_db: finite_or(self.trim_db, TAPE_TRIM_MIN_DB, TAPE_TRIM_MAX_DB),
            hf_hz: finite_or(self.hf_hz, TAPE_HF_MIN_HZ, TAPE_HF_MAX_HZ),
        }
    }
}

impl Default for TapeParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 磁带饱和器（立体声，两声道各一台过采样器＋一阶湿路低通）。
///
/// 构造期不分配（状态全是定长数组）；`process` 的暂存由调用方提供。
pub struct TapeSaturation {
    /// 已 `sanitised` 的参数（`set_params` 的唯一入口保证）。
    params: TapeParams,
    oversampler_l: Oversampler2x,
    oversampler_r: Oversampler2x,
    /// 湿路一阶低通状态，每声道一个。
    lp_state: [f32; 2],
    /// 湿路低通系数（`1 − exp(−2π·fc/fs)`）；旁通时不用它。
    lp_coeff: f32,
    /// 湿路低通是否旁通（`hf_hz ≥ 20000`）。
    wet_bypass: bool,
    sample_rate: f32,
    ready: bool,
}

impl TapeSaturation {
    /// 构造一个**未配置**的实例（直通）。采样率在 [`Self::configure`] 里定。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            params: TapeParams::DEFAULT,
            oversampler_l: Oversampler2x::new(),
            oversampler_r: Oversampler2x::new(),
            lp_state: [0.0; 2],
            lp_coeff: 0.0,
            wet_bypass: true,
            sample_rate: 0.0,
            ready: false,
        }
    }

    /// 按 `sample_rate` 定采样率并清零全部时间状态（过采样历史、滤波器状态）。
    ///
    /// **这是本类型唯一的状态初始化入口**，必须在音频回调之外调用一次。
    pub fn configure(&mut self, sample_rate: f32) {
        self.sample_rate = sanitise_sample_rate(sample_rate);
        self.reset();
        self.refresh_filter();
        self.ready = true;
    }

    /// 是否已经过 [`Self::configure`]。
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.ready
    }

    /// 写参数（`sanitised()` 后存入，并重算湿路低通系数；**不清时间状态**，
    /// 与 `Reverb::set_params` / `ChannelStrip::set_params` 同一条纪律 ⇒
    /// 块边界自动化改参数不会切断饱和尾巴）。
    pub fn set_params(&mut self, params: TapeParams) {
        self.params = params.sanitised();
        self.refresh_filter();
    }

    /// 当前参数（已 `sanitised`）。
    #[must_use]
    pub const fn params(&self) -> TapeParams {
        self.params
    }

    /// 本级是否执行：`drive > 0` 且 `mix > 0`（读的是已 `sanitised` 的值）。
    ///
    /// 任一为缺省 ⇒ 运行态不建本级（恒等），与插入链的建成条件同一条判定。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.params.drive > 0.0 && self.params.mix > 0.0
    }

    /// 本级执行时引入的处理延迟（帧），恒为 [`OS_LATENCY`] [ARCH-PDC-001]。
    ///
    /// 未武装时级不执行、延迟为 0；设备作者按“武装时声明此数”的口径把该值
    /// 计入设备的 `latency_samples`（见模块文档“延迟与 PDC”）。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        OS_LATENCY
    }

    /// 清空全部时间状态（过采样历史、滤波器状态），不碰参数与采样率。
    ///
    /// 复位后的实例与一台刚 [`Self::configure`] 过、同参数的实例在逐帧输出上
    /// **逐位相同**（判据 `reset_reproduces_a_freshly_built_device_bit_for_bit`）。
    pub fn reset(&mut self) {
        self.oversampler_l.reset();
        self.oversampler_r.reset();
        self.lp_state = [0.0; 2];
    }

    /// 按当前参数重算湿路低通系数与旁通位（`configure` / `set_params` 共用）。
    fn refresh_filter(&mut self) {
        self.wet_bypass = self.params.hf_hz >= TAPE_HF_MAX_HZ;
        if self.sample_rate > 0.0 {
            self.lp_coeff = 1.0
                - libm::expf(-2.0 * core::f32::consts::PI * self.params.hf_hz / self.sample_rate);
        }
    }

    /// 对一个声道的上采样缓冲原地做饱和（湿路低通＋`pade_tanh`＋干湿＋补偿）。
    ///
    /// 入口守卫：非有限样本先回落成 `0.0`（再进滤波器记忆与过采样历史）。
    fn saturate_channel(&mut self, channel: usize, samples: &mut [f32]) {
        let g = 1.0 + 9.0 * self.params.drive;
        let trim = db_to_gain(self.params.trim_db);
        let mix = self.params.mix;
        let coeff = self.lp_coeff;
        let bypass = self.wet_bypass;
        let state = &mut self.lp_state[channel];
        for slot in samples.iter_mut() {
            let x = finite_or_zero(*slot);
            let wet = if bypass {
                x
            } else {
                *state += (x - *state) * coeff;
                *state
            };
            let y_sat = pade_tanh(g * wet) / g;
            *slot = ((1.0 - mix) * x + mix * y_sat) * trim;
        }
    }

    /// 原地处理一个立体声块（`left.len() == right.len()`，取两者较短者）。
    ///
    /// 暂存由调用方提供（[`Oversampler2x`] 的口径）：`upsampled` 至少 `2 * n` 长，
    /// `scratch` 至少 `OS_TAPS − 1 + 2 * n` 长（`n` = 块帧数）；长度不足时**什么都不做**
    /// （实时路径上宁可静默跳过也不可能展开栈，与 `Oversampler2x` 同一条纪律）。
    /// 未 [`Self::configure`] 时是直通（不改变缓冲）。
    ///
    /// 入口归零：先把 `left[..n]` / `right[..n]` 里的非有限样本就地置 0，**再**进
    /// 过采样——滤波器历史（`up_tail` / `down_tail`）永远见不到 NaN/Inf，否则脏块
    /// 之后的首个干净块前约 `OS_LATENCY` 帧会被错误地压成 0（上采样先于饱和，
    /// 饱和处的守卫拦不住已经进历史的 NaN）。饱和处的 `finite_or_zero` 仍保留，
    /// 作第二层（防上采样溢出产生的非有限值）。
    ///
    /// 调用方缓冲区即工作缓冲，归零后不再恢复——离线链与实时池传进来的都是
    /// 自有缓冲，不是工程持久数据。
    pub fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        upsampled: &mut [f32],
        scratch: &mut [f32],
    ) {
        if !self.ready {
            return;
        }
        let n = left.len().min(right.len());
        if n == 0 || upsampled.len() < n * 2 || scratch.len() < OS_TAPS - 1 + n * 2 {
            return;
        }
        for sample in &mut left[..n] {
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
        for sample in &mut right[..n] {
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
        self.oversampler_l
            .upsample(&left[..n], &mut upsampled[..n * 2], scratch);
        self.saturate_channel(0, &mut upsampled[..n * 2]);
        self.oversampler_l
            .downsample(&upsampled[..n * 2], &mut left[..n], scratch);
        self.oversampler_r
            .upsample(&right[..n], &mut upsampled[..n * 2], scratch);
        self.saturate_channel(1, &mut upsampled[..n * 2]);
        self.oversampler_r
            .downsample(&upsampled[..n * 2], &mut right[..n], scratch);
    }
}

impl Default for TapeSaturation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn make_tape(drive: f32, mix: f32, trim_db: f32, hf_hz: f32) -> TapeSaturation {
        let mut tape = TapeSaturation::new();
        tape.configure(SR);
        tape.set_params(TapeParams {
            drive,
            mix,
            trim_db,
            hf_hz,
        });
        tape
    }

    /// 块暂存：上采样缓冲＋下采样暂存（`process` 的口径）。
    fn scratch_for(n: usize) -> (Vec<f32>, Vec<f32>) {
        (vec![0.0; n * 2], vec![0.0; OS_TAPS - 1 + n * 2])
    }

    fn process_all(tape: &mut TapeSaturation, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let (mut upsampled, mut scratch) = scratch_for(n);
        tape.process(left, right, &mut upsampled, &mut scratch);
    }

    #[test]
    fn sanitised_clamps_every_field_into_its_documented_domain() {
        for value in [
            f32::NAN,
            f32::NEG_INFINITY,
            f32::INFINITY,
            -1.0,
            0.0,
            0.5,
            1.0,
            2.0,
        ] {
            let params = TapeParams {
                drive: value,
                mix: value,
                trim_db: value,
                hf_hz: value,
            }
            .sanitised();
            assert!((0.0..=1.0).contains(&params.drive), "drive {value}");
            assert!((0.0..=1.0).contains(&params.mix), "mix {value}");
            assert!(
                (TAPE_TRIM_MIN_DB..=TAPE_TRIM_MAX_DB).contains(&params.trim_db),
                "trim_db {value}"
            );
            assert!(
                (TAPE_HF_MIN_HZ..=TAPE_HF_MAX_HZ).contains(&params.hf_hz),
                "hf_hz {value}"
            );
        }
        // 非有限一律归下界（不是上界、不是 NaN 透传）。
        let nan = TapeParams {
            drive: f32::NAN,
            mix: f32::NAN,
            trim_db: f32::NAN,
            hf_hz: f32::NAN,
        }
        .sanitised();
        assert_eq!(nan.drive, 0.0);
        assert_eq!(nan.mix, 0.0);
        assert_eq!(nan.trim_db, TAPE_TRIM_MIN_DB);
        assert_eq!(nan.hf_hz, TAPE_HF_MIN_HZ);
    }

    #[test]
    fn small_signal_gain_is_approximately_one() {
        // 小信号处 pade_tanh(g·x)/g ≈ x（与 drive 无关）：drive 只改谐波，不改电平。
        // 跳过级联 FIR 的建立段（约 2 × OS_LATENCY 帧），只看稳态。
        for drive in [0.0, 0.5, 1.0] {
            let mut tape = make_tape(drive, 1.0, 0.0, TAPE_HF_MAX_HZ);
            let mut left = vec![0.01; 512];
            let mut right = vec![0.01; 512];
            process_all(&mut tape, &mut left, &mut right);
            for sample in &left[128..] {
                assert!(
                    (*sample - 0.01).abs() < 1e-3,
                    "drive {drive}: 小信号增益偏离 0.01，实读 {sample}"
                );
            }
        }
    }

    #[test]
    fn mix_zero_is_identity_within_float_tolerance() {
        // mix = 0 ⇒ 干路直通（往返延迟 OS_LATENCY 仍在，故跳过建立段；
        // FIR 直流增益的浮点误差在 1e-5 内，不按位比）。
        let mut tape = make_tape(0.8, 0.0, 0.0, TAPE_HF_MAX_HZ);
        let mut left = vec![0.7; 512];
        let mut right = vec![0.7; 512];
        process_all(&mut tape, &mut left, &mut right);
        for sample in &left[128..] {
            assert!(
                (*sample - 0.7).abs() < 1e-5,
                "mix = 0 时稳态必须与输入相等，实读 {sample}"
            );
        }
    }

    #[test]
    fn inactive_params_report_not_active() {
        assert!(!make_tape(0.0, 0.5, 0.0, TAPE_HF_MAX_HZ).is_active());
        assert!(!make_tape(0.5, 0.0, 0.0, TAPE_HF_MAX_HZ).is_active());
        assert!(!make_tape(0.0, 0.0, 0.0, TAPE_HF_MAX_HZ).is_active());
        assert!(make_tape(0.5, 0.5, 0.0, TAPE_HF_MAX_HZ).is_active());
        // 负值经 sanitised 归 0 ⇒ 不激活（不是“负驱动”）。
        assert!(!make_tape(-1.0, 0.5, 0.0, TAPE_HF_MAX_HZ).is_active());
    }

    #[test]
    fn latency_samples_is_the_oversampler_round_trip() {
        assert_eq!(TapeSaturation::new().latency_samples(), OS_LATENCY);
        assert_eq!(OS_LATENCY, 31);
    }

    #[test]
    fn reset_reproduces_a_freshly_built_device_bit_for_bit() {
        let mut first = make_tape(0.6, 0.8, -3.0, 8000.0);
        let mut second = make_tape(0.6, 0.8, -3.0, 8000.0);
        let mut left: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.05).sin() * 0.7).collect();
        let mut right = left.clone();
        let mut left2 = left.clone();
        let mut right2 = right.clone();
        process_all(&mut first, &mut left, &mut right);
        // second 先跑一段别的音频、再复位 ⇒ 与刚建好的 first 逐位相同。
        let mut noise = vec![0.9; 512];
        let mut noise_r = vec![-0.9; 512];
        process_all(&mut second, &mut noise, &mut noise_r);
        second.reset();
        process_all(&mut second, &mut left2, &mut right2);
        assert_eq!(left, left2);
        assert_eq!(right, right2);
    }

    #[test]
    fn non_finite_input_falls_back_to_zero_and_does_not_poison_state() {
        let mut tape = make_tape(0.7, 1.0, 0.0, TAPE_HF_MAX_HZ);
        let mut fresh = make_tape(0.7, 1.0, 0.0, TAPE_HF_MAX_HZ);
        let mut dirty = vec![f32::NAN; 256];
        let mut dirty_r = vec![f32::INFINITY; 256];
        process_all(&mut tape, &mut dirty, &mut dirty_r);
        assert!(
            dirty.iter().chain(dirty_r.iter()).all(|s| s.is_finite()),
            "非有限输入不得透传出非有限输出"
        );
        // 脏块写进历史的全是 0 ⇒ 随后干净块的输出与全新实例逐位相同。
        let mut clean: Vec<f32> = (0..512).map(|i| (i as f32 * 0.03).sin() * 0.6).collect();
        let mut clean_r = clean.clone();
        let mut clean2 = clean.clone();
        let mut clean2_r = clean_r.clone();
        process_all(&mut tape, &mut clean, &mut clean_r);
        process_all(&mut fresh, &mut clean2, &mut clean2_r);
        assert_eq!(clean, clean2);
        assert_eq!(clean_r, clean2_r);
    }

    #[test]
    fn impulse_peak_lands_on_the_round_trip_latency() {
        // 小冲激（近线性区）⇒ 级联对称 FIR 的群延迟峰落在 OS_LATENCY。
        let mut tape = make_tape(0.5, 1.0, 0.0, TAPE_HF_MAX_HZ);
        let n = 512;
        let mut left = vec![0.0; n];
        let mut right = vec![0.0; n];
        left[0] = 0.1;
        process_all(&mut tape, &mut left, &mut right);
        let peak = left
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(peak, OS_LATENCY, "冲激峰必须落在往返延迟处，实读 {peak}");
        assert!(right.iter().all(|s| *s == 0.0), "右声道无输入 ⇒ 无串音");
    }

    /// 单点 DFT 幅值（`bin` 必须是整周期点，避免泄漏进断言）。
    fn dft_magnitude(samples: &[f32], bin: usize) -> f32 {
        let n = samples.len() as f32;
        let mut re = 0.0f32;
        let mut im = 0.0f32;
        for (i, sample) in samples.iter().enumerate() {
            let angle = 2.0 * core::f32::consts::PI * bin as f32 * i as f32 / n;
            re += *sample * libm::cosf(angle);
            im -= *sample * libm::sinf(angle);
        }
        (re * re + im * im).sqrt() / n
    }

    #[test]
    fn oversampling_aliasing_is_below_the_direct_formula() {
        // 2x 过采样把“3 次谐波折叠”这件事推迟了一个倍频程：基频 10044.6 Hz
        // 的 3 次谐波 30133.9 Hz 在基带采样率下折到 17866.1 Hz（1334 bin），
        // 在 2x 域里它低于 2x 奈奎斯特（48 kHz）且落在下采样低通的阻带
        // （OS_H 在 30 kHz 处 |H| ≈ 0，实测）⇒ 器件输出的该 bin 必须远低于直算。
        //
        // 频率选择是算过的：窗口 3584 帧、bin 宽 48000/3584 Hz；基频取 750 bin
        // （10044.6 Hz），3 次谐波在两域都只落 1334 bin 一处（5/7/9/11 次分别落
        // 166/1666/418/…bin，不与 1334 碰撞 ⇒ 单 bin 比较无抵消混杂）。
        // 18 kHz 不能作此比较：它的 5 次谐波 90 kHz 在 2x 域里已 alias 成 6 kHz
        // 通带分量（实测 alias_2x > alias_1x），那是 2x 的固有上限，不是器件缺陷。
        // 加窗：跳过前 512 帧（含级联 FIR 建立段），剩余 3584 帧仍是整周期。
        const N: usize = 4096;
        const SKIP: usize = 512;
        const F0_BIN: usize = 750; // 10044.6 Hz @ 48 kHz, 窗口 3584
        const THIRD_BIN: usize = 1334; // 17866.1 Hz（3 次谐波的折叠位置）
        let input: Vec<f32> = (0..N)
            .map(|i| {
                0.9 * libm::sinf(2.0 * core::f32::consts::PI * F0_BIN as f32 * i as f32 / 3584.0)
            })
            .collect();
        let mut tape = make_tape(1.0, 1.0, 0.0, TAPE_HF_MAX_HZ);
        let mut left = input.clone();
        let mut right = vec![0.0; N];
        process_all(&mut tape, &mut left, &mut right);
        let third_2x = dft_magnitude(&left[SKIP..], THIRD_BIN);
        let direct: Vec<f32> = input
            .iter()
            .map(|x| pade_tanh(10.0 * finite_or_zero(*x)) / 10.0)
            .collect();
        let third_1x = dft_magnitude(&direct[SKIP..], THIRD_BIN);
        assert!(
            third_1x > 0.001,
            "1x 直算的 3 次谐波折叠必须显著非零（否则比较真空），实读 {third_1x}"
        );
        assert!(
            third_2x < third_1x,
            "2x 过采样的 3 次谐波折叠（{third_2x}）必须低于 1x 直算（{third_1x}）"
        );
        // 基频本身必须在（输入不是静默、DFT 口径没错）：drive 1.0 时饱和把 0.9
        // 正弦峰值压到约 0.129，基频幅值约 0.063，故不按 0.1 判，只判器件基频
        // 不低于直算基频的一半（上下采样通带纹波在 10 kHz 处 |H| = 1.0000）。
        let f0_direct = dft_magnitude(&direct[SKIP..], F0_BIN);
        assert!(f0_direct > 0.05, "直算基频读数异常，实读 {f0_direct}");
        let f0_tape = dft_magnitude(&left[SKIP..], F0_BIN);
        assert!(
            f0_tape > 0.5 * f0_direct,
            "器件基频（{f0_tape}）不得远低于直算基频（{f0_direct}）"
        );
    }
}
