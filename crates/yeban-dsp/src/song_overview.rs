//! 整曲多层可视化总览图：离线四层分析（设计稿 `yeban-songmap-design.md` Phase 1）。
//!
//! 本模块是纯算法层：输入谱面（[`ScoreInput`]）与渲染音频（[`AudioInput`]），
//! 输出小节级列式序列（[`SongOverview`]，设计稿 §8.2 的 JSON 形状）。
//! UI 只画数、不算数；MCP 只搬运本模块的输出。
//!
//! 四层（设计稿 §2）：
//!
//! - 情绪层：5 维固定锚点加权（响度 0.30、密度 0.20、音区 0.20、和声 0.15、
//!   配器 0.15）+ 一维双边滤波（r = 2 拍，σ_v = 0.18），段落 Marker 处硬切断。
//!   **严禁全曲 Min-Max 动态拉伸**——所有归一化锚点都是固定心理声学常数。
//! - 频率层：4096 点 Hann 窗 STFT、50% 重叠、8 频段、+4.5 dB/oct 粉红倾斜补偿，
//!   按拍均方池化。FFT 用 [`realfft`]（纯 Rust 确定性实现）；旧实现复用的
//!   [`crate::math::fft`]（f64 朴素基 2）实测 4 分钟全曲 2.37 s，未达设计稿
//!   §9.1 的 30 ms 目标，故换 realfft（单次 4096 点约 2.2 μs）。数值须与旧
//!   实现逐位一致，由集成测试的已知答案断言钉死。
//! - 能量层：复用 [`crate::loudness::GatedLoudness`] 的 BS.1770-4 实现
//!   （瞬时 400 ms、短时 3 s），不重写 LUFS、不引 ebur128。
//! - 疏密层：旋律与打击分轨；旋律组用 Clustered Onset 率（同拍 30 ms 内柱式
//!   和弦记 1 次）+ 占空比，打击组只算 Onset 率。1 小节滑窗、1 拍步进。
//!
//! 诊断（[`Anomaly`]，MCP 与 UI 共用）：低频中段淤积（`low_mid_buildup`）、
//! 总线压缩压扁（`compression_squash`）与期望带偏离（`envelope_deviation`，
//! Phase 2），均为纯逻辑阈值判定；段内自相似度（[`analyze_repetition`]）
//! 找跨段重复（`arrangement_repetition`，设计稿 §8.2 追加，schema 1.1）；
//! 分轨频率打架（[`detect_frequency_clash`]，设计稿 §8.2 追加，schema 1.2）
//! 按拍按频段比各轨能量，次响轨距最响轨 6 dB 内且高于可听门限即记打架。
//! 谱面力度虚线（设计稿 §5.1）直接画 `bar_series.score_dyn` 列，不另算。
//!
//! Phase 2（设计稿 §3.5/§3.6/§5.1）：曲风预设 3 档 + 方差自适应权重
//! （[`TensionProfile`] / [`adaptive_weights`]）、期望结构包络带与偏离告警
//! （[`AnalysisOptions::expected_levels`] / [`envelope_band`]）、谱面力度列
//! 已存在、虚线由 UI 画。分析选项经 [`AnalysisOptions`] 传入，默认选项与
//! Phase 1 逐位一致。
//!
//! 约束：离线分析，不在实时音频回调里调用；确定性——固定锚点、固定阈值，
//! 同一输入跨进程结果一致；不写 `unsafe`；持久化集合只用 `BTreeMap`/`BTreeSet`
//! （本模块实际只用 `Vec` 与定长数组）。

use crate::loudness::GatedLoudness;
use realfft::RealFftPlanner;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// 设计稿 schema 版本（§8.2；1.1 起加 `repetition` 与 `arrangement_repetition`；
/// 1.2 起加 `frequency_clash`）。
pub const SCHEMA_VERSION: &str = "1.2";

/// 8 频段边界（Hz），顺序为 Sub → Air（设计稿 §4.1）。
pub const BAND_DEFINITIONS_HZ: [[f32; 2]; 8] = [
    [20.0, 90.0],
    [90.0, 250.0],
    [250.0, 500.0],
    [500.0, 1000.0],
    [1000.0, 2000.0],
    [2000.0, 5000.0],
    [5000.0, 10000.0],
    [10000.0, 20000.0],
];

/// STFT 窗长（点数，设计稿 §4.4）。
pub const STFT_WINDOW: usize = 4096;
/// STFT 步进（点数，50% 重叠，设计稿 §4.4）。
pub const STFT_HOP: usize = 2048;
/// 粉红倾斜补偿斜率（dB/oct，设计稿 §4.3）。
pub const TILT_DB_PER_OCTAVE: f32 = 4.5;
/// 频段能量下限（dB）：该频段无可测能量时记此值。
pub const BAND_DB_FLOOR: f32 = -120.0;
/// 响度无读数哨兵（LUFS）：窗口未满、静音或无音频时记此值
/// （绝对门限 −70 LUFS，见 [`crate::loudness`])。
pub const NO_LOUDNESS_READING_LUFS: f32 = -70.0;

/// 情绪张力权重（设计稿 §3.2）：感知响度。
pub const WEIGHT_LOUDNESS: f32 = 0.30;
/// 情绪张力权重（设计稿 §3.2）：事件密度。
pub const WEIGHT_DENSITY: f32 = 0.20;
/// 情绪张力权重（设计稿 §3.2）：音区高度。
pub const WEIGHT_PITCH: f32 = 0.20;
/// 情绪张力权重（设计稿 §3.2）：和声张力。
pub const WEIGHT_HARMONY: f32 = 0.15;
/// 情绪张力权重（设计稿 §3.2）：配器激活厚度。
pub const WEIGHT_ORCHESTRATION: f32 = 0.15;

/// 曲风预设档（设计稿 §3.6，Phase 2）：三档权重。
///
/// - Default：设计稿 §3.2 的固定默认权重（均衡流行摇滚）。
/// - Orchestral：管弦 / 原声叙事 —— 音区 +0.10、和声 +0.05（叙事靠音区抬升与
///   和声张力推高潮）；响度 −0.05、密度 −0.05、配器 −0.05。
/// - Electronic：电子 / 律动 —— 响度 +0.05、密度 +0.10；音区 −0.10、和声 −0.05。
///   诚实口径：Phase 1 公式里**没有**频谱亮度分量，亮度权重无处可落；
///   滤波打开时响度感知与打击密度同向抬升，故亮度落到响度 + 密度两项上。
///   真正的频谱质心分量是后一轮的事，不在本轮假装已算。
///
/// 三档权重和恒为 1.0。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TensionProfile {
    /// 均衡流行摇滚（默认）。
    #[default]
    Default,
    /// 管弦 / 原声叙事。
    Orchestral,
    /// 电子 / 律动。
    Electronic,
}

impl TensionProfile {
    /// 档名（进 `song_meta.profile`，与 MCP `profile` 参数同一组词）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Orchestral => "orchestral",
            Self::Electronic => "electronic",
        }
    }

    /// 按档名解析；未知档名返回 `None`（调用方响亮拒绝，不静默回退）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "default" => Some(Self::Default),
            "orchestral" => Some(Self::Orchestral),
            "electronic" => Some(Self::Electronic),
            _ => None,
        }
    }

    /// 本档的五维权重 `[响度, 密度, 音区, 和声, 配器]`，和恒为 1.0。
    #[must_use]
    pub const fn weights(self) -> [f32; 5] {
        match self {
            Self::Default => [
                WEIGHT_LOUDNESS,
                WEIGHT_DENSITY,
                WEIGHT_PITCH,
                WEIGHT_HARMONY,
                WEIGHT_ORCHESTRATION,
            ],
            Self::Orchestral => [0.25, 0.15, 0.30, 0.20, 0.10],
            Self::Electronic => [0.35, 0.30, 0.10, 0.10, 0.15],
        }
    }
}

/// 方差自适应衰减的 σ 门限（设计稿 §3.6）：某特征全曲标准差 ≤ 0.05 时视为
/// “死特征”（如单一和弦 Loop 的和声张力几乎不变）。
pub const ADAPTIVE_SIGMA_THRESHOLD: f32 = 0.05;
/// 方差自适应衰减的幅度：死特征的权重衰减 50%。
pub const ADAPTIVE_ATTENUATION: f32 = 0.5;

/// 方差自适应权重（设计稿 §3.6）：死特征权重 × 0.5，腾出的权重按比例重分配
/// 给方差较大的活跃特征（按活跃特征衰减后的权重占比分）。
///
/// 全部特征都死（全曲无方差，如单小节）时无处可分，衰减后的权重重归一到和为 1
/// —— 此时衰减是均匀的 × 0.5，重归一后等于原权重，不编造信息。
/// 返回的权重和恒为 1.0（输入权重和为 1 时）。纯函数、确定性。
#[must_use]
pub fn adaptive_weights(base: [f32; 5], std_devs: [f32; 5]) -> [f32; 5] {
    let mut attenuated = [0.0f32; 5];
    let mut removed = 0.0f32;
    for i in 0..5 {
        if std_devs[i] <= ADAPTIVE_SIGMA_THRESHOLD {
            attenuated[i] = base[i] * ADAPTIVE_ATTENUATION;
            removed += base[i] - attenuated[i];
        } else {
            attenuated[i] = base[i];
        }
    }
    if removed <= 0.0 {
        return base;
    }
    let active_sum: f32 = attenuated
        .iter()
        .zip(std_devs.iter())
        .filter(|(_, sigma)| **sigma > ADAPTIVE_SIGMA_THRESHOLD)
        .map(|(&w, _)| w)
        .sum();
    if active_sum <= 0.0 {
        // 全部特征都死：重归一（均匀 × 0.5 ⇒ 回到原权重）。
        let sum: f32 = attenuated.iter().sum();
        if sum > 0.0 {
            return attenuated.map(|w| w / sum);
        }
        return base;
    }
    let mut out = attenuated;
    for (i, w) in out.iter_mut().enumerate() {
        if std_devs[i] > ADAPTIVE_SIGMA_THRESHOLD {
            *w += removed * attenuated[i] / active_sum;
        }
    }
    out
}

/// 期望结构包络带的半宽（张力单位，Phase 2 择优值）：期望等级 L（1–5 级）的
/// 中心为 `0.1 + 0.2 × (L − 1)`（L1 = 0.1、L3 = 0.5、L5 = 0.9），带为
/// 中心 ± 0.15，夹到 [0, 1]。半宽 0.15 的理由：双边滤波的值域 σ_v = 0.18，
/// 小于 0.15 的偏离会被滤波抹平、不是结构信号；带宽 0.30 覆盖相邻等级间距
/// （0.2）以上，避免相邻等级的带互相打架。
pub const ENVELOPE_HALF_WIDTH: f32 = 0.15;
/// 期望带偏离告警阈值（张力单位，Phase 2 择优值）：实际张力超出期望带边界
/// 0.15 以上即告警。取 0.15 的理由：与半宽同量级 —— 偏离超过一整个半宽
/// 才算结构塌陷（如标了高潮、实际塌掉），带边附近的抖动（< 0.15）只记
/// `target_deviation` 列、不进异常摘要。
pub const ENVELOPE_DEVIATION_ALERT: f32 = 0.15;
/// 包络带缺失哨兵：该小节无期望等级时 `envelope_lo` / `envelope_hi` 记此值
/// （张力域为 [0, 1]，−1 在域外，不会与真带混淆）。
pub const NO_ENVELOPE: f32 = -1.0;

/// 期望等级（1–5 级）→ 期望带 `[下沿, 上沿]`（夹到 [0, 1]）；等级越界时
/// 返回 `None`（无带，不编造）。
#[must_use]
pub fn envelope_band(level: u8) -> Option<(f32, f32)> {
    if !(1..=5).contains(&level) {
        return None;
    }
    let center = 0.1 + 0.2 * f32::from(level - 1);
    Some((
        (center - ENVELOPE_HALF_WIDTH).max(0.0),
        (center + ENVELOPE_HALF_WIDTH).min(1.0),
    ))
}

/// 实际张力相对期望带的偏离量（≥ 0）：带内为 0，带外为到最近边界的距离。
#[must_use]
pub fn envelope_deviation(tension: f32, band_lo: f32, band_hi: f32) -> f32 {
    if band_lo < 0.0 || band_hi < 0.0 {
        return 0.0;
    }
    (band_lo - tension).max(tension - band_hi).max(0.0)
}

/// 响度归一化锚点下限（LUFS，设计稿 §3.2）：极弱背景。
pub const LOUDNESS_ANCHOR_MIN_LUFS: f32 = -36.0;
/// 响度归一化锚点上限（LUFS，设计稿 §3.2）：母带级满载高潮。
pub const LOUDNESS_ANCHOR_MAX_LUFS: f32 = -8.0;
/// 音区归一化锚点下限（MIDI，设计稿 §3.2）：C2。
pub const PITCH_ANCHOR_MIN_MIDI: f32 = 36.0;
/// 音区归一化锚点上限（MIDI，设计稿 §3.2）：C6。
pub const PITCH_ANCHOR_MAX_MIDI: f32 = 84.0;
/// 密度归一化锚点（音符/秒，设计稿 §3.2）：120 BPM 下双轨十六分音符满铺。
pub const DENSITY_ANCHOR_NOTES_PER_SEC: f32 = 16.0;

/// 双边滤波时间核半径（拍，设计稿 §3.4）。
pub const BILATERAL_RADIUS_BEATS: usize = 2;
/// 双边滤波值域高斯核 σ_v（设计稿 §3.4）。
pub const BILATERAL_SIGMA_V: f32 = 0.18;

/// 柱式和弦聚类窗（秒，设计稿 §6.1）：同一拍内 30 ms 内的齐奏记为 1 次触发。
pub const CLUSTER_WINDOW_SECONDS: f32 = 0.030;

/// 粗糙度未归一化求和的参考锚点（设计稿 §3.3 未定此常数，此处钉死）：
/// C4 小二度双音（6 谐波、振幅 1/k、单位力度）的未归一化粗糙度约为 0.25，
/// 锚点取其两倍，使典型小二度落在 0.5 附近；密集低音簇可饱和到 1.0。
/// 单音（只有自身谐波对）的求和约为 0.0002，落在 0 附近。
pub const ROUGHNESS_REFERENCE: f64 = 0.5;

/// 低频中段淤积诊断阈值（dB）：Band 3 超过其余 7 频段均值多少即记超标。
pub const LOW_MID_EXCESS_DB: f32 = 4.0;
/// 诊断所需的最短连续小节数。
pub const DIAGNOSTIC_MIN_BARS: usize = 4;

/// 分轨频率打架：可听门限（dBFS，设计稿 §8.2 追加，schema 1.2）：低于满刻度
/// 60 dB 的频段能量在任何现实混音下都听不见，不参评；混音频段下限是 −120 dB，
/// −60 取在静音/底噪区与可听区之间。
pub const CLASH_AUDIBILITY_FLOOR_DB: f32 = -60.0;
/// 分轨频率打架：重叠度重档下限（Wang 2026-11-11 13:17 定：输出以频谱重叠度
/// 为主，6 dB 不再是定性硬门限，只作换算参考点）：重叠度 > 0.5 即重档——
/// 第二强轨能量超最强轨一半（差距 < 3 dB），两轨在该频段近乎等强、强互掩蔽。
pub const CLASH_SEVERE_OVERLAP: f32 = 0.5;
/// 分轨频率打架：重叠度中档下限：重叠度 0.25–0.5 为中档（差距 3–6 dB，
/// 第二强轨能量为最强轨的四分之一到一半，清晰可闻、部分掩蔽）；低于 0.25
/// 为轻档（差距 > 6 dB，第二强轨在该频段处于被掩蔽侧，但仍是真实测得的重叠，
/// 照样计入输出）。3 dB 与 6 dB 是能量比的自然锚点（一半、四分之一）。
pub const CLASH_MODERATE_OVERLAP: f32 = 0.25;
/// 分轨频率打架：每条轨在某拍的某频段参评的前提——该频段能量距该轨当拍最响
/// 频段不得超过 20 dB。没有这一条，Hann 窗旁瓣泄漏（约低于峰值频段 30 dB 以上）
/// 会让一条纯音在相邻频段也"参评"，同频段测试会多报 2 条假打架；20 dB 既拦
/// 住泄漏，又留住轨的真实次要频段内容（典型在峰值 20 dB 以内）。
pub const CLASH_TRACK_BAND_FLOOR_DB: f32 = 20.0;
/// 分轨频率打架：每曲最多报几条（按重叠度降序截断；截断前的组数进
/// [`FrequencyClashSummary::total_groups`]）。
pub const CLASH_MAX_ENTRIES: usize = 20;

/// 段内自相似度：重复判定阈值（余弦相似度；设计稿 §8.2 追加：重复段检查）。
pub const REPEAT_SIMILARITY: f32 = 0.95;
/// 段内自相似度：成段重复的最短小节数（短于此只算撞分，不报段落）。
pub const MIN_REPEAT_BARS: usize = 4;
/// 段内自相似度：邻居间隔下限（小节）：间隔小于此的对是同质邻居，
/// 不是跨段重复，候选时排除。
pub const NEIGHBOR_GAP_BARS: usize = 4;
/// 段内自相似度：`best_matches` 最多保留几对。
pub const BEST_MATCHES_LIMIT: usize = 5;
/// 段内自相似度：疏密 Onset 率的固定锚点（次/拍）：`DENSITY_ANCHOR_NOTES_PER_SEC`
/// （16 音符/秒）在 [`DEFAULT_BPM`]（120 BPM，0.5 秒/拍）下折合 8 次/拍。
pub const DENSITY_ONSETS_PER_BEAT_ANCHOR: f32 = 8.0;
/// 压缩压扁诊断阈值：谱面力度在窗口内至少上升这么多（0–1 归一化力度）。
pub const SQUASH_SCORE_RISE: f32 = 0.25;
/// 压缩压扁诊断阈值：渲染 LUFS 在窗口内至多上升这么多（LU）。
pub const SQUASH_LUFS_RISE_MAX: f32 = 0.5;

/// 默认速度（BPM）：谱面未给速度图时用。
pub const DEFAULT_BPM: f32 = 120.0;
/// 默认每小节拍数：谱面未给时用 4/4。
pub const DEFAULT_BEATS_PER_BAR: u32 = 4;

/// 半音名（C = 0），用于 [`SongMeta::key_signature`]。
pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

// ---------------------------------------------------------------------------
// 输入：ScoreInput / AudioInput
// ---------------------------------------------------------------------------

/// 一个音符事件（谱面输入的最小单位）。
///
/// 时间一律用**拍**（beat），音高用 MIDI 音高号，力度用 0–127。
/// 拍是音乐时间：同一拍值在不同 BPM 下对应不同的物理秒，换算由速度图负责。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEvent {
    /// 起始拍（≥ 0）。
    pub start_beat: f32,
    /// 时值（拍；≤ 0 的音符只计 Onset，不计发声）。
    pub duration_beats: f32,
    /// MIDI 音高号（0–127）。
    pub midi: u8,
    /// 力度（0–127）。
    pub velocity: u8,
    /// 轨道编号（用于配器厚度统计中的活跃轨道数）。
    pub track: u32,
    /// 是否打击乐轨道（疏密层分轨的依据；打击组只算 Onset 率）。
    pub is_percussion: bool,
}

/// 速度图中的一次变速。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoChange {
    /// 变速发生的拍（≥ 0）。
    pub beat: f32,
    /// 新速度（BPM，必须 > 0）。
    pub bpm: f32,
}

/// 段落标记（Marker）：双边滤波的硬切断边界，也是小节 `section` 标签的来源。
///
/// `expected_level` 是 Phase 2 的期望张力等级（设计稿 §3.5）：1–5 级，
/// `None` = 用户未标注（无期望带）。越界值（0 或 > 5）在分析时按无带处理，
/// 不编造。标注入口见 [`AnalysisOptions::expected_levels`]（MCP 参数）与
/// 面板标尺点击（UI 入口）；工程 `sections` 本身不存等级（持久化模型未改），
/// 等级只活在分析请求的会话态里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionMarker {
    /// 段落起始小节（1-based）。
    pub bar: u32,
    /// 段落名（如 "Intro"、"Chorus"）。
    pub label: String,
    /// 期望张力等级（1–5 级；`None` = 未标注）。
    pub expected_level: Option<u8>,
}

/// Phase 2 分析选项（`analyze_song_overview` 的默认选项 = 全默认）。
///
/// `profile` 是曲风预设档（设计稿 §3.6）；`expected_levels` 是按段落名给的
/// 期望张力等级表（设计稿 §3.5，MCP `expected_levels` 参数的原样），只在
/// 分析请求里生效：匹配到的 Marker 按此表设 `expected_level`，未匹配到的
/// Marker 保持原值（直接写在 `SectionMarker.expected_level` 里的等级）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AnalysisOptions {
    /// 曲风预设档（默认 [`TensionProfile::Default`]）。
    pub profile: TensionProfile,
    /// 按段落名给的期望张力等级（段落名 → 1–5 级；越界值忽略）。
    pub expected_levels: std::collections::BTreeMap<String, u8>,
}

impl AnalysisOptions {
    /// 只指定曲风档的选项（无期望等级表）。
    #[must_use]
    pub fn with_profile(profile: TensionProfile) -> Self {
        Self {
            profile,
            expected_levels: std::collections::BTreeMap::new(),
        }
    }
}

/// 把期望等级表写到谱面 Marker 上（按段落名精确匹配；越界等级与无匹配段落
/// 忽略）。匹配到的 Marker 以本表为准，未匹配到的保持原值。
pub fn apply_expected_levels(
    score: &mut ScoreInput,
    levels: &std::collections::BTreeMap<String, u8>,
) {
    for marker in &mut score.markers {
        if let Some(&level) = levels.get(&marker.label)
            && (1..=5).contains(&level)
        {
            marker.expected_level = Some(level);
        }
    }
}

/// 谱面输入：render/model 线把 `YebanProjectV1` 映射到本结构。
///
/// 这是 dsp 分析与上层之间的**契约**：本结构只含 plain data，不依赖
/// `yeban-model`，`yeban-dsp` 保持零模型依赖。
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreInput {
    /// 全部音符事件（无序也可，分析内部按需排序）。
    pub notes: Vec<NoteEvent>,
    /// 速度图（可空；空即全曲 [`DEFAULT_BPM`]；条目按拍升序排列，
    /// 首条不在 0 拍时 0 拍之前按 [`DEFAULT_BPM`] 计）。
    pub tempo_map: Vec<TempoChange>,
    /// 每小节拍数（0 即 [`DEFAULT_BEATS_PER_BAR`]）。
    pub beats_per_bar: u32,
    /// 调号主音的音高类别（0 = C，…，11 = B；超出范围时取模）。
    pub key_tonic_pc: u8,
    /// 是否小调（决定效价值色相的冷暖：小调为负、大调为正）。
    pub is_minor: bool,
    /// 段落标记（可空）。
    pub markers: Vec<SectionMarker>,
}

impl Default for ScoreInput {
    fn default() -> Self {
        Self {
            notes: Vec::new(),
            tempo_map: Vec::new(),
            beats_per_bar: DEFAULT_BEATS_PER_BAR,
            key_tonic_pc: 0,
            is_minor: false,
            markers: Vec::new(),
        }
    }
}

impl ScoreInput {
    /// 每小节拍数（0 时回退到 [`DEFAULT_BEATS_PER_BAR`]）。
    #[must_use]
    pub fn beats_per_bar(&self) -> u32 {
        if self.beats_per_bar == 0 {
            DEFAULT_BEATS_PER_BAR
        } else {
            self.beats_per_bar
        }
    }
}

/// 渲染音频输入：交错立体声 `f32` 切片 + 采样率。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioInput<'a> {
    /// 交错立体声采样（L, R, L, R, …）；奇数长度时末尾单个采样忽略。
    pub samples: &'a [f32],
    /// 采样率（Hz；必须 > 0 且有限，否则分析把音频当作缺失处理）。
    pub sample_rate: f32,
}

impl AudioInput<'_> {
    /// 帧数（立体声帧 = 采样数 / 2）。
    #[must_use]
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// 时长（秒；采样率非法时为 0）。
    #[must_use]
    pub fn duration_sec(&self) -> f32 {
        if self.sample_rate.is_finite() && self.sample_rate > 0.0 {
            self.frames() as f32 / self.sample_rate
        } else {
            0.0
        }
    }

    /// 是否无音频帧。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames() == 0
    }
}

// ---------------------------------------------------------------------------
// 输出：SongOverview（设计稿 §8.2 的 JSON 形状，纯 Rust structs）
// ---------------------------------------------------------------------------

/// 结构异常（diagnostics）：MCP 工具默认返回的自动提炼摘要，也是 UI 高亮的依据。
#[derive(Debug, Clone, PartialEq)]
pub struct Anomaly {
    /// 异常覆盖的小节区间（闭区间，1-based，如 `[33, 40]`）。
    pub bars: [u32; 2],
    /// 异常类型。
    pub anomaly_type: AnomalyType,
    /// 人类可读描述，含实测数值（如 "Band 3 (250-500Hz) exceeds tilted balance by +5.2 dB"）。
    pub description: String,
}

/// 结构异常的类型（设计稿 §8.2 示例列出的两类；Phase 2 加期望带偏离一类；
/// schema 1.1 加段内重复一类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnomalyType {
    /// 低频中段淤积：Band 3（250–500 Hz）持续高于其余频段均值。
    LowMidBuildup,
    /// 总线压缩压扁：谱面力度上升、渲染 LUFS 几乎不动。
    CompressionSquash,
    /// 期望带偏离（Phase 2，设计稿 §3.5）：实际张力超出用户标注的期望带
    /// 边界 [`ENVELOPE_DEVIATION_ALERT`] 以上。
    EnvelopeDeviation,
    /// 成段重复（schema 1.1，设计稿 §8.2 追加）：连续 ≥ [`MIN_REPEAT_BARS`]
    /// 小节逐小节匹配到前面某段（相似度 ≥ [`REPEAT_SIMILARITY`]）。
    ArrangementRepetition,
}

impl AnomalyType {
    /// JSON 里的类型名（设计稿 §8.2 的 `type` 字段）。
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LowMidBuildup => "low_mid_buildup",
            Self::CompressionSquash => "compression_squash",
            Self::EnvelopeDeviation => "envelope_deviation",
            Self::ArrangementRepetition => "arrangement_repetition",
        }
    }
}

/// 歌曲元数据（设计稿 §8.2 `song_meta`）。
#[derive(Debug, Clone, PartialEq)]
pub struct SongMeta {
    /// 总小节数。
    pub total_bars: u32,
    /// 总时长（秒；有音频时取音频时长，否则取谱面按速度图折算的时长）。
    pub duration_sec: f32,
    /// 基准速度（BPM；速度图首条，未给时为 [`DEFAULT_BPM`]）。
    pub base_bpm: f32,
    /// 调号（如 "D minor"、"C major"）。
    pub key_signature: String,
    /// 权重预设档：Phase 2 的 3 档（`default` / `orchestral` / `electronic`，
    /// 设计稿 §3.6）；方差自适应后的实际权重见 [`SongOverview::effective_weights`]。
    pub profile: String,
}

/// 结构摘要（设计稿 §8.2 `structural_summary`）。
#[derive(Debug, Clone, PartialEq)]
pub struct StructuralSummary {
    /// 张力最高的小节（1-based；无小节时为 0）。
    pub climax_bar: u32,
    /// 最高张力（0–1）。
    pub climax_tension: f32,
    /// 动态范围（LU：短时 LUFS 的最大值减最小值；无有效读数时为 0）。
    pub dynamic_range_lu: f32,
    /// 检测到的结构异常（与 [`SongOverview::diagnostics`] 同一份列表）。
    pub detected_anomalies: Vec<Anomaly>,
}

/// 最相似小节对（`repetition.best_matches` 的一条；设计稿 §8.2 追加，schema 1.1）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BestMatch {
    /// 较早的小节（1-based）。
    pub bar_a: u32,
    /// 较晚的小节（1-based；`bar_b - bar_a >= NEIGHBOR_GAP_BARS`）。
    pub bar_b: u32,
    /// 余弦相似度（0–1）。
    pub similarity: f32,
}

/// 成段重复（`repetition.repeated_passages` 的一条）：连续 ≥ [`MIN_REPEAT_BARS`]
/// 小节逐小节匹配到前面某段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepeatedPassage {
    /// 重复段的小节区间（闭区间，1-based）。
    pub bars: [u32; 2],
    /// 匹配到的源段区间（闭区间，1-based；与 `bars` 同长）。
    pub source_bars: [u32; 2],
    /// 该段逐小节相似度的均值（0–1）。
    pub mean_similarity: f32,
}

/// 段内自相似度摘要（设计稿 §8.2 追加：`SongOverview.repetition`，schema 1.1）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepetitionSummary {
    /// 相似度最高的前 [`BEST_MATCHES_LIMIT`] 对小节（降序；间隔 ≥ [`NEIGHBOR_GAP_BARS`]）。
    pub best_matches: Vec<BestMatch>,
    /// 机械指数（0–1）：最佳非邻匹配 ≥ [`REPEAT_SIMILARITY`] 的小节占比。
    pub mechanical_index: f32,
    /// 成段重复（连续 ≥ [`MIN_REPEAT_BARS`] 小节、逐小节匹配到前面某段）。
    pub repeated_passages: Vec<RepeatedPassage>,
}

/// 分轨频率打架检测的一条分轨输入（设计稿 §8.2 追加，schema 1.2）：渲染侧
/// 把每条源轨 solo 渲染出的音频连同轨身份一起传进来；DSP 层不认识工程模型
/// （本 crate 无 `yeban-model` 依赖），轨只以 id + name 字符串区分。
#[derive(Debug, Clone)]
pub struct StemInput<'a> {
    /// 轨 id（`EntityId` 的规范字符串）。
    pub track_id: &'a str,
    /// 轨名（显示用）。
    pub track_name: &'a str,
    /// 该轨 solo 渲染的音频（交错立体声，与混音同采样率、同长度）。
    pub audio: AudioInput<'a>,
}

/// `frequency_clash.entries[]` 里的一条轨引用。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClashTrackRef {
    /// 轨 id。
    pub track_id: String,
    /// 轨名。
    pub track_name: String,
}

/// 分轨频率打架严重度分档（由重叠度划，见 [`severity_band`]；Wang 2026-11-11
/// 13:17 定）：重 = 第二强轨能量超最强轨一半，中 = 四分之一到一半，
/// 轻 = 低于四分之一（仍是真实测得的重叠，只是不掩蔽）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClashSeverity {
    /// 重叠度 > 0.5（差距 < 3 dB）。
    Severe,
    /// 重叠度 0.25–0.5（含 0.25 与 0.5；差距 3–6 dB）。
    Moderate,
    /// 重叠度 < 0.25（差距 > 6 dB）。
    #[default]
    Mild,
}

/// 分轨频率打架的一条（`frequency_clash.entries[]` 的一条；设计稿 §8.2 追加，
/// schema 1.2；Wang 2026-11-11 13:17 定：以频谱重叠度为主，不再二值定性）：
/// 同一小节、同一频段里最响的两条轨在各拍的重叠度汇总。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrequencyClashEntry {
    /// 小节号（1-based）。
    pub bar: u32,
    /// 频段下标（0–7，Sub → Air；Hz 区间见 [`BAND_DEFINITIONS_HZ`]）。
    pub band: u8,
    /// 最响的轨（该组重叠度最大那拍的胜者）。
    pub track_a: ClashTrackRef,
    /// 次响的轨（与 `track_a` 同拍同频段参评）。
    pub track_b: ClashTrackRef,
    /// 该组各拍重叠度的最大值（0–1；1 = 两轨等能）。
    pub overlap: f32,
    /// 该组各拍重叠度的均值（0–1）。
    pub mean_overlap: f32,
    /// 两轨在该组重叠度最大那拍的频段能量差距（dB；只作参考，不定性：
    /// `overlap = 10^(-gap_db / 10)`，6 dB ↔ 0.25、3 dB ↔ 0.5）。
    pub gap_db: f32,
    /// 严重度分档（由 `overlap` 划，见 [`severity_band`]）。
    pub severity: ClashSeverity,
    /// 该小节里这对轨在该频段参评的拍数。
    pub beats: u32,
}

/// 分轨频率打架：某一小节的总体重叠度（`frequency_clash.bar_overlaps[]` 的一条）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarOverlap {
    /// 小节号（1-based）。
    pub bar: u32,
    /// 该小节全部参评（拍，频段）重叠度的均值（0–1）。
    pub overlap: f32,
    /// 该小节全部参评（拍，频段）重叠度的最大值（0–1）。
    pub max_overlap: f32,
}

/// 分轨频率打架摘要（设计稿 §8.2 追加：`SongOverview.frequency_clash`，schema 1.2；
/// Wang 2026-11-11 13:17 定：以频谱重叠度为主，不再二值定性）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrequencyClashSummary {
    /// 重叠组总数（截断前；`entries.len()` ≤ [`CLASH_MAX_ENTRIES`]）。
    pub total_groups: usize,
    /// 全曲总体重叠度：全部参评（拍，频段）重叠度的均值（0–1；无参评时 0.0）。
    pub overall_overlap: f32,
    /// 每小节总体重叠度（只含至少一处参评的小节，按小节升序）。
    pub bar_overlaps: Vec<BarOverlap>,
    /// 按重叠度降序的前 [`CLASH_MAX_ENTRIES`] 条。
    pub entries: Vec<FrequencyClashEntry>,
}

/// 小节级列式序列（设计稿 §8.2：MCP 默认返回小节级**列式**数组，
///
/// 即 `"tension": [0.24, 0.26, …]` 而非每小节重复键名；拍级数据只在调用方
/// 显式要求时返回，不在本模块的输出里）。
///
/// 各列长度恒等于总小节数；第 `i` 项对应第 `i + 1` 小节。
#[derive(Debug, Clone, PartialEq)]
pub struct BarSeries {
    /// 小节号（1-based）。
    pub bar: Vec<u32>,
    /// 小节起始时刻（秒）。
    pub time_sec: Vec<f32>,
    /// 段落名（取该小节所属的最近 Marker；无 Marker 时为 `"Untitled"`）。
    pub section: Vec<String>,
    /// 综合情绪张力（0–1，双边滤波后按小节取均值）。
    pub tension: Vec<f32>,
    /// 效价值色相（−1–1：负为冷色小调、暗淡；正为暖色大调、明亮）。
    pub valence_hue: Vec<f32>,
    /// 短时 LUFS（3 s 滑窗；无有效读数时为 [`NO_LOUDNESS_READING_LUFS`]）。
    pub lufs_short: Vec<f32>,
    /// 小节内瞬时 LUFS（400 ms 窗）的最大值；无有效读数时为哨兵值。
    pub lufs_momentary_max: Vec<f32>,
    /// 谱面力度（0–1：本小节起音的平均力度 / 127；无起音时为 0）。
    /// Phase 2 的谱面力度虚线（设计稿 §5.1）直接画这一列。
    pub score_dyn: Vec<f32>,
    /// 期望带下沿（Phase 2，设计稿 §3.5）：有期望等级的小节为带下沿，
    /// 无等级的小节为 [`NO_ENVELOPE`] 哨兵。
    pub envelope_lo: Vec<f32>,
    /// 期望带上沿（Phase 2）：有期望等级的小节为带上沿，无等级为哨兵。
    pub envelope_hi: Vec<f32>,
    /// 实际张力相对期望带的偏离量（Phase 2）：带内或无等级时为 0，
    /// 带外为到最近边界的距离；超过 [`ENVELOPE_DEVIATION_ALERT`] 即告警
    /// （`envelope_deviation` 异常，面板上按此列高亮）。
    pub target_deviation: Vec<f32>,
    /// 旋律组去重后 Onset 率（次/拍）。
    pub density_pitched_onsets_per_beat: Vec<f32>,
    /// 旋律组发声占空比（0–1）。
    pub density_pitched_duty: Vec<f32>,
    /// 打击组 Onset 率（次/拍）。
    pub density_perc_onsets_per_beat: Vec<f32>,
    /// 8 频段倾斜补偿后能量（dB，顺序 Sub → Air；无音频时全为 [`BAND_DB_FLOOR`]）。
    pub bands_tilted_db: Vec<[f32; 8]>,
}

impl BarSeries {
    /// 列长（= 总小节数）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.bar.len()
    }

    /// 是否无小节。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bar.is_empty()
    }
}

/// 整曲总览分析结果（设计稿 §8.2 的顶层形状）。
#[derive(Debug, Clone, PartialEq)]
pub struct SongOverview {
    /// Schema 版本（恒为 [`SCHEMA_VERSION`]）。
    pub schema_version: &'static str,
    /// 歌曲元数据。
    pub song_meta: SongMeta,
    /// 频段定义（Hz，恒为 [`BAND_DEFINITIONS_HZ`]，Sub → Air）。
    pub band_definitions_hz: [[f32; 2]; 8],
    /// 方差自适应后的实际权重 `[响度, 密度, 音区, 和声, 配器]`（Phase 2，
    /// 设计稿 §3.6）：无死特征时等于所选档的权重；有死特征时按
    /// [`adaptive_weights`] 衰减并重分配；和恒为 1.0。
    pub effective_weights: [f32; 5],
    /// 结构摘要（含高潮小节、动态范围与异常列表）。
    pub structural_summary: StructuralSummary,
    /// 段内自相似度摘要（schema 1.1：最相似小节对、机械指数与成段重复）。
    pub repetition: RepetitionSummary,
    /// 分轨频率打架摘要（schema 1.2：逐拍逐频段最响两轨的频谱重叠度，按小节聚合；
    /// 重叠度为主，`gap_db` 只作参考）。
    pub frequency_clash: FrequencyClashSummary,
    /// 小节级列式序列。
    pub bar_series: BarSeries,
}

impl SongOverview {
    /// 诊断异常项（与 `structural_summary.detected_anomalies` 同一份列表）。
    #[must_use]
    pub fn diagnostics(&self) -> &[Anomaly] {
        &self.structural_summary.detected_anomalies
    }
}

// ---------------------------------------------------------------------------
// 公开算法原语（可独立复用与测试）
// ---------------------------------------------------------------------------

/// MIDI 音高转频率（Hz，A4 = 440 Hz）。
#[must_use]
pub fn midi_freq(midi: u8) -> f64 {
    440.0 * libm::pow(2.0, (f64::from(midi) - 69.0) / 12.0)
}

/// 感官粗糙度（Vassilakis / Plomp-Levelt 核，设计稿 §3.3）。
///
/// 输入为谐波分量 `(频率 Hz, 振幅)` 的列表：调用方把每个音符展开为前 6 个
/// 谐波（振幅按 `1/k` 衰减）。对所有谐波对求和后除以固定锚点
/// [`ROUGHNESS_REFERENCE`] 并夹断到 [0, 1]。单音（只有自身谐波对）接近 0；
/// 小二度、三全音与密集低音簇显著升高。纯函数、确定性。
#[must_use]
pub fn sensory_roughness(partials: &[(f64, f64)]) -> f32 {
    let mut sum = 0.0f64;
    for i in 0..partials.len() {
        for j in (i + 1)..partials.len() {
            let (f1, a1) = partials[i];
            let (f2, a2) = partials[j];
            let (f_lo, f_hi) = if f1 < f2 { (f1, f2) } else { (f2, f1) };
            let delta = f_hi - f_lo;
            if delta <= 0.0 {
                continue;
            }
            // Plomp-Levelt 拍频核：s 把频率差换算到临界带宽单位。
            let s = 0.24 / (0.0207 * f_lo + 18.96);
            let x = s * delta;
            let g = libm::exp(-3.5 * x) - libm::exp(-5.75 * x);
            sum += a1 * a2 * g;
        }
    }
    (sum / ROUGHNESS_REFERENCE).clamp(0.0, 1.0) as f32
}

/// 一组 MIDI 音高的感官粗糙度（单位力度：每个音符展开为前 6 个谐波，
///
/// 振幅 `1/k`）。与 [`sensory_roughness`] 的区别只是调用方不用自己展开谐波。
#[must_use]
pub fn roughness_of_midis(midis: &[u8]) -> f32 {
    let mut partials = Vec::with_capacity(midis.len() * 6);
    for &midi in midis {
        let f0 = midi_freq(midi);
        for k in 1..=6 {
            partials.push((f0 * f64::from(k), 1.0 / f64::from(k)));
        }
    }
    sensory_roughness(&partials)
}

/// 五度圈质心偏移度（设计稿 §3.3）：12 维音高类别轮廓（按时值 × 力度加权）
///
/// 投影到五度圈单位圆求几何质心，算该质心与工程调号主音的欧氏距离，再除以 2
/// 归一化到 [0, 1]。离调、借用和弦、副属和弦表现为距离跃升。轮廓全零时返回 0。
#[must_use]
pub fn circle_of_fifths_distance(profile: &[f32; 12], tonic_pc: u8) -> f32 {
    let mut weight_sum = 0.0f32;
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    for (pc, &weight) in profile.iter().enumerate() {
        if weight <= 0.0 {
            continue;
        }
        // 五度圈上的位置：C=0, G=1, D=2, …，即音高类别 × 7 mod 12。
        let fifths = (pc * 7) % 12;
        let angle = std::f32::consts::TAU * fifths as f32 / 12.0;
        x += weight * libm::cosf(angle);
        y += weight * libm::sinf(angle);
        weight_sum += weight;
    }
    if weight_sum <= 0.0 {
        return 0.0;
    }
    x /= weight_sum;
    y /= weight_sum;
    let tonic_fifths = (usize::from(tonic_pc % 12) * 7) % 12;
    let tonic_angle = std::f32::consts::TAU * tonic_fifths as f32 / 12.0;
    let (tx, ty) = (libm::cosf(tonic_angle), libm::sinf(tonic_angle));
    let dx = x - tx;
    let dy = y - ty;
    (libm::sqrtf(dx * dx + dy * dy) / 2.0).clamp(0.0, 1.0)
}

/// 一维双边滤波（设计稿 §3.4）：时间核半径 [`BILATERAL_RADIUS_BEATS`] 拍，
///
/// 值域高斯核 `exp(−ΔT² / 2σ_v²)`，σ_v = [`BILATERAL_SIGMA_V`]。
/// 相邻拍张力差小于 0.18 时强力平滑微抖动；跳变大于 0.25 时值域权重趋近于 0，
/// 跳变边缘被锐利保留。`boundaries` 为段落起始下标（须含 0），窗口严禁跨段
/// （Marker 处硬切断）；下标越界或未排序时先做清洗（排序、去重、夹断）。
#[must_use]
pub fn bilateral_smooth(values: &[f32], boundaries: &[usize]) -> Vec<f32> {
    let mut out = vec![0.0f32; values.len()];
    if values.is_empty() {
        return out;
    }
    // 清洗边界：排序、去重、夹断到 [0, len]，并保证 0 在场。
    let mut points: Vec<usize> = boundaries
        .iter()
        .copied()
        .filter(|&b| b <= values.len())
        .collect();
    points.sort_unstable();
    points.dedup();
    if points.first() != Some(&0) {
        points.insert(0, 0);
    }
    points.push(values.len());
    let sigma_sq_2 = 2.0 * f64::from(BILATERAL_SIGMA_V) * f64::from(BILATERAL_SIGMA_V);
    for segment in points.windows(2) {
        let (start, end) = (segment[0], segment[1]);
        for i in start..end {
            let lo = i.saturating_sub(BILATERAL_RADIUS_BEATS).max(start);
            let hi = (i + BILATERAL_RADIUS_BEATS + 1).min(end);
            let mut weight_sum = 0.0f64;
            let mut acc = 0.0f64;
            for j in lo..hi {
                let delta = f64::from(values[i] - values[j]);
                let weight = libm::exp(-delta * delta / sigma_sq_2);
                weight_sum += weight;
                acc += weight * f64::from(values[j]);
            }
            out[i] = (acc / weight_sum) as f32;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 内部：速度图与时间换算
// ---------------------------------------------------------------------------

/// 音符结束拍（时值 ≤ 0 时等于起始拍）。
fn note_end(note: &NoteEvent) -> f32 {
    (note.start_beat + note.duration_beats).max(note.start_beat)
}

/// 速度图：拍 ↔ 秒的双向换算（分段线性，段内速度恒定）。
struct TempoMap {
    /// (起始拍, 每拍秒数)，按拍升序，第一项恒为 0 拍。
    segments: Vec<(f32, f32)>,
}

impl TempoMap {
    /// 由谱面速度图构造：非法条目（BPM ≤ 0、非有限、拍 < 0）丢弃；
    /// 空图或首条不在 0 拍时，0 拍之前按 [`DEFAULT_BPM`] 计。
    fn from_score(score: &ScoreInput) -> Self {
        let mut entries: Vec<(f32, f32)> = score
            .tempo_map
            .iter()
            .filter(|t| t.bpm.is_finite() && t.bpm > 0.0 && t.beat.is_finite() && t.beat >= 0.0)
            .map(|t| (t.beat, 60.0 / t.bpm))
            .collect();
        entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
        entries.dedup_by(|b, a| b.0 == a.0);
        if entries.is_empty() || entries[0].0 > 0.0 {
            entries.insert(0, (0.0, 60.0 / DEFAULT_BPM));
        }
        Self { segments: entries }
    }

    /// 基准速度（BPM）。
    fn base_bpm(&self) -> f32 {
        60.0 / self.segments[0].1
    }

    /// 拍 → 秒。
    fn beat_to_sec(&self, beat: f32) -> f32 {
        let beat = beat.max(0.0);
        let mut sec = 0.0f32;
        for pair in self.segments.windows(2) {
            let (b0, spb) = pair[0];
            let (b1, _) = pair[1];
            if beat <= b1 {
                return sec + (beat - b0) * spb;
            }
            sec += (b1 - b0) * spb;
        }
        let (b0, spb) = *self.segments.last().expect("速度图恒含 0 拍段");
        sec + (beat - b0) * spb
    }

    /// 秒 → 拍。
    fn sec_to_beat(&self, sec: f32) -> f32 {
        let sec = sec.max(0.0);
        let mut acc = 0.0f32;
        for pair in self.segments.windows(2) {
            let (b0, spb) = pair[0];
            let (b1, _) = pair[1];
            let span = (b1 - b0) * spb;
            if sec <= acc + span {
                return b0 + (sec - acc) / spb;
            }
            acc += span;
        }
        let (b0, spb) = *self.segments.last().expect("速度图恒含 0 拍段");
        b0 + (sec - acc) / spb
    }
}

// ---------------------------------------------------------------------------
// 内部：响度轨（复用 GatedLoudness）
// ---------------------------------------------------------------------------

/// 沿音频每 100 ms 跳结算一次的 M/S 读数序列。
struct LoudnessTrack {
    /// 每次结算后的时刻（秒）。
    times: Vec<f32>,
    /// 瞬时响度（400 ms 窗；窗口未满时为负无穷）。
    momentary: Vec<f32>,
    /// 短时响度（3 s 窗；窗口未满时为负无穷）。
    short_term: Vec<f32>,
}

impl LoudnessTrack {
    /// 取 `time_sec` 时刻之前最近一次结算的读数（无结算时返回哨兵值）。
    fn at(&self, time_sec: f32) -> (f32, f32) {
        let n = self.times.partition_point(|&t| t <= time_sec);
        if n == 0 {
            return (NO_LOUDNESS_READING_LUFS, NO_LOUDNESS_READING_LUFS);
        }
        let i = n - 1;
        (
            sanitize_lufs(self.momentary[i]),
            sanitize_lufs(self.short_term[i]),
        )
    }
}

/// 非有限读数（窗口未满的负无穷）→ 哨兵值；有限读数夹到 [−70, 0]。
///
/// 近静音小节的短时窗能量极小，其对数是有限值但荒谬（演示工程实测
/// −1009.74 LUFS），直通会污染动态范围（实测 996.39 LU）与压缩挤压诊断。
/// BS.1770 绝对门限为 −70 LUFS，低于门限即记哨兵；高于 0 LUFS 不可能
/// 出现，夹到 0 防呆。
fn sanitize_lufs(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(NO_LOUDNESS_READING_LUFS, 0.0)
    } else {
        NO_LOUDNESS_READING_LUFS
    }
}

/// 全曲喂一遍 [`GatedLoudness`]，按跳记录 M/S 读数。
/// 采样率不在 K 加权支持档位时返回空轨（调用方回退到谱面力度代理响度）。
fn build_loudness_track(audio: Option<&AudioInput<'_>>) -> LoudnessTrack {
    let mut track = LoudnessTrack {
        times: Vec::new(),
        momentary: Vec::new(),
        short_term: Vec::new(),
    };
    let Some(audio) = audio else {
        return track;
    };
    let Some(mut meter) = GatedLoudness::for_sample_rate(audio.sample_rate) else {
        return track;
    };
    let hop = meter.hop_samples();
    if hop == 0 {
        return track;
    }
    let frames_total = audio.frames();
    let mut fed = 0usize;
    let mut left: Vec<f32> = Vec::new();
    let mut right: Vec<f32> = Vec::new();
    while fed < frames_total {
        let take = (frames_total - fed).min(hop);
        left.clear();
        right.clear();
        left.reserve(take);
        right.reserve(take);
        for i in 0..take {
            left.push(audio.samples[(fed + i) * 2]);
            right.push(audio.samples[(fed + i) * 2 + 1]);
        }
        meter.add_stereo(&left, &right);
        fed += take;
        track.times.push(fed as f32 / audio.sample_rate);
        track.momentary.push(meter.momentary_lufs());
        track.short_term.push(meter.short_term_lufs());
    }
    track
}

// ---------------------------------------------------------------------------
// 内部：STFT 频段能量（realfft 实数 FFT）
// ---------------------------------------------------------------------------

/// 频率（Hz）所属的频段下标；低于 20 Hz 或 ≥ 20 kHz 时无频段。
fn band_index_for_freq(freq_hz: f64) -> Option<usize> {
    BAND_DEFINITIONS_HZ
        .iter()
        .position(|&[lo, hi]| f64::from(lo) <= freq_hz && freq_hz < f64::from(hi))
}

/// 每拍的 8 频段倾斜补偿能量（dB）。
///
/// 4096 点 Hann 窗、50% 重叠；每帧功率按帧中心时刻归入拍；拍内做均方池化
/// （线性功率先平均再转 dB）。dB 标尺经 Parseval 归一化到时域均方：
/// `mean_square = 2·Σ|X_k|² / (N·Σw²)`，使 0.1 幅正弦的频段读数约为
/// −23 dBFS 再加倾斜补偿（见集成测试的已知答案断言）。
/// 无音频、音频短于一窗或某拍无帧时，该拍记全频段 [`BAND_DB_FLOOR`]。
fn band_energies_per_beat(
    audio: Option<&AudioInput<'_>>,
    tempo: &TempoMap,
    total_beats: usize,
) -> Vec<[f32; 8]> {
    let mut per_beat = vec![[BAND_DB_FLOOR; 8]; total_beats];
    let Some(audio) = audio else {
        return per_beat;
    };
    if total_beats == 0 || audio.frames() < STFT_WINDOW {
        return per_beat;
    }
    let sample_rate = f64::from(audio.sample_rate);
    // Hann 窗与其能量和（Parseval 归一化用）。
    let mut window = vec![0.0f64; STFT_WINDOW];
    for (n, slot) in window.iter_mut().enumerate() {
        *slot = 0.5
            * (1.0 - libm::cos(2.0 * std::f64::consts::PI * n as f64 / (STFT_WINDOW as f64 - 1.0)));
    }
    let window_power: f64 = window.iter().map(|w| w * w).sum();
    // realfft 实数 FFT：plan 一次复用；输出为 N/2+1 个复数 bin（0..=N/2），
    // 与旧 math::fft 的正频半区逐位一致（未归一化正变换，无 1/N 缩放）。
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(STFT_WINDOW);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    // bin→频段表：每帧 2047 个 bin 各做一次 8 段线性查表太贵（约 1150 万次），
    // 改为每采样率只算一次（2048 项），帧循环里只做数组下标。表内容与
    // band_index_for_freq 逐位一致，数值不受影响。
    let mut bin_band = [None; STFT_WINDOW / 2];
    for (k, slot) in bin_band.iter_mut().enumerate().skip(1) {
        *slot = band_index_for_freq(k as f64 * sample_rate / STFT_WINDOW as f64);
    }
    // 每拍累积线性功率与帧数（均方池化）。
    let mut energy = vec![[0.0f64; 8]; total_beats];
    let mut counts = vec![0u32; total_beats];
    let frames_total = audio.frames();
    let mut start = 0usize;
    while start + STFT_WINDOW <= frames_total {
        for n in 0..STFT_WINDOW {
            let l = f64::from(audio.samples[(start + n) * 2]);
            let r = f64::from(audio.samples[(start + n) * 2 + 1]);
            input[n] = (l + r) * 0.5 * window[n];
        }
        // 输入/输出向量由 plan 给出，长度恒合规；失败只可能来自长度错配。
        fft.process(&mut input, &mut spectrum)
            .expect("realfft 输入/输出长度由 plan 给出，process 不应失败");
        let center_sec = (start + STFT_WINDOW / 2) as f32 / audio.sample_rate;
        let beat = tempo.sec_to_beat(center_sec) as usize;
        if beat < total_beats {
            for (k, bin) in spectrum
                .iter()
                .enumerate()
                .skip(1)
                .take(STFT_WINDOW / 2 - 1)
            {
                let Some(band) = bin_band[k] else {
                    continue;
                };
                energy[beat][band] += bin.norm_sqr();
            }
            counts[beat] += 1;
        }
        start += STFT_HOP;
    }
    for beat in 0..total_beats {
        if counts[beat] == 0 {
            continue;
        }
        for band in 0..8 {
            let mean_square = 2.0 * energy[beat][band]
                / (STFT_WINDOW as f64 * window_power * f64::from(counts[beat]));
            let [lo, hi] = BAND_DEFINITIONS_HZ[band];
            let fc = libm::sqrt(f64::from(lo) * f64::from(hi));
            let tilt = f64::from(TILT_DB_PER_OCTAVE) * libm::log2(fc / 1000.0);
            let db = if mean_square > 0.0 {
                10.0 * libm::log10(mean_square) + tilt
            } else {
                f64::from(BAND_DB_FLOOR)
            };
            per_beat[beat][band] = (db as f32).max(BAND_DB_FLOOR);
        }
    }
    per_beat
}

// ---------------------------------------------------------------------------
// 内部：谱面特征小工具
// ---------------------------------------------------------------------------

/// 夹断到 [0, 1]。
fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// 固定锚点夹断归一化（设计稿 §3.2）：`clamp((x − min) / (max − min), 0, 1)`。
fn anchored(value: f32, min: f32, max: f32) -> f32 {
    clamp01((value - min) / (max - min))
}

/// Onset 时刻（秒）聚类计数：以簇首为锚，30 ms 内跟随的 Onset 记为同一次触发。
fn clustered_onset_count(onset_seconds: &[f32]) -> usize {
    let mut sorted = onset_seconds.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mut count = 0usize;
    let mut anchor: Option<f32> = None;
    for &t in &sorted {
        match anchor {
            Some(start) if t - start <= CLUSTER_WINDOW_SECONDS => {}
            _ => {
                count += 1;
                anchor = Some(t);
            }
        }
    }
    count
}

/// 发声占空比：给定拍区间内至少一个音符发声的时间占比（0–1）。
fn duty_cycle(intervals: &[(f32, f32)], win_start: f32, win_end: f32) -> f32 {
    let mut clipped: Vec<(f32, f32)> = intervals
        .iter()
        .filter_map(|&(s, e)| {
            let cs = s.max(win_start);
            let ce = e.min(win_end);
            if ce > cs { Some((cs, ce)) } else { None }
        })
        .collect();
    clipped.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    let mut covered = 0.0f32;
    let mut current: Option<(f32, f32)> = None;
    for (s, e) in clipped {
        match current {
            Some((cs, ce)) if s <= ce => current = Some((cs, ce.max(e))),
            Some((cs, ce)) => {
                covered += ce - cs;
                current = Some((s, e));
            }
            None => current = Some((s, e)),
        }
    }
    if let Some((cs, ce)) = current {
        covered += ce - cs;
    }
    let span = win_end - win_start;
    if span > 0.0 {
        (covered / span).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// 无音频时的响度代理（设计稿 §3.2 回退规则）：活跃音符力度平方和的对数映射。
///
/// `rms = sqrt(Σ(v/127)²)` 夹断到 [0, 1]，`LUFS = −8 + 20·log10(rms)`；
/// 无活跃音符时取锚点下限 −36 LUFS（即 L̂ = 0）。
fn fallback_lufs(active: &[&NoteEvent]) -> f32 {
    if active.is_empty() {
        return LOUDNESS_ANCHOR_MIN_LUFS;
    }
    let sum_sq: f32 = active
        .iter()
        .map(|n| {
            let v = f32::from(n.velocity) / 127.0;
            v * v
        })
        .sum();
    let rms = libm::sqrtf(sum_sq).min(1.0);
    if rms <= 0.0 {
        return LOUDNESS_ANCHOR_MIN_LUFS;
    }
    LOUDNESS_ANCHOR_MAX_LUFS + 20.0 * libm::log10f(rms)
}

/// 每拍特征（滤波前）。
struct BeatFeatures {
    /// 综合情绪张力（未平滑；方差自适应权重算出后填，见主入口第二遍）。
    tension: f32,
    /// 五维归一化分量 `[响度, 密度, 音区, 和声, 配器]`（各 0–1；方差自适应
    /// 的 σ 就按这五列逐拍算）。
    components: [f32; 5],
    /// 旋律组去重后 Onset 率（次/拍）。
    density_pitched: f32,
    /// 旋律组发声占空比（0–1）。
    duty_pitched: f32,
    /// 打击组 Onset 率（次/拍）。
    density_perc: f32,
}

/// 一小节的 14 维自相似度特征（固定锚点归一化到 0–1，不做全曲 Min-Max 拉伸）：
/// 张力 1、短时 LUFS 1（锚点 −70–0 LUFS）、谱面力度 1、疏密 3
/// （旋律 Onset 率、旋律占空比、打击 Onset 率）、8 频段能量 8
/// （锚点 −120–0 dB）。调用方保证各列等长（见 [`BarSeries`]）。
fn repetition_feature_row(series: &BarSeries, index: usize) -> [f32; 14] {
    let mut row = [0.0f32; 14];
    row[0] = series.tension[index];
    row[1] = anchored(series.lufs_short[index], NO_LOUDNESS_READING_LUFS, 0.0);
    row[2] = series.score_dyn[index];
    row[3] = anchored(
        series.density_pitched_onsets_per_beat[index],
        0.0,
        DENSITY_ONSETS_PER_BEAT_ANCHOR,
    );
    row[4] = series.density_pitched_duty[index];
    row[5] = anchored(
        series.density_perc_onsets_per_beat[index],
        0.0,
        DENSITY_ONSETS_PER_BEAT_ANCHOR,
    );
    for band in 0..8 {
        row[6 + band] = anchored(series.bands_tilted_db[index][band], BAND_DB_FLOOR, 0.0);
    }
    row
}

/// 余弦相似度（0–1）：零向量小节（全列为 0，如静默小节）与任何小节的
/// 相似度记 0（余弦未定义；静默不算重复）。
fn cosine_similarity(left: &[f32; 14], right: &[f32; 14]) -> f32 {
    let mut dot = 0.0f32;
    let mut norm_left = 0.0f32;
    let mut norm_right = 0.0f32;
    for index in 0..14 {
        dot += left[index] * right[index];
        norm_left += left[index] * left[index];
        norm_right += right[index] * right[index];
    }
    if norm_left <= 0.0 || norm_right <= 0.0 {
        return 0.0;
    }
    (dot / libm::sqrtf(norm_left * norm_right)).clamp(0.0, 1.0)
}

/// 打架聚合组的值：（最大重叠度，重叠度之和，拍数，最大重叠那拍的差距，
/// 最大重叠那拍的最响轨下标）。
type ClashGroup = (f32, f32, u32, f32, usize);

/// 频谱重叠度换算：两轨频段能量差距 `gap_db`（dB）→ 线性域能量比
/// `E2 / E1 = 10^(-gap_db / 10)`（等能为 1，3 dB 差为 0.5，6 dB 差为 0.25）。
fn overlap_from_gap_db(gap_db: f32) -> f32 {
    10f32.powf(-gap_db / 10.0)
}

/// 重叠度 → 严重度分档（Wang 2026-11-11 13:17 定）：> 0.5 重（差距 < 3 dB，
/// 第二强轨能量超最强轨一半）；0.25–0.5 中（差距 3–6 dB）；< 0.25 轻
/// （差距 > 6 dB）。3 dB 与 6 dB 是能量比的自然锚点（一半、四分之一），
/// 不是另设的硬门限——低于 0.25 的重叠仍计入输出，只定为轻档。
#[must_use]
pub fn severity_band(overlap: f32) -> ClashSeverity {
    if overlap > CLASH_SEVERE_OVERLAP {
        ClashSeverity::Severe
    } else if overlap >= CLASH_MODERATE_OVERLAP {
        ClashSeverity::Moderate
    } else {
        ClashSeverity::Mild
    }
}

/// 分轨频率打架检测（设计稿 §8.2 追加，schema 1.2；Wang 2026-11-11 13:17 定：
/// 输出以频谱重叠度为主，不再二值定性）：逐拍逐频段比各轨能量。
/// 速度图取自谱面（[`TempoMap::from_score`]，与混音分析同一份）；
/// `TempoMap` 是本模块私有类型，故本函数公开面只收 [`ScoreInput`]。
///
/// 每条分轨先复用 [`band_energies_per_beat`]（与混音频段同一口径：倾斜补偿后
/// dB）。每拍每频段：取能量最高的两个参评轨，记一次重叠度
/// `overlap = E2 / E1 = 10^(-gap_db / 10)`（等能为 1，3 dB 差为 0.5，
/// 6 dB 差为 0.25）。参评前提有二：绝对门限（两轨都高于
/// [`CLASH_AUDIBILITY_FLOOR_DB`]，−60 dBFS）与相对门限（各轨该频段能量距
/// 该轨当拍最响频段 ≤ [`CLASH_TRACK_BAND_FLOOR_DB`]，拦 Hann 窗旁瓣泄漏）。
/// 6 dB 不再是定性硬门限：低于 0.25 的重叠也计入，只定为轻档。候选按
/// （小节，频段，轨对）聚合：每组报最大重叠度与均值、最大重叠那拍的差距
/// 与拍数；组按重叠度降序、小节升序、频段升序、轨 id 升序排，取前
/// [`CLASH_MAX_ENTRIES`] 条；`total_groups` 为截断前的组数。另报全曲总体
/// 重叠度（全部参评重叠度的均值）与每小节总体重叠度（均值与最大值）。
///
/// 空分轨、单轨、全静音时返回空摘要（`total_groups = 0`，总体重叠度 0.0）。
/// 纯函数、确定性：聚合用 `BTreeMap`，不依赖哈希迭代顺序。
#[must_use]
pub fn detect_frequency_clash(
    stems: &[StemInput<'_>],
    score: &ScoreInput,
    total_beats: usize,
    beats_per_bar: u32,
) -> FrequencyClashSummary {
    if stems.len() < 2 || total_beats == 0 {
        return FrequencyClashSummary::default();
    }
    let tempo = TempoMap::from_score(score);
    let bpb = beats_per_bar.max(1) as usize;
    // 每条分轨的逐拍频段能量（与混音同一口径），及每轨每拍的最响频段
    // （相对门限的基准）。
    let per_stem: Vec<Vec<[f32; 8]>> = stems
        .iter()
        .map(|stem| band_energies_per_beat(Some(&stem.audio), &tempo, total_beats))
        .collect();
    let peaks: Vec<Vec<f32>> = per_stem
        .iter()
        .map(|bands| {
            bands
                .iter()
                .map(|row| row.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b)))
                .collect()
        })
        .collect();
    // 聚合：(小节 0-based, 频段, 轨下标 a, 轨下标 b) →
    // （最大重叠度，重叠度之和，拍数，最大重叠那拍的差距，最响轨下标）。
    let mut groups: BTreeMap<(usize, usize, usize, usize), ClashGroup> = BTreeMap::new();
    // 全曲总体重叠度与每小节总体重叠度的累加器：重叠度之和、参评个数、最大值。
    let mut song_sum = 0.0f32;
    let mut song_count = 0u32;
    let mut bar_totals: BTreeMap<usize, (f32, u32, f32)> = BTreeMap::new();
    for beat in 0..total_beats {
        for band in 0..8 {
            // 找能量最高的两条参评轨。
            let mut top: [(usize, f32); 2] = [(usize::MAX, f32::NEG_INFINITY); 2];
            for (index, bands) in per_stem.iter().enumerate() {
                let energy = bands[beat][band];
                if energy <= CLASH_AUDIBILITY_FLOOR_DB {
                    continue;
                }
                if energy < peaks[index][beat] - CLASH_TRACK_BAND_FLOOR_DB {
                    continue;
                }
                if energy > top[0].1 {
                    top[1] = top[0];
                    top[0] = (index, energy);
                } else if energy > top[1].1 {
                    top[1] = (index, energy);
                }
            }
            if top[1].0 == usize::MAX {
                continue;
            }
            let gap = top[0].1 - top[1].1;
            let overlap = overlap_from_gap_db(gap);
            let bar = beat / bpb;
            let (a, b) = if top[0].0 < top[1].0 {
                (top[0].0, top[1].0)
            } else {
                (top[1].0, top[0].0)
            };
            // 全曲与每小节的总体重叠度：全部参评（拍，频段）的重叠度都计入
            // （不只进榜的组），均值才是全曲/小节的真实重叠水平。
            song_sum += overlap;
            song_count += 1;
            let bar_total = bar_totals.entry(bar).or_insert((0.0, 0, 0.0));
            bar_total.0 += overlap;
            bar_total.1 += 1;
            bar_total.2 = bar_total.2.max(overlap);
            let entry =
                groups
                    .entry((bar, band, a, b))
                    .or_insert((0.0, 0.0, 0, f32::INFINITY, top[0].0));
            entry.2 += 1;
            entry.1 += overlap;
            if overlap > entry.0 {
                entry.0 = overlap;
                entry.3 = gap;
                entry.4 = top[0].0;
            }
        }
    }
    let total_groups = groups.len();
    let overall_overlap = if song_count > 0 {
        song_sum / song_count as f32
    } else {
        0.0
    };
    let bar_overlaps: Vec<BarOverlap> = bar_totals
        .into_iter()
        .map(|(bar, (sum, count, max))| BarOverlap {
            bar: (bar as u32) + 1,
            overlap: sum / count as f32,
            max_overlap: max,
        })
        .collect();
    let mut entries: Vec<FrequencyClashEntry> = groups
        .into_iter()
        .map(
            |((bar, band, a, b), (max_overlap, sum, beats, gap, loudest))| {
                let other = if loudest == a { b } else { a };
                FrequencyClashEntry {
                    bar: (bar as u32) + 1,
                    band: band as u8,
                    track_a: ClashTrackRef {
                        track_id: stems[loudest].track_id.to_string(),
                        track_name: stems[loudest].track_name.to_string(),
                    },
                    track_b: ClashTrackRef {
                        track_id: stems[other].track_id.to_string(),
                        track_name: stems[other].track_name.to_string(),
                    },
                    overlap: max_overlap,
                    mean_overlap: sum / beats as f32,
                    gap_db: gap,
                    severity: severity_band(max_overlap),
                    beats,
                }
            },
        )
        .collect();
    entries.sort_by(|left, right| {
        right
            .overlap
            .partial_cmp(&left.overlap)
            .unwrap_or(Ordering::Equal)
            .then(left.bar.cmp(&right.bar))
            .then(left.band.cmp(&right.band))
            .then(left.track_a.track_id.cmp(&right.track_a.track_id))
            .then(left.track_b.track_id.cmp(&right.track_b.track_id))
    });
    entries.truncate(CLASH_MAX_ENTRIES);
    FrequencyClashSummary {
        total_groups,
        overall_overlap,
        bar_overlaps,
        entries,
    }
}

/// 段内自相似度分析（设计稿 §8.2 追加，schema 1.1）：对 [`BarSeries`] 的
/// 14 维小节特征逐对算余弦相似度，找跨段重复。纯函数，无音频 I/O。
///
/// - `best_matches`：间隔 ≥ [`NEIGHBOR_GAP_BARS`] 的小节对按相似度降序，
///   取前 [`BEST_MATCHES_LIMIT`] 对；并列时小节号小的在前（确定性）。
/// - `mechanical_index`：最佳非邻匹配 ≥ [`REPEAT_SIMILARITY`] 的小节占比。
/// - `repeated_passages`：按滞后（`lag = 后段小节 − 前段小节`）扫对角线：
///   同一滞后上连续 ≥ [`MIN_REPEAT_BARS`] 小节逐小节相似度 ≥ 阈值 ⇒ 一段；
///   重叠的段按（长度降序、滞后升序、起点升序）贪心取，同一小节只进一段。
///   滞后 < [`MIN_REPEAT_BARS`] 的对角线最长不过 `n − lag < MIN_REPEAT_BARS`，
///   天然不成段 —— 只有间隔 ≥ [`NEIGHBOR_GAP_BARS`] 的跨段重复才能成段。
#[must_use]
pub fn analyze_repetition(series: &BarSeries) -> RepetitionSummary {
    let n = series.len();
    if n == 0 {
        return RepetitionSummary::default();
    }
    let features: Vec<[f32; 14]> = (0..n)
        .map(|index| repetition_feature_row(series, index))
        .collect();
    // 对称相似度矩阵（对角线为 0，不用；每对算两次，n ≤ 200 时约 4 万对 × 14 维，
    // 1 ms 级 —— 可读性优先于这点重复计算）。
    let similarity: Vec<Vec<f32>> = features
        .iter()
        .enumerate()
        .map(|(i, left)| {
            features
                .iter()
                .enumerate()
                .map(|(j, right)| {
                    if i == j {
                        0.0
                    } else {
                        cosine_similarity(left, right)
                    }
                })
                .collect()
        })
        .collect();
    let mut pairs: Vec<(usize, usize, f32)> = Vec::new();
    for (i, row) in similarity.iter().enumerate() {
        for (j, &value) in row.iter().enumerate().skip(i + NEIGHBOR_GAP_BARS) {
            pairs.push((i, j, value));
        }
    }
    pairs.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(Ordering::Equal)
            .then(a.0.cmp(&b.0))
            .then(a.1.cmp(&b.1))
    });
    let best_matches: Vec<BestMatch> = pairs
        .iter()
        .take(BEST_MATCHES_LIMIT)
        .map(|&(i, j, value)| BestMatch {
            bar_a: (i as u32) + 1,
            bar_b: (j as u32) + 1,
            similarity: value,
        })
        .collect();
    let mut mechanical = 0usize;
    for (i, row) in similarity.iter().enumerate() {
        let best = row
            .iter()
            .enumerate()
            .filter(|(j, _)| i.abs_diff(*j) >= NEIGHBOR_GAP_BARS)
            .map(|(_, &value)| value)
            .fold(0.0f32, f32::max);
        if best >= REPEAT_SIMILARITY {
            mechanical += 1;
        }
    }
    let mechanical_index = mechanical as f32 / n as f32;
    let mut candidates: Vec<RepeatedPassage> = Vec::new();
    for lag in NEIGHBOR_GAP_BARS..n {
        let mut start = lag;
        while start < n {
            if similarity[start][start - lag] < REPEAT_SIMILARITY {
                start += 1;
                continue;
            }
            let mut end = start;
            while end + 1 < n && similarity[end + 1][end + 1 - lag] >= REPEAT_SIMILARITY {
                end += 1;
            }
            let len = end - start + 1;
            if len >= MIN_REPEAT_BARS {
                let mean = (start..=end)
                    .map(|index| similarity[index][index - lag])
                    .sum::<f32>()
                    / len as f32;
                candidates.push(RepeatedPassage {
                    bars: [(start as u32) + 1, (end as u32) + 1],
                    source_bars: [((start - lag) as u32) + 1, ((end - lag) as u32) + 1],
                    mean_similarity: mean,
                });
            }
            start = end + 1;
        }
    }
    candidates.sort_by(|a, b| {
        let len_a = a.bars[1] - a.bars[0];
        let len_b = b.bars[1] - b.bars[0];
        let lag_a = a.bars[0] - a.source_bars[0];
        let lag_b = b.bars[0] - b.source_bars[0];
        len_b
            .cmp(&len_a)
            .then(lag_a.cmp(&lag_b))
            .then(a.bars[0].cmp(&b.bars[0]))
    });
    let mut claimed = vec![false; n];
    let mut repeated_passages: Vec<RepeatedPassage> = Vec::new();
    for passage in candidates {
        let start = (passage.bars[0] - 1) as usize;
        let end = (passage.bars[1] - 1) as usize;
        if claimed[start..=end].iter().any(|&taken| taken) {
            continue;
        }
        for taken in &mut claimed[start..=end] {
            *taken = true;
        }
        repeated_passages.push(passage);
    }
    repeated_passages.sort_by_key(|passage| passage.bars[0]);
    RepetitionSummary {
        best_matches,
        mechanical_index,
        repeated_passages,
    }
}

// ---------------------------------------------------------------------------
// 主入口
// ---------------------------------------------------------------------------

/// 整曲四层离线分析（设计稿 Phase 1）：谱面 + 可选渲染音频 → 小节级总览。
///
/// 音频缺失时：能量层与频率层记哨兵值（LUFS −70、频段 −120 dB），情绪层的
/// 响度分量回退到谱面力度代理；疏密层与和声层只用谱面，照常计算。
/// 空谱面且无音频时返回空 `bar_series`（`total_bars = 0`）。
/// 默认选项（`Default` 档、无期望等级表）；要指定曲风档或期望等级表时用
/// [`analyze_song_overview_with_options`]。
#[must_use]
pub fn analyze_song_overview(score: &ScoreInput, audio: Option<&AudioInput<'_>>) -> SongOverview {
    analyze_song_overview_with_options(score, audio, &AnalysisOptions::default())
}

/// 整曲四层离线分析（设计稿 Phase 2 选项版）：曲风预设档（§3.6）与期望等级
/// 表（§3.5）经 [`AnalysisOptions`] 传入。
///
/// 两遍结构：第一遍逐拍算五维归一化分量；第二遍按全曲逐维标准差算方差自适应
/// 权重（死特征衰减 50% 并重分配），再点乘得张力、双边滤波、小节聚合。
/// 期望等级表只改本次分析用的 Marker 副本（调用方的 `ScoreInput` 不动）。
#[must_use]
pub fn analyze_song_overview_with_options(
    score: &ScoreInput,
    audio: Option<&AudioInput<'_>>,
    options: &AnalysisOptions,
) -> SongOverview {
    analyze_song_overview_with_stems(score, audio, &[], options)
}

/// 整曲四层离线分析（分轨版，设计稿 §8.2 追加，schema 1.2）：`stems` 是渲染侧
/// 每条源轨 solo 渲染的音频；空分轨（`&[]`）时 `frequency_clash` 为空摘要，
/// 其余四层与 [`analyze_song_overview_with_options`] 逐位一致。
#[must_use]
pub fn analyze_song_overview_with_stems(
    score: &ScoreInput,
    audio: Option<&AudioInput<'_>>,
    stems: &[StemInput<'_>],
    options: &AnalysisOptions,
) -> SongOverview {
    let mut score = score.clone();
    apply_expected_levels(&mut score, &options.expected_levels);
    let score = &score;
    let audio = audio.filter(|a| a.sample_rate.is_finite() && a.sample_rate > 0.0 && !a.is_empty());
    let bpb = score.beats_per_bar();
    let bpb_f = bpb as f32;
    let tempo = TempoMap::from_score(score);
    let score_end_beat = score
        .notes
        .iter()
        .fold(0.0f32, |max, n| max.max(note_end(n)));
    let audio_beats = audio.map_or(0.0, |a| tempo.sec_to_beat(a.duration_sec()));
    let total_beats = score_end_beat.max(audio_beats).ceil().max(0.0) as usize;
    let total_bars = total_beats.div_ceil(bpb as usize);

    let duration_sec = audio.map_or_else(
        || tempo.beat_to_sec(score_end_beat),
        AudioInput::duration_sec,
    );
    let tonic = score.key_tonic_pc % 12;
    let key_signature = format!(
        "{} {}",
        NOTE_NAMES[usize::from(tonic)],
        if score.is_minor { "minor" } else { "major" }
    );

    let loudness = build_loudness_track(audio);
    let bands_per_beat = band_energies_per_beat(audio, &tempo, total_beats);

    // -- 每拍特征（1 小节滑窗、1 拍步进；窗以该拍为中心）--
    let mut beat_features = Vec::with_capacity(total_beats);
    for beat in 0..total_beats {
        let center = beat as f32 + 0.5;
        let win_start = (beat as f32 - bpb_f / 2.0).max(0.0);
        let win_end = (beat as f32 + bpb_f / 2.0).min(total_beats as f32);
        let win_beats = (win_end - win_start).max(0.0);
        let win_seconds = (tempo.beat_to_sec(win_end) - tempo.beat_to_sec(win_start)).max(0.0);

        let active: Vec<&NoteEvent> = score
            .notes
            .iter()
            .filter(|n| n.start_beat < win_end && note_end(n) > win_start)
            .collect();
        let pitched: Vec<&NoteEvent> = active
            .iter()
            .copied()
            .filter(|n| !n.is_percussion)
            .collect();
        let onsets: Vec<&NoteEvent> = score
            .notes
            .iter()
            .filter(|n| n.start_beat >= win_start && n.start_beat < win_end)
            .collect();

        // 响度分量：有音频时取短时 LUFS（未满 3 s 时取瞬时）；无音频时回退到谱面代理。
        let lufs = if audio.is_some() {
            let (momentary, short_term) = loudness.at(tempo.beat_to_sec(center));
            if short_term != NO_LOUDNESS_READING_LUFS {
                short_term
            } else {
                momentary
            }
        } else {
            fallback_lufs(&active)
        };
        let loudness_norm = anchored(lufs, LOUDNESS_ANCHOR_MIN_LUFS, LOUDNESS_ANCHOR_MAX_LUFS);

        // 密度分量：聚类后 Onset 数 / 窗秒数，对数压缩。
        let onset_seconds: Vec<f32> = onsets
            .iter()
            .map(|n| tempo.beat_to_sec(n.start_beat))
            .collect();
        let clustered = clustered_onset_count(&onset_seconds);
        let notes_per_sec = if win_seconds > 0.0 {
            clustered as f32 / win_seconds
        } else {
            0.0
        };
        let density_norm = clamp01(
            libm::logf(1.0 + notes_per_sec) / libm::logf(1.0 + DENSITY_ANCHOR_NOTES_PER_SEC),
        );

        // 音区分量：活跃旋律音符的力度加权平均 MIDI 音高。
        let pitch_norm = if pitched.is_empty() {
            0.0
        } else {
            let (weighted_sum, weight_sum) = pitched.iter().fold((0.0f32, 0.0f32), |(s, w), n| {
                let weight = f32::from(n.velocity) / 127.0;
                (s + weight * f32::from(n.midi), w + weight)
            });
            if weight_sum > 0.0 {
                anchored(
                    weighted_sum / weight_sum,
                    PITCH_ANCHOR_MIN_MIDI,
                    PITCH_ANCHOR_MAX_MIDI,
                )
            } else {
                0.0
            }
        };

        // 和声分量：窗口中心时刻的粗糙度 + 全窗音高类别轮廓的五度圈偏移。
        let center_active: Vec<&NoteEvent> = pitched
            .iter()
            .copied()
            .filter(|n| n.start_beat <= center && note_end(n) > center)
            .collect();
        let mut partials = Vec::with_capacity(center_active.len() * 6);
        for n in &center_active {
            let f0 = midi_freq(n.midi);
            let amp = f64::from(n.velocity) / 127.0;
            for k in 1..=6 {
                partials.push((f0 * f64::from(k), amp / f64::from(k)));
            }
        }
        let roughness = sensory_roughness(&partials);
        let mut profile = [0.0f32; 12];
        for n in &pitched {
            let overlap = note_end(n).min(win_end) - n.start_beat.max(win_start);
            if overlap > 0.0 {
                profile[usize::from(n.midi % 12)] += overlap * (f32::from(n.velocity) / 127.0);
            }
        }
        let cof = circle_of_fifths_distance(&profile, tonic);
        let harmony = 0.55 * roughness + 0.45 * cof;

        // 配器分量：活跃轨道数 + 旋律音区跨越的八度带宽。
        let mut tracks: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for n in &active {
            tracks.insert(n.track);
        }
        let pitch_span_oct = if pitched.is_empty() {
            0.0
        } else {
            let min_midi = pitched.iter().map(|n| n.midi).min().unwrap_or(0);
            let max_midi = pitched.iter().map(|n| n.midi).max().unwrap_or(0);
            f32::from(max_midi - min_midi) / 12.0
        };
        let orchestration =
            0.5 * (tracks.len() as f32 / 8.0).min(1.0) + 0.5 * (pitch_span_oct / 5.0).min(1.0);

        let tension = 0.0f32; // 第二遍：方差自适应权重算出后点乘填（见循环后）。
        let components = [
            loudness_norm,
            density_norm,
            pitch_norm,
            harmony,
            orchestration,
        ];

        // 疏密层：旋律组聚类 Onset 率 + 占空比；打击组只算原始 Onset 率。
        let pitched_onset_seconds: Vec<f32> = onsets
            .iter()
            .filter(|n| !n.is_percussion)
            .map(|n| tempo.beat_to_sec(n.start_beat))
            .collect();
        let pitched_intervals: Vec<(f32, f32)> = pitched
            .iter()
            .map(|n| (n.start_beat, note_end(n)))
            .collect();
        let perc_onsets = onsets.iter().filter(|n| n.is_percussion).count();
        let density_pitched = if win_beats > 0.0 {
            clustered_onset_count(&pitched_onset_seconds) as f32 / win_beats
        } else {
            0.0
        };
        let duty_pitched = duty_cycle(&pitched_intervals, win_start, win_end);
        let density_perc = if win_beats > 0.0 {
            perc_onsets as f32 / win_beats
        } else {
            0.0
        };

        beat_features.push(BeatFeatures {
            tension,
            components,
            density_pitched,
            duty_pitched,
            density_perc,
        });
    }

    // -- 第二遍：方差自适应权重（设计稿 §3.6）--
    // 五维分量逐拍的总体标准差；单拍 / 空曲时 σ 全为 0（全部死特征 ⇒
    // adaptive_weights 重归一回原权重，张力与固定权重逐位一致）。
    let mut std_devs = [0.0f32; 5];
    if !beat_features.is_empty() {
        let n = beat_features.len() as f32;
        for (dim, std_dev) in std_devs.iter_mut().enumerate() {
            let mean = beat_features.iter().map(|f| f.components[dim]).sum::<f32>() / n;
            let variance = beat_features
                .iter()
                .map(|f| {
                    let d = f.components[dim] - mean;
                    d * d
                })
                .sum::<f32>()
                / n;
            *std_dev = libm::sqrtf(variance);
        }
    }
    let effective_weights = adaptive_weights(options.profile.weights(), std_devs);
    for features in &mut beat_features {
        features.tension = features
            .components
            .iter()
            .zip(effective_weights.iter())
            .map(|(c, w)| c * w)
            .sum::<f32>()
            .clamp(0.0, 1.0);
    }

    // -- 双边滤波：段落 Marker 处硬切断 --
    let mut boundaries: Vec<usize> = score
        .markers
        .iter()
        .filter(|m| m.bar >= 1 && usize::try_from(m.bar - 1).is_ok())
        .map(|m| (m.bar - 1) as usize * bpb as usize)
        .filter(|&b| b < total_beats)
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    let raw_tension: Vec<f32> = beat_features.iter().map(|f| f.tension).collect();
    let smooth_tension = bilateral_smooth(&raw_tension, &boundaries);

    // -- 按小节聚合 --
    let mut series = BarSeries {
        bar: Vec::with_capacity(total_bars),
        time_sec: Vec::with_capacity(total_bars),
        section: Vec::with_capacity(total_bars),
        tension: Vec::with_capacity(total_bars),
        valence_hue: Vec::with_capacity(total_bars),
        lufs_short: Vec::with_capacity(total_bars),
        lufs_momentary_max: Vec::with_capacity(total_bars),
        score_dyn: Vec::with_capacity(total_bars),
        envelope_lo: Vec::with_capacity(total_bars),
        envelope_hi: Vec::with_capacity(total_bars),
        target_deviation: Vec::with_capacity(total_bars),
        density_pitched_onsets_per_beat: Vec::with_capacity(total_bars),
        density_pitched_duty: Vec::with_capacity(total_bars),
        density_perc_onsets_per_beat: Vec::with_capacity(total_bars),
        bands_tilted_db: Vec::with_capacity(total_bars),
    };
    let mut sorted_markers: Vec<&SectionMarker> = score.markers.iter().collect();
    sorted_markers.sort_by_key(|a| a.bar);
    for bar in 0..total_bars {
        let bar_start_beat = bar * bpb as usize;
        let bar_end_beat = ((bar + 1) * bpb as usize).min(total_beats);
        let bar_beats = bar_end_beat - bar_start_beat;
        let mean_beat = |pick: &dyn Fn(&BeatFeatures) -> f32| -> f32 {
            if bar_beats == 0 {
                return 0.0;
            }
            let sum: f32 = beat_features[bar_start_beat..bar_end_beat]
                .iter()
                .map(pick)
                .sum();
            sum / bar_beats as f32
        };
        let tension = if bar_beats == 0 {
            0.0
        } else {
            smooth_tension[bar_start_beat..bar_end_beat]
                .iter()
                .sum::<f32>()
                / bar_beats as f32
        };
        let valence_base = if score.is_minor { -0.6 } else { 0.6 };
        let bar_start_sec = tempo.beat_to_sec(bar_start_beat as f32);
        let bar_end_sec = tempo.beat_to_sec(bar_end_beat as f32);
        let bar_center_sec = tempo.beat_to_sec(bar_start_beat as f32 + bpb_f / 2.0);
        let (_, short_term) = loudness.at(bar_center_sec);
        let momentary_max = loudness
            .times
            .iter()
            .zip(loudness.momentary.iter())
            .filter(|(t, _)| **t >= bar_start_sec && **t < bar_end_sec)
            .map(|(_, &m)| sanitize_lufs(m))
            .fold(NO_LOUDNESS_READING_LUFS, f32::max);
        let score_dyn = {
            let velocities: Vec<f32> = score
                .notes
                .iter()
                .filter(|n| {
                    n.start_beat >= bar_start_beat as f32 && n.start_beat < bar_end_beat as f32
                })
                .map(|n| f32::from(n.velocity) / 127.0)
                .collect();
            if velocities.is_empty() {
                0.0
            } else {
                velocities.iter().sum::<f32>() / velocities.len() as f32
            }
        };
        let bands = if bar_beats == 0 {
            [BAND_DB_FLOOR; 8]
        } else {
            let mut acc = [0.0f32; 8];
            for beat_bands in &bands_per_beat[bar_start_beat..bar_end_beat] {
                for band in 0..8 {
                    acc[band] += beat_bands[band];
                }
            }
            let mut out = [0.0f32; 8];
            for band in 0..8 {
                out[band] = acc[band] / bar_beats as f32;
            }
            out
        };
        let current_marker = sorted_markers.iter().rfind(|m| m.bar <= (bar as u32) + 1);
        let section = current_marker.map_or_else(|| "Untitled".to_string(), |m| m.label.clone());
        // 期望带（Phase 2 §3.5）：本小节所属段落的期望等级 → 带；未标注 ⇒ 哨兵。
        // 偏离量按显示的张力（双边滤波后的小节均值）算，与面板画的线同一条。
        let (env_lo, env_hi) = current_marker
            .and_then(|m| m.expected_level)
            .and_then(envelope_band)
            .unwrap_or((NO_ENVELOPE, NO_ENVELOPE));
        let deviation = envelope_deviation(tension, env_lo, env_hi);

        series.bar.push((bar as u32) + 1);
        series.time_sec.push(bar_start_sec);
        series.section.push(section);
        series.tension.push(tension);
        series
            .valence_hue
            .push(valence_base * (1.0 - 0.25 * tension));
        series.lufs_short.push(short_term);
        series.lufs_momentary_max.push(momentary_max);
        series.score_dyn.push(score_dyn);
        series.envelope_lo.push(env_lo);
        series.envelope_hi.push(env_hi);
        series.target_deviation.push(deviation);
        series
            .density_pitched_onsets_per_beat
            .push(mean_beat(&|f: &BeatFeatures| f.density_pitched));
        series
            .density_pitched_duty
            .push(mean_beat(&|f: &BeatFeatures| f.duty_pitched));
        series
            .density_perc_onsets_per_beat
            .push(mean_beat(&|f: &BeatFeatures| f.density_perc));
        series.bands_tilted_db.push(bands);
    }

    // -- 结构摘要与诊断 --
    let (climax_bar, climax_tension) = series
        .tension
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(Ordering::Equal))
        .map_or((0, 0.0), |(i, &t)| ((i as u32) + 1, t));
    let readings: Vec<f32> = series
        .lufs_short
        .iter()
        .copied()
        .filter(|&v| v != NO_LOUDNESS_READING_LUFS)
        .collect();
    let dynamic_range_lu = if readings.len() >= 2 {
        let max = readings.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let min = readings.iter().copied().fold(f32::INFINITY, f32::min);
        (max - min).max(0.0)
    } else {
        0.0
    };
    let mut anomalies = Vec::new();
    // 期望带偏离（Phase 2 §3.5）：与音频无关，纯谱面标注 + 张力曲线即可判；
    // 连续超标小节合并为一段（单小节超标也报 —— 单小节塌掉就是一次结构信号，
    // 与频段淤积那种需要 4 小节防抖的噪声不同）。
    anomalies.extend(detect_envelope_deviation(
        &series.target_deviation,
        &series.section,
    ));
    if audio.is_some() {
        anomalies.extend(detect_low_mid_buildup(&series.bands_tilted_db));
        anomalies.extend(detect_compression_squash(
            &series.score_dyn,
            &series.lufs_short,
        ));
    }
    // -- 段内自相似度与重复段诊断（schema 1.1）--
    // 成段重复每段一条 `arrangement_repetition` 异常（与音频无关，纯谱面
    // 序列即可判；单小节撞分不成段，故只看 `repeated_passages`）。
    let repetition = analyze_repetition(&series);
    let frequency_clash = detect_frequency_clash(stems, score, total_beats, bpb);
    for passage in &repetition.repeated_passages {
        anomalies.push(Anomaly {
            bars: passage.bars,
            anomaly_type: AnomalyType::ArrangementRepetition,
            description: format!(
                "Bars {}-{} repeat bars {}-{} (mean similarity {:.2})",
                passage.bars[0],
                passage.bars[1],
                passage.source_bars[0],
                passage.source_bars[1],
                passage.mean_similarity
            ),
        });
    }

    SongOverview {
        schema_version: SCHEMA_VERSION,
        song_meta: SongMeta {
            total_bars: total_bars as u32,
            duration_sec,
            base_bpm: tempo.base_bpm(),
            key_signature,
            profile: options.profile.as_str().to_string(),
        },
        band_definitions_hz: BAND_DEFINITIONS_HZ,
        effective_weights,
        structural_summary: StructuralSummary {
            climax_bar,
            climax_tension,
            dynamic_range_lu,
            detected_anomalies: anomalies,
        },
        repetition,
        frequency_clash,
        bar_series: series,
    }
}

// ---------------------------------------------------------------------------
// 内部：诊断
// ---------------------------------------------------------------------------

/// 期望带偏离（设计稿 §3.5）：偏离量超过 [`ENVELOPE_DEVIATION_ALERT`] 的
/// 连续小节合并为一段；描述含该段峰值偏离与段落名。
fn detect_envelope_deviation(deviation: &[f32], sections: &[String]) -> Vec<Anomaly> {
    let mut anomalies = Vec::new();
    let mut i = 0usize;
    while i < deviation.len() {
        if deviation[i] <= ENVELOPE_DEVIATION_ALERT {
            i += 1;
            continue;
        }
        let start = i;
        let mut peak = deviation[i];
        while i < deviation.len() && deviation[i] > ENVELOPE_DEVIATION_ALERT {
            peak = peak.max(deviation[i]);
            i += 1;
        }
        let end = i - 1;
        let section = sections
            .get(start)
            .cloned()
            .unwrap_or_else(|| "Untitled".to_string());
        anomalies.push(Anomaly {
            bars: [(start as u32) + 1, (end as u32) + 1],
            anomaly_type: AnomalyType::EnvelopeDeviation,
            description: format!(
                "Tension deviates from the {section} target envelope by up to {peak:.2} over bars {}-{}",
                start + 1,
                end + 1
            ),
        });
    }
    anomalies
}

/// 低频中段淤积（设计稿 §8.2）：Band 3（250–500 Hz）超过其余 7 频段均值
/// [`LOW_MID_EXCESS_DB`] dB 以上、连续至少 [`DIAGNOSTIC_MIN_BARS`] 小节。
fn detect_low_mid_buildup(bands: &[[f32; 8]]) -> Vec<Anomaly> {
    let excess: Vec<f32> = bands
        .iter()
        .map(|bar| {
            let others: f32 = bar
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != 2)
                .map(|(_, &v)| v)
                .sum::<f32>()
                / 7.0;
            bar[2] - others
        })
        .collect();
    runs_to_anomalies(
        &excess,
        LOW_MID_EXCESS_DB,
        AnomalyType::LowMidBuildup,
        |start, end, peak| {
            format!(
                "Band 3 (250-500Hz) exceeds tilted balance by +{peak:.1} dB over bars {}-{}",
                start + 1,
                end + 1
            )
        },
    )
}

/// 总线压缩压扁（设计稿 §8.2）：连续 [`DIAGNOSTIC_MIN_BARS`] 小节窗口内谱面力度
/// 上升至少 [`SQUASH_SCORE_RISE`]、渲染 LUFS 上升至多 [`SQUASH_LUFS_RISE_MAX`] LU。
/// 窗口两端都须有有效 LUFS 读数；重叠窗口合并为一个异常。
fn detect_compression_squash(score_dyn: &[f32], lufs_short: &[f32]) -> Vec<Anomaly> {
    if score_dyn.len() < DIAGNOSTIC_MIN_BARS {
        return Vec::new();
    }
    let mut flagged = vec![false; score_dyn.len()];
    for start in 0..=(score_dyn.len() - DIAGNOSTIC_MIN_BARS) {
        let end = start + DIAGNOSTIC_MIN_BARS - 1;
        let lufs_start = lufs_short[start];
        let lufs_end = lufs_short[end];
        if lufs_start == NO_LOUDNESS_READING_LUFS || lufs_end == NO_LOUDNESS_READING_LUFS {
            continue;
        }
        let score_rise = score_dyn[end] - score_dyn[start];
        let lufs_rise = lufs_end - lufs_start;
        if score_rise >= SQUASH_SCORE_RISE && lufs_rise <= SQUASH_LUFS_RISE_MAX {
            for flag in flagged.iter_mut().take(end + 1).skip(start) {
                *flag = true;
            }
        }
    }
    // 连续被标旗的小节合并为一段；描述用该段首尾实测值。
    let mut anomalies = Vec::new();
    let mut i = 0usize;
    while i < flagged.len() {
        if !flagged[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < flagged.len() && flagged[i] {
            i += 1;
        }
        let end = i - 1;
        let score_rise = score_dyn[end] - score_dyn[start];
        let lufs_rise = lufs_short[end] - lufs_short[start];
        anomalies.push(Anomaly {
            bars: [(start as u32) + 1, (end as u32) + 1],
            anomaly_type: AnomalyType::CompressionSquash,
            description: format!(
                "Score velocity rises +{:.0}% but rendered LUFS changes only {:+.1} LU over bars {}-{}",
                score_rise * 100.0,
                lufs_rise,
                start + 1,
                end + 1
            ),
        });
    }
    anomalies
}

/// 超标连续段 → 异常：`values` 中连续 ≥ [`DIAGNOSTIC_MIN_BARS`] 项超过 `threshold`
/// 的段各记一条，描述由调用方按（段首下标，段尾下标，段内峰值）生成。
fn runs_to_anomalies(
    values: &[f32],
    threshold: f32,
    anomaly_type: AnomalyType,
    describe: impl Fn(usize, usize, f32) -> String,
) -> Vec<Anomaly> {
    let mut anomalies = Vec::new();
    let mut i = 0usize;
    while i < values.len() {
        if values[i] <= threshold {
            i += 1;
            continue;
        }
        let start = i;
        let mut peak = values[i];
        while i < values.len() && values[i] > threshold {
            peak = peak.max(values[i]);
            i += 1;
        }
        let end = i - 1;
        if end - start + 1 >= DIAGNOSTIC_MIN_BARS {
            anomalies.push(Anomaly {
                bars: [(start as u32) + 1, (end as u32) + 1],
                anomaly_type,
                description: describe(start, end, peak),
            });
        }
    }
    anomalies
}
