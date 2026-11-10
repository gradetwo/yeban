//! `--export-lyrics` 的端到端判据：**真二进制** + **真容器**。
//!
//! ## 这条判据要证的句子
//!
//! > `yeban-app --open 歌.yeban --export-lyrics 歌.lrc` 在没有显示器的机器上
//! > 跑完就退出，落盘的 LRC 与参照 LRC 同一口径，失败响亮、无半成品。
//!
//! ## 口径
//!
//! 与 `crates/yeban-model/tests/lyrics.rs` 同一份夹具（132 BPM、5 行词、
//! 每行隔 8 拍），断言同一份逐字节 LRC —— 两处判据钉的是**同一个**格式口径，
//! 一处改口径、另一处就红。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use yeban_app::cli::{EXIT_EXPORT, EXIT_USAGE};
use yeban_model::container::write_project_container;
use yeban_model::lyrics::LyricLine;
use yeban_model::samples::default_project;

/// 被测二进制的绝对路径（由 Cargo 注入）。
const BIN: &str = env!("CARGO_BIN_EXE_yeban-app");

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

/// 一次子进程执行允许的最长时间（与 `cli_contract.rs` 同款：无窗口开关若被错误地
/// 送进 GUI 路径，进程会进阻塞事件循环 —— 超时把它变成响亮的判据失败）。
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// 用给定参数跑一次真二进制（**只用无窗口参数**）。
fn invoke(args: &[&str]) -> Run {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
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
                    panic!(
                        "`yeban-app {args:?}` 在 {RUN_TIMEOUT:?} 内没有退出 —— \
                         它多半进了阻塞事件循环 (无窗口开关不该开窗口)"
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let output = child.wait_with_output().expect("收集输出");
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// 一次性的临时目录（**不用外部 crate**）。
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yeban-lyrics-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 参照口径的工程（与模型判据同一份夹具）写成**真容器**并落盘。
fn write_winter_container(dir: &Path, name: &str) -> PathBuf {
    use std::collections::BTreeMap;
    let mut project = default_project();
    project.title = "凛冬将至".to_owned();
    project.author = "夜半 · Yeban".to_owned();
    project.bpm = 132.0;
    project.lyrics = vec![
        LyricLine {
            start_tick: 0,
            text: "凛冬将至".to_owned(),
        },
        LyricLine {
            start_tick: 7680,
            text: "Winter Is Coming".to_owned(),
        },
        LyricLine {
            start_tick: 15360,
            text: "凛冬将至".to_owned(),
        },
        LyricLine {
            start_tick: 23040,
            text: "Winter Is Coming".to_owned(),
        },
        LyricLine {
            start_tick: 30720,
            text: "昨日 你 信誓旦旦".to_owned(),
        },
    ];
    project.validate().expect("夹具合法");
    let bytes = write_project_container(&project, b"lyrics-test-history", &BTreeMap::new())
        .expect("写出真容器");
    let path = dir.join(name);
    std::fs::write(&path, &bytes).expect("写容器");
    path
}

#[test]
fn export_lyrics_writes_the_reference_format() {
    let dir = scratch_dir("winter");
    let project = write_winter_container(&dir, "winter.yeban");
    let out = dir.join("winter.lrc");
    let run = invoke(&[
        "--open",
        project.to_str().expect("路径"),
        "--export-lyrics",
        out.to_str().expect("路径"),
    ]);
    assert_eq!(
        run.code, 0,
        "导出必须退出 0; stdout={}\nstderr={}",
        run.stdout, run.stderr
    );
    let lrc = std::fs::read_to_string(&out).expect("LRC 必须落盘");
    assert_eq!(
        lrc,
        "[ti:凛冬将至]\n[ar:夜半 · Yeban]\n[by:Yeban]\n[offset:0]\n\n\
         [00:00.00]凛冬将至\n[00:03.64]Winter Is Coming\n[00:07.27]凛冬将至\n\
         [00:10.91]Winter Is Coming\n[00:14.55]昨日 你 信誓旦旦\n"
    );
    // 回执是实测读数：落点 + 字节数 + 行数（⛔ 不从入参抄）。
    let receipt = run
        .stdout
        .lines()
        .find(|line| line.starts_with("exported-lyrics:"))
        .expect("必须有一行 `exported-lyrics:` 回执");
    assert!(receipt.contains("lines=5"), "回执必须写 lines=5: {receipt}");
    assert!(
        receipt.contains(&format!("bytes={}", lrc.len())),
        "回执字节数必须与落盘文件一致: {receipt}"
    );
    assert!(
        receipt.contains(out.to_str().expect("路径")),
        "回执必须写落点: {receipt}"
    );
}

#[test]
fn export_lyrics_without_lyrics_is_header_only() {
    let dir = scratch_dir("empty");
    let project = {
        let project = default_project();
        let bytes = write_project_container(
            &project,
            b"lyrics-test-history",
            &std::collections::BTreeMap::new(),
        )
        .expect("写出真容器");
        let path = dir.join("empty.yeban");
        std::fs::write(&path, &bytes).expect("写容器");
        path
    };
    let out = dir.join("empty.lrc");
    let run = invoke(&[
        "--open",
        project.to_str().expect("路径"),
        "--export-lyrics",
        out.to_str().expect("路径"),
    ]);
    assert_eq!(
        run.code, 0,
        "无词不是失败; stdout={}\nstderr={}",
        run.stdout, run.stderr
    );
    let lrc = std::fs::read_to_string(&out).expect("LRC 必须落盘");
    assert_eq!(lrc, "[ti:Untitled]\n[by:Yeban]\n[offset:0]\n\n");
    let receipt = run
        .stdout
        .lines()
        .find(|line| line.starts_with("exported-lyrics:"))
        .expect("必须有一行 `exported-lyrics:` 回执");
    assert!(
        receipt.contains("lines=0"),
        "无词工程的回执必须写 lines=0: {receipt}"
    );
}

#[test]
fn export_lyrics_to_an_unwritable_path_fails_loudly() {
    let dir = scratch_dir("bad-path");
    let project = write_winter_container(&dir, "winter.yeban");
    // 落点在不存在的目录里 ⇒ 原子落盘必然失败。
    let out = dir.join("no-such-dir").join("winter.lrc");
    let run = invoke(&[
        "--open",
        project.to_str().expect("路径"),
        "--export-lyrics",
        out.to_str().expect("路径"),
    ]);
    assert_eq!(
        run.code,
        i32::from(EXIT_EXPORT),
        "落盘失败必须退出 {EXIT_EXPORT}; stdout={}\nstderr={}",
        run.stdout,
        run.stderr
    );
    assert!(!out.exists(), "失败时不得留半成品: {}", out.display());
    assert!(
        run.stdout.contains("导出 LRC") || run.stderr.contains("导出 LRC"),
        "失败原因必须说出是 LRC 导出失败; stdout={}\nstderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn export_lyrics_conflicts_with_headless_idle() {
    let dir = scratch_dir("conflict");
    let out = dir.join("winter.lrc");
    let run = invoke(&[
        "--headless-idle",
        "--export-lyrics",
        out.to_str().expect("路径"),
    ]);
    assert_eq!(
        run.code,
        i32::from(EXIT_USAGE),
        "--headless-idle 与 --export-lyrics 冲突 ⇒ 用法错误 {EXIT_USAGE}; stdout={}\nstderr={}",
        run.stdout,
        run.stderr
    );
    assert!(!out.exists(), "冲突时不得写盘");
}

#[test]
fn help_lists_export_lyrics() {
    let run = invoke(&["--help"]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("--export-lyrics"),
        "用法必须列出 `--export-lyrics`"
    );
    assert!(
        run.stdout.contains("--export-lyrics <path.lrc>"),
        "用法必须写出 `--export-lyrics` 的参数形态"
    );
}
