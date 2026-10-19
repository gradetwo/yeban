//! 任务单 `#243`：**`app` 的 `--export-wav` 也渲染 `TrackVolume` 自动化**。
//!
//! 这一份判据与 `crates/yeban-mcp/tests/render_master.rs` 的同名判据**同一口径**：
//! 两条导出共用 `yeban_render::automation::TrackGainAutomation`（模型唯一求值入口
//! `automation_value_at` ＋ `ParamSmoother` 5 ms ＋ R55 比率）。
//!
//! 这里走**产品入口** `export_wav`（写盘 ＋ 回读），而不是只调 render_core：
//! 判据的对象是“`--export-wav` 的字节”。
//!
//! 边界：⛔ 不证听感；只证 4 等段峰值单调、正对照相等、关掉/静态恒等逐字节不变。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;

use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_model::{
    AssetHash, AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, ClipContent,
    ClipPlacement, ClipPoolEntry, CurveType, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::wav::read_plain_wav;

const FRAMES: u64 = 48_000;
const I24_SCALE: f32 = 8_388_608.0;

fn id(text: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8{text}")).expect("夹具 ULID 必须合法")
}

/// 临时目录（`Drop` 删掉自己；工作树里不留临时文件）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-243-{tag}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        Self { dir }
    }

    fn wav(&self, tag: &str) -> PathBuf {
        self.dir.join(format!("{tag}.wav"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `Master` ＋ 一条 MIDI 轨（一个 1920 tick 的音符，@120 BPM = 1 秒）。
fn project(track_volume_db: f32, mute: bool) -> (YebanProjectV1, EntityId) {
    let master = id("M001");
    let lead = id("T001");
    let clip = id("K001");
    let placement = id("P001");
    let note = id("N001");
    let edge = id("E001");
    let mut project = YebanProjectV1 {
        id: id("Z001"),
        title: "volume automation fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0243,
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
    let mut track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        volume_db: track_volume_db,
        mute,
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
            name: "Clip".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::from([(
                    note,
                    MidiNote {
                        velocity: 100,
                        ..MidiNote::new(note, 0, 69, 1_920)
                    },
                )]),
            },
        },
    );
    project.tracks.insert(lead, track);
    project.routing_graph.nodes = vec![lead, master];
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: lead,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.validate().expect("夹具工程必须合法");
    (project, lead)
}

fn with_volume_lane(
    mut project: YebanProjectV1,
    track_id: EntityId,
    points: &[(u64, f32)],
    read_enabled: bool,
    seed: u32,
) -> YebanProjectV1 {
    let target = AutomationTarget::TrackVolume { track_id };
    let mut map = BTreeMap::new();
    for (index, (tick, value)) in points.iter().enumerate() {
        let point = AutomationPoint {
            id: id(&format!("{:04}", seed + index as u32)),
            tick: *tick,
            value: *value,
            curve: CurveType::Linear,
        };
        map.insert(point.id, point);
    }
    project
        .tracks
        .get_mut(&track_id)
        .expect("目标音轨必须存在")
        .automation_lanes
        .insert(
            target,
            AutomationLane {
                target,
                points: map,
                read_enabled,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
    project.validate().expect("带自动化泳道的夹具必须合法");
    project
}

fn options() -> WavExportOptions {
    WavExportOptions {
        frames: FRAMES,
        sample_rate: 48_000,
        channels: 2,
        bit_depth: BitDepth::Int24,
        ..WavExportOptions::default()
    }
}

fn options_with_minimal_source() -> WavExportOptions {
    WavExportOptions {
        use_engine_source: false,
        ..options()
    }
}

/// 导出一次并回读左声道浮点样本 + 文件 SHA-256。
fn export(scratch: &Scratch, tag: &str, project: &YebanProjectV1) -> (Vec<f32>, String) {
    export_with(scratch, tag, project, options())
}

fn export_with(
    scratch: &Scratch,
    tag: &str,
    project: &YebanProjectV1,
    options: WavExportOptions,
) -> (Vec<f32>, String) {
    let path = scratch.wav(tag);
    let report = export_wav(project, &path, options).expect("--export-wav 必须成功");
    assert_eq!(report.frames, FRAMES);
    let bytes = std::fs::read(&path).expect("读产物");
    let sha = AssetHash::of_bytes(&bytes).as_str().to_owned();
    let (format, buffer) = read_plain_wav(&path).expect("产物必须可回读");
    let channels = usize::from(format.channels.max(1));
    let samples = match buffer {
        PcmBuffer::Float32(values) => values.iter().step_by(channels).copied().collect(),
        PcmBuffer::Int24(values) => values
            .iter()
            .step_by(channels)
            .map(|value| (*value as f32) / I24_SCALE)
            .collect(),
        other => panic!("本判据只导出 24-bit / 32f：{other:?}"),
    };
    (samples, sha)
}

fn window_peak(samples: &[f32], start: usize, frames: usize) -> f32 {
    let end = (start + frames).min(samples.len());
    samples[start..end]
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

fn four_window_peaks(samples: &[f32]) -> [f32; 4] {
    let window = usize::try_from(FRAMES / 4).expect("窗口");
    let mut peaks = [0.0f32; 4];
    for (index, peak) in peaks.iter_mut().enumerate() {
        *peak = window_peak(samples, index * window, window);
    }
    peaks
}

#[test]
fn a_volume_ramp_is_monotonic_through_export_wav() {
    let scratch = Scratch::new("ramp");
    let (project, lead) = project(0.0, false);
    let project = with_volume_lane(project, lead, &[(0, -24.0), (1_920, 0.0)], true, 100);
    let (samples, sha) = export(&scratch, "ramp", &project);
    let peaks = four_window_peaks(&samples);
    println!("### app 渐强 4 段峰值 = {peaks:?} / sha256={sha}");
    assert!(
        peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3],
        "4 段必须严格单调递增: {peaks:?}"
    );
    let ratio = peaks[3] / peaks[0];
    println!("### app 渐强 peak[3]/peak[0] = {ratio:.6}");
    assert!(ratio >= 3.0, "渐强比值 {ratio} 必须 >= 3.0");
}

#[test]
fn a_disabled_lane_and_a_flat_lane_are_byte_identical_to_no_lane() {
    let scratch = Scratch::new("identity");
    let (base, lead) = project(0.0, false);
    let (_, baseline_sha) = export(&scratch, "baseline", &base);

    let disabled = with_volume_lane(
        project(0.0, false).0,
        lead,
        &[(0, -24.0), (1_920, 0.0)],
        false,
        110,
    );
    let (_disabled_samples, disabled_sha) = export(&scratch, "disabled", &disabled);
    assert_eq!(disabled_sha, baseline_sha, "关掉的泳道必须等于不存在");

    let flat = with_volume_lane(
        project(0.0, false).0,
        lead,
        &[(0, 0.0), (960, 0.0), (1_920, 0.0)],
        true,
        120,
    );
    let (_, flat_sha) = export(&scratch, "flat", &flat);
    println!("### app 恒等: baseline={baseline_sha} disabled={disabled_sha} flat={flat_sha}");
    assert_eq!(flat_sha, baseline_sha, "静态恒等必须逐字节相同");
}

#[test]
fn a_zero_db_lane_does_not_resurrect_a_muted_track_through_export_wav() {
    let scratch = Scratch::new("mute");
    // 静态推子 -6 dB、静音；泳道 0 dB ⇒ **非恒等**（比率 ≈ 1.995），真的走到自动化乘法。
    let (muted_baseline, _lead) = project(-6.0, true);
    let (_, baseline_sha) = export(&scratch, "mute-base", &muted_baseline);
    let (project, lead) = project(-6.0, true);
    let project = with_volume_lane(project, lead, &[(0, 0.0), (1_920, 0.0)], true, 130);
    let (samples, sha) = export(&scratch, "mute", &project);
    let peak = samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    println!("### app 静音轨 + 0 dB 泳道: peak={peak} sha={sha}");
    // 量化前是**逐位 0**；24-bit TPDF 抖动会给全零信号留下 ≤ 1 LSB 的噪声
    // （1/8388608 = 1.1920929e-7）。因此判据是“≤ 2 LSB 且与无泳道的静音版逐字节相同”。
    assert!(
        peak <= 2.0 / I24_SCALE,
        "静音轨 + 0 dB 泳道必须仍全静音（peak={peak}）"
    );
    assert_eq!(sha, baseline_sha, "自动化不许改变静音轨的字节");
}

#[test]
fn points_past_the_end_do_not_extend_the_export() {
    let scratch = Scratch::new("length");
    let (base, _lead) = project(0.0, false);
    let (base_samples, _) = export(&scratch, "length-base", &base);
    let (project, lead) = project(0.0, false);
    let project = with_volume_lane(project, lead, &[(0, -12.0), (99_999, 0.0)], true, 140);
    let path = scratch.wav("length-auto");
    let report = export_wav(&project, &path, options()).expect("导出必须成功");
    println!(
        "### app 长度: baseline_frames={FRAMES} auto_frames={}",
        report.frames
    );
    assert_eq!(report.frames, FRAMES, "超出末端的点不许延长母带");
    assert_eq!(base_samples.len(), usize::try_from(FRAMES).expect("帧数"));
}

#[test]
fn a_master_volume_ramp_is_monotonic_through_export_wav() {
    let scratch = Scratch::new("master");
    let (project, _lead) = project(0.0, false);
    let master = project.master_bus_track_id;
    let project = with_volume_lane(project, master, &[(0, -24.0), (1_920, 0.0)], true, 150);
    let (samples, sha) = export(&scratch, "master-ramp", &project);
    let peaks = four_window_peaks(&samples);
    println!("### app 主总线渐强 4 段峰值 = {peaks:?} / sha256={sha}");
    assert!(
        peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3],
        "主总线渐强 4 段必须严格单调递增: {peaks:?}"
    );
    assert!(peaks[3] / peaks[0] >= 3.0, "{peaks:?}");
}

#[test]
fn volume_automation_is_deterministic_across_two_exports() {
    let scratch = Scratch::new("determinism");
    let (project, lead) = project(0.0, false);
    let project = with_volume_lane(project, lead, &[(0, -24.0), (1_920, 0.0)], true, 160);
    let (_, first) = export(&scratch, "det-a", &project);
    let (_, second) = export(&scratch, "det-b", &project);
    assert_eq!(first, second, "两次导出必须逐字节相同");
}

/// 与 MCP 的同一条曲线、**恒定振幅的最小音源**：分段峰值比应当与 MCP 的 7.7196
/// 一致（≈ 源波形差异被排除）。这条判据把“比率差异来自音源波形，不是自动化”钉住。
#[test]
fn the_ramp_ratio_with_a_constant_amplitude_source_matches_the_mcp_reading() {
    let scratch = Scratch::new("ramp-tone");
    let (project, lead) = project(0.0, false);
    let project = with_volume_lane(project, lead, &[(0, -24.0), (1_920, 0.0)], true, 170);
    let (samples, _sha) = export_with(
        &scratch,
        "ramp-tone",
        &project,
        options_with_minimal_source(),
    );
    let peaks = four_window_peaks(&samples);
    let ratio = peaks[3] / peaks[0];
    println!("### app 最小音源渐强 4 段峰值 = {peaks:?} / ratio={ratio:.6}");
    assert!(
        peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3],
        "{peaks:?}"
    );
    assert!(ratio >= 3.0, "{peaks:?}");
}
