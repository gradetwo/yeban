//! ⭐ 任务单 `#287`（`R927 ⑥` 后半）：`--track-sampler` 的目标轨可以**指定**。
//!
//! # 缺陷（`#286` 报告逐字，`84a97135` 已入主干）
//!
//! ```text
//! --track-sampler 只能装到"身份序第一条非主总线轨"、无法指定 aux（选错轨、不是 F6 的"炸"）
//! ```
//!
//! 本文件钉住**选轨**这一半；⛔ 不碰渲染语义（`F6`／`#240` 已修好，`#286` 已补判据）。
//!
//! # 量什么（对象 ＋ 单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `track-sampler:` 行的 `track=` | 报告行里**真的**被装的那条轨的名字 | 字符串（逐字） |
//! | 落盘后的 `TrackV3.sampler` | `--save-as` 容器里每条轨的绑定 | `Option<SamplerBinding>` |
//! | 退出码 | 真二进制 `yeban-app` 的进程退出码 | 整数（本文用 [`EXIT_OPEN`] / [`EXIT_USAGE`]） |
//!
//! # 判据与夹具
//!
//! 夹具（[`fixture_project`]）的身份序是 **`B0`（MIDI，名字 `Midi Lead`）< `Z0`（aux，
//! 名字 `Aux Return`）**：旧规则（身份序第一条非主总线轨）只能落到 `B0`，而 aux 是
//! `Z0` ⇒ "指定 aux"必须真的改变落点，⛔ 不可能靠"它本来就是第一条"蒙对。
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | [`the_old_path_only_form_still_targets_the_first_non_master_track`] | 只给 path ⇒ 仍是 `B0`（`track=Midi Lead`），且落盘的只有 `B0` 有 sampler |
//! | [`naming_the_aux_identity_installs_the_sampler_on_the_aux`] | `--sampler-track <Z0>` ⇒ `track=Aux Return`，落盘的只有 `Z0` 有 sampler |
//! | [`an_unknown_identity_is_a_loud_error_and_never_falls_back`] | 合法 ULID 但工程里没有 / 形状不对 ⇒ 退出码 3，⛔ 不落盘、⛔ 不回落到 `B0` |
//! | [`naming_the_master_bus_is_rejected_with_the_reason`] | 主总线身份 ⇒ 退出码 3，报文点名"主总线 / renderable / 消费点"，可用清单**不含**主总线 |
//! | [`the_selector_requires_the_track_sampler_switch`] | 单独给 `--sampler-track` ⇒ 退出码 2（解析期与 `run_batch` 两道守卫都判） |
//! | [`help_documents_both_shapes_verbatim`] | `--help` 里新旧两种形状**逐字**都在 |
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 不证"aux 上的采样器出声"：那是 `#286` 的 `the_sampler_installed_on_the_aux_return_is_heard_in_the_export`；
//! - ⛔ 不证"指定任意轨都听得出差别"：本文件只读**报告行 ＋ 落盘绑定**；
//! - ⛔ 不钉 `--track-sampler` 修前的二进制字节：跨版本 sha256 正对照在交付报告里（同一命令、同一路径）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr as _;
use std::time::{Duration, Instant};

use yeban_app::cli::{self, EXIT_OPEN, EXIT_USAGE};
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

/// 一次子进程执行允许的最长时间（与 `export_wav_aux_sampler` 同一条理由：不许把 CI 挂死）。
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// 夹具身份（26 字符 ULID；`B0` < `Z0` ⇒ 旧规则落在 MIDI 轨）。主总线 `M1` 不参与排序。
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
    midi: EntityId,
    aux: EntityId,
    master: EntityId,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-287-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        // 真样本腿：一份 480 帧、常量 0.25 的 16-bit 单声道 WAV（与既有夹具同形）。
        let samples: Vec<i16> = (0..480).map(|_| 8_192).collect();
        write_plain_wav(
            dir.join("tone.wav"),
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        let sfz = dir.join("tone.sfz");
        std::fs::write(&sfz, "<region> key=60 sample=tone.wav\n").expect("写夹具 SFZ");

        let midi = fixture_id(MIDI_TAIL);
        let aux = fixture_id(AUX_TAIL);
        let master = fixture_id(MASTER_TAIL);
        let project = fixture_project(master, midi, aux);
        let bytes = write_project_container(&project, b"yeban-287-history", &BTreeMap::new())
            .expect("写出真容器");
        let container = dir.join("fixture.yeban");
        std::fs::write(&container, &bytes).expect("写容器");
        Self {
            dir,
            container,
            sfz,
            midi,
            aux,
            master,
        }
    }

    fn sfz_text(&self) -> String {
        self.sfz.display().to_string()
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

/// 夹具形状：**身份序** `M1`(master) < `B0`(MIDI, `Midi Lead`) < `Z0`(aux, `Aux Return`)。
///
/// ⇒ 旧规则（身份序第一条非主总线轨）落在 `B0`，**不是** aux ⇒ "指定 aux"必须真的
/// 改变落点（⛔ 不能靠"aux 本来就是第一条"蒙对）。
fn fixture_project(master: EntityId, midi: EntityId, aux: EntityId) -> YebanProjectV1 {
    let mut project = YebanProjectV1 {
        id: fixture_id("P9"),
        title: "track sampler selector fixture".to_owned(),
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

    // MIDI 轨：给一个音符（导出链有内容）＋ 一条 TrackToBus 到主总线。
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

    // aux 返回轨：名字逐字 `Aux Return`（判据读数用），一条 BusToMaster 出口。
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

/// 真二进制执行（捕获 stdout / stderr；超时 ⇒ 杀掉并失败）。
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

/// `track-sampler:` 报告行（没有 ⇒ panic，让失败原因指向"根本没装"）。
fn track_line(stdout: &str) -> &str {
    stdout
        .lines()
        .find(|line| line.starts_with("track-sampler:"))
        .unwrap_or_else(|| panic!("必须有一行 `track-sampler:`；stdout=\n{stdout}"))
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

/// 判据 ①：**旧用法不破** —— 只给 path ⇒ 仍然落在身份序第一条非主总线轨（`B0`）。
#[test]
fn the_old_path_only_form_still_targets_the_first_non_master_track() {
    let fixture = Fixture::new("old");
    let out = fixture.out("old.yeban");
    let sfz = fixture.sfz_text();
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--track-sampler",
        &sfz,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[#287/old] rc={code}\n[#287/old] stdout:\n{stdout}\n[#287/old] stderr:\n{stderr}");
    assert_eq!(code, 0, "旧用法必须成功；stderr={stderr}");
    let line = track_line(&stdout);
    println!("[#287/old] {line}");
    assert!(
        line.starts_with(&format!("track-sampler: track=Midi Lead path={sfz} ops=1 ")),
        "旧用的目标轨仍是身份序第一条非主总线轨（B0 = Midi Lead），且报告行形状不变: {line}"
    );
    assert!(
        !line.contains("track=Aux Return"),
        "只给 path ⛔ 不许装到 aux（那正是 #287 要修的缺口）: {line}"
    );
    let saved = reopen(&out);
    assert_eq!(
        sampler_of(&saved, fixture.midi),
        Some(SamplerBinding { path: sfz }),
        "落盘的绑定必须在 B0 上"
    );
    assert_eq!(
        sampler_of(&saved, fixture.aux),
        None,
        "旧用法 ⛔ 不许动 aux"
    );
}

/// 判据 ②：**新用法生效** —— 指定 aux 的身份 ⇒ sampler 装到 aux 上（且只有 aux）。
#[test]
fn naming_the_aux_identity_installs_the_sampler_on_the_aux() {
    let fixture = Fixture::new("aux");
    let out = fixture.out("aux.yeban");
    let sfz = fixture.sfz_text();
    let aux = fixture.aux.to_string();
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--sampler-track",
        &aux,
        "--track-sampler",
        &sfz,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!(
        "[#287/aux] id={aux}\n[#287/aux] rc={code}\n[#287/aux] stdout:\n{stdout}\n[#287/aux] stderr:\n{stderr}"
    );
    assert_eq!(code, 0, "指定 aux 身份必须成功；stderr={stderr}");
    let line = track_line(&stdout);
    println!("[#287/aux] {line}");
    assert!(
        line.contains("track=Aux Return"),
        "采样器必须真的装到 aux 轨上: {line}"
    );
    assert!(
        line.starts_with(&format!(
            "track-sampler: track=Aux Return path={sfz} ops=1 "
        )),
        "报告行的形状与旧用法一致（只换目标轨）: {line}"
    );
    let saved = reopen(&out);
    assert_eq!(
        sampler_of(&saved, fixture.aux),
        Some(SamplerBinding { path: sfz }),
        "落盘的绑定必须在 aux（Z0）上"
    );
    assert_eq!(
        sampler_of(&saved, fixture.midi),
        None,
        "指定 aux 时 ⛔ 不许再装到身份序第一条（B0）"
    );
}

/// 判据 ③：**选错身份** ⇒ 响亮报错；⛔ 静默回落到第一条、⛔ 不落盘。
#[test]
fn an_unknown_identity_is_a_loud_error_and_never_falls_back() {
    let fixture = Fixture::new("unknown");
    let out = fixture.out("unknown.yeban");
    let sfz = fixture.sfz_text();

    // (a) 形状合法（26 字符 ULID）但工程里没有。
    let missing = fixture_id("Q9").to_string();
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--sampler-track",
        &missing,
        "--track-sampler",
        &sfz,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[#287/unknown] id={missing} rc={code}\nstderr={stderr}");
    assert_eq!(code, i32::from(EXIT_OPEN), "选错身份必须是退出码 3");
    assert!(
        stderr.contains(&format!("工程里没有身份 `{missing}` 的音轨")),
        "必须**点名**查不到的那个身份: {stderr}"
    );
    assert!(
        stderr.contains("可用:") && stderr.contains(&fixture.aux.to_string()),
        "必须列出可用身份（含 aux），⛔ 不让人瞎猜: {stderr}"
    );
    assert!(
        !stdout.contains("track-sampler:"),
        "失败时 ⛔ 不许打印一条'装好了'的报告行: {stdout}"
    );
    assert!(!out.exists(), "失败时 ⛔ 不许落盘");

    // (b) 形状不对（不是 ULID）：⛔ 不许把它当名字/序号去猜。
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--sampler-track",
        "Aux Return",
        "--track-sampler",
        &sfz,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[#287/unknown] malformed rc={code}\nstderr={stderr}");
    assert_eq!(code, i32::from(EXIT_OPEN), "形状不对必须是退出码 3");
    assert!(
        stderr.contains("不是音轨身份") && stderr.contains("26 字符 ULID"),
        "必须说清要求什么形状: {stderr}"
    );
    assert!(
        !stderr.contains("track=Aux Return") && !stdout.contains("track-sampler:"),
        "⛔ 不许把名字当身份、⛔ 不许回落: {stderr}"
    );
    assert!(!out.exists(), "失败时 ⛔ 不许落盘");
}

/// 判据 ④：**主总线** ⇒ 响亮拒绝 ＋ 说明为什么（它是混音出口，sampler 没有消费点）。
#[test]
fn naming_the_master_bus_is_rejected_with_the_reason() {
    let fixture = Fixture::new("master");
    let out = fixture.out("master.yeban");
    let sfz = fixture.sfz_text();
    let master = fixture.master.to_string();
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--sampler-track",
        &master,
        "--track-sampler",
        &sfz,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[#287/master] id={master} rc={code}\nstderr={stderr}");
    assert_eq!(code, i32::from(EXIT_OPEN), "主总线必须是退出码 3");
    assert!(
        stderr.contains("主总线") && stderr.contains("renderable") && stderr.contains("消费点"),
        "必须说明为什么拒绝（唯一混音出口 / renderable 排除 / 没有消费点）: {stderr}"
    );
    assert!(
        !stderr.contains(&format!("{master}=Master")),
        "可用清单 ⛔ 不许把主总线列成一个可选目标: {stderr}"
    );
    assert!(
        !stdout.contains("track-sampler:") && !out.exists(),
        "⛔ 不许落盘、⛔ 不许打印'装好了'"
    );
}

/// 判据 ⑤：`--sampler-track` 只是目标选择器 —— 单独给 = 用法错误（两道守卫都判）。
#[test]
fn the_selector_requires_the_track_sampler_switch() {
    let aux = fixture_id(AUX_TAIL).to_string();

    // (a) 解析期就拒（真二进制的路径：否则它会静默进 GUI、什么也不做）。
    let args = vec!["--sampler-track".to_owned(), aux.clone()];
    let error = cli::parse(&args).expect_err("单独给必须解析失败");
    println!("[#287/lone] parse: {error}");
    assert_eq!(error.exit_code(), EXIT_USAGE);
    assert!(
        error.to_string().contains("--sampler-track")
            && error.to_string().contains("--track-sampler"),
        "报文必须点名两个开关: {error}"
    );
    let (code, stdout, stderr) = invoke(&["--sampler-track", &aux]);
    println!("[#287/lone] rc={code}\nstderr={stderr}");
    assert_eq!(code, i32::from(EXIT_USAGE), "真二进制必须是退出码 2");
    assert!(
        stderr.contains("--sampler-track") && stderr.contains("--track-sampler"),
        "真二进制的报文必须点名两个开关: {stderr}"
    );
    assert!(
        !stdout.contains("track-sampler:") && !stdout.contains("headless ok"),
        "⛔ 不许悄悄走批处理（更不许证明任何'装好了'）: {stdout}"
    );

    // (b) 直接构造 `Options`（绕过解析器）⇒ `run_batch` 的第二道守卫。
    let options = cli::Options {
        sampler_track: Some(aux),
        ..cli::Options::default()
    };
    let error = cli::run_batch(&options).expect_err("run_batch 必须拒绝");
    println!("[#287/lone] run_batch: {error}");
    assert_eq!(error.exit_code(), EXIT_USAGE);
    assert!(
        error.to_string().contains("--sampler-track")
            && error.to_string().contains("--track-sampler"),
        "第二道守卫的报文必须点名两个开关: {error}"
    );
}

/// 判据 ⑥：`--help` 把**新旧两种形状**逐字写清（含"缺省 = 身份序第一条非主总线轨"）。
#[test]
fn help_documents_both_shapes_verbatim() {
    let (code, stdout, stderr) = invoke(&["--help"]);
    println!("[#287/help] rc={code} bytes={}", stdout.len());
    assert_eq!(code, 0, "`--help` 必须成功；stderr={stderr}");
    for needle in [
        "--track-sampler <path>",
        "--sampler-track <trackId>",
        "缺省 = 身份序第一条",
        "非主总线轨",
        "26 字符 ULID",
        "project.json",
        "主总线",
        "必须与 --track-sampler 一起给",
    ] {
        assert!(stdout.contains(needle), "`--help` 必须逐字写出 `{needle}`");
    }
    // 旧形状的示例行仍在（"示例"块里），⛔ 不因新开关被挤掉。
    assert!(
        stdout.contains("yeban-app --open song.yeban --track-sampler p.sfz --export-wav out.wav"),
        "旧用法的示例行必须还在"
    );
    assert!(
        stdout.contains("--sampler-track 01J8Z5Q0R7K3M9X2V4B6N8P0A0"),
        "新用法的示例行必须在（身份形状一眼可见）"
    );
}
