//! `yeban_render_master` 的**零第三方依赖纯逻辑**：时间轴换算、峰值归一化、
//! 包络、日历换算与输出命名。
//!
//! ## 为什么单独一个模块
//!
//! 本 crate 一旦依赖 `yeban-render`（rayon/hound/midly）就不能在本机编译
//! （`run-gates.sh crate yeban-mcp` 的纪律），而"最容易写错、又完全不依赖任何第三方
//! crate"的正是这些算术。把它们抽到这里，就能用
//!
//! ```text
//! rustc --edition 2024 --test -D warnings crates/yeban-mcp/verify/render_pure.rs
//! ```
//!
//! 在本机**真的执行**（与 `crates/yeban-render/verify/pure_modules.rs` 同一手法：
//! `#[path]` 引入**真实源文件**，不是抄一份会漂移的副本）。
//!
//! ## 这一层允许什么运算
//!
//! 只用 `+ - * /` 与 `f64::round`。[IEEE 754] 把 `round` 指定为
//! `roundToIntegralTiesAway` —— 它不是超越函数，在所有符合 IEEE 754 的目标上逐位相同。
//! 需要 `sin`/`powf`/`cos` 的地方（振荡器、等程律频率、等功率声相）留在
//! [`super::render`]，统一走 `libm` [ARCH-DET-001]。
//!
//! [IEEE 754]: https://en.wikipedia.org/wiki/IEEE_754

/// tick → 帧（四舍五入到最近帧）。
///
/// `seconds = tick / ppq * 60 / bpm`，再乘采样率。病态输入（`ppq == 0`、`bpm <= 0`、
/// 非有限 `bpm`、采样率 0、非有限结果）一律返回 `0` —— 调用方随后会把它当成
/// "没有可渲染内容"（`RENDER_FAILED`），而不是悄悄渲染一段垃圾长度。
#[must_use]
pub fn ticks_to_frames(end_tick: u64, ppq: u64, bpm: f64, sample_rate: u32) -> u64 {
    if ppq == 0 || !bpm.is_finite() || bpm <= 0.0 || sample_rate == 0 {
        return 0;
    }
    let seconds = (end_tick as f64) * 60.0 / (ppq as f64) / bpm;
    let frames = (seconds * f64::from(sample_rate)).round();
    if !frames.is_finite() || frames <= 0.0 {
        return 0;
    }
    if frames >= u64::MAX as f64 {
        return u64::MAX;
    }
    frames as u64
}

/// 帧 → tick（**向上取整**）。
///
/// 这是 [`ticks_to_frames`] 的反函数，用在"把一段音频的全部帧数换算成摆放时值"这件事上
/// （`yeban_import_audio` 的 `durationTicks` 缺省值）：**向上**取整保证素材尾部的
/// 不足一 tick 不会被切掉 —— 向下取整会让最后几帧永远听不到，那是一种静默的丢音。
///
/// `seconds = frames / sample_rate`，再换算成 tick：`frames * ppq * bpm / (sample_rate * 60)`。
/// 病态输入（`ppq == 0`、`bpm <= 0`、非有限 `bpm`、采样率 0、非有限结果）返回 `None` ——
/// 与 [`ticks_to_frames`] 返回 `0` 不同，这里**没有**一个安全的哨兵值：
/// [`yeban_model::ClipPlacement::validate`] 拒绝零时值的摆放，因此调用方必须明确拒绝，
/// 而不是悄悄写一个会被模型层拒掉的 `0`。
///
/// 结果的下界是 1（0 帧素材也不产出 0 tick）。
#[must_use]
pub fn frames_to_ticks_ceil(frames: u64, ppq: u64, bpm: f64, sample_rate: u32) -> Option<u64> {
    if ppq == 0 || !bpm.is_finite() || bpm <= 0.0 || sample_rate == 0 {
        return None;
    }
    let ticks = (frames as f64) * (ppq as f64) * bpm / (f64::from(sample_rate) * 60.0);
    if !ticks.is_finite() || ticks < 0.0 {
        return None;
    }
    if ticks >= u64::MAX as f64 {
        return Some(u64::MAX);
    }
    Some((ticks.ceil() as u64).max(1))
}

/// 峰值（最大绝对值）。
///
/// 非有限样本**不计入**：`NaN` 参与比较会让"全零 ⇒ 峰值 0"这条边界失效，
/// 而"全零信号"正是 `normalize` 必须如实处理的边界。
#[must_use]
pub fn peak_of(samples: &[f32]) -> f32 {
    let mut peak = 0.0f32;
    for &sample in samples {
        if sample.is_finite() {
            let magnitude = sample.abs();
            if magnitude > peak {
                peak = magnitude;
            }
        }
    }
    peak
}

/// 峰值归一化的增益：`target / peak`。
///
/// `peak <= 0`（**全零信号**）或任一输入非有限 ⇒ `1.0`（即"不改动任何样本"）。
/// 这不是"归一化失败"，是归一化对全零信号的**定义**：把 0 放大到任何目标都是
/// 在制造一个不存在的信号。
#[must_use]
pub fn normalize_gain(peak: f32, target: f32) -> f32 {
    if !peak.is_finite() || !target.is_finite() || peak <= 0.0 || target <= 0.0 {
        return 1.0;
    }
    target / peak
}

/// 原地乘增益。
pub fn scale_in_place(samples: &mut [f32], gain: f32) {
    for sample in samples.iter_mut() {
        *sample *= gain;
    }
}

/// 线性起音 / 释音包络（`(0, 1]`，区间外为 `0`）。
///
/// `offset` 是相对音符起点的帧偏移，`length` 是音符总帧数。
/// 起音与释音都是**线性**的：不需要超越函数，且在任何采样率下逐位可复现。
///
/// 取 `gain = min((offset+1)/attack, remaining/release, 1)`，其中
/// `attack = min(attack_frames, length)`、`release = min(release_frames, length)`。
/// 用 `offset+1` 而不是 `offset`：**极短音符**（`length == 1`）如果不这样写，
/// 起音系数恒为 0, 整个音符会被静音 —— 那不是"包络"，那是丢音。
/// `length` 落在分母上时 `release` 也被夹到 `length`，因此增益恒在 `(0, 1]`。
#[must_use]
pub fn envelope(offset: i64, length: i64, attack_frames: i64, release_frames: i64) -> f32 {
    if length <= 0 || offset < 0 || offset >= length {
        return 0.0;
    }
    let attack = attack_frames.clamp(0, length);
    let release = release_frames.clamp(0, length);
    let rising = if attack == 0 {
        1.0
    } else {
        (offset + 1) as f32 / attack as f32
    };
    let remaining = length - offset; // >= 1
    let falling = if release == 0 {
        1.0
    } else {
        remaining as f32 / release as f32
    };
    rising.min(falling).min(1.0)
}

/// 毫秒 → 帧数（起音/释音时长用），**半值向上**取整。
///
/// 整数运算而不是 `(sample_rate / 1000) * ms`：后者在 44.1 kHz 上会把
/// 1 秒算成 44 000 帧（差 100 帧），而"1 秒 = 采样率那么多帧"是母带长度的基线。
#[must_use]
pub fn ms_to_frames(ms: u32, sample_rate: u32) -> i64 {
    (i64::from(ms) * i64::from(sample_rate) + 500) / 1000
}

/// 音符的绝对帧区间 `[start, end)`（半开）。
///
/// 时钟口径：`placement.start_tick + note.start_tick + micro_timing_ticks`，
/// 三者相加后**夹到 `>= 0`**（微时值可以让第一个音符落到时间轴之前，
/// 那时它应当从第 0 帧开始而不是回绕成天文数字）。
#[must_use]
pub fn note_frame_span(
    start_tick: u64,
    duration_ticks: u64,
    micro_timing_ticks: i64,
    ppq: u64,
    bpm: f64,
    sample_rate: u32,
) -> (i64, i64) {
    let shifted = i128::from(start_tick) + i128::from(micro_timing_ticks);
    let clamped = if shifted <= 0 {
        0
    } else {
        u64::try_from(shifted).unwrap_or(u64::MAX)
    };
    let start = ticks_to_frames(clamped, ppq, bpm, sample_rate);
    let end_tick = clamped.saturating_add(duration_ticks.max(1));
    let end = ticks_to_frames(end_tick, ppq, bpm, sample_rate);
    (saturating_i64(start), saturating_i64(end.max(start + 1)))
}

/// `u64` → `i64` 的饱和转换（帧数上限远小于 `i64::MAX`，这里是防御性写法）。
#[must_use]
fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// `days`（1970-01-01 起）→ 民用日期 `(年, 月, 日)`。
///
/// Howard Hinnant 的 `civil_from_days`：纯整数运算、无时区数据库、无闰年表，
/// 因此不需要任何第三方日期库（也就没有多一份依赖要审计）。
#[must_use]
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    }) as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Unix 毫秒 → UTC 民用时间 `(年, 月, 日, 时, 分, 秒)`。
#[must_use]
pub fn civil_from_unix_ms(ms: u64) -> (i64, u32, u32, u32, u32, u32) {
    let total_seconds = saturating_i64(ms / 1000);
    let days = total_seconds.div_euclid(86_400);
    let remainder = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        (remainder / 3_600) as u32,
        ((remainder % 3_600) / 60) as u32,
        (remainder % 60) as u32,
    )
}

/// BWF `OriginationDate` / `OriginationTime` 的 ASCII 口径 `(YYYY-MM-DD, HH:MM:SS)`。
#[must_use]
pub fn bext_stamp(unix_ms: u64) -> (String, String) {
    let (year, month, day, hour, minute, second) = civil_from_unix_ms(unix_ms);
    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!("{hour:02}:{minute:02}:{second:02}"),
    )
}

/// 缺省输出文件名规则：`<工程文件 stem>.master.<format>`。
///
/// 规则只有一处（这里），因为它同时出现在 `dryRun` 预览与真调用里 ——
/// 两份实现必然漂移，而"预览的路径不是真写出的路径"是最难查的一类缺陷。
#[must_use]
pub fn default_output_file_name(stem: &str, format: &str) -> String {
    format!("{stem}.master.{format}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 1: 时间轴换算在整数关系上精确（120 BPM / 960 PPQ ⇒ 1920 tick = 1 秒）。
    #[test]
    fn one_second_of_ticks_maps_to_the_sample_rate() {
        assert_eq!(ticks_to_frames(1_920, 960, 120.0, 48_000), 48_000);
        assert_eq!(ticks_to_frames(1_920, 960, 120.0, 44_100), 44_100);
        assert_eq!(ticks_to_frames(960, 960, 120.0, 48_000), 24_000);
        assert_eq!(ticks_to_frames(2_048, 960, 128.0, 48_000), 48_000);
        // 1 tick @ 20 BPM = 3s / 960 = 0.003125s ⇒ 150 帧。
        assert_eq!(ticks_to_frames(1, 960, 20.0, 48_000), 150);
        assert_eq!(ticks_to_frames(0, 960, 120.0, 48_000), 0);
    }

    /// 判据 2: 病态输入返回 0，而不是 panic / 天文数字。
    #[test]
    fn degenerate_tempo_inputs_yield_zero_frames() {
        for (ppq, bpm, rate) in [
            (0u64, 120.0f64, 48_000u32),
            (960, 0.0, 48_000),
            (960, -120.0, 48_000),
            (960, f64::NAN, 48_000),
            (960, f64::INFINITY, 48_000),
            (960, 120.0, 0),
        ] {
            assert_eq!(
                ticks_to_frames(1_000, ppq, bpm, rate),
                0,
                "{ppq}/{bpm}/{rate}"
            );
        }
    }

    /// 判据 3: 帧 → tick 与 tick → 帧在整数关系上互逆，且**向上**取整。
    ///
    /// 这条判据的牙齿：把 `ceil` 换成 `floor`/`round`，下列"不足一 tick"的两行立刻变红
    /// （那些帧会落在一整个 tick 之外，等于被硬切掉）。
    #[test]
    fn frames_round_trip_through_ticks_and_never_cut_a_partial_tick() {
        // 1920 tick = 1 秒 @120 BPM / 960 PPQ ⇒ 48000 帧正好回到 1920 tick。
        assert_eq!(
            frames_to_ticks_ceil(48_000, 960, 120.0, 48_000),
            Some(1_920)
        );
        assert_eq!(frames_to_ticks_ceil(24_000, 960, 120.0, 48_000), Some(960));
        // 4800 帧 = 100 ms @128 BPM ⇒ 204.8 tick ⇒ 向上取整 205。
        assert_eq!(frames_to_ticks_ceil(4_800, 960, 128.0, 48_000), Some(205));
        // 小数部分 < 0.5 的一例：4300 帧 @48k = 0.089583 s ⇒ 183.466… tick。
        // `ceil` ⇒ 184；`round` 会给出 183（丢掉尾部不足半 tick 的那部分）。
        // 这一行专门让"把 ceil 换成 round"变红。
        assert_eq!(frames_to_ticks_ceil(4_300, 960, 128.0, 48_000), Some(184));
        // 0 帧仍给 1 tick（0 会被模型层拒绝）。
        assert_eq!(frames_to_ticks_ceil(0, 960, 120.0, 48_000), Some(1));
        // 1 帧远小于一 tick ⇒ 向上取整给它完整的一 tick，而不是 0。
        assert_eq!(frames_to_ticks_ceil(1, 960, 120.0, 48_000), Some(1));
        // 44.1 kHz 上的一秒：44100 帧 ⇒ 1920 tick（@120 BPM），浮点不引入 off-by-one。
        assert_eq!(
            frames_to_ticks_ceil(44_100, 960, 120.0, 44_100),
            Some(1_920)
        );
        // 互逆：帧 → tick → 帧 不得少于原帧数（向上取整的语义）。
        for frames in [1_u64, 7, 44_100, 48_000, 96_001] {
            let ticks = frames_to_ticks_ceil(frames, 960, 120.0, 48_000).expect("合法速度");
            assert!(
                ticks_to_frames(ticks, 960, 120.0, 48_000) >= frames,
                "{frames} 帧经 {ticks} tick 回来变少了"
            );
        }
    }

    /// 判据 4: 病态速度参数返回 `None`（不猜一个会被模型层拒绝的 0）。
    #[test]
    fn degenerate_tempo_inputs_yield_no_ticks() {
        for (ppq, bpm, rate) in [
            (0u64, 120.0f64, 48_000u32),
            (960, 0.0, 48_000),
            (960, -120.0, 48_000),
            (960, f64::NAN, 48_000),
            (960, f64::INFINITY, 48_000),
            (960, 120.0, 0),
        ] {
            assert_eq!(
                frames_to_ticks_ceil(4_800, ppq, bpm, rate),
                None,
                "{ppq}/{bpm}/{rate}"
            );
        }
    }

    /// 判据 3: 峰值忽略 `NaN`（"全零 ⇒ 0"这条边界不能被 `NaN` 破坏）。
    #[test]
    fn peak_ignores_non_finite_samples() {
        assert_eq!(peak_of(&[]), 0.0);
        assert_eq!(peak_of(&[0.0, -0.0]), 0.0);
        assert_eq!(peak_of(&[0.25, -0.5, f32::NAN]), 0.5);
        assert_eq!(peak_of(&[f32::INFINITY, 0.25]), 0.25);
        assert_eq!(peak_of(&[-1.5]), 1.5);
    }

    /// 判据 4: 归一化增益只在"真有信号"时改变样本；全零信号是恒等。
    #[test]
    fn normalize_of_silence_is_identity() {
        assert_eq!(normalize_gain(0.0, 1.0), 1.0);
        assert_eq!(normalize_gain(-0.0, 1.0), 1.0);
        assert_eq!(normalize_gain(f32::NAN, 1.0), 1.0);
        assert_eq!(normalize_gain(0.5, f32::NAN), 1.0);
        assert_eq!(normalize_gain(0.5, 0.0), 1.0);
        assert_eq!(normalize_gain(0.25, 1.0), 4.0);
        assert_eq!(normalize_gain(0.25, 0.5), 2.0);
    }

    /// 判据 5: 归一化后峰值命中目标（f32 乘法的舍入误差上界）。
    #[test]
    fn normalization_hits_the_target_within_one_ulp_scale() {
        let mut samples = vec![0.3f32, -0.7, 0.1, -0.2];
        let peak = peak_of(&samples);
        let gain = normalize_gain(peak, 1.0);
        scale_in_place(&mut samples, gain);
        let achieved = peak_of(&samples);
        let error = (achieved - 1.0f32).abs();
        assert!(
            error < 1.0e-6,
            "归一化后峰值 {achieved} 偏离 1.0 达 {error}"
        );
        // 全零信号: 归一化**不改动**任何样本。
        let mut silence = vec![0.0f32; 4];
        let silence_gain = normalize_gain(peak_of(&silence), 1.0);
        scale_in_place(&mut silence, silence_gain);
        assert!(silence.iter().all(|&sample| sample == 0.0));
    }

    /// 判据 6: 包络端点与边界（起音段线性上升、释音段线性下降、区间外为 0，
    /// 且**极短音符不会被静音**）。
    #[test]
    fn envelope_is_linear_and_bounded() {
        assert_eq!(envelope(-1, 100, 10, 10), 0.0);
        assert_eq!(envelope(100, 100, 10, 10), 0.0);
        // 第一帧就有非零增益 (offset+1): 首帧恰好为 0 的写法会把 1 帧音符整段静音。
        assert_eq!(envelope(0, 100, 10, 10), 0.1);
        assert_eq!(envelope(5, 100, 10, 10), 0.6);
        assert_eq!(envelope(10, 100, 10, 10), 1.0);
        assert_eq!(envelope(50, 100, 10, 10), 1.0);
        assert_eq!(envelope(95, 100, 10, 10), 0.5);
        assert_eq!(envelope(99, 100, 10, 10), 0.1);
        // 极短音符: 起音 + 释音被夹到 length ⇒ 恒在 (0, 1], 且**有声**。
        assert_eq!(envelope(0, 1, 10, 10), 1.0);
        for offset in 0..3 {
            let gain = envelope(offset, 3, 10, 10);
            assert!(gain > 0.0 && gain <= 1.0, "offset {offset} 增益 {gain}");
        }
        // 零长度音符没有声音。
        assert_eq!(envelope(0, 0, 10, 10), 0.0);
    }

    /// 判据 7: 微时值可以把音符推到时间轴之前，那时它从第 0 帧开始。
    #[test]
    fn micro_timing_never_produces_a_negative_frame() {
        let (start, end) = note_frame_span(0, 960, -240, 960, 120.0, 48_000);
        assert_eq!(start, 0);
        assert!(end > start);
        let (start, end) = note_frame_span(960, 960, -240, 960, 120.0, 48_000);
        assert_eq!(
            start,
            i64::try_from(ticks_to_frames(720, 960, 120.0, 48_000)).expect("小整数")
        );
        assert!(end > start);
        // 微时值把音符整体推后。
        let (shifted, _) = note_frame_span(960, 960, 240, 960, 120.0, 48_000);
        assert_eq!(
            shifted,
            i64::try_from(ticks_to_frames(1_200, 960, 120.0, 48_000)).expect("小整数")
        );
    }

    /// 判据 8: 日历换算对已知历元精确（含闰年与负 time_t 分支的整数路径）。
    #[test]
    fn civil_calendar_matches_known_epochs() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // 2024-01-01
        assert_eq!(civil_from_days(20_089), (2025, 1, 1));
        assert_eq!(civil_from_unix_ms(0), (1970, 1, 1, 0, 0, 0));
        // 1_760_000_000_000 ms = 2025-10-09T08:53:20Z（本仓库夹具用的注入时钟，
        // 已用独立的 Python `datetime.fromtimestamp(..., utc)` 对账）。
        assert_eq!(
            civil_from_unix_ms(1_760_000_000_000),
            (2025, 10, 9, 8, 53, 20)
        );
        assert_eq!(
            bext_stamp(1_760_000_000_000),
            ("2025-10-09".to_owned(), "08:53:20".to_owned())
        );
        // 闰年 2 月 29 日（2024-01-01 = day 19723，+31 天到 2 月 1 日 ⇒ 2024-02-29 = 19782）。
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    /// 判据 9: 缺省输出文件名规则。
    #[test]
    fn default_output_name_is_stem_dot_master_dot_format() {
        assert_eq!(default_output_file_name("demo", "wav"), "demo.master.wav");
        assert_eq!(default_output_file_name("demo", "rf64"), "demo.master.rf64");
        assert_eq!(
            default_output_file_name("no-ext", "bw64"),
            "no-ext.master.bw64"
        );
    }

    /// 判据 10: `ms_to_frames` 在整数毫秒上精确（半值向上），且 1 秒恒等于采样率。
    #[test]
    fn milliseconds_become_frames_exactly() {
        assert_eq!(ms_to_frames(5, 48_000), 240);
        assert_eq!(ms_to_frames(10, 48_000), 480);
        assert_eq!(ms_to_frames(5, 44_100), 221); // 220.5 帧, 半值向上
        assert_eq!(ms_to_frames(1_000, 44_100), 44_100);
        assert_eq!(ms_to_frames(1_000, 96_000), 96_000);
        assert_eq!(ms_to_frames(0, 0), 0);
        assert_eq!(ms_to_frames(5, 0), 0); // 2.5 帧 → 向下不足 1 帧 ⇒ (0+500)/1000 = 0
    }
}
