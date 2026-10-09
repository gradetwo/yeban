//! # yeban-theory — 乐理与流派规则引擎
//!
//! 本 crate 承载 `yeban_propose_section` 的领域逻辑：音高与音程、音阶构造、
//! 和弦构造与符号解析、罗马数字走向展开、声部连接 (voice leading) 与流派规则库。
//!
//! 规范来源 (Normative):
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §8 (`yeban-theory`)、§7 (`yeban_propose_section`)
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M4-003
//! - `AGENTS.md` §2 红线 3/4/8、§3 DoD
//!
//! ## 这一层**不**做什么（边界声明）
//!
//! 1. **不碰音频设备**。本 crate 没有、也永不引入 `cpal` / `symphonia` / 任何
//!    音频或 GUI 依赖 [ARCH-TOP-003]。它只产出"音高与 tick 的整数描述"，
//!    由 `yeban-engine` / `yeban-render` 去发声。
//! 2. **不读系统时钟**。没有任何 `SystemTime::now()` / `Instant::now()`；
//!    所有与时间的交互都通过调用方传入的 `start_tick` / `bars` / 拍号完成。
//! 3. **不做 I/O**。没有 `std::fs`、没有网络、没有打印。全部是纯函数与纯数据。
//! 4. **没有隐藏全局状态**。唯一的全局量是两张**只读**索引表
//!    ([`genre::GenreLibrary`] 的 ID 索引与搜索缓存)，由 `OnceLock` 惰性构建，
//!    构建过程只读常量表，不含任何可变状态或随机性。
//!
//! ## 随机性：全部由调用方种子驱动 [ARCH-DET-001]
//!
//! 本 crate 内部**不存在**随机数。需要"在若干选项中挑一个"的地方，一律使用
//! [`derive_index`] —— 一个把 `(rng_seed, salt)` 通过 SplitMix64 折成确定性
//! 下标的纯函数。同一个种子、同一份输入，在任何机器、任何编译器上得到同一个结果；
//! 反过来，调用方只要换种子就能得到不同的编排。
//!
//! 浮点只出现在**两个**地方，两处都走 `libm`（与架构文档对 L1 确定性
//! "统一启用纯 Rust `libm` 数学库"的要求一致）：
//!
//! 1. [`pitch::note_to_hz`] 的频率输出；
//! 2. [`genre::GenreRule::swing_permille`] 把登记的 `f32` 百分数折成整数千分比
//!    （唯一一次乘法与取整）。此后 [`swing`] 与 [`rhythm`] 的全部运算都是整数。
//!
//! 没有任何判定逻辑依赖浮点比较。
//!
//! ## 确定性集合 [MODEL-AST-003 / 红线 4]
//!
//! 本 crate 不使用 `HashMap` / `HashSet`；需要按键查表时用
//! `BTreeMap`（见 [`genre::GenreLibrary`] 的索引），迭代顺序在任何进程、
//! 任何平台上都一致。
//!
//! ## 快速开始
//!
//! ```
//! use yeban_theory::chord::Chord;
//! use yeban_theory::genre::GenreLibrary;
//! use yeban_theory::pitch::{note_to_hz, PitchClass};
//! use yeban_theory::progression::expand_progression;
//! use yeban_theory::scale::{Scale, ScaleKind};
//! use yeban_theory::voice_leading::realize_three_voices;
//!
//! // 1. 一个调、一条走向、4 小节 → 整数 tick 的和弦骨架
//! let key = Scale::new(PitchClass::C, ScaleKind::Major);
//! let spans = expand_progression(&key, "I-V-vi-IV", 4).unwrap();
//! assert_eq!(spans.len(), 4);
//! assert_eq!(spans.iter().map(|s| s.duration_ticks).sum::<u64>(), 4 * 960 * 4);
//!
//! // 2. 展开成 3 个声部
//! let voices = realize_three_voices(&spans).unwrap();
//! assert_eq!(voices.voicings.len(), 4);
//! assert!(voices.max_voice_jump() <= 12);
//!
//! // 3. 流派规则库
//! let bossa = GenreLibrary::get("bossa_nova").unwrap();
//! assert_eq!(bossa.meter, (4, 4));
//!
//! // 4. 和弦符号往返
//! let chord = Chord::from_symbol("F#m7b5").unwrap();
//! assert_eq!(chord.symbol(), "F#m7b5");
//!
//! // 5. 频率
//! assert!((note_to_hz(69) - 440.0).abs() < 1e-9);
//! ```
//!
//! ## 状态
//!
//! `Phase 1` 可编译可测试的实现切片（不再是 scaffold）。尚未实现的见
//! `docs/ledger/theory-core-notes.md` 的 `pending` 与缺口清单。
//!
//! `docs/ledger/theory-core-notes.md:304` 的 `pending 6`（"`swing` 只登记不应用"）
//! 由 [`swing`] 关闭：[`GenreRule::swing_permille`] 把登记表里的 `f32` 百分数
//! 折成整数千分比，[`swing::swung_pair_span`] / [`swing::quantize_onset`]
//! 按该比例切分与量化 tick。
//!
//! 同一台账的 `pending 3`（"没有具体的鼓点网格"）由 [`rhythm`] 补上**网格**那一半：
//! [`rhythm::swung_metric_grid`] 把拍号与摇摆比例落成逐 16 分音符的 onset 网格，
//! [`genre::GenreRule::rhythm_grid`] 直接读该流派登记的拍号与摇摆比例。
//! 网格的度量重量由 [`rhythm::metric_weight_in`] 按**拍号**算出，因此
//! 6/8（复合二拍）与 3/4（三拍）虽然小节长度相同，得到的网格不同。
//! 另一半（逐流派的鼓点型数据）**没有**做：登记的 `note_density_hint` 计的是
//! 音符数而不是 onset 数（见 [`rhythm`] 的模块文档），要补它需要新增登记数据。
//!
//! 同一处还有一条本模块自己记下的偏差（"各地区的节拍分组，例如 5/4 的 3+2 与
//! 2+3，本函数与 `metric_weight_in` 都不给：那需要逐流派数据"）：[`rhythm`]
//! 补上了它的**机制**侧 —— [`rhythm::BeatGrouping`] 让调用方把加性分组
//! （7/8 = 2+2+3、5/4 = 3+2、11/8 = 2+2+3+2+2）传进来，
//! [`rhythm::metric_weight_grouped`] 与 [`rhythm::grouped_metric_grid`] 按它
//! 算重量与选点。**逐流派的分组数据仍未登记**（`GENRES` 里没有这个字段，
//! 7/8 的两种切法都通行），因此 [`genre::GenreRule::rhythm_grid`] 继续走
//! 拍号口径，本 crate 不替任何流派猜一个分组。
//!
//! 同一台账的 `pending 4`（"没有实现旋律生成"）由 [`melody`] 关闭：
//! [`melody::melody_over_chords`] 在既有的音阶 + 和声区段 + 节奏网格上落出
//! 一条确定性的单声部旋律（种子驱动，[`melody::genre_melody`] 读流派的登记数据）。
//! 它**不新增任何登记数据**，也不做"好听"的判定（见 [`melody`] 的边界声明）。
//!
//! 同一台账的 `pending 3` 还剩"**具体的鼓点**"这一半，由 [`drum`] 补上：
//! [`drum::swung_drum_pattern`] 把**已经存在的** [`rhythm::MetricGrid`] 按
//! 度量重量与拍分组分派到底鼓／军鼓／踩镲／吊镲，产出一份可逐件读回的鼓组型。
//! 它同样**不新增登记数据**：分派只读网格既有的 `tick` / `bar` / `cell` / `weight`
//! 与调用方传入的分组、反拍位置。
//!
//! `pending 3` 的**数据那一半**（"哪条流派打什么样的鼓点"）本 crate 只登记了
//! **底鼓落点**这一维：[`genre::GenreRule`] 的 `drum_style` 字段对 **13 条**
//! 舞曲流派登记 [`drum::DrumStyle::FourOnTheFloor`]（每拍一击的底鼓），
//! 其余 **169 条**登记 [`drum::DrumStyle::Metric`]（= 旧口径，逐位不变）——
//! 本 crate **不替**这些流派猜一个鼓点型。军鼓位置、踩镲密度与 onset 数
//! **仍未**登记，因此这一半是**部分**关闭（见 [`drum`] 与
//! [`genre::GenreRule::drum_pattern`] 的文档）。
//!
//! ## 登记数据的"第 0 条"不再是唯一入口（本次）
//!
//! 规则库登记了 **391** 条典型走向与 **520** 个典型音阶，但此前**文档化的流派
//! API** 只读第 0 条：[`GenreRule::sketch`] / [`GenreRule::chords`] /
//! [`GenreRule::primary_scale`]。本次补上索引与种子两个入口，全部**只读已登记
//! 数据**、不新增任何走向或音阶：
//!
//! - 显式索引：[`GenreRule::progression_count`] / [`GenreRule::progression_at`] /
//!   [`GenreRule::scale_count`] / [`GenreRule::scale_name_at`] /
//!   [`GenreRule::scale_at`] / [`GenreRule::sketch_at`] / [`GenreRule::chords_at`]；
//!   越界用 `Option` 或既有的 [`TheoryError::EmptyProgression`] /
//!   [`TheoryError::ScaleNameUnknown`] 表达，**不新增错误变体**；
//! - 种子驱动（[ARCH-DET-001] 的既有口径）：[`GenreRule::progression_for`] /
//!   [`GenreRule::scale_for`] / [`GenreRule::sketch_for`] / [`GenreRule::chords_for`]，
//!   以及把整条链一起换版的 [`melody::genre_melody_for`] /
//!   [`melody::genre_melody_for_with`]。
//!
//! 判据钉住的读数：182 条流派全部至少登记 2 条走向（2 条 = 155、3 条 = 27）与
//! 至少 1 个音阶（1 个 = 5、2 个 = 40、3 个 = 113、4 个 = 24）；种子 0..=63 对
//! **每一条**流派都能取到全部登记的**走向**下标；对**音阶**则是"每个登记**种类**
//! 都取得到"——登记表里有 **5** 条流派把同一个音阶种类登记了两次
//! （`aeolian` 与 `natural_minor` 同种类、`ionian` 与 `major` 同种类、
//! `indie_rock` 的 `major` 出现两次），种子选择器返回的是**音阶**（种类），
//! 因此这 **5** 个重复位置**永远不被种子选中**，但它们与先出现的那一条**逐位
//! 相同**（判据 `the_seed_selector_can_reach_every_registered_kind` 与
//! `only_the_duplicate_kind_positions_stay_out_of_the_seed_space`；
//! 种子 0..65535 上实测的从未被选中位置 = 5）。种子 0..=31 里 182/182 条流派
//! 至少有一版骨架与旧的 [`GenreRule::sketch`] 不同；种子选中第 0 条时与旧 API
//! 逐位相同。
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod chord;
pub mod drum;
pub mod error;
pub mod genre;
pub mod melody;
pub mod pitch;
pub mod progression;
pub mod rhythm;
pub mod scale;
pub mod swing;
pub mod voice_leading;

pub use chord::{Chord, ChordKind, Tonality};
pub use drum::{
    Backbeat, DRUM_VOICE_COUNT, DrumHit, DrumPattern, DrumStyle, DrumVoice, default_backbeat,
    drum_pattern, is_meter_group_start, styled_drum_pattern, swung_drum_pattern,
    swung_styled_drum_pattern,
};
pub use error::TheoryError;
pub use genre::{GenreLibrary, GenreRule};
pub use melody::{
    CHORD_TONE_WEIGHT_FLOOR, MELODY_LOWER_BOUND, MELODY_MAX_LEAP, MELODY_UPPER_BOUND, Melody,
    MelodyConstraints, MelodyNote, genre_melody, genre_melody_for, genre_melody_for_with,
    genre_melody_with, melody_over_chords,
};
pub use pitch::{Interval, NoteName, Pitch, PitchClass, note_to_hz, parse_pitch_class};
pub use progression::{
    ChordSpan, Degree, Meter, PPQ, Progression, RomanQuality, expand_progression,
};
pub use rhythm::{
    BEAT_WEIGHT, BeatGrouping, GridHit, MAX_METRIC_WEIGHT, MetricGrid, OFFBEAT_WEIGHT,
    STRONG_BEAT_WEIGHT, cells_per_bar, felt_beats_per_bar, grouped_metric_grid,
    grouped_swung_metric_grid, is_compound_meter, metric_grid, metric_weight,
    metric_weight_grouped, metric_weight_in,
};
pub use scale::{Scale, ScaleKind};
pub use swing::{
    SWING_PERMILLE_MAX, SWING_PERMILLE_STRAIGHT, SwingPair, quantize_onset, swung_onset_offset,
    swung_pair_span,
};
pub use voice_leading::{
    VoiceLeadingResult, VoiceRange, VoicingConstraints, realize, realize_three_voices,
};

/// SplitMix64 的规范乘子（Steele et al., 2014；公有领域算法）。
///
/// 选择 SplitMix64 的理由：全部是 64 位整数运算（无浮点、无平台相关的
/// 溢出行为），实现只有几行，且不需要任何全局可变状态。
const SPLITMIX_GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// 把 `(seed, salt)` 折叠成 `0..=u64::MAX` 的确定性哈希。
///
/// 纯函数：同输入同输出，跨进程跨平台一致。用于替代"随机选一个"。
#[must_use]
pub fn splitmix64(seed: u64, salt: u64) -> u64 {
    let mut z = seed
        .wrapping_add(salt.wrapping_mul(SPLITMIX_GOLDEN_GAMMA))
        .wrapping_add(SPLITMIX_GOLDEN_GAMMA);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 用 `(rng_seed, salt)` 在 `0..count` 中确定性地挑一个下标。
///
/// `count == 0` 时返回 0（调用方负责保证集合非空；这里不 panic，也不引入
/// "除零"这种未定义路径）。
#[must_use]
pub fn derive_index(rng_seed: u64, salt: u64, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (splitmix64(rng_seed, salt) % count as u64) as usize
}

/// 用 `(rng_seed, salt)` 在闭区间 `[low, high]` 中确定性地挑一个整数。
///
/// `low >= high` 时返回 `low`。
///
/// 区间宽度在 `i64` 上**可能装不下**：例如 `low == i64::MIN, high == 0` 的宽度是
/// `2^63`，`high - low` 已经溢出。因此宽度在 `i128` 上算、偏移也在 `i128` 上算，
/// 结果必然落在 `low..=high` 内，且与"宽度装得下"时的旧口径**逐位相同**
/// （判据 `derive_range_stays_within_bounds` 的域内外都成立）。
#[must_use]
pub fn derive_range_i64(rng_seed: u64, salt: u64, low: i64, high: i64) -> i64 {
    if low >= high {
        return low;
    }
    let span = (i128::from(high) - i128::from(low) + 1) as u128;
    let offset = u128::from(splitmix64(rng_seed, salt)) % span;
    (i128::from(low) + offset as i128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 形态 D 注入实测（本票）：把 `derive_index` 的早退放宽成
    /// `count <= 1` 时没有任何既有判据变红。该改动在观测上等价
    /// （`count == 1` 时 `x % 1 == 0`），因此这里钉住的是**契约**：
    /// 单元素集合上任何种子都必须取到下标 0（而不是被早退"吞掉"成别的读数）。
    #[test]
    fn derive_index_on_a_single_element_set_is_always_zero() {
        for seed in 0u64..64 {
            assert_eq!(derive_index(seed, 0, 1), 0);
            assert_eq!(derive_index(seed, u64::MAX, 1), 0);
        }
        assert_eq!(derive_index(0, 0, 0), 0);
    }

    #[test]
    fn splitmix64_is_a_pure_function_of_its_inputs() {
        assert_eq!(splitmix64(42, 0), splitmix64(42, 0));
        assert_ne!(splitmix64(42, 0), splitmix64(43, 0));
        assert_ne!(splitmix64(42, 0), splitmix64(42, 1));
        // 与参考实现的固定向量对齐，防止"改了个常量但没人发现"。
        assert_eq!(splitmix64(0, 0), 0xE220_A839_7B1D_CDAF);
    }

    #[test]
    fn derive_index_stays_in_bounds_and_covers_the_range() {
        for count in [1usize, 2, 3, 7, 12, 182] {
            for seed in 0u64..50 {
                assert!(derive_index(seed, 0, count) < count);
                assert!(derive_index(seed, 7, count) < count);
            }
        }
        assert_eq!(derive_index(0, 0, 0), 0);
        // 不同种子应当能覆盖到多个不同下标（不是常量函数）。
        // 用定长位图统计，避免在测试里引入集合类型。
        let mut seen = [false; 7];
        for seed in 0u64..64 {
            seen[derive_index(seed, 0, 7)] = true;
        }
        assert!(
            seen.iter().filter(|&&hit| hit).count() >= 4,
            "derive_index looks degenerate: {seen:?}"
        );
    }

    #[test]
    fn derive_range_respects_both_ends() {
        for seed in 0u64..100 {
            let value = derive_range_i64(seed, 3, 60, 120);
            assert!((60..=120).contains(&value), "{value}");
        }
        assert_eq!(derive_range_i64(0, 0, 5, 5), 5);
        assert_eq!(derive_range_i64(0, 0, 9, 3), 9);
    }

    #[test]
    fn derive_range_survives_widths_that_do_not_fit_in_i64() {
        // 宽度可以是 `2^64 - 1`（`i64::MIN..=i64::MAX`）。在 `i64` 上算宽度会溢出
        // （debug 下 panic、release 下取余除零），因此这条判据同时钉住
        // "不 panic" 与 "仍在闭区间内"。
        for seed in 0u64..64 {
            for (low, high) in [
                (i64::MIN, i64::MAX),
                (i64::MIN, 0),
                (i64::MIN, i64::MIN + 1),
                (-5, i64::MAX),
                (0, i64::MAX),
            ] {
                let value = derive_range_i64(seed, 9, low, high);
                assert!(
                    (low..=high).contains(&value),
                    "seed {seed}: {value} outside [{low}, {high}]"
                );
            }
        }
        // 逐位一致读数：整段 `i64` 值域上的第 0 个种子。
        assert_eq!(
            derive_range_i64(0, 0, i64::MIN, i64::MAX),
            7_070_836_379_803_831_727
        );
    }

    /// `derive_range_i64` 的区间是**闭**的：两个端点都必须真的取得到。
    ///
    /// 注入实测（theory-16 形态 D）：把宽度从 `high - low + 1` 改成
    /// `high - low`，全部既有判据保持全绿 —— 既有判据只断言"落在区间内"，
    /// 而少算 1 只会让**上界**永远取不到，读数仍然落在区间内。这条判据补上
    /// "闭区间两端可达"这一半：宽度少 1 时上界立刻不可达，注入变红。
    ///
    /// 口径：对每个区间扫 `seed = 0..4096`（固定盐），收集实际出现的值；
    /// 断言 `low` 与 `high` 都出现过，且全部读数都落在 `low..=high` 内。
    /// 这是**探测**（宽度少 1 时上界要靠更大的种子空间才能偶然补回来），
    /// 不是统计断言：被测区间都远小于 4096 个种子的覆盖能力。
    #[test]
    fn derive_range_reaches_both_ends_of_the_closed_interval() {
        for (low, high) in [
            (0i64, 1i64),
            (0, 2),
            (0, 7),
            (0, 100),
            (60, 120),
            (-5, 5),
            (i64::MAX - 1, i64::MAX),
        ] {
            let mut seen_low = false;
            let mut seen_high = false;
            for seed in 0u64..4096 {
                let value = derive_range_i64(seed, 3, low, high);
                assert!(
                    (low..=high).contains(&value),
                    "seed {seed}: {value} outside [{low}, {high}]"
                );
                seen_low |= value == low;
                seen_high |= value == high;
            }
            assert!(seen_low, "[{low}, {high}]: the lower end is unreachable");
            assert!(seen_high, "[{low}, {high}]: the upper end is unreachable");
        }
        // 退化区间 `low >= high` 恒返回 `low`（与宽度无关的既有口径）。
        assert_eq!(derive_range_i64(0, 0, 5, 5), 5);
        assert_eq!(derive_range_i64(0, 0, 9, 3), 9);
    }

    #[test]
    fn crate_has_no_hidden_nondeterminism_sources() {
        // 机械检查：本 crate 的生产代码里不得出现时钟、线程、环境变量、
        // 文件系统或哈希集合。这是"不读系统时钟、无隐藏全局状态、确定性集合"
        // 这几条承诺的可执行判据。
        //
        // 检查口径：只看**非注释行**，且只看到本模块为止（忽略测试模块本身，
        // 那里允许使用 `&` 与 `Vec` 等辅助工具）。
        const SOURCES: [(&str, &str); 12] = [
            ("lib.rs", include_str!("lib.rs")),
            ("pitch.rs", include_str!("pitch.rs")),
            ("scale.rs", include_str!("scale.rs")),
            ("chord.rs", include_str!("chord.rs")),
            ("progression.rs", include_str!("progression.rs")),
            ("voice_leading.rs", include_str!("voice_leading.rs")),
            ("genre.rs", include_str!("genre.rs")),
            ("swing.rs", include_str!("swing.rs")),
            ("rhythm.rs", include_str!("rhythm.rs")),
            ("melody.rs", include_str!("melody.rs")),
            ("drum.rs", include_str!("drum.rs")),
            ("error.rs", include_str!("error.rs")),
        ];
        // 逐字节拼出禁词，避免这段代码自己包含禁词字面量。
        let forbidden = [
            "SystemTime::now",
            "Instant::now",
            "std::env::var",
            "std::fs::",
            "thread_rng",
            "rand::",
            concat!("Hash", "Map"),
            concat!("Hash", "Set"),
        ];
        for (name, source) in SOURCES {
            for (lineno, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if trimmed.starts_with("#[cfg(test)]") {
                    break;
                }
                for needle in forbidden {
                    assert!(
                        !line.contains(needle),
                        "{name}:{}: forbidden construct `{needle}`",
                        lineno + 1
                    );
                }
            }
        }
    }
}
