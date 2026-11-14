//! 夜半原生预制（设计稿 §1/§2.6）的模型层判据：轨字段缺省不落盘、
//! `Op::SetTrackPreset` 只写一个字段且可逆、预制格式校验（64 KiB / major /
//! 标识）、工厂登记映射、容器内嵌往返。

use std::collections::BTreeMap;
use std::str::FromStr;
use yeban_model::container::{
    ContainerLimits, ProjectArchive, read_project_container,
    write_project_container_with_presets_borrowed,
};
use yeban_model::preset::{
    PRESET_MAX_BYTES, Preset, TrackPresetState, default_preset_id_for_instrument,
    embedded_entry_name, instrument_sfz_path, preset_id_from_embedded_entry, slug_for_sfz_path,
    strip_param_pairs,
};
use yeban_model::{EntityId, Op, SamplerBinding, TrackKind, TrackV3, YebanProjectV1};

fn minimal_preset_json() -> serde_json::Value {
    serde_json::json!({
        "format": 1,
        "id": "sys:salamander-grand:piano-warm-room",
        "name": "Piano Warm Room",
        "instrument": "salamander-grand",
        "instrument_sfz": "assets/samples/salamander-grand/Salamander Grand Piano V3.sfz",
        "scope": "instrument",
        "author": "Yeban factory",
        "version": "1.0.0",
        "draft": true,
        "strip": {"input_gain_db": 0.0, "eq_enabled": true, "eq": {"low_gain": 0.0, "mid_gain": 0.0, "high_gain": 0.0}},
        "sends": [{"aux": "Room", "gain_db": -14.0, "effect": "freeverb"}],
        "macros": [{"name": "混响量", "value": 0.5, "mappings": []}]
    })
}

fn track_fixture() -> (YebanProjectV1, EntityId) {
    let mut doc = YebanProjectV1::default();
    let track_id = EntityId::from_str("01J8ZQ00000000000000000001").expect("合法 ULID");
    Op::AddTrack {
        track: TrackV3 {
            id: track_id,
            name: "Lead".to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        },
    }
    .apply(&mut doc)
    .expect("加轨");
    (doc, track_id)
}

#[test]
fn preset_field_is_absent_by_default_and_round_trips_verbatim() {
    let default_track = TrackV3::default();
    assert!(default_track.preset.is_none(), "默认音轨不得带预制状态");
    let json = serde_json::to_string(&default_track).expect("序列化");
    assert!(
        !json.contains("\"preset\""),
        "默认音轨不得写出 `preset` 键（`None` ⇒ `skip_serializing_if`）: {json}"
    );
    let back: TrackV3 = serde_json::from_str(&json).expect("反序列化");
    assert_eq!(back.preset, None);
    // 旧工程逐字节不变：不带 preset 的 JSON 与加字段前同一形态。
    let mut with_preset = default_track.clone();
    with_preset.preset = Some(Box::new(TrackPresetState {
        preset_id: Some("sys:salamander-grand:piano-warm-room".to_owned()),
        bypassed: false,
        auto_load: true,
        previous: None,
    }));
    let json_with = serde_json::to_string(&with_preset).expect("序列化");
    assert!(
        json_with.contains("\"preset\":{\"preset_id\":\"sys:salamander-grand:piano-warm-room\"}"),
        "带预制的落盘形态必须是 `preset.preset_id`：{json_with}"
    );
    assert!(
        !json_with.contains("bypassed") && !json_with.contains("auto_load"),
        "缺省开关（bypassed=false / auto_load=true）不得落盘：{json_with}"
    );
}

#[test]
fn set_track_preset_writes_exactly_one_field_and_is_reversible() {
    let (mut doc, track_id) = track_fixture();
    let before = doc.track(&track_id).expect("轨在").clone();
    let new_state = TrackPresetState {
        preset_id: Some("sys:ixox-flute:flute-airy-hall".to_owned()),
        bypassed: false,
        auto_load: true,
        previous: None,
    };
    // 陈旧载荷必被拒。
    let stale = Op::SetTrackPreset {
        track_id,
        old_preset: Some(new_state.clone()),
        new_preset: None,
    };
    assert!(stale.precondition(&doc).is_err(), "旧值不符必须被拒");
    let op = Op::SetTrackPreset {
        track_id,
        old_preset: None,
        new_preset: Some(new_state.clone()),
    };
    assert_eq!(op.precondition(&doc), Ok(()));
    op.apply(&mut doc).expect("施加");
    let after = doc.track(&track_id).expect("轨在");
    assert_eq!(after.preset, Some(Box::new(new_state.clone())));
    assert_eq!(after.sampler, before.sampler);
    assert_eq!(after.devices, before.devices);
    assert_eq!(after.macros, before.macros);
    assert_eq!(after.volume_db, before.volume_db);
    assert_eq!(after.pan, before.pan);
    // 逆操作还原。
    op.invert(&doc)
        .expect("逆操作可求")
        .apply(&mut doc)
        .expect("逆操作");
    assert_eq!(doc.track(&track_id).expect("轨在").preset, None);
    // sampler 字段不受影响（预制状态与采样器绑定是两条平行字段）。
    let sampler_op = Op::SetTrackSampler {
        track_id,
        old_sampler: None,
        new_sampler: Some(SamplerBinding {
            path: "assets/samples/ixox-flute/Ixox Flute.sfz".to_owned(),
        }),
    };
    sampler_op.apply(&mut doc).expect("绑采样器");
    assert_eq!(doc.track(&track_id).expect("轨在").preset, None);
}

#[test]
fn preset_parse_rejects_oversize_newer_major_and_bad_id() {
    let good = serde_json::to_vec(&minimal_preset_json()).expect("序列化");
    let preset = Preset::parse(&good).expect("合法预制必须通过");
    assert_eq!(preset.id, "sys:salamander-grand:piano-warm-room");
    assert_eq!(preset.sends.len(), 1);
    assert_eq!(preset.sends[0].gain_db, Some(-14.0));

    // 超 64 KiB 拒绝保存。
    let mut big = good.clone();
    big.resize(PRESET_MAX_BYTES + 1, b' ');
    assert!(matches!(
        Preset::parse(&big),
        Err(yeban_model::preset::PresetError::TooLarge { .. })
    ));

    // major 更新响亮拒绝，不静默降级。
    let mut newer = minimal_preset_json();
    newer["format"] = serde_json::json!(2);
    assert!(matches!(
        Preset::parse(&serde_json::to_vec(&newer).expect("序列化")),
        Err(yeban_model::preset::PresetError::NewerMajor { found: 2, .. })
    ));

    // 标识非法拒绝。
    let mut bad_id = minimal_preset_json();
    bad_id["id"] = serde_json::json!("piano-warm-room");
    assert!(matches!(
        Preset::parse(&serde_json::to_vec(&bad_id).expect("序列化")),
        Err(yeban_model::preset::PresetError::InvalidId { .. })
    ));
    // 用户预制标识合法。
    let mut user = minimal_preset_json();
    user["id"] = serde_json::json!("user:01J8ZQ00000000000000000001");
    assert!(Preset::parse(&serde_json::to_vec(&user).expect("序列化")).is_ok());
}

#[test]
fn factory_registry_maps_slug_to_preset_and_sfz() {
    for (slug, preset_id) in [
        ("salamander-grand", "sys:salamander-grand:piano-warm-room"),
        (
            "karoryfer-bigcat-cello",
            "sys:karoryfer-bigcat-cello:cello-warm-hall",
        ),
        (
            "freepats-spanish-classical-guitar",
            "sys:freepats-spanish-classical-guitar:guitar-warm-room",
        ),
        ("ixox-flute", "sys:ixox-flute:flute-airy-hall"),
        ("karoryfer-bear-sax", "sys:karoryfer-bear-sax:sax-warm-room"),
        (
            "karoryfer-big-rusty-drums",
            "sys:karoryfer-big-rusty-drums:drums-punchy-room",
        ),
    ] {
        assert_eq!(default_preset_id_for_instrument(slug), Some(preset_id));
        let sfz = instrument_sfz_path(slug).expect("工厂乐器有 SFZ 路径");
        assert_eq!(slug_for_sfz_path(sfz), Some(slug), "SFZ 路径须能反查 slug");
        assert!(sfz.starts_with("assets/samples/"), "SFZ 路径是库根相对形式");
    }
    assert_eq!(default_preset_id_for_instrument("vsco2ce"), None);
    assert_eq!(
        embedded_entry_name("user:01J8ZQ00000000000000000001"),
        Some("presets/user/01J8ZQ00000000000000000001.json".to_owned())
    );
    assert_eq!(
        preset_id_from_embedded_entry("presets/user/01J8ZQ00000000000000000001.json"),
        Some("user:01J8ZQ00000000000000000001".to_owned())
    );
}

#[test]
fn strip_param_pairs_flatten_to_insert_recognised_names() {
    let strip = serde_json::json!({
        "input_gain_db": 0.0,
        "hpf": {"freq_hz": 80.0},
        "eq_enabled": true,
        "eq": {"low_gain": 0.0, "low_freq": 200.0, "mid_gain": 2.0, "mid_freq": 1000.0, "mid_q": 0.9, "high_gain": 0.0, "high_freq": 4000.0},
        "filter_enabled": true,
        "filter": {"cutoff_hz": 20000.0, "resonance": 0.0, "drive": 0.0},
        "compressor_enabled": true,
        "compressor": {"threshold_db": -18.0, "ratio": 3.0, "knee_db": 6.0, "detector_s": 0.005, "attack_s": 0.01, "release_s": 0.16, "makeup_db": 2.0},
        "width": 1.0,
        "output_gain_db": 0.0
    });
    let pairs = strip_param_pairs(&strip);
    let get = |name: &str| -> Option<f32> {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| *value)
    };
    assert_eq!(get("hpf_hz"), Some(80.0));
    assert_eq!(get("eq_mid_gain"), Some(2.0));
    assert_eq!(get("threshold_db"), Some(-18.0));
    assert_eq!(get("ratio"), Some(3.0));
    assert_eq!(get("strip_width"), Some(1.0));
    assert!(
        get("width").is_none(),
        "width 必须以 insert.rs 认的 strip_width 写出"
    );
}

#[test]
fn embedded_user_preset_round_trips_through_the_container() {
    let project = YebanProjectV1::default();
    let preset_bytes = serde_json::to_vec(&minimal_preset_json()).expect("序列化");
    let user_bytes = {
        let mut user = minimal_preset_json();
        user["id"] = serde_json::json!("user:01J8ZQ00000000000000000001");
        serde_json::to_vec(&user).expect("序列化")
    };
    let bytes = write_project_container_with_presets_borrowed(
        &project,
        b"history",
        &[],
        &[(
            "presets/user/01J8ZQ00000000000000000001.json",
            user_bytes.as_slice(),
        )],
    )
    .expect("写出必须成功");
    let ProjectArchive {
        embedded_presets, ..
    } = read_project_container(&bytes, &ContainerLimits::default()).expect("读回必须成功");
    assert_eq!(embedded_presets.len(), 1);
    assert_eq!(
        embedded_presets[0].0,
        "presets/user/01J8ZQ00000000000000000001.json"
    );
    assert_eq!(embedded_presets[0].1, user_bytes);
    assert!(Preset::parse(&embedded_presets[0].1).is_ok());
    // 系统预制条目名非法 ⇒ 写出即拒（只有 user 预制能进包）。
    assert!(
        write_project_container_with_presets_borrowed(
            &project,
            b"history",
            &[],
            &[("presets/sys/piano.json", preset_bytes.as_slice())],
        )
        .is_err()
    );
    // 超 64 KiB 的内嵌条目写出即拒。
    let big = vec![b'x'; PRESET_MAX_BYTES + 1];
    assert!(
        write_project_container_with_presets_borrowed(
            &project,
            b"history",
            &[],
            &[(
                "presets/user/01J8ZQ00000000000000000001.json",
                big.as_slice()
            )],
        )
        .is_err()
    );
}

/// ⭐ 预制 `cc` 段（用户初始 CC）：往返保真、空表不落盘、越域响亮拒绝。
///
/// 判据：① 不带 `cc` 的旧文件解析出空表，序列化后仍无 `cc` 键（不升
/// format、逐字兼容）；② 带 `cc` 的文件解析与再序列化往返一致；
/// ③ 控制器号或值落在 `128..=255`（装得进 `u8`、出 MIDI CC 域）⇒
/// `Preset::parse` 以 `InvalidField { field: "cc" }` 拒绝（不钳位）。
#[test]
fn preset_cc_segment_round_trips_and_validates_domain() {
    let good = serde_json::to_vec(&minimal_preset_json()).expect("序列化");
    let preset = Preset::parse(&good).expect("合法预制必须通过");
    assert!(preset.cc.is_empty(), "旧文件无 cc 段 ⇒ 空表");
    let text = serde_json::to_string(&preset).expect("序列化");
    assert!(
        !text.contains("\"cc\":"),
        "空 cc 不得进序列化形态（旧形态逐字不变）: {text}"
    );

    let mut with_cc = minimal_preset_json();
    with_cc["cc"] = serde_json::json!({"1": 64, "11": 127});
    let preset =
        Preset::parse(&serde_json::to_vec(&with_cc).expect("序列化")).expect("带 cc 的合法预制");
    assert_eq!(preset.cc, BTreeMap::from([(1u8, 64u8), (11, 127)]));
    let round = Preset::parse(&serde_json::to_vec(&preset).expect("序列化")).expect("往返必须通过");
    assert_eq!(round.cc, preset.cc, "cc 段往返一致");

    let mut bad_key = minimal_preset_json();
    bad_key["cc"] = serde_json::json!({"200": 1});
    assert!(
        matches!(
            Preset::parse(&serde_json::to_vec(&bad_key).expect("序列化")),
            Err(yeban_model::preset::PresetError::InvalidField { field: "cc", .. })
        ),
        "控制器号 200 越出 CC 域必须响亮拒绝"
    );
    let mut bad_value = minimal_preset_json();
    bad_value["cc"] = serde_json::json!({"1": 200});
    assert!(
        matches!(
            Preset::parse(&serde_json::to_vec(&bad_value).expect("序列化")),
            Err(yeban_model::preset::PresetError::InvalidField { field: "cc", .. })
        ),
        "值 200 越出 CC 域必须响亮拒绝"
    );
}
