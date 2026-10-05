//! 静态预分配声部池与逐样本合成：**让渲染量子真的出声**
//! [ARCH-RT-001, ARCH-RT-004, ARCH-DET-001, ROAD-M2-005, ROAD-M2-006]。
//!
//! 本模块补上引擎最后一段空白的信号路径。改之前 `rt::render_track_into` 是**占位静音**
//! （`out.fill(0.0)`），于是 `process_quantum` 的端到端输出恒为静音 —— 电平计量、
//! 母线汇流、退役回收全部是真实的，但没有**声源**。现在：
//!
//! ```text
//! YebanProjectV1 ──(控制线程, snapshot 的投影)──► NoteSchedule
//!    TrackV3.clips ─► ClipPlacement ─► ClipPoolEntry(Midi) ─► MidiNote
//!         │  tick → sample 一次性换算 (f64, 构造期)
//!         ▼
//!   ScheduledNote { start_sample, end_sample, phase_inc, freq_hz, gain }
//!         │  RT: 游标触发 → 声部池（定长数组）
//!         ▼
//!   SynthEngine::render_track ──► [f32; 128] ──► 电平 ──► 母线
//! ```
//!
//! ## 1. 为什么"音符 → 样本位置"必须在构造期算完
//!
//! [ARCH-DET-001] 的 L1 契约要求同输入逐位相同。tick → sample 的换算含 `bpm`
//! （`f64`）与 960 PPQ，若放在实时侧逐音符算，就等于把"音频线程的浮点除法顺序"
//! 变成输出的一部分。现在这一步在 [`crate::snapshot`] 的投影里做一次，实时侧只看
//! **整数样本位置**，于是"同一个工程 → 同一串样本位置"是构造性的。
//!
//! ## 2. D32 分类：哪些运算是 IEEE 精确类，哪些是超越函数类
//!
//! [docs/adr/ADR-0001 D32] 要求按运算类别分策。本模块的**逐样本路径**
//! （[`SynthEngine::render_track`] 的内层循环）**只有** IEEE-754 精确类运算：
//!
//! | 逐样本运算 | 类别 | 为什么 |
//! | :--- | :--- | :--- |
//! | `phase.wrapping_add(inc)` | 整数精确 | 无浮点，回绕语义由 `u32` 定义 |
//! | `u64::from(phase) * len as u64` | 整数精确 | 无浮点；`idx = scaled >> 32 < len` |
//! | `a + (b - a) * frac` | IEEE 精确类 | 加/减/乘由 IEEE-754 完全规定 |
//! | `Adsr::process(gate)` | IEEE 精确类 | 逐样本只有加/乘；系数在构造期算好 |
//! | `* envelope * voice.gain` | IEEE 精确类 | 两次乘 |
//!
//! 超越函数（`exp2`/`sin`/`exp`）**只**出现在两类构造期路径上：
//!
//! 1. **控制线程**（[`crate::snapshot`] 的投影）：`note_to_hz`（`2^x`）、`db_to_gain`（`2^x`）；
//! 2. **快照边界**（每修订一次，非逐样本）：`Adsr::set_params` 里的 `exp`（一极点系数）。
//!
//! 因此 L1 逐位相同的**强度**是：同一架构同一工具链下整条合成链逐位相同
//! （由判据 `same_input_renders_byte_identical` 锁住）；跨架构下逐样本运算是位精确的，
//! 只有"音高、增益、包络系数"这几个标量落在 D32 的超越函数预算内。
//!
//! ## 3. 相位推进为什么用整数
//!
//! `phase: u32` 是"一个周期 = 2³²"的定点相位，`inc` 由
//! `round(freq_hz / sample_rate × 2³²)` 得到。整数回绕加法没有累积误差，
//! 也不受 FTZ/DAZ 影响 —— 浮点相位累加会在长音符上缓慢漂移，而漂移量取决于
//! 块切分与累加次数，那正是**非确定性**的来源。`inc == 0`（极低频）被钳到 1，
//! 于是最低可表达频率是 `sample_rate / 2³² ≈ 1.1e-5 Hz @48k`。
//!
//! ## 4. 边界（本切片**没有**做的）
//!
//! - **没有滤波器/音色参数**：只有一个内置波表（[`HOLLOW`] 配方），因为
//!   `yeban-model` 里还没有"乐器参数"到音频线程的形状（`DeviceDefinition::params`
//!   是字符串键值对，尚未投影进快照）；
//! - **声部窃取是硬窃取**：池满时直接抢占"最早结束"的声部并重新初始化，
//!   没有 [ARCH-RT-004] 要求的 3 ms 快速淡出（[`Adsr::start_steal_fade`] 的接入点
//!   在 [`SynthEngine::render_track`] 里，需要额外的"淡出声部"槽位才不产生爆音）；
//! - **没有循环片段展开**：`ClipPlacement::loop_config` 目前被忽略，一个摆放只播一遍；
//!   坐标语义（clip 局部 vs 时间轴）在规范里没有定义，见
//!   `docs/ledger/engine-sound-notes.md` 的 needs；
//! - **没有滑音/弯音/歌词/音素**：`MidiNote::slide`/`pitch_bend_curve`/`phonemes`
//!   不参与合成；
//! - **没有采样播放/SFZ**：`ClipContent::Audio` 与 `yeban-sfz` 的乐器都还没有接到
//!   声部池上（需要采样加载 + 重采样，见 notes 的 pending）。
//!
//! ## 5. 实时侧禁令自检（[AGENTS.md §2 红线 7]）
//!
//! [`SynthEngine`] 的**全部**状态都是定长数组与标量：波表是构造期建好的 `Vec`
//! （`process*` 只读），声部池是 `[TrackSlot; MAX_TRACK_SLOTS]`（每个内含
//! `[Voice; VOICES_PER_TRACK]`）。`render_track` 里没有 `Vec::push`/`Box::new`/
//! `format!`/`println!`/`Mutex::lock`/文件或网络调用。运行期由
//! `tests/synth_rt_zero_alloc.rs`（计数型全局分配器，`harness = false`）钉住。

use yeban_dsp::envelope::{Adsr, AdsrStage, STEAL_RELEASE_SECONDS};
use yeban_dsp::filter::LadderFilter;
use yeban_dsp::math::db_to_gain;
use yeban_dsp::oscillator::{HOLLOW, Wavetable};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, PPQ};

/// 声部池支持的**轨道槽**上限（定长，构造期确定）。
///
/// 超过上限的轨道不参与合成（不 panic、不扩容），由 [`SynthEngine::track_drops`] 计数。
/// 16 条合成轨道足够一个最小引擎；提高它需要重新评估实时侧内存占用
/// （每槽 16 个声部 ≈ 1.4 KiB）。
pub const MAX_TRACK_SLOTS: usize = 16;

/// 每个轨道槽的声部数（复音上限）。
pub const VOICES_PER_TRACK: usize = 16;

/// 单轨音符调度表的容量上限（条）。
///
/// 超过上限的音符在**构造期**被丢弃并计数
/// （[`crate::snapshot::EngineSnapshot::note_schedule_drops`]）。上限的作用是让
/// "快照边界处的游标校正"保持 `O(log n)`（`partition_point`），而不是一次无界的
/// 线性扫描。8192 个音符 ≈ 128 秒 @120 BPM 的十六分音符密度。
pub const MAX_NOTES_PER_TRACK: usize = 8192;

/// 线性插值的小数位宽（相位低 32 位里取最高的 12 位）。
///
/// 12 位是"抖动可忽略、整数运算便宜"的折中：量化误差约 −72 dBFS，
/// 且**完全确定**（没有浮点相位累积）。
const FRAC_BITS: u32 = 12;

/// `1 / 2^FRAC_BITS`，可精确表示 ⇒ 该乘法是 IEEE 精确类。
const FRAC_SCALE: f32 = 1.0 / (1u32 << FRAC_BITS) as f32;

/// 包络参数：线性 attack + 一极点 decay/release（[`Adsr`] 的四段）。
///
/// 选值是"最小可用音色"：5 ms 起音不咔哒、80 ms 衰减到 0.7 的延音、
/// 50 ms 释放让"音符终点"在判据里可界。
const ATTACK_SECONDS: f32 = 0.005;
const DECAY_SECONDS: f32 = 0.08;
const SUSTAIN: f32 = 0.7;
const RELEASE_SECONDS: f32 = 0.05;

/// 采样率下限（Hz），与 `yeban_dsp::MIN_SAMPLE_RATE` 同口径。
const MIN_SAMPLE_RATE: f32 = 1_000.0;

// ---------------------------------------------------------------------------
// 音色参数（引擎侧临时形状）
// ---------------------------------------------------------------------------

/// 滤波器**旁通**时使用的截止频率（Hz）。
///
/// 它不参与声音：旁通由 [`ToneParams::bypass`] 决定，`cutoff_hz` 只是为了让
/// `ToneParams` 的字段在任何状态下都有确定的值（便于 `PartialEq` 与诊断）。
const TONE_BYPASS_CUTOFF_HZ: f32 = 20_000.0;

/// 内置音色的**引擎侧**参数投影：一个四极梯形低通的三个旋钮。
///
/// ## 这是临时形状，不是模型层的第二份定义
///
/// [ARCH-DSP-001] 与 `ROAD-M2-006` 要求声部有音色参数，但 `yeban-model` 目前
/// **没有**"乐器参数 → 音频线程"的投影：`DeviceDefinition::params` 是
/// `Vec<ParameterValue>`（字符串名 + `f32` 值 + 可选单位），没有"哪个设备是合成器"
/// 的类型级信息，也没有参数名规范。因此本切片在自己拥有的目录里定义一层
/// **最小投影**，并把缺的形状登记为 needs（见 `docs/ledger/engine-mix-notes.md`）：
///
/// ```text
/// 现在（引擎侧临时形状）:                   等模型线补齐后:
/// TrackV3.devices[?]                        TrackV3.instrument: Option<InstrumentDefinition>
///   kind == InternalInstrument                ├ cutoff_hz: f32
///   params: [                                ├ resonance: f32
///     {name:"cutoff_hz",  value: 1200}       └ drive: f32
///     {name:"resonance",  value: 0.2}
///     {name:"drive",      value: 0.0}
///   ]                                       ⇒ 本模块的投影整体删除，改成直读字段
/// ```
///
/// ## 投影规则（完全确定，无猜测）
///
/// 1. 只看 [`DeviceKind::InternalInstrument`] 的设备（外部乐器由插件宿主负责，
///    `External*` 一律忽略）；`bypassed` 的设备整体忽略；
/// 2. 在**第一个**含 `cutoff_hz` 或 `cutoff` 参数的设备上取值；
/// 3. 三个参数名（大小写不敏感）：`cutoff_hz`/`cutoff`、`resonance`/`res`、`drive`；
/// 4. **一个参数都没有 ⇒ [`ToneParams::bypass`]**：这是刻意选的默认值，
///    因为"模型层没给参数"与"用户把滤波器拧到旁通"在音频上应当同解 ——
///    这样既有的（无设备链的）工程逐位不变，也避免了给所有轨道硬塞一个未裁决的音色；
/// 5. 取值在构造期钳制（[`LadderFilter::configure`] 内部再钳一次 Nyquist）：
///    截止频率 `20 Hz ..= 0.45·fs`、共振/驱动 `0.0 ..= 1.0`；
/// 6. 非有限值 ⇒ 该参数按默认取（截止 `12 kHz`、共振 `0`、驱动 `0`），绝不 `NaN`。
///
/// ## 确定性分类（[ADR-0001 D32]）
///
/// 构造期 `LadderFilter::configure` 含 `tan(π·fc/fs)` ⇒ **超越函数类**（4096 ulp 预算）；
/// 逐样本 `LadderFilter::process` 只有乘加与一次 Padé 除法 ⇒ **IEEE 精确类**。
/// 因此本切片对跨架构 L1 的强度声明是"系数冻结后逐样本位精确"，
/// **不是**"整条滤波器链跨架构位精确"（详见 notes 的 D32 分类表）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneParams {
    cutoff_hz: f32,
    resonance: f32,
    drive: f32,
    bypass: bool,
}

impl ToneParams {
    /// 旁通（**默认**）：渲染路径**完全不经过**滤波器。
    ///
    /// 不是"滤波器系数取成透明"——那样状态变量仍会吸收瞬态并染色。
    /// 旁通是**逐位恒等**的：`render_track` 根本不调用 `process`。
    #[must_use]
    pub const fn bypass() -> Self {
        Self {
            cutoff_hz: TONE_BYPASS_CUTOFF_HZ,
            resonance: 0.0,
            drive: 0.0,
            bypass: true,
        }
    }

    /// 显式配置音色（构造期调用；非有限值退回该参数的默认值）。
    #[must_use]
    pub fn new(cutoff_hz: f32, resonance: f32, drive: f32) -> Self {
        Self {
            cutoff_hz: if cutoff_hz.is_finite() {
                cutoff_hz
            } else {
                12_000.0
            },
            resonance: if resonance.is_finite() {
                resonance.clamp(0.0, 1.0)
            } else {
                0.0
            },
            drive: if drive.is_finite() {
                drive.clamp(0.0, 1.0)
            } else {
                0.0
            },
            bypass: false,
        }
    }

    /// 从模型层的设备链投影（**控制线程**；见本节文档的投影规则）。
    #[must_use]
    pub fn from_devices(devices: &[DeviceDefinition]) -> Self {
        let mut cutoff = None;
        let mut resonance = None;
        let mut drive = None;
        for device in devices {
            if device.bypassed || device.kind != DeviceKind::InternalInstrument {
                continue;
            }
            let mut device_cutoff = None;
            let mut device_resonance = None;
            let mut device_drive = None;
            for param in &device.params {
                let name = param.name.to_ascii_lowercase();
                match name.as_str() {
                    "cutoff_hz" | "cutoff" => device_cutoff = Some(param.value),
                    "resonance" | "res" => device_resonance = Some(param.value),
                    "drive" => device_drive = Some(param.value),
                    _ => {}
                }
            }
            if device_cutoff.is_none() {
                // 第一条**没有**截止频率的乐器设备不是"音色来源"⇒ 继续找下一条。
                continue;
            }
            cutoff = device_cutoff;
            resonance = device_resonance;
            drive = device_drive;
            break;
        }
        match cutoff {
            Some(cutoff_hz) => Self::new(cutoff_hz, resonance.unwrap_or(0.0), drive.unwrap_or(0.0)),
            None => Self::bypass(),
        }
    }

    /// 是否旁通。
    #[must_use]
    pub const fn is_bypass(&self) -> bool {
        self.bypass
    }

    /// 截止频率 (Hz)。
    #[must_use]
    pub const fn cutoff_hz(&self) -> f32 {
        self.cutoff_hz
    }

    /// 共振（0..1 旋钮值）。
    #[must_use]
    pub const fn resonance(&self) -> f32 {
        self.resonance
    }

    /// 驱动（0..1 旋钮值）。
    #[must_use]
    pub const fn drive(&self) -> f32 {
        self.drive
    }

    /// 在给定采样率下武装一个滤波器（**构造期**：含 `tan`，超越函数类）。
    #[must_use]
    pub fn filter(&self, sample_rate: f32) -> LadderFilter {
        let mut filter = LadderFilter::new();
        filter.configure(sample_rate, self.cutoff_hz, self.resonance, self.drive);
        filter
    }
}

impl Default for ToneParams {
    fn default() -> Self {
        Self::bypass()
    }
}

/// 构造期算好的**一个已调度音符**（实时侧只读）。
///
/// 全部字段在 [`crate::snapshot`] 的投影里由模型算出：实时侧只做整数比较与
/// 逐样本合成，不做 tick 换算、不做音高换算（见模块文档 §1）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduledNote {
    /// 起点（绝对样本位置，从工程 tick 0 起算）。
    start_sample: u64,
    /// 终点（绝对样本位置）；`end_sample > start_sample` 恒成立。
    end_sample: u64,
    /// MIDI 音高（诊断与判据用）。
    pitch: u8,
    /// MIDI 力度（诊断与判据用）。
    velocity: u8,
    /// 回放频率 (Hz)：`note_to_hz(pitch)`，构造期算好（超越函数类，D32 §2）。
    freq_hz: f32,
    /// 整数相位增量：`round(freq_hz / sample_rate × 2³²)`，钳到 `1..=u32::MAX`。
    phase_inc: u32,
    /// 逐样本增益：`velocity/127 × db_to_gain(track.volume_db) × 可闻门`。
    gain: f32,
}

impl ScheduledNote {
    /// 组装一个已调度音符（由 [`crate::snapshot`] 的投影调用）。
    ///
    /// `phase_inc` 在此处由 `freq_hz`/`sample_rate` 推出，而不是由调用方传入：
    /// 相位定标是**合成器的不变量**，不该让每个调用点各自记住 `2³²`。
    #[must_use]
    pub fn new(
        start_sample: u64,
        end_sample: u64,
        pitch: u8,
        velocity: u8,
        freq_hz: f32,
        gain: f32,
        sample_rate: f32,
    ) -> Self {
        Self {
            start_sample,
            end_sample,
            pitch,
            velocity,
            freq_hz,
            phase_inc: phase_increment(freq_hz, sample_rate),
            gain,
        }
    }

    /// 起点（绝对样本位置）。
    #[must_use]
    pub const fn start_sample(&self) -> u64 {
        self.start_sample
    }

    /// 终点（绝对样本位置，开区间）。
    #[must_use]
    pub const fn end_sample(&self) -> u64 {
        self.end_sample
    }

    /// MIDI 音高。
    #[must_use]
    pub const fn pitch(&self) -> u8 {
        self.pitch
    }

    /// MIDI 力度。
    #[must_use]
    pub const fn velocity(&self) -> u8 {
        self.velocity
    }

    /// 回放频率 (Hz)。
    #[must_use]
    pub const fn freq_hz(&self) -> f32 {
        self.freq_hz
    }

    /// 整数相位增量（每样本，`2³²` 定标）。
    #[must_use]
    pub const fn phase_inc(&self) -> u32 {
        self.phase_inc
    }

    /// 逐样本增益。
    #[must_use]
    pub const fn gain(&self) -> f32 {
        self.gain
    }

    /// 本音符的样本跨度（帧）。
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.end_sample.saturating_sub(self.start_sample)
    }
}

/// 相位增量：`freq_hz / sample_rate` 的一周期 = `2³²` 定标。
///
/// - `inc == 0` 会让声部永久停在相位 0（静音）⇒ 钳到下界 1；
/// - 上界钳到 `u32::MAX`（频率 ≥ `sample_rate` 时回绕成"每样本整周期"）；
///   实际调度侧的音高上限是 MIDI 127 ≈ 12.5 kHz。
///
/// 一次 `f64` 乘除 + `round`：这一步**不在**逐样本路径上（构造期一次），
/// `round` 本身是 IEEE 精确类（D32 §1）。
#[must_use]
fn phase_increment(freq_hz: f32, sample_rate: f32) -> u32 {
    let sample_rate = if sample_rate.is_finite() && sample_rate >= 1.0 {
        f64::from(sample_rate)
    } else {
        48_000.0
    };
    let freq_hz = if freq_hz.is_finite() {
        f64::from(freq_hz).max(0.0)
    } else {
        0.0
    };
    let increment = (freq_hz / sample_rate * 4_294_967_296.0).round();
    if !increment.is_finite() || increment < 1.0 {
        return 1;
    }
    if increment >= f64::from(u32::MAX) {
        return u32::MAX;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let increment = increment as u32;
    increment.max(1)
}

/// 单轨的音符调度表（按 `start_sample` 升序，构造期分配、实时侧只读）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoteSchedule {
    notes: Vec<ScheduledNote>,
}

impl NoteSchedule {
    /// 由**已排序**的音符序列构造。
    ///
    /// `notes` 必须按 `start_sample` 升序；[`crate::snapshot`] 的投影负责排序
    /// （实时侧的游标只向前推进，乱序会让音符被永久跳过）。
    #[must_use]
    pub const fn from_sorted(notes: Vec<ScheduledNote>) -> Self {
        Self { notes }
    }

    /// 音符条数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// 是否没有音符。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// 只读音符切片。
    #[must_use]
    pub fn notes(&self) -> &[ScheduledNote] {
        &self.notes
    }

    /// 第一个"可能仍在发声"的音符下标：`end_sample > position` 的下界。
    ///
    /// 快照切换时用它校正游标（见 [`SynthEngine::align_cursors`]）。
    #[must_use]
    pub fn first_sounding_index(&self, position: u64) -> usize {
        self.notes
            .partition_point(|note| note.end_sample <= position)
    }
}

/// 一个声部的全部状态（定长、`Copy`，构造期确定）。
///
/// ## `fade_remaining` / `pending`：3 ms 窃取淡出的两个字段
///
/// [ARCH-RT-004] 要求"窃取瞬间对被终止声部强制应用 3ms 快速指数衰减微淡出"。
/// 本实现把这件事表达成**同一个声部的两段寿命**，而不是另开一个"淡出声部"池
/// （那需要第二份 `[Voice; 16]` ⇒ 固定内存翻倍，且 polyphony 口径要重新定义）：
///
/// ```text
/// fade_remaining > 0, pending = true   ：旧音符按 STEAL_RELEASE_SECONDS 指数淡出
///                                        （phase/inc/gain 仍是旧音符的）
/// fade_remaining 到 0 的那一帧         ：phase = 0、env.reset()+gate_on()、
///                                        inc/gain 换成新音符 ⇒ 从 0 起 attack
/// fade_remaining > 0, pending = false  ：抢占了但**没有**接新音符（seek/回收）
/// ```
///
/// 代价：新音符的起音被推迟 ≤ 3 ms（= `STEAL_RELEASE_SECONDS`），
/// 收益是**不存在**"新音符起音与旧音符淡出"两个瞬态相撞的时刻 —— 那正是爆音的来源。
#[derive(Clone, Copy, Debug)]
struct Voice {
    active: bool,
    /// 整数相位（一周期 = `2³²`）。
    phase: u32,
    /// 每样本相位增量。
    inc: u32,
    /// 波表 mip 级（构造期由频高选出：`Wavetable::level_for`）。
    level: usize,
    /// 逐样本增益。
    gain: f32,
    /// 起点（绝对样本位置）。
    start_sample: u64,
    /// 终点（绝对样本位置）。
    end_sample: u64,
    env: Adsr,
    /// 声部级四极低通（[`ToneParams`]；旁通时**不**被调用）。
    filter: LadderFilter,
    /// 本声部是否处于"窃取淡出"窗口内（>0 表示还剩多少帧）。
    fade_remaining: u32,
    /// 是否有**已挂起的新音符**（淡出走完立刻起音）。
    pending: bool,
    /// 挂起音符的相位增量（`pending` 为真时有效）。
    note_inc: u32,
    /// 挂起音符的逐样本增益（`pending` 为真时有效）。
    note_gain: f32,
    /// 挂起音符的**起始样本位置**（淡出走完的那一帧赋给 `start_sample`）。
    note_start: u64,
}

impl Voice {
    const IDLE: Self = Self {
        active: false,
        phase: 0,
        inc: 1,
        level: 0,
        gain: 0.0,
        start_sample: 0,
        end_sample: 0,
        env: Adsr::new(),
        filter: LadderFilter::new(),
        fade_remaining: 0,
        pending: false,
        note_inc: 1,
        note_gain: 0.0,
        note_start: 0,
    };
}

/// 窃取淡出的默认帧数：`STEAL_RELEASE_SECONDS`（3 ms）× 采样率。
///
/// 上限 0.5 s（`sample_rate` 异常大时也不会把声部卡死几秒），下限 1 帧
/// （0 帧就是硬窃取 —— 那正是本切片要消灭的行为，但**判据要能注入它**，
/// 因此这个值是通过 [`SynthEngine::set_steal_fade_frames`] 可覆盖的）。
#[must_use]
fn steal_fade_frames(sample_rate: f32) -> u32 {
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return 1;
    }
    let frames = (STEAL_RELEASE_SECONDS * sample_rate).round();
    if !frames.is_finite() || frames <= 0.0 {
        return 1;
    }
    if frames >= 0.5 * sample_rate {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let capped = (0.5 * sample_rate) as u32;
        return capped.max(1);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = frames as u32;
    frames.max(1)
}

/// 一个轨道槽：身份 + 游标 + 定长声部池 + 本轨音色参数。
#[derive(Clone, Copy, Debug)]
struct TrackSlot {
    /// 是否已被某条轨道占用（占用后**不释放**：轨道身份与槽位一一绑定，
    /// 这样同一条轨道的声部状态能跨快照连续）。
    assigned: bool,
    /// 本快照里是否存在（不存在 ⇒ 不渲染）。
    present: bool,
    id: EntityId,
    /// 下一个待触发音符的下标（只增不减 ⇒ 绝不重复触发）。
    cursor: usize,
    /// 本轨的音色参数（**构造期**从快照投影进来；渲染路径只读）。
    tone: ToneParams,
    /// 本轨音色在当前采样率下的滤波器模板（系数已在构造期算好，
    /// 触发时按值拷进声部 ⇒ 逐样本路径不含 `tan`）。
    filter: LadderFilter,
    voices: [Voice; VOICES_PER_TRACK],
}

impl TrackSlot {
    /// 空槽（`EntityId::default()` 是 nil，`assigned = false` 时无意义）。
    fn empty() -> Self {
        Self {
            assigned: false,
            present: false,
            id: EntityId::default(),
            cursor: 0,
            tone: ToneParams::bypass(),
            filter: LadderFilter::new(),
            voices: [Voice::IDLE; VOICES_PER_TRACK],
        }
    }
}

/// 合成器：静态预分配声部池 + 可推进的播放头（**实时侧状态的全部**）。
///
/// 播放头（[`SynthEngine::position`]）由引擎自己拥有：`EngineSnapshot` 是
/// **不可变的模型投影**，按 [MODEL-ISO-001] 它不允许携带挥发性走带状态。
/// 每处理一个量子，播放头前进 `frames`；因此"从 tick 0 播放"是本实现的默认语义
/// （走带控制/定位属于后续切片，见 notes 的 pending）。
pub struct SynthEngine {
    /// 内置波表（构造期建好；渲染路径只读）。
    table: Wavetable,
    /// 当前采样率（快照边界更新）。
    sample_rate: f32,
    /// 窃取淡出帧数（默认 3 ms ⇒ @48k = 144 帧）。
    ///
    /// **判据可覆盖**（[`SynthEngine::set_steal_fade_frames`]）：把 0 设进去就回到
    /// "硬窃取"，那条"淡出后无跳变"的判据必须能对它变红。
    steal_fade_frames: u32,
    /// 播放头（绝对样本位置）。
    position: u64,
    /// 包络模板：参数只在采样率变化时重算（`exp` 因此不在逐样本路径上）。
    env_template: Adsr,
    slots: [TrackSlot; MAX_TRACK_SLOTS],
    voice_steals: u64,
    track_drops: u64,
    notes_triggered: u64,
}

impl SynthEngine {
    /// 构造：建好内置波表与包络模板（**允许分配**：这一步在打开设备之前）。
    ///
    /// `sample_rate` 先按传入值武装；真正的采样率在第一次
    /// [`begin_snapshot`](Self::begin_snapshot) 时按快照校准。
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sanitise_sample_rate(sample_rate);
        let mut env_template = Adsr::new();
        env_template.set_sample_rate(sample_rate);
        env_template.set_params(ATTACK_SECONDS, DECAY_SECONDS, SUSTAIN, RELEASE_SECONDS);
        Self {
            table: Wavetable::from_recipe(HOLLOW),
            sample_rate,
            steal_fade_frames: steal_fade_frames(sample_rate),
            position: 0,
            env_template,
            slots: [TrackSlot::empty(); MAX_TRACK_SLOTS],
            voice_steals: 0,
            track_drops: 0,
            notes_triggered: 0,
        }
    }

    /// 当前播放头（绝对样本位置）。
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }

    /// 当前采样率 (Hz)。
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// 池满时的**软窃取**次数：每次窃取都给被终止声部套上 3 ms 淡出
    /// [ARCH-RT-004]。把淡出帧数设为 0（[`Self::set_steal_fade_frames`]）即回到
    /// 旧行为"硬窃取"，但计数口径不变。
    #[must_use]
    pub const fn voice_steals(&self) -> u64 {
        self.voice_steals
    }

    /// 当前窃取淡出帧数（默认 3 ms × 采样率）。
    #[must_use]
    pub const fn steal_fade_frames(&self) -> u32 {
        self.steal_fade_frames
    }

    /// 覆盖窃取淡出帧数（**判据/注入用**；0 = 硬窃取）。
    ///
    /// 产物路径不调用它：帧数由采样率与 [`STEAL_RELEASE_SECONDS`] 决定。
    /// 存在的理由是"淡出后无跳变"这条判据必须能对着**硬窃取**变红 ——
    /// 否则它可能在"根本没窃取"的夹具上永真。
    pub fn set_steal_fade_frames(&mut self, frames: u32) {
        self.steal_fade_frames = frames;
    }

    /// 因 [`MAX_TRACK_SLOTS`] 耗尽而未获槽位的轨道次数。
    #[must_use]
    pub const fn track_drops(&self) -> u64 {
        self.track_drops
    }

    /// 累计触发过的音符数。
    #[must_use]
    pub const fn notes_triggered(&self) -> u64 {
        self.notes_triggered
    }

    /// 声部池的诊断转储（**仅 debug 构建**；`tests/` 判据用来定位"为什么没有窃取"）。
    ///
    /// 返回 `(活跃, end_sample, 包络阶段, 淡出剩余)`。它暴露的是**内部**状态，
    /// 因此只在 `debug_assertions` 下编译 —— 产物路径上没有这个方法，
    /// 也就没有"为了测试把内部状态公开成 API"的负担。
    #[cfg(debug_assertions)]
    #[must_use]
    pub fn debug_voice_state(&self, track: EntityId) -> Vec<(bool, u64, AdsrStage, u32)> {
        self.slots
            .iter()
            .find(|slot| slot.assigned && slot.id == track)
            .map(|slot| {
                slot.voices
                    .iter()
                    .map(|voice| {
                        (
                            voice.active,
                            voice.end_sample,
                            voice.env.stage(),
                            voice.fade_remaining,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 当前活跃声部数（**仅 debug 构建**；判据用来做"池真的满了"的覆盖度自检）。
    #[cfg(debug_assertions)]
    #[must_use]
    pub fn debug_active_voices(&self, track: EntityId) -> usize {
        self.slots
            .iter()
            .find(|slot| slot.assigned && slot.id == track)
            .map(|slot| slot.voices.iter().filter(|voice| voice.active).count())
            .unwrap_or(0)
    }

    /// 内置波表的 mip 级数（诊断用）。
    #[must_use]
    pub fn table_levels(&self) -> usize {
        self.table.level_count()
    }

    /// 跳到某个绝对样本位置（走带 seek 的接入点；当前只有测试与离线渲染用）。
    ///
    /// 释放全部声部并把播放头设到 `position`。**不**重算游标：下一个量子的触发
    /// 循环会把"起点已过"的音符直接消费掉（不触发），因此不需要额外的扫描。
    pub fn seek(&mut self, position: u64) {
        self.position = position;
        for slot in &mut self.slots {
            slot.voices.fill(Voice::IDLE);
        }
    }

    /// 播放头前进 `frames` 帧（每个渲染量子调用一次）。
    pub fn advance(&mut self, frames: usize) {
        self.position = self.position.saturating_add(frames as u64);
    }

    /// 快照边界：把轨道槽对齐到新快照的轨道集合，并校准采样率。
    ///
    /// 规则（四条，缺一不可）：
    ///
    /// 1. 新快照里存在的轨道 ⇒ 槽位 `present = true`（首次出现则占用一个空槽）；
    /// 2. 新快照里不存在 ⇒ `present = false`（游标与声部保留，等它回来时连续）；
    /// 3. 槽位耗尽 ⇒ 计数、不分配（不 panic、不扩容）；
    /// 4. **音色参数按新快照刷新**：`tones` 给出每轨的 [`ToneParams`]，
    ///    滤波器系数在此处（构造期语义）按当前采样率算好，逐样本路径不再出现 `tan`。
    ///    不在 `tones` 里的轨道退回旁通（`tone` 变了才重算系数）。
    ///
    /// 采样率变化时重算包络模板与全部轨道槽的滤波器系数（`exp`/`tan` 因此落在
    /// "每修订一次"而不是"每样本"）。
    pub fn begin_snapshot<'a, I, T>(&mut self, sample_rate: u32, tracks: I, tones: T)
    where
        I: IntoIterator<Item = &'a EntityId>,
        T: IntoIterator<Item = (&'a EntityId, &'a ToneParams)>,
    {
        let sample_rate = sanitise_sample_rate(sample_rate);
        let sample_rate_changed = sample_rate != self.sample_rate;
        if sample_rate_changed {
            self.sample_rate = sample_rate;
            self.steal_fade_frames = steal_fade_frames(sample_rate);
            self.env_template.set_sample_rate(sample_rate);
            self.env_template
                .set_params(ATTACK_SECONDS, DECAY_SECONDS, SUSTAIN, RELEASE_SECONDS);
        }

        // 先把本快照的每轨音色收进一个**定长数组**（栈上，无分配）：
        // 轨道槽最多 MAX_TRACK_SLOTS 个，因此"查表"永远不需要分配。
        let mut wanted: [(EntityId, ToneParams); MAX_TRACK_SLOTS] =
            [(EntityId::default(), ToneParams::bypass()); MAX_TRACK_SLOTS];
        let mut wanted_len = 0usize;
        for (id, tone) in tones {
            if wanted_len >= MAX_TRACK_SLOTS {
                break;
            }
            wanted[wanted_len] = (*id, *tone);
            wanted_len += 1;
        }

        // ⚠ 顺序是**判据的一部分**：先按 `tracks` 完成槽位分配（`slot.id` 写进去），
        // 再按 `slot.id` 配音色。第一版把音色循环写在分配循环**之前**，
        // 于是查表用的是**旧 id**：新占用的槽位永远匹配不到自己的音色，
        // 一路拿 `ToneParams::bypass()` ⇒ 滤波器**从未生效**
        //（判据 `a_tone_above_the_cutoff_is_attenuated` 实测衰减 0.00 dB 抓到了它）。
        for slot in &mut self.slots {
            slot.present = false;
        }
        for track in tracks {
            if let Some(slot) = self
                .slots
                .iter_mut()
                .find(|slot| slot.assigned && slot.id == *track)
            {
                slot.present = true;
                continue;
            }
            let free = self.slots.iter().position(|slot| !slot.assigned);
            match free {
                Some(index) => {
                    self.slots[index] = TrackSlot::empty();
                    self.slots[index].assigned = true;
                    self.slots[index].present = true;
                    self.slots[index].id = *track;
                }
                None => self.track_drops = self.track_drops.saturating_add(1),
            }
        }

        for slot in &mut self.slots {
            let tone = wanted[..wanted_len]
                .iter()
                .find(|(id, _)| *id == slot.id)
                .map(|(_, tone)| *tone)
                .unwrap_or_else(ToneParams::bypass);
            if sample_rate_changed || tone != slot.tone {
                slot.tone = tone;
                // 系数在**构造期**算（`tan`，超越函数类）；旁通时系数无意义但仍算一份，
                // 让"参数 → 系数"只有一条路径。
                slot.filter = tone.filter(sample_rate);
            }
        }
    }

    /// 快照边界处的游标校正 + 声部回收（需要调度表，故与
    /// [`begin_snapshot`](Self::begin_snapshot) 分开：调度表的迭代由调用方给）。
    ///
    /// - 游标**只增不减**地校正到 `partition_point(end_sample <= position)`：
    ///   调度表变化导致游标落后时补上，已经触发过的音符**绝不重复触发**；
    /// - 已过 `end_sample` 且包络已静音的声部立即回收；仍在释放段的声部**保留**
    ///   —— 快照切换不得切断在鸣的音符。
    pub fn align_cursors<'a, I>(&mut self, schedules: I)
    where
        I: IntoIterator<Item = (&'a EntityId, &'a NoteSchedule)>,
    {
        let position = self.position;
        for (id, schedule) in schedules {
            let Some(slot) = self
                .slots
                .iter_mut()
                .find(|slot| slot.assigned && slot.present && slot.id == *id)
            else {
                continue;
            };
            slot.cursor = slot.cursor.max(schedule.first_sounding_index(position));
            for voice in &mut slot.voices {
                if voice.active && voice.end_sample <= position && !voice.env.is_active() {
                    *voice = Voice::IDLE;
                }
            }
        }
    }

    /// 渲染一条轨道的本量子输出（单声道、声相之前）。**实时路径**。
    ///
    /// `schedule` 为 `None`（本轨没有调度表）时输出静音但不 panic；
    /// `out` 的长度就是本量子的有效帧数（尾块可以小于
    /// [`crate::block::DEFAULT_BLOCK_FRAMES`]）。
    pub fn render_track(
        &mut self,
        track: EntityId,
        schedule: Option<&NoteSchedule>,
        out: &mut [f32],
    ) {
        out.fill(0.0);
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.assigned && slot.id == track)
        else {
            return;
        };
        if !self.slots[index].present {
            return;
        }

        let position = self.position;
        let end = position.saturating_add(out.len() as u64);
        let fade_frames = self.steal_fade_frames;
        let tone_filter = self.slots[index].filter;
        let tone_bypass = self.slots[index].tone.is_bypass();
        {
            let Self {
                table,
                sample_rate,
                slots,
                voice_steals,
                notes_triggered,
                ..
            } = self;
            let slot = &mut slots[index];

            // --- 1) 触发本量子窗口内起音、且尚未结束的音符 ---
            // 游标只增不减 ⇒ 已触发过的音符永远不会被再次触发。
            if let Some(schedule) = schedule {
                let notes = schedule.notes();
                while let Some(note) = notes.get(slot.cursor) {
                    if note.start_sample >= end {
                        break;
                    }
                    if note.end_sample > position {
                        trigger(
                            slot,
                            note,
                            *sample_rate,
                            table,
                            tone_filter,
                            fade_frames,
                            voice_steals,
                        );
                        *notes_triggered = notes_triggered.saturating_add(1);
                    }
                    slot.cursor += 1;
                }
            }

            if !slot.voices.iter().any(|voice| voice.active) {
                return;
            }

            // --- 2) 逐样本合成（只有 IEEE 精确类运算，见模块文档 §2）---
            for (frame, output) in out.iter_mut().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                let now = position + frame as u64;
                let mut accumulator = 0.0f32;
                for voice in &mut slot.voices {
                    if !voice.active || now < voice.start_sample {
                        continue;
                    }
                    // --- 2a) 窃取淡出：旧音符指数淡出，期满换成挂起的新音符 ---
                    //
                    // 输出的是**旧音符**的样本 × 旧增益 × 正在下降的包络
                    //（`gate = false` ⇒ `Adsr` 走 Release 段，系数已在
                    // `start_steal_fade` 里覆盖成 3 ms）。新音符的值只在
                    // `fade_remaining` 归零的那一帧启用。
                    if voice.fade_remaining > 0 {
                        voice.fade_remaining -= 1;
                        let envelope = voice.env.process(false);
                        if voice.fade_remaining == 0 && voice.pending {
                            // 新音符从 0 起 attack：相位归零、起始位置改为当前帧、
                            // 包络 reset + gate_on。新音符**从这一帧**开始发声，
                            // 而旧音符在上一帧已经衰减到 ≈0（`Adsr` 在 < 1e-4 时归零）
                            // ⇒ 中间不存在"两个波形的和"。
                            voice.pending = false;
                            voice.phase = 0;
                            voice.inc = voice.note_inc;
                            voice.gain = voice.note_gain;
                            voice.start_sample = voice.note_start.max(now);
                            voice.env.reset();
                            voice.env.gate_on();
                        } else {
                            let samples = table.level_samples(voice.level);
                            let len = samples.len();
                            let scaled = u64::from(voice.phase) * len as u64;
                            let index = (scaled >> 32) as usize;
                            let first = samples[index];
                            let next = index + 1;
                            let second = samples[if next == len { 0 } else { next }];
                            let fraction =
                                ((scaled & 0xFFFF_FFFF) >> (32 - FRAC_BITS)) as f32 * FRAC_SCALE;
                            let mut sample =
                                (first + (second - first) * fraction) * envelope * voice.gain;
                            if !tone_bypass {
                                sample = voice.filter.process(sample);
                            }
                            accumulator += sample;
                            voice.phase = voice.phase.wrapping_add(voice.inc);
                            if !voice.env.is_active() {
                                *voice = Voice::IDLE;
                            }
                            continue;
                        }
                    }
                    let gate = now < voice.end_sample;
                    let envelope = voice.env.process(gate);
                    if !voice.env.is_active() {
                        *voice = Voice::IDLE;
                        continue;
                    }
                    let samples = table.level_samples(voice.level);
                    let len = samples.len();
                    // `phase < 2³²` ⇒ `index < len`：整数乘 + 右移代替除法，
                    // 且对任意 `len`（不要求 2 的幂）都正确。
                    let scaled = u64::from(voice.phase) * len as u64;
                    let index = (scaled >> 32) as usize;
                    let first = samples[index];
                    let next = index + 1;
                    // 末端回绕用一次比较代替取模（整数, 精确）。
                    let second = samples[if next == len { 0 } else { next }];
                    let fraction = ((scaled & 0xFFFF_FFFF) >> (32 - FRAC_BITS)) as f32 * FRAC_SCALE;
                    let mut sample = (first + (second - first) * fraction) * envelope * voice.gain;
                    // 声部级低通。**旁通时一次也不调用** ⇒ 逐位恒等
                    // （不是"系数取成透明"，那样状态仍会吸收瞬态）。
                    if !tone_bypass {
                        sample = voice.filter.process(sample);
                    }
                    accumulator += sample;
                    voice.phase = voice.phase.wrapping_add(voice.inc);
                }
                *output = accumulator;
            }
        }
    }
}

/// 触发一个音符：分配声部；池满时按 [ARCH-RT-004] **软窃取**。
///
/// 包络从池里的模板拷贝（参数只在采样率变化时重算），因此这里有 `exp` 的调用点
/// 只有"采样率变化"那一处 —— 逐样本路径不含超越函数（模块文档 §2）。
///
/// ## 窃取算法的三条口径（都是确定性的）
///
/// 1. **优先级**（[ARCH-RT-004] 原文："优先窃取处于 Release 阶段尾部、振幅能量
///    最低（< −60 dBFS）或最早被触发的声音"）：
///    包络已进入 `Release` 段**或**当前电平 < −60 dBFS（`0.001`）的声部优先；
///    同档内取 `start_sample` **最小**者（最早触发）；仍然并列 ⇒ 取**下标最小**者。
///    三级比较合起来是全序 ⇒ 任何平台/编译器下选出同一个声部。
/// 2. **淡出**：被窃取声部进入 `fade_remaining = steal_fade_frames` 帧的指数淡出
///    （默认 3 ms），新音符**挂起**在同一槽位上，淡出走完立刻从 0 起 attack。
///    池满时"旧声部淡出"与"新音符起音"因此**永不同时发声** ⇒ 不可能叠加爆音。
/// 3. **`fade_frames == 0` 退化为硬窃取**（判据注入用）：不动旧声部，直接覆盖。
///    这时输出会出现"旧波形硬切到新波形"的样本间跃变 ——
///    判据 `steal_fade_bounds_the_sample_step` 正是对着这个注入变红的。
#[allow(clippy::too_many_arguments)]
fn trigger(
    slot: &mut TrackSlot,
    note: &ScheduledNote,
    sample_rate: f32,
    table: &Wavetable,
    tone_filter: LadderFilter,
    fade_frames: u32,
    voice_steals: &mut u64,
) {
    let level = table.level_for(note.freq_hz, sample_rate);
    let fill = |voice: &mut Voice| {
        let mut env = Adsr::new();
        env.set_sample_rate(sample_rate);
        env.set_params(ATTACK_SECONDS, DECAY_SECONDS, SUSTAIN, RELEASE_SECONDS);
        env.reset();
        env.gate_on();
        *voice = Voice {
            active: true,
            phase: 0,
            inc: note.phase_inc,
            level,
            gain: note.gain,
            start_sample: note.start_sample,
            end_sample: note.end_sample,
            env,
            filter: tone_filter,
            fade_remaining: 0,
            pending: false,
            note_inc: note.phase_inc,
            note_gain: note.gain,
            note_start: note.start_sample,
        };
    };

    if let Some(index) = slot.voices.iter().position(|voice| !voice.active) {
        fill(&mut slot.voices[index]);
        return;
    }

    // 池满：选一个被终止者（见本节文档 §1）。
    let mut best = 0usize;
    for (index, voice) in slot.voices.iter().enumerate() {
        let candidate = (steal_priority(voice), voice.start_sample, index);
        let current = (
            steal_priority(&slot.voices[best]),
            slot.voices[best].start_sample,
            best,
        );
        if candidate < current {
            best = index;
        }
    }
    *voice_steals = voice_steals.saturating_add(1);

    let victim = &mut slot.voices[best];
    if fade_frames == 0 {
        // 硬窃取（注入路径 / 显式配置）：直接覆盖，不留淡出。
        fill(victim);
        return;
    }
    if victim.active && victim.fade_remaining == 0 {
        // 先把旧声部推进淡出（`start_steal_fade` 覆盖 release 为 3 ms 并 gate_off）。
        victim.env.start_steal_fade();
        victim.fade_remaining = fade_frames;
        // 相位/增益/终点仍是**旧音符**的（淡出的是旧声音）。
    }
    // 挂起新音符：淡出走完的那一帧换成它（起音从 0 开始）。
    //
    // ⚠ **不要**在这里改 `gain`/`start_sample`/`inc`：淡出期间的输出是"旧音符的样本
    // × 旧音符的增益 × 正在下降的包络"。第一版在这里顺手把它们换成了新音符的值
    // （`gain = note.gain`、`end_sample = note.end_sample`），再加上 `env.reset()`
    // 把包络电平清零 ⇒ 被窃取声部**瞬间静音**，3 ms 淡出等于没做
    // （实测：软窃取与硬窃取的输出只差 1.97，且在触发当帧就分叉）。
    // 新音符的值先寄存在 `note_*` 字段里，淡出走完的那一帧再启用。
    victim.pending = true;
    victim.note_inc = note.phase_inc;
    victim.note_gain = note.gain;
    victim.note_start = note.start_sample;
    victim.end_sample = note.end_sample;
    victim.level = level;
}

/// 窃取优先级：`0` = 正在释放（或已低于 −60 dBFS），`1` = 仍在持续发声。
///
/// 数字小者优先被窃取。−60 dBFS ≈ `0.001` 是规范原文给的阈值。
#[must_use]
fn steal_priority(voice: &Voice) -> u8 {
    let releasing = matches!(voice.env.stage(), AdsrStage::Release);
    if releasing || voice.env.value() < 0.001 {
        0
    } else {
        1
    }
}

/// 采样率下限钳制（与 `yeban_dsp::MIN_SAMPLE_RATE` 同口径；那个常量不是公共 API）。
#[must_use]
fn sanitise_sample_rate(sample_rate: u32) -> f32 {
    let value = sample_rate as f32;
    if value.is_finite() && value >= MIN_SAMPLE_RATE {
        value
    } else {
        MIN_SAMPLE_RATE
    }
}

/// tick → 样本位置的一次性换算（**构造期**，不在实时路径上）。
///
/// `samples_per_tick = 60 × sample_rate / (bpm × PPQ)`。
/// 乘/除/`round` 都是 IEEE-754 精确类运算 ⇒ 同一 `f64` 输入在任何架构上给出
/// **同一**样本位置（D32 §1）。
///
/// 非有限/非正值输入返回 `None`（调用方跳过该音符，绝不 panic）。
#[must_use]
pub fn tick_to_sample(tick: u64, samples_per_tick: f64) -> Option<u64> {
    if !samples_per_tick.is_finite() || samples_per_tick <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let samples = (tick as f64 * samples_per_tick).round();
    if !samples.is_finite() || samples < 0.0 {
        return None;
    }
    if samples >= (u64::MAX as f64) {
        return Some(u64::MAX);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let samples = samples as u64;
    Some(samples)
}

/// 每 tick 的样本数（`bpm` 非法时退回 120 BPM；`sample_rate == 0` 时退回 48 kHz）。
#[must_use]
pub fn samples_per_tick(bpm: f64, sample_rate: u32) -> f64 {
    let bpm = if bpm.is_finite() && bpm > 0.0 {
        bpm
    } else {
        120.0
    };
    let sample_rate = if sample_rate == 0 {
        48_000
    } else {
        sample_rate
    };
    #[allow(clippy::cast_precision_loss)]
    let ppq = PPQ as f64;
    60.0 * f64::from(sample_rate) / (bpm * ppq)
}

/// 线性增益：`velocity / 127`（IEEE 精确类除法）。
#[must_use]
pub fn velocity_gain(velocity: u8) -> f32 {
    f32::from(velocity.min(127)) / 127.0
}

/// 音轨可闻门：静音，或"有轨道独奏而自己既不独奏也不独奏安全"时为 `false`。
#[must_use]
pub fn track_is_audible(mute: bool, solo: bool, solo_safe: bool, any_solo: bool) -> bool {
    !mute && (!any_solo || solo || solo_safe)
}

/// 音轨线性增益（dB → 线性；非有限输入按静音处理）。超越函数类，构造期一次。
#[must_use]
pub fn track_gain(volume_db: f32) -> f32 {
    if volume_db.is_finite() {
        db_to_gain(volume_db)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: u64, end: u64, pitch: u8, velocity: u8, sample_rate: f32) -> ScheduledNote {
        let freq = yeban_dsp::math::note_to_hz(f32::from(pitch));
        ScheduledNote::new(
            start,
            end,
            pitch,
            velocity,
            freq,
            velocity_gain(velocity),
            sample_rate,
        )
    }

    fn rig(id: EntityId) -> SynthEngine {
        let mut engine = SynthEngine::new(48_000);
        engine.begin_snapshot(48_000, &[id], []);
        engine
    }

    fn render(
        engine: &mut SynthEngine,
        id: EntityId,
        schedule: &NoteSchedule,
        frames: usize,
    ) -> Vec<f32> {
        let mut out = vec![0.0f32; frames];
        engine.render_track(id, Some(schedule), &mut out);
        engine.advance(frames);
        out
    }

    #[test]
    fn phase_increment_has_a_nonzero_floor_and_covers_a_full_cycle() {
        assert_eq!(phase_increment(0.0, 48_000.0), 1, "0 Hz 被钳到 1 LSB");
        assert_eq!(phase_increment(-5.0, 48_000.0), 1);
        assert_eq!(phase_increment(f32::NAN, 48_000.0), 1);
        assert_eq!(phase_increment(48_000.0, 48_000.0), u32::MAX, "饱和上界");
        assert_eq!(phase_increment(24_000.0, 48_000.0), 2_147_483_648);
    }

    #[test]
    fn tick_to_sample_is_exact_for_the_canonical_grid() {
        // 120 BPM / 48 kHz / 960 PPQ ⇒ 25 样本每 tick
        let spt = samples_per_tick(120.0, 48_000);
        assert_eq!(spt, 25.0);
        assert_eq!(tick_to_sample(960, spt), Some(24_000));
        assert_eq!(tick_to_sample(u64::MAX, f64::NAN), None);
        assert_eq!(tick_to_sample(10, 0.0), None);
    }

    /// 判据：同输入两次渲染**逐位**相同（L1，模块文档 §2）。
    #[test]
    fn same_input_renders_byte_identical() {
        let id = EntityId::new();
        let schedule = NoteSchedule::from_sorted(vec![note(100, 5_000, 69, 100, 48_000.0)]);
        let mut a = rig(id);
        let mut b = rig(id);
        let mut rendered_a = Vec::new();
        let mut rendered_b = Vec::new();
        for _ in 0..64 {
            rendered_a.extend(render(&mut a, id, &schedule, 128));
            rendered_b.extend(render(&mut b, id, &schedule, 128));
        }
        assert!(rendered_a.iter().any(|s| *s != 0.0), "必须真的出声");
        let bits_a: Vec<u32> = rendered_a.iter().map(|s| s.to_bits()).collect();
        let bits_b: Vec<u32> = rendered_b.iter().map(|s| s.to_bits()).collect();
        assert_eq!(bits_a, bits_b, "同输入两次渲染必须逐位相同");
    }

    /// 判据：一个音符的**起点**对应的样本位置误差 ≤ 1 帧；终点之后包络走完归零。
    #[test]
    fn note_bounds_land_inside_the_scheduled_frame() {
        let id = EntityId::new();
        let start = 300u64;
        let end = 700u64;
        let schedule = NoteSchedule::from_sorted(vec![note(start, end, 69, 127, 48_000.0)]);
        let mut engine = rig(id);
        let mut rendered = Vec::new();
        // 100 个量子 = 12,800 帧 > 终点 + 释放上界（见下面的时间常数推导）
        for _ in 0..100 {
            rendered.extend(render(&mut engine, id, &schedule, 128));
        }
        assert!(
            rendered[..start as usize].iter().all(|s| *s == 0.0),
            "起点之前必须完全静音"
        );
        assert!(
            rendered[start as usize..(start + 128) as usize]
                .iter()
                .any(|s| *s != 0.0),
            "起点必须落在同一个量子内"
        );
        // 释放上界：`Adsr` 的 release 是一极点，系数让 `release_s` 走完 **6 个时间常数**
        // （0.05 s @48k = 2400 帧 ⇒ τ = 400 帧），并在包络值 < 1e-4 时归零。
        // 从 sustain 0.7 起算需要 ln(0.7/1e-4) ≈ 8.85 τ ⇒ 取 **10 τ** 作为上界。
        let release_tau = (RELEASE_SECONDS * 48_000.0 / 6.0) as usize;
        let tail = end as usize + 10 * release_tau + 128;
        assert!(
            rendered[tail..].iter().all(|s| *s == 0.0),
            "释放走完之后必须归零"
        );
    }

    /// 判据：力度 0 不发声；力度越大有效值越大（严格单调）。
    #[test]
    fn velocity_zero_is_silent_and_loudness_is_monotonic() {
        let id = EntityId::new();
        let mut rms = Vec::new();
        for velocity in [0u8, 1, 32, 64, 127] {
            let schedule = NoteSchedule::from_sorted(vec![note(0, 4_800, 69, velocity, 48_000.0)]);
            let mut engine = rig(id);
            let mut rendered = Vec::new();
            for _ in 0..40 {
                rendered.extend(render(&mut engine, id, &schedule, 128));
            }
            let energy: f64 = rendered.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
            rms.push((energy / rendered.len() as f64).sqrt());
        }
        assert_eq!(rms[0], 0.0, "力度 0 必须完全静音");
        for pair in rms.windows(2) {
            assert!(pair[1] > pair[0], "有效值必须随力度严格单调: {rms:?}");
        }
    }

    /// 判据：相位是整数递推 ⇒ 升八度让零交叉数翻倍（音高正确）。
    #[test]
    fn octave_doubles_the_zero_crossing_rate() {
        fn crossings(pitch: u8) -> usize {
            let id = EntityId::new();
            let schedule = NoteSchedule::from_sorted(vec![note(0, 96_000, pitch, 127, 48_000.0)]);
            let mut engine = rig(id);
            let mut rendered = Vec::new();
            // 375 个量子 = 48,000 帧 = **恰好** 1 秒
            for _ in 0..375 {
                rendered.extend(render(&mut engine, id, &schedule, 128));
            }
            rendered
                .windows(2)
                .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
                .count()
        }
        let low = crossings(69); // A4 = 440 Hz
        let high = crossings(81); // A5 = 880 Hz
        // 1 秒窗口内一个周期 2 次零交叉 ⇒ 440 Hz ≈ 880 次。
        // ±3 的绝对容差只为吸收窗口边界（首帧相位恰好为 0 ⇒ 第一次穿越不被计数）。
        assert!(
            (low as i64 - 880).abs() <= 3,
            "A4 在 1 秒内的零交叉数应 ≈ 880, 实际 {low}"
        );
        assert!(
            (high as i64 - 2 * low as i64).abs() <= 3,
            "升八度必须让零交叉数翻倍: A4={low} A5={high}"
        );
    }

    /// 判据：轨道槽耗尽不 panic，只计数。
    #[test]
    fn slot_exhaustion_is_counted_and_never_panics() {
        let mut engine = SynthEngine::new(48_000);
        let ids: Vec<EntityId> = (0..MAX_TRACK_SLOTS + 3).map(|_| EntityId::new()).collect();
        engine.begin_snapshot(48_000, &ids, []);
        assert_eq!(engine.track_drops(), 3);
        assert_eq!(engine.table_levels(), yeban_dsp::oscillator::LEVELS);
    }

    /// 判据：声部池满时**硬窃取**被计数，且仍然逐样本确定。
    #[test]
    fn voice_stealing_is_counted_and_deterministic() {
        let id = EntityId::new();
        let notes: Vec<ScheduledNote> = (0..VOICES_PER_TRACK + 4)
            .map(|index| note((index * 8) as u64, 40_000, 60, 127, 48_000.0))
            .collect();
        let schedule = NoteSchedule::from_sorted(notes);
        let mut engine = rig(id);
        let mut rendered = Vec::new();
        for _ in 0..8 {
            rendered.extend(render(&mut engine, id, &schedule, 128));
        }
        assert_eq!(engine.voice_steals(), 4, "超出复音上限的音符必须走窃取");
        assert!(rendered.iter().any(|s| *s != 0.0));
    }

    /// 判据：没有槽位/不在快照里的轨道渲染静音且不 panic。
    #[test]
    fn unknown_track_renders_silence() {
        let mut engine = SynthEngine::new(48_000);
        let mut out = [1.0f32; 128];
        engine.render_track(EntityId::new(), None, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(engine.position(), 0, "渲染不推进播放头（由 advance 负责）");
    }

    /// 判据：快照切换**不重触发**已消费的音符，也不切断在鸣的音符。
    #[test]
    fn snapshot_realign_never_retriggers_consumed_notes() {
        let id = EntityId::new();
        let schedule = NoteSchedule::from_sorted(vec![note(0, 8_000, 69, 127, 48_000.0)]);

        // 参照：不停机连续渲染 32 个量子。
        let mut continuous = rig(id);
        let mut expected = Vec::new();
        for _ in 0..32 {
            expected.extend(render(&mut continuous, id, &schedule, 128));
        }
        assert_eq!(continuous.notes_triggered(), 1);

        // 被测：第 16 个量子之后重新武装同一份快照。
        let mut switched = rig(id);
        let mut actual = Vec::new();
        for quantum in 0..32 {
            if quantum == 16 {
                switched.begin_snapshot(48_000, &[id], []);
                switched.align_cursors(std::iter::once((&id, &schedule)));
            }
            actual.extend(render(&mut switched, id, &schedule, 128));
        }
        assert_eq!(switched.notes_triggered(), 1, "起音只允许发生一次");
        let expected_bits: Vec<u32> = expected.iter().map(|s| s.to_bits()).collect();
        let actual_bits: Vec<u32> = actual.iter().map(|s| s.to_bits()).collect();
        assert_eq!(
            expected_bits, actual_bits,
            "等价快照的重新武装不得改变任何一个样本位"
        );
    }

    /// 判据：可闻门（静音/独奏）语义。
    #[test]
    fn audibility_gate_is_mute_and_solo_correct() {
        assert!(track_is_audible(false, false, false, false));
        assert!(!track_is_audible(true, false, false, false), "静音轨不可闻");
        assert!(
            !track_is_audible(false, false, false, true),
            "有独奏时非独奏轨不可闻"
        );
        assert!(track_is_audible(false, true, false, true), "独奏轨可闻");
        assert!(track_is_audible(false, false, true, true), "独奏安全轨可闻");
    }
}
