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
//!   按拍均方池化。FFT 复用树内已有的 [`crate::math::fft`]，不引新依赖。
//! - 能量层：复用 [`crate::loudness::GatedLoudness`] 的 BS.1770-4 实现
//!   （瞬时 400 ms、短时 3 s），不重写 LUFS、不引 ebur128。
//! - 疏密层：旋律与打击分轨；旋律组用 Clustered Onset 率（同拍 30 ms 内柱式
//!   和弦记 1 次）+ 占空比，打击组只算 Onset 率。1 小节滑窗、1 拍步进。
//!
//! 诊断（[`Anomaly`]，MCP 与 UI 共用）：低频中段淤积（`low_mid_buildup`）与
//! 总线压缩压扁（`compression_squash`），均为纯逻辑阈值判定。
//!
//! 约束：离线分析，不在实时音频回调里调用；确定性——固定锚点、固定阈值，
//! 同一输入跨进程结果一致；不写 `unsafe`；持久化集合只用 `BTreeMap`/`BTreeSet`
//! （本模块实际只用 `Vec` 与定长数组）。

use crate::loudness::GatedLoudness;
use crate::math::fft;
use std::cmp::Ordering;

/// 设计稿 schema 版本（§8.2）。
pub const SCHEMA_VERSION: &str = "1.0";

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionMarker {
    /// 段落起始小节（1-based）。
    pub bar: u32,
    /// 段落名（如 "Intro"、"Chorus"）。
    pub label: String,
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

/// 结构异常的类型（设计稿 §8.2 示例列出的两类；Phase 1 只做这两类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnomalyType {
    /// 低频中段淤积：Band 3（250–500 Hz）持续高于其余频段均值。
    LowMidBuildup,
    /// 总线压缩压扁：谱面力度上升、渲染 LUFS 几乎不动。
    CompressionSquash,
}

impl AnomalyType {
    /// JSON 里的类型名（设计稿 §8.2 的 `type` 字段）。
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LowMidBuildup => "low_mid_buildup",
            Self::CompressionSquash => "compression_squash",
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
    /// 权重预设档：Phase 1 固定默认权重，恒为 `"default"`（§3.6 的 3 档预设是 Phase 2）。
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
    pub score_dyn: Vec<f32>,
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
    /// 结构摘要（含高潮小节、动态范围与异常列表）。
    pub structural_summary: StructuralSummary,
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

/// 非有限读数（窗口未满的负无穷）→ 哨兵值。
fn sanitize_lufs(value: f32) -> f32 {
    if value.is_finite() {
        value
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
// 内部：STFT 频段能量（复用 crate::math::fft）
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
    let mut re = vec![0.0f64; STFT_WINDOW];
    let mut im = vec![0.0f64; STFT_WINDOW];
    // 每拍累积线性功率与帧数（均方池化）。
    let mut energy = vec![[0.0f64; 8]; total_beats];
    let mut counts = vec![0u32; total_beats];
    let frames_total = audio.frames();
    let mut start = 0usize;
    while start + STFT_WINDOW <= frames_total {
        for n in 0..STFT_WINDOW {
            let l = f64::from(audio.samples[(start + n) * 2]);
            let r = f64::from(audio.samples[(start + n) * 2 + 1]);
            re[n] = (l + r) * 0.5 * window[n];
            im[n] = 0.0;
        }
        fft(&mut re, &mut im, false);
        let center_sec = (start + STFT_WINDOW / 2) as f32 / audio.sample_rate;
        let beat = tempo.sec_to_beat(center_sec) as usize;
        if beat < total_beats {
            for k in 1..(STFT_WINDOW / 2) {
                let freq = k as f64 * sample_rate / STFT_WINDOW as f64;
                let Some(band) = band_index_for_freq(freq) else {
                    continue;
                };
                energy[beat][band] += re[k] * re[k] + im[k] * im[k];
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
    /// 综合情绪张力（未平滑）。
    tension: f32,
    /// 旋律组去重后 Onset 率（次/拍）。
    density_pitched: f32,
    /// 旋律组发声占空比（0–1）。
    duty_pitched: f32,
    /// 打击组 Onset 率（次/拍）。
    density_perc: f32,
}

// ---------------------------------------------------------------------------
// 主入口
// ---------------------------------------------------------------------------

/// 整曲四层离线分析（设计稿 Phase 1）：谱面 + 可选渲染音频 → 小节级总览。
///
/// 音频缺失时：能量层与频率层记哨兵值（LUFS −70、频段 −120 dB），情绪层的
/// 响度分量回退到谱面力度代理；疏密层与和声层只用谱面，照常计算。
/// 空谱面且无音频时返回空 `bar_series`（`total_bars = 0`）。
#[must_use]
pub fn analyze_song_overview(score: &ScoreInput, audio: Option<&AudioInput<'_>>) -> SongOverview {
    // 采样率非法或无帧的音频按缺失处理。
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

        let tension = (WEIGHT_LOUDNESS * loudness_norm
            + WEIGHT_DENSITY * density_norm
            + WEIGHT_PITCH * pitch_norm
            + WEIGHT_HARMONY * harmony
            + WEIGHT_ORCHESTRATION * orchestration)
            .clamp(0.0, 1.0);

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
            density_pitched,
            duty_pitched,
            density_perc,
        });
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
        let section = sorted_markers
            .iter()
            .rfind(|m| m.bar <= (bar as u32) + 1)
            .map_or_else(|| "Untitled".to_string(), |m| m.label.clone());

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
    if audio.is_some() {
        anomalies.extend(detect_low_mid_buildup(&series.bands_tilted_db));
        anomalies.extend(detect_compression_squash(
            &series.score_dyn,
            &series.lufs_short,
        ));
    }

    SongOverview {
        schema_version: SCHEMA_VERSION,
        song_meta: SongMeta {
            total_bars: total_bars as u32,
            duration_sec,
            base_bpm: tempo.base_bpm(),
            key_signature,
            profile: "default".to_string(),
        },
        band_definitions_hz: BAND_DEFINITIONS_HZ,
        structural_summary: StructuralSummary {
            climax_bar,
            climax_tension,
            dynamic_range_lu,
            detected_anomalies: anomalies,
        },
        bar_series: series,
    }
}

// ---------------------------------------------------------------------------
// 内部：诊断
// ---------------------------------------------------------------------------

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
