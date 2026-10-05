//! 属性测试（`proptest`）：给纯逻辑层的判据加一层"随机输入"覆盖 [AGENTS.md §3 DoD 2]。
//!
//! 为什么把属性测试单独放一个文件：本 crate 的 [`crate::limits`] 与 [`crate::duration`]
//! 是**零第三方依赖**的，因此可以在本机用 `rustc --edition 2024 --test` 单独编译执行
//! （见 notes §7）。若把 `proptest` 写进那两个文件的 `#[cfg(test)]` 块，它们就再也
//! 无法脱离依赖树编译，本机可验证的面积会**变小**。所以属性测试住在这里，只由
//! `cargo test` 拉起。
//!
//! 三条属性分别对应三条硬约束：
//! 1. 尺寸算术：`frames × channels` 要么精确，要么明确溢出 —— **绝不回绕**；
//! 2. 长度契约：任何采样率比例下区间都自洽，且"恰好越界"必须被拒（判据不能是空的）；
//! 3. 时长对账：对称、只把相等判为 `Exact`、`is_reconciled` 等价于"差在容差内"。

use proptest::prelude::*;

use crate::duration::{self, Reconciliation};
use crate::limits;

/// 三个规范采样率 + 任意合理采样率，用来覆盖"规范路径"与"任意路径"。
fn any_rate() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(44_100u32),
        Just(48_000u32),
        Just(96_000u32),
        1u32..=limits::DEFAULT_MAX_SAMPLE_RATE,
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn interleaved_sample_count_is_exact_unless_it_overflows(
        frames in any::<u64>(),
        channels in any::<u16>(),
    ) {
        let exact = u128::from(frames) * u128::from(channels);
        match limits::interleaved_samples(frames, channels) {
            Ok(value) => prop_assert_eq!(u128::from(value), exact),
            Err(_) => prop_assert!(exact > u128::from(u64::MAX)),
        }
    }

    #[test]
    fn layout_budget_admits_exactly_the_documented_envelope(
        channels in any::<u16>(),
        sample_rate in any::<u32>(),
        frames in 0u64..=40_000_000u64,
    ) {
        let budget = limits::PcmBudget::default();
        let outcome = limits::check_layout(channels, sample_rate, frames, &budget);
        // 四道闸门的合取；`&&` 的短路求值保证后面的除法/乘法只在采样率非 0 时求值。
        let expected_ok = (1..=budget.max_channels).contains(&channels)
            && (1..=budget.max_sample_rate).contains(&sample_rate)
            && u128::from(frames) <= u128::from(budget.max_duration_secs) * u128::from(sample_rate)
            && u128::from(frames) * u128::from(channels)
                <= u128::from(budget.interleaved_samples_limit());
        prop_assert_eq!(outcome.is_ok(), expected_ok);
        // 判据 (内存上界): 预算通过 ⇒ PCM 字节数 ≤ `max_pcm_bytes`。
        // 这是"解码缓冲不会超过预算"的机械上界（与输入文件长度无关）。
        if outcome.is_ok() {
            prop_assert!(
                u128::from(frames) * u128::from(channels) * 4 <= u128::from(budget.max_pcm_bytes)
            );
        }
    }

    #[test]
    fn pcm_size_conversion_is_exact_or_refused_never_wrapped(
        seconds in any::<u64>(),
        sample_rate in any::<u32>(),
        channels in any::<u16>(),
    ) {
        let exact = u128::from(seconds) * u128::from(sample_rate) * u128::from(channels) * 4;
        match limits::pcm_bytes_for(seconds, sample_rate, channels) {
            Some(bytes) => prop_assert_eq!(u128::from(bytes), exact),
            None => prop_assert!(exact == 0 || exact > u128::from(u64::MAX)),
        }
    }

    #[test]
    fn length_contract_is_self_consistent_and_not_vacuous(
        input_frames in 0u64..=5_000_000u64,
        in_rate in any_rate(),
        out_rate in any_rate(),
    ) {
        let contract = limits::resample_len_contract(input_frames, out_rate, in_rate)
            .expect("non-zero rates always yield a contract");
        prop_assert!(contract.min <= contract.ideal_floor);
        prop_assert!(contract.ideal_ceil <= contract.max);
        prop_assert!(contract.min <= contract.max);
        prop_assert!(
            limits::check_resampled_len(input_frames, out_rate, in_rate, contract.ideal_floor)
                .is_ok()
        );
        prop_assert!(
            limits::check_resampled_len(input_frames, out_rate, in_rate, contract.ideal_ceil)
                .is_ok()
        );
        // 区间之外必须被拒绝 —— 否则这条判据是空的。
        if contract.min > 0 {
            prop_assert!(
                limits::check_resampled_len(input_frames, out_rate, in_rate, contract.min - 1)
                    .is_err()
            );
        }
        if contract.max < u64::MAX {
            prop_assert!(
                limits::check_resampled_len(input_frames, out_rate, in_rate, contract.max + 1)
                    .is_err()
            );
        }
    }

    #[test]
    fn reconciliation_is_symmetric_and_only_exact_on_equality(
        declared in 1u64..=10_000_000u64,
        decoded in 0u64..=10_000_000u64,
        tolerance in 0u64..=1_000u64,
    ) {
        let forward = duration::reconcile(Some(declared), decoded, tolerance);
        let backward = duration::reconcile(Some(decoded), declared, tolerance);
        prop_assert_eq!(forward.is_reconciled(), backward.is_reconciled());
        prop_assert_eq!(
            forward.is_reconciled(),
            declared.abs_diff(decoded) <= tolerance
        );
        prop_assert_eq!(
            matches!(forward, Reconciliation::Exact),
            declared == decoded
        );
        prop_assert_eq!(
            duration::reconcile(None, decoded, tolerance),
            Reconciliation::DeclaredUnknown
        );
    }
}
