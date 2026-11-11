//! 谱面映射（`yeban_model::song_overview`）的集成判据。
//!
//! 夹具工程：120 BPM、4/4、D 大调、变速点 140 BPM @ 1 拍；三条轨（旋律、
//! 打击、静音旋律）+ 一处循环摆放。断言映射输出与 dsp 契约逐位一致，
//! 且与引擎调度语义（触发 / 循环 / 连击 / 静音门）同一口径。

use std::collections::BTreeMap;
use std::str::FromStr as _;

use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, KeySignature, LoopConfig, MidiNote,
    RoutingEdge, RoutingGraph, RoutingKind, SectionV3, TimeSignature, TrackKind, TrackV3,
    YebanProjectV1,
    song_overview::{key_tonic_pc, score_input_from_project},
};

/// 夹具 ULID：`01J8ZM` + 20 位十进制序号（Crockford 数字位合法）。
fn id(index: u32) -> EntityId {
    EntityId::from_str(&format!("01J8ZM{index:020}")).expect("夹具 ULID 必须合法")
}

fn track(track_id: EntityId, name: &str) -> TrackV3 {
    TrackV3 {
        id: track_id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    }
}

fn note(note_id: EntityId, start_tick: u64, pitch: u8, duration_ticks: u64) -> MidiNote {
    MidiNote::new(note_id, start_tick, pitch, duration_ticks)
}

/// 夹具工程（见文件头）。
fn fixture_project() -> YebanProjectV1 {
    let master = id(1);
    let melody = id(2);
    let drums = id(3);
    let muted = id(4);
    let clip_melody = id(10);
    let clip_loop = id(11);
    let clip_drums = id(12);
    let clip_muted = id(13);

    // 旋律片段：A（0..960，C4）/ B（960..1440，E4，连击 2）/
    // C（概率 0，永不触发）/ D（通道 9，E4，打击判定走通道）。
    let mut melody_notes = BTreeMap::new();
    melody_notes.insert(id(20), note(id(20), 0, 60, 960));
    let mut ratcheted = note(id(21), 960, 64, 480);
    ratcheted.ratchet = Some(2);
    melody_notes.insert(id(21), ratcheted);
    let mut never = note(id(22), 1_920, 67, 480);
    never.probability = Some(0.0);
    melody_notes.insert(id(22), never);
    let mut channel_nine = note(id(23), 1_920, 65, 480);
    channel_nine.channel = Some(9);
    melody_notes.insert(id(23), channel_nine);

    // 循环片段：一个 480 tick 音符，摆放循环 0..960、总长 1920 ⇒ 播两遍。
    let mut loop_notes = BTreeMap::new();
    loop_notes.insert(id(24), note(id(24), 0, 72, 480));

    // 打击片段：一个底鼓音（C1），打击判定走采样器路径。
    let mut drum_notes = BTreeMap::new();
    drum_notes.insert(id(25), note(id(25), 0, 36, 240));

    let mut muted_notes = BTreeMap::new();
    muted_notes.insert(id(26), note(id(26), 0, 60, 960));

    let mut clip_pool = BTreeMap::new();
    for (clip_id, name, notes) in [
        (clip_melody, "melody", melody_notes),
        (clip_loop, "loop", loop_notes),
        (clip_drums, "drums", drum_notes),
        (clip_muted, "muted", muted_notes),
    ] {
        clip_pool.insert(
            clip_id,
            ClipPoolEntry {
                id: clip_id,
                name: name.to_owned(),
                content: ClipContent::Midi { notes },
            },
        );
    }

    let mut tracks = BTreeMap::new();
    let mut melody_track = track(melody, "旋律");
    melody_track.clips.insert(
        id(30),
        ClipPlacement {
            id: id(30),
            clip_id: clip_melody,
            start_tick: 0,
            duration_ticks: 3_840,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    melody_track.clips.insert(
        id(31),
        ClipPlacement {
            id: id(31),
            clip_id: clip_loop,
            start_tick: 3_840,
            duration_ticks: 1_920,
            loop_config: LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: 960,
            },
            muted: false,
            offset_samples: None,
        },
    );
    tracks.insert(melody, melody_track);

    let mut drum_track = track(drums, "轻鼓");
    drum_track.sampler = Some(yeban_model::SamplerBinding {
        path: "assets/samples/discord-gm-sitar/Discord GM/Drums/001-Standard Kit.sfz".to_owned(),
    });
    drum_track.clips.insert(
        id(32),
        ClipPlacement {
            id: id(32),
            clip_id: clip_drums,
            start_tick: 0,
            duration_ticks: 960,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    tracks.insert(drums, drum_track);

    let mut muted_track = track(muted, "静音旋律");
    muted_track.mute = true;
    muted_track.clips.insert(
        id(33),
        ClipPlacement {
            id: id(33),
            clip_id: clip_muted,
            start_tick: 0,
            duration_ticks: 960,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    tracks.insert(muted, muted_track);

    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut sections = BTreeMap::new();
    for (section_id, name, start_tick) in [
        (id(40), "Intro", 0u64),
        (id(41), "Verse", 7_680u64),
        (id(42), "Verse-dup", 7_680u64),
    ] {
        sections.insert(
            section_id,
            SectionV3 {
                id: section_id,
                name: name.to_owned(),
                start_tick,
                end_tick: start_tick + 7_680,
                color: None,
            },
        );
    }

    let mut edges = BTreeMap::new();
    for (edge_id, source) in [(id(50), melody), (id(51), drums), (id(52), muted)] {
        edges.insert(
            edge_id,
            RoutingEdge {
                id: edge_id,
                source_node: source,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }

    let mut tempo_map = BTreeMap::new();
    tempo_map.insert(960u64, 140.0f64);

    YebanProjectV1 {
        bpm: 120.0,
        tempo_map,
        time_signature: TimeSignature {
            numerator: 4,
            denominator: 4,
        },
        key_signature: Some(KeySignature {
            sharps_flats: 2,
            is_minor: false,
        }),
        sections,
        tracks,
        master_bus_track_id: master,
        routing_graph: RoutingGraph {
            nodes: vec![master, melody, drums, muted],
            edges,
        },
        clip_pool,
        ..YebanProjectV1::default()
    }
}

#[test]
fn mapping_matches_the_dsp_contract_note_for_note() {
    let project = fixture_project();
    let score = score_input_from_project(&project);

    // 拍号 / 调号 / 速度图。
    assert_eq!(score.beats_per_bar, 4);
    assert_eq!(score.key_tonic_pc, 2, "D 大调主音 = 2");
    assert!(!score.is_minor);
    assert_eq!(score.tempo_map.len(), 2, "0 拍基准 + 1 个变速点");
    assert_eq!(score.tempo_map[0].beat, 0.0);
    assert_eq!(score.tempo_map[0].bpm, 120.0);
    assert_eq!(score.tempo_map[1].beat, 1.0, "960 tick = 1 拍");
    assert_eq!(score.tempo_map[1].bpm, 140.0);

    // 段落：同一小节的第二个段落被去重。
    assert_eq!(score.markers.len(), 2);
    assert_eq!(score.markers[0].bar, 1);
    assert_eq!(score.markers[0].label, "Intro");
    assert_eq!(score.markers[1].bar, 3, "7680 tick = 8 拍 = 第 3 小节");
    assert_eq!(score.markers[1].label, "Verse");

    // 音符：旋律 6（A + B 连击 2 + D 通道 9 + 循环 2）+ 打击 1 = 7。
    // 概率 0 的音符与静音轨的音符一位都不进。
    assert_eq!(score.notes.len(), 7, "概率 0 与静音轨必须一位都不进");
    let by_pitch: BTreeMap<u8, Vec<_>> = {
        let mut map: BTreeMap<u8, Vec<_>> = BTreeMap::new();
        for note in &score.notes {
            map.entry(note.midi).or_default().push(note);
        }
        map
    };
    // A：0 拍起、1 拍长、C4、力度 100、旋律轨（显示序 0）。
    let a = by_pitch[&60][0];
    assert_eq!((a.start_beat, a.duration_beats), (0.0, 1.0));
    assert_eq!(a.velocity, 100);
    assert_eq!(a.track, 0);
    assert!(!a.is_percussion);
    // B：连击 2 ⇒ 两个 0.25 拍脉冲（1.0 拍与 1.25 拍起）。
    let b = &by_pitch[&64];
    assert_eq!(b.len(), 2);
    assert_eq!((b[0].start_beat, b[0].duration_beats), (1.0, 0.25));
    assert_eq!((b[1].start_beat, b[1].duration_beats), (1.25, 0.25));
    // D：通道 9 ⇒ 打击轨。
    assert!(by_pitch[&65][0].is_percussion, "MIDI 通道 10 的音符是打击");
    // 循环：480 tick 音符在 0..960 区间播两遍（4 拍与 5 拍起，各 0.5 拍）。
    let looped = &by_pitch[&72];
    assert_eq!(looped.len(), 2);
    assert_eq!((looped[0].start_beat, looped[0].duration_beats), (4.0, 0.5));
    assert_eq!((looped[1].start_beat, looped[1].duration_beats), (5.0, 0.5));
    // 底鼓：采样器路径含 Drums ⇒ 打击轨（显示序 1）。
    let kick = by_pitch[&36][0];
    assert!(kick.is_percussion, "采样器路径含 Drums 的轨是打击");
    assert_eq!(kick.track, 1);
    assert_eq!((kick.start_beat, kick.duration_beats), (0.0, 0.25));

    // 确定性：两次映射逐位相同。
    assert_eq!(score_input_from_project(&project), score);
}

#[test]
fn key_signature_maps_through_the_circle_of_fifths() {
    // 无调号 ⇒ C 大调。
    assert_eq!(key_tonic_pc(None), (0, false));
    // 升号：G 大调（1）/ D 大调（2）/ F# 大调（6）。
    for (sharps, tonic) in [(1i8, 7u8), (2, 2), (6, 6)] {
        assert_eq!(
            key_tonic_pc(Some(KeySignature {
                sharps_flats: sharps,
                is_minor: false,
            })),
            (tonic, false),
            "升号 {sharps}"
        );
    }
    // 降号：F 大调（−1 → 5）/ Bb 大调（−2 → 10）。
    for (flats, tonic) in [(-1i8, 5u8), (-2, 10)] {
        assert_eq!(
            key_tonic_pc(Some(KeySignature {
                sharps_flats: flats,
                is_minor: false,
            })),
            (tonic, false),
            "降号 {flats}"
        );
    }
    // 小调取关系小调：A 小调（0 → 9）/ D 小调（−1 → 2）/ B 小调（2 → 11）。
    for (sharps, tonic) in [(0i8, 9u8), (-1, 2), (2, 11)] {
        assert_eq!(
            key_tonic_pc(Some(KeySignature {
                sharps_flats: sharps,
                is_minor: true,
            })),
            (tonic, true),
            "小调 {sharps}"
        );
    }
}

#[test]
fn beats_per_bar_counts_quarter_note_beats() {
    let mut project = fixture_project();
    project.time_signature = TimeSignature {
        numerator: 6,
        denominator: 8,
    };
    assert_eq!(
        score_input_from_project(&project).beats_per_bar,
        3,
        "6/8 一小节是 3 个四分音符拍"
    );
    project.time_signature = TimeSignature {
        numerator: 3,
        denominator: 4,
    };
    assert_eq!(score_input_from_project(&project).beats_per_bar, 3);
}

#[test]
fn solo_gates_the_score_the_way_the_mix_does() {
    let mut project = fixture_project();
    // 打击轨独奏 ⇒ 只有打击轨的音符进谱面。
    let drums = project
        .tracks
        .values_mut()
        .find(|track| track.name == "轻鼓")
        .expect("夹具必须有打击轨");
    drums.solo = true;
    let score = score_input_from_project(&project);
    assert_eq!(score.notes.len(), 1, "独奏时只有独奏轨发声");
    assert_eq!(score.notes[0].midi, 36);
}
