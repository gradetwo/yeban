//! ⭐ T-FIX-345：插件状态字节落在 `PluginBinding.state` 的序列化往返与旧工程兼容。
//!
//! 选址：状态属于"这台插件实例"，与 `path`/`class` 同一组身份；放在
//! `DeviceDefinition` 上会允许"有状态但没有插件"的非法组合。
//! 空 `Vec` = 没保存过状态（旧工程 / 新装插件）⇒ 保留插件默认值。

use yeban_model::EntityId;
use yeban_model::project::{DeviceDefinition, DeviceKind, PluginBinding, YebanProjectV1};
use yeban_model::samples::filled_project;

fn project_with_plugin_state(state: Vec<u8>) -> (YebanProjectV1, EntityId) {
    let mut project = filled_project();
    let track_id = project
        .tracks
        .keys()
        .copied()
        .find(|id| *id != project.master_bus_track_id)
        .expect("filled_project 必须有非主总线轨");
    let device_id = EntityId::new();
    project
        .tracks
        .get_mut(&track_id)
        .expect("轨")
        .devices
        .push(DeviceDefinition {
            id: device_id,
            name: "stub-gain".to_owned(),
            kind: DeviceKind::ExternalEffect,
            bypassed: false,
            params: Vec::new(),
            latency_samples: 0,
            plugin: Some(PluginBinding {
                path: "stub:gain".to_owned(),
                class: None,
                state,
            }),
        });
    (project, device_id)
}

fn state_of(project: &YebanProjectV1, device_id: EntityId) -> Vec<u8> {
    for track in project.tracks.values() {
        for device in &track.devices {
            if device.id == device_id {
                return device
                    .plugin
                    .as_ref()
                    .expect("设备必须有插件绑定")
                    .state
                    .clone();
            }
        }
    }
    panic!("设备 {device_id} 不在工程里");
}

#[test]
fn plugin_state_survives_a_project_round_trip() {
    // 一段可核对的处理品字节 + 一段控制品字节（与 YBPGST01 的载荷同形；模型只当不透明字节）。
    let processor = vec![0x11_u8, 0x22, 0x33, 0x44];
    let controller = vec![0xAA_u8, 0xBB, 0xCC];
    let mut state = Vec::new();
    state.extend_from_slice(b"YBPGST01");
    state.extend_from_slice(&1_u32.to_le_bytes());
    state.extend_from_slice(&(processor.len() as u32).to_le_bytes());
    state.extend_from_slice(&processor);
    state.extend_from_slice(&(controller.len() as u32).to_le_bytes());
    state.extend_from_slice(&controller);

    let (project, device_id) = project_with_plugin_state(state.clone());
    let json = serde_json::to_string(&project).expect("序列化工程");
    assert!(
        json.contains("\"state\""),
        "非空状态必须真的写进工程 JSON（⛔ 只存在内存里）"
    );
    let reopened: YebanProjectV1 = serde_json::from_str(&json).expect("重开工程");
    assert_eq!(
        state_of(&reopened, device_id),
        state,
        "处理器 + 控制品字节必须逐位往返"
    );
}

#[test]
fn an_old_project_without_state_loads_with_an_empty_state() {
    let (project, device_id) = project_with_plugin_state(vec![1, 2, 3, 4]);
    let json = serde_json::to_value(&project).expect("转 JSON 值");
    let mut json = json;
    // 模拟旧工程：把插件状态键整个删掉。
    let mut removed = false;
    if let Some(tracks) = json.get_mut("tracks").and_then(|v| v.as_object_mut()) {
        for track in tracks.values_mut() {
            if let Some(devices) = track.get_mut("devices").and_then(|v| v.as_array_mut()) {
                for device in devices {
                    if let Some(plugin) = device.get_mut("plugin").and_then(|v| v.as_object_mut()) {
                        removed |= plugin.remove("state").is_some();
                    }
                }
            }
        }
    }
    assert!(removed, "夹具必须真的删掉 state 键");
    let reopened: YebanProjectV1 = serde_json::from_value(json).expect("旧工程必须能打开");
    assert_eq!(
        state_of(&reopened, device_id),
        Vec::<u8>::new(),
        "缺字段 ⇒ 空状态（插件保持默认），⛔ 不拒载"
    );

    // 新建插件（空状态）也不写 state 键 ⇒ 旧版本读新工程同样不受影响。
    let (fresh, _) = project_with_plugin_state(Vec::new());
    let json = serde_json::to_string(&fresh).expect("序列化");
    assert!(
        !json.contains("\"state\""),
        "空状态应 skip_serializing ⇒ 新工程的字节形态不因本字段改变"
    );
}
