//! 罗马数字级数走向与 960 PPQ 整数展开 [ROAD-M4-003]。
//!
//! ## 记号
//!
//! [`Degree`] 表示一个级数和弦：
//!
//! ```text
//! [变音记号] <罗马数字> [七度/挂留/加音后缀]
//! ```
//!
//! - 大小写决定三和弦性质：`I` 大三、`ii` 小三、`vii°` 减三、`III+` 增三。
//! - 后缀 `7` / `maj7` / `ø` / `°` / `add9` / `sus4` / `6` / `9` 与
//!   [`crate::chord::ChordKind`] 的视频写法同源，走同一张后缀表。
//! - 前缀变音记号 `b` / `#`（也接受 `♭` / `♯`），例如 `bVII`、`#iv°`。
//! - 无后缀时按**调内自然三和弦**定性质：C 大调的 `vii` 自动成为 `B°`，
//!   A 和声小调的 `V` 自动成为 `E`（大三，而不是自然小调的 `Em`）。
//!
//! ## tick 纪律
//!
//! 展开结果全部是 **960 PPQ 整数 tick**，没有任何浮点拍号：
//! `ticks_per_bar = PPQ * 4 / denominator * numerator`（四分音符 = 960 tick）。

use crate::chord::{Chord, ChordKind, Tonality};
use crate::error::TheoryError;
use crate::scale::{Scale, ScaleKind};

/// 展开结果的最小时间分辨率：一个 16 分音符 = 240 tick。
///
/// 任何展开出来的和弦时值都是它的整数倍，因此所有 `start_tick` 都落在
/// 16 分音符网格上（960 PPQ ÷ 4）。这比"整数 tick 但落在 1/3 拍上"更可用：
/// 卷帘、量化与 MIDI 导出都以 16 分音符为基本格。
pub const MIN_DURATION_TICKS: u64 = PPQ / 4;

/// 每四分音符的 tick 数。
///
/// 与 `yeban-model` 的 `MODEL-AST-001` 同值。本 crate 不能依赖 `yeban-model`
/// （会形成多余的 crate 依赖方向），因此这里独立声明同一个常量，并在测试里
/// 断言两者相等由集成测试负责（见 `docs/ledger/theory-core-notes.md`）。
pub const PPQ: u64 = 960;

/// 节拍（拍号）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Meter {
    /// 每小节拍数。
    pub numerator: u8,
    /// 以几分音符为一拍（4 = 四分音符）。
    pub denominator: u8,
}

impl Meter {
    /// 构造拍号。
    ///
    /// # Errors
    ///
    /// 分子为 0、分母不是 2 的幂时返回 [`TheoryError::ZeroBars`]。
    pub const fn new(numerator: u8, denominator: u8) -> Result<Self, TheoryError> {
        if numerator == 0 || denominator == 0 || !denominator.is_power_of_two() {
            return Err(TheoryError::ZeroBars);
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    /// 每小节 tick 数（整数）。
    ///
    /// 分母为 0 的拍号只能由公有字段绕过 [`Meter::new`] 得到，此时返回 `0`
    /// （"没有时长"）而不是 panic —— 除数是入参，而本 crate 的计算路径承诺
    /// 绝不 panic。其余输入（含非 2 的幂的分母）的读数与旧口径**逐位相同**。
    #[must_use]
    pub const fn ticks_per_bar(self) -> u64 {
        if self.denominator == 0 {
            return 0;
        }
        PPQ * 4 * self.numerator as u64 / self.denominator as u64
    }

    /// 常用 4/4。
    pub const COMMON: Self = Self {
        numerator: 4,
        denominator: 4,
    };
    /// 3/4。
    pub const WALTZ: Self = Self {
        numerator: 3,
        denominator: 4,
    };
    /// 6/8。
    pub const COMPOUND_DUPLE: Self = Self {
        numerator: 6,
        denominator: 8,
    };
    /// 2/4。
    pub const MARCH: Self = Self {
        numerator: 2,
        denominator: 4,
    };
    /// 5/4。
    pub const QUINTUPLE: Self = Self {
        numerator: 5,
        denominator: 4,
    };
    /// 7/8。
    pub const SEVEN_EIGHT: Self = Self {
        numerator: 7,
        denominator: 8,
    };
}

/// 罗马数字的三和弦性质。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RomanQuality {
    /// 大三和弦（大写数字）。
    Major,
    /// 小三和弦（小写数字）。
    Minor,
    /// 减三和弦（`°` / `o`）。
    Diminished,
    /// 增三和弦（`+`）。
    Augmented,
    /// 半减七（`ø`）。
    HalfDiminished,
}

impl RomanQuality {
    /// 对应的三和弦种类。
    #[must_use]
    pub const fn triad_kind(self) -> ChordKind {
        match self {
            Self::Major => ChordKind::Major,
            Self::Minor => ChordKind::Minor,
            Self::Diminished => ChordKind::Diminished,
            Self::Augmented => ChordKind::Augmented,
            Self::HalfDiminished => ChordKind::Diminished,
        }
    }

    /// 对应的七和弦种类（`7` 后缀时使用）。
    #[must_use]
    pub const fn seventh_kind(self) -> ChordKind {
        match self {
            Self::Major => ChordKind::Major7,
            Self::Minor => ChordKind::Minor7,
            Self::Diminished => ChordKind::Diminished7,
            Self::Augmented => ChordKind::Dominant7,
            Self::HalfDiminished => ChordKind::HalfDiminished7,
        }
    }
}

/// 一个级数和弦，例如 `V`、`ii7`、`bVII`、`#iv°`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Degree {
    /// 音阶级数 `1..=7`。
    pub degree: u8,
    /// 变音记号半音数（`-1` = `b`，`+1` = `#`）。
    pub accidental: i8,
    /// 罗马数字显式给出的性质。
    pub quality: RomanQuality,
    /// 显式后缀（`None` 表示按调内自然和弦推导）。
    pub suffix: Option<ChordKind>,
}

/// 罗马数字到级数序号的映射表，按级数升序（下标 0 对应 `I`）。
const ROMAN_TABLE: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];

/// 解析时用于最长匹配的候选序列（按字符串长度降序）。
///
/// **必须先匹配 `VII` 再匹配 `V`**，否则 `VII` 会被贪婪地切成 `V` + 未知后缀。
const ROMAN_MATCH_ORDER: [&str; 7] = ["VII", "III", "VI", "IV", "II", "V", "I"];

impl Degree {
    /// 构造一个级数（无变音记号、无显式后缀，性质由大小写符号给出）。
    ///
    /// # Errors
    ///
    /// `degree` 不在 `1..=7` 时返回 [`TheoryError::DegreeNumberOutOfRange`]。
    pub const fn new(degree: u8, quality: RomanQuality) -> Result<Self, TheoryError> {
        if degree < 1 || degree > 7 {
            return Err(TheoryError::DegreeNumberOutOfRange);
        }
        Ok(Self {
            degree,
            accidental: 0,
            quality,
            suffix: None,
        })
    }

    /// 从罗马数字文本解析。
    ///
    /// # Errors
    ///
    /// 语法不合法时返回 [`TheoryError::DegreeSymbolUnknown`]，数字越界时返回
    /// [`TheoryError::DegreeNumberOutOfRange`]，前缀变音记号的绝对值超过
    /// `i8` 能表达的范围时也返回 [`TheoryError::DegreeSymbolUnknown`]
    /// （**拒绝**，不回绕）。
    pub fn parse(text: &str) -> Result<Self, TheoryError> {
        let trimmed = text.trim();
        let mut body = trimmed;
        // 累加在 `i32` 上做：前缀变音记号的个数就是输入长度，`i8` 会在第 129 个
        // 降号（或第 128 个升号）上溢出（debug 与开启溢出检查的 release 档都
        // panic；Cargo 默认关闭检查的 release 档回绕成 `Ok(127)` 这种错值）。
        // `accidental` 是 `i8`，因此只在**装不下**时拒绝；装得下的输入与旧口径
        // 逐位相同（含 `b` ×128 ⇒ `-128` 这条旧码不 panic 的边界）。
        let mut accidental: i32 = 0;
        // 前缀变音记号
        while let Some(ch) = body.chars().next() {
            match ch {
                'b' | '\u{266d}' => {
                    accidental = accidental.saturating_sub(1);
                    body = &body[ch.len_utf8()..];
                }
                '#' | '\u{266f}' => {
                    accidental = accidental.saturating_add(1);
                    body = &body[ch.len_utf8()..];
                }
                _ => break,
            }
        }
        let accidental = i8::try_from(accidental).map_err(|_| TheoryError::DegreeSymbolUnknown)?;
        // 罗马数字本体：取最长的前缀
        let rest: &str = body;
        let rest_upper = rest.to_ascii_uppercase();
        let mut matched: Option<(u8, usize, bool)> = None;
        for symbol in ROMAN_MATCH_ORDER {
            if rest_upper.starts_with(symbol) {
                let degree = match symbol {
                    "VII" => 7u8,
                    "III" => 3,
                    "VI" => 6,
                    "IV" => 4,
                    "II" => 2,
                    "V" => 5,
                    _ => 1,
                };
                let consumed = symbol.len();
                let uppercase = !rest[..consumed].chars().any(|ch| ch.is_ascii_lowercase());
                matched = Some((degree, consumed, uppercase));
                break;
            }
        }
        let Some((degree, consumed, uppercase)) = matched else {
            return Err(TheoryError::DegreeSymbolUnknown);
        };
        let mut quality = if uppercase {
            RomanQuality::Major
        } else {
            RomanQuality::Minor
        };
        let mut suffix_text = &rest[consumed..];
        // 显式性质记号
        if let Some(stripped) = suffix_text.strip_prefix('\u{00b0}') {
            quality = RomanQuality::Diminished;
            suffix_text = stripped;
        } else if let Some(stripped) = suffix_text.strip_prefix('o') {
            quality = RomanQuality::Diminished;
            suffix_text = stripped;
        } else if let Some(stripped) = suffix_text.strip_prefix('\u{00f8}') {
            quality = RomanQuality::HalfDiminished;
            suffix_text = stripped;
        } else if let Some(stripped) = suffix_text.strip_prefix('+') {
            quality = RomanQuality::Augmented;
            suffix_text = stripped;
        }
        let suffix = if suffix_text.is_empty() {
            None
        } else {
            Some(ChordKind::from_suffix(suffix_text)?)
        };
        Ok(Self {
            degree,
            accidental,
            quality,
            suffix,
        })
    }

    /// 调内叠置三度的三和弦性质。
    ///
    /// 交给 [`Scale::triad_quality`] 计算：那里用"八度累计音程"而不是
    /// 取模后的音级差，避免跨八度时算错三度/五度。
    fn diatonic_quality(key: &Scale, degree: u8) -> RomanQuality {
        match key.triad_quality(u16::from(degree - 1)) {
            ChordKind::Major => RomanQuality::Major,
            ChordKind::Minor => RomanQuality::Minor,
            ChordKind::Diminished => RomanQuality::Diminished,
            ChordKind::Augmented => RomanQuality::Augmented,
            ChordKind::HalfDiminished7 => RomanQuality::HalfDiminished,
            _ => RomanQuality::Major,
        }
    }

    /// 在给定调上求出具体和弦。
    ///
    /// 变音记号作用在音阶音级上；性质优先级为
    /// 显式后缀 > 调内自然三和弦 > 罗马数字大小写。
    #[must_use]
    pub fn to_chord(self, key: &Scale) -> Chord {
        let base = self.degree - 1;
        let pc = key.degree_pitch_class(u16::from(base)).unwrap_or(key.tonic);
        let root = pc.transpose(i16::from(self.accidental));
        let tonality = if matches!(
            key.kind,
            ScaleKind::NaturalMinor
                | ScaleKind::HarmonicMinor
                | ScaleKind::MelodicMinor
                | ScaleKind::Aeolian
                | ScaleKind::Dorian
                | ScaleKind::Phrygian
        ) {
            Tonality::Minor
        } else if key.kind.prefer_flat(key.tonic) {
            Tonality::FlatMajor
        } else {
            Tonality::SharpMajor
        };
        let kind = if let Some(explicit) = self.suffix {
            explicit
        } else if self.accidental == 0 {
            let quality = Self::diatonic_quality(key, self.degree);
            // 罗马数字大小写与调内性质冲突时，以调内性质为准（例如和声小调的 V）。
            match (quality, self.quality) {
                // 调内是大三，而作曲家写了大写字母：大三（最常见）。
                (RomanQuality::Major, _) => RomanQuality::Major,
                // 调内是减/增：以调内性质为准（例如和声小调的 vii° 与 III+）。
                (RomanQuality::Diminished, _) => RomanQuality::Diminished,
                (RomanQuality::Augmented, _) => RomanQuality::Augmented,
                // 调内是小三，而作曲家写了大写字母（例如和声小调的 V）：
                // 尊重显式大写，做成大三和弦。
                (RomanQuality::Minor, RomanQuality::Major | RomanQuality::Augmented) => {
                    RomanQuality::Major
                }
                (RomanQuality::Minor, _) => RomanQuality::Minor,
                // 调内半减七（例如和声小调的第二级七和弦）：三和弦按减三处理。
                (RomanQuality::HalfDiminished, _) => RomanQuality::Diminished,
            }
            .triad_kind()
        } else {
            // 变音后的级数用"大小写 + 半音数"定性质，与调内音阶无关。
            self.quality.triad_kind()
        };
        Chord {
            root,
            kind,
            tonality,
            bass: None,
        }
    }

    /// 规范记号输出。
    #[must_use]
    pub fn symbol(&self) -> String {
        let mut text = String::with_capacity(8);
        match self.accidental {
            a if a < 0 => {
                for _ in 0..(-a) {
                    text.push('b');
                }
            }
            a if a > 0 => {
                for _ in 0..a {
                    text.push('#');
                }
            }
            _ => {}
        }
        let roman = ROMAN_TABLE[usize::from(self.degree) - 1];
        if self.quality == RomanQuality::Minor {
            text.push_str(&roman.to_ascii_lowercase());
        } else {
            text.push_str(roman);
        }
        match (self.suffix, self.quality) {
            (Some(kind), _) => text.push_str(kind.suffix()),
            (None, RomanQuality::Diminished) => text.push('\u{00b0}'),
            (None, RomanQuality::HalfDiminished) => text.push('\u{00f8}'),
            (None, RomanQuality::Augmented) => text.push('+'),
            (None, _) => {}
        }
        text
    }
}

impl core::fmt::Display for Degree {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.symbol())
    }
}

/// 一个和弦走向：有序的级数序列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progression {
    degrees: Vec<Degree>,
    meter: Meter,
    bars: u32,
}

impl Progression {
    /// 构造一个走向。
    ///
    /// # Errors
    ///
    /// `degrees` 为空时返回 [`TheoryError::EmptyProgression`]；`bars` 为 0 或
    /// `meter` 不是合法拍号（见 [`Meter::new`]）时返回 [`TheoryError::ZeroBars`]。
    pub fn new(degrees: Vec<Degree>, meter: Meter, bars: u32) -> Result<Self, TheoryError> {
        if degrees.is_empty() {
            return Err(TheoryError::EmptyProgression);
        }
        if bars == 0 {
            return Err(TheoryError::ZeroBars);
        }
        // 借用 `Meter::new` 的校验：`Meter` 的字段是公有的，调用方可以绕过它的
        // 构造器；分母为 0 的拍号会让 `ticks_per_bar` 的整除无法定义。
        Meter::new(meter.numerator, meter.denominator)?;
        Ok(Self {
            degrees,
            meter,
            bars,
        })
    }

    /// 解析 `"I-V-vi-IV"` / `"ii V I"` / `"i, VI, III, VII"` / `"I|V|vi|IV"`。
    ///
    /// 分隔符接受 `-`、空格、`,`、`|`、`·`；首尾空白忽略。
    ///
    /// # Errors
    ///
    /// 任一级数不合法，或结果为空时返回对应错误。
    pub fn parse(text: &str) -> Result<Self, TheoryError> {
        let mut degrees = Vec::new();
        let mut token = String::new();
        let flush = |token: &mut String, degrees: &mut Vec<Degree>| -> Result<(), TheoryError> {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                degrees.push(Degree::parse(trimmed)?);
            }
            token.clear();
            Ok(())
        };
        for ch in text.chars() {
            match ch {
                '-' | ' ' | '\t' | ',' | '|' | '\u{00b7}' | '\u{2013}' => {
                    flush(&mut token, &mut degrees)?;
                }
                _ => token.push(ch),
            }
        }
        flush(&mut token, &mut degrees)?;
        Self::new(degrees, Meter::COMMON, 4)
    }

    /// 级数序列。
    #[must_use]
    pub fn degrees(&self) -> &[Degree] {
        &self.degrees
    }

    /// 拍号。
    #[must_use]
    pub const fn meter(&self) -> Meter {
        self.meter
    }

    /// 小节数。
    #[must_use]
    pub const fn bars(&self) -> u32 {
        self.bars
    }

    /// 设置小节数。
    ///
    /// # Errors
    ///
    /// `bars` 为 0 时返回 [`TheoryError::ZeroBars`]。
    pub fn with_bars(mut self, bars: u32) -> Result<Self, TheoryError> {
        if bars == 0 {
            return Err(TheoryError::ZeroBars);
        }
        self.bars = bars;
        Ok(self)
    }

    /// 设置拍号。
    ///
    /// 本函数是 `const fn` 且不返回 `Result`（`Progression` 的字段对外不可见，
    /// 日常入口是 [`Progression::parse`]），因此它**不**校验拍号：非法拍号由
    /// [`Progression::expand`] 以 [`TheoryError::ZeroBars`] 拒绝，
    /// [`Progression::ticks_per_bar`] 与 [`Progression::total_ticks`] 给 `0`。
    #[must_use]
    pub const fn with_meter(mut self, meter: Meter) -> Self {
        self.meter = meter;
        self
    }

    /// 每小节 tick 数。
    #[must_use]
    pub const fn ticks_per_bar(&self) -> u64 {
        self.meter.ticks_per_bar()
    }

    /// 走向总时长（tick）。恒等于 `bars * ticks_per_bar`。
    #[must_use]
    pub const fn total_ticks(&self) -> u64 {
        self.bars as u64 * self.ticks_per_bar()
    }

    /// 把走向展开为整数 tick 的和弦序列。
    ///
    /// 展开规则（**确定性、无随机**，全部落在整数网格上，绝无浮点拍）：
    ///
    /// 1. **每小节一个和弦（级数循环）**：`degrees.len() <= bars` 时，级数按顺序
    ///    循环取用填满每一小节。例如 `ii-V-I` 铺 4 小节 → `ii` `V` `I` `ii`，
    ///    每个和弦恰好 1 小节。
    /// 2. **按四分音符拍均分**：级数比小节多、但仍放得下"每级至少一拍"时，
    ///    把整段时长按拍（960 tick）均分给各级，余数从前面的级数开始各多一拍。
    /// 3. **按 16 分音符均分**：连"每级一拍"都放不下时，退化到
    ///    [`MIN_DURATION_TICKS`]（240 tick）网格。
    /// 4. **放不下就报错**：级数多到连 16 分音符网格都不够（
    ///    `degrees.len() > total_ticks / MIN_DURATION_TICKS`）时返回
    ///    [`TheoryError::ProgressionTooDense`]，绝不静默丢弃级数或产出零长度区段。
    ///
    /// 任何情况下输出都满足：按 `start_tick` 严格升序、相邻区段无空隙无重叠、
    /// `duration_ticks` 恒为正且是 [`MIN_DURATION_TICKS`] 的整数倍，
    /// 总和恒等于 [`Progression::total_ticks`]。
    ///
    /// # Errors
    ///
    /// 走向为空时返回 [`TheoryError::EmptyProgression`]；拍号非法（分母为 0，
    /// 只能由 [`Progression::with_meter`] 灌进来）时返回 [`TheoryError::ZeroBars`]；
    /// 过密时返回 [`TheoryError::ProgressionTooDense`]。
    pub fn expand(&self, key: &Scale) -> Result<Vec<ChordSpan>, TheoryError> {
        if self.degrees.is_empty() {
            return Err(TheoryError::EmptyProgression);
        }
        // `with_meter` 不返回 `Result`，因此非法拍号在这里也要拒绝：否则
        // `ticks_per_bar` 的整除会退化（分母为 0 时旧码 panic），而"时值恒为正、
        // 首尾相接"这两条不变量在总时长为 0 时会同时失效。
        Meter::new(self.meter.numerator, self.meter.denominator)?;
        let ticks_per_bar = self.ticks_per_bar();
        let total = self.total_ticks();
        let bars = u64::from(self.bars);
        let degrees = self.degrees.len();
        let mut spans = Vec::new();
        let mut current_tick = 0u64;

        if u64::from(self.bars) >= degrees as u64 {
            // 规则 1：每小节一个级数，级数循环取用。
            for bar in 0..bars {
                let degree = self.degrees[(bar as usize) % degrees];
                spans.push(ChordSpan {
                    start_tick: current_tick,
                    duration_ticks: ticks_per_bar,
                    degree,
                    chord: degree.to_chord(key),
                });
                current_tick += ticks_per_bar;
            }
        } else {
            // 规则 2/3：按拍，再退到 16 分音符。
            let durations = distribute(total, degrees, PPQ)
                .or_else(|| distribute(total, degrees, MIN_DURATION_TICKS))
                .ok_or(TheoryError::ProgressionTooDense {
                    degrees,
                    slots: (total / MIN_DURATION_TICKS) as usize,
                })?;
            for (index, degree) in self.degrees.iter().enumerate() {
                let duration = durations[index];
                spans.push(ChordSpan {
                    start_tick: current_tick,
                    duration_ticks: duration,
                    degree: *degree,
                    chord: degree.to_chord(key),
                });
                current_tick += duration;
            }
        }
        debug_assert_eq!(current_tick, total);
        Ok(spans)
    }

    /// 只要和弦序列（丢掉时间信息）。
    #[must_use]
    pub fn chords(&self, key: &Scale) -> Vec<Chord> {
        self.degrees.iter().map(|d| d.to_chord(key)).collect()
    }
}

impl core::fmt::Display for Progression {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let parts: Vec<String> = self.degrees.iter().map(Degree::symbol).collect();
        f.write_str(&parts.join("-"))
    }
}

/// 把 `total` 个 tick 均分给 `parts` 份，每份至少 `unit` 个 tick。
///
/// 除不尽时，**前面的份**各多拿一个 `unit`（余数从头分配，而不是全塞给最后一份）。
/// `total` 不是 `unit` 的整数倍、或 `parts * unit > total` 时返回 `None`，
/// 由调用方决定退化到更细的网格还是报错。
fn distribute(total: u64, parts: usize, unit: u64) -> Option<Vec<u64>> {
    if parts == 0 || unit == 0 || !total.is_multiple_of(unit) {
        return None;
    }
    let units = total / unit;
    if units < parts as u64 {
        return None;
    }
    let base = units / parts as u64;
    let remainder = units % parts as u64;
    Some(
        (0..parts)
            .map(|index| {
                let extra = u64::from((index as u64) < remainder);
                (base + extra) * unit
            })
            .collect(),
    )
}

/// 走向展开后的一个区段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChordSpan {
    /// 起始 tick（960 PPQ）。
    pub start_tick: u64,
    /// 持续 tick 数（恒为正）。
    pub duration_ticks: u64,
    /// 原始级数。
    pub degree: Degree,
    /// 在该调上求出的具体和弦。
    pub chord: Chord,
}

impl ChordSpan {
    /// 结束 tick（开区间）。
    #[must_use]
    pub const fn end_tick(&self) -> u64 {
        self.start_tick + self.duration_ticks
    }
}

/// 便捷入口：`expand_progression(key, "I-V-vi-IV", 4)`。
///
/// # Errors
///
/// 走向文本非法、`bars` 为 0 时返回错误。
pub fn expand_progression(
    key: &Scale,
    degree_sequence: &str,
    bars: u32,
) -> Result<Vec<ChordSpan>, TheoryError> {
    Progression::parse(degree_sequence)?
        .with_bars(bars)?
        .expand(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::PitchClass;

    #[test]
    fn required_progressions_parse_to_the_expected_lengths() {
        for text in ["I-V-vi-IV", "ii-V-I", "i-VI-III-VII"] {
            let progression = Progression::parse(text).unwrap();
            assert!(!progression.degrees().is_empty(), "{text}");
            assert!(!progression.to_string().is_empty());
        }
        assert_eq!(Progression::parse("I-V-vi-IV").unwrap().degrees().len(), 4);
        assert_eq!(Progression::parse("ii-V-I").unwrap().degrees().len(), 3);
        assert_eq!(
            Progression::parse("i-VI-III-VII").unwrap().degrees().len(),
            4
        );
        assert_eq!(
            Progression::parse("i, VI, III, VII").unwrap(),
            Progression::parse("i-VI-III-VII").unwrap()
        );
        assert_eq!(
            Progression::parse("I | V | vi | IV").unwrap(),
            Progression::parse("I-V-vi-IV").unwrap()
        );
    }

    #[test]
    fn roman_numerals_carry_case_quality_and_accidentals() {
        assert_eq!(Degree::parse("I").unwrap().quality, RomanQuality::Major);
        assert_eq!(Degree::parse("ii").unwrap().quality, RomanQuality::Minor);
        assert_eq!(
            Degree::parse("vii\u{00b0}").unwrap().quality,
            RomanQuality::Diminished
        );
        assert_eq!(
            Degree::parse("III+").unwrap().quality,
            RomanQuality::Augmented
        );
        assert_eq!(
            Degree::parse("ii\u{00f8}7").unwrap().quality,
            RomanQuality::HalfDiminished
        );
        let flat_seven = Degree::parse("bVII").unwrap();
        assert_eq!(flat_seven.degree, 7);
        assert_eq!(flat_seven.accidental, -1);
        assert_eq!(Degree::parse("bVII").unwrap().symbol(), "bVII");
        assert!(Degree::parse("VIII").is_err());
        assert!(Degree::parse("H").is_err());
        assert!(Degree::parse("").is_err());
    }

    #[test]
    fn degree_to_chord_uses_the_diatonic_triad_of_the_key() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        let i = Degree::parse("I").unwrap().to_chord(&c_major);
        assert_eq!(i.root, PitchClass::C);
        assert_eq!(i.kind, ChordKind::Major);
        let ii = Degree::parse("ii").unwrap().to_chord(&c_major);
        assert_eq!(ii.root, PitchClass::D);
        assert_eq!(ii.kind, ChordKind::Minor);
        let vii = Degree::parse("vii").unwrap().to_chord(&c_major);
        assert_eq!(vii.root, PitchClass::B);
        assert_eq!(vii.kind, ChordKind::Diminished);
    }

    #[test]
    fn harmonic_minor_gets_a_major_dominant() {
        let a_harmonic = Scale::new(PitchClass::A, ScaleKind::HarmonicMinor);
        let v = Degree::parse("V").unwrap().to_chord(&a_harmonic);
        assert_eq!(v.root, PitchClass::E);
        assert_eq!(v.kind, ChordKind::Major, "harmonic minor V must be major");
        // 自然小调的 V 是小三
        let a_natural = Scale::new(PitchClass::A, ScaleKind::NaturalMinor);
        assert_eq!(
            Degree::parse("v").unwrap().to_chord(&a_natural).kind,
            ChordKind::Minor
        );
    }

    #[test]
    fn borrowed_degrees_keep_their_written_quality() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        let flat_seven = Degree::parse("bVII").unwrap().to_chord(&c_major);
        assert_eq!(flat_seven.root, PitchClass::AS);
        assert_eq!(flat_seven.kind, ChordKind::Major);
        let flat_two = Degree::parse("bII").unwrap().to_chord(&c_major);
        assert_eq!(flat_two.root, PitchClass::CS);
    }

    #[test]
    fn ii_v_i_in_c_is_d_minor_g_seven_c() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        // 3 小节 = 3 个级数，一小节一个和弦。
        let spans = expand_progression(&c_major, "ii-V-I", 3).unwrap();
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].chord.root, PitchClass::D);
        assert_eq!(spans[0].chord.kind, ChordKind::Minor);
        assert_eq!(spans[1].chord.root, PitchClass::G);
        assert_eq!(spans[1].chord.kind, ChordKind::Major);
        assert_eq!(spans[2].chord.root, PitchClass::C);
        assert_eq!(
            spans.iter().map(|s| s.duration_ticks).collect::<Vec<_>>(),
            vec![3840, 3840, 3840]
        );
        assert_eq!(spans[0].chord.symbol(), "Dm");
        assert_eq!(spans[1].chord.symbol(), "G");
        assert_eq!(spans[2].chord.symbol(), "C");
        assert_eq!(spans[2].start_tick, 7680);
    }

    #[test]
    fn fewer_degrees_than_bars_cycles_the_sequence_by_the_bar() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        // 4 小节 + 3 个级数 → ii V I ii（级数按顺序循环取用）
        let spans = expand_progression(&c_major, "ii-V-I", 4).unwrap();
        assert_eq!(spans.len(), 4);
        assert_eq!(
            spans.iter().map(|s| s.chord.symbol()).collect::<Vec<_>>(),
            vec!["Dm", "G", "C", "Dm"]
        );
        assert_eq!(
            spans.iter().map(|s| s.duration_ticks).collect::<Vec<_>>(),
            vec![3840, 3840, 3840, 3840]
        );
        // 1 小节 + 4 个级数 → 均分为四个 960 tick 的区段
        let tight = expand_progression(&c_major, "I-V-vi-IV", 1).unwrap();
        assert_eq!(tight.len(), 4);
        assert_eq!(
            tight.iter().map(|s| s.duration_ticks).collect::<Vec<_>>(),
            vec![960, 960, 960, 960]
        );
    }

    #[test]
    fn expanded_ticks_are_contiguous_and_sum_exactly_to_total() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        for (text, bars) in [
            ("I-V-vi-IV", 4u32),
            ("ii-V-I", 4),
            ("I", 1),
            ("I-V-vi-IV", 8),
        ] {
            let spans = expand_progression(&c_major, text, bars).unwrap();
            let total: u64 = spans.iter().map(|s| s.duration_ticks).sum();
            assert_eq!(total, u64::from(bars) * 3840, "{text}/{bars}");
            let mut cursor = 0u64;
            for span in &spans {
                assert_eq!(span.start_tick, cursor, "{text}");
                assert!(span.duration_ticks > 0, "{text}");
                cursor = span.end_tick();
            }
            assert_eq!(cursor, total);
        }
    }

    #[test]
    fn more_degrees_than_bars_spreads_evenly() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        let spans = expand_progression(&c_major, "I-ii-iii-IV-V-vi-vii-I", 4).unwrap();
        assert_eq!(spans.len(), 8);
        let total: u64 = spans.iter().map(|s| s.duration_ticks).sum();
        assert_eq!(total, 4 * 3840);
    }

    #[test]
    fn meter_changes_the_tick_grid() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        let progression = Progression::parse("I-V-vi-IV")
            .unwrap()
            .with_meter(Meter::WALTZ)
            .with_bars(4)
            .unwrap();
        assert_eq!(progression.ticks_per_bar(), 2880);
        let spans = progression.expand(&c_major).unwrap();
        assert_eq!(
            spans.iter().map(|s| s.duration_ticks).sum::<u64>(),
            4 * 2880
        );
        assert_eq!(Meter::COMPOUND_DUPLE.ticks_per_bar(), 2880);
        assert_eq!(Meter::SEVEN_EIGHT.ticks_per_bar(), 3360);
        assert!(Meter::new(4, 3).is_err());
    }

    #[test]
    fn empty_progression_and_zero_bars_are_rejected() {
        assert_eq!(
            Progression::parse("").unwrap_err(),
            TheoryError::EmptyProgression
        );
        let progression = Progression::parse("I").unwrap();
        assert_eq!(
            progression.clone().with_bars(0).unwrap_err(),
            TheoryError::ZeroBars
        );
        assert!(expand_progression(&Scale::new(PitchClass::C, ScaleKind::Major), "I", 0).is_err());
    }

    /// 前缀变音记号的个数**就是输入长度**：`Degree::parse` 用 `i8` 累加，
    /// 第 129 个降号（或第 128 个升号）让它溢出 —— 边界随符号不对称，正是
    /// "累加器宽度"而不是"音乐语义"在起作用。新码在 `i32` 上累加，只在装不下
    /// `i8` 时用既有的 [`TheoryError::DegreeSymbolUnknown`] 拒绝。
    #[test]
    fn degree_accidentals_that_do_not_fit_i8_are_rejected_instead_of_overflowing() {
        assert_eq!(Degree::parse("VII").unwrap().accidental, 0);
        assert_eq!(Degree::parse("bVII").unwrap().accidental, -1);
        assert_eq!(Degree::parse("bbVII").unwrap().accidental, -2);

        // 旧码不 panic 的整段边界（`i8::MIN` 恰好装得下 128 个降号，
        // `i8::MAX` 只装得下 127 个升号）⇒ 读数逐位保留。
        let deepest_flats = format!("{}VII", "b".repeat(128));
        assert_eq!(Degree::parse(&deepest_flats).unwrap().accidental, -128);
        let highest_sharps = format!("{}VII", "#".repeat(127));
        assert_eq!(Degree::parse(&highest_sharps).unwrap().accidental, 127);

        // 越出 `i8`：拒绝，不回绕、不 panic。
        for count in [129usize, 200, 4096] {
            assert!(
                Degree::parse(&format!("{}VII", "b".repeat(count))).is_err(),
                "{count} flats must be rejected"
            );
        }
        for count in [128usize, 200, 4096] {
            assert!(
                Degree::parse(&format!("{}VII", "#".repeat(count))).is_err(),
                "{count} sharps must be rejected"
            );
        }

        // 同一条路径的公开入口也必须报错而不是 panic。
        assert!(Progression::parse(&format!("{}VII", "b".repeat(4096))).is_err());
    }

    /// 分母为 0 的拍号只能由**公有字段**绕过 [`Meter::new`] 得到，而
    /// `ticks_per_bar` 会做整除 ⇒ 旧码 panic（debug 与 release 都 panic）。
    /// 拒绝发生在构造期与展开期，读数口给"空"值 0。
    #[test]
    fn a_meter_with_a_zero_denominator_is_rejected_and_never_panics() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        let broken = Meter {
            numerator: 4,
            denominator: 0,
        };
        assert!(Meter::new(4, 0).is_err());
        assert_eq!(broken.ticks_per_bar(), 0);

        // 构造期拒绝。
        let degrees = vec![Degree::parse("I").unwrap()];
        assert_eq!(
            Progression::new(degrees.clone(), broken, 4).unwrap_err(),
            TheoryError::ZeroBars
        );
        // `with_meter` 仍是无 `Result` 的 `const fn`，因此展开期拒绝。
        let via_setter = Progression::parse("I-V").unwrap().with_meter(broken);
        assert_eq!(via_setter.ticks_per_bar(), 0);
        assert_eq!(via_setter.total_ticks(), 0);
        assert_eq!(
            via_setter.expand(&c_major).unwrap_err(),
            TheoryError::ZeroBars
        );

        // 合法拍号的读数逐位不变（含非 2 的幂分母这一"旧码不 panic"的分支）。
        assert_eq!(Meter::COMMON.ticks_per_bar(), 3840);
        assert_eq!(Meter::WALTZ.ticks_per_bar(), 2880);
        assert_eq!(Meter::COMPOUND_DUPLE.ticks_per_bar(), 2880);
        assert_eq!(Meter::MARCH.ticks_per_bar(), 1920);
        assert_eq!(Meter::QUINTUPLE.ticks_per_bar(), 4800);
        assert_eq!(Meter::SEVEN_EIGHT.ticks_per_bar(), 3360);
        assert_eq!(
            Meter {
                numerator: 4,
                denominator: 3
            }
            .ticks_per_bar(),
            5120
        );
        assert_eq!(
            Meter {
                numerator: 0,
                denominator: 4
            }
            .ticks_per_bar(),
            0
        );
    }

    #[test]
    fn sixteenth_grid_is_used_when_whole_beats_are_not_enough() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        // 1 小节 = 4 拍 = 16 个 16 分音符；8 个级数 → 每个占 2 个 16 分音符。
        let spans = expand_progression(&c_major, "I-ii-iii-IV-V-vi-vii-I", 1).unwrap();
        assert_eq!(spans.len(), 8);
        assert_eq!(
            spans.iter().map(|s| s.duration_ticks).collect::<Vec<_>>(),
            vec![480; 8]
        );
        assert_eq!(spans[7].end_tick(), 3840);
        for span in &spans {
            assert_eq!(span.start_tick % MIN_DURATION_TICKS, 0);
        }
    }

    #[test]
    fn distribution_prefers_whole_beats_before_sixteenths() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        // 1 小节 4 拍，3 个级数：先按拍均分 → 2 + 1 + 1 拍。
        let spans = expand_progression(&c_major, "I-V-vi", 1).unwrap();
        assert_eq!(
            spans.iter().map(|s| s.duration_ticks).collect::<Vec<_>>(),
            vec![1920, 960, 960]
        );
        // 4 个级数 1 小节：正好一拍一个。
        let four = expand_progression(&c_major, "I-V-vi-IV", 1).unwrap();
        assert_eq!(
            four.iter().map(|s| s.duration_ticks).collect::<Vec<_>>(),
            vec![960, 960, 960, 960]
        );
    }

    #[test]
    fn too_dense_progressions_fail_loudly_instead_of_dropping_degrees() {
        let c_major = Scale::new(PitchClass::C, ScaleKind::Major);
        // 1 小节只有 16 个 16 分音符槽位；17 个级数放不下。
        let degrees = "I-".repeat(16) + "V";
        assert_eq!(Progression::parse(&degrees).unwrap().degrees().len(), 17);
        let error = expand_progression(&c_major, &degrees, 1).unwrap_err();
        assert_eq!(
            error,
            TheoryError::ProgressionTooDense {
                degrees: 17,
                slots: 16,
            }
        );
        assert!(error.is_input_error());
        // 恰好 16 个则必须成功，且每个正好一个 16 分音符。
        let exact = "I-".repeat(15) + "V";
        let spans = expand_progression(&c_major, &exact, 1).unwrap();
        assert_eq!(spans.len(), 16);
        assert!(
            spans
                .iter()
                .all(|span| span.duration_ticks == MIN_DURATION_TICKS)
        );
    }

    #[test]
    fn progression_display_round_trips() {
        for text in ["I-V-vi-IV", "ii-V-I", "i-VI-III-VII", "bVII-I"] {
            let progression = Progression::parse(text).unwrap();
            let reparsed = Progression::parse(&progression.to_string()).unwrap();
            assert_eq!(progression.degrees(), reparsed.degrees(), "{text}");
        }
    }
}
