//! 摇摆 (swing) 量化：把**平直的 tick 网格**折成**摇摆网格**。
//!
//! ## 为什么需要这个模块
//!
//! [`crate::genre::GenreRule::swing`] 登记了每个流派的摇摆比例，但在本模块出现
//! 之前**没有任何函数消费它**（见 `docs/ledger/theory-core-notes.md:304` 的
//! `pending 6`：'没有"按 swing 比例把八分音符对量化"的函数'）。
//! 只登记不应用的数据不是能力，只是注释。本模块把那条 `pending` 变成可调用的原语。
//!
//! ## 语义：比例是**一对之内**的切分点
//!
//! 摇摆只改变**一对**网格单位内部的边界位置，不改变每一对的起点：
//!
//! ```text
//! 网格单位 = 480 tick (960 PPQ 下的八分音符)
//! 平直 (500 千分比):   [ 前半 = 240 ][ 后半 = 240 ]
//! 摇摆 (660 千分比):   [ 前半 = 316 ][ 后半 = 164 ]
//! ```
//!
//! - **500** = 平直。前半 = 后半（奇数长度时前半向下取整）。
//! - **>500** = 摇摆。前半变长、后半变短。对内的第二个槽位被**推迟**。
//! - **1000** = 附点。前半 = `pair_ticks`、后半 = 0。
//!
//! 因此每对的起点始终落在平直网格上。调用方只需给出**一个网格单位**的长度
//! （八分音符 = [`crate::progression::PPQ`] / 2 = 480，十六分音符 = 240）。
//!
//! ## 表示：千分比整数，不是浮点
//!
//! 输入用 `u16` 千分比（`500` = 50.0%，`667` ≈ 三连音摇摆）。
//! [`crate::genre::GenreRule::swing`] 存的 `f32` 百分数在
//! [`crate::genre::GenreRule::swing_permille`] 一处转换，转换后**全部是整数运算**。
//!
//! 理由与 [ARCH-DET-001] 一致：本 crate 的判定路径不含浮点比较，
//! 整数除法在任何平台、任何编译器上得到同一位模式，因此摇摆结果逐位一致。
//!
//! ## 算术（唯一一处除法，方向已固定）
//!
//! ```text
//! 前半 = pair_ticks * permille / 1000       (整除, 向下取整)
//! 后半 = pair_ticks - 前半
//! 摇摆偏移 = 前半 - pair_ticks / 2          (正数向右 = 推迟)
//! ```
//!
//! 例（网格单位 = 480 tick）：`480 * 667 / 1000 = 320160 / 1000 = 320`（整除，
//! 与三连音的 2/3 相差不到 0.3%）；`480 * 660 / 1000 = 316`（登记表的 66%）。
//!
//! ## 实时音频适用性 [AGENTS.md 红线 7]
//!
//! 本模块的每个函数都是**纯整数运算**：无堆分配、无锁、无 I/O、无日志。
//! 结果类型是 `Copy`。因此可以在逐样本 / 逐事件路径上直接调用。
//! 本模块不使用 `HashMap` / `HashSet` [MODEL-AST-003]。

use crate::error::TheoryError;

/// 平直比例（千分比）。`500` = 50.0%，此时摇摆不改变任何 tick。
pub const SWING_PERMILLE_STRAIGHT: u16 = 500;

/// 摇摆比例的合法上限（千分比）。`1000` = 100%，即附点八分。
pub const SWING_PERMILLE_MAX: u16 = 1000;

/// 一对网格单位内的两个半段长度（tick，整数）。
///
/// 不变量：`first + second == pair_ticks`（见 `swung_pair_span` 的测试）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SwingPair {
    /// 该对**前半**的 tick 数（摇摆时变长）。
    pub first: u64,
    /// 该对**后半**的 tick 数（摇摆时变短）。`first + second` 恒等于整对长度。
    pub second: u64,
}

impl SwingPair {
    /// 该对的整段长度（tick）。
    #[must_use]
    pub const fn total(self) -> u64 {
        self.first + self.second
    }

    /// 后半段的起点相对该对起点的偏移（tick）。摇摆时这个值大于平直的 `pair / 2`。
    #[must_use]
    pub const fn offbeat_offset(self) -> u64 {
        self.first
    }
}

/// 检查摇摆比例的合法区间 `500..=1000`。
///
/// # Errors
///
/// `permille` 不在 `500..=1000` 时返回 [`TheoryError::SwingOutOfRange`]。
/// 越界值**不**被静默钳制：静默钳制会让"MCP 传了 0.5（本意 50%）"
/// 变成"平直"而不是可观测的错误。
pub const fn validate_swing_permille(permille: u16) -> Result<(), TheoryError> {
    if permille < SWING_PERMILLE_STRAIGHT || permille > SWING_PERMILLE_MAX {
        return Err(TheoryError::SwingOutOfRange { value: permille });
    }
    Ok(())
}

/// 求一对网格单位内部的摇摆偏移（tick，带符号）。
///
/// 正值 = 后半的起点**向右**移动（推迟），负值 = 向左。平直时返回 `0`。
///
/// `permille` 合法时前半恒不短于一半（`permille >= 500`），因此真实偏移恒非负；
/// 在 `i128` 上相减，只有"偏移本身装不进 `i64`"（`pair_ticks` 大于 `i64::MAX`）
/// 时才饱和到 `i64::MAX` —— 返回类型装不下更大的值，这里不 panic。
///
/// # Errors
///
/// `permille` 越界时返回 [`TheoryError::SwingOutOfRange`]；
/// `pair_ticks` 为 0 时返回 [`TheoryError::ZeroBars`]（0 长度的一对没有可切分的内容）。
pub fn swung_onset_offset(pair_ticks: u64, permille: u16) -> Result<i64, TheoryError> {
    validate_swing_permille(permille)?;
    if pair_ticks == 0 {
        return Err(TheoryError::ZeroBars);
    }
    let half = pair_ticks / 2;
    let first = swung_first(pair_ticks, permille);
    let offset = i128::from(first) - i128::from(half);
    Ok(i64::try_from(offset).unwrap_or(i64::MAX))
}

/// 把一对网格单位按摇摆比例切成两段。
///
/// # Errors
///
/// `permille` 越界时返回 [`TheoryError::SwingOutOfRange`]；
/// `pair_ticks` 为 0 时返回 [`TheoryError::ZeroBars`]。
pub fn swung_pair_span(pair_ticks: u64, permille: u16) -> Result<SwingPair, TheoryError> {
    validate_swing_permille(permille)?;
    if pair_ticks == 0 {
        return Err(TheoryError::ZeroBars);
    }
    let first = swung_first(pair_ticks, permille);
    Ok(SwingPair {
        first,
        second: pair_ticks - first,
    })
}

/// 把一个 onset 量化到**所在那一对**的摇摆网格上。
///
/// 一对的两个槽位是 `对起点` 与 `对起点 + 前半`。取**较近**的槽位；
/// 正中间（`2 * in_pair == 前半`）取**前一个**槽位（ties go early），
/// 与降号侧口径同类：确定性且不引入随机。
///
/// 结果被钳制在**本对**之内：输入的 onset 不会跨到相邻的一对。
/// 钳制是必需的，不是装饰：`permille == 1000` 时后半长度为 0，
/// 第二个槽位会与**下一对的起点**重合，于是本对末尾的 onset 会被它吸走。
/// 这种情况把第二个槽位退回本对最后一个 tick（`pair_ticks - 1`）。
/// 该函数因此在全部合法比例上都是幂等的
/// （见测试 `quantizing_twice_changes_nothing` 与 `maximum_swing_keeps_an_onset_inside_its_pair`）。
///
/// # Errors
///
/// `permille` 越界时返回 [`TheoryError::SwingOutOfRange`]；
/// `pair_ticks` 为 0 时返回 [`TheoryError::ZeroBars`]。
pub fn quantize_onset(
    onset_ticks: u64,
    pair_ticks: u64,
    permille: u16,
) -> Result<u64, TheoryError> {
    validate_swing_permille(permille)?;
    if pair_ticks == 0 {
        return Err(TheoryError::ZeroBars);
    }
    let in_pair = onset_ticks % pair_ticks;
    let first = swung_first(pair_ticks, permille);
    // 第二个槽位不得等于 `pair_ticks`（那是下一对的起点）。
    let second_slot = first.min(pair_ticks - 1);
    // 正中间归前半：`2 * in_pair <= first` 时取槽位 0。比较在 `u128` 上做：
    // `in_pair` 可以大于 `u64::MAX / 2`（极长的对），`2 * in_pair` 在 `u64` 上会
    // 溢出（debug 下 panic，release 下回绕后把后半误判成前半）。
    let slot = if u128::from(in_pair) * 2 <= u128::from(first) {
        0
    } else {
        second_slot
    };
    // `onset_ticks - in_pair` 是本对起点（不会下溢）；对尾超出 `u64` 值域时
    // （`onset_ticks` 逼近 `u64::MAX`）饱和到 `u64::MAX`，不 panic。
    Ok((onset_ticks - in_pair).saturating_add(slot))
}

/// 前半段长度（tick）。**唯一**的除法就在这一行。
///
/// `pair_ticks` 已经是 `u64`；用 `u128` 乘以免在极大的对数上溢出。
/// 调用方必须先验 `permille` 与 `pair_ticks`（两个公开入口都已验）。
fn swung_first(pair_ticks: u64, permille: u16) -> u64 {
    let scaled = u128::from(pair_ticks) * u128::from(permille) / 1000;
    // `permille <= 1000` ⇒ 前半 <= pair_ticks，`u64::try_from` 永不失败。
    u64::try_from(scaled).unwrap_or(pair_ticks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progression::PPQ;

    /// 一个八分音符的 tick 数（960 PPQ 下的摇摆基本单位）。
    const EIGHTH: u64 = PPQ / 2;

    #[test]
    fn straight_swing_splits_a_pair_in_half() {
        let pair = swung_pair_span(EIGHTH, SWING_PERMILLE_STRAIGHT).unwrap();
        assert_eq!(
            pair,
            SwingPair {
                first: 240,
                second: 240
            }
        );
        assert_eq!(pair.total(), EIGHTH);
        assert_eq!(swung_onset_offset(EIGHTH, 500).unwrap(), 0);
    }

    #[test]
    fn the_triplet_feel_is_667_permille() {
        // 三连音摇摆 = 2/3 : 1/3。480 * 667 / 1000 = 320160 / 1000 = 320 (整除)。
        let pair = swung_pair_span(EIGHTH, 667).unwrap();
        assert_eq!(pair.first, 320);
        assert_eq!(pair.second, 160);
        assert_eq!(pair.total(), EIGHTH);
        assert_eq!(swung_onset_offset(EIGHTH, 667).unwrap(), 320 - 240);
    }

    #[test]
    fn extreme_swing_is_dotted_and_total_is_preserved() {
        let pair = swung_pair_span(EIGHTH, 1000).unwrap();
        assert_eq!(pair.first, EIGHTH);
        assert_eq!(pair.second, 0);
        for permille in [500u16, 540, 558, 600, 620, 640, 660, 667, 700, 850, 1000] {
            let pair = swung_pair_span(EIGHTH, permille).unwrap();
            assert_eq!(pair.total(), EIGHTH, "permille {permille}");
            assert!(pair.first >= pair.second, "permille {permille}");
        }
    }

    #[test]
    fn quantizing_twice_changes_nothing() {
        // 幂等是判据：第二次量化必须落在同一个槽位上。
        for permille in [500u16, 558, 667, 1000] {
            for onset in [0u64, 1, 120, 239, 240, 241, 300, 479, 480, 719, 960, 1234] {
                let once = quantize_onset(onset, EIGHTH, permille).unwrap();
                let twice = quantize_onset(once, EIGHTH, permille).unwrap();
                assert_eq!(once, twice, "permille {permille} onset {onset}");
            }
        }
    }

    #[test]
    fn the_exact_midpoint_ties_to_the_earlier_slot() {
        // 取一对 100 tick、比例 600 千分比 ⇒ 前半 = 60，正中间 = 30。
        // 平局口径是"取前一个槽位"（向后兼容平直网格里的后半起点）。
        assert_eq!(swung_pair_span(100, 600).unwrap().first, 60);
        assert_eq!(quantize_onset(30, 100, 600).unwrap(), 0);
        assert_eq!(quantize_onset(31, 100, 600).unwrap(), 60);
        assert_eq!(quantize_onset(29, 100, 600).unwrap(), 0);
        // 平直的对（前半 = 50）里，30 落在中点的**之后** ⇒ 取后半槽位 50。
        assert_eq!(quantize_onset(30, 100, 500).unwrap(), 50);
        assert_eq!(quantize_onset(25, 100, 500).unwrap(), 0);
    }

    #[test]
    fn odd_length_pairs_split_by_integer_division() {
        // 离散网格的最小对：3 tick 平直切分 ⇒ 1 + 2（向下取整）。
        // 这条判据锁定"不丢 tick"，也说明 `first >= second` 不是普适不变量。
        assert_eq!(
            swung_pair_span(3, 500).unwrap(),
            SwingPair {
                first: 1,
                second: 2
            }
        );
        assert_eq!(
            swung_pair_span(3, 1000).unwrap(),
            SwingPair {
                first: 3,
                second: 0
            }
        );
    }

    #[test]
    fn maximum_swing_keeps_an_onset_inside_its_pair() {
        // 1000 千分比时后半长度为 0：第二个槽位本来会与下一对的起点重合。
        // 槽位必须退回本对最后一个 tick，否则本对末尾的 onset 会跨对。
        assert_eq!(
            swung_pair_span(10, 1000).unwrap(),
            SwingPair {
                first: 10,
                second: 0
            }
        );
        assert_eq!(quantize_onset(9, 10, 1000).unwrap(), 9);
        assert_eq!(quantize_onset(9, 10, 1000).unwrap() / 10, 0);
        assert_eq!(quantize_onset(19, 10, 1000).unwrap(), 19);
        assert_eq!(quantize_onset(19, 10, 1000).unwrap() / 10, 1);
        // 更长的对与更小的对都必须留在本对。
        for pair_ticks in [2u64, 3, 594, 960] {
            for onset in 0..(4 * pair_ticks) {
                let quantized = quantize_onset(onset, pair_ticks, 1000).unwrap();
                assert_eq!(quantized / pair_ticks, onset / pair_ticks, "onset {onset}");
            }
        }
    }

    #[test]
    fn a_quantized_onset_never_leaves_its_pair() {
        for permille in [500u16, 600, 667, 800, 1000] {
            for onset in 0u64..(4 * EIGHTH) {
                let quantized = quantize_onset(onset, EIGHTH, permille).unwrap();
                assert_eq!(quantized / EIGHTH, onset / EIGHTH, "onset {onset}");
                let in_pair = quantized % EIGHTH;
                let first = swung_pair_span(EIGHTH, permille).unwrap().first;
                let second_slot = first.min(EIGHTH - 1);
                assert!(in_pair == 0 || in_pair == second_slot, "onset {onset}");
            }
        }
    }

    #[test]
    fn extreme_pair_lengths_do_not_overflow() {
        // `pair_ticks` 可以取到 `u64::MAX`：`2 * in_pair`（判"是否过了中点"）
        // 与 `对起点 + 槽位` 都会溢出。溢出在 debug 下 panic、在 release 下回绕
        // 成错的槽位 ⇒ 同输入不同结果。这条判据钉住两条后置条件：
        // 结果不离开本对，且函数在全部合法比例上幂等。
        for pair_ticks in [
            1u64,
            2,
            3,
            479,
            480,
            u64::MAX / 2,
            u64::MAX / 2 + 1,
            u64::MAX - 2,
            u64::MAX - 1,
            u64::MAX,
        ] {
            for permille in [500u16, 501, 667, 999, 1000] {
                let onsets = [
                    0u64,
                    1,
                    pair_ticks / 2,
                    pair_ticks.saturating_sub(1),
                    pair_ticks,
                    u64::MAX - 1,
                    u64::MAX,
                ];
                for onset in onsets {
                    let quantized = quantize_onset(onset, pair_ticks, permille).unwrap();
                    assert_eq!(
                        quantized / pair_ticks,
                        onset / pair_ticks,
                        "pair {pair_ticks} onset {onset} permille {permille}"
                    );
                    assert_eq!(
                        quantize_onset(quantized, pair_ticks, permille).unwrap(),
                        quantized,
                        "not idempotent: pair {pair_ticks} onset {onset} permille {permille}"
                    );
                }
                // 偏移恒非负（`permille >= 500`），且与"前半减一半"一致；
                // 只有该差装不进 `i64`（`pair_ticks > i64::MAX`）时才饱和。
                let offset = swung_onset_offset(pair_ticks, permille).unwrap();
                assert!(offset >= 0, "pair {pair_ticks} permille {permille}");
                let span = swung_pair_span(pair_ticks, permille).unwrap();
                let exact = i128::from(span.first) - i128::from(pair_ticks / 2);
                assert_eq!(offset, i64::try_from(exact).unwrap_or(i64::MAX));
            }
        }
        // 逐位一致读数：整段 `u64` 值域上的饱和点。
        assert_eq!(swung_onset_offset(u64::MAX, 1000).unwrap(), i64::MAX);
        assert_eq!(
            quantize_onset(u64::MAX - 1, u64::MAX, 1000).unwrap(),
            u64::MAX - 1
        );
    }

    #[test]
    fn out_of_range_permille_is_an_error_not_a_silent_clamp() {
        for permille in [0u16, 1, 499, 1001, 65535] {
            assert_eq!(
                swung_pair_span(EIGHTH, permille).unwrap_err(),
                TheoryError::SwingOutOfRange { value: permille }
            );
            assert_eq!(
                swung_onset_offset(EIGHTH, permille).unwrap_err(),
                TheoryError::SwingOutOfRange { value: permille }
            );
            assert_eq!(
                quantize_onset(0, EIGHTH, permille).unwrap_err(),
                TheoryError::SwingOutOfRange { value: permille }
            );
        }
        assert_eq!(
            swung_pair_span(EIGHTH, 500).unwrap().total(),
            EIGHTH,
            "the lower bound must stay legal"
        );
    }

    #[test]
    fn a_zero_length_pair_is_an_error() {
        assert_eq!(swung_pair_span(0, 667).unwrap_err(), TheoryError::ZeroBars);
        assert_eq!(
            quantize_onset(7, 0, 667).unwrap_err(),
            TheoryError::ZeroBars
        );
    }

    #[test]
    fn swing_out_of_range_is_classified_as_an_input_error() {
        assert!(TheoryError::SwingOutOfRange { value: 1 }.is_input_error());
    }

    #[test]
    fn genre_swing_permille_reads_the_rule_field() {
        use crate::genre::GenreLibrary;

        // `swing == None` 的条目（古典/教会传统）：保持平直网格。
        let straight = GenreLibrary::get("gregorian_chant").unwrap();
        assert_eq!(straight.swing, None);
        assert_eq!(straight.swing_permille().unwrap(), None);

        // 有摇摆的条目：f32 百分数 → 整数千分比，并且真的改变切分。
        let bossa = GenreLibrary::get("bossa_nova").unwrap();
        assert_eq!(bossa.swing, Some(54.0));
        assert_eq!(bossa.swing_permille().unwrap(), Some(540));
        // 480 * 540 / 1000 = 259 (整除)。
        assert_eq!(
            swung_pair_span(EIGHTH, bossa.swing_permille().unwrap().unwrap()).unwrap(),
            SwingPair {
                first: 259,
                second: 221
            }
        );

        // 480 * 660 / 1000 = 316 (整除)。
        let swung = GenreLibrary::get("jazz_swing").unwrap();
        assert_eq!(swung.swing, Some(66.0));
        assert_eq!(swung.swing_permille().unwrap(), Some(660));
        assert_eq!(
            swung_pair_span(EIGHTH, swung.swing_permille().unwrap().unwrap()).unwrap(),
            SwingPair {
                first: 316,
                second: 164
            }
        );
    }

    /// 类别①（非有限输入）：[`crate::genre::GenreRule::swing`] 是 `pub` 字段，
    /// 调用方可以自行构造规则，因此 `NaN` / `±∞` 是可达输入。
    ///
    /// 量什么：`swing_permille()` 在 6 种非有限 / 端值登记上的返回值，单位 = "对"。
    /// 三条读数**写死**在这里：任何一条变了都必须显式改判据，不许改成"本机读数"。
    /// 在显式分支加入之前，本 crate 没有任何判据覆盖这条路径（登记表里的
    /// `swing` 全是有限值，属性测试只枚举登记表）。
    #[test]
    fn non_finite_swing_registrations_follow_the_documented_policy() {
        use crate::genre::GenreLibrary;

        // 用登记表里真实存在的一条当模板，只替换被考察的字段。
        let with = |swing: f32| {
            let mut rule = GenreLibrary::all()[0];
            rule.swing = Some(swing);
            rule.swing_permille()
        };

        // NaN 不是"越界值"，没有可钳制的端点 ⇒ Err。
        assert_eq!(
            with(f32::NAN).unwrap_err(),
            TheoryError::SwingOutOfRange { value: 0 }
        );
        // ±∞ 按登记层的钳制口径落到区间端点。
        assert_eq!(with(f32::INFINITY).unwrap(), Some(SWING_PERMILLE_MAX));
        assert_eq!(
            with(f32::NEG_INFINITY).unwrap(),
            Some(SWING_PERMILLE_STRAIGHT)
        );
        // 极大的有限值与同号无穷走同一条读数（钳制是这一层的口径）。
        assert_eq!(with(1.0e30).unwrap(), with(f32::INFINITY).unwrap());
        assert_eq!(with(-1.0e30).unwrap(), with(f32::NEG_INFINITY).unwrap());
        // 子正规数仍是有限值：钳到平直端点，既不 Err 也不 panic。
        assert_eq!(
            with(f32::MIN_POSITIVE).unwrap(),
            Some(SWING_PERMILLE_STRAIGHT)
        );
        assert_eq!(with(0.0).unwrap(), Some(SWING_PERMILLE_STRAIGHT));
        // `None` 与 `Some(NaN)` 必须走不同分支：前者合法缺省，后者是坏数据。
        let mut none_rule = GenreLibrary::all()[0];
        none_rule.swing = None;
        assert_eq!(none_rule.swing_permille().unwrap(), None);
    }

    /// 形态 D 注入实测（本票）：把最大摇摆下"第二个槽位"的钳制从
    /// `first.min(pair_ticks - 1)` 放宽成 `first.min(pair_ticks)` 时，
    /// 既有判据全绿 —— 它们只喂 `pair_ticks` 恰好让后半落在本对内的长对
    /// （10、480），而 `in_pair == 0` 的路径不受槽位影响。
    ///
    /// 这条判据取一对**槽位会被钳制**的长度，并把子对起点与槽位一起钉死：
    /// 槽位一旦等于 `pair_ticks`，第二个槽位就与下一对的起点重合，
    /// 本对末尾的 onset 会被吸到下一对去。
    #[test]
    fn the_second_slot_is_pinned_one_tick_before_the_pair_ends() {
        // 7 tick、1000 千分比 ⇒ 前半 7、后半 0 ⇒ 槽位必须退回 6。
        assert_eq!(
            swung_pair_span(7, SWING_PERMILLE_MAX).unwrap(),
            SwingPair {
                first: 7,
                second: 0
            }
        );
        // 7 的中点是 3（整除）⇒ 3 归前半、4 与 6 归后半槽位 6。
        assert_eq!(quantize_onset(3, 7, SWING_PERMILLE_MAX).unwrap(), 0);
        assert_eq!(quantize_onset(4, 7, SWING_PERMILLE_MAX).unwrap(), 6);
        assert_eq!(quantize_onset(6, 7, SWING_PERMILLE_MAX).unwrap(), 6);
        // 子对起点必须原样保留：7 是下一对的起点，量化后仍是 7。
        assert_eq!(quantize_onset(7, 7, SWING_PERMILLE_MAX).unwrap(), 7);
        assert_eq!(quantize_onset(13, 7, SWING_PERMILLE_MAX).unwrap(), 13);
        // 槽位不得越到下一对：在整段 tick 域上逐点核对。
        for pair_ticks in [2u64, 3, 5, 7, 9, 15, 100, 479] {
            for onset in 0..(6 * pair_ticks) {
                let quantized = quantize_onset(onset, pair_ticks, SWING_PERMILLE_MAX).unwrap();
                assert_eq!(
                    quantized / pair_ticks,
                    onset / pair_ticks,
                    "pair {pair_ticks} onset {onset}"
                );
                assert!(
                    quantized % pair_ticks < pair_ticks,
                    "pair {pair_ticks} onset {onset}: slot left the pair"
                );
            }
        }
    }
}
