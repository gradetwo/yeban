//! Per-key solo-render audit for both drum kit families (line/drum-fx-layer
//! evidence tool, not a product path).
//!
//! Usage:
//! `cargo run -p yeban-app --example drum_key_audit -- <repo_root> <out_dir>`
//!
//! - Acoustic keys {36,37,38,42,43,45,46,48,49,82} through the Virtuosity
//!   SFZ kit (samples must be present under
//!   `<repo_root>/assets/samples/virtuosity-drums/Samples`), one hit at
//!   velocity 100 per key, no FX — the raw kit as the sampler plays it.
//! - Electronic keys {36,38,39,42,46} through the synth drum device, with
//!   the kit_setup parameters of `trap` (long 48 Hz kick) and of `house`
//!   (default 55 Hz kick); the other four voices are identical across
//!   styles and are rendered once (trap pass).
//!
//! Each render goes through the production offline path
//! (`yeban_render::render_project_to_buffer`) on a minimal project
//! (master + one drum track at 0 dB), and is written as a 16-bit WAV.
//! The tool prints one line per render with peak and RMS so a silent or
//! missing key is visible in the log, and fails loudly if the renderer
//! reports the drum track as skipped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use yeban_model::ops::Op;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, ParameterValue, RoutingEdge, RoutingKind, SamplerBinding, TrackKind, TrackV3,
    YebanProjectV1,
};
use yeban_render::engine_source::ArmedSfzInput;
use yeban_render::song_overview::{RenderSettings, render_project_to_buffer};

const ACOUSTIC_KEYS: [u8; 10] = [36, 37, 38, 42, 43, 45, 46, 48, 49, 82];
const SYNTH_KEYS: [u8; 5] = [36, 38, 39, 42, 46];
const VELOCITY: u8 = 100;

fn build_project(kit: &KitChoice, key: u8) -> (YebanProjectV1, EntityId) {
    let mut project = YebanProjectV1::default();
    let master_id = EntityId::new();
    project.master_bus_track_id = master_id;
    project.tracks.insert(
        master_id,
        TrackV3 {
            id: master_id,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.track_order.push(master_id);

    let track_id = EntityId::new();
    let mut ops = vec![
        Op::AddTrack {
            track: TrackV3 {
                id: track_id,
                name: format!("Audit Key {key}"),
                kind: TrackKind::Midi,
                volume_db: 0.0,
                ..TrackV3::default()
            },
        },
        Op::AddRoutingNode { node: track_id },
        Op::AddRoutingNode { node: master_id },
        Op::ConnectRouting {
            edge: RoutingEdge {
                id: EntityId::new(),
                source_node: track_id,
                destination_node: master_id,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
                            source_lane: None,
                dest_lane: None,
},
        },
    ];
    match kit {
        KitChoice::Acoustic => ops.push(Op::SetTrackSampler {
            track_id,
            old_sampler: None,
            new_sampler: Some(SamplerBinding {
                path: yeban_theory::ACOUSTIC_KIT_SFZ_PATH.to_owned(),
                note_octave_override: None,
            }),
        }),
        KitChoice::Synth {
            device_name,
            params,
        } => ops.push(Op::InsertDevice {
            track_id,
            slot_index: 0,
            device: DeviceDefinition {
                id: EntityId::new(),
                name: (*device_name).to_owned(),
                kind: DeviceKind::InternalInstrument,
                bypassed: false,
                params: params
                    .iter()
                    .map(|(name, value)| ParameterValue {
                        name: name.clone(),
                        value: *value,
                        unit: None,
                    })
                    .collect(),
                latency_samples: 0,
                plugin: None,
            },
        }),
    }
    let mut notes = BTreeMap::new();
    let note_id = EntityId::new();
    notes.insert(
        note_id,
        MidiNote {
            velocity: VELOCITY,
            ..MidiNote::new(note_id, 0, key, 240)
        },
    );
    let clip_id = EntityId::new();
    ops.push(Op::AddClip {
        clip: ClipPoolEntry {
            id: clip_id,
            name: format!("Key {key}"),
            content: ClipContent::Midi { notes },
        },
    });
    ops.push(Op::AddClipPlacement {
        track_id,
        placement: ClipPlacement {
            id: EntityId::new(),
            clip_id,
            start_tick: 0,
            duration_ticks: yeban_theory::TICKS_PER_BAR,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    });
    for op in &ops {
        op.apply(&mut project).expect("audit op must apply");
    }
    (project, track_id)
}

enum KitChoice {
    Acoustic,
    Synth {
        device_name: &'static str,
        params: Vec<(String, f32)>,
    },
}

/// Decode (once) every sample the given key can trigger at the audit
/// velocity, keyed by the region sample-path string the engine asks for.
fn arm_for_key(sfz_abs: &Path, key: u8) -> Result<ArmedSfzInput, Box<dyn std::error::Error>> {
    let programs_dir = sfz_abs.parent().ok_or("sfz path has no parent")?;
    let limits = yeban_sfz::ParseLimits::default();
    let resolver = yeban_sfz::IncludeResolver::new(programs_dir, limits)?;
    let entry_name = sfz_abs
        .file_name()
        .ok_or("sfz path has no file name")?
        .to_string_lossy()
        .into_owned();
    let sources = resolver.resolve(&entry_name)?;
    let instrument = yeban_sfz::parse_sources(&sources, &limits)?;
    let mut samples = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for region in instrument.regions() {
        if region.lokey > i32::from(key)
            || region.hikey < i32::from(key)
            || region.lovel > VELOCITY
            || region.hivel < VELOCITY
        {
            continue;
        }
        let sample_path = region.sample_path().into_owned();
        if !seen.insert(sample_path.clone()) {
            continue;
        }
        let rel = sample_path.replace('\\', "/");
        let file = programs_dir.join(&rel);
        let file = normalize(&file);
        if rel.contains("*silence") || !file.exists() {
            continue;
        }
        let mono =
            yeban_decode::decode_mono_channel0(&file, &yeban_decode::DecodeOptions::default())
                .map_err(|e| format!("decode {}: {e}", file.display()))?;
        samples.push((sample_path, (Arc::new(mono.pcm), 1, mono.sample_hz as f32)));
    }
    Ok(ArmedSfzInput::new(sfz_abs.to_path_buf(), samples))
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) -> std::io::Result<()> {
    // Minimal 16-bit PCM WAV writer (stereo interleaved input).
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&2u16.to_le_bytes()); // stereo
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes)
}

fn render_one(
    repo_root: &Path,
    out_dir: &Path,
    label: &str,
    kit: &KitChoice,
    key: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let (project, track_id) = build_project(kit, key);
    let sfz_abs = repo_root.join(yeban_theory::ACOUSTIC_KIT_SFZ_PATH);
    let armed = match kit {
        KitChoice::Acoustic => Some(arm_for_key(&sfz_abs, key)?),
        KitChoice::Synth { .. } => None,
    };
    let mut provider = |tid: EntityId, _raw: &str| -> ArmedSfzInput {
        if tid == track_id {
            armed.clone().expect("acoustic provider armed")
        } else {
            ArmedSfzInput::empty("")
        }
    };
    let rendered = render_project_to_buffer(&project, &RenderSettings::default(), &mut provider)
        .map_err(|e| format!("render {label} key {key}: {e}"))?;
    if rendered
        .skipped_tracks
        .iter()
        .any(|skip| skip.track == track_id)
    {
        return Err(format!("render {label} key {key}: drum track was skipped").into());
    }
    let peak = rendered
        .samples
        .iter()
        .fold(0.0f32, |acc, s| acc.max(s.abs()));
    let rms = (rendered
        .samples
        .iter()
        .map(|s| f64::from(*s) * f64::from(*s))
        .sum::<f64>()
        / rendered.samples.len().max(1) as f64)
        .sqrt();
    let out_path = out_dir.join(format!("{label}-key{key}.wav"));
    write_wav(&out_path, &rendered.samples, rendered.sample_rate)?;
    println!(
        "{label} key {key}: frames={} peak={peak:.4} rms={rms:.4} -> {}",
        rendered.frames,
        out_path.display()
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let repo_root = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out_dir)?;

    for key in ACOUSTIC_KEYS {
        render_one(&repo_root, &out_dir, "acoustic", &KitChoice::Acoustic, key)?;
    }
    // Synth voices: snare/clap/hats are style-independent; kick differs.
    for style_id in ["trap", "house"] {
        let preset = yeban_theory::factory_drum_style(style_id)?.ok_or("style missing")?;
        let yeban_theory::DrumKitSetup::SynthDevice {
            device_name,
            params,
        } = yeban_theory::kit_setup(&preset)
        else {
            return Err("electronic style must produce a synth device".into());
        };
        let kit = KitChoice::Synth {
            device_name,
            params,
        };
        let keys: &[u8] = if style_id == "trap" {
            &SYNTH_KEYS
        } else {
            &[36]
        };
        for &key in keys {
            render_one(
                &repo_root,
                &out_dir,
                &format!("synth-{style_id}"),
                &kit,
                key,
            )?;
        }
    }
    println!("AUDIT_RENDER_DONE");
    Ok(())
}
