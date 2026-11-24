//! cpal 声卡宿主、配置协商与 `NullBackend`。[ARCH-TOP-002, ROAD-M2-001, ARCH-PDC-002]
//!
//! ## 三条硬要求
//!
//! 1. **失败要给明确错误，绝不 panic**：设备不存在、采样率不支持、格式不是 `f32`、
//!    要求独占模式 —— 全部走 [`DeviceError`]；
//! 2. **回调内零分配、零锁、零阻塞 I/O、零日志** [AGENTS.md §2 红线 7]：
//!    数据回调只调用 [`crate::rt::EngineRuntime::process_quantum`]；
//!    错误回调只做一次原子自增（**不打印** —— 打印是阻塞式系统调用）；
//!
//!    回调体被抽成**具名函数** [`render_callback`]（它只有一行），因此"回调里到底做了
//!    什么"成了**可被执行的对象**：`build_output_stream` 的闭包与
//!    `crates/yeban-engine/tests/rt_zero_alloc.rs` 的判据 ⑳ 调用**同一个**
//!    [`render_callback`] ⇒ 往回调体里加任何工作（分配 / 锁 / I/O / 日志）都会让那条
//!    判据的四元组变红。⚠ 覆盖边界：判据 ⑳ 覆盖的是**回调体**；cpal 的**闭包/流**
//!    （`build_output_stream` → `play` → 真回调线程）仍然**未覆盖**（需要一台有声卡的
//!    机器，见 `rt_zero_alloc.rs` 的覆盖边界登记）。
//!
//!    ⭐ **T-FIX-72 的异率回调**：设备率 ≠ 工程率时，闭包改持
//!    [`ConvertedOutput`]、回调体是 [`render_callback_converted`] —— 渲染仍只走
//!    [`render_callback`]，外面套一层**预分配**的流式采样率转换
//!    （[`crate::resample::RateConverter`]，构造期分配、推进期零分配）。
//!    判据 ㊻ 以与 ⑳ 完全相同的口径钉住这条回调体；⑳ 本身一字未动。
//! 3. **真实设备路径必须能在不打开设备的前提下被编译与单测覆盖**：
//!    配置协商被抽成纯函数 [`negotiate`]（输入是 `&[SupportedStreamConfigRange]`，
//!    可以用 `SupportedStreamConfigRange::new` 手工构造），因此 CI 上无声卡也能测；
//!    [`NullBackend`] 则用同一份 [`EngineRuntime`] 在内存里驱动渲染。
//!
//! ## 与规范的两处**明确缺口**（见 `docs/ledger/engine-rt-notes.md` §4）
//!
//! - **[ROAD-M2-001] 实时线程优先级未实现**：cpal 0.18 的 `realtime` feature 只覆盖
//!   WASAPI / AAudio / PipeWire / JACK（`src/host/{wasapi,aaudio,pipewire,jack}/`），
//!   macOS CoreAudio 与 Linux-ALSA 路径没有任何开关（CoreAudio 由内核自行提升 RT 约束）。
//!   自行实现需要 `pthread_setschedparam`（也就是新的 `libc` 依赖）——
//!   那属于**依赖图裁决**，不由本工作线单独决定。
//! - **独占模式**：cpal 0.18 没有 WASAPI Exclusive 的 API（规范 §3.1 提到要用
//!   `wasapi` crate 实现）。[`ShareMode::PreferExclusive`] 会降级为共享并如实记录，
//!   [`ShareMode::RequireExclusive`] 返回 [`DeviceError::ExclusiveModeUnsupported`] ——
//!   宁可明确失败，也不假装独占成功。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, Host, SampleFormat, StreamConfig, SupportedBufferSize,
    SupportedStreamConfigRange,
};
use thiserror::Error;
use yeban_model::YebanProjectV1;

use crate::block::DEFAULT_BLOCK_FRAMES;
use crate::record::{RecordSender, push_input_block};
use crate::rt::{EngineRuntime, EngineStats};

/// 打开流的超时（cpal 0.18 的 `timeout` 参数）。避免设备被独占时无限等待。
pub const STREAM_OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// 错误回调里保存**首条**后端错误消息的最大字节数。
///
/// 固定缓冲 + 原子字节 ⇒ 错误回调内**零分配**（红线 7）。已知的 cpal 消息都是短的
/// ASCII（例如 `"Device disconnected"`）；超长消息按此上限截断。
pub const BACKEND_ERROR_MESSAGE_MAX: usize = 96;

/// cpal `ErrorKind` 的**稳定编号**（0 = 未知/未来新增的变体）。
///
/// 编号只在本 crate 的读数里使用；`name` 才是给人看的字面量。
#[must_use]
pub const fn backend_error_kind_code(kind: cpal::ErrorKind) -> u8 {
    // `ErrorKind` 是 `#[non_exhaustive]` ⇒ 必须留通配臂。
    match kind {
        cpal::ErrorKind::DeviceBusy => 1,
        cpal::ErrorKind::DeviceChanged => 2,
        cpal::ErrorKind::DeviceNotAvailable => 3,
        cpal::ErrorKind::HostUnavailable => 4,
        cpal::ErrorKind::InvalidInput => 5,
        cpal::ErrorKind::PermissionDenied => 6,
        cpal::ErrorKind::RealtimeDenied => 7,
        cpal::ErrorKind::ResourceExhausted => 8,
        cpal::ErrorKind::StreamInvalidated => 9,
        cpal::ErrorKind::UnsupportedConfig => 10,
        cpal::ErrorKind::UnsupportedOperation => 11,
        cpal::ErrorKind::Xrun => 12,
        cpal::ErrorKind::BackendError => 13,
        cpal::ErrorKind::Other => 14,
        _ => 0,
    }
}

/// 编号 → 稳定名字（机器可读行里**不带空白**）。
#[must_use]
pub const fn backend_error_kind_name(code: u8) -> &'static str {
    match code {
        1 => "device-busy",
        2 => "device-changed",
        3 => "device-not-available",
        4 => "host-unavailable",
        5 => "invalid-input",
        6 => "permission-denied",
        7 => "realtime-denied",
        8 => "resource-exhausted",
        9 => "stream-invalidated",
        10 => "unsupported-config",
        11 => "unsupported-operation",
        12 => "xrun",
        13 => "backend-error",
        14 => "other",
        _ => "unknown",
    }
}

/// 首条后端错误的种类 + 截断后的消息（控制面读，**不在实时路径上**）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendErrorInfo {
    kind: u8,
    message_len: u8,
    message: [u8; BACKEND_ERROR_MESSAGE_MAX],
}

impl BackendErrorInfo {
    /// 稳定编号（见 [`backend_error_kind_code`]）。
    #[must_use]
    pub const fn kind_code(&self) -> u8 {
        self.kind
    }

    /// 稳定名字（见 [`backend_error_kind_name`]）。
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        backend_error_kind_name(self.kind)
    }

    /// cpal 给的原始消息；没有消息时是空串。截断可能落在 UTF-8 边界外 ⇒ 退回空串。
    #[must_use]
    pub fn message(&self) -> &str {
        std::str::from_utf8(&self.message[..usize::from(self.message_len)]).unwrap_or("")
    }
}

/// 后端错误的两档计数：**运行期** vs **关闭/停止期**。
///
/// ⚠ 口径：阶段由**回调实际执行的时刻**决定（[`BackendErrorLog::begin_stop`] 之后
/// 到达的回调记入 `stop`）。边界附近已在飞行中的事件可能被归到 stop —— 这是
/// "两档读数"的自洽代价，登记在 `#147`；它**不会**改变 `total()`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendErrorCounts {
    /// 流处于运行阶段时到达的错误回调次数。
    pub runtime: u64,
    /// [`BackendErrorLog::begin_stop`] 之后到达的错误回调次数（暂停/关流阶段）。
    pub stop: u64,
    /// 首条错误的种类与消息（一次都没来时 `None`）。
    pub first: Option<BackendErrorInfo>,
}

impl BackendErrorCounts {
    /// 两档之和 —— 与旧口径的 `backend_errors` **逐位相同**。
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.runtime.saturating_add(self.stop)
    }

    /// 一行**机器可读**的分档读数（`BACKEND-ERRORS direction=... runtime=... ...`）。
    ///
    /// 设计约束：
    ///
    /// - 独立于 `BENCH` 行 ⇒ 不改变 `BENCH` 的任何键、键序、行长（`latency.rs` 的
    ///   严格解析器与既有指纹判据因此一位不变）；
    /// - `first_message` 只保留 `[A-Za-z0-9_.:-]`，其余字节换成 `_` ⇒ 值里没有空白，
    ///   可以被 `split_whitespace` 安全解析；没有错误时两项都是字面 `none`。
    #[must_use]
    pub fn machine_line(&self, direction: &str) -> String {
        let (kind, message) = match self.first {
            Some(info) => (
                info.kind_name(),
                backend_error_message_token(info.message()),
            ),
            None => ("none", "none".to_owned()),
        };
        format!(
            "BACKEND-ERRORS direction={direction} runtime={} stop={} total={} first_kind={kind} \
first_message={message}",
            self.runtime,
            self.stop,
            self.total(),
        )
    }
}

/// 把错误消息压成一个**不含空白**的机器可读 token（空文本 ⇒ `none`）。
fn backend_error_message_token(text: &str) -> String {
    if text.is_empty() {
        return "none".to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        out.push(
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':') {
                char::from(byte)
            } else {
                '_'
            },
        );
    }
    out
}

/// 后端错误回调的**零分配**计数器：只做原子自增；首条错误额外记下种类与消息。
///
/// 为什么分两档：`measure_latency --loopback` 实测"每方向恰好 1 次"（`#147`），
/// 而**关闭/暂停阶段**的错误与**运行期**的真 XRun 是两种不同的事实。旧口径把它们
/// 混成一个数，于是"1"既可能是真错误、也可能是关流副作用，读不出来是哪一种。
///
/// 实时约束：`record` 只做原子操作与固定缓冲拷贝 —— 零分配、零锁、零 I/O [红线 7]。
#[derive(Debug)]
pub struct BackendErrorLog {
    runtime: AtomicU64,
    stop: AtomicU64,
    stopping: AtomicBool,
    first_claimed: AtomicBool,
    first_ready: AtomicBool,
    first_kind: AtomicU8,
    first_message_len: AtomicU8,
    first_message: [AtomicU8; BACKEND_ERROR_MESSAGE_MAX],
}

impl Default for BackendErrorLog {
    fn default() -> Self {
        Self {
            runtime: AtomicU64::new(0),
            stop: AtomicU64::new(0),
            stopping: AtomicBool::new(false),
            first_claimed: AtomicBool::new(false),
            first_ready: AtomicBool::new(false),
            first_kind: AtomicU8::new(0),
            first_message_len: AtomicU8::new(0),
            first_message: std::array::from_fn(|_| AtomicU8::new(0)),
        }
    }
}

impl BackendErrorLog {
    /// 空的计数器（运行期阶段）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// **错误回调内**调用：按当前阶段自增，并（仅首条）记下种类与消息。
    ///
    /// ⛔ 不打印、不分配、不加锁。
    pub fn record(&self, error: &cpal::Error) {
        if self.stopping.load(Ordering::Acquire) {
            self.stop.fetch_add(1, Ordering::Relaxed);
        } else {
            self.runtime.fetch_add(1, Ordering::Relaxed);
        }
        if self
            .first_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.first_kind
                .store(backend_error_kind_code(error.kind()), Ordering::Relaxed);
            let bytes = error.message().unwrap_or("").as_bytes();
            let len = bytes.len().min(BACKEND_ERROR_MESSAGE_MAX);
            for (slot, byte) in self.first_message.iter().zip(&bytes[..len]) {
                slot.store(*byte, Ordering::Relaxed);
            }
            self.first_message_len
                .store(u8::try_from(len).unwrap_or(u8::MAX), Ordering::Relaxed);
            // Release 配对 counts() 的 Acquire load：所有 first_* 字段写完后再发布就绪标志。
            self.first_ready.store(true, Ordering::Release);
        }
    }

    /// 从这一时刻起，到达的错误回调计入**关闭/停止期**。
    pub fn begin_stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    /// 从这一时刻起，到达的错误回调重新计入**运行期**（`play` 之后用）。
    pub fn begin_run(&self) {
        self.stopping.store(false, Ordering::Release);
    }

    /// 控制面读数（暂停/关流之后读才有意义）。
    #[must_use]
    pub fn counts(&self) -> BackendErrorCounts {
        let first = if self.first_ready.load(Ordering::Acquire) {
            let len = usize::from(self.first_message_len.load(Ordering::Relaxed));
            let mut message = [0u8; BACKEND_ERROR_MESSAGE_MAX];
            for (slot, byte) in message.iter_mut().zip(&self.first_message) {
                *slot = byte.load(Ordering::Relaxed);
            }
            Some(BackendErrorInfo {
                kind: self.first_kind.load(Ordering::Relaxed),
                message_len: u8::try_from(len).unwrap_or(u8::MAX),
                message,
            })
        } else {
            None
        };
        BackendErrorCounts {
            runtime: self.runtime.load(Ordering::Relaxed),
            stop: self.stop.load(Ordering::Relaxed),
            first,
        }
    }
}

/// 共享/独占模式偏好。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShareMode {
    /// 共享模式（OS 混音器参与）。所有平台都支持。
    Shared,
    /// **尽力而为**地争取独占模式；不支持时静默降级为共享。
    #[default]
    PreferExclusive,
    /// **要求**独占模式；不支持时返回 [`DeviceError::ExclusiveModeUnsupported`]。
    ///
    /// cpal 0.18 没有独占 API，因此当前实现下这个选项**总是**失败 —— 这是有意为之：
    /// 规范要求"失败给出明确错误而不是 panic"。
    RequireExclusive,
}

/// 引擎想要的音频配置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    /// 目标采样率 (Hz)。
    pub sample_rate: u32,
    /// 目标每回调帧数（默认 128 = L1 规范块长 [ARCH-DET-001]）。
    pub block_frames: u32,
    /// 目标输出通道数。
    pub channels: u16,
    /// 共享/独占偏好。
    pub share_mode: ShareMode,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            block_frames: DEFAULT_BLOCK_FRAMES as u32,
            channels: 2,
            share_mode: ShareMode::default(),
        }
    }
}

impl EngineConfig {
    /// 从工程的音频配置投影（采样率唯一事实源是 `audio_config.sample_rate`）。
    #[must_use]
    pub fn from_project(project: &YebanProjectV1) -> Self {
        Self {
            sample_rate: project.audio_config.sample_rate.hz(),
            block_frames: project.audio_config.block_size.frames(),
            channels: 2,
            share_mode: ShareMode::default(),
        }
    }
}

/// 协商结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NegotiatedConfig {
    /// 实际通道数。
    pub channels: u16,
    /// 实际采样率 (Hz)：**设备流**真正跑的率（cpal `StreamConfig` 用的就是它）。
    pub sample_rate: u32,
    /// ⭐ **引擎侧采样率** (Hz)：输出腿是引擎的渲染率（工程率），输入腿是
    /// 请求率（工程率）。与 [`Self::sample_rate`] 不同 ⇒ 实时通路在设备腿做
    /// **采样率转换**（[`crate::resample::RateConverter`]，T-FIX-72），
    /// 见 [`Self::resampling`]。
    pub source_rate: u32,
    /// 请求的缓冲帧数（`BufferSize::Fixed` —— 由后端决定是否尊重）。
    pub buffer_size: BufferSize,
    /// 实际采样格式（当前只支持 `f32`）。
    pub sample_format: SampleFormat,
    /// **实际**共享模式：当前实现恒为 [`ShareMode::Shared`]（见模块文档的缺口说明）。
    pub share_mode: ShareMode,
    /// 目标块长是否被后端接受（`false` 表示后端只会用默认缓冲大小，
    /// 此时引擎内部仍以 128 帧为渲染量子，只是回调长度可能不同）。
    pub fixed_block_accepted: bool,
}

impl NegotiatedConfig {
    /// ⭐ 本次打开是否需要**采样率转换**：设备流的率（[`Self::sample_rate`]）
    /// 与引擎侧的率（[`Self::source_rate`]）不同 ⇒ 输出腿在回调里转换、
    /// 输入腿在录音抽取侧转换（T-FIX-72）。相等 ⇒ 逐位直通，现行为一字不变。
    #[must_use]
    pub const fn resampling(&self) -> bool {
        self.source_rate != self.sample_rate
    }

    /// ⭐ 缓冲模式的人读描述（T-FIX-71，控制面/状态行用）：固定缓冲 ⇒
    /// `固定 N 帧`；后端默认 ⇒ `后端默认`。放在引擎侧是因为 `buffer_size`
    /// 的类型是 cpal 的 —— 应用层（`yeban-app`）⛔ 不裸用 cpal 类型（D19），
    /// 它只读这一句现成的描述。
    #[must_use]
    pub fn buffer_label(&self) -> String {
        match self.buffer_size {
            BufferSize::Fixed(frames) => format!("固定 {frames} 帧"),
            BufferSize::Default => "后端默认".to_owned(),
        }
    }

    /// ⭐ **名义**单向延迟（毫秒）：固定缓冲帧数 ÷ 设备采样率。
    ///
    /// ⚠ 这是**名义值**（只算缓冲这一段），⛔ 不是实测往返延迟 —— 实测要走
    /// `latency.rs` 的回环测量。后端默认缓冲 ⇒ 帧数未知 ⇒ `None`
    /// （⛔ 不编一个数字出来）。
    #[must_use]
    pub fn nominal_latency_ms(&self) -> Option<f64> {
        match self.buffer_size {
            BufferSize::Fixed(frames) if self.sample_rate > 0 => {
                Some(f64::from(frames) * 1_000.0 / f64::from(self.sample_rate))
            }
            _ => None,
        }
    }
}

/// 设备/配置相关错误。**全部是返回值，绝不 panic**。
#[derive(Debug, Error)]
pub enum DeviceError {
    /// 系统上没有默认输出设备（CI 容器、无声卡服务器、设备被拔掉）。
    #[error("no default output device available")]
    NoDefaultOutputDevice,
    /// 没有默认**输入**设备（录音侧）。[A④]
    ///
    /// ⚠ 与 [`DeviceError::NoDefaultOutputDevice`] **分开**：把"没有输入设备"报成 output
    /// 是**错的话**，会把调用方引到错误的排查方向。
    #[error("no default input device available")]
    NoDefaultInputDevice,

    /// ⭐ 用户**按名指定**的输出设备不在场（T-FIX-71，音频设置线）：
    /// 枚举到的输出设备里没有这个显示名（设备被拔掉 / 改名 / 驱动未就绪）。
    ///
    /// ⚠ 与 [`DeviceError::NoDefaultOutputDevice`] **分开**，且 ⛔ **绝不静默
    /// 改用默认设备**：用户在设置里选了哪台就必须是哪台，打不开就响亮报错
    /// 并**点名** —— 静默换一台会让用户以为声音走了 A 设备、实际走了 B 设备，
    /// 排查方向被彻底带偏（与本枚举其它"分开报错"的裁决同一理由）。
    #[error(
        "the requested output device \"{name}\" is not available; \
         refusing to fall back to the default output device"
    )]
    NamedOutputDeviceUnavailable {
        /// 用户指定的输出设备显示名（原样回显，便于定位）。
        name: String,
    },

    /// ⭐ 用户**按名指定**的输入设备不在场（T-FIX-71）：与输出侧同款裁决，
    /// ⛔ 不静默改用默认输入设备，报错点名。
    #[error(
        "the requested input device \"{name}\" is not available; \
         refusing to fall back to the default input device"
    )]
    NamedInputDeviceUnavailable {
        /// 用户指定的输入设备显示名（原样回显，便于定位）。
        name: String,
    },

    /// 设备没有报告任何输出配置。
    #[error("device reports no supported output configuration")]
    NoSupportedConfigs,
    /// 设备没有报告任何**输入**配置（录音侧）。[A④]
    ///
    /// ⚠ 与 [`DeviceError::NoSupportedConfigs`] 分开：后者的文案含 "output" ⇒ 复用会把
    /// "没有输入配置"报成输出侧的问题。
    #[error("device reports no supported input configuration")]
    NoSupportedInputConfigs,

    /// 请求的采样率/通道数在设备能力范围之外。
    #[error(
        "requested config unsupported: want {want_channels}ch @ {want_rate}Hz, device offers {offered}"
    )]
    UnsupportedConfig {
        /// 请求的通道数。
        want_channels: u16,
        /// 请求的采样率。
        want_rate: u32,
        /// 设备能力的可读描述（用于错误信息，不是机器可解析字段）。
        offered: String,
    },

    /// 设备不支持 `f32` 采样格式。
    #[error("device does not support the f32 sample format (negotiated {format:?})")]
    UnsupportedSampleFormat {
        /// 协商到的格式。
        format: SampleFormat,
    },

    /// 设备不支持请求的采样率（T-FIX-59 的响亮拒绝形态）。
    ///
    /// ⭐ **T-FIX-72 起 [`negotiate`] 不再产生这个错误**：实时通路已有采样率
    /// 转换，协商改为接受设备率并在 [`NegotiatedConfig`] 里如实标记两侧
    /// 采样率（设备行/装载报告会写明正在转换，⛔ 不是静默转换）。本变体
    /// 保留在公共 API 里（下游的错误分流与文案不因政策变更而静默失效），
    /// 其原文仍可用于需要"点名两侧数值 + 补救办法"的场合。
    #[error(
        "device does not support the requested sample rate: want {want_rate} Hz, \
         the device would fall back to {device_rate} Hz; the realtime path has no \
         sample-rate conversion, so audio would run at the wrong speed and pitch. \
         Use a device that supports {want_rate} Hz, or change the project sample \
         rate to {device_rate} Hz"
    )]
    SampleRateMismatch {
        /// 请求的采样率（工程采样率）。
        want_rate: u32,
        /// 设备退回到的标准采样率（实际值）。
        device_rate: u32,
    },

    /// 请求了独占模式，但 cpal 0.18 没有该能力（见模块文档）。
    #[error("exclusive (non-shared) mode is not supported by this backend")]
    ExclusiveModeUnsupported,

    /// 后端返回的错误（设备忙、权限不足、配置失效……）。
    #[error("audio backend error: {0}")]
    Backend(#[from] cpal::Error),
}

/// 设备的可枚举信息（**非实时路径**：允许分配字符串）。
/// 输出侧由 [`enumerate_output_devices`] 产出、输入侧由
/// [`enumerate_input_devices`] 产出，字段口径一致（`channels` / `sample_rate` /
/// `sample_format` / `supported` 都指**该方向**的配置）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// 设备显示名（`cpal::Device` 的 `Display`）。
    pub name: String,
    /// 设备稳定标识（`DeviceTrait::id`；失败时为空）。
    pub id: String,
    /// 是否为宿主在该方向的默认设备（按 `id` 比对；`id` 读不到时为 `false`，
    /// ⛔ 不靠显示名猜 —— 显示名可以重名）。
    pub is_default: bool,
    /// 默认配置的通道数。
    pub channels: u16,
    /// 默认配置的采样率。
    pub sample_rate: u32,
    /// 默认配置的采样格式。
    pub sample_format: SampleFormat,
    /// 全部受支持配置的可读描述（[`describe`] 的格式：逐范围一行
    /// `Nch MIN-MAXHz FORMAT buffer LO..=HI`）。范围读不到 ⇒
    /// `unavailable: <后端原文>`（响亮写进字段，⛔ 不静默留空 —— 枚举的
    /// 用途就是诊断，缺这一格正是最该被看见的事实）。
    pub supported: String,
}

/// 把一台设备的受支持范围收成 [`DeviceInfo::supported`] 的文本。
fn supported_text(ranges: Result<Vec<SupportedStreamConfigRange>, cpal::Error>) -> String {
    match ranges {
        Ok(ranges) if !ranges.is_empty() => describe(&ranges),
        Ok(_) => "none reported".to_owned(),
        Err(error) => format!("unavailable: {error}"),
    }
}

/// 枚举全部输出设备。无声卡时返回空 `Vec`（不是错误）。
///
/// # Errors
///
/// 后端拒绝枚举（例如音频守护进程不可用）→ [`DeviceError::Backend`]。
pub fn enumerate_output_devices() -> Result<Vec<DeviceInfo>, DeviceError> {
    let host = cpal::default_host();
    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().map(|id| id.to_string()).ok())
        .unwrap_or_default();
    let mut out = Vec::new();
    for device in host.output_devices()? {
        let config = match device.default_output_config() {
            Ok(config) => config,
            // 单个设备读不到默认配置不应让整个枚举失败。
            Err(_) => continue,
        };
        let id = device.id().map(|id| id.to_string()).unwrap_or_default();
        out.push(DeviceInfo {
            name: device.to_string(),
            is_default: !id.is_empty() && id == default_id,
            supported: supported_text(device.supported_output_configs().map(|it| it.collect())),
            id,
            channels: config.channels(),
            sample_rate: config.sample_rate(),
            sample_format: config.sample_format(),
        });
    }
    Ok(out)
}

/// 枚举全部**输入**设备（录音侧）：与 [`enumerate_output_devices`] 同构同口径，
/// 只是方向换成输入（默认配置读 `default_input_config`、范围读
/// `supported_input_configs`、默认设备读 `default_input_device`）。
/// 无声卡时返回空 `Vec`（不是错误）。
///
/// # Errors
///
/// 后端拒绝枚举 → [`DeviceError::Backend`]。
pub fn enumerate_input_devices() -> Result<Vec<DeviceInfo>, DeviceError> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|device| device.id().map(|id| id.to_string()).ok())
        .unwrap_or_default();
    let mut out = Vec::new();
    for device in host.input_devices()? {
        let config = match device.default_input_config() {
            Ok(config) => config,
            // 单个设备读不到默认配置不应让整个枚举失败。
            Err(_) => continue,
        };
        let id = device.id().map(|id| id.to_string()).unwrap_or_default();
        out.push(DeviceInfo {
            name: device.to_string(),
            is_default: !id.is_empty() && id == default_id,
            supported: supported_text(device.supported_input_configs().map(|it| it.collect())),
            id,
            channels: config.channels(),
            sample_rate: config.sample_rate(),
            sample_format: config.sample_format(),
        });
    }
    Ok(out)
}

/// ⭐ **按名选设备的纯决策**（T-FIX-71）：在已枚举到的显示名清单里找 `want`，
/// 返回**首个**下标；找不到 ⇒ `None`。
///
/// 口径（写死，⛔ 不留模糊空间）：
/// - **精确相等**才算命中 —— ⛔ 不做前缀 / 子串 / 忽略大小写匹配：设备名是
///   用户在设置里选定的那台设备的身份，近似命中等于选错设备；
/// - 显示名**可以重名**（同型号两台 USB 设备），重名时取首个 —— 枚举顺序
///   由后端决定，调用方若在意区分应改用 [`DeviceInfo::id`]（本函数的输入
///   只是名字清单，拿不到 id，这是它的能力边界，如实写明）；
/// - 本函数**纯**（不触设备）⇒ CI 无声卡也可判；cpal 侧的取设备逻辑
///   （[`select_output_device`] / [`select_input_device`]）只准经它决策，
///   ⛔ 不另写一份查找（两处政策漂移正是本文件反复消灭的形态）。
#[must_use]
pub fn pick_named_device_index(names: &[String], want: &str) -> Option<usize> {
    names.iter().position(|name| name == want)
}

/// 按名（或默认）取一台**输出**设备（T-FIX-71 的选择点，输出侧唯一实现）。
///
/// - `want_name == None` ⇒ 系统默认输出设备（**现行为**，缺失 ⇒
///   [`DeviceError::NoDefaultOutputDevice`]）；
/// - `Some(name)` ⇒ 在宿主枚举里按 [`pick_named_device_index`] 精确命中；
///   枚举不到 ⇒ [`DeviceError::NamedOutputDeviceUnavailable`]（点名，
///   ⛔ 不回退默认）；枚举本身被后端拒绝 ⇒ [`DeviceError::Backend`]。
fn select_output_device(host: &Host, want_name: Option<&str>) -> Result<Device, DeviceError> {
    let Some(want) = want_name else {
        return host
            .default_output_device()
            .ok_or(DeviceError::NoDefaultOutputDevice);
    };
    let devices: Vec<Device> = host.output_devices()?.collect();
    let names: Vec<String> = devices.iter().map(ToString::to_string).collect();
    let picked =
        pick_named_device_index(&names, want).and_then(|index| devices.into_iter().nth(index));
    if let Some(device) = picked {
        return Ok(device);
    }
    Err(DeviceError::NamedOutputDeviceUnavailable {
        name: want.to_owned(),
    })
}

/// 按名（或默认）取一台**输入**设备：与 [`select_output_device`] 同构同口径，
/// 只是方向换成输入（缺失默认 ⇒ [`DeviceError::NoDefaultInputDevice`]；
/// 指定名不在场 ⇒ [`DeviceError::NamedInputDeviceUnavailable`]）。
fn select_input_device(host: &Host, want_name: Option<&str>) -> Result<Device, DeviceError> {
    let Some(want) = want_name else {
        return host
            .default_input_device()
            .ok_or(DeviceError::NoDefaultInputDevice);
    };
    let devices: Vec<Device> = host.input_devices()?.collect();
    let names: Vec<String> = devices.iter().map(ToString::to_string).collect();
    let picked =
        pick_named_device_index(&names, want).and_then(|index| devices.into_iter().nth(index));
    if let Some(device) = picked {
        return Ok(device);
    }
    Err(DeviceError::NamedInputDeviceUnavailable {
        name: want.to_owned(),
    })
}

/// ⭐ **输入侧的格式协商**（[A④]）：**纯函数**，⛔ 不触碰真实设备 ⇒ CI/本机均可单测。
///
/// 与输出侧的 [`negotiate`] **同一份政策**：本函数**委托**给它，⛔ 不复制一份选择逻辑
/// （否则两侧会漂移，正是"同类情形两处政策不同"的形态）。
///
/// 与输出侧唯一的差别是**错误措辞**：输入侧若需要报告"没有默认设备"，
/// 调用方应当用 [`DeviceError::NoDefaultInputDevice`]（本函数自身**不会**产生它 ——
/// 它只做"从已枚举到的配置里选择"）。
///
/// # Errors
/// 与 [`negotiate`] 相同：`NoSupportedConfigs`／`UnsupportedSampleFormat`／
/// `UnsupportedConfig`／`ExclusiveModeUnsupported`。
pub fn negotiate_input(
    supported: &[SupportedStreamConfigRange],
    want: &EngineConfig,
) -> Result<NegotiatedConfig, DeviceError> {
    // ⭐ 唯一的输入侧差异：空范围报**输入专用**错误；其余一律委托给 `negotiate`
    //（⛔ 不复制选择逻辑，免得两侧政策漂移 —— 那正是"同类情形两处政策不同"的形态）。
    if supported.is_empty() {
        return Err(DeviceError::NoSupportedInputConfigs);
    }
    negotiate(supported, want)
}

/// [`negotiate_input`] 的 trace 版：与它**同一条实现**，多收一份逐步记录
/// （口径见 [`negotiate_traced`]）。空范围时 trace 里就是拒绝那一步本身。
pub fn negotiate_input_traced(
    supported: &[SupportedStreamConfigRange],
    want: &EngineConfig,
) -> (Result<NegotiatedConfig, DeviceError>, Vec<String>) {
    if supported.is_empty() {
        return (
            Err(DeviceError::NoSupportedInputConfigs),
            vec![
                "negotiate-input: no supported input configs reported -> NoSupportedInputConfigs"
                    .to_owned(),
            ],
        );
    }
    negotiate_traced(supported, want)
}

/// 设备能力与请求的协商（**纯函数**，不触碰真实设备 ⇒ CI 上可单测）。
///
/// 选择规则（按优先级）：
///
/// 1. 优先 `f32` + 请求的采样率 + 请求的通道数（立体声不足时接受设备的默认通道数）；
/// 2. 采样率匹配但通道数不同 ⇒ 接受设备通道数；
/// 3. 设备给不了请求率 ⇒ **按设备的首选标准率开流 + 实时采样率转换**
///    （T-FIX-72）：结果里 [`NegotiatedConfig::source_rate`] 记引擎侧的率、
///    [`NegotiatedConfig::sample_rate`] 记设备实际率，两者不同即
///    [`NegotiatedConfig::resampling`] —— 设备腿据此装转换器，设备行/装载
///    报告如实写明"正在转换"，⛔ 不是静默退回（T-FIX-59 的响亮口径保留在
///    **报告**里，而不是拒绝里：44100-only 设备在真实环境里太常见，
///    拒绝等于不出声；照 Logic Pro 口径，率不一致时转换后以正确速度出声）；
/// 4. `f32` 完全不可用 → [`DeviceError::UnsupportedSampleFormat`]（不静默做格式转换，
///    因为那会引入一条未经确定性验证的样本路径 [ARCH-DET-001]）。
///
/// # Errors
///
/// 见上；另有 [`DeviceError::UnsupportedConfig`] / [`DeviceError::NoSupportedConfigs`]。
pub fn negotiate(
    supported: &[SupportedStreamConfigRange],
    want: &EngineConfig,
) -> Result<NegotiatedConfig, DeviceError> {
    negotiate_traced(supported, want).0
}

/// [`negotiate`] 的 trace 版（T-FIX-70，调试输出的数据源）：与它**同一条
/// 实现**（`negotiate` 就是本函数丢弃 trace 的薄壳，判据钉死两者结果一致），
/// 多收一份逐步记录 —— 请求与设备能力的第一行、每一步的命中/未命中、
/// 最后一行是最终结果。trace 是**控制面诊断文本**（英文，与 [`DeviceError`]
/// 的 Display 同语言，嵌进上层报告行），⛔ 不进实时路径。
pub fn negotiate_traced(
    supported: &[SupportedStreamConfigRange],
    want: &EngineConfig,
) -> (Result<NegotiatedConfig, DeviceError>, Vec<String>) {
    let mut trace = Vec::new();
    let result = negotiate_inner(supported, want, &mut trace);
    match &result {
        Ok(got) => trace.push(format!(
            "negotiate: result {} Hz / {} ch, buffer {:?}, fixed_block_accepted={}",
            got.sample_rate, got.channels, got.buffer_size, got.fixed_block_accepted
        )),
        Err(error) => trace.push(format!("negotiate: failed: {error}")),
    }
    (result, trace)
}

/// [`negotiate`] 的实现本体；`trace` 逐行记录每一步（见 [`negotiate_traced`]）。
fn negotiate_inner(
    supported: &[SupportedStreamConfigRange],
    want: &EngineConfig,
    trace: &mut Vec<String>,
) -> Result<NegotiatedConfig, DeviceError> {
    trace.push(format!(
        "negotiate: want {} ch @ {} Hz, block {} frames, share {:?}; offered: {}",
        want.channels,
        want.sample_rate,
        want.block_frames,
        want.share_mode,
        if supported.is_empty() {
            "none".to_owned()
        } else {
            describe(supported)
        },
    ));
    if want.share_mode == ShareMode::RequireExclusive {
        trace.push(
            "negotiate: exclusive mode requested -> refused (backend has no exclusive API)"
                .to_owned(),
        );
        return Err(DeviceError::ExclusiveModeUnsupported);
    }
    if supported.is_empty() {
        trace.push("negotiate: no supported configs -> NoSupportedConfigs".to_owned());
        return Err(DeviceError::NoSupportedConfigs);
    }

    let offered = describe(supported);
    let f32_ranges: Vec<&SupportedStreamConfigRange> = supported
        .iter()
        .filter(|range| range.sample_format() == SampleFormat::F32)
        .collect();
    if f32_ranges.is_empty() {
        trace.push(format!(
            "negotiate: no f32 range (first reported format {:?}) -> UnsupportedSampleFormat",
            supported[0].sample_format()
        ));
        return Err(DeviceError::UnsupportedSampleFormat {
            format: supported[0].sample_format(),
        });
    }
    trace.push(format!(
        "negotiate: {} of {} ranges are f32",
        f32_ranges.len(),
        supported.len()
    ));

    // 1) 精确匹配请求的采样率 + 通道数
    for range in &f32_ranges {
        if range.channels() == want.channels && range.contains_rate(want.sample_rate) {
            trace.push(format!(
                "negotiate: step 1 (exact rate + channels) matched {} ch {}-{} Hz",
                range.channels(),
                range.min_sample_rate(),
                range.max_sample_rate()
            ));
            return Ok(finish(range, want, want.sample_rate));
        }
    }
    trace.push("negotiate: step 1 (exact rate + channels) -> no match".to_owned());
    // 2) 采样率匹配但通道数不同（接受设备默认通道数）
    for range in &f32_ranges {
        if range.contains_rate(want.sample_rate) {
            trace.push(format!(
                "negotiate: step 2 (rate matches, channels differ) matched {} ch {}-{} Hz",
                range.channels(),
                range.min_sample_rate(),
                range.max_sample_rate()
            ));
            return Ok(finish(range, want, want.sample_rate));
        }
    }
    trace.push("negotiate: step 2 (rate matches, channels differ) -> no match".to_owned());
    // 3) 设备给不了请求率 ⇒ 按设备首选标准率开流，实时通路做采样率转换
    //    （T-FIX-72，取代 T-FIX-59 的响亮拒绝）：先找同通道数的范围，再任意
    //    范围。`finish` 会把请求率记进 `source_rate`、把设备率记进
    //    `sample_rate` ⇒ `resampling()` 为真 ⇒ 设备腿装转换器、设备行报告
    //    转换中。⛔ 不是静默退回：两侧数值都在协商结果里，报告层必须写明。
    for same_channels in [true, false] {
        for range in &f32_ranges {
            if same_channels && range.channels() != want.channels {
                continue;
            }
            if let Some(config) = range.try_with_standard_sample_rate() {
                trace.push(format!(
                    "negotiate: step 3 (device rate differs) matched {} ch at {} Hz -> resampling from wanted {} Hz",
                    range.channels(),
                    config.sample_rate(),
                    want.sample_rate
                ));
                return Ok(finish(range, want, config.sample_rate()));
            }
        }
    }
    trace.push(
        "negotiate: step 3 (standard-rate fallback) -> no standard rate available".to_owned(),
    );
    Err(DeviceError::UnsupportedConfig {
        want_channels: want.channels,
        want_rate: want.sample_rate,
        offered,
    })
}

/// 对**默认输出设备**做一次只协商、不建流的 trace（T-FIX-70，调试输出在
/// 打开失败时的数据源）：宿主、设备名、能力与 [`negotiate_traced`] 的逐步
/// 记录逐行返回；每一步的失败都写成行（⛔ 不返回 `Err` 把诊断吞掉 ——
/// 本函数存在的意义就是把失败现场说全）。⛔ 不建流、不碰 [`EngineRuntime`]。
#[must_use]
pub fn trace_output_negotiation(want: &EngineConfig) -> Vec<String> {
    let host = cpal::default_host();
    let mut lines = vec![format!("trace-output: host {:?}", host.id())];
    let Some(device) = host.default_output_device() else {
        lines.push("trace-output: no default output device -> NoDefaultOutputDevice".to_owned());
        return lines;
    };
    lines.push(format!("trace-output: device {device}"));
    match device.supported_output_configs() {
        Ok(ranges) => {
            let supported: Vec<SupportedStreamConfigRange> = ranges.collect();
            let (_, trace) = negotiate_traced(&supported, want);
            lines.extend(trace);
        }
        Err(error) => lines.push(format!(
            "trace-output: supported configs unreadable: {error}"
        )),
    }
    lines
}

/// [`trace_output_negotiation`] 的输入侧同构版（默认**输入**设备 +
/// [`negotiate_input_traced`]）。
#[must_use]
pub fn trace_input_negotiation(want: &EngineConfig) -> Vec<String> {
    let host = cpal::default_host();
    let mut lines = vec![format!("trace-input: host {:?}", host.id())];
    let Some(device) = host.default_input_device() else {
        lines.push("trace-input: no default input device -> NoDefaultInputDevice".to_owned());
        return lines;
    };
    lines.push(format!("trace-input: device {device}"));
    match device.supported_input_configs() {
        Ok(ranges) => {
            let supported: Vec<SupportedStreamConfigRange> = ranges.collect();
            let (_, trace) = negotiate_input_traced(&supported, want);
            lines.extend(trace);
        }
        Err(error) => lines.push(format!(
            "trace-input: supported configs unreadable: {error}"
        )),
    }
    lines
}

fn finish(
    range: &SupportedStreamConfigRange,
    want: &EngineConfig,
    sample_rate: u32,
) -> NegotiatedConfig {
    // 请求的块长只有在设备报告的缓冲区间内才写进 Fixed；否则用 Default，
    // 由后端决定（引擎内部渲染量子仍是 DEFAULT_BLOCK_FRAMES）。
    //
    // ⭐ T-FIX-59 复议：旧实现把采样率退回连坐进这里（`!rate_fallback && ...`），
    // 使退回路径**必然** Default —— 块长可否固定与采样率是否退回是两个独立
    // 问题，连坐只会放大设备侧变量。现本函数根本不接收退回标志，块长接受与否
    // 只由设备报告的区间决定；采样率差由 `source_rate` 与 `sample_rate`
    // 两侧数值如实记录（T-FIX-72），转换在设备腿做，不在这里连坐。
    let fixed_block_accepted = match range.buffer_size() {
        SupportedBufferSize::Range { min, max } => (*min..=*max).contains(&want.block_frames),
        SupportedBufferSize::Unknown => false,
    };
    NegotiatedConfig {
        channels: range.channels(),
        sample_rate,
        source_rate: want.sample_rate,
        buffer_size: if fixed_block_accepted {
            BufferSize::Fixed(want.block_frames)
        } else {
            BufferSize::Default
        },
        sample_format: SampleFormat::F32,
        share_mode: ShareMode::Shared,
        fixed_block_accepted,
    }
}

fn describe(supported: &[SupportedStreamConfigRange]) -> String {
    let mut text = String::new();
    for range in supported {
        if !text.is_empty() {
            text.push_str("; ");
        }
        let buffer = match range.buffer_size() {
            SupportedBufferSize::Range { min, max } => format!("{min}..={max}"),
            SupportedBufferSize::Unknown => "unknown".to_owned(),
        };
        let channels = range.channels();
        let min_rate = range.min_sample_rate();
        let max_rate = range.max_sample_rate();
        let sample_format = range.sample_format();
        text.push_str(&format!(
            "{channels}ch {min_rate}-{max_rate}Hz {sample_format:?} buffer {buffer}"
        ));
    }
    text
}

/// 已打开的输出流句柄。
pub struct OutputStreamHandle {
    stream: cpal::Stream,
    /// 打开时协商到的那台设备的显示名（`cpal::Device` 的 `Display`；**控制面**读，
    /// 不在实时路径上）。
    device_name: String,
    negotiated: NegotiatedConfig,
    /// 本次打开的协商 trace（[`negotiate_traced`] 的逐步记录；控制面诊断用，
    /// T-FIX-70）。打开只发生一次、行数个位数 ⇒ 常开收集，调用方按调试
    /// 开关决定打不打印，关闭时输出零差异。
    negotiation_trace: Vec<String>,
    backend_errors: Arc<BackendErrorLog>,
}

impl OutputStreamHandle {
    /// 协商到的配置。
    #[must_use]
    pub const fn negotiated(&self) -> &NegotiatedConfig {
        &self.negotiated
    }

    /// 本次打开的协商 trace（见 [`negotiate_traced`]）。
    #[must_use]
    pub fn negotiation_trace(&self) -> &[String] {
        &self.negotiation_trace
    }

    /// 这台流的设备显示名（`enumerate_output_devices` 的 `DeviceInfo::name` 同源）。
    #[must_use]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// 后端错误回调的**总**次数（运行期 + 关闭/停止期；错误回调只做自增，**从不打印**
    /// [红线 7]）。
    ///
    /// 口径与旧版**逐位相同**：分档只增加读数，不改变这个数。
    #[must_use]
    pub fn backend_errors(&self) -> u64 {
        self.backend_errors.counts().total()
    }

    /// 后端错误的两档读数 + 首条错误的种类/消息（控制面读）。
    #[must_use]
    pub fn backend_error_counts(&self) -> BackendErrorCounts {
        self.backend_errors.counts()
    }

    /// 启动流（cpal 建的流默认是停止的，必须显式 `play`）。
    ///
    /// 成功后错误回调重新计入**运行期**（`pause` 之后的 `play` 不把新错误算成关闭期）。
    ///
    /// # Errors
    ///
    /// 设备消失 / 流失效 → [`DeviceError::Backend`]。
    pub fn play(&self) -> Result<(), DeviceError> {
        self.backend_errors.begin_run();
        self.stream.play()?;
        Ok(())
    }

    /// 暂停流。**先**把阶段切到关闭/停止期，再暂停 ⇒ 暂停/关流阶段到达的错误单独计。
    ///
    /// # Errors
    ///
    /// 后端不支持暂停 → [`DeviceError::Backend`]。
    pub fn pause(&self) -> Result<(), DeviceError> {
        self.backend_errors.begin_stop();
        self.stream.pause()?;
        Ok(())
    }
}

impl std::fmt::Debug for OutputStreamHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputStreamHandle")
            .field("device_name", &self.device_name)
            .field("negotiated", &self.negotiated)
            .field("backend_errors", &self.backend_errors())
            .finish()
    }
}

/// **cpal 数据回调的完整负载** —— 本函数就是"回调体"本身。
///
/// 它存在的唯一理由，是让这条路径**在不打开设备的前提下可被判据执行**：
/// [`open_output`] 建流的闭包、[`NullBackend::render`] 与
/// `crates/yeban-engine/tests/rt_zero_alloc.rs` 的判据 ⑳ 调用**同一个**函数。
/// 因此"往回调里多加了一点事"不再是只能靠读源码发现的事 —— 计数型分配器与
/// 锁/I-O 探针会当场变红。
///
/// 硬要求 #2（模块文档）的字面含义就是本函数体只有一行：**只有** `process_quantum`。
/// ⛔ 在这里加任何东西（`Vec::with_capacity`、`Mutex::lock`、`println!`、`drop`）
/// 都会让判据 ⑳ 的四元组（分配 / 释放 / 锁 / 阻塞 I/O）变红。
///
/// 它是实时路径：零分配、零锁、零阻塞 I/O、零日志 [红线 7 / `MUST-GATE-001`]。
pub fn render_callback(runtime: &mut EngineRuntime, data: &mut [f32], channels: u16) {
    runtime.process_quantum(data, channels);
}

/// **相邻回调播放时间戳的跳跃帧数**（纯函数，便于无声卡单测）。
///
/// 入参：上一次 `OutputCallbackInfo::timestamp().playback`（首回调为 `None`）、
/// 本次播放时间戳、本次缓冲**期望帧数**、设备采样率（Hz）。
///
/// 口径：`elapsed_frames = playback 间隔 × sample_rate`；返回值是
/// `elapsed_frames − expected_frames` 的**饱和差**（`0` = 没有跳跃）。
///
/// - 首回调（`previous = None`）⇒ `0`（没有参照，不报假 xrun）；
/// - 时间戳回退/非单调（`checked_duration_since` 为 `None`）⇒ `0`（不把时钟回拨
///   当成掉帧）；
/// - `expected_frames == 0` 或 `sample_rate == 0` ⇒ `0`（读数无效，不编数字）。
///
/// ⚠ 设备后端可能在缓冲边界上有 ±1 帧的取整抖动；本函数按**整数帧**向下取整，
/// 只有真正超过一个完整缓冲的间隔才算跳跃。返回值单位是**帧**（不是毫秒）。
#[must_use]
pub fn callback_gap_frames(
    previous_playback: Option<cpal::StreamInstant>,
    playback: cpal::StreamInstant,
    expected_frames: usize,
    sample_rate: u32,
) -> u64 {
    let Some(previous) = previous_playback else {
        return 0;
    };
    let Ok(expected_frames) = u64::try_from(expected_frames) else {
        return 0;
    };
    if expected_frames == 0 || sample_rate == 0 {
        return 0;
    }
    let Some(elapsed) = playback.checked_duration_since(previous) else {
        return 0;
    };
    let elapsed_frames = elapsed.as_nanos().saturating_mul(u128::from(sample_rate)) / 1_000_000_000;
    let elapsed_frames = u64::try_from(elapsed_frames).unwrap_or(u64::MAX);
    elapsed_frames.saturating_sub(expected_frames)
}

/// **回调时间戳观测器**（每个输出流一个；住在 cpal 数据回调的闭包里）。
///
/// 只保存上一次播放时间戳与设备采样率；[`Self::observe`] 只用整数算术，
/// **零分配、零锁、零 I/O**（红线 7）。
pub struct CallbackGapObserver {
    sample_rate: u32,
    previous_playback: Option<cpal::StreamInstant>,
}

impl CallbackGapObserver {
    /// 新建观测器（首个回调没有参照）。
    #[must_use]
    pub const fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            previous_playback: None,
        }
    }

    /// 观察一次回调并把本次播放时间戳记为下一次的参照；返回跳跃帧数。
    pub fn observe(&mut self, playback: cpal::StreamInstant, expected_frames: usize) -> u64 {
        let gap = callback_gap_frames(
            self.previous_playback,
            playback,
            expected_frames,
            self.sample_rate,
        );
        self.previous_playback = Some(playback);
        gap
    }
}

/// **带时间戳观测的等率输出回调体** —— [`open_output`] 的直通腿调用它。
///
/// 顺序固定：先算跳跃（纯算术）⇒ [`EngineRuntime::record_callback_gap`] 记账
/// ⇒ 仍然只调用 [`render_callback`] 渲染。⛔ 不改变渲染输出、⛔ 不分配/加锁/打印。
pub fn render_callback_observed(
    runtime: &mut EngineRuntime,
    data: &mut [f32],
    channels: u16,
    info: &cpal::OutputCallbackInfo,
    observer: &mut CallbackGapObserver,
) {
    let channels = usize::from(channels.max(1));
    let expected_frames = data.len() / channels;
    let gap = observer.observe(info.timestamp().playback, expected_frames);
    runtime.record_callback_gap(gap);
    render_callback(runtime, data, channels as u16);
}

/// ⭐ **带采样率转换的输出回调状态**（T-FIX-72）：设备率 ≠ 工程率时，
/// [`open_output`] 的闭包持有的就是它（而不是裸的 [`EngineRuntime`]）。
///
/// 结构：运行时仍按**工程率**渲染（走带、事件、快照的时基一位不动），
/// 每次渲染一个 128 帧量子进预分配的 `scratch`，再由
/// [`crate::resample::RateConverter`] 换成**设备率**填进 cpal 的缓冲。
/// 转换器与暂存都在构造期分配 ⇒ 回调稳态零分配（判据 ㊻ 与 ⑳ 同口径钉住）。
pub struct ConvertedOutput {
    runtime: EngineRuntime,
    converter: crate::resample::RateConverter,
    /// 一个工程率量子的交错暂存（`DEFAULT_BLOCK_FRAMES × channels`，构造期分配）。
    scratch: Box<[f32]>,
}

impl ConvertedOutput {
    /// 按协商结果组装。`negotiated.resampling()` 为假（等率）⇒ 返回 `None`
    /// （调用方必须走 [`render_callback`] 的逐位直通路径，⛔ 不许"顺手"过一遍
    /// 转换器 —— 等率直通是回归判据钉住的现行为）。
    #[must_use]
    pub fn new(runtime: EngineRuntime, negotiated: &NegotiatedConfig) -> Option<Self> {
        let converter = crate::resample::RateConverter::new(
            negotiated.source_rate,
            negotiated.sample_rate,
            negotiated.channels,
        )?;
        Some(Self::from_converter(
            runtime,
            converter,
            negotiated.channels,
        ))
    }

    /// 按显式两侧采样率组装（判据 ㊻ 用：集成判据构造不出 `cpal` 的协商类型，
    /// 但必须与产品路径共用同一个组装点，⛔ 不许判据专用近路）。
    #[must_use]
    pub fn with_rates(
        runtime: EngineRuntime,
        source_rate: u32,
        device_rate: u32,
        channels: u16,
    ) -> Option<Self> {
        let converter = crate::resample::RateConverter::new(source_rate, device_rate, channels)?;
        Some(Self::from_converter(runtime, converter, channels))
    }

    /// 用一台**已建好**的转换器装配（[`open_output`] 用：先在运行时之外建
    /// 转换器来决定走哪条腿，再把运行时装进来 —— 避免"先按值交出运行时、
    /// 失败再退回"的借用死结，也不需要任何 panic 路径）。
    pub(crate) fn from_converter(
        runtime: EngineRuntime,
        converter: crate::resample::RateConverter,
        channels: u16,
    ) -> Self {
        let frames = DEFAULT_BLOCK_FRAMES * usize::from(channels.max(1));
        Self {
            runtime,
            converter,
            scratch: vec![0.0f32; frames].into_boxed_slice(),
        }
    }

    /// 转换器（控制面读两侧采样率与帧数记账）。
    #[must_use]
    pub const fn converter(&self) -> &crate::resample::RateConverter {
        &self.converter
    }

    /// 运行时统计（与 [`EngineRuntime::stats`] 同一份读数）。
    #[must_use]
    pub fn stats(&self) -> EngineStats {
        self.runtime.stats()
    }
}

impl std::fmt::Debug for ConvertedOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConvertedOutput")
            .field("converter", &self.converter)
            .field("stats", &self.stats())
            .finish()
    }
}

/// **带转换的 cpal 数据回调的完整负载** —— 与 [`render_callback`] 同款理由
/// 抽成具名函数：[`open_output`] 在异率时建流的闭包与
/// `crates/yeban-engine/tests/rt_zero_alloc.rs` 的判据 ㊻ 调用**同一个**
/// 函数 ⇒ 往这条路径里加分配/锁/I-O 会当场变红。
///
/// 推进形态：`data` 是**设备率**的交错缓冲；循环「渲染一个工程率量子到
/// 暂存 ⇒ 逐帧喂转换器 ⇒ 转换器把设备率帧排进 `data`」直到 `data` 的整帧
/// 部分填满（尾部不足一帧的样本保持原值，与 [`crate::rt::EngineRuntime::process_quantum`]
/// 的既有契约一致）。转换器自带待排输出：上一个回调没排完的帧在本回调
/// 先排（`push_frame` 的排空尝试），顺序不乱、样本不丢。
///
/// 实时约束（红线 7）：零分配、零锁、零阻塞 I/O、零日志 —— 渲染走
/// [`render_callback`]（判据 ⑳ 已钉），转换只做定长乘加（判据 ㊻ 钉）。
pub fn render_callback_converted(state: &mut ConvertedOutput, data: &mut [f32], channels: u16) {
    let ConvertedOutput {
        runtime,
        converter,
        scratch,
    } = state;
    let channels = usize::from(channels.max(1));
    let total_frames = data.len() / channels;
    let mut filled = 0usize;
    while filled < total_frames {
        render_callback(runtime, scratch, channels as u16);
        let frames = scratch.len() / channels;
        for index in 0..frames {
            let frame = &scratch[index * channels..(index + 1) * channels];
            // ⚠ `frame` 借 `scratch`、`converter` 是另一字段：两者是不相交
            // 字段借用（已由上面的解构拆开），编译期成立。
            //
            // `data` 填满后余下的帧**仍要喂完**：已渲染出来的样本丢掉任何
            // 一个都会让流错位；它们的输出留在转换器里待排，下一个回调
            // 先排（`push_frame` 对满的 `out` 只收不排）。追加量 ≤ 一个
            // 量子，落在转换器的历史深度不变式内（见 `resample` 模块文档）。
            converter.push_frame(frame, data, &mut filled);
        }
    }
}

/// **带时间戳观测的异率输出回调体** —— [`open_output`] 的转换腿调用它。
///
/// 与 [`render_callback_observed`] 同一条顺序：先算跳跃并记账，再走
/// [`render_callback_converted`]（渲染仍只经 [`render_callback`]）。帧数按**设备率**
/// 的交错缓冲计算（`data.len() / channels`），与 cpal 的播放时间戳同一时基。
pub fn render_callback_converted_observed(
    state: &mut ConvertedOutput,
    data: &mut [f32],
    channels: u16,
    info: &cpal::OutputCallbackInfo,
    observer: &mut CallbackGapObserver,
) {
    let channels_usize = usize::from(channels.max(1));
    let expected_frames = data.len() / channels_usize;
    let gap = observer.observe(info.timestamp().playback, expected_frames);
    state.runtime.record_callback_gap(gap);
    render_callback_converted(state, data, channels);
}

/// **音频回调健康读数的一行诊断文本**（UI 状态行/`--latency` 共用同一个纯函数）。
///
/// `xrun_count == 0` ⇒ 明确写"连续"；非零时写出次数与最大跳跃帧数，并在
/// `sample_rate > 0` 时附带毫秒换算。⛔ 不把"后端错误计数"混进来（那是另一个读数）。
#[must_use]
pub fn audio_callback_health_line(stats: &EngineStats, sample_rate: u32) -> String {
    if stats.xrun_count == 0 {
        return "音频回调：连续（xrun 0）".to_owned();
    }
    if sample_rate == 0 {
        return format!(
            "音频回调：xrun {} 次 · 最大间隔 {} 帧",
            stats.xrun_count, stats.max_gap_frames
        );
    }
    let max_ms = stats.max_gap_frames as f64 * 1_000.0 / f64::from(sample_rate);
    format!(
        "音频回调：xrun {} 次 · 最大间隔 {} 帧（{max_ms:.1} ms）",
        stats.xrun_count, stats.max_gap_frames
    )
}

/// 以已装箱的 [`EngineRuntime`] 打开默认输出设备，避免在调用栈上按值搬运整份运行时
/// （见 [`open_output`]）。
///
/// 真实设备相关的一切（枚举能力、协商、建流）都在这里，并且**只返回 `Result`**。
/// 无声卡环境下返回 [`DeviceError::NoDefaultOutputDevice`]，绝不 panic。
///
/// # Errors
///
/// 同 [`open_output`]。
pub fn open_output_boxed(
    want: &EngineConfig,
    runtime: Box<EngineRuntime>,
) -> Result<OutputStreamHandle, DeviceError> {
    open_output_boxed_impl(want, runtime, None)
}

/// ⭐ **按名打开输出设备**（T-FIX-71）：`device_name` 是用户在音频设置里
/// 选定的设备显示名（来自 [`enumerate_output_devices`] 的 [`DeviceInfo::name`]）。
///
/// 与 [`open_output_boxed`] 的唯一差别是**选设备那一步**：经
/// [`select_output_device`] 精确命中；指定的设备不在场 ⇒
/// [`DeviceError::NamedOutputDeviceUnavailable`]（点名，⛔ 不静默回退默认
/// 设备）。协商、建流、回调腿与默认路径**同一份实现**（本函数只把名字
/// 传进同一条 impl），⛔ 不复制第二份打开逻辑。
///
/// # Errors
///
/// 同 [`open_output`]，另加 [`DeviceError::NamedOutputDeviceUnavailable`]。
pub fn open_output_boxed_named(
    want: &EngineConfig,
    runtime: Box<EngineRuntime>,
    device_name: Option<&str>,
) -> Result<OutputStreamHandle, DeviceError> {
    open_output_boxed_impl(want, runtime, device_name)
}

/// [`open_output_boxed`] 与 [`open_output_boxed_named`] 的**唯一实现**：
/// `device_name == None` = 默认设备（现行为）；`Some(名)` = 按名精确选。
fn open_output_boxed_impl(
    want: &EngineConfig,
    mut runtime: Box<EngineRuntime>,
    device_name: Option<&str>,
) -> Result<OutputStreamHandle, DeviceError> {
    if want.share_mode == ShareMode::RequireExclusive {
        return Err(DeviceError::ExclusiveModeUnsupported);
    }
    let host = cpal::default_host();
    let device: Device = select_output_device(&host, device_name)?;
    let device_name = device.to_string();
    let supported: Vec<SupportedStreamConfigRange> = device.supported_output_configs()?.collect();
    let (negotiated, negotiation_trace) = negotiate_traced(&supported, want);
    let negotiated = negotiated?;

    let stream_config = StreamConfig {
        channels: negotiated.channels,
        sample_rate: negotiated.sample_rate,
        buffer_size: negotiated.buffer_size,
    };
    let channels = negotiated.channels;
    let backend_errors = Arc::new(BackendErrorLog::new());
    let error_counter = Arc::clone(&backend_errors);
    let error_counter_converted = Arc::clone(&backend_errors);

    // 数据回调分两条腿（都是**具名函数**，都受判据钉住）：
    // - 等率 ⇒ `render_callback`（= 只有 `process_quantum`，判据 ⑳）—— 逐位直通；
    // - 异率 ⇒ `render_callback_converted`（渲染仍走 `render_callback`，
    //   外加预分配的流式采样率转换，判据 ㊻）—— T-FIX-72。
    // 先在运行时之外把转换器建好（`None` = 等率/不可转换 ⇒ 直通腿），
    // 再决定运行时装进哪条腿 —— ⛔ 不先按值交出再退回。
    // 错误回调: 只做原子计数与固定缓冲拷贝（零分配 / 零锁 / 零 I/O；打印禁止 [红线 7]）。
    let converter = crate::resample::RateConverter::new(
        negotiated.source_rate,
        negotiated.sample_rate,
        negotiated.channels,
    );
    let stream = if let Some(converter) = converter {
        let mut state = ConvertedOutput::from_converter(*runtime, converter, channels);
        let mut gap_observer = CallbackGapObserver::new(negotiated.sample_rate);
        device.build_output_stream::<f32, _, _>(
            stream_config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                render_callback_converted_observed(
                    &mut state,
                    data,
                    channels,
                    info,
                    &mut gap_observer,
                );
            },
            move |error: cpal::Error| {
                error_counter_converted.record(&error);
            },
            Some(STREAM_OPEN_TIMEOUT),
        )?
    } else {
        let mut gap_observer = CallbackGapObserver::new(negotiated.sample_rate);
        device.build_output_stream::<f32, _, _>(
            stream_config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                render_callback_observed(&mut runtime, data, channels, info, &mut gap_observer);
            },
            move |error: cpal::Error| {
                error_counter.record(&error);
            },
            Some(STREAM_OPEN_TIMEOUT),
        )?
    };

    Ok(OutputStreamHandle {
        stream,
        device_name,
        negotiated,
        negotiation_trace,
        backend_errors,
    })
}

/// 用给定配置与预构造的 [`EngineRuntime`] 打开默认输出设备。
///
/// 实际实现见 [`open_output_boxed`]；调用方已持有 `Box<EngineRuntime>` 时优先用后者。
///
/// # Errors
///
/// - [`DeviceError::ExclusiveModeUnsupported`]：要求了独占模式；
/// - [`DeviceError::NoDefaultOutputDevice`]：没有默认输出设备（CI 常态）；
/// - [`DeviceError::NoSupportedConfigs`] / [`DeviceError::UnsupportedConfig`] /
///   [`DeviceError::UnsupportedSampleFormat`]：能力不匹配；
/// - [`DeviceError::Backend`]：后端错误（设备忙、权限、配置失效……）。
pub fn open_output(
    want: &EngineConfig,
    runtime: EngineRuntime,
) -> Result<OutputStreamHandle, DeviceError> {
    open_output_boxed(want, Box::new(runtime))
}

/// ⭐ **按名打开输出设备**（T-FIX-71）：[`open_output_boxed_named`] 的
/// 未装箱版（调用方持有按值 `EngineRuntime` 时用）。语义与错误同它。
///
/// # Errors
///
/// 同 [`open_output_boxed_named`]。
pub fn open_output_named(
    want: &EngineConfig,
    runtime: EngineRuntime,
    device_name: Option<&str>,
) -> Result<OutputStreamHandle, DeviceError> {
    open_output_boxed_named(want, Box::new(runtime), device_name)
}

/// **cpal 输入回调的完整负载** —— 本函数就是"录音回调体"本身。
///
/// 与 [`render_callback`] 同款理由：让这条路径**在不打开设备的前提下可被判据执行**。
/// [`open_input`] 建流的闭包与判据调用**同一个**函数 ⇒ "往录音回调里多加了一点事"
/// 不再是只能靠读源码发现的事。
///
/// # 实时约束（红线 7）
///
/// 只有一行：[`yeban_record::push_input_block`] 的**无锁推入**。
/// ⛔ 零分配、零锁、零阻塞 I/O、零日志。环满时它**按帧计数丢弃**，⛔ 不阻塞、⛔ 不分配。
pub fn capture_callback(sender: &mut RecordSender, data: &[f32], channels: u16) {
    let _ = push_input_block(sender, data, channels);
}

/// 已打开的**输入**流句柄（录音侧）。
///
/// 与 [`OutputStreamHandle`] 同构：`stream` 一丢，回调就停 ⇒ "谁持有它"就是生命期事实。
pub struct InputStreamHandle {
    stream: cpal::Stream,
    /// 打开时协商到的那台设备的显示名（控制面读，不在实时路径上）。
    device_name: String,
    negotiated: NegotiatedConfig,
    /// 本次打开的协商 trace（[`negotiate_input_traced`] 的逐步记录；
    /// 与输出侧同口径，见 [`OutputStreamHandle::negotiation_trace`]）。
    negotiation_trace: Vec<String>,
    backend_errors: Arc<BackendErrorLog>,
}

impl InputStreamHandle {
    /// 协商到的配置。
    #[must_use]
    pub const fn negotiated(&self) -> &NegotiatedConfig {
        &self.negotiated
    }

    /// 本次打开的协商 trace（见 [`negotiate_input_traced`]）。
    #[must_use]
    pub fn negotiation_trace(&self) -> &[String] {
        &self.negotiation_trace
    }

    /// 这台流的设备显示名。
    #[must_use]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// 后端错误回调的**总**次数（运行期 + 关闭/停止期；只增的原子量）。
    #[must_use]
    pub fn backend_errors(&self) -> u64 {
        self.backend_errors.counts().total()
    }

    /// 后端错误的两档读数 + 首条错误的种类/消息（控制面读）。
    #[must_use]
    pub fn backend_error_counts(&self) -> BackendErrorCounts {
        self.backend_errors.counts()
    }

    /// 启动捕获。
    ///
    /// 成功后错误回调重新计入**运行期**。
    ///
    /// # Errors
    ///
    /// 设备消失 / 流失效 → [`DeviceError::Backend`]。
    pub fn play(&self) -> Result<(), DeviceError> {
        self.backend_errors.begin_run();
        self.stream.play()?;
        Ok(())
    }

    /// 暂停捕获。**先**把阶段切到关闭/停止期，再暂停。
    ///
    /// # Errors
    ///
    /// 后端不支持暂停 → [`DeviceError::Backend`]。
    pub fn pause(&self) -> Result<(), DeviceError> {
        self.backend_errors.begin_stop();
        self.stream.pause()?;
        Ok(())
    }
}

impl std::fmt::Debug for InputStreamHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputStreamHandle")
            .field("device_name", &self.device_name)
            .field("negotiated", &self.negotiated)
            .field("backend_errors", &self.backend_errors())
            .finish()
    }
}

/// 打开默认**输入**设备，并把捕获到的样本推进 [`RecordSender`]（录音侧）。
///
/// # 为什么单独一个函数而不是给 [`open_output`] 加参数
///
/// 输出与输入是**两条独立的流**：一台机器可以只有输出、只有输入、或都有；它们的
/// 采样率/通道数也**可以不同**（`negotiate_input` 已单独存在）。塞进一个函数会让
/// "没有输入设备"与"没有输出设备"这两种**排查方向完全不同**的失败混在一个错误里
/// —— 那正是 [`DeviceError::NoDefaultInputDevice`] 文档点名要避免的。
///
/// # 失败要响亮（⛔ 不是静默静音）
///
/// 拿不到设备/拿不到配置/协商失败时**一律返回 `Result` 的 `Err`**，且理由**结构化**
/// （见 [`DeviceError`]）。⛔ **绝不**"打不开就当录到静音"地返回成功 —— 那会让调用方
/// 拿到一段静音却以为录音在跑。
///
/// # Errors
///
/// - [`DeviceError::ExclusiveModeUnsupported`]：要求了独占模式；
/// - [`DeviceError::NoDefaultInputDevice`]：没有默认输入设备（本机常态）；
/// - [`DeviceError::NoSupportedInputConfigs`] / [`DeviceError::UnsupportedConfig`] /
///   [`DeviceError::UnsupportedSampleFormat`]：能力不匹配；
/// - [`DeviceError::Backend`]：后端错误（设备忙、权限、配置失效……）。
pub fn open_input(
    want: &EngineConfig,
    sender: RecordSender,
) -> Result<InputStreamHandle, DeviceError> {
    open_input_impl(want, sender, None)
}

/// ⭐ **按名打开输入设备**（T-FIX-71）：`device_name` 是用户在音频设置里
/// 选定的输入设备显示名（来自 [`enumerate_input_devices`]）。
///
/// 与 [`open_input`] 的唯一差别是选设备那一步：经 [`select_input_device`]
/// 精确命中；不在场 ⇒ [`DeviceError::NamedInputDeviceUnavailable`]
/// （点名，⛔ 不静默回退默认输入设备）。协商与建流是**同一份 impl**。
///
/// # Errors
///
/// 同 [`open_input`]，另加 [`DeviceError::NamedInputDeviceUnavailable`]。
pub fn open_input_named(
    want: &EngineConfig,
    sender: RecordSender,
    device_name: Option<&str>,
) -> Result<InputStreamHandle, DeviceError> {
    open_input_impl(want, sender, device_name)
}

/// [`open_input`] 与 [`open_input_named`] 的**唯一实现**：
/// `device_name == None` = 默认输入设备（现行为）；`Some(名)` = 按名精确选。
fn open_input_impl(
    want: &EngineConfig,
    mut sender: RecordSender,
    device_name: Option<&str>,
) -> Result<InputStreamHandle, DeviceError> {
    if want.share_mode == ShareMode::RequireExclusive {
        return Err(DeviceError::ExclusiveModeUnsupported);
    }
    let host = cpal::default_host();
    let device: Device = select_input_device(&host, device_name)?;
    let device_name = device.to_string();
    let supported: Vec<SupportedStreamConfigRange> = device.supported_input_configs()?.collect();
    let (negotiated, negotiation_trace) = negotiate_input_traced(&supported, want);
    let negotiated = negotiated?;

    let stream_config = StreamConfig {
        channels: negotiated.channels,
        sample_rate: negotiated.sample_rate,
        buffer_size: negotiated.buffer_size,
    };
    let channels = negotiated.channels;
    let backend_errors = Arc::new(BackendErrorLog::new());
    let error_counter = Arc::clone(&backend_errors);

    // 数据回调：只有 `capture_callback`（= 只有无锁推入）——
    // 零分配 / 零锁 / 零 I/O / 零日志 [红线 7]。回调体是**具名函数**，见它的文档。
    let stream = device.build_input_stream::<f32, _, _>(
        stream_config,
        move |data: &[f32], _info: &cpal::InputCallbackInfo| {
            capture_callback(&mut sender, data, channels);
        },
        // 错误回调：只做原子计数与固定缓冲拷贝（零分配 / 零锁 / 零 I/O；打印禁止 [红线 7]）。
        move |error: cpal::Error| {
            error_counter.record(&error);
        },
        Some(STREAM_OPEN_TIMEOUT),
    )?;

    Ok(InputStreamHandle {
        stream,
        device_name,
        negotiated,
        negotiation_trace,
        backend_errors,
    })
}

/// ⭐ 设备测试音的频率（Hz）（T-FIX-71）：1 kHz 是 Logic 等宿主 I/O 测试的
/// 通行基准（人耳敏感、任何扬声器都放得出、示波/电平读数都好判）。
pub const TEST_TONE_FREQ_HZ: f64 = 1_000.0;

/// ⭐ 设备测试音的峰值电平（dBFS）：−12 dBFS —— 听得清，又离满幅足够远，
/// ⛔ 不会因为设备增益偏大就削波吓人。
pub const TEST_TONE_LEVEL_DBFS: f64 = -12.0;

/// ⭐ 设备测试音的时长（秒）：约 1 秒（够听清"响了"，不至于变成噪音折磨）。
pub const TEST_TONE_SECONDS: f64 = 1.0;

/// ⭐ **测试音回调的状态**（T-FIX-71）：构造期把相位步长与总帧数算好，
/// 回调里只推进 —— 零分配 / 零锁 / 零 I/O（红线 7，与 [`render_callback`]
/// 同纪律）。总帧数之后输出**静音**（流还开着，由持有方丢弃句柄收尾）。
pub struct TestToneState {
    /// 当前相位（弧度，恒在 `[0, 2π)` 内回绕，长时间运行不丢精度）。
    phase: f64,
    /// 每帧的相位增量（构造期由频率/采样率算出）。
    phase_step: f64,
    /// 峰值振幅（线性的，由 [`TEST_TONE_LEVEL_DBFS`] 换算）。
    amplitude: f32,
    /// 已经发出的帧数（声道交错计数以**帧**为单位，⛔ 不是样本数）。
    frames_done: u64,
    /// 测试音的总帧数（[`TEST_TONE_SECONDS`] × 采样率，取整）。
    total_frames: u64,
}

impl TestToneState {
    /// 按设备采样率构造（通道数不进状态：每帧各声道写**同一个**样本值，
    /// 通道数只决定回调里每帧写几份，由回调的 `channels` 参数给）。
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self {
            phase: 0.0,
            phase_step: 2.0 * core::f64::consts::PI * TEST_TONE_FREQ_HZ / f64::from(sample_rate),
            amplitude: libm::pow(10.0, TEST_TONE_LEVEL_DBFS / 20.0) as f32,
            frames_done: 0,
            total_frames: (TEST_TONE_SECONDS * f64::from(sample_rate)).round() as u64,
        }
    }

    /// 已发出的帧数（控制面/判据读：判断测试音是否已播完）。
    #[must_use]
    pub const fn frames_done(&self) -> u64 {
        self.frames_done
    }

    /// 测试音的总帧数（播完 = `frames_done() >= total_frames()`）。
    #[must_use]
    pub const fn total_frames(&self) -> u64 {
        self.total_frames
    }
}

/// ⭐ **测试音回调体**（T-FIX-71）：与 [`render_callback`] 同款理由抽成
/// 具名函数 —— "往测试音里多加一点事"从此是可被判据执行的对象。
///
/// 行为：前 [`TestToneState::total_frames`] 帧输出 1 kHz 正弦（各声道同相
/// 同值），其后输出静音。⛔ 只有算术与写出：零分配、零锁、零 I/O、零日志。
pub fn test_tone_callback(state: &mut TestToneState, data: &mut [f32], channels: u16) {
    let channels = usize::from(channels.max(1));
    for frame in data.chunks_exact_mut(channels) {
        let value = if state.frames_done < state.total_frames {
            let value = state.amplitude * libm::sin(state.phase) as f32;
            state.phase += state.phase_step;
            if state.phase >= 2.0 * core::f64::consts::PI {
                state.phase -= 2.0 * core::f64::consts::PI;
            }
            value
        } else {
            0.0
        };
        state.frames_done += 1;
        frame.fill(value);
    }
}

/// 已打开的**测试音**流句柄（T-FIX-71）：持有它 = 测试音流在场；
/// 丢弃它 = 流停（测试音播完后由持有方丢弃收尾）。
pub struct TestToneHandle {
    stream: cpal::Stream,
    device_name: String,
    sample_rate: u32,
    channels: u16,
    backend_errors: Arc<BackendErrorLog>,
}

impl TestToneHandle {
    /// 实际打开的设备显示名（控制面回显"在哪台设备上播"）。
    #[must_use]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// 流的采样率（= 设备默认配置的采样率）。
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 流的通道数（= 设备默认配置的通道数）。
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.channels
    }

    /// 本流累计的后端错误数（与 [`OutputStreamHandle::backend_errors`] 同口径）。
    #[must_use]
    pub fn backend_errors(&self) -> u64 {
        self.backend_errors.counts().total()
    }

    /// 开始播放（流打开时是暂停的，必须显式 `play`，与其它句柄同口径）。
    ///
    /// # Errors
    ///
    /// 后端拒绝启动 → [`DeviceError::Backend`]。
    pub fn play(&self) -> Result<(), DeviceError> {
        self.stream.play()?;
        Ok(())
    }
}

/// ⭐ **在指定（或默认）输出设备上打开一条测试音流**（T-FIX-71，Logic 口径
/// 的「播放测试音」引擎段）。
///
/// 配置口径：用设备的**默认输出配置**（采样率/通道数/格式），⛔ 不走工程
/// 协商 —— 测试音要测的是"这台设备本身通不通"，与工程采样率无关。
/// 格式不是 `f32` ⇒ [`DeviceError::UnsupportedSampleFormat`]（响亮拒绝，
/// ⛔ 不静默换格式）。指定的设备不在场 ⇒
/// [`DeviceError::NamedOutputDeviceUnavailable`]（点名，⛔ 不回退默认）。
///
/// 返回的句柄**还没在播**：调用方 `play()` 后测试音开始，约
/// [`TEST_TONE_SECONDS`] 秒后回调自动转静音，调用方丢弃句柄即收尾。
///
/// # Errors
///
/// - [`DeviceError::NoDefaultOutputDevice`] / [`DeviceError::NamedOutputDeviceUnavailable`]：选设备失败；
/// - [`DeviceError::UnsupportedSampleFormat`]：默认配置不是 `f32`；
/// - [`DeviceError::Backend`]：读配置 / 建流被后端拒绝。
pub fn play_test_tone(device_name: Option<&str>) -> Result<TestToneHandle, DeviceError> {
    let host = cpal::default_host();
    let device: Device = select_output_device(&host, device_name)?;
    let device_name = device.to_string();
    let config = device.default_output_config()?;
    let sample_format = config.sample_format();
    if sample_format != SampleFormat::F32 {
        return Err(DeviceError::UnsupportedSampleFormat {
            format: sample_format,
        });
    }
    let stream_config = StreamConfig {
        channels: config.channels(),
        sample_rate: config.sample_rate(),
        buffer_size: BufferSize::Default,
    };
    let channels = config.channels();
    let backend_errors = Arc::new(BackendErrorLog::new());
    let error_counter = Arc::clone(&backend_errors);
    let mut state = TestToneState::new(config.sample_rate());
    let stream = device.build_output_stream::<f32, _, _>(
        stream_config,
        move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
            test_tone_callback(&mut state, data, channels);
        },
        move |error: cpal::Error| {
            error_counter.record(&error);
        },
        Some(STREAM_OPEN_TIMEOUT),
    )?;
    Ok(TestToneHandle {
        stream,
        device_name,
        sample_rate: config.sample_rate(),
        channels,
        backend_errors,
    })
}

/// 不打开任何真实设备的宿主：用**同一份** [`EngineRuntime`] 在内存里推进渲染。
///
/// 用途：
///
/// - CI / 无声卡环境下的端到端回调路径覆盖（事件出队 → 快照切换 → 电平上报 → 退役入队）；
/// - 离线/无头场景下驱动引擎（例如单元测试与 MCP 的无音频模式）；
/// - 与真实回调的差异被压到最小：唯一不同是"谁来推进缓冲区"。
///
/// ⚠ **已知缺口（`line/engine-22` 登记，未修）**：下面这个交错暂存的容量恒为
/// **2 声道**（`DEFAULT_BLOCK_FRAMES * 2` = 256 样本），而 `render` 按协商到的
/// 通道数去切它 ⇒ `negotiated.channels >= 3` 时是一次**越界 panic**
/// （同一形态的替身实测：`range end index 384 out of range for slice of length 256`）。
/// 可达性由本模块的判据
/// `negotiate_accepts_wider_channel_count_when_stereo_is_unavailable` 证明：
/// `negotiate` 会返回 4 声道，而 [`Self::new`] 收下它。它是"容量随协商通道数增长"
/// 的一处改动，**需要能编译 `device` feature 的环境**（本机纪律禁止编译 cpal ⇒
/// 本机 check/clippy/test 覆盖不到这个模块）。详见
/// `tests/idempotency_and_channel_consistency.rs` 的模块文档 §4 发现 3。
pub struct NullBackend {
    runtime: EngineRuntime,
    negotiated: NegotiatedConfig,
    interleaved: [f32; DEFAULT_BLOCK_FRAMES * 2],
    frames_rendered: u64,
}

impl NullBackend {
    /// 组装一个无设备宿主。
    #[must_use]
    pub fn new(runtime: EngineRuntime, negotiated: NegotiatedConfig) -> Self {
        Self {
            runtime,
            negotiated,
            interleaved: [0.0; DEFAULT_BLOCK_FRAMES * 2],
            frames_rendered: 0,
        }
    }

    /// 用默认协商结果组装（CI 上无需任何设备信息）。
    #[must_use]
    pub fn with_default_config(runtime: EngineRuntime) -> Self {
        let config = EngineConfig::default();
        Self::new(
            runtime,
            NegotiatedConfig {
                channels: config.channels,
                sample_rate: config.sample_rate,
                source_rate: config.sample_rate,
                buffer_size: BufferSize::Fixed(config.block_frames),
                sample_format: SampleFormat::F32,
                share_mode: ShareMode::Shared,
                fixed_block_accepted: true,
            },
        )
    }

    /// 协商结果。
    #[must_use]
    pub const fn negotiated(&self) -> &NegotiatedConfig {
        &self.negotiated
    }

    /// 渲染驱动（只读）。
    #[must_use]
    pub const fn runtime(&self) -> &EngineRuntime {
        &self.runtime
    }

    /// 渲染驱动（可变；用于测试里发布快照之后继续推进）。
    pub fn runtime_mut(&mut self) -> &mut EngineRuntime {
        &mut self.runtime
    }

    /// 累计推进的帧数。
    #[must_use]
    pub const fn frames_rendered(&self) -> u64 {
        self.frames_rendered
    }

    /// 最近一个量子的输出（用于断言"确实写进了缓冲"）。
    #[must_use]
    pub fn last_block(&self) -> &crate::block::AudioBlock<DEFAULT_BLOCK_FRAMES> {
        self.runtime.last_block()
    }

    /// 推进 `frames` 帧（按 128 帧量子切分，与真实回调同路径）。
    ///
    /// "同路径"是**字面**的：本函数经 [`render_callback`] 调 `process_quantum`，
    /// 与 cpal 建流的闭包是同一个函数。
    pub fn render(&mut self, frames: usize) -> EngineStats {
        let channels = usize::from(self.negotiated.channels.max(1));
        let mut remaining = frames;
        while remaining > 0 {
            let quantum = remaining.min(DEFAULT_BLOCK_FRAMES);
            let samples = quantum * channels;
            // ⚠ `channels >= 3` 时 `samples` 超出 `interleaved` 的 256 样本容量
            // ⇒ 这里是那条越界的现场（见 `NullBackend` 的文档与判据文件的发现 3）。
            render_callback(
                &mut self.runtime,
                &mut self.interleaved[..samples],
                self.negotiated.channels,
            );
            self.frames_rendered = self.frames_rendered.saturating_add(quantum as u64);
            remaining -= quantum;
        }
        self.runtime.stats()
    }
}

impl std::fmt::Debug for NullBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NullBackend")
            .field("negotiated", &self.negotiated)
            .field("frames_rendered", &self.frames_rendered)
            .field("stats", &self.runtime.stats())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::meter_channel;
    use crate::record::record_channel;
    use crate::ring::event_channel;
    use crate::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
    use std::collections::BTreeMap;
    use yeban_model::{EntityId, RoutingGraph};

    fn fake_range(
        channels: u16,
        min_rate: u32,
        max_rate: u32,
        buffer: SupportedBufferSize,
        format: SampleFormat,
    ) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(channels, min_rate, max_rate, buffer, format)
    }

    fn default_runtime() -> EngineRuntime {
        let master = EntityId::new();
        let mut routing = RoutingGraph::default();
        routing.nodes.push(master);
        let snapshot = EngineSnapshot::from_parts(
            0,
            48_000,
            DEFAULT_BLOCK_FRAMES,
            2,
            master,
            BTreeMap::new(),
            &routing,
            &crate::graph::LatencyTable::new(),
        )
        .expect("单节点图合法");
        let slot = SnapshotSlot::new(snapshot);
        let (retire, _queue) = retire_channel(8);
        let (_sender, receiver) = event_channel(16);
        let (publisher, _collector) = meter_channel(64);
        EngineRuntime::new(&slot, retire, receiver, publisher)
    }

    #[test]
    fn negotiate_prefers_exact_match_and_keeps_fixed_block() {
        let ranges = [
            fake_range(
                2,
                48_000,
                48_000,
                SupportedBufferSize::Range { min: 64, max: 1024 },
                SampleFormat::F32,
            ),
            fake_range(
                2,
                44_100,
                44_100,
                SupportedBufferSize::Range { min: 64, max: 1024 },
                SampleFormat::F32,
            ),
        ];
        let config = EngineConfig::default();
        let got = negotiate(&ranges, &config).expect("精确匹配");
        assert_eq!(got.sample_rate, 48_000);
        assert_eq!(got.channels, 2);
        assert_eq!(got.sample_format, SampleFormat::F32);
        assert_eq!(got.buffer_size, BufferSize::Fixed(128));
        assert!(got.fixed_block_accepted);
        assert_eq!(
            got.share_mode,
            ShareMode::Shared,
            "独占不可用 → 如实记录为共享"
        );
    }

    #[test]
    fn negotiate_accepts_rate_fallback_and_marks_conversion() {
        // 设备只支持 44.1k、工程要 48k。
        // - T-FIX-59 之前：静默退回 44.1k 开流（按 44100/48000 降速降调出声，
        //   设备行毫无告警）；
        // - T-FIX-59：响亮拒绝（SampleRateMismatch）—— 在 44100-only 设备
        //   常见的真实环境里等于不出声，Wang 2026-11-22 亲测撞上后改判；
        // - T-FIX-72（现行）：**接受设备率 + 实时采样率转换**：协商成功，
        //   两侧数值都记进结果（`resampling()` 为真），设备腿装转换器、
        //   设备行如实报告转换中 —— 不是静默退回。
        let ranges = [fake_range(
            2,
            44_100,
            44_100,
            SupportedBufferSize::Unknown,
            SampleFormat::F32,
        )];
        let got = negotiate(&ranges, &EngineConfig::default())
            .expect("T-FIX-72: 异率必须协商成功并标记转换，⛔ 不再拒绝");
        assert_eq!(got.sample_rate, 44_100, "设备流跑设备实际率");
        assert_eq!(got.source_rate, 48_000, "引擎侧记工程（请求）率");
        assert!(
            got.resampling(),
            "两侧不同必须标记为转换中（响亮报告的依据）"
        );
        assert_eq!(
            got.buffer_size,
            BufferSize::Default,
            "设备给不出块长区间 ⇒ Default（与采样率无关的独立事实）"
        );
    }

    #[test]
    fn negotiate_rate_fallback_keeps_fixed_block_when_range_fits() {
        // 缓冲区间装得下请求块长 ⇒ Fixed 照样接受：采样率转换不许连坐块长
        //（与 T-FIX-59 对 `finish` 的复议口径一致；旧的拒绝版判据
        // `negotiate_rejects_rate_fallback_even_when_block_size_would_fit`
        // 随 T-FIX-72 的政策反转改写为本条）。
        let ranges = [fake_range(
            2,
            44_100,
            44_100,
            SupportedBufferSize::Range { min: 64, max: 1024 },
            SampleFormat::F32,
        )];
        let got = negotiate(&ranges, &EngineConfig::default())
            .expect("T-FIX-72: 异率 + 块长区间装得下 ⇒ 协商成功");
        assert_eq!(got.sample_rate, 44_100);
        assert_eq!(got.source_rate, 48_000);
        assert!(got.resampling());
        assert_eq!(got.buffer_size, BufferSize::Fixed(128));
        assert!(got.fixed_block_accepted);
    }

    #[test]
    fn negotiate_exact_rate_match_is_not_marked_resampling() {
        // 等率路径必须与转换完全脱钩：`resampling()` 为假 ⇒ 回调走逐位直通。
        let ranges = [fake_range(
            2,
            48_000,
            48_000,
            SupportedBufferSize::Range { min: 64, max: 1024 },
            SampleFormat::F32,
        )];
        let got = negotiate(&ranges, &EngineConfig::default()).expect("精确匹配");
        assert_eq!(got.sample_rate, 48_000);
        assert_eq!(got.source_rate, 48_000);
        assert!(!got.resampling(), "等率不得标记转换");
    }

    #[test]
    fn finish_block_acceptance_depends_only_on_the_reported_range() {
        // T-FIX-59 复议的钉：块长接受与否只看设备报告的区间，与采样率无关。
        // 在一个非请求率（44.1k）上直接调 finish：区间装得下 ⇒ Fixed 被接受
        //（旧实现在退回路径上连坐成恒 Default，这条会红）。
        let want = EngineConfig::default();
        let fitting = fake_range(
            2,
            44_100,
            44_100,
            SupportedBufferSize::Range { min: 64, max: 1024 },
            SampleFormat::F32,
        );
        let got = finish(&fitting, &want, 44_100);
        assert!(got.fixed_block_accepted);
        assert_eq!(got.buffer_size, BufferSize::Fixed(128));
        // 区间装不下（128 < 256）⇒ Default，且只因区间、不因别的。
        let tight = fake_range(
            2,
            44_100,
            44_100,
            SupportedBufferSize::Range { min: 256, max: 512 },
            SampleFormat::F32,
        );
        let got = finish(&tight, &want, 44_100);
        assert!(!got.fixed_block_accepted);
        assert_eq!(got.buffer_size, BufferSize::Default);
        // 设备不报区间 ⇒ 同样不得写 Fixed。
        let unknown = fake_range(
            2,
            44_100,
            44_100,
            SupportedBufferSize::Unknown,
            SampleFormat::F32,
        );
        let got = finish(&unknown, &want, 44_100);
        assert!(!got.fixed_block_accepted);
        assert_eq!(got.buffer_size, BufferSize::Default);
    }

    #[test]
    fn negotiate_traced_records_each_step_and_agrees_with_negotiate() {
        // T-FIX-70：trace 版与裸版必须是同一条实现的两个出口 —— 结果逐值
        // 一致，且 trace 逐步可读（命中第 1 步时不再往下走）。
        let ranges = [fake_range(
            2,
            44_100,
            48_000,
            SupportedBufferSize::Range { min: 64, max: 1024 },
            SampleFormat::F32,
        )];
        let want = EngineConfig::default();
        let (traced, trace) = negotiate_traced(&ranges, &want);
        let plain = negotiate(&ranges, &want);
        assert_eq!(traced.expect("traced 应成功"), plain.expect("裸版应成功"));
        assert!(
            trace
                .iter()
                .any(|line| line.contains("step 1") && line.contains("matched")),
            "trace 缺第 1 步命中行: {trace:?}"
        );
        assert!(
            !trace.iter().any(|line| line.contains("step 2")),
            "第 1 步命中后不应再试第 2 步: {trace:?}"
        );
        let last = trace.last().expect("trace 至少有结果行");
        assert!(
            last.contains("result") && last.contains("48000"),
            "trace 末行应是最终结果: {last}"
        );
    }

    #[test]
    fn negotiate_traced_records_the_resampling_step() {
        // T-FIX-70 × T-FIX-72：异率现场的 trace 必须把第 3 步的采样率转换
        // 写出来 —— 这正是用户回传诊断时要看的那一步（T-FIX-59 的响亮
        // 拒绝已被 T-FIX-72 的实时转换取代，本判据按新裁决改写）。
        let ranges = [fake_range(
            2,
            44_100,
            44_100,
            SupportedBufferSize::Unknown,
            SampleFormat::F32,
        )];
        let (traced, trace) = negotiate_traced(&ranges, &EngineConfig::default());
        let got = traced.expect("异率协商应成功并标记转换");
        assert!(got.resampling(), "协商结果应标记采样率转换");
        assert_eq!(got.source_rate, 48_000);
        assert_eq!(got.sample_rate, 44_100);
        assert!(
            trace
                .iter()
                .any(|line| line.contains("step 3") && line.contains("resampling")),
            "trace 缺第 3 步转换行: {trace:?}"
        );
        let last = trace.last().expect("trace 至少有结果行");
        assert!(
            last.contains("result 44100 Hz"),
            "trace 末行应是成功结果: {last}"
        );
    }

    #[test]
    fn negotiate_input_traced_names_the_input_side_empty_case() {
        // 输入侧空范围：错误是输入专用变体，trace 也必须点名输入侧。
        let (traced, trace) = negotiate_input_traced(&[], &EngineConfig::default());
        assert!(matches!(traced, Err(DeviceError::NoSupportedInputConfigs)));
        assert!(
            trace.iter().any(|line| line.contains("input")),
            "trace 应点名输入侧: {trace:?}"
        );
    }

    #[test]
    fn negotiate_exact_rate_with_block_outside_range_uses_default_buffer() {
        // 精确率命中、块长在区间外：协商成功，块长走 Default（不是失败、不是 Fixed）。
        let ranges = [fake_range(
            2,
            48_000,
            48_000,
            SupportedBufferSize::Range { min: 256, max: 512 },
            SampleFormat::F32,
        )];
        let got = negotiate(&ranges, &EngineConfig::default()).expect("精确率应成功");
        assert_eq!(got.sample_rate, 48_000);
        assert!(!got.fixed_block_accepted);
        assert_eq!(got.buffer_size, BufferSize::Default);
    }

    #[test]
    fn negotiate_rejects_non_f32_and_empty_capabilities() {
        let ranges = [fake_range(
            2,
            48_000,
            48_000,
            SupportedBufferSize::Unknown,
            SampleFormat::I16,
        )];
        match negotiate(&ranges, &EngineConfig::default()) {
            Err(DeviceError::UnsupportedSampleFormat { format }) => {
                assert_eq!(format, SampleFormat::I16);
            }
            other => panic!("预期 UnsupportedSampleFormat, 实际 {other:?}"),
        }
        match negotiate(&[], &EngineConfig::default()) {
            Err(DeviceError::NoSupportedConfigs) => {}
            other => panic!("预期 NoSupportedConfigs, 实际 {other:?}"),
        }
    }

    #[test]
    fn negotiate_accepts_wider_channel_count_when_stereo_is_unavailable() {
        let ranges = [fake_range(
            4,
            48_000,
            48_000,
            SupportedBufferSize::Range { min: 32, max: 512 },
            SampleFormat::F32,
        )];
        let got = negotiate(&ranges, &EngineConfig::default()).expect("接受设备默认通道数");
        assert_eq!(got.channels, 4);
        // 128 在 32..=512 内 → 仍然写 Fixed
        assert_eq!(got.buffer_size, BufferSize::Fixed(128));
    }

    #[test]
    fn requiring_exclusive_mode_fails_explicitly_instead_of_pretending() {
        let ranges = [fake_range(
            2,
            48_000,
            48_000,
            SupportedBufferSize::Unknown,
            SampleFormat::F32,
        )];
        let want = EngineConfig {
            share_mode: ShareMode::RequireExclusive,
            ..EngineConfig::default()
        };
        match negotiate(&ranges, &want) {
            Err(DeviceError::ExclusiveModeUnsupported) => {}
            other => panic!("预期 ExclusiveModeUnsupported, 实际 {other:?}"),
        }
        // open_output 也必须在**碰设备之前**就拒绝
        let runtime = default_runtime();
        match open_output(&want, runtime) {
            Err(DeviceError::ExclusiveModeUnsupported) => {}
            other => panic!("预期 ExclusiveModeUnsupported, 实际 {other:?}"),
        }
    }

    #[test]
    fn engine_config_projects_sample_rate_and_block_size_from_project() {
        let project = YebanProjectV1::default();
        let config = EngineConfig::from_project(&project);
        assert_eq!(config.sample_rate, 48_000);
        assert_eq!(config.block_frames, 256, "BlockSize 默认 256");
        assert_eq!(config.channels, 2);
    }

    /// 判据 (f)：**在无设备环境下允许失败、但绝不 panic**。
    ///
    /// CI 上没有声卡 ⇒ 期望 `Err(NoDefaultOutputDevice)`；开发机上可能成功 ⇒
    /// 立刻把流丢掉（**不调用 `play()`**，因此不会真的占用声卡）。
    /// 两种结果都算通过 —— 本判据只排除"panic / 挂死"。
    #[test]
    fn opening_the_default_output_device_never_panics() {
        let runtime = default_runtime();
        match open_output(&EngineConfig::default(), runtime) {
            Ok(handle) => {
                assert_eq!(handle.negotiated().sample_format, SampleFormat::F32);
                assert_eq!(handle.backend_errors(), 0);
                // 不 play(): 保持"不占用真实设备"的测试纪律
            }
            Err(error) => {
                // 无声卡是 CI 的常态; 这里只要求错误是"明确的一类"
                assert!(
                    matches!(
                        error,
                        DeviceError::NoDefaultOutputDevice
                            | DeviceError::NoSupportedConfigs
                            | DeviceError::UnsupportedConfig { .. }
                            | DeviceError::UnsupportedSampleFormat { .. }
                            | DeviceError::SampleRateMismatch { .. }
                            | DeviceError::Backend(_)
                    ),
                    "错误必须是 DeviceError 的明确变体, 实际 {error:?}"
                );
            }
        }
    }

    #[test]
    fn enumerating_output_devices_never_panics_without_a_sound_card() {
        // CI 上返回空 Vec 或 Err(Backend) 都合法; 不得 panic。
        if let Ok(devices) = enumerate_output_devices() {
            for device in devices {
                assert!(!device.name.is_empty());
            }
        }
    }

    /// 判据 (g)：`NullBackend` 用**同一份**渲染驱动覆盖真实回调路径（不需要任何设备）。
    #[test]
    fn null_backend_drives_the_same_render_path_without_a_device() {
        let runtime = default_runtime();
        let mut backend = NullBackend::with_default_config(runtime);
        assert_eq!(backend.negotiated().sample_rate, 48_000);

        let stats = backend.render(300);
        assert_eq!(backend.frames_rendered(), 300);
        assert_eq!(stats.quanta, 3, "300 帧 = 128 + 128 + 44");
        assert_eq!(stats.event_bulk_pops, 3, "每量子一次批量出队");
        assert!(backend.runtime().ftz_armed());
        assert!(backend.last_block().frames() > 0);
        assert!(backend.last_block().left().iter().all(|s| *s == 0.0));
        // Debug 实现不该 panic
        let _ = format!("{backend:?}");
    }

    #[test]
    fn zero_frame_render_is_a_noop() {
        let runtime = default_runtime();
        let mut backend = NullBackend::with_default_config(runtime);
        let stats = backend.render(0);
        assert_eq!(stats.quanta, 0);
        assert_eq!(backend.frames_rendered(), 0);
    }

    /// 判据 (h)：**真实墙钟**驱动渲染路径（不需要任何设备）。
    ///
    /// **量什么**：`NullBackend::render` 在 N 个**真的时钟节拍**上各推一个量子
    /// （节拍周期 = `DEFAULT_BLOCK_FRAMES` 帧 ÷ 48000 Hz = 2666 µs，来自
    /// `std::thread::sleep` 的墙钟），窗口内累计推进的**帧数**与墙钟**耗时**。
    /// 单位 = 帧、量子、微秒。
    ///
    /// ⚠ 本判据**不声称** cpal 的回调线程被驱动 —— 那需要一台有声卡的机器
    /// （覆盖边界登记在 `crates/yeban-engine/tests/rt_zero_alloc.rs` 的模块文档与
    /// `docs/ledger/gate-rt-zero-alloc-notes.md`）。它证明的是：这些帧是被**时钟**
    /// 推出来的（每个节拍一个量子，`sleep` 的契约是"至少睡这么久" ⇒ 耗时 ≥ N × 周期），
    /// 而且**回调体本身**（[`render_callback`]，与 cpal 闭包同一个函数）逐量子记账。
    #[test]
    fn a_real_clock_drives_the_render_path_without_any_device() {
        const QUANTA: u64 = 24;
        // 128 帧 @ 48 kHz = 2.666 ms。整数微秒，避免浮点。
        const PERIOD_US: u64 = 1_000_000 * DEFAULT_BLOCK_FRAMES as u64 / 48_000;
        let runtime = default_runtime();
        let mut backend = NullBackend::with_default_config(runtime);
        let start = std::time::Instant::now();
        for _ in 0..QUANTA {
            backend.render(DEFAULT_BLOCK_FRAMES);
            std::thread::sleep(Duration::from_micros(PERIOD_US));
        }
        let elapsed = start.elapsed();
        let stats = backend.runtime().stats();

        assert_eq!(
            stats.quanta, QUANTA,
            "N 个时钟节拍 ⇒ N 个量子（回调体逐次记账）"
        );
        assert_eq!(
            backend.frames_rendered(),
            QUANTA * DEFAULT_BLOCK_FRAMES as u64,
            "帧数 = 量子数 × 128"
        );
        assert_eq!(
            stats.position_frames,
            QUANTA * DEFAULT_BLOCK_FRAMES as u64,
            "走带位置（帧）与渲染帧数同步 —— 自由跑时二者相等"
        );
        assert_eq!(
            stats.transport_state,
            crate::transport::TransportState::Playing
        );
        assert!(
            stats.position_ticks > 0,
            "帧推进必须换算成 tick（实测 {}）",
            stats.position_ticks
        );
        // 时钟证据：`sleep` **至少**睡满给定时长 ⇒ 耗时不得短于 N × 周期。
        // 少了这一条，"忙循环推了 N 个量子"也会绿 —— 那正是本判据要排除的形态。
        assert!(
            elapsed >= Duration::from_micros(QUANTA * PERIOD_US),
            "耗时必须 ≥ {QUANTA} × {PERIOD_US} µs（实测 {elapsed:?}）"
        );
        println!(
            "[device-clock] quanta={QUANTA} frames={} ticks={} elapsed={:?} 周期={PERIOD_US}µs \
             backend_errors=0（无设备；cpal 真回调线程未执行）",
            backend.frames_rendered(),
            stats.position_ticks,
            elapsed,
        );
    }

    /// 判据 (i)：回调体**只有一个出口** —— `render_callback` 的量子记账与
    /// [`EngineRuntime::process_quantum`] 逐字相同（它是同一个函数）。
    #[test]
    fn the_callback_body_counts_one_quantum_per_call() {
        let mut runtime = default_runtime();
        let mut data = [0.0_f32; DEFAULT_BLOCK_FRAMES * 2];
        for expected in 1..=3_u64 {
            render_callback(&mut runtime, &mut data, 2);
            assert_eq!(runtime.stats().quanta, expected, "一次调用一个量子");
            assert_eq!(
                runtime.stats().event_bulk_pops,
                expected,
                "每量子恰好一次事件批量出队"
            );
        }
    }

    /// 判据：device 侧错误的 **Display 文案**是公开面，必须逐字钉住。
    ///
    /// 文案经 `EngineHostError::Device` 原样交给界面与日志 ⇒ 它是契约的一部分。
    ///
    /// **量什么**：三个变体的 `to_string()` 文本（单位：字符）。
    /// 注入实测（第四批）：`#[error("no default output device available")]` →
    /// `#[error("no device")]` ⇒ 本判据实测变红。⚠ 本判据**只在 `--features device` 档
    /// 存在**：默认档下 `device.rs` 整个模块被 `#[cfg]` 关掉（R50②）。
    #[test]
    fn device_error_messages_are_the_documented_text() {
        assert_eq!(
            DeviceError::NoDefaultOutputDevice.to_string(),
            "no default output device available"
        );
        assert_eq!(
            DeviceError::NoSupportedConfigs.to_string(),
            "device reports no supported output configuration"
        );
        assert_eq!(
            DeviceError::ExclusiveModeUnsupported.to_string(),
            "exclusive (non-shared) mode is not supported by this backend"
        );
        // R58：`==` 的判据必须另有一条 `assert_ne!` 落在**同一个**表达式上。
        assert_ne!(
            DeviceError::NoDefaultOutputDevice.to_string(),
            DeviceError::NoSupportedConfigs.to_string(),
            "两个变体的文案必须不同（否则上面的等号可能是'两边同一个常量'）"
        );
    }

    /// 判据：`NullBackend` 的**手写 `Debug` 形状**（结构体名 + 字段名）是契约。
    ///
    /// 它与 [`crate::rt::EngineStats::ftz`] 的诊断输出同源，人工排障读到的就是这份文本。
    ///
    /// **量什么**：`format!("{backend:?}")` 的文本（单位：字符）。
    /// 注入实测（第四批）：`f.debug_struct("NullBackend")` → `"NullBackendX"` ⇒
    /// 本判据实测变红（同样只在 device 档可见）。
    #[test]
    fn null_backend_debug_shape_names_the_type_and_fields() {
        let runtime = default_runtime();
        let backend = NullBackend::with_default_config(runtime);
        let text = format!("{backend:?}");
        assert!(
            text.starts_with("NullBackend {"),
            "结构体名是契约（实得 {text}）"
        );
        for field in ["negotiated", "frames_rendered", "stats"] {
            assert!(
                text.contains(&format!("{field}:")),
                "缺字段 `{field}`（实得 {text}）"
            );
        }
    }

    /// ⭐ [A④] 输入侧协商的**纯函数**判据（无声卡也能判）。
    ///
    /// 判定量（已命名）：**协商结果三元组** = `(channels, sample_rate, sample_format)`，
    /// 以及失败时的**错误变体名**。

    #[test]
    fn negotiate_input_is_pure_and_reports_input_specific_errors() {
        let want = EngineConfig::default();
        // ① 空范围 ⇒ ⭐ **输入专用**错误（⛔ 不是输出侧那条）
        let empty = negotiate_input(&[], &want).expect_err("空范围必须报错");
        println!("[device] 空范围（输入侧）⇒ {empty}");
        assert!(
            matches!(empty, DeviceError::NoSupportedInputConfigs),
            "空范围必须报 `NoSupportedInputConfigs`（实得 {empty}）"
        );
        // ② 只有 i16 ⇒ UnsupportedSampleFormat{I16}（与输出侧**同一条**规则）
        let only_i16 = [fake_range(
            2,
            48_000,
            48_000,
            SupportedBufferSize::Range { min: 64, max: 4096 },
            SampleFormat::I16,
        )];
        let err = negotiate_input(&only_i16, &want).expect_err("i16-only 必须报错");
        assert!(
            matches!(
                err,
                DeviceError::UnsupportedSampleFormat {
                    format: SampleFormat::I16
                }
            ),
            "i16-only ⇒ UnsupportedSampleFormat{{I16}}（实得 {err}）"
        );
        // ③ f32 可用 ⇒ 选中，且**如实回报**通道/采样率（判定量三元组）
        let f32_ok = [fake_range(
            2,
            48_000,
            48_000,
            SupportedBufferSize::Range { min: 64, max: 4096 },
            SampleFormat::F32,
        )];
        let got = negotiate_input(&f32_ok, &want).expect("f32 必须可协商");
        let triple = (got.channels, got.sample_rate, got.sample_format);
        println!("[device] f32 可用 ⇒ 协商三元组 = {triple:?}");
        assert_eq!(triple, (2, 48_000, SampleFormat::F32), "必须如实回报三元组");
        // ④ 非独占要求 ⇒ 与输出侧**同一条**规则（证明没有第二条政策）
        let mut exclusive = want;
        exclusive.share_mode = ShareMode::RequireExclusive;
        let err = negotiate_input(&f32_ok, &exclusive).expect_err("独占必须被拒");
        assert!(
            matches!(err, DeviceError::ExclusiveModeUnsupported),
            "独占 ⇒ ExclusiveModeUnsupported（实得 {err}）"
        );
        // ⑤ 两侧对**非空**输入的结论必须一致（对称性：⛔ 不是两套政策）
        // ⚠ `DeviceError` 不是 `PartialEq` ⇒ 比 `Ok` 侧的三元组（`Option`），⛔ 不比 Result。
        let input_ok = negotiate_input(&f32_ok, &want)
            .map(|c| (c.channels, c.sample_rate, c.sample_format))
            .ok();
        let output_ok = negotiate(&f32_ok, &want)
            .map(|c| (c.channels, c.sample_rate, c.sample_format))
            .ok();
        assert!(input_ok.is_some(), "输入侧必须协商成功");
        assert_eq!(
            input_ok, output_ok,
            "非空输入下，输入侧与输出侧的协商结论必须一致"
        );
    }

    /// ⭐ 判据（录音侧）：**注入一条已知斜坡 ⇒ 逐样本到达接收端**。
    ///
    /// ⚠ 本机**没有可用输入设备**，所以判据**不打开设备**：它直接喂
    /// [`capture_callback`] —— 那正是 [`open_input`] 建流闭包调用的**同一个**函数
    /// （与输出侧判据 ⑳ 对 [`render_callback`] 的做法同源）。
    ///
    /// ⛔ 断言是**逐样本相同**，⛔ 不是"有数据"：后者对"通道交错错位""只推了第一帧"
    /// 这类缺陷**不敏感**。
    #[test]
    fn capture_callback_delivers_an_injected_ramp_sample_by_sample() {
        const CHANNELS: u16 = 2;
        const FRAMES: usize = 8;
        let mut injected: Vec<f32> = Vec::new();
        for frame in 0..FRAMES {
            for channel in 0..usize::from(CHANNELS) {
                #[allow(clippy::cast_precision_loss)]
                injected.push((frame * 10 + channel) as f32);
            }
        }

        // ① 注入臂：斜坡必须**逐样本**到达，且 ⛔ 不允许丢弃。
        let (mut sender, mut receiver) = record_channel(64, CHANNELS);
        capture_callback(&mut sender, &injected, CHANNELS);
        let mut arrived: Vec<f32> = Vec::new();
        let drained = receiver.drain_frames(FRAMES, |frame| arrived.extend_from_slice(frame));
        assert_eq!(drained, FRAMES, "必须整帧到达（实得 {drained}）");
        assert_eq!(
            arrived, injected,
            "⭐ 必须**逐样本**相同（⛔ 不是「有数据」）"
        );
        assert_eq!(sender.dropped_frames(), 0, "环够大 ⇒ ⛔ 不允许丢弃");
    }

    /// ⭐ 判据（录音侧）**控制臂**：**不注入** ⇒ 一个样本都不该有。
    ///
    /// ⚠ **为什么单独一条**：它与斜坡臂放在同一函数里时，斜坡一旦变红就**断不到**它
    /// ⇒ "控制臂仍绿"这句就**无法被演示**。拆开之后，注入实测可以同时报出
    /// "斜坡红、控制绿"这一对读数（那才是控制臂存在的意义）。
    #[test]
    fn capture_callback_control_arm_stays_empty() {
        const CHANNELS: u16 = 2;
        const FRAMES: usize = 8;
        let (_sender, mut receiver) = record_channel(64, CHANNELS);
        let mut nothing: Vec<f32> = Vec::new();
        let drained = receiver.drain_frames(FRAMES, |frame| nothing.extend_from_slice(frame));
        assert_eq!(drained, 0, "控制臂必须空（实得 {drained}）");
        assert!(nothing.is_empty(), "控制臂不允许有样本");
    }

    /// 判据（录音侧）：**真实输入设备**在无设备环境下允许失败、但**绝不 panic**，
    /// 且失败必须是**结构化**的一类。
    ///
    /// ⚠ 与输出侧同名判据同款：CI／无声卡机器上期望
    /// [`DeviceError::NoDefaultInputDevice`]；有输入设备的机器上可能成功 ⇒
    /// 立刻把流丢掉（**不调用 `play()`**，因此不会真的占用麦克风）。
    /// ⛔ 两种结果都不许是"打不开就当录到静音"地返回成功 —— 见 [`open_input`] 文档。
    #[test]
    fn opening_the_default_input_device_never_panics() {
        let (sender, _queue) = record_channel(64, 2);
        match open_input(&EngineConfig::default(), sender) {
            Ok(handle) => {
                assert_eq!(handle.negotiated().sample_format, SampleFormat::F32);
                assert_eq!(handle.backend_errors(), 0);
                // 不 play(): 保持"不占用真实设备"的测试纪律。
                eprintln!(
                    "[录音侧] 真机输入设备：**打开成功** device_name={} negotiated={:?}",
                    handle.device_name(),
                    handle.negotiated()
                );
            }
            Err(error) => {
                // ⭐ 如实报：本机**没有**验证过真机输入（只验证了"结构化失败、不 panic"）。
                eprintln!("[录音侧] 真机输入设备：**不可用** ⇒ {error}");
                assert!(
                    matches!(
                        error,
                        DeviceError::NoDefaultInputDevice
                            | DeviceError::NoSupportedInputConfigs
                            | DeviceError::UnsupportedConfig { .. }
                            | DeviceError::UnsupportedSampleFormat { .. }
                            // T-FIX-78: 44.1 kHz-only device + 48 kHz project is a
                            // loud refusal, not a silent fallback. The exact
                            // 48_000/44_100 pair is pinned by
                            // negotiate_rejects_rate_fallback_loudly_when_requested_rate_is_unsupported.
                            | DeviceError::SampleRateMismatch { .. }
                            | DeviceError::Backend(_)
                    ),
                    "错误必须是 DeviceError 的**明确变体**，实际 {error:?}"
                );
                assert!(
                    !matches!(error, DeviceError::NoDefaultOutputDevice),
                    "⛔ 输入侧的失败**不许**报成输出侧的错误（会把调用方引错方向）"
                );
            }
        }
    }

    /// ⭐⭐ **判据（T-FIX-71，枚举）**：枚举接口在**任何**机器上都必须返回
    /// `Ok`（无声卡 ⇒ 空清单，⛔ 不是错误）；每台枚举到的设备名字非空、
    /// 默认标记至多一台（`is_default` 的口径是"与宿主默认设备 id 比对"，
    /// 两台同标默认 = 口径坏了）。
    #[test]
    fn enumerate_devices_is_ok_and_marks_at_most_one_default() {
        for (label, devices) in [
            ("output", enumerate_output_devices()),
            ("input", enumerate_input_devices()),
        ] {
            let devices = devices.unwrap_or_else(|error| {
                panic!("{label} 枚举必须返回 Ok（无声卡是空清单不是错误），实际 {error}")
            });
            assert!(
                devices.iter().filter(|device| device.is_default).count() <= 1,
                "{label} 至多一台默认设备"
            );
            for device in &devices {
                assert!(!device.name.is_empty(), "{label} 设备名不许为空");
                assert!(device.channels >= 1, "{label} 默认配置通道数必须 ≥ 1");
                assert!(device.sample_rate >= 1, "{label} 默认配置采样率必须 ≥ 1");
            }
        }
    }

    /// ⭐⭐ **判据（T-FIX-71，按名选设备的纯决策）**：精确相等才命中；
    /// 前缀/子串/大小写差异一律不命中；重名取首个；空清单恒 `None`。
    ///
    /// 反向腿：把 [`pick_named_device_index`] 改成子串匹配 ⇒ 前缀那行红；
    /// 改成取末个 ⇒ 重名那行红。
    #[test]
    fn pick_named_device_index_requires_an_exact_match() {
        let names: Vec<String> = [
            "Built-in Output",
            "USB Audio",
            "USB Audio", // 同型号第二台：重名
            "usb audio", // 只差大小写：另一台
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(pick_named_device_index(&names, "Built-in Output"), Some(0));
        assert_eq!(
            pick_named_device_index(&names, "USB Audio"),
            Some(1),
            "重名取**首个**"
        );
        assert_eq!(
            pick_named_device_index(&names, "usb audio"),
            Some(3),
            "大小写不同是**精确**命中的另一台"
        );
        assert_eq!(pick_named_device_index(&names, "USB"), None, "前缀不算命中");
        assert_eq!(
            pick_named_device_index(&names, "USB Audio "),
            None,
            "尾随空格不算命中"
        );
        assert_eq!(pick_named_device_index(&names, "不存在的设备"), None);
        assert_eq!(pick_named_device_index(&[], "USB Audio"), None);
    }

    /// ⭐⭐ **判据（T-FIX-71，按名打开输出 · 缺失响亮报错）**：指定一台
    /// **不存在**的输出设备 ⇒ 必须是 [`DeviceError::NamedOutputDeviceUnavailable`]
    /// 且错误文本**点名**。
    ///
    /// 这条判据在两种机器上都成立，且都能抓住"静默回退默认"的实现：
    /// - 有声卡的机器：回退实现会返回 `Ok`（打开了默认设备）⇒ 红；
    /// - 无声卡的机器（CI）：回退实现会返回 `NoDefaultOutputDevice` ⇒ 红。
    #[test]
    fn open_output_named_with_an_absent_device_fails_loudly_naming_it() {
        let absent = "夜半测试用不存在的输出设备 T-FIX-71";
        let error = match open_output_boxed_named(
            &EngineConfig::default(),
            Box::new(default_runtime()),
            Some(absent),
        ) {
            Ok(_) => panic!("指定不存在的输出设备必须失败（⛔ 不许静默回退默认设备）"),
            Err(error) => error,
        };
        assert!(
            matches!(error, DeviceError::NamedOutputDeviceUnavailable { ref name } if name == absent),
            "错误必须是点名的 NamedOutputDeviceUnavailable，实际 {error:?}"
        );
        assert!(
            error.to_string().contains(absent),
            "错误文本必须点名该设备，实际：{error}"
        );
    }

    /// ⭐⭐ **判据（T-FIX-71，按名打开输入 · 缺失响亮报错）**：输入侧同款
    /// （方向不能串：必须是 `NamedInputDeviceUnavailable`，⛔ 不是输出侧变体）。
    #[test]
    fn open_input_named_with_an_absent_device_fails_loudly_naming_it() {
        let absent = "夜半测试用不存在的输入设备 T-FIX-71";
        let (sender, _receiver) = record_channel(1024, 2);
        let error = match open_input_named(&EngineConfig::default(), sender, Some(absent)) {
            Ok(_) => panic!("指定不存在的输入设备必须失败（⛔ 不许静默回退默认设备）"),
            Err(error) => error,
        };
        assert!(
            matches!(error, DeviceError::NamedInputDeviceUnavailable { ref name } if name == absent),
            "错误必须是点名的 NamedInputDeviceUnavailable，实际 {error:?}"
        );
        assert!(error.to_string().contains(absent));
    }

    /// ⭐⭐ **判据（T-FIX-71，测试音回调）**：48 kHz 立体声下，前 1 秒是
    /// 1 kHz 正弦（过零数 ≈ 1000、峰值 ≈ −12 dBFS），满 1 秒后**全静音**，
    /// 且各声道同值。
    ///
    /// 反向腿：频率改 440 Hz ⇒ 过零数那行红；电平改 0 dBFS ⇒ 峰值那行红；
    /// 播完不停 ⇒ 静音那行红。
    #[test]
    fn test_tone_callback_writes_one_second_of_1khz_then_silence() {
        let mut state = TestToneState::new(48_000);
        assert_eq!(state.total_frames(), 48_000, "1 秒 @ 48 kHz = 48000 帧");
        let mut data = vec![0.0_f32; 48_000 * 2];
        test_tone_callback(&mut state, &mut data, 2);
        assert_eq!(state.frames_done(), 48_000);
        // 各声道同值（每帧左右相等）。
        for frame in data.as_chunks::<2>().0 {
            assert_eq!(frame[0], frame[1], "测试音各声道必须同相同值");
        }
        // 左声道 [0.1 s, 0.9 s) 的上行过零 ⇒ 频率 ≈ 1000 Hz（±1%）。
        let left: Vec<f32> = data.iter().step_by(2).copied().collect();
        let (from, to) = (4_800usize, 43_200usize);
        let crossings = left[from..to]
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count();
        let estimated = crossings as f64 * 48_000.0 / (to - from) as f64;
        assert!(
            (estimated - 1_000.0).abs() <= 10.0,
            "测试音频率应为 1000 Hz ±1%，实测 {estimated} Hz"
        );
        // 峰值 ≈ −12 dBFS（0.2512，±0.01）。
        let peak = left.iter().fold(0.0_f32, |acc, value| acc.max(value.abs()));
        assert!(
            (peak - 0.251_2).abs() <= 0.01,
            "测试音峰值应为 −12 dBFS ≈ 0.2512，实测 {peak}"
        );
        // 满 1 秒后：再喂一块，必须全静音。
        let mut tail = vec![1.0_f32; 4_800 * 2];
        test_tone_callback(&mut state, &mut tail, 2);
        assert!(
            tail.iter().all(|value| *value == 0.0),
            "测试音播完后必须转静音"
        );
    }

    /// ⭐⭐ **判据（T-FIX-71，协商结果的控制面描述）**：固定缓冲 ⇒ 标签带
    /// 帧数、名义延迟 = 帧数 ÷ 采样率；后端默认 ⇒ 标签是「后端默认」、
    /// 名义延迟 `None`（⛔ 不编数字）。
    #[test]
    fn negotiated_buffer_label_and_nominal_latency_are_honest() {
        let mut negotiated = NegotiatedConfig {
            channels: 2,
            sample_rate: 48_000,
            source_rate: 48_000,
            buffer_size: BufferSize::Fixed(128),
            sample_format: SampleFormat::F32,
            share_mode: ShareMode::Shared,
            fixed_block_accepted: true,
        };
        assert_eq!(negotiated.buffer_label(), "固定 128 帧");
        let ms = negotiated
            .nominal_latency_ms()
            .expect("固定缓冲必须有名义延迟");
        assert!((ms - 128.0 * 1_000.0 / 48_000.0).abs() < 1e-9, "实测 {ms}");
        negotiated.buffer_size = BufferSize::Default;
        assert_eq!(negotiated.buffer_label(), "后端默认");
        assert_eq!(negotiated.nominal_latency_ms(), None);
    }

    /// ⭐⭐ **判据（T-FIX-71，测试音选设备）**：在不存在的设备上播测试音
    /// ⇒ 点名的 [`DeviceError::NamedOutputDeviceUnavailable`]（与打开路径
    /// 同一条选设备实现，⛔ 不许测试音自己另写一份回退）。
    #[test]
    fn play_test_tone_with_an_absent_device_fails_loudly_naming_it() {
        let absent = "夜半测试用不存在的输出设备 T-FIX-71";
        let error = play_test_tone(Some(absent))
            .err()
            .expect("在不存在的设备上播测试音必须失败");
        assert!(
            matches!(error, DeviceError::NamedOutputDeviceUnavailable { ref name } if name == absent),
            "错误必须是点名的 NamedOutputDeviceUnavailable，实际 {error:?}"
        );
    }

    /// **判据（T-FIX-238）**：相邻回调播放时间戳的跳跃换算成帧。
    ///
    /// 48 kHz、期望 480 帧：连续 10 ms ⇒ 0；晚到 30 ms ⇒ 960 帧；
    /// 首回调/时钟回退/零期望 ⇒ 0（不报假 xrun）。
    #[test]
    fn callback_gap_frames_measures_only_real_timestamp_jumps() {
        let base = cpal::StreamInstant::new(10, 0);
        let on_time = base
            .checked_add(Duration::from_millis(10))
            .expect("10 ms 可表示");
        let late = base
            .checked_add(Duration::from_millis(30))
            .expect("30 ms 可表示");
        assert_eq!(
            callback_gap_frames(None, on_time, 480, 48_000),
            0,
            "首个回调没有参照 ⇒ 不报 xrun"
        );
        assert_eq!(
            callback_gap_frames(Some(base), on_time, 480, 48_000),
            0,
            "连续回调（10 ms = 480 帧）⇒ 0"
        );
        assert_eq!(
            callback_gap_frames(Some(base), late, 480, 48_000),
            960,
            "30 ms = 1440 帧，减去期望 480 ⇒ 跳跃 960 帧"
        );
        assert_eq!(
            callback_gap_frames(Some(on_time), base, 480, 48_000),
            0,
            "时间戳回退不是掉帧，不计数"
        );
        assert_eq!(
            callback_gap_frames(Some(base), late, 0, 48_000),
            0,
            "期望帧数为 0 ⇒ 读数无效，不编数字"
        );
    }

    /// **判据（T-FIX-238）**：诊断行在无 xrun / 有 xrun 两种状态下都说人话，
    /// 且毫秒换算只在采样率有效时出现。
    #[test]
    fn callback_health_line_reports_xruns_with_a_millisecond_reading() {
        let clean = EngineStats::default();
        assert_eq!(
            audio_callback_health_line(&clean, 48_000),
            "音频回调：连续（xrun 0）"
        );
        let lagging = EngineStats {
            xrun_count: 3,
            max_gap_frames: 240,
            ..EngineStats::default()
        };
        assert_eq!(
            audio_callback_health_line(&lagging, 48_000),
            "音频回调：xrun 3 次 · 最大间隔 240 帧（5.0 ms）"
        );
        assert_eq!(
            audio_callback_health_line(&lagging, 0),
            "音频回调：xrun 3 次 · 最大间隔 240 帧"
        );
    }
}
