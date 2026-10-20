//! ⭐ `#263` 判据：`DeviceParam` 的**序数地址**解析成**稳定名字**（唯一实现
//! `yeban_render::inserts::resolve_device_param`）。
//!
//! 背景（逐字）：`AutomationTarget::DeviceParam { slot_index, param_index }` 的两个字段
//! 都是 `Vec` 下标，而模型自己的寻址口径是 `track_id` + `device_id`（
//! `crates/yeban-model/src/ops.rs` 的 `SetDeviceBypassed`："⛔ 不用下标（`Vec`，增删会
//! 移位）"）。于是**中途插一台设备**会让泳道静默改指，而
//! `AutomationTarget::validate_against` 仍 `Ok(())`（新下标仍在范围）。
//!
//! 本文件钉住四件事：
//!
//! 1. 能解析：设备 `id` ＋ 参数名**原文** ＋ 白名单判定；
//! 2. 越界：两种下标各有**逐字**原因串（与 `validate_against` 的范围规则同式）；
//! 3. 名字不在白名单：**名字仍然报**，只是 `paramNotSupported`；
//! 4. 插一台设备：同一个序数解析到的 `device_id` **变了**，且 `validate_against`
//!    **不会**变红 —— 这就是"静默改指"的机械演示。

use yeban_model::{
    AutomationTarget, DeviceDefinition, DeviceKind, EntityId, ParameterValue, TrackV3,
    YebanProjectV1,
};
use yeban_render::inserts::{UnresolvedReason, resolve_device_param};

/// 固定 ULID 序号（判据不依赖随机身份，读数因此可逐字复核）。
fn node(seed: u32) -> EntityId {
    let mut text = [b'0'; 26];
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    for position in 0..4 {
        text[25 - position] = ALPHABET[((seed >> (5 * position)) & 0x1F) as usize];
    }
    core::str::from_utf8(&text)
        .expect("ASCII")
        .parse()
        .expect("合法 ULID")
}

fn param(name: &str, value: f32) -> ParameterValue {
    ParameterValue {
        name: name.to_owned(),
        value,
        unit: None,
    }
}

fn device(id: EntityId, params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id,
        name: "dev".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| param(name, *value))
            .collect(),
        latency_samples: 0,
    }
}

/// 一个只含一条轨的工程（解析只看 `tracks`，因此其它集合保持默认）。
fn project_with(devices: Vec<DeviceDefinition>) -> (YebanProjectV1, EntityId) {
    let track_id = node(1);
    let track = TrackV3 {
        id: track_id,
        name: "Lead".to_owned(),
        devices,
        ..TrackV3::default()
    };
    let mut project = YebanProjectV1 {
        master_bus_track_id: node(0xFF),
        ..YebanProjectV1::default()
    };
    project.tracks.insert(track_id, track);
    (project, track_id)
}

/// 判据 1：`DeviceParam { 0, 0 }` 且设备有参数 ⇒ 设备 `id` ＋ 参数名原文 ＋ 无原因。
#[test]
fn a_resolvable_address_reports_the_device_identity_and_the_param_name() {
    let device_id = node(101);
    let (project, track_id) = project_with(vec![device(
        device_id,
        &[("input_gain_db", -3.0), ("mix", 0.5)],
    )]);
    let resolved = resolve_device_param(&project, track_id, 0, 0).expect("必须能解析");
    println!(
        "判据 1 读数: device_id={} param={:?} supported={} unresolved={:?}",
        resolved.device_id.to_canonical_string(),
        resolved.param_name,
        resolved.is_supported,
        resolved.unresolved_reason()
    );
    assert_eq!(resolved.device_id, device_id);
    assert_eq!(resolved.param_name, "input_gain_db");
    assert!(resolved.is_supported);
    assert_eq!(resolved.unresolved_reason(), None);
}

/// 判据 2：两种越界各报**逐字**原因串；不存在的音轨也**不沉默**。
#[test]
fn out_of_range_addresses_report_the_model_reasons_verbatim() {
    let (project, track_id) = project_with(vec![device(node(101), &[("mix", 0.5)])]);

    let slot = resolve_device_param(&project, track_id, 99, 0).expect_err("槽位越界");
    assert_eq!(slot, UnresolvedReason::SlotOutOfRange);
    assert_eq!(slot.as_str(), "slotOutOfRange");

    let index = resolve_device_param(&project, track_id, 0, 99).expect_err("参数越界");
    assert_eq!(index, UnresolvedReason::ParamIndexOutOfRange);
    assert_eq!(index.as_str(), "paramIndexOutOfRange");

    let missing = resolve_device_param(&project, node(7), 0, 0).expect_err("音轨不存在");
    assert_eq!(missing, UnresolvedReason::TrackNotFound);
    assert_eq!(missing.as_str(), "trackNotFound");

    println!(
        "判据 2 读数: slot={:?} param={:?} track={:?}",
        slot.as_str(),
        index.as_str(),
        missing.as_str()
    );
}

/// 判据 3：名字不在白名单 ⇒ `paramNotSupported`，但**名字仍然报**（解析到了、
/// 只是 DSP 不认）。
#[test]
fn a_name_outside_the_whitelist_is_still_reported() {
    let device_id = node(101);
    let (project, track_id) = project_with(vec![device(device_id, &[("mix", 0.5)])]);
    let resolved = resolve_device_param(&project, track_id, 0, 0).expect("名字必须解析到");
    println!(
        "判据 3 读数: device_id={} param={:?} supported={} unresolved={:?}",
        resolved.device_id.to_canonical_string(),
        resolved.param_name,
        resolved.is_supported,
        resolved.unresolved_reason()
    );
    assert_eq!(resolved.device_id, device_id);
    assert_eq!(resolved.param_name, "mix");
    assert!(!resolved.is_supported);
    assert_eq!(resolved.unresolved_reason(), Some("paramNotSupported"));
}

/// 白名单判定与 `strip_of` 同口径（大小写不敏感）：否则"解析说支持"与"投影不认"
/// 会分叉。
#[test]
fn whitelist_matching_is_case_insensitive_like_the_insert_projection() {
    let (project, track_id) = project_with(vec![device(node(101), &[("Input_Gain_DB", -3.0)])]);
    let resolved = resolve_device_param(&project, track_id, 0, 0).expect("必须能解析");
    assert_eq!(resolved.param_name, "Input_Gain_DB", "报的是原文");
    assert!(resolved.is_supported, "白名单匹配必须大小写不敏感");
    assert_eq!(resolved.unresolved_reason(), None);
}

/// 判据 4（解析层）：**插一台设备** ⇒ 同一个序数解析到**另一台设备**，而
/// `AutomationTarget::validate_against` 仍然 `Ok(())` —— 模型不会替调用方发现改指。
#[test]
fn inserting_a_device_at_slot_zero_moves_the_resolved_identity() {
    let old_device = node(101);
    let new_device = node(202);
    let (mut project, track_id) =
        project_with(vec![device(old_device, &[("input_gain_db", -3.0)])]);
    let target = AutomationTarget::DeviceParam {
        track_id,
        slot_index: 0,
        param_index: 0,
    };
    target.validate_against(&project).expect("插设备前目标合法");
    let before = resolve_device_param(&project, track_id, 0, 0).expect("插设备前能解析");

    // 机械演示：模型 `Op::InsertDevice` 在槽位 0 插入一台新设备。
    let op = yeban_model::Op::InsertDevice {
        track_id,
        slot_index: 0,
        device: device(new_device, &[("input_gain_db", 0.0)]),
    };
    op.apply(&mut project).expect("插入设备");

    // ⚠ 这就是"静默"：新下标仍在范围，对账入口**不会**报任何东西。
    target
        .validate_against(&project)
        .expect("插设备后 validate_against 仍然 Ok（这正是隐患）");
    let after = resolve_device_param(&project, track_id, 0, 0).expect("插设备后也能解析");

    println!(
        "判据 4 读数: before={} after={}（validate_against 两次都是 Ok）",
        before.device_id.to_canonical_string(),
        after.device_id.to_canonical_string()
    );
    assert_eq!(before.device_id, old_device);
    assert_eq!(after.device_id, new_device);
    assert_ne!(
        before.device_id, after.device_id,
        "插设备必须让同一个序数改指 —— 否则本判据测的不是这件事"
    );
    assert_eq!(
        before.param_name, after.param_name,
        "名字仍是槽位 0 的参数名"
    );
}
