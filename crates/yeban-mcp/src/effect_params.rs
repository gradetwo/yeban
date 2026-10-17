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

/// 通道条：增益级与三个级开关。
const CHANNEL_STRIP: EffectGroup = EffectGroup {
    id: "channelStrip",
    title: "通道条",
    summary: "`kind:\"InternalEffect\"` 的设备写这些名字会武装引擎内置通道条（增益级 + EQ + 低通 + 压缩）",
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
            doc: "驱动量（0..=1 的旋钮值）",
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
    summary: "插入式卷积混响（引擎按 `conv_ir_decay_s` / `conv_ir_seed` 合成 IR）",
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
    ],
};

/// 全部**引擎真正认**的效果器参数，按器件分组（顺序稳定，schema 与读回共用）。
pub const EFFECT_GROUPS: [EffectGroup; 6] =
    [CHANNEL_STRIP, EQ, FILTER, DYNAMICS, REVERB, CONVOLUTION];

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
