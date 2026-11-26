//! 判据（CODE_AUDIT_REPORT P0-7）：**增量发布不在 UI 线程同步解码**。
//!
//! ## 缺陷与修法
//!
//! 60 Hz 心跳调 `EngineHost::publish_project` 时，缓存未命中的 SFZ 摊平与音频
//! 解码原本在**主线程同步**发生：挂载大音源库后第一次发布冻结界面数秒。
//! 修法：未命中的解码交给宿主私有的后台线程（[`yeban_app::arm_worker`]），
//! 本跳返回 [`PublishOutcome::Deferred`]（快照一位不动、发布标记不推进），
//! 心跳下一跳自动重试，就绪后完成发布；解码失败在就绪的那一跳以既有 `Err`
//! 形态响亮返回（⛔ 不静默）。
//!
//! ## 判定量（结构读数，⛔ 不用墙钟阈值判对错）
//!
//! | 判据 | 判定量 |
//! | :--- | :--- |
//! | [`publish_defers_on_uncached_sfz_and_publishes_when_ready`] | 首跳 `Deferred` ＋ 返回时 `misses == 0`（解码还没发生）＋ `should_publish` 仍真 ＋ 当前快照一位不动；重试就绪后 `Published` ＋ 快照带摊平表 ＋ `misses == 1`；再发布 ⇒ 立即 `Published`（缓存命中） |
//! | [`publish_defers_on_uncached_audio_and_publishes_when_ready`] | 首跳 `Deferred` ＋ 返回时 `decodes == 0`；就绪后 `Published` ＋ 注入报告 `injected == 1`、`decodes == 1` |
//! | [`publish_surfaces_sfz_failure_after_background_decode`] | 首跳 `Deferred`；重试 ⇒ `Err(Sfz)`（错误原文带 include cycle）＋ 当前快照一位不动；再重试仍 `Err`（失败记录复用，⛔ 不挂死、不重复解码） |
//!
//! ⚠ 若把解码改回同步内联，三条判据的"首跳 `Deferred`"与"返回时计数为 0"
//! 两组断言同时红 —— 这正是本单的判别点。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use yeban_app::audio_inject::AudioAssetSource;
use yeban_app::engine_host::{EditMark, EngineHost, EngineHostError, PublishOutcome};
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, RoutingEdge,
    RoutingGraph, RoutingKind, SamplerBinding, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

// ---------------------------------------------------------------------------
// SFZ 夹具（形态与 `sfz_realtime_cache.rs` 的夹具一致）
// ---------------------------------------------------------------------------

/// `24-bit` 满量程。
const I24_SCALE: f32 = 8_388_608.0;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-publish-async-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 写一条常量 PCM 样本（24-bit 单声道）。
fn write_const_sample(path: &Path, value: f32) {
    let scale = (I24_SCALE * value) as i32;
    let buffer = PcmBuffer::Int24(vec![scale; 4_800]);
    let format = PcmFormat::integer(1, 48_000, 24);
    write_plain_wav(path, &format, &buffer).expect("写样本夹具");
}

/// 工程里那条带 MIDI 音符的非主总线轨。
fn midi_track_of(project: &YebanProjectV1) -> EntityId {
    *project
        .tracks
        .iter()
        .find(|(id, track)| {
            **id != project.master_bus_track_id
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .map(|(id, _)| id)
        .expect("filled_project 必须有一条带 MIDI 音符的非主总线轨")
}

/// 那条轨上出现过的全部音高（排序 ＋ 去重）。
fn pitches_of(project: &YebanProjectV1, track_id: EntityId) -> Vec<u8> {
    let mut pitches: Vec<u8> = project
        .tracks
        .get(&track_id)
        .expect("轨必须在")
        .clips
        .values()
        .filter_map(|placement| project.clip_pool.get(&placement.clip_id))
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().map(|note| note.pitch))
        .collect();
    pitches.sort_unstable();
    pitches.dedup();
    assert!(!pitches.is_empty(), "夹具必须真的带音符");
    pitches
}

fn bind_sampler(project: &mut YebanProjectV1, track_id: EntityId, path: Option<&Path>) {
    project.tracks.get_mut(&track_id).expect("轨必须在").sampler =
        path.map(|path| SamplerBinding {
            note_octave_override: None,
            path: path.to_string_lossy().into_owned(),
        });
}

/// 写一份"每个工程音高一条区域"的 SFZ。
fn write_sfz(dir: &Path, name: &str, pitches: &[u8], sample: &str) -> PathBuf {
    let mut text = String::new();
    for pitch in pitches {
        text.push_str(&format!(
            "<region> sample={sample} key={pitch} pitch_keycenter={pitch}\n"
        ));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("写 SFZ 夹具");
    path
}

/// 反复发布同一标记直到不再延后；返回最终结局（`Published` 修订号或错误）。
///
/// 10 s 内不收敛 ⇒ panic（判据响亮地红，⛔ 不无限等）。
fn settle(
    host: &mut EngineHost,
    project: &YebanProjectV1,
    mark: EditMark,
) -> Result<u64, EngineHostError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match host.publish_project(project, mark)? {
            PublishOutcome::Published(revision) => return Ok(revision),
            PublishOutcome::Deferred => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "后台解码 10 s 内没有就绪"
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 音频夹具（形态与 `engine_audio_clip_injection.rs` 的夹具一致）
// ---------------------------------------------------------------------------

/// 写一份真 WAV 并读回字节（16-bit、常量样本）。
fn wav_bytes(frames: usize) -> Vec<u8> {
    let path = std::env::temp_dir().join(format!(
        "yeban-publish-async-audio-{}-{}.wav",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    let samples: Vec<i16> = vec![8_192; frames];
    write_plain_wav(
        &path,
        &PcmFormat::integer(1, 48_000, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写夹具 WAV");
    let bytes = std::fs::read(&path).expect("读夹具 WAV");
    std::fs::remove_file(&path).expect("删夹具 WAV");
    bytes
}

/// 最小工程：master ＋ 一条轨 ＋ 一个音频片段（一个摆放）。
fn project_with_audio(asset: &AssetHash) -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    let audio_clip = EntityId::new();

    let mut tracks = BTreeMap::new();
    tracks.insert(
        track,
        TrackV3 {
            id: track,
            name: "Audio".to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        },
    );
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut clip_pool = BTreeMap::new();
    clip_pool.insert(
        audio_clip,
        ClipPoolEntry {
            id: audio_clip,
            name: "take".to_owned(),
            content: ClipContent::Audio {
                asset: asset.clone(),
                gain_db: 0.0,
            },
        },
    );

    let mut graph = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );

    let mut project = YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks,
        routing_graph: graph,
        clip_pool,
        ..YebanProjectV1::default()
    };
    let placement_id = EntityId::new();
    project
        .tracks
        .get_mut(&track)
        .expect("轨在表里")
        .clips
        .insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id: audio_clip,
                start_tick: 0,
                duration_ticks: 1_920,
                loop_config: LoopConfig::default(),
                muted: false,
                offset_samples: None,
                fade_in_ticks: None,
                fade_out_ticks: None,
                source_start_samples: None,
            },
        );
    project
}

fn pool_source(pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>>) -> AudioAssetSource {
    Box::new(move |hash| pool.borrow().get(hash).cloned())
}

// ---------------------------------------------------------------------------
// 判据一：SFZ 未命中 ⇒ 延后；就绪 ⇒ 发布；之后命中 ⇒ 立即发布
// ---------------------------------------------------------------------------

#[test]
fn publish_defers_on_uncached_sfz_and_publishes_when_ready() {
    let dir = scratch("sfz-defer");
    write_const_sample(&dir.join("const.wav"), 0.25);
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let sfz_path = write_sfz(&dir, "keys.sfz", &pitches, "const.wav");

    // 起点：先 reload 一份**没有** sampler 的工程（缓存冷、引擎活）。
    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("无 sampler 的 reload");
    let before = host.current_snapshot().expect("有快照");

    bind_sampler(&mut project, lead, Some(&sfz_path));
    let mark = EditMark::new(None, 1);
    let first = host
        .publish_project(&project, mark)
        .expect("首跳不得失败（只是延后）");
    assert_eq!(
        first,
        PublishOutcome::Deferred,
        "⭐ 未命中的 SFZ 解码必须转后台：首跳延后，⛔ 不在主线程同步解码"
    );
    assert_eq!(
        host.sfz_cache_readings().misses,
        0,
        "⭐ 首跳返回时解码还没有发生（misses 仍 0）"
    );
    assert!(
        host.should_publish(mark),
        "⭐ 延后 ⇒ 发布标记不推进，下一跳心跳会重试"
    );
    assert_eq!(
        host.current_snapshot().expect("有快照"),
        before,
        "⭐ 延后期间当前快照一位不动"
    );

    let revision = settle(&mut host, &project, mark).expect("就绪后发布必须成功");
    assert_eq!(revision, 2, "就绪后发布的是第 2 个修订");
    let snapshot = host.current_snapshot().expect("有快照");
    assert!(
        snapshot.sfz_plan(&lead).is_some(),
        "⭐ 就绪后的快照必须带摊平表"
    );
    assert_eq!(host.sfz_cache_readings().misses, 1, "⭐ 后台恰好解码一次");
    assert!(!host.should_publish(mark), "发布完成 ⇒ 标记推进");

    // 同一工程再发布（新标记）⇒ 缓存命中 ⇒ 立即发布，不再延后。
    let mark2 = EditMark::new(None, 2);
    let again = host
        .publish_project(&project, mark2)
        .expect("命中发布必须成功");
    assert!(
        matches!(again, PublishOutcome::Published(_)),
        "⭐ 缓存命中 ⇒ 立即发布（实际 {again:?}）"
    );
    assert_eq!(host.sfz_cache_readings().misses, 1, "命中 ⛔ 不得再解码");
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// 判据二：音频未命中 ⇒ 延后；就绪 ⇒ 发布且注入报告如实
// ---------------------------------------------------------------------------

#[test]
fn publish_defers_on_uncached_audio_and_publishes_when_ready() {
    let bytes = wav_bytes(48_000);
    let hash = AssetHash::of_bytes(&bytes);
    let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> = Rc::new(RefCell::new(BTreeMap::new()));
    pool.borrow_mut().insert(hash.clone(), bytes);

    // 起点：reload 一份**没有**音频片段的工程（解码缓存冷、引擎活）。
    // ⚠ 不能用 `filled_project` 当基线：它自带音频片段，reload 会先把那份
    //    解码掉，`decodes == 0` 的判定量就被污染了。
    let mut host = EngineHost::new();
    host.set_audio_asset_source(pool_source(Rc::clone(&pool)));
    let mut plain = project_with_audio(&hash);
    plain.clip_pool.clear();
    for track in plain.tracks.values_mut() {
        track.clips.clear();
    }
    host.reload(&plain, 0).expect("无音频片段的 reload");
    let before = host.current_snapshot().expect("有快照");

    let project = project_with_audio(&hash);
    let mark = EditMark::new(None, 1);
    let first = host
        .publish_project(&project, mark)
        .expect("首跳不得失败（只是延后）");
    assert_eq!(
        first,
        PublishOutcome::Deferred,
        "⭐ 未命中的音频解码必须转后台：首跳延后"
    );
    assert_eq!(
        host.audio_cache_readings().decodes,
        0,
        "⭐ 首跳返回时解码还没有发生（decodes 仍 0）"
    );
    assert_eq!(
        host.current_snapshot().expect("有快照"),
        before,
        "⭐ 延后期间当前快照一位不动"
    );

    settle(&mut host, &project, mark).expect("就绪后发布必须成功");
    let report = host.audio_clip_report().clone();
    println!(
        "[publish-async/audio] {} | decodes={}",
        report.summary(),
        report.cache.decodes
    );
    assert_eq!(report.injected_assets, 1, "⭐ 就绪后注入 1 个素材");
    assert_eq!(report.cache.decodes, 1, "⭐ 后台恰好解码一次");
    assert_eq!(
        host.current_snapshot().expect("有快照").audio_pcm_spans(),
        1,
        "⭐ 就绪后的快照带 PCM 摆放"
    );
}

// ---------------------------------------------------------------------------
// 判据三：后台解码失败 ⇒ 就绪的那一跳响亮 Err，快照不动，且不重复解码
// ---------------------------------------------------------------------------

#[test]
fn publish_surfaces_sfz_failure_after_background_decode() {
    let dir = scratch("sfz-fail");
    // include 成环：文件都在（路径解析成功），解码必然失败（`sfz_include_load`
    // 的既有判据已钉住这条错误形态）。
    std::fs::write(dir.join("a.sfz"), "#include \"b.sfz\"\n").expect("写 a.sfz");
    std::fs::write(dir.join("b.sfz"), "#include \"a.sfz\"\n").expect("写 b.sfz");

    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("无 sampler 的 reload");
    let before = host.current_snapshot().expect("有快照");

    bind_sampler(&mut project, lead, Some(&dir.join("a.sfz")));
    let mark = EditMark::new(None, 1);
    let first = host
        .publish_project(&project, mark)
        .expect("首跳不得失败（只是延后）");
    assert_eq!(
        first,
        PublishOutcome::Deferred,
        "⭐ 解码转后台：首跳延后（失败还没有发生）"
    );

    let error = settle(&mut host, &project, mark).expect_err("后台解码失败必须响亮返回");
    println!("[publish-async/fail] error={error}");
    assert!(
        matches!(error, EngineHostError::Sfz(_)),
        "必须是 SFZ 错误（实际 {error:?}）"
    );
    assert!(
        error.to_string().contains("include cycle"),
        "错误原文必须带失败原因（实际 {error}）"
    );
    assert_eq!(
        host.current_snapshot().expect("有快照"),
        before,
        "⭐ 发布失败 ⇒ 当前快照一位不动"
    );

    // 再发布同一工程 ⇒ 失败记录直接返回（不挂死、不重复解码）。
    let again = host.publish_project(&project, EditMark::new(None, 2));
    assert!(
        matches!(again, Err(EngineHostError::Sfz(_))),
        "⭐ 失败记录必须复用（实际 {again:?}）"
    );
    std::fs::remove_dir_all(&dir).ok();
}
