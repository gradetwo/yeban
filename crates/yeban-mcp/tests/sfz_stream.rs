//! P2 磁盘流式引擎：预载范围与声部流行为。
//!
//! 夹具与 `sfz_demand_load.rs` 同形（14 区域、13 文件、每文件 4800 帧），
//! 断言分两层：
//!
//! - **范围层**：预览源文件数／帧数与 P1 的 `load_with` 收据一致
//!   （稀疏谱子跳过 13 区域、1 文件、4800 帧；全量谱子跳过 0、13 文件、62400 帧）；
//! - **流层**：磁盘线程喂环、消费侧读块，读出的 PCM 与 `read_sample_file`
//!   的整文件解码逐位相同（同一条解码腿 ⇒ 可比），起音预载门控 `ready()`。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::time::{Duration, Instant};

use yeban_mcp::domain::sfz::{
    DemandLoadFilter, SampleLimits, SampleStore, SfzLoad, TrackSampleCache, load_with, parse,
    read_sample_file,
};
use yeban_mcp::domain::sfz_stream::{
    StreamEngine, StreamOptions, StreamVoice, preview_sources, realtime_sources,
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
            "yeban-mcp-sfz-stream-{}-{tag}-{}",
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
        },
    );
    project.tracks.insert(track_id, track);
    project
}

/// 调一次 `load_with`（共享池全新 ⇒ 每轨记账独立）。
fn load_once(sfz: &Path, filter: Option<&DemandLoadFilter>) -> SfzLoad {
    let mut store = SampleStore::new();
    let mut cache = TrackSampleCache::with_limits(&mut store, SampleLimits::default());
    let (load, _) = load_with(sfz, 48000.0, 0, false, &mut cache, filter).expect("装载");
    load
}

fn full_notes() -> Vec<(u8, u8)> {
    let mut notes: Vec<(u8, u8)> = (60..=71).map(|pitch| (pitch, 100)).collect();
    notes.push((72, 32));
    notes.push((72, 96));
    notes
}

/// 等声部就绪（测试侧轮询；磁盘线程解码 4800 帧 WAV 远快于 10 秒上限）。
fn wait_ready(voice: &StreamVoice, what: &str) {
    let start = Instant::now();
    while !voice.ready() {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("{what} 在 10 秒内未就绪");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 等声部播完（同上轮询；读盘失败的声部立即播完）。
fn wait_finished(voice: &StreamVoice, what: &str) {
    let start = Instant::now();
    while !voice.is_finished() {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("{what} 在 10 秒内未播完");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 把一个声部读到播完，返回实声帧（补零部分不计入）。
fn drain_voice(voice: &StreamVoice) -> Vec<f32> {
    let mut pcm = Vec::new();
    let mut block = [0.0_f32; 512];
    for _ in 0..100_000 {
        let read = voice.read_block(&mut block);
        pcm.extend_from_slice(&block[..read.from_ring]);
        if read.finished {
            break;
        }
    }
    pcm
}

/// 整文件解码的 PCM（与流式输出比对的基准：同一条解码腿）。
fn expected_pcm(path: &Path) -> Vec<f32> {
    read_sample_file(path).expect("读样本").pcm.to_vec()
}

/// 稀疏谱子：P1 收据（跳过 13、1 文件、4800 帧）与预览源一致，且流出的 PCM 逐位相同。
#[test]
fn preview_sparse_matches_p1_receipt_and_streams_exact_pcm() {
    let scratch = Scratch::new("sparse");
    let sfz = write_fixture_sfz(&scratch.join("inst"));
    let track_id = uid(1001);
    let project = project_with_notes(track_id, &[(60, 100)]);
    let filter = DemandLoadFilter::from_project_track(&project, track_id);
    let load = load_once(&sfz, Some(&filter));
    assert_eq!(load.skipped_regions, 13, "14 区域命中 1 条 ⇒ 跳过 13");
    assert_eq!(load.sample_sources.len(), 1, "稀疏谱子只读 1 文件");
    assert_eq!(load.sample_frames_loaded, 4800, "稀疏谱子帧数全计");

    let text = fs::read_to_string(&sfz).expect("读 SFZ");
    let instrument = parse(&text).expect("解析 SFZ");
    let sources = preview_sources(&sfz, &instrument, &filter);
    assert_eq!(sources.len(), 1, "预览源与 P1 收据同为 1 文件");
    assert_eq!(load.sample_sources.len(), sources.len());

    // 小环（1024）＋小块（1024）：4800 帧必经多次回绕与生产消费交错。
    let engine = StreamEngine::new();
    assert!(engine.is_thread_running(), "磁盘线程必须在跑");
    let options = StreamOptions {
        preload_frames: 512,
        ring_frames: 1024,
        chunk_frames: 1024,
    };
    let voice = engine.trigger(&sources[0], options);
    wait_ready(&voice, "稀疏声部起音");
    let streamed = drain_voice(&voice);
    assert!(voice.is_finished(), "读完后声部播完");
    assert_eq!(streamed.len(), 4800, "流出帧数与文件帧数一致");
    assert_eq!(
        streamed,
        expected_pcm(&sources[0]),
        "流出 PCM 必须与整文件解码逐位相同"
    );
    assert_eq!(voice.stats().frames_read, 4800, "累计实声帧数一致");
}

/// 全量谱子：P1 收据（跳过 0、13 文件、62400 帧）与预览源一致，13 声部逐个流出逐位相同。
#[test]
fn preview_full_matches_p1_receipt_and_streams_all_files() {
    let scratch = Scratch::new("full");
    let sfz = write_fixture_sfz(&scratch.join("inst"));
    let track_id = uid(1002);
    let project = project_with_notes(track_id, &full_notes());
    let filter = DemandLoadFilter::from_project_track(&project, track_id);
    let load = load_once(&sfz, Some(&filter));
    assert_eq!(load.skipped_regions, 0, "全量谱子无跳过");
    assert_eq!(load.sample_sources.len(), 13, "全量谱子读全 13 文件");
    assert_eq!(load.sample_frames_loaded, 13 * 4800, "全量谱子帧数全计");

    let text = fs::read_to_string(&sfz).expect("读 SFZ");
    let instrument = parse(&text).expect("解析 SFZ");
    let sources = preview_sources(&sfz, &instrument, &filter);
    assert_eq!(sources.len(), 13, "预览源与 P1 收据同为 13 文件");

    let engine = StreamEngine::new();
    let mut total = 0usize;
    for source in &sources {
        let voice = engine.trigger(source, StreamOptions::default());
        wait_ready(&voice, "全量声部起音");
        let streamed = drain_voice(&voice);
        assert_eq!(
            streamed,
            expected_pcm(source),
            "声部 {} 流出 PCM 必须逐位相同",
            source.display()
        );
        total += streamed.len();
    }
    assert_eq!(total, 13 * 4800, "13 声部流出帧数之和与 P1 收据一致");
}

/// 实时预载：谱子再稀疏也预载整件乐器（13 文件），不看判定器。
#[test]
fn realtime_preloads_whole_instrument_despite_sparse_score() {
    let scratch = Scratch::new("realtime");
    let sfz = write_fixture_sfz(&scratch.join("inst"));
    let track_id = uid(1003);
    let project = project_with_notes(track_id, &[(60, 100)]);
    let filter = DemandLoadFilter::from_project_track(&project, track_id);
    let text = fs::read_to_string(&sfz).expect("读 SFZ");
    let instrument = parse(&text).expect("解析 SFZ");
    let preview = preview_sources(&sfz, &instrument, &filter);
    let realtime = realtime_sources(&sfz, &instrument);
    assert_eq!(preview.len(), 1, "预览只收已用 1 文件");
    assert_eq!(realtime.len(), 13, "实时收整件乐器 13 文件");
    assert!(
        preview.iter().all(|path| realtime.contains(path)),
        "预览源必须是实时源的子集"
    );
}

/// 起音预载门控：`ready()` 为真后，第一块 512 帧必全是实声且与文件头逐位相同。
#[test]
fn onset_preload_gates_ready_and_first_block_is_exact() {
    let scratch = Scratch::new("onset");
    write_fixture_sfz(&scratch.join("inst"));
    let source = scratch.join("inst").join("tone60.wav");
    let engine = StreamEngine::new();
    let options = StreamOptions {
        preload_frames: 1024,
        ring_frames: 2048,
        chunk_frames: 1024,
    };
    let voice = engine.trigger(&source, options);
    wait_ready(&voice, "起音预载");
    let expected = expected_pcm(&source);
    let mut block = [0.0_f32; 512];
    let read = voice.read_block(&mut block);
    assert_eq!(read.from_ring, 512, "就绪后第一块必全是实声");
    assert_eq!(read.underrun, 0, "就绪后第一块必无欠载");
    assert_eq!(&block[..], &expected[..512], "第一块必须与文件头逐位相同");
}

/// 丢弃的声部取消作业、引擎 `Drop` 不挂：小环＋不读 ⇒ 磁盘线程必在环满处等取消。
#[test]
fn dropped_voice_cancels_job_and_engine_drops_cleanly() {
    let scratch = Scratch::new("drop");
    let dir = scratch.join("inst");
    fs::create_dir_all(&dir).expect("建 SFZ 目录");
    let big = dir.join("big.wav");
    write_constant_wav(&big, 0.3, 48000);
    let engine = StreamEngine::new();
    let options = StreamOptions {
        preload_frames: 4096,
        ring_frames: 256,
        chunk_frames: 4096,
    };
    let voice = engine.trigger(&big, options);
    drop(voice);
    // 引擎 `Drop` 会 `join` 磁盘线程：若取消旗没起作用，本测试会在这里挂住。
    drop(engine);
}

/// 读不到的声部恒静音：立即播完、读块全零、`failed` 为真。
#[test]
fn missing_file_voice_is_silent_and_finished() {
    let scratch = Scratch::new("missing");
    let engine = StreamEngine::new();
    let voice = engine.trigger(&scratch.join("nope.wav"), StreamOptions::default());
    wait_finished(&voice, "缺失文件声部");
    assert!(voice.stats().failed, "读盘失败必须登记");
    let mut block = [0.0_f32; 128];
    let read = voice.read_block(&mut block);
    assert_eq!(read.from_ring, 0, "失败声部无实声");
    assert_eq!(read.underrun, 128, "失败声部全块补零");
    assert!(block.iter().all(|&sample| sample == 0.0));
}
