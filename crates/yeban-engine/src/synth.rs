//! 音符调度与逐轨合成装配：**让渲染量子真的出声**
//! [ARCH-RT-001, ARCH-RT-004, ARCH-DET-001, ROAD-M2-005, ROAD-M2-006]。
//!
//! ## 0. 本模块现在只有**调度**；声部层已上移到 `yeban-dsp`
//!
//! "减法合成器 ➔ `crates/yeban-dsp/src/polysynth.rs`"
//! （`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:111`）之后，本模块保留的是
//! **引擎独占**的那部分：tick → 样本的构造期换算、`NoteSchedule` 的游标、
//! 轨道槽身份、模型侧音色参数的投影。声部池、双振荡器、整数相位波表、
//! 声部级梯形低通、ADSR、[ARCH-RT-004] 的确定性窃取与 3 ms 淡出**全部**
//! 只有一份实现：[`yeban_dsp::polysynth::PolySynth`]（本文件 `pub use` 再导出，
//! 与 `mixer.rs` 上移限制器后同形）。逐位一致的证据见该模块 §0 与
//! `crates/yeban-engine/tests/synth_render.rs` / `steal_fade.rs` / `synth_filter.rs`
//!（这三个文件的期望值在本票中**一个字都没有改**）。
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
//!         │  RT: 游标触发 → PolySynth::note_on（dsp 器件）
//!         ▼
//!   SynthEngine::render_track ──► PolySynth::render ──► [f32; 128] ──► 电平 ──► 母线
//! ```
//!
//! ## 0.1 本票追加：轨道的**音源**可以是鼓机（`crate::drums`）
//!
//! `line/engine-drums` 之后，一条轨的音源由快照决定：
//!
//! - **默认**：复音合成器（上面的流程，一个字没改）；
//! - **识别出鼓机**（[`crate::drums::DrumsParams::from_devices`] 返回 `Some`）：
//!   同一张音符调度表改成驱动 `yeban_dsp::drums` —— 游标逻辑、窗口判定与增益
//!   完全一样，只有"音符 → 什么器件"这一步不同：
//!
//! ```text
//!   ScheduledNote { start_sample, pitch, gain }
//!         │  RT: 游标 → DrumNoteMap::voice_for(pitch) → DrumMachine::trigger
//!         ▼
//!   SynthEngine::render_track ──► DrumMachine::render(position) ──► [f32; 128] ──► 电平 ──► 母线
//! ```
//!
//! ⚠ **一条轨至多一件音源**：识别出鼓机时该槽的复音合成器**不渲染**
//! （理由与裁决见 [`crate::drums`] 模块文档 §4.1）。识别不出来 ⇒ 走复音合成器
//! ⇒ 既有工程逐位不变（实测读数见同一处 §6）。
//!
//! ## 1. 为什么"音符 → 样本位置"必须在构造期算完
//!
//! [ARCH-DET-001] 的 L1 契约要求同输入逐位相同。tick → sample 的换算含 `bpm`
//! （`f64`）与 960 PPQ，若放在实时侧逐音符算，就等于把"音频线程的浮点除法顺序"
//! 变成输出的一部分。现在这一步在 [`crate::snapshot`] 的投影里做一次，实时侧只看
//! **整数样本位置**，于是"同一个工程 → 同一串样本位置"是构造性的。
//!
//! ## 2. D32 分类：本模块在逐样本路径上只剩"转发"
//!
//! [docs/adr/ADR-0001 D32] 要求按运算类别分策。上移之后，[`SynthEngine::render_track`]
//! 的逐样本部分**一次浮点运算都没有**：它把 `(tables, position, out)` 交给器件
//! （逐样本运算的类别表见 `yeban_dsp::polysynth` 模块文档 §4）。
//!
//! 本模块仍然拥有的算术是**构造期**的（每音符一次，不在逐样本路径上）：
//!
//! | 运算 | 位置 | 类别 |
//! | :--- | :--- | :--- |
//! | `phase_increment(freq, sr)` | [`ScheduledNote::new`]（构造期） | IEEE 精确类（`f64` 乘除 + `round`） |
//! | `tick × samples_per_tick`（`round`） | [`crate::snapshot`] 的投影 | IEEE 精确类 |
//! | `Adsr::set_params` / `LadderFilter::configure` | 快照边界（每修订一次） | **超越函数类**（`exp`/`tan`） |
//! | `note_to_hz` / `db_to_gain` | 控制线程 | **超越函数类**（`2^x`） |
//!
//! 因此 L1 逐位相同的**强度**是：同一架构同一工具链下整条合成链逐位相同
//! （由判据 `same_input_renders_byte_identical` 锁住）；跨架构下逐样本运算是位精确的，
//! 只有"音高、增益、包络/滤波系数"这几个标量落在 D32 的超越函数预算内。
//!
//! ## 3. 相位推进为什么用整数
//!
//! `phase: u32` 是"一个周期 = 2³²"的定点相位，`inc` 由
//! `round(freq_hz / sample_rate × 2³²)` 得到。整数回绕加法没有累积误差，
//! 也不受 FTZ/DAZ 影响 —— 浮点相位累加会在长音符上缓慢漂移，而漂移量取决于
//! 块切分与累加次数，那正是**非确定性**的来源。`inc == 0`（极低频）被钳到 1，
//! 于是最低可表达频率是 `sample_rate / 2³² ≈ 1.1e-5 Hz @48k`。
//! 实现已随器件上移到 `yeban_dsp::polysynth::phase_increment`。
//!
//! ## 4. 边界（本模块**没有**做的）
//!
//! - **波形与两条支路的电平/失谐已可选**：`ToneParams` 投影了 `osc1_wave` / `osc2_wave` /
//!   `osc1_level` / `osc1_detune_cents` / `osc2_level` / `osc2_detune_cents` 六个参数
//!   （下标 0 恒等于历史默认的 [`HOLLOW`]；电平默认"一条满、一条关"，失谐默认 `0`）⇒
//!   器件的**双振荡器**两条支路都到得了声部；仍缺参数名的**模型层规范**
//!   （`DeviceDefinition::params` 是字符串键值对）——见 [`ToneParams`] 的文档与 needs N5；
//! - **没有循环片段展开**：`ClipPlacement::loop_config` 目前被忽略，一个摆放只播一遍；
//!   坐标语义（clip 局部 vs 时间轴）在规范里没有定义，见
//!   `docs/ledger/engine-sound-notes.md` 的 needs；
//! - **没有滑音/弯音/歌词/音素**：`MidiNote::slide`/`pitch_bend_curve`/`phonemes`
//!   不参与合成；
//! - **没有采样播放/SFZ**：`ClipContent::Audio` 与 `yeban-sfz` 的乐器都还没有接到
//!   声部池上（需要采样加载 + 重采样，见 notes 的 pending）；
//! - **窃取淡出是 3 ms 指数淡出，不是 [ARCH-DSP-001] 的 5 ms 升余弦窗**：
//!   这是**上移前就有**的差异，且 `tests/steal_fade.rs` 的 S4 把 144 帧
//!   （3 ms @48 kHz）写成了既有期望值 ⇒ 本票不改，登记在
//!   `yeban_dsp::polysynth` 模块文档 §5。
//!
//! ## 5. 实时侧禁令自检（[AGENTS.md §2 红线 7]）
//!
//! [`SynthEngine`] 的**全部**状态都是定长数组与标量：波形库是构造期建好的 `Vec`
//! （`render` 只读），每轨的声部池在器件里是 `[PolyVoice; VOICES_PER_TRACK]`。
//! `render_track` 里没有 `Vec::push`/`Box::new`/`format!`/`println!`/`Mutex::lock`/
//! 文件或网络调用。运行期由 `tests/synth_rt_zero_alloc.rs`（计数型全局分配器，
//! `harness = false`）钉住；器件自身另有一条
//! `crates/yeban-dsp/tests/polysynth_rt_zero_alloc.rs`。

use yeban_dsp::envelope::AdsrStage;
use yeban_dsp::filter::LadderFilter;
use yeban_dsp::math::db_to_gain;
use yeban_dsp::oscillator::{GLASS, HOLLOW, METALLIC, ORGAN, VOCAL};
use yeban_dsp::polysynth::{
    NoteEvent, PolySynth, PolySynthParams, PolySynthTables, phase_increment, steal_fade_frames_for,
};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, PPQ};

use crate::drums::{DRUM_SLOTS, DrumHit, DrumMachine, DrumNoteMap, DrumsParams};

/// 每个轨道槽的声部数（复音上限）。
///
/// **再导出**，不是第二份定义：唯一实现在
/// [`yeban_dsp::polysynth::VOICES_PER_SLOT`]（上移前是 `synth.rs` 里的 16）。
/// 与 `yeban-dsp::limiter` 上移后 `mixer.rs` 只 `pub use` 同形 [ARCH-DSP-001]。
pub use yeban_dsp::polysynth::VOICES_PER_SLOT as VOICES_PER_TRACK;

/// 声部池支持的**轨道槽**上限（定长，构造期确定）。
///
/// 超过上限的轨道不参与合成（不 panic、不扩容），由 [`SynthEngine::track_drops`] 计数。
/// 16 条合成轨道足够一个最小引擎；提高它需要重新评估实时侧内存占用
/// （每槽 16 个声部 ≈ 1.4 KiB）。
pub const MAX_TRACK_SLOTS: usize = 16;

/// 单轨音符调度表的容量上限（条）。
///
/// 超过上限的音符在**构造期**被丢弃并计数
/// （[`crate::snapshot::EngineSnapshot::note_schedule_drops`]）。上限的作用是让
/// "快照边界处的游标校正"保持 `O(log n)`（`partition_point`），而不是一次无界的
/// 线性扫描。8192 个音符 ≈ 128 秒 @120 BPM 的十六分音符密度。
pub const MAX_NOTES_PER_TRACK: usize = 8192;

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

/// 引擎**波形库**的配方顺序（契约，不是实现细节）。
///
/// 数组下标就是 `osc1_wave` / `osc2_wave` 两个参数的**取值**：
///
/// | 下标 | 名字 | 配方（`yeban_dsp::oscillator` 的公开常量） |
/// | :--- | :--- | :--- |
/// | 0 | `hollow` | [`HOLLOW`] —— **上移前的历史默认音色** |
/// | 1 | `organ` | [`ORGAN`] |
/// | 2 | `vocal` | [`VOCAL`] |
/// | 3 | `metallic` | [`METALLIC`] |
/// | 4 | `glass` | [`GLASS`] |
///
/// ⚠ **下标 0 必须是 [`HOLLOW`]**：没有振荡器参数的工程走 [`PolySynthParams::new`]
/// （`osc1` = 波表 0），因此"历史默认音色"与"下标 0"必须是同一张表 ——
/// 既有工程的输出才**逐位不变**。这条由 `tests/synth_osc.rs` 的 O1 钉死
/// （"不给参数"与"显式给下标 0"逐位相同，而给 1..=4 都不同）。
///
/// 五个配方**不是**引擎的第二份 DSP 定义：它们都是 `yeban_dsp::oscillator` 的公开
/// 常量，与同模块的 `FACTORY_RECIPES` **逐条同名同值**；引擎只决定**顺序**
/// （`FACTORY_RECIPES` 的顺序是 `organ` 在前）与**下标 0 的归属**。
/// 判据 O2 把"本表的名字集合 == `FACTORY_RECIPES` 的名字集合"与"下标 0 = `hollow`"
/// 一起对账。
///
/// ⚠ 类型写成全路径 `yeban_dsp::oscillator::WaveRecipe`（而不是 `use` 进来）是**有意**的：
/// 本文件在 [`VOICES_PER_TRACK`] 那一行**之前**的行数被两处外部台账的源码引用钉住
/// （见 `docs/ledger/feature-alignment.md` 与 `docs/ledger/phase-status.md` 的引用），
/// 导入区多一行会让那些引用漂移。功能上两种写法完全等价。
const ENGINE_WAVE_RECIPES: [(&str, yeban_dsp::oscillator::WaveRecipe); 5] = [
    ("hollow", HOLLOW),
    ("organ", ORGAN),
    ("vocal", VOCAL),
    ("metallic", METALLIC),
    ("glass", GLASS),
];

/// `osc1_wave` / `osc2_wave` 的 `f32` 取值 → 波表下标（`u8`）。
///
/// 规则（完全确定，无猜测）：非有限或负值 ⇒ `0`（[`HOLLOW`]，历史默认音色）；
/// 否则**四舍五入**到最近整数，并把结果钳到 `0 ..= 255`。下标 ≥ 库长度时由器件
/// 按库长度钳到最后一张表（与 `yeban_dsp::polysynth::OscSettings` 同口径）
/// ⇒ 越界下标不会变成"静默静音"。
///
/// 四舍五入本身是 IEEE 精确类运算（[ADR-0001 D32]），且这一步在**控制线程**上
/// （[`ToneParams::from_devices`]），不在逐样本路径上。
fn wave_index(value: Option<f32>) -> u8 {
    match value {
        Some(value) if value.is_finite() => {
            let rounded = value.round();
            if rounded <= 0.0 {
                0
            } else if rounded >= 255.0 {
                255
            } else {
                rounded as u8
            }
        }
        _ => 0,
    }
}

/// [`ToneParams::from_devices`] 的**每设备**投影中间量（私有）。
///
/// 九个 `Option` 字段各对应一个参数名；`None` = 本设备**没写**这个参数
/// （与"写了 `0`"不同：`osc2_level = 0` 是"显式关掉第二条支路"）。
#[derive(Clone, Copy, Default)]
struct DeviceTone {
    cutoff_hz: Option<f32>,
    resonance: Option<f32>,
    drive: Option<f32>,
    osc1_wave: Option<f32>,
    osc2_wave: Option<f32>,
    osc1_level: Option<f32>,
    osc1_detune_cents: Option<f32>,
    osc2_level: Option<f32>,
    osc2_detune_cents: Option<f32>,
}

impl DeviceTone {
    /// 本设备是否是"音色来源"：至少写了一个**识别键**。
    ///
    /// 识别键 = `cutoff_hz`/`cutoff` **或**六个振荡器键中的任意一个。
    /// 只写 `resonance`/`res`/`drive` 的设备**仍然不算**来源 —— 这是上移前的规则
    /// （滤波器的主参数是截止频率），本票只把振荡器键**并列**加进来。
    const fn is_source(&self) -> bool {
        self.cutoff_hz.is_some()
            || self.osc1_wave.is_some()
            || self.osc2_wave.is_some()
            || self.osc1_level.is_some()
            || self.osc1_detune_cents.is_some()
            || self.osc2_level.is_some()
            || self.osc2_detune_cents.is_some()
    }
}

/// 内置音色的**引擎侧**参数投影：一个四极梯形低通的三个旋钮 ＋ 两条振荡器支路。
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
///     {name:"cutoff_hz",  value: 1200}       ├ drive: f32
///     {name:"resonance",  value: 0.2}        ├ osc1_wave: u8
///     {name:"drive",      value: 0.0}        ├ osc2_wave: u8
///     {name:"osc1_wave",  value: 1}          ├ osc1_level: f32
///     {name:"osc2_wave",  value: 4}          ├ osc1_detune_cents: f32
///     {name:"osc1_level", value: 0.8}        ├ osc2_level: f32
///     {name:"osc1_detune_cents", value: -6}  └ osc2_detune_cents: f32
///     {name:"osc2_level", value: 0.5}
///     {name:"osc2_detune_cents", value: 7}   ⇒ 本模块的投影整体删除，改成直读字段
///   ]
/// ```
///
/// ## 投影规则（完全确定，无猜测）
///
/// 1. 只看 [`DeviceKind::InternalInstrument`] 的设备（外部乐器由插件宿主负责，
///    `External*` 一律忽略）；`bypassed` 的设备整体忽略；
/// 2. **识别键**：`cutoff_hz`/`cutoff` **或**六个振荡器键中的任意一个。取值发生在
///    **第一条**写了识别键的设备上（上移前的识别键只有截止频率；振荡器键是按
///    "引擎侧选不了波形 / 第二条振荡器"这条缺口登记补上的同一类键）；
/// 3. 九个参数名（大小写不敏感）：`cutoff_hz`/`cutoff`、`resonance`/`res`、`drive`、
///    `osc1_wave`、`osc2_wave`、`osc1_level`、`osc1_detune_cents`、`osc2_level`、
///    `osc2_detune_cents`；
/// 4. **一个识别键都没有 ⇒ [`ToneParams::bypass`]**：这是刻意选的默认值，
///    因为"模型层没给参数"与"用户把滤波器拧到旁通"在音频上应当同解 ——
///    这样既有的（无设备链的）工程逐位不变，也避免了给所有轨道硬塞一个未裁决的音色；
/// 5. 取值在构造期钳制（[`LadderFilter::configure`] 内部再钳一次 Nyquist）：
///    截止频率 `20 Hz ..= 0.45·fs`、共振/驱动 `0.0 ..= 1.0`；
/// 6. 非有限值 ⇒ **该参数**按默认取（截止 `12 kHz`、共振 `0`、驱动 `0`、
///    波形下标 `0`、`osc1_level` `1.0`（满）、`osc1_detune_cents` `0.0`、
///    `osc2_level` `0.0`（关）、`osc2_detune_cents` `0.0`），绝不 `NaN`；
/// 7. **只有振荡器键、没有截止频率** ⇒ 滤波器**整段旁通**，振荡器照常投影
///    （不是"截止取成 20 kHz 占位值"——那样状态变量仍会吸收瞬态并染色）。
///
/// ## 波形下标、两条支路的电平与失谐
///
/// `osc1_wave` / `osc2_wave` 的取值是 [`ENGINE_WAVE_RECIPES`] 的**下标**
/// （先四舍五入、再钳到 `0 ..= 255`；越界下标由器件钳到最后一张表）。
/// 两条支路的电平与失谐**都是可投影的**：`osc1_level` / `osc1_detune_cents` 与
/// `osc2_level` / `osc2_detune_cents` 各指一处。
///
/// **默认值是历史默认**（这是"接线不改变既有输出"的全部理由）：`osc1_level = 1.0`
/// （满）、`osc1_detune_cents = 0.0`、`osc2_level = 0.0`（关）、
/// `osc2_detune_cents = 0.0`。因此**不写这六个键**的工程与接通振荡器投影之前**逐位
/// 相同**（判据 O1 与 O8）。`osc2_level == 0.0`（默认）时器件对第二条支路**一帧也不
/// 执行**（[`yeban_dsp::polysynth`] 模块文档 §2）⇒ "关掉"是逐位恒等的；`osc1_level`
/// 取 `0.0` 时第一条支路同样一帧也不执行。
///
/// ## 确定性分类（[ADR-0001 D32]）
///
/// 构造期 `LadderFilter::configure` 含 `tan(π·fc/fs)`、包络含 `exp`
/// ⇒ **超越函数类**（4096 ulp 预算）；
/// 逐样本 `LadderFilter::process` 只有乘加与一次 Padé 除法，振荡器支路只有整数相位
/// 递推、波表线性插值与乘加 ⇒ **IEEE 精确类**。
/// 因此本切片对跨架构 L1 的强度声明是"系数冻结后逐样本位精确"，
/// **不是**"整条音色链跨架构位精确"（详见 notes 的 D32 分类表）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneParams {
    cutoff_hz: f32,
    resonance: f32,
    drive: f32,
    osc1_level: f32,
    osc1_detune_cents: f32,
    osc2_level: f32,
    osc2_detune_cents: f32,
    bypass: bool,
    osc1_wave: u8,
    osc2_wave: u8,
}

impl ToneParams {
    /// 旁通（**默认**）：渲染路径**完全不经过**滤波器。
    ///
    /// 不是"滤波器系数取成透明"——那样状态变量仍会吸收瞬态并染色。
    /// 旁通是**逐位恒等**的：`render_track` 根本不调用 `process`。
    ///
    /// 振荡器取**历史默认**：`osc1_level = 1.0`（满）、`osc1_detune_cents = 0.0`、
    /// `osc2_level = 0.0`（关）、`osc2_detune_cents = 0.0`。
    #[must_use]
    pub const fn bypass() -> Self {
        Self {
            cutoff_hz: TONE_BYPASS_CUTOFF_HZ,
            resonance: 0.0,
            drive: 0.0,
            osc1_level: 1.0,
            osc1_detune_cents: 0.0,
            osc2_level: 0.0,
            osc2_detune_cents: 0.0,
            bypass: true,
            osc1_wave: 0,
            osc2_wave: 0,
        }
    }

    /// 显式配置音色（构造期调用；非有限值退回该参数的默认值）。
    ///
    /// 振荡器取**默认**：`osc1_wave = osc2_wave = 0`（[`HOLLOW`]）、第一条支路满电平
    /// 不失谐、`osc2` 关。要选波形、开第二条支路或给第一条支路设电平/失谐，接着调
    /// [`ToneParams::with_oscillators`] 与 [`ToneParams::with_osc1`]。
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
            osc1_level: 1.0,
            osc1_detune_cents: 0.0,
            osc2_level: 0.0,
            osc2_detune_cents: 0.0,
            bypass: false,
            osc1_wave: 0,
            osc2_wave: 0,
        }
    }

    /// 设置两条振荡器支路的**可选**部分（**构造期**；非有限值退回该参数的默认值）。
    ///
    /// - `osc1_wave` / `osc2_wave`：波形下标，见 [`ENGINE_WAVE_RECIPES`]
    ///   （`u8`，越界下标由器件按库长度钳到最后一张表）；
    /// - `osc2_level`：`0.0`（**默认**）= 第二条支路**关**；非有限值 ⇒ `0.0`（关），
    ///   否则钳到 `0.0 ..= 1.0`；
    /// - `osc2_detune_cents`：非有限值 ⇒ `0.0`；量程由器件再钳一次
    ///   （`yeban_dsp::polysynth` 的 `MAX_DETUNE_CENTS`）。
    ///
    /// ⚠ `osc2_level` 的"非有限 ⇒ 关"是**本层**的口径：器件自己的
    /// `OscSettings::new` 对非有限电平取 `1.0`（满开，因为器件的默认是"开"），
    /// 而本层临时形状的默认是"第二条支路关" ⇒ 非法输入退回**本层的**默认值，
    /// 不让一个坏参数把一条没被要求的振荡器推满。这条由判据 O6 钉死。
    #[must_use]
    pub const fn with_oscillators(
        mut self,
        osc1_wave: u8,
        osc2_wave: u8,
        osc2_level: f32,
        osc2_detune_cents: f32,
    ) -> Self {
        self.osc1_wave = osc1_wave;
        self.osc2_wave = osc2_wave;
        self.osc2_level = if osc2_level.is_finite() {
            if osc2_level < 0.0 {
                0.0
            } else if osc2_level > 1.0 {
                1.0
            } else {
                osc2_level
            }
        } else {
            0.0
        };
        self.osc2_detune_cents = if osc2_detune_cents.is_finite() {
            osc2_detune_cents
        } else {
            0.0
        };
        self
    }

    /// 设置**第一条**振荡器支路的电平与失谐（**构造期**；非有限值退回该参数的默认值）。
    ///
    /// 这是把"引擎侧没有 `osc1` 电平/失谐的投影"这条登记在案的缺口补上的那条投影
    /// （[`crate::synth`] 模块文档 §4 与 `crate::lib` 的边界清单）。它是一条**独立**的
    /// 构造步骤，而不是 [`ToneParams::with_oscillators`] 的参数：后者在缺口存在时就
    /// 已经在用，改它的签名会让既有的调用点全部要改（而那与"补一条投影"无关）。
    ///
    /// - `level`：`1.0`（**默认**）= 满电平；非有限值 ⇒ `1.0`（**本层的默认 = 满**），
    ///   否则钳到 `0.0 ..= 1.0`（`0.0` = 关掉第一条支路）；
    /// - `detune_cents`：非有限值 ⇒ `0.0`；量程由器件再钳一次
    ///   （MAX_DETUNE_CENTS）。
    ///
    /// ⚠ 两条支路的"非有限 ⇒ 默认"**默认值不同**（`osc1` 满、`osc2` 关），因为
    /// 历史默认音色是"一条满电平的单振荡器"。这不是两条口径：两处都是"非法输入退回
    /// **该字段在该层的历史默认值**"，这正是判据 O1（不给振荡器键 ⇒ 逐位等于历史默认）
    /// 能成立的原因。
    #[must_use]
    pub const fn with_osc1(mut self, level: f32, detune_cents: f32) -> Self {
        self.osc1_level = if level.is_finite() {
            if level < 0.0 {
                0.0
            } else if level > 1.0 {
                1.0
            } else {
                level
            }
        } else {
            1.0
        };
        self.osc1_detune_cents = if detune_cents.is_finite() {
            detune_cents
        } else {
            0.0
        };
        self
    }

    /// 从模型层的设备链投影（**控制线程**；见本节文档的投影规则）。
    #[must_use]
    pub fn from_devices(devices: &[DeviceDefinition]) -> Self {
        for device in devices {
            if device.bypassed || device.kind != DeviceKind::InternalInstrument {
                continue;
            }
            let mut tone = DeviceTone::default();
            for param in &device.params {
                let name = param.name.to_ascii_lowercase();
                match name.as_str() {
                    "cutoff_hz" | "cutoff" => tone.cutoff_hz = Some(param.value),
                    "resonance" | "res" => tone.resonance = Some(param.value),
                    "drive" => tone.drive = Some(param.value),
                    "osc1_wave" => tone.osc1_wave = Some(param.value),
                    "osc2_wave" => tone.osc2_wave = Some(param.value),
                    "osc1_level" => tone.osc1_level = Some(param.value),
                    "osc1_detune_cents" => tone.osc1_detune_cents = Some(param.value),
                    "osc2_level" => tone.osc2_level = Some(param.value),
                    "osc2_detune_cents" => tone.osc2_detune_cents = Some(param.value),
                    _ => {}
                }
            }
            if tone.is_source() {
                return Self::from_projection(tone);
            }
            // 第一条**不是**音色来源的乐器设备 ⇒ 继续找下一条。
        }
        Self::bypass()
    }

    /// 由投影出来的原始字段组装（`cutoff_hz` 缺失 ⇒ 滤波器旁通，见投影规则 7）。
    fn from_projection(tone: DeviceTone) -> Self {
        let base = match tone.cutoff_hz {
            Some(cutoff_hz) => Self::new(
                cutoff_hz,
                tone.resonance.unwrap_or(0.0),
                tone.drive.unwrap_or(0.0),
            ),
            None => Self::bypass(),
        };
        base.with_oscillators(
            wave_index(tone.osc1_wave),
            wave_index(tone.osc2_wave),
            tone.osc2_level.unwrap_or(0.0),
            tone.osc2_detune_cents.unwrap_or(0.0),
        )
        .with_osc1(
            tone.osc1_level.unwrap_or(1.0),
            tone.osc1_detune_cents.unwrap_or(0.0),
        )
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

    /// 振荡器 1 的波形下标（见 [`ENGINE_WAVE_RECIPES`]）。
    #[must_use]
    pub const fn osc1_wave(&self) -> u8 {
        self.osc1_wave
    }

    /// 振荡器 1 的电平（`0.0` = 关；历史默认 `1.0` = 满）。
    #[must_use]
    pub const fn osc1_level(&self) -> f32 {
        self.osc1_level
    }

    /// 振荡器 1 的失谐（音分；历史默认 `0.0`）。
    #[must_use]
    pub const fn osc1_detune_cents(&self) -> f32 {
        self.osc1_detune_cents
    }

    /// 振荡器 2 的波形下标（见 [`ENGINE_WAVE_RECIPES`]）。
    #[must_use]
    pub const fn osc2_wave(&self) -> u8 {
        self.osc2_wave
    }

    /// 振荡器 2 的电平（`0.0` = 关）。
    #[must_use]
    pub const fn osc2_level(&self) -> f32 {
        self.osc2_level
    }

    /// 振荡器 2 的失谐（音分）。
    #[must_use]
    pub const fn osc2_detune_cents(&self) -> f32 {
        self.osc2_detune_cents
    }

    /// 在给定采样率下武装一个滤波器（**构造期**：含 `tan`，超越函数类）。
    #[must_use]
    pub fn filter(&self, sample_rate: f32) -> LadderFilter {
        let mut filter = LadderFilter::new();
        filter.configure(sample_rate, self.cutoff_hz, self.resonance, self.drive);
        filter
    }

    /// 一条振荡器支路的器件参数（**构造期**）。
    ///
    /// 写成全路径而不 `use` 的理由与 [`ENGINE_WAVE_RECIPES`] 相同：本文件导入区的
    /// 行数被外部台账的源码引用钉住。电平与失谐的钳制由 `OscSettings::new` 负责
    /// （本层已经在 [`ToneParams::with_oscillators`] 与 [`ToneParams::with_osc1`] 里
    /// 把非有限值换成默认值）。
    fn osc_settings(table: u8, level: f32, detune_cents: f32) -> yeban_dsp::polysynth::OscSettings {
        yeban_dsp::polysynth::OscSettings::new(usize::from(table), level, detune_cents)
    }

    /// 投影到 DSP 器件的参数（构造期/快照边界；由 [`SynthEngine`] 调用）。
    ///
    /// 两条振荡器支路与滤波器一起投影：`osc1` 取 [`ToneParams::osc1_level`] /
    /// [`ToneParams::osc1_detune_cents`]，`osc2` 取
    /// [`ToneParams::osc2_level`] / [`ToneParams::osc2_detune_cents`]。
    ///
    /// 没有振荡器参数的工程得到 `osc1_wave = 0`（[`HOLLOW`]）、`osc1_level = 1.0`、
    /// `osc1_detune_cents = 0.0`、`osc2_level = 0.0`（关）⇒ 与
    /// [`PolySynthParams::new`] 的参数**逐位相同**，
    /// 因此输出与接通振荡器投影之前**逐位相同**（判据 O1 与 O8）。
    #[must_use]
    pub fn poly_synth_params(&self) -> PolySynthParams {
        PolySynthParams::new()
            .with_oscillators(
                Self::osc_settings(self.osc1_wave, self.osc1_level, self.osc1_detune_cents),
                Self::osc_settings(self.osc2_wave, self.osc2_level, self.osc2_detune_cents),
            )
            .with_filter(self.cutoff_hz, self.resonance, self.drive, self.bypass)
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

// 相位增量的唯一实现在 `yeban_dsp::polysynth::phase_increment`（上移前是本文件的
// 私有函数，本票把它并入器件）。[`ScheduledNote::new`] 在这里调用它：
//
// - `inc == 0` 会让声部永久停在相位 0（静音）⇒ 钳到下界 1；
// - 上界钳到 `u32::MAX`（频率 ≥ `sample_rate` 时回绕成"每样本整周期"）；
//   实际调度侧的音高上限是 MIDI 127 ≈ 12.5 kHz。
//
// 一次 `f64` 乘除 + `round`：这一步**不在**逐样本路径上（构造期一次），
// `round` 本身是 IEEE 精确类（D32 §1）。

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

/// 一个声部的全部状态：**已上移到 `yeban-dsp`**。
///
/// 上移前这里有一个 `Voice` 结构体、`steal_fade_frames()` 函数与
/// `trigger()`/`steal_priority()` 两个自由函数，逐样本循环也长在
/// [`SynthEngine::render_track`] 里。现在它们**全部**只有一份实现：
/// [`yeban_dsp::polysynth::PolySynth`]（器件化，含双振荡器、整数相位波表、
/// 声部级梯形低通、ADSR、[ARCH-RT-004] 确定性窃取与 3 ms 指数淡出）。
///
/// 本文件因此不再有第二份合成器实现 —— 与 `mixer.rs` 上移限制器后只
/// `pub use` 同形。逐位一致的强度是构造性的：算式、运算顺序、状态初值都是
/// 上移前的那一串字符（见 `crates/yeban-dsp/src/polysynth.rs` §0–§5）。
///
/// 一个轨道槽：身份 + 游标 + 本轨音色参数 + **DSP 器件实例**（每轨一台合成器）。
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
    ///
    /// 它同时是"参数有没有变"的比较键（`tone != slot.tone` 才重算器件参数）。
    tone: ToneParams,
    /// 本轨的合成器器件：定长声部池 + 本轨音色（**实时侧唯一的状态拥有者**）。
    synth: PolySynth<VOICES_PER_TRACK>,
}

impl TrackSlot {
    /// 空槽（`EntityId::default()` 是 nil，`assigned = false` 时无意义）。
    fn empty(sample_rate: u32) -> Self {
        Self {
            assigned: false,
            present: false,
            id: EntityId::default(),
            cursor: 0,
            tone: ToneParams::bypass(),
            synth: PolySynth::new(sample_rate),
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
    /// 波形库（构造期建好；渲染路径只读）。
    ///
    /// 引擎装**五张**表，顺序与名字见 [`ENGINE_WAVE_RECIPES`]（下标 0 是
    /// [`HOLLOW`] = 上移前的唯一一张表 ⇒ 没有振荡器参数的工程逐位不变）。
    /// 每轨用哪一张由 [`ToneParams`] 的 `osc1_wave` / `osc2_wave` 决定；
    /// 第二条振荡器支路是器件的公共面，本票已经把它的投影接通。
    ///
    /// ⚠ 建库会分配（9 级 × 2048 点 ≈ 72 KiB / 张）⇒ **只**能在这里（打开设备之前）
    /// 做；`begin_snapshot` 运行在音频线程上，那里一次也不许重建波表。
    tables: PolySynthTables,
    /// 当前采样率（快照边界更新）。
    sample_rate: f32,
    /// 窃取淡出帧数（默认 3 ms ⇒ @48k = 144 帧）。
    ///
    /// **判据可覆盖**（[`SynthEngine::set_steal_fade_frames`]）：把 0 设进去就回到
    /// "硬窃取"，那条"淡出后无跳变"的判据必须能对它变红。
    /// 权威值在本结构体上，每次参数/采样率变化都下推给各槽的器件。
    steal_fade_frames: u32,
    /// 播放头（绝对样本位置）。
    position: u64,
    slots: [TrackSlot; MAX_TRACK_SLOTS],
    track_drops: u64,
    notes_triggered: u64,
    /// **每槽一台鼓机**（`line/engine-drums`）。下标与 [`Self::slots`] 同源。
    ///
    /// ⚠ 这是**唯一**被 `Box` 起来的字段，理由是**尺寸**（实测，
    /// `size_of::<DrumMachine<16>>() = 13616`）：16 槽 **inline** 会让
    /// `SynthEngine` 从实测 `68416` 变成 `68416 + 16 × 13616 = 286272` 字节，
    /// 而 [`crate::rt::EngineRuntime`] 是按值构造并按值搬进回调的。
    /// `Box` 之后实测 `SynthEngine` = `71248`、`EngineRuntime` = `137920`
    /// （原 `68416` / `135088`）⇒ 增量 `2832 = 16 × 176`（`drum_slots`，
    /// 实测 `size_of::<Option<(EntityId, DrumsParams)>>() = 176`）
    /// `+ 8`（`Box` 指针）`+ 8`（`drum_hits`）；这 `217856` 字节因此落在
    /// **构造期一次性分配**的堆上（`SynthEngine::new` 不在音频线程上），实时对象
    /// 只多一个指针。分配次数 **1**，且与 `PolySynthTables`（≈ 72 KiB 波表）
    /// 同一条纪律：**只在打开设备之前**。
    drums: Box<[DrumMachine<DRUM_SLOTS>]>,
    /// 每槽的**鼓机投影**（`None` = 本槽的音源是复音合成器）。
    ///
    /// 同时是审计状态：快照边界用它做"参数变了才 `set_params`"的比较、
    /// "换轨 ⇒ `reset`"，以及 [`SynthEngine::drum_params`] 的读数。
    /// 槽位身份一起存（`EntityId`），因为"本槽被另一条轨接管"必须能分辨。
    drum_slots: [Option<(EntityId, DrumsParams)>; MAX_TRACK_SLOTS],
    /// 累计的**鼓击触发数**（每触发一次 [`DrumMachine::trigger`] 记 1）。
    ///
    /// 它是"鼓机真的接在信号路径上"的覆盖度读数 —— 与 `EngineStats::metronome_clicks`
    /// 同族：`armed_drum_slots() > 0` 只说明"武装了"，这个数才说明"真的打了鼓"。
    drum_hits: u64,
}

impl SynthEngine {
    /// 构造：建好波形库与各槽的器件（**允许分配**：这一步在打开设备之前）。
    ///
    /// 波形库按 [`ENGINE_WAVE_RECIPES`] 建**五张**表（下标 0 = [`HOLLOW`]）。
    /// `sample_rate` 先按传入值武装；真正的采样率在第一次
    /// [`begin_snapshot`](Self::begin_snapshot) 时按快照校准。
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate_f32 = sanitise_sample_rate(sample_rate);
        let wave_recipes = ENGINE_WAVE_RECIPES.map(|(_, recipe)| recipe);
        Self {
            tables: PolySynthTables::from_recipes(&wave_recipes),
            sample_rate: sample_rate_f32,
            steal_fade_frames: steal_fade_frames_for(sample_rate_f32),
            position: 0,
            slots: [TrackSlot::empty(sample_rate); MAX_TRACK_SLOTS],
            track_drops: 0,
            notes_triggered: 0,
            // 鼓机池：**构造期一次性分配**（`vec!` 在这里，不在 `begin_snapshot` 里），
            // 分配次数 1（见字段文档的尺寸读数与理由）。
            drums: vec![DrumMachine::new(sample_rate); MAX_TRACK_SLOTS].into_boxed_slice(),
            drum_slots: [None; MAX_TRACK_SLOTS],
            drum_hits: 0,
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
    ///
    /// 器件各持一个计数器 ⇒ 这里是**各槽之和**（上移前是引擎上的单个计数器，
    /// 语义与总数相同）。
    #[must_use]
    pub fn voice_steals(&self) -> u64 {
        self.slots.iter().fold(0u64, |total, slot| {
            total.saturating_add(slot.synth.voice_steals())
        })
    }

    /// 当前窃取淡出帧数（默认 3 ms × 采样率）。
    #[must_use]
    pub const fn steal_fade_frames(&self) -> u32 {
        self.steal_fade_frames
    }

    /// 覆盖窃取淡出帧数（**判据/注入用**；0 = 硬窃取）。
    ///
    /// 产物路径不调用它：帧数由采样率与
    /// [`STEAL_RELEASE_SECONDS`](yeban_dsp::envelope::STEAL_RELEASE_SECONDS) 决定。
    /// 存在的理由是"淡出后无跳变"这条判据必须能对着**硬窃取**变红 ——
    /// 否则它可能在"根本没窃取"的夹具上永真。
    pub fn set_steal_fade_frames(&mut self, frames: u32) {
        self.steal_fade_frames = frames;
        for slot in &mut self.slots {
            slot.synth.set_steal_fade_frames(frames);
        }
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

    /// 累计的**鼓击触发数**（覆盖度读数：> 0 才说明鼓机真的在打鼓）。
    ///
    /// 口径：引擎在**派发处**记 1（`render_track` 的鼓机分支，紧接着调用
    /// `DrumMachine::trigger`）。它与 [`Self::drum_triggers`] 是同一个事件的**两侧**，
    /// 两者在静止点上恒等（判据见 `tests/drums_instrument.rs` 的 D9）。
    #[must_use]
    pub const fn drum_hits(&self) -> u64 {
        self.drum_hits
    }

    /// 鼓机器件**自己**累计接收的触发数（各槽之和；单位：次）。
    ///
    /// 它不是 [`Self::drum_hits`] 的第二份副本，而是那条**内部一致性判据**的另一侧：
    /// `drum_hits` 数的是"引擎派发了几次"，本读数数的是"器件收到了几次"。
    /// 两边恒等（等号判据见 `tests/drums_instrument.rs` 的 D9）；任何"派发了却没到
    /// 器件"或"到了器件却没派发"的接线错误都会让这条等号当场不成立。
    ///
    /// ⚠ 器件计数器**只增不减**，且 `DrumMachine::reset` 只清在响的槽位、**不**清
    /// 计数器 ⇒ 本读数是引擎生命周期里的累计量，量法与 [`Self::drum_hits`] 同口径。
    /// 读它只汇总 16 个 `u64` 字段，不碰样本、不分配、不加锁（器件文档指明该计数器
    /// 不改变音频输出）。
    #[must_use]
    pub fn drum_triggers(&self) -> u64 {
        self.drums.iter().fold(0u64, |total, machine| {
            total.saturating_add(machine.triggers())
        })
    }

    /// 鼓机**声部池**累计的窃取次数（各槽之和；单位：次）。
    ///
    /// 它与 [`Self::voice_steals`] 是**两个不同的池**，不能相加成"一个池"：
    /// `voice_steals` 汇总复音合成器的 `PolySynth` 槽位，本读数汇总
    /// `DrumMachine` 槽位 —— 两者是两件独立器件、两套独立计数器。
    /// 为什么需要它：鼓击池满时器件会替换一个在响槽位并推进自己的计数器，
    /// 而 `voice_steals` 只走复音那条 ⇒ **只挂鼓机的工程在池满时读数恒为 0**，
    /// "声部窃取"这件事在读数面上不可见。本读数把那一半补上。
    ///
    /// 单位与 [`Self::voice_steals`] 相同（次），两者都随各自的池饱和增长。
    #[must_use]
    pub fn drum_voice_steals(&self) -> u64 {
        self.drums.iter().fold(0u64, |total, machine| {
            total.saturating_add(machine.voice_steals())
        })
    }

    /// 累计的**闭镲 choke 开镲**次数（各槽之和；单位：次）。
    ///
    /// 器件口径：每当一个闭镲被触发，所有正在响的开镲槽位立刻进入同一条窃取淡出，
    /// 每一件被 choke 的开镲记 1。反向（开镲 choke 闭镲）**不做**。
    ///
    /// 为什么需要它：这是一条**跨声部**行为，任何单轨电平或触发计数都看不到它
    /// （被 choke 的开镲本来就该衰减）⇒ 只从音频输出反推不出来。
    #[must_use]
    pub fn drum_hat_chokes(&self) -> u64 {
        self.drums.iter().fold(0u64, |total, machine| {
            total.saturating_add(machine.hat_chokes())
        })
    }

    /// 累计的**发声槽位帧**（各槽之和；单位：槽位×帧）。
    ///
    /// 器件口径：一次 `render` 的某一帧里某个在响槽位产出非零样本，就记 1；
    /// 除以渲染帧数即"平均同时发声的槽位数"。器件文档明说它是**覆盖度仪器**、
    /// **不改变音频输出**：非零只说明那个槽位在算东西，不说明它多响。
    ///
    /// 为什么需要它：[`Self::drum_hits`] 只证明"触发到了器件"，证明不了"器件真的
    /// 算出了声音"（一个把槽位全部丢掉的重置 bug 会让 `drum_hits` 照常增长）。
    #[must_use]
    pub fn drum_sounding_slot_frames(&self) -> u64 {
        self.drums.iter().fold(0u64, |total, machine| {
            total.saturating_add(machine.sounding_slot_frames())
        })
    }

    /// 本快照武装为**鼓机音源**的槽位数。
    #[must_use]
    pub fn armed_drum_slots(&self) -> usize {
        self.drum_slots.iter().filter(|slot| slot.is_some()).count()
    }

    /// 某轨的鼓机投影（`None` = 本轨的音源是复音合成器）。
    ///
    /// 存在的理由与 [`Self::table_levels`] 同族：把"武装进去的那个数"变成**可读**的，
    /// 判据就不必从音频输出反推它。
    #[must_use]
    pub fn drum_params(&self, track: EntityId) -> Option<DrumsParams> {
        self.slot_index(track)
            .and_then(|index| self.drum_slots[index])
            .map(|(_, params)| params)
    }

    /// 轨道身份 → 槽位下标（`None` = 该轨没有占槽或不在本快照里）。
    #[must_use]
    fn slot_index(&self, track: EntityId) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.assigned && slot.present && slot.id == track)
    }

    /// 某轨**武装进去的**音色参数（`None` = 该轨没有占槽或不在本快照里）。
    ///
    /// 与 [`Self::drum_params`] 同族：把"武装进去的那个数"变成**可读**的。
    /// 判据因此能直接断言"波形下标 / 第二条支路真的到了器件那一侧"，
    /// 不必只从音频输出反推（见 `tests/synth_osc.rs` 的 O3）。
    #[must_use]
    pub fn tone_params(&self, track: EntityId) -> Option<ToneParams> {
        self.slot_index(track).map(|index| self.slots[index].tone)
    }

    /// 声部池的诊断转储（**仅 debug 构建**；`tests/` 判据用来定位"为什么没有窃取"）。
    ///
    /// 返回 `(活跃, end_sample, 包络阶段, 淡出剩余)`，逐条来自该槽器件的
    /// [`yeban_dsp::polysynth::PolySynth::debug_voice`]。它暴露的是**内部**状态，
    /// 因此只在 `debug_assertions` 下编译 —— 产物路径上没有这个方法，
    /// 也就没有"为了测试把内部状态公开成 API"的负担。
    #[cfg(debug_assertions)]
    #[must_use]
    pub fn debug_voice_state(&self, track: EntityId) -> Vec<(bool, u64, AdsrStage, u32)> {
        self.slots
            .iter()
            .find(|slot| slot.assigned && slot.id == track)
            .map(|slot| {
                (0..slot.synth.voices())
                    .filter_map(|index| slot.synth.debug_voice(index))
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
            .map_or(0, |slot| slot.synth.active_voices())
    }

    /// 波形库第一张表的 mip 级数（诊断用）。
    #[must_use]
    pub fn table_levels(&self) -> usize {
        self.tables.level_count(0)
    }

    /// 波形库的张数（诊断用；恒等于 [`ENGINE_WAVE_RECIPES`] 的长度）。
    ///
    /// 判据 O2 用它 + [`Self::table_name`] 钉住"库真的按本票那张表建起来了"：
    /// 只断言 `osc1_wave` 的读数无法区分"投影对了但库里只有一张表"（器件对越界
    /// 下标钳到最后一张表 ⇒ 五张表退化成一个声音）。
    #[must_use]
    pub fn table_count(&self) -> usize {
        self.tables.len()
    }

    /// 波形库第 `table` 张表的**名字**（越界下标钳到最后一张，与器件同口径）。
    ///
    /// 名字与下标的对应就是 [`ENGINE_WAVE_RECIPES`]；本方法让"下标 → 名字"只有
    /// 一处事实源（判据 O2 拿它与 `yeban_dsp::oscillator::FACTORY_RECIPES` 的名字集合对账）。
    #[must_use]
    pub fn table_name(&self, table: usize) -> &'static str {
        let last = ENGINE_WAVE_RECIPES.len().saturating_sub(1);
        ENGINE_WAVE_RECIPES[table.min(last)].0
    }

    /// 跳到某个绝对样本位置（走带 seek 的接入点；当前只有测试与离线渲染用）。
    ///
    /// 释放全部声部并把播放头设到 `position`。**不**重算游标：下一个量子的触发
    /// 循环会把"起点已过"的音符直接消费掉（不触发），因此不需要额外的扫描。
    ///
    /// ⚠ 鼓机也要释放：`DrumMachine::reset` 把槽位池清空（否则一次定位之后
    /// 旧的鼓声尾巴会继续从新位置响出来）。武装记录**保留**（配置没变，
    /// 下一个量子照旧是鼓机轨）。
    pub fn seek(&mut self, position: u64) {
        self.position = position;
        for slot in &mut self.slots {
            slot.synth.reset();
        }
        for machine in &mut self.drums {
            machine.reset();
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
    /// 5. **音源按新快照刷新**（`line/engine-drums`）：`drums` 给出每轨的
    ///    [`DrumsParams`]（`crate::drums`），在 `tones` 之后写入。不在 `drums` 里的
    ///    轨道 ⇒ `drum_slots[i]` 置 `None` ⇒ 音源是复音合成器；从鼓机**换回**
    ///    复音合成器时该槽的鼓机被 `reset`（丢掉可能正在响的尾巴，与节拍器关闭时
    ///    `MetronomeVoice::silence` 同一条口径）。
    ///
    /// 采样率变化时重算包络模板与全部轨道槽的滤波器系数（`exp`/`tan` 因此落在
    /// "每修订一次"而不是"每样本"）。
    ///
    /// ⚠ **签名扩展**：第三个参数之外多了 `drums`（`line/engine-drums`）。
    /// 与 `line/engine-sound` 给本函数加 `tones` 那次同款（见
    /// `docs/ledger/engine-mix-notes.md` §8.3 的跨线提醒）：调用点必须一起改。
    pub fn begin_snapshot<'a, I, T, D>(&mut self, sample_rate: u32, tracks: I, tones: T, drums: D)
    where
        I: IntoIterator<Item = &'a EntityId>,
        T: IntoIterator<Item = (&'a EntityId, &'a ToneParams)>,
        D: IntoIterator<Item = (&'a EntityId, &'a DrumsParams)>,
    {
        let sample_rate_u32 = sample_rate;
        let sample_rate = sanitise_sample_rate(sample_rate);
        let sample_rate_changed = sample_rate != self.sample_rate;
        if sample_rate_changed {
            self.sample_rate = sample_rate;
            self.steal_fade_frames = steal_fade_frames_for(sample_rate);
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

        // 同理收鼓机投影（**定长数组，栈上，无分配**）：`DrumsParams` 是 `Copy`
        // 值类型（`DrumKitParams` 128 字节 + 五个 `u8`），16 项 ≈ 2.7 KiB 栈。
        // 用 `Option` 而不是哨兵值：本 crate 的槽位身份是 `EntityId`，而"未占用"
        // 槽位的 id 就是 nil —— 哨兵会让"未占用槽位"误命中。
        let mut wanted_drums: [Option<(EntityId, DrumsParams)>; MAX_TRACK_SLOTS] =
            [None; MAX_TRACK_SLOTS];
        let mut wanted_drums_len = 0usize;
        for (id, params) in drums {
            if wanted_drums_len >= MAX_TRACK_SLOTS {
                break;
            }
            wanted_drums[wanted_drums_len] = Some((*id, *params));
            wanted_drums_len += 1;
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
                    self.slots[index] = TrackSlot::empty(sample_rate_u32);
                    self.slots[index].assigned = true;
                    self.slots[index].present = true;
                    self.slots[index].id = *track;
                }
                None => self.track_drops = self.track_drops.saturating_add(1),
            }
        }

        // 器件参数的写入（**构造期语义**，音频线程上运行 ⇒ 必须零分配）：
        // `set_params` 只算滤波器系数（`tan`）与包络系数（`exp`），不碰堆；
        // 波表库在 `new()` 里就建好了，这里一次也不重建。
        let tables = &self.tables;
        let steal_fade_frames = self.steal_fade_frames;
        for slot in &mut self.slots {
            let tone = wanted[..wanted_len]
                .iter()
                .find(|(id, _)| *id == slot.id)
                .map(|(_, tone)| *tone)
                .unwrap_or_else(ToneParams::bypass);
            if sample_rate_changed {
                slot.synth.set_sample_rate(sample_rate_u32);
                slot.synth.set_steal_fade_frames(steal_fade_frames);
            }
            if sample_rate_changed || tone != slot.tone {
                slot.tone = tone;
                // 系数在**构造期**算（`tan`，超越函数类）；旁通时系数无意义但仍算一份，
                // 让"参数 → 系数"只有一条路径。
                slot.synth.set_params(tone.poly_synth_params(), tables);
            }
        }

        // --- 鼓机武装（**构造期语义**，音频线程上运行 ⇒ 必须零分配）---
        // [`DrumMachine::set_params`] / [`DrumMachine::set_sample_rate`] 会重算在响槽位的
        // 包络与滤波器系数（`exp`/`tan`），但**不碰堆**（器件是定长数组）
        // —— 与同一个分支里的 `poly_synth_params` 是同一条纪律。
        //
        // 三条口径（与 §0.1 的"一条轨至多一件音源"配套）：
        // 1. 同一轨仍占同一槽、且参数逐位相同 ⇒ **一个字段都不写**（状态全保留）；
        // 2. 同一轨的参数变了 ⇒ 只 `set_params`（相位/包络/RNG/滤波器状态保留，
        //    这是器件的契约：`drums/mod.rs` 的 `set_params` 文档）；
        // 3. **换轨**（另一条轨接管本槽）或本轨不再是鼓机 ⇒ `reset` 丢掉上一件音源的
        //    尾巴；`present = false`（本轨暂时不在快照里）**不** reset
        //    —— 与声部池"等它回来时连续"同口径。
        let Self {
            slots,
            drums,
            drum_slots,
            ..
        } = self;
        for index in 0..MAX_TRACK_SLOTS {
            let present_id = if slots[index].assigned && slots[index].present {
                Some(slots[index].id)
            } else {
                None
            };
            let desired = present_id.and_then(|id| {
                wanted_drums[..wanted_drums_len]
                    .iter()
                    .flatten()
                    .find(|(want, _)| *want == id)
                    .map(|(_, params)| (id, *params))
            });
            let armed = drum_slots[index];
            match desired {
                Some((track, params)) => {
                    let same_track = armed.map(|(id, _)| id) == Some(track);
                    if !same_track {
                        drums[index].reset();
                        drums[index].set_sample_rate(sample_rate_u32);
                    } else if sample_rate_changed {
                        drums[index].set_sample_rate(sample_rate_u32);
                    }
                    if !same_track || armed.map(|(_, prev)| prev) != Some(params) {
                        drums[index].set_params(params.kit());
                    }
                    drum_slots[index] = Some((track, params));
                }
                None => match armed {
                    // 同一轨仍然 present、但不再是鼓机 ⇒ 丢掉正在响的尾巴。
                    Some((id, _)) if present_id == Some(id) => {
                        drums[index].reset();
                        drum_slots[index] = None;
                    }
                    // 本轨暂时不在快照里 ⇒ 保留状态与武装记录。
                    Some(_) if present_id.is_none() => {}
                    // 另一条轨接管本槽 ⇒ 丢掉上一件音源的尾巴。
                    Some(_) => {
                        drums[index].reset();
                        drum_slots[index] = None;
                    }
                    None => drum_slots[index] = None,
                },
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
            // 声部回收交给器件（"已过终点且包络已静音"才回收）。
            slot.synth.retire_finished(position);
        }
    }

    /// 渲染一条轨道的本量子输出（单声道、声相之前）。**实时路径**。
    ///
    /// `schedule` 为 `None`（本轨没有调度表）时输出静音但不 panic；
    /// `out` 的长度就是本量子的有效帧数（尾块可以小于
    /// [`crate::block::DEFAULT_BLOCK_FRAMES`]）。
    ///
    /// 音源由**快照**决定（见模块文档 §0.1）：本槽武装了鼓机（
    /// [`Self::drum_params`] 为 `Some`）⇒ 同一张调度表驱动
    /// [`DrumMachine::trigger`] / [`DrumMachine::render`]；否则走复音合成器。
    /// 两条路都只用"整数比较 + 器件内的乘加"，**零分配、零锁、零 I/O**
    /// [MUST-GATE-001]。
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
        let Self {
            tables,
            slots,
            drums,
            drum_slots,
            notes_triggered,
            drum_hits,
            ..
        } = self;
        let slot = &mut slots[index];
        // 本槽的音源：`Some(map)` = 鼓机（键位映射）。
        let drum_map: Option<DrumNoteMap> = drum_slots[index].map(|(_, params)| params.notes());

        // --- 1) 触发本量子窗口内起音、且尚未结束的音符 ---
        // 游标只增不减 ⇒ 已触发过的音符永远不会被再次触发。
        //
        // "分配声部 / 池满时软窃取 / 3 ms 淡出"全部在器件的 `note_on` 里
        // （[ARCH-RT-004]）；引擎只负责"哪个音符、在哪个量子边界"。
        // 鼓机分支共用同一个游标与同一个窗口判定，只有"音符 → 器件"这一步不同：
        // 键位映射里没有的音高**不触发任何鼓件**（也不改动器件状态）。
        if let Some(schedule) = schedule {
            let notes = schedule.notes();
            while let Some(note) = notes.get(slot.cursor) {
                if note.start_sample >= end {
                    break;
                }
                if note.end_sample > position {
                    match drum_map {
                        Some(map) => {
                            if let Some(voice) = map.voice_for(note.pitch) {
                                drums[index].trigger(DrumHit::new(
                                    voice,
                                    note.start_sample,
                                    note.gain,
                                ));
                                *drum_hits = drum_hits.saturating_add(1);
                                *notes_triggered = notes_triggered.saturating_add(1);
                            }
                        }
                        None => {
                            slot.synth.note_on(
                                NoteEvent::new(
                                    note.start_sample,
                                    note.end_sample,
                                    note.freq_hz,
                                    note.gain,
                                ),
                                tables,
                            );
                            *notes_triggered = notes_triggered.saturating_add(1);
                        }
                    }
                }
                slot.cursor += 1;
            }
        }

        // --- 2) 逐样本合成：全部在器件里（整数相位 + ADSR + 可选低通）---
        match drum_map {
            // 鼓机：器件自己把 `out` 清零再落样本（"无占用槽位 ⇒ 逐位静音"）。
            // 尾音的回收也在 `render` 里（包络走完的槽位被置回空闲）。
            Some(_) => drums[index].render(position, out),
            None => slot.synth.render(tables, position, out),
        }
    }
}

/// 触发一个音符、窃取选择与 3 ms 淡出：**已上移到 `yeban-dsp`**。
///
/// 上移前的 `trigger()` / `steal_priority()` 两个自由函数与本文件里的 `Voice`
/// 结构体已被 [`yeban_dsp::polysynth::PolySynth::note_on`] 完整取代
/// （[ARCH-RT-004] 的三条口径逐字搬过去，见该模块的 `note_on` 文档）。
/// 这里不保留任何副本 —— "全仓不许有两份合成器实现"。
///
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

    /// 默认音色的释放时间（秒）。
    ///
    /// ⚠ 刻意写成**字面量**而不是去读器件的常量：判据用实现自己的常量做期望值
    /// 是自我确认（"改错了也一起改对"）。默认音色的权威值在
    /// `yeban_dsp::polysynth::PolySynthParams::new()`（5 ms / 80 ms / 0.7 / 50 ms）。
    const RELEASE_SECONDS: f32 = 0.05;

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
        engine.begin_snapshot(48_000, &[id], [], []);
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
        engine.begin_snapshot(48_000, &ids, [], []);
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
                switched.begin_snapshot(48_000, &[id], [], []);
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
