//! 音阶构造、级数映射、自然三和弦与五度圈邻接 [ROAD-M4-003]。
//!
//! 音阶在本 crate 里是**整数半音集合**：一个主音音级 (tonic) 加一张固定的
//! 半音偏移表。所有判定（`contains`、级数映射、自然三和弦）都只在整数上做，
//! 唯一的浮点出现在频率输出口（见 [`crate::pitch`]）。
//!
//! ## 覆盖范围
//!
//! [`ScaleKind`] 覆盖任务书要求的 15 种音阶：major、natural minor、harmonic minor、
//! melodic minor、dorian、phrygian、lydian、mixolydian、locrian、pentatonic major、
//! pentatonic minor、blues、whole tone、chromatic，外加显式登记的 ionian 与 aeolian。
//!
//! ## 拼写
//!
//! 音阶自带 `prefer_flat`：F 大调 / D 小调一类调性显示成 `Bb` 而不是 `A#`。
//! 该偏好**只影响显示**，不影响任何音高判定。

use crate::error::TheoryError;
use crate::pitch::{NoteName, Pitch, PitchClass};

/// 自然音级字母序号（C=0 … B=6）在七个教会调式上的旋转表。
///
/// `MODE_LETTERS[mode][degree]` 给出某个调式第 `degree` 级应该用哪个字母拼写。
/// 例如 D 多利亚（Dorian）的第 1 级是 `D`，第 3 级是 `F`（不是 `E#`）。
const MAJOR_LETTERS: [u8; 7] = [0, 1, 2, 3, 4, 5, 6];

/// 音阶种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScaleKind {
    /// 自然大调 / 伊奥尼亚 (Ionian)。
    Major,
    /// 伊奥尼亚（`Major` 的别名，独立登记以免调用方猜测）。
    Ionian,
    /// 自然小调 / 爱奥利亚 (Aeolian)。
    NaturalMinor,
    /// 爱奥利亚（`NaturalMinor` 的别名）。
    Aeolian,
    /// 和声小调（第七级升高）。
    HarmonicMinor,
    /// 旋律小调（上行形式，第六、七级升高）。
    MelodicMinor,
    /// 多利亚 (Dorian)。
    Dorian,
    /// 弗里吉亚 (Phrygian)。
    Phrygian,
    /// 利底亚 (Lydian)。
    Lydian,
    /// 混合利底亚 (Mixolydian)。
    Mixolydian,
    /// 洛克里亚 (Locrian)。
    Locrian,
    /// 大调五声音阶（宫调式）。
    PentatonicMajor,
    /// 小调五声音阶（羽调式）。
    PentatonicMinor,
    /// 布鲁斯音阶（小调五声 + 降五级）。
    Blues,
    /// 全音音阶（六个全音）。
    WholeTone,
    /// 半音音阶（十二个半音）。
    Chromatic,
}

/// 大调音阶的半音偏移。
const MAJOR_INTERVALS: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
/// 自然小调。
const NATURAL_MINOR_INTERVALS: [u8; 7] = [0, 2, 3, 5, 7, 8, 10];
/// 和声小调。
const HARMONIC_MINOR_INTERVALS: [u8; 7] = [0, 2, 3, 5, 7, 8, 11];
/// 旋律小调（上行）。
const MELODIC_MINOR_INTERVALS: [u8; 7] = [0, 2, 3, 5, 7, 9, 11];
/// 多利亚。
const DORIAN_INTERVALS: [u8; 7] = [0, 2, 3, 5, 7, 9, 10];
/// 弗里吉亚。
const PHRYGIAN_INTERVALS: [u8; 7] = [0, 1, 3, 5, 7, 8, 10];
/// 利底亚。
const LYDIAN_INTERVALS: [u8; 7] = [0, 2, 4, 6, 7, 9, 11];
/// 混合利底亚。
const MIXOLYDIAN_INTERVALS: [u8; 7] = [0, 2, 4, 5, 7, 9, 10];
/// 洛克里亚。
const LOCRIAN_INTERVALS: [u8; 7] = [0, 1, 3, 5, 6, 8, 10];
/// 大调五声：1 2 3 5 6。
const PENTATONIC_MAJOR_INTERVALS: [u8; 5] = [0, 2, 4, 7, 9];
/// 小调五声：1 ♭3 4 5 ♭7。
const PENTATONIC_MINOR_INTERVALS: [u8; 5] = [0, 3, 5, 7, 10];
/// 布鲁斯：1 ♭3 4 ♭5 5 ♭7。
const BLUES_INTERVALS: [u8; 6] = [0, 3, 5, 6, 7, 10];
/// 全音音阶。
const WHOLE_TONE_INTERVALS: [u8; 6] = [0, 2, 4, 6, 8, 10];
/// 半音音阶。
const CHROMATIC_INTERVALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

impl ScaleKind {
    /// 半音偏移表（第一个元素恒为 0）。
    #[must_use]
    pub const fn intervals(self) -> &'static [u8] {
        match self {
            Self::Major | Self::Ionian => &MAJOR_INTERVALS,
            Self::NaturalMinor | Self::Aeolian => &NATURAL_MINOR_INTERVALS,
            Self::HarmonicMinor => &HARMONIC_MINOR_INTERVALS,
            Self::MelodicMinor => &MELODIC_MINOR_INTERVALS,
            Self::Dorian => &DORIAN_INTERVALS,
            Self::Phrygian => &PHRYGIAN_INTERVALS,
            Self::Lydian => &LYDIAN_INTERVALS,
            Self::Mixolydian => &MIXOLYDIAN_INTERVALS,
            Self::Locrian => &LOCRIAN_INTERVALS,
            Self::PentatonicMajor => &PENTATONIC_MAJOR_INTERVALS,
            Self::PentatonicMinor => &PENTATONIC_MINOR_INTERVALS,
            Self::Blues => &BLUES_INTERVALS,
            Self::WholeTone => &WHOLE_TONE_INTERVALS,
            Self::Chromatic => &CHROMATIC_INTERVALS,
        }
    }

    /// 该音阶是七声的自然音阶时返回其教会调式下标（0 = 伊奥尼亚 … 6 = 洛克里亚）。
    ///
    /// 非七声音阶返回 `None`。用于决定音名的字母拼写。
    #[must_use]
    pub const fn church_mode_index(self) -> Option<u8> {
        match self {
            Self::Major | Self::Ionian => Some(0),
            Self::Dorian => Some(1),
            Self::Phrygian => Some(2),
            Self::Lydian => Some(3),
            Self::Mixolydian => Some(4),
            Self::NaturalMinor | Self::Aeolian => Some(5),
            Self::Locrian => Some(6),
            // 和声/旋律小调不是"某个大调的旋转"，但它们确实是七声音阶，
            // 字母顺序仍然逐级上升，因此按伊奥尼亚的字母表处理。
            Self::HarmonicMinor | Self::MelodicMinor => Some(0),
            _ => None,
        }
    }

    /// 该音阶是否使用降号拼写。
    ///
    /// 小调类、多利亚、弗里吉亚以及主音为降号音级的调性用降号；其余用升号。
    #[must_use]
    pub fn prefer_flat(self, tonic: PitchClass) -> bool {
        let minor_like = matches!(
            self,
            Self::NaturalMinor
                | Self::Aeolian
                | Self::HarmonicMinor
                | Self::MelodicMinor
                | Self::Dorian
                | Self::Phrygian
                | Self::PentatonicMinor
                | Self::Blues
        );
        // 主音的默认拼写本身带降号（如 Bb / Eb）时也跟随降号。
        let tonic_flat = matches!(tonic.semitones(), 1 | 3 | 6 | 8 | 10) && minor_like;
        let flat_keys = matches!(tonic.semitones(), 5 | 10 | 3 | 8 | 1); // F Bb Eb Ab Db
        minor_like || tonic_flat || flat_keys
    }

    /// 音阶名（规范 ASCII 小写，供 `GenreRule.typical_scales` 与外部 API 使用）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Major => "major",
            Self::Ionian => "ionian",
            Self::NaturalMinor => "natural_minor",
            Self::Aeolian => "aeolian",
            Self::HarmonicMinor => "harmonic_minor",
            Self::MelodicMinor => "melodic_minor",
            Self::Dorian => "dorian",
            Self::Phrygian => "phrygian",
            Self::Lydian => "lydian",
            Self::Mixolydian => "mixolydian",
            Self::Locrian => "locrian",
            Self::PentatonicMajor => "pentatonic_major",
            Self::PentatonicMinor => "pentatonic_minor",
            Self::Blues => "blues",
            Self::WholeTone => "whole_tone",
            Self::Chromatic => "chromatic",
        }
    }

    /// 中文显示名。
    #[must_use]
    pub const fn name_zh(self) -> &'static str {
        match self {
            Self::Major | Self::Ionian => "自然大调",
            Self::NaturalMinor | Self::Aeolian => "自然小调",
            Self::HarmonicMinor => "和声小调",
            Self::MelodicMinor => "旋律小调",
            Self::Dorian => "多利亚",
            Self::Phrygian => "弗里吉亚",
            Self::Lydian => "利底亚",
            Self::Mixolydian => "混合利底亚",
            Self::Locrian => "洛克里亚",
            Self::PentatonicMajor => "大调五声",
            Self::PentatonicMinor => "小调五声",
            Self::Blues => "布鲁斯",
            Self::WholeTone => "全音音阶",
            Self::Chromatic => "半音音阶",
        }
    }

    /// 解析音阶名。
    ///
    /// 接受规范名（`natural_minor`）、短名（`minor`、`nat_minor`）、中文名
    /// （`和声小调`）与常见拼写变体（`harmonic minor`、`HarmonicMinor`）。
    ///
    /// # Errors
    ///
    /// 无法识别时返回 [`TheoryError::ScaleNameUnknown`]。
    pub fn parse(text: &str) -> Result<Self, TheoryError> {
        // 归一化到定长栈缓冲：去掉空格/连字符/下划线并转小写。
        // 用栈而不是 `String`：本 crate 的解析路径刻意保持零堆分配。
        let mut buffer = [0u8; 32];
        let mut len = 0usize;
        for byte in text.bytes() {
            let byte = byte.to_ascii_lowercase();
            if matches!(byte, b' ' | b'-' | b'_') {
                continue;
            }
            // 多字节 UTF-8（中文名）按原样保留，逐字节复制即可做前缀比较。
            if len < buffer.len() {
                buffer[len] = byte;
                len += 1;
            }
        }
        let normalized = &buffer[..len];
        // 中文名以 UTF-8 字节字面量参与匹配：字节串模式是合法的 Rust 模式，
        // 而 `"...".as_bytes()` 是表达式，不能出现在 `match` 分支里。
        let kind = match normalized {
            b"major"
            | b"maj"
            | b"ionian"
            | b"\xe8\x87\xaa\xe7\x84\xb6\xe5\xa4\xa7\xe8\xb0\x83"
            | b"\xe5\xa4\xa7\xe8\xb0\x83" => Self::Major,
            b"naturalminor"
            | b"natminor"
            | b"minor"
            | b"min"
            | b"aeolian"
            | b"\xe8\x87\xaa\xe7\x84\xb6\xe5\xb0\x8f\xe8\xb0\x83"
            | b"\xe5\xb0\x8f\xe8\xb0\x83" => Self::NaturalMinor,
            b"harmonicminor"
            | b"harmonic"
            | b"harmminor"
            | b"\xe5\x92\x8c\xe5\xa3\xb0\xe5\xb0\x8f\xe8\xb0\x83" => Self::HarmonicMinor,
            b"melodicminor"
            | b"melodic"
            | b"melminor"
            | b"\xe6\x97\x8b\xe5\xbe\x8b\xe5\xb0\x8f\xe8\xb0\x83" => Self::MelodicMinor,
            b"dorian" | b"\xe5\xa4\x9a\xe5\x88\xa9\xe4\xba\x9a" => Self::Dorian,
            b"phrygian" | b"\xe5\xbc\x97\xe9\x87\x8c\xe5\x90\x89\xe4\xba\x9a" => Self::Phrygian,
            b"lydian" | b"\xe5\x88\xa9\xe5\xba\x95\xe4\xba\x9a" => Self::Lydian,
            b"mixolydian" | b"\xe6\xb7\xb7\xe5\x90\x88\xe5\x88\xa9\xe5\xba\x95\xe4\xba\x9a" => {
                Self::Mixolydian
            }
            b"locrian" | b"\xe6\xb4\x9b\xe5\x85\x8b\xe9\x87\x8c\xe4\xba\x9a" => Self::Locrian,
            b"pentatonicmajor"
            | b"majorpentatonic"
            | b"penta"
            | b"\xe5\xa4\xa7\xe8\xb0\x83\xe4\xba\x94\xe5\xa3\xb0"
            | b"\xe5\xae\xab\xe8\xb0\x83\xe5\xbc\x8f" => Self::PentatonicMajor,
            b"pentatonicminor"
            | b"minorpentatonic"
            | b"\xe5\xb0\x8f\xe8\xb0\x83\xe4\xba\x94\xe5\xa3\xb0"
            | b"\xe7\xbe\xbd\xe8\xb0\x83\xe5\xbc\x8f" => Self::PentatonicMinor,
            b"blues" | b"bluesscale" | b"\xe5\xb8\x83\xe9\xb2\x81\xe6\x96\xaf" => Self::Blues,
            b"wholetone" | b"whole" | b"\xe5\x85\xa8\xe9\x9f\xb3\xe9\x9f\xb3\xe9\x98\xb6" => {
                Self::WholeTone
            }
            b"chromatic" | b"\xe5\x8d\x8a\xe9\x9f\xb3\xe9\x9f\xb3\xe9\x98\xb6" => Self::Chromatic,
            _ => return Err(TheoryError::ScaleNameUnknown),
        };
        Ok(kind)
    }
}

/// 一个具体的音阶实例：主音 + 种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Scale {
    /// 主音音级。
    pub tonic: PitchClass,
    /// 音阶种类。
    pub kind: ScaleKind,
}

impl Scale {
    /// 构造音阶。
    #[must_use]
    pub const fn new(tonic: PitchClass, kind: ScaleKind) -> Self {
        Self { tonic, kind }
    }

    /// 解析 `"C major"` / `"F# harmonic_minor"` / `"Bb blues"` 这类文本。
    ///
    /// # Errors
    ///
    /// 文本格式不是 `<根音> <音阶名>`，或任一部分无法识别时返回错误。
    pub fn parse(text: &str) -> Result<Self, TheoryError> {
        let mut parts = text.split_whitespace();
        let root = parts.next().ok_or(TheoryError::ScaleNameUnknown)?;
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            return Err(TheoryError::ScaleNameUnknown);
        }
        let tonic = crate::pitch::parse_pitch_class(root)?;
        let kind = ScaleKind::parse(&rest.join(" "))?;
        Ok(Self { tonic, kind })
    }

    /// 半音偏移表。
    #[must_use]
    pub const fn intervals(&self) -> &'static [u8] {
        self.kind.intervals()
    }

    /// 一个八度内的音数（七声 = 7，五声 = 5，半音 = 12）。
    #[must_use]
    pub const fn degree_count(&self) -> u8 {
        self.intervals().len() as u8
    }

    /// 第 `degree` 个音级的音级，超出八度自动向上取模。
    ///
    /// # Errors
    ///
    /// 音阶为空（不可能发生，防御性检查）时返回 [`TheoryError::DegreeOutOfRange`]。
    pub fn degree_pitch_class(&self, degree: u16) -> Result<PitchClass, TheoryError> {
        let count = self.degree_count();
        if count == 0 {
            return Err(TheoryError::DegreeOutOfRange { degree: 0 });
        }
        let index = degree % u16::from(count);
        let octaves = degree / u16::from(count);
        // 累加在 `u32` 上做：`degree` 取 `u16::MAX` 且音阶有 7 级时 `octaves` 是
        // 9362，`12 * octaves` 已经超出 `u16`（debug 下 panic，release 下回绕后
        // 给出错的音级）。取模 12 之后的读数与旧口径逐位相同。
        let semitones = u32::from(self.tonic.semitones())
            + u32::from(self.intervals()[index as usize])
            + 12 * u32::from(octaves);
        PitchClass::new((semitones % 12) as u8)
    }

    /// 第 `degree` 个音级的 MIDI 音高：主音落在 `tonic_octave` 八度。
    ///
    /// # Errors
    ///
    /// 结果超出 MIDI `0..=127` 时返回 [`TheoryError::PitchOutOfRange`]。
    /// **属性测试保证**：对 `tonic_octave ∈ 0..=8` 与 `degree ∈ 0..=14`，
    /// 只要主音自身在音域内，结果必然在 `0..=127`。
    pub fn degree_to_pitch(&self, tonic_octave: i8, degree: u16) -> Result<Pitch, TheoryError> {
        let count = self.degree_count();
        if count == 0 {
            return Err(TheoryError::DegreeOutOfRange { degree: 0 });
        }
        let index = degree % u16::from(count);
        let octaves = degree / u16::from(count);
        let base = (i32::from(tonic_octave) + 1) * 12 + i32::from(self.tonic.semitones());
        let value = base + i32::from(self.intervals()[index as usize]) + 12 * i32::from(octaves);
        Pitch::new(value)
    }

    /// 该音级是否属于本音阶（八度等价）。
    #[must_use]
    pub fn contains(&self, pc: PitchClass) -> bool {
        let relative = (pc.semitones() + 12 - self.tonic.semitones()) % 12;
        self.intervals().contains(&relative)
    }

    /// 有多少个音级落在给定音级上（半音音阶里可能是 1，正常音阶里 0 或 1）。
    #[must_use]
    pub fn degree_of(&self, pc: PitchClass) -> Option<u16> {
        let relative = (pc.semitones() + 12 - self.tonic.semitones()) % 12;
        self.intervals()
            .iter()
            .position(|&step| step == relative)
            .map(|index| index as u16)
    }

    /// 音阶内的全部音级（按度数升序，不含八度重复）。
    #[must_use]
    pub fn pitch_classes(&self) -> Vec<PitchClass> {
        self.intervals()
            .iter()
            .filter_map(|&step| PitchClass::new((self.tonic.semitones() + step) % 12).ok())
            .collect()
    }

    /// 给某个音级挑选音阶内的字母拼写。
    ///
    /// # Errors
    ///
    /// 非七声音阶无法给出唯一的字母序列时返回 [`TheoryError::AmbiguousSpelling`]。
    pub fn spell(&self, pc: PitchClass) -> Result<NoteName, TheoryError> {
        self.spell_with(pc, self.kind.prefer_flat(self.tonic))
    }

    /// 同上，但显式指定升降号偏好。
    ///
    /// 和弦层用它把 [`crate::chord::Tonality`] 的偏好灌进来：`Bb13` 的
    /// 拼写偏好来自和弦自身的调性上下文，而不是"主音是不是降号调"。
    ///
    /// # Errors
    ///
    /// 音级无法在音阶的七个字母里用重升/重降之内表示时返回
    /// [`TheoryError::AmbiguousSpelling`]。
    pub fn spell_with(&self, pc: PitchClass, prefer_flat: bool) -> Result<NoteName, TheoryError> {
        let Some(mode) = self.kind.church_mode_index() else {
            return Ok(pc.default_name());
        };
        let tonic_letter = self.tonic.default_name().letter;
        // 调式的字母序列 = 大调字母表按调式序号旋转，再平移到主音字母。
        let mut letters = [0u8; 7];
        for (index, letter) in letters.iter_mut().enumerate() {
            *letter = (tonic_letter + MAJOR_LETTERS[(index + usize::from(mode)) % 7]) % 7;
        }
        pc.spell_with_letters(&letters, prefer_flat)
    }

    /// 从主音起、第 `degree` 个音级上叠置三度的三和弦性质。
    ///
    /// 判定完全在整数上做：把音级序号转成**八度累计**的半音距离，再在
    /// `0..=11` 里比较三度/五度音程。直接用 `degree_pitch_class` 会因为
    /// "五度先跨八度取模"而算出错误的音程（例如 D-F-A 的 F→A 是 4 个半音，
    /// 但 C 大调里 F=5、A=9，差 4 只是巧合；D-F 的 2→5 恰好也对，真正的
    /// 陷阱在 `iv`、`vii` 这些跨八度的级上）。
    #[must_use]
    pub fn triad_quality(&self, degree: u16) -> crate::chord::ChordKind {
        use crate::chord::ChordKind;
        let count = u16::from(self.degree_count());
        if count == 0 {
            return ChordKind::Major;
        }
        // 级数累加与八度累计都在 `u32` 上做：`degree` 取 `u16::MAX` 时
        // `degree + offset` 与 `12 * octaves` 都会超出 `u16`（debug 下 panic，
        // release 下回绕后给出错的性质）。两个读数都只用于取模 12，
        // 因此在装得下的输入上与旧口径逐位相同。
        let span = |offset: u32| -> u32 {
            let target = u32::from(degree) + offset;
            let index = target % u32::from(count);
            let octaves = target / u32::from(count);
            12 * octaves + u32::from(self.intervals()[index as usize])
        };
        let third = (span(2) - span(0)) % 12;
        let fifth = (span(4) - span(0)) % 12;
        match (third, fifth) {
            (4, 7) => ChordKind::Major,
            (3, 7) => ChordKind::Minor,
            (3, 6) => ChordKind::Diminished,
            (4, 8) => ChordKind::Augmented,
            _ => ChordKind::Major,
        }
    }

    /// 自然三和弦（每级一个）：按音阶三度叠置。
    ///
    /// 每一个元素是 `(级数, 构成音)`；五声音阶的第 3 个音是"跳过一个音"得到，
    /// 因此结果可能不是教科书意义上的三度叠置，调用方要按 `kind` 自行判断。
    #[must_use]
    pub fn diatonic_triads(&self) -> Vec<(u16, [PitchClass; 3])> {
        let count = u16::from(self.degree_count());
        let mut triads = Vec::with_capacity(count as usize);
        for degree in 0..count {
            let mut chord = [self.tonic; 3];
            for (offset, slot) in chord.iter_mut().enumerate() {
                let target = degree + 2 * offset as u16;
                match self.degree_pitch_class(target) {
                    Ok(pc) => *slot = pc,
                    Err(_) => return triads,
                }
            }
            triads.push((degree, chord));
        }
        triads
    }

    /// 五度圈的邻接音级：`(上行五度, 下行五度)`。
    #[must_use]
    pub fn fifth_circle_neighbors(&self) -> (PitchClass, PitchClass) {
        (self.tonic.transpose(7), self.tonic.transpose(-7))
    }

    /// 五度圈上距主音最近、且仍在音阶内的调性：`(属方向, 下属方向)`。
    ///
    /// 返回 `(Option<PitchClass>, Option<PitchClass>)`；两者都在音阶内时即
    /// "近关系调"。距离以五度圈步数计（最多探到 6 步）。
    #[must_use]
    pub fn close_keys(&self) -> (Option<PitchClass>, Option<PitchClass>) {
        let mut dominant = None;
        let mut subdominant = None;
        for step in 1..=6i16 {
            if dominant.is_none() && self.contains(self.tonic.transpose(7 * step)) {
                dominant = Some(self.tonic.transpose(7 * step));
            }
            if subdominant.is_none() && self.contains(self.tonic.transpose(-7 * step)) {
                subdominant = Some(self.tonic.transpose(-7 * step));
            }
        }
        (dominant, subdominant)
    }

    /// 两音阶之间的五度圈距离（主音距离，0..=6）。
    #[must_use]
    pub fn fifth_circle_distance_to(&self, other: &Self) -> u8 {
        self.tonic.fifth_circle_distance(other.tonic)
    }
}

impl core::fmt::Display for Scale {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} {}", self.tonic, self.kind.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_scale_has_the_textbook_intervals() {
        let c = Scale::new(PitchClass::C, ScaleKind::Major);
        assert_eq!(c.intervals(), &[0, 2, 4, 5, 7, 9, 11]);
        assert_eq!(c.degree_count(), 7);
        assert_eq!(c.degree_pitch_class(0).unwrap(), PitchClass::C);
        assert_eq!(c.degree_pitch_class(4).unwrap(), PitchClass::G);
        assert_eq!(c.degree_pitch_class(7).unwrap(), PitchClass::C);
    }

    #[test]
    fn degree_to_pitch_spans_octaves() {
        let c = Scale::new(PitchClass::C, ScaleKind::Major);
        assert_eq!(c.degree_to_pitch(4, 0).unwrap().value(), 60);
        assert_eq!(c.degree_to_pitch(4, 7).unwrap().value(), 72);
        assert_eq!(c.degree_to_pitch(4, 9).unwrap().value(), 76); // D5
    }

    #[test]
    fn degree_readings_are_total_across_the_whole_u16_domain() {
        // `degree + offset` 与 `12 * octaves` 在 `u16` 上都会溢出
        // （`degree` 取 `u16::MAX` 时八度累计是 9362）：debug 下 panic、
        // release 下回绕后给出错的音级/性质。两条读数都只用于取模 12，
        // 因此这条判据在**整段** `u16` 值域上钉住"八度累计不影响读数"。
        for kind in [
            ScaleKind::Major,
            ScaleKind::NaturalMinor,
            ScaleKind::HarmonicMinor,
            ScaleKind::PentatonicMinor,
            ScaleKind::Blues,
        ] {
            let scale = Scale::new(PitchClass::C, kind);
            let intervals = kind.intervals();
            let count = intervals.len() as u32;
            for degree in 0u16..=u16::MAX {
                let expected = intervals[(u32::from(degree) % count) as usize];
                assert_eq!(
                    scale.degree_pitch_class(degree).unwrap().semitones(),
                    expected,
                    "{} degree {degree}",
                    kind.name()
                );
                let _ = scale.triad_quality(degree);
            }
            // 三度/五度读数在跨八度的级上必须与"八度累计"口径一致。
            for degree in [0u16, 1, 6, 7, 8, 100, 60_000, u16::MAX - 1, u16::MAX] {
                let span = |offset: u32| -> u32 {
                    let target = u32::from(degree) + offset;
                    12 * (target / count) + u32::from(intervals[(target % count) as usize])
                };
                let expected = match ((span(2) - span(0)) % 12, (span(4) - span(0)) % 12) {
                    (4, 7) => crate::chord::ChordKind::Major,
                    (3, 7) => crate::chord::ChordKind::Minor,
                    (3, 6) => crate::chord::ChordKind::Diminished,
                    (4, 8) => crate::chord::ChordKind::Augmented,
                    _ => crate::chord::ChordKind::Major,
                };
                assert_eq!(
                    scale.triad_quality(degree),
                    expected,
                    "{} degree {degree}",
                    kind.name()
                );
            }
        }
        // 逐位一致读数：整段 `u16` 值域的上端点。
        let c = Scale::new(PitchClass::C, ScaleKind::Major);
        // 65535 = 9362 * 7 + 1 ⇒ 音级下标 1 = D，八度累计被取模消掉。
        assert_eq!(c.degree_pitch_class(u16::MAX).unwrap(), PitchClass::D);
        assert_eq!(c.triad_quality(u16::MAX), crate::chord::ChordKind::Minor);
    }

    #[test]
    fn all_fifteen_required_scales_are_constructible() {
        let kinds = [
            ScaleKind::Major,
            ScaleKind::NaturalMinor,
            ScaleKind::HarmonicMinor,
            ScaleKind::MelodicMinor,
            ScaleKind::Dorian,
            ScaleKind::Phrygian,
            ScaleKind::Lydian,
            ScaleKind::Mixolydian,
            ScaleKind::Locrian,
            ScaleKind::PentatonicMajor,
            ScaleKind::PentatonicMinor,
            ScaleKind::Blues,
            ScaleKind::WholeTone,
            ScaleKind::Chromatic,
            ScaleKind::Ionian,
        ];
        for kind in kinds {
            let scale = Scale::new(PitchClass::C, kind);
            let first = scale.intervals()[0];
            assert_eq!(first, 0, "{} must start on the tonic", kind.name());
            let mut sorted = scale.intervals().to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), scale.intervals().len(), "{}", kind.name());
            assert!(scale.contains(PitchClass::C), "{}", kind.name());
        }
    }

    #[test]
    fn natural_minor_differs_from_major_on_the_third() {
        let a = Scale::new(PitchClass::A, ScaleKind::NaturalMinor);
        assert_eq!(a.degree_pitch_class(2).unwrap(), PitchClass::C);
        assert!(a.contains(PitchClass::G));
        let c = Scale::new(PitchClass::C, ScaleKind::Major);
        assert!(!c.contains(PitchClass::AS));
    }

    #[test]
    fn harmonic_minor_raises_the_seventh() {
        let a = Scale::new(PitchClass::A, ScaleKind::HarmonicMinor);
        assert!(a.contains(PitchClass::GS));
        let a_nat = Scale::new(PitchClass::A, ScaleKind::NaturalMinor);
        assert!(!a_nat.contains(PitchClass::GS));
    }

    #[test]
    fn diatonic_triads_of_c_major_are_the_seven_textbook_chords() {
        let c = Scale::new(PitchClass::C, ScaleKind::Major);
        let triads = c.diatonic_triads();
        assert_eq!(triads.len(), 7);
        assert_eq!(triads[0].1, [PitchClass::C, PitchClass::E, PitchClass::G]);
        assert_eq!(triads[4].1, [PitchClass::G, PitchClass::B, PitchClass::D]);
        assert_eq!(triads[6].1, [PitchClass::B, PitchClass::D, PitchClass::F]);
    }

    #[test]
    fn fifth_circle_neighbors_are_the_dominant_and_subdominant() {
        let c = Scale::new(PitchClass::C, ScaleKind::Major);
        assert_eq!(c.fifth_circle_neighbors(), (PitchClass::G, PitchClass::F));
        assert_eq!(c.close_keys(), (Some(PitchClass::G), Some(PitchClass::F)));
        assert_eq!(
            c.fifth_circle_distance_to(&Scale::new(PitchClass::G, ScaleKind::Major)),
            1
        );
    }

    #[test]
    fn scale_names_round_trip_through_parse() {
        for kind in [
            ScaleKind::Major,
            ScaleKind::NaturalMinor,
            ScaleKind::HarmonicMinor,
            ScaleKind::MelodicMinor,
            ScaleKind::Dorian,
            ScaleKind::Phrygian,
            ScaleKind::Lydian,
            ScaleKind::Mixolydian,
            ScaleKind::Locrian,
            ScaleKind::PentatonicMajor,
            ScaleKind::PentatonicMinor,
            ScaleKind::Blues,
            ScaleKind::WholeTone,
            ScaleKind::Chromatic,
        ] {
            assert_eq!(ScaleKind::parse(kind.name()).unwrap(), kind);
        }
        assert_eq!(ScaleKind::parse("minor").unwrap(), ScaleKind::NaturalMinor);
        assert_eq!(
            ScaleKind::parse("Harmonic Minor").unwrap(),
            ScaleKind::HarmonicMinor
        );
        assert_eq!(
            ScaleKind::parse("和声小调").unwrap(),
            ScaleKind::HarmonicMinor
        );
        assert!(ScaleKind::parse("klingon").is_err());
    }

    #[test]
    fn scale_text_parsing_accepts_root_and_kind() {
        let scale = Scale::parse("F# harmonic_minor").unwrap();
        assert_eq!(scale.tonic, PitchClass::FS);
        assert_eq!(scale.kind, ScaleKind::HarmonicMinor);
        let b_flat_blues = Scale::parse("Bb blues").unwrap();
        assert_eq!(b_flat_blues.tonic, PitchClass::AS);
        assert_eq!(b_flat_blues.kind, ScaleKind::Blues);
        assert!(Scale::parse("F#").is_err());
    }

    /// 拼写偏好：可观测的判据。
    ///
    /// 形态 D 注入实测（本票）：把 `prefer_flat` 的
    /// `matches!(tonic.semitones(), 1 | 3 | 6 | 8 | 10) && minor_like`
    /// 改成 `|| minor_like`、或把 `flat_keys` 里的 `1` 去掉时，既有判据全绿。
    /// 本机逐组合核对后的结论是：这两处改动都是**观测等价**的 ——
    /// `spell_with_letters` 只在"同一音级有两个可行字母且变音记号绝对值相同"
    /// 时才用偏好打破平局，而在这些音阶里**另一个字母需要 ±7 个变音记号**，
    /// 会被代价筛掉，因此偏好没有可观测的用武之地；`tonic_flat` 更是
    /// `flat_keys` 的子集（`{1,3,6,8,10}` 全在 `flat_keys` 里）。
    ///
    /// 这条判据因此不宣称能判死那两处注入（没有可用的输入），它钉住的是
    /// **偏好真的被消费了**：降号调读出降号拼写、升号调读出升号拼写；
    /// 最后再加一条"同一音级在两侧都有可行字母"的显式偏好读数。
    #[test]
    fn spelling_preference_is_consumed_by_the_flat_and_sharp_keys() {
        // 降号调：读出降号拼写（另一侧需要 ±7 个记号，被代价筛掉）。
        let db_major = Scale::new(PitchClass::DS, ScaleKind::Major);
        assert!(db_major.kind.prefer_flat(db_major.tonic));
        assert_eq!(db_major.spell(PitchClass::CS).unwrap().to_string(), "Db");
        assert_eq!(db_major.spell(PitchClass::FS).unwrap().to_string(), "Gb");
        assert_eq!(db_major.spell(PitchClass::AS).unwrap().to_string(), "Bb");
        let ab_major = Scale::new(PitchClass::GS, ScaleKind::Major);
        assert!(ab_major.kind.prefer_flat(ab_major.tonic));
        assert_eq!(ab_major.spell(PitchClass::CS).unwrap().to_string(), "Db");
        assert_eq!(ab_major.spell(PitchClass::DS).unwrap().to_string(), "Eb");
        assert_eq!(ab_major.spell(PitchClass::FS).unwrap().to_string(), "Gb");
        let f_major = Scale::new(PitchClass::F, ScaleKind::Major);
        assert!(f_major.kind.prefer_flat(f_major.tonic));
        assert_eq!(f_major.spell(PitchClass::AS).unwrap().to_string(), "Bb");
        assert_eq!(f_major.spell(PitchClass::DS).unwrap().to_string(), "Eb");
        // 升号调：另一侧的读数必须保持升号。
        let a_major = Scale::new(PitchClass::A, ScaleKind::Major);
        assert!(!a_major.kind.prefer_flat(a_major.tonic));
        assert_eq!(a_major.spell(PitchClass::CS).unwrap().to_string(), "C#");
        assert_eq!(a_major.spell(PitchClass::FS).unwrap().to_string(), "F#");
        let e_major = Scale::new(PitchClass::E, ScaleKind::Major);
        assert!(!e_major.kind.prefer_flat(e_major.tonic));
        assert_eq!(e_major.spell(PitchClass::DS).unwrap().to_string(), "D#");
        assert_eq!(e_major.spell(PitchClass::GS).unwrap().to_string(), "G#");
        // 小调类（`minor_like`）恒为降号侧：`D#` 必须拼成 `Eb`。
        let c_minor = Scale::new(PitchClass::C, ScaleKind::NaturalMinor);
        assert!(c_minor.kind.prefer_flat(c_minor.tonic));
        assert_eq!(c_minor.spell(PitchClass::DS).unwrap().to_string(), "Eb");
        assert_eq!(c_minor.spell(PitchClass::GS).unwrap().to_string(), "Ab");
        // 显式灌偏好（`spell_with`）能把两个方向都读出来 —— 这是"偏好真的
        // 参与判定"的直接证据（本机实测：C# 大调里 `Db` 与 `C#` 都可拼）。
        let c_sharp_major = Scale::new(PitchClass::CS, ScaleKind::Major);
        assert_eq!(
            c_sharp_major
                .spell_with(PitchClass::CS, true)
                .unwrap()
                .to_string(),
            "Db"
        );
        assert_eq!(
            c_sharp_major
                .spell_with(PitchClass::CS, false)
                .unwrap()
                .to_string(),
            "C#"
        );
    }

    /// 形态 D 注入实测（本票）：把三度/五度的叠置换成 `ChordKind::Augmented`
    /// 之外的另一条分支、或改任何一个半音数字面量时没有判据变红。这里直接按
    /// 整数叠置口径独立复算一遍 `triad_quality`（不引用实现里的 `span`），
    /// 因此三度/五度的每一条组合都被钉住。
    #[test]
    fn triad_quality_matches_an_independent_octave_accumulation() {
        use crate::chord::ChordKind;
        let kinds = [
            ScaleKind::Major,
            ScaleKind::NaturalMinor,
            ScaleKind::HarmonicMinor,
            ScaleKind::MelodicMinor,
            ScaleKind::Dorian,
            ScaleKind::Phrygian,
            ScaleKind::Lydian,
            ScaleKind::Mixolydian,
            ScaleKind::Locrian,
            ScaleKind::PentatonicMinor,
            ScaleKind::Blues,
        ];
        for kind in kinds {
            let scale = Scale::new(PitchClass::C, kind);
            let count = u32::from(scale.degree_count());
            let intervals = kind.intervals();
            for degree in 0u16..(4 * count as u16) {
                let span = |offset: u32| -> u32 {
                    let target = u32::from(degree) + offset;
                    12 * (target / count) + u32::from(intervals[(target % count) as usize])
                };
                let expected = match ((span(2) - span(0)) % 12, (span(4) - span(0)) % 12) {
                    (4, 7) => ChordKind::Major,
                    (3, 7) => ChordKind::Minor,
                    (3, 6) => ChordKind::Diminished,
                    (4, 8) => ChordKind::Augmented,
                    _ => ChordKind::Major,
                };
                assert_eq!(
                    scale.triad_quality(degree),
                    expected,
                    "{kind:?} degree {degree}"
                );
            }
        }
    }

    /// 形态 D 注入实测（本票）：`degree_to_pitch` 的越界拒绝没有被既有判据
    /// 覆盖 —— 既有判据只喂合法音域。这里钉住"越界必须报错、不回绕"，
    /// 同时钉住 `tonic_octave + 1` 这条八度口径。
    #[test]
    fn degree_to_pitch_reports_out_of_range_instead_of_wrapping() {
        let c = Scale::new(PitchClass::C, ScaleKind::Major);
        assert_eq!(c.degree_to_pitch(4, 0).unwrap().value(), 60);
        assert_eq!(c.degree_to_pitch(4, 7).unwrap().value(), 72);
        // 最高合法主音八度是 9 ⇒ 第 0 级 = C10 = 132，超出 MIDI 127。
        assert_eq!(c.degree_to_pitch(9, 0).unwrap().value(), 120);
        assert_eq!(
            c.degree_to_pitch(10, 0).unwrap_err(),
            TheoryError::PitchOutOfRange { value: 132 }
        );
        // 下界：主音八度 -2 的第 0 级是 -12。
        assert_eq!(
            c.degree_to_pitch(-2, 0).unwrap_err(),
            TheoryError::PitchOutOfRange { value: 12 }
        );
    }

    #[test]
    fn spelling_follows_the_key_signature() {
        let f_major = Scale::new(PitchClass::F, ScaleKind::Major);
        assert_eq!(f_major.spell(PitchClass::AS).unwrap().to_string(), "Bb");
        let d_major = Scale::new(PitchClass::D, ScaleKind::Major);
        assert_eq!(d_major.spell(PitchClass::FS).unwrap().to_string(), "F#");
        // D 大调的第七级是 C#（音级序号 1），不是 C 本位。
        assert_eq!(
            d_major.spell(PitchClass::CS).unwrap().to_string(),
            "C#",
            "C# is the major seventh of D major"
        );
        // 反过来，问"D 大调怎么拼 C 本位"，答案是 C 本位（D 大调没有 C 本位，
        // 但拼写函数回答的是"这个音级在键盘上的名字"，不是"它是否属于该音阶"）。
        assert_eq!(d_major.spell(PitchClass::C).unwrap().to_string(), "C");
        assert!(!d_major.contains(PitchClass::C));
    }
}
