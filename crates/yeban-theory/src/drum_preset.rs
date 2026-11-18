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

/// 一条鼓道（一个 GM 键）的完整 pattern（`bars * 16` 格）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrumLanePreset {
    /// 道角色名（`kick` / `snare` / `hihat` / `clap` / `open_hat` / …）。
    pub role: String,
    /// GM 打击乐键号 0..=127。
    pub gm_key: u8,
    /// 步进格，长度恒为 `bars * STEPS_PER_BAR`。
    pub steps: Vec<DrumStep>,
}

/// 磁带饱和段（`fx.tape`，可选）：给整组鼓加谐波与轻微高频滚降。
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

/// 风格效果层（`fx` 段）：按 Logic 通道条预置的思路给整轨鼓预调的
/// 一套完整处理——三段 EQ + 高通/低通塑形、压缩、输出微调、插入混响，
/// 按风格可选磁带饱和与延迟。生成时由 `yeban-app` 的 `drum_gen_ops`
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
    /// 混响尺寸 0..=1。
    pub reverb_size: f32,
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

/// 混响段（`fx.reverb` 与逐件链 `chain.reverb` 共用同一形状与值域）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumReverbPreset {
    /// 尺寸 0..=1。
    pub size: f32,
    /// 高频阻尼 0..=1。
    pub damp: f32,
    /// 湿电平 0..=1。
    pub wet: f32,
    /// 预延迟（秒）0..=0.1。
    pub predelay_s: f32,
}

/// 逐件链的侧链声明（`lane_chains[].chain.sidechain`）：本组压缩的
/// key 源 = `key_keys` 这些键命中的声部信号。格式层只落声明；
/// lane 级侧链求值在期 4 落地，生成期见到声明响亮拒绝（见
/// [`lane_chain_generation_violations`]），不出厂预置在期 5 之前不写
/// 本段（设计裁决）。
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
    /// 本组声相覆盖（可选）−1..=1。
    pub pan: Option<f32>,
    /// 侧链声明（可选；格式可写，生成期 4 前拒绝，出厂值期 5 才写）。
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
        let lane_chains = parse_lane_chains(obj, &lanes, fill_lanes.as_deref())?;
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
    for (idx, item) in arr.iter().enumerate() {
        let lane_obj = item
            .as_object()
            .ok_or_else(|| field_err(key, format!("lane {idx} must be an object")))?;
        let role = get_str(lane_obj, "role")?.to_owned();
        let gm_key = u8::try_from(get_u64(lane_obj, "gm_key")?)
            .map_err(|_| field_err("gm_key", format!("lane {idx} gm_key out of range")))?;
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
            steps,
        });
    }
    Ok(lanes)
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

/// 解析混响段（`{base}.reverb`）：整段字段必填。`fx` 与逐件链共用。
fn parse_reverb(
    obj: &[(String, Json)],
    base: &str,
) -> Result<DrumReverbPreset, DrumPresetError> {
    Ok(DrumReverbPreset {
        size: get_f32_ranged(obj, "size", &format!("{base}.reverb.size"), 0.0, 1.0)?,
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
    })
}

/// 解析延迟段（`{base}.delay`）：整段字段必填。`fx` 与逐件链共用。
fn parse_delay(obj: &[(String, Json)], base: &str) -> Result<DrumDelayPreset, DrumPresetError> {
    let ping_pong = match get(obj, "ping_pong")? {
        Json::Bool(b) => *b,
        _ => return Err(field_err(&format!("{base}.delay.ping_pong"), "expected a boolean")),
    };
    Ok(DrumDelayPreset {
        time_s: get_f32_ranged(obj, "time_s", &format!("{base}.delay.time_s"), 0.01, 2.0)?,
        feedback: get_f32_ranged(
            obj,
            "feedback",
            &format!("{base}.delay.feedback"),
            0.0,
            0.9,
        )?,
        mix: get_f32_ranged(obj, "mix", &format!("{base}.delay.mix"), 0.0, 0.6)?,
        damp: get_f32_ranged(obj, "damp", &format!("{base}.delay.damp"), 0.0, 1.0)?,
        ping_pong,
    })
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
        reverb_size: reverb.size,
        reverb_damp: reverb.damp,
        reverb_wet: reverb.wet,
        reverb_predelay_s: reverb.predelay_s,
        tape,
        delay,
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
/// 键的子集。choke 组不拆分与侧链声明拒绝是**生成期**校验（依赖所选
/// SFZ 程序与落地期次，见 [`lane_chain_generation_violations`]）。
fn parse_lane_chains(
    obj: &[(String, Json)],
    lanes: &[DrumLanePreset],
    fill_lanes: Option<&[DrumLanePreset]>,
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
            if !lane_keys.contains(key) {
                return Err(field_err(
                    &format!("lane_chains.{name}.keys"),
                    format!("key {key} is not one of this preset's lane keys"),
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
                let comp_obj = raw_comp.as_object().ok_or_else(|| {
                    field_err(&format!("{base}.comp"), "comp must be an object")
                })?;
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
                let tape_obj = raw_tape.as_object().ok_or_else(|| {
                    field_err(&format!("{base}.tape"), "tape must be an object")
                })?;
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
            && pan.is_none()
            && sidechain.is_none()
        {
            return Err(field_err(
                &format!("lane_chains.{name}"),
                "lane chain group declares an empty chain (no eq/comp/reverb/tape/delay/pan/sidechain)",
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
            pan,
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
}

/// 一台待装效果器件的配方（名字 + 参数表）；身份（`EntityId`）由调用方
/// 按各自纪律分配（UI 路径随机、MCP 路径确定性派生），配方只有这一份，
/// 保证两条生成路径装出的效果层逐参数一致。
#[derive(Debug, Clone, PartialEq)]
pub struct DrumFxDeviceSpec {
    /// 效果族标识（`strip` / `tape` / `reverb` / `delay`）。
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
        ("reverb_size".to_owned(), reverb.size),
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
            size: self.reverb_size,
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
    if let Some(tape) = &fx.tape
        && !present.tape
    {
        specs.push(DrumFxDeviceSpec {
            family: "tape",
            name: "Tape Saturation",
            params: tape_params(tape),
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
/// 是期 4 的求值输入，生成在期 4 之前由
/// [`lane_chain_generation_violations`] 响亮拒绝。
#[must_use]
pub fn drum_lane_device_specs(chain: &DrumLaneChainPreset) -> Vec<DrumFxDeviceSpec> {
    let mut specs = Vec::new();
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
    if let Some(tape) = &chain.tape {
        specs.push(DrumFxDeviceSpec {
            family: "tape",
            name: "Tape Saturation",
            params: tape_params(tape),
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
// 逐件链生成校验（choke 组不拆分 + 侧链声明拒绝；生成路径共用）
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
/// [`uncovered_pattern_keys`] 同一「默认不能静默」口径。两类违规：
///
/// 1. **侧链声明**：预置的逐件链带 `sidechain` 段。格式层可写，但
///    lane 级侧链求值在期 4 才落地；期 4 之前生成带侧链声明的逐件链
///    等于静默丢弃声明，响亮拒绝。出厂预置的侧链值期 5 才写。
/// 2. **choke 组被拆分**（仅原声族）：本预置用到的键里，同一 choke
///    组的键不得分布在两条不同的逐件链里，也不得一部分分组、一部分
///    留在整轨链上——逐件渲染按组分引擎，拆开后互掐关系断裂
///    （踩镲掐不开镲的语义被静默改掉）。整组同链或整组不分组才合法。
#[must_use]
pub fn lane_chain_generation_violations(preset: &DrumStylePreset) -> Vec<String> {
    let mut violations = Vec::new();
    for chain in &preset.lane_chains {
        if let Some(sidechain) = &chain.sidechain {
            violations.push(format!(
                "逐件链组 `{}` 声明了侧链（key 键 {:?}）：lane 级侧链求值在期 4 落地，本期生成响亮拒绝，不静默丢弃声明",
                chain.name, sidechain.key_keys
            ));
        }
    }
    if preset.kit_family != KitFamily::Acoustic || preset.lane_chains.is_empty() {
        return violations;
    }
    // 本预置真正用到的键（主道 + fill 道）。
    let mut used: Vec<u8> = preset.lanes.iter().map(|lane| lane.gm_key).collect();
    if let Some(fill) = &preset.fill_lanes {
        used.extend(fill.iter().map(|lane| lane.gm_key));
    }
    for component in choke_groups(preset.sfz_path.as_deref()) {
        // 这组 choke 键落在哪几个分区里：每条逐件链是一个分区，
        // 未被任何逐件链认领的键同属「未分组」分区。
        let mut cells: Vec<String> = Vec::new();
        let mut any_used = false;
        let mut any_ungrouped = false;
        for key in &component {
            if !used.contains(key) {
                continue;
            }
            any_used = true;
            match preset
                .lane_chains
                .iter()
                .find(|chain| chain.keys.contains(key))
            {
                Some(chain) => {
                    if !cells.contains(&chain.name) {
                        cells.push(chain.name.clone());
                    }
                }
                None => any_ungrouped = true,
            }
        }
        if !any_used {
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

    const MINIMAL_FX: &str = r#""fx": {
        "eq": {"low_db": 2.0, "mid_db": -1.0, "high_db": 1.5, "low_hz": 200,
               "mid_hz": 1000, "mid_q": 0.9, "high_hz": 4200, "hpf_hz": 30,
               "cutoff_hz": 20000},
        "comp": {"threshold_db": -16, "ratio": 3, "knee_db": 6,
                 "attack_s": 0.01, "release_s": 0.12, "makeup_db": 1.5},
        "output_db": 0.5,
        "reverb": {"size": 0.4, "damp": 0.45, "wet": 0.15, "predelay_s": 0.01},
        "tape": {"drive": 0.3, "mix": 0.5, "hf_hz": 9000, "asym": 0.2},
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
                format!(
                    "{{\"role\": \"lane{key}\", \"gm_key\": {key}, \"steps\": {STEPS16}}}"
                )
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
        assert!(!names.contains(&"compressor_enabled"), "无 comp 段不得拼压缩");
        assert!(!names.contains(&"output_gain_db"), "逐件链无 output 微调");
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
                \"reverb\": {\"size\": 0.3, \"damp\": 0.5, \"wet\": 0.1, \"predelay_s\": 0.01},\
                \"tape\": {\"drive\": 0.2, \"mix\": 0.4, \"hf_hz\": 8000, \"asym\": 0.1},\
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
        assert!(strip_names.contains(&"cutoff_hz"), "cutoff < 20000 必须拼低通");
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
        // 侧链声明：格式可解析，生成期拒绝（期 4 前）。
        let sidechained = DrumStylePreset::from_json(&preset_with_lanes(
            "electronic",
            &[36, 42],
            ", \"lane_chains\": [{\"name\": \"kick\", \"keys\": [36], \"chain\": {\
                \"comp\": {\"threshold_db\": -18, \"ratio\": 3, \"knee_db\": 6,\
                          \"attack_s\": 0.01, \"release_s\": 0.12, \"makeup_db\": 1},\
                \"sidechain\": {\"key_keys\": [36]}}}]",
        ))
        .expect("sidechain declaration parses");
        let violations = lane_chain_generation_violations(&sidechained);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("期 4"), "违规须点名期次: {violations:?}");

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

        // 42/46 同组 ⇒ 合法。
        let whole = DrumStylePreset::from_json(&preset_with_lanes(
            "acoustic",
            &[36, 42, 46],
            ", \"lane_chains\": [{\"name\": \"hats\", \"keys\": [42, 46], \"chain\": {\"pan\": 0}}]",
        ))
        .expect("parses");
        assert!(lane_chain_generation_violations(&whole).is_empty());

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
        let basic = choke_groups(Some("assets/samples/virtuosity-drums/Programs/01-basic-kit.sfz"));
        assert!(basic.contains(&vec![42, 44, 46]));
        let other = choke_groups(Some("assets/samples/virtuosity-drums/Programs/06-mid-mic.sfz"));
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
        // house：帽组 [42, 46] 一条链；trap：闭镲 [42]；lofi：kick [36]。
        let house = by_id("house");
        assert_eq!(house.lane_chains.len(), 1);
        assert_eq!(house.lane_chains[0].name, "hats");
        assert_eq!(house.lane_chains[0].keys, vec![42, 46]);
        let trap = by_id("trap");
        assert_eq!(trap.lane_chains.len(), 1);
        assert_eq!(trap.lane_chains[0].keys, vec![42]);
        let lofi = by_id("lofi");
        assert_eq!(lofi.lane_chains.len(), 1);
        assert_eq!(lofi.lane_chains[0].keys, vec![36]);
        // 其余出厂风格本期不带逐件链。
        for style in &styles {
            if !matches!(style.id.as_str(), "house" | "trap" | "lofi") {
                assert!(
                    style.lane_chains.is_empty(),
                    "出厂风格 {} 本期不带逐件链",
                    style.id
                );
            }
        }
    }
}
