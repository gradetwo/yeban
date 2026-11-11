//! 噪声门 (Noise Gate) 与 扩展器 (Expander) 及 专用闪避 (Ducker)。[ARCH-RT-001] [ARCH-DET-001] [ARCH-DSP-001]
//!
//! ## 规范与缺口对应
//!
//! - **G-D1**：噪声门与扩展器核心实现（`GateMode::Gate`、`GateMode::Expander`）。
//! - **G-S2**：侧链键信号滤波（HPF 高通滤波 + LPF 低通滤波，避免次声隆隆与高频串音误触发）。
//! - **G-S3**：专用闪避模式（`GateMode::Ducker`，键信号高于阈值时压低节目信号）。
//!
//! ## 信号处理流程（逐样本，固定顺序）
//!
//! ```text
//! ① 键信号来源：自检波（节目信号）或外侧链键信号（process_gain_with_key）
//! ② 侧链滤波（G-S2）：HPF（20–2000 Hz）+ LPF（200–20000 Hz）限带
//! ③ 检波器：立体声联动 max(l², r²) [RMS 模式] 或 max(|l|, |r|) [Peak 模式]
//! ④ 电平估算：level_db = 10·log10(ms) 或 20·log10(peak)
//! ⑤ 静态曲线与迟滞判断（Hysteresis + Hold）：
//!    - Gate: level < (threshold - hyst) ⇒ 关门衰减至 range_db；level >= threshold ⇒ 开门 (0 dB)
//!    - Expander: level < threshold ⇒ 沿比率斜率向下扩展衰减
//!    - Ducker: level >= threshold ⇒ 闪避衰减至 range_db；level < (threshold - hyst) ⇒ 恢复 (0 dB)
//! ⑥ 增益弹道：Attack / Hold / Release 一阶低通平滑
//! ⑦ 应用增益：节目信号 · db_to_gain(current_gain_db)
//! ```
//!
//! 实时安全：全流程零堆分配、零互斥锁、零系统调用、FTZ/DAZ 保护。

use crate::math::{db_to_gain, gain_to_db};
use crate::shaping::Biquad;

/// 噪声门工作模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GateMode {
    /// 经典噪声门：低于阈值时彻底或深度压低底噪与串音。
    #[default]
    Gate,
    /// 向下扩展器：低于阈值时按比率平滑衰减，用于更自然的动态收敛。
    Expander,
    /// 侧链闪避器（G-S3）：当键信号高于阈值时闪避压低节目信号。
    Ducker,
}

/// 检波口径。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GateDetector {
    /// 峰值检波：对打击乐瞬态响应极快，开门迅速。
    #[default]
    Peak,
    /// 均方根 (RMS) 检波：平滑感知电平，适合人声与持续乐器。
    Rms,
}

/// 噪声门/扩展器参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseGateParams {
    /// 开门阈值（dBFS）。范围 [-80.0, 0.0]，默认 -40.0。
    pub threshold_db: f32,
    /// 最大衰减量/下限范围（dB，负数）。范围 [-96.0, 0.0]，默认 -80.0。
    pub range_db: f32,
    /// 扩展/关门斜率比率。范围 [1.0, 100.0]，Gate 默认 20.0，Expander 默认 2.0。
    pub ratio: f32,
    /// 起振开门时间（秒）。范围 [0.0001, 0.1]（0.1 ms – 100 ms），默认 0.001 (1 ms)。
    pub attack_s: f32,
    /// 保持时间（秒）：信号落回阈值后保持开门的时间，防抖防爆音。
    /// 范围 [0.0, 1.0]（0 ms – 1000 ms），默认 0.020 (20 ms)。
    pub hold_s: f32,
    /// 释放关门时间（秒）。范围 [0.005, 2.0]（5 ms – 2000 ms），默认 0.100 (100 ms)。
    pub release_s: f32,
    /// 迟滞量（dB）：关门阈值相对于开门阈值的额外下潜量，防临界震荡。
    /// 范围 [0.0, 12.0]，默认 2.0。
    pub hysteresis_db: f32,
    /// 侧链高通滤波截止频率（Hz，G-S2）。<= 20.0 视为旁通。范围 [20.0, 2000.0]，默认 0.0 (旁通)。
    pub hpf_hz: f32,
    /// 侧链低通滤波截止频率（Hz，G-S2）。>= 20000.0 视为旁通。范围 [200.0, 20000.0]，默认 20000.0 (旁通)。
    pub lpf_hz: f32,
    /// 工作模式。
    pub mode: GateMode,
    /// 检波模式。
    pub detector: GateDetector,
}

impl Default for NoiseGateParams {
    fn default() -> Self {
        Self {
            threshold_db: -40.0,
            range_db: -80.0,
            ratio: 20.0,
            attack_s: 0.001,
            hold_s: 0.020,
            release_s: 0.100,
            hysteresis_db: 2.0,
            hpf_hz: 0.0,
            lpf_hz: 20000.0,
            mode: GateMode::Gate,
            detector: GateDetector::Peak,
        }
    }
}

impl NoiseGateParams {
    /// 扩展器推荐默认参数预设。
    #[must_use]
    pub fn expander_preset() -> Self {
        Self {
            threshold_db: -30.0,
            range_db: -24.0,
            ratio: 2.0,
            attack_s: 0.005,
            hold_s: 0.010,
            release_s: 0.150,
            hysteresis_db: 1.5,
            hpf_hz: 0.0,
            lpf_hz: 20000.0,
            mode: GateMode::Expander,
            detector: GateDetector::Peak,
        }
    }

    /// 侧链闪避器 (Ducker) 推荐默认参数预设（G-S3）。
    #[must_use]
    pub fn ducker_preset() -> Self {
        Self {
            threshold_db: -24.0,
            range_db: -12.0,
            ratio: 10.0,
            attack_s: 0.005,
            hold_s: 0.050,
            release_s: 0.200,
            hysteresis_db: 2.0,
            hpf_hz: 80.0, // 默认过滤低端次声
            lpf_hz: 20000.0,
            mode: GateMode::Ducker,
            detector: GateDetector::Peak,
        }
    }

    /// 钳制到合法物理定义域。
    #[must_use]
    pub fn sanitised(self) -> Self {
        let threshold_db = if self.threshold_db.is_finite() {
            self.threshold_db.clamp(-80.0, 0.0)
        } else {
            -40.0
        };
        let range_db = if self.range_db.is_finite() {
            self.range_db.clamp(-96.0, 0.0)
        } else {
            -80.0
        };
        let ratio = if self.ratio.is_finite() {
            self.ratio.clamp(1.0, 100.0)
        } else {
            20.0
        };
        let attack_s = if self.attack_s.is_finite() {
            self.attack_s.clamp(0.0001, 0.1)
        } else {
            0.001
        };
        let hold_s = if self.hold_s.is_finite() {
            self.hold_s.clamp(0.0, 1.0)
        } else {
            0.020
        };
        let release_s = if self.release_s.is_finite() {
            self.release_s.clamp(0.005, 2.0)
        } else {
            0.100
        };
        let hysteresis_db = if self.hysteresis_db.is_finite() {
            self.hysteresis_db.clamp(0.0, 12.0)
        } else {
            2.0
        };
        let hpf_hz = if self.hpf_hz.is_finite() && self.hpf_hz > 20.0 {
            self.hpf_hz.clamp(20.0, 2000.0)
        } else {
            0.0
        };
        let lpf_hz = if self.lpf_hz.is_finite() && self.lpf_hz < 20000.0 {
            self.lpf_hz.clamp(200.0, 20000.0)
        } else {
            20000.0
        };

        Self {
            threshold_db,
            range_db,
            ratio,
            attack_s,
            hold_s,
            release_s,
            hysteresis_db,
            hpf_hz,
            lpf_hz,
            mode: self.mode,
            detector: self.detector,
        }
    }
}

/// 纯前馈、零分配、带侧链滤波与保持的噪声门/扩展器/闪避器器件。
#[derive(Clone, Debug)]
pub struct NoiseGate {
    params: NoiseGateParams,
    sample_rate: f32,

    // 侧链键滤波器（左右独立，G-S2）
    key_hpf: [Biquad; 2],
    key_lpf: [Biquad; 2],

    // 检波状态
    detector_peak: f32,
    detector_ms: f32,

    // 门控逻辑状态
    is_open: bool,
    hold_remaining_samples: u32,

    // 弹道滤波状态 (dB)
    current_gain_db: f32,
    linear_gain: f32,

    // 预计算的一阶系数
    alpha_attack: f32,
    alpha_release: f32,
    alpha_det_ms: f32,
}

#[inline]
fn finite_or_zero(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

#[inline]
fn sanitise_sample_rate(sr: f32) -> f32 {
    if sr.is_finite() && (8000.0..=384000.0).contains(&sr) {
        sr
    } else {
        48000.0
    }
}

impl NoiseGate {
    /// 构造新实例。
    #[must_use]
    pub fn new(params: NoiseGateParams, sample_rate: f32) -> Self {
        let sanitised_params = params.sanitised();
        let sr = sanitise_sample_rate(sample_rate);
        let initial_gain_db = match sanitised_params.mode {
            GateMode::Ducker => 0.0,
            GateMode::Gate | GateMode::Expander => sanitised_params.range_db,
        };
        let mut gate = Self {
            params: sanitised_params,
            sample_rate: sr,
            key_hpf: [Biquad::identity(), Biquad::identity()],
            key_lpf: [Biquad::identity(), Biquad::identity()],
            detector_peak: 0.0,
            detector_ms: 0.0,
            is_open: false,
            hold_remaining_samples: 0,
            current_gain_db: initial_gain_db,
            linear_gain: db_to_gain(initial_gain_db),
            alpha_attack: 0.0,
            alpha_release: 0.0,
            alpha_det_ms: 0.0,
        };
        gate.update_filter_coefficients();
        gate.update_ballistics_coefficients();
        gate
    }

    /// 清空运行状态，保留配置。
    pub fn reset(&mut self) {
        self.key_hpf[0].clear_state();
        self.key_hpf[1].clear_state();
        self.key_lpf[0].clear_state();
        self.key_lpf[1].clear_state();
        self.detector_peak = 0.0;
        self.detector_ms = 0.0;
        self.is_open = false;
        self.hold_remaining_samples = 0;
        self.current_gain_db = match self.params.mode {
            GateMode::Ducker => 0.0,
            GateMode::Gate | GateMode::Expander => self.params.range_db,
        };
        self.linear_gain = db_to_gain(self.current_gain_db);
    }

    /// 更新参数。
    pub fn set_params(&mut self, params: NoiseGateParams) {
        let prev_hpf = self.params.hpf_hz;
        let prev_lpf = self.params.lpf_hz;
        self.params = params.sanitised();
        if self.params.hpf_hz != prev_hpf || self.params.lpf_hz != prev_lpf {
            self.update_filter_coefficients();
        }
        self.update_ballistics_coefficients();
    }

    /// 更新采样率。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sanitise_sample_rate(sample_rate);
        if (sr - self.sample_rate).abs() > 0.001 {
            self.sample_rate = sr;
            self.update_filter_coefficients();
            self.update_ballistics_coefficients();
        }
    }

    /// 获取当前有效参数。
    #[must_use]
    pub const fn params(&self) -> &NoiseGateParams {
        &self.params
    }

    /// 当前施加在信号上的线性乘子增益 [0.0, 1.0]。
    #[must_use]
    pub const fn current_gain(&self) -> f32 {
        self.linear_gain
    }

    /// 当前增益 (dB，非正数)。
    #[must_use]
    pub const fn current_gain_db(&self) -> f32 {
        self.current_gain_db
    }

    /// 当前衰减量 (dB，非负数，供增益衰减表使用)。
    #[must_use]
    pub fn reduction_db(&self) -> f32 {
        (-self.current_gain_db).max(0.0)
    }

    /// 当前门是否处于开启态。
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.is_open
    }

    fn update_ballistics_coefficients(&mut self) {
        let sr = self.sample_rate;
        // 1阶低通滤波器 α = 1 - exp(-1 / (τ * fs))
        let dt = 1.0 / sr;
        self.alpha_attack =
            (1.0 - (-dt / self.params.attack_s.max(0.00005)).exp()).clamp(0.0001, 1.0);
        self.alpha_release =
            (1.0 - (-dt / self.params.release_s.max(0.0005)).exp()).clamp(0.00001, 1.0);
        // 对称 RMS 平均窗口取 10ms
        self.alpha_det_ms = (1.0 - (-dt / 0.010).exp()).clamp(0.0001, 1.0);
    }

    fn update_filter_coefficients(&mut self) {
        let sr = self.sample_rate;
        let q = core::f32::consts::FRAC_1_SQRT_2; // Butterworth Q = 0.7071

        if self.params.hpf_hz > 20.0 {
            for hpf in &mut self.key_hpf {
                hpf.highpass(sr, self.params.hpf_hz, q);
            }
        } else {
            for hpf in &mut self.key_hpf {
                *hpf = Biquad::identity();
            }
        }

        if self.params.lpf_hz < 20000.0 {
            for lpf in &mut self.key_lpf {
                lpf.lowpass(sr, self.params.lpf_hz, q);
            }
        } else {
            for lpf in &mut self.key_lpf {
                *lpf = Biquad::identity();
            }
        }
    }

    /// 核心逐样本推进：计算给定立体声键信号驱动下的增益。
    #[inline]
    pub fn process_gain_with_key(&mut self, key_left: f32, key_right: f32) -> f32 {
        let mut kl = finite_or_zero(key_left);
        let mut kr = finite_or_zero(key_right);

        // ② 侧链滤波（G-S2）
        if self.params.hpf_hz > 20.0 {
            kl = self.key_hpf[0].process_sample(kl);
            kr = self.key_hpf[1].process_sample(kr);
        }
        if self.params.lpf_hz < 20000.0 {
            kl = self.key_lpf[0].process_sample(kl);
            kr = self.key_lpf[1].process_sample(kr);
        }

        kl = finite_or_zero(kl);
        kr = finite_or_zero(kr);

        // ③ 电平检波
        let level_db = match self.params.detector {
            GateDetector::Peak => {
                let peak_in = kl.abs().max(kr.abs());
                // 瞬态快速上升，5 ms 平滑释放（覆盖音频周期防过零震荡，同时不滞后关门）
                if peak_in >= self.detector_peak {
                    self.detector_peak = peak_in;
                } else {
                    let rel_alpha =
                        (1.0 - (-1.0 / (self.sample_rate * 0.005)).exp()).clamp(0.0001, 1.0);
                    self.detector_peak += rel_alpha * (peak_in - self.detector_peak);
                }
                let peak = self.detector_peak.max(1e-6);
                gain_to_db(peak)
            }
            GateDetector::Rms => {
                let ms_in = (kl * kl).max(kr * kr);
                self.detector_ms += self.alpha_det_ms * (ms_in - self.detector_ms);
                let ms = self.detector_ms.max(1e-12);
                10.0 * libm::log10f(ms)
            }
        };

        // ⑤ 静态曲线、迟滞与 Hold 计算目标增益
        let target_gain_db = match self.params.mode {
            GateMode::Gate => {
                let open_thresh = self.params.threshold_db;
                let close_thresh = open_thresh - self.params.hysteresis_db;

                if level_db >= open_thresh {
                    self.is_open = true;
                    self.hold_remaining_samples = (self.params.hold_s * self.sample_rate) as u32;
                    0.0
                } else if self.is_open {
                    if level_db >= close_thresh || self.hold_remaining_samples > 0 {
                        if self.hold_remaining_samples > 0 {
                            self.hold_remaining_samples -= 1;
                        }
                        0.0
                    } else {
                        self.is_open = false;
                        self.params.range_db
                    }
                } else {
                    // 门处于关闭态：若 ratio > 1.0 且有限，计算斜率衰减，否则到底
                    if self.params.ratio > 1.0 {
                        let diff = level_db - open_thresh; // 负数
                        let slope_att = diff * (self.params.ratio - 1.0);
                        slope_att.clamp(self.params.range_db, 0.0)
                    } else {
                        self.params.range_db
                    }
                }
            }
            GateMode::Expander => {
                let thresh = self.params.threshold_db;
                if level_db >= thresh {
                    self.is_open = true;
                    self.hold_remaining_samples = (self.params.hold_s * self.sample_rate) as u32;
                    0.0
                } else if self.hold_remaining_samples > 0 {
                    self.hold_remaining_samples -= 1;
                    0.0
                } else {
                    self.is_open = false;
                    let diff = level_db - thresh; // 负数
                    let att = diff * (self.params.ratio - 1.0);
                    att.clamp(self.params.range_db, 0.0)
                }
            }
            GateMode::Ducker => {
                // G-S3: 键信号高于阈值时闪避压低
                let duck_thresh = self.params.threshold_db;
                let release_thresh = duck_thresh - self.params.hysteresis_db;

                if level_db >= duck_thresh {
                    self.is_open = true; // "激活闪避"
                    self.hold_remaining_samples = (self.params.hold_s * self.sample_rate) as u32;
                    self.params.range_db
                } else if self.is_open {
                    if level_db >= release_thresh || self.hold_remaining_samples > 0 {
                        if self.hold_remaining_samples > 0 {
                            self.hold_remaining_samples -= 1;
                        }
                        self.params.range_db
                    } else {
                        self.is_open = false;
                        0.0
                    }
                } else {
                    0.0
                }
            }
        };

        // ⑥ 弹道平滑
        let alpha = match self.params.mode {
            GateMode::Ducker => {
                // 闪避器：压低是起振 (attack)，恢复是释放 (release)
                if target_gain_db < self.current_gain_db {
                    self.alpha_attack
                } else {
                    self.alpha_release
                }
            }
            GateMode::Gate | GateMode::Expander => {
                // 门/扩展器：开门提升增益是起振 (attack)，关门衰减是释放 (release)
                if target_gain_db > self.current_gain_db {
                    self.alpha_attack
                } else {
                    self.alpha_release
                }
            }
        };
        self.current_gain_db += alpha * (target_gain_db - self.current_gain_db);
        self.current_gain_db = self.current_gain_db.clamp(self.params.range_db, 0.0);
        self.linear_gain = db_to_gain(self.current_gain_db);
        self.linear_gain
    }

    /// 推进自检波，返回当前帧的增益。
    #[inline]
    pub fn process_gain(&mut self, left: f32, right: f32) -> f32 {
        self.process_gain_with_key(left, right)
    }

    /// 原地处理立体声块。
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> usize {
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let gain = self.process_gain(left[i], right[i]);
            left[i] = finite_or_zero(left[i] * gain);
            right[i] = finite_or_zero(right[i] * gain);
        }
        frames
    }

    /// 原地带键处理立体声块。
    pub fn process_stereo_with_key(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        key_l: &[f32],
        key_r: &[f32],
    ) -> usize {
        let frames = left
            .len()
            .min(right.len())
            .min(key_l.len())
            .min(key_r.len());
        for i in 0..frames {
            let gain = self.process_gain_with_key(key_l[i], key_r[i]);
            left[i] = finite_or_zero(left[i] * gain);
            right[i] = finite_or_zero(right[i] * gain);
        }
        frames
    }

    /// 原地处理单声道。
    pub fn process_mono(&mut self, samples: &mut [f32]) -> usize {
        let frames = samples.len();
        for sample in samples.iter_mut() {
            let gain = self.process_gain(*sample, *sample);
            *sample = finite_or_zero(*sample * gain);
        }
        frames
    }

    /// 原地带键处理单声道。
    pub fn process_mono_with_key(&mut self, samples: &mut [f32], key: &[f32]) -> usize {
        let frames = samples.len().min(key.len());
        for i in 0..frames {
            let gain = self.process_gain_with_key(key[i], key[i]);
            samples[i] = finite_or_zero(samples[i] * gain);
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_signal_is_gated_below_threshold() {
        let params = NoiseGateParams {
            threshold_db: -30.0,
            range_db: -60.0,
            attack_s: 0.001,
            release_s: 0.005,
            ..Default::default()
        };
        let mut gate = NoiseGate::new(params, 48000.0);

        // 喂入 -50 dBFS 的微弱噪声/信号（远低于 -30 dBFS 阈值）
        let quiet_val = db_to_gain(-50.0);
        let mut left = vec![quiet_val; 4800];
        let mut right = vec![quiet_val; 4800];

        gate.process_stereo(&mut left, &mut right);

        // 门应保持关闭，信号衰减约 -60 dB (总电平约 -110 dB)
        assert!(!gate.is_open());
        assert!(gate.current_gain_db() <= -59.0);
        let out_db = gain_to_db(left[4799].abs());
        assert!(out_db <= -100.0, "实际输出电平: {out_db} dB");
    }

    #[test]
    fn loud_signal_opens_gate_to_unity() {
        let params = NoiseGateParams {
            threshold_db: -30.0,
            range_db: -60.0,
            attack_s: 0.001,
            ..Default::default()
        };
        let mut gate = NoiseGate::new(params, 48000.0);

        // 喂入 -10 dBFS 的信号（高于 -30 dBFS 阈值）
        let loud_val = db_to_gain(-10.0);
        let mut left = vec![loud_val; 4800];
        let mut right = vec![loud_val; 4800];

        gate.process_stereo(&mut left, &mut right);

        assert!(gate.is_open());
        assert!(
            gate.current_gain_db() > -0.05,
            "增益应趋近 0 dB: {}",
            gate.current_gain_db()
        );
        assert!((left[4799] - loud_val).abs() < 1e-3);
    }

    #[test]
    fn ducker_mode_ducks_on_loud_key() {
        // G-S3 闪避模式验证
        let params = NoiseGateParams {
            threshold_db: -20.0,
            range_db: -12.0, // 闪避量 -12 dB
            attack_s: 0.001,
            release_s: 0.010,
            hpf_hz: 0.0, // 允许 DC 测试信号通过
            ..NoiseGateParams::ducker_preset()
        };
        let mut gate = NoiseGate::new(params, 48000.0);

        let prog_val = db_to_gain(-6.0);
        let key_loud = db_to_gain(-10.0); // 高于 -20 dB 触发闪避

        let mut left = vec![prog_val; 4800];
        let mut right = vec![prog_val; 4800];
        let key_l = vec![key_loud; 4800];
        let key_r = vec![key_loud; 4800];

        gate.process_stereo_with_key(&mut left, &mut right, &key_l, &key_r);

        // 节目信号应被压低约 12 dB
        assert!(gate.is_open());
        let expected_gain_db = -12.0;
        assert!(
            (gate.current_gain_db() - expected_gain_db).abs() < 0.2,
            "当前增益: {}",
            gate.current_gain_db()
        );
    }

    #[test]
    fn sidechain_hpf_rejects_sub_rumble_key() {
        // G-S2: 键信号 HPF 滤波验证
        let params = NoiseGateParams {
            threshold_db: -20.0,
            hpf_hz: 150.0, // 过滤 150 Hz 以下次声
            attack_s: 0.001,
            ..Default::default()
        };
        let mut gate = NoiseGate::new(params, 48000.0);

        // 构造一个 30 Hz 的高能量正弦波作为键信号（本应足以触发开门）
        let mut key_l = Vec::with_capacity(4800);
        let mut key_r = Vec::with_capacity(4800);
        for i in 0..4800 {
            let phase = 2.0 * core::f32::consts::PI * 30.0 * (i as f32) / 48000.0;
            let s = db_to_gain(-6.0) * libm::sinf(phase);
            key_l.push(s);
            key_r.push(s);
        }

        let mut prog_l = vec![db_to_gain(-40.0); 4800];
        let mut prog_r = vec![db_to_gain(-40.0); 4800];

        gate.process_stereo_with_key(&mut prog_l, &mut prog_r, &key_l, &key_r);

        // 30 Hz 信号被 150 Hz 二阶高通大幅衰减，检波电平无法达到 -20 dB 阈值 ⇒ 门保持关闭
        assert!(!gate.is_open());
        assert!(gate.current_gain_db() <= -70.0);
    }

    #[test]
    fn expander_mode_provides_proportional_reduction() {
        // G-D1 扩展器模式验证
        let mut params = NoiseGateParams::expander_preset();
        params.threshold_db = -20.0;
        params.ratio = 2.0; // 1:2 扩展：低于阈值 10 dB 则衰减 10 * (2 - 1) = 10 dB
        params.range_db = -40.0;
        params.attack_s = 0.001;
        params.release_s = 0.001;
        let mut gate = NoiseGate::new(params, 48000.0);

        // 喂入 -30 dBFS 信号（比阈值低 10 dB）
        let val = db_to_gain(-30.0);
        let mut left = vec![val; 4800];
        let mut right = vec![val; 4800];

        gate.process_stereo(&mut left, &mut right);

        // 预期衰减约 -10 dB (增益约 -10 dB)
        let g_db = gate.current_gain_db();
        assert!((g_db - -10.0).abs() < 1.0, "实际扩展衰减: {g_db} dB");
    }

    #[test]
    fn nan_and_infinity_safe() {
        let mut gate = NoiseGate::new(NoiseGateParams::default(), 48000.0);
        let mut bad_left = vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5];
        let mut bad_right = vec![f32::NAN, 1.0, f32::NEG_INFINITY, 0.5];

        gate.process_stereo(&mut bad_left, &mut bad_right);

        for &x in &bad_left {
            assert!(x.is_finite());
        }
        for &x in &bad_right {
            assert!(x.is_finite());
        }
    }
}
