//! 音高、音名拼写与音程 [ARCH-DET-001]。
//!
//! 本模块把"音高"固定成**整数**语义，避免任何平台浮点行为泄漏到乐理判定里：
//!
//! - [`PitchClass`] — 0..12 的八度等价音级（C = 0）。
//! - [`Pitch`] — MIDI note 0..=127（C-1 = 0，A4 = 69）。
//! - [`Interval`] — 半音数 + 规范名称。
//! - [`note_to_hz`] — 十二平均律频率，A4 = 440 Hz。
//!
//! ## 为什么频率走 `libm`
//!
//! `f64::powf` 在 std 里最终落到各平台的 `pow`，x86_64 与 AArch64 的
//! 末位（ULP）可能不同。本 crate 的**判定逻辑**全部只用整数（半音、音级、
//! 五度圈距离），浮点只出现在 `note_to_hz` 这一个输出口；即便如此也统一走
//! `libm::pow`，与 [ARCH-DET-001] 的"统一启用纯 Rust `libm` 数学库"一致。
//!
//! ## 音名拼写
//!
//! [`NoteName`] 是"**音级 + 变音记号**"的拼写结果，而不是频率。它只用于
//! 显示（`symbol()` / `Display`），判定永远回到 [`PitchClass`] 的整数比较。
//! A 小调与 B♭ 大调里的同一个黑键必须显示成不同的字母，这就是拼写的全部意义。

use core::fmt;
use core::str::FromStr;

use crate::error::TheoryError;

/// 音名解析/显示的最大字符数（含八度数位）：`C##-1` 与 `Bbb9` 都在 5 个字符内。
const MAX_NOTE_TEXT_LEN: usize = 5;

/// 规范的音高名称：音级字母 + 变音记号。
///
/// `letter` 是 `0..7` 的音级序号（C=0, D=1, E=2, F=3, G=4, A=5, B=6），
/// `alter` 是变音记号（`-2` 重降、`-1` 降、`0` 本位、`+1` 升、`+2` 重升）。
/// 两者一起才能唯一决定一个 [`PitchClass`]，但一个 `PitchClass` 对应多个 `NoteName`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteName {
    /// 音级字母序号 (C=0 .. B=6)。
    pub letter: u8,
    /// 变音记号半音数 (-2..=2)。
    pub alter: i8,
}

/// 七个音级字母的规范 ASCII 名。
const LETTER_NAMES: [char; 7] = ['C', 'D', 'E', 'F', 'G', 'A', 'B'];

/// 自然音级（本位音）对应的 [`PitchClass`]：C D E F G A B。
const LETTER_PITCH_CLASS: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];

impl NoteName {
    /// 构造一个音名。
    ///
    /// # Errors
    ///
    /// `letter` 不在 `0..7` 或 `alter` 不在 `-2..=2` 时返回
    /// [`TheoryError::NoteNameUnknown`]。
    pub const fn new(letter: u8, alter: i8) -> Result<Self, TheoryError> {
        if letter >= 7 || alter < -2 || alter > 2 {
            return Err(TheoryError::NoteNameUnknown);
        }
        Ok(Self { letter, alter })
    }

    /// 该拼写对应的音级。
    #[must_use]
    pub const fn pitch_class(self) -> PitchClass {
        // 取模 12：Cbb(-2) = 10、B#(+1) = 0 都落在合法区间。
        let raw = LETTER_PITCH_CLASS[self.letter as usize] as i16 + self.alter as i16;
        // `const fn` 里不能调用 `Result::expect`，但 `rem_euclid(12)` 的结果
        // 必然落在 0..12，所以这里直接用不校验的构造器。
        PitchClass::from_raw(raw.rem_euclid(12) as u8)
    }

    /// 变音记号显示文本：`""` / `"#"` / `"##"` / `"b"` / `"bb"`。
    #[must_use]
    pub const fn accidental_text(self) -> &'static str {
        match self.alter {
            -2 => "bb",
            -1 => "b",
            0 => "",
            1 => "#",
            2 => "##",
            _ => "",
        }
    }

    /// 把该拼写搬到另一个音级字母上，同时保持 [`PitchClass`] 不变。
    ///
    /// 变音记号是解出来的，可能落在重升/重降之外：
    /// `B#4` 按字母 `C` 写就是 `C##`（需要 +2，合法）；
    /// 而 `C` 按字母 `B` 写需要 -1（`Cb`），也是合法的。
    /// 只有解出的变音记号超出 `-2..=2` 时才报错。
    ///
    /// # Errors
    ///
    /// 目标音级超出 `0..7`，或所需变音记号超出 `-2..=2` 时返回
    /// [`TheoryError::NoteNameUnknown`]。
    pub const fn with_letter(self, letter: u8) -> Result<Self, TheoryError> {
        if letter >= 7 {
            return Err(TheoryError::NoteNameUnknown);
        }
        let pc = self.pitch_class().semitones() as i16;
        // 口径与 [`PitchClass::name_for_letter`] 一致：rem_euclid 后归一到 -5..=6。
        let raw = (pc - LETTER_PITCH_CLASS[letter as usize] as i16).rem_euclid(12);
        let alter = if raw > 6 { raw - 12 } else { raw };
        Self::new(letter, alter as i8)
    }
}

impl fmt::Display for NoteName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            LETTER_NAMES[self.letter as usize],
            self.accidental_text()
        )
    }
}

/// 带八度的完整音名拼写，例如 `C4`、`C#4`、`Db4`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpelledPitch {
    /// 音名（音级 + 变音记号）。
    pub name: NoteName,
    /// 科学音高记号法的八度号。
    pub octave: i8,
}

impl SpelledPitch {
    /// 构造一个带八度的音名。
    #[must_use]
    pub const fn new(name: NoteName, octave: i8) -> Self {
        Self { name, octave }
    }

    /// 该拼写对应的 MIDI 音高。
    ///
    /// # Errors
    ///
    /// 结果不在 `0..=127` 时返回 [`TheoryError::PitchOutOfRange`]。
    pub fn to_pitch(self) -> Result<Pitch, TheoryError> {
        let value = (self.octave as i32 + 1) * 12 + self.name.pitch_class().semitones() as i32;
        Pitch::new(value)
    }

    /// 用给定音名拼写一个已知 MIDI 音高。
    ///
    /// # Errors
    ///
    /// `name` 与 `pitch` 不在同一八度区间时返回 [`TheoryError::PitchOutOfRange`]。
    pub fn from_pitch_with_name(pitch: Pitch, name: NoteName) -> Result<Self, TheoryError> {
        let pc = name.pitch_class().semitones() as i32;
        // MIDI 0 = C-1，因此 octave = (midi - pc)/12 - 1。
        let diff = pitch.value() as i32 - pc;
        let octave = diff.div_euclid(12) - 1;
        if !(-1..=9).contains(&octave) {
            return Err(TheoryError::PitchOutOfRange {
                value: pitch.value() as u32,
            });
        }
        Ok(Self {
            name,
            octave: octave as i8,
        })
    }
}

impl fmt::Display for SpelledPitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}{}",
            LETTER_NAMES[self.name.letter as usize],
            self.name.accidental_text(),
            self.octave
        )
    }
}

impl FromStr for SpelledPitch {
    type Err = TheoryError;

    /// 解析 `C4` / `C#4` / `Db4` / `Cb-1` 这类科学音高记号写法。
    ///
    /// 大小写不敏感（`c#4` 合法）；变音记号接受 `#`、`b`，也接受 Unicode
    /// `♯` / `♭`。
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_NOTE_TEXT_LEN {
            return Err(TheoryError::NoteNameUnknown);
        }
        let bytes = trimmed.as_bytes();
        let letter = match bytes[0].to_ascii_uppercase() {
            b'C' => 0u8,
            b'D' => 1,
            b'E' => 2,
            b'F' => 3,
            b'G' => 4,
            b'A' => 5,
            b'B' => 6,
            _ => return Err(TheoryError::NoteNameUnknown),
        };
        let mut index = 1usize;
        let mut alter: i8 = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'#' => alter += 1,
                b'b' | b'B' => alter -= 1,
                _ => break,
            }
            index += 1;
        }
        // Unicode ♯ / ♭ 占 2 字节以上，单独处理。
        if index < bytes.len() {
            let rest = &trimmed[index..];
            let mut consumed = 0usize;
            for ch in rest.chars() {
                match ch {
                    '♯' => alter += 1,
                    '♭' => alter -= 1,
                    _ => break,
                }
                consumed += ch.len_utf8();
            }
            index += consumed;
        }
        let name = NoteName::new(letter, alter)?;
        let octave_text = &trimmed[index..];
        if octave_text.is_empty() {
            // 没有八度号：本 crate 的 `Pitch` 必须有八度，因此拒绝。
            return Err(TheoryError::NoteNameUnknown);
        }
        let octave: i8 = octave_text
            .parse()
            .map_err(|_| TheoryError::NoteNameUnknown)?;
        Ok(Self { name, octave })
    }
}

/// 八度等价的音级，取值 `0..12`（C = 0，A = 9）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PitchClass(u8);

impl PitchClass {
    /// 构造一个音级。
    ///
    /// # Errors
    ///
    /// `value >= 12` 时返回 [`TheoryError::PitchOutOfRange`]。
    pub const fn new(value: u8) -> Result<Self, TheoryError> {
        if value >= 12 {
            return Err(TheoryError::PitchOutOfRange {
                value: value as u32,
            });
        }
        Ok(Self(value))
    }

    /// 不做校验的构造，仅限本 crate 内部常量表使用。
    const fn from_raw(value: u8) -> Self {
        Self(value)
    }

    /// 半音序号 `0..12`。
    #[must_use]
    pub const fn semitones(self) -> u8 {
        self.0
    }

    /// 默认拼写：键盘上的自然音级用本位音，黑键一律用升号。
    ///
    /// 需要 `Db` 而不是 `C#` 时，调用方应使用 [`Self::spell_in_key`]。
    #[must_use]
    pub const fn default_name(self) -> NoteName {
        const SHARP_NAMES: [NoteName; 12] = [
            NoteName {
                letter: 0,
                alter: 0,
            },
            NoteName {
                letter: 0,
                alter: 1,
            },
            NoteName {
                letter: 1,
                alter: 0,
            },
            NoteName {
                letter: 1,
                alter: 1,
            },
            NoteName {
                letter: 2,
                alter: 0,
            },
            NoteName {
                letter: 3,
                alter: 0,
            },
            NoteName {
                letter: 3,
                alter: 1,
            },
            NoteName {
                letter: 4,
                alter: 0,
            },
            NoteName {
                letter: 4,
                alter: 1,
            },
            NoteName {
                letter: 5,
                alter: 0,
            },
            NoteName {
                letter: 5,
                alter: 1,
            },
            NoteName {
                letter: 6,
                alter: 0,
            },
        ];
        SHARP_NAMES[self.0 as usize]
    }

    /// 在给定音阶的七个字母拼写里挑选一个字母，使拼写落在 `prefer_flat` 偏好的一侧。
    ///
    /// `letters` 是音阶按度数排列的七个字母序号。返回的字母会优先取
    /// "默认拼写所需变音记号最少"的那个；平局时按 `prefer_flat` 打破。
    ///
    /// # Errors
    ///
    /// `letters` 为空或没有可行的变音记号时返回 [`TheoryError::NoteNameUnknown`]。
    pub fn spell_with_letters(
        self,
        letters: &[u8],
        prefer_flat: bool,
    ) -> Result<NoteName, TheoryError> {
        // 代价 = 变音记号绝对值；平局时用"是否**违背**偏好"打平：
        // 偏好降号时，带升号的候选罚 1、带降号的罚 0。取字典序更小的键，
        // 因此 `prefer_flat` 下 Bb (1,0) 胜过 A# (1,1)。
        let mut best: Option<(u8, u8, NoteName)> = None;
        for &letter in letters {
            let Ok(candidate) = self.name_for_letter(letter) else {
                continue;
            };
            let cost = candidate.alter.unsigned_abs();
            let penalty = u8::from(if prefer_flat {
                candidate.alter > 0
            } else {
                candidate.alter < 0
            });
            let candidate_key = (cost, penalty);
            if best.is_none_or(|(best_cost, best_penalty, _)| {
                candidate_key < (best_cost, best_penalty)
            }) {
                best = Some((cost, penalty, candidate));
            }
        }
        best.map(|(_, _, name)| name)
            .ok_or(TheoryError::AmbiguousSpelling)
    }

    /// 用指定字母拼写本音级。
    ///
    /// # Errors
    ///
    /// 需要超过重升/重降的变音记号时返回 [`TheoryError::NoteNameUnknown`]。
    pub const fn name_for_letter(self, letter: u8) -> Result<NoteName, TheoryError> {
        if letter >= 7 {
            return Err(TheoryError::NoteNameUnknown);
        }
        // 与 [`NoteName::with_letter`] 同一口径：先取最小有向差，再按八度
        // 调整到最近的写法，只有重升/重降之外才报错。
        let raw = (self.0 as i16 - LETTER_PITCH_CLASS[letter as usize] as i16).rem_euclid(12);
        // rem_euclid 后落在 0..11；> 6 时减去 12 得到 -5..=-1，即"最接近的写法"。
        let alter = if raw > 6 { raw - 12 } else { raw };
        if alter < -2 || alter > 2 {
            return Err(TheoryError::NoteNameUnknown);
        }
        Ok(NoteName {
            letter,
            alter: alter as i8,
        })
    }

    /// 向上移调 `semitones` 个半音（对 12 取模）。
    ///
    /// 加法在 `i32` 上做：`self` 最大 11，`semitones` 可以取 `i16::MAX`
    /// （`11 + 32767` 已经超出 `i16`），在 `i16` 上相加会溢出
    /// （debug 下 panic，release 下回绕后给出错的音级）。
    #[must_use]
    pub fn transpose(self, semitones: i16) -> Self {
        Self((i32::from(self.0) + i32::from(semitones)).rem_euclid(12) as u8)
    }

    /// 从 `self` 到 `other` 的**上行**音程半音数（0..12）。
    #[must_use]
    pub fn interval_up_to(self, other: Self) -> u32 {
        u32::from((other.0 + 12 - self.0) % 12)
    }

    /// 五度圈上的位置：`(self * 7) mod 12`，C=0、G=1、D=2 …
    #[must_use]
    pub const fn fifth_circle_index(self) -> u8 {
        (self.0 * 7) % 12
    }

    /// 与另一音级的五度圈距离（0..=6，无方向；三全音为 6）。
    #[must_use]
    pub fn fifth_circle_distance(self, other: Self) -> u8 {
        let a = self.fifth_circle_index();
        let b = other.fifth_circle_index();
        let diff = (a + 12 - b) % 12;
        if diff > 6 { 12 - diff } else { diff }
    }

    /// 常用音级常量。
    pub const C: Self = Self::from_raw(0);
    /// D。
    pub const D: Self = Self::from_raw(2);
    /// E。
    pub const E: Self = Self::from_raw(4);
    /// F。
    pub const F: Self = Self::from_raw(5);
    /// G。
    pub const G: Self = Self::from_raw(7);
    /// A。
    pub const A: Self = Self::from_raw(9);
    /// B。
    pub const B: Self = Self::from_raw(11);
    /// C♯ / D♭。
    pub const CS: Self = Self::from_raw(1);
    /// D♯ / E♭。
    pub const DS: Self = Self::from_raw(3);
    /// F♯ / G♭。
    pub const FS: Self = Self::from_raw(6);
    /// G♯ / A♭。
    pub const GS: Self = Self::from_raw(8);
    /// A♯ / B♭。
    pub const AS: Self = Self::from_raw(10);
}

impl fmt::Display for PitchClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.default_name();
        write!(
            f,
            "{}{}",
            LETTER_NAMES[name.letter as usize],
            name.accidental_text()
        )
    }
}

/// 解析只含音级、不含八度号的根音文本，例如 `C`、`F#`、`Bb`、`eb`。
///
/// 和弦符号的根音（`Cmaj7` 的 `C`）与音阶文本（`F# harmonic_minor` 的 `F#`）
/// 都走这里，保证两处的变音记号口径完全一致。
///
/// # Errors
///
/// 文本为空、音级字母非法，或变音记号超出重升/重降时返回
/// [`TheoryError::NoteNameUnknown`]。
///
/// 变音记号在 `i32` 上累加：记号个数就是输入长度，因此本函数对**任意长度**的
/// 输入都不 panic（见 `a_long_run_of_accidentals_is_rejected_instead_of_overflowing`）。
pub fn parse_pitch_class(text: &str) -> Result<PitchClass, TheoryError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(TheoryError::NoteNameUnknown);
    }
    let mut chars = trimmed.chars();
    let letter = match chars.next().expect("non-empty").to_ascii_uppercase() {
        'C' => 0u8,
        'D' => 1,
        'E' => 2,
        'F' => 3,
        'G' => 4,
        'A' => 5,
        'B' => 6,
        _ => return Err(TheoryError::NoteNameUnknown),
    };
    // 累加在 `i32` 上做：变音记号的个数**就是输入长度**，本函数没有
    // `SpelledPitch` 那条 `MAX_NOTE_TEXT_LEN` 上限，在 `i8` 上累加会在第 128 个
    // 同号记号处溢出（debug 与开启溢出检查的 release 档都 panic；Cargo 默认关闭
    // 检查的 release 档回绕 —— 回绕值可能落在 `-2..=2` 内而被当成合法音名）。
    // 区间判定只做一次，且与 `NoteName::new` 的 `-2..=2` 同口径，因此**装得下的
    // 输入与旧口径逐位相同**；饱和加法让任意长度的输入都不 panic。
    let mut alter: i32 = 0;
    for ch in chars {
        match ch {
            '#' | '\u{266f}' => alter = alter.saturating_add(1),
            'b' | 'B' | '\u{266d}' => alter = alter.saturating_sub(1),
            _ => return Err(TheoryError::NoteNameUnknown),
        }
    }
    if !(-2..=2).contains(&alter) {
        return Err(TheoryError::NoteNameUnknown);
    }
    let name = NoteName::new(letter, alter as i8)?;
    Ok(name.pitch_class())
}

/// MIDI 音高，取值 `0..=127`（C-1 = 0，A4 = 69）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pitch(u8);

/// MIDI 音高下界。
pub const MIN_PITCH: u8 = 0;
/// MIDI 音高上界。
pub const MAX_PITCH: u8 = 127;
/// 十二平均律参考音 A4 的 MIDI 音高。
pub const A4_MIDI: u8 = 69;
/// 十二平均律参考频率 A4 = 440 Hz。
pub const A4_HZ: f64 = 440.0;

impl Pitch {
    /// 构造一个 MIDI 音高。
    ///
    /// # Errors
    ///
    /// 超出 `0..=127` 时返回 [`TheoryError::PitchOutOfRange`]。
    pub const fn new(value: i32) -> Result<Self, TheoryError> {
        if value < 0 || value > 127 {
            return Err(TheoryError::PitchOutOfRange {
                value: value.unsigned_abs(),
            });
        }
        Ok(Self(value as u8))
    }

    /// 内部使用的不校验构造：调用方必须已经保证 `value < 128`。
    ///
    /// `const fn` 里不能 panic，因此越界断言只在 debug 构建生效；所有生产
    /// 调用点都来自本模块的常量表或已经过 [`Pitch::new`] 校验的路径。
    pub(crate) const fn from_raw(value: u8) -> Self {
        Self(value & 0x7f)
    }

    /// 内部使用的不校验构造（带 debug 断言版本），用于测试辅助代码。
    const fn from_midi(value: u8) -> Self {
        debug_assert!(value < 128);
        Self(value & 0x7f)
    }

    /// MIDI 音高值。
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }

    /// 八度等价的音级。
    #[must_use]
    pub const fn pitch_class(self) -> PitchClass {
        PitchClass::from_raw(self.0 % 12)
    }

    /// 科学音高记号法的八度号（C4 = 60 → 4）。
    #[must_use]
    pub const fn octave(self) -> i8 {
        // MIDI 0 = C-1。
        ((self.0 / 12) as i8) - 1
    }

    /// 与另一音高的**有向**半音差（`other - self`）。
    #[must_use]
    pub const fn semitone_distance_to(self, other: Self) -> i16 {
        other.0 as i16 - self.0 as i16
    }

    /// 与另一音高的**无向**音程半音数。
    #[must_use]
    pub const fn abs_distance_to(self, other: Self) -> u8 {
        self.0.abs_diff(other.0)
    }

    /// 移调，结果越界则报错。
    ///
    /// # Errors
    ///
    /// 结果不在 `0..=127` 时返回 [`TheoryError::PitchOutOfRange`]。
    pub const fn transposed(self, semitones: i16) -> Result<Self, TheoryError> {
        Self::new(self.0 as i32 + semitones as i32)
    }

    /// 默认升号拼写。
    #[must_use]
    pub const fn default_name(self) -> NoteName {
        self.pitch_class().default_name()
    }

    /// 默认拼写的完整音名（例如 `C4`、`C#4`）。
    #[must_use]
    pub const fn default_spelling(self) -> SpelledPitch {
        SpelledPitch {
            name: self.default_name(),
            octave: self.octave(),
        }
    }

    /// 任意拼写对应的频率（Hz），十二平均律。
    #[must_use]
    pub fn to_hz(self) -> f64 {
        note_to_hz(self.0)
    }
}

impl fmt::Display for Pitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.default_spelling())
    }
}

/// 为每个八度生成常用音高常量：`Pitch::C4` = MIDI 60。
macro_rules! pitch_constants {
    ($base:literal;) => {};
    ($base:literal; $name:ident, $offset:literal $(, $rest_name:ident, $rest_offset:literal)*) => {
        impl Pitch {
            #[doc = concat!("MIDI 音高常量：", stringify!($name), "。")]
            pub const $name: Self = Self::from_raw($base + $offset);
        }
        pitch_constants!($base; $($rest_name, $rest_offset),*);
    };
}

// C-1 = 0，因此第 n 个八度的 C 是 12 * (n + 1)。
pitch_constants!(12; C0, 0, CS0, 1, D0, 2, DS0, 3, E0, 4, F0, 5, FS0, 6, G0, 7, GS0, 8, A0, 9, AS0, 10, B0, 11);
pitch_constants!(24; C1, 0, CS1, 1, D1, 2, DS1, 3, E1, 4, F1, 5, FS1, 6, G1, 7, GS1, 8, A1, 9, AS1, 10, B1, 11);
pitch_constants!(36; C2, 0, CS2, 1, D2, 2, DS2, 3, E2, 4, F2, 5, FS2, 6, G2, 7, GS2, 8, A2, 9, AS2, 10, B2, 11);
pitch_constants!(48; C3, 0, CS3, 1, D3, 2, DS3, 3, E3, 4, F3, 5, FS3, 6, G3, 7, GS3, 8, A3, 9, AS3, 10, B3, 11);
pitch_constants!(60; C4, 0, CS4, 1, D4, 2, DS4, 3, E4, 4, F4, 5, FS4, 6, G4, 7, GS4, 8, A4, 9, AS4, 10, B4, 11);
pitch_constants!(72; C5, 0, CS5, 1, D5, 2, DS5, 3, E5, 4, F5, 5, FS5, 6, G5, 7, GS5, 8, A5, 9, AS5, 10, B5, 11);
pitch_constants!(84; C6, 0, CS6, 1, D6, 2, DS6, 3, E6, 4, F6, 5, FS6, 6, G6, 7, GS6, 8, A6, 9, AS6, 10, B6, 11);
pitch_constants!(96; C7, 0, CS7, 1, D7, 2, DS7, 3, E7, 4, F7, 5, FS7, 6, G7, 7, GS7, 8, A7, 9, AS7, 10, B7, 11);
pitch_constants!(108; C8, 0, CS8, 1, D8, 2, DS8, 3, E8, 4, F8, 5, FS8, 6, G8, 7, GS8, 8, A8, 9, AS8, 10, B8, 11);
pitch_constants!(120; C9, 0, CS9, 1, D9, 2, DS9, 3, E9, 4, F9, 5, FS9, 6, G9, 7);

/// 十二平均律频率：`hz = 440 * 2^((midi - 69)/12)`。
///
/// 使用 `libm::pow` 而非 `f64::powf`，以对齐 [ARCH-DET-001] 对
/// "统一启用纯 Rust `libm` 数学库"的要求（跨 x86_64 / AArch64 更稳定）。
#[must_use]
pub fn note_to_hz(midi: u8) -> f64 {
    let exponent = (f64::from(midi) - f64::from(A4_MIDI)) / 12.0;
    A4_HZ * libm::pow(2.0, exponent)
}

/// 与 [`note_to_hz`] 互逆：把频率换算回最近的 MIDI 音高。
///
/// 越界频率会被夹到 `0..=127`；非正/NaN 输入返回 `Err`。
///
/// # Errors
///
/// `hz` 不是有限正数时返回 [`TheoryError::PitchOutOfRange`]。
pub fn hz_to_note(hz: f64) -> Result<Pitch, TheoryError> {
    if !hz.is_finite() || hz <= 0.0 {
        return Err(TheoryError::PitchOutOfRange { value: 0 });
    }
    let midi = 69.0 + 12.0 * libm::log2(hz / A4_HZ);
    let rounded = libm::round(midi);
    if rounded < 0.0 {
        return Ok(Pitch::from_midi(0));
    }
    if rounded > 127.0 {
        return Ok(Pitch::from_midi(127));
    }
    Ok(Pitch::from_midi(rounded as u8))
}

/// 音程：半音数（0..=12）加规范名称。
///
/// 名称采用**英文缩写**（`P1` `m3` `P5` `M7` `A4` …），这是和声学教材的通用写法，
/// 不依赖任何特定语言的翻译表。`P` 完全、`M` 大、`m` 小、`A` 增、`d` 减。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Interval {
    /// 半音数 `0..=12`。
    semitones: u8,
    /// 规范名称（`'static` 引用，零堆分配、零生命周期烦恼）。
    name: &'static str,
    /// 该音程是否为协和音程。
    consonant: bool,
}

impl Interval {
    /// 构造一个音程。
    ///
    /// # Errors
    ///
    /// `semitones > 12` 时返回 [`TheoryError::PitchOutOfRange`]（本 crate 只用
    /// 单八度内音程；复音程由调用方叠加八度表示）。
    pub const fn new(semitones: u8) -> Result<Self, TheoryError> {
        if semitones > 12 {
            return Err(TheoryError::PitchOutOfRange {
                value: semitones as u32,
            });
        }
        Ok(Self {
            semitones,
            name: interval_name(semitones),
            consonant: is_consonant(semitones),
        })
    }

    /// 不做校验的构造，仅限常量表使用。
    const fn from_raw(semitones: u8) -> Self {
        Self {
            semitones,
            name: interval_name(semitones),
            consonant: is_consonant(semitones),
        }
    }

    /// 半音数。
    #[must_use]
    pub const fn semitones(self) -> u8 {
        self.semitones
    }

    /// 规范名称（英文缩写）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// 是否协和（完全协和 + 不完全协和）。
    #[must_use]
    pub const fn is_consonant(self) -> bool {
        self.consonant
    }

    /// 转位：`12 - semitones`。纯一度 ↔ 纯八度、大三度 ↔ 小六度。
    #[must_use]
    pub const fn inverted(self) -> Self {
        Self::from_raw(12 - self.semitones)
    }

    /// 完全一度。
    pub const UNISON: Self = Self::from_raw(0);
    /// 小二度。
    pub const MINOR_SECOND: Self = Self::from_raw(1);
    /// 大二度。
    pub const MAJOR_SECOND: Self = Self::from_raw(2);
    /// 小三度。
    pub const MINOR_THIRD: Self = Self::from_raw(3);
    /// 大三度。
    pub const MAJOR_THIRD: Self = Self::from_raw(4);
    /// 完全四度。
    pub const PERFECT_FOURTH: Self = Self::from_raw(5);
    /// 增四度 / 减五度（三全音）。
    pub const TRITONE: Self = Self::from_raw(6);
    /// 完全五度。
    pub const PERFECT_FIFTH: Self = Self::from_raw(7);
    /// 小六度。
    pub const MINOR_SIXTH: Self = Self::from_raw(8);
    /// 大六度。
    pub const MAJOR_SIXTH: Self = Self::from_raw(9);
    /// 小七度。
    pub const MINOR_SEVENTH: Self = Self::from_raw(10);
    /// 大七度。
    pub const MAJOR_SEVENTH: Self = Self::from_raw(11);
    /// 纯八度。
    pub const PERFECT_OCTAVE: Self = Self::from_raw(12);

    /// 半音数 → 音程，`0..=12` 之外返回 `None`。
    #[must_use]
    pub const fn from_semitones(semitones: u8) -> Option<Self> {
        if semitones > 12 {
            None
        } else {
            Some(Self::from_raw(semitones))
        }
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 标准音程缩写表；下标即半音数。
const INTERVAL_NAMES: [&str; 13] = [
    "P1", "m2", "M2", "m3", "M3", "P4", "A4", "P5", "m6", "M6", "m7", "M7", "P8",
];

/// 协和音程表：完全协和 (P1/P4/P5/P8) + 不完全协和 (m3/M3/m6/M6)。
const CONSONANT: [bool; 13] = [
    true, false, false, true, true, true, false, true, true, true, false, false, true,
];

/// 半音数 → 规范名称。
const fn interval_name(semitones: u8) -> &'static str {
    INTERVAL_NAMES[semitones as usize]
}

/// 查表判断协和性。
const fn is_consonant(semitones: u8) -> bool {
    CONSONANT[semitones as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_is_440_hz_and_middle_c_matches_the_standard_table() {
        assert!((note_to_hz(69) - 440.0).abs() < 1e-9);
        // 中央 C (MIDI 60) = 261.6255653... Hz
        assert!((note_to_hz(60) - 261.625_565_300_597_9).abs() < 1e-9);
        // 每低一个八度频率减半
        assert!((note_to_hz(57) - 220.0).abs() < 1e-9);
        assert!((note_to_hz(81) - 880.0).abs() < 1e-9);
    }

    #[test]
    fn hz_to_note_round_trips_every_midi_note() {
        for midi in 0u8..=127 {
            let hz = note_to_hz(midi);
            assert_eq!(hz_to_note(hz).unwrap().value(), midi, "midi {midi}");
        }
        assert!(hz_to_note(0.0).is_err());
        assert!(hz_to_note(f64::NAN).is_err());
    }

    /// 形态 D 注入实测（本票）：把 `Pitch::pitch_class` 的 `% 12` 换成 `% 11`
    /// 时，既有判据里**只有**和弦构成音与声部连接变红 —— 那条路径的读数来自
    /// [`PitchClass`] 的比较，不是本函数的直接判据。这条判据把 `Pitch::pitch_class`
    /// 与 `Pitch::octave` 的口径直接钉在 **MIDI 0..=127 全域**上。
    #[test]
    fn pitch_class_and_octave_are_pinned_across_the_whole_midi_domain() {
        for midi in 0u8..=127 {
            let pitch = Pitch::new(i32::from(midi)).unwrap();
            assert_eq!(pitch.pitch_class().semitones(), midi % 12, "midi {midi}");
            // MIDI 0 = C-1，因此八度号是 `midi / 12 - 1`。
            assert_eq!(pitch.octave(), (midi as i8 / 12) - 1, "midi {midi}");
        }
        // 端点上的字面读数：0 = C-1、127 = G9。
        assert_eq!(Pitch::new(0).unwrap().default_spelling().to_string(), "C-1");
        assert_eq!(
            Pitch::new(127).unwrap().default_spelling().to_string(),
            "G9"
        );
        assert_eq!(Pitch::new(127).unwrap().pitch_class(), PitchClass::G);
    }

    /// 形态 D 注入实测（本票）：把 `MAX_PITCH` 由 `127` 改成 `126` 时没有
    /// 任何既有判据变红（属性测试只断言"落在 `0..=127` 内"，常量本身没被钉住）。
    /// 这条判据把上界常量与真实拒绝阈值一起钉死：`127` 合法、`128` 必须报错。
    #[test]
    fn midi_pitch_bounds_are_pinned_to_literals() {
        assert_eq!(MIN_PITCH, 0);
        assert_eq!(MAX_PITCH, 127);
        assert_eq!(A4_MIDI, 69);
        assert_eq!(Pitch::new(127).unwrap().value(), MAX_PITCH);
        assert_eq!(
            Pitch::new(128).unwrap_err(),
            TheoryError::PitchOutOfRange { value: 128 }
        );
        assert_eq!(
            Pitch::new(-1).unwrap_err(),
            TheoryError::PitchOutOfRange { value: 1 }
        );
        // 频率换算是上界的另一个出口：127 必须仍能从频率还原。
        assert_eq!(
            hz_to_note(note_to_hz(MAX_PITCH)).unwrap().value(),
            MAX_PITCH
        );
    }

    /// 形态 D 注入实测（本票）：`SpelledPitch::from_str` 的 **Unicode** 分支
    /// （`♯` / `♭`）没有判据：既有判据只喂 ASCII 的 `#` / `b`。
    ///
    /// ⚠ 本票实测到一个**未修复的缺陷**（只登记，不改口径）：
    /// `"C♯♯4"` 这类"Unicode 记号后面还有八度号"的输入会让 `from_str` 在
    /// **多字节字符内部**切字符串（`&trimmed[index..]` 的 `index` 落在 `♯`
    /// 的字节中间），从而 panic（`byte index 2 is not a char boundary`）。
    /// 同一函数对 ASCII 的 `"C##4"` 正常返回 `D4`，因此这是 Unicode 分支
    /// 独有的崩溃路径。修它要改 `from_str` 的索引推进方式，会让"第二个
    /// Unicode 记号之后的内容"的读数从 panic 变成别的值 ⇒ 按读数纪律
    /// **只登记，不硬改**，留待裁决。
    ///
    /// 因此这条判据只喂**能正常返回**的 Unicode 输入（单个记号），
    /// 外加 ASCII 侧的完整读数。
    #[test]
    fn unicode_accidentals_move_the_pitch_class_in_their_own_direction() {
        // `SpelledPitch` 走 Unicode 表；`C♯4` 与 `C#4` 必须同值。
        assert_eq!(
            "C\u{266f}4".parse::<SpelledPitch>().unwrap(),
            "C#4".parse::<SpelledPitch>().unwrap()
        );
        assert_eq!(
            "D\u{266d}4".parse::<SpelledPitch>().unwrap(),
            "Db4".parse::<SpelledPitch>().unwrap()
        );
        assert_eq!(
            "C\u{266f}4".parse::<SpelledPitch>().unwrap().to_string(),
            "C#4"
        );
        assert_eq!(
            "D\u{266d}4".parse::<SpelledPitch>().unwrap().to_string(),
            "Db4"
        );
        // ASCII 侧的复合记号：**音名文本按入参保留**（`SpelledPitch` 是拼写
        // 而不是归一化器），但对应的 MIDI 音高必须正确。
        assert_eq!("C##4".parse::<SpelledPitch>().unwrap().to_string(), "C##4");
        assert_eq!(
            "C##4".parse::<SpelledPitch>().unwrap().to_pitch().unwrap(),
            Pitch::D4
        );
        assert_eq!("Dbb4".parse::<SpelledPitch>().unwrap().to_string(), "Dbb4");
        assert_eq!(
            "Dbb4".parse::<SpelledPitch>().unwrap().to_pitch().unwrap(),
            Pitch::C4
        );
        assert_eq!(
            "Dbbb4".parse::<SpelledPitch>(),
            Err(TheoryError::NoteNameUnknown)
        );
        // `parse_pitch_class` 走 ASCII 的 `#` / `b`（`B` 也算降号）。
        assert_eq!(parse_pitch_class("C#").unwrap(), PitchClass::CS);
        assert_eq!(parse_pitch_class("Cb").unwrap(), PitchClass::B);
        assert_eq!(parse_pitch_class("Dbb").unwrap(), PitchClass::C);
        assert_eq!(parse_pitch_class("B#").unwrap(), PitchClass::C);
        assert_eq!(parse_pitch_class("Cbb").unwrap(), PitchClass::AS);
        assert_eq!(parse_pitch_class("Cbbb"), Err(TheoryError::NoteNameUnknown));
        assert_eq!(parse_pitch_class("D###"), Err(TheoryError::NoteNameUnknown));
    }

    /// 形态 D 注入实测（本票）：`name_for_letter` 的 `raw > 6` 边界改成
    /// `raw >= 6` 或 `raw > 7` 时没有判据变红。本机实测的原因：`raw` 属于
    /// `{6, 7}` 的全部 14 个 `(音级, 字母)` 组合都走**同一条**出口 ——
    /// `raw` 落在 `> 6` 一侧时减 12 得到 `-5`，在 `-2..=2` 之外；
    /// 而 `raw == 6` 的组合会让 `pc - LETTER_PC` 落在 `±6`，减 12 后是 `-6`，
    /// 也在区间之外。因此两条分支在**全部 84 个组合**上给出同一读数，
    /// 该边界在观测上等价（这是本机实测的结论，不是推理）。
    ///
    /// 能观测到的是这张 35 个"可拼"组合与 49 个"拒绝"组合的分界
    /// （全部来自本机实测）：下面把**每一个**可拼组合的字面拼写钉住，
    /// 并逐点复核其余 49 个必须被拒绝。
    #[test]
    fn name_for_letter_separates_the_spellable_side_from_the_rejected_side() {
        let spell = |pc: u8, letter: u8| {
            PitchClass::new(pc)
                .unwrap()
                .name_for_letter(letter)
                .map(|name| name.to_string())
        };
        // 35 个可拼组合的完整字面读数（本机实测）。
        let spellable: [(u8, u8, &str); 35] = [
            (0, 0, "C"),
            (0, 1, "Dbb"),
            (0, 6, "B#"),
            (1, 0, "C#"),
            (1, 1, "Db"),
            (1, 6, "B##"),
            (2, 0, "C##"),
            (2, 1, "D"),
            (2, 2, "Ebb"),
            (3, 1, "D#"),
            (3, 2, "Eb"),
            (3, 3, "Fbb"),
            (4, 1, "D##"),
            (4, 2, "E"),
            (4, 3, "Fb"),
            (5, 2, "E#"),
            (5, 3, "F"),
            (5, 4, "Gbb"),
            (6, 2, "E##"),
            (6, 3, "F#"),
            (6, 4, "Gb"),
            (7, 3, "F##"),
            (7, 4, "G"),
            (7, 5, "Abb"),
            (8, 4, "G#"),
            (8, 5, "Ab"),
            (9, 4, "G##"),
            (9, 5, "A"),
            (9, 6, "Bbb"),
            (10, 0, "Cbb"),
            (10, 5, "A#"),
            (10, 6, "Bb"),
            (11, 0, "Cb"),
            (11, 5, "A##"),
            (11, 6, "B"),
        ];
        let mut expected_ok = [false; 84];
        for (pc, letter, name) in spellable {
            assert_eq!(spell(pc, letter).unwrap(), name, "pc {pc} letter {letter}");
            expected_ok[usize::from(pc) * 7 + usize::from(letter)] = true;
        }
        // 其余组合（49 个）必须一律 `NoteNameUnknown`，一个都不许回绕。
        for pc in 0u8..12 {
            for letter in 0u8..7 {
                if !expected_ok[usize::from(pc) * 7 + usize::from(letter)] {
                    assert_eq!(
                        spell(pc, letter),
                        Err(TheoryError::NoteNameUnknown),
                        "pc {pc} letter {letter} must be rejected"
                    );
                }
            }
        }
        // 边界另一侧：`letter >= 7` 一律 `NoteNameUnknown`。
        assert_eq!(spell(0, 7), Err(TheoryError::NoteNameUnknown));
        assert_eq!(spell(0, 255), Err(TheoryError::NoteNameUnknown));
    }

    #[test]
    fn note_names_parse_both_sharp_and_flat_spellings() {
        assert_eq!(parse_pitch_class("C#").unwrap(), PitchClass::CS);
        assert_eq!(parse_pitch_class("Db").unwrap(), PitchClass::CS);
        assert_eq!(parse_pitch_class("bb").unwrap(), PitchClass::AS);
        assert_eq!(parse_pitch_class("e#").unwrap(), PitchClass::F);
        assert!(parse_pitch_class("").is_err());
        assert!(parse_pitch_class("Cx").is_err());
        assert!(parse_pitch_class("H").is_err());
        let c_sharp: SpelledPitch = "C#4".parse().unwrap();
        let d_flat: SpelledPitch = "Db4".parse().unwrap();
        assert_eq!(c_sharp.to_pitch().unwrap().value(), 61);
        assert_eq!(d_flat.to_pitch().unwrap().value(), 61);
        assert_eq!(c_sharp.to_string(), "C#4");
        assert_eq!(d_flat.to_string(), "Db4");
        assert_eq!("c#4".parse::<SpelledPitch>().unwrap(), c_sharp);
        assert_eq!("C♯4".parse::<SpelledPitch>().unwrap(), c_sharp);
        assert_eq!(
            "c".parse::<SpelledPitch>(),
            Err(TheoryError::NoteNameUnknown)
        );
        assert_eq!(
            "H4".parse::<SpelledPitch>(),
            Err(TheoryError::NoteNameUnknown)
        );
    }

    /// 变音记号的个数**就是输入长度**，因此这是"极长入参"上的累加极值：
    /// `parse_pitch_class` 只有它自己这一条解析路径，没有 `SpelledPitch` 那样的
    /// 长度上限（后者见 `MAX_NOTE_TEXT_LEN`）。
    ///
    /// 旧码把变音记号累加进 `i8`：第 128 个**同号**记号让它溢出（debug 与开启
    /// 溢出检查的 release 档都 panic，关闭检查的 release 档回绕），而
    /// `NoteName::new` 的 `-2..=2` 校验在**之后**才跑。新码在 `i32` 上累加、
    /// 只在结尾判一次区间 ⇒ 装得下的输入与旧口径逐位相同，装不下的输入被拒绝
    /// 而不是回绕。
    #[test]
    fn a_long_run_of_accidentals_is_rejected_instead_of_overflowing() {
        // 下界 0 与上界"同号记号多到溢出"都要有明确读数。
        for count in [0usize, 1, 2, 3, 127, 128, 129, 255, 256, 4096] {
            let sharps = format!("C{}", "#".repeat(count));
            let flats = format!("C{}", "b".repeat(count));
            assert_eq!(
                parse_pitch_class(&sharps).is_ok(),
                count <= 2,
                "{count} sharps must be accepted iff within double-sharp"
            );
            assert_eq!(
                parse_pitch_class(&flats).is_ok(),
                count <= 2,
                "{count} flats must be accepted iff within double-flat"
            );
        }

        // 逐位一致读数：**旧码不 panic** 的 36 组输入（前缀全程留在 `i8` 值域内）
        // 上，新口径必须与旧口径给同一个答案 —— 旧口径的裁决就是
        // `NoteName::new` 的 `-2..=2`。这里独立复算净值，不复用被测实现。
        let mut compared = 0usize;
        for sharps in [0usize, 1, 2, 3, 62, 127] {
            for flats in [0usize, 1, 2, 3, 62, 127] {
                let text = format!("C{}{}", "#".repeat(sharps), "b".repeat(flats));
                let net = sharps as i32 - flats as i32;
                assert_eq!(
                    parse_pitch_class(&text).is_ok(),
                    (-2..=2).contains(&net),
                    "{sharps} sharps then {flats} flats (net {net})"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 36);

        // 混号但净值仍在重升/重降内：这条输入旧码**不** panic（先升到 127 再降
        // 回 0），因此读数必须逐位保留。
        let net_zero = format!("C{}{}", "#".repeat(127), "b".repeat(127));
        assert_eq!(parse_pitch_class(&net_zero).unwrap(), PitchClass::C);
        let net_two = format!("C{}{}", "#".repeat(127), "b".repeat(125));
        assert_eq!(parse_pitch_class(&net_two).unwrap(), PitchClass::D);
    }

    /// 变音记号对**音级读数**的作用量必须恰好是 1 个半音。
    ///
    /// 注入实测（theory-16 形态 D）：把 `parse_pitch_class` 里升号的
    /// `saturating_add(1)` 改成 `saturating_add(2)`，全部既有判据保持全绿 ——
    /// 既有的极值判据只用"接受/拒绝"这一个二值读数（`C#` 与 `C##` 都仍然
    /// 被接受，只是落在错的音级上），没有任何一条钉住"一个记号 = 一个半音"。
    /// 危害：`C#` 会被读成 D，`C#` 与 `Db` 不再同音，全部以文本音名解析的
    /// 入口（和弦符号、音阶名、MCP 文本）都会静默移调。
    #[test]
    fn one_accidental_moves_the_pitch_class_by_exactly_one_semitone() {
        let sharp = |n: usize| parse_pitch_class(&format!("C{}", "#".repeat(n))).unwrap();
        let flat = |n: usize| parse_pitch_class(&format!("C{}", "b".repeat(n))).unwrap();
        let unicode_sharp = parse_pitch_class("C\u{266f}").unwrap();
        let unicode_flat = parse_pitch_class("C\u{266d}").unwrap();

        assert_eq!(parse_pitch_class("C").unwrap().semitones(), 0);
        assert_eq!(sharp(1).semitones(), 1);
        assert_eq!(sharp(2).semitones(), 2);
        assert_eq!(flat(1).semitones(), 11);
        assert_eq!(flat(2).semitones(), 10);
        assert_eq!(unicode_sharp, sharp(1));
        assert_eq!(unicode_flat, flat(1));
        // 同音异名的两支必须落在同一个音级（一个记号 = 一个半音的直接推论）。
        assert_eq!(sharp(1), flat(1).transpose(2));
        // 一个记号的净值与 `NoteName::new` 的 `alter` 语义一致。
        assert_eq!(NoteName::new(0, 1).unwrap().pitch_class(), sharp(1));
        assert_eq!(NoteName::new(0, -1).unwrap().pitch_class(), flat(1));
        // 三个同号记号仍然越界（回归护栏）。
        assert!(parse_pitch_class("C###").is_err());
        assert!(parse_pitch_class("Cbbb").is_err());
    }

    #[test]
    fn octave_arithmetic_matches_scientific_pitch_notation() {
        assert_eq!(
            "C4".parse::<SpelledPitch>()
                .unwrap()
                .to_pitch()
                .unwrap()
                .value(),
            60
        );
        assert_eq!(Pitch::new(60).unwrap().octave(), 4);
        assert_eq!(
            "C-1"
                .parse::<SpelledPitch>()
                .unwrap()
                .to_pitch()
                .unwrap()
                .value(),
            0
        );
        assert_eq!(
            "G9".parse::<SpelledPitch>()
                .unwrap()
                .to_pitch()
                .unwrap()
                .value(),
            127
        );
        // 越界：B9 是 131
        assert!("B9".parse::<SpelledPitch>().unwrap().to_pitch().is_err());
    }

    #[test]
    fn pitch_class_transposition_and_interval_are_integer_only() {
        assert_eq!(PitchClass::C.transpose(7), PitchClass::G);
        assert_eq!(PitchClass::B.transpose(1), PitchClass::C);
        assert_eq!(PitchClass::C.transpose(-1), PitchClass::B);
        assert_eq!(PitchClass::C.interval_up_to(PitchClass::A), 9);
        assert_eq!(PitchClass::A.interval_up_to(PitchClass::C), 3);
    }

    #[test]
    fn fifth_circle_is_the_circle_of_fifths() {
        let order = [
            PitchClass::C,
            PitchClass::G,
            PitchClass::D,
            PitchClass::A,
            PitchClass::E,
            PitchClass::B,
            PitchClass::FS,
        ];
        for (index, pc) in order.iter().enumerate() {
            assert_eq!(usize::from(pc.fifth_circle_index()), index, "{pc}");
        }
        assert_eq!(PitchClass::C.fifth_circle_distance(PitchClass::G), 1);
        assert_eq!(PitchClass::C.fifth_circle_distance(PitchClass::F), 1);
        assert_eq!(PitchClass::C.fifth_circle_distance(PitchClass::FS), 6);
        assert_eq!(PitchClass::C.fifth_circle_distance(PitchClass::C), 0);
    }

    #[test]
    fn intervals_carry_names_and_invert() {
        assert_eq!(Interval::new(0).unwrap().name(), "P1");
        assert_eq!(Interval::new(7).unwrap().name(), "P5");
        assert_eq!(Interval::new(12).unwrap().name(), "P8");
        assert_eq!(Interval::MAJOR_THIRD.inverted().name(), "m6");
        assert_eq!(Interval::PERFECT_FIFTH.inverted().name(), "P4");
        assert_eq!(Interval::TRITONE.inverted(), Interval::TRITONE);
        assert!(Interval::PERFECT_FIFTH.is_consonant());
        assert!(!Interval::MAJOR_SECOND.is_consonant());
        assert!(Interval::new(13).is_err());
    }

    #[test]
    fn absolute_note_names_spell_the_black_keys_as_sharps() {
        assert_eq!(PitchClass::CS.to_string(), "C#");
        assert_eq!(PitchClass::AS.to_string(), "A#");
        // C 用字母 D 写就是 Dbb（重降），这正是"音级 + 变音记号"的表达力。
        assert_eq!(
            PitchClass::C.name_for_letter(1).unwrap(),
            NoteName {
                letter: 1,
                alter: -2
            }
        );
        assert_eq!(PitchClass::C.name_for_letter(1).unwrap().to_string(), "Dbb");
        // C 用字母 E 写需要 -4 个半音，超出重降范围 → 拒绝。
        assert_eq!(
            PitchClass::C.name_for_letter(2),
            Err(TheoryError::NoteNameUnknown)
        );
        // Db 用字母 D 写只需要一个降号。
        assert_eq!(PitchClass::CS.name_for_letter(1).unwrap().to_string(), "Db");
        // B# 与 C 是同一个音级，按字母 C 写不需要任何变音记号。
        let b_sharp = NoteName::new(6, 1).unwrap();
        assert_eq!(b_sharp.pitch_class(), PitchClass::C);
        assert_eq!(b_sharp.with_letter(0).unwrap().to_string(), "C");
        assert_eq!(PitchClass::AS.default_name().to_string(), "A#");
    }

    #[test]
    fn transpose_is_a_modular_rotation_over_the_whole_i16_range() {
        // `self.0 + semitones` 在 `i16` 上相加会溢出（`11 + i16::MAX`）：
        // debug 下 panic、release 下回绕后给出错的音级。这条判据在**整段**
        // `i16` 值域上钉住"对 12 取模的旋转"这一语义。
        for pc in 0u8..12 {
            let pitch_class = PitchClass::new(pc).unwrap();
            for semitones in -32768i32..=32767 {
                let semitones = semitones as i16;
                let expected = (i32::from(pc) + i32::from(semitones)).rem_euclid(12) as u8;
                assert_eq!(
                    pitch_class.transpose(semitones).semitones(),
                    expected,
                    "{pitch_class:?} + {semitones}"
                );
            }
        }
        // 逐位一致读数：两个端点。
        assert_eq!(PitchClass::B.transpose(i16::MAX), PitchClass::FS);
        // -32768 mod 12 == 4：C 向下 32768 个半音落在 E。
        assert_eq!(PitchClass::C.transpose(i16::MIN), PitchClass::E);
    }

    /// 形态 D 注入实测（本票）：三个公开边界判定的**上界那一侧**没有被任何
    /// 既有判据读到 —— 既有判据只喂域内的值。三条单行放宽
    /// （`NoteName::new` 的 `letter >= 7` → `> 7`、`NoteName::with_letter` 的
    /// 同名比较 → `> 7`、`PitchClass::new` 的 `value >= 12` → `> 12`）
    /// 全部保持既有判据全绿；而放宽之后越界值会被真的构造出来，随后在
    /// `LETTER_PITCH_CLASS[..]` / `LETTER_NAMES[..]` / `SHARP_NAMES[..]`
    /// 上越界索引（panic）。
    ///
    /// 口径：三条边界的合法侧都是**闭**的、非法侧从下一个整数开始。
    /// 上界与下界都显式钉住，且 `alter` 的 `-2..=2` 也一起对账。
    #[test]
    fn the_letter_and_pitch_class_bounds_are_closed_on_the_legal_side() {
        // `NoteName::new`：字母 0..7、变音记号 -2..=2。
        for letter in 0u8..7 {
            assert!(
                NoteName::new(letter, 0).is_ok(),
                "letter {letter} must be legal"
            );
        }
        for alter in -2i8..=2 {
            assert!(
                NoteName::new(0, alter).is_ok(),
                "alter {alter} must be legal"
            );
        }
        assert_eq!(
            NoteName::new(7, 0).unwrap_err(),
            TheoryError::NoteNameUnknown
        );
        assert_eq!(
            NoteName::new(u8::MAX, 0).unwrap_err(),
            TheoryError::NoteNameUnknown
        );
        assert_eq!(
            NoteName::new(0, 3).unwrap_err(),
            TheoryError::NoteNameUnknown
        );
        assert_eq!(
            NoteName::new(0, -3).unwrap_err(),
            TheoryError::NoteNameUnknown
        );

        // `NoteName::with_letter`：目标字母同样是 0..7。并非每个目标字母都能
        // 在重升/重降之内表示（那一层由 `NoteName::new` 的第二道校验负责），
        // 因此这里只钉**字母边界**：可表示时给 Ok，越界字母恒为 Err。
        let b_sharp = NoteName::new(6, 1).unwrap();
        assert!(b_sharp.with_letter(0).is_ok(), "B# written as C is C##");
        assert_eq!(
            b_sharp.with_letter(7).unwrap_err(),
            TheoryError::NoteNameUnknown
        );
        assert_eq!(
            b_sharp.with_letter(u8::MAX).unwrap_err(),
            TheoryError::NoteNameUnknown
        );
        // `name_for_letter` 是同一条边界的第三个入口（既有判据只覆盖这一侧，
        // 这里把三个入口对齐，免得只守住其中一个）。
        assert_eq!(
            PitchClass::C.name_for_letter(7).unwrap_err(),
            TheoryError::NoteNameUnknown
        );

        // `PitchClass::new`：0..12，上界 12 必须被拒。
        for value in 0u8..12 {
            assert_eq!(PitchClass::new(value).unwrap().semitones(), value);
        }
        assert_eq!(
            PitchClass::new(12).unwrap_err(),
            TheoryError::PitchOutOfRange { value: 12 }
        );
        assert_eq!(
            PitchClass::new(u8::MAX).unwrap_err(),
            TheoryError::PitchOutOfRange {
                value: u32::from(u8::MAX)
            }
        );
    }

    /// 形态 D 注入实测（本票）：`Interval::from_semitones` 在本 crate 里
    /// **没有任何判据**（`grep` 到的调用点只在生产代码）⇒ 把上界从 `> 12`
    /// 改成 `>= 12` 时全部既有判据仍然全绿，而纯八度（12 个半音）会从
    /// `Some(PERFECT_OCTAVE)` 变成 `None`。
    ///
    /// 口径：域是**闭区间** `0..=12`，13 与 `u8::MAX` 越界。
    #[test]
    fn from_semitones_covers_the_closed_range_up_to_the_octave() {
        for semitones in 0u8..=12 {
            let interval = Interval::from_semitones(semitones)
                .unwrap_or_else(|| panic!("{semitones} semitones must be constructible"));
            assert_eq!(interval.semitones(), semitones);
        }
        assert_eq!(Interval::from_semitones(0), Some(Interval::UNISON));
        assert_eq!(
            Interval::from_semitones(12),
            Some(Interval::PERFECT_OCTAVE),
            "the octave is the closed upper end of the domain"
        );
        assert_eq!(Interval::from_semitones(13), None);
        assert_eq!(Interval::from_semitones(u8::MAX), None);
    }
}
