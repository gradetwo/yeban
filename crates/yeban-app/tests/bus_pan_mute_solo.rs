//! 任务单 **B2**：总线（有入边的非主轨）的**声相 / mute / solo**。
//!
//! ## 本文件钉住的事实
//!
//! - **app 已绿**（`#296` 的顺带效果）：总线的推子用的是 `track_gain_of`
//!   （＝ `track_gain_scale`，含 `mute`/`solo`/`solo_safe`）⇒ 总线 `mute` / `solo`
//!   在 app 路径上**已经**是整条总线 ×0；本文件把这条既有事实钉住，⛔ 不改语义。
//! - **B2a**：总线静态 `pan` 乘在**整条总线**（自有源 ＋ 入边）上，⛔ 不再只染自有源。
//! - **B2a 泳道**：总线的 `TrackPan` 泳道在输出级逐块生效（`-1 → +1` 前左后右）。
//!
//! ## 与 MCP 的同一条曲线
//!
//! `pan = -0.5` 时 L/R 的期望比是 `tan(3π/8)`（等功率 −3 dB：`L=cos(π/8)`、
//! `R=sin(π/8)`）。同一个夹具在 `crates/yeban-mcp/tests/render_master.rs` 的
//! `a_bus_static_pan_moves_the_whole_bus` 里也量一次；两条路必须给出同一个比。
//!
//! 边界：⛔ 不证听感；只证样本上的 L/R、静音与逐字节正对照。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::render_core::{RenderProjectOutput, render_project_f32_with_assets_under};
use yeban_model::{
    AssetHash, AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, ClipContent,
    ClipPlacement, ClipPoolEntry, CurveType, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingKind, SampleRate, TrackKind, TrackV3, YebanProjectV1,
};

const FRAMES: u64 = 48_000;

fn id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P1{tail}")).expect("夹具 ULID 合法")
}

/// 临时目录（`Drop` 删掉自己；工作树里不留临时文件）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-b2-{tag}-{}-{}",
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

/// `lead → bus → master`：总线是 `AuxReturn`，**没有**自有音源；直连边被删掉。
fn project() -> YebanProjectV1 {
    let master = id("M1");
    let lead = id("T1");
    let bus = id("A0");
    let clip = id("K1");
    let placement = id("P1");
    let note = id("N1");
    let mut project = YebanProjectV1 {
        id: id("Z1"),
        title: "B2 bus pan".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0B20,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.audio_config.sample_rate = SampleRate::Hz48000;
    project.metadata.created_at_unix_ms = 1_700_000_000_000;
    project.metadata.modified_at_unix_ms = 1_700_000_000_000;
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut lead_track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        volume_db: -6.0,
        ..TrackV3::default()
    };
    lead_track.clips.insert(
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
            name: "lead-clip".to_owned(),
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
    project.tracks.insert(lead, lead_track);
    project.tracks.insert(
        bus,
        TrackV3 {
            id: bus,
            name: "Bus".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes = vec![lead, bus, master];
    for (id_tail, source_node, destination_node, kind) in [
        ("E1", lead, bus, RoutingKind::SendToAux),
        ("E2", bus, master, RoutingKind::BusToMaster),
    ] {
        let edge = id(id_tail);
        project.routing_graph.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node,
                destination_node,
                kind,
                gain_db: None,
            },
        );
    }
    project.validate().expect("B2 夹具必须合法");
    project
}

fn render(project: &YebanProjectV1) -> RenderProjectOutput {
    let options = WavExportOptions {
        frames: FRAMES,
        ..WavExportOptions::default()
    };
    render_project_f32_with_assets_under(project, options, None, &[], None)
        .expect("B2 夹具必须能渲染")
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

fn stereo_peaks(samples: &[f32]) -> (f32, f32) {
    let mut left = 0.0f32;
    let mut right = 0.0f32;
    for pair in samples.as_chunks::<2>().0 {
        left = left.max(pair[0].abs());
        right = right.max(pair[1].abs());
    }
    (left, right)
}

fn stereo_window_rms(samples: &[f32], start: usize, frames: usize) -> (f64, f64) {
    let available = samples.len() / 2;
    let mut left = 0.0f64;
    let mut right = 0.0f64;
    let mut count = 0usize;
    for frame in start..(start + frames).min(available) {
        let l = f64::from(samples[frame * 2]);
        let r = f64::from(samples[frame * 2 + 1]);
        left += l * l;
        right += r * r;
        count += 1;
    }
    if count == 0 {
        return (0.0, 0.0);
    }
    ((left / count as f64).sqrt(), (right / count as f64).sqrt())
}

fn with_pan_lane(mut project: YebanProjectV1, track_id: EntityId) -> YebanProjectV1 {
    let target = AutomationTarget::TrackPan { track_id };
    let mut points = BTreeMap::new();
    for (index, (tick, value)) in [(0u64, -1.0f32), (1_920, 1.0)].iter().enumerate() {
        let point = AutomationPoint {
            id: id(if index == 0 { "Q1" } else { "Q2" }),
            tick: *tick,
            value: *value,
            curve: CurveType::Linear,
        };
        points.insert(point.id, point);
    }
    project
        .tracks
        .get_mut(&track_id)
        .expect("Bus")
        .automation_lanes
        .insert(
            target,
            AutomationLane {
                target,
                points,
                read_enabled: true,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
    project.validate().expect("带声相泳道的夹具必须合法");
    project
}

fn export_sha(project: &YebanProjectV1, path: &Path) -> String {
    let options = WavExportOptions {
        frames: FRAMES,
        ..WavExportOptions::default()
    };
    export_wav(project, path, options).expect("导出 WAV");
    let bytes = std::fs::read(path).expect("读产物");
    AssetHash::of_bytes(&bytes).to_string()
}

/// ⭐ 正对照：中性总线（`pan=0` / `mute=solo=false` / 无泳道）与直连工程**逐位相同**。
#[test]
fn a_neutral_bus_is_bit_identical_to_the_direct_project() {
    let bus_project = project();
    let bus_output = render(&bus_project);
    let direct_output = render(&{
        let mut direct = project();
        let edge = id("E2");
        direct.routing_graph.edges.remove(&edge);
        let direct_edge = id("E3");
        direct.routing_graph.edges.insert(
            direct_edge,
            RoutingEdge {
                id: direct_edge,
                source_node: id("T1"),
                destination_node: id("M1"),
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        direct.validate().expect("直连夹具合法");
        direct
    });
    println!(
        "### app 中性总线: bus_digest={} direct_digest={}",
        bus_output.digest, direct_output.digest
    );
    assert_eq!(
        bus_output.digest, direct_output.digest,
        "中性总线的输出级必须恒等（与直连逐位相同）"
    );
}

/// ⭐ B2b（app 已绿）：总线 `mute = true` ⇒ 整条总线输出 0；WAV sha 改变。
#[test]
fn an_app_bus_mute_silences_the_whole_bus() {
    let scratch = Scratch::new("mute");
    let base = project();
    let base_sha = export_sha(&base, &scratch.wav("base"));
    let mut muted = project();
    muted.tracks.get_mut(&id("A0")).expect("Bus").mute = true;
    muted.validate().expect("静音夹具合法");
    let muted_output = render(&muted);
    let muted_peak = peak(&muted_output.samples);
    let muted_sha = export_sha(&muted, &scratch.wav("muted"));
    println!(
        "### app 总线 mute: base_sha={base_sha} muted_sha={muted_sha} peak={muted_peak:.9} \
         base_peak={:.9}",
        peak(&render(&base).samples)
    );
    assert_eq!(muted_peak, 0.0, "总线 mute ⇒ 浮点母带必须恰好 0");
    assert_ne!(muted_sha, base_sha, "总线 mute 必须改变文件字节");
}

/// ⭐ B2c（app 已绿）：源轨 solo ⇒ 未 solo 的总线静音；`solo_safe` ⇒ 仍可闻且逐位等于正对照。
#[test]
fn an_app_bus_is_muted_by_another_tracks_solo_unless_it_is_solo_safe() {
    let base = project();
    let base_output = render(&base);

    let mut soloed = project();
    soloed.tracks.get_mut(&id("T1")).expect("Lead").solo = true;
    soloed.validate().expect("solo 夹具合法");
    let soloed_output = render(&soloed);
    println!(
        "### app 源轨 solo / 总线未 solo: peak={:.9} base_digest={}",
        peak(&soloed_output.samples),
        base_output.digest
    );
    assert_eq!(
        peak(&soloed_output.samples),
        0.0,
        "未 solo 的总线必须被 any_solo 门掉"
    );

    let mut safe = project();
    safe.tracks.get_mut(&id("T1")).expect("Lead").solo = true;
    safe.tracks.get_mut(&id("A0")).expect("Bus").solo_safe = true;
    safe.validate().expect("solo_safe 夹具合法");
    let safe_output = render(&safe);
    println!("### app 总线 solo_safe: digest={}", safe_output.digest);
    assert_eq!(
        safe_output.digest, base_output.digest,
        "solo_safe 的总线 + 唯一源轨 solo ⇒ 与无 solo 正对照逐位相同"
    );
}

/// ⭐ B2a（app）：总线静态 `pan = -0.5` ⇒ 整条总线左重；比值 = `tan(3π/8)`。
#[test]
fn an_app_bus_static_pan_moves_the_whole_bus() {
    let mut project = project();
    project.tracks.get_mut(&id("A0")).expect("Bus").pan = -0.5;
    project.validate().expect("声相夹具合法");
    let output = render(&project);
    let (left, right) = stereo_peaks(&output.samples);
    let ratio = left / right;
    let expected = (3.0_f32 * core::f32::consts::FRAC_PI_8).tan();
    println!(
        "### app 总线静态 pan(-0.5): L={left:.9} R={right:.9} ratio={ratio:.9} \
         expected={expected:.9} digest={}",
        output.digest
    );
    assert!(
        (ratio - expected).abs() < 1.0e-4,
        "app 总线 pan(-0.5) 的 L/R 必须等于 tan(3π/8)={expected}: ratio={ratio}"
    );
}

/// ⭐ B2a（app 泳道）：总线的 `TrackPan` 泳道（`-1 → +1`）在输出级生效。
#[test]
fn an_app_bus_pan_lane_moves_the_whole_bus() {
    let project = with_pan_lane(project(), id("A0"));
    let output = render(&project);
    // 只看两端的 1/4：中间 1/2 正在扫过中心，会把 L/R 比拉平。
    let quarter = FRAMES as usize / 4;
    let (early_l, early_r) = stereo_window_rms(&output.samples, 0, quarter);
    let (late_l, late_r) = stereo_window_rms(&output.samples, FRAMES as usize - quarter, quarter);
    println!(
        "### app 总线 TrackPan 泳道: early L/R={early_l:.9}/{early_r:.9} \
         late L/R={late_l:.9}/{late_r:.9} digest={}",
        output.digest
    );
    assert!(
        early_l > early_r * 3.0,
        "泳道起点(-1)必须左重: L={early_l} R={early_r}"
    );
    assert!(
        late_r > late_l * 3.0,
        "泳道终点(+1)必须右重: L={late_l} R={late_r}"
    );
}
