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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, SampleFormat, StreamConfig, SupportedBufferSize, SupportedStreamConfigRange,
};
use thiserror::Error;
use yeban_model::YebanProjectV1;

use crate::block::DEFAULT_BLOCK_FRAMES;
use crate::rt::{EngineRuntime, EngineStats};

/// 打开流的超时（cpal 0.18 的 `timeout` 参数）。避免设备被独占时无限等待。
pub const STREAM_OPEN_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// 实际采样率 (Hz)。
    pub sample_rate: u32,
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

/// 设备/配置相关错误。**全部是返回值，绝不 panic**。
#[derive(Debug, Error)]
pub enum DeviceError {
    /// 系统上没有默认输出设备（CI 容器、无声卡服务器、设备被拔掉）。
    #[error("no default output device available")]
    NoDefaultOutputDevice,

    /// 设备没有报告任何输出配置。
    #[error("device reports no supported output configuration")]
    NoSupportedConfigs,

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

    /// 请求了独占模式，但 cpal 0.18 没有该能力（见模块文档）。
    #[error("exclusive (non-shared) mode is not supported by this backend")]
    ExclusiveModeUnsupported,

    /// 后端返回的错误（设备忙、权限不足、配置失效……）。
    #[error("audio backend error: {0}")]
    Backend(#[from] cpal::Error),
}

/// 输出设备的可枚举信息（**非实时路径**：允许分配字符串）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// 设备显示名（`cpal::Device` 的 `Display`）。
    pub name: String,
    /// 设备稳定标识（`DeviceTrait::id`；失败时为空）。
    pub id: String,
    /// 默认输出配置的通道数。
    pub channels: u16,
    /// 默认输出配置的采样率。
    pub sample_rate: u32,
    /// 默认输出配置的采样格式。
    pub sample_format: SampleFormat,
}

/// 枚举全部输出设备。无声卡时返回空 `Vec`（不是错误）。
///
/// # Errors
///
/// 后端拒绝枚举（例如音频守护进程不可用）→ [`DeviceError::Backend`]。
pub fn enumerate_output_devices() -> Result<Vec<DeviceInfo>, DeviceError> {
    let host = cpal::default_host();
    let mut out = Vec::new();
    for device in host.output_devices()? {
        let config = match device.default_output_config() {
            Ok(config) => config,
            // 单个设备读不到默认配置不应让整个枚举失败。
            Err(_) => continue,
        };
        out.push(DeviceInfo {
            name: device.to_string(),
            id: device.id().map(|id| id.to_string()).unwrap_or_default(),
            channels: config.channels(),
            sample_rate: config.sample_rate(),
            sample_format: config.sample_format(),
        });
    }
    Ok(out)
}

/// 设备能力与请求的协商（**纯函数**，不触碰真实设备 ⇒ CI 上可单测）。
///
/// 选择规则（按优先级）：
///
/// 1. 优先 `f32` + 请求的采样率 + 请求的通道数（立体声不足时接受设备的默认通道数）；
/// 2. 采样率不被支持时退回设备的**首选标准采样率**（48k → 44.1k）；
/// 3. `f32` 完全不可用 → [`DeviceError::UnsupportedSampleFormat`]（不静默做格式转换，
///    因为那会引入一条未经确定性验证的样本路径 [ARCH-DET-001]）。
///
/// # Errors
///
/// 见上；另有 [`DeviceError::UnsupportedConfig`] / [`DeviceError::NoSupportedConfigs`]。
pub fn negotiate(
    supported: &[SupportedStreamConfigRange],
    want: &EngineConfig,
) -> Result<NegotiatedConfig, DeviceError> {
    if want.share_mode == ShareMode::RequireExclusive {
        return Err(DeviceError::ExclusiveModeUnsupported);
    }
    if supported.is_empty() {
        return Err(DeviceError::NoSupportedConfigs);
    }

    let offered = describe(supported);
    let f32_ranges: Vec<&SupportedStreamConfigRange> = supported
        .iter()
        .filter(|range| range.sample_format() == SampleFormat::F32)
        .collect();
    if f32_ranges.is_empty() {
        return Err(DeviceError::UnsupportedSampleFormat {
            format: supported[0].sample_format(),
        });
    }

    // 1) 精确匹配请求的采样率 + 通道数
    for range in &f32_ranges {
        if range.channels() == want.channels && range.contains_rate(want.sample_rate) {
            return Ok(finish(range, want, want.sample_rate, false));
        }
    }
    // 2) 采样率匹配但通道数不同（接受设备默认通道数）
    for range in &f32_ranges {
        if range.contains_rate(want.sample_rate) {
            return Ok(finish(range, want, want.sample_rate, false));
        }
    }
    // 3) 退回该范围的首选标准采样率
    for range in &f32_ranges {
        if let Some(config) = range.try_with_standard_sample_rate() {
            return Ok(finish(range, want, config.sample_rate(), true));
        }
    }
    Err(DeviceError::UnsupportedConfig {
        want_channels: want.channels,
        want_rate: want.sample_rate,
        offered,
    })
}

fn finish(
    range: &SupportedStreamConfigRange,
    want: &EngineConfig,
    sample_rate: u32,
    rate_fallback: bool,
) -> NegotiatedConfig {
    // 请求的块长只有在设备报告的缓冲区间内才写进 Fixed；否则用 Default，
    // 由后端决定（引擎内部渲染量子仍是 DEFAULT_BLOCK_FRAMES）。
    let fixed_block_accepted = match range.buffer_size() {
        SupportedBufferSize::Range { min, max } => {
            !rate_fallback && (*min..=*max).contains(&want.block_frames)
        }
        SupportedBufferSize::Unknown => false,
    };
    NegotiatedConfig {
        channels: range.channels(),
        sample_rate,
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
    backend_errors: Arc<AtomicU64>,
}

impl OutputStreamHandle {
    /// 协商到的配置。
    #[must_use]
    pub const fn negotiated(&self) -> &NegotiatedConfig {
        &self.negotiated
    }

    /// 这台流的设备显示名（`enumerate_output_devices` 的 `DeviceInfo::name` 同源）。
    #[must_use]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// 后端错误计数（错误回调只做自增，**从不打印** [红线 7]）。
    #[must_use]
    pub fn backend_errors(&self) -> u64 {
        self.backend_errors.load(Ordering::Relaxed)
    }

    /// 启动流（cpal 建的流默认是停止的，必须显式 `play`）。
    ///
    /// # Errors
    ///
    /// 设备消失 / 流失效 → [`DeviceError::Backend`]。
    pub fn play(&self) -> Result<(), DeviceError> {
        self.stream.play()?;
        Ok(())
    }

    /// 暂停流。
    ///
    /// # Errors
    ///
    /// 后端不支持暂停 → [`DeviceError::Backend`]。
    pub fn pause(&self) -> Result<(), DeviceError> {
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

/// 打开默认输出设备并把 [`EngineRuntime`] 挂到回调上。
///
/// 真实设备相关的一切（枚举能力、协商、建流）都在这里，并且**只返回 `Result`**。
/// 无声卡环境下返回 [`DeviceError::NoDefaultOutputDevice`]，绝不 panic。
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
    mut runtime: EngineRuntime,
) -> Result<OutputStreamHandle, DeviceError> {
    if want.share_mode == ShareMode::RequireExclusive {
        return Err(DeviceError::ExclusiveModeUnsupported);
    }
    let host = cpal::default_host();
    let device: Device = host
        .default_output_device()
        .ok_or(DeviceError::NoDefaultOutputDevice)?;
    let device_name = device.to_string();
    let supported: Vec<SupportedStreamConfigRange> = device.supported_output_configs()?.collect();
    let negotiated = negotiate(&supported, want)?;

    let stream_config = StreamConfig {
        channels: negotiated.channels,
        sample_rate: negotiated.sample_rate,
        buffer_size: negotiated.buffer_size,
    };
    let channels = negotiated.channels;
    let backend_errors = Arc::new(AtomicU64::new(0));
    let error_counter = Arc::clone(&backend_errors);

    // 数据回调: 只有 `render_callback`（= 只有 `process_quantum`）——
    // 零分配 / 零锁 / 零 I/O / 零日志 [红线 7]。回调体是**具名函数**，见它的文档。
    let stream = device.build_output_stream::<f32, _, _>(
        stream_config,
        move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
            render_callback(&mut runtime, data, channels);
        },
        // 错误回调: 只自增原子计数器。打印/日志都在这里被禁止 [红线 7]。
        move |_error: cpal::Error| {
            error_counter.fetch_add(1, Ordering::Relaxed);
        },
        Some(STREAM_OPEN_TIMEOUT),
    )?;

    Ok(OutputStreamHandle {
        stream,
        device_name,
        negotiated,
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
    fn negotiate_falls_back_to_standard_rate_when_requested_rate_is_unsupported() {
        // 设备只支持 44.1k
        let ranges = [fake_range(
            2,
            44_100,
            44_100,
            SupportedBufferSize::Unknown,
            SampleFormat::F32,
        )];
        let got = negotiate(&ranges, &EngineConfig::default()).expect("应退回 44.1k");
        assert_eq!(got.sample_rate, 44_100);
        assert!(!got.fixed_block_accepted);
        assert_eq!(
            got.buffer_size,
            BufferSize::Default,
            "设备不报缓冲区间时不得写 Fixed"
        );
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
}
