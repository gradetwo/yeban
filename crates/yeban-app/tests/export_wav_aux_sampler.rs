//! 任务单 `#237`：**含 `AuxReturn`（有入边的总线节点）的工程必须能导出**。
//!
//! # 缺陷（`#220` 逐字，台账 `e93638ba` 登记为 F6）
//!
//! ```text
//! yeban-app: 导出 WAV 到 `…/diag-cli.wav` 失败: 渲染失败: 节点 01M5842E4HZ87CHA9E1G24GHRJ 有入边, 不能注册样本源
//! ```
//!
//! 根因**不在** `--track-sampler`：`render_core` 的 `renderable` 是"计划里的非主总线轨"，
//! 它把 **aux 返回轨**也当成源 → 给一个**有入边**的总线节点注册样本源 →
//! [`yeban_render::render::RenderPlan::execute`] 按定义拒绝（总线输出 = 入边归约，
//! 不是某个源）。因此**任何**含 `AuxReturn` ＋ 入边的工程都导不出来，与是否装采样器无关。
//!
//! ⭐ `#240`（Logic Pro 的通道条语义）**取代**了 `#237` 的绕法：渲染器的输入端现在
//! 把自己的源与全部入边**同权相加**，再进插入链 ⇒ aux 既注册源、也仍接受入边。
//! 本文件保留 `#237` 的端到端判据（CLI 能导出、发送仍生效），并把它对"aux 是源"的
//! 断言从**否定**改成**肯定**（那是语义改进，不是放宽）。
//!
//! # 判据与文件头的对应
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | [`the_cli_exports_a_project_whose_aux_return_carries_a_sampler`] | 真二进制：`--open` ＋ `--track-sampler` ＋ `--export-wav` ⇒ 退出码 0、`frames=`、产物峰值 > 0 |
//! | [`the_aux_send_still_reaches_the_master`] | aux 的发送**仍然生效**：`SendToAux` 增益 `-60 dB` ⇒ 母带峰值**必然变小** |
//! | [`the_aux_return_is_registered_as_a_sample_source`] | ⭐ `#240`：aux 仍**注册自己的源**（`#237` 的绕法已删）⇒ 逐源峰值表里**有** aux；它的源为空时峰值为 0、但节点在表里 |
//! | [`an_aux_return_without_a_send_still_exports`] | **无入边**的 aux 也是源节点，能导出（`MissingSource` 不会炸） |
//! | [`a_project_without_an_aux_return_exports_the_same_bytes_twice`] | 正对照：无 aux 的同形工程连导两次逐字节相同（同时给报告行 sha256，供修前/修后对比） |
//! | [`the_sampler_installed_on_the_aux_return_is_heard_in_the_export`] | ⭐ `#286`（F6 的**真**判据）：aux **自己带音符** ⇒ 母带上唯一能发声的源就是它的采样器；装/不装采样器两次 CLI 导出的 `sha256` **必须不同**，逐源峰值 > 0 |
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 不证"人耳听到混响"：只证**样本**上 aux 的贡献存在且随发送增益变化；
//! - ⛔ 不钉 WAV 的绝对 `sha256`（机器相关，台账第 313 条）—— 正对照用同一台机器的自比；
//! - ⚠ ⭐ `#286` 实测发现：[`fixture_project`] 的 aux **没有 MIDI 内容** ⇒ 上面那条
//!   CLI 判据的 `peak > 0` 其实来自 `T1`／`T2`，**不证明** aux 的采样器发声。
//!   补的判据用 [`aux_source_only_project`]（`T1` 静音、`A0` 自带音符）把声源
//!   **隔离到 aux**；"源 ＋ 发送两路都响"的分路读数仍在 `export_wav_bus_sum.rs`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr as _;
use std::time::{Duration, Instant};

use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::render_core::render_project_f32_with_assets_under;
use yeban_model::container::write_project_container;
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote,
    RoutingEdge, RoutingKind, SamplerBinding, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav_mono_f32, write_plain_wav};

/// 被测二进制的绝对路径（由 Cargo 注入）。
const BIN: &str = env!("CARGO_BIN_EXE_yeban-app");

/// 一次子进程执行允许的最长时间（与 `cli_contract` 同一条理由：不许把 CI 挂死）。
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

/// 临时夹具目录（`Drop` 删掉自己；⛔ 工作树里不留临时文件）。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-237-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        // 真样本腿：一份 480 帧、常量 0.25 的 16-bit 单声道 WAV。
        let samples: Vec<i16> = (0..480).map(|_| 8_192).collect();
        write_plain_wav(
            dir.join("tone.wav"),
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        Self { dir }
    }

    fn write_sfz(&self, name: &str, text: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, text).expect("写夹具 SFZ");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 工程形状（`#220` 的复现形状）：
///
/// - `A0` = **辅助返回轨**（`TrackKind::AuxReturn`，身份序**最小** ⇒ `--track-sampler`
///   的"身份序第一条非主总线轨"落在它身上，与 `#220` 的报错节点同一个形状）；
/// - `T1` / `T2` = 两条 MIDI 轨，各有 1 个音符；
/// - `E1`/`E2` = 两条 `TrackToBus` → master；`E3` = aux `BusToMaster` → master；
/// - `E4` = `T1` → aux 的 `SendToAux`（`send_gain_db` = `None` ⇒ 单位增益）。
///
/// `with_aux = false` ⇒ 同形工程但**没有** aux 轨 / 没有 `E3` / 没有 `E4`（正对照）。
fn fixture_project(with_aux: bool, send_gain_db: Option<f32>) -> YebanProjectV1 {
    let master = fixture_id("M1");
    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "aux return sampler fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0237,
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

    // 两台 MIDI 轨：互不相同的音高，力度不高（避免母带求和削波把"峰值变不变"这个读数糊掉）。
    for (index, pitch) in [60_u8, 67].into_iter().enumerate() {
        let track_id = fixture_id(&format!("T{}", index + 1));
        let clip = fixture_id(&format!("K{}", index + 1));
        let placement = fixture_id(&format!("P{}", index + 1));
        let note = fixture_id(&format!("N{}", index + 1));
        let edge = fixture_id(&format!("E{}", index + 1));
        let mut track = TrackV3 {
            id: track_id,
            name: format!("Midi {index}"),
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
                velocity: 48,
                ..MidiNote::new(note, 0, pitch, 1_920)
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

    if with_aux {
        let aux = fixture_id("A0");
        let aux_edge = fixture_id("E3");
        let send_edge = fixture_id("E4");
        project.tracks.insert(
            aux,
            TrackV3 {
                id: aux,
                name: "Aux Return".to_owned(),
                kind: TrackKind::AuxReturn,
                ..TrackV3::default()
            },
        );
        project.routing_graph.nodes.push(aux);
        project.routing_graph.edges.insert(
            aux_edge,
            RoutingEdge {
                id: aux_edge,
                source_node: aux,
                destination_node: master,
                kind: RoutingKind::BusToMaster,
                gain_db: None,
            },
        );
        project.routing_graph.edges.insert(
            send_edge,
            RoutingEdge {
                id: send_edge,
                source_node: fixture_id("T1"),
                destination_node: aux,
                kind: RoutingKind::SendToAux,
                gain_db: send_gain_db,
            },
        );
    }

    project.routing_graph.nodes.sort_unstable();
    project.validate().expect("夹具工程必须合法");
    project
}

/// ⭐ `#286`（F6 的**真**判据形状）：**母带上唯一能发声的源就是 aux 自己的采样器**。
///
/// 与 [`fixture_project`] 的差别：那条夹具的 aux 没有 MIDI 内容 ⇒ 导出峰值来自
/// `T1`／`T2`，"峰值 > 0"**不能**证明 aux 的采样器发声（`#286` 的实测发现）。
/// 这里 `T1` **没有片段**（只留一条 `SendToAux`，给 aux 造入边 —— F6 的形状），
/// `A0` 自己带一个 pitch 60 的音符 ⇒ 母带上的非零样本只可能来自 aux 的源。
///
/// 身份序：`A0` < `T1` ⇒ `--track-sampler` 的目标轨（第一条非主总线轨）就是 aux。
fn aux_source_only_project() -> YebanProjectV1 {
    let master = fixture_id("M1");
    let aux = fixture_id("A0");
    let send_only = fixture_id("T1");

    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "aux source only fixture".to_owned(),
        bpm: 120.0,
        // ⭐ `#286`：固定种子 ⇒ 概率触发与噪声在每次执行里同解。
        rng_seed: 0x5945_4241_4E00_0286,
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

    // T1：**没有片段**（静音），只有一条 `SendToAux` ⇒ aux 有入边。
    project.tracks.insert(
        send_only,
        TrackV3 {
            id: send_only,
            name: "Send Only".to_owned(),
            kind: TrackKind::Midi,
            volume_db: -6.0,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(send_only);

    // A0：自己带一个音符（pitch 60，与夹具 SFZ 的 region 对上）。
    let clip = fixture_id("K0");
    let placement = fixture_id("P0");
    let note = fixture_id("N0");
    let mut track = TrackV3 {
        id: aux,
        name: "Aux Return".to_owned(),
        kind: TrackKind::AuxReturn,
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
            velocity: 100,
            ..MidiNote::new(note, 0, 60, 1_920)
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "auxclip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.tracks.insert(aux, track);
    project.routing_graph.nodes.push(aux);

    for (tail, source_node, destination_node, kind) in [
        ("E1", send_only, aux, RoutingKind::SendToAux),
        ("E2", aux, master, RoutingKind::BusToMaster),
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

    project.routing_graph.nodes.sort_unstable();
    project.validate().expect("夹具工程必须合法");
    project
}

/// 真二进制的一次执行（只用无窗口参数）。
fn invoke(args: &[&str]) -> (i32, String, String) {
    let mut child = Command::new(BIN)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("无法执行 {BIN}: {error}"));
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("`yeban-app {args:?}` 在 {RUN_TIMEOUT:?} 内没有退出");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let output = child.wait_with_output().expect("收集输出");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// 取某一行里 `key=value` 的 `value`（到下一个空白为止）。
fn field(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

fn peak_of(path: &Path) -> f32 {
    let (samples, _, _) = read_plain_wav_mono_f32(path).expect("读导出的 WAV");
    samples
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()))
}

fn write_container(dir: &Path, name: &str, project: &YebanProjectV1) -> PathBuf {
    let bytes = write_project_container(project, b"aux-sampler-history", &BTreeMap::new())
        .expect("写出真容器");
    let path = dir.join(name);
    std::fs::write(&path, &bytes).expect("写容器");
    path
}

/// ⭐ 判据 ①（`#220` 的复现形状 ＋ 真二进制）：含 aux ＋ 发送的工程装采样器后**必须能导出**。
#[test]
fn the_cli_exports_a_project_whose_aux_return_carries_a_sampler() {
    let fixture = Fixture::new("cli");
    let sfz = fixture.write_sfz("tone.sfz", "<region> key=60 sample=tone.wav\n");
    // 先量一次"采样器装在哪条轨"：CLI 的选择器 = 身份序第一条非主总线轨。
    let project = fixture_project(true, None);
    let first = project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
        .expect("夹具必须有非主总线轨");
    println!(
        "[237/cli] 身份序第一条非主总线轨: id={} name={} kind={:?}",
        first.id, first.name, first.kind
    );
    assert_eq!(first.kind, TrackKind::AuxReturn, "夹具必须复现 #220 的形状");
    let container = write_container(&fixture.dir, "aux-sampler.yeban", &project);
    let out = fixture.dir.join("diag-cli.wav");

    let (code, stdout, stderr) = invoke(&[
        "--open",
        container.to_str().expect("UTF-8"),
        "--track-sampler",
        sfz.to_str().expect("UTF-8"),
        "--export-wav",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[237/cli] rc={code}\n[237/cli] stdout:\n{stdout}\n[237/cli] stderr:\n{stderr}");
    assert_eq!(
        code, 0,
        "含 AuxReturn 的工程必须能导出（#220 的失败: 有入边, 不能注册样本源）; stderr={stderr}"
    );
    let line = stdout
        .lines()
        .find(|line| line.starts_with("track-sampler:"))
        .expect("必须有 track-sampler 报告行");
    assert!(
        line.contains("track=Aux Return"),
        "采样器必须真的装到 aux 轨上（#220 的形状）: {line}"
    );
    let line = stdout
        .lines()
        .find(|line| line.starts_with("exported-wav:"))
        .expect("必须有 exported-wav 报告行");
    let frames: u64 = field(line, "frames")
        .expect("frames")
        .parse()
        .expect("frames");
    let bytes: u64 = field(line, "bytes").expect("bytes").parse().expect("bytes");
    let peak = peak_of(&out);
    println!("[237/cli] {line}\n[237/cli] peak={peak}");
    assert_eq!(frames, 48_000 * 8, "默认 8 秒 48 kHz");
    assert!(bytes > 0, "必须是真文件");
    assert!(peak > 0.0, "导出的 WAV 不能是静音（peak = {peak}）");
}

/// ⭐ 判据 ②：aux 的**发送仍然生效** —— 同一工程把 `SendToAux` 增益拉到 `-60 dB`，
/// 母带峰值必须**变小**（发送若被丢掉，两次峰值会逐位相同）。
#[test]
fn the_aux_send_still_reaches_the_master() {
    let options = WavExportOptions {
        frames: 48_000 * 2,
        ..WavExportOptions::default()
    };
    let render = |send_gain_db: Option<f32>| {
        let project = fixture_project(true, send_gain_db);
        let output = render_project_f32_with_assets_under(&project, options, None, &[], None)
            .expect("含 aux 的工程必须能渲染");
        let peak = output
            .samples
            .iter()
            .fold(0.0_f32, |acc, value| acc.max(value.abs()));
        (peak, output)
    };
    let (unity_peak, unity) = render(Some(0.0));
    let (quiet_peak, quiet) = render(Some(-60.0));
    println!(
        "[237/send] unity_peak={unity_peak:.9} quiet_peak={quiet_peak:.9} delta={:.9} \
         unity_sources={:?} quiet_sources={:?}",
        unity_peak - quiet_peak,
        unity
            .source_peaks
            .iter()
            .map(|(id, peak)| (id.to_canonical_string(), *peak))
            .collect::<Vec<_>>(),
        quiet
            .source_peaks
            .iter()
            .map(|(id, peak)| (id.to_canonical_string(), *peak))
            .collect::<Vec<_>>(),
    );
    assert!(unity_peak > 0.0, "母线不能是静音");
    assert!(
        quiet_peak < unity_peak,
        "把发送拉到 -60 dB 后母带峰值必须变小（发送被丢了）: unity={unity_peak} quiet={quiet_peak}"
    );
}

/// ⭐ 判据 ③（`#240` 的语义改进）：aux 是**既有自己的源、又有入边**的通道条 ——
/// 它的源**仍注册**（不再是 `#237` 的"从源表里拿掉"），入边也仍然归约。
///
/// 本夹具的 aux 没有 MIDI 内容 ⇒ 源的峰值为 **0**，但节点必须在表里：这正是
/// "两路都在"的静态证据（动态的分路读数见 `export_wav_bus_sum.rs`）。
#[test]
fn the_aux_return_is_registered_as_a_sample_source() {
    let project = fixture_project(true, None);
    let aux = fixture_id("A0");
    let output = render_project_f32_with_assets_under(
        &project,
        WavExportOptions {
            frames: 48_000,
            ..WavExportOptions::default()
        },
        None,
        &[],
        None,
    )
    .expect("含 aux 的工程必须能渲染");
    let ids: Vec<EntityId> = output.source_peaks.iter().map(|(id, _)| *id).collect();
    println!(
        "[237/sources] sources={:?}",
        ids.iter()
            .map(EntityId::to_canonical_string)
            .collect::<Vec<_>>()
    );
    assert!(
        ids.contains(&aux),
        "aux 是有入边的通道条，但它的源仍必须注册（#240）"
    );
    assert!(ids.contains(&fixture_id("T1")), "MIDI 轨仍必须是源");
    assert!(ids.contains(&fixture_id("T2")), "MIDI 轨仍必须是源");
    let aux_peak = output
        .source_peaks
        .iter()
        .find(|(id, _)| *id == aux)
        .map(|(_, peak)| *peak)
        .expect("aux 必须在逐源峰值表里");
    assert_eq!(aux_peak, 0.0, "本夹具的 aux 没有 MIDI 内容 ⇒ 源必须静音");
}

/// ⭐ 判据 ⑤：**没有**发送（⇒ 没有入边）的 `AuxReturn` 仍然能导出。
///
/// 无入边的 aux 是"没有入边的源节点"，计划要求它注册一个（静音）源，⛔ 不能拿掉
/// （否则 `execute` 会报 `MissingSource`）。界面「+辅助返回」刚建完就是这种形状。
/// `#240` 之后这条与"有入边的 aux"走**同一个**渲染形状（源进输入端，入边为空）。
#[test]
fn an_aux_return_without_a_send_still_exports() {
    let mut project = fixture_project(true, None);
    // 取走 SendToAux：只剩 `BusToMaster` ⇒ aux 没有入边。
    project.routing_graph.edges.remove(&fixture_id("E4"));
    project.validate().expect("夹具工程必须合法");
    let output = render_project_f32_with_assets_under(
        &project,
        WavExportOptions {
            frames: 48_000 * 2,
            ..WavExportOptions::default()
        },
        None,
        &[],
        None,
    )
    .expect("无发送的 aux 也必须能导出");
    let peak = output
        .samples
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()));
    let aux = fixture_id("A0");
    let ids: Vec<EntityId> = output.source_peaks.iter().map(|(id, _)| *id).collect();
    println!("[237/aux-no-send] peak={peak:.9} sources={ids:?}");
    assert!(peak > 0.0, "两台 MIDI 轨必须仍然出声");
    assert!(
        ids.contains(&aux),
        "无入边的 aux 是源节点，必须注册（静音）源，否则 MissingSource"
    );
}

/// ⭐ 判据 ④（正对照）：**不含** aux 的同形工程连导两次逐字节相同；
/// 同时打印 sha256，供「修前 / 修后」在同一台机器上对比"字节不变"。
#[test]
fn a_project_without_an_aux_return_exports_the_same_bytes_twice() {
    let fixture = Fixture::new("control");
    let project = fixture_project(false, None);
    let options = WavExportOptions {
        frames: 48_000 * 2,
        ..WavExportOptions::default()
    };
    let first_path = fixture.dir.join("control-a.wav");
    let second_path = fixture.dir.join("control-b.wav");
    let first = export_wav(&project, &first_path, options).expect("正对照第一次导出");
    let second = export_wav(&project, &second_path, options).expect("正对照第二次导出");
    let first_bytes = std::fs::read(&first_path).expect("读第一次产物");
    let second_bytes = std::fs::read(&second_path).expect("读第二次产物");
    let sha = yeban_model::AssetHash::of_bytes(&first_bytes);
    println!(
        "[237/control] bytes={} sha256={} digest={}",
        first.bytes, sha, first.digest
    );
    assert_eq!(first.bytes, second.bytes, "字节数");
    assert_eq!(first_bytes, second_bytes, "同一工程连导两次必须逐字节相同");
    assert_eq!(first.digest, second.digest, "母带 digest 必须相同");
}

/// ⭐ `#286`（F6 的**真**判据）：CLI 装到 aux 上的采样器**真的进导出**。
///
/// 为什么补这一条：`#220` 的复现形状里，`--track-sampler` 落到 aux（身份序第一条
/// 非主总线轨），但 aux 本身没有 MIDI 内容 ⇒ 那条 CLI 判据的 `peak > 0` 来自别的
/// MIDI 轨，**采不采样器都成立**。本判据把 `T1` 静音、把音符放在 aux 上：
///
/// - 母带上**唯一**能发声的源 = aux（它自带音符 ＋ 采样器）；
/// - 同一份容器，**装/不装** `--track-sampler` 各导一次 ⇒ WAV `sha256` **必须不同**
///   （采样器被静默丢掉时两次会逐字节相同 —— 这正是 `#324` 的"静默少做"）；
/// - 逐源读数：aux 的源峰值 > 0，且装/不装时峰值不同。
///
/// ⚠ 本判据**不钉**绝对 `sha256`（机器相关，台账第 313 条）：只比同一台机器上
/// 同一次执行里的"装 vs 不装"。
#[test]
fn the_sampler_installed_on_the_aux_return_is_heard_in_the_export() {
    let fixture = Fixture::new("audible");
    let sfz = fixture.write_sfz("tone.sfz", "<region> key=60 sample=tone.wav\n");
    let project = aux_source_only_project();
    let aux = fixture_id("A0");
    let container = write_container(&fixture.dir, "audible-plain.yeban", &project);

    // ---- ① 不装采样器：aux 走引擎合成器（"采样器真的改变了声音"的对照）。 ----
    let plain_wav = fixture.dir.join("audible-plain.wav");
    let (plain_code, plain_stdout, plain_stderr) = invoke(&[
        "--open",
        container.to_str().expect("UTF-8"),
        "--export-wav",
        plain_wav.to_str().expect("UTF-8"),
    ]);
    assert_eq!(
        plain_code, 0,
        "无采样器的形状也必须能导出; stderr={plain_stderr}"
    );
    let plain_line = plain_stdout
        .lines()
        .find(|line| line.starts_with("exported-wav:"))
        .expect("必须有 exported-wav 报告行");
    assert_eq!(field(plain_line, "frames").as_deref(), Some("384000"));
    let plain_peak = peak_of(&plain_wav);
    let plain_sha = AssetHash::of_bytes(&std::fs::read(&plain_wav).expect("读无采样器产物"));

    // ---- ② 同一份容器 ＋ `--track-sampler`：采样器落到 aux，导出必须成功。 ----
    let sampled_wav = fixture.dir.join("audible-sampled.wav");
    let (code, stdout, stderr) = invoke(&[
        "--open",
        container.to_str().expect("UTF-8"),
        "--track-sampler",
        sfz.to_str().expect("UTF-8"),
        "--export-wav",
        sampled_wav.to_str().expect("UTF-8"),
    ]);
    assert_eq!(code, 0, "aux 装采样器后必须能导出; stderr={stderr}");
    let track_line = stdout
        .lines()
        .find(|line| line.starts_with("track-sampler:"))
        .expect("必须有 track-sampler 报告行");
    assert!(
        track_line.contains("track=Aux Return"),
        "采样器必须装到 aux 上（F6 的形状）: {track_line}"
    );
    let line = stdout
        .lines()
        .find(|line| line.starts_with("exported-wav:"))
        .expect("必须有 exported-wav 报告行");
    assert_eq!(field(line, "frames").as_deref(), Some("384000"), "{line}");
    assert!(
        line.contains("sfz(tracks=1") && line.contains("silent=0"),
        "导出必须把 aux 的采样器摊平进来: {line}"
    );
    let sampled_peak = peak_of(&sampled_wav);
    let sampled_sha = AssetHash::of_bytes(&std::fs::read(&sampled_wav).expect("读采样器产物"));

    println!(
        "[286/cli] plain:  bytes={} peak={plain_peak:.9} sha256={plain_sha}",
        std::fs::metadata(&plain_wav).expect("stat").len()
    );
    println!(
        "[286/cli] sampled: bytes={} peak={sampled_peak:.9} sha256={sampled_sha}",
        std::fs::metadata(&sampled_wav).expect("stat").len()
    );
    assert!(plain_peak > 0.0, "对照档（引擎合成器）必须非静音");
    assert!(
        sampled_peak > 0.0,
        "aux 的采样器必须发声（母带上没有别的声源）: {sampled_peak}"
    );
    assert_ne!(
        plain_sha, sampled_sha,
        "装/不装采样器的导出字节必须不同（相同 ⇒ 采样器被静默丢掉）"
    );
    assert!(
        (plain_peak - sampled_peak).abs() > 1e-3,
        "装/不装采样器的母带峰值必须不同: plain={plain_peak} sampled={sampled_peak}"
    );

    // ---- ③ 逐源读数：aux 的源峰值 > 0，且装/不装的 `digest` 不同。 ----
    //
    // 采样器的绑定与 `--track-sampler` 的落点相同（`Op::SetTrackSampler` 只写这一个
    // 字段）；CLI 的安装→导出接线已经由上面的两次子进程钉住。
    let mut with_sampler = aux_source_only_project();
    with_sampler
        .tracks
        .get_mut(&aux)
        .expect("夹具必须有 aux")
        .sampler = Some(SamplerBinding {
        path: sfz.to_str().expect("UTF-8").to_owned(),
    });
    let options = WavExportOptions {
        frames: 48_000 * 2,
        ..WavExportOptions::default()
    };
    let plain =
        render_project_f32_with_assets_under(&project, options, None, &[], Some(&fixture.dir))
            .expect("无采样器的工程必须能渲染");
    let sampled =
        render_project_f32_with_assets_under(&with_sampler, options, None, &[], Some(&fixture.dir))
            .expect("aux 采样器的工程必须能渲染");
    let aux_peak_of = |output: &yeban_app::render_core::RenderProjectOutput| {
        output
            .source_peaks
            .iter()
            .find(|(id, _)| *id == aux)
            .map(|(_, peak)| *peak)
            .expect("aux 必须在逐源峰值表里")
    };
    let plain_aux = aux_peak_of(&plain);
    let sampled_aux = aux_peak_of(&sampled);
    println!(
        "[286/sources] plain_aux={plain_aux:.9} sampled_aux={sampled_aux:.9} \
         plain_digest={} sampled_digest={}",
        plain.digest, sampled.digest
    );
    assert!(
        sampled_aux > 0.0,
        "aux 的源必须发声（采样器 PCM 常数 0.25）: {sampled_aux}"
    );
    assert!(
        (plain_aux - sampled_aux).abs() > 1e-3,
        "装/不装采样器的 aux 源峰值必须不同: plain={plain_aux} sampled={sampled_aux}"
    );
    assert_ne!(
        plain.digest, sampled.digest,
        "装/不装采样器的母带 digest 必须不同（相同 ⇒ 采样器被静默丢掉）"
    );
}
