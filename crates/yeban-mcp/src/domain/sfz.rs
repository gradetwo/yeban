//! `yeban_load_sfz` 与 `yeban_render_sfz_wav` 的实现：**SFZ 音色真的出声**。
//!
//! ## 为什么有这一条（缺口的实测证据，不是从词面推的）
//!
//! `AGENTS.md` §6.3 要求先 `grep` 交付形态再下结论。本线开工时实测：
//!
//! ```text
//! grep -rn "sfz" crates/yeban-mcp/src/            ⇒ 只有 render.rs 的 unsupported 键 "sfzSampler"
//! grep -rn "yeban_load_sfz\|yeban_render_sfz_wav" crates/ ⇒ 0 命中
//! ```
//!
//! `yeban_render_master` **能**导 WAV（真走 `yeban-render`，见 `render.rs` 的模块头），
//! 但它的 `unsupported` 里**明写** `sfzSampler`（"本模型版本（`DeviceKind`）里**没有**
//! SFZ 设备变体"）⇒ 工具面**没有**任何通路让 SFZ 音色参与导出。本模块补的就是这一条。
//!
//! ## 数据流（每一段都复用既有设施，⛔ 无旁路）
//!
//! ```text
//! yeban_load_sfz
//!   读文件 ⇒ sha256(bytes)
//!          ⇒ yeban_sfz::IncludeResolver::resolve_text_lossy(根文本, 基准目录)
//!               ├─ 读到 ⇒ parse_sources(片段序列)
//!               └─ 缺失 ⇒ 记一条 include_issue（⛔ 不失败；循环/超深仍是错误）
//!          ⇒ 区域数 + 逐区域摘要 (键/力度范围 + sample 路径)
//!          ⇒ 会话内 registry.insert(id, text + 片段序列 + 摘要 + 路径)
//!               [MODEL-ISO-001 第 2 层]
//!
//! yeban_render_sfz_wav
//!   活跃工程的 MIDI 片段 ⇒ 每个音符 (start_frame, frames, pitch, velocity)
//!                        [与 render.rs 的连击/微时值格点**同一份**公式]
//!          ⇒ 逐音符: 区域选择 Region::matches_key/matches_velocity
//!                    播放速率 Region::playback_rate
//!                    总增益   Region::playback_spec(..).total_gain()
//!          ⇒ 交织成立体声（音轨音量 × 等功率声相）
//!          ⇒ render::write_master_container(.., with_bext = false)
//!               ← 与 yeban_render_master **同一份**编码器：TPDF 抖动 → 24-bit PCM
//!                 → RIFF 容器。⛔ 本模块不自写 RIFF 头。
//!                 `false` = 不写 bext ⇒ 无时间戳 ⇒ 同一请求逐字节可复现。
//!          ⇒ store::write_project_atomic(临时文件) ⇒ rename  [ARCH-SEC-004]
//! ```
//!
//! ## 六条诚实登记（⛔ 不假装）
//!
//! 1. **采样文件会被真读**：`Region::sample_path()`（含 `<control> default_path=` 前缀的
//!    **有效路径**）指向的 PCM WAV 由 [`read_sample_file`]
//!    （→ `yeban_render::wav::read_plain_wav_mono_f32`，与 `app` 侧**同一份**归一化）解码，
//!    经 [`SampleCache`] 去重并受三条上限约束；响应里的 `sampleSource` / `sampleFilesRead`
//!    逐字反映读了哪些文件。⚠ 读不到（或没有区域声明 `sample=`）时**如实回退**到
//!    [`InMemorySfzSample::sine`]（内存正弦）并把原因写进 `sampleSource`。
//!    ⛔ 内建静音伪样本**不走这条路**（它没有文件可读，见第 6 条）。
//! 2. **工具面没有设备链**：`DeviceKind` 没有 SFZ 变体，本模块**不**往
//!    `TrackV3::devices` 里塞任何东西（塞一个就是发明规范）。SFZ 音色只在这一条
//!    **显式**导出路径上取代 MIDI 合成源。
//! 3. **没有循环**：SFZ 的 `loop_mode` / `loop_start` / `loop_end` 没有被求值
//!    （`PlaybackSpec::loops()` 为真时**登记**，⛔ 不静默当一个长音）。
//! 4. **只走 `yeban-sfz` 的公开口径**：区域选择用 `matches_key` / `matches_velocity`，
//!    速率用 `playback_rate`，增益用 `playback_spec(..).total_gain()` —— 本层
//!    ⛔ 不重写这些规则（也没有把 `yeban-engine` 拖进 MCP：见
//!    `crates/yeban-mcp/Cargo.toml` 的"轻量 crate"注释）。
//! 5. ⭐ **`#include` 在装载期展开**（任务单 `#208`）：走 `yeban-sfz` 的
//!    `IncludeResolver` + `parse_sources`（与 `yeban-engine` 的
//!    `SfzInstrument::load_with_includes` **同一份**实现，⛔ 不是 MCP 侧第二套展开器）。
//!    缺失的 include 只记进 [`SfzLoad::include_issues`]（⛔ 不让装载失败）；循环 / 超深 /
//!    沙箱逃逸仍是错误。展开后的片段序列随 [`SfzLoad::sources`] 一起登记，因此
//!    `yeban_render_sfz_wav` 的 `with_instrument` 重放的是**同一份**片段（⛔ 不是拿根文本
//!    再 `parse_text` 把 include 悄悄丢掉）。
//! 6. ⭐ **只认 `*silence`**（任务单 `#218`）：`sample=*silence` 是 SFZ 的**内建静音伪样本**
//!    （⛔ 不是文件）。判据**只有一份**：`yeban-sfz` 的 [`Region::is_builtin_silence()`]
//!    （判 `sample=` **原值**，⛔ 不判拼过 `default_path` 的 `sample_path()`；也⛔ 不判
//!    `*Silence` 这类大小写变体）。装载期：**⛔ 不读盘**、⛔ 不进 `sampleFailures`、
//!    计入 [`SfzLoad::silent_region_count`]、区域状态逐字 [`BUILTIN_SILENCE_SOURCE`]；
//!    渲染期：该区域**真的写零**（⛔ 不回退内存正弦）。其它 ARIA 内建波形
//!    （`*sine` / `*saw` / `*square` / …）**未实现** —— 仍按普通样本路径处理。
//!
//! 本模块是**离线/控制侧**代码：`parse_text` / `parse_sources` 会分配 ⇒ ⛔ 不得在音频
//! 回调里调用（红线 7 / `MUST-GATE-001`）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use yeban_model::{AssetHash, ClipContent, PPQ, YebanProjectV1};
use yeban_render::wav::read_plain_wav_mono_f32;
use yeban_sfz::instrument::{Instrument, Region};
use yeban_sfz::parser::{
    IncludeIssue, IncludeIssueKind, IncludeResolver, ParseLimits, SfzSource, parse_sources,
    parse_text,
};
use yeban_sfz::playback::RenderRates;

use super::error::Fault;
use super::render;
use super::render_math as math;
use crate::tools::ErrorCode;

/// 装载实参的默认样本采样率（Hz）。
pub const DEFAULT_SAMPLE_HZ: f32 = 48_000.0;

/// 渲染实参的默认输出采样率（Hz）。
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// 渲染实参的默认力度。
pub const DEFAULT_VELOCITY: u8 = 100;

/// 渲染实参的默认样本频率（Hz）——与 `yeban-app --sfz` 的探测音同一个值。
pub const DEFAULT_TONE_HZ: f32 = 440.0;

/// 样本来源的**如实**文案（⛔ 不是采样文件名）。
pub const SAMPLE_SOURCE: &str = "in-memory-sine";

/// ⭐ 内建静音伪样本的来源/状态文案（任务单 `#218`）。
///
/// 与 `yeban-app::sfz_load::BUILTIN_SILENCE_SOURCE` **逐字相同**（本 crate 不依赖 app，
/// 因此这里是同一个显示字面量；**判定逻辑**仍只有 `yeban-sfz` 一份）。
/// 它描述的区域状态是 `builtin-silence(*silence)` —— ⛔ 不是 `unreadable(...)`。
pub const BUILTIN_SILENCE_SOURCE: &str = "builtin-silence(*silence)";

/// 每个音符末尾的淡出帧数（去咔哒；与 `render.rs` 的 `RELEASE_MS` 同族的最小实现）。
const NOTE_FADE_FRAMES: u64 = 64;

/// 单个样本文件的**读取上限**（字节）。
///
/// `read_plain_wav` 会把整份 PCM 读进内存 ⇒ 一个 10 分钟的立体声文件就是几百 MB。
/// 因此先看 `metadata.len()`：**超过上限就拒绝读取**，并把原因写进来源文案 ——
/// ⛔ **绝不静默截断**（截断会让"听到的"与"文件里的"不是同一段音频，且没人知道）。
pub const MAX_SAMPLE_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// 一次调用最多读多少个**不同**样本文件（去重后）。
pub const MAX_SAMPLE_FILES_READ: usize = 32;

/// 一次调用最多读入多少帧（所有文件**合计**）——第二道上界，按**解码后**的规模算。
pub const MAX_SAMPLE_FRAMES_TOTAL: u64 = 8_000_000;

/// 内存正弦回退的频率（Hz）——与 `app` 侧 `region_sample_for` 的回退同值。
pub const FALLBACK_SINE_HZ: f32 = 440.0;

/// 内存正弦回退的采样率（Hz）——与 `app` 侧同值。
pub const FALLBACK_SINE_HZ_RATE: f32 = 48_000.0;

/// 读一个样本文件的结果（单声道，归一化到 `[-1, 1]`）。
#[derive(Clone, Debug, PartialEq)]
pub struct SampleFileReading {
    /// 单声道 PCM。
    pub pcm: Arc<Vec<f32>>,
    /// 文件自身的采样率（Hz）。
    pub sample_hz: f32,
    /// 帧数（= `pcm.len()`）。
    pub frames: u64,
    /// 如实描述（`file(<path> frames=N rate=R)`）。
    pub describe: String,
}

/// 读一份 WAV 样本文件（先过**单文件字节上限**，再交给共享读取器）。
///
/// ⚠ 归一化**不在本函数里**：它调 `yeban_render::wav::read_plain_wav_mono_f32`
/// （集成者裁决：归一化的**唯一**自然归属是 `yeban-render` —— `app` 与 `mcp`
/// **都**依赖它，那不是新 crate、也不是第二份实现）。⇒ 全仓的"WAV → 单声道 f32"
/// 只有一份口径。
///
/// 本函数只多做两件属于 MCP 侧的事：
/// ① **先看 `metadata.len()`** 再决定读不读（`read_plain_wav` 会把整份 PCM 读进内存）；
/// ② 把读数包成 [`SampleFileReading`]（`Arc` 让同一份样本被多个音符共享）。
///
/// # Errors
///
/// 返回**人话原因**（⛔ 不是 `Fault`）：调用方按"如实回退"处理，而不是让整次调用失败。
pub fn read_sample_file(path: &Path) -> Result<SampleFileReading, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("{} ({})", path.display(), error.kind()))?;
    if metadata.is_dir() {
        return Err(format!("{} 是一个目录", path.display()));
    }
    if metadata.len() > MAX_SAMPLE_FILE_BYTES {
        return Err(format!(
            "{} 有 {} 字节, 超过单文件上限 {} 字节 (拒绝读取, ⛔ 不截断)",
            path.display(),
            metadata.len(),
            MAX_SAMPLE_FILE_BYTES
        ));
    }
    let (pcm, sample_rate, describe) =
        read_plain_wav_mono_f32(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let frames = u64::try_from(pcm.len()).unwrap_or(u64::MAX);
    Ok(SampleFileReading {
        pcm: Arc::new(pcm),
        sample_hz: sample_rate as f32,
        frames,
        describe,
    })
}

/// `sample=` 的路径解析：绝对路径原样用；相对路径**相对 SFZ 文件所在目录**（SFZ 惯例）。
#[must_use]
pub fn resolve_sample_path(sfz_path: &Path, sample: &str) -> PathBuf {
    let raw = Path::new(sample);
    if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        sfz_path
            .parent()
            .map_or_else(|| raw.to_path_buf(), |parent| parent.join(raw))
    }
}

/// 一次调用里的**样本文件缓存**（去重 + 上界 + 逐条如实记账）。
///
/// 上界三条（任一命中就**停止再读**并如实标注，⛔ 不静默丢弃）：
/// ① 单个文件字节数 > [`MAX_SAMPLE_FILE_BYTES`]；② 不同文件数 > [`MAX_SAMPLE_FILES_READ`]；
/// ③ 累计解码帧数 > [`MAX_SAMPLE_FRAMES_TOTAL`]。
#[derive(Debug, Default)]
pub struct SampleCache {
    entries: BTreeMap<PathBuf, Result<SampleFileReading, String>>,
    frames_loaded: u64,
    truncated: bool,
}

impl SampleCache {
    /// 空缓存。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 取一份样本（命中缓存则**不**重复读盘）。
    ///
    /// # Errors
    ///
    /// 返回**人话原因**：读盘失败、上界命中（含"已截断"）。
    pub fn get(&mut self, path: &Path) -> Result<SampleFileReading, String> {
        if let Some(hit) = self.entries.get(path) {
            return hit.clone();
        }
        if self.entries.len() >= MAX_SAMPLE_FILES_READ {
            self.truncated = true;
            let reason = format!(
                "已有 {} 个不同样本文件, 达到本次调用的上限 {} ⇒ 不再读盘 (⛔ 不静默截断)",
                self.entries.len(),
                MAX_SAMPLE_FILES_READ
            );
            self.entries.insert(path.to_path_buf(), Err(reason.clone()));
            return Err(reason);
        }
        let outcome = match read_sample_file(path) {
            Ok(reading) => {
                if self.frames_loaded.saturating_add(reading.frames) > MAX_SAMPLE_FRAMES_TOTAL {
                    self.truncated = true;
                    Err(format!(
                        "累计样本帧数会超过本次调用的上限 {} 帧 (已读 {}), 拒绝再读 {} (⛔ 不静默截断)",
                        MAX_SAMPLE_FRAMES_TOTAL,
                        self.frames_loaded,
                        path.display()
                    ))
                } else {
                    self.frames_loaded = self.frames_loaded.saturating_add(reading.frames);
                    Ok(reading)
                }
            }
            Err(reason) => Err(reason),
        };
        self.entries.insert(path.to_path_buf(), outcome.clone());
        outcome
    }

    /// 读成功的**不同**文件数。
    #[must_use]
    pub fn files_read(&self) -> usize {
        self.entries.values().filter(|entry| entry.is_ok()).count()
    }

    /// 读失败的**不同**文件数。
    #[must_use]
    pub fn files_failed(&self) -> usize {
        self.entries.values().filter(|entry| entry.is_err()).count()
    }

    /// 是否命中过任何上界。
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// 已解码的总帧数。
    #[must_use]
    pub const fn frames_loaded(&self) -> u64 {
        self.frames_loaded
    }

    /// 逐条缓存项的只读迭代（路径 → 结果；`BTreeMap` ⇒ 路径字典序 ⇒ 确定性）。
    pub fn entries_iter(
        &self,
    ) -> impl Iterator<Item = (&PathBuf, &Result<SampleFileReading, String>)> {
        self.entries.iter()
    }

    /// 逐条来源描述（成功与失败都收；顺序 = 路径字典序 ⇒ 确定性）。
    #[must_use]
    pub fn descriptions(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|(path, entry)| match entry {
                Ok(reading) => reading.describe.clone(),
                Err(reason) => format!("unreadable({path}): {reason}", path = path.display()),
            })
            .collect()
    }
}

/// SFZ 装载/导出的错误码（全部取自 `ADR-0001` D25 的 20 值联集，⛔ 不发明新码）。
const CODE_IO: ErrorCode = ErrorCode::IoError;
const CODE_RANGE: ErrorCode = ErrorCode::InvalidParameterRange;
const CODE_RENDER: ErrorCode = ErrorCode::RenderFailed;

// ---------------------------------------------------------------------------
// ① yeban_load_sfz
// ---------------------------------------------------------------------------

/// 一条区域的只读摘要。
#[derive(Clone, Debug, PartialEq)]
pub struct RegionSummary {
    /// 区域下标（文件里的顺序）。
    pub index: usize,
    /// `sample=` 的原始取值（**原样登记**；它指向的文件读到没有，见 `sample_file_state`）。
    pub sample: String,
    /// 键下限（`lokey`）。
    pub key_low: i32,
    /// 键上限（`hikey`）。
    pub key_high: i32,
    /// 力度下限（`lovel`）。
    pub velocity_low: u8,
    /// 力度上限（`hivel`）。
    pub velocity_high: u8,
    /// 根音（`pitch_keycenter`）。
    pub pitch_keycenter: i32,
    /// 是否由音符触发（`key=-1` 时为 `false`）。
    pub trigger_by_note: bool,
    /// 该区域是否要求循环（`loop_mode` 命中且窗口有效）——本实现**不**展开循环。
    pub loops: bool,
    /// ⭐ `sample=` 是**内建静音伪样本** `*silence`（任务单 `#218`）。
    ///
    /// 判定来自 [`Region::is_builtin_silence()`]（与引擎/app **同一份**）；为真时该区域
    /// 不读盘、不计入 `sampleFailures`、渲染时写零。
    pub builtin_silence: bool,
    /// `sample=` 指向的文件的**读取真相**：`file(<path> frames=N rate=R)`（读到了）
    /// 或 `unreadable(<path>): <原因>`（没读到 ⇒ 渲染时回退内存正弦）；
    /// ⭐ 内建静音则是 [`BUILTIN_SILENCE_SOURCE`]（没有文件可读）。
    pub sample_file_state: String,
}

impl RegionSummary {
    fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "index": self.index,
            "sample": self.sample,
            "keyRange": [self.key_low, self.key_high],
            "velocityRange": [self.velocity_low, self.velocity_high],
            "pitchKeycenter": self.pitch_keycenter,
            "triggerByNote": self.trigger_by_note,
            "loops": self.loops,
            "builtinSilence": self.builtin_silence,
            "sampleFileState": self.sample_file_state,
        })
    }
}

/// 一次 SFZ 装载的**已解析**产物（`plan` 的产物；`apply` 把它放进会话 registry）。
#[derive(Clone, Debug)]
pub struct SfzLoad {
    /// 会话内 instrumentId。
    pub id: u64,
    /// 磁盘路径（响应里原样回显）。
    pub path: PathBuf,
    /// 文件字节数。
    pub bytes: u64,
    /// 文件内容的 SHA-256（`AssetHash` 的文本形态）。
    pub digest: String,
    /// SFZ 原文（根文件；会话 registry 的载荷之一）。
    pub text: String,
    /// ⭐ 已展开 `#include` 的片段序列（`Some` ⇒ `parse_sources`；`None` ⇒ 纯文本
    /// `parse_text`，即根路径没有可用的 UTF-8 文件名时的既有行为）。
    pub sources: Option<Arc<[SfzSource]>>,
    /// ⭐ 容错展开时被跳过的 include（空 ⇒ 全部目标都读到了）。
    pub include_issues: Vec<IncludeIssue>,
    /// 区域摘要（顺序 = 文件里的顺序）。
    pub regions: Vec<RegionSummary>,
    /// ⭐ 内建静音区域数（`sample=*silence`；任务单 `#218`）。
    ///
    /// 与 app 的 `silent_region_count` **口径差别如实登记**：app 的 `silent` 是引擎摊平
    /// 口径（缺样本 ＋ 内建静音都不产生输出）；MCP **没有** `NO_SAMPLE` 摊平 —— 读不到的
    /// 样本会**回退内存正弦**（模块头第 1 条），因此这里**只**数"按设计就不产生采样输出"
    /// 的内建静音区域，⛔ 不把回退正弦的区域算成静音（那会是假话）。缺样本另有
    /// [`Self::sample_failures`] / `sampleFilesFailedCount` 两个读数。
    pub silent_region_count: usize,
    /// 是否复用了已装载的同一份内容（内容寻址 ⇒ 同文本同一个 instrumentId）。
    pub reused: bool,
    /// 请求的样本侧采样率（进报告；它只用于**回退正弦**与报告口径）。
    pub sample_hz: f32,
    /// 本次装载**真的读到**的样本文件来源描述（升序；空 = 一个都没读到）。
    pub sample_sources: Vec<String>,
    /// 读失败的样本文件（`unreadable(...)` 摘要，升序）。
    pub sample_failures: Vec<String>,
    /// 是否命中过读盘上界（文件数 / 累计帧数）。
    pub sample_reads_truncated: bool,
    /// 累计解码帧数（所有读成功的文件之和）。
    pub sample_frames_loaded: u64,
}

impl SfzLoad {
    /// 装载的样本来源**汇总文案**（如实：读到就点名文件，读不到就说原因）。
    ///
    /// - 读到了至少一个文件 ⇒ `file(<第一个来源>)`（其余进 `sampleSources` 数组）；
    /// - 一个都没读到 ⇒ [`SAMPLE_SOURCE`]（内存正弦）＋**回退原因**（若有区域声明了 `sample=`）；
    /// - ⭐ 只有内建静音区域、既无文件也无失败 ⇒ [`BUILTIN_SILENCE_SOURCE`]
    ///   （⛔ 不说成 `in-memory-sine` —— 那会把"按设计静音"说成"缺文件回退"）。
    #[must_use]
    pub fn sample_source(&self) -> String {
        let silence_note = if self.silent_region_count > 0 {
            format!("; {} 个区域内建静音", self.silent_region_count)
        } else {
            String::new()
        };
        match (self.sample_sources.first(), self.sample_failures.first()) {
            // `describe` 自己就以 `file(` 开头 ⇒ ⛔ 不再包一层 `file(...)`。
            (Some(first), _) => format!(
                "{first}{}{silence_note}",
                if self.sample_sources.len() > 1 {
                    format!("; 另 {} 个见 sampleSources", self.sample_sources.len() - 1)
                } else {
                    String::new()
                }
            ),
            (None, Some(reason)) => format!("{SAMPLE_SOURCE} (回退: {reason}){silence_note}"),
            (None, None) if self.silent_region_count > 0 => BUILTIN_SILENCE_SOURCE.to_owned(),
            (None, None) => SAMPLE_SOURCE.to_owned(),
        }
    }

    /// **只读**地把原文（或已展开的片段序列）交给 `yeban-sfz` 的解析器
    /// （`plan` 与 `apply` 共用一份口径）。
    ///
    /// ⭐ `sources` 为 `Some` 时重放**同一份**片段序列 —— 否则渲染路径会把装载时
    /// 展开过的 include 悄悄丢掉（区域数变 0，音符无处可去）。
    ///
    /// # Errors
    ///
    /// 文本非法 ⇒ `INVALID_PARAMETER_RANGE` + `data.reason = "sfzParseFailed"`。
    pub fn with_instrument<R>(&self, f: impl FnOnce(&Instrument<'_>) -> R) -> Result<R, Fault> {
        parse_expanded(&self.text, self.sources.as_deref()).map(|instrument| f(&instrument))
    }

    /// ⭐ 被跳过的 `#include` 的 JSON 形态（`line` / `path` / `kind` / `display`）。
    #[must_use]
    pub fn include_issues_value(&self) -> serde_json::Value {
        include_issues_value(&self.include_issues)
    }

    /// 真调用响应的 `data`。
    #[must_use]
    pub fn response_data(&self) -> serde_json::Value {
        serde_json::json!({
            "loaded": true,
            "instrumentId": self.id,
            "path": self.path.display().to_string(),
            "bytes": self.bytes,
            "sha256": self.digest,
            "regions": self.regions.len(),
            "silentRegionCount": self.silent_region_count,
            "regionDetails": self.regions.iter().map(RegionSummary::to_value).collect::<Vec<_>>(),
            "includeIssues": self.include_issues_value(),
            "includeIssueCount": self.include_issues.len(),
            "sampleSource": self.sample_source(),
            "sampleFilesRead": !self.sample_sources.is_empty(),
            "sampleFilesReadCount": self.sample_sources.len(),
            "sampleFilesFailedCount": self.sample_failures.len(),
            "sampleSources": self.sample_sources,
            "sampleFailures": self.sample_failures,
            "sampleReadsTruncated": self.sample_reads_truncated,
            "sampleFramesLoaded": self.sample_frames_loaded,
            "sampleHz": self.sample_hz,
            "sessionScoped": true,
            "contentAddressed": true,
            "reused": self.reused,
        })
    }

    /// `dryRun` 预览：同一批实测数字 + `wouldRegister`（**不注册**）。
    #[must_use]
    pub fn preview(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path.display().to_string(),
            "bytes": self.bytes,
            "sha256": self.digest,
            "regions": self.regions.len(),
            "silentRegionCount": self.silent_region_count,
            "regionDetails": self.regions.iter().map(RegionSummary::to_value).collect::<Vec<_>>(),
            "includeIssues": self.include_issues_value(),
            "includeIssueCount": self.include_issues.len(),
            "sampleSource": self.sample_source(),
            "sampleFilesRead": !self.sample_sources.is_empty(),
            "sampleFilesReadCount": self.sample_sources.len(),
            "sampleFilesFailedCount": self.sample_failures.len(),
            "sampleSources": self.sample_sources,
            "sampleFailures": self.sample_failures,
            "sampleReadsTruncated": self.sample_reads_truncated,
            "sampleFramesLoaded": self.sample_frames_loaded,
            "wouldRegister": {
                "instrumentId": self.id,
                "sessionScoped": true,
                "contentAddressed": true,
                "reused": self.reused,
            },
        })
    }
}

/// 解析 SFZ 文本（**纯文本模式**：`#include` 被忽略，既有行为）。
///
/// 装载路径用 `parse_expanded`（带已展开的片段序列）；本函数只保留给
/// "手里只有文本"的调用方，⛔ 不要在装载路径上用它（那会把 include 悄悄丢掉）。
///
/// # Errors
///
/// 文本非法 ⇒ `INVALID_PARAMETER_RANGE` + `data.reason = "sfzParseFailed"`。
pub fn parse(text: &str) -> Result<Instrument<'_>, Fault> {
    parse_expanded(text, None)
}

/// 解析根文本或**已展开的片段序列**（本模块唯一的解析入口）。
///
/// `Some(sources)` ⇒ `parse_sources`（跨片段状态延续，`#include` 真的进来了）；
/// `None` ⇒ `parse_text`（纯文本，既有行为）。
///
/// # Errors
///
/// 解析器拒绝（语法 / 行超长 / 警告上限等）⇒ `INVALID_PARAMETER_RANGE`
/// + `data.reason = "sfzParseFailed"`。
fn parse_expanded<'a>(
    text: &'a str,
    sources: Option<&'a [SfzSource]>,
) -> Result<Instrument<'a>, Fault> {
    let limits = ParseLimits::default();
    match sources {
        Some(sources) => parse_sources(sources, &limits),
        None => parse_text(text, &limits),
    }
    .map_err(|error| sfz_fault(&error))
}

/// 把 `yeban-sfz` 的错误映射成 MCP 的领域错误（码取自 D25 联集，⛔ 不发明新码）。
///
/// 循环 include 也走这条：`error` 的 `Display` 是
/// `include cycle detected at ...` ⇒ 调用方拿得到逐字原因。
fn sfz_fault(error: &yeban_sfz::SfzError) -> Fault {
    Fault::domain_with_data(
        CODE_RANGE,
        format!("SFZ 文本被解析器拒绝: {error}"),
        serde_json::json!({
            "reason": "sfzParseFailed",
            "parser": format!("{error:?}"),
        }),
    )
}

/// 一次 include 展开的结果：片段序列 ＋ 被跳过的 include（`None` ⇒ 纯文本模式）。
type ResolvedIncludeSources = (Vec<SfzSource>, Vec<IncludeIssue>);

/// ⭐ 在**装载期**展开 `#include`（任务单 `#208`）。
///
/// 与 `yeban-engine` 的 `SfzInstrument::load_with_includes` 走**同一份** `yeban-sfz`
/// 实现：`IncludeResolver` 做沙箱 + 环检测 + 容错展开，`parse_sources` 解析片段序列。
/// 本函数只补两件 MCP 侧的事：
/// ① 由**根文件路径**推出基准目录与 entry（`path.parent()` / `path.file_name()`）；
/// ② 把错误映射成 [`Fault`]。
///
/// 返回 `None` ⇒ 路径没有可用的 UTF-8 文件名（例如以 `..` 结尾的怪路径）⇒ 调用方
/// 退回纯文本模式（`parse_text` 的既有行为），⛔ 不因为一个无法定位的文件名让装载失败。
///
/// # Errors
///
/// 循环 / 超深 / 超文件数 / 沙箱逃逸 / 被包含文件非 UTF-8 ⇒ [`Fault`]
/// （`INVALID_PARAMETER_RANGE` + `sfzParseFailed`）。**缺失的目标不是错误**。
fn resolve_include_sources(
    text: &str,
    path: &Path,
) -> Result<Option<ResolvedIncludeSources>, Fault> {
    let base_dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let Some(entry) = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|entry| !entry.is_empty())
    else {
        return Ok(None);
    };
    let limits = ParseLimits::default();
    let resolver = IncludeResolver::new(base_dir, limits).map_err(|error| sfz_fault(&error))?;
    let resolution = resolver
        .resolve_text_lossy(entry, text)
        .map_err(|error| sfz_fault(&error))?;
    Ok(Some((resolution.sources, resolution.issues)))
}

/// `include_issues` 的 JSON 形态（`line` / `path` / `kind` / `display`）。
fn include_issues_value(issues: &[IncludeIssue]) -> serde_json::Value {
    serde_json::Value::Array(
        issues
            .iter()
            .map(|issue| {
                serde_json::json!({
                    "line": issue.line,
                    "path": issue.path,
                    "kind": match issue.kind {
                        IncludeIssueKind::NotFound => "notFound",
                        IncludeIssueKind::NoMatch => "noMatch",
                    },
                    "display": issue.to_string(),
                })
            })
            .collect(),
    )
}

/// 解析 `yeban_load_sfz` 的实参（**只做形状校验**，读文件在 [`load`] 里）。
///
/// # Errors
///
/// 缺 `path` / 空 `path` / `sampleHz` 非有限正数 ⇒ `INVALID_PARAMETER_RANGE`。
pub fn parse_load_arguments(
    arguments: &serde_json::Map<String, serde_json::Value>,
) -> Result<(PathBuf, f32), Fault> {
    let path = arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Fault::domain(CODE_RANGE, "`path` 必须是字符串 (SFZ 文件路径)"))?;
    if path.is_empty() {
        return Err(Fault::domain(CODE_RANGE, "`path` 不得为空串"));
    }
    let sample_hz = match arguments.get("sampleHz") {
        None => DEFAULT_SAMPLE_HZ,
        Some(value) => {
            let hz = value.as_f64().ok_or_else(|| {
                Fault::domain(
                    CODE_RANGE,
                    format!("`sampleHz` 必须是数字, 实际收到 {value}"),
                )
            })? as f32;
            if !hz.is_finite() || hz <= 0.0 {
                return Err(Fault::domain(
                    CODE_RANGE,
                    format!("`sampleHz` 必须是有限正数 (Hz), 实际收到 {hz}"),
                ));
            }
            hz
        }
    };
    Ok((PathBuf::from(path), sample_hz))
}

/// 装载一份 SFZ：读文件 ⇒ SHA-256 ⇒ 解析 ⇒ 区域摘要。
///
/// **只读**：不写盘、不改工程。`assign_id` 由调用方（`Domain`）按已有的 registry 决定。
///
/// # Errors
///
/// - `IO_ERROR`：文件不存在 / 是目录 / 读失败；
/// - `INVALID_PARAMETER_RANGE`：非 UTF-8 文本，或 SFZ 文本被解析器拒绝。
pub fn load(path: &Path, sample_hz: f32, assign_id: u64, reused: bool) -> Result<SfzLoad, Fault> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        Fault::domain_with_data(
            CODE_IO,
            format!("读 SFZ 文件失败: {} ({error})", path.display()),
            serde_json::json!({
                "reason": "sfzReadFailed",
                "path": path.display().to_string(),
                "ioError": error.kind().to_string(),
            }),
        )
    })?;
    if metadata.is_dir() {
        return Err(Fault::domain_with_data(
            CODE_IO,
            format!("SFZ 路径是一个目录, 不是文件: {}", path.display()),
            serde_json::json!({
                "reason": "sfzPathIsDirectory",
                "path": path.display().to_string(),
            }),
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        Fault::domain_with_data(
            CODE_IO,
            format!("读 SFZ 文件失败: {} ({error})", path.display()),
            serde_json::json!({
                "reason": "sfzReadFailed",
                "path": path.display().to_string(),
                "ioError": error.kind().to_string(),
            }),
        )
    })?;
    // ⚠ `String::from_utf8` 而不是 `from_utf8_lossy`：把非法字节**静默**换成 U+FFFD
    // 之后解析器会看到一份"看起来合法"的文本，区域数可能凭空变多 —— 那是发明输入。
    let text = String::from_utf8(bytes.clone()).map_err(|error| {
        Fault::domain_with_data(
            CODE_RANGE,
            format!("SFZ 文件不是合法 UTF-8 文本: {}", path.display()),
            serde_json::json!({
                "reason": "sfzNotUtf8",
                "path": path.display().to_string(),
                "validUpTo": error.utf8_error().valid_up_to(),
            }),
        )
    })?;
    let digest = AssetHash::of_bytes(&bytes).as_str().to_owned();
    // ⭐ `#208`：先把 `#include` 展开（缺失只记一条、循环/超深报错），再解析片段序列。
    let resolution = resolve_include_sources(&text, path)?;
    // ---- 逐区域**真读** `sample=<path>`（有界；读不到如实回退，⛔ 不让装载失败）----
    // ⭐ `#218`：`sample=*silence` **⛔ 不读盘**（它不是文件）—— 区域状态记内建静音，
    // ⛔ 不进 `sampleFailures`。判定只有一份：`yeban-sfz` 的 `Region::is_builtin_silence()`。
    let (
        regions,
        silent_region_count,
        sample_sources,
        sample_failures,
        sample_reads_truncated,
        sample_frames_loaded,
    ) = {
        let instrument = match resolution.as_ref() {
            Some((sources, _)) => parse_expanded(&text, Some(sources.as_slice())),
            None => parse_expanded(&text, None),
        }?;
        let mut cache = SampleCache::new();
        let mut regions: Vec<RegionSummary> = Vec::new();
        for (index, region) in instrument.regions().iter().enumerate() {
            let mut summary = summarize(index, region);
            summary.sample_file_state = if summary.builtin_silence {
                BUILTIN_SILENCE_SOURCE.to_owned()
            } else {
                // ⚠ 读盘用**有效路径**（`sample_path()` 会拼 `default_path=`）；
                // 内建静音的判定用**原值**（上面 `builtin_silence`，`#217` 的裁决）。
                let sample = region.sample_path();
                if sample.trim().is_empty() {
                    "noSampleOpcode".to_owned()
                } else {
                    let resolved = resolve_sample_path(path, &sample);
                    match cache.get(&resolved) {
                        Ok(reading) => reading.describe,
                        Err(reason) => format!("unreadable({}): {reason}", resolved.display()),
                    }
                }
            };
            regions.push(summary);
        }
        let silent_region_count = regions
            .iter()
            .filter(|region| region.builtin_silence)
            .count();
        let mut sample_sources: Vec<String> = cache
            .entries_iter()
            .filter_map(|(_, entry)| entry.as_ref().ok().map(|reading| reading.describe.clone()))
            .collect();
        sample_sources.sort_unstable();
        let mut sample_failures: Vec<String> = cache
            .entries_iter()
            .filter_map(|(path, entry)| {
                entry
                    .as_ref()
                    .err()
                    .map(|reason| format!("unreadable({}): {reason}", path.display()))
            })
            .collect();
        sample_failures.sort_unstable();
        (
            regions,
            silent_region_count,
            sample_sources,
            sample_failures,
            cache.truncated(),
            cache.frames_loaded(),
        )
    };
    // 片段序列随装载一起登记：渲染路径的 `with_instrument` 必须重放**同一份**展开结果。
    let (sources, include_issues) = match resolution {
        Some((sources, issues)) => (Some(Arc::from(sources)), issues),
        None => (None, Vec::new()),
    };
    Ok(SfzLoad {
        id: assign_id,
        path: path.to_path_buf(),
        bytes: metadata.len(),
        digest,
        text,
        sources,
        include_issues,
        regions,
        silent_region_count,
        reused,
        sample_hz,
        sample_sources,
        sample_failures,
        sample_reads_truncated,
        sample_frames_loaded,
    })
}

/// 一条区域的摘要（字段直接读 `yeban-sfz` 的公开字段，⛔ 不重写键/力度判定）。
fn summarize(index: usize, region: &Region<'_>) -> RegionSummary {
    RegionSummary {
        index,
        // ⚠ 读 `Region::sample`（`sample=` 的**原值**），⛔ 不读 `sample_path()`：
        // 后者拼了 `<control> default_path=` 前缀，而本字段的契约是"原样登记"。
        // 内建静音的判定同理判原值（`#217` 的裁决）。
        sample: region.sample.as_ref().to_owned(),
        key_low: region.lokey,
        key_high: region.hikey,
        velocity_low: region.lovel,
        velocity_high: region.hivel,
        pitch_keycenter: region.pitch_keycenter,
        trigger_by_note: region.trigger_by_note,
        loops: region
            .playback_spec(
                index_to_u8(region.pitch_keycenter),
                100,
                RenderRates::default(),
            )
            .loops(),
        builtin_silence: region.is_builtin_silence(),
        // 由 `load` 的读盘那一步填（`summarize` 只做纯解析摘要，⛔ 不碰文件系统）。
        sample_file_state: "notProbed".to_owned(),
    }
}

/// `pitch_keycenter` 是 `i32`（规范允许 `key=-1` 这类极端写法），摘要只需要一个
/// 可用的音符号 ⇒ 夹到 `0..=127`。
fn index_to_u8(value: i32) -> u8 {
    u8::try_from(value.clamp(0, 127)).unwrap_or(60)
}

// ---------------------------------------------------------------------------
// ② yeban_render_sfz_wav
// ---------------------------------------------------------------------------

/// 一次已校验的 SFZ 导出请求（**还没有渲染**）。
#[derive(Clone, Debug, PartialEq)]
pub struct SfzWavRequest {
    /// 会话内 instrumentId。
    pub instrument_id: u64,
    /// 输出路径。
    pub path: PathBuf,
    /// 输出总帧数（> 0）。
    pub frames: u64,
    /// 输出采样率（Hz）。
    pub sample_rate: u32,
    /// 整段使用的力度（0..=127）。
    pub velocity: u8,
    /// 内存正弦样本的频率（Hz）。
    pub tone_hz: f32,
    /// 只渲染这一个音高；`None` = 渲染活跃工程 MIDI 片段里的全部音符。
    pub note: Option<u8>,
}

impl SfzWavRequest {
    /// 诚实描述这次请求（响应里原样回显）。
    #[must_use]
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "instrumentId": self.instrument_id,
            "path": self.path.display().to_string(),
            "frames": self.frames,
            "sampleRate": self.sample_rate,
            "velocity": self.velocity,
            "toneHz": self.tone_hz,
            "note": self.note,
            "bitDepth": render::OUTPUT_BIT_DEPTH,
            "channels": render::MASTER_CHANNELS,
        })
    }
}

/// 解析并**校验** `yeban_render_sfz_wav` 的实参。
///
/// # Errors
///
/// 缺必填 / 类型不对 / 越界 / 帧数超上限 / 输出路径撞工程或锁文件
/// ⇒ 一律 `INVALID_PARAMETER_RANGE`（联集里的既有码，⛔ 不发明新码）。
pub fn validate(
    arguments: &serde_json::Map<String, serde_json::Value>,
) -> Result<SfzWavRequest, Fault> {
    let instrument_id = match arguments.get("instrumentId") {
        Some(value) => value.as_u64().filter(|id| *id >= 1).ok_or_else(|| {
            Fault::domain(
                CODE_RANGE,
                format!("`instrumentId` 必须是 >= 1 的整数, 实际收到 {value}"),
            )
        })?,
        None => {
            return Err(Fault::domain(
                CODE_RANGE,
                "缺少必填实参 `instrumentId` (先调 yeban_load_sfz)",
            ));
        }
    };
    let path = arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Fault::domain(CODE_RANGE, "`path` 必须是字符串 (输出 WAV 路径)"))?;
    if path.is_empty() {
        return Err(Fault::domain(CODE_RANGE, "`path` 不得为空串"));
    }
    let frames = match arguments.get("frames") {
        Some(value) => value.as_u64().filter(|frames| *frames > 0).ok_or_else(|| {
            Fault::domain(
                CODE_RANGE,
                format!("`frames` 必须是 > 0 的整数, 实际收到 {value}"),
            )
        })?,
        None => {
            return Err(Fault::domain(
                CODE_RANGE,
                "缺少必填实参 `frames` (输出总帧数)",
            ));
        }
    };
    if frames > render::MAX_RENDER_FRAMES {
        return Err(Fault::domain_with_data(
            CODE_RANGE,
            format!(
                "`frames` 超出单次上限 ({} 帧 = 1 小时 @48 kHz), 实际 {frames}",
                render::MAX_RENDER_FRAMES
            ),
            serde_json::json!({
                "reason": "framesExceedCap",
                "frames": frames,
                "maxFrames": render::MAX_RENDER_FRAMES,
            }),
        ));
    }
    let sample_rate = match arguments.get("sampleRate") {
        None => DEFAULT_SAMPLE_RATE,
        Some(value) => value
            .as_u64()
            .and_then(|rate| u32::try_from(rate).ok())
            .filter(|rate| *rate > 0)
            .ok_or_else(|| {
                Fault::domain(
                    CODE_RANGE,
                    format!("`sampleRate` 必须是 > 0 的整数 (Hz), 实际收到 {value}"),
                )
            })?,
    };
    let velocity = match arguments.get("velocity") {
        None => DEFAULT_VELOCITY,
        Some(value) => value
            .as_u64()
            .and_then(|velocity| u8::try_from(velocity).ok())
            .filter(|velocity| *velocity <= 127)
            .ok_or_else(|| {
                Fault::domain(
                    CODE_RANGE,
                    format!("`velocity` 必须是 0..=127 的整数, 实际收到 {value}"),
                )
            })?,
    };
    let tone_hz = match arguments.get("toneHz") {
        None => DEFAULT_TONE_HZ,
        Some(value) => {
            let hz = value.as_f64().ok_or_else(|| {
                Fault::domain(CODE_RANGE, format!("`toneHz` 必须是数字, 实际收到 {value}"))
            })? as f32;
            if !hz.is_finite() || hz <= 0.0 {
                return Err(Fault::domain(
                    CODE_RANGE,
                    format!("`toneHz` 必须是有限正数 (Hz), 实际收到 {hz}"),
                ));
            }
            hz
        }
    };
    let note = match arguments.get("note") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|note| u8::try_from(note).ok())
                // ⚠ `u8::try_from` 只挡住 0..=255：`note: 200` 会**通过**它然后
                // 安静地不命中任何区域（"看起来成功、其实什么都没渲染"）。
                // MIDI 音高的上界是 127（与 `velocity` 同一条口径）。
                .filter(|note| *note <= 127)
                .ok_or_else(|| {
                    Fault::domain(
                        CODE_RANGE,
                        format!("`note` 必须是 0..=127 的整数, 实际收到 {value}"),
                    )
                })?,
        ),
    };
    Ok(SfzWavRequest {
        instrument_id,
        path: PathBuf::from(path),
        frames,
        sample_rate,
        velocity,
        tone_hz,
        note,
    })
}

/// 一次导出的产物（`plan` 里渲染 + 量化完成；`apply` 只落盘）。
#[derive(Clone, Debug)]
pub struct SfzWavArtifact {
    /// 请求（响应里原样回显）。
    pub request: SfzWavRequest,
    /// 输出路径（与 `request.path` 相同；单独存一份便于 `apply` 直接用）。
    pub path: PathBuf,
    /// 24-bit WAV 字节（由 `render::write_master_container` 编码 ⇒ 与
    /// `yeban_render_master` **同一份**实现，唯一差别是这里 `with_bext = false`）。
    pub bytes: Vec<u8>,
    /// 容器头字节数（编码器的读数）。
    pub header_bytes: usize,
    /// 载荷字节数（= 帧 × 声道 × 3）。
    pub payload_bytes: usize,
    /// 渲染出的帧数（**实测**，不是估算）。
    pub frames: u64,
    /// 浮点母带的峰值（抖动之前）。
    pub peak: f32,
    /// 工程里被排程的音符数（`note` 过滤之后）。
    pub notes_scheduled: u64,
    /// 区域选择**没有命中**的音符数（合法：该音高在这份 SFZ 里不发声）。
    pub notes_unmatched: u64,
    /// 命中的区域下标（去重升序）。
    pub regions_used: Vec<usize>,
    /// 工程 MIDI 里被概率判为不触发的音符数（读数）。
    pub notes_silenced: u64,
    /// 是否有音符落在要求的循环区域上（本实现**不**展开循环 ⇒ 该音符只响一遍）。
    pub notes_looping: u64,
    /// ⭐ 落在**内建静音**区域（`sample=*silence`）上的音符数（任务单 `#218`）。
    ///
    /// 这些音符**不读盘**、⛔ 不计入 [`Self::notes_using_sine_fallback`]，写零帧。
    pub notes_using_builtin_silence: u64,
    /// 用**文件样本**发声的音符数。
    pub notes_using_file_sample: u64,
    /// 回退到**内存正弦**的音符数。
    pub notes_using_sine_fallback: u64,
    /// 真的读到的样本文件来源（升序去重）。
    pub sample_sources: Vec<String>,
    /// 读失败的样本文件（升序去重）。
    pub sample_failures: Vec<String>,
    /// 是否命中过读盘上界。
    pub sample_reads_truncated: bool,
    /// 累计解码帧数。
    pub sample_frames_loaded: u64,
}

impl SfzWavArtifact {
    /// 本次渲染**明确没做**的键（⛔ 不报已经做到的键）。
    #[must_use]
    pub fn unsupported_keys(&self) -> Vec<&'static str> {
        let mut keys = vec!["sfzLoopExpansion", "deviceChainDsp", "automationLanes"];
        // 只有**一个音符都没用上**文件样本时，才登记"样本文件解码没做"。
        if self.notes_using_file_sample == 0 {
            keys.insert(0, "samplerFileDecode");
        }
        keys
    }

    /// 音源**汇总文案**（如实：有文件样本就点名，全回退就说回退，内建静音单独点名）。
    #[must_use]
    pub fn sample_source(&self) -> String {
        let silence_note = if self.notes_using_builtin_silence > 0 {
            format!("; {} 个音符内建静音", self.notes_using_builtin_silence)
        } else {
            String::new()
        };
        match (self.sample_sources.first(), self.sample_failures.first()) {
            (Some(first), _) => format!(
                "{first}{}{}{silence_note}",
                if self.sample_sources.len() > 1 {
                    format!("; 另 {} 个见 sampleSources", self.sample_sources.len() - 1)
                } else {
                    String::new()
                },
                if self.notes_using_sine_fallback > 0 {
                    format!("; {} 个音符回退内存正弦", self.notes_using_sine_fallback)
                } else {
                    String::new()
                }
            ),
            (None, Some(reason)) => {
                format!("{SAMPLE_SOURCE} (回退: {reason}){silence_note}")
            }
            // ⭐ 只有内建静音（⛔ 不说成 `in-memory-sine`：没有回退，是设计如此）。
            (None, None) if self.notes_using_builtin_silence > 0 => {
                BUILTIN_SILENCE_SOURCE.to_owned()
            }
            (None, None) => SAMPLE_SOURCE.to_owned(),
        }
    }

    /// `dryRun` 预览（同一批实测数字 + `wouldWrite`；**不落盘**）。
    #[must_use]
    pub fn preview(&self) -> serde_json::Value {
        serde_json::json!({
            "renderer": "yeban-render",
            "request": self.request.to_value(),
            "sampleSource": self.sample_source(),
            "sampleFilesRead": !self.sample_sources.is_empty(),
            "sampleSources": self.sample_sources,
            "sampleFailures": self.sample_failures,
            "sampleReadsTruncated": self.sample_reads_truncated,
            "sampleFramesLoaded": self.sample_frames_loaded,
            "notesUsingFileSample": self.notes_using_file_sample,
            "notesUsingBuiltinSilence": self.notes_using_builtin_silence,
            "notesUsingSineFallback": self.notes_using_sine_fallback,
            "notesScheduled": self.notes_scheduled,
            "notesUnmatched": self.notes_unmatched,
            "notesSilenced": self.notes_silenced,
            "notesLooping": self.notes_looping,
            "regionsUsed": self.regions_used,
            "peak": self.peak,
            "wouldWrite": {
                "path": self.path.display().to_string(),
                "format": "wav",
                "frames": self.frames,
                "bytes": self.bytes.len(),
                "headerBytes": self.header_bytes,
                "payloadBytes": self.payload_bytes,
                "sha256": AssetHash::of_bytes(&self.bytes).as_str(),
                "bitDepth": render::OUTPUT_BIT_DEPTH,
                "channels": render::MASTER_CHANNELS,
            },
        })
    }

    /// 真调用响应的 `data`。
    #[must_use]
    pub fn response_data(&self) -> serde_json::Value {
        serde_json::json!({
            "rendered": true,
            "renderer": "yeban-render",
            "request": self.request.to_value(),
            "path": self.path.display().to_string(),
            "format": "wav",
            "bitDepth": render::OUTPUT_BIT_DEPTH,
            "channels": render::MASTER_CHANNELS,
            "sampleRate": self.request.sample_rate,
            "frames": self.frames,
            "bytes": self.bytes.len(),
            "headerBytes": self.header_bytes,
            "payloadBytes": self.payload_bytes,
            "sha256": AssetHash::of_bytes(&self.bytes).as_str(),
            "peak": self.peak,
            "notesScheduled": self.notes_scheduled,
            "notesUnmatched": self.notes_unmatched,
            "notesSilenced": self.notes_silenced,
            "notesLooping": self.notes_looping,
            "regionsUsed": self.regions_used,
            "sampleSource": self.sample_source(),
            "sampleFilesRead": !self.sample_sources.is_empty(),
            "sampleSources": self.sample_sources,
            "sampleFailures": self.sample_failures,
            "sampleReadsTruncated": self.sample_reads_truncated,
            "sampleFramesLoaded": self.sample_frames_loaded,
            "notesUsingFileSample": self.notes_using_file_sample,
            "notesUsingBuiltinSilence": self.notes_using_builtin_silence,
            "notesUsingSineFallback": self.notes_using_sine_fallback,
            "dither": "TPDF (ARCH-FMT-001)",
            "bwf": false,
            "bext": "none (本工具不写 bext/时间戳 ⇒ 同一请求逐字节可复现)",
            "atomic": true,
            "strategy": "同目录 .tmp-{ulid} + File::sync_all + rename (ARCH-SEC-004)",
            // ⚠ `samplerFileDecode` **只在真的没有音符用上文件样本时**才登记 ——
            // 报一个已经做到的键就是假话（与 `render.rs` 撤掉 `noteProbability` 同一条纪律）。
            "unsupported": self.unsupported_keys(),
        })
    }
}

/// 一条已经排程的音符（帧区间 + 音高 + 力度 + 该音符的左右增益）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfzNote {
    /// 起始帧（含）。
    pub start_frame: u64,
    /// 时值（帧；至少 1）。
    pub frames: u64,
    /// 音高 0..=127。
    pub pitch: u8,
    /// 力度 0..=127。
    pub velocity: u8,
    /// 左声道增益（音轨音量 × 等功率声相；mute/solo 已折进 0）。
    pub gain_l: f32,
    /// 右声道增益。
    pub gain_r: f32,
}

/// 从活跃工程收集 MIDI 音符（**与 `render_master` 同一份格点公式**）。
///
/// - 概率触发走模型层 `MidiNote::triggers`（本层不掷骰子、不引入熵源）；
/// - 连击按整数除法等分成 `ratchet` 个脉冲、余数不补（与实时引擎同一裁决）；
/// - 微时值偏移按 `render_math::note_frame_span` 换算；
/// - `only_note` 给了就只收那一个音高；
/// - 被曲目 mute / solo 门控掉的音轨 ⇒ 增益 0（它**仍**在排程里，读数因此看得见）。
///
/// 返回 `(音符列表, 被概率判为不触发的音符数)`。
#[must_use]
pub fn collect_notes(
    project: &YebanProjectV1,
    sample_rate: u32,
    only_note: Option<u8>,
) -> (Vec<SfzNote>, u64) {
    let mut notes: Vec<SfzNote> = Vec::new();
    let mut silenced = 0u64;
    let any_solo = project.tracks.values().any(|track| track.solo);
    for track in project.tracks.values() {
        let audible = !track.mute && (!any_solo || track.solo || track.solo_safe);
        let track_gain = if audible {
            db_to_linear(track.volume_db)
        } else {
            0.0
        };
        let (gain_l, gain_r) = if track.pan.is_finite() && track.pan != 0.0 {
            let (left, right) = render::pan_gains(track.pan);
            (track_gain * left, track_gain * right)
        } else {
            (track_gain, track_gain)
        };
        for placement in track.clips.values() {
            if placement.muted {
                continue;
            }
            let Some(clip) = project.clip_pool.get(&placement.clip_id) else {
                // 悬空引用由 `render_master` 响亮拒绝（`RENDER_FAILED`）。本工具在
                // **收集**阶段跳过它：响应里的 `notesScheduled` 会如实体现差异，
                // ⛔ 不为导出路径再造第二条错误出口。
                continue;
            };
            let ClipContent::Midi { notes: content } = &clip.content else {
                continue;
            };
            for note in content.values() {
                if !note.triggers(project.rng_seed) {
                    silenced = silenced.saturating_add(1);
                    continue;
                }
                if let Some(only) = only_note
                    && note.pitch != only
                {
                    continue;
                }
                let micro = i64::from(note.micro_timing_ticks.unwrap_or(0));
                let ratchet = u64::from(note.ratchet.unwrap_or(1).clamp(1, 16));
                let step = (note.duration_ticks / ratchet).max(1);
                let base_tick = placement.start_tick.saturating_add(note.start_tick);
                for pulse in 0..ratchet {
                    let (start, end) = math::note_frame_span(
                        base_tick.saturating_add(pulse.saturating_mul(step)),
                        step,
                        micro,
                        PPQ,
                        project.bpm,
                        sample_rate,
                    );
                    let start_frame = u64::try_from(start.max(0)).unwrap_or(0);
                    let end_frame = u64::try_from(end.max(0)).unwrap_or(start_frame);
                    notes.push(SfzNote {
                        start_frame,
                        frames: end_frame.saturating_sub(start_frame).max(1),
                        pitch: note.pitch,
                        velocity: note.velocity,
                        gain_l,
                        gain_r,
                    });
                }
            }
        }
    }
    // 浮点求和不满足结合律 [ARCH-DET-002]：排程必须是**全序**且与输入顺序无关。
    notes.sort_by(|a, b| {
        a.start_frame
            .cmp(&b.start_frame)
            .then(a.frames.cmp(&b.frames))
            .then(a.pitch.cmp(&b.pitch))
            .then(a.velocity.cmp(&b.velocity))
            .then(a.gain_l.to_bits().cmp(&b.gain_l.to_bits()))
            .then(a.gain_r.to_bits().cmp(&b.gain_r.to_bits()))
    });
    (notes, silenced)
}

/// 内存样本源（`sample=` 指向的文件本仓不读 ⇒ 数据由调用方给）。
#[derive(Clone, Debug)]
pub struct InMemorySfzSample {
    /// 单声道 PCM（归一化到 `[-1, 1]`）。
    pub pcm: Vec<f32>,
    /// 原始采样率（Hz）。
    pub sample_hz: f32,
}

impl InMemorySfzSample {
    /// 极短正弦：`frames` 个采样点 @ `sample_hz`。
    ///
    /// 相位由**绝对采样点下标**算出（与 `render.rs` 的 `MidiSynthSource` 同一条口径：
    /// 不累加相位 ⇒ 与块边界无关、逐位可复现）。
    #[must_use]
    pub fn sine(freq_hz: f32, sample_hz: f32, frames: usize) -> Self {
        let pcm = (0..frames)
            .map(|index| {
                let phase = core::f32::consts::TAU * freq_hz * (index as f32) / sample_hz;
                // ⚠ `libm::sinf` 而不是 `phase.sin()`：`ARCH-DET-001` 要求
                // "统一启用纯 Rust libm"，禁止落到宿主 libm —— 否则同一份工程
                // 导出的母带会随平台差 LSB。本 crate 的判据
                // `probe_authenticity::the_crates_own_transcendental_call_sites_are_the_registered_list`
                // 会**当场**把 std 的 `.sin()` 抓出来（本线第一版就是这么红的）。
                libm::sinf(phase) * 0.5
            })
            .collect();
        Self { pcm, sample_hz }
    }
}

/// 一次 SFZ 渲染的读数汇总。
#[derive(Clone, Debug, PartialEq)]
pub struct SfzRenderOutcome {
    /// 交织的立体声浮点母带（`frames * 2` 个样本）。
    pub samples: Vec<f32>,
    /// 帧数。
    pub frames: u64,
    /// 峰值（抖动之前）。
    pub peak: f32,
    /// 排程的音符数。
    pub notes_scheduled: u64,
    /// 区域选择没命中的音符数。
    pub notes_unmatched: u64,
    /// 命中的区域下标（升序）。
    pub regions_used: Vec<usize>,
    /// 落在**要求循环**的区域上的音符数（本实现不展开循环 ⇒ 只响一遍）。
    pub notes_looping: u64,
    /// ⭐ 落在**内建静音**区域（`sample=*silence`）上的音符数（任务单 `#218`）。
    ///
    /// 这些音符 ⛔ 不读盘、⛔ 不回退内存正弦，写入的样本恒为 0。
    pub notes_using_builtin_silence: u64,
    /// 用**文件样本**发声的音符数。
    pub notes_using_file_sample: u64,
    /// 回退到**内存正弦**的音符数。
    pub notes_using_sine_fallback: u64,
    /// 本次渲染**真的读到**的样本文件来源（升序去重）。
    pub sample_sources: Vec<String>,
    /// 读失败的样本文件（升序去重）。
    pub sample_failures: Vec<String>,
    /// 是否命中过读盘上界。
    pub sample_reads_truncated: bool,
    /// 累计解码帧数。
    pub sample_frames_loaded: u64,
}

/// 把工程里的音符经一份**已解析**的 SFZ 渲染成交织立体声浮点母带。
///
/// 语义（每条都刻意可判据化）：
///
/// - **每个音符整段一次**：区域选择 / 速率 / 增益都只算一次，然后在目标帧区间上写一遍；
/// - **块无关**：样本波形由绝对采样点下标算出 ⇒ 同一输入逐位相同；
/// - **首尾淡出**：末尾 [`NOTE_FADE_FRAMES`] 帧线性淡出（去咔哒），淡入不做
///   （SFZ 的起音包络不在本切片内，如实登记）；
/// - 越出输出长度的部分**截断**（不写、不报错；`frames` 是调用方要的时长）。
///
/// # Errors
///
/// `sample_rate == 0` ⇒ `RENDER_FAILED`（除零在这里是明确拒绝，不是钳位）。
pub fn render(
    instrument: &Instrument<'_>,
    notes: &[SfzNote],
    tone_hz: f32,
    frames: u64,
    sample_rate: u32,
    sfz_path: Option<&Path>,
) -> Result<SfzRenderOutcome, Fault> {
    if sample_rate == 0 {
        return Err(Fault::domain(CODE_RENDER, "输出采样率不得为 0"));
    }
    let channels = render::MASTER_CHANNELS;
    let total = usize::try_from(frames)
        .unwrap_or(usize::MAX)
        .saturating_mul(channels);
    let mut samples = vec![0.0_f32; total];
    let engine_hz = sample_rate as f32;
    // 这对采样率只服务**两个**用途：① 区域选择后的 gate / 增益 / 循环
    // （`total_gain()` 与 `loops()` 都不吃 `sample_hz`）；② **内存正弦回退** ——
    // 下面的 `InMemorySfzSample::sine(tone_hz, engine_hz, ..)` 就是在 `engine_hz`
    // 上生成的，所以它的步进比恰好是 `pitch_ratio * engine_hz / engine_hz`。
    // ⛔ **文件样本不走这对**：它有自己的采样率（`SampleFileReading::sample_hz`），
    // 见下面的 `file_rate` —— 拿 `engine_hz` 当 `sample_hz` 会让 44.1 kHz 样本在
    // 48 kHz 输出上快 48/44.1 ≈ 8.8%（≈ +1.47 半音）并提前耗尽。
    let rates = RenderRates::new(engine_hz, engine_hz);
    let mut regions_used: BTreeSet<usize> = BTreeSet::new();
    let mut notes_unmatched = 0u64;
    let mut notes_looping = 0u64;
    let mut notes_using_builtin_silence = 0u64;
    let mut notes_using_file_sample = 0u64;
    let mut notes_using_sine_fallback = 0u64;
    let mut cache = SampleCache::new();
    for note in notes {
        let Some((region_index, region)) =
            instrument.regions().iter().enumerate().find(|(_, region)| {
                region.matches_key(note.pitch) && region.matches_velocity(note.velocity)
            })
        else {
            notes_unmatched = notes_unmatched.saturating_add(1);
            continue;
        };
        regions_used.insert(region_index);
        let rate = region.playback_rate(note.pitch, rates);
        let gain = region
            .playback_spec(note.pitch, note.velocity, rates)
            .total_gain();
        if region
            .playback_spec(note.pitch, note.velocity, rates)
            .loops()
        {
            notes_looping = notes_looping.saturating_add(1);
        }
        if !(rate.is_finite() && rate > 0.0) || gain == 0.0 {
            notes_unmatched = notes_unmatched.saturating_add(1);
            continue;
        }
        // ---- 样本来源：**能读文件就用文件**，否则回退内存正弦（如实记账）----
        //
        // ⚠ 区域选择 / 播放速率 / 总增益**一位不改**：它们仍全部来自 `yeban-sfz`
        // 的公开读数（上面三行）。这里换的**只有样本数据**。
        //
        // ⭐ `#218`：`sample=*silence` 是**内建静音**（⛔ 不是文件）——
        // 判定用 `yeban-sfz` 的 `Region::is_builtin_silence()`（判 `sample=` 原值），
        // 在**任何读盘之前**截住 ⇒ ⛔ 不进 `sample_failures`、⛔ 不回退内存正弦；
        // 下面写零帧（该音符照常排程、照常计入 `regionsUsed`）。
        let builtin_silence = region.is_builtin_silence();
        let file_sample = if builtin_silence {
            None
        } else {
            sfz_path.and_then(|sfz| {
                let raw = region.sample_path();
                if raw.trim().is_empty() {
                    return None;
                }
                let resolved = resolve_sample_path(sfz, &raw);
                // 读不到 ⇒ `None` ⇒ 回退内存正弦（原因已进 cache，渲染完由
                // `sample_failures` 如实报出）。
                cache.get(&resolved).ok()
            })
        };
        let span = usize::try_from(note.frames.max(1)).unwrap_or(0);
        let mut mono = vec![0.0_f32; span];
        let written = if builtin_silence {
            notes_using_builtin_silence = notes_using_builtin_silence.saturating_add(1);
            // `mono` 全是 0.0；写 `span` 帧等价于"这个音符真的响，但样本值为 0"。
            span
        } else if let Some(reading) = &file_sample {
            notes_using_file_sample = notes_using_file_sample.saturating_add(1);
            // ⭐ 文件样本的步进比按**它自己的**采样率换算（`yeban-sfz` 的
            // `playback_rate` = `pitch_ratio * sample_hz / engine_hz`），与
            // `yeban-engine::SfzInstrument::render_note_into` 的
            // `RenderRates::new(source.sample_hz, engine_hz)` **同一口径**。
            let file_rate =
                region.playback_rate(note.pitch, RenderRates::new(reading.sample_hz, engine_hz));
            render_pcm_into(reading.pcm.as_slice(), file_rate, gain, &mut mono)
        } else {
            notes_using_sine_fallback = notes_using_sine_fallback.saturating_add(1);
            // 样本长度 = 这个音符的时值（外加一个淡出尾巴）⇒ 内存正弦在整段时值里都有
            // 数据可播。为什么按音符造：读到样本末尾就停的语义下，用一个固定 1 秒的
            // 样本会让长音符在第 1 秒之后静音（那是**静默的错误音频**）。
            let want = note.frames.saturating_add(NOTE_FADE_FRAMES).max(1);
            let sample_frames = usize::try_from(want.min(8_000_000)).unwrap_or(0).max(2);
            let sample = InMemorySfzSample::sine(tone_hz, engine_hz, sample_frames);
            render_pcm_into(&sample.pcm, rate, gain, &mut mono)
        };
        let start = usize::try_from(note.start_frame).unwrap_or(usize::MAX);
        if start >= samples.len() / channels {
            continue;
        }
        let limit = span.min(written);
        for (offset, sample) in mono.iter().enumerate().take(limit) {
            let frame = start + offset;
            if frame >= frames as usize {
                break;
            }
            // 末尾线性淡出（只在音符自己的时值内，不影响别的音符）。
            let remaining = limit.saturating_sub(offset) as u64;
            let envelope = if remaining <= NOTE_FADE_FRAMES {
                remaining as f32 / NOTE_FADE_FRAMES as f32
            } else {
                1.0
            };
            let value = sample * envelope;
            let base = frame * channels;
            samples[base] += value * note.gain_l;
            samples[base + 1] += value * note.gain_r;
        }
    }
    let peak = math::peak_of(&samples);
    Ok(SfzRenderOutcome {
        samples,
        frames,
        peak,
        notes_scheduled: u64::try_from(notes.len()).unwrap_or(u64::MAX),
        notes_unmatched,
        regions_used: regions_used.into_iter().collect(),
        notes_looping,
        notes_using_builtin_silence,
        notes_using_file_sample,
        notes_using_sine_fallback,
        sample_sources: {
            let mut sources: Vec<String> = cache
                .entries_iter()
                .filter_map(|(_, entry)| {
                    entry.as_ref().ok().map(|reading| reading.describe.clone())
                })
                .collect();
            sources.sort_unstable();
            sources.dedup();
            sources
        },
        sample_failures: {
            let mut failures: Vec<String> = cache
                .entries_iter()
                .filter_map(|(path, entry)| {
                    entry
                        .as_ref()
                        .err()
                        .map(|reason| format!("unreadable({}): {reason}", path.display()))
                })
                .collect();
            failures.sort_unstable();
            failures.dedup();
            failures
        },
        sample_reads_truncated: cache.truncated(),
        sample_frames_loaded: cache.frames_loaded(),
    })
}

/// 把**一个**区域渲染进 `out`（`+=` 叠加），返回实际写入的帧数。
///
/// 与 `yeban-engine` 的 `SfzInstrument::render_note_into` 同一条语义（区域由调用方
/// 选好；线性插值；读到样本末尾就停）。为什么不直接调引擎那一个：那会把
/// `yeban-engine`（默认带 `cpal`）拖进本 crate，而 `yeban-mcp` 的清单注释明文写着
/// "轻量 crate：不拖音频栈进 MCP"。本函数只依赖 `yeban-sfz` 的公开读数。
fn render_pcm_into(pcm: &[f32], rate: f32, gain: f32, out: &mut [f32]) -> usize {
    if pcm.is_empty() {
        return 0;
    }
    let mut position = 0.0f32;
    let mut written = 0usize;
    for slot in out.iter_mut() {
        let index = position as usize;
        if index + 1 >= pcm.len() {
            break;
        }
        let frac = position - index as f32;
        let value = pcm[index] * (1.0 - frac) + pcm[index + 1] * frac;
        *slot += value * gain;
        position += rate;
        written += 1;
    }
    written
}

/// dB → 线性增益（与 `render.rs` 同一个公式与同一个 `libm` 口径）。
fn db_to_linear(db: f32) -> f32 {
    if !db.is_finite() {
        return 0.0;
    }
    if db <= -120.0 {
        return 0.0;
    }
    libm::powf(10.0, db / 20.0)
}
