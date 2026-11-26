//! P2 磁盘流式引擎：预载范围与声部流行为。
//!
//! 夹具与 `sfz_demand_load.rs` 同形（14 区域、13 文件、每文件 4800 帧），
//! 断言分两层：
//!
//! - **范围层**：预览源文件数／帧数与 P1 的 `load_with` 收据一致
//!   （稀疏谱子跳过 13 区域、1 文件、4800 帧；全量谱子跳过 0、13 文件、62400 帧）；
//! - **流层**：磁盘线程喂环、消费侧读块，读出的 PCM 与 `read_sample_file`
//!   的整文件解码逐位相同（同一条解码腿 ⇒ 可比），起音预载门控 `ready()`。
//!
//! 立体声（台账 §15.8 缺口二）：流层全程保声道——立体声 WAV／FLAC 流出的
//! 交错 PCM 与整文件保声道解码逐位相同、side 分量可测；>2 声道响亮拒绝
//! （声部 `failed`、不入缓存）；池按立体声求和（单声道声部居中入两声道）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::time::{Duration, Instant};

use yeban_decode::testfix::{FlacSpec, flac_constant};
use yeban_decode::{DecodeOptions, decode_mono_channel0, decode_path};
use yeban_mcp::domain::sfz::{
    DemandLoadFilter, SampleLimits, SampleStore, SfzLoad, TrackSampleCache, load_with, parse,
    read_sample_file,
};
use yeban_mcp::domain::sfz_stream::{
    StreamCacheConfig, StreamEngine, StreamOptions, StreamVoice, StreamingVoicePool,
    preview_sources, realtime_sources,
};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, TrackV3,
    YebanProjectV1,
};
use yeban_sfz::voice_pool::StealFade;

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

/// 手写一份常量多声道 16-bit 48 kHz WAV：`channel_values` 逐声道给值
/// （长度即声道数），每帧各声道恒为该值。立体声用两个不同值造声像，
/// >2 声道用三个值造拒绝判据的输入。
fn write_wav_channels(path: &Path, channel_values: &[f32], frames: usize) {
    let channels = channel_values.len() as u16;
    let scaled: Vec<i16> = channel_values
        .iter()
        .map(|value| (value * 32767.0).round() as i16)
        .collect();
    let block_align = channels * 2;
    let mut data = Vec::with_capacity(frames * block_align as usize);
    for _ in 0..frames {
        for value in &scaled {
            data.extend_from_slice(&value.to_le_bytes());
        }
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&48000_u32.to_le_bytes());
    out.extend_from_slice(&(48000_u32 * u32::from(block_align)).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    fs::write(path, &out).expect("写多声道样本夹具");
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
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
        },
    );
    project.tracks.insert(track_id, track);
    project
}

/// 调一次 `load_with`（共享池全新 ⇒ 每轨记账独立）。
fn load_once(sfz: &Path, filter: Option<&DemandLoadFilter>) -> SfzLoad {
    let mut store = SampleStore::new();
    let mut cache = TrackSampleCache::with_limits(&mut store, SampleLimits::default());
    let (load, _) =
        load_with(sfz, 48000.0, 0, false, &mut cache, filter, &BTreeMap::new()).expect("装载");
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

/// 把一个声部读到播完，返回实声的**交错样本**（补零部分不计入）。
/// 声道数在就绪后已发布（≥1）；读块按 帧 × 声道 定长，
/// `from_ring` 按帧计 ⇒ 实声样本数 = `from_ring × channels`。
fn drain_voice(voice: &StreamVoice) -> Vec<f32> {
    let channels = voice.channels().max(1);
    let mut pcm = Vec::new();
    let mut block = vec![0.0_f32; 512 * channels];
    for _ in 0..100_000 {
        let read = voice.read_block(&mut block);
        pcm.extend_from_slice(&block[..read.from_ring * channels]);
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
    assert_eq!(block.len(), 128, "守卫界：读块非空，all 断言非真空");
    assert!(block.iter().all(|&sample| sample == 0.0));
}

/// 等池内声部就绪（`voice_ready() == Some(true)`；与 `wait_ready` 同口径轮询）。
fn wait_pool_ready(
    pool: &StreamingVoicePool,
    handle: yeban_sfz::voice_pool::VoiceHandle,
    what: &str,
) {
    let start = Instant::now();
    while pool.voice_ready(handle) != Some(true) {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("{what} 在 10 秒内未就绪");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 等缓存统计量收敛到稳态（磁盘线程按序喂完、全部 insert 落袋后才稳定）。
fn wait_cache_steady(pool: &StreamingVoicePool, entries: usize, evictions: u64, what: &str) {
    let start = Instant::now();
    loop {
        let stats = pool.cache_stats();
        if stats.entries == entries && stats.evictions == evictions {
            return;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!(
                "{what} 在 10 秒内未收敛（entries={} evictions={}）",
                stats.entries, stats.evictions
            );
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 逐位比对两条 f32 序列（`to_bits` 向量相等；不走近似比较）。
fn assert_bits_eq(actual: &[f32], expected: &[f32], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}：长度不同");
    let actual_bits: Vec<u32> = actual.iter().map(|sample| sample.to_bits()).collect();
    let expected_bits: Vec<u32> = expected.iter().map(|sample| sample.to_bits()).collect();
    assert_eq!(actual_bits, expected_bits, "{what}：逐位不同");
}

/// P3 池两声部求和与 P1 整文件解码求和逐位相同（0 dB、无包络、无抢占、无循环）。
/// 池出口是立体声：两条**单声道**声部居中入两声道 ⇒ 每帧左右同为单声道和。
#[test]
fn pool_sums_two_voices_bit_identical_to_p1() {
    let scratch = Scratch::new("pool_sum");
    let a = scratch.join("a.wav");
    let b = scratch.join("b.wav");
    write_constant_wav(&a, 0.25, 4800);
    write_constant_wav(&b, 0.5, 4800);
    let options = StreamOptions {
        preload_frames: 4800,
        ring_frames: 16384,
        chunk_frames: 4096,
    };
    let mut pool = StreamingVoicePool::new(4, 48000.0, 4096).expect("建池");
    let ha = pool.trigger(60, 100, 0.0, &a, options).started();
    let hb = pool.trigger(64, 100, 0.0, &b, options).started();
    wait_pool_ready(&pool, ha, "声部 a");
    wait_pool_ready(&pool, hb, "声部 b");
    let mut rendered = Vec::new();
    for _ in 0..5 {
        let mut block = vec![0.0_f32; 2048];
        pool.render_block(&mut block);
        rendered.extend_from_slice(&block);
    }
    let pcm_a = expected_pcm(&a);
    let pcm_b = expected_pcm(&b);
    let mut expected = Vec::with_capacity(5120 * 2);
    for i in 0..5120 {
        let mut sample = 0.0_f32;
        if i < 4800 {
            sample += pcm_a[i];
            sample += pcm_b[i];
        }
        expected.push(sample);
        expected.push(sample);
    }
    assert_bits_eq(&rendered, &expected, "池求和与 P1 求和");
    assert_eq!(pool.active_count(), 0, "两声部播完后槽位应全部回收");
}

/// §15.8 缺口二判据：池按声道求和——立体声声部左右各入各声道，
/// 单声道声部同值入两声道（居中），逐位比对。
#[test]
fn pool_sums_stereo_and_mono_voices_per_channel() {
    let scratch = Scratch::new("pool_stereo");
    let stereo = scratch.join("s.wav");
    let mono = scratch.join("m.wav");
    write_wav_channels(&stereo, &[0.25, 0.5], 4800);
    write_constant_wav(&mono, 0.25, 4800);
    let options = StreamOptions {
        preload_frames: 4800,
        ring_frames: 16384,
        chunk_frames: 4096,
    };
    let mut pool = StreamingVoicePool::new(4, 48000.0, 4096).expect("建池");
    let hs = pool.trigger(60, 100, 0.0, &stereo, options).started();
    let hm = pool.trigger(64, 100, 0.0, &mono, options).started();
    wait_pool_ready(&pool, hs, "立体声声部");
    wait_pool_ready(&pool, hm, "单声道声部");
    let mut rendered = Vec::new();
    for _ in 0..5 {
        let mut block = vec![0.0_f32; 2048];
        pool.render_block(&mut block);
        rendered.extend_from_slice(&block);
    }
    let pcm_s = expected_pcm(&stereo);
    let pcm_m = expected_pcm(&mono);
    let mut expected = Vec::with_capacity(5120 * 2);
    for i in 0..5120 {
        let (mut left, mut right) = (0.0_f32, 0.0_f32);
        if i < 4800 {
            left += pcm_s[i * 2];
            right += pcm_s[i * 2 + 1];
            left += pcm_m[i];
            right += pcm_m[i];
        }
        expected.push(left);
        expected.push(right);
    }
    assert_bits_eq(&rendered, &expected, "池立体声求和（单声道声部居中）");
    assert_eq!(pool.active_count(), 0, "两声部播完后槽位应全部回收");
}

/// note-off 退役淡出：每**帧**按 `StealFade::gain_at` 衰减（两声道同增益），
/// 144 帧后归零并回收槽位。
#[test]
fn pool_retire_fade_follows_steal_fade_per_frame() {
    let scratch = Scratch::new("pool_retire");
    let file = scratch.join("c.wav");
    write_constant_wav(&file, 0.5, 4800);
    let options = StreamOptions {
        preload_frames: 4800,
        ring_frames: 16384,
        chunk_frames: 4096,
    };
    let mut pool = StreamingVoicePool::new(2, 48000.0, 4096).expect("建池");
    let handle = pool.trigger(60, 100, 0.0, &file, options).started();
    wait_pool_ready(&pool, handle, "退役声部");
    let mut head = vec![0.0_f32; 200];
    pool.render_block(&mut head);
    assert_bits_eq(&head, &vec![0.5_f32; 200], "退役前应为全增益（左右同值）");
    pool.note_off(handle).expect("note-off");
    let mut tail = vec![0.0_f32; 400];
    pool.render_block(&mut tail);
    let fade = StealFade::new(48000.0);
    let pcm = expected_pcm(&file);
    let mut expected = Vec::with_capacity(400);
    for i in 0..200 {
        let sample = pcm[100 + i] * 1.0 * fade.gain_at(i as u32);
        expected.push(sample);
        expected.push(sample);
    }
    assert_bits_eq(&tail, &expected, "退役淡出应逐帧跟随 StealFade");
    assert_eq!(pool.active_count(), 0, "淡出走完后槽位应回收");
}

/// 抢占：新声部按 `StealFade` 的剩余计数镜像淡入（首帧 0.0，144 帧后满增益）。
#[test]
fn pool_steal_fades_in_the_replacement_per_frame() {
    let scratch = Scratch::new("pool_steal");
    let a = scratch.join("a.wav");
    let b = scratch.join("b.wav");
    write_constant_wav(&a, 0.25, 4800);
    write_constant_wav(&b, 0.5, 4800);
    let options = StreamOptions {
        preload_frames: 4800,
        ring_frames: 16384,
        chunk_frames: 4096,
    };
    let mut pool = StreamingVoicePool::new(1, 48000.0, 4096).expect("建池");
    let ha = pool.trigger(60, 100, 0.0, &a, options).started();
    let outcome = pool.trigger(64, 100, 0.0, &b, options);
    assert_eq!(
        outcome.victim(),
        Some(ha),
        "容量 1 时第二触发必须窃取第一声部"
    );
    let hb = outcome.started();
    wait_pool_ready(&pool, hb, "窃取后的新声部");
    let mut block = vec![0.0_f32; 400];
    pool.render_block(&mut block);
    let fade = StealFade::new(48000.0);
    let pcm_b = expected_pcm(&b);
    let mut expected = Vec::with_capacity(400);
    for (i, sample) in pcm_b.iter().take(200).enumerate() {
        let gain = if i < 144 {
            fade.gain_at((144 - i) as u32)
        } else {
            1.0
        };
        let faded = sample * 1.0 * gain;
        expected.push(faded);
        expected.push(faded);
    }
    assert_bits_eq(&block, &expected, "窃取后的新声部应逐帧淡入");
    assert_eq!(pool.active_count(), 1, "新声部仍在播");
}

/// 样本回收：上限 2 时触发 3 个不同文件，必有 1 次淘汰、条目数回到上限。
#[test]
fn pool_cache_evicts_under_a_small_cap() {
    let scratch = Scratch::new("pool_gc");
    let files: Vec<PathBuf> = ["a.wav", "b.wav", "c.wav"]
        .iter()
        .map(|name| {
            let path = scratch.join(name);
            write_constant_wav(&path, 0.3, 4800);
            path
        })
        .collect();
    let options = StreamOptions {
        preload_frames: 4800,
        ring_frames: 16384,
        chunk_frames: 4096,
    };
    let mut pool = StreamingVoicePool::with_cache_config(
        4,
        48000.0,
        4096,
        StreamCacheConfig {
            max_entries: 2,
            idle_timeout_secs: 3600,
        },
    )
    .expect("建池");
    let mut handles = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let handle = pool
            .trigger(60 + index as u8, 100, 0.0, file, options)
            .started();
        handles.push(handle);
    }
    for (index, handle) in handles.iter().enumerate() {
        wait_pool_ready(&pool, *handle, &format!("GC 声部 {index}"));
    }
    wait_cache_steady(&pool, 2, 1, "GC 缓存");
    let stats = pool.cache_stats();
    assert_eq!(stats.hits, 0, "三文件各触发一次 ⇒ 无命中");
    assert_eq!(stats.misses, 3, "三文件各触发一次 ⇒ 三次未命中");
    assert_eq!(stats.evictions, 1, "上限 2 存 3 条 ⇒ 恰一次淘汰");
    assert_eq!(stats.entries, 2, "淘汰后条目数回到上限");
}

/// 流式 FLAC 声部：磁盘线程解出的 PCM 与 `decode_mono_channel0` 逐位相同。
#[test]
fn streamed_flac_voice_matches_decode_mono_channel0() {
    let scratch = Scratch::new("stream_flac");
    let spec = FlacSpec {
        sample_rate: 48000,
        channels: 1,
        bits: 16,
        block_frames: 256,
        total_samples_override: None,
    };
    let bytes = flac_constant(&spec, 4, 16384);
    let path = scratch.join("tone.flac");
    fs::write(&path, &bytes).expect("写 FLAC 夹具");
    let engine = StreamEngine::new();
    let voice = engine.trigger(&path, StreamOptions::default());
    wait_ready(&voice, "FLAC 声部");
    let drained = drain_voice(&voice);
    let expected = decode_mono_channel0(&path, &DecodeOptions::default())
        .expect("整文件解 FLAC")
        .pcm;
    assert_bits_eq(&drained, &expected, "流式 FLAC 与整文件解码");
}

/// ⭐ §15.8 缺口二判据：立体声 WAV 流出的交错 PCM 与整文件保声道解码
/// **逐位相同**（小环＋小块强制多次回绕），且 side/mid 比与夹具构造值同量级
/// （L=0.4、R=−0.2 ⇒ mid=0.1、side=0.3、比值 3；折声道 0 的假立体声恒为 0）。
#[test]
fn stereo_wav_streams_interleaved_bit_identical_and_side_is_measurable() {
    let scratch = Scratch::new("stereo_stream");
    let path = scratch.join("stereo.wav");
    write_wav_channels(&path, &[0.4, -0.2], 4800);
    let engine = StreamEngine::new();
    let options = StreamOptions {
        preload_frames: 512,
        ring_frames: 1024,
        chunk_frames: 1024,
    };
    let voice = engine.trigger(&path, options);
    wait_ready(&voice, "立体声声部起音");
    assert_eq!(voice.channels(), 2, "声道数必须发布为 2");
    let streamed = drain_voice(&voice);
    assert_eq!(streamed.len(), 4800 * 2, "流出交错样本数 = 帧数 × 声道数");
    assert_bits_eq(
        &streamed,
        &expected_pcm(&path),
        "流式立体声与整文件保声道解码",
    );
    assert_eq!(voice.stats().frames_read, 4800, "累计实声按帧计");
    let (mut mid_sq, mut side_sq) = (0.0_f64, 0.0_f64);
    for frame_index in 0..streamed.len() / 2 {
        let (left, right) = (
            f64::from(streamed[frame_index * 2]),
            f64::from(streamed[frame_index * 2 + 1]),
        );
        mid_sq += ((left + right) / 2.0).powi(2);
        side_sq += ((left - right) / 2.0).powi(2);
    }
    let ratio = (side_sq / mid_sq).sqrt();
    println!("[sfz_stream] stereo side/mid RMS ratio = {ratio:.3}（构造值 3.0）");
    assert!(
        ratio > 2.0,
        "side/mid 应约 3（实测 {ratio}）；折声道 0 或复制单声道的假立体声恒为 0"
    );
}

/// ⭐ §15.5／§15.8 判据：>2 声道 WAV 在流式腿**响亮拒绝**——声部 `failed`、
/// 恒静音、不发布声道数、文件不入样本缓存（⛔ 不再静默折声道 0）。
#[test]
fn three_channel_wav_voice_is_rejected_loudly_not_folded() {
    let scratch = Scratch::new("three_channel");
    let path = scratch.join("trio.wav");
    write_wav_channels(&path, &[0.3, 0.3, 0.3], 4800);
    let engine = StreamEngine::new();
    let voice = engine.trigger(&path, StreamOptions::default());
    wait_finished(&voice, "三声道声部");
    assert!(voice.stats().failed, ">2 声道必须响亮拒绝（failed 为真）");
    assert_eq!(voice.channels(), 0, "被拒声部不发布声道数");
    let mut block = [0.0_f32; 128];
    let read = voice.read_block(&mut block);
    assert_eq!(read.from_ring, 0, "被拒声部无实声");
    assert_eq!(block.len(), 128, "守卫界：读块非空，all 断言非真空");
    assert!(block.iter().all(|&sample| sample == 0.0), "被拒声部全零");
    assert_eq!(engine.cache_stats().entries, 0, "被拒文件不得入样本缓存");
}

/// §15.8 缺口二判据：立体声 FLAC 流出的交错 PCM 与整文件解码逐位相同
/// （流式解码腿不再折声道 0）。
#[test]
fn streamed_flac_stereo_matches_whole_file_interleaved() {
    let scratch = Scratch::new("stream_flac_stereo");
    let spec = FlacSpec {
        sample_rate: 48000,
        channels: 2,
        bits: 16,
        block_frames: 256,
        total_samples_override: None,
    };
    let bytes = flac_constant(&spec, 4, 16384);
    let path = scratch.join("tone2.flac");
    fs::write(&path, &bytes).expect("写 FLAC 立体声夹具");
    let engine = StreamEngine::new();
    let voice = engine.trigger(&path, StreamOptions::default());
    wait_ready(&voice, "FLAC 立体声声部");
    assert_eq!(voice.channels(), 2, "声道数必须发布为 2");
    let drained = drain_voice(&voice);
    let asset = decode_path(&path, &DecodeOptions::default()).expect("整文件解 FLAC 立体声");
    assert_eq!(asset.channels(), 2, "整文件解码必须报 2 声道");
    assert_bits_eq(&drained, asset.samples(), "流式 FLAC 立体声与整文件解码");
}
