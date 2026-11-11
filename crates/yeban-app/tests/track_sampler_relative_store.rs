//! ⭐ 存入相对化（Wang 2026-11-01 定）：`--track-sampler` 给库根下绝对路径 ⇒
//! 落盘的 `TrackV3::sampler.path` 是 `assets/samples/…` 相对形式；库根外 ⇒ 原样。
//!
//! # 驱动面
//!
//! 真二进制（`CARGO_BIN_EXE_yeban-app`）＋子进程 env `YEBAN_SAMPLE_LIBRARY`
//!（⛔ 不碰本进程环境）；读回走生产打开入口。夹具形状与
//! `track_sampler_track_selector.rs` 同款（主总线 ＋ MIDI 轨 ＋ aux 轨），目标轨用
//! `--sampler-track` 显式点名，⛔ 不依赖兜底落点。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr as _;
use std::time::{Duration, Instant};

use yeban_app::open::{ProjectOpenOptions, open_project_document_file};
use yeban_model::container::write_project_container;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingKind, SamplerBinding, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

/// 被测二进制的绝对路径（由 Cargo 注入）。
const BIN: &str = env!("CARGO_BIN_EXE_yeban-app");

/// 一次子进程执行允许的最长时间（与 `track_sampler_track_selector` 同一条理由）。
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// 夹具身份（26 字符 ULID；与 `track_sampler_track_selector` 同款尾缀）。
fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

const MASTER_TAIL: &str = "M1";
const MIDI_TAIL: &str = "B0";
const AUX_TAIL: &str = "Z0";

/// 临时夹具目录（`Drop` 删掉自己；⛔ 工作树里不留临时文件）。
struct Fixture {
    dir: PathBuf,
    container: PathBuf,
    sfz: PathBuf,
    library: PathBuf,
    midi: EntityId,
}

impl Fixture {
    /// `under_root` = true ⇒ SFZ 落在库根 `vsco2ce/` 下；false ⇒ 落在库根外。
    fn new(name: &str, under_root: bool) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-relstore-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        let library = dir.join("library");
        let sfz_dir = if under_root {
            library.join("vsco2ce")
        } else {
            dir.join("elsewhere")
        };
        std::fs::create_dir_all(&sfz_dir).expect("建 SFZ 目录");
        // 真样本腿：一份 480 帧、常量 0.25 的 16-bit 单声道 WAV（与既有夹具同形）。
        let samples: Vec<i16> = (0..480).map(|_| 8_192).collect();
        write_plain_wav(
            sfz_dir.join("tone.wav"),
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        let sfz = sfz_dir.join("Harp_slim.sfz");
        std::fs::write(&sfz, "<region> key=60 sample=tone.wav\n").expect("写夹具 SFZ");

        let midi = fixture_id(MIDI_TAIL);
        let aux = fixture_id(AUX_TAIL);
        let master = fixture_id(MASTER_TAIL);
        let project = fixture_project(master, midi, aux);
        let bytes = write_project_container(&project, b"yeban-relstore-history", &BTreeMap::new())
            .expect("写出真容器");
        let container = dir.join("fixture.yeban");
        std::fs::write(&container, &bytes).expect("写容器");
        Self {
            dir,
            container,
            sfz,
            library,
            midi,
        }
    }

    fn out(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 夹具形状：主总线 ＋ MIDI 轨（`Midi Lead`，一个音符 ＋ TrackToBus）＋ aux 返回轨
///（与 `track_sampler_track_selector.rs` 的 `fixture_project` 同款）。
fn fixture_project(master: EntityId, midi: EntityId, aux: EntityId) -> YebanProjectV1 {
    let mut project = YebanProjectV1 {
        id: fixture_id("P9"),
        title: "track sampler relative store fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0287,
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

    let clip = fixture_id("K0");
    let placement = fixture_id("P0");
    let note = fixture_id("N0");
    let mut track = TrackV3 {
        id: midi,
        name: "Midi Lead".to_owned(),
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
            offset_samples: None,
        },
    );
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
            name: "clip0".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.tracks.insert(midi, track);
    project.routing_graph.nodes.push(midi);
    project.routing_graph.edges.insert(
        fixture_id("E1"),
        RoutingEdge {
            id: fixture_id("E1"),
            source_node: midi,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );

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
        fixture_id("E2"),
        RoutingEdge {
            id: fixture_id("E2"),
            source_node: aux,
            destination_node: master,
            kind: RoutingKind::BusToMaster,
            gain_db: None,
        },
    );

    project.validate().expect("夹具工程必须合法");
    project
}

/// 真二进制执行（子进程 env 可另给；捕获 stdout / stderr；超时 ⇒ 杀掉并失败）。
fn invoke(args: &[&str], envs: &[(&str, &str)]) -> (i32, String, String) {
    let mut child = Command::new(BIN)
        .args(args)
        .envs(envs.iter().copied())
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

/// 读回 `--save-as` 的容器（走**生产**打开入口，⛔ 不另写一份解包）。
fn reopen(path: &Path) -> YebanProjectV1 {
    open_project_document_file(path, &ProjectOpenOptions::default())
        .unwrap_or_else(|error| panic!("读回 `{}` 失败: {error}", path.display()))
        .archive
        .project
}

fn sampler_of(project: &YebanProjectV1, id: EntityId) -> Option<SamplerBinding> {
    project
        .tracks
        .get(&id)
        .unwrap_or_else(|| panic!("夹具必须含轨 {id}"))
        .sampler
        .clone()
}

/// 判据 ①：库根下绝对路径 ⇒ 落盘相对形式。
#[test]
fn cli_installs_an_under_library_root_absolute_path_as_the_relative_form() {
    let fixture = Fixture::new("under", true);
    let out = fixture.out("under.yeban");
    let sfz = fixture.sfz.to_str().expect("UTF-8").to_owned();
    let library = fixture.library.to_str().expect("UTF-8").to_owned();
    let midi = fixture.midi.to_canonical_string();
    let (code, stdout, stderr) = invoke(
        &[
            "--open",
            fixture.container.to_str().expect("UTF-8"),
            "--sampler-track",
            &midi,
            "--track-sampler",
            &sfz,
            "--save-as",
            out.to_str().expect("UTF-8"),
        ],
        &[("YEBAN_SAMPLE_LIBRARY", &library)],
    );
    assert_eq!(code, 0, "安装必须成功；stdout={stdout} stderr={stderr}");
    let saved = reopen(&out);
    assert_eq!(
        sampler_of(&saved, fixture.midi),
        Some(SamplerBinding {
            path: "assets/samples/vsco2ce/Harp_slim.sfz".to_owned()
        }),
        "库根下绝对路径必须落盘成相对形式"
    );
}

/// 判据 ②：库根外绝对路径 ⇒ 落盘原样（行为不变）。
#[test]
fn cli_installs_an_outside_library_root_absolute_path_verbatim() {
    let fixture = Fixture::new("outside", false);
    let out = fixture.out("outside.yeban");
    let sfz = fixture.sfz.to_str().expect("UTF-8").to_owned();
    let library = fixture.library.to_str().expect("UTF-8").to_owned();
    let midi = fixture.midi.to_canonical_string();
    let (code, stdout, stderr) = invoke(
        &[
            "--open",
            fixture.container.to_str().expect("UTF-8"),
            "--sampler-track",
            &midi,
            "--track-sampler",
            &sfz,
            "--save-as",
            out.to_str().expect("UTF-8"),
        ],
        &[("YEBAN_SAMPLE_LIBRARY", &library)],
    );
    assert_eq!(code, 0, "安装必须成功；stdout={stdout} stderr={stderr}");
    let saved = reopen(&out);
    assert_eq!(
        sampler_of(&saved, fixture.midi),
        Some(SamplerBinding { path: sfz }),
        "库根外的绝对路径必须落盘原样"
    );
}
