//! **每轨插入链**的离线投影与运行态：[EQ] ＋ [Freeverb] ＋ [卷积混响]。
//!
//! ## 0. 这一份是**副本** —— 必须下移到 `yeban-dsp`（待办）
//!
//! 投影规则不是本模块发明的，逐条抄自实时引擎：
//!
//! | 内容 | 来源（base `1d60e5eb`） |
//! | :--- | :--- |
//! | `InsertParams::from_devices` 的"首个含已识别参数名的设备是来源"规则 | `crates/yeban-engine/src/insert.rs:704` |
//! | 通道条参数名白名单（EQ 部分） | `crates/yeban-engine/src/insert.rs:886-975` |
//! | 混响参数名白名单（⚠ `reverb_mix` / `reverb_width` **刻意不认**） | `crates/yeban-engine/src/insert.rs:1055-1070` |
//! | 卷积混响参数名白名单 ＋ IR 合成 | `crates/yeban-engine/src/insert.rs:622-654` |
//! | 三件 DSP 本体 | `crates/yeban-dsp/src/shaping.rs:123`（3 段 EQ）、`reverb.rs:239`（Freeverb）、`convolution_reverb.rs:182` |
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
//! **接**：3 段 EQ（低架 / 中峰 / 高架）＋ Freeverb ＋ 卷积混响。
//!
//! **没接**（刻意，留 S2）：
//!
//! - **压缩**与**滤波**：实时侧是**单声道**插入（检波器看到的是单声道），离线侧接在
//!   **求和之后**、是立体声；把压缩接在这里会让左右声道各有一个检波器 ⇒ 声像被拉开。
//!   那需要对"立体声 vs 单声道检波"单独裁决，不在本切片内。
//! - **输入 / 输出增益**：同属通道条，但本切片只接 EQ；写了这些名字的设备会被
//!   [`track_has_unhandled_dsp`] 判成"仍有未消费的参数"⇒ MCP 如实登记 `deviceChainDsp`。
//! - **主总线插入**（⚠ 本节在任务单 `#173` **已接通**，原文见 §1 的修订段）：
//!   本模块的 [`crate::render::RenderPlan`] 现在给**每个**节点挂链，**包括 master**。
//!   - **为什么改**：`#173` 之前实时引擎的逐轨循环**跳过 master**
//!     （`crates/yeban-engine/src/rt.rs` 的 `render_branch` 只处理轨道），而离线侧
//!     也把 master 的链写成 `None` ⇒ 主总线上挂设备**两条路都听不到**。
//!   - **改法（实时）**：主总线在汇流之后（3a⁗⁗）跑**同一条**链：立体声和
//!     `(L+R)·0.5` 下混 → EQ → 混响 → 卷积 → 复制回两路（裁决 (甲)，与 `#158`
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
//! 实时：声源 → 插入（单声道）→ 声相 → 母线求和
//! 离线：声源 → 声相 → 母线求和 → 插入（立体声）
//! ```
//!
//! - **EQ 逐声道线性等价**：声相是每声道的常数增益，线性时不变滤波与常数乘法可交换
//!   （`EQ(p·x) = p·EQ(x)`），所以 EQ 的位置差异不改变可听结果（只差浮点舍入顺序）。
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
//!
//! ## 4. 音频线程边界
//!
//! [`InsertRuntime`] 的全部缓冲在构造期分配（[`InsertRuntime::new`]）。逐块
//! [`InsertRuntime::process`] 只有环形缓冲读写与乘加：**零分配、零锁、零 I/O**。

use std::collections::BTreeMap;

use yeban_dsp::convolution_reverb::{ConvolutionReverb, ConvolutionReverbParams};
use yeban_dsp::noise::Rng;
use yeban_dsp::reverb::{Reverb, ReverbParams};
use yeban_dsp::shaping::{EqParams, ShapingEq};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, TrackV3};

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

/// 一条轨的**插入链投影**（本切片三件：EQ、Freeverb、卷积混响）。
///
/// 三件**各自独立**取来源（与引擎同口径）：同一台设备可以同时是三者或其中之一的来源。
/// `None` 表示"本轨没有这一件" ⇒ 运行态整级跳过。
#[derive(Clone, Debug, PartialEq)]
pub struct InsertChain {
    /// EQ 三段的参数；`None` = 没有可执行的 EQ 级。
    eq: Option<EqParams>,
    /// Freeverb 参数；`None` = 本轨没有混响。
    reverb: Option<ReverbParams>,
    /// 卷积混响计划；`None` = 本轨没有卷积混响。
    convolution: Option<ConvolutionPlan>,
}

impl InsertChain {
    /// **空链**：本轨没有可执行的插入器件。
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            eq: None,
            reverb: None,
            convolution: None,
        }
    }

    /// 从模型层的设备链投影（**构造期**，允许分配：卷积的 IR 在这里合成）。
    ///
    /// 规则与引擎逐条一致：
    ///
    /// 1. 只看**非旁通**的 [`DeviceKind::InternalEffect`]；
    /// 2. 三件各自取**第一个**含该件已识别参数名的设备（先命中者不被后来者覆盖）；
    /// 3. 未出现的字段保持 `yeban-dsp` 的默认值；
    /// 4. 同名参数**后者胜**；参数名大小写不敏感；
    /// 5. `eq_enabled = 0` 或全部 EQ 旋钮都没写 ⇒ 没有 EQ 级（不会"沉默地开一条平坦 EQ"）。
    #[must_use]
    pub fn from_devices(devices: &[DeviceDefinition], sample_rate: u32) -> Self {
        let mut eq: Option<EqParams> = None;
        let mut reverb: Option<ReverbParams> = None;
        let mut convolution: Option<ConvolutionPlan> = None;
        for device in devices {
            if device.bypassed || device.kind != DeviceKind::InternalEffect {
                continue;
            }
            if eq.is_none() {
                eq = eq_of(device);
            }
            if reverb.is_none() {
                reverb = reverb_of(device);
            }
            if convolution.is_none() {
                convolution = convolution_of(device, sample_rate);
            }
            if eq.is_some() && reverb.is_some() && convolution.is_some() {
                break;
            }
        }
        Self {
            eq,
            reverb,
            convolution,
        }
    }

    /// 本链是否为空（没有任何可执行的器件）。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.eq.is_none() && self.reverb.is_none() && self.convolution.is_none()
    }

    /// 投影出来的 EQ 参数（判据用：把"将要交给器件的那个数"变成可读的）。
    #[must_use]
    pub const fn eq(&self) -> Option<EqParams> {
        self.eq
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
    tracks
        .iter()
        .filter_map(|(&id, track)| {
            let chain = InsertChain::from_devices(&track.devices, sample_rate);
            if chain.is_empty() {
                None
            } else {
                Some((id, chain))
            }
        })
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
/// - 有**非旁通**设备的任一参数名不在本切片三件器件的白名单里（滤波 / 压缩 / 增益 /
///   未知第三方旋钮都算）。
///
/// 全部非旁通设备都是本切片认识的内置效果器参数 ⇒ `false` ⇒ MCP 不再登记
/// `deviceChainDsp`（报一个已经做到的键就是假话）。
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
/// 刻意**不含**：`input_gain_db` / `output_gain_db` / `filter_*` / `cutoff_hz` /
/// 压缩的七个名字 / `reverb_mix` / `reverb_width` —— 它们要么留 S2，要么与单声道
/// 检波口径绑定。写这些名字的设备会被 [`track_has_unhandled_dsp`] 如实登记。
fn is_supported_param(name: &str) -> bool {
    matches!(
        name,
        "eq_enabled"
            | "eq_low_gain"
            | "low_gain"
            | "eq_low_freq"
            | "low_freq"
            | "eq_mid_gain"
            | "mid_gain"
            | "eq_mid_freq"
            | "mid_freq"
            | "eq_mid_q"
            | "mid_q"
            | "eq_high_gain"
            | "high_gain"
            | "eq_high_freq"
            | "high_freq"
            | "reverb_size"
            | "reverb_damp"
            | "reverb_wet"
            | "reverb_wet_mix"
            | "reverb_predelay"
            | "reverb_predelay_s"
            | "conv_predelay_s"
            | "conv_pre_delay_s"
            | "conv_predelay"
            | "conv_dry"
            | "conv_wet"
            | "conv_wet_mix"
            | "conv_ir_gain_db"
            | "conv_ir_gain"
            | "conv_ir_decay_s"
            | "conv_ir_decay"
            | "conv_ir_seed"
    )
}

/// 单台设备 → EQ 参数（`None` = 没有可执行的 EQ 级）。
fn eq_of(device: &DeviceDefinition) -> Option<EqParams> {
    let mut params = EqParams::default();
    let mut seen = false;
    let mut enabled: Option<bool> = None;
    for param in &device.params {
        let name = param.name.to_ascii_lowercase();
        let value = param.value;
        match name.as_str() {
            "eq_enabled" => enabled = Some(value != 0.0),
            "eq_low_gain" | "low_gain" => {
                params.low_gain = value;
                seen = true;
            }
            "eq_low_freq" | "low_freq" => {
                params.low_freq = value;
                seen = true;
            }
            "eq_mid_gain" | "mid_gain" => {
                params.mid_gain = value;
                seen = true;
            }
            "eq_mid_freq" | "mid_freq" => {
                params.mid_freq = value;
                seen = true;
            }
            "eq_mid_q" | "mid_q" => {
                params.mid_q = value;
                seen = true;
            }
            "eq_high_gain" | "high_gain" => {
                params.high_gain = value;
                seen = true;
            }
            "eq_high_freq" | "high_freq" => {
                params.high_freq = value;
                seen = true;
            }
            _ => {}
        }
    }
    // 与引擎 §4.1 同口径：`eq_enabled` 写过的优先，否则"写了旋钮才开"。
    if enabled.unwrap_or(seen) {
        Some(params)
    } else {
        None
    }
}

/// 单台设备 → 混响参数（`None` = 本设备不是混响来源）。
///
/// ⚠ `reverb_mix` / `reverb_width` **刻意不认**：与引擎的 `ReverbProjection::apply`
/// 逐字同口径（这两个名字属于"单声道插入点"那一组，见引擎模块文档 §8.1）。
fn reverb_of(device: &DeviceDefinition) -> Option<ReverbParams> {
    let mut params = ReverbParams::default();
    let mut seen = false;
    for param in &device.params {
        let name = param.name.to_ascii_lowercase();
        let value = param.value;
        match name.as_str() {
            "reverb_size" => {
                params.size = value;
                seen = true;
            }
            "reverb_damp" => {
                params.damp = value;
                seen = true;
            }
            "reverb_wet" | "reverb_wet_mix" => {
                params.mix = value;
                seen = true;
            }
            "reverb_predelay" | "reverb_predelay_s" => {
                params.predelay = value;
                seen = true;
            }
            _ => {}
        }
    }
    if seen { Some(params) } else { None }
}

/// 单台设备 → 卷积计划（`None` = 本设备不是卷积来源）。
fn convolution_of(device: &DeviceDefinition, sample_rate: u32) -> Option<ConvolutionPlan> {
    let mut params = ConvolutionReverbParams::default();
    let mut decay_s = CONV_IR_DEFAULT_DECAY_S;
    let mut seed = CONV_IR_DEFAULT_SEED as u32;
    let mut seen = false;
    for param in &device.params {
        let name = param.name.to_ascii_lowercase();
        let value = param.value;
        match name.as_str() {
            "conv_predelay_s" | "conv_pre_delay_s" | "conv_predelay" => {
                params.pre_delay_s = value;
                seen = true;
            }
            "conv_dry" => {
                params.dry = value;
                seen = true;
            }
            "conv_wet" | "conv_wet_mix" => {
                params.wet = value;
                seen = true;
            }
            "conv_ir_gain_db" | "conv_ir_gain" => {
                params.ir_gain_db = value;
                seen = true;
            }
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
    if !seen {
        return None;
    }
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
    /// EQ 状态（`Some` 时 `eq_params` 是这一份参数）。
    eq: Option<ShapingEq>,
    /// EQ 参数（`eq` 为 `Some` 时有效）。
    eq_params: EqParams,
    /// Freeverb 状态；`None` = 不动（`mix = 0` 或没有）。
    reverb: Option<Reverb>,
    /// 卷积混响状态；`None` = 不动（`wet = 0` 或没有）。
    convolution: Option<ConvolutionReverb>,
    /// 采样率（Hz，构造期钳制后的值由器件负责）。
    sample_rate: f32,
    /// 左声道去交错缓冲。
    left: Vec<f32>,
    /// 右声道去交错缓冲。
    right: Vec<f32>,
    /// EQ 输出左缓冲。
    eq_left: Vec<f32>,
    /// EQ 输出右缓冲。
    eq_right: Vec<f32>,
}

impl InsertRuntime {
    /// 从一条非空链构造运行态（`block_size` 是单块帧数，`sample_rate` 是采样率）。
    ///
    /// **只在控制路径调用**（允许分配：卷积核与混响延迟线在这里建起来）。
    pub(crate) fn new(chain: &InsertChain, sample_rate: u32, block_size: usize) -> Self {
        let sample_rate_f32 = sample_rate as f32;
        let (eq, eq_params) = match chain.eq {
            Some(params) => (Some(ShapingEq::new()), params),
            None => (None, EqParams::default()),
        };
        let reverb = chain.reverb.and_then(|params| {
            let mut state = Reverb::new();
            state.set_sample_rate(sample_rate_f32);
            state.set_params(params);
            if state.is_active() { Some(state) } else { None }
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
            if state.is_active() { Some(state) } else { None }
        });
        Self {
            eq,
            eq_params,
            reverb,
            convolution,
            sample_rate: sample_rate_f32,
            left: vec![0.0; block_size],
            right: vec![0.0; block_size],
            eq_left: vec![0.0; block_size],
            eq_right: vec![0.0; block_size],
        }
    }

    /// 复位全部器件的时间状态（下一次 `execute` 与"刚编译"逐位等价）。
    pub(crate) fn reset(&mut self) {
        if let Some(eq) = &mut self.eq {
            eq.reset();
        }
        if let Some(reverb) = &mut self.reverb {
            reverb.reset();
        }
        if let Some(convolution) = &mut self.convolution {
            convolution.reset();
        }
    }

    /// 原地处理一个**立体声交错**块（`buffer.len() == frames * 2`）。
    ///
    /// 顺序与实时引擎同一条链：EQ → Freeverb → 卷积混响。各级在参数为 0 时整级
    /// 跳过（状态不推进），与器件自己的 `is_active` 守卫同口径。
    pub(crate) fn process(&mut self, buffer: &mut [f32]) {
        let frames = buffer.len() / 2;
        if frames == 0 {
            return;
        }
        let Self {
            eq,
            eq_params,
            reverb,
            convolution,
            sample_rate,
            left,
            right,
            eq_left,
            eq_right,
        } = self;
        for frame in 0..frames {
            left[frame] = buffer[2 * frame];
            right[frame] = buffer[2 * frame + 1];
        }
        if let Some(eq) = eq {
            eq.process(
                &left[..frames],
                &right[..frames],
                &mut eq_left[..frames],
                &mut eq_right[..frames],
                *eq_params,
                *sample_rate,
            );
            left[..frames].copy_from_slice(&eq_left[..frames]);
            right[..frames].copy_from_slice(&eq_right[..frames]);
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
