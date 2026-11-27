//! T-FIX-443：音频摆放 fade 的**渲染器端到端**判据。
//!
//! 已有判据只有 `yeban-model::clip_fade` 的模型单元（窗口数学）与 notes 的解析往返；
//! MCP/引擎/app 测试里 `fade_in_ticks: Some` 零命中。本文件走**离线母带出口**
//! （`export_wav_with_sfz_file_and_assets_under`，与实时引擎共用同一份 audio_clip
//! 采样循环）从 WAV 字节读回包络。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use yeban_app::export_wav::{WavExportOptions, export_wav_with_sfz_file_and_assets_under};
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote,
    RoutingEdge, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav_f32, write_plain_wav};

fn id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-fade-{tag}-{}-{}",
        std::process::id(),
        EntityId::new()
    ));
    std::fs::create_dir_all(&dir).expect("临时目录");
    dir
}

fn sine_asset(dir: &Path, name: &str, frequency: f64) -> (AssetHash, Vec<u8>) {
    let path = dir.join(name);
    let mut samples = Vec::with_capacity(48_000 * 2);
    for frame in 0..48_000_u32 {
        let phase = 2.0 * std::f64::consts::PI * frequency * f64::from(frame) / 48_000.0;
        let value = (phase.sin() * 0.5) as f32;
        samples.push(value);
        samples.push(value);
    }
    write_plain_wav(
        &path,
        &PcmFormat::float(2, 48_000, 32),
        &PcmBuffer::Float32(samples),
    )
    .expect("写夹具");
    let bytes = std::fs::read(&path).expect("读夹具");
    (AssetHash::of_bytes(&bytes), bytes)
}

fn project_with(
    asset: AssetHash,
    asset_two: Option<AssetHash>,
    placements: &[(u64, u64, Option<u64>, Option<u64>)],
    midi: bool,
) -> YebanProjectV1 {
    let master = id("M1");
    let track_id = id("T0");
    let mut project = YebanProjectV1 {
        id: id("Z1"),
        title: "fade e2e".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0002,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(master);
    let mut track = TrackV3 {
        id: track_id,
        name: if midi { "Midi" } else { "Audio" }.to_owned(),
        kind: if midi {
            TrackKind::Midi
        } else {
            TrackKind::Audio
        },
        ..TrackV3::default()
    };
    for (index, (start, duration, fade_in, fade_out)) in placements.iter().enumerate() {
        let clip = id(&format!("K{index}"));
        let placement = id(&format!("P{index}"));
        let content = if midi {
            ClipContent::Midi {
                notes: BTreeMap::from([(
                    id(&format!("N{index}")),
                    MidiNote::new(id(&format!("N{index}")), 0, 60, 480),
                )]),
            }
        } else {
            ClipContent::Audio {
                asset: if index == 1 {
                    asset_two.clone().unwrap_or_else(|| asset.clone())
                } else {
                    asset.clone()
                },
                gain_db: 0.0,
            }
        };
        project.clip_pool.insert(
            clip,
            ClipPoolEntry {
                id: clip,
                name: format!("clip-{index}"),
                content,
            },
        );
        track.clips.insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: clip,
                start_tick: *start,
                duration_ticks: *duration,
                loop_config: LoopConfig::default(),
                muted: false,
                offset_samples: None,
                fade_in_ticks: *fade_in,
                fade_out_ticks: *fade_out,
                source_start_samples: None,
            },
        );
    }
    project.tracks.insert(track_id, track);
    project.routing_graph.nodes.push(track_id);
    let edge = id("E0");
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
    project
}

fn export(
    project: &YebanProjectV1,
    assets: &[(AssetHash, Vec<u8>)],
    dir: &Path,
    name: &str,
) -> (Vec<f32>, usize) {
    let path = dir.join(name);
    let options = WavExportOptions {
        frames: 48_000,
        ..Default::default()
    };
    export_wav_with_sfz_file_and_assets_under(project, &path, options, None, assets, None)
        .expect("离线导出必须成功");
    let (samples, channels, _, _) = read_plain_wav_f32(&path).expect("读回 WAV");
    (samples, usize::from(channels))
}

fn rms(samples: &[f32], channels: usize, start: usize, count: usize) -> f32 {
    let mut sum = 0.0_f64;
    let mut n = 0;
    for frame in start..(start + count).min(samples.len() / channels) {
        for channel in 0..channels {
            let value = f64::from(samples[frame * channels + channel]);
            sum += value * value;
            n += 1;
        }
    }
    (sum / n as f64).sqrt() as f32
}

#[test]
fn a_single_placement_fade_envelope_is_audible_at_head_and_tail() {
    let dir = scratch("single");
    let (asset, bytes) = sine_asset(&dir, "sine.wav", 440.0);
    let (asset_two, bytes_two) = sine_asset(&dir, "sine2.wav", 660.0);
    let assets = vec![(asset.clone(), bytes), (asset_two.clone(), bytes_two)];
    let dry = project_with(asset.clone(), None, &[(0, 1920, None, None)], false);
    let wet = project_with(asset, None, &[(0, 1920, Some(240), Some(240))], false);
    let (dry_samples, dry_channels) = export(&dry, &assets, &dir, "dry.wav");
    let (wet_samples, wet_channels) = export(&wet, &assets, &dir, "wet.wav");
    assert_ne!(
        AssetHash::of_bytes(&std::fs::read(dir.join("dry.wav")).unwrap()).as_str(),
        AssetHash::of_bytes(&std::fs::read(dir.join("wet.wav")).unwrap()).as_str(),
        "有 fade 与无 fade 的母带字节必须不同"
    );
    let head_dry = rms(&dry_samples, dry_channels, 0, 2400);
    let head_wet = rms(&wet_samples, wet_channels, 0, 2400);
    let tail_dry = rms(&dry_samples, dry_channels, 45_600, 2400);
    let tail_wet = rms(&wet_samples, wet_channels, 45_600, 2400);
    assert!(
        head_wet < head_dry * 0.5,
        "淡入必须压低头部包络: {head_wet} vs {head_dry}"
    );
    assert!(
        tail_wet < tail_dry * 0.5,
        "淡出必须压低头部包络: {tail_wet} vs {tail_dry}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn overlapping_placements_with_explicit_fades_keep_power_constant() {
    let dir = scratch("overlap");
    let (asset, bytes) = sine_asset(&dir, "sine.wav", 440.0);
    let (asset_two, bytes_two) = sine_asset(&dir, "sine2.wav", 660.0);
    let assets = vec![(asset.clone(), bytes), (asset_two.clone(), bytes_two)];
    // 第二条从 0.75 s（1440 tick）开始，与第一条重叠 0.25 s；两条都在重叠区显式 fade。
    let project = project_with(
        asset.clone(),
        Some(asset_two),
        &[(0, 1920, None, Some(480)), (1440, 1920, Some(480), None)],
        false,
    );
    let (samples, channels) = export(&project, &assets, &dir, "overlap.wav");
    // 重叠区 0.75–1.00 s：等功率交叉淡化 ⇒ 与单条 RMS 同一量级（不是 2×，也不是 0）。
    let overlap_rms = rms(&samples, channels, 36_000, 12_000);
    assert!(
        (0.1..0.45).contains(&overlap_rms),
        "重叠区等功率和必须落在单条量级（0.1..0.45），实际 {overlap_rms}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_midi_placement_cannot_carry_audio_fades_silently() {
    let dir = scratch("midi");
    let (asset, _bytes) = sine_asset(&dir, "sine.wav", 440.0);
    let project = project_with(asset, None, &[(0, 1920, Some(240), Some(240))], true);
    let error = project
        .validate()
        .expect_err("MIDI 摆放带 fade 必须响亮拒绝，⛔ 不静默 no-op");
    let text = error.to_string();
    assert!(text.contains("fade"), "拒绝文案必须点名 fade: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}
