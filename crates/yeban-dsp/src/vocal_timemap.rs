//! 连续流式 timemap 人声对齐：单实例 [`signalsmith_stretch::Stretch`] 渲染。
//!
//! 本模块实现 Melodyne 对标 Phase 1 的对齐管线（ADR-0067）：
//!
//! * [`PiecewiseTimeMap`]：输出时间 → 输入时间的分段线性映射。锚点是浊音
//!   韵母起点（VOP），不是能量通量峰；每音节起音窗与收音尾以斜率 1.0
//!   锁定，稳态浊音核承担伸缩，换气/清音段锁定 1.0。
//! * [`build_timemap`]：从精修边界与乐谱目标网格构造映射。间隙按输入
//!   实际成分分段：换气段斜率 1.0，真静音段自由伸缩（静音拉伸无听感）；
//!   松弛量先由静音吸收，静音耗尽后由浊音核承担；超限区间记入
//!   [`TimemapBuild::clamps`]（可观测信号），渲染侧只计数、不静默改写
//!   之外的任何事。
//! * [`render_continuous_timemap`]：整行/整段只用一个拉伸实例，不在锚点
//!   处切开、不做段间交叉淡化。固定 240 采样输出块，输入步长由映射斜率
//!   决定；步长钳制到 `[1, 960]` 并计数（见 [`RenderReport`]）。
//!
//! # 离线限定
//!
//! 本模块只用于导入/参数变更时的离线渲染，绝不进入实时回调：C++ 侧在
//! 增长时可能分配，且拉伸本身是块处理。调用方必须在离线线程调用。
//!
//! # 时间基准
//!
//! 全部时间为**秒**。输入时间是 take 内时间（回执 `measured_onset_s` 的
//! 坐标：首条 score 14.4 s 对应 take 0.02 s），不是工程时间；输出时间是
//! 乐谱相对时间（行首为 0），工程侧放置时再加行首工程偏移。乐谱目前
//! 无 note_id，音节以索引标识。
//!
//! # 速率约定
//!
//! 本模块的“速率”一律指**拉伸倍数 = 输出秒 / 输入秒**（大于 1 表示拉长）。
//! 映射斜率是其倒数（输入秒 / 输出秒），只在 [`PiecewiseTimeMap`] 内部
//! 出现。

use signalsmith_stretch::Stretch;

/// 起音窗锁定：VOP 之前多少秒锁定 1.0（专家 Q28：10 ms）。
pub const LOCK_PRE_S: f64 = 0.010;
/// 起音窗锁定：VOP 之后多少秒锁定 1.0（专家 Q28：35 ms）。
pub const LOCK_VOP_S: f64 = 0.035;
/// 收音尾锁定：voiced_end 之前多少秒锁定 1.0（专家 Q28：30 ms）。
pub const LOCK_RELEASE_S: f64 = 0.030;
/// 渲染钳制下限：单块输入步长不低于 1 采样。
pub const MIN_STEP: usize = 1;
/// 渲染钳制上限：单块输入步长不超过 4 倍输出块（专家 Q16 骨架的 `[1,
/// out_chunk × 4]`，改写为计数事件而非静默）。
pub const MAX_STEP_MULTIPLIER: usize = 4;
/// 固定输出块长度（采样），5 ms @ 48 kHz。
pub const OUT_CHUNK: usize = 240;
/// 静音判定：RMS 低于此分贝即视为真静音（可自由伸缩，听感上仍是本底）。
pub const SILENCE_DB: f64 = -55.0;
/// 静音最短持续：短于此的低能量段按换气处理，不按静音吸收。
pub const SILENCE_MIN_S: f64 = 0.030;

/// 拉伸实例配置：探针修正后的两种合法构造（二选一）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchConfig {
    /// `Stretch::new(1, 2400, 480)`：块 2400、间隔 480（50 ms / 10 ms）。
    Custom2400_480,
    /// `Stretch::preset_default(1, sample_rate)`：48 kHz 时块 5760、间隔 1440。
    /// ⚠ 速率变化处的孤立瞬态峰会抹开约 49 ms（突发 pilot 实测），不适用
    /// 作人声对齐默认配置；仅作音色对比备选。
    PresetDefault,
}

/// 输出时间 → 输入时间的分段线性映射。
///
/// 拐点按输出时间单调递增；区间外按端点斜率 1.0 延伸（行首前/行尾后
/// 的预卷与拖尾）。
#[derive(Debug, Clone, PartialEq)]
pub struct PiecewiseTimeMap {
    /// `(输出秒, 输入秒)` 拐点，按输出秒严格递增。
    pub knots: Vec<(f64, f64)>,
}

impl PiecewiseTimeMap {
    /// 构造映射。拐点必须按输出秒严格递增，且至少两个点。
    ///
    /// # Panics
    ///
    /// 拐点少于两个或输出秒不严格递增时 panic（构造期错误，不应进入渲染）。
    pub fn new(mut knots: Vec<(f64, f64)>) -> Self {
        assert!(knots.len() >= 2, "timemap needs at least two knots");
        knots.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for pair in knots.windows(2) {
            assert!(
                pair[1].0 > pair[0].0,
                "timemap knots must be strictly increasing"
            );
        }
        Self { knots }
    }

    /// 输出时间 `out_s` 对应的输入时间（秒）。区间外斜率 1.0 延伸。
    pub fn map(&self, out_s: f64) -> f64 {
        let knots = &self.knots;
        if out_s <= knots[0].0 {
            return knots[0].1 + (out_s - knots[0].0);
        }
        let last = knots.len() - 1;
        if out_s >= knots[last].0 {
            return knots[last].1 + (out_s - knots[last].0);
        }
        let idx = knots
            .partition_point(|knot| knot.0 <= out_s)
            .saturating_sub(1);
        let (x0, y0) = knots[idx];
        let (x1, y1) = knots[idx + 1];
        let t = (out_s - x0) / (x1 - x0);
        y0 + t * (y1 - y0)
    }

    /// 输出区间 `[out_start, out_end]` 的拉伸倍数（输出秒 / 输入秒）。
    pub fn rate(&self, out_start: f64, out_end: f64) -> f64 {
        if out_end <= out_start {
            return 1.0;
        }
        let in_len = self.map(out_end) - self.map(out_start);
        if in_len <= 0.0 {
            return 1.0;
        }
        (out_end - out_start) / in_len
    }
}

/// 一个音节的精修边界（take 内秒）。
///
/// 平铺契约：调用方须保证非末音节的 `off_s` ≤ 下一音节 `pre_s` −
/// 10 ms（驱动侧钳制；构造器对违规输入防御性再钳制并记警告）。连唱中
/// 实测 off 常越过下一 pre（浊辅音延续），不钳制则映射输入回跳、输出
/// 重复一段音频。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyllableIn {
    /// 辅音起点（能量谷回溯值）。
    pub pre_s: f64,
    /// 浊音韵母起点（VOP）。
    pub vop_s: f64,
    /// 浊音结束。
    pub off_s: f64,
}

/// 一个音节的乐谱目标（行相对秒）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyllableTarget {
    /// 目标 VOP（行相对秒）。
    pub vop_s: f64,
}

/// 间隙成分：行内间隙在映射中的处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapKind {
    /// 真静音：自由伸缩（听感仍是本底）。
    Silence,
    /// 换气/清音：锁定 1.0。
    Breath,
}

/// 构造后的一段区间（用于门检与诊断）。`rate` 为需求拉伸倍数
/// （输出秒 / 输入秒）；`rate_clamped` 为钳制后倍数（无钳制时等于 `rate`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuiltInterval {
    /// 区间种类：`true` 为浊音核（参与门检），`false` 为锁定/间隙段。
    pub is_voiced_core: bool,
    /// 输出起止（行相对秒）。
    pub out_start: f64,
    /// 输出起止（行相对秒）。
    pub out_end: f64,
    /// 需求拉伸倍数。
    pub rate: f64,
    /// 钳制后拉伸倍数。
    pub rate_clamped: f64,
}

/// 速率钳制事件：可观测信号，绝不静默。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClampEvent {
    /// 输出时间（行相对秒）。
    pub out_s: f64,
    /// 需求拉伸倍数。
    pub requested_rate: f64,
    /// 钳制后拉伸倍数。
    pub clamped_rate: f64,
}

/// 构造选项。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimemapOptions {
    /// 浊音核渲染钳制区间（专家 Q13：`[0.60, 1.65]`）。
    pub core_clamp: (f64, f64),
    /// 换气段允许区间（被迫时才压缩到 0.9，绝不拉长）。
    pub breath_range: (f64, f64),
    /// 静音判定分贝。
    pub silence_db: f64,
}

impl Default for TimemapOptions {
    fn default() -> Self {
        Self {
            core_clamp: (0.60, 1.65),
            breath_range: (0.90, 1.0),
            silence_db: SILENCE_DB,
        }
    }
}

/// 构造结果。
#[derive(Debug, Clone)]
pub struct TimemapBuild {
    /// 分段线性映射。
    pub map: PiecewiseTimeMap,
    /// 全部区间（门检用浊音核区间）。
    pub intervals: Vec<BuiltInterval>,
    /// 速率钳制事件（空表示无钳制）。
    pub clamps: Vec<ClampEvent>,
    /// 非致命警告（如钳制溢出导致拐点被跳过）。
    pub warnings: Vec<String>,
}

/// 输入 PCM 的 RMS 曲线（20 ms 窗、10 ms 跳），用于静音/换气分类。
fn rms_curve(source: &[f32], sample_rate: u32) -> Vec<f32> {
    let win = (sample_rate as usize * 20 / 1000).max(1);
    let hop = (sample_rate as usize * 10 / 1000).max(1);
    let mut curve = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let end = (start + win).min(source.len());
        let mut sum = 0.0_f64;
        for sample in &source[start..end] {
            let value = f64::from(*sample);
            sum += value * value;
        }
        let mean = sum / (end - start).max(1) as f64;
        curve.push(mean.sqrt() as f32);
        start += hop;
    }
    curve
}

/// 区间 `[start_s, end_s]` 内的成分分段（输入秒）：持续超过
/// [`SILENCE_MIN_S`] 的低能量段为 [`GapKind::Silence`]，其余为
/// [`GapKind::Breath`]。分段按输入时间递增、无重叠、无间隙。
fn gap_segments(
    rms: &[f32],
    sample_rate: u32,
    start_s: f64,
    end_s: f64,
    silence_db: f64,
) -> Vec<(f64, f64, GapKind)> {
    if end_s <= start_s {
        return Vec::new();
    }
    let hop = (sample_rate as usize * 10 / 1000).max(1);
    let hop_s = hop as f64 / sample_rate as f64;
    let threshold = 10_f32.powf((silence_db / 20.0) as f32);
    let start_idx = ((start_s * sample_rate as f64) as usize / hop).min(rms.len());
    let end_idx = ((end_s * sample_rate as f64) as usize / hop).min(rms.len());
    let min_run = (SILENCE_MIN_S / hop_s).ceil() as usize;
    let mut segments: Vec<(f64, f64, GapKind)> = Vec::new();
    let mut cursor = start_s;
    let mut idx = start_idx;
    while idx < end_idx {
        if rms[idx] < threshold {
            let run_start = idx;
            while idx < end_idx && rms[idx] < threshold {
                idx += 1;
            }
            let run_len = idx - run_start;
            if run_len >= min_run {
                let seg_start = run_start as f64 * hop_s;
                let seg_end = idx as f64 * hop_s;
                let seg_start = seg_start.max(cursor);
                if seg_start > cursor {
                    segments.push((cursor, seg_start, GapKind::Breath));
                }
                segments.push((seg_start, seg_end.min(end_s), GapKind::Silence));
                cursor = seg_end.min(end_s);
            }
        } else {
            idx += 1;
        }
    }
    if cursor < end_s {
        segments.push((cursor, end_s, GapKind::Breath));
    }
    segments
}

/// 构造行 timemap。
///
/// # 参数
///
/// * `source`：整段 take PCM（单声道，`sample_rate` 采样率），用于静音分类。
/// * `syllables`/`targets`：等长；音节按 take 时间递增，目标按行相对时间递增。
/// * `out_end_s`：行输出总长（行相对秒，= 下一行首或段尾）。
/// * `tail_in_end`：行尾间隙的输入终点（take 内秒：下一行首音节 pre，或
///   take 末尾）。
///
/// # 布局
///
/// 每音节：`[pre, vop)` 起音窗（斜率 1.0，含 pre 前 10 ms 锁定）、
/// `[vop + 35 ms, off - 30 ms)` 浊音核（吸收伸缩）、`[off - 30 ms, off)`
/// 收音尾（斜率 1.0）。音节间间隙（收音尾终点 → 下一音节锁窗起点）
/// 按输入实际成分分段：换气段斜率 1.0，真静音段按比例分摊静音目标长度。
/// 分配顺序：松弛量先由静音吸收（核保持 1.0），静音耗尽后核承担；
/// 收紧时静音先压到 0，再压核，核到下限后再压换气到 0.9，仍不够则记
/// 钳制事件（渲染按钳制执行，门检按需求速率判失败 → 重抽）。
///
/// # Panics
///
/// 音节与目标长度不等、音节不足一个时 panic。
pub fn build_timemap(
    source: &[f32],
    sample_rate: u32,
    syllables: &[SyllableIn],
    targets: &[SyllableTarget],
    out_end_s: f64,
    tail_in_end: f64,
    options: TimemapOptions,
) -> TimemapBuild {
    assert!(!syllables.is_empty(), "need at least one syllable");
    assert_eq!(
        syllables.len(),
        targets.len(),
        "syllables and targets must match"
    );
    let rms = rms_curve(source, sample_rate);
    let mut knots: Vec<(f64, f64)> = Vec::new();
    let mut intervals: Vec<BuiltInterval> = Vec::new();
    let mut clamps: Vec<ClampEvent> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let push_knot =
        |out_s: f64, in_s: f64, knots: &mut Vec<(f64, f64)>, warnings: &mut Vec<String>| {
            if let Some(last) = knots.last()
                && out_s <= last.0
            {
                warnings.push(format!(
                    "knot skipped: out {out_s:.4} <= last out {:.4} (clamp overflow)",
                    last.0
                ));
                return;
            }
            knots.push((out_s, in_s));
        };

    let push_knot_silent = |out_s: f64, in_s: f64, knots: &mut Vec<(f64, f64)>| {
        if let Some(last) = knots.last()
            && out_s <= last.0
        {
            return;
        }
        knots.push((out_s, in_s));
    };

    for (i, (syllable, target)) in syllables.iter().zip(targets.iter()).enumerate() {
        let target_pre = target.vop_s - (syllable.vop_s - syllable.pre_s);
        let lock_start_out = target_pre - LOCK_PRE_S;
        let lock_end_out = target.vop_s + LOCK_VOP_S;
        let lock_start_in = syllable.pre_s - LOCK_PRE_S;
        let lock_end_in = syllable.vop_s + LOCK_VOP_S;
        push_knot(lock_start_out, lock_start_in, &mut knots, &mut warnings);
        push_knot(lock_end_out, lock_end_in, &mut knots, &mut warnings);
        intervals.push(BuiltInterval {
            is_voiced_core: false,
            out_start: lock_start_out,
            out_end: lock_end_out,
            rate: 1.0,
            rate_clamped: 1.0,
        });

        let core_out_start = lock_end_out;
        let (next_lock_start_out, next_lock_start_in, gap_in_end) = if i + 1 < syllables.len() {
            let next = &syllables[i + 1];
            let next_target = &targets[i + 1];
            let next_target_pre = next_target.vop_s - (next.vop_s - next.pre_s);
            (
                next_target_pre - LOCK_PRE_S,
                next.pre_s - LOCK_PRE_S,
                next.pre_s - LOCK_PRE_S,
            )
        } else {
            (out_end_s, tail_in_end, tail_in_end)
        };
        let core_out_end = next_lock_start_out;
        let core_in_start = lock_end_in;
        // 平铺契约：调用方须保证 off ≤ 下一音节 pre − 10 ms（驱动侧已钳制）。
        // 防御性再钳制一次（直接 API 调用方遗忘时）：收音尾终点不越过间隙
        // 输入终点，核终点不越过收音尾终点，保证映射输入单调不回跳。
        // 连唱中实测 off 常越过下一 pre（浊辅音延续），钳制时记警告。
        let release_in_end = if syllable.off_s > gap_in_end {
            warnings.push(format!(
                "syllable {i}: off {:.4} clamped to gap end {:.4} (legato overlap)",
                syllable.off_s, gap_in_end
            ));
            gap_in_end
        } else {
            syllable.off_s
        };
        let core_in_end = (syllable.off_s - LOCK_RELEASE_S)
            .max(core_in_start)
            .min(release_in_end);
        let core_src = (core_in_end - core_in_start).max(0.0);

        let avail = (core_out_end - core_out_start).max(0.0);
        let release_tgt = LOCK_RELEASE_S.min(avail);
        let gap_segs = gap_segments(
            &rms,
            sample_rate,
            release_in_end,
            gap_in_end,
            options.silence_db,
        );
        let breath_src: f64 = gap_segs
            .iter()
            .filter(|seg| seg.2 == GapKind::Breath)
            .map(|seg| seg.1 - seg.0)
            .sum();
        let silence_src: f64 = gap_segs
            .iter()
            .filter(|seg| seg.2 == GapKind::Silence)
            .map(|seg| seg.1 - seg.0)
            .sum();

        // 分配：need = 锁窗之后到下一锁窗之前除收音尾外的输出长度。
        let need = (avail - release_tgt).max(0.0);
        let mut core_tgt;
        let mut breath_tgt = breath_src;
        let silence_tgt;
        let is_last = i + 1 == syllables.len();
        if is_last {
            // 行尾间隙是行间空间：尾换气可完全丢弃，绝不挤占歌词核（行
            // 13 隙字实测：尾换气 0.6 s + 核 0.545 s 挤 0.535 s 预算，旧
            // 规则把核压到下限并记钳制，歌词本身对齐（R 0.999）却被判失败）。
            // 核优先（不超过自然长度），余量归静音；无静音吸纳时核拉伸填
            // 满（钳制路径正常检查 1.65 上限）；预算连核下限都容不下时记钳制。
            if need >= core_src {
                if silence_src > 0.0 {
                    core_tgt = core_src;
                    breath_tgt = 0.0;
                    silence_tgt = need - core_src;
                } else {
                    core_tgt = need;
                    breath_tgt = 0.0;
                    silence_tgt = 0.0;
                }
            } else if need >= core_src * options.core_clamp.0 {
                breath_tgt = 0.0;
                silence_tgt = 0.0;
                core_tgt = need;
            } else {
                breath_tgt = 0.0;
                silence_tgt = 0.0;
                core_tgt = core_src * options.core_clamp.0;
                if core_src > 0.0 {
                    clamps.push(ClampEvent {
                        out_s: core_out_start,
                        requested_rate: need / core_src,
                        clamped_rate: options.core_clamp.0,
                    });
                }
                if need < core_tgt {
                    warnings.push(format!(
                        "syllable {i}: tail gap too tight, need {need:.3}s < core min"
                    ));
                }
            }
        } else if silence_src > 0.0 && need >= breath_src + core_src {
            // 宽松且有静音：核 1.0，静音吸收全部松弛。
            core_tgt = core_src;
            silence_tgt = need - breath_src - core_src;
        } else if need >= breath_src + core_src * options.core_clamp.0 {
            // 收紧或无静音宽松：静音压到 0（或不存在），核承担剩余。
            silence_tgt = 0.0;
            core_tgt = need - breath_src;
        } else {
            // 极紧：核压到下限，换气压到下限，仍不够则收音尾也被压
            // （release_tgt 已在 need 之外，此处只记警告；拐点跳过会告警）。
            // 下限压制同样是钳制：记 ClampEvent（需求率按未压制几何算），
            // 否则门检的“钳制即失败”规则会漏掉这支（压制后需求率恰为下限
            // 值，落在钳制界内，看似未钳制）。
            core_tgt = core_src * options.core_clamp.0;
            breath_tgt = breath_src * options.breath_range.0;
            silence_tgt = 0.0;
            if core_src > 0.0 {
                clamps.push(ClampEvent {
                    out_s: core_out_start,
                    requested_rate: (need - breath_src).max(0.0) / core_src,
                    clamped_rate: options.core_clamp.0,
                });
            }
            if need < core_tgt + breath_tgt {
                warnings.push(format!(
                    "syllable {i}: gap too tight, need {need:.3}s < core min + breath min"
                ));
            }
        }
        // 零长核（极短音）：不给它分配输出长度（否则映射平段导致渲染侧
        // 步长钳制 drones），其输出只剩锁窗 + 收音尾，松弛量归间隙。
        if core_src <= 0.0 {
            if core_tgt > 0.0 {
                warnings.push(format!(
                    "syllable {i}: zero-length core, core output dropped"
                ));
            }
            core_tgt = 0.0;
        }
        let required_rate = if core_src > 0.0 {
            core_tgt / core_src
        } else {
            1.0
        };
        let mut clamped_rate = required_rate;
        if core_src > 0.0 {
            if required_rate < options.core_clamp.0 {
                clamped_rate = options.core_clamp.0;
            } else if required_rate > options.core_clamp.1 {
                clamped_rate = options.core_clamp.1;
            }
            if (clamped_rate - required_rate).abs() > 1e-9 {
                clamps.push(ClampEvent {
                    out_s: core_out_start,
                    requested_rate: required_rate,
                    clamped_rate,
                });
                core_tgt = core_src * clamped_rate;
            }
        }
        let core_out_end_actual = core_out_start + core_tgt;
        let release_out_end = core_out_end_actual + release_tgt;
        intervals.push(BuiltInterval {
            is_voiced_core: true,
            out_start: core_out_start,
            out_end: core_out_end_actual,
            rate: required_rate,
            rate_clamped: clamped_rate,
        });
        push_knot(core_out_end_actual, core_in_end, &mut knots, &mut warnings);
        push_knot(release_out_end, release_in_end, &mut knots, &mut warnings);
        intervals.push(BuiltInterval {
            is_voiced_core: false,
            out_start: core_out_end_actual,
            out_end: release_out_end,
            rate: 1.0,
            rate_clamped: 1.0,
        });

        // 间隙拐点：按成分分段，换气段斜率 1.0（被迫时按比例压缩），
        // 静音段按比例分摊；零长静音段只跳过输入，不记警告。
        let gap_out_end = next_lock_start_out;
        let mut out_cursor = release_out_end;
        let mut in_cursor: f64;
        let seg_count = gap_segs.len();
        for (idx, (seg_start, seg_end, kind)) in gap_segs.iter().enumerate() {
            let seg_in_len = seg_end - seg_start;
            let last = idx + 1 == seg_count;
            let seg_out_len = if last {
                (gap_out_end - out_cursor).max(0.0)
            } else if *kind == GapKind::Breath {
                if breath_src > 0.0 {
                    seg_in_len * breath_tgt / breath_src
                } else {
                    seg_in_len
                }
            } else if silence_src > 0.0 {
                seg_in_len * silence_tgt / silence_src
            } else {
                seg_in_len
            };
            out_cursor += seg_out_len;
            in_cursor = *seg_end;
            intervals.push(BuiltInterval {
                is_voiced_core: false,
                out_start: out_cursor - seg_out_len,
                out_end: out_cursor,
                rate: if seg_in_len > 0.0 {
                    seg_out_len / seg_in_len
                } else {
                    1.0
                },
                rate_clamped: if seg_in_len > 0.0 {
                    seg_out_len / seg_in_len
                } else {
                    1.0
                },
            });
            if !last {
                push_knot_silent(out_cursor, in_cursor, &mut knots);
            }
        }
        // 间隙终点拐点由下一音节的锁窗起点（或行尾）承接；此处补一个
        // 显式拐点以保证间隙终点精确落在目标上。
        push_knot(gap_out_end, next_lock_start_in, &mut knots, &mut warnings);
    }

    TimemapBuild {
        map: PiecewiseTimeMap::new(knots),
        intervals,
        clamps,
        warnings,
    }
}

/// Pilot 实测的延迟对齐丢弃量（采样，48 kHz 突发峰值法，见 ADR-0067）。
///
/// `output_latency()` 声称值与实测峰值对齐量不符：Custom  claim 1200、
/// 实测 1500（偏差 +6.2 ms）；Preset claim 2880、实测 4860（偏差
/// +41.2 ms，用 claim 值会直接吃掉 VOP 验收余量）。因此默认丢弃量取
/// 实测值，不取 API 声称值。
///
/// * Custom：块/间隔固定采样数，丢弃量与采样率无关，恒为 1500 采样。
/// * Preset：块/间隔随采样率缩放，丢弃量按 `4860 × sr / 48000` 缩放。
pub fn default_align_drop(config: StretchConfig, sample_rate: u32) -> usize {
    match config {
        StretchConfig::Custom2400_480 => 1500,
        StretchConfig::PresetDefault => ((4860_u64 * u64::from(sample_rate)) / 48_000) as usize,
    }
}

/// 渲染报告：钳制事件全部可观测。
#[derive(Debug, Clone, Default)]
pub struct RenderReport {
    /// 块级输入步长钳制次数（0 表示映射全程在 `[1, 960]` 内）。
    pub step_clamps: usize,
    /// 被钳制的块（输出块起点采样，需求步长，钳制后步长），最多前 32 条。
    pub step_clamp_details: Vec<(usize, usize, usize)>,
    /// 实际输出长度（采样）。
    pub out_len: usize,
}

/// 单实例连续流式渲染。
///
/// * `source`：单声道 PCM（f32，`sample_rate` 采样率）。
/// * `map`：输出秒 → 输入秒映射（行相对输出时间；行首输出对应 `map(0)`）。
/// * `target_len`：目标输出长度（采样）。
/// * `config`：拉伸实例配置。
/// * `align_drop`：延迟对齐丢弃量（采样）。默认 `None` 时取
///   [`default_align_drop`] 的 pilot 实测值（不用 `output_latency()` 声称值，
///   见该函数文档）；调用方传入实测值可覆盖。
///
/// 固定 240 采样输出块；每块输入步长 = 映射差值四舍五入，钳制到
/// `[1, 960]` 并计数。映射平段（步长 ≤0，多见于钳制后的空缺间隙）
/// 喂 1 个零采样（拉伸器自然衰减后归于静音，不喂冻结采样 drones）。
/// 生成 `target_len + align_drop` 个输出后丢弃前 `align_drop` 个，
/// 取接下来 `target_len` 个（延迟对齐）。
pub fn render_continuous_timemap(
    source: &[f32],
    sample_rate: u32,
    map: &PiecewiseTimeMap,
    target_len: usize,
    config: StretchConfig,
    align_drop: Option<usize>,
) -> (Vec<f32>, RenderReport) {
    let mut stretch = match config {
        StretchConfig::Custom2400_480 => Stretch::new(1, 2400, 480),
        StretchConfig::PresetDefault => Stretch::preset_default(1, sample_rate),
    };
    let drop = align_drop.unwrap_or_else(|| default_align_drop(config, sample_rate));
    let total = target_len + drop;
    let mut out_all: Vec<f32> = Vec::with_capacity(total);
    let mut report = RenderReport::default();
    let mut in_pos = map.map(0.0);
    let mut out_cursor = 0usize;
    while out_cursor < total {
        let chunk = OUT_CHUNK.min(total - out_cursor);
        let out_end_s = (out_cursor + chunk) as f64 / sample_rate as f64;
        let in_next = map.map(out_end_s);
        let raw_step = ((in_next - in_pos) * sample_rate as f64).round() as i64;
        let mut step = raw_step;
        // 平段（映射无输入推进，多见于钳制后的空缺间隙）：不喂冻结采样
        // （直流 drones），喂 1 个零采样（拉伸器自然衰减后归于静音），
        // 但仍计数为钳制事件（details 里 requested=0 可辨认）。
        let input = if raw_step <= 0 {
            if report.step_clamp_details.len() < 32 {
                report.step_clamp_details.push((out_cursor, 0, MIN_STEP));
            }
            report.step_clamps += 1;
            vec![0.0_f32; MIN_STEP]
        } else {
            if step < MIN_STEP as i64 {
                if report.step_clamp_details.len() < 32 {
                    report
                        .step_clamp_details
                        .push((out_cursor, step.max(0) as usize, MIN_STEP));
                }
                report.step_clamps += 1;
                step = MIN_STEP as i64;
            } else if step > (OUT_CHUNK * MAX_STEP_MULTIPLIER) as i64 {
                if report.step_clamp_details.len() < 32 {
                    report.step_clamp_details.push((
                        out_cursor,
                        step as usize,
                        OUT_CHUNK * MAX_STEP_MULTIPLIER,
                    ));
                }
                report.step_clamps += 1;
                step = (OUT_CHUNK * MAX_STEP_MULTIPLIER) as i64;
            }
            let step = step as usize;
            let in_start = (in_pos * sample_rate as f64).round() as i64;
            let mut input = vec![0.0_f32; step];
            for (k, slot) in input.iter_mut().enumerate() {
                let idx = in_start + k as i64;
                if idx >= 0 && (idx as usize) < source.len() {
                    *slot = source[idx as usize];
                }
            }
            input
        };
        let mut output = vec![0.0_f32; chunk];
        stretch.process(&input, &mut output);
        out_all.extend_from_slice(&output);
        in_pos = in_next;
        out_cursor += chunk;
    }
    let start = drop.min(out_all.len());
    let end = (drop + target_len).min(out_all.len());
    let mut rendered = out_all[start..end].to_vec();
    rendered.resize(target_len, 0.0);
    report.out_len = rendered.len();
    (rendered, report)
}

/// 门检 verdict（专家 Q11/Q15）。
#[derive(Debug, Clone, PartialEq)]
pub struct GateVerdict {
    /// 是否通过全部阈值。
    pub pass: bool,
    /// 行级纯歌词跨度比（score 歌词跨度 / sung 歌词跨度；>1 表示需拉长）。
    pub line_ratio: f64,
    /// 浊音核区间中落在 `[0.70, 1.40]` 内的比例。
    pub in_range_fraction: f64,
    /// 超出 `[0.55, 1.80]` 的浊音核区间数（任一即触发重抽）。
    pub out_of_hard_count: usize,
    /// 失败原因（空表示通过）。
    pub failures: Vec<String>,
}

/// 门检：行级跨度比 + 浊音核区间速率分布。
///
/// * `line_ratio`：行级纯歌词跨度比（score 跨度 / sung 跨度；>1 表示需拉长）。
/// * `core_rates`：各浊音核区间的需求拉伸倍数（钳制前，来自 [`TimemapBuild`]）。
///
/// 通过条件：`line_ratio ∈ [0.80, 1.25]`；≥90% 核区间倍数 ∈ `[0.70,
/// 1.40]`；无核区间超出 `[0.55, 1.80]`。任一失败 ⇒ 调用方触发
/// `SynthesisRetryRequired`（换 Seed 局部重抽该行，有界重试）。
pub fn gate_verdict(line_ratio: f64, core_rates: &[f64]) -> GateVerdict {
    let mut failures = Vec::new();
    if !(0.80..=1.25).contains(&line_ratio) {
        failures.push(format!("line ratio {line_ratio:.3} outside [0.80, 1.25]"));
    }
    let in_range = core_rates
        .iter()
        .filter(|rate| (0.70..=1.40).contains(*rate))
        .count();
    let fraction = if core_rates.is_empty() {
        1.0
    } else {
        in_range as f64 / core_rates.len() as f64
    };
    if fraction < 0.90 {
        failures.push(format!(
            "only {fraction:.3} of voiced cores in [0.70, 1.40], need >= 0.90"
        ));
    }
    let out_of_hard = core_rates
        .iter()
        .filter(|rate| !(0.55..=1.80).contains(*rate))
        .count();
    if out_of_hard > 0 {
        failures.push(format!("{out_of_hard} voiced cores outside [0.55, 1.80]"));
    }
    GateVerdict {
        pass: failures.is_empty(),
        line_ratio,
        in_range_fraction: fraction,
        out_of_hard_count: out_of_hard,
        failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear_map(rate: f64) -> PiecewiseTimeMap {
        PiecewiseTimeMap::new(vec![(0.0, 0.0), (10.0, 10.0 / rate)])
    }

    #[test]
    fn map_interpolates_and_extends() {
        let map = linear_map(1.5);
        assert!((map.map(2.0) - 2.0 / 1.5).abs() < 1e-9);
        // 区间外按斜率 1.0 延伸（预卷按原速），不是按映射斜率。
        assert!((map.map(-1.0) - (-1.0)).abs() < 1e-9);
        assert!((map.rate(0.0, 2.0) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn gate_pass_and_fail() {
        let pass = gate_verdict(1.0, &[1.0, 1.1, 0.9, 1.2]);
        assert!(pass.pass);
        let fail_ratio = gate_verdict(1.447, &[1.0, 1.1]);
        assert!(!fail_ratio.pass);
        let fail_hard = gate_verdict(1.0, &[1.0, 4.286]);
        assert!(!fail_hard.pass);
        assert_eq!(fail_hard.out_of_hard_count, 1);
    }

    #[test]
    fn build_single_syllable_locks() {
        let sr = 48_000;
        let source = vec![0.0_f32; sr * 2];
        let syllables = [SyllableIn {
            pre_s: 0.5,
            vop_s: 0.6,
            off_s: 1.0,
        }];
        let targets = [SyllableTarget { vop_s: 0.6 }];
        let build = build_timemap(
            &source,
            sr as u32,
            &syllables,
            &targets,
            1.1,
            2.0,
            TimemapOptions::default(),
        );
        // 锁窗内斜率 1.0：输入差应等于输出差。
        let delta = build.map.map(0.59) - build.map.map(0.49);
        assert!(
            (delta - 0.10).abs() < 1e-6,
            "lock window slope must be 1.0, got {delta}"
        );
        assert!(build.clamps.is_empty());
    }

    #[test]
    fn build_gap_silence_absorbs_slack() {
        let sr = 48_000;
        // 两音节：第一音节核 0.20 s，第二音节目标 VOP 较远，间隙含静音。
        let mut source = vec![0.0_f32; sr * 4];
        for sample in source.iter_mut().take(sr) {
            *sample = 0.5;
        }
        let syllables = [
            SyllableIn {
                pre_s: 0.5,
                vop_s: 0.6,
                off_s: 1.0,
            },
            SyllableIn {
                pre_s: 2.0,
                vop_s: 2.1,
                off_s: 2.5,
            },
        ];
        let targets = [SyllableTarget { vop_s: 0.6 }, SyllableTarget { vop_s: 2.6 }];
        let build = build_timemap(
            &source,
            sr as u32,
            &syllables,
            &targets,
            3.0,
            4.0,
            TimemapOptions::default(),
        );
        // 第一音节核应保持 1.0（静音吸收松弛），无钳制。
        let cores: Vec<&BuiltInterval> = build
            .intervals
            .iter()
            .filter(|interval| interval.is_voiced_core)
            .collect();
        assert_eq!(cores.len(), 2);
        assert!(
            (cores[0].rate - 1.0).abs() < 1e-6,
            "first core must stay 1.0, got {}",
            cores[0].rate
        );
        assert!(build.clamps.is_empty());
    }

    #[test]
    fn render_identity_is_deterministic() {
        let sr = 48_000;
        let mut source = vec![0.0_f32; sr];
        for (i, sample) in source.iter_mut().enumerate() {
            *sample = (i as f32 * 0.001).sin() * 0.5;
        }
        let map = linear_map(1.0);
        let (a, report_a) = render_continuous_timemap(
            &source,
            sr as u32,
            &map,
            sr / 2,
            StretchConfig::Custom2400_480,
            None,
        );
        let (b, _) = render_continuous_timemap(
            &source,
            sr as u32,
            &map,
            sr / 2,
            StretchConfig::Custom2400_480,
            None,
        );
        assert_eq!(a.len(), sr / 2);
        assert_eq!(a, b, "render must be deterministic");
        assert_eq!(report_a.step_clamps, 0);
    }

    /// 抖动诊断探针（REPORT-jitter-diagnosis.md）：速率 > 2.0 的拉伸在逐轮
    /// 之间逐位不同，速率 ≤ 2.0 的逐位相同。
    ///
    /// 机理（signalsmith-stretch 0.1.3，`signalsmith-stretch.h`）：
    /// 默认构造以 `std::random_device{}()` 播种 RNG（:38），Rust 绑定
    /// （`src/wrapper.cpp`）只用默认构造，从不调播种构造（:39）；
    /// 块 `timeFactor`（≈ 局部拉伸倍数）> `maxCleanStretch`（2，:484）
    /// 时，逐 bin 从该 RNG 取随机时间因子（:614–615，:724，:744）。
    /// 速率 ≤ 2.0 时三目取固定分支，不取数，故逐位确定。
    ///
    /// 3.0 臂钉住“修前不同”：修法（固定种子）落地后须改为 `assert_eq!`，
    /// 与 1.5 臂同形；两臂合起来即修后逐轮逐位相同的回归判据。
    #[test]
    fn render_rate_3p0_differs_across_instances() {
        let sr = 48_000;
        let mut source = vec![0.0_f32; sr];
        for (i, sample) in source.iter_mut().enumerate() {
            *sample = (i as f32 * 0.001).sin() * 0.5;
        }
        let map = linear_map(3.0);
        let (a, _) = render_continuous_timemap(
            &source,
            sr as u32,
            &map,
            sr,
            StretchConfig::Custom2400_480,
            None,
        );
        let (b, _) = render_continuous_timemap(
            &source,
            sr as u32,
            &map,
            sr,
            StretchConfig::Custom2400_480,
            None,
        );
        assert_eq!(a.len(), sr);
        let diffs = a.iter().zip(b.iter()).filter(|(x, y)| x != y).count();
        eprintln!("rate 3.0: differing samples {diffs} / {}", a.len());
        assert_ne!(
            a, b,
            "rate 3.0 renders must differ across instances (pre-fix probe)"
        );
    }

    #[test]
    fn render_rate_1p5_is_deterministic() {
        let sr = 48_000;
        let mut source = vec![0.0_f32; sr];
        for (i, sample) in source.iter_mut().enumerate() {
            *sample = (i as f32 * 0.001).sin() * 0.5;
        }
        let map = linear_map(1.5);
        let (a, _) = render_continuous_timemap(
            &source,
            sr as u32,
            &map,
            sr,
            StretchConfig::Custom2400_480,
            None,
        );
        let (b, _) = render_continuous_timemap(
            &source,
            sr as u32,
            &map,
            sr,
            StretchConfig::Custom2400_480,
            None,
        );
        assert_eq!(a, b, "rate 1.5 renders must be deterministic");
    }

    #[test]
    fn render_burst_alignment_identity_and_stretched() {
        // 指标：突发包络峰在输出中的位置（采样）与映射预测位置之差（毫秒）。
        // 身份映射与 1.5 倍拉伸区各测一次；两种拉伸配置各测一次。
        let sr = 48_000;
        let burst_start = sr * 3 / 2;
        let burst_len = sr * 50 / 1000;
        let mut source = vec![0.0_f32; sr * 4];
        for (i, sample) in source
            .iter_mut()
            .enumerate()
            .skip(burst_start)
            .take(burst_len)
        {
            let t = (i - burst_start) as f32 / sr as f32;
            *sample = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.8;
        }
        let peak_at = |out: &[f32], center: usize, radius: usize| -> usize {
            let start = center.saturating_sub(radius);
            let end = (center + radius).min(out.len());
            let mut best = start;
            let mut best_value = 0.0_f32;
            for (i, sample) in out.iter().enumerate().take(end).skip(start) {
                if sample.abs() > best_value {
                    best_value = sample.abs();
                    best = i;
                }
            }
            best
        };
        for config in [StretchConfig::Custom2400_480, StretchConfig::PresetDefault] {
            let identity = linear_map(1.0);
            let (out, _) =
                render_continuous_timemap(&source, sr as u32, &identity, sr * 3, config, None);
            let peak = peak_at(&out, burst_start + burst_len / 2, sr / 20);
            let error_ms = (peak as i64 - (burst_start + burst_len / 2) as i64).abs() as f64
                / sr as f64
                * 1000.0;
            // Custom 在身份映射同样 ≤15 ms；Preset 的 120 ms 大块把孤立突发峰
            // 抹开：Linux 上恰好过 15 ms 界，Windows／macOS 上实测 25.0 ms
            // （平台方差，CI 41448139686 两腿同错），故 PresetDefault 身份臂
            // 只钉 <60 ms 回归界，与拉伸臂的 PresetDefault 界一致；该界仍在
            // ADR 注明的 Preset 局限（拉伸突发峰约 48.8 ms）量级内，不当作
            // Phase 1 默认配置的验收标准（真实人声 VOP 是持续能量上升沿，
            // 以行 0 pilot 的带上升沿实测为准）。
            let bound = match config {
                StretchConfig::Custom2400_480 => 15.0,
                StretchConfig::PresetDefault => 60.0,
            };
            assert!(
                error_ms <= bound,
                "identity burst alignment error {error_ms:.1}ms too large ({config:?})"
            );
            let stretched =
                PiecewiseTimeMap::new(vec![(0.0, 0.0), (1.0, 1.0), (3.0, 1.0 + 2.0 / 1.5)]);
            let (out, _) =
                render_continuous_timemap(&source, sr as u32, &stretched, sr * 3, config, None);
            // 输入中心 1.525 s 处突发 → 输出 1.0 + 0.525 × 1.5 = 1.7875 s。
            let input_center_s = 1.5 + 0.025;
            let expected = ((1.0 + (input_center_s - 1.0) * 1.5) * sr as f64) as usize;
            let peak = peak_at(&out, expected, sr / 20);
            let error_ms = (peak as i64 - expected as i64).abs() as f64 / sr as f64 * 1000.0;
            // Custom 在拉伸区同样 ≤15 ms；Preset 的 120 ms 大块在速率变化处
            // 把孤立突发峰抹开约 49 ms（实测），故只钉 <60 ms 回归界，不当
            // 作 Phase 1 默认配置的验收标准（真实人声 VOP 是持续能量上升沿，
            // 以行 0 pilot 的带上升沿实测为准）。
            let bound = match config {
                StretchConfig::Custom2400_480 => 15.0,
                StretchConfig::PresetDefault => 60.0,
            };
            assert!(
                error_ms <= bound,
                "stretched burst alignment error {error_ms:.1}ms too large ({config:?})"
            );
        }
    }

    #[test]
    fn render_silence_stays_silence() {
        let sr = 48_000;
        let source = vec![0.0_f32; sr];
        let map = linear_map(2.0);
        let (out, _) = render_continuous_timemap(
            &source,
            sr as u32,
            &map,
            sr / 4,
            StretchConfig::Custom2400_480,
            None,
        );
        let peak = out
            .iter()
            .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));
        assert!(peak < 1e-4, "silence in must stay silence, peak {peak}");
    }
}
