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
//! | `tick × samples_per_tick`（`round`） | [`crate::snapshot`] 的投影（算式在 [`yeban_model::time_math::ticks_to_frames`]） | IEEE 精确类 |
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
//! - ⭐ ⭐ **追加修订段（任务单 `T-FIX-273`，台账第 278 条：⛔ 不改上面"波形与两条支路的
//!   电平/失谐已可选"原文）**：那一条描述的是 `T-FIX-273` **之前**的事实（当时投影只有
//!   滤波器三旋钮 ＋ 六个振荡器键，包络恒为器件默认 5/80/0.7/50 ms），现仍逐字保留。
//!   自 `T-FIX-273` 起 `ToneParams` **还**投影四条**振幅包络**参数
//!   （`attack_s` / `decay_s` / `sustain` / `release_s`），
//!   [`ToneParams::poly_synth_params`] 在 `with_filter` 之后调用
//!   [`PolySynthParams::with_envelope`] ⇒ 工程里写的包络真的进声音。
//!   四条键**不是**音源识别键（只写包络的设备仍被跳过），默认值逐字等于器件默认
//!   ⇒ 不写这四个键的工程与接通前**逐位相同**（判据 E1–E4，见
//!   `crates/yeban-engine/tests/synth_env.rs`）。仍缺参数名的**模型层规范**这一半没变。
//! - ⭐ ⭐ **追加修订段（任务单 `T-FIX-277`，台账第 278 条：⛔ 不改上面"包络已投影"
//!   原文）**：那一条描述的是 `T-FIX-277` **之前**的事实（当时投影只有滤波器三旋钮
//!   ＋ 六个振荡器键 ＋ 四条包络键，LFO 恒为器件默认"深度 `0` = 关"），现仍逐字保留。
//!   自 `T-FIX-277` 起 `ToneParams` **还**投影四条**全局 LFO** 参数
//!   （`lfo_wave` / `lfo_rate_hz` / `lfo_depth` / `lfo_target`），
//!   [`ToneParams::poly_synth_params`] 在 `with_envelope` 之后调用
//!   [`PolySynthParams::with_lfo`] ⇒ 工程里写的 LFO 真的进声音（每槽一份全局 LFO，
//!   所有声部共用；见 [`ToneParams`] 文档"## 全局 LFO"与
//!   [`yeban_dsp::polysynth`] 模块文档 §10）。
//!   四条键**不是**音源识别键（只写 LFO 的设备仍被跳过），默认值逐字等于器件默认
//!   ⇒ 不写这四个键的工程与接通前**逐位相同**（判据 L1–L4，见
//!   `crates/yeban-engine/tests/synth_lfo.rs`）。仍缺参数名的**模型层规范**这一半没变。
//! - **没有滑音/弯音/歌词/音素**：`MidiNote::slide`/`pitch_bend_curve`/`phonemes`
//!   不参与合成；
//! - ⭐ ⭐ **追加修订段（任务单 `#311b`，台账第 278 条：⛔ 不改上一条原文）**：上一条
//!   描述的是 `#311b` **之前**的事实，现仍逐字保留。自 `#311b` 起
//!   `MidiNote::pitch_bend_curve` 与 `MidiNote::slide` **已经真的发声**：
//!   [`crate::snapshot::project_schedules`] 在构造期把它们投影成每轨的
//!   [`PitchShape`] 旁表（音符局部**样本**偏移 ＋ 音分；滑音的 `target_cents =
//!   (target_pitch − pitch) × 100`），[`ScheduledNote::pitch_shape`] 只带 `u16` 下标；
//!   实时侧的 [`SynthEngine::render_track`] 把 `Option<&dyn Fn(u16, u64) -> f32>`
//!   探针交给器件（与 `cc` 探针同形），器件在**音符局部 64 样本**网格上重算
//!   `phase_inc`。空曲线 ⇒ 旁表为空 ⇒ 探针 `None` ⇒ 与接线前**逐位相同**。
//!   ⚠ `phonemes` 仍**不**参与合成（没有歌声合成），`noteLyrics` 仍登记为未做。
//! - **没有采样播放/SFZ**：`ClipContent::Audio` 与 `yeban-sfz` 的乐器都还没有接到
//!   声部池上（需要采样加载 + 重采样，见 notes 的 pending）；
//! - ⭐ ⭐ **追加修订段（任务单 `#135`，台账第 278 条：⛔ 不改上一条原文）**：上一条
//!   描述的是**接线之前**的事实，现仍逐字保留。自 `#135` 起 `ClipContent::Audio`
//!   **已经在实时侧发声**，但它⛔ **不经过声部池**：音频摆放是**独立音源**，由
//!   [`crate::audio_clip::mix_into`] 在 `rt::render_branch` 的**逐轨单声道暂存**上累加
//!   （位置在 `render_track` 之后、插入链之前）。前提是控制侧已经把 PCM 注入快照
//!   （[`crate::snapshot::EngineSnapshot::with_audio_asset`]）；⛔ 引擎不读盘、⛔ 不重采样。
//!   `yeban-sfz` 那一半早已由 [`crate::sfz_instrument`] 接通；声部池本身仍然**只**做
//!   复音波表合成，⛔ 不播采样。
//! - ⭐ ⭐ **追加修订段（任务单 `#246`，台账第 278 条：⛔ 不改上面"没有循环片段展开"原文）**：
//!   上一条描述的是 `#246` 之前的事实，现仍逐字保留。自 `#246` 起片段循环语义由
//!   [ADR-0010] D1–D3 落定，并**下移到 `yeban-model`**（`ClipPlacement::loop_repeat` /
//!   `note_copies`）；[`crate::snapshot`] 的 MIDI 排程按它**真的展开**多条副本，
//!   每条副本再走本模块的 ratchet/pulse 路径 ⇒ 监听音频**真的重复**。
//!   ⚠ 音频片段的循环仍**不在**本票范围（[ADR-0010] D4 的 `source_offset` 引擎侧不存在）。
//! - ⭐ ⭐ **追加修订段（任务单 `#249` / `[ADR-0010 D4]`：⛔ 不改上面"音频循环仍不在本票范围"原文）**：
//!   上一条描述的是 `#249` 之前的事实，现仍逐字保留。自 `#249` 起音频片段循环已由
//!   [`crate::audio_clip`] 在实时侧**真的展开**（`AudioClipSpan::source_offset` 按遍
//!   指向素材内的循环区间，与 MIDI 的 `loop_repeat` 共用同一份模型语义）；
//!   非循环工程仍只有一个 `source_offset == 0` 的 span ⇒ 与展开之前**逐位相同**。
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

#[cfg(debug_assertions)]
use yeban_dsp::envelope::AdsrStage;
use yeban_dsp::filter::LadderFilter;
use yeban_dsp::math::db_to_gain;
use yeban_dsp::oscillator::{GLASS, HOLLOW, LfoWave, METALLIC, ORGAN, VOCAL};
use yeban_dsp::polysynth::{
    LfoTarget, NoteEvent, PolySynth, PolySynthParams, PolySynthTables, phase_increment,
    steal_fade_frames_for,
};
// ⭐ `#311b`：音高形状的**哨兵**（`u16::MAX`）—— 再导出，让 `crate::snapshot` 与
// 引擎的判据不必各自写一遍"没有形状"的魔数。
pub use yeban_dsp::polysynth::NO_PITCH_SHAPE;
use yeban_model::{CurveType, DeviceDefinition, DeviceKind, EntityId, PPQ};
use yeban_sfz::modulation::tune_ratio_from_cents;

use crate::drums::{DRUM_SLOTS, DrumHit, DrumMachine, DrumNoteMap, DrumVoice, DrumsParams};
use crate::sfz_instrument::{PreparedSfz, SfzVoicePool};
use crate::snapshot::{EngineSnapshot, SnapshotError};

/// 每个轨道槽的声部数（复音上限）。
///
/// **再导出**，不是第二份定义：唯一实现在
/// [`yeban_dsp::polysynth::VOICES_PER_SLOT`]（上移前是 `synth.rs` 里的 16）。
/// 与 `yeban-dsp::limiter` 上移后 `mixer.rs` 只 `pub use` 同形 [ARCH-DSP-001]。
pub use yeban_dsp::polysynth::VOICES_PER_SLOT as VOICES_PER_TRACK;

/// 声部池的**默认轨道槽数**（构造期确定，运行期可覆盖）。
///
/// 为什么从 16 提到 1024：16 条轨是"最小引擎"的尺寸，而**三部以上**的组曲、
/// 影视配乐、逐轨分轨渲染都会轻易越过它，越过之后引擎**不报错**——只是把多出来的
/// 轨道静默丢弃（只有 [`SynthEngine::track_drops`] 计数）。这不是"够不够用"的
/// 调优问题，是"工程规模稍大就少声部"的正确性问题。
///
/// 代价（构造期一次性分配，**不在音频线程上**）：每槽 ≈
/// `size_of::<TrackSlot>()`（16 个复音声部 ≈ 1.4 KiB ＋ 一台鼓机 13 616 B）
/// ⇒ 1024 槽 ≈ **15.4 MiB** 堆。⛔ 不是栈：见 [`SynthEngine`] 的字段文档。
///
/// 调小/调大用 [`SynthEngine::with_track_slots`]；上限 [`MAX_TRACK_SLOTS`]。
pub const DEFAULT_TRACK_SLOTS: usize = 1024;

/// 声部池支持的**轨道槽**硬上限（构造期校验，`with_track_slots` 越界即钳到它）。
///
/// 它⛔ 不再是"实际容量"，只是"参数调整的合法区间"的上端。取 2048 而不是更大的
/// 理由**不是**声部池本身（它每格 1.4 KiB，8192 也只 11 MiB），而是
/// `rt.rs` 里那批**与它同源**的表：`armed_pan_gains` / `armed_pan_l` /
/// `armed_pan_r` / `armed_strips` / `key_taps` / `routing_buses` 都按这个长度定长，
/// 而且其中几张在每个量子上都要整体拷一次（`rt.rs` 的 `#300` 声相段）。
/// 2048 是"两档"：默认 1024，够调高一倍，又不让那些**每量子**的拷贝变成
/// 几十 KiB 一次的常态开销。
pub const MAX_TRACK_SLOTS: usize = 2048;

/// 声部池的**最少**轨道槽数（至少要能装一条轨）。
pub const MIN_TRACK_SLOTS: usize = 1;

/// ⭐ **lane 组槽**容量（逐件处理，期 5）：组槽与轨槽平行、独立计数。
///
/// 每轨至多 [`yeban_model::MAX_LANE_CHAINS_PER_TRACK`] = 16 组；256 槽 =
/// 16 条满组轨。代价与轨槽同价（每槽一台鼓机 + 一份 SFZ 池，构造期一次性
/// 堆分配 ≈ 256 × 17 KiB ≈ 4.4 MiB，⛔ 不在音频线程上）。装不下的组在
/// 武装时计数（[`SynthEngine::group_drops`]），其轨整轨回退单路渲染
/// （⛔ 不做半组渲染 —— 部分 lane 化会让组间扼流/键控语义 silently 错位）。
pub const MAX_LANE_GROUP_SLOTS: usize = 256;

/// ⭐ 渲染体查哪张槽表（期 5）：轨槽与组槽共用同一份渲染实现，本枚举只是
/// 槽表选择器（见 `SynthEngine::render_bank_inner`）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotBank {
    /// 轨槽（接线前的唯一槽表）。
    Track,
    /// lane 组槽（期 5）。
    Group,
}

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

/// 振幅包络的四个**默认值**（秒 / 秒 / 0..1 / 秒）—— ⭐ `T-FIX-273`。
///
/// 这四个常量**逐字等于**器件 `yeban_dsp::polysynth` 的默认包络
/// （`DEFAULT_ATTACK_SECONDS = 0.005`、`DEFAULT_DECAY_SECONDS = 0.08`、
/// `DEFAULT_SUSTAIN = 0.7`、`DEFAULT_RELEASE_SECONDS = 0.05`）。为什么不直接引用器件
/// 常量：它们是**私有的**（器件只公开 `PolySynthParams::*()` 读数）。
///
/// "逐字相等"由判据 `ToneParams::bypass().poly_synth_params() == PolySynthParams::new()`
/// 钉住（判据 E1）：本层若把任何一个默认值写偏，那条等式当场变红。
const TONE_DEFAULT_ATTACK_S: f32 = 0.005;
/// 见 [`TONE_DEFAULT_ATTACK_S`]。
const TONE_DEFAULT_DECAY_S: f32 = 0.08;
/// 见 [`TONE_DEFAULT_ATTACK_S`]。
const TONE_DEFAULT_SUSTAIN: f32 = 0.7;
/// 见 [`TONE_DEFAULT_ATTACK_S`]。
const TONE_DEFAULT_RELEASE_S: f32 = 0.05;

/// 包络"时长"参数的钳制：非有限或负值 ⇒ 默认（`0.0` 合法 = 瞬时）。
///
/// 与器件 `PolySynthParams::with_envelope` 里的 `positive_or` **同口径**：
/// 让非法输入穿过去会产生 `NaN` 包络，那是整条链路的静默死亡。
#[must_use]
fn tone_duration_or_default(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        fallback
    }
}

/// 包络延音电平的钳制：非有限 ⇒ 默认，否则钳到 `0.0 ..= 1.0`（与器件同口径）。
#[must_use]
fn tone_sustain_or_default(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        TONE_DEFAULT_SUSTAIN
    }
}

/// 全局 LFO 的两个**默认值**（Hz / 深度）—— ⭐ `T-FIX-277`。
///
/// 这两个常量**逐字等于**器件 `yeban_dsp::polysynth` 的默认 LFO
/// （`DEFAULT_LFO_RATE_HZ = 5.0`、`DEFAULT_LFO_DEPTH = 0.0`）。为什么不直接引用器件
/// 常量：它们是**私有的**（器件只公开 `PolySynthParams::*()` 读数）。
///
/// "逐字相等"由判据 `ToneParams::bypass().poly_synth_params() == PolySynthParams::new()`
/// 钉住（判据 L1，见 `crates/yeban-engine/tests/synth_lfo.rs`）：本层若把任何一个
/// 默认值写偏，那条等式当场变红。波形与目标的默认值（正弦 / 音高）是枚举，
/// 没有对应的数值常量。
const TONE_DEFAULT_LFO_RATE_HZ: f32 = 5.0;
/// 见 [`TONE_DEFAULT_LFO_RATE_HZ`]。
const TONE_DEFAULT_LFO_DEPTH: f32 = 0.0;

/// LFO 速率的钳制：非有限或负值 ⇒ 默认，否则钳到 `0.0 ..= 100.0`
/// （与器件 `PolySynthParams::with_lfo` 同口径；`0.0` 合法 = LFO 冻结）。
#[must_use]
fn tone_lfo_rate_or_default(value: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value.min(100.0)
    } else {
        TONE_DEFAULT_LFO_RATE_HZ
    }
}

/// LFO 深度的钳制：非有限或负值 ⇒ 默认（`0.0` = 关），否则钳到 `0.0 ..= 1200.0`
/// （与器件 `PolySynthParams::with_lfo` 同口径；量纲按目标解释，见
/// [`ToneParams::with_lfo`]）。
#[must_use]
fn tone_lfo_depth_or_default(value: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value.min(1200.0)
    } else {
        TONE_DEFAULT_LFO_DEPTH
    }
}

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
/// 十七个 `Option` 字段各对应一个参数名；`None` = 本设备**没写**这个参数
/// （与"写了 `0`"不同：`osc2_level = 0` 是"显式关掉第二条支路"，`attack_s = 0`
/// 是"显式要一条瞬时起振"，`lfo_depth = 0` 是"显式关掉 LFO"）。
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
    attack_s: Option<f32>,
    decay_s: Option<f32>,
    sustain: Option<f32>,
    release_s: Option<f32>,
    lfo_wave: Option<f32>,
    lfo_rate_hz: Option<f32>,
    lfo_depth: Option<f32>,
    lfo_target: Option<f32>,
}

impl DeviceTone {
    /// 本设备是否是"音色来源"：至少写了一个**识别键**。
    ///
    /// 识别键 = `cutoff_hz`/`cutoff` **或**六个振荡器键中的任意一个。
    /// 只写 `resonance`/`res`/`drive` 的设备**仍然不算**来源 —— 这是上移前的规则
    /// （滤波器的主参数是截止频率），本票只把振荡器键**并列**加进来。
    ///
    /// ⭐ `T-FIX-273` 的四条包络键（`attack_s` / `decay_s` / `sustain` / `release_s`）
    /// **不是**识别键，与 `resonance`/`drive` 同一条理由：包络是**声部**的修饰，
    /// 它不回答"这台设备是不是音源"。只写包络键的设备仍然被跳过（`from_devices`
    /// 继续找下一条），因此"接通 ADSR"没有放宽音源判据 —— 既有工程里没有被
    /// 误认成音源的哑设备。
    ///
    /// ⭐ `T-FIX-277` 的四条 LFO 键（`lfo_wave` / `lfo_rate_hz` / `lfo_depth` /
    /// `lfo_target`）**同样不是**识别键，与包络键同一条理由：LFO 是**声部**的
    /// 修饰，它不回答"这台设备是不是音源"。只写 LFO 键的设备仍然被跳过
    /// （`from_devices` 继续找下一条），因此"接通 LFO"没有放宽音源判据。
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

/// 内置音色的**引擎侧**参数投影：一个四极梯形低通的三个旋钮 ＋ 两条振荡器支路
/// ＋ 振幅包络四个参数（⭐ `T-FIX-273` 接通后四项，见下面"## 振幅包络"）
/// ＋ 全局 LFO 四个参数（⭐ `T-FIX-277` 接通后四项，见下面"## 全局 LFO"）。
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
///     {name:"osc1_detune_cents", value: -6}  ├ osc2_detune_cents: f32
///     {name:"osc2_level", value: 0.5}        ├ attack_s: f32
///     {name:"osc2_detune_cents", value: 7}   ├ decay_s: f32
///     {name:"attack_s",  value: 0.005}       ├ sustain: f32
///     {name:"decay_s",   value: 0.08}        ├ release_s: f32
///     {name:"sustain",   value: 0.7}         ├ lfo_wave: u8
///     {name:"release_s", value: 0.05}        ├ lfo_rate_hz: f32
///     {name:"lfo_wave",  value: 0}            ├ lfo_depth: f32
///     {name:"lfo_rate_hz", value: 5.0}        └ lfo_target: u8
///     {name:"lfo_depth", value: 0.0}
///     {name:"lfo_target", value: 0}           ⇒ 本模块的投影整体删除，改成直读字段
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
///    ⭐ 四条包络键（`attack_s` / `decay_s` / `sustain` / `release_s`）**不是**识别键
///    （理由见 [`DeviceTone::is_source`]），它们跟着**已识别**的那条音源设备一起投影；
///    ⭐ 四条 LFO 键（`lfo_wave` / `lfo_rate_hz` / `lfo_depth` / `lfo_target`）
///    **同样不是**识别键（理由见 [`DeviceTone::is_source`]），它们跟着**已识别**的那条
///    音源设备一起投影；
/// 3. 十七个参数名（大小写不敏感）：`cutoff_hz`/`cutoff`、`resonance`/`res`、`drive`、
///    `osc1_wave`、`osc2_wave`、`osc1_level`、`osc1_detune_cents`、`osc2_level`、
///    `osc2_detune_cents`、`attack_s`、`decay_s`、`sustain`、`release_s`、
///    `lfo_wave`、`lfo_rate_hz`、`lfo_depth`、`lfo_target`；
/// 4. **一个识别键都没有 ⇒ [`ToneParams::bypass`]**：这是刻意选的默认值，
///    因为"模型层没给参数"与"用户把滤波器拧到旁通"在音频上应当同解 ——
///    这样既有的（无设备链的）工程逐位不变，也避免了给所有轨道硬塞一个未裁决的音色；
/// 5. 取值在构造期钳制（[`LadderFilter::configure`] 内部再钳一次 Nyquist）：
///    截止频率 `20 Hz ..= 0.45·fs`、共振/驱动 `0.0 ..= 1.0`；
/// 6. 非有限值 ⇒ **该参数**按默认取（截止 `12 kHz`、共振 `0`、驱动 `0`、
///    波形下标 `0`、`osc1_level` `1.0`（满）、`osc1_detune_cents` `0.0`、
///    `osc2_level` `0.0`（关）、`osc2_detune_cents` `0.0`、`attack_s` `0.005`、
///    `decay_s` `0.08`、`sustain` `0.7`、`release_s` `0.05`、`lfo_wave` `0`
///    （正弦）、`lfo_rate_hz` `5.0`、`lfo_depth` `0.0`（关）、`lfo_target` `0`
///    （音高）），绝不 `NaN`；
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
/// ## 振幅包络（⭐ `T-FIX-273`：`attack_s` / `decay_s` / `sustain` / `release_s`）
///
/// 器件侧 [`PolySynthParams::with_envelope`] 早就存在并有单元判据，但**引擎侧**
/// 此前没有这四个字段、`poly_synth_params()` 也从不调用它 ⇒ 所有内置 PolySynth 轨道
/// 只能用器件的默认包络（5 / 80 / 0.7 / 50 ms），工程里写的包络参数**一个也不进声音**。
/// 本票把四条投影接通：
///
/// - 参数名（大小写不敏感）：`attack_s` / `decay_s` / `sustain` / `release_s`；
/// - **默认值 = 器件默认值**（这是"接线不改变既有输出"的全部理由）：
///   [`TONE_DEFAULT_ATTACK_S`] `0.005`、[`TONE_DEFAULT_DECAY_S`] `0.08`、
///   [`TONE_DEFAULT_SUSTAIN`] `0.7`、[`TONE_DEFAULT_RELEASE_S`] `0.05`；
///   不写这四个键的工程与接通之前**逐位相同**（判据 E1）；
/// - **非法值回落本层默认**：三个"时长"参数非有限或**负值** ⇒ 该参数取默认
///   （与器件 `positive_or` 同口径；`0.0` 是**合法值** = 瞬时起振/瞬时释放，
///   不是"没写"）；`sustain` 非有限 ⇒ 默认，否则钳到 `0.0 ..= 1.0`；
///   本层因此**从不**把 `NaN`/`inf` 交给器件（判据 E4）；
/// - 包络键**不是识别键**：只写包络的设备仍不构成音源（见 [`DeviceTone::is_source`]）。
///
/// 包络系数含 `exp` ⇒ 落在下面的**超越函数类**；接线不改变这一分类。
///
/// ## 全局 LFO（⭐ `T-FIX-277`：`lfo_wave` / `lfo_rate_hz` / `lfo_depth` / `lfo_target`）
///
/// 器件侧 [`PolySynthParams::with_lfo`] 与 [`PolySynth`](yeban_dsp::polysynth::PolySynth)
/// 的每槽一份 LFO 状态早就存在并有单元判据，但**引擎侧**此前没有这四个字段、
/// `poly_synth_params()` 也从不调用它 ⇒ 所有内置 PolySynth 轨道只能用器件的默认
/// LFO（深度 `0` = 关），工程里写的 LFO 参数**一个也不进声音**。本票把四条投影接通：
///
/// - 参数名（大小写不敏感）：`lfo_wave` / `lfo_rate_hz` / `lfo_depth` / `lfo_target`；
///   波形与目标的**值是下标**（`f32` 参数先四舍五入、再钳到 `u8`，与 `wave_index`
///   同款取整口径；`0.5` 四舍五入到 `1`，见判据 L2 的注）：
///   波形 `0`=正弦 / `1`=三角 / `2`=方波 / `3`=锯齿（`3` 及以上都取锯齿）；
///   目标 `0`=音高 / `1`=振幅 / `2`=截止频率（`2` 及以上都取截止频率）；
/// - **默认值 = 器件默认值**（这是"接线不改变既有输出"的全部理由）：
///   [`TONE_DEFAULT_LFO_RATE_HZ`] `5.0`、[`TONE_DEFAULT_LFO_DEPTH`] `0.0`、
///   波形正弦、目标音高；不写这四个键的工程与接通之前**逐位相同**（判据 L1）；
/// - **非法值回落本层默认**：速率非有限或**负值** ⇒ `5.0`，否则钳到 `0.0 ..= 100.0`
///   （`0.0` 是**合法值** = LFO 冻结在 `shape(wave, 0)` 的常数值上）；深度非有限或
///   **负值** ⇒ `0.0`（关），否则钳到 `0.0 ..= 1200.0`（量纲按目标解释，见
///   [`ToneParams::with_lfo`]）；本层因此**从不**把 `NaN`/`inf` 交给器件（判据 L4）；
/// - LFO 键**不是识别键**：只写 LFO 的设备仍不构成音源（见 [`DeviceTone::is_source`]）；
/// - **作用域**：每槽一份全局 LFO（所有声部共用，见
///   [`yeban_dsp::polysynth`] 模块文档 §10）；per-note 颤音不在本票范围。
///
/// LFO 系数含 `sinf`（正弦波形）⇒ 落在下面的**超越函数类**；接线不改变这一分类。
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
    attack_s: f32,
    decay_s: f32,
    sustain: f32,
    release_s: f32,
    lfo_wave: LfoWave,
    lfo_rate_hz: f32,
    lfo_depth: f32,
    lfo_target: LfoTarget,
}

impl ToneParams {
    /// 旁通（**默认**）：渲染路径**完全不经过**滤波器。
    ///
    /// 不是"滤波器系数取成透明"——那样状态变量仍会吸收瞬态并染色。
    /// 旁通是**逐位恒等**的：`render_track` 根本不调用 `process`。
    ///
    /// 振荡器取**历史默认**：`osc1_level = 1.0`（满）、`osc1_detune_cents = 0.0`、
    /// `osc2_level = 0.0`（关）、`osc2_detune_cents = 0.0`；包络取**器件默认**
    /// （`0.005` / `0.08` / `0.7` / `0.05` 秒）；LFO 取**器件默认**
    /// （正弦 / `5.0` Hz / 深度 `0.0` / 音高目标 ⇒ 关）。
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
            attack_s: TONE_DEFAULT_ATTACK_S,
            decay_s: TONE_DEFAULT_DECAY_S,
            sustain: TONE_DEFAULT_SUSTAIN,
            release_s: TONE_DEFAULT_RELEASE_S,
            lfo_wave: LfoWave::Sine,
            lfo_rate_hz: TONE_DEFAULT_LFO_RATE_HZ,
            lfo_depth: TONE_DEFAULT_LFO_DEPTH,
            lfo_target: LfoTarget::Pitch,
        }
    }

    /// 显式配置音色（构造期调用；非有限值退回该参数的默认值）。
    ///
    /// 振荡器取**默认**：`osc1_wave = osc2_wave = 0`（[`HOLLOW`]）、第一条支路满电平
    /// 不失谐、`osc2` 关。要选波形、开第二条支路或给第一条支路设电平/失谐，接着调
    /// [`ToneParams::with_oscillators`] 与 [`ToneParams::with_osc1`]；要改包络接着调
    /// [`ToneParams::with_envelope`]；要改 LFO 接着调 [`ToneParams::with_lfo`]。
    /// 包络在这里取**器件默认**（判据 E1 的构造依据）；LFO 在这里取**器件默认**
    /// （判据 L1 的构造依据）。
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
            attack_s: TONE_DEFAULT_ATTACK_S,
            decay_s: TONE_DEFAULT_DECAY_S,
            sustain: TONE_DEFAULT_SUSTAIN,
            release_s: TONE_DEFAULT_RELEASE_S,
            lfo_wave: LfoWave::Sine,
            lfo_rate_hz: TONE_DEFAULT_LFO_RATE_HZ,
            lfo_depth: TONE_DEFAULT_LFO_DEPTH,
            lfo_target: LfoTarget::Pitch,
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

    /// 设置**振幅包络**四个参数（**构造期**；非法值退回本层的默认值）。
    ///
    /// ⭐ `T-FIX-273`：这是"工程参数真的进声音"的那条投影。器件侧
    /// [`PolySynthParams::with_envelope`] 早就存在，本层此前没有这四个字段 ⇒ 所有内置
    /// PolySynth 轨道只能用器件默认包络（5 / 80 / 0.7 / 50 ms）。
    ///
    /// - `attack_s` / `decay_s` / `release_s`：秒。**非有限或负值** ⇒ 该参数取本层默认
    ///   （[`TONE_DEFAULT_ATTACK_S`] / [`TONE_DEFAULT_DECAY_S`] / [`TONE_DEFAULT_RELEASE_S`]）；
    ///   `0.0` 是**合法值**（瞬时起振/瞬时释放），与"没写"（`None` ⇒ 默认）不同；
    /// - `sustain`：`0.0 ..= 1.0` 的延音电平。非有限 ⇒ [`TONE_DEFAULT_SUSTAIN`]，
    ///   否则钳到 `0.0 ..= 1.0`。
    ///
    /// ⚠ 本层**不**做"包络与滤波器旁通的联动"：包络挂在声部上，与滤波器是否旁通无关
    /// （`bypass()` 出来的音色照样可以是长起振）。
    #[must_use]
    pub fn with_envelope(
        mut self,
        attack_s: f32,
        decay_s: f32,
        sustain: f32,
        release_s: f32,
    ) -> Self {
        self.attack_s = tone_duration_or_default(attack_s, TONE_DEFAULT_ATTACK_S);
        self.decay_s = tone_duration_or_default(decay_s, TONE_DEFAULT_DECAY_S);
        self.sustain = tone_sustain_or_default(sustain);
        self.release_s = tone_duration_or_default(release_s, TONE_DEFAULT_RELEASE_S);
        self
    }

    /// 设置**全局 LFO**四个参数（**构造期**；非法值退回本层的默认值）。
    ///
    /// ⭐ `T-FIX-277`：这是"工程参数真的进声音"的那条投影。器件侧
    /// [`PolySynthParams::with_lfo`] 早就存在，本层此前没有这四个字段 ⇒ 所有内置
    /// PolySynth 轨道只能用器件默认 LFO（深度 `0` = 关）。
    ///
    /// - `wave`：波形（[`LfoWave`]；正弦/三角/方波/锯齿）；
    /// - `rate_hz`：速率（Hz）。**非有限或负值** ⇒ [`TONE_DEFAULT_LFO_RATE_HZ`]
    ///   （`5.0`）；否则钳到 `0.0 ..= 100.0`。`0.0` 是**合法值**（LFO 冻结在
    ///   `shape(wave, 0)` 的常数值上），与"没写"（`None` ⇒ 默认）不同；
    /// - `depth`：深度，**量纲按 `target` 解释**：音高目标是**音分**（`0..=1200`）、
    ///   振幅目标是**线性增益分数**（渲染时再钳到 `0..=1`）、截止频率目标是**八度**
    ///   （渲染时再钳到 `0..=4`）。**非有限或负值** ⇒ `0.0`（关）；否则钳到
    ///   `0.0 ..= 1200.0`。`0.0` 是**合法值**（关掉调制），与"没写"不同；
    /// - `target`：调制目标（[`LfoTarget`]；音高/振幅/截止三者互斥）。
    ///
    /// ⚠ 本层**不**做"LFO 与滤波器旁通的联动"：截止频率目标只在滤波器**启用**时
    /// 才出声（旁通时滤波器一次也不调用）；音高/振幅目标与旁通无关。
    #[must_use]
    pub fn with_lfo(mut self, wave: LfoWave, rate_hz: f32, depth: f32, target: LfoTarget) -> Self {
        self.lfo_wave = wave;
        self.lfo_rate_hz = tone_lfo_rate_or_default(rate_hz);
        self.lfo_depth = tone_lfo_depth_or_default(depth);
        self.lfo_target = target;
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
                    "attack_s" => tone.attack_s = Some(param.value),
                    "decay_s" => tone.decay_s = Some(param.value),
                    "sustain" => tone.sustain = Some(param.value),
                    "release_s" => tone.release_s = Some(param.value),
                    "lfo_wave" => tone.lfo_wave = Some(param.value),
                    "lfo_rate_hz" => tone.lfo_rate_hz = Some(param.value),
                    "lfo_depth" => tone.lfo_depth = Some(param.value),
                    "lfo_target" => tone.lfo_target = Some(param.value),
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
        .with_envelope(
            tone.attack_s.unwrap_or(TONE_DEFAULT_ATTACK_S),
            tone.decay_s.unwrap_or(TONE_DEFAULT_DECAY_S),
            tone.sustain.unwrap_or(TONE_DEFAULT_SUSTAIN),
            tone.release_s.unwrap_or(TONE_DEFAULT_RELEASE_S),
        )
        .with_lfo(
            LfoWave::from_index(wave_index(tone.lfo_wave)),
            tone.lfo_rate_hz.unwrap_or(TONE_DEFAULT_LFO_RATE_HZ),
            tone.lfo_depth.unwrap_or(TONE_DEFAULT_LFO_DEPTH),
            LfoTarget::from_index(wave_index(tone.lfo_target)),
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

    /// 起振时间（秒；⭐ `T-FIX-273`）。
    #[must_use]
    pub const fn attack_s(&self) -> f32 {
        self.attack_s
    }

    /// 衰减时间（秒；⭐ `T-FIX-273`）。
    #[must_use]
    pub const fn decay_s(&self) -> f32 {
        self.decay_s
    }

    /// 延音电平（0..1；⭐ `T-FIX-273`）。
    #[must_use]
    pub const fn sustain(&self) -> f32 {
        self.sustain
    }

    /// 释放时间（秒；⭐ `T-FIX-273`）。
    #[must_use]
    pub const fn release_s(&self) -> f32 {
        self.release_s
    }

    /// 全局 LFO 的波形（⭐ `T-FIX-277`）。
    #[must_use]
    pub const fn lfo_wave(&self) -> LfoWave {
        self.lfo_wave
    }

    /// 全局 LFO 的速率（Hz；⭐ `T-FIX-277`）。
    #[must_use]
    pub const fn lfo_rate_hz(&self) -> f32 {
        self.lfo_rate_hz
    }

    /// 全局 LFO 的深度（⭐ `T-FIX-277`；量纲按目标解释，见 [`Self::with_lfo`]）。
    #[must_use]
    pub const fn lfo_depth(&self) -> f32 {
        self.lfo_depth
    }

    /// 全局 LFO 的调制目标（⭐ `T-FIX-277`）。
    #[must_use]
    pub const fn lfo_target(&self) -> LfoTarget {
        self.lfo_target
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
    ///
    /// ⭐ `T-FIX-273`：包络在 `with_filter` **之后**接入（`.with_envelope(..)`），
    /// 四个字段一律取自本层（已钳制的）字段 ⇒ 不写包络键时是
    /// `0.005 / 0.08 / 0.7 / 0.05`，与 [`PolySynthParams::new`] **逐字段相同**
    /// （判据 E1，它同时钉住"默认值写偏"）。
    ///
    /// ⭐ `T-FIX-277`：LFO 在 `with_envelope` **之后**接入（`.with_lfo(..)`），
    /// 四个字段一律取自本层（已钳制的）字段 ⇒ 不写 LFO 键时是
    /// 正弦 / `5.0` Hz / `0.0` / 音高目标，与 [`PolySynthParams::new`] **逐字段相同**
    /// （判据 L1，它同时钉住"默认值写偏"）。
    #[must_use]
    pub fn poly_synth_params(&self) -> PolySynthParams {
        PolySynthParams::new()
            .with_oscillators(
                Self::osc_settings(self.osc1_wave, self.osc1_level, self.osc1_detune_cents),
                Self::osc_settings(self.osc2_wave, self.osc2_level, self.osc2_detune_cents),
            )
            .with_filter(self.cutoff_hz, self.resonance, self.drive, self.bypass)
            .with_envelope(self.attack_s, self.decay_s, self.sustain, self.release_s)
            .with_lfo(
                self.lfo_wave,
                self.lfo_rate_hz,
                self.lfo_depth,
                self.lfo_target,
            )
    }
}

impl Default for ToneParams {
    fn default() -> Self {
        Self::bypass()
    }
}

/// ⭐ `#341`：`at` 这一刻**更早开始且尚未结束**的音符数（`trigger=first` / `legato` 的输入）。
///
/// 口径：只看调度表里**下标更小**（更早开始）的音符 —— 同刻起音的音符不算"按住"
/// （`.sfz` 的 `trigger=first` 语义是"乐句第一个音"，同刻齐奏不构成连音）。
/// 复杂度 `O(index)`，每音符一次（⛔ 不在逐样本路径上）。
fn held_before(notes: &[ScheduledNote], index: usize, at: u64) -> u32 {
    let held = notes[..index]
        .iter()
        .filter(|other| other.end_sample > at)
        .count();
    u32::try_from(held).unwrap_or(u32::MAX)
}

/// 构造期算好的**一个已调度音符**（实时侧只读）。
///
/// 全部字段在 [`crate::snapshot`] 的投影里由模型算出：实时侧只做整数比较与
/// 逐样本合成，不做 tick 换算、不做音高换算（见模块文档 §1）。
///
/// ## ⭐ `pitch_shape`（`#311b`）：音高随时间变化的那条通道
///
/// 只带**下标**（[`NO_PITCH_SHAPE`] = 无），曲线本体在 [`NoteSchedule::pitch_shapes`]
/// 的旁表里 —— 内联 `Vec` 会破 `Copy`，而本结构在触发路径上逐音符拷贝
/// （48 处 `NoteEvent::new`、11 处 `ScheduledNote::new` 一个字都不用改）。
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
    /// ⭐ 音高形状下标（[`NoteSchedule::pitch_shapes`]；[`NO_PITCH_SHAPE`] = 无）。
    ///
    /// 它**不参与**任何音频算式，只被投影写、被触发路径读一次
    /// （`NoteEvent::with_pitch_shape`）；`Copy`/`PartialEq` 因此都不受影响。
    pitch_shape: u16,
}

impl ScheduledNote {
    /// 组装一个已调度音符（由 [`crate::snapshot`] 的投影调用）。
    ///
    /// `phase_inc` 在此处由 `freq_hz`/`sample_rate` 推出，而不是由调用方传入：
    /// 相位定标是**合成器的不变量**，不该让每个调用点各自记住 `2³²`。
    ///
    /// 音高形状默认 [`NO_PITCH_SHAPE`]（无弯音/滑音）⇒ 既有 11 个调用点与
    /// 全部手工夹具**一个字都不用改**。
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
            pitch_shape: NO_PITCH_SHAPE,
        }
    }

    /// ⭐ 挂一个音高形状下标（`#311b`；由 [`crate::snapshot`] 的投影调用）。
    #[must_use]
    pub const fn with_pitch_shape(mut self, shape: u16) -> Self {
        self.pitch_shape = shape;
        self
    }

    /// ⭐ 本音符的音高形状下标（[`NO_PITCH_SHAPE`] = 无）。
    #[must_use]
    pub const fn pitch_shape(&self) -> u16 {
        self.pitch_shape
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

/// ⭐ **弯音曲线上的一个点**（`#311b`）：音符局部**样本**偏移 ＋ 音分。
///
/// 坐标原本是模型层的 `(tick, cents)`（`ADR-0012` D1：tick 是**音符局部**坐标）；
/// [`crate::snapshot`] 在构造期用 `samples_per_tick` 把它换算成样本偏移 ——
/// 实时侧因此**不做任何 tick 数学**（与模块文档 §1 同一条纪律）。
///
/// ⚠ 相邻两个点的样本偏移可能**相等**（BPM/采样率极端时 1 tick 不足 1 样本，
/// 而模型只保证 tick 严格升序）：求值时对 `span == 0` 有专门的兜底，⛔ 不会除零。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BendPoint {
    /// 音符局部样本偏移（`tick × samples_per_tick` 四舍五入）。
    offset_samples: u64,
    /// 相对 `MidiNote::pitch` 的音分偏移（模型已保证 `-1200..=1200`）。
    cents: f32,
}

impl BendPoint {
    /// 组装一个弯音点（构造期；偏移与音分都是模型投影算好的）。
    #[must_use]
    pub const fn new(offset_samples: u64, cents: f32) -> Self {
        Self {
            offset_samples,
            cents,
        }
    }

    /// 音符局部样本偏移。
    #[must_use]
    pub const fn offset_samples(&self) -> u64 {
        self.offset_samples
    }

    /// 音分偏移。
    #[must_use]
    pub const fn cents(&self) -> f32 {
        self.cents
    }
}

/// ⭐ **滑音段**（`#311b`；`ADR-0012` D9 的落地形态）。
///
/// `cents(t) = target_cents × curve.ease(t / end_samples)`，`t >= end_samples` 之后
/// **保持** `target_cents`（与 D5 的"末点之后保持末值"同一条口径）。
/// `target_cents = (target_pitch − pitch) × 100`：十二平均律里一个半音**恰好** 100
/// 音分，因此端点**精确**落在目标音高上（⛔ 不是用 `log2` 反算出的近似值）。
///
/// `curve` 的类型是模型层的 [`CurveType`]，`ease` **只在模型层实现一份** ——
/// 本模块（乃至 `yeban-dsp`）⛔ 不复制公式。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlideSpan {
    /// 滑音时长（音符局部样本数）；模型保证 `duration_ticks != 0`，这里再防一次 0。
    end_samples: u64,
    /// 目标音高相对本音符的音分偏移（`(target − pitch) × 100`）。
    target_cents: f32,
    /// 曲线形状（`CurveType::ease` 是唯一实现）。
    curve: CurveType,
}

impl SlideSpan {
    /// 组装一个滑音段（构造期）。
    #[must_use]
    pub const fn new(end_samples: u64, target_cents: f32, curve: CurveType) -> Self {
        Self {
            end_samples,
            target_cents,
            curve,
        }
    }

    /// 滑音时长（音符局部样本数）。
    #[must_use]
    pub const fn end_samples(&self) -> u64 {
        self.end_samples
    }

    /// 目标音分偏移。
    #[must_use]
    pub const fn target_cents(&self) -> f32 {
        self.target_cents
    }

    /// 曲线形状。
    #[must_use]
    pub const fn curve(&self) -> CurveType {
        self.curve
    }

    /// 音符局部 `local` 处的滑音音分。
    #[must_use]
    pub fn cents_at(&self, local: u64) -> f32 {
        if self.end_samples == 0 || local >= self.end_samples {
            return self.target_cents;
        }
        #[allow(clippy::cast_precision_loss)]
        let t = local as f32 / self.end_samples as f32;
        self.target_cents * self.curve.ease(t)
    }
}

/// ⭐ **一个音符的音高形状**（`#311b`）：弯音曲线 ＋ 滑音，**相加**（`D9`）。
///
/// ## 坐标与求值口径（逐条对应 ADR）
///
/// - **D1**：点偏移是**音符局部**的 ⇒ 每个 `ScheduledNote`（含 `ratchet` 子脉冲与
///   循环副本）的局部时间从**它自己的** `start_sample` 算起 ⇒ **每个副本从 0 重放**
///   曲线（`D10`：每次重击是一段新生命）。
/// - **D4**（2026-10-25 裁决）：`local` 小于**首点**偏移 ⇒ 弯音部分取 **0**
///   （逻辑上的"弯音是绝对状态、默认值 0"，⛔ 不是外插首值）。
/// - **D5**：相邻点之间**线性**插值（单位 cents）；末点之后**保持**末值。
/// - **D9**：`total = bend + slide`（两条独立通路、叠加）。
///
/// ## 为什么不是"逐样本"求值
///
/// 本结构只在**控制率**上被调用（`yeban-dsp` 的音符局部 64 样本网格），
/// 逐样本路径上一次都不经过它。这是保"块切分无关"的前提，见
/// `yeban_dsp::polysynth::PITCH_UPDATE_SAMPLES` 的文档。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PitchShape {
    /// 弯音点（构造期已按样本偏移**升序**；空 = 无弯音）。
    bend: Vec<BendPoint>,
    /// 滑音（`None` = 无滑音）。
    slide: Option<SlideSpan>,
}

impl PitchShape {
    /// 组装一个音高形状（构造期；`bend` 必须已按偏移升序）。
    #[must_use]
    pub const fn new(bend: Vec<BendPoint>, slide: Option<SlideSpan>) -> Self {
        Self { bend, slide }
    }

    /// 弯音点（只读；诊断/判据用）。
    #[must_use]
    pub fn bend(&self) -> &[BendPoint] {
        &self.bend
    }

    /// 滑音段（只读；诊断/判据用）。
    #[must_use]
    pub const fn slide(&self) -> Option<&SlideSpan> {
        self.slide.as_ref()
    }

    /// 本形状是否**恒为 0**（空弯音 ＋ 无滑音）⇒ 投影可以不给它建表项。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bend.is_empty() && self.slide.is_none()
    }

    /// ⭐ 音符局部 `local` 处的**总音分**（弯音 ＋ 滑音；`D9`）。
    #[must_use]
    pub fn cents_at(&self, local: u64) -> f32 {
        self.bend_cents_at(local) + self.slide_cents_at(local)
    }

    /// 弯音部分（`D4` 首点前取 0、`D5` 线性插值、末点后保持）。
    #[must_use]
    pub fn bend_cents_at(&self, local: u64) -> f32 {
        let Some(first) = self.bend.first() else {
            return 0.0;
        };
        // ⭐ D4：首点之前**取 0**（⛔ 不外插首值）。
        if local < first.offset_samples {
            return 0.0;
        }
        // 最后一个 `offset <= local` 的下标（bend 非空且 local >= 首点 ⇒ >= 1）。
        let index = self
            .bend
            .partition_point(|point| point.offset_samples <= local);
        let lower = &self.bend[index - 1];
        let Some(upper) = self.bend.get(index) else {
            // ⭐ D5：末点之后**保持**末值。
            return lower.cents;
        };
        let span = upper.offset_samples - lower.offset_samples;
        if span == 0 {
            // 两个 tick 落到同一个样本（极端 BPM/采样率）：取后一个点的值，
            // ⛔ 不除零（那会产出 NaN 并让整条声部静音）。
            return upper.cents;
        }
        #[allow(clippy::cast_precision_loss)]
        let t = (local - lower.offset_samples) as f32 / span as f32;
        lower.cents + (upper.cents - lower.cents) * t
    }

    /// 滑音部分（`D9`；端点精确）。
    #[must_use]
    pub fn slide_cents_at(&self, local: u64) -> f32 {
        self.slide
            .as_ref()
            .map_or(0.0, |slide| slide.cents_at(local))
    }
}

/// 单轨的音符调度表（按 `start_sample` 升序，构造期分配、实时侧只读）。
///
/// # ⭐ 本表携带**两个**增益量（归属裁决见 `phase-status.md` 的 R711）
///
/// | 量 | 落在哪 | 谁拥有 |
/// | :--- | :--- | :--- |
/// | `ScheduledNote::gain`（`velocity_gain(v) × track_gain`）| 每个音符（**逐音符**）| ⭐ **引擎层**的轨增益 × ⭐ 复音/鼓机的力度 |
/// | [`Self::track_gain`]（`track_gain_scale(...)`，**与力度无关**）| 每条轨（**一个标量**）| ⭐ **引擎层**的轨增益（推子 × 静音/独奏门）|
///
/// ⚠ **为什么需要第二个**：SFZ 声部的力度增益由**乐器层**拥有（`yeban-sfz` 的
/// `velocity_gain[128]` / 交叉淡化 / `amp_veltrack`）⇒ 引擎⛔ 不能把含力度的那一份
/// （`ScheduledNote::gain`）交给它（那会**双重计力度**）。它只交**轨增益**这一半，
/// 与鼓机拿到的轨增益**同一个来源、同一个算子**（`snapshot::project_schedules` 里
/// 同一次 [`track_gain_scale`] 调用的结果）⇒ 两者**相乘、⛔ 不重复**。
#[derive(Clone, Debug, PartialEq)]
pub struct NoteSchedule {
    notes: Vec<ScheduledNote>,
    /// ⭐ 本轨的**轨增益门**（可闻门 ＝ 静音/独奏 × 推子；⛔ 与力度无关）。
    ///
    /// 默认 **`1.0`（单位增益）** ⇒ 用手工构造的调度表（测试夹具、离线拼接）驱动时，
    /// "没有轨参数"就是"不衰减"。⚠ 产品路径的值由
    /// [`crate::snapshot::project_schedules`] 写入（与 `note.gain` 里那一份同源）。
    track_gain: f32,
    /// ⭐ B8 轨级调律偏移（音分；缺省 `0.0` ⇒ 乘子恰 `1.0`）。
    ///
    /// 默认 **`0.0`（无偏移）** ⇒ 用手工构造的调度表（测试夹具、离线拼接）驱动时，
    /// "没有轨参数"就是"不移调"。⚠ 产品路径的值由
    /// [`crate::snapshot::project_schedules`] 写入（与 `note.freq_hz` 里那一份同源：
    /// 同一次投影、**同一个**音分值 ⇒ 复音与 SFZ 两条路听到的偏移一致）。
    tune_cents: f32,
    /// ⭐ **音高形状旁表**（`#311b`）：`ScheduledNote::pitch_shape` 是它的下标。
    ///
    /// 空 = 本轨没有一个音符带弯音/滑音 ⇒ 引擎传 `None` 探针 ⇒ 器件走**今天这条**
    /// 路径（一次浮点都不加）。容量受 [`MAX_NOTES_PER_TRACK`] 约束 ⇒ 下标远小于
    /// `u16::MAX`（`u16::MAX` 留给 `NO_PITCH_SHAPE` 当哨兵）。
    pitch_shapes: Vec<PitchShape>,
}

impl Default for NoteSchedule {
    /// 空表 ＋ **单位轨增益**（`1.0`，⛔ 不是 `0.0`：默认不是静音）＋ **零调律偏移**。
    fn default() -> Self {
        Self {
            notes: Vec::new(),
            track_gain: 1.0,
            tune_cents: 0.0,
            pitch_shapes: Vec::new(),
        }
    }
}

impl NoteSchedule {
    /// 由**已排序**的音符序列构造（轨增益门 ＝ **单位增益** `1.0`；调律偏移 ＝ `0.0`）。
    ///
    /// `notes` 必须按 `start_sample` 升序；[`crate::snapshot`] 的投影负责排序
    /// （实时侧的游标只向前推进，乱序会让音符被永久跳过）。
    #[must_use]
    pub const fn from_sorted(notes: Vec<ScheduledNote>) -> Self {
        Self {
            notes,
            track_gain: 1.0,
            tune_cents: 0.0,
            pitch_shapes: Vec::new(),
        }
    }

    /// ⭐ 写入本轨的音高形状旁表（构造期；见 [`Self::pitch_shapes`]）。
    #[must_use]
    pub fn with_pitch_shapes(mut self, pitch_shapes: Vec<PitchShape>) -> Self {
        self.pitch_shapes = pitch_shapes;
        self
    }

    /// ⭐ 本轨的音高形状旁表（只读）。
    #[must_use]
    pub fn pitch_shapes(&self) -> &[PitchShape] {
        &self.pitch_shapes
    }

    /// ⭐ 本轨是否有**任何**音高形状（判断"要不要传探针"的唯一入口）。
    #[must_use]
    pub fn has_pitch_shapes(&self) -> bool {
        !self.pitch_shapes.is_empty()
    }

    /// ⭐ 形状 `shape` 在音符局部 `local` 处的总音分（弯音 ＋ 滑音）。
    ///
    /// 下标越界 / 哨兵 [`NO_PITCH_SHAPE`] ⇒ **0.0**（⛔ 不 panic：这是实时路径）。
    /// 本函数**不分配**、只读，可在音频线程上调用（控制率，每声部每 64 样本一次）。
    #[must_use]
    pub fn cents_at(&self, shape: u16, local: u64) -> f32 {
        self.pitch_shapes
            .get(usize::from(shape))
            .map_or(0.0, |pitch_shape| pitch_shape.cents_at(local))
    }

    /// ⭐ 写入本轨的**轨增益门**（构造期；见类型文档的两增益量表）。
    #[must_use]
    pub const fn with_track_gain(mut self, track_gain: f32) -> Self {
        self.track_gain = track_gain;
        self
    }

    /// ⭐ 本轨的**轨增益门**（与力度无关：可闻门 × 推子）。
    ///
    /// 读数口径：产品路径下它**逐位等于** `SynthEngine` 槽位里那一份
    /// （`slot.gain_scale`）—— 两者都由 [`track_gain_scale`] 在同一次投影里算出。
    #[must_use]
    pub const fn track_gain(&self) -> f32 {
        self.track_gain
    }

    /// ⭐ 写入本轨的**调律偏移**（构造期；见 [`Self::tune_cents`]）。
    #[must_use]
    pub const fn with_tune_cents(mut self, tune_cents: f32) -> Self {
        self.tune_cents = tune_cents;
        self
    }

    /// ⭐ 本轨的**调律偏移**（音分；`0.0` ＝ 无偏移）。
    #[must_use]
    pub const fn tune_cents(&self) -> f32 {
        self.tune_cents
    }

    /// ⭐ 本轨调律偏移的**音高比乘子**（与 `note.freq_hz` 里那一份同一份换算）。
    ///
    /// `0.0` ⇒ 恰 `1.0`。触发期每轨算**一次**（调用方缓存），⛔ 不逐音符重算。
    #[must_use]
    pub fn tune_ratio(&self) -> f32 {
        tune_ratio_from_cents(self.tune_cents)
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
#[derive(Debug)]
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
    /// ⭐ **与力度无关**的增益门（可闻门 × 推子）⇒ 活音符的
    /// `gain = velocity_gain(velocity) * gain_scale`（⭐ 与已调度音符同一个来源）。
    ///
    /// ⚠ 槽**未占用**或**释放后**时必须是 `0.0`（⭐ 见 [`TrackSlot::empty`] 与
    /// [`SynthEngine::set_track_gain_scales`]）⇒ ⛔ 不留上一次的残值。
    gain_scale: f32,
    /// ⭐ **本轨的 SFZ 声部池**（`[SfzVoice; 16]`，定长全 `Copy`）。
    ///
    /// 音源分派次序是**鼓机 ＞ SFZ ＞ 复音**（见 [`SynthEngine::render_track`]）：
    /// 只有本轨**是** SFZ 音源时它才发声；鼓机/复音分支会把整个池**释放**
    /// （⛔ 不留上一件音源的尾巴 —— 与鼓机换回复音时的 `DrumMachine::reset` 同口径）。
    ///
    /// ⚠ 池里的声部状态**跨快照连续**（与复音声部池同口径）：本轨暂时不在快照里
    /// 时不释放，等它回来接着响。
    sfz: SfzVoicePool,
    /// 停止态实时试听推进的本地帧时钟（`Seek`/空闲时对齐到 `SynthEngine::position`，
    /// 运行态与 `render_track_with_cc` 窗口末端保持单调同步）。
    live_position: u64,
    /// 停止态存在由实时 `NoteOn` 触发的 SFZ 试听声部（与时间轴回放尾巴隔离）。
    live_sfz: bool,
    /// 停止态存在由实时 `NoteOn` 触发的鼓机试听击音（与时间轴回放尾巴隔离）。
    live_drum: bool,
    /// ⭐ **实时 CC64 的踏板状态**（T-FIX-274）。
    ///
    /// 为什么放在**槽**上、而不是只放在 [`SfzVoicePool`] 里：踏板是**演奏状态**，
    /// 与"本轨用哪种音源"无关。此前只有 SFZ 池记这个位 ⇒ 纯复音轨根本没有踏板
    /// 状态，实时 CC64 对合成器完全无效（本票的缺口）。
    ///
    /// ⚠ 与池里那份**同源**（都只由 [`SynthEngine::set_sustain_pedal`] 写）：两处都在，
    /// 是因为 SFZ 的释放族（`release` / `release_key`）要用池自己那份判谓词，
    /// 而合成器支路要用槽这份。
    pedal_down: bool,
    /// ⭐ 踏板踩下期间**已经松开**的键（T-FIX-274）：踏板抬起时逐个 `note_off`。
    ///
    /// 定长 `[bool; 128]`（⛔ 无分配 ⇒ 实时路径可写）；下标就是 MIDI 音高。
    /// 键重新按下时清位（它不再是"等踏板放掉"的挂起键）。
    ///
    /// ⚠ **一个音高一位就够**：`note_on_with_sources` 对同频会先 `note_off` 再触发
    /// （抢占同一槽）⇒ 同一个 pitch 上不会同时存在两个"被踏板挂起"的声部。
    /// 将来若支持同音多声部叠加，这里必须换成**计数器**，否则踏板抬起只能放掉一个
    /// （另一个会永久卡在 Sustain）。
    pedal_pending: [bool; 128],
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
            // ⭐ 空槽/释放后的**归零**（⛔ 不是残值）。
            gain_scale: 0.0,
            // ⭐ 空池（定长数组 ⇒ 零分配；`active` 全 `false` ⇒ 不产生任何样本）。
            // 淡出帧数按本槽的采样率定（与 `PolySynth` 同一个 3 ms 口径）。
            sfz: SfzVoicePool::new(sanitise_sample_rate(sample_rate)),
            live_position: 0,
            live_sfz: false,
            live_drum: false,
            // ⭐ 空槽没有踏板状态（T-FIX-274）：槽被复用给另一条轨时不许继承旧踏板。
            pedal_down: false,
            pedal_pending: [false; 128],
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
    /// **声部槽表**（容量 = 构造期传入的 `track_slots`，见 [`DEFAULT_TRACK_SLOTS`]）。
    ///
    /// ⚠ 从定长数组改成 `Box<[_]>` 的原因不是风格：1024 槽 × `size_of::<TrackSlot>()`
    /// ≈ 15 MiB，而 [`crate::rt::EngineRuntime`] 是**按值**构造、按值搬进回调的
    /// ⇒ 放在栈上会在打开设备时直接爆栈。分配次数 **1**，发生在
    /// [`SynthEngine::with_track_slots`]（⛔ 不在 `begin_snapshot` 里）。
    slots: Box<[TrackSlot]>,
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
    drum_slots: Box<[Option<(EntityId, DrumsParams)>]>,
    /// 累计的**鼓击触发数**（每触发一次 [`DrumMachine::trigger`] 记 1）。
    ///
    /// 它是"鼓机真的接在信号路径上"的覆盖度读数 —— 与 `EngineStats::metronome_clicks`
    /// 同族：`armed_drum_slots() > 0` 只说明"武装了"，这个数才说明"真的打了鼓"。
    drum_hits: u64,
    /// ⭐ 每槽**最近一次** [`Self::render_track`] 里**武装的音源数**（`0` = 还没渲染过）。
    ///
    /// 复音合成器**恒在场**（每个槽都有一台 ⇒ 至少 `1`），鼓机 `+1`
    /// （[`Self::drum_params`] 为 `Some`）、SFZ `+1`（`render_track` 收到 `Some(plan)`）。
    /// ⇒ 三件同轨 = `3`，而分派次序（鼓机 ＞ SFZ ＞ 复音）只让**一件**发声
    /// ⇒ [`Self::ignored_sources`] = `2`：这就是 `sfz-needs-3` 的**可读信号**
    /// （⛔ 不是把三件叠加）。`u8` ⇒ 定长数组，⛔ 不分配、不进逐样本路径。
    armed_sources: Box<[u8]>,
    /// `begin_snapshot` 的**音色投影暂存**（容量 = `track_slots`，构造期分配一次）。
    ///
    /// 为什么是字段而不是局部变量：`begin_snapshot` 运行在音频线程上
    /// [MUST-GATE-001] ⇒ 那里**一次也不许分配**；而"按 id 查本轨音色"又必须先把
    /// 一次性迭代器 `tones` 落下来才能对着 `slots` 查两遍。定长栈数组在容量可配置
    /// 之后不再可行（1024 项 × `size_of::<(EntityId, ToneParams)>`），
    /// ⇒ 用一个**复用的**堆缓冲，语义与原来的 `wanted` 栈数组逐位相同。
    wanted_tones: Box<[(EntityId, ToneParams)]>,
    /// `begin_snapshot` 的**鼓机投影暂存**（与 [`Self::wanted_tones`] 同一条纪律）。
    wanted_drums: Box<[Option<(EntityId, DrumsParams)>]>,
    /// 构造时**请求**的轨道槽数（与 [`Self::slots`] 的长度对照，用来看是否被钳过）。
    requested_track_slots: usize,
    /// ⭐ **lane 组槽表**（逐件处理，期 5）：与轨槽平行的一套声部/鼓机/SFZ 池，
    /// 容量 [`MAX_LANE_GROUP_SLOTS`]，构造期一次性分配（纪律同 [`Self::slots`]）。
    ///
    /// 一组 = 一条轨的一个键组：它用**该轨的**音色/鼓机参数渲染**只含本组键**
    /// 的分调度表（快照已切分），再由 rt 送组链、施组声相、累加回轨。
    /// 组槽独立的理由：组与组之间不能共享声部状态（扼流组、包络、SFZ 声部池
    /// 都是逐组语义）；而组槽**不**进 `EngineRuntime` 的第二分支共享 —— 它住在
    /// `SynthEngine` 里，A/B 试听的每个分支各有一台引擎 ⇒ 天然隔离。
    group_slots: Box<[TrackSlot]>,
    /// 每组槽一台鼓机（与 [`Self::drums`] 同一条尺寸纪律）。
    group_drums: Box<[DrumMachine<DRUM_SLOTS>]>,
    /// 每组槽的鼓机投影（`Some((轨, 参数))`；与 [`Self::drum_slots`] 同口径）。
    group_drum_slots: Box<[Option<(EntityId, DrumsParams)>]>,
    /// 每组槽的武装音源数读数（与 [`Self::armed_sources`] 同口径）。
    group_armed_sources: Box<[u8]>,
    /// 每组槽的**绑定**：`Some((轨, 组首键))` = 本槽服务于哪条轨的哪个组。
    /// 组身份用首键（模型校验保证组键集互不重叠、组内升序 ⇒ 首键唯一）。
    group_keys: Box<[Option<(EntityId, u8)>]>,
    /// 累计因组槽容量不足而没装上的组数（武装时计数，⛔ 不静默）。
    group_drops: u64,
}

impl SynthEngine {
    /// 构造：按**默认**轨道槽数（[`DEFAULT_TRACK_SLOTS`] = 1024）建好波形库与各槽器件。
    ///
    /// 需要别的容量用 [`Self::with_track_slots`]（这是唯一的容量入口）。
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self::with_track_slots(sample_rate, DEFAULT_TRACK_SLOTS)
    }

    /// 构造：按**指定**轨道槽数建好波形库与各槽器件（**允许分配**：这一步在打开设备前）。
    ///
    /// `track_slots` 会被钳进 `[`MIN_TRACK_SLOTS`, `MAX_TRACK_SLOTS`]`：
    /// 越界不报错（构造期没有可上报的地方），但会被记在
    /// [`Self::requested_track_slots`] 与 [`Self::track_slots`] 的差上 ⇒
    /// "参数被钳过"是可读的，⛔ 不是静默。
    ///
    /// 波形库按 [`ENGINE_WAVE_RECIPES`] 建**五张**表（下标 0 = [`HOLLOW`]）。
    /// `sample_rate` 先按传入值武装；真正的采样率在第一次
    /// [`begin_snapshot`](Self::begin_snapshot) 时按快照校准。
    ///
    /// # 内存
    ///
    /// 三条 `Box<[_]>` 各分配**一次**（`slots` / `drums` / `drum_slots` /
    /// `armed_sources` / 两份投影暂存），全部在这一次调用里完成，回调内一次也不分配。
    #[must_use]
    pub fn with_track_slots(sample_rate: u32, track_slots: usize) -> Self {
        let sample_rate_f32 = sanitise_sample_rate(sample_rate);
        let wave_recipes = ENGINE_WAVE_RECIPES.map(|(_, recipe)| recipe);
        let capacity = track_slots.clamp(MIN_TRACK_SLOTS, MAX_TRACK_SLOTS);
        Self {
            tables: PolySynthTables::from_recipes(&wave_recipes),
            sample_rate: sample_rate_f32,
            steal_fade_frames: steal_fade_frames_for(sample_rate_f32),
            position: 0,
            slots: (0..capacity)
                .map(|_| TrackSlot::empty(sample_rate))
                .collect(),
            track_drops: 0,
            notes_triggered: 0,
            // 鼓机池：**构造期一次性分配**（`vec!` 在这里，不在 `begin_snapshot` 里），
            // 分配次数 1（见字段文档的尺寸读数与理由）。
            drums: vec![DrumMachine::new(sample_rate); capacity].into_boxed_slice(),
            drum_slots: vec![None; capacity].into_boxed_slice(),
            drum_hits: 0,
            // ⭐ `0` = 本槽还没渲染过（见字段文档）；渲染一次即写入真实件数。
            armed_sources: vec![0; capacity].into_boxed_slice(),
            // 投影暂存：与槽位数同长（`ToneParams` 是 `Copy` ⇒ 用哨兵填充即可）。
            wanted_tones: vec![(EntityId::default(), ToneParams::bypass()); capacity]
                .into_boxed_slice(),
            wanted_drums: vec![None; capacity].into_boxed_slice(),
            requested_track_slots: track_slots,
            // ⭐ 组槽：固定容量、构造期一次性分配（纪律同轨槽）。
            group_slots: (0..MAX_LANE_GROUP_SLOTS)
                .map(|_| TrackSlot::empty(sample_rate))
                .collect(),
            group_drums: vec![DrumMachine::new(sample_rate); MAX_LANE_GROUP_SLOTS]
                .into_boxed_slice(),
            group_drum_slots: vec![None; MAX_LANE_GROUP_SLOTS].into_boxed_slice(),
            group_armed_sources: vec![0; MAX_LANE_GROUP_SLOTS].into_boxed_slice(),
            group_keys: vec![None; MAX_LANE_GROUP_SLOTS].into_boxed_slice(),
            group_drops: 0,
        }
    }

    /// 本引擎**实际**的轨道槽数（构造期确定，运行期不变）。
    #[must_use]
    pub fn track_slots(&self) -> usize {
        self.slots.len()
    }

    /// 构造时**请求**的轨道槽数（可能被钳过，见 [`Self::with_track_slots`]）。
    #[must_use]
    pub const fn requested_track_slots(&self) -> usize {
        self.requested_track_slots
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

    /// ⭐ 因 **SFZ 声部池满**（> [`SFZ_VOICES_PER_TRACK`](crate::sfz_instrument::SFZ_VOICES_PER_TRACK)
    /// 条区域同时匹配）而**真丢弃**的声部数（**累计**；各槽之和）。
    ///
    /// ⚠ 池满的常规出路现在是**窃取**（登记为 sfz-needs-2，已落地）⇒️ 本读数**不**计窃取：
    /// 只有"挂起者还没轮到发声就被再一次窃取换掉"才计入（⛔ 不静默）。
    /// 窃取数读 [`Self::sfz_voice_steals`]。
    #[must_use]
    pub fn sfz_voice_drops(&self) -> u64 {
        self.slots
            .iter()
            .fold(0u64, |total, slot| total.saturating_add(slot.sfz.dropped()))
    }

    /// ⭐ 因 **SFZ 声部池满**而**窃取**（换掉别人）的声部数（**累计**；各槽之和）。
    ///
    /// 与 [`Self::voice_steals`]（复音那条）**同一条口径**、同一把 3 ms 舵；
    /// 与 [`Self::sfz_voice_drops`] 配对：`池满的溢出 = 窃取 + 真丢弃`。
    #[must_use]
    pub fn sfz_voice_steals(&self) -> u64 {
        self.slots
            .iter()
            .fold(0u64, |total, slot| total.saturating_add(slot.sfz.stolen()))
    }

    /// ⭐ 某轨 SFZ 声部的**瞬时**占用（**量规**：可升可降）；`None` = 引擎里没有这条轨的槽。
    ///
    /// 与 [`Self::sfz_voice_drops`]（累计、单调）配对 —— 与鼓机的
    /// `drum_active_slots` / `drum_hits` 同一条口径。
    #[must_use]
    pub fn sfz_active_voices(&self, track: EntityId) -> Option<usize> {
        self.slots
            .iter()
            .find(|slot| slot.assigned && slot.id == track)
            .map(|slot| slot.sfz.active_count())
    }

    /// ⭐ **本槽最近一次渲染里武装的音源数**（`0..=3`；`None` = 该槽还没渲染过）。
    ///
    /// 口径：**复音合成器恒在场**（每个槽都有一台 ⇒ 至少 `1`）＋ 鼓机（
    /// [`Self::drum_params`] 为 `Some`）`+1` ＋ SFZ（`render_track` 收到 `Some(plan)`）`+1`。
    /// 它回答的是 `sfz-needs-3` 那个"用户看不出被忽略了"的问题：
    /// 三件同轨（鼓机 ＋ SFZ ＋ 复音）时本读数是 **`3`** ⇒ 见 [`Self::ignored_sources`]。
    ///
    /// ⚠ 这是**投影读数**（"最近一次渲染"的快照）⇒ 渲染中按构造每量子刷新；
    /// `None` 区分"还没渲染过"与"真的 0 件"（后者不可达：复音恒在场）。
    #[must_use]
    pub fn armed_sources(&self, track: EntityId) -> Option<usize> {
        let index = self.slot_index(track)?;
        match self.armed_sources[index] {
            0 => None,
            armed => Some(usize::from(armed)),
        }
    }

    /// ⭐ 本槽**被分派次序忽略**的音源件数 ＝ [`Self::armed_sources`] `− 1`
    /// （`0` = 只有一件音源，没有取舍）。
    ///
    /// 次序是**鼓机 ＞ SFZ ＞ 复音**（[`Self::render_track`]）：三件同轨 ⇒ **`2`**，
    /// 鼓机＋复音 ⇒ `1`，SFZ＋复音 ⇒ `1`，只有复音 ⇒ `0`。
    /// ⛔ 本读数**不**改变输出（那条口径另有 S7 逐位钉住）：它只让"被忽略"可见。
    #[must_use]
    pub fn ignored_sources(&self, track: EntityId) -> Option<usize> {
        self.armed_sources(track).map(|armed| armed - 1)
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
    ///
    /// ⭐ **一条舵，两件器件**：同一个数同时下推给每个槽的复音合成器与
    /// [`SfzVoicePool`]（SFZ 的池满窃取淡出与复音**同一条口径** ⇒ ⛔ 不是第二份常数）。
    /// ⭐ 控制线程**预热**某轨的 SFZ 效果总线（`ADR-0077` D5/D8）。
    ///
    /// 传 `frames` 为上界块长（引擎是 [`crate::block::DEFAULT_BLOCK_FRAMES`]）。
    /// 宿主在**发布快照之后、开始播放之前**调用：`BusBank`（≤4 台混响 ＋ 8 条
    /// 缓冲）在这里分配，音频回调里就只剩原地混音 ⇒ 不违 [MUST-GATE-001]。
    ///
    /// ⚠ 没有该轨的槽（快照还没建槽）⇒ **什么都不做**（不是错：下一块会用
    /// 兜底路径建，代价是一次分配）。
    /// ⭐ 控制线程**预占**一条轨的槽（`ADR-0077` D8 的"绑定期分配"清零）。
    ///
    /// 与 [`Self::begin_snapshot`] 的槽位分配循环**同一条口径**（占第一个空槽、
    /// `assigned = true`、`id` 写入、`live_position` 对齐当前播放头），但**只**
    /// 建槽：⛔ 不碰其它槽的 `present`／音色／鼓机武装。`present` 仍为 `false`
    /// ⇒ 真快照到达前这个槽**不会出声**。
    ///
    /// 为什么需要它：`TrackSlot::empty` 本身零分配，但落在**快照边界**（音频
    /// 线程）⇒ 总线预热没有可挂的槽。先在控制线程占好槽，再`warm_sfz_buses`，
    /// 音频回调里就没有"第一次分配"了。
    ///
    /// 已经有该轨的槽 ⇒ **严格空操作**（可重复调用）。
    pub fn reserve_track_slot(&mut self, sample_rate: u32, track: EntityId) {
        if self.reserved_slot_index(track).is_some() {
            return;
        }
        let free = self.slots.iter().position(|slot| !slot.assigned);
        if let Some(index) = free {
            self.slots[index] = TrackSlot::empty(sample_rate);
            self.slots[index].assigned = true;
            self.slots[index].id = track;
            self.slots[index].live_position = self.position;
        }
    }

    /// 某轨的 SFZ 总线构造次数（`None` ＝ 还没有该轨的槽）。
    ///
    /// ⚠ 与 [`Self::slot_index`] 的区别：**不要求 `present`** —— 预占的槽
    /// （`present == false`，见 [`Self::reserve_track_slot`]）也要读得到，
    /// 否则"预热是否命中"这条判据没法闭合。
    #[must_use]
    pub fn sfz_bus_builds(&self, track: EntityId) -> Option<u64> {
        self.reserved_slot_index(track)
            .map(|index| self.slots[index].sfz.bus_builds())
    }

    /// 按"已分配 ＋ 轨号"找槽（**不看 `present`**）：预占/预热用。
    fn reserved_slot_index(&self, track: EntityId) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.assigned && slot.id == track)
    }

    /// ⭐ 控制线程预热某轨的 SFZ 效果总线（`ADR-0077` D5/D8）。
    ///
    /// 传 `frames` 为上界块长（引擎是 [`crate::block::DEFAULT_BLOCK_FRAMES`]）。
    /// 先 [`Self::reserve_track_slot`] 再调本函数 ⇒ 音频回调里不再有第一次分配。
    pub fn warm_sfz_buses(&mut self, track: EntityId, plan: &PreparedSfz, frames: usize) {
        if let Some(index) = self.reserved_slot_index(track) {
            self.slots[index].sfz.warm_buses(plan, frames);
        }
    }

    /// 覆盖窃取淡出帧数（**判据/注入用**；`0` ＝ 硬窃取）。
    ///
    /// 产物路径不调用它：帧数由采样率与
    /// [`STEAL_RELEASE_SECONDS`](yeban_dsp::envelope::STEAL_RELEASE_SECONDS) 决定。
    /// ⭐ 同一条数值同时下推给每个槽的复音合成器与 `SfzVoicePool`。
    pub fn set_steal_fade_frames(&mut self, frames: u32) {
        self.steal_fade_frames = frames;
        for slot in &mut self.slots {
            slot.synth.set_steal_fade_frames(frames);
            slot.sfz.set_steal_fade_frames(frames);
        }
    }

    /// **离线/控制侧的武装入口**：从快照把工程里的乐器装进合成器。
    ///
    /// 镜像实时侧的武装序列（`rt.rs` 的 `begin_snapshot` ＋ `align_cursors`），并**排除 master**，
    /// 使离线渲染（`yeban-app --export-wav`）能与实时播放走**同一条**乐器装载路径。
    ///
    /// # ⚠ 谁在何时调用它（红线纪律，[MUST-GATE-001]）
    ///
    /// * ⛔ **不得在音频回调里调用**：`begin_snapshot` 会按新快照**增长区间表**、
    ///   `align_cursors` 会校正游标表 ⇒ **会分配** ⇒ 违反**红线 7**（实时回调内禁堆分配/释放、
    ///   禁锁等待、禁阻塞 I/O）。
    /// * ✅ **允许的调用点**：离线导出（`yeban-app --export-wav`）、控制线程加载/换工程、
    ///   以及测试。实时线换快照走的是 `rt.rs` 的 `SnapshotSlot` 中转（那是**预分配**路径），
    ///   ⛔ 与本函数不是同一条边。
    ///
    /// 幂等：同一快照重复调用只做区间对齐与游标校正（游标**只增不减** ⇒ 已触发过的音符
    /// 不会重复触发，在鸣的音符不被切断）。
    ///
    /// # Errors
    /// 快照里找不到 master 母线（`NoMasterBus`）⇒ 返回 [`SnapshotError`]。
    /// 调用方应先做**慢路径**校验，再进入渲染循环（⛔ 不要边渲染边试错）。
    pub fn arm_from_snapshot(&mut self, snapshot: &EngineSnapshot) -> Result<(), SnapshotError> {
        let master = snapshot.master();
        // ⚠ master 是**母线 id**，实时侧只把它当过滤器用（`rt.rs` 的 `.filter(|id| **id != master)`）：
        // ⛔ 不要求它出现在 `tracks()` 表里 —— 本批实测过一次：要求"master 必须在 tracks 表内"
        // 会让**合法**的最小工程（一条轨道 → master）被误判为无母线。
        // 因此这里只拦"**没有任何可武装的轨道/音色/鼓/SFZ**"的快照（空快照 ⇒ 谈不上武装）。
        if snapshot.tracks().is_empty()
            && snapshot.tones().is_empty()
            && snapshot.drums().is_empty()
            && snapshot.sfz().is_empty()
        {
            return Err(SnapshotError::NoMasterBus);
        }
        self.begin_snapshot(
            snapshot.sample_rate(),
            snapshot.tracks().keys().filter(|id| **id != master),
            snapshot.tones().iter().filter(|(id, _)| **id != master),
            snapshot.drums().iter().filter(|(id, _)| **id != master),
        );
        self.align_cursors(snapshot.schedules().iter().filter(|(id, _)| **id != master));
        // ⭐ 第三条快照派生状态：⭐ 逐轨增益门（⭐ 与投影同一个 [`track_gain_scale`]）。
        self.set_track_gain_scales(snapshot);
        // ⭐ 期 5：lane 组同刻武装（组绑定 + 组游标 + 组增益门）。
        self.arm_lane_from_snapshot(snapshot);
        Ok(())
    }

    /// ⭐ **lane 组武装**（逐件处理，期 5）：把快照的 lane 组表装进组槽。
    ///
    /// 组成三步（与轨武装同序）：[`Self::arm_lane_groups`] 绑定并下推轨级
    /// 音色/鼓机参数 → [`Self::align_group_cursors`] 校正组游标 →
    /// [`Self::set_group_gain_scales`] 写组增益门。没有 lane 组的快照里
    /// 三步都是空转（组槽 `present` 全清零）⇒ 既有路径逐位不变。
    pub fn arm_lane_from_snapshot(&mut self, snapshot: &EngineSnapshot) {
        let master = snapshot.master();
        self.arm_lane_groups(
            snapshot.sample_rate(),
            snapshot.lane_chains().iter().flat_map(|(track, groups)| {
                groups.iter().map(move |group| (*track, group.first_key()))
            }),
            snapshot.tones().iter().filter(|(id, _)| **id != master),
            snapshot.drums().iter().filter(|(id, _)| **id != master),
        );
        self.align_group_cursors(snapshot.lane_group_schedules());
        self.set_group_gain_scales(snapshot);
    }

    /// **单轨武装**：只把 `track` 一条轨投影进槽位（离线逐轨源用）。
    ///
    /// ## 为什么需要它（实测缺陷）
    ///
    /// 离线渲染的 `EngineTrackSource`（`yeban-render::engine_source`）**每个源独享一台引擎**
    /// 且只渲染自己那条轨，但它此前调用 [`Self::arm_from_snapshot`] 把快照里的**全部**轨道
    /// 塞进 [`MAX_TRACK_SLOTS`] 个槽。工程超过 16 条轨时，按快照顺序排在第 17 条起的轨
    /// 没有槽位 ⇒ [`Self::render_track`] 找不到槽位、整轨**数字静音**，而
    /// `track_drops` 只是一个没人读的计数（`render_master` 回执里没有任何提示）。
    ///
    /// 单轨武装后槽位上限与"工程里有几条源"彻底解耦：一个源只占一个槽
    /// （这条路径也⛔ 不改变实时侧 [`MAX_TRACK_SLOTS`] 的内存预算）。
    ///
    /// 语义与 [`Self::arm_from_snapshot`] **逐条对齐**：同一条 `begin_snapshot` 路径、
    /// 同一个 `align_cursors`、同一份 [`Self::set_track_gain_scales`] 的 solo/mute 口径
    /// （`any_solo` 仍按**整份快照**判定）。`track` 不在快照的 `tracks()` 里、或它就是
    /// master 母线时，什么都不武装（`render_track` 照旧输出静音 —— 与全量武装时
    /// 该轨拿不到槽位的语义相同）。
    ///
    /// # Errors
    ///
    /// 空快照 ⇒ [`SnapshotError::NoMasterBus`]（与 [`Self::arm_from_snapshot`] 同一条拒绝）。
    pub fn arm_track_from_snapshot(
        &mut self,
        snapshot: &EngineSnapshot,
        track: EntityId,
    ) -> Result<(), SnapshotError> {
        if snapshot.tracks().is_empty()
            && snapshot.tones().is_empty()
            && snapshot.drums().is_empty()
            && snapshot.sfz().is_empty()
        {
            return Err(SnapshotError::NoMasterBus);
        }
        let armable = track != snapshot.master() && snapshot.tracks().contains_key(&track);
        let tracks = armable.then_some(&track).into_iter();
        let tones = armable
            .then(|| snapshot.tones().get(&track))
            .flatten()
            .map(|tone| (&track, tone));
        let drums = armable
            .then(|| snapshot.drums().get(&track))
            .flatten()
            .map(|params| (&track, params));
        self.begin_snapshot(snapshot.sample_rate(), tracks, tones, drums);
        let schedules = armable
            .then(|| snapshot.schedules().get(&track))
            .flatten()
            .map(|schedule| (&track, schedule));
        self.align_cursors(schedules);
        self.set_track_gain_scales(snapshot);
        Ok(())
    }

    /// ⭐ 把每条轨"与力度无关"的增益门投影进槽位（**快照边界**调用）。
    ///
    /// ⚠ **超越函数留在边界**：本函数里的 `exp2`（经 [`track_gain`]）与既有
    /// `set_params` 的 `tan`/`exp` **同一类** ⇒ ⛔ 不进逐样本路径。
    /// ⚠ 未占用的槽显式归 `0.0` ⇒ ⭐ 槽释放/重用时⛔ 不留残值。
    pub fn set_track_gain_scales(&mut self, snapshot: &EngineSnapshot) {
        let any_solo = snapshot.tracks().values().any(|params| params.is_soloed());
        for slot in &mut self.slots {
            slot.gain_scale = if slot.assigned {
                snapshot.tracks().get(&slot.id).map_or(0.0, |params| {
                    track_gain_scale(
                        params.is_muted(),
                        params.is_soloed(),
                        params.is_solo_safe(),
                        any_solo,
                        params.volume_db(),
                    )
                })
            } else {
                0.0
            };
        }
    }

    /// ⭐ 组槽的增益门（期 5）：与 [`Self::set_track_gain_scales`] 同一算子，
    /// 组槽读**所属轨**的那一份（组没有自己的推子/静音/独奏 —— 轨级混音
    /// 语义在组求和之后还要再走一遍轨级链，⛔ 不在这里重复施加音量，
    /// 本门只承载调度表音符增益的同源折算，与轨槽口径一致）。
    pub fn set_group_gain_scales(&mut self, snapshot: &EngineSnapshot) {
        let any_solo = snapshot.tracks().values().any(|params| params.is_soloed());
        for slot in &mut self.group_slots {
            slot.gain_scale = if slot.assigned {
                snapshot.tracks().get(&slot.id).map_or(0.0, |params| {
                    track_gain_scale(
                        params.is_muted(),
                        params.is_soloed(),
                        params.is_solo_safe(),
                        any_solo,
                        params.volume_db(),
                    )
                })
            } else {
                0.0
            };
        }
    }

    // ## ⭐ 环音符路径的交接记录（⭐ 2026-10-12，⭐ 本模块的活音符接线）
    //
    // ⚠ 本节**不是规范**，⭐ 是给下一位接手者的**可查事实源**（⭐ 会话会散）：
    //
    // * ⭐ **已接线**：`EngineEvent::NoteOn` / `NoteOff` ⇒ [`SynthEngine::note_on`] /
    //   [`SynthEngine::note_off`] ⇒ 声部槽；⭐ 增益与已调度音符**同源**（[`track_gain_scale`]）。
    // * ⭐ **判据逼出来的三个缺陷**（⭐ 三处都不是事先看出来的）：
    //   1. ⭐ "活音符永不被回收" —— **被实测证否**：[`PolySynth::render`] 在包络 `Idle` 时
    //      当场回收声部（`polysynth.rs` 里 `!voice.env.is_active()` 那两处），⭐ 与
    //      `end_sample` 无关 ⇒ ⭐ `retire_finished` 只是**另一条**路径。
    //   2. ⭐ **提升点漏写 `freq_hz`**：⭐ 被窃取过的音符**自己松不掉** ⇒ ⭐ 挂音。
    //      ⭐ **(β) 已修**：⭐ `PolySynth` 的提升点现在写 `voice.freq_hz = pending.freq_hz;`
    //      （⭐ 缺陷 1）。
    //   3. ⭐ **淡出窗口盲区**：⭐ 窃取后的 3 ms 淡出窗口内到达的 `NoteOff` 只作用于
    //      **被窃取者**（⭐ 槽仍宣称旧频率）⇒ ⭐ 新音符此后只能由**后续**松键结束 ⇒
    //      ⭐ **用户可见的挂音**。⭐ **(β) 已修**：⭐ 一个声部记两个频率
    //      （`freq_hz` ＋ `pending_freq_hz`）、⭐ `note_off` 双支路、⭐ 提升点按
    //      `pending_released` 补 `gate_off`；⭐ 判据
    //      `a_release_for_the_stolen_note_works_inside_the_fade_window_and_after_promotion`
    //      （⭐ 缺陷 2）。
    // * ⭐ **`size_of::<PolyVoice>()` = 296**（⭐ 加 `freq_hz` 之前是 292；⭐ 加 `pending_*`
    //   两字段之后**实测仍是** 296 —— 增量落进既有填充）⇒ ⭐ 判据钉住了它；
    //   ⭐ 若将来再加字段，⭐ 那条判据会红。
    // * ⭐ **作业纪律**（⭐ 本路径三次停手换来的）：⭐ ① 文本替换**改前看 / 两端定界 / 改后看**；
    //   ⭐ ② **按所有构造点 grep**；⭐ ③ **未证明不说证明**；⭐ ④ ⭐ **一组自洽读数 ≠ 解释正确**
    //   —— ⭐ 一条读数一个机制、⭐ 必要时**再要一条独立读数**。
    //
    /// ⭐ **推一个活音符**（环事件 `EngineEvent::NoteOn` 的落地）。
    ///
    /// * ⭐ `gain = velocity_gain(velocity) * slot.gain_scale` ⇒ ⭐ 与**已调度**音符
    ///   **同一个来源**（⭐ 见 [`track_gain_scale`] 与 [`SynthEngine::set_track_gain_scales`]）；
    /// * ⭐ `start_sample = position`（⭐ 现在），⭐ `end_sample = u64::MAX`（⭐ **开区间**：
    ///   活音符由 `NoteOff` 结束，⛔ 不靠定时）；
    /// * ⭐ **先**对同一频率做一次 [`PolySynth::note_off_live`]（⭐ 裁定：⭐ 同 pitch 未松键又按
    ///   ⇒ **抢占同一槽、⭐ 重触发**）⇒ ⭐ 返回值告诉调用方"⭐ 是否发生了抢占/重触发"；
    ///   ⚠ 作用域是**活声部**（`end_sample == u64::MAX`）—— 时间轴排音带有限终点、由器件
    ///   自己收尾，⛔ 不在实时裁定的射程内（T-FIX-389）；
    /// * ⭐ **实时路径**：零分配、零锁、零 I/O。
    ///
    /// ⚠ ⭐ `u64::MAX` 的**准确**代价（⭐ 实测更正）：⭐ [`PolySynth::retire_finished`]
    /// 的谓词要求 `end_sample <= position` ⇒ ⭐ 它**对活音符不适用**；⭐ 但 ⭐ [`PolySynth::render`]
    /// 会在包络回到 `Idle` 时**当场回收**声部（⭐ `polysynth.rs:1273`／`:1281`，⭐ 与 `end_sample` 无关）
    /// ⇒ ⭐ **松键的活音符照常回收** ✓；⭐ **未松键者持续占声部**（⭐ 这是对的），⭐ 直到池满被窃取
    /// ⇒ ⭐ 有界（⭐ 每槽 ≤ [`VOICES_PER_TRACK`]）。
    pub fn note_on(&mut self, track: EntityId, pitch: u8, velocity: u8) -> bool {
        self.note_on_with_sources(track, pitch, velocity, None, None, true)
    }

    /// ⭐ **松一个活音符**（环事件 `EngineEvent::NoteOff` 的落地）。
    ///
    /// 返回"⭐ 是否找到了匹配的活跃声部"：⭐ `false` ⇒ ⭐ 调用方按**合并口径**登记
    /// （⭐ 不区分"⭐ 从未存在"／"⭐ 已被窃取或换音"）。
    /// ⭐ **实时路径**：零分配、零锁、零 I/O。
    pub fn note_off(&mut self, track: EntityId, pitch: u8) -> bool {
        self.note_off_with_sources(track, pitch, None, true)
    }

    /// 按轨主音源次序（鼓机 ＞ SFZ ＞ 复音）触发实时试听 `NoteOn`（零分配、零锁、零 I/O）。
    ///
    /// 返回是否发生声部窃取或同键重触发。
    pub fn note_on_with_sources(
        &mut self,
        track: EntityId,
        pitch: u8,
        velocity: u8,
        sfz: Option<&PreparedSfz>,
        cc: Option<&dyn Fn(u8) -> u8>,
        running: bool,
    ) -> bool {
        let Some(index) = self.slot_index(track) else {
            return false;
        };
        if !self.slots[index].present {
            return false;
        }
        if !running && !self.has_live_source_at(index) {
            self.slots[index].live_position = self.position;
        }
        let start_clock = if running {
            self.position
        } else {
            self.slots[index].live_position
        };

        if let Some((_, params)) = self.drum_slots[index] {
            self.slots[index].sfz.silence();
            self.slots[index].live_sfz = false;
            if let Some(voice) = params.notes().voice_for(pitch) {
                let gain = velocity_gain(velocity) * self.slots[index].gain_scale;
                let before_steals = self.drums[index].voice_steals();
                self.drums[index].trigger(DrumHit::new(voice, start_clock, gain));
                self.slots[index].live_drum = true;
                self.drum_hits = self.drum_hits.saturating_add(1);
                self.notes_triggered = self.notes_triggered.saturating_add(1);
                return self.drums[index].voice_steals() > before_steals;
            }
            return false;
        }

        if let Some(plan) = sfz {
            let mut held = 0u32;
            for voice in self.slots[index].sfz.voices() {
                if voice.is_active() && voice.end_sample() > start_clock {
                    held = held.saturating_add(1);
                }
            }
            self.slots[index].sfz.set_held_notes(Some(held));
            let before_steals = self.slots[index].sfz.stolen();
            let created = self.slots[index].sfz.trigger_note_with_pitch(
                plan,
                pitch,
                velocity,
                start_clock,
                u64::MAX,
                self.slots[index].gain_scale,
                cc,
                NO_PITCH_SHAPE,
                0.0,
            );
            if created > 0 {
                self.slots[index].live_sfz = true;
                self.notes_triggered = self
                    .notes_triggered
                    .saturating_add(u64::try_from(created).unwrap_or(u64::MAX));
            }
            return self.slots[index].sfz.stolen() > before_steals;
        }

        let freq_hz = yeban_dsp::math::note_to_hz(f32::from(pitch));
        // ⭐ 键又按下了 ⇒ 它不再是"等踏板放掉"的挂起键（T-FIX-274）。
        self.slots[index].pedal_pending[usize::from(pitch)] = false;
        // ⭐ 池满 ⇒ 这一次会走窃取路径（⭐ 触发**之前**量，⭐ 触发之后就不是"满"了）。
        let pool_full = self.slots[index].synth.active_voices() >= VOICES_PER_TRACK;
        // ⭐ 同 pitch 重触发：⭐ 先松、⭐ 再触发（⭐ 裁定：⭐ 抢占同一槽）。
        // ⚠ **只松活声部**（T-FIX-389）：时间轴上可能正有同音高排音（它们带有限
        // `end_sample`、由器件自己收尾）。沿用无作用域的 `note_off` 会把**时间轴那条**
        // 挑走（`start_sample` 更早）⇒ 用户按一个键、回放里的音符被截短，而这条刚建的
        // 活声部再没有事件会放它。裁定只对"用户按下的"生效 ⇒ 用 `note_off_live`。
        let retriggered = self.slots[index].synth.note_off_live(freq_hz) > 0;
        let gain = velocity_gain(velocity) * self.slots[index].gain_scale;
        self.slots[index].synth.note_on(
            NoteEvent::new(self.position, u64::MAX, freq_hz, gain),
            &self.tables,
        );
        pool_full || retriggered
    }

    /// 按轨主音源次序释放由实时试听触发的 `NoteOff`（零分配、零锁、零 I/O）。
    ///
    /// ⭐ 合成器路径**每次调用最多放掉一个**同频试听声部（T-FIX-275：同音重叠时只放
    /// 最早触发的那个，⛔ 不再把另一个仍按住的音符一起送进释放段）；重复松键才逐个放。
    /// SFZ / 鼓组路径的释放语义不变（它们按 `pitch` 自己配对）。
    ///
    /// ⚠ **复音支路的作用域是活声部**（T-FIX-389）：只放 [`PolySynth::note_off_live`]
    /// 认的那些（`end_sample == u64::MAX`，即实时按键建的声部）。时间轴排音带有限终点、
    /// 由器件在终点自己关 gate ⇒ 实时松键⛔ 不许截断正在回放的音符；当该频率上只有
    /// 时间轴声部时本方法返回 `false`（调用方按孤儿松键登记，而不是"找到了"）。
    ///
    /// 返回是否命中了活跃或挂起的试听声部（返回 `false` 时调用方计为孤儿松键）。
    pub fn note_off_with_sources(
        &mut self,
        track: EntityId,
        pitch: u8,
        sfz: Option<&PreparedSfz>,
        running: bool,
    ) -> bool {
        let Some(index) = self.slot_index(track) else {
            return false;
        };
        let mut released = false;
        let freq_hz = yeban_dsp::math::note_to_hz(f32::from(pitch));
        if self.slots[index].pedal_down {
            // ⭐ **踏板踩下**（T-FIX-274）：键松开**不**停止声部，只记挂起 —— 与 SFZ
            // 支路"attack 声部不停，等踏板"同一条语义。真正的释放发生在踏板抬起
            // （[`Self::set_sustain_pedal`]）。
            //
            // ⚠ 返回值口径**不变**：只有"确实有同频声部被这次松键挂住"才算命中。
            // 否则调用方会把它记成孤儿松键（`orphan_note_offs` 是诊断读数，⛔ 不虚增）。
            if self.slots[index].synth.holds(freq_hz) {
                self.slots[index].pedal_pending[usize::from(pitch)] = true;
                released = true;
            }
        } else if self.slots[index].synth.note_off_live(freq_hz) > 0 {
            released = true;
            // 没踩踏板时不该有挂起位；顺手清掉（轨道换过音源、早前的踏板没抬起来
            // 这类残留），⛔ 不让它在下一次踏板抬起时误放一个刚松的键。
            self.slots[index].pedal_pending[usize::from(pitch)] = false;
        }
        if let Some((_, params)) = self.drum_slots[index] {
            if self.slots[index].live_drum && params.notes().voice_for(pitch).is_some() {
                released = true;
            }
            return released;
        }
        if let Some(plan) = sfz {
            let off_clock = if running {
                self.position
            } else {
                self.slots[index].live_position
            };
            let pedal_down = self.slots[index].sfz.pedal_down();
            if pedal_down {
                // ⭐ **踏板踩下**（本片）：键松开**不**停止声部（等踏板松开），也**不**
                // 触发 `trigger=release` 的区域（只在 `PedalUp` 时响）；`release_key`
                // 照规范立刻响。挂起位在这里置上，好让踏板松开时知道要放掉谁。
                if self.slots[index]
                    .sfz
                    .voices()
                    .iter()
                    .any(|voice| voice.is_active() && voice.note() == pitch)
                {
                    released = true;
                }
                self.slots[index].sfz.mark_sustained(pitch);
            } else if self.slots[index].sfz.release_note(plan, pitch, off_clock) {
                released = true;
            }
            // ⭐ **释放族真的发声**（本片）：note-off 不只缩短已有声部，还按
            // `trigger=release` / `release_key` 建**新**声部（`TriggerEvent::NoteOff`）。
            // ⚠ 与上面那句**同一帧**：缩短与新建都用 `off_clock` ⇒ 释放采样从松键那一帧起。
            // ⚠ 踏板踩下时事件带 `pedal_down: true` ⇒ 只有 `release_key` 会响（谓词在
            // `yeban-sfz` 的 `Trigger::responds_to`）。
            let created = self.slots[index].sfz.trigger_release(
                plan,
                pitch,
                off_clock,
                self.slots[index].gain_scale,
                None,
            );
            if created > 0 {
                released = true;
                self.slots[index].live_sfz = true;
                self.notes_triggered = self
                    .notes_triggered
                    .saturating_add(u64::try_from(created).unwrap_or(u64::MAX));
            }
        }
        released
    }

    /// ⭐ **延音踏板（CC64）**（本片）：设状态，并在 **down → up** 时放掉被踏板挂住的键。
    ///
    /// 语义（规范逐字见 `yeban-sfz` 的 `TriggerEvent`）：
    /// * 踩下时键松开 ⇒ `release_key` 立刻响、`release` 等踏板；attack 声部**不**停；
    /// * 踏板松开 ⇒ 每个被挂住的键：`release_note`（真正停声部）＋ `PedalUp`
    ///   （只让 `release` 区响，⛔ 不重复播 `release_key`）。
    ///
    /// `sfz` 是**该轨当前**的摊平表（踏板松开那一刻建释放声部要用它）；`None` ⇒
    /// 只能清挂起位（没有区域表就建不出声部，⛔ 不猜）。
    ///
    /// ⭐ **踏板是演奏状态，与音源无关**（T-FIX-274）：本方法对**纯复音轨**同样生效
    /// （`sfz = None`），把踏板踩下期间松开的合成器键挂起，抬起时逐个 `note_off`。
    /// 状态没变（重复踩下/重复抬起）⇒ 直接返回 `0`，⛔ 不做任何工作。
    ///
    /// 返回踏板松开时**新建的**释放声部数（只有 SFZ 会建声部；合成器支路只是让
    /// 已有声部进 Release，不计入）。零分配、零锁、零 I/O ⇒ 可在实时路径调用。
    pub fn set_sustain_pedal(
        &mut self,
        track: EntityId,
        down: bool,
        sfz: Option<&PreparedSfz>,
    ) -> usize {
        let Some(index) = self.slot_index(track) else {
            return 0;
        };
        // ⚠ 顺序：先取池的"这次是不是抬起"，再写两处状态 —— 池的返回值是它自己
        // 那份状态机的跃变，与本方法末尾的早退判据不是同一件事。
        let pool_released = self.slots[index].sfz.set_pedal(down);
        let was_down = self.slots[index].pedal_down;
        self.slots[index].pedal_down = down;
        if down || !(was_down || pool_released) {
            // 踩下（或状态没变）⇒ 没有要放掉的键。
            return 0;
        }
        // 踏板松开 ①：复音支路 —— 把踩踏板期间松开的键真正送进 Release。
        //
        // ⭐ **一个 pitch 放掉它的全部声部**（T-FIX-406）：`pedal_pending` 每个 pitch 只有
        // 一个 bool，而同一频率上可能同时有好几个仍被按住的声部（时间轴重叠的同音音符
        // ＋ 被实时 `NoteOn` 抢先触发的试听声部）。`note_off` 每次最多放一个
        // （T-FIX-275 语义）⇒ 单次调用会留下一半，其中 `end_sample = u64::MAX` 的试听
        // 声部**再无任何事件能释放**（复验探针：抬起后尾部 RMS 0.46 的持续音）。
        // `release_all` 才是"抬起踏板"该有的形状（⛔ 不是 `while holds() { note_off() }`：
        // `holds` 把 Release 段也算"有"、`note_off` 在释放段会幂等返回 1 ⇒ 那个循环不退出）。
        for pitch in 0..128u8 {
            if !self.slots[index].pedal_pending[usize::from(pitch)] {
                continue;
            }
            self.slots[index].pedal_pending[usize::from(pitch)] = false;
            let freq_hz = yeban_dsp::math::note_to_hz(f32::from(pitch));
            self.slots[index].synth.release_all(freq_hz);
        }
        // 踏板松开 ②：SFZ 支路 —— 逐个放掉挂起的键（原样）。
        let now = self.position;
        let mut created = 0usize;
        for pitch in 0..128u8 {
            if !self.slots[index].sfz.sustained(pitch) {
                continue;
            }
            if let Some(plan) = sfz {
                let _ = self.slots[index].sfz.release_note(plan, pitch, now);
                created += self.slots[index].sfz.trigger_pedal_up(
                    plan,
                    pitch,
                    now,
                    self.slots[index].gain_scale,
                    None,
                );
            }
            self.slots[index].sfz.clear_sustained(pitch);
        }
        if created > 0 {
            self.slots[index].live_sfz = true;
            self.notes_triggered = self
                .notes_triggered
                .saturating_add(u64::try_from(created).unwrap_or(u64::MAX));
        }
        created
    }
    /// ⭐ **是否已经有轨道占过槽位**（⭐ 即"⭐ 本运行时至少武装过一次快照"）。
    ///
    /// ⚠ 用途：⭐ 首块的环音符事件在**快照武装之前**出队 ⇒ ⭐ 槽位还没影 ⇒
    /// `note_on` 早退、⭐ tick 0 的音符被丢。⭐ 本探针**不新增状态**：⭐ 槽位只会被
    /// `begin_snapshot` 占用 ⇒ ⭐ "⭐ 有槽位 ≠ 空"就是"⭐ 武装过"的见证。
    #[must_use]
    pub fn has_armed_tracks(&self) -> bool {
        self.slots.iter().any(|slot| slot.assigned)
    }

    /// 因 [`MAX_TRACK_SLOTS`] 耗尽而未获槽位的轨道次数。
    #[must_use]
    pub const fn track_drops(&self) -> u64 {
        self.track_drops
    }

    /// 累计触发过的音符数。
    ///
    /// ⚠ 口径（必须与 [`Self::poly_notes_triggered`] 一起读）：本计数器在**两条**
    /// 派发分支上都 `+1` —— 鼓机分支（紧接着 [`DrumMachine::trigger`]）与复音分支
    /// （紧接着 `PolySynth::note_on`）⇒ 它数的是"**两条音源合起来**派发了几次"。
    /// 想单独要复音那一条，读 [`Self::poly_notes_triggered`]（器件侧），
    /// 或做这条减法：`notes_triggered − drum_hits`。
    #[must_use]
    pub const fn notes_triggered(&self) -> u64 {
        self.notes_triggered
    }

    /// **复音合成器器件自己**累计接收的触发数（各槽之和；单位：次；0 = 从没触发）。
    ///
    /// 它不是 [`Self::notes_triggered`] 的第二份副本，而是那条**内部一致性判据**的
    /// 另一侧 —— 与鼓机那一对（[`Self::drum_hits`] / [`Self::drum_triggers`]）**同款**：
    /// 引擎在派发处记数，器件在 `note_on` 入口记数，两侧数的是同一个事件。
    ///
    /// 口径（三条，都要与判据一起读）：
    ///
    /// 1. 读数取 `PolySynth::notes_triggered`（器件里**早就存在**的 getter，
    ///    住在 `crates/yeban-dsp/src/polysynth.rs`，现位于第 828 行），
    ///    而 `yeban-engine` 此前**一个读者都没有**（量法：
    ///    `grep -rn 'slot\.synth\.' crates/yeban-engine/src` 在本次改动前命中
    ///    **12** 个器件调用点 = 11 个方法名（`set_steal_fade_frames` 出现两次）——
    ///    `voice_steals` / `set_steal_fade_frames` / `voices` / `debug_voice` /
    ///    `active_voices` / `reset` / `set_sample_rate` / `set_params` /
    ///    `retire_finished` / `note_on` / `render` —— 其中 `notes_triggered` **零**命中；
    ///    本次改动后同一条命令命中 **13** 行，多的那一条就是本读数）。
    ///    它只读一个 `u64` 字段：不碰样本、不分配、不加锁 ⇒ 渲染输出逐位不变；
    /// 2. **恒等式**（等号，无容差；静止点上）：
    ///    `notes_triggered − drum_hits == poly_notes_triggered`。
    ///    成立的理由是两条派发分支的记账形状：鼓机分支同时 `+1` 到
    ///    [`Self::drum_hits`] 与 [`Self::notes_triggered`]，复音分支只 `+1` 到
    ///    [`Self::notes_triggered`] 并调用一次 `note_on`（器件那一侧**每次都** `+1`）
    ///    ⇒ 相减恰好隔离出复音那一条。任何"派发了却没到器件"（或反之）的接线错误
    ///    都会让这条等号当场不成立，而从音频输出反推不出来；
    /// 3. 器件计数器**只增不减**，且器件自己的 `reset` 只清在响的声部、**不**清它
    ///    ⇒ 本读数是引擎生命周期里的累计量。槽位一旦占用就**不会**被重建
    ///    （`begin_snapshot` 只为 `assigned == false` 的槽建新器件，而 `assigned`
    ///    只从 `false` 变 `true`）⇒ 各槽计数器不会被悄悄清零。
    ///
    /// 为什么需要它：[`Self::notes_triggered`] 混了两条音源 ⇒ 只读它无法区分
    /// "复音轨真的触发了"与"全是鼓机在打"；而 [`Self::voice_steals`] 只在
    /// **声部池溢出**时才推进 ⇒ 一个从不溢出的复音工程在读数面上没有"触发过"的见证。
    /// 全部轨都不是复音合成器时（默认）**恒为 0**。
    ///
    /// 读它只汇总 16 个 `u64` 字段（`fold`），与 [`Self::voice_steals`] /
    /// [`Self::drum_triggers`] 同一条路径：零分配、零锁、零 I/O。
    #[must_use]
    pub fn poly_notes_triggered(&self) -> u64 {
        self.slots.iter().fold(0u64, |total, slot| {
            total.saturating_add(slot.synth.notes_triggered())
        })
    }

    /// **复音合成器此刻占用着的声部数**（各槽之和；单位：声部；0 = 一个都没有）。
    ///
    /// 这是一个**量规**（可升可降），不是累计量 —— 与上面那一族的每一个读数都不同：
    /// [`Self::notes_triggered`] / [`Self::poly_notes_triggered`] / [`Self::voice_steals`]
    /// 都只增不减，它们回答"曾经发生过什么"；本读数回答"**现在**占了多少"。
    ///
    /// 口径（三条，都要与判据一起读）：
    ///
    /// 1. 读数取 `PolySynth::active_voices`（器件里**早就存在**的 getter，现位于
    ///    `crates/yeban-dsp/src/polysynth.rs` 第 868 行）。它在 `yeban-engine` 里
    ///    此前**只有一条读者**，而且那条读者住在
    ///    `#[cfg(debug_assertions)]` 的 [`Self::debug_active_voices`] 里 —— 那个访问器
    ///    在 `src` 与 `tests` 里**零调用者**（量法：
    ///    `grep -rnw 'debug_active_voices' crates/yeban-engine` 在本次改动前命中
    ///    `src` **1** 行 = 它自己的定义、`tests` **0** 行）⇒ **产物路径上**
    ///    "现在有几个声部在响"根本读不出来；
    /// 2. 它只数 `PolySynth` 的定长声部数组（每个声部一个 `active` 布尔）⇒ 不碰样本、
    ///    不分配、不加锁、不做 I/O。汇总方式是 16 个槽位的 `fold`，与
    ///    [`Self::poly_notes_triggered`] / [`Self::voice_steals`] 同一条路径；
    /// 3. **逐轨口径不可用**：本读数是**全池合计**（与 [`Self::voice_steals`] 的跨槽
    ///    口径相同）。要"某一轨有几个声部在响"请用
    ///    [`Self::debug_active_voices`]（仅 debug 构建），产物路径没有它，
    ///    这正是产物读数取合计口径的理由。
    ///
    /// 为什么需要它：[`Self::voice_steals`] 只在**声部池溢出**时推进 ⇒ 它证明
    /// "池**曾经**满过"，证明不了"池**现在**有多满"。一个正在溢出的工程与一个刚好
    /// 差一个声部就溢出的工程在这两个读数上完全一样，只有本读数能把它们分开。
    /// 全部轨都不是复音合成器时（默认）**恒为 0**。
    #[must_use]
    pub fn poly_active_voices(&self) -> u64 {
        self.slots.iter().fold(0u64, |total, slot| {
            total.saturating_add(slot.synth.active_voices() as u64)
        })
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

    /// **鼓机此刻占用着的槽位数**（各槽之和；单位：槽位；0 = 一件鼓都不在响）。
    ///
    /// 它与 [`Self::drum_sounding_slot_frames`] 是**两个不同的质量维度**，所以两个都
    /// 要留着：那个是**累计**的"槽位×帧"覆盖度读数（只增不减，回答"器件算过东西吗"），
    /// 本读数是**瞬时**的占用数（可升可降，回答"现在同时有几件鼓在响"）。
    ///
    /// 口径（三条，都要与判据一起读）：
    ///
    /// 1. 读数取 `DrumMachine::active_slots`（器件里**早就存在**的 getter，现位于
    ///    `crates/yeban-dsp/src/drums/mod.rs` 第 956 行）。它在 `yeban-engine` 的
    ///    `src` 与 `tests` 里**一个读者都没有**（量法：
    ///    `grep -rnw 'active_slots' crates/yeban-engine` 在本次改动前命中 `src`
    ///    **0** 行、`tests` **0** 行）—— 连 debug 访问器都没有，与复音那一侧不同；
    /// 2. 它只数 `DrumMachine` 的定长槽位数组（每个槽位一个 `active` 布尔）
    ///    ⇒ 不碰样本、不分配、不加锁、不做 I/O。汇总方式是 16 个槽位的 `fold`，
    ///    与 [`Self::drum_triggers`] / [`Self::drum_voice_steals`] 同一条路径；
    /// 3. 器件计数器**只增不减**，槽位数组却是**活的**：`DrumMachine::reset` 清空
    ///    全部槽位 ⇒ 本读数会当场回落。这与 [`Self::drum_voice_steals`] 的
    ///    "生命周期累计量"口径**刻意不同**，判据必须钉住这条差别。
    ///
    /// 为什么需要它：与复音那一侧同一个理由 —— [`Self::drum_voice_steals`] 只在
    /// **槽位池满**时才推进 ⇒ 它证明"池曾经满过"，证明不了"池现在有多满"。
    /// 鼓机与复音是**两个独立的池**（该口径见 [`Self::drum_voice_steals`] 的文档），
    /// 因此这条瞬时读数也要两池各一份，不能相加成一个数。
    /// 全部轨都不是鼓机时（默认）**恒为 0**。
    #[must_use]
    pub fn drum_active_slots(&self) -> u64 {
        self.drums.iter().fold(0u64, |total, machine| {
            total.saturating_add(machine.active_slots() as u64)
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

    /// ⭐ **实时踏板的读数**（T-FIX-274）：`(踏板是否踩下, 被踏板挂起的键数)`。
    ///
    /// `None` = 该轨没有占槽或不在本快照里（与 [`Self::tone_params`] 同口径）。
    /// 判据用它钉"踏板抬起后挂起表清空 / `seek` 后清空"，⛔ 不必从音频反推。
    #[must_use]
    pub fn debug_pedal_state(&self, track: EntityId) -> Option<(bool, usize)> {
        self.slot_index(track).map(|index| {
            (
                self.slots[index].pedal_down,
                self.slots[index]
                    .pedal_pending
                    .iter()
                    .filter(|pending| **pending)
                    .count(),
            )
        })
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

    /// ⭐ **只读取值器**（⛔ 不改行为、⛔ 不加字段、⛔ 不进逐样本路径）：某个
    /// `(track, pitch)` 当前活跃声部**真正落地**的逐样本增益
    /// （＝ `PolyVoice.gain`，经 [`yeban_dsp::polysynth::PolySynth::debug_gain_for_freq`]）。
    ///
    /// 为什么需要它：两条触发路径 —— 环音符（[`Self::note_on`]：
    /// `velocity_gain(velocity) × slot.gain_scale`）与已调度音符（快照投影
    /// `snapshot.rs`：`velocity_gain(velocity) × track_gain_scale(...)` → 游标 →
    /// [`Self::render_track`]）—— 的**输入公式**由构造保证同源，但"同源"只是
    /// **源码形状**；判据要能读**器件里那一份**并逐位比较（`tests/two_path_gain.rs`）。
    ///
    /// ⚠ 与 [`Self::debug_voice_state`] 不同，本方法**不做** `debug_assertions` 门控：
    /// 它只读一个 `f32`、不分配（与 `PolySynth::debug_voice` 同一条纪律），
    /// 因此 release 档判据也能编译。
    #[must_use]
    pub fn debug_gain_for(&self, track: EntityId, pitch: u8) -> Option<f32> {
        let index = self.slot_index(track)?;
        let freq_hz = yeban_dsp::math::note_to_hz(f32::from(pitch));
        self.slots[index].synth.debug_gain_for_freq(freq_hz)
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
    ///
    /// ⚠ **SFZ 声部池同办**（[`SfzVoicePool::silence`]）：定位后⛔ 不留上一段采样。
    /// 累计读数（`dropped`）保留，与 `DrumMachine::reset` 保留 `drum_hits` 同口径。
    pub fn seek(&mut self, position: u64) {
        self.position = position;
        for slot in &mut self.slots {
            slot.synth.reset();
            slot.sfz.silence();
            // ⭐ 定位 = 全部声部放掉 ⇒ 踏板的挂起表也没有意义了（T-FIX-274）：
            // 不清就会在**下一次**踏板抬起时放掉一批早已被 `reset` 的键位（无害但
            // 会让"挂起表已清空"的读数说谎），并且让 `pedal_down` 与真实演奏脱节。
            slot.pedal_down = false;
            slot.pedal_pending = [false; 128];
            slot.live_position = position;
            slot.live_sfz = false;
            slot.live_drum = false;
        }
        for machine in &mut self.drums {
            machine.reset();
        }
    }

    fn has_live_source_at(&self, index: usize) -> bool {
        (self.slots[index].live_sfz && self.slots[index].sfz.has_active_voices())
            || (self.slots[index].live_drum
                && self.drum_slots[index].is_some()
                && self.drums[index].active_slots() > 0)
    }

    /// 判断某轨是否存在由实时 `EngineEvent::NoteOn` 触发的活跃试听声部
    /// （复音 `end_sample == u64::MAX` / 实时 SFZ / 实时鼓机击音）。时间轴调度尾巴不受此方法影响。
    /// 零分配、零锁、零 I/O。
    #[must_use]
    pub fn has_active_live_voices(&self, track: EntityId) -> bool {
        let Some(index) = self.slot_index(track) else {
            return false;
        };
        has_live_poly_voice(&self.slots[index].synth) || self.has_live_source_at(index)
    }

    /// 走带停止状态下渲染某轨的实时 `EngineEvent::NoteOn` 试听声部（兼容旧签名：不含 SFZ 计划与 CC）。
    pub fn render_live_voices(&mut self, track: EntityId, out: &mut [f32]) {
        self.render_live_voices_with_sources(track, None, out, None);
    }

    /// 走带停止状态下渲染某轨的实时 `EngineEvent::NoteOn` 试听声部（支持鼓机 / SFZ / 复音）：
    /// 不推进时间轴游标、不触发时间轴调度音符，仅推进本槽 `live_position` 以保证包络与释放衰减完整。
    /// 零分配、零锁、零 I/O。
    pub fn render_live_voices_with_sources(
        &mut self,
        track: EntityId,
        sfz: Option<&PreparedSfz>,
        out: &mut [f32],
        cc: Option<&dyn Fn(u8) -> u8>,
    ) {
        out.fill(0.0);
        let Some(index) = self.slot_index(track) else {
            return;
        };
        if !self.slots[index].present {
            return;
        }
        let position = self.position;
        let has_poly = has_live_poly_voice(&self.slots[index].synth);
        let is_drum = self.slots[index].live_drum && self.drum_slots[index].is_some();
        let is_sfz = self.slots[index].live_sfz && self.slots[index].sfz.has_active_voices();

        if is_drum {
            let live_pos = self.slots[index].live_position;
            self.drums[index].render(live_pos, out);
            self.slots[index].live_position = live_pos.saturating_add(out.len() as u64);
            if self.drums[index].active_slots() == 0 {
                self.slots[index].live_drum = false;
            }
            if has_poly {
                mix_live_poly_voices(&mut self.slots[index].synth, &self.tables, position, out);
            }
            return;
        }
        if is_sfz {
            if let Some(plan) = sfz {
                let live_pos = self.slots[index].live_position;
                self.slots[index]
                    .sfz
                    .render_into_with_probes(plan, live_pos, out, cc, None);
                self.slots[index].live_position = live_pos.saturating_add(out.len() as u64);
                if !self.slots[index].sfz.has_active_voices() {
                    self.slots[index].live_sfz = false;
                }
            } else {
                self.slots[index].sfz.silence();
                self.slots[index].live_sfz = false;
            }
            if has_poly {
                mix_live_poly_voices(&mut self.slots[index].synth, &self.tables, position, out);
            }
            return;
        }
        if has_poly {
            self.slots[index].synth.render(&self.tables, position, out);
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

        // 先把本快照的每轨音色收进**构造期就分配好的**暂存（⚠ 不能是栈数组：
        // 容量现在是运行期值，1024 项放在栈上既爆栈也不可移植 [MUST-GATE-001]）。
        let capacity = self.slots.len();
        let mut wanted_len = 0usize;
        for (id, tone) in tones {
            if wanted_len >= capacity {
                break;
            }
            self.wanted_tones[wanted_len] = (*id, *tone);
            wanted_len += 1;
        }

        // 同理收鼓机投影（**复用的堆暂存，零分配**）：`DrumsParams` 是 `Copy`
        // 值类型（`DrumKitParams` 128 字节 + 五个 `u8`）。
        // 用 `Option` 而不是哨兵值：本 crate 的槽位身份是 `EntityId`，而"未占用"
        // 槽位的 id 就是 nil —— 哨兵会让"未占用槽位"误命中。
        let mut wanted_drums_len = 0usize;
        for (id, params) in drums {
            if wanted_drums_len >= capacity {
                break;
            }
            self.wanted_drums[wanted_drums_len] = Some((*id, *params));
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
                    self.slots[index].live_position = self.position;
                }
                None => self.track_drops = self.track_drops.saturating_add(1),
            }
        }

        // ⭐ **快照切换**：本快照里**不存在**的轨道不再有踏板语义（T-FIX-274）。
        //
        // 为什么必须在这里收尾：踏板抬起是**外部事件**，轨道被移除/换了音源之后它
        // 可能永远不来了；留在挂起表里的键会一直占着声部（用户听到的是"有个音卡住"）。
        // ⚠ 只放掉**被踏板挂起的**那些键，仍被按住的键照旧保留 —— 与"声部状态跨快照
        // 连续"同口径（`begin_snapshot` 不释放声部池）。
        //
        // ⭐ **与踏板抬起同一条口径：每个 pitch 放掉它的全部声部**（T-FIX-406）。这条离场
        // 排水是"抬起永远不会来"时的兜底 ⇒ 它必须与抬起分支做**同样多**的事，否则被移除
        // 的轨道上照样会留一个永不自释的同频声部。
        for slot in &mut self.slots {
            if slot.present || !slot.assigned {
                continue;
            }
            slot.pedal_down = false;
            for pitch in 0..128u8 {
                if !slot.pedal_pending[usize::from(pitch)] {
                    continue;
                }
                slot.pedal_pending[usize::from(pitch)] = false;
                slot.synth
                    .release_all(yeban_dsp::math::note_to_hz(f32::from(pitch)));
            }
        }

        // 器件参数的写入（**构造期语义**，音频线程上运行 ⇒ 必须零分配）：
        // `set_params` 只算滤波器系数（`tan`）与包络系数（`exp`），不碰堆；
        // 波表库在 `new()` 里就建好了，这里一次也不重建。
        let tables = &self.tables;
        let steal_fade_frames = self.steal_fade_frames;
        for slot in &mut self.slots {
            let tone = self.wanted_tones[..wanted_len]
                .iter()
                .find(|(id, _)| *id == slot.id)
                .map(|(_, tone)| *tone)
                .unwrap_or_else(ToneParams::bypass);
            if sample_rate_changed {
                slot.synth.set_sample_rate(sample_rate_u32);
                slot.synth.set_steal_fade_frames(steal_fade_frames);
                // ⭐ SFZ 池同一把 3 ms 舵：帧数按新采样率重算，再把引擎上那一份
                // （可能被判据覆盖成 0 = 硬窃取）原样下推 ⇒ 两条音源**同一条口径**。
                slot.sfz.set_sample_rate(sample_rate);
                slot.sfz.set_steal_fade_frames(steal_fade_frames);
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
            wanted_drums,
            ..
        } = self;
        let wanted_drums = &wanted_drums[..wanted_drums_len];
        // ⚠ 槽位数是**运行期**值（`with_track_slots` 决定）⇒ 循环边界取数组长度，
        // ⛔ 不再是编译期常量。三张表长度由构造期同一次 `capacity` 保证一致。
        for index in 0..slots.len() {
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
                        slots[index].live_drum = false;
                    } else if sample_rate_changed {
                        drums[index].set_sample_rate(sample_rate_u32);
                    }
                    slots[index].sfz.silence();
                    slots[index].live_sfz = false;
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
                        slots[index].live_drum = false;
                    }
                    // 本轨暂时不在快照里 ⇒ 保留状态与武装记录。
                    Some(_) if present_id.is_none() => {}
                    // 另一条轨接管本槽 ⇒ 丢掉上一件音源的尾巴。
                    Some(_) => {
                        drums[index].reset();
                        drum_slots[index] = None;
                        slots[index].live_drum = false;
                    }
                    None => drum_slots[index] = None,
                },
            }
        }
    }

    /// ⭐ **lane 组武装**（逐件处理，期 5）：快照边界把 `(轨, 组首键)` 绑定到
    /// 组槽，并把**该轨的**音色/鼓机参数下推到组槽的器件。
    ///
    /// 纪律与 [`Self::begin_snapshot`] **逐条同款**（它在音频线程的快照边界上
    /// 运行 ⇒ 零分配、只写定长表）：
    ///
    /// 1. 已绑定的组**留在原槽**（声部/包络状态连续）；新组占空槽；本快照里
    ///    不再出现的组**不**解绑、不 reset（等它回来，与轨的 `present = false`
    ///    同口径）—— 组槽的 `present` 只表示「本快照里有这个组」；
    /// 2. 同组同参数 ⇒ 一个字段都不写；参数变了 ⇒ 只 `set_params`；
    ///    **换组接管**（槽被另一个 `(轨, 首键)` 占用）⇒ `reset` 丢尾巴；
    /// 3. 槽位装不下的组计数进 [`Self::group_drops`]（⛔ 不静默）；rt 侧据
    ///    [`Self::lane_groups_armed_for`] 判定整轨是否 lane 化（半组不渲染）。
    ///
    /// `tones` / `drums` 是**轨级**投影（组用所属轨的那一份）；迭代器只读一遍，
    /// 落进与 `begin_snapshot` 共用的 `wanted_*` 暂存（调用序上两者在同一边界
    /// 先后运行，暂存复用不冲突）。
    pub fn arm_lane_groups<'a, G, T, D>(&mut self, sample_rate: u32, groups: G, tones: T, drums: D)
    where
        G: IntoIterator<Item = (EntityId, u8)>,
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

        // 轨级音色/鼓机投影落暂存（与 begin_snapshot 同一条复用纪律）。
        let capacity = self.slots.len();
        let mut wanted_len = 0usize;
        for (id, tone) in tones {
            if wanted_len >= capacity {
                break;
            }
            self.wanted_tones[wanted_len] = (*id, *tone);
            wanted_len += 1;
        }
        let mut wanted_drums_len = 0usize;
        for (id, params) in drums {
            if wanted_drums_len >= capacity {
                break;
            }
            self.wanted_drums[wanted_drums_len] = Some((*id, *params));
            wanted_drums_len += 1;
        }

        // 绑定：先全标 not-present，再按给定序认领/新占槽位。
        for slot in &mut self.group_slots {
            slot.present = false;
        }
        for (track, first_key) in groups {
            if let Some(index) = self
                .group_keys
                .iter()
                .position(|binding| *binding == Some((track, first_key)))
            {
                self.group_slots[index].present = true;
                continue;
            }
            match self.group_keys.iter().position(|binding| binding.is_none()) {
                Some(index) => {
                    self.group_slots[index] = TrackSlot::empty(sample_rate_u32);
                    self.group_slots[index].assigned = true;
                    self.group_slots[index].present = true;
                    self.group_slots[index].id = track;
                    self.group_slots[index].live_position = self.position;
                    self.group_keys[index] = Some((track, first_key));
                    // 换组接管：鼓机/SFZ 的旧尾巴必须丢（与轨的换轨 reset 同口径）。
                    self.group_drums[index].reset();
                    self.group_drums[index].set_sample_rate(sample_rate_u32);
                    self.group_drum_slots[index] = None;
                }
                None => self.group_drops = self.group_drops.saturating_add(1),
            }
        }

        // 器件参数下推：音色（逐组查所属轨的那一份）。
        let tables = &self.tables;
        let steal_fade_frames = self.steal_fade_frames;
        for (index, slot) in self.group_slots.iter_mut().enumerate() {
            if self.group_keys[index].is_none() {
                continue;
            }
            let tone = self.wanted_tones[..wanted_len]
                .iter()
                .find(|(id, _)| *id == slot.id)
                .map(|(_, tone)| *tone)
                .unwrap_or_else(ToneParams::bypass);
            if sample_rate_changed {
                slot.synth.set_sample_rate(sample_rate_u32);
                slot.synth.set_steal_fade_frames(steal_fade_frames);
                slot.sfz.set_sample_rate(sample_rate);
                slot.sfz.set_steal_fade_frames(steal_fade_frames);
            }
            if sample_rate_changed || tone != slot.tone {
                slot.tone = tone;
                slot.synth.set_params(tone.poly_synth_params(), tables);
            }
        }

        // 鼓机下推：三口径与 begin_snapshot 的鼓机段同款（同轨同参不写、
        // 变参只 set_params、非鼓机了就 reset 本槽那一份）。
        let Self {
            group_slots,
            group_drums,
            group_drum_slots,
            group_keys,
            wanted_drums,
            ..
        } = self;
        let wanted_drums = &wanted_drums[..wanted_drums_len];
        for index in 0..group_slots.len() {
            let Some((track, _)) = group_keys[index] else {
                continue;
            };
            let present = group_slots[index].present;
            let desired = if present {
                wanted_drums
                    .iter()
                    .flatten()
                    .find(|(want, _)| *want == track)
                    .map(|(_, params)| (track, *params))
            } else {
                None
            };
            let armed = group_drum_slots[index];
            match desired {
                Some((track, params)) => {
                    let same_track = armed.map(|(id, _)| id) == Some(track);
                    if !same_track {
                        group_drums[index].reset();
                        group_drums[index].set_sample_rate(sample_rate_u32);
                        group_slots[index].live_drum = false;
                    } else if sample_rate_changed {
                        group_drums[index].set_sample_rate(sample_rate_u32);
                    }
                    group_slots[index].sfz.silence();
                    group_slots[index].live_sfz = false;
                    if !same_track || armed.map(|(_, prev)| prev) != Some(params) {
                        group_drums[index].set_params(params.kit());
                    }
                    group_drum_slots[index] = Some((track, params));
                }
                None => match armed {
                    // 本组仍在快照里、但所属轨不再是鼓机 ⇒ 丢尾巴。
                    Some((id, _)) if present && id == track => {
                        group_drums[index].reset();
                        group_drum_slots[index] = None;
                        group_slots[index].live_drum = false;
                    }
                    // 本组暂时不在快照里 ⇒ 保留状态与武装记录。
                    Some(_) if !present => {}
                    Some(_) => {
                        group_drums[index].reset();
                        group_drum_slots[index] = None;
                        group_slots[index].live_drum = false;
                    }
                    None => group_drum_slots[index] = None,
                },
            }
        }
    }

    /// ⭐ 某条轨有几个组**在本快照里**武装好了（期 5；rt 整轨 lane 化的判据）。
    #[must_use]
    pub fn lane_groups_armed_for(&self, track: &EntityId) -> usize {
        self.group_keys
            .iter()
            .zip(self.group_slots.iter())
            .filter(|(binding, slot)| slot.present && binding.is_some_and(|(id, _)| id == *track))
            .count()
    }

    /// ⭐ 某个组是否已绑定且在本快照里在场（期 5）。
    #[must_use]
    pub fn lane_group_armed(&self, track: &EntityId, first_key: u8) -> bool {
        self.group_keys
            .iter()
            .zip(self.group_slots.iter())
            .any(|(binding, slot)| slot.present && *binding == Some((*track, first_key)))
    }

    /// ⭐ 累计因组槽容量不足而没装上的组数（期 5；⛔ 不静默的读数）。
    #[must_use]
    pub const fn group_drops(&self) -> u64 {
        self.group_drops
    }

    /// ⭐ 组游标校正（期 5）：与 [`Self::align_cursors`] 同纪律，只作用于组槽。
    pub fn align_group_cursors<'a, I>(&mut self, schedules: I)
    where
        I: IntoIterator<Item = ((EntityId, u8), &'a NoteSchedule)>,
    {
        let position = self.position;
        for ((track, first_key), schedule) in schedules {
            let Some(index) = self
                .group_keys
                .iter()
                .position(|binding| *binding == Some((track, first_key)))
            else {
                continue;
            };
            let slot = &mut self.group_slots[index];
            if !slot.present {
                continue;
            }
            slot.cursor = slot.cursor.max(schedule.first_sounding_index(position));
            slot.synth.retire_finished(position);
        }
    }

    /// ⭐ 组游标重定位（期 5）：与 [`Self::realign_cursors`] 同纪律（定位专用）。
    pub fn realign_group_cursors<'a, I>(&mut self, schedules: I)
    where
        I: IntoIterator<Item = ((EntityId, u8), &'a NoteSchedule)>,
    {
        let position = self.position;
        for ((track, first_key), schedule) in schedules {
            let Some(index) = self
                .group_keys
                .iter()
                .position(|binding| *binding == Some((track, first_key)))
            else {
                continue;
            };
            let slot = &mut self.group_slots[index];
            if !slot.present {
                continue;
            }
            slot.cursor = schedule.first_sounding_index(position);
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

    /// ⭐ **定位之后把游标搬回新位置**（任务单 `#231`）。
    ///
    /// 与 [`Self::align_cursors`] 的唯一差别：这里的赋值**不是** `max`。快照边界上
    /// 位置只增 ⇒ `max` 是对的（"已经触发过的音符绝不重复触发"）；定位
    /// （`TransportCommand::SeekTicks`）不同 —— 它允许**向后**跳（回起点），
    /// 而游标只增不减会把"新位置之后仍应发声"的音符**永久消费掉**：
    /// 回 0 之后触发循环从旧游标往后看，一个音符都不再起音 ⇒ 再按播放**没声音**。
    ///
    /// 实测（`demo_project`、48 kHz、120 BPM、`#231` 的复现矩阵）：初次播放峰值
    /// `0.324276` / 不定位继续播 `0.249923`；`Stop + SeekTicks(0)` 之后同样渲染
    /// ⇒ 非零样本 **0** / 峰值 **0.000000**。
    ///
    /// 调用契约：调用方必须先 [`Self::seek`] —— 它释放全部声部并把播放头写到新位置。
    /// 本函数只搬游标，⛔ 不碰声部、⛔ 不分配、⛔ 不触发：触发仍由下一个量子的
    /// 正常窗口判定（"本量子窗口内起音"）完成 ⇒ "定位到音符之后不得补触发"
    /// 这条既有语义**一位不变**（`tests/transport_contract.rs` 判据 ④ 仍成立）。
    pub fn realign_cursors<'a, I>(&mut self, schedules: I)
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
            slot.cursor = schedule.first_sounding_index(position);
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
    ///
    /// # ⭐ 第三条音源支路：SFZ（任务 `#66` 的步 2）
    ///
    /// `sfz` 是**快照里那一份**摊平表（[`crate::sfz_instrument::SfzPlan::prepared`]）的
    /// **借用**：⛔ 不 clone 它、⛔ 不 `Arc` clone 里面的 `Vec`（PCM **借用快照**）。
    /// 与既有 `schedule: Option<&NoteSchedule>` **对称**（同样是"快照里的只读投影"）。
    ///
    /// **分派次序 ＝ 鼓机 ＞ SFZ ＞ 复音**（一条轨至多一件音源，[`crate::drums`] §4.1）：
    ///
    /// | 鼓机键位映射 | `sfz` | 走哪条路 |
    /// | :---: | :---: | :--- |
    /// | `Some` | 任意 | 鼓机（SFZ 池**被释放**；"鼓机＋SFZ 同轨"的裁决与信号见下） |
    /// | `None` | `Some` | SFZ 声部池（⛔ **不叠加**复音） |
    /// | `None` | `None` | 复音合成器（SFZ 池**被释放**） |
    ///
    /// ⭐ **`sfz-needs-3` 的裁决与落地（本单）**：次序**维持** `鼓机 ＞ SFZ ＞ 复音`
    /// —— 它与 [`crate::drums`] §4.1"一条轨至多一件音源"的既有裁决一致
    /// ⇒ ⛔ **不改成叠加**。真正缺的是"用户看得出被忽略了"：本函数把
    /// **武装的音源件数**写进 [`SynthEngine::armed_sources`] 这张**投影读数**
    /// （复音恒在场 ⇒ 三件同轨 = `3`）⇒ [`SynthEngine::ignored_sources`] = `2`。
    /// ⛔ 这行只写一个 `u8`：不改输出（S7 逐位钉住）、不分配、不进逐样本路径。
    ///
    /// SFZ 分支的终点口径与分配纪律见 [`crate::sfz_instrument`] 模块头
    /// （`min(end_sample, span 终点, 样本播完)`；⛔ 跳过 `ampeg_*`、用样本自然结束）。
    ///
    /// # ⭐ 轨增益（**三条支路同径**；归属裁决见台账 `R711`）
    ///
    /// 三条支路都在**本函数**里拿到"引擎层的轨增益"：鼓机与复音读的是调度表上**逐音符**的
    /// `ScheduledNote::gain`（投影期已乘进 `track_gain_scale(...)`），SFZ 读的是**同一张表**上的
    /// [`NoteSchedule::track_gain`]（同一次投影、**同一个**算子）⇒ ⭐ 推子/静音/独奏对三者
    /// **同径生效**；而 SFZ 的**力度**那一半由乐器层拥有 ⇒ ⛔ 不重复计。
    ///
    /// ⭐ 任务单 `#107`：本方法是 [`Self::render_track_with_cc`] 传 `cc = None` 的**薄包装**
    /// （与 [`crate::sfz_instrument`] 的 `*_with_cc` 一族同一形态）⇒ 所有既有调用点
    /// （含离线导出）**一个字都不用改**，读数与接线前逐位相同。
    pub fn render_track(
        &mut self,
        track: EntityId,
        schedule: Option<&NoteSchedule>,
        sfz: Option<&PreparedSfz>,
        out: &mut [f32],
    ) {
        self.render_track_with_cc(track, schedule, sfz, out, None);
    }

    /// ⭐ **带 MIDI CC 探针的渲染入口**（任务单 `#107`）：语义、分派次序与分配纪律
    /// **逐字**沿用 [`Self::render_track`]（见那份文档 —— 本函数只多**一个**参数）。
    ///
    /// * `cc == None` ⇒ 与 [`Self::render_track`] **逐位相同**（该包装就传这个值）；
    /// * `cc == Some(probe)` ⇒ SFZ 那条路把它交给
    ///   [`crate::sfz_instrument::SfzVoicePool::render_into_with_cc`]
    ///   ⇒ 逐块的 `ampeg_*` 求值按 `Fn(u8) -> u8` 读 CC（`ampeg_*ccN` 的 CC 支生效）；
    ///   鼓机与复音两条支路**不读** CC（它们没有 CC 面）⇒ 那两条输出逐位不变。
    ///
    /// ⚠ **不改区域选择**：CC 门控（`loccN`/`hiccN`）在**摊平期**就已烧进掩码
    /// （登记见 `crate::sfz_instrument` 模块头的"摊平期门控登记"）⇒ 探针只影响
    /// **取值**。⛔ 这不是静默：那条登记在案。
    ///
    /// **零分配、零锁、零 I/O**：探针是**只读借用**（`&dyn Fn`），逐块只多几次整数读。
    pub fn render_track_with_cc(
        &mut self,
        track: EntityId,
        schedule: Option<&NoteSchedule>,
        sfz: Option<&PreparedSfz>,
        out: &mut [f32],
        cc: Option<&dyn Fn(u8) -> u8>,
    ) {
        self.render_track_inner(track, schedule, sfz, out, None, cc);
    }

    /// ⭐ **分声道渲染（区域 `pan` 的落点）**：左/右两条总线各写一遍。
    ///
    /// * SFZ 支路 ⇒ [`crate::sfz_instrument::SfzVoicePool::render_into_stereo_with_probes`]
    ///   （声相在**源侧**按区域施加，与 Logic 的 sampler 同口径）；
    /// * 鼓机 / 复音支路⛔ 没有源侧声相 ⇒ 先写左声道，再整体复制给右
    ///   （与单声道路径**逐位相同**的"两声道同值"）。
    ///
    /// ⚠ 与 [`Self::render_track_with_cc`] 的差别**仅**是"多写一条右声道"：
    /// 触发、音源、增益、块切分**逐字同一份**实现（同一个内层函数）。
    pub fn render_track_stereo(
        &mut self,
        track: EntityId,
        schedule: Option<&NoteSchedule>,
        sfz: Option<&PreparedSfz>,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        self.render_track_stereo_with_cc(track, schedule, sfz, left, right, None);
    }

    /// [`Self::render_track_stereo`] ＋ CC 探针（语义同 [`Self::render_track_with_cc`]）。
    pub fn render_track_stereo_with_cc(
        &mut self,
        track: EntityId,
        schedule: Option<&NoteSchedule>,
        sfz: Option<&PreparedSfz>,
        left: &mut [f32],
        right: &mut [f32],
        cc: Option<&dyn Fn(u8) -> u8>,
    ) {
        right.fill(0.0);
        self.render_track_inner(track, schedule, sfz, left, Some(right), cc);
    }

    /// ⭐ **lane 组的带 CC 渲染入口**（逐件处理，期 5）：与
    /// [`Self::render_track_with_cc`] 同语义，只是槽位换成 `(轨, 组首键)`
    /// 绑定的**组槽**、调度表换成该组的分表。组没武装 ⇒ 输出静音（`out`
    /// 已清零），⛔ 不 panic。零分配、零锁、零 I/O（同轨渲染）。
    pub fn render_group_with_cc(
        &mut self,
        track: EntityId,
        first_key: u8,
        schedule: Option<&NoteSchedule>,
        sfz: Option<&PreparedSfz>,
        out: &mut [f32],
        cc: Option<&dyn Fn(u8) -> u8>,
    ) {
        self.render_bank_inner(
            SlotBank::Group,
            track,
            first_key,
            schedule,
            sfz,
            out,
            None,
            cc,
        );
    }

    /// ⭐ **lane 组的分声道渲染入口**（期 5）：与
    /// [`Self::render_track_stereo_with_cc`] 同形（组渲染的常态是立体声对：
    /// 组链与组声相都按对处理）。
    #[allow(clippy::too_many_arguments)]
    pub fn render_group_stereo_with_cc(
        &mut self,
        track: EntityId,
        first_key: u8,
        schedule: Option<&NoteSchedule>,
        sfz: Option<&PreparedSfz>,
        left: &mut [f32],
        right: &mut [f32],
        cc: Option<&dyn Fn(u8) -> u8>,
    ) {
        right.fill(0.0);
        self.render_bank_inner(
            SlotBank::Group,
            track,
            first_key,
            schedule,
            sfz,
            left,
            Some(right),
            cc,
        );
    }

    /// 单声道/分声道**共用的唯一一份**轨渲染体（`out_r == None` ⇒ 逐位同旧）。
    #[allow(clippy::too_many_arguments)]
    fn render_track_inner(
        &mut self,
        track: EntityId,
        schedule: Option<&NoteSchedule>,
        sfz: Option<&PreparedSfz>,
        out: &mut [f32],
        out_r: Option<&mut [f32]>,
        cc: Option<&dyn Fn(u8) -> u8>,
    ) {
        self.render_bank_inner(SlotBank::Track, track, 0, schedule, sfz, out, out_r, cc);
    }

    /// ⭐ 轨槽 / 组槽**共用的唯一一份**渲染体（期 5）：`bank` 只决定查哪张
    /// 槽表、写哪张武装读数，其余（触发、分派次序、三条音源支路、增益口径）
    /// **逐字同一份** ⇒ 组渲染与轨渲染不可能漂移出两套语义。
    #[allow(clippy::too_many_arguments)]
    fn render_bank_inner(
        &mut self,
        bank: SlotBank,
        track: EntityId,
        first_key: u8,
        schedule: Option<&NoteSchedule>,
        sfz: Option<&PreparedSfz>,
        out: &mut [f32],
        mut out_r: Option<&mut [f32]>,
        cc: Option<&dyn Fn(u8) -> u8>,
    ) {
        out.fill(0.0);
        let index = match bank {
            SlotBank::Track => self
                .slots
                .iter()
                .position(|slot| slot.assigned && slot.id == track),
            SlotBank::Group => self
                .group_keys
                .iter()
                .position(|binding| *binding == Some((track, first_key))),
        };
        let Some(index) = index else {
            return;
        };
        let present = match bank {
            SlotBank::Track => self.slots[index].present,
            SlotBank::Group => self.group_slots[index].present,
        };
        if !present {
            return;
        }

        let position = self.position;
        let end = position.saturating_add(out.len() as u64);
        let tables = &self.tables;
        let notes_triggered = &mut self.notes_triggered;
        let drum_hits = &mut self.drum_hits;
        // 本槽的音源：`Some(map)` = 鼓机（键位映射）。
        let drum_map: Option<DrumNoteMap> = match bank {
            SlotBank::Track => self.drum_slots[index].map(|(_, params)| params.notes()),
            SlotBank::Group => self.group_drum_slots[index].map(|(_, params)| params.notes()),
        };
        let (slot, drum_machine, armed_count) = match bank {
            SlotBank::Track => (
                &mut self.slots[index],
                &mut self.drums[index],
                &mut self.armed_sources[index],
            ),
            SlotBank::Group => (
                &mut self.group_slots[index],
                &mut self.group_drums[index],
                &mut self.group_armed_sources[index],
            ),
        };
        // ⭐ `sfz-needs-3` 的可读信号：把"本槽武装了几件音源"写成**投影读数**。
        // 复音合成器恒在场（每个槽一台）⇒ 至少 1；鼓机 +1、SFZ +1。
        // 三件同轨 = 3（被忽略 2 件，读 `ignored_sources`）；⛔ 本行不改任何输出。
        *armed_count = 1u8
            .saturating_add(u8::from(drum_map.is_some()))
            .saturating_add(u8::from(sfz.is_some()));

        // ⭐ `ADR-0077` D5：SFZ 效果总线的**热**（控制侧 API 之外的兜底）。
        // 有 `<effect>` 的乐器第一次进渲染时把 `BusBank`（≤4 台混响 ＋ 8 条缓冲）
        // 建起来；这是**一次性**分配，之后 `frames`/器件表不变就严格空操作。
        // ⛔ 只在轨槽上做：组槽与 SFZ 同一套池，但组没有 SFZ 音源。
        if let (SlotBank::Track, Some(plan)) = (bank, sfz) {
            slot.sfz.warm_buses(plan, out.len());
        }

        // --- 1) 触发本量子窗口内起音、且尚未结束的音符 ---
        // 游标只增不减 ⇒ 已触发过的音符永远不会被再次触发。
        //
        // "分配声部 / 池满时软窃取 / 3 ms 淡出"全部在器件的 `note_on` 里
        // （[ARCH-RT-004]）；引擎只负责"哪个音符、在哪个量子边界"。
        // 鼓机分支共用同一个游标与同一个窗口判定，只有"音符 → 器件"这一步不同：
        // 键位映射里没有的音高**不触发任何鼓件**（也不改动器件状态）。
        // SFZ 分支同办：为**每条匹配区域**占一个声部（多匹配 ＝ 叠加，与摊平表同口径）。
        if let Some(schedule) = schedule {
            let notes = schedule.notes();
            // ⭐ ⭐ **SFZ 支路：按音符自己的起始帧分段渲染**（任务单 `#222`）。
            //
            // 缺陷（`#221` 实测）：本循环把"本块内起音"的音符**全部在块首触发**，
            // 而 `SfzVoicePool` 从块首开始写 `out` ⇒ 一个起点在块中间的音符会**提前**
            // 最多 `block_frames − 1` 帧发声。两个相邻的短音符（例如 `ratchet=2`
            // 拆出的两个脉冲）于是**重叠叠加**：实测 `block=468 first=59904` 时
            // `starts=[0,48000,60000,95400,144000]` 中 60000 那个脉冲被提前 96 帧，
            // 活声部 = **2**、峰值 = 单声部的**恰好 2 倍**（常量样本 ⇒ 0.219463661 → 0.438927323）。
            //
            // 修法：把本块按"音符起始帧"切成若干段；每段用该段的**绝对位置**渲染，
            // 段边界处才触发下一批音符 ⇒ 每个声部都从自己的帧起振，且**不新增**任何
            // 逐声部字段（`SfzVoice` 仍 40 B、池仍 1712 B）。
            // ⚠ 事件次序仍与旧实现一致（先触发再渲染该段）；块内没有中途起音时
            // 只渲染一段、输出与接线前**逐位相同**。
            if drum_map.is_none()
                && let Some(plan) = sfz
            {
                // ⭐ **弯音探针**（本单）：与下面复音支路**逐字同一份构造** ——
                // 有音高形状才传探针，探针就是调度表旁表的 `cents_at`
                // （⛔ 不另立第二套曲线语义）；空表 ⇒ `None` ⇒ SFZ 池走接线前
                // 的恒定速率路，输出逐位相同。
                let bend_probe = |shape: u16, local: u64| schedule.cents_at(shape, local);
                let pitch_probe: Option<&dyn Fn(u16, u64) -> f32> = if schedule.has_pitch_shapes() {
                    Some(&bend_probe as &dyn Fn(u16, u64) -> f32)
                } else {
                    None
                };
                let mut offset = 0usize;
                let mut cursor_frame = position;
                loop {
                    while let Some(note) = notes.get(slot.cursor) {
                        if note.start_sample >= end {
                            break;
                        }
                        if note.start_sample > cursor_frame {
                            break;
                        }
                        if note.end_sample > cursor_frame
                            && {
                                // ⭐ `#341`：本音符起音那一刻"其它音符的按住数"
                                // （`trigger=first` / `legato` 的运行时门控输入）。
                                slot.sfz.set_held_notes(Some(held_before(
                                    notes,
                                    slot.cursor,
                                    note.start_sample,
                                )));
                                true
                            }
                            && slot.sfz.trigger_note_with_pitch(
                                plan,
                                note.pitch,
                                note.velocity,
                                note.start_sample,
                                note.end_sample,
                                schedule.track_gain(),
                                cc,
                                note.pitch_shape,
                                schedule.tune_cents(),
                            ) > 0
                        {
                            *notes_triggered = notes_triggered.saturating_add(1);
                        }
                        slot.cursor += 1;
                    }
                    let boundary = match notes.get(slot.cursor) {
                        Some(note) if note.start_sample < end => {
                            note.start_sample.max(cursor_frame)
                        }
                        _ => end,
                    };
                    if boundary > cursor_frame {
                        #[allow(clippy::cast_possible_truncation)]
                        let length = (boundary - cursor_frame) as usize;
                        let length = length.min(out.len().saturating_sub(offset));
                        if length == 0 {
                            break;
                        }
                        match out_r.as_mut() {
                            Some(right) => slot.sfz.render_into_stereo_with_probes(
                                plan,
                                cursor_frame,
                                &mut out[offset..offset + length],
                                &mut right[offset..offset + length],
                                cc,
                                pitch_probe,
                            ),
                            None => slot.sfz.render_into_with_probes(
                                plan,
                                cursor_frame,
                                &mut out[offset..offset + length],
                                cc,
                                pitch_probe,
                            ),
                        }
                        offset += length;
                        cursor_frame = boundary;
                    }
                    if cursor_frame >= end || offset >= out.len() {
                        break;
                    }
                }
                mix_live_poly_voices(&mut slot.synth, tables, position, out);
                slot.live_position = slot.live_position.max(end);
                if !slot.sfz.has_active_voices() {
                    slot.live_sfz = false;
                }
                return;
            }
            while let Some(note) = notes.get(slot.cursor) {
                if note.start_sample >= end {
                    break;
                }
                if note.end_sample > position {
                    match drum_map {
                        Some(map) => {
                            if let Some(voice) = map.voice_for(note.pitch) {
                                let mut hit = DrumHit::new(voice, note.start_sample, note.gain);
                                // ⭐ 逐拍音高（T-DRUM-15）：牛铃 / 808 两件
                                // 读本音符在**起点**的弯音＋滑音总音分，
                                // 换成比率交给这一击（基频同比缩放，器件
                                // 侧消费）。弯音是整条曲线、鼓击是一击
                                // 即走 ⇒ 只取起点值，曲线其余部分对鼓件
                                // 无语义。其余鼓件与无形状音符（绝大多
                                // 数）不进本支 ⇒ 触发逐位不变。
                                if matches!(voice, DrumVoice::Cowbell | DrumVoice::Bass808) {
                                    let cents = schedule.cents_at(note.pitch_shape(), 0);
                                    if cents != 0.0 {
                                        hit = hit.with_pitch_ratio(tune_ratio_from_cents(cents));
                                    }
                                }
                                drum_machine.trigger(hit);
                                *drum_hits = drum_hits.saturating_add(1);
                                *notes_triggered = notes_triggered.saturating_add(1);
                            }
                        }
                        // ⭐ SFZ：鼓机缺席时才轮到它（次序 ＝ 鼓机 ＞ SFZ ＞ 复音）。
                        None => match sfz {
                            Some(plan) => {
                                // 覆盖度：真的建出声部才记一次"触发"（与鼓机
                                // "映射里没有的音高不记"同口径）。
                                // ⭐ 轨增益门（引擎层）在这里**与鼓机同一处**交给音源：
                                // 鼓机拿的是含轨门的 `note.gain`，SFZ 拿的是**同一张调度表**
                                // 上的 `track_gain`（力度那一半由乐器层自己算 ⇒ ⛔ 不重复）。
                                // ⭐ 任务单 `#109`：**同一个** CC 探针也进触发期 ——
                                // `loccN`/`hiccN` 是**区域选择**判据（规范逐字见
                                // `SfzVoicePool::trigger_note` 的文档）⇒ 门控不成立的
                                // 区域在这里就**不建声部**（⛔ 不是逐块调制）。
                                slot.sfz.set_held_notes(Some(held_before(
                                    notes,
                                    slot.cursor,
                                    note.start_sample,
                                )));
                                if slot.sfz.trigger_note_with_pitch(
                                    plan,
                                    note.pitch,
                                    note.velocity,
                                    note.start_sample,
                                    note.end_sample,
                                    schedule.track_gain(),
                                    cc,
                                    note.pitch_shape,
                                    schedule.tune_cents(),
                                ) > 0
                                {
                                    *notes_triggered = notes_triggered.saturating_add(1);
                                }
                            }
                            None => {
                                // ⭐ **时间轴排音刻意直调**（T-FIX-389：理由与边界钉在这里）。
                                //
                                // 与实时通路（`note_on_with_sources`）的差别是**结构性的**，
                                // 不是漏接线：本支的每条音符自带**有限** `end_sample`
                                // （`note.start_sample..note.end_sample`），器件按
                                // `now >= end_sample` 自己关 gate ⇒ 不需要、也不该有
                                // `NoteOff`。同一音高在时间轴上叠两段是**刻意的复音**
                                // （长音垫 + 短音、分轨层的同音加厚），套上实时的
                                // "抢占同一槽 = 重触发"裁定会把叠层里较早那条**吃掉**
                                // ⇒ ⛔ 这里不调 `note_off` / `note_off_live`。
                                //
                                // 边界（反过来那一半由实时侧守）：实时松键/重触发只放
                                // 活声部（`end_sample == u64::MAX`）⇒ 本支建出来的声部
                                // 不会被用户的手指截短；判据见
                                // `tests/live_release_scope.rs` 的四条。
                                slot.synth.note_on(
                                    NoteEvent::new(
                                        note.start_sample,
                                        note.end_sample,
                                        note.freq_hz,
                                        note.gain,
                                    )
                                    // ⭐ `#311b`：形状下标随触发进声部（`NO_PITCH_SHAPE`
                                    // = 无弯音/滑音 ⇒ 与接线前逐位相同）。
                                    .with_pitch_shape(note.pitch_shape),
                                    tables,
                                );
                                *notes_triggered = notes_triggered.saturating_add(1);
                            }
                        },
                    }
                }
                slot.cursor += 1;
            }
        }

        // --- 2) 逐样本合成 ---
        // 鼓机分声道渲染标记：为真时末尾不再把左声道复制给右声道
        // （逐件声像已写进右声道）。有实时试听声部要混时退回旧行为
        // （试听声部只有单声道一份，复制保它两边都在）。
        let mut drum_stereo_written = false;
        match drum_map {
            // 鼓机：器件自己把 `out` 清零再落样本（"无占用槽位 ⇒ 逐位静音"）。
            // 尾音的回收也在 `render` 里（包络走完的槽位被置回空闲）。
            // ⚠ SFZ 池在这里被**释放**：本轨的音源是鼓机 ⇒ ⛔ 不留采样尾巴。
            Some(_) => {
                slot.sfz.silence();
                match out_r.as_deref_mut() {
                    Some(right) if !has_live_poly_voice(&slot.synth) => {
                        drum_machine.render_stereo(position, out, right);
                        drum_stereo_written = true;
                    }
                    _ => drum_machine.render(position, out),
                }
            }
            None => match sfz {
                // ⭐ SFZ：逐**声部**混（线性插值 + 增益乘加），位置跨量子延续。
                // 零分配：只读 `plan` 的 PCM、原地写 `out`。
                // 尾音在源采样播完（或音符终点）时于池内退休。
                // ⭐ 任务单 `#107`：CC 探针沿这条路下行（`None` ⇒ 逐位同接线前）。
                // ⭐ **弯音探针**（本单）：与复音支路**同一份构造**（有音高形状
                // 才传探针 ⇒ 同一份 `cents_at`）；分段渲染路缺席时的兜底支路，
                // 两条路的弯音语义必须一致（⛔ 不许一条生效一条静默）。
                Some(plan) => {
                    // ⚠ 闭包先绑定在本臂作用域里再取 `&dyn`（探针要活过整个调用）。
                    let bend_probe = |shape: u16, local: u64| {
                        schedule.map_or(0.0, |schedule| schedule.cents_at(shape, local))
                    };
                    let pitch_probe: Option<&dyn Fn(u16, u64) -> f32> = match schedule {
                        Some(schedule) if schedule.has_pitch_shapes() => {
                            Some(&bend_probe as &dyn Fn(u16, u64) -> f32)
                        }
                        _ => None,
                    };
                    slot.sfz
                        .render_into_with_probes(plan, position, out, cc, pitch_probe);
                }
                // 复音：整数相位 + ADSR + 可选低通（器件内）。
                None => {
                    slot.sfz.silence();
                    // ⭐ `#311b`：有音高形状才传探针。探针把"形状下标 ＋ 音符局部
                    // 样本"映成音分，**借用**本轨调度表的旁表（⛔ 不 clone、⛔ 不分配）；
                    // 空表 ⇒ 参数为 `None` ⇒ 器件走**今天这条**路径（逐位相同）。
                    match schedule {
                        Some(schedule) if schedule.has_pitch_shapes() => {
                            let probe = |shape: u16, local: u64| schedule.cents_at(shape, local);
                            slot.synth.render_with_pitch(
                                tables,
                                position,
                                out,
                                Some(&probe as &dyn Fn(u16, u64) -> f32),
                            );
                        }
                        _ => slot.synth.render(tables, position, out),
                    }
                }
            },
        }
        if drum_map.is_some() || sfz.is_some() {
            mix_live_poly_voices(&mut slot.synth, tables, position, out);
        }
        slot.live_position = slot.live_position.max(end);
        if !slot.sfz.has_active_voices() {
            slot.live_sfz = false;
        }
        if drum_machine.active_slots() == 0 {
            slot.live_drum = false;
        }
        // ⭐ 鼓机 / 复音的兜底支路没有源侧声相 ⇒ 它们只写了左声道，
        // 这里把左声道整份复制给右（"两声道同值"）；SFZ 分段路已在写的时候就分好了。
        // 例外：鼓机已按逐件声像分声道写过（`drum_stereo_written`）⇒ 不复制。
        if let Some(right) = out_r
            && !drum_stereo_written
        {
            right.copy_from_slice(out);
        }
    }
}

/// 检查某轨复音池内是否存在由实时 `EngineEvent::NoteOn` 触发的活跃试听声部（`end_sample == u64::MAX`）。
#[inline]
fn has_live_poly_voice(synth: &PolySynth) -> bool {
    if synth.active_voices() == 0 {
        return false;
    }
    (0..VOICES_PER_TRACK).any(|idx| matches!(synth.debug_voice(idx), Some((true, u64::MAX, _, _))))
}

/// 当轨道主音源为鼓机或 SFZ 时，若用户通过实时 `NoteOn` 触发了复音试听声部，
/// 使用定长栈缓冲叠加渲染，避免主音源覆盖实时键盘试听声部，且保持零堆分配。
fn mix_live_poly_voices(
    synth: &mut PolySynth,
    tables: &PolySynthTables,
    position: u64,
    out: &mut [f32],
) {
    if !has_live_poly_voice(synth) {
        return;
    }
    let mut live_chunk = [0.0_f32; 256];
    let mut offset = 0usize;
    while offset < out.len() {
        let take = (out.len() - offset).min(live_chunk.len());
        synth.render(
            tables,
            position.saturating_add(offset as u64),
            &mut live_chunk[..take],
        );
        for (dst, src) in out[offset..offset + take]
            .iter_mut()
            .zip(live_chunk[..take].iter())
        {
            *dst += *src;
        }
        offset += take;
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

/// tick → 样本位置（**构造期**，不在实时路径上）。
///
/// 唯一实现已经下移到 [`yeban_model::time_math::tick_to_sample`]（`#247`）：
/// 本文件不再持有"乘 tick + `round` + 饱和"这份算式。这里用 `pub use` 保留
/// 既有导入路径（`crate::synth::tick_to_sample`），并让引擎与 MCP 的
/// `render_math::ticks_to_frames` 落在**同一份** IEEE-754 运算顺序上。
///
/// 非有限/非正值 `samples_per_tick` 返回 `None`（调用方跳过该音符，绝不 panic）。
pub use yeban_model::time_math::tick_to_sample;

/// 每 tick 的样本数（`bpm` 非法时退回 120 BPM；`sample_rate == 0` 时退回 48 kHz）。
///
/// 算式本身只有一份：[`yeban_model::time_math::samples_per_tick`]（`#247`）。
/// 本函数只保留**引擎的兜底口径**（与 MCP 的"病态 ⇒ 0"不同），因此调用方看到的
/// 数值与 `#247` 之前逐位相同。
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
    yeban_model::time_math::samples_per_tick(bpm, PPQ, sample_rate)
}

/// tick → 帧的**构造期**适配层：空 tempo_map 走恒定 BPM 旧路径（逐位不变），
/// 非空走模型层的分段常速换算（`#339`，[ADR-0021]）。
///
/// 为什么要有这一层：投影代码（`project_schedules` / `project_audio_clips`）在
/// `#339` 之前直接乘每 tick 帧数，`project.tempo_map` 因此**静默无效**。把
/// "有没有变速"这一个分支收在这里，投影本身只调 `sample_at`。
///
/// ⚠ 空表的路径**必须**继续用 [`tick_to_sample`]：它的 `round` + 饱和口径是与
/// 既有母带逐位一致的判据（`TempoMap::frames_at_tick` 的空表分支虽然也委托给
/// 同一个函数，但这里直接用省一次分支且让"空表 = 旧行为"在代码上是显然的）。
#[derive(Clone, Copy)]
pub(crate) struct TickTimebase<'a> {
    tempo: &'a yeban_model::time_math::TempoMap,
    constant_per_tick: f64,
    sample_rate: u32,
}

impl<'a> TickTimebase<'a> {
    pub(crate) fn new(tempo: &'a yeban_model::time_math::TempoMap, sample_rate: u32) -> Self {
        Self {
            tempo,
            constant_per_tick: samples_per_tick(tempo.base_bpm(), sample_rate),
            sample_rate,
        }
    }

    /// 绝对 tick → 绝对帧。
    pub(crate) fn sample_at(&self, tick: u64) -> Option<u64> {
        if self.tempo.is_constant() {
            tick_to_sample(tick, self.constant_per_tick)
        } else {
            self.tempo.frames_at_tick(tick, PPQ, self.sample_rate)
        }
    }

    /// 相对 `anchor` 的本地 tick 偏移 → 本地帧偏移（弯音/滑音的局部坐标用）。
    ///
    /// 空表 ⇒ 与 [`tick_to_sample`] 同式（逐位不变）；非空 ⇒ 先各自换算成绝对帧
    /// 再相减（`frames_at_tick` 每段只舍入一次，差值因此仍单调）。
    pub(crate) fn local_sample(&self, anchor: u64, local: u64) -> Option<u64> {
        if self.tempo.is_constant() {
            tick_to_sample(local, self.constant_per_tick)
        } else {
            let end =
                self.tempo
                    .frames_at_tick(anchor.saturating_add(local), PPQ, self.sample_rate)?;
            let start = self.tempo.frames_at_tick(anchor, PPQ, self.sample_rate)?;
            end.checked_sub(start)
        }
    }
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

/// ⭐ G5（审计 2026-10-27）：[`ToneParams::from_devices`] **真的消费**的
/// 内置乐器参数名（小写）的**权威清单**。
///
/// 存在意义：离线路径（`yeban-render` 的 `deviceChainDsp` 登记）需要一个
/// **不需要再抄一份**的判据来回答"这个乐器参数有没有被音源求值"。清单与
/// `from_devices` 的 `match` 分支由判据测试
/// `every_consumed_instrument_param_reaches_the_engine`（本文件 tests）钉在一起 ——
/// 两边漂移 ⇒ 测试红。
///
/// 音色选择的入口就是这里的前两项：`osc1_wave` / `osc2_wave` 的取值是
/// [`ENGINE_WAVE_RECIPES`] 的**下标**（0=hollow / 1=organ / 2=vocal /
/// 3=metallic / 4=glass，钳位规则见 [`wave_index`]）。
///
/// ⭐ `T-FIX-273` 追加四条包络键（`attack_s` / `decay_s` / `sustain` / `release_s`）：
/// `from_devices` 消费它们、[`ToneParams::poly_synth_params`] 经
/// [`PolySynthParams::with_envelope`] 把它们送进器件 ⇒ 离线路径**不许**再把它们登记成
/// "没被求值"（否则 `deviceChainDsp` 会对一条真的在响的轨报假话）。
/// 数组长度因此从 11 变成 15；判据
/// `every_consumed_instrument_param_reaches_the_engine_projection` 对每条新名字
/// 都要求"注入后投影必须变"。
///
/// ⭐ `T-FIX-277` 追加四条 LFO 键（`lfo_wave` / `lfo_rate_hz` / `lfo_depth` /
/// `lfo_target`）：`from_devices` 消费它们、[`ToneParams::poly_synth_params`] 经
/// [`PolySynthParams::with_lfo`] 把它们送进器件 ⇒ 离线路径**不许**再把它们登记成
/// "没被求值"。数组长度因此从 15 变成 19；判据同上。
pub const CONSUMED_INSTRUMENT_PARAMS: [&str; 19] = [
    "cutoff_hz",
    "cutoff",
    "resonance",
    "res",
    "drive",
    "osc1_wave",
    "osc2_wave",
    "osc1_level",
    "osc1_detune_cents",
    "osc2_level",
    "osc2_detune_cents",
    "attack_s",
    "decay_s",
    "sustain",
    "release_s",
    "lfo_wave",
    "lfo_rate_hz",
    "lfo_depth",
    "lfo_target",
];

/// [`CONSUMED_INSTRUMENT_PARAMS`] 的成员判定 **∪** 鼓机键位/旋钮名
/// （[`crate::drums::consumes_drum_param`]）—— 即"内置乐器设备的这条参数
/// 会被**某个**音源路径真实求值"。入参**必须**已转小写。
#[must_use]
pub fn is_consumed_instrument_param(name_lower: &str) -> bool {
    CONSUMED_INSTRUMENT_PARAMS.contains(&name_lower)
        || crate::drums::consumes_drum_param(name_lower)
}

/// 音轨线性增益（dB → 线性；非有限输入按静音处理）。超越函数类，构造期一次。
///
/// ## 值域（`line/engine-23` 的实测边界，**不改本函数**）
///
/// 本函数只**要求有限**：`volume_db ≤ -120` 给 `0.0`，其上是
/// `yeban_dsp::math::db_to_gain`。后者对**溢出**的 `exp2` 结果归零
/// （`yeban_dsp::math::exp2` 对非有限结果返回 `0.0`），因此 `volume_db ≥ ~770.7`
/// 得到的是**静音**（实测 `771.0 → 0.0`），而 `~760 … ~770.6` 之间是一段
/// **有限但极大**的增益（`770.0 → 3.1622822e38`）。
///
/// ⚠ 那一段的乘积会**上溢成 `±∞`**：`TrackV3::validate` 只要求 `volume_db` 有限
/// （没有上界）⇒ `volume_db = 770.0` 是**模型接受**的工程。本函数**不**把这段夹掉
/// （那会改变既有渲染输出，且需要一条"值域上限"的裁决）；引擎保证的是**信号侧**
/// 不会把非有限值交给母线限制器 —— 见 `crate::rt` 的 `render_block` 步骤 3b⁰
/// （母线有限值守卫）。
#[must_use]
pub fn track_gain(volume_db: f32) -> f32 {
    if volume_db.is_finite() {
        db_to_gain(volume_db)
    } else {
        0.0
    }
}
/// ⭐ **单一来源**：一条轨"与力度无关"的增益门 ＝ **可闻门 × 推子**。
///
/// ⚠ 本函数是那段组合的**唯一**实现：⭐ 快照投影（`snapshot.rs`）与声部槽
/// （[`SynthEngine::set_track_gain_scales`]）**都调它** ⇒ ⭐ 两条路按**构造**一致，
/// ⛔ 不是靠两处各写一遍再对齐。
#[must_use]
pub fn track_gain_scale(
    mute: bool,
    solo: bool,
    solo_safe: bool,
    any_solo: bool,
    volume_db: f32,
) -> f32 {
    if track_is_audible(mute, solo, solo_safe, any_solo) {
        track_gain(volume_db)
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
        engine.render_track(id, Some(schedule), None, &mut out);
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

    /// ⭐ `#247` 判据：引擎的公开换算与模型层在
    /// **11 BPM × 3 采样率 × tick 0..=4000** 上逐项相同（0 分歧）。
    ///
    /// 引擎保留 `samples_per_tick` 的兜底口径（非法速度 ⇒ 120 BPM、采样率 0 ⇒
    /// 48 kHz），但算式与 `round` 只有模型层一份；本判据让"有人把旧算式搬回
    /// `synth.rs`"或"兜底改动碰了正常路径"都立刻变红。
    #[test]
    fn the_engine_agrees_with_the_model_on_the_whole_tick_grid() {
        const BPMS: [f64; 11] = [
            30.0, 60.0, 72.0, 100.0, 110.0, 120.0, 128.0, 130.0, 140.0, 174.0, 200.0,
        ];
        let mut checked = 0usize;
        for bpm in BPMS {
            for rate in [44_100u32, 48_000, 96_000] {
                let spt = samples_per_tick(bpm, rate);
                for tick in 0..=4_000u64 {
                    let ours = tick_to_sample(tick, spt).expect("合法速度/采样率");
                    let model =
                        yeban_model::time_math::ticks_to_frames(tick, bpm, yeban_model::PPQ, rate)
                            .expect("合法速度/采样率");
                    assert_eq!(ours, model, "bpm={bpm} rate={rate} tick={tick}");
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 11 * 3 * 4_001);
        // ⭐ 44.1 kHz 的第一例分歧：引擎仍是 7718（收口没有改引擎的读数）。
        assert_eq!(
            tick_to_sample(168, samples_per_tick(60.0, 44_100)),
            Some(7_718)
        );
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
    ///
    /// ⚠ 用 **4 槽**的小引擎跑（`with_track_slots(.., 4)`），而不是默认的 1024：
    /// 这条判据量的是"越界只计数"这条**边界**，⛔ 不是容量本身 ⇒ 边界必须能被
    /// 一次性踩到，否则判据既慢又测不到东西。
    #[test]
    fn slot_exhaustion_is_counted_and_never_panics() {
        let mut engine = SynthEngine::with_track_slots(48_000, 4);
        assert_eq!(engine.track_slots(), 4, "容量必须是运行期可配的那一个");
        let ids: Vec<EntityId> = (0..7).map(|_| EntityId::new()).collect();
        engine.begin_snapshot(48_000, &ids, [], []);
        assert_eq!(engine.track_drops(), 3);
        assert_eq!(engine.table_levels(), yeban_dsp::oscillator::LEVELS);
    }

    /// 判据：**默认**容量是 [`DEFAULT_TRACK_SLOTS`]（1024），且越界请求被钳。
    #[test]
    fn track_slot_capacity_is_configurable_and_clamped() {
        assert_eq!(SynthEngine::new(48_000).track_slots(), DEFAULT_TRACK_SLOTS);
        assert_eq!(
            SynthEngine::with_track_slots(48_000, 0).track_slots(),
            MIN_TRACK_SLOTS,
            "0 必须被钳到下限（⛔ 不是建一个没有槽的引擎）"
        );
        assert_eq!(
            SynthEngine::with_track_slots(48_000, MAX_TRACK_SLOTS + 1).track_slots(),
            MAX_TRACK_SLOTS,
            "超上限必须被钳（⛔ 不是无界分配）"
        );
        let engine = SynthEngine::with_track_slots(48_000, 32);
        assert_eq!(engine.track_slots(), 32);
        assert_eq!(engine.requested_track_slots(), 32);
    }

    /// 判据：**鼓机投影**的槽位边界与声部池是**同一条边界**（`>= capacity`）。
    ///
    /// `begin_snapshot` 把本快照的鼓机收进构造期分配的暂存
    /// `wanted_drums: Box<[Option<(EntityId, DrumsParams)>]>`（长度 = 槽位数）。
    /// 守卫若写成 `wanted_drums_len > capacity`，第 `capacity + 1` 条鼓机轨就会写
    /// `wanted_drums[capacity]` ⇒ **越界 panic**。声部池那一侧（上一条判据）早已被
    /// 钉住，鼓机这一侧此前没有判据 —— 同一道边界的**两半必须各自有判据**。
    ///
    /// **量什么**：`capacity + 1` 条鼓机轨下 `begin_snapshot` 是否返回
    /// （不 panic）、`track_drops`（条）、以及第一条鼓机轨是否真的被武装（覆盖度见证）。
    ///
    /// ⚠ 容量取 **4**（运行期可配）⇒ 边界一次性踩到；用默认 1024 跑这条判据
    /// 既慢又只是在"没踩到边界"上拿绿灯。
    ///
    /// 注入实测：`if wanted_drums_len >= capacity {` → `>`
    /// （`synth.rs`）⇒ 本判据实测变红（`index out of bounds`）。
    #[test]
    fn drum_slot_exhaustion_is_counted_and_never_panics() {
        let device = DeviceDefinition {
            id: EntityId::new(),
            name: "Kit".to_owned(),
            kind: DeviceKind::InternalInstrument,
            bypassed: false,
            params: [
                ("kick_note", 36.0),
                ("snare_note", 38.0),
                ("closed_hat_note", 42.0),
                ("open_hat_note", 46.0),
                ("clap_note", 39.0),
            ]
            .iter()
            .map(|(name, value)| yeban_model::ParameterValue {
                name: (*name).to_owned(),
                value: *value,
                unit: None,
            })
            .collect(),
            latency_samples: 0,
            plugin: None,
        };
        let params = DrumsParams::from_devices(&[device]).expect("五个键位写全 ⇒ 必须武装");
        const CAPACITY: usize = 4;
        let ids: Vec<EntityId> = (0..CAPACITY + 1).map(|_| EntityId::new()).collect();
        let drums: Vec<(&EntityId, &DrumsParams)> = ids.iter().map(|id| (id, &params)).collect();

        let mut engine = SynthEngine::with_track_slots(48_000, CAPACITY);
        engine.begin_snapshot(48_000, &ids, [], drums);
        assert_eq!(
            engine.track_drops(),
            1,
            "第 {CAPACITY} 条轨必须被计数（不 panic、不扩容）"
        );
        assert!(
            engine.drum_params(ids[0]).is_some(),
            "第一条鼓机轨必须真的被武装（覆盖度见证：这条路径真的跑到了）"
        );
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

    /// 判据：复音触发面的**两侧**（引擎派发 vs 器件收到）在**每个**量子边界上恒等，
    /// 且器件计数器只增不减（`seek` / 重新武装都不清它）。
    ///
    /// 量什么（三个数，单位都是**次**）：
    ///
    /// | 读数 | 数什么 | 出处 |
    /// | :--- | :--- | :--- |
    /// | [`SynthEngine::notes_triggered`] | **两条**音源合起来的派发数（鼓机 + 复音） | 引擎派发处 |
    /// | [`SynthEngine::drum_hits`] | 只有鼓机那条的派发数 | 引擎派发处 |
    /// | [`SynthEngine::poly_notes_triggered`] | 只有复音那条的**器件收到**数 | `PolySynth::notes_triggered` |
    ///
    /// 恒等式：`notes_triggered − drum_hits == poly_notes_triggered`。
    ///
    /// 怎么变红：① 把 `poly_notes_triggered` 写成常数（等号与"必须 > 0"同时红）；
    /// ② 把它写成 `notes_triggered` 的副本（本夹具 `drum_hits == 0`，所以这一条**不会**
    /// 红 —— 它由 `tests/drums_instrument.rs` 的 D10 混合夹具抓住）；③ 在复音分支
    /// 漏记一次派发，或让器件 `note_on` 不被调用（等号当场不成立）。
    ///
    /// ⚠ 本夹具的声部数**低于**池容量 ⇒ `voice_steals == 0`：这正是"新读数存在的理由"
    /// 的见证 —— 一条从不溢出的复音工程在 `voice_steals` 上看不到任何触发。
    #[test]
    fn poly_trigger_faces_agree_and_the_device_counter_only_grows() {
        let id = EntityId::new();
        // 一个量子一个起音（128 帧 = 1 个量子）⇒ 8 个量子恰好消费全部 8 个音符。
        let notes: Vec<ScheduledNote> = (0..8)
            .map(|index| note((index * 128) as u64, 40_000, 60, 127, 48_000.0))
            .collect();
        let schedule = NoteSchedule::from_sorted(notes);
        let mut engine = rig(id);
        assert_eq!(engine.poly_notes_triggered(), 0, "冷引擎必须是 0");

        let mut rendered = Vec::new();
        let mut previous = engine.poly_notes_triggered();
        for quantum in 0..8 {
            rendered.extend(render(&mut engine, id, &schedule, 128));
            let now = engine.poly_notes_triggered();
            assert!(
                now >= previous,
                "器件计数器只增不减：量子 {quantum} 读到 {now} < 上一次 {previous}"
            );
            previous = now;
        }
        assert!(
            rendered.iter().any(|sample| *sample != 0.0),
            "夹具必须真的出声（否则'器件收到过触发'是空转）"
        );
        assert_eq!(engine.drum_hits(), 0, "本夹具没有鼓机轨");
        assert_eq!(engine.notes_triggered(), 8, "8 个音符各起音一次");
        assert_eq!(engine.voice_steals(), 0, "声部数低于池容量 ⇒ 不许有窃取");
        assert_eq!(
            engine.poly_notes_triggered(),
            engine.notes_triggered() - engine.drum_hits(),
            "触发面的两侧必须恒等：引擎派发 − 鼓机派发 == 器件收到"
        );
        assert_eq!(engine.poly_notes_triggered(), 8);

        // `seek` + 等价快照重新武装**不**清器件计数器、也不重触发（游标只增不减）。
        engine.begin_snapshot(48_000, &[id], [], []);
        engine.align_cursors(std::iter::once((&id, &schedule)));
        engine.seek(0);
        assert_eq!(
            engine.poly_notes_triggered(),
            8,
            "重新武装与 seek 都不得清零器件计数器"
        );
        assert_eq!(engine.notes_triggered(), 8, "起音只允许发生一次");
    }

    /// 判据：复音声部池的**瞬时占用**读数随重叠起音上升、随包络走完回落。
    ///
    /// 量什么：`SynthEngine::poly_active_voices()`（单位：**声部**）。
    /// 它数的是器件里 `active == true` 的声部，与累计量 [`SynthEngine::notes_triggered`]
    /// / [`SynthEngine::poly_notes_triggered`] 是**两个维度**：后两者只增不减。
    ///
    /// 怎么变红：① 把它写成常数 0（三次等号全红）；② 把它写成
    /// [`SynthEngine::notes_triggered`] 的副本（累计量只增不减 ⇒ 回落那一档红）；
    /// ③ 漏读器件 `PolySynth::active_voices`（冷值那一档红白对照都在）。
    ///
    /// ⚠ 期望值是**由夹具算出**的确定值（8 个重叠起音 / 池容量 16），不是"大于 0"：
    /// 8 < 16 ⇒ 本夹具不产生窃取，占用数因此恰好等于"已起音且未回收"的音符数。
    #[test]
    fn poly_active_voices_is_a_gauge_that_rises_with_overlap_and_falls_on_retire() {
        let id = EntityId::new();
        // 一个量子一个起音（128 帧 = 1 个量子）；终点 40000 远在 8 个量子之后
        // ⇒ 8 个音符全部重叠。
        let notes: Vec<ScheduledNote> = (0..8)
            .map(|index| note((index * 128) as u64, 40_000, 60, 127, 48_000.0))
            .collect();
        let schedule = NoteSchedule::from_sorted(notes);
        let mut engine = rig(id);
        assert_eq!(engine.poly_active_voices(), 0, "冷引擎的占用必须是 0");

        for quantum in 0..8usize {
            render(&mut engine, id, &schedule, 128);
            assert_eq!(
                engine.poly_active_voices(),
                (quantum + 1) as u64,
                "第 {} 个量子之后必须有 {} 个声部在响（占用是量规，不是累计量）",
                quantum + 1,
                quantum + 1
            );
        }
        assert_eq!(engine.voice_steals(), 0, "8 个声部 < 池容量 16 ⇒ 不许窃取");
        assert!(
            engine.poly_active_voices() <= VOICES_PER_TRACK as u64,
            "占用不可能超过池容量"
        );
        let peak = engine.poly_active_voices();

        // 走完终点（40000）＋释放（50 ms @48 kHz = 2400 帧）⇒ 声部全部回收。
        // 400 个量子 = 51200 帧，位置 52224 > 42400。
        let mut tail_nonzero = 0usize;
        for _ in 0..400 {
            tail_nonzero += render(&mut engine, id, &schedule, 128)
                .iter()
                .filter(|sample| **sample != 0.0)
                .count();
        }
        println!(
            "[engine-synth/G1] 复音占用量规: 8 个重叠起音后={peak} 尾段(400 量子)后={} \
             累计触发={} 窃取={} 尾段非零={tail_nonzero} 池容量={VOICES_PER_TRACK}",
            engine.poly_active_voices(),
            engine.notes_triggered(),
            engine.voice_steals(),
        );
        assert!(tail_nonzero > 0, "尾段必须真的出过声（否则回落是空转）");
        assert_eq!(
            engine.poly_active_voices(),
            0,
            "包络走完、声部回收 ⇒ 占用必须回落到 0（累计量则不回退）"
        );
        assert_eq!(
            engine.notes_triggered(),
            8,
            "累计触发数在占用回落后**不得**跟着回落（两个维度必须分开）"
        );
    }

    /// 判据：没有槽位/不在快照里的轨道渲染静音且不 panic。
    #[test]
    fn unknown_track_renders_silence() {
        let mut engine = SynthEngine::new(48_000);
        let mut out = [1.0f32; 128];
        engine.render_track(EntityId::new(), None, None, &mut out);
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

    /// ⭐ G5（审计 2026-10-27）判据：[`CONSUMED_INSTRUMENT_PARAMS`] 与
    /// [`ToneParams::from_devices`] 的 `match` **不许漂移**。
    ///
    /// **量什么**：对清单里每个名字，`from_devices` 在"已有 `cutoff_hz` 来源"的
    /// 底座上多加这一个参数（值 `0.5`，对每个字段都远离该字段的默认值）后，
    /// 投影结果必须**改变**；名单之外的名字必须**不改变**。
    /// 注入实测：把清单里任何一项删掉/改名而 `match` 不动 ⇒ 该名字"不改变"分支
    /// 红；`match` 少一个臂而清单留着 ⇒ "必须改变"分支红。
    #[test]
    fn every_consumed_instrument_param_reaches_the_engine_projection() {
        let base = [
            ("cutoff_hz", 1000.0),
            ("resonance", 0.0),
            ("drive", 0.0),
            ("osc1_wave", 0.0),
            ("osc2_wave", 0.0),
            ("osc1_level", 1.0),
            ("osc1_detune_cents", 0.0),
            ("osc2_level", 0.0),
            ("osc2_detune_cents", 0.0),
            // ⭐ `T-FIX-273`：四条包络键进底座（值 = 默认）。注入值 `0.5` 对四个字段
            // 都远离默认（0.005 / 0.08 / 0.7 / 0.05）⇒ "必须改变"分支不空转。
            ("attack_s", 0.005),
            ("decay_s", 0.08),
            ("sustain", 0.7),
            ("release_s", 0.05),
            // ⭐ `T-FIX-277`：四条 LFO 键进底座（值 = 默认）。注入值 `0.5` 对四个字段
            // 都远离默认：波形下标 `0.5` 四舍五入到 `1`（三角 ≠ 正弦）、速率
            // `0.5 ≠ 5.0`、深度 `0.5 ≠ 0.0`、目标下标 `0.5` 四舍五入到 `1`
            // （振幅 ≠ 音高）⇒ "必须改变"分支不空转。
            ("lfo_wave", 0.0),
            ("lfo_rate_hz", 5.0),
            ("lfo_depth", 0.0),
            ("lfo_target", 0.0),
        ];
        let device_with = |name: &str, value: f32| {
            let mut params: Vec<yeban_model::ParameterValue> = base
                .iter()
                .map(|(n, v)| yeban_model::ParameterValue {
                    name: (*n).to_owned(),
                    value: *v,
                    unit: None,
                })
                .collect();
            params.push(yeban_model::ParameterValue {
                name: name.to_owned(),
                value,
                unit: None,
            });
            DeviceDefinition {
                id: EntityId::new(),
                name: "T".to_owned(),
                kind: DeviceKind::InternalInstrument,
                bypassed: false,
                params,
                latency_samples: 0,
                plugin: None,
            }
        };
        let baseline = ToneParams::from_devices(std::slice::from_ref(&device_with(
            "definitely_not_a_real_knob",
            0.5,
        )));
        for name in CONSUMED_INSTRUMENT_PARAMS {
            assert!(
                is_consumed_instrument_param(name),
                "清单里的名字必须被谓词承认: {name}"
            );
            assert_ne!(
                ToneParams::from_devices(std::slice::from_ref(&device_with(name, 0.5))),
                baseline,
                "`{name}` 在清单里 ⇒ `from_devices` 必须真的消费它（清单与 match 漂移）"
            );
        }
        assert!(
            !is_consumed_instrument_param("definitely_not_a_real_knob"),
            "名单之外的名字不许被谓词承认"
        );
    }
}
