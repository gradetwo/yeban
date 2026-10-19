//! 任务单 `#226`：**CLI 的 `--export-wav` 也报削波 ＋ 建议 trim**（与 MCP `#225` 同一份口径）。
//!
//! # 判据与文件头的对应
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | [`a_clipping_project_reports_clipped_samples_and_a_suggested_trim`] | 削波工程 ⇒ 四个读数 ＋ `summary()` 的**第二行**（含 `clippedSamples` / `suggestedTrimDb`） |
//! | [`a_clean_project_keeps_the_legacy_summary_byte_for_byte`] | ⛔ 不削波 ⇒ `summary()` **逐字不变**（既有文本判据不破） |
//! | [`a_clean_export_is_byte_identical_when_repeated`] | 干净夹具**连导两次** ⇒ 整文件 `sha256` / 母带 `digest` 逐字节相同（**自比**） |
//! | [`reporting_readings_does_not_change_a_single_byte`] | 削波夹具**连导两次** ⇒ 整文件 `sha256` / 母带 `digest` 逐字节相同（**自比**；绝对字节是机器相关的） |
//! | [`a_single_source_below_full_scale_can_still_clip_the_sum`] | 最响源 ≠ 过载原因：单源 `peakDbfs <= 0` 但母带削波 ⇒ 逐源建议 `0.0`、文案逐字说"多源求和" |
//! | [`the_readings_match_the_mcp_side_on_the_same_audio`] | 同一段音频（音频片段夹具）CLI vs MCP 的 `clippedSamples` / `truePeakDbfs` / `suggestedTrimDb` **逐项相等** |
//! | [`the_real_cli_prints_the_warning_only_when_the_master_clips`] | 真二进制：削波 ⇒ stdout 多一行；不削波 ⇒ stdout 逐字不变 |
//!
//! # 口径
//!
//! - 夹具是**代码里构造的最小工程**（master ＋ N 条 MIDI 轨，音符完全相同），⛔ 不读仓库外文件；
//! - 读数全部来自 [`yeban_app::export_wav`] 的**真导出产物**（不是手算的假数）；
//! - "不改音频"用**同一台机器上的自比**钉住：同一份夹具**连导两次**，整文件 `sha256` 与母带
//!   `digest` 必须逐字节相同（见 [`reporting_readings_does_not_change_a_single_byte`] 与
//!   [`a_clean_export_is_byte_identical_when_repeated`]）。
//! - ⭐ **绝对字节是机器相关的**：不同平台/CPU 的 `sin`/`libm`/FMA 会让渲染出的样本位型不同
//!   ⇒ WAV 的 `sha256` 与母带 `digest` 也不同（台账第 313 条）。⛔ 因此本文件**不**钉任何
//!   WAV 的绝对哈希常量 —— 那种常量只描述"某一台机器上的值"，换一台机器就红（`#232`）。
//!   自比证明的是**可跨平台成立**的那一半：同一二进制、同一机器、同一输入 ⇒ 同一串字节。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use yeban_app::export_wav::{
    WavExportOptions, export_wav, export_wav_with_sfz_file_and_assets_under,
};
use yeban_model::container::write_project_container;
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, CommitGraph, EntityId, LoopConfig,
    MidiNote, RoutingEdge, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

// ⚠ 这里**没有** WAV 的绝对 `sha256`/`digest` 常量：绝对字节是机器相关的（第 313 条），
//   判据一律用同一台机器上的**自比**（导出两次，比两次的字节/摘要）。

/// ⭐ MCP 侧（`yeban-mcp --stdio` → `yeban_render_master`）在**同一份**音频片段夹具上
/// 实测的三项读数（`#226` 的证据；夹具与 [`audio_clip_project`] 逐字节同源）。
///
/// 为什么它能当"两侧同口径"的锚：这份夹具的母带**只由音频片段构成**（没有 MIDI 音符），
/// 而 app 与 MCP 的音频片段语义是**逐条对齐**的（见 `crate::audio_clip` 的模块文档）
/// ⇒ 两侧算的是同一段样本，读数必须逐项相等。MIDI 夹具做不到这一点（MCP 走自己的
/// 确定性振荡器，app 走引擎乐器）—— 那是**渲染路径**不同，不是读数口径不同。
const MCP_CLIPPED_SAMPLES: u64 = 63_840;
/// 见 [`MCP_CLIPPED_SAMPLES`]。
const MCP_TRUE_PEAK_DBFS: f32 = 5.979_400_6;
/// 见 [`MCP_CLIPPED_SAMPLES`]。
const MCP_SUGGESTED_TRIM_DB: f32 = -6.0;
/// 见 [`MCP_CLIPPED_SAMPLES`]（最响源的线性峰值）。
const MCP_LOUDEST_PEAK: f32 = 1.990_536;

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-226-{tag}-{}-{}",
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

/// 每个 `(名字, volume_db)` 一条 MIDI 轨；音符**完全相同**（同音高／同时值／力度 127）。
///
/// 音符相同是刻意的：多轨时它们**相干求和**，于是"单条源没过满量程、合奏却过载"可以
/// 用两个 −3 dB 的轨精确造出来（[`a_single_source_below_full_scale_can_still_clip_the_sum`]）。
fn project_with_tracks(tracks: &[(&str, f32)]) -> YebanProjectV1 {
    let master = fixture_id("M1");
    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "cli clipping fixture".to_owned(),
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

/// 音频片段夹具：**没有 MIDI 音符**，母带只由一段 440 Hz 浮点样本构成（片段增益 +12 dB）。
///
/// 返回 `(工程, 资产池)`。为什么用 **Float32** 素材：app 的 `read_plain_wav` 与 MCP 的
/// `yeban_decode` 对浮点样本都**原样直用** ⇒ 两侧拿到的是逐位相同的 PCM（整数素材会在
/// 归一化分母上差 1 LSB：app 用 `i16::MAX`、symphonia 用 `2^15`）。
fn audio_clip_project(dir: &Path) -> (YebanProjectV1, BTreeMap<AssetHash, Vec<u8>>) {
    let asset_path = dir.join("asset-f32.wav");
    let frames = 48_000_usize;
    let mut samples: Vec<f32> = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        let phase = std::f64::consts::TAU * 440.0 * frame as f64 / 48_000.0;
        let value = (phase.sin() * 0.5) as f32;
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
        title: "audio clip clipping fixture".to_owned(),
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
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "sine-f32".to_owned(),
            content: ClipContent::Audio {
                asset,
                gain_db: 12.0,
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
    project.validate().expect("音频片段夹具必须合法");
    (project, assets)
}

/// 公式（逐字）：`suggestedTrimDb = -(ceil(truePeakDbfs * 2) / 2)`。
fn formula(true_peak_dbfs: f32) -> f32 {
    -((true_peak_dbfs * 2.0).ceil() / 2.0)
}

// ---------------------------------------------------------------------------
// ① 削波工程 ⇒ CLI 有提示
// ---------------------------------------------------------------------------

#[test]
fn a_clipping_project_reports_clipped_samples_and_a_suggested_trim() {
    let dir = scratch_dir("clip");
    let project = project_with_tracks(&[("Lead", 24.0)]);
    let path = dir.join("clip.wav");
    let report = export_wav(&project, &path, WavExportOptions::default()).expect("导出必须成功");

    println!(
        "### #226 削波工程读数（逐字）: clippedSamples={} truePeakDbfs={:?} headroomDb={:?} suggestedTrimDb={} basis={} loudest={:?}",
        report.clipped_samples,
        report.true_peak_dbfs,
        report.headroom_db,
        report.suggested_trim_db,
        report.trim_basis,
        report.loudest_source.as_ref().map(|source| (
            source.track.clone(),
            source.peak,
            source.peak_dbfs,
            source.suggested_source_trim_db
        )),
    );
    println!("### #226 summary() 逐字:\n{}", report.summary());

    // 判据 1：削波工程 ⇒ 读数非零、且**真的**过满量程。
    assert!(
        report.clipped_samples > 0,
        "过载夹具必须报出被 24-bit 量化器钳住的样本数"
    );
    let true_peak = report.true_peak_dbfs.expect("真峰值必须可测");
    assert!(true_peak > 0.0, "过载 ⇒ truePeakDbfs > 0，实测 {true_peak}");
    assert_eq!(report.headroom_db, Some(-true_peak), "headroom = -truePeak");
    assert!(report.headroom_db.is_some_and(|value| value < 0.0));

    // 判据 4：建议值**可算** —— 与公式逐字对账，且"减完不削波、最多多减 0.5 dB"。
    assert_eq!(report.trim_basis, "truePeakDbfs");
    assert_eq!(
        report.suggested_trim_db,
        formula(true_peak),
        "建议值必须等于公式（suggestedTrimDb = -(ceil(truePeakDbfs * 2) / 2)）"
    );
    assert!(
        true_peak + report.suggested_trim_db <= 1.0e-6,
        "按建议减完不得再削波: truePeak={true_peak}, trim={}",
        report.suggested_trim_db
    );
    assert!(
        true_peak + report.suggested_trim_db >= -0.5 - 1.0e-6,
        "最多多减 0.5 dB: truePeak={true_peak}, trim={}",
        report.suggested_trim_db
    );

    // 判据 2：最响的源被点名，且它自己也过了满量程 ⇒ 逐源建议是负的。
    let loudest = report.loudest_source.as_ref().expect("必须有最响的源");
    assert_eq!(loudest.track, "Lead");
    assert_eq!(loudest.node, fixture_id("T0"));
    assert!(loudest.peak > 1.0, "单源夹具里最响源自己也过载");
    assert!(loudest.peak_dbfs.is_some_and(|value| value > 0.0));
    assert!(
        loudest
            .suggested_source_trim_db
            .is_some_and(|value| value < 0.0),
        "自己过载 ⇒ 逐源建议必须是负的"
    );

    // 判据 1：`summary()` 在削波时**多一行**，且含两个关键读数。
    let summary = report.summary();
    let lines: Vec<&str> = summary.lines().collect();
    assert_eq!(lines.len(), 2, "削波时 summary() 必须恰好两行: {summary}");
    assert!(lines[0].starts_with("WAV 已写出: "), "{}", lines[0]);
    let note = lines[1];
    assert!(note.contains("clippedSamples"), "{note}");
    assert!(note.contains("suggestedTrimDb"), "{note}");
    assert!(note.contains("最响的源"), "{note}");
    assert!(note.contains("不自动改增益"), "{note}");
    assert!(note.contains("-24"), "建议 trim 必须进文案: {note}");

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// ② 不削波 ⇒ 文本不变
// ---------------------------------------------------------------------------

#[test]
fn a_clean_project_keeps_the_legacy_summary_byte_for_byte() {
    let dir = scratch_dir("clean");
    let project = project_with_tracks(&[("Lead", 0.0)]);
    let path = dir.join("clean.wav");
    let report = export_wav(&project, &path, WavExportOptions::default()).expect("导出必须成功");

    println!(
        "### #226 不削波读数（逐字）: clippedSamples={} truePeakDbfs={:?} headroomDb={:?} suggestedTrimDb={}",
        report.clipped_samples, report.true_peak_dbfs, report.headroom_db, report.suggested_trim_db
    );
    println!("### #226 不削波 summary() 逐字:\n{}", report.summary());

    assert_eq!(report.clipped_samples, 0, "干净夹具不得报削波样本");
    assert_eq!(report.suggested_trim_db, 0.0, "不削波 ⇒ 建议 trim 是 0");
    assert!(report.clipping_note().is_none(), "不削波 ⇒ 没有提示行");
    assert!(!report.summary().contains("削波"));

    // 逐字：既有摘要**一个字符都不变**（不削波时 summary 只有原来的那一行）。
    let legacy = format!(
        "WAV 已写出: {}（{} 帧 / {} Hz / {} 声道 / {} bit / {} 字节 / 音源 {} / digest {}）",
        report.path.display(),
        report.frames,
        report.sample_rate,
        report.channels,
        report.bits,
        report.bytes,
        report.source,
        report.digest
    );
    assert_eq!(report.summary(), legacy);
    assert_eq!(report.summary().lines().count(), 1);

    // ⚠ 这里**不**比 WAV 的绝对 `sha256`：绝对字节是机器相关的（第 313 条）。
    //    字节侧的事实由 [`a_clean_export_is_byte_identical_when_repeated`] 用**自比**钉住；
    //    本判据只钉**文本** —— 不削波时 `summary()` 与旧格式逐字相同，这一条跨平台成立。
    std::fs::remove_dir_all(&dir).ok();
}

/// 干净夹具的**字节自比**：同一份工程连导两次，整文件 `sha256` 与母带 `digest` 逐字节相同。
///
/// 为什么是自比而不是绝对常量：样本位型来自 `sin`/`libm`/FMA，**机器相关**（第 313 条）。
/// 绝对常量只能证明"某一台机器上是这个值"，换一台机器就红（`#232`：Linux/Windows/macOS
/// 三个平台三个 `sha256`）；自比证明的是**可跨平台成立**的那一半 —— 同一二进制、同一机器、
/// 同一输入 ⇒ 同一串字节。
///
/// ⚠ 边界：它**不**证明"与 `#226` 之前的历史版本逐字节相同"。那需要一台机器上的绝对锚，
/// 而绝对锚跨平台不成立；本判据如实只声明"连导两次不变"。
#[test]
fn a_clean_export_is_byte_identical_when_repeated() {
    let dir = scratch_dir("clean-bytes");
    let project = project_with_tracks(&[("Lead", 0.0)]);
    let path = dir.join("clean-bytes.wav");

    let before =
        export_wav(&project, &path, WavExportOptions::default()).expect("第一次导出必须成功");
    assert_eq!(before.clipped_samples, 0, "干净夹具不得报削波样本");
    let sha_before = sha_hex(&path);
    let digest_before = before.digest.clone();
    let bytes_before = before.bytes;

    let after =
        export_wav(&project, &path, WavExportOptions::default()).expect("第二次导出必须成功");
    assert_eq!(after.clipped_samples, 0, "第二次也必须不削波");
    let sha_after = sha_hex(&path);
    let digest_after = after.digest.clone();
    let bytes_after = after.bytes;

    println!(
        "### #226 干净自比: sha_before={sha_before} sha_after={sha_after} digest_before={digest_before} digest_after={digest_after} bytes_before={bytes_before} bytes_after={bytes_after}"
    );
    assert_eq!(
        sha_before, sha_after,
        "同一台机器连导两次，整文件 sha256 必须逐字节相同"
    );
    assert_eq!(
        digest_before, digest_after,
        "母带样本的位级摘要也必须逐位相同"
    );
    assert_eq!(bytes_before, bytes_after, "字节数必须相同");
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// ③ ⛔ 不改音频
// ---------------------------------------------------------------------------

/// 同一份削波夹具**连导两次**（同一二进制、同一路径）的整文件 `sha256` / 母带 `digest`
/// 必须逐字节相同 —— 这是"读数不写样本"的可跨平台口径。
///
/// # 为什么是自比，而不是钉常量
///
/// **绝对字节是机器相关的**（第 313 条）：样本位型来自 `sin`/`libm`/FMA，不同平台的
/// 结果可以在低位不同 ⇒ WAV 的 `sha256` 与母带 `digest` 也随之不同。`#226` 曾在 macOS 上
/// 量到 `36918563…` / `93827504…` 并钉成常量，Linux 与 Windows 立刻红（`#232`：同一个夹具
/// 在三个平台产出三个不同的 `sha256`）。⇒ 判据改成**自比**：
///
/// 1. 第一次导出 ⇒ `sha_before` / `digest_before`；
/// 2. 第二次导出 ⇒ `sha_after` / `digest_after`；
/// 3. 断言两次相等，⛔ 不与任何绝对常量比。
///
/// # 它证明什么、不证明什么
///
/// - 证明：读数只从同一份渲染结果里读；**同一二进制、同一机器、同一输入 ⇒ 同一串字节**
///   （两次导出里读数都参与，第二次没有因为"已经量过一次"而改样本）。
/// - ⛔ 不证明："与 `#226` 之前的历史版本逐字节相同"。那需要一台机器上的绝对锚，而绝对锚
///   跨平台不成立 —— 那条断言本来就不该是跨平台判据；它的跨平台部分只能是自比。
#[test]
fn reporting_readings_does_not_change_a_single_byte() {
    let dir = scratch_dir("bytes");
    let project = project_with_tracks(&[("Lead", 24.0)]);
    let path = dir.join("bytes.wav");

    let before =
        export_wav(&project, &path, WavExportOptions::default()).expect("第一次导出必须成功");
    assert!(
        before.clipped_samples > 0,
        "这一条必须在**真有一条读数**的产物上量字节"
    );
    let sha_before = sha_hex(&path);
    let digest_before = before.digest.clone();
    let bytes_before = before.bytes;

    let after =
        export_wav(&project, &path, WavExportOptions::default()).expect("第二次导出必须成功");
    assert!(
        after.clipped_samples > 0,
        "第二次也必须真有一条读数（⛔ 不是空夹具上的恒等）"
    );
    let sha_after = sha_hex(&path);
    let digest_after = after.digest.clone();
    let bytes_after = after.bytes;

    println!(
        "### #226 不改音频自比: sha_before={sha_before} sha_after={sha_after} digest_before={digest_before} digest_after={digest_after} bytes_before={bytes_before} bytes_after={bytes_after}"
    );
    assert_eq!(
        sha_before, sha_after,
        "同一台机器连导两次，整文件 sha256 必须逐字节相同"
    );
    assert_eq!(
        digest_before, digest_after,
        "母带样本的位级摘要也必须逐位相同"
    );
    // 结构事实（⛔ 不是指纹）：384000 帧 × 2 声道 × 3 字节 ＋ 68 字节 RIFF 头。
    assert_eq!(bytes_before, 2_304_068, "字节数是结构事实，跨平台成立");
    assert_eq!(bytes_after, bytes_before, "两次导出的字节数必须相同");
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// ④ 最响源 ≠ 过载原因
// ---------------------------------------------------------------------------

/// 两条 −3 dB 的轨（单源峰值 ≈ 0.698 ＝ −3.12 dBFS）相干求和 ⇒ 母带真峰值 ≈ 2.907 dBFS。
///
/// 判据：单源**没过满量程** ⇒ 逐源建议必须是 `0.0`，且文案逐字说"过载来自多源求和、
/// 单独减这一条不会解决"。
#[test]
fn a_single_source_below_full_scale_can_still_clip_the_sum() {
    let dir = scratch_dir("sum");
    let project = project_with_tracks(&[("Lead", -3.0), ("Pad", -3.0)]);
    let path = dir.join("sum.wav");
    let report = export_wav(&project, &path, WavExportOptions::default()).expect("导出必须成功");
    let note = report.clipping_note().expect("合奏过载必须有一条提示");
    println!(
        "### #226 求和过载读数（逐字）: clippedSamples={} truePeakDbfs={:?} suggestedTrimDb={}",
        report.clipped_samples, report.true_peak_dbfs, report.suggested_trim_db
    );
    println!("### #226 hint 逐字:\n{note}");

    assert!(
        report.clipped_samples > 0 && report.true_peak_dbfs.is_some_and(|value| value > 0.0),
        "夹具必须让母带过载"
    );
    let loudest = report.loudest_source.as_ref().expect("最响的源");
    assert!(
        loudest.peak < 1.0,
        "前提：最响的**单条源**自己没过满量程，实测 {}",
        loudest.peak
    );
    assert!(
        loudest.peak_dbfs.is_some_and(|value| value <= 0.0),
        "前提：最响的单源 peakDbfs <= 0，实测 {:?}",
        loudest.peak_dbfs
    );
    assert_eq!(
        loudest.suggested_source_trim_db,
        Some(0.0),
        "单源没过满量程 ⇒ 逐源 trim 建议必须是 0（⛔ 不是负值）"
    );
    assert!(note.contains("多源求和"), "必须说清过载来自求和: {note}");
    assert!(
        note.contains("单独减这一条不会解决"),
        "必须说清单独减最响源没用: {note}"
    );
    assert!(
        note.contains("suggestedTrimDb"),
        "必须给出母带级建议: {note}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// ⑤ 与 MCP 同一段音频的逐项对照
// ---------------------------------------------------------------------------

/// 同一份音频片段夹具：CLI 的三项读数必须与 **MCP 侧**逐项相等
/// （常量见 [`MCP_CLIPPED_SAMPLES`] 的文档）。
///
/// ⚠ 为什么不用 MIDI 夹具：MCP 的 `MidiSynthSource` 与 app 的引擎乐器是**两条渲染路径**，
/// 同一份 MIDI 工程会产出**不同的音频** ⇒ 读数本来就不该相等（那是渲染路径的差异，
/// 不是读数口径的漂移）。音频片段是两侧**语义逐条对齐**的唯一形态 ⇒ 拿它验"同一份口径"。
#[test]
fn the_readings_match_the_mcp_side_on_the_same_audio() {
    let dir = scratch_dir("mcp-parity");
    let (project, assets) = audio_clip_project(&dir);
    let assets_slice: Vec<(AssetHash, Vec<u8>)> = assets
        .iter()
        .map(|(hash, bytes)| (hash.clone(), bytes.clone()))
        .collect();
    let path = dir.join("audio-clip.wav");
    let report = export_wav_with_sfz_file_and_assets_under(
        &project,
        &path,
        WavExportOptions::default(),
        None,
        &assets_slice,
        None,
    )
    .expect("音频片段导出必须成功");
    println!(
        "### #226 CLI 侧: clippedSamples={} truePeakDbfs={:?} suggestedTrimDb={} loudest={:?}",
        report.clipped_samples,
        report.true_peak_dbfs,
        report.suggested_trim_db,
        report.loudest_source.as_ref().map(|source| (
            source.peak,
            source.peak_dbfs,
            source.suggested_source_trim_db
        )),
    );
    println!(
        "### #226 MCP 侧常量: clippedSamples={MCP_CLIPPED_SAMPLES} truePeakDbfs={MCP_TRUE_PEAK_DBFS} suggestedTrimDb={MCP_SUGGESTED_TRIM_DB}"
    );
    assert_eq!(
        report.clipped_samples, MCP_CLIPPED_SAMPLES,
        "clippedSamples 必须与 MCP 逐项相等"
    );
    assert_eq!(
        report.true_peak_dbfs,
        Some(MCP_TRUE_PEAK_DBFS),
        "truePeakDbfs 必须与 MCP 逐项相等"
    );
    assert_eq!(
        report.suggested_trim_db, MCP_SUGGESTED_TRIM_DB,
        "suggestedTrimDb 必须与 MCP 逐项相等"
    );
    let loudest = report.loudest_source.as_ref().expect("最响的源");
    assert_eq!(loudest.peak, MCP_LOUDEST_PEAK);
    assert_eq!(loudest.peak_dbfs, Some(MCP_TRUE_PEAK_DBFS));
    assert_eq!(
        loudest.suggested_source_trim_db,
        Some(MCP_SUGGESTED_TRIM_DB)
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// ⑥ 真二进制：削波时 stdout 多一行；不削波时逐字不变
// ---------------------------------------------------------------------------

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

#[test]
fn the_real_cli_prints_the_warning_only_when_the_master_clips() {
    let dir = scratch_dir("cli-binary");

    // 削波工程 ⇒ stdout 里必须有提示行（含两个关键读数）。
    let clipping = write_container(
        &dir,
        "clipping.yeban",
        &project_with_tracks(&[("Lead", 24.0)]),
    );
    let clipping_out = dir.join("clipping.wav");
    let (code, stdout, stderr) = run_cli(&[
        "--open",
        clipping.to_str().expect("utf8"),
        "--export-wav",
        clipping_out.to_str().expect("utf8"),
    ]);
    assert_eq!(code, 0, "削波导出必须成功; stderr={stderr}");
    assert!(
        stdout.lines().any(|line| line == "headless ok"),
        "必须走无窗口批处理路径:\n{stdout}"
    );
    let note = stdout
        .lines()
        .find(|line| line.contains("clippedSamples") && line.contains("suggestedTrimDb"))
        .unwrap_or_else(|| panic!("削波工程必须打印提示行:\n{stdout}"));
    println!("### #226 真 CLI 提示行（逐字）:\n{note}");
    assert!(note.contains("不自动改增益"), "{note}");
    assert!(note.contains("最响的源"), "{note}");

    // 不削波工程 ⇒ stdout **逐字不变**（没有多出来的提示行）。
    let clean = write_container(&dir, "clean.yeban", &project_with_tracks(&[("Lead", 0.0)]));
    let clean_out = dir.join("clean.wav");
    let (code, stdout, stderr) = run_cli(&[
        "--open",
        clean.to_str().expect("utf8"),
        "--export-wav",
        clean_out.to_str().expect("utf8"),
    ]);
    assert_eq!(code, 0, "干净导出必须成功; stderr={stderr}");
    assert!(
        stdout.lines().any(|line| line.starts_with("exported-wav:")),
        "既有报告行必须还在:\n{stdout}"
    );
    assert!(
        !stdout.contains("clippedSamples") && !stdout.contains("削波"),
        "不削波时 stdout 不得出现提示行（既有文本判据不破）:\n{stdout}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
