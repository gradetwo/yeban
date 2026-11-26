//! T-FIX-418：GUI 分轨导出的回执必须有**音频片段维度**的计数，toast 不能只报"完成"。
//!
//! 修前：`ExportStemsReport` 只有轨级 `stems` / `skipped`（整轨不出文件才进
//! `skipped`）；一条分轨的音频片段全因"资产字节不在会话池里"被跳过时，文件照
//! 常落盘、却是静音的，而 toast 只按 `report.stems.len()` /
//! `report.skipped.len()` 报 `"[导出] 分轨完成：{} 条（跳过 {}）"` —— 用户看
//! 到"完成"，听到的却是静音。
//!
//! 修法：`RenderArtifact.audio_clips_unrendered`（逐源 `audioClipsUnrendered`
//! 的合计）→ `StemRow.audio_clips_rendered` / `audio_clips_skipped` →
//! `ExportStemsReport.audio_clips_rendered` / `audio_clips_skipped`（
//! `lines()` 的 `exported-stems:` / `stem:` 行同款带出，manifest 的每条
//! `stems[]` 行同款带出）→ toast 由纯函数 `export_stems::toast_message`
//! 判定：未渲染数 > 0 降级为"分轨警告"，= 0 时仍是原来的"分轨完成"一字不改。
//!
//! ## 判定量（先说指标，再跑命令）
//!
//! 每个数都是**实测读数**（不是文案断言）：
//! - `audio_clips_rendered` / `audio_clips_skipped`：`ExportStemsReport` 上
//!   的两个 `u64` 字段；空池下音频轨必须读成 `0 / 1`，满池下读成 `1 / 0`；
//! - `inside_peak`：分轨 WAV 单声道峰值（归一化到 `[-1, 1]`），摆放窗口
//!   `[start_frame, start_frame + ASSET_FRAMES)` 内；素材是 0.25 常量、增益
//!   0 dB ⇒ 期望 ≈ 0.25，断言 `0.05 < inside_peak < 0.5`（与 188 同款）；
//!   静音轨的读数是精确的 0；
//! - `midi_peak`：MIDI 分轨前 2000 帧内的峰值（采样器样本只有 480 帧、
//!   0.25 常量，力度 48 ⇒ 峰值会被力度缩放，只断言 `> 0.01`：可闻、非静音）；
//! - toast 文案：后端两条判据直接对**真实报告**调 `toast_message` 纯函数
//!   断言（空池 ⇒ "分轨警告…1 个音频片段未渲染"，满池 ⇒ 逐字的旧"分轨完成"）。
//!   GUI 臂的 toast **在 Tier-1 下不可读**：平台没有 `new_event_loop_proxy` ⇒
//!   `upgrade_in_event_loop` 恒返回 `Err`，工作线程的 toast 回调跑不到 UI 线程
//!   （`export_menu_repro_167.rs` 文件头同款留白）。臂钉子（`toast_message` 必须
//!   出现在臂里、 `"[导出] 分轨完成"` 不得再写死在臂里）+ 后端纯函数判据 +
//!   GUI manifest 计数，三者合起来才是 toast 降级的完整证据链；"文案真能
//!   显示在真机屏幕上"只在真机事件循环上可证，本文件如实留白（与 167 同口径）。
//! - GUI/CLI 样本同一性：同一份工程、同一份满池，GUI 分轨与后端直调分轨的
//!   音频轨 f32 样本向量**逐个相等**（确定性渲染，同一条后端）。
//!
//! ## 口径（⛔ 别把两个"跳过"混账）
//!
//! - 轨级 `skipped`：整轨不出文件（主总线 / 非源节点 / 空轨），`skipped.len()`；
//! - 片段级 `audioClipsSkipped`：出了文件、但其中"索引有、池里无"的音频片段
//!   摆放数；**不进**轨级 `skipped`，两者不重复计数；
//! - 被 `mute` / `solo` 门控的摆放数在 `audioClipsGated`，**不进**这个数
//!   （静音是调用方的明确意图，缺资产是意外）。
//!
//! ⚠ 分轨输出目录是全局的 `temp_dir()/yeban-stems`（与 167 / 188 同款）；
//! 两条 GUI 场景各是一条测试函数（Tier-1 平台每个线程只能装一次），用一把
//! 静态锁（`GUI_STEMS_DIR_LOCK`）串行化，⛔ 不许同时写同一目录。
//! `stems-manifest.json` 出现即判工作线程跑完（60 秒上限，与 188 同款观测法）。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use yeban_app::bridge::ViewState;
use yeban_app::export_stems::toast_message;
use yeban_app::host;
use yeban_app::host::{ModulePaths, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{
    AssetHash, AssetMetadata, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig,
    MediaKind, MidiNote, RoutingEdge, RoutingGraph, RoutingKind, SamplerBinding, TrackKind,
    TrackV3, YebanProjectV1,
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
        "yeban-418-stems-clip-counts-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 分轨输出目录（与 `host::stems_output_dir` 在 Linux 上的口径相同：
/// 无 `USERPROFILE` ⇒ `temp_dir()/yeban-stems`）。
fn isolate_stems_dir() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // 产品回退目录由 `host::stems_output_dir` 读 `USERPROFILE` 决定；把本测试
        // 进程的 profile 指到进程唯一根，产品与实际断言就都落在同一份私有目录，
        // 不再与别的用户/测试二进制抢 `temp_dir()/yeban-stems`。
        let root = std::env::temp_dir().join(format!("yeban-stems-{}", std::process::id()));
        // SAFETY: 只在本进程第一次碰分轨目录前写一次；本文件没有别的 USERPROFILE 读者。
        unsafe { std::env::set_var("USERPROFILE", root) };
    });
}

fn stems_dir() -> PathBuf {
    isolate_stems_dir();
    let root = std::env::var("USERPROFILE").expect("USERPROFILE 已由本测试进程设置");
    PathBuf::from(root)
        .join("Documents")
        .join("Yeban")
        .join("stems")
}

fn clear_stems_dir() {
    let dir = stems_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("清理分轨输出目录");
    }
}

/// tick ⇒ 帧（四舍五入），与 `yeban_engine::synth::{samples_per_tick, tick_to_sample}` 同式。
fn frames_of(tick: u64, rate: u32) -> usize {
    let samples_per_tick = 60.0 * f64::from(rate) / (BPM * PPQ as f64);
    (tick as f64 * samples_per_tick).round() as usize
}

/// 合成一份**字节可寻址**的常量 PCM WAV，返回容器字节（与 188 同款夹具）。
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

/// 写一份最小 SFZ（`key=60` 播 480 帧、0.25 常量的 `tone.wav`），返回 SFZ 路径。
fn write_tone_sfz(dir: &Path) -> PathBuf {
    let samples: Vec<i16> = (0..480).map(|_| SOURCE_INT16).collect();
    write_plain_wav(
        dir.join("tone.wav"),
        &PcmFormat::integer(1, EXPORT_RATE, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写夹具 WAV");
    let sfz = dir.join("tone.sfz");
    std::fs::write(&sfz, "<region> key=60 sample=tone.wav\n").expect("写夹具 SFZ");
    sfz
}

/// 建一个**最小可渲染工程**：一条非 master 轨＋一个音频片段摆放，资产同时登记进
/// 工程的 `assets` 索引（与 188 同款：`assets/{sha256}` 与索引必须同时存在，
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
                source_start_samples: None,
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

            frames: None,
            sample_rate: None,
        },
    );
    project.validate().expect("夹具工程必须合法");
    project
}

/// 给工程加一条 MIDI 轨：一个 60 号音符（tick 0 起、1920 tick 长、力度 48），
/// 绑 `sfz_path` 的采样器（与 `export_wav_aux_sampler` 同款形状）。
fn add_midi_track(project: &mut YebanProjectV1, sfz_path: &Path) {
    let master = project.master_bus_track_id;
    let track = EntityId::new();
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note = EntityId::new();
    let edge = EntityId::new();

    let mut notes = BTreeMap::new();
    notes.insert(
        note,
        MidiNote {
            velocity: 48,
            ..MidiNote::new(note, 0, 60, 1_920)
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "midi-take".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    let mut track_v3 = TrackV3 {
        id: track,
        name: "Midi".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    track_v3.sampler = Some(SamplerBinding {
        path: sfz_path.to_str().expect("UTF-8").to_owned(),
        note_octave_override: None,
    });
    track_v3.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
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
    project.tracks.insert(track, track_v3);
    project.routing_graph.nodes.push(track);
    project.routing_graph.edges.insert(
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
    project.validate().expect("加 MIDI 轨后工程必须合法");
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

/// 从 manifest 里取指定轨名的那条 `stems[]` 行。
fn manifest_stem_row(manifest_path: &Path, name: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(manifest_path).expect("读 manifest");
    let value: serde_json::Value = serde_json::from_str(&text).expect("manifest 必须是 JSON");
    value["stems"]
        .as_array()
        .expect("manifest 必须有 stems 数组")
        .iter()
        .find(|row| row["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("manifest 里必须有 {name} 轨"))
        .clone()
}

/// 等 manifest 落盘（60 秒上限，与 188 同款观测法）。
fn wait_manifest() {
    let manifest = stems_dir().join("stems-manifest.json");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !manifest.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(manifest.exists(), "GUI 分轨 manifest 必须落盘（60 秒内）");
}

#[test]
fn empty_pool_backend_report_counts_the_skipped_audio_clip_and_warns() {
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
    assert_eq!(report.audio_clips_rendered, 0, "空池下没有音频片段被渲染");
    assert_eq!(
        report.audio_clips_skipped, 1,
        "空池下恰好 1 个音频片段因资产不在池里而未渲染"
    );
    // 轨级 skipped 是 1：主总线（`masterBusTrack`）。音频轨**不在**里面 ——
    // 它出了文件（静音的），不是"整轨不出文件"；片段级跳过不进轨级 skipped。
    assert_eq!(report.skipped.len(), 1, "轨级 skipped 只有主总线一条");
    assert_eq!(
        report.skipped[0].track, "Master",
        "轨级 skipped 的那条必须是主总线"
    );

    // 分轨本身确实静音（与 188 同款读数）。
    let stem_path = report.stems[0].path.clone();
    let (samples, rate) = mono_of(&stem_path);
    let start = frames_of(START_TICK, rate);
    let window_end = start + ASSET_FRAMES;
    let inside_peak = peak_in(&samples, start..window_end);
    eprintln!("[418-empty-pool] inside_peak={inside_peak}");
    assert!(
        inside_peak < 1e-3,
        "空池下同一轨必须静音，实测 inside_peak={inside_peak}"
    );

    // 回执行里必须带这两个数。
    let lines = report.lines();
    let summary = lines
        .iter()
        .find(|line| line.starts_with("exported-stems:"))
        .expect("回执里必须有 exported-stems: 一行");
    assert!(
        summary.contains("audioClipsRendered=0"),
        "汇总行必须报 audioClipsRendered=0，实得 {summary}"
    );
    assert!(
        summary.contains("audioClipsSkipped=1"),
        "汇总行必须报 audioClipsSkipped=1，实得 {summary}"
    );
    let stem_line = lines
        .iter()
        .find(|line| line.starts_with("stem:"))
        .expect("回执里必须有 stem: 一行");
    assert!(
        stem_line.contains("audioClipsRendered=0") && stem_line.contains("audioClipsSkipped=1"),
        "stem: 行必须带片段计数，实得 {stem_line}"
    );

    // manifest 的每条 stems[] 行同款带数。
    let row = manifest_stem_row(&report.manifest_path, "Audio");
    assert_eq!(
        row["audioClipsRendered"], 0,
        "manifest 音频轨 rendered 必须是 0"
    );
    assert_eq!(
        row["audioClipsSkipped"], 1,
        "manifest 音频轨 skipped 必须是 1"
    );

    // toast 必须降级，不再只报"完成"。
    let toast = toast_message(&report, &out);
    eprintln!("[418-empty-pool] toast={toast}");
    assert!(
        toast.starts_with("[导出] 分轨警告"),
        "有未渲染片段时 toast 必须降级为警告，实得 {toast}"
    );
    assert!(
        toast.contains("1 个音频片段未渲染"),
        "警告里必须点名未渲染的片段数，实得 {toast}"
    );
    assert!(
        !toast.contains("分轨完成"),
        "警告文案里不得再出现\"分轨完成\"，实得 {toast}"
    );

    std::fs::remove_dir_all(&dir).expect("清理临时目录");
}

#[test]
fn full_pool_backend_report_counts_the_rendered_audio_clip_and_completes() {
    let dir = scratch("full-pool");
    let bytes = synth_asset(&dir);
    let hash = AssetHash::of_bytes(&bytes);
    let project = project_with_audio_clip(hash.clone(), &bytes);
    let pool = vec![(hash, bytes.clone())];

    let out = dir.join("stems-full");
    std::fs::create_dir_all(&out).expect("建输出目录");
    let report = yeban_app::export_stems::export_stems(
        &project,
        &dir.join("project.yeban"),
        &out,
        None,
        &pool,
    )
    .expect("满池下导出必须成功");
    assert_eq!(report.stems.len(), 1, "满池下应产出一条分轨");
    assert_eq!(
        report.audio_clips_rendered, 1,
        "满池下恰好 1 个音频片段被渲染"
    );
    assert_eq!(report.audio_clips_skipped, 0, "满池下没有未渲染的片段");

    let stem_path = report.stems[0].path.clone();
    let (samples, rate) = mono_of(&stem_path);
    let start = frames_of(START_TICK, rate);
    let window_end = start + ASSET_FRAMES;
    let inside_peak = peak_in(&samples, start..window_end);
    eprintln!("[418-full-pool] inside_peak={inside_peak}");
    assert!(
        inside_peak > 0.05 && inside_peak < 0.5,
        "满池下摆放窗口内必须可闻（期望 ≈ 0.25），实测 inside_peak={inside_peak}"
    );

    let lines = report.lines();
    let summary = lines
        .iter()
        .find(|line| line.starts_with("exported-stems:"))
        .expect("回执里必须有 exported-stems: 一行");
    assert!(
        summary.contains("audioClipsRendered=1"),
        "汇总行必须报 audioClipsRendered=1，实得 {summary}"
    );
    assert!(
        summary.contains("audioClipsSkipped=0"),
        "汇总行必须报 audioClipsSkipped=0，实得 {summary}"
    );
    let row = manifest_stem_row(&report.manifest_path, "Audio");
    assert_eq!(
        row["audioClipsRendered"], 1,
        "manifest 音频轨 rendered 必须是 1"
    );
    assert_eq!(
        row["audioClipsSkipped"], 0,
        "manifest 音频轨 skipped 必须是 0"
    );

    // 零未渲染时 toast 必须是原来的"完成"文案，**一字不改**（"跳过 1" 是主总线）。
    let toast = toast_message(&report, &out);
    assert_eq!(
        toast,
        format!("[导出] 分轨完成：1 条（跳过 1）→ {}", out.display()),
        "零未渲染时 toast 必须逐字沿用旧文案"
    );

    std::fs::remove_dir_all(&dir).expect("清理临时目录");
}

/// GUI 两条场景各是一条测试函数（Tier-1 平台**每个线程只能装一次**，
/// 两条场景若在同一线程里顺序建两个活窗口，第二个 `LivePort::new` 必报
/// `PlatformUnavailable`；cargo 每个测试函数跑在自己的线程上，正好一边一个）。
/// 两条都写全局 `temp_dir()/yeban-stems`，因此用一把静态锁串行化 ——
/// ⛔ 不许两个场景同时写同一目录。
static GUI_STEMS_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 场景（1）满池：音频轨 + MIDI 轨都可闻，manifest 报 rendered=1/skipped=0，
/// 且 GUI/CLI 音频分轨样本逐个相等。
#[test]
fn gui_export_stems_full_pool_reports_rendered_clip_counts() {
    let _guard = GUI_STEMS_DIR_LOCK.lock().expect("GUI 分轨场景锁必须能拿到");
    let dir = scratch("gui-full");
    let bytes = synth_asset(&dir);
    let hash = AssetHash::of_bytes(&bytes);
    let sfz = write_tone_sfz(&dir);
    let mut project = project_with_audio_clip(hash.clone(), &bytes);
    add_midi_track(&mut project, &sfz);

    run_gui_export(&project, &hash, &bytes, true);
    let manifest_path = stems_dir().join("stems-manifest.json");
    let audio_row = manifest_stem_row(&manifest_path, "Audio");
    assert_eq!(
        audio_row["audioClipsRendered"], 1,
        "GUI 满池：音频轨 rendered 必须是 1"
    );
    assert_eq!(
        audio_row["audioClipsSkipped"], 0,
        "GUI 满池：音频轨 skipped 必须是 0"
    );
    let midi_row = manifest_stem_row(&manifest_path, "Midi");
    assert_eq!(
        midi_row["audioClipsRendered"], 0,
        "MIDI 轨没有音频片段，rendered 必须是 0（不是漏数）"
    );
    assert_eq!(
        midi_row["audioClipsSkipped"], 0,
        "MIDI 轨没有音频片段，skipped 必须是 0"
    );
    // toast 文案在这里**不断言**：Tier-1 平台没有 `new_event_loop_proxy` ⇒
    // `upgrade_in_event_loop` 恒返回 `Err`，工作线程的 toast 回调跑不到 UI 线程
    // （`export_menu_repro_167.rs` 的文件头已如实留白："toast 能不能到，只在真机
    // 事件循环上可证"）。toast 的**判定**由后端两条判据（`toast_message` 纯函数，
    // 拿的是同一份 `ExportStemsReport`）+ 下面的臂钉子钉死；GUI 臂能证明的是
    // 报告里的数 —— manifest 的行就是臂收到的那份报告的落盘形态。
    let gui_audio_path = PathBuf::from(
        audio_row["path"]
            .as_str()
            .expect("manifest 音频轨必须有 path"),
    );
    let (gui_audio, gui_rate) = mono_of(&gui_audio_path);
    let start = frames_of(START_TICK, gui_rate);
    let window_end = start + ASSET_FRAMES;
    let inside_peak = peak_in(&gui_audio, start..window_end);
    eprintln!("[418-gui-full] audio inside_peak={inside_peak}");
    assert!(
        inside_peak > 0.05 && inside_peak < 0.5,
        "GUI 满池：音频分轨摆放窗口内必须可闻，实测 inside_peak={inside_peak}"
    );
    let midi_path = PathBuf::from(
        midi_row["path"]
            .as_str()
            .expect("manifest MIDI 轨必须有 path"),
    );
    let (midi_samples, _midi_rate) = mono_of(&midi_path);
    let midi_peak = peak_in(&midi_samples, 0..2_000);
    eprintln!("[418-gui-full] midi peak(0..2000)={midi_peak}");
    assert!(
        midi_peak > 0.01,
        "GUI 满池：MIDI 分轨前 2000 帧内必须可闻，实测 midi_peak={midi_peak}"
    );

    // ---- 验收（3）：同一份工程、同一份满池，后端直调的音频分轨样本必须与 GUI 的逐个相等。
    let pool = vec![(hash.clone(), bytes.clone())];
    let cli_out = dir.join("cli-stems");
    std::fs::create_dir_all(&cli_out).expect("建 CLI 输出目录");
    let cli_report = yeban_app::export_stems::export_stems(
        &project,
        &dir.join("project.yeban"),
        &cli_out,
        None,
        &pool,
    )
    .expect("CLI 腿导出必须成功");
    let cli_audio = cli_report
        .stems
        .iter()
        .find(|stem| stem.track == "Audio")
        .expect("CLI 回执里必须有 Audio 轨");
    let (cli_samples, cli_rate) = mono_of(&cli_audio.path);
    assert_eq!(cli_rate, gui_rate, "GUI/CLI 音频分轨采样率必须相同");
    assert_eq!(
        cli_samples.len(),
        gui_audio.len(),
        "GUI/CLI 音频分轨帧数必须相同"
    );
    assert_eq!(
        cli_samples, gui_audio,
        "GUI/CLI 音频分轨样本必须逐个相等（同一份工程、同一份满池、同一条后端）"
    );

    std::fs::remove_dir_all(&dir).expect("清理临时目录");
    clear_stems_dir();
}

/// 场景（2）空池：音频轨静音（manifest 报 rendered=0/skipped=1）、MIDI 轨
/// （采样器走磁盘，不走资产池）仍可闻。
#[test]
fn gui_export_stems_empty_pool_reports_skipped_audio_clip() {
    let _guard = GUI_STEMS_DIR_LOCK.lock().expect("GUI 分轨场景锁必须能拿到");
    let dir = scratch("gui-empty");
    let bytes = synth_asset(&dir);
    let hash = AssetHash::of_bytes(&bytes);
    let sfz = write_tone_sfz(&dir);
    let mut project = project_with_audio_clip(hash.clone(), &bytes);
    add_midi_track(&mut project, &sfz);

    run_gui_export(&project, &hash, &bytes, false);
    let manifest_path = stems_dir().join("stems-manifest.json");
    let audio_row = manifest_stem_row(&manifest_path, "Audio");
    assert_eq!(
        audio_row["audioClipsRendered"], 0,
        "GUI 空池：音频轨 rendered 必须是 0"
    );
    assert_eq!(
        audio_row["audioClipsSkipped"], 1,
        "GUI 空池：音频轨 skipped 必须是 1"
    );
    // toast 不断言：Tier-1 下工作线程的 toast 回调跑不到 UI 线程（见文件头）。
    let gui_audio_path = PathBuf::from(
        audio_row["path"]
            .as_str()
            .expect("manifest 音频轨必须有 path"),
    );
    let (gui_audio, gui_rate) = mono_of(&gui_audio_path);
    let start = frames_of(START_TICK, gui_rate);
    let window_end = start + ASSET_FRAMES;
    let inside_peak = peak_in(&gui_audio, start..window_end);
    eprintln!("[418-gui-empty] audio inside_peak={inside_peak}");
    assert!(
        inside_peak < 1e-3,
        "GUI 空池：音频分轨必须静音，实测 inside_peak={inside_peak}"
    );
    let midi_row = manifest_stem_row(&manifest_path, "Midi");
    let midi_path = PathBuf::from(
        midi_row["path"]
            .as_str()
            .expect("manifest MIDI 轨必须有 path"),
    );
    let (midi_samples, _midi_rate) = mono_of(&midi_path);
    let midi_peak = peak_in(&midi_samples, 0..2_000);
    eprintln!("[418-gui-empty] midi peak(0..2000)={midi_peak}");
    assert!(
        midi_peak > 0.01,
        "GUI 空池：MIDI 分轨（采样器走磁盘，不走资产池）仍必须可闻，实测 midi_peak={midi_peak}"
    );

    std::fs::remove_dir_all(&dir).expect("清理临时目录");
    clear_stems_dir();
}

/// 跑一次真实 GUI「导出分轨」：`command_execute("export-stems")`，等 manifest
/// 落盘即返回（toast 不读 —— Tier-1 下工作线程的 toast 回调跑不到 UI 线程，
/// 见文件头与场景内的注释）。`with_assets` 为真时把资产池换进端口（满池），
/// 为假时端口保持空池。
fn run_gui_export(project: &YebanProjectV1, hash: &AssetHash, bytes: &[u8], with_assets: bool) {
    let view = ViewState::from_project(project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<418-gui>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    if with_assets {
        // ⭐ 生产装配同款（`main.rs` 的 `--open`）：归档资产池换进端口。
        undo.replace_assets(vec![(hash.clone(), bytes.to_vec())]);
    }
    let production = ProductionLoop::start(project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // 生产同序：模块入口先接，分派层最后覆盖。
    let _ = host::wire_module_actions_with(
        ui,
        Some(Rc::clone(&undo)),
        ModulePaths::default(),
        Rc::new(yeban_app::file_picker::NoPicker),
    );
    host::wire_aether_actions(
        ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::clone(&view_cell),
    );

    clear_stems_dir();
    ui.invoke_command_execute("export-stems".into());
    wait_manifest();
}

/// 钉住调用点：`export-stems` 臂的 toast 文案必须走 `toast_message` 判定，
/// ⛔ 不得再把 `"[导出] 分轨完成"` 写死在臂里。
///
/// 这只是**钉子**（行为证明在上面三条判据里）：它让"臂被改回写死'完成'"这个
/// 一行回退在 CI 里立刻变红，而不需要等活窗口。
#[test]
fn export_stems_arm_toast_goes_through_toast_message() {
    let arm_start = HOST
        .find(r#""export-stems" => {"#)
        .expect("host.rs 必须有 export-stems 臂（带 `{` 的那条，不是菜单文案表）");
    let arm_end = HOST[arm_start..]
        .find(r#""app-quit" => {"#)
        .map(|offset| arm_start + offset)
        .expect("export-stems 臂后必须有 app-quit 臂");
    let arm = &HOST[arm_start..arm_end];
    assert!(
        arm.contains("toast_message"),
        "export-stems 臂的 toast 必须走 export_stems::toast_message 判定"
    );
    assert!(
        !arm.contains("[导出] 分轨完成"),
        "export-stems 臂不得再写死\"分轨完成\"文案"
    );
}
