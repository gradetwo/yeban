//! **每轨鼓机音源**的引擎侧接线：把 `TrackV3.devices` 里的一台
//! [`DeviceKind::InternalInstrument`] 设备投影成音频线程可执行的**鼓机**
//! （`yeban_dsp::drums`）。[ARCH-RT-001, ARCH-RT-004, ARCH-DSP-001, ARCH-DET-001]
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
//! 这条映射**不在模型里**：`crates/yeban-model/src/music.rs` 的 `MidiNote` 只有
//! `pitch` / `velocity` / `probability` / `ratchet` / `micro_timing_ticks` / `slide` /
//! `pitch_bend_curve` / `syllable` / `phonemes` —— **没有**鼓件、通道或键位映射字段；
//! `grep -rniE 'drum' crates/yeban-model/src/ schemas/` 命中 **0** 行。
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
//! ### 3.2 已识别的名字（每项列出目标字段）
//!
//! | 目标字段 | 名字 |
//! | :--- | :--- |
//! | `kick_note` | `kick_note` |
//! | `snare_note` | `snare_note` |
//! | `closed_hat_note` | `closed_hat_note` |
//! | `open_hat_note` | `open_hat_note` |
//! | `clap_note` | `clap_note` |
//! | `kit.kick.*` | `kick_tune_hz`、`kick_glide_hz`、`kick_glide_s`、`kick_attack_s`、`kick_decay_s`、`kick_level` |
//! | `kit.snare.*` | `snare_tone_hz`、`snare_tone_ratio`、`snare_tone_decay_s`、`snare_tone_level`、`snare_noise_highpass_hz`、`snare_noise_lowpass_hz`、`snare_noise_decay_s`、`snare_noise_level`、`snare_attack_s`、`snare_level` |
//! | `kit.hihat.*` | `hihat_base_hz`、`hihat_highpass_hz`、`hihat_lowpass_hz`、`hihat_attack_s`、`hihat_closed_decay_s`、`hihat_open_decay_s`、`hihat_level` |
//! | `kit.clap.*` | `clap_highpass_hz`、`clap_lowpass_hz`、`clap_bursts`、`clap_burst_decay_s`、`clap_burst_spacing_s`、`clap_attack_s`、`clap_decay_s`、`clap_level` |
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
//! | needs-3 | **同轨多件音源的裁决**：一台设备链里既有复音合成器参数、又有完整鼓机映射时谁发声 | 今天引擎的裁决是"鼓机优先"（§4.1）；模型若禁止这种组合，本条删除 |
//!
//! ## 8. 判据与注入
//!
//! 单元判据（本文件 `mod tests`）+ 端到端判据（`tests/drums_instrument.rs`）。
//! 注入 → 变红的实测记录写在提交的报告里。
//!
//! ⚠ 源码级判据 `engine_drums_module_has_no_second_drum_implementation` 对**本文件的
//! 注释**也生效：注释里不许出现被禁的实现记号（用 `concat!` 拼记号，理由与
//! `crate::insert` 的三条同款判据相同）。

use yeban_model::{DeviceDefinition, DeviceKind};

// 鼓机的**唯一实现**住在 `yeban-dsp`（`crates/yeban-dsp/src/drums/mod.rs`）。
// 本模块只转发：器件 + 它的四个音色参数类型 + 音色身份 + 一次触发请求。
// 判据（`tests/drums_instrument.rs` 的 D2）按这些名字断言"引擎的鼓机**就是** dsp 的鼓机"
// （类型同一性）。
pub use yeban_dsp::drums::{
    ClapParams, DRUM_SLOTS, DrumHit, DrumKitParams, DrumMachine, DrumVoice, HiHatParams,
    KickParams, SnareParams,
};

/// 一台鼓机的**键位映射**：五个鼓件各自的 MIDI 音高（`0..=127`）。
///
/// 它**必须**由工程逐件写出（模块文档 §3.1）：引擎不发明默认键位。
/// 一个音高只触发一个鼓件，碰撞时的裁决见模块文档 §4.2。
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
}

impl DrumNoteMap {
    /// 逐件给出音高（构造期；值域 `0..=127` 由 [`DrumsParams::from_devices`] 保证）。
    #[must_use]
    pub const fn new(kick: u8, snare: u8, closed_hat: u8, open_hat: u8, clap: u8) -> Self {
        Self {
            kick,
            snare,
            closed_hat,
            open_hat,
            clap,
        }
    }

    /// 该音高触发哪个鼓件（`None` = 本映射里没有它 ⇒ 不触发）。
    ///
    /// 顺序 = [`DrumVoice`] 的判别值顺序（模块文档 §4.2 的碰撞裁决）。
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
            None
        }
    }

    /// 该鼓件的音高（[`Self::voice_for`] 的逆，碰撞时可能不成立 —— 见模块文档 §4.2）。
    #[must_use]
    pub const fn note_for(&self, voice: DrumVoice) -> u8 {
        match voice {
            DrumVoice::Kick => self.kick,
            DrumVoice::Snare => self.snare,
            DrumVoice::ClosedHat => self.closed_hat,
            DrumVoice::OpenHat => self.open_hat,
            DrumVoice::Clap => self.clap,
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
        for param in &device.params {
            let name = param.name.to_ascii_lowercase();
            let value = param.value;
            if let Some(index) = note_slot(&name) {
                notes[index] = note_from_f32(value);
                continue;
            }
            apply_kit_knob(&mut kit, &name, value);
        }
        let notes = DrumNoteMap::new(notes[0]?, notes[1]?, notes[2]?, notes[3]?, notes[4]?);
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

/// 五个键位在投影中间态里的下标（顺序 = [`DrumVoice::ALL`]）。
const NOTE_SLOTS: usize = 5;

/// 参数名 → 键位下标（`None` = 不是键位名）。
#[must_use]
fn note_slot(name: &str) -> Option<usize> {
    match name {
        "kick_note" => Some(0),
        "snare_note" => Some(1),
        "closed_hat_note" => Some(2),
        "open_hat_note" => Some(3),
        "clap_note" => Some(4),
        _ => None,
    }
}

/// 键位取值：非有限 ⇒ `None`（规则 5：整台设备不识别）；否则四舍五入并钳到 `0..=127`。
#[must_use]
fn note_from_f32(value: f32) -> Option<u8> {
    if !value.is_finite() {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let note = value.round().clamp(0.0, 127.0) as u8;
    Some(note)
}

/// `clap_bursts` 的 `f32 → u32`：非有限 ⇒ 器件默认值（规则 5）。
///
/// 合法域（`1..=8`）由 [`ClapParams::sanitised`] 在收尾时钳制 —— 这里只做类型转换，
/// 因此**不**在这里重复一份域。
#[must_use]
fn bursts_from_f32(value: f32) -> u32 {
    if !value.is_finite() {
        return ClapParams::DEFAULT.bursts;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bursts = value.round().clamp(0.0, f32::from(u16::MAX)) as u32;
    bursts
}

/// 把一条**音色旋钮**写进投影；返回它是否被认下（名字已转小写）。
///
/// 先赋原值、由 [`DrumKitParams::sanitised`] 在收尾时统一钳制：
/// 这样"值域"只有**一个**事实源（器件自己），本模块不持有第二份域表。
fn apply_kit_knob(kit: &mut DrumKitParams, name: &str, value: f32) -> bool {
    match name {
        // --- 底鼓 ---
        "kick_tune_hz" => kit.kick.tune_hz = value,
        "kick_glide_hz" => kit.kick.glide_hz = value,
        "kick_glide_s" => kit.kick.glide_s = value,
        "kick_attack_s" => kit.kick.attack_s = value,
        "kick_decay_s" => kit.kick.decay_s = value,
        "kick_level" => kit.kick.level = value,
        // --- 军鼓 ---
        "snare_tone_hz" => kit.snare.tone_hz = value,
        "snare_tone_ratio" => kit.snare.tone_ratio = value,
        "snare_tone_decay_s" => kit.snare.tone_decay_s = value,
        "snare_tone_level" => kit.snare.tone_level = value,
        "snare_noise_highpass_hz" => kit.snare.noise_highpass_hz = value,
        "snare_noise_lowpass_hz" => kit.snare.noise_lowpass_hz = value,
        "snare_noise_decay_s" => kit.snare.noise_decay_s = value,
        "snare_noise_level" => kit.snare.noise_level = value,
        "snare_attack_s" => kit.snare.attack_s = value,
        "snare_level" => kit.snare.level = value,
        // --- 踩镲（闭/开共用一套配方）---
        "hihat_base_hz" => kit.hihat.base_hz = value,
        "hihat_highpass_hz" => kit.hihat.highpass_hz = value,
        "hihat_lowpass_hz" => kit.hihat.lowpass_hz = value,
        "hihat_attack_s" => kit.hihat.attack_s = value,
        "hihat_closed_decay_s" => kit.hihat.closed_decay_s = value,
        "hihat_open_decay_s" => kit.hihat.open_decay_s = value,
        "hihat_level" => kit.hihat.level = value,
        // --- 拍手 ---
        "clap_highpass_hz" => kit.clap.highpass_hz = value,
        "clap_lowpass_hz" => kit.clap.lowpass_hz = value,
        "clap_bursts" => kit.clap.bursts = bursts_from_f32(value),
        "clap_burst_decay_s" => kit.clap.burst_decay_s = value,
        "clap_burst_spacing_s" => kit.clap.burst_spacing_s = value,
        "clap_attack_s" => kit.clap.attack_s = value,
        "clap_decay_s" => kit.clap.decay_s = value,
        "clap_level" => kit.clap.level = value,
        // --- 总线 ---
        "master_level" => kit.master_level = value,
        _ => return false,
    }
    true
}

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
        // `as_array` 与 `note_for` 必须给出同一组数（逐件）。
        for (index, voice) in DrumVoice::ALL.iter().enumerate() {
            assert_eq!(notes.as_array()[index], notes.note_for(*voice));
        }
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
}
