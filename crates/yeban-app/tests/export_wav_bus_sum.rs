//! 任务单 `#240`：**总线（`AuxReturn`）既发自己的音源、又接收入边 ⇒ 两路相加**。
//!
//! # 要抄的 Logic 形状
//!
//! ```text
//! 输入端求和（送进来的 send ＋ 自己的音源输出）→ 插入链 → 输出
//! ```
//!
//! 本文件从**产品装配**（`render_project_f32_with_assets_under` ＋
//! `export_wav`）出发，给同一工程的**四种开关组合**各一份读数：
//!
//! | 形状 | aux 自己的源 | `SendToAux` |
//! | :--- | :--- | :--- |
//! | 只音源 | 有（sampler ＋ 音符） | 无 |
//! | 只发送 | 无（解绑 sampler ＋ 清掉 aux 音符） | 有 |
//! | 两者合 | 有 | 有 |
//! | 都没有（线性基线） | 无 | 无 |
//!
//! ⭐ 判据：`合 ≈ 只音源 ＋ 只发送 − 都没有`（逐样本最大绝对差），并给三个 WAV 的
//! `sha256` 与母带峰值。⇒ 证明**两路都在**，且是**线性相加**。
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 不证"听感像 Logic"：只证样本上两路都在、且相加；
//! - ⛔ 不证插入链在**这条产品路径**上跑：`render_core` 走的是 `RenderPlan::compile`
//!   （不挂插入链；`#226` 已登记的口径）。插入链"同时处理两路"的结构判据在
//!   `yeban-render/tests/insert_chain.rs::the_source_and_the_incoming_edge_are_summed_before_the_insert_chain`，
//!   **本文件不假装**产品导出挂了链；
//! - ⛔ 不钉 WAV 的绝对 `sha256` 作为跨机常量（机器相关，台账第 313 条）：这里只报同机读数。
//!
//! # 测试形状（为什么 aux 有音符）
//!
//! 只有"aux 自己的源"**有内容**，分路读数才有意义。因此 aux 轨上放一个 MIDI 音符
//! （pitch 60），并用 `TrackV3.sampler` 绑定夹具 SFZ（region key=60）；"只发送"那一路
//! 把 sampler **解绑**并清掉音符 ⇒ aux 的源静音（但仍注册、仍在逐源峰值表里）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;

use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::render_core::render_project_f32_with_assets_under;
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote,
    RoutingEdge, RoutingKind, SamplerBinding, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

const FRAMES: u64 = 48_000 * 2;

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P1{tail}")).expect("夹具 ULID 必须合法")
}

/// 临时夹具目录（`Drop` 删掉自己；⛔ 工作树里不留临时文件）。
struct Fixture {
    dir: PathBuf,
    sfz: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-240-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        // 一份 480 帧、常量 0.25 的 16-bit 单声道 WAV。
        let samples: Vec<i16> = (0..480).map(|_| 8_192).collect();
        write_plain_wav(
            dir.join("tone.wav"),
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        let sfz = dir.join("tone.sfz");
        std::fs::write(&sfz, "<region> key=60 sample=tone.wav\n").expect("写夹具 SFZ");
        Self { dir, sfz }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 由源节点身份派生一个 26 字符 ULID（`tag` 占 1 位，保留源身份的末 2 位）。
fn derived_id(tag: char, source: EntityId) -> EntityId {
    let text = source.to_canonical_string();
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P{tag}{}", &text[24..]))
        .expect("派生 ULID 必须合法")
}

/// 一条带一个 MIDI 音符的轨（音符放在 `clip` 里，摆放在 `start_tick = 0`）。
fn midi_track(id: EntityId, name: &str, pitch: u8, velocity: u8) -> (TrackV3, EntityId, EntityId) {
    let clip = derived_id('K', id);
    let placement = derived_id('P', id);
    let note = derived_id('N', id);
    let mut track = TrackV3 {
        id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        volume_db: -6.0,
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
            velocity,
            ..MidiNote::new(note, 0, pitch, 1_920)
        },
    );
    (track, clip, note)
}

/// 四种开关组合共用同一形状：`A0`（aux）＋ `T1`（MIDI）＋ `M1`（master）。
///
/// - `with_source = false` ⇒ aux **解绑 sampler** 并清掉音符 ⇒ 自己的源静音；
/// - `with_send = false` ⇒ 取走 `T1 → A0` 的 `SendToAux`。
fn fixture_project(fixture: &Fixture, with_source: bool, with_send: bool) -> YebanProjectV1 {
    let master = fixture_id("M1");
    let aux = fixture_id("A0");
    let midi = fixture_id("T1");

    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "bus source + send sum fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0240,
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

    let (mut aux_track, aux_clip, aux_note) = midi_track(aux, "Aux Return", 60, 40);
    aux_track.kind = TrackKind::AuxReturn;
    aux_track.sampler = Some(SamplerBinding {
        path: fixture.sfz.to_string_lossy().into_owned(),
    });
    project.clip_pool.insert(
        aux_clip,
        ClipPoolEntry {
            id: aux_clip,
            name: "aux-clip".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::from([(
                    aux_note,
                    MidiNote {
                        velocity: 40,
                        ..MidiNote::new(aux_note, 0, 60, 1_920)
                    },
                )]),
            },
        },
    );
    if !with_source {
        // "只发送"：解绑 sampler ＋ 清掉 aux 的音符 ⇒ 源静音。
        aux_track.sampler = None;
        aux_track.clips.clear();
    }
    project.tracks.insert(aux, aux_track);
    project.routing_graph.nodes.push(aux);

    let (midi_track, midi_clip, midi_note) = midi_track(midi, "Midi", 67, 48);
    project.clip_pool.insert(
        midi_clip,
        ClipPoolEntry {
            id: midi_clip,
            name: "midi-clip".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::from([(
                    midi_note,
                    MidiNote {
                        velocity: 48,
                        ..MidiNote::new(midi_note, 0, 67, 1_920)
                    },
                )]),
            },
        },
    );
    project.tracks.insert(midi, midi_track);
    project.routing_graph.nodes.push(midi);

    for (tail, source_node, destination_node, kind) in [
        ("E1", aux, master, RoutingKind::BusToMaster),
        ("E2", midi, master, RoutingKind::TrackToBus),
    ] {
        let id = fixture_id(tail);
        project.routing_graph.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node,
                destination_node,
                kind,
                gain_db: None,
            },
        );
    }
    if with_send {
        let id = fixture_id("E4");
        project.routing_graph.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: midi,
                destination_node: aux,
                kind: RoutingKind::SendToAux,
                gain_db: None,
            },
        );
    }
    project.routing_graph.nodes.sort_unstable();
    project.validate().expect("夹具工程必须合法");
    project
}

/// 一份完整读数：母带样本、峰值、逐源峰值、WAV 文件 `sha256`。
struct Reading {
    samples: Vec<f32>,
    peak: f32,
    source_peaks: Vec<(EntityId, f32)>,
    sha256: String,
}

fn measure(fixture: &Fixture, with_source: bool, with_send: bool, label: &str) -> Reading {
    let project = fixture_project(fixture, with_source, with_send);
    let options = WavExportOptions {
        frames: FRAMES,
        ..WavExportOptions::default()
    };
    let output = render_project_f32_with_assets_under(
        &project,
        options,
        None,
        &[],
        Some(fixture.dir.as_path()),
    )
    .expect("含 aux 源 ＋ 发送的工程必须能渲染");
    let peak = output
        .samples
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()));

    let path = fixture.dir.join(format!("{label}.wav"));
    let report = export_wav(&project, &path, options).expect("导出 WAV");
    let bytes = std::fs::read(&path).expect("读产物");
    let sha256 = AssetHash::of_bytes(&bytes).to_string();
    println!(
        "[240/{label}] frames={} bytes={} peak={peak:.9} sha256={sha256} \
         sources={:?}",
        report.frames,
        report.bytes,
        output
            .source_peaks
            .iter()
            .map(|(id, value)| (id.to_canonical_string(), *value))
            .collect::<Vec<_>>()
    );
    Reading {
        samples: output.samples,
        peak,
        source_peaks: output.source_peaks,
        sha256,
    }
}

fn source_peak(reading: &Reading, node: EntityId) -> f32 {
    reading
        .source_peaks
        .iter()
        .find(|(id, _)| *id == node)
        .map_or(f32::NAN, |(_, peak)| *peak)
}

/// ⭐ 判据：`合 ≈ 只音源 ＋ 只发送 − 都没有`，且三个 WAV 的 `sha256` 互不相同。
///
/// 读数单位：样本（无量纲，`f32`），差值取**逐样本绝对差的最大值**。
#[test]
fn the_bus_emits_its_own_source_and_its_incoming_edge_summed() {
    let fixture = Fixture::new("sum");
    let aux = fixture_id("A0");

    let neither = measure(&fixture, false, false, "neither");
    let source = measure(&fixture, true, false, "source-only");
    let send = measure(&fixture, false, true, "send-only");
    let both = measure(&fixture, true, true, "both");

    // ⭐ 逐源峰值：aux 的源在"有源"的两档必须出声，在"只发送"的两档静音但仍注册。
    let aux_source = source_peak(&source, aux);
    let aux_send = source_peak(&send, aux);
    let aux_both = source_peak(&both, aux);
    let aux_neither = source_peak(&neither, aux);
    println!(
        "[240/aux-source-peak] source-only={aux_source:.9} send-only={aux_send:.9} \
         both={aux_both:.9} neither={aux_neither:.9}"
    );
    assert!(aux_source > 0.0, "只音源档：aux 自己的源必须出声");
    assert_eq!(aux_send, 0.0, "只发送档：aux 的源已解绑 ⇒ 必须静音");
    assert!(aux_both > 0.0, "两者合档：aux 自己的源必须仍在");
    assert_eq!(aux_neither, 0.0);

    // ⭐ 两个单路档都必须非静音（否则"相加"无从谈起）。
    assert!(source.peak > 0.0, "只音源档必须非静音");
    assert!(send.peak > 0.0, "只发送档必须非静音");
    assert!(both.peak > 0.0, "两者合档必须非静音");

    // ⭐ 线性相加：合 = 只音源 + 只发送 − 都没有（都没有 = 两路都不存在的基线）。
    assert_eq!(both.samples.len(), source.samples.len());
    assert_eq!(both.samples.len(), send.samples.len());
    assert_eq!(both.samples.len(), neither.samples.len());
    let mut max_diff = 0.0_f32;
    let mut max_diff_index = 0usize;
    for index in 0..both.samples.len() {
        let expected = source.samples[index] + send.samples[index] - neither.samples[index];
        let diff = (both.samples[index] - expected).abs();
        if diff > max_diff {
            max_diff = diff;
            max_diff_index = index;
        }
    }
    println!(
        "[240/linearity] frames={} max|combined - (source + send - neither)|={max_diff:.3e} \
         at sample {max_diff_index}",
        both.samples.len() / 2
    );
    assert!(
        max_diff < 1e-5,
        "两路必须线性相加：逐样本最大绝对差 = {max_diff:.3e}"
    );

    // ⭐ 三个 `sha256`（加上基线共四份）必须互不相同：内容真的变了，不是同一份产物。
    let shas = [
        both.sha256.as_str(),
        source.sha256.as_str(),
        send.sha256.as_str(),
        neither.sha256.as_str(),
    ];
    for left in 0..shas.len() {
        for right in (left + 1)..shas.len() {
            assert_ne!(
                shas[left], shas[right],
                "四种开关组合的 WAV 不能逐字节相同（否则分路读数没有区分力）"
            );
        }
    }
}

/// ⭐ `#296`：app 侧**总线推子**（`RenderPlan` 的输出级）真的乘在**入边求和**上。
///
/// 夹具：`midi → aux → master`（aux 自己无音源）＋ `midi → master` 直连。aux 的
/// `volume_db` 从 `-6 dB` 改成 `0 dB` ⇒ **发送那一路**的贡献必须是
/// `10^(6/20) = 1.9952623` 倍（直连那一路不经过 aux，必须先用"无发送"基线减掉）。
/// `digest` 必须真的变。修前（总线推子没有落点）两次读数会**完全相同**（−6 dB 只
/// 作用在 aux 自己的源上，而它的源是静音的）。
#[test]
fn the_bus_fader_scales_the_incoming_send_on_export_wav() {
    let fixture = Fixture::new("fader");
    let aux = fixture_id("A0");
    let baseline_project = fixture_project(&fixture, false, false);
    let quiet_project = fixture_project(&fixture, false, true);
    assert_eq!(
        quiet_project.tracks.get(&aux).expect("Aux").volume_db,
        -6.0,
        "夹具的 aux 推子必须是 -6 dB（`midi_track` 的默认值）"
    );
    let options = WavExportOptions {
        frames: FRAMES,
        ..WavExportOptions::default()
    };
    let quiet = render_project_f32_with_assets_under(
        &quiet_project,
        options,
        None,
        &[],
        Some(fixture.dir.as_path()),
    )
    .expect("只发送的工程必须能渲染");
    let quiet_peak = quiet
        .samples
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()));
    let mut loud_project = quiet_project;
    loud_project.tracks.get_mut(&aux).expect("Aux").volume_db = 0.0;
    loud_project.validate().expect("改过推子的工程必须合法");
    let loud = render_project_f32_with_assets_under(
        &loud_project,
        options,
        None,
        &[],
        Some(fixture.dir.as_path()),
    )
    .expect("只发送的工程必须能渲染");
    let loud_peak = loud
        .samples
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()));
    let baseline = render_project_f32_with_assets_under(
        &baseline_project,
        options,
        None,
        &[],
        Some(fixture.dir.as_path()),
    )
    .expect("无发送的基线必须能渲染");
    let baseline_peak = baseline
        .samples
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()));
    let quiet_send = quiet_peak - baseline_peak;
    let loud_send = loud_peak - baseline_peak;
    let ratio = loud_send / quiet_send;
    println!(
        "[296/bus-fader] baseline(no send) peak={baseline_peak:.9} quiet(-6 dB) peak={quiet_peak:.9} \
         loud(0 dB) peak={loud_peak:.9} send_ratio={ratio:.9} quiet_digest={} loud_digest={}",
        quiet.digest, loud.digest
    );
    assert!(quiet_send > 0.0, "只发送档的发送那一路必须非静音");
    assert!(
        (ratio - 1.995_262_3).abs() < 1e-3,
        "推子 0 dB / -6 dB 的**发送贡献**比必须是 1.9952623，实测 {ratio}"
    );
    assert_ne!(
        quiet.digest, loud.digest,
        "总线推子必须真的改变母带样本（digest 不同）"
    );
}
