//! 声部连接 (Voice Leading)。
//!
//! ## 目标与硬约束
//!
//! 给定一串和弦，为每个和弦选一个**声位 (voicing)**，使相邻和弦之间的声部
//! 总移动量尽可能小。这里明确区分两件事：
//!
//! - **硬约束 (invariant)**：任何相邻和弦之间，**没有任何单声部移动超过
//!   [`VoicingConstraints::max_voice_jump`] 个半音**（默认 12，即一个八度）。
//!   违反即 [`TheoryError::NoFeasibleVoicing`]，绝不静默产出一个跳进过大的声位。
//! - **软目标**：总移动量最小。用**束搜索 (beam search)** 在全部候选声位上择优，
//!   搜索宽度固定（[`SEARCH_BEAM`]），因此结果对同一输入完全确定。
//!
//! ## 为什么不是"贪心最近"
//!
//! 逐步取"离上一个声位最近"的声位会陷入局部最优：第一步为了省 1 个半音，
//! 可能把后三个声部推到高把位。束搜索保留 `SEARCH_BEAM` 条前缀路径，
//! 用 `总移动 + 声部跨度过宽惩罚` 作为代价函数，代价完全相同的前缀按
//! 声位字典序打破平局——因此没有任何依赖哈希迭代顺序的行为。

use crate::error::TheoryError;
use crate::pitch::Pitch;
use crate::progression::ChordSpan;

/// 束搜索宽度。固定值，保证同输入同输出。
pub const SEARCH_BEAM: usize = 24;

/// 单个声部的音域（含端点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoiceRange {
    /// 下界（含）。
    pub lower: Pitch,
    /// 上界（含）。
    pub upper: Pitch,
}

impl VoiceRange {
    /// 构造音域。
    ///
    /// # Errors
    ///
    /// `lower > upper` 时返回 [`TheoryError::VoiceRangeInvalid`]。
    pub const fn new(lower: Pitch, upper: Pitch) -> Result<Self, TheoryError> {
        if lower.value() > upper.value() {
            return Err(TheoryError::VoiceRangeInvalid {
                lower: lower.value(),
                upper: upper.value(),
            });
        }
        Ok(Self { lower, upper })
    }

    /// 该音域内的音高个数。
    #[must_use]
    pub const fn width(&self) -> u8 {
        self.upper.value() - self.lower.value() + 1
    }
}

/// 声部连接的配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoicingConstraints {
    /// 每个声部的音域（长度即声部数）。
    pub ranges: &'static [VoiceRange],
    /// 相邻和弦之间任一单声部允许的最大跳进（半音）。
    pub max_voice_jump: i16,
    /// 允许的总移动量上界（软约束，用于报告"未达标"而不是直接失败）。
    pub max_total_movement: i16,
}

impl VoicingConstraints {
    /// 默认的 3 声部音域（低/中/高各两个八度），跳进上界一个八度。
    pub const THREE_VOICES: Self = Self {
        ranges: &DEFAULT_THREE_VOICES,
        max_voice_jump: 12,
        max_total_movement: 24,
    };

    /// 默认的 4 声部音域（SATB 近似的纯音高区间）。
    pub const FOUR_VOICES: Self = Self {
        ranges: &DEFAULT_FOUR_VOICES,
        max_voice_jump: 12,
        max_total_movement: 32,
    };

    /// 声部数。
    #[must_use]
    pub const fn voice_count(&self) -> usize {
        self.ranges.len()
    }
}

/// 默认 3 声部音域：低音 C3–C5、中音 C4–C6、高音 C5–C7。
const DEFAULT_THREE_VOICES: [VoiceRange; 3] = [
    VoiceRange {
        lower: Pitch::C3,
        upper: Pitch::C5,
    },
    VoiceRange {
        lower: Pitch::C4,
        upper: Pitch::C6,
    },
    VoiceRange {
        lower: Pitch::C5,
        upper: Pitch::C7,
    },
];

/// 默认 4 声部音域：C2–C4 / C3–C5 / C5–C7 / C6–C8。
///
/// 高两个声部故意抬到 C5–C7 与 C6–C8：三和弦只有 3 个构成音，4 个声部必然
/// 有一个音被重复；把重复放到**上方八度**是四声部写作的常规做法。默认 3 声部
/// 配置就是三和弦的标准写法，4 声部留给需要加九音/七音的进行。
const DEFAULT_FOUR_VOICES: [VoiceRange; 4] = [
    VoiceRange {
        lower: Pitch::C2,
        upper: Pitch::C4,
    },
    VoiceRange {
        lower: Pitch::C3,
        upper: Pitch::C5,
    },
    VoiceRange {
        lower: Pitch::C5,
        upper: Pitch::C7,
    },
    VoiceRange {
        lower: Pitch::C6,
        upper: Pitch::C8,
    },
];

/// 单个候选声位。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Voicing {
    voices: Vec<Pitch>,
}

impl Voicing {
    /// 声部总跨度，用于在总移动量相同时偏好紧凑排列。
    fn spread(&self) -> i32 {
        match (self.voices.first(), self.voices.last()) {
            (Some(low), Some(high)) => i32::from(high.value() - low.value()),
            _ => 0,
        }
    }
}

/// 枚举某个和弦在给定音域下的**全部**候选声位。
///
/// 结果按"音高序列字典序"排序并去重，因此完全确定，与任何哈希迭代顺序无关。
fn candidate_voicings(chord: &[Pitch], ranges: &[VoiceRange]) -> Result<Vec<Voicing>, TheoryError> {
    if ranges.is_empty() || ranges.len() < 2 {
        return Err(TheoryError::TooFewVoices {
            count: ranges.len(),
        });
    }
    let mut tones: Vec<u8> = chord.iter().map(|p| p.value()).collect();
    tones.sort_unstable();
    tones.dedup();
    if tones.len() < ranges.len() {
        return Err(TheoryError::TooManyVoices {
            count: ranges.len(),
        });
    }
    let mut results: Vec<Voicing> = Vec::new();
    let mut current = vec![0u8; ranges.len()];
    recurse(0, &mut current, &tones, ranges, &mut results);
    results.sort();
    results.dedup();
    if results.is_empty() {
        return Err(TheoryError::NoFeasibleVoicing);
    }
    Ok(results)
}

/// 逐声部枚举：第 `index` 个声部在自己的音域里取一个比前一个声部更高的和弦音。
fn recurse(
    index: usize,
    current: &mut [u8],
    tones: &[u8],
    ranges: &[VoiceRange],
    results: &mut Vec<Voicing>,
) {
    if index == ranges.len() {
        let voices = current
            .iter()
            .filter_map(|&value| Pitch::new(i32::from(value)).ok())
            .collect();
        results.push(Voicing { voices });
        return;
    }
    let range = ranges[index];
    let floor = if index == 0 {
        range.lower.value()
    } else {
        // 严格上行，避免两个声部落在同一个音高上。
        current[index - 1].saturating_add(1)
    };
    let start = floor.max(range.lower.value());
    let end = range.upper.value();
    if start > end {
        return;
    }
    for value in start..=end {
        if tones.binary_search(&value).is_ok() {
            current[index] = value;
            recurse(index + 1, current, tones, ranges, results);
        }
    }
}

/// 一次声部连接的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceLeadingResult {
    /// 每个和弦对应的声位（外层按和弦顺序，内层按声部由低到高）。
    pub voicings: Vec<Vec<Pitch>>,
    /// 相邻和弦之间每个声部的移动量（绝对值，半音）；长度 = `voicings.len() - 1`。
    pub movements: Vec<Vec<u8>>,
    /// 全部移动量之和。
    pub total_movement: u32,
    /// 是否有相邻和弦的**总**移动量超过了
    /// [`VoicingConstraints::max_total_movement`]（单声部硬约束仍然满足）。
    pub exceeded_total_target: bool,
}

impl VoiceLeadingResult {
    /// 最大单声部跳进（半音）。无相邻和弦时返回 0。
    #[must_use]
    pub fn max_voice_jump(&self) -> u8 {
        self.movements
            .iter()
            .flat_map(|row| row.iter().copied())
            .max()
            .unwrap_or(0)
    }
}

/// 把和弦序列连接成声部。
///
/// # Errors
///
/// - 声部数 < 2 → [`TheoryError::TooFewVoices`]
/// - 和弦可选音少于声部数 → [`TheoryError::TooManyVoices`]
/// - 音域内没有可行声位 → [`TheoryError::NoFeasibleVoicing`]
/// - 相邻和弦之间无法满足单声部跳进上界 → [`TheoryError::NoFeasibleVoicing`]
pub fn realize(
    spans: &[ChordSpan],
    constraints: &VoicingConstraints,
) -> Result<VoiceLeadingResult, TheoryError> {
    if constraints.voice_count() < 2 {
        return Err(TheoryError::TooFewVoices {
            count: constraints.voice_count(),
        });
    }
    let mut layers: Vec<Vec<Voicing>> = Vec::with_capacity(spans.len());
    for span in spans {
        // 和弦音铺在 C2..C7 六个八度内：4 声部配置的音域横跨 C2..C8，
        // 只铺两个八度会让最高声部找不到可用和弦音（三和弦只有 3 个构成音，
        // 必须靠八度重复补足声部数）。
        let mut tones: Vec<Pitch> = Vec::new();
        for octave in 2..=7i16 {
            for pitch in span.chord.pitches(octave as i8)? {
                tones.push(pitch);
            }
        }
        layers.push(candidate_voicings(&tones, constraints.ranges)?);
    }
    if layers.is_empty() {
        return Ok(VoiceLeadingResult {
            voicings: Vec::new(),
            movements: Vec::new(),
            total_movement: 0,
            exceeded_total_target: false,
        });
    }

    // --- 束搜索 ---------------------------------------------------------
    // 每条状态 = (累计代价, 当前声位, 从起点到这里的声位序列)。
    let mut beam: Vec<(i64, Voicing, Vec<Voicing>)> = layers[0]
        .iter()
        .map(|voicing| {
            (
                i64::from(voicing.spread()),
                voicing.clone(),
                vec![voicing.clone()],
            )
        })
        .collect();
    beam.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    beam.truncate(SEARCH_BEAM);

    for layer in &layers[1..] {
        let mut next: Vec<(i64, Voicing, Vec<Voicing>)> = Vec::new();
        for (cost, previous, path) in &beam {
            for candidate in layer {
                let movements: Vec<u8> = previous
                    .voices
                    .iter()
                    .zip(candidate.voices.iter())
                    .map(|(from, to)| {
                        // MIDI 音高差必然在 0..=127，`as u8` 不会截断。
                        from.semitone_distance_to(*to).unsigned_abs() as u8
                    })
                    .collect();
                // 硬约束：任一单声部跳进超界即剪枝。
                if movements
                    .iter()
                    .any(|&step| i16::from(step) > constraints.max_voice_jump)
                {
                    continue;
                }
                let delta = movements.iter().map(|&step| i64::from(step)).sum::<i64>();
                let new_cost = cost + delta + i64::from(candidate.spread());
                let mut new_path = path.clone();
                new_path.push(candidate.clone());
                next.push((new_cost, candidate.clone(), new_path));
            }
        }
        if next.is_empty() {
            return Err(TheoryError::NoFeasibleVoicing);
        }
        next.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        next.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        next.truncate(SEARCH_BEAM);
        beam = next;
    }

    let (_, _, path) = beam
        .into_iter()
        .next()
        .ok_or(TheoryError::NoFeasibleVoicing)?;
    let voicings: Vec<Vec<Pitch>> = path.into_iter().map(|voicing| voicing.voices).collect();

    // --- 统计 -----------------------------------------------------------
    let mut movements: Vec<Vec<u8>> = Vec::with_capacity(voicings.len().saturating_sub(1));
    let mut total_movement = 0u32;
    let mut exceeded_total_target = false;
    for pair in voicings.windows(2) {
        let row: Vec<u8> = pair[0]
            .iter()
            .zip(pair[1].iter())
            .map(|(from, to)| from.semitone_distance_to(*to).unsigned_abs() as u8)
            .collect();
        let sum: u32 = row.iter().map(|&step| u32::from(step)).sum();
        if sum > u32::from(constraints.max_total_movement.unsigned_abs()) {
            exceeded_total_target = true;
        }
        total_movement += sum;
        movements.push(row);
    }

    Ok(VoiceLeadingResult {
        voicings,
        movements,
        total_movement,
        exceeded_total_target,
    })
}

/// 便捷入口：默认 3 声部。
///
/// # Errors
///
/// 同 [`realize`]。
pub fn realize_three_voices(spans: &[ChordSpan]) -> Result<VoiceLeadingResult, TheoryError> {
    realize(spans, &VoicingConstraints::THREE_VOICES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::PitchClass;
    use crate::progression::expand_progression;
    use crate::scale::{Scale, ScaleKind};

    fn c_major() -> Scale {
        Scale::new(PitchClass::C, ScaleKind::Major)
    }

    #[test]
    fn three_voices_are_produced_for_every_chord() {
        let key = c_major();
        let spans = expand_progression(&key, "I-V-vi-IV", 4).unwrap();
        let result = realize_three_voices(&spans).unwrap();
        assert_eq!(result.voicings.len(), 4);
        for voicing in &result.voicings {
            assert_eq!(voicing.len(), 3);
            // 严格由低到高
            assert!(voicing[0] < voicing[1] && voicing[1] < voicing[2]);
            for (index, pitch) in voicing.iter().enumerate() {
                let range = VoicingConstraints::THREE_VOICES.ranges[index];
                assert!(
                    pitch.value() >= range.lower.value() && pitch.value() <= range.upper.value(),
                    "{pitch} outside voice {index} range"
                );
            }
        }
    }

    #[test]
    fn every_voice_stays_on_a_chord_tone() {
        let key = c_major();
        let spans = expand_progression(&key, "ii-V-I-vi", 4).unwrap();
        let result = realize_three_voices(&spans).unwrap();
        for (span, voicing) in spans.iter().zip(result.voicings.iter()) {
            for pitch in voicing {
                assert!(
                    span.chord.pitch_classes().contains(&pitch.pitch_class()),
                    "{pitch} is not a chord tone of {}",
                    span.chord.symbol()
                );
            }
        }
    }

    #[test]
    fn the_documented_jump_bound_holds_for_the_canonical_progressions() {
        let key = c_major();
        for text in ["I-V-vi-IV", "ii-V-I", "I-vi-ii-V", "I-IV-V-I", "vi-IV-I-V"] {
            let spans = expand_progression(&key, text, 4).unwrap();
            let result = realize_three_voices(&spans).unwrap();
            assert!(
                result.max_voice_jump() <= 12,
                "{text}: max voice jump was {}",
                result.max_voice_jump()
            );
        }
    }

    #[test]
    fn greedy_is_not_used_because_total_movement_stays_within_target() {
        let key = c_major();
        let spans = expand_progression(&key, "I-V-vi-IV", 4).unwrap();
        let result = realize_three_voices(&spans).unwrap();
        assert!(
            !result.exceeded_total_target,
            "total movement {} exceeded target",
            result.total_movement
        );
        assert!(result.total_movement > 0);
    }

    #[test]
    fn movements_match_the_voicings() {
        let key = c_major();
        let spans = expand_progression(&key, "I-IV-V-I", 4).unwrap();
        let result = realize_three_voices(&spans).unwrap();
        assert_eq!(result.movements.len(), 3);
        for (index, row) in result.movements.iter().enumerate() {
            let from = &result.voicings[index];
            let to = &result.voicings[index + 1];
            let expected: Vec<u8> = from
                .iter()
                .zip(to.iter())
                .map(|(a, b)| a.semitone_distance_to(*b).unsigned_abs() as u8)
                .collect();
            assert_eq!(*row, expected);
        }
        assert_eq!(
            result.total_movement,
            result
                .movements
                .iter()
                .flatten()
                .map(|&step| u32::from(step))
                .sum::<u32>()
        );
    }

    #[test]
    fn four_voices_also_work() {
        let key = c_major();
        let spans = expand_progression(&key, "I-vi-IV-V", 4).unwrap();
        let result = realize(&spans, &VoicingConstraints::FOUR_VOICES).unwrap();
        assert_eq!(result.voicings.len(), 4);
        assert!(result.voicings.iter().all(|v| v.len() == 4));
        assert!(result.max_voice_jump() <= 12);
    }

    #[test]
    fn four_voices_double_a_chord_tone_when_the_triad_has_only_three() {
        // 三和弦只有 3 个构成音，4 声部必然重复其中一个；重复音必须在
        // 和弦音集合里，而不能是"凑数"的邻音。
        let key = c_major();
        let spans = expand_progression(&key, "I", 1).unwrap();
        let result = realize(&spans, &VoicingConstraints::FOUR_VOICES).unwrap();
        let voicing = &result.voicings[0];
        assert_eq!(voicing.len(), 4);
        for pitch in voicing {
            assert!(
                spans[0]
                    .chord
                    .pitch_classes()
                    .contains(&pitch.pitch_class()),
                "{pitch} is not a chord tone"
            );
        }
    }

    #[test]
    fn impossible_constraints_fail_loudly_instead_of_silently() {
        let key = c_major();
        let spans = expand_progression(&key, "I", 1).unwrap();
        // 音域窄到只容得下 2 个半音，三声部放不下
        const TIGHT: [VoiceRange; 3] = [
            VoiceRange {
                lower: Pitch::C4,
                upper: Pitch::CS4,
            },
            VoiceRange {
                lower: Pitch::C4,
                upper: Pitch::CS4,
            },
            VoiceRange {
                lower: Pitch::C4,
                upper: Pitch::CS4,
            },
        ];
        let constraints = VoicingConstraints {
            ranges: &TIGHT,
            max_voice_jump: 12,
            max_total_movement: 24,
        };
        assert_eq!(
            realize(&spans, &constraints).unwrap_err(),
            TheoryError::NoFeasibleVoicing
        );
        // 跳进上界为 0 时，任何伴奏进行都必然无解
        let spans = expand_progression(&key, "I-ii", 2).unwrap();
        let zero_jump = VoicingConstraints {
            max_voice_jump: 0,
            ..VoicingConstraints::THREE_VOICES
        };
        assert_eq!(
            realize(&spans, &zero_jump).unwrap_err(),
            TheoryError::NoFeasibleVoicing
        );
    }

    #[test]
    fn empty_input_is_an_empty_result_not_a_panic() {
        let result = realize_three_voices(&[]).unwrap();
        assert!(result.voicings.is_empty());
        assert_eq!(result.max_voice_jump(), 0);
    }

    #[test]
    fn search_is_deterministic_across_repeated_calls() {
        let key = c_major();
        let spans = expand_progression(&key, "I-V-vi-IV-ii-V-I", 8).unwrap();
        let first = realize_three_voices(&spans).unwrap();
        let second = realize_three_voices(&spans).unwrap();
        assert_eq!(first, second);
    }
}
