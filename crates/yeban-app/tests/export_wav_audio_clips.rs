//! 判据：`--export-wav` 必须**渲染**工程里摆放的音频片段（`ClipContent::Audio`）。
//!
//! ## 这条判据回答什么
//!
//! 产品里有一条**用户可触发**的入口：
//!
//! ```text
//! MCP 会话：yeban_import_audio（导入并摆放）⇒ yeban_save_project（写成 song.yeban）
//! 人：      yeban-app --open song.yeban --export-wav out.wav
//! ```
//!
//! 本文件走的是**生产入口** `yeban_app::cli::run_batch`（＝ `yeban-app` 二进制的批处理
//! 分发），而不是直接调导出函数：这样"容器里的资产池真的被读到"这件事也在覆盖内。
//!
//! ## 判定量（每个数字都要有单位）
//!
//! 1. `inside_peak`：导出 WAV 的**左声道**在摆放帧窗口 `[start, end)` 内的**峰值**（归一化
//!    到 `[-1, 1]`）；
//! 2. `expected_peak`：源样本（`SOURCE_INT16 / 32767`）× `db_to_linear(CLIP_GAIN_DB)`；
//! 3. `outside_peak`：窗口**之外**的峰值（抖动只有 ±1 LSB，量级 `1e-7`）。
//!
//! 断言 `inside_peak ≈ expected_peak` 且 `outside_peak ≈ 0` ⇒ "全程非零" 或 "窗口错位"
//! 都不能通过。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_app::cli::{self, run_batch};
use yeban_app::export_wav::{WavExportOptions, export_wav_with_sfz_file_and_assets};
use yeban_model::container::write_project_container;
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::render::db_to_linear;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav, write_plain_wav};

/// 夹具速度：120 BPM / 960 PPQ / 48 kHz ⇒ **1 tick = 25 帧**（可手算）。
const BPM: f64 = 120.0;
/// 权威时钟（`[MODEL-AST-001]`）。
const PPQ: u64 = 960;
/// 摆放起点：4800 tick = 0.25 s = **12000 帧**。
const START_TICK: u64 = 4_800;
/// 摆放时值：4800 tick = 0.25 s = **12000 帧**。
const DURATION_TICKS: u64 = 4_800;
/// 片段增益（dB）：-6.0206 ≈ ×0.5，便于判"增益真的被用了"。
const CLIP_GAIN_DB: f32 = -6.0;
/// 源素材帧数（20000 > 窗口的 12000 ⇒ 覆盖硬切的截断分支）。
const ASSET_FRAMES: usize = 20_000;
/// 源样本（16-bit 满量程的 1/4 ⇒ 归一化 ≈ 0.25）。
const SOURCE_INT16: i16 = 8_192;
/// 导出采样率（默认 48 kHz；48 k 素材走"同率、不重采样"支路）。
const EXPORT_RATE: u32 = 48_000;
/// 24-bit 整数的归一化分母（与 `read_plain_wav_mono_f32` 的 `BitDepth::Int24` 口径一致）。
const I24_SCALE: f32 = 8_388_608.0;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-export-audio-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// tick ⇒ 帧（四舍五入），与 `yeban_engine::synth::{samples_per_tick, tick_to_sample}` 同式。
fn frames_of(tick: u64) -> u64 {
    let samples_per_tick = 60.0 * f64::from(EXPORT_RATE) / (BPM * PPQ as f64);
    (tick as f64 * samples_per_tick).round() as u64
}

/// 合成一份**字节可寻址**的 PCM WAV 并返回 `(容器字节, 期望的源样本) `。
fn synth_asset(dir: &Path, rate: u32) -> (Vec<u8>, f32) {
    let path = dir.join(format!("take-{rate}.wav"));
    let samples: Vec<i16> = vec![SOURCE_INT16; ASSET_FRAMES];
    write_plain_wav(
        &path,
        &PcmFormat::integer(1, rate, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写夹具 WAV");
    (
        std::fs::read(&path).expect("读夹具 WAV"),
        f32::from(SOURCE_INT16) / 32_767.0,
    )
}

/// 合成一份 **32-bit float / 0.5 常量**素材（`#249` 的三端对照用）。
///
/// 0.5 在 f32 与 24-bit 整数量化下都可精确表示 ⇒ 引擎 / MCP / app 三端的
/// 左声道段峰值可以**直接比数值**，不需要各自再乘一次增益系数。
fn synth_asset_half(dir: &Path) -> Vec<u8> {
    let path = dir.join("half-48000.wav");
    let samples: Vec<f32> = vec![0.5; 24_000];
    write_plain_wav(
        &path,
        &PcmFormat::float(1, 48_000, 32),
        &PcmBuffer::Float32(samples),
    )
    .expect("写 0.5 常量浮点 WAV");
    std::fs::read(&path).expect("读 0.5 常量浮点 WAV")
}

/// 合成一份 **440 Hz 正弦**（16-bit / 单声道，振幅 0.25 ⇒ 峰值样本 ≈ 8192）。
///
/// 频率与采样率**解耦**：44.1 kHz 素材按 440/44100 的相位步进写盘，导出到 48 kHz 后
/// 若"只改标签"或"丢帧"，音高就会偏离 440 Hz ⇒ 零交叉读数可判（判据 §4②）。
fn synth_tone(dir: &Path, rate: u32, frames: usize) -> Vec<u8> {
    let path = dir.join(format!("tone-{rate}-{frames}.wav"));
    let samples: Vec<i16> = (0..frames)
        .map(|index| {
            let phase = core::f64::consts::TAU * 440.0 * (index as f64) / f64::from(rate);
            (0.25 * 32_767.0 * phase.sin()).round() as i16
        })
        .collect();
    write_plain_wav(
        &path,
        &PcmFormat::integer(1, rate, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写正弦夹具 WAV");
    std::fs::read(&path).expect("读正弦夹具 WAV")
}

/// 合成指定**声道数**的常量 WAV（用于"声道无法映射 ⇒ 响亮跳过"这条判据）。
fn synth_asset_channels(dir: &Path, name: &str, rate: u32, channels: u16) -> Vec<u8> {
    let path = dir.join(name);
    let frames = ASSET_FRAMES / 2;
    let samples: Vec<i16> = vec![SOURCE_INT16; frames * usize::from(channels)];
    write_plain_wav(
        &path,
        &PcmFormat::integer(channels, rate, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写多声道夹具 WAV");
    std::fs::read(&path).expect("读多声道夹具 WAV")
}

/// 判据窗口的**素材长度**：夹具正弦 / 常量 WAV 都是 1 秒 ⇒ 取摆放起点后 1 秒。
///
/// ⚠ 摆放时值（`DURATION_TICKS = 4800` × 25 帧/tick）实际是 **2.5 s**，比夹具长；
/// 判据只看"素材还活着"的那 1 秒（⛔ 不把静音尾巴算进非零读数）。
const WINDOW_FRAMES: usize = EXPORT_RATE as usize;

/// 清掉工程里**全部**摆放（判据 §4① 的反向腿：没有摆放 ⇒ 窗口内必须 ≈ 0）。
fn without_placements(mut project: YebanProjectV1) -> YebanProjectV1 {
    for track in project.tracks.values_mut() {
        track.clips.clear();
    }
    project
}

/// 零交叉估频：线性插值出每次过零的**小数帧位置**，用首末过零间隔算频率。
///
/// 返回 `(Hz, 过零次数, 首末过零间隔秒)`。对 440 Hz / 1 s 窗口，过零约 880 次、
/// 间隔约 0.998 s ⇒ 频率分辨率远小于 1 Hz（判据 §4② 的 `±1 Hz`）。两端各留 1000 帧
/// 以避开重采样器 `SINC_LEN / 2 = 128` 帧量级的过渡带。
fn zero_crossing_hz(samples: &[f32], rate: u32) -> (f64, usize, f64) {
    let mut times: Vec<f64> = Vec::new();
    for (index, pair) in samples.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        if (a < 0.0 && b >= 0.0) || (a > 0.0 && b <= 0.0) {
            let width = f64::from(b) - f64::from(a);
            let fraction = if width.abs() > 0.0 {
                -f64::from(a) / width
            } else {
                0.0
            };
            times.push(index as f64 + fraction);
        }
    }
    assert!(
        times.len() >= 2,
        "窗口内至少要有两个零交叉才能估频（实测 {} 个）",
        times.len()
    );
    let first = times[0];
    let last = times[times.len() - 1];
    let span = (last - first) / f64::from(rate);
    assert!(span > 0.0, "零交叉跨度必须为正（实测 {span} s）");
    let hz = (times.len() - 1) as f64 / (2.0 * span);
    (hz, times.len(), span)
}

/// 建一个**最小可渲染工程**：一条非 master 轨（无 MIDI 音符 ⇒ 引擎侧静音）＋
/// 一个音频片段摆放。
fn project_with_audio_clip(asset: AssetHash, gain_db: f32) -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    let clip = EntityId::new();
    let placement = EntityId::new();

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
        clip,
        ClipPoolEntry {
            id: clip,
            name: "take".to_owned(),
            content: ClipContent::Audio { asset, gain_db },
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
    project
        .tracks
        .get_mut(&track)
        .expect("轨在表里")
        .clips
        .insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: clip,
                start_tick: START_TICK,
                duration_ticks: DURATION_TICKS,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
    project
}

/// 把工程＋资产池写成真容器，返回容器路径。
fn write_container(dir: &Path, name: &str, project: &YebanProjectV1, bytes: &[u8]) -> PathBuf {
    let hash = AssetHash::of_bytes(bytes);
    let mut assets = BTreeMap::new();
    assets.insert(hash, bytes.to_vec());
    let container = write_project_container(project, b"", &assets).expect("写容器");
    let path = dir.join(name);
    std::fs::write(&path, &container).expect("落容器文件");
    path
}

/// 驱动生产入口 `run_batch(--headless --open <容器> --export-wav <目标>)`。
fn export_via_cli(container: &Path, out: &Path) -> Vec<String> {
    let options = cli::parse(&[
        "--headless".to_owned(),
        "--open".to_owned(),
        container.display().to_string(),
        "--export-wav".to_owned(),
        out.display().to_string(),
    ])
    .expect("命令行必须可解析");
    run_batch(&options).expect("生产导出入口必须成功")
}

/// 读回导出 WAV 的**左声道**（归一化到 `[-1, 1]`）。
fn left_channel_of(path: &Path) -> Vec<f32> {
    let (format, buffer) = read_plain_wav(path).expect("导出 WAV 必须可回读");
    let channels = usize::from(format.channels.max(1));
    let interleaved: Vec<f32> = match &buffer {
        PcmBuffer::Int24(values) => values.iter().map(|v| *v as f32 / I24_SCALE).collect(),
        other => panic!("默认导出必须是 24-bit 整数: {other:?}"),
    };
    interleaved.chunks(channels).map(|frame| frame[0]).collect()
}

fn peak_in(samples: &[f32], range: core::ops::Range<usize>) -> f32 {
    samples[range]
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()))
}

// ---------------------------------------------------------------------------
// 判据 1（运行时）：窗口内可闻且增益正确，窗口外 ≈ 0
// ---------------------------------------------------------------------------

#[test]
fn a_placed_audio_clip_is_audible_inside_its_frame_window_and_silent_outside() {
    let dir = scratch("audible");
    let (bytes, source_sample) = synth_asset(&dir, EXPORT_RATE);
    let hash = AssetHash::of_bytes(&bytes);
    let project = project_with_audio_clip(hash, CLIP_GAIN_DB);
    let container = write_container(&dir, "song.yeban", &project, &bytes);
    let out = dir.join("out.wav");
    let lines = export_via_cli(&container, &out);

    let exported = lines
        .iter()
        .find(|line| line.starts_with("exported-wav:"))
        .expect("必须打印 exported-wav 行");
    assert!(
        exported.contains("audio-clips(rendered=1"),
        "报告行必须如实登记渲染了 1 个音频摆放: {exported}"
    );

    let left = left_channel_of(&out);
    assert_eq!(left.len(), 48_000 * 8, "默认导出的帧数变了 ⇒ 本判据需同步");

    let start = usize::try_from(frames_of(START_TICK)).expect("窗口起点");
    let end = usize::try_from(frames_of(START_TICK + DURATION_TICKS)).expect("窗口终点");
    let in_window = peak_in(&left, start..end);
    let outside = peak_in(&left, 0..start).max(peak_in(&left, end..left.len()));
    let expected = source_sample * db_to_linear(CLIP_GAIN_DB);

    println!(
        "[export-audio] 窗口 [{start}, {end}) 内峰值 {in_window:.6}；期望 {expected:.6}；窗口外峰值 {outside:.8}；源样本 {source_sample:.6}"
    );

    assert!(
        in_window > 0.05,
        "摆放的音频片段必须可闻（窗口内峰值 = {in_window}）"
    );
    assert!(
        (in_window - expected).abs() <= 0.02 * expected,
        "窗口内峰值必须 ≈ 源样本 × db_to_linear(gain_db)：实测 {in_window}，期望 {expected}"
    );
    assert!(outside <= 1e-4, "窗口之外必须 ≈ 0（实测峰值 {outside}）");

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// 判据 2（`#141`）：44.1k 素材 ⇒ 离线导出**重采样后出现**，且音高仍是 440 Hz
// ---------------------------------------------------------------------------

#[test]
fn a_44k1_asset_is_resampled_into_the_export_with_the_right_pitch() {
    let dir = scratch("44k1");
    // 素材 44.1 kHz / 440 Hz；导出 48 kHz。旧行为是"跳过"（窗口内 0），现在是"重采样后出现"。
    let bytes = synth_tone(&dir, 44_100, 44_100);
    let hash = AssetHash::of_bytes(&bytes);
    let project = project_with_audio_clip(hash.clone(), CLIP_GAIN_DB);
    let out = dir.join("resampled.wav");
    let report = export_wav_with_sfz_file_and_assets(
        &project,
        &out,
        WavExportOptions::default(),
        None,
        &[(hash.clone(), bytes.clone())],
    )
    .expect("44.1k 素材在 48k 导出里必须导出成功（重采样，⛔ 不再跳过）");

    println!(
        "[export-audio/#141] 44.1k→48k：rendered={} skipped={:?} source={}",
        report.audio_clips_rendered, report.audio_clips_skipped, report.source
    );
    assert_eq!(
        report.audio_clips_rendered, 1,
        "44.1k 素材必须被重采样后渲染（旧断言是 0 ⇒ 跳过）"
    );
    assert!(
        report.audio_clips_skipped.is_empty(),
        "44.1k 不再进跳过报告: {:?}",
        report.audio_clips_skipped
    );
    assert!(
        report.source.contains("audio-clips(rendered=1 skipped=0)"),
        "报告行必须如实登记 rendered=1/skipped=0: {}",
        report.source
    );

    let left = left_channel_of(&out);
    assert_eq!(left.len(), 48_000 * 8, "默认导出的帧数变了 ⇒ 本判据需同步");
    let start = usize::try_from(frames_of(START_TICK)).expect("窗口起点");
    let end = start + WINDOW_FRAMES;
    let window = &left[start..end];
    let in_window = peak_in(&left, start..end);
    let expected = 0.25_f32 * db_to_linear(CLIP_GAIN_DB);
    let audible = window.iter().filter(|value| value.abs() > 0.01).count();
    // 两端各留 1000 帧：避开重采样器 ≈128 帧的过渡带。
    let (hz, crossings, span) = zero_crossing_hz(&window[1_000..window.len() - 1_000], EXPORT_RATE);
    println!(
        "[export-audio/#141] 窗口 [{start}, {end}) 非零(|x|>0.01)样本={audible}/{} 峰值={in_window:.6}\
         （源 440 Hz 振幅 0.25 × gain ⇒ 期望 {expected:.6}）；零交叉={crossings} 首末间隔={span:.6}s \
         ⇒ {hz:.4} Hz",
        window.len()
    );
    assert!(
        audible > window.len() / 2,
        "重采样后的 44.1k 素材必须在窗口内**大面积非零**（实测 {audible}/{}）",
        window.len()
    );
    assert!(
        in_window > 0.05,
        "重采样后的 44.1k 素材必须可闻（窗口内峰值 = {in_window}）"
    );
    assert!(
        (in_window - expected).abs() <= 0.02 * expected,
        "峰值必须 ≈ 源振幅 × 增益：实测 {in_window}，期望 {expected}"
    );
    assert!(
        (hz - 440.0).abs() <= 1.0,
        "音高必须仍是 440 Hz ±1（实测 {hz} Hz，零交叉 {crossings} 次）"
    );

    // ---- 反向腿：移除摆放 ⇒ 同一窗口必须 ≈ 0（非零只能来自那个摆放）----
    let silent_project = without_placements(project_with_audio_clip(hash.clone(), CLIP_GAIN_DB));
    let silent_out = dir.join("no-placement.wav");
    let silent_report = export_wav_with_sfz_file_and_assets(
        &silent_project,
        &silent_out,
        WavExportOptions::default(),
        None,
        &[(hash.clone(), bytes.clone())],
    )
    .expect("无摆放必须导出成功");
    let silent_peak = peak_in(&left_channel_of(&silent_out), start..end);
    println!(
        "[export-audio/#141] 反向腿（移除摆放）：rendered={} skipped={} 窗口峰值={silent_peak:.8}",
        silent_report.audio_clips_rendered,
        silent_report.audio_clips_skipped.len()
    );
    assert_eq!(
        silent_report.audio_clips_rendered, 0,
        "没有摆放 ⇒ rendered=0"
    );
    assert!(
        silent_report.audio_clips_skipped.is_empty(),
        "没有摆放 ⇒ 也不该有跳过原因"
    );
    assert!(
        silent_peak <= 1e-4,
        "移除摆放后窗口内必须 ≈ 0（实测 {silent_peak}）"
    );

    // ---- 同率对照：48k 素材 / 48k 输出 ⇒ 音高同样是 440 Hz ±1，且两次导出逐字节相同 ----
    let matched_bytes = synth_tone(&dir, EXPORT_RATE, EXPORT_RATE as usize);
    let matched_hash = AssetHash::of_bytes(&matched_bytes);
    let matched_project = project_with_audio_clip(matched_hash.clone(), CLIP_GAIN_DB);
    let matched_out = dir.join("same-rate.wav");
    let matched_report = export_wav_with_sfz_file_and_assets(
        &matched_project,
        &matched_out,
        WavExportOptions::default(),
        None,
        &[(matched_hash.clone(), matched_bytes.clone())],
    )
    .expect("48k 同率素材必须导出成功");
    assert_eq!(matched_report.audio_clips_rendered, 1);
    let matched_left = left_channel_of(&matched_out);
    let matched_peak = peak_in(&matched_left, start..end);
    let (matched_hz, ..) = zero_crossing_hz(&matched_left[start + 1_000..end - 1_000], EXPORT_RATE);
    let matched_sha = AssetHash::of_bytes(&std::fs::read(&matched_out).expect("读 48k 导出"));
    let matched_out_again = dir.join("same-rate-again.wav");
    export_wav_with_sfz_file_and_assets(
        &matched_project,
        &matched_out_again,
        WavExportOptions::default(),
        None,
        &[(matched_hash, matched_bytes)],
    )
    .expect("同率第二次导出必须成功");
    let matched_sha_again =
        AssetHash::of_bytes(&std::fs::read(&matched_out_again).expect("读第二次 48k 导出"));
    println!(
        "[export-audio/#141] 48k 同率对照：rendered=1 峰值={matched_peak:.6} 零交叉频率={matched_hz:.4} Hz；\
         导出 WAV sha256={matched_sha}（第二次 {matched_sha_again}）"
    );
    assert!(
        (matched_hz - 440.0).abs() <= 1.0,
        "48k 同率对照的音高必须也是 440 Hz ±1（实测 {matched_hz} Hz）"
    );
    assert!(matched_peak > 0.05, "48k 同率对照必须可闻");
    assert_eq!(
        matched_sha, matched_sha_again,
        "同率导出必须逐字节确定（dither seed 固定）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// 判据 3（诚实臂）：仍不能处理的形态 ⇒ **响亮跳过**（逐字原因 ＋ 报告计数）
// ---------------------------------------------------------------------------

/// 3 声道 → 立体声母线：即便采样率相同，声道布局也无法映射 ⇒ 必须跳过并**明说**。
#[test]
fn a_three_channel_asset_is_skipped_loudly_and_counted() {
    let dir = scratch("3ch");
    let bytes = synth_asset_channels(&dir, "take-3ch.wav", EXPORT_RATE, 3);
    let hash = AssetHash::of_bytes(&bytes);
    let project = project_with_audio_clip(hash.clone(), CLIP_GAIN_DB);
    let out = dir.join("three-channel.wav");
    let report = export_wav_with_sfz_file_and_assets(
        &project,
        &out,
        WavExportOptions::default(),
        None,
        &[(hash.clone(), bytes.clone())],
    )
    .expect("3 声道只跳过那一段，⛔ 不让整次导出失败");
    let reason = report
        .audio_clips_skipped
        .first()
        .expect("3 声道必须留下一条跳过原因");
    println!(
        "[export-audio/#141] 3 声道：rendered={} skipped={} reason={reason}",
        report.audio_clips_rendered,
        report.audio_clips_skipped.len()
    );
    assert_eq!(report.audio_clips_rendered, 0);
    assert_eq!(report.audio_clips_skipped.len(), 1);
    assert!(
        reason.contains("3 声道无法映射到 2 声道母线"),
        "原因必须逐字点名声道映射失败: {reason}"
    );
    assert!(
        reason.contains("不悄悄丢声道"),
        "原因必须逐字声明不悄悄丢声道: {reason}"
    );
    assert!(
        report.source.contains("audio-clips(rendered=0 skipped=1)"),
        "报告行必须明说少了一段（rendered=0 skipped=1）: {}",
        report.source
    );
    let start = usize::try_from(frames_of(START_TICK)).expect("窗口起点");
    let end = start + WINDOW_FRAMES;
    let skipped_peak = peak_in(&left_channel_of(&out), start..end);
    assert!(
        skipped_peak <= 1e-4,
        "被跳过的 3 声道素材必须 ≈ 0（实测 {skipped_peak}）—— ⛔ 不许按错误布局播放"
    );

    // 生产入口的 `exported-wav:` 行同样带这个计数（无桌面会话可判）。
    let container = write_container(&dir, "song-3ch.yeban", &project, &bytes);
    let cli_out = dir.join("three-channel-cli.wav");
    let lines = export_via_cli(&container, &cli_out);
    let exported = lines
        .iter()
        .find(|line| line.starts_with("exported-wav:"))
        .expect("必须打印 exported-wav 行");
    assert!(
        exported.contains("audio-clips(rendered=0 skipped=1)"),
        "CLI 报告行必须明说 skipped=1: {exported}"
    );
    println!("[export-audio/#141] 3 声道 CLI 报告行：{exported}");

    std::fs::remove_dir_all(&dir).ok();
}

/// 坏 WAV：解码失败也必须**响亮跳过**（逐字原因 ＋ 报告计数），⛔ 不静默少一段。
#[test]
fn a_corrupt_wav_is_skipped_loudly_and_counted() {
    let dir = scratch("corrupt");
    let bytes = b"this is not a RIFF/WAVE file".to_vec();
    let hash = AssetHash::of_bytes(&bytes);
    let project = project_with_audio_clip(hash.clone(), CLIP_GAIN_DB);
    let out = dir.join("corrupt.wav");
    let report = export_wav_with_sfz_file_and_assets(
        &project,
        &out,
        WavExportOptions::default(),
        None,
        &[(hash, bytes)],
    )
    .expect("坏 WAV 只跳过那一段，⛔ 不让整次导出失败");
    let reason = report
        .audio_clips_skipped
        .first()
        .expect("坏 WAV 必须留下一条跳过原因");
    println!(
        "[export-audio/#141] 坏 WAV：rendered={} skipped={} reason={reason}",
        report.audio_clips_rendered,
        report.audio_clips_skipped.len()
    );
    assert_eq!(report.audio_clips_rendered, 0);
    assert_eq!(report.audio_clips_skipped.len(), 1);
    assert!(
        reason.contains("读临时 WAV 失败"),
        "原因必须逐字点名 WAV 解码失败: {reason}"
    );
    assert!(
        report.source.contains("audio-clips(rendered=0 skipped=1)"),
        "报告行必须明说少了一段（rendered=0 skipped=1）: {}",
        report.source
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// 判据 7（`#249` / `[ADR-0010 D4]`）：音频片段按遍展开（离线导出真的循环）
// ---------------------------------------------------------------------------

/// 把夹具工程的唯一摆放改成"起点 0 / 时值 1920 / 给定循环配置"。
fn project_with_audio_loop(
    asset: AssetHash,
    gain_db: f32,
    loop_config: LoopConfig,
) -> YebanProjectV1 {
    let mut project = project_with_audio_clip(asset, gain_db);
    for track in project.tracks.values_mut() {
        for placement in track.clips.values_mut() {
            placement.start_tick = 0;
            placement.duration_ticks = 1_920;
            placement.loop_config = loop_config;
        }
    }
    project
}

/// 导出一次并返回 `(WAV 字节的 SHA-256, 左声道样本, 命令行输出)`。
///
/// 用 `AssetHash::of_bytes`：它是**本仓唯一的 SHA-256 实现**（`yeban-model::ids`），
/// ⛔ 不在这里另写一份摘要。
fn export_audio_loop(
    dir: &Path,
    tag: &str,
    project: &YebanProjectV1,
    bytes: &[u8],
) -> (AssetHash, Vec<f32>, Vec<String>) {
    let container = write_container(dir, &format!("{tag}.yeban"), project, bytes);
    let out = dir.join(format!("{tag}.wav"));
    let lines = export_via_cli(&container, &out);
    let wav = std::fs::read(&out).expect("导出 WAV 必须存在");
    let sha = AssetHash::of_bytes(&wav);
    let left = left_channel_of(&out);
    (sha, left, lines)
}

/// `0..960 / 摆放 1920` 的音频片段在 app 离线导出里 ⇒ 两段峰值相等。
///
/// 素材：20 000 帧常量（约 0.4167 s）；循环区间 24 000 帧 ⇒ 每一遍只有前
/// 20 000 帧发声、后 4 000 帧是静音尾巴，但两遍的**峰值相同**。
/// "关掉循环"的同一工程第 2 段静音（＝修前口径的现场复现）。
///
/// 同时钉住两个口径：`rendered=1`（摆放数）＋ `spans=2`（span 数）。
#[test]
fn an_audio_loop_is_expanded_in_the_app_export() {
    let dir = scratch("audioloop");
    let bytes = synth_asset_half(&dir);
    let source_sample = 0.5_f32;
    let hash = AssetHash::of_bytes(&bytes);

    // 正腿：循环 0..960 / 摆放 1920 ⇒ N = 2。
    let looped = project_with_audio_loop(
        hash.clone(),
        0.0,
        LoopConfig {
            enabled: true,
            start_tick: 0,
            end_tick: 960,
        },
    );
    let (looped_sha, looped_left, looped_lines) =
        export_audio_loop(&dir, "looped", &looped, &bytes);
    let exported = looped_lines
        .iter()
        .find(|line| line.starts_with("exported-wav:"))
        .expect("必须打印 exported-wav 行");
    println!("[#249/app] 循环导出行（逐字）: {exported}");
    assert!(
        exported.contains("audio-clips(rendered=1"),
        "摆放数仍必须是 1: {exported}"
    );
    assert!(
        exported.contains("spans=2"),
        "循环展开出的 span 数必须是 2: {exported}"
    );
    let segment = 24_000usize;
    let peak_first = peak_in(&looped_left, 0..segment);
    let peak_second = peak_in(&looped_left, segment..2 * segment);
    let expected = source_sample * db_to_linear(0.0);
    let relative = (peak_first - peak_second).abs() / expected;
    println!(
        "[#249/app] 循环两段峰值（逐字）：第1段={peak_first:.9} 第2段={peak_second:.9} \
         期望={expected:.9} 相对差={relative:e} 文件 sha256={looped_sha}"
    );
    assert!(
        (peak_first - expected).abs() <= 2.0 / 8_388_608.0,
        "第 1 段峰值必须 ≈ 素材 × 增益（实测 {peak_first}，期望 {expected}）"
    );
    assert!(
        (peak_second - expected).abs() <= 2.0 / 8_388_608.0,
        "第 2 段峰值必须与第 1 段相同（实测 {peak_second}，期望 {expected}）"
    );
    assert!(
        relative <= 1e-6,
        "两段相对差必须 ≤ 1e-6（实测 {relative:e}）"
    );

    // 控制腿：同一夹具关掉循环 ⇒ 第 2 段静音（旧代码不读 loop_config ⇒ 行为相同）。
    let plain = project_with_audio_loop(hash, 0.0, LoopConfig::default());
    let (_plain_sha, plain_left, plain_lines) = export_audio_loop(&dir, "plain", &plain, &bytes);
    let plain_exported = plain_lines
        .iter()
        .find(|line| line.starts_with("exported-wav:"))
        .expect("必须打印 exported-wav 行");
    let plain_first = peak_in(&plain_left, 0..segment);
    let plain_second = peak_in(&plain_left, segment..2 * segment);
    println!(
        "[#249/app] 关掉循环（＝修前口径）两段峰值：{plain_first:.9} / {plain_second:.9}；\
         报告行={plain_exported}"
    );
    assert!(plain_exported.contains("spans=1"), "关掉循环 ⇒ span 数 1");
    assert!((plain_first - expected).abs() <= 2.0 / 8_388_608.0);
    assert!(
        plain_second <= 2.0 / 8_388_608.0,
        "关掉循环后第 2 段必须只剩抖动（实测 {plain_second}）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 正对照：不循环 / 恰好覆盖 / 越过末端 / 关掉 ⇒ 导出 **SHA-256 逐字节相同**。
#[test]
fn a_non_looping_app_export_is_byte_identical_under_every_non_wrapping_config() {
    let dir = scratch("audioloop-positive");
    let (bytes, _source_sample) = synth_asset(&dir, EXPORT_RATE);
    let hash = AssetHash::of_bytes(&bytes);

    let baseline = project_with_audio_loop(hash.clone(), CLIP_GAIN_DB, LoopConfig::default());
    let (baseline_sha, baseline_left, _) = export_audio_loop(&dir, "baseline", &baseline, &bytes);
    println!("[#249/app-positive] 基线（不循环）sha256={baseline_sha}");

    for (tag, loop_config) in [
        ("disabled", LoopConfig::default()),
        (
            "covering",
            LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: 1_920,
            },
        ),
        (
            "beyond",
            LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: 3_840,
            },
        ),
    ] {
        let project = project_with_audio_loop(hash.clone(), CLIP_GAIN_DB, loop_config);
        let (sha, left, lines) = export_audio_loop(&dir, tag, &project, &bytes);
        let exported = lines
            .iter()
            .find(|line| line.starts_with("exported-wav:"))
            .expect("必须打印 exported-wav 行");
        println!("[#249/app-positive] {tag}：sha256={sha}；{exported}");
        assert_eq!(sha, baseline_sha, "{tag} 的导出文件必须与不循环逐字节相同");
        assert_eq!(left, baseline_left, "{tag} 的左声道必须逐帧等于不循环");
        assert!(exported.contains("spans=1"), "{tag} 必须仍是单 span");
    }

    std::fs::remove_dir_all(&dir).ok();
}
