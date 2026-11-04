//! P2 磁盘流式引擎：后台读盘线程＋逐声部无锁环＋起音预载。
//!
//! 位置：P1（`sfz::load_with` 按需装载）解决的是**离线渲染导出**——
//! 无实时约束，按需整文件读就够，正确完整第一。本模块解决的是
//! **预览播放与实时演奏**那一侧：起播要快、音频线程绝不能碰磁盘。
//!
//! ## 架构（三件套）
//!
//! - **磁盘线程**（[`StreamEngine`] 内一条）：唯一碰文件系统的地方。
//!   用与 P1 **同一条**整文件解码腿（`sfz::read_sample_file`，含 64 MiB
//!   单文件护栏）读盘，再按块喂给声部环。一次只抱一个文件在途
//!   （单线程顺序喂 ⇒ 在途解码内存恒为一个文件）。
//! - **逐声部无锁环**（[`SampleRing`]）：每个声部一个。生产者（磁盘线程）
//!   写、消费者（音频线程）读，只用原子操作交接：**无锁、无分配、无 IO**。
//!   消费者路径（[`StreamVoice::read_block`]）只有原子读写与切片拷贝，
//!   满足实时音频回调"不分配、不加锁、不阻塞 IO"那条红线。
//! - **起音预载**：声部触发后，磁盘线程先把前 `preload_frames` 帧喂进环
//!   并打上 `preloaded` 闩，[`StreamVoice::ready`] 才为真 ⇒ 调用方在
//!   `ready()` 为真后再开播，起音一到就有声，不用等整文件解码。
//!
//! ## 预载范围（场景三分的落法）
//!
//! - **预览**（[`preview_sources`]）：只预载**已用区域**的起音。
//!   判定与 P1 的 `load_with(wanted)` **同一套**（`DemandLoadFilter::wants`），
//!   ⇒ 预载文件数 / 帧数与 P1 的收据（`skippedRegions`、文件数、帧数）一致。
//! - **实时**（[`realtime_sources`]）：预载**整件乐器**的起音
//!   （实时演奏一般只涉及一个音源、无谱子可算）。
//!   含 `key=-1` 的一次性区域：整件乐器的起音就该全含，触发与否是播放侧的事。
//!
//! ## 内存故事（与 P1 对比）
//!
//! P1 的离线渲染把**全部已用文件**解码后抱在 `SampleStore` 里直到渲染结束。
//! P2 的稳态内存 = 每声部一个有界环（默认 16384 帧）＋ 磁盘线程在途的一个文件。
//! 文件数越多，P2 相对 P1 省得越多；代价是声部读到环空时出零（欠载计数见
//! [`VoiceStats::underrun_frames`]），⛔ 不是阻塞等盘。
//!
//! ## 本期边界（如实登记，⛔ 不多认）
//!
//! - 解码仍是**整文件腿**：分块解码（WAV 流式读、FLAC/OGG 流式解码）是 P3，
//!   不在本期。本期"流式"指的是**喂环分块**＋**消费侧永不碰盘**。
//! - 本期**不接引擎声部池**：[`StreamVoice`] 是播放侧可直接消费的声部流，
//!   未来的预览/实时播放器拿它当声部源；离线渲染仍走 P1 的 `load_with`。
//! - 循环窗口**不展开**（与 P1 层同口径）：声部播完一遍即结束，
//!   `is_finished` 为真后继续读只出零。
//! - 预算（`SampleLimits`）**不在本层执行**：预算仍由装载层（P1）管；
//!   本层只报数（预载文件数、帧数），调用方拿去对预算。
//! - 本模块**零新依赖**（只用 `std`）：`Cargo.toml` 是集成者专属文件，
//!   执行员不动；无锁环用 `AtomicU32` 存 `f32` 位模式，⛔ 不写 `unsafe`
//!   （本 crate 已 `#![forbid(unsafe_code)]`）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use yeban_sfz::instrument::Instrument;

use super::sfz::{DemandLoadFilter, read_sample_file, resolve_sample_path};

/// 磁盘线程喂环的块大小缺省值（帧）。
const DEFAULT_CHUNK_FRAMES: usize = 4096;
/// 声部环容量缺省值（帧；约 0.34 s @ 48 kHz）。
const DEFAULT_RING_FRAMES: usize = 16384;
/// 起音预载缺省值（帧；约 85 ms @ 48 kHz）。
const DEFAULT_PRELOAD_FRAMES: usize = 4096;
/// 环空时磁盘线程的退让间隔：只发生在"环满且消费侧停读"时，
///
/// 配合 `cancelled` / `shutdown` 检查 ⇒ 引擎 `Drop` 的 `join` 永不挂住。
const RING_FULL_BACKOFF: Duration = Duration::from_millis(1);

// ---------------------------------------------------------------------------
// ① 单生产者／单消费者无锁环
// ---------------------------------------------------------------------------

/// 单生产者／单消费者无锁环：样本以 `f32` 位模式存在 `AtomicU32` 槽里。
///
/// 协议（SPSC 的标准原子交接，⛔ 不写 `unsafe` 也能成立）：
///
/// - 生产者只写下标在 `[read_pos, read_pos + capacity)` 里的槽
///   （写之前用 `Acquire` 读 `read_pos` 算空闲 ⇒ 绝不覆盖未消费的槽）；
/// - 生产者先写槽、再用 `Release` 推 `write_pos` ⇒ 槽写入对消费者可见；
/// - 消费者只读下标 `< write_pos` 的槽（`Acquire` 读 `write_pos` 后再读槽）；
/// - 消费者读完后用 `Release` 推 `read_pos` ⇒ 生产者能安全复用这些槽。
///
/// 每样本一个原子 `u32` ⇒ 不存在撕裂读；`f32` 经 `to_bits`／`from_bits`
/// 逐位往返（含 `-0.0`、次正规数在内都逐位相同）。
///
/// ⚠ **一环一生产者、一环一消费者**：两个生产者或两个消费者同时碰
/// 同一个环会破坏游标归属（`write_pos`／`read_pos` 各只有一方能写）。
/// 本模块里生产者恒为磁盘线程、消费者恒为持有声部的那一方。
#[derive(Debug)]
pub struct SampleRing {
    slots: Box<[AtomicU32]>,
    /// 容量掩码（容量恒为 2 的幂 ⇒ 下标用 `& mask` 回绕）。
    mask: usize,
    write_pos: AtomicU64,
    read_pos: AtomicU64,
}

impl SampleRing {
    /// 建一个容量为 `frames`（向上取到 2 的幂，夹在 `[2, 2^30]`）的环。
    #[must_use]
    pub fn new(frames: usize) -> Self {
        let capacity = frames.clamp(2, 1 << 30).next_power_of_two();
        let mut slots = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            slots.push(AtomicU32::new(0));
        }
        Self {
            slots: slots.into_boxed_slice(),
            mask: capacity - 1,
            write_pos: AtomicU64::new(0),
            read_pos: AtomicU64::new(0),
        }
    }

    /// 容量（帧；构造时向上取到的 2 的幂）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// 环里可读的帧数（生产者已发布、消费者未读）。
    #[must_use]
    pub fn available(&self) -> usize {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Relaxed);
        write.wrapping_sub(read) as usize
    }

    /// 环里还能写的帧数（`capacity - available`，饱和）。
    #[must_use]
    pub fn free(&self) -> usize {
        self.capacity().saturating_sub(self.available())
    }

    /// 生产者写：写 `min(samples.len(), free())` 帧，返回实际写下的帧数。
    ///
    /// 写不下时返回部分数（⛔ 不阻塞、不覆盖未消费数据）；调用方以后再写。
    pub fn write(&self, samples: &[f32]) -> usize {
        let read = self.read_pos.load(Ordering::Acquire);
        let write = self.write_pos.load(Ordering::Relaxed);
        let in_use = write.wrapping_sub(read) as usize;
        let free = self.capacity().saturating_sub(in_use);
        let n = samples.len().min(free);
        for (i, &sample) in samples[..n].iter().enumerate() {
            let index = (write as usize + i) & self.mask;
            self.slots[index].store(sample.to_bits(), Ordering::Relaxed);
        }
        // `Release` 发布：槽写入先于 `write_pos` 可见。
        self.write_pos.store(write + n as u64, Ordering::Release);
        n
    }

    /// 消费者读：读 `min(out.len(), available())` 帧，剩余位置零，返回实读帧数。
    ///
    /// 环空时返回 0 且 `out` 全零（⛔ 不阻塞、不等盘）——欠载就是零，由调用方计数。
    pub fn read(&self, out: &mut [f32]) -> usize {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Relaxed);
        let available = write.wrapping_sub(read) as usize;
        let n = out.len().min(available);
        for (i, slot) in out[..n].iter_mut().enumerate() {
            let index = (read as usize + i) & self.mask;
            *slot = f32::from_bits(self.slots[index].load(Ordering::Relaxed));
        }
        for slot in &mut out[n..] {
            *slot = 0.0;
        }
        // `Release` 发布：读完的位置可供生产者复用。
        self.read_pos.store(read + n as u64, Ordering::Release);
        n
    }
}

// ---------------------------------------------------------------------------
// ② 声部流
// ---------------------------------------------------------------------------

/// 一次 `read_block` 的读数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceRead {
    /// 本次从环里读到的帧数（实声）。
    pub from_ring: usize,
    /// 本次因环空而补零的帧数（欠载）。
    pub underrun: usize,
    /// 读完后该声部是否已播完（`eof` 且已读帧数 ≥ 总帧数）。
    pub finished: bool,
}

/// 声部的累计读数（任意线程可读，只含原子量 ⇒ 读数本身不阻塞音频线程）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceStats {
    /// 累计从环里读到的帧数（实声）。
    pub frames_read: u64,
    /// 累计补零的帧数（欠载）。
    pub underrun_frames: u64,
    /// 当前环里可读的帧数。
    pub available: usize,
    /// 是否已播完。
    pub finished: bool,
    /// 磁盘线程是否读盘失败（文件不存在／超 64 MiB／解码失败 ⇒ 本声部恒静音）。
    pub failed: bool,
}

/// 声部的共享状态（磁盘线程与消费侧各持一份 `Arc`）。
#[derive(Debug)]
struct VoiceState {
    ring: SampleRing,
    preload_frames: usize,
    /// 起音闩：磁盘线程发布够 `preload_frames` 帧（或文件播完）后置真，⛔ 不复位。
    preloaded: AtomicBool,
    eof: AtomicBool,
    failed: AtomicBool,
    /// 声部丢弃标志：`StreamVoice::drop` 置真 ⇒ 磁盘线程放弃本作业。
    cancelled: AtomicBool,
    /// 磁盘线程解码后才知道的总帧数（未知时为 0；`eof` 为真时它已是终值）。
    total_frames: AtomicU64,
    frames_read: AtomicU64,
    underrun_frames: AtomicU64,
}

/// 一条声部流：磁盘线程在后台喂环，消费侧用 [`Self::read_block`] 取声。
///
/// ⚠ **一流一消费者**：`read_block` 的游标归属是单消费者的，⛔ 不要克隆
/// 本类型后两方同时读（本类型故意不实现 `Clone`）。
/// 消费侧只做原子读写与切片拷贝：`read_block` 永不阻塞、永不碰盘、永不分配。
#[derive(Debug)]
pub struct StreamVoice {
    state: Arc<VoiceState>,
}

impl StreamVoice {
    /// 起音是否已就位：磁盘线程已发布够 `preload_frames` 帧，或文件已播完。
    ///
    /// 调用方应在 `ready()` 为真后再开播 ⇒ 起音一到就有声。
    /// `preload_frames == 0` 的声部触发时即就绪。
    #[must_use]
    pub fn ready(&self) -> bool {
        self.state.preloaded.load(Ordering::Acquire)
    }

    /// 是否已播完（`eof` 且已读帧数 ≥ 总帧数；读盘失败的声部也算播完——恒静音）。
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.state.eof.load(Ordering::Acquire)
            && self.state.frames_read.load(Ordering::Relaxed)
                >= self.state.total_frames.load(Ordering::Acquire)
    }

    /// 读一块：从环里取声、环空处补零，返回本次读数。
    ///
    /// ⛔ 永不阻塞：环里有多少就给多少，缺的补零并计入欠载；`out` 的长度
    /// 就是本次消费的帧数（实声＋补零之和），与调用方的块长恒一致。
    pub fn read_block(&self, out: &mut [f32]) -> VoiceRead {
        let from_ring = self.state.ring.read(out);
        let underrun = out.len() - from_ring;
        self.state
            .frames_read
            .fetch_add(from_ring as u64, Ordering::Relaxed);
        self.state
            .underrun_frames
            .fetch_add(underrun as u64, Ordering::Relaxed);
        VoiceRead {
            from_ring,
            underrun,
            finished: self.is_finished(),
        }
    }

    /// 累计读数（诊断用；音频线程调它也不阻塞——全是原子读）。
    #[must_use]
    pub fn stats(&self) -> VoiceStats {
        VoiceStats {
            frames_read: self.state.frames_read.load(Ordering::Relaxed),
            underrun_frames: self.state.underrun_frames.load(Ordering::Relaxed),
            available: self.state.ring.available(),
            finished: self.is_finished(),
            failed: self.state.failed.load(Ordering::Acquire),
        }
    }
}

impl Drop for StreamVoice {
    fn drop(&mut self) {
        // 声部丢弃 ⇒ 磁盘线程放弃本作业（否则环满时它会一直等消费侧读）。
        self.state.cancelled.store(true, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// ③ 磁盘线程引擎
// ---------------------------------------------------------------------------

/// 触发声部时的选项（全部是帧数）。
#[derive(Clone, Copy, Debug)]
pub struct StreamOptions {
    /// 起音预载帧数（`ready()` 的门槛；0 ＝ 触发即就绪）。
    pub preload_frames: usize,
    /// 声部环容量（向上取到 2 的幂）。
    pub ring_frames: usize,
    /// 磁盘线程每次喂环的块大小（⛔ 不得为 0，触发时钳到 ≥ 1）。
    pub chunk_frames: usize,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            preload_frames: DEFAULT_PRELOAD_FRAMES,
            ring_frames: DEFAULT_RING_FRAMES,
            chunk_frames: DEFAULT_CHUNK_FRAMES,
        }
    }
}

/// 磁盘线程的一项作业：把一个文件喂进一个声部的环。
struct StreamJob {
    state: Arc<VoiceState>,
    path: PathBuf,
    chunk_frames: usize,
}

/// 磁盘流式引擎：一条后台读盘线程，服务全部声部。
///
/// 线程模型：单磁盘线程＋`mpsc` 作业队列 ⇒ 作业顺序喂、在途解码恒为一个文件。
/// `Drop` 时先立 `shutdown` 旗、再丢发送端、再 `join` 线程：
///
/// - 线程在 `recv` 里 ⇒ 发送端一丢就退出；
/// - 线程在喂环里 ⇒ 每块都查 `cancelled`／`shutdown`，环满时只退让 1 ms
///   就重查 ⇒ `join` 永不挂住（在途的那次整文件解码本身是有界的：
///   单文件 64 MiB 护栏在 `read_sample_file` 里）。
///
/// ⚠ 正常用法是**先丢声部、再丢引擎**；即使声部比引擎活得久，引擎 `Drop`
/// 后声部也只是读零＋记欠载（⛔ 不挂、不崩），因为喂环停了、环空了。
#[derive(Debug)]
pub struct StreamEngine {
    sender: Option<mpsc::Sender<StreamJob>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl StreamEngine {
    /// 建引擎并起磁盘线程（线程起不来时退化：`trigger` 给恒静音声部）。
    #[must_use]
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<StreamJob>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let thread = thread::Builder::new()
            .name("yeban-sfz-stream".to_owned())
            .spawn(move || disk_main(receiver, thread_shutdown))
            .ok();
        Self {
            sender: if thread.is_some() { Some(sender) } else { None },
            shutdown,
            thread,
        }
    }

    /// 磁盘线程是否在跑（`false` ＝ 退化态，本引擎触发的声部恒静音）。
    #[must_use]
    pub fn is_thread_running(&self) -> bool {
        self.thread.is_some()
    }

    /// 触发一个声部：磁盘线程在后台读 `path`、按块喂环。
    ///
    /// 返回的声部在 `ready()` 为真后可开播；`read_block` 永不阻塞。
    /// `path` 读不到时声部恒静音（`stats().failed` 为真、立即 `is_finished`）。
    #[must_use]
    pub fn trigger(&self, path: &Path, options: StreamOptions) -> StreamVoice {
        let state = Arc::new(VoiceState {
            ring: SampleRing::new(options.ring_frames),
            preload_frames: options.preload_frames,
            preloaded: AtomicBool::new(options.preload_frames == 0),
            eof: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            total_frames: AtomicU64::new(0),
            frames_read: AtomicU64::new(0),
            underrun_frames: AtomicU64::new(0),
        });
        let job = StreamJob {
            state: Arc::clone(&state),
            path: path.to_path_buf(),
            chunk_frames: options.chunk_frames.max(1),
        };
        let sent = self
            .sender
            .as_ref()
            .is_some_and(|sender| sender.send(job).is_ok());
        if !sent {
            state.failed.store(true, Ordering::Release);
            state.eof.store(true, Ordering::Release);
        }
        StreamVoice { state }
    }
}

impl Default for StreamEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StreamEngine {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // 发送端一丢，`recv` 里的线程即退出；喂环中的线程每块都查旗。
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 磁盘线程主循环：顺序喂作业，发送端丢了（引擎已 `Drop`）即退出。
fn disk_main(receiver: mpsc::Receiver<StreamJob>, shutdown: Arc<AtomicBool>) {
    while let Ok(job) = receiver.recv() {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        feed_job(&job, &shutdown);
    }
}

/// 把一个文件喂进一个声部的环：与 P1 同一条整文件解码腿，分块喂环。
fn feed_job(job: &StreamJob, shutdown: &AtomicBool) {
    let state = &job.state;
    let reading = match read_sample_file(&job.path) {
        Ok(reading) => reading,
        Err(_) => {
            // 读不到 ⇒ 声部恒静音（原因由调用方重读文件自查，本层不另记字符串，
            // 以免消费侧读它时要锁）。
            state.failed.store(true, Ordering::Release);
            state.eof.store(true, Ordering::Release);
            return;
        }
    };
    state.total_frames.store(reading.frames, Ordering::Release);
    let pcm = &reading.pcm;
    let mut offset = 0usize;
    let mut published = 0u64;
    while offset < pcm.len() {
        if state.cancelled.load(Ordering::Acquire) || shutdown.load(Ordering::Acquire) {
            return;
        }
        let end = (offset + job.chunk_frames).min(pcm.len());
        let written = state.ring.write(&pcm[offset..end]);
        if written == 0 {
            // 环满且消费侧停读：只退让、不覆盖、不阻塞。
            thread::sleep(RING_FULL_BACKOFF);
            continue;
        }
        offset += written;
        published += written as u64;
        if published >= state.preload_frames as u64 {
            state.preloaded.store(true, Ordering::Release);
        }
    }
    // 文件短于预载时：播完即就绪（剩下的帧消费侧读零，但 `ready` 不该永假）。
    state.preloaded.store(true, Ordering::Release);
    state.eof.store(true, Ordering::Release);
}

// ---------------------------------------------------------------------------
// ④ 预载范围：预览（已用区域）与实时（整件乐器）
// ---------------------------------------------------------------------------

/// 收集预载源文件：去重（同一文件多区域只列一次）、按路径排序（确定性）。
///
/// - `wanted == Some(filter)` ⇒ 预览：只收 `filter.wants(region)` 的区域
///   （与 P1 的 `load_with(wanted)` 同一套判定 ⇒ 文件数／帧数与 P1 收据一致）；
/// - `wanted == None` ⇒ 实时：收全部声明了样本文件的区域（含 `key=-1` 的
///   一次性区域——整件乐器的起音就该全含）。
///
/// 两路都跳过：内建静音（`*silence`，无文件可读）与空 `sample=`。
/// 路径解析与 `load_with` 同一条：`sample_path()`（含 `default_path=` 前缀）
/// 再相对 SFZ 所在目录解。
fn collect_sources(
    sfz_path: &Path,
    instrument: &Instrument<'_>,
    wanted: Option<&DemandLoadFilter>,
) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for region in instrument.regions() {
        if let Some(filter) = wanted
            && !filter.wants(region)
        {
            continue;
        }
        if region.is_builtin_silence() {
            continue;
        }
        let sample = region.sample_path();
        if sample.trim().is_empty() {
            continue;
        }
        let resolved = resolve_sample_path(sfz_path, &sample);
        if seen.insert(resolved.clone()) {
            out.push(resolved);
        }
    }
    out
}

/// 预览预载源：本谱子会用到的区域的样本文件（去重、路径排序）。
///
/// 调用方先用 `DemandLoadFilter::from_project_track` 建判定器，再用
/// `SfzLoad::with_instrument` 拿到解析后的乐器调本函数。
/// 返回的文件数与 P1 对同一谱子的收据（文件数、帧数）一致——判据见
/// `tests/sfz_stream.rs` 的稀疏／全谱两案。
#[must_use]
pub fn preview_sources(
    sfz_path: &Path,
    instrument: &Instrument<'_>,
    filter: &DemandLoadFilter,
) -> Vec<PathBuf> {
    collect_sources(sfz_path, instrument, Some(filter))
}

/// 实时预载源：整件乐器的样本文件（去重、路径排序），不看谱子。
#[must_use]
pub fn realtime_sources(sfz_path: &Path, instrument: &Instrument<'_>) -> Vec<PathBuf> {
    collect_sources(sfz_path, instrument, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 环的基本读写：写多少读多少，位模式逐位相同。
    #[test]
    fn ring_roundtrip_is_bit_exact() {
        let ring = SampleRing::new(16);
        let input = [0.5, -0.25, 0.1, -0.0, 1.0, -1.0, 0.0, 0.75];
        assert_eq!(ring.write(&input), input.len());
        assert_eq!(ring.available(), input.len());
        let mut out = [0.0_f32; 8];
        assert_eq!(ring.read(&mut out), input.len());
        for (got, want) in out.iter().zip(input.iter()) {
            assert_eq!(got.to_bits(), want.to_bits(), "位模式必须逐位相同");
        }
        assert_eq!(ring.available(), 0);
    }

    /// 环空读：返回 0 且输出全零（⛔ 不阻塞——本测试本身就是证明：它返回了）。
    #[test]
    fn ring_empty_read_returns_zeros() {
        let ring = SampleRing::new(8);
        let mut out = [9.0_f32; 4];
        assert_eq!(ring.read(&mut out), 0);
        assert_eq!(out.len(), 4, "守卫界：输出块非空，all 断言非真空");
        assert!(out.iter().all(|&sample| sample == 0.0));
    }

    /// 环满写：只写得下空闲数，⛔ 不覆盖未消费数据。
    #[test]
    fn ring_full_write_is_capped_not_overwriting() {
        let ring = SampleRing::new(4);
        assert_eq!(ring.write(&[1.0, 2.0, 3.0, 4.0]), 4);
        assert_eq!(ring.write(&[5.0, 6.0]), 0, "环满时一帧都写不进");
        let mut out = [0.0_f32; 2];
        assert_eq!(ring.read(&mut out), 2);
        assert_eq!(out, [1.0, 2.0]);
        assert_eq!(ring.write(&[5.0, 6.0]), 2, "读出两帧后恰好写进两帧");
        assert_eq!(ring.free(), 0);
    }

    /// 环回绕：写读交错越过容量边界后，数据顺序仍正确。
    #[test]
    fn ring_wraps_around_without_reordering() {
        let ring = SampleRing::new(4);
        let mut all = Vec::new();
        for chunk in 0..6 {
            let data = [chunk as f32; 3];
            let mut written = 0;
            while written < data.len() {
                written += ring.write(&data[written..]);
                let mut out = [0.0_f32; 3];
                let read = ring.read(&mut out);
                all.extend_from_slice(&out[..read]);
                if written < data.len() && read == 0 {
                    panic!("环既不满又不进 ⇒ 协议坏了");
                }
            }
            let mut out = [0.0_f32; 3];
            let read = ring.read(&mut out);
            all.extend_from_slice(&out[..read]);
        }
        assert_eq!(all.len(), 18, "6 块 × 3 帧必须一帧不少");
        for (i, &sample) in all.iter().enumerate() {
            assert_eq!(sample, (i / 3) as f32, "第 {i} 帧顺序错");
        }
    }
}
