//! T-FIX-124：lane 化轨走「延后汇总」时右声道必须存活。
//!
//! 构造一条 **lane_active = true** 的鼓轨（一个 `lane_chains` 组、首键 36、
//! 组声相硬右），并给它一条**入边**（空轨 → 鼓轨）使 `defer_own_to_forward`
//! 为真、又不是处理型总线。此时 `stereo_track = false`、`pair_track = true`；
//! 延后汇总段若按 `stereo_track` 选源，右声道会被 `track_scratch` 覆盖成静音。
//! 母线右声道必须有能量、左声道应接近静音。

mod support;

use support::{NoteSpec, note_project, render};
use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, LaneChainDef, ParameterValue, RoutingEdge, RoutingKind,
    TrackKind, TrackV3,
};

fn drum_kit() -> DeviceDefinition {
    let params = [
        ("kick_note", 36.0),
        ("snare_note", 38.0),
        ("closed_hat_note", 42.0),
        ("open_hat_note", 46.0),
        ("clap_note", 39.0),
    ];
    DeviceDefinition {
        id: EntityId::new(),
        name: "Yeban Drums".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| ParameterValue {
                name: (*name).to_owned(),
                value: *value,
                unit: None,
            })
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

fn edge(source: EntityId, destination: EntityId) -> RoutingEdge {
    RoutingEdge {
        id: EntityId::new(),
        source_node: source,
        destination_node: destination,
        kind: RoutingKind::TrackToBus,
        gain_db: None,
        source_lane: None,
        dest_lane: None,
    }
}

#[test]
fn lane_deferred_forward_keeps_the_right_channel() {
    let fixture = note_project(&[NoteSpec::at(0, 480, 36, 100)]);
    let drum = fixture.track;
    let mut project = fixture.project;

    // 鼓机 + 一个 lane 组（只覆盖底鼓），组声相硬右。
    let entry = project.tracks.get_mut(&drum).expect("鼓轨");
    entry.devices = vec![drum_kit()];
    entry.lane_chains = vec![LaneChainDef {
        keys: vec![36],
        name: "kick".to_owned(),
        devices: Vec::new(),
        pan: Some(1.0),
        gain_db: None,
    }];

    // 空源轨 → 鼓轨：让鼓轨有入边，走 `defer_own_to_forward`；空轨不出声。
    let source = EntityId::new();
    let source_track = TrackV3 {
        id: source,
        name: "Empty source".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    project.tracks.insert(source, source_track);
    project.routing_graph.nodes.push(source);
    let id = EntityId::new();
    project.routing_graph.edges.insert(
        id,
        RoutingEdge {
            id,
            ..edge(source, drum)
        },
    );

    let rendered = render(&project, 24);
    let peak_l = rendered
        .left
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    let peak_r = rendered
        .right
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));

    assert!(
        peak_r > 1e-3,
        "lane 化鼓轨的右声道必须出声: peak_l={peak_l}, peak_r={peak_r}, stats={:?}",
        rendered.stats
    );
    assert!(
        peak_l < 1e-5,
        "组声相硬右时左声道必须接近静音: peak_l={peak_l}, peak_r={peak_r}, stats={:?}",
        rendered.stats
    );
    assert_eq!(
        rendered.stats.lane_bus_fallbacks, 0,
        "本夹具不得回退到总线路径"
    );
    assert_eq!(
        rendered.stats.lane_group_capacity_drops, 0,
        "本夹具的 lane 组必须武装"
    );
}
