//! ⭐ **装载侧记法适配的判定**（阶段 2，设计见 `docs/ledger/sfz-core-notes.md` §14.4）。
//!
//! 本模块只有**纯函数**：输入是逐 region 的证据（有效根音 + 拼写出处 + 采样嵌入
//! 根音），输出是结构化结论。⛔ 无 I/O、无随机、无墙钟 ⇒ 同输入同结论
//! （`ARCH-DET-001`）；取证据（读 WAV `smpl` 头）与施加移位（解析入参）都在
//! 调用方，两条装载路径（app `sfz_load` 与 engine 摊平的调用方）共用本判定，
//! ⛔ 不许各写一份。
//!
//! ## 判定规则（§14.4 钉死，逐条可判）
//!
//! 记 `delta_i = root_i − keycenter_written_i`，证据集 E ＝「根音以**音名拼写**
//! 写出 **且** 嵌入根音可读」的 region：
//!
//! 1. `|E| ≥ 5`，且 E 占「带采样 region 总数」`≥ 60%`；
//! 2. 候选移位只认 `+12` / `−12`（八度级差才是记法冲突的形状）；
//! 3. `#{delta = S} ≥ 90% × |E|` 且 `#{delta = −S} = 0` 才命中 S；
//! 4. 任一不满足 ⇒ **不移位**（结论带未命中原因与证据计数，报告不许静默）。
//! 5. **零证据独立结论**：E = ∅ 但音名拼写 region ≥ 1 条时，未命中原因记
//!    [`NotationMissReason::NoEvidence`] 而非 `TooFewEvidence` —— 这类文件
//!    （SSO 式老采样库：音名键位 + 采样无 `smpl` 嵌入根音）必须在报告里
//!    被单独点名，⛔ 不许混进「数字键位文件本来就无可检测」的静默多数。
//!
//! 数字拼写与缺省根音的 region **不进 E**：数字写法的作者说的就是数字，
//! 不存在记法可猜（§14.4 的证据资格）。

use std::collections::BTreeMap;

use crate::instrument::{KeycenterSpelling, Region};

/// 一条 region 的检测证据（调用方逐 region 组装）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotationEvidence {
    /// 归约后的有效 `pitch_keycenter`（显式优先、否则 `key` 推导；
    /// **含**文件自带 (a) —— (a) 已把记法对齐时 delta 自然为 0，见 §14.4）。
    pub keycenter_written: i32,
    /// 该根音的拼写出处（只有 [`KeycenterSpelling::NoteName`] 进证据集）。
    pub spelling: KeycenterSpelling,
    /// 采样文件的嵌入根音（WAV `smpl` 头 `dwMIDIUnityNote`，MIDI 号）；
    /// `None` ＝ 读不到（非 WAV、无 `smpl`、字段越界）⇒ 不进证据集。
    pub root: Option<i32>,
}

impl NotationEvidence {
    /// 从一条**未加外部移位**解析出的 region 组装证据（根音由调用方读文件得到）。
    #[must_use]
    pub fn from_region(region: &Region<'_>, root: Option<i32>) -> Self {
        Self {
            keycenter_written: region.pitch_keycenter,
            spelling: region.pitch_keycenter_spelling,
            root,
        }
    }
}

/// 未命中自动移位的原因（报告与提示行用；每一条都对应判定规则的一条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotationMissReason {
    /// ⭐ **零证据**：文件确实用音名写键位（音名拼写 region ≥ 1 条），但
    /// **没有任何一条**的采样带可读嵌入根音 ⇒ E = ∅。这是 SSO 式老采样库
    /// 的形态（采样无 `smpl` 块），与「文件本来就是数字键位」是两回事：
    /// 报告必须给它**独立结论**并响亮透出（问题文件 drum-octave-issue-2
    /// 修复建议第 1 条），⛔ 不许与不移位的静默多数混在一起。
    NoEvidence,
    /// 证据集太小（`0 < |E| < 5`）：音名拼写且根音可读的 region 不足五条。
    TooFewEvidence,
    /// 证据占比不足（`E < 60% × 带采样 region 总数`）。
    EvidenceShareTooLow,
    /// 证据够多，但 delta 不构成「同一八度方向 ≥90% 且对向为 0」。
    NoConsistentOctave,
}

impl NotationMissReason {
    /// 报告行里的人话（与 §14.4 的提示口径一致）。
    #[must_use]
    pub fn describe(&self) -> &'static str {
        match self {
            Self::NoEvidence => "文件用音名写键位，但采样没有可读的嵌入根音，证据为零",
            Self::TooFewEvidence => "音名拼写且嵌入根音可读的 region 不足 5 条",
            Self::EvidenceShareTooLow => "证据 region 占比不足 60%",
            Self::NoConsistentOctave => "嵌入根音的差值不构成一致的八度偏移",
        }
    }

    /// 机器可读的短名（CLI 报告行的 `reason=` 字段用，⛔ 不许随文案漂移）。
    #[must_use]
    pub fn machine_name(&self) -> &'static str {
        match self {
            Self::NoEvidence => "no_evidence",
            Self::TooFewEvidence => "too_few_evidence",
            Self::EvidenceShareTooLow => "evidence_share_too_low",
            Self::NoConsistentOctave => "no_consistent_octave",
        }
    }
}

/// 一次记法检测的**结构化结论**（装载回执原样携带，⛔ 不压成一句话）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotationDetection {
    /// 命中的外部移位（半音，`+12` 或 `−12`）；未命中 ⇒ `None`（不移位）。
    pub applied_shift: Option<i32>,
    /// 未命中但证据呈「同一八度方向占多数」时的**疑似**移位（只进提示行，
    /// ⛔ 不施加）：给用户在乐器面板手动设八度覆盖的指引。
    pub suspected_shift: Option<i32>,
    /// 未命中原因；命中时为 `None`。
    pub miss_reason: Option<NotationMissReason>,
    /// 证据集大小 `|E|`（音名拼写且根音可读的 region 数）。
    pub evidence_count: usize,
    /// **音名拼写**的 region 数（不论根音是否可读）。它与
    /// [`Self::evidence_count`] 的差就是「写了音名、但采样给不出根音」的
    /// 条数 —— 零证据形态（[`NotationMissReason::NoEvidence`]）靠它与
    /// 纯数字键位文件区分开。
    pub named_key_region_count: usize,
    /// 带采样 region 总数（占比规则的分母，由输入切片长度给出）。
    pub sampled_region_count: usize,
    /// E 中 `delta = +12` 的条数。
    pub plus_twelve_count: usize,
    /// E 中 `delta = −12` 的条数。
    pub minus_twelve_count: usize,
    /// E 中出现最多的非零 delta（并列时取**最小**的 delta，确定性）；无非零 ⇒ `None`。
    pub dominant_delta: Option<i32>,
    /// [`Self::dominant_delta`] 的条数。
    pub dominant_delta_count: usize,
}

impl NotationDetection {
    /// 命中方向上的证据条数（回执文案「证据 X/Y 一致」的 X）；未命中 ⇒ 0。
    #[must_use]
    pub fn matching_count(&self) -> usize {
        match self.applied_shift {
            Some(12) => self.plus_twelve_count,
            Some(-12) => self.minus_twelve_count,
            _ => 0,
        }
    }

    /// 是否命中并施加了移位。
    #[must_use]
    pub fn is_hit(&self) -> bool {
        self.applied_shift.is_some()
    }
}

/// 证据集下限（规则 1 的 `|E| ≥ 5`）。
pub const MIN_EVIDENCE_REGIONS: usize = 5;
/// 证据占比下限的分子/分母（规则 1 的 `≥ 60%`，整数比，⛔ 不用浮点）。
pub const MIN_EVIDENCE_SHARE_NUM: usize = 60;
/// 证据占比下限的分母（见 [`MIN_EVIDENCE_SHARE_NUM`]）。
pub const MIN_EVIDENCE_SHARE_DEN: usize = 100;
/// 一致性下限的分子（规则 3 的 `≥ 90%`，整数比）。
pub const MIN_AGREEMENT_NUM: usize = 90;
/// 一致性下限的分母（见 [`MIN_AGREEMENT_NUM`]）。
pub const MIN_AGREEMENT_DEN: usize = 100;

/// 按 §14.4 的规则判定一份乐器的记法移位。
///
/// `evidence` 必须**逐条对应带采样 region**（一条 region 一条，含数字拼写与
/// 根音不可读的 —— 它们计入分母、不进证据集）。纯函数、确定性。
#[must_use]
pub fn detect_notation_shift(evidence: &[NotationEvidence]) -> NotationDetection {
    let sampled_region_count = evidence.len();
    let mut plus_twelve_count = 0usize;
    let mut minus_twelve_count = 0usize;
    let mut delta_counts: BTreeMap<i32, usize> = BTreeMap::new();
    let mut evidence_count = 0usize;
    let mut named_key_region_count = 0usize;
    for item in evidence {
        if item.spelling == KeycenterSpelling::NoteName {
            named_key_region_count += 1;
        }
        // 证据资格（§14.4）：音名拼写 **且** 嵌入根音可读。
        if item.spelling != KeycenterSpelling::NoteName {
            continue;
        }
        let Some(root) = item.root else {
            continue;
        };
        evidence_count += 1;
        let delta = root - item.keycenter_written;
        match delta {
            12 => plus_twelve_count += 1,
            -12 => minus_twelve_count += 1,
            _ => {}
        }
        if delta != 0 {
            *delta_counts.entry(delta).or_insert(0) += 1;
        }
    }
    // 出现最多的非零 delta（BTreeMap 升序 + 严格大于 ⇒ 并列取最小 delta）。
    let mut dominant_delta = None;
    let mut dominant_delta_count = 0usize;
    for (delta, count) in &delta_counts {
        if *count > dominant_delta_count {
            dominant_delta = Some(*delta);
            dominant_delta_count = *count;
        }
    }

    let mut detection = NotationDetection {
        applied_shift: None,
        suspected_shift: None,
        miss_reason: None,
        evidence_count,
        named_key_region_count,
        sampled_region_count,
        plus_twelve_count,
        minus_twelve_count,
        dominant_delta,
        dominant_delta_count,
    };

    // 规则 1a：证据集下限。零证据且文件确实用音名写键位 ⇒ 独立结论
    // `NoEvidence`（SSO 形态）；零证据且全是数字/缺省拼写 ⇒ 仍是
    // `TooFewEvidence`（数字文件本来就没有记法可猜，不许给它加噪音）。
    if evidence_count < MIN_EVIDENCE_REGIONS {
        detection.miss_reason = Some(if evidence_count == 0 && named_key_region_count > 0 {
            NotationMissReason::NoEvidence
        } else {
            NotationMissReason::TooFewEvidence
        });
        return with_suspicion(detection);
    }
    // 规则 1b：证据占比（整数比，⛔ 不用浮点）。
    if evidence_count * MIN_EVIDENCE_SHARE_DEN < sampled_region_count * MIN_EVIDENCE_SHARE_NUM {
        detection.miss_reason = Some(NotationMissReason::EvidenceShareTooLow);
        return with_suspicion(detection);
    }
    // 规则 2＋3：候选只有 ±12；命中要求该方向 ≥90% 且对向严格为 0。
    for (shift, matching, opposing) in [
        (12, plus_twelve_count, minus_twelve_count),
        (-12, minus_twelve_count, plus_twelve_count),
    ] {
        if opposing == 0 && matching * MIN_AGREEMENT_DEN >= evidence_count * MIN_AGREEMENT_NUM {
            detection.applied_shift = Some(shift);
            return detection;
        }
    }
    detection.miss_reason = Some(NotationMissReason::NoConsistentOctave);
    with_suspicion(detection)
}

/// 给未命中结论补「疑似」读数：主导 delta 是八度方向（±12）且占 E 的**严格
/// 多数** ⇒ 疑似该方向（只提示、⛔ 不施加，见 [`NotationDetection::suspected_shift`]）。
fn with_suspicion(mut detection: NotationDetection) -> NotationDetection {
    if detection.evidence_count > 0
        && let Some(delta) = detection.dominant_delta
        && (delta == 12 || delta == -12)
        && detection.dominant_delta_count * 2 > detection.evidence_count
    {
        detection.suspected_shift = Some(delta);
    }
    detection
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(keycenter: i32, root: i32) -> NotationEvidence {
        NotationEvidence {
            keycenter_written: keycenter,
            spelling: KeycenterSpelling::NoteName,
            root: Some(root),
        }
    }

    fn numeric(keycenter: i32, root: i32) -> NotationEvidence {
        NotationEvidence {
            keycenter_written: keycenter,
            spelling: KeycenterSpelling::Numeric,
            root: Some(root),
        }
    }

    fn repeat(item: NotationEvidence, count: usize) -> Vec<NotationEvidence> {
        vec![item; count]
    }

    #[test]
    fn hits_at_the_exact_thresholds() {
        // 恰达门槛：|E| = 5（下限）、占比 100%、一致性 100% ⇒ 命中 +12。
        let detection = detect_notation_shift(&repeat(named(48, 60), 5));
        assert_eq!(detection.applied_shift, Some(12));
        assert_eq!(detection.miss_reason, None);
        assert_eq!(detection.matching_count(), 5);
        assert_eq!(detection.evidence_count, 5);
        assert_eq!(detection.sampled_region_count, 5);
        // 一致性恰 90%：10 条里 9 条 +12、1 条 delta 0（对向仍为 0）⇒ 命中。
        let mut evidence = repeat(named(48, 60), 9);
        evidence.push(named(60, 60));
        let detection = detect_notation_shift(&evidence);
        assert_eq!(detection.applied_shift, Some(12));
        assert_eq!(detection.plus_twelve_count, 9);
        // 占比恰 60%：|E| = 6 / 带采样 10 ⇒ 命中。
        let mut evidence = repeat(named(48, 60), 6);
        evidence.extend(repeat(numeric(48, 60), 4));
        let detection = detect_notation_shift(&evidence);
        assert_eq!(detection.applied_shift, Some(12));
        assert_eq!(detection.evidence_count, 6);
        assert_eq!(detection.sampled_region_count, 10);
    }

    #[test]
    fn hits_minus_twelve_symmetrically() {
        // 作者记法比 IPN 高一个八度（写 c5 意指 60）：delta 全 −12 ⇒ 移位 −12。
        let detection = detect_notation_shift(&repeat(named(72, 60), 6));
        assert_eq!(detection.applied_shift, Some(-12));
        assert_eq!(detection.matching_count(), 6);
        assert_eq!(detection.minus_twelve_count, 6);
    }

    #[test]
    fn one_short_of_the_agreement_threshold_misses_but_suspects() {
        // 差一条：10 条里 8 条 +12（80% < 90%）⇒ 不命中；主导方向占多数 ⇒ 疑似。
        let mut evidence = repeat(named(48, 60), 8);
        evidence.extend(repeat(named(50, 50), 2));
        let detection = detect_notation_shift(&evidence);
        assert_eq!(detection.applied_shift, None);
        assert_eq!(
            detection.miss_reason,
            Some(NotationMissReason::NoConsistentOctave)
        );
        assert_eq!(detection.suspected_shift, Some(12));
        assert_eq!(detection.dominant_delta, Some(12));
        assert_eq!(detection.dominant_delta_count, 8);
    }

    #[test]
    fn an_opposing_delta_blocks_the_hit() {
        // 对向 delta 出现（9 条 +12、1 条 −12）⇒ 规则 3 的「对向为 0」不满足。
        let mut evidence = repeat(named(48, 60), 9);
        evidence.push(named(72, 60));
        let detection = detect_notation_shift(&evidence);
        assert_eq!(detection.applied_shift, None);
        assert_eq!(
            detection.miss_reason,
            Some(NotationMissReason::NoConsistentOctave)
        );
        assert_eq!(detection.minus_twelve_count, 1);
    }

    #[test]
    fn too_few_evidence_regions_misses() {
        // |E| = 4 < 5：方向再一致也不命中。
        let detection = detect_notation_shift(&repeat(named(48, 60), 4));
        assert_eq!(detection.applied_shift, None);
        assert_eq!(
            detection.miss_reason,
            Some(NotationMissReason::TooFewEvidence)
        );
        assert_eq!(detection.suspected_shift, Some(12));
    }

    #[test]
    fn evidence_share_below_sixty_percent_misses() {
        // |E| = 6、带采样 20（30% < 60%）⇒ 占比不足（其余 region 是数字拼写）。
        let mut evidence = repeat(named(48, 60), 6);
        evidence.extend(repeat(numeric(48, 60), 14));
        let detection = detect_notation_shift(&evidence);
        assert_eq!(detection.applied_shift, None);
        assert_eq!(
            detection.miss_reason,
            Some(NotationMissReason::EvidenceShareTooLow)
        );
        assert_eq!(detection.suspected_shift, Some(12));
    }

    #[test]
    fn numeric_and_default_spellings_are_never_evidence() {
        // 数字拼写的作者说的就是数字：delta 全 +12 也不进证据集。
        let detection = detect_notation_shift(&repeat(numeric(48, 60), 10));
        assert_eq!(detection.applied_shift, None);
        assert_eq!(detection.evidence_count, 0);
        assert_eq!(
            detection.miss_reason,
            Some(NotationMissReason::TooFewEvidence)
        );
        assert_eq!(detection.suspected_shift, None);
        // 缺省出处同理；根音不可读的音名 region 同样不进 E。
        let mut evidence = repeat(
            NotationEvidence {
                keycenter_written: 60,
                spelling: KeycenterSpelling::Default,
                root: Some(72),
            },
            5,
        );
        evidence.extend(repeat(
            NotationEvidence {
                keycenter_written: 48,
                spelling: KeycenterSpelling::NoteName,
                root: None,
            },
            5,
        ));
        let detection = detect_notation_shift(&evidence);
        assert_eq!(detection.evidence_count, 0);
        assert_eq!(detection.applied_shift, None);
    }

    #[test]
    fn note_named_keys_without_any_roots_are_a_distinct_no_evidence_miss() {
        // ⭐ SSO 形态（问题文件 drum-octave-issue-2）：键位用音名拼写、
        // 采样全部没有嵌入根音 ⇒ E = ∅。结论必须是**独立**的
        // `NoEvidence`，⛔ 不是与纯数字文件同款的 `TooFewEvidence`。
        let rootless = |keycenter: i32| NotationEvidence {
            keycenter_written: keycenter,
            spelling: KeycenterSpelling::NoteName,
            root: None,
        };
        let detection = detect_notation_shift(&[rootless(48), rootless(49), rootless(50)]);
        assert_eq!(detection.applied_shift, None);
        assert_eq!(detection.evidence_count, 0);
        assert_eq!(detection.named_key_region_count, 3);
        assert_eq!(detection.sampled_region_count, 3);
        assert_eq!(
            detection.miss_reason,
            Some(NotationMissReason::NoEvidence)
        );
        assert_eq!(detection.miss_reason.unwrap().machine_name(), "no_evidence");
        assert_eq!(detection.suspected_shift, None);
        // 对照：音名拼写为零（纯数字）时零证据仍是 TooFewEvidence。
        let detection = detect_notation_shift(&repeat(numeric(48, 60), 3));
        assert_eq!(
            detection.miss_reason,
            Some(NotationMissReason::TooFewEvidence)
        );
        assert_eq!(detection.named_key_region_count, 0);
    }

    #[test]
    fn scattered_drum_like_roots_do_not_hit_and_do_not_suspect() {
        // SSO 式鼓组：嵌入根音是缺省值、delta 散列 ⇒ 不命中、无主导方向 ⇒ 不疑似。
        let evidence = vec![
            named(48, 60),
            named(49, 60),
            named(50, 62),
            named(51, 55),
            named(52, 60),
            named(53, 48),
        ];
        let detection = detect_notation_shift(&evidence);
        assert_eq!(detection.applied_shift, None);
        assert_eq!(
            detection.miss_reason,
            Some(NotationMissReason::NoConsistentOctave)
        );
        assert_eq!(detection.suspected_shift, None);
    }

    #[test]
    fn aligned_library_is_a_clean_miss_with_no_suspicion() {
        // 记法本来就对齐（delta 全 0）：不命中、无疑似、无主导 delta。
        let detection = detect_notation_shift(&repeat(named(60, 60), 8));
        assert_eq!(detection.applied_shift, None);
        assert_eq!(detection.suspected_shift, None);
        assert_eq!(detection.dominant_delta, None);
        assert_eq!(
            detection.miss_reason,
            Some(NotationMissReason::NoConsistentOctave)
        );
    }
}
