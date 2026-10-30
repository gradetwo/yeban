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
//! ## tick ↔ 帧的唯一实现都不在本模块（`#247` 正向、`#248` 反向）
//!
//! [`ticks_to_frames`] 只是 [`yeban_model::time_math::ticks_to_frames`] 的
//! **`None → 0` 适配器**：算式（先算 `samples_per_tick = 60 × sr / (bpm × ppq)`，
//! 再乘 tick 并 `round`）只有模型层一份。反向的「帧 → tick（向上取整）」自
//! `#248` 起由调用方**直接调用** [`yeban_model::time_math::frames_to_ticks_ceil`]，
//! 本模块**不再持有**第二份 `ceil` 算式（`#247` 时它还是一条逐字同式的副本）。
//! 验证脚手架（`verify/render_pure.rs`）用 `extern crate self as yeban_model`
//! ＋ `#[path]` 引入模型层的真实源文件，因此两个方向在裸 `rustc` 下同样可执行。
//!
//! 本模块其余算术仍只用 `+ - * /` 与
//! `f64::round`。[IEEE 754] 把 `round` 指定为 `roundToIntegralTiesAway` —— 它不是
//! 超越函数，在所有符合 IEEE 754 的目标上逐位相同。需要 `sin`/`powf`/`cos` 的地方
//! （振荡器、等程律频率、等功率声相）留在 [`super::render`]，统一走 `libm`
//! [ARCH-DET-001]。
//!
//! [IEEE 754]: https://en.wikipedia.org/wiki/IEEE_754

/// tick → 帧（四舍五入到最近帧；**唯一实现在** [`yeban_model::time_math::ticks_to_frames`]）。
///
/// 本函数只做 **`None → 0` 的哨兵适配**：病态输入（`ppq == 0`、`bpm <= 0`、
/// 非有限 `bpm`、采样率 0）在模型层返回 `None`，这里映射成 `0` —— 调用方随后会
/// 把它当成"没有可渲染内容"（`RENDER_FAILED`），而不是悄悄渲染一段垃圾长度。
/// `tick == 0` 仍然返回 `0`（合法的零帧，不是病态）。
///
/// ⚠ `#247` 之前这里是一份**独立的**浮点算式
/// （`tick × 60 / ppq / bpm × sr`）：与引擎的
/// `tick × (60 × sr / (bpm × ppq))` 代数等价、浮点求值不等价，在 44.1 kHz 上
/// 实测 **118 处**一帧级分歧。第一例 `bpm=60, ppq=960, sr=44100, tick=168`：
/// 旧实现 **7717**、引擎 **7718**。收口后本函数用模型层的求值顺序，因此这一例
/// 现在给出 **7718**；48/96 kHz 上两种顺序逐位相同，读数不变。
#[must_use]
pub fn ticks_to_frames(end_tick: u64, ppq: u64, bpm: f64, sample_rate: u32) -> u64 {
    yeban_model::time_math::ticks_to_frames(end_tick, bpm, ppq, sample_rate).unwrap_or(0)
}

/// tick → 帧（读传入的 [`yeban_model::time_math::TempoMap`]）。
///
/// 空表走 [`ticks_to_frames`] 的同一条恒定 BPM 路径（逐位相同）；非空走模型层的
/// 分段常速换算（`#339`，[ADR-0021]）。`None` 仍映射成 `0`（与 [`ticks_to_frames`]
/// 同一个哨兵口径）。
#[must_use]
pub fn frames_at_tick(
    tick: u64,
    ppq: u64,
    tempo: &yeban_model::time_math::TempoMap,
    sample_rate: u32,
) -> u64 {
    tempo.frames_at_tick(tick, ppq, sample_rate).unwrap_or(0)
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

/// 会被 **24-bit 量化器钳住**的样本数（#175 坑⑧：削波必须**可见**）。
///
/// ## 量什么、单位是什么
///
/// 对象 = 抖动 / 量化**之前**的浮点母带（`RenderArtifact` 写进容器的正是它）；
/// 单位 = **样本个数**（不是帧数、不是声道数）。判据是 `|sample| > 1.0`
/// （严格大于）：
///
/// - `1.0` / `-1.0` 恰好落在 PCM 满量程的端点（`quantize_i24(1.0) == 8_388_607`、
///   `quantize_i24(-1.0) == -8_388_608`），它们**不需要**钳位；
/// - `|sample| > 1.0` 的样本会被 `clamp_i32` 压到端点 —— 那就是削波，波形顶部变平。
///
/// 非有限样本（`NaN`）不计入：本渲染链不产出它们，且 `NaN > 1.0` 为假，不会制造假读数。
///
/// ## 为什么这个读数必须存在（实测）
///
/// #174 实测：响应里 `peak.before = 1.5748`，而写出的文件里 **6134 个样本**被钳在
/// `±1.0` —— 调用方无法从"峰值 1.5748"这个数推出"有多少个样本真的被削平了"。
#[must_use]
pub fn clipped_sample_count(samples: &[f32]) -> u64 {
    samples
        .iter()
        .filter(|sample| sample.is_finite() && sample.abs() > 1.0)
        .count() as u64
}

/// ⭐ `#225`：削波时建议的 **trim 量**（dB；负值 = 减增益，`0.0` = 不需要减）。
///
/// ## 公式（逐字，响应里的 `suggestedTrimDbFormula` 就是这一行）
///
/// ```text
/// suggestedTrimDb = -(ceil(truePeakDbfs * 2) / 2)
/// ```
///
/// 语义：把真峰值**向上取整到 0.5 dB 的整数倍**再取负 ⇒ 按这个量减增益后
/// `truePeakDbfs <= 0`（不会差 0.1 dB 又削一次），且**最多多减 0.5 dB**
/// （不会把一段只过载 0.2 dB 的母带砍掉 1 dB）。
///
/// ## 为什么是「向上」而不是四舍五入
///
/// 四舍五入没有保证：`truePeakDbfs = 5.44` 会给出 `5.5`（够），但 `5.46` 会给出
/// `5.5`（**不够**）。`ceil(x * 2) / 2 >= x` 对所有有限 `x` 成立，才是"按这个量减
/// 之后不削波"的保证。
///
/// 非有限值与 `<= 0` 的输入返回 `0.0`（不需要 trim）—— 这一条让"没有削波"
/// 与"测不出真峰值"都不会被翻译成一个正数（那是**提升**增益，语义正好相反）。
///
/// 判据：`render_math.rs` 的 `suggested_trim_is_ceiled_to_half_a_db` 与
/// `tests/render_master.rs` 的
/// `a_clipping_master_gets_a_clipping_warning_with_a_trim_suggestion`（后者在真实响应上验算
/// 同一个公式）。
#[must_use]
pub fn suggested_trim_db(true_peak_dbfs: f32) -> f32 {
    if !true_peak_dbfs.is_finite() || true_peak_dbfs <= 0.0 {
        return 0.0;
    }
    let trimmed = (true_peak_dbfs * 2.0).ceil() / 2.0;
    if trimmed <= 0.0 { 0.0 } else { -trimmed }
}

/// 线性幅度 → dBFS（`20 * log10(amplitude)`）。
///
/// 返回 `None` 而不是 `-inf`：`0`（静音）与"测不出"在 JSON 里都写成 `null`，
/// ⛔ 不写 `-Infinity`（JSON 没有它）也⛔ 不写 `0 dBFS`（那是满量程，语义相反）。
///
/// ⚠ 这里用 `libm::log10f`（纯 Rust 实现）而不是宿主 `log10f`：同一份工程在
/// 不同平台上必须给出同一个读数 [ARCH-DET-001]。`libm` 已在依赖里（见
/// `crates/yeban-mcp/Cargo.toml` 的那条注释），本模块此前"只允许 `+ - * /` 与
/// `round`"的口径**不适用于本函数** —— 它不参与 bit-exact 渲染，只把已经算完的
/// 峰值换个单位报出来；判据只钉住"单调 + 边界"，不钉住 LSB。
#[must_use]
pub fn amplitude_dbfs(amplitude: f32) -> Option<f32> {
    if amplitude.is_finite() && amplitude > 0.0 {
        Some(20.0 * libm::log10f(amplitude))
    } else {
        None
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
    note_frame_span_with_tempo(
        start_tick,
        duration_ticks,
        micro_timing_ticks,
        ppq,
        &yeban_model::time_math::TempoMap::constant(bpm),
        sample_rate,
    )
}

/// 同 [`note_frame_span`]，但 tick → 帧读 `tempo`（空表与本函数逐位相同）。
#[must_use]
pub fn note_frame_span_with_tempo(
    start_tick: u64,
    duration_ticks: u64,
    micro_timing_ticks: i64,
    ppq: u64,
    tempo: &yeban_model::time_math::TempoMap,
    sample_rate: u32,
) -> (i64, i64) {
    let shifted = i128::from(start_tick) + i128::from(micro_timing_ticks);
    let clamped = if shifted <= 0 {
        0
    } else {
        u64::try_from(shifted).unwrap_or(u64::MAX)
    };
    let start = frames_at_tick(clamped, ppq, tempo, sample_rate);
    let end_tick = clamped.saturating_add(duration_ticks.max(1));
    let end = frames_at_tick(end_tick, ppq, tempo, sample_rate);
    // `start` 是**饱和**过的帧数（[`ticks_to_frames`] 的 `frames >= u64::MAX as f64`
    // 分支返回 `u64::MAX`）⇒ "至少 1 帧"这条下界必须用饱和加法表达。
    // 写成 `start + 1` 时 `start == u64::MAX` 会**整数溢出 panic**；工具参数
    // （`yeban_edit_notes` 的 `note.startTick` / `placement.startTick`）能取到
    // 让 `placement.start_tick + note.start_tick >= 737869762948382017` 的值
    // ⇒ 随后一次 `yeban_render_master` 不是返回 `RENDER_FAILED`，而是把进程打死。
    // 姊妹函数 `render_clip_math::clip_frame_span` 一直是 `start.saturating_add(1)`。
    (
        saturating_i64(start),
        saturating_i64(end.max(start.saturating_add(1))),
    )
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
    // `#248`：反向换算的唯一实现在模型层；本模块的副本已删除，判据直接对模型层测。
    use yeban_model::time_math::frames_to_ticks_ceil;

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

    /// ⭐ `#247` 判据: 44.1 kHz 的 `.5` 取引擎的值（7718），⛔ 不是旧 MCP 的 7717。
    ///
    /// 这是 118 处分歧里的第一例；把本函数改回
    /// `tick × 60 / ppq / bpm × sr` 的旧顺序，本行立刻变红。
    #[test]
    fn the_44_1_khz_half_frame_is_the_engine_value_not_the_old_mcp_value() {
        assert_eq!(ticks_to_frames(168, 960, 60.0, 44_100), 7_718);
        // 48/96 kHz 是整数，收口前后都必须是这两个值（摘要不变的基线）。
        assert_eq!(ticks_to_frames(168, 960, 60.0, 48_000), 8_400);
        assert_eq!(ticks_to_frames(168, 960, 60.0, 96_000), 16_800);
    }

    /// ⭐ `#247` 判据: 本包装器与模型层在 **11 BPM × 3 采样率 × tick 0..=4000**
    /// 上逐项相同（0 分歧）。
    ///
    /// 这条判据在 MCP 侧钉住"只剩一次调用、没有第二份算式"：若有人把旧公式搬回来，
    /// 44.1 kHz 的 118 个点会立刻红。
    #[test]
    fn the_wrapper_agrees_with_the_model_on_the_whole_tick_grid() {
        const BPMS: [f64; 11] = [
            30.0, 60.0, 72.0, 100.0, 110.0, 120.0, 128.0, 130.0, 140.0, 174.0, 200.0,
        ];
        let mut checked = 0usize;
        for bpm in BPMS {
            for rate in [44_100u32, 48_000, 96_000] {
                for tick in 0..=4_000u64 {
                    let ours = ticks_to_frames(tick, 960, bpm, rate);
                    let model =
                        yeban_model::time_math::ticks_to_frames(tick, bpm, 960, rate).unwrap_or(0);
                    assert_eq!(ours, model, "bpm={bpm} rate={rate} tick={tick}");
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 11 * 3 * 4_001);
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

    /// ⭐ `#225` 判据: trim 建议**向上取整到 0.5 dB**，且"按它减完不削波"有保证。
    #[test]
    fn suggested_trim_is_ceiled_to_half_a_db() {
        // 任务单里的两个逐字例子。
        assert_eq!(suggested_trim_db(5.45), -5.5);
        assert_eq!(suggested_trim_db(1.874_512), -2.0);
        // 正好落在 0.5 网格上时不额外多减。
        assert_eq!(suggested_trim_db(24.0), -24.0);
        assert_eq!(suggested_trim_db(0.5), -0.5);
        // 小于 0.5 dB 的过载也至少减 0.5（否则仍会削）。
        assert_eq!(suggested_trim_db(0.02), -0.5);
        // 没有过载 / 测不出 ⇒ 不需要 trim（⛔ 不是"提升"）。
        assert_eq!(suggested_trim_db(0.0), 0.0);
        assert_eq!(suggested_trim_db(-3.666_596_4), 0.0);
        assert_eq!(suggested_trim_db(f32::NAN), 0.0);
        assert_eq!(suggested_trim_db(f32::INFINITY), 0.0);
        // **保证**：减完之后真峰值 <= 0，且最多多减 0.5 dB（含 f32 舍入余量）。
        for peak in [0.01f32, 0.2, 0.5, 2.001, 5.45, 5.46, 24.0, 31.123] {
            let trim = suggested_trim_db(peak);
            assert!(trim < 0.0, "过载 {peak} 必须给负 trim，实际 {trim}");
            assert!(
                peak + trim <= 1.0e-4,
                "按建议减完必须不削波: peak={peak} trim={trim}"
            );
            assert!(
                (peak + trim).abs() <= 0.5 + 1.0e-4,
                "最多多减 0.5 dB: peak={peak} trim={trim}"
            );
        }
    }

    /// ⭐ `#225` 判据: 幅度 → dBFS 的边界（静音 ⇒ `None`，满量程 ⇒ `0`）。
    #[test]
    fn amplitude_dbfs_is_none_for_silence_and_zero_at_full_scale() {
        assert_eq!(amplitude_dbfs(0.0), None);
        assert_eq!(amplitude_dbfs(-0.0), None);
        assert_eq!(amplitude_dbfs(-1.0), None);
        assert_eq!(amplitude_dbfs(f32::NAN), None);
        assert_eq!(amplitude_dbfs(f32::INFINITY), None);
        let full_scale = amplitude_dbfs(1.0).expect("1.0 是可测的");
        assert!(full_scale.abs() < 1.0e-6, "1.0 ⇒ 0 dBFS，实际 {full_scale}");
        let double = amplitude_dbfs(2.0).expect("2.0 是可测的");
        assert!(
            (double - 6.020_6).abs() < 1.0e-3,
            "2.0 ⇒ +6.02 dBFS，实际 {double}"
        );
        // 单调：更响的源读数更大。
        assert!(amplitude_dbfs(0.5).unwrap() < amplitude_dbfs(0.6).unwrap());
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
        assert_eq!(silence.len(), 4, "夹具是四个样本（否则下面那条恒真）");
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

    /// 判据 6b: `release == 0` 时，"区间外为 `0`"仍然成立。
    ///
    /// `offset == length` 时 `remaining == 0`，但 `release == 0` 会让 `falling`
    /// 走 `if release == 0 { 1.0 }` 那条腿 ⇒ "起点已经越界"这件事**只能**由
    /// `offset >= length` 那道闸门拦住，不能指望释音系数归零兜底。
    ///
    /// 生产调用点（现位于 `render.rs` 第 1566 行起）的循环条件是 `global < note.end`
    /// ⇒ `offset < length` 恒成立，且起音/释音都来自 `ms_to_frames`（母带采样率下
    /// 至少 220 / 441 帧）⇒ 本条钉的是**函数自己的文档契约**（"区间外为 `0`"），
    /// 不是当前生产路径上的读数。
    #[test]
    fn envelope_outside_the_interval_is_zero_even_when_release_is_zero() {
        assert_eq!(envelope(100, 100, 10, 0), 0.0);
        assert_eq!(envelope(101, 100, 0, 0), 0.0);
        assert_eq!(envelope(200, 100, 0, 0), 0.0);
        // 反向: 区间内仍然必须有声（闸门不是"一律 0"）。
        assert!(envelope(0, 100, 10, 0) > 0.0);
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

    /// 判据 7b: 时间轴末端**饱和**到 `u64::MAX` 时，"至少 1 帧"这条下界不得整数溢出。
    ///
    /// 触发点是 `ticks_to_frames` 的 `frames >= u64::MAX as f64` 分支（它返回
    /// `u64::MAX`）。960 PPQ / 120 BPM / 48 kHz 下 `frames == tick * 25`，因此第一个
    /// 命中的 tick 是 `737869762948382017` —— 本判据先**机械复核**这个常量，再要求
    /// 区间是 `(i64::MAX, i64::MAX)`。
    ///
    /// 可以变红的方式：把 `start.saturating_add(1)` 写回 `start + 1` ⇒ 本判据
    /// `attempt to add with overflow`。工具面可达（`note.startTick` /
    /// `placement.startTick`，端到端判据在 `tests/render_master.rs`）。
    #[test]
    fn a_saturated_timeline_end_does_not_overflow_the_one_frame_floor() {
        const SATURATING_TICK: u64 = 737_869_762_948_382_017;
        assert_eq!(
            ticks_to_frames(SATURATING_TICK, 960, 120.0, 48_000),
            u64::MAX,
            "这个常量必须是**第一个**让 tick→帧饱和的 tick，否则本判据测的不是那条分支"
        );
        assert!(
            ticks_to_frames(SATURATING_TICK - 1, 960, 120.0, 48_000) < u64::MAX,
            "紧邻它的上一个 tick 还没有饱和 ⇒ 这个常量是那条分支的**第一个**触发点"
        );
        let (start, end) = note_frame_span(SATURATING_TICK, 480, 0, 960, 120.0, 48_000);
        assert_eq!((start, end), (i64::MAX, i64::MAX));
        // 正常取值仍然满足"至少 1 帧"（饱和分支不能被当成唯一路径）。
        let (start, end) = note_frame_span(0, 480, 0, 960, 120.0, 48_000);
        assert_eq!(start, 0);
        assert_eq!(
            end,
            i64::try_from(ticks_to_frames(480, 960, 120.0, 48_000)).expect("小整数")
        );
        assert!(end > start);
    }

    /// 判据 7c: 起点取到 `u64::MAX` 时，**终点那一次加法**也必须饱和。
    ///
    /// `note_frame_span` 有两次加法：`start.saturating_add(1)`（判据 7b 钉住）与
    /// `clamped.saturating_add(duration_ticks.max(1))`（本条钉住）。后者在
    /// `clamped == u64::MAX` 时若写成 `clamped + …` 就是 `attempt to add with overflow`。
    ///
    /// 工具面可达：`placement.start_tick` / `note.start_tick` 都是**无上界**的 `u64`
    /// （`read_optional_u64` / `read_u64` 只要求非负整数），而 `render.rs` 第 2124 行起
    /// 的 `base_tick = placement.start_tick.saturating_add(note.start_tick)` 会把它压到
    /// `u64::MAX` 再传进来；`ClipPlacement::validate` 也不管 `start_tick` 的大小。
    /// 端到端判据在 `tests/render_master.rs` 的极值 tick 那一条。
    #[test]
    fn a_maximal_start_tick_saturates_the_end_tick_addition_too() {
        assert_eq!(
            ticks_to_frames(u64::MAX, 960, 120.0, 48_000),
            u64::MAX,
            "夹具前提: 这个 tick 本身就落在 tick→帧 的饱和分支里"
        );
        let (start, end) = note_frame_span(u64::MAX, 480, 0, 960, 120.0, 48_000);
        assert_eq!((start, end), (i64::MAX, i64::MAX));
        // 微时值把起点推过 `u64::MAX` 也走同一条路（`i128` 相加后 `try_from` 失败）。
        let (start, end) = note_frame_span(u64::MAX, 480, 240, 960, 120.0, 48_000);
        assert_eq!((start, end), (i64::MAX, i64::MAX));
    }

    /// 判据 8: 日历换算对已知历元精确（含闰年；负天数另见判据 8b）。
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

    /// 判据 8b: `civil_from_days` 的**负天数分支**。
    ///
    /// 算法里 `shifted < 0` 时要再减 `146_096` 天，那是**另一条**算术：
    /// 只取 `days >= 0` 的判据（判据 8）一个字节都碰不到它。
    ///
    /// 期望值用**独立**的儒略日算法（Fliegel–Van Flandern 的 `JDN → 公历`，
    /// 与 Hinnant 的 `civil_from_days` 不是同一份代码）逐条对账过：
    /// JDN `1721120` = 0000-03-01、JDN `1721119` = 0000-02-29（公元 0 年是闰年）、
    /// JDN `2440587` = 1969-12-31。
    #[test]
    fn civil_from_days_covers_the_negative_era_correction() {
        assert_eq!(civil_from_days(0), (1970, 1, 1), "零点必须钉住");
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(-719_468), (0, 3, 1));
        // 纪元修正项的分界: 再往前一天就落进 `era` 的负数侧。
        assert_eq!(civil_from_days(-719_469), (0, 2, 29));
        // 回程: 从 0000-02-29 起 719_469 天正好是 1970-01-01。
        assert_eq!(civil_from_days(-719_469 + 719_469), (1970, 1, 1));
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
