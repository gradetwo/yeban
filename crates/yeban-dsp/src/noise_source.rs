//! 噪声源器件（鼓机缺口 D5）：连续底噪（白 ↔ 粉红）＋爆裂声（crackle），
//! 种子可复现。[ARCH-DET-001]
//!
//! 这是插入链里唯一的**加性源**：其余器件都是处理器（进什么出什么的
//! 变形），本器件在静音输入上也产出信号——黑胶底噪的本质就是"没有节目
//! 时也在响"。因此它在链上的位置是**链头**（采样延迟之后、通道条之前）：
//! 噪声与节目从同一入口进入，之后被 EQ/压缩/磁带/碾碎/混响/延迟同样
//! 处理，如同噪声本来就录在素材里；末级限制器在其后，爆裂声的峰值仍被
//! 兜底（链序的完整论证见 `yeban-render` 的 `inserts.rs` 模块头 §6）。
//!
//! ## 确定性口径（Wang 硬规则：随机性必须种子绑定可复现）
//!
//! 全部随机数来自一台 [`Rng`]（xorshift32），种子由 `noise_seed` 参数
//! 给出。逐样本抽取顺序固定（见 [`NoiseSource::process_in_place`] 的
//! 文档），与块大小无关：同种子、同参数、同采样率 ⇒ 逐样本一致，两次
//! 渲染逐位相同（渲染级判据在 `yeban-render/tests/noise_chain_audio.rs`）。
//!
//! ## 声道口径
//!
//! - 连续底噪是**单声道**的（左右同一样本）：黑胶表面噪声近单声道，
//!   且单声道底噪不会给节目凭空加宽；
//! - 爆裂声**左右独立**排布（灰尘与划痕在左右沟槽里独立出现），两声道
//!   共用同一台 RNG、按固定顺序抽取。
//!
//! ## 恒等口径
//!
//! `level == 0` 且爆裂声关（`crackle_level == 0` 或 `crackle_density == 0`）
//! ⇒ **逐位恒等**：[`NoiseSource::process_in_place`] 整段跳过，连 RNG 与
//! 滤波器状态都不推进（加 `+0.0` 会翻转 `-0.0` 的符号位，所以"加上零"
//! 不是恒等，必须真跳过）。运行态据此不建器件（与瞬态塑形的恒等门同理）。

use crate::math::sanitise_sample_rate;
use crate::noise::{NoiseColour, NoiseGen, Rng};

/// 种子在 `f32` 参数面上可精确表示的最大值（2²⁴ − 1）：超过它的整数
/// `f32` 无法逐个区分，钳到这里保证"写进去的种子就是用到的种子"。
pub const MAX_NOISE_SEED: f32 = 16_777_215.0;

/// 爆裂声密度的上限（次/秒/声道）：240 已是"破损唱片"量级，再高就
/// 不是爆裂声而是连续噪声，交给底噪电平表达。
pub const MAX_CRACKLE_DENSITY: f32 = 240.0;

/// 爆裂声单个事件的包络时间常数（秒）：约 0.8 ms 的衰减尾——听感是
/// "啪"而不是"嗡"，且事件在 5 ms 量级内基本落回底噪，可数。
const CRACKLE_TAU_S: f32 = 0.000_8;

/// 粉红噪声的 RMS 归一化增益：本常数把粉红端拉回与白噪声同一量级，
/// 使 `color` 扫动时底噪响度大致恒定。标定（2026-11-19，wow 实测）：
/// 初值 1.9 时粉红/白 RMS 比为 +4.03 dB（0.09175 / 0.05771 = 1.5898），
/// 故除以 1.5898 得 1.195；判据 `pink_and_white_levels_are_comparable`
/// 守住 ±2 dB。
const PINK_NORM: f32 = 1.195;

/// 噪声源的控制量（一个块的有效值）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseParams {
    /// 连续底噪电平（线性增益）0..1；0 = 无底噪。
    pub level: f32,
    /// 噪声颜色 0..1：0 = 白噪声、1 = 粉红噪声，中间是两者的线性混合。
    pub color: f32,
    /// 爆裂声密度（次/秒/声道）0..=[`MAX_CRACKLE_DENSITY`]；0 = 无爆裂声。
    pub crackle_density: f32,
    /// 爆裂声电平（单个事件的峰值量级）0..1；0 = 无爆裂声。
    pub crackle_level: f32,
    /// 随机种子 0..=[`MAX_NOISE_SEED`]（写进器件时取整）。
    pub seed: f32,
}

impl NoiseParams {
    /// 缺省参数（底噪与爆裂声全关 = **逐位恒等**，见模块文档）。
    ///
    /// `color` 的缺省是 1（全粉红）：只写 `noise_level` 的器件得到的是
    /// 黑胶底噪的频谱形状，而不是刺耳的全频白噪声——这是刻意的产品
    /// 选择，写明在此。`seed` 缺省 0（`Rng` 内部会换成非零常数）。
    pub const DEFAULT: Self = Self {
        level: 0.0,
        color: 1.0,
        crackle_density: 0.0,
        crackle_level: 0.0,
        seed: 0.0,
    };

    /// 缺省参数。
    #[must_use]
    pub const fn new() -> Self {
        Self::DEFAULT
    }

    /// 与 [`NoiseSource::process_in_place`] 入口**同一次**钳制（入口直接
    /// 调本函数，⛔ 不复制第二份规则）：`level` 钳 0..1（非有限 → 0）、
    /// `color` 钳 0..1（非有限 → 1，即缺省色）、密度钳
    /// 0..=[`MAX_CRACKLE_DENSITY`]（非有限 → 0）、爆裂电平钳 0..1
    /// （非有限 → 0）、种子取整后钳 0..=[`MAX_NOISE_SEED`]（非有限 → 0）。
    #[must_use]
    pub fn sanitised(self) -> Self {
        Self {
            level: if self.level.is_finite() {
                self.level.clamp(0.0, 1.0)
            } else {
                0.0
            },
            color: if self.color.is_finite() {
                self.color.clamp(0.0, 1.0)
            } else {
                1.0
            },
            crackle_density: if self.crackle_density.is_finite() {
                self.crackle_density.clamp(0.0, MAX_CRACKLE_DENSITY)
            } else {
                0.0
            },
            crackle_level: if self.crackle_level.is_finite() {
                self.crackle_level.clamp(0.0, 1.0)
            } else {
                0.0
            },
            seed: if self.seed.is_finite() {
                self.seed.round().clamp(0.0, MAX_NOISE_SEED)
            } else {
                0.0
            },
        }
    }

    /// 钳制后的种子整数值。
    #[must_use]
    pub fn seed_u32(self) -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // 已钳到 0..=2^24-1 的整数值 f32，转换无损。
        let value = self.sanitised().seed as u32;
        value
    }

    /// 爆裂声是否在响（密度与电平都非零）。
    #[must_use]
    pub fn crackle_active(self) -> bool {
        let clean = self.sanitised();
        clean.crackle_density > 0.0 && clean.crackle_level > 0.0
    }

    /// 本参数是否**恒等**（底噪关且爆裂声关 ⇒ 输出逐位等于输入，
    /// 运行态可整级跳过）。
    #[must_use]
    pub fn is_identity(self) -> bool {
        let clean = self.sanitised();
        clean.level == 0.0 && !clean.crackle_active()
    }
}

impl Default for NoiseParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 噪声源状态（一台器件一个实例，无共享状态）。
#[derive(Clone, Debug)]
pub struct NoiseSource {
    /// 唯一的随机源（种子见 [`NoiseParams::seed`]）。
    rng: Rng,
    /// 粉红滤波状态（`NoiseGen` 的 Kellet 近似；白噪声样本由本器件喂入）。
    pink: NoiseGen,
    /// 左声道爆裂包络（衰减中的事件尾巴）。
    crackle_l: f32,
    /// 右声道爆裂包络。
    crackle_r: f32,
    /// 当前 RNG 的种子（块边界发现参数种子变了就重播种）。
    seed: u32,
}

impl NoiseSource {
    /// 按参数种子构造一台干净的噪声源。
    #[must_use]
    pub fn new(params: NoiseParams) -> Self {
        let seed = params.seed_u32();
        let mut pink = NoiseGen::new();
        pink.set_colour(NoiseColour::Pink);
        Self {
            rng: Rng::new(seed),
            pink,
            crackle_l: 0.0,
            crackle_r: 0.0,
            seed,
        }
    }

    /// 以新种子重播种并清空全部状态（粉红滤波与爆裂包络）。
    pub fn reseed(&mut self, seed: u32) {
        self.seed = seed;
        self.rng = Rng::new(seed);
        self.pink.reset();
        self.crackle_l = 0.0;
        self.crackle_r = 0.0;
    }

    /// 复位到"按当前种子全新构造"的状态（渲染重跑/定位回零的口径：
    /// 复位后与同种子新实例逐位同流）。
    pub fn reset(&mut self) {
        self.reseed(self.seed);
    }

    /// 当前种子。
    #[must_use]
    pub const fn seed(&self) -> u32 {
        self.seed
    }

    /// 原地把噪声**加**进一对声道缓冲（`left.len() == right.len()`）。
    ///
    /// 逐样本抽取顺序（确定性契约，⛔ 不得更改顺序或增减抽取）：
    /// 1. 白噪声样本 `w = rng.next_bipolar()`，喂粉红滤波得 `pink`；
    ///    连续底噪 `cont = ((1−color)·w + color·pink·PINK_NORM) · level`
    ///    （左右声道同一值）；
    /// 2. 爆裂声在响时：左声道事件判定 `u = rng.next_unit()`，命中
    ///    （`u < density / sample_rate`）再抽两次（幅度抖动、极性）给左
    ///    包络加一个冲激；右声道同序重复。包络先按
    ///    `exp(−1/(τ·sr))` 衰减、再叠加新冲激；
    /// 3. `left[i] += cont + crackle_l`、`right[i] += cont + crackle_r`。
    ///
    /// 参数为恒等值时整段跳过（不抽 RNG、不推状态、缓冲逐位不变）。
    /// 参数种子与当前种子不同 ⇒ 在块头重播种（块边界语义，与其余器件的
    /// `set_params` 时机一致）。
    pub fn process_in_place(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        params: NoiseParams,
        sample_rate: f32,
    ) {
        let params = params.sanitised();
        if params.is_identity() {
            return;
        }
        let seed = params.seed_u32();
        if seed != self.seed {
            self.reseed(seed);
        }
        let sr = sanitise_sample_rate(sample_rate);
        let crackle = params.crackle_active();
        let event_p = (params.crackle_density / sr).clamp(0.0, 1.0);
        let decay = (-1.0 / (CRACKLE_TAU_S * sr)).exp();
        let white_gain = (1.0 - params.color) * params.level;
        let pink_gain = params.color * PINK_NORM * params.level;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let w = self.rng.next_bipolar();
            let pink = self.pink.process(w, sr);
            let cont = w * white_gain + pink * pink_gain;
            self.crackle_l *= decay;
            self.crackle_r *= decay;
            if crackle {
                if self.rng.next_unit() < event_p {
                    let amp = 0.35 + 0.65 * self.rng.next_unit();
                    let sign = if self.rng.next_unit() < 0.5 {
                        -1.0
                    } else {
                        1.0
                    };
                    self.crackle_l += params.crackle_level * amp * sign;
                }
                if self.rng.next_unit() < event_p {
                    let amp = 0.35 + 0.65 * self.rng.next_unit();
                    let sign = if self.rng.next_unit() < 0.5 {
                        -1.0
                    } else {
                        1.0
                    };
                    self.crackle_r += params.crackle_level * amp * sign;
                }
            }
            *l += cont + self.crackle_l;
            *r += cont + self.crackle_r;
        }
    }
}

impl Default for NoiseSource {
    fn default() -> Self {
        Self::new(NoiseParams::DEFAULT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(params: NoiseParams, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut source = NoiseSource::new(params);
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        source.process_in_place(&mut left, &mut right, params, SR);
        (left, right)
    }

    fn rms(samples: &[f32]) -> f64 {
        let sum: f64 = samples.iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
        (sum / samples.len() as f64).sqrt()
    }

    /// 种子可复现：同种子两台实例逐样本一致；换种子必须变。
    #[test]
    fn same_seed_reproduces_bit_exactly_and_other_seed_differs() {
        let params = NoiseParams {
            level: 0.05,
            crackle_density: 6.0,
            crackle_level: 0.5,
            seed: 2026.0,
            ..NoiseParams::DEFAULT
        };
        let (l1, r1) = render(params, 8192);
        let (l2, r2) = render(params, 8192);
        assert_eq!(l1, l2, "同种子左声道必须逐位一致");
        assert_eq!(r1, r2, "同种子右声道必须逐位一致");
        let (l3, _) = render(
            NoiseParams {
                seed: 7.0,
                ..params
            },
            8192,
        );
        assert_ne!(l1, l3, "换种子输出必须改变");
    }

    /// 块大小无关：一次处理 8192 帧与分 16 块处理逐位一致（抽取顺序
    /// 是逐样本的，不许依赖分块）。
    #[test]
    fn output_does_not_depend_on_block_size() {
        let params = NoiseParams {
            level: 0.05,
            crackle_density: 6.0,
            crackle_level: 0.5,
            seed: 99.0,
            ..NoiseParams::DEFAULT
        };
        let (whole_l, whole_r) = render(params, 8192);
        let mut source = NoiseSource::new(params);
        let mut left = vec![0.0f32; 8192];
        let mut right = vec![0.0f32; 8192];
        for chunk in 0..16 {
            let range = chunk * 512..(chunk + 1) * 512;
            source.process_in_place(&mut left[range.clone()], &mut right[range], params, SR);
        }
        assert_eq!(whole_l, left);
        assert_eq!(whole_r, right);
    }

    /// 恒等参数：缓冲逐位不变（包括 `-0.0` 的符号位），且状态不推进
    /// （恒等块之后接非恒等块，与从头非恒等逐位一致）。
    #[test]
    fn identity_params_leave_the_buffer_and_the_stream_untouched() {
        let mut left = vec![-0.0f32, 0.25, -0.5];
        let mut right = vec![-0.0f32, -0.25, 0.5];
        let before_l: Vec<u32> = left.iter().map(|v| v.to_bits()).collect();
        let before_r: Vec<u32> = right.iter().map(|v| v.to_bits()).collect();
        let mut source = NoiseSource::new(NoiseParams::DEFAULT);
        source.process_in_place(&mut left, &mut right, NoiseParams::DEFAULT, SR);
        assert_eq!(
            left.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            before_l,
            "恒等参数不得改缓冲（含 -0.0 符号位）"
        );
        assert_eq!(
            right.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            before_r
        );
        // 状态不推进：先过 3 个恒等样本、再出声的流 == 从头出声的流的
        // 前 3 个样本之后的部分对不齐——恒等跳过意味着出声从流的第 0
        // 个样本开始，与一台新实例的前 3 个出声样本逐位一致。
        let loud = NoiseParams {
            level: 0.05,
            seed: 42.0,
            ..NoiseParams::DEFAULT
        };
        let mut l2 = vec![0.0f32; 3];
        let mut r2 = vec![0.0f32; 3];
        source.process_in_place(&mut l2, &mut r2, loud, SR);
        let (fresh_l, fresh_r) = render(loud, 3);
        assert_eq!(l2, fresh_l, "恒等块不得消耗随机流");
        assert_eq!(r2, fresh_r);
    }

    /// `reset` 后与同种子新实例逐位同流。
    #[test]
    fn reset_reproduces_a_fresh_source() {
        let params = NoiseParams {
            level: 0.05,
            crackle_density: 6.0,
            crackle_level: 0.5,
            seed: 1234.0,
            ..NoiseParams::DEFAULT
        };
        let mut used = NoiseSource::new(params);
        let mut l = vec![0.0f32; 1024];
        let mut r = vec![0.0f32; 1024];
        used.process_in_place(&mut l, &mut r, params, SR);
        used.reset();
        let mut l2 = vec![0.0f32; 1024];
        let mut r2 = vec![0.0f32; 1024];
        used.process_in_place(&mut l2, &mut r2, params, SR);
        let (fresh_l, fresh_r) = render(params, 1024);
        assert_eq!(l2, fresh_l);
        assert_eq!(r2, fresh_r);
    }

    /// 钳制口径：越界/非有限一律落回文档域（与器件入口同一次钳制）。
    #[test]
    fn sanitised_clamps_every_field() {
        let clean = NoiseParams {
            level: f32::NAN,
            color: f32::NAN,
            crackle_density: 999.0,
            crackle_level: -2.0,
            seed: 3.6,
        }
        .sanitised();
        assert_eq!(clean.level, 0.0);
        assert_eq!(clean.color, 1.0, "color 非有限回落缺省色 1");
        assert_eq!(clean.crackle_density, MAX_CRACKLE_DENSITY);
        assert_eq!(clean.crackle_level, 0.0);
        assert_eq!(clean.seed, 4.0, "种子取整");
        assert_eq!(
            NoiseParams {
                seed: 1.0e9,
                ..NoiseParams::DEFAULT
            }
            .seed_u32(),
            16_777_215
        );
        assert_eq!(
            NoiseParams {
                seed: -5.0,
                ..NoiseParams::DEFAULT
            }
            .seed_u32(),
            0
        );
        assert!(NoiseParams::DEFAULT.is_identity());
        assert!(
            !NoiseParams {
                level: 0.01,
                ..NoiseParams::DEFAULT
            }
            .is_identity()
        );
        assert!(
            !NoiseParams {
                crackle_density: 1.0,
                crackle_level: 0.5,
                ..NoiseParams::DEFAULT
            }
            .is_identity()
        );
        assert!(
            NoiseParams {
                crackle_density: 1.0,
                crackle_level: 0.0,
                ..NoiseParams::DEFAULT
            }
            .is_identity(),
            "爆裂电平为 0 时密度再大也是恒等"
        );
    }

    /// 颜色两端的响度可比：`PINK_NORM` 标定后，纯粉红与纯白在同一
    /// `level` 下的 RMS 相差不超过 2 dB（扫色不许伴随大幅响度跳变）。
    #[test]
    fn pink_and_white_levels_are_comparable() {
        let frames = 480_000;
        let white = NoiseParams {
            level: 0.1,
            color: 0.0,
            seed: 5.0,
            ..NoiseParams::DEFAULT
        };
        let pink = NoiseParams {
            color: 1.0,
            ..white
        };
        let (white_l, _) = render(white, frames);
        let (pink_l, _) = render(pink, frames);
        let ratio_db = 20.0 * (rms(&pink_l) / rms(&white_l)).log10();
        eprintln!(
            "white rms {:.5}, pink rms {:.5}, ratio {:+.2} dB",
            rms(&white_l),
            rms(&pink_l),
            ratio_db
        );
        assert!(
            ratio_db.abs() <= 2.0,
            "粉红/白噪声 RMS 差须在 ±2 dB 内，实测 {ratio_db:+.2} dB"
        );
        assert_ne!(white_l, pink_l, "两种颜色的样本流必须不同");
    }

    /// 爆裂声：事件可数且稀疏。10 秒、密度 10/秒/声道 ⇒ 每声道期望
    /// 100 次；用与渲染级判据同一口径数事件（越过 0.15、间隔 > 5 ms）。
    #[test]
    fn crackle_events_are_countable_and_sparse() {
        let params = NoiseParams {
            level: 0.0,
            crackle_density: 10.0,
            crackle_level: 0.6,
            seed: 2026.0,
            ..NoiseParams::DEFAULT
        };
        let frames = 480_000;
        let (left, right) = render(params, frames);
        for (name, samples) in [("L", &left), ("R", &right)] {
            let mut events = 0usize;
            let mut last = usize::MAX;
            for (index, sample) in samples.iter().enumerate() {
                if sample.abs() > 0.15 && (last == usize::MAX || index - last > 240) {
                    events += 1;
                    last = index;
                }
            }
            eprintln!("crackle {name}: {events} events in 10 s at density 10/s");
            assert!(
                (50..=160).contains(&events),
                "{name} 声道事件数须在期望 100 的合理带内，实测 {events}"
            );
            assert!(
                events < frames / 1000,
                "事件必须稀疏（平均间隔 > 1 ms 量级以上），实测 {events}"
            );
        }
        assert_ne!(left, right, "左右爆裂排布独立，不得逐样本相同");
    }

    /// 连续底噪是单声道的：无爆裂声时左右逐样本相同。
    #[test]
    fn continuous_floor_is_mono() {
        let params = NoiseParams {
            level: 0.05,
            seed: 3.0,
            ..NoiseParams::DEFAULT
        };
        let (left, right) = render(params, 4096);
        assert_eq!(left, right, "连续底噪左右必须同一样本（单声道口径）");
        assert!(rms(&left) > 0.0, "底噪必须真的在响");
    }
}
