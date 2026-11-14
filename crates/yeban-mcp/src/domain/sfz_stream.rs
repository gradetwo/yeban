//! P3 磁盘流式引擎：后台读盘线程＋逐声部无锁环＋起音预载＋样本回收＋声部池。
//!
//! 位置：P1（`sfz::load_with` 按需装载）解决的是**离线渲染导出**——
//! 无实时约束，按需整文件读就够，正确完整第一。本模块解决的是
//! **预览播放与实时演奏**那一侧：起播要快、音频线程绝不能碰磁盘。
//!
//! ## 架构（四件套，P2 三件＋P3 两件）
//!
//! - **磁盘线程**（[`StreamEngine`] 内一条）：唯一碰文件系统的地方。
//!   P3 起按**后缀分腿**喂环（⛔ 不再一律整文件解码）：WAV 走
//!   [`PagedWav`](yeban_render::wav::PagedWav) 按需读（见下），FLAC/OGG
//!   走 [`StreamingDecoder`](yeban_decode::StreamingDecoder) 流式解码，
//!   按需读腿拒收的格式（32 位整数、ADPCM 等）回落 P1 整文件腿。
//!   一次只抱一个文件在途（单线程顺序喂 ⇒ 在途解码内存恒为一个文件）。
//! - **逐声部无锁环**（[`SampleRing`]）：每个声部一个。生产者（磁盘线程）
//!   写、消费者（音频线程）读，只用原子操作交接：**无锁、无分配、无 IO**。
//!   消费者路径（[`StreamVoice::read_block`]）只有原子读写与切片拷贝，
//!   满足实时音频回调"不分配、不加锁、不阻塞 IO"那条红线。
//! - **起音预载**：声部触发后，磁盘线程先把前 `preload_frames` 帧喂进环
//!   并打上 `preloaded` 闩，[`StreamVoice::ready`] 才为真 ⇒ 调用方在
//!   `ready()` 为真后再开播，起音一到就有声，不用等整文件解码。
//! - **样本回收**（[`SourceCache`]，P3）：磁盘线程内的按需读句柄缓存。
//!   同一文件被多声部触发时只开一次句柄；久未用的句柄按闲置超时与
//!   数量上限淘汰（策略见 [`StreamCacheConfig`]）。
//! - **声部池**（[`StreamingVoicePool`]，P3）：`VoicePool`（分配／窃取／
//!   退役淡出）＋ `StreamEngine`（流式声部）＋单声道求和。离线渲染
//!   仍走 P1 的 `load_with`；本池是预览／实时播放器的声部源。
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
//! ## P3 的 WAV 腿为什么是"按需读"而不是 `mmap`
//!
//! 原定 `mmap`：锁文件里唯一的映射库 `memmap2 0.9.11` 在 0.9 里把全部
//! 文件映射构造都改成了 `unsafe fn`，而 `yeban-render`
//! `#![forbid(unsafe_code)]`；新立允许 `unsafe` 的小 crate 又要改根清单
//! （集成者专属）。因此 WAV 腿改用纯 `std` 的 `Seek + Read` 按需读
//! （[`PagedWav`](yeban_render::wav::PagedWav)）：目标不变（无整文件拷贝、
//! 有界常驻、页缓存），机制换一条；接口与原定映射腿同形，将来若集成者
//! 授权映射隔离可直换。详见 `yeban-render` 的 `PagedWav` 文档。
//!
//! ## 内存故事（与 P1 对比）
//!
//! P1 的离线渲染把**全部已用文件**解码后抱在 `SampleStore` 里直到渲染结束。
//! P3 的稳态内存 = 每声部一个有界环（默认 16384 帧）＋ 磁盘线程在途的一个文件
//! ＋ 样本缓存（上限 256 句柄，每句柄一个打开文件与一个复用窗口缓冲）。
//! 文件数越多，P3 相对 P1 省得越多；代价是声部读到环空时出零（欠载计数见
//! [`VoiceStats::underrun_frames`]），⛔ 不是阻塞等盘。
//!
//! ## 本期边界（如实登记，⛔ 不多认）
//!
//! - WAV 按需读腿**不**执行 P1 的单文件 64 MiB 上限（那条上限管的是"解码拷贝
//!   常驻内存"，按需读不拷贝）；离线 P1 路径的上限不动。回落整文件腿时
//!   64 MiB 护栏照常生效。
//! - 流式解码腿（FLAC/OGG）**不**做整段资产级校验：声明时长对账与 0 帧拒绝
//!   是整段入口的语义，流式只播"解出来的东西"；中途硬错后已播出的部分
//!   不再收回（声部转恒静音＋已播完）。
//! - 样本缓存只缓存**位置无关**的按需读句柄（WAV）：流式解码器有位置状态，
//!   ⛔ 不进缓存、随声部丢弃即是其回收。缓存条目用文件长度＋修改时间做
//!   失效判定（同长同 mtime 的改写在同一 mtime 刻度内是残留口子；闲置超时
//!   把它 bound 在 300 秒内）；缓存是会话期快照，⛔ 不是文件监听。
//! - 声部池**不**做变速／音高／包络／循环：那是引擎 `PreparedSfz` 的职责，
//!   未来播放器再组合。池只做分配／窃取／退役淡出／电平求和。
//! - 池的淡入曲线是 P3 的决定（`VoicePool` 只给计数器，不给增益函数）：
//!   退役淡出用 [`StealFade::gain_at`](yeban_sfz::voice_pool::StealFade::gain_at)
//!   的已走采样数；窃取淡入用**同一函数**的剩余采样数（时间镜像：
//!   剩余 N ⇒ 增益 0，剩余 0 ⇒ 增益 1，与退役淡出同一指数形状）。
//!   淡出／淡入增益按**样本**算（块内逐样本推进），⛔ 不按块取常数 ——
//!   3 ms 的淡出若按块取常数，块边界上就是一次增益台阶（可闻咔哒）。
//! - 循环窗口**不展开**（与 P1 层同口径）：声部播完一遍即结束，
//!   `is_finished` 为真后继续读只出零。
//! - 预算（`SampleLimits`）**不在本层执行**：预算仍由装载层（P1）管；
//!   本层只报数（预载文件数、帧数），调用方拿去对预算。
//! - 本模块**零新依赖**（只用 `std` ＋ 既有 `yeban-render`／`yeban-decode`／
//!   `yeban-sfz` 边）：`Cargo.toml` 是集成者专属文件，执行员不动；无锁环用
//!   `AtomicU32` 存 `f32` 位模式，⛔ 不写 `unsafe`
//!   （本 crate 已 `#![forbid(unsafe_code)]`）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use yeban_decode::DecodeOptions;
use yeban_decode::decode::StreamingDecoder;
use yeban_decode::error::DecodeError;
use yeban_render::wav::PagedWav;
use yeban_sfz::voice_pool::{NoteOnOutcome, VoiceHandle, VoicePool};
use yeban_sfz::{SfzError, instrument::Instrument};

use super::sfz::{DemandLoadFilter, is_plain_wav_suffix, read_sample_file, resolve_sample_path};

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
// ②.5 样本回收缓存（P3，只活在磁盘线程里）
// ---------------------------------------------------------------------------

/// 样本回收缓存的配置（P3 策略，见模块文档）。
#[derive(Clone, Copy, Debug)]
pub struct StreamCacheConfig {
    /// 缓存条目上限（句柄数；超过时按 `last_used` 最旧淘汰）。
    pub max_entries: usize,
    /// 闲置超时（秒；每次作业开始时扫一遍，清掉超时未用的条目）。
    pub idle_timeout_secs: u64,
}

impl Default for StreamCacheConfig {
    fn default() -> Self {
        Self {
            // 界定句柄表（覆盖 realistic 套件；重开句柄只是微秒级 `open`＋读头）。
            max_entries: 256,
            // 长于乐句间隙、短于会话：演奏中途换音色不至于刚换就丢。
            idle_timeout_secs: 300,
        }
    }
}

/// 缓存读数快照（调用方线程可读；磁盘线程用原子量维护，⛔ 不加锁）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// 缓存命中次数。
    pub hits: u64,
    /// 缓存未命中次数（含失效后重开）。
    pub misses: u64,
    /// 淘汰次数（上限淘汰＋闲置超时＋失效，三者之和）。
    pub evictions: u64,
    /// 当前条目数。
    pub entries: usize,
}

/// [`CacheStats`] 的磁盘线程侧共享计数器（引擎与磁盘线程各持一份 `Arc`）。
#[derive(Debug, Default)]
struct CacheStatsInner {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    entries: AtomicUsize,
}

impl CacheStatsInner {
    fn snapshot(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            entries: self.entries.load(Ordering::Relaxed),
        }
    }
}

/// 一条缓存条目：位置无关的按需读句柄＋失效快照＋最近使用时间。
struct CacheEntry {
    wav: PagedWav,
    /// `open` 时的文件长度（失效判定用）。
    len: u64,
    /// `open` 时的修改时间（失效判定用；`None` ＝ 当时取不到时间 ⇒ 永不因时间失效）。
    modified: Option<SystemTime>,
    last_used: Instant,
}

/// P3 样本回收缓存：只活在磁盘线程里（⛔ 不跨线程、不加锁）。
///
/// 缓存对象是**位置无关**的按需读句柄（[`PagedWav`]）：同一句柄可供同一文件
/// 的全部声部共享（磁盘线程串行喂环，共享不需要锁）。流式解码器有位置状态，
/// ⛔ 不进缓存、随声部丢弃即是其回收；回落整文件腿的格式（按需读腿拒收者）
/// 同样不进缓存（每次触发重读，与 P2 同成本）。
///
/// 淘汰策略（两条，缺一不可）：
/// - **闲置超时**：每次作业开始时扫一遍，清掉 `now - last_used > idle_timeout`
///   的条目（长于乐句间隙、短于会话）；
/// - **数量上限**：`insert` 时若已满，按 `last_used` 最旧淘汰到上限内
///   （新条目恒为最新，⛔ 不会被自己的这次插入淘汰）。
///
/// 失效判定：命中时先 `stat` 一次（长度＋修改时间），与 `open` 时的快照不符
/// ⇒ 视为失效：删条目、按未命中处理（调用方重开句柄）。同长同 `mtime` 的
/// 改写在同一 `mtime` 刻度内是残留口子；闲置超时把它 bound 在超时内。
/// 文件被删时 `stat` 失败 ⇒ 同样视为失效（调用方重开失败后走回落腿）。
struct SourceCache {
    entries: BTreeMap<PathBuf, CacheEntry>,
    config: StreamCacheConfig,
}

impl SourceCache {
    fn new(config: StreamCacheConfig) -> Self {
        Self {
            entries: BTreeMap::new(),
            config,
        }
    }

    /// 命中：`stat` 校验通过 ⇒ 刷新 `last_used` 并交出句柄；
    /// 校验失败（改过／删了）⇒ 删条目、返回 `None`（调用方按未命中重开）。
    fn get_mut(&mut self, path: &Path, stats: &CacheStatsInner) -> Option<&mut PagedWav> {
        // 先拷快照（`len`／`modified` 皆 `Copy`），借用随即结束，后面才能删条目。
        let entry = self.entries.get(path)?;
        let (len, modified) = (entry.len, entry.modified);
        let fresh = std::fs::metadata(path).ok().is_some_and(|metadata| {
            metadata.len() == len
                && match (metadata.modified().ok(), modified) {
                    (Some(now), Some(then)) => now == then,
                    (None, None) => true,
                    _ => false,
                }
        });
        if !fresh {
            self.entries.remove(path);
            stats.evictions.fetch_add(1, Ordering::Relaxed);
            stats.entries.store(self.entries.len(), Ordering::Relaxed);
            return None;
        }
        let entry = self.entries.get_mut(path)?;
        entry.last_used = Instant::now();
        Some(&mut entry.wav)
    }

    /// 存入（调用方已喂完、句柄不再被借用时调）：满了先按最旧淘汰。
    fn insert(&mut self, path: PathBuf, wav: PagedWav, stats: &CacheStatsInner) {
        let len = std::fs::metadata(&path).ok().map(|metadata| metadata.len());
        let modified = std::fs::metadata(&path)
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        // `len` 取不到（喂完后文件被删）时用 0 占位：下次命中 `stat` 必不符 ⇒
        // 立刻失效重开，⛔ 不会把已删文件的句柄当成有效缓存。
        let entry = CacheEntry {
            wav,
            len: len.unwrap_or(0),
            modified,
            last_used: Instant::now(),
        };
        while self.entries.len() >= self.config.max_entries.max(1) {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
            stats.evictions.fetch_add(1, Ordering::Relaxed);
        }
        self.entries.insert(path, entry);
        stats.entries.store(self.entries.len(), Ordering::Relaxed);
    }

    /// 清闲置超时条目（每次作业开始时调一次）。
    fn sweep(&mut self, stats: &CacheStatsInner) {
        let timeout = Duration::from_secs(self.config.idle_timeout_secs);
        let now = Instant::now();
        let before = self.entries.len();
        self.entries
            .retain(|_, entry| now.duration_since(entry.last_used) <= timeout);
        let removed = before - self.entries.len();
        if removed > 0 {
            stats.evictions.fetch_add(removed as u64, Ordering::Relaxed);
        }
        stats.entries.store(self.entries.len(), Ordering::Relaxed);
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
    cache_stats: Arc<CacheStatsInner>,
}

impl StreamEngine {
    /// 建引擎并起磁盘线程（线程起不来时退化：`trigger` 给恒静音声部）。
    #[must_use]
    pub fn new() -> Self {
        Self::with_cache_config(StreamCacheConfig::default())
    }

    /// 建引擎并起磁盘线程，样本回收缓存用给定配置（测试用小上限强制淘汰时用）。
    #[must_use]
    pub fn with_cache_config(config: StreamCacheConfig) -> Self {
        let (sender, receiver) = mpsc::channel::<StreamJob>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let stats = Arc::new(CacheStatsInner::default());
        let thread_stats = Arc::clone(&stats);
        let thread = thread::Builder::new()
            .name("yeban-sfz-stream".to_owned())
            .spawn(move || disk_main(receiver, thread_shutdown, config, thread_stats))
            .ok();
        Self {
            sender: if thread.is_some() { Some(sender) } else { None },
            shutdown,
            thread,
            cache_stats: stats,
        }
    }

    /// 磁盘线程是否在跑（`false` ＝ 退化态，本引擎触发的声部恒静音）。
    #[must_use]
    pub fn is_thread_running(&self) -> bool {
        self.thread.is_some()
    }

    /// 样本回收缓存的读数快照（命中／未命中／淘汰／当前条目数）。
    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        self.cache_stats.snapshot()
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
/// 样本回收缓存归本线程所有（⛔ 不跨线程）。
fn disk_main(
    receiver: mpsc::Receiver<StreamJob>,
    shutdown: Arc<AtomicBool>,
    config: StreamCacheConfig,
    stats: Arc<CacheStatsInner>,
) {
    let mut cache = SourceCache::new(config);
    while let Ok(job) = receiver.recv() {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        feed_job(&job, &shutdown, &mut cache, &stats);
    }
}

/// 喂环的块源：每次产出一块**单声道**帧进 `out`，返回帧数；
/// `None` ＝ 播完。⛔ 永不返回 `Some(0)`（0 帧即播完，调用方不自旋）。
trait ChunkSource {
    fn next_mono(&mut self, out: &mut [f32]) -> Result<Option<usize>, DecodeError>;
    /// 播完时的总帧数（喂环结束时写进声部状态；流式腿到 EOF 才知道）。
    fn final_total(&self) -> u64;
}

/// 整文件腿的块源（回落用）：整段 PCM 的滑动窗口。
struct WholeFileSource<'a> {
    pcm: &'a [f32],
    offset: usize,
}

impl ChunkSource for WholeFileSource<'_> {
    fn next_mono(&mut self, out: &mut [f32]) -> Result<Option<usize>, DecodeError> {
        if self.offset >= self.pcm.len() {
            return Ok(None);
        }
        let n = out.len().min(self.pcm.len() - self.offset);
        out[..n].copy_from_slice(&self.pcm[self.offset..self.offset + n]);
        self.offset += n;
        Ok(Some(n))
    }

    fn final_total(&self) -> u64 {
        self.pcm.len() as u64
    }
}

/// 按需读腿的块源：[`PagedWav`] 的帧窗口（位置无关，⛔ 不持有文件外状态）。
struct PagedSource<'a> {
    wav: &'a mut PagedWav,
    offset: u64,
}

impl ChunkSource for PagedSource<'_> {
    fn next_mono(&mut self, out: &mut [f32]) -> Result<Option<usize>, DecodeError> {
        let n = self.wav.read_mono_frames(self.offset, out);
        if n == 0 {
            return Ok(None);
        }
        self.offset += n as u64;
        Ok(Some(n))
    }

    fn final_total(&self) -> u64 {
        self.wav.frames()
    }
}

/// 流式解码腿的块源：[`StreamingDecoder`] 的交错输出折成单声道（取第 0 声道）。
///
/// 交错缓冲按需增长：`fill` 在 out 装不下一帧时返回 0 且未播完（⛔ 不是 EOF），
/// 此时把缓冲翻倍再调 —— 声道数 ≤ 65535（`u16`）⇒ 缓冲上界 65536 样本，
/// 恒能推进；`fill` 的硬错原样上报（调用方把声部标失败＋已播完）。
struct StreamedSource<'s> {
    decoder: StreamingDecoder<'s>,
    inter: Vec<f32>,
    inter_pos: usize,
    inter_len: usize,
    channels: usize,
}

impl<'s> StreamedSource<'s> {
    fn new(decoder: StreamingDecoder<'s>) -> Self {
        Self {
            decoder,
            inter: Vec::new(),
            inter_pos: 0,
            inter_len: 0,
            channels: 0,
        }
    }
}

impl ChunkSource for StreamedSource<'_> {
    fn next_mono(&mut self, out: &mut [f32]) -> Result<Option<usize>, DecodeError> {
        if self.inter_pos >= self.inter_len {
            if self.decoder.is_finished() {
                return Ok(None);
            }
            if self.inter.is_empty() {
                self.inter.resize(out.len().max(1) * 2, 0.0);
            }
            loop {
                let frames = self.decoder.fill(&mut self.inter)?;
                if frames > 0 {
                    let channels = self.decoder.channels().ok_or(DecodeError::EmptyStream)?;
                    self.channels = usize::from(channels);
                    self.inter_pos = 0;
                    self.inter_len = frames * self.channels;
                    break;
                }
                if self.decoder.is_finished() {
                    return Ok(None);
                }
                // out 装不下一帧（声道数未知时也可能）⇒ 翻倍再调。
                self.inter.resize(self.inter.len() * 2, 0.0);
            }
        }
        let available_frames = (self.inter_len - self.inter_pos) / self.channels.max(1);
        let take = available_frames.min(out.len());
        if take == 0 {
            return Ok(None);
        }
        for (index, slot) in out[..take].iter_mut().enumerate() {
            *slot = self.inter[self.inter_pos + index * self.channels];
        }
        self.inter_pos += take * self.channels;
        Ok(Some(take))
    }

    fn final_total(&self) -> u64 {
        self.decoder.decoded_frames()
    }
}

/// 喂环循环的公共骨架：取消／关停检查、环满退让、预载闩、EOF 旗全部共用；
/// 三腿只差块源。`initial_total` 已知时先写进声部状态（WAV／整文件腿），
/// 播完时再以 `final_total` 为准写一次（流式腿到 EOF 才知道总数）。
///
/// 块源硬错时返回 `Err`：调用方把声部标失败＋已播完（已喂进环的帧照常播出）。
fn feed_loop(
    job: &StreamJob,
    shutdown: &AtomicBool,
    initial_total: Option<u64>,
    source: &mut dyn ChunkSource,
) -> Result<(), DecodeError> {
    let state = &job.state;
    if let Some(total) = initial_total {
        state.total_frames.store(total, Ordering::Release);
    }
    let mut scratch = vec![0.0_f32; job.chunk_frames];
    let mut published = 0u64;
    loop {
        if state.cancelled.load(Ordering::Acquire) || shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        let Some(n) = source.next_mono(&mut scratch)? else {
            break;
        };
        if n == 0 {
            break;
        }
        let mut written_total = 0usize;
        while written_total < n {
            if state.cancelled.load(Ordering::Acquire) || shutdown.load(Ordering::Acquire) {
                return Ok(());
            }
            let written = state.ring.write(&scratch[written_total..n]);
            if written == 0 {
                // 环满且消费侧停读：只退让、不覆盖、不阻塞。
                thread::sleep(RING_FULL_BACKOFF);
                continue;
            }
            written_total += written;
            published += written as u64;
            if published >= state.preload_frames as u64 {
                state.preloaded.store(true, Ordering::Release);
            }
        }
    }
    state
        .total_frames
        .store(source.final_total(), Ordering::Release);
    // 文件短于预载时：播完即就绪（剩下的帧消费侧读零，但 `ready` 不该永假）。
    state.preloaded.store(true, Ordering::Release);
    state.eof.store(true, Ordering::Release);
    Ok(())
}

/// 把一个文件喂进一个声部的环：按后缀分腿（P3），失败时声部恒静音。
///
/// - WAV（`is_plain_wav_suffix`，与 `read_sample_file` 同一条规则，⛔ 不另立
///   第二份）：缓存命中直接喂；未命中则 `PagedWav::open`，成功入库再喂，
///   失败回落整文件腿（与 P2 同行为：整文件腿再失败才标失败）。
/// - 非 WAV：`StreamingDecoder::open_path`，打开失败即失败＋已播完
///   （与现行读失败结局相同，不做冗余重读）。
/// - 每次作业开始时先扫缓存（清闲置超时条目）。
fn feed_job(
    job: &StreamJob,
    shutdown: &AtomicBool,
    cache: &mut SourceCache,
    stats: &CacheStatsInner,
) {
    let state = &job.state;
    cache.sweep(stats);
    if is_plain_wav_suffix(&job.path) {
        if let Some(wav) = cache.get_mut(&job.path, stats) {
            stats.hits.fetch_add(1, Ordering::Relaxed);
            let total = wav.frames();
            let mut source = PagedSource { wav, offset: 0 };
            let _ = feed_loop(job, shutdown, Some(total), &mut source);
            return;
        }
        stats.misses.fetch_add(1, Ordering::Relaxed);
        match PagedWav::open(&job.path) {
            Ok(mut wav) => {
                let total = wav.frames();
                let mut source = PagedSource {
                    wav: &mut wav,
                    offset: 0,
                };
                let _ = feed_loop(job, shutdown, Some(total), &mut source);
                cache.insert(job.path.clone(), wav, stats);
            }
            Err(_) => feed_whole(job, shutdown),
        }
        return;
    }
    match StreamingDecoder::open_path(&job.path, &DecodeOptions::default()) {
        Ok(decoder) => {
            let mut source = StreamedSource::new(decoder);
            if feed_loop(job, shutdown, None, &mut source).is_err() {
                // 流式中途硬错：已喂的帧照常播出，声部转恒静音＋已播完。
                state.failed.store(true, Ordering::Release);
                state.eof.store(true, Ordering::Release);
            }
        }
        Err(_) => {
            state.failed.store(true, Ordering::Release);
            state.eof.store(true, Ordering::Release);
        }
    }
}

/// 回落整文件腿（P2 原行为）：整文件解码后分块喂环，读不到即失败＋已播完。
fn feed_whole(job: &StreamJob, shutdown: &AtomicBool) {
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
    let total = reading.frames;
    let mut source = WholeFileSource {
        pcm: &reading.pcm,
        offset: 0,
    };
    let _ = feed_loop(job, shutdown, Some(total), &mut source);
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
    // ⭐ set_ccN 离线播种（路线 a）：与 `load_with_resolved` 同一条并集 ——
    // 预览预载集必须与 P1 装载的命中集同口径（种子 ∪ 泳道，泳道赢）。
    let filter = filter.with_cc_seeds(instrument.cc_defaults());
    collect_sources(sfz_path, instrument, Some(&filter))
}

/// 实时预载源：整件乐器的样本文件（去重、路径排序），不看谱子。
#[must_use]
pub fn realtime_sources(sfz_path: &Path, instrument: &Instrument<'_>) -> Vec<PathBuf> {
    collect_sources(sfz_path, instrument, None)
}

// ---------------------------------------------------------------------------
// ⑤ 流式声部池（P3）
// ---------------------------------------------------------------------------

/// dB 转线性增益（与 `yeban-dsp` 的 `db_to_gain` **同一条公式**，
///
/// `exp2(db / 6.0206)`，`-120 dB` 以下为 0；`db_to_gain(0) = 1.0` 精确成立）。
/// mcp 不直接依赖 `yeban-dsp`（那条边不存在），因此公式在此复写并注明出处，
/// ⛔ 不另发明口径。
fn db_to_gain(db: f32) -> f32 {
    if db <= -120.0 {
        0.0
    } else {
        (db / 6.020_6).exp2()
    }
}

/// 池内一条声部：流式声部＋触发时 `VoicePool` 给的句柄（代数随槽位走）。
#[derive(Debug)]
struct PoolVoice {
    voice: StreamVoice,
    handle: VoiceHandle,
}

/// P3 流式声部池：`VoicePool`（分配／窃取／退役淡出）＋ [`StreamEngine`]
/// （流式声部）＋单声道求和。
///
/// 位置：预览／实时播放器的声部源。离线渲染仍走 P1 的 `load_with`。
///
/// 渲染口径（`render_block`）：
/// - `out` 是**单声道**求和缓冲；本函数先清零再求和（调用方不必预清零）。
/// - 每槽增益 = `db_to_gain(level_db)` × 包络增益（逐样本）：
///   退役槽用 `StealFade::gain_at(已走采样数)`，窃取淡入槽用同一函数的
///   剩余采样数（时间镜像，见模块文档）；其余槽包络增益恒 1。
/// - 每块结束时调 `pool.process(frames)` 推进淡出计时并回收槽位；
///   被回收的槽位其 `StreamVoice` 随 `Drop` 取消磁盘作业。
/// - 播完（文件播完且环读尽）且未退役的槽位由本池 `finish` 并清槽
///   （文件播完的声部不应继续占槽位；退役槽位仍由 `process` 回收，
///   ⛔ 不提前截断淡出）。
/// - 电平为 0 的槽位仍照常 `read_block`（消费环、推进播完判定），只是
///   不做加法 —— ⛔ 不读会让磁盘线程卡在环满退让里。
/// - `out` 长于内部读缓冲时按缓冲分块处理（⛔ 不要求调用方块长 ≤ 缓冲）。
#[derive(Debug)]
pub struct StreamingVoicePool {
    pool: VoicePool,
    engine: StreamEngine,
    voices: Vec<Option<PoolVoice>>,
    scratch: Vec<f32>,
}

impl StreamingVoicePool {
    /// 建池：`capacity` 个槽位、`sample_rate` 定 3 ms 淡出长度、
    /// `scratch_frames` 为内部读缓冲（块长上限，分块处理时可小于 `out`）。
    pub fn new(capacity: usize, sample_rate: f32, scratch_frames: usize) -> Result<Self, SfzError> {
        Self::with_cache_config(
            capacity,
            sample_rate,
            scratch_frames,
            StreamCacheConfig::default(),
        )
    }

    /// 建池（样本回收缓存用给定配置；测试用小上限强制淘汰时用）。
    pub fn with_cache_config(
        capacity: usize,
        sample_rate: f32,
        scratch_frames: usize,
        cache_config: StreamCacheConfig,
    ) -> Result<Self, SfzError> {
        let pool = VoicePool::new(capacity, sample_rate)?;
        let capacity = pool.capacity();
        Ok(Self {
            pool,
            engine: StreamEngine::with_cache_config(cache_config),
            voices: (0..capacity).map(|_| None).collect(),
            scratch: vec![0.0_f32; scratch_frames.max(1)],
        })
    }

    /// 池容量（槽位数）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.pool.capacity()
    }

    /// 当前在场声部数。
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.pool.active_count()
    }

    /// 样本回收缓存的读数快照（透传引擎）。
    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        self.engine.cache_stats()
    }

    /// 触发一个声部：池分配槽位（满时窃取并丢弃被窃者的流），
    /// 磁盘线程在后台读 `path`、按块喂环。
    ///
    /// 返回 `VoicePool` 的判决（`Started`／`Stolen`，含被窃者句柄）。
    /// 被窃者的 `StreamVoice` 在此立即丢弃（`Drop` 取消其磁盘作业）。
    #[must_use]
    pub fn trigger(
        &mut self,
        note: u8,
        velocity: u8,
        level_db: f32,
        path: &Path,
        options: StreamOptions,
    ) -> NoteOnOutcome {
        let outcome = self.pool.note_on(note, velocity, level_db);
        if let Some(victim) = outcome.victim() {
            self.voices[victim.index as usize].take();
        }
        let started = outcome.started();
        let voice = self.engine.trigger(path, options);
        self.voices[started.index as usize] = Some(PoolVoice {
            voice,
            handle: started,
        });
        outcome
    }

    /// 槽位声部是否就绪（`ready()`；句柄失效或槽位已换主时返回 `None`）。
    #[must_use]
    pub fn voice_ready(&self, handle: VoiceHandle) -> Option<bool> {
        let pool_voice = self.voices.get(handle.index as usize)?.as_ref()?;
        if pool_voice.handle != handle {
            return None;
        }
        Some(pool_voice.voice.ready())
    }

    /// note-off：进 Release 阶段并开始 3 ms 退役淡出（`retire`）。
    /// 句柄已失效时返回 `Err`（`StaleVoiceHandle`），⛔ 不静默。
    pub fn note_off(&mut self, handle: VoiceHandle) -> Result<(), SfzError> {
        self.pool.note_off(handle)?;
        self.pool.retire(handle)?;
        Ok(())
    }

    /// 改槽位电平（透传 `VoicePool::set_level`）。
    pub fn set_level(&mut self, handle: VoiceHandle, level_db: f32) -> Result<(), SfzError> {
        self.pool.set_level(handle, level_db)
    }

    /// 渲染一块：全部在场声部按电平＋包络增益求和进 `out`（单声道）。
    pub fn render_block(&mut self, out: &mut [f32]) {
        let scratch_len = self.scratch.len();
        for chunk in out.chunks_mut(scratch_len) {
            for slot in chunk.iter_mut() {
                *slot = 0.0;
            }
            let frames = chunk.len();
            let fade = self.pool.steal_fade();
            let fade_samples = fade.samples();
            for index in 0..self.voices.len() {
                let Some(pool_voice) = &self.voices[index] else {
                    continue;
                };
                let Some(info) = self.pool.voice_at(index) else {
                    continue;
                };
                if !info.active {
                    continue;
                }
                let read = pool_voice.voice.read_block(&mut self.scratch[..frames]);
                let _ = read;
                let level = db_to_gain(info.level_db);
                if level == 0.0 {
                    continue;
                }
                for (sample_index, slot) in chunk.iter_mut().enumerate() {
                    let envelope = if info.retiring {
                        let elapsed =
                            fade_samples.saturating_sub(info.fade_remaining) + sample_index as u32;
                        fade.gain_at(elapsed)
                    } else if info.fade_in_remaining > 0 {
                        let remaining = info.fade_in_remaining.saturating_sub(sample_index as u32);
                        fade.gain_at(remaining)
                    } else {
                        1.0
                    };
                    *slot += self.scratch[sample_index] * level * envelope;
                }
            }
            self.pool.process(frames as u32);
            self.reap_finished();
        }
    }

    /// 回收扫描（每块结束时）：被 `process` 回收的槽位清流；
    /// 播完且未退役的槽位 `finish` 并清流（退役槽位仍由 `process` 回收）。
    fn reap_finished(&mut self) {
        for index in 0..self.voices.len() {
            let (handle, finished) = match &self.voices[index] {
                Some(pool_voice) => (pool_voice.handle, pool_voice.voice.is_finished()),
                None => continue,
            };
            match self.pool.voice_at(index) {
                None => {
                    // 槽位不存在（防御性：voices 长度恒等于容量，不应发生）。
                    self.voices[index].take();
                }
                Some(info) if !info.active => {
                    // 已被 `process` 回收：清流（句柄已失效，不再 `finish`）。
                    self.voices[index].take();
                }
                Some(info) if finished && !info.retiring => {
                    // 播完且未退役：`finish` 并清流（句柄有效，失败不可能，忽略结果）。
                    let _ = self.pool.finish(handle);
                    self.voices[index].take();
                }
                _ => {}
            }
        }
    }
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

    /// P3 缓存夹具：手写最小单声道 int16 WAV（48 kHz）。
    fn write_cache_wav(path: &Path, samples: &[i16]) {
        let data_len = (samples.len() * 2) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&96_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, &bytes).expect("写缓存夹具");
    }

    fn cache_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "yeban_p3_cache_{}_{}.wav",
            std::process::id(),
            name
        ))
    }

    /// P3 判据：缓存满时按最近使用最旧淘汰（命中刷新 `last_used` 后，
    /// 被淘汰的是未被刷新的那一条，不是刚命中的）。
    #[test]
    fn cache_evicts_the_least_recently_used_entry_when_full() {
        let stats = CacheStatsInner::default();
        let mut cache = SourceCache::new(StreamCacheConfig {
            max_entries: 2,
            idle_timeout_secs: 3600,
        });
        let pa = cache_temp_path("a");
        let pb = cache_temp_path("b");
        let pc = cache_temp_path("c");
        write_cache_wav(&pa, &[1000, 2000]);
        write_cache_wav(&pb, &[3000, 4000]);
        write_cache_wav(&pc, &[5000, 6000]);
        cache.insert(pa.clone(), PagedWav::open(&pa).expect("开 a"), &stats);
        cache.insert(pb.clone(), PagedWav::open(&pb).expect("开 b"), &stats);
        assert_eq!(stats.snapshot().entries, 2);
        assert!(cache.get_mut(&pa, &stats).is_some(), "命中 a 刷新其时间");
        cache.insert(pc.clone(), PagedWav::open(&pc).expect("开 c"), &stats);
        assert_eq!(stats.snapshot().entries, 2, "上限 2 ⇒ 插入第三条必淘汰一条");
        assert!(
            cache.get_mut(&pa, &stats).is_some(),
            "a 刚命中过，不该被淘汰"
        );
        assert!(cache.get_mut(&pc, &stats).is_some(), "c 是新条目");
        assert!(cache.get_mut(&pb, &stats).is_none(), "b 最旧，必须被淘汰");
        assert_eq!(stats.snapshot().evictions, 1);
        for path in [&pa, &pb, &pc] {
            let _ = std::fs::remove_file(path);
        }
    }

    /// P3 判据：闲置超时扫掉过期条目（超时 0 秒＋睡 2 ms 后 sweep 必清）。
    #[test]
    fn cache_sweep_removes_idle_entries() {
        let stats = CacheStatsInner::default();
        let mut cache = SourceCache::new(StreamCacheConfig {
            max_entries: 10,
            idle_timeout_secs: 0,
        });
        let path = cache_temp_path("idle");
        write_cache_wav(&path, &[1000, 2000]);
        cache.insert(
            path.clone(),
            PagedWav::open(&path).expect("开 idle"),
            &stats,
        );
        assert_eq!(stats.snapshot().entries, 1);
        std::thread::sleep(Duration::from_millis(2));
        cache.sweep(&stats);
        assert_eq!(stats.snapshot().entries, 0, "超时 0 秒 ⇒ 睡后 sweep 必清");
        assert_eq!(stats.snapshot().evictions, 1);
        let _ = std::fs::remove_file(&path);
    }

    /// P3 判据：文件改过（长度变了）后命中判定失效，旧条目被删、按未命中处理。
    #[test]
    fn cache_get_rejects_a_modified_file() {
        let stats = CacheStatsInner::default();
        let mut cache = SourceCache::new(StreamCacheConfig {
            max_entries: 10,
            idle_timeout_secs: 3600,
        });
        let path = cache_temp_path("modified");
        write_cache_wav(&path, &[1000, 2000]);
        cache.insert(
            path.clone(),
            PagedWav::open(&path).expect("开 modified"),
            &stats,
        );
        assert!(cache.get_mut(&path, &stats).is_some(), "未改时命中");
        write_cache_wav(&path, &[1000, 2000, 3000, 4000]);
        assert!(
            cache.get_mut(&path, &stats).is_none(),
            "长度变了 ⇒ 命中必须失效"
        );
        assert_eq!(stats.snapshot().entries, 0, "失效条目必须已删");
        assert_eq!(stats.snapshot().evictions, 1);
        let _ = std::fs::remove_file(&path);
    }

    /// P3 判据：上限配成 0 时钳到 1（缓存恒能留一条，不会插一条丢一条）。
    #[test]
    fn cache_insert_with_zero_cap_still_holds_one() {
        let stats = CacheStatsInner::default();
        let mut cache = SourceCache::new(StreamCacheConfig {
            max_entries: 0,
            idle_timeout_secs: 3600,
        });
        let pa = cache_temp_path("zero_a");
        let pb = cache_temp_path("zero_b");
        write_cache_wav(&pa, &[1000]);
        write_cache_wav(&pb, &[2000]);
        cache.insert(pa.clone(), PagedWav::open(&pa).expect("开 zero_a"), &stats);
        cache.insert(pb.clone(), PagedWav::open(&pb).expect("开 zero_b"), &stats);
        assert_eq!(stats.snapshot().entries, 1);
        assert!(cache.get_mut(&pb, &stats).is_some());
        for path in [&pa, &pb] {
            let _ = std::fs::remove_file(path);
        }
    }
}
