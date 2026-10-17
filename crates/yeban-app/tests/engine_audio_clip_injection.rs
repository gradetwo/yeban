//! 任务单 `#136`／`#138` 的判据：**控制侧把音频片段的 PCM 注入引擎快照** ⇒ 实时走带可闻；
//! **采样率不符的素材先在控制侧重采样到工程率**（`#138`）。
//!
//! # 判据与文件头的对应
//!
//! | 判据 | 内容 |
//! | :--- | :--- |
//! | [`the_product_path_plays_a_recorded_audio_clip_and_the_reverse_leg_is_bit_silent`] | **产品路径**（`ProductionLoop` → `EngineHost` → `EngineRuntime::process_quantum`）⇒ 非静音 ＋ 峰值；**反向腿**（不接字节来源）⇒ 逐位静音 ＋ 与"没有这条摆放"的工程**指纹相同** |
//! | [`every_revision_reinjects_and_each_asset_is_decoded_once`] | 同一素材两个摆放 ⇒ `decodes == 1`、`spans == 2`；改一次工程（加一条音符）再发布 ⇒ 新快照**仍有** PCM、`decodes` 仍 1 |
//! | [`a_44k1_asset_is_resampled_to_the_project_rate_before_injection`] | **44.1k → 48k 不再跳过**：注入 1／跳过 0／`frames == ceil(44100 × 48000 / 44100)`；产品路径出声；移除摆放 ⇒ 零；缓存不重采样 |
//! | [`a_44k1_sine_keeps_its_pitch_after_resampling`] | 44.1k 的 **440 Hz** 正弦 ⇒ 重采样后仍是 **440 Hz**（零交叉计数，容差 ±1 Hz） |
//! | [`a_48k_asset_is_not_resampled_and_stays_bit_identical`] | 同率 ⇒ `resampled == false`、帧数不变、字节逐位相同（单元级 `audio_inject` 判据 ＋ 报告读数） |
//! | [`degenerate_and_extreme_rate_assets_resample_without_panicking`] | 0 帧／单帧／8k→48k／192k→48k：帧数读数 ＋ 不 panic |
//! | [`the_resampled_path_still_refuses_tampered_bytes_by_sha256`] | 注入第 1 条：44.1k 重采样路① 换掉字节（哈希不变）⇒ SHA-256 拒绝，`resamples == 0` |
//! | [`the_resampled_output_follows_the_source_frequency`] | 注入第 2 条：44.1k 的 440／880 Hz ⇒ 实测 440／880 Hz、帧数相同、指纹不同（判据不是常数） |
//! | [`a_three_channel_asset_is_still_skipped_and_reported_verbatim`] | 边界：3 声道**仍然跳过**（本单只做采样率转换），原因逐字 ＝ 引擎文本 |
//! | [`pcm_comes_from_the_real_bytes_in_the_pool_not_a_synthetic_pcm`] | 资产池里换掉字节（哈希不变）⇒ 注入被**哈希校验**拒绝；用真字节 ⇒ 输出确实来自那段 WAV 的样本 |
//!
//! # 口径
//!
//! - 夹具用 `yeban_render::wav::write_plain_wav` 写**真 WAV 字节**（`#132`/`#134` 落进容器
//!   的那条路同一个写入器），⛔ 不手搓 `AudioPcm`；
//! - 实时侧只跑 `EngineRuntime::process_quantum`（无设备、无需桌面会话）；
//! - "非静音" = 输出样本里 `!= 0.0` 的**样本个数**与 `abs()` **峰值**（单位见断言文本）；
//! - 音高 = 左声道**零交叉个数** × 采样率 /（2 × 窗口帧数），窗口避开片段两端各 1000 帧。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use yeban_app::engine_host::{EditMark, EngineHost};
use yeban_app::host::ProductionLoop;
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote,
    RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

/// 夹具采样率（= 工程默认采样率）。
const RATE: u32 = 48_000;
/// 120 BPM / 960 PPQ / 48 kHz ⇒ **1 tick = 25 帧**（可手算）。
const BPM: f64 = 120.0;
/// 摆放时值：1920 tick = 1 s = 48000 帧。
const CLIP_TICKS: u64 = 1_920;
/// 源素材帧数（恰好覆盖一个摆放窗口）。
const ASSET_FRAMES: usize = 48_000;
/// 源样本（16-bit 满量程的 1/4 ⇒ 归一化 0.25；实数可手算）。
const SOURCE_INT16: i16 = 8_192;
/// 源素材的采样率（44.1 kHz —— `#138` 的换算臂）。
const RATE_44K1: u32 = 44_100;
/// 音高判据的正弦频率 (Hz)。
const SINE_HZ: f64 = 440.0;
/// 音高判据的渲染长度：1 s = 48000 帧 = 375 个运行时量子。
const PITCH_QUANTA: usize = 375;
/// 一个运行时量子的帧数（与 `EngineHost` 的驱动同一条常量）。
const QUANTA: usize = 24;

/// 写一份**真 WAV** 并读回字节（与 `--import-audio` / 录音落盘同一个写入器）。
fn wav_bytes(tag: &str, channels: u16, rate: u32, frames: usize) -> Vec<u8> {
    let samples: Vec<i16> = vec![SOURCE_INT16; frames * usize::from(channels)];
    wav_bytes_from(tag, channels, rate, &samples)
}

/// 用**给定的样本**写一份真 WAV 并读回字节（正弦／多声道／空素材都走这一条）。
fn wav_bytes_from(tag: &str, channels: u16, rate: u32, samples: &[i16]) -> Vec<u8> {
    let path = std::env::temp_dir().join(format!(
        "yeban-136-{tag}-{}-{}.wav",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    write_plain_wav(
        &path,
        &PcmFormat::integer(channels, rate, 16),
        &PcmBuffer::Int16(samples.to_vec()),
    )
    .expect("写夹具 WAV");
    let bytes = std::fs::read(&path).expect("读夹具 WAV");
    std::fs::remove_file(&path).expect("删夹具 WAV");
    bytes
}

/// 一份**440 Hz 正弦**的真 WAV（44.1 kHz）：音高判据的输入。
fn sine_wav_bytes(tag: &str, rate: u32, frames: usize, frequency: f64) -> Vec<u8> {
    let samples: Vec<i16> = (0..frames)
        .map(|frame| {
            let phase = std::f64::consts::TAU * frequency * frame as f64 / f64::from(rate);
            (phase.sin() * f64::from(SOURCE_INT16)).round() as i16
        })
        .collect();
    wav_bytes_from(tag, 1, rate, &samples)
}

/// 左声道在 `[start, end)` 帧内的**零交叉个数** ⇒ 频率估计 (Hz)。
///
/// `f = crossings × rate / (2 × window_frames)`；跨零点的样本被跳过（不把 0 当符号）。
fn zero_crossing_hz(samples: &[f32], channels: usize, rate: u32, start: usize, end: usize) -> f64 {
    let mut previous: Option<f32> = None;
    let mut crossings = 0_u64;
    for frame in start..end {
        let current = samples[frame * channels];
        if current == 0.0 {
            continue;
        }
        if previous.is_some_and(|prev| (prev > 0.0) != (current > 0.0)) {
            crossings += 1;
        }
        previous = Some(current);
    }
    let window = (end - start) as f64;
    crossings as f64 * f64::from(rate) / (2.0 * window)
}

/// 一个最小可播放工程：master ＋ 一条 MIDI 轨 ＋ 一个音频片段（可多个摆放）＋ 一个空 MIDI 片段。
///
/// 返回 `(project, midi_clip_id)` —— 后者给"改一次工程（加一条音符）"用。
fn project_with_audio(asset: &AssetHash, starts: &[u64]) -> (YebanProjectV1, EntityId) {
    let master = EntityId::new();
    let track = EntityId::new();
    let audio_clip = EntityId::new();
    let midi_clip = EntityId::new();

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
    clip_pool.insert(
        midi_clip,
        ClipPoolEntry {
            id: midi_clip,
            name: "midi".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::new(),
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
        },
    );

    let mut project = YebanProjectV1 {
        bpm: BPM,
        master_bus_track_id: master,
        tracks,
        routing_graph: graph,
        clip_pool,
        ..YebanProjectV1::default()
    };
    let track_clips = &mut project.tracks.get_mut(&track).expect("轨在表里").clips;
    for start_tick in starts {
        let placement_id = EntityId::new();
        track_clips.insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id: audio_clip,
                start_tick: *start_tick,
                duration_ticks: CLIP_TICKS,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
    }
    let midi_placement_id = EntityId::new();
    track_clips.insert(
        midi_placement_id,
        ClipPlacement {
            id: midi_placement_id,
            clip_id: midi_clip,
            start_tick: 0,
            duration_ticks: CLIP_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    (project, midi_clip)
}

/// 给工程里的空 MIDI 片段加**一条音符**（"改一次工程"的最小编辑）。
fn add_one_note(project: &mut YebanProjectV1, midi_clip: EntityId) {
    let note = MidiNote::new(EntityId::new(), 0, 60, 480);
    let ClipContent::Midi { notes } = &mut project
        .clip_pool
        .get_mut(&midi_clip)
        .expect("MIDI 片段在池里")
        .content
    else {
        panic!("夹具的片段必须是 MIDI");
    };
    notes.insert(note.id, note);
}

/// 判据用的"会话资产池"（与产品的 `UndoPort::asset_bytes` 同一个只读形状）。
fn pool_source(
    pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>>,
) -> yeban_app::audio_inject::AudioAssetSource {
    Box::new(move |hash| pool.borrow().get(hash).cloned())
}

/// 推 `quanta` 个量子并收集**交错**输出（左/右逐帧）。
fn render(host: &mut EngineHost, quanta: usize) -> Vec<f32> {
    let channels = host
        .current_snapshot()
        .map_or(2, |snapshot| snapshot.channels().max(1));
    let mut output = Vec::with_capacity(quanta * DEFAULT_BLOCK_FRAMES * usize::from(channels));
    let mut block = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * usize::from(channels)];
    {
        let runtime = host
            .runtime_for_criteria()
            .expect("刚 reload 过的宿主必须有控制线程可用的运行时");
        for _ in 0..quanta {
            block.fill(0.0);
            runtime.process_quantum(&mut block, channels);
            output.extend_from_slice(&block);
        }
    }
    output
}

/// 非零**样本**个数与 `abs()` 峰值（交错缓冲，两个声道都算）。
fn nonzero_and_peak(samples: &[f32]) -> (usize, f32) {
    let nonzero = samples.iter().filter(|sample| **sample != 0.0).count();
    let peak = samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));
    (nonzero, peak)
}

/// FNV-1a（64 位）对**样本位型**做指纹 ⇒ "逐位相同"可机械判 [ARCH-DET-001]。
fn fingerprint(samples: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

// ---------------------------------------------------------------------------
// 判据 2（产品路径能响）＋ 反向腿
// ---------------------------------------------------------------------------

#[test]
fn the_product_path_plays_a_recorded_audio_clip_and_the_reverse_leg_is_bit_silent() {
    let bytes = wav_bytes("audible", 1, RATE, ASSET_FRAMES);
    let hash = AssetHash::of_bytes(&bytes);
    let (project, _midi_clip) = project_with_audio(&hash, &[0]);

    // ---- 正腿：产品装配点（`ProductionLoop::start_with_audio_source`）真的注入 ----
    let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> = Rc::new(RefCell::new(BTreeMap::new()));
    pool.borrow_mut().insert(hash.clone(), bytes.clone());
    let production =
        ProductionLoop::start_with_audio_source(&project, 0, Some(pool_source(Rc::clone(&pool))))
            .expect("产品装配点必须建起一代引擎");
    let engine = production.engine_handle();
    let report = engine.borrow().audio_clip_report().clone();
    println!(
        "[#136/1] {} | {:?}",
        report.summary(),
        report
            .assets
            .iter()
            .map(|asset| (
                asset.frames,
                asset.channels,
                asset.sample_rate,
                asset.spans_attached
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(report.injected_assets, 1, "必须注入 1 个素材");
    assert_eq!(report.spans_attached, 1, "1 条摆放必须被接上 PCM");
    let injected = report.assets.first().expect("必须有逐素材读数");
    assert_eq!(injected.frames, ASSET_FRAMES as u64);
    assert_eq!(injected.channels, 1);
    assert_eq!(injected.sample_rate, RATE);
    assert_eq!(injected.source_sample_rate, RATE);
    assert_eq!(injected.source_frames, ASSET_FRAMES as u64);
    assert!(!injected.resampled, "48k 工程 + 48k 素材 ⇒ ⛔ 不重采样");
    assert_eq!(injected.spans_attached, 1);

    engine.borrow_mut().play();
    let (nonzero, peak) = {
        let mut host = engine.borrow_mut();
        let samples = render(&mut host, QUANTA);
        nonzero_and_peak(&samples)
    };
    println!("[#136/2] 产品路径: 非零样本 = {nonzero} / 峰值 = {peak:.6}");
    assert!(nonzero > 0, "产品路径必须出声（非零样本数 = {nonzero}）");
    assert!(peak > 0.05, "峰值必须可闻（实测 {peak}）");

    // ---- 反向腿：不接字节来源 ⇒ 逐位静音 ----
    let mut silent = EngineHost::new();
    silent.reload(&project, 0).expect("没有来源也必须能建快照");
    let silent_report = silent.audio_clip_report().clone();
    println!("[#136/2r] 反向腿: {}", silent_report.summary());
    assert_eq!(silent_report.injected_assets, 0);
    assert_eq!(silent_report.skipped.len(), 1);
    assert!(
        silent_report.skipped[0]
            .reason
            .contains("没有接上资产字节来源"),
        "跳过原因必须可读: {}",
        silent_report.skipped[0].reason
    );
    silent.play();
    let silent_samples = render(&mut silent, QUANTA);
    let (silent_nonzero, silent_peak) = nonzero_and_peak(&silent_samples);
    let silent_fingerprint = fingerprint(&silent_samples);
    assert_eq!(silent_nonzero, 0, "不注入 ⇒ 逐位静音");
    assert_eq!(silent_peak, 0.0);

    // ---- 反向腿对照：同一工程**没有这条音频摆放** ⇒ 指纹相同（"与接线前逐位相同"）----
    //
    // `project_with_audio(&hash, &[])` 没有音频摆放；再把那条**没有被摆放引用**的音频
    // 片段条目从池里去掉 ⇒ 快照的音频表是空的（对照的是"接线前"的形态）。
    let (mut without_clip, _) = project_with_audio(&hash, &[]);
    without_clip
        .clip_pool
        .retain(|_, entry| matches!(entry.content, ClipContent::Midi { .. }));
    let mut baseline = EngineHost::new();
    baseline
        .reload(&without_clip, 0)
        .expect("对照工程必须能建快照");
    baseline.play();
    let baseline_samples = render(&mut baseline, QUANTA);
    println!(
        "[#136/2c] 对照（无音频摆放）: 非零 = {} / 指纹 = {silent_fingerprint:#018x} == {:#018x}",
        baseline_samples
            .iter()
            .filter(|sample| **sample != 0.0)
            .count(),
        fingerprint(&baseline_samples)
    );
    assert_eq!(
        fingerprint(&baseline_samples),
        silent_fingerprint,
        "反向腿必须与'没有这条摆放'逐位相同"
    );
}

// ---------------------------------------------------------------------------
// 判据 3（每修订重注入）＋ 判据 5（同 asset 两个摆放只解码一次）
// ---------------------------------------------------------------------------

#[test]
fn every_revision_reinjects_and_each_asset_is_decoded_once() {
    let bytes = wav_bytes("cache", 1, RATE, ASSET_FRAMES);
    let hash = AssetHash::of_bytes(&bytes);
    // 同一素材、两个摆放（0 与 1920 tick）。
    let (project, midi_clip) = project_with_audio(&hash, &[0, CLIP_TICKS]);
    let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> = Rc::new(RefCell::new(BTreeMap::new()));
    pool.borrow_mut().insert(hash.clone(), bytes);

    let mut host = EngineHost::new();
    host.set_audio_asset_source(pool_source(Rc::clone(&pool)));
    host.reload(&project, 0).expect("第一代必须建起来");
    let first = host.audio_clip_report().clone();
    println!(
        "[#136/3-1] {} | spans={} decodes={} hits={}",
        first.summary(),
        first.spans_attached,
        first.cache.decodes,
        first.cache.hits
    );
    assert_eq!(first.injected_assets, 1);
    assert_eq!(first.spans_attached, 2, "同一素材的两个摆放都要接上");
    assert_eq!(first.cache.decodes, 1, "同一素材只解码一次");
    assert_eq!(first.cache.resamples, 0, "48k 同率 ⇒ ⛔ 不重采样");
    assert!(!first.assets.first().expect("逐素材读数").resampled);
    assert_eq!(
        host.current_snapshot().expect("有快照").audio_pcm_spans(),
        2
    );

    // 改一次工程（加一条音符）⇒ 增量发布（与 GUI 的编辑路径同一个函数）。
    let mut edited = project.clone();
    add_one_note(&mut edited, midi_clip);
    host.publish_project(&edited, EditMark::new(None, 1))
        .expect("编辑发布必须成功");
    let second = host.audio_clip_report().clone();
    let snapshot = host.current_snapshot().expect("有快照");
    println!(
        "[#136/3-2] {} | 新快照 audio_pcm_spans={} frames={}",
        second.summary(),
        snapshot.audio_pcm_spans(),
        second.assets.first().map_or(0, |asset| asset.frames)
    );
    assert_eq!(second.revision, 2, "第二个修订");
    assert_eq!(second.injected_assets, 1, "新修订**仍然**注入");
    assert_eq!(second.spans_attached, 2);
    assert_eq!(second.cache.decodes, 1, "缓存命中 ⇒ 不重解码");
    assert!(second.cache.hits >= 1);
    assert_eq!(
        snapshot.audio_pcm_spans(),
        2,
        "新快照里必须有 PCM（⛔ 不是'有摆放但静音'）"
    );
    assert_eq!(
        second.assets.first().map(|asset| asset.frames),
        Some(ASSET_FRAMES as u64),
        "第二个快照的采样数（frames）"
    );
}

// ---------------------------------------------------------------------------
// 判据 ①④（44.1k → 48k 不再跳过；产品路径能响 ＋ 反向腿）
// ---------------------------------------------------------------------------

#[test]
fn a_44k1_asset_is_resampled_to_the_project_rate_before_injection() {
    // 素材 44.1 kHz（1 s = 44100 帧）；工程 48 kHz（`YebanProjectV1::default`）。
    let bytes = wav_bytes("44k1-to-48k", 1, RATE_44K1, RATE_44K1 as usize);
    let hash = AssetHash::of_bytes(&bytes);
    let (project, _midi_clip) = project_with_audio(&hash, &[0]);
    assert_eq!(project.audio_config.sample_rate.hz(), RATE);

    let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> = Rc::new(RefCell::new(BTreeMap::new()));
    pool.borrow_mut().insert(hash.clone(), bytes);
    let mut host = EngineHost::new();
    host.set_audio_asset_source(pool_source(Rc::clone(&pool)));
    host.reload(&project, 0).expect("投影本身必须成功");

    let report = host.audio_clip_report().clone();
    println!("[#138/1] {}", report.summary());
    for skip in &report.skipped {
        println!("[#138/1] 跳过 asset {}：{}", skip.asset, skip.reason);
    }
    assert_eq!(
        report.injected_assets, 1,
        "44.1k 素材必须注入（⛔ 不再整条跳过）"
    );
    assert_eq!(report.resampled_assets, 1, "必须记下这次重采样");
    assert!(
        report.skipped.is_empty(),
        "44.1k → 48k 必须不再跳过: {:?}",
        report.skipped
    );
    let injected = report.assets.first().expect("必须有逐素材读数");
    let source_frames = u64::from(RATE_44K1);
    let expected_frames = u128::from(source_frames) * u128::from(RATE) / u128::from(RATE_44K1);
    let expected_frames = u64::try_from(expected_frames).expect("帧数在 u64 内");
    println!(
        "[#138/1] 注入 frames={} = ceil({} × {}/{}) = {}；source={} Hz/{} 帧；resampled={}",
        injected.frames,
        source_frames,
        RATE,
        RATE_44K1,
        expected_frames,
        injected.source_sample_rate,
        injected.source_frames,
        injected.resampled
    );
    assert_eq!(
        injected.frames, expected_frames,
        "新帧数必须逐字等于 ceil(原帧数 × 48000/44100)"
    );
    assert_eq!(expected_frames, 48_000);
    assert_eq!(injected.source_frames, source_frames);
    assert_eq!(injected.source_sample_rate, RATE_44K1);
    assert_eq!(injected.sample_rate, RATE);
    assert!(injected.resampled);
    assert_eq!(injected.channels, 1);
    assert_eq!(injected.spans_attached, 1);
    assert_eq!(report.cache.resamples, 1, "每个素材每目标率只重采样一次");
    assert_eq!(
        host.current_snapshot().expect("有快照").audio_pcm_spans(),
        1
    );

    host.play();
    let (nonzero, peak) = nonzero_and_peak(&render(&mut host, QUANTA));
    println!("[#138/4] 44.1k 素材 + 48k 工程: 非零样本 = {nonzero} / 峰值 = {peak:.6}");
    assert!(nonzero > 0, "44.1k 素材必须出声（非零样本数 = {nonzero}）");
    assert!(peak > 0.05, "峰值必须可闻（实测 {peak}）");

    // ---- 反向腿：同一素材、**移除摆放** ⇒ 逐位静音 ----
    let (without_placement, _) = project_with_audio(&hash, &[]);
    let mut silent = EngineHost::new();
    silent.set_audio_asset_source(pool_source(Rc::clone(&pool)));
    silent
        .reload(&without_placement, 0)
        .expect("无摆放也必须能建快照");
    let silent_report = silent.audio_clip_report().clone();
    println!("[#138/4r] 反向腿: {}", silent_report.summary());
    assert_eq!(silent_report.injected_assets, 1, "素材仍在池里 ⇒ 仍注入");
    assert_eq!(
        silent_report.spans_attached, 0,
        "没有摆放 ⇒ 没有可发声的 span"
    );
    silent.play();
    let silent_samples = render(&mut silent, QUANTA);
    let (silent_nonzero, silent_peak) = nonzero_and_peak(&silent_samples);
    println!("[#138/4r] 输出: 非零 = {silent_nonzero} / 峰值 = {silent_peak}");
    assert_eq!(silent_nonzero, 0, "移除摆放 ⇒ 逐位静音");
    assert_eq!(silent_peak, 0.0);

    // ---- 缓存：第二个修订**不重解码、不重采样** ----
    host.publish_project(&project, EditMark::new(None, 1))
        .expect("发布必须成功");
    let second = host.audio_clip_report().clone();
    println!("[#138/1c] {}", second.summary());
    assert_eq!(second.cache.decodes, 1, "缓存命中 ⇒ 不重解码");
    assert_eq!(second.cache.resamples, 1, "缓存命中 ⇒ 不重采样");
    assert!(second.assets.first().is_some_and(|asset| asset.from_cache));
}

// ---------------------------------------------------------------------------
// 判据 ③（48k 同率 ⇒ 不重采样）
// ---------------------------------------------------------------------------

#[test]
fn a_48k_asset_is_not_resampled_and_stays_bit_identical() {
    let bytes = wav_bytes("same-rate", 1, RATE, ASSET_FRAMES);
    let hash = AssetHash::of_bytes(&bytes);
    let (project, _midi_clip) = project_with_audio(&hash, &[0]);
    let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> = Rc::new(RefCell::new(BTreeMap::new()));
    pool.borrow_mut().insert(hash.clone(), bytes);
    let mut host = EngineHost::new();
    host.set_audio_asset_source(pool_source(Rc::clone(&pool)));
    host.reload(&project, 0).expect("投影必须成功");

    let report = host.audio_clip_report().clone();
    let injected = report.assets.first().expect("必须注入");
    println!(
        "[#138/3] {} | frames={} source_frames={} sample_rate={} source_sample_rate={} resampled={} resamples={}",
        report.summary(),
        injected.frames,
        injected.source_frames,
        injected.sample_rate,
        injected.source_sample_rate,
        injected.resampled,
        report.cache.resamples
    );
    assert!(!injected.resampled, "同率 ⇒ ⛔ 不重采样");
    assert_eq!(injected.frames, ASSET_FRAMES as u64, "帧数必须不变");
    assert_eq!(injected.source_frames, injected.frames);
    assert_eq!(injected.source_sample_rate, RATE);
    assert_eq!(injected.sample_rate, RATE);
    assert_eq!(report.cache.resamples, 0);

    // 逐位相同：`audio_inject::tests::a_same_rate_lookup_returns_the_decoded_samples_bit_for_bit`
    // 直接比对「解码样本」与「注入 PCM」的**位模式**（这里只给报告读数，并确认能出声）。
    host.play();
    let (nonzero, peak) = nonzero_and_peak(&render(&mut host, QUANTA));
    println!("[#138/3] 输出: 非零样本 = {nonzero} / 峰值 = {peak:.6}");
    assert!(nonzero > 0);
}

// ---------------------------------------------------------------------------
// 边界：>2 声道**仍然跳过**（本单只做采样率转换 —— 跳过必须可见）
// ---------------------------------------------------------------------------

#[test]
fn a_three_channel_asset_is_still_skipped_and_reported_verbatim() {
    let bytes = wav_bytes("three-channels", 3, RATE, 100);
    let hash = AssetHash::of_bytes(&bytes);
    let (project, _midi_clip) = project_with_audio(&hash, &[0]);
    let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> = Rc::new(RefCell::new(BTreeMap::new()));
    pool.borrow_mut().insert(hash.clone(), bytes);
    let mut host = EngineHost::new();
    host.set_audio_asset_source(pool_source(Rc::clone(&pool)));
    host.reload(&project, 0).expect("投影必须成功");
    let report = host.audio_clip_report().clone();
    println!("[#138/7] {}", report.summary());
    for skip in &report.skipped {
        println!("[#138/7] 跳过 asset {}：{}", skip.asset, skip.reason);
    }
    assert_eq!(
        report.injected_assets, 0,
        ">2 声道 ⇒ ⛔ 不注入（本单不做声道转换）"
    );
    assert_eq!(report.skipped.len(), 1, "必须有一条可读的跳过登记");
    assert_eq!(
        report.skipped[0].reason, "audio PCM channel count 3 is neither 1 (mono) nor 2 (stereo)",
        "跳过原因必须是引擎的逐字文本（⛔ 不静默）"
    );
    assert_eq!(
        host.current_snapshot().expect("有快照").audio_pcm_spans(),
        0
    );
    host.play();
    let (nonzero, _) = nonzero_and_peak(&render(&mut host, QUANTA));
    assert_eq!(nonzero, 0, "被跳过的素材 ⇒ 静音");
}

// ---------------------------------------------------------------------------
// 判据 ②（音高正确：440 Hz 仍是 440 Hz）
// ---------------------------------------------------------------------------

#[test]
fn a_44k1_sine_keeps_its_pitch_after_resampling() {
    let bytes = sine_wav_bytes("pitch-440", RATE_44K1, RATE_44K1 as usize, SINE_HZ);
    let hash = AssetHash::of_bytes(&bytes);
    let (project, _midi_clip) = project_with_audio(&hash, &[0]);
    let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> = Rc::new(RefCell::new(BTreeMap::new()));
    pool.borrow_mut().insert(hash.clone(), bytes);
    let mut host = EngineHost::new();
    host.set_audio_asset_source(pool_source(Rc::clone(&pool)));
    host.reload(&project, 0).expect("投影必须成功");

    let report = host.audio_clip_report().clone();
    let injected = report.assets.first().expect("必须注入");
    assert!(injected.resampled);
    assert_eq!(injected.frames, RATE as u64);

    host.play();
    let channels = usize::from(
        host.current_snapshot()
            .map_or(2, |snapshot| snapshot.channels().max(1)),
    );
    let samples = render(&mut host, PITCH_QUANTA);
    assert_eq!(
        samples.len(),
        PITCH_QUANTA * DEFAULT_BLOCK_FRAMES * channels,
        "渲染长度（帧 × 声道）"
    );
    let window_start = 1_000;
    let window_end = 47_000;
    let hz = zero_crossing_hz(&samples, channels, RATE, window_start, window_end);
    let crossings = (hz * 2.0 * (window_end - window_start) as f64 / f64::from(RATE)).round();
    println!(
        "[#138/2] 44.1k {SINE_HZ} Hz 正弦 ⇒ 重采样后实测 {hz:.3} Hz\
         （零交叉 {crossings:.0} 次 / 窗口 {}..{} 帧 / 容差 ±1 Hz）",
        window_start, window_end
    );
    assert!(
        (hz - SINE_HZ).abs() <= 1.0,
        "440 Hz 必须仍是 440 Hz（实测 {hz:.3} Hz）"
    );
}

// ---------------------------------------------------------------------------
// 判据 ⑤（0 帧 / 单帧 / 极端比例）
// ---------------------------------------------------------------------------

#[test]
fn degenerate_and_extreme_rate_assets_resample_without_panicking() {
    let cases: [(&str, u32, usize); 4] = [
        ("zero-frames", RATE_44K1, 0),
        ("one-frame", RATE_44K1, 1),
        ("8k-to-48k", 8_000, 800),
        ("192k-to-48k", 192_000, 19_200),
    ];
    for (tag, rate, frames) in cases {
        let bytes = wav_bytes(tag, 1, rate, frames);
        let hash = AssetHash::of_bytes(&bytes);
        let (project, _midi_clip) = project_with_audio(&hash, &[0]);
        let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> =
            Rc::new(RefCell::new(BTreeMap::new()));
        pool.borrow_mut().insert(hash.clone(), bytes);
        let mut host = EngineHost::new();
        host.set_audio_asset_source(pool_source(Rc::clone(&pool)));
        host.reload(&project, 0).expect("投影必须成功");
        let report = host.audio_clip_report().clone();
        let expected = if frames == 0 {
            0_u64
        } else {
            u64::try_from(
                u128::try_from(frames)
                    .expect("帧数在 u128 内")
                    .saturating_mul(u128::from(RATE))
                    .div_ceil(u128::from(rate)),
            )
            .expect("帧数在 u64 内")
        };
        let injected = report
            .assets
            .first()
            .unwrap_or_else(|| panic!("{tag}: 必须注入（跳过原因 {:?}）", report.skipped));
        println!(
            "[#138/5] {tag}: {rate} Hz / {frames} 帧 ⇒ {} 帧（期望 {expected}）；resampled={}；跳过={}",
            injected.frames,
            injected.resampled,
            report.skipped.len()
        );
        assert_eq!(report.injected_assets, 1, "{tag}: 必须注入 1 个");
        assert!(
            report.skipped.is_empty(),
            "{tag}: 不许跳过: {:?}",
            report.skipped
        );
        assert_eq!(
            injected.frames, expected,
            "{tag}: 帧数必须 = ceil(原帧数 × 比例)"
        );
        assert_eq!(injected.source_frames, frames as u64, "{tag}: 源帧数");
        // 推一个量子 ⇒ 实时路径不 panic（0 帧的 PCM 由引擎整段跳过）。
        host.play();
        let _ = render(&mut host, 1);
    }
}

// ---------------------------------------------------------------------------
// ⭐ 注入第 1 条（另加）：重采样路径仍吃**真字节**（哈希门 ⛔ 不被绕过）
// ---------------------------------------------------------------------------

#[test]
fn the_resampled_path_still_refuses_tampered_bytes_by_sha256() {
    let bytes = sine_wav_bytes("resample-real", RATE_44K1, RATE_44K1 as usize, SINE_HZ);
    let hash = AssetHash::of_bytes(&bytes);
    let (project, _midi_clip) = project_with_audio(&hash, &[0]);

    // 池里的字节被换掉（声明哈希不变）⇒ 哈希校验必须拒绝（重采样路⛔ 不绕过它）。
    let mut tampered = bytes.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    let tampered_pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> =
        Rc::new(RefCell::new(BTreeMap::new()));
    tampered_pool.borrow_mut().insert(hash.clone(), tampered);
    let mut rejected = EngineHost::new();
    rejected.set_audio_asset_source(pool_source(Rc::clone(&tampered_pool)));
    rejected.reload(&project, 0).expect("投影必须成功");
    let rejected_report = rejected.audio_clip_report().clone();
    println!("[#138/6a] {}", rejected_report.summary());
    assert_eq!(rejected_report.injected_assets, 0);
    assert_eq!(rejected_report.skipped.len(), 1);
    assert!(
        rejected_report.skipped[0].reason.contains("SHA-256 不符"),
        "必须因哈希不符拒绝: {}",
        rejected_report.skipped[0].reason
    );
    assert_eq!(
        rejected_report.cache.resamples, 0,
        "哈希门排在重采样之前 ⇒ 根本没进重采样"
    );
}

// ---------------------------------------------------------------------------
// ⭐ 注入第 2 条（另加）：输出随**源字节**变（440／880 Hz ⇒ 音高与指纹都不同）
// ---------------------------------------------------------------------------

#[test]
fn the_resampled_output_follows_the_source_frequency() {
    let bytes_440 = sine_wav_bytes("resample-real-440", RATE_44K1, RATE_44K1 as usize, SINE_HZ);
    let bytes_880 = sine_wav_bytes(
        "resample-real-880",
        RATE_44K1,
        RATE_44K1 as usize,
        SINE_HZ * 2.0,
    );
    let hash_440 = AssetHash::of_bytes(&bytes_440);
    let hash_880 = AssetHash::of_bytes(&bytes_880);
    let (project_440, _) = project_with_audio(&hash_440, &[0]);
    let (project_880, _) = project_with_audio(&hash_880, &[0]);
    let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> = Rc::new(RefCell::new(BTreeMap::new()));
    pool.borrow_mut().insert(hash_440, bytes_440);
    pool.borrow_mut().insert(hash_880, bytes_880);

    let mut host_440 = EngineHost::new();
    host_440.set_audio_asset_source(pool_source(Rc::clone(&pool)));
    host_440.reload(&project_440, 0).expect("投影必须成功");
    host_440.play();
    let samples_440 = render(&mut host_440, PITCH_QUANTA);

    let mut host_880 = EngineHost::new();
    host_880.set_audio_asset_source(pool_source(Rc::clone(&pool)));
    host_880.reload(&project_880, 0).expect("投影必须成功");
    host_880.play();
    let samples_880 = render(&mut host_880, PITCH_QUANTA);

    let report_440 = host_440.audio_clip_report().clone();
    let report_880 = host_880.audio_clip_report().clone();
    let channels = usize::from(
        host_440
            .current_snapshot()
            .map_or(2, |snapshot| snapshot.channels().max(1)),
    );
    let hz_440 = zero_crossing_hz(&samples_440, channels, RATE, 1_000, 47_000);
    let hz_880 = zero_crossing_hz(&samples_880, channels, RATE, 1_000, 47_000);
    println!(
        "[#138/6b] 440 Hz ⇒ {hz_440:.3} Hz；880 Hz ⇒ {hz_880:.3} Hz；\
         指纹 {:#018x} / {:#018x}；frames {} / {}",
        fingerprint(&samples_440),
        fingerprint(&samples_880),
        report_440.assets[0].frames,
        report_880.assets[0].frames
    );
    assert!(
        (hz_440 - SINE_HZ).abs() <= 1.0,
        "440 Hz 必须仍是 440 Hz（实测 {hz_440:.3}）"
    );
    assert!(
        (hz_880 - SINE_HZ * 2.0).abs() <= 1.0,
        "880 Hz 必须仍是 880 Hz（实测 {hz_880:.3}）"
    );
    assert_ne!(
        fingerprint(&samples_440),
        fingerprint(&samples_880),
        "源频率变了，输出指纹必须变（判据不是常数）"
    );
    assert_eq!(report_440.assets[0].frames, report_880.assets[0].frames);
    assert!(report_440.assets[0].resampled && report_880.assets[0].resampled);
}

// ---------------------------------------------------------------------------
// ⭐ 注入的两条（另加）：① 真字节来源可证；② 指纹会随字节变（判据有牙）
// ---------------------------------------------------------------------------

#[test]
fn pcm_comes_from_the_real_bytes_in_the_pool_not_a_synthetic_pcm() {
    let bytes = wav_bytes("real-bytes", 1, RATE, ASSET_FRAMES);
    let hash = AssetHash::of_bytes(&bytes);
    let (project, _midi_clip) = project_with_audio(&hash, &[0]);

    // ① 池里的字节被换掉（声明哈希不变）⇒ 哈希校验必须拒绝（⛔ 不信"标签"）。
    let mut tampered = bytes.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    let tampered_pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> =
        Rc::new(RefCell::new(BTreeMap::new()));
    tampered_pool.borrow_mut().insert(hash.clone(), tampered);
    let mut rejected = EngineHost::new();
    rejected.set_audio_asset_source(pool_source(Rc::clone(&tampered_pool)));
    rejected.reload(&project, 0).expect("投影必须成功");
    let report = rejected.audio_clip_report().clone();
    println!("[#136/5a] {}", report.summary());
    assert_eq!(report.injected_assets, 0);
    assert_eq!(report.skipped.len(), 1);
    assert!(
        report.skipped[0].reason.contains("SHA-256 不符"),
        "必须因哈希不符拒绝: {}",
        report.skipped[0].reason
    );

    // ② 真字节 ⇒ 出声；把**源样本**换成不同值（新哈希）⇒ 指纹必须不同。
    let real_pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> =
        Rc::new(RefCell::new(BTreeMap::new()));
    real_pool.borrow_mut().insert(hash.clone(), bytes);
    let mut first = EngineHost::new();
    first.set_audio_asset_source(pool_source(Rc::clone(&real_pool)));
    first.reload(&project, 0).expect("投影必须成功");
    first.play();
    let first_samples = render(&mut first, QUANTA);
    assert!(first_samples.iter().any(|sample| *sample != 0.0));

    // 半量程 ⇒ 峰值应约为一半（同一路由/声相/限制器，只改源样本）。
    let half_path = std::env::temp_dir().join(format!(
        "yeban-136-half-{}-{}.wav",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    write_plain_wav(
        &half_path,
        &PcmFormat::integer(1, RATE, 16),
        &PcmBuffer::Int16(vec![SOURCE_INT16 / 2; ASSET_FRAMES]),
    )
    .expect("写半量程夹具");
    let half_bytes = std::fs::read(&half_path).expect("读半量程夹具");
    std::fs::remove_file(&half_path).expect("删半量程夹具");
    let half_hash = AssetHash::of_bytes(&half_bytes);
    let (half_project, _) = project_with_audio(&half_hash, &[0]);
    let half_pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> =
        Rc::new(RefCell::new(BTreeMap::new()));
    half_pool.borrow_mut().insert(half_hash, half_bytes);
    let mut half = EngineHost::new();
    half.set_audio_asset_source(pool_source(Rc::clone(&half_pool)));
    half.reload(&half_project, 0).expect("投影必须成功");
    half.play();
    let half_samples = render(&mut half, QUANTA);
    let (_, first_peak) = nonzero_and_peak(&first_samples);
    let (_, half_peak) = nonzero_and_peak(&half_samples);
    println!(
        "[#136/5b] 满量程峰值 = {first_peak:.6} / 半量程峰值 = {half_peak:.6} / 指纹 {:#018x} vs {:#018x}",
        fingerprint(&first_samples),
        fingerprint(&half_samples)
    );
    assert_ne!(
        fingerprint(&first_samples),
        fingerprint(&half_samples),
        "源样本变了，输出指纹必须变（判据不是常数）"
    );
    assert!(
        (first_peak / half_peak - 2.0).abs() < 0.05,
        "半量程的峰值应约为满量程的一半：{first_peak} / {half_peak}"
    );
}
