//! `yeban_load_sfz` 与 `yeban_render_sfz_wav` 的实现：**SFZ 音色真的出声**。
//!
//! ## 为什么有这一条（缺口的实测证据，不是从词面推的）
//!
//! `AGENTS.md` §6.3 要求先 `grep` 交付形态再下结论。本线开工时实测：
//!
//! ```text
//! grep -rn "sfz" crates/yeban-mcp/src/            ⇒ 当时只有 render.rs 的一个 unsupported 键
//! grep -rn "yeban_load_sfz\|yeban_render_sfz_wav" crates/ ⇒ 0 命中
//! ```
//!
//! 上面那个键就是**采样器绑定**，已在 `#221` **退役**（采样器现在真的被渲染）——
//! 现行 `unsupported` 全集以 `domain/render.rs` 的 `KNOWN_UNSUPPORTED_KEYS` 为
//! **唯一事实源**，里面没有它。
//!
//! `yeban_render_master` **能**导 WAV（真走 `yeban-render`，见 `render.rs` 的模块头），
//! 但当时它的 `unsupported` 里**明写**"本模型版本（`DeviceKind`）里**没有** SFZ 设备
//! 变体"⇒ 工具面**没有**任何通路让 SFZ 音色参与导出。本模块补的就是这一条。
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
//!                        力度可被 `velocity` 实参整段覆盖（不给 ⇒ 逐音符用自己的）
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
//!    **有效路径**）指向的样本由 [`read_sample_file`] 解码（`#223` 起与 app 的
//!    `sfz_load::read_sample_file` **同一条分派**：`.wav`/`.wave` 走既有 hound 腿
//!    `yeban_render::wav::read_plain_wav_mono_f32`，其余走 `yeban_decode::decode_mono_channel0`
//!    ＝ symphonia ＋ 声道 0 折叠），经 [`SampleCache`] 去重并受三条上限约束；
//!    响应里的 `sampleSource` / `sampleFilesRead` 逐字反映读了哪些文件。
//!    ⚠ 读不到（或没有区域声明 `sample=`）时，**只有 `yeban_render_sfz_wav` 这条
//!    试听导出路径**会如实回退到 [`InMemorySfzSample::sine`]（内存正弦）并把原因写进
//!    该工具的 `sampleSource`；装载回执里的 `sampleSource` 同样点名这条回退，
//!    但**加上了适用范围**（它不描述绑轨后的母带路径）。
//!    ⭐ `#275`：**何时非 0** 只有两条 —— ① 命中的区域不是 `sample=*silence`；
//!    ② 它的 `sample=` 为空 / 路径读不到 / 解码失败。`notesUsingSineFallback` 就是
//!    这两种情形下的音符数；`*silence` 与未命中区域**都不计**。
//!    ⚠ 采样器轨（`yeban_render_master` 的 `TrackV3.sampler`）**没有**这条回退：
//!    它走引擎 SFZ 支路，缺样本 ⇒ 静音 ＋ `notesWithoutSample`（`#274`/`#275`）。
//!    装载回执的 `silentRegionCount` 因此按**引擎摊平口径**（缺样本 ＋ 内建静音）计数，
//!    与 `yeban_render_master` 的 `sampler.silentRegionCount` 同一个含义（`#G4`）。
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
//!    计入 [`SfzLoad::silent_regions`] / [`SfzLoad::builtin_silent_regions`]、
//!    区域状态逐字 [`BUILTIN_SILENCE_SOURCE`]；
//!    渲染期：该区域**真的写零**（⛔ 不回退内存正弦）。其它 ARIA 内建波形
//!    （`*sine` / `*saw` / `*square` / …）**未实现** —— 仍按普通样本路径处理。
//!
//! 本模块是**离线/控制侧**代码：`parse_text` / `parse_sources` 会分配 ⇒ ⛔ 不得在音频
//! 回调里调用（红线 7 / `MUST-GATE-001`）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use yeban_model::{AssetHash, AutomationTarget, ClipContent, EntityId, PPQ, YebanProjectV1};
use yeban_render::wav::{read_plain_wav_mono_f32, read_plain_wav_mono_f32_from_bytes};
use yeban_sfz::SampleEnd;
use yeban_sfz::instrument::{AMPLITUDE_DEFAULT_PERCENT, CcGate, Instrument, Region, Trigger};
use yeban_sfz::modulation::{CcMod, ModTarget};
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

/// 渲染实参的默认样本频率（Hz）——与 `yeban-app --sfz` 的探测音同一个值。
pub const DEFAULT_TONE_HZ: f32 = 440.0;

/// 样本来源的**如实**文案（⛔ 不是采样文件名）。
pub const SAMPLE_SOURCE: &str = "in-memory-sine";

/// 正弦回退的**适用范围**文案（报告缺口 `G4`）。
///
/// 装载/枚举回执里的 `sampleSource` 会带它：正弦回退只存在于 `yeban_render_sfz_wav`
/// 试听导出路径；绑到采样器轨后 `yeban_render_master` 缺样本即整轨静音 —— 不带这句话
/// 时，`yeban_load_sfz` 的回执会被读成"母带也有声"（Zimmer 2026-10-31 报告的实测：
/// 同一乐器载入回执称回退正弦、渲染音频 −144.53 dBFS）。
pub const SINE_FALLBACK_SCOPE: &str =
    "仅 yeban_render_sfz_wav 试听路径; 采样器轨不回退, 缺样本即整轨静音";

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

/// 一次装载 / 一次渲染的**缺省**文件数上界（去重后**不同**文件数）。
///
/// ⚠ 512 而不是 32／128：真实大库的**一件**乐器就会有上百份不同样本 ——
/// `GM-StylePerc`（220 区域 / 184 份 wav）、`freepats-spanish-classical-guitar`
/// （48 区域在 32 时只读到 26）。截断是**响亮报告**的（`sampleReadsTruncated`），
/// 但"读不全"会让大乐器的**力度层 / 音区层静默缺席** ⇒ 默认值必须容得下真实乐器。
/// 这一条只放行**文件数**，内存的真实上界仍是 [`MAX_SAMPLE_FRAMES_TOTAL`]。
///
/// ⭐ 运行期可用 `yeban_load_sfz.maxSampleFiles` / `yeban_render_master.maxSampleFiles`
/// 覆盖（见 [`SampleLimits`]），⛔ 不必改常量重编。
pub const MAX_SAMPLE_FILES_READ: usize = 512;

/// 一次装载 / 一次渲染的**缺省**帧数上界（所有文件**合计**，按**解码后**的规模算）。
///
/// ⚠ 为什么是 64,000,000 而不是 8,000,000：**八百万帧只够 181 s 单声道**，而真实乐器
/// 一件就要 4–37 M 帧。VSCO-2-CE 的实测解码量（本仓库自带库，逐文件 WAV 头求和）：
///
/// | 乐器 | 帧数 | ≈ 内存 |
/// | :--- | ---: | ---: |
/// | `CelloEnsSpic` | 4.36 M | 17 MB |
/// | `ViolinEnsSusVib` | 11.49 M | 46 MB |
/// | `ContrabassSusVB` / `NV` | 11.99 / 13.20 M | 48 / 53 MB |
/// | `FHornSus` | 12.66 M | 51 MB |
/// | `TromboneSus` | 15.03 M | 60 MB |
/// | `SViolinVib` | 18.59 M | 74 MB |
/// | `GM-StylePerc` | **36.82 M** | 147 MB |
///
/// 旧默认值下装载 `FHornSus` 的读数是 `regions=29 samples=17 silent=12` —— **12 个音区
/// 被预算砍掉，而采样器轨没有正弦回退**，那些音就是数字静音。旧值不是保护，是缺陷：
/// 它等于"稍大一点的乐器就用不了力度分层"。
///
/// 新默认值按"一件最重的真实乐器 × 1.7 余量"取：64 M 帧 ＝ 256 MB 单声道 f32／轨。
/// ⚠ 记账口径是**逐轨**（[`TrackSampleCache`]），一次渲染里已解码的 PCM 由
/// [`SampleStore`] 跨轨共享 ⇒ 峰值内存 ≈ 全部**不同**样本之和（实测本曲 12 件
/// 乐器合计 157.7 M 帧 ≈ 630 MB），⛔ 不是"轨数 × 上界"。
///
/// ⭐ 运行期可用 `maxSampleFrames` 覆盖（见 [`SampleLimits`]）；`0` ＝ 不限。
pub const MAX_SAMPLE_FRAMES_TOTAL: u64 = 64_000_000;

/// 一次装载 / 一次渲染的**样本读取预算**（运行期可配置，⛔ 不是编译期常量）。
///
/// 三条上界里，单文件字节数是**格式护栏**（[`MAX_SAMPLE_FILE_BYTES`]，与预算无关），
/// 其余两条由本类型承载：不同文件数、文件数之外的累计解码帧数。
///
/// ## 优先级
///
/// `resolve(调用方给的文件数, 调用方给的帧数)`：
/// ① 调用方显式给值（`yeban_load_sfz` / `yeban_render_master` 的 `maxSampleFiles` /
///   `maxSampleFrames`）→ ② [`MAX_SAMPLE_FILES_READ`] / [`MAX_SAMPLE_FRAMES_TOTAL`]。
///
/// ⚠ **`0` ＝ 不限**（两条都如此）—— 给"我知道这库有多大，别拦我"留一条正门；
/// 截断仍然是响亮报告的（`sampleReadsTruncated` ＋ 逐条 `sampleFailures`），
/// ⛔ 从来不会静默丢弃。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleLimits {
    /// 不同样本文件数上界（`0` ＝ 不限）。
    pub max_files: usize,
    /// 累计解码帧数上界（`0` ＝ 不限）。
    pub max_frames: u64,
}

impl Default for SampleLimits {
    fn default() -> Self {
        Self {
            max_files: MAX_SAMPLE_FILES_READ,
            max_frames: MAX_SAMPLE_FRAMES_TOTAL,
        }
    }
}

impl SampleLimits {
    /// 用调用方给的值（`None` ＝ 用默认值）解析出一份预算。
    #[must_use]
    pub fn resolve(max_files: Option<usize>, max_frames: Option<u64>) -> Self {
        let defaults = Self::default();
        Self {
            max_files: max_files.unwrap_or(defaults.max_files),
            max_frames: max_frames.unwrap_or(defaults.max_frames),
        }
    }

    /// 文件数是否已到上限（`0` ＝ 不限 ⇒ 恒 `false`）。
    #[must_use]
    pub const fn files_exhausted(self, count: usize) -> bool {
        self.max_files != 0 && count >= self.max_files
    }

    /// 再读 `frames` 帧是否会超上限（`0` ＝ 不限 ⇒ 恒 `false`）。
    #[must_use]
    pub const fn frames_exceeded(self, loaded: u64, frames: u64) -> bool {
        self.max_frames != 0 && loaded.saturating_add(frames) > self.max_frames
    }

    /// 读数的 JSON 形态（响应里回显"这一次用的是哪一条预算"）。
    #[must_use]
    pub fn to_value(self) -> serde_json::Value {
        let limit = |value: u64| {
            if value == 0 {
                serde_json::Value::Null
            } else {
                serde_json::Value::from(value)
            }
        };
        serde_json::json!({
            "maxSampleFiles": limit(self.max_files as u64),
            "maxSampleFrames": limit(self.max_frames),
            "unlimited": self.max_files == 0 && self.max_frames == 0,
        })
    }
}

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

/// 读一份样本文件（先过**单文件字节上限**，再交给共享读取器）。
///
/// ## 格式面（⭐ `#223`：与 `yeban-app` 的 `sfz_load::read_sample_file` **同一条**）
///
/// | 后缀 | 解码腿 | 依据 |
/// | :--- | :--- | :--- |
/// | `wav` / `wave`（大小写不敏感） | [`yeban_render::wav::read_plain_wav_mono_f32`]（hound） | ⭐ **既有 WAV 路径逐位不变**（归一化口径与读数都靠它） |
/// | **其余**（含 `.flac` / `.ogg` / 无后缀） | [`yeban_decode::decode_mono_channel0`]（symphonia） | 与 app 的非 WAV 腿**同一份**实现（`#223` 把它下移到 `yeban-decode`） |
///
/// ⚠ **为什么按后缀而不是按内容嗅探**：与 app 的 `decode_sample_classified` 逐字同一条
/// 理由 —— 内容嗅探会给 `.wav` 增加一次探测，并让"哪个读取器先认领"变成隐含规则；
/// 按后缀分派让"WAV 走老腿、读数逐字不变"这条**可判据的**承诺成立。
///
/// ⚠ **归一化不在本函数里**：WAV 腿的归一化在 `yeban-render`，非 WAV 腿的归一化在
/// `yeban-decode`（symphonia）＋ `decode_mono_channel0` 的声道 0 折叠。
///
/// 本函数只多做两件属于 MCP 侧的事：
/// ① **先看 `metadata.len()`** 再决定读不读（两条腿都会把整份 PCM 读进内存）；
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
    let (pcm, sample_rate, describe) = if is_plain_wav_suffix(path) {
        read_plain_wav_mono_f32(path).map_err(|error| format!("{}: {error}", path.display()))?
    } else {
        // ⭐ `#223`：FLAC／OGG 等由 `yeban-decode` 按**内容**探测容器；折成声道 0 与
        // 描述串的唯一实现都在那里（⛔ 本层不写第二份 `step_by(channels)`）。
        let mono =
            yeban_decode::decode_mono_channel0(path, &yeban_decode::DecodeOptions::default())
                .map_err(|error| format!("{}: {error}", path.display()))?;
        let describe = mono.describe(path);
        (mono.pcm, mono.sample_hz, describe)
    };
    let frames = u64::try_from(pcm.len()).unwrap_or(u64::MAX);
    Ok(SampleFileReading {
        pcm: Arc::new(pcm),
        sample_hz: sample_rate as f32,
        frames,
        describe,
    })
}

/// 后缀是不是**既有 hound WAV 腿**的那一族（`wav` / `wave`，大小写不敏感）。
///
/// ⚠ 只决定**走哪条腿**，⛔ 不决定成功与否（与 app 的 `is_wav_suffix` 逐字同一条判据）。
/// ⛔ 不用内容嗅探：那会给 `.wav` 多一次读盘/探测，见 [`read_sample_file`] 的文档。
#[must_use]
pub(crate) fn is_plain_wav_suffix(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("wav") || extension.eq_ignore_ascii_case("wave")
        })
}

/// `sample=` 的路径解析：绝对路径原样用；相对路径**相对 SFZ 文件所在目录**（SFZ 惯例）。
#[must_use]
pub fn resolve_sample_path(sfz_path: &Path, sample: &str) -> PathBuf {
    // ⭐ ⭐ `#221`（照抄 app 的 `#205` 口径，⛔ 不另立一套）：SFZ 的
    // `opcode=value` 值里**没有转义语法**（解析器只去掉一层成对双引号），
    // 所以 `sample=arco\\c4.wav` 里的 `\` 是作者写的**路径分隔符**，不是转义符。
    // 同一条口径在 `#include` 的沙箱里早已是既定事实（`yeban-sfz` 的
    // `parser.rs` 用 `raw.replace('\\', "/")`）。
    //
    // ⚠ **本单实测到它的后果**：真实库 `dsmolken-double-bass` 的 SFZ 里 `sample=`
    // 全部写成 `arco\\arco_f2_mf_down.wav` 这种 Windows 分隔符 ⇒ 归一之前
    // `SampleCache` 一律读不到、静默回退到内存正弦（读数会如实报
    // `samples=0 missing=["…"]`，但**听见的不是那个音色**）。归一之后
    // 同一份 SFZ 读到 210 份样本（见报告里的实测数字）。
    let normalized = normalize_sample_path(sample);
    let raw = Path::new(normalized.as_ref());
    if raw.is_absolute() || is_windows_drive_absolute(raw) {
        raw.to_path_buf()
    } else {
        sfz_path
            .parent()
            .map_or_else(|| raw.to_path_buf(), |parent| parent.join(raw))
    }
}

/// 把 SFZ 样本路径里的 Windows 分隔符 `\` 归一成 `/`（与 app 的
/// `sfz_load::normalize_sample_path` **同一份语义**、**同一份实现**）。
///
/// **零分配**地借用原字符串；只有真的含 `\` 时才分配一份归一后的副本。
#[must_use]
fn normalize_sample_path(sample: &str) -> std::borrow::Cow<'_, str> {
    if sample.contains('\\') {
        std::borrow::Cow::Owned(sample.replace('\\', "/"))
    } else {
        std::borrow::Cow::Borrowed(sample)
    }
}

/// 归一之后，`C:/…` 形式（盘符 ＋ 分隔符）算**绝对路径** —— 即使宿主是 Unix。
///
/// 只认 `字母:` 后紧跟 `/` 的形式（`C:foo` 是盘符相对路径，不在此列）。
#[must_use]
fn is_windows_drive_absolute(sample: &Path) -> bool {
    sample.to_str().is_some_and(|text| {
        let bytes = text.as_bytes();
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
    })
}

/// 缓存里那张**路径 → 读取结果**表（`BTreeMap` ⇒ 路径字典序 ⇒ 确定性）。
type SampleEntries = BTreeMap<PathBuf, Result<SampleFileReading, String>>;

/// 一次调用里的**样本文件缓存**（去重 + 上界 + 逐条如实记账）。
///
/// 上界三条（任一命中就**停止再读**并如实标注，⛔ 不静默丢弃）：
/// ① 单个文件字节数 > [`MAX_SAMPLE_FILE_BYTES`]；② 不同文件数 > `limits.max_files`；
/// ③ 累计解码帧数 > `limits.max_frames`。②③ 由 [`SampleLimits`] 运行期给出。
#[derive(Debug, Default)]
pub struct SampleCache {
    entries: SampleEntries,
    frames_loaded: u64,
    truncated: bool,
    limits: SampleLimits,
}

impl SampleCache {
    /// 空缓存（用[`SampleLimits::default`]）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 空缓存，指定预算。
    #[must_use]
    pub fn with_limits(limits: SampleLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    /// 取一份样本（命中缓存则**不**重复读盘）。
    ///
    /// # Errors
    ///
    /// 返回**人话原因**：读盘失败、上界命中（含"已截断"）。
    pub fn get(&mut self, path: &Path) -> Result<SampleFileReading, String> {
        let Self {
            entries,
            frames_loaded,
            truncated,
            limits,
        } = self;
        cache_get(
            path,
            entries,
            frames_loaded,
            truncated,
            *limits,
            read_sample_file,
        )
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

/// 读一份样本时的**共同记账**（[`SampleCache`] 与 [`TrackSampleCache`] 共用同一份实现）。
///
/// 记账口径逐条不变：命中 ⇒ 直接返回（**不**计帧、**不**计文件数）；上界命中 ⇒
/// 记一条 `Err` 到本次调用的表里并置 `truncated`；读成功 ⇒ 累加帧数。
fn cache_get(
    path: &Path,
    entries: &mut SampleEntries,
    frames_loaded: &mut u64,
    truncated: &mut bool,
    limits: SampleLimits,
    read: impl FnOnce(&Path) -> Result<SampleFileReading, String>,
) -> Result<SampleFileReading, String> {
    if let Some(hit) = entries.get(path) {
        return hit.clone();
    }
    if limits.files_exhausted(entries.len()) {
        *truncated = true;
        let reason = format!(
            "已有 {} 个不同样本文件, 达到本次调用的上限 {} ⇒ 不再读盘 (⛔ 不静默截断; \
             可用实参 maxSampleFiles 提高, 0 = 不限)",
            entries.len(),
            limits.max_files
        );
        entries.insert(path.to_path_buf(), Err(reason.clone()));
        return Err(reason);
    }
    let outcome = match read(path) {
        Ok(reading) => {
            if limits.frames_exceeded(*frames_loaded, reading.frames) {
                *truncated = true;
                Err(format!(
                    "累计样本帧数会超过本次调用的上限 {} 帧 (已读 {}, 本文件 {} 帧), 拒绝再读 {} \
                     (⛔ 不静默截断; 可用实参 maxSampleFrames 提高, 0 = 不限)",
                    limits.max_frames,
                    *frames_loaded,
                    reading.frames,
                    path.display()
                ))
            } else {
                *frames_loaded = frames_loaded.saturating_add(reading.frames);
                Ok(reading)
            }
        }
        Err(reason) => Err(reason),
    };
    entries.insert(path.to_path_buf(), outcome.clone());
    outcome
}

/// [`load_with`] 的**只读样本视图**：让"装载"既能用一次性缓存（[`SampleCache`]），
/// 也能用一次渲染内的共享池（[`TrackSampleCache`]），而记账口径只有一份。
pub trait SampleReads {
    /// 取一份样本（命中 ⇒ **不**重复读盘）。
    ///
    /// # Errors
    ///
    /// 返回**人话原因**：读盘失败、上界命中（含"已截断"）。
    fn get(&mut self, path: &Path) -> Result<SampleFileReading, String>;
    /// 本次调用已经过的**路径 → 结果**表（装载读数的唯一来源）。
    fn entries(&self) -> &SampleEntries;
    /// 是否命中过任何上界。
    fn truncated(&self) -> bool;
    /// 本次调用累计解码帧数（上界③的记账值）。
    fn frames_loaded(&self) -> u64;
    /// 这次读取用的预算（回显用；⛔ 不从常量再读一遍 —— 那样会在"调用方覆盖过"时说错话）。
    fn limits(&self) -> SampleLimits;
}

impl SampleReads for SampleCache {
    fn get(&mut self, path: &Path) -> Result<SampleFileReading, String> {
        SampleCache::get(self, path)
    }

    fn entries(&self) -> &SampleEntries {
        &self.entries
    }

    fn truncated(&self) -> bool {
        self.truncated
    }

    fn frames_loaded(&self) -> u64 {
        self.frames_loaded
    }

    fn limits(&self) -> SampleLimits {
        self.limits
    }
}

/// ⭐ `#276`：**一次渲染调用内**已解码样本的共享池。
///
/// 同一份样本被**多条轨**引用 ⇒ 只解码一次（`read_sample_file` 只在池里没有时调用）。
/// ⚠ 预算（文件数 / 帧数 / 截断）**不**在池上记账 —— 它由每条轨自己的
/// [`TrackSampleCache`] 按与 [`SampleCache`] **逐条相同**的口径记账，因此
/// `SfzLoad` 的 `sampleSources` / `sampleFailures` / `sampleFramesLoaded` /
/// `sampleReadsTruncated` 与"每条轨各读一遍"时**逐字相同**。
///
/// 生命周期＝一次 `build`（渲染）；渲染结束随作用域释放，⛔ 不跨调用留存。
#[derive(Debug, Default)]
pub struct SampleStore {
    decoded: SampleEntries,
}

impl SampleStore {
    /// 空池。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 池里没有 ⇒ 真读一次并记住（成功与失败都记）；池里有 ⇒ 直接复用。
    fn read(&mut self, path: &Path) -> Result<SampleFileReading, String> {
        if let Some(hit) = self.decoded.get(path) {
            return hit.clone();
        }
        let outcome = read_sample_file(path);
        self.decoded.insert(path.to_path_buf(), outcome.clone());
        outcome
    }

    /// 本次渲染**真的解码成功**的不同文件数（判据读数；⛔ 不是某条轨的装载读数）。
    #[must_use]
    pub fn decoded_files(&self) -> usize {
        self.decoded.values().filter(|entry| entry.is_ok()).count()
    }
}

/// 一条轨在共享池上的**预算视图**（记账口径与 [`SampleCache`] 逐条相同）。
///
/// 存在理由：池负责"同一文件只解码一次"，本类型负责"每条轨仍按自己的上限记账"——
/// 两者分开，才能既省解码、又让每条轨的读数（含截断）与修前**逐字相同**。
#[derive(Debug)]
pub struct TrackSampleCache<'a> {
    store: &'a mut SampleStore,
    entries: SampleEntries,
    frames_loaded: u64,
    truncated: bool,
    limits: SampleLimits,
}

impl<'a> TrackSampleCache<'a> {
    /// 在共享池上开一条轨的预算视图（[`SampleLimits::default`]）。
    #[must_use]
    pub fn new(store: &'a mut SampleStore) -> Self {
        Self::with_limits(store, SampleLimits::default())
    }

    /// 在共享池上开一条轨的预算视图，指定预算（渲染实参透传的那一份）。
    #[must_use]
    pub fn with_limits(store: &'a mut SampleStore, limits: SampleLimits) -> Self {
        Self {
            store,
            entries: SampleEntries::new(),
            frames_loaded: 0,
            truncated: false,
            limits,
        }
    }
}

impl SampleReads for TrackSampleCache<'_> {
    fn get(&mut self, path: &Path) -> Result<SampleFileReading, String> {
        let Self {
            store,
            entries,
            frames_loaded,
            truncated,
            limits,
        } = self;
        cache_get(
            path,
            entries,
            frames_loaded,
            truncated,
            *limits,
            |resolved| store.read(resolved),
        )
    }

    fn entries(&self) -> &SampleEntries {
        &self.entries
    }

    fn truncated(&self) -> bool {
        self.truncated
    }

    fn frames_loaded(&self) -> u64 {
        self.frames_loaded
    }

    fn limits(&self) -> SampleLimits {
        self.limits
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
    /// 或 `unreadable(<path>): <原因>`（没读到 ⇒ ⚠ 只有 `yeban_render_sfz_wav` 试听
    /// 路径回退内存正弦；绑到采样器轨（`yeban_render_master`）则该轨整轨静音）；
    /// ⭐ 内建静音则是 [`BUILTIN_SILENCE_SOURCE`]（没有文件可读）。
    pub sample_file_state: String,
    /// ⭐ CC 门控（`loccN` / `hiccN` 归约后的闭区间，解析产物 [`Region::cc_gates`]
    /// 的原样抄录）。乐器初值状态（`set_ccN` 种子，未声明的 CC = 0）下初值落在
    /// 区间外的区域不会发声，见 [`CcGateCensus`]。
    pub cc_gates: Vec<CcGate>,
    /// ⭐ `amplitude` 基值（线性百分比，缺省 [`AMPLITUDE_DEFAULT_PERCENT`] = 100）。
    pub amplitude: f32,
    /// ⭐ `amplitude` 族的 CC 调制项（[`Region::cc_modulations`] 里
    /// `target == ModTarget::Amplitude` 的子集，原样抄录）。
    pub amplitude_cc_mods: Vec<CcMod>,
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
            "ccGates": self.cc_gates.iter().map(|gate| serde_json::json!({
                "cc": gate.cc,
                "lo": gate.lo,
                "hi": gate.hi,
            })).collect::<Vec<_>>(),
            "amplitude": self.amplitude,
            "amplitudeCcMods": self.amplitude_cc_mods.iter().map(|m| serde_json::json!({
                "cc": m.cc,
                "delta": m.delta,
                "curve": m.curve,
            })).collect::<Vec<_>>(),
        })
    }
}

/// ⭐ **乐器初值状态下的门控普查**（装载期一次算清，⛔ 不是渲染期判定）。
///
/// 口径：每路控制器取乐器初值 —— 文件 `<control>` 里 `set_ccN` 声明了的取
/// 种子值，没声明的取 MIDI 缺省值 `0`。这正是离线渲染探针的种子层
/// （`CcProbe`，路线 a）与 [`DemandLoadFilter::wants`] 并入种子后的可达集
/// 在「无泳道」时的同一份状态：普查说静音的区域，就是离线渲染里（无泳道
/// 干预时）真的不会发声的区域。没有 `set_ccN` 的乐器初值全 `0`，普查结果
/// 与旧「全 0」口径逐条相同。回执键名 `silentAtDefaultCc` 是既有 wire 形状，
/// 沿用不改（其语义自路线 a 起是「乐器初值」）。
/// 一条区域在初值状态下**不可发声**，当且仅当下面两条之一成立：
///
/// - **门控（gate）**：某个 `loccN` / `hiccN` 区间不含该控制器的初值
///   （初值 `< lo` 或 `> hi`）；
/// - **振幅（amplitude）**：`amplitude` 基值 ＋ 各 `amplitude_*ccN` 调制项在
///   初值处的值（`delta × curve(初值 / 127)`，曲线解析不到的项不贡献 —— 求值走
///   `Instrument::amplitude_value_with_cc`）恰为 `0` ⇒ 有效增益 `value / 100 == 0`。
///
/// ⚠ 只写 `amplitude_ccN=100` 而基值缺省 100 的区域**不算**静音：曲线 0 在
/// `x = 0` 处为 `0` ⇒ 有效值仍是 `100`、增益恰 `1.0`（`modulation.rs` §5 的裁决）。
/// ⚠ 交叉淡化（`xfin_*` / `xfout_*`）不在本普查内：那是逐块增益，不是区域门控。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CcGateCensus {
    /// 乐器初值下不可发声的区域总数（门控与振幅的**并集**，同一区域只计一次）。
    pub silent_regions: usize,
    /// 其中因 CC 门控不含初值而静音的区域数。
    pub gate_silent_regions: usize,
    /// 其中因有效振幅为 0 而静音的区域数。
    pub amplitude_silent_regions: usize,
    /// 涉及的 CC 号（升序去重）：静音区域的门控 CC ＋ 振幅调制 CC。
    pub cc_numbers: Vec<u8>,
}

impl CcGateCensus {
    /// 回执形态（`response_data` / `preview` 的 `ccGateCensus` 键）。
    #[must_use]
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "silentAtDefaultCc": self.silent_regions,
            "byGate": self.gate_silent_regions,
            "byAmplitude": self.amplitude_silent_regions,
            "ccNumbers": self.cc_numbers,
        })
    }
}

/// 乐器初值状态下某控制器的值：`set_ccN` 声明了的取种子值，没声明的取 MIDI 缺省 `0`。
///
/// 种子值在解析期已被限定在 `0..=127`（越界是解析错误，见 `yeban-sfz` 的
/// `set_cc_declaration`），因此本读数与引擎摊平的 `CcSeedTable` 逐值相同；
/// 本 crate ⛔ 不依赖 `yeban-engine`（见 `Cargo.toml` 的依赖纪律），故在这里
/// 直接读解析产物，不复用引擎的表类型。
fn initial_cc_value(instrument: &Instrument<'_>, cc: u8) -> u8 {
    instrument.cc_default(u16::from(cc)).unwrap_or(0)
}

/// 一条区域的 `amplitude` 在乐器初值 CC 下的有效值（单位 %）。
///
/// 求值属主是 `yeban-sfz` 的 [`Instrument::amplitude_value_with_cc`]：公式、
/// 曲线解析与「解析不到的项不贡献」都在那一处，本函数只把探针钉成乐器初值
/// （`set_ccN` 种子，未声明的控制器为 `0`），⛔ 不在这里自己求值曲线。
fn amplitude_at_initial_cc(instrument: &Instrument<'_>, region: &Region<'_>) -> f32 {
    instrument
        .amplitude_value_with_cc(region, &|cc| initial_cc_value(instrument, cc))
        .0
}

/// 对一整件乐器做 [`CcGateCensus`] 普查（纯函数：只读解析产物，⛔ 不碰文件系统）。
#[must_use]
pub fn cc_gate_census(instrument: &Instrument<'_>) -> CcGateCensus {
    let mut census = CcGateCensus::default();
    let mut cc_numbers: BTreeSet<u8> = BTreeSet::new();
    for region in instrument.regions() {
        let gate_missed = |gate: &CcGate| {
            let value = initial_cc_value(instrument, gate.cc);
            value < gate.lo || value > gate.hi
        };
        let gate_hit = region.cc_gates.iter().any(gate_missed);
        let amplitude_hit = amplitude_at_initial_cc(instrument, region) == 0.0;
        if gate_hit {
            census.gate_silent_regions += 1;
            for gate in &region.cc_gates {
                if gate_missed(gate) {
                    cc_numbers.insert(gate.cc);
                }
            }
        }
        if amplitude_hit {
            census.amplitude_silent_regions += 1;
            for m in region
                .cc_modulations()
                .iter()
                .filter(|m| m.target == ModTarget::Amplitude)
            {
                cc_numbers.insert(m.cc);
            }
        }
        if gate_hit || amplitude_hit {
            census.silent_regions += 1;
        }
    }
    census.cc_numbers = cc_numbers.into_iter().collect();
    census
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
    /// ⭐ **引擎摊平口径**的静音区域数（缺样本 ＋ 内建静音；任务单 `#218` 起计数、
    /// 报告缺口 `G4` 起与渲染侧对齐）。
    ///
    /// 口径与 app 的 `sfz_load::silent_region_count`、引擎 `PreparedSfz::silent_region_count`
    /// **逐条相同**：一条区域只要**不会产生采样输出**（`sample=*silence`、没声明 `sample=`、
    /// 或声明的文件读不到 / 解码失败）就计入。因此 `yeban_load_sfz` / `yeban_list_instruments`
    /// 的 `silentRegionCount` 与 `yeban_render_master` 的 `sampler.silentRegionCount`
    /// **可以直接比**（报告缺口 `G4` 修前：前者只数内建静音、后者是摊平 ⇒ 同一乐器
    /// 一个报 0、另一个报 360，正是被报告点名的矛盾）。
    ///
    /// ⚠ 正弦回退**不**改变这个计数：它只存在于 `yeban_render_sfz_wav` 试听导出路径，
    /// 采样器轨（`yeban_render_master`）缺样本就是静音（模块头第 1 条）。
    pub silent_regions: usize,
    /// ⭐ 内建静音（`sample=*silence`）的**子集**（任务单 `#218`）。
    ///
    /// 与 app 的 `builtin_silent_region_count` 同口径；`silent_regions` 已包含它。
    pub builtin_silent_regions: usize,
    /// ⭐ P1 按需装载：本次渲染因判定为不会发声而跳过解码的区域数。
    ///
    /// ⛔ 不计入 `silent_regions`：它们不是缺样本，是渲染步按本轨音符/CC 主动不读；
    /// 逐条摘要的 `sample_file_state` 登记为 `skippedDemandLoad`。
    pub skipped_regions: usize,
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
    /// ⭐ 本次装载**用的**预算（回显；`sampleReadsTruncated` 是不是"被这条预算"截的，
    /// 只有把它一起报出来才能判）。
    pub limits: SampleLimits,
    /// ⭐ 乐器初值状态下的门控普查（[`cc_gate_census`]，装载期与摘要同一次解析算出）。
    pub cc_census: CcGateCensus,
}

impl SfzLoad {
    /// 装载的样本来源**汇总文案**（如实：读到就点名文件，读不到就说原因）。
    ///
    /// - 读到了至少一个文件 ⇒ `file(<第一个来源>)`（其余进 `sampleSources` 数组）；
    /// - 一个都没读到 ⇒ [`SAMPLE_SOURCE`]（内存正弦）＋**回退原因**（若有区域声明了 `sample=`）；
    ///   ⭐ **回退带适用范围**：正弦只属于 `yeban_render_sfz_wav` 试听导出路径，
    ///   绑到采样器轨后 `yeban_render_master` **不回退**（报告缺口 `G4`：不带范围时，
    ///   "in-memory-sine (回退: ...)" 会被读成"母带也有声"，而实测是整轨数字静音）；
    /// - ⭐ 只有内建静音区域、既无文件也无失败 ⇒ [`BUILTIN_SILENCE_SOURCE`]
    ///   （⛔ 不说成 `in-memory-sine` —— 那会把"按设计静音"说成"缺文件回退"）。
    #[must_use]
    pub fn sample_source(&self) -> String {
        let silence_note = if self.builtin_silent_regions > 0 {
            format!("; {} 个区域内建静音", self.builtin_silent_regions)
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
            (None, Some(reason)) => {
                format!("{SAMPLE_SOURCE} ({SINE_FALLBACK_SCOPE}) (回退: {reason}){silence_note}")
            }
            (None, None) if self.builtin_silent_regions > 0 => BUILTIN_SILENCE_SOURCE.to_owned(),
            (None, None) => format!("{SAMPLE_SOURCE} ({SINE_FALLBACK_SCOPE})"),
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

    /// ⭐ G4（`[MCP-TOOL-EXT-SFZ-LOAD]`）：样本读不到时的**非阻塞警告**。
    ///
    /// `loaded: true` 说的是"SFZ 文本解析成功、registry 注册成功"，**不**蕴含
    /// "有声音"：仓库采样是登记式（`assets/samples/manifest.json` 的 `policy`，
    /// 9.371 GiB 不随包分发），按 `instruments[].repo` + `pin` 外取后才会读到文件。
    /// 一个样本文件都读不到时，`status` 仍是 `success` —— 若没有这条警告，
    /// 调用方就只能从 `sampleFailures` 数组里自己数出"整轨绝对静音"。
    /// 因此口径与 `domain/render.rs` 的 `silent_sources_warning` 同型：
    /// 点名数量、给例子、说清后果（采样器轨**没有**正弦回退，见模块头第 1 条 ⚠）、
    /// ⛔ 不自动修、⛔ 不阻断。
    #[must_use]
    pub fn samples_warning(&self) -> Option<serde_json::Value> {
        if self.sample_failures.is_empty() {
            return None;
        }
        let all_unreadable = self.sample_sources.is_empty();
        Some(serde_json::json!({
            "code": "sfzSamplesUnreadable",
            "severity": "warning",
            "blocking": false,
            "count": self.sample_failures.len(),
            "allSamplesUnreadable": all_unreadable,
            "examples": self.sample_failures.iter().take(5).cloned().collect::<Vec<_>>(),
            "hint": format!(
                "SFZ 定义解析并注册成功（`loaded:true` 说的只是这件事），但 {} 个样本文件读不到{}\
                 。把这件乐器绑到采样器轨（`setTrackSampler`）后 `yeban_render_master` 会**整轨数字静音**\
                 —— 采样器路径没有正弦回退（`yeban_render_sfz_wav` 的回退只覆盖那条导出路径）。\
                 官方仓库按登记式策略**不随包分发**采样字节：请照 `assets/samples/manifest.json` 里\
                 `instruments[].repo` + `pin`（或 `archive.url`）自行拉取，并按每条 `sha256` 校验后\
                 放到 SFZ `sample=` 相对路径解析得到的位置。\
                 ⛔ 本工具不下载、不代生成采样，音频一个字节都没造。",
                self.sample_failures.len(),
                if all_unreadable { "（全部 —— 绑轨渲染即绝对静音）" } else { "" },
            ),
        }))
    }

    /// ⭐ 乐器初值下有区域不发声时的**非阻塞警告**（与 [`Self::samples_warning`] 同型）。
    ///
    /// `loaded: true` 只说解析与注册成功；分层选奏法的库（`loccN` 分层）在初值层
    /// 之外的区域**本来就不发声** —— 这不是错误，但调用方若不知道，就会把
    /// "没声"误诊成装载失败。因此普查非 0 时点名：数量、原因分计、涉及的 CC 号，
    /// 以及怎么让它发声（注入 `MidiCc` 泳道；泳道值覆于 `set_ccN` 种子之上）。
    /// ⛔ 不阻断、⛔ 不自动改。
    #[must_use]
    pub fn cc_gate_warning(&self) -> Option<serde_json::Value> {
        if self.cc_census.silent_regions == 0 {
            return None;
        }
        Some(serde_json::json!({
            "code": "sfzCcGatedSilence",
            "severity": "warning",
            "blocking": false,
            "count": self.cc_census.silent_regions,
            "byGate": self.cc_census.gate_silent_regions,
            "byAmplitude": self.cc_census.amplitude_silent_regions,
            "ccNumbers": self.cc_census.cc_numbers,
            "hint": format!(
                "该乐器有 {} 个区域在乐器初值下不发声（门控 {} 个、振幅 {} 个；涉及 CC {:?}）\
                 ：这些区域要求控制器离开初值（`<control> set_ccN` 种子，未声明者为 0）才发声。\
                 需要 CC 校准 —— 给工程加对应的 `MidiCc` 自动化泳道（泳道值覆于种子之上）。\
                 ⛔ 这不是装载失败：解析与注册已成功，本警告不阻断任何操作。",
                self.cc_census.silent_regions,
                self.cc_census.gate_silent_regions,
                self.cc_census.amplitude_silent_regions,
                self.cc_census.cc_numbers,
            ),
        }))
    }

    /// 本次装载的全部非阻塞警告（样本读不到 ＋ CC 门控普查，顺序固定）。
    #[must_use]
    pub fn all_warnings(&self) -> Vec<serde_json::Value> {
        let mut warnings: Vec<serde_json::Value> = Vec::new();
        if let Some(warning) = self.samples_warning() {
            warnings.push(warning);
        }
        if let Some(warning) = self.cc_gate_warning() {
            warnings.push(warning);
        }
        warnings
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
            "silentRegionCount": self.silent_regions,
            "builtinSilentRegionCount": self.builtin_silent_regions,
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
            "sampleLimits": self.limits.to_value(),
            "sampleHz": self.sample_hz,
            "ccGateCensus": self.cc_census.to_value(),
            "warnings": self.all_warnings(),
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
            "silentRegionCount": self.silent_regions,
            "builtinSilentRegionCount": self.builtin_silent_regions,
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
            "sampleLimits": self.limits.to_value(),
            "sampleHz": self.sample_hz,
            "ccGateCensus": self.cc_census.to_value(),
            "warnings": self.all_warnings(),
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

/// 解析 `maxSampleFiles` / `maxSampleFrames` 两个预算实参（两个工具共用同一份口径）。
///
/// 形状规则（与工具面其余整数实参同一条纪律）：
/// - 缺键 / `null` ⇒ `None` ⇒ 用[`SampleLimits::default`]；
/// - 整数 `>= 0`（`0` ＝ **不限**）⇒ 覆盖；
/// - 负数 / 非整数 / 布尔 / 字符串 ⇒ `INVALID_PARAMETER_RANGE`（⛔ 不静默钳制）。
///
/// # Errors
///
/// 两个键任一不是非负整数 ⇒ `INVALID_PARAMETER_RANGE`（带 `field` 与 `received`）。
pub fn parse_sample_limits(
    arguments: &serde_json::Map<String, serde_json::Value>,
) -> Result<SampleLimits, Fault> {
    let read = |field: &'static str| -> Result<Option<u64>, Fault> {
        match arguments.get(field) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::Number(number)) => number.as_u64().map(Some).ok_or_else(|| {
                Fault::domain_with_data(
                    CODE_RANGE,
                    format!("`{field}` 必须是非负整数 (0 = 不限), 实际收到 {number}"),
                    serde_json::json!({ "field": field, "received": number.to_string() }),
                )
            }),
            Some(other) => Err(Fault::domain_with_data(
                CODE_RANGE,
                format!("`{field}` 必须是非负整数 (0 = 不限), 实际收到 {other}"),
                serde_json::json!({ "field": field, "received": other.to_string() }),
            )),
        }
    };
    let max_files = read("maxSampleFiles")?.map(|value| {
        // `usize` 在 32 位宿主上比 u64 窄 ⇒ 用 try_from 而不是 as（⛔ 不静默截断）。
        usize::try_from(value).unwrap_or(usize::MAX)
    });
    let max_frames = read("maxSampleFrames")?;
    Ok(SampleLimits::resolve(max_files, max_frames))
}

/// 解析 `yeban_load_sfz` 的实参（**只做形状校验**，读文件在 [`load`] 里）。
///
/// # 相对路径（⭐ 补丁 `P1`）
///
/// `path` 为**相对路径**时，按 [`super::render::resolve_sampler_path`] 的**四档**解析
/// （工程目录 ⇒ 采样库根 ⇒ 仓库根 ⇒ cwd），与 `yeban_render_master` 说同一句话。
/// 修前这里只 `PathBuf::from(path)`，随后直接 `fs::metadata` —— 于是"哪条路径算数"
/// 在工具面有**两套**答案：装载要绝对、渲染要相对；而
/// `docs/SFZ_SAMPLE_ACQUISITION.md` 教的正是在工程里存 `assets/samples/<乐器>/…`
/// 这种相对形态。
///
/// 三条纪律：
///
/// - **绝对路径⛔ 不过四档**：原样返回，前序调用逐字节不变（严格扩张）；
/// - **四档都不中⛔ 不报错**：仍返回原样路径，由 [`load`] / [`plan_load_sfz`] 以既有
///   的 `sfzReadFailed` 报告 —— 错误码与 `data.reason` 的形状不变，只是 `path`
///   现在可能已被解析成绝对路径；
/// - `project_dir` 为 `None`（没打开工程）⇒ 少掉"工程目录"一档，其余三档照试。
///
/// # Errors
///
/// 缺 `path` / 空 `path` / `sampleHz` 非有限正数 ⇒ `INVALID_PARAMETER_RANGE`。
pub fn parse_load_arguments(
    arguments: &serde_json::Map<String, serde_json::Value>,
    project_dir: Option<&std::path::Path>,
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
    Ok((resolve_load_path(path, project_dir), sample_hz))
}

/// `path` → **真的存在**的那份文件；解析不出（四档都不中）⇒ 原样返回。
///
/// 与 [`super::render::resolve_sampler_path`] 是**同一个函数**（⛔ 不复制候选表）：
/// 后者在绝对路径 / 四档都不中时返回 `Fault`，这里把它翻成"原样"，因为装载失败
/// 的报告职责在 [`load`]，本函数只管"能不能先换成一份找得到的路径"。
fn resolve_load_path(raw: &str, project_dir: Option<&std::path::Path>) -> PathBuf {
    // ⚠ `resolve_sampler_path` 收的是**工程文件**的路径、内部取 `parent()` 当"工程目录"。
    // 所以这里要**补一个文件名**再交过去 —— 直接传目录会再被削一层（实测：传
    // `/x/projects` ⇒ 候选变成 `/x/assets/samples/…`，工程目录那一档整个失效）。
    // 没有活跃工程时给一个不存在的相对名 ⇒ `parent()` 为空 ⇒ 跳过该档。
    let anchor = match project_dir {
        Some(dir) => dir.join("__active_project__.yeban"),
        None => PathBuf::from("__no_project__"),
    };
    match super::render::resolve_sampler_path(raw, &anchor) {
        Ok(resolved) => resolved,
        Err(_) => PathBuf::from(raw),
    }
}

/// 装载一份 SFZ：读文件 ⇒ SHA-256 ⇒ 解析 ⇒ 区域摘要。
///
/// **只读**：不写盘、不改工程。`assign_id` 由调用方（`Domain`）按已有的 registry 决定。
/// `limits` 是这一次装载的样本读取预算（[`SampleLimits`]；实参
/// `maxSampleFiles` / `maxSampleFrames` 由调用方解析后传进来）。
///
/// # Errors
///
/// - `IO_ERROR`：文件不存在 / 是目录 / 读失败；
/// - `INVALID_PARAMETER_RANGE`：非 UTF-8 文本，或 SFZ 文本被解析器拒绝。
pub fn load(
    path: &Path,
    sample_hz: f32,
    assign_id: u64,
    reused: bool,
    limits: SampleLimits,
) -> Result<SfzLoad, Fault> {
    let mut cache = SampleCache::with_limits(limits);
    load_with(path, sample_hz, assign_id, reused, &mut cache, None).map(|(load, _)| load)
}

/// 提供表的一行：`sample=` 的**原样字符串** ＋ 该文件的读取结果（读不到 ⇒ `Err`）。
///
/// ⭐ `#276`：这张表由 [`load_with`] 在同一次解析里产出，供引擎摊平使用。
pub type SampledProviderEntry = (String, Result<SampleFileReading, String>);

/// 引擎摊平用的**提供表**（按区域首见顺序、按 `sample=` 原样字符串去重）。
pub type SampledProviderTable = Vec<SampledProviderEntry>;

/// [`load`] 的**共享样本池 ＋ 提供表**版本（⭐ `#276`）。
///
/// ⭐ P1 按需装载：渲染步的命中区域判定器。
///
/// 只供渲染路径（[`load_with`] 的 `wanted` 参数）使用：按本轨的音符与 CC 状态，
/// 判断一条 SFZ 区域在本次离线渲染里会不会发声。判定**逐条复刻引擎自己的选择语义**
/// （`SfzVoicePool::trigger_note` ＋ `flatten_regions` 的摊平门控），⛔ 不是按音高估算：
///
/// - **通道**：采样器路径不传通道，摊平固定通道 `1` ⇒ 区域必须含通道 `1`。
/// - **键位切换**（摊平期 `last=None`、`down≡false` 时的 `keyswitch_ok`）：`sw_down` ⇒ 永假；
///   `sw_last` 有值且与 `sw_default` 不等（含无缺省）⇒ 假；`sw_up` 不挡；无门控 ⇒ 过。
/// - **CC 门控**：离线探针来自本轨 `MidiCc` 泳道 ＋ 宏级联 ＋ 本乐器自带的 `set_ccN`
///   种子（`CcProbe` 的种子层，路线 a —— 推翻 `#112` 的「种子只进实时会话」旧口径，
///   Wang 2026-11-14 裁决）。种子在本构造器之外并入（[`Self::with_cc_seeds`]，
///   装载点拿到解析产物时调用；本构造器不读文件），优先级与探针组表一致：
///   **有有效泳道／级联的控制器以泳道可达集为准，种子不并入；没有泳道的控制器
///   以种子值 `(v, v)` 为可达集；两者都没有的控制器恒 `(0, 0)`**。
///   **没有条目不等于控制器不可达**：MIDI CC 的缺省值是 `0`，所以无条目时每一路
///   控制器都按 `(0, 0)` 参与门控判定 —— 含 `0` 的区间（例如 `hicc105=63`、
///   `locc1=0 hicc1=127`）照常发声，不含 `0` 的区间（例如 `locc105=64`）在没有
///   种子与泳道时不发声。有泳道的控制器取值落在「曲线值域 ＋ 宏偏移值域」内
///   （`value_at` 首末保持、段内插值 ⇒ 值恒在点值域内；宏偏移 ＝ 各路
///   `depth × 宏曲线值` 之和）。CC 门控一控制器一门 ⇒ 逐门区间交集判定是精确的
///   （各控制器取值独立选取）。
///
///   ⚠ 修前这一条写的是「无有效探针 ⇒ 门控区域一律不发声」。实测后果：`locc/hicc`
///   分层选奏法的库（`cithara-barbarica` 的 `locc105=64`/`hicc105=63`、
///   `aliexpress-erhu`、`ixox-flute` 的 `locc$F1_ART=0 hicc$F1_ART=42`）在没有 CC 泳道
///   的工程里被**整件判死** —— 渲染回执是 `samples=0`、`silent=全部区域`、
///   `missing=[全部样本]`，而同一个 sfz 走 `yeban_load_sfz` 却逐条 `file(... frames=…)`。
///   这就是「编辑器里能试听、渲染出来整轨没声」的根因。
/// - **音符匹配**：键/力度范围（`trigger_by_note == false` 即 `key=-1` ⇒ 永不命中）＋ trigger 族。
///   `Attack`/`First`/`Legato` 走 note-on 掩码；`First` 要求 `held == 0`（轨内首个音符起音时
///   恒成立 ⇒ 只要有匹配音符就计入，多算安全）；`Legato` 要求 `held > 0`（至少两个音符
///   才可能 ⇒ 音符总数 `< 2` 时排除，多算安全）；`Release`/`ReleaseKey` 族当前引擎无
///   note-off 样本触发入口，但按任务要求保守计入（匹配音符即命中，多算安全）。
/// - **超集安全**：静音摆放、概率音符、循环重复、超 `MAX_NOTES_PER_TRACK` 丢弃的音符
///   一律照常计入 —— 它们至多让过滤器多读文件，⛔ 绝不会让可发声区域被漏掉。
///   轮询变体（`seq_*`）不在区域判定里：同一音高的全部变体按同一音符命中 ⇒ 全计入。
///
/// 构造只读工程模型（音符、泳道、宏映射），⛔ 不碰文件系统、不解码。
#[derive(Clone)]
pub struct DemandLoadFilter {
    /// 本轨全部 MIDI 音符的 `(pitch, velocity)`（含静音摆放的音符，见超集安全）。
    notes: Vec<(u8, u8)>,
    /// 本轨音符总数（`Legato` 门控用）。
    total_notes: usize,
    /// 每路控制器的离线探针可达整数区间 `[lo, hi]`（已 clamp 到 `0..=127`）。
    /// 无条目的控制器恒 `0`（与「没有探针」同一条缺省，见 `wants` 的 CC 分支）。
    /// 经 [`Self::with_cc_seeds`] 并入种子后，无泳道控制器的条目是种子值 `(v, v)`。
    cc_reachable: BTreeMap<u8, (u8, u8)>,
}

impl DemandLoadFilter {
    /// 从真实工程与本轨身份构造判定器。
    #[must_use]
    pub fn from_project_track(project: &YebanProjectV1, track: EntityId) -> Self {
        let mut notes = Vec::new();
        if let Some(track_v3) = project.tracks.get(&track) {
            for placement in track_v3.clips.values() {
                if let Some(entry) = project.clip_pool.get(&placement.clip_id)
                    && let ClipContent::Midi { notes: midi_notes } = &entry.content
                {
                    for note in midi_notes.values() {
                        notes.push((note.pitch, note.velocity));
                    }
                }
            }
        }
        let total_notes = notes.len();
        // 本轨 MidiCc 泳道的曲线值域（`read_enabled` 且有点 ⇒ 有效；否则不算曲线）。
        let mut curve_range: BTreeMap<u8, (f32, f32)> = BTreeMap::new();
        // 宏级联到本轨 MidiCc 目标的偏移值域（`depth × 宏曲线值` 之和；逐控制器累加）。
        let mut cascade_range: BTreeMap<u8, (f32, f32)> = BTreeMap::new();
        // 宏级联是否非零（`CcProbe::is_zero` 口径：全部点都 `0.0` ⇒ 级联为零）。
        let mut cascade_nonzero: BTreeSet<u8> = BTreeSet::new();
        if let Some(track_v3) = project.tracks.get(&track) {
            for (target, lane) in &track_v3.automation_lanes {
                if let AutomationTarget::MidiCc {
                    track_id,
                    controller,
                } = target
                    && *track_id == track
                    && *controller <= 127
                    && lane.read_enabled
                    && !lane.points.is_empty()
                {
                    let mut lo = f32::MAX;
                    let mut hi = f32::MIN;
                    for point in lane.points.values() {
                        lo = lo.min(point.value);
                        hi = hi.max(point.value);
                    }
                    curve_range.insert(*controller, (lo, hi));
                }
            }
        }
        // 宏级联扫描与 `MacroCascade::for_target` 同口径：全部音轨的 `Macro` 泳道
        // （`read_enabled` 且有点）→ 所属宏参数的映射 → 目标是本轨 MidiCc 且 `depth != 0`。
        for track_v3 in project.tracks.values() {
            for (lane_target, lane) in &track_v3.automation_lanes {
                let AutomationTarget::Macro {
                    track_id: macro_track_id,
                    macro_index,
                } = lane_target
                else {
                    continue;
                };
                if !lane.read_enabled || lane.points.is_empty() {
                    continue;
                }
                let Some(macro_track) = project.tracks.get(macro_track_id) else {
                    continue;
                };
                let Some(macro_param) = macro_track.macros.get(*macro_index) else {
                    continue;
                };
                let mut lo = f32::MAX;
                let mut hi = f32::MIN;
                let mut any_nonzero = false;
                for point in lane.points.values() {
                    lo = lo.min(point.value);
                    hi = hi.max(point.value);
                    any_nonzero |= point.value != 0.0;
                }
                for mapping in &macro_param.mappings {
                    if let AutomationTarget::MidiCc {
                        track_id,
                        controller,
                    } = mapping.target
                        && track_id == track
                        && controller <= 127
                        && mapping.depth != 0.0
                    {
                        let (off_lo, off_hi) =
                            cascade_range.entry(controller).or_insert((0.0, 0.0));
                        let a = mapping.depth * lo;
                        let b = mapping.depth * hi;
                        *off_lo += a.min(b);
                        *off_hi += a.max(b);
                        if any_nonzero {
                            cascade_nonzero.insert(controller);
                        }
                    }
                }
            }
        }
        let mut cc_reachable: BTreeMap<u8, (u8, u8)> = BTreeMap::new();
        let controllers: BTreeSet<u8> = curve_range
            .keys()
            .copied()
            .chain(cascade_range.keys().copied())
            .collect();
        for controller in controllers {
            let has_curve = curve_range.contains_key(&controller);
            let has_cascade = cascade_nonzero.contains(&controller);
            // `CcProbe::new` 的取舍：曲线 present 或级联非零 ⇒ 该控制器进探针；
            // 否则该控制器不在探针里（探针存在时恒 `0`，见 `wants` 的缺省分支）。
            if !has_curve && !has_cascade {
                continue;
            }
            let (mut raw_lo, mut raw_hi) = if has_curve {
                curve_range[&controller]
            } else {
                (0.0, 0.0)
            };
            if has_cascade {
                let (off_lo, off_hi) = cascade_range[&controller];
                raw_lo += off_lo;
                raw_hi += off_hi;
            }
            let lo = raw_lo.clamp(0.0, 127.0).round() as u8;
            let hi = raw_hi.clamp(0.0, 127.0).round() as u8;
            cc_reachable.insert(controller, (lo, hi));
        }
        Self {
            notes,
            total_notes,
            cc_reachable,
        }
    }

    /// 并入一份乐器自带的 `set_ccN` 种子表（`Instrument::cc_defaults()`），返回新判定器。
    ///
    /// 并集口径与 `CcProbe` 的组表**逐条一致**（泳道赢）：`cc_reachable` 已有条目
    /// （＝该控制器有有效泳道／宏级联）的控制器**不动**；没有条目的控制器若有
    /// 种子 `(cc, v)`（`cc ≤ 127`；域外声明不进 CC 域，与 `CcSeedTable` 同口径），
    /// 插入 `(v, v)` —— 探针在该控制器上恒读种子值，可达集就是单点。
    /// 空种子表 ⇒ 返回的判定器与 `self` 逐字段相同（无 `set_ccN` 的工程装载
    /// 行为与接线前一致）。
    #[must_use]
    pub fn with_cc_seeds(&self, defaults: &BTreeMap<u16, u8>) -> Self {
        let mut next = self.clone();
        for (cc, value) in defaults {
            // CC 域只有 0..=127：域外声明（`set_cc200` 等）不进可达集，
            // 与引擎侧 `CcSeedTable` 的「只计数、不进表」同口径。
            if *cc <= 127 {
                let cc = u8::try_from(*cc).expect("≤127 必能进 u8");
                next.cc_reachable.entry(cc).or_insert((*value, *value));
            }
        }
        next
    }

    /// 该区域在本次离线渲染里会不会发声（见结构体文档的语义逐条）。
    #[must_use]
    pub fn wants(&self, region: &Region<'_>) -> bool {
        // 通道：摊平固定通道 1。
        if region.lochan > 1 || region.hichan < 1 {
            return false;
        }
        // 键位切换摊平规则（`keyswitch_ok(None, |_| false)`）。
        if region.sw_down.is_some() {
            return false;
        }
        if let Some(expected) = region.sw_last
            && region.sw_default != Some(expected)
        {
            return false;
        }
        // CC 门控。⚠ 无条目（无泳道、无种子）时**不是**"一律不发声"：MIDI CC 的缺省值是 0，
        // `locc/hicc` 的常见写法正是「0..63 一层、64..127 另一层」，缺省 0 落在第一层。
        // 修前这里 `if !probe_exists { return false }` 把整件库判死（见结构体文档）。
        for gate in &region.cc_gates {
            let (lo, hi) = self.cc_reachable.get(&gate.cc).copied().unwrap_or((0, 0));
            if hi < gate.lo || lo > gate.hi {
                return false;
            }
        }
        // `key=-1` 的区域不被任何音符触发。
        if !region.trigger_by_note {
            return false;
        }
        let key_vel_hit = self.notes.iter().any(|&(pitch, velocity)| {
            i32::from(pitch) >= region.lokey
                && i32::from(pitch) <= region.hikey
                && velocity >= region.lovel
                && velocity <= region.hivel
        });
        if !key_vel_hit {
            return false;
        }
        match region.trigger {
            Trigger::Attack | Trigger::First | Trigger::Release | Trigger::ReleaseKey => true,
            // `Legato` 要求起音时已有别的音符按住 ⇒ 至少两个音符才可能。
            Trigger::Legato => self.total_notes >= 2,
        }
    }
}

/// 与 [`load`] 在**同一次解析**里多产出一件东西：引擎摊平要的那张
/// `(sample= 的**原样字符串**, 读取结果)` 表 —— 按区域首见顺序、按原样字符串去重，
/// 跳过内建静音（`*silence`）与空 `sample=`。于是渲染路径
/// ⛔ 不必为了这张表再解析一遍（修前 `prepare_sampler_track` 会再开一次
/// `with_instrument`），也 ⛔ 不必再解码一遍（修前每次渲染自建一个 [`SampleCache`]）。
///
/// 读盘与预算仍由 `cache`（[`SampleReads`]）记账：传 [`SampleCache`] ⇒ 与 [`load`]
/// **完全同一条**；传 [`TrackSampleCache`] ⇒ 多轨共享已解码 PCM，而每条轨的记账
/// （文件数 / 帧数 / 截断）逐条不变。
///
/// # Errors
///
/// 与 [`load`] 相同。
pub fn load_with(
    path: &Path,
    sample_hz: f32,
    assign_id: u64,
    reused: bool,
    cache: &mut dyn SampleReads,
    wanted: Option<&DemandLoadFilter>,
) -> Result<(SfzLoad, SampledProviderTable), Fault> {
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
    load_with_resolved(
        text,
        metadata.len(),
        digest,
        path,
        resolution,
        sample_hz,
        assign_id,
        reused,
        cache,
        wanted,
    )
}

/// [`load_with`] 与 [`load_with_embedded`] 共用的装载体：SFZ 文本已在手里，
/// 只差 `#include` 展开结果与样本来源（盘上缓存 / 内嵌 CAS 缓存）。
///
/// `sfz_path` 的双重身份：① `#include` 基准目录与 `sample=` 解析的锚点
/// （盘上腿 = 磁盘路径；内嵌腿 = 绑定路径原样 —— `resolve_sample_path`
/// 拼出的就是内嵌映射表的键）；② `SfzLoad.path` 的回显值。
/// 内嵌腿调用方必须保证 `resolution` 是 `None`（无 `#include`，见
/// [`load_with_embedded`]），⛔ 不能把绑定路径交给磁盘解析器去找 include 文件。
#[allow(clippy::too_many_arguments)]
fn load_with_resolved(
    text: String,
    byte_len: u64,
    digest: String,
    sfz_path: &Path,
    resolution: Option<ResolvedIncludeSources>,
    sample_hz: f32,
    assign_id: u64,
    reused: bool,
    cache: &mut dyn SampleReads,
    wanted: Option<&DemandLoadFilter>,
) -> Result<(SfzLoad, SampledProviderTable), Fault> {
    // ---- 逐区域**真读** `sample=<path>`（有界；读不到如实回退，⛔ 不让装载失败）----
    // ⭐ `#218`：`sample=*silence` **⛔ 不读盘**（它不是文件）—— 区域状态记内建静音，
    // ⛔ 不进 `sampleFailures`。判定只有一份：`yeban-sfz` 的 `Region::is_builtin_silence()`。
    // ⭐ 报告缺口 `G4`：静音计数按**引擎摊平口径**（缺样本 ＋ 内建静音），与
    // `yeban_render_master` 的 `sampler.silentRegionCount` 同一个含义 —— 修前这里只数
    // 内建静音，同一份"40 个样本全读不到"的乐器会被报成 0，而渲染侧报 360。
    let (
        regions,
        silent_regions,
        builtin_silent_regions,
        skipped_regions,
        sample_sources,
        sample_failures,
        sample_reads_truncated,
        sample_frames_loaded,
        provider,
        cc_census,
    ) = {
        let instrument = match resolution.as_ref() {
            Some((sources, _)) => parse_expanded(&text, Some(sources.as_slice())),
            None => parse_expanded(&text, None),
        }?;
        // ⭐ set_ccN 离线播种（路线 a）：需求过滤器的 CC 可达集在这里并入本乐器的
        // 种子表 —— 与探针的种子层同一次解析、同一个来源（`cc_defaults()`），
        // 装载侧与渲染侧自此读同一份初值（泳道赢的优先级在 `with_cc_seeds` 里）。
        let wanted_owned;
        let wanted = match wanted {
            Some(filter) => {
                wanted_owned = filter.with_cc_seeds(instrument.cc_defaults());
                Some(&wanted_owned)
            }
            None => None,
        };
        // ⭐ CC 门控普查与摘要在**同一次解析**里算（⛔ 不为普查再解析一遍）。
        let cc_census = cc_gate_census(&instrument);
        let mut regions: Vec<RegionSummary> = Vec::new();
        // ⭐ `#276`：提供表在**同一次**区域遍历里产出（⛔ 第二遍解析的替代品）。
        let mut provider: SampledProviderTable = Vec::new();
        let mut provider_seen: BTreeSet<String> = BTreeSet::new();
        let mut silent_regions = 0usize;
        let mut builtin_silent_regions = 0usize;
        let mut skipped_regions = 0usize;
        for (index, region) in instrument.regions().iter().enumerate() {
            let mut summary = summarize(index, region);
            // ⭐ P1 按需装载：判定器说本区域在本次渲染里不会发声 ⇒ 跳过解码。
            // 不进提供表（引擎摊平按路径查表 ⇒ 缺项即 `NO_SAMPLE`，即静音）、
            // 不占预算、不计静音（它不是缺样本，是主动不读）。
            if let Some(filter) = wanted
                && !filter.wants(region)
            {
                summary.sample_file_state = "skippedDemandLoad".to_owned();
                skipped_regions += 1;
                regions.push(summary);
                continue;
            }
            // 本条区域会不会产生采样输出（引擎摊平口径）：内建静音 / 没声明 `sample=` /
            // 声明的文件读不到 ⇒ 静音；读到了 ⇒ 有声。
            let silent = if summary.builtin_silence {
                builtin_silent_regions += 1;
                summary.sample_file_state = BUILTIN_SILENCE_SOURCE.to_owned();
                true
            } else {
                // ⚠ 读盘用**有效路径**（`sample_path()` 会拼 `default_path=`）；
                // 内建静音的判定用**原值**（上面 `builtin_silence`，`#217` 的裁决）。
                let sample = region.sample_path();
                if sample.trim().is_empty() {
                    summary.sample_file_state = "noSampleOpcode".to_owned();
                    true
                } else {
                    let resolved = resolve_sample_path(sfz_path, &sample);
                    let outcome = cache.get(&resolved);
                    // ⭐ 提供表的键是 `sample_path()` 的**原样字符串**（含 `default_path=`
                    // 前缀、⛔ 不规范化）—— 与引擎摊平的去重键同一句。
                    let raw_path = sample.into_owned();
                    if provider_seen.insert(raw_path.clone()) {
                        provider.push((raw_path, outcome.clone()));
                    }
                    match outcome {
                        Ok(reading) => {
                            summary.sample_file_state = reading.describe;
                            false
                        }
                        Err(reason) => {
                            summary.sample_file_state =
                                format!("unreadable({}): {reason}", resolved.display());
                            true
                        }
                    }
                }
            };
            if silent {
                silent_regions += 1;
            }
            regions.push(summary);
        }
        let mut sample_sources: Vec<String> = cache
            .entries()
            .values()
            .filter_map(|entry| entry.as_ref().ok().map(|reading| reading.describe.clone()))
            .collect();
        sample_sources.sort_unstable();
        let mut sample_failures: Vec<String> = cache
            .entries()
            .iter()
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
            silent_regions,
            builtin_silent_regions,
            skipped_regions,
            sample_sources,
            sample_failures,
            cache.truncated(),
            cache.frames_loaded(),
            provider,
            cc_census,
        )
    };
    // 片段序列随装载一起登记：渲染路径的 `with_instrument` 必须重放**同一份**展开结果。
    let (sources, include_issues) = match resolution {
        Some((sources, issues)) => (Some(Arc::from(sources)), issues),
        None => (None, Vec::new()),
    };
    Ok((
        SfzLoad {
            id: assign_id,
            path: sfz_path.to_path_buf(),
            bytes: byte_len,
            digest,
            text,
            sources,
            include_issues,
            regions,
            silent_regions,
            builtin_silent_regions,
            skipped_regions,
            reused,
            sample_hz,
            sample_sources,
            sample_failures,
            sample_reads_truncated,
            sample_frames_loaded,
            // ⭐ 预算随装载一起登记：读数的"截没截"必须能对回是哪一条预算判的。
            limits: cache.limits(),
            cc_census,
        },
        provider,
    ))
}

// ---------------------------------------------------------------------------
// ③ 采样内嵌（`embedded_samples`）：CAS 字节装载与逐音触发集
// ---------------------------------------------------------------------------

/// 内嵌映射表的样本键：绑定目录 + `sample=` 原串的词法拼接。
///
/// 与 [`resolve_sample_path`] 的**同一条解析语义**（`sample_path()` 原串、
/// `\\` 先归一成 `/`、相对路径相对 SFZ 所在目录），只差两点：
/// ① 锚点是**绑定路径**（渲染根相对）而不是磁盘路径 —— 根被约掉之后，
/// 保存侧与渲染侧算出同一个键；② 结果再经 [`collapse_embedded_path`] 词法折叠
/// （`.`/`..` 消解），因为渲染侧 `resolve_sample_path` 拼出的路径**不**折叠，
/// 而映射表里只存折叠后的那一份（缓存入口处统一折叠，见
/// [`EmbeddedSampleCache::get`]）。
///
/// 绝对路径（POSIX `/…` 或 Windows 盘符 `C:/…`）没有渲染根相对形态 ——
/// 原样（折叠后）作键：键只需在保存/渲染两侧算出同一个字符串，不必非是相对路径。
#[must_use]
pub(crate) fn embedded_sample_key(binding_path: &str, raw_sample: &str) -> String {
    let raw = raw_sample.replace('\\', "/");
    if raw.starts_with('/') || is_windows_drive_absolute(Path::new(&raw)) {
        return collapse_embedded_path(&raw);
    }
    let base = binding_path.replace('\\', "/");
    let base_dir = match base.rfind('/') {
        Some(index) => &base[..index],
        None => "",
    };
    if base_dir.is_empty() {
        collapse_embedded_path(&raw)
    } else {
        collapse_embedded_path(&format!("{base_dir}/{raw}"))
    }
}

/// 词法折叠一条 `/` 分隔路径：`.` 丢弃、`..` 弹一格（到顶为止）、空段丢弃；
/// 绝对路径（`/` 开头）保留前导 `/`。⛔ 不碰文件系统（无 symlink 解析），
/// 因此保存侧与渲染侧算出**同一个**字符串。
///
/// ⭐ Windows 分隔符先归一成 `/`：渲染侧的输入来自 `PathBuf::to_string_lossy()`
///（Windows 上是 `\`），而保存侧的键是字符串拼出的 `/` 形式 —— 归一必须
/// 发生在这里（两侧的唯一交汇点），否则 Windows 上查表永远失配
///（`assets/samples/kit\low.wav` 查不到 `assets/samples/kit/low.wav`）。
#[must_use]
fn collapse_embedded_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let absolute = normalized.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// 从 CAS 字节解码一份样本（与 [`read_sample_file`] 同一条格式口径，⛔ 不另写一份）。
///
/// * WAV（`wav` / `wave` 后缀）⇒ [`read_plain_wav_mono_f32_from_bytes`]
///   （hound 腿，与盘上腿**逐位相同**）；
/// * 其余 ⇒ `yeban-decode` 按内容探测（与盘上非 WAV 腿同一条）后折声道 0
///   （与 [`yeban_decode::decode_mono_channel0`] 同一个 `step_by(channels)`）。
///
/// 单文件字节上限与盘上腿同一条（[`MAX_SAMPLE_FILE_BYTES`]，⛔ 不截断）；
/// 描述串用调用方给的 `display`（渲染根相对键，跨机稳定）。
/// 读不到 ⇒ 返回**人话原因**（调用方按"缺样本 ⇒ 静音 ＋ 计数"处理，⛔ 不让装载失败）。
pub(crate) fn decode_sample_bytes(
    bytes: &[u8],
    display: &str,
) -> Result<SampleFileReading, String> {
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if len > MAX_SAMPLE_FILE_BYTES {
        return Err(format!(
            "{display} 有 {len} 字节, 超过单文件上限 {MAX_SAMPLE_FILE_BYTES} 字节 (拒绝读取, ⛔ 不截断)"
        ));
    }
    let (pcm, sample_rate, describe) = if is_plain_wav_suffix(Path::new(display)) {
        read_plain_wav_mono_f32_from_bytes(bytes, display)
            .map_err(|error| format!("{display}: {error:?}"))?
    } else {
        let asset = yeban_decode::decode_bytes(bytes, &yeban_decode::DecodeOptions::default())
            .map_err(|error| format!("{display}: {error}"))?;
        let channels = usize::from(asset.channels().max(1));
        let pcm: Vec<f32> = asset.samples().iter().step_by(channels).copied().collect();
        let describe = format!(
            "file({display} frames={} rate={})",
            pcm.len(),
            asset.sample_rate()
        );
        (pcm, asset.sample_rate(), describe)
    };
    let frames = u64::try_from(pcm.len()).unwrap_or(u64::MAX);
    Ok(SampleFileReading {
        pcm: Arc::new(pcm),
        sample_hz: sample_rate as f32,
        frames,
        describe,
    })
}

/// 从会话 CAS 池读一份内嵌样本（`key` → 哈希 → 字节 → 解码）。
///
/// 键不在表里 / 哈希非法 / 资产不在池里 ⇒ 人话原因（调用方按缺样本处理）。
fn read_embedded_sample(
    map: &BTreeMap<String, String>,
    assets: &dyn super::render::AssetStore,
    key: &str,
) -> Result<SampleFileReading, String> {
    let hash_text = map
        .get(key)
        .ok_or_else(|| format!("内嵌映射表里没有样本 `{key}`（容器与映射表不一致）"))?;
    let hash = AssetHash::parse(hash_text)
        .map_err(|error| format!("内嵌映射表里样本 `{key}` 的哈希 `{hash_text}` 非法: {error}"))?;
    let bytes = assets.asset(&hash).ok_or_else(|| {
        format!("内嵌样本 `{key}` 的资产 `{hash}` 不在会话 CAS 池里（容器缺资产条目）")
    })?;
    decode_sample_bytes(bytes, key)
}

/// 内嵌样本缓存：[`SampleReads`] 的 CAS 实现。
///
/// 查表键是 [`load_with_resolved`] 经 [`resolve_sample_path`] 拼出的渲染根相对路径
/// （`load_with_embedded` 传绑定路径当锚点）—— 入口处先经
/// [`collapse_embedded_path`] 折叠，与保存侧 [`embedded_sample_key`] 的输出
/// **逐字相同**。记账（文件数 / 帧数 / 截断）与 [`SampleCache`] 共用
/// [`cache_get`]，⛔ 不另写一份。
pub(crate) struct EmbeddedSampleCache<'a> {
    map: &'a BTreeMap<String, String>,
    assets: &'a dyn super::render::AssetStore,
    entries: SampleEntries,
    frames_loaded: u64,
    truncated: bool,
    limits: SampleLimits,
}

impl<'a> EmbeddedSampleCache<'a> {
    /// 空缓存，指定预算（渲染实参透传的那一份，与盘上腿同一条）。
    #[must_use]
    pub(crate) fn with_limits(
        map: &'a BTreeMap<String, String>,
        assets: &'a dyn super::render::AssetStore,
        limits: SampleLimits,
    ) -> Self {
        Self {
            map,
            assets,
            entries: SampleEntries::new(),
            frames_loaded: 0,
            truncated: false,
            limits,
        }
    }
}

impl SampleReads for EmbeddedSampleCache<'_> {
    fn get(&mut self, path: &Path) -> Result<SampleFileReading, String> {
        // ⚠ 折叠必须发生在这里（而不是调用方）：`load_with_resolved` 拼出的路径
        // 不折叠 `..`，而映射表的键是折叠后的 —— 两侧的唯一交汇点就是这一行。
        let key = collapse_embedded_path(&path.to_string_lossy());
        let key_path = PathBuf::from(&key);
        // 共享引用是 `Copy` ⇒ 先拷出，再把 `&mut` 借给 `cache_get`（⛔ 不解构：
        // 解构 `&mut self` 会让 `map` / `assets` 变成 `&mut &…`，闭包里要再解一次）。
        let map = self.map;
        let assets = self.assets;
        cache_get(
            &key_path,
            &mut self.entries,
            &mut self.frames_loaded,
            &mut self.truncated,
            self.limits,
            |resolved| read_embedded_sample(map, assets, &resolved.to_string_lossy()),
        )
    }

    fn entries(&self) -> &SampleEntries {
        &self.entries
    }

    fn truncated(&self) -> bool {
        self.truncated
    }

    fn frames_loaded(&self) -> u64 {
        self.frames_loaded
    }

    fn limits(&self) -> SampleLimits {
        self.limits
    }
}

/// 内嵌腿装载：SFZ 文本与样本字节**全部来自会话 CAS 池**，⛔ 不读盘。
///
/// `binding_path` 是工程里登记的绑定路径原样（渲染根相对）；`map` 是工程的
/// `embedded_samples` 表；`assets` 是会话 CAS 池。SFZ 文本由调用方
/// （`prepare_sampler_track`）按 `map[binding_path]` 从池里取出；`digest` 是
/// 该文本的 SHA-256（即表里的值）。
///
/// * `#include`：内嵌文本含 `#include` 时**响亮失败**（`samplerUnreadable` ——
///   保存侧从不内嵌这类 SFZ，因此它只可能来自手工拼的容器；展开需要磁盘解析器，
///   在无采样库的机器上没有可信的基准目录）。
/// * 缺样本：与盘上腿同一口径 —— 不进提供表 ⇒ 引擎按缺样本静音 ＋ 计数
///   （`missing_sample_paths`），⛔ 不发明替代内容；但**映射表本身悬空**
///   （键在表里、资产不在池里）是容器不一致 ⇒ `read_embedded_sample` 的失败
///   同样进 `sampleFailures`，由调用方按既有口径登记。
///
/// # Errors
///
/// 文本不是合法 UTF-8 / 含 `#include` / 文本非法 ⇒ [`Fault`]（消息含绑定路径）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn load_with_embedded(
    text: String,
    digest: String,
    binding_path: &str,
    map: &BTreeMap<String, String>,
    assets: &dyn super::render::AssetStore,
    sample_hz: f32,
    assign_id: u64,
    reused: bool,
    limits: SampleLimits,
    wanted: Option<&DemandLoadFilter>,
) -> Result<(SfzLoad, SampledProviderTable), Fault> {
    let file_name = Path::new(binding_path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            Fault::domain_with_data(
                CODE_IO,
                format!("内嵌采样器绑定路径没有文件名，无法装载: `{binding_path}`"),
                serde_json::json!({
                    "reason": "samplerUnreadable",
                    "path": binding_path,
                }),
            )
        })?;
    if embedded_text_has_include(&text, file_name)? {
        return Err(Fault::domain_with_data(
            CODE_IO,
            format!(
                "内嵌采样器定义文件含 `#include`，无法只凭容器装载: `{binding_path}`（保存侧从不内嵌这类 SFZ；请用磁盘上的原文件渲染）"
            ),
            serde_json::json!({
                "reason": "samplerUnreadable",
                "path": binding_path,
            }),
        ));
    }
    let byte_len = u64::try_from(text.len()).unwrap_or(u64::MAX);
    let mut cache = EmbeddedSampleCache::with_limits(map, assets, limits);
    load_with_resolved(
        text,
        byte_len,
        digest,
        Path::new(binding_path),
        None,
        sample_hz,
        assign_id,
        reused,
        &mut cache,
        wanted,
    )
}

/// 探测一段 SFZ 文本是否含 `#include`：把文本写进一个**空**临时目录再走真实的
/// [`resolve_include_sources`] —— 有 include ⇒ 至少一条目标在空目录里找不到 ⇒
/// `issues` 非空（或 `sources` 多于根文件本身）；无 include ⇒ 两者皆空。
///
/// 判据：`sources.len() == 1 && issues.is_empty()` ⟺ 无 include（`sources[0]`
/// 是根文件；`resolve_inner` 的文件数"含入口本身"）。
///
/// # Errors
///
/// 临时目录建不出 / 文本写不出 / 解析器报结构性错误（成环 / 超深 / 沙箱逃逸）⇒ [`Fault`]。
fn embedded_text_has_include(text: &str, file_name: &str) -> Result<bool, Fault> {
    let dir = std::env::temp_dir().join(format!(
        "yeban-embed-include-probe-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).map_err(|error| {
        Fault::domain(CODE_IO, format!("建内嵌 `#include` 探测目录失败: {error}"))
    })?;
    let probe = dir.join(file_name);
    let outcome = (|| {
        std::fs::write(&probe, text).map_err(|error| {
            Fault::domain(CODE_IO, format!("写内嵌 `#include` 探测文件失败: {error}"))
        })?;
        let resolution = resolve_include_sources(text, &probe)?;
        Ok(resolution.is_some_and(|(sources, issues)| sources.len() > 1 || !issues.is_empty()))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    outcome
}

/// 本轨谱子实际触发的 `(sample= 原串, 内嵌键)` 去重排序集。
///
/// 复用引擎既有区域匹配逻辑：[`DemandLoadFilter::from_project_track`] 逐音算 ＋
/// [`DemandLoadFilter::wants`] 逐区域判定（⛔ 不另写一套匹配；力度缺省值
/// —— 音符力度取模型值、新音符缺省 `DEFAULT_VELOCITY = 100` —— 键位切换、CC 门控
/// 全部与渲染的按需装载同一口径；CC 触发的区域（`trigger_by_note == false`）
/// 不在本次范围，见 `wants` 的同一条判定）。
///
/// `sfz_disk_path = Some(disk)` ⇒ 磁盘 SFZ：`#include` 走真实展开；含 include
/// 时返回 `Ok(None)`（调用方跳过该轨的内嵌 —— 展开后的片段无法用单文件键索引，
/// 且保存时静默改写 SFZ 文本是禁区）。
/// `sfz_disk_path = None` ⇒ 已内嵌文本：含 include 时同样返回 `Ok(None)`。
///
/// 内建静音 `sample=*silence` 不是文件、⛔ 不进集；`sample=` 为空的区域同理。
///
/// # Errors
///
/// SFZ 文本非法 ⇒ `sfzParseFailed`（与 [`load_with`] 同一条）。
pub(crate) fn triggered_sample_keys(
    text: &str,
    sfz_disk_path: Option<&Path>,
    binding_path: &str,
    filter: &DemandLoadFilter,
) -> Result<Option<BTreeSet<(String, String)>>, Fault> {
    // ⚠ `resolution` 只活在 `Some` 分支里 —— `Instrument` 借它，因此触发集
    // 必须在**同一分支内**算完（⛔ 不能把 `instrument` 带出 match）。
    let triggered = |instrument: &Instrument<'_>| {
        // ⭐ set_ccN 离线播种（路线 a）：与 `load_with_resolved` 同一条并集 ——
        // 保存路径收集的样本集必须与渲染装载的命中集同口径（种子 ∪ 泳道，泳道赢）。
        let filter = filter.with_cc_seeds(instrument.cc_defaults());
        let mut out: BTreeSet<(String, String)> = BTreeSet::new();
        for region in instrument.regions().iter() {
            if !filter.wants(region) {
                continue;
            }
            if region.is_builtin_silence() {
                continue;
            }
            let raw = region.sample_path().into_owned();
            if raw.trim().is_empty() {
                continue;
            }
            out.insert((raw.clone(), embedded_sample_key(binding_path, &raw)));
        }
        out
    };
    match sfz_disk_path {
        Some(disk) => {
            let resolution = resolve_include_sources(text, disk)?;
            if let Some((sources, issues)) = &resolution
                && (sources.len() > 1 || !issues.is_empty())
            {
                return Ok(None);
            }
            let instrument = parse_expanded(
                text,
                resolution.as_ref().map(|(sources, _)| sources.as_slice()),
            )?;
            Ok(Some(triggered(&instrument)))
        }
        None => {
            let file_name = Path::new(binding_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if file_name.is_empty() || embedded_text_has_include(text, file_name)? {
                return Ok(None);
            }
            let instrument = parse(text)?;
            Ok(Some(triggered(&instrument)))
        }
    }
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
        // ⭐ CC 门控与 amplitude 族调制：解析产物的原样抄录（普查在
        // [`cc_gate_census`] 里按整件乐器算，本摘要只携带逐区域原料）。
        cc_gates: region.cc_gates.clone(),
        amplitude: region.amplitude,
        amplitude_cc_mods: region
            .cc_modulations()
            .iter()
            .copied()
            .filter(|m| m.target == ModTarget::Amplitude)
            .collect(),
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
    /// 整段统一的力度覆盖（0..=127）；`None` = 每个音符用工程里自己的力度。
    ///
    /// ⚠ 2026-10-28 前这里是 `u8`（缺省 100）但**从未到达渲染器**：`collect_notes`
    /// 不读它，于是响应回显一个数字、音频里却毫无变化（R142"静默永不生效"）。
    /// 现在它真的覆盖每个音符的力度 ⇒ 区域选择（`matches_velocity`）与增益都随之改变。
    pub velocity: Option<u8>,
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
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|velocity| u8::try_from(velocity).ok())
                .filter(|velocity| *velocity <= 127)
                .ok_or_else(|| {
                    Fault::domain(
                        CODE_RANGE,
                        format!("`velocity` 必须是 0..=127 的整数, 实际收到 {value}"),
                    )
                })?,
        ),
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
    // ⭐ `#223`：四类**已知但不生效**的区域参数。⚠ **只有本工具**
    // （`yeban_render_sfz_wav`）的音源走 [`note_wave`] ⇒ 只有它不应用它们；
    // `yeban_render_master` 自 `#274` 起只经 [`survey_sampler`]（⛔ 不渲染）取读数，
    // 发声由引擎的 SFZ 支路完成 ⇒ 那边 `sfzSampleSpan` / `sfzRegionLayering` **已退役**。
    // 键名与 `render::KNOWN_UNSUPPORTED_KEYS` 的同一族逐字相同；这里按**音符数**计。
    /// 命中区域声明了 `amplitude` 一族的音符数。
    pub notes_with_amplitude: u64,
    /// 命中区域声明了 `tune` 的 CC 调制的音符数。
    pub notes_with_tune_cc: u64,
    /// 命中区域声明了 `volume` 的 CC 调制的音符数。
    pub notes_with_volume_cc: u64,
    /// 命中区域声明了 `offset` / `end` 播放区间的音符数。
    pub notes_with_sample_span: u64,
    /// ⭐ `#273`：**多于一条区域命中**同一音符的音符数（本工具只取第一条，引擎叠加全部）。
    pub notes_with_multiple_regions: u64,
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
        // ⭐ `#223`：四条与 `yeban_render_master` **同名**的已知不支持项。
        // 只在真的命中时登记（与那个工具的动态口径同一条纪律：⛔ 不无条件报）。
        if self.notes_with_amplitude > 0 {
            keys.push("sfzAmplitude");
        }
        if self.notes_with_tune_cc > 0 {
            keys.push("sfzTuneCcModulation");
        }
        if self.notes_with_volume_cc > 0 {
            keys.push("sfzVolumeCcModulation");
        }
        if self.notes_with_sample_span > 0 {
            keys.push("sfzSampleSpan");
        }
        // ⭐ `#273`：有多于一条区域命中同一音符 ⇒ 本工具只取第一条、引擎叠加全部。
        if self.notes_with_multiple_regions > 0 {
            keys.push("sfzRegionLayering");
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
/// - `velocity` 给了就**覆盖每个音符的力度**（⛔ 默认值：不给 = 每个音符自己的力度），
///   于是区域选择与增益都按这个力度算 —— 这是 `yeban_render_sfz_wav.velocity` 的消费点；
/// - 被曲目 mute / solo 门控掉的音轨 ⇒ 增益 0（它**仍**在排程里，读数因此看得见）。
///
/// 返回 `(音符列表, 被概率判为不触发的音符数)`。
#[must_use]
pub fn collect_notes(
    project: &YebanProjectV1,
    sample_rate: u32,
    only_note: Option<u8>,
    velocity: Option<u8>,
) -> (Vec<SfzNote>, u64) {
    let mut notes: Vec<SfzNote> = Vec::new();
    let mut silenced = 0u64;
    let any_solo = project.tracks.values().any(|track| track.solo);
    // `#339`：排程的 tick → 帧换算读工程的速度点（空表 = 恒定 BPM，逐位不变）。
    let tempo = project.tempo();
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
                    let (start, end) = math::note_frame_span_with_tempo(
                        base_tick.saturating_add(pulse.saturating_mul(step)),
                        step,
                        micro,
                        PPQ,
                        &tempo,
                        sample_rate,
                    );
                    let start_frame = u64::try_from(start.max(0)).unwrap_or(0);
                    let end_frame = u64::try_from(end.max(0)).unwrap_or(start_frame);
                    notes.push(SfzNote {
                        start_frame,
                        frames: end_frame.saturating_sub(start_frame).max(1),
                        pitch: note.pitch,
                        velocity: velocity.unwrap_or(note.velocity),
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
    /// 相位由**绝对采样点下标**算出（与引擎音源同一条口径：
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
    /// 回退到**内存正弦**的音符数（何时非 0 见
    /// [`SfzWavArtifact::notes_using_sine_fallback`] 的逐条定义）。
    pub notes_using_sine_fallback: u64,
    /// 本次渲染**真的读到**的样本文件来源（升序去重）。
    pub sample_sources: Vec<String>,
    /// 读失败的样本文件（升序去重）。
    pub sample_failures: Vec<String>,
    /// 是否命中过读盘上界。
    pub sample_reads_truncated: bool,
    /// 累计解码帧数。
    pub sample_frames_loaded: u64,
    /// ⭐ `#223`：命中区域声明了 `amplitude` 一族的音符数（本渲染器不应用）。
    pub notes_with_amplitude: u64,
    /// ⭐ `#223`：命中区域声明了 `tune` 的 CC 调制的音符数（本层没有 CC 状态）。
    pub notes_with_tune_cc: u64,
    /// ⭐ `#223`：命中区域声明了 `volume` 的 CC 调制的音符数（本层没有 CC 状态）。
    pub notes_with_volume_cc: u64,
    /// ⭐ `#223`：命中区域声明了 `offset` / `end` 播放区间的音符数（不按 span 切片）。
    pub notes_with_sample_span: u64,
    /// ⭐ `#273`：**多于一条区域命中**同一音符的音符数（本渲染器只取第一条，引擎叠加全部）。
    ///
    /// 只数**真的发声**的音符（[`note_wave`] 选出了区域）；`> 0` ⇒
    /// `yeban_render_sfz_wav` 登记 `sfzRegionLayering`。计数本身⛔ 不改变区域选择。
    pub notes_with_multiple_regions: u64,
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
    // 见 [`note_wave`] 里的 `file_rate` —— 拿 `engine_hz` 当 `sample_hz` 会让
    // 44.1 kHz 样本在 48 kHz 输出上快 48/44.1 ≈ 8.8%（≈ +1.47 半音）并提前耗尽。
    let mut regions_used: BTreeSet<usize> = BTreeSet::new();
    let mut notes_unmatched = 0u64;
    let mut notes_looping = 0u64;
    let mut notes_using_builtin_silence = 0u64;
    let mut notes_using_file_sample = 0u64;
    let mut notes_using_sine_fallback = 0u64;
    // ⭐ `#223`：区域参数读数（`prepare_sampler` 已退休；判据函数与
    //    `render::survey_sampler` **同一份**，`yeban_render_sfz_wav` 仍用本函数）。
    let mut notes_with_amplitude = 0u64;
    let mut notes_with_tune_cc = 0u64;
    let mut notes_with_volume_cc = 0u64;
    let mut notes_with_sample_span = 0u64;
    // ⭐ `#273`：多于一条区域命中同一音符的音符数（区域选择仍是"第一条"）。
    let mut notes_with_multiple_regions = 0u64;
    let mut cache = SampleCache::new();
    for note in notes {
        // ⭐ `#221`：区域选择 / 速率 / 增益 / 样本来源**只有一份实现**（[`note_wave`]）。
        // ⚠ 走这条路**发声**的只有 `yeban_render_sfz_wav`；`yeban_render_master` 自
        // `#274` 起不经这里发声（它只调 [`survey_sampler`] 取读数，音频由引擎的
        // SFZ 支路渲染，钉住这条的判据是 `tests/sampler_source_single_path.rs`）。
        let Some(wave) = note_wave(instrument, note, tone_hz, engine_hz, sfz_path, &mut cache)
        else {
            notes_unmatched = notes_unmatched.saturating_add(1);
            continue;
        };
        regions_used.insert(wave.region_index);
        // ⭐ `#273`：这个音符被**多于一条**区域命中 ⇒ 引擎会叠加全部，本渲染器只取第一条。
        // 只数发声的音符；`wave.layered` 由 [`note_wave`] 在**同一次**选择里顺手算出。
        if wave.layered {
            notes_with_multiple_regions = notes_with_multiple_regions.saturating_add(1);
        }
        // ⭐ `#223`：四类已知但不生效的区域参数，逐音符数出来（键进 `unsupported`）。
        if let Some(region) = instrument.regions().get(wave.region_index) {
            if region_declares_amplitude(region) {
                notes_with_amplitude = notes_with_amplitude.saturating_add(1);
            }
            if region_declares_cc_target(region, ModTarget::Tune) {
                notes_with_tune_cc = notes_with_tune_cc.saturating_add(1);
            }
            if region_declares_cc_target(region, ModTarget::Volume) {
                notes_with_volume_cc = notes_with_volume_cc.saturating_add(1);
            }
            if region_declares_sample_span(region) {
                notes_with_sample_span = notes_with_sample_span.saturating_add(1);
            }
        }
        if wave.looping {
            notes_looping = notes_looping.saturating_add(1);
        }
        match wave.kind {
            NoteSampleKind::BuiltinSilence => {
                notes_using_builtin_silence = notes_using_builtin_silence.saturating_add(1);
            }
            NoteSampleKind::FileSample => {
                notes_using_file_sample = notes_using_file_sample.saturating_add(1);
            }
            NoteSampleKind::SineFallback => {
                notes_using_sine_fallback = notes_using_sine_fallback.saturating_add(1);
            }
        }
        let span = usize::try_from(note.frames.max(1)).unwrap_or(0);
        let mono = &wave.mono;
        let written = wave.written;
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
        notes_with_amplitude,
        notes_with_tune_cc,
        notes_with_volume_cc,
        notes_with_sample_span,
        notes_with_multiple_regions,
    })
}

/// 一个音符的**样本来源裁决**（三种互斥出口；读数按它们分类计数）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NoteSampleKind {
    /// `sample=*silence`：按设计不产生采样输出（写零帧）。
    BuiltinSilence,
    /// 真的读到了文件样本并用它发声。
    FileSample,
    /// 读不到文件 ⇒ 回退到内存正弦（如实记账）。
    SineFallback,
}

/// 一个音符经采样器算出的**最终单声道波形**（已含区域增益）。
///
/// ⛔ 不含轨增益 / 声相 / 末尾淡出 —— 那三样是**调用方**的口径
/// （`yeban_render_sfz_wav` 乘音符自己的 `gain_l`/`gain_r` 并加淡出；
/// `yeban_render_master` 乘轨增益 `mute/solo × volume_db`）。
pub(super) struct NoteWave {
    /// 单声道波形（长度 = 该音符的时值帧；`written` 之后是 0）。
    pub mono: Vec<f32>,
    /// 实际写入的帧数（读到样本末尾就停）。
    pub written: usize,
    /// 命中的区域下标。
    pub region_index: usize,
    /// 该区域是否要求循环（本实现**不**展开循环，只响一遍）。
    pub looping: bool,
    /// ⭐ `#273`：**多于一条**区域命中这个音符（引擎叠加全部，本渲染器只取第一条）。
    pub layered: bool,
    /// 样本来源裁决。
    pub kind: NoteSampleKind,
}

/// **唯一的**"一个音符怎么响"实现：区域选择 → 播放速率 → 总增益 → 样本来源。
///
/// 返回 `None` 的唯一情形是"这个音符在这份 SFZ 里不发声"：没有区域匹配它的
/// 键/力度，或区域的播放速率/增益不可用（沿用既有的那一条守卫）。
///
/// 为什么抽出来（`#221`）：`yeban_render_master` 现在**真的消费**
/// `TrackV3.sampler`，而它必须与 `yeban_render_sfz_wav` 用的是**同一条**音源路径。
/// 如果各自写一遍区域选择，两条路会在"哪个样本、多快、多响"上漂移 —— 那正是
/// "监听/导出的不是同一个作品"那一族缺陷。
pub(super) fn note_wave(
    instrument: &Instrument<'_>,
    note: &SfzNote,
    tone_hz: f32,
    engine_hz: f32,
    sfz_path: Option<&Path>,
    cache: &mut SampleCache,
) -> Option<NoteWave> {
    let rates = RenderRates::new(engine_hz, engine_hz);
    // ⭐ `#273`：`find` 仍然只取**第一条**（区域选择逐位不变），但顺手数一下还有没有
    // 第二条命中 —— 引擎（`sfz_instrument.rs` 的 `render_note_into`）遍历全部命中并
    // 叠加 ⇒ 层叠本身是一项**已知分叉**，必须能报出来。
    //
    // 用同一个 `filter` 谓词选出迭代器 ⇒ "选中的区域"与"层叠的判定"不可能漂移。
    let mut matching = instrument
        .regions()
        .iter()
        .enumerate()
        .filter(|(_, region)| {
            region.matches_key(note.pitch) && region.matches_velocity(note.velocity)
        });
    let (region_index, region) = matching.next()?;
    let layered = matching.next().is_some();
    let rate = region.playback_rate(note.pitch, rates);
    let spec = region.playback_spec(note.pitch, note.velocity, rates);
    let gain = spec.total_gain();
    let looping = spec.loops();
    if !(rate.is_finite() && rate > 0.0) || gain == 0.0 {
        return None;
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
    let (written, kind) = if builtin_silence {
        // `mono` 全是 0.0；写 `span` 帧等价于"这个音符真的响，但样本值为 0"。
        (span, NoteSampleKind::BuiltinSilence)
    } else if let Some(reading) = &file_sample {
        // ⭐ 文件样本的步进比按**它自己的**采样率换算（`yeban-sfz` 的
        // `playback_rate` = `pitch_ratio * sample_hz / engine_hz`），与
        // `yeban-engine::SfzInstrument::render_note_into` 的
        // `RenderRates::new(source.sample_hz, engine_hz)` **同一口径**。
        let file_rate =
            region.playback_rate(note.pitch, RenderRates::new(reading.sample_hz, engine_hz));
        (
            render_pcm_into(reading.pcm.as_slice(), file_rate, gain, &mut mono),
            NoteSampleKind::FileSample,
        )
    } else {
        // 样本长度 = 这个音符的时值（外加一个淡出尾巴）⇒ 内存正弦在整段时值里都有
        // 数据可播。为什么按音符造：读到样本末尾就停的语义下，用一个固定 1 秒的
        // 样本会让长音符在第 1 秒之后静音（那是**静默的错误音频**）。
        let want = note.frames.saturating_add(NOTE_FADE_FRAMES).max(1);
        let sample_frames = usize::try_from(want.min(8_000_000)).unwrap_or(0).max(2);
        let sample = InMemorySfzSample::sine(tone_hz, engine_hz, sample_frames);
        (
            render_pcm_into(&sample.pcm, rate, gain, &mut mono),
            NoteSampleKind::SineFallback,
        )
    };
    Some(NoteWave {
        mono,
        written,
        region_index,
        looping,
        layered,
        kind,
    })
}

/// 一条采样器轨的**只读读数**（⭐ `#274`：发声改由引擎的 SFZ 支路负责，
/// 本层**不再**逐音符烘 PCM ⇒ `prepare_sampler`／`PreparedSampler` 已退休）。
///
/// 为什么还留这一组读数：`data.sources[].sampler` 的字段是响应契约的一部分
/// （既有判据逐条读它们），而且"这条轨命中了什么"与"引擎会怎么响"是**同一份**
/// 已解析区域表上的两个投影 —— 读数从区域表来，⛔ 不是第二份音源。
///
/// ⚠ 与引擎口径的**差别如实登记**：区域选择这里仍用 `matches_key && matches_velocity`
/// （与 `#221` 的 `note_wave` 同一条），**不含**通道 / `trigger` / keyswitch / CC 门控
/// —— 那些由引擎在摊平期烧进掩码。因此"读数说命中、引擎不出声"的情形只可能出现在
/// 那些门控上；本层的三个 `unsupported` 键（`sfzAmplitude` / `sfzTuneCcModulation` /
/// `sfzVolumeCcModulation`）只报**CC 调制**，静态参数已由引擎应用。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SamplerSurvey {
    /// 排程音符数（含没匹配到区域的）。
    pub notes_total: u64,
    /// 没匹配到任何区域的音符数。
    pub notes_unmatched: u64,
    /// 至少有一条命中区域要求循环的音符数（引擎**真的**展开循环）。
    pub notes_looping: u64,
    /// 至少有一条命中区域的样本**真的解码成功**的音符数。
    pub notes_using_file_sample: u64,
    /// 命中区域**只有**内建静音（`*silence`）的音符数。
    pub notes_using_builtin_silence: u64,
    /// 有命中区域、但**没有任何已解码样本**的音符数
    /// （引擎按"缺样本 ⇒ 静音 ＋ 计数"处理，⛔ 不回退正弦）。
    pub notes_without_sample: u64,
    /// 命中区域里**声明了 `ampeg_*`**（非缺省）的个数（引擎**应用**它，读数仍如实给出）。
    pub regions_with_envelope: u64,
    /// 命中区域里声明了 **`amplitude_oncc*`**（CC 调制）的个数。
    ///
    /// ⭐ `#274`：静态 `amplitude` 已由引擎应用 ⇒ **不再**计入这个键；只有 CC 那一半
    /// （本层没有 CC 状态）仍登记 `sfzAmplitude`。
    pub notes_with_amplitude_cc: u64,
    /// 命中区域里声明了 **`tune` 的 CC 调制**的个数（本层没有 CC 状态）。
    pub notes_with_tune_cc: u64,
    /// 命中区域里声明了 **`volume` 的 CC 调制**的个数（本层没有 CC 状态）。
    pub notes_with_volume_cc: u64,
    /// **多于一条**区域命中同一音符的音符数（引擎叠加全部 —— ⭐ `#274` 起不再是
    /// "已知分叉"，只是读数）。
    pub notes_with_multiple_regions: u64,
}

/// 在**已解析**的区域表上投影出 [`SamplerSurvey`]（`#274`；⛔ 不渲染、⛔ 不读盘）。
///
/// `decoded` 是本轨**真的解码成功**的 `sample=` 原样字符串集合
/// （由 `render.rs` 的 provider 从同一份区域表 + `SampleCache` 得到）。
///
/// `notes` 是 `(起始帧, 时值帧, 音高, 力度)`；只有后两项参与读数。
pub(super) fn survey_sampler(
    instrument: &Instrument<'_>,
    notes: &[(u64, u64, u8, u8)],
    decoded: &BTreeSet<String>,
) -> SamplerSurvey {
    let mut survey = SamplerSurvey {
        notes_total: u64::try_from(notes.len()).unwrap_or(u64::MAX),
        ..SamplerSurvey::default()
    };
    for &(_start, _frames, pitch, velocity) in notes {
        let mut matching = instrument
            .regions()
            .iter()
            .filter(|region| region.matches_key(pitch) && region.matches_velocity(velocity));
        let Some(first) = matching.next() else {
            survey.notes_unmatched = survey.notes_unmatched.saturating_add(1);
            continue;
        };
        let mut layered = matching.next().is_some();
        let mut looping = first
            .playback_spec(pitch, velocity, RenderRates::default())
            .loops();
        let mut has_file_sample = {
            let raw = first.sample_path();
            !first.is_builtin_silence() && decoded.contains(raw.as_ref())
        };
        let mut has_builtin_silence = first.is_builtin_silence();
        let envelope = region_declares_envelope(first);
        let amplitude_cc = region_declares_cc_target(first, ModTarget::Amplitude);
        let tune_cc = region_declares_cc_target(first, ModTarget::Tune);
        let volume_cc = region_declares_cc_target(first, ModTarget::Volume);
        // 其余命中的区域只参与"有没有文件样本 / 内建静音 / 循环"这四条读数：
        // 引擎会把**每一条**命中区域都建成声部 ⇒ 判定必须看全集（⛔ 只看第一条）。
        for region in matching {
            layered = true;
            looping |= region
                .playback_spec(pitch, velocity, RenderRates::default())
                .loops();
            let raw = region.sample_path();
            has_file_sample |= !region.is_builtin_silence() && decoded.contains(raw.as_ref());
            has_builtin_silence |= region.is_builtin_silence();
            // ⚠ 包络 / CC 三条仍按**第一条**数（与 `#221` 的读数口径**逐位不变**）；
            //    它们不是"引擎会不会响"的判据，而是"作者写了什么"的逐音符计数。
        }
        if layered {
            survey.notes_with_multiple_regions =
                survey.notes_with_multiple_regions.saturating_add(1);
        }
        if looping {
            survey.notes_looping = survey.notes_looping.saturating_add(1);
        }
        if has_file_sample {
            survey.notes_using_file_sample = survey.notes_using_file_sample.saturating_add(1);
        } else if has_builtin_silence {
            survey.notes_using_builtin_silence =
                survey.notes_using_builtin_silence.saturating_add(1);
        } else {
            survey.notes_without_sample = survey.notes_without_sample.saturating_add(1);
        }
        if envelope {
            survey.regions_with_envelope = survey.regions_with_envelope.saturating_add(1);
        }
        if amplitude_cc {
            survey.notes_with_amplitude_cc = survey.notes_with_amplitude_cc.saturating_add(1);
        }
        if tune_cc {
            survey.notes_with_tune_cc = survey.notes_with_tune_cc.saturating_add(1);
        }
        if volume_cc {
            survey.notes_with_volume_cc = survey.notes_with_volume_cc.saturating_add(1);
        }
    }
    survey
}

/// 该区域是否**声明了** SFZ 的幅度包络（`ampeg_*` 非缺省值）。
///
/// 缺省值由 `yeban-sfz` 的 `AMPEG_*` 常量钉住：attack/hold/decay/release 全 `0`、
/// sustain `100`（= 全程保持）⇒ 那是"没有包络"，⛔ 不算声明。
#[must_use]
fn region_declares_envelope(region: &Region<'_>) -> bool {
    region.ampeg_attack > 0.0
        || region.ampeg_hold > 0.0
        || region.ampeg_decay > 0.0
        || region.ampeg_release > 0.0
        || region.ampeg_sustain < 100.0
}

/// 该区域是否声明了 **`amplitude` 一族**（静态值非缺省，或有 `amplitude*ccN` 调制项）。
///
/// 判据的两半各有出处：
/// - 静态：`yeban-sfz` 的 [`AMPLITUDE_DEFAULT_PERCENT`]（规范缺省 100 ⇒ 乘子恰 1.0，
///   即"没声明"）；
/// - CC：`Region::cc_mods` 里 `target == ModTarget::Amplitude`。**不**按 `delta == 0`
///   过滤：本渲染器**没有任何 CC 状态**，任何一条声明都无法求值。
#[must_use]
fn region_declares_amplitude(region: &Region<'_>) -> bool {
    region.amplitude != AMPLITUDE_DEFAULT_PERCENT
        || region_declares_cc_target(region, ModTarget::Amplitude)
}

/// 该区域是否声明了**某个目标**的 CC 调制项（`ccN` / `_ccN` / `_onccN` / `_curveccN`）。
///
/// ⚠ 只数"声明"，⛔ 不按 `delta == 0` 过滤：本渲染器没有 CC 状态，任何一条声明都
/// 无法求值；而"作者写了 `delta=0`"与"作者没写"是两件事（与 `off_time` 的
/// `None` / `Some(0.0)` 同一条区分口径）。
#[must_use]
fn region_declares_cc_target(region: &Region<'_>, target: ModTarget) -> bool {
    region.cc_mods.iter().any(|item| item.target == target)
}

/// 该区域是否声明了 **`offset` / `end`**（播放区间会被归约，而本渲染器不切片）。
///
/// 缺省是 `offset = 0` ＋ `end = unspecified`（播到样本末尾）—— 那恰好是本渲染器的
/// 行为，因此**只有**偏离这对缺省值才算"声明了区间"。
#[must_use]
fn region_declares_sample_span(region: &Region<'_>) -> bool {
    region.offset != 0 || !matches!(region.end, SampleEnd::Unspecified)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `yeban_load_sfz.sampleHz` 的**形状校验**（[`parse_load_arguments`]）。
    ///
    /// 为什么这条判据在本模块而不在 `tests/sfz_tools.rs`：工具注册表先拒非法类型
    /// （`ParamSpec` 声明 `number` ⇒ 非数字走 JSON-RPC `-32602`），因此
    /// "必须是数字"这一支**经 `tools/call` 不可达**，集成判据到不了。
    /// 三支在这里一次钉住：缺省值 / 非数字 / 非正数；非正数的 MCP 侧回执
    /// 另由集成判据 `the_sample_hz_argument_is_honoured_and_bad_values_are_rejected` 断言。
    #[test]
    fn load_arguments_take_a_default_sample_hz_and_reject_bad_values() {
        let arguments = |value: Option<serde_json::Value>| {
            let mut map = serde_json::Map::new();
            map.insert(
                "path".to_owned(),
                serde_json::Value::String("/tmp/never-read.sfz".to_owned()),
            );
            if let Some(value) = value {
                map.insert("sampleHz".to_owned(), value);
            }
            map
        };
        assert_eq!(
            parse_load_arguments(&arguments(None), None)
                .expect("缺省组合法")
                .1,
            DEFAULT_SAMPLE_HZ,
            "缺省 ⇒ {DEFAULT_SAMPLE_HZ}"
        );
        assert_eq!(
            parse_load_arguments(&arguments(Some(serde_json::json!(44_100))), None)
                .expect("数字合法")
                .1,
            44_100.0
        );

        let Fault::Domain { code, message, .. } =
            parse_load_arguments(&arguments(Some(serde_json::json!("fast"))), None)
                .expect_err("非数字必须被拒")
        else {
            panic!("必须是领域失败");
        };
        assert_eq!(code, ErrorCode::InvalidParameterRange);
        assert!(
            message.contains("必须是数字"),
            "文案必须点名形状错误: {message}"
        );

        for bad in [0.0, -1.0] {
            let Fault::Domain { code, message, .. } =
                parse_load_arguments(&arguments(Some(serde_json::json!(bad))), None)
                    .expect_err("非正数必须被拒")
            else {
                panic!("必须是领域失败");
            };
            assert_eq!(code, ErrorCode::InvalidParameterRange);
            assert!(
                message.contains("必须是有限正数 (Hz)"),
                "文案必须点名要求: {message}"
            );
        }
    }

    /// E4：文件数上限是 128，且第 129 个**响亮拒绝**而不是静默丢弃。
    ///
    /// 用**不存在的路径**当探针：条目表按"不同路径"计数（失败也记一条，读成功的帧数
    /// 另算），因此不必真的造 128 个 WAV 就能钉住上限。真实大库的动机见
    /// [`MAX_SAMPLE_FILES_READ`] 的文档。
    #[test]
    fn the_sample_cache_accepts_128_distinct_files_and_refuses_the_129th() {
        let base = std::env::temp_dir().join(format!("yeban-mcp-sfz-cap-{}", std::process::id()));
        let mut cache = SampleCache::new();
        for index in 0..MAX_SAMPLE_FILES_READ {
            let path = base.join(format!("missing-{index}.wav"));
            assert!(cache.get(&path).is_err(), "不存在的文件必须如实报错");
        }
        assert_eq!(cache.files_failed(), MAX_SAMPLE_FILES_READ);
        assert!(!cache.truncated(), "正好 128 个还没触发截断");

        let overflow = base.join("missing-overflow.wav");
        let reason = cache.get(&overflow).expect_err("第 129 个必须被拒绝");
        assert!(
            reason.contains(&MAX_SAMPLE_FILES_READ.to_string()),
            "拒绝文案必须点名上限 128: {reason}"
        );
        assert!(cache.truncated(), "上限命中必须被记账");
    }

    /// ⭐ `#276` 判据 6 的**解码次数那一半**：共享池让同一份样本只解码一次，
    /// 而每条轨的预算记账与 [`SampleCache`] 逐条相同（帧数照记）。
    ///
    /// 判法：读完第一条轨的样本后**把文件删掉** —— 第二条轨仍然能拿到同一份 PCM
    /// ⇒ 它**不可能**是重新解码出来的。负对照用两个独立缓存：文件已删 ⇒ 必须失败，
    /// 证明"文件真的没了"，因此正腿不是空转。
    #[test]
    fn a_shared_store_decodes_a_sample_once_and_a_second_track_reads_the_pool() {
        use yeban_render::dither::PcmBuffer;
        use yeban_render::rf64::PcmFormat;
        use yeban_render::wav::write_plain_wav;

        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-sfz-shared-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |delta| delta.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("tone.wav");
        write_plain_wav(
            &path,
            &PcmFormat::float(1, 48_000, 32),
            &PcmBuffer::Float32(vec![0.25; 480]),
        )
        .expect("写夹具 WAV");

        let mut store = SampleStore::new();
        let first = {
            let mut track = TrackSampleCache::new(&mut store);
            let reading = track.get(&path).expect("第一条轨必须读到文件");
            assert_eq!(track.frames_loaded(), 480, "第一条轨照旧记账");
            assert!(!track.truncated(), "第一条轨没有命中任何上界");
            reading
        };
        assert_eq!(first.frames, 480);

        // ⚠ 删掉文件：第二次若重新解码就必然失败 ⇒ "只解码一次"于是可判。
        std::fs::remove_file(&path).expect("删夹具 WAV");
        let second = {
            let mut track = TrackSampleCache::new(&mut store);
            let reading = track
                .get(&path)
                .expect("第二条轨必须命中共享池（⛔ 不再读盘）");
            // 记账口径不变：命中共享池仍按该轨的预算记一次帧数（与修前逐条相同）。
            assert_eq!(track.frames_loaded(), 480, "第二条轨同样记账");
            assert!(!track.truncated(), "命中共享池不触发上界");
            reading
        };
        assert_eq!(second.frames, 480, "同一份 PCM 的帧数必须相同");
        assert_eq!(store.decoded_files(), 1, "同一文件只解码一次");

        // 负对照（修前的形状）：两个独立缓存 ⇒ 第二次读不到刚被删掉的文件。
        let mut independent = SampleCache::new();
        let err = independent
            .get(&path)
            .expect_err("文件已删，独立缓存必须读失败");
        assert!(
            err.contains("tone.wav"),
            "失败原因必须点名路径（证明对照真的在说这个文件）: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1 单测共用：26 位 ULID（与本模块既有测试同一拼法）。
    fn demand_uid(n: u128) -> EntityId {
        use std::str::FromStr as _;
        EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8{n:04}")).expect("合法 ULID")
    }

    /// P1 单测共用：含一条 MIDI 片段（给定音高/力度）的最小工程。
    fn demand_project(track_id: EntityId, notes: &[(u8, u8)]) -> YebanProjectV1 {
        let mut project = YebanProjectV1::default();
        let clip_id = demand_uid(9001);
        let mut midi_notes = BTreeMap::new();
        for (i, &(pitch, velocity)) in notes.iter().enumerate() {
            let id = demand_uid(9100 + i as u128);
            let mut note = yeban_model::MidiNote::new(id, (i as u64) * 960, pitch, 960);
            note.velocity = velocity;
            midi_notes.insert(id, note);
        }
        project.clip_pool.insert(
            clip_id,
            yeban_model::ClipPoolEntry {
                id: clip_id,
                name: "t".to_owned(),
                content: ClipContent::Midi { notes: midi_notes },
            },
        );
        let mut track = yeban_model::TrackV3 {
            id: track_id,
            ..yeban_model::TrackV3::default()
        };
        let placement_id = demand_uid(9200);
        track.clips.insert(
            placement_id,
            yeban_model::ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick: 0,
                duration_ticks: 960 * notes.len().max(1) as u64,
                loop_config: yeban_model::LoopConfig::default(),
                muted: false,
                offset_samples: None,
            },
        );
        project.tracks.insert(track_id, track);
        project
    }

    /// P1 单测共用：给本轨加一条 MidiCc 泳道（点值即原始 7 位值）。
    fn demand_cc_lane(
        project: &mut YebanProjectV1,
        track_id: EntityId,
        controller: u8,
        points: &[(u64, f32)],
        read_enabled: bool,
    ) {
        let target = AutomationTarget::MidiCc {
            track_id,
            controller,
        };
        let mut lane_points = BTreeMap::new();
        for (i, &(tick, value)) in points.iter().enumerate() {
            let id = demand_uid(9300 + i as u128);
            lane_points.insert(
                id,
                yeban_model::AutomationPoint {
                    id,
                    tick,
                    value,
                    curve: yeban_model::CurveType::Linear,
                },
            );
        }
        let track = project.tracks.get_mut(&track_id).expect("轨在");
        track.automation_lanes.insert(
            target,
            yeban_model::AutomationLane {
                target,
                points: lane_points,
                read_enabled,
                write_mode: yeban_model::AutomationWriteMode::Off,
                domain: None,
            },
        );
    }

    /// P1：Attack 区域按键/力度命中，范围外不命中。
    #[test]
    fn demand_load_attack_region_hit_by_matching_note_only() {
        let track_id = demand_uid(1001);
        let project = demand_project(track_id, &[(60, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60\n<region> sample=b.wav key=61\n<region> sample=c.wav key=60 lovel=101\n<region> sample=d.wav key=60 hivel=99\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(filter.wants(&regions[0]), "键 60 力度 100 命中");
        assert!(!filter.wants(&regions[1]), "键 61 不命中");
        assert!(!filter.wants(&regions[2]), "力度下界 101 不命中");
        assert!(!filter.wants(&regions[3]), "力度上界 99 不命中");
    }

    /// P1：通道门固定为 1（采样器路径不传通道）。
    #[test]
    fn demand_load_channel_gate_is_fixed_to_one() {
        let track_id = demand_uid(1002);
        let project = demand_project(track_id, &[(60, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60 lochan=2 hichan=2\n<region> sample=b.wav key=60 lochan=1 hichan=16\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(!filter.wants(&regions[0]), "不含通道 1 的区域不命中");
        assert!(filter.wants(&regions[1]), "含通道 1 的区域命中");
    }

    /// P1：键位切换摊平规则（`keyswitch_ok(None, |_| false)`）。
    #[test]
    fn demand_load_keyswitch_flatten_rule() {
        let track_id = demand_uid(1003);
        let project = demand_project(track_id, &[(60, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60 sw_last=60\n<region> sample=b.wav key=60 sw_last=60 sw_default=60\n<region> sample=c.wav key=60 sw_last=60 sw_default=61\n<region> sample=d.wav key=60 sw_down=61\n<region> sample=e.wav key=60 sw_up=61\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(!filter.wants(&regions[0]), "sw_last 无缺省 ⇒ 假");
        assert!(filter.wants(&regions[1]), "sw_last == sw_default ⇒ 过");
        assert!(!filter.wants(&regions[2]), "sw_last != sw_default ⇒ 假");
        assert!(!filter.wants(&regions[3]), "sw_down ⇒ 永假");
        assert!(filter.wants(&regions[4]), "sw_up 不挡（down 恒假）");
    }

    /// P1：CC 门控跟离线探针（无探针 ⇒ 控制器取缺省 0；有探针 ⇒ 无泳道控制器恒 0）。
    #[test]
    fn demand_load_cc_gates_follow_offline_probe() {
        let track_id = demand_uid(1004);
        // 无任何泳道 ⇒ 无探针 ⇒ 每一路控制器取缺省值 0 ⇒ 含 0 的门过、不含 0 的门假。
        let project = demand_project(track_id, &[(60, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60 locc1=0 hicc1=127\n\
                    <region> sample=b.wav key=60 locc105=64 hicc105=127\n\
                    <region> sample=c.wav key=60 hicc105=63\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(filter.wants(&regions[0]), "缺省 0 落在 [0,127] ⇒ 发声");
        assert!(
            !filter.wants(&regions[1]),
            "缺省 0 不在 [64,127] ⇒ 不发声（这是 cithara 的上层奏法）"
        );
        assert!(
            filter.wants(&regions[2]),
            "缺省 0 落在 [..,63] ⇒ 发声（这是 cithara 的默认奏法）"
        );
        // 有一条 CC2 泳道 ⇒ 探针存在；CC1 无泳道 ⇒ 恒 0。
        let mut project = demand_project(track_id, &[(60, 100)]);
        demand_cc_lane(&mut project, track_id, 2, &[(0, 64.0)], true);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60 locc1=0 hicc1=0\n<region> sample=b.wav key=60 locc1=1 hicc1=127\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(filter.wants(&regions[0]), "无泳道控制器恒 0 ⇒ 含 0 的门过");
        assert!(
            !filter.wants(&regions[1]),
            "无泳道控制器恒 0 ⇒ 不含 0 的门假"
        );
        // CC1 泳道值域 [10, 20] ⇒ 交集判定。
        let mut project = demand_project(track_id, &[(60, 100)]);
        demand_cc_lane(&mut project, track_id, 1, &[(0, 10.0), (960, 20.0)], true);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60 locc1=15 hicc1=25\n<region> sample=b.wav key=60 locc1=30 hicc1=40\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(filter.wants(&regions[0]), "值域 [10,20] 与 [15,25] 相交");
        assert!(!filter.wants(&regions[1]), "值域 [10,20] 与 [30,40] 不交");
        // 泳道被关掉 ⇒ 不算曲线 ⇒ 该控制器退回缺省 0。
        let mut project = demand_project(track_id, &[(60, 100)]);
        demand_cc_lane(&mut project, track_id, 1, &[(0, 10.0)], false);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60 locc1=0 hicc1=127\n\
                    <region> sample=b.wav key=60 locc1=64 hicc1=127\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(
            filter.wants(&regions[0]),
            "关掉的泳道 ⇒ 退回缺省 0 ⇒ 含 0 过"
        );
        assert!(
            !filter.wants(&regions[1]),
            "关掉的泳道 ⇒ 退回缺省 0 ⇒ 不含 0 假"
        );
    }

    /// P1（路线 a）：`set_ccN` 种子并入可达集 —— 无泳道时种子值就是可达集；
    /// 有泳道时**泳道赢**，种子不得把已由泳道决定的可达集撑大；
    /// 空种子表逐条不变。
    #[test]
    fn demand_load_cc_gates_include_set_cc_seeds() {
        let track_id = demand_uid(1006);
        let text = "<region> sample=a.wav key=60 locc1=64 hicc1=127\n\
                    <region> sample=b.wav key=60 hicc1=63\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        let mut seeds = BTreeMap::new();
        seeds.insert(1u16, 64u8);

        // 无泳道：并入前按缺省 0 判定（a 假、b 真）；并入种子 64 后翻转（a 真、b 假）。
        let project = demand_project(track_id, &[(60, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        assert!(!filter.wants(&regions[0]), "并入前：0 不在 [64,127]");
        assert!(filter.wants(&regions[1]), "并入前：0 在 [..,63]");
        let filter = filter.with_cc_seeds(&seeds);
        assert!(filter.wants(&regions[0]), "种子 64 在 [64,127] ⇒ 发声");
        assert!(!filter.wants(&regions[1]), "种子 64 不在 [..,63] ⇒ 该层熄");

        // 有泳道（值域单点 0）：可达集已有条目 ⇒ 种子不并入 ⇒ 判定不变（泳道赢）。
        let mut project = demand_project(track_id, &[(60, 100)]);
        demand_cc_lane(&mut project, track_id, 1, &[(0, 0.0)], true);
        let filter = DemandLoadFilter::from_project_track(&project, track_id).with_cc_seeds(&seeds);
        assert!(!filter.wants(&regions[0]), "泳道 0 赢：种子不得撑开可达集");
        assert!(filter.wants(&regions[1]), "泳道 0 赢：[..,63] 仍命中");

        // 空种子表：逐条不变。
        let project = demand_project(track_id, &[(60, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id)
            .with_cc_seeds(&BTreeMap::new());
        assert!(!filter.wants(&regions[0]), "空种子 ⇒ 与并入前一致");
        assert!(filter.wants(&regions[1]), "空种子 ⇒ 与并入前一致");
    }

    /// P1：宏级联进离线探针（偏移值域 ＝ depth × 宏曲线值域；全零点 ⇒ 级联为零）。
    #[test]
    fn demand_load_macro_cascade_feeds_cc_probe() {
        let track_id = demand_uid(1005);
        let mut project = demand_project(track_id, &[(60, 100)]);
        let track = project.tracks.get_mut(&track_id).expect("轨在");
        track.macros.push(yeban_model::MacroParameter {
            name: "m".to_owned(),
            value: 0.5,
            mappings: vec![yeban_model::MacroMapping {
                target: AutomationTarget::MidiCc {
                    track_id,
                    controller: 1,
                },
                depth: 10.0,
            }],
        });
        let macro_target = AutomationTarget::Macro {
            track_id,
            macro_index: 0,
        };
        let mut points = BTreeMap::new();
        for (i, &(tick, value)) in [(0, 0.0), (960, 1.0)].iter().enumerate() {
            let id = demand_uid(9400 + i as u128);
            points.insert(
                id,
                yeban_model::AutomationPoint {
                    id,
                    tick,
                    value,
                    curve: yeban_model::CurveType::Linear,
                },
            );
        }
        project
            .tracks
            .get_mut(&track_id)
            .expect("轨在")
            .automation_lanes
            .insert(
                macro_target,
                yeban_model::AutomationLane {
                    target: macro_target,
                    points,
                    read_enabled: true,
                    write_mode: yeban_model::AutomationWriteMode::Off,
                    domain: None,
                },
            );
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60 locc1=5 hicc1=15\n<region> sample=b.wav key=60 locc1=20 hicc1=30\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(
            filter.wants(&regions[0]),
            "宏偏移值域 [0,10] 与 [5,15] 相交"
        );
        assert!(
            !filter.wants(&regions[1]),
            "宏偏移值域 [0,10] 与 [20,30] 不交"
        );
        // 全零宏点 ⇒ 级联为零 ⇒ 无探针 ⇒ 门控假。
        let mut project = demand_project(track_id, &[(60, 100)]);
        let track = project.tracks.get_mut(&track_id).expect("轨在");
        track.macros.push(yeban_model::MacroParameter {
            name: "m".to_owned(),
            value: 0.0,
            mappings: vec![yeban_model::MacroMapping {
                target: AutomationTarget::MidiCc {
                    track_id,
                    controller: 1,
                },
                depth: 10.0,
            }],
        });
        let mut points = BTreeMap::new();
        for (i, &(tick, value)) in [(0, 0.0), (960, 0.0)].iter().enumerate() {
            let id = demand_uid(9500 + i as u128);
            points.insert(
                id,
                yeban_model::AutomationPoint {
                    id,
                    tick,
                    value,
                    curve: yeban_model::CurveType::Linear,
                },
            );
        }
        project
            .tracks
            .get_mut(&track_id)
            .expect("轨在")
            .automation_lanes
            .insert(
                macro_target,
                yeban_model::AutomationLane {
                    target: macro_target,
                    points,
                    read_enabled: true,
                    write_mode: yeban_model::AutomationWriteMode::Off,
                    domain: None,
                },
            );
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        // 全零宏级联 ⇒ 该控制器不进探针 ⇒ 取值退回缺省 0（⛔ 不是"整件不发声"）。
        let text = "<region> sample=a.wav key=60 locc1=0 hicc1=127\n\
                    <region> sample=b.wav key=60 locc1=64 hicc1=127\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(filter.wants(&regions[0]), "全零级联 ⇒ 缺省 0 ⇒ 含 0 的门过");
        assert!(
            !filter.wants(&regions[1]),
            "全零级联 ⇒ 缺省 0 ⇒ 不含 0 的门假"
        );
    }

    /// P1：First / Legato 的 held 规则（单音 ⇒ First 过、Legato 假；双音 ⇒ Legato 过）。
    #[test]
    fn demand_load_first_and_legato_held_rules() {
        let track_id = demand_uid(1006);
        let project = demand_project(track_id, &[(60, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60 trigger=first\n<region> sample=b.wav key=60 trigger=legato\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(filter.wants(&regions[0]), "首音起音时 held==0 ⇒ First 过");
        assert!(
            !filter.wants(&regions[1]),
            "单音不可能有 held>0 ⇒ Legato 假"
        );
        let project = demand_project(track_id, &[(60, 100), (64, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        assert!(filter.wants(&regions[1]), "双音 ⇒ Legato 可能（多算安全）");
    }

    /// P1：释放族按匹配音符保守计入；`key=-1` 永不命中。
    #[test]
    fn demand_load_release_family_and_key_minus_one() {
        let track_id = demand_uid(1007);
        let project = demand_project(track_id, &[(60, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60 trigger=release\n<region> sample=b.wav key=61 trigger=release\n<region> sample=c.wav key=60 trigger=release_key\n<region> sample=d.wav key=-1\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let regions = instrument.regions();
        assert!(filter.wants(&regions[0]), "匹配音符的 release 计入");
        assert!(!filter.wants(&regions[1]), "不匹配音符的 release 不计入");
        assert!(filter.wants(&regions[2]), "匹配音符的 release_key 计入");
        assert!(!filter.wants(&regions[3]), "key=-1 永不命中");
    }

    /// P1：与引擎自身判定的一致性（无 CC 门控的 Attack 区域：`wants` ⟺ 存在音符使
    /// `matches_query` 成立；后者是引擎控制侧查询的同一谓词）。
    #[test]
    fn demand_load_agrees_with_engine_query_on_plain_attack_regions() {
        use yeban_sfz::instrument::RegionQuery;
        let track_id = demand_uid(1008);
        let notes = [(60, 100), (64, 80), (72, 127)];
        let project = demand_project(track_id, &notes);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let text = "<region> sample=a.wav key=60\n<region> sample=b.wav key=61\n<region> sample=c.wav key=60 lovel=101\n<region> sample=d.wav key=64 lovel=80 hivel=80\n<region> sample=e.wav key=72 hivel=126\n<region> sample=f.wav key=60 sw_last=60\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        for region in instrument.regions() {
            let query_hit = notes
                .iter()
                .any(|&(pitch, velocity)| region.matches_query(&RegionQuery::new(pitch, velocity)));
            assert_eq!(
                filter.wants(region),
                query_hit,
                "区域 sample={} 的 wants 必须等于存在音符使 matches_query 成立",
                region.sample.as_ref()
            );
        }
    }

    /// 内嵌键：绑定目录 + `sample=` 原串的词法拼接（`\` 归一、`.`/`..` 折叠）。
    #[test]
    fn embedded_sample_key_joins_binding_dir_and_collapses_segments() {
        let binding = "assets/samples/kit/001.sfz";
        assert_eq!(
            embedded_sample_key(binding, "tone60.wav"),
            "assets/samples/kit/tone60.wav"
        );
        assert_eq!(
            embedded_sample_key(binding, "sub\\tone60.wav"),
            "assets/samples/kit/sub/tone60.wav"
        );
        assert_eq!(
            embedded_sample_key(binding, "../shared/kick.wav"),
            "assets/samples/shared/kick.wav"
        );
        assert_eq!(
            embedded_sample_key(binding, "./tone60.wav"),
            "assets/samples/kit/tone60.wav"
        );
        // `sample_path()` 含 `default_path=` 前缀时照原串拼（前缀就是路径的一段）。
        assert_eq!(
            embedded_sample_key(binding, "samples/tone60.wav"),
            "assets/samples/kit/samples/tone60.wav"
        );
        // 无目录的绑定 ⇒ 键就是原串本身。
        assert_eq!(embedded_sample_key("001.sfz", "a.wav"), "a.wav");
        // 绝对路径没有渲染根相对形态 —— 原样（折叠后）作键。
        assert_eq!(
            embedded_sample_key(binding, "/abs/kick.wav"),
            "/abs/kick.wav"
        );
        assert_eq!(
            embedded_sample_key(binding, "C:/lib/kick.wav"),
            "C:/lib/kick.wav"
        );
        assert_eq!(
            embedded_sample_key(binding, "sub/../tone60.wav"),
            "assets/samples/kit/tone60.wav"
        );
    }

    /// 渲染侧查表键：`EmbeddedSampleCache::get` 收到的是 `PathBuf::to_string_lossy()`
    /// 的结果 —— Windows 上是 `\` 分隔符。`collapse_embedded_path` 必须先把 `\`
    /// 归一成 `/`，否则查不到保存侧的正斜杠键（Windows CI 独败，Linux/macOS 全过）。
    #[test]
    fn collapse_embedded_path_normalises_windows_separators() {
        assert_eq!(
            collapse_embedded_path("assets\\samples\\kit\\low.wav"),
            "assets/samples/kit/low.wav"
        );
        assert_eq!(
            collapse_embedded_path("assets\\samples\\kit\\..\\shared\\kick.wav"),
            "assets/samples/shared/kick.wav"
        );
        assert_eq!(
            collapse_embedded_path("assets/samples/kit/low.wav"),
            "assets/samples/kit/low.wav"
        );
    }

    /// `#include` 探测：无 include ⇒ `false`；行首与行中 include ⇒ `true`。
    ///
    /// 探测走真实的 `resolve_include_sources`（空目录当沙箱），不是子串扫描：
    /// 行中 include（`<group> #include "…"`）同样能被认出。
    #[test]
    fn embedded_text_has_include_detects_only_real_includes() {
        let plain = "<region> sample=a.wav key=60\n<region> sample=b.wav key=61\n";
        assert!(
            !embedded_text_has_include(plain, "001.sfz").expect("探测"),
            "无 include 的文本必须报 false"
        );
        let with_include =
            "<region> sample=a.wav key=60\n#include \"sub.sfz\"\n<region> sample=b.wav key=61\n";
        assert!(
            embedded_text_has_include(with_include, "001.sfz").expect("探测"),
            "行首 include 必须报 true"
        );
        let mid_line =
            "<group> #include \"data/vel.txt\" lovel=1 hivel=26\n<region> sample=a.wav key=60\n";
        assert!(
            embedded_text_has_include(mid_line, "001.sfz").expect("探测"),
            "行中 include 必须报 true"
        );
    }

    /// 逐音触发集：只收音符命中区域的样本（多力度层只收触发到的层），
    /// 内建静音与未命中区域不收；磁盘腿与内嵌文本腿同一份集。
    #[test]
    fn triggered_sample_keys_collects_only_note_hit_regions() {
        let track_id = demand_uid(1010);
        let project = demand_project(track_id, &[(60, 100), (72, 32)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let binding = "assets/samples/kit/001.sfz";
        let text = "<region> sample=a.wav key=60\n\
                    <region> sample=b.wav key=61\n\
                    <region> sample=c.wav key=60 lovel=101\n\
                    <region> sample=shared.wav key=72 lovel=0 hivel=63\n\
                    <region> sample=shared.wav key=72 lovel=64 hivel=127\n\
                    <region> sample=*silence key=60\n";
        // 内嵌文本腿（`None`）：无磁盘路径。
        let from_text = triggered_sample_keys(text, None, binding, &filter)
            .expect("解析")
            .expect("无 include ⇒ 必须有集");
        // 磁盘腿（`Some`）：SFZ 写进临时目录，`#include` 走真实展开（无 include）。
        let dir = std::env::temp_dir().join(format!(
            "yeban-embed-triggered-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let disk_path = dir.join("001.sfz");
        std::fs::write(&disk_path, text).expect("写 SFZ 夹具");
        let from_disk = triggered_sample_keys(text, Some(&disk_path), binding, &filter)
            .expect("解析")
            .expect("无 include ⇒ 必须有集");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(from_text, from_disk, "磁盘腿与内嵌文本腿的触发集必须相同");
        let expected: BTreeSet<(String, String)> = [
            ("a.wav".to_owned(), "assets/samples/kit/a.wav".to_owned()),
            (
                "shared.wav".to_owned(),
                "assets/samples/kit/shared.wav".to_owned(),
            ),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            from_text, expected,
            "只收命中项：a（60/100）与 shared（72/32 低力度层）；b（61 无音符）、c（60 但力度 101 未触发）、*silence 都不收"
        );
    }

    /// 含 `#include` 的 SFZ：磁盘腿与内嵌文本腿都返回 `None`（调用方跳过该轨内嵌）。
    #[test]
    fn triggered_sample_keys_skips_sfz_with_includes() {
        let track_id = demand_uid(1011);
        let project = demand_project(track_id, &[(60, 100)]);
        let filter = DemandLoadFilter::from_project_track(&project, track_id);
        let binding = "assets/samples/kit/001.sfz";
        let dir = std::env::temp_dir().join(format!(
            "yeban-embed-triggered-inc-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        std::fs::write(dir.join("sub.sfz"), "<region> sample=a.wav key=60\n")
            .expect("写 include 目标");
        let text = "<region> sample=b.wav key=61\n#include \"sub.sfz\"\n";
        let disk_path = dir.join("001.sfz");
        std::fs::write(&disk_path, text).expect("写 SFZ 夹具");
        assert!(
            triggered_sample_keys(text, Some(&disk_path), binding, &filter)
                .expect("解析")
                .is_none(),
            "磁盘腿：含 include ⇒ None"
        );
        assert!(
            triggered_sample_keys(text, None, binding, &filter)
                .expect("解析")
                .is_none(),
            "内嵌文本腿：含 include ⇒ None"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⭐ CC 门控普查：门控与振幅两族按缺省 CC（全 0）分计，并集只计一次。
    ///
    /// 夹具逐条：a 门控不含 0 ⇒ gate；b 门控含 0 ⇒ 不计；c `amplitude=0` ⇒
    /// amplitude（无 CC 涉及）；d 只写 `amplitude_oncc7=100`、基值缺省 100 ⇒
    /// **不计**（曲线 0 在 0 处为 0 ⇒ 有效值仍 100、增益 1.0）；e 基值 0 ＋
    /// `amplitude_oncc7=100` ⇒ CC=0 处有效值 0 ⇒ amplitude、涉及 CC7；
    /// f 门控 ＋ 振幅双中 ⇒ 两族各计一次、总数只计一次；g 无门控 ⇒ 不计。
    #[test]
    fn cc_gate_census_counts_gate_and_amplitude_at_default_cc() {
        let text = "<region> sample=a.wav key=60 locc1=64 hicc1=127\n\
                    <region> sample=b.wav key=61 hicc1=63\n\
                    <region> sample=c.wav key=62 amplitude=0\n\
                    <region> sample=d.wav key=63 amplitude_oncc7=100\n\
                    <region> sample=e.wav key=64 amplitude=0 amplitude_oncc7=100\n\
                    <region> sample=f.wav key=65 locc2=10 amplitude=0\n\
                    <region> sample=g.wav key=66\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let census = cc_gate_census(&instrument);
        assert_eq!(census.silent_regions, 4, "并集 = a/c/e/f: {census:?}");
        assert_eq!(census.gate_silent_regions, 2, "门控 = a/f: {census:?}");
        assert_eq!(
            census.amplitude_silent_regions, 3,
            "振幅 = c/e/f: {census:?}"
        );
        assert_eq!(
            census.cc_numbers,
            vec![1, 2, 7],
            "涉及 CC 升序去重: {census:?}"
        );
        // 回执形态同值。
        let value = census.to_value();
        assert_eq!(value["silentAtDefaultCc"], 4);
        assert_eq!(value["byGate"], 2);
        assert_eq!(value["byAmplitude"], 3);
        assert_eq!(value["ccNumbers"], serde_json::json!([1, 2, 7]));
    }

    /// ⭐ CC 门控普查（路线 a）：探针取**乐器初值** —— `set_cc1=64` 让
    /// `locc1=64` 的区域离开静音集；种子落在门外的区域仍计静音；振幅臂同样
    /// 按初值求值（`set_cc7=127` 让 `amplitude=0 amplitude_oncc7=100` 的区域
    /// 有效值离开 0）。与离线播种同口径：普查说静音 ⇔ 无泳道时离线不发声。
    #[test]
    fn cc_gate_census_uses_set_cc_initial_values() {
        let text = "<control> set_cc1=64 set_cc7=127\n\
                    <region> sample=a.wav key=60 locc1=64 hicc1=127\n\
                    <region> sample=b.wav key=61 locc1=100 hicc1=127\n\
                    <region> sample=c.wav key=62 amplitude=0 amplitude_oncc7=100\n\
                    <region> sample=d.wav key=63 amplitude=0\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let census = cc_gate_census(&instrument);
        assert_eq!(census.silent_regions, 2, "并集 = b/d: {census:?}");
        assert_eq!(
            census.gate_silent_regions, 1,
            "门控 = b（种子 64 < 100）: {census:?}"
        );
        assert_eq!(
            census.amplitude_silent_regions, 1,
            "振幅 = d（c 被种子 127 抬离 0）: {census:?}"
        );
        assert_eq!(
            census.cc_numbers,
            vec![1],
            "只有 b 的门控 CC 进名册: {census:?}"
        );
    }

    /// ⭐ CC 门控普查：无门控库普查为 0（摘要仍带原料字段，值为空/缺省）。
    #[test]
    fn cc_gate_census_is_zero_for_a_plain_library() {
        let text =
            "<region> sample=a.wav key=60\n<region> sample=b.wav key=64 amplitude_oncc7=100\n";
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        let census = cc_gate_census(&instrument);
        assert_eq!(census, CcGateCensus::default(), "无门控 ⇒ 全 0: {census:?}");
        // 逐区域摘要的原料：无门控区域 cc_gates 为空、amplitude 为缺省 100。
        let plain = summarize(0, &instrument.regions()[0]);
        assert!(plain.cc_gates.is_empty());
        assert_eq!(plain.amplitude, AMPLITUDE_DEFAULT_PERCENT);
        assert!(plain.amplitude_cc_mods.is_empty());
        let modulated = summarize(1, &instrument.regions()[1]);
        assert_eq!(modulated.amplitude_cc_mods.len(), 1);
        assert_eq!(modulated.amplitude_cc_mods[0].cc, 7);
    }
}
