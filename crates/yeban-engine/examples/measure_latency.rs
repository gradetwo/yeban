//! `BASELINE-005`（音频硬件往返时延，目标 `≤ 5.5 ms`）的**测量机器**。
//!
//! 规范（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.2）：
//! 「以 64 采样点缓冲运行于 cpal 驱动下，通过底层平台原生系统 API 实测回路延迟：
//! macOS CoreAudio 查询 `kAudioDevicePropertyLatency` 与 `kAudioStreamPropertyLatency`；
//! Windows WASAPI 查询 `IAudioClient::GetStreamLatency`；Linux PipeWire/JACK 借助硬件回环测试」
//! —— 目标 `≤ 5.5 ms`。
//!
//! # 这个工具**能**做什么、**不能**做什么（必须先读）
//!
//! **能测**（本工具真的输出数字）：
//!
//! 1. **标称时延**：设备报告的缓冲帧数 ÷ 采样率，输入/输出**分别**给（`NOMINAL` 行）；
//! 2. **后端报告的实际缓冲帧数**：打开流之后从**流**上回读 `StreamTrait::buffer_size()`；
//! 3. **主机报告的驱动侧时延**（`DRIVER-LATENCY` 行）：输出 `playback − callback`、
//!    输入 `callback − capture`，给 p50 / p99 / max / mean。
//!    这是**真的**：cpal 每个回调都带着主机用厂商 API 算出来的时刻 ——
//!    CoreAudio 折入 `kAudioDevicePropertyLatency` + `kAudioDevicePropertySafetyOffset`
//!    （见 `cpal-0.18.2/src/host/coreaudio/macos/device.rs` 的
//!    `get_device_extra_latency_frames`），WASAPI 用
//!    `buffered_frames + IAudioClient::GetStreamLatency`，ALSA 用 PCM delay，
//!    PulseAudio 用自己的时延估计（**是估计值**）。
//! 4. **回调调度抖动**（**CPU 侧**）：真的跑 2 秒输出流 + 输入流，统计每次回调到达间隔
//!    与标称周期的偏差的 p50 / p99 / max / mean（`CALLBACK` 行）。
//!
//! **默认腿不能测**（因此不带 `--loopback` 时本工具**永远**不会宣布达标）：
//!
//! - **真正的声学 / DAC-ADC 往返时延**：`playback`/`capture` 是**主机自己的预测**，
//!   不是测出来的声学量。默认腿没有**物理回环**（输出接输入，或
//!   BlackHole/Loopback 这类虚拟设备）与**采集比对**，所以它拿不到往返证据。
//! - **cpal 0.18.2 没有显式的时延查询 API**（`DeviceTrait` 上没有 `buffer_size()`/`now()`；
//!   这两个在 `StreamTrait` 上）——
//!   但这**不等于**"拿不到硬件时延"：上面第 3 条就是证据。想直接调
//!   `kAudioDevicePropertyLatency` / `IAudioClient::GetStreamLatency` / `snd_pcm_delay`
//!   仍然需要新写 FFI（= **依赖图裁决**），只是本工具已经不必走那条路。
//! - ⚠ **本线第一版把上面这条说错过**（写成"cpal 一个都不暴露"），已在核对
//!   `cpal-0.18.2/src/` 源码后更正并留痕，见 `docs/ledger/audio-latency-notes.md` §1.1。
//!
//! **`--loopback` 腿能测**（默认**关**；只有显式加 `--loopback` 才走）：
//!
//! 1. 同时在**默认输出**与**默认输入**上各开一条 cpal 流（本文件自己
//!    `build_output_stream` / `build_input_stream`，与 `run_output`/`run_input` 同款）；
//! 2. 往输出回调写一条**线性 chirp**（200 Hz → 8 kHz、约 50 ms；参数见
//!    [`yeban_engine::latency::LOOPBACK_CHIRP_MS`] 一族常量）；
//! 3. 把输入采集攒起来，用**归一化互相关**
//!    （[`yeban_engine::latency::detect_delay_frames`]）找 chirp 的滞后；
//! 4. 往返 = **输出回调首次送出 chirp 的 `Instant`** 与 **输入回调首次检出该 chirp 的
//!    `Instant`** 之差（两侧同进程、同一个 `Instant` 时基 ⇒ 不需要共享流帧计数器）。
//!
//! ⚠ **这个数字的诚实口径**（⛔ 不许读成"真硬件往返"）：
//! 它从"输出回调被**调用**"算起（**不是**样本离开 DAC 的时刻），到"载着 chirp 的
//! 输入回调被**调用**"为止 ⇒ 它**含驱动缓冲、不含 DAC／ADC 的模拟段**。
//! **虚拟回环（BlackHole/Loopback）没有 AD／DA** ⇒ 那是真硬件往返的**下界**，
//! ⛔ **不等于**真硬件往返。要判 `BASELINE-005` 是否达标，仍需人类按 ADR 裁决口径
//! （"原生 API 口径"是否算数）。
//!
//! 检出的**不确定性**（峰值比、主瓣外次高峰、滞后帧数、回调量化）都写进
//! `LOOPBACK:` / `NOTE:` 行与 stderr；**没检出**时本工具**不给数**，并**保持**
//! `unmeasurable-without-loopback` + 非零退出（诚实闸，见正文）。
//!
//! ⇒ 所以本工具**没有** `roundtrip_ms` 这种"看起来达标的总数"字段：实测往返只出现在
//! `measured_roundtrip_ms=`，默认腿上恒为 `none`（只有 `--loopback` 腿**真的检出**才是一个数）；
//! 驱动侧时延**故意不给合计**（合计最像"达标总数"）。判定由
//! [`yeban_engine::latency::verdict_for`] 给出，**没有回环证据时 `within-target` 不可达**。
//!
//! # 退出码（"没测到"必须区别于"达标"）
//!
//! | 码 | 含义 |
//! | :---: | :--- |
//! | 0 | **实测**达标：只有 `--loopback` 腿**真的检出** chirp 且 `≤ 5.5 ms` 才产生；或 `--allow-unmeasurable` 把"没测到"映射过来 |
//! | 1 | **实测**超目标：只有 `--loopback` 腿**真的检出** chirp 且 `> 5.5 ms` 才产生 |
//! | 2 | 命令行用法错误 |
//! | 3 | 本环境**没有音频设备**（托管 CI runner 的常态）—— **不是**达标，**不是** 0 ms |
//! | 4 | 有设备但**没有回环证据** ⇒ 往返时延不可判定 —— **不是**达标 |
//! | 5 | 工具无法给出可用读数（编译时关掉 `device` feature，或自检发现机器可读行坏了）|
//!
//! `--allow-unmeasurable` 只把 3/4 映射成 0，好让 CI 的日志步骤不红；
//! 机器可读行里的 `verdict=` **一个字节都不会变**。
//!
//! # 用法
//!
//! ```text
//! cargo run --release -p yeban-engine --example measure_latency -- \
//!     --label m2-builtin --frames 64 --seconds 2 --sample-rate 48000
//! ```
//!
//! 详见 `docs/ledger/audio-latency-notes.md`（能力矩阵、CI 上会发生什么、要在什么机器上
//! 跑哪条命令才**可能**判达标）。

#[cfg(feature = "device")]
fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    imp::run(&args)
}

/// 编译时关掉了 `device` feature（`--no-default-features` 的轻量变体）。
///
/// 本机（Apple M2，按纪律不编译 cpal）走的就是这条路；CI 走的是上面的真实路径。
/// **不许**在这里假装成功、也不许静默 —— 明确报未启用并以 [`EXIT_TOOL_DISABLED`] 退出。
#[cfg(not(feature = "device"))]
fn main() -> std::process::ExitCode {
    eprintln!("measure_latency: built WITHOUT the `device` feature (cpal is not compiled in).");
    eprintln!(
        "  This is the expected lightweight variant on a developer machine \
         (docs/DEV_WORKFLOW.md: cpal is compiled only in CI)."
    );
    eprintln!(
        "  Nothing was measured. Real measurement requires \
         `cargo run -p yeban-engine --example measure_latency` (default features) \
         on a machine with an audio device."
    );
    std::process::ExitCode::from(yeban_engine::latency::EXIT_TOOL_DISABLED)
}

#[cfg(feature = "device")]
mod imp {
    use std::collections::BTreeMap;
    use std::process::ExitCode;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{
        BufferSize, SampleFormat, StreamConfig, SupportedBufferSize, SupportedStreamConfigRange,
    };
    use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
    use yeban_engine::device::{
        BackendErrorCounts, BackendErrorLog, EngineConfig, STREAM_OPEN_TIMEOUT, ShareMode,
        capture_callback, negotiate, negotiate_input,
    };
    use yeban_engine::graph::LatencyTable;
    use yeban_engine::latency::{
        CallbackReport, DRIVER_IN_LATENCY_SOURCE, DRIVER_OUT_LATENCY_SOURCE, DeviceReport,
        Direction, EXIT_TOOL_DISABLED, EXIT_USAGE, Evidence, JitterStats, LOOPBACK_CHIRP_MS,
        LOOPBACK_PEAK_RATIO_THRESHOLD, NO_DEVICE_REASON, NominalReport, Report,
        TARGET_ROUNDTRIP_MS, detect_delay_frames, distribution_ms, driver_latency_unreported,
        exit_code_for, jitter_stats, loopback_chirp, nominal_ms, parse_bench_line,
    };
    use yeban_engine::meter::meter_channel;
    use yeban_engine::record::record_channel;
    use yeban_engine::ring::event_channel;
    use yeban_engine::rt::EngineRuntime;
    use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
    use yeban_model::{EntityId, RoutingGraph};

    /// 每个流最多记录多少次回调间隔（**预分配**，回调内零分配 [红线 7]）。
    ///
    /// 2 秒 @64 帧/48 kHz ≈ 1500 次回调 ⇒ 4096 足够；溢出时**丢弃**而不是扩容，
    /// 并在 `NOTE` 里如实说明（丢弃会让抖动统计偏乐观，必须说出来）。
    const MAX_CALLBACKS: usize = 4096;

    /// 命令行选项。
    struct Options {
        frames: u32,
        seconds: f64,
        sample_rate: u32,
        label: String,
        force_no_device: bool,
        allow_unmeasurable: bool,
        /// `--loopback`：是否走**回环腿**（默认**关** ⇒ 默认腿行为一位不变）。
        loopback: bool,
    }

    const USAGE: &str = "\
usage: measure_latency [options]

  --frames N            requested buffer frames (default 64 = the spec's buffer)
  --seconds S           how long each stream runs (default 2)
  --sample-rate HZ      requested sample rate (default 48000)
  --label NAME          reading label, goes into the BENCH line (default \"default\")
  --force-no-device     report the no-device path as if this host had no audio device
  --loopback            ALSO open the default output+input streams SIMULTANEOUSLY, play a
                        linear chirp (200 Hz -> 8 kHz, ~50 ms) on the output and matched-filter
                        (normalized cross-correlation) the input capture. A roundtrip number is
                        reported ONLY if the chirp is really detected; it is the host-observed
                        callback-to-callback time (includes driver buffers, EXCLUDES the DAC/ADC
                        analog sections). On a virtual loopback (BlackHole) there is no AD/DA, so
                        that figure is a LOWER BOUND, not a real hardware roundtrip. If the chirp
                        is not detected the reading stays unmeasurable and exits non-zero.
  --allow-unmeasurable  map exit codes 3 (no-device) and 4 (no loopback) to 0; the BENCH
                        line keeps verdict=no-device / unmeasurable-without-loopback
                        and NEVER becomes within-target
  -h, --help            print this help and exit 0

exit codes: 0 within-target (measured) | 1 over-target (measured) | 2 usage
            3 no-device (nothing measured, NOT a pass) | 4 unmeasurable-without-loopback
            5 tool could not produce a usable reading";

    fn value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
        args.get(index)
            .map(String::as_str)
            .ok_or_else(|| format!("{flag} needs a value (try --help)"))
    }

    fn parse_args(args: &[String]) -> Result<Options, String> {
        let mut options = Options {
            frames: 64,
            seconds: 2.0,
            sample_rate: 48_000,
            label: "default".to_owned(),
            force_no_device: false,
            allow_unmeasurable: false,
            loopback: false,
        };
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--frames" => {
                    index += 1;
                    options.frames = value(args, index, "--frames")?
                        .parse()
                        .map_err(|error| format!("--frames: {error}"))?;
                }
                "--seconds" => {
                    index += 1;
                    options.seconds = value(args, index, "--seconds")?
                        .parse()
                        .map_err(|error| format!("--seconds: {error}"))?;
                }
                "--sample-rate" => {
                    index += 1;
                    options.sample_rate = value(args, index, "--sample-rate")?
                        .parse()
                        .map_err(|error| format!("--sample-rate: {error}"))?;
                }
                "--label" => {
                    index += 1;
                    options.label = value(args, index, "--label")?.to_owned();
                }
                "--force-no-device" => options.force_no_device = true,
                "--allow-unmeasurable" => options.allow_unmeasurable = true,
                "--loopback" => options.loopback = true,
                other => return Err(format!("unknown argument {other:?} (try --help)")),
            }
            index += 1;
        }
        if options.frames == 0 {
            return Err("--frames must be > 0".to_owned());
        }
        if options.sample_rate == 0 {
            return Err("--sample-rate must be > 0".to_owned());
        }
        if !(options.seconds.is_finite() && options.seconds > 0.0) {
            return Err("--seconds must be a positive finite number".to_owned());
        }
        Ok(options)
    }

    /// 预分配的回调时钟：回调内**零分配 / 零锁 / 零阻塞 I/O** [AGENTS.md §2 红线 7]。
    ///
    /// 这不是引擎的实时回调，但它跑在**同一个** cpal 音频线程上 —— 在这里分配或加锁会自己
    /// 制造出被测的抖动（甚至 XRun），所以测量工具也必须守同一条纪律。
    struct CallbackClock {
        last_ns: AtomicU64,
        seen: AtomicBool,
        count: AtomicUsize,
        write_index: AtomicUsize,
        intervals_ns: Box<[AtomicU64]>,
        driver_index: AtomicUsize,
        driver_ns: Box<[AtomicU64]>,
        errors: BackendErrorLog,
    }

    impl CallbackClock {
        fn new() -> Arc<Self> {
            let intervals: Vec<AtomicU64> = (0..MAX_CALLBACKS).map(|_| AtomicU64::new(0)).collect();
            let driver: Vec<AtomicU64> = (0..MAX_CALLBACKS).map(|_| AtomicU64::new(0)).collect();
            Arc::new(Self {
                last_ns: AtomicU64::new(0),
                seen: AtomicBool::new(false),
                count: AtomicUsize::new(0),
                write_index: AtomicUsize::new(0),
                intervals_ns: intervals.into_boxed_slice(),
                driver_index: AtomicUsize::new(0),
                driver_ns: driver.into_boxed_slice(),
                errors: BackendErrorLog::new(),
            })
        }

        /// 记一次回调（实时路径）。
        fn record(&self, instant: cpal::StreamInstant) {
            self.count.fetch_add(1, Ordering::Relaxed);
            let now = u64::try_from(instant.as_nanos()).unwrap_or(u64::MAX);
            if self.seen.swap(true, Ordering::Relaxed) {
                let last = self.last_ns.load(Ordering::Relaxed);
                let interval = now.saturating_sub(last);
                let index = self.write_index.load(Ordering::Relaxed);
                if index < self.intervals_ns.len() {
                    self.intervals_ns[index].store(interval, Ordering::Relaxed);
                    self.write_index.store(index + 1, Ordering::Relaxed);
                }
            }
            self.last_ns.store(now, Ordering::Relaxed);
        }

        /// 记一次**主机报告的驱动侧时延**（实时路径，纳秒）。
        ///
        /// 输出方向传 `playback − callback`，输入方向传 `callback − capture`
        /// （都用 `as_nanos()` 的饱和减法预先算好）。
        fn record_driver_latency(&self, delta_ns: u64) {
            let index = self.driver_index.load(Ordering::Relaxed);
            if index < self.driver_ns.len() {
                self.driver_ns[index].store(delta_ns, Ordering::Relaxed);
                self.driver_index.store(index + 1, Ordering::Relaxed);
            }
        }

        /// 非实时线程上的抽干。
        fn drain(&self) -> Vec<u64> {
            let count = self
                .write_index
                .load(Ordering::Relaxed)
                .min(self.intervals_ns.len());
            self.intervals_ns[..count]
                .iter()
                .map(|slot| slot.load(Ordering::Relaxed))
                .collect()
        }

        /// 驱动侧时延样本的抽干。
        fn drain_driver(&self) -> Vec<u64> {
            let count = self
                .driver_index
                .load(Ordering::Relaxed)
                .min(self.driver_ns.len());
            self.driver_ns[..count]
                .iter()
                .map(|slot| slot.load(Ordering::Relaxed))
                .collect()
        }

        /// 错误回调内调用：按当前阶段计数，首条错误记下种类与消息（零分配 [红线 7]）。
        fn note_error(&self, error: &cpal::Error) {
            self.errors.record(error);
        }

        /// 从这一时刻起到达的错误计入**关闭/停止期**（在 `pause()` 之前调用）。
        fn begin_stop(&self) {
            self.errors.begin_stop();
        }

        fn invocations(&self) -> usize {
            self.count.load(Ordering::Relaxed)
        }

        fn errors(&self) -> BackendErrorCounts {
            self.errors.counts()
        }
    }

    /// `later − earlier`，单位纳秒，**饱和**（时钟不单调时给 0，不 panic）。
    fn delta_ns(later: cpal::StreamInstant, earlier: cpal::StreamInstant) -> u64 {
        u64::try_from(later.as_nanos().saturating_sub(earlier.as_nanos())).unwrap_or(u64::MAX)
    }

    /// 一个方向跑完流之后的原始读数（统计留到 [`summarize`] 里算，避免两处逻辑）。
    struct StreamRun {
        invocations: usize,
        intervals_ns: Vec<u64>,
        driver_ns: Vec<u64>,
        backend_errors: BackendErrorCounts,
        sample_rate_hz: u32,
        negotiated_frames: Option<u32>,
        reported_frames: Option<u32>,
        intervals_truncated: bool,
    }

    /// 通道的**另一端**与快照锚：测量期间必须存活（尤其是 `Arc<SnapshotSlot>` ——
    /// 实时侧的 `SnapshotReader` 持有指向它的裸指针，它先落地就是 use-after-free）。
    ///
    /// 字段名带 `_` 前缀 + `allow(dead_code)`：这些字段**故意**只被"持有"而不被读 ——
    /// 它们的生命周期本身就是被测对象的一部分。
    #[allow(dead_code)]
    struct EngineKeepAlive {
        _retire_queue: yeban_engine::snapshot::RetireQueue,
        _event_sender: yeban_engine::ring::EventSender,
        _meter_collector: yeban_engine::meter::MeterCollector,
        _slot: Arc<SnapshotSlot>,
    }

    /// 组装输出流的引擎运行期（与 `crates/yeban-engine/src/device.rs` 的测试用**同一套**公共 API）。
    ///
    /// 通道与快照槽都必须在**打开设备之前**建立 [红线 7]。
    fn build_runtime(
        sample_rate: u32,
        channels: u16,
    ) -> Result<(EngineRuntime, EngineKeepAlive), String> {
        let master = EntityId::new();
        let mut routing = RoutingGraph::default();
        routing.nodes.push(master);
        let snapshot = EngineSnapshot::from_parts(
            0,
            sample_rate,
            DEFAULT_BLOCK_FRAMES,
            channels,
            master,
            BTreeMap::new(),
            &routing,
            &LatencyTable::new(),
        )
        .map_err(|error| format!("engine snapshot build failed: {error}"))?;
        let slot = SnapshotSlot::new(snapshot);
        let (retire_producer, retire_queue) = retire_channel(8);
        let (event_sender, event_receiver) = event_channel(16);
        let (meter_publisher, meter_collector) = meter_channel(64);
        let runtime = EngineRuntime::new(&slot, retire_producer, event_receiver, meter_publisher);
        let keep_alive = EngineKeepAlive {
            _retire_queue: retire_queue,
            _event_sender: event_sender,
            _meter_collector: meter_collector,
            _slot: slot,
        };
        Ok((runtime, keep_alive))
    }

    /// 默认设备上取第一个 `f32` 能力区间（没有 `f32` 就退到第一个区间）。
    fn capability_range(
        device: &cpal::Device,
        direction: Direction,
    ) -> Option<SupportedStreamConfigRange> {
        let ranges: Vec<SupportedStreamConfigRange> = match direction {
            Direction::Output => device.supported_output_configs().ok()?.collect(),
            Direction::Input => device.supported_input_configs().ok()?.collect(),
        };
        ranges
            .iter()
            .find(|range| range.sample_format() == SampleFormat::F32)
            .or_else(|| ranges.first())
            .copied()
    }

    fn describe(device: &cpal::Device, direction: Direction, index: usize) -> DeviceReport {
        let description = device.description().ok();
        let name = description
            .as_ref()
            .map_or_else(|| device.to_string(), |desc| desc.name().to_owned());
        let driver = description
            .as_ref()
            .and_then(|desc| desc.driver().map(str::to_owned));
        let interface = description
            .as_ref()
            .map(|desc| desc.interface_type().to_string());

        let default = match direction {
            Direction::Output => device.default_output_config().ok(),
            Direction::Input => device.default_input_config().ok(),
        };
        let (buffer_min_frames, buffer_max_frames, buffer_range_unknown) =
            match capability_range(device, direction).map(|range| *range.buffer_size()) {
                Some(SupportedBufferSize::Range { min, max }) => (Some(min), Some(max), false),
                Some(SupportedBufferSize::Unknown) | None => (None, None, true),
            };

        DeviceReport {
            index,
            direction,
            name,
            driver,
            interface,
            default_channels: default.as_ref().map(|config| config.channels()),
            default_sample_rate: default.as_ref().map(|config| config.sample_rate()),
            default_sample_format: default
                .as_ref()
                .map(|config| format!("{:?}", config.sample_format())),
            buffer_min_frames,
            buffer_max_frames,
            buffer_range_unknown,
            // 实际缓冲帧数只有打开流之后才拿得到 ⇒ 由 `mark_opened` 回填。
            buffer_frames_reported: None,
        }
    }

    fn enumerate(host: &cpal::Host) -> (Vec<DeviceReport>, Vec<String>) {
        let mut reports = Vec::new();
        let mut notes = Vec::new();
        match host.output_devices() {
            Ok(devices) => {
                for device in devices {
                    let index = reports.len();
                    reports.push(describe(&device, Direction::Output, index));
                }
            }
            Err(error) => notes.push(format!("output_devices() failed: {error}")),
        }
        match host.input_devices() {
            Ok(devices) => {
                for device in devices {
                    let index = reports.len();
                    reports.push(describe(&device, Direction::Input, index));
                }
            }
            Err(error) => notes.push(format!("input_devices() failed: {error}")),
        }
        (reports, notes)
    }

    fn mark_opened(
        reports: &mut [DeviceReport],
        direction: Direction,
        name: &str,
        frames: Option<u32>,
    ) {
        for report in reports.iter_mut() {
            if report.direction == direction && report.name == name {
                report.buffer_frames_reported = frames;
            }
        }
    }

    fn negotiated_frames(buffer_size: BufferSize) -> Option<u32> {
        match buffer_size {
            BufferSize::Fixed(frames) => Some(frames),
            BufferSize::Default => None,
        }
    }

    fn nominal_period_ns(sample_rate_hz: u32, frames: u32) -> u64 {
        if sample_rate_hz == 0 {
            return 0;
        }
        (u128::from(frames) * 1_000_000_000 / u128::from(sample_rate_hz)) as u64
    }

    /// 把一个方向的原始读数变成"标称时延 + 回调抖动"，并把**每一个**不确定点写成 NOTE。
    ///
    /// 帧数的选择口径（**先看后端回读，再退到请求值，都没有就 `none`**）：
    /// 1. `StreamTrait::buffer_size()`（**开流之后从流上**）的回读 —— 后端报告的实际值，最可信；
    /// 2. 我们请求的 `BufferSize::Fixed(n)`（且协商接受）—— 请求值，不是回读值；
    /// 3. 都没有 ⇒ 标称 `none`、抖动周期用 `--frames`（并**明说**这是假设）。
    fn summarize(
        direction: Direction,
        run: &StreamRun,
        fallback_frames: u32,
        notes: &mut Vec<String>,
    ) -> (Option<NominalReport>, CallbackReport) {
        let frames = run
            .reported_frames
            .or(run.negotiated_frames)
            .unwrap_or(fallback_frames);
        if run.reported_frames.is_none() {
            notes.push(format!(
                "{}: the backend reported no buffer_size(); the nominal latency and the jitter \
period use {} (no guess beyond that)",
                direction.as_str(),
                match run.negotiated_frames {
                    Some(frames) => format!("the requested Fixed({frames})"),
                    None => format!("--frames {fallback_frames} as an ASSUMPTION"),
                },
            ));
        }
        if let (Some(requested), Some(reported)) = (run.negotiated_frames, run.reported_frames)
            && requested != reported
        {
            notes.push(format!(
                "{}: requested Fixed({requested}) but the driver reports {reported} frames; \
the reported value is used",
                direction.as_str()
            ));
        }
        // 后端既不报、协商也只给了 Default ⇒ 标称时延**没有事实来源**，宁可 none。
        let nominal = match (run.reported_frames, run.negotiated_frames) {
            (None, None) => None,
            _ => nominal_ms(frames, run.sample_rate_hz).map(|latency_ms| NominalReport {
                direction,
                buffer_frames: frames,
                sample_rate_hz: run.sample_rate_hz,
                latency_ms,
            }),
        };
        if nominal.is_none() {
            notes.push(format!(
                "{}: buffer frames unknown (BufferSize::Default and no backend buffer_size()) -> \
nominal=none (no guess)",
                direction.as_str()
            ));
        }
        if run.intervals_truncated {
            notes.push(format!(
                "{}: interval buffer filled up ({MAX_CALLBACKS}); the jitter figure is a LOWER bound",
                direction.as_str()
            ));
        }
        let stats: Option<JitterStats> = if run.intervals_ns.is_empty() {
            notes.push(format!(
                "{}: fewer than two callbacks arrived -> jitter unmeasured",
                direction.as_str()
            ));
            None
        } else {
            jitter_stats(
                &run.intervals_ns,
                nominal_period_ns(run.sample_rate_hz, frames),
            )
        };
        // 主机报告的驱动侧时延（输出 playback−callback / 输入 callback−capture）。
        let driver_latency = distribution_ms(&run.driver_ns);
        match driver_latency {
            Some(distribution) if driver_latency_unreported(&distribution) => notes.push(format!(
                "{}: the host reported {} == {} for every callback -> driver-side latency is \
UNREPORTED on this host; that is NOT 0 ms",
                direction.as_str(),
                match direction {
                    Direction::Input => "callback",
                    Direction::Output => "playback",
                },
                match direction {
                    Direction::Input => "capture",
                    Direction::Output => "callback",
                },
            )),
            Some(distribution) => notes.push(format!(
                "{}: driver-side latency is the HOST's own prediction ({}), p50={:.4} ms \
p99={:.4} ms max={:.4} ms; it is NOT an acoustic roundtrip and it is NOT summed with the other \
direction on purpose",
                direction.as_str(),
                match direction {
                    Direction::Input => DRIVER_IN_LATENCY_SOURCE,
                    Direction::Output => DRIVER_OUT_LATENCY_SOURCE,
                },
                distribution.p50_ms,
                distribution.p99_ms,
                distribution.max_ms,
            )),
            None => notes.push(format!(
                "{}: no callback carried a playback/capture instant -> driver-side latency unmeasured",
                direction.as_str()
            )),
        }
        // 后端错误的两档读数：**运行期** vs **关闭/停止期**（`#147`）。
        //
        // 旧口径把两者相加成一个 `backend_errors`，于是"每方向恰好 1 次"读不出
        // 是运行期真错误还是关流阶段的一次事件。机器可读行（`BENCH`）的总数**不变**；
        // 分档与首条错误的种类/消息走独立的 `BACKEND-ERRORS` 行与人读 NOTE。
        let counts = run.backend_errors;
        let first = match counts.first {
            Some(info) => format!(
                "first kind={} message={:?}",
                info.kind_name(),
                info.message()
            ),
            None => "first kind=none".to_owned(),
        };
        notes.push(format!(
            "{}: backend errors split by phase: runtime={} stop/close={} total={} ({first})",
            direction.as_str(),
            counts.runtime,
            counts.stop,
            counts.total(),
        ));
        (
            nominal,
            CallbackReport {
                direction,
                callbacks: run.invocations,
                backend_errors: counts.total(),
                stats,
                driver_latency,
            },
        )
    }

    fn wanted(options: &Options) -> EngineConfig {
        EngineConfig {
            sample_rate: options.sample_rate,
            block_frames: options.frames,
            channels: 2,
            share_mode: ShareMode::PreferExclusive,
        }
    }

    /// 跑一个输出流：真实 cpal 回调 + **引擎自己的** `process_quantum` 渲染路径。
    fn run_output(
        device: &cpal::Device,
        config: &EngineConfig,
        seconds: f64,
    ) -> Result<StreamRun, String> {
        let ranges: Vec<SupportedStreamConfigRange> = device
            .supported_output_configs()
            .map_err(|error| format!("supported_output_configs: {error}"))?
            .collect();
        let negotiated = negotiate(&ranges, config)
            .map_err(|error| format!("output negotiation failed: {error}"))?;
        let stream_config = StreamConfig {
            channels: negotiated.channels,
            sample_rate: negotiated.sample_rate,
            buffer_size: negotiated.buffer_size,
        };
        // 顺序用**作用域**表达，不靠 `drop()` 也不靠逆序落地推理：
        //   - `_keep_alive`（快照锚 + 通道另一端）在块**之前**绑定 ⇒ 块结束后仍然存活；
        //   - `stream` 在块内绑定 ⇒ 块结束时先结束回调线程；
        // 于是"实时侧还可能读快照"的那段时间里，快照锚一定活着（不会有 use-after-free）。
        // `runtime` 必须是 `mut`：它被移进 `FnMut` 回调，并在回调里被可变借用。
        let (mut runtime, _keep_alive) =
            build_runtime(negotiated.sample_rate, negotiated.channels)?;
        let clock = CallbackClock::new();
        let reported_frames;
        {
            let callback_clock = Arc::clone(&clock);
            let channels = negotiated.channels;
            let stream = device
                .build_output_stream::<f32, _, _>(
                    stream_config,
                    move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                        // 真实回调：先记时刻与**主机报的驱动侧时延**（零分配），
                        // 再走引擎自己的渲染量子路径。
                        let stamp = info.timestamp();
                        callback_clock.record(stamp.callback);
                        // `playback` = 主机预计"现在写入的数据被播放"的时刻
                        // （CoreAudio 折入了 kAudioDevicePropertyLatency + 安全偏移）。
                        callback_clock
                            .record_driver_latency(delta_ns(stamp.playback, stamp.callback));
                        runtime.process_quantum(data, channels);
                    },
                    {
                        let error_clock = Arc::clone(&clock);
                        move |error: cpal::Error| error_clock.note_error(&error)
                    },
                    Some(STREAM_OPEN_TIMEOUT),
                )
                .map_err(|error| format!("build_output_stream: {error}"))?;
            stream
                .play()
                .map_err(|error| format!("output play: {error}"))?;
            // ⚠ `buffer_size()` 挂在 **`StreamTrait`**（流）上，不是 `DeviceTrait`（设备）——
            // 必须**开流之后**从流上回读。CI 的第一次编译就是这么红掉的。
            reported_frames = stream.buffer_size().ok();
            std::thread::sleep(Duration::from_secs_f64(seconds));
            clock.begin_stop();
            let _ = stream.pause();
        }
        Ok(finish(
            clock.drain(),
            clock.drain_driver(),
            clock.invocations(),
            clock.errors(),
            negotiated,
            reported_frames,
        ))
    }

    /// 跑一个输入流：真实采集回调，只记时刻（零分配）。
    fn run_input(
        device: &cpal::Device,
        config: &EngineConfig,
        seconds: f64,
    ) -> Result<StreamRun, String> {
        let ranges: Vec<SupportedStreamConfigRange> = device
            .supported_input_configs()
            .map_err(|error| format!("supported_input_configs: {error}"))?
            .collect();
        let negotiated = negotiate(&ranges, config)
            .map_err(|error| format!("input negotiation failed: {error}"))?;
        let stream_config = StreamConfig {
            channels: negotiated.channels,
            sample_rate: negotiated.sample_rate,
            buffer_size: negotiated.buffer_size,
        };
        let clock = CallbackClock::new();
        let reported_frames;
        {
            let callback_clock = Arc::clone(&clock);
            let stream = device
                .build_input_stream::<f32, _, _>(
                    stream_config,
                    move |_data: &[f32], info: &cpal::InputCallbackInfo| {
                        let stamp = info.timestamp();
                        callback_clock.record(stamp.callback);
                        // `capture` = 主机认为"这批数据被 ADC 采到"的时刻
                        // ⇒ `callback − capture` 是输入侧的驱动时延。
                        callback_clock
                            .record_driver_latency(delta_ns(stamp.callback, stamp.capture));
                    },
                    {
                        let error_clock = Arc::clone(&clock);
                        move |error: cpal::Error| error_clock.note_error(&error)
                    },
                    Some(STREAM_OPEN_TIMEOUT),
                )
                .map_err(|error| format!("build_input_stream: {error}"))?;
            stream
                .play()
                .map_err(|error| format!("input play: {error}"))?;
            // ⚠ `buffer_size()` 挂在 **`StreamTrait`**（流）上，不是 `DeviceTrait`（设备）——
            // 必须**开流之后**从流上回读。CI 的第一次编译就是这么红掉的。
            reported_frames = stream.buffer_size().ok();
            std::thread::sleep(Duration::from_secs_f64(seconds));
            clock.begin_stop();
            let _ = stream.pause();
        }
        Ok(finish(
            clock.drain(),
            clock.drain_driver(),
            clock.invocations(),
            clock.errors(),
            negotiated,
            reported_frames,
        ))
    }

    fn finish(
        intervals_ns: Vec<u64>,
        driver_ns: Vec<u64>,
        invocations: usize,
        backend_errors: BackendErrorCounts,
        negotiated: yeban_engine::device::NegotiatedConfig,
        reported_frames: Option<u32>,
    ) -> StreamRun {
        let intervals_truncated = intervals_ns.len() >= MAX_CALLBACKS;
        StreamRun {
            invocations,
            intervals_ns,
            driver_ns,
            backend_errors,
            sample_rate_hz: negotiated.sample_rate,
            negotiated_frames: negotiated_frames(negotiated.buffer_size),
            reported_frames,
            intervals_truncated,
        }
    }

    // =======================================================================
    // `--loopback` 腿：同时开两条流 + 线性 chirp + 采集比对
    // =======================================================================

    /// 回环腿的**预热**时长（毫秒）：先让输入流跑起来，再送 chirp。
    const LOOPBACK_PREROLL_MS: f64 = 250.0;

    /// 回环腿最多采集多少秒（采集环容量上限；超出即截断 ⇒ 判为不可测）。
    const LOOPBACK_MAX_CAPTURE_SECONDS: f64 = 30.0;

    /// 回环腿的**共享锚**：控制侧建好，实时回调只写原子量 / 预分配槽
    /// ⇒ 回调内零分配、零锁、零阻塞 I/O [AGENTS.md §2 红线 7]。
    struct LoopbackAnchors {
        /// 两条流共用的时基原点（同一个 `Instant` 基准 ⇒ 两侧可比）。
        base: Instant,
        /// 输出回调**首次送出 chirp** 的时刻（相对 `base` 的纳秒；0 = 还没送过）。
        emit_ns: AtomicU64,
        /// 第 k 次输入回调的**首帧序号**。
        stamp_frames: Box<[AtomicU64]>,
        /// 第 k 次输入回调的时刻（相对 `base` 的纳秒）。
        stamp_ns: Box<[AtomicU64]>,
        /// 已写入多少个输入回调时间戳槽。
        stamp_count: AtomicUsize,
        /// 采集环因满而丢弃的**帧**数（> 0 ⇒ 录音不连续 ⇒ 判为不可测）。
        dropped_frames: AtomicU64,
    }

    impl LoopbackAnchors {
        fn new() -> Arc<Self> {
            let frames: Vec<AtomicU64> = (0..MAX_CALLBACKS).map(|_| AtomicU64::new(0)).collect();
            let stamps: Vec<AtomicU64> = (0..MAX_CALLBACKS).map(|_| AtomicU64::new(0)).collect();
            Arc::new(Self {
                base: Instant::now(),
                emit_ns: AtomicU64::new(0),
                stamp_frames: frames.into_boxed_slice(),
                stamp_ns: stamps.into_boxed_slice(),
                stamp_count: AtomicUsize::new(0),
                dropped_frames: AtomicU64::new(0),
            })
        }

        /// `base` 起算的纳秒（饱和到 `u64`；回调内只读时钟，不分配不加锁）。
        fn now_ns(&self) -> u64 {
            u64::try_from(self.base.elapsed().as_nanos()).unwrap_or(u64::MAX)
        }
    }

    /// 回环腿的读数：两条流的常规统计 ＋ 一个（可能的）实测往返。
    struct LoopbackRun {
        output_name: String,
        input_name: String,
        output: StreamRun,
        input: StreamRun,
        /// 真的检出、且用两个回调的 `Instant` 算出来的往返（毫秒）；没检出 ⇒ `None`。
        measurement: Option<f64>,
    }

    /// 把检出到的滞后帧映射成"载着它的那次输入回调"的时刻（相对 `base` 的纳秒）。
    ///
    /// 口径：`stamp_frames[k]` 是第 k 次回调的**首帧**序号 ⇒ 块内按采样率线性插值。
    /// 返回 `None` 当滞后不在**已记录的回调窗**内 —— 这正是诚实闸要求的
    /// "检出的延迟必须落在录制窗内"。
    fn input_instant_ns(
        anchors: &LoopbackAnchors,
        onset_frame: usize,
        recorded_frames: usize,
        sample_rate_hz: u32,
    ) -> Option<f64> {
        let count = anchors
            .stamp_count
            .load(Ordering::Relaxed)
            .min(anchors.stamp_frames.len());
        if count == 0 {
            return None;
        }
        let mut chosen: Option<usize> = None;
        for index in 0..count {
            let begin = anchors.stamp_frames[index].load(Ordering::Relaxed) as usize;
            if begin <= onset_frame {
                chosen = Some(index);
            } else {
                break;
            }
        }
        let index = chosen?;
        let begin = anchors.stamp_frames[index].load(Ordering::Relaxed) as usize;
        let end = if index + 1 < count {
            anchors.stamp_frames[index + 1].load(Ordering::Relaxed) as usize
        } else {
            recorded_frames
        };
        if onset_frame < begin || onset_frame >= end {
            return None;
        }
        let within_block = (onset_frame - begin) as f64 / f64::from(sample_rate_hz);
        Some(anchors.stamp_ns[index].load(Ordering::Relaxed) as f64 + within_block * 1e9)
    }

    /// 回环腿：**同时**开默认输出与默认输入两条流，输出送 chirp、输入采集比对。
    ///
    /// 两条流都由本文件自己 `build_*_stream`（与 [`run_output`]/[`run_input`] 同款），
    /// 于是时间锚完全由我们掌握。⛔ 只有**真的检出**才给往返数 —— 没检出时返回
    /// `Ok(.., None)` 并把"为什么没检出"写进 `notes` 与 stderr（诚实闸）。
    /// 任何"开不了流"都返回 `Err`（⛔ 不许静默当 0、不许假装达标）。
    fn run_loopback(
        options: &Options,
        config: &EngineConfig,
        notes: &mut Vec<String>,
    ) -> Result<LoopbackRun, String> {
        let host = cpal::default_host();
        let output_device = host
            .default_output_device()
            .ok_or_else(|| "no default output device".to_owned())?;
        let input_device = host
            .default_input_device()
            .ok_or_else(|| "no default input device".to_owned())?;
        let output_name = output_device.to_string();
        let input_name = input_device.to_string();

        let output_ranges: Vec<SupportedStreamConfigRange> = output_device
            .supported_output_configs()
            .map_err(|error| format!("supported_output_configs: {error}"))?
            .collect();
        let output_negotiated = negotiate(&output_ranges, config)
            .map_err(|error| format!("output negotiation failed: {error}"))?;
        let input_ranges: Vec<SupportedStreamConfigRange> = input_device
            .supported_input_configs()
            .map_err(|error| format!("supported_input_configs: {error}"))?
            .collect();
        let input_negotiated = negotiate_input(&input_ranges, config)
            .map_err(|error| format!("input negotiation failed: {error}"))?;

        // ⚠ 采样率不同 ⇒ 录回来的 chirp 被重采样过，参考信号对不上。
        // 宁可**响亮地不测**，也不要拿一条对不上的相关峰当真（诚实闸）。
        if input_negotiated.sample_rate != output_negotiated.sample_rate {
            return Err(format!(
                "input sample rate {} != output sample rate {}: the chirp would be resampled \
before capture and the matched filter would compare different signals, so no roundtrip is \
claimed",
                input_negotiated.sample_rate, output_negotiated.sample_rate
            ));
        }
        let sample_rate = output_negotiated.sample_rate;
        let rate = f64::from(sample_rate);
        let chirp = Arc::<[f32]>::from(loopback_chirp(sample_rate, LOOPBACK_CHIRP_MS));
        if chirp.is_empty() {
            return Err("chirp generation produced no samples".to_owned());
        }

        let output_channels = output_negotiated.channels;
        let input_channels = input_negotiated.channels;
        let preroll_frames = (rate * LOOPBACK_PREROLL_MS / 1000.0).round() as usize;

        // 采集环：容量按 --seconds 给足（另留一秒余量），并有硬上限。
        let wanted =
            ((options.seconds * rate).ceil() as usize).saturating_add(sample_rate as usize);
        let capacity = wanted
            .min((LOOPBACK_MAX_CAPTURE_SECONDS * rate) as usize)
            .max(1);
        let (sender, mut receiver) = record_channel(capacity, input_channels);
        let anchors = LoopbackAnchors::new();
        let output_clock = CallbackClock::new();
        let input_clock = CallbackClock::new();

        let reported_output;
        let reported_input;
        {
            // ---- 输出流：本文件自己建；回调只做静音 + 覆盖 chirp + 记两个时刻 ----
            let anchor = Arc::clone(&anchors);
            let signal = Arc::clone(&chirp);
            let clock = Arc::clone(&output_clock);
            let channels = usize::from(output_channels);
            let mut cursor: usize = 0;
            let mut emitted = false;
            let output_stream = output_device
                .build_output_stream::<f32, _, _>(
                    StreamConfig {
                        channels: output_channels,
                        sample_rate,
                        buffer_size: output_negotiated.buffer_size,
                    },
                    move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                        let stamp = info.timestamp();
                        clock.record(stamp.callback);
                        clock.record_driver_latency(delta_ns(stamp.playback, stamp.callback));
                        // 先整块静音（回调内零分配），再把 chirp 窗口覆盖上去。
                        for sample in data.iter_mut() {
                            *sample = 0.0;
                        }
                        let frames = data.len() / channels;
                        for index in 0..frames {
                            let global = cursor + index;
                            if global >= preroll_frames && global < preroll_frames + signal.len() {
                                let value = signal[global - preroll_frames];
                                for channel in 0..channels {
                                    data[index * channels + channel] = value;
                                }
                                if !emitted {
                                    emitted = true;
                                    anchor.emit_ns.store(anchor.now_ns(), Ordering::Relaxed);
                                }
                            }
                        }
                        cursor = cursor.saturating_add(frames);
                    },
                    {
                        let error_clock = Arc::clone(&output_clock);
                        move |error: cpal::Error| error_clock.note_error(&error)
                    },
                    Some(STREAM_OPEN_TIMEOUT),
                )
                .map_err(|error| format!("build_output_stream (loopback): {error}"))?;
            // ---- 输入流：回调只写时间戳 + 无锁推入采集环（零分配 / 零锁）----
            let anchor = Arc::clone(&anchors);
            let clock = Arc::clone(&input_clock);
            let mut sender = sender;
            let in_channels = usize::from(input_channels);
            let mut cursor: usize = 0;
            let input_stream = input_device
                .build_input_stream::<f32, _, _>(
                    StreamConfig {
                        channels: input_channels,
                        sample_rate,
                        buffer_size: input_negotiated.buffer_size,
                    },
                    move |data: &[f32], info: &cpal::InputCallbackInfo| {
                        let stamp = info.timestamp();
                        clock.record(stamp.callback);
                        clock.record_driver_latency(delta_ns(stamp.callback, stamp.capture));
                        let index = anchor.stamp_count.load(Ordering::Relaxed);
                        if index < anchor.stamp_frames.len() {
                            anchor.stamp_frames[index].store(cursor as u64, Ordering::Relaxed);
                            anchor.stamp_ns[index].store(anchor.now_ns(), Ordering::Relaxed);
                            anchor.stamp_count.store(index + 1, Ordering::Relaxed);
                        }
                        capture_callback(&mut sender, data, input_channels);
                        anchor
                            .dropped_frames
                            .store(sender.dropped_frames(), Ordering::Relaxed);
                        cursor = cursor.saturating_add(data.len() / in_channels);
                    },
                    {
                        let error_clock = Arc::clone(&input_clock);
                        move |error: cpal::Error| error_clock.note_error(&error)
                    },
                    Some(STREAM_OPEN_TIMEOUT),
                )
                .map_err(|error| format!("build_input_stream (loopback): {error}"))?;

            // ⚠ **先起输入、后起输出**：输入先采，chirp 才不会落在录制窗之前。
            input_stream
                .play()
                .map_err(|error| format!("input play (loopback): {error}"))?;
            output_stream
                .play()
                .map_err(|error| format!("output play (loopback): {error}"))?;
            reported_output = output_stream.buffer_size().ok();
            reported_input = input_stream.buffer_size().ok();
            std::thread::sleep(Duration::from_secs_f64(options.seconds));
            // 两条流都在**同一时刻**进入关闭/停止阶段：先切阶段、再暂停。
            output_clock.begin_stop();
            input_clock.begin_stop();
            let _ = output_stream.pause();
            let _ = input_stream.pause();
        }

        // ---- 采集侧抽干（控制线程，允许分配）----
        let mut mono: Vec<f32> = Vec::new();
        receiver.drain_frames(usize::MAX, |frame| {
            let sum: f32 = frame.iter().sum();
            mono.push(sum / frame.len() as f32);
        });
        let dropped = anchors.dropped_frames.load(Ordering::Relaxed);

        let measurement = if dropped > 0 {
            let reason = format!(
                "--loopback: the capture ring dropped {dropped} frame(s) (capacity too small) -> \
the recording is not contiguous, so no roundtrip is claimed"
            );
            eprintln!("measure_latency: {reason}");
            notes.push(reason);
            None
        } else if mono.is_empty() {
            let reason = "--loopback: the input stream captured no samples -> no roundtrip is \
claimed (this is NOT a pass and NOT 0 ms)"
                .to_owned();
            eprintln!("measure_latency: {reason}");
            notes.push(reason);
            None
        } else if let Some(hit) = detect_delay_frames(&chirp, &mono, LOOPBACK_PEAK_RATIO_THRESHOLD)
        {
            let emitted_ns = anchors.emit_ns.load(Ordering::Relaxed);
            if emitted_ns == 0 {
                let reason = "--loopback: a correlation peak was found but the chirp was never \
emitted by the output callback -> no roundtrip is claimed"
                    .to_owned();
                eprintln!("measure_latency: {reason}");
                notes.push(reason);
                None
            } else {
                match input_instant_ns(&anchors, hit.delay_frames, mono.len(), sample_rate) {
                    Some(input_ns) if input_ns >= emitted_ns as f64 => {
                        let roundtrip_ms = (input_ns - emitted_ns as f64) / 1e6;
                        if roundtrip_ms.is_finite() && roundtrip_ms >= 0.0 {
                            notes.push(format!(
                                "--loopback: chirp detected (delay_frames={delay} peak_ncc={ncc:.4} \
peak_ratio={ratio:.2} lags={lags}); roundtrip_ms={ms:.4} is the HOST-OBSERVED \
callback-to-callback time: it starts when the output callback that carried the chirp was invoked \
(NOT when the samples left the DAC) and ends when the input callback that carried the chirp was \
invoked; it therefore includes the driver buffers and EXCLUDES the DAC/ADC analog sections. On a \
virtual loopback (BlackHole/Loopback) there is no AD/DA at all, so this is a LOWER BOUND for a \
real hardware roundtrip, NOT equal to it",
                                delay = hit.delay_frames,
                                ncc = hit.peak_ncc,
                                ratio = hit.peak_ratio,
                                lags = hit.lags,
                                ms = roundtrip_ms,
                            ));
                            Some(roundtrip_ms)
                        } else {
                            let reason = format!(
                                "--loopback: the derived roundtrip is not a finite non-negative \
number ({roundtrip_ms}) -> no roundtrip is claimed"
                            );
                            eprintln!("measure_latency: {reason}");
                            notes.push(reason);
                            None
                        }
                    }
                    Some(_) => {
                        let reason =
                            "--loopback: the chirp was detected BEFORE the output callback \
emitted it (impossible timing) -> no roundtrip is claimed"
                                .to_owned();
                        eprintln!("measure_latency: {reason}");
                        notes.push(reason);
                        None
                    }
                    None => {
                        let reason = "--loopback: the detected delay does not fall inside the \
captured callback window -> no roundtrip is claimed"
                            .to_owned();
                        eprintln!("measure_latency: {reason}");
                        notes.push(reason);
                        None
                    }
                }
            }
        } else {
            // 诊断读数：把"最佳匹配长什么样"如实报出来（阈值 0 ⇒ 不收口），
            // 这**不**构成检出（判定仍看阈值）。
            let reading = detect_delay_frames(&chirp, &mono, 0.0).map_or_else(
                || {
                    "no correlation reading at all (capture all-zero or shorter than the reference)"
                        .to_owned()
                },
                |hit| {
                    format!(
                        "best peak_ratio={:.2} (threshold={LOOPBACK_PEAK_RATIO_THRESHOLD}), \
peak_ncc={:.4}, delay_frames={}, lags={}",
                        hit.peak_ratio, hit.peak_ncc, hit.delay_frames, hit.lags
                    )
                },
            );
            let reason = format!(
                "--loopback: NO chirp detected -> evidence stays nominal-only, \
measured_roundtrip_ms=none, verdict=unmeasurable-without-loopback ({reading})"
            );
            eprintln!("measure_latency: {reason}");
            notes.push(reason);
            None
        };

        let output_run = finish(
            output_clock.drain(),
            output_clock.drain_driver(),
            output_clock.invocations(),
            output_clock.errors(),
            output_negotiated,
            reported_output,
        );
        let input_run = finish(
            input_clock.drain(),
            input_clock.drain_driver(),
            input_clock.invocations(),
            input_clock.errors(),
            input_negotiated,
            reported_input,
        );
        Ok(LoopbackRun {
            output_name,
            input_name,
            output: output_run,
            input: input_run,
            measurement,
        })
    }

    /// 主流程。
    pub fn run(args: &[String]) -> ExitCode {
        if args.iter().any(|arg| arg == "--help" || arg == "-h") {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        let options = match parse_args(args) {
            Ok(options) => options,
            Err(message) => {
                eprintln!("measure_latency: {message}");
                eprintln!("{USAGE}");
                return ExitCode::from(EXIT_USAGE);
            }
        };

        println!(
            "measure_latency: BASELINE-005 tool (target <= {TARGET_ROUNDTRIP_MS} ms roundtrip)"
        );
        println!("host_os={}", std::env::consts::OS);

        if options.force_no_device {
            println!("--force-no-device: skipping enumeration and streams (deterministic seam)");
            let report = Report {
                label: options.label,
                requested_frames: options.frames,
                target_ms: TARGET_ROUNDTRIP_MS,
                devices: Vec::new(),
                nominals: Vec::new(),
                callbacks: Vec::new(),
                evidence: Evidence::NominalOnly,
                measured_roundtrip_ms: None,
                notes: vec![format!("forced no-device path: {NO_DEVICE_REASON}")],
            };
            return emit(&report, options.allow_unmeasurable, &[]);
        }

        let host = cpal::default_host();
        let (mut devices, mut notes) = enumerate(&host);
        if devices.is_empty() {
            notes.push(format!("{NO_DEVICE_REASON}; no stream was opened"));
            let report = Report {
                label: options.label,
                requested_frames: options.frames,
                target_ms: TARGET_ROUNDTRIP_MS,
                devices,
                nominals: Vec::new(),
                callbacks: Vec::new(),
                evidence: Evidence::NominalOnly,
                measured_roundtrip_ms: None,
                notes,
            };
            return emit(&report, options.allow_unmeasurable, &[]);
        }

        let config = wanted(&options);
        let mut nominals = Vec::new();
        let mut callbacks = Vec::new();
        // `#147`：错误分档的机器可读行（独立于 BENCH ⇒ 不改既有键序/行长）。
        let mut backend_lines: Vec<String> = Vec::new();

        // ---- 回环腿（可选）----
        // ⚠ `--loopback` **关**的时候，下面 `else` 分支里的默认腿流程与读数
        // **一位都不许变**（`latency.rs` 的指纹判据钉住了默认腿的 BENCH 行字面）。
        let mut evidence = Evidence::NominalOnly;
        let mut measured_roundtrip_ms: Option<f64> = None;
        if options.loopback {
            // 两条流**同时**开：输出送 chirp、输入采集比对（时间锚由本文件掌握）。
            match run_loopback(&options, &config, &mut notes) {
                Ok(run) => {
                    mark_opened(
                        &mut devices,
                        Direction::Output,
                        &run.output_name,
                        run.output.reported_frames,
                    );
                    mark_opened(
                        &mut devices,
                        Direction::Input,
                        &run.input_name,
                        run.input.reported_frames,
                    );
                    let (nominal, callback) =
                        summarize(Direction::Output, &run.output, options.frames, &mut notes);
                    nominals.extend(nominal);
                    callbacks.push(callback);
                    let (nominal, callback) =
                        summarize(Direction::Input, &run.input, options.frames, &mut notes);
                    nominals.extend(nominal);
                    callbacks.push(callback);
                    backend_lines.push(
                        run.output
                            .backend_errors
                            .machine_line(Direction::Output.as_str()),
                    );
                    backend_lines.push(
                        run.input
                            .backend_errors
                            .machine_line(Direction::Input.as_str()),
                    );
                    // 诚实闸：证据等级只在**真的检出**时才升级。
                    if let Some(milliseconds) = run.measurement {
                        evidence = Evidence::LoopbackMeasured;
                        measured_roundtrip_ms = Some(milliseconds);
                    }
                }
                Err(message) => notes.push(format!(
                    "--loopback leg NOT measured (this is NOT a pass): {message}"
                )),
            }
        } else {
            // ---- 输出流 ----
            match host.default_output_device() {
                Some(device) => {
                    let name = device.to_string();
                    match run_output(&device, &config, options.seconds) {
                        Ok(run) => {
                            mark_opened(
                                &mut devices,
                                Direction::Output,
                                &name,
                                run.reported_frames,
                            );
                            let (nominal, callback) =
                                summarize(Direction::Output, &run, options.frames, &mut notes);
                            nominals.extend(nominal);
                            callbacks.push(callback);
                            backend_lines
                                .push(run.backend_errors.machine_line(Direction::Output.as_str()));
                        }
                        Err(message) => notes.push(format!(
                            "output stream NOT measured (this is NOT a pass): {message}"
                        )),
                    }
                }
                None => notes.push("no default output device (this is NOT a pass)".to_owned()),
            }

            // ---- 输入流 ----
            match host.default_input_device() {
                Some(device) => {
                    let name = device.to_string();
                    match run_input(&device, &config, options.seconds) {
                        Ok(run) => {
                            mark_opened(&mut devices, Direction::Input, &name, run.reported_frames);
                            let (nominal, callback) =
                                summarize(Direction::Input, &run, options.frames, &mut notes);
                            nominals.extend(nominal);
                            callbacks.push(callback);
                            backend_lines
                                .push(run.backend_errors.machine_line(Direction::Input.as_str()));
                        }
                        Err(message) => notes.push(format!(
                            "input stream NOT measured (this is NOT a pass): {message}"
                        )),
                    }
                }
                None => notes.push("no default input device (this is NOT a pass)".to_owned()),
            }
        }

        // 只有**没拿到**回环证据时才出这条理由（拿到了就不该说"没有回环证据"）。
        if measured_roundtrip_ms.is_none() {
            notes.push(
                "verdict is unmeasurable-without-loopback: the DRIVER-LATENCY rows are the host's \
own playback/capture prediction (not an acoustic roundtrip), cpal 0.18.2 has no explicit hardware \
latency query API, and this invocation obtained NO loopback evidence (add `--loopback` for a real \
output->input capture comparison; if it was already added but did not detect the chirp, the reason \
is in the NOTE lines above)"
                    .to_owned(),
            );
        }

        let report = Report {
            label: options.label,
            requested_frames: options.frames,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices,
            nominals,
            callbacks,
            evidence,
            measured_roundtrip_ms,
            notes,
        };
        emit(&report, options.allow_unmeasurable, &backend_lines)
    }

    /// 打印人读摘要 + 机器可读行，并按判定给退出码。
    fn emit(report: &Report, allow_unmeasurable: bool, backend_lines: &[String]) -> ExitCode {
        println!("{}", report.human_summary());
        // `#147`：分档读数（每次运行都打印；没有打开流时为空）。
        for line in backend_lines {
            println!("{line}");
        }
        let line = report.bench_line();
        // 自检：机器可读行必须能被自己的严格解析器读回来（格式漂移会在这里露馅）。
        match parse_bench_line(&line) {
            Some(fields) => {
                debug_assert_eq!(fields.verdict, report.verdict().as_str());
                debug_assert_eq!(fields.devices, report.device_count());
            }
            None => {
                eprintln!(
                    "measure_latency: INTERNAL ERROR: emitted BENCH line does not parse: {line}"
                );
                return ExitCode::from(EXIT_TOOL_DISABLED);
            }
        }
        println!("{line}");
        let verdict = report.verdict();
        if !verdict.is_pass() {
            eprintln!(
                "measure_latency: verdict={} (NOT within-target)",
                verdict.as_str()
            );
        }
        ExitCode::from(exit_code_for(verdict, allow_unmeasurable))
    }
}
