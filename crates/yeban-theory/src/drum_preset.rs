//! **鼓机风格预置包**（一期）：格式定义、解析与出厂预置。
//!
//! 格式是 JSON，模式见 `crates/yeban-theory/drum_styles/FORMAT.md`。本模块
//! 只做三件事：把 JSON 文本解析成 [`DrumStylePreset`]、校验形状与值域、
//! 给出仓内出厂的 14 个风格（`include_str!` 嵌入，解析逻辑与将来从
//! yeban-extras 下载的同格式文件共用同一条路径）。
//!
//! 数据来源：出厂 10 风格改编自 groove 仓（`gradetwo/groove`，MIT）的
//! 流派 pattern 数据，经 `scripts/drum_styles/convert_groove.py` 机械
//! 转换 + 调优（扩 2 小节、力度 shaping、hat roll、fill、tier 分配）。
//! 每份 JSON 的 `source` 字段逐条留痕（源流派 id、许可、调优项）。
//!
//! 确定性：解析是纯函数；同一文本恒得同一结构。本模块不读时钟、不做
//! 文件 I/O（嵌入文本除外）、不用 `HashMap`/`HashSet`。
//!
//! 本模块自带一个**严格子集** JSON 解析器：`yeban-theory` 刻意只依赖
//! `thiserror` + `libm`（见 `crates/yeban-mcp/Cargo.toml` 的依赖方向
//! 注释），不为读自己的预置格式引入 serde。解析器只接受 JSON 的标准
//! 语法（对象/数组/字符串/数字/布尔/null），非法输入响亮报错并给字节
//! 偏移，⛔ 不做任何容错猜测。

use thiserror::Error;

/// 预置格式名（JSON 顶层 `format` 字段的唯一合法值）。
pub const DRUM_STYLE_FORMAT: &str = "yeban-drum-style";

/// 预置格式版本（JSON 顶层 `version` 字段的唯一合法值）。
pub const DRUM_STYLE_FORMAT_VERSION: u64 = 1;

/// 一小节的步数（16 分音符网格）。
pub const STEPS_PER_BAR: usize = 16;

/// 预置步进的最大 ratchet（一步内等分音数）。
pub const MAX_RATCHET: u8 = 8;

/// 预置解析/校验失败。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DrumPresetError {
    /// JSON 语法错误（带字节偏移）。
    #[error("drum style JSON parse failed at byte {offset}: {message}")]
    Parse {
        /// 出错位置（字节偏移）。
        offset: usize,
        /// 错误说明（英文，供日志与测试断言）。
        message: String,
    },
    /// 字段形状或值域错误。
    #[error("drum style preset field `{field}` invalid: {message}")]
    Field {
        /// 出错字段名。
        field: String,
        /// 错误说明（英文，供日志与测试断言）。
        message: String,
    },
}

fn field_err(field: &str, message: impl Into<String>) -> DrumPresetError {
    DrumPresetError::Field {
        field: field.to_owned(),
        message: message.into(),
    }
}

/// 鼓组族：决定生成出的鼓轨挂哪件音源（见 [`kit_setup`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KitFamily {
    /// 原声：SFZ 采样鼓组（Virtuosity Drums 基础组，GM 映射）。
    Acoustic,
    /// 电子：DSP 合成鼓机（`yeban-dsp` DrumMachine，5 键 GM 映射）。
    Electronic,
}

/// 一格步进：`[力度, ratchet, tier]` 的展开形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrumStep {
    /// 力度 0..=127；0 = 休止。
    pub velocity: u8,
    /// 一格内的等分音数 1..=8（3 = 三连音式三分）；休止格恒为 1。
    pub ratchet: u8,
    /// 复杂度层级：0 = 核心、1 = 标准、2 = 装饰。生成时按复杂度旋钮取舍。
    pub tier: u8,
}

impl DrumStep {
    /// 休止格。
    pub const REST: Self = Self {
        velocity: 0,
        ratchet: 1,
        tier: 0,
    };

    /// 这一格是否有击。
    #[must_use]
    pub const fn is_hit(self) -> bool {
        self.velocity > 0
    }
}

/// 一条鼓道的一个**叠层键**（分层音色，缺口 P6）：与主道同格同刻
/// 触发的附加 GM 键，有自己的电平与力度窗。军鼓叠 clap 是典型
/// 用法：主道写满 pattern，叠层跟随发声——格式能表达叠层之前，
/// 这类内容只能单开一条道手抄同一串步进（hiphop 的 clap 道即是）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumLaneLayer {
    /// 叠层 GM 打击乐键号 0..=127。不得与本道主键、本道其它叠层键、
    /// 同表其它道的主键或叠层键重复（解析期响亮拒绝，保生成层
    /// 「同 tick 同键至多一项」不变量）。
    pub gm_key: u8,
    /// 相对主道的电平（dB）−48..=12，缺省 0。生成时按振幅口径
    /// `10^(dB/20)` 缩放该格的写力度，再走统一的强度/ratchet 变换。
    pub level_db: f32,
    /// 力度窗下限 1..=127，缺省 1：写力度低于此值的格不触发本层。
    /// 窗对**预置写力度**判定，不含生成期的强度缩放。
    pub vel_min: u8,
    /// 力度窗上限 1..=127，缺省 127：写力度高于此值的格不触发本层。
    pub vel_max: u8,
}

impl DrumLaneLayer {
    /// 本层是否在写力度 `velocity` 的格上触发（力度窗判定）。
    #[must_use]
    pub const fn fires_on(self, velocity: u8) -> bool {
        velocity >= self.vel_min && velocity <= self.vel_max
    }
}

/// 一条鼓道（一个 GM 键）的完整 pattern（`bars * 16` 格）。
/// （不派生 `Eq`：`layers` 的 `level_db` 是 `f32`。）
#[derive(Debug, Clone, PartialEq)]
pub struct DrumLanePreset {
    /// 道角色名（`kick` / `snare` / `hihat` / `clap` / `open_hat` / …）。
    pub role: String,
    /// GM 打击乐键号 0..=127。
    pub gm_key: u8,
    /// 叠层键（`layers` 段）；空 = 单键道。旧格式无此段 ⇒ 空，
    /// 生成结果与引入本字段前逐位一致。
    pub layers: Vec<DrumLaneLayer>,
    /// 步进格，长度恒为 `bars * STEPS_PER_BAR`。
    pub steps: Vec<DrumStep>,
}

/// 磁带饱和段（`fx.tape`，可选）：给整组鼓加谐波与轻微高频滚降，
/// 并可带 wow/flutter 走带调制（P7；深度 0 = 无调制）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumTapePreset {
    /// 驱动量 0..=1。
    pub drive: f32,
    /// 干湿比 0..=1。
    pub mix: f32,
    /// 湿路低通拐点（Hz）2000..=20000。
    pub hf_hz: f32,
    /// 曲线不对称量 0..=0.5（偶次谐波 = 「暖」的来源）。
    pub asym: f32,
    /// wow 速率（Hz）0.05..=2。
    pub wow_rate_hz: f32,
    /// wow 深度（ms，延迟峰值偏离）0..=8。
    pub wow_depth_ms: f32,
    /// flutter 速率（Hz）2..=15。
    pub flutter_rate_hz: f32,
    /// flutter 深度（ms，延迟峰值偏离）0..=2。
    pub flutter_depth_ms: f32,
    /// 调制种子（整数）0..=16777215：同 seed 同渲染（可复现口径）。
    pub seed: f32,
}

/// 延迟段（`fx.delay`，可选）：整轨插入式延迟（引擎无逐道发送，
/// 因此出厂值一律低混合，只作空间/律动点缀，不作主效果）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumDelayPreset {
    /// 延迟时间（秒）0.01..=2。
    pub time_s: f32,
    /// 反馈量 0..=0.9。
    pub feedback: f32,
    /// 湿电平 0..=0.6。
    pub mix: f32,
    /// 反馈高频阻尼 0..=1。
    pub damp: f32,
    /// 乒乓开关。
    pub ping_pong: bool,
}

/// 瞬态塑形段（`fx.transient` 与逐件链 `chain.transient`，可选）：整组鼓的
/// 起音/尾巴塑形（鼓机缺口 D3 接线；器件本体在 `yeban-dsp` 的
/// `shaping.rs`，参数名是 `TransientField` 的规范拼写）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumTransientPreset {
    /// 起音瞬态强度 −1..=1（正值抬起音、负值压起音）。
    pub attack: f32,
    /// 延音包络强度 −1..=1（正值抬尾巴、负值收尾巴）。
    pub sustain: f32,
}

/// 比特碾碎段（`fx.crusher` 与逐件链 `chain.crusher`，可选）：量化 +
/// 采样保持降采样的老采样器味（鼓机缺口 D4 接线；器件本体在
/// `yeban-dsp` 的 `shaping.rs`，参数名是 `CrusherField` 的规范拼写）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumCrusherPreset {
    /// 量化位数 4..=16。
    pub bits: f32,
    /// 采样保持除数 1..=64。
    pub down: f32,
    /// 抗混叠量 0..=1。
    pub aa: f32,
}

/// 噪声源段（`fx.noise` 与逐件链 `chain.noise`，可选）：连续底噪
/// （白 ↔ 粉红）＋爆裂声的黑胶表面层（鼓机缺口 D5 新器件；器件本体在
/// `yeban-dsp` 的 `noise_source.rs`，参数名是 `NoiseField` 的规范
/// 拼写）。噪声是加性源：整段渲染期间连续存在，不逐击触发；爆裂声是
/// 稀疏瞬态事件，按 `seed` 确定性排布（同种子同输出，Wang 硬规则）。
/// 不写本段 ⇒ 无噪声器件 ⇒ 渲染逐位不变（无隐藏默认噪声）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumNoisePreset {
    /// 连续底噪电平（线性）0..=1。
    pub level: f32,
    /// 噪声颜色 0..=1（0 = 白、1 = 粉红）。
    pub color: f32,
    /// 爆裂声密度（次/秒/声道）0..=240。
    pub crackle_density: f32,
    /// 爆裂声电平 0..=1。
    pub crackle_level: f32,
    /// 随机种子 0..=16777215（器件侧取整使用）。
    pub seed: f32,
}

/// 风格效果层（`fx` 段）：按 Logic 通道条预置的思路给整轨鼓预调的
/// 一套完整处理——三段 EQ + 高通/低通塑形、压缩、输出微调、插入混响，
/// 按风格可选磁带饱和、延迟、瞬态塑形、比特碾碎与噪声源。生成时由 `yeban-app` 的 `drum_gen_ops`
/// 翻译成轨上效果器件（EQ 与压缩合为一台通道条器件，见该函数文档）。
///
/// 字段缺失（旧格式文件无 `fx`）⇒ 生成走干声，与一期行为一致；
/// 出厂 10 风格全部带 `fx`（判据钉住）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumFxPreset {
    /// EQ 低频搁架增益（dB）−18..=18。
    pub eq_low_db: f32,
    /// EQ 中频峰值增益（dB）−18..=18。
    pub eq_mid_db: f32,
    /// EQ 高频搁架增益（dB）−18..=18。
    pub eq_high_db: f32,
    /// EQ 低频转折（Hz）20..=1000。
    pub eq_low_hz: f32,
    /// EQ 中频中心（Hz）100..=8000。
    pub eq_mid_hz: f32,
    /// EQ 中频 Q 0.3..=6。
    pub eq_mid_q: f32,
    /// EQ 高频转折（Hz）1000..=16000。
    pub eq_high_hz: f32,
    /// 高通转折（Hz）0..=500；0 = 不启用。
    pub hpf_hz: f32,
    /// 低通截止（Hz）800..=20000；20000 = 不启用。
    pub cutoff_hz: f32,
    /// 压缩阈值（dBFS）−60..=0。
    pub comp_threshold_db: f32,
    /// 压缩比 1..=20。
    pub comp_ratio: f32,
    /// 压缩软膝（dB）0..=24。
    pub comp_knee_db: f32,
    /// 压缩起振（秒）0.0005..=0.2。
    pub comp_attack_s: f32,
    /// 压缩释放（秒）0.01..=1。
    pub comp_release_s: f32,
    /// 压缩 makeup（dB）−12..=12。
    pub comp_makeup_db: f32,
    /// 通道条输出微调（dB）−12..=12（风格间电平平衡的落点之一）。
    pub output_db: f32,
    /// 混响空间类型（Plate/Room/Hall/Spring）。
    pub reverb_kind: DrumReverbKind,
    /// 混响时间（秒，T60）0.1..=10。
    pub reverb_decay_s: f32,
    /// 混响高频阻尼 0..=1。
    pub reverb_damp: f32,
    /// 混响湿电平 0..=1。
    pub reverb_wet: f32,
    /// 混响预延迟（秒）0..=0.1。
    pub reverb_predelay_s: f32,
    /// 磁带饱和（可选）。
    pub tape: Option<DrumTapePreset>,
    /// 延迟（可选）。
    pub delay: Option<DrumDelayPreset>,
    /// 瞬态塑形（可选）。
    pub transient: Option<DrumTransientPreset>,
    /// 比特碾碎（可选）。
    pub crusher: Option<DrumCrusherPreset>,
    /// 噪声源（可选）。
    pub noise: Option<DrumNoisePreset>,
}

/// EQ 段（`fx.eq` 与逐件链 `chain.eq` 共用同一形状与值域）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumEqPreset {
    /// 低频搁架增益（dB）−18..=18。
    pub low_db: f32,
    /// 中频峰值增益（dB）−18..=18。
    pub mid_db: f32,
    /// 高频搁架增益（dB）−18..=18。
    pub high_db: f32,
    /// 低频转折（Hz）20..=1000。
    pub low_hz: f32,
    /// 中频中心（Hz）100..=8000。
    pub mid_hz: f32,
    /// 中频 Q 0.3..=6。
    pub mid_q: f32,
    /// 高频转折（Hz）1000..=16000。
    pub high_hz: f32,
    /// 高通转折（Hz）0..=500；0 = 不启用。
    pub hpf_hz: f32,
    /// 低通截止（Hz）800..=20000；20000 = 不启用。
    pub cutoff_hz: f32,
}

/// 压缩段（`fx.comp` 与逐件链 `chain.comp` 共用同一形状与值域）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumCompPreset {
    /// 阈值（dBFS）−60..=0。
    pub threshold_db: f32,
    /// 压缩比 1..=20。
    pub ratio: f32,
    /// 软膝（dB）0..=24。
    pub knee_db: f32,
    /// 起振（秒）0.0005..=0.2。
    pub attack_s: f32,
    /// 释放（秒）0.01..=1。
    pub release_s: f32,
    /// makeup（dB）−12..=12。
    pub makeup_db: f32,
}

/// 混响空间类型（预置侧表示）：与 `yeban-dsp` 的 `ReverbKind` 同一套四档，
/// 生成路径经 [`DrumReverbKind::device_value`] 写出 `reverb_kind` 器件参数。
///
/// ⚠ 索引值是两个 crate 之间的镜像契约（theory 不依赖 dsp，无法直接引用）：
/// dsp 侧 `ReverbKind` 的档位序改动必须同步改这里；两端一致由 render 侧
/// 端到端判据（按类型生成预置、读回器件参数）钉住。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrumReverbKind {
    /// 房间（现行 Freeverb 标定本身）。
    Room,
    /// 板式（密早期反射、高频保持最强）。
    Plate,
    /// 大厅（长延迟线、早期反射最稀）。
    Hall,
    /// 弹簧（参数组近似：强色散 allpass 链，非物理弹簧模型）。
    Spring,
}

impl DrumReverbKind {
    /// JSON 名 → 类型。未知名字是内容错误，解析期响亮失败（默认不能静默）。
    fn from_name(name: &str, field: &str) -> Result<Self, DrumPresetError> {
        match name {
            "room" => Ok(Self::Room),
            "plate" => Ok(Self::Plate),
            "hall" => Ok(Self::Hall),
            "spring" => Ok(Self::Spring),
            _ => Err(field_err(
                field,
                format!("unknown reverb kind {name:?} (expected room/plate/hall/spring)"),
            )),
        }
    }

    /// 规范名（JSON 与回读共用）。
    pub fn name(self) -> &'static str {
        match self {
            Self::Room => "room",
            Self::Plate => "plate",
            Self::Hall => "hall",
            Self::Spring => "spring",
        }
    }

    /// 器件参数值（镜像 `yeban_dsp::reverb::ReverbKind` 的档位序）。
    pub fn device_value(self) -> f32 {
        match self {
            Self::Room => 0.0,
            Self::Plate => 1.0,
            Self::Hall => 2.0,
            Self::Spring => 3.0,
        }
    }
}

/// 混响段（`fx.reverb` 与逐件链 `chain.reverb` 共用同一形状与值域）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumReverbPreset {
    /// 空间类型。
    pub kind: DrumReverbKind,
    /// 混响时间（秒，T60）0.1..=10。
    pub decay_s: f32,
    /// 高频阻尼 0..=1。
    pub damp: f32,
    /// 湿电平 0..=1。
    pub wet: f32,
    /// 预延迟（秒）0..=0.1。
    pub predelay_s: f32,
}

/// 逐件链的侧链声明（`lane_chains[].chain.sidechain`）：本组压缩的
/// key 源 = `key_keys` 这些键命中的声部信号。逐件＋侧链期 4 起，
/// 生成路径把声明落成同轨 lane 级侧链边（`Op::ConnectRouting`，
/// render 侧真求值）；出厂预置的侧链值期 5 才写（设计裁决）。
#[derive(Debug, Clone, PartialEq)]
pub struct DrumLaneSidechainPreset {
    /// key 源键组（严格升序、非空；必须是本预置鼓道键的子集）。
    pub key_keys: Vec<u8>,
}

/// 一条逐件（per-lane）处理链预置（`lane_chains[]`）：按 GM 键组寻址，
/// 组单位与 SFZ choke 组对齐。`chain` 内各段全部可选（出现则整段字段
/// 必填，值域与 `fx` 同段完全一致、共用同一套校验）；整段为空
/// （什么处理都不带）是内容错误，解析期拒绝。逐件链参数 v1 不接
/// 自动化。
#[derive(Debug, Clone, PartialEq)]
pub struct DrumLaneChainPreset {
    /// 组名（与生成后 `LaneChainDef::name` 一致，如 `hats`）。
    pub name: String,
    /// 本组覆盖的 GM 键（严格升序、非空；组间不重叠、按首键升序；
    /// 必须是本预置鼓道键的子集）。
    pub keys: Vec<u8>,
    /// EQ 段（可选）。
    pub eq: Option<DrumEqPreset>,
    /// 压缩段（可选）。
    pub comp: Option<DrumCompPreset>,
    /// 混响段（可选）。
    pub reverb: Option<DrumReverbPreset>,
    /// 磁带段（可选）。
    pub tape: Option<DrumTapePreset>,
    /// 延迟段（可选）。
    pub delay: Option<DrumDelayPreset>,
    /// 瞬态塑形段（可选）。
    pub transient: Option<DrumTransientPreset>,
    /// 比特碾碎段（可选）。
    pub crusher: Option<DrumCrusherPreset>,
    /// 噪声源段（可选）。
    pub noise: Option<DrumNoisePreset>,
    /// 本组声相覆盖（可选）−1..=1。
    pub pan: Option<f32>,
    /// 本组增益覆盖（可选，dB）−60..=24：逐件相对电平的独立落点，
    /// 生成落成 `LaneChainDef::gain_db`，渲染在组链与组声相之后施加。
    /// 只写增益（无任何处理段）的组同样合法——原声系逐件电平微调
    /// 正是这种组。
    pub gain_db: Option<f32>,
    /// 侧链声明（可选；期 4 起生成落成 lane 级侧链边，出厂值期 5 才写）。
    pub sidechain: Option<DrumLaneSidechainPreset>,
}

/// 鼓总线胶水压缩段（`bus.glue`）：把整组鼓粘成一个整体的慢起振压缩。
/// 数值口径照 Wang 的 v2 定值：比 2:1–4:1、起振 30 ms 让瞬态先过、
/// 增益衰减目标 1–2 dB（由阈值与 makeup 共同落）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumBusGluePreset {
    /// 压缩阈值（dBFS）−60..=0。
    pub threshold_db: f32,
    /// 压缩比 1..=20（出厂定值 2..=4）。
    pub ratio: f32,
    /// 压缩软膝（dB）0..=24。
    pub knee_db: f32,
    /// 压缩起振（秒）0.0005..=0.2（出厂定值 0.03）。
    pub attack_s: f32,
    /// 压缩释放（秒）0.01..=1。
    pub release_s: f32,
    /// 压缩 makeup（dB）−12..=12。
    pub makeup_db: f32,
}

/// 鼓总线末级软削波段（`bus.clipper`）：链上最后一台，削个别瞬态尖峰、
/// 防削波失真并抬整体响度。`ceiling_db` 是削波渐近线，出厂定值 −0.3
/// 量级，保证渲染真峰值留 −0.1 dBTP 以上的余量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumBusClipperPreset {
    /// 输入驱动（dB）0..=24。
    pub drive_db: f32,
    /// 削波渐近线（dBFS）−6..=0。
    pub ceiling_db: f32,
    /// 软膝宽度（dB）0..=6。
    pub knee_db: f32,
    /// 干湿比 0..=1。
    pub mix: f32,
    /// 过采样倍数（1/2/4/8）。
    pub oversample: u32,
}

/// 鼓总线段（`bus`）：生成时新建一条鼓总线轨（`AuxReturn`），鼓轨先
/// 进总线、总线再进主总线；总线链 = 胶水压缩 → 末级 Soft Clipper。
/// 字段缺失（旧格式文件无 `bus`）⇒ 生成不建总线，鼓轨直连主总线，
/// 与一期行为一致；出厂 14 风格全部带 `bus`（判据钉住）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumBusPreset {
    /// 胶水压缩。
    pub glue: DrumBusGluePreset,
    /// 末级软削波。
    pub clipper: DrumBusClipperPreset,
}

/// 自动化曲线形状（预置侧表示）：与模型 `yeban_model::CurveType` 同一套
/// 四档（一一镜像——theory 不依赖 model，无法直接引用，与
/// [`DrumReverbKind`] 的镜像契约同型）。生成路径把曲线**逐点**写进
/// `AutomationPoint::curve`（模型侧该字段必需、无缺省）：本格式的段级
/// `curve` 只是书写便利，落点处展开到每一点，不在模型侧留缺省。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrumAutomationCurve {
    /// 线性插值。
    Linear,
    /// 指数插值。
    Exponential,
    /// 对数插值。
    Logarithmic,
    /// S 型（缓入缓出）插值。
    SCurve,
}

impl DrumAutomationCurve {
    /// JSON 名 → 曲线。未知名字是内容错误，解析期响亮失败（默认不能静默）。
    fn from_name(name: &str, field: &str) -> Result<Self, DrumPresetError> {
        match name {
            "linear" => Ok(Self::Linear),
            "exponential" => Ok(Self::Exponential),
            "logarithmic" => Ok(Self::Logarithmic),
            "s_curve" => Ok(Self::SCurve),
            _ => Err(field_err(
                field,
                format!(
                    "unknown automation curve {name:?} (expected linear/exponential/logarithmic/s_curve)"
                ),
            )),
        }
    }

    /// 规范名（JSON 与回读共用）。
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Exponential => "exponential",
            Self::Logarithmic => "logarithmic",
            Self::SCurve => "s_curve",
        }
    }
}

/// 自动化目标（预置侧表示）：与模型 `AutomationTarget` 对齐的三种寻址。
///
/// - `track_volume` / `track_pan`：鼓轨自身的音量 / 声相（Auto-Pan 的
///   落点就是声相泳道）；
/// - `<family>.<param>`：鼓轨效果层里某族器件的一个参数，`family` 与
///   [`DrumFxDeviceSpec::family`] 同一词表（`strip` / `tape` / `reverb` /
///   `delay` / `transient` / `crusher` / `noise`），`param` 是
///   `yeban-dsp` `device_param` 的规范拼写（如 `reverb_wet`）。
///
/// 逐件链（`lane_chains`）与鼓总线（`bus`）的器件**不在**本词表内：
/// 逐件链参数 v1 不接自动化（见 [`DrumLaneChainPreset`]）；总线轨只在
/// 新建轨路径存在，寻址不对称。写这两类目标 ⇒ 未知目标，解析期拒绝。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrumAutomationTarget {
    /// 鼓轨音量（dB）。
    TrackVolume,
    /// 鼓轨声相（双极 −1..=1）。
    TrackPan,
    /// 效果器件参数（族 + 参数名）。
    DeviceParam {
        /// 效果族（`DrumFxDeviceSpec::family` 词表）。
        family: String,
        /// 参数规范名（`device_param` 拼写）。
        param: String,
    },
}

impl DrumAutomationTarget {
    /// 与 JSON 拼写一致的目标名（报错、计划文本与回读共用）。
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::TrackVolume => "track_volume".to_owned(),
            Self::TrackPan => "track_pan".to_owned(),
            Self::DeviceParam { family, param } => format!("{family}.{param}"),
        }
    }

    /// 本目标的解析期取值域：音量 / 声相取模型固有域（镜像
    /// `yeban_model` 的 `NOMINAL_GAIN_MIN_DB` / `NOMINAL_GAIN_MAX_DB`
    /// −60..=12 dB 与双极 −1..=1——theory 不依赖 model，镜像值改动
    /// 须两端同步）；器件参数取 [`drum_fx_param_domain`] 的格式参数表。
    fn value_domain(&self) -> (f32, f32) {
        match self {
            Self::TrackVolume => (-60.0, 12.0),
            Self::TrackPan => (-1.0, 1.0),
            Self::DeviceParam { family, param } => drum_fx_param_domain(family, param)
                .expect("device param targets are only built after a table hit"),
        }
    }
}

/// 一个自动化点（预置侧表示）：`tick` 是 pattern 内绝对 tick
/// （0 起，上限 = pattern 全长 `bars * TICKS_PER_BAR`，含末点）；
/// 生成路径按片段摆放起点平移后写进模型的 `AutomationPoint`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumAutomationPoint {
    /// 时间位置（tick），段内严格升序（解析期钉死）。
    pub tick: u64,
    /// 参数值（目标值域内、有限；解析期钉死）。
    pub value: f32,
}

/// 一段自动化（`automation[]` 一条 = 模型里一条自动化泳道）：一个
/// 目标 + 一列点 + 一条段级曲线（落点时逐点展开）。同一预置内目标
/// 不许重复（模型里一个目标只有一条泳道，第二段无处落，解析期拒绝）。
#[derive(Debug, Clone, PartialEq)]
pub struct DrumAutomationPreset {
    /// 目标。
    pub target: DrumAutomationTarget,
    /// 段内各点的曲线形状（到下一点的形状，逐点展开写进模型）。
    pub curve: DrumAutomationCurve,
    /// 点列（非空、tick 严格升序；解析期钉死）。
    pub points: Vec<DrumAutomationPoint>,
}

/// 自动化在生成后鼓轨上的落点（生成路径的换算结果）：轨级目标直接
/// 成形；器件参数已换算成（器件槽位， 参数下标）——与模型
/// `AutomationTarget::DeviceParam` 的寻址逐字对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrumAutomationLanding {
    /// 鼓轨音量。
    TrackVolume,
    /// 鼓轨声相。
    TrackPan,
    /// 器件参数：鼓轨器件表的槽位与该器件参数表的下标。
    DeviceParam {
        /// 器件槽位（鼓轨 `devices` 下标）。
        slot_index: usize,
        /// 参数下标（该器件 `params` 下标）。
        param_index: usize,
    },
}

impl DrumAutomationPreset {
    /// 在生成后的鼓轨器件表上换算落点。`device_param_names` 是鼓轨
    /// `devices` 按槽位序的参数名表（生成路径按其最终器件布局构造，
    /// 含既有器件与本次装入的器件）。器件参数按名取**第一台**含该
    /// 参数的器件（与插入链投影「按族取第一台命中器件」同一口径）；
    /// 找不到 ⇒ `None`，生成路径据此响亮拒绝（不许静默丢段）。
    #[must_use]
    pub fn landing(&self, device_param_names: &[Vec<String>]) -> Option<DrumAutomationLanding> {
        match &self.target {
            DrumAutomationTarget::TrackVolume => Some(DrumAutomationLanding::TrackVolume),
            DrumAutomationTarget::TrackPan => Some(DrumAutomationLanding::TrackPan),
            DrumAutomationTarget::DeviceParam { param, .. } => {
                for (slot_index, names) in device_param_names.iter().enumerate() {
                    if let Some(param_index) = names.iter().position(|name| name == param) {
                        return Some(DrumAutomationLanding::DeviceParam {
                            slot_index,
                            param_index,
                        });
                    }
                }
                None
            }
        }
    }
}

/// 效果族参数的自动化取值域表：（族， 参数名， 下限， 上限）。
///
/// 参数名全集 = 本格式 `fx` 配方能写出的名字（[`drum_fx_device_specs`]
/// 各族参数表的并集，含通道条的条件参数 `hpf_hz` / `cutoff_hz` /
/// `filter_enabled` / `output_gain_db`）；值域 = 对应预置字段的解析
/// 值域（与 `parse_eq` / `parse_comp` 等的 `get_f32_ranged` 钉值逐条
/// 一致——同一参数只许有一套合法区间，改一处必须同步另一处）。
/// 表外名字 = 未知参数，解析期响亮拒绝。
const DRUM_FX_PARAM_DOMAINS: &[(&str, &str, f32, f32)] = &[
    // 通道条（strip）：EQ 段。
    ("strip", "eq_enabled", 0.0, 1.0),
    ("strip", "eq_low_gain", -18.0, 18.0),
    ("strip", "eq_low_freq", 20.0, 1000.0),
    ("strip", "eq_mid_gain", -18.0, 18.0),
    ("strip", "eq_mid_freq", 100.0, 8000.0),
    ("strip", "eq_mid_q", 0.3, 6.0),
    ("strip", "eq_high_gain", -18.0, 18.0),
    ("strip", "eq_high_freq", 1000.0, 16000.0),
    ("strip", "hpf_hz", 0.0, 500.0),
    ("strip", "filter_enabled", 0.0, 1.0),
    ("strip", "cutoff_hz", 800.0, 20000.0),
    // 通道条（strip）：压缩段 + 输出微调。
    ("strip", "compressor_enabled", 0.0, 1.0),
    ("strip", "threshold_db", -60.0, 0.0),
    ("strip", "ratio", 1.0, 20.0),
    ("strip", "knee_db", 0.0, 24.0),
    ("strip", "attack_s", 0.0005, 0.2),
    ("strip", "release_s", 0.01, 1.0),
    ("strip", "makeup_db", -12.0, 12.0),
    ("strip", "output_gain_db", -12.0, 12.0),
    // 磁带（tape）。后五条为 P7 的 wow/flutter 段：`tape_params` 早已
    // 写出这五个名字，值域表此前漏登记，自动化写到即被判未知参数。
    ("tape", "tape_drive", 0.0, 1.0),
    ("tape", "tape_mix", 0.0, 1.0),
    ("tape", "tape_hf_hz", 2000.0, 20000.0),
    ("tape", "tape_asym", 0.0, 0.5),
    ("tape", "tape_wow_rate_hz", 0.05, 2.0),
    ("tape", "tape_wow_depth_ms", 0.0, 8.0),
    ("tape", "tape_flutter_rate_hz", 2.0, 15.0),
    ("tape", "tape_flutter_depth_ms", 0.0, 2.0),
    ("tape", "tape_seed", 0.0, 16777215.0),
    // 混响（reverb）。`reverb_kind` 是离散档位（0..=3），列入表内是
    // 为了让拼写合法性有单一判据；把它写成连续自动化没有音乐意义，
    // 但格式层不替作者做审美裁决，只钉值域。
    ("reverb", "reverb_kind", 0.0, 3.0),
    ("reverb", "reverb_decay_s", 0.1, 10.0),
    ("reverb", "reverb_damp", 0.0, 1.0),
    ("reverb", "reverb_wet", 0.0, 1.0),
    ("reverb", "reverb_predelay", 0.0, 0.1),
    // 延迟（delay）。
    ("delay", "delay_time_s", 0.01, 2.0),
    ("delay", "delay_feedback", 0.0, 0.9),
    ("delay", "delay_mix", 0.0, 0.6),
    ("delay", "delay_damp", 0.0, 1.0),
    ("delay", "delay_ping_pong", 0.0, 1.0),
    // 瞬态塑形（transient）。
    ("transient", "transient_attack", -1.0, 1.0),
    ("transient", "transient_sustain", -1.0, 1.0),
    // 比特碾碎（crusher）。
    ("crusher", "crush_bits", 4.0, 16.0),
    ("crusher", "crush_down", 1.0, 64.0),
    ("crusher", "crush_aa", 0.0, 1.0),
    // 噪声源（noise）。
    ("noise", "noise_level", 0.0, 1.0),
    ("noise", "noise_color", 0.0, 1.0),
    ("noise", "noise_crackle_density", 0.0, 240.0),
    ("noise", "noise_crackle_level", 0.0, 1.0),
    ("noise", "noise_seed", 0.0, 16777215.0),
];

/// 查一个效果族参数的自动化取值域；名字不在格式参数表内 ⇒ `None`
/// （未知参数，调用方响亮拒绝）。生成路径与解析校验共用这一份表。
#[must_use]
pub fn drum_fx_param_domain(family: &str, param: &str) -> Option<(f32, f32)> {
    DRUM_FX_PARAM_DOMAINS
        .iter()
        .find(|(fam, name, _, _)| *fam == family && *name == param)
        .map(|(_, _, min, max)| (*min, *max))
}

/// 族名是否在自动化词表内（`DrumFxDeviceSpec::family` 的七族）。
fn drum_fx_family_known(family: &str) -> bool {
    DRUM_FX_PARAM_DOMAINS
        .iter()
        .any(|(fam, _, _, _)| *fam == family)
}

/// 一个风格预置。（不派生 `Eq`：`fx` 段含 `f32`。）
#[derive(Debug, Clone, PartialEq)]
pub struct DrumStylePreset {
    /// 稳定 id（ASCII 小写，如 `trap`）。
    pub id: String,
    /// 英文名。
    pub name: String,
    /// 中文名（可带英文后缀，如 `陷阱 Trap`）。
    pub name_zh: String,
    /// groove 源流派 id（来源留痕；构造型风格写构造说明的标记）。
    pub source_genre_id: String,
    /// 调优项清单（来源留痕的一部分，逐条短句）。
    pub tuning: Vec<String>,
    /// 鼓组族。
    pub kit_family: KitFamily,
    /// 缺省 BPM。
    pub bpm_default: u16,
    /// BPM 下限。
    pub bpm_min: u16,
    /// BPM 上限。
    pub bpm_max: u16,
    /// 缺省 swing（千分比，500 = 平直，合法域 500..=1000）。
    pub swing_permille: u16,
    /// pattern 小节数（出厂恒为 2）。
    pub bars: u32,
    /// 鼓道表（顺序即生成时的道序；同 tick 内按此序输出）。
    pub lanes: Vec<DrumLanePreset>,
    /// fill（1 小节）的鼓道表；`None` = 该风格一期无 fill。
    pub fill_lanes: Option<Vec<DrumLanePreset>>,
    /// 逐件链预置（`lane_chains` 段）；空表 = 文件未带逐件链段，
    /// 生成不写逐件链（与本字段引入前行为一致）。
    pub lane_chains: Vec<DrumLaneChainPreset>,
    /// 效果层预置（`fx` 段）；`None` = 文件未带效果段，生成走干声。
    pub fx: Option<DrumFxPreset>,
    /// 鼓总线预置（`bus` 段）；`None` = 文件未带总线段，生成不建总线。
    pub bus: Option<DrumBusPreset>,
    /// 自动化段（`automation` 段）；空表 = 文件未带自动化段，
    /// 生成不写自动化泳道（与本字段引入前行为逐位一致）。
    pub automation: Vec<DrumAutomationPreset>,
    /// 原声族的 SFZ 鼓组路径覆盖（`sfz_path` 字段，仓库相对路径）；
    /// `None` = 用默认基础组 [`ACOUSTIC_KIT_SFZ_PATH`]。电子族忽略。
    pub sfz_path: Option<String>,
}

impl DrumStylePreset {
    /// 从 JSON 文本解析一份预置（格式见模块文档）。
    ///
    /// # Errors
    ///
    /// 语法错误返回 [`DrumPresetError::Parse`]；字段缺失/形状/值域错误
    /// 返回 [`DrumPresetError::Field`] 并点名字段。
    pub fn from_json(text: &str) -> Result<Self, DrumPresetError> {
        let root = parse_json(text)?;
        let obj = root
            .as_object()
            .ok_or_else(|| field_err("<root>", "top level must be an object"))?;
        let format = get_str(obj, "format")?;
        if format != DRUM_STYLE_FORMAT {
            return Err(field_err(
                "format",
                format!("expected `{DRUM_STYLE_FORMAT}`, got `{format}`"),
            ));
        }
        let version = get_u64(obj, "version")?;
        if version != DRUM_STYLE_FORMAT_VERSION {
            return Err(field_err(
                "version",
                format!("expected {DRUM_STYLE_FORMAT_VERSION}, got {version}"),
            ));
        }
        let id = get_str(obj, "id")?.to_owned();
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(field_err(
                "id",
                format!("id must be non-empty ASCII lowercase/digit/dash, got `{id}`"),
            ));
        }
        let name = get_str(obj, "name")?.to_owned();
        let name_zh = get_str(obj, "name_zh")?.to_owned();
        let source = get_obj(obj, "source")?;
        let source_genre_id = get_str(source, "groove_genre_id")?.to_owned();
        let tuning = get_str_list(source, "tuning")?;
        let kit_family = match get_str(obj, "kit_family")? {
            "acoustic" => KitFamily::Acoustic,
            "electronic" => KitFamily::Electronic,
            other => {
                return Err(field_err(
                    "kit_family",
                    format!("expected `acoustic` or `electronic`, got `{other}`"),
                ));
            }
        };
        let bpm = get_obj(obj, "bpm")?;
        let bpm_default = get_u16(bpm, "default")?;
        let bpm_min = get_u16(bpm, "min")?;
        let bpm_max = get_u16(bpm, "max")?;
        if bpm_min == 0 || bpm_min > bpm_default || bpm_default > bpm_max {
            return Err(field_err(
                "bpm",
                format!(
                    "expected 0 < min <= default <= max, got min={bpm_min} default={bpm_default} max={bpm_max}"
                ),
            ));
        }
        let swing_permille = get_u16(obj, "swing_permille")?;
        crate::swing::validate_swing_permille(swing_permille)
            .map_err(|e| field_err("swing_permille", e.to_string()))?;
        let bars = u32::try_from(get_u64(obj, "bars")?)
            .map_err(|_| field_err("bars", "bars out of range"))?;
        if bars == 0 || bars > 8 {
            return Err(field_err("bars", format!("bars must be 1..=8, got {bars}")));
        }
        let lanes = parse_lanes(obj, "lanes", bars as usize * STEPS_PER_BAR)?;
        if lanes.is_empty() {
            return Err(field_err("lanes", "at least one lane is required"));
        }
        let fill_lanes = match get_opt(obj, "fill") {
            None => None,
            Some(fill) => {
                let fill_obj = fill
                    .as_object()
                    .ok_or_else(|| field_err("fill", "fill must be an object"))?;
                Some(parse_lanes(fill_obj, "lanes", STEPS_PER_BAR)?)
            }
        };
        let sfz_raw = match get_opt(obj, "sfz_path") {
            Some(Json::Str(s)) => Some(s.as_str()),
            _ => None,
        };
        let lane_chains =
            parse_lane_chains(obj, &lanes, fill_lanes.as_deref(), kit_family, sfz_raw)?;
        let fx = match get_opt(obj, "fx") {
            None => None,
            Some(fx_json) => {
                let fx_obj = fx_json
                    .as_object()
                    .ok_or_else(|| field_err("fx", "fx must be an object"))?;
                Some(parse_fx(fx_obj)?)
            }
        };
        let bus = match get_opt(obj, "bus") {
            None => None,
            Some(bus_json) => {
                let bus_obj = bus_json
                    .as_object()
                    .ok_or_else(|| field_err("bus", "bus must be an object"))?;
                Some(parse_bus(bus_obj)?)
            }
        };
        // 自动化段在 fx 之后解析：器件目标要对 fx 的在场形状校验
        // （无 fx 段 / 无对应子段 ⇒ 目标无处落，解析期响亮拒绝）。
        let automation = parse_automation(obj, fx.as_ref(), bars)?;
        let sfz_path = match get_opt(obj, "sfz_path") {
            None => None,
            Some(raw) => {
                let path = match raw {
                    Json::Str(s) => s.clone(),
                    _ => return Err(field_err("sfz_path", "expected a string")),
                };
                if path.is_empty() || !path.ends_with(".sfz") || path.contains("..") {
                    return Err(field_err(
                        "sfz_path",
                        format!("expected a repo-relative .sfz path, got `{path}`"),
                    ));
                }
                Some(path)
            }
        };
        Ok(Self {
            id,
            name,
            name_zh,
            source_genre_id,
            tuning,
            kit_family,
            bpm_default,
            bpm_min,
            bpm_max,
            swing_permille,
            bars,
            lanes,
            fill_lanes,
            lane_chains,
            fx,
            bus,
            automation,
            sfz_path,
        })
    }

    /// 按 GM 键找鼓道（生成与校验用）。
    #[must_use]
    pub fn lane_by_key(&self, gm_key: u8) -> Option<&DrumLanePreset> {
        self.lanes.iter().find(|lane| lane.gm_key == gm_key)
    }
}

fn parse_lanes(
    obj: &[(String, Json)],
    key: &str,
    expected_steps: usize,
) -> Result<Vec<DrumLanePreset>, DrumPresetError> {
    let arr = get_arr(obj, key)?;
    let mut lanes = Vec::with_capacity(arr.len());
    // 全表发声键登记（主键与叠层键分开记）：叠层键不得与任何主键或
    // 叠层键撞键，主键不得与先行叠层键撞键——生成层「同 tick 同键
    // 至多一项」的不变量靠这条在解析期钉死，撞键响亮拒绝，不许
    // 生成层去重猜测。主键撞先行主键是格式既有行为，本函数不新拒。
    let mut primary_keys: Vec<u8> = Vec::new();
    let mut layer_keys: Vec<u8> = Vec::new();
    for (idx, item) in arr.iter().enumerate() {
        let lane_obj = item
            .as_object()
            .ok_or_else(|| field_err(key, format!("lane {idx} must be an object")))?;
        let role = get_str(lane_obj, "role")?.to_owned();
        let gm_key = u8::try_from(get_u64(lane_obj, "gm_key")?)
            .map_err(|_| field_err("gm_key", format!("lane {idx} gm_key out of range")))?;
        if layer_keys.contains(&gm_key) {
            return Err(field_err(
                "layers",
                format!(
                    "lane {idx} ({role}) gm_key {gm_key} collides with an earlier lane's layer key"
                ),
            ));
        }
        let layers = parse_lane_layers(lane_obj, idx, &role)?;
        for layer in &layers {
            if layer.gm_key == gm_key
                || primary_keys.contains(&layer.gm_key)
                || layer_keys.contains(&layer.gm_key)
            {
                return Err(field_err(
                    "layers",
                    format!(
                        "lane {idx} ({role}) layer key {} is already used in this lane table",
                        layer.gm_key
                    ),
                ));
            }
            layer_keys.push(layer.gm_key);
        }
        primary_keys.push(gm_key);
        let raw_steps = get_arr(lane_obj, "steps")?;
        if raw_steps.len() != expected_steps {
            return Err(field_err(
                "steps",
                format!(
                    "lane {idx} ({role}) has {} steps, expected {expected_steps}",
                    raw_steps.len()
                ),
            ));
        }
        let mut steps = Vec::with_capacity(expected_steps);
        for (step_idx, raw) in raw_steps.iter().enumerate() {
            let triple = raw.as_array().ok_or_else(|| {
                field_err(
                    "steps",
                    format!("lane {idx} step {step_idx} must be [velocity, ratchet, tier]"),
                )
            })?;
            if triple.len() != 3 {
                return Err(field_err(
                    "steps",
                    format!("lane {idx} step {step_idx} must have exactly 3 numbers"),
                ));
            }
            let velocity = as_u8(&triple[0], "velocity")?;
            let ratchet = as_u8(&triple[1], "ratchet")?;
            let tier = as_u8(&triple[2], "tier")?;
            if velocity > 127 {
                return Err(field_err(
                    "steps",
                    format!("lane {idx} step {step_idx} velocity {velocity} > 127"),
                ));
            }
            if !(1..=MAX_RATCHET).contains(&ratchet) {
                return Err(field_err(
                    "steps",
                    format!(
                        "lane {idx} step {step_idx} ratchet {ratchet} out of 1..={MAX_RATCHET}"
                    ),
                ));
            }
            if tier > 2 {
                return Err(field_err(
                    "steps",
                    format!("lane {idx} step {step_idx} tier {tier} > 2"),
                ));
            }
            steps.push(DrumStep {
                velocity,
                ratchet,
                tier,
            });
        }
        lanes.push(DrumLanePreset {
            role,
            gm_key,
            layers,
            steps,
        });
    }
    Ok(lanes)
}

/// 解析一条道的 `layers` 段（缺省 = 无叠层）。形状/值域非法一律
/// 响亮拒绝：空数组、键越界、电平越界、力度窗越界或上下限颠倒。
fn parse_lane_layers(
    lane_obj: &[(String, Json)],
    lane_idx: usize,
    role: &str,
) -> Result<Vec<DrumLaneLayer>, DrumPresetError> {
    let Some(raw) = get_opt(lane_obj, "layers") else {
        return Ok(Vec::new());
    };
    let arr = match raw {
        Json::Arr(items) => items,
        _ => {
            return Err(field_err(
                "layers",
                format!("lane {lane_idx} ({role}) layers must be an array"),
            ));
        }
    };
    if arr.is_empty() {
        return Err(field_err(
            "layers",
            format!(
                "lane {lane_idx} ({role}) layers is empty; omit the field for a single-key lane"
            ),
        ));
    }
    let mut layers = Vec::with_capacity(arr.len());
    for (layer_idx, item) in arr.iter().enumerate() {
        let layer_obj = item.as_object().ok_or_else(|| {
            field_err(
                "layers",
                format!("lane {lane_idx} ({role}) layer {layer_idx} must be an object"),
            )
        })?;
        let gm_key = u8::try_from(get_u64(layer_obj, "gm_key")?).map_err(|_| {
            field_err(
                "layers",
                format!("lane {lane_idx} ({role}) layer {layer_idx} gm_key out of range"),
            )
        })?;
        let level_db = match get_opt(layer_obj, "level_db") {
            None => 0.0,
            Some(Json::Num(n)) => {
                if !n.is_finite() || !(-48.0..=12.0).contains(n) {
                    return Err(field_err(
                        "layers",
                        format!(
                            "lane {lane_idx} ({role}) layer {layer_idx} level_db {n} out of -48..=12"
                        ),
                    ));
                }
                *n as f32
            }
            Some(_) => {
                return Err(field_err(
                    "layers",
                    format!("lane {lane_idx} ({role}) layer {layer_idx} level_db must be a number"),
                ));
            }
        };
        let vel_min = parse_layer_vel_bound(layer_obj, "vel_min", 1, lane_idx, role, layer_idx)?;
        let vel_max = parse_layer_vel_bound(layer_obj, "vel_max", 127, lane_idx, role, layer_idx)?;
        if vel_min > vel_max {
            return Err(field_err(
                "layers",
                format!(
                    "lane {lane_idx} ({role}) layer {layer_idx} vel_min {vel_min} > vel_max {vel_max}"
                ),
            ));
        }
        layers.push(DrumLaneLayer {
            gm_key,
            level_db,
            vel_min,
            vel_max,
        });
    }
    Ok(layers)
}

/// 读一个叠层的力度窗界（缺省 `default`）；在场则必须是 1..=127 的
/// 整数（0 是休止、>127 不是 MIDI 力度，一律响亮拒绝）。
fn parse_layer_vel_bound(
    layer_obj: &[(String, Json)],
    key: &str,
    default: u8,
    lane_idx: usize,
    role: &str,
    layer_idx: usize,
) -> Result<u8, DrumPresetError> {
    let Some(raw) = get_opt(layer_obj, key) else {
        return Ok(default);
    };
    let value = as_u8(raw, "layers")?;
    if !(1..=127).contains(&value) {
        return Err(field_err(
            "layers",
            format!("lane {lane_idx} ({role}) layer {layer_idx} {key} {value} out of 1..=127"),
        ));
    }
    Ok(value)
}

/// 读一个浮点字段并校验值域（值域与 `yeban-dsp` 对应器件的钳制域对齐：
/// 预置写出界值是内容错误，必须在解析期响亮失败，不能等到渲染期被静默钳）。
fn get_f32_ranged(
    obj: &[(String, Json)],
    key: &str,
    field: &str,
    min: f32,
    max: f32,
) -> Result<f32, DrumPresetError> {
    let raw = match get(obj, key)? {
        Json::Num(n) => *n,
        _ => return Err(field_err(field, "expected a number")),
    };
    if !raw.is_finite() || raw < f64::from(min) || raw > f64::from(max) {
        return Err(field_err(
            field,
            format!("value {raw} out of {min}..={max}"),
        ));
    }
    #[allow(clippy::cast_possible_truncation)] // 已校验落在 f32 可精确承载的值域内
    Ok(raw as f32)
}

/// 解析 EQ 段（`{base}.eq`）：整段字段必填，值域与通道条 EQ 钳制域
/// 对齐。`fx` 与逐件链 `chain` 共用本实现——同一形状只许有一套校验。
fn parse_eq(obj: &[(String, Json)], base: &str) -> Result<DrumEqPreset, DrumPresetError> {
    Ok(DrumEqPreset {
        low_db: get_f32_ranged(obj, "low_db", &format!("{base}.eq.low_db"), -18.0, 18.0)?,
        mid_db: get_f32_ranged(obj, "mid_db", &format!("{base}.eq.mid_db"), -18.0, 18.0)?,
        high_db: get_f32_ranged(obj, "high_db", &format!("{base}.eq.high_db"), -18.0, 18.0)?,
        low_hz: get_f32_ranged(obj, "low_hz", &format!("{base}.eq.low_hz"), 20.0, 1000.0)?,
        mid_hz: get_f32_ranged(obj, "mid_hz", &format!("{base}.eq.mid_hz"), 100.0, 8000.0)?,
        mid_q: get_f32_ranged(obj, "mid_q", &format!("{base}.eq.mid_q"), 0.3, 6.0)?,
        high_hz: get_f32_ranged(
            obj,
            "high_hz",
            &format!("{base}.eq.high_hz"),
            1000.0,
            16000.0,
        )?,
        hpf_hz: get_f32_ranged(obj, "hpf_hz", &format!("{base}.eq.hpf_hz"), 0.0, 500.0)?,
        cutoff_hz: get_f32_ranged(
            obj,
            "cutoff_hz",
            &format!("{base}.eq.cutoff_hz"),
            800.0,
            20000.0,
        )?,
    })
}

/// 解析压缩段（`{base}.comp`）：整段字段必填。`fx` 与逐件链共用。
fn parse_comp(obj: &[(String, Json)], base: &str) -> Result<DrumCompPreset, DrumPresetError> {
    Ok(DrumCompPreset {
        threshold_db: get_f32_ranged(
            obj,
            "threshold_db",
            &format!("{base}.comp.threshold_db"),
            -60.0,
            0.0,
        )?,
        ratio: get_f32_ranged(obj, "ratio", &format!("{base}.comp.ratio"), 1.0, 20.0)?,
        knee_db: get_f32_ranged(obj, "knee_db", &format!("{base}.comp.knee_db"), 0.0, 24.0)?,
        attack_s: get_f32_ranged(
            obj,
            "attack_s",
            &format!("{base}.comp.attack_s"),
            0.0005,
            0.2,
        )?,
        release_s: get_f32_ranged(
            obj,
            "release_s",
            &format!("{base}.comp.release_s"),
            0.01,
            1.0,
        )?,
        makeup_db: get_f32_ranged(
            obj,
            "makeup_db",
            &format!("{base}.comp.makeup_db"),
            -12.0,
            12.0,
        )?,
    })
}

/// 解析混响段（`{base}.reverb`）：`damp`／`wet`／`predelay_s` 整段必填；`kind` 与
/// `decay_s` 可缺——旧式段（只有 `size`、无 `kind`/`decay_s`）仍须解析：kind 缺省
/// 为 Room（ legacy 调音即 Room 参数集），decay_s 缺省为 0（未设 ⇒ 走 legacy size 路）。
/// 写了就必须合法：kind 非字符串、未知名、decay_s 越界一律响亮拒绝。`fx` 与逐件链共用。
fn parse_reverb(obj: &[(String, Json)], base: &str) -> Result<DrumReverbPreset, DrumPresetError> {
    let kind = match obj.iter().find(|(k, _)| k == "kind") {
        None => DrumReverbKind::Room,
        Some((_, Json::Str(name))) => {
            DrumReverbKind::from_name(name, &format!("{base}.reverb.kind"))?
        }
        Some(_) => {
            return Err(field_err(
                &format!("{base}.reverb.kind"),
                "reverb kind must be a string",
            ));
        }
    };
    let decay_s = match obj.iter().find(|(k, _)| k == "decay_s") {
        None => 0.0,
        Some((_, Json::Num(n))) => {
            let raw = *n;
            if !raw.is_finite() || raw < f64::from(0.1f32) || raw > f64::from(10.0f32) {
                return Err(field_err(
                    &format!("{base}.reverb.decay_s"),
                    format!("value {raw} out of 0.1..=10"),
                ));
            }
            #[allow(clippy::cast_possible_truncation)] // 已校验落在 f32 可精确承载的值域内
            let value = raw as f32;
            value
        }
        Some(_) => {
            return Err(field_err(
                &format!("{base}.reverb.decay_s"),
                "expected a number",
            ));
        }
    };
    Ok(DrumReverbPreset {
        kind,
        decay_s,
        damp: get_f32_ranged(obj, "damp", &format!("{base}.reverb.damp"), 0.0, 1.0)?,
        wet: get_f32_ranged(obj, "wet", &format!("{base}.reverb.wet"), 0.0, 1.0)?,
        predelay_s: get_f32_ranged(
            obj,
            "predelay_s",
            &format!("{base}.reverb.predelay_s"),
            0.0,
            0.1,
        )?,
    })
}

/// 解析磁带段（`{base}.tape`）：整段字段必填。`fx` 与逐件链共用。
fn parse_tape(obj: &[(String, Json)], base: &str) -> Result<DrumTapePreset, DrumPresetError> {
    Ok(DrumTapePreset {
        drive: get_f32_ranged(obj, "drive", &format!("{base}.tape.drive"), 0.0, 1.0)?,
        mix: get_f32_ranged(obj, "mix", &format!("{base}.tape.mix"), 0.0, 1.0)?,
        hf_hz: get_f32_ranged(obj, "hf_hz", &format!("{base}.tape.hf_hz"), 2000.0, 20000.0)?,
        asym: get_f32_ranged(obj, "asym", &format!("{base}.tape.asym"), 0.0, 0.5)?,
        wow_rate_hz: get_f32_ranged(
            obj,
            "wow_rate_hz",
            &format!("{base}.tape.wow_rate_hz"),
            0.05,
            2.0,
        )?,
        wow_depth_ms: get_f32_ranged(
            obj,
            "wow_depth_ms",
            &format!("{base}.tape.wow_depth_ms"),
            0.0,
            8.0,
        )?,
        flutter_rate_hz: get_f32_ranged(
            obj,
            "flutter_rate_hz",
            &format!("{base}.tape.flutter_rate_hz"),
            2.0,
            15.0,
        )?,
        flutter_depth_ms: get_f32_ranged(
            obj,
            "flutter_depth_ms",
            &format!("{base}.tape.flutter_depth_ms"),
            0.0,
            2.0,
        )?,
        seed: get_f32_ranged(obj, "seed", &format!("{base}.tape.seed"), 0.0, 16_777_215.0)?,
    })
}

/// 解析延迟段（`{base}.delay`）：整段字段必填。`fx` 与逐件链共用。
fn parse_delay(obj: &[(String, Json)], base: &str) -> Result<DrumDelayPreset, DrumPresetError> {
    let ping_pong = match get(obj, "ping_pong")? {
        Json::Bool(b) => *b,
        _ => {
            return Err(field_err(
                &format!("{base}.delay.ping_pong"),
                "expected a boolean",
            ));
        }
    };
    Ok(DrumDelayPreset {
        time_s: get_f32_ranged(obj, "time_s", &format!("{base}.delay.time_s"), 0.01, 2.0)?,
        feedback: get_f32_ranged(obj, "feedback", &format!("{base}.delay.feedback"), 0.0, 0.9)?,
        mix: get_f32_ranged(obj, "mix", &format!("{base}.delay.mix"), 0.0, 0.6)?,
        damp: get_f32_ranged(obj, "damp", &format!("{base}.delay.damp"), 0.0, 1.0)?,
        ping_pong,
    })
}

/// 解析瞬态塑形段（`{base}.transient`）：整段字段必填。`fx` 与逐件链共用。
fn parse_transient(
    obj: &[(String, Json)],
    base: &str,
) -> Result<DrumTransientPreset, DrumPresetError> {
    Ok(DrumTransientPreset {
        attack: get_f32_ranged(
            obj,
            "attack",
            &format!("{base}.transient.attack"),
            -1.0,
            1.0,
        )?,
        sustain: get_f32_ranged(
            obj,
            "sustain",
            &format!("{base}.transient.sustain"),
            -1.0,
            1.0,
        )?,
    })
}

/// 解析比特碾碎段（`{base}.crusher`）：整段字段必填。`fx` 与逐件链共用。
fn parse_crusher(obj: &[(String, Json)], base: &str) -> Result<DrumCrusherPreset, DrumPresetError> {
    Ok(DrumCrusherPreset {
        bits: get_f32_ranged(obj, "bits", &format!("{base}.crusher.bits"), 4.0, 16.0)?,
        down: get_f32_ranged(obj, "down", &format!("{base}.crusher.down"), 1.0, 64.0)?,
        aa: get_f32_ranged(obj, "aa", &format!("{base}.crusher.aa"), 0.0, 1.0)?,
    })
}

/// 解析噪声源段（`{base}.noise`）：整段字段必填。`fx` 与逐件链共用。
fn parse_noise(obj: &[(String, Json)], base: &str) -> Result<DrumNoisePreset, DrumPresetError> {
    Ok(DrumNoisePreset {
        level: get_f32_ranged(obj, "level", &format!("{base}.noise.level"), 0.0, 1.0)?,
        color: get_f32_ranged(obj, "color", &format!("{base}.noise.color"), 0.0, 1.0)?,
        crackle_density: get_f32_ranged(
            obj,
            "crackle_density",
            &format!("{base}.noise.crackle_density"),
            0.0,
            240.0,
        )?,
        crackle_level: get_f32_ranged(
            obj,
            "crackle_level",
            &format!("{base}.noise.crackle_level"),
            0.0,
            1.0,
        )?,
        seed: get_f32_ranged(
            obj,
            "seed",
            &format!("{base}.noise.seed"),
            0.0,
            16_777_215.0,
        )?,
    })
}

/// 自动化目标引用的效果族在预置 `fx` 段里是否真有着陆器件：
/// `strip` / `reverb` 随 `fx` 本身在场；其余五族要有对应子段。
/// 没有着陆器件的目标是内容错误（生成时无处落），解析期拒绝。
fn automation_family_lands(fx: Option<&DrumFxPreset>, family: &str) -> bool {
    let Some(fx) = fx else {
        return false;
    };
    match family {
        "strip" | "reverb" => true,
        "tape" => fx.tape.is_some(),
        "delay" => fx.delay.is_some(),
        "transient" => fx.transient.is_some(),
        "crusher" => fx.crusher.is_some(),
        "noise" => fx.noise.is_some(),
        _ => false,
    }
}

/// 解析一个自动化目标字符串（`automation[i].target`）：词表与值域
/// 见 [`DrumAutomationTarget`] 与 [`DRUM_FX_PARAM_DOMAINS`]。
/// 未知目标 / 未知族 / 未知参数 / 目标无着陆器件 / 条件参数未启用，
/// 全部响亮拒绝并点名（默认不能静默）。
fn parse_automation_target(
    raw: &str,
    fx: Option<&DrumFxPreset>,
    idx: usize,
) -> Result<DrumAutomationTarget, DrumPresetError> {
    let field = "automation.target";
    match raw {
        "track_volume" => return Ok(DrumAutomationTarget::TrackVolume),
        "track_pan" => return Ok(DrumAutomationTarget::TrackPan),
        _ => {}
    }
    let Some((family, param)) = raw.split_once('.') else {
        return Err(field_err(
            field,
            format!(
                "segment {idx}: unknown automation target {raw:?} (expected track_volume, track_pan, or <family>.<param>)"
            ),
        ));
    };
    if !drum_fx_family_known(family) {
        return Err(field_err(
            field,
            format!(
                "segment {idx}: unknown effect family {family:?} in target {raw:?} (expected strip/tape/reverb/delay/transient/crusher/noise)"
            ),
        ));
    }
    if drum_fx_param_domain(family, param).is_none() {
        return Err(field_err(
            field,
            format!(
                "segment {idx}: unknown parameter {param:?} for family {family:?} in target {raw:?}"
            ),
        ));
    }
    if !automation_family_lands(fx, family) {
        return Err(field_err(
            field,
            format!(
                "segment {idx}: target {raw:?} has no device to land on (the preset has no fx section or no fx.{family} subsection)"
            ),
        ));
    }
    // 通道条的条件参数：`hpf_hz` / `cutoff_hz` / `filter_enabled` /
    // `output_gain_db` 只在 fx 定值启用对应级时才写进器件参数表
    // （见 `strip_eq_params` 与 `drum_fx_device_specs`），未启用即
    // 无落点，与「无对应子段」同罪，解析期拒绝。
    if family == "strip" {
        let fx = fx.expect("strip only lands when fx exists");
        let disabled = match param {
            "hpf_hz" if fx.hpf_hz <= 0.0 => Some("fx.hpf_hz is 0 (high-pass disabled)"),
            "cutoff_hz" | "filter_enabled" if fx.cutoff_hz >= 20_000.0 => {
                Some("fx.cutoff_hz is 20000 (low-pass disabled)")
            }
            "output_gain_db" if fx.output_db == 0.0 => {
                Some("fx.output_db is 0 (no output_gain_db param is written)")
            }
            _ => None,
        };
        if let Some(reason) = disabled {
            return Err(field_err(
                field,
                format!("segment {idx}: target {raw:?} has no landing param: {reason}"),
            ));
        }
    }
    Ok(DrumAutomationTarget::DeviceParam {
        family: family.to_owned(),
        param: param.to_owned(),
    })
}

/// 解析自动化段（`automation`）：缺省（无此键）⇒ 空表，与引入本段
/// 前的行为一致。形状与值域的全部校验都在这里一次钉死，生成路径
/// 只做落点换算，不再猜测：段必须是对象、目标必须可解析、点列
/// 非空、tick 严格升序且不越 pattern 全长、值在目标值域内、
/// 同一目标不许出现两段。
fn parse_automation(
    obj: &[(String, Json)],
    fx: Option<&DrumFxPreset>,
    bars: u32,
) -> Result<Vec<DrumAutomationPreset>, DrumPresetError> {
    let Some(raw) = get_opt(obj, "automation") else {
        return Ok(Vec::new());
    };
    let arr = raw
        .as_array()
        .ok_or_else(|| field_err("automation", "automation must be an array"))?;
    let span = u64::from(bars) * crate::drum_gen::TICKS_PER_BAR;
    let mut segments: Vec<DrumAutomationPreset> = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let seg = item
            .as_object()
            .ok_or_else(|| field_err("automation", format!("segment {idx} must be an object")))?;
        let target_raw = match get_opt(seg, "target") {
            Some(Json::Str(s)) => s.clone(),
            _ => {
                return Err(field_err(
                    "automation.target",
                    format!("segment {idx}: `target` is required and must be a string"),
                ));
            }
        };
        let target = parse_automation_target(&target_raw, fx, idx)?;
        if segments
            .iter()
            .any(|s: &DrumAutomationPreset| s.target == target)
        {
            return Err(field_err(
                "automation.target",
                format!(
                    "segment {idx}: duplicate automation segment for target `{}`",
                    target.describe()
                ),
            ));
        }
        let curve = match get_opt(seg, "curve") {
            None => DrumAutomationCurve::Linear,
            Some(Json::Str(s)) => DrumAutomationCurve::from_name(s, "automation.curve")?,
            Some(_) => {
                return Err(field_err(
                    "automation.curve",
                    format!("segment {idx}: `curve` must be a string"),
                ));
            }
        };
        let points_raw = match get_opt(seg, "points") {
            Some(Json::Arr(points)) => points,
            _ => {
                return Err(field_err(
                    "automation.points",
                    format!(
                        "segment {idx} ({}): `points` is required and must be an array of [tick, value] pairs",
                        target.describe()
                    ),
                ));
            }
        };
        if points_raw.is_empty() {
            return Err(field_err(
                "automation.points",
                format!(
                    "segment {idx} ({}): points must not be empty (omit the segment instead)",
                    target.describe()
                ),
            ));
        }
        let (min, max) = target.value_domain();
        let mut points: Vec<DrumAutomationPoint> = Vec::with_capacity(points_raw.len());
        for (j, point_raw) in points_raw.iter().enumerate() {
            let pair = match point_raw {
                Json::Arr(pair) if pair.len() == 2 => pair,
                _ => {
                    return Err(field_err(
                        "automation.points",
                        format!(
                            "segment {idx} ({}): point {j} must be a [tick, value] pair",
                            target.describe()
                        ),
                    ));
                }
            };
            let tick_raw = match &pair[0] {
                Json::Num(n) => *n,
                _ => {
                    return Err(field_err(
                        "automation.points",
                        format!(
                            "segment {idx} ({}): point {j} tick must be a number",
                            target.describe()
                        ),
                    ));
                }
            };
            // fract 先行：非整数 tick 在转 u64 前拒绝；`as` 对超大
            // 浮点是饱和转换，饱和值随后被 span 检查抓住，不会绕过。
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let tick = tick_raw as u64;
            if tick_raw.fract() != 0.0 || tick_raw < 0.0 || tick > span {
                return Err(field_err(
                    "automation.points",
                    format!(
                        "segment {idx} ({}): point {j} tick {tick_raw} out of 0..={span} (the pattern span) or not an integer",
                        target.describe()
                    ),
                ));
            }
            let value_raw = match &pair[1] {
                Json::Num(n) => *n,
                _ => {
                    return Err(field_err(
                        "automation.points",
                        format!(
                            "segment {idx} ({}): point {j} value must be a number",
                            target.describe()
                        ),
                    ));
                }
            };
            if !value_raw.is_finite() || value_raw < f64::from(min) || value_raw > f64::from(max) {
                return Err(field_err(
                    "automation.points",
                    format!(
                        "segment {idx} ({}): point {j} value {value_raw} out of {min}..={max} for this target",
                        target.describe()
                    ),
                ));
            }
            #[allow(clippy::cast_possible_truncation)] // 已校验落在目标值域（f32 可承载）内
            let value = value_raw as f32;
            if let Some(prev) = points.last()
                && tick <= prev.tick
            {
                return Err(field_err(
                    "automation.points",
                    format!(
                        "segment {idx} ({}): point ticks must be strictly ascending (point {j} tick {tick} <= previous {})",
                        target.describe(),
                        prev.tick
                    ),
                ));
            }
            points.push(DrumAutomationPoint { tick, value });
        }
        segments.push(DrumAutomationPreset {
            target,
            curve,
            points,
        });
    }
    Ok(segments)
}

fn parse_fx(obj: &[(String, Json)]) -> Result<DrumFxPreset, DrumPresetError> {
    let eq = parse_eq(get_obj(obj, "eq")?, "fx")?;
    let comp = parse_comp(get_obj(obj, "comp")?, "fx")?;
    let reverb = parse_reverb(get_obj(obj, "reverb")?, "fx")?;
    let tape = match get_opt(obj, "tape") {
        None => None,
        Some(tape_json) => {
            let t = tape_json
                .as_object()
                .ok_or_else(|| field_err("fx.tape", "tape must be an object"))?;
            Some(parse_tape(t, "fx")?)
        }
    };
    let delay = match get_opt(obj, "delay") {
        None => None,
        Some(delay_json) => {
            let d = delay_json
                .as_object()
                .ok_or_else(|| field_err("fx.delay", "delay must be an object"))?;
            Some(parse_delay(d, "fx")?)
        }
    };
    let transient = match get_opt(obj, "transient") {
        None => None,
        Some(transient_json) => {
            let t = transient_json
                .as_object()
                .ok_or_else(|| field_err("fx.transient", "transient must be an object"))?;
            Some(parse_transient(t, "fx")?)
        }
    };
    let crusher = match get_opt(obj, "crusher") {
        None => None,
        Some(crusher_json) => {
            let c = crusher_json
                .as_object()
                .ok_or_else(|| field_err("fx.crusher", "crusher must be an object"))?;
            Some(parse_crusher(c, "fx")?)
        }
    };
    let noise = match get_opt(obj, "noise") {
        None => None,
        Some(noise_json) => {
            let n = noise_json
                .as_object()
                .ok_or_else(|| field_err("fx.noise", "noise must be an object"))?;
            Some(parse_noise(n, "fx")?)
        }
    };
    Ok(DrumFxPreset {
        eq_low_db: eq.low_db,
        eq_mid_db: eq.mid_db,
        eq_high_db: eq.high_db,
        eq_low_hz: eq.low_hz,
        eq_mid_hz: eq.mid_hz,
        eq_mid_q: eq.mid_q,
        eq_high_hz: eq.high_hz,
        hpf_hz: eq.hpf_hz,
        cutoff_hz: eq.cutoff_hz,
        comp_threshold_db: comp.threshold_db,
        comp_ratio: comp.ratio,
        comp_knee_db: comp.knee_db,
        comp_attack_s: comp.attack_s,
        comp_release_s: comp.release_s,
        comp_makeup_db: comp.makeup_db,
        output_db: get_f32_ranged(obj, "output_db", "fx.output_db", -12.0, 12.0)?,
        reverb_kind: reverb.kind,
        reverb_decay_s: reverb.decay_s,
        reverb_damp: reverb.damp,
        reverb_wet: reverb.wet,
        reverb_predelay_s: reverb.predelay_s,
        tape,
        delay,
        transient,
        crusher,
        noise,
    })
}

/// 读一个 GM 键数组字段并校验规范化形态：非空、每键 0..=127、严格
/// 升序（升序 + 无重复一并钉死）。`field` 是报错点名的完整字段路径。
fn parse_key_group(
    obj: &[(String, Json)],
    key: &str,
    field: &str,
) -> Result<Vec<u8>, DrumPresetError> {
    let arr = get_arr(obj, key).map_err(|_| field_err(field, "expected an array of GM keys"))?;
    if arr.is_empty() {
        return Err(field_err(field, "key group must not be empty"));
    }
    let mut keys = Vec::with_capacity(arr.len());
    for raw in arr {
        let value = match raw {
            Json::Num(n) if n.fract() == 0.0 && *n >= 0.0 && *n <= 127.0 => *n as u8,
            _ => {
                return Err(field_err(
                    field,
                    format!("keys must be integers in 0..=127, got {raw:?}"),
                ));
            }
        };
        if let Some(previous) = keys.last()
            && value <= *previous
        {
            return Err(field_err(
                field,
                "keys must be strictly ascending (no duplicates)",
            ));
        }
        keys.push(value);
    }
    Ok(keys)
}

/// 解析 `lane_chains` 段（可选；缺省 = 空表）。规范化校验在本层完成：
/// 组数上限、组内升序非空、组间不重叠、组按首键升序、键必须是本预置
/// 鼓道（主道 ∪ fill 道）的键、链内至少一段处理、侧链 key 键是鼓道
/// 键的子集。唯一例外：原声族里与某个鼓道键同属一个 choke 组的键
/// 可以被同一条链认领——引擎按程序全部 region 分区，choke 组必须
/// 整组同链（见 [`lane_chain_generation_violations`]），而踩镲 44
/// 这类键未必是 pattern 鼓道；不放行则出厂预置无从写对。choke 组
/// 不拆分是**生成期**校验（依赖所选 SFZ 程序与落地期次，见
/// [`lane_chain_generation_violations`]）。
fn parse_lane_chains(
    obj: &[(String, Json)],
    lanes: &[DrumLanePreset],
    fill_lanes: Option<&[DrumLanePreset]>,
    kit_family: KitFamily,
    sfz_path: Option<&str>,
) -> Result<Vec<DrumLaneChainPreset>, DrumPresetError> {
    let Some(raw) = get_opt(obj, "lane_chains") else {
        return Ok(Vec::new());
    };
    let arr = raw
        .as_array()
        .ok_or_else(|| field_err("lane_chains", "lane_chains must be an array"))?;
    if arr.len() > 16 {
        return Err(field_err(
            "lane_chains",
            format!("at most 16 lane chain groups per preset, got {}", arr.len()),
        ));
    }
    let mut lane_keys: Vec<u8> = lanes.iter().map(|lane| lane.gm_key).collect();
    if let Some(fill) = fill_lanes {
        lane_keys.extend(fill.iter().map(|lane| lane.gm_key));
    }
    // 可认领键 = 鼓道键 ∪ 与鼓道键同 choke 组的程序键（原声族）。
    let mut claimable: Vec<u8> = lane_keys.clone();
    if kit_family == KitFamily::Acoustic {
        for component in choke_groups(sfz_path) {
            if component.iter().any(|key| lane_keys.contains(key)) {
                for key in component {
                    if !claimable.contains(&key) {
                        claimable.push(key);
                    }
                }
            }
        }
    }
    let mut chains: Vec<DrumLaneChainPreset> = Vec::with_capacity(arr.len());
    let mut claimed: Vec<u8> = Vec::new();
    let mut previous_first: Option<u8> = None;
    for (idx, item) in arr.iter().enumerate() {
        let entry = item
            .as_object()
            .ok_or_else(|| field_err("lane_chains", format!("group {idx} must be an object")))?;
        let name = get_str(entry, "name")?.to_owned();
        if name.is_empty() {
            return Err(field_err(
                "lane_chains",
                format!("group {idx} name must not be empty"),
            ));
        }
        let base = format!("lane_chains.{name}.chain");
        let keys = parse_key_group(entry, "keys", &format!("lane_chains.{name}.keys"))?;
        for key in &keys {
            if !claimable.contains(key) {
                return Err(field_err(
                    &format!("lane_chains.{name}.keys"),
                    format!(
                        "key {key} is not one of this preset's lane keys or its choke-group mates"
                    ),
                ));
            }
            if claimed.contains(key) {
                return Err(field_err(
                    &format!("lane_chains.{name}.keys"),
                    format!("key {key} is already claimed by an earlier lane chain group"),
                ));
            }
            claimed.push(*key);
        }
        let first = keys[0];
        if let Some(previous) = previous_first
            && first <= previous
        {
            return Err(field_err(
                "lane_chains",
                "groups must be ordered by their first key (strictly ascending)",
            ));
        }
        previous_first = Some(first);
        let chain_obj = get_obj(entry, "chain")?;
        let eq = match get_opt(chain_obj, "eq") {
            None => None,
            Some(raw_eq) => {
                let eq_obj = raw_eq
                    .as_object()
                    .ok_or_else(|| field_err(&format!("{base}.eq"), "eq must be an object"))?;
                Some(parse_eq(eq_obj, &base)?)
            }
        };
        let comp = match get_opt(chain_obj, "comp") {
            None => None,
            Some(raw_comp) => {
                let comp_obj = raw_comp
                    .as_object()
                    .ok_or_else(|| field_err(&format!("{base}.comp"), "comp must be an object"))?;
                Some(parse_comp(comp_obj, &base)?)
            }
        };
        let reverb = match get_opt(chain_obj, "reverb") {
            None => None,
            Some(raw_reverb) => {
                let reverb_obj = raw_reverb.as_object().ok_or_else(|| {
                    field_err(&format!("{base}.reverb"), "reverb must be an object")
                })?;
                Some(parse_reverb(reverb_obj, &base)?)
            }
        };
        let tape = match get_opt(chain_obj, "tape") {
            None => None,
            Some(raw_tape) => {
                let tape_obj = raw_tape
                    .as_object()
                    .ok_or_else(|| field_err(&format!("{base}.tape"), "tape must be an object"))?;
                Some(parse_tape(tape_obj, &base)?)
            }
        };
        let delay = match get_opt(chain_obj, "delay") {
            None => None,
            Some(raw_delay) => {
                let delay_obj = raw_delay.as_object().ok_or_else(|| {
                    field_err(&format!("{base}.delay"), "delay must be an object")
                })?;
                Some(parse_delay(delay_obj, &base)?)
            }
        };
        let transient = match get_opt(chain_obj, "transient") {
            None => None,
            Some(raw_transient) => {
                let transient_obj = raw_transient.as_object().ok_or_else(|| {
                    field_err(&format!("{base}.transient"), "transient must be an object")
                })?;
                Some(parse_transient(transient_obj, &base)?)
            }
        };
        let crusher = match get_opt(chain_obj, "crusher") {
            None => None,
            Some(raw_crusher) => {
                let crusher_obj = raw_crusher.as_object().ok_or_else(|| {
                    field_err(&format!("{base}.crusher"), "crusher must be an object")
                })?;
                Some(parse_crusher(crusher_obj, &base)?)
            }
        };
        let noise = match get_opt(chain_obj, "noise") {
            None => None,
            Some(raw_noise) => {
                let noise_obj = raw_noise.as_object().ok_or_else(|| {
                    field_err(&format!("{base}.noise"), "noise must be an object")
                })?;
                Some(parse_noise(noise_obj, &base)?)
            }
        };
        let pan = match get_opt(chain_obj, "pan") {
            None => None,
            Some(_) => Some(get_f32_ranged(
                chain_obj,
                "pan",
                &format!("{base}.pan"),
                -1.0,
                1.0,
            )?),
        };
        let gain_db = match get_opt(chain_obj, "gain_db") {
            None => None,
            Some(_) => Some(get_f32_ranged(
                chain_obj,
                "gain_db",
                &format!("{base}.gain_db"),
                -60.0,
                24.0,
            )?),
        };
        let sidechain = match get_opt(chain_obj, "sidechain") {
            None => None,
            Some(raw_sidechain) => {
                let sidechain_obj = raw_sidechain.as_object().ok_or_else(|| {
                    field_err(&format!("{base}.sidechain"), "sidechain must be an object")
                })?;
                let key_keys = parse_key_group(
                    sidechain_obj,
                    "key_keys",
                    &format!("{base}.sidechain.key_keys"),
                )?;
                for key in &key_keys {
                    if !lane_keys.contains(key) {
                        return Err(field_err(
                            &format!("{base}.sidechain.key_keys"),
                            format!("key key {key} is not one of this preset's lane keys"),
                        ));
                    }
                }
                Some(DrumLaneSidechainPreset { key_keys })
            }
        };
        if eq.is_none()
            && comp.is_none()
            && reverb.is_none()
            && tape.is_none()
            && delay.is_none()
            && transient.is_none()
            && crusher.is_none()
            && noise.is_none()
            && pan.is_none()
            && gain_db.is_none()
            && sidechain.is_none()
        {
            return Err(field_err(
                &format!("lane_chains.{name}"),
                "lane chain group declares an empty chain (no eq/comp/reverb/tape/delay/transient/crusher/noise/pan/gain_db/sidechain)",
            ));
        }
        chains.push(DrumLaneChainPreset {
            name,
            keys,
            eq,
            comp,
            reverb,
            tape,
            delay,
            transient,
            crusher,
            noise,
            pan,
            gain_db,
            sidechain,
        });
    }
    Ok(chains)
}

fn parse_bus(obj: &[(String, Json)]) -> Result<DrumBusPreset, DrumPresetError> {
    let glue = get_obj(obj, "glue")?;
    let clipper = get_obj(obj, "clipper")?;
    let oversample_raw = get_u64(clipper, "oversample")?;
    let oversample = u32::try_from(oversample_raw)
        .ok()
        .filter(|v| matches!(v, 1 | 2 | 4 | 8))
        .ok_or_else(|| {
            field_err(
                "bus.clipper.oversample",
                format!("oversample must be 1, 2, 4 or 8, got {oversample_raw}"),
            )
        })?;
    Ok(DrumBusPreset {
        glue: DrumBusGluePreset {
            threshold_db: get_f32_ranged(
                glue,
                "threshold_db",
                "bus.glue.threshold_db",
                -60.0,
                0.0,
            )?,
            ratio: get_f32_ranged(glue, "ratio", "bus.glue.ratio", 1.0, 20.0)?,
            knee_db: get_f32_ranged(glue, "knee_db", "bus.glue.knee_db", 0.0, 24.0)?,
            attack_s: get_f32_ranged(glue, "attack_s", "bus.glue.attack_s", 0.0005, 0.2)?,
            release_s: get_f32_ranged(glue, "release_s", "bus.glue.release_s", 0.01, 1.0)?,
            makeup_db: get_f32_ranged(glue, "makeup_db", "bus.glue.makeup_db", -12.0, 12.0)?,
        },
        clipper: DrumBusClipperPreset {
            drive_db: get_f32_ranged(clipper, "drive_db", "bus.clipper.drive_db", 0.0, 24.0)?,
            ceiling_db: get_f32_ranged(clipper, "ceiling_db", "bus.clipper.ceiling_db", -6.0, 0.0)?,
            knee_db: get_f32_ranged(clipper, "knee_db", "bus.clipper.knee_db", 0.0, 6.0)?,
            mix: get_f32_ranged(clipper, "mix", "bus.clipper.mix", 0.0, 1.0)?,
            oversample,
        },
    })
}

/// 效果族在场标记（生成侧的去重判据由调用方按参数名扫轨上已有器件得出，
/// 判读口径在调用方：参数名命中 `yeban-dsp` 的族拼写表）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrumFxFamilies {
    /// 通道条族（EQ/滤波/压缩）已在场。
    pub strip: bool,
    /// 磁带族已在场。
    pub tape: bool,
    /// 混响族已在场。
    pub reverb: bool,
    /// 延迟族已在场。
    pub delay: bool,
    /// 瞬态塑形族已在场（D3 接线）。
    pub transient: bool,
    /// 比特碾碎族已在场（D4 接线）。
    pub crusher: bool,
    /// 噪声源族已在场（D5 新器件）。
    pub noise: bool,
}

/// 一台待装效果器件的配方（名字 + 参数表）；身份（`EntityId`）由调用方
/// 按各自纪律分配（UI 路径随机、MCP 路径确定性派生），配方只有这一份，
/// 保证两条生成路径装出的效果层逐参数一致。
#[derive(Debug, Clone, PartialEq)]
pub struct DrumFxDeviceSpec {
    /// 效果族标识（`strip` / `tape` / `reverb` / `delay` / `transient` / `crusher` / `noise`）。
    pub family: &'static str,
    /// 器件显示名。
    pub name: &'static str,
    /// `(参数名, 值)` 表，顺序稳定。参数名一律用 `yeban-dsp`
    /// `device_param` 的规范拼写（与共享契约 `EFFECT_GROUPS` 一致）。
    pub params: Vec<(String, f32)>,
}

/// EQ 段 → 通道条参数表的前半（EQ 开关 + 三段 + 高通/低通塑形）。
/// 整轨 `fx` 与逐件链共用：同一段只许有一套参数拼法。
fn strip_eq_params(eq: &DrumEqPreset) -> Vec<(String, f32)> {
    let mut params: Vec<(String, f32)> = vec![
        ("eq_enabled".to_owned(), 1.0),
        ("eq_low_gain".to_owned(), eq.low_db),
        ("eq_low_freq".to_owned(), eq.low_hz),
        ("eq_mid_gain".to_owned(), eq.mid_db),
        ("eq_mid_freq".to_owned(), eq.mid_hz),
        ("eq_mid_q".to_owned(), eq.mid_q),
        ("eq_high_gain".to_owned(), eq.high_db),
        ("eq_high_freq".to_owned(), eq.high_hz),
    ];
    if eq.hpf_hz > 0.0 {
        params.push(("hpf_hz".to_owned(), eq.hpf_hz));
    }
    if eq.cutoff_hz < 20_000.0 {
        params.push(("filter_enabled".to_owned(), 1.0));
        params.push(("cutoff_hz".to_owned(), eq.cutoff_hz));
    }
    params
}

/// 压缩段 → 通道条参数表的后半（压缩级开关 + 六参数）。
fn strip_comp_params(comp: &DrumCompPreset) -> Vec<(String, f32)> {
    vec![
        ("compressor_enabled".to_owned(), 1.0),
        ("threshold_db".to_owned(), comp.threshold_db),
        ("ratio".to_owned(), comp.ratio),
        ("knee_db".to_owned(), comp.knee_db),
        ("attack_s".to_owned(), comp.attack_s),
        ("release_s".to_owned(), comp.release_s),
        ("makeup_db".to_owned(), comp.makeup_db),
    ]
}

/// 混响段 → 混响器件参数表。
fn reverb_params(reverb: &DrumReverbPreset) -> Vec<(String, f32)> {
    vec![
        ("reverb_kind".to_owned(), reverb.kind.device_value()),
        ("reverb_decay_s".to_owned(), reverb.decay_s),
        ("reverb_damp".to_owned(), reverb.damp),
        ("reverb_wet".to_owned(), reverb.wet),
        ("reverb_predelay".to_owned(), reverb.predelay_s),
    ]
}

/// 磁带段 → 磁带器件参数表。
fn tape_params(tape: &DrumTapePreset) -> Vec<(String, f32)> {
    vec![
        ("tape_drive".to_owned(), tape.drive),
        ("tape_mix".to_owned(), tape.mix),
        ("tape_hf_hz".to_owned(), tape.hf_hz),
        ("tape_asym".to_owned(), tape.asym),
        ("tape_wow_rate_hz".to_owned(), tape.wow_rate_hz),
        ("tape_wow_depth_ms".to_owned(), tape.wow_depth_ms),
        ("tape_flutter_rate_hz".to_owned(), tape.flutter_rate_hz),
        ("tape_flutter_depth_ms".to_owned(), tape.flutter_depth_ms),
        ("tape_seed".to_owned(), tape.seed),
    ]
}

/// 瞬态塑形段 → 瞬态塑形器件参数表。
fn transient_params(transient: &DrumTransientPreset) -> Vec<(String, f32)> {
    vec![
        ("transient_attack".to_owned(), transient.attack),
        ("transient_sustain".to_owned(), transient.sustain),
    ]
}

/// 比特碾碎段 → 比特碾碎器件参数表。
fn crusher_params(crusher: &DrumCrusherPreset) -> Vec<(String, f32)> {
    vec![
        ("crush_bits".to_owned(), crusher.bits),
        ("crush_down".to_owned(), crusher.down),
        ("crush_aa".to_owned(), crusher.aa),
    ]
}

/// 噪声源段 → 噪声源器件参数表。
fn noise_params(noise: &DrumNoisePreset) -> Vec<(String, f32)> {
    vec![
        ("noise_level".to_owned(), noise.level),
        ("noise_color".to_owned(), noise.color),
        ("noise_crackle_density".to_owned(), noise.crackle_density),
        ("noise_crackle_level".to_owned(), noise.crackle_level),
        ("noise_seed".to_owned(), noise.seed),
    ]
}

/// 延迟段 → 延迟器件参数表。
fn delay_params(delay: &DrumDelayPreset) -> Vec<(String, f32)> {
    vec![
        ("delay_time_s".to_owned(), delay.time_s),
        ("delay_feedback".to_owned(), delay.feedback),
        ("delay_mix".to_owned(), delay.mix),
        ("delay_damp".to_owned(), delay.damp),
        (
            "delay_ping_pong".to_owned(),
            if delay.ping_pong { 1.0 } else { 0.0 },
        ),
    ]
}

impl DrumFxPreset {
    /// 扁平 `fx` 字段的 EQ 段视图（与逐件链共用参数构造的接缝）。
    fn eq_view(&self) -> DrumEqPreset {
        DrumEqPreset {
            low_db: self.eq_low_db,
            mid_db: self.eq_mid_db,
            high_db: self.eq_high_db,
            low_hz: self.eq_low_hz,
            mid_hz: self.eq_mid_hz,
            mid_q: self.eq_mid_q,
            high_hz: self.eq_high_hz,
            hpf_hz: self.hpf_hz,
            cutoff_hz: self.cutoff_hz,
        }
    }

    /// 扁平 `fx` 字段的压缩段视图。
    fn comp_view(&self) -> DrumCompPreset {
        DrumCompPreset {
            threshold_db: self.comp_threshold_db,
            ratio: self.comp_ratio,
            knee_db: self.comp_knee_db,
            attack_s: self.comp_attack_s,
            release_s: self.comp_release_s,
            makeup_db: self.comp_makeup_db,
        }
    }

    /// 扁平 `fx` 字段的混响段视图。
    fn reverb_view(&self) -> DrumReverbPreset {
        DrumReverbPreset {
            kind: self.reverb_kind,
            decay_s: self.reverb_decay_s,
            damp: self.reverb_damp,
            wet: self.reverb_wet,
            predelay_s: self.reverb_predelay_s,
        }
    }
}

/// 把 `fx` 段翻译成器件配方（跳过 `present` 里已在场的族）。
///
/// ⚠ EQ 与压缩必须合在**同一台**通道条器件里：插入链投影对通道条族只取
/// 第一台命中器件，分开装两台（EQ 一台、压缩一台）时第二台会被静默忽略。
/// 开关参数（`eq_enabled` 等）显式写出，保证对应级一定执行（投影的
/// 「没写旋钮的级不执行」口径）。
#[must_use]
pub fn drum_fx_device_specs(fx: &DrumFxPreset, present: DrumFxFamilies) -> Vec<DrumFxDeviceSpec> {
    let mut specs = Vec::new();
    // 噪声源是链头（加性源，见 `yeban-render` 的 `inserts.rs` §6），
    // 配方表也把它排在最前，与信号次序一致。
    if let Some(noise) = &fx.noise
        && !present.noise
    {
        specs.push(DrumFxDeviceSpec {
            family: "noise",
            name: "Noise Source",
            params: noise_params(noise),
        });
    }
    if !present.strip {
        let mut params = strip_eq_params(&fx.eq_view());
        params.extend(strip_comp_params(&fx.comp_view()));
        if fx.output_db != 0.0 {
            params.push(("output_gain_db".to_owned(), fx.output_db));
        }
        specs.push(DrumFxDeviceSpec {
            family: "strip",
            name: "Channel Strip",
            params,
        });
    }
    if let Some(transient) = &fx.transient
        && !present.transient
    {
        specs.push(DrumFxDeviceSpec {
            family: "transient",
            name: "Transient Shaper",
            params: transient_params(transient),
        });
    }
    if let Some(tape) = &fx.tape
        && !present.tape
    {
        specs.push(DrumFxDeviceSpec {
            family: "tape",
            name: "Tape Saturation",
            params: tape_params(tape),
        });
    }
    if let Some(crusher) = &fx.crusher
        && !present.crusher
    {
        specs.push(DrumFxDeviceSpec {
            family: "crusher",
            name: "Bitcrusher",
            params: crusher_params(crusher),
        });
    }
    if !present.reverb {
        specs.push(DrumFxDeviceSpec {
            family: "reverb",
            name: "Reverb",
            params: reverb_params(&fx.reverb_view()),
        });
    }
    if let Some(delay) = &fx.delay
        && !present.delay
    {
        specs.push(DrumFxDeviceSpec {
            family: "delay",
            name: "Delay",
            params: delay_params(delay),
        });
    }
    specs
}

/// 把一条逐件链预置翻译成器件配方（逐件处理线期 2）。
///
/// 与整轨 `fx` 的差别：① **无同族在场去重**——逐件链装在独立的 lane
/// 分支上，与轨上整轨链并行存在，不互相顶替；② 无 output 微调
/// （逐件链格式没有这个字段）；③ EQ 与压缩仍合在**同一台**通道条
/// 器件里（与 [`drum_fx_device_specs`] 同一口径）。`pan` 与
/// `sidechain` 不产器件：pan 进模型的 `LaneChainDef::pan`；sidechain
/// 由生成路径落成 lane 级侧链边（期 4），不进器件表。
#[must_use]
pub fn drum_lane_device_specs(chain: &DrumLaneChainPreset) -> Vec<DrumFxDeviceSpec> {
    let mut specs = Vec::new();
    if let Some(noise) = &chain.noise {
        specs.push(DrumFxDeviceSpec {
            family: "noise",
            name: "Noise Source",
            params: noise_params(noise),
        });
    }
    if chain.eq.is_some() || chain.comp.is_some() {
        let mut params = Vec::new();
        if let Some(eq) = &chain.eq {
            params.extend(strip_eq_params(eq));
        }
        if let Some(comp) = &chain.comp {
            params.extend(strip_comp_params(comp));
        }
        specs.push(DrumFxDeviceSpec {
            family: "strip",
            name: "Channel Strip",
            params,
        });
    }
    if let Some(transient) = &chain.transient {
        specs.push(DrumFxDeviceSpec {
            family: "transient",
            name: "Transient Shaper",
            params: transient_params(transient),
        });
    }
    if let Some(tape) = &chain.tape {
        specs.push(DrumFxDeviceSpec {
            family: "tape",
            name: "Tape Saturation",
            params: tape_params(tape),
        });
    }
    if let Some(crusher) = &chain.crusher {
        specs.push(DrumFxDeviceSpec {
            family: "crusher",
            name: "Bitcrusher",
            params: crusher_params(crusher),
        });
    }
    if let Some(reverb) = &chain.reverb {
        specs.push(DrumFxDeviceSpec {
            family: "reverb",
            name: "Reverb",
            params: reverb_params(reverb),
        });
    }
    if let Some(delay) = &chain.delay {
        specs.push(DrumFxDeviceSpec {
            family: "delay",
            name: "Delay",
            params: delay_params(delay),
        });
    }
    specs
}

/// 把 `bus` 段翻译成鼓总线链的器件配方：胶水压缩在前、Soft Clipper
/// 末级。链内次序由插入链投影的固定级序保证（压缩级 → 削波级），
/// 这里只需给出两台器件。参数名用 `yeban-dsp` `device_param` 的规范
/// 拼写（压缩 = 通道条压缩拼写、削波 = `ClipperField` 拼写）。
#[must_use]
pub fn drum_bus_device_specs(bus: &DrumBusPreset) -> Vec<DrumFxDeviceSpec> {
    vec![
        DrumFxDeviceSpec {
            family: "glue",
            name: "Glue Compressor",
            params: vec![
                ("compressor_enabled".to_owned(), 1.0),
                ("threshold_db".to_owned(), bus.glue.threshold_db),
                ("ratio".to_owned(), bus.glue.ratio),
                ("knee_db".to_owned(), bus.glue.knee_db),
                ("attack_s".to_owned(), bus.glue.attack_s),
                ("release_s".to_owned(), bus.glue.release_s),
                ("makeup_db".to_owned(), bus.glue.makeup_db),
            ],
        },
        DrumFxDeviceSpec {
            family: "clipper",
            name: "Soft Clipper",
            params: vec![
                ("clipper_drive_db".to_owned(), bus.clipper.drive_db),
                ("clipper_ceiling_db".to_owned(), bus.clipper.ceiling_db),
                ("clipper_knee_db".to_owned(), bus.clipper.knee_db),
                ("clipper_mix".to_owned(), bus.clipper.mix),
                (
                    "clipper_oversample".to_owned(),
                    bus.clipper.oversample as f32,
                ),
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// 鼓组装配（默认鼓组一份；按族分工，GM 映射统一）
// ---------------------------------------------------------------------------

/// 原声默认鼓组：Virtuosity Drums 基础组（CC0，仓内登记
/// `virtuosity-drums-basic`），路径为仓库相对路径（与 SFZ 装载的既有
/// 口径一致：相对仓库根解析）。
pub const ACOUSTIC_KIT_SFZ_PATH: &str = "assets/samples/virtuosity-drums/Programs/01-basic-kit.sfz";

/// 一份预置对应的鼓轨音源装配。
#[derive(Debug, Clone, PartialEq)]
pub enum DrumKitSetup {
    /// 原声：轨采样器绑定到 SFZ 定义文件。
    AcousticSfz {
        /// SFZ 路径（仓库相对）。
        sfz_path: String,
    },
    /// 电子：轨上插入一台内置乐器设备，参数表即鼓机识别所需的
    /// 五个键位名 + 音色参数（名字口径见 `yeban-engine` 的 `drums`
    /// 模块 §3.2：一个名字只指一处）。
    SynthDevice {
        /// 设备显示名。
        device_name: &'static str,
        /// `(参数名, 值)` 表，顺序稳定。
        params: Vec<(String, f32)>,
    },
}

/// 新建鼓轨 / 鼓总线的**出厂电平**（只动电平，不动 pattern 与音色参数）。
///
/// 2026-11-21 `T-FIX-42`：默认 `-6 dB` 鼓轨 + `0 dB` 鼓总线时，电子族多风格
/// 原始渲染真峰值越过 0 dBFS（phonk/lofi 还削波）。这里按风格把鼓总线出厂
/// 电平下调，给末级 Soft Clipper 之前的信号留出确定余量；鼓轨仍保持既有的
/// `-6 dB`，用户后续可自行在调音台改。原声族 / 用户自写预置保持旧值
/// （`0.0 dB`），逐位不变。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumFactoryLevels {
    /// 新建鼓轨的音量（dB）。
    pub track_volume_db: f32,
    /// 新建鼓总线（`AuxReturn`）的音量（dB）。
    pub bus_volume_db: f32,
}

impl Default for DrumFactoryLevels {
    fn default() -> Self {
        Self {
            track_volume_db: -6.0,
            bus_volume_db: 0.0,
        }
    }
}

/// 一份预置对应的出厂电平（见 [`DrumFactoryLevels`]）。
///
/// 电子族按风格给不同总线余量：实测（48 kHz、24-bit 量化前浮点母带）
/// 在旧的 `-6 / 0 dB` 下 phonk +1.05、lofi +0.93、trap +0.34、
/// hiphop +0.49 dBFS，故按各自峰值再留约 5–6 dB 余量。原声族总线沿用
/// `0 dB`（其峰值由采样音源决定，不在本任务口径内）。
#[must_use]
pub fn factory_drum_levels(preset: &DrumStylePreset) -> DrumFactoryLevels {
    if preset.kit_family != KitFamily::Electronic {
        return DrumFactoryLevels::default();
    }
    let bus_volume_db = match preset.id.as_str() {
        "trap" => -6.5,
        "drill" => -6.0,
        "phonk" => -7.0,
        "lofi" => -7.0,
        "house" => -6.0,
        "techno" => -6.0,
        "edm" => -6.5,
        "hiphop" => -6.5,
        // 未知 id 只来自用户自写预置：保持旧值，不替用户改电平。
        _ => 0.0,
    };
    DrumFactoryLevels {
        track_volume_db: -6.0,
        bus_volume_db,
    }
}

/// 默认鼓组的装配（按风格族分工）：
/// 原声族走 SFZ 采样；电子族走 DSP 合成鼓机。GM 键位两边统一
/// （kick 36 / snare 38 / clap 39 / closed hat 42 / open hat 46），
/// 这是引擎一轨一音源约束下的混搭形态：**同一份默认鼓组定义**，
/// 按件的性质选源，而不是一轨之内并行两件音源（引擎不支持，
/// 见 `yeban-engine::drums` §4.1）。
#[must_use]
pub fn kit_setup(preset: &DrumStylePreset) -> DrumKitSetup {
    match preset.kit_family {
        KitFamily::Acoustic => DrumKitSetup::AcousticSfz {
            sfz_path: preset
                .sfz_path
                .clone()
                .unwrap_or_else(|| ACOUSTIC_KIT_SFZ_PATH.to_owned()),
        },
        KitFamily::Electronic => {
            // 键位：核心五件 + 可选八件（引擎 `drums` 模块 §3.2 的
            // 可选键位名单）。GM 键位与原声族统一。
            let mut params: Vec<(String, f32)> = vec![
                ("kick_note".to_owned(), 36.0),
                ("snare_note".to_owned(), 38.0),
                ("closed_hat_note".to_owned(), 42.0),
                ("open_hat_note".to_owned(), 46.0),
                ("clap_note".to_owned(), 39.0),
                ("rim_note".to_owned(), 37.0),
                ("tom_low_note".to_owned(), 43.0),
                ("tom_mid_note".to_owned(), 45.0),
                ("tom_high_note".to_owned(), 48.0),
                ("crash_note".to_owned(), 49.0),
                ("ride_note".to_owned(), 51.0),
                ("cowbell_note".to_owned(), 56.0),
                ("shaker_note".to_owned(), 82.0),
                // 独立 808 低音（逐件＋侧链期 4）：GM 35（原声低鼓位）。
                // 电子族此前 35 无映射（静默不触发），本期起触发独立
                // 808 声部——trap/phonk 的 808 从此与 36 号 kick 分家。
                ("bass808_note".to_owned(), 35.0),
            ];
            for (name, value) in synth_style_params(preset.id.as_str()) {
                params.push((name.to_owned(), value));
            }
            DrumKitSetup::SynthDevice {
                device_name: "Yeban Drum Kit (Synth)",
                params,
            }
        }
    }
}

/// 电子族**逐风格**的合成参数全表（段 2）：每种风格把 13 个音色的
/// 全部参数逐项写全（⛔ 不许"没写的走器件默认"——一期电子 6 风格
/// 军鼓/拍手/踩镲逐值相同的事故正是默认回退造成的）。性格分工：
/// trap/drill/phonk/hiphop = 808 系（长底鼓，trap 与 drill 靠滑音
/// 时长与军鼓带宽区分）；edm = 909 基表（与 `yeban-dsp` 的
/// `DrumKitParams::RECIPE_909` 核心四件逐值相同，由 yeban-app 的
/// 跨 crate 判据对钉），house/techno = 909 变体（house 开镲更长、
/// techno 底鼓更短更硬）；lofi = HipHop 合成底 + 整体压暗
/// （高频收窄，空间与底噪近似在预置 FX 层）。未知 id 落 909 基表
/// （出厂预置的 id 集合由 FACTORY_SOURCES 钉住，未知 id 只来自
/// 用户自写预置文件，取基表是装配默认值、不是静默近似音色）。
fn synth_style_params(id: &str) -> Vec<(&'static str, f32)> {
    match id {
        "trap" => vec![
            ("kick_tune_hz", 45.0),
            ("kick_glide_hz", 150.0),
            ("kick_glide_s", 0.05),
            ("kick_attack_s", 0.0005),
            ("kick_decay_s", 1.1),
            ("kick_level", 1.0),
            ("kick_pan", 0.0),
            ("snare_tone_hz", 200.0),
            ("snare_tone_ratio", 1.8),
            ("snare_tone_decay_s", 0.06),
            ("snare_tone_level", 0.4),
            ("snare_noise_highpass_hz", 2500.0),
            ("snare_noise_lowpass_hz", 12000.0),
            ("snare_noise_decay_s", 0.07),
            ("snare_noise_level", 1.0),
            ("snare_attack_s", 0.0005),
            ("snare_level", 1.0),
            ("snare_pan", 0.0),
            ("hihat_base_hz", 320.0),
            ("hihat_highpass_hz", 8000.0),
            ("hihat_lowpass_hz", 16000.0),
            ("hihat_attack_s", 0.0005),
            ("hihat_closed_decay_s", 0.04),
            ("hihat_open_decay_s", 0.3),
            ("hihat_level", 0.65),
            ("hihat_pan", 0.15),
            ("clap_highpass_hz", 1800.0),
            ("clap_lowpass_hz", 7000.0),
            ("clap_bursts", 3.0),
            ("clap_burst_decay_s", 0.011),
            ("clap_burst_spacing_s", 0.008),
            ("clap_attack_s", 0.0005),
            ("clap_decay_s", 0.18),
            ("clap_level", 0.7),
            ("clap_pan", -0.1),
            ("tom_tune_hz", 90.0),
            ("tom_glide_hz", 130.0),
            ("tom_glide_s", 0.03),
            ("tom_attack_s", 0.0005),
            ("tom_decay_s", 0.35),
            ("tom_level", 0.8),
            ("tom_pan", 0.15),
            ("rim_tone_hz", 1800.0),
            ("rim_noise_highpass_hz", 1500.0),
            ("rim_noise_lowpass_hz", 6500.0),
            ("rim_decay_s", 0.045),
            ("rim_attack_s", 0.0005),
            ("rim_level", 0.7),
            ("rim_pan", 0.1),
            ("shaker_highpass_hz", 6000.0),
            ("shaker_lowpass_hz", 14000.0),
            ("shaker_attack_s", 0.007),
            ("shaker_decay_s", 0.12),
            ("shaker_level", 0.55),
            ("shaker_pan", 0.55),
            ("cowbell_freq_a_hz", 560.0),
            ("cowbell_freq_b_hz", 845.0),
            ("cowbell_highpass_hz", 500.0),
            ("cowbell_lowpass_hz", 4500.0),
            ("cowbell_attack_s", 0.0005),
            ("cowbell_decay_s", 0.25),
            ("cowbell_level", 0.6),
            ("cowbell_pan", -0.2),
            ("crash_base_hz", 300.0),
            ("crash_highpass_hz", 6000.0),
            ("crash_lowpass_hz", 15000.0),
            ("crash_attack_s", 0.0005),
            ("crash_decay_s", 1.4),
            ("crash_level", 0.7),
            ("crash_pan", -0.5),
            ("ride_base_hz", 420.0),
            ("ride_highpass_hz", 7500.0),
            ("ride_lowpass_hz", 16000.0),
            ("ride_attack_s", 0.0005),
            ("ride_decay_s", 1.0),
            ("ride_level", 0.6),
            ("ride_pan", 0.55),
            // 独立 808 低音（期 4）：逐风格定值与 kick 同族分工。
            ("bass808_tune_hz", 49.0),
            ("bass808_glide_hz", 120.0),
            ("bass808_glide_s", 0.05),
            ("bass808_attack_s", 0.002),
            ("bass808_decay_s", 1.8),
            ("bass808_level", 0.95),
            ("bass808_pan", 0.0),
        ],
        "drill" => vec![
            ("kick_tune_hz", 47.0),
            ("kick_glide_hz", 170.0),
            ("kick_glide_s", 0.07),
            ("kick_attack_s", 0.0005),
            ("kick_decay_s", 0.85),
            ("kick_level", 1.0),
            ("kick_pan", 0.0),
            ("snare_tone_hz", 210.0),
            ("snare_tone_ratio", 1.8),
            ("snare_tone_decay_s", 0.05),
            ("snare_tone_level", 0.45),
            ("snare_noise_highpass_hz", 3000.0),
            ("snare_noise_lowpass_hz", 11000.0),
            ("snare_noise_decay_s", 0.055),
            ("snare_noise_level", 1.0),
            ("snare_attack_s", 0.0005),
            ("snare_level", 1.0),
            ("snare_pan", 0.0),
            ("hihat_base_hz", 330.0),
            ("hihat_highpass_hz", 8500.0),
            ("hihat_lowpass_hz", 16000.0),
            ("hihat_attack_s", 0.0005),
            ("hihat_closed_decay_s", 0.035),
            ("hihat_open_decay_s", 0.25),
            ("hihat_level", 0.6),
            ("hihat_pan", 0.1),
            ("clap_highpass_hz", 2000.0),
            ("clap_lowpass_hz", 6500.0),
            ("clap_bursts", 3.0),
            ("clap_burst_decay_s", 0.011),
            ("clap_burst_spacing_s", 0.008),
            ("clap_attack_s", 0.0005),
            ("clap_decay_s", 0.15),
            ("clap_level", 0.65),
            ("clap_pan", -0.2),
            ("tom_tune_hz", 95.0),
            ("tom_glide_hz", 140.0),
            ("tom_glide_s", 0.03),
            ("tom_attack_s", 0.0005),
            ("tom_decay_s", 0.3),
            ("tom_level", 0.75),
            ("tom_pan", 0.15),
            ("rim_tone_hz", 1900.0),
            ("rim_noise_highpass_hz", 1300.0),
            ("rim_noise_lowpass_hz", 6000.0),
            ("rim_decay_s", 0.04),
            ("rim_attack_s", 0.0005),
            ("rim_level", 0.75),
            ("rim_pan", 0.15),
            ("shaker_highpass_hz", 6500.0),
            ("shaker_lowpass_hz", 14500.0),
            ("shaker_attack_s", 0.007),
            ("shaker_decay_s", 0.1),
            ("shaker_level", 0.5),
            ("shaker_pan", 0.35),
            ("cowbell_freq_a_hz", 540.0),
            ("cowbell_freq_b_hz", 845.0),
            ("cowbell_highpass_hz", 500.0),
            ("cowbell_lowpass_hz", 4500.0),
            ("cowbell_attack_s", 0.0005),
            ("cowbell_decay_s", 0.22),
            ("cowbell_level", 0.55),
            ("cowbell_pan", -0.25),
            ("crash_base_hz", 300.0),
            ("crash_highpass_hz", 5200.0),
            ("crash_lowpass_hz", 14800.0),
            ("crash_attack_s", 0.0005),
            ("crash_decay_s", 1.2),
            ("crash_level", 0.65),
            ("crash_pan", -0.2),
            ("ride_base_hz", 420.0),
            ("ride_highpass_hz", 7200.0),
            ("ride_lowpass_hz", 15800.0),
            ("ride_attack_s", 0.0005),
            ("ride_decay_s", 0.9),
            ("ride_level", 0.55),
            ("ride_pan", 0.25),
            // 独立 808 低音（期 4）：逐风格定值与 kick 同族分工。
            // 2026-11-20 起带 808 道（Wang 立项）：drill 的 808 要滑音
            // 长尾——glide 拉长到 0.12 s、decay 拉长到 2.4 s（跨小节
            //  sustain），与 hiphop 的短而圆拉开差距。
            ("bass808_tune_hz", 49.0),
            ("bass808_glide_hz", 160.0),
            ("bass808_glide_s", 0.12),
            ("bass808_attack_s", 0.002),
            ("bass808_decay_s", 2.4),
            ("bass808_level", 0.95),
            ("bass808_pan", 0.0),
        ],
        "phonk" => vec![
            ("kick_tune_hz", 50.0),
            ("kick_glide_hz", 140.0),
            ("kick_glide_s", 0.04),
            ("kick_attack_s", 0.0005),
            ("kick_decay_s", 0.7),
            ("kick_level", 1.0),
            ("kick_pan", 0.0),
            ("snare_tone_hz", 180.0),
            ("snare_tone_ratio", 1.8),
            ("snare_tone_decay_s", 0.08),
            ("snare_tone_level", 0.5),
            ("snare_noise_highpass_hz", 1800.0),
            ("snare_noise_lowpass_hz", 8000.0),
            ("snare_noise_decay_s", 0.09),
            ("snare_noise_level", 1.0),
            ("snare_attack_s", 0.0005),
            ("snare_level", 0.95),
            ("snare_pan", 0.0),
            ("hihat_base_hz", 300.0),
            ("hihat_highpass_hz", 6500.0),
            ("hihat_lowpass_hz", 12000.0),
            ("hihat_attack_s", 0.0005),
            ("hihat_closed_decay_s", 0.05),
            ("hihat_open_decay_s", 0.35),
            ("hihat_level", 0.6),
            ("hihat_pan", 0.1),
            ("clap_highpass_hz", 1500.0),
            ("clap_lowpass_hz", 5500.0),
            ("clap_bursts", 4.0),
            ("clap_burst_decay_s", 0.011),
            ("clap_burst_spacing_s", 0.009),
            ("clap_attack_s", 0.0005),
            ("clap_decay_s", 0.22),
            ("clap_level", 0.75),
            ("clap_pan", -0.05),
            ("tom_tune_hz", 85.0),
            ("tom_glide_hz", 140.0),
            ("tom_glide_s", 0.03),
            ("tom_attack_s", 0.0005),
            ("tom_decay_s", 0.4),
            ("tom_level", 0.8),
            ("tom_pan", 0.15),
            ("rim_tone_hz", 1600.0),
            ("rim_noise_highpass_hz", 1300.0),
            ("rim_noise_lowpass_hz", 6000.0),
            ("rim_decay_s", 0.05),
            ("rim_attack_s", 0.0005),
            ("rim_level", 0.7),
            ("rim_pan", 0.25),
            ("shaker_highpass_hz", 5000.0),
            ("shaker_lowpass_hz", 11000.0),
            ("shaker_attack_s", 0.007),
            ("shaker_decay_s", 0.16),
            ("shaker_level", 0.6),
            ("shaker_pan", 0.25),
            ("cowbell_freq_a_hz", 540.0),
            ("cowbell_freq_b_hz", 800.0),
            ("cowbell_highpass_hz", 400.0),
            ("cowbell_lowpass_hz", 4000.0),
            ("cowbell_attack_s", 0.0005),
            ("cowbell_decay_s", 0.35),
            ("cowbell_level", 0.85),
            ("cowbell_pan", -0.3),
            ("crash_base_hz", 300.0),
            ("crash_highpass_hz", 4500.0),
            ("crash_lowpass_hz", 12000.0),
            ("crash_attack_s", 0.0005),
            ("crash_decay_s", 1.5),
            ("crash_level", 0.7),
            ("crash_pan", -0.3),
            ("ride_base_hz", 420.0),
            ("ride_highpass_hz", 6000.0),
            ("ride_lowpass_hz", 13000.0),
            ("ride_attack_s", 0.0005),
            ("ride_decay_s", 1.1),
            ("ride_level", 0.6),
            ("ride_pan", 0.55),
            // 独立 808 低音（期 4）：逐风格定值与 kick 同族分工。
            ("bass808_tune_hz", 46.0),
            ("bass808_glide_hz", 115.0),
            ("bass808_glide_s", 0.05),
            ("bass808_attack_s", 0.002),
            ("bass808_decay_s", 1.5),
            ("bass808_level", 0.95),
            ("bass808_pan", 0.0),
        ],
        "edm" => vec![
            ("kick_tune_hz", 50.0),
            ("kick_glide_hz", 160.0),
            ("kick_glide_s", 0.02),
            ("kick_attack_s", 0.0005),
            ("kick_decay_s", 0.3),
            ("kick_level", 1.0),
            ("kick_pan", 0.0),
            ("snare_tone_hz", 190.0),
            ("snare_tone_ratio", 1.784),
            ("snare_tone_decay_s", 0.1),
            ("snare_tone_level", 0.5),
            ("snare_noise_highpass_hz", 1800.0),
            ("snare_noise_lowpass_hz", 12000.0),
            ("snare_noise_decay_s", 0.09),
            ("snare_noise_level", 1.0),
            ("snare_attack_s", 0.0005),
            ("snare_level", 1.0),
            ("snare_pan", 0.0),
            ("hihat_base_hz", 340.0),
            ("hihat_highpass_hz", 7000.0),
            ("hihat_lowpass_hz", 15000.0),
            ("hihat_attack_s", 0.0005),
            ("hihat_closed_decay_s", 0.05),
            ("hihat_open_decay_s", 0.4),
            ("hihat_level", 0.7),
            ("hihat_pan", 0.2),
            ("clap_highpass_hz", 1500.0),
            ("clap_lowpass_hz", 6000.0),
            ("clap_bursts", 4.0),
            ("clap_burst_decay_s", 0.012),
            ("clap_burst_spacing_s", 0.01),
            ("clap_attack_s", 0.0005),
            ("clap_decay_s", 0.25),
            ("clap_level", 0.8),
            ("clap_pan", -0.15),
            ("tom_tune_hz", 100.0),
            ("tom_glide_hz", 150.0),
            ("tom_glide_s", 0.03),
            ("tom_attack_s", 0.0005),
            ("tom_decay_s", 0.28),
            ("tom_level", 0.9),
            ("tom_pan", 0.1),
            ("rim_tone_hz", 1700.0),
            ("rim_noise_highpass_hz", 1200.0),
            ("rim_noise_lowpass_hz", 6000.0),
            ("rim_decay_s", 0.05),
            ("rim_attack_s", 0.0005),
            ("rim_level", 0.8),
            ("rim_pan", -0.1),
            ("shaker_highpass_hz", 5000.0),
            ("shaker_lowpass_hz", 12000.0),
            ("shaker_attack_s", 0.008),
            ("shaker_decay_s", 0.14),
            ("shaker_level", 0.6),
            ("shaker_pan", 0.4),
            ("cowbell_freq_a_hz", 540.0),
            ("cowbell_freq_b_hz", 845.0),
            ("cowbell_highpass_hz", 500.0),
            ("cowbell_lowpass_hz", 4500.0),
            ("cowbell_attack_s", 0.0005),
            ("cowbell_decay_s", 0.28),
            ("cowbell_level", 0.65),
            ("cowbell_pan", -0.3),
            ("crash_base_hz", 300.0),
            ("crash_highpass_hz", 5000.0),
            ("crash_lowpass_hz", 15000.0),
            ("crash_attack_s", 0.0005),
            ("crash_decay_s", 1.6),
            ("crash_level", 0.8),
            ("crash_pan", -0.3),
            ("ride_base_hz", 420.0),
            ("ride_highpass_hz", 7000.0),
            ("ride_lowpass_hz", 16000.0),
            ("ride_attack_s", 0.0005),
            ("ride_decay_s", 1.1),
            ("ride_level", 0.7),
            ("ride_pan", 0.35),
            // 独立 808 低音（期 4）：逐风格定值与 kick 同族分工。
            ("bass808_tune_hz", 50.0),
            ("bass808_glide_hz", 100.0),
            ("bass808_glide_s", 0.04),
            ("bass808_attack_s", 0.002),
            ("bass808_decay_s", 0.8),
            ("bass808_level", 0.9),
            ("bass808_pan", 0.0),
        ],
        "house" => vec![
            ("kick_tune_hz", 50.0),
            ("kick_glide_hz", 160.0),
            ("kick_glide_s", 0.02),
            ("kick_attack_s", 0.0005),
            ("kick_decay_s", 0.3),
            ("kick_level", 1.0),
            ("kick_pan", 0.0),
            ("snare_tone_hz", 190.0),
            ("snare_tone_ratio", 1.784),
            ("snare_tone_decay_s", 0.1),
            ("snare_tone_level", 0.5),
            ("snare_noise_highpass_hz", 1800.0),
            ("snare_noise_lowpass_hz", 12000.0),
            ("snare_noise_decay_s", 0.09),
            ("snare_noise_level", 1.0),
            ("snare_attack_s", 0.0005),
            ("snare_level", 0.95),
            ("snare_pan", 0.0),
            ("hihat_base_hz", 340.0),
            ("hihat_highpass_hz", 7000.0),
            ("hihat_lowpass_hz", 15000.0),
            ("hihat_attack_s", 0.0005),
            ("hihat_closed_decay_s", 0.05),
            ("hihat_open_decay_s", 0.45),
            ("hihat_level", 0.7),
            ("hihat_pan", 0.15),
            ("clap_highpass_hz", 1500.0),
            ("clap_lowpass_hz", 6000.0),
            ("clap_bursts", 4.0),
            ("clap_burst_decay_s", 0.012),
            ("clap_burst_spacing_s", 0.01),
            ("clap_attack_s", 0.0005),
            ("clap_decay_s", 0.25),
            ("clap_level", 0.8),
            ("clap_pan", -0.1),
            ("tom_tune_hz", 105.0),
            ("tom_glide_hz", 140.0),
            ("tom_glide_s", 0.03),
            ("tom_attack_s", 0.0005),
            ("tom_decay_s", 0.26),
            ("tom_level", 0.85),
            ("tom_pan", 0.15),
            ("rim_tone_hz", 1750.0),
            ("rim_noise_highpass_hz", 1300.0),
            ("rim_noise_lowpass_hz", 6000.0),
            ("rim_decay_s", 0.05),
            ("rim_attack_s", 0.0005),
            ("rim_level", 0.85),
            ("rim_pan", -0.05),
            ("shaker_highpass_hz", 5500.0),
            ("shaker_lowpass_hz", 12500.0),
            ("shaker_attack_s", 0.007),
            ("shaker_decay_s", 0.13),
            ("shaker_level", 0.65),
            ("shaker_pan", 0.35),
            ("cowbell_freq_a_hz", 540.0),
            ("cowbell_freq_b_hz", 845.0),
            ("cowbell_highpass_hz", 500.0),
            ("cowbell_lowpass_hz", 4500.0),
            ("cowbell_attack_s", 0.0005),
            ("cowbell_decay_s", 0.28),
            ("cowbell_level", 0.65),
            ("cowbell_pan", 0.15),
            ("crash_base_hz", 300.0),
            ("crash_highpass_hz", 5200.0),
            ("crash_lowpass_hz", 14800.0),
            ("crash_attack_s", 0.0005),
            ("crash_decay_s", 1.5),
            ("crash_level", 0.75),
            ("crash_pan", -0.5),
            ("ride_base_hz", 420.0),
            ("ride_highpass_hz", 7200.0),
            ("ride_lowpass_hz", 15800.0),
            ("ride_attack_s", 0.0005),
            ("ride_decay_s", 1.0),
            ("ride_level", 0.65),
            ("ride_pan", 0.55),
            // 独立 808 低音（期 4）：逐风格定值与 kick 同族分工。
            ("bass808_tune_hz", 50.0),
            ("bass808_glide_hz", 100.0),
            ("bass808_glide_s", 0.04),
            ("bass808_attack_s", 0.002),
            ("bass808_decay_s", 1.0),
            ("bass808_level", 0.9),
            ("bass808_pan", 0.0),
        ],
        "techno" => vec![
            ("kick_tune_hz", 52.0),
            ("kick_glide_hz", 165.0),
            ("kick_glide_s", 0.018),
            ("kick_attack_s", 0.0005),
            ("kick_decay_s", 0.26),
            ("kick_level", 1.0),
            ("kick_pan", 0.0),
            ("snare_tone_hz", 195.0),
            ("snare_tone_ratio", 1.784),
            ("snare_tone_decay_s", 0.1),
            ("snare_tone_level", 0.5),
            ("snare_noise_highpass_hz", 2000.0),
            ("snare_noise_lowpass_hz", 12500.0),
            ("snare_noise_decay_s", 0.08),
            ("snare_noise_level", 1.0),
            ("snare_attack_s", 0.0005),
            ("snare_level", 1.0),
            ("snare_pan", 0.0),
            ("hihat_base_hz", 345.0),
            ("hihat_highpass_hz", 7500.0),
            ("hihat_lowpass_hz", 15500.0),
            ("hihat_attack_s", 0.0005),
            ("hihat_closed_decay_s", 0.045),
            ("hihat_open_decay_s", 0.35),
            ("hihat_level", 0.72),
            ("hihat_pan", 0.18),
            ("clap_highpass_hz", 1600.0),
            ("clap_lowpass_hz", 6200.0),
            ("clap_bursts", 4.0),
            ("clap_burst_decay_s", 0.012),
            ("clap_burst_spacing_s", 0.01),
            ("clap_attack_s", 0.0005),
            ("clap_decay_s", 0.22),
            ("clap_level", 0.8),
            ("clap_pan", -0.12),
            ("tom_tune_hz", 98.0),
            ("tom_glide_hz", 140.0),
            ("tom_glide_s", 0.03),
            ("tom_attack_s", 0.0005),
            ("tom_decay_s", 0.27),
            ("tom_level", 0.88),
            ("tom_pan", 0.15),
            ("rim_tone_hz", 1750.0),
            ("rim_noise_highpass_hz", 1300.0),
            ("rim_noise_lowpass_hz", 6000.0),
            ("rim_decay_s", 0.048),
            ("rim_attack_s", 0.0005),
            ("rim_level", 0.82),
            ("rim_pan", 0.25),
            ("shaker_highpass_hz", 5200.0),
            ("shaker_lowpass_hz", 12800.0),
            ("shaker_attack_s", 0.007),
            ("shaker_decay_s", 0.135),
            ("shaker_level", 0.62),
            ("shaker_pan", 0.38),
            ("cowbell_freq_a_hz", 540.0),
            ("cowbell_freq_b_hz", 845.0),
            ("cowbell_highpass_hz", 500.0),
            ("cowbell_lowpass_hz", 4500.0),
            ("cowbell_attack_s", 0.0005),
            ("cowbell_decay_s", 0.29),
            ("cowbell_level", 0.68),
            ("cowbell_pan", 0.18),
            ("crash_base_hz", 300.0),
            ("crash_highpass_hz", 5200.0),
            ("crash_lowpass_hz", 14800.0),
            ("crash_attack_s", 0.0005),
            ("crash_decay_s", 1.55),
            ("crash_level", 0.78),
            ("crash_pan", -0.28),
            ("ride_base_hz", 420.0),
            ("ride_highpass_hz", 7200.0),
            ("ride_lowpass_hz", 15800.0),
            ("ride_attack_s", 0.0005),
            ("ride_decay_s", 1.05),
            ("ride_level", 0.68),
            ("ride_pan", 0.32),
            // 独立 808 低音（期 4）：逐风格定值与 kick 同族分工。
            ("bass808_tune_hz", 50.0),
            ("bass808_glide_hz", 105.0),
            ("bass808_glide_s", 0.04),
            ("bass808_attack_s", 0.002),
            ("bass808_decay_s", 0.7),
            ("bass808_level", 0.85),
            ("bass808_pan", 0.0),
        ],
        "lofi" => vec![
            ("kick_tune_hz", 48.0),
            ("kick_glide_hz", 130.0),
            ("kick_glide_s", 0.04),
            ("kick_attack_s", 0.0005),
            ("kick_decay_s", 0.55),
            ("kick_level", 1.0),
            ("kick_pan", 0.0),
            ("snare_tone_hz", 175.0),
            ("snare_tone_ratio", 1.8),
            ("snare_tone_decay_s", 0.08),
            ("snare_tone_level", 0.55),
            ("snare_noise_highpass_hz", 1500.0),
            ("snare_noise_lowpass_hz", 7000.0),
            ("snare_noise_decay_s", 0.1),
            ("snare_noise_level", 1.0),
            ("snare_attack_s", 0.0005),
            ("snare_level", 0.95),
            ("snare_pan", 0.0),
            ("hihat_base_hz", 290.0),
            ("hihat_highpass_hz", 5500.0),
            ("hihat_lowpass_hz", 10000.0),
            ("hihat_attack_s", 0.0005),
            ("hihat_closed_decay_s", 0.055),
            ("hihat_open_decay_s", 0.32),
            ("hihat_level", 0.55),
            ("hihat_pan", 0.08),
            ("clap_highpass_hz", 1300.0),
            ("clap_lowpass_hz", 5000.0),
            ("clap_bursts", 3.0),
            ("clap_burst_decay_s", 0.011),
            ("clap_burst_spacing_s", 0.009),
            ("clap_attack_s", 0.0005),
            ("clap_decay_s", 0.24),
            ("clap_level", 0.7),
            ("clap_pan", -0.05),
            ("tom_tune_hz", 88.0),
            ("tom_glide_hz", 140.0),
            ("tom_glide_s", 0.03),
            ("tom_attack_s", 0.0005),
            ("tom_decay_s", 0.38),
            ("tom_level", 0.78),
            ("tom_pan", 0.15),
            ("rim_tone_hz", 1500.0),
            ("rim_noise_highpass_hz", 1000.0),
            ("rim_noise_lowpass_hz", 5000.0),
            ("rim_decay_s", 0.055),
            ("rim_attack_s", 0.0005),
            ("rim_level", 0.72),
            ("rim_pan", 0.25),
            ("shaker_highpass_hz", 4500.0),
            ("shaker_lowpass_hz", 9500.0),
            ("shaker_attack_s", 0.007),
            ("shaker_decay_s", 0.17),
            ("shaker_level", 0.58),
            ("shaker_pan", 0.2),
            ("cowbell_freq_a_hz", 520.0),
            ("cowbell_freq_b_hz", 780.0),
            ("cowbell_highpass_hz", 350.0),
            ("cowbell_lowpass_hz", 3800.0),
            ("cowbell_attack_s", 0.0005),
            ("cowbell_decay_s", 0.34),
            ("cowbell_level", 0.7),
            ("cowbell_pan", -0.1),
            ("crash_base_hz", 300.0),
            ("crash_highpass_hz", 4000.0),
            ("crash_lowpass_hz", 10000.0),
            ("crash_attack_s", 0.0005),
            ("crash_decay_s", 1.3),
            ("crash_level", 0.62),
            ("crash_pan", -0.2),
            ("ride_base_hz", 420.0),
            ("ride_highpass_hz", 5500.0),
            ("ride_lowpass_hz", 11000.0),
            ("ride_attack_s", 0.0005),
            ("ride_decay_s", 0.95),
            ("ride_level", 0.55),
            ("ride_pan", 0.22),
            // 独立 808 低音（期 4）：逐风格定值与 kick 同族分工。
            ("bass808_tune_hz", 45.0),
            ("bass808_glide_hz", 90.0),
            ("bass808_glide_s", 0.08),
            ("bass808_attack_s", 0.002),
            ("bass808_decay_s", 1.1),
            ("bass808_level", 0.8),
            ("bass808_pan", 0.0),
        ],
        "hiphop" => vec![
            ("kick_tune_hz", 46.0),
            ("kick_glide_hz", 145.0),
            ("kick_glide_s", 0.045),
            ("kick_attack_s", 0.0005),
            ("kick_decay_s", 0.8),
            ("kick_level", 1.0),
            ("kick_pan", 0.0),
            ("snare_tone_hz", 185.0),
            ("snare_tone_ratio", 1.8),
            ("snare_tone_decay_s", 0.08),
            ("snare_tone_level", 0.5),
            ("snare_noise_highpass_hz", 2000.0),
            ("snare_noise_lowpass_hz", 10000.0),
            ("snare_noise_decay_s", 0.085),
            ("snare_noise_level", 1.0),
            ("snare_attack_s", 0.0005),
            ("snare_level", 1.0),
            ("snare_pan", 0.0),
            ("hihat_base_hz", 310.0),
            ("hihat_highpass_hz", 7000.0),
            ("hihat_lowpass_hz", 13000.0),
            ("hihat_attack_s", 0.0005),
            ("hihat_closed_decay_s", 0.045),
            ("hihat_open_decay_s", 0.35),
            ("hihat_level", 0.62),
            ("hihat_pan", 0.3),
            ("clap_highpass_hz", 1700.0),
            ("clap_lowpass_hz", 6500.0),
            ("clap_bursts", 4.0),
            ("clap_burst_decay_s", 0.011),
            ("clap_burst_spacing_s", 0.009),
            ("clap_attack_s", 0.0005),
            ("clap_decay_s", 0.2),
            ("clap_level", 0.8),
            ("clap_pan", -0.2),
            ("tom_tune_hz", 92.0),
            ("tom_glide_hz", 140.0),
            ("tom_glide_s", 0.03),
            ("tom_attack_s", 0.0005),
            ("tom_decay_s", 0.33),
            ("tom_level", 0.8),
            ("tom_pan", 0.15),
            ("rim_tone_hz", 1750.0),
            ("rim_noise_highpass_hz", 1300.0),
            ("rim_noise_lowpass_hz", 6000.0),
            ("rim_decay_s", 0.045),
            ("rim_attack_s", 0.0005),
            ("rim_level", 0.8),
            ("rim_pan", 0.08),
            ("shaker_highpass_hz", 5500.0),
            ("shaker_lowpass_hz", 12000.0),
            ("shaker_attack_s", 0.007),
            ("shaker_decay_s", 0.13),
            ("shaker_level", 0.55),
            ("shaker_pan", 0.28),
            ("cowbell_freq_a_hz", 540.0),
            ("cowbell_freq_b_hz", 845.0),
            ("cowbell_highpass_hz", 500.0),
            ("cowbell_lowpass_hz", 4500.0),
            ("cowbell_attack_s", 0.0005),
            ("cowbell_decay_s", 0.27),
            ("cowbell_level", 0.6),
            ("cowbell_pan", -0.18),
            ("crash_base_hz", 300.0),
            ("crash_highpass_hz", 5200.0),
            ("crash_lowpass_hz", 14800.0),
            ("crash_attack_s", 0.0005),
            ("crash_decay_s", 1.45),
            ("crash_level", 0.72),
            ("crash_pan", -0.22),
            ("ride_base_hz", 420.0),
            ("ride_highpass_hz", 7200.0),
            ("ride_lowpass_hz", 15800.0),
            ("ride_attack_s", 0.0005),
            ("ride_decay_s", 1.05),
            ("ride_level", 0.62),
            ("ride_pan", 0.28),
            // 独立 808 低音（期 4）：逐风格定值与 kick 同族分工。
            // 2026-11-20 起带 808 道（Wang 立项）：hiphop 的 808 要
            // 短而圆——decay 收进 0.85 s、glide 压到 0.02 s（几乎不
            // 滑），与 drill 的滑音长尾拉开差距。
            ("bass808_tune_hz", 49.0),
            ("bass808_glide_hz", 95.0),
            ("bass808_glide_s", 0.02),
            ("bass808_attack_s", 0.002),
            ("bass808_decay_s", 0.85),
            ("bass808_level", 0.95),
            ("bass808_pan", 0.0),
        ],
        _ => vec![
            ("kick_tune_hz", 50.0),
            ("kick_glide_hz", 160.0),
            ("kick_glide_s", 0.02),
            ("kick_attack_s", 0.0005),
            ("kick_decay_s", 0.3),
            ("kick_level", 1.0),
            ("kick_pan", 0.0),
            ("snare_tone_hz", 190.0),
            ("snare_tone_ratio", 1.784),
            ("snare_tone_decay_s", 0.1),
            ("snare_tone_level", 0.5),
            ("snare_noise_highpass_hz", 1800.0),
            ("snare_noise_lowpass_hz", 12000.0),
            ("snare_noise_decay_s", 0.09),
            ("snare_noise_level", 1.0),
            ("snare_attack_s", 0.0005),
            ("snare_level", 1.0),
            ("snare_pan", 0.0),
            ("hihat_base_hz", 340.0),
            ("hihat_highpass_hz", 7000.0),
            ("hihat_lowpass_hz", 15000.0),
            ("hihat_attack_s", 0.0005),
            ("hihat_closed_decay_s", 0.05),
            ("hihat_open_decay_s", 0.4),
            ("hihat_level", 0.7),
            ("hihat_pan", 0.2),
            ("clap_highpass_hz", 1500.0),
            ("clap_lowpass_hz", 6000.0),
            ("clap_bursts", 4.0),
            ("clap_burst_decay_s", 0.012),
            ("clap_burst_spacing_s", 0.01),
            ("clap_attack_s", 0.0005),
            ("clap_decay_s", 0.25),
            ("clap_level", 0.8),
            ("clap_pan", -0.15),
            ("tom_tune_hz", 100.0),
            ("tom_glide_hz", 150.0),
            ("tom_glide_s", 0.03),
            ("tom_attack_s", 0.0005),
            ("tom_decay_s", 0.28),
            ("tom_level", 0.9),
            ("tom_pan", 0.1),
            ("rim_tone_hz", 1700.0),
            ("rim_noise_highpass_hz", 1200.0),
            ("rim_noise_lowpass_hz", 6000.0),
            ("rim_decay_s", 0.05),
            ("rim_attack_s", 0.0005),
            ("rim_level", 0.8),
            ("rim_pan", -0.1),
            ("shaker_highpass_hz", 5000.0),
            ("shaker_lowpass_hz", 12000.0),
            ("shaker_attack_s", 0.008),
            ("shaker_decay_s", 0.14),
            ("shaker_level", 0.6),
            ("shaker_pan", 0.4),
            ("cowbell_freq_a_hz", 540.0),
            ("cowbell_freq_b_hz", 845.0),
            ("cowbell_highpass_hz", 500.0),
            ("cowbell_lowpass_hz", 4500.0),
            ("cowbell_attack_s", 0.0005),
            ("cowbell_decay_s", 0.28),
            ("cowbell_level", 0.65),
            ("cowbell_pan", -0.3),
            ("crash_base_hz", 300.0),
            ("crash_highpass_hz", 5000.0),
            ("crash_lowpass_hz", 15000.0),
            ("crash_attack_s", 0.0005),
            ("crash_decay_s", 1.6),
            ("crash_level", 0.8),
            ("crash_pan", -0.3),
            ("ride_base_hz", 420.0),
            ("ride_highpass_hz", 7000.0),
            ("ride_lowpass_hz", 16000.0),
            ("ride_attack_s", 0.0005),
            ("ride_decay_s", 1.1),
            ("ride_level", 0.7),
            ("ride_pan", 0.35),
            // 独立 808 低音（期 4）：逐风格定值与 kick 同族分工。
            ("bass808_tune_hz", 50.0),
            ("bass808_glide_hz", 100.0),
            ("bass808_glide_s", 0.04),
            ("bass808_attack_s", 0.002),
            ("bass808_decay_s", 0.9),
            ("bass808_level", 0.9),
            ("bass808_pan", 0.0),
        ],
    }
}

// ---------------------------------------------------------------------------
// 击点覆盖审计（段 5 守卫：绑到不存在音色的击点必须响亮失败，不许无声）
// ---------------------------------------------------------------------------

/// 原声默认鼓组（`01-basic-kit.sfz` 的 `keymap_basic.sfz` 键位图）里
/// **有 region 落点**的鼓件键位。出处：`Programs/keymaps/keymap_basic.sfz`
/// 的 `#define` 键位表（鼓件部分）+ 2026-11-18 全库装载审计逐键核对
/// （出厂预置用到的每个键都有样本发声）。⚠ 其它 Virtuosity 程序用
/// 另一张键位图、覆盖更小，逐程序修正见 [`acoustic_covered_keys`]。
/// 扩展打击乐键（54–84 的非鼓件部分）**不**列入：它们在部分单麦
/// 程序里没有 region，列入会让守卫对没声音的键放行 —— 宁可将来
/// 真用到时守卫响亮报错、人工复核后再扩表，也不让表先行放宽。
pub const ACOUSTIC_KIT_KEYS: &[u8] = &[
    35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 53, 55, 57, 59, 82,
];

/// 本预置的 pattern（主道 + fill）里**有击点但绑不到任何音色**的鼓道：
/// 返回 `(role, gm_key)` 清单，空 = 全部击点有声。
///
/// - 电子族：音色集 = [`kit_setup`] 的合成鼓机参数表里全部 `*_note`
///   键位（引擎鼓机识别口径的同一份名单，⛔ 不另抄一份）；
/// - 原声族：音色集 = [`ACOUSTIC_KIT_KEYS`] 按预置选定的 SFZ 程序
///   修正（见 [`acoustic_covered_keys`]：完整键位图程序缺 46 等键）。
///
/// 生成路径（MCP `yeban_generate_drum_pattern` 与 UI `drum_gen_ops`）
/// 在生成前调用本函数，非空即响亮拒绝 —— 这是 Wang「默认不能静默」
/// 规则在鼓机上的落点：一期电子族 shaker 道（键 82）整道无声的事故
/// 不许重演。
#[must_use]
pub fn uncovered_pattern_keys(preset: &DrumStylePreset) -> Vec<(String, u8)> {
    let covered: Vec<u8> = match preset.kit_family {
        KitFamily::Electronic => match kit_setup(preset) {
            DrumKitSetup::SynthDevice { params, .. } => params
                .iter()
                .filter(|(name, _)| name.ends_with("_note"))
                .map(|(_, value)| *value as u8)
                .collect(),
            DrumKitSetup::AcousticSfz { .. } => Vec::new(),
        },
        KitFamily::Acoustic => acoustic_covered_keys(preset.sfz_path.as_deref()),
    };
    let mut uncovered = Vec::new();
    let mut check = |lanes: &[DrumLanePreset]| {
        for lane in lanes {
            if lane.steps.iter().any(|step| step.is_hit()) && !covered.contains(&lane.gm_key) {
                uncovered.push((lane.role.clone(), lane.gm_key));
            }
            // 叠层键同样发声：本层力度窗内有击点且键无音色覆盖
            // ⇒ 与主键同口径点名，不许叠层整层静默无声。
            for layer in &lane.layers {
                if lane
                    .steps
                    .iter()
                    .any(|step| step.is_hit() && layer.fires_on(step.velocity))
                    && !covered.contains(&layer.gm_key)
                {
                    uncovered.push((lane.role.clone(), layer.gm_key));
                }
            }
        }
    };
    check(&preset.lanes);
    if let Some(fill) = &preset.fill_lanes {
        check(fill);
    }
    uncovered
}

/// 原声族在**指定 SFZ 程序**下的实际覆盖键集（[`uncovered_pattern_keys`]
/// 的原声分支）。
///
/// ⚠ Virtuosity 的程序并不共用一张键位图（2026-11-18 逐程序装载
/// 探针实测，探针口径 = note 触发 region 的键区间覆盖计数）：
/// `01-basic-kit.sfz` 用 `keymap_basic.sfz`（GM 分键，开镲独立在
/// 46）；其余出厂程序用 `keymap.sfz`——踩镲是 CC4 踏板连续乐器、
/// 全部落在 42，**46 没有任何 region**；`08-vintage-mic.sfz` 与
/// `03-kick-mic.sfz` 还缺 56（牛铃）与 82（沙锤）。未知程序按
/// basic 全集计（出厂预置只用已探针核过的程序；用户自选程序的
/// 装载期覆盖审计是另一条线，见预置格式文档）。
fn acoustic_covered_keys(sfz_path: Option<&str>) -> Vec<u8> {
    let mut keys = ACOUSTIC_KIT_KEYS.to_vec();
    let Some(path) = sfz_path else {
        return keys;
    };
    let file = path.rsplit('/').next().unwrap_or(path);
    if file != "01-basic-kit.sfz" {
        // 完整键位图程序：开镲不在 46（并入 42 + CC4 踏板）。
        keys.retain(|key| *key != 46);
    }
    if matches!(file, "08-vintage-mic.sfz" | "03-kick-mic.sfz") {
        keys.retain(|key| !matches!(key, 56 | 82));
    }
    keys
}

// ---------------------------------------------------------------------------
// 逐件链生成校验（choke 组不拆分；生成路径共用）
// ---------------------------------------------------------------------------

/// 原声鼓组在**指定 SFZ 程序**下的 choke（exclusive / off_by）互斥组：
/// 每组内的键互掐——一个键发声会掐掉组内其它键的声部。出处：
/// 2026-11-18 用 `yeban-sfz` 装载四个出厂程序、按 region 的
/// `group`/`off_by` 建无向互掐连通分量实测（与 [`acoustic_covered_keys`]
/// 同一「静态表 + 出处留痕」口径：`yeban-theory` 不依赖 SFZ 解析，
/// 探针在装载侧跑、结论钉成表）。`01-basic-kit.sfz` 的帽组是
/// {42, 44, 46}；其余出厂程序（`keymap.sfz` 系）开镲并入 42、另有
/// 92 号「掐音键」与 42/44 同组，帽组为 {42, 44, 92}。未知程序按
/// 非 basic 键位图计（与覆盖表的缺省口径一致）。
#[must_use]
pub fn choke_groups(sfz_path: Option<&str>) -> Vec<Vec<u8>> {
    let tail = vec![vec![62, 63], vec![71, 72], vec![80, 81]];
    let file = sfz_path.map(|path| path.rsplit('/').next().unwrap_or(path));
    let hats: Vec<u8> = match file {
        None | Some("01-basic-kit.sfz") => vec![42, 44, 46],
        Some(_) => vec![42, 44, 92],
    };
    let mut groups = vec![hats];
    groups.extend(tail);
    groups
}

/// 逐件链的**生成期**违规清单（空 = 可生成）。生成路径（MCP
/// `yeban_generate_drum_pattern` 与 UI `drum_gen_ops`）在覆盖审计之后
/// 调用本函数，非空即响亮拒绝并逐条点名——与
/// [`uncovered_pattern_keys`] 同一「默认不能静默」口径。一类违规：
///
/// 1. **choke 组被拆分**（仅原声族）：同一 choke 组的键不得分布在
///    两条不同的逐件链里，也不得一部分分组、一部分留在整轨链上——
///    逐件渲染按组分引擎，拆开后互掐关系断裂（踩镲掐不开镲的语义
///    被静默改掉）。整组同链或整组不分组才合法。口径与引擎装配侧
///    的分区校验一致：按**程序的全部 region 键**计，不只 pattern
///    用到的键——出厂预置曾只按 pattern 键过检（01-basic-kit 的
///    踩镲 44 不在 pattern 里），引擎按 region 分区响亮拒绝渲染，
///    六个原声风格一度全部渲不出（2026-11-20 重渲实测）。
#[must_use]
pub fn lane_chain_generation_violations(preset: &DrumStylePreset) -> Vec<String> {
    let mut violations = Vec::new();
    // 侧链声明已由期 4 落成生成路径的 lane 级侧链边，不再是违规
    // （期 4 之前在此响亮拒绝，历史见 git）。
    if preset.kit_family != KitFamily::Acoustic || preset.lane_chains.is_empty() {
        return violations;
    }
    for component in choke_groups(preset.sfz_path.as_deref()) {
        // 这组 choke 键落在哪几个分区里：每条逐件链是一个分区，
        // 未被任何逐件链认领的键同属「未分组」分区。按程序的全部
        // region 键计（静态表即程序口径），不按 pattern 用到的键
        // 过滤——与引擎装配侧的分区校验同口径。
        let mut cells: Vec<String> = Vec::new();
        let mut any_grouped = false;
        let mut any_ungrouped = false;
        for key in &component {
            match preset
                .lane_chains
                .iter()
                .find(|chain| chain.keys.contains(key))
            {
                Some(chain) => {
                    any_grouped = true;
                    if !cells.contains(&chain.name) {
                        cells.push(chain.name.clone());
                    }
                }
                None => any_ungrouped = true,
            }
        }
        if !any_grouped {
            continue;
        }
        if any_ungrouped {
            cells.push("未分组（整轨链）".to_owned());
        }
        if cells.len() > 1 {
            violations.push(format!(
                "choke 组 {component:?} 被拆开（分布在 {}）：同一 choke 组的键必须同属一条逐件链或整组不分组，拆开会破坏掐音语义",
                cells.join(" / ")
            ));
        }
    }
    violations
}

// ---------------------------------------------------------------------------
// 出厂预置（嵌入文本与磁盘文件同一格式）
// ---------------------------------------------------------------------------

/// 出厂 10 风格的嵌入源（id 与文件一一对应）。
const FACTORY_SOURCES: [(&str, &str); 14] = [
    ("trap", include_str!("../drum_styles/trap.json")),
    ("drill", include_str!("../drum_styles/drill.json")),
    ("phonk", include_str!("../drum_styles/phonk.json")),
    ("lofi", include_str!("../drum_styles/lofi.json")),
    ("boombap", include_str!("../drum_styles/boombap.json")),
    ("house", include_str!("../drum_styles/house.json")),
    ("techno", include_str!("../drum_styles/techno.json")),
    ("edm", include_str!("../drum_styles/edm.json")),
    ("pop", include_str!("../drum_styles/pop.json")),
    ("rock", include_str!("../drum_styles/rock.json")),
    ("jazz", include_str!("../drum_styles/jazz.json")),
    ("funk", include_str!("../drum_styles/funk.json")),
    ("rnb", include_str!("../drum_styles/rnb.json")),
    ("hiphop", include_str!("../drum_styles/hiphop.json")),
];

/// 出厂风格 id 清单（顺序即 UI 列表顺序）。
#[must_use]
pub fn factory_drum_style_ids() -> Vec<&'static str> {
    FACTORY_SOURCES.iter().map(|(id, _)| *id).collect()
}

/// 全部出厂预置（每次调用重新解析；文本小，无缓存必要）。
///
/// # Errors
///
/// 任一嵌入文本非法 ⇒ [`DrumPresetError`]（出厂数据被判据钉住，
/// 正常构建下不会发生）。
pub fn factory_drum_styles() -> Result<Vec<DrumStylePreset>, DrumPresetError> {
    FACTORY_SOURCES
        .iter()
        .map(|(_, text)| DrumStylePreset::from_json(text))
        .collect()
}

/// 按 id 取一份出厂预置；id 不存在返回 `Ok(None)`。
///
/// # Errors
///
/// id 存在但嵌入文本非法 ⇒ [`DrumPresetError`]。
pub fn factory_drum_style(id: &str) -> Result<Option<DrumStylePreset>, DrumPresetError> {
    for (known, text) in FACTORY_SOURCES {
        if known == id {
            return DrumStylePreset::from_json(text).map(Some);
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// 严格子集 JSON 解析（对象/数组/字符串/数字/布尔/null）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(pairs) => Some(pairs),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }
}

fn get_opt<'a>(obj: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    obj.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn get<'a>(obj: &'a [(String, Json)], key: &str) -> Result<&'a Json, DrumPresetError> {
    get_opt(obj, key).ok_or_else(|| field_err(key, "missing required field"))
}

fn get_str<'a>(obj: &'a [(String, Json)], key: &str) -> Result<&'a str, DrumPresetError> {
    match get(obj, key)? {
        Json::Str(s) => Ok(s),
        _ => Err(field_err(key, "expected a string")),
    }
}

fn get_u64(obj: &[(String, Json)], key: &str) -> Result<u64, DrumPresetError> {
    match get(obj, key)? {
        Json::Num(n) => as_u64_num(*n, key),
        _ => Err(field_err(key, "expected a number")),
    }
}

fn get_u16(obj: &[(String, Json)], key: &str) -> Result<u16, DrumPresetError> {
    let value = get_u64(obj, key)?;
    u16::try_from(value).map_err(|_| field_err(key, format!("value {value} out of u16 range")))
}

fn get_obj<'a>(
    obj: &'a [(String, Json)],
    key: &str,
) -> Result<&'a [(String, Json)], DrumPresetError> {
    match get(obj, key)? {
        Json::Obj(pairs) => Ok(pairs),
        _ => Err(field_err(key, "expected an object")),
    }
}

fn get_arr<'a>(obj: &'a [(String, Json)], key: &str) -> Result<&'a [Json], DrumPresetError> {
    match get(obj, key)? {
        Json::Arr(items) => Ok(items),
        _ => Err(field_err(key, "expected an array")),
    }
}

fn get_str_list(obj: &[(String, Json)], key: &str) -> Result<Vec<String>, DrumPresetError> {
    let items = get_arr(obj, key)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Json::Str(s) => out.push(s.clone()),
            _ => return Err(field_err(key, "expected an array of strings")),
        }
    }
    Ok(out)
}

fn as_u64_num(n: f64, field: &str) -> Result<u64, DrumPresetError> {
    if !n.is_finite() || n < 0.0 || n.fract() != 0.0 || n > u64::MAX as f64 {
        return Err(field_err(
            field,
            format!("expected a non-negative integer, got {n}"),
        ));
    }
    Ok(n as u64)
}

fn as_u8(value: &Json, field: &str) -> Result<u8, DrumPresetError> {
    match value {
        Json::Num(n) => {
            let v = as_u64_num(*n, field)?;
            u8::try_from(v).map_err(|_| field_err(field, format!("value {v} out of u8 range")))
        }
        _ => Err(field_err(field, "expected a number")),
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

fn parse_json(text: &str) -> Result<Json, DrumPresetError> {
    let mut parser = JsonParser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    parser.skip_ws();
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(parser.err("trailing characters after top-level value"));
    }
    Ok(value)
}

impl JsonParser<'_> {
    fn err(&self, message: &str) -> DrumPresetError {
        DrumPresetError::Parse {
            offset: self.pos,
            message: message.to_owned(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), DrumPresetError> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", byte as char)))
        }
    }

    fn parse_value(&mut self) -> Result<Json, DrumPresetError> {
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(Json::Str(self.parse_string()?)),
            Some(b't') => self.parse_lit("true", Json::Bool(true)),
            Some(b'f') => self.parse_lit("false", Json::Bool(false)),
            Some(b'n') => self.parse_lit("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            _ => Err(self.err("unexpected character")),
        }
    }

    fn parse_lit(&mut self, lit: &str, value: Json) -> Result<Json, DrumPresetError> {
        if self.bytes[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            Ok(value)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn parse_object(&mut self) -> Result<Json, DrumPresetError> {
        self.expect(b'{')?;
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Obj(pairs));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.parse_value()?;
            pairs.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Obj(pairs));
                }
                _ => return Err(self.err("expected `,` or `}` in object")),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Json, DrumPresetError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            self.skip_ws();
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(self.err("expected `,` or `]` in array")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, DrumPresetError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self.peek().ok_or_else(|| self.err("unterminated string"))?;
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let esc = self.peek().ok_or_else(|| self.err("unterminated escape"))?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let cp = self.parse_hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                // 高代理项：必须跟 \uXXXX 低代理项。
                                if self.peek() == Some(b'\\') {
                                    self.pos += 1;
                                    self.expect(b'u')?;
                                    let low = self.parse_hex4()?;
                                    if !(0xDC00..0xE000).contains(&low) {
                                        return Err(self.err("invalid low surrogate"));
                                    }
                                    let c = 0x1_0000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                                    out.push(
                                        char::from_u32(c)
                                            .ok_or_else(|| self.err("invalid code point"))?,
                                    );
                                } else {
                                    return Err(self.err("lone high surrogate"));
                                }
                            } else if (0xDC00..0xE000).contains(&cp) {
                                return Err(self.err("lone low surrogate"));
                            } else {
                                out.push(
                                    char::from_u32(cp)
                                        .ok_or_else(|| self.err("invalid code point"))?,
                                );
                            }
                        }
                        _ => return Err(self.err("invalid escape")),
                    }
                }
                0x00..=0x1F => return Err(self.err("control character in string")),
                _ => {
                    // 多字节 UTF-8：按 char 边界推进（输入是 &str，恒合法）。
                    let rest = &self.bytes[self.pos..];
                    let s = std::str::from_utf8(rest)
                        .map_err(|_| self.err("invalid UTF-8 in string"))?;
                    let ch = s.chars().next().ok_or_else(|| self.err("bad char"))?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, DrumPresetError> {
        let mut value: u32 = 0;
        for _ in 0..4 {
            let byte = self.peek().ok_or_else(|| self.err("bad \\u escape"))?;
            let digit = match byte {
                b'0'..=b'9' => u32::from(byte - b'0'),
                b'a'..=b'f' => u32::from(byte - b'a') + 10,
                b'A'..=b'F' => u32::from(byte - b'A') + 10,
                _ => return Err(self.err("bad hex digit in \\u escape")),
            };
            value = value * 16 + digit;
            self.pos += 1;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Json, DrumPresetError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.err("invalid number"))?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| self.err("invalid number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "format": "yeban-drum-style", "version": 1,
        "id": "trap", "name": "Trap", "name_zh": "陷阱 Trap",
        "source": {"kind": "groove-adapted", "groove_genre_id": "edm-trap",
                   "repo": "gradetwo/groove", "license": "MIT",
                   "converted": "2026-11-17", "tuning": ["expanded to 2 bars"]},
        "kit_family": "electronic",
        "bpm": {"default": 140, "min": 130, "max": 150},
        "swing_permille": 515, "bars": 1,
        "lanes": [{"role": "kick", "gm_key": 36, "steps": [
            [120,1,0],[0,1,0],[0,1,0],[0,1,0],[0,1,0],[0,1,0],[0,1,0],[0,1,0],
            [0,1,0],[0,1,0],[105,1,1],[0,1,0],[0,1,0],[80,1,1],[0,1,0],[0,1,0]
        ]}]
    }"#;

    #[test]
    fn parses_minimal_preset() {
        let preset = DrumStylePreset::from_json(MINIMAL).expect("valid preset");
        assert_eq!(preset.id, "trap");
        assert_eq!(preset.kit_family, KitFamily::Electronic);
        assert_eq!(preset.lanes.len(), 1);
        assert_eq!(preset.lanes[0].steps.len(), 16);
        assert!(preset.lanes[0].steps[0].is_hit());
        assert_eq!(preset.lanes[0].steps[10].tier, 1);
        assert!(preset.fill_lanes.is_none());
        assert!(preset.fx.is_none(), "no fx section means dry generation");
    }

    #[test]
    fn parses_lane_layers_with_defaults() {
        let text = MINIMAL.replace(
            "\"gm_key\": 36,",
            "\"gm_key\": 36, \"layers\": [{\"gm_key\": 39}, \
             {\"gm_key\": 37, \"level_db\": -6.0, \"vel_min\": 40, \"vel_max\": 100}],",
        );
        let preset = DrumStylePreset::from_json(&text).expect("valid layered preset");
        let layers = &preset.lanes[0].layers;
        assert_eq!(layers.len(), 2);
        // 缺省：电平 0 dB、力度窗全开。
        assert_eq!(layers[0].gm_key, 39);
        assert_eq!(layers[0].level_db, 0.0);
        assert_eq!((layers[0].vel_min, layers[0].vel_max), (1, 127));
        assert_eq!(layers[1].gm_key, 37);
        assert_eq!(layers[1].level_db, -6.0);
        assert_eq!((layers[1].vel_min, layers[1].vel_max), (40, 100));
        // 力度窗含两端、窗外不触发。
        assert!(layers[1].fires_on(40) && layers[1].fires_on(100));
        assert!(!layers[1].fires_on(39) && !layers[1].fires_on(101));
    }

    #[test]
    fn rejects_invalid_layers_loudly() {
        let with_layers = |layers: &str| {
            MINIMAL.replace(
                "\"gm_key\": 36,",
                &format!("\"gm_key\": 36, \"layers\": {layers},"),
            )
        };
        // 空数组：单键道应省略字段，不许写空段。
        assert!(DrumStylePreset::from_json(&with_layers("[]")).is_err());
        // 叠层键 == 本道主键。
        assert!(DrumStylePreset::from_json(&with_layers("[{\"gm_key\": 36}]")).is_err());
        // 同一道内叠层键重复。
        assert!(
            DrumStylePreset::from_json(&with_layers("[{\"gm_key\": 39}, {\"gm_key\": 39}]"))
                .is_err()
        );
        // 电平越界（两端）。
        assert!(
            DrumStylePreset::from_json(&with_layers("[{\"gm_key\": 39, \"level_db\": 12.5}]"))
                .is_err()
        );
        assert!(
            DrumStylePreset::from_json(&with_layers("[{\"gm_key\": 39, \"level_db\": -48.5}]"))
                .is_err()
        );
        // 力度窗颠倒 / 越界（0 是休止、128 不是 MIDI 力度）。
        assert!(
            DrumStylePreset::from_json(&with_layers(
                "[{\"gm_key\": 39, \"vel_min\": 90, \"vel_max\": 40}]"
            ))
            .is_err()
        );
        assert!(
            DrumStylePreset::from_json(&with_layers("[{\"gm_key\": 39, \"vel_min\": 0}]")).is_err()
        );
        assert!(
            DrumStylePreset::from_json(&with_layers("[{\"gm_key\": 39, \"vel_max\": 128}]"))
                .is_err()
        );
        // 形状错误：layers 非数组、层非对象。
        assert!(DrumStylePreset::from_json(&with_layers("{}")).is_err());
        assert!(DrumStylePreset::from_json(&with_layers("[39]")).is_err());
    }

    #[test]
    fn rejects_layer_keys_colliding_across_lanes() {
        // 叠层键撞另一道的主键（两个先后方向都拒）或撞另一道的
        // 叠层键，都必须响亮失败；不撞的组合放行。
        let mk = |lanes: &str| {
            format!(
                r#"{{
                    "format": "yeban-drum-style", "version": 1,
                    "id": "x", "name": "X", "name_zh": "X",
                    "source": {{"kind": "test", "groove_genre_id": "x",
                               "repo": "t", "license": "MIT",
                               "converted": "2026-11-19", "tuning": []}},
                    "kit_family": "electronic",
                    "bpm": {{"default": 90, "min": 80, "max": 100}},
                    "swing_permille": 500, "bars": 1,
                    "lanes": [{lanes}]
                }}"#
            )
        };
        let plain = |role: &str, key: u8| {
            format!("{{\"role\": \"{role}\", \"gm_key\": {key}, \"steps\": {STEPS16}}}")
        };
        let layered = |role: &str, key: u8, layers: &str| {
            format!(
                "{{\"role\": \"{role}\", \"gm_key\": {key}, \"layers\": {layers}, \"steps\": {STEPS16}}}"
            )
        };
        // 先行道叠层 39，后行道主键 39 ⇒ 拒。
        assert!(
            DrumStylePreset::from_json(&mk(&format!(
                "{}, {}",
                layered("snare", 38, "[{\"gm_key\": 39}]"),
                plain("clap", 39)
            )))
            .is_err()
        );
        // 先行道主键 39 在前，后行道叠层 39 ⇒ 拒。
        assert!(
            DrumStylePreset::from_json(&mk(&format!(
                "{}, {}",
                plain("clap", 39),
                layered("snare", 38, "[{\"gm_key\": 39}]")
            )))
            .is_err()
        );
        // 两道叠层键互相撞（都叠 40）⇒ 拒。
        assert!(
            DrumStylePreset::from_json(&mk(&format!(
                "{}, {}",
                layered("snare", 38, "[{\"gm_key\": 40}]"),
                layered("clap", 39, "[{\"gm_key\": 40}]")
            )))
            .is_err()
        );
        // 不撞的叠层（38 叠 37、39 叠 40）⇒ 通过。
        assert!(
            DrumStylePreset::from_json(&mk(&format!(
                "{}, {}",
                layered("snare", 38, "[{\"gm_key\": 37}]"),
                layered("clap", 39, "[{\"gm_key\": 40}]")
            )))
            .is_ok()
        );
    }

    #[test]
    fn uncovered_pattern_keys_includes_firing_layers() {
        // 电子族合成键位表没有 60：叠层键无覆盖且力度窗内有击点
        // ⇒ 与主键同口径被点名；力度窗避开全部写力度 ⇒ 不点名。
        let firing = MINIMAL.replace(
            "\"gm_key\": 36,",
            "\"gm_key\": 36, \"layers\": [{\"gm_key\": 60}],",
        );
        let preset = DrumStylePreset::from_json(&firing).expect("valid layered preset");
        assert!(uncovered_pattern_keys(&preset).contains(&("kick".to_owned(), 60)));
        let windowed_out = MINIMAL.replace(
            "\"gm_key\": 36,",
            "\"gm_key\": 36, \"layers\": [{\"gm_key\": 60, \"vel_min\": 121}],",
        );
        let preset = DrumStylePreset::from_json(&windowed_out).expect("valid layered preset");
        assert!(!uncovered_pattern_keys(&preset).contains(&("kick".to_owned(), 60)));
    }

    #[test]
    fn electronic_factory_styles_trim_the_drum_bus_for_headroom() {
        let styles = factory_drum_styles().expect("factory presets parse");
        let mut electronic = 0;
        for style in &styles {
            let levels = factory_drum_levels(style);
            assert_eq!(levels.track_volume_db, -6.0, "{} 鼓轨电平", style.id);
            if style.kit_family == KitFamily::Electronic {
                electronic += 1;
                assert!(
                    levels.bus_volume_db <= -6.0,
                    "{} 鼓总线至少 −6 dB 才能给 0 dBFS 留余量，实际 {}",
                    style.id,
                    levels.bus_volume_db
                );
            } else {
                assert_eq!(
                    levels.bus_volume_db, 0.0,
                    "{} 是原声族，总线电平必须保持旧值",
                    style.id
                );
            }
        }
        assert_eq!(electronic, 8, "电子族出厂风格数");
    }

    #[test]
    fn factory_styles_parse_and_only_hiphop_carries_layers() {
        let styles = factory_drum_styles().expect("factory presets parse");
        assert_eq!(styles.len(), 14);
        for style in &styles {
            let mut layered: Vec<(&str, u8, u8)> = Vec::new();
            for lane in style.lanes.iter().chain(style.fill_lanes.iter().flatten()) {
                for layer in &lane.layers {
                    layered.push((style.id.as_str(), lane.gm_key, layer.gm_key));
                }
            }
            if style.id == "hiphop" {
                // 主道与 fill 道各一条：军鼓(38) 叠 clap(39)。
                assert_eq!(layered, vec![("hiphop", 38, 39), ("hiphop", 38, 39)]);
                let snare = style
                    .lanes
                    .iter()
                    .find(|lane| lane.gm_key == 38)
                    .expect("hiphop snare lane");
                assert!((snare.layers[0].level_db - -3.2).abs() < 1e-6);
            } else {
                assert!(
                    layered.is_empty(),
                    "{} must not gain layers in this change: {layered:?}",
                    style.id
                );
            }
        }
    }

    const MINIMAL_FX: &str = r#""fx": {
        "eq": {"low_db": 2.0, "mid_db": -1.0, "high_db": 1.5, "low_hz": 200,
               "mid_hz": 1000, "mid_q": 0.9, "high_hz": 4200, "hpf_hz": 30,
               "cutoff_hz": 20000},
        "comp": {"threshold_db": -16, "ratio": 3, "knee_db": 6,
                 "attack_s": 0.01, "release_s": 0.12, "makeup_db": 1.5},
        "output_db": 0.5,
        "reverb": {"kind": "plate", "decay_s": 1.1, "damp": 0.45, "wet": 0.15, "predelay_s": 0.01},
        "tape": {"drive": 0.3, "mix": 0.5, "hf_hz": 9000, "asym": 0.2, "wow_rate_hz": 0.5, "wow_depth_ms": 0.0, "flutter_rate_hz": 6.0, "flutter_depth_ms": 0.0, "seed": 0},
        "delay": {"time_s": 0.28, "feedback": 0.25, "mix": 0.1, "damp": 0.4,
                  "ping_pong": true}
    }"#;

    #[test]
    fn parses_fx_section_with_tape_and_delay() {
        let text = MINIMAL.replace("\"bars\": 1,", &format!("\"bars\": 1, {MINIMAL_FX},"));
        let preset = DrumStylePreset::from_json(&text).expect("valid fx preset");
        let fx = preset.fx.expect("fx parsed");
        assert_eq!(fx.eq_low_db, 2.0);
        assert_eq!(fx.eq_mid_hz, 1000.0);
        assert_eq!(fx.hpf_hz, 30.0);
        assert_eq!(fx.cutoff_hz, 20000.0);
        assert_eq!(fx.comp_threshold_db, -16.0);
        assert_eq!(fx.comp_ratio, 3.0);
        assert_eq!(fx.output_db, 0.5);
        assert_eq!(fx.reverb_wet, 0.15);
        assert_eq!(fx.reverb_kind, DrumReverbKind::Plate);
        assert_eq!(fx.reverb_decay_s, 1.1);
        let tape = fx.tape.expect("tape parsed");
        assert_eq!(tape.drive, 0.3);
        assert_eq!(tape.asym, 0.2);
        let delay = fx.delay.expect("delay parsed");
        assert_eq!(delay.time_s, 0.28);
        assert!(delay.ping_pong);
    }

    #[test]
    fn rejects_out_of_range_or_incomplete_fx() {
        let base = MINIMAL.replace("\"bars\": 1,", &format!("\"bars\": 1, {MINIMAL_FX},"));
        let wet_out = base.replace("\"wet\": 0.15", "\"wet\": 1.5");
        assert!(
            DrumStylePreset::from_json(&wet_out).is_err(),
            "reverb wet above 1 must be rejected"
        );
        let missing_knee = base.replace("\"knee_db\": 6,", "");
        assert!(
            DrumStylePreset::from_json(&missing_knee).is_err(),
            "a missing comp field must be rejected"
        );
        let bad_ratio = base.replace("\"ratio\": 3", "\"ratio\": 0.5");
        assert!(
            DrumStylePreset::from_json(&bad_ratio).is_err(),
            "comp ratio below 1 must be rejected"
        );
        let bad_kind = base.replace("\"kind\": \"plate\"", "\"kind\": \"cathedral\"");
        assert!(
            DrumStylePreset::from_json(&bad_kind).is_err(),
            "unknown reverb kind must be rejected"
        );
        let bad_decay = base.replace("\"decay_s\": 1.1", "\"decay_s\": 42");
        assert!(
            DrumStylePreset::from_json(&bad_decay).is_err(),
            "reverb decay_s above 10 must be rejected"
        );
    }

    #[test]
    fn factory_styles_all_carry_individually_tuned_fx() {
        let styles = factory_drum_styles().expect("factory presets parse");
        assert_eq!(styles.len(), 14);
        let mut signatures = std::collections::BTreeSet::new();
        for style in &styles {
            let fx = style
                .fx
                .unwrap_or_else(|| panic!("factory style {} must carry an fx section", style.id));
            signatures.insert((
                fx.reverb_wet.to_bits(),
                fx.comp_threshold_db.to_bits(),
                fx.eq_low_db.to_bits(),
            ));
        }
        assert!(
            signatures.len() > 1,
            "factory fx must be tuned per style, not one shared parameter set"
        );
    }

    /// P1/P2 落点钉：14 风格的混响类型与混响时间逐条按分配表钉死，
    /// Room/Plate/Hall/Spring 四档都有归属；且生成路径真写出
    /// `reverb_kind`/`reverb_decay_s` 两个器件参数、不再写 `reverb_size`。
    #[test]
    fn factory_styles_pin_their_reverb_kind_and_decay() {
        let styles = factory_drum_styles().expect("factory presets parse");
        let expected: [(&str, DrumReverbKind, f32); 14] = [
            ("boombap", DrumReverbKind::Room, 0.9),
            ("drill", DrumReverbKind::Room, 0.7),
            ("edm", DrumReverbKind::Hall, 1.4),
            ("funk", DrumReverbKind::Room, 0.5),
            ("hiphop", DrumReverbKind::Room, 0.9),
            ("house", DrumReverbKind::Plate, 1.1),
            ("jazz", DrumReverbKind::Room, 1.0),
            ("lofi", DrumReverbKind::Spring, 1.0),
            ("phonk", DrumReverbKind::Hall, 1.3),
            ("pop", DrumReverbKind::Plate, 1.35),
            ("rnb", DrumReverbKind::Hall, 2.2),
            ("rock", DrumReverbKind::Hall, 2.0),
            ("techno", DrumReverbKind::Room, 0.8),
            ("trap", DrumReverbKind::Room, 0.8),
        ];
        let mut seen_kinds = std::collections::BTreeSet::new();
        for (id, kind, decay_s) in expected {
            let style = styles
                .iter()
                .find(|s| s.id == id)
                .unwrap_or_else(|| panic!("factory style {id} missing"));
            let fx = style
                .fx
                .unwrap_or_else(|| panic!("factory style {id} must carry an fx section"));
            assert_eq!(fx.reverb_kind, kind, "{id} reverb kind");
            assert_eq!(fx.reverb_decay_s, decay_s, "{id} reverb decay_s");
            seen_kinds.insert(kind.name());
            let params = reverb_params(&fx.reverb_view());
            assert!(
                params.contains(&("reverb_kind".to_owned(), kind.device_value())),
                "{id} generation must write reverb_kind"
            );
            assert!(
                params.contains(&("reverb_decay_s".to_owned(), decay_s)),
                "{id} generation must write reverb_decay_s"
            );
            assert!(
                !params.iter().any(|(n, _)| n == "reverb_size"),
                "{id} generation must not write reverb_size anymore"
            );
        }
        assert_eq!(
            seen_kinds.len(),
            4,
            "all four reverb kinds must be represented: {seen_kinds:?}"
        );
    }

    /// 段 5 守卫钉子（Wang「默认不能静默」）：14 个出厂预置的全部
    /// 击点（主道 + fill）都必须绑得到音色——`uncovered_pattern_keys`
    /// 对每一份出厂预置都为空。
    #[test]
    fn every_factory_preset_hit_has_a_voice() {
        let styles = factory_drum_styles().expect("factory presets parse");
        assert_eq!(styles.len(), 14);
        for style in &styles {
            let uncovered = uncovered_pattern_keys(style);
            assert!(
                uncovered.is_empty(),
                "{} 有绑不到音色的击点：{uncovered:?}",
                style.id
            );
        }
    }

    /// 守卫的反面钉子：绑到不存在音色的击点必须被报出来（电子族
    /// 未知键 99；原声族在完整键位图程序上用开镲键 46）。
    #[test]
    fn the_guard_names_hits_bound_to_missing_voices() {
        let electronic = MINIMAL.replace("\"gm_key\": 36", "\"gm_key\": 99");
        let preset = DrumStylePreset::from_json(&electronic).expect("parses");
        assert_eq!(
            uncovered_pattern_keys(&preset),
            vec![("kick".to_owned(), 99)],
            "电子族键 99 无音色必须被点名"
        );

        let acoustic = MINIMAL
            .replace(
                "\"kit_family\": \"electronic\"",
                "\"kit_family\": \"acoustic\"",
            )
            .replace(
                "\"swing_permille\": 515, \"bars\": 1,",
                "\"swing_permille\": 515, \"bars\": 1, \
                 \"sfz_path\": \"assets/samples/virtuosity-drums/Programs/07-room-mic.sfz\",",
            )
            .replace("\"gm_key\": 36", "\"gm_key\": 46");
        let preset = DrumStylePreset::from_json(&acoustic).expect("parses");
        assert_eq!(
            uncovered_pattern_keys(&preset),
            vec![("kick".to_owned(), 46)],
            "完整键位图程序（07-room-mic）上键 46 无 region 必须被点名"
        );
    }

    /// 段 1 钉子：14 个出厂预置全部带鼓总线段，且胶水压缩落在
    /// Wang 定值带内（比 2:1–4:1、启动 30 ms）。
    #[test]
    fn every_factory_preset_carries_a_drum_bus_in_the_ruled_band() {
        let styles = factory_drum_styles().expect("factory presets parse");
        for style in &styles {
            let bus = style
                .bus
                .unwrap_or_else(|| panic!("factory style {} must carry a bus section", style.id));
            assert!(
                (2.0..=4.0).contains(&bus.glue.ratio),
                "{} 胶水压缩比 {} 不在 2:1–4:1 带内",
                style.id,
                bus.glue.ratio
            );
            assert!(
                (bus.glue.attack_s - 0.03).abs() < 1e-6,
                "{} 胶水启动 {} 不是 30 ms",
                style.id,
                bus.glue.attack_s
            );
            let specs = drum_bus_device_specs(&bus);
            assert_eq!(specs.len(), 2, "{} 总线链必须是胶水 + 削波两台", style.id);
        }
    }

    /// 段 2 钉子：电子族逐风格参数表两两不同（非键位参数至少差
    /// 8 项）——不许再出现"trap 与 drill 音色差异为 0"。
    #[test]
    fn electronic_factory_styles_have_pairwise_distinct_synth_tables() {
        let styles = factory_drum_styles().expect("factory presets parse");
        let electronic: Vec<&DrumStylePreset> = styles
            .iter()
            .filter(|style| style.kit_family == KitFamily::Electronic)
            .collect();
        assert_eq!(electronic.len(), 8, "电子族应为 8 个风格");
        let tables: Vec<Vec<(String, f32)>> = electronic
            .iter()
            .map(|style| match kit_setup(style) {
                DrumKitSetup::SynthDevice { params, .. } => params
                    .into_iter()
                    .filter(|(name, _)| !name.ends_with("_note"))
                    .collect(),
                DrumKitSetup::AcousticSfz { .. } => panic!("electronic style must be synth"),
            })
            .collect();
        for (i, a) in tables.iter().enumerate() {
            for (j, b) in tables.iter().enumerate().skip(i + 1) {
                let differing = a
                    .iter()
                    .zip(b.iter())
                    .filter(|((an, av), (bn, bv))| an == bn && av.to_bits() != bv.to_bits())
                    .count();
                assert!(
                    differing >= 8,
                    "{} 与 {} 的合成参数只差 {differing} 项（下限 8）",
                    electronic[i].id,
                    electronic[j].id
                );
            }
        }
    }

    /// 段 3 钉子：原声族的程序分配与命名——新风格用各自的麦克风
    /// 视角程序（ pairwise 不同），Acoustic 前缀齐全。
    #[test]
    fn acoustic_factory_styles_carry_acoustic_names_and_chosen_programs() {
        let styles = factory_drum_styles().expect("factory presets parse");
        let acoustic: Vec<&DrumStylePreset> = styles
            .iter()
            .filter(|style| style.kit_family == KitFamily::Acoustic)
            .collect();
        assert_eq!(acoustic.len(), 6, "原声族应为 6 个风格");
        for style in &acoustic {
            assert!(
                style.name.starts_with("Acoustic "),
                "{} 的英文名 {} 缺 Acoustic 前缀",
                style.id,
                style.name
            );
            match kit_setup(style) {
                DrumKitSetup::AcousticSfz { .. } => {}
                DrumKitSetup::SynthDevice { .. } => panic!("acoustic style must be SFZ"),
            }
        }
        let path_of = |id: &str| match kit_setup(acoustic.iter().find(|s| s.id == id).unwrap()) {
            DrumKitSetup::AcousticSfz { sfz_path } => sfz_path,
            DrumKitSetup::SynthDevice { .. } => unreachable!(),
        };
        assert!(path_of("jazz").ends_with("06-mid-mic.sfz"));
        assert!(path_of("funk").ends_with("04-snare-mic.sfz"));
        assert!(path_of("rnb").ends_with("05-oh-mic.sfz"));
        assert!(path_of("pop").ends_with("01-basic-kit.sfz"));
    }

    #[test]
    fn rejects_bad_format_and_bad_steps() {
        assert!(DrumStylePreset::from_json("{}").is_err());
        let bad_steps = MINIMAL.replace("[120,1,0]", "[300,1,0]");
        assert!(DrumStylePreset::from_json(&bad_steps).is_err());
        let bad_swing = MINIMAL.replace("515", "1200");
        assert!(DrumStylePreset::from_json(&bad_swing).is_err());
    }

    #[test]
    fn parser_handles_escapes_and_unicode() {
        let value = parse_json(r#"{"a": "x\n☃\ud83d\ude00", "b": [1, -2.5e2, true, null]}"#)
            .expect("valid json");
        let obj = value.as_object().expect("object");
        assert_eq!(get_str(obj, "a").expect("a"), "x\n☃😀");
    }

    // -----------------------------------------------------------------
    // 逐件链（lane_chains）判据（逐件处理线期 2）
    // -----------------------------------------------------------------

    /// 16 步全击点步进格（测试夹具）。
    const STEPS16: &str = "[[100,1,0],[100,1,0],[100,1,0],[100,1,0],\
        [100,1,0],[100,1,0],[100,1,0],[100,1,0],\
        [100,1,0],[100,1,0],[100,1,0],[100,1,0],\
        [100,1,0],[100,1,0],[100,1,0],[100,1,0]]";

    /// 带多道与可选段的预置夹具：`family` 为 `electronic` / `acoustic`，
    /// `extra` 是插在顶层 lanes 之后的原始 JSON 片段（如 lane_chains 段）。
    fn preset_with_lanes(family: &str, lane_keys: &[u8], extra: &str) -> String {
        let lanes: Vec<String> = lane_keys
            .iter()
            .map(|key| {
                format!("{{\"role\": \"lane{key}\", \"gm_key\": {key}, \"steps\": {STEPS16}}}")
            })
            .collect();
        format!(
            "{{\
                \"format\": \"yeban-drum-style\", \"version\": 1,\
                \"id\": \"fixture\", \"name\": \"Fixture\", \"name_zh\": \"夹具\",\
                \"source\": {{\"kind\": \"constructed\", \"groove_genre_id\": \"none\",\
                           \"tuning\": []}},\
                \"kit_family\": \"{family}\",\
                \"bpm\": {{\"default\": 120, \"min\": 100, \"max\": 140}},\
                \"swing_permille\": 500, \"bars\": 1,\
                \"lanes\": [{}]{extra}\
            }}",
            lanes.join(", ")
        )
    }

    const HATS_EQ: &str = "\"eq\": {\"low_db\": 0, \"mid_db\": 0, \"high_db\": 2,\
        \"low_hz\": 200, \"mid_hz\": 1000, \"mid_q\": 0.9, \"high_hz\": 6000,\
        \"hpf_hz\": 400, \"cutoff_hz\": 20000}";

    #[test]
    fn lane_chains_absent_means_empty_and_present_parses() {
        let plain = DrumStylePreset::from_json(&preset_with_lanes("electronic", &[36, 42], ""))
            .expect("valid preset");
        assert!(plain.lane_chains.is_empty(), "无 lane_chains 段 = 空表");

        let text = preset_with_lanes(
            "electronic",
            &[36, 42, 46],
            &format!(
                ", \"lane_chains\": [{{\"name\": \"hats\", \"keys\": [42, 46], \
                 \"chain\": {{{HATS_EQ}, \"pan\": -0.2}}}}]"
            ),
        );
        let preset = DrumStylePreset::from_json(&text).expect("valid lane chains");
        assert_eq!(preset.lane_chains.len(), 1);
        let chain = &preset.lane_chains[0];
        assert_eq!(chain.name, "hats");
        assert_eq!(chain.keys, vec![42, 46]);
        assert_eq!(chain.pan, Some(-0.2));
        assert!(chain.sidechain.is_none());
        let eq = chain.eq.expect("eq parsed");
        assert_eq!(eq.hpf_hz, 400.0);
        assert_eq!(eq.high_db, 2.0);
        // 器件配方：一台通道条，带 EQ 与高通，无压缩段、无 output。
        let specs = drum_lane_device_specs(chain);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].family, "strip");
        let names: Vec<&str> = specs[0].params.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"eq_enabled"));
        assert!(names.contains(&"hpf_hz"));
        assert!(
            !names.contains(&"compressor_enabled"),
            "无 comp 段不得拼压缩"
        );
        assert!(!names.contains(&"output_gain_db"), "逐件链无 output 微调");
    }

    #[test]
    fn lane_chain_gain_db_parses_and_rejects_loudly() {
        // 只带 gain_db 的链合法（纯增益组不算空链），值原样落预置。
        let text = preset_with_lanes(
            "acoustic",
            &[36, 42],
            ", \"lane_chains\": [{\"name\": \"kick\", \"keys\": [36], \"chain\": {\"gain_db\": -1.5}}]",
        );
        let preset = DrumStylePreset::from_json(&text).expect("纯增益组必须可解析");
        assert_eq!(preset.lane_chains.len(), 1);
        assert_eq!(preset.lane_chains[0].gain_db, Some(-1.5));
        // 增益不是器件：器件配方仍为空。
        assert!(drum_lane_device_specs(&preset.lane_chains[0]).is_empty());
        // 闭区间端点接受。
        for edge in [-60.0, 24.0] {
            let text = preset_with_lanes(
                "acoustic",
                &[36],
                &format!(
                    ", \"lane_chains\": [{{\"name\": \"kick\", \"keys\": [36], \"chain\": {{\"gain_db\": {edge}}}}}]"
                ),
            );
            let preset = DrumStylePreset::from_json(&text).expect("端点增益必须可解析");
            assert_eq!(preset.lane_chains[0].gain_db, Some(edge as f32));
        }
        // 超界响亮拒绝（不钳位）。
        for bad in ["24.5", "-60.5", "96"] {
            let text = preset_with_lanes(
                "acoustic",
                &[36],
                &format!(
                    ", \"lane_chains\": [{{\"name\": \"kick\", \"keys\": [36], \"chain\": {{\"gain_db\": {bad}}}}}]"
                ),
            );
            let err = DrumStylePreset::from_json(&text).expect_err("越界增益必须被拒");
            assert!(
                err.to_string().contains("gain_db"),
                "错误必须点名 gain_db: {err}"
            );
        }
    }

    #[test]
    fn lane_device_specs_cover_all_families_in_order() {
        let text = preset_with_lanes(
            "electronic",
            &[36],
            ", \"lane_chains\": [{\"name\": \"kick\", \"keys\": [36], \"chain\": {\
                \"eq\": {\"low_db\": 1, \"mid_db\": -2, \"high_db\": -6, \"low_hz\": 120,\
                        \"mid_hz\": 400, \"mid_q\": 0.8, \"high_hz\": 3000,\
                        \"hpf_hz\": 0, \"cutoff_hz\": 4500},\
                \"comp\": {\"threshold_db\": -18, \"ratio\": 3, \"knee_db\": 6,\
                          \"attack_s\": 0.01, \"release_s\": 0.12, \"makeup_db\": 1},\
                \"reverb\": {\"kind\": \"room\", \"decay_s\": 0.8, \"damp\": 0.5, \"wet\": 0.1, \"predelay_s\": 0.01},\
                \"tape\": {\"drive\": 0.2, \"mix\": 0.4, \"hf_hz\": 8000, \"asym\": 0.1, \"wow_rate_hz\": 0.5, \"wow_depth_ms\": 0.0, \"flutter_rate_hz\": 6.0, \"flutter_depth_ms\": 0.0, \"seed\": 0},\
                \"delay\": {\"time_s\": 0.25, \"feedback\": 0.2, \"mix\": 0.08,\
                           \"damp\": 0.3, \"ping_pong\": false}\
            }}]",
        );
        let preset = DrumStylePreset::from_json(&text).expect("valid lane chains");
        let specs = drum_lane_device_specs(&preset.lane_chains[0]);
        let families: Vec<&str> = specs.iter().map(|spec| spec.family).collect();
        assert_eq!(families, vec!["strip", "tape", "reverb", "delay"]);
        // strip = EQ 参数在前、压缩参数在后，同一台器件。
        let strip_names: Vec<&str> = specs[0].params.iter().map(|(n, _)| n.as_str()).collect();
        let eq_pos = strip_names.iter().position(|n| *n == "eq_enabled").unwrap();
        let comp_pos = strip_names
            .iter()
            .position(|n| *n == "compressor_enabled")
            .unwrap();
        assert!(eq_pos < comp_pos);
        assert!(
            strip_names.contains(&"cutoff_hz"),
            "cutoff < 20000 必须拼低通"
        );
    }

    #[test]
    fn lane_chains_reject_bad_shapes() {
        let base = |chains: &str| {
            preset_with_lanes(
                "electronic",
                &[36, 42, 46],
                &format!(", \"lane_chains\": {chains}"),
            )
        };
        let cases: Vec<String> = vec![
            // 键组为空。
            base("[{\"name\": \"hats\", \"keys\": [], \"chain\": {\"pan\": 0}}]"),
            // 键组未升序。
            base("[{\"name\": \"hats\", \"keys\": [46, 42], \"chain\": {\"pan\": 0}}]"),
            // 键不是本预置的鼓道键。
            base("[{\"name\": \"toms\", \"keys\": [43], \"chain\": {\"pan\": 0}}]"),
            // 两组抢同一个键。
            base(
                "[{\"name\": \"a\", \"keys\": [36, 42], \"chain\": {\"pan\": 0}},\
                 {\"name\": \"b\", \"keys\": [42], \"chain\": {\"pan\": 0}}]",
            ),
            // 组序未按首键升序。
            base(
                "[{\"name\": \"b\", \"keys\": [46], \"chain\": {\"pan\": 0}},\
                 {\"name\": \"a\", \"keys\": [42], \"chain\": {\"pan\": 0}}]",
            ),
            // 空链（什么处理都不带）。
            base("[{\"name\": \"hats\", \"keys\": [42], \"chain\": {}}]"),
            // EQ 值越界（与 fx 同一值域：low_db 只许 −18..=18）。
            base(
                "[{\"name\": \"hats\", \"keys\": [42], \"chain\": {\
                 \"eq\": {\"low_db\": 99, \"mid_db\": 0, \"high_db\": 0, \"low_hz\": 200,\
                         \"mid_hz\": 1000, \"mid_q\": 0.9, \"high_hz\": 6000,\
                         \"hpf_hz\": 0, \"cutoff_hz\": 20000}}}]",
            ),
            // pan 越界。
            base("[{\"name\": \"hats\", \"keys\": [42], \"chain\": {\"pan\": 2}}]"),
            // 侧链 key 键不是鼓道键。
            base(
                "[{\"name\": \"kick\", \"keys\": [36], \"chain\": {\
                 \"comp\": {\"threshold_db\": -18, \"ratio\": 3, \"knee_db\": 6,\
                           \"attack_s\": 0.01, \"release_s\": 0.12, \"makeup_db\": 1},\
                 \"sidechain\": {\"key_keys\": [43]}}}]",
            ),
            // 侧链 key 键未升序。
            base(
                "[{\"name\": \"kick\", \"keys\": [36], \"chain\": {\
                 \"comp\": {\"threshold_db\": -18, \"ratio\": 3, \"knee_db\": 6,\
                           \"attack_s\": 0.01, \"release_s\": 0.12, \"makeup_db\": 1},\
                 \"sidechain\": {\"key_keys\": [46, 42]}}}]",
            ),
        ];
        for (idx, text) in cases.iter().enumerate() {
            assert!(
                DrumStylePreset::from_json(text).is_err(),
                "用例 {idx} 必须被拒绝: {text}"
            );
        }
        // 组数超上限：组数检查在逐组解析之前，任何 17 组数组都必须被拒。
        let many = (0..17)
            .map(|i| format!("{{\"name\": \"g{i}\", \"keys\": [36], \"chain\": {{\"pan\": 0}}}}"))
            .collect::<Vec<_>>()
            .join(", ");
        assert!(DrumStylePreset::from_json(&base(&format!("[{many}]"))).is_err());
    }

    #[test]
    fn lane_chain_generation_violations_cover_sidechain_and_choke() {
        // 侧链声明：格式可解析，期 4 起生成落边、不再违规。
        let sidechained = DrumStylePreset::from_json(&preset_with_lanes(
            "electronic",
            &[36, 42],
            ", \"lane_chains\": [{\"name\": \"kick\", \"keys\": [36], \"chain\": {\
                \"comp\": {\"threshold_db\": -18, \"ratio\": 3, \"knee_db\": 6,\
                          \"attack_s\": 0.01, \"release_s\": 0.12, \"makeup_db\": 1},\
                \"sidechain\": {\"key_keys\": [36]}}}]",
        ))
        .expect("sidechain declaration parses");
        assert!(
            lane_chain_generation_violations(&sidechained).is_empty(),
            "期 4 起侧链声明落边生成，不再是违规"
        );

        // 原声 basic：帽组 {42,44,46}，预置用到 42/46。只把 42 分组 ⇒
        // 46 留在整轨链上，choke 组被拆 ⇒ 违规。
        let split = DrumStylePreset::from_json(&preset_with_lanes(
            "acoustic",
            &[36, 42, 46],
            ", \"lane_chains\": [{\"name\": \"closed\", \"keys\": [42], \"chain\": {\"pan\": 0}}]",
        ))
        .expect("parses");
        let violations = lane_chain_generation_violations(&split);
        assert_eq!(violations.len(), 1, "拆 choke 组必须违规: {violations:?}");
        assert!(violations[0].contains("choke"));

        // 帽组三键同链 ⇒ 合法。44 不是鼓道键，但它是 42/46 的 choke
        // 同组键，解析期可被同一条链认领（引擎按 region 分区）。
        let whole = DrumStylePreset::from_json(&preset_with_lanes(
            "acoustic",
            &[36, 42, 46],
            ", \"lane_chains\": [{\"name\": \"hats\", \"keys\": [42, 44, 46], \"chain\": {\"pan\": 0}}]",
        ))
        .expect("parses");
        assert!(lane_chain_generation_violations(&whole).is_empty());

        // 42/46 同链但漏 44 ⇒ 仍是拆组（出厂预置 2026-11-20 实测的
        // 失败形态：44 不在 pattern 里，旧口径只看 pattern 键漏检，
        // 引擎按 region 分区拒绝渲染）。
        let missing_mate = DrumStylePreset::from_json(&preset_with_lanes(
            "acoustic",
            &[36, 42, 46],
            ", \"lane_chains\": [{\"name\": \"hats\", \"keys\": [42, 46], \"chain\": {\"pan\": 0}}]",
        ))
        .expect("parses");
        let violations = lane_chain_generation_violations(&missing_mate);
        assert_eq!(
            violations.len(),
            1,
            "漏 choke 同组键必须违规: {violations:?}"
        );
        assert!(violations[0].contains("choke"));

        // 电子族不查 choke：同样的拆法合法。
        let electronic_split = DrumStylePreset::from_json(&preset_with_lanes(
            "electronic",
            &[36, 42, 46],
            ", \"lane_chains\": [{\"name\": \"closed\", \"keys\": [42], \"chain\": {\"pan\": 0}}]",
        ))
        .expect("parses");
        assert!(lane_chain_generation_violations(&electronic_split).is_empty());
    }

    #[test]
    fn choke_groups_match_probed_programs() {
        let basic = choke_groups(Some(
            "assets/samples/virtuosity-drums/Programs/01-basic-kit.sfz",
        ));
        assert!(basic.contains(&vec![42, 44, 46]));
        let other = choke_groups(Some(
            "assets/samples/virtuosity-drums/Programs/06-mid-mic.sfz",
        ));
        assert!(other.contains(&vec![42, 44, 92]));
        // 缺省（无 sfz_path = 默认基础组）按 basic 计。
        assert!(choke_groups(None).contains(&vec![42, 44, 46]));
    }

    #[test]
    fn factory_lane_chain_opt_ins_are_present_and_generatable() {
        let styles = factory_drum_styles().expect("factory presets parse");
        for style in &styles {
            assert!(
                lane_chain_generation_violations(style).is_empty(),
                "出厂风格 {} 的逐件链必须可生成",
                style.id
            );
        }
        let by_id = |id: &str| styles.iter().find(|s| s.id == id).unwrap();
        // house：帽组 [42, 46] 一条链；lofi：kick [36]。
        let house = by_id("house");
        assert_eq!(house.lane_chains.len(), 1);
        assert_eq!(house.lane_chains[0].name, "hats");
        assert_eq!(house.lane_chains[0].keys, vec![42, 46]);
        let lofi = by_id("lofi");
        assert_eq!(lofi.lane_chains.len(), 1);
        assert_eq!(lofi.lane_chains[0].keys, vec![36]);
        // trap / phonk（期 5 出厂侧链）：bass[35] 压缩 + kick 键控、kick[36]
        // 独立成组（键必须有组认领）、hats 高通组。组序按首键升序。
        for (id, hats_keys) in [("trap", vec![42]), ("phonk", vec![42, 46])] {
            let style = by_id(id);
            assert_eq!(style.lane_chains.len(), 3, "{id} 须有 bass/kick/hats 三组");
            assert_eq!(style.lane_chains[0].name, "bass");
            assert_eq!(style.lane_chains[0].keys, vec![35]);
            let bass_comp = style.lane_chains[0]
                .comp
                .as_ref()
                .expect("bass 组必须带压缩");
            assert_eq!(bass_comp.threshold_db, -30.0);
            assert_eq!(bass_comp.ratio, 6.0);
            let sidechain = style.lane_chains[0]
                .sidechain
                .as_ref()
                .expect("bass 组必须带侧链声明");
            assert_eq!(
                sidechain.key_keys,
                vec![36],
                "{id} 的 key 必须是 kick 键 36"
            );
            assert_eq!(style.lane_chains[1].name, "kick");
            assert_eq!(style.lane_chains[1].keys, vec![36]);
            assert_eq!(style.lane_chains[2].name, "hats");
            assert_eq!(style.lane_chains[2].keys, hats_keys);
            // 808 道与 kick 同步点（steps 镜像）：生成出的 808 音符数与 kick 相等。
            let bass_lane = style
                .lanes
                .iter()
                .find(|lane| lane.gm_key == 35)
                .expect("出厂侧链风格必须带 808 道（键 35）");
            let kick_lane = style
                .lanes
                .iter()
                .find(|lane| lane.gm_key == 36)
                .expect("必须有 kick 道");
            assert_eq!(
                bass_lane.steps, kick_lane.steps,
                "{id} 的 808 步进须镜像 kick"
            );
        }
        // drill / hiphop（2026-11-20 Wang 立项加 808 道）：只带 bass 一组
        // ——压缩 + kick 键控侧链，定值与 trap 同族（阈值/比/Knee/起音
        // 一致），release 按各自 BPM 分档（drill 142 → 0.16，hiphop
        // 92 → 0.24）。drill 的 808 步进镜像 kick（跟 kick 走滑音长音，
        // 滑音与长尾在 synth 参数面）；hiphop 的 808 不与 kick 同点，
        // 专补 kick 的空缺拍并以长音 sustain 衔接（步点逐条钉住）。
        for (id, release_s, hits) in [
            ("drill", 0.16f32, None),
            (
                "hiphop",
                0.24f32,
                Some(vec![
                    (4usize, 96u8),
                    (8, 100),
                    (14, 88),
                    (20, 96),
                    (24, 100),
                    (30, 90),
                ]),
            ),
        ] {
            let style = by_id(id);
            assert_eq!(style.lane_chains.len(), 1, "{id} 只带 bass 一组");
            assert_eq!(style.lane_chains[0].name, "bass");
            assert_eq!(style.lane_chains[0].keys, vec![35]);
            let comp = style.lane_chains[0]
                .comp
                .as_ref()
                .expect("bass 组必须带压缩");
            assert_eq!(comp.threshold_db, -30.0);
            assert_eq!(comp.ratio, 6.0);
            assert_eq!(comp.release_s, release_s, "{id} 的 release 定值不符");
            let sidechain = style.lane_chains[0]
                .sidechain
                .as_ref()
                .expect("bass 组必须带侧链声明");
            assert_eq!(
                sidechain.key_keys,
                vec![36],
                "{id} 的 key 必须是 kick 键 36"
            );
            let bass_lane = style
                .lanes
                .iter()
                .find(|lane| lane.gm_key == 35)
                .expect("必须带 808 道（键 35）");
            let kick_lane = style
                .lanes
                .iter()
                .find(|lane| lane.gm_key == 36)
                .expect("必须有 kick 道");
            if let Some(hits) = hits {
                let actual: Vec<(usize, u8)> = bass_lane
                    .steps
                    .iter()
                    .enumerate()
                    .filter(|(_, step)| step.is_hit())
                    .map(|(idx, step)| (idx, step.velocity))
                    .collect();
                assert_eq!(actual, hits, "{id} 的 808 步点须为补空缺设计");
            } else {
                assert_eq!(
                    bass_lane.steps, kick_lane.steps,
                    "{id} 的 808 步进须镜像 kick"
                );
            }
        }
        // 原声系 6 风格（P4）：只带增益的逐件相对电平组 —— 无器件段、
        // 无声相、无侧链；增益值逐条钉住（出厂定值，改值须改本判据）。
        type GainGroup = (&'static str, &'static [u8], f32);
        let acoustic_gains: [(&str, &[GainGroup]); 6] = [
            (
                "pop",
                &[
                    ("kick", &[36], 1.0),
                    ("snare", &[38], 0.5),
                    ("hats", &[42, 44, 46], -2.0),
                    ("shaker", &[82], -1.0),
                ],
            ),
            (
                "rock",
                &[
                    ("kick", &[36], 1.0),
                    ("snare", &[38], 1.0),
                    ("hats", &[42, 44, 46], -1.5),
                    ("shaker", &[82], -1.0),
                ],
            ),
            (
                "boombap",
                &[
                    ("kick", &[36], 1.5),
                    ("snare", &[38], 0.5),
                    ("hats", &[42, 44, 46], -1.5),
                    ("shaker", &[82], -1.0),
                ],
            ),
            (
                "funk",
                &[
                    ("kick", &[36], 1.0),
                    ("snare", &[37, 38], 0.5),
                    ("hats", &[42, 44, 92], -2.0),
                    ("crash", &[49], -1.0),
                    ("shaker", &[82], -1.0),
                ],
            ),
            (
                "jazz",
                &[
                    ("kick", &[36], 1.0),
                    ("snare", &[37, 38], 0.5),
                    ("hats", &[42, 44, 92], -1.0),
                    ("crash", &[49], -1.5),
                    ("ride", &[51, 53], -1.0),
                ],
            ),
            (
                "rnb",
                &[
                    ("kick", &[36], 0.5),
                    ("snare", &[37, 38], 1.0),
                    ("hats", &[42, 44, 92], -1.5),
                    ("crash", &[49], -1.0),
                    ("shaker", &[82], -1.5),
                ],
            ),
        ];
        for (id, expected) in acoustic_gains {
            let style = by_id(id);
            assert_eq!(style.lane_chains.len(), expected.len(), "{id} 增益组数不符");
            for (chain, (name, keys, gain)) in style.lane_chains.iter().zip(expected) {
                assert_eq!(chain.name, *name, "{id} 组名不符");
                assert_eq!(chain.keys, *keys, "{id} {name} 键组不符");
                assert_eq!(chain.gain_db, Some(*gain), "{id} {name} 增益定值不符");
                assert!(
                    chain.eq.is_none()
                        && chain.comp.is_none()
                        && chain.reverb.is_none()
                        && chain.tape.is_none()
                        && chain.delay.is_none()
                        && chain.pan.is_none()
                        && chain.sidechain.is_none(),
                    "{id} {name} 必须是纯增益组（无器件/声相/侧链段）"
                );
            }
        }
        // 其余出厂风格本期不带逐件链（合成系相对电平走 *_level，不在此写）。
        // drill / hiphop 自 2026-11-20（Wang 立项加 808 道）起带 bass 组，
        // 已在上方逐条钉住，故列入带链名单。
        for style in &styles {
            if !matches!(
                style.id.as_str(),
                "house"
                    | "trap"
                    | "lofi"
                    | "phonk"
                    | "drill"
                    | "hiphop"
                    | "pop"
                    | "rock"
                    | "boombap"
                    | "funk"
                    | "jazz"
                    | "rnb"
            ) {
                assert!(
                    style.lane_chains.is_empty(),
                    "出厂风格 {} 本期不带逐件链",
                    style.id
                );
            }
        }
    }

    /// D3/D4 接线（预置面）：funk 的 fx 带瞬态段、lofi 的 fx 带碾碎段，
    /// 且器件配方把两段翻成带规范拼写的器件；在场标记能压掉重复生成。
    #[test]
    fn factory_funk_and_lofi_carry_the_wired_shaping_sections() {
        let styles = factory_drum_styles().expect("factory presets parse");
        let by_id = |id: &str| styles.iter().find(|s| s.id == id).unwrap();

        let funk = by_id("funk");
        let fx = funk.fx.expect("funk fx");
        let transient = fx.transient.expect("funk fx 必须带瞬态段（D3 接线）");
        assert_eq!(transient.attack.to_bits(), 0.45f32.to_bits());
        assert_eq!(transient.sustain.to_bits(), 0.15f32.to_bits());
        assert!(fx.crusher.is_none(), "funk 不带碾碎段");
        let specs = drum_fx_device_specs(&fx, DrumFxFamilies::default());
        let transient_spec = specs
            .iter()
            .find(|spec| spec.family == "transient")
            .expect("配方必须含瞬态器件");
        assert_eq!(transient_spec.name, "Transient Shaper");
        assert_eq!(
            transient_spec.params,
            vec![
                ("transient_attack".to_owned(), 0.45),
                ("transient_sustain".to_owned(), 0.15),
            ]
        );
        let present = DrumFxFamilies {
            transient: true,
            ..Default::default()
        };
        let specs = drum_fx_device_specs(&fx, present);
        let families: Vec<&str> = specs.iter().map(|spec| spec.family).collect();
        assert!(
            !families.contains(&"transient"),
            "瞬态族已在场 ⇒ 不得重复生成，实测配方 {families:?}"
        );

        let lofi = by_id("lofi");
        let fx = lofi.fx.expect("lofi fx");
        let crusher = fx.crusher.expect("lofi fx 必须带碾碎段（D4 接线）");
        assert_eq!(crusher.bits.to_bits(), 10.0f32.to_bits());
        assert_eq!(crusher.down.to_bits(), 2.0f32.to_bits());
        assert_eq!(crusher.aa.to_bits(), 0.35f32.to_bits());
        assert!(fx.transient.is_none(), "lofi 不带瞬态段");
        let specs = drum_fx_device_specs(&fx, DrumFxFamilies::default());
        let crusher_spec = specs
            .iter()
            .find(|spec| spec.family == "crusher")
            .expect("配方必须含碾碎器件");
        assert_eq!(crusher_spec.name, "Bitcrusher");
        assert_eq!(
            crusher_spec.params,
            vec![
                ("crush_bits".to_owned(), 10.0),
                ("crush_down".to_owned(), 2.0),
                ("crush_aa".to_owned(), 0.35),
            ]
        );
        let present = DrumFxFamilies {
            crusher: true,
            ..Default::default()
        };
        let specs = drum_fx_device_specs(&fx, present);
        let families: Vec<&str> = specs.iter().map(|spec| spec.family).collect();
        assert!(
            !families.contains(&"crusher"),
            "碾碎族已在场 ⇒ 不得重复生成，实测配方 {families:?}"
        );
    }

    /// D3/D4 接线（解析面）：fx 与逐件链两处都认新段；越界与缺字段响亮拒绝；
    /// 只带碾碎段的逐件链不算空链。
    #[test]
    fn shaping_sections_parse_and_reject_loudly() {
        // fx 两段齐备。
        let text = MINIMAL.replace(
            "\"bars\": 1,",
            "\"bars\": 1, \"fx\": {\
             \"eq\": {\"low_db\": 0, \"mid_db\": 0, \"high_db\": 0, \"low_hz\": 200,\
                    \"mid_hz\": 1000, \"mid_q\": 0.9, \"high_hz\": 6000,\
                    \"hpf_hz\": 0, \"cutoff_hz\": 20000},\
             \"comp\": {\"threshold_db\": -16, \"ratio\": 3, \"knee_db\": 6,\
                      \"attack_s\": 0.01, \"release_s\": 0.12, \"makeup_db\": 0},\
             \"output_db\": 0,\
             \"reverb\": {\"size\": 0.4, \"damp\": 0.45, \"wet\": 0.15, \"predelay_s\": 0.01},\
             \"transient\": {\"attack\": 0.3, \"sustain\": -0.2},\
             \"crusher\": {\"bits\": 12, \"down\": 3, \"aa\": 0.5}},",
        );
        let preset = DrumStylePreset::from_json(&text).expect("valid shaping fx");
        let fx = preset.fx.expect("fx parsed");
        // 旧式混响段（只有 size、无 kind/decay_s）解析为 Room + decay_s 未设（0）。
        assert_eq!(fx.reverb_kind, DrumReverbKind::Room);
        assert_eq!(fx.reverb_decay_s.to_bits(), 0.0f32.to_bits());
        assert_eq!(
            fx.transient.expect("transient").attack.to_bits(),
            0.3f32.to_bits()
        );
        assert_eq!(
            fx.crusher.expect("crusher").down.to_bits(),
            3.0f32.to_bits()
        );

        // 越界与缺字段必须拒绝。
        let bad_attack = text.replace("\"attack\": 0.3", "\"attack\": 1.5");
        assert!(
            DrumStylePreset::from_json(&bad_attack).is_err(),
            "attack 越界须拒绝"
        );
        let bad_bits = text.replace("\"bits\": 12", "\"bits\": 3");
        assert!(
            DrumStylePreset::from_json(&bad_bits).is_err(),
            "bits < 4 须拒绝"
        );
        let missing_aa = text.replace(", \"aa\": 0.5", "");
        assert!(
            DrumStylePreset::from_json(&missing_aa).is_err(),
            "缺 aa 须拒绝"
        );

        // 逐件链：只带碾碎段也算非空链，配方按序产出碾碎器件。
        let text = preset_with_lanes(
            "electronic",
            &[36, 42],
            ", \"lane_chains\": [{\"name\": \"kick\", \"keys\": [36], \"chain\": {\
             \"crusher\": {\"bits\": 8, \"down\": 4, \"aa\": 0}}}]",
        );
        let preset = DrumStylePreset::from_json(&text).expect("crusher-only lane chain");
        let chain = &preset.lane_chains[0];
        assert_eq!(
            chain.crusher.expect("lane crusher").bits.to_bits(),
            8.0f32.to_bits()
        );
        let specs = drum_lane_device_specs(chain);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].family, "crusher");
    }

    /// D5 新器件（预置面）：lofi 的 fx 带噪声段且只有 lofi 带；其余 13
    /// 风格不写噪声段（不写段 = 无噪声器件 = 渲染逐位不变）；配方把噪声
    /// 段翻成链头第一台器件；在场标记能压掉重复生成。
    #[test]
    fn factory_lofi_carries_the_noise_section_and_others_do_not() {
        let styles = factory_drum_styles().expect("factory presets parse");
        let by_id = |id: &str| styles.iter().find(|s| s.id == id).unwrap();

        let lofi = by_id("lofi");
        let fx = lofi.fx.expect("lofi fx");
        let noise = fx.noise.expect("lofi fx 必须带噪声段（D5 新器件）");
        assert_eq!(noise.level.to_bits(), 0.035f32.to_bits());
        assert_eq!(noise.color.to_bits(), 0.85f32.to_bits());
        assert_eq!(noise.crackle_density.to_bits(), 5.0f32.to_bits());
        assert_eq!(noise.crackle_level.to_bits(), 0.4f32.to_bits());
        assert_eq!(noise.seed.to_bits(), 2026.0f32.to_bits());
        for style in &styles {
            if style.id != "lofi" {
                let other = style.fx.expect("出厂风格都带 fx");
                assert!(
                    other.noise.is_none(),
                    "{} 不得带噪声段（只有 lofi 出厂写噪声）",
                    style.id
                );
            }
        }
        let specs = drum_fx_device_specs(&fx, DrumFxFamilies::default());
        let noise_spec = specs
            .iter()
            .find(|spec| spec.family == "noise")
            .expect("配方必须含噪声器件");
        assert_eq!(noise_spec.name, "Noise Source");
        assert_eq!(specs[0].family, "noise", "噪声源是链头，配方也必须排第一");
        assert_eq!(
            noise_spec.params,
            vec![
                ("noise_level".to_owned(), 0.035),
                ("noise_color".to_owned(), 0.85),
                ("noise_crackle_density".to_owned(), 5.0),
                ("noise_crackle_level".to_owned(), 0.4),
                ("noise_seed".to_owned(), 2026.0),
            ]
        );
        let present = DrumFxFamilies {
            noise: true,
            ..Default::default()
        };
        let specs = drum_fx_device_specs(&fx, present);
        let families: Vec<&str> = specs.iter().map(|spec| spec.family).collect();
        assert!(
            !families.contains(&"noise"),
            "噪声族已在场 ⇒ 不得重复生成，实测配方 {families:?}"
        );
    }

    /// D5 新器件（解析面）：fx 与逐件链两处都认噪声段；越界、缺字段、
    /// 坏类型响亮拒绝；只带噪声段的逐件链不算空链。
    #[test]
    fn noise_sections_parse_and_reject_loudly() {
        let text = MINIMAL.replace(
            "\"bars\": 1,",
            "\"bars\": 1, \"fx\": {\"eq\": {\"low_db\": 0, \"mid_db\": 0, \"high_db\": 0, \"low_hz\": 200, \"mid_hz\": 1000, \"mid_q\": 0.9, \"high_hz\": 6000, \"hpf_hz\": 0, \"cutoff_hz\": 20000}, \"comp\": {\"threshold_db\": -16, \"ratio\": 3, \"knee_db\": 6, \"attack_s\": 0.01, \"release_s\": 0.12, \"makeup_db\": 0}, \"output_db\": 0, \"reverb\": {\"size\": 0.4, \"damp\": 0.45, \"wet\": 0.15, \"predelay_s\": 0.01}, \"noise\": {\"level\": 0.03, \"color\": 0.5, \"crackle_density\": 4, \"crackle_level\": 0.3, \"seed\": 99}},",
        );
        let preset = DrumStylePreset::from_json(&text).expect("valid noise fx");
        let fx = preset.fx.expect("fx parsed");
        let noise = fx.noise.expect("noise parsed");
        assert_eq!(noise.level.to_bits(), 0.03f32.to_bits());
        assert_eq!(noise.seed.to_bits(), 99.0f32.to_bits());

        // 越界与缺字段必须拒绝。
        let bad_level = text.replace("\"level\": 0.03", "\"level\": 1.5");
        assert!(
            DrumStylePreset::from_json(&bad_level).is_err(),
            "level 越界须拒绝"
        );
        let bad_density = text.replace("\"crackle_density\": 4", "\"crackle_density\": 241");
        assert!(
            DrumStylePreset::from_json(&bad_density).is_err(),
            "crackle_density 越界须拒绝"
        );
        let bad_seed = text.replace("\"seed\": 99", "\"seed\": -1");
        assert!(
            DrumStylePreset::from_json(&bad_seed).is_err(),
            "seed 为负须拒绝"
        );
        let missing_seed = text.replace(", \"seed\": 99", "");
        assert!(
            DrumStylePreset::from_json(&missing_seed).is_err(),
            "缺 seed 须拒绝"
        );
        let bad_type = text.replace("\"level\": 0.03", "\"level\": \"loud\"");
        assert!(
            DrumStylePreset::from_json(&bad_type).is_err(),
            "level 坏类型须拒绝"
        );

        // 逐件链：只带噪声段也算非空链，配方产出噪声器件。
        let text = preset_with_lanes(
            "electronic",
            &[36, 42],
            ", \"lane_chains\": [{\"name\": \"kick\", \"keys\": [36], \"chain\": {\"noise\": {\"level\": 0.02, \"color\": 1, \"crackle_density\": 0, \"crackle_level\": 0, \"seed\": 1}}}]",
        );
        let preset = DrumStylePreset::from_json(&text).expect("noise-only lane chain");
        let chain = &preset.lane_chains[0];
        assert_eq!(
            chain.noise.expect("lane noise").level.to_bits(),
            0.02f32.to_bits()
        );
        let specs = drum_lane_device_specs(chain);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].family, "noise");
    }

    // ------------------------------------------------ 自动化段（P5）

    /// 夹具 fx 段：hpf 关（0）、低通关（20000）、output 0 —— 通道条
    /// 的三个条件参数都未启用，供条件参数拒绝判据用。
    const FIXTURE_FX: &str = r#""fx": {"eq": {"low_db": 0, "mid_db": 0, "high_db": 0, "low_hz": 100, "mid_hz": 1000, "mid_q": 1, "high_hz": 8000, "hpf_hz": 0, "cutoff_hz": 20000}, "comp": {"threshold_db": -12, "ratio": 2, "knee_db": 6, "attack_s": 0.01, "release_s": 0.1, "makeup_db": 0}, "output_db": 0, "reverb": {"kind": "room", "decay_s": 1.2, "damp": 0.5, "wet": 0.2, "predelay_s": 0.01}}"#;

    fn preset_with_automation(fx: Option<&str>, automation: &str) -> String {
        let fx_part = fx.map_or_else(String::new, |fx| format!(", {fx}"));
        preset_with_lanes(
            "electronic",
            &[36],
            &format!("{fx_part}, \"automation\": {automation}"),
        )
    }

    #[test]
    fn automation_absent_means_empty_and_factory_only_edm_carries_it() {
        let plain = DrumStylePreset::from_json(MINIMAL).expect("valid preset");
        assert!(plain.automation.is_empty(), "无 automation 段 = 空表");

        // 出厂 14 风格全部可解析；只有 edm 写了自动化段，其余 13 份
        // 空表（生成逐位不变的格式侧钉子）。
        let styles = factory_drum_styles().expect("factory styles parse");
        assert_eq!(styles.len(), 14);
        for preset in &styles {
            if preset.id == "edm" {
                assert_eq!(preset.automation.len(), 1, "edm 带一段自动化");
                let segment = &preset.automation[0];
                assert_eq!(
                    segment.target,
                    DrumAutomationTarget::DeviceParam {
                        family: "reverb".to_owned(),
                        param: "reverb_wet".to_owned(),
                    }
                );
                assert_eq!(segment.curve, DrumAutomationCurve::SCurve);
                let points: Vec<(u64, f32)> =
                    segment.points.iter().map(|p| (p.tick, p.value)).collect();
                assert_eq!(points, vec![(0, 0.10), (3840, 0.15), (7680, 0.22)]);
            } else {
                assert!(preset.automation.is_empty(), "{} 不带自动化段", preset.id);
            }
        }
    }

    #[test]
    fn automation_segment_parses_targets_points_curve_and_landing() {
        let text = preset_with_automation(
            Some(FIXTURE_FX),
            r#"[
                {"target": "track_pan", "points": [[0, -0.5], [3840, 0.5]]},
                {"target": "track_volume", "curve": "exponential", "points": [[0, -6.0], [3840, -3.0]]},
                {"target": "reverb.reverb_wet", "points": [[0, 0.2], [1920, 0.35]]}
            ]"#,
        );
        let preset = DrumStylePreset::from_json(&text).expect("valid automation");
        assert_eq!(preset.automation.len(), 3);
        // 缺省 curve = linear（格式级缺省，落点时逐点展开）。
        assert_eq!(preset.automation[0].target, DrumAutomationTarget::TrackPan);
        assert_eq!(preset.automation[0].curve, DrumAutomationCurve::Linear);
        assert_eq!(preset.automation[0].points[1].tick, 3840);
        assert_eq!(preset.automation[0].points[1].value, 0.5);
        assert_eq!(
            preset.automation[1].target,
            DrumAutomationTarget::TrackVolume
        );
        assert_eq!(preset.automation[1].curve, DrumAutomationCurve::Exponential);
        // 落点换算：轨级目标直接成形；器件参数按名取第一台命中器件。
        let devices: Vec<Vec<String>> = vec![
            vec!["kick_tune_hz".to_owned()],
            vec![
                "reverb_kind".to_owned(),
                "reverb_decay_s".to_owned(),
                "reverb_damp".to_owned(),
                "reverb_wet".to_owned(),
                "reverb_predelay".to_owned(),
            ],
        ];
        assert_eq!(
            preset.automation[0].landing(&devices),
            Some(DrumAutomationLanding::TrackPan)
        );
        assert_eq!(
            preset.automation[1].landing(&devices),
            Some(DrumAutomationLanding::TrackVolume)
        );
        assert_eq!(
            preset.automation[2].landing(&devices),
            Some(DrumAutomationLanding::DeviceParam {
                slot_index: 1,
                param_index: 3,
            })
        );
        // 器件不在轨上 ⇒ 无落点（生成路径据此响亮拒绝，不静默丢段）。
        assert_eq!(preset.automation[2].landing(&[]), None);
        // 参数词表自查：已知参数有值域，未知名字无。
        assert_eq!(
            drum_fx_param_domain("reverb", "reverb_wet"),
            Some((0.0, 1.0))
        );
        assert_eq!(drum_fx_param_domain("reverb", "reverb_wetness"), None);
        assert_eq!(drum_fx_param_domain("echo", "delay_mix"), None);
        // P7 回归钉：`tape_params` 写出的九个名字必须全部有值域——
        // wow/flutter 五个曾漏登记，自动化写到即被判未知参数。
        let tape = DrumTapePreset {
            drive: 0.3,
            mix: 0.5,
            hf_hz: 9000.0,
            asym: 0.2,
            wow_rate_hz: 0.5,
            wow_depth_ms: 0.0,
            flutter_rate_hz: 6.0,
            flutter_depth_ms: 0.0,
            seed: 0.0,
        };
        for (name, _) in tape_params(&tape) {
            assert!(
                drum_fx_param_domain("tape", &name).is_some(),
                "tape 参数 {name} 无值域登记"
            );
        }
        assert_eq!(
            drum_fx_param_domain("tape", "tape_wow_rate_hz"),
            Some((0.05, 2.0))
        );
        assert_eq!(
            drum_fx_param_domain("tape", "tape_seed"),
            Some((0.0, 16777215.0))
        );
    }

    #[test]
    fn automation_rejections_are_loud_and_name_the_field() {
        let seg = |target: &str, points: &str| {
            format!(r#"[{{"target": "{target}", "points": {points}}}]"#)
        };
        let cases: Vec<(String, &str)> = vec![
            // 未知参数名 / 未知族 / 未知裸目标。
            (
                preset_with_automation(
                    Some(FIXTURE_FX),
                    &seg("reverb.reverb_wetness", "[[0, 0.2]]"),
                ),
                "automation.target",
            ),
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("echo.delay_mix", "[[0, 0.2]]")),
                "automation.target",
            ),
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("sidechain", "[[0, 0.2]]")),
                "automation.target",
            ),
            // 器件目标无 fx 段可落。
            (
                preset_with_automation(None, &seg("reverb.reverb_wet", "[[0, 0.2]]")),
                "automation.target",
            ),
            // fx 在场但无 tape 子段。
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("tape.tape_mix", "[[0, 0.2]]")),
                "automation.target",
            ),
            // 条件参数未启用（hpf 关 / output 0）。
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("strip.hpf_hz", "[[0, 100.0]]")),
                "automation.target",
            ),
            (
                preset_with_automation(
                    Some(FIXTURE_FX),
                    &seg("strip.output_gain_db", "[[0, 1.0]]"),
                ),
                "automation.target",
            ),
            // 同一目标两段。
            (
                preset_with_automation(
                    Some(FIXTURE_FX),
                    r#"[{"target": "track_pan", "points": [[0, 0.0]]}, {"target": "track_pan", "points": [[0, 0.5]]}]"#,
                ),
                "automation.target",
            ),
            // 空点列。
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("track_pan", "[]")),
                "automation.points",
            ),
            // tick 逆序 / 同 tick 两点 / 非整数 tick / 越 pattern 全长（bars=1 ⇒ 3840）。
            (
                preset_with_automation(
                    Some(FIXTURE_FX),
                    &seg("track_pan", "[[100, 0.0], [50, 0.5]]"),
                ),
                "automation.points",
            ),
            (
                preset_with_automation(
                    Some(FIXTURE_FX),
                    &seg("track_pan", "[[100, 0.0], [100, 0.5]]"),
                ),
                "automation.points",
            ),
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("track_pan", "[[0.5, 0.0]]")),
                "automation.points",
            ),
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("track_pan", "[[3841, 0.0]]")),
                "automation.points",
            ),
            // 值越目标值域（声相 ±1 / 混响湿 0..=1 / 音量 −60..=12）。
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("track_pan", "[[0, 1.5]]")),
                "automation.points",
            ),
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("reverb.reverb_wet", "[[0, -0.1]]")),
                "automation.points",
            ),
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("track_volume", "[[0, 13.0]]")),
                "automation.points",
            ),
            // 点形状错误（单元素 / 非数组元素）。
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("track_pan", "[[0]]")),
                "automation.points",
            ),
            (
                preset_with_automation(Some(FIXTURE_FX), &seg("track_pan", "[0, 0.5]")),
                "automation.points",
            ),
            // 未知曲线名。
            (
                preset_with_automation(
                    Some(FIXTURE_FX),
                    r#"[{"target": "track_pan", "curve": "wobbly", "points": [[0, 0.0]]}]"#,
                ),
                "automation.curve",
            ),
        ];
        for (text, field) in &cases {
            match DrumStylePreset::from_json(text) {
                Err(DrumPresetError::Field {
                    field: got,
                    message,
                }) => {
                    assert_eq!(&got, field, "错误须点名字段: {message}");
                }
                other => panic!("非法自动化段须响亮拒绝（期望字段 {field}），实际: {other:?}"),
            }
        }
        // 对照：启用 hpf 后 strip.hpf_hz 合法（条件参数的放行侧）。
        let fx_hpf = FIXTURE_FX.replace("\"hpf_hz\": 0", "\"hpf_hz\": 120");
        let ok = preset_with_automation(
            Some(&fx_hpf),
            &seg("strip.hpf_hz", "[[0, 120.0], [3840, 300.0]]"),
        );
        assert!(
            DrumStylePreset::from_json(&ok).is_ok(),
            "hpf 启用后可自动化"
        );
    }
}
