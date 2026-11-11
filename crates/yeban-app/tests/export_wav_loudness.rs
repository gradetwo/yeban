//! `--export-wav` 的响度归一化开关（`--loudness-target` / `--true-peak-ceiling`）。
//!
//! # 判据与文件头的对应
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | [`loudness_target_is_reached_within_tolerance`] | 给目标 ⇒ 落盘文件实测响度落在目标 ± 0.5 LU 内，增益真施加了，`summary()` 多一行 |
//! | [`true_peak_ceiling_wins_over_the_target`] | 上限赢 ⇒ `bound=TruePeakCeiling`，真峰值 ≤ 上限，目标没达到（**预期**，不失败） |
//! | [`applying_the_same_preset_twice_is_bit_exact`] | 同一预设连导两次 ⇒ 整文件 `sha256` / 母带 `digest` / 响度报告逐位相同（**自比**） |
//! | [`default_export_reports_no_loudness_and_keeps_the_legacy_summary`] | 不给开关 ⇒ 报告无响度字段，`summary()` 逐字不变（既有文本判据不破） |
//! | [`silence_cannot_reach_a_target`] | 静音 ＋ 有目标 ⇒ 响亮失败，不留文件 |
//! | [`unsupported_sample_rate_fails_loudly`] | 无 K 计权系数的采样率 ⇒ 响亮失败，不留文件 |
//! | [`non_stereo_is_refused_when_loudness_is_requested`] | 非立体声 ＋ 给开关 ⇒ 响亮失败，不留文件 |
//! | [`the_real_cli_prints_the_loudness_line_only_when_the_switches_are_given`] | 真二进制：给开关 ⇒ stdout 多一行 `loudness:`；不给 ⇒ 一行都不多 |
//! | [`the_loudness_tolerance_matches_the_mcp_verdict`] | 导出侧容差与 MCP `render_master` 的判定容差同值（`in-process-mcp` 档才编译） |
//!
//! # 口径
//!
//! - 夹具是**代码里构造的最小工程**（与 `export_wav_clipping.rs` 同款写法），⛔ 不读仓库外文件；
//! - 音频片段夹具：440 Hz 正弦、幅度 0.5、片段增益 0 dB、1 秒 —— 母带响度约 −9.7 LUFS、
//!   真峰值约 −6.0 dBTP（**约数**：判据只断言关系与容差，不钉绝对读数）；
//! - "不改音频"用**同一台机器上的自比**钉住（见 [`applying_the_same_preset_twice_is_bit_exact`]）；
//!   绝对字节是机器相关的（`export_wav_clipping.rs` 的文件头口径），本文件不钉绝对哈希常量。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use yeban_app::export_wav::{
    WavExportError, WavExportOptions, export_wav_with_sfz_file_and_assets_under,
};
use yeban_model::container::write_project_container;
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, CommitGraph, EntityId, LoopConfig,
    MidiNote, RoutingEdge, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::mastering::{ExportPreset, GainBound, LOUDNESS_TARGET_TOLERANCE_LU};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-loudness-{tag}-{}-{}",
        std::process::id(),
        EntityId::new()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn sha_hex(path: &Path) -> String {
    AssetHash::of_bytes(&std::fs::read(path).expect("读产物"))
        .as_str()
        .to_owned()
}

/// 音频片段夹具：**没有 MIDI 音符**，母带只由一段 440 Hz 正弦构成（片段增益 0 dB）。
///
/// 返回 `(工程, 资产池)`。正弦幅度 `amplitude`：0.5 ⇒ 母带约 −9.7 LUFS、真峰值约
/// −6.0 dBTP，与目标 −16 LUFS / −3 LUFS 都差得够远，增益的方向与"上限赢"都测得到；
/// 0.0 ⇒ 全静音（[`silence_cannot_reach_a_target`] 用它：静音也要能**渲染成功**，
/// 失败必须来自响度那一层 —— 只有主总线的工程在渲染计划里就失败，测不到响度）。
/// 夹具与 `export_wav_clipping.rs` 的 `audio_clip_project` 同形（只改了幅度与增益）。
fn sine_clip_project(dir: &Path, amplitude: f32) -> (YebanProjectV1, BTreeMap<AssetHash, Vec<u8>>) {
    let asset_path = dir.join("asset-f32.wav");
    let frames = 48_000_usize;
    let mut samples: Vec<f32> = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        let phase = std::f64::consts::TAU * 440.0 * frame as f64 / 48_000.0;
        let value = (phase.sin() * f64::from(amplitude)) as f32;
        samples.push(value);
        samples.push(value);
    }
    write_plain_wav(
        &asset_path,
        &PcmFormat::float(2, 48_000, 32),
        &PcmBuffer::Float32(samples),
    )
    .expect("写夹具素材");
    let bytes = std::fs::read(&asset_path).expect("读夹具素材");
    let asset = AssetHash::of_bytes(&bytes);
    let mut assets = BTreeMap::new();
    assets.insert(asset.clone(), bytes);

    let master = fixture_id("M1");
    let track_id = fixture_id("T0");
    let clip = fixture_id("K0");
    let placement = fixture_id("P0");
    let edge = fixture_id("E0");
    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "loudness sine fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0001,
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
        name: "Audio".to_owned(),
        kind: TrackKind::Audio,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 1_920,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "sine-f32".to_owned(),
            content: ClipContent::Audio {
                asset,
                gain_db: 0.0,
            },
        },
    );
    project.tracks.insert(track_id, track);
    project.routing_graph.nodes.push(track_id);
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.validate().expect("夹具工程必须合法");
    (project, assets)
}

/// 每个 `(名字, volume_db)` 一条 MIDI 轨；音符**完全相同**（同音高／同时值／力度 127）。
///
/// 与 `export_wav_clipping.rs` 的 `project_with_tracks` 同形（真二进制与采样率／声道判据用它）。
fn project_with_tracks(tracks: &[(&str, f32)]) -> YebanProjectV1 {
    let master = fixture_id("M1");
    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "cli loudness fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0001,
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
    for (index, (name, volume_db)) in tracks.iter().enumerate() {
        let track_id = fixture_id(&format!("T{index}"));
        let clip = fixture_id(&format!("K{index}"));
        let placement = fixture_id(&format!("P{index}"));
        let note = fixture_id(&format!("N{index}"));
        let edge = fixture_id(&format!("E{index}"));
        let mut track = TrackV3 {
            id: track_id,
            name: (*name).to_owned(),
            kind: TrackKind::Midi,
            volume_db: *volume_db,
            ..TrackV3::default()
        };
        track.clips.insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: clip,
                start_tick: 0,
                duration_ticks: 1_920,
                loop_config: LoopConfig::default(),
                muted: false,
                offset_samples: None,
            },
        );
        let mut notes = BTreeMap::new();
        notes.insert(
            note,
            MidiNote {
                velocity: 127,
                ..MidiNote::new(note, 0, 69, 1_920)
            },
        );
        project.clip_pool.insert(
            clip,
            ClipPoolEntry {
                id: clip,
                name: format!("clip{index}"),
                content: ClipContent::Midi { notes },
            },
        );
        project.tracks.insert(track_id, track);
        project.routing_graph.nodes.push(track_id);
        project.routing_graph.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: track_id,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }
    project.validate().expect("夹具工程必须合法");
    project
}

/// 正弦夹具导出一次：1 秒（48_000 帧）＋ 给定的响度预设。
fn export_sine(
    dir: &Path,
    name: &str,
    preset: Option<ExportPreset>,
) -> (
    PathBuf,
    Result<yeban_app::export_wav::WavExportReport, WavExportError>,
) {
    let (project, assets) = sine_clip_project(dir, 0.5);
    let assets_slice: Vec<(AssetHash, Vec<u8>)> = assets
        .iter()
        .map(|(hash, bytes)| (hash.clone(), bytes.clone()))
        .collect();
    let path = dir.join(name);
    let options = WavExportOptions {
        frames: 48_000,
        loudness: preset,
        ..Default::default()
    };
    let report = export_wav_with_sfz_file_and_assets_under(
        &project,
        &path,
        options,
        None,
        &assets_slice,
        None,
    );
    (path, report)
}

fn write_container(dir: &Path, name: &str, project: &YebanProjectV1) -> PathBuf {
    let history = serde_json::to_vec(&CommitGraph::new()).expect("空图谱 JSON");
    let bytes = write_project_container(project, &history, &BTreeMap::new()).expect("写容器工程");
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("写容器");
    path
}

fn run_cli(arguments: &[&str]) -> (i32, String, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_yeban-app"))
        .args(arguments)
        .output()
        .expect("启动 yeban-app");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

#[test]
fn loudness_target_is_reached_within_tolerance() {
    let dir = scratch_dir("target");
    let (path, report) = export_sine(
        &dir,
        "target.wav",
        Some(ExportPreset::new(Some(-16.0), None)),
    );
    let report = report.expect("目标 −16 LUFS 的导出必须成功");
    let loudness = report
        .loudness
        .as_ref()
        .expect("给了开关 ⇒ 报告必须有响度字段");
    println!(
        "### 响度报告: target={:?} achieved={:?} gain={} bound={:?} truePeak={:?}",
        loudness.target_lufs,
        loudness.achieved_lufs,
        loudness.gain_db,
        loudness.bound,
        loudness.achieved_true_peak_dbtp
    );
    assert_eq!(loudness.target_lufs, Some(-16.0), "目标必须如实进报告");
    assert_eq!(
        loudness.bound,
        GainBound::LoudnessTarget,
        "目标赢 ⇒ bound 必须是 LoudnessTarget"
    );
    assert!(
        loudness.gain_db < -1.0,
        "夹具约 −9.7 LUFS ⇒ 到 −16 必须真减增益（gain_db = {}）",
        loudness.gain_db
    );
    let achieved = loudness.achieved_lufs.expect("非静音 ⇒ 落盘响度必须测得出");
    assert!(
        (achieved - (-16.0)).abs() <= LOUDNESS_TARGET_TOLERANCE_LU,
        "落盘响度 {achieved} LUFS 必须落在目标 ±{LOUDNESS_TARGET_TOLERANCE_LU} LU 内"
    );
    // `summary()` 多一行响度行（第二行），且行里同时有目标与实测值。
    let summary = report.summary();
    let note = report.loudness_note().expect("给了开关 ⇒ 提示行必须存在");
    assert!(
        summary.contains(&note),
        "summary() 必须含响度行:\n{summary}"
    );
    assert!(note.contains("target=-16.0"), "提示行必须写目标: {note}");
    assert!(
        note.contains("bound=LoudnessTarget"),
        "提示行必须写谁赢了: {note}"
    );
    assert_eq!(
        report.digest.len(),
        64,
        "digest 必须是 64 位十六进制（归一化后重算）"
    );
    assert!(path.exists(), "成功导出必须落盘");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn true_peak_ceiling_wins_over_the_target() {
    let dir = scratch_dir("ceiling");
    let (path, report) = export_sine(
        &dir,
        "ceiling.wav",
        Some(ExportPreset::new(Some(-3.0), Some(-3.0))),
    );
    let report = report.expect("上限赢的导出必须成功（上限赢不是失败）");
    let loudness = report
        .loudness
        .as_ref()
        .expect("给了开关 ⇒ 报告必须有响度字段");
    println!(
        "### 响度报告: target={:?} achieved={:?} gain={} bound={:?} truePeak={:?}",
        loudness.target_lufs,
        loudness.achieved_lufs,
        loudness.gain_db,
        loudness.bound,
        loudness.achieved_true_peak_dbtp
    );
    assert_eq!(
        loudness.bound,
        GainBound::TruePeakCeiling,
        "上限赢 ⇒ bound 必须是 TruePeakCeiling"
    );
    assert!(
        loudness.gain_db > 0.0 && loudness.gain_db < 3.5,
        "到 −3 LUFS 需约 +6.7 dB，上限只放行约 +3.0 dB（gain_db = {}）",
        loudness.gain_db
    );
    let achieved_peak = loudness
        .achieved_true_peak_dbtp
        .expect("非静音 ⇒ 落盘真峰值必须测得出");
    assert!(
        achieved_peak <= -3.0 + 0.1,
        "落盘真峰值 {achieved_peak} dBTP 不得超过上限 −3.0 dBTP（+0.1 dB 测量裕量）"
    );
    let achieved = loudness.achieved_lufs.expect("非静音 ⇒ 落盘响度必须测得出");
    assert!(
        achieved < -3.0,
        "上限赢 ⇒ 目标 −3 LUFS 没达到是预期（实测 {achieved} LUFS 必须比目标更轻，即更负）"
    );
    assert!(path.exists(), "上限赢的导出必须落盘");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn applying_the_same_preset_twice_is_bit_exact() {
    let dir = scratch_dir("idempotent");
    let preset = Some(ExportPreset::new(Some(-16.0), Some(-1.0)));
    let (first_path, first) = export_sine(&dir, "first.wav", preset);
    let (second_path, second) = export_sine(&dir, "second.wav", preset);
    let first = first.expect("第一次导出必须成功");
    let second = second.expect("第二次导出必须成功");
    assert_eq!(
        sha_hex(&first_path),
        sha_hex(&second_path),
        "同一预设连导两次 ⇒ 整文件 sha256 必须逐位相同"
    );
    assert_eq!(
        first.digest, second.digest,
        "同一预设连导两次 ⇒ 母带 digest 必须逐位相同"
    );
    assert_eq!(
        first.loudness, second.loudness,
        "同一预设连导两次 ⇒ 响度报告必须逐位相同"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn default_export_reports_no_loudness_and_keeps_the_legacy_summary() {
    let dir = scratch_dir("default");
    let (path, report) = export_sine(&dir, "default.wav", None);
    let report = report.expect("默认导出必须成功");
    assert!(report.loudness.is_none(), "不给开关 ⇒ 报告不得有响度字段");
    assert!(
        report.loudness_note().is_none(),
        "不给开关 ⇒ 提示行必须是 None"
    );
    let summary = report.summary();
    assert!(
        !summary.contains("loudness:"),
        "不给开关 ⇒ summary() 不得多一行:\n{summary}"
    );
    assert_eq!(
        summary.lines().count(),
        1,
        "不给开关 ⇒ summary() 必须只有既有的那一行:\n{summary}"
    );
    assert!(
        summary.starts_with("WAV 已写出:"),
        "既有首行逐字不变:\n{summary}"
    );
    assert!(path.exists(), "默认导出必须落盘");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn silence_cannot_reach_a_target() {
    let dir = scratch_dir("silence");
    // 零振幅正弦：音频片段源已注册、渲染成功，但样本全是 0 ⇒ 静音。
    // ⛔ 不用"只有主总线"的工程：它在渲染计划里就失败（`没有注册样本源`），
    // 测不到响度那一层（见上一轮实测）。
    let (project, assets) = sine_clip_project(&dir, 0.0);
    let assets_slice: Vec<(AssetHash, Vec<u8>)> = assets
        .iter()
        .map(|(hash, bytes)| (hash.clone(), bytes.clone()))
        .collect();
    let path = dir.join("silence.wav");
    let options = WavExportOptions {
        frames: 48_000,
        loudness: Some(ExportPreset::new(Some(-16.0), None)),
        ..Default::default()
    };
    let result = export_wav_with_sfz_file_and_assets_under(
        &project,
        &path,
        options,
        None,
        &assets_slice,
        None,
    );
    match result {
        Err(WavExportError::Loudness(reason)) => {
            println!("### 静音响亮失败（预期）: {reason}");
        }
        Err(other) => panic!("静音 ＋ 有目标必须是 Loudness 失败，实际是: {other}"),
        Ok(report) => panic!("静音 ＋ 有目标必须失败，实际导出了: {:?}", report.summary()),
    }
    assert!(!path.exists(), "响亮失败时不得留半成品文件");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn unsupported_sample_rate_fails_loudly() {
    let dir = scratch_dir("rate");
    let project = project_with_tracks(&[("Lead", 0.0)]);
    let path = dir.join("rate.wav");
    let options = WavExportOptions {
        frames: 48_000,
        // 22_050 Hz 没有 K 计权系数：用最小音源（不走引擎乐器），让失败只来自响度测量。
        sample_rate: 22_050,
        use_engine_source: false,
        loudness: Some(ExportPreset::new(Some(-16.0), None)),
        ..Default::default()
    };
    let result =
        export_wav_with_sfz_file_and_assets_under(&project, &path, options, None, &[], None);
    match result {
        Err(WavExportError::Loudness(reason)) => {
            println!("### 不支持的采样率响亮失败（预期）: {reason}");
        }
        Err(other) => panic!("无 K 系数的采样率必须是 Loudness 失败，实际是: {other}"),
        Ok(report) => panic!(
            "无 K 系数的采样率必须失败，实际导出了: {:?}",
            report.summary()
        ),
    }
    assert!(!path.exists(), "响亮失败时不得留半成品文件");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn non_stereo_is_refused_when_loudness_is_requested() {
    let dir = scratch_dir("mono");
    let project = project_with_tracks(&[("Lead", 0.0)]);
    let path = dir.join("mono.wav");
    let options = WavExportOptions {
        frames: 48_000,
        channels: 1,
        use_engine_source: false,
        loudness: Some(ExportPreset::new(Some(-16.0), None)),
        ..Default::default()
    };
    let result =
        export_wav_with_sfz_file_and_assets_under(&project, &path, options, None, &[], None);
    match result {
        Err(WavExportError::Loudness(reason)) => {
            println!("### 单声道响亮失败（预期）: {reason}");
        }
        Err(other) => panic!("单声道 ＋ 给开关必须是 Loudness 失败，实际是: {other}"),
        Ok(report) => panic!(
            "单声道 ＋ 给开关必须失败，实际导出了: {:?}",
            report.summary()
        ),
    }
    assert!(!path.exists(), "响亮失败时不得留半成品文件");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_real_cli_prints_the_loudness_line_only_when_the_switches_are_given() {
    let dir = scratch_dir("cli-binary");
    let project = project_with_tracks(&[("Lead", 0.0)]);
    let container = write_container(&dir, "lead.yeban", &project);

    // 给开关 ⇒ stdout 多一行 `loudness:`（目标／实测／谁赢了都在行里）。
    let out = dir.join("loud.wav");
    let (code, stdout, stderr) = run_cli(&[
        "--open",
        container.to_str().expect("utf8"),
        "--export-wav",
        out.to_str().expect("utf8"),
        "--wav-seconds",
        "1",
        "--loudness-target",
        "-16",
    ]);
    assert_eq!(code, 0, "带响度目标的导出必须成功; stderr={stderr}");
    assert!(
        stdout.lines().any(|line| line == "headless ok"),
        "必须走无窗口批处理路径:\n{stdout}"
    );
    let note = stdout
        .lines()
        .find(|line| line.starts_with("loudness:"))
        .unwrap_or_else(|| panic!("给开关时 stdout 必须有 loudness: 行:\n{stdout}"));
    println!("### 真 CLI 响度行（逐字）:\n{note}");
    assert!(note.contains("target=-16.0"), "行里必须有目标: {note}");
    assert!(note.contains("achieved="), "行里必须有实测值: {note}");
    assert!(
        note.contains("bound=LoudnessTarget"),
        "行里必须写谁赢了: {note}"
    );

    // 不给开关 ⇒ stdout 一行都不多（没有 `loudness:` 行）。
    let plain_out = dir.join("plain.wav");
    let (code, stdout, stderr) = run_cli(&[
        "--open",
        container.to_str().expect("utf8"),
        "--export-wav",
        plain_out.to_str().expect("utf8"),
        "--wav-seconds",
        "1",
    ]);
    assert_eq!(code, 0, "默认导出必须成功; stderr={stderr}");
    assert!(
        stdout.lines().any(|line| line.starts_with("exported-wav:")),
        "既有报告行必须还在:\n{stdout}"
    );
    assert!(
        !stdout.contains("loudness:"),
        "不给开关时 stdout 不得出现响度行（既有文本判据不破）:\n{stdout}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 导出侧容差与 MCP `render_master` 的判定容差同值。
///
/// 两仓的口径刻意不共享常量（见 `yeban-render` 的 [`LOUDNESS_TARGET_TOLERANCE_LU`] 文档）：
/// 一侧改值，另一侧这条判据立即红。只在 `in-process-mcp` 档编译（默认构建里没有
/// `yeban-mcp` 这条依赖边，见 `crates/yeban-app/Cargo.toml` 的 `in-process-mcp` 段）。
#[cfg(feature = "in-process-mcp")]
#[test]
fn the_loudness_tolerance_matches_the_mcp_verdict() {
    assert_eq!(
        LOUDNESS_TARGET_TOLERANCE_LU,
        yeban_mcp::domain::render::TARGET_TOLERANCE_LU,
        "导出侧容差必须等于 MCP 判定容差（两仓同口径）"
    );
}
