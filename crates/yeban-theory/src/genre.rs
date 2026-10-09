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
use crate::drum::DrumStyle;
use crate::error::TheoryError;
use crate::progression::{ChordSpan, Meter, Progression};
use crate::scale::{Scale, ScaleKind};

/// 来源标记：传统音乐理论（公有领域）。
pub const SOURCE_TRADITIONAL_THEORY: &str = "traditional-theory/public-domain";
/// 来源标记：该流派的通行实践（地区性公共形式，无个人著作权）。
pub const SOURCE_COMMON_PRACTICE: &str = "common-practice/public-domain";
/// 来源标记：20 世纪通行商业实践（形式本身不受版权保护；仅登记 BPM/拍号等事实）。
pub const SOURCE_20C_COMMERCIAL_PRACTICE: &str = "20c-commercial-practice-facts";
/// 来源标记：本 crate 依据传统乐理自行编码（夜半原创）。
pub const SOURCE_YEBAN_ORIGINAL: &str = "yeban-original-encoding";

/// 走向选择动作的领域分隔盐（ASCII `PROGRESS`）。
///
/// 见 [`GenreRule::progression_for`]：它把"选哪条走向"与"选哪个音阶"、
/// 以及本 crate 其它用 [`crate::derive_index`] 的动作（旋律的音高选择）
/// 从种子上分开，免得同一个种子在每个动作里都落到同一个相对位置。
const GENRE_PROGRESSION_SALT: u64 = 0x5052_4F47_5245_5353;

/// 音阶选择动作的领域分隔盐（ASCII `SCALE___`）。
const GENRE_SCALE_SALT: u64 = 0x5343_414C_455F_5F5F;

/// FNV-1a 64 位哈希（公有领域算法），只用来把流派 ID 折成一个盐。
///
/// 目的：两条**条数相同**的流派不该在同一个种子下同时选中同一个下标
/// （否则"换流派"会静默退化成"换名字"）。纯整数、`const fn`、
/// 无分配、无全局状态。
const fn genre_id_salt(id: &str) -> u64 {
    let bytes = id.as_bytes();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut index = 0usize;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

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
    /// 底鼓落点口径（[`crate::drum::DrumStyle`]）：`pending 3` 里
    /// "逐流派的鼓点型数据"**已登记的那一部分**。
    ///
    /// 登记口径（只有满足它才登记非缺省值，否则一律
    /// [`crate::drum::DrumStyle::Metric`]）：
    ///
    /// - [`crate::drum::DrumStyle::FourOnTheFloor`]：**每拍一击的底鼓**是该流派
    ///   不加修饰的通行默认（disco / house / techno / trance 一类舞曲）；
    /// - [`crate::drum::DrumStyle::Metric`]：其余全部流派**不替它们猜**一个
    ///   口径，行为与登记之前**逐位相同**（这也是缺省值的定义）。
    ///
    /// ⚠ 本字段只登记**底鼓落点**这一维。军鼓位置仍由调用方给的
    /// `backbeat`（[`crate::drum::Backbeat`]）决定，踩镲仍铺满网格，
    /// onset 数仍由调用方给出：`pending 3` 的数据那一半因此是**部分**关闭。
    pub drum_style: crate::drum::DrumStyle,
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
            && self.drum_style == other.drum_style
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
    /// `bars` 的单位是**该流派自己的小节**，不是 4/4 的小节：展开前把
    /// [`GenreRule::meter_value`]（即 [`GenreRule::meter`]）灌进走向，
    /// 因此 `waltz.sketch(C, 4)` 得到 `4 * 2880 = 11520` tick，
    /// 而不是 `4 * 3840 = 15360` tick。`meter_value()` 若不被消费，
    /// 规则数据与实际时长就会静默不一致。
    ///
    /// # Errors
    ///
    /// 没有典型走向、走向或音阶无法解析、`bars` 为 0 时返回相应错误。
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
        Progression::parse(progression)?
            .with_meter(self.meter_value())
            .with_bars(bars)?
            .expand(&scale)
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

    /// 该流派登记的典型走向条数。
    ///
    /// 单位是"条"。登记的每一条都能被 [`GenreRule::sketch_at`] /
    /// [`GenreRule::chords_at`] 展开（判据 `every_registered_progression_expands`
    /// 对全部条目生效）。
    #[must_use]
    pub const fn progression_count(&self) -> usize {
        self.typical_progressions.len()
    }

    /// 第 `index` 条登记走向的原文；越界返回 `None`。
    ///
    /// 与 [`crate::rhythm::cells_per_bar`] / [`crate::rhythm::BeatGrouping::new`]
    /// 同口径：**不新增** [`TheoryError`] 变体（那是 `yeban-mcp` 也在消费的跨
    /// crate 契约），越界这种"调用方索引错"用 `Option` 表达，不 panic、不截断。
    #[must_use]
    pub fn progression_at(&self, index: usize) -> Option<&'static str> {
        self.typical_progressions.get(index).copied()
    }

    /// 该流派登记的典型音阶条数。
    ///
    /// 单位是"个"。与 [`GenreRule::primary_scale`] 只取第 0 个不同，
    /// [`GenreRule::scale_at`] 能取到全部登记音阶。
    #[must_use]
    pub const fn scale_count(&self) -> usize {
        self.typical_scales.len()
    }

    /// 第 `index` 个登记音阶名的原文；越界返回 `None`。
    #[must_use]
    pub fn scale_name_at(&self, index: usize) -> Option<&'static str> {
        self.typical_scales.get(index).copied()
    }

    /// 第 `index` 个登记音阶（主音为 `tonic`）。
    ///
    /// # Errors
    ///
    /// `index` 越界（该位置没有登记音阶）或该位置的音阶名无法识别时返回
    /// [`TheoryError::ScaleNameUnknown`] —— 两者同口径："这个位置没有可用的
    /// 音阶名"。与 [`GenreRule::primary_scale`] 的解析路径是同一条。
    pub fn scale_at(
        &self,
        tonic: crate::pitch::PitchClass,
        index: usize,
    ) -> Result<Scale, TheoryError> {
        let name = self
            .typical_scales
            .get(index)
            .ok_or(TheoryError::ScaleNameUnknown)?;
        Ok(Scale::new(tonic, ScaleKind::parse(name)?))
    }

    /// 按种子确定性地选一条登记走向。
    ///
    /// 选择动作用 [`crate::derive_index`]（SplitMix64，纯 64 位整数），
    /// 盐 = [`GENRE_PROGRESSION_SALT`] 与**该流派自己的 ID 哈希**异或 ⇒
    /// 不同流派的同一条走向不会因为"条数相同"而被同一批种子同时选中。
    /// 同种子同输出、跨进程跨平台逐位一致 [ARCH-DET-001]；没有浮点、
    /// 没有全局可变状态。
    ///
    /// `seed` 的语义是"这一版编排的种子"：调用方换种子就能在**已登记的**走向
    /// 里换一条，不必自己写索引循环。种子空间对每个流派都能取到全部登记条目
    /// （判据 `seeds_cover_every_registered_progression_and_scale`）。
    ///
    /// # Errors
    ///
    /// 该流派没有登记任何走向时返回 [`TheoryError::EmptyProgression`]。
    pub fn progression_for(&self, seed: u64) -> Result<&'static str, TheoryError> {
        if self.typical_progressions.is_empty() {
            return Err(TheoryError::EmptyProgression);
        }
        let index = crate::derive_index(
            seed,
            GENRE_PROGRESSION_SALT ^ genre_id_salt(self.id),
            self.typical_progressions.len(),
        );
        Ok(self.typical_progressions[index])
    }

    /// 按种子确定性地选一个登记音阶。
    ///
    /// 与 [`GenreRule::progression_for`] 同口径，但用**另一个盐**
    /// （[`GENRE_SCALE_SALT`]）⇒ 走向与音阶是两次互相独立的抽取，
    /// 能组合出 `走向条数 × 音阶个数` 种编排。
    ///
    /// # Errors
    ///
    /// 该流派没有登记任何音阶、或选中的音阶名无法识别时返回
    /// [`TheoryError::ScaleNameUnknown`]。
    pub fn scale_for(
        &self,
        tonic: crate::pitch::PitchClass,
        seed: u64,
    ) -> Result<Scale, TheoryError> {
        // 这里**故意没有**"登记表为空"的显式守卫：`derive_index(.., 0)` 返回 0，
        // 空表上 `scale_at(tonic, 0)` 就是 `ScaleNameUnknown`，与显式守卫逐位同果。
        // 实测：删掉那三行守卫，全部判据仍然全绿 ⇒ 不可判的分支不留
        // （与本线在 `metric_weight_in` 上删掉 `beats == 0` 的处置同口径）。
        let index = crate::derive_index(
            seed,
            GENRE_SCALE_SALT ^ genre_id_salt(self.id),
            self.typical_scales.len(),
        );
        self.scale_at(tonic, index)
    }

    /// 用第 `index` 条登记走向生成小节骨架（[`GenreRule::sketch`] 的显式索引版）。
    ///
    /// 音阶取 [`GenreRule::primary_scale`]（与 [`GenreRule::sketch`] 同一条），
    /// 拍号取该流派自己的 [`GenreRule::meter_value`]，`bars` 的单位也是该流派的
    /// 小节。因此 `sketch_at(_, 4, 0)` 与 `sketch(_, 4)` **逐位相同**。
    ///
    /// # Errors
    ///
    /// `index` 越界以 [`TheoryError::EmptyProgression`] 表达（"这个位置没有走向"，
    /// 与"一条都没登记"同口径）；其余错误见 [`GenreRule::sketch`]。
    pub fn sketch_at(
        &self,
        tonic: crate::pitch::PitchClass,
        bars: u32,
        index: usize,
    ) -> Result<Vec<ChordSpan>, TheoryError> {
        let progression = self
            .progression_at(index)
            .ok_or(TheoryError::EmptyProgression)?;
        let scale = self.primary_scale(tonic)?;
        Progression::parse(progression)?
            .with_meter(self.meter_value())
            .with_bars(bars)?
            .expand(&scale)
    }

    /// 用第 `index` 条登记走向的和弦序列（[`GenreRule::chords`] 的显式索引版）。
    ///
    /// # Errors
    ///
    /// `index` 越界以 [`TheoryError::EmptyProgression`] 表达；
    /// 其余错误见 [`GenreRule::chords`]。
    pub fn chords_at(
        &self,
        tonic: crate::pitch::PitchClass,
        index: usize,
    ) -> Result<Vec<Chord>, TheoryError> {
        let progression = self
            .progression_at(index)
            .ok_or(TheoryError::EmptyProgression)?;
        let scale = self.primary_scale(tonic)?;
        Progression::parse(progression)?
            .with_bars(4)
            .map(|p| p.chords(&scale))
    }

    /// 按种子选走向**与音阶**，再生成小节骨架（一次调用拿到一版编排）。
    ///
    /// 与 [`GenreRule::sketch`] 的唯一差别是"读哪一条登记数据"：
    /// 走向来自 [`GenreRule::progression_for`]，音阶来自 [`GenreRule::scale_for`]，
    /// 两者都由同一个 `seed` 决定。拍号、`bars` 口径、
    /// [`crate::progression::Progression::expand`] 的三级时长规则**完全不变**。
    ///
    /// 因此：`seed` 若恰好选中第 0 条走向与第 0 个音阶，本函数与
    /// [`GenreRule::sketch`] **逐位相同**；换种子只会在**已登记的**数据里换，
    /// 不发明任何新走向、新音阶 [ARCH-DET-001]。
    ///
    /// 每个和弦的根音恒属于**本次选中的**音阶：展开用的音阶就是选中的那一个。
    ///
    /// # Errors
    ///
    /// 与 [`GenreRule::sketch`] 相同（错误顺序也是"先走向、后音阶"：
    /// 没有登记走向 ⇒ [`TheoryError::EmptyProgression`]，
    /// 没有登记音阶或音阶名无法识别 ⇒ [`TheoryError::ScaleNameUnknown`]）。
    pub fn sketch_for(
        &self,
        tonic: crate::pitch::PitchClass,
        bars: u32,
        seed: u64,
    ) -> Result<Vec<ChordSpan>, TheoryError> {
        let progression = self.progression_for(seed)?;
        let scale = self.scale_for(tonic, seed)?;
        Progression::parse(progression)?
            .with_meter(self.meter_value())
            .with_bars(bars)?
            .expand(&scale)
    }

    /// 按种子选走向与音阶，返回和弦序列（[`GenreRule::chords`] 的种子版）。
    ///
    /// # Errors
    ///
    /// 与 [`GenreRule::chords`] 相同，顺序同 [`GenreRule::sketch_for`]。
    pub fn chords_for(
        &self,
        tonic: crate::pitch::PitchClass,
        seed: u64,
    ) -> Result<Vec<Chord>, TheoryError> {
        let progression = self.progression_for(seed)?;
        let scale = self.scale_for(tonic, seed)?;
        Progression::parse(progression)?
            .with_bars(4)
            .map(|p| p.chords(&scale))
    }

    /// 把 [`GenreRule::swing`]（百分数 `f32`）折成 [`crate::swing`] 的千分比整数。
    ///
    /// - `swing == None` ⇒ `Ok(None)`：该流派不适用摇摆，调用方保持平直网格。
    /// - `swing == Some(p)` ⇒ `Ok(Some(permille))`，`permille = round(p * 10)`。
    ///
    /// 浮点只出现在这一步：`f32` 是**登记数据**（通行实践的近似值），
    /// 一旦进入 [`crate::swing`]，全部判定都是整数运算 [ARCH-DET-001]。
    /// 取整走 `libm`（与 [`crate::pitch::note_to_hz`] 同一口径），
    /// 避免落到各平台的标准库实现。越界值钳制到区间端点，再交
    /// [`crate::swing::validate_swing_permille`] 校验（防御性分支）。
    ///
    /// # Errors
    ///
    /// 钳制后的千分比仍越界时返回 [`TheoryError::SwingOutOfRange`]。
    pub fn swing_permille(&self) -> Result<Option<u16>, TheoryError> {
        let Some(percent) = self.swing else {
            return Ok(None);
        };
        let clamped = libm::roundf(percent * 10.0).clamp(
            f32::from(crate::swing::SWING_PERMILLE_STRAIGHT),
            f32::from(crate::swing::SWING_PERMILLE_MAX),
        );
        let permille = clamped as u16;
        crate::swing::validate_swing_permille(permille)?;
        Ok(Some(permille))
    }

    /// 用该流派**登记的**拍号与摇摆比例生成节奏网格（每小节 `onsets_per_bar` 个 onset）。
    ///
    /// 这是 `pending 3` 所说"具体的鼓点网格"的**网格**那一半：拍号来自
    /// [`GenreRule::meter_value`]，摇摆比例来自 [`GenreRule::swing_permille`]，
    /// onset 的选取与推移见 [`crate::rhythm`]。
    ///
    /// onset 数**不**读 [`GenreRule::note_density_hint`]：那个字段登记的是
    /// **每小节的音符数**（含和弦内的复音，最大区间到 160），而 4/4 一小节只有
    /// 16 个 16 分格位，两者不是同一个量。理由见 [`crate::rhythm`] 的模块文档。
    ///
    /// # Errors
    ///
    /// 见 [`crate::rhythm::swung_metric_grid`]：
    /// `bars == 0` 或拍号非法 ⇒ [`TheoryError::ZeroBars`]；
    /// `onsets_per_bar` 超过小节内的格位数 ⇒ [`TheoryError::ProgressionTooDense`]；
    /// 登记的摇摆比例折算后越界 ⇒ [`TheoryError::SwingOutOfRange`]。
    pub fn rhythm_grid(
        &self,
        bars: u32,
        onsets_per_bar: u32,
    ) -> Result<crate::rhythm::MetricGrid, TheoryError> {
        crate::rhythm::swung_metric_grid(
            self.meter_value(),
            bars,
            onsets_per_bar,
            self.swing_permille()?,
        )
    }

    /// 用该流派**登记的**拍号、摇摆比例与底鼓口径生成鼓组型
    /// （每小节 `onsets_per_bar` 个 onset）。
    ///
    /// 这是 `pending 3` 所说"具体的鼓点"那一半的**机制**侧加**已登记的数据**：
    /// 网格来自本流派的拍号与摇摆比例（见 [`GenreRule::rhythm_grid`]），底鼓落点
    /// 来自本流派的 [`GenreRule::drum_style`]，分派规则见 [`crate::drum`]。
    /// 拍分组仍走**拍号自身**的度量层级，反拍位置仍走
    /// [`crate::drum::default_backbeat`]（两者都只读拍号）—— 这两维**没有**登记
    /// 流派数据，见 [`GenreRule::drum_style`] 的登记口径。
    ///
    /// onset 数**不**读 [`GenreRule::note_density_hint`]（理由同
    /// [`GenreRule::rhythm_grid`]）。返回 `Ok(None)` 的两种情形：
    /// `onsets_per_bar` 为 0（网格没有 onset，鼓组型为空是合法的，因此不会走到这里）
    /// 不会发生；实际只有"该拍号每小节不足一拍"这一种（病态拍号，见
    /// [`crate::drum::swung_drum_pattern`]）。
    ///
    /// # Errors
    ///
    /// 见 [`crate::rhythm::swung_metric_grid`]：
    /// `bars == 0` 或拍号非法 ⇒ [`TheoryError::ZeroBars`]；
    /// `onsets_per_bar` 超过小节内的格位数 ⇒ [`TheoryError::ProgressionTooDense`]；
    /// 登记的摇摆比例折算后越界 ⇒ [`TheoryError::SwingOutOfRange`]。
    pub fn drum_pattern(
        &self,
        bars: u32,
        onsets_per_bar: u32,
    ) -> Result<Option<crate::drum::DrumPattern>, TheoryError> {
        crate::drum::swung_styled_drum_pattern(
            self.meter_value(),
            bars,
            onsets_per_bar,
            self.swing_permille()?,
            None,
            crate::drum::default_backbeat(self.meter_value()),
            self.drum_style,
        )
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::FourOnTheFloor,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
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
        drum_style: DrumStyle::Metric,
        source: SOURCE_TRADITIONAL_THEORY,
    },
];

/// 流派规则库（只读视图 + 确定性索引）。
///
/// 索引是 [`BTreeMap`]：`ids()` 与 `search()` 的顺序在任何进程、任何平台上
/// 都完全一致（[MODEL-AST-003] 的精神，红线 4）。
#[derive(Debug, Clone, Copy, Default)]
pub struct GenreLibrary;

/// ID → `GENRES` 下标的只读索引（[`BTreeMap`]，因此 `ids()` 是字典序）。
///
/// 由 `OnceLock` 惰性构建一次，只读常量表，不含可变状态或随机性。
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

    /// 按底鼓口径筛选流派（用于审计"哪些流派登记了四踩底鼓"）。
    #[must_use]
    pub fn by_drum_style(style: DrumStyle) -> Vec<&'static GenreRule> {
        Self::all()
            .iter()
            .filter(|rule| rule.drum_style == style)
            .collect()
    }

    /// 全部底鼓口径及其条数（用于 notes 文档与审计）。
    ///
    /// 键是 [`DrumStyle::name`]，与 [`GenreLibrary::by_drum_style`] 的计数一致
    /// （判据 `drum_style_histogram_covers_every_rule`）。
    #[must_use]
    pub fn drum_style_histogram() -> BTreeMap<&'static str, usize> {
        let mut histogram = BTreeMap::new();
        for rule in Self::all() {
            *histogram.entry(rule.drum_style.name()).or_insert(0usize) += 1;
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
            // 4 小节**恰好**覆盖该流派自己拍号的 4 小节（相等，不是"至少"：
            // 原先的 `>=` 会放过"3/4 的曲子按 4/4 计时"这种静默不一致）。
            assert_eq!(
                total,
                4 * rule.meter_value().ticks_per_bar(),
                "{}: total {total} is not 4 bars of {}/{}",
                rule.id,
                rule.meter.0,
                rule.meter.1
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
    fn every_rule_sketch_uses_the_rules_own_meter() {
        // `GenreRule.meter` 是该流派自己的拍号（waltz 3/4、march 2/4、
        // jig 6/8 之类）。`sketch(tonic, bars)` 产出的**小节**必须是**这个**
        // 拍号的小节；否则 `meter_value()` 这个公开取值口没有任何消费者，
        // 规则数据与实际时长静默不一致。
        let mut non_common = 0usize;
        for rule in GenreLibrary::all() {
            let meter = rule.meter_value();
            let bars = 4u64;
            let spans = rule
                .sketch(PitchClass::C, bars as u32)
                .unwrap_or_else(|err| panic!("{}: sketch failed: {err}", rule.id));
            let total: u64 = spans.iter().map(|span| span.duration_ticks).sum();
            assert_eq!(
                total,
                bars * meter.ticks_per_bar(),
                "{}: {bars} bars of {}/{} must total {} ticks, got {total}",
                rule.id,
                meter.numerator,
                meter.denominator,
                bars * meter.ticks_per_bar()
            );
            // 级数不多于小节时，每小节恰好一个和弦，且每段正好一小节。
            let degrees = Progression::parse(rule.typical_progressions[0])
                .expect("idiomatic_data_is_structurally_valid pins this")
                .degrees()
                .len();
            if degrees <= bars as usize {
                assert_eq!(spans.len(), bars as usize, "{}", rule.id);
                for span in &spans {
                    assert_eq!(span.duration_ticks, meter.ticks_per_bar(), "{}", rule.id);
                }
            }
            if meter != Meter::COMMON {
                non_common += 1;
            }
        }
        // 防真空：若不是 4/4 的规则一条都不剩，这条判据就测不到任何东西。
        assert_eq!(
            non_common, 28,
            "非 4/4 拍号的规则条数变了（实测 182 条中 28 条:3/4=13, 2/4=8, 7/8=4, 6/8=3）"
        );
    }

    #[test]
    fn waltz_sketch_is_in_three_four_not_four_four() {
        // 定点读数：这条判据用**字面数字**，避免上面那条循环判据被改成
        // "两边取自同一个来源"而不自知。圆舞曲 = 3/4 ⇒ 一小节
        // 960 * 4 * 3 / 4 = 2880 tick。
        let waltz = GenreLibrary::get("waltz").unwrap();
        assert_eq!(waltz.meter, (3, 4));
        let spans = waltz.sketch(PitchClass::C, 4).unwrap();
        assert_eq!(spans.len(), 4);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.duration_ticks)
                .collect::<Vec<_>>(),
            vec![2880, 2880, 2880, 2880]
        );
        assert_eq!(
            spans.iter().map(|span| span.duration_ticks).sum::<u64>(),
            11520
        );
        assert_eq!(spans[3].start_tick, 8640);
        // 走向 "I-V-I" 铺 4 小节 ⇒ C G C C（级数按小节循环取用）。
        assert_eq!(
            spans
                .iter()
                .map(|span| span.chord.symbol())
                .collect::<Vec<_>>(),
            vec!["C", "G", "C", "C"]
        );
        // 反例锚点：同样的 4 小节若按 4/4 计会是 4 * 3840 = 15360 tick。
        assert_eq!(Meter::COMMON.ticks_per_bar(), 3840);
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

    /// 一个字段齐全、走向与音阶都是空表的流派，用来验证"没有登记数据"的路径。
    fn empty_rule() -> GenreRule {
        GenreRule {
            id: "empty_probe",
            name_zh: "空",
            name_en: "Empty",
            default_bpm_range: (100, 120),
            meter: (4, 4),
            typical_progressions: &[],
            typical_scales: &[],
            swing: None,
            note_density_hint: (1, 1),
            drum_style: DrumStyle::Metric,
            source: SOURCE_YEBAN_ORIGINAL,
        }
    }

    #[test]
    fn registered_progression_and_scale_counts_are_pinned() {
        // 数什么：把 182 条流派的 `typical_progressions` / `typical_scales`
        // 逐条相加。单位 = "条"（走向）与"个"（音阶）。
        let (mut progressions, mut scales) = (0usize, 0usize);
        let mut progression_histogram = [0usize; 8];
        let mut scale_histogram = [0usize; 8];
        for rule in GenreLibrary::all() {
            progressions += rule.progression_count();
            scales += rule.scale_count();
            progression_histogram[rule.progression_count()] += 1;
            scale_histogram[rule.scale_count()] += 1;
            assert!(rule.progression_count() >= 2, "{}", rule.id);
            assert!(rule.scale_count() >= 1, "{}", rule.id);
        }
        assert_eq!(progressions, 391, "registered progression texts");
        assert_eq!(scales, 520, "registered scale names");
        // 直方图下标 = 每条的条数，值 = 有多少条流派。
        assert_eq!(progression_histogram, [0, 0, 155, 27, 0, 0, 0, 0]);
        assert_eq!(scale_histogram, [0, 5, 40, 113, 24, 0, 0, 0]);
    }

    #[test]
    fn the_bounded_accessors_answer_every_registered_index_and_nothing_beyond() {
        for rule in GenreLibrary::all() {
            let last = rule.progression_count() - 1;
            assert_eq!(rule.progression_at(0), Some(rule.typical_progressions[0]));
            assert_eq!(
                rule.progression_at(last),
                Some(rule.typical_progressions[last])
            );
            assert_eq!(rule.progression_at(rule.progression_count()), None);
            assert_eq!(rule.progression_at(usize::MAX), None);

            let last = rule.scale_count() - 1;
            assert_eq!(rule.scale_name_at(0), Some(rule.typical_scales[0]));
            assert_eq!(rule.scale_name_at(last), Some(rule.typical_scales[last]));
            assert_eq!(rule.scale_name_at(rule.scale_count()), None);
            assert_eq!(rule.scale_name_at(usize::MAX), None);
        }
    }

    #[test]
    fn scale_at_parses_every_registered_name_and_reports_bounds_with_the_existing_error() {
        for rule in GenreLibrary::all() {
            for index in 0..rule.scale_count() {
                let scale = rule.scale_at(PitchClass::C, index).unwrap();
                let registered = rule.scale_name_at(index).unwrap();
                assert_eq!(scale.tonic, PitchClass::C);
                // 语义判据：种子/索引两条路都走 `ScaleKind::parse`，与
                // `primary_scale` 同一条解析路径。
                assert_eq!(
                    scale.kind,
                    ScaleKind::parse(registered).unwrap(),
                    "{}",
                    rule.id
                );
                assert_eq!(
                    scale.intervals(),
                    ScaleKind::parse(registered).unwrap().intervals()
                );
            }
            assert_eq!(
                rule.scale_at(PitchClass::C, rule.scale_count()),
                Err(TheoryError::ScaleNameUnknown),
                "{}",
                rule.id
            );
        }
    }

    #[test]
    fn registered_scale_names_fold_onto_canonical_kinds_except_two_documented_aliases() {
        // 实测（不是假设）：登记表里出现两个"别名"名 `ionian` 与 `aeolian`，
        // 而 `ScaleKind::parse` 把它们折到 `Major` / `NaturalMinor`。
        // 后果：`parse(name).name()` 的往返对这两个名字**不成立**，
        // 且 5 条流派的音阶表里因此出现了一对**同一个 kind**。
        // 本条判据把这件事钉住（`scale_count` 仍然如实回报**登记条数**，
        // 不偷偷去重）。
        let mut genres_with_an_alias = 0usize;
        let mut duplicate_kind_entries = 0usize;
        for rule in GenreLibrary::all() {
            let mut kinds: Vec<&str> = Vec::new();
            let mut aliased = false;
            for index in 0..rule.scale_count() {
                let registered = rule.scale_name_at(index).unwrap();
                let kind = rule.scale_at(PitchClass::C, index).unwrap().kind;
                if kind.name() != registered {
                    aliased = true;
                }
                if kinds.contains(&kind.name()) {
                    duplicate_kind_entries += 1;
                } else {
                    kinds.push(kind.name());
                }
            }
            if aliased {
                genres_with_an_alias += 1;
            }
        }
        assert_eq!(genres_with_an_alias, 13);
        assert_eq!(duplicate_kind_entries, 5);
    }

    #[test]
    fn seeds_cover_every_registered_progression_and_every_distinct_scale_kind() {
        // 数什么：对每条流派，种子 0..64 里能取到多少个**不同的**登记条目。
        // 单位 = 条（走向）/ 个（音阶，按 kind 去重后的口径）。
        let (mut distinct_kinds, mut duplicate_kind_entries) = (0usize, 0usize);
        for rule in GenreLibrary::all() {
            let mut registered_kinds: Vec<&str> = Vec::new();
            for index in 0..rule.scale_count() {
                let name = rule.scale_at(PitchClass::C, index).unwrap().kind.name();
                if registered_kinds.contains(&name) {
                    duplicate_kind_entries += 1;
                } else {
                    registered_kinds.push(name);
                }
            }
            distinct_kinds += registered_kinds.len();

            let mut progression_seen: Vec<&str> = Vec::new();
            let mut kind_seen: Vec<&str> = Vec::new();
            for seed in 0u64..64 {
                let text = rule.progression_for(seed).unwrap();
                if !progression_seen.contains(&text) {
                    progression_seen.push(text);
                }
                let kind = rule.scale_for(PitchClass::C, seed).unwrap().kind.name();
                if !kind_seen.contains(&kind) {
                    kind_seen.push(kind);
                }
            }
            assert_eq!(
                progression_seen.len(),
                rule.progression_count(),
                "{}: seeds 0..64 do not cover every registered progression",
                rule.id
            );
            assert_eq!(
                kind_seen.len(),
                registered_kinds.len(),
                "{}: seeds 0..64 do not cover every distinct registered scale",
                rule.id
            );
        }
        assert_eq!(distinct_kinds, 515);
        assert_eq!(duplicate_kind_entries, 5);
    }

    #[test]
    fn sketch_at_zero_and_a_zero_picking_seed_match_sketch_bit_for_bit() {
        // 旧 API 的行为是**不变契约**：显式索引 0 与"种子恰好选中第 0 条"
        // 都必须与 `sketch` 逐位相同。
        for rule in GenreLibrary::all() {
            let baseline = rule.sketch(PitchClass::C, 4).unwrap();
            assert_eq!(
                rule.sketch_at(PitchClass::C, 4, 0).unwrap(),
                baseline,
                "{}",
                rule.id
            );
            assert_eq!(
                rule.chords_at(PitchClass::C, 0).unwrap(),
                rule.chords(PitchClass::C).unwrap()
            );
            for seed in 0u64..32 {
                let zero_progression =
                    rule.progression_for(seed).unwrap() == rule.progression_at(0).unwrap();
                let zero_scale = rule.scale_for(PitchClass::C, seed).unwrap().kind
                    == rule.primary_scale(PitchClass::C).unwrap().kind;
                if zero_progression && zero_scale {
                    assert_eq!(
                        rule.sketch_for(PitchClass::C, 4, seed).unwrap(),
                        baseline,
                        "{} seed {seed}",
                        rule.id
                    );
                }
            }
        }
    }

    #[test]
    fn every_seeded_sketch_keeps_the_sketch_invariants_and_stays_in_its_own_key() {
        for rule in GenreLibrary::all() {
            let bars = 4u32;
            let meter = rule.meter_value();
            for seed in 0u64..8 {
                let spans = rule.sketch_for(PitchClass::C, bars, seed).unwrap();
                let total: u64 = spans.iter().map(|span| span.duration_ticks).sum();
                assert_eq!(
                    total,
                    u64::from(bars) * meter.ticks_per_bar(),
                    "{}",
                    rule.id
                );
                let key = rule.scale_for(PitchClass::C, seed).unwrap();
                let mut cursor = 0u64;
                for span in &spans {
                    assert_eq!(span.start_tick, cursor, "{}", rule.id);
                    assert!(span.duration_ticks > 0, "{}", rule.id);
                    assert_eq!(span.duration_ticks % 240, 0, "{}", rule.id);
                    assert!(key.contains(span.chord.root), "{} seed {seed}", rule.id);
                    cursor = span.end_tick();
                }
            }
        }
    }

    #[test]
    fn the_seeded_selector_is_pinned_to_literal_readings() {
        // 字面读数（写死，不引用常量）：waltz 有 3 条走向 / 2 个音阶，
        // funk 有 3 条走向 / 4 个音阶。
        let waltz = GenreLibrary::get("waltz").unwrap();
        assert_eq!(waltz.progression_count(), 3);
        assert_eq!(waltz.scale_count(), 2);
        assert_eq!(waltz.progression_for(0).unwrap(), "i-VI-III-VII");
        assert_eq!(waltz.progression_for(1).unwrap(), "I-V-I");
        assert_eq!(waltz.progression_for(4).unwrap(), "I-IV-V-I");
        assert_eq!(
            waltz.scale_for(PitchClass::C, 1).unwrap().kind.name(),
            "natural_minor"
        );
        assert_eq!(
            waltz.scale_for(PitchClass::C, 2).unwrap().kind.name(),
            "major"
        );

        // funk 的第 2 条走向 `I7-IV7` 含七和弦，旧的 `sketch` **永远取不到**它
        // （只读第 0 条 `i-VII`）；种子 0 就能取到 ⇒ 登记数据真的可达了。
        let funk = GenreLibrary::get("funk").unwrap();
        assert_eq!(funk.progression_at(0).unwrap(), "i-VII");
        assert_eq!(funk.progression_for(0).unwrap(), "I7-IV7");
        let chords = funk.chords_for(PitchClass::C, 0).unwrap();
        assert_eq!(
            chords.first().unwrap().kind,
            crate::chord::ChordKind::Dominant7
        );
        assert_eq!(
            funk.chords(PitchClass::C).unwrap().first().unwrap().kind,
            crate::chord::ChordKind::Minor
        );
    }

    #[test]
    fn the_per_genre_salt_decorrelates_two_genres_with_identical_data() {
        // 两条流派的登记数据逐字节相同、只有 ID 不同：选择必须仍然分开，
        // 否则"换流派"会静默退化成"换名字"。
        let progressions: &[&str] = &["I-V-I", "I-IV-V-I", "i-VI-III-VII"];
        let scales: &[&str] = &["major", "natural_minor", "dorian"];
        let make = |id: &'static str| GenreRule {
            id,
            name_zh: "探针",
            name_en: "Probe",
            default_bpm_range: (100, 120),
            meter: (4, 4),
            typical_progressions: progressions,
            typical_scales: scales,
            swing: None,
            note_density_hint: (4, 8),
            drum_style: DrumStyle::Metric,
            source: SOURCE_YEBAN_ORIGINAL,
        };
        let (alpha, beta) = (make("alpha_salt_probe"), make("beta_salt_probe"));
        let (mut progression_differs, mut scale_differs) = (0usize, 0usize);
        for seed in 0u64..64 {
            if alpha.progression_for(seed).unwrap() != beta.progression_for(seed).unwrap() {
                progression_differs += 1;
            }
            if alpha.scale_for(PitchClass::C, seed).unwrap().kind
                != beta.scale_for(PitchClass::C, seed).unwrap().kind
            {
                scale_differs += 1;
            }
        }
        // 实测读数：64 个种子里走向差 42 次、音阶差 46 次（写死）。
        assert_eq!(progression_differs, 42);
        assert_eq!(scale_differs, 46);
    }

    #[test]
    fn a_genre_with_no_registered_data_reports_instead_of_panicking() {
        let empty = empty_rule();
        assert_eq!(empty.progression_count(), 0);
        assert_eq!(empty.progression_at(0), None);
        assert_eq!(empty.scale_count(), 0);
        assert_eq!(empty.scale_name_at(0), None);
        assert_eq!(empty.progression_for(0), Err(TheoryError::EmptyProgression));
        assert_eq!(
            empty.scale_for(PitchClass::C, 0),
            Err(TheoryError::ScaleNameUnknown)
        );
        assert_eq!(
            empty.scale_at(PitchClass::C, 0),
            Err(TheoryError::ScaleNameUnknown)
        );
        assert_eq!(
            empty.sketch_at(PitchClass::C, 4, 0),
            Err(TheoryError::EmptyProgression)
        );
        assert_eq!(
            empty.chords_at(PitchClass::C, 0),
            Err(TheoryError::EmptyProgression)
        );
        assert_eq!(
            empty.sketch_for(PitchClass::C, 4, 0),
            Err(TheoryError::EmptyProgression)
        );
        assert_eq!(
            empty.chords_for(PitchClass::C, 0),
            Err(TheoryError::EmptyProgression)
        );
        // 旧的入口行为未变（回归护栏）。
        assert_eq!(
            empty.primary_scale(PitchClass::C),
            Err(TheoryError::ScaleNameUnknown)
        );
        assert_eq!(
            empty.sketch(PitchClass::C, 4),
            Err(TheoryError::EmptyProgression)
        );
        assert_eq!(
            empty.chords(PitchClass::C),
            Err(TheoryError::EmptyProgression)
        );
    }

    #[test]
    fn drum_style_histogram_covers_every_rule() {
        // 数什么：182 条流派的 `drum_style` 字段逐条计数，单位 = "条"。
        // 登记读数：169 条 metric（= 旧口径）+ 13 条 four-on-the-floor。
        let histogram = GenreLibrary::drum_style_histogram();
        let total: usize = histogram.values().sum();
        assert_eq!(total, GenreLibrary::len());
        assert_eq!(histogram.len(), DrumStyle::ALL.len());
        assert_eq!(histogram.get("metric"), Some(&169));
        assert_eq!(histogram.get("four-on-the-floor"), Some(&13));
        for style in DrumStyle::ALL {
            assert_eq!(
                GenreLibrary::by_drum_style(style).len(),
                histogram[style.name()],
                "{style:?}"
            );
        }
        // 登记了四踩底鼓的流派必须都是 4/4：换一个拍号，"每拍一击"就要求
        // 另一套读数（这条判据把登记错误变成红行，而不是静默给出别的拍数）。
        for rule in GenreLibrary::by_drum_style(DrumStyle::FourOnTheFloor) {
            assert_eq!(rule.meter, (4, 4), "{}", rule.id);
        }
    }

    #[test]
    fn the_four_on_the_floor_registration_is_pinned_to_the_measured_ids() {
        // 数什么：登记了四踩底鼓的流派 **ID**（单位 = "条"），按字典序写死。
        // 条数判据抓不到"把两条流派的登记互换"（例如把 house 改回 metric、
        // 把 gregorian_chant 改成 four-on-the-floor：13/169 不变、4/4 也成立）。
        // 这条判据把 ID 本身钉住，互换因此变红。
        let mut ids: Vec<&str> = GenreLibrary::by_drum_style(DrumStyle::FourOnTheFloor)
            .iter()
            .map(|rule| rule.id)
            .collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            [
                "amapiano",
                "boogie",
                "deep_house",
                "disco",
                "garage_house",
                "hardstyle",
                "house",
                "minimal_techno",
                "progressive_house",
                "psytrance",
                "tech_house",
                "techno",
                "trance",
            ]
        );
    }

    #[test]
    fn registered_four_on_the_floor_genres_get_a_kick_on_every_beat() {
        // 4/4 下每拍一击：只要网格至少含 4 个 onset（每拍一个），
        // 四踩底鼓恒是 4 个；度量口径恒是 2 个（第 0、2 拍这两个组起点）。
        let cells = crate::rhythm::cells_per_bar(Meter::COMMON).unwrap() as u32;
        let registered = GenreLibrary::by_drum_style(DrumStyle::FourOnTheFloor);
        assert_eq!(registered.len(), 13);
        for rule in registered {
            for onsets in [4u32, 8, cells] {
                let pattern = rule.drum_pattern(1, onsets).unwrap().unwrap();
                assert_eq!(
                    pattern.hit_count(crate::drum::DrumVoice::Kick),
                    4,
                    "{} onsets {onsets}",
                    rule.id
                );
                let metric = crate::drum::styled_drum_pattern(
                    rule.meter_value(),
                    1,
                    onsets,
                    None,
                    crate::drum::default_backbeat(rule.meter_value()),
                    DrumStyle::Metric,
                )
                .unwrap()
                .unwrap();
                assert_eq!(
                    metric.hit_count(crate::drum::DrumVoice::Kick),
                    2,
                    "{} onsets {onsets}",
                    rule.id
                );
            }
        }
    }

    #[test]
    fn unregistered_genres_stay_bit_identical_to_the_legacy_drum_entry_point() {
        // 未登记四踩底鼓的流派：`GenreRule::drum_pattern` 必须与**旧入口**
        // （不知道 `drum_style` 的那一个）逐位相同。
        // 数什么：比较过的 (流派, onset 数) 组合个数，单位 = "个"。
        let mut compared = 0usize;
        for rule in GenreLibrary::by_drum_style(DrumStyle::Metric) {
            for onsets in [1u32, 2, 3, 4, 8] {
                let Ok(Some(pattern)) = rule.drum_pattern(2, onsets) else {
                    continue;
                };
                let legacy = crate::drum::swung_drum_pattern(
                    rule.meter_value(),
                    2,
                    onsets,
                    rule.swing_permille().unwrap(),
                    None,
                    crate::drum::default_backbeat(rule.meter_value()),
                )
                .unwrap()
                .unwrap();
                assert!(
                    pattern.hits() == legacy.hits(),
                    "{} onsets {onsets}",
                    rule.id
                );
                assert!(
                    pattern.grid().hits() == legacy.grid().hits(),
                    "{} onsets {onsets}",
                    rule.id
                );
                compared += 1;
            }
        }
        // 169 条 metric 流派 × 5 个 onset 数 = 845 个组合（全部构造成功：
        // 请求的最大 onset 数 8 不超过任何登记拍号的格位数）。
        assert_eq!(compared, 845);
    }
}
