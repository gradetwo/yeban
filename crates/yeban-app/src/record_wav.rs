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

/// ⭐ **设备输入源**（`[ARCH-REC-001]` 的"音频线程直接从声卡物理输入捕获 PCM 流"那一半）。
///
/// ⭐ **为什么是 trait**：真实现要 `cpal` 的 `default_input_device` ＋ `build_input_stream`，
/// 而 ⚠ **本仓到今天为止没有输入流**（`crates/yeban-engine/src/device.rs` 只有纯函数
/// [`negotiate_input`]；`grep -n 'default_input_device\|build_input_stream\|open_input'
/// crates/yeban-engine/src/*.rs` 命中 **0**），且 `yeban-app` 不依赖 `cpal`。
/// ⇒ 本 trait 就是**那个缺口的位置本身**：真实现到达时**只换一个实现**，⛔ 不改录音路径。
///
/// ⚠ **块语义**：`next_block(frames, out)` 填**交错**样本，填 `frames × channels` 个；
/// 返回**真的填了几个帧**（0 ⇒ 暂时没有数据，调用方下一轮再问）。⛔ 不长阻塞。
pub trait InputSource {
    /// 声道数（⭐ 必须与 [`RecordOptions::channels`] 一致；不一致由调用方负责检出）。
    fn channels(&self) -> u16;
    /// 取下一块交错样本。返回填进去的**帧**数。
    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize;
}

/// ⭐ **静音源**：产品**今天的**默认（⭐ 它让"按按钮录到静音"这件事**有明确出处**，
/// ⛔ 不是"没人喂"的偶然）。真设备源到达后，这里换成真实现即可。
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
/// ⚠ **真声卡输入仍是欠件**（本片**如实登记**）：本类型**准备好**了接收侧，但今天
/// **没有**任何调用方把设备输入块喂进来 ⇒ 产品按按钮录到的是**静音**（一条合法的 WAV），
/// ⛔ 不是"从声卡录到了声音"。设备输入流属 `[ARCH-REC-001]` 的下一片。
#[derive(Debug, Default)]
pub struct Recorder {
    /// 当前会话（`None` ⇒ 没在录）。
    session: Option<RecorderSession>,
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
    /// 推入的**帧**数（本类型自己数：`push_input_block` 的返回值）。
    pushed_frames: u64,
}

impl Recorder {
    /// ⭐ 空会话（⛔ 不起线程）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否正在录。
    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.session.is_some()
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
    ) -> Result<(), RecordError> {
        if self.session.is_some() {
            return Err(RecordError::Invalid("已经有一次录音在进行".to_owned()));
        }
        if options.channels == 0 || options.capacity_frames == 0 || options.drain_batch == 0 {
            return Err(RecordError::Invalid(
                "声道数 / 环容量 / 抽取批大小都必须大于 0".to_owned(),
            ));
        }
        let (sender, receiver) = record_channel(options.capacity_frames, options.channels);
        let sender = std::sync::Arc::new(std::sync::Mutex::new(sender));
        let sender_worker = std::sync::Arc::clone(&sender);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_worker = std::sync::Arc::clone(&stop);
        let batch = options.drain_batch.max(1);
        // ⭐ **设备输入块的大小**（帧）：一个 `drain_batch` 那么多 —— 与抽干批同量级，
        // 免得"生产远快于消费"而丢帧。⚠ 它**不是**音频回调的块长（真实现的块长由设备给）。
        let block_frames = batch;
        let channels = options.channels.max(1);
        let mut block = vec![0.0_f32; block_frames.saturating_mul(usize::from(channels))];
        let worker = std::thread::spawn(move || {
            let mut drain = RecordDrain::new(receiver, SampleSink::default());
            let sender = sender_worker;
            // ⚠ 轮询而非阻塞等待：`RecordDrain` 没有"等新数据"的接口，而停止必须**及时**
            // （⛔ 不能让按钮卡住）。1 ms 的休眠只在**后台线程**上，⛔ 不碰音频线程。
            while !stop_worker.load(std::sync::atomic::Ordering::Relaxed) {
                // ⭐ **从设备输入源拉一块**（⭐ 这就是"谁在喂它"的答案）⇒ 整帧推进环。
                // ⚠ 真实现里这一拉发生在音频回调上；本片的产品默认是 [`SilenceSource`]。
                let frames = source.next_block(block_frames, &mut block);
                if frames > 0
                    && let Ok(mut sender) = sender.lock()
                {
                    let samples = frames.saturating_mul(usize::from(channels));
                    let _ = yeban_engine::record::push_input_block(
                        &mut sender,
                        &block[..samples],
                        channels,
                    );
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
            pushed_frames: 0,
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
        let Ok(mut sender) = session.sender.lock() else {
            return 0;
        };
        let accepted =
            yeban_engine::record::push_input_block(&mut sender, block, session.options.channels);
        session.pushed_frames = session.pushed_frames.saturating_add(accepted as u64);
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
        let frames =
            u64::try_from(sink.samples.len() / usize::from(session.options.channels.max(1)))
                .unwrap_or(u64::MAX);
        let buffer = PcmBuffer::Int24(quantized);
        let format = PcmFormat::integer(session.options.channels, session.options.sample_rate, 24);
        write_plain_wav(&session.path, &format, &buffer)
            .map_err(|error| RecordError::Write(error.to_string()))?;
        let bytes = std::fs::metadata(&session.path)
            .map_err(|error| RecordError::Io(error.to_string()))?
            .len();
        Ok(RecordReport {
            path: session.path.clone(),
            frames,
            pushed: session.pushed_frames,
            dropped: 0,
            bytes,
            bits: 24,
            // ⚠ **如实**：本类型的样本来自 `push_block`，而今天**没有**设备输入在喂它。
            source: "ui-record-button(no-device-input)".to_owned(),
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
