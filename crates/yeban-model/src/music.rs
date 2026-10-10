//! 表现力 MIDI 音符与音乐性基础类型 [MODEL-AST-005]。
//!
//! 本模块是 960 PPQ 权威时钟上的**唯一**音符事实源：
//!
//! - [`MidiNote`] — 带概率触发、连击 (ratchet)、微时序 (micro timing)、滑音 (slide)、
//!   弯音曲线与音节/音素歌词的表现力音符。
//! - [`SlideConfig`] / [`CurveType`] — 滑音与曲线形状。
//! - [`MidiNote::triggers`] — **确定性**概率触发判定（同输入恒同输出，跨进程/跨重启一致）。
//!
//! ## 为什么触发判定必须是手写确定性哈希
//!
//! 规范要求 `probability` 结合 `rng_seed + note.id` 计算（`MODEL-AST-005`）。
//! 这里刻意**不引入**任何真随机数发生器：一旦触发判定依赖平台 RNG 或线程局部的
//! PRNG 状态，同一工程在两台机器上就会渲染出不同的音频，直接违反
//! `ARCH-DET-001`（L1/L2 声学确定性契约）与 `AGENTS.md` §2 红线 4 的精神
//! （跨进程、跨重启的确定性）。
//!
//! 实现采用 Steele 等人的 **splitmix64** 位混合函数（公有领域算法，无依赖、无 `unsafe`、
//! 无堆分配），把 `(rng_seed, note.id)` 混合为一个 64-bit 值，再取高 53 位映射到 `[0,1)`
//! 与 `probability` 比较。

use serde::{Deserialize, Serialize};

use crate::error::ModelError;
use crate::ids::EntityId;

/// MIDI 音高的合法上界（含），标准 128 半音。
pub const MIDI_PITCH_MAX: u8 = 127;

/// MIDI 力度的合法上界（含）。
pub const MIDI_VELOCITY_MAX: u8 = 127;

/// `ratchet` 连击数的合法下界（含）。
pub const RATCHET_MIN: u8 = 1;

/// `ratchet` 连击数的合法上界（含）。
pub const RATCHET_MAX: u8 = 16;

/// 微时值偏移的合法绝对值上界（含）：±240 tick = ±1/16 音符 @960 PPQ。
pub const MICRO_TIMING_MAX_ABS: i16 = 240;

/// 默认力度。
pub const DEFAULT_VELOCITY: u8 = 100;

/// 默认音符时值（一个四分音符 = 960 tick）[MODEL-AST-001]。
pub const DEFAULT_DURATION_TICKS: u64 = crate::ids::PPQ;

/// 曲线形状 [MODEL-AST-005]。
///
/// 同时被滑音 (slide) 与自动化点 (automation point) 复用。
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum CurveType {
    /// 线性插值。
    #[default]
    Linear,
    /// 指数插值。
    Exponential,
    /// 对数插值。
    Logarithmic,
    /// S 型（缓入缓出）插值。
    SCurve,
}

impl CurveType {
    /// 把归一化位置 `t ∈ [0,1]` 映射为归一化插值权重 `u ∈ [0,1]`。
    ///
    /// 这是夜半**唯一**的曲线口径：自动化泳道求值（`AutomationLane::value_at`）与
    /// 滑音/弯音求值都调这一个函数，下游（渲染 / 引擎 / 界面）**不得**各写一份
    /// （两份实现必然漂移，见 ADR-0001 D28）。
    ///
    /// ## 公式（规范只给了四个名字，没有给公式 —— 口径由本线裁决并钉在判据里）
    ///
    /// | 变体 | `u(t)` |
    /// | :--- | :--- |
    /// | `Linear` | `t` |
    /// | `Exponential` | `t²` |
    /// | `Logarithmic` | `t·(2−t)` |
    /// | `SCurve` | `t²·(3−2t)` |
    ///
    /// 三个非平凡形状只用 `+` `-` `*`：**没有超越函数**，因此逐位可复现
    /// （`ARCH-DET-001`；按 ADR-0001 D32 的分类属于"IEEE 精确类"，跨架构**零容差**）。
    /// 四者都满足 `u(0)=0`、`u(1)=1`、单调不减，因此曲线**精确穿过两个端点**，
    /// 不会在采样点上产生跳变。
    ///
    /// `t` 越界一律钳到 `[0,1]`。求值入口不会产生 `NaN`（插值分母 `span > 0`），
    /// 而若调用方硬塞 `NaN`，它会原样传出 `NaN` —— 这比"悄悄当成 0"更诚实：
    /// 模型层绝不把非有限值修成一个看起来合法的数值。
    #[must_use]
    pub fn ease(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Exponential => t * t,
            Self::Logarithmic => t * (2.0 - t),
            Self::SCurve => t * t * (3.0 - 2.0 * t),
        }
    }
}

/// 滑音配置 [MODEL-AST-005]。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlideConfig {
    /// 滑音的时值 (tick)，必须非零。
    pub duration_ticks: u64,
    /// 滑音结束时的目标音高 (0..=127)。
    pub target_pitch: u8,
    /// 滑音曲线形状。
    pub curve: CurveType,
}

impl SlideConfig {
    /// 校验滑音配置的取值范围 [MODEL-AST-005]。
    ///
    /// # Errors
    ///
    /// - `target_pitch` 越界 → [`ModelError::PitchOutOfRange`]；
    /// - `duration_ticks` 为零 → [`ModelError::ZeroDuration`]。
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.target_pitch > MIDI_PITCH_MAX {
            return Err(ModelError::PitchOutOfRange {
                value: u16::from(self.target_pitch),
            });
        }
        if self.duration_ticks == 0 {
            return Err(ModelError::ZeroDuration);
        }
        Ok(())
    }
}

impl Default for SlideConfig {
    /// 默认滑音：一个八分音符的线性滑向当前音高（由调用方随后覆盖 `target_pitch`）。
    fn default() -> Self {
        Self {
            duration_ticks: crate::ids::PPQ / 2,
            target_pitch: 0,
            curve: CurveType::Linear,
        }
    }
}

/// 表现力 MIDI 音符 [MODEL-AST-005]。
///
/// 所有字段都是持久化文档的一部分；集合概不使用哈希容器，
/// 本结构本身不持有任何集合（`pitch_bend_curve` 是**有序** `Vec`，按 tick 升序）。
///
/// ## 表现力字段的 `#[serde(default)]` 是**语义**，不是兼容 [ADR-0001 D43]
///
/// - `probability` / `ratchet` / `micro_timing_ticks` / `slide` / `syllable` 是
///   `Option<T>`：`None` 是模型自己定义的一等状态（"必然触发"/"等价于 1"/"无偏移"/
///   "无滑音"/"无歌词"），D43 第 2 条明确豁免；
/// - `pitch_bend_curve` / `phonemes` 与 `skip_serializing_if = "Vec::is_empty"` **对偶**：
///   空集在序列化时本来就不落盘（稀疏编码，每个音符省几十字节），因此"缺键"与本写入器
///   自己的输出逐字节一致 —— 若这里要求必需，本写入器写出的文档就会被本读取器拒绝。
///   这对字段的宽容读是**格式自身的对偶性**，与"为了旧文件还能读"无关。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MidiNote {
    /// 音符身份 [MODEL-AST-001]。
    pub id: EntityId,
    /// 起始 tick (960 PPQ 整数时钟，绝不用浮点)。
    pub start_tick: u64,
    /// 时值 (tick)，必须非零。
    pub duration_ticks: u64,
    /// 音高 0..=127。
    pub pitch: u8,
    /// 力度 0..=127。
    pub velocity: u8,
    /// 触发概率：`None` = 必然触发；`Some(0.0)` = 永不触发；必须在 0.0..=1.0 且有限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability: Option<f32>,
    /// 连击次数 1..=16；`None` 等价于 1。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratchet: Option<u8>,
    /// 微时值偏移 -240..=240 tick；`None` 等价于 0。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub micro_timing_ticks: Option<i16>,
    /// 滑音配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide: Option<SlideConfig>,
    /// 弯音曲线 `(tick, cents)` 序列，按 tick 升序；空表示无弯音。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pitch_bend_curve: Vec<(u64, i16)>,
    /// 歌词音节。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syllable: Option<String>,
    /// 音素序列（歌声合成的对齐输入）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phonemes: Vec<String>,
}

impl MidiNote {
    /// 构造一个确定性的、**合法**的音符（便于测试与默认夹具）。
    #[must_use]
    pub fn new(id: EntityId, start_tick: u64, pitch: u8, duration_ticks: u64) -> Self {
        Self {
            id,
            start_tick,
            duration_ticks,
            pitch,
            velocity: DEFAULT_VELOCITY,
            probability: None,
            ratchet: None,
            micro_timing_ticks: None,
            slide: None,
            pitch_bend_curve: Vec::new(),
            syllable: None,
            phonemes: Vec::new(),
        }
    }

    /// 校验全部取值范围 [MODEL-AST-005]。
    ///
    /// 校验项与规范一一对应：
    /// `pitch`/`velocity` ∈ 0..=127；`probability` ∈ 0.0..=1.0 且有限；
    /// `ratchet` ∈ 1..=16；`micro_timing_ticks` ∈ -240..=240；`duration_ticks` ≠ 0；
    /// 滑音配置合法。
    ///
    /// # Errors
    ///
    /// 任意一项越界即返回对应的 [`ModelError`] 变体。
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.pitch > MIDI_PITCH_MAX {
            return Err(ModelError::PitchOutOfRange {
                value: u16::from(self.pitch),
            });
        }
        if self.velocity > MIDI_VELOCITY_MAX {
            return Err(ModelError::VelocityOutOfRange {
                value: u16::from(self.velocity),
            });
        }
        if let Some(probability) = self.probability {
            // `RangeInclusive::contains` 对 NaN 恒为 false，因此 NaN 会被这里拦下。
            if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
                return Err(ModelError::ProbabilityOutOfRange { value: probability });
            }
        }
        if let Some(ratchet) = self.ratchet
            && !(RATCHET_MIN..=RATCHET_MAX).contains(&ratchet)
        {
            return Err(ModelError::RatchetOutOfRange { value: ratchet });
        }
        if let Some(micro_timing) = self.micro_timing_ticks
            && !(-MICRO_TIMING_MAX_ABS..=MICRO_TIMING_MAX_ABS).contains(&micro_timing)
        {
            return Err(ModelError::MicroTimingOutOfRange {
                value: i32::from(micro_timing),
            });
        }
        if self.duration_ticks == 0 {
            return Err(ModelError::ZeroDuration);
        }
        if let Some(slide) = self.slide {
            slide.validate()?;
        }
        Ok(())
    }

    /// **确定性**概率触发判定 [MODEL-AST-005, ARCH-DET-001]。
    ///
    /// - `probability == None` 或 `Some(1.0)` → 必然触发 (返回 `true`)；
    /// - `Some(0.0)` → 永不触发 (返回 `false`)；
    /// - `Some(p)` → 对 `(rng_seed, self.id)` 做 splitmix64 稳定哈希，取 `[0,1)` 与 `p` 比较。
    ///
    /// 同一 `(rng_seed, id, probability)` 三元组**恒**给出同一结果：与调用次数、调用顺序、
    /// 线程、进程、平台、编译优化级别全部无关。哈希只使用 `id`，因此修改
    /// `start_tick`/`velocity` 等字段不会改变触发结果（音符在时间轴上移动后仍保持
    /// 相同的触发判定，这正是制作人期望的"概率属于身份而不是位置"）。
    #[must_use]
    pub fn triggers(&self, rng_seed: u64) -> bool {
        match self.probability {
            None => true,
            Some(probability) if probability >= 1.0 => true,
            Some(probability) if probability <= 0.0 => false,
            Some(probability) => {
                let unit = trigger_unit_interval(rng_seed, self.id);
                // NaN 在此处恒为 false（永不触发），且 `validate()` 已把 NaN 判为非法。
                unit < f64::from(probability)
            }
        }
    }
}

impl Default for MidiNote {
    /// 默认音符**本身合法**：中音 C、力度 100、时值一个四分音符、必然触发。
    fn default() -> Self {
        Self::new(EntityId::default(), 0, 60, DEFAULT_DURATION_TICKS)
    }
}

/// 把 `(rng_seed, id)` 稳定映射到 `[0.0, 1.0)`。
///
/// 取 splitmix64 输出的高 53 位（`f64` 的有效精度位）再除以 `2^53`，
/// 因此映射无浮点精度损失、无偏置（53 位均匀分布）。
#[must_use]
fn trigger_unit_interval(rng_seed: u64, id: EntityId) -> f64 {
    const TWO_POW_53: f64 = 9_007_199_254_740_992.0;
    let mixed = trigger_hash(rng_seed, id);
    let mantissa = mixed >> 11;
    // `u64 -> f64` 在 53 位以内是精确的，这里显式转换避免任何隐式精度假设。
    #[allow(clippy::cast_precision_loss)]
    let value = mantissa as f64;
    value / TWO_POW_53
}

/// 对 `(rng_seed, id)` 做 splitmix64 稳定哈希 [MODEL-AST-005, ARCH-DET-001]。
///
/// ULID 是 128-bit，拆成高低两个 `u64` 后与种子一起混合；整个函数只有整数运算，
/// 无分配、无 `unsafe`、无平台相关行为，因此输出在所有受支持平台上逐位相同。
#[must_use]
fn trigger_hash(rng_seed: u64, id: EntityId) -> u64 {
    let raw: u128 = id.as_ulid().0;
    #[allow(clippy::cast_possible_truncation)]
    let low = raw as u64;
    #[allow(clippy::cast_possible_truncation)]
    let high = (raw >> 64) as u64;
    splitmix64(rng_seed ^ splitmix64(low ^ splitmix64(high)))
}

/// splitmix64 位混合函数（Steele et al., 公有领域）。
///
/// `pub(crate)` 而非私有：`ops` 的属性测试需要一个**同一个**确定性 PRNG 来生成
/// 可复现的随机操作序列，重复实现一份就会有两套行为。
#[must_use]
pub(crate) const fn splitmix64(seed: u64) -> u64 {
    let z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn note_with_id(index: u128) -> MidiNote {
        MidiNote::new(EntityId::from_ulid(ulid::Ulid::from(index)), 0, 60, 960)
    }

    #[test]
    fn default_note_is_legal() {
        let note = MidiNote::default();
        assert_eq!(note.validate(), Ok(()));
        assert_eq!(note.duration_ticks, DEFAULT_DURATION_TICKS);
        assert!(note.triggers(0));
    }

    #[test]
    fn pitch_out_of_range_is_rejected() {
        let note = MidiNote {
            pitch: 128,
            ..MidiNote::default()
        };
        assert_eq!(
            note.validate(),
            Err(ModelError::PitchOutOfRange { value: 128 })
        );
        let max = MidiNote {
            pitch: MIDI_PITCH_MAX,
            ..MidiNote::default()
        };
        assert_eq!(max.validate(), Ok(()));
    }

    #[test]
    fn velocity_out_of_range_is_rejected() {
        let note = MidiNote {
            velocity: 128,
            ..MidiNote::default()
        };
        assert_eq!(
            note.validate(),
            Err(ModelError::VelocityOutOfRange { value: 128 })
        );
        let max = MidiNote {
            velocity: MIDI_VELOCITY_MAX,
            ..MidiNote::default()
        };
        assert_eq!(max.validate(), Ok(()));
    }

    #[test]
    fn probability_out_of_range_and_non_finite_are_rejected() {
        for bad in [1.5_f32, -0.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let note = MidiNote {
                probability: Some(bad),
                ..MidiNote::default()
            };
            // NaN != NaN，因此这里按位比较而不是 `assert_eq!`（否则 NaN 用例会假红）。
            match note.validate() {
                Err(ModelError::ProbabilityOutOfRange { value }) => assert_eq!(
                    value.to_bits(),
                    bad.to_bits(),
                    "probability {bad} must be rejected with the offending value"
                ),
                other => panic!("probability {bad} must be rejected, got {other:?}"),
            }
        }
        for good in [0.0_f32, 0.5, 1.0] {
            let note = MidiNote {
                probability: Some(good),
                ..MidiNote::default()
            };
            assert_eq!(
                note.validate(),
                Ok(()),
                "probability {good} must be accepted"
            );
        }
    }

    #[test]
    fn ratchet_out_of_range_is_rejected() {
        for bad in [0_u8, 17, 255] {
            let note = MidiNote {
                ratchet: Some(bad),
                ..MidiNote::default()
            };
            assert_eq!(
                note.validate(),
                Err(ModelError::RatchetOutOfRange { value: bad })
            );
        }
        for good in [RATCHET_MIN, 2, RATCHET_MAX] {
            let note = MidiNote {
                ratchet: Some(good),
                ..MidiNote::default()
            };
            assert_eq!(note.validate(), Ok(()));
        }
    }

    #[test]
    fn micro_timing_out_of_range_is_rejected() {
        for bad in [-241_i16, 241, i16::MIN, i16::MAX] {
            let note = MidiNote {
                micro_timing_ticks: Some(bad),
                ..MidiNote::default()
            };
            assert_eq!(
                note.validate(),
                Err(ModelError::MicroTimingOutOfRange {
                    value: i32::from(bad)
                })
            );
        }
        for good in [-MICRO_TIMING_MAX_ABS, 0, MICRO_TIMING_MAX_ABS] {
            let note = MidiNote {
                micro_timing_ticks: Some(good),
                ..MidiNote::default()
            };
            assert_eq!(note.validate(), Ok(()));
        }
    }

    #[test]
    fn zero_duration_is_rejected() {
        let note = MidiNote {
            duration_ticks: 0,
            ..MidiNote::default()
        };
        assert_eq!(note.validate(), Err(ModelError::ZeroDuration));
    }

    #[test]
    fn slide_validation_covers_both_bounds() {
        let bad_pitch = SlideConfig {
            target_pitch: 200,
            ..SlideConfig::default()
        };
        assert_eq!(
            bad_pitch.validate(),
            Err(ModelError::PitchOutOfRange { value: 200 })
        );

        let bad_duration = SlideConfig {
            duration_ticks: 0,
            ..SlideConfig::default()
        };
        assert_eq!(bad_duration.validate(), Err(ModelError::ZeroDuration));

        let note = MidiNote {
            slide: Some(bad_pitch),
            ..MidiNote::default()
        };
        assert!(note.validate().is_err(), "滑音错误必须冒泡到音符校验");
    }

    /// 滑音目标音高是**闭区间** `0..=127`：上端点 127 必须放行。
    ///
    /// 实测：把 `self.target_pitch > MIDI_PITCH_MAX` 改成 `>=` 时，全仓判据保持全绿
    /// —— 既有判据只钉住 200（区间外）被拒，没有钉住区间内的上端点。
    #[test]
    fn a_slide_may_target_the_top_pitch() {
        for target_pitch in [0_u8, 127] {
            assert_eq!(
                SlideConfig {
                    target_pitch,
                    ..SlideConfig::default()
                }
                .validate(),
                Ok(()),
                "目标音高 {target_pitch} 是闭区间的端点, 必须合法"
            );
        }
    }

    #[test]
    fn none_probability_always_triggers() {
        let note = MidiNote::default();
        for seed in [0_u64, 1, 42, u64::MAX] {
            assert!(note.triggers(seed), "None 必须必然触发 (seed {seed})");
        }
    }

    #[test]
    fn zero_probability_never_triggers() {
        for index in 0..64_u128 {
            let note = MidiNote {
                probability: Some(0.0),
                ..note_with_id(index)
            };
            for seed in [0_u64, 7, u64::MAX] {
                assert!(
                    !note.triggers(seed),
                    "Some(0.0) 必须永不触发 (id {index}, seed {seed})"
                );
            }
        }
    }

    #[test]
    fn full_probability_always_triggers() {
        for index in 0..64_u128 {
            let note = MidiNote {
                probability: Some(1.0),
                ..note_with_id(index)
            };
            assert!(note.triggers(0));
            assert!(note.triggers(u64::MAX));
        }
    }

    #[test]
    fn triggers_is_stable_across_calls_and_instances() {
        let note = MidiNote {
            probability: Some(0.5),
            ..note_with_id(12345)
        };
        let clone = note.clone();
        let first = note.triggers(99);
        for _ in 0..16 {
            assert_eq!(note.triggers(99), first, "同输入必须同输出");
            assert_eq!(clone.triggers(99), first, "等值实例必须同输出");
        }
        // 不同种子下允许不同结论，但同一批种子必须在两次遍历中完全一致。
        let seeds: Vec<u64> = (0..256_u64)
            .map(|s| s.wrapping_mul(2_654_435_761))
            .collect();
        let first_pass: Vec<bool> = seeds.iter().map(|&s| note.triggers(s)).collect();
        let second_pass: Vec<bool> = seeds.iter().map(|&s| note.triggers(s)).collect();
        assert_eq!(first_pass, second_pass);
    }

    #[test]
    fn trigger_hash_ignores_non_identity_fields() {
        let base = MidiNote {
            probability: Some(0.5),
            ..note_with_id(777)
        };
        let moved = MidiNote {
            start_tick: 123_456,
            velocity: 1,
            ..base.clone()
        };
        for seed in 0..32_u64 {
            assert_eq!(
                base.triggers(seed),
                moved.triggers(seed),
                "概率判定必须只依赖 (seed, id)"
            );
        }
    }

    #[test]
    fn trigger_hash_separates_seeds_and_ids() {
        let note = MidiNote {
            probability: Some(0.5),
            ..note_with_id(1)
        };
        let hits = (0..512_u64).filter(|&seed| note.triggers(seed)).count();
        assert!(
            (160..352).contains(&hits),
            "p=0.5 在 512 个种子上应落在宽裕的 1/3..2/3 带内, 实际 {hits}"
        );
        // 身份不同 ⇒ 在足够多的种子上两个音符的判定序列不应完全相同。
        let other = MidiNote {
            probability: Some(0.5),
            ..note_with_id(2)
        };
        let same = (0..512_u64)
            .filter(|&seed| note.triggers(seed) == other.triggers(seed))
            .count();
        assert!(same < 512, "不同身份不应给出完全相同的判定序列");
    }

    #[test]
    fn splitmix64_matches_published_vector() {
        // splitmix64 的公开测试向量: 种子 0 的首个输出。
        assert_eq!(splitmix64(0), 0xE220_A839_7B1D_CDAF);
        assert_eq!(splitmix64(1), 0x910A_2DEC_8902_5CC1);
    }

    /// **跨版本可复现性**：概率触发的哈希与 `(id, seed) ⇒ 判定` 被冻结。
    ///
    /// 为什么需要：既有判据只断言"同一进程内两次一致"与"`p = 0.5` 在 512 个种子上的
    /// 命中数落在宽裕的 1/3..2/3 带内"。实测：把 `trigger_hash` 的
    /// `low ^ splitmix64(high)` 改成 `low.wrapping_add(splitmix64(high))` 时，全仓判据
    /// 保持全绿 —— 也就是说**每个概率音符到底触不触发**可以随一次重构整体改变而无人
    /// 察觉，`ARCH-DET-001` 的"同一 `(seed, id, probability)` 三元组恒给出同一结果"
    /// 因此只覆盖了单进程重放，不覆盖跨版本。
    ///
    /// 期望值来源：**独立实现**（Python，按本模块文档的公开 splitmix64 与组合公式）
    /// 算出的常量，不是本实现的运行读数；公开 splitmix64 向量已在
    /// `splitmix64_matches_published_vector` 里逐位核对。路径上只有整数与 2 的幂除法
    /// （无超越函数），按 ADR-0001 D32 属"IEEE 精确类零容差"，因此可在所有架构上硬断言。
    #[test]
    fn trigger_decisions_are_frozen_against_an_independent_implementation() {
        // (ULID 原始 u128, 种子, trigger_hash 的期望值)
        const FROZEN_HASHES: &[(u128, u64, u64)] = &[
            (0, 0, 0x2382_75BC_38FC_BE91),
            (0, 99, 0xCE8C_385E_28B1_97FA),
            (1, 0, 0x44E5_B981_00C6_7FB0),
            (1, 99, 0xF2BB_C0EE_19EC_F0C2),
            (2, 0, 0xD5F0_95A9_9714_7825),
            (2, 99, 0x578A_ABEB_49C4_D8B2),
            (777, 0, 0xDA77_E771_92AD_4C68),
            (777, 99, 0x59D8_7EDC_BC60_7AD8),
            (12345, 0, 0x559B_725A_95A0_6C4D),
            (12345, 99, 0x3B38_F39B_8088_3A98),
        ];
        for &(raw, seed, expected) in FROZEN_HASHES {
            let id = EntityId::from_ulid(ulid::Ulid::from(raw));
            assert_eq!(
                trigger_hash(seed, id),
                expected,
                "(raw={raw}, seed={seed}) 的触发哈希必须逐位冻结"
            );
        }

        // 用户可见的判定：`p = 0.5` 时上面每一条都由同一个哈希决定。
        const FROZEN_TRIGGERS: &[(u128, u64, bool)] = &[
            (0, 0, true),
            (0, 99, false),
            (1, 0, true),
            (1, 99, false),
            (2, 0, false),
            (2, 99, true),
            (777, 0, false),
            (777, 99, true),
            (12345, 0, true),
            (12345, 99, true),
        ];
        for &(raw, seed, expected) in FROZEN_TRIGGERS {
            let note = MidiNote {
                probability: Some(0.5),
                ..MidiNote::new(EntityId::from_ulid(ulid::Ulid::from(raw)), 0, 60, 960)
            };
            assert_eq!(
                note.triggers(seed),
                expected,
                "(raw={raw}, seed={seed}, p=0.5) 的触发判定必须冻结"
            );
        }
    }

    #[test]
    fn midi_note_serde_round_trip() {
        let note = MidiNote {
            probability: Some(0.25),
            ratchet: Some(3),
            micro_timing_ticks: Some(-12),
            slide: Some(SlideConfig {
                duration_ticks: 120,
                target_pitch: 72,
                curve: CurveType::Exponential,
            }),
            pitch_bend_curve: vec![(0, 0), (240, 200)],
            syllable: Some("la".to_owned()),
            phonemes: vec!["l".to_owned(), "a".to_owned()],
            ..MidiNote::default()
        };
        let json = serde_json::to_string(&note).expect("serialize");
        let back: MidiNote = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, note);
        assert_eq!(back.validate(), Ok(()));
    }

    #[test]
    fn optional_fields_are_omitted_when_absent() {
        let json = serde_json::to_string(&MidiNote::default()).expect("serialize");
        for absent in [
            "probability",
            "ratchet",
            "micro_timing_ticks",
            "slide",
            "pitch_bend_curve",
            "syllable",
            "phonemes",
        ] {
            assert!(
                !json.contains(absent),
                "默认音符的 JSON 不应出现 `{absent}`: {json}"
            );
        }
    }

    #[test]
    fn ids_parse_in_tests_to_prove_fixtures_are_canonical() {
        let id = EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").expect("canonical ulid");
        assert_eq!(id.to_canonical_string(), "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    }

    /// `MidiNote::default()` 的载荷必须被**逐字段**钉住（常量本身也钉住取值）。
    ///
    /// 为什么需要（第六轮注入实测）：把默认音高 `60` 改成 `61` 时全仓判据保持全绿 ——
    /// `impl Default for MidiNote` 的**取值**此前没有任何判据看着（`music.rs` 的 20 条
    /// 判据都在探 `MidiNote::new` 的入参与触发判定）。
    #[test]
    fn midi_note_default_is_middle_c_with_the_documented_payload() {
        // 先钉常量本身的取值：常量被改时"常量 vs 常量−1"那种边界写法抓不到。
        assert_eq!(DEFAULT_VELOCITY, 100);
        assert_eq!(DEFAULT_DURATION_TICKS, 960);
        assert_eq!(DEFAULT_DURATION_TICKS, crate::ids::PPQ);

        let note = MidiNote::default();
        assert_eq!(note.id, EntityId::default());
        assert_eq!(note.start_tick, 0);
        assert_eq!(note.pitch, 60, "默认必须是中音 C");
        assert_eq!(note.velocity, 100);
        assert_eq!(note.duration_ticks, 960);
        assert_eq!(note.probability, None, "默认必须必然触发");
    }
}
