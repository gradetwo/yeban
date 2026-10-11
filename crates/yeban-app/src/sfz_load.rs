//! SFZ 乐器**装载**与**离线音符探测**（用户："加载一个 SFZ ⇒ 出声"）。
//!
//! ## 数据流（每一步都复用既有设施，⛔ 无旁路）
//!
//! ```text
//! 文件文本 ──▶ SfzInstrument::load(text, &ParseLimits)   // 引擎的闭包式装载器（控制侧）
//!          ──▶ region_count()                            // 报告里的"区域数"
//!          ──▶ InMemorySample::sine(440 Hz, engine_hz, n) // ⚠ 样本数据是**内存正弦**
//!          ──▶ render_note_into(&sample, note, vel, &mut out, engine_hz)
//!          ──▶ 帧数 ＋ 能量（Σx²）                        // 可机械判定的"非静音"证据
//! ```
//!
//! ## ⚠ 三条诚实登记（⛔ 不假装）
//!
//! 1. **样本文件没有被读取/解码**：`SfzInstrument` 只用文本；`sample=a.wav` 在本仓今天**只是路径字符串**
//!    ⇒ 本模块用 [`InMemorySample::sine`]（440 Hz）当数据源，报告行里明写
//!    `sample_source=in-memory-sine(440Hz)`，⛔ 不声称读到了 wav。
//! 2. **红线 7**：`render_note_into` **每次调用都解析 ⇒ 分配** ⇒ 只能**离线/控制侧**调用，
//!    ⛔ 绝不可在音频回调里调用（与 `arm_from_snapshot` 同一条边）。
//! 3. **尚未接到导出路径**：`--export-wav` 今天仍走引擎合成器（`arm_from_snapshot`）；
//!    SFZ 音色**没有**接进去 ⇒ 报告里的 `wired_to_export=false` 如实说明（下一步工作）。

use std::path::{Path, PathBuf};

use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument};
use yeban_sfz::parser::ParseLimits;

/// 探测用的样本源（440 Hz 内存正弦）的文案——报告行里逐字使用。
pub const SAMPLE_SOURCE: &str = "in-memory-sine(440Hz)";

/// 探测音符的默认音高（A4）。
pub const DEFAULT_PROBE_NOTE: u8 = 60;

/// 探测用的力度。
pub const DEFAULT_PROBE_VELOCITY: u8 = 100;

/// 装载失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SfzLoadError {
    /// 读文件失败。
    Read(String),
    /// SFZ 文本非法（语法/超限）—— 引擎原样透出的裁决。
    Parse(String),
}

impl core::fmt::Display for SfzLoadError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Read(reason) => write!(formatter, "读 SFZ 失败: {reason}"),
            Self::Parse(reason) => write!(formatter, "SFZ 非法: {reason}"),
        }
    }
}

impl core::error::Error for SfzLoadError {}

/// 一次音符探测的读数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfzProbeReading {
    /// 探测音高。
    pub note: u8,
    /// 探测力度。
    pub velocity: u8,
    /// `render_note_into` 写入的帧数（`0` = 没有匹配区域，**合法**：该音符不发声）。
    pub frames: usize,
    /// 输出能量 `Σx²`（`0.0` = 静音）。
    pub energy: f32,
}

impl SfzProbeReading {
    /// `true` 表示这次探测**真的出了声**（有帧且能量大于 0）。
    #[must_use]
    pub fn is_audible(&self) -> bool {
        self.frames > 0 && self.energy > 0.0
    }
}

/// 装载报告（形态照 `MidiImportReport`：路径／字节／区域数／探测读数／诚实字段）。
#[derive(Clone, Debug, PartialEq)]
pub struct SfzLoadReport {
    /// SFZ 文件路径。
    pub path: PathBuf,
    /// 文件字节数。
    pub bytes: u64,
    /// 区域数（`SfzInstrument::region_count`）。
    pub regions: usize,
    /// 探测读数（`probe_note = None` 时为 `None`）。
    pub probe: Option<SfzProbeReading>,
    /// 样本数据来源的**如实**文案（⛔ 不是 wav 文件名）。
    pub sample_source: &'static str,
    /// 是否已经接到 `--export-wav` 的音源上（今天恒为 `false`，见模块文档 ③）。
    pub wired_to_export: bool,
}

impl SfzLoadReport {
    /// 一行可读摘要（CLI 打印它）。
    #[must_use]
    pub fn summary(&self) -> String {
        let probe = match self.probe {
            Some(reading) => format!(
                "{}:{}:{}:{}",
                reading.note, reading.velocity, reading.frames, reading.energy
            ),
            None => "-".to_owned(),
        };
        format!(
            "loaded-sfz: path={} bytes={} regions={} sample_source={} probe(note:vel:frames:energy)={probe} wired_to_export={}",
            self.path.display(),
            self.bytes,
            self.regions,
            self.sample_source,
            if self.wired_to_export { "yes" } else { "no" },
        )
    }
}

/// 装载一份 SFZ 并（可选）探测一个音符。
///
/// # Errors
///
/// 读文件失败 / SFZ 文本非法。⛔ 本函数**不写任何文件** ⇒ 失败时不会留下半成品。
pub fn load_sfz(
    path: &Path,
    probe_note: Option<u8>,
    engine_hz: f32,
) -> Result<SfzLoadReport, SfzLoadError> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| SfzLoadError::Read(format!("读 `{}` 失败: {error}", path.display())))?;
    let bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    // 控制侧专用（会分配）—— 见模块文档 ②。
    let instrument = SfzInstrument::load(text, &ParseLimits::default())
        .map_err(|error| SfzLoadError::Parse(error.to_string()))?;
    let probe = probe_note
        .map(|note| probe_note_energy(&instrument, note, engine_hz))
        .transpose()?;
    Ok(SfzLoadReport {
        path: path.to_path_buf(),
        bytes,
        regions: instrument.region_count(),
        probe,
        sample_source: SAMPLE_SOURCE,
        // 如实：SFZ 音色今天**没有**接进 `--export-wav` 的音源链。
        wired_to_export: false,
    })
}

/// 用一个内存正弦样本触发 `note`，返回帧数与能量（离线/控制侧）。
///
/// # Errors
///
/// 文本在此期间变得非法时透出解析错误（正常路径不会发生）。
pub fn probe_note_energy(
    instrument: &SfzInstrument,
    note: u8,
    engine_hz: f32,
) -> Result<SfzProbeReading, SfzLoadError> {
    // 0.5 秒的样本数据足够覆盖一次探测（引擎按 `RenderRates` 换算播放速率）。
    let frames = usize::try_from(engine_hz.max(1.0) as u64 / 2)
        .unwrap_or(1)
        .max(1);
    let source = InMemorySample::sine(440.0, engine_hz, frames);
    let mut out = vec![0.0_f32; frames];
    let written = instrument
        .render_note_into(&source, note, DEFAULT_PROBE_VELOCITY, &mut out, engine_hz)
        .map_err(|error| SfzLoadError::Parse(error.to_string()))?;
    let energy: f32 = out.iter().map(|sample| sample * sample).sum();
    Ok(SfzProbeReading {
        note,
        velocity: DEFAULT_PROBE_VELOCITY,
        frames: written,
        energy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-sfz-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 判据 a：最小 SFZ（两条区域）⇒ 区域数与文件内容一致。
    #[test]
    fn the_region_count_matches_the_file() {
        let dir = temp_dir("count");
        let path = dir.join("two.sfz");
        std::fs::write(
            &path,
            "<region> sample=a.wav key=60\n<region> sample=b.wav key=64\n",
        )
        .expect("写夹具");
        let report = load_sfz(&path, None, 48_000.0).expect("装载");
        assert_eq!(report.regions, 2, "两条 <region> ⇒ 2");
        assert!(report.probe.is_none(), "没要求探测 ⇒ 无探测读数");
        assert!(
            report.summary().contains(SAMPLE_SOURCE),
            "报告必须如实说明样本来源（⛔ 不是 wav 文件名）"
        );
        assert!(!report.wired_to_export, "今天必须如实说未接到导出");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 b：**触发某音符 ⇒ 非静音**（帧数 > 0 且能量 > 0），且与**不触发**（不匹配的键）可区分。
    #[test]
    fn a_matching_key_is_audible_and_an_unmatched_key_is_silent() {
        let dir = temp_dir("probe");
        let path = dir.join("one.sfz");
        std::fs::write(&path, "<region> sample=a.wav key=60\n").expect("写夹具");
        let instrument = {
            let text = std::fs::read_to_string(&path).expect("读");
            SfzInstrument::load(text, &ParseLimits::default()).expect("装载")
        };
        let hit = probe_note_energy(&instrument, 60, 48_000.0).expect("探测");
        let miss = probe_note_energy(&instrument, 127, 48_000.0).expect("探测");
        println!(
            "[sfz-probe] key=60: {} 帧，能量 {:.6}；key=127: {} 帧，能量 {:.6}",
            hit.frames, hit.energy, miss.frames, miss.energy
        );
        assert!(hit.is_audible(), "匹配键必须非静音: {hit:?}");
        assert!(
            !miss.is_audible(),
            "不匹配键必须静音（合法的不发声）: {miss:?}"
        );
        assert_eq!(miss.frames, 0, "没有匹配区域 ⇒ 写入 0 帧");
        assert_eq!(miss.energy, 0.0, "没有匹配区域 ⇒ 能量 0");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 c：文件不存在 ⇒ `Read` 错误（⛔ 不 panic），且**不新增任何文件**。
    #[test]
    fn a_missing_file_is_an_error_and_writes_nothing() {
        let dir = temp_dir("missing");
        let before = std::fs::read_dir(&dir).expect("列目录").count();
        let error =
            load_sfz(&dir.join("nope.sfz"), Some(60), 48_000.0).expect_err("文件不存在必须失败");
        assert!(matches!(error, SfzLoadError::Read(_)), "实际: {error:?}");
        assert_eq!(std::fs::read_dir(&dir).expect("列目录").count(), before);
        std::fs::remove_dir_all(&dir).ok();
    }
}
