//! 双振荡器减法复音合成器（器件化）[ARCH-RT-001, ARCH-RT-004, ARCH-DSP-001, ARCH-DET-001]。
//!
//! ## 0. 来源与归属：本模块**不是**第二份合成器实现
//!
//! 本模块的声部层逐条来自 `crates/yeban-engine/src/synth.rs`（上移前 **1449** 行，
//! 本机实测 `wc -l`）—— 也就是引擎**唯一**的那台复音合成器：
//! 定长声部池、整数相位累加、mip 波表线性插值、声部级四极梯形低通、ADSR，
//! 以及 [ARCH-RT-004] 的确定性窃取与 3 ms 指数淡出。裁决依据三处规范原文：
//!
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:111`
//!   —— "减法合成器 ➔ `crates/yeban-dsp/src/polysynth.rs`（双振荡器减法复音合成器）"；
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:347`
//!   —— 参考工程 A 含 "16 个 PolySynth 减法合成器"；
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:445`
//!   —— "纯 Rust 建模合成器 (PolySynth, GS-1, 808/909)"。
//!
//! 另外两处约束本模块形状的原文：
//!
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:383`（[ARCH-RT-004]）：
//!   "激活确定性声部窃取算法：优先窃取处于 Release 阶段尾部、振幅能量最低（<-60dBFS）
//!   或最早被触发的声音；"（窃取淡出时长见下方的**刻意偏离**）；
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:426`（[ARCH-DSP-001]）：
//!   "分配器对被偷取声部强制施加 5.0ms 升余弦窗 … 或五次多项式平滑窗快速淡出"。
//!
//! ## 1. 引擎侧只剩 `pub use`（"全仓不许有两份合成器实现"）
//!
//! `crates/yeban-engine/src/synth.rs` 现在只保留**调度**（`NoteSchedule` /
//! `ScheduledNote` / tick → 样本的构造期换算 / 轨道槽与游标）并把本器件再导出。
//! 与 `yeban-dsp::limiter` 上移的先例同形（`crates/yeban-engine/src/mixer.rs`
//! 只 `pub use` 限制器）。逐位一致的强度因此是构造性的：逐样本算式、运算顺序、
//! 状态初值都与上移前**同一串字符**。
//!
//! ## 2. 双振荡器：两个独立的相位累加器
//!
//! 规范（`:111`）要求**双振荡器**。上移前的引擎只有**一个**波表振荡器
//! （`Wavetable::from_recipe(HOLLOW)`，逐声部一份相位）。本模块把它扩成两条
//! 独立振荡器支路：各自一张波表（[`PolySynthTables`] 里的下标）、各自电平、
//! 各自以音分表示的失谐、各自 `u32` 相位累加器。
//!
//! ⚠ **默认只有一个在发声**：`PolySynthParams::default()` 的 `osc1` 是
//! `table 0 / level 1.0 / detune 0`，`osc2` 是 `level 0.0`（关）。
//! 关掉时第二条（`osc2`）支路的代码**一帧也不执行**（不是"乘 0 求和"）。
//! 这条规则首先是**性能与结构**上的保证：只开一条时，逐样本开销与上移前的
//! 单振荡器**完全相同**。它**不是**数值上的必要条件 —— 实测（报告里的注入 I6）：
//! 把 `if osc2_on` 去掉、恒做 `first + second`（`second = interp2 × 0.0 = +0.0`）
//! 之后，本 crate 的位模式判据与引擎的 5 个指纹**都没有改变**；唯一会变的是
//! `first` 恰为 `-0.0` 的情形（`-0.0 + 0.0 = +0.0`），本夹具没有构造出那个位模式。
//! ⇒ "引擎走默认参数时的输出与上移前逐位相同"由**整条算式的字面一致**保证（见 §0），
//! 不是靠这个分支。
//!
//! ## 3. 实时侧纪律（[ARCH-RT-001] / `MUST-GATE-001`）
//!
//! [`PolySynth::render`] 与 [`PolySynth::note_on`] 是实时路径：
//!
//! | 禁令 | 本模块的做法 |
//! | :--- | :--- |
//! | 堆分配 | 声部池是 `[PolyVoice; VOICES]`（定长数组）；波表在 `new()` 之前建好 |
//! | 堆释放 | 声部状态是 `Copy` 值类型；没有 `Box`/`Vec`/`Rc` |
//! | 锁 | 器件自身没有任何锁，也不调用任何会加锁的东西 |
//! | 阻塞 I/O | 器件没有 I/O，也不打印 |
//! | 日志 | `render`/`note_on` 里没有 `println!`/`eprintln!`/`format!` |
//!
//! 运行期由 `crates/yeban-dsp/tests/polysynth_rt_zero_alloc.rs`（计数型全局分配器）
//! 钉住：10 000 个量子 × 每个量子 128 帧的窗口内 `allocations == 0`。
//!
//! ⚠ 波表**不**在 `set_params` / `note_on` 里重建：`Wavetable::from_recipe` 会分配
//! （9 级 × 2048 点 ≈ 72 KiB / 张）。因此波表库由调用方在**打开设备之前**
//! 建好（[`PolySynthTables::from_recipes`]），实时侧只读它。
//!
//! ## 4. 确定性（[ARCH-DET-001]）
//!
//! 1. **无随机源**：本模块不持有 RNG；波表内容在构造期由配方唯一确定；
//! 2. **状态少且只前向依赖**：每个声部的状态是 `u32` 相位、`f32` 包络电平、
//!    梯形滤波器的四个状态变量与几个整数；逐样本只读"上一帧的自己" ⇒
//!    与块切分**构造性无关**（判据：`determinism.rs` 的多种切分逐位比对）；
//! 3. **相位推进用整数**：`phase.wrapping_add(inc)` 无累积误差、不受 FTZ/DAZ 影响。
//!    浮点相位累加会在长音符上漂移，且漂移量取决于累加次数 —— 那正是非确定性的来源；
//! 4. **窃取选择是全序**：`(优先级, start_sample, 下标)` 三级比较，任何平台同解。
//!
//! ## 5. 与规范的一处**刻意偏离**（不改，登记）
//!
//! [ARCH-DSP-001]（`…AND_SYSTEM_DESIGN.md:426`）写的是 **5.0 ms 升余弦窗**；
//! 本模块用的是 [ARCH-RT-004] 的 **3 ms 指数淡出**（[`STEAL_RELEASE_SECONDS`]）。
//! 这不是本次上移引入的：上移前就是 3 ms，且 `crates/yeban-engine/tests/steal_fade.rs`
//! 的 S4 把 `144 帧 = 3 ms @48 kHz` 写成了**既有期望值**。
//! 本票遵守"既有判据不许弱化、期望值一个字不改" ⇒ 保持 3 ms，并把
//! 5 ms 升余弦登记为**未做**（引擎侧的 `light` 门禁与 CI 都读不到这条差异）。

use crate::MIN_SAMPLE_RATE;
use crate::envelope::{Adsr, AdsrStage, STEAL_RELEASE_SECONDS};
use crate::filter::LadderFilter;
use crate::math::exp2;
use crate::oscillator::{HOLLOW, WaveRecipe, Wavetable};

/// 一个器件实例的**声部池容量**（复音上限），定长数组的长度。
///
/// 引擎侧的轨道槽用它作为"每轨复音数"（上移前是 `synth.rs` 的
/// `VOICES_PER_TRACK = 16`，值不变）。规格没有给出复音数；
/// `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:347` 的 "16 个 PolySynth"
/// 数是**实例数**（轨道数），不是每实例声部数 —— 本注释刻意区分这两件事。
pub const VOICES_PER_SLOT: usize = 16;

/// 相位小数的位宽：`u32` 相位里取最高的 12 位作线性插值的小数。
///
/// 12 位是"抖动可忽略、整数运算便宜"的折中：量化误差约 −72 dBFS，
/// 且**完全确定**（没有浮点相位累积）。
const FRAC_BITS: u32 = 12;

/// `1 / 2^FRAC_BITS`，可精确表示 ⇒ 该乘法是 IEEE 精确类。
const FRAC_SCALE: f32 = 1.0 / (1u32 << FRAC_BITS) as f32;

/// 一周期对应的定点相位数（`2³²`）。
const PHASE_ONE: f64 = 4_294_967_296.0;

/// 失谐的上界（音分）：±4 个八度。超出即钳制，非有限值按 0 取。
const MAX_DETUNE_CENTS: f32 = 4_800.0;

/// 单条振荡器支路的设置（[`PolySynthParams`] 的一半）。
///
/// 全部取值在构造期钳制 ⇒ 逐样本路径上不出现 `NaN`/`inf`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscSettings {
    table: usize,
    level: f32,
    detune_cents: f32,
}

impl OscSettings {
    /// 构造：`table` 是 [`PolySynthTables`] 里的波表下标（取值时按库长度钳制），
    /// `level` 钳到 `0.0..=1.0`（**非有限值 ⇒ `1.0`**，即该参数的默认值；
    /// 求"关掉"请显式传 `0.0` 或用 [`OscSettings::off`]），
    /// `detune_cents` 钳到 `±MAX_DETUNE_CENTS`（非有限值 ⇒ `0.0`）。
    ///
    /// 手写钳制（而不是 `f32::clamp`）是为了让它是 `const fn` ——
    /// [`PolySynthParams::new`] 要用它在常量上下文里建默认参数。
    /// "非有限值退回该参数的默认值"与 `ToneParams::new`、`Adsr::set_params`
    /// 的口径一致：非法输入绝不把 `NaN` 灌进逐样本路径。
    #[must_use]
    pub const fn new(table: usize, level: f32, detune_cents: f32) -> Self {
        let level = if level.is_finite() {
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
        let detune_cents = if detune_cents.is_finite() {
            if detune_cents < -MAX_DETUNE_CENTS {
                -MAX_DETUNE_CENTS
            } else if detune_cents > MAX_DETUNE_CENTS {
                MAX_DETUNE_CENTS
            } else {
                detune_cents
            }
        } else {
            0.0
        };
        Self {
            table,
            level,
            detune_cents,
        }
    }

    /// **关掉**这条支路（`level = 0`）。
    ///
    /// 关掉的支路在 [`PolySynth::render`] 里**一帧也不执行** ——
    /// 既不算波形也不推进相位。这条规则是**零开销**的保证（只开一条振荡器时
    /// 逐样本不碰第二条）；它**不是**数值上的必要条件，见模块文档 §2 的注入 I6。
    #[must_use]
    pub const fn off() -> Self {
        Self {
            table: 0,
            level: 0.0,
            detune_cents: 0.0,
        }
    }

    /// 波表下标（按库长度钳制后才生效）。
    #[must_use]
    pub const fn table(&self) -> usize {
        self.table
    }

    /// 电平（`0.0` = 关）。
    #[must_use]
    pub const fn level(&self) -> f32 {
        self.level
    }

    /// 失谐（音分）。
    #[must_use]
    pub const fn detune_cents(&self) -> f32 {
        self.detune_cents
    }

    /// 这条支路是否参与发声（`level != 0`）。
    #[must_use]
    pub const fn is_on(&self) -> bool {
        self.level != 0.0
    }
}

impl Default for OscSettings {
    /// 默认：`table 0`、满电平、不失谐。
    fn default() -> Self {
        Self::new(0, 1.0, 0.0)
    }
}

/// 器件的**静态参数**：双振荡器 + 声部低通 + 振幅包络。
///
/// 参数只在快照边界或构造期写入，逐样本路径只读它。
/// 默认值刻意等于上移前 `synth.rs` 的硬编码音色
/// （`HOLLOW` 波表、旁通滤波器、5 ms 起音 / 80 ms 衰减 / 0.7 延音 / 50 ms 释放）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolySynthParams {
    osc1: OscSettings,
    osc2: OscSettings,
    cutoff_hz: f32,
    resonance: f32,
    drive: f32,
    filter_bypass: bool,
    attack_s: f32,
    decay_s: f32,
    sustain: f32,
    release_s: f32,
}

impl PolySynthParams {
    /// 默认参数：单振荡器（`osc2` 关）、滤波器**旁通**、最小可用包络。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            osc1: OscSettings::new(0, 1.0, 0.0),
            osc2: OscSettings::off(),
            cutoff_hz: TONE_BYPASS_CUTOFF_HZ,
            resonance: 0.0,
            drive: 0.0,
            filter_bypass: true,
            attack_s: DEFAULT_ATTACK_SECONDS,
            decay_s: DEFAULT_DECAY_SECONDS,
            sustain: DEFAULT_SUSTAIN,
            release_s: DEFAULT_RELEASE_SECONDS,
        }
    }

    /// 设置两条振荡器支路。
    #[must_use]
    pub const fn with_oscillators(mut self, osc1: OscSettings, osc2: OscSettings) -> Self {
        self.osc1 = osc1;
        self.osc2 = osc2;
        self
    }

    /// 设置声部低通（三个旋钮 + 旁通）。
    ///
    /// 钳制口径与引擎侧临时形状 `ToneParams::new` 逐条一致：
    /// 非有限截止频率 ⇒ `12 kHz`；共振与驱动 ⇒ `0.0..=1.0`；非有限 ⇒ `0.0`。
    /// `bypass` 为真时**渲染路径一次也不调用**滤波器（逐位恒等，不是"系数取透明"）。
    #[must_use]
    pub fn with_filter(mut self, cutoff_hz: f32, resonance: f32, drive: f32, bypass: bool) -> Self {
        self.cutoff_hz = if cutoff_hz.is_finite() {
            cutoff_hz
        } else {
            DEFAULT_CUTOFF_HZ
        };
        self.resonance = if resonance.is_finite() {
            resonance.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.drive = if drive.is_finite() {
            drive.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.filter_bypass = bypass;
        self
    }

    /// 设置振幅包络（秒 / 秒 / 0..1 / 秒）。
    ///
    /// 非有限值或负值退回该参数的默认值 —— 包络系数含 `exp`，
    /// 让非法输入穿过去会产生 `NaN` 包络，那是整条链路的静默死亡。
    #[must_use]
    pub fn with_envelope(
        mut self,
        attack_s: f32,
        decay_s: f32,
        sustain: f32,
        release_s: f32,
    ) -> Self {
        self.attack_s = positive_or(attack_s, DEFAULT_ATTACK_SECONDS);
        self.decay_s = positive_or(decay_s, DEFAULT_DECAY_SECONDS);
        self.sustain = if sustain.is_finite() {
            sustain.clamp(0.0, 1.0)
        } else {
            DEFAULT_SUSTAIN
        };
        self.release_s = positive_or(release_s, DEFAULT_RELEASE_SECONDS);
        self
    }

    /// 振荡器 1 的设置。
    #[must_use]
    pub const fn osc1(&self) -> OscSettings {
        self.osc1
    }

    /// 振荡器 2 的设置。
    #[must_use]
    pub const fn osc2(&self) -> OscSettings {
        self.osc2
    }

    /// 低通截止频率 (Hz)。
    #[must_use]
    pub const fn cutoff_hz(&self) -> f32 {
        self.cutoff_hz
    }

    /// 低通共振（0..1 旋钮值）。
    #[must_use]
    pub const fn resonance(&self) -> f32 {
        self.resonance
    }

    /// 低通驱动（0..1 旋钮值）。
    #[must_use]
    pub const fn drive(&self) -> f32 {
        self.drive
    }

    /// 低通是否旁通。
    #[must_use]
    pub const fn filter_bypass(&self) -> bool {
        self.filter_bypass
    }

    /// 起振时间（秒）。
    #[must_use]
    pub const fn attack_s(&self) -> f32 {
        self.attack_s
    }

    /// 衰减时间（秒）。
    #[must_use]
    pub const fn decay_s(&self) -> f32 {
        self.decay_s
    }

    /// 延音电平（0..1）。
    #[must_use]
    pub const fn sustain(&self) -> f32 {
        self.sustain
    }

    /// 释放时间（秒）。
    #[must_use]
    pub const fn release_s(&self) -> f32 {
        self.release_s
    }
}

impl Default for PolySynthParams {
    fn default() -> Self {
        Self::new()
    }
}

/// 滤波旁通时的截止频率（Hz）。它不参与声音，只让字段在任何状态下都有确定值
/// （便于 `PartialEq` 与诊断）。与引擎侧临时形状 `ToneParams` 的取值一致。
const TONE_BYPASS_CUTOFF_HZ: f32 = 20_000.0;

/// 非有限截止频率的退回值（Hz）。
const DEFAULT_CUTOFF_HZ: f32 = 12_000.0;

/// 默认起振（秒）：5 ms 不咔哒。
const DEFAULT_ATTACK_SECONDS: f32 = 0.005;
/// 默认衰减（秒）：80 ms 衰减到延音。
const DEFAULT_DECAY_SECONDS: f32 = 0.08;
/// 默认延音电平。
const DEFAULT_SUSTAIN: f32 = 0.7;
/// 默认释放（秒）：50 ms 让"音符终点"在判据里可界。
const DEFAULT_RELEASE_SECONDS: f32 = 0.05;

/// 非有限或非正值退回默认值。
#[must_use]
fn positive_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        fallback
    }
}

/// **波形库**：`PolySynth` 逐样本只读的一组 mip 波表。
///
/// # 为什么波表不在器件里
///
/// 一张 [`Wavetable`] 是 9 级 × 2048 点 `f32` ≈ **72 KiB**。引擎有 16 个轨道槽，
/// 若每个槽各建一份，代价是 16 倍（且 `Wavetable::from_recipe` 会**分配**）。
/// 更硬的理由是实时性：引擎在**音频线程**上应用快照，而波表构建会分配 ——
/// 因此波表必须在打开设备之前建好，运行期只读。库由调用方持有，
/// 器件按 [`OscSettings::table`] 的下标选取。
#[derive(Clone, Debug)]
pub struct PolySynthTables {
    tables: Vec<Wavetable>,
}

impl PolySynthTables {
    /// 按配方列表建库（**构造期**；会分配，绝不在实时路径上调用）。
    ///
    /// 空切片按内置 [`HOLLOW`] 建**一张**表：本器件在"零张表"上无法发声，
    /// 而"悄悄静音"比"按文档退回内置配方"更难排查。这条规则是显式的、确定性的。
    #[must_use]
    pub fn from_recipes(recipes: &[WaveRecipe]) -> Self {
        let tables: Vec<Wavetable> = if recipes.is_empty() {
            vec![Wavetable::from_recipe(HOLLOW)]
        } else {
            recipes
                .iter()
                .map(|recipe| Wavetable::from_recipe(recipe))
                .collect()
        };
        Self { tables }
    }

    /// 库里的波表张数（恒 ≥ 1）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// 库是否为空（恒为 `false`；存在只为满足 `len`/`is_empty` 配对约定）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// 某张表的 mip 级数（诊断用）。
    #[must_use]
    pub fn level_count(&self, table: usize) -> usize {
        self.wavetable(table).level_count()
    }

    /// 某个回放频率在某张表上应选的 mip 级号（越界下标钳到最后一张表）。
    #[must_use]
    pub fn level_for(&self, table: usize, freq_hz: f32, sample_rate: f32) -> usize {
        self.wavetable(table).level_for(freq_hz, sample_rate)
    }

    /// 只读访问某张表某一级的原始采样（离线分析与判据用）。
    #[must_use]
    pub fn samples(&self, table: usize, level: usize) -> &[f32] {
        self.wavetable(table).level_samples(level)
    }

    /// 越界下标钳制（`tables` 恒非空 ⇒ 不会 panic）。
    #[must_use]
    fn wavetable(&self, table: usize) -> &Wavetable {
        &self.tables[table.min(self.tables.len().saturating_sub(1))]
    }
}

/// 一次**音符触发请求**（构造期或控制侧组装；实时侧只读）。
///
/// `freq_hz` 与 `gain` 由调用方给出（引擎侧的 tick → 样本换算与增益投影在
/// `crate::snapshot` 完成）；相位定标是**合成器的不变量**，因此
/// `phase_inc` 由本模块的 [`phase_increment`] 从 `freq_hz`/`sample_rate` 推出，
/// 不让每个调用点各自记住 `2³²`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteEvent {
    start_sample: u64,
    end_sample: u64,
    freq_hz: f32,
    gain: f32,
}

impl NoteEvent {
    /// 组装一次触发请求。非有限 `gain` ⇒ `0.0`（绝不把 `NaN` 灌进声部）。
    #[must_use]
    pub fn new(start_sample: u64, end_sample: u64, freq_hz: f32, gain: f32) -> Self {
        Self {
            start_sample,
            end_sample,
            freq_hz,
            gain: if gain.is_finite() { gain } else { 0.0 },
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

    /// 回放频率 (Hz)。
    #[must_use]
    pub const fn freq_hz(&self) -> f32 {
        self.freq_hz
    }

    /// 逐样本增益。
    #[must_use]
    pub const fn gain(&self) -> f32 {
        self.gain
    }
}

/// 相位增量：`freq_hz / sample_rate` 的一周期 = `2³²` 定标。
///
/// - `inc == 0` 会让声部永久停在相位 0（静音）⇒ 钳到下界 1；
/// - 上界钳到 `u32::MAX`（频率 ≥ `sample_rate` 时回绕成"每样本整周期"）；
/// - 非有限 `freq_hz` ⇒ 0（再钳到 1）；`sample_rate` 非法 ⇒ 按 48 kHz 算。
///
/// 一次 `f64` 乘除 + `round`：这一步**不在**逐样本路径上（每次触发一次），
/// `round` 本身是 IEEE 精确类（`ADR-0001` D32 §1）。
#[must_use]
pub fn phase_increment(freq_hz: f32, sample_rate: f32) -> u32 {
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
    let increment = (freq_hz / sample_rate * PHASE_ONE).round();
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

/// 窃取淡出的帧数：`STEAL_RELEASE_SECONDS`（3 ms）× 采样率。
///
/// 上限 0.5 s（采样率异常大时也不会把声部卡死几秒），下限 1 帧
/// （0 帧就是硬窃取 —— 那正是要消灭的行为，但它必须可注入 ⇒
/// [`PolySynth::set_steal_fade_frames`] 可以把它覆盖成 0）。
#[must_use]
pub fn steal_fade_frames_for(sample_rate: f32) -> u32 {
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

/// 一条振荡器支路的逐声部状态。
#[derive(Clone, Copy, Debug)]
struct OscState {
    /// 整数相位（一周期 = `2³²`）。
    phase: u32,
    /// 每样本相位增量。
    inc: u32,
    /// 波表下标。
    table: usize,
    /// mip 级（触发时由频率选出）。
    level: usize,
    /// 逐样本增益（= `OscSettings::level`）。
    gain: f32,
}

impl OscState {
    const IDLE: Self = Self {
        phase: 0,
        inc: 1,
        table: 0,
        level: 0,
        gain: 0.0,
    };
}

/// 一个**已挂起**的新音符：窃取淡出走完的那一帧才启用。
#[derive(Clone, Copy, Debug)]
struct PendingNote {
    start_sample: u64,
    end_sample: u64,
    gain: f32,
    osc1: OscState,
    osc2: OscState,
}

/// 一个声部的全部状态。
#[derive(Clone, Copy, Debug)]
struct PolyVoice {
    active: bool,
    osc1: OscState,
    osc2: OscState,
    /// 逐样本增益（力度 × 轨道音量，构造期算好）。
    gain: f32,
    /// 起点（绝对样本位置）。
    start_sample: u64,
    /// 终点（绝对样本位置）。
    end_sample: u64,
    env: Adsr,
    /// 声部级四极低通（旁通时**不**被调用）。
    filter: LadderFilter,
    /// 本声部是否处于"窃取淡出"窗口内（>0 表示还剩多少帧）。
    fade_remaining: u32,
    /// 是否有**已挂起的新音符**（淡出走完立刻起音）。
    pending: Option<PendingNote>,
}

impl PolyVoice {
    const IDLE: Self = Self {
        active: false,
        osc1: OscState::IDLE,
        osc2: OscState::IDLE,
        gain: 0.0,
        start_sample: 0,
        end_sample: 0,
        env: Adsr::new(),
        filter: LadderFilter::new(),
        fade_remaining: 0,
        pending: None,
    };

    /// 推进两条在用的振荡器支路的相位。
    #[inline]
    fn advance(&mut self, osc2_on: bool) {
        self.osc1.phase = self.osc1.phase.wrapping_add(self.osc1.inc);
        if osc2_on {
            self.osc2.phase = self.osc2.phase.wrapping_add(self.osc2.inc);
        }
    }
}

/// 窃取优先级：`0` = 正在释放（或已低于 −60 dBFS），`1` = 仍在持续发声。
///
/// 数字小者优先被窃取。−60 dBFS ≈ `0.001` 是 [ARCH-RT-004] 原文给的阈值。
#[must_use]
fn steal_priority(voice: &PolyVoice) -> u8 {
    let releasing = matches!(voice.env.stage(), AdsrStage::Release);
    if releasing || voice.env.value() < 0.001 {
        0
    } else {
        1
    }
}

/// 按 `u32` 定点相位在某一级波表上做线性插值。
///
/// `phase < 2³²` ⇒ `index < len`：整数乘 + 右移代替除法，且对任意 `len`
/// （不要求 2 的幂）都正确。末端回绕用一次比较代替取模（整数，精确）。
#[inline]
#[must_use]
fn interpolate(samples: &[f32], phase: u32) -> f32 {
    let len = samples.len();
    let scaled = u64::from(phase) * len as u64;
    let index = (scaled >> 32) as usize;
    let first = samples[index];
    let next = index + 1;
    let second = samples[if next == len { 0 } else { next }];
    let fraction = ((scaled & 0xFFFF_FFFF) >> (32 - FRAC_BITS)) as f32 * FRAC_SCALE;
    first + (second - first) * fraction
}

/// 本帧的振荡器和：`osc2` 关时**只算 `osc1`**（零开销；数值上的强度见模块文档 §2）。
#[inline]
#[must_use]
fn oscillator_sample(tables: &PolySynthTables, voice: &PolyVoice, osc2_on: bool) -> f32 {
    let first = interpolate(
        tables.samples(voice.osc1.table, voice.osc1.level),
        voice.osc1.phase,
    ) * voice.osc1.gain;
    if osc2_on {
        let second = interpolate(
            tables.samples(voice.osc2.table, voice.osc2.level),
            voice.osc2.phase,
        ) * voice.osc2.gain;
        first + second
    } else {
        first
    }
}

/// 把一条振荡器设置解析成逐声部状态（触发时一次）。
///
/// `base_inc` 是不失谐时的相位增量；失谐为 0 时**直接复用**它，
/// 让默认参数的输出与上移前逐位相同（`exp2(0)` 不参与）。
#[must_use]
fn resolve_osc(
    settings: OscSettings,
    freq_hz: f32,
    base_inc: u32,
    sample_rate: f32,
    tables: &PolySynthTables,
) -> OscState {
    let table = settings.table.min(tables.len().saturating_sub(1));
    let (inc, level_freq) = if settings.detune_cents == 0.0 {
        (base_inc, freq_hz)
    } else {
        let ratio = exp2(settings.detune_cents / 1_200.0);
        let detuned = if ratio.is_finite() && ratio > 0.0 {
            freq_hz * ratio
        } else {
            freq_hz
        };
        (phase_increment(detuned, sample_rate), detuned)
    };
    OscState {
        phase: 0,
        inc,
        table,
        level: tables.level_for(table, level_freq, sample_rate),
        gain: settings.level,
    }
}

/// 双振荡器减法复音合成器：**定容预分配声部池** + 确定性窃取 + 3 ms 指数淡出。
///
/// 容量 `VOICES` 是编译期常量（默认 [`VOICES_PER_SLOT`]）⇒ 声部池是定长数组，
/// `render` 与 `note_on` 均**零分配、零锁、零 I/O**（模块文档 §3）。
/// 形状参照 `crates/yeban-sfz/src/voice_pool.rs` 的"定容池 + 确定性窃取 + 3 ms
/// 指数淡出"，但**不依赖** `yeban-sfz`（那会给 `yeban-dsp` 加一条内部依赖）。
#[derive(Clone, Copy, Debug)]
pub struct PolySynth<const VOICES: usize = VOICES_PER_SLOT> {
    voices: [PolyVoice; VOICES],
    params: PolySynthParams,
    sample_rate: f32,
    /// 滤波器模板：系数在参数写入时算好（含 `tan`），触发时按值拷进声部
    /// ⇒ 逐样本路径不含超越函数。
    filter_template: LadderFilter,
    /// 窃取淡出帧数（默认 3 ms ⇒ @48k = 144 帧）。判据可覆盖成 0（硬窃取）。
    steal_fade_frames: u32,
    voice_steals: u64,
    notes_triggered: u64,
}

impl<const VOICES: usize> PolySynth<VOICES> {
    /// 构造：定长生部池 + 默认参数。**允许分配**（这一步在打开设备之前；
    /// 本构造自身不分配，波表由调用方另行构建）。
    ///
    /// `sample_rate` 先按传入值武装；真正的采样率由 [`Self::set_sample_rate`] 校准。
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sanitise_sample_rate(sample_rate);
        let params = PolySynthParams::new();
        Self {
            voices: [PolyVoice::IDLE; VOICES],
            params,
            sample_rate,
            filter_template: configure_filter(&params, sample_rate),
            steal_fade_frames: steal_fade_frames_for(sample_rate),
            voice_steals: 0,
            notes_triggered: 0,
        }
    }

    /// 当前采样率 (Hz)。
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// 声部池容量（复音上限）。
    #[must_use]
    pub const fn voices(&self) -> usize {
        VOICES
    }

    /// 当前活跃声部数。
    #[must_use]
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|voice| voice.active).count()
    }

    /// 池满时的**软窃取**次数：每次窃取都给被终止声部套上 3 ms 淡出 [ARCH-RT-004]。
    /// 把淡出帧数设为 0（[`Self::set_steal_fade_frames`]）即回到"硬窃取"，
    /// 但计数口径不变。
    #[must_use]
    pub const fn voice_steals(&self) -> u64 {
        self.voice_steals
    }

    /// 累计触发过的音符数。
    #[must_use]
    pub const fn notes_triggered(&self) -> u64 {
        self.notes_triggered
    }

    /// 当前窃取淡出帧数（默认 `3 ms × 采样率`）。
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

    /// 当前参数。
    #[must_use]
    pub const fn params(&self) -> PolySynthParams {
        self.params
    }

    /// 校准采样率（快照边界；采样率变化时重算淡出帧数与滤波器系数）。
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        let sample_rate = sanitise_sample_rate(sample_rate);
        if sample_rate == self.sample_rate {
            return;
        }
        self.sample_rate = sample_rate;
        self.steal_fade_frames = steal_fade_frames_for(sample_rate);
        self.filter_template = configure_filter(&self.params, sample_rate);
    }

    /// 写入参数（快照边界/构造期）。`tables` 只用于把波表下标钳进库长度。
    ///
    /// 滤波器系数在此处算好（含 `tan`，超越函数类）⇒ 逐样本路径不含 `tan`。
    /// **不重触发、不重置声部**：参数变化不得切断在鸣的音符。
    pub fn set_params(&mut self, params: PolySynthParams, tables: &PolySynthTables) {
        let params = PolySynthParams {
            osc1: clamp_table(params.osc1, tables),
            osc2: clamp_table(params.osc2, tables),
            ..params
        };
        self.params = params;
        self.filter_template = configure_filter(&params, self.sample_rate);
    }

    /// 释放全部声部（走带 seek 的接入点）。不重算参数、不改采样率。
    pub fn reset(&mut self) {
        self.voices = [PolyVoice::IDLE; VOICES];
    }

    /// 回收"已过终点且包络已静音"的声部（快照边界的游标校正一并做）。
    ///
    /// 仍在释放段的声部**保留** —— 快照切换不得切断在鸣的音符。
    pub fn retire_finished(&mut self, position: u64) {
        for voice in &mut self.voices {
            if voice.active && voice.end_sample <= position && !voice.env.is_active() {
                *voice = PolyVoice::IDLE;
            }
        }
    }

    /// 触发一个音符：占用空槽；池满时按 [ARCH-RT-004] **软窃取**。
    ///
    /// ## 窃取算法的三条口径（都是确定性的）
    ///
    /// 1. **优先级**（[ARCH-RT-004] 原文："优先窃取处于 Release 阶段尾部、振幅能量
    ///    最低（< −60 dBFS）或最早被触发的声音"）：包络已进入 `Release` 段**或**
    ///    当前电平 < −60 dBFS（`0.001`）的声部优先；同档内取 `start_sample`
    ///    **最小**者（最早触发）；仍然并列 ⇒ 取**下标最小**者。
    ///    三级比较合起来是全序 ⇒ 任何平台/编译器下选出同一个声部。
    /// 2. **淡出**：被窃取声部进入 `fade_remaining = steal_fade_frames` 帧的指数淡出
    ///    （默认 3 ms），新音符**挂起**在同一槽位上，淡出走完立刻从 0 起 attack。
    ///    池满时"旧声部淡出"与"新音符起音"因此**永不同时发声** ⇒ 不可能叠加爆音。
    /// 3. **`fade_frames == 0` 退化为硬窃取**（判据注入用）：不动旧声部，直接覆盖。
    ///    这时输出会出现"旧波形硬切到新波形"的样本间跃变。
    ///
    /// ## 逐位保真的一处怪癖（**不改**）
    ///
    /// 进入淡出时，被窃取声部的 **mip 级**被换成新音符的（上移前 `synth.rs` 的
    /// `trigger` 结尾就是 `victim.level = level`），而相位/增量/增益仍是**旧音符**的。
    /// 这是既有行为，`crates/yeban-engine/tests/steal_fade.rs` 的读数是对着它记的，
    /// 本票逐位保真 ⇒ 原样保留，只在 `pending` 里存整份新音符状态。
    ///
    /// **实时路径**：零分配、零锁、零 I/O。
    pub fn note_on(&mut self, note: NoteEvent, tables: &PolySynthTables) {
        let base_inc = phase_increment(note.freq_hz, self.sample_rate);
        let osc1 = resolve_osc(
            self.params.osc1,
            note.freq_hz,
            base_inc,
            self.sample_rate,
            tables,
        );
        let osc2 = resolve_osc(
            self.params.osc2,
            note.freq_hz,
            base_inc,
            self.sample_rate,
            tables,
        );
        let pending = PendingNote {
            start_sample: note.start_sample,
            end_sample: note.end_sample,
            gain: note.gain,
            osc1,
            osc2,
        };

        if let Some(index) = self.voices.iter().position(|voice| !voice.active) {
            let voice = self.fresh_voice(&pending);
            self.voices[index] = voice;
            self.notes_triggered = self.notes_triggered.saturating_add(1);
            return;
        }

        // 池满：选一个被终止者（三级比较，见本节文档 §1）。
        let mut best = 0usize;
        for (index, voice) in self.voices.iter().enumerate() {
            let candidate = (steal_priority(voice), voice.start_sample, index);
            let current = (
                steal_priority(&self.voices[best]),
                self.voices[best].start_sample,
                best,
            );
            if candidate < current {
                best = index;
            }
        }
        self.voice_steals = self.voice_steals.saturating_add(1);
        self.notes_triggered = self.notes_triggered.saturating_add(1);

        let fade_frames = self.steal_fade_frames;
        if fade_frames == 0 || !self.voices[best].active {
            // 硬窃取（注入路径 / 显式配置）：直接覆盖，不留淡出。
            let voice = self.fresh_voice(&pending);
            self.voices[best] = voice;
            return;
        }
        let victim = &mut self.voices[best];
        if victim.fade_remaining == 0 {
            // 先把旧声部推进淡出（`start_steal_fade` 覆盖 release 为 3 ms 并 gate_off）。
            victim.env.start_steal_fade();
            victim.fade_remaining = fade_frames;
            // 相位/增量/增益/表仍是**旧音符**的（淡出的是旧声音）；
            // 只有 mip 级被换成新音符的（见本节文档的"逐位保真的怪癖"）。
            victim.osc1.level = pending.osc1.level;
            victim.osc2.level = pending.osc2.level;
        }
        // 挂起新音符：淡出走完的那一帧换成它（起音从 0 开始）。
        victim.end_sample = pending.end_sample;
        victim.pending = Some(pending);
    }

    /// 渲染本器件的下一个块（**实时路径**）。`out.len()` 就是本块的有效帧数。
    ///
    /// `position` 是 `out[0]` 对应的**绝对样本位置**（由调用方的播放头给出）——
    /// 声部只在 `now >= start_sample` 时发声，`gate` 由 `now < end_sample` 决定。
    ///
    /// **实时路径**：零分配、零释放、零锁、零阻塞 I/O、零日志
    /// [ARCH-RT-001 / `MUST-GATE-001`]。
    pub fn render(&mut self, tables: &PolySynthTables, position: u64, out: &mut [f32]) {
        out.fill(0.0);
        if !self.voices.iter().any(|voice| voice.active) {
            return;
        }
        let filter_bypass = self.params.filter_bypass;
        let osc2_on = self.params.osc2.is_on();
        for (frame, output) in out.iter_mut().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let now = position + frame as u64;
            let mut accumulator = 0.0f32;
            for voice in &mut self.voices {
                if !voice.active || now < voice.start_sample {
                    continue;
                }
                // --- 窃取淡出：旧音符指数淡出，期满换成挂起的新音符 ---
                //
                // 输出的是**旧音符**的样本 × 旧增益 × 正在下降的包络
                //（`gate = false` ⇒ `Adsr` 走 Release 段，系数已在
                // `start_steal_fade` 里覆盖成 3 ms）。新音符的值只在
                // `fade_remaining` 归零的那一帧启用。
                if voice.fade_remaining > 0 {
                    voice.fade_remaining -= 1;
                    let envelope = voice.env.process(false);
                    if voice.fade_remaining == 0 && voice.pending.is_some() {
                        // 新音符从 0 起 attack：相位归零、起始位置改为当前帧、
                        // 包络 reset + gate_on。新音符**从这一帧**开始发声，
                        // 而旧音符在上一帧已经衰减到 ≈0（`Adsr` 在 < 1e-4 时归零）
                        // ⇒ 中间不存在"两个波形的和"。
                        if let Some(pending) = voice.pending.take() {
                            voice.osc1 = OscState {
                                phase: 0,
                                ..pending.osc1
                            };
                            voice.osc2 = OscState {
                                phase: 0,
                                ..pending.osc2
                            };
                            voice.gain = pending.gain;
                            voice.start_sample = pending.start_sample.max(now);
                            voice.end_sample = pending.end_sample;
                            voice.env.reset();
                            voice.env.gate_on();
                        }
                    } else {
                        let mut sample =
                            oscillator_sample(tables, voice, osc2_on) * envelope * voice.gain;
                        if !filter_bypass {
                            sample = voice.filter.process(sample);
                        }
                        accumulator += sample;
                        voice.advance(osc2_on);
                        if !voice.env.is_active() {
                            *voice = PolyVoice::IDLE;
                        }
                        continue;
                    }
                }
                let gate = now < voice.end_sample;
                let envelope = voice.env.process(gate);
                if !voice.env.is_active() {
                    *voice = PolyVoice::IDLE;
                    continue;
                }
                let mut sample = oscillator_sample(tables, voice, osc2_on) * envelope * voice.gain;
                // 声部级低通。**旁通时一次也不调用** ⇒ 逐位恒等
                //（不是"系数取成透明"，那样状态仍会吸收瞬态）。
                if !filter_bypass {
                    sample = voice.filter.process(sample);
                }
                accumulator += sample;
                voice.advance(osc2_on);
            }
            *output = accumulator;
        }
    }

    /// 单个声部的诊断快照：`(活跃, end_sample, 包络阶段, 淡出剩余)`。
    ///
    /// 它**不分配**、不改变任何状态，因此也可以在实时侧调用 —— 但产物路径并不需要它。
    /// 这里**不做** `cfg(debug_assertions)` 门控：判据要能在 release 档下编译
    /// （把方法 gated 掉会让 `cargo test --release` 编译不过，那是判据强度的损失）。
    /// 引擎侧的 `SynthEngine::debug_voice_state` 仍然按
    /// `debug_assertions` 门控（它要收集成 `Vec`）。
    #[must_use]
    pub fn debug_voice(&self, index: usize) -> Option<(bool, u64, AdsrStage, u32)> {
        self.voices.get(index).map(|voice| {
            (
                voice.active,
                voice.end_sample,
                voice.env.stage(),
                voice.fade_remaining,
            )
        })
    }

    /// 新建一个"刚起音"的声部（触发路径用；含包络系数的 `exp`，每次触发一次）。
    #[must_use]
    fn fresh_voice(&self, pending: &PendingNote) -> PolyVoice {
        PolyVoice {
            active: true,
            osc1: pending.osc1,
            osc2: pending.osc2,
            gain: pending.gain,
            start_sample: pending.start_sample,
            end_sample: pending.end_sample,
            env: self.env_for(),
            filter: self.filter_template,
            fade_remaining: 0,
            pending: None,
        }
    }

    /// 按当前参数造一个"已 gate_on、电平 0"的包络。
    #[must_use]
    fn env_for(&self) -> Adsr {
        let mut env = Adsr::new();
        env.set_sample_rate(self.sample_rate);
        env.set_params(
            self.params.attack_s,
            self.params.decay_s,
            self.params.sustain,
            self.params.release_s,
        );
        env.reset();
        env.gate_on();
        env
    }
}

impl<const VOICES: usize> Default for PolySynth<VOICES> {
    fn default() -> Self {
        Self::new(48_000)
    }
}

/// 按参数在给定采样率下武装一个滤波器（含 `tan`，超越函数类）。
#[must_use]
fn configure_filter(params: &PolySynthParams, sample_rate: f32) -> LadderFilter {
    let mut filter = LadderFilter::new();
    filter.configure(
        sample_rate,
        params.cutoff_hz,
        params.resonance,
        params.drive,
    );
    filter
}

/// 把波表下标钳进库长度（`set_params` 时一次，之后触发不再需要库）。
#[must_use]
fn clamp_table(settings: OscSettings, tables: &PolySynthTables) -> OscSettings {
    OscSettings {
        table: settings.table.min(tables.len().saturating_sub(1)),
        ..settings
    }
}

/// 采样率下限钳制（与 `crate::MIN_SAMPLE_RATE` 同口径）。
#[must_use]
fn sanitise_sample_rate(sample_rate: u32) -> f32 {
    let value = sample_rate as f32;
    if value.is_finite() && value >= MIN_SAMPLE_RATE {
        value
    } else {
        MIN_SAMPLE_RATE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oscillator::{HOLLOW, ORGAN};

    /// 单 bin DFT 的幅度（判据用；窗口为矩形，故读数含泄漏 —— 只用于"峰在不在"）。
    fn bin_magnitude(samples: &[f32], freq_hz: f32, sample_rate: f32) -> f64 {
        let mut re = 0.0f64;
        let mut im = 0.0f64;
        for (index, sample) in samples.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let phase =
                core::f64::consts::TAU * f64::from(freq_hz) * index as f64 / f64::from(sample_rate);
            re += f64::from(*sample) * phase.cos();
            im += f64::from(*sample) * phase.sin();
        }
        (re * re + im * im).sqrt() / samples.len() as f64 * 2.0
    }

    fn tables() -> PolySynthTables {
        PolySynthTables::from_recipes(&[HOLLOW, ORGAN])
    }

    fn render_note(params: PolySynthParams, frames: usize) -> Vec<f32> {
        let tables = tables();
        let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
        synth.set_params(params, &tables);
        synth.note_on(NoteEvent::new(0, 96_000, 440.0, 1.0), &tables);
        let mut out = vec![0.0f32; frames];
        synth.render(&tables, 0, &mut out);
        out
    }

    /// 默认参数：单振荡器、滤波器旁通、包络 5/80/0.7/50 ms。
    #[test]
    fn default_params_match_the_moved_engine_tone() {
        let params = PolySynthParams::new();
        assert_eq!(params.osc1(), OscSettings::new(0, 1.0, 0.0));
        assert_eq!(params.osc2(), OscSettings::off());
        assert!(!params.osc2().is_on());
        assert!(params.filter_bypass());
        assert_eq!(params.attack_s(), 0.005);
        assert_eq!(params.decay_s(), 0.08);
        assert_eq!(params.sustain(), 0.7);
        assert_eq!(params.release_s(), 0.05);
        assert_eq!(params, PolySynthParams::default());
    }

    /// 波表库：空切片退回**一张** `HOLLOW` 表（不静音、不 panic）。
    #[test]
    fn an_empty_recipe_list_still_yields_one_table() {
        let tables = PolySynthTables::from_recipes(&[]);
        assert_eq!(tables.len(), 1);
        assert!(!tables.is_empty());
        // 越界下标被钳到最后一张表（不 panic）。
        assert_eq!(
            tables.level_for(999, 440.0, 48_000.0),
            tables.level_for(0, 440.0, 48_000.0)
        );
    }

    /// A4 = 440 Hz 的基频落在 440 Hz 的 bin 上（单 bin DFT，单位 Hz 的判据见集成测试）。
    #[test]
    fn a4_puts_its_energy_in_the_440_hz_bin() {
        let out = render_note(PolySynthParams::new(), 4_800);
        let at_440 = bin_magnitude(&out, 440.0, 48_000.0);
        let at_200 = bin_magnitude(&out, 200.0, 48_000.0);
        let at_900 = bin_magnitude(&out, 900.0, 48_000.0);
        assert!(at_440 > 0.1, "440 Hz bin 太弱: {at_440:.6}");
        assert!(
            at_440 > 20.0 * at_200,
            "440 vs 200: {at_440:.6} / {at_200:.6}"
        );
        assert!(
            at_440 > 5.0 * at_900,
            "440 vs 900: {at_440:.6} / {at_900:.6}"
        );
    }

    /// 双振荡器：`osc2` 失谐 −1200 音分（低一个八度）⇒ 220 Hz 的 bin 显著抬高。
    #[test]
    fn the_second_oscillator_adds_its_own_fundamental() {
        let single = render_note(PolySynthParams::new(), 4_800);
        let dual = render_note(
            PolySynthParams::new().with_oscillators(
                OscSettings::new(0, 1.0, 0.0),
                OscSettings::new(1, 0.5, -1_200.0),
            ),
            4_800,
        );
        let single_220 = bin_magnitude(&single, 220.0, 48_000.0);
        let dual_220 = bin_magnitude(&dual, 220.0, 48_000.0);
        assert!(
            dual_220 > 20.0 * single_220,
            "第二条振荡器必须带来它自己的基频: 单 {single_220:.6} / 双 {dual_220:.6}"
        );
        // 两个声部的和必须仍然有限。
        assert!(dual.iter().all(|sample| sample.is_finite()));
    }

    /// 关掉的 `osc2` 支路**不改变任何一个位**（引擎侧逐位不变的前提）。
    #[test]
    fn an_off_second_oscillator_is_bit_identical() {
        let a = render_note(PolySynthParams::new(), 2_048);
        let b = render_note(
            PolySynthParams::new().with_oscillators(
                OscSettings::new(0, 1.0, 0.0),
                OscSettings::new(1, 0.0, 700.0),
            ),
            2_048,
        );
        let a_bits: Vec<u32> = a.iter().map(|s| s.to_bits()).collect();
        let b_bits: Vec<u32> = b.iter().map(|s| s.to_bits()).collect();
        assert_eq!(a_bits, b_bits, "level = 0 的支路必须完全不参与");
    }

    /// 退化参数不产生 `NaN`/`inf`，也不静音。
    #[test]
    fn degenerate_params_stay_finite_and_audible() {
        let params = PolySynthParams::new()
            .with_filter(f32::NAN, f32::INFINITY, f32::NEG_INFINITY, false)
            .with_envelope(f32::NAN, -1.0, f32::NAN, f32::INFINITY)
            .with_oscillators(
                OscSettings::new(usize::MAX, f32::NAN, f32::NAN),
                OscSettings::new(usize::MAX, f32::INFINITY, f32::INFINITY),
            );
        let out = render_note(params, 2_048);
        assert!(
            out.iter().all(|sample| sample.is_finite()),
            "退化参数产生了非有限值"
        );
        assert!(
            out.iter().any(|sample| *sample != 0.0),
            "退化参数把链路弄成静音"
        );
    }

    /// 窃取是确定性的：同一夹具两次，逐位相同，且被窃取的声部数相同。
    #[test]
    fn stealing_is_deterministic() {
        let mut active = Vec::new();
        let mut bits = Vec::new();
        for _ in 0..2 {
            let tables = tables();
            let mut synth = PolySynth::<4>::new(48_000);
            synth.set_params(PolySynthParams::new(), &tables);
            for index in 0..4u64 {
                synth.note_on(
                    NoteEvent::new(index * 128, 96_000, 100.0 + index as f32 * 10.0, 1.0),
                    &tables,
                );
            }
            // 先让包络真的在发声，再逼出窃取（见 `the_earliest_voice_is_stolen_first`
            // 的注释：在电平为 0 的声部上淡出会一帧就回收）。
            let mut warmup = vec![0.0f32; 500];
            synth.render(&tables, 0, &mut warmup);
            synth.note_on(NoteEvent::new(2_000, 96_000, 4_000.0, 1.0), &tables);
            assert_eq!(synth.voice_steals(), 1);
            let mut out = vec![0.0f32; 1_024];
            synth.render(&tables, 500, &mut out);
            bits.push(out.iter().map(|s| s.to_bits()).collect::<Vec<u32>>());
            active.push(synth.active_voices());
        }
        assert_eq!(bits[0], bits[1], "窃取必须逐位确定");
        assert_eq!(active[0], active[1]);
        assert_eq!(active[0], 4, "夹具必须始终是 4 个活跃声部");
    }

    /// 池满时被窃取的是"最早触发"的那个声部（优先级相同 ⇒ 取 `start_sample` 最小者）。
    ///
    /// 身份口径：每个初始音符给一个**互不相同的 `end_sample`**（都在很远的将来，
    /// 因此渲染窗口里不会有声部自然退役），两个新音符给另外两个值；
    /// `note_on` 在被窃取的槽位上**立刻**写入新音符的 `end_sample`
    ///（`victim.end_sample = pending.end_sample`）⇒ 读出那个槽位的下标，
    /// 就是"窃取的是哪一个"的机械读数。
    #[test]
    fn the_earliest_voice_is_stolen_first() {
        let tables = tables();
        let mut synth = PolySynth::<4>::new(48_000);
        synth.set_params(PolySynthParams::new(), &tables);
        for index in 0..4u64 {
            synth.note_on(
                NoteEvent::new(index * 128, 90_001 + index, 200.0, 1.0),
                &tables,
            );
        }
        let holder = |synth: &PolySynth<4>, tag: u64| -> Option<usize> {
            (0..4).find(|index| {
                synth
                    .debug_voice(*index)
                    .is_some_and(|state| state.1 == tag)
            })
        };

        // ⚠ 先渲染 500 帧：包络必须已经真的在发声（sustain）。若在"刚 gate_on、电平还是 0"
        // 的声部上开始淡出，`Adsr` 从 0 释放 ⇒ 一帧就判静音、声部被回收，
        // 窃取的淡出根本不会发生（本线实测踩到过：`voice_steals` 只有 1）。
        let mut warmup = vec![0.0f32; 500];
        synth.render(&tables, 0, &mut warmup);

        // 第 5 个音符：池满 ⇒ 恰好一次窃取；起点最小（0）的下标 0 被拿走。
        synth.note_on(NoteEvent::new(2_000, 99_999, 4_000.0, 1.0), &tables);
        assert_eq!(synth.voice_steals(), 1);
        assert_eq!(
            holder(&synth, 99_999),
            Some(0),
            "必须先窃取最早触发的下标 0"
        );

        // 走完 3 ms 淡出（144 帧）⇒ 挂起的新音符在下标 0 上换入。
        // 它的起点是 2000（`pending.start_sample.max(now)` 取大者），
        // 而其余声部的起点仍是 128 / 256 / 384 ⇒ 最早者变成下标 1。
        let mut fade = vec![0.0f32; 200];
        synth.render(&tables, 500, &mut fade);

        synth.note_on(NoteEvent::new(2_100, 88_888, 5_000.0, 1.0), &tables);
        assert_eq!(synth.voice_steals(), 2);
        assert_eq!(holder(&synth, 88_888), Some(1), "第二次必须窃取下标 1");
        assert_eq!(
            holder(&synth, 99_999),
            Some(0),
            "下标 0 此刻是新音符，不该再被窃取"
        );

        // 渲染不得 panic，输出必须有限。
        let mut out = [0.0f32; 8];
        synth.render(&tables, 2_100, &mut out);
        assert!(out.iter().all(|sample| sample.is_finite()));
    }

    /// 窃取的**第三级**口径：`start_sample` 并列时取**下标最小**者。
    ///
    /// ⚠ 这条判据是**注入 I3 逼出来的**：把三级比较的第三项从 `index` 改成
    /// `usize::MAX - index` 之后，`the_earliest_voice_is_stolen_first` 与集成的 P5
    /// **都没有变红** —— 因为那两个夹具的 4 个声部起点互不相同
    /// （0/128/256/384），比较在**第二项**就分出胜负，第三项从未被走到。
    /// 这里把 4 个声部的起点全部设成 0（和弦：同一时刻起音），逼出真正的并列。
    #[test]
    fn ties_on_start_sample_are_broken_by_the_smallest_index() {
        let tables = tables();
        let mut synth = PolySynth::<4>::new(48_000);
        synth.set_params(PolySynthParams::new(), &tables);
        for index in 0..4u64 {
            // 起点**全部为 0**、终点互不相同（身份标签）。
            synth.note_on(
                NoteEvent::new(0, 90_010 + index, 200.0 + index as f32, 1.0),
                &tables,
            );
        }
        let mut warmup = vec![0.0f32; 500];
        synth.render(&tables, 0, &mut warmup);
        let holder = |synth: &PolySynth<4>, tag: u64| -> Option<usize> {
            (0..4).find(|index| {
                synth
                    .debug_voice(*index)
                    .is_some_and(|state| state.1 == tag)
            })
        };

        // 第一次窃取：4 个声部起点并列 ⇒ 下标最小者（0）。
        synth.note_on(NoteEvent::new(900, 99_999, 1_500.0, 1.0), &tables);
        assert_eq!(synth.voice_steals(), 1);
        assert_eq!(
            holder(&synth, 99_999),
            Some(0),
            "起点并列时必须取下标最小者"
        );

        // 走完淡出 ⇒ 下标 0 换成新音符（起点 900）；下标 1/2/3 仍然并列在起点 0。
        let mut fade = vec![0.0f32; 200];
        synth.render(&tables, 500, &mut fade);

        synth.note_on(NoteEvent::new(901, 88_888, 1_700.0, 1.0), &tables);
        assert_eq!(synth.voice_steals(), 2);
        assert_eq!(
            holder(&synth, 88_888),
            Some(1),
            "并列集合缩小到 1/2/3 之后，必须取下标 1"
        );
    }

    /// 旁通**不是**"把系数取成透明"：旁通输出与"在旁通截止频率上真的开一个低通"
    /// 的输出必须**不同**。
    ///
    /// ⚠ 这条判据是**注入 I7 逼出来的**：把 `render` 里两处 `if !filter_bypass`
    /// 都改成恒真（"旁通时仍然 process"）之后，
    /// `crates/yeban-engine/tests/synth_filter.rs` 的 F1 **没有变红** ——
    /// F1 的两个臂是"旁通 vs 默认（也是旁通）"与"旁通 vs 300 Hz 低通"，
    /// 前者在注入下两边**同时**被滤波 ⇒ 仍然相等；后者仍然不等。
    /// 也就是说"旁通 = 20 kHz 透明滤波"这种错法在 F1 下是**不可见**的。
    ///
    /// 这里补上真正缺的那一个臂：把"旁通"与"显式在 20 kHz 上开低通"直接对比
    ///（`ToneParams::bypass` 用的就是这个 20 kHz 的占位截止频率）。
    #[test]
    fn bypass_is_not_a_transparent_filter() {
        let bypassed = render_note(PolySynthParams::new(), 2_048);
        let transparent = render_note(
            PolySynthParams::new().with_filter(TONE_BYPASS_CUTOFF_HZ, 0.0, 0.0, false),
            2_048,
        );
        assert!(
            bypassed.iter().any(|sample| *sample != 0.0),
            "夹具必须真的出声"
        );
        assert_ne!(
            bypassed.iter().map(|s| s.to_bits()).collect::<Vec<u32>>(),
            transparent
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<u32>>(),
            "旁通路径与'20 kHz 透明低通'必须是不同的位模式（否则旁通等于没做）"
        );
    }

    /// 硬窃取（淡出 = 0）与软窃取的输出**不同**，且硬窃取的样本跳变更大。
    #[test]
    fn hard_stealing_jumps_more_than_soft_stealing() {
        let step = |fade: u32| -> f32 {
            let tables = tables();
            let mut synth = PolySynth::<2>::new(48_000);
            synth.set_params(PolySynthParams::new(), &tables);
            synth.note_on(NoteEvent::new(0, 96_000, 110.0, 1.0), &tables);
            synth.note_on(NoteEvent::new(0, 96_000, 110.0, 1.0), &tables);
            synth.set_steal_fade_frames(fade);
            let mut out = vec![0.0f32; 1_024];
            synth.render(&tables, 0, &mut out);
            synth.note_on(NoteEvent::new(1_024, 96_000, 4_186.0, 1.0), &tables);
            let mut after = vec![0.0f32; 256];
            synth.render(&tables, 1_024, &mut after);
            after
                .windows(2)
                .fold(0.0f32, |worst, pair| worst.max((pair[1] - pair[0]).abs()))
        };
        let soft = step(steal_fade_frames_for(48_000.0));
        let hard = step(0);
        assert!(
            hard > soft,
            "硬窃取的样本跳变必须更大: 硬 {hard:.6} / 软 {soft:.6}"
        );
    }
}
