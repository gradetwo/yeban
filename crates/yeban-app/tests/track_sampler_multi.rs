//! ⭐ 任务单 `#331`：`--track-sampler` **可重复** —— 一次命令装**多份**乐器。
//!
//! # 缺陷（`#330` 报告逐字）
//!
//! ```text
//! --track-sampler 一次只能装一条轨（多乐器工程要跑 N 次命令 / 或只能手工点 GUI）
//! ```
//!
//! 本文件钉住**批量**这一半，⛔ 不碰渲染语义（装上去能不能出声是 `#286` 的判据）。
//!
//! # 量什么（对象 ＋ 单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `track-sampler:` 行 | 每份乐器**一行**（顺序 ＝ 命令行出现顺序） | 字符串（逐字） |
//! | `commit_count=` | 提交进提交史的次数 | 整数（期望 **2**，与单份基线**相同**） |
//! | 落盘后的 `TrackV3.sampler` | `--save-as` 容器里**每条**轨的绑定 | `Option<SamplerBinding>` |
//! | 退出码 | 真二进制 `yeban-app` 的进程退出码 | 整数（[`EXIT_USAGE`]） |
//!
//! # 为什么要钉 `commit_count`
//!
//! "N 份乐器 = N 次提交"是**能用但错**的实现：撤销栈里只有最后一次 ⇒ 用户按一次撤销
//! 只撤掉一件乐器，前面那些留在原地。本文件用 `commit_count=2`（＝ 打开工程那一次 ＋
//! **整批**那一次，与只装 1 份的基线**读数相同**）把"整批一次提交"钉成**可判据的
//! 事实**（⛔ 不是注释里的承诺）：真按 N 次提交实现，这里会读到 3、4、…
//!
//! # 判据
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | [`two_installs_land_on_two_tracks_in_one_commit`] | 两份乐器 ⇒ 两条报告行、两条轨各带自己的绑定、`commit_count=2`（＝ 一次提交） |
//! | [`pairing_follows_command_line_order_not_sorting`] | 位置配对：`a.sfz` 配**第一个** `--sampler-track`，⛔ 不按路径字典序 |
//! | [`count_mismatch_is_a_usage_error`] | 2 份 vs 1 个 ⇒ 退出码 2，报文点名"按出现位置一一配对" |
//! | [`multi_without_targets_is_a_usage_error`] | 2 份 vs 0 个 ⇒ 退出码 2（⛔ 不把两份装到同一条轨） |
//! | [`duplicate_target_track_is_a_usage_error`] | 同一身份两次 ⇒ 退出码 2，报文说清"后者会覆盖前者" |
//! | [`a_single_install_still_emits_exactly_one_line`] | 1 份 ＋ 1 个 ⇒ **一行**（`#287` 的形状逐字不变） |
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 不证"装两份都能出声"：两条轨都要有音符与路由，那是夹具之外的事；
//! - ⛔ 不钉撤销的**实现**（`UndoBackend`）：只读 `commit_count`／`undoable` 两个报告读数。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr as _;
use std::time::{Duration, Instant};

use yeban_app::cli::EXIT_USAGE;
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

/// 一次子进程执行允许的最长时间（与 `#287` 的判据同一条理由：不许把 CI 挂死）。
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

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
    /// 第一份 SFZ（名字里带 `a`，⛔ 不是字典序最小 —— 位置配对与排序在这里会分歧）。
    sfz_a: PathBuf,
    /// 第二份 SFZ。
    sfz_b: PathBuf,
    midi: EntityId,
    aux: EntityId,
    master: EntityId,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-331-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        // 一份 480 帧、常量 0.25 的 16-bit 单声道 WAV（与 `#287` 的夹具同形）。
        let samples: Vec<i16> = (0..480).map(|_| 8_192).collect();
        write_plain_wav(
            dir.join("tone.wav"),
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        // ⭐ 两份 SFZ：内容相同、**路径不同** ⇒ 落盘的 `SamplerBinding.path` 可区分
        //    （判据靠它读"哪份装在谁身上"，⛔ 不靠报告行的顺序蒙）。
        let sfz_a = dir.join("zz_first.sfz");
        let sfz_b = dir.join("aa_second.sfz");
        std::fs::write(&sfz_a, "<region> key=60 sample=tone.wav\n").expect("写夹具 SFZ a");
        std::fs::write(&sfz_b, "<region> key=60 sample=tone.wav\n").expect("写夹具 SFZ b");

        let midi = fixture_id(MIDI_TAIL);
        let aux = fixture_id(AUX_TAIL);
        let master = fixture_id(MASTER_TAIL);
        let project = fixture_project(master, midi, aux);
        let bytes = write_project_container(&project, b"yeban-331-history", &BTreeMap::new())
            .expect("写出真容器");
        let container = dir.join("fixture.yeban");
        std::fs::write(&container, &bytes).expect("写容器");
        Self {
            dir,
            container,
            sfz_a,
            sfz_b,
            midi,
            aux,
            master,
        }
    }

    fn text(&self, path: &Path) -> String {
        path.display().to_string()
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
fn fixture_project(master: EntityId, midi: EntityId, aux: EntityId) -> YebanProjectV1 {
    let mut project = YebanProjectV1 {
        id: fixture_id("P9"),
        title: "track sampler multi fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0331,
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

/// 真二进制执行（捕获 stdout / stderr；超时 ⇒ 杀掉并失败）。
///
/// # ⚠ 为什么**先起两个读线程**再 `try_wait`（⛔ 不是"等退出再收输出"）
///
/// 用法错误的 stderr 是**整份用法文本**（约 37 KB）。父进程若在 `try_wait` 循环里
/// 一直不读管道，子进程写满管道缓冲（容器里可能只有 8–16 KB）就会**阻塞在 write**，
/// 而父进程在等它退出 ⇒ 双向死锁 ⇒ 判据以"120 秒没退出"的形式**假红**
/// （实测：本文件 ⑤ 在 `#331` 第一版 harness 上必现；同样的命令手动重定向到文件
/// 却立刻退出 2 —— 差别就在"管道没人读"）。
///
/// 修法：两个 `read_to_end` 线程**一 spawn 就开读**，主线程只等退出 ＋ join。
/// ⚠ 既有 `#287` 的判据用的是旧形状：它的输出小（几千字节），碰不到这条死锁，
/// 本文件不复制那个隐患。
fn invoke(args: &[&str]) -> (i32, String, String) {
    use std::io::Read as _;
    let mut child = Command::new(BIN)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("无法执行 {BIN}: {error}"));
    let stdout = child.stdout.take().expect("stdout 已 piped");
    let stderr = child.stderr.take().expect("stderr 已 piped");
    let drain = |mut pipe: std::process::ChildStdout| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    };
    let stdout_reader = drain(stdout);
    let stderr_reader = std::thread::spawn(move || {
        let mut pipe = stderr;
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + RUN_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("`yeban-app {args:?}` 在 {RUN_TIMEOUT:?} 内没有退出");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let out = stdout_reader.join().expect("stdout 读线程");
    let err = stderr_reader.join().expect("stderr 读线程");
    (
        status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

/// 所有 `track-sampler:` 报告行（顺序即输出顺序）。
fn track_lines(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| line.starts_with("track-sampler:"))
        .collect()
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

/// 判据 ①：两份乐器 ⇒ 两条报告行、两条轨各带自己的绑定，**整批一次提交**。
#[test]
fn two_installs_land_on_two_tracks_in_one_commit() {
    let fixture = Fixture::new("two");
    let out = fixture.out("two.yeban");
    let (a, b) = (fixture.text(&fixture.sfz_a), fixture.text(&fixture.sfz_b));
    let (midi, aux) = (fixture.midi.to_string(), fixture.aux.to_string());
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--sampler-track",
        &midi,
        "--track-sampler",
        &a,
        "--sampler-track",
        &aux,
        "--track-sampler",
        &b,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[#331/two] rc={code}\n[#331/two] stdout:\n{stdout}\n[#331/two] stderr:\n{stderr}");
    assert_eq!(code, 0, "一次装两份必须成功；stderr={stderr}");
    let lines = track_lines(&stdout);
    println!("[#331/two] lines={lines:?}");
    assert_eq!(lines.len(), 2, "两份乐器 = 两条报告行: {lines:?}");
    assert!(
        lines[0].starts_with(&format!("track-sampler: track=Midi Lead path={a} ops=1 ")),
        "第 1 行对应第 1 对（midi ＋ zz_first.sfz）: {}",
        lines[0]
    );
    assert!(
        lines[1].starts_with(&format!("track-sampler: track=Aux Return path={b} ops=1 ")),
        "第 2 行对应第 2 对（aux ＋ aa_second.sfz）: {}",
        lines[1]
    );
    // ⭐⭐ **整批一次提交**的读数证据：`commit_count` 与"只装 1 份"（`#287` 的基线，
    //    同一份夹具）**完全相同** ＝ 2（打开工程那一次 ＋ 这一批那一次）。
    //    N 次提交会让它变成 3、4、…，而撤销栈里只剩最后一份 ⇒ 用户按一次撤销
    //    只撤掉最后一件乐器。判据 ⑥ 钉的正是"单份也恰好是 2"。
    for line in &lines {
        assert!(
            line.contains("commit_count=2 undoable=1"),
            "⭐⭐ 整批必须**一次**提交（N 次提交会让 commit_count > 2，撤销栈只剩最后一份）: {line}"
        );
    }
    let saved = reopen(&out);
    assert_eq!(
        sampler_of(&saved, fixture.midi),
        Some(SamplerBinding { path: a.clone() }),
        "midi 轨上必须是第一份"
    );
    assert_eq!(
        sampler_of(&saved, fixture.aux),
        Some(SamplerBinding { path: b.clone() }),
        "aux 轨上必须是第二份"
    );
    assert_ne!(
        sampler_of(&saved, fixture.master),
        Some(SamplerBinding { path: a }),
        "⛔ 主总线上不许有绑定"
    );
    let _ = fixture.master;
}

/// 判据 ②：**位置配对**，⛔ 不按路径字典序。
///
/// 夹具的路径故意让字典序与出现顺序**相反**（`zz_first.sfz` < `aa_second.sfz` 的说法
/// 反过来说就是：按字典序排，`aa_second` 会跑到前面）。这里把 `zz_first.sfz` **先**给、
/// 目标是 **aux**；`aa_second.sfz` 后给、目标是 **midi** ⇒ 落盘必须是
/// `aux→zz_first` / `midi→aa_second`。任何"按路径排序后配对"的实现都会把它对调。
#[test]
fn pairing_follows_command_line_order_not_sorting() {
    let fixture = Fixture::new("order");
    let out = fixture.out("order.yeban");
    let (a, b) = (fixture.text(&fixture.sfz_a), fixture.text(&fixture.sfz_b));
    let (midi, aux) = (fixture.midi.to_string(), fixture.aux.to_string());
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--sampler-track",
        &aux,
        "--track-sampler",
        &a,
        "--sampler-track",
        &midi,
        "--track-sampler",
        &b,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!(
        "[#331/order] rc={code}\n[#331/order] stdout:\n{stdout}\n[#331/order] stderr:\n{stderr}"
    );
    assert_eq!(code, 0, "位置配对必须成功；stderr={stderr}");
    let saved = reopen(&out);
    assert_eq!(
        sampler_of(&saved, fixture.aux),
        Some(SamplerBinding { path: a.clone() }),
        "第一个 `--track-sampler`（zz_first.sfz）配第一个 `--sampler-track`（aux）"
    );
    assert_eq!(
        sampler_of(&saved, fixture.midi),
        Some(SamplerBinding { path: b.clone() }),
        "第二个 `--track-sampler`（aa_second.sfz）配第二个 `--sampler-track`（midi）"
    );
}

/// 判据 ③：条数对不上 ⇒ 退出码 2，报文点名配对规则（⛔ 不静默忽略多余的那份）。
#[test]
fn count_mismatch_is_a_usage_error() {
    let fixture = Fixture::new("mismatch");
    let out = fixture.out("mismatch.yeban");
    let (a, b) = (fixture.text(&fixture.sfz_a), fixture.text(&fixture.sfz_b));
    let midi = fixture.midi.to_string();
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--sampler-track",
        &midi,
        "--track-sampler",
        &a,
        "--track-sampler",
        &b,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[#331/mismatch] rc={code}\n[#331/mismatch] stderr:\n{stderr}");
    assert_eq!(code, i32::from(EXIT_USAGE), "条数不等 = 用法错误");
    assert!(
        stderr.contains("2 份") && stderr.contains("1 个"),
        "报文必须报出两边的条数: {stderr}"
    );
    assert!(
        stderr.contains("按出现位置一一配对"),
        "报文必须说清配对规则: {stderr}"
    );
    assert!(
        !stdout.contains("track-sampler:") && !out.exists(),
        "⛔ 用法错误不许落任何东西（更不许装一半）: stdout={stdout}"
    );
}

/// 判据 ④：多份乐器但一个目标轨都不给 ⇒ 退出码 2（⛔ 不把两份装到同一条轨）。
#[test]
fn multi_without_targets_is_a_usage_error() {
    let fixture = Fixture::new("notarget");
    let out = fixture.out("notarget.yeban");
    let (a, b) = (fixture.text(&fixture.sfz_a), fixture.text(&fixture.sfz_b));
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--track-sampler",
        &a,
        "--track-sampler",
        &b,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[#331/notarget] rc={code}\n[#331/notarget] stderr:\n{stderr}");
    assert_eq!(code, i32::from(EXIT_USAGE), "多份无目标轨 = 用法错误");
    assert!(
        stderr.contains("2 份") && stderr.contains("0 个"),
        "报文必须报出两边的条数: {stderr}"
    );
    assert!(!out.exists(), "⛔ 不许把两份乐器都装到身份序第一条轨再落盘");
    assert!(
        !stdout.contains("track-sampler:"),
        "⛔ 不许有半批: {stdout}"
    );
}

/// 判据 ⑤：同一个目标轨被指定两次 ⇒ 退出码 2，报文说清"后者会覆盖前者"。
#[test]
fn duplicate_target_track_is_a_usage_error() {
    let fixture = Fixture::new("dup");
    let out = fixture.out("dup.yeban");
    let (a, b) = (fixture.text(&fixture.sfz_a), fixture.text(&fixture.sfz_b));
    let midi = fixture.midi.to_string();
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--sampler-track",
        &midi,
        "--track-sampler",
        &a,
        "--sampler-track",
        &midi,
        "--track-sampler",
        &b,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[#331/dup] rc={code}\n[#331/dup] stderr:\n{stderr}");
    assert_eq!(code, i32::from(EXIT_USAGE), "同一目标轨两次 = 用法错误");
    assert!(
        stderr.contains("指定了两次") && stderr.contains("覆盖"),
        "报文必须说清后果（后者覆盖前者 ⇒ 前一份静默不生效）: {stderr}"
    );
    assert!(!out.exists(), "⛔ 不许落一份'看起来装了两份'的工程");
    assert!(
        !stdout.contains("track-sampler:"),
        "⛔ 不许有半批: {stdout}"
    );
}

/// 判据 ⑥：只装 1 份（带目标轨）⇒ **恰好一行**，形状与 `#287` 逐字相同。
#[test]
fn a_single_install_still_emits_exactly_one_line() {
    let fixture = Fixture::new("single");
    let out = fixture.out("single.yeban");
    let a = fixture.text(&fixture.sfz_a);
    let aux = fixture.aux.to_string();
    let (code, stdout, stderr) = invoke(&[
        "--open",
        fixture.container.to_str().expect("UTF-8"),
        "--sampler-track",
        &aux,
        "--track-sampler",
        &a,
        "--save-as",
        out.to_str().expect("UTF-8"),
    ]);
    println!("[#331/single] rc={code}\n[#331/single] stdout:\n{stdout}");
    assert_eq!(code, 0, "单份必须成功；stderr={stderr}");
    let lines = track_lines(&stdout);
    assert_eq!(lines.len(), 1, "单份 = 一行: {lines:?}");
    assert!(
        lines[0].starts_with(&format!(
            "track-sampler: track=Aux Return path={a} ops=1 commit_count=2 undoable=1"
        )),
        "1 份的形状必须与 #287 逐字相同: {}",
        lines[0]
    );
    assert_eq!(
        sampler_of(&reopen(&out), fixture.aux),
        Some(SamplerBinding { path: a }),
        "单份落盘仍只在目标是的那条轨上"
    );
    assert_eq!(
        sampler_of(&reopen(&out), fixture.midi),
        None,
        "⛔ 单份不许碰别的轨"
    );
}
