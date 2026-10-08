//! **每轨插入器件**的引擎侧接线：把 `TrackV3.devices` 里的内置效果器投影成
//! 音频线程可执行的**通道条**（`yeban_dsp::channel_strip`，内含 EQ ＋ 滤波 ＋ 压缩）。
//! [ARCH-RT-001, ARCH-DET-001]
//!
//! ## 1. 规范出处（原文引用）
//!
//! | 出处 | 原文 |
//! | :--- | :--- |
//! | `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:411` | `\| **内部 DSP 拓扑调度** \| - \| **1.00 ms** \| 声部合成、通道条 EQ/压缩与 PDC 延迟线插入计算 \|` |
//! | `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:109` | `- 通道条算法 ➔ crates/yeban-dsp/src/channel_strip.rs（EQ、滤波、动态旁通链）` |
//! | `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:108` | 压缩器算法的目的地 `crates/yeban-dsp/src/compressor.rs`（通道条的动态级复用同一份实现） |
//!
//! 器件**早就实现并自带判据**，缺的只有一件事：**引擎侧没有任何调用点**
//! （核实方式：`grep -rn 'use yeban_dsp' crates/yeban-engine/src` 在接线前只命中
//! `envelope` / `filter` / `math` / `oscillator` / `polysynth` / `limiter` / `meter`）。
//! 本模块补的就是那条调用点。
//!
//! ## 2. 与上一轮（`c792fdc`，每轨压缩器）的关系：**本文件是同一个模块的第二件器件**
//!
//! `c792fdc` 把**压缩器**接成每轨插入（一件器件、一条链、一个定长槽位表）。
//! 本票把**通道条**接进**同一条链**，做法是同一套形状的**扩展**（不是第二套接线）：
//!
//! 1. [`InsertParams`] 里原来只有 `compressor`；本票把它改成
//!    **一个** `Option<ChannelStripParams>`（`ChannelStrip` 内部**本来就含**动态级，
//!    见 `crates/yeban-dsp/src/channel_strip.rs` 的模块注释 §2 第 ④ 级）。
//!    ⇒ 一条轨**至多一件**插入器件，实时侧的"逐轨查找 + 整段跳过"骨架与
//!    槽位表形状**一个字没改**。
//! 2. 只有压缩器参数的设备仍然**只有动态级工作**：投影把它折成
//!    `eq_enabled = false` / `filter_enabled = false` 的通道条，其余字段取
//!    [`ChannelStripParams::DEFAULT`] ⇒ 输入/输出增益 `0 dB`、链路其余部分是旁通。
//!    ⇒ 这条路径的输出与 `c792fdc` 的裸压缩器**逐位相同**（判据
//!    `tests/compressor_insert.rs` 的 C1/C4/C5 原样保留并继续钉住它）。
//! 3. 出现 EQ / 滤波旋钮名的设备 ⇒ 那些级**才**被启用。
//!
//! ## 3. 为什么不是"器件一存在就生效"
//!
//! 模型层的 `DeviceDefinition` 只有 `kind`（`InternalEffect`）说明"这是一个内置效果器"，
//! **没有**"这是通道条"的类型级信息（该缺口是 `docs/ledger/engine-mix-notes.md` §8.2 的
//! **N5**：`DeviceDefinition::params` 没有参数名规范）。
//! 因此"有 `InternalEffect` ⇒ 挂通道条"会让**任何**内置效果器（例如只为上报延迟而存在的
//! 那一台）突然开始 EQ 与压限 —— 那是把未知器件猜成通道条。
//!
//! 本模块沿用**同一仓库已有的**投影规则（[`crate::synth::ToneParams::from_devices`] 的
//! 形状：只认 `InternalInstrument` 设备的 `cutoff_hz`/`resonance`/`drive` 三个约定名）：
//! **只有当设备的 `params` 里真的出现通道条参数名时，它才是通道条来源**。
//! 于是"没有参数的效果器"与"没有效果器"在音频上同解 ⇒ 既有工程逐位不变。
//!
//! ## 4. 投影规则（完全确定、无猜测）
//!
//! 1. 只看 [`DeviceKind::InternalEffect`] 的设备（外部效果器由插件宿主负责，
//!    `External*` 一律忽略；内置乐器是**音源**，由 [`crate::synth`] 处理）；
//! 2. `bypassed` 的设备整体忽略（旁通就是旁通，不"取默认参数偷偷接回来"）；
//! 3. 在**第一个**含**至少一个**已识别参数名的设备上取值；更早的、一个已识别参数都没有的
//!    效果器不是来源 ⇒ 继续往下找；
//! 4. 未出现的字段保持 `yeban_dsp` 的默认值：
//!    - 增益类（`input_gain_db` / `output_gain_db`）⇒ `0 dB`（数学恒等 `x · 1.0`）；
//!    - EQ 字段 ⇒ `EqParams::default()`（**平坦**，三个增益 `0 dB`）；
//!    - 滤波字段 ⇒ [`FilterParams::DEFAULT`]（截止 `20 kHz`、无谐振、无驱动）；
//!    - 动态字段 ⇒ [`CompressorParams::DEFAULT`]（与 `c792fdc` 的裸压缩器**同一个常数**）；
//! 5. 同一设备内同名参数**后者胜**（与 `ToneParams::from_devices` 同口径）；
//! 6. 参数名大小写不敏感（`to_ascii_lowercase` 后比较）；
//! 7. 全部取值经 [`ChannelStripParams::sanitised`] 钳到合法域
//!    ⇒ 快照里存的就是音频线程将要用的那一份数（`NaN` 也有定义）；
//! 8. **一个已识别参数都没有 ⇒ [`InsertParams::is_empty`]**，实时侧整段跳过。
//!
//! ### 4.1 三级的启用开关（默认口径 = "没写的级不执行"）
//!
//! 通道条的 EQ 与滤波级**不是逐位恒等**变换：
//! `crates/yeban-dsp/src/channel_strip.rs` 的模块注释 §3 实测"平坦 EQ 仍有
//! `4.566e-5` 的末位舍入"，§6 也说明 `20 kHz` 低通仍有增益。因此
//! **"沉默地启用一条平坦 EQ"会改变输出**（这是听得见与逐位的双重改变）。
//!
//! 本模块的口径是：**只有写出了该级旋钮的设备才启用该级**。
//!
//! | 设备写了什么 | `eq_enabled` | `filter_enabled` | `compressor_enabled` |
//! | :--- | :--- | :--- | :--- |
//! | 一个通道条参数都没有 | ——（整轨不武装，判据见规则 8） | | |
//! | 只有 EQ 旋钮或无（见下） | `true` | `false` | `false` |
//! | 只有滤波旋钮 | `false` | `true` | `false` |
//! | 只有动态旋钮（`c792fdc` 的既有形状） | `false` | `false` | `true` |
//! | EQ ／ 滤波 ＋ 动态 | `true` | `true` | `true` |
//! | `eq_enabled = true` 写死（无 EQ 增益旋钮） | `true` | —— | —— |
//!
//! ⚠ 由此得到一个**明说的**取舍：一条只写 `eq_low_gain` 的设备会让**平坦**的中/高频
//! 双二阶也开起来（滤波器是三级串联的，`ShapingEq` 只有整级开关）。这与
//! `yeban_dsp::channel_strip` 的 `eq_enabled` 语义一致（该级**整级**执行或整级跳过）。
//!
//! ### 4.2 已识别的名字（每项列出全部别名）
//!
//! | 目标字段 | 名字（全部别名） |
//! | :--- | :--- |
//! | `input_gain_db` | `input_gain_db`、`input_gain`、`trim_db` |
//! | `eq_enabled` | `eq_enabled` |
//! | `eq.low_gain` | `eq_low_gain`、`low_gain` |
//! | `eq.low_freq` | `eq_low_freq`、`low_freq` |
//! | `eq.mid_gain` | `eq_mid_gain`、`mid_gain` |
//! | `eq.mid_freq` | `eq_mid_freq`、`mid_freq` |
//! | `eq.mid_q` | `eq_mid_q`、`mid_q` |
//! | `eq.high_gain` | `eq_high_gain`、`high_gain` |
//! | `eq.high_freq` | `eq_high_freq`、`high_freq` |
//! | `filter_enabled` | `filter_enabled` |
//! | `filter.cutoff_hz` | `cutoff_hz`（⚠ **没有** `filter_cutoff_hz` 这个别名，理由见 §4.3） |
//! | `filter.resonance` | `resonance`、`filter_resonance` |
//! | `filter.drive` | `drive`、`filter_drive` |
//! | `compressor_enabled` | `compressor_enabled` |
//! | `compressor.*` | `c792fdc` 已登记的七个名字及其别名（`threshold_db`/`threshold`、`ratio`、`knee_db`/`knee`、`detector_s`/`detector`、`attack_s`/`attack`、`release_s`/`release`、`makeup_db`/`makeup`） |
//! | `output_gain_db` | `output_gain_db`、`output_gain`、`fader_db` |
//!
//! ### 4.3 为什么**不**认 `filter_cutoff_hz`
//!
//! `tests/compressor_insert.rs` 的 C0 用两台效果器表达"**弄不懂的**效果器"：
//! 一台 `params` 为空，另一台带 `("reverb_mix", 0.3)` 与 `("filter_cutoff_hz", 800.0)`
//! （`crates/yeban-engine/tests/compressor_insert.rs:122`）。C0 断言这类工程与
//! "根本没有设备链"的工程**逐字节相同** —— 它是 `c792fdc` 的**字面判据**，
//! 本票不修改它的期望值。
//!
//! **实测**：把 `filter_cutoff_hz` 收作 `filter.cutoff_hz` 的别名之后，
//! `cargo test -p yeban-engine --no-default-features --test compressor_insert` 的 C0
//! **变红**（`参数不认识的的效果器: 输出必须与没有设备链时逐位相同`，
//! 指纹 `11503627533076961125` ≠ `7952998718812303221`）—— 因为那台设备从"不认识"
//! 变成了"一条 800 Hz 低通的通道条"。
//!
//! 本票的裁决：**不收这个名字**。它换来的是"接线不改变既有听感"这条更高的约束
//! （brief 的 ⚠ 项），代价是"想用 `filter_cutoff_hz` 表达通道条低通"的调用方
//! 必须改写 `cutoff_hz`。⚠ 这条取舍**是明说的**，不是遗漏。
//!
//! ## 5. 这是**临时形状**，不是模型层的第二份定义
//!
//! 与 [`crate::synth::ToneParams`] 一样：`yeban-model` 补齐"效果器参数 → 音频线程"的
//! 投影（N5 的裁决）之后，本模块的投影部分应当**整体删除**，只保留 `pub use` 与
//! 参数类型的再导出。删除前它必须保持**唯一实现**：
//! 本文件不定义通道条算法、不定义压缩器算法、不定义第二份参数类型 —— 判据
//! `engine_insert_module_has_no_second_compressor_implementation`（既有）与
//! `engine_insert_module_has_no_second_channel_strip_implementation`（本票新增）
//! 用源码级检查钉住它。
//!
//! ## 6. 实时分类（[ADR-0001 D32]）
//!
//! - **构造期**（控制线程，[`InsertParams::from_devices`]）：字符串比较、`Vec` 扫描、
//!   `sanitised` 的钳制 —— 允许分配，允许超越函数。
//! - **快照边界**（音频线程，每个修订一次）：[`ChannelStrip::new`] /
//!   [`ChannelStrip::set_params`] / [`ChannelStrip::set_sample_rate`] 会重算 EQ 与
//!   滤波系数、以及压缩器的三个一阶低通系数（含 `exp`）⇒ **超越函数类**。
//!   它与引擎里既有的"武装"步骤同一条口径（`crate::rt` 在同一个分支里调
//!   [`crate::mixer::pan_gains`] 的 `cos`/`sin`）。**不在逐样本路径上**。
//! - **逐样本**（[`ChannelStrip::process_mono`]）：乘加、一阶递归、除法；
//!   `exp`/`cos`/`sin` 不在路径上 ⇒ IEEE 精确类；**零分配、零锁、零 I/O**。
//!   EQ 的中间缓冲是**栈数组** `[f32; 64]`（`crates/yeban-dsp/src/channel_strip.rs` §3）。
//!
//! ## 7. 与"接线会改变渲染输出"的关系（本模块的边界）
//!
//! 接线的**默认口径**是"没有已识别的效果器设备 ⇒ 不挂通道条"：
//! 实时侧对此**整段跳过**（不是"参数取成透明"）⇒ 这类工程的输出与接线前**逐位相同**。
//! 只有**显式**携带通道条或压缩器参数的工程会改变输出 —— 那正是本模块的目的。
//! 逐位一致的**实测**证据（原始样本落盘 + `shasum -a 256`）见
//! `crates/yeban-engine/tests/compressor_insert.rs` 的 C0（未武装）与
//! `crates/yeban-engine/tests/channel_strip_insert.rs` 的 C8（未武装的四条口径）。

use yeban_model::{DeviceDefinition, DeviceKind};

// 通道条的**唯一实现**住在 `yeban-dsp`（`crates/yeban-dsp/src/channel_strip.rs`，
// 规范出处见 `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:109`）。本模块只做转发：
// 类型 + 参数类型。⚠ 引擎侧的既有命名先例是 `mixer.rs` 的 `Limiter as BusLimiter`
// —— 那是为了不改动既有调用点而保留旧名；本模块是**新**调用点，故直接用 dsp 的名字。
pub use yeban_dsp::channel_strip::{
    ChannelStrip, ChannelStripParams, FilterParams, MAX_CUTOFF_HZ, MAX_GAIN_DB, MIN_CUTOFF_HZ,
    MIN_GAIN_DB,
};
// 通道条的动态级复用 `yeban_dsp::compressor`（本模块**不**定义第二份）。
//
// ⚠ `Compressor` 与 `CompressorParams` **必须继续再导出**：`c792fdc` 的判据
// （`tests/compressor_insert.rs` 的 C6）按这两个名字断言"引擎的压缩器**就是** dsp 的
// 压缩器"（类型同一性 + `process_mono` / `new` / `set_params` 的函数地址同一性）。
// 本票把槽位里的器件换成通道条，但**没有**让那条判据失去对象：通道条的动态级仍是
// 同一个类型。删除这两个再导出会让 C6 编译失败（那是判据被削弱，不是被满足）。
pub use yeban_dsp::compressor::{Compressor, CompressorParams};
// EQ 参数类型同样转发（`ChannelStripParams::eq` 的字段类型）。
pub use yeban_dsp::shaping::EqParams;
// 判据要断言"引擎的通道条**就是** dsp 的通道条"（类型同一性 + 函数地址同一性）。
// `ChannelStrip` 这个名字已经被本模块上面的再导出占用，故再给一个**别名**让判据能写出
// "dsp 类型 ← engine 路径构造"的赋值（与 `mixer.rs` 的 `Limiter as BusLimiter` 同款）。
pub use yeban_dsp::channel_strip::ChannelStrip as DspChannelStrip;

/// 一条轨的**插入链**投影（当前只有一件器件：通道条，其动态级即 `c792fdc` 的压缩器）。
///
/// 结构体保留一个"链"的形状而不是裸 `Option<ChannelStripParams>`：后续器件
/// （`drums` / `reverb`）接线时在这里**加字段**即可，实时侧的
/// "逐轨查找 + 整段跳过"骨架不必改。
///
/// ⚠ 本票**没有**为 `drums` / `reverb` 加字段：它们仍然**未接线**
/// （核实方式见 `docs/ledger/integration-rulings-notes.md:83` 的口径）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InsertParams {
    /// 本轨的通道条参数：`None` = 本轨**没有**插入器件（实时侧整段跳过）。
    strip: Option<ChannelStripParams>,
}

impl InsertParams {
    /// **空链**：实时侧对这条轨整段跳过 ⇒ 输出逐位不变。
    #[must_use]
    pub const fn empty() -> Self {
        Self { strip: None }
    }

    /// 从模型层的设备链投影（**构造期**；见模块文档 §4 的规则表）。
    #[must_use]
    pub fn from_devices(devices: &[DeviceDefinition]) -> Self {
        let mut strip: Option<ChannelStripParams> = None;
        for device in devices {
            // 规则 1 + 2：只看非旁通的内置效果器。
            if device.bypassed || device.kind != DeviceKind::InternalEffect {
                continue;
            }
            // 规则 3：`None` = 本设备一个已识别参数都没有 ⇒ 它不是通道条来源。
            strip = Self::strip_of(device);
            if strip.is_some() {
                break;
            }
        }
        // 规则 7：快照里存的就是音频线程将要用的那一份数。
        Self {
            strip: strip.map(ChannelStripParams::sanitised),
        }
    }

    /// 单台设备 → 通道条参数（`None` = 一个已识别名字都没有）。
    ///
    /// 基值是 `yeban_dsp` 的设备默认值；只有**出现**的名字才覆盖它（规则 4）。
    /// 三级的启用开关按模块文档 §4.1 的表推导：**没写旋钮的级不执行**。
    fn strip_of(device: &DeviceDefinition) -> Option<ChannelStripParams> {
        let mut resolved = StripProjection::default();
        for param in &device.params {
            let name = param.name.to_ascii_lowercase();
            let value = param.value;
            // 规则 5：同名后者胜（逐个赋值）。规则 6：名字已转小写。
            resolved.apply(&name, value);
        }
        resolved.into_params()
    }

    /// 本轨的**通道条参数**；`None` = 整段跳过。
    #[must_use]
    pub const fn strip(&self) -> Option<ChannelStripParams> {
        self.strip
    }

    /// 本轨通道条的**动态级**参数（`ChannelStripParams::compressor` 的直读）。
    ///
    /// 存在的理由与 `c792fdc` 的同类读数一样：把"投影出来的那个数"变成可读的，
    /// 判据就不必从音频输出反推。**只**压缩器参数的设备经模块文档 §2 第 2 条折成
    /// 只开动态级的通道条 ⇒ 这里的读数是那条既有路径的**权威**参数。
    #[must_use]
    pub fn compressor(&self) -> Option<CompressorParams> {
        self.strip.map(|strip| strip.compressor)
    }

    /// 本链是否为空（没有任何激活的器件）。
    ///
    /// 快照只收录**非空**的链 ⇒ "不在表里"与"表里是空链"在实时侧同解。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.strip.is_none()
    }
}

impl Default for InsertParams {
    fn default() -> Self {
        Self::empty()
    }
}

/// 一台设备的**逐字段投影中间态**（`insert.rs` 私有）。
///
/// 职责：把"参数名 → 字段"的匹配与"哪一级被启用"的推导集中在一处，
/// 让 [`InsertParams::strip_of`] 只负责遍历参数。
///
/// 为什么需要中间态而不是直接改 `ChannelStripParams`：三级的启用开关**不能**用
/// 设备默认值（`eq_enabled` / `filter_enabled` 在 dsp 里是 `true`，而"没写旋钮"必须
/// 得到 `false`，见模块文档 §4.1）。`None` 在这里的语义是"设备没写这个开关"。
#[derive(Clone, Copy, Debug)]
struct StripProjection {
    /// 出现过的 EQ 旋钮。
    eq: StripEq,
    /// 出现过滤波旋钮。
    filter_seen: bool,
    /// 出现过动态旋钮（或 `compressor_enabled`）。
    compressor_seen: bool,
    /// 设备写过的开关（`eq_enabled` / `filter_enabled` / `compressor_enabled`）。
    eq_enabled: Option<bool>,
    filter_enabled: Option<bool>,
    compressor_enabled: Option<bool>,
    /// 非旋钮字段（增益 / 截止频率 / 动态参数 / EQ 频率与 Q）。
    params: ChannelStripParams,
}

#[derive(Clone, Copy, Debug, Default)]
struct StripEq {
    seen: bool,
    low_gain: Option<f32>,
    low_freq: Option<f32>,
    mid_gain: Option<f32>,
    mid_freq: Option<f32>,
    mid_q: Option<f32>,
    high_gain: Option<f32>,
    high_freq: Option<f32>,
}

impl Default for StripProjection {
    fn default() -> Self {
        Self {
            eq: StripEq::default(),
            filter_seen: false,
            compressor_seen: false,
            eq_enabled: None,
            filter_enabled: None,
            compressor_enabled: None,
            // 基值 = dsp 的设备默认值（规则 4）。
            params: ChannelStripParams::DEFAULT,
        }
    }
}

/// 布尔参数的取值口径：非零即真（`0.0` 是假，其余值是真）。
///
/// 模型层的参数值是 `f32`（`ParameterValue::value`），没有布尔类型 ⇒ 这里给出一条
/// 机械且确定的读法，而不是"大于 0.5 算真"这类需要解释的阈值。
fn flag(value: f32) -> bool {
    value != 0.0
}

impl StripProjection {
    /// 把一条参数写进投影（**构造期**；名字已转小写）。
    fn apply(&mut self, name: &str, value: f32) {
        match name {
            // --- 增益级（逐样本乘一个常数）---
            "input_gain_db" | "input_gain" | "trim_db" => self.params.input_gain_db = value,
            "output_gain_db" | "output_gain" | "fader_db" => self.params.output_gain_db = value,

            // --- 级开关 ---
            "eq_enabled" => self.eq_enabled = Some(flag(value)),
            "filter_enabled" => self.filter_enabled = Some(flag(value)),
            "compressor_enabled" => {
                self.compressor_enabled = Some(flag(value));
                self.compressor_seen = true;
            }

            // --- EQ 旋钮 ---
            "eq_low_gain" | "low_gain" => {
                self.eq.low_gain = Some(value);
                self.eq.seen = true;
            }
            "eq_low_freq" | "low_freq" => {
                self.eq.low_freq = Some(value);
                self.eq.seen = true;
            }
            "eq_mid_gain" | "mid_gain" => {
                self.eq.mid_gain = Some(value);
                self.eq.seen = true;
            }
            "eq_mid_freq" | "mid_freq" => {
                self.eq.mid_freq = Some(value);
                self.eq.seen = true;
            }
            "eq_mid_q" | "mid_q" => {
                self.eq.mid_q = Some(value);
                self.eq.seen = true;
            }
            "eq_high_gain" | "high_gain" => {
                self.eq.high_gain = Some(value);
                self.eq.seen = true;
            }
            "eq_high_freq" | "high_freq" => {
                self.eq.high_freq = Some(value);
                self.eq.seen = true;
            }

            // --- 滤波旋钮 ---
            // ⚠ 只有 `cutoff_hz`：`filter_cutoff_hz` **刻意不认**，理由见模块文档 §4.3。
            "cutoff_hz" => {
                self.params.filter.cutoff_hz = value;
                self.filter_seen = true;
            }
            "resonance" | "filter_resonance" => {
                self.params.filter.resonance = value;
                self.filter_seen = true;
            }
            "drive" | "filter_drive" => {
                self.params.filter.drive = value;
                self.filter_seen = true;
            }

            // --- 动态旋钮（`c792fdc` 已登记的七个名字及其别名）---
            "threshold_db" | "threshold" => {
                self.params.compressor.threshold_db = value;
                self.compressor_seen = true;
            }
            "ratio" => {
                self.params.compressor.ratio = value;
                self.compressor_seen = true;
            }
            "knee_db" | "knee" => {
                self.params.compressor.knee_db = value;
                self.compressor_seen = true;
            }
            "detector_s" | "detector" => {
                self.params.compressor.detector_s = value;
                self.compressor_seen = true;
            }
            "attack_s" | "attack" => {
                self.params.compressor.attack_s = value;
                self.compressor_seen = true;
            }
            "release_s" | "release" => {
                self.params.compressor.release_s = value;
                self.compressor_seen = true;
            }
            "makeup_db" | "makeup" => {
                self.params.compressor.makeup_db = value;
                self.compressor_seen = true;
            }

            _ => {}
        }
    }

    /// 收尾：把出现过（或一个都没出现）的字段落成 [`ChannelStripParams`]。
    ///
    /// `None` = 一个已识别参数都没有 ⇒ 本设备不是通道条来源（规则 8）。
    fn into_params(self) -> Option<ChannelStripParams> {
        // "已识别"的三个来源：EQ 旋钮 / 滤波旋钮 / 动态旋钮（含 `compressor_enabled`）、
        // 三个级开关、以及任何非旋钮字段（增益等）被写过。
        // ⚠ `params != DEFAULT` 是**兜底**：任何一条被 `apply` 认下的名字都至少改到一处，
        // 因此它只可能漏掉"写了一个与默认值逐位相同的值"——那种写法由三个 `*_seen`
        // 标志覆盖（`compressor_enabled` 自己也设 `compressor_seen`）。
        // `eq_enabled` / `filter_enabled` 单独列出：它们是**其他**参数都不写时唯一的痕迹。
        let recognised = self.eq.seen
            || self.filter_seen
            || self.compressor_seen
            || self.eq_enabled.is_some()
            || self.filter_enabled.is_some()
            || self.params != ChannelStripParams::DEFAULT;
        if !recognised {
            return None;
        }
        let mut params = self.params;
        // EQ 字段：写过的覆盖 `EqParams::default()`（未写的保持平坦）。
        let eq = &mut params.eq;
        if let Some(value) = self.eq.low_gain {
            eq.low_gain = value;
        }
        if let Some(value) = self.eq.low_freq {
            eq.low_freq = value;
        }
        if let Some(value) = self.eq.mid_gain {
            eq.mid_gain = value;
        }
        if let Some(value) = self.eq.mid_freq {
            eq.mid_freq = value;
        }
        if let Some(value) = self.eq.mid_q {
            eq.mid_q = value;
        }
        if let Some(value) = self.eq.high_gain {
            eq.high_gain = value;
        }
        if let Some(value) = self.eq.high_freq {
            eq.high_freq = value;
        }
        // 启用开关：写过的优先，否则按"没写的级不执行"推导（模块文档 §4.1）。
        params.eq_enabled = self.eq_enabled.unwrap_or(self.eq.seen);
        params.filter_enabled = self.filter_enabled.unwrap_or(self.filter_seen);
        params.compressor_enabled = self.compressor_enabled.unwrap_or(self.compressor_seen);
        Some(params)
    }
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

    /// **默认口径**：没有设备 / 没有已识别参数 ⇒ 空链（实时侧整段跳过）。
    #[test]
    fn devices_without_recognised_params_do_not_arm_a_channel_strip() {
        assert!(InsertParams::from_devices(&[]).is_empty());
        // 只上报延迟的效果器（既有夹具的形状）：参数为空。
        assert!(
            InsertParams::from_devices(&[device(DeviceKind::InternalEffect, false, &[])])
                .is_empty()
        );
        // 参数名不认识的第三方效果器：不许猜成通道条。
        // ⚠ `filter_cutoff_hz` **不**是本模块认的名字（模块文档 §4.3 说明了理由：
        // 它正是 `tests/compressor_insert.rs` 的 C0 夹具用来表达"弄不懂的效果器"的名字，
        // 把它收下会让那条既有的逐位一致判据变红 ⇒ 那是接线在改变既有听感）。
        // `filter_slope` 是另一类**不认**的名字。
        assert!(
            InsertParams::from_devices(&[device(
                DeviceKind::InternalEffect,
                false,
                &[("reverb_mix", 0.3), ("filter_slope", 0.7)]
            )])
            .is_empty()
        );
        // 内置乐器（音源）不是插入器件 —— 同一个名字也不认。
        assert!(
            InsertParams::from_devices(&[device(
                DeviceKind::InternalInstrument,
                false,
                &[("ratio", 4.0)]
            )])
            .is_empty()
        );
        // 旁通的效果器就是旁通：不许"取默认参数偷偷接回来"。
        assert!(
            InsertParams::from_devices(&[device(
                DeviceKind::InternalEffect,
                true,
                &[("threshold_db", -20.0), ("ratio", 8.0)]
            )])
            .is_empty()
        );
    }

    /// **命中**：出现的名字覆盖默认值，未出现的字段保持 dsp 的默认值。
    #[test]
    fn recognised_params_override_the_defaults_field_by_field() {
        let insert = InsertParams::from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("threshold", -20.0), ("RATIO", 8.0), ("release", 0.25)],
        )]);
        let params = insert.compressor().expect("必须武装动态级");
        assert_eq!(params.threshold_db.to_bits(), (-20.0f32).to_bits());
        assert_eq!(params.ratio.to_bits(), 8.0f32.to_bits());
        assert_eq!(params.release_s.to_bits(), 0.25f32.to_bits());
        // 未出现的字段 = 默认值（逐位）。
        let default = CompressorParams::DEFAULT;
        assert_eq!(params.knee_db.to_bits(), default.knee_db.to_bits());
        assert_eq!(params.detector_s.to_bits(), default.detector_s.to_bits());
        assert_eq!(params.attack_s.to_bits(), default.attack_s.to_bits());
        assert_eq!(params.makeup_db.to_bits(), default.makeup_db.to_bits());
        // 只有动态参数的设备 ⇒ EQ 与滤波级**不执行**（模块文档 §4.1）。
        let strip = insert.strip().expect("必须武装通道条");
        assert!(!strip.eq_enabled, "没写 EQ 旋钮 ⇒ EQ 级不许启用");
        assert!(!strip.filter_enabled, "没写滤波旋钮 ⇒ 滤波级不许启用");
        assert!(strip.compressor_enabled, "写了动态参数 ⇒ 动态级启用");
    }

    /// 规则 3：第一条**没有**已识别参数的效果器不是来源，继续找下一条。
    /// 规则 5：同一设备内同名参数**后者胜**。
    /// 规则 7：非有限值经 `sanitised` 有定义（不许 `NaN` 进快照）。
    #[test]
    fn source_selection_is_ordered_and_values_are_sanitised() {
        let chain = [
            device(DeviceKind::InternalEffect, false, &[("mix", 0.5)]),
            device(
                DeviceKind::InternalEffect,
                false,
                &[("ratio", 2.0), ("ratio", 3.0), ("threshold_db", f32::NAN)],
            ),
            device(DeviceKind::InternalEffect, false, &[("ratio", 99.0)]),
        ];
        let params = InsertParams::from_devices(&chain)
            .compressor()
            .expect("第二条必须命中");
        assert_eq!(params.ratio.to_bits(), 3.0f32.to_bits(), "同名后者胜");
        assert!(params.threshold_db.is_finite(), "非有限值必须被钳掉");
        assert_eq!(
            params.threshold_db.to_bits(),
            yeban_dsp::compressor::MIN_LEVEL_DB.to_bits(),
            "NaN 归到下界（与 sanitised 同口径）"
        );
    }

    /// **EQ ／ 滤波级只在写了对应旋钮时才启用**（模块文档 §4.1 的表）。
    ///
    /// 这条判据是"沉默地启用平坦 EQ"这类缺陷的机械判据：平坦 EQ 仍会改变低位
    /// （`crates/yeban-dsp/src/channel_strip.rs` §3 实测 `4.566e-5`），
    /// 因此"写了 `eq_low_gain`"与"没写"必须在**启用位**上可区分。
    #[test]
    fn stage_enable_flags_follow_the_written_knobs() {
        // 只有 EQ 旋钮。
        let eq_only = InsertParams::from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("eq_low_gain", 6.0)],
        )])
        .strip()
        .expect("EQ 旋钮是已识别参数");
        assert!(eq_only.eq_enabled);
        assert!(!eq_only.filter_enabled);
        assert!(!eq_only.compressor_enabled);
        assert_eq!(eq_only.eq.low_gain.to_bits(), 6.0f32.to_bits());
        // 未写的 EQ 字段 = 平坦（`EqParams::default`）。
        assert_eq!(eq_only.eq.mid_gain.to_bits(), 0.0f32.to_bits());
        assert_eq!(eq_only.eq.high_gain.to_bits(), 0.0f32.to_bits());

        // 只有滤波旋钮。
        let filter_only = InsertParams::from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("cutoff_hz", 800.0), ("resonance", 0.4)],
        )])
        .strip()
        .expect("滤波旋钮是已识别参数");
        assert!(!filter_only.eq_enabled);
        assert!(filter_only.filter_enabled);
        assert!(!filter_only.compressor_enabled);
        assert_eq!(filter_only.filter.cutoff_hz.to_bits(), 800.0f32.to_bits());

        // EQ ＋ 动态。
        let both = InsertParams::from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("eq_high_gain", -12.0), ("threshold_db", -24.0)],
        )])
        .strip()
        .expect("两类旋钮都是已识别参数");
        assert!(both.eq_enabled);
        assert!(!both.filter_enabled);
        assert!(both.compressor_enabled);

        // 显式开关：写了 `eq_enabled = true` 而没有任何 EQ 增益旋钮 ⇒ EQ 级仍然启用
        // （"写过的优先"）。平坦 EQ 的舍入是**调用方明说要的**。
        let forced = InsertParams::from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("eq_enabled", 1.0)],
        )])
        .strip()
        .expect("开关名也是已识别参数");
        assert!(forced.eq_enabled);
        assert!(!forced.filter_enabled);
        assert!(!forced.compressor_enabled);
    }

    /// 判据：**engine 侧没有第二份压缩器实现**（源码级机械检查）。
    ///
    /// 与 `mixer.rs` 的 `engine_mixer_module_has_no_second_limiter_implementation`
    /// 同款。记号用 `concat!` 拼出来，避免判据自己的字面量命中自己。
    ///
    /// 注入：把 `yeban_dsp::compressor` 的**类型定义**复制进本文件（`struct` + 类型名），
    /// 或在本文件里新增一个 `process_mono` 方法 ⇒ 本判据立即变红。
    /// ⚠ 源码级检查对**本判据自己的注释**也生效：注释里不许出现被禁的字面量
    /// （第一版就是这样自我命中，实测变红后改写了本条注释）。
    #[test]
    fn engine_insert_module_has_no_second_compressor_implementation() {
        let source = include_str!("insert.rs");
        let forbidden = [
            concat!("struct", " Compressor", " {"),
            concat!("impl", " Compressor"),
            concat!("struct", " CompressorParams", " {"),
            concat!("impl", " CompressorParams"),
            concat!("fn", " process_mono("),
            concat!("fn", " gain_db_for("),
            concat!("fn", " output_db_for("),
            concat!("const", " DEFAULT_DETECTOR_S"),
            concat!("const", " POWER_FLOOR"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 insert.rs 里出现了实现记号 `{needle}` —— 这里只允许投影 + `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::compressor::{Compressor, CompressorParams};"),
            "engine 的 insert.rs 必须把 dsp 压缩器的**类型与参数类型**都再导出（C6 的同一性判据靠它）"
        );
    }

    /// 判据：**engine 侧没有第二份通道条实现**（源码级机械检查）。
    ///
    /// 与本文件上一条同款、对准 `yeban_dsp::channel_strip`。注入：在本文件里复制
    /// `ChannelStrip` 的类型定义或它的任何一个逐样本内核 ⇒ 本判据立即变红。
    /// ⚠ 与上一条同样的自我命中风险：本条注释里也**不许**出现被禁字面量。
    #[test]
    fn engine_insert_module_has_no_second_channel_strip_implementation() {
        let source = include_str!("insert.rs");
        let forbidden = [
            concat!("struct", " ChannelStrip", " {"),
            concat!("impl", " ChannelStrip"),
            concat!("struct", " ChannelStripParams", " {"),
            concat!("impl", " ChannelStripParams"),
            concat!("struct", " FilterParams", " {"),
            concat!("impl", " FilterParams"),
            concat!("fn", " render_frame("),
            concat!("fn", " observe_input("),
            concat!("fn", " process_stereo("),
            concat!("const", " EQ_CHUNK_FRAMES"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 insert.rs 里出现了实现记号 `{needle}` —— 这里只允许投影 + `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::channel_strip::{"),
            "engine 的 insert.rs 必须是 dsp 通道条与其参数类型的再导出"
        );
    }
}
