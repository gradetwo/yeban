//! 夜半原生预制（`track.preset`）的数据层：预制文件格式 v1、轨上预制状态、
//! 工厂预制登记（§3 的 6 件）与用户预制库位置。
//!
//! 口径（设计稿 `~/workspace/yeban-preset-design.md`，基线 `b6ddba02`）：
//! 预制是小 JSON 文本，`format = 1`；结构变化升 major，只改调优参数升 minor；
//! 旧版读到 major 更新的预制**响亮拒绝**，不静默降级；单份上限 64 KiB，超了拒绝保存。
//! 系统预制标识 `sys:<乐器 slug>:<预制名>`，用户预制标识 `user:<ulid>`。
//! 用户预制库默认放用户主目录下预制库，抄 Logic：`~/Music/Audio Music Apps/Patches/`。
//! 整轨快照不含 `volume_db` 与 `pan`（§7 已定夺，抄 Logic 官方定义）。
//!
//! 本模块是**纯数据**：不读盘、不解码采样、不进 RT 回调。预制加载成本只是解析
//! JSON 加投影（构造期纯函数），应用时不触发采样解码。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::project::MacroParameter;

/// 预制格式的现行 major 版本。
pub const PRESET_FORMAT_VERSION: u32 = 1;

/// 单份预制上限：64 KiB。超了拒绝保存，响亮报错。
pub const PRESET_MAX_BYTES: usize = 64 * 1024;

/// 系统预制在仓库里的目录（只读，随发行）。
pub const FACTORY_PRESETS_DIR: &str = "assets/presets";

/// 工程包内嵌用户预制的条目名前缀（§5）：`presets/user/<id>.json`，一律 stored。
pub const EMBEDDED_USER_PRESET_PREFIX: &str = "presets/user/";

/// 预制校验失败（响亮拒绝，不静默降级、不静默回干声）。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PresetError {
    /// 单份预制超过 64 KiB。
    #[error("预制超过 64 KiB 上限（{actual} 字节）")]
    TooLarge {
        /// 实测字节数。
        actual: usize,
    },
    /// JSON 解析失败。
    #[error("预制不是合法 JSON：{detail}")]
    InvalidJson {
        /// 解析器给出的细节。
        detail: String,
    },
    /// `format` 字段缺失或不是数字。
    #[error("预制缺少数字 `format` 字段")]
    MissingFormat,
    /// major 版本比现行版本新：响亮拒绝，不静默降级。
    #[error("预制 major 版本 {found} 高于本版支持的 {supported}，拒绝读取（不静默降级）")]
    NewerMajor {
        /// 预制声明的 major 版本。
        found: u32,
        /// 本版支持的 major 版本。
        supported: u32,
    },
    /// 标识不符合 `sys:<slug>:<name>` / `user:<ulid>` 规则。
    #[error("预制标识 `{id}` 不合法（须为 sys:<乐器>:<名> 或 user:<ulid>）")]
    InvalidId {
        /// 被拒绝的标识原文。
        id: String,
    },
    /// 必填字段缺失或形状不对。
    #[error("预制字段 `{field}` 不合法：{detail}")]
    InvalidField {
        /// 出问题的字段名。
        field: &'static str,
        /// 细节。
        detail: String,
    },
}

/// 预制的作用域：整轨快照（track）或乐器调优（instrument）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetScope {
    /// 整轨快照：动 instrument、strip、effects、sends、aux、mix 六类。
    Track,
    /// 乐器调优：只动 instrument、strip、sends 三类；不动混音、不动路由结构。
    #[default]
    Instrument,
}

/// 预制里的一条发送：目标 Aux 名与增益（dB）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresetSend {
    /// 目标 Aux 名（如 `Room` / `Hall`）。
    pub aux: String,
    /// 发送增益（dB）；`None` = 单位增益。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
    /// 该发送走的效果（`freeverb` / `convolution`），仅作记录，不建设备。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

/// 预制文件 v1（JSON）。
///
/// `strip` 写完整 `ChannelStripParams` 形状（嵌套对象，与
/// `crates/yeban-dsp/src/channel_strip.rs` 的字段名一致）；其余四件各写完整参数
/// 对象；`sends` 写 `SendToAux` 边表；`aux` / `mix` 只在整轨快照里出现。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// 格式版本，现行值 1。
    pub format: u32,
    /// 标识：`sys:<乐器 slug>:<预制名>` 或 `user:<ulid>`。
    pub id: String,
    /// 显示名（可改，标识不变；凭名引用只认标识）。
    pub name: String,
    /// 适用乐器 slug（须与 `assets/samples/manifest.json` 的条目一致）。
    pub instrument: String,
    /// 乐器 SFZ 路径（库根相对形式，复用 `setTrackSampler` 的改写规则）。
    pub instrument_sfz: String,
    /// 作用域：`instrument`（乐器调优）或 `track`（整轨快照）。
    #[serde(default)]
    pub scope: PresetScope,
    /// 作者。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// 预制自身版本（只改调优参数升 minor）。
    #[serde(
        default = "default_preset_version",
        skip_serializing_if = "String::is_empty"
    )]
    pub version: String,
    /// 草案标记：首批 6 件的数值全是草案值，待耳验（Wang 的耳朵为准）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub draft: bool,
    /// 通道条完整参数（`ChannelStripParams` 形状，启用开关全量写）。
    pub strip: Value,
    /// 饱和参数（完整对象；无则 `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<Value>,
    /// 混响参数（完整对象；无则 `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverb: Option<Value>,
    /// 卷积混响参数（完整对象；无则 `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convolution: Option<Value>,
    /// 延迟参数（完整对象；无则 `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<Value>,
    /// 发送边表。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<PresetSend>,
    /// Aux 定义（只在整轨快照里出现）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aux: Option<Value>,
    /// 混音（`mute` / `solo` / 路由增益；只在整轨快照里出现；**不含
    /// `volume_db` 与 `pan`**，见 §7 已定夺）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mix: Option<Value>,
    /// 宏数组（绑定到通道条或发送参数）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<MacroParameter>,
}

fn default_preset_version() -> String {
    "1.0.0".to_owned()
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Preset {
    /// 校验一份预制字节（64 KiB 上限 → JSON → format → 标识 → 必填字段）。
    ///
    /// # Errors
    ///
    /// 超限、非法 JSON、缺 `format`、major 更新、标识非法、必填字段非法。
    pub fn parse(bytes: &[u8]) -> Result<Self, PresetError> {
        if bytes.len() > PRESET_MAX_BYTES {
            return Err(PresetError::TooLarge {
                actual: bytes.len(),
            });
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|error| PresetError::InvalidJson {
                detail: error.to_string(),
            })?;
        let format = value
            .get("format")
            .and_then(Value::as_u64)
            .ok_or(PresetError::MissingFormat)?;
        let format = u32::try_from(format).map_err(|_| PresetError::MissingFormat)?;
        if format > PRESET_FORMAT_VERSION {
            return Err(PresetError::NewerMajor {
                found: format,
                supported: PRESET_FORMAT_VERSION,
            });
        }
        if format == 0 {
            return Err(PresetError::InvalidField {
                field: "format",
                detail: "format 不得为 0".to_owned(),
            });
        }
        let preset: Self =
            serde_json::from_value(value).map_err(|error| PresetError::InvalidJson {
                detail: error.to_string(),
            })?;
        preset.validate()?;
        Ok(preset)
    }

    /// 校验已解析的预制（标识规则与必填字段）。
    ///
    /// # Errors
    ///
    /// 标识非法、必填字段非法。
    pub fn validate(&self) -> Result<(), PresetError> {
        validate_preset_id(&self.id)?;
        if self.name.trim().is_empty() {
            return Err(PresetError::InvalidField {
                field: "name",
                detail: "名不得为空".to_owned(),
            });
        }
        if self.instrument.trim().is_empty() {
            return Err(PresetError::InvalidField {
                field: "instrument",
                detail: "适用乐器不得为空".to_owned(),
            });
        }
        if self.instrument_sfz.trim().is_empty() {
            return Err(PresetError::InvalidField {
                field: "instrument_sfz",
                detail: "乐器 SFZ 路径不得为空".to_owned(),
            });
        }
        if !self.strip.is_object() {
            return Err(PresetError::InvalidField {
                field: "strip",
                detail: "通道条必须是对象（完整 ChannelStripParams）".to_owned(),
            });
        }
        for send in &self.sends {
            if send.aux.trim().is_empty() {
                return Err(PresetError::InvalidField {
                    field: "sends",
                    detail: "发送目标 Aux 名不得为空".to_owned(),
                });
            }
            if let Some(gain_db) = send.gain_db
                && !gain_db.is_finite()
            {
                return Err(PresetError::InvalidField {
                    field: "sends",
                    detail: "发送增益必须有限".to_owned(),
                });
            }
        }
        Ok(())
    }

    /// 是否系统预制（`sys:` 开头）。
    #[must_use]
    pub fn is_system(&self) -> bool {
        self.id.starts_with("sys:")
    }
}

/// 校验预制标识：`sys:<slug>:<name>` 或 `user:<ulid>`。
///
/// # Errors
///
/// 标识非法。
pub fn validate_preset_id(id: &str) -> Result<(), PresetError> {
    let ok = if let Some(rest) = id.strip_prefix("sys:") {
        let mut parts = rest.split(':');
        let slug = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        !slug.is_empty() && !name.is_empty() && parts.next().is_none()
    } else if let Some(rest) = id.strip_prefix("user:") {
        !rest.is_empty() && !rest.contains(':') && !rest.contains('/')
    } else {
        false
    };
    if ok {
        Ok(())
    } else {
        Err(PresetError::InvalidId { id: id.to_owned() })
    }
}

/// 首批 6 件工厂预制的标识（设计稿 §3，全部是单乐器 SFZ，数值草案待耳验）。
pub const FACTORY_PRESET_IDS: [&str; 6] = [
    "sys:salamander-grand:piano-warm-room",
    "sys:karoryfer-bigcat-cello:cello-warm-hall",
    "sys:freepats-spanish-classical-guitar:guitar-warm-room",
    "sys:ixox-flute:flute-airy-hall",
    "sys:karoryfer-bear-sax:sax-warm-room",
    "sys:karoryfer-big-rusty-drums:drums-punchy-room",
];

/// 某标识是不是首批工厂预制之一。
#[must_use]
pub fn is_factory_preset(id: &str) -> bool {
    FACTORY_PRESET_IDS.contains(&id)
}

/// 乐器 slug → 该乐器的默认系统预制标识（每乐器至多一个默认标记）。
#[must_use]
pub fn default_preset_id_for_instrument(slug: &str) -> Option<&'static str> {
    match slug {
        "salamander-grand" => Some("sys:salamander-grand:piano-warm-room"),
        "karoryfer-bigcat-cello" => Some("sys:karoryfer-bigcat-cello:cello-warm-hall"),
        "freepats-spanish-classical-guitar" => {
            Some("sys:freepats-spanish-classical-guitar:guitar-warm-room")
        }
        "ixox-flute" => Some("sys:ixox-flute:flute-airy-hall"),
        "karoryfer-bear-sax" => Some("sys:karoryfer-bear-sax:sax-warm-room"),
        "karoryfer-big-rusty-drums" => Some("sys:karoryfer-big-rusty-drums:drums-punchy-room"),
        _ => None,
    }
}

/// 乐器 slug → 该乐器的 SFZ 路径（库根相对形式，按
/// `assets/samples/manifest.json` 的实际条目解析；`relative_root` + `sfz`）。
#[must_use]
pub fn instrument_sfz_path(slug: &str) -> Option<&'static str> {
    match slug {
        "salamander-grand" => Some("assets/samples/salamander-grand/Salamander Grand Piano V3.sfz"),
        "karoryfer-bigcat-cello" => {
            Some("assets/samples/karoryfer-bigcat-cello/Programs/01- Bowed (velocity layer).sfz")
        }
        "freepats-spanish-classical-guitar" => Some(
            "assets/samples/freepats-spanish-classical-guitar/SpanishClassicalGuitar-20190618.sfz",
        ),
        "ixox-flute" => Some("assets/samples/ixox-flute/Ixox Flute.sfz"),
        "karoryfer-bear-sax" => Some("assets/samples/karoryfer-bear-sax/Programs/1-solo-mono.sfz"),
        "karoryfer-big-rusty-drums" => {
            Some("assets/samples/karoryfer-big-rusty-drums/Programs/01-full.sfz")
        }
        _ => None,
    }
}

/// SFZ 路径（库根相对或绝对均可，按后缀匹配）→ 乐器 slug。
#[must_use]
pub fn slug_for_sfz_path(path: &str) -> Option<&'static str> {
    for slug in [
        "salamander-grand",
        "karoryfer-bigcat-cello",
        "freepats-spanish-classical-guitar",
        "ixox-flute",
        "karoryfer-bear-sax",
        "karoryfer-big-rusty-drums",
    ] {
        if let Some(sfz) = instrument_sfz_path(slug)
            && path.ends_with(sfz)
        {
            return Some(slug);
        }
    }
    None
}

/// 用户预制库默认位置：用户主目录下预制库，抄 Logic
/// `~/Music/Audio Music Apps/Patches/`（设计稿 §7 已定夺）。
#[must_use]
pub fn user_preset_library_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Music")
        .join("Audio Music Apps")
        .join("Patches")
}

/// 用户预制标识 `user:<ulid>` → 工程包内嵌条目名 `presets/user/<ulid>.json`。
#[must_use]
pub fn embedded_entry_name(preset_id: &str) -> Option<String> {
    let ulid = preset_id.strip_prefix("user:")?;
    if ulid.is_empty() || ulid.contains('/') || ulid.contains(':') {
        return None;
    }
    Some(format!("{EMBEDDED_USER_PRESET_PREFIX}{ulid}.json"))
}

/// 内嵌条目名 → 用户预制标识（`presets/user/<ulid>.json` → `user:<ulid>`）。
#[must_use]
pub fn preset_id_from_embedded_entry(name: &str) -> Option<String> {
    let rest = name.strip_prefix(EMBEDDED_USER_PRESET_PREFIX)?;
    let ulid = rest.strip_suffix(".json")?;
    if ulid.is_empty() || ulid.contains('/') {
        return None;
    }
    Some(format!("user:{ulid}"))
}

/// 旁通快照：只存**处理参数**，不存采样、不存乐器绑定（设计稿 §2.6）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackPresetSnapshot {
    /// 应用前那条轨的通道条设备参数（扁平 `name → value` 对象）。
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub strip: serde_json::Map<String, Value>,
    /// 应用前其余四件的设备参数（按 `saturation` / `reverb` / `convolution` /
    /// `delay` 分组，各为扁平对象；无则空对象）。
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub effects: serde_json::Map<String, Value>,
    /// 应用前的发送边表。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<PresetSend>,
    /// 应用前的宏数组。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<MacroParameter>,
}

/// 音轨上的预制状态（`track.preset`，设计稿 §2.6）。
///
/// 字段为 `None` 时不落盘，旧工程逐字节不变（与 `sampler` 字段同一口径）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackPresetState {
    /// 当前预制标识；为空等于无预制。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<String>,
    /// 旁通：只旁通处理，不换乐器；`true` = 恢复 `previous` 快照。
    #[serde(default, skip_serializing_if = "is_false")]
    pub bypassed: bool,
    /// 默认加载开关；缺省 `true`（绑定乐器有默认系统预制时自动应用）。
    #[serde(default = "default_auto_load", skip_serializing_if = "is_true")]
    pub auto_load: bool,
    /// 最近一次应用前旧值的快照（只保留一份；应用新预制即覆盖）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<TrackPresetSnapshot>,
}

fn default_auto_load() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

impl TrackPresetState {
    /// 校验：标识合法、快照里的宏合法。
    ///
    /// # Errors
    ///
    /// 标识非法或宏非法 → 对应模型错误由调用方映射（本函数只返回描述串，
    /// 模型 `validate()` 的错误类型不新增变体）。
    pub fn check(&self) -> Result<(), String> {
        if let Some(id) = &self.preset_id {
            validate_preset_id(id).map_err(|error| error.to_string())?;
        }
        if let Some(previous) = &self.previous {
            for macro_parameter in &previous.macros {
                macro_parameter
                    .validate()
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
}

/// 把嵌套的 `ChannelStripParams` 形状扁平成 `insert.rs` 认得的设备参数名
/// （`eq_low_gain`、`cutoff_hz`、`threshold_db`、`strip_width` 等）。
/// 只认数值；开关写成 `1.0` / `0.0`。未知键静默跳过（预制文件里的扩展字段
/// 不得让投影失败）。
#[must_use]
pub fn strip_param_pairs(strip: &Value) -> Vec<(String, f32)> {
    let mut pairs: Vec<(String, f32)> = Vec::new();
    let Some(object) = strip.as_object() else {
        return pairs;
    };
    let num = |key: &str, out: &mut Vec<(String, f32)>| {
        if let Some(value) = object.get(key).and_then(Value::as_f64) {
            #[allow(clippy::cast_possible_truncation)]
            out.push((key.to_owned(), value as f32));
        }
    };
    num("input_gain_db", &mut pairs);
    num("output_gain_db", &mut pairs);
    num("width", &mut pairs);
    // `width` 在 insert.rs 里认的名字是 `strip_width`，不是 `width`。
    if let Some(last) = pairs.last_mut()
        && last.0 == "width"
    {
        last.0 = "strip_width".to_owned();
    }
    num("mono_below_hz", &mut pairs);
    num("ms_mid_gain_db", &mut pairs);
    num("ms_mid_freq_hz", &mut pairs);
    for (key, out_key) in [
        ("eq_enabled", "eq_enabled"),
        ("filter_enabled", "filter_enabled"),
        ("compressor_enabled", "compressor_enabled"),
    ] {
        if let Some(flag) = object.get(key).and_then(Value::as_bool) {
            pairs.push((out_key.to_owned(), if flag { 1.0 } else { 0.0 }));
        }
    }
    if let Some(hpf) = object.get("hpf").and_then(Value::as_object)
        && let Some(freq) = hpf.get("freq_hz").and_then(Value::as_f64)
    {
        #[allow(clippy::cast_possible_truncation)]
        pairs.push(("hpf_hz".to_owned(), freq as f32));
    }
    flatten_group(
        object,
        "eq",
        &mut pairs,
        &[
            ("low_gain", "eq_low_gain"),
            ("low_freq", "eq_low_freq"),
            ("mid_gain", "eq_mid_gain"),
            ("mid_freq", "eq_mid_freq"),
            ("mid_q", "eq_mid_q"),
            ("high_gain", "eq_high_gain"),
            ("high_freq", "eq_high_freq"),
        ],
    );
    flatten_group(
        object,
        "filter",
        &mut pairs,
        &[
            ("cutoff_hz", "cutoff_hz"),
            ("resonance", "resonance"),
            ("drive", "drive"),
        ],
    );
    flatten_group(
        object,
        "compressor",
        &mut pairs,
        &[
            ("threshold_db", "threshold_db"),
            ("ratio", "ratio"),
            ("knee_db", "knee_db"),
            ("detector_s", "detector_s"),
            ("attack_s", "attack_s"),
            ("release_s", "release_s"),
            ("makeup_db", "makeup_db"),
        ],
    );
    flatten_group(
        object,
        "dyn_eq",
        &mut pairs,
        &[
            ("freq_hz", "deq_freq"),
            ("threshold_db", "deq_threshold_db"),
            ("max_cut_db", "deq_cut_db"),
            ("q", "deq_q"),
            ("ratio", "deq_ratio"),
        ],
    );
    pairs
}

fn flatten_group(
    object: &serde_json::Map<String, Value>,
    group: &str,
    pairs: &mut Vec<(String, f32)>,
    fields: &[(&str, &str)],
) {
    let Some(sub) = object.get(group).and_then(Value::as_object) else {
        return;
    };
    for (field, out_key) in fields {
        if let Some(value) = sub.get(*field).and_then(Value::as_f64) {
            #[allow(clippy::cast_possible_truncation)]
            pairs.push(((*out_key).to_owned(), value as f32));
        }
    }
}

/// 把四件其余效果（`saturation` / `reverb` / `convolution` / `delay`）的
/// 参数对象扁平成设备参数名：`reverb.size` → `reverb_size` 等（与
/// `insert.rs` §8/§9 登记的名字一致；饱和与延迟沿用同一 `组_字段` 拼法，
/// 未登记的名字由引擎侧忽略，不静默改声）。
#[must_use]
pub fn effect_param_pairs(kind: &str, params: &Value) -> Vec<(String, f32)> {
    let mut pairs: Vec<(String, f32)> = Vec::new();
    let Some(object) = params.as_object() else {
        return pairs;
    };
    let prefix = match kind {
        "reverb" => "reverb",
        "convolution" => "conv",
        "delay" => "delay",
        "saturation" => "sat",
        _ => return pairs,
    };
    for (field, value) in object {
        if let Some(number) = value.as_f64() {
            #[allow(clippy::cast_possible_truncation)]
            pairs.push((format!("{prefix}_{field}"), number as f32));
        }
    }
    pairs
}
