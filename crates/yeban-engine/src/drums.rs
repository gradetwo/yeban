//! **每轨鼓机音源**的引擎侧接线：把 `TrackV3.devices` 里的一台
//! [`DeviceKind::InternalInstrument`] 设备投影成音频线程可执行的**鼓机**
//! （`yeban_dsp::drums`）。[ARCH-RT-001, ARCH-RT-004, ARCH-DSP-001, ARCH-DET-001]
//!
//! 识别用的名字表（键位 `note_slot` / 可选键位 `extra_note_slot` /
//! 旋钮 `apply_kit_knob` 与诊断 `unknown_kit_param_names` /
//! `incomplete_drum_note_map` / `consumes_drum_param`）**不住这里**：
//! T-FIX-846 把它们下移到了 `crates/yeban-model/src/drums.rs`
//! （Round 261/273 的取向：共享件下移到轻量 crate，而不是加依赖；
//! 引擎保留 `pub use` 再导出，`DrumsParams::from_devices` 的行为一个字未变）。
//!
//! 本模块是本 crate 接线的**第 12 个、也是最后一个** `yeban_dsp` 模块名
//! （量法：`grep -rn 'use yeban_dsp' crates/yeban-engine/src`，只数真实
//! `use` / `pub use` 行；接线前 `drums` 在该目录里只命中 3 行注释）。
//!
//! ## 1. 规范出处（原文引用）
//!
//! | 出处 | 原文 |
//! | :--- | :--- |
//! | `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:110` | 鼓机电路建模 ➔ `crates/yeban-dsp/src/drums/`（808/909 模拟电路物理建模） |
//! | `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:710` | `yeban-dsp/` …纯数学 DSP 库 (真峰值限制器, SSL 压缩, 通道条, 808/909) |
//! | `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:411` | `\| **内部 DSP 拓扑调度** \| - \| **1.00 ms** \| 声部合成、通道条 EQ/压缩与 PDC 延迟线插入计算 \|` |
//!
//! ⚠ 上述三处**没有**给出：鼓机的复音数、参数表、**打击乐键位映射**、以及
//! "一台内置乐器设备如何声明自己是鼓机"。器件侧已把"规范未定义细节"登记为
//! 核实过的结论（`crates/yeban-dsp/src/drums/mod.rs` §1 与 §0.1 的 22 条未实现清单，
//! 其中第 15／16 条点名**没有工程持久化**与**没有引擎接线**）。本次接线因此必须
//! 自己声明两件语义（见 §4），并把它们登记为缺口（见 §7）。
//!
//! ## 2. 为什么是**新模块**，不是 `crate::insert` 的第三个字段
//!
//! [`crate::insert`] 是**效果器链**：`ChannelStrip::process_mono` 与 `Reverb::process`
//! 都是"把轨道已经合成好的单声道/立体声信号逐样本改写"的变换
//! （实时侧消费点 `crates/yeban-engine/src/rt.rs` 的 `strip.process_mono(...)` 与
//! `reverb.process(...)`）。鼓机**不是**这种形状：
//!
//! 1. 它的公共面是**触发 + 渲染**，没有 `process_*`
//!    （`crates/yeban-dsp/src/drums/mod.rs` 的 `trigger` / `render`）；
//! 2. 它的输入不是"上游的样本"，而是**带绝对样本位置的事件**（[`DrumHit::start_sample`]）；
//! 3. 插入链那一段**没有任何事件输入**（`rt.rs` 的 3a' 与 3a''' 只有
//!    `&mut track_scratch[..frames]`），把一台鼓机武装在那里 ⇒ 它永远收不到触发，
//!    而 `DrumMachine::render` 在无占用槽位时**逐位填 0 并返回**
//!    （`crates/yeban-dsp/src/drums/mod.rs`）⇒ 那是"接了线但不发声"的假接线。
//!
//! 因此鼓机的正确位置是**音源**：轨道自己的渲染那一步
//! （[`crate::synth::SynthEngine::render_track`]），在插入链**之前**
//! —— 与 `insert` 模块文档 §8.3 记的插入点（"轨道自己的渲染之后、逐轨电平与 PDC 之前"）
//! 正好形成上下游。
//!
//! ## 3. 投影规则（完全确定、无猜测）
//!
//! 1. 只看 [`DeviceKind::InternalInstrument`] 的设备（[`DeviceKind::InternalEffect`]
//!    是插入链的地盘；`External*` 由插件宿主负责）；`bypassed` 的设备整体忽略；
//! 2. 在**第一个**"键位映射完整"的设备上取值（规则见 3.1）；更早的、
//!    映射不完整的设备**不是来源** ⇒ 继续往下找；
//! 3. 参数名大小写不敏感（`to_ascii_lowercase` 后比较）；同一设备内同名参数**后者胜**；
//! 4. 未出现的音色字段保持 `yeban_dsp` 的设备默认值（[`DrumKitParams::DEFAULT`]）；
//! 5. 非有限值（`NaN`/`±∞`）：**音色字段**按器件默认值取（绝不把 `NaN` 放进快照）；
//!    而**五个键位名**只要有一个缺失或非有限 ⇒ 这台设备**整体不识别**
//!    （理由见 3.1）；
//! 6. 全部取值经 [`DrumKitParams::sanitised`] 钳到合法域 ⇒ 快照里存的就是
//!    音频线程将要用的那一份数。
//!
//! ### 3.1 为什么"五个键位名必须写全"（本模块最重要的一条）
//!
//! 鼓机是**触发式**器件：没有"哪个音符打哪个鼓件"这条映射，它一个音也发不出来。
//! 这条映射**不在工程数据里**：`crates/yeban-model/src/music.rs` 的 `MidiNote` 只有
//! `pitch` / `velocity` / `probability` / `ratchet` / `micro_timing_ticks` / `slide` /
//! `pitch_bend_curve` / `syllable` / `phonemes` —— **没有**鼓件、通道或键位映射字段；
//! 识别用的名字表本身住在 `crates/yeban-model/src/drums.rs`（T-FIX-846 自本模块
//! 下移 —— 名字表是共享词表，不是工程数据）。
//!
//! 于是只有两条路：**(a)** 在引擎里发明一张默认映射（例如通用 MIDI 打击乐键位），
//! **(b)** 要求工程把映射逐件写出来。本模块选 **(b)** ——
//! 理由是 AGENTS.md §6.4 第 4 条的口径（"未证的语义要登记，不要补齐"）：
//! 通用 MIDI 的键位表是外部的、可引用的标准，但"本工程用哪张表"是**工程语义**，
//! 引擎猜一个默认值会让"没写映射的工程"听起来像"写错映射的工程"。
//!
//! 这条规则的第二个作用与 `crate::insert` 模块文档 §4.1 的"**没写旋钮的级不执行**"
//! 同款：**没写全映射的设备不是鼓机** ⇒ 该轨照旧走复音合成器 ⇒ 既有工程**逐位不变**
//! （实测见 §6）。
//!
//! ⭐ **识别失败不再静默**（P2-3 修复）：上述"不识别"对**只写了部分键位**的设备是
//! 误伤面 —— 漏写一个键位的工程得不到任何提示，整轨静音而回执正常。本模块因此
//! 给出诊断读数 [`yeban_model::drums::incomplete_drum_note_map`]：区分"一个键位名都没写"（不是鼓机，
//! 诊断为 `None`）与"写了但没写全"（诊断逐条点名缺的键位名）。用户面落点在工具面：
//! `yeban_edit_notes` 的 `insertDevice` 解析期响亮拒绝（`incompleteDrumNoteMap`），
//! 渲染回执对存量工程给同名预警。识别规则本身（[`DrumsParams::from_devices`]）
//! **一个字未改** ⇒ 完整五键与无键位工程逐位不变。
//!
//! ### 3.2 已识别的名字（每项列出目标字段）
//!
//! 权威名单是 `crates/yeban-model/src/drums.rs` 的三张 match 表
//! （`note_slot` / `extra_note_slot` / `apply_kit_knob`，T-FIX-846 下移）；
//! 下表只作阅读索引，改名只改模型侧。
//!
//! | 目标字段 | 名字 |
//! | :--- | :--- |
//! | `kick_note` | `kick_note` |
//! | `snare_note` | `snare_note` |
//! | `closed_hat_note` | `closed_hat_note` |
//! | `open_hat_note` | `open_hat_note` |
//! | `clap_note` | `clap_note` |
//! | 可选键位（2026-11-18 新增 8 件） | `rim_note`、`tom_low_note`、`tom_mid_note`、`tom_high_note`、`crash_note`、`ride_note`、`cowbell_note`、`shaker_note`（**可选**：不写 = 该件无键位、不触发，不影响五键识别；写了但非有限 = 整台不识别，与五键同规则 5。识别名单在**独立的** `extra_note_slot`，不进 `note_slot` —— 仓内判据按源码钉 `note_slot` 恰为五名） |
//! | `kit.kick.*` | `kick_tune_hz`、`kick_glide_hz`、`kick_glide_s`、`kick_attack_s`、`kick_decay_s`、`kick_level`、`kick_pan` |
//! | `kit.kick_style` | `kick_style`（`0` = 经典、`>= 0.5` = Anatomy，见 [`KickStyle::from_f32`]） |
//! | `kit.anatomy.*` | `anatomy_softness`、`anatomy_click_amount`、`anatomy_tame_highs`、`anatomy_grit`、`anatomy_boom_to_where`、`anatomy_hit_skin`、`anatomy_rumble`、`anatomy_base_pitch_hz`、`anatomy_volume`、`anatomy_sub_mute`、`anatomy_sub_solo`、`anatomy_thump_mute`、`anatomy_thump_solo`、`anatomy_click_mute`、`anatomy_click_solo` |
//! | `kit.snare.*` | `snare_tone_hz`、`snare_tone_ratio`、`snare_tone_decay_s`、`snare_tone_level`、`snare_noise_highpass_hz`、`snare_noise_lowpass_hz`、`snare_noise_decay_s`、`snare_noise_level`、`snare_attack_s`、`snare_level`、`snare_pan` |
//! | `kit.hihat.*` | `hihat_base_hz`、`hihat_highpass_hz`、`hihat_lowpass_hz`、`hihat_attack_s`、`hihat_closed_decay_s`、`hihat_open_decay_s`、`hihat_level`、`hihat_pan` |
//! | `kit.clap.*` | `clap_highpass_hz`、`clap_lowpass_hz`、`clap_bursts`、`clap_burst_decay_s`、`clap_burst_spacing_s`、`clap_attack_s`、`clap_decay_s`、`clap_level`、`clap_pan` |
//! | `kit.tom.*` | `tom_tune_hz`、`tom_glide_hz`、`tom_glide_s`、`tom_attack_s`、`tom_decay_s`、`tom_level`、`tom_pan` |
//! | `kit.rim.*` | `rim_tone_hz`、`rim_noise_highpass_hz`、`rim_noise_lowpass_hz`、`rim_decay_s`、`rim_attack_s`、`rim_level`、`rim_pan` |
//! | `kit.shaker.*` | `shaker_highpass_hz`、`shaker_lowpass_hz`、`shaker_attack_s`、`shaker_decay_s`、`shaker_level`、`shaker_pan` |
//! | `kit.cowbell.*` | `cowbell_freq_a_hz`、`cowbell_freq_b_hz`、`cowbell_highpass_hz`、`cowbell_lowpass_hz`、`cowbell_attack_s`、`cowbell_decay_s`、`cowbell_level`、`cowbell_pan` |
//! | `kit.crash.*` | `crash_base_hz`、`crash_highpass_hz`、`crash_lowpass_hz`、`crash_attack_s`、`crash_decay_s`、`crash_level`、`crash_pan` |
//! | `kit.ride.*` | `ride_base_hz`、`ride_highpass_hz`、`ride_lowpass_hz`、`ride_attack_s`、`ride_decay_s`、`ride_level`、`ride_pan` |
//! | `kit.bass808.*` | `bass808_tune_hz`、`bass808_glide_hz`、`bass808_glide_s`、`bass808_attack_s`、`bass808_decay_s`、`bass808_level`、`bass808_drive`、`bass808_pan`（逐件＋侧链期 4 新增：独立 808 低音，键位名 `bass808_note` 进可选名单槽位 8；`bass808_drive` 为 T-DRUM-12 新增的软饱和驱动量，0 = 逐位不变） |
//! | `kit.master_level` | `master_level` |
//!
//! ⛔ **没有别名**，这是**刻意**的：`crates/yeban-dsp/src/drums/mod.rs` 的音色参数是
//! **逐件**的（4 套配方 × 各自的字段），单名（例如 `level`）会把四个鼓件混成一件。
//! 与 `crate::insert` 模块文档 §4.3 拒收 `filter_cutoff_hz` 同款：**一个名字只指一处**，
//! 歧义直接不认，而不是猜。
//!
//! ## 4. 两条**明说的**语义（本模块声明的，规范没有要求）
//!
//! ### 4.1 鼓机是**音源** ⇒ 该轨不再渲染复音合成器
//!
//! 一条轨**至多一件音源**。出现规则 3.1 的完整映射 ⇒ 这一轨的音源是鼓机，
//! [`crate::synth::SynthEngine::render_track`] **不**调用该槽的复音合成器
//! （否则每一记鼓都会叠上一个波表音）。这条取舍是**明说的**：一台既有
//! `cutoff_hz`（复音合成器）又有完整鼓机映射的设备链，引擎解析为**鼓机优先**，
//! 且这个组合被登记为缺口（§7 的 needs-3）。
//!
//! ⭐ **任务单 `#70` 起，"被忽略"变成可读的**（sfz-needs-3 的信号腿，⛔ 不改次序）：
//! `render_track` 每量子把"本槽武装了几件音源"写进
//! [`crate::synth::SynthEngine::armed_sources`]（复音恒在场 ⇒ 至少 1；鼓机 +1、
//! SFZ +1）⇒ 鼓机＋SFZ＋复音同轨读 `3`、[`crate::synth::SynthEngine::ignored_sources`]
//! 读 `2`。⚠ §7 的 `needs-3` 说的是**模型面**（"一台设备链里几件音源"该由谁裁），
//! 那一半仍未闭合；本信号只让**引擎侧的取舍**对用户可见。
//!
//! ### 4.2 一个音高只触发**一个**鼓件（映射碰撞的裁决）
//!
//! 两个鼓件写了同一个音高时，按 [`DrumVoice`] 的判别值顺序
//! （`kick` → `snare` → `closed_hat` → `open_hat` → `clap`）取**第一个**。
//! 这是全序、与平台无关 [ARCH-DET-001]，不是"任选一个"。
//!
//! ## 5. 实时分类（[ADR-0001 D32]）
//!
//! - **构造期**（控制线程，[`DrumsParams::from_devices`]）：字符串比较、`Vec` 扫描、
//!   `DrumKitParams::sanitised` 的钳制 —— 允许分配。
//! - **快照边界**（音频线程，每个修订一次）：[`DrumMachine::set_params`] 与
//!   [`DrumMachine::set_sample_rate`] 会重算在响槽位的包络系数与滤波器系数
//!   （`exp`/`tan` ⇒ 超越函数类）。两者都**不碰堆**（`DrumMachine` 是定长数组：
//!   `crates/yeban-dsp/src/drums/mod.rs` 的 `[Slot; SLOTS]`）—— 实测由
//!   `crates/yeban-engine/tests/synth_rt_zero_alloc.rs` 的鼓机场景钉住。
//! - **逐样本**（[`DrumMachine::trigger`] / [`DrumMachine::render`]）：整数相位推进、
//!   包络乘加、一阶递归、环形缓冲读写；**零分配、零锁、零 I/O、零日志**
//!   [ARCH-RT-001 / MUST-GATE-001]。
//!
//! ## 6. 默认口径：没有完整映射的工程**逐位不变**
//!
//! `EngineSnapshot::drums` 只收录**识别出鼓机**的轨道 ⇒ 其余工程的实时侧
//! **整段不执行**鼓机代码（不是"参数取成透明"）。实测（同一个夹具、同一台机器、
//! 接线**前**测得的字面读数，接线后由 `tests/drums_instrument.rs` 的 D0 原样重测）：
//!
//! | 夹具 | 帧数 | 非零样本 | 峰值 | 指纹（FNV-1a 64，逐位） |
//! | :--- | :--- | :--- | :--- | :--- |
//! | 两个音符、无设备链 | 2560 | 5052 | 0.5478619 | `0x71dca4dc55b44df5` |
//! | 两个音符、`cutoff_hz = 800` / `resonance = 0.3` | 2560 | 5052 | 0.32756245 | `0x6eee378072730521` |
//! | 两个音符、设备链只有 `kick_tune_hz` / `snare_tone_hz`（无映射） | 2560 | 5052 | 0.5478619 | `0x71dca4dc55b44df5` |
//!
//! ⭐ 第三行就是规则 3.1 的实测：**只写音色旋钮**的设备与"根本没有设备链"逐位同解。
//!
//! ## 7. 这是**临时形状**，并登记三条缺口
//!
//! 与 [`crate::synth::ToneParams`] / [`crate::insert`] 同族：`yeban-model` 补齐
//! "乐器参数 → 音频线程"的投影（`docs/ledger/engine-mix-notes.md` §8.2 的 **N5**）之后，
//! 本模块的投影部分应整体删除，只保留 `pub use` 与参数类型的再导出。
//!
//! | # | 缺口 | 为什么需要模型侧裁决 |
//! | :--- | :--- | :--- |
//! | needs-1 | **"这台设备是鼓机"的类型级信息**：`DeviceKind` 只有 `InternalInstrument` / `InternalEffect` / `External*`（`crates/yeban-model/src/project.rs:403`），没有鼓机档 | 今天的识别靠"五个键位名写全"这一条**名称约定**；类型级信息落下后这条约定删除 |
//! | needs-2 | **打击乐键位映射的形状**：`MidiNote` 没有鼓件/通道字段（见 §3.1） | 今天由工程在设备参数里逐件写出；模型若给出 `InstrumentDefinition` 的键位表，这条要求删除 |
//! | needs-3 | ⭐ **已闭合（任务单 `#96`，模型面）**：同轨多件音源的裁决 —— 模型层**不禁止**同一轨既有 `TrackV3.sampler` 又有会被本模块当鼓机的 `InternalInstrument` 设备 | ⭐ 裁决：本条自己的措辞「**模型若禁止这种组合，本条删除**」那一支**不成立** ⇒ **本条删除**（`DeviceKind` 四档无鼓机档；识别用的名字表住在 `yeban-model/src/drums.rs`，T-FIX-846 下移）。引擎侧的既有权衡**照旧**、且**明说它是引擎的裁决**：`鼓机 ＞ SFZ ＞ 复音`（§4.1），可读信号 [`SynthEngine::armed_sources`]／[`SynthEngine::ignored_sources`]（`sfz-needs-3` 的信号腿，判据 S20）。常驻判据：`crates/yeban-model/tests/track_multi_source_coexistence.rs` 的 `model_allows_a_track_to_hold_both_a_sampler_and_a_drum_shaped_instrument` ＋ `the_five_key_names_are_exactly_the_models_drum_recognition_names` |
//!
//! ## 8. 判据与注入
//!
//! 单元判据（本文件 `mod tests`）+ 端到端判据（`tests/drums_instrument.rs`）。
//! 注入 → 变红的实测记录写在提交的报告里。
//!
//! ⚠ 源码级判据 `engine_drums_module_has_no_second_drum_implementation` 对**本文件的
//! 注释**也生效：注释里不许出现被禁的实现记号（用 `concat!` 拼记号，理由与
//! `crate::insert` 的三条同款判据相同）。

use yeban_model::drums::{
    EXTRA_NOTE_SLOTS, NOTE_SLOTS, apply_kit_knob, extra_note_slot, note_from_f32, note_slot,
};
use yeban_model::{DeviceDefinition, DeviceKind};

// 鼓机的**唯一实现**住在 `yeban-dsp`（`crates/yeban-dsp/src/drums/mod.rs`）。
// 本模块只转发：器件 + 它的四个音色参数类型 + 音色身份 + 一次触发请求。
// 判据（`tests/drums_instrument.rs` 的 D2）按这些名字断言"引擎的鼓机**就是** dsp 的鼓机"
// （类型同一性）。
pub use yeban_dsp::drums::{
    ANATOMY_KICK_PRESETS, AnatomyKickCategory, AnatomyKickParams, AnatomyKickPreset, ClapParams,
    CowbellParams, CrashParams, DRUM_SLOTS, DrumHit, DrumKitParams, DrumMachine, DrumVoice,
    HiHatParams, KickParams, KickStyle, RideParams, RimParams, ShakerParams, SnareParams,
    TomParams,
};

// 识别表住在 `yeban-model`（T-FIX-846 下移）；引擎保留再导出，
// 既有调用方（`synth.rs`、本模块判据）一个字不用改。
pub use yeban_model::drums::{
    consumes_drum_param, incomplete_drum_note_map, unknown_kit_param_names,
};

/// 一台鼓机的**键位映射**：五个必备鼓件 + 八个可选鼓件各自的 MIDI
/// 音高（`0..=127`）。
///
/// 它**必须**由工程逐件写出（模块文档 §3.1）：引擎不发明默认键位。
/// 一个音高只触发一个鼓件，碰撞时的裁决见模块文档 §4.2。可选八件
/// （2026-11-18）不写就没有键位、不影响五键识别。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrumNoteMap {
    /// 底鼓的音高。
    kick: u8,
    /// 军鼓的音高。
    snare: u8,
    /// 闭镲的音高。
    closed_hat: u8,
    /// 开镲的音高。
    open_hat: u8,
    /// 拍手的音高。
    clap: u8,
    /// 可选八件的音高（下标 = [`yeban_model::drums::extra_note_slot`] 的槽位序；
    /// `None` = 工程没给这件写键位）。
    extra: [Option<u8>; EXTRA_NOTE_SLOTS],
}

impl DrumNoteMap {
    /// 逐件给出音高（构造期；值域 `0..=127` 由 [`DrumsParams::from_devices`] 保证）。
    /// 可选八件默认无键位，用 [`Self::with_extra`] 逐件补。
    #[must_use]
    pub const fn new(kick: u8, snare: u8, closed_hat: u8, open_hat: u8, clap: u8) -> Self {
        Self {
            kick,
            snare,
            closed_hat,
            open_hat,
            clap,
            extra: [None; EXTRA_NOTE_SLOTS],
        }
    }

    /// 给一件**可选**鼓件设键位（构造期链式调用）。核心五件走
    /// [`Self::new`]，传核心五件到这里是无操作（它们的键位不可被
    /// 可选通道覆盖）。
    #[must_use]
    pub const fn with_extra(mut self, voice: DrumVoice, note: u8) -> Self {
        match voice {
            DrumVoice::Rimshot => self.extra[0] = Some(note),
            DrumVoice::TomLow => self.extra[1] = Some(note),
            DrumVoice::TomMid => self.extra[2] = Some(note),
            DrumVoice::TomHigh => self.extra[3] = Some(note),
            DrumVoice::Crash => self.extra[4] = Some(note),
            DrumVoice::Ride => self.extra[5] = Some(note),
            DrumVoice::Cowbell => self.extra[6] = Some(note),
            DrumVoice::Shaker => self.extra[7] = Some(note),
            DrumVoice::Bass808 => self.extra[8] = Some(note),
            _ => {}
        }
        self
    }

    /// 该音高触发哪个鼓件（`None` = 本映射里没有它 ⇒ 不触发）。
    ///
    /// 顺序：核心五件先（判别值顺序，模块文档 §4.2 的碰撞裁决），
    /// 然后可选八件按 [`yeban_model::drums::DRUM_EXTRA_NOTE_NAMES`] 的槽位序。
    #[must_use]
    pub const fn voice_for(&self, pitch: u8) -> Option<DrumVoice> {
        if pitch == self.kick {
            Some(DrumVoice::Kick)
        } else if pitch == self.snare {
            Some(DrumVoice::Snare)
        } else if pitch == self.closed_hat {
            Some(DrumVoice::ClosedHat)
        } else if pitch == self.open_hat {
            Some(DrumVoice::OpenHat)
        } else if pitch == self.clap {
            Some(DrumVoice::Clap)
        } else {
            let mut index = 0;
            while index < EXTRA_NOTE_SLOTS {
                if let Some(note) = self.extra[index]
                    && note == pitch
                {
                    return Some(EXTRA_VOICES[index]);
                }
                index += 1;
            }
            None
        }
    }

    /// 该鼓件的音高（[`Self::voice_for`] 的逆，碰撞时可能不成立 —— 见模块文档 §4.2）。
    /// 可选八件未写键位时返回 `0`（哨兵值：用 [`Self::extra_note_for`]
    /// 区分"键位是 0"与"没写"）。
    #[must_use]
    pub const fn note_for(&self, voice: DrumVoice) -> u8 {
        match voice {
            DrumVoice::Kick => self.kick,
            DrumVoice::Snare => self.snare,
            DrumVoice::ClosedHat => self.closed_hat,
            DrumVoice::OpenHat => self.open_hat,
            DrumVoice::Clap => self.clap,
            _ => match self.extra_note_for(voice) {
                Some(note) => note,
                None => 0,
            },
        }
    }

    /// 可选八件的键位（`None` = 没写；核心五件恒 `Some`）。
    #[must_use]
    pub const fn extra_note_for(&self, voice: DrumVoice) -> Option<u8> {
        match voice {
            DrumVoice::Kick => Some(self.kick),
            DrumVoice::Snare => Some(self.snare),
            DrumVoice::ClosedHat => Some(self.closed_hat),
            DrumVoice::OpenHat => Some(self.open_hat),
            DrumVoice::Clap => Some(self.clap),
            DrumVoice::Rimshot => self.extra[0],
            DrumVoice::TomLow => self.extra[1],
            DrumVoice::TomMid => self.extra[2],
            DrumVoice::TomHigh => self.extra[3],
            DrumVoice::Crash => self.extra[4],
            DrumVoice::Ride => self.extra[5],
            DrumVoice::Cowbell => self.extra[6],
            DrumVoice::Shaker => self.extra[7],
            DrumVoice::Bass808 => self.extra[8],
        }
    }

    /// 五个音高，顺序 = [`DrumVoice::ALL`]（诊断与判据用）。
    #[must_use]
    pub const fn as_array(&self) -> [u8; 5] {
        [
            self.kick,
            self.snare,
            self.closed_hat,
            self.open_hat,
            self.clap,
        ]
    }

    /// 底鼓音高。
    #[must_use]
    pub const fn kick(&self) -> u8 {
        self.kick
    }

    /// 军鼓音高。
    #[must_use]
    pub const fn snare(&self) -> u8 {
        self.snare
    }

    /// 闭镲音高。
    #[must_use]
    pub const fn closed_hat(&self) -> u8 {
        self.closed_hat
    }

    /// 开镲音高。
    #[must_use]
    pub const fn open_hat(&self) -> u8 {
        self.open_hat
    }

    /// 拍手音高。
    #[must_use]
    pub const fn clap(&self) -> u8 {
        self.clap
    }
}

/// 一台轨的**鼓机投影**：音色参数 + 键位映射（**引擎侧临时形状**，见模块文档 §7）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrumsParams {
    /// 音色参数（4 套配方 + 总线电平）。
    kit: DrumKitParams,
    /// 键位映射。
    notes: DrumNoteMap,
}

impl DrumsParams {
    /// 组装（判据与离线渲染器用；生产路径走 [`Self::from_devices`]）。
    #[must_use]
    pub fn new(kit: DrumKitParams, notes: DrumNoteMap) -> Self {
        Self {
            kit: kit.sanitised(),
            notes,
        }
    }

    /// 从模型层的设备链投影（**构造期**；规则见模块文档 §3）。
    ///
    /// `None` = 这条轨的设备链里**没有**一台键位映射完整的鼓机 ⇒ 实时侧整段跳过
    /// ⇒ 该轨的输出与接线前**逐位相同**（模块文档 §6）。
    #[must_use]
    pub fn from_devices(devices: &[DeviceDefinition]) -> Option<Self> {
        for device in devices {
            // 规则 1：只看非旁通的内置乐器。
            if device.bypassed || device.kind != DeviceKind::InternalInstrument {
                continue;
            }
            if let Some(params) = Self::from_device(device) {
                return Some(params);
            }
        }
        None
    }

    /// 单台设备 → 鼓机投影（`None` = 映射不完整或一件已识别的名字都没有）。
    fn from_device(device: &DeviceDefinition) -> Option<Self> {
        let mut kit = DrumKitParams::DEFAULT;
        // 五个键位：`None` = 没写或写了非有限值 ⇒ 这台设备不是鼓机（规则 5）。
        let mut notes: [Option<u8>; NOTE_SLOTS] = [None; NOTE_SLOTS];
        // 可选八件：不写 = 无键位（不影响识别）；写了但非有限 = 与
        // 五键同规则 5，整台不识别（`extra_broken`）。
        let mut extra: [Option<u8>; EXTRA_NOTE_SLOTS] = [None; EXTRA_NOTE_SLOTS];
        let mut extra_broken = false;
        for param in &device.params {
            let name = param.name.to_ascii_lowercase();
            let value = param.value;
            if let Some(index) = note_slot(&name) {
                notes[index] = note_from_f32(value);
                continue;
            }
            if let Some(index) = extra_note_slot(&name) {
                match note_from_f32(value) {
                    Some(note) => extra[index] = Some(note),
                    None => extra_broken = true,
                }
                continue;
            }
            apply_kit_knob(&mut kit, &name, value);
        }
        if extra_broken {
            return None;
        }
        let mut notes = DrumNoteMap::new(notes[0]?, notes[1]?, notes[2]?, notes[3]?, notes[4]?);
        for (index, note) in extra.iter().enumerate() {
            if let Some(note) = note {
                notes = notes.with_extra(EXTRA_VOICES[index], *note);
            }
        }
        // 规则 6：快照里存的就是音频线程将要用的那一份数。
        Some(Self {
            kit: kit.sanitised(),
            notes,
        })
    }

    /// 音色参数。
    #[must_use]
    pub const fn kit(&self) -> DrumKitParams {
        self.kit
    }

    /// 键位映射。
    #[must_use]
    pub const fn notes(&self) -> DrumNoteMap {
        self.notes
    }
}

/// 可选槽位 → 鼓件（下标与 [`yeban_model::drums::DRUM_EXTRA_NOTE_NAMES`] 同序）。
const EXTRA_VOICES: [DrumVoice; EXTRA_NOTE_SLOTS] = [
    DrumVoice::Rimshot,
    DrumVoice::TomLow,
    DrumVoice::TomMid,
    DrumVoice::TomHigh,
    DrumVoice::Crash,
    DrumVoice::Ride,
    DrumVoice::Cowbell,
    DrumVoice::Shaker,
    DrumVoice::Bass808,
];

#[cfg(test)]
mod tests {

    use super::*;
    use yeban_model::{EntityId, ParameterValue};

    fn device(kind: DeviceKind, bypassed: bool, params: &[(&str, f32)]) -> DeviceDefinition {
        DeviceDefinition {
            id: EntityId::new(),
            name: "Device".to_owned(),
            kind,
            bypassed,
            params: params
                .iter()
                .map(|(name, value)| ParameterValue {
                    name: (*name).to_owned(),
                    value: *value,
                    unit: None,
                })
                .collect(),
            latency_samples: 0,
            plugin: None,
        }
    }

    /// 完整的五个键位（判据里反复用）。
    const NOTES: [(&str, f32); 5] = [
        ("kick_note", 36.0),
        ("snare_note", 38.0),
        ("closed_hat_note", 42.0),
        ("open_hat_note", 46.0),
        ("clap_note", 39.0),
    ];

    /// 把 [`NOTES`] 与额外参数拼成一台设备。
    fn kit_device(kind: DeviceKind, bypassed: bool, extra: &[(&str, f32)]) -> DeviceDefinition {
        let mut params = NOTES.to_vec();
        params.extend_from_slice(extra);
        device(kind, bypassed, &params)
    }

    /// 交叉对账：鼓机形状的设备上，诊断与识别**恰一成立** ——
    /// 诊断说缺键 ⇔ 识别不认；五键齐全 ⇔ 识别认且诊断沉默。
    #[test]
    fn incomplete_drum_note_map_complements_recognition() {
        let complete = kit_device(DeviceKind::InternalInstrument, false, &[]);
        assert!(DrumsParams::from_devices(std::slice::from_ref(&complete)).is_some());
        assert!(incomplete_drum_note_map(&complete).is_none());

        let partial = device(DeviceKind::InternalInstrument, false, &NOTES[..4]);
        assert!(DrumsParams::from_devices(std::slice::from_ref(&partial)).is_none());
        assert_eq!(incomplete_drum_note_map(&partial), Some(vec!["clap_note"]));
    }

    /// **默认口径**：映射不完整 ⇒ 不是鼓机（模块文档 §3.1）。
    #[test]
    fn an_incomplete_note_map_is_not_a_drum_machine() {
        assert!(DrumsParams::from_devices(&[]).is_none());
        // 空设备。
        assert!(
            DrumsParams::from_devices(&[device(DeviceKind::InternalInstrument, false, &[])])
                .is_none()
        );
        // 只有音色旋钮、一个键位都没有。
        assert!(
            DrumsParams::from_devices(&[device(
                DeviceKind::InternalInstrument,
                false,
                &[("kick_tune_hz", 45.0), ("snare_tone_hz", 200.0)],
            )])
            .is_none()
        );
        // 缺一个键位（四个写全、缺 `clap_note`）。
        assert!(
            DrumsParams::from_devices(&[device(
                DeviceKind::InternalInstrument,
                false,
                &NOTES[..4],
            )])
            .is_none()
        );
        // 非有限键位（`NaN`）⇒ 整台设备不识别（规则 5）。
        assert!(
            DrumsParams::from_devices(&[device(
                DeviceKind::InternalInstrument,
                false,
                &[
                    ("kick_note", f32::NAN),
                    ("snare_note", 38.0),
                    ("closed_hat_note", 42.0),
                    ("open_hat_note", 46.0),
                    ("clap_note", 39.0),
                ],
            )])
            .is_none()
        );
        // 效果器不是音源：同一组名字也不认。
        assert!(
            DrumsParams::from_devices(&[kit_device(DeviceKind::InternalEffect, false, &[])])
                .is_none()
        );
        // 外部乐器由插件宿主负责。
        assert!(
            DrumsParams::from_devices(&[kit_device(DeviceKind::ExternalInstrument, false, &[])])
                .is_none()
        );
        // 旁通的设备就是旁通：不许"取默认参数偷偷接回来"。
        assert!(
            DrumsParams::from_devices(&[kit_device(DeviceKind::InternalInstrument, true, &[])])
                .is_none()
        );
    }

    /// **命中**：五个键位立即可用（音色取器件默认值）；音色旋钮逐字段覆盖默认值。
    #[test]
    fn a_complete_note_map_arms_the_kit_and_knobs_override_defaults() {
        let params = DrumsParams::from_devices(&[kit_device(
            DeviceKind::InternalInstrument,
            false,
            &[
                ("KICK_TUNE_HZ", 45.0),
                ("kick_decay_s", 1.25),
                ("clap_bursts", 6.0),
                ("master_level", 0.5),
            ],
        )])
        .expect("五个键位写全 ⇒ 必须武装");
        let notes = params.notes();
        assert_eq!(notes.as_array(), [36, 38, 42, 46, 39]);
        assert_eq!(notes.kick(), 36);
        assert_eq!(notes.clap(), 39);
        // `as_array` 与 `note_for` 必须给出同一组数（核心五件逐件；
        // 可选八件不在 `as_array` 里，走 `extra_note_for`）。
        const CORE_FIVE: [DrumVoice; 5] = [
            DrumVoice::Kick,
            DrumVoice::Snare,
            DrumVoice::ClosedHat,
            DrumVoice::OpenHat,
            DrumVoice::Clap,
        ];
        for (index, voice) in CORE_FIVE.iter().enumerate() {
            assert_eq!(notes.as_array()[index], notes.note_for(*voice));
        }
        // 未写可选键位：`extra_note_for` 为 None、`voice_for` 不触发。
        assert_eq!(notes.extra_note_for(DrumVoice::Shaker), None);
        assert_eq!(notes.voice_for(82), None);
        assert_eq!(notes.note_for(DrumVoice::Kick), 36);
        assert_eq!(notes.note_for(DrumVoice::Snare), 38);
        assert_eq!(notes.note_for(DrumVoice::ClosedHat), 42);
        assert_eq!(notes.note_for(DrumVoice::OpenHat), 46);
        assert_eq!(notes.note_for(DrumVoice::Clap), 39);
        assert_eq!(notes.voice_for(36), Some(DrumVoice::Kick));
        assert_eq!(notes.voice_for(38), Some(DrumVoice::Snare));
        assert_eq!(notes.voice_for(42), Some(DrumVoice::ClosedHat));
        assert_eq!(notes.voice_for(46), Some(DrumVoice::OpenHat));
        assert_eq!(notes.voice_for(39), Some(DrumVoice::Clap));
        assert_eq!(notes.voice_for(60), None, "映射里没有的音高不许触发");
        // 音色字段：写过的覆盖默认值，未写的保持器件默认值（逐位）。
        let kit = params.kit();
        assert_eq!(
            kit.kick.tune_hz.to_bits(),
            45.0f32.to_bits(),
            "名字大小写不敏感"
        );
        assert_eq!(kit.kick.decay_s.to_bits(), 1.25f32.to_bits());
        assert_eq!(kit.clap.bursts, 6);
        assert_eq!(kit.master_level.to_bits(), 0.5f32.to_bits());
        let default = DrumKitParams::DEFAULT;
        assert_eq!(kit.kick.glide_hz.to_bits(), default.kick.glide_hz.to_bits());
        assert_eq!(kit.snare.tone_hz.to_bits(), default.snare.tone_hz.to_bits());
        assert_eq!(kit.hihat.level.to_bits(), default.hihat.level.to_bits());
        assert_eq!(kit.clap.decay_s.to_bits(), default.clap.decay_s.to_bits());
    }

    /// Anatomy 旋钮投影：`kick_style` 与 9 个连续参数 + 6 个开关逐字段覆盖默认值；
    /// 未写的字段保持器件默认值；`NaN` 开关落到"关"。
    #[test]
    fn anatomy_knobs_project_onto_the_kit() {
        let params = DrumsParams::from_devices(&[kit_device(
            DeviceKind::InternalInstrument,
            false,
            &[
                ("kick_style", 1.0),
                ("anatomy_base_pitch_hz", 50.0),
                ("anatomy_grit", 0.5),
                ("anatomy_click_solo", 1.0),
                ("anatomy_sub_mute", f32::NAN),
            ],
        )])
        .expect("五个键位写全 ⇒ 必须武装");
        let kit = params.kit();
        assert_eq!(kit.kick_style, KickStyle::Anatomy);
        assert_eq!(kit.anatomy.base_pitch_hz.to_bits(), 50.0f32.to_bits());
        assert_eq!(kit.anatomy.grit.to_bits(), 0.5f32.to_bits());
        assert!(kit.anatomy.click_solo);
        assert!(!kit.anatomy.sub_mute, "NaN 开关落到关");
        // 未写的字段保持器件默认值。
        let default = AnatomyKickParams::DEFAULT;
        assert_eq!(kit.anatomy.softness.to_bits(), default.softness.to_bits());
        assert_eq!(kit.anatomy.volume.to_bits(), default.volume.to_bits());
        // 经典声部仍是缺省：不写 kick_style ⇒ Classic。
        let classic =
            DrumsParams::from_devices(&[kit_device(DeviceKind::InternalInstrument, false, &[])])
                .expect("五个键位写全 ⇒ 必须武装");
        assert_eq!(classic.kit().kick_style, KickStyle::Classic);
        // `consumes_drum_param` 承认新名字（探针复用同一张认名逻辑）。
        assert!(consumes_drum_param("kick_style"));
        assert!(consumes_drum_param("anatomy_grit"));
        assert!(consumes_drum_param("anatomy_click_solo"));
        assert!(!consumes_drum_param("anatomy_nope"));
    }

    /// 规则 2：映射不完整的设备不是来源，继续找下一条；规则 3：同名后者胜。
    /// 规则 6：非有限音色值不落进快照。
    #[test]
    fn source_selection_is_ordered_and_values_are_sanitised() {
        let chain = [
            device(
                DeviceKind::InternalInstrument,
                false,
                &[("kick_tune_hz", 45.0), ("kick_note", 36.0)],
            ),
            kit_device(
                DeviceKind::InternalInstrument,
                false,
                &[("kick_tune_hz", f32::NAN), ("master_level", f32::INFINITY)],
            ),
        ];
        let params = DrumsParams::from_devices(&chain).expect("第二条必须命中");
        let kit = params.kit();
        assert!(kit.kick.tune_hz.is_finite(), "非有限值必须被钳掉");
        assert!(kit.master_level.is_finite());
        assert_eq!(
            kit.kick.tune_hz.to_bits(),
            yeban_dsp::drums::MIN_FREQ_HZ.to_bits()
        );
        assert_eq!(kit.master_level.to_bits(), 0.0f32.to_bits(), "∞ 增益归 0");
        assert_eq!(params.notes().kick(), 36);

        // 同名后者胜（同一个设备内）。
        let overwritten = DrumsParams::from_devices(&[kit_device(
            DeviceKind::InternalInstrument,
            false,
            &[("kick_note", 1.0), ("kick_note", 2.0)],
        )])
        .expect("键位写全");
        assert_eq!(overwritten.notes().kick(), 2, "同名后者胜");

        // 首个来源不被后来的覆盖。
        let ordered = DrumsParams::from_devices(&[
            kit_device(DeviceKind::InternalInstrument, false, &[("kick_note", 1.0)]),
            kit_device(DeviceKind::InternalInstrument, false, &[("kick_note", 2.0)]),
        ])
        .expect("第一条必须命中");
        assert_eq!(ordered.notes().kick(), 1, "首个来源不被后来的覆盖");
    }

    /// 规则 5 的边界：键位被钳到 `0..=127`（四舍五入）。
    #[test]
    fn note_values_are_clamped_into_the_midi_range() {
        let params = DrumsParams::from_devices(&[kit_device(
            DeviceKind::InternalInstrument,
            false,
            &[
                ("kick_note", -5.0),
                ("snare_note", 200.0),
                ("closed_hat_note", 41.6),
                ("open_hat_note", 46.4),
                ("clap_note", 39.0),
            ],
        )])
        .expect("键位写全");
        assert_eq!(params.notes().as_array(), [0, 127, 42, 46, 39]);
    }

    /// 模块文档 §4.2：两个鼓件写了同一个音高 ⇒ 按 [`DrumVoice`] 判别值顺序取第一个。
    #[test]
    fn colliding_notes_resolve_in_discriminant_order() {
        let params = DrumsParams::from_devices(&[kit_device(
            DeviceKind::InternalInstrument,
            false,
            &[
                ("kick_note", 60.0),
                ("snare_note", 60.0),
                ("closed_hat_note", 60.0),
                ("open_hat_note", 61.0),
                ("clap_note", 61.0),
            ],
        )])
        .expect("键位写全");
        assert_eq!(params.notes().voice_for(60), Some(DrumVoice::Kick));
        assert_eq!(params.notes().voice_for(61), Some(DrumVoice::OpenHat));
    }

    /// 判据：**engine 侧没有第二份鼓机实现**（源码级机械检查）。
    ///
    /// 与 `crate::insert` 的三条同类判据同款，对准 `yeban_dsp::drums`。
    /// 注入：把 `DrumMachine` / `DrumKitParams` 的类型定义或它的任何一个逐样本内核
    /// 复制进本文件 ⇒ 本判据立即变红。
    /// ⚠ 与前三条同样的自我命中风险：本条注释里也**不许**出现被禁字面量。
    #[test]
    fn engine_drums_module_has_no_second_drum_implementation() {
        let source = include_str!("drums.rs");
        let forbidden = [
            concat!("struct", " DrumMachine"),
            concat!("struct", " DrumKitParams", " {"),
            concat!("impl", " DrumKitParams"),
            concat!("struct", " DrumVoice", " {"),
            concat!("fn", " render_frame("),
            concat!("fn", " observe_input("),
            concat!("const", " HAT_RATIOS"),
            concat!("const", " HAT_PARTIALS"),
            concat!("const", " COMB_TUNING"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 drums.rs 里出现了实现记号 `{needle}` —— 这里只允许投影 + `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::drums::{"),
            "engine 的 drums.rs 必须是 dsp 鼓机与其参数类型的再导出"
        );
    }

    /// 判据（2026-11-18 段 2）：**可选八件**的键位与旋钮走独立名单
    /// 落地——写了就触发、旋钮进配方；可选键位写非有限值与五键同
    /// 规则（整台不识别）；核心五件与可选件同音高时核心件优先。
    #[test]
    fn extra_voices_arm_through_the_separate_name_list() {
        let mut with_extras: Vec<(&str, f32)> = NOTES.to_vec();
        with_extras.extend_from_slice(&[
            ("shaker_note", 82.0),
            ("tom_low_note", 43.0),
            ("crash_note", 49.0),
            ("shaker_decay_s", 0.33),
            ("tom_decay_s", 0.44),
            ("crash_pan", -0.5),
            ("snare_pan", 0.25),
        ]);
        let params = DrumsParams::from_devices(&[kit_device(
            DeviceKind::InternalInstrument,
            false,
            &with_extras,
        )])
        .expect("五键写全 + 可选键位 ⇒ 必须武装");
        let notes = params.notes();
        assert_eq!(notes.voice_for(82), Some(DrumVoice::Shaker));
        assert_eq!(notes.voice_for(43), Some(DrumVoice::TomLow));
        assert_eq!(notes.voice_for(49), Some(DrumVoice::Crash));
        assert_eq!(notes.extra_note_for(DrumVoice::Shaker), Some(82));
        assert_eq!(notes.extra_note_for(DrumVoice::Ride), None);
        let kit = params.kit();
        assert_eq!(kit.shaker.decay_s.to_bits(), 0.33f32.to_bits());
        assert_eq!(kit.tom.decay_s.to_bits(), 0.44f32.to_bits());
        assert_eq!(kit.crash.pan.to_bits(), (-0.5f32).to_bits());
        assert_eq!(kit.snare.pan.to_bits(), 0.25f32.to_bits());
        assert!(consumes_drum_param("shaker_note"));
        assert!(consumes_drum_param("tom_decay_s"));
        assert!(consumes_drum_param("ride_pan"));

        // 可选键位非有限 ⇒ 整台不识别（与五键同规则 5）。
        let mut broken: Vec<(&str, f32)> = NOTES.to_vec();
        broken.push(("ride_note", f32::NAN));
        assert!(
            DrumsParams::from_devices(&[kit_device(
                DeviceKind::InternalInstrument,
                false,
                &broken,
            )])
            .is_none(),
            "可选键位写 NaN 必须整台不识别"
        );

        // 碰撞：可选件与核心件同音高 ⇒ 核心件优先（五键先查）。
        let mut colliding: Vec<(&str, f32)> = NOTES.to_vec();
        colliding.push(("cowbell_note", 38.0));
        let params = DrumsParams::from_devices(&[kit_device(
            DeviceKind::InternalInstrument,
            false,
            &colliding,
        )])
        .expect("五键写全 ⇒ 必须武装");
        assert_eq!(params.notes().voice_for(38), Some(DrumVoice::Snare));
    }
}
