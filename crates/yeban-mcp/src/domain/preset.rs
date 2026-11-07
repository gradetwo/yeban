//! 夜半原生预制（设计稿 §1/§2/§5）的 MCP 领域面：工厂预制登记、
//! 列出 / 解析 / 应用 / 旁通 / 另存、导出内嵌与打开校验。
//!
//! 口径：
//! - 预制应用与保存走**提案路径**（与 `yeban_edit_notes` 同一条路）：
//!   本模块只产 `Op`，由调用方经 `propose_draft` 建提案（`autoMerge` 可当场落盘）。
//! - 预制应用是控制侧操作，不进 RT 回调；加载本身不触发采样解码。
//! - 设备投影复用 `insert.rs` 认得的参数名（见 `yeban_model::preset` 的
//!   `strip_param_pairs` / `effect_param_pairs`）：预制里的通道条与四件
//!   落成名为 `Preset: …` 的内置效果器设备，引擎侧按既有投影读它们。
//! - 宏只能调**已有的**宏位（`Op::SetMacro` 要求下标在界内）；预制里超出
//!   现有宏位数的宏本次跳过并在响应里点名（`macrosSkipped`），不静默丢弃。
//!   （补一条 `AddMacro` Op 是后续项，见本模块末尾的登记。）
//! - 发送只改**已有**边的增益（乐器调优的 §2.5 口径）；目标 Aux 不存在时
//!   本次跳过并在响应里点名（`sendsSkipped`），不增边、不删边、不建 Aux。
//!   整轨快照的 Aux 创建同样是后续项。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Map, Value};

use yeban_model::preset::{
    PRESET_MAX_BYTES, Preset, PresetScope, PresetSend, TrackPresetSnapshot,
    default_preset_id_for_instrument, embedded_entry_name, instrument_sfz_path,
    preset_id_from_embedded_entry, slug_for_sfz_path, strip_param_pairs, user_preset_library_dir,
    validate_preset_id,
};
use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, Op, ParameterValue, RoutingKind, TrackPresetState,
    TrackV3, YebanProjectV1,
};

use super::error::{Fault, from_model};
use super::ids::deterministic_id;
use crate::tools::ErrorCode;

/// 预制管理的设备名前缀：预制写进设备链的设备一律以此前缀开头，
///
/// 快照与重应用时凭此前缀认出“哪些设备是预制管的”。
pub const PRESET_DEVICE_PREFIX: &str = "Preset: ";

/// 五类应用开关（设计稿 §2.5）：`instrument` / `strip` / `effects` / `sends` / `macros`。
pub const PRESET_CATEGORIES: [&str; 5] = ["instrument", "strip", "effects", "sends", "macros"];

/// 工厂预制登记：标识 → 仓库 `assets/presets/` 里的 JSON（编译期内嵌）。
const FACTORY_PRESETS: [(&str, &str); 6] = [
    (
        "sys:salamander-grand:piano-warm-room",
        include_str!("../../../../assets/presets/piano-warm-room.json"),
    ),
    (
        "sys:karoryfer-bigcat-cello:cello-warm-hall",
        include_str!("../../../../assets/presets/cello-warm-hall.json"),
    ),
    (
        "sys:freepats-spanish-classical-guitar:guitar-warm-room",
        include_str!("../../../../assets/presets/guitar-warm-room.json"),
    ),
    (
        "sys:ixox-flute:flute-airy-hall",
        include_str!("../../../../assets/presets/flute-airy-hall.json"),
    ),
    (
        "sys:karoryfer-bear-sax:sax-warm-room",
        include_str!("../../../../assets/presets/sax-warm-room.json"),
    ),
    (
        "sys:karoryfer-big-rusty-drums:drums-punchy-room",
        include_str!("../../../../assets/presets/drums-punchy-room.json"),
    ),
];

/// 取一份工厂预制（按标识）；未知标识 ⇒ `None`。
#[must_use]
pub fn factory_preset(id: &str) -> Option<Preset> {
    FACTORY_PRESETS
        .iter()
        .find(|(known, _)| *known == id)
        .and_then(|(_, text)| Preset::parse(text.as_bytes()).ok())
}

/// 全部工厂预制（按登记顺序）。
#[must_use]
pub fn factory_presets() -> Vec<Preset> {
    FACTORY_PRESETS
        .iter()
        .filter_map(|(id, _)| factory_preset(id))
        .collect()
}

/// 预制解析失败 → 契约错误（格式问题是调用方参数问题，不是内部错误）。
fn preset_fault(message: String) -> Fault {
    Fault::domain(ErrorCode::InvalidParameterRange, message)
}

/// 按标识解析一份预制：系统预制查发行表，用户预制先查工程内嵌副本，
///
/// 再查用户预制库。查不到 ⇒ `ENTITY_NOT_FOUND`（响亮报错，不静默回干声）。
///
/// # Errors
///
/// 未知标识、内嵌/库文件损坏、标识非法。
pub fn resolve_preset(embedded: &BTreeMap<String, Preset>, id: &str) -> Result<Preset, Fault> {
    validate_preset_id(id).map_err(|error| preset_fault(error.to_string()))?;
    if id.starts_with("sys:") {
        return factory_preset(id).ok_or_else(|| {
            Fault::domain(
                ErrorCode::EntityNotFound,
                format!("发行预制表查不到系统预制 `{id}`，拒绝静默回干声"),
            )
        });
    }
    if let Some(preset) = embedded.get(id) {
        return Ok(preset.clone());
    }
    let path = user_preset_path(id);
    let bytes = std::fs::read(&path).map_err(|_| {
        Fault::domain(
            ErrorCode::EntityNotFound,
            format!(
                "缺用户预制 `{id}`（用户库里没有 {}），拒绝静默回干声",
                path.display()
            ),
        )
    })?;
    Preset::parse(&bytes).map_err(|error| preset_fault(format!("用户预制 `{id}` 损坏：{error}")))
}

/// 用户预制标识 → 用户库里的文件路径。
#[must_use]
pub fn user_preset_path(id: &str) -> PathBuf {
    let file = id.strip_prefix("user:").unwrap_or(id);
    user_preset_library_dir().join(format!("{file}.json"))
}

/// 读用户预制库里的全部预制（解析失败的逐条记进 `skipped`，不让一条坏文件
/// 拦住整个列表；列表本身不是写操作）。
#[must_use]
pub fn user_library_presets() -> (Vec<(Preset, PathBuf)>, Vec<String>) {
    let dir = user_preset_library_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (Vec::new(), Vec::new());
    };
    let mut presets = Vec::new();
    let mut skipped = Vec::new();
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    for path in paths {
        match std::fs::read(&path).and_then(|bytes| {
            Preset::parse(&bytes).map_err(|error| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
            })
        }) {
            Ok(preset) => presets.push((preset, path)),
            Err(_) => skipped.push(path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            )),
        }
    }
    (presets, skipped)
}

/// 列出预制（`yeban_list_presets` 的读数部分；不要求活跃工程）。
#[must_use]
pub fn list_presets_json(kind: &str, instrument: Option<&str>) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    if kind == "system" || kind == "all" {
        for preset in factory_presets() {
            if let Some(want) = instrument
                && preset.instrument != want
            {
                continue;
            }
            rows.push(serde_json::json!({
                "id": preset.id,
                "name": preset.name,
                "kind": "system",
                "instrument": preset.instrument,
                "instrumentSfz": preset.instrument_sfz,
                "scope": if preset.scope == PresetScope::Track { "track" } else { "instrument" },
                "draft": preset.draft,
                "default": default_preset_id_for_instrument(&preset.instrument).is_some_and(|id| id == preset.id),
                "macros": preset.macros.iter().map(|macro_parameter| macro_parameter.name.clone()).collect::<Vec<_>>(),
                "sends": preset.sends,
            }));
        }
    }
    let mut skipped: Vec<String> = Vec::new();
    if kind == "user" || kind == "all" {
        let (user_presets, bad) = user_library_presets();
        skipped = bad;
        for (preset, path) in user_presets {
            if let Some(want) = instrument
                && preset.instrument != want
            {
                continue;
            }
            rows.push(serde_json::json!({
                "id": preset.id,
                "name": preset.name,
                "kind": "user",
                "instrument": preset.instrument,
                "instrumentSfz": preset.instrument_sfz,
                "scope": if preset.scope == PresetScope::Track { "track" } else { "instrument" },
                "draft": preset.draft,
                "default": false,
                "path": path.display().to_string(),
                "macros": preset.macros.iter().map(|macro_parameter| macro_parameter.name.clone()).collect::<Vec<_>>(),
                "sends": preset.sends,
            }));
        }
    }
    serde_json::json!({
        "presets": rows,
        "count": rows.len(),
        "skipped": skipped,
        "userLibraryDir": user_preset_library_dir().display().to_string(),
    })
}

/// 打开工程时校验全部轨引用：系统预制必须在发行表里，用户预制必须在
/// 内嵌副本或用户库里。查不到 ⇒ 响亮报错，不静默回干声（设计稿 §5）。
///
/// # Errors
///
/// 任一引用查不到。
pub fn validate_preset_references(
    project: &YebanProjectV1,
    embedded: &BTreeMap<String, Preset>,
) -> Result<(), Fault> {
    for track in project.tracks.values() {
        let Some(state) = &track.preset else { continue };
        let Some(id) = &state.preset_id else { continue };
        resolve_preset(embedded, id)?;
    }
    Ok(())
}

/// 解析 `categories` 实参：缺省 = 全部五类；未知类名 ⇒ 响亮拒绝。
///
/// # Errors
///
/// 类别值不是字符串数组，或含未知类名。
pub fn parse_categories(raw: Option<&Value>) -> Result<Vec<&'static str>, Fault> {
    let Some(raw) = raw else {
        return Ok(PRESET_CATEGORIES.to_vec());
    };
    let Some(items) = raw.as_array() else {
        return Err(preset_fault("`categories` 必须是字符串数组".to_owned()));
    };
    let mut out: Vec<&'static str> = Vec::new();
    for item in items {
        let Some(name) = item.as_str() else {
            return Err(preset_fault(
                "`categories` 里的每一项必须是字符串".to_owned(),
            ));
        };
        let known = PRESET_CATEGORIES
            .iter()
            .find(|candidate| **candidate == name)
            .copied()
            .ok_or_else(|| {
                preset_fault(format!(
                    "`categories` 含未知类别 `{name}`（只认 instrument/strip/effects/sends/macros）"
                ))
            })?;
        if !out.contains(&known) {
            out.push(known);
        }
    }
    Ok(out)
}

/// 一次预制应用的规划结果（`Op` + 响应里如实报告的跳过项）。
#[derive(Debug)]
pub struct ApplyPlan {
    /// 将要施加的 `Op`（提案路径）。
    pub ops: Vec<Op>,
    /// 跳过的发送（目标 Aux 不存在，只改已有边）。
    pub sends_skipped: Vec<String>,
    /// 跳过的宏（超出当前宏位数，`SetMacro` 够不到）。
    pub macros_skipped: usize,
    /// 实际动了的类别。
    pub categories: Vec<String>,
}

/// 给一条轨拍一张处理快照（只存处理参数，不存采样与乐器绑定）。
///
/// # Errors
///
/// 音轨不存在。
pub fn snapshot_track(
    project: &YebanProjectV1,
    track_id: &EntityId,
) -> Result<TrackPresetSnapshot, Fault> {
    let track = project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    let mut strip: Map<String, Value> = Map::new();
    let mut effects: Map<String, Value> = Map::new();
    for device in &track.devices {
        let Some(name) = device.name.strip_prefix(PRESET_DEVICE_PREFIX) else {
            continue;
        };
        let mut params: Map<String, Value> = Map::new();
        for param in &device.params {
            params.insert(param.name.clone(), Value::from(f64::from(param.value)));
        }
        match name {
            "Channel Strip" => strip = params,
            "Saturation" => {
                effects.insert("saturation".to_owned(), Value::Object(params));
            }
            "Reverb" => {
                effects.insert("reverb".to_owned(), Value::Object(params));
            }
            "Convolution" => {
                effects.insert("convolution".to_owned(), Value::Object(params));
            }
            "Delay" => {
                effects.insert("delay".to_owned(), Value::Object(params));
            }
            _ => {}
        }
    }
    let mut sends: Vec<PresetSend> = Vec::new();
    for edge in project.routing_graph.edges.values() {
        if edge.kind != RoutingKind::SendToAux || edge.source_node != *track_id {
            continue;
        }
        let Ok(destination) = project.track(&edge.destination_node) else {
            continue;
        };
        sends.push(PresetSend {
            aux: destination.name.clone(),
            gain_db: edge.gain_db,
            effect: None,
        });
    }
    Ok(TrackPresetSnapshot {
        strip,
        effects,
        sends,
        macros: track.macros.clone(),
    })
}

/// 预制管理的设备（`Preset: …`）在设备链里的下标（升序）。
fn preset_device_slots(track: &TrackV3) -> Vec<usize> {
    track
        .devices
        .iter()
        .enumerate()
        .filter(|(_, device)| device.name.starts_with(PRESET_DEVICE_PREFIX))
        .map(|(index, _)| index)
        .collect()
}

/// 造一台预制设备（确定性身份：dryRun 预览与真调用的 `Op` 逐字节相同）。
fn preset_device(
    track_id: &EntityId,
    preset_id: &str,
    label: &str,
    params: Vec<(String, f32)>,
) -> DeviceDefinition {
    DeviceDefinition {
        id: deterministic_id(&format!("preset:{preset_id}:{track_id}:{label}")),
        name: format!("{PRESET_DEVICE_PREFIX}{label}"),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .into_iter()
            .map(|(name, value)| ParameterValue {
                name,
                value,
                unit: None,
            })
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

/// 把一份预制应用到一条轨的 `Op` 规划（提案路径；不直接改工程）。
///
/// 顺序：先拆旧预制设备（下标降序），再追加新设备，最后写发送增益、
/// 宏、乐器绑定与 `track.preset`。`previous` 快照取自应用前的轨状态。
///
/// # Errors
///
/// 音轨不存在、类别非法、乐器调优的发送目标不存在时不报错（跳过并报告）。
pub fn build_apply_ops(
    project: &YebanProjectV1,
    track_id: &EntityId,
    preset: &Preset,
    categories: &[&str],
) -> Result<ApplyPlan, Fault> {
    let track = project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    let snapshot = snapshot_track(project, track_id)?;
    let old_state = track.preset.as_deref().cloned();
    let mut ops: Vec<Op> = Vec::new();
    let mut sends_skipped: Vec<String> = Vec::new();
    let mut macros_skipped: usize = 0;
    let mut applied: Vec<String> = Vec::new();
    let mut device_len = track.devices.len();

    // ① 拆旧预制设备（下标降序：前面的下标不受影响）。
    let mut slots = preset_device_slots(track);
    slots.sort_unstable_by(|a, b| b.cmp(a));
    for slot in slots {
        ops.push(Op::RemoveDevice {
            track_id: *track_id,
            slot_index: slot,
            previous_device: track.devices[slot].clone(),
        });
        device_len -= 1;
    }
    // ② 通道条（strip 类）。
    if categories.contains(&"strip") {
        let params = strip_param_pairs(&preset.strip);
        if !params.is_empty() {
            ops.push(Op::InsertDevice {
                track_id: *track_id,
                slot_index: device_len,
                device: preset_device(track_id, &preset.id, "Channel Strip", params),
            });
            device_len += 1;
        }
        applied.push("strip".to_owned());
    }
    // ③ 其余四件（effects 类）。
    if categories.contains(&"effects") {
        for (kind, label, params) in [
            ("saturation", "Saturation", preset.saturation.as_ref()),
            ("reverb", "Reverb", preset.reverb.as_ref()),
            ("convolution", "Convolution", preset.convolution.as_ref()),
            ("delay", "Delay", preset.delay.as_ref()),
        ] {
            let Some(params) = params else { continue };
            let pairs = yeban_model::preset::effect_param_pairs(kind, params);
            if pairs.is_empty() {
                continue;
            }
            ops.push(Op::InsertDevice {
                track_id: *track_id,
                slot_index: device_len,
                device: preset_device(track_id, &preset.id, label, pairs),
            });
            device_len += 1;
        }
        applied.push("effects".to_owned());
    }
    // ④ 发送：只改已有边的增益（§2.5：不增边、不删边、不建 Aux）。
    if categories.contains(&"sends") {
        for send in &preset.sends {
            let edge = project.routing_graph.edges.values().find(|edge| {
                edge.kind == RoutingKind::SendToAux
                    && edge.source_node == *track_id
                    && project
                        .track(&edge.destination_node)
                        .is_ok_and(|destination| destination.name == send.aux)
            });
            match edge {
                Some(edge) => ops.push(Op::SetRoutingGain {
                    edge_id: edge.id,
                    old_gain_db: edge.gain_db,
                    new_gain_db: send.gain_db,
                }),
                None => sends_skipped.push(send.aux.clone()),
            }
        }
        applied.push("sends".to_owned());
    }
    // ⑤ 宏：只调已有的宏位。
    if categories.contains(&"macros") {
        for (index, macro_parameter) in preset.macros.iter().enumerate() {
            if index < track.macros.len() {
                ops.push(Op::SetMacro {
                    track_id: *track_id,
                    macro_index: index,
                    old_val: track.macros[index].value,
                    new_val: macro_parameter.value,
                });
            } else {
                macros_skipped += 1;
            }
        }
        applied.push("macros".to_owned());
    }
    // ⑥ 乐器绑定（instrument 类）：只在绑定变化时写。
    if categories.contains(&"instrument") {
        let new_sampler = Some(yeban_model::SamplerBinding {
            path: preset.instrument_sfz.clone(),
        });
        if track.sampler != new_sampler {
            ops.push(Op::SetTrackSampler {
                track_id: *track_id,
                old_sampler: track.sampler.clone(),
                new_sampler,
            });
        }
        applied.push("instrument".to_owned());
    }
    // ⑦ 轨预制状态（`previous` = 应用前快照，只保留一份）。
    let auto_load = old_state.as_ref().is_none_or(|state| state.auto_load);
    ops.push(Op::SetTrackPreset {
        track_id: *track_id,
        old_preset: old_state,
        new_preset: Some(TrackPresetState {
            preset_id: Some(preset.id.clone()),
            bypassed: false,
            auto_load,
            previous: Some(snapshot),
        }),
    });
    Ok(ApplyPlan {
        ops,
        sends_skipped,
        macros_skipped,
        categories: applied,
    })
}

/// 旁通开关的 `Op` 规划。
///
/// - `bypassed = true`：恢复 `previous` 快照（通道条、四件、发送、宏），
///   乐器绑定不变；没有快照 ⇒ 响亮拒绝。
/// - `bypassed = false`：按当前 `preset_id` 重新应用该预制（`previous`
///   换成旁通前那一刻的状态）。
///
/// # Errors
///
/// 音轨不存在、无预制、无快照（旁通时）、预制查不到（取消旁通时）。
pub fn build_bypass_ops(
    project: &YebanProjectV1,
    track_id: &EntityId,
    bypassed: bool,
    embedded: &BTreeMap<String, Preset>,
) -> Result<(Vec<Op>, Vec<String>, usize), Fault> {
    let track = project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    let state = track
        .preset
        .as_deref()
        .cloned()
        .ok_or_else(|| preset_fault(format!("音轨 {track_id} 没有预制状态，旁通无从谈起")))?;
    if !bypassed {
        let Some(id) = &state.preset_id else {
            return Err(preset_fault(format!(
                "音轨 {track_id} 没有当前预制，无法取消旁通"
            )));
        };
        let preset = resolve_preset(embedded, id)?;
        let categories = PRESET_CATEGORIES.to_vec();
        let plan = build_apply_ops(project, track_id, &preset, &categories)?;
        return Ok((plan.ops, plan.sends_skipped, plan.macros_skipped));
    }
    let Some(previous) = &state.previous else {
        return Err(preset_fault(format!(
            "音轨 {track_id} 没有应用前快照（previous 为空），旁通无法恢复"
        )));
    };
    let mut ops: Vec<Op> = Vec::new();
    let mut sends_skipped: Vec<String> = Vec::new();
    let mut macros_skipped: usize = 0;
    let mut device_len = track.devices.len();
    let mut slots = preset_device_slots(track);
    slots.sort_unstable_by(|a, b| b.cmp(a));
    for slot in slots {
        ops.push(Op::RemoveDevice {
            track_id: *track_id,
            slot_index: slot,
            previous_device: track.devices[slot].clone(),
        });
        device_len -= 1;
    }
    // 快照里的通道条与四件按快照原样装回去（设备名与应用时同一套）。
    if !previous.strip.is_empty() {
        let params: Vec<(String, f32)> = previous
            .strip
            .iter()
            .filter_map(|(name, value)| value.as_f64().map(|number| (name.clone(), number as f32)))
            .collect();
        ops.push(Op::InsertDevice {
            track_id: *track_id,
            slot_index: device_len,
            device: preset_device(
                track_id,
                state.preset_id.as_deref().unwrap_or(""),
                "Channel Strip",
                params,
            ),
        });
        device_len += 1;
    }
    for (kind, label) in [
        ("saturation", "Saturation"),
        ("reverb", "Reverb"),
        ("convolution", "Convolution"),
        ("delay", "Delay"),
    ] {
        let Some(params) = previous.effects.get(kind).and_then(Value::as_object) else {
            continue;
        };
        let pairs: Vec<(String, f32)> = params
            .iter()
            .filter_map(|(name, value)| value.as_f64().map(|number| (name.clone(), number as f32)))
            .collect();
        if pairs.is_empty() {
            continue;
        }
        ops.push(Op::InsertDevice {
            track_id: *track_id,
            slot_index: device_len,
            device: preset_device(
                track_id,
                state.preset_id.as_deref().unwrap_or(""),
                label,
                pairs,
            ),
        });
        device_len += 1;
    }
    // 发送恢复到快照里的增益（边不存在则跳过并报告）。
    for send in &previous.sends {
        let edge = project.routing_graph.edges.values().find(|edge| {
            edge.kind == RoutingKind::SendToAux
                && edge.source_node == *track_id
                && project
                    .track(&edge.destination_node)
                    .is_ok_and(|destination| destination.name == send.aux)
        });
        match edge {
            Some(edge) => ops.push(Op::SetRoutingGain {
                edge_id: edge.id,
                old_gain_db: edge.gain_db,
                new_gain_db: send.gain_db,
            }),
            None => sends_skipped.push(send.aux.clone()),
        }
    }
    // 宏恢复到快照里的位置（只调已有的宏位）。
    for (index, macro_parameter) in previous.macros.iter().enumerate() {
        if index < track.macros.len() {
            ops.push(Op::SetMacro {
                track_id: *track_id,
                macro_index: index,
                old_val: track.macros[index].value,
                new_val: macro_parameter.value,
            });
        } else {
            macros_skipped += 1;
        }
    }
    ops.push(Op::SetTrackPreset {
        track_id: *track_id,
        old_preset: Some(state.clone()),
        new_preset: Some(TrackPresetState {
            preset_id: state.preset_id.clone(),
            bypassed: true,
            auto_load: state.auto_load,
            previous: Some(previous.clone()),
        }),
    });
    Ok((ops, sends_skipped, macros_skipped))
}

/// 把扁平的通道条参数对嵌回 `ChannelStripParams` 形状（另存用）。
#[must_use]
pub fn nest_strip(pairs: &[(String, f32)]) -> Value {
    let get = |name: &str| -> Option<f64> {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| f64::from(*value))
    };
    let group = |prefix: &str, fields: &[&str]| -> Map<String, Value> {
        let mut map = Map::new();
        for field in fields {
            let key = format!("{prefix}{field}");
            if let Some(value) = get(&key) {
                map.insert((*field).to_owned(), Value::from(value));
            }
        }
        map
    };
    let mut strip = Map::new();
    for key in [
        "input_gain_db",
        "output_gain_db",
        "width",
        "mono_below_hz",
        "ms_mid_gain_db",
        "ms_mid_freq_hz",
    ] {
        if let Some(value) = get(key) {
            let out_key = if key == "width" { "width" } else { key };
            strip.insert(out_key.to_owned(), Value::from(value));
        }
    }
    // `strip_width` 是投影名，嵌回时写回 `width`。
    if let Some(value) = get("strip_width") {
        strip.insert("width".to_owned(), Value::from(value));
    }
    for key in ["eq_enabled", "filter_enabled", "compressor_enabled"] {
        if let Some(value) = get(key) {
            strip.insert(key.to_owned(), Value::from(value > 0.0));
        }
    }
    if let Some(value) = get("hpf_hz") {
        strip.insert("hpf".to_owned(), serde_json::json!({"freq_hz": value}));
    }
    let eq = group(
        "eq_",
        &[
            "low_gain",
            "low_freq",
            "mid_gain",
            "mid_freq",
            "mid_q",
            "high_gain",
            "high_freq",
        ],
    );
    if !eq.is_empty() {
        strip.insert("eq".to_owned(), Value::Object(eq));
    }
    let filter = group("", &["cutoff_hz", "resonance", "drive"]);
    if !filter.is_empty() {
        strip.insert("filter".to_owned(), Value::Object(filter));
    }
    let compressor = group(
        "",
        &[
            "threshold_db",
            "ratio",
            "knee_db",
            "detector_s",
            "attack_s",
            "release_s",
            "makeup_db",
        ],
    );
    if !compressor.is_empty() {
        strip.insert("compressor".to_owned(), Value::Object(compressor));
    }
    let dyn_eq = group(
        "deq_",
        &["freq_hz", "threshold_db", "max_cut_db", "q", "ratio"],
    );
    if !dyn_eq.is_empty() {
        strip.insert("dyn_eq".to_owned(), Value::Object(dyn_eq));
    }
    Value::Object(strip)
}

/// 把扁平的效果参数对嵌回对应器件的参数对象（另存用）。
#[must_use]
pub fn nest_effect(kind: &str, pairs: &[(String, f32)]) -> Value {
    let mut map = Map::new();
    let prefix = match kind {
        "reverb" => "reverb_",
        "convolution" => "conv_",
        "delay" => "delay_",
        "saturation" => "sat_",
        _ => return Value::Object(map),
    };
    for (name, value) in pairs {
        if let Some(field) = name.strip_prefix(prefix) {
            // `reverb_wet` / `reverb_wet_mix` 都是 mix；`conv_pre_delay_s` / `conv_predelay_s` 都是 pre_delay_s。
            let field = match field {
                "wet_mix" => "mix",
                "pre_delay_s" | "predelay" | "predelay_s" => "pre_delay_s",
                other => other,
            };
            // 延迟的 ping_pong 是布尔值：f32 参数里 1.0/0.0 记成布尔。
            if kind == "delay" && field == "ping_pong" {
                map.insert(field.to_owned(), Value::from(*value != 0.0));
            } else {
                map.insert(field.to_owned(), Value::from(f64::from(*value)));
            }
        }
    }
    Value::Object(map)
}

/// 从当前轨状态另存一份用户预制（**纯构造，不写盘**：写盘是 `apply` 的事，
/// 因此 `dryRun` 拿到的预览与真调用是同一份已校验的预制）。
///
/// 名重复时响亮拒绝（`overwrite: true` 才覆盖同一标识的文件）。
///
/// # Errors
///
/// 音轨不存在、轨未绑定已知乐器、名重复且未显式覆盖、序列化超 64 KiB。
pub fn build_save_preset(
    project: &YebanProjectV1,
    track_id: &EntityId,
    name: &str,
    scope: PresetScope,
    overwrite: bool,
) -> Result<(Preset, PathBuf, bool), Fault> {
    let track = project
        .track(track_id)
        .map_err(|error| from_model("音轨查找", &error))?;
    if name.trim().is_empty() {
        return Err(preset_fault("`name` 不得为空".to_owned()));
    }
    let Some(sampler) = &track.sampler else {
        return Err(preset_fault(format!(
            "音轨 {track_id} 未绑定采样器，无法确定适用乐器"
        )));
    };
    let Some(slug) = slug_for_sfz_path(&sampler.path) else {
        return Err(preset_fault(format!(
            "音轨 {track_id} 绑定的采样器 `{}` 不在工厂乐器登记里，无法另存为预制",
            sampler.path
        )));
    };
    let (existing, _) = user_library_presets();
    let duplicate = existing.iter().find(|(preset, _)| preset.name == name);
    let (id, path, overwritten) = match duplicate {
        Some((preset, path)) if overwrite => (preset.id.clone(), path.clone(), true),
        Some(_) => {
            return Err(Fault::domain(
                ErrorCode::Conflict,
                format!(
                    "用户库里已有同名预制 `{name}`，不静默覆盖（显式给 `overwrite: true` 才覆盖）"
                ),
            ));
        }
        None => {
            let id = format!("user:{}", EntityId::new().to_canonical_string());
            (id.clone(), user_preset_path(&id), false)
        }
    };
    let snapshot = snapshot_track(project, track_id)?;
    let strip_pairs: Vec<(String, f32)> = snapshot
        .strip
        .iter()
        .filter_map(|(name, value)| value.as_f64().map(|number| (name.clone(), number as f32)))
        .collect();
    // 未经预制的轨没有 `Preset:` 设备：快照是空的 ⇒ 通道条取“当前设备链里
    // 认得的通道条参数”——这里只认预制设备；空快照另存为平坦通道条（显式写出，
    // 不猜设备链里其它设备的用途）。
    let strip = if strip_pairs.is_empty() {
        nest_strip(&[])
    } else {
        nest_strip(&strip_pairs)
    };
    let mut preset = Preset {
        format: yeban_model::preset::PRESET_FORMAT_VERSION,
        id: id.clone(),
        name: name.to_owned(),
        instrument: slug.to_owned(),
        instrument_sfz: instrument_sfz_path(slug).unwrap_or("").to_owned(),
        scope,
        author: String::new(),
        version: "1.0.0".to_owned(),
        draft: false,
        strip,
        saturation: None,
        reverb: None,
        convolution: None,
        delay: None,
        sends: snapshot.sends.clone(),
        aux: None,
        mix: if scope == PresetScope::Track {
            Some(serde_json::json!({"mute": track.mute, "solo": track.solo}))
        } else {
            None
        },
        macros: track.macros.clone(),
    };
    for (kind, slot) in [
        ("saturation", &mut preset.saturation),
        ("reverb", &mut preset.reverb),
        ("convolution", &mut preset.convolution),
        ("delay", &mut preset.delay),
    ] {
        let Some(params) = snapshot.effects.get(kind).and_then(Value::as_object) else {
            continue;
        };
        let pairs: Vec<(String, f32)> = params
            .iter()
            .filter_map(|(name, value)| value.as_f64().map(|number| (name.clone(), number as f32)))
            .collect();
        if !pairs.is_empty() {
            *slot = Some(nest_effect(kind, &pairs));
        }
    }
    let bytes = serde_json::to_string_pretty(&preset)
        .map_err(|error| preset_fault(format!("预制序列化失败：{error}")))?;
    if bytes.len() > PRESET_MAX_BYTES {
        return Err(preset_fault(format!(
            "预制超过 64 KiB 上限（{} 字节），拒绝保存",
            bytes.len()
        )));
    }
    Ok((preset, path, overwritten))
}

/// 把一份已校验的用户预制写进用户库（`yeban_save_preset` 的 `apply`）。
///
/// # Errors
///
/// 序列化超 64 KiB、建库目录失败、写盘失败。
pub fn write_user_preset_file(path: &std::path::Path, preset: &Preset) -> Result<(), Fault> {
    let bytes = serde_json::to_string_pretty(preset)
        .map_err(|error| preset_fault(format!("预制序列化失败：{error}")))?;
    if bytes.len() > PRESET_MAX_BYTES {
        return Err(preset_fault(format!(
            "预制超过 64 KiB 上限（{} 字节），拒绝保存",
            bytes.len()
        )));
    }
    let dir = user_preset_library_dir();
    std::fs::create_dir_all(&dir).map_err(|error| {
        Fault::domain(
            ErrorCode::IoError,
            format!("建用户预制库 {} 失败：{error}", dir.display()),
        )
    })?;
    std::fs::write(path, format!("{bytes}\n")).map_err(|error| {
        Fault::domain(
            ErrorCode::IoError,
            format!("写用户预制 {} 失败：{error}", path.display()),
        )
    })?;
    Ok(())
}

/// 保存时要内嵌进工程包的用户预制（设计稿 §5）：轨引用的每个 `user:` 预制
/// 先查会话内嵌副本，再查用户库；`include = false` 时一个都不带（接收方
/// 打开缺预制会响亮报错）；查不到 ⇒ 响亮拒绝保存，不静默写出缺件工程。
///
/// # Errors
///
/// 任一引用的用户预制查不到，或内嵌字节超 64 KiB。
pub fn collect_embedded_presets(
    embedded: &BTreeMap<String, Preset>,
    project: &YebanProjectV1,
    include: bool,
) -> Result<Vec<(String, Vec<u8>)>, Fault> {
    if !include {
        return Ok(Vec::new());
    }
    let mut ids: Vec<&str> = Vec::new();
    for track in project.tracks.values() {
        if let Some(state) = &track.preset
            && let Some(id) = &state.preset_id
            && id.starts_with("user:")
            && !ids.contains(&id.as_str())
        {
            ids.push(id.as_str());
        }
    }
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for id in ids {
        let preset = resolve_preset(embedded, id)?;
        let bytes = serde_json::to_vec(&preset)
            .map_err(|error| preset_fault(format!("内嵌预制 `{id}` 序列化失败：{error}")))?;
        if bytes.len() > PRESET_MAX_BYTES {
            return Err(preset_fault(format!(
                "内嵌预制 `{id}` 超过 64 KiB 上限（{} 字节）",
                bytes.len()
            )));
        }
        let Some(entry) = embedded_entry_name(id) else {
            return Err(preset_fault(format!(
                "用户预制标识 `{id}` 无法映射成包内条目名"
            )));
        };
        out.push((entry, bytes));
    }
    out.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(out)
}

/// 解析并校验容器里的内嵌用户预制条目（名 → 标识，字节 → 预制）。
///
/// # Errors
///
/// 条目名非法、预制解析失败、条目里的标识与条目名不符。
pub fn parse_embedded_presets(
    entries: &[(String, Vec<u8>)],
) -> Result<BTreeMap<String, Preset>, Fault> {
    let mut map: BTreeMap<String, Preset> = BTreeMap::new();
    for (entry_name, bytes) in entries {
        let Some(id) = embedded_preset_id(entry_name) else {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("内嵌预制条目名非法：`{entry_name}`"),
            ));
        };
        let preset = Preset::parse(bytes).map_err(|error| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("内嵌预制 `{id}` 解析失败：{error}"),
            )
        })?;
        if preset.id != id {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!(
                    "内嵌预制条目 `{entry_name}` 里的标识是 `{}`，与条目名不符",
                    preset.id
                ),
            ));
        }
        map.insert(id, preset);
    }
    Ok(map)
}

/// 内嵌条目名 → 预制标识（容器读回时的登记口径）。
#[must_use]
pub fn embedded_preset_id(entry_name: &str) -> Option<String> {
    preset_id_from_embedded_entry(entry_name)
}
