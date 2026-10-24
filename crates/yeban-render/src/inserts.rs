//! **每轨插入链**的离线投影与运行态：[通道条（增益 → EQ → 低通）] ＋ [Freeverb] ＋ [卷积混响]。
//!
//! ## 0. 这一份是**副本** —— 必须下移到 `yeban-dsp`（待办）
//!
//! 投影规则不是本模块发明的，逐条抄自实时引擎：
//!
//! | 内容 | 来源（base `1d60e5eb`） |
//! | :--- | :--- |
//! | `InsertParams::from_devices` 的"首个含已识别参数名的设备是来源"规则 | `crates/yeban-engine/src/insert.rs:704` |
//! | 通道条参数名白名单（增益 / EQ / 滤波 / 动态 / 级开关） | `crates/yeban-engine/src/insert.rs:869-1100` |
//! | 混响参数名白名单（⚠ `reverb_mix` / `reverb_width` **刻意不认**） | `crates/yeban-engine/src/insert.rs:1055-1070` |
//! | 卷积混响参数名白名单 ＋ IR 合成 | `crates/yeban-engine/src/insert.rs:622-654` |
//! | DSP 本体 | `crates/yeban-dsp/src/channel_strip.rs`（通道条；内含 `shaping.rs:123` 的 3 段 EQ）、`reverb.rs:239`（Freeverb）、`convolution_reverb.rs:182` |
//!
//! 为什么是副本而不是直接调用引擎：`crates/yeban-mcp` **不能依赖** `yeban-engine`
//! （`scripts/gates/check_mcp_dependency_direction.py` 的 `FORBIDDEN` 逐字含它），
//! 而本条线的调用方就是 `yeban-mcp` 的离线导出。`yeban-render` 已依赖 `yeban-dsp`，
//! 因此 DSP 本体是**同一份**；只有"参数名 → 器件参数"这段投影被复制。
//!
//! ⚠ **待办（ADR 建议）**：把 `InsertChain` / `from_devices` / 四个白名单搬到
//! `yeban-dsp`（或一个 model 级 `yeban-mix`），让引擎与离线渲染共用**同一条**投影，
//! 本模块只保留运行态。在那之前，**改白名单必须同时改两处**，而两处漂移只有
//! `crates/yeban-render/tests/insert_chain.rs` 的镜像判据能抓住。
//!
//! ## 1. 本切片接了什么 / 没接什么
//!
//! **接**：通道条的**输入增益 → 3 段 EQ → 低通滤波 → 动态（压缩）→ 输出增益**
//! （与引擎同一个 `yeban_dsp::ChannelStrip`）＋ Freeverb ＋ 卷积混响。
//!
//! ### 1.1 ⭐ 压缩级接通（任务单 `#303`；**这是行为变更**）
//!
//! `#303` 之前 [`InsertChain::effective_strip`] 无条件写
//! `params.compressor_enabled = false`（旧 `:357`）⇒ 离线导出里**压缩级整级不执行**，
//! 只有压缩参数的通道条被判成空链。三条前置裁决之后接通：
//!
//! 1. **检波参考点 = 声相之前**（与实时 `crates/yeban-engine/src/rt.rs:1857-1867`
//!    → `:1888-1903` 同序：插入 → PDC → 声相）。离线为此加一条 **pre-pan tap**：
//!    **会执行压缩的纯源轨**把声相从源侧搬到 [`crate::render::OutputStage`]
//!    （插入链之后），源侧 `pan` 取 `(1, 1)` ⇒ 检波器看到的就是声相之前的信号。
//!    ⚠ 总线 / 主总线的声相本来就在输出级 ⇒ 它们的插入点**一直**在声相之前。
//!    ⭐ 这条搬迁**只对会压缩的轨生效**（生产判据 =
//!    [`crate::automation::compressing_nodes_with_lanes`]：静态投影
//!    [`compressing_nodes`] ∪ 非恒等 `compressor_enabled` 泳道，`#309` 起）：
//!    其余轨的声相仍在源侧，**一个落点都不改**。
//! 2. **接上就生效，没有"启用压缩"开关**（与 Logic Pro 一致）：写了一组压缩参数
//!    （`threshold_db` / `ratio` / … 或显式 `compressor_enabled`）⇒ 该级执行；
//!    一个都没写 ⇒ 不执行。
//! 3. **无 lookahead、零延迟**：`yeban_dsp::compressor::Compressor::latency_samples()
//!    == 0`（`compressor.rs:679-690`）⇒ **PDC 一条不动**。
//!
//! **旧工程兼容说明（行为变更，⛔ 不是回归）**：`#303` 之前导出的工程若写了压缩参数，
//! 那台压缩器**从未执行**（母带里没有压缩）；`#303` 起同一份工程**会真的压** ⇒
//! 导出字节与旧版本不同。没有写压缩参数的工程**逐字节不变**（会压缩的轨集合为空）。
//!
//! ⚠ **旧 §1 那句话已作废**：原文写"离线接在声相与求和之后（等功率居中时每声道
//! −3 dB）"—— **"−3 dB"这一半今天已经是假的**：[`crate::automation::centered_pan_gains`]
//! 对 `pan == 0` 返回 `(1.0, 1.0)`（`#255` 之后的裁决，⛔ 0.707），所以居中信号在两条
//! 路上本来就是同一个电平。仍在的差异只有"声相 ≠ 0 时声相乘在哪一级"的浮点结合顺序
//! （见 §2）。
//!
//! **没接**：
//!
//! - **卷积 IR 形状**（`conv_ir_decay*` / `conv_ir_seed`）：构造期烘焙，自动化不落点
//!   （与 `#293` 的边界同款：静态生效 ≠ 可自动化）；
//! - **未知旋钮 / 外部器件**：同一条如实登记口径。
//!
//! ⭐ **压缩参数的自动化自 `#304` 起接通**：压缩的 14 个拼写（8 个字段）并入了
//! [`StripField::SPELLINGS`]，成为**唯一名字表**的一部分（⛔ 不再有第二张
//! `STRIP_COMPRESSOR_SPELLINGS`）。`DeviceParam` 泳道改的是同一台
//! `ChannelStrip` 的压缩参数；`Compressor::set_params` 的"弹道状态不重置"是这条
//! 接线的依据（逐字见 `yeban-dsp/src/compressor.rs`）。⚠ 两条边界（一条**已修**）：
//! ① 静态 `compressor_enabled == false` 时，非 `compressor_enabled` 的压缩参数泳道
//! 仍会**报 applied 但听不出**（与 `eq_enabled: 0` ＋ EQ 参数泳道同款）；
//! ② 把静态关闭的压缩器从 `0` 自动到 `1` 时，pre-pan tap 的预扫描曾只看静态位
//! （`#304` 登记）；⭐ `#309` 起生产判据改为
//! [`crate::automation::compressing_nodes_with_lanes`]（静态 ∪ 非恒等
//! `compressor_enabled` 泳道）⇒ 该轨的检波器仍在声相之前，⛔ 不再是登记边界。
//! - **主总线插入**（⚠ 本节在任务单 `#173` **已接通**，原文见 §1 的修订段）：
//!   本模块的 [`crate::render::RenderPlan`] 现在给**每个**节点挂链，**包括 master**。
//!   - **为什么改**：`#173` 之前实时引擎的逐轨循环**跳过 master**
//!     （`crates/yeban-engine/src/rt.rs` 的 `render_branch` 只处理轨道），而离线侧
//!     也把 master 的链写成 `None` ⇒ 主总线上挂设备**两条路都听不到**。
//!   - **改法（实时）**：主总线在汇流之后（3a⁗⁗）跑**同一条**链：立体声和
//!     `(L+R)·0.5` 下混 → 通道条（增益 / EQ / 滤波）→ 混响 → 卷积 → 复制回两路（裁决 (甲)，与 `#158`
//!     的总线插入同一条）。**改法（离线）**：`render_node` 的"求和之后"那一行对
//!     master 也执行 ⇒ 真立体声插入。
//!   - **仍存在的两路差异**（如实登记，⛔ 不是沉默）：实时是单声道插入，
//!     离线是立体声插入 ⇒ 主总线上**只要**有已武装器件（哪怕湿声为 0），
//!     实时母线的左右差异就会被下混折掉；居中信号逐位等价，硬声相信号不同。
//!     这与逐轨/总线插入的 `#155`／`#158` 差异**同族**。
//!
//! ## 2. 顺序差异（⚠ 必须登记）
//!
//! ```text
//! 实时：声源 → 插入（单声道）→ PDC → 声相 → 母线求和
//! 离线：声源 → [会压缩的纯源轨：插入 → 声相；其余轨：声相 → 插入] → 母线求和
//! ```
//!
//! - **增益 / EQ / 滤波逐声道线性等价**：声相是每声道的常数增益，线性时不变器件
//!   与常数乘法可交换（`F(p·x) = p·F(x)`），所以这三级的**位置**差异不改变可听结果
//!   （只差浮点舍入顺序）。低通的两声道各有一份状态，居中信号下与实时的单声道
//!   滤波器逐样本同值。
//! - **压缩的检波参考点已对齐**（§1.1 的 pre-pan tap）：会压缩的纯源轨插入在声相
//!   之前 ⇒ 检波电平与实时同一个口径（声相之前的能量 `1.0`；居中时 `pan = (1,1)`
//!   本来就逐位相同）。⚠ 剩余差异两条，都**只影响会压缩的轨**：
//!   (a) 实时是**单声道**插入、离线是**立体声联动**插入（检波 `max(l², r²)`）⇒
//!   逐轨单声道源在居中/硬声相下都与实时同解，只有**总线立体声内容**才不同；
//!   (b) 同一节点有 MIDI ＋ 音频片段时，**会压缩的**轨把声相乘在**求和之后**
//!   （整条通道），而其余轨仍是逐源乘 ⇒ 两者只差浮点结合顺序（实测 ≤ 1 ulp；
//!   读数见 `crates/yeban-app/tests/compressor_export.rs`）。
//! - **Freeverb 的干湿比与宽度会有差异**：实时侧把**声相之前**的单声道信号同时喂给
//!   L/R 再取中值；离线侧拿到的是**声相之后**的立体声（可能左右不同）。湿路径的输入
//!   因此不同，`width` 的展开也不同。这是**听得见的口径差**，不是 bug；人耳验收时要
//!   拿同一条轨的监听与导出对照。
//! - **卷积混响**同款：实时侧是把单声道复制成 L/R 再取中值；离线侧是真立体声输入。
//!   两条对角通路都是同一条 IR，交叉通路为零 ⇒ 单声道源在居中声相下**逐位等价**，
//!   有声相时湿路的左右分配不同。
//!
//! ## 3. 默认口径（逐位不退化）
//!
//! 没有设备 / 全部旁通 / 参数名一个都不认识 ⇒ [`InsertChain::is_empty`] ⇒ 编译期
//! **不挂运行态**，`render_node` 整段跳过 ⇒ 输出与接线前逐位相同（`C0` 口径）。
//! 设备写了 `eq_enabled: 0` / `reverb_wet: 0` ⇒ 链里的该级为 `None` ⇒ 同样整级跳过
//! （器件状态**不推进**，与 `Reverb::is_active` / `ConvolutionReverb::is_active` 同口径）。
//! 例外是 §3.1 的强制实例化：有**非恒等**泳道时该族器件仍会被建出来（否则块边界
//! 没有落点），但恒等泳道下这条默认口径逐字不变。
//!
//! ### 3.1 设备参数自动化的**编译期强制实例化**（任务单 `#266`/`#293`）
//!
//! 一条 `DeviceParam` 泳道（`#266` 只接 `output_gain_db`；`#267` 接通道条整组；
//! `#293` 接 Freeverb 与卷积的标量字段）要改的是**器件的参数**，不是增益乘子。
//!
//! - **通道条**：若静态参数恰好是"全恒等"（`0 dB` ＋ 三级全关），
//!   [`InsertChain::effective_strip`] 会返回 `None` ⇒ 编译期根本没有器件可改。
//!   因此调用方在**编译计划之前**用 [`ForcedInsertInstances::strip`] 声明"本轨有非恒等
//!   的通道条泳道" ⇒ 该轨的通道条被**强制实例化**（参数取投影值）。
//! - **Freeverb / 卷积**：静态 `reverb_wet: 0` / `conv_wet: 0` 时，投影里的器件
//!   `is_active()` 为假，[`InsertRuntime::new`] 会把它丢掉 ⇒ 一条非恒等泳道会
//!   `attach` 成功、被报成 applied，却**静默无效**。因此 [`ForcedInsertInstances::reverb`]
//!   / [`ForcedInsertInstances::convolution`] 让运行态仍然建出器件（IR 仍是构造期烘好的
//!   那一份；`set_params` 零分配）。
//!
//! 恒等泳道（无点 / `read_enabled:false` / 处处等于静态值）**不**强实例化 ⇒ 产物
//! 与没有泳道逐字节相同。三族**分开**置位：混响泳道不会白实例化一条恒等通道条。
//!
//! ## 4. 音频线程边界
//!
//! [`InsertRuntime`] 的全部缓冲在构造期分配（[`InsertRuntime::new`]）。逐块
//! [`InsertRuntime::process`] 只有环形缓冲读写与乘加：**零分配、零锁、零 I/O**。

use std::collections::{BTreeMap, BTreeSet};

use yeban_dsp::channel_strip::{ChannelStrip, ChannelStripParams};
use yeban_dsp::convolution_reverb::{
    ConvolutionReverb, ConvolutionReverbParams, MAX_IR_GAIN_DB, MAX_PRE_DELAY_SECONDS,
    MIN_IR_GAIN_DB,
};
use yeban_dsp::noise::Rng;
use yeban_dsp::reverb::{Reverb, ReverbParams};
use yeban_dsp::shaping::EqParams;
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, TrackV3, YebanProjectV1};

/// 卷积混响 IR 的长度除数（帧数 = 采样率 ÷ 10）。
///
/// 与实时引擎的 `CONV_IR_SAMPLE_RATE_DIVISOR` **同值**（`crates/yeban-engine/src/insert.rs`）：
/// `48 kHz ⇒ 4 800 帧 = 0.1 s`。它是常量而不是旋钮 —— 换长度会改变两边的尾音形状。
const CONV_IR_SAMPLE_RATE_DIVISOR: usize = 10;

/// IR 衰减时间常数的缺省值（秒）＝ `1 / e` 处。与引擎同值。
const CONV_IR_DEFAULT_DECAY_S: f32 = 0.35;

/// IR 衰减时间常数的合法下界（秒）。与引擎同值。
const CONV_IR_MIN_DECAY_S: f32 = 0.005;

/// IR 衰减时间常数的合法上界（秒）。与引擎同值。
const CONV_IR_MAX_DECAY_S: f32 = 5.0;

/// IR 随机种子的缺省值。与引擎同值（非零，避免"缺省"与"显式 0"不可区分）。
const CONV_IR_DEFAULT_SEED: f32 = 1.0;

/// 卷积混响的 IR 帧数：`采样率 ÷ 10`，至少 1 帧。与引擎的
/// `convolution_ir_frames` 逐字同式。
#[must_use]
pub const fn convolution_ir_frames(sample_rate: u32) -> usize {
    let frames = sample_rate as usize / CONV_IR_SAMPLE_RATE_DIVISOR;
    if frames == 0 { 1 } else { frames }
}

/// Freeverb 预延迟的上界（秒），与 `yeban_dsp::reverb::Reverb::set_params` 里的
/// `0.1` **同值**。
///
/// ⚠ 那一侧只在函数体里写常量、没有 `pub` 常量可给，因此这里只能镜像；镜像
/// 由 `reverb_field_clamp_is_the_device_clamp` 用**真实器件**的网格对账钉住
/// （⛔ 不是一份会静默漂移的第二套钳制）。
const REVERB_MAX_PREDELAY_S: f32 = 0.1;

/// 构造期合成一条单声道 IR：白噪声 × 指数衰减包络，再峰值归一化。
///
/// 与引擎的 `synthesise_impulse_response` 逐字同式（同一个 `yeban_dsp::noise::Rng`、
/// 同一个 `libm::expf`）⇒ 同一组旋钮在两条路径上得到**同一条** IR。
fn synthesise_impulse_response(
    frames: usize,
    decay_s: f32,
    sample_rate: u32,
    seed: u32,
) -> Vec<f32> {
    let decay_frames = (decay_s * sample_rate as f32).max(1.0);
    let mut rng = Rng::new(seed);
    let mut ir = Vec::with_capacity(frames);
    for index in 0..frames {
        let envelope = libm::expf(-(index as f32) / decay_frames);
        ir.push(rng.next_bipolar() * envelope);
    }
    let peak = ir.iter().fold(0.0f32, |acc, value| acc.max(value.abs()));
    if peak > 0.0 && peak.is_finite() {
        for value in &mut ir {
            *value /= peak;
        }
    }
    ir
}

/// 一条轨的卷积混响计划：四个标量参数 ＋ 一份**已合成**的 IR。
#[derive(Clone, Debug, PartialEq)]
pub struct ConvolutionPlan {
    /// 器件的四个标量参数。
    params: ConvolutionReverbParams,
    /// 单声道 IR（`h_LL = h_RR = ir`，交叉通路为零）。
    ir: Vec<f32>,
}

impl ConvolutionPlan {
    /// 器件的四个标量参数。
    #[must_use]
    pub const fn params(&self) -> ConvolutionReverbParams {
        self.params
    }

    /// 单声道 IR。
    #[must_use]
    pub fn ir(&self) -> &[f32] {
        &self.ir
    }
}

/// 一条轨的**插入链投影**（通道条、Freeverb、卷积混响）。
///
/// 三件**各自独立**取来源（与引擎 `InsertParams::from_devices` 同口径）：同一台设备
/// 可以同时是三者或其中之一的来源。`None` 表示"本轨没有这一件" ⇒ 运行态整级跳过。
#[derive(Clone, Debug, PartialEq)]
pub struct InsertChain {
    /// 通道条参数（增益 / EQ / 滤波 / 压缩）；`None` = 没有通道条来源。
    ///
    /// ⚠ 投影**如实**保留引擎给出的 `compressor_enabled`；`#303` 起运行态**照它执行**
    /// （之前是无条件关掉）。是否真的执行某一级由 [`Self::is_empty`] 与运行态共同决定。
    strip: Option<ChannelStripParams>,
    /// Freeverb 参数；`None` = 本轨没有混响。
    reverb: Option<ReverbParams>,
    /// 卷积混响计划；`None` = 本轨没有卷积混响。
    convolution: Option<ConvolutionPlan>,
    /// **编译期强制实例化通道条**（任务单 `#266`）。
    ///
    /// `true` = 本轨有**非恒等**的 `DeviceParam` 泳道 ⇒ 即使投影结果是"全恒等"
    /// （`0 dB` ＋ 三级全关，[`Self::effective_strip`] 本来会返回 `None`），也必须有
    /// 一台器件让块边界的 `set_params` 有落点。恒等泳道**不**置这一位。
    force_strip: bool,
    /// **编译期强制实例化 Freeverb**（`#293`）。
    ///
    /// `true` = 本轨有**非恒等**的混响 `DeviceParam` 泳道。投影里 `reverb` 已经是
    /// `Some`（泳道目标的那个参数名就是 `reverb_of` 的来源），但若静态值是
    /// `reverb_wet: 0`，[`InsertRuntime::new`] 会因 `is_active()` 为假把器件丢掉 ⇒
    /// 块边界没有东西可改（“报 applied 却静默无效”）。这一位让运行态仍然建出器件。
    force_reverb: bool,
    /// **编译期强制实例化卷积混响**（`#293`）。与 [`Self::force_reverb`] 同理：
    /// `conv_wet: 0` 的静态链会让 `InsertRuntime` 丢掉器件。
    force_convolution: bool,
}

impl InsertChain {
    /// **空链**：本轨没有可执行的插入器件。
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            strip: None,
            reverb: None,
            convolution: None,
            force_strip: false,
            force_reverb: false,
            force_convolution: false,
        }
    }

    /// 从模型层的设备链投影（**构造期**，允许分配：卷积的 IR 在这里合成）。
    ///
    /// 规则与引擎 `InsertParams::from_devices` 逐条一致：
    ///
    /// 1. 只看**非旁通**的 [`DeviceKind::InternalEffect`]；
    /// 2. 三件各自取**第一个**含该件已识别参数名的设备（先命中者不被后来者覆盖）；
    /// 3. 未出现的字段保持 `yeban-dsp` 的默认值；
    /// 4. 同名参数**后者胜**；参数名大小写不敏感；
    /// 5. 没写旋钮的级不执行（`eq_enabled` 等开关写过的优先）。
    #[must_use]
    pub fn from_devices(devices: &[DeviceDefinition], sample_rate: u32) -> Self {
        let mut strip: Option<ChannelStripParams> = None;
        let mut reverb: Option<ReverbParams> = None;
        let mut convolution: Option<ConvolutionPlan> = None;
        for device in devices {
            if device.bypassed || device.kind != DeviceKind::InternalEffect {
                continue;
            }
            if strip.is_none() {
                strip = strip_of(device);
            }
            if reverb.is_none() {
                reverb = reverb_of(device);
            }
            if convolution.is_none() {
                convolution = convolution_of(device, sample_rate);
            }
            if strip.is_some() && reverb.is_some() && convolution.is_some() {
                break;
            }
        }
        Self {
            strip: strip.map(ChannelStripParams::sanitised),
            reverb,
            convolution,
            force_strip: false,
            force_reverb: false,
            force_convolution: false,
        }
    }

    /// 置"强制实例化通道条"位（任务单 `#266`；见模块头 §3.1）。
    ///
    /// 调用方在**编译渲染计划之前**按"本轨有没有非恒等的设备参数泳道"设置它；
    /// `project_inserts_with_forced_instances` 是唯一的生产调用者。
    #[must_use]
    pub const fn with_forced_strip(mut self, force: bool) -> Self {
        self.force_strip = force;
        self
    }

    /// 置"强制实例化 Freeverb"位（任务单 `#293`）。见 [`Self::force_reverb`]。
    #[must_use]
    pub const fn with_forced_reverb(mut self, force: bool) -> Self {
        self.force_reverb = force;
        self
    }

    /// 置"强制实例化卷积混响"位（任务单 `#293`）。见 [`Self::force_convolution`]。
    #[must_use]
    pub const fn with_forced_convolution(mut self, force: bool) -> Self {
        self.force_convolution = force;
        self
    }

    /// 本链是否被编译期强制实例化了通道条（读数用）。
    #[must_use]
    pub const fn is_forced_strip(&self) -> bool {
        self.force_strip
    }

    /// 本链是否被编译期强制实例化了 Freeverb（读数用）。
    #[must_use]
    pub const fn is_forced_reverb(&self) -> bool {
        self.force_reverb
    }

    /// 本链是否被编译期强制实例化了卷积混响（读数用）。
    #[must_use]
    pub const fn is_forced_convolution(&self) -> bool {
        self.force_convolution
    }

    /// 本链是否为空（没有任何**离线会执行**的器件）。
    ///
    /// ⭐ `#303`：**只有压缩参数的通道条算非空** —— 压缩级现在真的执行（§1.1）。
    /// 反过来说，`compressor_enabled: 0` 且增益/EQ/滤波全关仍是空链（逐位不退化）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.effective_strip().is_none() && self.reverb.is_none() && self.convolution.is_none()
    }

    /// 本链是否会执行**压缩级**（读数用：pre-pan tap 的判据就是它）。
    ///
    /// `true` 的条件：投影里有通道条、且 `compressor_enabled` 为真（写了一组压缩参数，
    /// 或显式 `compressor_enabled != 0`）—— 与 [`Self::effective_strip`] 同一份参数，
    /// ⛔ 不是第二套判定。
    #[must_use]
    pub fn compresses(&self) -> bool {
        self.effective_strip()
            .is_some_and(|params| params.compressor_enabled)
    }

    /// 投影出来的通道条参数（判据用：把"将要交给器件的那个数"变成可读的）。
    ///
    /// ⚠ 这里保留投影的**原样**（包括 `compressor_enabled`）；`#303` 起运行态照它执行
    /// （之前是无条件关掉）。
    #[must_use]
    pub const fn strip(&self) -> Option<ChannelStripParams> {
        self.strip
    }

    /// 实际会交给运行态的通道条参数：`compressor_enabled` **照投影执行**，
    /// 且"三级全关 + 增益 0 dB"⇒ `None`。
    ///
    /// 例外：[`Self::force_strip`] 为真时**恒返回 `Some`** —— 那是"有设备参数泳道要改
    /// 这台器件"的编译期声明（任务单 `#266`），此时"全恒等参数"不是"没有器件"，
    /// 而是"器件此刻是恒等、等泳道来改它"。
    fn effective_strip(&self) -> Option<ChannelStripParams> {
        let params = match self.strip {
            Some(params) => params,
            None if self.force_strip => forced_identity_strip(),
            None => return None,
        };
        if self.force_strip {
            return Some(params);
        }
        let identity = params.input_gain_db == 0.0
            && params.output_gain_db == 0.0
            && !params.eq_enabled
            && !params.filter_enabled
            && !params.compressor_enabled;
        if identity { None } else { Some(params) }
    }

    /// 投影出来的 EQ 参数（判据用；EQ 级未启用时为 `None`）。
    #[must_use]
    pub const fn eq(&self) -> Option<EqParams> {
        match self.strip {
            Some(params) if params.eq_enabled => Some(params.eq),
            _ => None,
        }
    }

    /// 投影出来的混响参数（判据用）。
    #[must_use]
    pub const fn reverb(&self) -> Option<ReverbParams> {
        self.reverb
    }

    /// 投影出来的卷积计划（判据用）。
    #[must_use]
    pub const fn convolution(&self) -> Option<&ConvolutionPlan> {
        self.convolution.as_ref()
    }
}

impl Default for InsertChain {
    fn default() -> Self {
        Self::empty()
    }
}

/// 把整个工程的每轨设备链投成"节点 → 插入链"（**只收录非空链**）。
///
/// 调用方把这张表交给 [`crate::render::RenderPlan::compile_with_latencies_and_inserts`]。
/// 本函数**不剔除任何节点**（包括 master）：`#173` 起渲染计划对**每个**节点挂链，
/// 主总线与其它总线走同一条"求和之后 → 插入"⇒ 这里若偷偷剔掉 master，导出里
/// 主总线的设备就又听不到了。⛔ 本函数不做"猜主总线"的第二套判断。
#[must_use]
pub fn project_inserts(
    tracks: &BTreeMap<EntityId, TrackV3>,
    sample_rate: u32,
) -> BTreeMap<EntityId, InsertChain> {
    project_inserts_with_forced_instances(tracks, sample_rate, &ForcedInsertInstances::default())
}

/// 编译期强制实例化的**按器件族**集合（任务单 `#293`；`#266` 的通道条是其中一族）。
///
/// 由 `yeban_render::automation::forced_insert_instances` 从“非恒等的 `DeviceParam`
/// 泳道”算出来 —— 两个导出（MCP / app）用**同一份**扫描器。空集等价于
/// [`project_inserts`]（逐字节不退化）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForcedInsertInstances {
    /// 需要强制实例化通道条的轨。
    pub strip: BTreeSet<EntityId>,
    /// 需要强制实例化 Freeverb 的轨。
    pub reverb: BTreeSet<EntityId>,
    /// 需要强制实例化卷积混响的轨。
    pub convolution: BTreeSet<EntityId>,
}

impl ForcedInsertInstances {
    /// 三族都空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.strip.is_empty() && self.reverb.is_empty() && self.convolution.is_empty()
    }
}

/// 与 [`project_inserts`] 相同，但按 [`ForcedInsertInstances`] **强制实例化**对应器件。
///
/// 理由见模块头 §3.1（通道条）与本单 `#293`（混响/卷积的 `wet = 0` 静态链）：
/// 恒等泳道**不**进集合 ⇒ 产物与没有泳道逐字节相同。
#[must_use]
pub fn project_inserts_with_forced_instances(
    tracks: &BTreeMap<EntityId, TrackV3>,
    sample_rate: u32,
    forced: &ForcedInsertInstances,
) -> BTreeMap<EntityId, InsertChain> {
    tracks
        .iter()
        .filter_map(|(&id, track)| {
            let chain = InsertChain::from_devices(&track.devices, sample_rate)
                .with_forced_strip(forced.strip.contains(&id))
                .with_forced_reverb(forced.reverb.contains(&id))
                .with_forced_convolution(forced.convolution.contains(&id));
            if chain.is_empty() {
                None
            } else {
                Some((id, chain))
            }
        })
        .collect()
}

/// 与 [`project_inserts`] 相同，但给 `forced_strip` 里的每条轨**强制实例化通道条**
/// （任务单 `#266`；理由见模块头 §3.1）。
///
/// 这是 [`project_inserts_with_forced_instances`] 的**只含通道条**的包装：保留它
/// 是为了既有调用点/判据的签名不变；混响与卷积的强制位由新函数承担。
#[must_use]
pub fn project_inserts_with_forced_strips(
    tracks: &BTreeMap<EntityId, TrackV3>,
    sample_rate: u32,
    forced_strip: &BTreeSet<EntityId>,
) -> BTreeMap<EntityId, InsertChain> {
    let forced = ForcedInsertInstances {
        strip: forced_strip.clone(),
        ..ForcedInsertInstances::default()
    };
    project_inserts_with_forced_instances(tracks, sample_rate, &forced)
}

/// 本工程里**静态投影**会执行压缩级的节点集合（任务单 `#303` 的 pre-pan tap 预扫描）。
///
/// ⚠ 自 `#309` 起这**不是**生产判据的完整答案：pre-pan tap 还要看**非恒等
/// `compressor_enabled` 泳道**（静态关的压缩器可以被泳道打开）。生产唯一入口是
/// [`crate::automation::compressing_nodes_with_lanes`]，它把本函数（静态那一半）与
/// 泳道那一半取并集；本函数保留为"静态投影"的可读投影与判据的对照。
///
/// ## 为什么需要"预扫描"
///
/// 两个导出（app 的 `render_core` / MCP 的 `domain::render`）必须在**构造音源之前**
/// 知道"哪些轨会压缩" —— 因为"检波器看声相之前"这条裁决要求把**这些轨**的声相从
/// 源侧搬到 `RenderPlan` 的输出级。插入链本身在音源之后才投影，所以这里用
/// **同一份投影入口**（[`strip_source_params`]，与 `from_devices` 同一条"首个来源"
/// 规则）做一次轻量预扫描：⛔ 不建运行态、⛔ 不烘卷积 IR。
///
/// 返回集合与 [`InsertChain::compresses`] 在**同一份工程 + 同一份设备链**上必须同解；
/// 判据 `compressing_nodes_agree_with_the_compiled_chains`（`tests/insert_chain.rs`）钉住。
#[must_use]
pub fn compressing_nodes(tracks: &BTreeMap<EntityId, TrackV3>) -> BTreeSet<EntityId> {
    tracks
        .iter()
        .filter(|(_, track)| {
            strip_source_params(track).is_some_and(|params| params.compressor_enabled)
        })
        .map(|(&id, _)| id)
        .collect()
}

/// 本轨是否仍有**离线路径没有消费**的设备参数（MCP 的 `deviceChainDsp` 登记口径）。
///
/// 返回 `true` 的条件（任一成立）：
///
/// - 有**非旁通**设备不是 [`DeviceKind::InternalEffect`]（外部插件 / 内置乐器；
///   后者是音源，其参数由音源负责）；
/// - 有**非旁通**设备一个参数都没写（"没有参数可求值"不是"已经求值"—— 与既有
///   MCP 判据的口径一致，保留 `deviceChainDsp` 登记）；
/// - 有**非旁通**设备的任一参数名不在本切片的白名单里（未知第三方旋钮 / 刻意不认的
///   `filter_cutoff_hz` / `reverb_mix` / `reverb_width`）。
///
/// 全部非旁通设备都是本切片认识并**真的执行**的内置效果器参数 ⇒ `false` ⇒ MCP
/// 不再登记 `deviceChainDsp`（报一个已经做到的键就是假话）。增益 / EQ / 滤波 / 压缩
/// 都已被通道条执行（压缩自 `#303` 起，见 §1.1）⇒ 它们的名字**不再**登记。
#[must_use]
pub fn track_has_unhandled_dsp(track: &TrackV3) -> bool {
    track.devices.iter().any(|device| {
        if device.bypassed {
            return false;
        }
        if device.kind != DeviceKind::InternalEffect || device.params.is_empty() {
            return true;
        }
        device
            .params
            .iter()
            .any(|param| !is_supported_param(&param.name.to_ascii_lowercase()))
    })
}

/// 本切片**消费**的参数名（小写）全集。
///
/// 它**不是**另一张表：名字来自 [`DeviceParamField::from_name`]（通道条 — 含自
/// `#304` 起的压缩 14 拼写 — / 混响 / 卷积标量的**同一张**拼写表）＋ 三个
/// **构造期** IR 形状名字（`conv_ir_decay*` / `conv_ir_seed`，静态投影会消费它们，
/// 但自动化不落点）。
///
/// ⚠ 自 `#304` 起**压缩不再有第二张表**：14 个拼写住在 [`StripField::SPELLINGS`]，
/// 静态投影（[`StripProjection::apply`]）与自动化（`DeviceParamField::from_name`）
/// 读的是同一组名字。刻意**不**在里面的仍是 `reverb_mix` / `reverb_width`
/// （引擎同样不认）。写这些名字的设备会被 [`track_has_unhandled_dsp`] 如实登记。
fn is_supported_param(name: &str) -> bool {
    DeviceParamField::from_name(name).is_some()
        || matches!(name, "conv_ir_decay_s" | "conv_ir_decay" | "conv_ir_seed")
}

/// 本轨通道条的**来源设备下标**：`from_devices` 的"首个含已识别通道条名字的设备"规则。
///
/// 与 [`InsertChain::from_devices`] 同式：跳过**旁通**设备与非 [`DeviceKind::InternalEffect`]
/// 设备，再要求 `strip_of` 认出一组通道条名字。`None` = 本轨没有通道条来源。
///
/// 设备参数自动化（任务单 `#266`）只能落在**这一台**设备上：`AutomationTarget::DeviceParam`
/// 的 `slot_index` 若是别的下标，那个参数**不是**进 DSP 的那一份 ⇒ 不落点
/// （由 [`crate::automation::device_param_lane_is_applicable`] 如实回答）。
#[must_use]
pub fn strip_source_slot(track: &TrackV3) -> Option<usize> {
    track.devices.iter().position(|device| {
        !device.bypassed && device.kind == DeviceKind::InternalEffect && strip_of(device).is_some()
    })
}

/// 本轨通道条**来源设备**的投影参数（已 `sanitised`，与 `from_devices` 交给器件的同一份）。
#[must_use]
pub fn strip_source_params(track: &TrackV3) -> Option<ChannelStripParams> {
    let slot = strip_source_slot(track)?;
    strip_of(&track.devices[slot]).map(ChannelStripParams::sanitised)
}

/// 本轨 **Freeverb 来源设备**的下标（`from_devices` 的“首个含已识别混响名字的设备”）。
///
/// 与 [`strip_source_slot`] 同一条规则；自动化只能落在这一台上，指向别的下标
/// 那个参数**不是**进 DSP 的那一份。
#[must_use]
pub fn reverb_source_slot(track: &TrackV3) -> Option<usize> {
    track.devices.iter().position(|device| {
        !device.bypassed && device.kind == DeviceKind::InternalEffect && reverb_of(device).is_some()
    })
}

/// 本轨 Freeverb **来源设备**的投影参数（未 sanitised；器件 `set_params` 再钳制）。
#[must_use]
pub fn reverb_source_params(track: &TrackV3) -> Option<ReverbParams> {
    let slot = reverb_source_slot(track)?;
    reverb_of(&track.devices[slot])
}

/// 本轨 **卷积混响来源设备**的下标（`from_devices` 的“首个含已识别卷积名字的设备”）。
#[must_use]
pub fn convolution_source_slot(track: &TrackV3) -> Option<usize> {
    track.devices.iter().position(|device| {
        !device.bypassed
            && device.kind == DeviceKind::InternalEffect
            && convolution_projection(device).is_some()
    })
}

/// 本轨卷积混响**来源设备**的四个标量参数（不合成 IR；IR 形状由静态投影烘）。
#[must_use]
pub fn convolution_source_params(track: &TrackV3) -> Option<ConvolutionReverbParams> {
    let slot = convolution_source_slot(track)?;
    convolution_projection(&track.devices[slot]).map(|(params, _decay_s, _seed)| params)
}

/// 通道条**实际会用的**输出增益值（`dB`）—— 与器件内部同一次钳制
/// （[`ChannelStripParams::sanitised`]，非有限值归 `0 dB`）。
///
/// 设备参数自动化的恒等判定必须比这个值，而不是比原始参数：`-100 dB` 与
/// `-60 dB` 交给器件是**同一个值**，把它们当"不同"会让恒等快路径漏掉一次
/// 不必要的器件实例化。
/// 任务单 `#267` 起它只是 [`StripField::sanitise`] 的一个特例（⛔ 不是第二套钳制）。
#[must_use]
pub fn sanitised_output_gain_db(value: f32) -> f32 {
    StripField::OutputGainDb.sanitise(value)
}

/// 通道条上**可被设备参数自动化落点**的字段（任务单 `#267`；`#304` 扩压缩）。
///
/// 与 [`StripProjection::apply`] 的**通道条分支一一对应**：同一组名字、同一个别名表、
/// 同一个目标字段。⛔ 刻意**不含**：
///
/// - **混响 / 卷积**（`reverb_*` / `conv_*`）：它们不是**通道条**字段，而是
///   [`ReverbField`] / [`ConvolutionField`] 的落点（`#293`）。其中卷积的 IR 形状
///   （`conv_ir_decay*` / `conv_ir_seed`）是构造期烘焙、不可自动化；其余标量由对应
///   器件的 `set_params` 在块边界改（⛔ 不是 `ChannelStrip::set_params`）；
/// - 未知旋钮。
///
/// ⭐ **压缩 8 字段自 `#304` 起在本表里**（14 个拼写：`compressor_enabled`、
/// `threshold_db` / `threshold`、`ratio`、`knee_db` / `knee`、`detector_s` /
/// `detector`、`attack_s` / `attack`、`release_s` / `release`、`makeup_db` /
/// `makeup`）。它们与增益 / EQ / 滤波共用**同一台** `ChannelStrip`：块边界写进
/// `ChannelStripParams`，`ChannelStrip::set_params` 转给 `Compressor::set_params`
/// （⚠ 该函数**不清弹道状态**）。`compressor_enabled` 是这一级的开关字段；
/// 静态关闭时，其它压缩字段的泳道仍会报 applied 但**听不出**（与 `eq_enabled: 0`
/// ＋ EQ 参数泳道同款，`#304` 明确登记）。
///
/// ## 单位（原生绝对值，⛔ 不归一化）
///
/// 字段名自带单位：`_db` = dB、`_hz` = Hz、`_s` = 秒、`resonance` / `drive` /
/// `mid_q` / `ratio` = 旋钮原值。
/// 开关字段（[`Self::EqEnabled`] / [`Self::FilterEnabled`] /
/// [`Self::CompressorEnabled`]）的自动化值是 `0.0` = 假、其它 = 真（与 [`flag`] 的
/// "非零即真"同式）—— 读数与写入都用这个口径。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StripField {
    /// 输入增益（trim，dB）。`input_gain_db` / `input_gain` / `trim_db`。
    InputGainDb,
    /// 输出增益（推子，dB）。`output_gain_db` / `output_gain` / `fader_db`。
    OutputGainDb,
    /// EQ 级开关。`eq_enabled`。
    EqEnabled,
    /// 滤波级开关。`filter_enabled`。
    FilterEnabled,
    /// EQ 低频增益（dB）。`eq_low_gain` / `low_gain`。
    EqLowGain,
    /// EQ 低频转折频率（Hz）。`eq_low_freq` / `low_freq`。
    EqLowFreq,
    /// EQ 中频增益（dB）。`eq_mid_gain` / `mid_gain`。
    EqMidGain,
    /// EQ 中频中心频率（Hz）。`eq_mid_freq` / `mid_freq`。
    EqMidFreq,
    /// EQ 中频 Q。`eq_mid_q` / `mid_q`。
    EqMidQ,
    /// EQ 高频增益（dB）。`eq_high_gain` / `high_gain`。
    EqHighGain,
    /// EQ 高频转折频率（Hz）。`eq_high_freq` / `high_freq`。
    EqHighFreq,
    /// 低通截止频率（Hz）。`cutoff_hz`（⚠ `filter_cutoff_hz` 与引擎一样**不认**）。
    CutoffHz,
    /// 低通谐振。`resonance` / `filter_resonance`。
    Resonance,
    /// 低通驱动。`drive` / `filter_drive`。
    Drive,
    /// 压缩级开关（`0.0` = 关，其它 = 开）。`compressor_enabled`。
    CompressorEnabled,
    /// 压缩阈值（dBFS）。`threshold_db` / `threshold`。
    ThresholdDb,
    /// 压缩比率（`≥ 1`）。`ratio`。
    Ratio,
    /// 压缩软膝宽度（dB）。`knee_db` / `knee`。
    KneeDb,
    /// 检波器平均窗口（秒）。`detector_s` / `detector`。
    DetectorS,
    /// 起振时间常数（秒）。`attack_s` / `attack`。
    AttackS,
    /// 释放时间常数（秒）。`release_s` / `release`。
    ReleaseS,
    /// makeup 补偿增益（dB）。`makeup_db` / `makeup`。
    MakeupDb,
}

impl StripField {
    /// 全部字段（每个字段的**规范名**见 [`Self::canonical_name`]）。
    pub const ALL: &'static [Self] = &[
        Self::InputGainDb,
        Self::OutputGainDb,
        Self::EqEnabled,
        Self::FilterEnabled,
        Self::EqLowGain,
        Self::EqLowFreq,
        Self::EqMidGain,
        Self::EqMidFreq,
        Self::EqMidQ,
        Self::EqHighGain,
        Self::EqHighFreq,
        Self::CutoffHz,
        Self::Resonance,
        Self::Drive,
        Self::CompressorEnabled,
        Self::ThresholdDb,
        Self::Ratio,
        Self::KneeDb,
        Self::DetectorS,
        Self::AttackS,
        Self::ReleaseS,
        Self::MakeupDb,
    ];

    /// **全部拼写** → 字段（`41` 条；这是"哪个名字能落点"的**唯一**名字表）。
    ///
    /// 与 [`StripProjection::apply`] 的通道条分支逐字同集：7 个参数各有 2 个拼写，
    /// 两个增益各有 3 个，两个开关与 `cutoff_hz` 各 1 个；⭐ `#304` 起压缩的 8 个
    /// 字段共 14 个拼写也在这里（⛔ 不再有第二张 `STRIP_COMPRESSOR_SPELLINGS`）。
    /// `yeban_render::automation::APPLIED_DEVICE_PARAM_NAMES` 必须与本表**完全一致**
    /// （由 `the_applied_names_and_the_field_table_agree` 双向钉住）。
    pub const SPELLINGS: &'static [(&'static str, Self)] = &[
        ("input_gain_db", Self::InputGainDb),
        ("input_gain", Self::InputGainDb),
        ("trim_db", Self::InputGainDb),
        ("output_gain_db", Self::OutputGainDb),
        ("output_gain", Self::OutputGainDb),
        ("fader_db", Self::OutputGainDb),
        ("eq_enabled", Self::EqEnabled),
        ("filter_enabled", Self::FilterEnabled),
        ("eq_low_gain", Self::EqLowGain),
        ("low_gain", Self::EqLowGain),
        ("eq_low_freq", Self::EqLowFreq),
        ("low_freq", Self::EqLowFreq),
        ("eq_mid_gain", Self::EqMidGain),
        ("mid_gain", Self::EqMidGain),
        ("eq_mid_freq", Self::EqMidFreq),
        ("mid_freq", Self::EqMidFreq),
        ("eq_mid_q", Self::EqMidQ),
        ("mid_q", Self::EqMidQ),
        ("eq_high_gain", Self::EqHighGain),
        ("high_gain", Self::EqHighGain),
        ("eq_high_freq", Self::EqHighFreq),
        ("high_freq", Self::EqHighFreq),
        ("cutoff_hz", Self::CutoffHz),
        ("resonance", Self::Resonance),
        ("filter_resonance", Self::Resonance),
        ("drive", Self::Drive),
        ("filter_drive", Self::Drive),
        // ⭐ `#304`：压缩 8 字段 / 14 拼写（与 `StripProjection::apply` 同集）。
        ("compressor_enabled", Self::CompressorEnabled),
        ("threshold_db", Self::ThresholdDb),
        ("threshold", Self::ThresholdDb),
        ("ratio", Self::Ratio),
        ("knee_db", Self::KneeDb),
        ("knee", Self::KneeDb),
        ("detector_s", Self::DetectorS),
        ("detector", Self::DetectorS),
        ("attack_s", Self::AttackS),
        ("attack", Self::AttackS),
        ("release_s", Self::ReleaseS),
        ("release", Self::ReleaseS),
        ("makeup_db", Self::MakeupDb),
        ("makeup", Self::MakeupDb),
    ];

    /// 名字 → 字段（**大小写不敏感**，别名表与 [`StripProjection::apply`] 逐字同集）。
    ///
    /// `None` = 本字段集不认这个名字（混响 / 卷积 / 未知旋钮）。⚠ 本函数是
    /// "哪个名字能落点"的唯一判定处：`yeban_render::automation::APPLIED_DEVICE_PARAM_NAMES`
    /// 是它的**可读清单**，两者由 `the_applied_names_and_the_field_table_agree` 钉住。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::SPELLINGS
            .iter()
            .find(|(spelling, _)| name.eq_ignore_ascii_case(spelling))
            .map(|(_, field)| *field)
    }

    /// 规范名（进读数 / 文档的那一个拼写）。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::InputGainDb => "input_gain_db",
            Self::OutputGainDb => "output_gain_db",
            Self::EqEnabled => "eq_enabled",
            Self::FilterEnabled => "filter_enabled",
            Self::EqLowGain => "eq_low_gain",
            Self::EqLowFreq => "eq_low_freq",
            Self::EqMidGain => "eq_mid_gain",
            Self::EqMidFreq => "eq_mid_freq",
            Self::EqMidQ => "eq_mid_q",
            Self::EqHighGain => "eq_high_gain",
            Self::EqHighFreq => "eq_high_freq",
            Self::CutoffHz => "cutoff_hz",
            Self::Resonance => "resonance",
            Self::Drive => "drive",
            Self::CompressorEnabled => "compressor_enabled",
            Self::ThresholdDb => "threshold_db",
            Self::Ratio => "ratio",
            Self::KneeDb => "knee_db",
            Self::DetectorS => "detector_s",
            Self::AttackS => "attack_s",
            Self::ReleaseS => "release_s",
            Self::MakeupDb => "makeup_db",
        }
    }

    /// 本字段是否是**开关**（读作 `1.0` / `0.0`，写时非零即真）。
    #[must_use]
    pub const fn is_flag(self) -> bool {
        matches!(
            self,
            Self::EqEnabled | Self::FilterEnabled | Self::CompressorEnabled
        )
    }

    /// 从**已 sanitised 的**通道条参数里读这个字段（开关读成 `1.0` / `0.0`）。
    ///
    /// 与 [`Self::store`] 是同一张字段表的两个方向：`store` 之后 `read` 必得
    /// （钳制后的）存入值。
    #[must_use]
    pub fn read(self, params: &ChannelStripParams) -> f32 {
        match self {
            Self::InputGainDb => params.input_gain_db,
            Self::OutputGainDb => params.output_gain_db,
            Self::EqEnabled => f32::from(u8::from(params.eq_enabled)),
            Self::FilterEnabled => f32::from(u8::from(params.filter_enabled)),
            Self::EqLowGain => params.eq.low_gain,
            Self::EqLowFreq => params.eq.low_freq,
            Self::EqMidGain => params.eq.mid_gain,
            Self::EqMidFreq => params.eq.mid_freq,
            Self::EqMidQ => params.eq.mid_q,
            Self::EqHighGain => params.eq.high_gain,
            Self::EqHighFreq => params.eq.high_freq,
            Self::CutoffHz => params.filter.cutoff_hz,
            Self::Resonance => params.filter.resonance,
            Self::Drive => params.filter.drive,
            Self::CompressorEnabled => f32::from(u8::from(params.compressor_enabled)),
            Self::ThresholdDb => params.compressor.threshold_db,
            Self::Ratio => params.compressor.ratio,
            Self::KneeDb => params.compressor.knee_db,
            Self::DetectorS => params.compressor.detector_s,
            Self::AttackS => params.compressor.attack_s,
            Self::ReleaseS => params.compressor.release_s,
            Self::MakeupDb => params.compressor.makeup_db,
        }
    }

    /// 把**原生绝对值**写进参数（开关：非零即真，与 [`flag`] 同式）。
    ///
    /// ⚠ 不在这里钳制：钳制是器件自己的纪律（`ChannelStrip::set_params` 内部调
    /// [`ChannelStripParams::sanitised`]）。[`Self::sanitise`] 只是把**同一次**钳制
    /// 提前算给恒等判定看。
    pub fn store(self, params: &mut ChannelStripParams, value: f32) {
        match self {
            Self::InputGainDb => params.input_gain_db = value,
            Self::OutputGainDb => params.output_gain_db = value,
            Self::EqEnabled => params.eq_enabled = value != 0.0,
            Self::FilterEnabled => params.filter_enabled = value != 0.0,
            Self::EqLowGain => params.eq.low_gain = value,
            Self::EqLowFreq => params.eq.low_freq = value,
            Self::EqMidGain => params.eq.mid_gain = value,
            Self::EqMidFreq => params.eq.mid_freq = value,
            Self::EqMidQ => params.eq.mid_q = value,
            Self::EqHighGain => params.eq.high_gain = value,
            Self::EqHighFreq => params.eq.high_freq = value,
            Self::CutoffHz => params.filter.cutoff_hz = value,
            Self::Resonance => params.filter.resonance = value,
            Self::Drive => params.filter.drive = value,
            Self::CompressorEnabled => params.compressor_enabled = value != 0.0,
            Self::ThresholdDb => params.compressor.threshold_db = value,
            Self::Ratio => params.compressor.ratio = value,
            Self::KneeDb => params.compressor.knee_db = value,
            Self::DetectorS => params.compressor.detector_s = value,
            Self::AttackS => params.compressor.attack_s = value,
            Self::ReleaseS => params.compressor.release_s = value,
            Self::MakeupDb => params.compressor.makeup_db = value,
        }
    }

    /// 与器件内部**同一次**钳制：把值写进一份 `DEFAULT` 参数、`sanitised()`、再读回来。
    ///
    /// ⛔ 不复制上下界：`ChannelStripParams::sanitised` 是唯一一份钳制规则（包括
    /// `cutoff_hz` 的非有限值归**上界**这条不对称裁决）。把它抄到这里必然漂移。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        let mut params = ChannelStripParams::DEFAULT;
        self.store(&mut params, value);
        self.read(&params.sanitised())
    }
}

/// Freeverb 上**可被 `DeviceParam` 自动化落点**的字段。
///
/// 与 `reverb_of` 的静态投影**同一张拼写表**（[`Self::SPELLINGS`]）：静态投影与
/// 块边界自动化读的是同一组名字，⛔ 不写第二份。刻意**不含** `reverb_mix` /
/// `reverb_width`（与引擎 `ReverbProjection::apply` 同口径，那两个名字**不认**）。
///
/// 四个字段都是器件 `Reverb::set_params` 的标量入参；该函数文档承诺**零分配、可逐块**
/// ⇒ 块边界自动化不需要重新分配延迟线（`set_sample_rate` 才是分配入口）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReverbField {
    /// 衰减长度（0..1）。`reverb_size`。
    Size,
    /// 尾巴高频阻尼（0..1）。`reverb_damp`。
    Damp,
    /// 湿/干平衡（0..1）。`reverb_wet` / `reverb_wet_mix`。
    Wet,
    /// 湿路预延迟（秒）。`reverb_predelay` / `reverb_predelay_s`。
    Predelay,
}

impl ReverbField {
    /// 全部字段。
    pub const ALL: &'static [Self] = &[Self::Size, Self::Damp, Self::Wet, Self::Predelay];

    /// **全部拼写** → 字段（`6` 条 = `4` 个字段）。
    pub const SPELLINGS: &'static [(&'static str, Self)] = &[
        ("reverb_size", Self::Size),
        ("reverb_damp", Self::Damp),
        ("reverb_wet", Self::Wet),
        ("reverb_wet_mix", Self::Wet),
        ("reverb_predelay", Self::Predelay),
        ("reverb_predelay_s", Self::Predelay),
    ];

    /// 名字 → 字段（大小写不敏感）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::SPELLINGS
            .iter()
            .find(|(spelling, _)| name.eq_ignore_ascii_case(spelling))
            .map(|(_, field)| *field)
    }

    /// 规范名。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Size => "reverb_size",
            Self::Damp => "reverb_damp",
            Self::Wet => "reverb_wet",
            Self::Predelay => "reverb_predelay_s",
        }
    }

    /// 从投影参数里读这个字段。
    #[must_use]
    pub fn read(self, params: &ReverbParams) -> f32 {
        match self {
            Self::Size => params.size,
            Self::Damp => params.damp,
            Self::Wet => params.mix,
            Self::Predelay => params.predelay,
        }
    }

    /// 把**原生绝对值**写进投影参数（⛔ 不在这里钳制：钳制在器件的 `set_params`）。
    pub fn store(self, params: &mut ReverbParams, value: f32) {
        match self {
            Self::Size => params.size = value,
            Self::Damp => params.damp = value,
            Self::Wet => params.mix = value,
            Self::Predelay => params.predelay = value,
        }
    }

    /// 与 `Reverb::set_params` **同一次**钳制/回落（只给恒等判定用）。
    ///
    /// 非有限值：`size` / `damp` / `mix` 回落到 [`ReverbParams::default`] 的对应值，
    /// `predelay` 回落到 `0.0`（与器件逐字一致）。由真实器件的网格判据钉住。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        let fallback = ReverbParams::default();
        let finite = |value: f32, fallback: f32| {
            if value.is_finite() { value } else { fallback }
        };
        match self {
            Self::Size => finite(value, fallback.size).clamp(0.0, 1.0),
            Self::Damp => finite(value, fallback.damp).clamp(0.0, 1.0),
            Self::Wet => finite(value, fallback.mix).clamp(0.0, 1.0),
            Self::Predelay => finite(value, 0.0).clamp(0.0, REVERB_MAX_PREDELAY_S),
        }
    }
}

/// 卷积混响上**可被 `DeviceParam` 自动化落点**的**标量**字段。
///
/// 刻意**不含** IR 形状（`conv_ir_decay_s` / `conv_ir_decay` / `conv_ir_seed`）：
/// 那三个是**构造期烘焙**，换值要重新合成 IR 并走 `set_impulse_response`
/// （`yeban-dsp` 文档逐字“分配入口之一”）⇒ 不是块边界能改的标量。
///
/// 四个字段都是 `ConvolutionReverb::set_params` 的入参；该函数文档承诺**零分配、
/// 可以逐块调用** ⇒ 块边界自动化不需要重烘 IR。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConvolutionField {
    /// 湿路预延迟（秒，0..0.1）。`conv_predelay*`。
    PreDelay,
    /// 干路电平（0..1）。`conv_dry`。
    Dry,
    /// 湿路电平（0..1）。`conv_wet` / `conv_wet_mix`。
    Wet,
    /// IR 增益（dB，−60..+24）。`conv_ir_gain_db` / `conv_ir_gain`。
    IrGainDb,
}

impl ConvolutionField {
    /// 全部字段。
    pub const ALL: &'static [Self] = &[Self::PreDelay, Self::Dry, Self::Wet, Self::IrGainDb];

    /// **全部拼写** → 字段（`8` 条 = `4` 个字段）。
    pub const SPELLINGS: &'static [(&'static str, Self)] = &[
        ("conv_predelay_s", Self::PreDelay),
        ("conv_pre_delay_s", Self::PreDelay),
        ("conv_predelay", Self::PreDelay),
        ("conv_dry", Self::Dry),
        ("conv_wet", Self::Wet),
        ("conv_wet_mix", Self::Wet),
        ("conv_ir_gain_db", Self::IrGainDb),
        ("conv_ir_gain", Self::IrGainDb),
    ];

    /// 名字 → 字段（大小写不敏感）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::SPELLINGS
            .iter()
            .find(|(spelling, _)| name.eq_ignore_ascii_case(spelling))
            .map(|(_, field)| *field)
    }

    /// 规范名。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::PreDelay => "conv_predelay_s",
            Self::Dry => "conv_dry",
            Self::Wet => "conv_wet",
            Self::IrGainDb => "conv_ir_gain_db",
        }
    }

    /// 从投影参数里读这个字段。
    #[must_use]
    pub fn read(self, params: &ConvolutionReverbParams) -> f32 {
        match self {
            Self::PreDelay => params.pre_delay_s,
            Self::Dry => params.dry,
            Self::Wet => params.wet,
            Self::IrGainDb => params.ir_gain_db,
        }
    }

    /// 把**原生绝对值**写进投影参数（⛔ 钳制在器件的 `set_params`）。
    pub fn store(self, params: &mut ConvolutionReverbParams, value: f32) {
        match self {
            Self::PreDelay => params.pre_delay_s = value,
            Self::Dry => params.dry = value,
            Self::Wet => params.wet = value,
            Self::IrGainDb => params.ir_gain_db = value,
        }
    }

    /// 与 `ConvolutionReverb::set_params` **同一次**钳制/回落（只给恒等判定用）。
    ///
    /// 上下界直接引用 `yeban-dsp` 的 `pub` 常量（`MAX_PRE_DELAY_SECONDS` /
    /// `MIN_IR_GAIN_DB` / `MAX_IR_GAIN_DB`），⛔ 不手抄数字。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        let fallback = ConvolutionReverbParams::default();
        let finite = |value: f32, fallback: f32| {
            if value.is_finite() { value } else { fallback }
        };
        match self {
            Self::PreDelay => finite(value, fallback.pre_delay_s).clamp(0.0, MAX_PRE_DELAY_SECONDS),
            Self::Dry => finite(value, fallback.dry).clamp(0.0, 1.0),
            Self::Wet => finite(value, fallback.wet).clamp(0.0, 1.0),
            Self::IrGainDb => {
                finite(value, fallback.ir_gain_db).clamp(MIN_IR_GAIN_DB, MAX_IR_GAIN_DB)
            }
        }
    }
}

/// 一条 `DeviceParam` 泳道在离线插入链上的**落点**：器件族 ＋ 字段。
///
/// [`Self::from_name`] 是“哪个名字有落点”的**唯一**判定处：静态投影
/// （`strip_of` / `reverb_of` / `convolution_of`）与块边界自动化都从这张表出发。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceParamField {
    /// 通道条字段。
    Strip(StripField),
    /// Freeverb 标量字段。
    Reverb(ReverbField),
    /// 卷积混响**标量**字段。
    Convolution(ConvolutionField),
}

/// 强制实例化按**器件族**分开。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceParamFamily {
    /// 通道条。
    Strip,
    /// Freeverb。
    Reverb,
    /// 卷积混响。
    Convolution,
}

impl DeviceParamField {
    /// 名字 → 落点（大小写不敏感）。顺序：通道条 → 混响 → 卷积。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        StripField::from_name(name)
            .map(Self::Strip)
            .or_else(|| ReverbField::from_name(name).map(Self::Reverb))
            .or_else(|| ConvolutionField::from_name(name).map(Self::Convolution))
    }

    /// 规范名。
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Strip(field) => field.canonical_name(),
            Self::Reverb(field) => field.canonical_name(),
            Self::Convolution(field) => field.canonical_name(),
        }
    }

    /// 器件族（强制实例化用）。
    #[must_use]
    pub const fn family(self) -> DeviceParamFamily {
        match self {
            Self::Strip(_) => DeviceParamFamily::Strip,
            Self::Reverb(_) => DeviceParamFamily::Reverb,
            Self::Convolution(_) => DeviceParamFamily::Convolution,
        }
    }

    /// 与器件**同一次**钳制/回落（只给恒等判定用）。
    #[must_use]
    pub fn sanitise(self, value: f32) -> f32 {
        match self {
            Self::Strip(field) => field.sanitise(value),
            Self::Reverb(field) => field.sanitise(value),
            Self::Convolution(field) => field.sanitise(value),
        }
    }
}

/// 通道条被**强制实例化**、但本轨没有投影来源时的参数（任务单 `#266` 的缝）。
///
/// 三级全关 ＋ 增益 `0 dB` ⇒ 器件是**逐样本恒等**的直通，只是"此刻由泳道来改它"。
/// [`InsertChain::effective_strip`] 与
/// [`crate::automation::applicable_device_param`] 的静态基值都用**这一份**
/// ⇒ "器件拿到的静态参数"与"恒等判定比的那个静态值"不可能各写一套。
#[must_use]
pub fn forced_identity_strip() -> ChannelStripParams {
    ChannelStripParams {
        eq_enabled: false,
        filter_enabled: false,
        compressor_enabled: false,
        ..ChannelStripParams::DEFAULT
    }
}

/// 一次**成功解析**的设备参数地址（序数 → 稳定身份）。
///
/// 为什么需要翻译：`AutomationTarget::DeviceParam` 的 `slot_index` / `param_index`
/// 是 `track.devices` / `device.params` 两个 `Vec` 的**下标**，而模型自己的寻址
/// 口径是 `track_id` + `device_id`（`crates/yeban-model/src/ops.rs` 的
/// `SetDeviceBypassed` 逐字写着"⛔ 不用下标（`Vec`，增删会移位）"）。于是**中途插一台
/// 设备**会让泳道静默改指另一台设备，而 `AutomationTarget::validate_against` 仍
/// `Ok(())`（新下标仍在范围）。本结构不改变那个行为，只把"此刻指到谁"变成读数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDeviceParam {
    /// 这个序数**此刻**指到的设备身份。
    pub device_id: EntityId,
    /// 这个序数**此刻**指到的参数名（**原文**，不改大小写）。
    pub param_name: String,
    /// 该名字是否在**本切片真的消费**的白名单里（[`is_supported_param`]，大小写不敏感）。
    ///
    /// ⚠ 只判**名字**：不看 `bypassed` / `kind` —— "这台设备会不会被执行"由
    /// [`track_has_unhandled_dsp`] 与 [`InsertChain`] 各自回答。
    pub is_supported: bool,
}

impl ResolvedDeviceParam {
    /// 名字解析到了、但 DSP 不认时的**唯一**原因串。
    ///
    /// `None` = 名字在[`is_supported_param`] 里。返回 `&'static str` 而不是让调用方
    /// 各自拼字面量：MCP 读数里 `unresolved: "paramNotSupported"` 只有这一处来源。
    #[must_use]
    pub const fn unresolved_reason(&self) -> Option<&'static str> {
        if self.is_supported {
            None
        } else {
            Some("paramNotSupported")
        }
    }
}

/// 序数地址**解析失败**的原因。
///
/// 变体刻意与 `yeban_model::ModelError` 的三种下标错误一一对应，但**不**携带
/// `len` / 缺失实体：MCP 读数只要一个稳定的原因串，长度由模型的对账入口自己报。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// 音轨不存在（`project.tracks` 里没有这个身份）。
    TrackNotFound,
    /// `slot_index` 越界（`track.devices` 没有这个下标）。
    SlotOutOfRange,
    /// `param_index` 越界（该设备的 `params` 没有这个下标）。
    ParamIndexOutOfRange,
}

impl UnresolvedReason {
    /// 进 JSON 的逐字原因串（MCP 读数 `unresolved` 的唯一词表）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TrackNotFound => "trackNotFound",
            Self::SlotOutOfRange => "slotOutOfRange",
            Self::ParamIndexOutOfRange => "paramIndexOutOfRange",
        }
    }
}

/// 把 `(track_id, slot_index, param_index)` 这组**序数**翻译成稳定名字
/// （**唯一实现**：MCP 只调用它，⛔ 不复制一份解析）。
///
/// 范围规则与 `AutomationTarget::validate_against`（`crates/yeban-model/src/automation.rs`）
/// **同式**：先 `track.devices.get(slot_index)`，再 `param_index >= device.params.len()`。
/// 因此"解析失败"与"模型判目标不存在"不会互相矛盾；反过来，**解析成功也不代表泳道
/// 没改指** —— 插设备之后同一个序数仍解析成功，只是 `device_id` 换了一台。
///
/// # Errors
///
/// - 音轨不存在 → [`UnresolvedReason::TrackNotFound`]；
/// - `slot_index` 越界 → [`UnresolvedReason::SlotOutOfRange`]；
/// - `param_index` 越界 → [`UnresolvedReason::ParamIndexOutOfRange`]。
pub fn resolve_device_param(
    project: &YebanProjectV1,
    track_id: EntityId,
    slot_index: usize,
    param_index: usize,
) -> Result<ResolvedDeviceParam, UnresolvedReason> {
    let track = project
        .track(&track_id)
        .map_err(|_| UnresolvedReason::TrackNotFound)?;
    let device = track
        .devices
        .get(slot_index)
        .ok_or(UnresolvedReason::SlotOutOfRange)?;
    let param = device
        .params
        .get(param_index)
        .ok_or(UnresolvedReason::ParamIndexOutOfRange)?;
    Ok(ResolvedDeviceParam {
        device_id: device.id,
        param_name: param.name.clone(),
        is_supported: is_supported_param(&param.name.to_ascii_lowercase()),
    })
}

/// 单台设备 → 通道条参数（`None` = 一个已识别的通道条名字都没有）。
///
/// 逐字镜像引擎 `crates/yeban-engine/src/insert.rs` 的 `strip_of` /
/// `StripProjection`（`from_devices` 的"首个来源"规则也包括在内）：离线与实时
/// 必须识别**同一组名字**、推导**同一组级开关**，否则 `deviceChainDsp` 的登记
/// 会与真实求值脱节。镜像由测试 `strip_names_are_the_engine_whitelist` 对账。
fn strip_of(device: &DeviceDefinition) -> Option<ChannelStripParams> {
    let mut resolved = StripProjection::default();
    for param in &device.params {
        resolved.apply(&param.name.to_ascii_lowercase(), param.value);
    }
    resolved.into_params()
}

/// 布尔参数的取值口径：非零即真（与引擎的 `flag` 同式）。
fn flag(value: f32) -> bool {
    value != 0.0
}

/// 通道条投影的中间态（引擎 `StripProjection` 的镜像）。
///
/// `None` 在开关字段上的语义是"设备没写这个开关"：没写旋钮的级必须得到 `false`，
/// 不能沿用 dsp 默认值（那一份把三级都默认打开）。
#[derive(Clone, Copy, Debug)]
struct StripProjection {
    eq: StripEq,
    filter_seen: bool,
    compressor_seen: bool,
    eq_enabled: Option<bool>,
    filter_enabled: Option<bool>,
    compressor_enabled: Option<bool>,
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
            params: ChannelStripParams::DEFAULT,
        }
    }
}

impl StripProjection {
    fn apply(&mut self, name: &str, value: f32) {
        match name {
            "input_gain_db" | "input_gain" | "trim_db" => self.params.input_gain_db = value,
            "output_gain_db" | "output_gain" | "fader_db" => self.params.output_gain_db = value,

            "eq_enabled" => self.eq_enabled = Some(flag(value)),
            "filter_enabled" => self.filter_enabled = Some(flag(value)),
            "compressor_enabled" => {
                self.compressor_enabled = Some(flag(value));
                self.compressor_seen = true;
            }

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

            // ⚠ 只有 `cutoff_hz`：`filter_cutoff_hz` 与引擎一样**刻意不认**。
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

    fn into_params(self) -> Option<ChannelStripParams> {
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
        params.eq_enabled = self.eq_enabled.unwrap_or(self.eq.seen);
        params.filter_enabled = self.filter_enabled.unwrap_or(self.filter_seen);
        params.compressor_enabled = self.compressor_enabled.unwrap_or(self.compressor_seen);
        Some(params)
    }
}

/// 单台设备 → 混响参数（`None` = 本设备不是混响来源）。
///
/// 名字表是 [`ReverbField::SPELLINGS`]（与自动化**同一张**）；⚠ `reverb_mix` /
/// `reverb_width` **刻意不认**：与引擎的 `ReverbProjection::apply` 逐字同口径
/// （这两个名字属于"单声道插入点"那一组，见引擎模块文档 §8.1）。
fn reverb_of(device: &DeviceDefinition) -> Option<ReverbParams> {
    let mut params = ReverbParams::default();
    let mut seen = false;
    for param in &device.params {
        if let Some(field) = ReverbField::from_name(&param.name) {
            field.store(&mut params, param.value);
            seen = true;
        }
    }
    if seen { Some(params) } else { None }
}

/// 单台设备 → 卷积的**投影中间态**：`(标量参数, IR 衰减秒, IR 种子)`。
///
/// 与 [`convolution_of`] 的识别规则**同一份**；单独拆出来是为了让“来源设备”判定与
/// 标量参数读取**不用重烘 IR**（[`convolution_source_slot`] /
/// [`convolution_source_params`]）。`None` = 本设备不是卷积来源。
fn convolution_projection(
    device: &DeviceDefinition,
) -> Option<(ConvolutionReverbParams, f32, u32)> {
    let mut params = ConvolutionReverbParams::default();
    let mut decay_s = CONV_IR_DEFAULT_DECAY_S;
    let mut seed = CONV_IR_DEFAULT_SEED as u32;
    let mut seen = false;
    for param in &device.params {
        let name = param.name.to_ascii_lowercase();
        let value = param.value;
        if let Some(field) = ConvolutionField::from_name(&name) {
            field.store(&mut params, value);
            seen = true;
            continue;
        }
        match name.as_str() {
            "conv_ir_decay_s" | "conv_ir_decay" => {
                decay_s = value;
                seen = true;
            }
            "conv_ir_seed" => {
                seed = value as u32;
                seen = true;
            }
            _ => {}
        }
    }
    if seen {
        Some((params, decay_s, seed))
    } else {
        None
    }
}

/// 单台设备 → 卷积计划（`None` = 本设备不是卷积来源）。
fn convolution_of(device: &DeviceDefinition, sample_rate: u32) -> Option<ConvolutionPlan> {
    let (params, decay_s, seed) = convolution_projection(device)?;
    let decay_s = if decay_s.is_finite() {
        decay_s.clamp(CONV_IR_MIN_DECAY_S, CONV_IR_MAX_DECAY_S)
    } else {
        CONV_IR_DEFAULT_DECAY_S
    };
    let ir = synthesise_impulse_response(
        convolution_ir_frames(sample_rate),
        decay_s,
        sample_rate,
        seed,
    );
    Some(ConvolutionPlan { params, ir })
}

/// 一条链的**运行态**（每个渲染节点一份）。
///
/// 构造期把全部缓冲与器件状态准备好；逐块处理**零分配**。器件状态跨块存活，
/// 因此 [`Self::reset`] 必须在每次 `execute` 开始时调用（复位契约）。
pub(crate) struct InsertRuntime {
    /// 通道条状态（输入增益 → EQ → 低通 → **动态** → 输出增益；动态级自 `#303` 起执行）。
    strip: Option<ChannelStrip>,
    /// Freeverb 状态；`None` = 不动（`mix = 0` 或没有）。
    reverb: Option<Reverb>,
    /// 卷积混响状态；`None` = 不动（`wet = 0` 或没有）。
    convolution: Option<ConvolutionReverb>,
    /// 左声道去交错缓冲。
    left: Vec<f32>,
    /// 右声道去交错缓冲。
    right: Vec<f32>,
}

/// 通道条**动态级**的只读读数（任务单 `#303`；[`crate::render::InsertMeterReading`]
/// 的原料）。
///
/// 全部来自器件自己的计数器 —— ⛔ 本层不另做测量。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StripMeter {
    /// 通道条处理过的帧数（自本次 `execute` 起）。
    pub(crate) strip_frames: u64,
    /// 被压缩过的帧数。
    pub(crate) gain_reductions: u64,
    /// 全程最大增益衰减（dB，`≥ 0`）。
    pub(crate) max_reduction_db: f32,
    /// 读到的那一刻的增益衰减（dB，`≥ 0`）。
    pub(crate) current_reduction_db: f32,
    /// 检波器看到的电平（dBFS）。
    pub(crate) detector_level_db: f32,
}

impl InsertRuntime {
    /// 通道条的动态级读数；`None` = 本题没有通道条运行态。
    pub(crate) fn strip_meter(&self) -> Option<StripMeter> {
        let strip = self.strip.as_ref()?;
        Some(StripMeter {
            strip_frames: strip.processed_frames(),
            gain_reductions: strip.gain_reduction_count(),
            max_reduction_db: strip.max_gain_reduction_db(),
            current_reduction_db: strip.current_gain_reduction_db(),
            detector_level_db: strip.detector_level_db(),
        })
    }

    /// 从一条非空链构造运行态（`block_size` 是单块帧数，`sample_rate` 是采样率）。
    ///
    /// **只在控制路径调用**（允许分配：卷积核与混响延迟线在这里建起来）。
    pub(crate) fn new(chain: &InsertChain, sample_rate: u32, block_size: usize) -> Self {
        let sample_rate_f32 = sample_rate as f32;
        let strip = chain
            .effective_strip()
            .map(|params| ChannelStrip::new(params, sample_rate_f32));
        let reverb = chain.reverb.and_then(|params| {
            let mut state = Reverb::new();
            state.set_sample_rate(sample_rate_f32);
            state.set_params(params);
            // ⭐ `#293`：静态 `reverb_wet: 0` 时器件 `is_active()` 为假，但一条
            // 非恒等泳道会在块边界把它拉起来 ⇒ 强制位为真时必须保留这个实例，
            // 否则“报 applied、却静默无效”。
            if state.is_active() || chain.is_forced_reverb() {
                Some(state)
            } else {
                None
            }
        });
        let convolution = chain.convolution.as_ref().and_then(|plan| {
            let mut state = ConvolutionReverb::new();
            state.set_sample_rate(sample_rate_f32);
            let ir = plan.ir();
            let silence = vec![0.0f32; ir.len()];
            let frames = state.set_impulse_response(ir, &silence, &silence, ir);
            if frames == 0 {
                return None;
            }
            state.set_params(plan.params());
            // ⭐ `#293`：与 Freeverb 同一条缝（静态 `conv_wet: 0`）。
            if state.is_active() || chain.is_forced_convolution() {
                Some(state)
            } else {
                None
            }
        });
        Self {
            strip,
            reverb,
            convolution,
            left: vec![0.0; block_size],
            right: vec![0.0; block_size],
        }
    }

    /// 复位全部器件的时间状态（下一次 `execute` 与"刚编译"逐位等价）。
    pub(crate) fn reset(&mut self) {
        if let Some(strip) = &mut self.strip {
            strip.reset();
        }
        if let Some(reverb) = &mut self.reverb {
            reverb.reset();
        }
        if let Some(convolution) = &mut self.convolution {
            convolution.reset();
        }
    }

    /// 把一个新的**原生绝对值**交给某个插入器件的字段并**重算系数**
    /// （`#266` 开 `output_gain_db` / `#267` 扩到通道条整组 / `#293` 扩到 Freeverb
    /// 与卷积的标量字段 / `#304` 扩到压缩 8 字段；`ChannelStrip::set_params` /
    /// `Reverb::set_params` / `ConvolutionReverb::set_params` 是各自**唯一**入口）。
    ///
    /// 调用纪律（器件自己不做平滑，模块头 §4 的器件纪律）：
    ///
    /// - **只在块边界**调用（自动化块的 128 帧起点）—— 改的是**参数**，⛔ 不是逐样本
    ///   乘一个增益；
    /// - 三个 `set_params` 都**不清状态**（EQ 双二阶 / 滤波器 / 梳状线 / 卷积尾 /
    ///   **压缩器弹道**保留），因此块与块之间没有复位爆音；混响/卷积的 `set_params`
    ///   的预延迟长度变化会清对应延迟线（器件文档 §6 / §2.2，本单不另做平滑）；
    /// - 写哪个字段由 [`DeviceParamField`] 决定（对应器件为 `None` 时什么都没发生 ——
    ///   恒等泳道根本不会走到这里）。
    pub(crate) fn apply_device_params(&mut self, field: DeviceParamField, value: f32) {
        match field {
            DeviceParamField::Strip(field) => {
                if let Some(strip) = &mut self.strip {
                    let mut params = strip.params();
                    field.store(&mut params, value);
                    strip.set_params(params);
                }
            }
            DeviceParamField::Reverb(field) => {
                if let Some(reverb) = &mut self.reverb {
                    let mut params = reverb.params();
                    field.store(&mut params, value);
                    reverb.set_params(params);
                }
            }
            DeviceParamField::Convolution(field) => {
                if let Some(convolution) = &mut self.convolution {
                    let mut params = convolution.params();
                    field.store(&mut params, value);
                    convolution.set_params(params);
                }
            }
        }
    }

    /// 原地处理一个**立体声交错**块（`buffer.len() == frames * 2`）。
    ///
    /// 顺序与实时引擎同一条链：通道条 → Freeverb → 卷积混响。各级在参数为 0 时
    /// 整级跳过（状态不推进），与器件自己的 `is_active` 守卫同口径。
    pub(crate) fn process(&mut self, buffer: &mut [f32]) {
        let frames = buffer.len() / 2;
        if frames == 0 {
            return;
        }
        let Self {
            strip,
            reverb,
            convolution,
            left,
            right,
        } = self;
        for frame in 0..frames {
            left[frame] = buffer[2 * frame];
            right[frame] = buffer[2 * frame + 1];
        }
        if let Some(strip) = strip {
            strip.process_stereo(&mut left[..frames], &mut right[..frames]);
        }
        if let Some(reverb) = reverb {
            reverb.process(&mut left[..frames], &mut right[..frames]);
        }
        if let Some(convolution) = convolution {
            for frame in 0..frames {
                buffer[2 * frame] = left[frame];
                buffer[2 * frame + 1] = right[frame];
            }
            convolution.process(&mut buffer[..2 * frames]);
        } else {
            for frame in 0..frames {
                buffer[2 * frame] = left[frame];
                buffer[2 * frame + 1] = right[frame];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_dsp::channel_strip::{MAX_CUTOFF_HZ, MAX_GAIN_DB, MIN_CUTOFF_HZ, MIN_GAIN_DB};
    use yeban_dsp::compressor::{
        MAX_KNEE_DB, MAX_MAKEUP_DB, MAX_RATIO, MAX_THRESHOLD_DB, MIN_LEVEL_DB, MIN_TIME_S,
    };
    use yeban_model::ParameterValue;

    fn device(params: &[(&str, f32)]) -> DeviceDefinition {
        DeviceDefinition {
            id: EntityId::default(),
            name: "strip".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
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

    /// 每个字段一个"非默认"的样本值：`strip_of` 必须认出它（⛔ 不能落进
    /// `params != DEFAULT` 的兜底缝，否则测的就不是投影）。
    fn sample(field: StripField) -> f32 {
        match field {
            StripField::InputGainDb | StripField::OutputGainDb => 3.0,
            StripField::EqEnabled | StripField::FilterEnabled | StripField::CompressorEnabled => {
                1.0
            }
            StripField::EqLowGain => 3.0,
            StripField::EqLowFreq => 800.0,
            StripField::EqMidGain => -3.0,
            StripField::EqMidFreq => 2_000.0,
            StripField::EqMidQ => 1.5,
            StripField::EqHighGain => -3.0,
            StripField::EqHighFreq => 6_000.0,
            StripField::CutoffHz => 5_000.0,
            StripField::Resonance | StripField::Drive => 0.5,
            StripField::ThresholdDb => -20.0,
            StripField::Ratio => 4.0,
            StripField::KneeDb => 6.0,
            StripField::DetectorS => 0.005,
            StripField::AttackS => 0.010,
            StripField::ReleaseS => 0.100,
            StripField::MakeupDb => 3.0,
        }
    }

    /// 每个**设备参数落点**一个“非默认”的样本值：静态投影必须认出它，且落在
    /// 器件合法域内（这样“投影读数”与“钳制镜像”可以直接按位比）。
    fn sample_device(field: DeviceParamField) -> f32 {
        match field {
            DeviceParamField::Strip(field) => sample(field),
            DeviceParamField::Reverb(ReverbField::Size | ReverbField::Damp | ReverbField::Wet) => {
                0.72
            }
            DeviceParamField::Reverb(ReverbField::Predelay) => 0.05,
            DeviceParamField::Convolution(ConvolutionField::PreDelay) => 0.05,
            DeviceParamField::Convolution(ConvolutionField::Dry | ConvolutionField::Wet) => 0.72,
            DeviceParamField::Convolution(ConvolutionField::IrGainDb) => -7.5,
        }
    }

    /// ⭐ `#267`/`#293` 判据：`APPLIED_DEVICE_PARAM_NAMES` 的**每一个拼写**都投影到
    /// 它自己那个字段，且恒等判定用的钳制值与**真实器件**读数是同一个。
    ///
    /// 这条就是"名单 ↔ 投影 ↔ 器件"三者的镜像判据：只改名单（不加字段表分支）⇒
    /// `from_name` 返回 `None` ⇒ 红；只改投影（不扩字段表）⇒ 投影值读不出来 ⇒ 红；
    /// 钳制镜像与器件漂移 ⇒ 红（混响/卷积走的是 `new → set_params → params` 的真实实例）。
    #[test]
    fn every_applied_name_projects_to_its_own_field() {
        for &name in crate::automation::APPLIED_DEVICE_PARAM_NAMES {
            let field = DeviceParamField::from_name(name)
                .unwrap_or_else(|| panic!("名单里的 {name} 必须是设备参数字段"));
            let value = sample_device(field);
            match field {
                DeviceParamField::Strip(strip) => {
                    let projected = strip_of(&device(&[(name, value)]))
                        .unwrap_or_else(|| panic!("{name}={value} 必须被 strip_of 认出来"))
                        .sanitised();
                    assert_eq!(
                        strip.read(&projected).to_bits(),
                        strip.sanitise(value).to_bits(),
                        "{name} 的投影值与恒等判定值必须同一个"
                    );
                }
                DeviceParamField::Reverb(reverb) => {
                    let projected = reverb_of(&device(&[(name, value)]))
                        .unwrap_or_else(|| panic!("{name}={value} 必须被 reverb_of 认出来"));
                    let mut device = Reverb::new();
                    device.set_sample_rate(48_000.0);
                    device.set_params(projected);
                    assert_eq!(
                        reverb.read(&device.params()).to_bits(),
                        reverb.sanitise(value).to_bits(),
                        "{name} 的器件读数与恒等判定值必须同一个"
                    );
                }
                DeviceParamField::Convolution(convolution_field) => {
                    let (projected, _, _) = convolution_projection(&device(&[(name, value)]))
                        .unwrap_or_else(|| panic!("{name}={value} 必须被 convolution_of 认出来"));
                    let mut device = ConvolutionReverb::new();
                    device.set_sample_rate(48_000.0);
                    device.set_params(projected);
                    assert_eq!(
                        convolution_field.read(&device.params()).to_bits(),
                        convolution_field.sanitise(value).to_bits(),
                        "{name} 的器件读数与恒等判定值必须同一个"
                    );
                }
            }
        }
    }

    /// 别名与规范名指向**同一个**落点（⛔ 不允许一个别名被投影认、被自动化拒）。
    #[test]
    fn aliases_resolve_to_the_same_field() {
        for field in StripField::ALL {
            assert_eq!(StripField::from_name(field.canonical_name()), Some(*field));
        }
        for field in ReverbField::ALL {
            assert_eq!(ReverbField::from_name(field.canonical_name()), Some(*field));
        }
        for field in ConvolutionField::ALL {
            assert_eq!(
                ConvolutionField::from_name(field.canonical_name()),
                Some(*field)
            );
        }
        for (alias, field) in [
            ("input_gain", StripField::InputGainDb),
            ("trim_db", StripField::InputGainDb),
            ("output_gain", StripField::OutputGainDb),
            ("fader_db", StripField::OutputGainDb),
            ("low_gain", StripField::EqLowGain),
            ("low_freq", StripField::EqLowFreq),
            ("mid_gain", StripField::EqMidGain),
            ("mid_freq", StripField::EqMidFreq),
            ("mid_q", StripField::EqMidQ),
            ("high_gain", StripField::EqHighGain),
            ("high_freq", StripField::EqHighFreq),
            ("filter_resonance", StripField::Resonance),
            ("filter_drive", StripField::Drive),
            // ⭐ `#304`：压缩 6 个别名（每个都指向与规范名同一个落点）。
            ("threshold", StripField::ThresholdDb),
            ("knee", StripField::KneeDb),
            ("detector", StripField::DetectorS),
            ("attack", StripField::AttackS),
            ("release", StripField::ReleaseS),
            ("makeup", StripField::MakeupDb),
        ] {
            assert_eq!(StripField::from_name(alias), Some(field), "{alias}");
            assert_eq!(
                StripField::from_name(&alias.to_ascii_uppercase()),
                Some(field),
                "大小写不敏感：{alias}"
            );
        }
        for (alias, field) in [
            ("reverb_wet_mix", ReverbField::Wet),
            ("reverb_predelay", ReverbField::Predelay),
        ] {
            assert_eq!(
                DeviceParamField::from_name(alias),
                Some(DeviceParamField::Reverb(field)),
                "{alias}"
            );
        }
        for (alias, field) in [
            ("conv_pre_delay_s", ConvolutionField::PreDelay),
            ("conv_predelay", ConvolutionField::PreDelay),
            ("conv_wet_mix", ConvolutionField::Wet),
            ("conv_ir_gain", ConvolutionField::IrGainDb),
        ] {
            assert_eq!(
                DeviceParamField::from_name(alias),
                Some(DeviceParamField::Convolution(field)),
                "{alias}"
            );
        }
    }

    /// ⛔ IR 形状 / 未知名字**不是**设备参数落点；混响/卷积标量**不是**
    /// 通道条字段（但它们有各自的落点）。⭐ `#304` 起压缩 14 拼写**是**通道条字段。
    #[test]
    fn non_applied_names_are_not_device_param_fields() {
        for name in [
            "conv_ir_decay_s",
            "conv_ir_decay",
            "conv_ir_seed",
            "reverb_mix",
            "reverb_width",
            "filter_cutoff_hz",
            "mix",
            "",
        ] {
            assert_eq!(DeviceParamField::from_name(name), None, "{name}");
            assert_eq!(StripField::from_name(name), None, "{name}");
        }
        // ⭐ `#304`：压缩 14 拼写全部有落点（与静态投影同一张表）。
        for name in [
            "compressor_enabled",
            "threshold_db",
            "threshold",
            "ratio",
            "knee_db",
            "knee",
            "detector_s",
            "detector",
            "attack_s",
            "attack",
            "release_s",
            "release",
            "makeup_db",
            "makeup",
        ] {
            assert!(
                DeviceParamField::from_name(name).is_some(),
                "{name} 自 #304 起必须有设备参数落点"
            );
            assert_eq!(
                DeviceParamField::from_name(name),
                StripField::from_name(name).map(DeviceParamField::Strip),
                "{name} 必须解析到通道条字段"
            );
        }
        for name in [
            "reverb_size",
            "reverb_damp",
            "reverb_wet",
            "reverb_predelay",
            "conv_predelay_s",
            "conv_dry",
            "conv_wet",
            "conv_ir_gain_db",
        ] {
            assert_eq!(StripField::from_name(name), None, "{name} 不是通道条字段");
            assert!(
                DeviceParamField::from_name(name).is_some(),
                "{name} 必须有设备参数落点"
            );
        }
    }

    /// 钳制是**器件那一份**（⛔ 不复制上下界）：几个不对称的边界。
    #[test]
    fn the_field_clamp_is_the_device_clamp() {
        assert_eq!(StripField::CutoffHz.sanitise(1.0e9), MAX_CUTOFF_HZ);
        assert_eq!(
            StripField::CutoffHz.sanitise(f32::NAN).to_bits(),
            MAX_CUTOFF_HZ.to_bits(),
            "非有限截止频率归**上界**（低通最保守的一侧）"
        );
        assert_eq!(StripField::CutoffHz.sanitise(1.0), MIN_CUTOFF_HZ);
        assert_eq!(StripField::OutputGainDb.sanitise(-1.0e9), MIN_GAIN_DB);
        assert_eq!(StripField::OutputGainDb.sanitise(1.0e9), MAX_GAIN_DB);
        assert_eq!(
            StripField::InputGainDb.sanitise(f32::NAN).to_bits(),
            0.0f32.to_bits(),
            "非有限增益归 0 dB（逐位透明）"
        );
        assert_eq!(StripField::Resonance.sanitise(5.0), 1.0);
        assert_eq!(StripField::Drive.sanitise(-5.0), 0.0);
        assert_eq!(StripField::EqEnabled.sanitise(0.0), 0.0);
        assert_eq!(StripField::EqEnabled.sanitise(0.25), 1.0);
        assert_eq!(StripField::EqEnabled.sanitise(-2.0), 1.0);
        // ⭐ `#304`：压缩字段的钳制走同一次 `ChannelStripParams::sanitised`。
        assert_eq!(StripField::CompressorEnabled.sanitise(0.0), 0.0);
        assert_eq!(StripField::CompressorEnabled.sanitise(0.25), 1.0);
        assert_eq!(StripField::CompressorEnabled.sanitise(-2.0), 1.0);
        assert_eq!(StripField::ThresholdDb.sanitise(-1.0e9), MIN_LEVEL_DB);
        assert_eq!(StripField::ThresholdDb.sanitise(1.0e9), MAX_THRESHOLD_DB);
        assert_eq!(StripField::Ratio.sanitise(0.0), 1.0);
        assert_eq!(StripField::Ratio.sanitise(1.0e9), MAX_RATIO);
        assert_eq!(StripField::KneeDb.sanitise(-1.0), 0.0);
        assert_eq!(StripField::KneeDb.sanitise(1.0e9), MAX_KNEE_DB);
        assert_eq!(
            StripField::DetectorS.sanitise(f32::NAN).to_bits(),
            MIN_TIME_S.to_bits(),
            "非有限时间常数归下界（与器件同一条裁决）"
        );
        assert_eq!(StripField::MakeupDb.sanitise(1.0e9), MAX_MAKEUP_DB);
    }

    /// ⭐ `#293`：混响字段的钳制镜像必须与**真实器件**逐位一致（含非有限值与越界）。
    ///
    /// 这是“⛔ 不是会漂移的第二套钳制”的机械证明：`yeban-dsp` 的 sanitise 是私有的，
    /// 这里只能镜像；对账对象是 `Reverb::new → set_params → params()` 的真读数。
    #[test]
    fn reverb_field_clamp_is_the_device_clamp() {
        for field in ReverbField::ALL {
            for value in [
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                -1.0,
                0.0,
                0.25,
                0.72,
                1.0,
                2.0,
                f32::MIN,
                f32::MAX,
            ] {
                let mut projected = ReverbParams::default();
                field.store(&mut projected, value);
                let mut device = Reverb::new();
                device.set_sample_rate(48_000.0);
                device.set_params(projected);
                assert_eq!(
                    field.sanitise(value).to_bits(),
                    field.read(&device.params()).to_bits(),
                    "{field:?} {value}"
                );
            }
        }
    }

    /// ⭐ `#293`：卷积标量字段的钳制镜像必须与**真实器件**逐位一致。
    #[test]
    fn convolution_field_clamp_is_the_device_clamp() {
        for field in ConvolutionField::ALL {
            for value in [
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                -1.0,
                0.0,
                0.05,
                0.25,
                0.75,
                1.0,
                2.0,
                f32::MIN,
                f32::MAX,
            ] {
                let mut projected = ConvolutionReverbParams::default();
                field.store(&mut projected, value);
                let mut device = ConvolutionReverb::new();
                device.set_sample_rate(48_000.0);
                device.set_params(projected);
                assert_eq!(
                    field.sanitise(value).to_bits(),
                    field.read(&device.params()).to_bits(),
                    "{field:?} {value}"
                );
            }
        }
    }

    /// ⭐ `#293` 判据 2（机制层）：静态 `reverb_wet: 0` 的链在**不强制**时**不建**器件；
    /// 一置 `force_reverb`，运行态就真的有一台 Freeverb 可让块边界改它。
    ///
    /// 没有这一位，`attach_device_param_automation` 会成功、MCP 会报 applied，而
    /// `apply_device_params` 找不到 `self.reverb` ⇒ **静默无效**。
    #[test]
    fn forced_reverb_keeps_a_wet_zero_instance_alive() {
        let chain = InsertChain::from_devices(&[device(&[("reverb_wet", 0.0)])], 48_000);
        assert!(chain.reverb().is_some(), "静态投影认得 `reverb_wet`");
        assert!(!chain.is_empty(), "链本身非空");
        let quiet = InsertRuntime::new(&chain, 48_000, 128);
        assert!(
            quiet.reverb.is_none(),
            "没有强制位 ⇒ 静态 `wet = 0` 仍整级跳过（既有口径）"
        );

        let forced = chain.with_forced_reverb(true);
        assert!(forced.is_forced_reverb());
        let mut runtime = InsertRuntime::new(&forced, 48_000, 128);
        assert!(
            runtime.reverb.is_some(),
            "强制位 ⇒ 运行态必须有器件，否则泳道静默无效"
        );

        // 真的能改：从 0 拉到 1 之后器件是 active 的。
        runtime.apply_device_params(DeviceParamField::Reverb(ReverbField::Wet), 1.0);
        let reverb = runtime.reverb.as_ref().expect("器件");
        assert_eq!(reverb.params().mix.to_bits(), 1.0f32.to_bits());
        assert!(reverb.is_active());
    }

    /// ⭐ `#293` 判据 2（机制层）：卷积同一条缝。
    #[test]
    fn forced_convolution_keeps_a_wet_zero_instance_alive() {
        let chain = InsertChain::from_devices(&[device(&[("conv_wet", 0.0)])], 48_000);
        assert!(chain.convolution().is_some());
        let quiet = InsertRuntime::new(&chain, 48_000, 128);
        assert!(quiet.convolution.is_none(), "静态 `wet = 0` 整级跳过");

        let forced = chain.with_forced_convolution(true);
        assert!(forced.is_forced_convolution());
        let mut runtime = InsertRuntime::new(&forced, 48_000, 128);
        assert!(runtime.convolution.is_some(), "强制位 ⇒ 运行态必须有器件");
        runtime.apply_device_params(DeviceParamField::Convolution(ConvolutionField::Wet), 1.0);
        let convolution = runtime.convolution.as_ref().expect("器件");
        assert_eq!(convolution.params().wet.to_bits(), 1.0f32.to_bits());
        assert!(convolution.is_active());
    }

    /// 强制实例化 + 没有投影来源 ⇒ 器件拿到的就是 [`forced_identity_strip`]
    /// （静态基值与器件参数**同一份**）。
    #[test]
    fn a_forced_strip_without_a_source_uses_the_identity_params() {
        let chain = InsertChain::empty().with_forced_strip(true);
        assert_eq!(chain.effective_strip(), Some(forced_identity_strip()));
        let params = forced_identity_strip();
        assert!(!params.eq_enabled && !params.filter_enabled && !params.compressor_enabled);
        assert_eq!(params.input_gain_db.to_bits(), 0.0f32.to_bits());
        assert_eq!(params.output_gain_db.to_bits(), 0.0f32.to_bits());
    }
}
