//! P1 按需装载（装载级）：`load_with` 的过滤器行为。
//!
//! 夹具与 `sfz_demand_load_render.rs` 同形（14 区域、13 文件、每文件 4800 帧），
//! 但这里直接调 `load_with`，断言提供表与收据字段，不走完整渲染。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use yeban_mcp::domain::sfz::{
    DemandLoadFilter, SampleLimits, SampleStore, SfzLoad, TrackSampleCache, load_with,
};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, TrackV3,
    YebanProjectV1,
};

/// 一个临时目录，`Drop` 时删除自己。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-demand-load-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn uid(n: u128) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8{n:04}")).expect("合法 ULID")
}

/// 手写一份常量 16-bit 单声道 48 kHz WAV。
fn write_constant_wav(path: &Path, value: f32, frames: usize) {
    let scaled = (value * 32767.0).round() as i16;
    let mut data = Vec::with_capacity(frames * 2);
    for _ in 0..frames {
        data.extend_from_slice(&scaled.to_le_bytes());
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&48000_u32.to_le_bytes());
    out.extend_from_slice(&96000_u32.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    fs::write(path, &out).expect("写样本夹具");
}

/// 夹具乐器：音高 60..=71 各一文件，音高 72 的两个力度层共用一个文件。
fn write_fixture_sfz(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).expect("建 SFZ 目录");
    let mut text = String::new();
    for pitch in 60..=71 {
        let name = format!("tone{pitch}.wav");
        write_constant_wav(&dir.join(&name), 0.1 + (pitch - 60) as f32 * 0.05, 4800);
        text.push_str(&format!("<region> sample={name} key={pitch}\n"));
    }
    write_constant_wav(&dir.join("shared.wav"), 0.9, 4800);
    text.push_str("<region> sample=shared.wav key=72 lovel=0 hivel=63\n");
    text.push_str("<region> sample=shared.wav key=72 lovel=64 hivel=127\n");
    let path = dir.join("demand.sfz");
    fs::write(&path, &text).expect("写 SFZ 夹具");
    path
}

/// 含一条 MIDI 片段（给定音高/力度）的最小工程。
fn project_with_notes(track_id: EntityId, notes: &[(u8, u8)]) -> YebanProjectV1 {
    let mut project = YebanProjectV1::default();
    let clip_id = uid(9001);
    let mut midi_notes = BTreeMap::new();
    for (i, &(pitch, velocity)) in notes.iter().enumerate() {
        let id = uid(9100 + i as u128);
        let mut note = MidiNote::new(id, (i as u64) * 960, pitch, 960);
        note.velocity = velocity;
        midi_notes.insert(id, note);
    }
    project.clip_pool.insert(
        clip_id,
        ClipPoolEntry {
            id: clip_id,
            name: "t".to_owned(),
            content: ClipContent::Midi { notes: midi_notes },
        },
    );
    let mut track = TrackV3 {
        id: track_id,
        ..TrackV3::default()
    };
    let placement_id = uid(9200);
    track.clips.insert(
        placement_id,
        ClipPlacement {
            id: placement_id,
            clip_id,
            start_tick: 0,
            duration_ticks: 960 * notes.len().max(1) as u64,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    project.tracks.insert(track_id, track);
    project
}

/// 调一次 `load_with`（共享池全新 ⇒ 每轨记账独立）。
fn load_once(
    sfz: &Path,
    filter: Option<&DemandLoadFilter>,
    limits: SampleLimits,
) -> (SfzLoad, Vec<(String, bool, String, u64)>) {
    let mut store = SampleStore::new();
    let mut cache = TrackSampleCache::with_limits(&mut store, limits);
    let (load, table) = load_with(sfz, 48000.0, 0, false, &mut cache, filter).expect("装载");
    let table_summary = table
        .iter()
        .map(|(path, outcome)| {
            (
                path.clone(),
                outcome.is_ok(),
                outcome
                    .as_ref()
                    .map(|r| r.describe.clone())
                    .unwrap_or_default(),
                outcome.as_ref().map(|r| r.frames).unwrap_or(0),
            )
        })
        .collect();
    (load, table_summary)
}

fn full_notes() -> Vec<(u8, u8)> {
    let mut notes: Vec<(u8, u8)> = (60..=71).map(|pitch| (pitch, 100)).collect();
    notes.push((72, 32));
    notes.push((72, 96));
    notes
}

/// 全量谱子：过滤后装载与不过滤装载的提供表与收据字段逐项相同（收据一致口径）。
#[test]
fn full_score_load_matches_unfiltered_load() {
    let scratch = Scratch::new("full");
    let sfz = write_fixture_sfz(&scratch.join("inst"));
    let track_id = uid(1001);
    let project = project_with_notes(track_id, &full_notes());
    let filter = DemandLoadFilter::from_project_track(&project, track_id);
    let (filtered, filtered_table) = load_once(&sfz, Some(&filter), SampleLimits::default());
    let (plain, plain_table) = load_once(&sfz, None, SampleLimits::default());
    assert_eq!(filtered.skipped_regions, 0, "全量谱子无跳过");
    assert_eq!(filtered_table, plain_table, "提供表必须逐项相同");
    assert_eq!(filtered.regions.len(), plain.regions.len(), "区域数相同");
    assert_eq!(filtered.sample_sources, plain.sample_sources, "来源相同");
    assert_eq!(filtered.sample_failures, plain.sample_failures, "失败相同");
    assert_eq!(
        filtered.sample_frames_loaded, plain.sample_frames_loaded,
        "帧数相同"
    );
    assert_eq!(
        filtered.sample_reads_truncated, plain.sample_reads_truncated,
        "截断标志相同"
    );
    assert_eq!(filtered.silent_regions, plain.silent_regions, "静音数相同");
    assert_eq!(filtered.sample_sources.len(), 13, "全量谱子读全 13 文件");
    assert_eq!(filtered.sample_frames_loaded, 13 * 4800, "全量谱子帧数全计");
}

/// 预算只对已用子集记账：上限 2 文件时，1 文件谱子不截断、3 文件谱子截断。
#[test]
fn budget_counts_only_used_subset() {
    let scratch = Scratch::new("budget");
    let sfz = write_fixture_sfz(&scratch.join("inst"));
    let limits = SampleLimits {
        max_files: 2,
        max_frames: 0,
    };
    let track_id = uid(1002);
    let project = project_with_notes(track_id, &[(60, 100)]);
    let filter = DemandLoadFilter::from_project_track(&project, track_id);
    let (sparse, _) = load_once(&sfz, Some(&filter), limits);
    assert!(
        !sparse.sample_reads_truncated,
        "已用 1 文件 < 上限 2 ⇒ 不截断"
    );
    assert_eq!(sparse.sample_sources.len(), 1, "稀疏谱子只读 1 文件");
    let project = project_with_notes(track_id, &[(60, 100), (61, 100), (62, 100)]);
    let filter = DemandLoadFilter::from_project_track(&project, track_id);
    let (three, _) = load_once(&sfz, Some(&filter), limits);
    assert!(
        three.sample_reads_truncated,
        "已用 3 文件 > 上限 2 ⇒ 响亮截断"
    );
}

/// 共用文件：命中层解码一次进提供表，跳过层的摘要记 skippedDemandLoad，表里仍有该文件。
#[test]
fn shared_path_decoded_once_and_visible_to_skipped_region() {
    let scratch = Scratch::new("shared");
    let sfz = write_fixture_sfz(&scratch.join("inst"));
    let track_id = uid(1003);
    let project = project_with_notes(track_id, &[(72, 32)]);
    let filter = DemandLoadFilter::from_project_track(&project, track_id);
    let (load, table) = load_once(&sfz, Some(&filter), SampleLimits::default());
    assert_eq!(load.skipped_regions, 13, "14 区域命中 1 条 ⇒ 跳过 13");
    let shared_entries: Vec<_> = table
        .iter()
        .filter(|(path, _, _, _)| path == "shared.wav")
        .collect();
    assert_eq!(shared_entries.len(), 1, "共用文件在提供表里恰好一次");
    assert!(shared_entries[0].1, "共用文件读取成功");
    let shared_summaries: Vec<_> = load
        .regions
        .iter()
        .filter(|summary| summary.sample == "shared.wav")
        .collect();
    assert_eq!(shared_summaries.len(), 2, "两个力度层都在摘要里");
    assert!(
        shared_summaries
            .iter()
            .any(|summary| summary.sample_file_state == "skippedDemandLoad"),
        "跳过层摘要记 skippedDemandLoad: {:?}",
        shared_summaries
            .iter()
            .map(|summary| &summary.sample_file_state)
            .collect::<Vec<_>>()
    );
}
