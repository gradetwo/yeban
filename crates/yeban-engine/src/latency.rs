//! `BASELINE-005`（音频硬件往返时延，目标 `≤ 5.5 ms`）的**机器**那一半：
//! 标称时延换算、**主机报告的驱动侧时延**、回调调度抖动统计、机器可读行、
//! 以及**把"测不到"与"达标"分开**的判定。
//!
//! 本模块**零依赖**（不引用 cpal / rtrb / yeban-model，也不引用本 crate 其它模块）
//! ⇒ 可以用 `rustc --edition 2024 --test -D warnings crates/yeban-engine/src/latency.rs`
//! 在**没有声卡**的机器上真跑（本机 M2 就是这么验的，见 `docs/ledger/audio-latency-notes.md`）。
//!
//! # 为什么先写"判定"再写"测量"
//!
//! `BASELINE-005` 是全仓库**唯一真正需要音频硬件**的门禁。它此前连测量工具都没有，
//! 于是长期 `PENDING`。本线做机器，但**第一步不是把数字打出来，而是把边界钉死**：
//!
//! | 量 | 本模块能算吗 | 它是什么 |
//! | :--- | :---: | :--- |
//! | 标称时延 `nominal_ms` | ✅ | 设备报告的**缓冲帧数 ÷ 采样率**。**不是**往返时延 |
//! | 主机报的驱动侧时延 `distribution_ms` | ✅ | 输出：`playback − callback`；输入：`callback − capture`。**主机自己算的预测值** |
//! | 回调调度抖动 `jitter_stats` | ✅ | **CPU 侧**回调到达间隔与标称周期的偏差。**不是**往返时延 |
//! | 声学 / DAC-ADC 往返 | 默认腿 ❌ / `--loopback` 腿 ✅ | 默认腿要**物理回环**（输出接输入）；`--loopback` 腿用**线性 chirp + 归一化互相关**（本模块的 [`detect_delay_frames`]）做采集比对，给的是**主机观测的 callback→callback 往返**（含驱动缓冲、不含 DAC/ADC 模拟段） |
//!
//! ## ⚠ 关于 cpal 的准确说法（本线第一版说错过，这里更正并留痕）
//!
//! `cpal 0.18.2` **没有**显式的时延查询 API（`DeviceTrait` 只有 `buffer_size()` / `now()`），
//! 但它**并非不携带硬件时延信息**：每个**输出**回调收到的
//! `OutputStreamTimestamp::playback` 是"现在写入的数据**预计**被播放的时刻"，
//! 每个**输入**回调收到的 `InputStreamTimestamp::capture` 是"这批数据被采到的时刻"。
//! 两者都是**主机自己**用厂商 API 算出来的：
//!
//! | 主机 | `playback` 的算法（cpal 0.18.2 源码） | 因此在算谁的账 |
//! | :--- | :--- | :--- |
//! | CoreAudio (macOS) | `device_buffer_frames + kAudioDevicePropertyLatency + kAudioDevicePropertySafetyOffset` | 设备缓冲 + 设备时延 + 安全偏移 |
//! | WASAPI (Windows) | `buffered_frames + stream.stream_latency` | 已提交未消费帧 + `IAudioClient::GetStreamLatency` |
//! | ALSA (Linux) | `delay_frames`（PCM 状态里的 delay） | `snd_pcm_delay` 一族 |
//! | PulseAudio | `elapsed + 估算的流时延` | 流时延估计（**是估计值**） |
//! | PipeWire / JACK / ASIO / AAudio | 各自的流时间 + 缓冲 | 厂商/服务器报的值 |
//!
//! ⇒ 所以 `playback − callback`（输出）与 `callback − capture`（输入）是**可测的**，
//! 而且**不需要任何新依赖**。但它**仍然不是**"实测往返"：
//! ① 它是**主机/驱动的预测**，不是测出来的声学量；
//! ② 方向分开、**故意不给合计**（合计会变成"看起来达标的总数"）；
//! ③ 有些主机压根不报（`playback == callback`）⇒ 此时必须报 `unreported-or-zero`，
//!    **不许**当成 0 ms 时延。
//!
//! ⇒ 因此 [`verdict_for`] 有一条**不可绕过**的规则：
//! **没有回环证据时，`verdict` 永远不可能是 [`Verdict::WithinTarget`]**，
//! 无论标称值、驱动侧值或抖动多好看。这就是"宁可少宣称"的机械形式。
//!
//! 本模块的判定与解析都被 `.github` 之外可跑的判据盯着（见文件末尾 `mod tests` 与
//! `crates/yeban-engine/tests/latency_cli_contract.rs`）。

/// `BASELINE-005` 的门禁编号（写进 `BENCH baseline=005` 行）。
pub const BASELINE_ID: &str = "BASELINE-005";

/// 机器可读行的 `baseline=` 字段值（与 [`BASELINE_ID`] 的数字部分同源）。
pub const BASELINE_NUMBER: &str = "005";

/// 规范目标：音频硬件往返时延 `≤ 5.5 ms`
/// （`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.2、ARCH §3.5 分解表）。
pub const TARGET_ROUNDTRIP_MS: f64 = 5.5;

/// 流方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// 采集（ADC → 内存）。
    Input,
    /// 回放（内存 → DAC）。
    Output,
}

impl Direction {
    /// 机器可读字段里的写法。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }
}

/// 读数的**证据等级**。它决定 [`verdict_for`] 能给出什么结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// 只有设备报告的标称量（缓冲帧数、采样率）与 CPU 侧回调抖动。
    ///
    /// **这不构成往返时延的证据**：标称值不含驱动/硬件额外延迟，也不含回环本身的时延。
    NominalOnly,
    /// 有回环证据（物理输出→输入回环，或厂商 API 给出的硬件时延读数）。
    ///
    /// 只有这一等级才允许出现 [`Verdict::WithinTarget`]。
    LoopbackMeasured,
}

impl Evidence {
    /// 机器可读字段里的写法。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NominalOnly => "nominal-only",
            Self::LoopbackMeasured => "loopback-measured",
        }
    }
}

/// `BASELINE-005` 的判定结果。
///
/// ⚠ [`Verdict::WithinTarget`] / [`Verdict::OverTarget`] **只**能由
/// [`Evidence::LoopbackMeasured`] + 一个有限实测数触发（见 [`verdict_for`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// **真·实测达标**（有回环证据且 ≤ [`TARGET_ROUNDTRIP_MS`]）。
    WithinTarget,
    /// **真·实测超目标**（有回环证据且 > [`TARGET_ROUNDTRIP_MS`]）。
    OverTarget,
    /// 有设备、但**没有回环证据** ⇒ 往返时延无法判定。**这不是达标，也不是不达标。**
    UnmeasurableWithoutLoopback,
    /// 本环境**没有音频设备**（托管 CI runner 的常态）⇒ 什么都没测到。**这不是达标。**
    NoDevice,
}

impl Verdict {
    /// 机器可读字段里的写法。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WithinTarget => "within-target",
            Self::OverTarget => "over-target",
            Self::UnmeasurableWithoutLoopback => "unmeasurable-without-loopback",
            Self::NoDevice => "no-device",
        }
    }

    /// 这个判定是否等于"已经证明达标"。
    #[must_use]
    pub const fn is_pass(self) -> bool {
        matches!(self, Self::WithinTarget)
    }
}

/// 退出码：**实测**达标。
pub const EXIT_WITHIN_TARGET: u8 = 0;
/// 退出码：**实测**超目标。
pub const EXIT_OVER_TARGET: u8 = 1;
/// 退出码：命令行用法错误。
pub const EXIT_USAGE: u8 = 2;
/// 退出码：本环境没有音频设备（**"没测到"，不是"达标"**）。
pub const EXIT_NO_DEVICE: u8 = 3;
/// 退出码：有设备但没有回环证据 ⇒ 往返时延不可判定（**不是"达标"**）。
pub const EXIT_UNMEASURABLE: u8 = 4;
/// 退出码：工具**未启用**（编译时关掉了 `device` feature ⇒ 本机轻量变体）。
pub const EXIT_TOOL_DISABLED: u8 = 5;

/// "无设备"的精确文案。判据对着它断言（**不许**被改写成"0 ms"或"通过"）。
pub const NO_DEVICE_REASON: &str = "no audio device reported by this host: nothing was measured, \
this is NOT a pass and NOT 0 ms";

/// "有设备但无回环"的精确文案。
///
/// ⚠ 后半句明确"`--loopback` 腿**能**补上这个证据"：本常量是**这次调用没有回环证据**的
/// 理由，不是"本工具永远测不到往返"。判据对着 `cpal 0.18.2` 这个字面断言。
pub const NO_LOOPBACK_REASON: &str = "no loopback evidence in this invocation: cpal 0.18.2 has no \
explicit latency query API, and although each callback carries a host-computed playback/capture \
instant that we DO report, that is a driver-side prediction, not an acoustic DAC-ADC roundtrip. \
The `--loopback` leg CAN supply real evidence (a linear chirp matched-filtered over the input \
capture); without `--loopback` - or when it does not detect the chirp - the roundtrip stays \
unmeasured";

/// 输出侧驱动时延的推导口径（进机器可读行，让读数字的人知道这个数**是从哪来的**）。
pub const DRIVER_OUT_LATENCY_SOURCE: &str = "playback_minus_callback";

/// 输入侧驱动时延的推导口径。
pub const DRIVER_IN_LATENCY_SOURCE: &str = "callback_minus_capture";

/// 主机**不报**驱动时延（`playback == callback`）时的字段值。
///
/// ⚠ 这个字面量**故意**不是一个数字：`0` 与"没报"是两件事，混淆它们就是制造假绿。
pub const DRIVER_LATENCY_UNREPORTED: &str = "unreported-or-zero";

/// 标称单方向时延（毫秒）= `buffer_frames / sample_rate_hz * 1000`。
///
/// **这不是往返时延**：它只含设备报告的缓冲，不含驱动/硬件额外流水线延迟，
/// 也不含回环路径本身。返回 `None` 当采样率为 0（**绝不**发明一个数）。
#[must_use]
pub fn nominal_ms(buffer_frames: u32, sample_rate_hz: u32) -> Option<f64> {
    if sample_rate_hz == 0 {
        return None;
    }
    Some(f64::from(buffer_frames) * 1000.0 / f64::from(sample_rate_hz))
}

/// **最近秩（nearest-rank）**百分位：对**已升序**的切片取第 `ceil(p/100·n)` 个元素。
///
/// 口径钉死（判据对着它断言）：
/// `rank = ceil(p/100 · n)`，下标 `= max(rank, 1) - 1`，再钳到 `n - 1`。
/// `p` 被钳到 `0..=100`；空切片或非有限 `p` 返回 `None`。
///
/// 选最近秩而不是插值，是因为它**不发明数据点**：回调抖动要报的是"真的发生过这么差的间隔"。
#[must_use]
pub fn percentile_nearest_rank(sorted_ms: &[f64], percentile: f64) -> Option<f64> {
    if sorted_ms.is_empty() || !percentile.is_finite() {
        return None;
    }
    let clamped = percentile.clamp(0.0, 100.0);
    let n = sorted_ms.len();
    // rank ∈ [1, n]（ceil 后可略大于 n 只在浮点极端下发生）⇒ 转 usize 安全；
    // 末尾仍用 saturating + min 防御式收口。
    let rank = (((clamped / 100.0) * n as f64).ceil()).max(1.0) as usize;
    Some(sorted_ms[rank.saturating_sub(1).min(n - 1)])
}

/// 回调调度抖动统计（**CPU 侧**，全部单位毫秒）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JitterStats {
    /// 参与统计的回调间隔个数。
    pub samples: usize,
    /// 标称回调周期 = `buffer_frames / sample_rate`。
    pub nominal_period_ms: f64,
    /// 偏差（`|实际间隔 − 标称周期|`）的中位数。
    pub p50_ms: f64,
    /// 偏差的 p99。
    pub p99_ms: f64,
    /// 偏差的最大值。
    pub max_ms: f64,
    /// 偏差的算术平均 —— **单独一个字段**，就是为了让"用均值冒充 p99"这件事
    /// 在判据里可被抓住（右偏分布下 `p99 > mean`，见模块测试）。
    pub mean_ms: f64,
}

/// 由一串回调到达间隔（纳秒）算抖动统计。
///
/// 偏差定义：`|interval_ns − nominal_period_ns|`。返回 `None` 当 `intervals_ns` 为空
/// （**"没采到"和"抖动为 0"是两件事**，不许混）。
#[must_use]
pub fn jitter_stats(intervals_ns: &[u64], nominal_period_ns: u64) -> Option<JitterStats> {
    if intervals_ns.is_empty() {
        return None;
    }
    let nominal = nominal_period_ns as f64;
    let mut deviations: Vec<f64> = Vec::with_capacity(intervals_ns.len());
    let mut sum = 0.0_f64;
    for &interval in intervals_ns {
        let deviation = ((interval as f64) - nominal).abs();
        sum += deviation;
        deviations.push(deviation);
    }
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = deviations.len();
    let p50 = percentile_nearest_rank(&deviations, 50.0)?;
    let p99 = percentile_nearest_rank(&deviations, 99.0)?;
    let max = *deviations.last()?;
    Some(JitterStats {
        samples: n,
        nominal_period_ms: nominal / 1_000_000.0,
        p50_ms: p50 / 1_000_000.0,
        p99_ms: p99 / 1_000_000.0,
        max_ms: max / 1_000_000.0,
        mean_ms: sum / n as f64 / 1_000_000.0,
    })
}

/// 一串毫秒读数的分布（**不是**相对某个周期的偏差，与 [`JitterStats`] 区分开）。
///
/// 用途：主机报告的驱动侧时延（输出 `playback − callback`、输入 `callback − capture`）
/// 与"标称周期"无关，因此不能套 [`JitterStats`] 的口径（它减掉了一个标称周期）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MsDistribution {
    /// 参与统计的样本个数。
    pub samples: usize,
    /// 中位数。
    pub p50_ms: f64,
    /// p99。
    pub p99_ms: f64,
    /// 最大值。
    pub max_ms: f64,
    /// 算术平均。
    pub mean_ms: f64,
}

/// 由一串**纳秒读数**算毫秒分布（最近秩百分位，口径与 [`percentile_nearest_rank`] 同源）。
///
/// 返回 `None` 当 `values_ns` 为空（**"没采到"和"全是 0"是两件事**）。
#[must_use]
pub fn distribution_ms(values_ns: &[u64]) -> Option<MsDistribution> {
    if values_ns.is_empty() {
        return None;
    }
    let mut values: Vec<f64> = values_ns
        .iter()
        .map(|value| *value as f64 / 1_000_000.0)
        .collect();
    let sum: f64 = values.iter().sum();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = values.len();
    Some(MsDistribution {
        samples: n,
        p50_ms: percentile_nearest_rank(&values, 50.0)?,
        p99_ms: percentile_nearest_rank(&values, 99.0)?,
        max_ms: *values.last()?,
        mean_ms: sum / n as f64,
    })
}

/// 判断一个驱动侧时延分布是不是"主机压根没报"（所有样本都恰好是 0）。
///
/// ⚠ 这是**无法区分**"真的是 0"与"主机不报"时唯一诚实的表达方式 ⇒ 见
/// [`DRIVER_LATENCY_UNREPORTED`]。
#[must_use]
pub fn driver_latency_unreported(distribution: &MsDistribution) -> bool {
    distribution.max_ms == 0.0
}

/// 把设备数 / 证据等级 / 实测往返数变成判定。
///
/// # 规则（判据钉死的就是这四条）
///
/// 1. `device_count == 0` ⇒ [`Verdict::NoDevice`]（**任何**其它输入都不能翻盘）；
/// 2. [`Evidence::LoopbackMeasured`] + 有限实测数 ⇒ 达标 / 超目标；
/// 3. 其余一切（`NominalOnly`、或"声称有回环但没有数字"、或非有限数字）
///    ⇒ [`Verdict::UnmeasurableWithoutLoopback`]。
///    **标称值再小也永远走不到 [`Verdict::WithinTarget`]**。
/// 4. 没有 `device_count == 0` 也没有回环 ⇒ [`Verdict::UnmeasurableWithoutLoopback`]。
#[must_use]
pub fn verdict_for(
    device_count: usize,
    evidence: Evidence,
    measured_roundtrip_ms: Option<f64>,
) -> Verdict {
    if device_count == 0 {
        return Verdict::NoDevice;
    }
    match (evidence, measured_roundtrip_ms) {
        (Evidence::LoopbackMeasured, Some(ms)) if ms.is_finite() => {
            if ms <= TARGET_ROUNDTRIP_MS {
                Verdict::WithinTarget
            } else {
                Verdict::OverTarget
            }
        }
        _ => Verdict::UnmeasurableWithoutLoopback,
    }
}

/// 判定 → 退出码。
///
/// `allow_unmeasurable` **只**把"没测到"这一类（[`Verdict::NoDevice`] /
/// [`Verdict::UnmeasurableWithoutLoopback`]）映射成 0，好让 CI 的日志步骤不必为了
/// "本环境没有声卡"变红；它**不改变判定本身**（`BENCH` 行仍是 `no-device` /
/// `unmeasurable-without-loopback`），也**永远不会**制造 [`Verdict::WithinTarget`]。
#[must_use]
pub const fn exit_code_for(verdict: Verdict, allow_unmeasurable: bool) -> u8 {
    match verdict {
        Verdict::WithinTarget => EXIT_WITHIN_TARGET,
        Verdict::OverTarget => EXIT_OVER_TARGET,
        Verdict::NoDevice => {
            if allow_unmeasurable {
                EXIT_WITHIN_TARGET
            } else {
                EXIT_NO_DEVICE
            }
        }
        Verdict::UnmeasurableWithoutLoopback => {
            if allow_unmeasurable {
                EXIT_WITHIN_TARGET
            } else {
                EXIT_UNMEASURABLE
            }
        }
    }
}

/// 枚举到的一个端点（输入或输出）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceReport {
    /// 枚举序号（仅用于让人对上号）。
    pub index: usize,
    /// 这个端点是输入还是输出。
    pub direction: Direction,
    /// 设备显示名。
    pub name: String,
    /// 驱动名（cpal 的 `DeviceDescription::driver`；后端不提供时 `None`）。
    pub driver: Option<String>,
    /// 连接/接口类型（`USB` / `Built-in` / …；后端不提供时 `None`）。
    pub interface: Option<String>,
    /// 默认配置的通道数。
    pub default_channels: Option<u16>,
    /// 默认配置的采样率。
    pub default_sample_rate: Option<u32>,
    /// 默认配置的采样格式名。
    pub default_sample_format: Option<String>,
    /// 设备报告的**最小**缓冲帧数（`SupportedBufferSize::Range` 时）。
    pub buffer_min_frames: Option<u32>,
    /// 设备报告的**最大**缓冲帧数。
    pub buffer_max_frames: Option<u32>,
    /// 后端**不报**缓冲区间（`SupportedBufferSize::Unknown`）。
    pub buffer_range_unknown: bool,
    /// 打开流之后后端报告的**实际**缓冲帧数（`DeviceTrait::buffer_size()`）。
    pub buffer_frames_reported: Option<u32>,
}

/// 一个方向的标称时延读数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NominalReport {
    /// 方向。
    pub direction: Direction,
    /// 用到的缓冲帧数。
    pub buffer_frames: u32,
    /// 采样率。
    pub sample_rate_hz: u32,
    /// 标称时延（毫秒）。
    pub latency_ms: f64,
}

/// 一个方向真正跑过流之后的回调统计。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CallbackReport {
    /// 方向。
    pub direction: Direction,
    /// 回调总次数（含第一次，第一次没有间隔可比）。
    pub callbacks: usize,
    /// 后端错误回调次数（只自增，不打印 [红线 7]）。
    pub backend_errors: u64,
    /// 抖动统计；一次间隔都没采到（或只采到一次回调）时为 `None`。
    pub stats: Option<JitterStats>,
    /// **主机报告的驱动侧时延**分布：
    /// 输出方向 = `playback − callback`，输入方向 = `callback − capture`。
    ///
    /// `None` = 一次都没采到；`Some` 且 [`driver_latency_unreported`] 为真 =
    /// 主机把 `playback` 与 `callback` 报成同一时刻（**"不报"，不是"0 ms"**）。
    pub driver_latency: Option<MsDistribution>,
}

/// 一次完整运行的报告：**有设备/没设备、有回环/没回环都走这一个类型**。
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// 读数标签（进 `BENCH label=`）。
    pub label: String,
    /// 请求的缓冲帧数（规范口径是 64）。
    pub requested_frames: u32,
    /// 目标毫秒数（[`TARGET_ROUNDTRIP_MS`]）。
    pub target_ms: f64,
    /// 枚举到的端点。
    pub devices: Vec<DeviceReport>,
    /// 标称时延读数（帧数未知时不产出条目 ⇒ 也就不会在行里出现数字）。
    pub nominals: Vec<NominalReport>,
    /// 回调统计。
    pub callbacks: Vec<CallbackReport>,
    /// 证据等级。
    pub evidence: Evidence,
    /// **实测**往返时延：**默认腿**恒为 `None`（且**不许**被标称值填上）；
    /// `--loopback` 腿真的检出 chirp 时才是一个数（见 `measure_latency --loopback`）。
    pub measured_roundtrip_ms: Option<f64>,
    /// 额外的人类可读备注（**不**进机器可读行）。
    pub notes: Vec<String>,
}

impl Report {
    /// 枚举到的端点数。0 ⇒ [`Verdict::NoDevice`]。
    #[must_use]
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// 判定（**唯一**的判定入口）。
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        verdict_for(
            self.device_count(),
            self.evidence,
            self.measured_roundtrip_ms,
        )
    }

    /// `measured_roundtrip_ms=` 字段的值：数值或字面 `none`。
    #[must_use]
    pub fn measured_roundtrip_field(&self) -> String {
        match self.measured_roundtrip_ms {
            Some(ms) if ms.is_finite() => format!("{ms:.4}"),
            _ => "none".to_owned(),
        }
    }

    /// 某个方向的标称时延（毫秒）。
    #[must_use]
    pub fn nominal_of(&self, direction: Direction) -> Option<f64> {
        self.nominals
            .iter()
            .find(|item| item.direction == direction)
            .map(|item| item.latency_ms)
    }

    /// 某个方向的回调统计。
    #[must_use]
    pub fn callback_of(&self, direction: Direction) -> Option<&CallbackReport> {
        self.callbacks
            .iter()
            .find(|item| item.direction == direction)
    }

    fn opt_ms(value: Option<f64>) -> String {
        match value {
            Some(ms) if ms.is_finite() => format!("{ms:.4}"),
            _ => "none".to_owned(),
        }
    }

    /// **机器可读行**（单行、键序固定、值里不含空白 ⇒ 可被 [`parse_bench_line`] 严格解析）。
    ///
    /// `label` 里的空白与 `=` 会被替换成 `-`（见 [`sanitize_label`]），
    /// 因此**任何**标签都不会破坏行长格式。
    ///
    /// ⚠ 注意本行**没有** `roundtrip_ms` 这种"看起来达标的总数"字段：
    /// 实测往返只出现在 `measured_roundtrip_ms=` —— 在**默认腿**（没加 `--loopback`）
    /// 上它恒为 `none`，只有 `--loopback` 腿**真的检出** chirp 之后才会是一个数
    /// （那时 `evidence=loopback-measured`、`loopback=true`）。
    /// `nominal_io_sum_ms` 是**标称输入 + 标称输出**的和，随行的
    /// `nominal_io_sum_is_roundtrip=false` 明确声明它**不是**往返时延；
    /// `driver_out_latency_*` / `driver_in_latency_*` 是**主机报告的驱动侧时延**，
    /// **故意不给合计** —— 一个"输入+输出+驱动"的总数正是最容易被误读成达标的形状。
    #[must_use]
    pub fn bench_line(&self) -> String {
        let input = self.nominal_of(Direction::Input);
        let output = self.nominal_of(Direction::Output);
        let sum = match (input, output) {
            (Some(a), Some(b)) => Self::opt_ms(Some(a + b)),
            _ => "none".to_owned(),
        };
        let jitter = |direction: Direction, pick: fn(&JitterStats) -> f64| -> String {
            match self.callback_of(direction).and_then(|item| item.stats) {
                Some(stats) => format!("{:.4}", pick(&stats)),
                None => "none".to_owned(),
            }
        };
        // 驱动侧时延字段：none（没采到）/ unreported-or-zero（主机不报）/ 数值。
        let driver = |direction: Direction, pick: fn(&MsDistribution) -> f64| -> String {
            match self
                .callback_of(direction)
                .and_then(|item| item.driver_latency)
            {
                None => "none".to_owned(),
                Some(distribution) if driver_latency_unreported(&distribution) => {
                    DRIVER_LATENCY_UNREPORTED.to_owned()
                }
                Some(distribution) => format!("{:.4}", pick(&distribution)),
            }
        };
        let driver_reported = |direction: Direction| -> String {
            match self
                .callback_of(direction)
                .and_then(|item| item.driver_latency)
            {
                None => "none".to_owned(),
                Some(distribution) => (!driver_latency_unreported(&distribution)).to_string(),
            }
        };
        let callbacks = |direction: Direction| -> String {
            match self.callback_of(direction) {
                Some(item) => item.callbacks.to_string(),
                None => "none".to_owned(),
            }
        };
        let errors = |direction: Direction| -> String {
            match self.callback_of(direction) {
                Some(item) => item.backend_errors.to_string(),
                None => "none".to_owned(),
            }
        };
        format!(
            "BENCH baseline={BASELINE_NUMBER} label={label} verdict={verdict} evidence={evidence} \
devices={devices} output_devices={outputs} input_devices={inputs} \
measured_roundtrip_ms={measured} target_ms={target:.4} \
nominal_out_ms={nominal_out} nominal_in_ms={nominal_in} nominal_io_sum_ms={nominal_sum} \
nominal_io_sum_is_roundtrip=false \
callbacks_out={callbacks_out} callbacks_in={callbacks_in} \
backend_errors_out={errors_out} backend_errors_in={errors_in} \
jitter_out_p50_ms={out_p50} jitter_out_p99_ms={out_p99} jitter_out_max_ms={out_max} \
jitter_in_p50_ms={in_p50} jitter_in_p99_ms={in_p99} jitter_in_max_ms={in_max} \
driver_out_latency_source={DRIVER_OUT_LATENCY_SOURCE} \
driver_out_latency_reported={driver_out_reported} \
driver_out_latency_p50_ms={driver_out_p50} driver_out_latency_p99_ms={driver_out_p99} \
driver_out_latency_max_ms={driver_out_max} \
driver_in_latency_source={DRIVER_IN_LATENCY_SOURCE} \
driver_in_latency_reported={driver_in_reported} \
driver_in_latency_p50_ms={driver_in_p50} driver_in_latency_p99_ms={driver_in_p99} \
driver_in_latency_max_ms={driver_in_max} \
driver_io_sum_is_roundtrip=false loopback={loopback}",
            label = sanitize_label(&self.label),
            verdict = self.verdict().as_str(),
            evidence = self.evidence.as_str(),
            devices = self.device_count(),
            outputs = self
                .devices
                .iter()
                .filter(|device| device.direction == Direction::Output)
                .count(),
            inputs = self
                .devices
                .iter()
                .filter(|device| device.direction == Direction::Input)
                .count(),
            measured = self.measured_roundtrip_field(),
            target = self.target_ms,
            nominal_out = Self::opt_ms(output),
            nominal_in = Self::opt_ms(input),
            nominal_sum = sum,
            callbacks_out = callbacks(Direction::Output),
            callbacks_in = callbacks(Direction::Input),
            errors_out = errors(Direction::Output),
            errors_in = errors(Direction::Input),
            out_p50 = jitter(Direction::Output, |stats| stats.p50_ms),
            out_p99 = jitter(Direction::Output, |stats| stats.p99_ms),
            out_max = jitter(Direction::Output, |stats| stats.max_ms),
            in_p50 = jitter(Direction::Input, |stats| stats.p50_ms),
            in_p99 = jitter(Direction::Input, |stats| stats.p99_ms),
            in_max = jitter(Direction::Input, |stats| stats.max_ms),
            driver_out_reported = driver_reported(Direction::Output),
            driver_out_p50 = driver(Direction::Output, |distribution| distribution.p50_ms),
            driver_out_p99 = driver(Direction::Output, |distribution| distribution.p99_ms),
            driver_out_max = driver(Direction::Output, |distribution| distribution.max_ms),
            driver_in_reported = driver_reported(Direction::Input),
            driver_in_p50 = driver(Direction::Input, |distribution| distribution.p50_ms),
            driver_in_p99 = driver(Direction::Input, |distribution| distribution.p99_ms),
            driver_in_max = driver(Direction::Input, |distribution| distribution.max_ms),
            // `loopback=` = "这条读数**真的**是回环测量吗"：只有在证据等级确实是
            // 回环时才 `true`。⛔ 它**不**表示"这次调用加了 `--loopback`" ——
            // 加了但没检出时证据仍是 nominal-only（`loopback=false`），
            // 理由进 `notes` / `human_summary` / stderr。
            loopback = self.evidence == Evidence::LoopbackMeasured,
        )
    }

    /// 人读的摘要：**先说什么被测了、什么没被测**，再给数字。
    ///
    /// "无设备"路径的文案由 [`NO_DEVICE_REASON`] 决定，判据对着它断言。
    #[must_use]
    pub fn human_summary(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "MEASURES: nominal per-direction latency (device-reported buffer frames / sample rate); \
device-reported actual buffer frames after opening; host-reported driver-side latency \
(output playback-minus-callback, input callback-minus-capture) as p50/p99/max; callback arrival \
jitter (p50/p99/max) on this CPU; and - only with `--loopback` - a REAL output->input capture \
comparison: a linear chirp (200 Hz -> 8 kHz, ~50 ms) is played on the output stream and \
matched-filtered (normalized cross-correlation) over the input capture. That roundtrip is the \
difference between the two callbacks' own Instants (same process, same clock).\n",
        );
        out.push_str(
            "DOES-NOT-MEASURE: without `--loopback`, the acoustic / DAC-ADC roundtrip itself. The \
playback/capture instants are the HOST's own prediction (CoreAudio folds in \
kAudioDevicePropertyLatency + safety offset, WASAPI IAudioClient::GetStreamLatency, ALSA the PCM \
delay, PulseAudio an estimate) - they are not a measured loop. With `--loopback` the figure is \
still the HOST-OBSERVED callback-to-callback roundtrip: it starts when the output callback that \
carried the chirp was invoked (NOT when the samples left the DAC) and ends when the input callback \
that carried the chirp was invoked; it therefore includes the driver buffers and EXCLUDES the \
DAC/ADC analog sections. On a virtual loopback (BlackHole/Loopback) there is no AD/DA at all, so \
that number is a LOWER BOUND for a real hardware roundtrip - it is NOT equal to it.\n",
        );
        // 只有"有设备但没回环"才谈得上"没有回环证据"；没有设备时理由是 NO-DEVICE。
        if self.device_count() > 0 && self.evidence == Evidence::NominalOnly {
            out.push_str(&format!("NO-LOOPBACK: {NO_LOOPBACK_REASON}\n"));
        }
        if self.device_count() == 0 {
            out.push_str(&format!("NO-DEVICE: {NO_DEVICE_REASON}\n"));
        }
        // 只有**真的**有回环证据才报这一行（诚实闸的出口）；数值为 `none` 时也照实写
        // `none`（那时判定仍是 unmeasurable —— 见 `verdict_for`）。
        if self.evidence == Evidence::LoopbackMeasured {
            out.push_str(&format!(
                "LOOPBACK: measured_roundtrip_ms={ms} source=chirp-normalized-cross-correlation \
kind=host-observed-callback-to-callback-not-acoustic \
scope=includes-driver-buffers-excludes-DAC-ADC-analog \
note=virtual-loopback-is-a-LOWER-BOUND-not-a-real-hardware-roundtrip\n",
                ms = self.measured_roundtrip_field(),
            ));
        }
        for device in &self.devices {
            out.push_str(&format!(
                "DEVICE index={index} direction={direction} name=\"{name}\" driver={driver} \
interface={interface} channels={channels} sample_rate={rate} format={format} \
buffer_min={min} buffer_max={max} buffer_unknown={unknown} buffer_frames_reported={reported}\n",
                index = device.index,
                direction = device.direction.as_str(),
                name = device.name,
                driver = device.driver.as_deref().unwrap_or("unknown"),
                interface = device.interface.as_deref().unwrap_or("unknown"),
                channels = device
                    .default_channels
                    .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                rate = device
                    .default_sample_rate
                    .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                format = device.default_sample_format.as_deref().unwrap_or("unknown"),
                min = device
                    .buffer_min_frames
                    .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                max = device
                    .buffer_max_frames
                    .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                unknown = device.buffer_range_unknown,
                reported = device
                    .buffer_frames_reported
                    .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
            ));
        }
        for nominal in &self.nominals {
            out.push_str(&format!(
                "NOMINAL direction={direction} buffer_frames={frames} sample_rate={rate} \
latency_ms={ms:.4} kind=nominal-not-roundtrip\n",
                direction = nominal.direction.as_str(),
                frames = nominal.buffer_frames,
                rate = nominal.sample_rate_hz,
                ms = nominal.latency_ms,
            ));
        }
        for callback in &self.callbacks {
            match callback.stats {
                Some(stats) => out.push_str(&format!(
                    "CALLBACK direction={direction} callbacks={callbacks} backend_errors={errors} \
nominal_period_ms={period:.4} jitter_p50_ms={p50:.4} jitter_p99_ms={p99:.4} \
jitter_max_ms={max:.4} jitter_mean_ms={mean:.4} kind=cpu-scheduling-not-roundtrip\n",
                    direction = callback.direction.as_str(),
                    callbacks = callback.callbacks,
                    errors = callback.backend_errors,
                    period = stats.nominal_period_ms,
                    p50 = stats.p50_ms,
                    p99 = stats.p99_ms,
                    max = stats.max_ms,
                    mean = stats.mean_ms,
                )),
                None => out.push_str(&format!(
                    "CALLBACK direction={direction} callbacks={callbacks} backend_errors={errors} \
jitter=unmeasured kind=cpu-scheduling-not-roundtrip\n",
                    direction = callback.direction.as_str(),
                    callbacks = callback.callbacks,
                    errors = callback.backend_errors,
                )),
            }
            match callback.driver_latency {
                Some(distribution) if driver_latency_unreported(&distribution) => {
                    out.push_str(&format!(
                        "DRIVER-LATENCY direction={direction} source={source} reported=false \
value={DRIVER_LATENCY_UNREPORTED} note=the-host-reported-playback-equals-callback-so-this-is-\
NOT-zero-latency\n",
                        direction = callback.direction.as_str(),
                        source = match callback.direction {
                            Direction::Input => DRIVER_IN_LATENCY_SOURCE,
                            Direction::Output => DRIVER_OUT_LATENCY_SOURCE,
                        },
                    ))
                }
                Some(distribution) => out.push_str(&format!(
                    "DRIVER-LATENCY direction={direction} source={source} reported=true \
samples={samples} p50_ms={p50:.4} p99_ms={p99:.4} max_ms={max:.4} mean_ms={mean:.4} \
kind=host-reported-driver-side-not-acoustic\n",
                    direction = callback.direction.as_str(),
                    source = match callback.direction {
                        Direction::Input => DRIVER_IN_LATENCY_SOURCE,
                        Direction::Output => DRIVER_OUT_LATENCY_SOURCE,
                    },
                    samples = distribution.samples,
                    p50 = distribution.p50_ms,
                    p99 = distribution.p99_ms,
                    max = distribution.max_ms,
                    mean = distribution.mean_ms,
                )),
                None => out.push_str(&format!(
                    "DRIVER-LATENCY direction={direction} source={source} reported=none \
value=none kind=host-reported-driver-side-not-acoustic\n",
                    direction = callback.direction.as_str(),
                    source = match callback.direction {
                        Direction::Input => DRIVER_IN_LATENCY_SOURCE,
                        Direction::Output => DRIVER_OUT_LATENCY_SOURCE,
                    },
                )),
            }
        }
        for note in &self.notes {
            out.push_str(&format!("NOTE: {note}\n"));
        }
        out
    }
}

/// 把标签规范成机器可读行能承受的形式：空白与 `=` 一律换成 `-`。
#[must_use]
pub fn sanitize_label(label: &str) -> String {
    if label.is_empty() {
        return "default".to_owned();
    }
    label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

/// 解析 [`Report::bench_line`] 产出的机器可读行。
#[derive(Clone, Debug, PartialEq)]
pub struct BenchFields {
    /// `baseline=`（恒为 `005`，否则解析失败）。
    pub baseline: String,
    /// `label=`。
    pub label: String,
    /// `verdict=`。
    pub verdict: String,
    /// `evidence=`。
    pub evidence: String,
    /// `devices=`。
    pub devices: usize,
    /// `measured_roundtrip_ms=`；字面 `none` ⇒ `None`。
    pub measured_roundtrip_ms: Option<f64>,
    /// `target_ms=`。
    pub target_ms: f64,
    /// `nominal_out_ms=`；`none` ⇒ `None`。
    pub nominal_out_ms: Option<f64>,
    /// `nominal_in_ms=`；`none` ⇒ `None`。
    pub nominal_in_ms: Option<f64>,
    /// `nominal_io_sum_ms=`；`none` ⇒ `None`。
    pub nominal_io_sum_ms: Option<f64>,
    /// `nominal_io_sum_is_roundtrip=`。
    pub nominal_io_sum_is_roundtrip: bool,
    /// `loopback=`。
    pub loopback: bool,
    /// `jitter_out_p99_ms=`；`none` ⇒ `None`。
    pub jitter_out_p99_ms: Option<f64>,
    /// `jitter_in_p99_ms=`；`none` ⇒ `None`。
    pub jitter_in_p99_ms: Option<f64>,
    /// `driver_out_latency_p99_ms=`；`none` ⇒ `None`，
    /// 字面 `unreported-or-zero` ⇒ [`BenchFields::driver_out_latency_reported`] 为 `false`。
    pub driver_out_latency_p99_ms: Option<f64>,
    /// `driver_out_latency_reported=`；`none`（没采到）也记 `false`。
    pub driver_out_latency_reported: bool,
    /// `driver_in_latency_p99_ms=`；`none` ⇒ `None`。
    pub driver_in_latency_p99_ms: Option<f64>,
    /// `driver_in_latency_reported=`。
    pub driver_in_latency_reported: bool,
    /// `driver_io_sum_is_roundtrip=`（恒为 `false` —— 驱动侧**故意不给合计**）。
    pub driver_io_sum_is_roundtrip: bool,
}

/// 严格解析机器可读行：首 token 必须是 `BENCH`，其余每个 token 必须是 `key=value`，
/// 且 `baseline=005` 必须存在。
///
/// **严格是故意的**：它是"行长格式稳定"这条判据的判别力来源 ——
/// 任何一个游离 token、漏掉的键、或 `baseline` 写错都会让它返回 `None`。
#[must_use]
pub fn parse_bench_line(line: &str) -> Option<BenchFields> {
    let mut tokens = line.split_whitespace();
    if tokens.next()? != "BENCH" {
        return None;
    }
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for token in tokens {
        let (key, value) = token.split_once('=')?;
        if key.is_empty() {
            return None;
        }
        pairs.push((key, value));
    }
    let baseline = lookup(&pairs, "baseline")?;
    if baseline != BASELINE_NUMBER {
        return None;
    }
    Some(BenchFields {
        baseline: baseline.to_owned(),
        label: lookup(&pairs, "label")?.to_owned(),
        verdict: lookup(&pairs, "verdict")?.to_owned(),
        evidence: lookup(&pairs, "evidence")?.to_owned(),
        devices: lookup(&pairs, "devices")?.parse::<usize>().ok()?,
        measured_roundtrip_ms: number_field(&pairs, "measured_roundtrip_ms")?,
        target_ms: lookup(&pairs, "target_ms")?.parse::<f64>().ok()?,
        nominal_out_ms: number_field(&pairs, "nominal_out_ms")?,
        nominal_in_ms: number_field(&pairs, "nominal_in_ms")?,
        nominal_io_sum_ms: number_field(&pairs, "nominal_io_sum_ms")?,
        nominal_io_sum_is_roundtrip: lookup(&pairs, "nominal_io_sum_is_roundtrip")? == "true",
        loopback: lookup(&pairs, "loopback")? == "true",
        jitter_out_p99_ms: number_field(&pairs, "jitter_out_p99_ms")?,
        jitter_in_p99_ms: number_field(&pairs, "jitter_in_p99_ms")?,
        driver_out_latency_p99_ms: driver_number_field(&pairs, "driver_out_latency_p99_ms")?,
        driver_out_latency_reported: lookup(&pairs, "driver_out_latency_reported")? == "true",
        driver_in_latency_p99_ms: driver_number_field(&pairs, "driver_in_latency_p99_ms")?,
        driver_in_latency_reported: lookup(&pairs, "driver_in_latency_reported")? == "true",
        driver_io_sum_is_roundtrip: lookup(&pairs, "driver_io_sum_is_roundtrip")? == "true",
    })
}

/// 在 `key=value` 序列里找 `key`（用显式生命周期，避免闭包返回借用时的推断陷阱）。
fn lookup<'a>(pairs: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, value)| *value)
}

/// 读一个"数值或字面 `none`"的字段。外层 `None` = **字段缺失**（坏行），
/// 内层 `None` = 字段存在但值为 `none`（**没测到**）。这两件事不许混。
fn number_field(pairs: &[(&str, &str)], key: &str) -> Option<Option<f64>> {
    match lookup(pairs, key)? {
        "none" => Some(None),
        raw => Some(Some(raw.parse::<f64>().ok()?)),
    }
}

/// 读**驱动侧时延**字段：`none`（没采到）与 [`DRIVER_LATENCY_UNREPORTED`]（主机不报）
/// 都 → `None`。**这两个字面量都不是数字**，所以谁也不能把它们当成 0 ms。
fn driver_number_field(pairs: &[(&str, &str)], key: &str) -> Option<Option<f64>> {
    match lookup(pairs, key)? {
        "none" => Some(None),
        DRIVER_LATENCY_UNREPORTED => Some(None),
        raw => Some(Some(raw.parse::<f64>().ok()?)),
    }
}

// ===========================================================================
// `--loopback` 腿（回环往返）的**纯计算**那一半：线性 chirp + 归一化互相关
// ===========================================================================
//
// 为什么放在这里、而不是 `examples/measure_latency.rs`：本文件是**零依赖**的判定面
// （见模块头），能在没有声卡、甚至没有 `device` feature 的机器上真跑
// ⇒ 把"能不能检出"的核心做成纯函数放这里，判据就不必依赖 cpal。
// ⛔ 这里不引用 cpal / rtrb / 本 crate 其它模块，也不引入新依赖。

/// `--loopback` 腿的 chirp **起始频率**（Hz）。
pub const LOOPBACK_CHIRP_START_HZ: f64 = 200.0;

/// `--loopback` 腿的 chirp **终止频率**（Hz）。
pub const LOOPBACK_CHIRP_END_HZ: f64 = 8_000.0;

/// `--loopback` 腿的 chirp **时长**（毫秒）。
///
/// 50 ms 是折中：48 kHz 下 2 400 帧，时间带宽积远大于 1（互相关峰尖锐），
/// 又远短于"往返窗"，不会盖住下一次比对。
pub const LOOPBACK_CHIRP_MS: f64 = 50.0;

/// 判定"检出"的**峰值比**阈值 = `|互相关峰| / 主瓣外次高峰`（SNR 类读数）。
///
/// 口径：白噪声里 `|ncc|` 的最大与次大（主瓣外）几乎一样（比值 ≈ 1）；真检出时
/// 主瓣外旁瓣低一个量级（比值 ≫ 1）⇒ 阈值 `3.0` 把"相关峰**突出**"与"随机起伏"
/// 分开。注入实测：把它改成 `0.0` ⇒ 纯噪声判据变红
/// （见 `mod tests` 的 `detect_delay_frames_rejects_pure_noise_zeros_and_constants`）。
pub const LOOPBACK_PEAK_RATIO_THRESHOLD: f32 = 3.0;

/// 归一化互相关分母的下限（防 0/0）。**不是**阈值。
const NCC_EPS: f32 = 1e-12;

/// [`detect_delay_frames`] 的检出结果。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detected {
    /// 参考信号在 `recorded` 里的**起点**（帧）= 互相关峰所在的滞后 `d`。
    pub delay_frames: usize,
    /// 峰值处的**归一化互相关**（带符号；`±1` = 完全对齐）。
    pub peak_ncc: f32,
    /// **峰值比**（SNR 类读数）：`|峰| / 次高 |ncc|`。
    ///
    /// 次高**在互相关主瓣之外**取（见 [`detect_delay_frames`] 的"主瓣保护带"口径）
    /// —— 否则紧邻峰值的滞后与峰值几乎一样高（重叠了 `L−1` 个样本），比值恒 ≈ 1。
    /// 主瓣与旁瓣之外都没有别的滞后时（录制窗太短）为 `+∞`。
    pub peak_ratio: f32,
    /// 扫过的滞后总数（`recorded.len() - reference.len() + 1`）。
    /// `1` ⇒ 没有任何对照滞后，检出**没有判别力**（调用方读它自检）。
    pub lags: usize,
}

/// 生成一条**线性 chirp**参考信号（单声道 `f32`）。
///
/// 口径：瞬时频率 `f(t) = f0 + (f1 - f0)·t/T`，相位
/// `φ(t) = 2π(f0·t + (f1 - f0)·t²/(2T))`；首尾各 5% 做**半余弦淡入淡出**
/// —— ⛔ 不用单点脉冲（脉冲的时间带宽积 ≈ 1，抗噪差且容易与咔哒混淆）。
///
/// 返回长度 `round(sample_rate · ms / 1000)`；`sample_rate == 0`、`ms` 非正或非有限
/// ⇒ 空切片（⛔ 不发明信号）。
#[must_use]
pub fn loopback_chirp(sample_rate_hz: u32, chirp_ms: f64) -> Vec<f32> {
    /// 首尾淡入淡出各占 chirp 的比例。
    const FADE_FRACTION: f64 = 0.05;
    if sample_rate_hz == 0 || !chirp_ms.is_finite() || chirp_ms <= 0.0 {
        return Vec::new();
    }
    let length = (f64::from(sample_rate_hz) * chirp_ms / 1000.0).round() as usize;
    if length == 0 {
        return Vec::new();
    }
    let rate = f64::from(sample_rate_hz);
    let total = length as f64;
    let sweep = LOOPBACK_CHIRP_END_HZ - LOOPBACK_CHIRP_START_HZ;
    let fade = ((total * FADE_FRACTION).round() as usize).max(1);
    let mut samples = Vec::with_capacity(length);
    for index in 0..length {
        let t = index as f64 / rate;
        let phase = std::f64::consts::TAU
            * (LOOPBACK_CHIRP_START_HZ * t + sweep * t * t / (2.0 * total / rate));
        let mut value = libm::sin(phase) as f32;
        if index < fade {
            let ramp = 0.5 - 0.5 * libm::cos(std::f64::consts::PI * index as f64 / fade as f64);
            value *= ramp as f32;
        }
        if index + fade >= length {
            let tail = (length - 1 - index) as f64 / fade as f64;
            let ramp = 0.5 - 0.5 * libm::cos(std::f64::consts::PI * tail);
            value *= ramp as f32;
        }
        samples.push(value);
    }
    samples
}

/// **归一化互相关（matched filter）**：检出参考信号在录音里的滞后。
///
/// 纯函数：无 I/O、无时钟、无随机数 ⇒ 同一批输入永远给同一批输出。它跑在**控制侧**
/// （可以分配 `Vec`）；⛔ 不是实时回调体。
///
/// # 口径（判据钉死的就是这几条）
///
/// 记 `L = reference.len()`、`M = recorded.len()`（要求 `M >= L`）。对每个滞后
/// `d ∈ 0..=(M-L)`：
/// `ncc[d] = (Σ_i ref[i]·rec[d+i]) / (‖ref‖·‖rec[d..d+L]‖)`；分母 `≤` [`NCC_EPS`]
/// ⇒ `ncc[d] = 0`。取 `d* = argmax|ncc|`（并列取**最小** `d` ⇒ 确定性），于是：
///
/// - `delay_frames = d*`、`peak_ncc = ncc[d*]`；
/// - `peak_ratio = |ncc[d*]| / max(主瓣外次高 |ncc[d]|, NCC_EPS)`；
/// - 主瓣保护带 = `±max(reference.len()/8, 1)` 个滞后（匹配滤波的主瓣宽度随
///   时间带宽积缩小；实测相邻滞后重叠 `L−1` 样本，不设保护带则比值恒 ≈ 1）；
/// - `reference` 为空 / `M < L` / 参考能量 `≤` [`NCC_EPS`] ⇒ `None`；
/// - `|peak| == 0` 或 `peak_ratio < threshold` ⇒ `None`（**没有检出**）。
///
/// ⛔ 返回 `Some` **只**说明"这条参考信号被相关出来了"，**不**说明往返时延达标
/// —— 达标与否只由 [`verdict_for`] 判（它要求 [`Evidence::LoopbackMeasured`]）。
#[must_use]
pub fn detect_delay_frames(
    reference: &[f32],
    recorded: &[f32],
    threshold: f32,
) -> Option<Detected> {
    let length = reference.len();
    if length == 0 || recorded.len() < length {
        return None;
    }
    let reference_energy: f64 = reference
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    if reference_energy <= f64::from(NCC_EPS) {
        return None;
    }
    let reference_norm = reference_energy.sqrt();
    let lags = recorded.len() - length + 1;
    let mut ncc = vec![0.0f32; lags];
    for d in 0..lags {
        let mut dot = 0.0f64;
        let mut energy = 0.0f64;
        for i in 0..length {
            let x = f64::from(reference[i]);
            let y = f64::from(recorded[d + i]);
            dot += x * y;
            energy += y * y;
        }
        let denominator = reference_norm * energy.sqrt();
        ncc[d] = if denominator <= f64::from(NCC_EPS) {
            0.0
        } else {
            (dot / denominator) as f32
        };
    }
    let mut best = 0usize;
    let mut best_abs = 0.0f32;
    for (d, value) in ncc.iter().enumerate() {
        let magnitude = value.abs();
        if magnitude > best_abs {
            best_abs = magnitude;
            best = d;
        }
    }
    let guard = (length / 8).max(1);
    let mut second_abs = 0.0f32;
    for (d, value) in ncc.iter().enumerate() {
        if d.abs_diff(best) > guard {
            let magnitude = value.abs();
            if magnitude > second_abs {
                second_abs = magnitude;
            }
        }
    }
    let peak_ratio = if second_abs <= NCC_EPS {
        if best_abs <= NCC_EPS {
            0.0
        } else {
            f32::INFINITY
        }
    } else {
        best_abs / second_abs
    };
    if best_abs <= 0.0 || peak_ratio < threshold {
        return None;
    }
    Some(Detected {
        delay_frames: best,
        peak_ncc: ncc[best],
        peak_ratio,
        lags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(direction: Direction, index: usize) -> DeviceReport {
        DeviceReport {
            index,
            direction,
            name: format!("fake-{}-{index}", direction.as_str()),
            driver: Some("fake-driver".to_owned()),
            interface: Some("Built-in".to_owned()),
            default_channels: Some(2),
            default_sample_rate: Some(48_000),
            default_sample_format: Some("f32".to_owned()),
            buffer_min_frames: Some(32),
            buffer_max_frames: Some(1024),
            buffer_range_unknown: false,
            buffer_frames_reported: Some(64),
        }
    }

    /// 判据①：标称时延换算正确（**纯计算**，无声卡也能真跑）。
    #[test]
    fn nominal_latency_conversion_is_exact() {
        let sixty_four = nominal_ms(64, 48_000).expect("48 kHz 合法");
        assert!(
            (sixty_four - 1.333_333_333).abs() < 1e-9,
            "64/48000 应为 1.3333… ms，实际 {sixty_four}"
        );
        let one_twenty_eight = nominal_ms(128, 48_000).expect("48 kHz 合法");
        assert!((one_twenty_eight - 2.666_666_667).abs() < 1e-9);
        // ARCH §3.5 的两个锚点：64 帧 @48k = 1.33 ms；128 帧 @44.1k = 2.9025 ms
        let at_44k = nominal_ms(128, 44_100).expect("44.1 kHz 合法");
        assert!((at_44k - 2.902_494_331).abs() < 1e-9);
        // 0 帧是退化但合法的输入 ⇒ 0 ms；0 采样率是**没信息** ⇒ None（不许发明数）
        assert_eq!(nominal_ms(0, 48_000), Some(0.0));
        assert_eq!(nominal_ms(64, 0), None, "采样率 0 时必须返回 None");
    }

    /// 判据②：抖动统计的**纯计算**部分（人造时间戳 ⇒ 钉死 p50/p99/max/mean）。
    #[test]
    fn jitter_percentiles_are_exact_on_handmade_timestamps() {
        // 标称周期 1 ms；间隔 1 ms / 1 ms / 1.002 ms / 1 ms / 101 ms
        // ⇒ 偏差（ms）= 0, 0, 0.002, 0, 100
        let nominal_ns = 1_000_000_u64;
        let intervals = [1_000_000_u64, 1_000_000, 1_002_000, 1_000_000, 101_000_000];
        let stats = jitter_stats(&intervals, nominal_ns).expect("非空");
        assert_eq!(stats.samples, 5);
        assert!((stats.nominal_period_ms - 1.0).abs() < 1e-12);
        // 升序偏差 [0,0,0,0.002,100]：p50 = ceil(0.5*5)=3 ⇒ 下标 2 ⇒ 0
        assert!((stats.p50_ms - 0.0).abs() < 1e-12, "p50={}", stats.p50_ms);
        // p99 = ceil(0.99*5)=5 ⇒ 下标 4 ⇒ 100
        assert!((stats.p99_ms - 100.0).abs() < 1e-12, "p99={}", stats.p99_ms);
        assert!((stats.max_ms - 100.0).abs() < 1e-12);
        assert!(
            (stats.mean_ms - 20.000_4).abs() < 1e-9,
            "mean={}",
            stats.mean_ms
        );
        // ⚠ 右偏分布下 p99 ≠ mean —— 这条断言让"均值冒充 p99"的注入必然变红。
        assert!(
            stats.p99_ms > stats.mean_ms,
            "p99({}) 必须严格大于 mean({})：否则均值冒充 p99 无法被抓住",
            stats.p99_ms,
            stats.mean_ms
        );
    }

    /// 判据②附带：完美规律的流 ⇒ 抖动恒为 0（工具不会自己造噪声）。
    #[test]
    fn perfectly_regular_callbacks_have_zero_jitter() {
        let intervals = [480_000_u64; 64]; // 64 帧 @48k = 1 ms = 1_000_000 ns… 这里用 0.48 ms 做对照
        let stats = jitter_stats(&intervals, 480_000).expect("非空");
        assert_eq!(stats.p50_ms, 0.0);
        assert_eq!(stats.p99_ms, 0.0);
        assert_eq!(stats.max_ms, 0.0);
        assert_eq!(stats.mean_ms, 0.0);
    }

    /// 判据②附带：**"没采到" ≠ "抖动 0"**。
    #[test]
    fn empty_or_degenerate_samples_never_invent_a_number() {
        assert_eq!(jitter_stats(&[], 1_000_000), None);
        assert_eq!(percentile_nearest_rank(&[], 50.0), None);
        assert_eq!(percentile_nearest_rank(&[1.0], f64::NAN), None);
    }

    /// 判据⑥（核心）：**没有回环证据 ⇒ 永远不许判达标**，标称值再小也不行。
    #[test]
    fn nominal_values_can_never_satisfy_the_baseline() {
        // 荒谬地"好"的标称值也翻不了盘 —— 这正是"标称冒充实测"注入必须撞的墙。
        let verdict = verdict_for(2, Evidence::NominalOnly, Some(0.001));
        assert_eq!(
            verdict,
            Verdict::UnmeasurableWithoutLoopback,
            "标称证据下绝不允许出现 within-target"
        );
        assert!(!verdict.is_pass());
        // 就算调用方把标称值塞进 measured_roundtrip_ms，只要 evidence 还是 nominal-only 就不认。
        assert_eq!(
            verdict_for(2, Evidence::NominalOnly, Some(1.0)).as_str(),
            "unmeasurable-without-loopback"
        );
        // 声称有回环但没有数字 ⇒ 同样不认（"没有数字"不是"数字很好看"）。
        assert_eq!(
            verdict_for(2, Evidence::LoopbackMeasured, None),
            Verdict::UnmeasurableWithoutLoopback
        );
        assert_eq!(
            verdict_for(2, Evidence::LoopbackMeasured, Some(f64::NAN)),
            Verdict::UnmeasurableWithoutLoopback
        );
    }

    /// 判据⑥反面：**有回环证据才**允许达标/超目标，且阈值边界精确。
    #[test]
    fn loopback_evidence_is_the_only_route_to_a_pass() {
        assert_eq!(
            verdict_for(1, Evidence::LoopbackMeasured, Some(5.5)),
            Verdict::WithinTarget,
            "恰好 5.5 ms 算达标（≤）"
        );
        assert_eq!(
            verdict_for(1, Evidence::LoopbackMeasured, Some(5.500_001)),
            Verdict::OverTarget
        );
        assert!(verdict_for(1, Evidence::LoopbackMeasured, Some(5.5)).is_pass());
        assert!(!verdict_for(1, Evidence::LoopbackMeasured, Some(6.0)).is_pass());
    }

    /// 判据③：**"无设备"路径**的判定/退出码/文案精确，且绝不被写成 0 ms。
    #[test]
    fn the_no_device_path_reports_no_device_and_never_zero_ms() {
        let report = Report {
            label: "no-device".to_owned(),
            requested_frames: 64,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices: Vec::new(),
            nominals: Vec::new(),
            callbacks: Vec::new(),
            evidence: Evidence::NominalOnly,
            measured_roundtrip_ms: None,
            notes: Vec::new(),
        };
        let verdict = report.verdict();
        assert_eq!(verdict, Verdict::NoDevice);
        assert_eq!(verdict.as_str(), "no-device");
        assert!(!verdict.is_pass(), "无设备**不是**达标");
        assert_eq!(exit_code_for(verdict, false), EXIT_NO_DEVICE);
        assert_ne!(exit_code_for(verdict, false), 0, "无设备必须非零退出");

        let line = report.bench_line();
        let fields = parse_bench_line(&line).expect("机器可读行必须可解析");
        assert_eq!(fields.verdict, "no-device");
        assert_eq!(fields.devices, 0);
        assert_eq!(
            fields.measured_roundtrip_ms, None,
            "无设备时不许有实测往返数"
        );
        assert_eq!(fields.nominal_out_ms, None, "无设备时不许有标称数");
        assert_eq!(fields.nominal_in_ms, None);
        assert_eq!(fields.nominal_io_sum_ms, None);
        assert!(!line.contains("within-target"), "行里不许出现达标字样");
        assert!(
            !line.contains("0.0000"),
            "无设备时行里不许出现任何看起来像 0 ms 的读数: {line}"
        );

        // 文案必须**出声**：明确说"没测到 / 不是达标 / 不是 0 ms"。
        let summary = report.human_summary();
        assert!(summary.contains("NO-DEVICE"));
        assert!(summary.contains(NO_DEVICE_REASON));
        assert!(summary.contains("NOT a pass and NOT 0 ms"));
        assert!(summary.contains("DOES-NOT-MEASURE"));
        assert!(summary.contains("MEASURES"));
    }

    /// 判据⑤：机器可读行的格式稳定 —— 写出 → 严格解析 → 逐个字段对账。
    #[test]
    fn bench_line_round_trips_through_the_strict_parser() {
        let normal = jitter_stats(&[1_000_000, 1_010_000, 1_000_000], 1_000_000).expect("非空");
        let report = Report {
            // 故意塞入空白与 '='：标签必须被规范化，**不许**破坏行长格式
            label: "lab with = spaces".to_owned(),
            requested_frames: 64,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices: vec![device(Direction::Output, 0), device(Direction::Input, 1)],
            nominals: vec![
                NominalReport {
                    direction: Direction::Output,
                    buffer_frames: 64,
                    sample_rate_hz: 48_000,
                    latency_ms: nominal_ms(64, 48_000).expect("合法"),
                },
                NominalReport {
                    direction: Direction::Input,
                    buffer_frames: 64,
                    sample_rate_hz: 48_000,
                    latency_ms: nominal_ms(64, 48_000).expect("合法"),
                },
            ],
            callbacks: vec![
                CallbackReport {
                    direction: Direction::Output,
                    callbacks: 4,
                    backend_errors: 0,
                    stats: Some(normal),
                    // 主机报的驱动侧时延：2.5 / 2.9 ms
                    driver_latency: distribution_ms(&[2_500_000, 2_500_000, 2_900_000, 2_500_000]),
                },
                CallbackReport {
                    direction: Direction::Input,
                    callbacks: 2,
                    backend_errors: 0,
                    stats: Some(normal),
                    driver_latency: distribution_ms(&[1_000_000, 1_000_000]),
                },
            ],
            evidence: Evidence::NominalOnly,
            measured_roundtrip_ms: None,
            notes: Vec::new(),
        };
        let line = report.bench_line();
        assert_eq!(line.lines().count(), 1, "机器可读行必须是单行");
        let fields = parse_bench_line(&line).expect("必须可解析");
        assert_eq!(fields.baseline, "005");
        assert_eq!(fields.label, "lab-with---spaces");
        assert_eq!(fields.verdict, "unmeasurable-without-loopback");
        assert_eq!(fields.evidence, "nominal-only");
        assert_eq!(fields.devices, 2);
        assert_eq!(fields.measured_roundtrip_ms, None);
        assert!((fields.target_ms - 5.5).abs() < 1e-12);
        assert!((fields.nominal_out_ms.expect("有输出标称") - 1.333_3).abs() < 1e-3);
        assert!((fields.nominal_io_sum_ms.expect("有合计") - 2.666_6).abs() < 1e-3);
        assert!(!fields.nominal_io_sum_is_roundtrip, "合计不许被当成往返");
        assert!(!fields.loopback, "本工具不声称回环");
        assert!(fields.jitter_out_p99_ms.expect("有输出抖动") >= 0.0);
        assert!(fields.jitter_in_p99_ms.expect("有输入抖动") >= 0.0);
        // 驱动侧时延：**有数字**、来源被写进行里、且**绝不是**往返。
        assert!(fields.driver_out_latency_reported, "驱动侧输出时延应被报出");
        let driver_out = fields.driver_out_latency_p99_ms.expect("有输出驱动时延");
        assert!(
            (driver_out - 2.9).abs() < 1e-9,
            "driver_out p99={driver_out}"
        );
        assert!((fields.driver_in_latency_p99_ms.expect("有输入驱动时延") - 1.0).abs() < 1e-9);
        assert!(
            line.contains("driver_out_latency_source=playback_minus_callback"),
            "行里必须写清驱动侧时延的推导口径"
        );
        assert!(line.contains("driver_in_latency_source=callback_minus_capture"));
        assert!(
            !fields.driver_io_sum_is_roundtrip,
            "驱动侧合计不许被当成往返"
        );
        // ⚠ 关键：**有驱动侧读数也不允许判定达标** —— 它是主机预测，不是声学往返。
        assert!(!report.verdict().is_pass());
        assert_eq!(report.verdict(), Verdict::UnmeasurableWithoutLoopback);
    }

    /// 判据②附带（新）：[`distribution_ms`] 的纯计算口径。
    #[test]
    fn driver_latency_distribution_is_exact_on_handmade_values() {
        // 2.5 / 2.5 / 2.5 / 2.9 ms ⇒ 升序 [2.5,2.5,2.5,2.9]
        let distribution =
            distribution_ms(&[2_500_000, 2_500_000, 2_500_000, 2_900_000]).expect("非空");
        assert_eq!(distribution.samples, 4);
        // p50 = ceil(0.5*4)=2 ⇒ 下标 1 ⇒ 2.5
        assert!((distribution.p50_ms - 2.5).abs() < 1e-12);
        // p99 = ceil(0.99*4)=4 ⇒ 下标 3 ⇒ 2.9
        assert!((distribution.p99_ms - 2.9).abs() < 1e-12);
        assert!((distribution.max_ms - 2.9).abs() < 1e-12);
        assert!((distribution.mean_ms - 2.6).abs() < 1e-12);
        assert!(!driver_latency_unreported(&distribution));
        assert_eq!(distribution_ms(&[]), None, "没采到 ≠ 0 ms");
    }

    /// 判据⑥补充：**"主机不报"绝不能变成"0 ms"**，更不能变成达标。
    #[test]
    fn unreported_driver_latency_is_not_zero_and_never_a_pass() {
        // 主机把 playback 与 callback 报成同一时刻 ⇒ 全是 0 样本。
        let all_zero = distribution_ms(&[0, 0, 0]).expect("非空");
        assert!(
            driver_latency_unreported(&all_zero),
            "全 0 必须判为 unreported"
        );
        let report = Report {
            label: "driver-unreported".to_owned(),
            requested_frames: 64,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices: vec![device(Direction::Output, 0)],
            nominals: Vec::new(),
            callbacks: vec![CallbackReport {
                direction: Direction::Output,
                callbacks: 3,
                backend_errors: 0,
                stats: None,
                driver_latency: Some(all_zero),
            }],
            evidence: Evidence::NominalOnly,
            measured_roundtrip_ms: None,
            notes: Vec::new(),
        };
        let line = report.bench_line();
        assert!(
            line.contains(&format!(
                "driver_out_latency_p99_ms={DRIVER_LATENCY_UNREPORTED}"
            )),
            "主机不报时必须写字面 unreported-or-zero，不许写 0.0000: {line}"
        );
        assert!(
            !line.contains("driver_out_latency_p99_ms=0.0000"),
            "把'不报'写成 0.0000 就是制造假绿"
        );
        let fields = parse_bench_line(&line).expect("可解析");
        assert!(!fields.driver_out_latency_reported);
        assert_eq!(fields.driver_out_latency_p99_ms, None);
        assert!(!report.verdict().is_pass(), "主机不报 ≠ 达标");
        // 人类可读行也必须出声。
        let summary = report.human_summary();
        assert!(summary.contains(DRIVER_LATENCY_UNREPORTED));
        assert!(
            summary.contains("NOT-zero-latency"),
            "必须明确说'这不是 0 ms 时延'"
        );
    }

    /// 判据⑥补充：**驱动侧时延再小也不构成达标**（它是主机预测，不是声学往返）。
    #[test]
    fn a_tiny_driver_latency_still_cannot_satisfy_the_baseline() {
        let tiny = distribution_ms(&[1, 1, 1]).expect("非空"); // 3 ns ≈ 0.000003 ms
        let report = Report {
            label: "driver-tiny".to_owned(),
            requested_frames: 64,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices: vec![device(Direction::Output, 0), device(Direction::Input, 1)],
            nominals: Vec::new(),
            callbacks: vec![
                CallbackReport {
                    direction: Direction::Output,
                    callbacks: 3,
                    backend_errors: 0,
                    stats: None,
                    driver_latency: Some(tiny),
                },
                CallbackReport {
                    direction: Direction::Input,
                    callbacks: 3,
                    backend_errors: 0,
                    stats: None,
                    driver_latency: Some(tiny),
                },
            ],
            evidence: Evidence::NominalOnly,
            measured_roundtrip_ms: None,
            notes: Vec::new(),
        };
        assert_eq!(
            report.verdict(),
            Verdict::UnmeasurableWithoutLoopback,
            "驱动侧读数（无论多小）都不许把判定变成达标"
        );
        assert!(!report.verdict().is_pass());
        // 而且行里**故意没有**驱动侧合计字段（合计最像"达标总数"）。
        let line = report.bench_line();
        assert!(
            !line.contains("driver_io_sum_ms"),
            "不许出现驱动侧合计字段（避免看起来达标的总数）"
        );
    }

    /// 判据⑤反面：解析器**有判别力**（坏行必须被拒），不是恒绿。
    #[test]
    fn the_parser_rejects_malformed_lines() {
        let good = "BENCH baseline=005 label=x verdict=no-device evidence=nominal-only devices=0 \
measured_roundtrip_ms=none target_ms=5.5000 nominal_out_ms=none nominal_in_ms=none \
nominal_io_sum_ms=none nominal_io_sum_is_roundtrip=false loopback=false \
jitter_out_p99_ms=none jitter_in_p99_ms=none \
driver_out_latency_p99_ms=none driver_out_latency_reported=false \
driver_in_latency_p99_ms=none driver_in_latency_reported=false \
driver_io_sum_is_roundtrip=false";
        assert!(parse_bench_line(good).is_some());
        // 前缀不对
        assert!(parse_bench_line(&good.replacen("BENCH", "BENCHMARK", 1)).is_none());
        // 门禁号不对（防串线）
        assert!(parse_bench_line(&good.replacen("baseline=005", "baseline=002", 1)).is_none());
        // 游离 token（没有 '='）
        assert!(parse_bench_line(&format!("{good} oops")).is_none());
        // 缺关键字段
        assert!(parse_bench_line(&good.replacen("verdict=no-device ", "", 1)).is_none());
        // 数值字段不是数字
        assert!(
            parse_bench_line(&good.replacen("target_ms=5.5000", "target_ms=fast", 1)).is_none()
        );
        // 空
        assert!(parse_bench_line("").is_none());
    }

    /// 判据⑤附带：帧数未知时**不许**有标称数字（宁可 `none`）。
    #[test]
    fn unknown_buffer_frames_produce_none_instead_of_a_guess() {
        let report = Report {
            label: "unknown-frames".to_owned(),
            requested_frames: 64,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices: vec![device(Direction::Output, 0)],
            nominals: Vec::new(),
            callbacks: Vec::new(),
            evidence: Evidence::NominalOnly,
            measured_roundtrip_ms: None,
            notes: Vec::new(),
        };
        let fields = parse_bench_line(&report.bench_line()).expect("可解析");
        assert_eq!(fields.nominal_out_ms, None);
        assert_eq!(fields.nominal_in_ms, None);
        assert_eq!(fields.nominal_io_sum_ms, None);
        assert_eq!(fields.jitter_out_p99_ms, None);
    }

    /// 判据⑤附带：退出码把"没测到"和"达标"分开，`--allow-unmeasurable` 只解决 CI 便利性。
    #[test]
    fn exit_codes_separate_unmeasured_from_pass() {
        assert_eq!(exit_code_for(Verdict::WithinTarget, false), 0);
        assert_eq!(exit_code_for(Verdict::OverTarget, false), 1);
        assert_eq!(exit_code_for(Verdict::NoDevice, false), EXIT_NO_DEVICE);
        assert_ne!(EXIT_NO_DEVICE, 0);
        assert_eq!(
            exit_code_for(Verdict::UnmeasurableWithoutLoopback, false),
            EXIT_UNMEASURABLE
        );
        assert_ne!(EXIT_UNMEASURABLE, 0);
        // 便利开关：把"没测到"变成 CI 不红，但判定不动。
        assert_eq!(exit_code_for(Verdict::NoDevice, true), 0);
        assert_eq!(exit_code_for(Verdict::UnmeasurableWithoutLoopback, true), 0);
        assert_eq!(exit_code_for(Verdict::OverTarget, true), 1);
        // 关键：开关**不会**把任何"没测到"变成达标判定。
        assert!(!Verdict::NoDevice.is_pass());
        assert!(!Verdict::UnmeasurableWithoutLoopback.is_pass());
    }

    /// 判据⑤附带：`--allow-unmeasurable` 不改机器可读行的**任何一个字节**。
    #[test]
    fn the_convenience_flag_never_changes_the_machine_readable_verdict() {
        let report = Report {
            label: "flag".to_owned(),
            requested_frames: 64,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices: vec![device(Direction::Output, 0)],
            nominals: Vec::new(),
            callbacks: Vec::new(),
            evidence: Evidence::NominalOnly,
            measured_roundtrip_ms: None,
            notes: Vec::new(),
        };
        let verdict = report.verdict();
        let line = report.bench_line();
        let fields = parse_bench_line(&line).expect("可解析");
        assert_eq!(fields.verdict, "unmeasurable-without-loopback");
        assert_eq!(exit_code_for(verdict, true), 0);
        assert_eq!(fields.verdict, verdict.as_str(), "开关不许改判定");
        assert_eq!(fields.measured_roundtrip_ms, None);
    }

    /// 判据④的纯计算侧：有设备时标称与抖动都**非负**且真的被打印出来。
    #[test]
    fn device_present_reports_non_negative_nominal_and_jitter() {
        let report = Report {
            label: "with-device".to_owned(),
            requested_frames: 64,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices: vec![device(Direction::Output, 0), device(Direction::Input, 1)],
            nominals: vec![NominalReport {
                direction: Direction::Output,
                buffer_frames: 64,
                sample_rate_hz: 48_000,
                latency_ms: nominal_ms(64, 48_000).expect("合法"),
            }],
            callbacks: vec![CallbackReport {
                direction: Direction::Output,
                callbacks: 2000,
                backend_errors: 0,
                stats: jitter_stats(&[1_000_000, 1_050_000], 1_000_000),
                driver_latency: distribution_ms(&[3_000_000, 3_100_000]),
            }],
            evidence: Evidence::NominalOnly,
            measured_roundtrip_ms: None,
            notes: Vec::new(),
        };
        assert_eq!(report.verdict(), Verdict::UnmeasurableWithoutLoopback);
        let summary = report.human_summary();
        assert!(summary.contains("NOMINAL direction=output"));
        assert!(summary.contains("CALLBACK direction=output"));
        assert!(summary.contains("kind=cpu-scheduling-not-roundtrip"));
        assert!(summary.contains("kind=nominal-not-roundtrip"));
        let nominal = report.nominal_of(Direction::Output).expect("有标称");
        assert!(nominal >= 0.0);
        let stats = report
            .callback_of(Direction::Output)
            .and_then(|item| item.stats)
            .expect("有抖动");
        assert!(stats.p50_ms >= 0.0 && stats.p99_ms >= 0.0 && stats.max_ms >= 0.0);
        assert!(stats.p99_ms >= stats.p50_ms, "p99 不该小于 p50");
        assert!(stats.max_ms >= stats.p99_ms, "max 不该小于 p99");
        // 驱动侧时延同样要打印且非负。
        assert!(summary.contains("DRIVER-LATENCY direction=output"));
        assert!(summary.contains("kind=host-reported-driver-side-not-acoustic"));
        let driver = report
            .callback_of(Direction::Output)
            .and_then(|item| item.driver_latency)
            .expect("有驱动侧时延");
        assert!((driver.p50_ms - 3.0).abs() < 1e-9);
        assert!((driver.max_ms - 3.1).abs() < 1e-9);
        assert!(driver.p50_ms >= 0.0 && driver.max_ms >= driver.p50_ms);
    }

    /// 阈值与门禁号的字面量被规范钉死 —— 改它们必须改规范。
    #[test]
    fn target_and_baseline_id_match_the_specification() {
        assert!((TARGET_ROUNDTRIP_MS - 5.5).abs() < 1e-12);
        assert_eq!(BASELINE_ID, "BASELINE-005");
        assert_eq!(BASELINE_NUMBER, "005");
    }

    /// 标签规范化：空白/`=`/非 ASCII 一律换成 `-`，空标签回落到 `default`。
    #[test]
    fn labels_are_sanitized_so_the_line_stays_parseable() {
        assert_eq!(sanitize_label("a b=c"), "a-b-c");
        assert_eq!(sanitize_label(""), "default");
        assert_eq!(sanitize_label("ok-1.2:3_x"), "ok-1.2:3_x");
        assert_eq!(sanitize_label("中文"), "--");
    }

    /// 判据：百分位的**钳位上界是 `100`** ⇒ `p = 100` 必须取到**最大**元素。
    ///
    /// 最近秩口径是 `rank = ceil(p/100 · n)`、下标 `max(rank, 1) − 1` 再钳到 `n − 1`
    /// （见 `percentile_nearest_rank` 的文档）。100 个升序样本时：
    /// `p = 100` ⇒ `rank = 100` ⇒ 下标 `99`（最大值）；`p = 0` ⇒ `rank = 1` ⇒ 下标 `0`。
    /// 上界若被写成 `99`，同一个调用会静默返回**第 99 个**元素 ——
    /// 报出来的"最差回调间隔"于是从来不是真的最差。
    ///
    /// **量什么**：`percentile_nearest_rank(&0..100, p)` 的返回值（单位：与输入同）。
    /// 末一条断言是**非空证明**：`p = 100` 与 `p = 99` 必须落在不同元素上。
    ///
    /// 注入实测：`percentile.clamp(0.0, 100.0)` → `clamp(0.0, 99.0)`（`latency.rs`）
    /// ⇒ 本判据实测变红（`left == Some(98.0)`）。
    #[test]
    fn percentile_one_hundred_is_the_maximum_and_zero_is_the_minimum() {
        let sorted: Vec<f64> = (0..100).map(|index| index as f64).collect();
        assert_eq!(
            percentile_nearest_rank(&sorted, 100.0),
            Some(99.0),
            "p = 100 必须取最大值"
        );
        assert_eq!(
            percentile_nearest_rank(&sorted, 0.0),
            Some(0.0),
            "p = 0 必须取最小值"
        );
        assert_ne!(
            percentile_nearest_rank(&sorted, 100.0),
            percentile_nearest_rank(&sorted, 99.0),
            "100 个样本时 p = 100 与 p = 99 必须落在不同元素上（非空证明）"
        );
    }

    // =======================================================================
    // `--loopback` 腿：判据（§3.1–§3.5）
    // =======================================================================

    /// 可复现的**伪噪声**（xorshift32；⛔ 不用随机数 ⇒ 判据确定性可复跑）。
    fn pseudo_noise(length: usize, seed: u32) -> Vec<f32> {
        let mut state = seed | 1;
        (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    /// FNV-1a 64 位指纹（仓内既有先例：`crates/yeban-app/tests/sfz_model_chain.rs`）。
    fn fnv1a_64(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// §3.5 的输入：一份字段尽量多的**默认腿**规范报告（逐位确定 ⇒ 指纹可复跑）。
    fn canonical_default_report() -> Report {
        Report {
            label: "fingerprint".to_owned(),
            requested_frames: 64,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices: vec![device(Direction::Output, 0), device(Direction::Input, 1)],
            nominals: vec![
                NominalReport {
                    direction: Direction::Output,
                    buffer_frames: 64,
                    sample_rate_hz: 48_000,
                    latency_ms: nominal_ms(64, 48_000).expect("合法"),
                },
                NominalReport {
                    direction: Direction::Input,
                    buffer_frames: 64,
                    sample_rate_hz: 48_000,
                    latency_ms: nominal_ms(64, 48_000).expect("合法"),
                },
            ],
            callbacks: vec![
                CallbackReport {
                    direction: Direction::Output,
                    callbacks: 1500,
                    backend_errors: 0,
                    stats: jitter_stats(&[1_000_000, 1_000_000, 1_002_000], 1_000_000),
                    driver_latency: distribution_ms(&[2_500_000, 2_500_000, 2_900_000]),
                },
                CallbackReport {
                    direction: Direction::Input,
                    callbacks: 1499,
                    backend_errors: 1,
                    stats: jitter_stats(&[1_000_000, 1_010_000], 1_000_000),
                    driver_latency: distribution_ms(&[1_000_000, 1_200_000]),
                },
            ],
            evidence: Evidence::NominalOnly,
            measured_roundtrip_ms: None,
            notes: vec!["a note".to_owned()],
        }
    }

    /// §3.1：注入已知延迟 ⇒ **逐帧精确**返回注入值（含 `D = 0` 与窗边界）。
    #[test]
    fn detect_delay_frames_recovers_the_injected_delay_exactly() {
        let reference = loopback_chirp(48_000, 8.0);
        let length = reference.len();
        assert!(length >= 256, "参考 chirp 太短：{length} 帧");
        let recorded_len = 2 * length + 50;
        // 窗边界 = 最后一个合法滞后（参考信号正好贴住录制窗的右沿）。
        let boundary = recorded_len - length;
        for delay in [0usize, length / 2, boundary] {
            let mut recorded = pseudo_noise(recorded_len, 0x1234_5678);
            for (index, value) in reference.iter().enumerate() {
                recorded[delay + index] += value;
            }
            let hit = detect_delay_frames(&reference, &recorded, LOOPBACK_PEAK_RATIO_THRESHOLD)
                .unwrap_or_else(|| panic!("注入延迟 {delay} 帧必须被检出"));
            assert_eq!(
                hit.delay_frames, delay,
                "注入延迟必须逐帧精确（delay={delay}）"
            );
            assert_eq!(hit.lags, recorded_len - length + 1);
            assert!(
                hit.peak_ratio > LOOPBACK_PEAK_RATIO_THRESHOLD,
                "峰值比要真的过阈值：{}",
                hit.peak_ratio
            );
        }
    }

    /// §3.2：**没有信号 ⇒ `None`**（负判据），且同一批数据里必须有一个**已知非空**的正对照。
    #[test]
    fn detect_delay_frames_rejects_pure_noise_zeros_and_constants() {
        let reference = loopback_chirp(48_000, 8.0);
        let length = reference.len();
        let recorded_len = length + 200;
        // 正对照（非空证明）：同一段伪噪声里埋一个 chirp ⇒ 必须检出。
        let mut planted = pseudo_noise(recorded_len, 0x0BAD_F00D);
        for (index, value) in reference.iter().enumerate() {
            planted[100 + index] += value;
        }
        let positive = detect_delay_frames(&reference, &planted, LOOPBACK_PEAK_RATIO_THRESHOLD)
            .expect("正对照必须检出（否则下面的 None 没有判别力）");
        assert_eq!(positive.delay_frames, 100);
        assert!(positive.peak_ratio > 1.0);
        // 负判据：纯噪声 / 全零 / 全常数 / 比参考还短。
        assert_eq!(
            detect_delay_frames(
                &reference,
                &pseudo_noise(recorded_len, 0xC0FF_EE01),
                LOOPBACK_PEAK_RATIO_THRESHOLD
            ),
            None,
            "纯噪声不许被检出"
        );
        assert_eq!(
            detect_delay_frames(
                &reference,
                &vec![0.0_f32; recorded_len],
                LOOPBACK_PEAK_RATIO_THRESHOLD
            ),
            None,
            "全零不许被检出"
        );
        assert_eq!(
            detect_delay_frames(
                &reference,
                &vec![0.5_f32; recorded_len],
                LOOPBACK_PEAK_RATIO_THRESHOLD
            ),
            None,
            "全常数不许被检出"
        );
        assert_eq!(
            detect_delay_frames(&reference, &[1.0_f32; 4], LOOPBACK_PEAK_RATIO_THRESHOLD),
            None
        );
        assert_eq!(
            detect_delay_frames(&[], &[1.0_f32; 16], LOOPBACK_PEAK_RATIO_THRESHOLD),
            None
        );
    }

    /// §3.3（诚实闸）：`LoopbackMeasured` ＋ **没有数字** ⇒ 仍不可判定（⛔ 不许翻成达标）。
    #[test]
    fn loopback_evidence_without_a_measured_number_is_still_unmeasurable() {
        for claim in [
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
        ] {
            let verdict = verdict_for(2, Evidence::LoopbackMeasured, claim);
            assert_eq!(
                verdict,
                Verdict::UnmeasurableWithoutLoopback,
                "声称有回环但没有有限数字（claim={claim:?}）绝不许翻成达标"
            );
            assert!(!verdict.is_pass());
        }
        // 整条 `Report` 也走同一条规则（不是只有裸函数被钉住）。
        let report = Report {
            label: "loopback-claimed-no-number".to_owned(),
            requested_frames: 64,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices: vec![device(Direction::Output, 0), device(Direction::Input, 1)],
            nominals: Vec::new(),
            callbacks: Vec::new(),
            evidence: Evidence::LoopbackMeasured,
            measured_roundtrip_ms: None,
            notes: vec!["claimed loopback but carried no number".to_owned()],
        };
        assert_eq!(report.verdict(), Verdict::UnmeasurableWithoutLoopback);
        assert!(!report.verdict().is_pass());
        let line = report.bench_line();
        assert!(
            !line.contains("within-target"),
            "行里不许出现达标字样: {line}"
        );
        let fields = parse_bench_line(&line).expect("可解析");
        assert_eq!(fields.measured_roundtrip_ms, None);
        assert_eq!(fields.verdict, "unmeasurable-without-loopback");
        assert_ne!(
            exit_code_for(report.verdict(), false),
            0,
            "没有数字必须非零退出"
        );
    }

    /// §3.4：阈值边界**精确**（`≤` 含等号，不是 `<`）。
    #[test]
    fn the_roundtrip_boundary_at_the_target_is_inclusive() {
        assert_eq!(
            verdict_for(1, Evidence::LoopbackMeasured, Some(TARGET_ROUNDTRIP_MS)),
            Verdict::WithinTarget,
            "恰好 {TARGET_ROUNDTRIP_MS} ms 算达标（≤）"
        );
        // 最接近的**更大**可表示值（nextafter）也必须翻面。
        let above = f64::from_bits(TARGET_ROUNDTRIP_MS.to_bits() + 1);
        assert!(above > TARGET_ROUNDTRIP_MS);
        assert_eq!(
            verdict_for(1, Evidence::LoopbackMeasured, Some(above)),
            Verdict::OverTarget
        );
        assert_eq!(
            verdict_for(
                1,
                Evidence::LoopbackMeasured,
                Some(TARGET_ROUNDTRIP_MS + 1e-9)
            ),
            Verdict::OverTarget,
            "TARGET + ε 必须超目标"
        );
        let below = f64::from_bits(TARGET_ROUNDTRIP_MS.to_bits() - 1);
        assert_eq!(
            verdict_for(1, Evidence::LoopbackMeasured, Some(below)),
            Verdict::WithinTarget
        );
        assert!(verdict_for(1, Evidence::LoopbackMeasured, Some(TARGET_ROUNDTRIP_MS)).is_pass());
    }

    /// §3.5：默认腿（`--loopback` 关掉）的 `bench_line()` **字面**被指纹钉死。
    ///
    /// 指纹常量 = **改动前**（`main`：`29836135a51dac17db028fa04485b35898d8f807`）实测值。
    #[test]
    fn the_default_leg_bench_line_literal_is_frozen() {
        let line = canonical_default_report().bench_line();
        assert_eq!(
            fnv1a_64(line.as_bytes()),
            0x7707_0a2b_9447_c69a,
            "默认腿的机器可读行字面变了 —— `--loopback` 关掉时它必须逐字不变:\n{line}"
        );
        // 判别力（非空证明）：末尾多一个空格，指纹必须不同。
        assert_ne!(
            fnv1a_64(line.as_bytes()),
            fnv1a_64(format!("{line} ").as_bytes())
        );
    }
}
