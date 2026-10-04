//! 流派规则库 [ROAD-M4-003]。
//!
//! ## 许可纪律（先读这一段）
//!
//! `docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md` §7 的许可矩阵把
//! "159 种流派规则与和弦走向" 登记为 **CC0 / Public Domain**，并写明
//! "传统音乐理论与数学比例属于公有领域，规则编码归属于夜半原创代码"。
//!
//! 本模块严格按这条口径落地：
//!
//! 1. **不搬运任何第三方数据集**。`GENRES` 表里的每一条都是本 crate 作者依据
//!    公有领域的传统乐理常识（教会调式、功能和声、各地区的通行节拍与
//!    12 小节布鲁斯之类的公共形式）**自行撰写**的规则编码；
//! 2. **不复制受版权保护的文本**。名称只用通用英文/中文称法，描述字段只有
//!    BPM、拍号、级数、音阶这些公有领域的音乐事实，没有抄任何百科全书的句子；
//! 3. 每条规则都带一个 `SOURCE_*` 来源标记，逐条登记在
//!    `docs/ledger/theory-core-notes.md`。
//!
//! 与本仓库既有口径一致的取舍：**宁可条数少而合法，也不让不明许可的数据混进来**。
//!
//! ## 结构
//!
//! [`GenreRule`] 是纯数据记录；[`GenreLibrary`] 是它的只读视图，提供
//! `get` / `ids` / `search`。底层索引用 [`BTreeMap`]（不是 `HashMap`），
//! 因此迭代顺序在任何进程、任何平台上都一致 [MODEL-AST-003 的精神]。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::chord::Chord;
use crate::error::TheoryError;
use crate::progression::{ChordSpan, Meter, Progression, expand_progression};
use crate::scale::{Scale, ScaleKind};

/// 来源标记：传统音乐理论（公有领域）。
pub const SOURCE_TRADITIONAL_THEORY: &str = "traditional-theory/public-domain";
/// 来源标记：该流派的通行实践（地区性公共形式，无个人著作权）。
pub const SOURCE_COMMON_PRACTICE: &str = "common-practice/public-domain";
/// 来源标记：20 世纪通行商业实践（形式本身不受版权保护；仅登记 BPM/拍号等事实）。
pub const SOURCE_20C_COMMERCIAL_PRACTICE: &str = "20c-commercial-practice-facts";
/// 来源标记：本 crate 依据传统乐理自行编码（夜半原创）。
pub const SOURCE_YEBAN_ORIGINAL: &str = "yeban-original-encoding";

/// 一个流派的乐理规则。
///
/// 全部字段都是"音乐事实"级别的数据（速度区间、拍号、级数、音阶名），
/// 不含任何来自受版权保护文本的段落。
#[derive(Debug, Clone, Copy)]
pub struct GenreRule {
    /// 稳定 ID（小写蛇形，`GenreLibrary::get` 的键，对应 MCP `stylePreset`）。
    pub id: &'static str,
    /// 中文名。
    pub name_zh: &'static str,
    /// 英文名。
    pub name_en: &'static str,
    /// 默认速度区间 `(下限, 上限)` BPM。
    pub default_bpm_range: (u16, u16),
    /// 典型拍号（分子, 分母）。
    pub meter: (u8, u8),
    /// 典型级数走向（罗马数字，可被 [`Progression::parse`] 解析）。
    pub typical_progressions: &'static [&'static str],
    /// 典型音阶（可被 [`ScaleKind::parse`] 解析）。
    pub typical_scales: &'static [&'static str],
    /// 摇摆比例：50 = 平直八分音符，>50 = 三连音摇摆。`None` = 该流派不适用。
    pub swing: Option<f32>,
    /// 音符密度提示：每小节的典型音符数区间，供编曲骨架选密度用。
    pub note_density_hint: (u8, u8),
    /// 来源标记（见本模块顶部的 `SOURCE_*` 常量）。
    pub source: &'static str,
}

/// 手写 `PartialEq` 而不是派生：`swing` 是 `f32`，`f32` 不满足 `Eq`，
/// 而规则库的相等性在本 crate 里只用于测试断言（逐字段比较）。
impl PartialEq for GenreRule {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name_zh == other.name_zh
            && self.name_en == other.name_en
            && self.default_bpm_range == other.default_bpm_range
            && self.meter == other.meter
            && self.typical_progressions == other.typical_progressions
            && self.typical_scales == other.typical_scales
            && self.swing == other.swing
            && self.note_density_hint == other.note_density_hint
            && self.source == other.source
    }
}

impl Eq for GenreRule {}

impl GenreRule {
    /// 典型拍号。
    #[must_use]
    pub const fn meter_value(&self) -> Meter {
        Meter {
            numerator: self.meter.0,
            denominator: self.meter.1,
        }
    }

    /// 速度区间是否合法（非零且有序）。
    #[must_use]
    pub const fn has_valid_bpm_range(&self) -> bool {
        self.default_bpm_range.0 > 0 && self.default_bpm_range.0 <= self.default_bpm_range.1
    }

    /// 该流派的主调音阶（取第一个典型音阶）。
    ///
    /// # Errors
    ///
    /// `typical_scales` 为空或首个音阶名无法识别时返回
    /// [`TheoryError::ScaleNameUnknown`]。
    pub fn primary_scale(&self, tonic: crate::pitch::PitchClass) -> Result<Scale, TheoryError> {
        let kind = self
            .typical_scales
            .first()
            .ok_or(TheoryError::ScaleNameUnknown)?;
        Ok(Scale::new(tonic, ScaleKind::parse(kind)?))
    }

    /// 用该流派的第一条典型走向生成小节骨架。
    ///
    /// # Errors
    ///
    /// 没有典型走向、走向或音阶无法解析时返回相应错误。
    pub fn sketch(
        &self,
        tonic: crate::pitch::PitchClass,
        bars: u32,
    ) -> Result<Vec<ChordSpan>, TheoryError> {
        let progression = self
            .typical_progressions
            .first()
            .ok_or(TheoryError::EmptyProgression)?;
        let scale = self.primary_scale(tonic)?;
        let spans = expand_progression(&scale, progression, bars)?;
        Ok(spans)
    }

    /// 该流派第一条典型走向的和弦序列（不含时间信息）。
    ///
    /// # Errors
    ///
    /// 走向或音阶无法解析时返回相应错误。
    pub fn chords(&self, tonic: crate::pitch::PitchClass) -> Result<Vec<Chord>, TheoryError> {
        let progression = self
            .typical_progressions
            .first()
            .ok_or(TheoryError::EmptyProgression)?;
        let scale = self.primary_scale(tonic)?;
        Progression::parse(progression)?
            .with_bars(4)
            .map(|p| p.chords(&scale))
    }
}

/// 流派规则表。
///
/// 分节组织，每节前注明该节的规则来源口径。**新增条目必须同时**：
/// 1. 在 `docs/ledger/theory-core-notes.md` 登记来源与许可；
/// 2. 让 `idiomatic_data_is_structurally_valid` 测试通过（走向/音阶可解析）。
pub static GENRES: &[GenreRule] = &[
    // ------------------------------------------------------------------
    // 第一节：西方古典 / 教会传统
    // 来源：传统乐理（SOURCE_TRADITIONAL_THEORY）。级数与调式均为公有领域。
    // ------------------------------------------------------------------
    GenreRule {
        id: "gregorian_chant",
        name_zh: "格里高利圣咏",
        name_en: "Gregorian chant",
        default_bpm_range: (50, 76),
        meter: (4, 4),
        typical_progressions: &["I-IV-I", "I-vi-IV-I"],
        typical_scales: &["dorian", "phrygian", "mixolydian", "ionian"],
        swing: None,
        note_density_hint: (4, 12),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "renaissance_polyphony",
        name_zh: "文艺复兴复调",
        name_en: "Renaissance polyphony",
        default_bpm_range: (60, 88),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-vi-ii-V"],
        typical_scales: &["dorian", "ionian", "aeolian"],
        swing: None,
        note_density_hint: (16, 40),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "baroque_chorale",
        name_zh: "巴洛克众赞歌",
        name_en: "Baroque chorale",
        default_bpm_range: (56, 84),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-ii-V-I", "I-vi-ii-V-I"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (12, 32),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "baroque_fugue",
        name_zh: "巴洛克赋格",
        name_en: "Baroque fugue",
        default_bpm_range: (66, 104),
        meter: (4, 4),
        typical_progressions: &["I-V-I", "i-V-i", "I-IV-V-I"],
        typical_scales: &["major", "natural_minor", "dorian"],
        swing: None,
        note_density_hint: (24, 64),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "baroque_suite",
        name_zh: "巴洛克组曲",
        name_en: "Baroque dance suite",
        default_bpm_range: (72, 132),
        meter: (3, 4),
        typical_progressions: &["I-V-I", "i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "classical_sonata",
        name_zh: "古典奏鸣曲",
        name_en: "Classical sonata",
        default_bpm_range: (80, 132),
        meter: (4, 4),
        typical_progressions: &["I-V-I", "I-ii-V-I", "I-IV-V-I"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "classical_minuet",
        name_zh: "小步舞曲",
        name_en: "Minuet",
        default_bpm_range: (108, 132),
        meter: (3, 4),
        typical_progressions: &["I-V-I", "I-IV-V-I"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (12, 32),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "waltz",
        name_zh: "圆舞曲",
        name_en: "Waltz",
        default_bpm_range: (84, 180),
        meter: (3, 4),
        typical_progressions: &["I-V-I", "I-IV-V-I", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (12, 36),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "march",
        name_zh: "进行曲",
        name_en: "March",
        default_bpm_range: (100, 140),
        meter: (2, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (8, 24),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "romantic_lied",
        name_zh: "浪漫主义艺术歌曲",
        name_en: "Romantic Lied",
        default_bpm_range: (56, 96),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "i-VI-III-VII", "I-vi-ii-V"],
        typical_scales: &["major", "natural_minor", "harmonic_minor"],
        swing: None,
        note_density_hint: (12, 36),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "nocturne",
        name_zh: "夜曲",
        name_en: "Nocturne",
        default_bpm_range: (52, 84),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor", "melodic_minor"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "etude",
        name_zh: "练习曲",
        name_en: "Étude",
        default_bpm_range: (80, 176),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "i-VII-VI-VII"],
        typical_scales: &["major", "natural_minor", "chromatic"],
        swing: None,
        note_density_hint: (32, 96),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "impressionism",
        name_zh: "印象主义",
        name_en: "Impressionism",
        default_bpm_range: (54, 92),
        meter: (4, 4),
        typical_progressions: &["I-ii-I", "I-VI-II-V", "i-III-VI-III"],
        typical_scales: &["whole_tone", "pentatonic_major", "lydian", "dorian"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "impressionist_piano",
        name_zh: "印象派钢琴",
        name_en: "Impressionist piano",
        default_bpm_range: (50, 88),
        meter: (4, 4),
        typical_progressions: &["I-IV-I", "I-ii-V-I"],
        typical_scales: &["whole_tone", "pentatonic_major", "lydian"],
        swing: None,
        note_density_hint: (20, 64),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "orchestral_film_score",
        name_zh: "管弦配乐",
        name_en: "Orchestral film score",
        default_bpm_range: (60, 140),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "I-V-vi-IV", "i-iv-VII-III"],
        typical_scales: &["natural_minor", "harmonic_minor", "dorian", "aeolian"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "epic_trailer",
        name_zh: "史诗预告片配乐",
        name_en: "Epic trailer music",
        default_bpm_range: (70, 130),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "i-iv-VI-VII"],
        typical_scales: &["natural_minor", "harmonic_minor", "phrygian"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "minimalism",
        name_zh: "极简主义",
        name_en: "Minimalism",
        default_bpm_range: (72, 160),
        meter: (4, 4),
        typical_progressions: &["I-IV-I", "i-VII-i"],
        typical_scales: &["major", "natural_minor", "pentatonic_major", "dorian"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "hymn",
        name_zh: "赞美诗",
        name_en: "Hymn",
        default_bpm_range: (60, 92),
        meter: (4, 4),
        typical_progressions: &["I-IV-I-V-I", "I-vi-IV-V"],
        typical_scales: &["major", "ionian"],
        swing: None,
        note_density_hint: (8, 24),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "anthem",
        name_zh: "颂歌",
        name_en: "Anthem",
        default_bpm_range: (64, 108),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-V-vi-IV"],
        typical_scales: &["major"],
        swing: None,
        note_density_hint: (8, 28),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "carol",
        name_zh: "圣诞颂歌",
        name_en: "Carol",
        default_bpm_range: (76, 132),
        meter: (3, 4),
        typical_progressions: &["I-IV-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (8, 24),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "lullaby",
        name_zh: "摇篮曲",
        name_en: "Lullaby",
        default_bpm_range: (52, 80),
        meter: (3, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major", "pentatonic_major"],
        swing: None,
        note_density_hint: (4, 16),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "opera_aria",
        name_zh: "歌剧咏叹调",
        name_en: "Opera aria",
        default_bpm_range: (60, 104),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "I-IV-V-I", "i-VI-iv-V"],
        typical_scales: &["major", "natural_minor", "harmonic_minor"],
        swing: None,
        note_density_hint: (12, 48),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "operetta",
        name_zh: "轻歌剧",
        name_en: "Operetta",
        default_bpm_range: (88, 152),
        meter: (3, 4),
        typical_progressions: &["I-V-I", "I-IV-V-I"],
        typical_scales: &["major"],
        swing: Some(56.0),
        note_density_hint: (12, 36),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "ballet",
        name_zh: "芭蕾配乐",
        name_en: "Ballet score",
        default_bpm_range: (64, 144),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor", "harmonic_minor"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "passacaglia",
        name_zh: "帕萨卡利亚",
        name_en: "Passacaglia",
        default_bpm_range: (56, 88),
        meter: (3, 4),
        typical_progressions: &["i-VII-VI-V", "i-iv-VII-III"],
        typical_scales: &["natural_minor", "harmonic_minor", "dorian"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "chaconne",
        name_zh: "恰空",
        name_en: "Chaconne",
        default_bpm_range: (60, 96),
        meter: (3, 4),
        typical_progressions: &["i-VII-VI-VII", "i-iv-V-i"],
        typical_scales: &["natural_minor", "harmonic_minor"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "toccata",
        name_zh: "托卡塔",
        name_en: "Toccata",
        default_bpm_range: (92, 152),
        meter: (4, 4),
        typical_progressions: &["i-V-i", "I-V-I"],
        typical_scales: &["natural_minor", "harmonic_minor", "major"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "prelude",
        name_zh: "前奏曲",
        name_en: "Prelude",
        default_bpm_range: (66, 120),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V-I", "i-VII-VI-VII"],
        typical_scales: &["major", "natural_minor", "chromatic"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "sonatina",
        name_zh: "小奏鸣曲",
        name_en: "Sonatina",
        default_bpm_range: (88, 132),
        meter: (4, 4),
        typical_progressions: &["I-V-I", "I-ii-V-I"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (16, 40),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    // ------------------------------------------------------------------
    // 第二节：爵士与布鲁斯
    // 来源：20 世纪通行实践（SOURCE_20C_COMMERCIAL_PRACTICE）+ 传统乐理。
    // 走向（ii-V-I、12 小节布鲁斯）是公有领域的公共形式。
    // ------------------------------------------------------------------
    GenreRule {
        id: "jazz_swing",
        name_zh: "摇摆爵士",
        name_en: "Swing jazz",
        default_bpm_range: (120, 260),
        meter: (4, 4),
        typical_progressions: &["ii-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "dorian", "mixolydian"],
        swing: Some(66.0),
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "big_band",
        name_zh: "大乐队",
        name_en: "Big band",
        default_bpm_range: (110, 220),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "ii-V-I"],
        typical_scales: &["major", "blues", "mixolydian"],
        swing: Some(66.0),
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "bebop",
        name_zh: "比博普",
        name_en: "Bebop",
        default_bpm_range: (180, 320),
        meter: (4, 4),
        typical_progressions: &["ii-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "dorian", "mixolydian", "blues"],
        swing: Some(58.0),
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "hard_bop",
        name_zh: "硬博普",
        name_en: "Hard bop",
        default_bpm_range: (140, 260),
        meter: (4, 4),
        typical_progressions: &["ii-V-I", "i-iv-i", "I-vi-ii-V"],
        typical_scales: &["blues", "dorian", "mixolydian", "major"],
        swing: Some(64.0),
        note_density_hint: (24, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "cool_jazz",
        name_zh: "冷爵士",
        name_en: "Cool jazz",
        default_bpm_range: (88, 180),
        meter: (4, 4),
        typical_progressions: &["ii-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "dorian", "lydian"],
        swing: Some(60.0),
        note_density_hint: (12, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "modal_jazz",
        name_zh: "调式爵士",
        name_en: "Modal jazz",
        default_bpm_range: (96, 200),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv", "I-IV"],
        typical_scales: &["dorian", "mixolydian", "lydian", "aeolian"],
        swing: Some(60.0),
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "free_jazz",
        name_zh: "自由爵士",
        name_en: "Free jazz",
        default_bpm_range: (80, 240),
        meter: (4, 4),
        typical_progressions: &["i-VI", "I-ii"],
        typical_scales: &["chromatic", "whole_tone", "locrian", "blues"],
        swing: None,
        note_density_hint: (32, 160),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "bossa_nova",
        name_zh: "巴萨诺瓦",
        name_en: "Bossa nova",
        default_bpm_range: (96, 140),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "ii-V-I", "i-VII-VI-VII"],
        typical_scales: &["major", "dorian", "natural_minor"],
        swing: Some(54.0),
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "latin_jazz",
        name_zh: "拉丁爵士",
        name_en: "Latin jazz",
        default_bpm_range: (120, 220),
        meter: (4, 4),
        typical_progressions: &["ii-V-I", "i-iv-V-i"],
        typical_scales: &["major", "dorian", "blues", "mixolydian"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "smooth_jazz",
        name_zh: "顺畅爵士",
        name_en: "Smooth jazz",
        default_bpm_range: (80, 116),
        meter: (4, 4),
        typical_progressions: &["ii-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "dorian"],
        swing: Some(54.0),
        note_density_hint: (8, 24),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "jazz_waltz",
        name_zh: "爵士圆舞曲",
        name_en: "Jazz waltz",
        default_bpm_range: (120, 220),
        meter: (3, 4),
        typical_progressions: &["ii-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "dorian", "melodic_minor"],
        swing: Some(62.0),
        note_density_hint: (12, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "gypsy_jazz",
        name_zh: "吉普赛爵士",
        name_en: "Gypsy jazz",
        default_bpm_range: (140, 280),
        meter: (4, 4),
        typical_progressions: &["ii-V-I", "I-vi-ii-V", "i-VII-VI-VII"],
        typical_scales: &["harmonic_minor", "dorian", "major", "blues"],
        swing: Some(66.0),
        note_density_hint: (24, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "ragtime",
        name_zh: "拉格泰姆",
        name_en: "Ragtime",
        default_bpm_range: (88, 140),
        meter: (2, 4),
        typical_progressions: &["I-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "chromatic"],
        swing: None,
        note_density_hint: (24, 80),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "stride_piano",
        name_zh: "跨步钢琴",
        name_en: "Stride piano",
        default_bpm_range: (110, 200),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "ii-V-I"],
        typical_scales: &["major", "blues", "chromatic"],
        swing: Some(62.0),
        note_density_hint: (32, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "boogie_woogie",
        name_zh: "布吉伍吉",
        name_en: "Boogie-woogie",
        default_bpm_range: (100, 180),
        meter: (4, 4),
        typical_progressions: &["I-IV-I-V-I", "I-V-IV-I"],
        typical_scales: &["blues", "major"],
        swing: Some(62.0),
        note_density_hint: (32, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "blues",
        name_zh: "布鲁斯",
        name_en: "Blues",
        default_bpm_range: (60, 160),
        meter: (4, 4),
        typical_progressions: &["I-IV-I-V-I", "i-iv-i-V-i"],
        typical_scales: &["blues", "pentatonic_minor", "mixolydian"],
        swing: Some(64.0),
        note_density_hint: (8, 32),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "delta_blues",
        name_zh: "三角洲布鲁斯",
        name_en: "Delta blues",
        default_bpm_range: (58, 104),
        meter: (4, 4),
        typical_progressions: &["I-IV-I-V-I", "i-iv-i-V-i"],
        typical_scales: &["blues", "pentatonic_minor"],
        swing: Some(58.0),
        note_density_hint: (4, 16),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "chicago_blues",
        name_zh: "芝加哥布鲁斯",
        name_en: "Chicago blues",
        default_bpm_range: (80, 160),
        meter: (4, 4),
        typical_progressions: &["I-IV-I-V-I", "I-IV-V-I"],
        typical_scales: &["blues", "mixolydian", "pentatonic_minor"],
        swing: Some(62.0),
        note_density_hint: (12, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "jump_blues",
        name_zh: "跳跃布鲁斯",
        name_en: "Jump blues",
        default_bpm_range: (130, 220),
        meter: (4, 4),
        typical_progressions: &["I-IV-I-V-I", "I-vi-ii-V"],
        typical_scales: &["blues", "major", "mixolydian"],
        swing: Some(66.0),
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "blues_rock",
        name_zh: "布鲁斯摇滚",
        name_en: "Blues rock",
        default_bpm_range: (90, 170),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "i-iv-i"],
        typical_scales: &["blues", "pentatonic_minor", "mixolydian"],
        swing: Some(56.0),
        note_density_hint: (12, 40),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "gospel",
        name_zh: "福音音乐",
        name_en: "Gospel",
        default_bpm_range: (68, 140),
        meter: (4, 4),
        typical_progressions: &["I-IV-I-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "pentatonic_major", "blues"],
        swing: Some(58.0),
        note_density_hint: (12, 48),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "spiritual",
        name_zh: "灵歌",
        name_en: "Spiritual",
        default_bpm_range: (60, 104),
        meter: (4, 4),
        typical_progressions: &["I-IV-I-V-I", "I-V-I"],
        typical_scales: &["major", "pentatonic_major", "blues"],
        swing: None,
        note_density_hint: (6, 20),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "work_song",
        name_zh: "劳动号子",
        name_en: "Work song",
        default_bpm_range: (72, 132),
        meter: (4, 4),
        typical_progressions: &["I-IV-I", "i-iv-i"],
        typical_scales: &["pentatonic_minor", "blues", "major"],
        swing: None,
        note_density_hint: (4, 16),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "field_holler",
        name_zh: "田野呼喊",
        name_en: "Field holler",
        default_bpm_range: (52, 92),
        meter: (4, 4),
        typical_progressions: &["I-IV-I", "i-iv-i"],
        typical_scales: &["blues", "pentatonic_minor"],
        swing: None,
        note_density_hint: (2, 10),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    // ------------------------------------------------------------------
    // 第三节：灵魂 / 放克 / R&B / 迪斯科
    // 来源：20 世纪通行商业实践（只登记速度、拍号、级数等事实）。
    // ------------------------------------------------------------------
    GenreRule {
        id: "soul",
        name_zh: "灵魂乐",
        name_en: "Soul",
        default_bpm_range: (72, 140),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "i-VI-III-VII", "I-vi-ii-V"],
        typical_scales: &["major", "pentatonic_minor", "blues"],
        swing: Some(56.0),
        note_density_hint: (12, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "motown",
        name_zh: "摩城",
        name_en: "Motown",
        default_bpm_range: (100, 148),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "I-IV-V-I"],
        typical_scales: &["major", "pentatonic_major"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "funk",
        name_zh: "放克",
        name_en: "Funk",
        default_bpm_range: (90, 130),
        meter: (4, 4),
        typical_progressions: &["i-VII", "I7-IV7", "i-iv"],
        typical_scales: &["dorian", "mixolydian", "blues", "pentatonic_minor"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "p_funk",
        name_zh: "P 放克",
        name_en: "P-funk",
        default_bpm_range: (96, 128),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv-VII"],
        typical_scales: &["dorian", "mixolydian", "blues"],
        swing: None,
        note_density_hint: (24, 80),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "disco",
        name_zh: "迪斯科",
        name_en: "Disco",
        default_bpm_range: (112, 132),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "I-vi-ii-V", "i-iv-VII-III"],
        typical_scales: &["natural_minor", "dorian", "major"],
        swing: None,
        note_density_hint: (24, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "boogie",
        name_zh: "布吉舞曲",
        name_en: "Boogie",
        default_bpm_range: (108, 136),
        meter: (4, 4),
        typical_progressions: &["i-VII", "I-IV"],
        typical_scales: &["natural_minor", "blues"],
        swing: None,
        note_density_hint: (24, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "contemporary_rnb",
        name_zh: "当代 R&B",
        name_en: "Contemporary R&B",
        default_bpm_range: (64, 104),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "I-vi-ii-V", "ii-V-I"],
        typical_scales: &["natural_minor", "major", "dorian"],
        swing: Some(54.0),
        note_density_hint: (12, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "neo_soul",
        name_zh: "新灵魂乐",
        name_en: "Neo soul",
        default_bpm_range: (68, 104),
        meter: (4, 4),
        typical_progressions: &["ii-V-I", "i-VII-VI-VII", "I-vi-ii-V"],
        typical_scales: &["dorian", "major", "melodic_minor", "blues"],
        swing: Some(56.0),
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "quiet_storm",
        name_zh: "静夜风暴",
        name_en: "Quiet storm",
        default_bpm_range: (60, 92),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "ii-V-I"],
        typical_scales: &["major", "dorian", "natural_minor"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "doo_wop",
        name_zh: "嘟喔普",
        name_en: "Doo-wop",
        default_bpm_range: (76, 128),
        meter: (4, 4),
        typical_progressions: &["I-vi-IV-V", "I-vi-ii-V"],
        typical_scales: &["major"],
        swing: None,
        note_density_hint: (8, 24),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    // ------------------------------------------------------------------
    // 第四节：摇滚与金属
    // 来源：20 世纪通行商业实践。
    // ------------------------------------------------------------------
    GenreRule {
        id: "rock_and_roll",
        name_zh: "摇滚乐",
        name_en: "Rock and roll",
        default_bpm_range: (120, 200),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-vi-ii-V", "I-V-vi-IV"],
        typical_scales: &["major", "blues", "pentatonic_major"],
        swing: Some(56.0),
        note_density_hint: (12, 40),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "rockabilly",
        name_zh: "洛卡比里",
        name_en: "Rockabilly",
        default_bpm_range: (140, 220),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major", "blues", "pentatonic_major"],
        swing: Some(58.0),
        note_density_hint: (16, 56),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "surf_rock",
        name_zh: "冲浪摇滚",
        name_en: "Surf rock",
        default_bpm_range: (120, 176),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "i-VII-VI-VII"],
        typical_scales: &["major", "pentatonic_major", "dorian"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "garage_rock",
        name_zh: "车库摇滚",
        name_en: "Garage rock",
        default_bpm_range: (120, 180),
        meter: (4, 4),
        typical_progressions: &["I-IV-V", "i-VII"],
        typical_scales: &["major", "pentatonic_minor", "blues"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "psychedelic_rock",
        name_zh: "迷幻摇滚",
        name_en: "Psychedelic rock",
        default_bpm_range: (76, 140),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["dorian", "mixolydian", "pentatonic_minor"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "progressive_rock",
        name_zh: "前卫摇滚",
        name_en: "Progressive rock",
        default_bpm_range: (70, 176),
        meter: (7, 8),
        typical_progressions: &["i-VI-III-VII", "I-vi-ii-V"],
        typical_scales: &["natural_minor", "dorian", "mixolydian", "whole_tone"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "hard_rock",
        name_zh: "硬摇滚",
        name_en: "Hard rock",
        default_bpm_range: (100, 160),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V", "i-iv-VII"],
        typical_scales: &["pentatonic_minor", "natural_minor", "blues"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "heavy_metal",
        name_zh: "重金属",
        name_en: "Heavy metal",
        default_bpm_range: (100, 180),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "i-iv-VII-III"],
        typical_scales: &["natural_minor", "phrygian", "pentatonic_minor"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "thrash_metal",
        name_zh: "鞭击金属",
        name_en: "Thrash metal",
        default_bpm_range: (160, 260),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-V", "i-iv-VII-III"],
        typical_scales: &["natural_minor", "phrygian", "locrian"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "death_metal",
        name_zh: "死亡金属",
        name_en: "Death metal",
        default_bpm_range: (140, 260),
        meter: (4, 4),
        typical_progressions: &["i-VI-VII", "i-iv-VII"],
        typical_scales: &["phrygian", "locrian", "harmonic_minor"],
        swing: None,
        note_density_hint: (48, 160),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "black_metal",
        name_zh: "黑金属",
        name_en: "Black metal",
        default_bpm_range: (120, 240),
        meter: (4, 4),
        typical_progressions: &["i-VI-VII", "i-VII-VI"],
        typical_scales: &["natural_minor", "phrygian", "locrian"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "doom_metal",
        name_zh: "末日金属",
        name_en: "Doom metal",
        default_bpm_range: (50, 90),
        meter: (4, 4),
        typical_progressions: &["i-VI-VII", "i-iv-i"],
        typical_scales: &["natural_minor", "phrygian", "blues"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "power_metal",
        name_zh: "力量金属",
        name_en: "Power metal",
        default_bpm_range: (130, 200),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "I-V-vi-IV"],
        typical_scales: &["natural_minor", "harmonic_minor", "major"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "progressive_metal",
        name_zh: "前卫金属",
        name_en: "Progressive metal",
        default_bpm_range: (90, 200),
        meter: (7, 8),
        typical_progressions: &["i-VI-III-VII", "i-iv-VII-III"],
        typical_scales: &["natural_minor", "dorian", "whole_tone", "locrian"],
        swing: None,
        note_density_hint: (32, 160),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "metalcore",
        name_zh: "金属核",
        name_en: "Metalcore",
        default_bpm_range: (130, 220),
        meter: (4, 4),
        typical_progressions: &["i-VI-VII", "i-iv-VII"],
        typical_scales: &["natural_minor", "phrygian", "blues"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "nu_metal",
        name_zh: "新金属",
        name_en: "Nu metal",
        default_bpm_range: (80, 140),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv"],
        typical_scales: &["natural_minor", "phrygian", "blues"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "punk_rock",
        name_zh: "朋克摇滚",
        name_en: "Punk rock",
        default_bpm_range: (140, 220),
        meter: (4, 4),
        typical_progressions: &["I-IV-V", "i-VII-VI-VII"],
        typical_scales: &["major", "pentatonic_minor", "blues"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "pop_punk",
        name_zh: "流行朋克",
        name_en: "Pop punk",
        default_bpm_range: (140, 200),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "I-IV-V-I"],
        typical_scales: &["major", "pentatonic_major"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "post_punk",
        name_zh: "后朋克",
        name_en: "Post-punk",
        default_bpm_range: (110, 170),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "i-iv"],
        typical_scales: &["natural_minor", "dorian", "phrygian"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "new_wave",
        name_zh: "新浪潮",
        name_en: "New wave",
        default_bpm_range: (110, 160),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor", "dorian"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "shoegaze",
        name_zh: "自赏",
        name_en: "Shoegaze",
        default_bpm_range: (76, 130),
        meter: (4, 4),
        typical_progressions: &["I-IV", "i-VI-III-VII"],
        typical_scales: &["major", "dorian", "lydian", "pentatonic_major"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "grunge",
        name_zh: "垃圾摇滚",
        name_en: "Grunge",
        default_bpm_range: (80, 150),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V"],
        typical_scales: &["pentatonic_minor", "natural_minor", "blues"],
        swing: None,
        note_density_hint: (12, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "alternative_rock",
        name_zh: "另类摇滚",
        name_en: "Alternative rock",
        default_bpm_range: (90, 150),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "I-V-vi-IV"],
        typical_scales: &["natural_minor", "major", "dorian"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "indie_rock",
        name_zh: "独立摇滚",
        name_en: "Indie rock",
        default_bpm_range: (96, 150),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "I-vi-ii-V"],
        typical_scales: &["major", "dorian", "major"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "math_rock",
        name_zh: "数学摇滚",
        name_en: "Math rock",
        default_bpm_range: (110, 190),
        meter: (7, 8),
        typical_progressions: &["I-IV", "i-VI"],
        typical_scales: &["major", "dorian", "lydian", "whole_tone"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "post_rock",
        name_zh: "后摇滚",
        name_en: "Post-rock",
        default_bpm_range: (60, 130),
        meter: (4, 4),
        typical_progressions: &["I-IV-I", "i-VI-III"],
        typical_scales: &["major", "lydian", "natural_minor", "pentatonic_major"],
        swing: None,
        note_density_hint: (16, 80),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "emo",
        name_zh: "情绪摇滚",
        name_en: "Emo",
        default_bpm_range: (110, 180),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor", "pentatonic_minor"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    // ------------------------------------------------------------------
    // 第五节：电子舞曲与嘻哈
    // 来源：20 世纪通行商业实践。四拍底鼓、级数循环等属于公共形式。
    // ------------------------------------------------------------------
    GenreRule {
        id: "house",
        name_zh: "浩室",
        name_en: "House",
        default_bpm_range: (118, 130),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "i-iv-VII", "I-vi-ii-V"],
        typical_scales: &["natural_minor", "dorian", "major"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "deep_house",
        name_zh: "深度浩室",
        name_en: "Deep house",
        default_bpm_range: (110, 125),
        meter: (4, 4),
        typical_progressions: &["i-iv-VII-III", "ii-V-I"],
        typical_scales: &["dorian", "natural_minor", "major"],
        swing: None,
        note_density_hint: (12, 40),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "tech_house",
        name_zh: "科技浩室",
        name_en: "Tech house",
        default_bpm_range: (122, 130),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv"],
        typical_scales: &["natural_minor", "phrygian", "dorian"],
        swing: None,
        note_density_hint: (24, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "progressive_house",
        name_zh: "前卫浩室",
        name_en: "Progressive house",
        default_bpm_range: (124, 132),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "i-VII-VI-VII"],
        typical_scales: &["natural_minor", "dorian", "aeolian"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "garage_house",
        name_zh: "车库浩室",
        name_en: "Garage house",
        default_bpm_range: (120, 132),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI", "ii-V-I"],
        typical_scales: &["natural_minor", "dorian", "major"],
        swing: Some(56.0),
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "techno",
        name_zh: "铁克诺",
        name_en: "Techno",
        default_bpm_range: (125, 150),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i"],
        typical_scales: &["natural_minor", "phrygian", "chromatic"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "minimal_techno",
        name_zh: "极简铁克诺",
        name_en: "Minimal techno",
        default_bpm_range: (120, 132),
        meter: (4, 4),
        typical_progressions: &["i", "i-VII"],
        typical_scales: &["natural_minor", "phrygian", "chromatic"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "trance",
        name_zh: "迷幻舞曲",
        name_en: "Trance",
        default_bpm_range: (128, 145),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "i-VII-VI-III"],
        typical_scales: &["natural_minor", "harmonic_minor", "dorian"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "psytrance",
        name_zh: "迷幻出神",
        name_en: "Psytrance",
        default_bpm_range: (138, 150),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-VI"],
        typical_scales: &["phrygian", "natural_minor", "harmonic_minor"],
        swing: None,
        note_density_hint: (48, 160),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "hardstyle",
        name_zh: "硬派",
        name_en: "Hardstyle",
        default_bpm_range: (145, 160),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "i-VII-VI"],
        typical_scales: &["natural_minor", "harmonic_minor"],
        swing: None,
        note_density_hint: (32, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "dubstep",
        name_zh: "回响贝斯",
        name_en: "Dubstep",
        default_bpm_range: (138, 145),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-VI"],
        typical_scales: &["natural_minor", "phrygian", "blues"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "drum_and_bass",
        name_zh: "鼓打贝斯",
        name_en: "Drum and bass",
        default_bpm_range: (165, 180),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI", "i-iv"],
        typical_scales: &["natural_minor", "dorian", "blues"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "jungle",
        name_zh: "丛林",
        name_en: "Jungle",
        default_bpm_range: (155, 175),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv"],
        typical_scales: &["natural_minor", "dorian", "blues"],
        swing: None,
        note_density_hint: (32, 160),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "breakbeat",
        name_zh: "碎拍",
        name_en: "Breakbeat",
        default_bpm_range: (120, 145),
        meter: (4, 4),
        typical_progressions: &["i-VII", "I-IV"],
        typical_scales: &["natural_minor", "mixolydian", "blues"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "big_beat",
        name_zh: "大节拍",
        name_en: "Big beat",
        default_bpm_range: (120, 140),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv"],
        typical_scales: &["natural_minor", "mixolydian", "blues"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "trip_hop",
        name_zh: "神游舞曲",
        name_en: "Trip hop",
        default_bpm_range: (75, 100),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "i-iv-VII"],
        typical_scales: &["natural_minor", "dorian", "pentatonic_minor"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "downtempo",
        name_zh: "缓拍",
        name_en: "Downtempo",
        default_bpm_range: (70, 110),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI", "I-vi-ii-V"],
        typical_scales: &["dorian", "natural_minor", "major"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "ambient",
        name_zh: "氛围",
        name_en: "Ambient",
        default_bpm_range: (50, 90),
        meter: (4, 4),
        typical_progressions: &["I", "i-VI"],
        typical_scales: &["lydian", "dorian", "pentatonic_major", "whole_tone"],
        swing: None,
        note_density_hint: (2, 16),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "drone",
        name_zh: "持续音",
        name_en: "Drone",
        default_bpm_range: (40, 76),
        meter: (4, 4),
        typical_progressions: &["I", "i"],
        typical_scales: &["dorian", "phrygian", "lydian"],
        swing: None,
        note_density_hint: (1, 8),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "new_age",
        name_zh: "新世纪",
        name_en: "New age",
        default_bpm_range: (56, 92),
        meter: (4, 4),
        typical_progressions: &["I-IV-I", "I-vi-IV"],
        typical_scales: &["major", "lydian", "pentatonic_major"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "synthwave",
        name_zh: "合成器浪潮",
        name_en: "Synthwave",
        default_bpm_range: (80, 118),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "i-VII-VI-VII"],
        typical_scales: &["natural_minor", "harmonic_minor", "aeolian"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "vaporwave",
        name_zh: "蒸汽波",
        name_en: "Vaporwave",
        default_bpm_range: (60, 90),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "ii-V-I"],
        typical_scales: &["major", "dorian", "lydian"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "lo_fi_hip_hop",
        name_zh: "低保真嘻哈",
        name_en: "Lo-fi hip hop",
        default_bpm_range: (70, 95),
        meter: (4, 4),
        typical_progressions: &["ii-V-I", "i-VI-III-VII", "I-vi-ii-V"],
        typical_scales: &["dorian", "major", "natural_minor"],
        swing: Some(56.0),
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "boom_bap",
        name_zh: "布姆巴普",
        name_en: "Boom bap",
        default_bpm_range: (85, 100),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv"],
        typical_scales: &["natural_minor", "dorian", "pentatonic_minor"],
        swing: Some(58.0),
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "trap",
        name_zh: "陷阱音乐",
        name_en: "Trap",
        default_bpm_range: (130, 150),
        meter: (4, 4),
        typical_progressions: &["i-VI", "i-VII-VI"],
        typical_scales: &["natural_minor", "harmonic_minor", "phrygian"],
        swing: None,
        note_density_hint: (16, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "drill",
        name_zh: "德里尔",
        name_en: "Drill",
        default_bpm_range: (138, 150),
        meter: (4, 4),
        typical_progressions: &["i-VI", "i-VII"],
        typical_scales: &["natural_minor", "harmonic_minor", "phrygian"],
        swing: None,
        note_density_hint: (16, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "grime",
        name_zh: "格兰姆",
        name_en: "Grime",
        default_bpm_range: (138, 142),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv"],
        typical_scales: &["natural_minor", "phrygian", "chromatic"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "chiptune",
        name_zh: "芯片音乐",
        name_en: "Chiptune",
        default_bpm_range: (110, 180),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "I-IV-V-I"],
        typical_scales: &["major", "natural_minor", "pentatonic_major"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "video_game_score",
        name_zh: "电子游戏配乐",
        name_en: "Video game score",
        default_bpm_range: (90, 176),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor", "lydian", "dorian"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    // ------------------------------------------------------------------
    // 第六节：流行与歌谣
    // 来源：20 世纪通行商业实践 + 传统歌谣形式。
    // ------------------------------------------------------------------
    GenreRule {
        id: "pop",
        name_zh: "流行",
        name_en: "Pop",
        default_bpm_range: (90, 130),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "I-vi-IV-V", "vi-IV-I-V"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (12, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "dance_pop",
        name_zh: "舞曲流行",
        name_en: "Dance pop",
        default_bpm_range: (110, 128),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "synth_pop",
        name_zh: "合成器流行",
        name_en: "Synth-pop",
        default_bpm_range: (100, 140),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor", "dorian"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "ballad",
        name_zh: "民谣抒情曲",
        name_en: "Ballad",
        default_bpm_range: (60, 88),
        meter: (4, 4),
        typical_progressions: &["I-vi-IV-V", "I-V-vi-IV", "ii-V-I"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (6, 24),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "power_ballad",
        name_zh: "力量抒情曲",
        name_en: "Power ballad",
        default_bpm_range: (64, 92),
        meter: (4, 4),
        typical_progressions: &["I-V-vi-IV", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "folk",
        name_zh: "民谣",
        name_en: "Folk",
        default_bpm_range: (80, 140),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major", "mixolydian", "dorian", "pentatonic_major"],
        swing: None,
        note_density_hint: (6, 24),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "americana",
        name_zh: "美式民谣",
        name_en: "Americana",
        default_bpm_range: (76, 132),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "mixolydian", "pentatonic_major"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "country",
        name_zh: "乡村",
        name_en: "Country",
        default_bpm_range: (80, 140),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-V-vi-IV", "I-vi-ii-V"],
        typical_scales: &["major", "pentatonic_major", "mixolydian"],
        swing: Some(56.0),
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "bluegrass",
        name_zh: "蓝草",
        name_en: "Bluegrass",
        default_bpm_range: (110, 180),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major", "pentatonic_major", "blues"],
        swing: None,
        note_density_hint: (32, 96),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "honky_tonk",
        name_zh: "酒馆乡村",
        name_en: "Honky tonk",
        default_bpm_range: (100, 150),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major", "blues", "pentatonic_major"],
        swing: Some(58.0),
        note_density_hint: (16, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "outlaw_country",
        name_zh: "亡命乡村",
        name_en: "Outlaw country",
        default_bpm_range: (84, 136),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "mixolydian"],
        swing: Some(56.0),
        note_density_hint: (8, 32),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "celtic",
        name_zh: "凯尔特",
        name_en: "Celtic",
        default_bpm_range: (90, 160),
        meter: (6, 8),
        typical_progressions: &["i-VII-VI-VII", "I-IV-I-V"],
        typical_scales: &["dorian", "mixolydian", "aeolian", "pentatonic_minor"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "irish_trad",
        name_zh: "爱尔兰传统",
        name_en: "Irish traditional",
        default_bpm_range: (100, 180),
        meter: (6, 8),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["dorian", "mixolydian", "ionian", "aeolian"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "jig",
        name_zh: "吉格舞曲",
        name_en: "Jig",
        default_bpm_range: (110, 160),
        meter: (6, 8),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["dorian", "mixolydian", "aeolian"],
        swing: None,
        note_density_hint: (24, 72),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "reel",
        name_zh: "里尔舞曲",
        name_en: "Reel",
        default_bpm_range: (130, 200),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["dorian", "mixolydian", "ionian"],
        swing: None,
        note_density_hint: (32, 96),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "scottish_trad",
        name_zh: "苏格兰传统",
        name_en: "Scottish traditional",
        default_bpm_range: (90, 160),
        meter: (4, 4),
        typical_progressions: &["I-IV-I-V", "i-VII-VI-VII"],
        typical_scales: &["mixolydian", "dorian", "pentatonic_major"],
        swing: None,
        note_density_hint: (24, 72),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "klezmer",
        name_zh: "克莱兹梅尔",
        name_en: "Klezmer",
        default_bpm_range: (90, 160),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-V", "i-iv-VII-III"],
        typical_scales: &["phrygian", "harmonic_minor", "dorian"],
        swing: Some(56.0),
        note_density_hint: (16, 64),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "balkan",
        name_zh: "巴尔干",
        name_en: "Balkan",
        default_bpm_range: (110, 180),
        meter: (7, 8),
        typical_progressions: &["i-VII-VI-VII", "i-iv-VII"],
        typical_scales: &["phrygian", "harmonic_minor", "dorian"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "polka",
        name_zh: "波尔卡",
        name_en: "Polka",
        default_bpm_range: (110, 150),
        meter: (2, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "chanson",
        name_zh: "法国香颂",
        name_en: "Chanson",
        default_bpm_range: (76, 140),
        meter: (3, 4),
        typical_progressions: &["i-VI-III-VII", "I-vi-ii-V"],
        typical_scales: &["major", "natural_minor", "harmonic_minor"],
        swing: Some(56.0),
        note_density_hint: (8, 32),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "fado",
        name_zh: "法多",
        name_en: "Fado",
        default_bpm_range: (60, 100),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "i-iv-V-i"],
        typical_scales: &["natural_minor", "harmonic_minor", "phrygian"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "cabaret",
        name_zh: "卡巴莱",
        name_en: "Cabaret",
        default_bpm_range: (88, 150),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "i-VI-III-VII"],
        typical_scales: &["major", "natural_minor"],
        swing: Some(58.0),
        note_density_hint: (12, 48),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "music_hall",
        name_zh: "杂耍剧场",
        name_en: "Music hall",
        default_bpm_range: (100, 160),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major"],
        swing: None,
        note_density_hint: (12, 40),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    // ------------------------------------------------------------------
    // 第七节：拉丁美洲与加勒比
    // 来源：地区性公共形式（SOURCE_COMMON_PRACTICE）+ 传统乐理。
    // ------------------------------------------------------------------
    GenreRule {
        id: "samba",
        name_zh: "桑巴",
        name_en: "Samba",
        default_bpm_range: (90, 130),
        meter: (2, 4),
        typical_progressions: &["I-vi-ii-V", "i-VII-VI-VII"],
        typical_scales: &["major", "dorian", "natural_minor"],
        swing: None,
        note_density_hint: (32, 96),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "bossa_nova_brazil",
        name_zh: "巴西巴萨诺瓦",
        name_en: "Brazilian bossa nova",
        default_bpm_range: (100, 136),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "i-VII-VI-VII"],
        typical_scales: &["major", "dorian", "melodic_minor"],
        swing: Some(54.0),
        note_density_hint: (16, 56),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "choro",
        name_zh: "肖罗",
        name_en: "Choro",
        default_bpm_range: (110, 160),
        meter: (2, 4),
        typical_progressions: &["I-vi-ii-V", "i-VII-VI-V"],
        typical_scales: &["major", "harmonic_minor", "dorian"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "tango",
        name_zh: "探戈",
        name_en: "Tango",
        default_bpm_range: (60, 120),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-V", "i-iv-V-i"],
        typical_scales: &["natural_minor", "harmonic_minor", "phrygian"],
        swing: None,
        note_density_hint: (12, 48),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "milonga",
        name_zh: "米隆加",
        name_en: "Milonga",
        default_bpm_range: (90, 130),
        meter: (2, 4),
        typical_progressions: &["i-VII-VI-V", "i-iv-V-i"],
        typical_scales: &["natural_minor", "harmonic_minor", "dorian"],
        swing: None,
        note_density_hint: (16, 48),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "bolero",
        name_zh: "波莱罗情歌",
        name_en: "Bolero",
        default_bpm_range: (60, 96),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "I-vi-ii-V"],
        typical_scales: &["natural_minor", "major"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "son_cubano",
        name_zh: "古巴颂",
        name_en: "Son cubano",
        default_bpm_range: (90, 140),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "i-VII-VI-VII"],
        typical_scales: &["major", "dorian", "natural_minor"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "salsa",
        name_zh: "萨尔萨",
        name_en: "Salsa",
        default_bpm_range: (150, 220),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["natural_minor", "major", "dorian"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "merengue",
        name_zh: "梅伦格",
        name_en: "Merengue",
        default_bpm_range: (120, 180),
        meter: (2, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (24, 80),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "bachata",
        name_zh: "巴恰塔",
        name_en: "Bachata",
        default_bpm_range: (110, 150),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["natural_minor", "major"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "cumbia",
        name_zh: "昆比亚",
        name_en: "Cumbia",
        default_bpm_range: (85, 120),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["natural_minor", "major", "pentatonic_minor"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "reggaeton",
        name_zh: "雷击顿",
        name_en: "Reggaeton",
        default_bpm_range: (88, 100),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "i-VII-VI"],
        typical_scales: &["natural_minor", "harmonic_minor"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "mariachi",
        name_zh: "玛利亚奇",
        name_en: "Mariachi",
        default_bpm_range: (90, 150),
        meter: (3, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (12, 48),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "ranchera",
        name_zh: "兰切拉",
        name_en: "Ranchera",
        default_bpm_range: (70, 130),
        meter: (3, 4),
        typical_progressions: &["I-IV-V-I", "i-VII-VI-VII"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "norteno",
        name_zh: "北方音乐",
        name_en: "Norteño",
        default_bpm_range: (100, 150),
        meter: (2, 4),
        typical_progressions: &["I-IV-V-I", "I-V-I"],
        typical_scales: &["major", "mixolydian"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "tejano",
        name_zh: "特哈诺",
        name_en: "Tejano",
        default_bpm_range: (100, 150),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "i-VII-VI-VII"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "flamenco",
        name_zh: "弗拉门戈",
        name_en: "Flamenco",
        default_bpm_range: (90, 220),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-V", "i-iv-V-i", "i-VI-VII-i"],
        typical_scales: &["phrygian", "harmonic_minor", "natural_minor"],
        swing: None,
        note_density_hint: (24, 128),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "sevillanas",
        name_zh: "塞维亚纳斯",
        name_en: "Sevillanas",
        default_bpm_range: (120, 180),
        meter: (3, 4),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["phrygian", "major", "natural_minor"],
        swing: None,
        note_density_hint: (16, 56),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "rumba_flamenca",
        name_zh: "弗拉门戈伦巴",
        name_en: "Rumba flamenca",
        default_bpm_range: (100, 160),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "i-iv-VII"],
        typical_scales: &["phrygian", "natural_minor", "harmonic_minor"],
        swing: None,
        note_density_hint: (24, 80),
        source: SOURCE_COMMON_PRACTICE,
    },
    // ------------------------------------------------------------------
    // 第八节：加勒比 / 非洲 / 中东 / 亚洲 / 世界
    // 来源：地区性公共形式 + 传统乐理（含教会调式与五声音阶的对应）。
    // ------------------------------------------------------------------
    GenreRule {
        id: "ska",
        name_zh: "斯卡",
        name_en: "Ska",
        default_bpm_range: (120, 180),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "natural_minor", "blues"],
        swing: None,
        note_density_hint: (24, 80),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "rocksteady",
        name_zh: "慢摇",
        name_en: "Rocksteady",
        default_bpm_range: (76, 110),
        meter: (4, 4),
        typical_progressions: &["I-vi-ii-V", "i-VII-VI-VII"],
        typical_scales: &["major", "natural_minor"],
        swing: None,
        note_density_hint: (12, 40),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "reggae",
        name_zh: "雷鬼",
        name_en: "Reggae",
        default_bpm_range: (60, 96),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "i-VII-VI-VII", "I-vi-ii-V"],
        typical_scales: &["major", "natural_minor", "pentatonic_minor"],
        swing: None,
        note_density_hint: (8, 32),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "dub",
        name_zh: "回响",
        name_en: "Dub",
        default_bpm_range: (60, 100),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-VI-III-VII"],
        typical_scales: &["natural_minor", "dorian", "pentatonic_minor"],
        swing: None,
        note_density_hint: (4, 24),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "dancehall",
        name_zh: "舞厅雷鬼",
        name_en: "Dancehall",
        default_bpm_range: (90, 120),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "i-VI"],
        typical_scales: &["natural_minor", "pentatonic_minor"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "afrobeat",
        name_zh: "非洲节拍",
        name_en: "Afrobeat",
        default_bpm_range: (95, 130),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv-VII"],
        typical_scales: &["dorian", "natural_minor", "pentatonic_minor"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "highlife",
        name_zh: "海莱夫",
        name_en: "Highlife",
        default_bpm_range: (100, 150),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "I-vi-ii-V"],
        typical_scales: &["major", "pentatonic_major"],
        swing: None,
        note_density_hint: (24, 80),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "soukous",
        name_zh: "苏库斯",
        name_en: "Soukous",
        default_bpm_range: (120, 180),
        meter: (4, 4),
        typical_progressions: &["I-IV-V-I", "i-VII-VI-VII"],
        typical_scales: &["major", "dorian", "pentatonic_major"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "amapiano",
        name_zh: "阿玛皮亚诺",
        name_en: "Amapiano",
        default_bpm_range: (108, 118),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI", "i-VI-III-VII"],
        typical_scales: &["natural_minor", "dorian"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_20C_COMMERCIAL_PRACTICE,
    },
    GenreRule {
        id: "afro_cuban",
        name_zh: "非洲古巴",
        name_en: "Afro-Cuban",
        default_bpm_range: (100, 180),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "I-IV-V-I"],
        typical_scales: &["natural_minor", "dorian", "major"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "raita",
        name_zh: "拉伊塔",
        name_en: "Gnawa",
        default_bpm_range: (90, 140),
        meter: (4, 4),
        typical_progressions: &["i-VII", "i-iv"],
        typical_scales: &["phrygian", "natural_minor", "pentatonic_minor"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "arabic_maqam",
        name_zh: "阿拉伯木卡姆",
        name_en: "Arabic maqam",
        default_bpm_range: (70, 160),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-V", "i-iv-V-i"],
        typical_scales: &["phrygian", "harmonic_minor", "dorian"],
        swing: None,
        note_density_hint: (12, 64),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "turkish_makam",
        name_zh: "土耳其马卡姆",
        name_en: "Turkish makam",
        default_bpm_range: (70, 160),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-V", "i-iv-V-i"],
        typical_scales: &["phrygian", "harmonic_minor", "dorian"],
        swing: None,
        note_density_hint: (12, 64),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "persian_dastgah",
        name_zh: "波斯达斯特加赫",
        name_en: "Persian dastgah",
        default_bpm_range: (60, 140),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-V", "i-iv-i"],
        typical_scales: &["phrygian", "harmonic_minor", "dorian"],
        swing: None,
        note_density_hint: (8, 48),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "hindustani",
        name_zh: "印度斯坦古典",
        name_en: "Hindustani classical",
        default_bpm_range: (50, 180),
        meter: (4, 4),
        typical_progressions: &["I", "i-VII-VI-VII"],
        typical_scales: &["dorian", "mixolydian", "aeolian", "phrygian"],
        swing: None,
        note_density_hint: (8, 96),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "carnatic",
        name_zh: "卡纳提克古典",
        name_en: "Carnatic classical",
        default_bpm_range: (60, 180),
        meter: (4, 4),
        typical_progressions: &["I", "i-VII-VI-VII"],
        typical_scales: &["dorian", "mixolydian", "aeolian"],
        swing: None,
        note_density_hint: (16, 96),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "bollywood",
        name_zh: "宝莱坞",
        name_en: "Bollywood",
        default_bpm_range: (80, 150),
        meter: (4, 4),
        typical_progressions: &["i-VI-III-VII", "I-IV-V-I"],
        typical_scales: &["harmonic_minor", "dorian", "major", "pentatonic_major"],
        swing: None,
        note_density_hint: (24, 96),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "bhangra",
        name_zh: "邦格拉",
        name_en: "Bhangra",
        default_bpm_range: (100, 160),
        meter: (4, 4),
        typical_progressions: &["i-VII", "I-IV-V-I"],
        typical_scales: &["mixolydian", "dorian", "pentatonic_major"],
        swing: None,
        note_density_hint: (32, 128),
        source: SOURCE_COMMON_PRACTICE,
    },
    GenreRule {
        id: "gamelan",
        name_zh: "甘美兰",
        name_en: "Gamelan",
        default_bpm_range: (50, 100),
        meter: (4, 4),
        typical_progressions: &["I", "I-IV"],
        typical_scales: &["pentatonic_major", "pentatonic_minor"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "pentatonic_east_asian",
        name_zh: "东亚五声",
        name_en: "East Asian pentatonic",
        default_bpm_range: (56, 120),
        meter: (4, 4),
        typical_progressions: &["I-IV-I", "I-V-I"],
        typical_scales: &["pentatonic_major", "pentatonic_minor"],
        swing: None,
        note_density_hint: (8, 40),
        source: SOURCE_TRADITIONAL_THEORY,
    },
    GenreRule {
        id: "andean",
        name_zh: "安第斯",
        name_en: "Andean",
        default_bpm_range: (80, 140),
        meter: (4, 4),
        typical_progressions: &["i-VII-VI-VII", "i-iv-VII"],
        typical_scales: &["natural_minor", "pentatonic_minor", "dorian"],
        swing: None,
        note_density_hint: (16, 64),
        source: SOURCE_TRADITIONAL_THEORY,
    },
];

/// 流派规则库（只读视图 + 确定性索引）。
///
/// 索引是 [`BTreeMap`]：`ids()` 与 `search()` 的顺序在任何进程、任何平台上
/// 都完全一致（[MODEL-AST-003] 的精神，红线 4）。
#[derive(Debug, Clone, Copy, Default)]
pub struct GenreLibrary;

/// 从未在文档里出现过的默认音阶名，用于 `primary_scale` 的零值回退。
fn library_index() -> &'static BTreeMap<&'static str, usize> {
    static INDEX: OnceLock<BTreeMap<&'static str, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = BTreeMap::new();
        for (position, rule) in GENRES.iter().enumerate() {
            // 重复 ID 会让 `get` 的结果取决于插入顺序；这里保持"首次登记者胜"，
            // 由 `ids_are_unique_and_stable` 测试负责拦住重复。
            map.entry(rule.id).or_insert(position);
        }
        map
    })
}

/// 预先小写化并拼好的搜索文本，与 [`GENRES`] 同下标。
///
/// 预先算一次，`search()` 就退化成纯字节子串查找，不再每次分配 `String`。
/// 这也是"同一关键词的搜索结果永远一致"的实现基础。
fn search_haystacks() -> &'static [String] {
    static HAYSTACKS: OnceLock<Vec<String>> = OnceLock::new();
    HAYSTACKS.get_or_init(|| {
        GENRES
            .iter()
            .map(|rule| {
                let mut haystack = String::with_capacity(160);
                haystack.push_str(&rule.id.to_ascii_lowercase());
                haystack.push(' ');
                haystack.push_str(&rule.name_en.to_ascii_lowercase());
                haystack.push(' ');
                haystack.push_str(&rule.name_zh.to_ascii_lowercase());
                haystack.push(' ');
                for scale in rule.typical_scales {
                    haystack.push_str(&scale.to_ascii_lowercase());
                    haystack.push(' ');
                }
                for progression in rule.typical_progressions {
                    haystack.push_str(&progression.to_ascii_lowercase());
                    haystack.push(' ');
                }
                haystack
            })
            .collect()
    })
}

/// ASCII 子串查找（对 UTF-8 输入安全：needle 是合法 UTF-8，匹配边界必然落在字符边界）。
fn contains_ascii(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

impl GenreLibrary {
    /// 全部流派（按登记顺序）。
    #[must_use]
    pub const fn all() -> &'static [GenreRule] {
        GENRES
    }

    /// 当前登记的流派条数。
    #[must_use]
    pub const fn len() -> usize {
        GENRES.len()
    }

    /// 规则库是否为空。
    #[must_use]
    pub const fn is_empty() -> bool {
        GENRES.is_empty()
    }

    /// 按 ID 取流派。
    ///
    /// # Errors
    ///
    /// ID 不存在时返回 [`TheoryError::GenreNotFound`]（对应 MCP 的
    /// `STYLE_NOT_FOUND`）。
    pub fn get(id: &str) -> Result<&'static GenreRule, TheoryError> {
        let index = library_index()
            .get(id)
            .copied()
            .ok_or(TheoryError::GenreNotFound)?;
        GENRES.get(index).ok_or(TheoryError::GenreNotFound)
    }

    /// 按 ID 取流派，`Option` 版本（不想处理 `Result` 时用）。
    #[must_use]
    pub fn try_get(id: &str) -> Option<&'static GenreRule> {
        library_index().get(id).and_then(|&index| GENRES.get(index))
    }

    /// 全部 ID，按字典序升序（确定性）。
    #[must_use]
    pub fn ids() -> Vec<&'static str> {
        library_index().keys().copied().collect()
    }

    /// 关键词搜索：匹配 ID、中英文名、典型音阶名与走向文本。
    ///
    /// 匹配是**大小写不敏感的 ASCII 子串匹配**；空关键词返回全部流派。
    /// 结果按 ID 字典序升序，因此完全确定。
    #[must_use]
    pub fn search(keyword: &str) -> Vec<&'static GenreRule> {
        let needle = keyword.trim().to_ascii_lowercase();
        let haystacks = search_haystacks();
        Self::ids()
            .into_iter()
            .filter_map(Self::try_get)
            .filter(|rule| {
                library_index()
                    .get(rule.id)
                    .and_then(|&position| haystacks.get(position))
                    .is_some_and(|haystack| contains_ascii(haystack, &needle))
            })
            .collect()
    }

    /// 按典型音阶筛选流派。
    #[must_use]
    pub fn by_scale(scale: &str) -> Vec<&'static GenreRule> {
        Self::all()
            .iter()
            .filter(|rule| rule.typical_scales.contains(&scale))
            .collect()
    }

    /// 按来源标记筛选流派（用于许可审计）。
    #[must_use]
    pub fn by_source(source: &str) -> Vec<&'static GenreRule> {
        Self::all()
            .iter()
            .filter(|rule| rule.source == source)
            .collect()
    }

    /// 全部来源标记及其条数（用于 notes 文档与审计）。
    #[must_use]
    pub fn source_histogram() -> BTreeMap<&'static str, usize> {
        let mut histogram = BTreeMap::new();
        for rule in Self::all() {
            *histogram.entry(rule.source).or_insert(0usize) += 1;
        }
        histogram
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::PitchClass;

    #[test]
    fn the_library_is_not_empty() {
        assert!(GenreLibrary::len() > 0);
        assert!(!GenreLibrary::is_empty());
    }

    #[test]
    fn library_size_is_pinned_to_the_measured_number() {
        // 这个数字就是 notes 文档里登记的"当前条数"。改动本表必须同步改这里
        // 与 `docs/ledger/theory-core-notes.md`。
        assert_eq!(
            GenreLibrary::len(),
            182,
            "GENRES 条数变了：请同步 docs/ledger/theory-core-notes.md 与缺口清单"
        );
    }

    #[test]
    fn ids_are_unique_and_stable() {
        let ids = GenreLibrary::ids();
        assert_eq!(ids.len(), GenreLibrary::len());
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "duplicate genre id");
        // ids() 必须是升序的（BTreeMap 保证）
        assert_eq!(ids, sorted);
    }

    #[test]
    fn every_rule_has_a_plausible_bpm_range_meter_and_names() {
        for rule in GenreLibrary::all() {
            assert!(rule.has_valid_bpm_range(), "{}: bad bpm", rule.id);
            assert!(
                rule.default_bpm_range.1 <= 400,
                "{}: bpm upper bound too high",
                rule.id
            );
            assert!(
                rule.meter.0 > 0 && rule.meter.1.is_power_of_two(),
                "{}: bad meter",
                rule.id
            );
            assert!(!rule.name_en.is_empty(), "{}: empty english name", rule.id);
            assert!(!rule.name_zh.is_empty(), "{}: empty chinese name", rule.id);
            assert!(
                !rule.typical_progressions.is_empty(),
                "{}: no progressions",
                rule.id
            );
            assert!(!rule.typical_scales.is_empty(), "{}: no scales", rule.id);
            assert!(
                rule.note_density_hint.0 > 0
                    && rule.note_density_hint.0 <= rule.note_density_hint.1,
                "{}: bad density hint",
                rule.id
            );
            assert!(!rule.source.is_empty(), "{}: no source", rule.id);
            if let Some(swing) = rule.swing {
                assert!(
                    (50.0..=80.0).contains(&swing),
                    "{}: swing ratio {swing} outside 50..=80",
                    rule.id
                );
            }
        }
    }

    #[test]
    fn id_is_lowercase_snake_case_ascii() {
        for rule in GenreLibrary::all() {
            assert!(
                rule.id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{}: id is not lowercase snake_case ascii",
                rule.id
            );
        }
    }

    #[test]
    fn idiomatic_data_is_structurally_valid() {
        // 每条规则的走向与音阶都必须能被本 crate 自己的解析器吃下，
        // 否则 MCP 的 `yeban_propose_section` 会在运行时才发现数据是坏的。
        for rule in GenreLibrary::all() {
            for scale_name in rule.typical_scales {
                assert!(
                    ScaleKind::parse(scale_name).is_ok(),
                    "{}: unknown scale `{scale_name}`",
                    rule.id
                );
            }
            for text in rule.typical_progressions {
                let progression = Progression::parse(text)
                    .unwrap_or_else(|err| panic!("{}: `{text}` failed to parse: {err}", rule.id));
                assert!(!progression.degrees().is_empty(), "{}", rule.id);
            }
        }
    }

    #[test]
    fn every_rule_can_produce_a_chord_sketch() {
        // 端到端：从流派规则直接产出 4 小节和弦骨架。
        for rule in GenreLibrary::all() {
            let spans = rule.sketch(PitchClass::C, 4).unwrap_or_else(|err| {
                panic!("{}: sketch failed: {err}", rule.id);
            });
            let total: u64 = spans.iter().map(|s| s.duration_ticks).sum();
            assert!(!spans.is_empty(), "{}", rule.id);
            // 4 小节至少覆盖 4 小节的时长（级数多于小节时会更长）。
            assert!(
                total >= 4 * rule.meter_value().ticks_per_bar(),
                "{}: total {total} shorter than 4 bars",
                rule.id
            );
            // 区段必须首尾相接、严格升序。
            let mut cursor = 0u64;
            for span in &spans {
                assert_eq!(span.start_tick, cursor, "{}", rule.id);
                assert!(span.duration_ticks > 0, "{}", rule.id);
                cursor = span.end_tick();
            }
        }
    }

    #[test]
    fn get_and_ids_agree() {
        for id in GenreLibrary::ids() {
            let rule = GenreLibrary::get(id).unwrap();
            assert_eq!(rule.id, id);
        }
        assert_eq!(
            GenreLibrary::get("no_such_genre").unwrap_err(),
            TheoryError::GenreNotFound
        );
        assert!(GenreLibrary::try_get("no_such_genre").is_none());
    }

    #[test]
    fn search_matches_id_english_chinese_and_scale() {
        assert!(
            GenreLibrary::search("bossa")
                .iter()
                .any(|rule| rule.id == "bossa_nova")
        );
        assert!(
            GenreLibrary::search("Bossa Nova")
                .iter()
                .any(|rule| rule.id == "bossa_nova")
        );
        assert!(
            GenreLibrary::search("探戈")
                .iter()
                .any(|rule| rule.id == "tango"),
            "chinese keyword must match"
        );
        assert!(
            GenreLibrary::search("blues")
                .iter()
                .any(|rule| rule.id == "blues")
        );
        assert!(GenreLibrary::search("no_such_keyword_xyz").is_empty());
        assert_eq!(GenreLibrary::search("   ").len(), GenreLibrary::len());
        // 搜索顺序必须与 ids() 的字典序一致
        let found: Vec<&str> = GenreLibrary::search("rock").iter().map(|r| r.id).collect();
        let mut expected = found.clone();
        expected.sort_unstable();
        assert_eq!(found, expected);
    }

    #[test]
    fn source_histogram_covers_every_rule() {
        let histogram = GenreLibrary::source_histogram();
        let total: usize = histogram.values().sum();
        assert_eq!(total, GenreLibrary::len());
        // 计数必须和按来源筛选一致
        for (source, count) in &histogram {
            assert_eq!(GenreLibrary::by_source(source).len(), *count, "{source}");
        }
        assert!(histogram.contains_key(SOURCE_TRADITIONAL_THEORY));
    }

    #[test]
    fn by_scale_filters_within_the_library() {
        let blues = GenreLibrary::by_scale("blues");
        assert!(!blues.is_empty());
        assert!(
            blues
                .iter()
                .all(|rule| rule.typical_scales.contains(&"blues"))
        );
        assert!(GenreLibrary::by_scale("no_such_scale").is_empty());
    }

    #[test]
    fn library_is_deterministic_across_calls() {
        assert_eq!(GenreLibrary::ids(), GenreLibrary::ids());
        assert_eq!(GenreLibrary::search("jazz"), GenreLibrary::search("jazz"));
    }
}
