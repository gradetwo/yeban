//! 旋律生成 (melody line)：把**和声骨架 + 节奏网格**落成一条确定性的单声部旋律。
//!
//! ## 为什么需要这个模块
//!
//! `docs/ledger/theory-core-notes.md` 的 `pending 4` 记着：本 crate
//! "只产出和声与声部连接，没有 `melody()`"。消费者一侧也把这件事留在本工作线：
//! `crates/yeban-mcp/src/domain/section_build.rs:51` 写着"真正的'写旋律'在
//! `line/theory-core` 一侧"（同一句话见 `docs/ledger/theory-wiring-notes.md:99`）。
//! 本模块补上这条线：给定音阶、和声区段、节奏网格与一个种子，
//! 产出**每个 onset 一个音**的单声部旋律。
//!
//! ## 本模块**不**做什么（边界声明）
//!
//! 1. **不新增任何登记数据**。全部音高都来自既有的 [`Scale`] 与 [`ChordSpan`]，
//!    时值来自既有的 [`MetricGrid`]。本模块没有流派表、没有旋律型表。
//! 2. **不做"好听"的判定**。旋律型 (motif)、句法、终止式一律不做：那需要
//!    登记数据或人类审美裁决，本 crate 无权发明。
//! 3. **不产出和弦外的音**（见下面第 3 条语义）：音高集合是
//!    `音阶音级 ∩ 窗口`（强拍再与和弦构成音相交）⇒ 换一个音阶就换一条旋律。
//!
//! ## 语义（全部是整数运算）
//!
//! 1. **时值**：旋律的 onset 序列**就是**输入网格的 onset 序列（一格一音，
//!    不增不减、不量化）。第 `i` 个音的时长 = 第 `i + 1` 个 onset 的 tick − 本
//!    onset 的 tick；最后一个音接到 `bars × ticks_per_bar` 为止。因此
//!    `Σ duration_ticks == grid.total_ticks()`、相邻音首尾相接、无重叠无空隙。
//! 2. **音高窗口**：候选音高恒在 [`MelodyConstraints::lower`]`..=`
//!    [`MelodyConstraints::upper`] 之内，且其音级必须属于 [`Scale`]。
//! 3. **强拍优先取和弦音**：onset 的度量重量 ≥ [`CHORD_TONE_WEIGHT_FLOOR`] 时，
//!    候选集先取"属于音阶的**和弦构成音**"这一档。这一档里没有**可行**候选
//!    （窗口里没有和弦音，或它们的跳进都超界）时，退回"属于音阶的音"这一档。
//!    退路是**显式登记**的：`MelodyNote::chord_tone` 如实回报该音到底是不是
//!    该 onset 所在和声区段的构成音，因此调用方能看到退路发生过。
//! 4. **跳进上界**：相邻两音的半音距离恒 ≤ [`MelodyConstraints::max_leap`]
//!    （默认 12，与 [`crate::voice_leading`] 的单声部硬约束同口径）。
//!    只要窗口里有**一个**音阶音，这条上界就总能满足（最坏情况是同音反复）。
//!    窗口里一个音阶音都没有时返回 [`TheoryError::NoFeasibleVoicing`]，
//!    **不**静默放宽上界、也不产出窗口外的音。
//! 5. **确定性 [ARCH-DET-001]**：唯一的选择动作用
//!    [`crate::derive_index`]（SplitMix64，纯 64 位整数）完成，
//!    种子是调用方入参。同输入同输出、跨进程跨平台逐位一致；
//!    本模块没有浮点、没有时钟、没有全局可变状态。
//!
//! ## 边界（音频红线）
//!
//! - **构造期分配，逐样本零分配**：构造函数为 `bars × onsets_per_bar` 个音
//!   分配一个 `Vec`，因此**不得**在音频线程上调用。读侧（[`Melody::notes`]、
//!   [`Melody::notes_in_bar`]）只返回切片：不分配、不加锁、不做阻塞 I/O、不打日志。
//! - 候选集的枚举用**栈上定长数组**（`[u8; 128]` = MIDI 音高全域），因此逐 onset
//!   不产生任何堆分配；堆分配只发生在结果 `Vec` 上。
//! - 本模块不含 `HashMap` / `HashSet` [MODEL-AST-003 / AGENTS.md 红线 4]。

use crate::derive_index;
use crate::error::TheoryError;
use crate::genre::GenreRule;
use crate::pitch::PitchClass;
use crate::progression::{ChordSpan, Meter};
use crate::rhythm::MetricGrid;
use crate::scale::Scale;

/// 默认音域下界：`C3`（MIDI 48）。
pub const MELODY_LOWER_BOUND: u8 = 48;

/// 默认音域上界：`C6`（MIDI 84）。
pub const MELODY_UPPER_BOUND: u8 = 84;

/// 默认跳进上界：12 个半音（与 [`crate::voice_leading`] 的单声部硬约束同值）。
pub const MELODY_MAX_LEAP: u8 = 12;

/// 强拍的度量重量门槛：重量 ≥ 本值的 onset 优先取和弦构成音。
///
/// 取 1 的依据：[`crate::rhythm::metric_weight_in`] 里重量 ≥ 1 的格点，
/// 在**每拍格位数为偶数**的拍号上恰好是**偶数格点**（4/4 的八分音符位置
/// 及更强；6/8 的六个八分位置），重量 0 是十六分反拍位置。
/// 因此"强拍取和弦音、弱位可取音阶任意音"这条口径落到 4/4 上就是
/// "八分及以上的位置取和弦音"。
pub const CHORD_TONE_WEIGHT_FLOOR: u8 = 1;

/// 音高选择动作的领域分隔盐（ASCII `MELODY`）。
///
/// 每个 onset 用 `MELODY_PITCH_SALT + index` 作为盐，因此不同 onset 的选择
/// 互相独立（SplitMix64 就是为"种子 + 递增计数器"设计的）。
const MELODY_PITCH_SALT: u64 = 0x4D45_4C4F_4459;

/// 旋律的音域与跳进约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MelodyConstraints {
    /// 允许的最低 MIDI 音高（含）。
    pub lower: u8,
    /// 允许的最高 MIDI 音高（含）。
    pub upper: u8,
    /// 相邻两音允许的最大半音距离；`0` = 只允许同音反复。
    pub max_leap: u8,
}

impl MelodyConstraints {
    /// 默认约束：[`MELODY_LOWER_BOUND`]`..=`[`MELODY_UPPER_BOUND`]、
    /// 跳进上界 [`MELODY_MAX_LEAP`]。
    pub const DEFAULT: Self = Self {
        lower: MELODY_LOWER_BOUND,
        upper: MELODY_UPPER_BOUND,
        max_leap: MELODY_MAX_LEAP,
    };

    /// 构造并校验一组约束。
    ///
    /// # Errors
    ///
    /// 见 [`MelodyConstraints::validate`]。
    pub const fn new(lower: u8, upper: u8, max_leap: u8) -> Result<Self, TheoryError> {
        if lower > upper {
            return Err(TheoryError::VoiceRangeInvalid { lower, upper });
        }
        if upper > 127 {
            return Err(TheoryError::VoiceRangeOutOfMidi);
        }
        Ok(Self {
            lower,
            upper,
            max_leap,
        })
    }

    /// 校验音域区间：`lower > upper` ⇒ [`TheoryError::VoiceRangeInvalid`]；
    /// `upper > 127` ⇒ [`TheoryError::VoiceRangeOutOfMidi`]。
    ///
    /// `max_leap` 不在这里校验：`0` 是合法语义（同音反复），
    /// 不可行的组合由 [`melody_over_chords`] 返回 [`TheoryError::NoFeasibleVoicing`]。
    ///
    /// # Errors
    ///
    /// 同上两条。
    pub const fn validate(self) -> Result<(), TheoryError> {
        if self.lower > self.upper {
            return Err(TheoryError::VoiceRangeInvalid {
                lower: self.lower,
                upper: self.upper,
            });
        }
        if self.upper > 127 {
            return Err(TheoryError::VoiceRangeOutOfMidi);
        }
        Ok(())
    }
}

/// 旋律里的一个音。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MelodyNote {
    /// 起始 tick（等于对应网格 onset 的 tick）。
    pub start_tick: u64,
    /// 持续 tick 数（恒为正；= 下一个 onset 的 tick − 本 onset 的 tick）。
    pub duration_ticks: u64,
    /// MIDI 音高 (0..=127)。
    pub pitch: u8,
    /// 所在小节（0 基）。
    pub bar: u32,
    /// 小节内的格点下标（0 基，与 [`crate::rhythm::GridHit::cell`] 同值）。
    pub cell: u32,
    /// 该 onset 的度量重量（与 [`crate::rhythm::GridHit::weight`] 同值）。
    pub weight: u8,
    /// 该音是否同时是所在和声区段的构成音（且属于本音阶）。
    pub chord_tone: bool,
}

impl MelodyNote {
    /// 结束 tick（开区间）。
    #[must_use]
    pub const fn end_tick(&self) -> u64 {
        self.start_tick + self.duration_ticks
    }
}

/// 一条单声部旋律线。
///
/// 由 [`melody_over_chords`] / [`genre_melody`] / [`genre_melody_with`] 构造。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Melody {
    key: Scale,
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
    seed: u64,
    constraints: MelodyConstraints,
    notes: Vec<MelodyNote>,
}

impl Melody {
    /// 全部音，按 `start_tick` 严格升序。
    #[must_use]
    pub fn notes(&self) -> &[MelodyNote] {
        &self.notes
    }

    /// 音数（恒等于网格的 onset 数）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// 音数为 0（即调用方请求了 `onsets_per_bar == 0`）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// 旋律所用的音阶。
    #[must_use]
    pub const fn key(&self) -> Scale {
        self.key
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

    /// 每小节的音数（= 网格的 onset 数）。
    #[must_use]
    pub const fn onsets_per_bar(&self) -> u32 {
        self.onsets_per_bar
    }

    /// 应用在网格上的摇摆千分比；`None` = 平直。
    #[must_use]
    pub const fn swing_permille(&self) -> Option<u16> {
        self.permille
    }

    /// 生成时使用的种子。
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// 生成时使用的约束。
    #[must_use]
    pub const fn constraints(&self) -> MelodyConstraints {
        self.constraints
    }

    /// 覆盖的总 tick 数（恒等于 `bars × ticks_per_bar`）。
    #[must_use]
    pub const fn total_ticks(&self) -> u64 {
        self.meter.ticks_per_bar() * self.bars as u64
    }

    /// 第 `bar` 小节的音切片；`bar >= bars` 时返回空切片。
    ///
    /// 不分配、不加锁、不做 I/O。
    #[must_use]
    pub fn notes_in_bar(&self, bar: u32) -> &[MelodyNote] {
        if bar >= self.bars {
            return &[];
        }
        let width = self.onsets_per_bar as usize;
        let start = bar as usize * width;
        &self.notes[start..start + width]
    }
}

/// 在既有和声骨架上、按既有网格生成一条确定性旋律。
///
/// 音高的选择规则见模块文档：候选 = `[lower, upper]` 内属于 `key` 的音级。
/// onset 的度量重量 ≥ [`CHORD_TONE_WEIGHT_FLOOR`] 时先试"和弦构成音 ∩ 音阶"
/// 那一档；该档没有满足跳进上界的候选时退回音阶档（`chord_tone` 如实回报）。
/// 选择动作用 [`derive_index`] 在**可行候选**（满足跳进上界的那些）中确定性地取一个。
///
/// # Errors
///
/// - 约束非法（`lower > upper`、`upper > 127`）⇒ [`TheoryError::VoiceRangeInvalid`] /
///   [`TheoryError::VoiceRangeOutOfMidi`]；
/// - `spans` 为空 ⇒ [`TheoryError::EmptyProgression`]（"没有和声骨架"是输入错误）；
/// - 窗口 `[lower, upper]` 里没有任何满足跳进上界的音阶音 ⇒
///   [`TheoryError::NoFeasibleVoicing`]（第一个 onset 就会报）。
pub fn melody_over_chords(
    key: &Scale,
    spans: &[ChordSpan],
    grid: &MetricGrid,
    constraints: MelodyConstraints,
    seed: u64,
) -> Result<Melody, TheoryError> {
    constraints.validate()?;
    if spans.is_empty() {
        return Err(TheoryError::EmptyProgression);
    }

    // 音阶音级掩码：12 位布尔表（不是 HashSet）。
    let mut scale_mask = [false; 12];
    for pc in key.pitch_classes() {
        scale_mask[pc.semitones() as usize] = true;
    }

    let total_ticks = grid.total_ticks();
    let hits = grid.hits();
    let mut notes = Vec::with_capacity(hits.len());
    let mut previous: Option<u8> = None;

    for (index, hit) in hits.iter().enumerate() {
        // 该 onset 所在的和声区段：按切片顺序取**第一个**包含它的区段。
        // 用线性查找而不是游标，是为了对任意顺序的 `spans` 都安全（不 panic、不跳过）。
        let chord_mask = spans
            .iter()
            .find(|span| span.start_tick <= hit.tick && hit.tick < span.end_tick())
            .map_or([false; 12], |span| {
                let mut mask = [false; 12];
                for pc in span.chord.pitch_classes() {
                    let slot = pc.semitones() as usize;
                    // 只在音阶内的和弦音进候选，因此"每个音都属于音阶"是硬不变量。
                    if scale_mask[slot] {
                        mask[slot] = true;
                    }
                }
                mask
            });

        // 候选枚举在栈上完成（MIDI 全域 128 个音高），逐 onset 零堆分配。
        // 两档：音阶档（恒有）+ 和弦档（强拍上属于和弦构成音的那些）。
        let mut scale_candidates = [0u8; 128];
        let mut scale_count = 0usize;
        let mut chord_candidates = [0u8; 128];
        let mut chord_count = 0usize;
        for pitch in constraints.lower..=constraints.upper {
            let slot = (pitch % 12) as usize;
            if !scale_mask[slot] {
                continue;
            }
            scale_candidates[scale_count] = pitch;
            scale_count += 1;
            if hit.weight >= CHORD_TONE_WEIGHT_FLOOR && chord_mask[slot] {
                chord_candidates[chord_count] = pitch;
                chord_count += 1;
            }
        }

        let salt = MELODY_PITCH_SALT.wrapping_add(index as u64);
        // 优先级：强拍的和弦档 → 音阶档。两档都过不了跳进上界才报错。
        let pitch = pick_within_leap(
            &chord_candidates[..chord_count],
            previous,
            constraints.max_leap,
            seed,
            salt,
        )
        .or_else(|| {
            pick_within_leap(
                &scale_candidates[..scale_count],
                previous,
                constraints.max_leap,
                seed,
                salt,
            )
        })
        .ok_or(TheoryError::NoFeasibleVoicing)?;
        previous = Some(pitch);
        notes.push(MelodyNote {
            start_tick: hit.tick,
            duration_ticks: hits.get(index + 1).map_or(total_ticks, |next| next.tick) - hit.tick,
            pitch,
            bar: hit.bar,
            cell: hit.cell,
            weight: hit.weight,
            chord_tone: chord_mask[(pitch % 12) as usize],
        });
    }

    Ok(Melody {
        key: *key,
        meter: grid.meter(),
        bars: grid.bars(),
        onsets_per_bar: grid.onsets_per_bar(),
        permille: grid.swing_permille(),
        seed,
        constraints,
        notes,
    })
}

/// 在候选里过滤出满足跳进上界的那些，再用 [`derive_index`] 确定性地取一个。
///
/// 返回 `None` 表示一个可行候选都没有（调用方据此换下一档或报错）。
/// 过滤缓冲在栈上（MIDI 全域 128 个音高），不产生堆分配。
fn pick_within_leap(
    candidates: &[u8],
    previous: Option<u8>,
    max_leap: u8,
    seed: u64,
    salt: u64,
) -> Option<u8> {
    let mut feasible = [0u8; 128];
    let mut count = 0usize;
    for &pitch in candidates {
        if previous.is_some_and(|last| pitch.abs_diff(last) > max_leap) {
            continue;
        }
        feasible[count] = pitch;
        count += 1;
    }
    if count == 0 {
        return None;
    }
    Some(feasible[derive_index(seed, salt, count)])
}

/// 用该流派**登记的**音阶、走向、拍号与摇摆比例生成一条旋律。
///
/// 等价于 `genre_melody_with(genre, tonic, bars, onsets_per_bar,
/// MelodyConstraints::DEFAULT, seed)`。
///
/// # Errors
///
/// 见 [`genre_melody_with`]。
pub fn genre_melody(
    genre: &GenreRule,
    tonic: PitchClass,
    bars: u32,
    onsets_per_bar: u32,
    seed: u64,
) -> Result<Melody, TheoryError> {
    genre_melody_with(
        genre,
        tonic,
        bars,
        onsets_per_bar,
        MelodyConstraints::DEFAULT,
        seed,
    )
}

/// 同 [`genre_melody`]，但由调用方给出音域与跳进约束。
///
/// 和声骨架来自 [`GenreRule::sketch`]（该流派第一条典型走向 + 它自己的拍号），
/// onset 网格来自 [`GenreRule::rhythm_grid`]（该流派的拍号与摇摆比例），
/// 音阶来自 [`GenreRule::primary_scale`]。三者的小节数与拍号同源，
/// 因此 `spans` 的 tick 域恰好覆盖网格的 tick 域。
///
/// # Errors
///
/// - `bars == 0` 或拍号非法 ⇒ [`TheoryError::ZeroBars`]；
/// - `onsets_per_bar` 超过小节内的 16 分格位数 ⇒ [`TheoryError::ProgressionTooDense`]；
/// - 登记的摇摆比例折算后越界 ⇒ [`TheoryError::SwingOutOfRange`]；
/// - 约束非法、没有典型走向/音阶 ⇒ 见 [`melody_over_chords`] 与
///   [`GenreRule::sketch`]。
pub fn genre_melody_with(
    genre: &GenreRule,
    tonic: PitchClass,
    bars: u32,
    onsets_per_bar: u32,
    constraints: MelodyConstraints,
    seed: u64,
) -> Result<Melody, TheoryError> {
    let key = genre.primary_scale(tonic)?;
    let spans = genre.sketch(tonic, bars)?;
    let grid = genre.rhythm_grid(bars, onsets_per_bar)?;
    melody_over_chords(&key, &spans, &grid, constraints, seed)
}

/// 同 [`genre_melody`]，但按种子在**该流派登记的音阶与走向**里选一版。
///
/// 等价于 `genre_melody_for_with(genre, tonic, bars, onsets_per_bar,
/// MelodyConstraints::DEFAULT, seed)`。
///
/// # Errors
///
/// 见 [`genre_melody_for_with`]。
pub fn genre_melody_for(
    genre: &GenreRule,
    tonic: PitchClass,
    bars: u32,
    onsets_per_bar: u32,
    seed: u64,
) -> Result<Melody, TheoryError> {
    genre_melody_for_with(
        genre,
        tonic,
        bars,
        onsets_per_bar,
        MelodyConstraints::DEFAULT,
        seed,
    )
}

/// 同 [`genre_melody_with`]，但和声与音阶由 `seed` 从该流派**已登记的**数据里选。
///
/// 与 [`genre_melody_with`] 的唯一差别是"读哪一条登记数据"：
///
/// | 输入 | [`genre_melody_with`] | 本函数 |
/// | :--- | :--- | :--- |
/// | 音阶 | [`GenreRule::primary_scale`]（第 0 个） | [`GenreRule::scale_for`]（种子选） |
/// | 和声 | [`GenreRule::sketch`]（第 0 条走向） | [`GenreRule::sketch_for`]（种子选） |
/// | onset 网格 | [`GenreRule::rhythm_grid`] | 同左（不随种子变） |
///
/// 于是"同一个流派、同一个种子"完全决定一条旋律，而**换种子**同时换走向、
/// 换音阶、换音高选择 —— 全部只在登记数据之内，不发明新走向/新音阶
/// [ARCH-DET-001]。种子若恰好选中第 0 条走向与第 0 个音阶，本函数与
/// [`genre_melody_with`] **逐位相同**。
///
/// 展开和声用的音阶**就是**本次选中的音阶，因此每个和弦的根音都属于旋律的
/// 调（判据 `every_seeded_melody_note_is_in_its_own_key`）。
///
/// # Errors
///
/// 与 [`genre_melody_with`] 逐条相同，顺序也相同（先音阶、后和声、再网格）。
pub fn genre_melody_for_with(
    genre: &GenreRule,
    tonic: PitchClass,
    bars: u32,
    onsets_per_bar: u32,
    constraints: MelodyConstraints,
    seed: u64,
) -> Result<Melody, TheoryError> {
    let key = genre.scale_for(tonic, seed)?;
    let spans = genre.sketch_for(tonic, bars, seed)?;
    let grid = genre.rhythm_grid(bars, onsets_per_bar)?;
    melody_over_chords(&key, &spans, &grid, constraints, seed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genre::GenreLibrary;
    use crate::progression::{Progression, expand_progression};
    use crate::rhythm::metric_grid;
    use crate::scale::ScaleKind;

    fn c_major() -> Scale {
        Scale::new(PitchClass::C, ScaleKind::Major)
    }

    fn four_spans() -> Vec<ChordSpan> {
        expand_progression(&c_major(), "I-V-vi-IV", 4).unwrap()
    }

    #[test]
    fn constraints_reject_an_inverted_range_and_an_out_of_midi_upper_bound() {
        assert_eq!(
            MelodyConstraints::new(72, 60, 12).unwrap_err(),
            TheoryError::VoiceRangeInvalid {
                lower: 72,
                upper: 60
            }
        );
        assert_eq!(
            MelodyConstraints::new(60, 128, 12).unwrap_err(),
            TheoryError::VoiceRangeOutOfMidi
        );
        // 合法：上界正好 127。
        assert!(MelodyConstraints::new(60, 127, 12).is_ok());
        // max_leap = 0 是合法语义（同音反复），不在校验里拒绝。
        assert!(MelodyConstraints::new(60, 72, 0).is_ok());
    }

    #[test]
    fn every_note_is_a_key_scale_tone_and_the_line_is_contiguous() {
        let key = c_major();
        let spans = four_spans();
        let grid = metric_grid(Meter::COMMON, 4, 4).unwrap();
        let melody =
            melody_over_chords(&key, &spans, &grid, MelodyConstraints::DEFAULT, 7).unwrap();
        assert_eq!(melody.len(), grid.len());
        assert_eq!(melody.total_ticks(), 4 * 3840);
        for note in melody.notes() {
            let pc = PitchClass::new(note.pitch % 12).unwrap();
            assert!(key.contains(pc), "pitch {} escapes {key}", note.pitch);
            // 字面量音域（= 文档里的 48..=84）：不引用常量，避免判据空转。
            assert!(
                (48..=84).contains(&note.pitch),
                "pitch {} leaves the documented window",
                note.pitch
            );
            assert!(note.duration_ticks > 0);
        }
        // 首尾相接：第 i 个音的结束 tick 恒等于第 i + 1 个音的起始 tick。
        for pair in melody.notes().windows(2) {
            assert_eq!(pair[0].end_tick(), pair[1].start_tick);
        }
        // 总时长恰好铺满请求的小节数。
        assert_eq!(
            melody
                .notes()
                .iter()
                .map(|note| note.duration_ticks)
                .sum::<u64>(),
            grid.total_ticks()
        );
        assert_eq!(
            melody.notes().last().unwrap().end_tick(),
            grid.total_ticks()
        );
    }

    #[test]
    fn every_onset_comes_from_the_grid_in_ascending_order() {
        let key = c_major();
        let spans = four_spans();
        let grid = metric_grid(Meter::COMMON, 4, 6).unwrap();
        let melody =
            melody_over_chords(&key, &spans, &grid, MelodyConstraints::DEFAULT, 11).unwrap();
        let starts: Vec<u64> = melody.notes().iter().map(|note| note.start_tick).collect();
        let grid_ticks: Vec<u64> = grid.hits().iter().map(|hit| hit.tick).collect();
        assert_eq!(starts, grid_ticks);
        for (note, hit) in melody.notes().iter().zip(grid.hits()) {
            assert_eq!(note.bar, hit.bar);
            assert_eq!(note.cell, hit.cell);
            assert_eq!(note.weight, hit.weight);
            assert!(note.start_tick < melody.total_ticks());
        }
        assert_eq!(melody.notes_in_bar(0).len(), 6);
        assert!(melody.notes_in_bar(4).is_empty());
    }

    #[test]
    fn no_leap_exceeds_the_constraint() {
        for max_leap in [0u8, 1, 2, 3, 5, 12] {
            for seed in 0u64..8 {
                let key = c_major();
                let spans = four_spans();
                let grid = metric_grid(Meter::COMMON, 3, 8).unwrap();
                let constraints = MelodyConstraints::new(60, 84, max_leap).unwrap();
                let melody = melody_over_chords(&key, &spans, &grid, constraints, seed).unwrap();
                let mut worst = 0u8;
                for pair in melody.notes().windows(2) {
                    worst = worst.max(pair[0].pitch.abs_diff(pair[1].pitch));
                }
                assert!(
                    worst <= max_leap,
                    "max_leap {max_leap}, seed {seed}: leap {worst}"
                );
            }
        }
    }

    #[test]
    fn a_strong_beat_falls_back_to_the_scale_and_flags_it() {
        // I 铺 1 小节 + V 铺 1 小节，每小节 1 个 onset（都在强拍）：
        // 窗口 60..=64 里 I 的和弦音是 C(60)/E(64)，V 的和弦音只有 D(62)。
        // `max_leap = 0` 要求第二个音与第一个音**同高**，而 60/64 都不是 V 的和弦音
        // ⇒ 和弦档没有可行候选 ⇒ 退回音阶档（60/62/64 里与上一个音相同的那个）。
        // 该音因此**不是**和弦音，`chord_tone` 如实为 false。
        let key = c_major();
        let spans = expand_progression(&key, "I-V", 2).unwrap();
        let grid = metric_grid(Meter::COMMON, 2, 1).unwrap();
        let constraints = MelodyConstraints::new(60, 64, 0).unwrap();
        for seed in 0u64..8 {
            let melody = melody_over_chords(&key, &spans, &grid, constraints, seed).unwrap();
            assert_eq!(melody.len(), 2);
            assert_eq!(
                melody.notes()[0].pitch,
                melody.notes()[1].pitch,
                "seed {seed}: max_leap 0 must repeat the pitch"
            );
            assert!(
                melody.notes()[0].chord_tone,
                "seed {seed}: I onset is a chord tone"
            );
            assert!(
                !melody.notes()[1].chord_tone,
                "seed {seed}: the V onset fell back to the scale, so it is not a chord tone"
            );
        }
    }

    #[test]
    fn max_leap_zero_repeats_the_first_pitch() {
        // I 和弦 + 音阶 C 大调：窗口 60..=60 里只有 C，它既是音阶音也是和弦音。
        let key = c_major();
        let spans = expand_progression(&key, "I", 2).unwrap();
        let grid = metric_grid(Meter::COMMON, 2, 4).unwrap();
        let constraints = MelodyConstraints::new(60, 60, 0).unwrap();
        let melody = melody_over_chords(&key, &spans, &grid, constraints, 3).unwrap();
        assert_eq!(melody.len(), 8);
        assert!(melody.notes().iter().all(|note| note.pitch == 60));
    }

    #[test]
    fn the_documented_thresholds_are_pinned_to_literals() {
        // 判据纪律：常量本身在这里被钉成字面量，而行为判据（本测试之后的那几条）
        // 用**字面量**做门槛，不引用常量。否则"把常量改坏"会让行为判据一起空转
        // —— 这条纪律是注入 I1 实测换来的：常量改成 255 后，引用常量的
        // "强拍取和弦音"判据因为 `weight < 255` 恒真而**全部跳过**、照样绿。
        assert_eq!(MELODY_LOWER_BOUND, 48);
        assert_eq!(MELODY_UPPER_BOUND, 84);
        assert_eq!(MELODY_MAX_LEAP, 12);
        assert_eq!(CHORD_TONE_WEIGHT_FLOOR, 1);
        assert_eq!(MelodyConstraints::DEFAULT.lower, 48);
        assert_eq!(MelodyConstraints::DEFAULT.upper, 84);
        assert_eq!(MelodyConstraints::DEFAULT.max_leap, 12);
    }

    #[test]
    fn strong_beats_take_a_chord_tone_when_the_window_has_one() {
        let key = c_major();
        let spans = four_spans();
        let grid = metric_grid(Meter::COMMON, 4, 8).unwrap();
        for seed in 0u64..16 {
            let melody =
                melody_over_chords(&key, &spans, &grid, MelodyConstraints::DEFAULT, seed).unwrap();
            for note in melody.notes() {
                // 字面量门槛（= 文档里的 1）：不引用常量，避免常量被改坏时判据空转。
                if note.weight < 1 {
                    continue;
                }
                let span = spans
                    .iter()
                    .find(|span| {
                        span.start_tick <= note.start_tick && note.start_tick < span.end_tick()
                    })
                    .expect("every grid onset sits inside the tile of spans");
                let pcs: Vec<u8> = span
                    .chord
                    .pitch_classes()
                    .iter()
                    .map(|pc| pc.semitones())
                    .collect();
                let has_chord_tone = (48..=84).any(|pitch| pcs.contains(&(pitch % 12)));
                assert!(has_chord_tone, "test window must contain a chord tone");
                assert!(
                    note.chord_tone,
                    "seed {seed}: strong beat {note:?} is not a chord tone"
                );
            }
        }
    }

    #[test]
    fn the_same_seed_reproduces_the_line_bit_for_bit() {
        let key = c_major();
        let spans = four_spans();
        let grid = metric_grid(Meter::COMMON, 4, 5).unwrap();
        let first =
            melody_over_chords(&key, &spans, &grid, MelodyConstraints::DEFAULT, 99).unwrap();
        let second =
            melody_over_chords(&key, &spans, &grid, MelodyConstraints::DEFAULT, 99).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.seed(), 99);
        assert_eq!(first.key(), key);
        assert_eq!(first.meter(), Meter::COMMON);
        assert_eq!(first.bars(), 4);
        assert_eq!(first.onsets_per_bar(), 5);
        assert_eq!(first.swing_permille(), None);
        assert_eq!(first.constraints(), MelodyConstraints::DEFAULT);
    }

    #[test]
    fn different_seeds_produce_different_lines() {
        let key = c_major();
        let spans = four_spans();
        let grid = metric_grid(Meter::COMMON, 4, 8).unwrap();
        let mut distinct = std::collections::BTreeSet::new();
        for seed in 0u64..8 {
            let melody =
                melody_over_chords(&key, &spans, &grid, MelodyConstraints::DEFAULT, seed).unwrap();
            distinct.insert(
                melody
                    .notes()
                    .iter()
                    .map(|note| note.pitch)
                    .collect::<Vec<u8>>(),
            );
        }
        // 实测：8 个种子给出 8 条不同的音高序列（下界写 2，避免过度钉死）。
        assert!(
            distinct.len() >= 2,
            "seed does not influence the line: {distinct:?}"
        );
    }

    #[test]
    fn an_empty_harmony_skeleton_is_an_error() {
        let key = c_major();
        let grid = metric_grid(Meter::COMMON, 1, 4).unwrap();
        assert_eq!(
            melody_over_chords(&key, &[], &grid, MelodyConstraints::DEFAULT, 1).unwrap_err(),
            TheoryError::EmptyProgression
        );
    }

    #[test]
    fn zero_onsets_per_bar_yields_an_empty_melody() {
        let key = c_major();
        let spans = four_spans();
        let grid = metric_grid(Meter::COMMON, 2, 0).unwrap();
        let melody =
            melody_over_chords(&key, &spans, &grid, MelodyConstraints::DEFAULT, 1).unwrap();
        assert!(melody.is_empty());
        assert!(melody.notes_in_bar(0).is_empty());
        assert_eq!(melody.total_ticks(), 2 * 3840);
    }

    #[test]
    fn a_window_without_any_scale_tone_is_an_error() {
        // C 大调 + C# 窗口：窗口里一个音阶音都没有 ⇒ 不产出任何音，显式报错。
        let key = c_major();
        let spans = four_spans();
        let grid = metric_grid(Meter::COMMON, 1, 4).unwrap();
        let constraints = MelodyConstraints::new(61, 61, 12).unwrap();
        assert_eq!(
            melody_over_chords(&key, &spans, &grid, constraints, 1).unwrap_err(),
            TheoryError::NoFeasibleVoicing
        );
    }

    #[test]
    fn genre_melody_uses_the_genres_own_meter_and_swing() {
        use crate::genre::GenreLibrary;

        // `jazz_swing`：4/4、swing = 66.0 ⇒ 660 千分比 ⇒ 格点 1 落在 316。
        let jazz = GenreLibrary::get("jazz_swing").unwrap();
        let melody = genre_melody(jazz, PitchClass::C, 1, 9, 5).unwrap();
        assert_eq!(melody.meter(), Meter::COMMON);
        assert_eq!(melody.swing_permille(), Some(660));
        assert_eq!(melody.len(), 9);
        assert_eq!(melody.total_ticks(), 3840);
        let offbeat = melody
            .notes()
            .iter()
            .find(|note| note.cell == 1)
            .expect("9 onsets select the first weak sixteenth");
        assert_eq!(offbeat.start_tick, 316);
        assert_eq!(offbeat, &melody.notes()[1], "格点 1 是网格里第二个 onset");

        // `waltz`：3/4 ⇒ 每小节 2880 tick，音阶是该流派登记的。
        let waltz = GenreLibrary::get("waltz").unwrap();
        let melody = genre_melody(waltz, PitchClass::C, 2, 4, 1).unwrap();
        assert_eq!(melody.meter(), Meter::WALTZ);
        assert_eq!(melody.total_ticks(), 2 * 2880);
        assert_eq!(melody.key(), waltz.primary_scale(PitchClass::C).unwrap());
        assert_eq!(melody.notes_in_bar(1)[0].start_tick, 2880);
    }

    #[test]
    fn genre_melody_rejects_a_zero_bar_request() {
        use crate::genre::GenreLibrary;

        let jazz = GenreLibrary::get("jazz_swing").unwrap();
        assert_eq!(
            genre_melody(jazz, PitchClass::C, 0, 4, 1).unwrap_err(),
            TheoryError::ZeroBars
        );
        // 4/4 只有 16 个格位 ⇒ 17 个 onset 放不下（不静默钳制）。
        assert_eq!(
            genre_melody(jazz, PitchClass::C, 1, 17, 1).unwrap_err(),
            TheoryError::ProgressionTooDense {
                degrees: 17,
                slots: 16
            }
        );
    }

    #[test]
    fn the_same_progression_parsed_twice_gives_the_same_line() {
        // 抵消"spans 由不同路径构造"的可能性：parse 出来的区段与 expand_progression 一致。
        let key = c_major();
        let spans = four_spans();
        let parsed = Progression::parse("I-V-vi-IV")
            .unwrap()
            .with_meter(Meter::COMMON)
            .with_bars(4)
            .unwrap()
            .expand(&key)
            .unwrap();
        assert_eq!(spans, parsed);
        let grid = metric_grid(Meter::COMMON, 4, 4).unwrap();
        assert_eq!(
            melody_over_chords(&key, &spans, &grid, MelodyConstraints::DEFAULT, 4).unwrap(),
            melody_over_chords(&key, &parsed, &grid, MelodyConstraints::DEFAULT, 4).unwrap()
        );
    }

    #[test]
    fn genre_melody_for_matches_the_unseeded_api_when_the_seed_picks_the_first_entries() {
        // 旧 API 的行为是不变契约：种子若选中第 0 条走向与第 0 个音阶，
        // 种子版必须与 `genre_melody_with` 逐位相同。
        for rule in GenreLibrary::all() {
            let mut matched = 0usize;
            for seed in 0u64..256 {
                let zero_progression =
                    rule.progression_for(seed).unwrap() == rule.progression_at(0).unwrap();
                let zero_scale = rule.scale_for(PitchClass::C, seed).unwrap().kind
                    == rule.primary_scale(PitchClass::C).unwrap().kind;
                if zero_progression && zero_scale {
                    assert_eq!(
                        genre_melody_for(rule, PitchClass::C, 4, 4, seed).unwrap(),
                        genre_melody_with(
                            rule,
                            PitchClass::C,
                            4,
                            4,
                            MelodyConstraints::DEFAULT,
                            seed
                        )
                        .unwrap(),
                        "{} seed {seed}",
                        rule.id
                    );
                    matched += 1;
                }
            }
            assert!(matched > 0, "{}: no zero-picking seed in 0..256", rule.id);
        }
    }

    #[test]
    fn every_seeded_melody_stays_in_the_key_it_was_built_from() {
        let mut differing = 0usize;
        let mut total = 0usize;
        for rule in GenreLibrary::all() {
            let mut first: Option<Melody> = None;
            for seed in 0u64..8 {
                let melody = genre_melody_for(rule, PitchClass::C, 4, 4, seed).unwrap();
                let key = rule.scale_for(PitchClass::C, seed).unwrap();
                assert_eq!(melody.key(), key, "{}", rule.id);
                assert_eq!(melody.len(), 4 * 4, "{}", rule.id);
                assert_eq!(melody.total_ticks(), 4 * rule.meter_value().ticks_per_bar());
                for note in melody.notes() {
                    let pc = PitchClass::new(note.pitch % 12).unwrap();
                    assert!(key.contains(pc), "{} seed {seed}: {}", rule.id, note.pitch);
                    // 字面量音域（= 文档里的 48..=84）。
                    assert!((48..=84).contains(&note.pitch), "{}", rule.id);
                }
                total += 1;
                match &first {
                    None => first = Some(melody),
                    Some(base) => {
                        if base != &melody {
                            differing += 1;
                        }
                    }
                }
            }
        }
        // 实测读数：182 条流派 × 7 次比较 = 1274 次里，有 1274 次与种子 0 的旋律不同。
        assert_eq!(total, 182 * 8);
        assert_eq!(differing, 1274);
    }

    #[test]
    fn the_seeded_melody_is_pinned_to_a_literal_reading() {
        // funk 的 4/4、4 onset/小节：种子 0 选 `I7-IV7` + blues。
        let funk = GenreLibrary::get("funk").unwrap();
        assert_eq!(funk.progression_for(0).unwrap(), "I7-IV7");
        let melody = genre_melody_for(funk, PitchClass::C, 2, 4, 0).unwrap();
        let pitches: Vec<u8> = melody.notes().iter().map(|note| note.pitch).collect();
        assert_eq!(melody.key().kind.name(), "blues");
        assert_eq!(melody.len(), 8);
        assert_eq!(pitches, vec![70, 79, 72, 60, 58, 66, 54, 54]);
    }
}
