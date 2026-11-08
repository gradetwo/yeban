//! **效果器参数契约**：`yeban_edit_notes` 的 `ops[].device.params[]` 认哪些名字、
//! 什么单位、什么值域。
//!
//! ## 为什么这份表住在 `yeban-mcp`
//!
//! 唯一**执行**这些名字的地方是 `crates/yeban-engine/src/insert.rs` 的三段
//! `match`（通道条 `:888`、混响 `:1050`、卷积 `:624`，行号以本片基线为准）。
//! 但 `yeban-mcp` **不能依赖** `yeban-engine`/`yeban-dsp`（依赖方向门
//! `scripts/gates/check_mcp_dependency_direction.py`）⇒ 本模块把**引擎真正认的名字**
//! 逐字抄一份。抄写不是"第二份语义"：它只是**只读的公告牌**，不参与任何计算；
//! 一致性由 `tests/effect_contract.rs` 机械对账（每个名字都必须以带引号字面量
//! 出现在 `insert.rs` 里）。
//!
//! ## 值域是**钳制域**，不是拒绝域
//!
//! 引擎对越界值取 `clamp`（`EqParams` / `ReverbParams` / `ConvolutionReverbParams`
//! 的 `sanitise`），**不报错**。因此这里的 `min`/`max` 是"有效值域"：写进 schema 的
//! `minimum`/`maximum` 告诉调用方**该落在哪**，而超出时引擎会钳到端点。
//!
//! ## 与"读回"共用同一份真相
//!
//! `yeban_query_engine_state` 的 `tracks[].inserts[].recognisedParams` /
//! `unrecognisedParams` 也走 [`recognise`] —— 公告牌与验收读数不会各说各话。
//!
//! ## #175：拼错的名字在**写入时**就响亮（⛔ 不再是"存下来、静默不生效"）
//!
//! 实测（#174 外部 agent 试跑）：
//!
//! ```text
//! {"name": "eq_low_gian", "value": 9.0}
//!   ⇒ status:"success"; inserts[0].recognisedParams:[]
//!     inserts[0].unrecognisedParams:["eq_low_gian"]   ← 没有错误
//! ```
//!
//! 外部 agent 于是**以为写成了**。现在 [`crate::domain::notes`] 的 `insertDevice`
//! 解析会调用 [`similar_spellings`]：拼写与某个已识别名字的**编辑距离 ≤
//! [`TYPO_MAX_DISTANCE`]** ⇒ `INVALID_PARAMETER_RANGE`，报文与 `data.suggestions`
//! 逐字给出**最近邻**（`eq_low_gian` ⇒ `eq_low_gain`）。
//!
//! 为什么是"**够近才拒绝**"而不是"未知就拒绝"：
//!
//! - `#156` 的验收面刻意接受"引擎不认、但不是拼错"的名字（`reverb_mix` /
//!   `reverb_width`），并要求它们出现在 `unrecognisedParams` 里。`reverb_mix` 与
//!   最近的已识别拼写 `reverb_wet` 差 **3** ⇒ 它**不**触发拒绝（阈值 2），
//!   `#156` 的判据因此逐字仍绿；
//! - 距离 ≤ 2 时我们能**确定**调用方想写哪个名字。既然能确定，就不该猜着写下去 ——
//!   拒绝并把可用名字逐字交还，比"成功但没生效"诚实。
//!
//! 证据：判据 `tests/trap_175.rs::a_misspelled_effect_param_is_refused_with_the_nearest_name`。
//!
//! ⚠ **刻意不认**的两个名字（引擎同样不认，见 `insert.rs` 的模块文档）：
//! `reverb_mix`、`reverb_width`。本表里也不许出现它们。

use serde_json::{Value, json};

/// 一个**引擎真正认**的效果器参数（规范名 + 别名 + 单位 + 钳制域 + 缺省值）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectParam {
    /// 规范名（表里的第一拼写，也是读回的 `canonicalName`）。
    pub name: &'static str,
    /// 引擎**同样认**的别名（逐字来自 `insert.rs` 的 `match` 分支）。
    pub aliases: &'static [&'static str],
    /// 单位（`"dB"` / `"Hz"` / `"s"` / `"Q"`；空串 = 无量纲）。
    pub unit: &'static str,
    /// 有效值域下界（引擎越界时钳到这里）。
    pub min: f64,
    /// 有效值域上界（引擎越界时钳到这里）。
    pub max: f64,
    /// 缺省值（引擎侧的投影基值）。
    pub default: f64,
    /// 一句话语义。
    pub doc: &'static str,
}

/// 一个效果器件分组（通道条 / EQ / 滤波 / 动态 / 混响 / 卷积混响）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectGroup {
    /// 分组 id（读回的 `effects[]` 用它）。
    pub id: &'static str,
    /// 给人看的标题。
    pub title: &'static str,
    /// 这句话会出现在 schema 的描述里。
    pub summary: &'static str,
    /// 本组认的全部参数。
    pub params: &'static [EffectParam],
}

/// 通道条：增益级、三个级开关、低切（`#341`）、立体声宽度（`#343`）、
/// mono-maker（`#344`）与 M/S 中频（`#346`）。
const CHANNEL_STRIP: EffectGroup = EffectGroup {
    id: "channelStrip",
    title: "通道条",
    summary: "`kind:\"InternalEffect\"` 的设备写这些名字会武装引擎内置通道条（增益级 + 低切 + EQ + 低通 + 压缩 + 宽度 / mono-maker / M/S 中频）",
    params: &[
        EffectParam {
            name: "input_gain_db",
            aliases: &["input_gain", "trim_db"],
            unit: "dB",
            min: -60.0,
            max: 24.0,
            default: 0.0,
            doc: "输入增益（trim，链上第一级）",
        },
        EffectParam {
            name: "output_gain_db",
            aliases: &["output_gain", "fader_db"],
            unit: "dB",
            min: -60.0,
            max: 24.0,
            default: 0.0,
            doc: "输出增益（推子，链上最后一级）",
        },
        EffectParam {
            name: "eq_enabled",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 1.0,
            doc: "EQ 级开关；值是 f32，0.0 = 关，非零 = 开（引擎按 `!= 0.0` 判）",
        },
        EffectParam {
            name: "filter_enabled",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 1.0,
            doc: "低通级开关；0.0 = 关，非零 = 开",
        },
        EffectParam {
            name: "compressor_enabled",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 1.0,
            doc: "动态级开关；0.0 = 关，非零 = 开",
        },
        EffectParam {
            name: "hpf_hz",
            aliases: &["high_pass_hz"],
            unit: "Hz",
            min: 0.0,
            max: 1000.0,
            default: 0.0,
            doc: "低切转折频率（`#341`，链上第二级，在 EQ 之前；≤ 0 = 旁通，20..=1000 = 真实滤波）",
        },
        EffectParam {
            name: "strip_width",
            aliases: &["channel_width"],
            unit: "",
            min: 0.0,
            max: 2.0,
            default: 1.0,
            doc: "立体声宽度（`#343`，M/S 缩放；1.0 = 原样，0.0 = 单声道，2.0 = 加倍）",
        },
        EffectParam {
            name: "mono_below_hz",
            aliases: &["mono_maker_hz"],
            unit: "Hz",
            min: 0.0,
            max: 1000.0,
            default: 0.0,
            doc: "mono-maker 转折频率（`#344`；≤ 0 = 旁通，该频率以下的 side 分量整段丢掉）",
        },
        EffectParam {
            name: "ms_mid_gain_db",
            aliases: &["ms_mid_gain"],
            unit: "dB",
            min: -60.0,
            max: 24.0,
            default: 0.0,
            doc: "M/S 中频增益（`#346`，只加在 mid 上的峰值均衡；0.0 = 旁通）",
        },
        EffectParam {
            name: "ms_mid_freq_hz",
            aliases: &["ms_mid_freq"],
            unit: "Hz",
            min: 100.0,
            max: 12000.0,
            default: 2000.0,
            doc: "M/S 中频峰值的中心频率（`#346`）",
        },
    ],
};

/// EQ：三段塑形（低架 / 中峰 / 高架）。
const EQ: EffectGroup = EffectGroup {
    id: "eq",
    title: "EQ",
    summary: "三段塑形 EQ（低架 + 中峰 + 高架）；名字见 `eq_*`，别名是去掉 `eq_` 前缀的短名",
    params: &[
        EffectParam {
            name: "eq_low_gain",
            aliases: &["low_gain"],
            unit: "dB",
            min: -18.0,
            max: 18.0,
            default: 0.0,
            doc: "低架增益",
        },
        EffectParam {
            name: "eq_low_freq",
            aliases: &["low_freq"],
            unit: "Hz",
            min: 20.0,
            max: 1_000.0,
            default: 200.0,
            doc: "低架转折频率",
        },
        EffectParam {
            name: "eq_mid_gain",
            aliases: &["mid_gain"],
            unit: "dB",
            min: -18.0,
            max: 18.0,
            default: 0.0,
            doc: "中频峰值增益",
        },
        EffectParam {
            name: "eq_mid_freq",
            aliases: &["mid_freq"],
            unit: "Hz",
            min: 200.0,
            max: 8_000.0,
            default: 1_000.0,
            doc: "中频中心频率",
        },
        EffectParam {
            name: "eq_mid_q",
            aliases: &["mid_q"],
            unit: "Q",
            min: 0.3,
            max: 6.0,
            default: 0.9,
            doc: "中频品质因数（频带宽度）",
        },
        EffectParam {
            name: "eq_high_gain",
            aliases: &["high_gain"],
            unit: "dB",
            min: -18.0,
            max: 18.0,
            default: 0.0,
            doc: "高架增益",
        },
        EffectParam {
            name: "eq_high_freq",
            aliases: &["high_freq"],
            unit: "Hz",
            min: 1_000.0,
            max: 16_000.0,
            default: 4_000.0,
            doc: "高架转折频率",
        },
    ],
};

/// 滤波：一级低通。
const FILTER: EffectGroup = EffectGroup {
    id: "filter",
    title: "低通滤波",
    summary: "一级低通滤波（截止 / 谐振 / 驱动）",
    params: &[
        EffectParam {
            name: "cutoff_hz",
            aliases: &[],
            unit: "Hz",
            min: 20.0,
            max: 20_000.0,
            default: 20_000.0,
            doc: "低通截止频率（⚠ 没有 `filter_cutoff_hz` 这个别名，引擎刻意不认）",
        },
        EffectParam {
            name: "resonance",
            aliases: &["filter_resonance"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            doc: "谐振量（0..=1 的旋钮值）",
        },
        EffectParam {
            name: "drive",
            aliases: &["filter_drive"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            doc: "驱动量（0..=1 的旋钮，**等 dB** 映射到 0..=+36 dB 预增益）。`0` 是逐位旁通；\
                  输出按 0.25 参考幅度自补偿 ⇒ 只换谐波、不换音量。⚠ 修前只有 0..=+5.1 dB\
                  （`1.0 + 旋钮 · 0.8`）⇒ 在 −12 dBFS 的声部上谐波比约 −31 dB，听感是\"旋钮没反应\"。\
                  本旋钮是**合成器与通道条唯一的失真级**：要\"后级失真\"就把 `cutoff_hz` 开大再拧它",
        },
    ],
};

/// 动态：压缩器七旋钮。
const DYNAMICS: EffectGroup = EffectGroup {
    id: "dynamics",
    title: "动态（压缩）",
    summary: "软膝压缩器（阈值 / 比率 / 膝宽 / 检波 / 起振 / 释放 / makeup）",
    params: &[
        EffectParam {
            name: "threshold_db",
            aliases: &["threshold"],
            unit: "dBFS",
            min: -120.0,
            max: 24.0,
            default: -12.0,
            doc: "压缩阈值",
        },
        EffectParam {
            name: "ratio",
            aliases: &[],
            unit: ":1",
            min: 1.0,
            max: 1_000.0,
            default: 4.0,
            doc: "压缩比（1 = 不压缩）",
        },
        EffectParam {
            name: "knee_db",
            aliases: &["knee"],
            unit: "dB",
            min: 0.0,
            max: 24.0,
            default: 6.0,
            doc: "软膝宽度",
        },
        EffectParam {
            name: "detector_s",
            aliases: &["detector"],
            unit: "s",
            min: 0.000_1,
            max: 10.0,
            default: 0.005,
            doc: "检波器平均窗口",
        },
        EffectParam {
            name: "attack_s",
            aliases: &["attack"],
            unit: "s",
            min: 0.000_1,
            max: 10.0,
            default: 0.01,
            doc: "起振时间常数",
        },
        EffectParam {
            name: "release_s",
            aliases: &["release"],
            unit: "s",
            min: 0.000_1,
            max: 10.0,
            default: 0.1,
            doc: "释放时间常数",
        },
        EffectParam {
            name: "makeup_db",
            aliases: &["makeup"],
            unit: "dB",
            min: -24.0,
            max: 24.0,
            default: 0.0,
            doc: "补偿增益（负值合法）",
        },
    ],
};

/// 动态 EQ（`#342`，通道条第四级，在 EQ 之后、低通之前）。
const DYN_EQ: EffectGroup = EffectGroup {
    id: "dynEq",
    title: "动态 EQ",
    summary: "窄带动态衰减（扫描找点、压指定频段的刺 / 齿音）；`deq_cut_db = 0` = 旁通",
    params: &[
        EffectParam {
            name: "deq_freq",
            aliases: &["dyn_eq_freq"],
            unit: "Hz",
            min: 80.0,
            max: 12000.0,
            default: 300.0,
            doc: "被处理频带的中心频率",
        },
        EffectParam {
            name: "deq_threshold_db",
            aliases: &["dyn_eq_threshold_db"],
            unit: "dBFS",
            min: -60.0,
            max: 0.0,
            default: -24.0,
            doc: "触发阈值（带通电平）",
        },
        EffectParam {
            name: "deq_cut_db",
            aliases: &["dyn_eq_cut_db"],
            unit: "dB",
            min: 0.0,
            max: 18.0,
            default: 0.0,
            doc: "最大衰减量（正数；0 = 旁通）",
        },
        EffectParam {
            name: "deq_q",
            aliases: &["dyn_eq_q"],
            unit: "Q",
            min: 0.3,
            max: 8.0,
            default: 0.9,
            doc: "带通 / 峰值的 Q",
        },
        EffectParam {
            name: "deq_ratio",
            aliases: &["dyn_eq_ratio"],
            unit: "",
            min: 1.0,
            max: 20.0,
            default: 4.0,
            doc: "压缩比（1.0 = 不压，等价旁通）",
        },
    ],
};

/// 混响：Freeverb（插入式）。
const REVERB: EffectGroup = EffectGroup {
    id: "reverb",
    title: "混响（Freeverb）",
    summary: "插入式 Freeverb；湿路参数叫 `reverb_wet`（⚠ 引擎刻意不认 `reverb_mix` / `reverb_width`）",
    params: &[
        EffectParam {
            name: "reverb_size",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.45,
            doc: "衰减长度（映射到梳状反馈 0.72…0.985）",
        },
        EffectParam {
            name: "reverb_damp",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.35,
            doc: "尾巴里的高频阻尼",
        },
        EffectParam {
            name: "reverb_wet",
            aliases: &["reverb_wet_mix"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.25,
            doc: "湿/干平衡（湿路电平）",
        },
        EffectParam {
            name: "reverb_predelay",
            aliases: &["reverb_predelay_s"],
            unit: "s",
            min: 0.0,
            max: 0.1,
            default: 0.012,
            doc: "湿路预延迟",
        },
    ],
};

/// 卷积混响：合成 IR 的插入式卷积。
const CONVOLUTION: EffectGroup = EffectGroup {
    id: "convolution",
    title: "卷积混响",
    summary: "插入式卷积混响（引擎按 `conv_ir_decay_s` / `conv_ir_seed` 合成 IR；`conv_ir_stereo` 开立体声 IR）",
    params: &[
        EffectParam {
            name: "conv_predelay_s",
            aliases: &["conv_pre_delay_s", "conv_predelay"],
            unit: "s",
            min: 0.0,
            max: 0.1,
            default: 0.0,
            doc: "湿路预延迟",
        },
        EffectParam {
            name: "conv_dry",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 1.0,
            doc: "干路电平（线性）",
        },
        EffectParam {
            name: "conv_wet",
            aliases: &["conv_wet_mix"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.25,
            doc: "湿路电平（线性）",
        },
        EffectParam {
            name: "conv_ir_gain_db",
            aliases: &["conv_ir_gain"],
            unit: "dB",
            min: -60.0,
            max: 24.0,
            default: 0.0,
            doc: "IR 的增益",
        },
        EffectParam {
            name: "conv_ir_decay_s",
            aliases: &["conv_ir_decay"],
            unit: "s",
            min: 0.005,
            max: 5.0,
            default: 0.35,
            doc: "合成 IR 的指数衰减时间常数",
        },
        EffectParam {
            name: "conv_ir_seed",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 4_294_967_295.0,
            default: 1.0,
            doc: "合成 IR 的随机种子（f32 → u32 饱和转换）",
        },
        EffectParam {
            name: "conv_ir_stereo",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            doc: "IR 立体声开关：0 = 左右同 IR（缺省，老工程不变），非 0 = 左右独立 IR（左 = seed，右 = seed + 黄金比例常数）；构造期烘焙，不可自动化",
        },
    ],
};

/// 乒乓延迟：左右交替回声的插入式延迟（复用 `yeban-dsp` 的 `Delay` 器件）。
const DELAY: EffectGroup = EffectGroup {
    id: "delay",
    title: "乒乓延迟",
    summary: "插入式乒乓延迟（链尾、卷积之后；`delay_mix = 0` 时整级不建）",
    params: &[
        EffectParam {
            name: "delay_time_s",
            aliases: &["delay_time"],
            unit: "s",
            min: 0.001,
            max: 2.0,
            default: 0.3,
            doc: "延迟时间（同步关时直接用它；同步开时由工程拍速换算，超 2.0 s 钳制）",
        },
        EffectParam {
            name: "delay_feedback",
            aliases: &["delay_fb"],
            unit: "",
            min: 0.0,
            max: 0.95,
            default: 0.3,
            doc: "反馈量（上限 0.95，防自激）",
        },
        EffectParam {
            name: "delay_mix",
            aliases: &["delay_wet"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            doc: "湿路电平；0 = 整级不建（级缺省关闭）",
        },
        EffectParam {
            name: "delay_damp",
            aliases: &["delay_damping"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            doc: "反馈回路里的高频阻尼",
        },
        EffectParam {
            name: "delay_ping_pong",
            aliases: &["delay_pingpong"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 1.0,
            doc: "乒乓开关（非零即真；缺省 1 = 功能缺省即乒乓）",
        },
        EffectParam {
            name: "delay_sync_enabled",
            aliases: &["delay_sync"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            doc: "音符同步开关（开时延迟时间由工程拍速在投影期换算，不在块边界重算）",
        },
        EffectParam {
            name: "delay_sync_num",
            aliases: &[],
            unit: "",
            min: 1.0,
            max: 16.0,
            default: 3.0,
            doc: "同步分子（拍数，取整后换算）",
        },
        EffectParam {
            name: "delay_sync_den",
            aliases: &[],
            unit: "",
            min: 1.0,
            max: 16.0,
            default: 8.0,
            doc: "同步分母（拍数，取整后换算）",
        },
        EffectParam {
            name: "delay_sync_dotted",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 1.0,
            doc: "附点开关（非零即真，时长 ×1.5；换算后超 2.0 s 钳制）",
        },
    ],
};

/// 磁带饱和：链级饱和器（`ADR-0062`）。
const TAPE: EffectGroup = EffectGroup {
    id: "tape",
    title: "磁带饱和",
    summary: "链级磁带饱和（`pade_tanh`＋2x 过采样＋湿路低通），位置在通道条之后、Freeverb 之前；链级 ⇒ 覆盖采样器轨（⚠ 通道条的 `drive` 是滤波器驱动，不作用于采样器轨）。级建成条件：`tape_drive > 0` 且 `tape_mix > 0`（任一缺省 ⇒ 不建器件，不另设 enabled 开关）。过采样往返延迟 31 采样，进器件的 `latency_samples`（PDC 唯一事实源）。",
    params: &[
        EffectParam {
            name: "tape_drive",
            aliases: &["saturation_drive"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            doc: "饱和驱动（曲线 `pade_tanh(g·x)/g`，`g = 1 + 9·drive`，小信号增益 ≈ 1）",
        },
        EffectParam {
            name: "tape_mix",
            aliases: &["saturation_mix"],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            doc: "干湿比（0 = 干路直通）",
        },
        EffectParam {
            name: "tape_trim_db",
            aliases: &["saturation_trim_db"],
            unit: "dB",
            min: -24.0,
            max: 24.0,
            default: 0.0,
            doc: "输出补偿增益",
        },
        EffectParam {
            name: "tape_hf_hz",
            aliases: &["saturation_hf_hz"],
            unit: "Hz",
            min: 2000.0,
            max: 20000.0,
            default: 20000.0,
            doc: "湿路一阶低通截止频率（≥ 20000 时旁通）",
        },
        EffectParam {
            name: "tape_asym",
            aliases: &["saturation_asym"],
            unit: "",
            min: 0.0,
            max: 0.5,
            default: 0.0,
            doc: "曲线不对称量（0 = 对称 ⇒ 纯奇次谐波；> 0 产生偶次谐波，磁带「暖」的来源）。\
                  ⚠ 只在级建成时生效（`tape_drive > 0` 且 `tape_mix > 0`）；\
                  小信号增益会略降（−1 dB 量级），用 `tape_trim_db` 补",
        },
    ],
};

/// 插入式限制器：前瞻 + 峰值检测的砖墙限制器（**本组是新增器件**）。
///
/// 与母线那份（`BusLimiter`，阈值/天花板写死、永远在 master 末端）不同，
/// 这一份是**可插在任意轨上的器件**，四个旋钮全部生效：
///
/// - **前瞻**（`limiter_lookahead`，帧）：输出相对输入后移这么多帧，同时就是
///   上报给 PDC 的延迟（渲染侧按它补相位）；
/// - **真峰值**（`limiter_true_peak`）：检波器在样本之间做三次插值估计，
///   抓"样本没超但重建波形超了"的那一类峰；
/// - **天花板**（`limiter_ceiling_db`）：软膝渐近到它 ⇒ 输出**严格**不超过它；
/// - **释放**（`limiter_release_ms`）：增益从 0 回到 1 所需时间。
///
/// 攻击没有旋钮，是**一步到位**的：任何"慢攻击"都会让瞬态漏过阈值，
/// 那正是限制器与压缩器的分界。要软一点请用动态组的压缩器。
const LIMITER: EffectGroup = EffectGroup {
    id: "limiter",
    title: "限制器（前瞻 + 峰值检测）",
    summary: "砖墙限制器：前瞻延迟 + 峰值/真峰值检波 + 软膝天花板。攻击一步到位，\
              释放可调。链上位置在**全部插入之后**（本轨最后一级）。\
              前瞻帧数会计入 PDC（`latency_samples`），多轨合流不会相位错位。",
    params: &[
        EffectParam {
            name: "limiter_threshold_db",
            aliases: &[],
            unit: "dB",
            min: -30.0,
            max: 0.0,
            default: -1.0,
            doc: "阈值：窗口峰值超过它就压。默认 −1 dBFS",
        },
        EffectParam {
            name: "limiter_ceiling_db",
            aliases: &[],
            unit: "dB",
            min: -6.0,
            max: 0.0,
            default: -0.3,
            doc: "天花板：软膝渐近值，输出严格不超过它。必须高于阈值",
        },
        EffectParam {
            name: "limiter_release_ms",
            aliases: &[],
            unit: "ms",
            min: 10.0,
            max: 1000.0,
            default: 80.0,
            doc: "释放：增益从 0 回到 1 所需时间（每样本回升量 = 1/(ms×sr/1000)）",
        },
        EffectParam {
            name: "limiter_lookahead",
            aliases: &[],
            unit: "frames",
            min: 1.0,
            max: 192.0,
            default: 48.0,
            doc: "前瞻帧数（1–192；48 kHz 下 1 帧 ≈ 0.021 ms）。同时是本器件上报的延迟",
        },
        EffectParam {
            name: "limiter_true_peak",
            aliases: &[],
            unit: "",
            min: 0.0,
            max: 1.0,
            default: 1.0,
            doc: "真峰值检测（1 = 开）：在样本之间做三次插值估计，抓 inter-sample peak。\
                  口径是估计器，⛔ 不是 ITU-R BS.1770-4 的 4× 过采样表",
        },
    ],
};

/// 全部**引擎真正认**的效果器参数，按器件分组（顺序稳定，schema 与读回共用）。
pub const EFFECT_GROUPS: [EffectGroup; 10] = [
    CHANNEL_STRIP,
    EQ,
    FILTER,
    DYNAMICS,
    DYN_EQ,
    REVERB,
    CONVOLUTION,
    TAPE,
    DELAY,
    LIMITER,
];

/// 按名字（大小写不敏感）查一个参数的归属。
///
/// 返回 `(分组, 参数)`；`None` = 引擎不认这个拼写（写进设备也**不会**生效）。
#[must_use]
pub fn recognise(name: &str) -> Option<(&'static EffectGroup, &'static EffectParam)> {
    EFFECT_GROUPS.iter().find_map(|group| {
        group
            .params
            .iter()
            .find(|param| {
                param.name.eq_ignore_ascii_case(name)
                    || param
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(name))
            })
            .map(|param| (group, param))
    })
}

/// 全部被认的拼写（规范名在前、别名紧随），用于公告与判据。
///
/// 现下有三个公告口用它：`insertDevice.device.params.items.oneOf`（`op_schema`）、
/// 读回的 `inserts[].recognisedParams` 口径，以及 G14 起每条
/// `unrecognisedEffectParam` 警告里的 `knownParams` —— 同一个词表，⛔ 不允许第四份手抄。
#[must_use]
pub fn recognised_names() -> Vec<&'static str> {
    let mut names = Vec::new();
    for group in &EFFECT_GROUPS {
        for param in group.params {
            names.push(param.name);
            names.extend(param.aliases.iter().copied());
        }
    }
    names
}

/// 拼写"最近邻"的**编辑距离上限**（Levenshtein，⛔ 不是字符重合度）。
///
/// 取 **2** 是一个刻意的窄门槛，两端都有实测依据：
///
/// - **挡住手滑**：`eq_low_gian`（2）、`cuttof_hz`（1）、`reverb_siz`（1）都落在门槛内；
/// - **不碰刻意不认的名字**：`reverb_mix` 与最近的已识别拼写 `reverb_wet` 差 **3**
///   ⇒ 仍走 `#156` 的"存储 + 读回 `unrecognisedParams`"那条路（判据
///   `tests/effect_contract.rs::eq_and_reverb_round_trip_and_sends_are_readable` 钉住）。
pub const TYPO_MAX_DISTANCE: usize = 2;

/// 一个与调用方拼写"够近"的已识别拼写（错字建议）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimilarSpelling {
    /// 引擎认的拼写（可能是别名，例如 `low_gain`）。
    pub spelling: &'static str,
    /// 该拼写所属参数的**规范名**（建议优先写它）。
    pub canonical: &'static str,
    /// 该拼写所属的效果器件分组 id（`eq` / `reverb` / …）。
    pub group: &'static str,
    /// 与调用方拼写的编辑距离（`1..=TYPO_MAX_DISTANCE`）。
    pub distance: usize,
}

impl SimilarSpelling {
    /// 进错误 `data.suggestions[]` 的 JSON 形态（`requested` 是调用方写的那个词）。
    #[must_use]
    pub fn to_value(self, requested: &str) -> Value {
        json!({
            "requested": requested,
            "spelling": self.spelling,
            "canonicalName": self.canonical,
            "effect": self.group,
            "distance": self.distance,
        })
    }
}

/// 与 `name` 的编辑距离 ≤ [`TYPO_MAX_DISTANCE`] 的全部已识别拼写（按距离、再按拼写排序）。
///
/// 返回空 = "没有任何已识别拼写像它" ⇒ 该名字不是可确定的错字（可能是外部插件自己的
/// 参数名，也可能是 `reverb_mix` 那种"引擎刻意不认"的名字）⇒ 调用方**不**拒绝它，
/// 由提案响应的 `warnings[]` 与读回的 `unrecognisedParams` 如实报出。
#[must_use]
pub fn similar_spellings(name: &str) -> Vec<SimilarSpelling> {
    let mut found: Vec<SimilarSpelling> = Vec::new();
    for group in &EFFECT_GROUPS {
        for param in group.params {
            for spelling in std::iter::once(param.name).chain(param.aliases.iter().copied()) {
                let distance = edit_distance(name, spelling);
                if distance > 0 && distance <= TYPO_MAX_DISTANCE {
                    found.push(SimilarSpelling {
                        spelling,
                        canonical: param.name,
                        group: group.id,
                        distance,
                    });
                }
            }
        }
    }
    found.sort_unstable_by(|left, right| {
        left.distance
            .cmp(&right.distance)
            .then_with(|| left.spelling.cmp(right.spelling))
    });
    found
}

/// 两个拼写的 **Levenshtein 编辑距离**（插入 / 删除 / 替换各记 1，⛔ 不算相邻换位）。
///
/// 大小写不敏感（与 [`recognise`] 同一口径：引擎把名字转小写后匹配）。
/// 只用两行 DP，因此不会为一次错误路径分配平方级内存。
#[must_use]
pub fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().map(|c| c.to_ascii_lowercase()).collect();
    let right: Vec<char> = right.chars().map(|c| c.to_ascii_lowercase()).collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current: Vec<usize> = vec![0; right.len() + 1];
    for (row, left_char) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != right_char);
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

/// 一个参数在 `device.params.items` 里的 JSON Schema 分支。
///
/// 形状是"**名字用 `enum` 锁定、值带 `minimum`/`maximum`**"的对象：
/// 名字 + 类型 + 范围 + 单位 + 缺省值都是机器可读的，⛔ 不靠一段散文去猜。
fn param_schema(group: &'static EffectGroup, param: &'static EffectParam) -> Value {
    let mut names: Vec<&'static str> = vec![param.name];
    names.extend(param.aliases.iter().copied());
    let unit = if param.unit.is_empty() {
        "无量纲 / 开关".to_owned()
    } else {
        param.unit.to_owned()
    };
    json!({
        "type": "object",
        "title": format!("{} · {}", group.title, param.name),
        "description": format!(
            "{}（{}）。值域 {}..={} {}（越界由引擎钳到端点，不报错）；缺省 {}。",
            param.doc, group.title, param.min, param.max, unit, param.default,
        ),
        "required": ["name", "value"],
        "properties": {
            "name": {
                "enum": names,
                "description": format!(
                    "引擎认的拼写（大小写不敏感）；规范名 `{}`{}",
                    param.name,
                    if param.aliases.is_empty() {
                        String::new()
                    } else {
                        format!("，别名 {}", param.aliases.join(" / "))
                    },
                ),
            },
            "value": {
                "type": "number",
                "minimum": param.min,
                "maximum": param.max,
                "default": param.default,
                "description": format!("{}；单位 {}", param.doc, unit),
            },
            "unit": {
                "type": ["string", "null"],
                "description": format!("可选；建议写 \"{}\"（引擎不读这个字段，只作注记）", param.unit),
            },
        },
        "additionalProperties": false,
    })
}

/// `device.params.items` 的 Schema：每个已识别参数一个分支。
#[must_use]
pub fn param_schema_branches() -> Value {
    Value::Array(
        EFFECT_GROUPS
            .iter()
            .flat_map(|group| {
                group
                    .params
                    .iter()
                    .map(move |param| param_schema(group, param))
            })
            .collect(),
    )
}
