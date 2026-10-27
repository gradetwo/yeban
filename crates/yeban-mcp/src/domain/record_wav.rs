//! `yeban_record_wav` 的实现：把 `ARCH-REC-001` 的**三段**接成一条 MCP 工具。
//!
//! ## 规范要求（逐字要点）
//!
//! `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:756`：
//! 「音频线程直接从声卡物理输入捕获 PCM 流并写入**预分配的无锁环形缓冲区**，
//! 由**后台 I/O 线程**刷盘至 CAS 临时暂存」。
//!
//! 本模块实现前两段（与 `crates/yeban-app/src/record_wav.rs` **同一条装配**）：
//!
//! ```text
//! ① 构造期（唯一分配点）: record_channel(capacity_frames, channels) -> (sender, receiver)
//! ② 生产端（红线 7 安全）: 先问 sender.spare_frames() >= 1，再 push_frame(&frame)
//!                          环满 ⇒ 丢弃这一帧并**自己**计数（⛔ 不阻塞、⛔ 不 panic）
//! ③ 消费端: RecordDrain::new(receiver, SampleSink) ⇒ pump(batch)
//!          ⇒ 量化 24-bit（TPDF，种子 1）⇒ 容器字节 ⇒ 一次原子落盘
//! ```
//!
//! ## 五条诚实登记（⛔ 不假装）
//!
//! 1. **不碰声卡**：设备输入流不在本切片（本 crate 依赖的是**只带 `rtrb` 的**
//!    `yeban-record`，⛔ 不是带 `cpal` 的 `yeban-engine`；见 `Cargo.toml` 的注释与
//!    `cargo tree -p yeban-mcp -e normal | grep -c cpal == 0` 的读数）。
//!    样本来源是一条**可控的内存测试音**
//!    （440 Hz 正弦），响应 `source` 逐字写明 `in-memory-sine(440Hz)`。
//!    ⭐ [G12] 名字容易让人以为在"录工程"：响应与预览因此恒带
//!    `notProjectBounce: true` 与 `alternatives.projectBounce`（指路
//!    `yeban_render_master`）。接工程源本身是待裁项（HD-69），不在本切片擅自接线。
//! 2. **丢弃是**帧**级**：引擎的 `RecordSender::dropped_frames()` 是**样本**级
//!    （它的文档写明这一点），本模块按**整帧原子**推送并自己数**帧**级丢弃 ⇒
//!    响应里的 `dropped` 是帧级读数，⛔ 不与引擎的样本级读数混用。
//! 3. **生产与消费交错**：与 app 侧同一个教训 —— "先全推、再抽干"会让一条 1 秒的环
//!    在 8 秒请求下只留下 1 秒（实测 `frames=48000 / dropped=336000`）。
//! 4. **`seconds` 是请求时长**：实际写出帧数由环与抽干决定，响应把 `framesWritten` /
//!    `pushed` / `dropped` 三个数**分别**给出 ⇒ 调用方看得见"请求"与"实得"的差。
//! 5. **交付形态是普通 RIFF WAV（无 `bext`）**：与 `yeban_render_sfz_wav` 同一条
//!    容器写入器（`encode_container_without_bext`）；因此**没有** BWF 元数据，
//!    也**没有**时间戳 ⇒ 同一请求逐字节可复现。
//!
//! ## 与 `yeban_import_audio` 的分工
//!
//! `import_audio` 把一个**已存在的**音频文件**登记**成 `clip_pool` 条目（它是"素材入库"）；
//! 本工具**产生**一段 PCM 并落成 WAV（它是"采集"）。两者都不做对方的事。

use std::io::Write;
use std::path::{Path, PathBuf};

use yeban_model::AssetHash;
use yeban_record::{RecordDrain, record_channel};

use super::error::Fault;
use super::render;
use super::render_math as math;
use crate::tools::ErrorCode;

/// 默认录制秒数（与 app 侧 `RecordOptions::default().seconds` 同值）。
pub const DEFAULT_SECONDS: f64 = 2.0;

/// 默认采样率（Hz）。
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// 默认声道数。
pub const DEFAULT_CHANNELS: u16 = 2;

/// 默认测试音频率（Hz）。
pub const DEFAULT_TONE_HZ: f32 = 440.0;

/// 测试音的默认幅度（留足余量，⛔ 不削波）——与 app 侧同值。
pub const TONE_AMPLITUDE: f32 = 0.5;

/// 默认环容量（毫秒）⇒ 实际帧数由采样率换算（默认 1 秒的环）。
pub const DEFAULT_CAPACITY_MS: u32 = 1_000;

/// 默认抽取批大小（帧/次 `pump`）——与 app 侧同值。
pub const DEFAULT_DRAIN_BATCH: u32 = 4_096;

/// 单次录制的秒数上限（防呆闸门：不设它，一个 `seconds: 1e9` 会申请无界缓冲）。
pub const MAX_SECONDS: f64 = 3_600.0;

/// 样本来源的**如实**文案（⛔ 不是声卡）。
pub const SOURCE_LABEL: &str = "in-memory-sine(440Hz)";

/// 抖动种子：录音侧用固定 `1`（录音没有"工程种子"；与
/// `crates/yeban-app/src/record_wav.rs` 的 `DeterministicDitherRng::new(1)` **同一条口径**）。
pub const DITHER_SEED: u32 = 1;

/// 一次已校验的录制请求（**还没有录制**）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordRequest {
    /// 录制秒数。
    pub seconds: f64,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 声道数（1 或 2）。
    pub channels: u16,
    /// 环容量（帧）。
    pub capacity_frames: usize,
    /// 抽取批大小（帧/次）。
    pub drain_batch: usize,
    /// 测试音频率（Hz）。
    pub tone_hz: f32,
}

/// 解析并**校验** `yeban_record_wav` 的实参（⛔ 不碰文件系统）。
///
/// # Errors
///
/// 缺 `path` / 空串 / 非有限或越界的数 ⇒ `INVALID_PARAMETER_RANGE`。
pub fn validate(
    arguments: &serde_json::Map<String, serde_json::Value>,
) -> Result<(PathBuf, RecordRequest), Fault> {
    let path = arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`path` 必须是字符串 (输出 WAV 路径)",
            )
        })?;
    if path.is_empty() {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`path` 不得为空串",
        ));
    }
    let seconds = match arguments.get("seconds") {
        None => DEFAULT_SECONDS,
        Some(value) => value.as_f64().ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`seconds` 必须是数字, 实际收到 {value}"),
            )
        })?,
    };
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`seconds` 必须是有限正数, 实际收到 {seconds}"),
        ));
    }
    if seconds > MAX_SECONDS {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`seconds` 超出单次上限 ({MAX_SECONDS} 秒), 实际 {seconds}"),
            serde_json::json!({
                "reason": "secondsExceedCap",
                "seconds": seconds,
                "maxSeconds": MAX_SECONDS,
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
                    ErrorCode::InvalidParameterRange,
                    format!("`sampleRate` 必须是 > 0 的整数 (Hz), 实际收到 {value}"),
                )
            })?,
    };
    let channels = match arguments.get("channels") {
        None => DEFAULT_CHANNELS,
        Some(value) => value
            .as_u64()
            .and_then(|channels| u16::try_from(channels).ok())
            .filter(|channels| matches!(channels, 1 | 2))
            .ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`channels` 只接受 1 (单声道) 或 2 (立体声), 实际收到 {value}"),
                )
            })?,
    };
    let capacity_frames = match arguments.get("capacityFrames") {
        None => usize::try_from(u64::from(sample_rate) * u64::from(DEFAULT_CAPACITY_MS) / 1_000)
            .unwrap_or(48_000)
            .max(1),
        Some(value) => value
            .as_u64()
            .and_then(|frames| usize::try_from(frames).ok())
            .filter(|frames| *frames > 0)
            .ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`capacityFrames` 必须是 > 0 的整数 (帧), 实际收到 {value}"),
                )
            })?,
    };
    let drain_batch = match arguments.get("drainBatch") {
        None => DEFAULT_DRAIN_BATCH as usize,
        Some(value) => value
            .as_u64()
            .and_then(|frames| usize::try_from(frames).ok())
            .filter(|frames| *frames > 0)
            .ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`drainBatch` 必须是 > 0 的整数 (帧), 实际收到 {value}"),
                )
            })?,
    };
    let tone_hz = match arguments.get("toneHz") {
        None => DEFAULT_TONE_HZ,
        Some(value) => {
            let hz = value.as_f64().ok_or_else(|| {
                Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`toneHz` 必须是数字, 实际收到 {value}"),
                )
            })? as f32;
            if !hz.is_finite() || hz <= 0.0 {
                return Err(Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    format!("`toneHz` 必须是有限正数 (Hz), 实际收到 {hz}"),
                ));
            }
            hz
        }
    };
    Ok((
        PathBuf::from(path),
        RecordRequest {
            seconds,
            sample_rate,
            channels,
            capacity_frames,
            drain_batch,
            tone_hz,
        },
    ))
}

impl RecordRequest {
    /// 诚实描述这次请求（响应里原样回显）。
    #[must_use]
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "seconds": self.seconds,
            "sampleRate": self.sample_rate,
            "channels": self.channels,
            "capacityFrames": self.capacity_frames,
            "drainBatch": self.drain_batch,
            "toneHz": self.tone_hz,
            "bitDepth": render::OUTPUT_BIT_DEPTH,
        })
    }

    /// 请求的帧数（`seconds × sampleRate`，向下取整）。
    #[must_use]
    pub fn requested_frames(&self) -> u64 {
        (self.seconds * f64::from(self.sample_rate)) as u64
    }
}

/// **解码型 sink**：接住 `RecordDrain::pump` 写出的**小端 f32**（引擎文档的口径），
/// 还原成交错样本 ⇒ 之后统一量化。与 app 侧 `record_wav.rs` 的 `SampleSink` 同一职责。
#[derive(Debug, Default)]
struct SampleSink {
    pending: Vec<u8>,
    samples: Vec<f32>,
}

impl SampleSink {
    /// 把完整的 4 字节组解成样本；不足 4 字节的尾巴留到下一次（`Write` 可能分批到达）。
    fn decode(&mut self) {
        let whole = self.pending.len() - self.pending.len() % 4;
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

/// 一次录制的产物（`plan` 里录完 + 编码完；`apply` 只落盘）。
#[derive(Clone, Debug)]
pub struct RecordArtifact {
    /// 请求（响应里原样回显）。
    pub request: RecordRequest,
    /// 输出路径。
    pub path: PathBuf,
    /// 24-bit WAV 字节。
    pub bytes: Vec<u8>,
    /// 容器头字节数。
    pub header_bytes: usize,
    /// 载荷字节数。
    pub payload_bytes: usize,
    /// 成功写出的帧数（`RecordDrain::frames_written`）。
    pub frames_written: u64,
    /// **整帧**成功推入的帧数（`RecordSender::pushed_frames`）。
    pub pushed: u64,
    /// 因环满被丢弃的帧数（**帧**级；本模块自己数，见模块文档 ②）。
    pub dropped: u64,
    /// 浮点峰值（量化之前）。
    pub peak: f32,
    /// 样本来源文案（⛔ 不是声卡）。
    pub source: &'static str,
}

impl RecordArtifact {
    /// 容器字节的 SHA-256。
    #[must_use]
    pub fn sha256(&self) -> String {
        AssetHash::of_bytes(&self.bytes).as_str().to_owned()
    }

    /// [G12] "样本不是工程音频"的**显式**读数（预览与响应共用一处定义 ⇒ 不会漂移）。
    const NOT_PROJECT_BOUNCE: bool = true;

    /// [G12] 指路：录工程声音的替代工具（接工程源本身是待裁项 HD-69，不在此擅自接线）。
    #[must_use]
    fn project_bounce_alternative() -> serde_json::Value {
        serde_json::json!({
            "projectBounce": {
                "tool": "yeban_render_master",
                "why": "`yeban_record_wav` 只录内存测试音 (ARCH-REC-001)；要录工程的声音请改用 `yeban_render_master`"
            }
        })
    }

    /// `dryRun` 预览（同一批实测数字 + `wouldWrite`；**不落盘**）。
    #[must_use]
    pub fn preview(&self) -> serde_json::Value {
        serde_json::json!({
            "source": self.source,
            "notProjectBounce": Self::NOT_PROJECT_BOUNCE,
            "alternatives": Self::project_bounce_alternative(),
            "archRec": {
                "specId": "ARCH-REC-001",
                "phases": ["record_channel (构造期唯一分配)", "push_frame (整帧原子 + 帧级丢弃计数)", "RecordDrain::pump (后台抽干)"],
                "deviceStream": false,
            },
            "request": self.request.to_value(),
            "requestedFrames": self.request.requested_frames(),
            "framesWritten": self.frames_written,
            "pushed": self.pushed,
            "dropped": self.dropped,
            "peak": self.peak,
            "wouldWrite": {
                "path": self.path.display().to_string(),
                "format": "wav",
                "frames": self.frames_written,
                "bytes": self.bytes.len(),
                "headerBytes": self.header_bytes,
                "payloadBytes": self.payload_bytes,
                "sha256": self.sha256(),
                "bitDepth": render::OUTPUT_BIT_DEPTH,
                "channels": self.request.channels,
                "sampleRate": self.request.sample_rate,
            },
        })
    }

    /// 真调用响应的 `data`。
    #[must_use]
    pub fn response_data(&self) -> serde_json::Value {
        serde_json::json!({
            "recorded": true,
            "path": self.path.display().to_string(),
            "format": "wav",
            "bitDepth": render::OUTPUT_BIT_DEPTH,
            "channels": self.request.channels,
            "sampleRate": self.request.sample_rate,
            "framesWritten": self.frames_written,
            "requestedFrames": self.request.requested_frames(),
            "pushed": self.pushed,
            "dropped": self.dropped,
            "bytes": self.bytes.len(),
            "headerBytes": self.header_bytes,
            "payloadBytes": self.payload_bytes,
            "sha256": self.sha256(),
            "peak": self.peak,
            "source": self.source,
            "notProjectBounce": Self::NOT_PROJECT_BOUNCE,
            "alternatives": Self::project_bounce_alternative(),
            "deviceStream": false,
            "request": self.request.to_value(),
            "dither": "TPDF, seed 1 (ARCH-FMT-001; 与 app 侧 record_wav 同一条口径)",
            "bwf": false,
            "bext": "none (本工具不写 bext/时间戳 ⇒ 同一请求逐字节可复现)",
            "atomic": true,
            "strategy": "同目录 .tmp-{ulid} + File::sync_all + rename (ARCH-SEC-004)",
            "unsupported": ["deviceInputStream", "inputLatencyCompensation"],
        })
    }
}

/// 录一段**测试音**并编码成 24-bit WAV（三段式 + 一次编码；⛔ 不落盘）。
///
/// 调用方（`plan_record_wav`）负责路径护栏；本函数只碰内存。
///
/// # Errors
///
/// 抽干 I/O 失败 ⇒ `IO_ERROR`；容器编码失败 ⇒ `RENDER_FAILED`。
pub fn record(request: &RecordRequest) -> Result<RecordArtifact, Fault> {
    let total_frames = request.requested_frames();
    let channels = usize::from(request.channels);
    // ---- ① 构造期：**唯一分配点** ----
    let (mut sender, receiver) = record_channel(request.capacity_frames, request.channels);
    // ---- ② 生产端 ＋ ③ 消费端**交错**（贴近真实双线程拓扑）----
    let mut drain = RecordDrain::new(receiver, SampleSink::default());
    let mut frame = vec![0.0_f32; channels];
    let mut phase = 0.0_f32;
    let step = core::f32::consts::TAU * request.tone_hz / request.sample_rate as f32;
    let mut dropped_frames = 0u64;
    for index in 0..total_frames {
        let value = libm::sinf(phase) * TONE_AMPLITUDE;
        frame.fill(value);
        // ⚠ 引擎文档的契约：推之前先问"还放得下整帧吗"（`push_frame` 环满时无法回滚
        // 已接受的样本 ⇒ 盲推会留**半帧** ⇒ 声道错位）。
        if sender.spare_frames() >= 1 {
            let _ = sender.push_frame(&frame);
        } else {
            dropped_frames += 1;
        }
        phase = (phase + step) % core::f32::consts::TAU;
        // 每 `drain_batch` 帧抽干一次（与生产交替）。
        // ⚠ 用**循环下标**判断：常量条件会让交错永不触发（app 侧实测踩过）。
        if (index + 1) % u64::try_from(request.drain_batch).unwrap_or(1) == 0 {
            while drain
                .pump(request.drain_batch)
                .map_err(|error| io_fault(&error))?
                > 0
            {}
        }
    }
    let pushed = sender.pushed_frames();
    // ---- ③ 收尾抽干 ----
    loop {
        let written = drain
            .pump(request.drain_batch)
            .map_err(|error| io_fault(&error))?;
        if written == 0 {
            break;
        }
    }
    let frames_written = drain.frames_written();
    let sink = drain.into_inner();
    let peak = math::peak_of(&sink.samples);
    let (bytes, header_bytes, payload_bytes) =
        render::encode_container_without_bext(&sink.samples, request.sample_rate, DITHER_SEED)?;
    Ok(RecordArtifact {
        request: *request,
        path: PathBuf::new(),
        bytes,
        header_bytes,
        payload_bytes,
        frames_written,
        pushed,
        dropped: dropped_frames,
        peak,
        source: SOURCE_LABEL,
    })
}

/// 抽干失败 ⇒ `IO_ERROR`（联集里的既有码）。
fn io_fault(error: &std::io::Error) -> Fault {
    Fault::domain_with_data(
        ErrorCode::IoError,
        format!("录音抽干失败: {error}"),
        serde_json::json!({ "reason": "recordDrainFailed", "ioError": error.kind().to_string() }),
    )
}

/// 输出路径的两条护栏（复用母带导出那一份：不得是工程文件、不得是锁文件）。
///
/// # Errors
///
/// 撞工程文件或锁文件 ⇒ `INVALID_PARAMETER_RANGE`。
pub fn guard_output_path(output: &Path, project_path: &Path) -> Result<(), Fault> {
    render::guard_output_path(output, project_path)
}
