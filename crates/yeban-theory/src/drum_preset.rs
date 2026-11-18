//! **鼓机风格预置包**（一期）：格式定义、解析与出厂预置。
//!
//! 格式是 JSON，模式见 `crates/yeban-theory/drum_styles/FORMAT.md`。本模块
//! 只做三件事：把 JSON 文本解析成 [`DrumStylePreset`]、校验形状与值域、
//! 给出仓内出厂的 10 个风格（`include_str!` 嵌入，解析逻辑与将来从
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
    /// 效果层预置（`fx` 段）；`None` = 文件未带效果段，生成走干声。
    pub fx: Option<DrumFxPreset>,
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
        let fx = match get_opt(obj, "fx") {
            None => None,
            Some(fx_json) => {
                let fx_obj = fx_json
                    .as_object()
                    .ok_or_else(|| field_err("fx", "fx must be an object"))?;
                Some(parse_fx(fx_obj)?)
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
            fx,
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

fn parse_fx(obj: &[(String, Json)]) -> Result<DrumFxPreset, DrumPresetError> {
    let eq = get_obj(obj, "eq")?;
    let comp = get_obj(obj, "comp")?;
    let reverb = get_obj(obj, "reverb")?;
    let tape = match get_opt(obj, "tape") {
        None => None,
        Some(tape_json) => {
            let t = tape_json
                .as_object()
                .ok_or_else(|| field_err("fx.tape", "tape must be an object"))?;
            Some(DrumTapePreset {
                drive: get_f32_ranged(t, "drive", "fx.tape.drive", 0.0, 1.0)?,
                mix: get_f32_ranged(t, "mix", "fx.tape.mix", 0.0, 1.0)?,
                hf_hz: get_f32_ranged(t, "hf_hz", "fx.tape.hf_hz", 2000.0, 20000.0)?,
                asym: get_f32_ranged(t, "asym", "fx.tape.asym", 0.0, 0.5)?,
            })
        }
    };
    let delay = match get_opt(obj, "delay") {
        None => None,
        Some(delay_json) => {
            let d = delay_json
                .as_object()
                .ok_or_else(|| field_err("fx.delay", "delay must be an object"))?;
            let ping_pong = match get(d, "ping_pong")? {
                Json::Bool(b) => *b,
                _ => return Err(field_err("fx.delay.ping_pong", "expected a boolean")),
            };
            Some(DrumDelayPreset {
                time_s: get_f32_ranged(d, "time_s", "fx.delay.time_s", 0.01, 2.0)?,
                feedback: get_f32_ranged(d, "feedback", "fx.delay.feedback", 0.0, 0.9)?,
                mix: get_f32_ranged(d, "mix", "fx.delay.mix", 0.0, 0.6)?,
                damp: get_f32_ranged(d, "damp", "fx.delay.damp", 0.0, 1.0)?,
                ping_pong,
            })
        }
    };
    Ok(DrumFxPreset {
        eq_low_db: get_f32_ranged(eq, "low_db", "fx.eq.low_db", -18.0, 18.0)?,
        eq_mid_db: get_f32_ranged(eq, "mid_db", "fx.eq.mid_db", -18.0, 18.0)?,
        eq_high_db: get_f32_ranged(eq, "high_db", "fx.eq.high_db", -18.0, 18.0)?,
        eq_low_hz: get_f32_ranged(eq, "low_hz", "fx.eq.low_hz", 20.0, 1000.0)?,
        eq_mid_hz: get_f32_ranged(eq, "mid_hz", "fx.eq.mid_hz", 100.0, 8000.0)?,
        eq_mid_q: get_f32_ranged(eq, "mid_q", "fx.eq.mid_q", 0.3, 6.0)?,
        eq_high_hz: get_f32_ranged(eq, "high_hz", "fx.eq.high_hz", 1000.0, 16000.0)?,
        hpf_hz: get_f32_ranged(eq, "hpf_hz", "fx.eq.hpf_hz", 0.0, 500.0)?,
        cutoff_hz: get_f32_ranged(eq, "cutoff_hz", "fx.eq.cutoff_hz", 800.0, 20000.0)?,
        comp_threshold_db: get_f32_ranged(
            comp,
            "threshold_db",
            "fx.comp.threshold_db",
            -60.0,
            0.0,
        )?,
        comp_ratio: get_f32_ranged(comp, "ratio", "fx.comp.ratio", 1.0, 20.0)?,
        comp_knee_db: get_f32_ranged(comp, "knee_db", "fx.comp.knee_db", 0.0, 24.0)?,
        comp_attack_s: get_f32_ranged(comp, "attack_s", "fx.comp.attack_s", 0.0005, 0.2)?,
        comp_release_s: get_f32_ranged(comp, "release_s", "fx.comp.release_s", 0.01, 1.0)?,
        comp_makeup_db: get_f32_ranged(comp, "makeup_db", "fx.comp.makeup_db", -12.0, 12.0)?,
        output_db: get_f32_ranged(obj, "output_db", "fx.output_db", -12.0, 12.0)?,
        reverb_size: get_f32_ranged(reverb, "size", "fx.reverb.size", 0.0, 1.0)?,
        reverb_damp: get_f32_ranged(reverb, "damp", "fx.reverb.damp", 0.0, 1.0)?,
        reverb_wet: get_f32_ranged(reverb, "wet", "fx.reverb.wet", 0.0, 1.0)?,
        reverb_predelay_s: get_f32_ranged(reverb, "predelay_s", "fx.reverb.predelay_s", 0.0, 0.1)?,
        tape,
        delay,
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
        let mut params: Vec<(String, f32)> = vec![
            ("eq_enabled".to_owned(), 1.0),
            ("eq_low_gain".to_owned(), fx.eq_low_db),
            ("eq_low_freq".to_owned(), fx.eq_low_hz),
            ("eq_mid_gain".to_owned(), fx.eq_mid_db),
            ("eq_mid_freq".to_owned(), fx.eq_mid_hz),
            ("eq_mid_q".to_owned(), fx.eq_mid_q),
            ("eq_high_gain".to_owned(), fx.eq_high_db),
            ("eq_high_freq".to_owned(), fx.eq_high_hz),
        ];
        if fx.hpf_hz > 0.0 {
            params.push(("hpf_hz".to_owned(), fx.hpf_hz));
        }
        if fx.cutoff_hz < 20_000.0 {
            params.push(("filter_enabled".to_owned(), 1.0));
            params.push(("cutoff_hz".to_owned(), fx.cutoff_hz));
        }
        params.push(("compressor_enabled".to_owned(), 1.0));
        params.push(("threshold_db".to_owned(), fx.comp_threshold_db));
        params.push(("ratio".to_owned(), fx.comp_ratio));
        params.push(("knee_db".to_owned(), fx.comp_knee_db));
        params.push(("attack_s".to_owned(), fx.comp_attack_s));
        params.push(("release_s".to_owned(), fx.comp_release_s));
        params.push(("makeup_db".to_owned(), fx.comp_makeup_db));
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
            params: vec![
                ("tape_drive".to_owned(), tape.drive),
                ("tape_mix".to_owned(), tape.mix),
                ("tape_hf_hz".to_owned(), tape.hf_hz),
                ("tape_asym".to_owned(), tape.asym),
            ],
        });
    }
    if !present.reverb {
        specs.push(DrumFxDeviceSpec {
            family: "reverb",
            name: "Reverb",
            params: vec![
                ("reverb_size".to_owned(), fx.reverb_size),
                ("reverb_damp".to_owned(), fx.reverb_damp),
                ("reverb_wet".to_owned(), fx.reverb_wet),
                ("reverb_predelay".to_owned(), fx.reverb_predelay_s),
            ],
        });
    }
    if let Some(delay) = &fx.delay
        && !present.delay
    {
        specs.push(DrumFxDeviceSpec {
            family: "delay",
            name: "Delay",
            params: vec![
                ("delay_time_s".to_owned(), delay.time_s),
                ("delay_feedback".to_owned(), delay.feedback),
                ("delay_mix".to_owned(), delay.mix),
                ("delay_damp".to_owned(), delay.damp),
                (
                    "delay_ping_pong".to_owned(),
                    if delay.ping_pong { 1.0 } else { 0.0 },
                ),
            ],
        });
    }
    specs
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
        sfz_path: &'static str,
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
            sfz_path: ACOUSTIC_KIT_SFZ_PATH,
        },
        KitFamily::Electronic => {
            let mut params: Vec<(String, f32)> = vec![
                ("kick_note".to_owned(), 36.0),
                ("snare_note".to_owned(), 38.0),
                ("closed_hat_note".to_owned(), 42.0),
                ("open_hat_note".to_owned(), 46.0),
                ("clap_note".to_owned(), 39.0),
            ];
            // 电子族的音色定值（808 系）：长衰减底鼓是 trap/drill 的
            // 定义性音色；house/techno/edm 用较短的标准底鼓。
            let (kick_decay_s, kick_tune_hz) = match preset.id.as_str() {
                "trap" | "drill" => (0.95, 48.0),
                "phonk" => (0.70, 52.0),
                _ => (0.42, 55.0),
            };
            params.push(("kick_decay_s".to_owned(), kick_decay_s));
            params.push(("kick_tune_hz".to_owned(), kick_tune_hz));
            DrumKitSetup::SynthDevice {
                device_name: "Yeban Drum Kit (Synth)",
                params,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 出厂预置（嵌入文本与磁盘文件同一格式）
// ---------------------------------------------------------------------------

/// 出厂 10 风格的嵌入源（id 与文件一一对应）。
const FACTORY_SOURCES: [(&str, &str); 10] = [
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
        assert_eq!(styles.len(), 10);
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
}
