//! `yeban-app` 命令行面的**真二进制**端到端判据。
//!
//! ## 为什么必须是"真二进制"
//!
//! `crates/yeban-app/src/cli.rs` 的单元判据（本机也用 `rustc --edition 2024 --test` 真跑过，
//! 见 `docs/ledger/app-cli-notes.md` §4）验证的是**逻辑**；这一份验证的是**进程**：
//! 真的 argv 进来、真的 stdout/stderr 出去、真的**退出码**。两者不可互相替代 ——
//! 文档（README / 官网 Quick Start）里写的是命令行，用户拿到的是退出码。
//!
//! 二进制的绝对路径在运行期解析（`common::bin_path`，见 `tests/common/mod.rs`）：`cargo test --all-targets`
//! 会先构建 `yeban-app` 这个 bin 目标，因此 **CI 的默认门禁**
//! （`.github/workflows/ci.yml` 的 `cargo test --workspace --all-targets`）就会跑到本文件 ——
//! 不需要任何人记得加一步。
//!
//! ## 只跑"无窗口"参数
//!
//! 本文件**绝不**调用不带无窗口开关的命令：那会构造真窗口并进入阻塞事件循环，把 CI 挂死。
//! 每条调用都带 `--headless` / `--headless-idle` / `--dump-elements` / `--export-elements` /
//! `--export-midi` / `--export-wav` / `--save-as` / `--print-shortcuts` / `--help` /
//! `--version` 之一。
//!
//! ⚠ 上面那句曾经**漏掉 `--export-wav`**（它当时不在 `Options::batch()` 里 ⇒
//! 不带 `--headless` 时走的是 GUI 路径）：本文件的
//! `export_wav_runs_on_the_windowless_path_from_the_real_binary` 就是那条缺口的
//! 回归判据 —— 它**刻意不带** `--headless`，钉的是"路由谓词自己"。
//!
//! `--headless-idle` 属于允许集合：它**不创建 OS 窗口**（平台是自研的
//! `MinimalSoftwareWindow` + `SoftwareRenderer`），而且空闲秒数上限（`MAX_IDLE_SECONDS`
//! = 60）刻意小于本文件的 [`RUN_TIMEOUT`]（120 秒）—— 于是"秒数敲错"会变成一条用法错误，
//! 而不是一次挂住。
//!
//! ## 规范来源 (Normative)
//!
//! - `[ARCH-SEC-004]`：`--save-as` 是"同目录临时文件 → fsync → rename"的原子替换；
//! - `[ARCH-SEC-003]` / `[MUST-GATE-006]` / `[MUST-GATE-007]`：容器拒绝的原因必须**原样**转达；
//! - `[ARCH-UI-003]` / `[UI-TEST-003]`：无头握手行 `headless ok`；
//! - `[UI-TEST-001]`：`--dump-elements` / `--export-elements` 的语义元素清单；
//! - `docs/adr/ADR-0001` D28（唯一注入点）/ D30（容器读法歧义一律拒绝）/
//!   **D43**（1.0.0 之前没有兼容包袱 ⇒ 裸 `project.json` 读路径已删除，容器是唯一格式）。

#![allow(unused)]
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod common;

use common::bin_path;

/// 一次子进程执行的全部可观测输出。
#[derive(Debug)]
struct Run {
    /// 退出码（被信号杀死时为 `-1`）。
    code: i32,
    /// stdout 全文。
    stdout: String,
    /// stderr 全文。
    stderr: String,
}

/// 一次子进程执行允许的最长时间。
///
/// 为什么必须有超时：本文件跑的是**真二进制**，如果某个"无窗口"开关因为回归被错误地
/// 送进了 GUI 路径，进程会进阻塞事件循环 —— `Command::output()` 会**永远**等下去，
/// 把 CI 挂在超时上限上（几十分钟），把一次明确的失败变成一次"卡住"。
/// 有了它，这种回归变成一次**响亮的判据失败**（并且进程被 kill 掉）。
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// 用给定参数跑一次真二进制（**只用无窗口参数**，见模块文档）。
fn invoke(args: &[&str]) -> Run {
    invoke_with_env(args, &[])
}

/// 同 [`invoke`]，但可以额外注入环境变量（`SLINT_BACKEND` 哨兵的判据用）。
fn invoke_with_env(args: &[&str], env: &[(&str, &str)]) -> Run {
    let bin = bin_path();
    let mut command = Command::new(&bin);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("无法执行 {bin:?}: {error}"));

    // ⭐ 两根读线程边跑边排空管道：子进程的 stdout 一旦超过 64 KiB 管道缓冲，
    // 没人读它就写阻塞 —— 旧写法只在退出后才 `wait_with_output()`，`--dump-elements`
    // （946 行、约 135 KiB）会在这里活活等满 120 秒超时。读线程让"输出大"不再等于"挂住"。
    let mut stdout_pipe = child.stdout.take().expect("取 stdout 管道");
    let mut stderr_pipe = child.stderr.take().expect("取 stderr 管道");
    let stdout_reader = std::thread::spawn(move || read_pipe_to_string(&mut stdout_pipe, "stdout"));
    let stderr_reader = std::thread::spawn(move || read_pipe_to_string(&mut stderr_pipe, "stderr"));

    let deadline = Instant::now() + RUN_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "`yeban-app {args:?}` 在 {RUN_TIMEOUT:?} 内没有退出 —— \
                         它多半进了阻塞事件循环 (无窗口开关不该开窗口)"
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };

    let stdout = stdout_reader.join().expect("stdout 读线程");
    let stderr = stderr_reader.join().expect("stderr 读线程");
    Run {
        code: status.code().unwrap_or(-1),
        stdout,
        stderr,
    }
}

/// 把一根管道读到字符串（读线程用；子进程写多少都不阻塞）。
fn read_pipe_to_string(pipe: &mut impl std::io::Read, name: &str) -> String {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)
        .unwrap_or_else(|error| panic!("读子进程 {name} 失败: {error}"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// 一次性的临时目录（**不用外部 crate**）。
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yeban-cli-bin-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 取某一行里 `key=value` 的 `value`（到下一个空白为止）。
fn field(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// 取以 `prefix` 开头的那一行。
fn line_with<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find(|line| line.starts_with(prefix))
}

/// 断言一次调用是**用法错误**：退出码 2、stdout 为空、stderr 点名原因并附上用法文本。
fn expect_usage(run: &Run, needle: &str) {
    assert_eq!(run.code, 2, "用法错误必须是退出码 2: {run:?}");
    assert!(run.stdout.is_empty(), "失败时 stdout 必须为空: {run:?}");
    assert!(
        run.stderr.contains(needle),
        "stderr 必须点名 `{needle}`: {run:?}"
    );
    assert!(
        run.stderr.contains("用法:"),
        "用法错误必须附上用法文本: {run:?}"
    );
}

/// 断言一次调用是 **`run_batch` 守卫**的拒绝：退出码 2、stdout 为空、stderr 点名原因。
///
/// 与 [`expect_usage`] 的差别：解析期错误会附上完整用法文本，`run_batch` 的
/// "必须与 `--export-wav` 一起给"守卫只印一行 `用法错误: …`、不附用法文本
/// （实测）。两类都是响亮的失败，判据按实测分开断，不把一种说成另一种。
fn expect_usage_reason(run: &Run, needle: &str) {
    assert_eq!(run.code, 2, "守卫拒绝必须是退出码 2: {run:?}");
    assert!(run.stdout.is_empty(), "失败时 stdout 必须为空: {run:?}");
    assert!(
        run.stderr.contains(needle),
        "stderr 必须点名 `{needle}`: {run:?}"
    );
}

/// 演示工程的音符键（通道， 音高， 力度， 起始 tick, 时值），按 (通道， 音高， 起始）排过序。
///
/// 与 `--export-midi` 的报告行、`parse_smf` 的回读逐项对账的事实源 —— 不抄
/// `cli.rs` 单元判据里的 `DEMO_MIDI_NOTES` 常量，同一份工程同一份口径。
fn demo_note_keys() -> Vec<(u8, u8, u8, u64, u64)> {
    let project = yeban_app::bridge::demo_project();
    let mut keys = Vec::new();
    for entry in project.clip_pool.values() {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        for note in notes.values() {
            keys.push((
                note.channel.unwrap_or(0),
                note.pitch,
                note.velocity,
                note.start_tick,
                note.duration_ticks,
            ));
        }
    }
    keys.sort_unstable();
    keys
}

/// 回读一份 `--export-wav` 的产物：断言它非静音，并返回（帧数， 采样率， 声道， 位深， 是否浮点）。
fn read_wav_shape(path: &Path) -> (u64, u32, u32, u32, bool) {
    use yeban_render::dither::PcmBuffer;
    use yeban_render::wav::read_plain_wav;
    let (format, buffer) = read_plain_wav(path).expect("写出的 WAV 必须可回读");
    let frames = u64::try_from(buffer.len() / usize::from(format.channels)).expect("帧数");
    let peak = match &buffer {
        PcmBuffer::Int16(samples) => samples
            .iter()
            .map(|sample| sample.unsigned_abs() as u64)
            .max()
            .unwrap_or(0),
        PcmBuffer::Int24(samples) => samples
            .iter()
            .map(|sample| sample.unsigned_abs() as u64)
            .max()
            .unwrap_or(0),
        PcmBuffer::Float32(samples) => u64::from(samples.iter().any(|sample| sample.abs() > 0.0)),
    };
    assert!(
        peak > 0,
        "导出的 WAV 不能是静音（{} peak = {peak}）",
        path.display()
    );
    (
        frames,
        format.sample_rate,
        u32::from(format.channels),
        u32::from(format.bits_per_sample),
        format.is_float,
    )
}

/// 从 `yeban_render` 的**公开**逐族常量里取某个 chunk 家族在**可读序**下的实测条数。
///
/// ⭐ `#199`（落实 `#197` 的建议①）：族条数不再抄字面量 —— 供体的逐族条数由
/// **产出它的那个 crate** 公开（`yeban_render::logic::LOGIC_OWNER_DONOR_3T_CHUNK_FAMILIES`），
/// 判据读它。这样容量 3 → 4 时，render 侧那份表随新供体一起改，本判据**自动跟随**，
/// 而"产物里的族条数 == 供体声明的族条数"**仍是真判据**（常量被改错 ⇒ 本判据红）。
///
/// ⚠ 落盘字节是**可读名的反序**（`Envi` 落盘为 `ivnE`），那个常量用可读序 ⇒ 这里翻转 4 字节再查表。
/// ⚠ 查不到就 panic（⛔ 不返回 `None`）：否则"产物里没有该族"与"常量表里没有该族"
/// 会一起退化成 `None == None`，把一条真判据变成**恒真**。
///
/// ⚠ **必须带 `#[cfg]`**：`yeban_render::logic` 那个模块本身在
/// `experimental-logic-export` / `experimental-logic-import` 后面
/// （`crates/yeban-render/src/lib.rs`）⇒ 默认特性腿里它**不存在**。
/// 不带这道门时默认腿是 `error[E0433]: cannot find `logic` in `yeban_render``
/// （本线实测：`clippy --lib --tests` / `check --all-targets` / `test --features midi-input` /
/// `test --features in-process-mcp` 四条**同时红**）。门与它服务的那条判据同款。
#[cfg(feature = "experimental-logic-export")]
fn donor_family_count(landed_tag: &[u8; 4]) -> usize {
    let readable = [landed_tag[3], landed_tag[2], landed_tag[1], landed_tag[0]];
    let name = std::str::from_utf8(&readable).expect("落盘 chunk 名必须是 ASCII");
    yeban_render::logic::LOGIC_OWNER_DONOR_3T_CHUNK_FAMILIES
        .iter()
        .find_map(|(tag, count)| (*tag == name).then_some(*count))
        .unwrap_or_else(|| {
            panic!("`LOGIC_OWNER_DONOR_3T_CHUNK_FAMILIES` 里必须有 `{name}`（`#199`）")
        })
}

/// 造一个**真容器**（含 `history.dag` 与一个 CAS 资产）并写盘。
///
/// 用 `yeban_model::container::write_project_container` —— 与生产写出路径**同一个**函数，
/// 因此判据吃进去的"坏输入"与生产能吐出来的"好输入"是同一批字节。
fn write_real_container(dir: &Path, name: &str) -> PathBuf {
    use std::collections::BTreeMap;
    use yeban_model::container::write_project_container;
    use yeban_model::ids::AssetHash;

    let project = yeban_app::bridge::demo_project();
    let assets: BTreeMap<AssetHash, Vec<u8>> = yeban_model::samples::demo_project_assets()
        .into_iter()
        .collect();
    let bytes = write_project_container(&project, b"binary-history", &assets).expect("写出真容器");
    let path = dir.join(name);
    std::fs::write(&path, &bytes).expect("写容器");
    path
}

/// 判据 B1: `--help` 与 `--version` 走短路路径、退出码 0、内容是真的。
#[test]
fn help_and_version_are_real_and_short_circuit() {
    let help = invoke(&["--help"]);
    assert_eq!(help.code, 0, "--help 必须是退出码 0: {help:?}");
    assert!(
        help.stderr.is_empty(),
        "--help 不该往 stderr 写东西: {help:?}"
    );
    assert!(help.stdout.contains("用法:"), "--help 必须是真的用法文本");
    assert!(help.stdout.contains("--open"), "--help 必须列出 --open");
    assert!(
        help.stdout.contains("--save-as"),
        "--help 必须列出 --save-as"
    );
    assert!(
        help.stdout.contains("--export-midi") && help.stdout.contains("--export-wav"),
        "--help 必须列出导出开关"
    );

    let version = invoke(&["--version"]);
    assert_eq!(version.code, 0, "--version 必须是退出码 0: {version:?}");
    assert!(
        version.stderr.is_empty(),
        "--version 不该往 stderr 写东西: {version:?}"
    );
    assert_eq!(
        version.stdout.trim(),
        format!("yeban-app {}", env!("CARGO_PKG_VERSION")),
        "--version 必须是 `yeban-app <版本号>`（与工作区清单同源）"
    );

    // 短路：`--help --bogus` 仍是 0 —— 未知开关不得抢在短路之前。
    let short = invoke(&["--help", "--bogus"]);
    assert_eq!(short.code, 0, "--help 必须短路掉后面的未知开关: {short:?}");
    assert!(short.stderr.is_empty(), "短路后 stderr 必须为空: {short:?}");
    assert!(
        short.stdout.contains("--save-as"),
        "短路后仍是真的 help 文本（不是一句空话）"
    );
}

/// 判据 B2: 无头握手行**恰好**出现一次，且工程来源被明写。
#[test]
fn headless_handshake_appears_exactly_once_and_names_the_source() {
    let run = invoke(&["--headless"]);
    assert_eq!(run.code, 0, "--headless 必须是退出码 0: {run:?}");
    assert!(
        run.stderr.is_empty(),
        "--headless 不该往 stderr 写东西: {run:?}"
    );
    assert_eq!(
        run.stdout
            .lines()
            .filter(|line| *line == "headless ok")
            .count(),
        1,
        "无头握手行必须**恰好**出现一次:\n{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("project-source: sample=default (内置演示工程; 未给 --open ⇒ 未读任何文件)"),
        "工程来源必须明写是演示工程、未读任何文件:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("未构造 MainWindow"),
        "边界声明必须出现（没有建窗口）:\n{}",
        run.stdout
    );
}

/// 判据 B2b: `SLINT_BACKEND=headless` **单独**（不给任何参数）也等价于 `--headless`。
///
/// 这条判据的价值在于它真的能变红：哨兵失效 ⇒ 进程会去开窗口 —— 在无显示器环境
/// 直接失败，在有显示器环境进阻塞事件循环（由 [`RUN_TIMEOUT`] 变成一次响亮失败）。
#[test]
fn slint_backend_headless_sentinel_alone_keeps_the_window_closed() {
    let run = invoke_with_env(&[], &[("SLINT_BACKEND", "headless")]);
    assert_eq!(run.code, 0, "哨兵单独给时必须是退出码 0: {run:?}");
    assert!(
        run.stderr.is_empty(),
        "哨兵路径不该往 stderr 写东西: {run:?}"
    );
    assert_eq!(
        run.stdout
            .lines()
            .filter(|line| *line == "headless ok")
            .count(),
        1,
        "哨兵必须等价于 --headless（握手行恰好一次）:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("project-source: sample=default"),
        "哨兵路径的工程来源必须是演示工程:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("未构造 MainWindow"),
        "哨兵路径必须没有建窗口:\n{}",
        run.stdout
    );
}

/// 判据 B3b: `--project-sample empty` 在**真二进制**上是 0 条音轨 ＋ 1 条主总线，
/// `default` 是 6 条普通轨。
///
/// 为什么这条必须走真二进制：`BASELINE-002` 的读数是**进程级**的
/// （`scripts/gates/measure_rss.py -- <本二进制> --project-sample empty --headless`），
/// 所以"取样对象真的是空工程"必须在同一条进程命令上被钉住 —— 单元判据证明不了 argv 与
/// 二进制一致。`tracks-all` **含**主总线，因此普通轨数 = `tracks-all - master-track`。
#[test]
fn empty_sample_reports_zero_audio_tracks_and_default_reports_six() {
    use yeban_app::cli::Sample;
    let empty = Sample::Empty.project();
    assert_eq!(empty.tracks.len(), 1, "empty 必须恰有 1 条轨（主总线）");
    assert_eq!(
        empty.tracks.len() - 1,
        0,
        "empty 的**非主总线轨**必须是 0 条"
    );
    let demo = Sample::Default.project();
    assert_eq!(demo.tracks.len(), 7, "演示夹具的轨道数变了 ⇒ 本判据需同步");
    assert_eq!(demo.tracks.len() - 1, 6, "default 的普通轨必须是 6 条");

    let empty_run = invoke(&["--project-sample", "empty", "--headless"]);
    assert_eq!(
        empty_run.code, 0,
        "--project-sample empty 必须是退出码 0: {empty_run:?}"
    );
    assert!(
        empty_run
            .stdout
            .contains("project-counts: tracks-all=1 master-track=1"),
        "empty 的轨道数见证必须是 1（1 主总线 ＋ 0 音轨）:\n{}",
        empty_run.stdout
    );
    assert!(
        empty_run
            .stdout
            .contains("project-source: sample=empty (内置空工程 0 音轨 ＋ 1 主总线"),
        "来源行必须如实说明这是空工程（不许写成演示工程）:\n{}",
        empty_run.stdout
    );
    assert!(
        empty_run.stdout.contains("view-counts: tracks=0 master=1"),
        "0 音轨空工程必须**能**被投影（主总线投影到 `master`）:\n{}",
        empty_run.stdout
    );

    let demo_run = invoke(&["--project-sample", "default", "--headless"]);
    assert!(
        demo_run
            .stdout
            .contains("project-counts: tracks-all=7 master-track=1"),
        "default 的对照读数必须是 7（6 普通 + 1 主总线）:\n{}",
        demo_run.stdout
    );
    assert_ne!(
        empty_run.stdout, demo_run.stdout,
        "两个样本必须给出不同的读数（相同 ⇒ 模式没生效，见证是空转）"
    );
}

/// 判据 B3c: `--headless-idle --idle-seconds N` **真的**构造了 Slint 控件树。
///
/// 为什么这条必须是**真二进制**判据：`BASELINE-002` 的读数取自
/// `scripts/gates/measure_rss.py -- <本二进制> --headless-idle --idle-seconds N`，
/// 所以"这个进程真的建了树"只能在**同一条进程命令**上钉住 —— 单元判据（`cli.rs`）只能
/// 证明参数被解析、格式化行长什么样。
///
/// 见证是**量出来的**，不是断言出来的（见 `src/headless_idle.rs` 的模块文档）：
/// `windows-created=1`（平台自数）、`rendered=true`、`lines=`（真的光栅化过的行数）、
/// `non-black-pixels>0` + `distinct-colors>=2`（像素伪造不了）。任何一项退化成 0 / false，
/// 这条就红 —— 那正是"没建树"与"未生效"两种失败的样子。
#[test]
fn headless_idle_builds_a_real_control_tree_and_reports_a_non_trivial_witness() {
    let run = invoke(&["--headless-idle", "--idle-seconds", "1"]);
    assert_eq!(run.code, 0, "--headless-idle 必须是退出码 0: {run:?}");
    assert!(
        run.stderr.is_empty(),
        "--headless-idle 不该往 stderr 写东西: {run:?}"
    );
    let witness = line_with(&run.stdout, "headless-idle-witness:")
        .expect("必须有 headless-idle-witness 见证行");
    assert_eq!(
        field(witness, "windows-created").as_deref(),
        Some("1"),
        "必须真的建了一个窗口对象: {witness}"
    );
    assert_eq!(
        field(witness, "size").as_deref(),
        Some("1920x1080"),
        "见证视口必须是 1920x1080: {witness}"
    );
    assert_eq!(
        field(witness, "rendered").as_deref(),
        Some("true"),
        "必须真的渲染了一帧: {witness}"
    );
    let lines: usize = field(witness, "lines")
        .as_deref()
        .expect("lines")
        .parse()
        .expect("lines 必须是数字");
    assert_eq!(lines, 1080, "必须逐行光栅化 1080 行: {witness}");
    let pixels: usize = field(witness, "non-black-pixels")
        .as_deref()
        .expect("non-black-pixels")
        .parse()
        .expect("non-black-pixels 必须是数字");
    assert!(
        pixels > 0,
        "必须真的画了非黑像素（空转会被判红）: {witness}"
    );
    let permille: usize = field(witness, "non-black-permille")
        .as_deref()
        .expect("non-black-permille")
        .parse()
        .expect("non-black-permille 必须是数字");
    assert!(
        (0..=1000).contains(&permille),
        "千分比必须在 0..=1000 内: {witness}"
    );
    let colors: usize = field(witness, "distinct-colors")
        .as_deref()
        .expect("distinct-colors")
        .parse()
        .expect("distinct-colors 必须是数字");
    assert!(colors > 1, "画面必须多于一种颜色: {witness}");
    let elements: usize = field(witness, "elements")
        .as_deref()
        .expect("elements")
        .parse()
        .expect("elements 必须是数字");
    assert!(elements > 0, "控件树必须非空: {witness}");
    // 见证的 elements 必须与同一命令的 view-counts 同口径。
    let view = line_with(&run.stdout, "view-counts:").expect("必须有 view-counts 行");
    assert_eq!(
        field(view, "elements").as_deref(),
        Some(elements.to_string().as_str()),
        "见证的 elements 必须与投影一致: {view} / {witness}"
    );
    let idle = line_with(&run.stdout, "headless-idle-idle:").expect("必须有空闲行");
    assert_eq!(
        field(idle, "seconds").as_deref(),
        Some("1"),
        "空闲行必须报出请求的秒数: {idle}"
    );
    let ticks: usize = field(idle, "ticks")
        .as_deref()
        .expect("ticks")
        .parse()
        .expect("ticks 必须是数字");
    assert!(ticks > 0, "空闲循环必须真的走了 tick: {idle}");
    assert!(
        run.stdout.contains("已构造真 MainWindow"),
        "必须声明构造了真 MainWindow（不是降级成不建树）:\n{}",
        run.stdout
    );
}

/// 判据 B3: `--open` 真容器 ⇒ 打印的读数与工程一致（轨道 / 音符 / 历史 / 资产）。
#[test]
fn open_reports_the_real_project_counts() {
    let dir = scratch_dir("open-counts");
    let path = write_real_container(&dir, "song.yeban");
    let run = invoke(&["--open", &path.display().to_string(), "--headless"]);
    assert_eq!(run.code, 0, "--open 必须是退出码 0: {run:?}");
    assert!(
        run.stderr.is_empty(),
        "--open 不该往 stderr 写东西: {run:?}"
    );

    let project = yeban_app::bridge::demo_project();
    assert_eq!(
        project.tracks.len(),
        7,
        "演示夹具的轨道数变了 ⇒ 本判据需同步"
    );
    let counts = line_with(&run.stdout, "project-counts:").expect("必须有 project-counts 行");
    assert_eq!(
        field(counts, "tracks-all").as_deref(),
        Some(project.tracks.len().to_string().as_str()),
        "打印的轨道数必须与工程一致: {counts}"
    );
    assert_eq!(
        field(counts, "master-track").as_deref(),
        Some("1"),
        "工程有主总线: {counts}"
    );
    assert_eq!(
        field(counts, "history-bytes").as_deref(),
        Some("14"),
        "夹具的 history.dag 是 binary-history（14 字节）: {counts}"
    );
    assert_eq!(
        field(counts, "asset-blobs").as_deref(),
        Some("1"),
        "资产池必须如实报告: {counts}"
    );
    assert_eq!(
        field(counts, "midi-notes").as_deref(),
        Some("6"),
        "演示夹具的六颗音符: {counts}"
    );
    assert!(
        run.stdout.contains("format=yeban-container"),
        "报告必须明说容器格式:\n{}",
        run.stdout
    );
    let opened = line_with(&run.stdout, "opened:").expect("必须有 opened 行");
    assert!(
        opened.contains("song.yeban"),
        "opened 行必须点名打开的文件: {opened}"
    );
    assert_eq!(
        field(opened, "format").as_deref(),
        Some("yeban-container"),
        "opened 行必须报出容器格式: {opened}"
    );
    let view = line_with(&run.stdout, "view-counts:").expect("--headless 必须给出 view-counts");
    assert_eq!(
        field(view, "tracks").as_deref(),
        Some((project.tracks.len() - 1).to_string().as_str()),
        "投影的轨道数 = 工程轨道数 - 主总线: {view}"
    );
    assert_eq!(
        field(view, "notes").as_deref(),
        Some("6"),
        "界面投影的音符数必须与工程同口径: {view}"
    );
    assert_eq!(
        field(view, "master").as_deref(),
        Some("1"),
        "主总线必须出现在投影里: {view}"
    );
    assert_eq!(
        run.stdout
            .lines()
            .filter(|line| *line == "headless ok")
            .count(),
        1,
        "无头握手行必须恰好一次:\n{}",
        run.stdout
    );
    assert!(run.stdout.contains("未构造 MainWindow"), "边界声明必须出现");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B4: 坏输入 ⇒ **非零退出码 + 精确原因**，不是 panic、不是空工程。
#[test]
fn broken_inputs_exit_non_zero_without_panicking() {
    let dir = scratch_dir("broken-inputs");
    let source = write_real_container(&dir, "good.yeban");
    let bytes = std::fs::read(&source).expect("读容器");
    let truncated = dir.join("truncated.yeban");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).expect("写截断容器");
    let junk = dir.join("junk.bin");
    std::fs::write(&junk, b"not a zip").expect("写垃圾");
    let empty = dir.join("empty.bin");
    std::fs::write(&empty, b"").expect("写空文件");

    // 截断容器：退出码 3，容器裁决原样转达。
    let run = invoke(&["--open", &truncated.display().to_string(), "--headless"]);
    assert_eq!(run.code, 3, "截断容器必须是退出码 3: {run:?}");
    assert!(run.stdout.is_empty(), "失败时 stdout 必须为空: {run:?}");
    assert!(
        run.stderr.contains("容器被拒绝"),
        "必须转达容器裁决: {run:?}"
    );
    assert!(
        run.stderr.contains("end-of-central-directory"),
        "必须是**精确**的容器原因: {run:?}"
    );

    // 非容器 / 空文件：明确拒绝，理由精确（不是"未知格式"、不是空工程）。
    for bad in [&junk, &empty] {
        let run = invoke(&["--open", &bad.display().to_string(), "--headless"]);
        assert_eq!(run.code, 3, "非容器必须是退出码 3: {run:?}");
        assert!(run.stdout.is_empty(), "失败时 stdout 必须为空: {run:?}");
        assert!(run.stderr.contains("不是 `.yeban` 容器"), "实测: {run:?}");
        assert!(
            run.stderr.contains("end-of-central-directory"),
            "必须带上容器的精确原裁决: {run:?}"
        );
    }

    // 不存在的文件 / 目录：都是精确的 I/O 裁决，都不是 panic。
    let missing = dir.join("nope.yeban");
    for bad in [&missing, &dir] {
        let run = invoke(&["--open", &bad.display().to_string(), "--headless"]);
        assert_eq!(run.code, 3, "打不开必须是退出码 3: {run:?}");
        assert!(run.stdout.is_empty(), "失败时 stdout 必须为空: {run:?}");
        assert!(
            run.stderr.contains("无法读取"),
            "实测: {run:?} (path={})",
            bad.display()
        );
        assert!(
            !run.stderr.contains("panicked"),
            "坏输入绝不能 panic: {run:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B5: `--open` + `--save-as` ⇒ 落盘文件能被公开入口读回，且**归档等价**；
/// 两次保存的字节完全相同（确定性）。
/// 本判据覆盖 round-trip 路径；样本路径（无 `--open`）见
/// `save_as_sample_is_deterministic_except_created_at`。
#[test]
fn save_as_round_trips_and_is_deterministic_from_the_real_binary() {
    let dir = scratch_dir("save");
    let source = write_real_container(&dir, "in.yeban");
    let first = dir.join("out-a.yeban");
    let second = dir.join("out-b.yeban");
    let source = source.to_str().expect("utf8");

    let mut printed_bytes = Vec::new();
    for target in [&first, &second] {
        let run = invoke(&[
            "--open",
            source,
            "--save-as",
            target.to_str().expect("utf8"),
        ]);
        assert_eq!(run.code, 0, "stderr={}", run.stderr);
        let saved = line_with(&run.stdout, "saved:").expect("必须有 saved: 行");
        assert!(saved.contains("assets=1"), "资产池不得静默丢失: {saved}");
        assert!(
            saved.contains("history-bytes=14"),
            "history.dag 必须被保真写出: {saved}"
        );
        let expected = std::fs::metadata(target).expect("文件在").len();
        assert_eq!(
            field(saved, "bytes").as_deref(),
            Some(expected.to_string().as_str()),
            "打印的字节数必须是实际落盘字节数: {saved}"
        );
        printed_bytes.push(field(saved, "bytes").expect("bytes="));
    }
    assert_eq!(printed_bytes[0], printed_bytes[1], "两次保存字节数必须相同");
    assert_eq!(
        std::fs::read(&first).expect("读 a"),
        std::fs::read(&second).expect("读 b"),
        "同一输入两次 --save-as 的字节必须完全相同"
    );

    // 公开入口读回：工程等价 **且** 归档等价（历史 / 资产都没丢）。
    let read_back = yeban_app::open::open_project_file(&first).expect("读回");
    assert_eq!(read_back, yeban_app::bridge::demo_project());
    let archive = yeban_app::open::open_project_archive_file(
        &first,
        &yeban_app::open::ProjectOpenOptions::default(),
    )
    .expect("全保真读回");
    assert_eq!(archive.history_dag, b"binary-history");
    assert_eq!(archive.assets.len(), 1);
    // 所有 `*.tmp-*` 都必须已被重命名掉（原子替换的痕迹）。
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B5（样本路径补充）：无 `--open` 的两次 `--save-as`（内置样本）⇒
/// 除 `project.json` 的 `metadata.created_at_unix_ms` 外字节完全相同。
///
/// 为什么不是字节 `==`：样本 `--save-as` 是**创建**一份新工程（#215），
/// `load_project_with_clock` 在样本分支只取一次时钟，写的是真实墙钟
/// （`crates/yeban-app/src/cli.rs` 的样本分支；B6 判据钉了 `created_at_unix_ms > 0`，
/// 因为 BWF 起源日期需要它）。两次运行的墙钟不同 ⇒ `project.json` 里的这一字段不同，
/// 其余内容（`history.dag`、资产池 blob、`project.json` 的其余字段）必须逐字节相同。
/// 台账 `docs/ledger/app-cli-notes.md:216` 判据 ⑦ 的字节 `==` 只覆盖 round-trip 路径。
#[test]
fn save_as_sample_is_deterministic_except_created_at() {
    let dir = scratch_dir("sample-determinism");
    let first = dir.join("sample-a.yeban");
    let second = dir.join("sample-b.yeban");

    let mut printed_bytes = Vec::new();
    for target in [&first, &second] {
        let run = invoke(&["--save-as", target.to_str().expect("utf8")]);
        assert_eq!(run.code, 0, "stderr={}", run.stderr);
        let saved = line_with(&run.stdout, "saved:").expect("必须有 saved: 行");
        let expected = std::fs::metadata(target).expect("文件在").len();
        assert_eq!(
            field(saved, "bytes").as_deref(),
            Some(expected.to_string().as_str()),
            "打印的字节数必须是实际落盘字节数: {saved}"
        );
        printed_bytes.push(field(saved, "bytes").expect("bytes="));
    }
    assert_eq!(printed_bytes[0], printed_bytes[1], "两次保存字节数必须相同");

    // 逐条目比对容器：条目名/顺序相同；除 project.json 外逐字节相同；
    // project.json 解析后除 created_at_unix_ms 外逐字段相同。
    let (names_a, entries_a) = read_container_entries(&first);
    let (names_b, entries_b) = read_container_entries(&second);
    assert_eq!(names_a, names_b, "两次保存的容器条目名/顺序必须相同");
    let mut stamps = Vec::new();
    for name in &names_a {
        if name == "project.json" {
            let mut a: serde_json::Value =
                serde_json::from_slice(&entries_a[name]).expect("project.json 必须是 JSON");
            let mut b: serde_json::Value =
                serde_json::from_slice(&entries_b[name]).expect("project.json 必须是 JSON");
            let stamp = |v: &serde_json::Value| {
                v.get("metadata")
                    .and_then(|m| m.get("created_at_unix_ms"))
                    .and_then(serde_json::Value::as_u64)
            };
            let stamp_a = stamp(&a);
            let stamp_b = stamp(&b);
            assert!(
                stamp_a.is_some_and(|v| v > 0),
                "第一次保存必须写真实创建时刻（实得 {stamp_a:?} ⇒ BWF 起源日期会退回 1970-01-01）"
            );
            assert!(
                stamp_b.is_some_and(|v| v > 0),
                "第二次保存必须写真实创建时刻（实得 {stamp_b:?} ⇒ BWF 起源日期会退回 1970-01-01）"
            );
            stamps.push((stamp_a.unwrap(), stamp_b.unwrap()));
            for v in [&mut a, &mut b] {
                if let Some(meta) = v
                    .get_mut("metadata")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    meta.remove("created_at_unix_ms");
                }
            }
            assert_eq!(a, b, "project.json 除 created_at_unix_ms 外必须逐字段相同");
        } else {
            assert_eq!(
                entries_a[name], entries_b[name],
                "条目 {name} 必须逐字节相同（history.dag / 资产池 blob 也不例外）"
            );
        }
    }
    // 两次的创建时刻都是 13 位墙钟毫秒（BWF 起源日期需要真实时间，见 B6）。
    for (a, b) in stamps {
        assert!(
            (1_000_000_000_000..10_000_000_000_000).contains(&a),
            "创建时刻必须是 13 位墙钟毫秒: {a}"
        );
        assert!(
            (1_000_000_000_000..10_000_000_000_000).contains(&b),
            "创建时刻必须是 13 位墙钟毫秒: {b}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// 把 `.yeban` 容器按条目读成（条目名顺序，条目名 → 原始字节）。
fn read_container_entries(
    path: &Path,
) -> (Vec<String>, std::collections::BTreeMap<String, Vec<u8>>) {
    use std::io::Read as _;
    let file = std::fs::File::open(path).expect("打开容器");
    let mut archive = zip::ZipArchive::new(file).expect("容器必须是 zip");
    let mut names = Vec::new();
    let mut entries = std::collections::BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("条目在");
        let name = entry.name().expect("条目名").into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("读条目");
        names.push(name.clone());
        entries.insert(name, bytes);
    }
    (names, entries)
}

/// 判据 B6: `--save-as` 不给 `--open` ⇒ 保存演示工程，并且**在输出里明说**这一点。
#[test]
fn save_as_without_open_says_it_saved_the_demo_project() {
    let dir = scratch_dir("sample");
    let target = dir.join("demo.yeban");
    let run = invoke(&["--save-as", target.to_str().expect("utf8")]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("project-source: sample=default"),
        "必须明写工程来自内置样本:\n{}",
        run.stdout
    );
    assert!(run.stdout.contains("未读任何文件"), "{}", run.stdout);
    let saved = line_with(&run.stdout, "saved:").expect("saved:");
    assert!(
        saved.contains("from=sample=default") && saved.contains("不是从文件打开的"),
        "saved: 行必须说清来源: {saved}"
    );
    // ⭐ `#215`：`--save-as` 不给 `--open` 是**创建**一份新工程 ⇒ 创建时刻写真实时间。
    // 因此对账口径是"除创建时刻外逐字段等于演示工程"，并单独断言时刻真的非 0。
    let read_back = yeban_app::open::open_project_file(&target).expect("读回");
    assert!(
        read_back.metadata.created_at_unix_ms > 0,
        "新建工程必须写真实创建时刻（实得 0 ⇒ BWF 起源日期会退回 1970-01-01）"
    );
    let mut expected = yeban_app::bridge::demo_project_for_playback();
    expected.metadata.created_at_unix_ms = read_back.metadata.created_at_unix_ms;
    assert_eq!(read_back, expected, "除创建时刻外必须逐字段等于演示工程");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B7: 未知开关 / 缺取值 ⇒ 退出码 2 + **用法提示**（不许静默忽略）。
#[test]
fn unknown_switches_exit_two_with_usage_on_stderr() {
    // 实测过的每一种：退出码 2 + 点名原因 + 用法文本。
    for (args, needle) in [
        (&["--bogus"][..], "无法识别的参数 `--bogus`"),
        (&["--open"][..], "`--open` 需要一个取值"),
        (&["--open", "--headless"][..], "`--open` 需要一个取值"),
        (&["--headless=1"][..], "`--headless` 不接受取值"),
        (&["--project-sample", "nope"][..], "未知的工程样本 `nope`"),
        (&["--open", "a", "--open", "b"][..], "`--open` 只能给一次"),
        (&["--export-midi"][..], "`--export-midi` 需要一个取值"),
        (&["--open="][..], "`--open` 需要一个取值"),
    ] {
        let run = invoke(args);
        expect_usage(&run, needle);
    }

    // 其余形态（位置参数 / 缺值 / 重复给）：同样是用法错误，只不断言措辞。
    for args in [
        vec!["filled"],
        vec!["--save-as"],
        vec!["--save-as", "a", "--save-as", "b"],
        vec!["--export-elements", "a", "--export-elements", "b"],
        vec!["--export-midi", "a", "--export-midi", "b"],
    ] {
        let run = invoke(&args);
        assert_eq!(run.code, 2, "{args:?} 必须是退出码 2: {run:?}");
        assert!(run.stdout.is_empty(), "失败时 stdout 必须为空: {run:?}");
        assert!(!run.stderr.is_empty(), "错误必须有可读原因: {run:?}");
        assert!(
            run.stderr.contains("用法:"),
            "用法错误必须附上用法文本: {run:?}"
        );
    }

    // 用法文本真的有内容可提示（未知开关的名字当然不在里面）。
    let usage = yeban_app::cli::usage_text();
    assert!(usage.contains("--save-as"), "用法文本必须提到 --save-as");
    assert!(
        !usage.contains("无法识别的参数"),
        "用法文本不该自带一条报错"
    );
}

/// 判据 B7b: **未编译该 feature 的产物层面**拒绝 `--enable-mcp-http`（`[ROAD-M4-001]` / `[MUST-GATE-009]`）。
///
/// 这是"未编译的危险能力必须点名拒绝"在 **app 二进制产物**上的断言（与
/// `scripts/gates/check_release_defaults.sh` 对 `yeban-mcp` 二进制做的那一条同一条纪律、
/// 另一个产物）：未编译 `in-process-mcp` 的构建里（`--no-default-features`；
/// 该 feature 自 2026-11-12 负责人裁决起为默认，ADR-0072），这个开关**必须**以用法错误
/// （退出码 2）被拒，并**点名**缺的是哪个 feature —— 不许静默地"以为开了其实没开"。
///
/// 为什么只在未编译该 feature 的构建里跑：带 `in-process-mcp` 时它是**合法**的 GUI 开关，给它会走
/// 开窗口那一档，而本文件只用无窗口参数（见模块文档的纪律）。带 feature 的正面判据在
/// `tests/in_process_mcp.rs`（真环回 socket 上的完整往返）。
#[cfg(not(feature = "in-process-mcp"))]
#[test]
fn default_artifact_refuses_the_in_process_mcp_switch() {
    // ⚠ 本体只在 `--no-default-features`（不带 `in-process-mcp`）的构建里编译：
    // 默认构建里 `--enable-mcp-http` 是合法的 GUI 开关，不是拒绝项。
    // （基线 9e5fb9f7 上该构建目前跑不起来：lib 在无默认特性下有 24 处 E0433，
    // 且自引用 dev-dependency 会把默认特性重新拉回任何 `cargo test` —— 见回执。）
    let run = invoke(&["--enable-mcp-http"]);
    expect_usage(
        &run,
        "`--enable-mcp-http` 需要本次构建带 `--features in-process-mcp`",
    );
    // `--flag=value` 同样被拒（不接受取值在特性门之前判）。
    let valued = invoke(&["--enable-mcp-http=1"]);
    assert_eq!(
        valued.code, 2,
        "--enable-mcp-http=1 必须是退出码 2: {valued:?}"
    );
    assert!(
        valued.stdout.is_empty(),
        "失败时 stdout 必须为空: {valued:?}"
    );
    assert!(
        valued.stderr.contains("`--enable-mcp-http` 不接受取值"),
        "实测: {valued:?}"
    );
}

/// 判据 B7c-2: **默认产物层面**拒绝 `--enable-ui-mcp-http`（`[ARCH-UI-004]` / `[MUST-GATE-009]`）。
///
/// 与 B7c（`--enable-mcp-http`）逐条同款，因为两条控制面说的是同一条红线：默认 release
/// 构建里**不得**默认开启这类能力，因此"开了但没有那段代码"必须是一次**点名 feature 的**
/// 用法错误（退出码 2），而不是静默忽略 —— 静默忽略会让人以为环回控制面起来了。
///
/// 只在**不带** `ui-mcp-http` 的构建里存在：带了那个 feature 时这个开关会真的去
/// 建控件树 + 绑 socket（那是产品行为，不是用法错误），所以判据必须 cfged out。
#[cfg(not(feature = "ui-mcp-http"))]
#[test]
fn default_artifact_refuses_the_ui_mcp_http_switch() {
    let run = invoke(&["--enable-ui-mcp-http"]);
    expect_usage(
        &run,
        "`--enable-ui-mcp-http` 需要本次构建带 `--features ui-mcp-http`",
    );
}

/// 判据 B7d: **默认产物层面**拒绝 `--export-logic`（`[ARCH-FMT-002]` / `[ROAD-M4-007]`）。
///
/// 与 B7c 同一条纪律、另一个产物形态（`.logicx` 是**目录**而不是单文件）：默认构建里
/// `experimental-logic-export` 没被编译，于是这个开关**必须**以用法错误（退出码 2）被拒，
/// 并**点名**缺的是哪个 feature。
#[cfg(not(feature = "experimental-logic-export"))]
#[test]
fn default_artifact_refuses_the_experimental_logic_switch() {
    let run = invoke(&["--export-logic", "x.logicx"]);
    expect_usage(
        &run,
        "`--export-logic` 需要本次构建带 `--features experimental-logic-export`",
    );
}

/// 判据 B7c: **默认产物层面**拒绝 `--export-als`（`[ARCH-FMT-002]` / `[ROAD-M4-007]`）。
///
/// 与 B7b 同一条纪律、另一个产物形态：默认构建里 `experimental-als-export` 没被编译
/// （那条导出路径与可选的 `flate2` 都不在依赖图上），于是这个开关**必须**以用法错误
/// （退出码 2）被拒，并**点名**缺的是哪个 feature —— 绝不静默地"以为写了其实没写"。
///
/// 为什么只在默认构建里跑：带 feature 时它是合法的无窗口开关，正面判据是同文件的
/// `export_als_writes_a_gzip_document_and_prints_the_loss_summary`。
#[cfg(not(feature = "experimental-als-export"))]
#[test]
fn default_artifact_refuses_the_experimental_als_switch() {
    let run = invoke(&["--export-als", "x.als"]);
    expect_usage(
        &run,
        "`--export-als` 需要本次构建带 `--features experimental-als-export`",
    );
}

/// 判据 B7d: **未编译该 feature 的产物层面**拒绝 `--export-song-overview`。
///
/// 与 B7b/B7c 同一条纪律：这个出口的产物就是 `get_song_overview` 的 `data`
/// （§8.2 的 structs 与取整函数 `rounded` 住在 `yeban-mcp`），未编译 `in-process-mcp`
/// 的构建里（`--no-default-features`）那条依赖边
/// 不存在 ⇒ 必须是以用法错误（退出码 2）被拒、并**点名**缺的 feature，
/// 绝不静默地"以为写了其实没写"。
///
/// 为什么只在未编译该 feature 的构建里跑：带 feature 时它是合法的无窗口开关，正面判据是同包的
/// `tests/song_overview_export.rs`（它拿批处理出口与工具出口逐字节对账）。
#[cfg(not(feature = "in-process-mcp"))]
#[test]
fn default_artifact_refuses_the_song_overview_switch() {
    // ⚠ 本体只在 `--no-default-features`（不带 `in-process-mcp`）的构建里编译，
    // 与 `default_artifact_refuses_the_in_process_mcp_switch` 同一条执行限制（见回执）。
    let run = invoke(&["--export-song-overview", "overview.json"]);
    expect_usage(
        &run,
        "--export-song-overview 需要本次构建带 `--features in-process-mcp`",
    );
}

/// 判据 B7e: **未编译该 feature 的产物层面**拒绝 `--apply-ops`。
///
/// 与 B7d 同一条纪律：执行管线（`parse_ops` / `compile` / `UndoPort` 提交）全住在
/// `yeban-mcp`，未编译 `in-process-mcp` 的构建里（`--no-default-features`）那条依赖边不存在 ⇒ 必须是以用法错误（退出码 2）被拒、并
/// **点名**缺的 feature，绝不静默地"以为改了其实没改"。
///
/// 为什么只在未编译该 feature 的构建里跑：带 feature 时它是合法的无窗口开关，正面判据是同包的
/// `tests/apply_ops.rs`（改名走 `--save-as` 回读即是新名）。
#[cfg(not(feature = "in-process-mcp"))]
#[test]
fn default_artifact_refuses_the_apply_ops_switch() {
    // ⚠ 本体只在 `--no-default-features`（不带 `in-process-mcp`）的构建里编译，
    // 与 `default_artifact_refuses_the_in_process_mcp_switch` 同一条执行限制（见回执）。
    let run = invoke(&["--apply-ops", "ops.json"]);
    expect_usage(
        &run,
        "--apply-ops 需要本次构建带 `--features in-process-mcp`",
    );
}

/// 判据 B7f: **未编译该 feature 的产物层面**拒绝 `--export-stems`。
///
/// 与 B7d 同一条纪律：渲染与落盘（`stems::export` / `stems::write`）全住在
/// `yeban-mcp`，未编译 `in-process-mcp` 的构建里（`--no-default-features`）那条依赖边不存在 ⇒ 必须是以用法错误（退出码 2）被拒、并
/// **点名**缺的 feature，绝不静默地"以为导了其实没导"。
///
/// 为什么只在未编译该 feature 的构建里跑：带 feature 时它是合法的无窗口开关，正面判据是同包的
/// `tests/export_stems.rs`（每条轨各得一个有效 32-bit Float WAV）。
#[cfg(not(feature = "in-process-mcp"))]
#[test]
fn default_artifact_refuses_the_export_stems_switch() {
    // ⚠ 本体只在 `--no-default-features`（不带 `in-process-mcp`）的构建里编译，
    // 与 `default_artifact_refuses_the_in_process_mcp_switch` 同一条执行限制（见回执）。
    let run = invoke(&["--export-stems", "stems"]);
    expect_usage(
        &run,
        "--export-stems 需要本次构建带 `--features in-process-mcp`",
    );
}

/// 判据 B8: `--export-elements` 与 `--dump-elements` 同源（同一批行、同一个字节数）。
#[test]
fn export_elements_matches_dump_elements() {
    let dir = scratch_dir("export-elements");
    let source = write_real_container(&dir, "src.yeban");
    let target = dir.join("elements.txt");
    let saved = dir.join("out.yeban");
    let run = invoke(&[
        "--open",
        &source.display().to_string(),
        "--dump-elements",
        "--export-elements",
        &target.display().to_string(),
        "--save-as",
        &saved.display().to_string(),
    ]);
    assert_eq!(run.code, 0, "导出 + 保存必须是退出码 0: {run:?}");

    // 落盘的内容 = 打到 stdout 的那一串元素行（同源，不是第二份生成器）。
    let dumped: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.starts_with("element "))
        .collect();
    assert!(!dumped.is_empty(), "必须有元素行:\n{}", run.stdout);
    let file = std::fs::read_to_string(&target).expect("读导出文件");
    assert_eq!(
        file.lines().count(),
        dumped.len(),
        "行数必须一致（落盘 {} 行，stdout {} 行）",
        file.lines().count(),
        dumped.len()
    );
    for line in &dumped {
        assert!(file.contains(line), "导出文件必须含 `{line}`");
    }
    let exported = line_with(&run.stdout, "exported:").expect("必须有 exported: 行");
    assert_eq!(
        field(exported, "lines").as_deref(),
        Some(dumped.len().to_string().as_str()),
        "导出的行数必须是实际落盘行数: {exported}"
    );
    assert_eq!(
        field(exported, "bytes").as_deref(),
        Some(file.len().to_string().as_str()),
        "导出的字节数必须是实际落盘字节数: {exported}"
    );
    assert!(saved.exists(), "两个都给时保存也要发生");
    let saved_bytes = std::fs::read(&saved).expect("读保存的工程");

    // 导出失败 ⇒ **不**写工程（顺序是契约：先导出，再保存）。
    let blocked = dir.join("blocked").join("elements.txt");
    let failed = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-elements",
        &blocked.display().to_string(),
        "--save-as",
        &saved.display().to_string(),
    ]);
    assert_eq!(failed.code, 5, "导出失败是退出码 5: {failed:?}");
    assert!(
        failed.stdout.is_empty(),
        "失败时 stdout 必须为空: {failed:?}"
    );
    assert!(
        failed.stderr.contains("导出元素清单"),
        "必须说清是导出元素清单失败: {failed:?}"
    );
    assert!(!blocked.exists(), "失败不得留下目标文件");
    assert!(
        !dir.join("blocked").exists(),
        "失败不得凭空造出目录（更不许留半个文件）"
    );
    assert_eq!(
        std::fs::read(&saved).expect("旧文件仍在"),
        saved_bytes,
        "被拒绝的保存绝不能碰旧文件"
    );
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(yeban_app::save::TEMP_INFIX))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B9: 只读目录下 `--save-as` ⇒ 退出码 4，且**原文件一字未改**（原子替换的可观测后果）。
#[cfg(unix)]
#[test]
fn save_as_into_a_read_only_directory_exits_four_and_keeps_the_old_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = scratch_dir("readonly");
    let source = write_real_container(&dir, "source.yeban");
    let target = dir.join("target.yeban");
    std::fs::copy(&source, &target).expect("先放一个真容器当旧文件");
    let before = std::fs::read(&target).expect("读原文");

    let mut permissions = std::fs::metadata(&dir).expect("目录元数据").permissions();
    permissions.set_mode(0o555);
    std::fs::set_permissions(&dir, permissions).expect("降权");

    let probe = dir.join(".probe");
    let writable = std::fs::write(&probe, b"x").is_ok();
    let _ = std::fs::remove_file(&probe);

    if writable {
        eprintln!("[tests/cli_contract] 只读目录仍可写 (特权进程?), 本条判据无从判定 —— 响亮跳过");
    } else {
        let run = invoke(&[
            "--open",
            source.to_str().expect("utf8"),
            "--save-as",
            target.to_str().expect("utf8"),
        ]);
        assert_eq!(run.code, 4, "只读目录必须退出 4; stderr={}", run.stderr);
        assert!(run.stderr.contains("保存到"), "{}", run.stderr);
        assert_eq!(
            std::fs::read(&target).expect("旧文件必须还在"),
            before,
            "失败的保存绝不能破坏旧文件 (这就是原子替换的意义)"
        );
    }

    let mut permissions = std::fs::metadata(&dir).expect("目录元数据").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&dir, permissions).expect("还原权限");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B14（`ROAD-M4-008` 选项 (a) 第三片）：`.yeban.lock` 被**别的持有者**持有时，
/// `--save-as` 必须**拒绝写入**（退出 4），而不是绕过锁覆盖工程文件
/// （`[ARCH-SEC-001]` / `[MUST-GATE-008]`）。
///
/// 持锁用的是 [`yeban_app::project_lock`] —— 与保存路径**同一份源码**
/// （`#[path]` 共享 `crates/yeban-mcp/src/domain/lock.rs`），因此本判据证的正是
/// "app 的保存路径与控制面争**同一把**建议锁"这件事。
///
/// 三步各自可失败：
/// 1. 持锁期间 `--save-as` ⇒ 退出 4 且 stderr 点名"拒绝写入"；
/// 2. 目标文件**逐字节未变**（第二次保存写的是**另一份**工程：样本工程而不是
///    `--open` 进来的那个容器 ⇒ "写没写"在字节上看得见，不是同一份内容的自证）；
/// 3. 释放锁之后**同一条命令必须成功**，且排他持有者把自己的锁文件收走。
#[test]
fn save_as_is_refused_while_the_project_lock_is_held() {
    use yeban_app::project_lock::lock::{LockMode, acquire, lock_path};
    let dir = scratch_dir("locked-save");
    let source = write_real_container(&dir, "src.yeban");
    let target = dir.join("locked.yeban");
    let first = invoke(&[
        "--open",
        &source.display().to_string(),
        "--save-as",
        &target.display().to_string(),
    ]);
    assert_eq!(first.code, 0, "先放一个真容器: {first:?}");
    let before = std::fs::read(&target).expect("读原文");

    // 与保存路径**同一个实现、同一把锁**。
    let guard = acquire(&target, LockMode::ExclusiveWrite).expect("取排他写建议锁");
    let refused = invoke(&[
        "--open",
        &source.display().to_string(),
        "--save-as",
        &target.display().to_string(),
    ]);
    assert_eq!(refused.code, 4, "持锁时保存必须是退出码 4: {refused:?}");
    assert!(
        refused.stdout.is_empty(),
        "失败时 stdout 必须为空: {refused:?}"
    );
    assert!(
        refused.stderr.contains(".yeban.lock"),
        "拒绝必须点名被占用的锁文件: {refused:?}"
    );
    assert!(
        refused.stderr.contains(&target.display().to_string()),
        "拒绝必须点名企图写入的工程: {refused:?}"
    );
    assert_eq!(
        std::fs::read(&target).expect("旧文件仍在"),
        before,
        "被拒绝的保存绝不能碰旧文件"
    );
    drop(guard);

    // 释放之后同一份保存必须成功 ⇒ 拒绝的原因只能是那把锁。
    let after = invoke(&[
        "--open",
        &source.display().to_string(),
        "--save-as",
        &target.display().to_string(),
    ]);
    assert_eq!(after.code, 0, "锁释放后必须能保存: {after:?}");
    assert!(
        !lock_path(&target).exists(),
        "成功的排他保存必须把自己的锁文件收走（不留残留锁）: {}",
        lock_path(&target).display()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B10（**反转**）: 裸 `project.json` 必须被**明确拒绝** —— 容器是唯一工程格式
/// （ADR-0001 D43）。旧判据在这里断言"能打开 + `format=project-json`"。
#[test]
fn open_rejects_a_bare_project_json_with_a_precise_reason() {
    use yeban_model::container::{ContainerLimits, read_container};
    let dir = scratch_dir("bare-json");
    let source = write_real_container(&dir, "src.yeban");
    let bytes = std::fs::read(&source).expect("读容器");
    let json = read_container(&bytes, &ContainerLimits::default())
        .expect("真容器")
        .get("project.json")
        .expect("必有 project.json")
        .data
        .clone();
    let bare = dir.join("project.json");
    std::fs::write(&bare, &json).expect("写裸 JSON");

    // 旧判据在这里断言"能打开 + format=project-json"。反转之后它必须**失败**。
    let run = invoke(&["--open", &bare.display().to_string(), "--headless"]);
    assert_eq!(run.code, 3, "裸 project.json 必须是退出码 3: {run:?}");
    assert!(run.stdout.is_empty(), "失败时 stdout 必须为空: {run:?}");
    assert!(run.stderr.contains("不是 `.yeban` 容器"), "实测: {run:?}");
    assert!(
        run.stderr.contains("end-of-central-directory"),
        "必须带上容器的精确原裁决: {run:?}"
    );

    // 用法文本里不再有裸 JSON 读法。
    let usage = yeban_app::cli::usage_text();
    assert!(
        !usage.contains("裸") && !usage.contains("project-json"),
        "用法不得再提裸 JSON 读法"
    );

    // 内容本身没问题：同一份 JSON 在真容器里就能打开（拒绝的是容器边界）。
    let wrapped = dir.join("wrapped.yeban");
    let ok = invoke(&[
        "--open",
        &source.display().to_string(),
        "--save-as",
        &wrapped.display().to_string(),
    ]);
    assert_eq!(ok.code, 0, "容器必须能打开并另存: {ok:?}");
    assert!(wrapped.exists(), "另存的文件必须真的在");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B11: `--print-shortcuts` 是无窗口路径，且退出 0（CI 与人核对策略表的入口）。
#[test]
fn print_shortcuts_is_a_batch_command() {
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "--print-shortcuts 必须是退出码 0: {run:?}");
    assert!(
        run.stderr.is_empty(),
        "--print-shortcuts 不该往 stderr 写东西: {run:?}"
    );
    let expected = yeban_app::cli::shortcut_lines();
    let stdout_lines: Vec<&str> = run.stdout.lines().collect();
    assert!(
        stdout_lines.len() > expected.len(),
        "表后面必须还有批处理报告（只印表、没有批处理尾 = 形态错了）"
    );
    // 表在前、批处理报告在后：stdout 的前 N 行必须逐字等于 shortcut_lines()。
    for (index, line) in expected.iter().enumerate() {
        assert_eq!(
            stdout_lines[index], line,
            "第 {index} 行必须与 shortcut_lines() 逐字一致"
        );
    }
    // 批处理尾：握手行恰好一次，表里至少有一条未实现标记。
    assert_eq!(
        run.stdout
            .lines()
            .filter(|line| *line == "headless ok")
            .count(),
        1,
        "无头握手行必须恰好一次:\n{}",
        run.stdout
    );
    assert!(run.stdout.contains("(未实现)"), "表里必须标出未实现的键");
    assert!(run.stdout.contains("界面直达键"), "表后必须有界面直达键段");
}

/// 判据 B11b: 快捷键表**说能用**的键, 解析 + 宿主必须真的接受; 表里标 `(未实现)` 的键,
/// 宿主必须真的拒绝。
///
/// 这一条针对一类具体的谎：表里写着某个键能用, 而 `host::apply_action` 对它
/// 返回 `false`（`reject`）—— 用户按了没反应, 表却已经许下了承诺。判据把三个观测面
/// **逐条对账**（单位是**条目**, 不是行）：
///
/// 1. **渲染结果**：真二进制 `--print-shortcuts` 的 stdout 里, 每一行的"规范写法"一列
///    要么是 `label`（表声称能用），要么是 `label (未实现)`（表声称不能用）；
/// 2. **解析**：`InputContext::resolve(key, modifiers)` 必须解析出这一行声称的
///    `Action`（标签与它绑的键不能各说各话），且 stdout 那一行的"画布聚焦"列与它一致；
/// 3. **宿主**：`host::action_has_implementation(action)` 必须等于这一行的 `implemented`
///    —— 宿主说没有实现的动作, 表就必须标 `(未实现)`; 反之亦然。
///
/// 实例会变、判据不变：`Z → 选区撑满视口` 与 `Cmd/Ctrl+D → 复制到下一格` 都曾是这条判据的
/// 样板谎（表说能用、宿主 `reject`），它们现在都有落地实现（见 B11d / B11e）；
/// 今天的样例是 `[ → 试听主线`（`Shift+Enter → 采纳 AI 建议` 曾是这一条的样板谎，
/// 2026-10-31 起它有了落地实现 ⇒ 从名单里移出，见判据 B11f 的文档）；
///
/// 能无头查的就是这三面。`apply_action` 对 `Undo`/`Redo`/`OpenTimeMachine` 在**没有撤销
/// 端口**的装配下也返回 `false` —— 那是装配缺件, 不是动作没实现, 所以这条判据判的是
/// "有没有落地实现", 而不是"当前这个装配有没有接上它"。真实窗口上的消费 / 拒绝由
/// `tests/live_ui_mcp.rs` 的判据 16 用真事件源直接见证。
#[test]
fn shortcut_table_status_matches_the_resolution_and_host_pipeline() {
    use yeban_app::cli::{UNIMPLEMENTED_MARKER, shortcut_rows};
    use yeban_app::host::action_has_implementation;
    use yeban_app::input::{Focus, InputContext};
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "--print-shortcuts 必须是退出码 0: {run:?}");
    let rows = shortcut_rows();
    let stdout_lines: Vec<&str> = run.stdout.lines().collect();
    assert!(
        stdout_lines.len() >= 4 + rows.len(),
        "stdout 必须完整印出 {} 行快捷键表（实际 {} 行）",
        rows.len(),
        stdout_lines.len()
    );
    assert_eq!(
        stdout_lines[0], "# 快捷键策略表 (yeban-app scaffold)",
        "表头第一行必须逐字一致"
    );

    let mut canvas = InputContext::new();
    canvas.set_focus(Focus::MainCanvas);
    let mut composing = InputContext::new();
    composing.set_focus(Focus::TextInput);
    composing.begin_composition();
    let mut unimplemented = 0usize;
    for (index, row) in rows.iter().enumerate() {
        let rendered = stdout_lines[4 + index];
        // 平台折叠：非 macOS 上 `Cmd/Ctrl+` 印成 `Ctrl+`（与 shortcut_lines 同款）。
        let label = row.label.replace(
            "Cmd/Ctrl+",
            if cfg!(target_os = "macos") {
                "Cmd+"
            } else {
                "Ctrl+"
            },
        );
        assert!(
            rendered.contains(&label),
            "第 {index} 行必须印出规范写法 `{label}`: {rendered}"
        );
        // 解析：画布聚焦列与合成态列必须等于 InputContext::resolve 的 Debug。
        assert!(
            rendered.contains(&format!("{:?}", canvas.resolve(row.key, row.modifiers))),
            "第 {index} 行的画布列必须等于 resolve 的结果: {rendered}"
        );
        assert!(
            rendered.contains(&format!("{:?}", composing.resolve(row.key, row.modifiers))),
            "第 {index} 行的合成态列必须等于 resolve 的结果: {rendered}"
        );
        // 合成态下除 F5/F6 外一切都被输入法吞掉。
        if !matches!(
            composing.resolve(row.key, row.modifiers),
            yeban_app::input::Resolution::ConsumedByIme
        ) {
            assert!(
                label.contains("F5") || label.contains("F6"),
                "合成态下非 F5/F6 的键必须被输入法吞掉: {rendered}"
            );
        }
        // 落地状态：表的主张必须等于宿主自己的陈述；未实现的行必须带标记。
        assert_eq!(
            row.implemented,
            action_has_implementation(row.action),
            "第 {index} 行（{}）的主张与宿主陈述分叉",
            row.label
        );
        if row.implemented {
            assert!(
                !rendered.contains(UNIMPLEMENTED_MARKER),
                "已实现的行不该带未实现标记: {rendered}"
            );
        } else {
            assert!(
                rendered.contains(UNIMPLEMENTED_MARKER),
                "未实现的行必须带标记: {rendered}"
            );
            unimplemented += 1;
        }
    }
    assert_eq!(unimplemented, 1, "未实现条目必须恰好是 AuditionMain 一条");
}

/// 判据 B11c: `Delete/Backspace → 删除` 已落地 —— 宿主认它, 快捷表也说能用, 渲染不带标记。
///
/// 这一条是"删除回退成不实现"的**直接牙**：把 `Action::DeleteSelection` 放回
/// `host::action_has_implementation` 的 `!matches!` 名单（或把 `cli.rs` 的
/// `implemented` 改回 `false`）⇒ 这里立刻红。B11b 也覆盖这一条, 但它的失败信息是
/// "三面之一分叉"; 本判据的失败信息直接点名"删除被回退了"。
#[test]
fn delete_selection_is_implemented_in_the_host_table_and_rendered_output() {
    use yeban_app::cli::shortcut_rows;
    use yeban_app::host::action_has_implementation;
    use yeban_app::input::{Focus, InputContext};
    let rows = shortcut_rows();
    let row = rows
        .iter()
        .find(|row| row.label.contains("Delete/Backspace → 删除"))
        .expect("表里必须有删除行");
    assert!(row.implemented, "删除键必须已实现");
    assert!(
        action_has_implementation(row.action),
        "宿主必须消费删除动作"
    );
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "--print-shortcuts 必须是退出码 0: {run:?}");
    let rendered = run
        .stdout
        .lines()
        .find(|line| line.contains("Delete/Backspace → 删除"))
        .expect("渲染输出里必须有删除行");
    assert!(
        rendered.contains("Action(DeleteSelection)"),
        "渲染行必须解析到 DeleteSelection: {rendered}"
    );
    assert!(
        !rendered.contains(yeban_app::cli::UNIMPLEMENTED_MARKER),
        "删除行不该带未实现标记: {rendered}"
    );
    // 解析口径：画布聚焦下这一键解析到 DeleteSelection。
    let mut canvas = InputContext::new();
    canvas.set_focus(Focus::MainCanvas);
    assert_eq!(
        format!("{:?}", canvas.resolve(row.key, row.modifiers)),
        "Action(DeleteSelection)",
        "画布聚焦下删除键必须解析到 DeleteSelection"
    );
}

/// 判据 B11d: `Z → 选区撑满视口` 与 `Shift+Z → 全曲总览` 已落地 —— 宿主认它们,
/// 快捷表也说能用, 渲染不带 `(未实现)`。
///
/// 这一条是"缩放回退成不实现"的**直接牙**：把 `Action::ZoomToSelection` /
/// `Action::ZoomToFit` 放回 `host::action_has_implementation` 的 `!matches!` 名单
/// （或把 `cli.rs` 的 `implemented` 改回 `false`）⇒ 这里立刻红。B11b 也覆盖这两条,
/// 但它的失败信息是"三面之一分叉"; 本判据的失败信息直接点名"缩放被回退了"。
///
/// 运行时语义（空选区 / 空工程时**不消费**）由 `tests/live_ui_mcp.rs` 的判据 Z1
/// 用真事件源见证 —— 本判据只回答"这个动作有没有落点"。
#[test]
fn zoom_shortcuts_are_implemented_in_the_host_table_and_rendered_output() {
    use yeban_app::cli::shortcut_rows;
    use yeban_app::host::action_has_implementation;
    use yeban_app::input::Action;
    let rows = shortcut_rows();
    let zoom: Vec<_> = rows
        .iter()
        .filter(|row| {
            matches!(
                row.action,
                Action::ZoomIn
                    | Action::ZoomOut
                    | Action::ZoomToSelection
                    | Action::ZoomToFit
                    | Action::VerticalZoomIn
                    | Action::VerticalZoomOut
            )
        })
        .collect();
    assert_eq!(zoom.len(), 6, "缩放族必须恰好 6 条（水平/垂直/选区/总览）");
    for row in &zoom {
        assert!(row.implemented, "{} 必须已实现", row.label);
        assert!(
            action_has_implementation(row.action),
            "宿主必须消费 {}",
            row.label
        );
    }
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "--print-shortcuts 必须是退出码 0: {run:?}");
    for row in &zoom {
        let label = row.label.replace(
            "Cmd/Ctrl+",
            if cfg!(target_os = "macos") {
                "Cmd+"
            } else {
                "Ctrl+"
            },
        );
        let rendered = run
            .stdout
            .lines()
            .find(|line| line.contains(&label))
            .unwrap_or_else(|| panic!("渲染输出里必须有 `{label}` 行"));
        assert!(
            rendered.contains(&format!("Action({:?})", row.action)),
            "渲染行必须解析到 {:?}: {rendered}",
            row.action
        );
        assert!(
            !rendered.contains(yeban_app::cli::UNIMPLEMENTED_MARKER),
            "缩放行不该带未实现标记: {rendered}"
        );
    }
}

/// 判据 B11e: `Cmd/Ctrl+D → 复制到下一格` 已落地 —— 宿主认它, 快捷表也说能用, 渲染不带 `(未实现)`。
///
/// 这一条是"复制回退成不实现"的**直接牙**：把 `Action::Duplicate` 放回
/// `host::action_has_implementation` 的 `!matches!` 名单（或把 `cli.rs` 的 `implemented`
/// 改回 `false`）⇒ 这里立刻红。B11b 也覆盖这一条, 但它的失败信息是"三面之一分叉";
/// 本判据的失败信息直接点名"复制被回退了"。
///
/// 运行时的四条语义（音符数 `N → N+k`、副本与原文可区分、一次 `Cmd+Z` 全部回来、
/// 空选区**不消费**）由 `tests/live_ui_mcp.rs` 的端到端判据用真事件源见证 ——
/// 本判据只回答"这个动作有没有落点"。
#[test]
fn duplicate_shortcut_is_implemented_in_the_host_table_and_rendered_output() {
    use yeban_app::cli::shortcut_rows;
    use yeban_app::host::action_has_implementation;
    use yeban_app::input::{Focus, InputContext};
    let rows = shortcut_rows();
    let row = rows
        .iter()
        .find(|row| row.label.contains("复制到下一格"))
        .expect("表里必须有复制行");
    assert!(row.implemented, "复制键必须已实现");
    assert!(
        action_has_implementation(row.action),
        "宿主必须消费复制动作"
    );
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "--print-shortcuts 必须是退出码 0: {run:?}");
    let rendered = run
        .stdout
        .lines()
        .find(|line| line.contains("复制到下一格"))
        .expect("渲染输出里必须有复制行");
    assert!(
        rendered.contains("Action(Duplicate)"),
        "渲染行必须解析到 Duplicate: {rendered}"
    );
    assert!(
        !rendered.contains(yeban_app::cli::UNIMPLEMENTED_MARKER),
        "复制行不该带未实现标记: {rendered}"
    );
    let mut canvas = InputContext::new();
    canvas.set_focus(Focus::MainCanvas);
    assert_eq!(
        format!("{:?}", canvas.resolve(row.key, row.modifiers)),
        "Action(Duplicate)",
        "画布聚焦下复制键必须解析到 Duplicate"
    );
}

/// 判据 B11f: **能力缺失**的快捷键仍然如实标记为 `(未实现)` —— 本判据是"它还不可用"的牙。
///
/// ## 为什么要有这一条（它不是 B11b 的重复）
///
/// B11b（`shortcut_table_status_matches_the_resolution_and_host_pipeline`）判的是**三面一致**
/// ——"表 / 解析 / 宿主有没有各说各话"。它**允许**这三面一起改成"已实现"。而本判据钉住的是
/// **事实本身**：`[` 与 `]`（试听主线 / 试听 AI 提案分支）的能力**今天不存在**
/// —— 缺的不是接线，是规范 `[ARCH-RT-005]` 的 A/B 并发渲染。
///
/// 因此它会在两种情况下变红，两种都该红：
/// 1. **假实现**：把 `implemented` 改成 `true`（或把动作从 `action_has_implementation`
///    移出）而能力仍未落地 ⇒ 第 ①③④ 步全部失败；
/// 2. **真实现但忘了改判据**：谁真的把能力接上了（例如引擎把 `ARCH-RT-005` 落进
///    `IMPLEMENTED_SPEC_IDS`）⇒ 第 ⑤ 步失败，逼作者回来更新这一条、`cli.rs` 的
///    `implemented` 与 `host::action_has_implementation` 的名单。
///
/// ⚠ **本判据于 2026-10-31 少了一条**：`Shift+Enter → 采纳 AI 建议` 从"能力缺失"名单里
/// **移出**了（抽屉长出提案身份 `ProposalRowView::id` + 选中态窗口属性 + `HostAction` 的
/// `MergeProposal` / `RejectProposal` 两臂 ⇒ 落点真的存在）。那不是"断言变松"：
/// 它由 `tests/live_ui_mcp.rs` 的 `selecting_a_row_lights_up_accept_and_it_really_merges_the_proposal` 的运行时判据**反向**接管 ——
/// 那边要求"点选一行 ⇒ 按钮亮起 ⇒ `Shift+Enter` 与「采纳」都让权威里那条提案真的消失、提交数 +1"。
/// 一删一补，⛔ 不留"没人管"的那一格。
///
/// ## 各步的"单位"（防止把行数当条数）
///
/// - 第 ③ 步数的是 `--print-shortcuts` 的**条目行**里 `(未实现)` 的出现**次数**（单位：条目标记），
///   表头那一行 `# (未实现) = …` 是**说明文字**，不计入（口径与 B11b 第 ① 步同款）。
/// - 第 ④ 步数的是 `shortcut_rows()` 里 `implemented == false` 的**条目数**（单位：条目）。
///
/// 运行时的"按下不消费且一位不变"由 `tests/live_ui_mcp.rs` 的
/// `the_ai_suggestion_and_audition_keys_are_rejected_without_any_state_change` 用真事件源见证
/// （那里 `Shift+Enter` 仍然不消费 —— 那一台装配没有提案来源，本判据说的是"有没有落点"）。
#[test]
fn the_ai_suggestion_and_audition_keys_are_still_declared_unimplemented() {
    use yeban_app::cli::{UNIMPLEMENTED_MARKER, shortcut_rows};
    use yeban_app::host::action_has_implementation;
    let rows = shortcut_rows();
    let unimplemented: Vec<_> = rows.iter().filter(|row| !row.implemented).collect();
    assert_eq!(unimplemented.len(), 1, "未实现条目必须恰好一条");
    assert_eq!(
        unimplemented[0].label, "[ → 试听主线",
        "唯一未实现的必须是试听主线（`[`）"
    );
    assert!(
        !action_has_implementation(unimplemented[0].action),
        "宿主也必须把试听主线列为未实现"
    );
    // 渲染：恰好一行带 (未实现) 标记（表头那句说明不含 Action(…)，不计）。
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "--print-shortcuts 必须是退出码 0: {run:?}");
    let marked: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.contains(UNIMPLEMENTED_MARKER) && line.contains("Action("))
        .collect();
    assert_eq!(marked.len(), 1, "带标记的渲染行必须恰好一条: {marked:?}");
    assert!(
        marked[0].contains("[ → 试听主线"),
        "带标记的行必须是试听主线: {}",
        marked[0]
    );
    // 反向：采纳 AI 建议已实现（Shift+Enter 不再是未实现）。
    let accept = rows
        .iter()
        .find(|row| row.label.contains("采纳 AI 建议"))
        .expect("表里必须有采纳 AI 建议行");
    assert!(accept.implemented, "Shift+Enter 采纳 AI 建议已实现");
    assert!(
        action_has_implementation(accept.action),
        "宿主必须消费采纳 AI 建议"
    );
}

/// 判据 B12: `--export-midi` 从**真二进制**导出的字节可被 SMF 读取面读回，
/// 逐音符与工程一致，`MThd` 的 PPQ 字段是 960，且两次导出逐字节相同
/// （① 回读 / ② 逐音符 / ③ PPQ 头 / ④ 确定性，全部在**进程级**再证一次）。
#[test]
fn export_midi_writes_a_parseable_deterministic_smf_from_the_real_binary() {
    use yeban_render::midi::{MidiFormat, parse_smf, track_chunks};
    let dir = scratch_dir("export-midi");
    let source = write_real_container(&dir, "song.yeban");
    let target = dir.join("song.mid");
    let run = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-midi",
        &target.display().to_string(),
    ]);
    assert_eq!(run.code, 0, "--export-midi 必须是退出码 0: {run:?}");
    let exported = line_with(&run.stdout, "exported-midi:").expect("必须有 exported-midi: 行");

    // 报告行**自描述**：落点 / 字节数 / PPQ / 格式 / 轨道 / 音符 / 来源。
    assert!(exported.contains("song.mid"), "{exported}");
    assert_eq!(field(exported, "ppq").as_deref(), Some("960"), "{exported}");
    assert_eq!(
        field(exported, "format").as_deref(),
        Some("parallel"),
        "{exported}"
    );
    assert_eq!(
        field(exported, "tracks").as_deref(),
        Some("1"),
        "{exported}"
    );
    assert_eq!(field(exported, "notes").as_deref(), Some("6"), "{exported}");
    assert_eq!(
        field(exported, "tempos").as_deref(),
        Some("1"),
        "{exported}"
    );
    assert_eq!(
        field(exported, "channelsReused").as_deref(),
        Some("0"),
        "exported-midi: 必须披露复用通道数: {exported}"
    );
    assert_eq!(
        field(exported, "maxTracksPerChannel").as_deref(),
        Some("1"),
        "exported-midi: 必须披露单通道最大轨数: {exported}"
    );
    assert!(
        exported.contains(yeban_app::save::TEMP_INFIX),
        "必须用临时文件（原子落盘）: {exported}"
    );
    assert!(
        exported.contains("from=") && exported.contains("song.yeban"),
        "exported-midi: 行必须说清来源: {exported}"
    );
    let bytes = std::fs::read(&target).expect("文件必须真的在");
    assert_eq!(
        field(exported, "bytes").as_deref(),
        Some(bytes.len().to_string().as_str()),
        "打印的字节数必须是实际落盘字节数: {exported}"
    );

    // 字节能被 SMF 读取面读回；chunk 布局 = MThd + conductor + 一条音符轨。
    let chunks = track_chunks(&bytes).expect("chunk 布局");
    assert_eq!(&chunks[0].fourcc, b"MThd");
    assert_eq!(chunks.len(), 3);
    for chunk in &chunks[1..] {
        let payload = &bytes[chunk.payload.clone()];
        assert_eq!(payload[payload.len() - 3..], [0xFF, 0x2F, 0x00]);
    }
    let parsed = parse_smf(&bytes).expect("回读");
    assert_eq!(parsed.format, MidiFormat::Parallel);
    assert_eq!(parsed.ppq, 960);
    assert_eq!(&bytes[12..14], &[0x03, 0xC0], "0x03C0 = 960");

    // 逐音符：（通道， 音高， 力度， 起始 tick, 时值）与工程一致。
    let mut actual: Vec<(u8, u8, u8, u64, u64)> =
        parsed.notes.iter().map(|note| note.key()).collect();
    actual.sort_unstable();
    assert_eq!(actual, demo_note_keys(), "六颗音符逐项一致");

    // 同一工程两次导出 ⇒ 逐字节相同。
    let second = dir.join("song-again.mid");
    let again = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-midi",
        &second.display().to_string(),
    ]);
    assert_eq!(again.code, 0, "第二次导出必须成功: {again:?}");
    assert_eq!(
        std::fs::read(&target).expect("读第一次"),
        std::fs::read(&second).expect("读第二次"),
        "同一工程的两次导出必须逐字节相同"
    );
    assert!(
        run.stdout.lines().any(|line| line == "headless ok"),
        "无窗口路径必须打印握手行"
    );
    // 没有 `.tmp-` 残留（原子替换的痕迹）。
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(yeban_app::save::TEMP_INFIX))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 C（回归）: `--export-wav` **单独**（不给 `--headless`，也不给
/// `SLINT_BACKEND=headless`）就必须走无窗口路径：真二进制在 [`RUN_TIMEOUT`] 内退出 0、
/// 打印握手行 `headless ok` 与 `exported-wav:`、并真的写出可读的 WAV。
///
/// 为什么它**刻意不带**无窗口开关: 本切片修复前 `--export-wav` **不在**
/// `Options::batch()` 里 ⇒ `main.rs` 把它当 GUI 调用（`main.rs` 里
/// **0** 处 `options.export_wav`，`run_gui` 结构上不可能消费这个字段）
/// ⇒ 进程进阻塞事件循环、既不打印 `exported-wav:` 也不写文件（本判据在修复前
/// 会挂在 [`RUN_TIMEOUT`] 上并被 kill）。因此本判据钉的是**路由谓词**本身，
/// 而不是导出实现（实现早有 `--headless` 下的正面判据）。
#[test]
fn export_wav_runs_on_the_windowless_path_from_the_real_binary() {
    let dir = scratch_dir("export-wav");
    let source = write_real_container(&dir, "song.yeban");
    let target = dir.join("song.wav");
    // 刻意不带 `--headless`：钉的是路由谓词自己（`--export-wav` 在 batch() 里）。
    let run = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-wav",
        &target.display().to_string(),
        "--wav-seconds",
        "8",
    ]);
    assert_eq!(run.code, 0, "--export-wav 必须是退出码 0: {run:?}");
    let line = line_with(&run.stdout, "exported-wav:").expect("必须有 exported-wav 报告行");
    assert!(line.contains("from="), "报告行必须明说来源: {line}");
    assert!(line.contains("source="), "报告行必须登记音源: {line}");
    assert_eq!(
        field(line, "frames").as_deref(),
        Some("384000"),
        "帧数必须与请求的 8 秒一致: {line}"
    );

    // 回读：帧数 / 采样率 / 声道与请求一致（24-bit 整数 PCM），且非静音。
    let (frames, sample_rate, channels, bits, is_float) = read_wav_shape(&target);
    assert_eq!(sample_rate, 48_000, "采样率");
    assert_eq!(channels, 2, "声道");
    assert_eq!(bits, 24, "位深");
    assert!(!is_float, "整数 PCM");
    assert_eq!(frames, 48_000 * 8, "帧数与请求一致");

    // 同一工程两次导出 ⇒ 逐字节相同（确定性）。
    let again = dir.join("song-again.wav");
    let second = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-wav",
        &again.display().to_string(),
        "--wav-seconds",
        "8",
    ]);
    assert_eq!(second.code, 0, "第二次导出必须成功: {second:?}");
    assert_eq!(
        std::fs::read(&target).expect("读第一次"),
        std::fs::read(&again).expect("读第二次"),
        "同一工程的两次导出必须逐字节相同"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B16: `--export-wav` 的**位深 / 采样率 / 时长 / 精确帧数**可被 CLI 选，
/// 且**缺省兼容**。
///
/// 本条钉四件事（都**回读真文件**，⛔ 不读 stdout 的自报值）：
/// ① 显式给 `--wav-seconds 8` ⇒ 与旧行为逐字节一致（48 kHz / 立体声 / 24-bit / 8 秒）——
///    演示工程的 `audio_config.sample_rate` 就是 48 kHz，所以"跟随工程"在该档**不改变产物**；
///    T-FIX-360 起"不给时长"改为按工程末端推导，旧的 8 秒口径一律显式给；
/// ② 显式给三条 ⇒ 产物形状**逐条**跟着变（16-bit / 44.1 kHz / 1 秒）；
///    ②b `--wav-frames` 原样生效（与 MCP `render_master.frames` 同量纲）；
///    ②c `--wav-frames` 与 `--wav-seconds` 互斥（两种顺序都是用法错误）；
/// ③ `--wav-*` 单独给（没有 `--export-wav`）⇒ 用法错误退出码 **2**，
///    ⛔ 不是"静默忽略"（本仓对修饰开关的一贯纪律）。
#[test]
fn export_wav_bit_depth_sample_rate_and_seconds_are_selectable_and_default_stays_compatible() {
    let dir = scratch_dir("export-wav-shape");
    let source = write_real_container(&dir, "song.yeban");

    // 16-bit / 44100 / 1 秒：报告行与回读的形状都必须跟着走。
    let s16 = dir.join("s16.wav");
    let run = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-wav",
        &s16.display().to_string(),
        "--wav-bit-depth",
        "16",
        "--wav-sample-rate",
        "44100",
        "--wav-seconds",
        "1",
    ]);
    assert_eq!(run.code, 0, "16-bit/44100 导出必须是退出码 0: {run:?}");
    let line = line_with(&run.stdout, "exported-wav:").expect("必须有 exported-wav 行");
    assert_eq!(
        field(line, "sample_rate").as_deref(),
        Some("44100"),
        "{line}"
    );
    assert_eq!(field(line, "bits").as_deref(), Some("16"), "{line}");
    let (frames, sample_rate, channels, bits, is_float) = read_wav_shape(&s16);
    assert_eq!(
        (frames, sample_rate, channels, bits),
        (44_100, 44_100, 2, 16),
        "回读的形状必须与请求一致"
    );
    assert!(!is_float, "16-bit 必须是整数 PCM");

    // --wav-frames 48000 ⇒ 1 秒 @48k（帧数原样使用）。
    let f = dir.join("f.wav");
    let run2 = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-wav",
        &f.display().to_string(),
        "--wav-frames",
        "48000",
    ]);
    assert_eq!(run2.code, 0, "--wav-frames 导出必须是退出码 0: {run2:?}");
    let (frames2, _, _, _, _) = read_wav_shape(&f);
    assert_eq!(frames2, 48_000, "显式帧数必须原样使用");

    // 默认：不给时长/位深/采样率 ⇒ 48k/24-bit，时长取工程可闻末端（不再截成 8 秒）。
    let d = dir.join("d.wav");
    let run3 = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-wav",
        &d.display().to_string(),
    ]);
    assert_eq!(run3.code, 0, "默认导出必须是退出码 0: {run3:?}");
    let line3 = line_with(&run3.stdout, "exported-wav:").expect("必须有 exported-wav 行");
    assert_eq!(
        field(line3, "sample_rate").as_deref(),
        Some("48000"),
        "默认采样率必须是 48000: {line3}"
    );
    assert_eq!(
        field(line3, "bits").as_deref(),
        Some("24"),
        "默认位深必须是 24: {line3}"
    );
    assert_eq!(
        field(line3, "secondsSource").as_deref(),
        Some("projectEnd"),
        "默认时长必须取工程可闻末端: {line3}"
    );
    let expected_end =
        yeban_app::export_wav::project_audible_end_seconds(&yeban_app::bridge::demo_project())
            .expect("演示工程必须有可闻末端");
    let reported_end: f64 = field(line3, "projectEndSeconds")
        .expect("projectEndSeconds")
        .parse()
        .expect("projectEndSeconds 必须是数字");
    assert!(
        (reported_end - expected_end).abs() < 0.002,
        "回执的末端秒数必须与工程一致: {line3}"
    );
    let (frames3, _, _, _, _) = read_wav_shape(&d);
    assert_eq!(
        frames3,
        (expected_end * 48_000.0).round() as u64,
        "默认导出的帧数必须是末端 × 采样率"
    );

    // 互斥：--wav-frames + --wav-seconds 同给 ⇒ 用法错误（不许静默丢一个）。
    let clash = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-wav",
        &dir.join("x.wav").display().to_string(),
        "--wav-frames",
        "100",
        "--wav-seconds",
        "1",
    ]);
    expect_usage(&clash, "`--wav-frames` 与 `--wav-seconds` 互斥");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据（审评 2026-10-31，P1-1）：`--render-threads` 只改耗时、不改字节 ——
/// 同一工程缺省导出与 `--render-threads 4` 导出的文件逐字节相同
/// （固定顺序汇聚；非法值已由上一判据第 ④ 节覆盖）。
#[test]
fn export_wav_render_threads_change_only_time_not_bytes() {
    let dir = scratch_dir("export-wav-threads");
    let source = write_real_container(&dir, "song.yeban");
    let t4 = dir.join("t4.wav");
    let run4 = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-wav",
        &t4.display().to_string(),
        "--render-threads",
        "4",
        "--wav-seconds",
        "1",
    ]);
    assert_eq!(run4.code, 0, "--render-threads 4 必须是退出码 0: {run4:?}");
    let t0 = dir.join("t0.wav");
    let run0 = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-wav",
        &t0.display().to_string(),
        "--wav-seconds",
        "1",
    ]);
    assert_eq!(run0.code, 0, "默认线程数必须是退出码 0: {run0:?}");
    assert_eq!(
        std::fs::read(&t4).expect("读 4 线程产物"),
        std::fs::read(&t0).expect("读默认产物"),
        "--render-threads 只改时间、不改字节"
    );
    // 报告行的 digest 也必须相同（同一份产物）。
    let digest4 = field(
        line_with(&run4.stdout, "exported-wav:").expect("t4 报告行"),
        "digest",
    )
    .expect("digest");
    let digest0 = field(
        line_with(&run0.stdout, "exported-wav:").expect("t0 报告行"),
        "digest",
    )
    .expect("digest");
    assert_eq!(digest4, digest0, "digest 必须相同");
    // 修饰开关单独给 ⇒ 退出码 2 + 精确原因（run_batch 守卫，不附用法文本）。
    let alone = invoke(&["--headless", "--render-threads", "2"]);
    expect_usage_reason(&alone, "`--render-threads` 必须与 `--export-wav` 一起给");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据：`--loudness-target` / `--true-peak-ceiling` 的用法护栏与真二进制落点。
///
/// - 两开关同属 `--export-wav` 的**修饰**开关：单独给 = 用法错误（退出码 2），与
///   `--wav-*` / `--render-threads` 同款（上一判据第 ③ 节的口径）；
/// - 值域护栏：目标超出 `[-70, 0]`、上限为正、非数字、`NaN` 都必须被拒（⛔ 不静默钳制）；
/// - 重复给同一开关 = 用法错误；
/// - 真二进制：演示工程 `--loudness-target -16` ⇒ 退出码 0 且 stdout 多一行 `loudness:`。
#[test]
fn export_wav_loudness_switches_are_guarded_and_reach_the_target() {
    let dir = scratch_dir("export-wav-loudness");
    let source = write_real_container(&dir, "song.yeban");
    let target = dir.join("loud.wav");
    let src = source.display().to_string();
    let wav = dir.join("x.wav").display().to_string();
    let run = invoke(&[
        "--open",
        &src,
        "--export-wav",
        &target.display().to_string(),
        "--loudness-target",
        "-16",
        "--wav-seconds",
        "1",
    ]);
    assert_eq!(run.code, 0, "--loudness-target -16 必须是退出码 0: {run:?}");
    let loudness = line_with(&run.stdout, "loudness:").expect("必须有 loudness: 行");
    let achieved: f64 = field(loudness, "achieved")
        .as_deref()
        .expect("achieved")
        .parse()
        .expect("achieved 必须是数字");
    assert!(
        (achieved - (-16.0)).abs() <= 0.5,
        "实测响度必须落在目标 ±0.5 LU 内: {loudness}"
    );
    assert_eq!(
        field(loudness, "bound").as_deref(),
        Some("LoudnessTarget"),
        "目标赢 ⇒ bound 必须是 LoudnessTarget: {loudness}"
    );
    let peak: f64 = field(loudness, "truePeak")
        .as_deref()
        .expect("truePeak")
        .parse()
        .expect("truePeak 必须是数字");
    assert!(peak <= 0.0, "真峰值不得超 0 dBTP: {loudness}");

    // 真峰值上限收紧时仍达标，且峰值受上限约束。
    let capped = dir.join("capped.wav");
    let run2 = invoke(&[
        "--open",
        &src,
        "--export-wav",
        &capped.display().to_string(),
        "--loudness-target",
        "-16",
        "--true-peak-ceiling",
        "-1.0",
        "--wav-seconds",
        "1",
    ]);
    assert_eq!(run2.code, 0, "带真峰值上限必须是退出码 0: {run2:?}");
    let loudness2 = line_with(&run2.stdout, "loudness:").expect("必须有 loudness: 行");
    let achieved2: f64 = field(loudness2, "achieved")
        .as_deref()
        .expect("achieved")
        .parse()
        .expect("achieved 必须是数字");
    assert!(
        (achieved2 - (-16.0)).abs() <= 0.5,
        "收紧上限后实测响度仍须落在目标 ±0.5 LU 内: {loudness2}"
    );
    let peak2: f64 = field(loudness2, "truePeak")
        .as_deref()
        .expect("truePeak")
        .parse()
        .expect("truePeak 必须是数字");
    assert!(
        peak2 <= -1.0,
        "真峰值必须受 -1.0 dBTP 上限约束: {loudness2}"
    );

    // 守卫：越界 / 非数字 / 重复给 ⇒ 用法错误（附用法文本）；单独给 ⇒ 退出码 2 + 精确原因
    // （run_batch 守卫只印原因行、不附用法文本 —— 见 expect_usage_reason）。
    for (args, needle, with_usage) in [
        (
            vec![
                "--open",
                &src,
                "--export-wav",
                &wav,
                "--loudness-target",
                "-71",
                "--wav-seconds",
                "1",
            ],
            "`--loudness-target` 的取值 `-71` 非法",
            true,
        ),
        (
            vec![
                "--open",
                &src,
                "--export-wav",
                &wav,
                "--loudness-target",
                "1",
                "--wav-seconds",
                "1",
            ],
            "`--loudness-target` 的取值 `1` 非法",
            true,
        ),
        (
            vec![
                "--open",
                &src,
                "--export-wav",
                &wav,
                "--loudness-target",
                "abc",
                "--wav-seconds",
                "1",
            ],
            "`--loudness-target` 的取值 `abc` 非法",
            true,
        ),
        (
            vec![
                "--open",
                &src,
                "--export-wav",
                &wav,
                "--loudness-target",
                "NaN",
                "--wav-seconds",
                "1",
            ],
            "`--loudness-target` 的取值 `NaN` 非法",
            true,
        ),
        (
            vec![
                "--open",
                &src,
                "--export-wav",
                &wav,
                "--true-peak-ceiling",
                "0.5",
                "--wav-seconds",
                "1",
            ],
            "`--true-peak-ceiling` 的取值 `0.5` 非法",
            true,
        ),
        (
            vec![
                "--open",
                &src,
                "--export-wav",
                &wav,
                "--loudness-target",
                "-16",
                "--loudness-target",
                "-17",
                "--wav-seconds",
                "1",
            ],
            "`--loudness-target` 只能给一次",
            true,
        ),
        (
            vec!["--headless", "--loudness-target", "-16"],
            "`--loudness-target` 必须与 `--export-wav` 一起给",
            false,
        ),
        (
            vec!["--headless", "--true-peak-ceiling", "-0.5"],
            "`--true-peak-ceiling` 必须与 `--export-wav` 一起给",
            false,
        ),
    ] {
        let run = invoke(&args);
        if with_usage {
            expect_usage(&run, needle);
        } else {
            expect_usage_reason(&run, needle);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B13: `--export-midi` 的失败语义与既有 CLI 一致 —— 非容器输入 ⇒ 退出码 **3**
/// 且一个字节都不写；目标不可写 ⇒ 退出码 **5**、精确原因、**不留下半个文件**。
#[test]
fn export_midi_failures_reuse_the_existing_exit_codes_and_write_nothing() {
    let dir = scratch_dir("export-midi-failures");
    let junk = dir.join("junk.bin");
    std::fs::write(&junk, b"not a zip at all").expect("写垃圾");
    let target = dir.join("never.mid");
    let run = invoke(&[
        "--open",
        &junk.display().to_string(),
        "--export-midi",
        &target.display().to_string(),
    ]);
    assert_eq!(run.code, 3, "非容器输入复用 --open 的退出码 3: {run:?}");
    assert!(run.stdout.is_empty(), "失败时 stdout 必须为空: {run:?}");
    assert!(
        run.stderr.contains("不是 `.yeban` 容器"),
        "必须转达容器的精确裁决: {run:?}"
    );
    assert!(!target.exists(), "打开失败 ⇒ 一个字节都不许写出去");

    // 目标路径不可写 ⇒ 退出码 5 + 精确原因 + **不留下半个文件**。
    let source = write_real_container(&dir, "src.yeban");
    let blocked = dir.join("missing").join("out.mid");
    let run2 = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-midi",
        &blocked.display().to_string(),
    ]);
    assert_eq!(run2.code, 5, "写不出去是退出码 5: {run2:?}");
    assert!(run2.stdout.is_empty(), "失败时 stdout 必须为空: {run2:?}");
    assert!(
        run2.stderr.contains("导出 MIDI 到") && run2.stderr.contains("写临时文件"),
        "必须说清是导出 MIDI 失败以及精确的 I/O 动作: {run2:?}"
    );
    assert!(!blocked.exists(), "失败不得留下目标文件");
    assert!(
        !dir.join("missing").exists(),
        "失败不得凭空造出目录（更不许留半个文件）"
    );
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(yeban_app::save::TEMP_INFIX))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B14: 带 `experimental-als-export` 时，`--export-als` 写出 Gzip 文档并把
/// **映射损失表**逐条打到 stdout（`ADR-0001 D47` 的价值 = 损失表对用户可见）。
///
/// 与 B12 同一条思路，但断言的是另一件事：B12 证"字节能被 SMF 读取面读回"，
/// 这里证"**没被映射的东西真的被说出来了**" —— 只打印一个计数不算交付。
#[cfg(feature = "experimental-als-export")]
#[test]
fn export_als_writes_a_gzip_document_and_prints_the_loss_summary() {
    use yeban_render::als::{build_live_set_xml, export_project};
    let dir = scratch_dir("export-als");
    let source = write_real_container(&dir, "song.yeban");
    let target = dir.join("song.als");
    let run = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-als",
        &target.display().to_string(),
    ]);
    assert_eq!(run.code, 0, "--export-als 必须是退出码 0: {run:?}");
    let line = line_with(&run.stdout, "exported-als:").expect("必须有 exported-als: 行");
    assert!(line.contains("song.als"), "{line}");
    assert_eq!(field(line, "tracks").as_deref(), Some("7"), "{line}");
    assert_eq!(field(line, "clips").as_deref(), Some("3"), "{line}");
    assert_eq!(field(line, "notes").as_deref(), Some("6"), "{line}");
    assert_eq!(field(line, "losses").as_deref(), Some("18"), "{line}");
    assert!(
        line.contains(yeban_app::save::TEMP_INFIX),
        "必须用临时文件（原子落盘）: {line}"
    );
    assert!(
        line.contains("from=") && line.contains("song.yeban"),
        "报告行必须说清来源: {line}"
    );

    // 落盘字节 = 同一工程的库内导出逐字节相等（不是"差不多"）。
    let bytes = std::fs::read(&target).expect("文件必须真的在");
    assert_eq!(
        field(line, "bytes").as_deref(),
        Some(bytes.len().to_string().as_str()),
        "打印的字节数必须是实际落盘字节数: {line}"
    );
    let expected = export_project(&yeban_app::bridge::demo_project()).expect("库内导出");
    assert_eq!(
        bytes, expected.bytes,
        "二进制写出的 .als 必须逐字节等于同一工程的库内导出"
    );
    assert_eq!(expected.mapped_tracks, 7, "库内导出的轨道数");
    assert_eq!(expected.mapped_clips, 3, "库内导出的片段数");
    assert_eq!(expected.mapped_notes, 6, "库内导出的音符数");
    assert_eq!(expected.losses.len(), 18, "库内导出的损失条数");

    // gzip 形态：魔数 + deflate 方法 + 尾部 ISIZE（未压缩长度）。
    assert_eq!(&bytes[..2], &[0x1F, 0x8B], "必须是 gzip 魔数开头");
    assert_eq!(bytes[2], 8, "压缩方法必须是 deflate");
    let isize = u32::from_le_bytes(
        bytes[bytes.len() - 4..]
            .try_into()
            .expect("gzip 尾部 ISIZE"),
    );
    // 未压缩长度不钉死数字：探针的 12706 来自二进制另存的容器（created_at 是 13 位
    // 墙钟），本夹具的 created_at 是 0，短 12 字节 —— 长度的事实源是同一工程的库内 XML。
    let document = build_live_set_xml(&yeban_app::bridge::demo_project());
    assert_eq!(
        isize as usize,
        document.xml.len(),
        "gzip 尾部声明的未压缩长度必须等于同一工程的库内 XML 长度"
    );

    // 真解压回读（unix）：确实是一份能解开的 XML 文档。
    #[cfg(unix)]
    {
        let out = std::process::Command::new("gzip")
            .arg("-dc")
            .arg(&target)
            .output()
            .expect("unix 测试宿主必须有 gzip");
        assert!(
            out.status.success(),
            "gzip -dc 必须成功: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            out.stdout.len(),
            document.xml.len(),
            "解出的 XML 长度必须等于同一工程的库内 XML 长度"
        );
        assert!(out.stdout.starts_with(b"<?xml"), "解出的内容必须是 XML");
        let xml = String::from_utf8(out.stdout).expect("XML 必须是 UTF-8");
        assert!(xml.contains("<Ableton"), "XML 必须是 LiveSet 文档");
        assert!(xml.contains("yeban-loss"), "XML 必须带损失注释");
    }

    // 损失表：18 条全打印（不截断、不静默），每条都有可读原因。
    let losses = line_with(&run.stdout, "als-losses:").expect("必须有 als-losses: 行");
    assert_eq!(field(losses, "count").as_deref(), Some("18"), "{losses}");
    let printed: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.starts_with("als-loss:"))
        .collect();
    assert_eq!(printed.len(), 18, "18 条损失必须全打印");
    assert!(
        printed.iter().all(|line| line.contains("reason=")),
        "每条损失必须有可读原因"
    );
    assert!(
        !run.stdout.contains("als-losses-truncated"),
        "18 条不该被截断"
    );

    // 同一工程两次导出 ⇒ 逐字节相同。
    let second = dir.join("song-again.als");
    let again = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-als",
        &second.display().to_string(),
    ]);
    assert_eq!(again.code, 0, "第二次导出必须成功: {again:?}");
    assert_eq!(
        std::fs::read(&target).expect("读第一次"),
        std::fs::read(&second).expect("读第二次"),
        "同一工程的两次导出必须逐字节相同"
    );

    // 不给 --open ⇒ 导出演示工程，来源行如实写 sample=default。
    let noopen = dir.join("sample.als");
    let run2 = invoke(&["--export-als", &noopen.display().to_string()]);
    assert_eq!(run2.code, 0, "不给 --open 也能导出演示工程: {run2:?}");
    let line2 = line_with(&run2.stdout, "exported-als:").expect("必须有 exported-als: 行");
    assert!(
        line2.contains("from=sample=default"),
        "不给 --open ⇒ from=sample=default: {line2}"
    );

    // 目标路径不可写 ⇒ 退出码 5 + 精确原因 + 不留下半个文件。
    let blocked = dir.join("missing").join("out.als");
    let run3 = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-als",
        &blocked.display().to_string(),
    ]);
    assert_eq!(run3.code, 5, "写不出去是退出码 5: {run3:?}");
    assert!(run3.stdout.is_empty(), "失败时 stdout 必须为空: {run3:?}");
    assert!(
        run3.stderr.contains("导出 .als 到") && run3.stderr.contains("写临时文件"),
        "必须说清是导出 ALS 失败以及精确的 I/O 动作: {run3:?}"
    );
    assert!(!blocked.exists(), "失败不得留下目标文件");
    assert!(
        !dir.join("missing").exists(),
        "失败不得凭空造出目录（更不许留半个文件）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B15: 带 `experimental-logic-export` 时，`--export-logic` 写出 `.logicx`
/// **bundle 目录**（四个文件）并把映射损失表逐条打到 stdout。
///
/// 与 B14 同一条思路，但产物是多文件目录：因此它另外钉住 ① 目录与四个文件真的在、
/// ② `ProjectData` 的**供体**头部（`23 47 C0 AB` / 版本码 `0x09d0` / 声明长度 @0x10 /
/// `gnoS` @0x18）、③ `MetaData.plist` 是二进制 plist（`bplist00`）、④ 两次导出**逐文件**相同、
/// ⑤ 目标被普通文件挡住时退出 5。
///
/// 供体路线（负责人裁决选项 A）把 `ProjectData` 从自研的 1,420 字节换成
/// `jonkubis/logicproformatwriter`（MIT）那份 **Logic 存过的**夹具骨架（127,689 字节）+ 我们自己的
/// 拍号/速度/region 名/音符，因此这里也钉住"它确实带着供体"这一条。
#[cfg(feature = "experimental-logic-export")]
#[test]
fn export_logic_writes_a_bundle_directory_and_prints_the_loss_summary() {
    let dir = scratch_dir("export-logic");
    let source = write_real_container(&dir, "song.yeban");
    let target = dir.join("out.logicx");
    let run = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-logic",
        &target.display().to_string(),
    ]);
    assert_eq!(run.code, 0, "--export-logic 必须是退出码 0: {run:?}");
    let line = line_with(&run.stdout, "exported-logic:").expect("必须有 exported-logic: 行");
    assert!(line.contains("out.logicx"), "{line}");
    assert_eq!(field(line, "files").as_deref(), Some("4"), "{line}");
    assert_eq!(field(line, "bytes").as_deref(), Some("294270"), "{line}");
    assert_eq!(field(line, "regions").as_deref(), Some("1"), "{line}");
    assert_eq!(field(line, "notes").as_deref(), Some("6"), "{line}");
    assert_eq!(field(line, "losses").as_deref(), Some("46"), "{line}");
    assert!(
        line.contains("from=") && line.contains("song.yeban"),
        "报告行必须说清来源: {line}"
    );

    // bundle 里恰好是这 4 个文件（不多不少）。
    let mut found = Vec::new();
    let mut stack = vec![target.clone()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).expect("列 bundle 目录") {
            let path = entry.expect("目录项").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(
                    path.strip_prefix(&target)
                        .expect("相对路径")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    found.sort();
    assert_eq!(
        found,
        [
            "Alternatives/000/DisplayState.plist",
            "Alternatives/000/MetaData.plist",
            "Alternatives/000/ProjectData",
            "Resources/ProjectInformation.plist",
        ],
        "bundle 必须恰好是这 4 个文件"
    );

    // ProjectData 的供体头部：魔数 / 版本码 0x09d0 / 声明长度 / Song 标记。
    let project_data =
        std::fs::read(target.join("Alternatives/000/ProjectData")).expect("读 ProjectData");
    assert_eq!(
        project_data.len(),
        266_151,
        "ProjectData 必须是 266151 字节"
    );
    assert_eq!(
        &project_data[..4],
        &[0x23, 0x47, 0xC0, 0xAB],
        "供体头部魔数"
    );
    assert_eq!(
        u16::from_le_bytes(project_data[4..6].try_into().expect("版本码")),
        0x09d0,
        "根头版本码必须是 0x09d0（旧判据写的 0x09CF 是错的，已同步修正）"
    );
    // 根头 0x18 字节（logic_read.rs 的格式表）；@0x10 声明的是根头**之后**的记录区长度。
    assert_eq!(
        u32::from_le_bytes(project_data[0x10..0x14].try_into().expect("声明长度")) as usize,
        project_data.len() - 0x18,
        "声明长度 @0x10 必须等于文件长度减去 24 字节根头"
    );
    assert_eq!(
        &project_data[0x18..0x1c],
        b"gnoS",
        "Song 记录标记必须在 0x18"
    );

    // MetaData.plist 是二进制 plist（完整损失表在它的 YebanMappingLosses 键里）。
    let meta = std::fs::read(target.join("Alternatives/000/MetaData.plist")).expect("读 MetaData");
    assert!(
        meta.starts_with(b"bplist00"),
        "MetaData.plist 必须是二进制 plist"
    );

    // 损失表：46 条计数，印 20 条后截断，截断行明说还剩 26 条。
    let losses = line_with(&run.stdout, "logic-losses:").expect("必须有 logic-losses: 行");
    assert_eq!(field(losses, "count").as_deref(), Some("46"), "{losses}");
    let printed: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.starts_with("logic-loss:"))
        .collect();
    assert_eq!(printed.len(), 20, "必须恰好印 20 条损失行");
    let truncated = line_with(&run.stdout, "logic-losses-truncated:").expect("必须有截断行");
    assert!(
        truncated.contains("and 26 more"),
        "截断行必须明说还剩 26 条: {truncated}"
    );

    // 同一工程两次导出 ⇒ 每个文件逐字节相同。
    let second = dir.join("out2.logicx");
    let again = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-logic",
        &second.display().to_string(),
    ]);
    assert_eq!(again.code, 0, "第二次导出必须成功: {again:?}");
    for rel in &found {
        assert_eq!(
            std::fs::read(target.join(rel)).expect("读第一次"),
            std::fs::read(second.join(rel)).expect("读第二次"),
            "{rel} 两次导出必须逐字节相同"
        );
    }

    // 不给 --open ⇒ 导出演示工程，来源行如实写 sample=default。
    let noopen = dir.join("sample.logicx");
    let run2 = invoke(&["--export-logic", &noopen.display().to_string()]);
    assert_eq!(run2.code, 0, "不给 --open 也能导出演示工程: {run2:?}");
    let line2 = line_with(&run2.stdout, "exported-logic:").expect("必须有 exported-logic: 行");
    assert!(
        line2.contains("from=sample=default"),
        "不给 --open ⇒ from=sample=default: {line2}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B15b（`#200`）: `--export-logic` 的目标目录**必须**以 `.logicx` 结尾。
///
/// 为什么这条判据存在：macOS 的 `open -a "Logic Pro" hello` **报错** —— LaunchServices
/// 靠**扩展名**认 bundle。而"导出成功"离"打不开"只有一步之遥：`--export-logic hello` 曾经
/// 写出一个叫 `hello` 的目录、回执 `path=hello files=4` 看起来完全正常。因此本判据钉三件事：
///
/// ① 缺后缀 ⇒ 落在 `<给的目录>.logicx`（且**不在**无后缀的那个名字上留任何东西）；
/// ② 回执的 `path=` 打印的是**实际**路径（以 `.logicx` 结尾）—— 用户照它敲 `open -a` 能认；
/// ③ 已带后缀 ⇒ 路径**不变**（不重复加：`foo.logicx` ⛔ 不变 `foo.logicx.logicx`）。
///
/// ⚠ 本条**不**调用 `open -a`：真开 Logic 是人的事（本文件只跑无窗口参数）。
#[cfg(feature = "experimental-logic-export")]
#[test]
fn export_logic_forces_the_logicx_suffix_and_prints_the_actual_path() {
    let dir = scratch_dir("export-logic-suffix");
    let source = write_real_container(&dir, "song.yeban");

    // 不带后缀 ⇒ 补 .logicx，且不留裸 `hello`。
    let bare = dir.join("hello");
    let run = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-logic",
        &bare.display().to_string(),
    ]);
    assert_eq!(run.code, 0, "不带后缀的导出必须是退出码 0: {run:?}");
    let line = line_with(&run.stdout, "exported-logic:").expect("必须有 exported-logic: 行");
    assert!(
        line.contains("hello.logicx"),
        "报告行必须印**实际**落点（补后缀后的路径）: {line}"
    );
    assert!(
        dir.join("hello.logicx").is_dir(),
        "bundle 必须落在 hello.logicx"
    );
    assert!(!bare.exists(), "不许留下裸 `hello`");

    // 已带后缀 ⇒ 不再叠加。
    let suffixed = dir.join("foo.logicx");
    let run2 = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-logic",
        &suffixed.display().to_string(),
    ]);
    assert_eq!(run2.code, 0, "已带后缀的导出必须是退出码 0: {run2:?}");
    assert!(suffixed.is_dir(), "foo.logicx 必须保持 foo.logicx");
    assert!(
        !dir.join("foo.logicx.logicx").exists(),
        "不许叠加第二个 .logicx"
    );

    // 挡路：一个普通文件占了 bundle 该在的位置 ⇒ 退出码 5 + 精确原因 + 原文件无损。
    let blocked = dir.join("blocked.logicx");
    std::fs::write(&blocked, b"in the way").expect("写挡路文件");
    let run3 = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-logic",
        &blocked.display().to_string(),
    ]);
    assert_eq!(run3.code, 5, "bundle 目录建不出来是退出码 5: {run3:?}");
    assert!(run3.stdout.is_empty(), "失败时 stdout 必须为空: {run3:?}");
    assert!(
        run3.stderr.contains("创建 bundle 目录"),
        "必须说清是创建 bundle 目录失败: {run3:?}"
    );
    assert!(
        run3.stderr.contains("Not a directory (os error 20)"),
        "必须带上精确的系统原因: {run3:?}"
    );
    assert_eq!(
        std::fs::read(&blocked).expect("挡路文件仍在"),
        b"in the way",
        "失败的导出绝不能碰挡路文件"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B10（T-FIX-565；T-FIX-384 的跟进）: `--open` + `--import-audio` + `--save-as`
/// 在**真二进制**上走通：导入的音频资产落盘后可读回。
///
/// 为什么这条必须住在真二进制这一层：`cli.rs` 里 T-FIX-384 的单元判据
/// （`import_audio_indexes_the_asset_so_the_container_passes_the_cross_layer_check`）
/// 跑的是进程内 `run_batch(&parse(...))` —— 它证明逻辑对，但证明不了 argv 进来、真
/// 进程出去、真退出码这一条产品路径。本判据与那条同一组断言、另一条路径：真 argv、
/// 真 stdout、`imported-audio:` 报告行、读回落盘容器（池字节逐字节相同、索引自洽、
/// 片段指向同一 SHA-256、取回的字节可解码、跨层审计无半登记）。
///
/// 红对照（T-FIX-565 交付时在盒上双向验证）：删掉 `cli.rs` 里
/// `register_audio_asset` 那一行调用 ⇒ 保存被跨层核对以 `MissingAsset` 拒掉、
/// 进程非零退出，本判据红；恢复 ⇒ 绿。
#[test]
fn import_audio_save_as_round_trips_asset_through_the_real_binary() {
    let dir = scratch_dir("import-audio-e2e");
    let input = write_real_container(&dir, "in.yeban");

    // 真 WAV：单声道 48 kHz 16-bit、64 帧（与 T-FIX-384 单元判据同一份写法、同一只写入器）。
    let wav = dir.join("take.wav");
    let samples: Vec<i16> = (0..64_i16).map(|index| index * 137 - 4_000).collect();
    yeban_render::wav::write_plain_wav(
        &wav,
        &yeban_render::rf64::PcmFormat::integer(1, 48_000, 16),
        &yeban_render::dither::PcmBuffer::Int16(samples),
    )
    .expect("写夹具 WAV");
    let wav_bytes = std::fs::read(&wav).expect("读夹具 WAV");
    let expected = yeban_model::ids::AssetHash::of_bytes(&wav_bytes);

    let out = dir.join("out.yeban");
    let run = invoke(&[
        "--open",
        input.to_str().expect("输入路径必须是 UTF-8"),
        "--import-audio",
        wav.to_str().expect("WAV 路径必须是 UTF-8"),
        "--save-as",
        out.to_str().expect("输出路径必须是 UTF-8"),
    ]);
    assert_eq!(
        run.code, 0,
        "导入 + 另存必须成功（退出 0），stderr: {}",
        run.stderr
    );
    let line =
        line_with(&run.stdout, "imported-audio:").expect("stdout 必须有 imported-audio 报告行");
    assert_eq!(
        field(line, "asset").as_deref(),
        Some(expected.as_str()),
        "报告行的 asset 必须是源字节的 SHA-256: {line}"
    );
    assert_eq!(
        field(line, "frames").as_deref(),
        Some("64"),
        "解码事实：帧数: {line}"
    );
    assert_eq!(
        field(line, "rate").as_deref(),
        Some("48000"),
        "解码事实：采样率: {line}"
    );
    assert_eq!(
        field(line, "channels").as_deref(),
        Some("1"),
        "解码事实：声道数: {line}"
    );
    assert_eq!(
        field(line, "origin").as_deref(),
        Some("import"),
        "来源标记必须是 `OpOrigin::Import`: {line}"
    );

    // 公开入口回读 `--save-as` 写出的容器。
    let reopened = yeban_app::open::open_project_archive_file(
        &out,
        &yeban_app::open::ProjectOpenOptions::default(),
    )
    .expect("回读另存的容器");
    let entry = reopened
        .project
        .clip_pool
        .values()
        .find(|entry| match &entry.content {
            yeban_model::ClipContent::Audio { asset, .. } => asset.as_str() == expected.as_str(),
            _ => false,
        })
        .expect("保存的工程里必须多一条这份字节的音频片段");
    match entry.content.clone() {
        yeban_model::ClipContent::Audio { asset, gain_db } => {
            assert_eq!(asset, expected, "片段必须指向源字节的 SHA-256");
            assert_eq!(gain_db, 0.0, "本路径的片段增益是单位增益 0.0 dB");
        }
        other => panic!("必须是音频片段: {other:?}"),
    }
    let stored = reopened
        .assets
        .iter()
        .find(|(hash, _)| *hash == expected)
        .map(|(_, bytes)| bytes.clone())
        .expect("容器的资产池必须有该 sha256 的字节");
    assert_eq!(stored, wav_bytes, "容器里的字节必须与源文件逐字节相同");

    let metadata = reopened.project.assets.get(&expected).unwrap_or_else(|| {
        panic!(
            "工程资产索引必须有该 sha256 的条目，实测索引键={:?}",
            reopened.project.assets.keys().collect::<Vec<_>>()
        )
    });
    assert_eq!(metadata.hash, expected, "索引项的 hash 必须自指");
    assert_eq!(
        metadata.byte_len,
        u64::try_from(wav_bytes.len()).expect("字节数放得进 u64"),
        "索引项的 byte_len 必须等于源字节长度"
    );
    assert_eq!(
        metadata.media_kind,
        yeban_model::project::MediaKind::Audio,
        "导入的 WAV 是音频资产"
    );
    assert_eq!(
        metadata.license, "user-imported",
        "许可口径与 import_logic 的导入口径同款"
    );
    assert_eq!(
        metadata.original_path, "take.wav",
        "original_path 只留文件名（⛔ 不把导入机的临时目录钉进工程）"
    );

    // 跨层审计：两层必须同时写 —— 缺一边就是 T-FIX-384 修的那种半登记。
    let audit = yeban_app::open::audit_project_assets(&reopened);
    assert!(
        audit.missing.is_empty(),
        "引用不得悬空: {:?}",
        audit.missing
    );
    assert!(
        audit.unindexed_pool.is_empty(),
        "池里不许有未索引的条目: {:?}",
        audit.unindexed_pool
    );
    assert!(
        audit.index_without_pool.is_empty(),
        "索引不许指向池里没有的字节: {:?}",
        audit.index_without_pool
    );

    // 解码证据：从容器取回的字节再走同一个唯一读取器 ⇒ 帧数／采样率／声道不变。
    let extracted = dir.join("extracted.wav");
    std::fs::write(&extracted, &stored).expect("写出取回的资产");
    let (back, buffer) =
        yeban_render::wav::read_plain_wav(&extracted).expect("取回的资产必须可解码");
    assert_eq!(back.sample_rate, 48_000);
    assert_eq!(back.channels, 1);
    assert_eq!(buffer.len(), 64, "帧数 = 64（单声道）");
}

/// 判据 B15c（`#200`）: 3 轨产物的**损失表文本**必须同时写明「已验的音色」「音色来自**供体**」
/// 「⛔ **不是用户工程的设备链**」。
///
/// 为什么这条判据住在**真二进制**这一层：音色验收是**人类**给的读数（负责人 2026-10-18：
/// 三条轨分别是 `Studio Grand` / `Studio Grand` / `Epic Cloud Formation`），而"这三个音色其实
/// 是**供体**的三个通道条"是本产物**最容易被误读**的一条 —— 用户看不到这条就会以为
/// "我挂的混响导出后还在"。因此它必须是**用户能读到的 stdout 文本**，而不是只活在代码注释里。
///
/// 四个片段是**跨模块契约**（`yeban_render::logic` 的 3 轨打开范围常量 ⇄ 本判据）。
#[cfg(feature = "experimental-logic-export")]
#[test]
fn export_logic_loss_table_names_the_donor_timbres_and_the_device_chain_gap() {
    use yeban_render::logic::LOGIC_OWNER_DONOR_3T_CHUNK_FAMILIES;
    let dir = scratch_dir("export-logic-losses");
    let source = write_real_container(&dir, "song.yeban");
    let target = dir.join("out.logicx");
    let run = invoke(&[
        "--open",
        &source.display().to_string(),
        "--export-logic",
        &target.display().to_string(),
    ]);
    assert_eq!(run.code, 0, "--export-logic 必须是退出码 0: {run:?}");

    // 供体音色：第一条 loss 行必须点名三条轨各自的音色。
    let scope = run
        .stdout
        .lines()
        .find(|line| line.contains("Studio Grand"))
        .expect("损失表必须点名供体音色");
    assert_eq!(
        scope.matches("Studio Grand").count(),
        2,
        "恰好两条轨是 Studio Grand"
    );
    assert!(
        scope.contains("Epic Cloud Formation"),
        "第 3 轨必须是 Epic Cloud Formation"
    );
    // 设备链缺口：同一行必须明说用户工程的设备链一个都不会出现在 .logicx 里。
    assert!(
        scope.contains("用户工程里挂的 EQ / 混响 / 采样器一个都不会出现在 `.logicx` 里"),
        "必须明说设备链缺口（不是含糊的部分丢失）"
    );

    // chunk 家族：登记表条数 = ProjectData 里的实际落盘条数（每个家族都验）；
    // 打印行则受 20 行上限约束：3 条范围说明 + 17 个家族 = 20 行，第 18 个家族
    // （GenM）是第 21 条损失，落在打印上限之外 —— 它在完整表里（MetaData.plist），
    // 不在 stdout，截断行须明说（B15 已断 "and 26 more"）。
    let project_data =
        std::fs::read(target.join("Alternatives/000/ProjectData")).expect("读 ProjectData");
    for (index, (tag, expected)) in LOGIC_OWNER_DONOR_3T_CHUNK_FAMILIES.iter().enumerate() {
        let landed: Vec<u8> = tag.as_bytes().iter().rev().copied().collect();
        let landed_str = String::from_utf8(landed.clone()).expect("落盘 tag 是 ASCII");
        let actual = project_data
            .windows(4)
            .filter(|window| *window == landed.as_slice())
            .count();
        assert_eq!(
            actual, *expected,
            "chunk 家族 {tag}（落盘字节 {landed_str}）：ProjectData 里实际 {actual} 条，登记表说 {expected} 条"
        );
        let sentence = format!(
            "chunk 家族 `{tag}`（落盘字节 `{landed_str}`）由供体**原样携带** {expected} 条"
        );
        if index < 17 {
            assert!(
                run.stdout.contains(&sentence),
                "损失表必须逐条印出 chunk 家族条数: {sentence}"
            );
        } else {
            assert_eq!(*tag, "GenM", "第 18 个家族必须是 GenM");
            assert!(
                !run.stdout.contains(&sentence),
                "GenM 是第 21 条损失，须在 20 行打印上限之外"
            );
            assert!(
                run.stdout.contains("logic-losses-truncated:"),
                "被截断的家族必须有截断行作证"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
