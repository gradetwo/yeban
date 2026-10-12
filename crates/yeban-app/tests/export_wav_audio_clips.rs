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
/// 导出采样率（= 夹具素材采样率，走"不重采样"这条支路）。
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
// 判据 2（诚实臂）：采样率不匹配 ⇒ 跳过并报告，⛔ 不按错误音高播放
// ---------------------------------------------------------------------------

#[test]
fn a_rate_mismatched_asset_is_skipped_and_reported_not_mis_pitched() {
    let dir = scratch("rate");
    // 素材 44.1 kHz，导出默认 48 kHz：app 侧够不着重采样器 ⇒ 必须跳过并报告。
    let (bytes, _) = synth_asset(&dir, 44_100);
    let hash = AssetHash::of_bytes(&bytes);
    let project = project_with_audio_clip(hash, CLIP_GAIN_DB);
    let out = dir.join("mismatch.wav");
    let report = export_wav_with_sfz_file_and_assets(
        &project,
        &out,
        WavExportOptions::default(),
        None,
        &[(AssetHash::of_bytes(&bytes), bytes.clone())],
    )
    .expect("采样率不匹配只跳过那一段，⛔ 不让整次导出失败");

    println!(
        "[export-audio] 采样率不匹配：rendered={} skipped={:?}",
        report.audio_clips_rendered, report.audio_clips_skipped
    );
    assert_eq!(
        report.audio_clips_rendered, 0,
        "44.1 kHz 素材在 48 kHz 导出里必须被跳过"
    );
    assert!(
        report
            .audio_clips_skipped
            .iter()
            .any(|reason| reason.contains("采样率")),
        "跳过原因必须**明说采样率不匹配**: {:?}",
        report.audio_clips_skipped
    );

    let start = usize::try_from(frames_of(START_TICK)).expect("窗口起点");
    let end = usize::try_from(frames_of(START_TICK + DURATION_TICKS)).expect("窗口终点");
    let skipped_peak = peak_in(&left_channel_of(&out), start..end);
    assert!(
        skipped_peak <= 1e-4,
        "被跳过的素材在窗口内必须 ≈ 0（实测 {skipped_peak}）—— ⛔ 不许按错误音高播放"
    );

    // 反向对照：同一份**48 kHz** 字节必须真的发声（排除"一律跳过"的假绿）。
    let (matched_bytes, _) = synth_asset(&dir, EXPORT_RATE);
    let matched_project =
        project_with_audio_clip(AssetHash::of_bytes(&matched_bytes), CLIP_GAIN_DB);
    let matched_out = dir.join("matched.wav");
    let matched_report = export_wav_with_sfz_file_and_assets(
        &matched_project,
        &matched_out,
        WavExportOptions::default(),
        None,
        &[(AssetHash::of_bytes(&matched_bytes), matched_bytes.clone())],
    )
    .expect("48 kHz 素材必须导出成功");
    assert_eq!(
        matched_report.audio_clips_rendered, 1,
        "48 kHz 对照必须发声（否则本判据退化成恒绿）"
    );
    assert!(
        peak_in(&left_channel_of(&matched_out), start..end) > 0.05,
        "48 kHz 对照的窗口内必须可闻"
    );

    std::fs::remove_dir_all(&dir).ok();
}
