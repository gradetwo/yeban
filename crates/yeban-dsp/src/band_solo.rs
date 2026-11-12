//! 频段独奏（Band-Solo）监听滤波器（设计稿 `yeban-songmap-design.md` §7.3 P1/P2）。
//!
//! 面板上按住 Alt 点频率层某一行（如 Low-Mid 250–500 Hz）时，监听总线临时挂上
//! 本滤波器，只放过该频段、其它频段压下去，直接听该频段为什么发亮。再点一次
//! （同一频段）或换频段即切换 / 关闭。
//!
//! 实现：复用 [`crate::shaping`] 的 RBJ 二阶带通（`Biquad::bandpass`，峰值
//! 0 dB，`q` 定带宽）。中心频率取频段几何平均 `sqrt(lo·hi)`，`Q = fc / (hi − lo)`
//! ⇒ −3 dB 点恰好落在频段两端。滤波器是**监听侧**的，只作用在送往声卡的主总线
//! 输出块上（[`yeban_engine::rt::EngineRuntime::process_quantum`]），不进
//! 渲染、不进录音、不改工程。
//!
//! 实时纪律（与本 crate 其它器件一致）：`process_in_place` 内零堆分配；
//! 采样率在构造期给定；`set_band` 只换系数并清状态（换频段时旧递推不带入新频段，
//! 避免切换爆音）；无音频设备时滤波器照常存在、只是没有声音可滤（控制面模式下
//! `process_quantum` 仍被墙钟泵送驱动，状态与有设备时一致）。

use crate::shaping::Biquad;
use crate::song_overview::BAND_DEFINITIONS_HZ;

/// 频段独奏的频段数（与总览图频率层一致：8 频段，Sub → Air）。
pub const BAND_SOLO_COUNT: usize = 8;

/// 独奏滤波器缺省采样率（Hz）：调用方给的采样率非法（非有限 / ≤ 0）时回退到它。
const DEFAULT_SAMPLE_RATE_HZ: f32 = 48_000.0;

/// 第 `band` 频段的中心频率（Hz，几何平均 `sqrt(lo·hi)`）。
///
/// 频段定义与总览图频率层同一份表（[`BAND_DEFINITIONS_HZ`]，设计稿 §4.1）。
/// 越界下标返回 `None`。
#[must_use]
pub fn band_center_hz(band: usize) -> Option<f32> {
    let [lo, hi] = *BAND_DEFINITIONS_HZ.get(band)?;
    Some(libm::sqrtf(lo * hi))
}

/// 第 `band` 频段的品质因数 `Q = fc / (hi − lo)`。
///
/// −3 dB 点恰好落在频段两端：独奏该频段时，相邻频段在交界处正好 −3 dB，
/// 听到的就是“这个频段本身”，不是被邻段漏进来的声音。
/// 越界下标返回 `None`。
#[must_use]
pub fn band_q(band: usize) -> Option<f32> {
    let [lo, hi] = *BAND_DEFINITIONS_HZ.get(band)?;
    let fc = libm::sqrtf(lo * hi);
    Some(fc / (hi - lo))
}

/// 立体声带通独奏滤波器（监听侧，一只实例同时滤左右声道）。
///
/// 未激活（`band() == None`）时 `process_in_place` 是空操作，输出与输入逐位一致；
/// 激活时两声道各走同一节 RBJ 带通（状态独立，系数相同）。
pub struct BandSoloFilter {
    filter: Biquad,
    sample_rate: f32,
    band: Option<usize>,
}

impl BandSoloFilter {
    /// 新建一个未激活的独奏滤波器。
    ///
    /// `sample_rate` 非法（非有限 / ≤ 0）时回退到 48 kHz，⛔ 不 panic、不记 NaN 系数。
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            DEFAULT_SAMPLE_RATE_HZ
        };
        Self {
            filter: Biquad::identity(),
            sample_rate,
            band: None,
        }
    }

    /// 构造期给定的采样率（Hz）。
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// 当前独奏的频段（`None` = 未激活，直通）。
    #[must_use]
    pub fn band(&self) -> Option<usize> {
        self.band
    }

    /// 是否已激活（独奏中）。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.band.is_some()
    }

    /// 设定独奏频段：`Some(band)` 激活（换频段时清递推状态），`None` 关闭（直通）。
    ///
    /// 越界下标（`>= BAND_SOLO_COUNT`）按 `None` 处理：⛔ 不配非法系数、不 panic。
    /// 中心频率钳到奈奎斯特之内（采样率很低时 Air 段中心会超限，钳住才有意义）。
    pub fn set_band(&mut self, band: Option<usize>) {
        let Some(band) = band.filter(|&b| b < BAND_SOLO_COUNT) else {
            self.band = None;
            self.filter = Biquad::identity();
            return;
        };
        let Some(fc) = band_center_hz(band) else {
            self.band = None;
            self.filter = Biquad::identity();
            return;
        };
        let Some(q) = band_q(band) else {
            self.band = None;
            self.filter = Biquad::identity();
            return;
        };
        let fc = fc.clamp(1.0, self.sample_rate * 0.499);
        self.filter.bandpass(self.sample_rate, fc, q);
        self.filter.clear_state();
        self.band = Some(band);
    }

    /// 就地滤一对左右声道缓冲（长度取两者较短者）。
    ///
    /// 未激活时是空操作（不读不写，逐位直通）；实时路径零分配。
    pub fn process_in_place(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.band.is_none() {
            return;
        }
        let frames = left.len().min(right.len());
        self.filter.process_in_place(left, right, frames);
    }

    /// 当前系数在 `freq_hz` 处的幅频响应（dB；未激活时为 0 dB）。
    ///
    /// 诊断 / 测试用：实时路径不调用它。
    #[must_use]
    pub fn response_db(&self, freq_hz: f32) -> f32 {
        if self.band.is_none() {
            0.0
        } else {
            self.filter.response_db(freq_hz, self.sample_rate)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 已知答案：8 频段的中心频率（几何平均）与 Q（`fc / 带宽`）。
    #[test]
    fn centers_and_q_are_geometric_means() {
        let expected_centers = [
            42.4264, 150.0, 353.5534, 707.1068, 1414.2136, 3162.2777, 7_071.068, 14142.136,
        ];
        for (band, &expected) in expected_centers.iter().enumerate() {
            let center = band_center_hz(band).expect("频段下标 0..8 必须有中心频率");
            assert!(
                (center - expected).abs() < 0.01,
                "频段 {band} 中心频率应为 {expected} Hz，实际 {center}"
            );
            let [lo, hi] = BAND_DEFINITIONS_HZ[band];
            let q = band_q(band).expect("频段下标 0..8 必须有 Q");
            assert!(
                (q - expected / (hi - lo)).abs() < 1e-4,
                "频段 {band} Q 应为 {}，实际 {q}",
                expected / (hi - lo)
            );
        }
        assert_eq!(band_center_hz(8), None, "越界频段无中心频率");
        assert_eq!(band_q(8), None, "越界频段无 Q");
    }

    /// 幅频响应：中心处约 0 dB，十倍频程外压 20 dB 以上（RBJ 二阶带通的形状）。
    #[test]
    fn response_is_unity_at_center_and_rejects_a_decade_away() {
        let mut filter = BandSoloFilter::new(48_000.0);
        filter.set_band(Some(3));
        let center = band_center_hz(3).expect("频段 3 有中心频率");
        assert!(
            filter.response_db(center).abs() < 0.5,
            "中心 {center} Hz 处响应应约 0 dB，实际 {}",
            filter.response_db(center)
        );
        assert!(
            filter.response_db(center / 10.0) < -20.0,
            "低十倍频处应压 20 dB 以上，实际 {}",
            filter.response_db(center / 10.0)
        );
        assert!(
            filter.response_db(center * 10.0) < -20.0,
            "高十倍频处应压 20 dB 以上，实际 {}",
            filter.response_db(center * 10.0)
        );
    }

    /// 正弦实测：中心频率的正弦基本无损通过，远离中心的正弦被压掉。
    #[test]
    fn sine_at_center_passes_and_far_sine_is_rejected() {
        fn sine_rms_ratio(band: usize, freq_hz: f64, sample_rate: f32, len: usize) -> f32 {
            let mut filter = BandSoloFilter::new(sample_rate);
            filter.set_band(Some(band));
            let mut input = Vec::with_capacity(len);
            for i in 0..len {
                let phase = std::f64::consts::TAU * freq_hz * i as f64 / f64::from(sample_rate);
                input.push((0.5 * phase.sin()) as f32);
            }
            let mut left = input.clone();
            let mut right = input.clone();
            filter.process_in_place(&mut left, &mut right);
            // 前一半是建立瞬态，丢掉；以后一半算输出 RMS / 输入 RMS
            // （正弦 RMS = 幅值 / √2，⛔ 不是幅值本身）。
            let settled_in = &input[len / 2..];
            let settled_out = &left[len / 2..];
            let rms = |buf: &[f32]| {
                libm::sqrtf(buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32)
            };
            rms(settled_out) / rms(settled_in)
        }
        let ratio = sine_rms_ratio(3, 707.106_78, 48_000.0, 9600);
        assert!(
            (0.9..=1.1).contains(&ratio),
            "707 Hz 正弦过 Mid 段（中心 707 Hz）应基本无损，RMS 比实际 {ratio}"
        );
        // 707 Hz 距 Bass 段中心 150 Hz 只有 0.67 个十倍频程：二阶带通每十倍频程
        // 只压 20 dB，解析响应约 −12.7 dB（RMS 比约 0.23）—— 阈值须按解析值定，
        // ⛔ 不能拍一个 0.05。断言两点：实测与解析响应一致（±0.03），且确实被压掉
        // （> 10 dB）。更远的 Air 段（中心 14142 Hz，1.3 个十倍频程）才压到 0.05 下。
        let mut far = BandSoloFilter::new(48_000.0);
        far.set_band(Some(1));
        let predicted = 10f32.powf(far.response_db(707.106_78) / 20.0);
        let ratio = sine_rms_ratio(1, 707.106_78, 48_000.0, 9600);
        assert!(
            (ratio - predicted).abs() < 0.03,
            "707 Hz 过 Bass 段的实测 RMS 比 {ratio} 应与解析响应 {predicted} 一致"
        );
        assert!(
            ratio < 0.32,
            "707 Hz 正弦过 Bass 段（中心 150 Hz）应被压 10 dB 以上，RMS 比实际 {ratio}"
        );
        let ratio = sine_rms_ratio(7, 707.106_78, 48_000.0, 9600);
        assert!(
            ratio < 0.05,
            "707 Hz 正弦过 Air 段（中心 14142 Hz）应被压掉，RMS 比实际 {ratio}"
        );
    }

    /// 静音进、静音出；未激活时逐位直通；越界下标按关闭处理。
    #[test]
    fn silence_stays_silent_and_bypass_is_bit_identical() {
        let mut filter = BandSoloFilter::new(48_000.0);
        let mut left = vec![0.0f32; 128];
        let mut right = vec![0.0f32; 128];
        filter.set_band(Some(2));
        filter.process_in_place(&mut left, &mut right);
        assert!(left.iter().all(|&s| s == 0.0), "静音过带通仍须是静音");
        assert!(right.iter().all(|&s| s == 0.0), "静音过带通仍须是静音");

        let mut filter = BandSoloFilter::new(48_000.0);
        let input: Vec<f32> = (0..128).map(|i| (i as f32).sin() * 0.25).collect();
        let mut left = input.clone();
        let mut right = input.clone();
        filter.process_in_place(&mut left, &mut right);
        assert_eq!(left, input, "未激活时左声道须逐位直通");
        assert_eq!(right, input, "未激活时右声道须逐位直通");

        filter.set_band(Some(8));
        assert_eq!(filter.band(), None, "越界频段须按关闭处理");
        assert!(!filter.is_active(), "越界频段须未激活");
        filter.set_band(Some(5));
        assert_eq!(filter.band(), Some(5));
        filter.set_band(None);
        assert_eq!(filter.band(), None, "None 须关闭独奏");
        assert!(!filter.is_active(), "None 之后须未激活");
    }

    /// 非法采样率回退到 48 kHz，不 panic、不产 NaN 系数。
    #[test]
    fn illegal_sample_rate_falls_back_to_48k() {
        for bad in [0.0, -44100.0, f32::NAN, f32::INFINITY] {
            let mut filter = BandSoloFilter::new(bad);
            assert_eq!(
                filter.sample_rate(),
                48_000.0,
                "采样率 {bad} 应回退到 48 kHz"
            );
            filter.set_band(Some(3));
            assert!(
                filter.response_db(707.106_78).is_finite(),
                "采样率 {bad} 下中心响应须有限"
            );
        }
    }
}
