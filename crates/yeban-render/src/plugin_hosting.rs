//! 离线渲染的插件宿主装配（`plugin_hosting`）：**首台胜出**的设备选择、
//! 验证加载与状态恢复。
//!
//! 本模块是 GUI 侧两条离线路径（`yeban-app` 的导出／录音核心与本 crate 的
//! 整曲预览）共用的**唯一**装配口径，与 MCP 离线渲染
//! （`crates/yeban-mcp/src/domain/render.rs` 的 `hosted_plugin_index`／
//! `hosted_effect_index`）逐条同义，⛔ 不另立第二套规则：
//!
//! 1. **首台胜出**：每轨至多一台已装载的乐器与一台已装载的效果器执行 ——
//!    各取首个**非旁通、带插件绑定**的设备；其余外置设备（未绑定、第二台
//!    及以上绑定设备）一律不执行，进未执行收据（[`unhosted_external_devices`]）。
//!    旁通的设备不进收据（旁通是显式选择，不是装配失败，与 MCP 同口径）。
//! 2. **加载即验证**：绑定路径在 `dlopen` 之前先过 [`Host::validate_plugin_path`]
//!    白名单；加载／状态恢复失败 ⇒ 调用方响亮失败，⛔ 不静默回退到默认插件
//!    或干声（与 MCP 的 `RENDER_FAILED` 同一条纪律）。
//! 3. **状态恢复**：T-FIX-345 起装载时刻抓的状态快照随绑定进工程；渲染前
//!    恢复（空 = 默认）。状态损坏／恢复失败 ⇒ 响亮失败，⛔ 不按默认状态渲染。
//!
//! ## 实时播放不在此列
//!
//! 本模块只服务**离线**渲染（`AudioSource::render_block` 允许分配、允许调
//! 用第三方插件的阻塞 FFI）。实时音频回调内执行任意 VST3 插件无法满足
//! AGENTS.md §2 红线 7（回调内禁分配／锁／IO），且引擎没有插件实例的无锁
//! 生命周期、MIDI 路由与延迟补偿 —— 那是另立路线的工程，不在本模块假装
//! 接通。实时侧的口径见 `yeban-app` 的 `PLUGIN_REGISTRATION_NOTE`
//! （“仅登记”，显式 `not executed`）。

use yeban_model::{DeviceKind, PluginBinding, TrackV3};
use yeban_plugin_host::{Host, PluginInstance, PluginState};

/// 一期插件块上限（`setupProcessing` 的块大小；调用方保证 ≥ 渲染计划的块大小，
/// 与 MCP 渲染装配同一个数）。
pub const PLUGIN_MAX_BLOCK: usize = 8192;

/// 首个非旁通、带插件绑定的外置乐器设备下标（`None` = 本轨没有已装载的乐器）。
#[must_use]
pub fn hosted_instrument_index(track: &TrackV3) -> Option<usize> {
    track.devices.iter().position(|device| {
        !device.bypassed && device.kind == DeviceKind::ExternalInstrument && device.plugin.is_some()
    })
}

/// 首个非旁通、带插件绑定的外置效果器设备下标（`None` = 本轨没有已装载的效果器）。
#[must_use]
pub fn hosted_effect_index(track: &TrackV3) -> Option<usize> {
    track.devices.iter().position(|device| {
        !device.bypassed && device.kind == DeviceKind::ExternalEffect && device.plugin.is_some()
    })
}

/// 一台没有被宿主的外置设备（进未执行收据）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnhostedExternal {
    /// 设备在 `track.devices` 里的下标。
    pub index: usize,
    /// 设备种类（只会是 `ExternalInstrument` / `ExternalEffect`）。
    pub kind: DeviceKind,
    /// 设备名（收据用）。
    pub name: String,
    /// 未执行的原因（固定词，见 [`unhosted_external_devices`]）。
    pub reason: &'static str,
}

/// 本轨全部**未被宿主**的非旁通外置设备（按设备表顺序）。
///
/// 旁通的设备不在内（旁通是显式选择）；已被宿主的那一台（首台胜出）也不在内。
/// 原因只有两种：`"未绑定插件"`（没有 `plugin` 绑定可实例化）与
/// `"第二台绑定设备，首台胜出"`（绑定了，但 first-source 语义只执行首台）。
#[must_use]
pub fn unhosted_external_devices(track: &TrackV3) -> Vec<UnhostedExternal> {
    let hosted_instrument = hosted_instrument_index(track);
    let hosted_effect = hosted_effect_index(track);
    track
        .devices
        .iter()
        .enumerate()
        .filter_map(|(index, device)| {
            if device.bypassed {
                return None;
            }
            let hosted = match device.kind {
                DeviceKind::ExternalInstrument => hosted_instrument,
                DeviceKind::ExternalEffect => hosted_effect,
                _ => return None,
            };
            if Some(index) == hosted {
                return None;
            }
            Some(UnhostedExternal {
                index,
                kind: device.kind,
                name: device.name.clone(),
                reason: if device.plugin.is_some() {
                    "第二台绑定设备，首台胜出"
                } else {
                    "未绑定插件"
                },
            })
        })
        .collect()
}

/// 已执行插件的收据行：`"轨名: 设备名 [乐器|效果器]"`。
#[must_use]
pub fn executed_label(track_name: &str, device_name: &str, kind: DeviceKind) -> String {
    format!("{track_name}: {device_name} [{}]", kind_label(kind))
}

/// 未执行插件的收据行：`"轨名: 设备名 [乐器|效果器] (未执行: 原因)"`。
#[must_use]
pub fn not_executed_label(
    track_name: &str,
    device_name: &str,
    kind: DeviceKind,
    reason: &str,
) -> String {
    format!(
        "{track_name}: {device_name} [{}] (未执行: {reason})",
        kind_label(kind)
    )
}

/// 设备种类的收据词：外置乐器 ⇒ `乐器`，外置效果器 ⇒ `效果器`。
const fn kind_label(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::ExternalInstrument => "乐器",
        DeviceKind::ExternalEffect => "效果器",
        _ => "设备",
    }
}

/// 恢复一条绑定里的状态快照（与 MCP `restore_binding_state` 同一条纪律）。
///
/// 空状态 = 默认，不做任何事；状态损坏或恢复失败 ⇒ `Err`（调用方响亮失败，
/// ⛔ 不按默认状态渲染）。
///
/// # Errors
///
/// 状态容器解码失败，或插件拒绝恢复该状态。
pub fn restore_binding_state(
    instance: &mut dyn PluginInstance,
    binding: &PluginBinding,
) -> Result<(), String> {
    if binding.state.is_empty() {
        return Ok(());
    }
    let state = PluginState::decode(&binding.state)
        .map_err(|error| format!("插件状态损坏，拒绝按默认状态渲染: {error}"))?;
    instance
        .restore_state(&state)
        .map_err(|error| format!("插件状态恢复失败，拒绝按默认状态渲染: {error}"))
}

/// 加载一条已装载乐器绑定的实例（路径白名单 → `Host::load` → 状态恢复）。
///
/// 调用方只传 [`hosted_instrument_index`] 给出的下标（⛔ 不对任意下标调用 ——
/// 未绑定设备没有可加载的东西）。
///
/// # Errors
///
/// 路径门失败、宿主加载失败、状态恢复失败（报文与 MCP 同一条前缀）。
pub fn load_hosted_instrument(
    track: &TrackV3,
    index: usize,
) -> Result<Box<dyn PluginInstance>, String> {
    let binding = track.devices[index]
        .plugin
        .as_ref()
        .expect("hosted_instrument_index 只指向带绑定的设备");
    Host::validate_plugin_path(&binding.path).map_err(|error| format!("插件装载失败: {error}"))?;
    let mut instance = Host::load(&binding.path, binding.class.as_deref())
        .map_err(|error| format!("插件装载失败: {error}"))?;
    restore_binding_state(instance.as_mut(), binding)?;
    Ok(instance)
}

/// 加载一条已装载效果器绑定的实例（路径白名单 → `Host::load_effect` → 状态恢复）。
///
/// 调用方只传 [`hosted_effect_index`] 给出的下标。
///
/// # Errors
///
/// 路径门失败、宿主加载失败、状态恢复失败（报文与 MCP 同一条前缀）。
pub fn load_hosted_effect(
    track: &TrackV3,
    index: usize,
) -> Result<Box<dyn PluginInstance>, String> {
    let binding = track.devices[index]
        .plugin
        .as_ref()
        .expect("hosted_effect_index 只指向带绑定的设备");
    Host::validate_plugin_path(&binding.path)
        .map_err(|error| format!("效果器装载失败: {error}"))?;
    let mut instance = Host::load_effect(&binding.path, binding.class.as_deref())
        .map_err(|error| format!("效果器装载失败: {error}"))?;
    restore_binding_state(instance.as_mut(), binding)?;
    Ok(instance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::{DeviceDefinition, EntityId, ParameterValue};
    use yeban_plugin_host::{PluginState, StubEffect};

    fn device(kind: DeviceKind, name: &str, plugin: Option<PluginBinding>) -> DeviceDefinition {
        DeviceDefinition {
            id: EntityId::new(),
            name: name.to_owned(),
            kind,
            bypassed: false,
            params: Vec::<ParameterValue>::new(),
            latency_samples: 0,
            plugin,
        }
    }

    fn binding(path: &str) -> PluginBinding {
        PluginBinding {
            path: path.to_owned(),
            class: Some(path.to_owned()),
            state: Vec::new(),
        }
    }

    fn track(devices: Vec<DeviceDefinition>) -> TrackV3 {
        TrackV3 {
            devices,
            ..TrackV3::default()
        }
    }

    #[test]
    fn first_bound_device_wins_and_bypassed_is_skipped() {
        let bypassed = DeviceDefinition {
            bypassed: true,
            ..device(
                DeviceKind::ExternalInstrument,
                "bypassed",
                Some(binding("stub:sine")),
            )
        };
        let first = device(
            DeviceKind::ExternalInstrument,
            "first",
            Some(binding("stub:sine")),
        );
        let second = device(
            DeviceKind::ExternalInstrument,
            "second",
            Some(binding("stub:sine")),
        );
        let unbound = device(DeviceKind::ExternalInstrument, "unbound", None);
        let track = track(vec![bypassed, first, second, unbound]);
        assert_eq!(hosted_instrument_index(&track), Some(1));
        let unhosted = unhosted_external_devices(&track);
        assert_eq!(unhosted.len(), 2, "旁通的不进收据，只剩第二台与未绑定");
        assert_eq!(unhosted[0].name, "second");
        assert_eq!(unhosted[0].reason, "第二台绑定设备，首台胜出");
        assert_eq!(unhosted[1].name, "unbound");
        assert_eq!(unhosted[1].reason, "未绑定插件");
    }

    #[test]
    fn effect_and_instrument_are_independent_first_wins() {
        let track = track(vec![
            device(
                DeviceKind::ExternalEffect,
                "fx1",
                Some(binding("stub:gain")),
            ),
            device(
                DeviceKind::ExternalInstrument,
                "inst",
                Some(binding("stub:sine")),
            ),
            device(
                DeviceKind::ExternalEffect,
                "fx2",
                Some(binding("stub:gain")),
            ),
        ]);
        assert_eq!(hosted_effect_index(&track), Some(0));
        assert_eq!(hosted_instrument_index(&track), Some(1));
        let unhosted = unhosted_external_devices(&track);
        assert_eq!(unhosted.len(), 1);
        assert_eq!(unhosted[0].name, "fx2");
    }

    #[test]
    fn no_external_devices_means_nothing_hosted_or_unhosted() {
        let track = track(vec![device(DeviceKind::InternalEffect, "eq", None)]);
        assert_eq!(hosted_instrument_index(&track), None);
        assert_eq!(hosted_effect_index(&track), None);
        assert!(unhosted_external_devices(&track).is_empty());
    }

    #[test]
    fn stub_gain_state_round_trip_restores_the_gain() {
        // 增益 0.5 经 save_state → 绑定 → restore 必须回来（面板抓状态、渲染恢复
        // 的同一条路；不是直接调 set_parameter）。
        let mut effect = StubEffect::new();
        effect.set_parameter(0, 0.5).expect("设增益");
        let state = effect.save_state().expect("存状态");
        let binding = PluginBinding {
            path: "stub:gain".to_owned(),
            class: Some("stub:gain".to_owned()),
            state: state.encode(),
        };
        let track = track(vec![device(
            DeviceKind::ExternalEffect,
            "gain",
            Some(binding),
        )]);
        let mut instance = load_hosted_effect(&track, 0).expect("桩效果器必须能加载并恢复状态");
        assert_eq!(
            instance.parameter_value(0).expect("读增益参数"),
            0.5,
            "恢复后的增益必须是 0.5（不是默认值 1.0）"
        );
        // 状态本身也对得上：save_state 编码的就是 0.5 的 LE 字节。
        assert_eq!(
            PluginState::decode(&track.devices[0].plugin.as_ref().expect("绑定").state)
                .expect("解码")
                .processor,
            0.5f32.to_le_bytes().to_vec()
        );
    }

    #[test]
    fn stub_instrument_loads_and_reports_its_name() {
        let track = track(vec![device(
            DeviceKind::ExternalInstrument,
            "sine",
            Some(binding("stub:sine")),
        )]);
        let instance = load_hosted_instrument(&track, 0).expect("桩乐器必须能加载");
        assert_eq!(instance.name(), "stub:sine");
        assert_eq!(
            executed_label("轨", "sine", DeviceKind::ExternalInstrument),
            "轨: sine [乐器]"
        );
        assert_eq!(
            not_executed_label("轨", "sine", DeviceKind::ExternalInstrument, "未绑定插件"),
            "轨: sine [乐器] (未执行: 未绑定插件)"
        );
    }

    #[test]
    fn corrupt_binding_state_fails_loudly() {
        let binding = PluginBinding {
            path: "stub:gain".to_owned(),
            class: Some("stub:gain".to_owned()),
            state: b"not a YBPGST01 container".to_vec(),
        };
        let track = track(vec![device(
            DeviceKind::ExternalEffect,
            "gain",
            Some(binding),
        )]);
        let error = match load_hosted_effect(&track, 0) {
            Ok(_) => panic!("坏状态必须响亮失败"),
            Err(error) => error,
        };
        assert!(error.contains("插件状态损坏"), "{error}");
    }
}
