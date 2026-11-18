//! 逐件（per-lane）处理链的模型层判据（逐件处理线期 2）：
//! `TrackV3::lane_chains` 缺省不落盘、旧工程 JSON 逐字节可读且重序列化
//! 不变、带逐件链的工程往返一致、规范化校验逐条响亮拒绝。

use std::str::FromStr;
use yeban_model::{
    DeviceDefinition, EntityId, LaneChainDef, ModelError, TrackKind, TrackV3, YebanProjectV1,
};

fn hats_chain() -> LaneChainDef {
    LaneChainDef {
        keys: vec![42, 44, 46],
        name: "hats".to_owned(),
        devices: Vec::new(),
        pan: None,
    }
}

fn track_with(chains: Vec<LaneChainDef>) -> TrackV3 {
    TrackV3 {
        lane_chains: chains,
        ..TrackV3::default()
    }
}

#[test]
fn lane_chains_absent_by_default_and_old_json_loads_unchanged() {
    // 缺省（空表）不落盘：无逐件链的音轨序列化形态与引入本字段之前一致。
    let default_track = TrackV3::default();
    assert!(default_track.lane_chains.is_empty());
    let json = serde_json::to_string(&default_track).expect("序列化");
    assert!(
        !json.contains("\"lane_chains\""),
        "空逐件链不得写出 `lane_chains` 键: {json}"
    );
    // 这份 JSON 就是旧工程的形态：读回必须逐值相等、校验通过。
    let back: TrackV3 = serde_json::from_str(&json).expect("旧形态 JSON 必须可读");
    assert_eq!(back, default_track);
    back.validate().expect("旧形态音轨必须校验通过");

    // 工程级同样成立：整份旧工程 JSON 无 `lane_chains` 键，往返逐值相等。
    let mut doc = YebanProjectV1::default();
    let track_id = EntityId::from_str("01J8ZQ00000000000000000001").expect("合法 ULID");
    yeban_model::Op::AddTrack {
        track: TrackV3 {
            id: track_id,
            name: "Drums".to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        },
    }
    .apply(&mut doc)
    .expect("加轨");
    let project_json = serde_json::to_string(&doc).expect("工程序列化");
    assert!(
        !project_json.contains("\"lane_chains\""),
        "无逐件链的工程不得写出 `lane_chains` 键: {project_json}"
    );
    let project_back: YebanProjectV1 =
        serde_json::from_str(&project_json).expect("旧工程必须可读");
    assert_eq!(project_back, doc);
    project_back.validate().expect("旧工程必须校验通过");
}

#[test]
fn lane_chains_round_trip_verbatim_when_present() {
    let track = track_with(vec![
        LaneChainDef {
            keys: vec![36],
            name: "kick".to_owned(),
            devices: vec![DeviceDefinition {
                name: "Channel Strip".to_owned(),
                kind: yeban_model::DeviceKind::InternalEffect,
                params: vec![yeban_model::ParameterValue {
                    name: "eq_low_gain".to_owned(),
                    value: 1.5,
                    unit: None,
                }],
                ..DeviceDefinition::default()
            }],
            pan: Some(-0.25),
        },
        hats_chain(),
    ]);
    track.validate().expect("规范化逐件链必须校验通过");
    let json = serde_json::to_string(&track).expect("序列化");
    assert!(json.contains("\"lane_chains\""), "非空逐件链必须落盘");
    let back: TrackV3 = serde_json::from_str(&json).expect("反序列化");
    assert_eq!(back, track);
    assert_eq!(
        serde_json::to_string(&back).expect("再序列化"),
        json,
        "往返必须逐字节一致"
    );
}

#[test]
fn track_validate_rejects_unnormalized_lane_chains() {
    // 组内键为空。
    let empty_keys = track_with(vec![LaneChainDef {
        keys: Vec::new(),
        ..hats_chain()
    }]);
    assert_eq!(
        empty_keys.validate(),
        Err(ModelError::LaneChainKeysEmpty {
            name: "hats".to_owned()
        })
    );
    // 组内未升序。
    let unsorted = track_with(vec![LaneChainDef {
        keys: vec![46, 42],
        ..hats_chain()
    }]);
    assert_eq!(
        unsorted.validate(),
        Err(ModelError::LaneChainKeysNotAscending {
            name: "hats".to_owned()
        })
    );
    // 组内重复键（非严格升序的一种）。
    let duplicated = track_with(vec![LaneChainDef {
        keys: vec![42, 42],
        ..hats_chain()
    }]);
    assert_eq!(
        duplicated.validate(),
        Err(ModelError::LaneChainKeysNotAscending {
            name: "hats".to_owned()
        })
    );
    // 两组抢同一个键（组序本身合法：首键 36 < 42）。
    let overlap = track_with(vec![
        LaneChainDef {
            keys: vec![36, 42],
            name: "low".to_owned(),
            ..hats_chain()
        },
        LaneChainDef {
            keys: vec![42, 46],
            name: "open".to_owned(),
            ..hats_chain()
        },
    ]);
    assert_eq!(
        overlap.validate(),
        Err(ModelError::LaneChainKeyOverlap { key: 42 })
    );
    // 组序未按首键升序。
    let unordered = track_with(vec![
        LaneChainDef {
            keys: vec![46],
            name: "open".to_owned(),
            ..hats_chain()
        },
        LaneChainDef {
            keys: vec![42],
            name: "closed".to_owned(),
            ..hats_chain()
        },
    ]);
    assert_eq!(
        unordered.validate(),
        Err(ModelError::LaneChainsNotOrdered)
    );
    // 组数超上限（17 组）。
    let too_many = track_with(
        (0..17u8)
            .map(|key| LaneChainDef {
                keys: vec![key],
                name: format!("g{key}"),
                devices: Vec::new(),
                pan: None,
            })
            .collect(),
    );
    assert_eq!(
        too_many.validate(),
        Err(ModelError::LaneChainCountOutOfRange {
            count: 17,
            max: 16
        })
    );
    // 链内设备带非零延迟（逐件分支不做 PDC）。
    let latency_device = DeviceDefinition {
        latency_samples: 64,
        ..DeviceDefinition::default()
    };
    let with_latency = track_with(vec![LaneChainDef {
        devices: vec![latency_device.clone()],
        ..hats_chain()
    }]);
    assert_eq!(
        with_latency.validate(),
        Err(ModelError::LaneChainDeviceLatency {
            name: "hats".to_owned(),
            device: latency_device.id,
            latency: 64
        })
    );
    // 声相越界。
    let bad_pan = track_with(vec![LaneChainDef {
        pan: Some(1.5),
        ..hats_chain()
    }]);
    assert_eq!(
        bad_pan.validate(),
        Err(ModelError::LaneChainPanOutOfRange {
            name: "hats".to_owned(),
            value: 1.5
        })
    );
    // 声相非有限。
    let nan_pan = track_with(vec![LaneChainDef {
        pan: Some(f32::NAN),
        ..hats_chain()
    }]);
    assert!(matches!(
        nan_pan.validate(),
        Err(ModelError::NonFiniteValue { .. })
    ));
    // 链内设备参数非有限 ⇒ 设备自身校验冒泡。
    let nan_param_device = DeviceDefinition {
        params: vec![yeban_model::ParameterValue {
            name: "eq_low_gain".to_owned(),
            value: f32::NAN,
            unit: None,
        }],
        ..DeviceDefinition::default()
    };
    let with_nan_param = track_with(vec![LaneChainDef {
        devices: vec![nan_param_device],
        ..hats_chain()
    }]);
    assert!(matches!(
        with_nan_param.validate(),
        Err(ModelError::NonFiniteValue { .. })
    ));
}
