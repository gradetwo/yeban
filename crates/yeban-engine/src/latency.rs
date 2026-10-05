//! `BASELINE-005`（音频硬件往返时延，目标 `≤ 5.5 ms`）的**机器**那一半：
//! 标称时延换算、回调调度抖动统计、机器可读行、以及**把"测不到"与"达标"分开**的判定。
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
//! | 回调调度抖动 `jitter_stats` | ✅ | **CPU 侧**回调到达间隔与标称周期的偏差。**不是**往返时延 |
//! | 声学 / DAC-ADC 往返 | ❌ | 需要**物理回环**（输出接输入）或厂商 API |
//!
//! cpal **0.18.2 不暴露任何硬件时延查询 API**（已按 crate 源码核对：`src/traits.rs` 的
//! `DeviceTrait` 只有 `buffer_size()` / `now()`，全 crate 里 `latency` 一词只出现在
//! 错误文案与文档注释里）。macOS 的 `kAudioDevicePropertyLatency` /
//! `kAudioStreamPropertyLatency`、WASAPI 的 `IAudioClient::GetStreamLatency`、
//! ALSA 的 `snd_pcm_delay` 都要另写 FFI（= 新依赖裁决，不在本工作线权限内）。
//!
//! ⇒ 因此 [`verdict_for`] 有一条**不可绕过**的规则：
//! **没有回环证据时，`verdict` 永远不可能是 [`Verdict::WithinTarget`]**，
//! 无论标称值多好看、无论有没有设备。这就是"宁可少宣称"的机械形式。
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
pub const NO_LOOPBACK_REASON: &str = "no loopback evidence: cpal 0.18.2 exposes no hardware \
roundtrip latency API, and this tool does not capture from a physical output->input loopback";

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
    /// **实测**往返时延：本工具在无回环时恒为 `None`（且**不许**被标称值填上）。
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
    /// 实测往返只出现在 `measured_roundtrip_ms=`，且在本工具的路径上恒为 `none`。
    /// `nominal_io_sum_ms` 是**标称输入 + 标称输出**的和，随行的
    /// `nominal_io_sum_is_roundtrip=false` 明确声明它**不是**往返时延。
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
loopback=false driver_reported_extra_latency_ms=unknown",
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
device-reported actual buffer frames after opening; callback arrival jitter (p50/p99/max) on this CPU\n",
        );
        out.push_str(
            "DOES-NOT-MEASURE: acoustic / DAC-ADC roundtrip. That needs a physical output->input \
loopback or a vendor API (CoreAudio kAudioDevicePropertyLatency, WASAPI \
IAudioClient::GetStreamLatency, ALSA snd_pcm_delay). cpal 0.18.2 exposes none of these.\n",
        );
        // 只有"有设备但没回环"才谈得上"没有回环证据"；没有设备时理由是 NO-DEVICE。
        if self.device_count() > 0 && self.evidence == Evidence::NominalOnly {
            out.push_str(&format!("NO-LOOPBACK: {NO_LOOPBACK_REASON}\n"));
        }
        if self.device_count() == 0 {
            out.push_str(&format!("NO-DEVICE: {NO_DEVICE_REASON}\n"));
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
                },
                CallbackReport {
                    direction: Direction::Input,
                    callbacks: 2,
                    backend_errors: 0,
                    stats: Some(normal),
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
        // 有了设备也**不能**判达标（没有回环证据）。
        assert!(!report.verdict().is_pass());
    }

    /// 判据⑤反面：解析器**有判别力**（坏行必须被拒），不是恒绿。
    #[test]
    fn the_parser_rejects_malformed_lines() {
        let good = "BENCH baseline=005 label=x verdict=no-device evidence=nominal-only devices=0 \
measured_roundtrip_ms=none target_ms=5.5000 nominal_out_ms=none nominal_in_ms=none \
nominal_io_sum_ms=none nominal_io_sum_is_roundtrip=false loopback=false \
jitter_out_p99_ms=none jitter_in_p99_ms=none";
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
}
