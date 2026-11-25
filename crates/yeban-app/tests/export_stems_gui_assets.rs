//! T-FIX-188：GUI「导出分轨」必须把会话资产池传给后端。
//!
//! 修前：`host.rs` 的 `export-stems` 臂调 `export_stems(..., None, &[])` ——
//! 内嵌样本（`assets/{sha256}`）的轨拿不到字节，渲染静音（`render::resolve_asset`：
//! 索引有、池里无 ⇒ 登记 + 静音），而 CLI（`&loaded.archive.assets`）与 TUI
//! （`archive.assets.clone()`）同一份工程却正常。
//!
//! 修法：臂里从 `UndoPort::assets()` 取池（与保存同一来源，装配点 `main.rs` 的
//! `--open` / `open_archive_runtime` 已把归档资产换进这只端口），传给后端。
//!
//! ## 判定量（先说指标，再跑命令）
//!
//! 每个数都是**分轨 WAV 单声道峰值**（归一化到 `[-1, 1]`，单位：无量纲振幅）：
//! - `inside_peak`：摆放窗口 `[start_frame, start_frame + ASSET_FRAMES)` 内的峰值。
//!   素材是 0.25 常量、增益 0 dB ⇒ 期望 ≈ 0.25，断言 `0.05 < inside_peak < 0.5`；
//! - `outside_peak`：窗口前与素材耗尽后（仍在摆放内）的峰值，断言 `< 1e-3`。
//!   静音轨的读数是精确的 0，`&[]` 回退时 `inside_peak` 必为 0 ⇒ 本判据变红。
//!
//! ## 两条臂
//!
//! - GUI 臂：走真实菜单入口（`command_execute("export-stems")`，与
//!   `export_menu_repro_167.rs` 同款装配），断言分轨 WAV 里**真有**内嵌样本的声音；
//! - 后端对照臂：同一份工程直接调 `export_stems(..., &[])`，断言同一轨**静音**
//!   —— 它钉住「`&[]` 回退的失败形态」，证明 GUI 臂的判据对回退敏感（不是
//!   「无论如何都绿」的空判据）。
//!
//! ⚠ 分轨输出目录是全局的 `temp_dir()/yeban-stems`（与 167 同款，`host::stems_output_dir`
//! 在 Linux 上的口径）；开跑前清掉、跑完清掉。`stems-manifest.json` 出现即判
//! 工作线程跑完（60 秒上限，与 167 同款观测法）。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::host::{ModulePaths, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{
    AssetHash, AssetMetadata, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig,
    MediaKind, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav_mono_f32, write_plain_wav};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

/// 夹具速度：120 BPM / 960 PPQ / 48 kHz ⇒ **1 tick = 25 帧**（可手算）。
const BPM: f64 = 120.0;
/// 权威时钟（`[MODEL-AST-001]`）。
const PPQ: u64 = 960;
/// 摆放起点：4800 tick = 0.25 s = **12000 帧**。
const START_TICK: u64 = 4_800;
/// 摆放时值：4800 tick = 0.25 s = **12000 帧**。
const DURATION_TICKS: u64 = 4_800;
/// 片段增益 0 dB（单位增益）：期望峰值 = 素材峰值，不用再乘增益系数。
const CLIP_GAIN_DB: f32 = 0.0;
/// 源素材帧数（20000 帧 ≈ 0.42 s）。
const ASSET_FRAMES: usize = 20_000;
/// 源样本（16-bit 满量程的 1/4 ⇒ 归一化 ≈ 0.25）。
const SOURCE_INT16: i16 = 8_192;
/// 导出采样率（默认 48 kHz；48 k 素材走"同率、不重采样"支路）。
const EXPORT_RATE: u32 = 48_000;
const NOW_MS: u64 = 1_700_000_000_000;

const HOST: &str = include_str!("../src/host.rs");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-188-stems-assets-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 分轨输出目录（与 `host::stems_output_dir` 在 Linux 上的口径相同：
/// 无 `USERPROFILE` ⇒ `temp_dir()/yeban-stems`）。
fn stems_dir() -> PathBuf {
    std::env::temp_dir().join("yeban-stems")
}

fn list_files(dir: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push((name, len));
        }
    }
    out.sort();
    out
}

/// tick ⇒ 帧（四舍五入），与 `yeban_engine::synth::{samples_per_tick, tick_to_sample}` 同式。
fn frames_of(tick: u64, rate: u32) -> usize {
    let samples_per_tick = 60.0 * f64::from(rate) / (BPM * PPQ as f64);
    (tick as f64 * samples_per_tick).round() as usize
}

/// 合成一份**字节可寻址**的常量 PCM WAV，返回容器字节。
fn synth_asset(dir: &Path) -> Vec<u8> {
    let path = dir.join("take-48000.wav");
    let samples: Vec<i16> = vec![SOURCE_INT16; ASSET_FRAMES];
    write_plain_wav(
        &path,
        &PcmFormat::integer(1, EXPORT_RATE, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写夹具 WAV");
    std::fs::read(&path).expect("读夹具 WAV")
}

/// 建一个**最小可渲染工程**：一条非 master 轨＋一个音频片段摆放，资产同时登记进
/// 工程的 `assets` 索引（真容器的形态：`assets/{sha256}` 与索引必须同时存在，
/// 否则 `resolve_asset` 会报 `assetMissing` 硬错而非静音）。
fn project_with_audio_clip(asset: AssetHash, bytes: &[u8]) -> YebanProjectV1 {
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
            content: ClipContent::Audio {
                asset: asset.clone(),
                gain_db: CLIP_GAIN_DB,
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
                offset_samples: None,
                fade_in_ticks: None,
                fade_out_ticks: None,
            },
        );
    project.assets.insert(
        asset.clone(),
        AssetMetadata {
            hash: asset,
            original_path: "samples/take-48000.wav".to_owned(),
            byte_len: u64::try_from(bytes.len()).expect("小文件"),
            media_kind: MediaKind::Audio,
            license: "CC0-1.0".to_owned(),
        },
    );
    project.validate().expect("夹具工程必须合法");
    project
}

/// 读回分轨 WAV 的单声道（归一化到 `[-1, 1]`）与采样率。
fn mono_of(path: &Path) -> (Vec<f32>, u32) {
    let (samples, rate, _display) = read_plain_wav_mono_f32(path).expect("分轨 WAV 必须可回读");
    (samples, rate)
}

fn peak_in(samples: &[f32], range: core::ops::Range<usize>) -> f32 {
    samples[range]
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()))
}

#[test]
fn gui_export_stems_passes_the_session_asset_pool_to_the_backend() {
    let dir = scratch("gui");
    let bytes = synth_asset(&dir);
    let hash = AssetHash::of_bytes(&bytes);
    let project = project_with_audio_clip(hash.clone(), &bytes);

    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<188-gui>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    // ⭐ 生产装配同款（`main.rs` 的 `--open`）：归档资产池换进端口。
    undo.replace_assets(vec![(hash, bytes.clone())]);
    let production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // 生产同序：模块入口先接，分派层最后覆盖。
    let _ = host::wire_module_actions_with(ui, Some(Rc::clone(&undo)), ModulePaths::default());
    host::wire_aether_actions(
        ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::clone(&view_cell),
    );

    let _ = std::fs::remove_dir_all(stems_dir());
    ui.invoke_command_execute("export-stems".into());
    let manifest = stems_dir().join("stems-manifest.json");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !manifest.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    let files = list_files(&stems_dir());
    assert!(
        manifest.exists(),
        "GUI 分轨 manifest 必须落盘（60 秒内），目录内容 {files:?}"
    );
    let wavs: Vec<_> = files
        .iter()
        .filter(|(name, _)| name.ends_with(".wav"))
        .collect();
    assert_eq!(
        wavs.len(),
        1,
        "恰好一条分轨 WAV 必须落盘（一条有声轨），目录内容 {files:?}"
    );
    assert!(wavs[0].1 > 0, "分轨 WAV 必须非空");
    let stem_path = stems_dir().join(&wavs[0].0);
    let (samples, rate) = mono_of(&stem_path);
    let start = frames_of(START_TICK, rate);
    let window_end = start + ASSET_FRAMES;
    let clip_end = start + frames_of(DURATION_TICKS, rate);
    assert!(
        samples.len() >= window_end,
        "分轨 WAV 必须覆盖摆放窗口（{} 帧），实得 {} 帧",
        window_end,
        samples.len()
    );
    let inside_peak = peak_in(&samples, start..window_end);
    let before_peak = peak_in(&samples, 0..start);
    let after_end = clip_end.min(samples.len());
    let after_peak = if window_end < after_end {
        peak_in(&samples, window_end..after_end)
    } else {
        0.0
    };
    eprintln!(
        "[188-gui] stem={} rate={rate} inside_peak={inside_peak} before_peak={before_peak} after_peak={after_peak}",
        wavs[0].0
    );
    assert!(
        inside_peak > 0.05 && inside_peak < 0.5,
        "摆放窗口内必须可闻（期望 ≈ 0.25），实测 inside_peak={inside_peak}"
    );
    assert!(
        before_peak < 1e-3,
        "摆放窗口前必须静音，实测 before_peak={before_peak}"
    );
    assert!(
        after_peak < 1e-3,
        "素材耗尽后（仍在摆放内）必须静音，实测 after_peak={after_peak}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(stems_dir());
}

/// 后端对照：同一份工程、`&[]` 空池 ⇒ 同一轨静音。
///
/// 这是 GUI 臂判据的**灵敏度证明**：`&[]` 回退时本臂断言的峰值是精确的 0，
/// 因此 GUI 臂若被改回 `&[]`，上面的 GUI 判据必红（不是"无论如何都绿"）。
#[test]
fn stems_export_with_empty_asset_pool_renders_the_audio_track_silent() {
    let dir = scratch("empty-pool");
    let bytes = synth_asset(&dir);
    let hash = AssetHash::of_bytes(&bytes);
    let project = project_with_audio_clip(hash, &bytes);

    let out = dir.join("stems-empty");
    std::fs::create_dir_all(&out).expect("建输出目录");
    let report = yeban_app::export_stems::export_stems(
        &project,
        &dir.join("project.yeban"),
        &out,
        None,
        &[],
    )
    .expect("空池下导出本身必须成功（静音，不是硬错）");
    assert_eq!(report.stems.len(), 1, "空池下仍应产出一条分轨（静音）");
    let stem_path = report.stems[0].path.clone();
    let (samples, rate) = mono_of(&stem_path);
    let start = frames_of(START_TICK, rate);
    let window_end = start + ASSET_FRAMES;
    assert!(
        samples.len() >= window_end,
        "空池分轨 WAV 也必须覆盖摆放窗口，实得 {} 帧",
        samples.len()
    );
    let inside_peak = peak_in(&samples, start..window_end);
    eprintln!("[188-empty-pool] inside_peak={inside_peak}");
    assert!(
        inside_peak < 1e-3,
        "空池下同一轨必须静音，实测 inside_peak={inside_peak}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 钉住调用点：`export-stems` 臂必须从端口取池，⛔ 不得写死 `&[]`。
///
/// 这只是**钉子**（行为证明在上面两条臂里）：它让"臂被改回 `&[]`"这个
/// 一行回退在 CI 里立刻变红，而不需要等 60 秒的活窗口。
#[test]
fn export_stems_arm_reads_the_asset_pool_from_the_port() {
    let arm_start = HOST
        .find(r#""export-stems" => {"#)
        .expect("host.rs 必须有 export-stems 臂（带 `{` 的那条，不是菜单文案表）");
    let arm_end = HOST[arm_start..]
        .find(r#""app-quit" => {"#)
        .map(|offset| arm_start + offset)
        .expect("export-stems 臂后必须有 app-quit 臂");
    let arm = &HOST[arm_start..arm_end];
    assert!(
        arm.contains("port.assets()"),
        "export-stems 臂必须从 UndoPort 取资产池（port.assets()）"
    );
    assert!(!arm.contains("&[]"), "export-stems 臂不得再传空池 &[]");
}
