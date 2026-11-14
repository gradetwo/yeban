//! 逐声部静态滤波器（SFZ 滤波静态族的 DSP 落点）。
//!
//! 规范来源 (Normative): `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`
//! ROAD-M2-005（SFZ 引擎）；实时纪律见 [ARCH-RT-001]（逐样本路径零分配）。
//! 格式事实见 `yeban-sfz` 的 `FilterType` / `Region::cutoff` 文档
//! （<https://sfzformat.com/opcodes/fil_type/> 等）。
//!
//! # 定位（⛔ 不是第二套通用滤波）
//!
//! * 本模块只服务 **SFZ 声部**：参数在声部触发时按 (类型, 有效截止, 谐振,
//!   采样率) 一次性配置，之后逐样本只跑递推，**不做逐采样调制**
//!   （`cutoff_onccN` / `fileg_*` / LFO 属后续线，届时另加调制入口，
//!   ⛔ 不在本结构里预埋）。
//!
//!   ⭐ ⭐ **追加修订段（SFZ 包络族片）—— 被修订的是上面这句里 `fileg_*`
//!   那一支**（⛔ 原文一字未删）：`fileg_*` 已由本片落地 ⇒ 调制入口就是
//!   [`VoiceFilter::retune_cutoff`]：只换系数、**保留递推状态**（与
//!   [`VoiceFilter::configured`] 的"状态清零 ＝ note-on 语义"相对）。
//!   为此本结构自带 (类型, 谐振, 采样率) 三个配置标量（尺寸 88 → 96 B）。
//!   `cutoff_onccN` 与 LFO 两支**仍未做**，原文对它们继续成立。
//! * 既有 [`crate::filter::LadderFilter`] 是 0..1 旋钮语义的合成器梯形
//!   滤波，[`crate::shaping::Biquad`] 是 `pub(crate)` 的通道条 EQ 节 ——
//!   两者都不是 SFZ 的 (Hz, dB) 参数面，故另立本模块；系数表与
//!   [`crate::shaping`] 用**同一套 RBJ 公式**（出处相同，不发明第二套）。
//!
//! # 系数与语义裁决（全部明记，⛔ 不静默）
//!
//! * **二阶节**：RBJ 双二阶（Audio EQ Cookbook），与
//!   [`crate::shaping::Biquad`] 同式；递推用转置直接 II 型、单声道状态。
//! * **谐振换算**：`resonance`（dB）定义为**截止频率处的峰值增益**。
//!   RBJ 低通/高通在 f0 处的幅值恰为 Q（线性）⇒ `Q = 10^(dB/20)`。
//!   级联型（4p/6p）把总 dB 在各节**均分**，使 f0 处总增益仍等于该 dB。
//! * **一阶节**：单极点 RC 形，`p = exp(-2π·fc/sr)`；低通
//!   `H(z) = (1-p)/(1-p·z⁻¹)`、高通 `H(z) = p(1-z⁻¹)/(1-p·z⁻¹)`，
//!   套进同一双二阶递推形（`b2 = a2 = 0`）。一阶没有谐振峰 ⇒
//!   `resonance` 对 1p 类型**不生效**（格式未给 1p 定义谐振语义）。
//! * **带通**：RBJ 恒 0 dB 峰值增益形（`b = (α, 0, −α)`）；**带阻**：
//!   RBJ 陷波形（`b = (1, −2cos, 1)`）。两者的 Q 由 `resonance` 同式给出
//!   （决定带宽/陷宽）。
//! * **频率钳制**（系数域，⛔ 不回写调用方的原始 Hz）：有效截止在系数
//!   计算时钳进 `[1 Hz, 0.495 × sr]`。上界是双二阶稳定域的必需
//!   （f0 必须 < Nyquist）；下界避免极低频下 f32 系数退化。钳制只影响
//!   系数，不影响 `yeban-sfz` 报告的原始/有效 Hz 读数。
//! * **防御性归一**：`sample_rate` 走 [`sanitise_sample_rate`]；
//!   `resonance_db` 钳进规范域 `[0, 40]`（解析层已保证，此处只为手工
//!   调用兜底）；`cutoff_hz` 为 NaN ⇒ 直通（⛔ 不让 NaN 进递推）。
//!
//! # 直通逐位恒等
//!
//! [`VoiceFilter::BYPASSED`] 的 [`VoiceFilter::process`] **原样返回**
//! 输入（连递推都不进）⇒ 没有 `cutoff` 的 SFZ 文件渲染逐位不变。
//! 这是本模块的第一判据（模块测试 `bypass_is_bit_exact`）。

use crate::math::sanitise_sample_rate;

use core::f32::consts::TAU;

/// 截止频率钳制的下界（Hz；系数域，见模块文档）。
const MIN_CUTOFF_HZ: f32 = 1.0;
/// 截止频率钳制的上界（占采样率比例；系数域，见模块文档）。
const MAX_CUTOFF_FRACTION: f32 = 0.495;
/// 谐振的规范域（dB；与 `yeban-sfz` 的 `RESONANCE_MIN/MAX_DB` 同值，
/// 本 crate 不反向依赖 `yeban-sfz`，故在此复述并由测试对齐语义）。
const RESONANCE_DB_RANGE: (f32, f32) = (0.0, 40.0);
/// Q 的下限（与 [`crate::shaping::Biquad`] 的 `q.max(0.05)` 同值）：
/// 防 `alpha = sin/(2Q)` 在 Q→0 时爆掉。
const MIN_Q: f32 = 0.05;

/// SFZ 滤波器类型在本 crate 的镜像（与 `yeban-sfz` 的 `FilterType`
/// 一一对应；映射由引擎层做，本 crate ⛔ 不依赖 `yeban-sfz`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceFilterKind {
    /// `lpf_1p`：一阶低通。
    Lowpass1p,
    /// `hpf_1p`：一阶高通。
    Highpass1p,
    /// `lpf_2p`：二阶低通。
    Lowpass2p,
    /// `hpf_2p`：二阶高通。
    Highpass2p,
    /// `bpf_2p`：二阶带通（恒 0 dB 峰值）。
    Bandpass2p,
    /// `brf_2p`：二阶带阻（陷波）。
    Bandreject2p,
    /// `lpf_4p`：四阶低通（两节级联）。
    Lowpass4p,
    /// `hpf_4p`：四阶高通（两节级联）。
    Highpass4p,
    /// `lpf_6p`：六阶低通（三节级联）。
    Lowpass6p,
    /// `hpf_6p`：六阶高通（三节级联）。
    Highpass6p,
}

impl VoiceFilterKind {
    /// 级联节数：一阶/二阶 = 1 节，4p = 2 节，6p = 3 节。
    #[must_use]
    pub fn sections(self) -> usize {
        match self {
            VoiceFilterKind::Lowpass1p
            | VoiceFilterKind::Highpass1p
            | VoiceFilterKind::Lowpass2p
            | VoiceFilterKind::Highpass2p
            | VoiceFilterKind::Bandpass2p
            | VoiceFilterKind::Bandreject2p => 1,
            VoiceFilterKind::Lowpass4p | VoiceFilterKind::Highpass4p => 2,
            VoiceFilterKind::Lowpass6p | VoiceFilterKind::Highpass6p => 3,
        }
    }

    /// ⭐ 紧凑编码（[`VoiceFilter`] 自带配置用；`0` ＝ 无类型/直通）。
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            VoiceFilterKind::Lowpass1p => 1,
            VoiceFilterKind::Highpass1p => 2,
            VoiceFilterKind::Lowpass2p => 3,
            VoiceFilterKind::Highpass2p => 4,
            VoiceFilterKind::Bandpass2p => 5,
            VoiceFilterKind::Bandreject2p => 6,
            VoiceFilterKind::Lowpass4p => 7,
            VoiceFilterKind::Highpass4p => 8,
            VoiceFilterKind::Lowpass6p => 9,
            VoiceFilterKind::Highpass6p => 10,
        }
    }

    /// [`Self::code`] 的逆（认不出 ⇒ `None`）。
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(VoiceFilterKind::Lowpass1p),
            2 => Some(VoiceFilterKind::Highpass1p),
            3 => Some(VoiceFilterKind::Lowpass2p),
            4 => Some(VoiceFilterKind::Highpass2p),
            5 => Some(VoiceFilterKind::Bandpass2p),
            6 => Some(VoiceFilterKind::Bandreject2p),
            7 => Some(VoiceFilterKind::Lowpass4p),
            8 => Some(VoiceFilterKind::Highpass4p),
            9 => Some(VoiceFilterKind::Lowpass6p),
            10 => Some(VoiceFilterKind::Highpass6p),
            _ => None,
        }
    }
}

/// 一节滤波（双二阶形，含一阶的退化形）：系数 `(b0, b1, b2, a1, a2)`
/// 已按 `a0` 归一化；状态 `(v1, v2)` 是转置直接 II 型的中间量
/// （与 [`crate::shaping::Biquad`] 同一条递推）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Section {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    v1: f32,
    v2: f32,
}

impl Section {
    const IDENTITY: Section = Section {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
        v1: 0.0,
        v2: 0.0,
    };

    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        let inv = 1.0 / a0;
        self.b0 = b0 * inv;
        self.b1 = b1 * inv;
        self.b2 = b2 * inv;
        self.a1 = a1 * inv;
        self.a2 = a2 * inv;
        self.v1 = 0.0;
        self.v2 = 0.0;
    }

    /// RBJ 二阶低通（与 [`crate::shaping::Biquad::lowpass`] 同式）。
    fn lowpass(&mut self, sample_rate: f32, freq: f32, q: f32) {
        let w0 = TAU * (freq / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(MIN_Q));
        self.set(
            (1.0 - cos) * 0.5,
            1.0 - cos,
            (1.0 - cos) * 0.5,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        );
    }

    /// RBJ 二阶高通（与 [`crate::shaping::Biquad::highpass`] 同式）。
    fn highpass(&mut self, sample_rate: f32, freq: f32, q: f32) {
        let w0 = TAU * (freq / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(MIN_Q));
        self.set(
            (1.0 + cos) * 0.5,
            -(1.0 + cos),
            (1.0 + cos) * 0.5,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        );
    }

    /// RBJ 二阶带通，恒 0 dB 峰值增益形（与
    /// [`crate::shaping::Biquad::bandpass`] 同式）。
    fn bandpass(&mut self, sample_rate: f32, freq: f32, q: f32) {
        let w0 = TAU * (freq / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(MIN_Q));
        self.set(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha);
    }

    /// RBJ 二阶带阻（陷波）。
    fn bandreject(&mut self, sample_rate: f32, freq: f32, q: f32) {
        let w0 = TAU * (freq / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(MIN_Q));
        self.set(1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha);
    }

    /// 一阶低通（单极点 RC 形，套双二阶递推：`b2 = a2 = 0`）。
    fn lowpass_1p(&mut self, sample_rate: f32, freq: f32) {
        let p = (-TAU * freq / sample_rate).exp();
        self.set(1.0 - p, 0.0, 0.0, 1.0, -p, 0.0);
    }

    /// 一阶高通（单极点 RC 形，套双二阶递推：`b2 = a2 = 0`）。
    fn highpass_1p(&mut self, sample_rate: f32, freq: f32) {
        let p = (-TAU * freq / sample_rate).exp();
        self.set(p, -p, 0.0, 1.0, -p, 0.0);
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let v = x - self.a1 * self.v1 - self.a2 * self.v2;
        let y = self.b0 * v + self.b1 * self.v1 + self.b2 * self.v2;
        self.v2 = self.v1;
        self.v1 = v;
        y
    }
}

/// 一只声部的静态滤波器：至多 3 节级联，单声道，`Copy`。
///
/// 尺寸 96 B（3 节 × 28 B ＋ 节数标记 ＋ 自带配置三标量；SFZ 包络族片
/// 为 [`VoiceFilter::retune_cutoff`] 加的 (类型码, 谐振, 采样率) ——
/// 滤波静态族时是 88 B，净增 8 B：类型码占原对齐空隙），可整数组放进声部池侧
/// （引擎 `SfzVoicePool`），⛔ 不进逐声部结构体本身。
/// 构造只有两条路：[`VoiceFilter::BYPASSED`]（直通）与
/// [`VoiceFilter::configured`]（按参数算系数、状态清零 ＝ note-on 语义）；
/// 配置之后改截止走 [`VoiceFilter::retune_cutoff`]（保状态）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceFilter {
    sections: [Section; 3],
    active: u8,
    /// ⭐ 配置类型码（[`VoiceFilterKind::code`]；`0` ＝ 直通）。
    kind_code: u8,
    /// ⭐ 配置时的谐振（dB，已钳进规范域；retune 时原样复用）。
    resonance_db: f32,
    /// ⭐ 配置时的采样率（已 [`sanitise_sample_rate`]；retune 时原样复用）。
    sample_rate: f32,
}

impl VoiceFilter {
    /// 直通滤波器：[`VoiceFilter::process`] 原样返回输入（逐位恒等）。
    pub const BYPASSED: VoiceFilter = VoiceFilter {
        sections: [Section::IDENTITY; 3],
        active: 0,
        kind_code: 0,
        resonance_db: 0.0,
        sample_rate: 0.0,
    };

    /// 按 (类型, 有效截止 Hz, 谐振 dB, 采样率) 配置一只新滤波器
    /// （递推状态清零）。参数的防御性归一见模块文档。
    ///
    /// `cutoff_hz` 是**已折算**的有效截止（键位/力度跟踪由调用方经
    /// `yeban-sfz` 的 `filter_cutoff_hz` 折好），本函数只做系数域钳制。
    #[must_use]
    pub fn configured(
        kind: VoiceFilterKind,
        cutoff_hz: f32,
        resonance_db: f32,
        sample_rate: f32,
    ) -> VoiceFilter {
        if cutoff_hz.is_nan() {
            return VoiceFilter::BYPASSED;
        }
        let sample_rate = sanitise_sample_rate(sample_rate);
        let freq = cutoff_hz.clamp(MIN_CUTOFF_HZ, sample_rate * MAX_CUTOFF_FRACTION);
        let db = if resonance_db.is_finite() {
            resonance_db.clamp(RESONANCE_DB_RANGE.0, RESONANCE_DB_RANGE.1)
        } else {
            0.0
        };
        let sections = kind.sections();
        // 级联均分：每节 dB = 总 dB / 节数 ⇒ f0 处总增益（各节 Q 之积）
        // 恰为 10^(总 dB/20)，与单节口径一致。
        let q = 10.0f32.powf(db / f32::from(sections as u8) / 20.0);
        let mut filter = VoiceFilter::BYPASSED;
        filter.active = sections as u8;
        // ⭐ 自带配置（retune 复用同一份 (类型, 谐振, 采样率)；存的是
        // **归一后**的值 ⇒ retune 重跑 `configured` 幂等、系数逐位相同）。
        filter.kind_code = kind.code();
        filter.resonance_db = db;
        filter.sample_rate = sample_rate;
        for section in filter.sections.iter_mut().take(sections) {
            match kind {
                VoiceFilterKind::Lowpass1p => section.lowpass_1p(sample_rate, freq),
                VoiceFilterKind::Highpass1p => section.highpass_1p(sample_rate, freq),
                VoiceFilterKind::Lowpass2p
                | VoiceFilterKind::Lowpass4p
                | VoiceFilterKind::Lowpass6p => section.lowpass(sample_rate, freq, q),
                VoiceFilterKind::Highpass2p
                | VoiceFilterKind::Highpass4p
                | VoiceFilterKind::Highpass6p => section.highpass(sample_rate, freq, q),
                VoiceFilterKind::Bandpass2p => section.bandpass(sample_rate, freq, q),
                VoiceFilterKind::Bandreject2p => section.bandreject(sample_rate, freq, q),
            }
        }
        filter
    }

    /// 是否直通（没有有效节）。
    #[must_use]
    pub fn is_bypassed(&self) -> bool {
        self.active == 0
    }

    /// ⭐ **调制入口**（SFZ 包络族片，`fileg_*` 的落点）：换截止、**保状态**。
    ///
    /// 用自带的 (类型, 谐振, 采样率) 按新截止重算系数（与 [`Self::configured`]
    /// **同一份**系数代码与钳制），然后把三节的递推状态 `(v1, v2)` 原样装回
    /// ⇒ 包络扫频时没有系数重置的爆音/断层。直通（`active == 0`）或新截止
    /// 非有限 ⇒ 什么都不做（⛔ 不让 NaN 进递推，与 `configured` 的口径一致）。
    ///
    /// ⚠ 调用方的节流义务：系数重算含 `sin_cos`/`powf`，只应在目标截止
    /// **真的变了**时调用（引擎在电平变化时才调，恒定段零重算）。
    pub fn retune_cutoff(&mut self, cutoff_hz: f32) {
        if self.active == 0 || !cutoff_hz.is_finite() {
            return;
        }
        let Some(kind) = VoiceFilterKind::from_code(self.kind_code) else {
            return;
        };
        let states = [
            (self.sections[0].v1, self.sections[0].v2),
            (self.sections[1].v1, self.sections[1].v2),
            (self.sections[2].v1, self.sections[2].v2),
        ];
        *self = VoiceFilter::configured(kind, cutoff_hz, self.resonance_db, self.sample_rate);
        for (section, (v1, v2)) in self.sections.iter_mut().zip(states) {
            section.v1 = v1;
            section.v2 = v2;
        }
    }

    /// 复位为直通（退休一个声部槽时调用；与重新 `configured` 等价于换新）。
    pub fn reset(&mut self) {
        *self = VoiceFilter::BYPASSED;
    }

    /// 逐样本处理一个单声道样本。直通时**原样返回**（逐位恒等）。
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        if self.active == 0 {
            return x;
        }
        let mut y = x;
        for section in self.sections.iter_mut().take(usize::from(self.active)) {
            y = section.process(y);
        }
        y
    }

    /// 本滤波器在 `freq`（Hz）处的幅频响应（dB），供测试与诊断
    /// （逐节查表式计算，⛔ 不进逐样本路径）。
    #[must_use]
    pub fn response_db(&self, freq: f32, sample_rate: f32) -> f32 {
        if self.active == 0 {
            return 0.0;
        }
        let sr = sanitise_sample_rate(sample_rate);
        let f = freq.clamp(1.0, sr * 0.499);
        let w = TAU * (f / sr);
        let (sin_w, cos_w) = w.sin_cos();
        let (sin_2w, cos_2w) = (2.0 * w).sin_cos();
        let mut total_db = 0.0f32;
        for section in self.sections.iter().take(usize::from(self.active)) {
            let num_re = section.b0 + section.b1 * cos_w + section.b2 * cos_2w;
            let num_im = -section.b1 * sin_w - section.b2 * sin_2w;
            let den_re = 1.0 + section.a1 * cos_w + section.a2 * cos_2w;
            let den_im = -section.a1 * sin_w - section.a2 * sin_2w;
            let num_sq = num_re * num_re + num_im * num_im;
            let den_sq = den_re * den_re + den_im * den_im;
            if den_sq <= 1e-12 {
                continue;
            }
            let mag_sq = num_sq / den_sq;
            if mag_sq > 1e-12 {
                total_db += 10.0 * mag_sq.log10();
            } else {
                total_db += -120.0;
            }
        }
        total_db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// 尺寸钉：3 节 × 28 B ＋ `active`/`kind_code` 各 1 B ＋ 对齐 2 B ＋
    /// 配置两标量 × 4 B = **96 B**（滤波静态族时 88 B；包络族片自带配置
    /// 三标量里的类型码占的是原对齐空隙，净增只有谐振/采样率 8 B）。
    #[test]
    fn voice_filter_size_is_pinned() {
        assert_eq!(std::mem::size_of::<VoiceFilter>(), 96);
    }

    /// 第一判据：直通逐位恒等（含符号位与 NaN 之外的任意位型）。
    #[test]
    fn bypass_is_bit_exact() {
        let mut filter = VoiceFilter::BYPASSED;
        assert!(filter.is_bypassed());
        for x in [0.0, -0.0, 1.0, -1.0, 0.1, -123.456, f32::MIN_POSITIVE] {
            assert_eq!(filter.process(x).to_bits(), x.to_bits());
        }
        // 直通的响应恒 0 dB。
        assert_eq!(filter.response_db(1_000.0, SR), 0.0);
    }

    /// 低通压高频、放行低频；高通反之（2p，resonance 0）。
    #[test]
    fn lowpass_and_highpass_shape_the_spectrum() {
        let lp = VoiceFilter::configured(VoiceFilterKind::Lowpass2p, 1_000.0, 0.0, SR);
        assert!(lp.response_db(100.0, SR) > -1.0, "低频应近乎无损");
        assert!(lp.response_db(8_000.0, SR) < -30.0, "高频应被压下");
        let hp = VoiceFilter::configured(VoiceFilterKind::Highpass2p, 1_000.0, 0.0, SR);
        assert!(hp.response_db(8_000.0, SR) > -1.0);
        assert!(hp.response_db(100.0, SR) < -30.0);
    }

    /// 谐振语义：截止频率处的增益 ≈ resonance dB（单节与级联同口径）。
    #[test]
    fn resonance_is_the_gain_at_cutoff() {
        for kind in [
            VoiceFilterKind::Lowpass2p,
            VoiceFilterKind::Lowpass4p,
            VoiceFilterKind::Lowpass6p,
        ] {
            let filter = VoiceFilter::configured(kind, 1_000.0, 12.0, SR);
            let at_cutoff = filter.response_db(1_000.0, SR);
            assert!(
                (at_cutoff - 12.0).abs() < 0.5,
                "{kind:?} 在截止处应约 +12 dB，实测 {at_cutoff}"
            );
        }
    }

    /// 级联更陡：6p 在阻带比 2p 压得更深。
    #[test]
    fn cascades_are_steeper() {
        let lp2 = VoiceFilter::configured(VoiceFilterKind::Lowpass2p, 1_000.0, 0.0, SR);
        let lp6 = VoiceFilter::configured(VoiceFilterKind::Lowpass6p, 1_000.0, 0.0, SR);
        assert!(lp6.response_db(4_000.0, SR) < lp2.response_db(4_000.0, SR) - 20.0);
    }

    /// 带通只放行中心附近；带阻只压中心。
    #[test]
    fn bandpass_and_bandreject() {
        let bp = VoiceFilter::configured(VoiceFilterKind::Bandpass2p, 1_000.0, 0.0, SR);
        assert!(bp.response_db(1_000.0, SR) > -3.0);
        assert!(bp.response_db(100.0, SR) < -15.0);
        assert!(bp.response_db(10_000.0, SR) < -15.0);
        let br = VoiceFilter::configured(VoiceFilterKind::Bandreject2p, 1_000.0, 0.0, SR);
        assert!(br.response_db(1_000.0, SR) < -30.0);
        assert!(br.response_db(100.0, SR) > -3.0);
    }

    /// 一阶低通：直流增益 1、高频渐衰（斜率比二阶缓）。
    #[test]
    fn one_pole_lowpass() {
        let mut filter = VoiceFilter::configured(VoiceFilterKind::Lowpass1p, 1_000.0, 0.0, SR);
        // 阶跃响应单调趋近 1（直流增益 1）。
        let mut y = 0.0;
        for _ in 0..48_000 {
            y = filter.process(1.0);
        }
        assert!((y - 1.0).abs() < 1e-3, "阶跃终值应趋近 1，实测 {y}");
        assert!(filter.response_db(8_000.0, SR) < -15.0);
        assert!(
            filter.response_db(8_000.0, SR) > -25.0,
            "一阶在 8 kHz 应只压约 18 dB"
        );
    }

    /// 逐样本稳定性：极端谐振（40 dB）＋脉冲输入，输出有界、无 NaN。
    #[test]
    fn impulse_response_decays_without_nan() {
        let mut filter = VoiceFilter::configured(VoiceFilterKind::Lowpass2p, 5_000.0, 40.0, SR);
        let mut peak_tail = 0.0f32;
        for i in 0..48_000 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let y = filter.process(x);
            assert!(y.is_finite(), "第 {i} 帧输出非有限：{y}");
            if i > 24_000 {
                peak_tail = peak_tail.max(y.abs());
            }
        }
        assert!(peak_tail < 0.5, "后半程峰值应衰减，实测 {peak_tail}");
    }

    /// 截止越过 Nyquist 时钳到 0.495×sr：低通近乎全通（但不是直通位型）。
    #[test]
    fn cutoff_above_nyquist_is_clamped_not_bypassed() {
        let filter = VoiceFilter::configured(VoiceFilterKind::Lowpass2p, 96_000.0, 0.0, SR);
        assert!(!filter.is_bypassed());
        assert!(filter.response_db(1_000.0, SR) > -1.0);
    }

    /// NaN 截止 ⇒ 直通（防御性归一，模块文档明记）。
    #[test]
    fn nan_cutoff_falls_back_to_bypass() {
        let filter = VoiceFilter::configured(VoiceFilterKind::Lowpass2p, f32::NAN, 0.0, SR);
        assert!(filter.is_bypassed());
    }
}
