//! `yeban-theory` 的统一错误类型。
//!
//! 本 crate 是不可信输入的落点：和弦符号、罗马数字级数、音名文本都可能来自
//! MCP 代理（`yeban_propose_section` 的 `stylePreset` / `scale` 参数）或用户手打。
//! 因此解析路径**绝不 panic**，一律返回 [`TheoryError`]。

use thiserror::Error;

/// `yeban-theory` 的统一错误类型。
///
/// 全部变体都是 `Copy` 语义（不携带 `String`），因此本类型派生 `Eq`；
/// 需要保留原始输入文本时，由调用方在错误日志里自行拼接，避免在错误类型上分配。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TheoryError {
    /// 音名文本无法解析（例如 `H4`、`C#`、空串）。
    #[error("unrecognized note name")]
    NoteNameUnknown,

    /// MIDI 音高越界 (允许 0..=127)。
    #[error("pitch {value} out of range 0..=127")]
    PitchOutOfRange {
        /// 实际收到的音高值。
        value: u32,
    },

    /// 音级 (音阶 degree) 越界 —— 音阶只有 7 个音级，取模后仍是 0..=6。
    #[error("scale degree {degree} out of range 0..=6")]
    DegreeOutOfRange {
        /// 实际收到的音级。
        degree: u8,
    },

    /// 音阶名无法解析（例如 `klingon`）。
    #[error("unrecognized scale name")]
    ScaleNameUnknown,

    /// 调式名无法解析。
    #[error("unrecognized mode name")]
    ModeNameUnknown,

    /// 和弦符号的根音名无法解析。
    #[error("unrecognized chord root")]
    ChordRootUnknown,

    /// 根音之后的后缀不是已知和弦类型（例如 `Cmaj9#11`）。
    #[error("unrecognized chord quality suffix")]
    ChordQualityUnknown,

    /// 罗马数字级数无法解析（例如 `VIII`、`H`、空串）。
    #[error("unrecognized roman numeral degree")]
    DegreeSymbolUnknown,

    /// 罗马数字级数越界（I..=VII 之外的数字）。
    #[error("roman numeral degree number out of range 1..=7")]
    DegreeNumberOutOfRange,

    /// 走向为空：没有任何级数就无法展开。
    #[error("progression is empty")]
    EmptyProgression,

    /// `bars` 为 0：小节数为 0 的走向没有定义。
    #[error("bars must be non-zero")]
    ZeroBars,

    /// 级数太密：请求的时长里放不下这么多个级数（每个级数至少要占一个 16 分音符）。
    ///
    /// 这里选择**显式报错**而不是静默丢级数或产出零长度区段 —— MCP 层据此回
    /// `INVALID_PARAMS`，让调用方自己把 `bars` 调大。
    ///
    /// 同一个变体也承载节奏网格的同类错误：请求的 `onsets_per_bar` 超过小节内的
    /// 16 分格位数时，[`crate::rhythm::metric_grid`] 用它报错（`degrees` 承载
    /// 请求的 onset 数，`slots` 承载可用格位数），同样**不**静默钳制。
    #[error("progression is too dense: {degrees} degrees do not fit {slots} sixteenth-note slots")]
    ProgressionTooDense {
        /// 级数个数（节奏网格复用本变体时承载请求的 onset 数）。
        degrees: usize,
        /// 可用槽位数（每槽 = 一个 16 分音符 = 240 tick）。
        slots: usize,
    },

    /// 声部数少于 2：单声部没有"连接"可言。
    #[error("voice count {count} is too small (need >= 2)")]
    TooFewVoices {
        /// 实际请求的声部数。
        count: usize,
    },

    /// 声部数超过和弦可选音数：无法给每个声部一个不同音高。
    #[error("voice count {count} exceeds available distinct pitches")]
    TooManyVoices {
        /// 实际请求的声部数。
        count: usize,
    },

    /// 声部音域区间非法（`lower > upper`）。
    #[error("voice range {lower}..={upper} is empty")]
    VoiceRangeInvalid {
        /// 区间下界（含）。
        lower: u8,
        /// 区间上界（含）。
        upper: u8,
    },

    /// 声部音域超出 MIDI 0..=127。
    #[error("voice range exceeds MIDI 0..=127")]
    VoiceRangeOutOfMidi,

    /// 在给定音域与跳进约束下找不到任何可行声位。
    #[error("no feasible voicing for the given ranges and voice-leading bound")]
    NoFeasibleVoicing,

    /// 流派 ID 不存在于规则库中（对应 MCP 的 `STYLE_NOT_FOUND`）。
    #[error("genre id not found in library")]
    GenreNotFound,

    /// 摇摆比例越界：合法区间是千分之 `500..=1000`（500 = 平直，1000 = 附点）。
    #[error("swing permille {value} out of range 500..=1000")]
    SwingOutOfRange {
        /// 实际收到的千分比。
        value: u16,
    },

    /// 五度圈距离计算失败：音级映射缺失（内部一致性错误）。
    #[error("fifth-circle mapping failed for this scale")]
    FifthCircleUnavailable,

    /// 音高拼写歧义或无法用当前调性表示（例如重升号）。
    #[error("pitch spelling is ambiguous")]
    AmbiguousSpelling,

    /// 请求的音域/八度导致 MIDI 越界。
    #[error("spelling root not present in scale")]
    SpellingRootNotInScale,
}

impl TheoryError {
    /// 该错误是否属于"用户输入错误"（而不是内部一致性错误）。
    ///
    /// MCP 层用这个分类决定是回 `INVALID_PARAMS` 还是 `INTERNAL_ERROR`；
    /// 本 crate 只负责分类，不做协议映射。
    #[must_use]
    pub const fn is_input_error(self) -> bool {
        matches!(
            self,
            Self::NoteNameUnknown
                | Self::PitchOutOfRange { .. }
                | Self::DegreeOutOfRange { .. }
                | Self::ScaleNameUnknown
                | Self::ModeNameUnknown
                | Self::ChordRootUnknown
                | Self::ChordQualityUnknown
                | Self::DegreeSymbolUnknown
                | Self::DegreeNumberOutOfRange
                | Self::EmptyProgression
                | Self::ZeroBars
                | Self::ProgressionTooDense { .. }
                | Self::TooFewVoices { .. }
                | Self::TooManyVoices { .. }
                | Self::VoiceRangeInvalid { .. }
                | Self::VoiceRangeOutOfMidi
                | Self::NoFeasibleVoicing
                | Self::GenreNotFound
                | Self::SwingOutOfRange { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_errors_are_classified_as_input_errors() {
        assert!(TheoryError::NoteNameUnknown.is_input_error());
        assert!(TheoryError::PitchOutOfRange { value: 200 }.is_input_error());
        assert!(TheoryError::GenreNotFound.is_input_error());
        assert!(!TheoryError::FifthCircleUnavailable.is_input_error());
        assert!(!TheoryError::AmbiguousSpelling.is_input_error());
    }

    #[test]
    fn display_messages_carry_the_offending_number() {
        assert_eq!(
            TheoryError::PitchOutOfRange { value: 200 }.to_string(),
            "pitch 200 out of range 0..=127"
        );
    }

    #[test]
    fn the_swing_error_message_states_the_real_legal_range() {
        // 判据：错误文本里的区间必须与 `swing::SWING_PERMILLE_STRAIGHT..=MAX` 同值。
        // 这条判据在修复前是红的：文本写的是 "50..=100"，而真实区间是 500..=1000。
        let message = TheoryError::SwingOutOfRange { value: 1 }.to_string();
        assert_eq!(message, "swing permille 1 out of range 500..=1000");
        let expected = format!(
            "swing permille 1 out of range {}..={}",
            crate::swing::SWING_PERMILLE_STRAIGHT,
            crate::swing::SWING_PERMILLE_MAX
        );
        assert_eq!(message, expected);
    }
}
