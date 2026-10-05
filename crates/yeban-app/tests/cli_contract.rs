//! `yeban-app` 命令行面的**真二进制**端到端判据。
//!
//! ## 为什么必须是"真二进制"
//!
//! `crates/yeban-app/src/cli.rs` 的单元判据（本机也用 `rustc --edition 2024 --test` 真跑过，
//! 见 `docs/ledger/app-cli-notes.md` §4）验证的是**逻辑**；这一份验证的是**进程**：
//! 真的 argv 进来、真的 stdout/stderr 出去、真的**退出码**。两者不可互相替代 ——
//! 文档（README / 官网 Quick Start）里写的是命令行，用户拿到的是退出码。
//!
//! 二进制的绝对路径由 Cargo 注入（`CARGO_BIN_EXE_<bin name>`）：`cargo test --all-targets`
//! 会先构建 `yeban-app` 这个 bin 目标再把路径传进来，因此 **CI 的默认门禁**
//! （`.github/workflows/ci.yml` 的 `cargo test --workspace --all-targets`）就会跑到本文件 ——
//! 不需要任何人记得加一步。
//!
//! ## 只跑"无窗口"参数
//!
//! 本文件**绝不**调用不带无窗口开关的命令：那会构造真窗口并进入阻塞事件循环，把 CI 挂死。
//! 每条调用都带 `--headless` / `--dump-elements` / `--export-elements` / `--export-midi` /
//! `--save-as` / `--print-shortcuts` / `--help` / `--version` 之一。
//!
//! ## 规范来源 (Normative)
//!
//! - `[ARCH-SEC-004]`：`--save-as` 是"同目录临时文件 → fsync → rename"的原子替换；
//! - `[ARCH-SEC-003]` / `[MUST-GATE-006]` / `[MUST-GATE-007]`：容器拒绝的原因必须**原样**转达；
//! - `[ARCH-UI-003]` / `[UI-TEST-003]`：无头握手行 `headless ok`；
//! - `[UI-TEST-001]`：`--dump-elements` / `--export-elements` 的语义元素清单；
//! - `docs/adr/ADR-0001` D28（唯一注入点）/ D30（容器读法歧义一律拒绝）/
//!   **D43**（1.0.0 之前没有兼容包袱 ⇒ 裸 `project.json` 读路径已删除，容器是唯一格式）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 被测二进制的绝对路径（由 Cargo 注入，见模块文档）。
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
    let mut command = Command::new(BIN);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
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

/// 造一个**真容器**（含 `history.dag` 与一个 CAS 资产）并写盘。
///
/// 用 `yeban_model::container::write_project_container` —— 与生产写出路径**同一个**函数，
/// 因此判据吃进去的"坏输入"与生产能吐出来的"好输入"是同一批字节。
fn write_real_container(dir: &Path, name: &str) -> PathBuf {
    use std::collections::BTreeMap;
    use yeban_model::container::write_project_container;
    use yeban_model::ids::AssetHash;

    let project = yeban_app::bridge::demo_project();
    let asset = b"yeban-cli-binary-asset".to_vec();
    let hash = AssetHash::of_bytes(&asset);
    let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
    assets.insert(hash, asset);
    let bytes = write_project_container(&project, b"binary-history", &assets).expect("写出真容器");
    let path = dir.join(name);
    std::fs::write(&path, &bytes).expect("写容器");
    path
}

/// 判据 B1: `--help` 与 `--version` 走短路路径、退出码 0、内容是真的。
#[test]
fn help_and_version_are_real_and_short_circuit() {
    let help = invoke(&["--help"]);
    assert_eq!(help.code, 0, "--help 必须退出 0; stderr={}", help.stderr);
    for switch in [
        "--open",
        "--save-as",
        "--dump-elements",
        "--export-elements",
        "--export-midi",
        "--print-shortcuts",
        "--project-sample",
        "--headless",
        "--version",
    ] {
        assert!(help.stdout.contains(switch), "用法必须列出 `{switch}`");
    }
    for code in ["0", "1", "2", "3", "4", "5"] {
        assert!(help.stdout.contains(code), "用法必须写出退出码 `{code}`");
    }
    // 兼容读路径已删除（D43）：用法里不能再出现裸 JSON 读法的承诺。
    assert!(
        !help.stdout.contains("裸"),
        "用法不得再提裸 JSON 读法:\n{}",
        help.stdout
    );
    assert!(
        !help.stdout.contains("project-json"),
        "用法不得再提第二种种格式:\n{}",
        help.stdout
    );
    // 短路命令**不**打印无头握手行（它们没做任何工程工作）。
    // 注意判定口径：用法文本里**本来就提到** `headless ok` 这个字面值，
    // 因此这里断言的是"没有独立成行的那一行"，不是 contains。
    assert!(
        !help.stdout.lines().any(|line| line == "headless ok"),
        "{}",
        help.stdout
    );

    let version = invoke(&["--version"]);
    assert_eq!(version.code, 0, "stderr={}", version.stderr);
    assert_eq!(
        version.stdout.trim_end(),
        format!("yeban-app {}", env!("CARGO_PKG_VERSION")),
        "版本必须与 Cargo 注入的包版本一致"
    );
    // 再直接读根清单对一次：这条正是"把 --version 写死常量"会红掉的地方。
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let text = std::fs::read_to_string(&manifest).expect("必须能读根清单");
    assert!(
        text.contains(&format!("version = \"{}\"", env!("CARGO_PKG_VERSION"))),
        "根清单里必须出现同一个版本号"
    );
    assert!(!version.stdout.lines().any(|line| line == "headless ok"));
}

/// 判据 B2: 无头握手行**恰好**出现一次，且工程来源被明写。
#[test]
fn headless_handshake_appears_exactly_once_and_names_the_source() {
    let run = invoke(&["--headless"]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    assert_eq!(
        run.stdout.matches("headless ok").count(),
        1,
        "握手行必须恰好一次 (CI 只认这一行):\n{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("project-source: sample=default (内置演示工程; 未给 --open ⇒ 未读任何文件)"),
        "必须明写工程来自内置样本:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("未构造 MainWindow"),
        "必须声明无头路径的边界:\n{}",
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
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    assert_eq!(
        run.stdout.matches("headless ok").count(),
        1,
        "哨兵必须把进程推离 GUI 路径:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("project-source: sample=default"),
        "{}",
        run.stdout
    );
}

/// 判据 B3: `--open` 真容器 ⇒ 打印的读数与工程一致（轨道 / 音符 / 历史 / 资产）。
#[test]
fn open_reports_the_real_project_counts() {
    let dir = scratch_dir("open");
    let path = write_real_container(&dir, "song.yeban");
    let run = invoke(&["--open", path.to_str().expect("utf8"), "--headless"]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);

    let project = yeban_app::bridge::demo_project();
    let counts = line_with(&run.stdout, "project-counts:").expect("必须有 project-counts 行");
    assert_eq!(
        field(counts, "tracks-all").as_deref(),
        Some(project.tracks.len().to_string().as_str()),
        "{counts}"
    );
    assert_eq!(
        field(counts, "master-track").as_deref(),
        Some("1"),
        "{counts}"
    );
    assert_eq!(
        field(counts, "history-bytes").as_deref(),
        Some(b"binary-history".len().to_string().as_str()),
        "history.dag 的字节数必须如实报告: {counts}"
    );
    assert_eq!(
        field(counts, "asset-blobs").as_deref(),
        Some("1"),
        "{counts}"
    );

    let expected_notes = yeban_app::bridge::ViewState::from_project(&project)
        .expect("投影")
        .notes
        .len();
    assert!(expected_notes > 0, "演示夹具必须含 MIDI 音符");
    assert_eq!(
        field(counts, "midi-notes").as_deref(),
        Some(expected_notes.to_string().as_str()),
        "{counts}"
    );
    let view = line_with(&run.stdout, "view-counts:").expect("必须有 view-counts 行");
    assert_eq!(
        field(view, "notes").as_deref(),
        Some(expected_notes.to_string().as_str()),
        "{view}"
    );
    assert!(
        run.stdout.contains("format=yeban-container"),
        "{}",
        run.stdout
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B4: 坏输入 ⇒ **非零退出码 + 精确原因**，不是 panic、不是空工程。
#[test]
fn broken_inputs_exit_non_zero_without_panicking() {
    let dir = scratch_dir("broken");
    let good = write_real_container(&dir, "good.yeban");
    let bytes = std::fs::read(&good).expect("读容器");

    let truncated = dir.join("truncated.yeban");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).expect("写截断容器");
    let run = invoke(&["--open", truncated.to_str().expect("utf8"), "--headless"]);
    assert_eq!(run.code, 3, "截断容器必须退出 3; stdout={}", run.stdout);
    assert!(
        run.stderr.contains("容器被拒绝"),
        "必须原样转达容器裁决: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("truncated") || run.stderr.contains("end-of-central-directory"),
        "必须是精确的容器原因: {}",
        run.stderr
    );
    assert!(
        run.stdout.is_empty(),
        "失败不得留下半截输出: {}",
        run.stdout
    );
    assert!(
        !run.stderr.contains("panicked"),
        "不许 panic: {}",
        run.stderr
    );
    assert_ne!(
        run.code, 101,
        "101 是 Rust panic 的退出码 —— 那就是 panic 了"
    );

    // 非容器输入：空文件 / 垃圾 / 随机字节 —— 各自**明确拒绝**（退出 3，理由精确），
    // 既不是"未知格式"，也不是"打开成空工程"。
    let empty = dir.join("empty.bin");
    std::fs::write(&empty, b"").expect("写空文件");
    let junk = dir.join("junk.bin");
    std::fs::write(&junk, b"not a zip at all").expect("写垃圾");
    let random = dir.join("random.bin");
    std::fs::write(&random, [0xAB_u8; 64]).expect("写随机字节");
    for (label, path) in [("空文件", &empty), ("垃圾", &junk), ("随机字节", &random)] {
        let run = invoke(&["--open", path.to_str().expect("utf8"), "--headless"]);
        assert_eq!(run.code, 3, "{label} 必须退出 3; stderr={}", run.stderr);
        assert!(
            run.stderr.contains("不是 `.yeban` 容器"),
            "{label} 必须给出精确理由: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains("end-of-central-directory"),
            "{label} 必须带上容器原裁决: {}",
            run.stderr
        );
        assert!(
            run.stdout.is_empty(),
            "{label} 失败不得留下半截输出: {}",
            run.stdout
        );
    }

    // 不存在的文件。
    let missing = dir.join("nope.yeban");
    let run = invoke(&["--open", missing.to_str().expect("utf8"), "--headless"]);
    assert_eq!(run.code, 3, "stderr={}", run.stderr);
    assert!(run.stderr.contains("无法读取"), "{}", run.stderr);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B5: `--open` + `--save-as` ⇒ 落盘文件能被公开入口读回，且**归档等价**；
/// 两次保存的字节完全相同（确定性）。
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
    assert_eq!(
        yeban_app::open::open_project_file(&target).expect("读回"),
        yeban_app::bridge::demo_project()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B7: 未知开关 / 缺取值 ⇒ 退出码 2 + **用法提示**（不许静默忽略）。
#[test]
fn unknown_switches_exit_two_with_usage_on_stderr() {
    for args in [
        vec!["--bogus"],
        vec!["filled"],
        vec!["--open"],
        vec!["--headless", "--open"],
        vec!["--headless=1"],
    ] {
        let run = invoke(&args);
        assert_eq!(run.code, 2, "{args:?} 必须退出 2; stderr={}", run.stderr);
        assert!(
            run.stderr.contains("无法识别")
                || run.stderr.contains("需要一个取值")
                || run.stderr.contains("不接受取值"),
            "{args:?} 必须说明原因: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains("用法:") && run.stderr.contains("--save-as"),
            "{args:?} 必须带用法提示: {}",
            run.stderr
        );
        assert!(run.stdout.is_empty(), "{args:?} 不该有 stdout");
    }
}

/// 判据 B8: `--export-elements` 与 `--dump-elements` 同源（同一批行、同一个字节数）。
#[test]
fn export_elements_matches_dump_elements() {
    let dir = scratch_dir("export");
    let source = write_real_container(&dir, "src.yeban");
    let source = source.to_str().expect("utf8");
    let target = dir.join("elements.txt");

    let dumped = invoke(&["--open", source, "--dump-elements", "--headless"]);
    assert_eq!(dumped.code, 0, "stderr={}", dumped.stderr);
    let dumped_lines: Vec<&str> = dumped
        .stdout
        .lines()
        .filter(|line| line.starts_with("element "))
        .collect();
    assert!(!dumped_lines.is_empty(), "必须有元素行");

    let exported = invoke(&[
        "--open",
        source,
        "--export-elements",
        target.to_str().expect("utf8"),
        "--headless",
    ]);
    assert_eq!(exported.code, 0, "stderr={}", exported.stderr);
    let file = std::fs::read_to_string(&target).expect("读导出文件");
    assert_eq!(file.lines().count(), dumped_lines.len(), "行数必须一致");
    for line in &dumped_lines {
        assert!(file.contains(line), "导出文件必须含 `{line}`");
    }
    let report = line_with(&exported.stdout, "exported:").expect("exported:");
    assert_eq!(
        field(report, "bytes").as_deref(),
        Some(file.len().to_string().as_str()),
        "{report}"
    );

    // 导出失败（父目录不存在）⇒ 退出 5，且**不**写工程（顺序契约）。
    let saved = dir.join("never.yeban");
    let blocked_parent = dir.join("blocked");
    let blocked = blocked_parent.join("elements.txt");
    let run = invoke(&[
        "--open",
        source,
        "--export-elements",
        blocked.to_str().expect("utf8"),
        "--save-as",
        saved.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 5, "stderr={}", run.stderr);
    assert!(run.stderr.contains("导出元素清单"), "{}", run.stderr);
    assert!(!saved.exists(), "导出失败 ⇒ 不许写工程");

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

/// 判据 B10（**反转**）: 裸 `project.json` 必须被**明确拒绝** —— 容器是唯一工程格式
/// （ADR-0001 D43）。旧判据在这里断言"能打开 + `format=project-json`"。
#[test]
fn open_rejects_a_bare_project_json_with_a_precise_reason() {
    use yeban_model::container::{ContainerLimits, read_container};

    let dir = scratch_dir("bare");
    let container = write_real_container(&dir, "src.yeban");
    let bytes = std::fs::read(&container).expect("读容器");
    let json = read_container(&bytes, &ContainerLimits::default())
        .expect("真容器")
        .get("project.json")
        .expect("必有 project.json")
        .data
        .clone();
    let bare = dir.join("project.json");
    std::fs::write(&bare, &json).expect("写裸 JSON");

    let run = invoke(&["--open", bare.to_str().expect("utf8"), "--headless"]);
    assert_eq!(
        run.code, 3,
        "裸 project.json 必须退出 3; stdout={}",
        run.stdout
    );
    assert!(
        run.stdout.is_empty(),
        "失败不得留下半截输出（尤其不许退化成空工程）:\n{}",
        run.stdout
    );
    assert!(
        run.stderr.contains("不是 `.yeban` 容器"),
        "理由必须精确:\n{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("end-of-central-directory"),
        "必须带上容器原裁决:\n{}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("panicked"),
        "不许 panic: {}",
        run.stderr
    );

    // 拒绝的是**容器边界**，不是内容：同一个真容器照样能打开（它的 project.json 就是上面那份）。
    let ok = invoke(&["--open", container.to_str().expect("utf8"), "--headless"]);
    assert_eq!(ok.code, 0, "stderr={}", ok.stderr);
    assert!(
        ok.stdout.contains("format=yeban-container"),
        "唯一的 format 取值:\n{}",
        ok.stdout
    );
    assert!(
        !ok.stdout.contains("format=project-json"),
        "兼容读法已被删除:\n{}",
        ok.stdout
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B11: `--print-shortcuts` 是无窗口路径，且退出 0（CI 与人核对策略表的入口）。
#[test]
fn print_shortcuts_is_a_batch_command() {
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    assert!(run.stdout.contains("快捷键策略表"), "{}", run.stdout);
    assert_eq!(
        run.stdout.matches("headless ok").count(),
        1,
        "{}",
        run.stdout
    );
    // 策略表的判定结果必须真的在里面（画布列 vs IME 合成列）。
    assert!(run.stdout.contains("F5 → Session 视图"), "{}", run.stdout);
}

/// 判据 B12: `--export-midi` 从**真二进制**导出的字节可被 SMF 读取面读回，
/// 逐音符与工程一致，`MThd` 的 PPQ 字段是 960，且两次导出逐字节相同
/// （① 回读 / ② 逐音符 / ③ PPQ 头 / ④ 确定性，全部在**进程级**再证一次）。
#[test]
fn export_midi_writes_a_parseable_deterministic_smf_from_the_real_binary() {
    use yeban_render::midi::{MidiFormat, parse_smf, track_chunks};

    let dir = scratch_dir("export-midi");
    let source = write_real_container(&dir, "song.yeban");
    let source = source.to_str().expect("utf8");
    let first = dir.join("song.mid");
    let second = dir.join("song-again.mid");

    let mut printed_bytes = Vec::new();
    for target in [&first, &second] {
        let run = invoke(&[
            "--open",
            source,
            "--export-midi",
            target.to_str().expect("utf8"),
        ]);
        assert_eq!(run.code, 0, "stderr={}", run.stderr);
        let line = line_with(&run.stdout, "exported-midi:").expect("必须有 exported-midi: 行");
        assert_eq!(field(line, "ppq").as_deref(), Some("960"), "{line}");
        assert_eq!(field(line, "format").as_deref(), Some("parallel"), "{line}");
        assert_eq!(field(line, "tracks").as_deref(), Some("1"), "{line}");
        assert_eq!(field(line, "notes").as_deref(), Some("6"), "{line}");
        assert!(
            line.contains("from=") && line.contains("song.yeban"),
            "来源必须明写: {line}"
        );
        let expected = std::fs::metadata(target).expect("文件在").len();
        assert_eq!(
            field(line, "bytes").as_deref(),
            Some(expected.to_string().as_str()),
            "打印的字节数必须是实际落盘字节数: {line}"
        );
        assert!(
            !run.stdout.contains("panicked") && !run.stderr.contains("panicked"),
            "不许 panic: {}",
            run.stderr
        );
        printed_bytes.push(field(line, "bytes").expect("bytes="));
    }
    assert_eq!(
        printed_bytes[0], printed_bytes[1],
        "两次导出的字节数必须相同"
    );
    assert_eq!(
        std::fs::read(&first).expect("读 a"),
        std::fs::read(&second).expect("读 b"),
        "同一工程两次 --export-midi 的字节必须完全相同"
    );

    let bytes = std::fs::read(&first).expect("读导出的 SMF");
    // ③ `MThd` 的时间分度（大端）= 960 = 0x03C0。
    assert_eq!(&bytes[12..14], &[0x03, 0xC0], "PPQ 字段必须是 960");
    // ① 读取面读回 + chunk 布局。
    let chunks = track_chunks(&bytes).expect("chunk 布局");
    assert_eq!(&chunks[0].fourcc, b"MThd");
    assert_eq!(chunks.len(), 3, "MThd + conductor + 一条音符轨");
    for chunk in &chunks[1..] {
        let payload = &bytes[chunk.payload.clone()];
        assert_eq!(payload[payload.len() - 3..], [0xFF, 0x2F, 0x00]);
    }
    let parsed = parse_smf(&bytes).expect("回读");
    assert_eq!(parsed.format, MidiFormat::Parallel);
    assert_eq!(parsed.ppq, 960);
    // ② 逐音符（演示夹具的六颗音符，独立数出来的期望值）。
    let mut actual: Vec<(u8, u8, u8, u64, u64)> =
        parsed.notes.iter().map(|note| note.key()).collect();
    actual.sort_unstable();
    let mut expected: Vec<(u8, u8, u8, u64, u64)> = [
        (60_u8, 0_u64),
        (64, 480),
        (67, 960),
        (72, 1440),
        (74, 1920),
        (76, 2400),
    ]
    .iter()
    .map(|&(key, start)| (0, key, 100, start, 480))
    .collect();
    expected.sort_unstable();
    assert_eq!(actual, expected, "六颗音符逐项一致");

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

/// 判据 B13: `--export-midi` 的失败语义与既有 CLI 一致 —— 非容器输入 ⇒ 退出码 **3**
/// 且一个字节都不写；目标不可写 ⇒ 退出码 **5**、精确原因、**不留下半个文件**。
#[test]
fn export_midi_failures_reuse_the_existing_exit_codes_and_write_nothing() {
    let dir = scratch_dir("export-midi-bad");
    let junk = dir.join("junk.bin");
    std::fs::write(&junk, b"not a zip at all").expect("写垃圾");
    let target = dir.join("never.mid");

    let run = invoke(&[
        "--open",
        junk.to_str().expect("utf8"),
        "--export-midi",
        target.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 3, "非容器输入必须退出 3; stderr={}", run.stderr);
    assert!(
        run.stderr.contains("不是 `.yeban` 容器"),
        "必须转达容器裁决: {}",
        run.stderr
    );
    assert!(!target.exists(), "打开失败 ⇒ 不许写出任何 MIDI 字节");
    assert!(
        run.stdout.is_empty(),
        "失败不得留下半截输出: {}",
        run.stdout
    );

    // 目标父目录不存在 ⇒ 导出失败（退出码 5），且不留下任何文件 / 目录。
    let source = write_real_container(&dir, "good.yeban");
    let blocked = dir.join("missing").join("out.mid");
    let run = invoke(&[
        "--open",
        source.to_str().expect("utf8"),
        "--export-midi",
        blocked.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 5, "不可写路径必须退出 5; stderr={}", run.stderr);
    assert!(run.stderr.contains("导出 MIDI 到"), "{}", run.stderr);
    assert!(!blocked.exists(), "失败不得留下目标文件");
    assert!(!dir.join("missing").exists(), "失败不得凭空造出目录");
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
