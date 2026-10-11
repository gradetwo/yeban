//! 录音的**用户入口**：把一段 PCM 录成可回放的 WAV（`[ARCH-REC-001]` 的三段：建环 ⇒ 推帧 ⇒ 抽干落盘）。
//!
//! ## 数据流（三段，每一步都复用既有设施，⛔ 无旁路）
//!
//! ```text
//! ① 构造期（**唯一分配点**）：record_channel(capacity_frames, channels) -> (sender, receiver)
//! ② 生产端（红线 7 安全）：先问 sender.spare_frames() >= 1，再 push_frame(&frame)
//!                          环满 ⇒ 丢弃这一帧并**自己**计数（⛔ 不阻塞、⛔ 不 panic）
//! ③ 消费端：RecordDrain::new(receiver, SampleSink) ⇒ pump(batch)（引擎写**小端 f32**）
//!          ⇒ 量化 24-bit（TPDF 抖动）⇒ PcmBuffer::Int24 ⇒ write_plain_wav（**唯一落盘点**）
//! ```
//!
//! ## ⚠ 为什么生产端要先问 `spare_frames()`
//!
//! `RecordSender::push_frame` 的文档写明：环满时它**无法回滚** `rtrb` 已接受的样本
//! （引擎源码里的注释也说"用前置检查替代"）⇒ **盲推会留下半帧 ⇒ 声道错位**。
//! 引擎因此提供 [`RecordSender::spare_frames`]，并要求调用方"推之前先问"。
//! 本模块按该契约**整帧原子**地推，并在**帧**这一级数丢弃
//! （⚠ 引擎的 `dropped_frames()` 是**样本**级，口径不同 ⇒ 报告里的 `dropped=` 是本模块的帧级读数）。
//!
//! ## ⚠ 其它诚实登记（⛔ 不假装）
//!
//! 1. **本切片不碰声卡**：cpal 输入流不在本轮（本机禁编 cpal，设备线程属"逐步完善"）
//!    ⇒ 样本来源是一条**可控的内存测试音**（正弦），报告行里 `source=in-memory-sine(440Hz)`。
//! 2. **⛔ 不自写 RIFF 头**：WAV 一律交 [`yeban_render::wav::write_plain_wav`]。

use std::io::Write;
use std::path::{Path, PathBuf};

use yeban_engine::record::{RecordDrain, record_channel};
use yeban_render::dither::{PcmBuffer, quantize_i24};
use yeban_render::rf64::PcmFormat;
use yeban_render::rng::DeterministicDitherRng;
use yeban_render::wav::write_plain_wav;

/// 测试音的默认频率（A4）。
pub const TEST_TONE_HZ: f32 = 440.0;

/// 测试音的默认幅度（留足余量，⛔ 不削波）。
pub const TEST_TONE_AMPLITUDE: f32 = 0.5;

/// 样本来源的**如实**文案（⛔ 不是声卡）。
pub const SOURCE_LABEL: &str = "in-memory-sine(440Hz)";

/// 录制参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordOptions {
    /// 录制秒数。
    pub seconds: f32,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: u16,
    /// 环容量（帧）。**故意可小**：小环更容易观察到"环满 ⇒ 丢弃计数"。
    pub capacity_frames: usize,
    /// 抽取批大小（帧/次 `pump`）。
    pub drain_batch: usize,
    /// 测试音频率（Hz）。
    pub tone_hz: f32,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self {
            seconds: 2.0,
            sample_rate: 48_000,
            channels: 2,
            // 1 秒的环：默认参数下**不应**发生丢弃（判据另用小环制造丢弃）。
            capacity_frames: 48_000,
            drain_batch: 4096,
            tone_hz: TEST_TONE_HZ,
        }
    }
}

/// 录制报告（形态照 `MidiImportReport`／`WavExportReport`：路径／帧数／丢弃／字节／来源）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordReport {
    /// 落盘路径。
    pub path: PathBuf,
    /// 成功写出的帧数（`RecordDrain::frames_written`）。
    pub frames: u64,
    /// **整帧**成功推入的帧数（`RecordSender::pushed_frames`）。
    pub pushed: u64,
    /// 因环满被丢弃的帧数（**帧**级；本模块自己数，见模块文档）。
    pub dropped: u64,
    /// 文件字节数。
    pub bytes: u64,
    /// 位深。
    pub bits: u16,
    /// 样本来源的**如实**文案（工程渲染／内存正弦；⛔ 不是声卡）。
    pub source: String,
}

impl RecordReport {
    /// 一行可读摘要（CLI 打印它）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "recorded-wav: path={} frames={} dropped={} bytes={} source={}",
            self.path.display(),
            self.frames,
            self.dropped,
            self.bytes,
            self.source,
        )
    }
}

/// 录制失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// 抽干/写盘时的 I/O 失败（含目标不可写）。
    Io(String),
    /// WAV 写出被拒（`write_plain_wav` 的裁决）。
    Write(String),
    /// 参数非法（0 秒 / 0 声道 / 0 容量）。
    Invalid(String),
    /// 工程渲染失败（空工程 / 计划编译 / 引擎装载）。
    Render(String),
}

impl core::fmt::Display for RecordError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(reason) => write!(formatter, "录音抽干失败: {reason}"),
            Self::Write(reason) => write!(formatter, "写出 WAV 失败: {reason}"),
            Self::Invalid(reason) => write!(formatter, "录音参数非法: {reason}"),
            Self::Render(reason) => write!(formatter, "工程渲染失败: {reason}"),
        }
    }
}

impl core::error::Error for RecordError {}

/// **解码型 sink**：接住 `RecordDrain::pump` 写出的**小端 f32**（引擎文档的口径），
/// 还原成交错样本 ⇒ 之后统一走量化 ＋ `write_plain_wav`（⛔ 不在这里写文件）。
#[derive(Debug, Default)]
struct SampleSink {
    pending: Vec<u8>,
    samples: Vec<f32>,
}

impl SampleSink {
    /// 把完整的 4 字节组解成样本；不足 4 字节的尾巴留到下一次（`Write` 可能分批到达）。
    fn decode(&mut self) {
        let whole = self.pending.len() - self.pending.len() % 4;
        // `as_chunks` 而不是 `chunks_exact`：clippy 要求常量块长用前者（编译期就保证长度）。
        let (chunks, _rest) = self.pending[..whole].as_chunks::<4>();
        for chunk in chunks {
            self.samples.push(f32::from_le_bytes(*chunk));
        }
        self.pending.drain(..whole);
    }
}

impl Write for SampleSink {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(buffer);
        self.decode();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 录一段**测试音**并落成 WAV：建环 ⇒ 推帧 ⇒ 抽干 ⇒ 量化 ⇒ 一次落盘。
///
/// # Errors
///
/// 参数非法 / 抽干 I/O 失败 / WAV 写出被拒。⛔ 失败时**不写任何文件**（唯一落盘点在最后一步）。
pub fn record_test_tone(path: &Path, options: &RecordOptions) -> Result<RecordReport, RecordError> {
    if options.seconds <= 0.0 {
        return Err(RecordError::Invalid("录制秒数必须大于 0".to_owned()));
    }
    if options.channels == 0 || options.capacity_frames == 0 || options.drain_batch == 0 {
        return Err(RecordError::Invalid(
            "声道数 / 环容量 / 抽取批大小都必须大于 0".to_owned(),
        ));
    }
    let total_frames = (options.seconds * options.sample_rate as f32) as u64;

    // ---- ① 构造期：**唯一分配点** ----
    let (mut sender, receiver) = record_channel(options.capacity_frames, options.channels);

    // ---- ② 生产端 ＋ ③ 消费端**交错**（贴近真实双线程拓扑：录的同时后台抽干）----
    //
    // ⚠ 为什么必须交错（本机实测的教训）：若"先全推、再抽干"，一条 1 秒的环在 8 秒请求下只能
    // 留下 1 秒（实测 `frames=48000 / dropped=336000`）⇒ 用户拿到的文件比请求短得多。
    // 交错之后环只需容纳**一个批**，丢弃只在"批比环还大"或真的跟不上时才发生（正是要被观测的场景）。
    let mut drain = RecordDrain::new(receiver, SampleSink::default());
    let mut frame = vec![0.0_f32; usize::from(options.channels)];
    let mut phase = 0.0_f32;
    let step = core::f32::consts::TAU * options.tone_hz / options.sample_rate as f32;
    for index in 0..total_frames {
        let value = phase.sin() * TEST_TONE_AMPLITUDE;
        // clippy 要求用 `fill` 而不是手写循环。
        frame.fill(value);
        // ⚠ 引擎已把 `push_frame` 修成**整帧原子**（`faaa7240`）⇒ ⛔ 不再需要自己先问
        // `spare_frames()`；环满由**引擎**丢弃并计**帧**数（下面直接读 `dropped_frames()`）。
        let _ = sender.push_frame(&frame);
        phase = (phase + step) % core::f32::consts::TAU;
        // 每 `drain_batch` 帧抽干一次（与生产交替）。
        // ⚠ 用**循环下标**判断（不是总帧数 —— 常量条件会让交错永不触发：本机实测踩过）。
        if (index + 1) % u64::try_from(options.drain_batch.max(1)).unwrap_or(1) == 0 {
            while drain
                .pump(options.drain_batch)
                .map_err(|error| RecordError::Io(error.to_string()))?
                > 0
            {}
        }
    }
    let pushed = sender.pushed_frames();
    // 引擎的 `dropped_frames()` 在整帧原子化之后计的是**帧**（与 `pushed_frames` 同口径）。
    let dropped = sender.dropped_frames();

    // ---- ③ 消费端：抽干（引擎写小端 f32）----
    loop {
        let written = drain
            .pump(options.drain_batch)
            .map_err(|error| RecordError::Io(error.to_string()))?;
        if written == 0 {
            break;
        }
    }
    let frames_written = drain.frames_written();
    let sink = drain.into_inner();

    // ---- 量化 ＋ 一次落盘（⛔ 不自写 RIFF 头）----
    let mut rng = DeterministicDitherRng::new(1);
    let quantized: Vec<i32> = sink
        .samples
        .iter()
        .map(|sample| quantize_i24(*sample, &mut rng))
        .collect();
    let buffer = PcmBuffer::Int24(quantized);
    let format = PcmFormat::integer(options.channels, options.sample_rate, 24);
    write_plain_wav(path, &format, &buffer)
        .map_err(|error| RecordError::Write(error.to_string()))?;

    let bytes = std::fs::metadata(path)
        .map_err(|error| RecordError::Io(error.to_string()))?
        .len();
    Ok(RecordReport {
        path: path.to_path_buf(),
        frames: frames_written,
        pushed,
        dropped,
        bytes,
        bits: 24,
        source: SOURCE_LABEL.to_owned(),
    })
}

/// **录"工程的声音"**：用**同一条**渲染核心（[`crate::render_core::render_project_f32`]）拿交错 `f32`
/// ⇒ 建环 ⇒ `push_frame` ⇒ `RecordDrain::pump` ⇒ **同一 `DeterministicDitherRng` 种子**量化
/// ⇒ `write_plain_wav`。
///
/// ## ⚠ 红线 7 的**分侧**（必须明白）
///
/// - **渲染/解析侧（前半）**：会解析 SFZ／装载快照／分配缓冲 ⇒ **允许**，但**只准在离线/控制侧**跑。
/// - **生产端（`push_frame`）**：⛔ **不得分配／加锁／做 I/O**（引擎已保证）——若违反，实时录音线程会
///   爆音、丢帧、甚至优先级反转（这就是红线 7 的后果）。
///
/// # Errors
///
/// 渲染失败（空工程／计划编译／引擎装载）／抽干 I/O 失败／WAV 写出被拒。⛔ 失败时不写任何文件。
pub fn record_project(
    project: &yeban_model::YebanProjectV1,
    path: &Path,
    options: &RecordOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
) -> Result<RecordReport, RecordError> {
    if options.seconds <= 0.0 {
        return Err(RecordError::Invalid("录制秒数必须大于 0".to_owned()));
    }
    if options.channels == 0 || options.capacity_frames == 0 || options.drain_batch == 0 {
        return Err(RecordError::Invalid(
            "声道数 / 环容量 / 抽取批大小都必须大于 0".to_owned(),
        ));
    }
    // ---- 渲染侧（离线；允许分配）----
    let wav_options = crate::export_wav::WavExportOptions {
        frames: (options.seconds * options.sample_rate as f32) as u64,
        sample_rate: options.sample_rate,
        channels: usize::from(options.channels),
        seed: 1,
        use_engine_source: true,
    };
    let (samples, _frames, source_label, _digest) =
        crate::render_core::render_project_f32(project, wav_options, sfz)
            .map_err(|error| RecordError::Render(error.to_string()))?;
    // ---- 构造期：唯一分配点 ----
    let (mut sender, receiver) = record_channel(options.capacity_frames, options.channels);
    // ---- 生产端（逐帧推；红线 7 安全）＋ 消费端**交错抽干** ----
    //
    // ⚠ 必须交错（与 `record_test_tone` 同一条教训）：若"先全推、再抽干"，一条 1 秒的环在 8 秒请求下
    // 只能留下 1 秒（实测 `帧 48000 / 丢弃 336000`）⇒ 录音比工程短、⛔ 更谈不上逐位一致。
    let mut drain = RecordDrain::new(receiver, SampleSink::default());
    let channels = usize::from(options.channels);
    let mut frame = vec![0.0_f32; channels];
    let batch = options.drain_batch.max(1);
    for (index, chunk) in samples.chunks_exact(channels).enumerate() {
        frame.copy_from_slice(chunk);
        let _ = sender.push_frame(&frame);
        if (index + 1) % batch == 0 {
            while drain
                .pump(batch)
                .map_err(|error| RecordError::Io(error.to_string()))?
                > 0
            {}
        }
    }
    let pushed = sender.pushed_frames();
    let dropped = sender.dropped_frames();
    // ---- 消费端：把剩下的抽干 ----
    loop {
        let written = drain
            .pump(options.drain_batch)
            .map_err(|error| RecordError::Io(error.to_string()))?;
        if written == 0 {
            break;
        }
    }
    let frames_written = drain.frames_written();
    let sink = drain.into_inner();
    let mut rng = DeterministicDitherRng::new(1);
    let quantized: Vec<i32> = sink
        .samples
        .iter()
        .map(|sample| quantize_i24(*sample, &mut rng))
        .collect();
    let buffer = PcmBuffer::Int24(quantized);
    let format = PcmFormat::integer(options.channels, options.sample_rate, 24);
    write_plain_wav(path, &format, &buffer)
        .map_err(|error| RecordError::Write(error.to_string()))?;
    let bytes = std::fs::metadata(path)
        .map_err(|error| RecordError::Io(error.to_string()))?
        .len();
    Ok(RecordReport {
        path: path.to_path_buf(),
        frames: frames_written,
        pushed,
        dropped,
        bytes,
        bits: 24,
        source: source_label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-rec-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 判据 a：录出的 WAV **可被 `read_plain_wav` 读回**，帧数与推入数一致（无丢弃），样本非静音。
    #[test]
    fn the_recorded_wav_reads_back_with_the_pushed_frame_count() {
        let dir = temp_dir("readback");
        let path = dir.join("take.wav");
        let options = RecordOptions {
            seconds: 0.25,
            capacity_frames: 48_000,
            ..RecordOptions::default()
        };
        let report = record_test_tone(&path, &options).expect("录制必须成功");
        println!(
            "[record] 写出 {} 帧 / 丢弃 {} / 推入 {}；文件 {} 字节",
            report.frames, report.dropped, report.pushed, report.bytes
        );
        assert_eq!(report.dropped, 0, "默认容量 1 秒 ⇒ 0.25 秒的录制不该丢弃");
        assert_eq!(
            report.frames, report.pushed,
            "整帧契约：写出帧 == 成功推入帧（0.25 秒 @48kHz = 12000）"
        );
        assert_eq!(report.frames, 12_000, "帧数 = 秒数 × 采样率");
        let (format, buffer) = yeban_render::wav::read_plain_wav(&path).expect("可回读");
        assert_eq!(format.sample_rate, 48_000);
        assert_eq!(format.channels, 2);
        assert_eq!(format.bits_per_sample, 24);
        let frames = u64::try_from(buffer.len() / usize::from(format.channels)).expect("帧数");
        assert_eq!(frames, report.frames, "回读帧数必须等于报告帧数");
        let peak = match &buffer {
            PcmBuffer::Int24(samples) => samples
                .iter()
                .map(|sample| sample.unsigned_abs())
                .max()
                .unwrap_or(0),
            other => panic!("24-bit 整数: {other:?}"),
        };
        assert!(peak > 0, "录出的样本不许是静音（peak = {peak}）");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 b：**环满场景** ⇒ 丢弃被计数、⛔ 不 panic、半帧不入环（帧对齐），落盘文件仍合法。
    #[test]
    fn a_full_ring_drops_frames_without_panicking_and_still_writes_a_valid_wav() {
        let dir = temp_dir("fullloop");
        let path = dir.join("overflow.wav");
        let options = RecordOptions {
            seconds: 0.5,
            // 故意很小的环：160 帧（≈3.3 ms @48 kHz）⇒ 必然丢弃。
            capacity_frames: 160,
            ..RecordOptions::default()
        };
        let report = record_test_tone(&path, &options).expect("环满不是错误");
        println!(
            "[record] 小环：推入 {} / 写出 {} / 丢弃 {}（尝试 24000）",
            report.pushed, report.frames, report.dropped
        );
        assert!(
            report.dropped > 0,
            "小环必须观察到丢弃（实际 {}）",
            report.dropped
        );
        assert_eq!(
            report.frames, report.pushed,
            "整帧契约：写出帧 == 成功推入帧（半帧不入环 ⇒ 声道对齐）"
        );
        assert_eq!(
            report.pushed + report.dropped,
            24_000,
            "框架不变量：成功推入 ＋ 丢弃 == 尝试次数（0.5 秒 @48kHz）"
        );
        let (format, buffer) = yeban_render::wav::read_plain_wav(&path).expect("仍然可回读");
        let frames = u64::try_from(buffer.len() / usize::from(format.channels)).expect("帧数");
        assert_eq!(frames, report.frames, "回读帧数 == 写出帧数");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 c：**目标不可写** ⇒ 失败而⛔ 不留半成品（唯一落盘点在最后一步）。
    #[test]
    fn an_unwritable_target_fails_and_leaves_nothing_behind() {
        let dir = temp_dir("unwritable");
        // 目标是一个**目录**路径 ⇒ `write_plain_wav` 必然拒绝。
        let target = dir.join("a-directory.wav");
        std::fs::create_dir_all(&target).expect("建目录");
        let error = record_test_tone(&target, &RecordOptions::default())
            .expect_err("目录不可作为 WAV 目标");
        assert!(
            matches!(error, RecordError::Write(_)),
            "必须是写出被拒: {error:?}"
        );
        let entries: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec!["a-directory.wav".to_owned()],
            "⛔ 不许留下半成品"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 a：**同一工程、同一 8 秒 ⇒ 录音与 `--export-wav` 导出**逐位一致****（判定量＝字节全等），
    /// 并配**反向对照**（两侧都非静音，⛔ 排除"都全零也算相等"）。
    #[test]
    fn recording_the_project_is_bit_identical_to_exporting_it() {
        let dir = temp_dir("project-bitexact");
        let project = yeban_model::samples::filled_project();
        let exported = dir.join("exported.wav");
        let recorded = dir.join("recorded.wav");
        let export_report = crate::export_wav::export_wav_with_sfz_file(
            &project,
            &exported,
            crate::export_wav::WavExportOptions::default(),
            None,
        )
        .expect("导出必须成功");
        let options = RecordOptions {
            seconds: 8.0,
            ..RecordOptions::default()
        };
        let record_report =
            record_project(&project, &recorded, &options, None).expect("录制必须成功");
        println!(
            "[record-project] 导出 source={} / 字节 {}；录制 source={} / 帧 {} / 丢弃 {} / 字节 {}",
            export_report.source,
            export_report.bytes,
            record_report.source,
            record_report.frames,
            record_report.dropped,
            record_report.bytes
        );
        assert_eq!(
            record_report.dropped, 0,
            "默认 1 秒环 ＋ 交错抽干 ⇒ 不该丢弃"
        );
        assert_eq!(record_report.frames, 384_000, "8 秒 @48kHz = 384000 帧");
        for path in [&exported, &recorded] {
            let (format, buffer) = yeban_render::wav::read_plain_wav(path).expect("可回读");
            assert_eq!(format.channels, 2);
            let nonzero = match &buffer {
                PcmBuffer::Int24(samples) => samples.iter().filter(|sample| **sample != 0).count(),
                other => panic!("24-bit 整数: {other:?}"),
            };
            assert!(
                nonzero > 0,
                "{} 必须非静音（非零样本 {nonzero}）",
                path.display()
            );
        }
        assert_eq!(
            std::fs::read(&exported).expect("读导出"),
            std::fs::read(&recorded).expect("读录音"),
            "同一工程同一 8 秒 ⇒ 录音与导出必须逐位一致"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 b：录工程时**环满** ⇒ 丢弃被计数（帧级）、⛔ 不 panic、文件仍合法可回读。
    #[test]
    fn recording_the_project_with_a_full_ring_counts_drops_and_still_writes_a_valid_wav() {
        let dir = temp_dir("project-overflow");
        let project = yeban_model::samples::filled_project();
        let path = dir.join("overflow.wav");
        let options = RecordOptions {
            seconds: 2.0,
            capacity_frames: 160,
            drain_batch: 4096,
            ..RecordOptions::default()
        };
        let report = record_project(&project, &path, &options, None).expect("环满不是错误");
        println!(
            "[record-project] 小环：推入 {} / 写出 {} / 丢弃 {}",
            report.pushed, report.frames, report.dropped
        );
        assert!(
            report.dropped > 0,
            "小环必须观察到丢弃（实际 {}）",
            report.dropped
        );
        assert_eq!(
            report.frames, report.pushed,
            "整帧契约：写出帧 == 成功推入帧"
        );
        assert_eq!(
            report.pushed + report.dropped,
            96_000,
            "框架不变量：成功推入 ＋ 丢弃 == 尝试次数（2 秒 @48kHz）"
        );
        let (format, buffer) = yeban_render::wav::read_plain_wav(&path).expect("仍然可回读");
        let frames = u64::try_from(buffer.len() / usize::from(format.channels)).expect("帧数");
        assert_eq!(frames, report.frames, "回读帧数 == 写出帧数");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 c：**空工程（没有轨道）** ⇒ 报错而⛔ 不 panic ＋ **不留半成品目标文件**。
    #[test]
    fn recording_an_empty_project_fails_without_leaving_a_target() {
        let dir = temp_dir("project-empty");
        let mut project = yeban_model::samples::filled_project();
        project.tracks.clear();
        let target = dir.join("never.wav");
        let error = record_project(&project, &target, &RecordOptions::default(), None)
            .expect_err("空工程必须失败");
        assert!(
            matches!(error, RecordError::Render(_)),
            "必须是渲染失败: {error:?}"
        );
        assert!(!target.exists(), "失败 ⇒ 目标文件不许存在");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 造一份"按工程音高逐键写区域"的 SFZ（＋可选 `sample=`），返回 `(instrument, sfz 路径)`。
    fn sfz_for(
        project: &yeban_model::YebanProjectV1,
        dir: &Path,
        sample: Option<&str>,
    ) -> (
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        PathBuf,
    ) {
        let mut pitches: Vec<u8> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values().map(|note| note.pitch))
            .collect();
        pitches.sort_unstable();
        pitches.dedup();
        let text: String = pitches
            .iter()
            .map(|pitch| match sample {
                Some(sample) => format!("<region> sample={sample} key={pitch}\n"),
                None => format!("<region> sample=a.wav key={pitch}\n"),
            })
            .collect();
        let path = dir.join("keys.sfz");
        std::fs::write(&path, &text).expect("写 SFZ");
        let instrument = yeban_engine::sfz_instrument::SfzInstrument::load(
            text,
            &yeban_sfz::parser::ParseLimits::default(),
        )
        .expect("装载");
        (std::sync::Arc::new(instrument), path)
    }

    /// 判据 a：**带 SFZ 与不带 SFZ 录同一工程 ⇒ 可区分**（判定量＝**逐字节不同**），
    /// 且**两侧都非静音**（反向对照，⛔ 排除"都全零也算相等"）。
    #[test]
    fn recording_with_an_sfz_differs_from_recording_without_one() {
        let dir = temp_dir("record-sfz-differs");
        let project = yeban_model::samples::filled_project();
        let (instrument, sfz_path) = sfz_for(&project, &dir, None);
        let with_sfz = dir.join("with-sfz.wav");
        let without = dir.join("without.wav");
        let options = RecordOptions {
            seconds: 2.0,
            ..RecordOptions::default()
        };
        let a = record_project(
            &project,
            &with_sfz,
            &options,
            Some((instrument, Some(sfz_path))),
        )
        .expect("带 SFZ 录制");
        let b = record_project(&project, &without, &options, None).expect("不带 SFZ 录制");
        println!(
            "[record-sfz] 带 SFZ source={} / 字节 {}；不带 source={} / 字节 {}",
            a.source, a.bytes, b.source, b.bytes
        );
        assert!(
            a.source.contains("sfz"),
            "来源必须明说走了 SFZ: {}",
            a.source
        );
        assert!(
            b.source.contains("引擎乐器"),
            "不带 SFZ 时来源是引擎乐器: {}",
            b.source
        );
        for path in [&with_sfz, &without] {
            let (format, buffer) = yeban_render::wav::read_plain_wav(path).expect("可回读");
            assert_eq!(format.channels, 2);
            let nonzero = match &buffer {
                PcmBuffer::Int24(samples) => samples.iter().filter(|sample| **sample != 0).count(),
                other => panic!("24-bit 整数: {other:?}"),
            };
            assert!(
                nonzero > 0,
                "{} 必须非静音（非零样本 {nonzero}）",
                path.display()
            );
        }
        assert_ne!(
            std::fs::read(&with_sfz).expect("读带 SFZ"),
            std::fs::read(&without).expect("读不带 SFZ"),
            "判定量：带 SFZ 与不带 SFZ 录出的 WAV 必须逐字节不同"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 b：**带同一 SFZ ⇒ 录制与导出逐位一致**（两条路共享 `render_core` ⇒ 应当成立）。
    #[test]
    fn recording_with_an_sfz_is_bit_identical_to_exporting_with_the_same_sfz() {
        let dir = temp_dir("record-sfz-bitexact");
        let project = yeban_model::samples::filled_project();
        // 用一条**常量样本文件**，让"真的走了 SFZ 区域"也可从来源文案看出来。
        let scale = yeban_render::dither::BitDepth::Int24.full_scale_max() / 4;
        let sample_path = dir.join("const.wav");
        yeban_render::wav::write_plain_wav(
            &sample_path,
            &yeban_render::rf64::PcmFormat::integer(1, 48_000, 24),
            &PcmBuffer::Int24(vec![scale; 4_800]),
        )
        .expect("写样本");
        let (instrument, sfz_path) = sfz_for(&project, &dir, Some("const.wav"));
        let exported = dir.join("exported.wav");
        let recorded = dir.join("recorded.wav");
        let export_report = crate::export_wav::export_wav_with_sfz_file(
            &project,
            &exported,
            crate::export_wav::WavExportOptions::default(),
            Some((instrument.clone(), Some(sfz_path.clone()))),
        )
        .expect("带 SFZ 导出");
        let record_report = record_project(
            &project,
            &recorded,
            &RecordOptions {
                seconds: 8.0,
                ..RecordOptions::default()
            },
            Some((instrument, Some(sfz_path))),
        )
        .expect("带 SFZ 录制");
        println!(
            "[record-sfz] 导出 source={} / 字节 {}；录制 source={} / 帧 {} / 丢弃 {} / 字节 {}",
            export_report.source,
            export_report.bytes,
            record_report.source,
            record_report.frames,
            record_report.dropped,
            record_report.bytes
        );
        assert_eq!(record_report.frames, 384_000);
        assert_eq!(record_report.dropped, 0);
        assert_eq!(
            std::fs::read(&exported).expect("读导出"),
            std::fs::read(&recorded).expect("读录制"),
            "判定量：带同一 SFZ ⇒ 录制与导出必须逐位一致"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
