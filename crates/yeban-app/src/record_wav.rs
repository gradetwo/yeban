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
//! 1. **本模块自身不做设备 I/O**：[`record_test_tone`] 的样本来源是一条**可控的内存测试音**
//!    （正弦，报告行 `source=in-memory-sine(440Hz)`）。⭐ **产品的录音键**（[`Recorder`]）
//!    从任务单 `#88` 起**不再**是"没人喂"的静音：源由
//!    [`crate::record_device::InputSourceFactory`] 给出（默认 = **真设备输入流**；
//!    显式静音腿 = `YEBAN_RECORD_SOURCE=silence`；打不开 ⇒ **响亮失败**，⛔ 不录静音）。
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
    /// ⭐ **输入源打不开**（`[ARCH-REC-001]` 任务单 `#88`）：设备缺失 / 协商不匹配 /
    /// 流启动失败 —— 由 [`crate::record_device::InputSourceFactory::open`] 原样给出。
    ///
    /// ⚠ 与 [`Self::Invalid`] **分开**：这是"环境打不开"，那头是"调用方给错了" ——
    /// 把两者混在一起会把排查方向引错（与引擎 `DeviceError` 把输入/输出分开同一条理由）。
    /// ⛔ 本变体**绝不**表示"那就录静音"：出现它 ⇒ **没有**会话、**没有**文件。
    Source(String),
}

impl core::fmt::Display for RecordError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(reason) => write!(formatter, "录音抽干失败: {reason}"),
            Self::Write(reason) => write!(formatter, "写出 WAV 失败: {reason}"),
            Self::Invalid(reason) => write!(formatter, "录音参数非法: {reason}"),
            Self::Render(reason) => write!(formatter, "工程渲染失败: {reason}"),
            Self::Source(reason) => write!(formatter, "打开录音输入源失败: {reason}"),
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

/// ⭐ **录音会话的默认落盘路径**（`[ARCH-REC-001]`）。
///
/// 读 `YEBAN_UI_RECORD_WAV_PATH`（本 crate 既有的录音路径环境变量族，见
/// `tests/ui_module_actions.rs`）；缺省落到**当前目录**的 `ui-record.wav`
/// （与 `host::ModulePaths` 的同一族默认值同源，⛔ 不落系统临时目录 ——
/// 那里的路径带随机分量，回执行取证困难）。
#[must_use]
pub fn default_record_path() -> PathBuf {
    std::env::var_os("YEBAN_UI_RECORD_WAV_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ui-record.wav"))
}

/// ⭐ **一次录音会话用到的参数**（路径 ＋ 建环参数）—— 任务单 `#88` 的**可注入面**。
///
/// ## 为什么要有它（⛔ 不是包装癖）
///
/// 原来这两样是**写死在录音闭包里**的：路径读环境变量、参数取
/// [`RecordOptions::default`]。于是判据想验"录到的**不是**静音"就必须
/// **改进程环境变量**才能决定落盘位置 —— 而测量纪律第 6 条明确**禁止**判据读环境。
/// 把它提升成一个**值**（[`Self::production`] 是产品那一条），判据就能从**代码缝**
/// 注入自己的路径与参数，⛔ 不碰环境、⛔ 不往仓库根写文件。
#[derive(Clone, Debug, PartialEq)]
pub struct RecordPlan {
    /// 落盘路径。
    pub path: PathBuf,
    /// 建环参数（声道数 / 采样率 / 环容量 / 抽取批）。
    pub options: RecordOptions,
}

impl RecordPlan {
    /// ⭐ **产品那一条**：默认路径（[`default_record_path`]）＋ 默认参数。
    #[must_use]
    pub fn production() -> Self {
        Self {
            path: default_record_path(),
            options: RecordOptions::default(),
        }
    }
}

/// ⭐ **设备输入源**（`[ARCH-REC-001]` 的"音频线程直接从声卡物理输入捕获 PCM 流"那一半）。
///
/// ⭐ **为什么是 trait**：真实现要 `cpal` 的 `default_input_device` ＋ `build_input_stream`
/// （`yeban_engine::device::open_input`，线程与生命期都属引擎层），而判据要注入
/// **脚本化假源**。⇒ 本 trait 是"源的形状"，[`crate::record_device`] 是它的两条实现
/// （真设备 / 显式静音）＋ [`crate::record_device::InputSourceFactory`] 工厂缝。
///
/// ⚠ **块语义**：`next_block(frames, out)` 填**交错**样本，填 `frames × channels` 个；
/// 返回**真的填了几个帧**（0 ⇒ 暂时没有数据，调用方下一轮再问）。⛔ 不长阻塞。
pub trait InputSource {
    /// 声道数。⭐ **它是权威**：调用方（[`Recorder`]）以它为准建环 / 切块 / 写 WAV 头 ——
    /// 设备协商到的通道数与 [`RecordOptions::channels`]（**请求值**）不同时，录下来的是
    /// **设备**那一条（如实），⛔ 不是伪造的立体声。
    ///
    /// ⚠ 它**必须**与 `next_block` 实际填的交错宽度一致（否则会错位）。
    fn channels(&self) -> u16;
    /// 取下一块交错样本。返回填进去的**帧**数。
    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize;
    /// ⭐ **如实文案**：落进 [`RecordReport::source`] ⇒ "这条录音是从哪来的"可读、可判。
    ///
    /// ⚠ 默认值是**诚实的中性文案**（⛔ 不是"内存正弦"——那会把判据里的假源
    /// 报成产品源）；每条产品/判据实现都应当覆盖它。
    fn label(&self) -> &'static str {
        "input-source(unlabelled)"
    }
}

/// ⭐ **静音源**（⭐ 任务单 `#88` 起：产品的**显式**静音腿，⛔ 不再是默认）。
///
/// ⚠ 它**必须**活着，理由有两条（第 244 条的口径）：
/// ① 判据需要一条"控制臂"（喂静音 ⇒ 落盘必须全 0）来证明非静音判据不是"有文件就算过"；
/// ② 产品需要一条**显式**选静音的路（[`crate::record_device::RECORD_SOURCE_ENV`]）——
/// 于是"录到静音"永远是**有人明确选的**，⛔ 不是"没人喂"的偶然。
#[derive(Debug, Clone, Copy, Default)]
pub struct SilenceSource {
    /// 声道数。
    pub channels: u16,
}

impl InputSource for SilenceSource {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let want = frames.saturating_mul(usize::from(self.channels.max(1)));
        let fill = want.min(out.len());
        out[..fill].fill(0.0);
        fill / usize::from(self.channels.max(1))
    }

    fn label(&self) -> &'static str {
        crate::record_device::SilenceFactory::LABEL
    }
}

/// ⭐ **确定性斜坡源**（⭐ 判据用）：`sample[frame] = (frame - half) * step`，**居中**。
///
/// ⚠ 与 `record_frames` 的判据**同一口径**（同一个居中斜坡公式）⇒ 两条判据可比。
#[derive(Debug, Clone, Copy)]
pub struct RampSource {
    /// 声道数。
    pub channels: u16,
    /// 归一化步长（⭐ 管道吃 `[-1, 1]`）。
    pub step: f32,
    /// 已产出的**帧**数（⭐ 让斜坡可复算：判据按同一个公式算预期）。
    pub produced: u64,
}

impl InputSource for RampSource {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let channels = usize::from(self.channels.max(1));
        let want = frames.saturating_mul(channels);
        let fill = want.min(out.len());
        let whole = fill / channels;
        for index in 0..whole {
            let frame = self.produced + index as u64;
            let value = frame as f32 * self.step;
            out[index * channels..(index + 1) * channels].fill(value);
        }
        self.produced += whole as u64;
        whole
    }

    fn label(&self) -> &'static str {
        "injected-ramp-source"
    }
}

/// ⭐ **预备拍长度（帧）** —— ⛔ 不靠墙钟，**算出**：
/// `bars × 拍号分子 × (60 ÷ bpm) × 采样率`。
///
/// ⭐ 确定性 ⇒ 判据可**本地**断言它（见 `count_in_tests`）。`bpm` 非有限或非正 ⇒ 退回 120
/// （⛔ 不 panic、⛔ 不产出荒谬值）。
#[must_use]
pub fn count_in_frames(project: &yeban_model::YebanProjectV1, sample_rate: u32) -> u64 {
    let bars = u64::from(project.transport.count_in_bars);
    if bars == 0 {
        return 0;
    }
    let beats = bars.saturating_mul(u64::from(project.time_signature.numerator));
    let bpm = if project.bpm.is_finite() && project.bpm > 0.0 {
        project.bpm
    } else {
        120.0
    };
    let seconds = (beats as f64) * 60.0 / bpm;
    (seconds * f64::from(sample_rate)) as u64
}

/// ⭐ **一次录音会话**（⭐ 按钮背后的那一件事）。
///
/// ## 形状（⭐ 为什么是"停录时才落盘"）
///
/// ① **起录**：建环（唯一分配点）⇒ 起一条**后台线程**跑 [`RecordDrain`] 把样本写进
///    一个**内存 sink** ⇒ 本类型只留 [`RecordSender`]（⛔ 它才是音频线程能碰的那一半）。
/// ② **推样本**：[`Self::push_block`] 走引擎的 [`push_input_block`]（整帧、⛔ 不阻塞、⛔ 不分配）。
/// ③ **停录**：发停止信号 ⇒ 收尸 ⇒ 用**同一个** [`SampleSink`] 量化 ⇒ **一次**落盘。
///
/// ⚠ **为什么不在录音中直接写文件**：`RecordDrain` 的 sink 是**流式**的，而落盘要走
/// "量化 ＋ `write_plain_wav`"（⛔ 不自写 RIFF 头）。在内存里接住整段再落盘，可以复用
/// [`write_plain_wav`] 这**唯一**的落盘点；代价是这一段录音在内存里（⚠ 长录音要注意）。
///
/// ⚠ **源从哪来**（任务单 `#88` 起）：本类型**自己不做设备 I/O**，它只吃
/// [`InputSource`]。产品的源由 [`crate::record_device::InputSourceFactory`] 给出
/// （默认 = 真设备输入流；`YEBAN_RECORD_SOURCE=silence` = 显式静音腿），
/// ⛔ **没有**"打不开设备就悄悄录静音"这条路径（见 [`Self::start_with_factory`]）。
#[derive(Debug, Default)]
pub struct Recorder {
    /// 当前会话（`None` ⇒ 没在录）。
    session: Option<RecorderSession>,
    /// ⭐ **从设备源拉到的帧数**（⛔ 与"推入环的帧数"不同）：⭐ 预备拍期间**照拉**，故它
    /// **大于 0** 而推入为 0 ⇒ ⭐ "照拉、只不推"这件事**可被观测**（见 `count_in_tests`）。
    last_pulled: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// ⭐ **整帧成功推入的帧数**（任务单 `#88` 修）：**两条生产路共用**同一个计数器 ——
    /// 控制侧 [`Self::push_block`] 与工作线程从**源**拉块的那条路。
    /// ⚠ 修之前它**只**数控制侧 ⇒ 设备源接上后报告行会写 `pushed=0` 而 `frames` 非 0
    /// （自相矛盾的报告，本单顺手钉住）。
    pushed_frames: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// ⭐ **因环满被丢弃的帧数**（本模块的**帧**级口径，同一条路共用）。
    ///
    /// ⚠ 修之前 `stop` 里是**写死的 0**：设备源一旦跑得比抽干快，报告就会说"没丢"。
    dropped_frames: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[derive(Debug)]
struct RecorderSession {
    /// 生产端（⭐ 共享：工人线程从**设备源**拉块喂它，`push_block` 也可直接推）。
    ///
    /// ⚠ `Mutex` 在这里**不违反红线 7**：本类型的推入点跑在**控制侧**（UI 线程 / 后台录音
    /// 线程），⛔ **不是**音频回调。真设备源把回调搬进来时，那条路仍只用
    /// `push_input_block`（它自己不分配、不加锁地写 `rtrb`）。
    sender: std::sync::Arc<std::sync::Mutex<yeban_engine::record::RecordSender>>,
    /// 停止信号（工作线程每轮检查）。
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 工作线程（停录时收尸）。
    worker: Option<std::thread::JoinHandle<SampleSink>>,
    /// 这次录音的落盘路径。
    path: PathBuf,
    /// 建环参数（落盘时要同一组）。
    options: RecordOptions,
    /// ⭐ **这次会话实际的声道数**（= `source.channels()`，⛔ 不是请求值）：切块与 WAV 头都用它。
    channels: u16,
    /// ⭐ 源的**如实**文案（起录时从 `InputSource::label` 取一次 ⇒ 落进报告的 `source=`）。
    source_label: &'static str,
}

impl Recorder {
    /// ⭐ 空会话（⛔ 不起线程）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// ⭐ **从设备源拉到的帧数**（⭐ 含预备拍期间拉的：那一段**只拉不推**）。
    #[must_use]
    pub fn pulled_frames(&self) -> u64 {
        self.last_pulled.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 是否正在录。
    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.session.is_some()
    }

    /// ⭐ **经工厂缝起录**（任务单 `#88`）：源**只能**由
    /// [`crate::record_device::InputSourceFactory`] 给出 —— 于是"产品用什么源"成了
    /// 一个**可注入、可判**的选择，而"打不开"是一条**响亮的失败**。
    ///
    /// ⭐ 这是裁决 §2.2 的"`Recorder` 与 `host.rs` **都**通过它拿源"里 `Recorder` 那一半：
    /// 判据用 `Recorder::start_with_factory` 就能在**不碰 UI** 的前提下验源的选择/失败语义。
    ///
    /// ⛔ **失败时**：不建会话（`is_recording() == false`）、不落盘、不产生任何文件；
    /// 原因原样装进 [`RecordError::Source`]。
    ///
    /// # Errors
    ///
    /// [`RecordError::Source`]（工厂拒绝），以及 [`Self::start`] 的那几条。
    pub fn start_with_factory(
        &mut self,
        path: &Path,
        options: &RecordOptions,
        factory: &dyn crate::record_device::InputSourceFactory,
        count_in_frames: u64,
    ) -> Result<(), RecordError> {
        // ⭐ 工厂**恰好调用一次**（判据会数它）：这一处是产品路径上唯一的 `open` 调用点。
        let source = factory.open(options).map_err(RecordError::Source)?;
        self.start(path, options, source, count_in_frames)
    }

    /// ⭐ **起录**。已有一场会话在手 ⇒ 返回错误（⛔ 不静默覆盖）。
    ///
    /// # Errors
    ///
    /// 参数非法，或**已经在录**。
    pub fn start(
        &mut self,
        path: &Path,
        options: &RecordOptions,
        mut source: Box<dyn InputSource + Send>,
        count_in_frames: u64,
    ) -> Result<(), RecordError> {
        if self.session.is_some() {
            return Err(RecordError::Invalid("已经有一次录音在进行".to_owned()));
        }
        if options.channels == 0 || options.capacity_frames == 0 || options.drain_batch == 0 {
            return Err(RecordError::Invalid(
                "声道数 / 环容量 / 抽取批大小都必须大于 0".to_owned(),
            ));
        }
        // ⭐ **实际声道数以源为准**（任务单 `#88`）：`RecordOptions::channels` 是**请求值**，
        // 而真设备的协商结果可能只有 1 声道（macOS 内建麦克风就是）——
        // 录音环、切块宽度、WAV 头必须**一致地**用同一个数，否则就是声道错位。
        // 用源的值 = "照设备说的录一条单声道"，⛔ 不是拒绝录音、⛔ 不是偷偷上混。
        let channels = source.channels().max(1);
        let (sender, receiver) = record_channel(options.capacity_frames, channels);
        let sender = std::sync::Arc::new(std::sync::Mutex::new(sender));
        let sender_worker = std::sync::Arc::clone(&sender);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_worker = std::sync::Arc::clone(&stop);
        let batch = options.drain_batch.max(1);
        // ⭐ **设备输入块的大小**（帧）：一个 `drain_batch` 那么多 —— 与抽干批同量级，
        // 免得"生产远快于消费"而丢帧。⚠ 它**不是**音频回调的块长（真实现的块长由设备给）。
        let block_frames = batch;
        let mut block = vec![0.0_f32; block_frames.saturating_mul(usize::from(channels))];
        // ⭐ **预备拍计数器**：`Recorder` 与 worker 共享（⭐ 让"预备拍期间**照拉**"可被观测）。
        let pulled = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let pulled_worker = std::sync::Arc::clone(&pulled);
        self.last_pulled = std::sync::Arc::clone(&pulled);
        // ⭐ **两条生产路共用的帧级计数器**（推入 / 丢弃）：工作线程那一半从**源**推。
        let pushed = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let pushed_worker = std::sync::Arc::clone(&pushed);
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let dropped_worker = std::sync::Arc::clone(&dropped);
        self.pushed_frames = std::sync::Arc::clone(&pushed);
        self.dropped_frames = std::sync::Arc::clone(&dropped);
        // ⭐ **如实标签**（在源被 move 进线程**之前**取一次）：它进报告的 `source=` 字段。
        let source_label = source.label();
        let worker = std::thread::spawn(move || {
            let mut drain = RecordDrain::new(receiver, SampleSink::default());
            let sender = sender_worker;
            // ⭐ **已经开始录了没有**：预备拍走完之前为 `false`。
            let mut recording = count_in_frames == 0;
            let mut pulled_so_far: u64 = 0;
            // ⚠ 轮询而非阻塞等待：`RecordDrain` 没有"等新数据"的接口，而停止必须**及时**
            // （⛔ 不能让按钮卡住）。1 ms 的休眠只在**后台线程**上，⛔ 不碰音频线程。
            while !stop_worker.load(std::sync::atomic::Ordering::Relaxed) {
                // ⭐ **从设备输入源拉一块**（⭐ 这就是"谁在喂它"的答案）⇒ 整帧推进环。
                // ⚠ 真设备源（`crate::record_device::DeviceInputSource`）的这一拉发生在
                // **本后台线程**上：设备回调只做无锁推入，⛔ 不在这一层分配/加锁。
                let frames = source.next_block(block_frames, &mut block);
                if frames > 0 {
                    // ⭐ ⭐ **照拉、只不推**：预备拍期间**照常**从源拉块（⛔ 不能停止拉取，
                    // 否则真设备的回调会积压、甚至丢帧/溢出 —— 那会把"不录预备拍"变成
                    // "录音整体出问题"），但**⛔ 不推入录音环** ⇒ **不产生任何录音内容**。
                    pulled_so_far = pulled_so_far.saturating_add(frames as u64);
                    pulled_worker.store(pulled_so_far, std::sync::atomic::Ordering::Relaxed);
                    if !recording && pulled_so_far >= count_in_frames {
                        // ⭐ 预备拍走完 ⇒ **从这一块起**开始录（⛔ 不追录预备拍期间的块）。
                        recording = true;
                    }
                    if recording && let Ok(mut sender) = sender.lock() {
                        let samples = frames.saturating_mul(usize::from(channels));
                        let accepted = yeban_engine::record::push_input_block(
                            &mut sender,
                            &block[..samples],
                            channels,
                        );
                        // ⭐ 帧级如实计数（⭐ 与 `push_block` 那一条路共用同一对计数器）。
                        pushed_worker
                            .fetch_add(accepted as u64, std::sync::atomic::Ordering::Relaxed);
                        dropped_worker.fetch_add(
                            frames.saturating_sub(accepted) as u64,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                    }
                }
                let _ = drain.pump(batch);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            // ⭐ 收尾：把环里剩下的**全部**抽干（⛔ 不丢最后一截）。
            loop {
                match drain.pump(batch) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            drain.into_inner()
        });
        self.session = Some(RecorderSession {
            sender,
            stop,
            worker: Some(worker),
            path: path.to_path_buf(),
            options: *options,
            channels,
            // ⭐ **如实**取自源自己（⛔ 不再是一条写死的"no-device-input"文案）。
            source_label,
        });
        Ok(())
    }

    /// ⭐ **推一块交错样本**（音频线程那一半）。返回被接受的**帧**数。
    ///
    /// ⚠ 没在录 ⇒ 返回 0（⛔ 不 panic、⛔ 不新建会话）。
    pub fn push_block(&mut self, block: &[f32]) -> usize {
        let Some(session) = self.session.as_mut() else {
            return 0;
        };
        let channels = session.channels.max(1);
        let Ok(mut sender) = session.sender.lock() else {
            return 0;
        };
        let accepted = yeban_engine::record::push_input_block(&mut sender, block, channels);
        // ⭐ 与工作线程**同一对**计数器（见 [`Recorder`] 的字段文档）。
        self.pushed_frames
            .fetch_add(accepted as u64, std::sync::atomic::Ordering::Relaxed);
        let attempted = block.len() / usize::from(channels);
        self.dropped_frames.fetch_add(
            attempted.saturating_sub(accepted) as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        accepted
    }

    /// ⭐ **停录并落盘**。返回报告；⛔ 没在录 ⇒ 返回错误（⛔ 不假装成功）。
    ///
    /// # Errors
    ///
    /// 没在录，或抽干 / 落盘失败。
    pub fn stop(&mut self) -> Result<RecordReport, RecordError> {
        let mut session = self
            .session
            .take()
            .ok_or_else(|| RecordError::Invalid("没有正在进行的录音".to_owned()))?;
        // ① 先发停止信号，再收尸（⛔ 顺序反了会让工作线程永远等下去）。
        session
            .stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let sink = match session.worker.take() {
            Some(handle) => handle
                .join()
                .map_err(|_| RecordError::Invalid("录音线程 panic".to_owned()))?,
            None => SampleSink::default(),
        };
        // ② 量化 ＋ **一次**落盘（与 `record_test_tone` 同一个落盘点与同一个抖动种子）。
        let mut rng = DeterministicDitherRng::new(1);
        let quantized: Vec<i32> = sink
            .samples
            .iter()
            .map(|sample| quantize_i24(*sample, &mut rng))
            .collect();
        // ⭐ 帧数与 WAV 头都用**这次会话实际的**声道数（`session.channels` = 源协商值）。
        let frames = u64::try_from(sink.samples.len() / usize::from(session.channels.max(1)))
            .unwrap_or(u64::MAX);
        let buffer = PcmBuffer::Int24(quantized);
        let format = PcmFormat::integer(session.channels, session.options.sample_rate, 24);
        write_plain_wav(&session.path, &format, &buffer)
            .map_err(|error| RecordError::Write(error.to_string()))?;
        let bytes = std::fs::metadata(&session.path)
            .map_err(|error| RecordError::Io(error.to_string()))?
            .len();
        Ok(RecordReport {
            path: session.path.clone(),
            frames,
            // ⭐ **如实**（任务单 `#88`）：两条生产路（控制侧 `push_block` / 工作线程从源拉块）
            // 共用的帧级计数器 —— ⛔ 原来 `pushed` 只数控制侧（设备源接上后会写 0），
            // `dropped` 更是**写死的 0**。
            pushed: self
                .pushed_frames
                .load(std::sync::atomic::Ordering::Relaxed),
            dropped: self
                .dropped_frames
                .load(std::sync::atomic::Ordering::Relaxed),
            bytes,
            bits: 24,
            // ⭐ **如实**取自源自己的标签（任务单 `#88`）：真设备源报 `device-input-stream`、
            // 显式静音腿报 `explicit-silence-source`、判据的假源报它自己写的那一条。
            // ⛔ 原来这里写死 `ui-record-button(no-device-input)` —— 设备源接上之后
            // 那句话就**成了假话**（报告行是"这条录音从哪来"的唯一出处）。
            source: session.source_label.to_owned(),
        })
    }
}

/// ⭐ **录一段由调用方给出的帧源**（`[ARCH-REC-001]` 的"可注入源"入口）。
///
/// ⭐ 为什么是**泛型闭包**（⛔ 不是枚举、⛔ 不是 `Box<dyn FnMut>`）：注入源有两个真实用途 ——
/// ① 判据要注入**确定性斜坡**去逐样本比对；② 将来的**真声卡输入**要按块喂进来。
/// 两者都只需要"给帧号 ⇒ 给一个归一化值"，泛型把它做在**编译期**（⛔ 不付一次分配）。
///
/// ⚠ **与 [`record_test_tone`] 是同一条管道**（建环 ⇒ 整帧推送 ＋ 交错抽干 ⇒ 自己的
/// `SampleSink` ⇒ 同一量化 ＋ 同一次落盘）⇒ ⛔ **不是**判据里另写的一条旁路。
///
/// # Errors
///
/// 与 [`record_test_tone`] 同一组（参数非法 / 抽干 I/O 失败 / WAV 写出被拒）。
pub fn record_frames<F>(
    path: &Path,
    options: &RecordOptions,
    mut frame_value: F,
) -> Result<RecordReport, RecordError>
where
    F: FnMut(u64) -> f32,
{
    if options.seconds <= 0.0 {
        return Err(RecordError::Invalid("录制秒数必须大于 0".to_owned()));
    }
    if options.channels == 0 || options.capacity_frames == 0 || options.drain_batch == 0 {
        return Err(RecordError::Invalid(
            "声道数 / 环容量 / 抽取批大小都必须大于 0".to_owned(),
        ));
    }
    let total_frames = (options.seconds * options.sample_rate as f32) as u64;
    let (mut sender, receiver) = record_channel(options.capacity_frames, options.channels);
    let mut drain = RecordDrain::new(receiver, SampleSink::default());
    let mut frame = vec![0.0_f32; usize::from(options.channels)];
    for index in 0..total_frames {
        let value = frame_value(index);
        frame.fill(value);
        let _ = sender.push_frame(&frame);
        if (index + 1) % u64::try_from(options.drain_batch.max(1)).unwrap_or(1) == 0 {
            while drain
                .pump(options.drain_batch)
                .map_err(|error| RecordError::Io(error.to_string()))?
                > 0
            {}
        }
    }
    let pushed = sender.pushed_frames();
    let dropped = sender.dropped_frames();
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
        source: "injected-frames".to_owned(),
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
        // `--record-wav` 的位深**不在本切片的选择面里**：钉住 24-bit（旧行为），
        // 于是"录制"的中间渲染仍与 `--export-wav` 的缺省同一格式。
        bit_depth: yeban_render::dither::BitDepth::Int24,
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

#[cfg(test)]
mod injected_source_tests {
    use super::*;

    /// i24 满标（读回是归一化 [-1, 1]，i24 值除以此数）。
    const FULL_SCALE: f32 = 8_388_608.0;
    /// ⚠ 容差 = **2 个 i24 LSB**：管道**故意**带 TPDF 抖动（每样本 ±1 LSB）⇒ 不引入抖动就
    /// 不可能逐样本比对；2 LSB 是"抖动不可能超过"的上界，⛔ 不是放松到看不出错。
    const TOLERANCE_LSB: f32 = 2.0;

    /// ⭐ 判据用目录（`target/` 已 gitignore ⇒ ⛔ 不往系统目录写）。
    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/mod-app-record-injected");
        std::fs::create_dir_all(&dir).expect("建判据目录");
        dir.join(format!("{name}-{}.wav", std::process::id()))
    }

    fn options() -> RecordOptions {
        RecordOptions {
            seconds: 0.02,
            sample_rate: 48_000,
            channels: 2,
            capacity_frames: 4096,
            drain_batch: 512,
            tone_hz: TEST_TONE_HZ,
        }
    }

    /// ⭐ **判据 ①**：注入**确定性斜坡** ⇒ 落盘后**每一个**读回样本都等于**独立算出的预期值**
    /// （± 抖动上限），且**相邻帧的差**就是斜坡的步长（± 抖动上限）。
    ///
    /// ⚠ 读回是**单声道**：`read_plain_wav_mono_f32` 按 `step_by(channels)` 取**第 0 声道**
    /// （`crates/yeban-render/src/wav.rs` 的文档口径）⇒ 一个读回样本 = 一个**帧**。
    ///
    /// 读法：① "逐样本"证明**没有错位、没有丢帧、没有换源**；② "相邻帧差"证明**幅度尺度**
    /// 也对（⛔ 不是"文件非空"、⛔ 不是"长度对得上"）。
    #[test]
    fn recording_an_injected_ramp_reads_back_sample_by_sample() {
        let opts = options();
        let total_frames = (opts.seconds * opts.sample_rate as f32) as u64;
        // ⚠ **归一化步长**（管道吃的是 [-1, 1] 的值，⛔ 不是 LSB）：整段用掉 ±0.5 全幅
        // ⇒ 台阶清晰（每帧约 4369 LSB）、且**不越界**。
        // 早期试验版把这里的 `step` 写成 LSB 单位 ⇒ 全被钳到 ±1，实测踩过。
        let step = 0.5_f32 / total_frames as f32;
        // ⚠ 斜坡**居中**：`value = (frame - total/2) * step` ⇒ 首尾关于 0 对称。
        let half = total_frames as f64 / 2.0;
        let expected_of = |frame: u64| (frame as f64 - half) * f64::from(step);

        let path = scratch("ramp");
        let report = record_frames(&path, &opts, |frame| expected_of(frame) as f32)
            .expect("注入斜坡必须能落盘");
        assert_eq!(report.frames, total_frames, "写出的帧数必须与请求一致");
        assert_eq!(report.dropped, 0, "环够大 ⇒ 不许丢帧");
        assert_eq!(report.source, "injected-frames");

        let (samples, sample_rate, encoding) =
            yeban_render::wav::read_plain_wav_mono_f32(&path).expect("读回");
        assert_eq!(sample_rate, opts.sample_rate);
        assert!(
            encoding.contains(&format!("frames={total_frames}")),
            "读回描述必须点名帧数: {encoding}"
        );
        assert_eq!(
            samples.len() as u64,
            total_frames,
            "单声道读回 ⇒ 样本数 = 帧数（⛔ 不许少）"
        );

        // ① 逐样本比对（⛔ 不是抽查）。
        let mut worst = 0.0_f32;
        for (frame, got) in samples.iter().enumerate() {
            let want = expected_of(frame as u64) as f32;
            let diff = (got - want).abs() * FULL_SCALE;
            worst = worst.max(diff);
            assert!(
                diff <= TOLERANCE_LSB,
                "第 {frame} 帧偏离预期 {diff} 个 LSB（> {TOLERANCE_LSB}）"
            );
        }
        assert!(worst > 0.0, "抖动必须真的动过某个样本（否则容差没意义）");

        // ② 相邻帧的差 = 步长（± 抖动上限）⇒ 幅度尺度正确。
        // 步长的 **LSB** 表示（读回与预期都在归一化域，差要乘满标才成 LSB）。
        let want_delta = f64::from(step) * f64::from(FULL_SCALE);
        for frame in 100..(samples.len() - 100) {
            let delta =
                f64::from((samples[frame + 1] - samples[frame]).abs()) * f64::from(FULL_SCALE);
            assert!(
                (delta - want_delta).abs() <= f64::from(TOLERANCE_LSB),
                "第 {frame} 帧的增量 {delta} 必须约等于步长 {want_delta}"
            );
        }
    }

    /// ⭐ **判据 ②（控制臂）**：⛔ **不推进样本**（源恒为 0）⇒ 读回的必须是**全静音**
    /// （每个样本 |值| ≤ 抖动上限）。
    ///
    /// 没有这一条，判据 ① 的"逐样本对得上"可能只是**管道根本没在写**而读回的恰好也匹配。
    #[test]
    fn recording_without_injected_signal_reads_back_silence() {
        let opts = options();
        let path = scratch("silence");
        let report = record_frames(&path, &opts, |_frame| 0.0).expect("零源必须能落盘");
        let (samples, _rate, _enc) =
            yeban_render::wav::read_plain_wav_mono_f32(&path).expect("读回");
        assert_eq!(samples.len() as u64, report.frames);
        assert!(!samples.is_empty(), "控制臂必须真的产出一个文件");
        let peak = samples.iter().fold(0.0_f32, |max, s| max.max(s.abs())) * FULL_SCALE;
        assert!(
            peak <= TOLERANCE_LSB,
            "零注入 ⇒ 读回必须全静音（|峰值| ≤ 抖动上限），实测峰值 {peak} 个 LSB"
        );
    }
}

#[cfg(test)]
mod device_input_tests {
    use super::*;

    /// i24 满标（读回是归一化 `[-1, 1]`）。
    const FULL_SCALE: f32 = 8_388_608.0;
    /// ⚠ 与录音片同一条口径：管道**故意**带 TPDF 抖动（±1 LSB）⇒ 容差 2 LSB 是
    /// "抖动不可能超过"的上界，⛔ 不是放松到看不出错。
    const TOLERANCE_LSB: f32 = 2.0;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/mod-app-device-input");
        std::fs::create_dir_all(&dir).expect("建判据目录");
        dir.join(format!("{name}-{}.wav", std::process::id()))
    }

    fn options() -> RecordOptions {
        RecordOptions {
            seconds: 0.05,
            sample_rate: 48_000,
            channels: 2,
            capacity_frames: 48_000,
            drain_batch: 256,
            tone_hz: TEST_TONE_HZ,
        }
    }

    /// ⭐ **判据 ①（正对照）**：把**设备输入源**接成确定性斜坡 ⇒ 落盘后**每一个**读回样本
    /// 都等于那条斜坡（± 抖动上限）。
    ///
    /// ⚠ 走的是**产品路径**：`Recorder::start`（真 `RecordDrain` 线程）⇒ 停止 ⇒ 落盘
    /// ⇒ `read_plain_wav_mono_f32`（单声道读回 = 每帧一个样本）。
    #[test]
    fn device_input_ramp_is_recorded_sample_by_sample() {
        let opts = options();
        let step = 0.25 / 48_000.0;
        let path = scratch("ramp");
        let _ = std::fs::remove_file(&path);

        let mut recorder = Recorder::new();
        recorder
            .start(
                &path,
                &opts,
                Box::new(RampSource {
                    channels: opts.channels,
                    step,
                    produced: 0,
                }),
                0,
            )
            .expect("起录");
        std::thread::sleep(std::time::Duration::from_millis(400));
        let report = recorder.stop().expect("停录并落盘");

        let (samples, rate, _desc) =
            yeban_render::wav::read_plain_wav_mono_f32(&path).expect("读回 WAV");
        assert_eq!(rate, opts.sample_rate);
        assert!(
            samples.len() as u64 > 100,
            "必须真的录到一批样本（实测 {}）",
            samples.len()
        );
        assert_eq!(
            samples.len() as u64,
            report.frames,
            "读回样本数必须等于报告的帧数（单声道读回）"
        );

        // ⭐ **逐样本**比对（⛔ 不是"文件非空"、⛔ 不是"长度对得上"）。
        let mut worst = 0.0_f32;
        for (frame, got) in samples.iter().enumerate() {
            let want = frame as f32 * step;
            let diff = (got - want).abs() * FULL_SCALE;
            worst = worst.max(diff);
            assert!(
                diff <= TOLERANCE_LSB,
                "第 {frame} 帧偏离设备源的斜坡 {diff} 个 LSB（> {TOLERANCE_LSB}）"
            );
        }
        assert!(worst > 0.0, "抖动必须真的动过某个样本（否则容差没意义）");
        // ⭐ 幅度尺度也对：相邻帧的差就是斜坡步长（± 抖动上限）。
        let want_delta = step * FULL_SCALE;
        // ⚠ 容差是 **2 ×** 逐样本容差：相邻两帧各带 ±1 LSB 的**独立**抖动 ⇒
        // 差的误差上界是 4 LSB（不是 2）。实测踩过（第 397 帧差 2.3 LSB 被判红）。
        let delta_tolerance = TOLERANCE_LSB * 2.0;
        for frame in 10..(samples.len() - 10) {
            let delta = (samples[frame + 1] - samples[frame]).abs() * FULL_SCALE;
            assert!(
                (delta - want_delta).abs() <= delta_tolerance,
                "第 {frame} 帧的增量 {delta} 必须约等于步长 {want_delta}"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    /// ⭐ **判据 ②（控制臂）**：把源接成**静音** ⇒ 落盘必须**全 0**。
    ///
    /// 没有这一条，判据 ① 可能只是"管道把噪声写成了斜坡"而恰好对上。
    #[test]
    fn device_input_silence_is_recorded_as_zero() {
        let opts = options();
        let path = scratch("silence");
        let _ = std::fs::remove_file(&path);

        let mut recorder = Recorder::new();
        recorder
            .start(
                &path,
                &opts,
                Box::new(SilenceSource {
                    channels: opts.channels,
                }),
                0,
            )
            .expect("起录");
        std::thread::sleep(std::time::Duration::from_millis(400));
        let report = recorder.stop().expect("停录并落盘");

        let (samples, _rate, _desc) =
            yeban_render::wav::read_plain_wav_mono_f32(&path).expect("读回 WAV");
        assert!(!samples.is_empty(), "控制臂必须真的录到一批样本");
        assert_eq!(samples.len() as u64, report.frames);
        let peak = samples.iter().fold(0.0_f32, |max, s| max.max(s.abs())) * FULL_SCALE;
        assert!(
            peak <= TOLERANCE_LSB,
            "静音源 ⇒ 读回必须全 0（|峰值| ≤ 抖动上限），实测峰值 {peak} 个 LSB"
        );
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod count_in_tests {
    use super::*;

    fn opts() -> RecordOptions {
        RecordOptions {
            seconds: 0.05,
            sample_rate: 48_000,
            channels: 2,
            capacity_frames: 480_000,
            drain_batch: 256,
            tone_hz: TEST_TONE_HZ,
        }
    }

    fn project_with_bars(bars: u8) -> yeban_model::YebanProjectV1 {
        let mut project = yeban_model::samples::filled_project();
        project.transport.count_in_bars = bars;
        project.bpm = 120.0;
        project.time_signature.numerator = 4;
        project
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/mod-app-count-in");
        std::fs::create_dir_all(&dir).expect("建判据目录");
        dir.join(format!("{name}-{}.wav", std::process::id()))
    }

    fn ramp() -> Box<dyn InputSource + Send> {
        Box::new(RampSource {
            channels: 2,
            step: 0.0001,
            produced: 0,
        })
    }

    /// ⭐ **判据 ①（纯函数、确定性）**：预备拍长度**算出**，⛔ 不靠墙钟。
    ///
    /// 1 小节 4/4 @120bpm ⇒ 4 拍 × 0.5 s = **2 s** ⇒ 48kHz 下 **96_000 帧**。
    #[test]
    fn count_in_frames_is_computed_from_the_project() {
        assert_eq!(
            count_in_frames(&project_with_bars(0), 48_000),
            0,
            "0 小节 ⇒ 0"
        );
        assert_eq!(
            count_in_frames(&project_with_bars(1), 48_000),
            96_000,
            "1 小节 4/4 @120bpm @48k ⇒ 96_000 帧"
        );
        assert_eq!(
            count_in_frames(&project_with_bars(2), 48_000),
            192_000,
            "2 小节 ⇒ 翻倍（线性）"
        );
        // ⚠ 非有限 / 非正 bpm ⇒ 退回 120（⛔ 不 panic、⛔ 不产出荒谬值）。
        let mut bad = project_with_bars(1);
        bad.bpm = 0.0;
        assert_eq!(count_in_frames(&bad, 48_000), 96_000, "bpm=0 ⇒ 退回 120");
    }

    /// ⭐ **判据 ②**：⭐ **预备拍期间⛔ 不产生录音内容** —— 且 ⭐ ⭐ **源仍被拉取**（"照拉、只不推"）。
    #[test]
    fn nothing_is_recorded_during_the_count_in_but_the_source_is_still_pulled() {
        let path = scratch("during");
        let _ = std::fs::remove_file(&path);
        let mut recorder = Recorder::new();
        // 预备拍长到本判据期间**走不完**（10 分钟）⇒ 全程处于预备拍。
        recorder
            .start(&path, &opts(), ramp(), 48_000 * 600)
            .expect("起录");
        std::thread::sleep(std::time::Duration::from_millis(300));

        let pulled = recorder.pulled_frames();
        assert!(
            pulled > 0,
            "⭐ “照拉、只不推”：预备拍期间**源必须仍被拉取**（实测 pulled={pulled}）"
        );
        let report = recorder.stop().expect("停录");
        assert_eq!(
            report.frames, 0,
            "⭐ 判据 ②：预备拍期间⛔ 不许产生任何录音内容（实测 {} 帧）",
            report.frames
        );
        assert_eq!(report.pushed, 0, "⛔ 不许推入任何帧");
        let _ = std::fs::remove_file(&path);
    }

    /// ⭐ **判据 ③**：预备拍**走完之后**按预期开始录。
    #[test]
    fn recording_starts_after_the_count_in_elapses() {
        let path = scratch("after");
        let _ = std::fs::remove_file(&path);
        let mut recorder = Recorder::new();
        // 预备拍很短（256 帧 ≈ 5.3 ms）⇒ 300 ms 之后必然已经在录。
        recorder.start(&path, &opts(), ramp(), 256).expect("起录");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let report = recorder.stop().expect("停录");
        assert!(
            report.frames > 0,
            "⭐ 判据 ③：预备拍走完后必须**真的开始录**（实测 {} 帧）",
            report.frames
        );
        let _ = std::fs::remove_file(&path);
    }

    /// ⭐ **判据 ④（控制臂，独立判据、走产品路径）**：⭐ **`count_in_bars = 0` ⇒ 立即开始录**。
    ///
    /// ⚠ 单独一条（第 46/47 条）：没有它，判据 ② 的"0 帧"可能只是**别的原因**（例如源没喂上）。
    #[test]
    fn without_a_count_in_recording_starts_immediately() {
        let path = scratch("nocountin");
        let _ = std::fs::remove_file(&path);
        let mut recorder = Recorder::new();
        recorder.start(&path, &opts(), ramp(), 0).expect("起录");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let report = recorder.stop().expect("停录");
        assert!(
            report.frames > 0,
            "⭐ 控制臂：预备拍=0 ⇒ 必须**立即**开始录（实测 {} 帧）⇒ 判据 ② 的 0 帧是**因为预备拍**",
            report.frames
        );
        // ⚠ 同一条控制臂里的"纯函数"那一半：工程里 `count_in_bars = 0` ⇒ 算出的长度是 0。
        assert_eq!(
            count_in_frames(&project_with_bars(0), opts().sample_rate),
            0,
            "控制臂：工程 0 小节 ⇒ 长度 0"
        );
        let _ = std::fs::remove_file(&path);
    }
}
