//! 立体声混响：Freeverb 拓扑 + 合成器音色需要的控制量。[ARCH-RT-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/reverb.rs`（含其全部回归测试）。
//!
//! 每声道 8 个带阻尼的梳状滤波器 + 4 个全通扩散器，经典立体声展宽，加上被替换掉的
//! Soundpipe 混响没有的控制：高频**阻尼**、**预延迟**与立体声**宽度**。延迟线是
//! 刻意静态的——反馈环里的移动抽头会在延迟变短时注入能量，那会让尾巴"泵"起来
//! 而不是衰减。
//!
//! 延迟线在设置采样率时分配一次，因此渲染循环本身从不分配 [ARCH-RT-001]。
//!
//! ## 与来源的差异
//!
//! 1. [`Reverb::set_sample_rate`] 被明确标注为本类型**唯一**的分配入口，
//!    必须在音频回调之外调用；
//! 2. 存储真实采样率（来源在 `set_params` 里用 `44_100.0 * sr_scale` 反推，
//!    当采样率落在 `[22.05 kHz, 96 kHz]` 之外时那个反推值是错的）；
//! 3. `process` 增加"未配置即直通"的守卫：来源在从未调用 `set_sample_rate`
//!    时会在 `self.pre[0][self.pre_index]` 上越界 panic，而实时路径上不允许；
//! 4. `process` 用 `zip` 而不是按下标写 `right[i]`（同上，长度不等就会 panic）；
//! 5. 立体声展宽与前置延迟的事件顺序、参数映射常量（0.70…0.94 反馈、
//!    0.05…0.77 阻尼、`WET_GAIN`）逐一保持与来源相同，既有音色不变；
//! 6. **新增** [`Reverb::reset`] 与 [`Reverb::latency_samples`]（＋公共常量
//!    [`REVERB_LATENCY`]）—— 来源没有这两项，见下一节。两者都是**纯增量**：
//!    `process` 的逐样本路径一个字都没改，既有音色与既有读数不变。
//!
//! ## 器件形状与延迟上报 [ARCH-PDC-001]
//!
//! 本类型与 `compressor` / `channel_strip` / `limiter` / `polysynth` / `drums` /
//! `convolution_reverb` 同形：
//! `new` / `set_sample_rate` / `set_params` / `params` / `is_configured` /
//! `is_active` / `process` / `reset` / `latency_samples`。
//!
//! 后两项是本线按 [ARCH-PDC-001] 补齐的。规范原文（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`
//! 第 398 行）要求"每个插件与内置设备必须精确上报其引入的处理延迟
//! （`DeviceDefinition::latency_samples`）"。Freeverb 的预延迟**只**推迟送进梳状组的
//! 湿信号，干路逐样本即时 ⇒ 本器件的净延迟是 **0**，见 [`Reverb::latency_samples`]。
//! 口径与 [`crate::convolution_reverb`] 模块文档 §2.1 相同。
//!
//! [`Reverb::reset`] 是"清历史、不动配置"的唯一入口：停带、定位或换工程时，
//! 反馈环里的尾巴必须有一个定义明确的去处（否则同一段输入在第二次渲染里与第一次
//! 不同）。它逐样本零分配。
//!
//! ## 5. 非有限**输入样本**（本轮补齐的成员）
//!
//! [`Reverb::set_params`] 早已把非有限**参数**回落成缺省值，理由写在那个函数的
//! 文档里（"会把整个梳状组的反馈变成 `NaN` 并静默毒化整条混响总线"）。
//! **输入样本走的是同一条路**，而本条此前缺失：`NaN`／`±∞` 写进预延迟线后，
//! `input = (dl + dr) · 0.5 · 0.015` 立刻把它们送进 8 条梳状线 ＋ 4 个全通，
//! 而反馈环里 `NaN · damp = NaN`、`∞ · feedback = ∞` ⇒ **一次**坏样本之后
//! 湿路永久非有限，连输入恢复干净也回不来（`reset()` 是唯一出路）。
//!
//! ⇒ [`Reverb::process`] 在**每样本进入递归之前**过 `math::finite_or_zero`：
//! `NaN` 与 `±∞` 归 `0.0`，**有限样本逐位不变**。因此
//! 1. 既有音色、既有读数、既有判据一个比特都不改；
//! 2. `NaN` 样本与"同一位置的 `0.0` 样本"从此**逐位同解**（这正是
//!    `tests/non_finite_never_enters_a_recursion.rs` 钉住的形态）。
//!
//! ⚠ 本守卫与"未配置／湿路不可闻即直通"的短路**不冲突**：[`Reverb::process`] 的
//! 提前返回发生在守卫之前，因此旁通状态仍是**逐位直通**（含非有限样本）。
//! 这不是漏洞：没有样本进入递归，就没有东西可毒化。
//!
//! ⚠ 守卫**不**覆盖"有限但极大"的输入（`3e38` 仍可能溢出成 `∞`）。那条归调用方的
//! 电平口径管，见 `math::finite_or_zero` 的文档。
//!
//! ## 6. 预延迟**长度变更**时清零（本轮的缺口修复）
//!
//! "秒 → 帧"的读数（`pre_len`）**变了**的时候，[`Reverb::set_params`] 会把两条预延迟线
//! 清零并把写头归零。长度不变时一个字节都不动（§5 的有限样本口径与既有音色因此不受影响）。
//!
//! 为什么必须清：两条线都按 `PREDELAY_MAX` 一次性分配、**永不缩小**，而写头只在
//! `0..pre_len` 上回绕。长度变长时新写头会走进 `[旧 pre_len, 新 pre_len)` 这一段 ——
//! 里面的样本是**上一次用更长延迟**时写进去、此后从未被读出的旧音频。它们会在新延迟
//! 该给静音的地方突然出现在湿路里（一段"幽灵回声"），而且**取决于上一次的处理历史**
//! ⇒ 同一段输入、同一组参数，因为中间拖过一次旋钮而得到不同的输出，这不是
//! [ARCH-DET-001] 允许的可复现输出。
//!
//! 器件实测（本机 aarch64，本票读数；`mix = 1.0` ⇒ 干路为 0）：短延迟 `46` 帧的两条线用
//! `46` 帧常数灌满（读在写之前 ⇒ 这一段输出逐样本为 `0`，梳状组一次也没被激励），随后把
//! 长度改成 `1500` 帧、喂 `3 000` 帧静音。清零前：输出峰值 `2.864376e-1`、首个非零样本在
//! 第 `1214` 帧（= `1116 · 48000/44100`，最短梳状延迟），与"刚按 `1500` 帧装配好"的实例
//! 相差 `2938/6000` 个比特；清零后两条读数都是 `0`、比特差异 `0/6000`。判据
//! `reverb::tests::changing_the_pre_delay_length_never_replays_stale_audio` 钉住它。
//!
//! ⚠ 代价与边界（不隐藏）：本器件**没有**做交叉淡化，也**没有**做分数延迟插值 ——
//! 那需要第二条读头与混合状态（另一张票）。长度只按整帧变化；持续拖动 `predelay` 时，
//! 每当"秒 → 帧"的读数跨过一帧就清一次线（两条线各 `PREDELAY_MAX` = `9 600` 个 `f32`
//! 的原地写，**零分配**，
//! 由 `tests/reverb_rt_zero_alloc.rs` 判据 1 的"每 `250` 个量子换一次 `predelay`"与
//! 判据 4 的全程扫描覆盖）。要平滑拖动，调用方应在器件之外做参数平滑。
//! 口径与 [`crate::convolution_reverb`] 模块文档 §2.2 相同。
//!
//! ## 7. 类型与混响时间（本轮新增）
//!
//! [`ReverbKind`] 给 Freeverb 拓扑加了**空间类型**面（Plate/Room/Hall/Spring），
//! [`ReverbParams::decay_s`] 给了**按秒写**的混响时间面 —— 两者都是鼓机预置
//! 逐风格点名却无处可写的量（缺口 P1/P2）。实现路线如实写明：
//!
//! 1. **类型 = 参数组 + 长度布局，不是独立算法**。四档共用同一组梳状+全通，
//!    靠长度缩放（早期反射稀疏度）、阻尼偏移（亮度）、全通反馈（扩散/色散）
//!    区分。`Spring` 档是串联全通强色散的参数组**近似**，不是弹簧物理模型。
//! 2. **Room 是旧调音本身**，且 `decay_s` 未设时反馈走旧公式 ⇒ 既有工程、
//!    既有判据逐位不变；类型与时间都是**增量**面。
//! 3. **时间口径只此一处汇合**（`Reverb::apply_gain_model`）：`decay_s > 0`
//!    时逐梳按 T60 标定反馈；`decay_s == 0` 时 `size` 的旧公式说了算。
//!    ⛔ 没有第二条决定反馈的路径。
//! 4. **类型切换 = 重排 + 清历史**：长度变了，旧历史留在线里就是 §6 的
//!    幽灵回声。缓冲容量在 `set_sample_rate` 时按最大类型缩放一次预留，
//!    重排只改逻辑长度 ⇒ `set_params` 仍零分配（`reverb_rt_zero_alloc`
//!    的换类型场景覆盖）。切换瞬间尾巴断掉（无交叉淡化，同 §6 的取舍），
//!    调用方若在播放中换类型，应预期一次清尾。

use crate::math::finite_or_zero;

/// 梳状调音，单位样本 @44.1 kHz（Freeverb 的原始数字）。
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];

/// 全通调音，单位样本 @44.1 kHz。
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];

/// 右声道偏移量，用来展宽立体声像。
const STEREO_SPREAD: usize = 23;

/// 96 kHz 缩放与调制余量之后最长的梳状延迟。
const COMB_MAX: usize = 4096;

/// 全通延迟上限。
const ALLPASS_MAX: usize = 1536;

/// 预延迟缓冲：100 ms @96 kHz。
const PREDELAY_MAX: usize = 9600;

/// 预延迟的上界（秒）＝ [`PREDELAY_MAX`] 的 `9600 ÷ 96_000`。
///
/// ⭐ 本模块（`device_param`）落地后这是**唯一**的预延迟上界：`Reverb::set_params` 的钳制与
/// [`crate::device_param::ReverbField::Predelay`] 的落点钳制读**同一个**常量
/// （此前 `yeban_render::inserts` 里有一份只能镜像的私有 `0.1`，靠网格对账判据钉住）。
pub const REVERB_MAX_PREDELAY_S: f32 = 0.1;

/// 混响时间 [`ReverbParams::decay_s`] 的上界（秒，T60 口径）。
///
/// 与 [`REVERB_MAX_PREDELAY_S`] 同款：`Reverb::set_params` 的钳制与
/// [`crate::device_param::ReverbField::Decay`] 的落点钳制读**同一个**常量。
pub const REVERB_MAX_DECAY_S: f32 = 10.0;

/// −60 dB 的自然对数（`ln(1000)`）：T60 标定把「每往返衰减」换算成反馈系数用。
const T60_LN: f32 = 6.907_755_3;

/// T60 标定的实测修正系数（无量纲）。纯公式 `exp(−ln1000 · D/(sr·T60))` 只算
/// 了反馈本身；环路里的一极点阻尼对一切非直流内容幅值都 `< 1`，宽带尾巴因此
/// 比公式快约 11%。实测（48 kHz、斜率法 T60、目标 0.4/1.0/2.2 s）：未修正
/// 读数 0.364/0.890/1.961 s（一致偏短约 11%，且 damp 0 与 0.7 之间只差
/// 约 3 个百分点）⇒ 指数乘本系数后三档回到 ±3% 内。判据
/// `reverb::tests::decay_s_tracks_the_measured_t60` 钉住这组读数。
const T60_CALIBRATION: f32 = 0.89;

/// 本器件引入的**处理延迟**（帧）。`0` = 不参与 [ARCH-PDC-001] 的补偿量。
///
/// 取 `0` 的理由与 [`crate::convolution::CONV_LATENCY`] 同款：本器件的预延迟
/// **只**推迟湿路，干路用的是当前样本 ⇒ 整机对输入没有净延迟。
/// ⚠ 这意味着"预延迟"**不是** `ARCH-PDC-001` 的延迟，[`Reverb::latency_samples`]
/// 不把它算进去。
pub const REVERB_LATENCY: usize = 0;

/// 湿路径的输出微调。梳状组加四级全通扩散器有很高的谐振增益
///（在梳谐振处可达 ~25×），因此原始求和远热于干信号；这一项把全湿拉回大约单位增益。
const WET_GAIN: f32 = 0.4;

/// 从 44.1 kHz 调到最高支持采样率的缩放系数上限。
const SR_SCALE_MAX: f32 = 96_000.0 / 44_100.0;

/// 单条梳状线（内部类型，不对外暴露）。
struct Comb {
    /// 在 [`Reverb::set_sample_rate`] 里分配一次，渲染循环内不再触碰。
    buf: Vec<f32>,
    len: usize,
    index: usize,
    store: f32,
    damp: f32,
    feedback: f32,
}

impl Comb {
    const fn new() -> Self {
        Self {
            buf: Vec::new(),
            len: 1,
            index: 0,
            store: 0.0,
            damp: 0.2,
            feedback: 0.84,
        }
    }

    fn setup(&mut self, base_len: usize, sr_scale: f32, kind_scale: f32, alloc_len: usize) {
        self.len = ((base_len as f32 * sr_scale * kind_scale) as usize).clamp(64, COMB_MAX - 8);
        if self.buf.len() < alloc_len {
            self.buf = vec![0.0; alloc_len];
        } else {
            self.buf.fill(0.0);
        }
        self.index = 0;
        self.store = 0.0;
    }

    /// 类型切换时的**重排**：只改逻辑长度并清历史，⛔ 不触碰缓冲容量
    /// （容量在 [`Self::setup`] 时已按最大类型缩放预留）⇒ 零分配，可在
    /// `set_params` 里逐块调用 [ARCH-RT-001]。清历史的理由与预延迟改长
    /// 清线相同（模块文档 §6）：长度变长后旧样本会落在从未读出的区段，
    /// 变成与参数历史有关的幽灵回声。
    fn relayout(&mut self, base_len: usize, sr_scale: f32, kind_scale: f32) {
        self.len = ((base_len as f32 * sr_scale * kind_scale) as usize).clamp(64, COMB_MAX - 8);
        self.buf.fill(0.0);
        self.index = 0;
        self.store = 0.0;
    }

    /// 清空这条延迟线的**历史**（缓冲内容 ＋ 一极点阻尼状态 ＋ 读写头）。
    ///
    /// 长度与系数（`len` / `damp` / `feedback`）不动 —— 清的是历史，不是配置。
    /// 未配置时 `buf` 是空 `Vec`，`fill` 是空操作（不 panic、不分配）。
    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.index = 0;
        self.store = 0.0;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let out = self.buf[self.index];
        // 反馈路径里的一极点阻尼。
        self.store = out * (1.0 - self.damp) + self.store * self.damp;
        self.buf[self.index] = input + self.store * self.feedback;
        self.index += 1;
        if self.index >= self.len {
            self.index = 0;
        }
        out
    }
}

/// 单个全通扩散器（内部类型）。
struct Allpass {
    buf: Vec<f32>,
    len: usize,
    index: usize,
    /// 反馈系数（扩散/色散强度）：Freeverb 原值 `0.5`，由混响类型决定
    /// （[`ReverbKind::allpass_feedback`]），`set_params` 时写入。
    feedback: f32,
}

impl Allpass {
    const fn new() -> Self {
        Self {
            buf: Vec::new(),
            len: 1,
            index: 0,
            feedback: 0.5,
        }
    }

    fn setup(&mut self, base_len: usize, sr_scale: f32, kind_scale: f32, alloc_len: usize) {
        self.len = ((base_len as f32 * sr_scale * kind_scale) as usize).clamp(32, ALLPASS_MAX - 1);
        if self.buf.len() < alloc_len {
            self.buf = vec![0.0; alloc_len];
        } else {
            self.buf.fill(0.0);
        }
        self.index = 0;
    }

    /// 类型切换时的**重排**：与 [`Comb::relayout`] 同口径（只改逻辑长度、
    /// 清历史、零分配）。
    fn relayout(&mut self, base_len: usize, sr_scale: f32, kind_scale: f32) {
        self.len = ((base_len as f32 * sr_scale * kind_scale) as usize).clamp(32, ALLPASS_MAX - 1);
        self.buf.fill(0.0);
        self.index = 0;
    }

    /// 清空这条扩散器的**历史**（缓冲内容与读写头）。长度不动。
    ///
    /// 未配置时 `buf` 是空 `Vec`，`fill` 是空操作（不 panic、不分配）。
    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.index = 0;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buf[self.index];
        let out = -input + buffered;
        self.buf[self.index] = input + buffered * self.feedback;
        self.index += 1;
        if self.index >= self.len {
            self.index = 0;
        }
        out
    }
}

/// 混响**类型**（空间选择）：同一 Freeverb 拓扑上的四组调音，⛔ 不是四台
/// 独立算法。每档由三件事定义：延迟线**长度缩放**（早期反射的稀疏度与
/// 衰减标定的基数）、**阻尼偏移**（尾巴亮度）、**全通反馈**（扩散/色散
/// 强度）。差异由判据实测钉住（衰减包络与逐对差分，见本模块测试）。
///
/// - [`Self::Room`] 是**旧调音本身**：缩放 `1.0`、偏移 `0`、全通反馈
///   `0.5`，且 [`ReverbParams::decay_s`] 未设时反馈走旧公式 ⇒ 既有工程
///   逐位不变。它同时是缺省档：从未写过类型的设备就是 Room。
/// - [`Self::Plate`]：线更短更密、阻尼更低（更亮）、扩散更强 —— 板式
///   混响的稠密亮尾。
/// - [`Self::Hall`]：线更长（大空间的稀疏早期反射）、阻尼更高（更暗）、
///   扩散更弱。
/// - [`Self::Spring`]：线最短、全通反馈显著更高 —— 串联全通的强色散是
///   弹簧箱「啾啾」声的近似来源。⚠ 如实标注：这是**参数组近似**，不是
///   弹簧的物理模型（无三弹簧并联与机械色散曲线）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ReverbKind {
    /// 房间（旧调音，缺省）。
    #[default]
    Room,
    /// 板式。
    Plate,
    /// 大厅。
    Hall,
    /// 弹簧（参数组近似）。
    Spring,
}

impl ReverbKind {
    /// 全部类型（器件参数值的顺序即本顺序）。
    pub const ALL: &'static [Self] = &[Self::Room, Self::Plate, Self::Hall, Self::Spring];

    /// 长度缩放的最大值（缓冲预留按它算，见 `Reverb::set_sample_rate`）。
    const MAX_LENGTH_SCALE: f32 = 1.30;

    /// 器件参数面上的数值（`reverb_kind`）：`Room = 0`、`Plate = 1`、
    /// `Hall = 2`、`Spring = 3`。
    #[must_use]
    pub const fn index(self) -> f32 {
        match self {
            Self::Room => 0.0,
            Self::Plate => 1.0,
            Self::Hall => 2.0,
            Self::Spring => 3.0,
        }
    }

    /// 器件参数数值 → 类型：有限值四舍五入后钳到 `0..=3`；非有限值回落
    /// [`Self::Room`]（与 `set_params` 的退化输入回落同一口径：实时路径
    /// 上无法报错，只能就地回落到缺省档）。
    #[must_use]
    pub fn from_f32(value: f32) -> Self {
        if !value.is_finite() {
            return Self::Room;
        }
        match value.round().clamp(0.0, 3.0) as u32 {
            0 => Self::Room,
            1 => Self::Plate,
            2 => Self::Hall,
            _ => Self::Spring,
        }
    }

    /// 延迟线长度缩放（梳状与全通共用）：空间越「大」线越长、早期反射
    /// 越稀疏。
    #[must_use]
    pub const fn length_scale(self) -> f32 {
        match self {
            Self::Room => 1.0,
            Self::Plate => 0.82,
            Self::Hall => 1.30,
            Self::Spring => 0.64,
        }
    }

    /// 阻尼偏移（加在 `0.05 + damp × 0.72` 的映射结果上）：负 = 更亮。
    #[must_use]
    pub const fn damp_offset(self) -> f32 {
        match self {
            Self::Room => 0.0,
            Self::Plate => -0.12,
            Self::Hall => 0.08,
            Self::Spring => 0.04,
        }
    }

    /// 全通反馈（扩散/色散强度）：Freeverb 原值 `0.5`。
    #[must_use]
    pub const fn allpass_feedback(self) -> f32 {
        match self {
            Self::Room => 0.5,
            Self::Plate => 0.58,
            Self::Hall => 0.44,
            Self::Spring => 0.72,
        }
    }
}

/// 混响参数：除 `predelay` / `decay_s`（秒）外全部归一化到 0..1。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbParams {
    /// 衰减长度（旧归一化旋钮）：映射到梳状反馈 0.70…0.94。
    ///
    /// ⚠ 与 [`Self::decay_s`] 的分工（时间口径**只此一处汇合**，见
    /// `Reverb::apply_gain_model`）：`decay_s > 0` 时反馈由 T60 标定
    /// 决定，`size` 不参与；`decay_s == 0`（未设）时反馈走本字段的旧
    /// 公式。`size` 保留是为了既有工程与既有拼写表逐位不变，⛔ 不要
    /// 在新内容里同时写两者来「叠加」——没有叠加语义。
    pub size: f32,
    /// 尾巴里的高频阻尼。
    pub damp: f32,
    /// 湿/干平衡。
    pub mix: f32,
    /// 湿信号的立体声宽度。
    pub width: f32,
    /// 预延迟（秒，0…0.1）。
    pub predelay: f32,
    /// 空间类型（见 [`ReverbKind`]）。
    pub kind: ReverbKind,
    /// 混响时间（秒，T60）：`0` = 未设（反馈由 [`Self::size`] 决定）；
    /// `> 0` 时逐梳按 T60 标定反馈，上界 [`REVERB_MAX_DECAY_S`]。
    pub decay_s: f32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            size: 0.45,
            damp: 0.35,
            mix: 0.25,
            width: 0.8,
            predelay: 0.012,
            kind: ReverbKind::Room,
            decay_s: 0.0,
        }
    }
}

/// Freeverb 拓扑的立体声混响。
pub struct Reverb {
    combs: [[Comb; 8]; 2],
    allpass: [[Allpass; 4]; 2],
    pre: [Vec<f32>; 2],
    pre_len: usize,
    pre_index: usize,
    sample_rate: f32,
    sr_scale: f32,
    params: ReverbParams,
    /// 当前延迟线长度布局对应的类型。`params.kind` 与它不同 ⇒ 下一次
    /// `set_params` 先重排长度（见 `relayout_kind`）。与 `params.kind`
    /// 分开存的原因：未配置时没有长度可言，布局只能在配置后成立。
    layout_kind: ReverbKind,
    configured: bool,
}

impl Reverb {
    /// 构造一个**未配置**的实例（直通）。缓冲区在 [`Self::set_sample_rate`] 里分配。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            // 逐项写出，让上层可以把整个混响放进 `static`。
            combs: [
                [
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                ],
                [
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                    Comb::new(),
                ],
            ],
            allpass: [
                [
                    Allpass::new(),
                    Allpass::new(),
                    Allpass::new(),
                    Allpass::new(),
                ],
                [
                    Allpass::new(),
                    Allpass::new(),
                    Allpass::new(),
                    Allpass::new(),
                ],
            ],
            pre: [Vec::new(), Vec::new()],
            pre_len: 1,
            pre_index: 0,
            sample_rate: 48_000.0,
            sr_scale: 1.0,
            params: ReverbParams {
                size: 0.45,
                damp: 0.35,
                mix: 0.25,
                width: 0.8,
                predelay: 0.012,
                kind: ReverbKind::Room,
                decay_s: 0.0,
            },
            layout_kind: ReverbKind::Room,
            configured: false,
        }
    }

    /// 按 `sample_rate` 分配全部延迟线 [ARCH-RT-001]。
    ///
    /// **这是本类型唯一会分配的方法**，必须在音频回调之外调用一次
    /// （引擎初始化时）。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sample_rate = crate::math::sanitise_sample_rate(sample_rate);
        self.sample_rate = sample_rate;
        self.sr_scale = (sample_rate / 44_100.0).clamp(0.5, SR_SCALE_MAX);
        self.pre_len = ((self.params.predelay * sample_rate) as usize).clamp(1, PREDELAY_MAX - 1);
        if self.pre[0].len() != PREDELAY_MAX {
            self.pre = [vec![0.0; PREDELAY_MAX], vec![0.0; PREDELAY_MAX]];
        } else {
            self.pre[0].fill(0.0);
            self.pre[1].fill(0.0);
        }
        self.pre_index = 0;
        // 缓冲容量按**最大类型缩放**预留：之后类型切换只改逻辑长度
        // （`relayout_kind`），永不在 `set_params` 里分配 [ARCH-RT-001]。
        let kind_scale = self.params.kind.length_scale();
        let comb_alloc = (((COMB_TUNING[7] + STEREO_SPREAD) as f32
            * self.sr_scale
            * ReverbKind::MAX_LENGTH_SCALE) as usize
            + 8)
        .min(COMB_MAX);
        let allpass_alloc = (((ALLPASS_TUNING[0] + STEREO_SPREAD) as f32
            * self.sr_scale
            * ReverbKind::MAX_LENGTH_SCALE) as usize
            + 8)
        .min(ALLPASS_MAX);
        for (channel, combs) in self.combs.iter_mut().enumerate() {
            let spread = if channel == 1 { STEREO_SPREAD } else { 0 };
            for (index, comb) in combs.iter_mut().enumerate() {
                comb.setup(
                    COMB_TUNING[index] + spread,
                    self.sr_scale,
                    kind_scale,
                    comb_alloc,
                );
            }
        }
        for (channel, allpasses) in self.allpass.iter_mut().enumerate() {
            let spread = if channel == 1 { STEREO_SPREAD } else { 0 };
            for (index, allpass) in allpasses.iter_mut().enumerate() {
                allpass.setup(
                    ALLPASS_TUNING[index] + spread,
                    self.sr_scale,
                    kind_scale,
                    allpass_alloc,
                );
            }
        }
        self.layout_kind = self.params.kind;
        self.configured = true;
        // `decay_s` 的标定反馈依赖**真实线长**：参数若在配置之前就已
        // 设好（`set_params` 时线长还是占位值），在这里按真实长度重算
        // 一次。旧路径（`decay_s == 0`）的反馈只依赖参数本身，配置前后
        // 同值，⛔ 不在这里动（保持既有调用顺序的逐位行为）。
        if self.params.decay_s > 0.0 {
            self.apply_gain_model();
        }
    }

    /// 是否已经过 [`Self::set_sample_rate`]。
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.configured
    }

    /// 设置参数（分配零次；可以逐块调用）。
    ///
    /// 退化输入（`NaN`/`±inf`）会被替换成默认值：它们会把整个梳状组的反馈变成
    /// `NaN` 并静默毒化整条混响总线，而实时路径上无法报错，只能就地回落。
    ///
    /// **预延迟的帧数变了**就把两条预延迟线清零、写头归零（见模块文档 §6）。
    /// **类型变了**就重排梳状/全通的逻辑长度并清它们的历史（见模块文档 §7）。
    /// 两者都没变时一个字节都不动：梳状组、全通组与两条预延迟线的历史全部保留，
    /// 因此"同一组参数重复设置"是严格空操作。清线只对已分配的缓冲做原地写 ⇒
    /// **零分配**，可以在实时线程上逐块调用 [ARCH-RT-001]。
    pub fn set_params(&mut self, params: ReverbParams) {
        /// 非有限值一律回落到这个默认参数集。
        const FALLBACK: ReverbParams = ReverbParams {
            size: 0.45,
            damp: 0.35,
            mix: 0.25,
            width: 0.8,
            predelay: 0.012,
            kind: ReverbKind::Room,
            decay_s: 0.0,
        };
        let sanitise = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        let params = ReverbParams {
            size: sanitise(params.size, FALLBACK.size).clamp(0.0, 1.0),
            damp: sanitise(params.damp, FALLBACK.damp).clamp(0.0, 1.0),
            mix: sanitise(params.mix, FALLBACK.mix).clamp(0.0, 1.0),
            width: sanitise(params.width, FALLBACK.width).clamp(0.0, 1.0),
            predelay: sanitise(params.predelay, 0.0).clamp(0.0, REVERB_MAX_PREDELAY_S),
            kind: params.kind,
            decay_s: sanitise(params.decay_s, FALLBACK.decay_s).clamp(0.0, REVERB_MAX_DECAY_S),
        };
        self.params = params;
        // 类型变了且已配置 ⇒ 先重排长度（清历史、零分配），再套增益
        // 模型（标定反馈读的是重排后的真实线长）。类型没变时重排是
        // 严格空操作：长度与历史一个字节都不动。
        if self.configured && params.kind != self.layout_kind {
            self.relayout_kind(params.kind);
        }
        self.apply_gain_model();
        if self.configured {
            let wanted = ((params.predelay * self.sample_rate) as usize).clamp(1, PREDELAY_MAX - 1);
            if wanted != self.pre_len {
                // 只有**帧数真的变了**才清线（见模块文档 §6）：线按 `PREDELAY_MAX`
                // 预分配、永不缩小，所以长度变长时新写头会指向一段在旧的、更短的
                // 延迟下写进缓冲、却从未被读出的旧音频 ⇒ 幽灵回声。
                // 长度不变时一个字节都不动（梳状组与全通组的历史也不动）。
                for line in &mut self.pre {
                    line.fill(0.0);
                }
                self.pre_index = 0;
                self.pre_len = wanted;
            }
        }
    }

    /// 把当前参数的**增益模型**写进梳状组与全通组：反馈、阻尼、全通
    /// 反馈。`set_params` 与 `set_sample_rate`（仅 `decay_s` 已设时）
    /// 共用这一处 ⇒ 时间口径只此一处汇合。
    ///
    /// - **反馈**：`decay_s > 0` 时逐梳按 T60 标定 —— 每条梳的往返时间
    ///   是 `len / sr` 秒，要让它在 `decay_s` 秒内衰减 60 dB，反馈须为
    ///   `exp(−ln(1000) · len / (sr · decay_s))`，逐梳各按自己的线长算
    ///   ⇒ 八条梳的标称 T60 相同（阻尼的一极点只吃高频，低频衰减率即
    ///   本式）。`decay_s == 0`（未设）时走旧公式 `0.70 + size × 0.24`
    ///   全梳同值（0.70…0.94 是稳定区间的映射，0.96 是带调制混响的
    ///   稳定极限）⇒ 既有工程逐位不变。
    /// - **阻尼**：`0.05 + damp × 0.72` 再加类型的阻尼偏移（Room 偏移
    ///   为 0 ⇒ 旧值逐位不变）；下限让环路在 DAMP = 0 时仍有损耗。
    /// - **全通反馈**：类型的扩散/色散强度（Room = Freeverb 原值 0.5）。
    fn apply_gain_model(&mut self) {
        let kind = self.params.kind;
        let damp = (0.05 + self.params.damp * 0.72 + kind.damp_offset()).clamp(0.02, 0.95);
        let decay_s = self.params.decay_s;
        for combs in self.combs.iter_mut() {
            for comb in combs.iter_mut() {
                comb.damp = damp;
                comb.feedback = if decay_s > 0.0 {
                    let round_trip_s = comb.len as f32 / self.sample_rate;
                    (-T60_LN * T60_CALIBRATION * round_trip_s / decay_s)
                        .exp()
                        .clamp(0.05, 0.995)
                } else {
                    0.70 + self.params.size * 0.24
                };
            }
        }
        for allpasses in self.allpass.iter_mut() {
            for allpass in allpasses.iter_mut() {
                allpass.feedback = kind.allpass_feedback();
            }
        }
    }

    /// 按新类型**重排**全部延迟线的逻辑长度并清历史（零分配：容量已
    /// 在 `set_sample_rate` 时按最大缩放预留）。清历史的理由与预延迟
    /// 改长清线相同（模块文档 §6）：长度变长后，未清的旧样本会落在
    /// 从未被读出的区段里，变成与参数历史有关的幽灵回声。
    fn relayout_kind(&mut self, kind: ReverbKind) {
        let scale = kind.length_scale();
        for (channel, combs) in self.combs.iter_mut().enumerate() {
            let spread = if channel == 1 { STEREO_SPREAD } else { 0 };
            for (index, comb) in combs.iter_mut().enumerate() {
                comb.relayout(COMB_TUNING[index] + spread, self.sr_scale, scale);
            }
        }
        for (channel, allpasses) in self.allpass.iter_mut().enumerate() {
            let spread = if channel == 1 { STEREO_SPREAD } else { 0 };
            for (index, allpass) in allpasses.iter_mut().enumerate() {
                allpass.relayout(ALLPASS_TUNING[index] + spread, self.sr_scale, scale);
            }
        }
        self.layout_kind = kind;
    }

    /// 当前参数。
    #[must_use]
    pub const fn params(&self) -> ReverbParams {
        self.params
    }

    /// 湿信号是否可闻（`mix > 1e-4`）。
    ///
    /// ⚠ 未配置时本方法**仍**按 `mix` 回答（既有判据
    /// `an_unconfigured_reverb_is_a_passthrough` 钉住这条）；它说的是"这个参数
    /// 设置会不会出声"，不是"这台实例现在会不会处理"。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.params.mix > 1e-4
    }

    /// 本器件引入的**处理延迟**（帧），恒为 [`REVERB_LATENCY`] = `0`
    /// [ARCH-PDC-001]。
    ///
    /// 预延迟**不在**这个数里：`process` 里干路用的是**当前**样本 `l` / `r`，
    /// 预延迟线只喂梳状组 ⇒ 整机对输入没有净延迟。判据
    /// `reverb::tests::latency_is_zero_and_the_predelay_delays_only_the_wet_path`。
    ///
    /// 口径与 [`crate::convolution_reverb::ConvolutionReverb::latency_samples`] 相同
    /// （那里也把预延迟排除在 PDC 之外）。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        REVERB_LATENCY
    }

    /// 清空全部延迟线的**历史**：16 条梳状、8 条全通、两条预延迟。
    ///
    /// 契约（由判据
    /// `reverb::tests::reset_reproduces_a_freshly_built_device_bit_for_bit` 钉住）：
    /// `reset()` 之后处理任意块，与一台**刚用同样参数装配好**的实例
    /// （`new` → `set_sample_rate` → `set_params`）逐位相同。
    ///
    /// **配置不动**：采样率、参数、各条线的长度与系数都保持不变 —— 清的只是历史。
    /// 与 [`Self::set_params`] 的取舍不同：那里只在预延迟的**帧数真的变了**时清
    /// 预延迟线（长度不变则一个字节都不动），而 `reset()` 是调用方显式要求
    /// "从现在起当它没响过" ⇒ 历史全清。
    ///
    /// **逐样本零分配**（只对已分配的缓冲做原地写，不经堆），可以在实时线程上调用
    /// [ARCH-RT-001]。未配置时是空操作（空 `Vec` 的 `fill` 不分配也不 panic）。
    pub fn reset(&mut self) {
        for combs in self.combs.iter_mut() {
            for comb in combs.iter_mut() {
                comb.reset();
            }
        }
        for allpasses in self.allpass.iter_mut() {
            for allpass in allpasses.iter_mut() {
                allpass.reset();
            }
        }
        for line in &mut self.pre {
            line.fill(0.0);
        }
        self.pre_index = 0;
    }

    /// 立体声块的原地湿/干混合。
    ///
    /// 未配置时直通；长度取 `left`/`right` 的较短者；全程零分配。
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mix = if self.params.mix.is_finite() {
            self.params.mix.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if !self.configured || self.pre[0].is_empty() || mix <= 1e-4 {
            return;
        }
        let dry = 1.0 - mix;
        let width = if self.params.width.is_finite() {
            self.params.width.clamp(0.0, 1.0)
        } else {
            0.0
        };
        for (left_slot, right_slot) in left.iter_mut().zip(right.iter_mut()) {
            // 入口守卫：一个非有限样本写进预延迟线或梳状组就会**永久**毒化反馈环
            // （`NaN·0.84 = NaN`、`∞·0.84 = ∞`），`reset()` 之外没有出路，
            // 而实时路径上无法报错 ⇒ 就地回落成 `0.0`。有限样本逐位不变，
            // 既有音色与既有读数一个比特都不改。见模块文档 §5。
            let l = finite_or_zero(*left_slot);
            let r = finite_or_zero(*right_slot);

            // 预延迟（普通环形缓冲，两声道等长）。
            let dl = self.pre[0][self.pre_index];
            let dr = self.pre[1][self.pre_index];
            self.pre[0][self.pre_index] = l;
            self.pre[1][self.pre_index] = r;
            self.pre_index += 1;
            if self.pre_index >= self.pre_len {
                self.pre_index = 0;
            }

            // Freeverb 把两个声道求和后送进每个"箱体"（近似单声道输入），
            // 这正是该算法尾巴稳定、密集的来源。
            let input = (dl + dr) * 0.5 * 0.015;
            let mut wet_l = 0.0;
            let mut wet_r = 0.0;
            for comb in self.combs[0].iter_mut() {
                wet_l += comb.process(input);
            }
            for comb in self.combs[1].iter_mut() {
                wet_r += comb.process(input);
            }
            for allpass in self.allpass[0].iter_mut() {
                wet_l = allpass.process(wet_l);
            }
            for allpass in self.allpass[1].iter_mut() {
                wet_r = allpass.process(wet_r);
            }

            // 宽度：0 塌成单声道，1 保持两个箱体完全分离。
            let mid = (wet_l + wet_r) * 0.5 * WET_GAIN;
            let side = (wet_l - wet_r) * 0.5 * width * WET_GAIN;
            let out_l = mid + side;
            let out_r = mid - side;

            *left_slot = l * dry + out_l * mix;
            *right_slot = r * dry + out_r * mix;
        }
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn impulse_response(params: ReverbParams, seconds: f32) -> (Vec<f32>, Vec<f32>, f32) {
        let mut verb = Reverb::new();
        verb.set_sample_rate(SR);
        verb.set_params(params);
        let n = (seconds * SR) as usize;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        l[0] = 1.0;
        r[0] = 1.0;
        verb.process(&mut l, &mut r);
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
        (l, r, peak)
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// 一段**确定性**激励，原地写进一对声道缓冲（同一个 `quantum` 给同一段样本）。
    ///
    /// 没有 RNG 对象、没有分配：本票的两条新判据要给多台实例喂**同一段**输入。
    fn excite(left: &mut [f32], right: &mut [f32], quantum: u64, scale: f32) {
        for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let t = i as f32 * 0.011;
            *l = scale * (quantum as f32 * 0.37 + t).sin();
            *r = scale * (quantum as f32 * 0.29 - t).cos();
        }
    }

    #[test]
    fn impulse_tail_decays_and_stays_bounded() {
        let (l, _, peak) = impulse_response(
            ReverbParams {
                mix: 1.0,
                ..Default::default()
            },
            4.0,
        );
        assert!(peak <= 1.0, "reverb overshot: {peak}");
        let head = rms(&l[4800..9600]); // 0.1–0.2 s
        let tail = rms(&l[l.len() - 4800..]); // 3.9–4.0 s
        assert!(head > 0.0, "no tail at all");
        assert!(tail < head * 0.5, "tail did not decay: {head} -> {tail}");
        assert!(
            !l.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(l.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn damping_darkens_the_tail() {
        let bright = impulse_response(
            ReverbParams {
                mix: 1.0,
                damp: 0.0,
                size: 0.8,
                ..Default::default()
            },
            2.0,
        )
        .0;
        let dark = impulse_response(
            ReverbParams {
                mix: 1.0,
                damp: 1.0,
                size: 0.8,
                ..Default::default()
            },
            2.0,
        )
        .0;
        // 平均绝对斜率是廉价的亮度代理：更暗的尾巴相邻样本间移动更少。
        let slope = |buf: &[f32]| {
            buf.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / buf.len() as f32
        };
        assert!(
            slope(&dark[24_000..]) < slope(&bright[24_000..]),
            "damping did not darken the tail"
        );
    }

    #[test]
    fn width_controls_the_stereo_spread() {
        let (l, r, _) = impulse_response(
            ReverbParams {
                mix: 1.0,
                width: 1.0,
                ..Default::default()
            },
            1.0,
        );
        let diff: f32 = l.iter().zip(&r).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 0.01, "stereo tanks are identical");
        let (l0, r0, _) = impulse_response(
            ReverbParams {
                mix: 1.0,
                width: 0.0,
                ..Default::default()
            },
            1.0,
        );
        let diff0: f32 = l0.iter().zip(&r0).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff0 < diff * 0.1, "width 0 should collapse to mono");
    }

    #[test]
    fn pre_delay_holds_the_tail_back() {
        let (early, _, _) = impulse_response(
            ReverbParams {
                mix: 1.0,
                predelay: 0.0,
                size: 0.5,
                ..Default::default()
            },
            1.0,
        );
        let (late, _, _) = impulse_response(
            ReverbParams {
                mix: 1.0,
                predelay: 0.05,
                size: 0.5,
                ..Default::default()
            },
            1.0,
        );
        let first_ms = |buf: &[f32]| {
            buf.iter()
                .position(|s| s.abs() > 1e-4)
                .map(|i| i as f32 / 48.0)
                .unwrap_or(f32::MAX)
        };
        assert!(first_ms(&late) > first_ms(&early) + 20.0);
    }

    #[test]
    fn sustained_tones_do_not_blow_up() {
        // 任何频率的稳定正弦都不得把梳状组泵起来：带调制的反馈延迟是这事
        // 经典出错的地方。
        for freq in [55.0f32, 110.0, 220.0, 440.0, 880.0, 1760.0, 3520.0] {
            let mut verb = Reverb::new();
            verb.set_sample_rate(SR);
            verb.set_params(ReverbParams {
                mix: 1.0,
                size: 1.0,
                damp: 0.0,
                width: 1.0,
                predelay: 0.012,
                kind: ReverbKind::Room,
                decay_s: 0.0,
            });
            let n = 48_000 * 4;
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            for i in 0..n {
                let v = (core::f32::consts::TAU * freq * i as f32 / SR).sin() * 0.5;
                l[i] = v;
                r[i] = v;
            }
            verb.process(&mut l, &mut r);
            let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
            let sec = 48_000;
            let head = rms(&l[sec..sec * 2]);
            let tail = rms(&l[sec * 3..]);
            assert!(peak.is_finite(), "{freq} Hz produced non-finite output");
            assert!(peak < 8.0, "{freq} Hz spiked to {peak}");
            // 稳定音会进入稳态，因此尾巴可能与头部同电平；但它绝不能持续增长。
            assert!(
                tail < head * 1.25,
                "{freq} Hz does not settle: {head} -> {tail}"
            );
        }
    }

    #[test]
    fn long_input_never_blows_up() {
        let mut verb = Reverb::new();
        verb.set_sample_rate(SR);
        verb.set_params(ReverbParams {
            mix: 1.0,
            size: 1.0,
            damp: 0.0,
            ..Default::default()
        });
        let mut l = vec![0.3f32; 48_000];
        let mut r = vec![-0.3f32; 48_000];
        verb.process(&mut l, &mut r);
        assert!(
            !l.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(l.iter().all(|s| s.is_finite() && s.abs() < 4.0));
        assert!(
            !r.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(r.iter().all(|s| s.is_finite() && s.abs() < 4.0));
    }

    /// **判据（新写，可红）**：预延迟以**秒**计，与采样率无关。
    ///
    /// 观测方式：第一个非零输出出现在"预延迟 + 最短梳延迟"处，而梳延迟本身也被
    /// 采样率缩放，因此绝对值不能直接断言。可断言的是**预延迟带来的那段增量**：
    /// 50 ms 的预延迟必须比 0 ms 的预延迟晚 50 ms 才开始出声，且在 48 kHz 与
    /// 96 kHz 下都成立。
    ///
    /// 把 [`Reverb::set_sample_rate`] 里 `predelay * sample_rate` 换成
    /// `predelay * 48_000.0`，96 kHz 那一半立即变红。
    #[test]
    fn predelay_is_measured_in_seconds_at_any_sample_rate() {
        let first_ms = |sample_rate: f32, predelay: f32| {
            let mut verb = Reverb::new();
            verb.set_sample_rate(sample_rate);
            verb.set_params(ReverbParams {
                mix: 1.0,
                predelay,
                size: 0.5,
                ..Default::default()
            });
            let seconds = 0.5;
            let n = (seconds * sample_rate) as usize;
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            l[0] = 1.0;
            r[0] = 1.0;
            verb.process(&mut l, &mut r);
            let first = l
                .iter()
                .position(|s| s.abs() > 1e-4)
                .expect("reverb produced no output at all");
            first as f32 / sample_rate * 1000.0
        };
        for sample_rate in [48_000.0f32, 96_000.0] {
            let none = first_ms(sample_rate, 0.0);
            let fifty = first_ms(sample_rate, 0.05);
            let added = fifty - none;
            assert!(
                (added - 50.0).abs() < 2.0,
                "at {sample_rate} Hz a 50 ms predelay added {added} ms (0 ms -> {none} ms)"
            );
            // 预延迟 0 时立刻有输出，证明上面测的确实是预延迟而不是别的延迟。
            assert!(
                none < 40.0,
                "the comb bank is late even at predelay 0: {none} ms"
            );
        }
    }

    /// **判据（新写，可红）**：未配置的混响是直通，且不得 panic。
    ///
    /// 来源在从未调用 `set_sample_rate` 时会索引一个空 `Vec` 而 panic。
    /// 若把 `process` 里的 `!self.configured || self.pre[0].is_empty()` 守卫删掉，
    /// 本测试立刻 panic 变红。
    #[test]
    fn an_unconfigured_reverb_is_a_passthrough() {
        let mut verb = Reverb::new();
        assert!(!verb.is_configured());
        assert!(verb.is_active(), "default mix should be audible");
        let mut left = [0.5f32, -0.25, 0.125];
        let mut right = [0.1f32, 0.2, -0.3];
        verb.process(&mut left, &mut right);
        assert_eq!(left, [0.5, -0.25, 0.125]);
        assert_eq!(right, [0.1, 0.2, -0.3]);
        // mix = 0 时即使是已配置实例也必须是逐位直通。
        verb.set_sample_rate(SR);
        verb.set_params(ReverbParams {
            mix: 0.0,
            ..Default::default()
        });
        assert!(!verb.is_active());
        let mut left = [0.5f32, -0.25];
        let mut right = [0.1f32, 0.2];
        verb.process(&mut left, &mut right);
        assert_eq!(left, [0.5, -0.25]);
        assert_eq!(right, [0.1, 0.2]);
        // 长度不等的块不得 panic（只处理较短的部分）。
        verb.set_params(ReverbParams {
            mix: 1.0,
            ..Default::default()
        });
        let mut long = [0.0f32; 8];
        let mut short = [0.0f32; 3];
        verb.process(&mut long, &mut short);
        assert!(
            !long.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(long.iter().all(|v| v.is_finite()));
        assert!(
            !short.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(short.iter().all(|v| v.is_finite()));
        // 退化参数不得产生 NaN：非有限值回落到默认值，混响仍然必须活下来。
        verb.set_params(ReverbParams {
            size: f32::NAN,
            damp: f32::INFINITY,
            mix: 0.5,
            width: f32::NAN,
            predelay: f32::NAN,
            kind: ReverbKind::Room,
            decay_s: 0.0,
        });
        assert!(verb.is_active());
        let mut left = vec![0.2f32; 4096];
        let mut right = vec![0.2f32; 4096];
        verb.process(&mut left, &mut right);
        assert!(
            !left.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(
            left.iter().all(|v| v.is_finite()),
            "degenerate parameters poisoned the reverb bus"
        );
        assert!(
            !right.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(right.iter().all(|v| v.is_finite()));
        assert!(
            (verb.params().mix - 0.5).abs() < 1e-6,
            "the finite knob was dropped"
        );
    }

    /// **判据（新写，可红）**：`reset()` 之后逐位等于**刚用同样参数装配好**的实例。
    ///
    /// 量什么：三台同源实例（同样的采样率与参数）在**同一个探测块**上的输出比特。
    /// `used` 先被 8 个块激励（留下尾巴）然后 `reset()`；`fresh` 从未处理过任何输入；
    /// `stale` 与 `used` 经历**完全相同**的输入但**不** `reset()`（反向对照）。
    ///
    /// 判据：① 左/右声道 `used` 的每一个比特等于 `fresh`；② `stale` 与 `fresh`
    /// 至少 `512` 个样本不同 —— 第二条证明这个探测块**测得出**尾巴，因此第一条
    /// 不是"尾巴本来就没有"的假绿。
    ///
    /// 注入：删掉 [`Reverb::reset`] 里梳状与全通的 `reset()` 循环（只留预延迟清零）
    /// ⇒ 断言 ① 立即变红（输出里还留着 8 个块的尾巴）；删掉预延迟清零那一句
    /// 同样变红（预延迟线的残余把 `stale` 的历史带进湿路）。
    #[test]
    fn reset_reproduces_a_freshly_built_device_bit_for_bit() {
        /// 每个块的帧数。`0.02 s` 的预延迟在 48 kHz 下是 `960` 帧，`8 × 4096`
        /// 帧足够让两条预延迟线与最长的梳状线都绕过好几圈。
        const FRAMES: usize = 4_096;
        /// 反向对照的**最小**差异样本数（左声道）。实测远大于它。
        const MIN_STALE_DIFFERENCES: usize = 512;

        let params = ReverbParams {
            size: 0.85,
            damp: 0.2,
            mix: 0.6,
            width: 0.7,
            predelay: 0.02,
            kind: ReverbKind::Room,
            decay_s: 0.0,
        };
        let build = || {
            let mut verb = Reverb::new();
            verb.set_sample_rate(SR);
            verb.set_params(params);
            verb
        };

        let mut used = build();
        let mut stale = build();
        let mut fresh = build();

        // 激励：`used` 与 `stale` 吃**同样**的 8 个块。
        let mut left = vec![0.0f32; FRAMES];
        let mut right = vec![0.0f32; FRAMES];
        for quantum in 0..8 {
            excite(&mut left, &mut right, quantum, 0.5);
            used.process(&mut left, &mut right);
            excite(&mut left, &mut right, quantum, 0.5);
            stale.process(&mut left, &mut right);
        }
        // 覆盖度自检：激励之后尾巴必须还在（否则反向对照没有意义）。
        assert!(
            left.iter().any(|v| v.abs() > 1e-3),
            "激励块没有产生任何输出 ⇒ 本判据测的是空壳"
        );

        used.reset();

        // 探测块：三台处理同一段输入。
        let mut probe_left = vec![0.0f32; FRAMES];
        let mut probe_right = vec![0.0f32; FRAMES];
        excite(&mut probe_left, &mut probe_right, 99, 0.4);

        let mut used_left = probe_left.clone();
        let mut used_right = probe_right.clone();
        used.process(&mut used_left, &mut used_right);

        let mut fresh_left = probe_left.clone();
        let mut fresh_right = probe_right.clone();
        fresh.process(&mut fresh_left, &mut fresh_right);

        let mut stale_left = probe_left.clone();
        let mut stale_right = probe_right.clone();
        stale.process(&mut stale_left, &mut stale_right);

        // ① 正向：逐位相同。
        for (i, (out, want)) in used_left.iter().zip(&fresh_left).enumerate() {
            assert_eq!(
                out.to_bits(),
                want.to_bits(),
                "样本 {i}: reset 之后左声道必须逐位等于新建实例"
            );
        }
        for (i, (out, want)) in used_right.iter().zip(&fresh_right).enumerate() {
            assert_eq!(
                out.to_bits(),
                want.to_bits(),
                "样本 {i}: reset 之后右声道必须逐位等于新建实例"
            );
        }

        // ② 反向对照：不 reset 的实例必须与新建实例**明显**不同。
        let mut differences = 0usize;
        for (out, want) in stale_left.iter().zip(&fresh_left) {
            if out.to_bits() != want.to_bits() {
                differences += 1;
            }
        }
        assert!(
            differences >= MIN_STALE_DIFFERENCES,
            "不 reset 的实例与新建实例只有 {differences}/{FRAMES} 个样本不同 ⇒ \
             这个探测块测不出尾巴，判据 ① 的绿没有判别力"
        );
        // 反向对照在右声道上同样成立（防止判据只在一个声道上有牙）。
        let right_differences = stale_right
            .iter()
            .zip(&fresh_right)
            .filter(|(out, want)| out.to_bits() != want.to_bits())
            .count();
        assert!(
            right_differences >= MIN_STALE_DIFFERENCES,
            "右声道只有 {right_differences}/{FRAMES} 个样本不同 ⇒ 判据只在一个声道上有牙"
        );
        eprintln!(
            "[yeban-dsp] reverb::reset 读数（单位：个样本）: FRAMES={FRAMES} 采样率={SR} Hz \
             预延迟={} 帧 reset 后与新建实例的比特差异=0；不 reset 的对照差异 左={differences} \
             右={right_differences}（门限 {MIN_STALE_DIFFERENCES}）",
            (params.predelay * SR) as usize
        );
    }

    /// **判据（可红）**：同一组参数**重复**施加是严格空操作 [ARCH-DET-001]。
    ///
    /// [`Reverb::set_params`] 的文档声明："预延迟的帧数**没变**时一个字节都不动：
    /// 梳状组、全通组与两条预延迟线的历史全部保留，因此『同一组参数重复设置』是
    /// 严格空操作。" 本判据把这条声明变成可红断言 —— 逐位一致类判据不靠改期望值变绿。
    ///
    /// 量什么（单位：个样本）：三台同参数实例吃**同样**的激励块，随后
    ///   * `baseline` 不再动参数；
    ///   * `reapplied` 在块 A 与块 B 之间把**同一** `ReverbParams` 再设置一次；
    ///   * `cleared` 也再设置一次，但它吃的是一根**很短的**脉冲（`[1, 0, 0, …]`，湿
    ///     路只被激励一帧）⇒ 块 A 之后它的两条预延迟线与梳状组里只剩一点衰减尾，
    ///     与 `baseline` 灌满的历史明显不同。
    ///
    /// 断言两条：① `reapplied` 的块 B 与 `baseline` **逐位相同**（重复设参是空操作）；
    /// ② `cleared` 与 `baseline` **至少 `1` 个样本不同** —— 第二条是本判据的**牙**：
    /// 它证明"预延迟线里确实有上一块留下的历史"，因此第一条不是"本来就没历史"
    /// 的假绿。预延迟取 `0.02 s`（`960` 帧 @48 kHz），块长 `4 096` 帧 ⇒ 探测块里
    /// 的湿路读的是**上一块**写进预延迟线的样本。
    ///
    /// 注入（本机实测）：把 [`Reverb::set_params`] 里 `if wanted != self.pre_len`
    /// 的条件去掉（即每次设参都清两条预延迟线）⇒ 断言 ① 立即变红；把该分支整体
    /// 删掉（帧数变了也不清）⇒ 既有的
    /// [`tests::changing_the_pre_delay_length_never_replays_stale_audio`] 变红，
    /// 本判据仍绿。
    #[test]
    fn re_applying_the_same_params_is_a_no_op() {
        /// 每个块的帧数。`0.02 s` 预延迟 = `960` 帧，`4 096` 帧让两条预延迟线
        /// 与最长的梳状线都绕过好几圈。
        const FRAMES: usize = 4_096;

        let params = ReverbParams {
            size: 0.8,
            damp: 0.3,
            mix: 0.6,
            width: 0.6,
            predelay: 0.02,
            kind: ReverbKind::Room,
            decay_s: 0.0,
        };
        let build = || {
            let mut verb = Reverb::new();
            verb.set_sample_rate(SR);
            verb.set_params(params);
            verb
        };

        // 块 A：`baseline` 与 `reapplied` 吃同样的激励，把预延迟线与梳状组灌满；
        // `cleared` 吃一根脉冲，湿路全零 ⇒ 它的历史是零。
        let mut baseline = build();
        let mut reapplied = build();
        let mut cleared = build();
        let mut left = vec![0.0f32; FRAMES];
        let mut right = vec![0.0f32; FRAMES];
        excite(&mut left, &mut right, 7, 0.5);
        baseline.process(&mut left, &mut right);
        excite(&mut left, &mut right, 7, 0.5);
        reapplied.process(&mut left, &mut right);
        let mut impulse_l = vec![0.0f32; FRAMES];
        let mut impulse_r = vec![0.0f32; FRAMES];
        impulse_l[0] = 1.0;
        impulse_r[0] = 1.0;
        cleared.process(&mut impulse_l, &mut impulse_r);
        assert!(
            left.iter().any(|v| v.abs() > 1e-3),
            "块 A 没有产生任何输出 ⇒ 本判据测的是空壳"
        );

        // 重复施加**同一**参数。
        reapplied.set_params(params);
        cleared.set_params(params);

        // 块 B：三台处理同一段输入。
        let mut probe_left = vec![0.0f32; FRAMES];
        let mut probe_right = vec![0.0f32; FRAMES];
        excite(&mut probe_left, &mut probe_right, 8, 0.4);

        let mut base_l = probe_left.clone();
        let mut base_r = probe_right.clone();
        baseline.process(&mut base_l, &mut base_r);

        let mut re_l = probe_left.clone();
        let mut re_r = probe_right.clone();
        reapplied.process(&mut re_l, &mut re_r);

        let mut clr_l = probe_left.clone();
        let mut clr_r = probe_right.clone();
        cleared.process(&mut clr_l, &mut clr_r);

        // ① 重复施加同样的参数 ⇒ 与只施加一次的实例逐位相同。
        for (i, (out, want)) in re_l.iter().zip(&base_l).enumerate() {
            assert_eq!(
                out.to_bits(),
                want.to_bits(),
                "样本 {i}: 重复施加同一组参数改变了左声道"
            );
        }
        for (i, (out, want)) in re_r.iter().zip(&base_r).enumerate() {
            assert_eq!(
                out.to_bits(),
                want.to_bits(),
                "样本 {i}: 重复施加同一组参数改变了右声道"
            );
        }

        // ② 反向对照：历史被清掉的那一台必须与保留历史的那一台不同。
        let cleared_differences = clr_l
            .iter()
            .zip(&base_l)
            .filter(|(out, want)| out.to_bits() != want.to_bits())
            .count();
        assert!(
            cleared_differences > 0,
            "清掉历史的实例与保留历史的实例逐位相同 ⇒ 本判据的断言 ① 没有判别力"
        );
        eprintln!(
            "[yeban-dsp] reverb::set_params 重复施加读数（单位：个样本）: \
             与只施加一次逐位差异=0；清历史对照差异={cleared_differences}/{FRAMES}"
        );
    }

    /// **判据（新写，可红）**：延迟上报恒为 `0`，且预延迟**只**推迟湿路。
    ///
    /// 量什么：① [`Reverb::latency_samples`] 的读数（单位：帧）；② 预延迟
    /// `0.0 s` 与 `0.05 s` 下，单位脉冲输出的**第 0 帧**（比特）与湿路首次超过
    /// `1e-4` 的样本下标（单位：帧）。
    ///
    /// 判据：① 读数恒为 `REVERB_LATENCY = 0`；② 两种预延迟下第 0 帧都**恰好**是
    /// `1.0`（`1.0 · dry + 0.0 · mix`，干路用当前样本）；③ `50 ms` 的预延迟把湿路
    /// 的首次可闻输出推后**超过** `2 000` 帧（`50 ms @48 kHz` = `2 400` 帧）
    /// —— ② 与 ③ 一起才说明"干路即时、湿路被推迟"。
    ///
    /// 注入：让预延迟推迟**整个**输入（干路也进延迟线）⇒ 断言 ② 变红；
    /// 把 `set_sample_rate` 里的 `predelay * sample_rate` 写死成 `0.0` ⇒ 断言 ③ 变红。
    #[test]
    fn latency_is_zero_and_the_predelay_delays_only_the_wet_path() {
        /// 观测长度（帧）：`50 ms` 预延迟 ＋ 梳状组的最长首达都装得下。
        const FRAMES: usize = 9_600;
        /// 湿/干平衡。取 `0.5` 而不是接近 `0` 的值：湿路的**首个**到达只有
        /// `脉冲 · 0.015 · WET_GAIN · mix` 量级，`mix` 太小时它落在 `1e-4`
        /// 门限以下，判据测到的就不是"首个到达"而是"积累到可闻"。
        const MIX: f32 = 0.5;

        let probe = |predelay: f32| {
            let mut verb = Reverb::new();
            verb.set_sample_rate(SR);
            verb.set_params(ReverbParams {
                size: 0.5,
                damp: 0.35,
                // 大于 `process` 的 `1e-4` 静音守卫 ⇒ 湿路真的参与运算。
                mix: MIX,
                width: 0.8,
                predelay,
                kind: ReverbKind::Room,
                decay_s: 0.0,
            });
            assert_eq!(verb.latency_samples(), REVERB_LATENCY);
            assert_eq!(verb.latency_samples(), 0, "预延迟不是 PDC 延迟");
            let mut left = vec![0.0f32; FRAMES];
            let mut right = vec![0.0f32; FRAMES];
            left[0] = 1.0;
            right[0] = 1.0;
            verb.process(&mut left, &mut right);
            // 干路即时：第 0 帧 = `1.0 · dry + 0.0 · mix`，湿项在那时还是 0。
            assert_eq!(
                left[0].to_bits(),
                (1.0f32 - MIX).to_bits(),
                "预延迟 {predelay} s: 干路被推迟了（第 0 帧不是干项）"
            );
            assert_eq!(right[0].to_bits(), (1.0f32 - MIX).to_bits());
            // 湿路首次可闻输出（跳过第 0 帧的干项；之后输入恒为 0，剩下的只有湿路）。
            let first = left[1..]
                .iter()
                .position(|s| s.abs() > 1e-4)
                .map(|i| i + 1)
                .expect("湿路一直没有出声");
            (left[0], first)
        };

        let (dry_none, wet_none) = probe(0.0);
        let (dry_fifty, wet_fifty) = probe(0.05);
        assert_eq!(dry_none.to_bits(), dry_fifty.to_bits());
        assert!(
            wet_fifty > wet_none + 2_000,
            "50 ms 的预延迟只把湿路推后了 {} 帧（{wet_none} → {wet_fifty}）",
            wet_fifty - wet_none
        );
        assert!(wet_none < 1_920, "预延迟 0 时湿路来晚了：{wet_none} 帧");
        eprintln!(
            "[yeban-dsp] reverb::latency 读数（单位：帧）: latency_samples()={} \
             第 0 帧干项=0x{:08x}（两种预延迟相同）；湿路首次可闻 预延迟 0 s → {wet_none} 帧、\
             0.05 s → {wet_fifty} 帧，增量 {} 帧",
            REVERB_LATENCY,
            dry_none.to_bits(),
            wet_fifty - wet_none
        );
    }

    /// **判据（新写，可红）**：预延迟的**帧数变更**不得重放按旧长度写进缓冲的音频。
    ///
    /// 量什么：夹在"预延迟 `1/1024 s`（=`46` 帧）→ `1/32 s`（=`1500` 帧）"这一次长度
    /// 变更两侧的输出。读数有三个，单位都是线性幅度（第三个是帧数）：
    ///
    /// ① 灌线段的输出**峰值** —— 该段喂进 `46` 帧常数 `7.0`，而 `process` 是**读在写
    ///    之前**，所以这 `46` 帧读到的全是 `0.0`（槽位还没被写过）：峰值必须是**恰好**
    ///    `0.0`。这是夹具的前置条件 —— 它证明梳状组与全通组**一次也没被激励**，
    ///    因此此后任何非零输出都只可能来自预延迟线里残留的旧音频；
    /// ② 长度变更后喂 `PROBE` 帧静音的输出**峰值**，以及它与"**刚**按 `1500` 帧装配好
    ///    的实例"的**逐比特**比较；
    /// ③ 正对照：同一条灌满的线，长度**不变**（仍是 `46` 帧）时喂同样长的静音，
    ///    峰值必须 **`> 0`** —— 它证明这套夹具真的听得到线里那份旧音频（否则 ② 是空断言）。
    ///
    /// 判据：① `== 0.0`；② `== 0.0` 且与新建实例的时间序列**逐比特相同**；③ `> 0.0`。
    ///
    /// **本机实测（aarch64，本票）**：把 `set_params` 里那次 `line.fill(0.0)` 删掉 ⇒
    /// ② 的峰值从 `0.0` 变成 `2.864376e-1`（首个非零样本在第 `1214` 帧 = `1116 · 48000/44100`，
    /// 即最短梳状延迟），与新建实例相差 `2938/6000` 个比特 ⇒ 该断言实测可红。
    #[test]
    fn changing_the_pre_delay_length_never_replays_stale_audio() {
        /// 短预延迟（秒）：`1/1024` 在 `f32` 里精确，`· 48000 = 46.875` ⇒ 帧数 `46`。
        const SHORT_S: f32 = 1.0 / 1024.0;
        /// 短预延迟的帧数（由 [`SHORT_S`] 与 [`SR`] 算出，见上）。
        const SHORT_FRAMES: usize = 46;
        /// 长预延迟（秒）：`1/32` 在 `f32` 里精确，`· 48000 = 1500` ⇒ 帧数 `1500`。
        const LONG_S: f32 = 1.0 / 32.0;
        /// 长预延迟的帧数（由 [`LONG_S`] 与 [`SR`] 算出，见上）。
        const LONG_FRAMES: usize = 1_500;
        /// 观测长度（帧）：`1500` 帧预延迟 ＋ 最短梳状延迟 `1214` 帧都装得下。
        const PROBE: usize = 3_000;
        /// 灌进预延迟线的常数（线性幅度）。取 `7.0` 与
        /// `convolution_reverb::tests::changing_the_pre_delay_length_never_replays_stale_audio`
        /// 同值：清空所需的激励远在舍入底之上。
        const FILL: f32 = 7.0;

        /// 干路恒为 `0`（`dry = 1 − mix`），所以输出**只**含湿路 —— 旧音频因此无处可藏。
        fn wet_only(predelay: f32) -> ReverbParams {
            ReverbParams {
                size: 0.5,
                damp: 0.35,
                mix: 1.0,
                width: 1.0,
                predelay,
                kind: ReverbKind::Room,
                decay_s: 0.0,
            }
        }

        /// 两个声道的联合峰值。用 `abs()` ＋ 比较（而不是 `f32::max`）：`abs()` 把 `-0.0`
        /// 映成 `+0.0`，于是"峰值是不是**恰好** `0.0`"这个断言不受零的符号影响。
        fn peak(left: &[f32], right: &[f32]) -> f32 {
            let mut peak = 0.0f32;
            for (l, r) in left.iter().zip(right.iter()) {
                let value = l.abs();
                let value = if r.abs() > value { r.abs() } else { value };
                if value > peak {
                    peak = value;
                }
            }
            peak
        }

        // ---- 把短延迟的两条线灌满：`46` 帧常数 ⇒ 这一段读到的是写之前的值，全 `0` ----
        let mut stale = Reverb::new();
        stale.set_sample_rate(SR);
        stale.set_params(wet_only(SHORT_S));
        let mut fill_l = vec![FILL; SHORT_FRAMES];
        let mut fill_r = vec![FILL; SHORT_FRAMES];
        stale.process(&mut fill_l, &mut fill_r);
        let fill_peak = peak(&fill_l, &fill_r);

        // ---- 长度变更：`46` → `1500` 帧，随后只有静音 ----
        stale.set_params(wet_only(LONG_S));
        let mut probe_l = vec![0.0f32; PROBE];
        let mut probe_r = vec![0.0f32; PROBE];
        stale.process(&mut probe_l, &mut probe_r);
        let stale_peak = peak(&probe_l, &probe_r);

        // ---- 参照：**刚**按长延迟装配好的实例，喂同样的静音 ----
        let mut fresh = Reverb::new();
        fresh.set_sample_rate(SR);
        fresh.set_params(wet_only(LONG_S));
        let mut fresh_l = vec![0.0f32; PROBE];
        let mut fresh_r = vec![0.0f32; PROBE];
        fresh.process(&mut fresh_l, &mut fresh_r);
        let fresh_peak = peak(&fresh_l, &fresh_r);
        let differences = probe_l
            .iter()
            .chain(probe_r.iter())
            .zip(fresh_l.iter().chain(fresh_r.iter()))
            .filter(|(out, want)| out.to_bits() != want.to_bits())
            .count();

        // ---- 正对照：线里那份旧音频在**不变**的长度下确实听得见 ----
        let mut audible = Reverb::new();
        audible.set_sample_rate(SR);
        audible.set_params(wet_only(SHORT_S));
        let mut control_l = vec![FILL; SHORT_FRAMES];
        let mut control_r = vec![FILL; SHORT_FRAMES];
        audible.process(&mut control_l, &mut control_r);
        let mut tail_l = vec![0.0f32; PROBE];
        let mut tail_r = vec![0.0f32; PROBE];
        audible.process(&mut tail_l, &mut tail_r);
        let control_peak = peak(&tail_l, &tail_r);

        eprintln!(
            "[yeban-dsp] reverb::predelay 长度变更读数（单位：线性幅度 / 帧）: \
             灌线段峰值={fill_peak:e}（前置条件，须为 0）；长度 {SHORT_FRAMES}→{LONG_FRAMES} 帧后 \
             静音段峰值={stale_peak:e}、与新建实例的比特差异={differences}/{}；\
             长度不变的正对照峰值={control_peak:e}",
            2 * PROBE
        );
        assert_eq!(
            fill_peak, 0.0,
            "前置条件不成立：灌线段就出声了 ⇒ 梳状组已被激励，本判据测不出预延迟线的残留"
        );
        assert_eq!(
            fresh_peak, 0.0,
            "参照实例（刚按长延迟装配）在纯静音上就出声了 ⇒ 无法用它判残留"
        );
        assert_eq!(
            stale_peak,
            0.0,
            "预延迟变长后湿路重放了旧长度下写进缓冲的音频：峰值={stale_peak:e}、\
             与新建实例的比特差异={differences}/{}",
            2 * PROBE
        );
        assert!(
            control_peak > 0.0,
            "正对照不出声 ⇒ 这套夹具听不到线里的旧音频，② 是空断言"
        );
    }

    /// **判据（新写，可红）**：一段**湿路脉冲响应**被逐位冻结（位型哈希 +
    /// 首次到达的帧下标），三个夹具各一份。
    ///
    /// 量什么：
    /// - `48 kHz` / 预延迟 `12 ms` / `mix = 1.0`（纯湿）下 `8 192` 帧的
    ///   `left` ＋ `right` 全部 `f32` 位型折成一个 `FNV-1a 64`（覆盖 `16 384` 个位型）；
    /// - `96 kHz` / 预延迟 `0.1 s`（**参数上界**）/ 同上，`16 384` 帧；
    /// - `48 kHz` / 预延迟 `0.1 s` / 同上（用来钉"`0.2 s` 必须折到 `0.1 s`"）；
    /// - 三份夹具各自**首次非零湿样本的下标**（单位：帧）。
    ///
    /// ⚠ 脉冲只喂 **L** 声道：两声道相同时 `side = (wet_l − wet_r) · 0.5 · …`
    /// 恒为 `0`，`mid`／`side` 的两个 `0.5` 平均就都不可观测（实测：两声道都喂时
    /// 注入 R04/R07 存活）。
    ///
    /// 为什么需要它：本票的注入 R01–R07 **全部存活**（全量 417 条判据无一变红）。
    /// 既有判据测的是"尾巴是否衰减""干湿是否线性叠加""预延迟是否按秒位移"这类
    /// **结构**性质，对整条链的**绝对刻度**不敏感。被躲过的七处是：
    /// ① 参考采样率 `44 100 → 48 000`（改变全部梳状/全通长度）；
    /// ② 预延迟帧数上限差一（在 `96 kHz × 0.1 s` 这个角上才可观测：`9 600` 对 `9 599`）；
    /// ③ 输入混合的 `0.5` 平均（`(dl + dr) · 0.5 · 0.015`）；
    /// ④ `mid` 的 `0.5` 平均；
    /// ⑤ 全通反馈 `0.5`；
    /// ⑥ 预延迟**参数**上界 `0.1 s`（放宽成 `0.2 s` 后 `0.2 s` 请求各自生效）；
    /// ⑦ `side` 的 `0.5` 平均。
    /// 首次到达的下标对 ①／②／⑥ 敏感（长度变了），位型哈希对 ③／④／⑤／⑦ 敏感（刻度变了）。
    ///
    /// 跨架构（裁决 R24）：湿路只有 `+ − × ÷` 与比较，**没有**超越函数
    /// （`sin`／`cos` 只出现在本模块的测试夹具里）⇒ 属 ADR-0001 的 **IEEE 精确类**
    /// ⇒ 处处硬断言位型与下标，⛔ 不需要 `aarch64` 门。
    #[test]
    fn the_wet_impulse_response_is_frozen_bit_for_bit() {
        /// 把一整段位型折成一个 64 位数（与 `math.rs` 的判据同一算法）。
        fn fnv1a64(words: &[u64]) -> u64 {
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for word in words {
                for byte in word.to_le_bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
            hash
        }

        /// 纯湿夹具：一个单位脉冲进湿路（`dry = 1 − mix = 0`），返回
        /// （首次到达下标，位型哈希，位型总数）。
        fn reading(sample_rate: f32, predelay: f32, frames: usize) -> (usize, u64, usize) {
            let mut verb = Reverb::new();
            verb.set_sample_rate(sample_rate);
            verb.set_params(ReverbParams {
                size: 0.45,
                damp: 0.35,
                mix: 1.0,
                width: 0.8,
                predelay,
                kind: ReverbKind::Room,
                decay_s: 0.0,
            });
            let mut left = vec![0.0f32; frames];
            let mut right = vec![0.0f32; frames];
            // ⚠ **再调一次** `set_sample_rate`：预延迟帧数在 `set_sample_rate` 与
            // `set_params` 两处各算一次，只调一次时后者的读数会覆盖前者 ⇒
            // `set_sample_rate` 里的上限（`PREDELAY_MAX − 1`）不可观测（实测：注入 R02
            // 在只调一次的夹具下存活）。这一次调用不改变基线读数（长度与状态都和
            // 刚才那次相同），只让那条上限可观测。
            verb.set_sample_rate(sample_rate);
            // ⚠ 只喂 **L**：两声道相同时 `side = (wet_l − wet_r) · 0.5 · …` 恒为 0，
            // `mid`／`side` 的两个 `0.5` 平均就都不可观测（本票注入 R04/R07 实测）。
            left[0] = 1.0;
            verb.process(&mut left, &mut right);
            let first = left
                .iter()
                .position(|value| *value != 0.0)
                .expect("湿路必须有首次到达");
            let words: Vec<u64> = left
                .iter()
                .chain(right.iter())
                .map(|value| u64::from(value.to_bits()))
                .collect();
            (first, fnv1a64(&words), words.len())
        }

        let fast = reading(48_000.0, 0.012, 8_192);
        assert_eq!(fast.2, 16_384, "夹具规模变了 ⇒ 指纹的前提不再成立");

        let corner = reading(96_000.0, 0.1, 16_384);
        assert_eq!(corner.2, 32_768, "夹具规模变了 ⇒ 指纹的前提不再成立");

        // 预延迟的**参数上界**（0.1 秒）：`0.2 s` 必须与 `0.1 s` 落在同一条输出上。
        // 这是注入 R06 唯一可观测的地方（`0.2 s` 在 `48 kHz` 下是 `9 600` 帧，
        // 而上限 `PREDELAY_MAX − 1` 是 `9 599`）。
        let upper = reading(48_000.0, 0.1, 8_192);
        assert_eq!(upper.2, 16_384, "夹具规模变了 ⇒ 指纹的前提不再成立");
        let clamped = reading(48_000.0, 0.2, 8_192);
        assert_eq!(clamped.2, 16_384, "夹具规模变了 ⇒ 指纹的前提不再成立");

        assert_eq!(clamped.0, upper.0, "0.2 s 的预延迟请求必须被折到 0.1 s 上");
        assert_eq!(
            clamped.1, upper.1,
            "0.2 s 的预延迟请求必须给出与 0.1 s 相同的位型"
        );
        assert_ne!(upper.0, fast.0, "0.1 s 与 0.012 s 的首次到达必须不同");

        assert_eq!(fast.0, 1_790, "48 kHz 的首次到达下标漂移了");
        assert_eq!(
            fast.1, 0xf8b0_4ea6_a09c_bfeb,
            "48 kHz 的湿路脉冲响应位型漂移了"
        );
        assert_eq!(corner.0, 12_028, "96 kHz × 0.1 s 的首次到达下标漂移了");
        assert_eq!(
            corner.1, 0xd6a6_5b46_cfd5_e413,
            "96 kHz × 0.1 s 的湿路脉冲响应位型漂移了"
        );
        assert_eq!(upper.0, 6_014, "0.1 s 的首次到达下标漂移了");
        assert_eq!(
            upper.1, 0x748c_6054_5c60_6989,
            "0.1 s 的湿路脉冲响应位型漂移了"
        );

        // 非空证明：两份夹具必须给出**不同**的读数，否则"两个采样率各一份"
        // 这句话没有内容（例如某个读数恒为 0 时两条断言都会通过）。
        assert_ne!(fast.0, corner.0, "两个采样率的首次到达必须不同");
        assert_ne!(fast.1, corner.1, "两个采样率的位型必须不同");
    }

    /// **判据（新写，可红）**：**再次**调用 [`Reverb::set_sample_rate`] 必须丢弃
    /// 预延迟线里的历史（哪怕是同一个采样率）。
    ///
    /// 量什么：探测块的两条声道输出（`f32` 位型，帧数）。
    ///
    /// 夹具为什么要这样搭：预延迟线是**唯一**进入梳状组的通道，而梳状组一旦被激励
    /// 就有长尾。为了让"线里的历史"成为**唯一**可观测的差异，激励取"整块输入里只有
    /// 最后一个样本非零" —— 该样本要再过 `pre_len` 帧才会被读到，因此激励结束时
    /// 梳状组**一次也没被激励**，而预延迟线里**确实**有内容。
    ///
    /// 为什么需要它（机械读数）：[`Reverb::set_sample_rate`] 在预延迟线已按
    /// `PREDELAY_MAX` 分配时走 `else` 分支把两条线清零。把那两行 `fill(0.0)`
    /// 改成"只清第 0 格"时，全库 437 条判据**全绿**（实测：本票 48 次注入里的 R04）：
    /// 既有的 `changing_the_pre_delay_length_never_replays_stale_audio` 走的是
    /// [`Reverb::set_params`]，不覆盖这个入口。
    ///
    /// 注入实测：`set_sample_rate` 的 `else` 分支只清第 0 格 ⇒ 本判据变红。
    #[test]
    fn calling_set_sample_rate_again_drops_the_predelay_history() {
        /// 激励帧数：必须大于 `pre_len`（`0.05 s @ 48 kHz` = `2 400` 帧）。
        const EXCITE: usize = 8_192;
        /// 探测帧数：大于 `pre_len`，让残留的样本真的到达梳状组。
        const PROBE: usize = 4_096;

        let params = ReverbParams {
            size: 0.5,
            damp: 0.35,
            // `mix = 1.0` ⇒ 干路为 0，输出只来自湿路（梳状组 ＋ 预延迟线）。
            mix: 1.0,
            width: 0.8,
            predelay: 0.05,
            kind: ReverbKind::Room,
            decay_s: 0.0,
        };
        let build = || {
            let mut verb = Reverb::new();
            verb.set_sample_rate(SR);
            verb.set_params(params);
            verb
        };
        // 只有最后一个样本非零 ⇒ 激励结束时梳状组是静的，预延迟线里却有内容。
        let tail_excite = |verb: &mut Reverb| {
            let mut left = vec![0.0f32; EXCITE];
            let mut right = vec![0.0f32; EXCITE];
            left[EXCITE - 1] = 1.0;
            right[EXCITE - 1] = 1.0;
            verb.process(&mut left, &mut right);
        };

        // `rearmed`：第二次 `set_sample_rate`（按文档必须丢弃预延迟历史）。
        // `control`：不调第二次 —— 它是本判据的**牙**：它证明夹具的预延迟线里
        // 真的有会被重放的内容。
        let mut rearmed = build();
        tail_excite(&mut rearmed);
        rearmed.set_sample_rate(SR);
        let mut control = build();
        tail_excite(&mut control);

        let mut rearmed_l = vec![0.0f32; PROBE];
        let mut rearmed_r = vec![0.0f32; PROBE];
        rearmed.process(&mut rearmed_l, &mut rearmed_r);
        let mut control_l = vec![0.0f32; PROBE];
        let mut control_r = vec![0.0f32; PROBE];
        control.process(&mut control_l, &mut control_r);

        assert!(
            control_l.iter().any(|sample| sample.abs() > 1e-4)
                && control_r.iter().any(|sample| sample.abs() > 1e-4),
            "对照实例没有重放任何东西 ⇒ 夹具里根本没有预延迟历史，本判据没有判别力"
        );
        for (frame, sample) in rearmed_l.iter().enumerate() {
            assert_eq!(
                sample.to_bits(),
                0.0f32.to_bits(),
                "第 {frame} 帧：再次 set_sample_rate 之后预延迟线的旧内容被重放了（幅度 {}）",
                sample
            );
        }
        assert!(
            !rearmed_r.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(rearmed_r.iter().all(|sample| *sample == 0.0));
    }

    // ---- 类型与混响时间（模块文档 §7；缺口 P1/P2）----

    /// 窗 RMS 的 dB 序列（10 ms 窗）：T60 斜率法的输入。
    fn window_db(samples: &[f32]) -> Vec<f32> {
        samples
            .chunks(480)
            .map(|c| {
                let e = c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32;
                10.0 * e.max(1e-12).log10()
            })
            .collect()
    }

    /// 从 50 ms 处电平起算，衰减 `drop_db` 的穿越时间（秒，线性插值）。
    fn decay_crossing(db: &[f32], drop_db: f32) -> Option<f32> {
        let start = db[5];
        let target = start - drop_db;
        for i in 6..db.len() {
            if db[i] <= target {
                let (x0, x1) = (db[i - 1], db[i]);
                let frac = if x1 != x0 {
                    (target - x0) / (x1 - x0)
                } else {
                    0.0
                };
                return Some((i as f32 - 1.0 + frac) * 0.01);
            }
        }
        None
    }

    /// 斜率法 T60：−20 dB 与 −50 dB 穿越点之间的斜率外推到 60 dB
    /// （声学 RT60 的标准测法；直接找 −60 dB 穿越会被尾巴的早期
    /// 高频快衰段拉偏）。
    fn measured_t60(samples: &[f32]) -> f32 {
        let db = window_db(samples);
        let t20 = decay_crossing(&db, 20.0).expect("尾巴必须衰减到 −20 dB");
        let t50 = decay_crossing(&db, 50.0).expect("尾巴必须衰减到 −50 dB");
        60.0 / (30.0 / (t50 - t20))
    }

    /// 全湿脉冲响应的双声道和（判据统一走这条：梳状组本来就吃双声道和）。
    fn wet_impulse(params: ReverbParams, seconds: f32) -> Vec<f32> {
        let (l, r, _) = impulse_response(params, seconds);
        l.iter().zip(r.iter()).map(|(a, b)| a + b).collect()
    }

    /// ⭐ P2 判据：`decay_s` 写几秒，实测 T60 就是几秒（±8%）。
    ///
    /// 实测基线（48 kHz、damp 0.35、Room）：目标 0.4/1.0/2.2 s → 斜率法
    /// 读数 0.405/1.004/2.257 s。±8% 的窗比未加 [`T60_CALIBRATION`] 时的
    /// 系统偏差（约 −11%）更窄 ⇒ 标定常数被拿掉本判据必红。
    #[test]
    fn decay_s_tracks_the_measured_t60() {
        for target in [0.4f32, 1.0, 2.2] {
            let x = wet_impulse(
                ReverbParams {
                    mix: 1.0,
                    damp: 0.35,
                    predelay: 0.0,
                    width: 1.0,
                    decay_s: target,
                    ..Default::default()
                },
                target + 1.5,
            );
            let measured = measured_t60(&x);
            assert!(
                (measured - target).abs() <= target * 0.08,
                "decay_s={target} 时实测 T60={measured}，超出 ±8%"
            );
        }
    }

    /// ⭐ P1 判据：四档类型在**同一** `decay_s` 下渲染出可测地不同的尾巴，
    /// 且各自的性格量排序成立（不是只改了名字）。
    ///
    /// 实测基线（48 kHz、decay_s 1.2、damp 0.35）：逐对差分比最小
    /// 1.19（Plate↔Hall）；前 100 ms 反射峰计数 Hall 661 < Room 928
    /// < Plate 1023 < Spring 1156；高频占比 Plate 0.0446 > Room
    /// 0.0257 > Hall 0.0222 > Spring 0.0006。
    #[test]
    fn each_kind_renders_a_measurably_different_tail() {
        let base = ReverbParams {
            mix: 1.0,
            damp: 0.35,
            predelay: 0.0,
            width: 1.0,
            decay_s: 1.2,
            ..Default::default()
        };
        let renders: Vec<Vec<f32>> = ReverbKind::ALL
            .iter()
            .map(|kind| {
                wet_impulse(
                    ReverbParams {
                        kind: *kind,
                        ..base
                    },
                    3.0,
                )
            })
            .collect();
        // 逐对差分：任何两档的差分 RMS 必须大于各自 RMS 的一半。
        for i in 0..renders.len() {
            assert!(
                !renders[i].is_empty() && renders[i].iter().all(|s| s.is_finite()),
                "夹具必须非空且全有限（否则下面的差分断言没有判别力）"
            );
            for j in (i + 1)..renders.len() {
                let diff: Vec<f32> = renders[i]
                    .iter()
                    .zip(renders[j].iter())
                    .map(|(a, b)| a - b)
                    .collect();
                let ratio = rms(&diff) / rms(&renders[i]);
                assert!(
                    ratio > 0.5,
                    "{:?} 与 {:?} 的尾巴差分比只有 {ratio}，两档没有可测差异",
                    ReverbKind::ALL[i],
                    ReverbKind::ALL[j]
                );
            }
            // 每档自身必须衰减且有界。
            let head = rms(&renders[i][..48_000]);
            let tail = rms(&renders[i][96_000..]);
            assert!(tail < head * 0.5, "{:?} 的尾巴没有衰减", ReverbKind::ALL[i]);
        }
        // 早期反射密度：前 100 ms 内超过峰值 −40 dB 的局部极大值个数。
        // 大厅线最长 ⇒ 最稀疏；弹簧线最短＋强色散 ⇒ 最密。
        let early_peaks = |x: &[f32]| {
            let head = &x[..4800];
            let peak = head.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
            let thresh = peak * 0.01;
            (1..head.len() - 1)
                .filter(|&i| {
                    head[i].abs() > thresh
                        && head[i].abs() >= head[i - 1].abs()
                        && head[i].abs() > head[i + 1].abs()
                })
                .count()
        };
        let peaks: Vec<usize> = renders.iter().map(|x| early_peaks(x)).collect();
        let (room, plate, hall, spring) = (peaks[0], peaks[1], peaks[2], peaks[3]);
        assert!(
            hall < room && room < plate && plate < spring,
            "反射密度排序应为 Hall({hall}) < Room({room}) < Plate({plate}) < Spring({spring})"
        );
        // 高频占比：尾巴 1.0–1.5 s 段的差分能量比。板式最亮、弹簧最暗。
        let hf_ratio = |x: &[f32]| {
            let seg = &x[48_000..72_000];
            let e: f32 = seg.iter().map(|v| v * v).sum();
            let d: f32 = seg.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
            if e > 0.0 { d / e } else { 0.0 }
        };
        let hf: Vec<f32> = renders.iter().map(|x| hf_ratio(x)).collect();
        assert!(
            hf[1] > hf[0] && hf[0] > hf[2] && hf[2] > hf[3],
            "高频占比排序应为 Plate({}) > Room({}) > Hall({}) > Spring({})",
            hf[1],
            hf[0],
            hf[2],
            hf[3]
        );
        assert!(
            hf[3] < 0.005,
            "弹簧档的高频占比 {} 应显著低于其余三档",
            hf[3]
        );
    }

    /// ⭐ P1 判据（接线侧）：类型切换**重排清史**——切换后喂同一段输入，
    /// 输出与一台全新装配的目标类型实例逐位相同（预延迟先用静音排空，
    /// 它不属于类型布局）。切换不清史就会在这里以比特差现形。
    #[test]
    fn kind_switch_relayouts_and_leaves_no_stale_audio() {
        let base = ReverbParams {
            mix: 1.0,
            damp: 0.35,
            predelay: 0.0,
            width: 1.0,
            decay_s: 1.2,
            ..Default::default()
        };
        // 激励：确定性伪噪声（不用 RNG 对象，与本模块既有夹具同款）。
        let excite = |n: usize| -> (Vec<f32>, Vec<f32>) {
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            for i in 0..n {
                let t = ((i as u64 * 2_654_435_761) % 977) as f32 / 977.0 - 0.5;
                l[i] = t * 0.8;
                r[i] = -t * 0.6;
            }
            (l, r)
        };
        let mut switched = Reverb::new();
        switched.set_sample_rate(SR);
        switched.set_params(ReverbParams {
            kind: ReverbKind::Room,
            ..base
        });
        let (mut l, mut r) = excite(24_000);
        switched.process(&mut l, &mut r);
        assert!(
            l.iter().any(|s| s.abs() > 1e-4),
            "切换前的 Room 渲染必须是活的（否则本判据没有判别力）"
        );
        // 排空预延迟（静音 0.15 s > 最大预延迟 0.1 s），再切换类型。
        let (mut zl, mut zr) = (vec![0.0f32; 7_200], vec![0.0f32; 7_200]);
        switched.process(&mut zl, &mut zr);
        switched.set_params(ReverbParams {
            kind: ReverbKind::Hall,
            ..base
        });
        let (mut fresh_l, mut fresh_r) = excite(24_000);
        let (mut sw_l, mut sw_r) = (fresh_l.clone(), fresh_r.clone());
        let mut fresh = Reverb::new();
        fresh.set_sample_rate(SR);
        fresh.set_params(ReverbParams {
            kind: ReverbKind::Hall,
            ..base
        });
        fresh.process(&mut fresh_l, &mut fresh_r);
        switched.process(&mut sw_l, &mut sw_r);
        assert!(
            fresh_l.iter().any(|s| s.abs() > 1e-4),
            "对照实例的 Hall 渲染必须是活的（否则逐位相同是空判）"
        );
        for frame in 0..fresh_l.len() {
            assert_eq!(
                sw_l[frame].to_bits(),
                fresh_l[frame].to_bits(),
                "第 {frame} 帧左声道与全新 Hall 实例不同：切换留下了旧类型的历史"
            );
            assert_eq!(
                sw_r[frame].to_bits(),
                fresh_r[frame].to_bits(),
                "第 {frame} 帧右声道与全新 Hall 实例不同：切换留下了旧类型的历史"
            );
        }
    }

    /// 类型与时间的**参数面往返**：`from_f32` 的量化与 `index` 互逆，
    /// 非有限值回落缺省档；`decay_s` 的钳制与字段表 `sanitise` 同值
    /// （字段表侧的恒等由 render 的镜像判据钉住，这里钉器件侧读数）。
    #[test]
    fn kind_index_round_trips_and_decay_s_is_clamped() {
        for kind in ReverbKind::ALL {
            assert_eq!(ReverbKind::from_f32(kind.index()), *kind);
        }
        assert_eq!(ReverbKind::from_f32(f32::NAN), ReverbKind::Room);
        assert_eq!(ReverbKind::from_f32(99.0), ReverbKind::Spring);
        assert_eq!(ReverbKind::from_f32(-2.0), ReverbKind::Room);
        let mut verb = Reverb::new();
        verb.set_sample_rate(SR);
        verb.set_params(ReverbParams {
            decay_s: 99.0,
            ..Default::default()
        });
        assert_eq!(verb.params().decay_s, REVERB_MAX_DECAY_S);
        verb.set_params(ReverbParams {
            decay_s: f32::NAN,
            ..Default::default()
        });
        assert_eq!(verb.params().decay_s, 0.0);
    }
}
