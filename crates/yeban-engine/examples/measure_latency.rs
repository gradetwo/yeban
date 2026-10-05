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
//! **不能测**（因此本工具**永远**不会宣布达标）：
//!
//! - **真正的声学 / DAC-ADC 往返时延**：`playback`/`capture` 是**主机自己的预测**，
//!   不是测出来的声学量。真正的往返需要**物理回环**（输出接输入，或
//!   BlackHole/Loopback 这类虚拟设备）并做采集比对。
//! - **cpal 0.18.2 没有显式的时延查询 API**（`DeviceTrait` 上没有 `buffer_size()`/`now()`；
//!   这两个在 `StreamTrait` 上）——
//!   但这**不等于**"拿不到硬件时延"：上面第 3 条就是证据。想直接调
//!   `kAudioDevicePropertyLatency` / `IAudioClient::GetStreamLatency` / `snd_pcm_delay`
//!   仍然需要新写 FFI（= **依赖图裁决**），只是本工具已经不必走那条路。
//! - ⚠ **本线第一版把上面这条说错过**（写成"cpal 一个都不暴露"），已在核对
//!   `cpal-0.18.2/src/` 源码后更正并留痕，见 `docs/ledger/audio-latency-notes.md` §1.1。
//!
//! ⇒ 所以本工具**没有** `roundtrip_ms` 这种"看起来达标的总数"字段：实测往返只出现在
//! `measured_roundtrip_ms=`，且在本工具的路径上恒为 `none`；驱动侧时延**故意不给合计**
//! （合计最像"达标总数"）。判定由 [`yeban_engine::latency::verdict_for`] 给出，
//! **没有回环证据时 `within-target` 不可达**。
//!
//! # 退出码（"没测到"必须区别于"达标"）
//!
//! | 码 | 含义 |
//! | :---: | :--- |
//! | 0 | **实测**达标（本工具不产生此码）；或 `--allow-unmeasurable` 把"没测到"映射过来 |
//! | 1 | **实测**超目标（本工具不产生此码）|
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
    use std::time::Duration;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{
        BufferSize, SampleFormat, StreamConfig, SupportedBufferSize, SupportedStreamConfigRange,
    };
    use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
    use yeban_engine::device::{EngineConfig, STREAM_OPEN_TIMEOUT, ShareMode, negotiate};
    use yeban_engine::graph::LatencyTable;
    use yeban_engine::latency::{
        CallbackReport, DRIVER_IN_LATENCY_SOURCE, DRIVER_OUT_LATENCY_SOURCE, DeviceReport,
        Direction, EXIT_TOOL_DISABLED, EXIT_USAGE, Evidence, JitterStats, NO_DEVICE_REASON,
        NominalReport, Report, TARGET_ROUNDTRIP_MS, distribution_ms, driver_latency_unreported,
        exit_code_for, jitter_stats, nominal_ms, parse_bench_line,
    };
    use yeban_engine::meter::meter_channel;
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
    }

    const USAGE: &str = "\
usage: measure_latency [options]

  --frames N            requested buffer frames (default 64 = the spec's buffer)
  --seconds S           how long each stream runs (default 2)
  --sample-rate HZ      requested sample rate (default 48000)
  --label NAME          reading label, goes into the BENCH line (default \"default\")
  --force-no-device     report the no-device path as if this host had no audio device
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
        errors: AtomicU64,
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
                errors: AtomicU64::new(0),
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

        fn note_error(&self) {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }

        fn invocations(&self) -> usize {
            self.count.load(Ordering::Relaxed)
        }

        fn errors(&self) -> u64 {
            self.errors.load(Ordering::Relaxed)
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
        backend_errors: u64,
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
        (
            nominal,
            CallbackReport {
                direction,
                callbacks: run.invocations,
                backend_errors: run.backend_errors,
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
                        move |_error: cpal::Error| error_clock.note_error()
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
                        move |_error: cpal::Error| error_clock.note_error()
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
        backend_errors: u64,
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
            return emit(&report, options.allow_unmeasurable);
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
            return emit(&report, options.allow_unmeasurable);
        }

        let config = wanted(&options);
        let mut nominals = Vec::new();
        let mut callbacks = Vec::new();

        // ---- 输出流 ----
        match host.default_output_device() {
            Some(device) => {
                let name = device.to_string();
                match run_output(&device, &config, options.seconds) {
                    Ok(run) => {
                        mark_opened(&mut devices, Direction::Output, &name, run.reported_frames);
                        let (nominal, callback) =
                            summarize(Direction::Output, &run, options.frames, &mut notes);
                        nominals.extend(nominal);
                        callbacks.push(callback);
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
                    }
                    Err(message) => notes.push(format!(
                        "input stream NOT measured (this is NOT a pass): {message}"
                    )),
                }
            }
            None => notes.push("no default input device (this is NOT a pass)".to_owned()),
        }

        notes.push(
            "verdict is unmeasurable-without-loopback: the DRIVER-LATENCY rows are the host's own \
playback/capture prediction (not an acoustic roundtrip), cpal 0.18.2 has no explicit hardware \
latency query API, and no physical output->input loopback was used"
                .to_owned(),
        );

        let report = Report {
            label: options.label,
            requested_frames: options.frames,
            target_ms: TARGET_ROUNDTRIP_MS,
            devices,
            nominals,
            callbacks,
            evidence: Evidence::NominalOnly,
            measured_roundtrip_ms: None,
            notes,
        };
        emit(&report, options.allow_unmeasurable)
    }

    /// 打印人读摘要 + 机器可读行，并按判定给退出码。
    fn emit(report: &Report, allow_unmeasurable: bool) -> ExitCode {
        println!("{}", report.human_summary());
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
