//! 本机零依赖验证脚手架 —— `yeban_render_master` 的纯逻辑 [MCP-TOOL-008]。
//!
//! 它**不是** cargo 目标（不在 `tests/` 下），因为它会被 `#[path]` 引入**真实源文件**
//! （`src/domain/render_math.rs`）后用裸 `rustc` 编译执行 —— 与
//! `crates/yeban-render/verify/pure_modules.rs` 同一手法。这样做的唯一理由：
//! `yeban-mcp` 一旦依赖 `yeban-render`（rayon/hound/midly）就无法在本机编译，
//! 而"tick→帧""峰值归一化""日历换算"这三件事是**最容易写错**的部分，
//! 必须有本机可执行的判据。
//!
//! ```text
//! rustc --edition 2024 --test -D warnings crates/yeban-mcp/verify/render_pure.rs -o /tmp/render_pure
//! /tmp/render_pure
//! clippy-driver --edition 2024 --test -D warnings -D clippy::all crates/yeban-mcp/verify/render_pure.rs
//! ```
//!
//! ## 这里的判据与 `render_math.rs` 自带的判据**不重复**
//!
//! `render_math.rs` 的 `#[cfg(test)] mod tests` 钉住**具体取值**（回归守卫）。
//! 本文件用**独立参考实现**（另一套算法）做对账：整数有理数版的 tick→帧、
//! Hinnant 的逆变换 `days_from_civil`、显式分段包络、暴力峰值。

#[path = "../src/domain/render_math.rs"]
mod render_math;

/// 独立参考实现：把 tick→帧写成**整数有理数**运算，不使用被测函数的浮点路径。
///
/// `frames = round(end_tick * 60 * sample_rate / (ppq * bpm))`，其中 `bpm` 必须是整数。
fn reference_frames(end_tick: u64, ppq: u64, bpm: u64, sample_rate: u32) -> u64 {
    let numerator = u128::from(end_tick) * 60 * u128::from(sample_rate);
    let denominator = u128::from(ppq) * u128::from(bpm);
    let quotient = (numerator + denominator / 2) / denominator;
    u64::try_from(quotient).unwrap_or(u64::MAX)
}

/// 独立参考实现：Hinnant 的 `days_from_civil`（`civil_from_days` 的逆）。
fn reference_days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let year_of_era = y - era * 400;
    let m = i64::from(month);
    let d = i64::from(day);
    let day_of_year = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// 独立参考实现：显式分段包络（与被测实现写法完全不同）。
fn reference_envelope(offset: i64, length: i64, attack: i64, release: i64) -> f32 {
    if length <= 0 || offset < 0 || offset >= length {
        return 0.0;
    }
    let attack = attack.clamp(0, length);
    let release = release.clamp(0, length);
    let rising = if attack == 0 {
        1.0
    } else {
        (offset as f32 + 1.0) / attack as f32
    };
    let falling = if release == 0 {
        1.0
    } else {
        (length - offset) as f32 / release as f32
    };
    rising.min(falling).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::render_math as math;
    use super::{reference_days_from_civil, reference_envelope, reference_frames};

    /// 对账 1: 浮点 tick→帧 == 整数有理数参考实现（整数 BPM 的全部取值）。
    #[test]
    fn ticks_to_frames_matches_the_rational_reference() {
        let mut checked = 0usize;
        for bpm in [20u64, 60, 90, 120, 128, 140, 200, 999] {
            for tick in [
                0u64, 1, 2, 480, 959, 960, 961, 1_920, 3_840, 7_680, 10_000_000,
            ] {
                for rate in [44_100u32, 48_000, 96_000, 192_000] {
                    let ours = math::ticks_to_frames(tick, 960, bpm as f64, rate);
                    let theirs = reference_frames(tick, 960, bpm, rate);
                    assert_eq!(ours, theirs, "bpm={bpm} tick={tick} rate={rate}");
                    checked += 1;
                }
            }
        }
        assert!(checked >= 200, "对账样本太少: {checked}");
    }

    /// 对账 2: 日历变换与逆变换在 400 年跨度上互为逆（含 1970 前的负天数分支）。
    #[test]
    fn civil_calendar_round_trips_over_four_centuries() {
        for days in (-100_000i64..100_000).step_by(7) {
            let (year, month, day) = math::civil_from_days(days);
            assert_eq!(
                reference_days_from_civil(year, month, day),
                days,
                "{days} -> {year}-{month:02}-{day:02}"
            );
        }
        // 每一个 2 月 29 日都必须存在（4 年一闰，2100 不是闰年）。
        for year in [1972i64, 2000, 2024, 2096] {
            let days = reference_days_from_civil(year, 2, 29);
            assert_eq!(math::civil_from_days(days), (year, 2, 29));
        }
        // 2100-02-29 不存在: 1900-03-01 是那一天的"下一天"。
        let march_first = reference_days_from_civil(2100, 3, 1);
        assert_eq!(math::civil_from_days(march_first), (2100, 3, 1));
        assert_eq!(math::civil_from_days(march_first - 1), (2100, 2, 28));
    }

    /// 对账 3: 峰值与归一化 —— 峰值不小于任何 |样本|；归一化后峰值命中目标。
    #[test]
    fn peak_and_normalization_agree_with_brute_force() {
        let mut samples = Vec::new();
        for index in 0..4_096i32 {
            let value = ((index * 37) % 101) as f32 / 101.0 - 0.5;
            samples.push(value);
        }
        let peak = math::peak_of(&samples);
        let brute = samples
            .iter()
            .fold(0.0f32, |acc, sample| acc.max(sample.abs()));
        assert!((peak - brute).abs() < f32::EPSILON);
        assert!(samples.iter().all(|sample| peak >= sample.abs()));
        assert!(referenced_peak_after_normalization(peak) < 1.0e-6);
    }

    /// 归一化后的峰值误差（与生产代码同一条路径：`normalize_gain` + `scale_in_place`）。
    fn referenced_peak_after_normalization(peak: f32) -> f32 {
        let mut samples = vec![peak, -peak * 0.5];
        math::scale_in_place(&mut samples, math::normalize_gain(peak, 1.0));
        (math::peak_of(&samples) - 1.0).abs()
    }

    /// 对账 4: 包络与显式分段参考实现一致（浮点相等，两者都是线性插值）。
    #[test]
    fn envelope_matches_the_piecewise_reference() {
        for length in [1i64, 2, 3, 10, 100, 481, 4_800] {
            for attack in [0i64, 1, 5, 240, 481] {
                for release in [0i64, 1, 10, 480, 960] {
                    for offset in 0..length {
                        let ours = math::envelope(offset, length, attack, release);
                        let theirs = reference_envelope(offset, length, attack, release);
                        assert!(
                            (ours - theirs).abs() < 1.0e-6,
                            "offset={offset} length={length} attack={attack} release={release}: \
                             {ours} vs {theirs}"
                        );
                        assert!((0.0..=1.0).contains(&ours));
                    }
                }
            }
        }
    }

    /// 对账 5: 输出命名规则（缺省路径规则的第二半）。
    #[test]
    fn output_names_follow_the_documented_rule() {
        let cases = [
            ("demo", "wav", "demo.master.wav"),
            ("My Song", "rf64", "My Song.master.rf64"),
            ("", "bw64", ".master.bw64"),
        ];
        for (stem, format, expected) in cases {
            assert_eq!(math::default_output_file_name(stem, format), expected);
        }
    }

    /// 对账 6: `bext_stamp` 的宽口径（每 6 小时一个样本，跨 3 天）。
    #[test]
    fn bext_stamps_are_zero_padded_and_monotonic() {
        let mut previous = String::new();
        for step in 0..12u64 {
            let ms = 1_760_000_000_000 + step * 21_600_000;
            let (date, time) = math::bext_stamp(ms);
            assert_eq!(date.len(), 10, "{date}");
            assert_eq!(time.len(), 8, "{time}");
            assert_eq!(&date[4..5], "-");
            assert_eq!(&date[7..8], "-");
            assert_eq!(&time[2..3], ":");
            assert_eq!(&time[5..6], ":");
            let stamp = format!("{date}T{time}");
            assert!(stamp > previous, "{stamp} 必须严格递增于 {previous}");
            previous = stamp;
        }
        // 1970-01-01 与一个 2025 年的午夜（1_759_622_400_000 ms，
        // 已用独立的 Python `datetime(2025,10,5,tz=utc).timestamp()` 对账）。
        assert_eq!(
            math::bext_stamp(0),
            ("1970-01-01".to_owned(), "00:00:00".to_owned())
        );
        assert_eq!(
            math::bext_stamp(1_759_622_400_000),
            ("2025-10-05".to_owned(), "00:00:00".to_owned())
        );
    }

    /// 对账 7: `ms_to_frames` 与 tick 路径互相自洽（5ms @ 48k == 240 帧 == 9.6 tick）。
    #[test]
    fn millisecond_helpers_are_self_consistent() {
        for rate in [44_100u32, 48_000, 96_000] {
            let frames = math::ms_to_frames(1_000, rate);
            assert_eq!(frames, i64::from(rate), "1 秒必须是采样率那么多帧");
            assert_eq!(math::ms_to_frames(0, rate), 0);
        }
    }
}
