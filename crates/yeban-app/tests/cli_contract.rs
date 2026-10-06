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
//! 每条调用都带 `--headless` / `--headless-idle` / `--dump-elements` / `--export-elements` /
//! `--export-midi` / `--save-as` / `--print-shortcuts` / `--help` / `--version` 之一。
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

/// 判据 B3b: `--project-sample empty` 在**真二进制**上是 0 轨，`default` 是 6 条普通轨。
///
/// 为什么这条必须走真二进制：`BASELINE-002` 的读数是**进程级**的
/// （`scripts/gates/measure_rss.py -- <本二进制> --project-sample empty --headless`），
/// 所以"取样对象真的是空工程"必须在同一条进程命令上被钉住 —— 单元判据证明不了 argv 与
/// 二进制一致。`tracks-all` **含**主总线，因此普通轨数 = `tracks-all - master-track`。
#[test]
fn empty_sample_reports_zero_tracks_and_default_reports_six() {
    let empty = invoke(&["--project-sample", "empty", "--headless"]);
    assert_eq!(empty.code, 0, "stderr={}", empty.stderr);
    let counts = line_with(&empty.stdout, "project-counts:").expect("必须有 project-counts 行");
    assert_eq!(
        field(counts, "tracks-all").as_deref(),
        Some("0"),
        "empty 必须真的是 0 轨: {counts}"
    );
    assert_eq!(
        field(counts, "master-track").as_deref(),
        Some("0"),
        "0 轨工程没有主总线: {counts}"
    );
    assert!(
        empty.stdout.contains("project-source: sample=empty"),
        "来源必须明写是 empty 样本:\n{}",
        empty.stdout
    );
    assert!(
        empty.stdout.contains("内置空工程 0 轨"),
        "输出不许把 0 轨空工程说成演示工程:\n{}",
        empty.stdout
    );

    let demo = invoke(&["--project-sample", "default", "--headless"]);
    assert_eq!(demo.code, 0, "stderr={}", demo.stderr);
    let counts = line_with(&demo.stdout, "project-counts:").expect("必须有 project-counts 行");
    let all: usize = field(counts, "tracks-all")
        .expect("tracks-all")
        .parse()
        .expect("轨道数必须是整数");
    let master: usize = field(counts, "master-track")
        .expect("master-track")
        .parse()
        .expect("主总线计数必须是整数");
    assert_eq!(master, 1, "default 必须有主总线: {counts}");
    assert_eq!(all - master, 6, "default 必须是 6 条普通轨: {counts}");
    assert_ne!(
        empty.stdout, demo.stdout,
        "两个样本的读数不许相同（相同 ⇒ 见证是空转）"
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
    let empty = invoke(&[
        "--project-sample",
        "empty",
        "--headless-idle",
        "--idle-seconds",
        "1",
    ]);
    assert_eq!(empty.code, 0, "stderr={}", empty.stderr);

    // 握手行是与 `headless ok` **刻意不同**的一行：后者的语义是"一个 Slint 对象都没构造"，
    // 而本命令真的建了树 —— 两者都出现才是自相矛盾。
    assert_eq!(
        empty.stdout.matches("headless-idle ok").count(),
        1,
        "本档的握手行必须恰好一次:\n{}",
        empty.stdout
    );
    assert!(
        !empty.stdout.contains("headless ok"),
        "不许打出 `headless ok`（它的语义是零 Slint 对象）:\n{}",
        empty.stdout
    );

    // ---- 见证 ----
    let witness = line_with(&empty.stdout, "headless-idle-witness:")
        .unwrap_or_else(|| panic!("必须有见证行:\n{}", empty.stdout));
    assert_eq!(
        field(witness, "windows-created").as_deref(),
        Some("1"),
        "平台必须恰好被要过一个窗口适配器: {witness}"
    );
    assert_eq!(
        field(witness, "rendered").as_deref(),
        Some("true"),
        "这一次必须真的发生了重绘: {witness}"
    );
    // 尺寸与 GUI / Tier-1 端口同源（`DemoScene` 的视口 = [UI-GRID-002] 的全展开档）。
    assert_eq!(
        field(witness, "size").as_deref(),
        Some("1920x1080"),
        "{witness}"
    );
    let rendered_lines: u64 = field(witness, "lines")
        .expect("lines")
        .parse()
        .expect("lines 必须是整数");
    let non_black: u64 = field(witness, "non-black-pixels")
        .expect("non-black-pixels")
        .parse()
        .expect("非黑像素数必须是整数");
    let colors: usize = field(witness, "distinct-colors")
        .expect("distinct-colors")
        .parse()
        .expect("颜色数必须是整数");
    assert_eq!(
        rendered_lines, 1080,
        "逐行光栅化必须覆盖整个窗口高度: {witness}"
    );
    assert!(non_black > 0, "见证不许是空转（非黑像素为 0）: {witness}");
    assert!(
        colors >= 2,
        "界面至少应有背景 + 一个图元颜色（>1 才叫真的画了东西）: {witness}"
    );

    // 见证行里的 `elements=` 与同一命令的 `view-counts:` 必须**同源同值**
    // （都来自 `ElementRegistry::from_view`）—— 否则这两个读数就没法互相对账。
    let view = line_with(&empty.stdout, "view-counts:").expect("必须有 view-counts 行");
    assert_eq!(
        field(witness, "elements"),
        field(view, "elements"),
        "见证行与 view-counts 的元素数必须一致:\n{witness}\n{view}"
    );
    let elements: usize = field(witness, "elements")
        .expect("elements")
        .parse()
        .expect("元素数必须是整数");
    assert!(elements > 0, "非平凡的控件树的元素数必须 > 0: {witness}");

    // ---- 取样对象真的是规范所指的那个（0 轨空工程）----
    let counts = line_with(&empty.stdout, "project-counts:").expect("必须有 project-counts 行");
    assert_eq!(
        field(counts, "tracks-all").as_deref(),
        Some("0"),
        "{counts}"
    );
    assert!(
        empty.stdout.contains("project-source: sample=empty"),
        "来源必须明写 empty:\n{}",
        empty.stdout
    );

    // ---- 空闲读数必须是**实测**的（不是把输入抄一遍）----
    let idle = line_with(&empty.stdout, "headless-idle-idle:").expect("必须有空闲行");
    assert_eq!(field(idle, "seconds").as_deref(), Some("1"), "{idle}");
    let elapsed_ms: u128 = field(idle, "elapsed-ms")
        .expect("elapsed-ms")
        .parse()
        .expect("实测毫秒数必须是整数");
    assert!(
        (990..30_000).contains(&elapsed_ms),
        "空闲实测时长应当在请求值附近（1 秒级）: {idle}"
    );
    let ticks: u64 = field(idle, "ticks")
        .expect("ticks")
        .parse()
        .expect("tick 数必须是整数");
    assert!(ticks > 0, "空闲循环必须真的转过: {idle}");

    // ---- 边界行必须说清"没做什么"----
    assert!(
        empty.stdout.contains("未创建 OS 窗口"),
        "必须声明本档没有 OS 窗口:\n{}",
        empty.stdout
    );

    // ---- 对照：`--headless` 不许出现任何见证行（那才是"零 Slint 对象"）----
    let plain = invoke(&["--project-sample", "empty", "--headless"]);
    assert_eq!(plain.code, 0, "stderr={}", plain.stderr);
    assert_eq!(
        plain.stdout.matches("headless ok").count(),
        1,
        "{}",
        plain.stdout
    );
    assert!(
        !plain.stdout.contains("headless-idle-witness:"),
        "--headless 不该有任何建树见证:\n{}",
        plain.stdout
    );
    assert_ne!(
        plain.stdout, empty.stdout,
        "两个模式的输出必须不同（相同 ⇒ 新开关没生效）"
    );

    // ---- 用法错误档：`--idle-seconds` 单独给必须是退出码 2（真二进制上也不许静默忽略）----
    let lonely = invoke(&["--idle-seconds", "1"]);
    assert_eq!(lonely.code, 2, "stderr={}", lonely.stderr);
    assert!(
        lonely.stderr.contains("--headless-idle"),
        "错误信息必须点名正确的搭档:\n{}",
        lonely.stderr
    );
    let unpaired = invoke(&["--headless-idle"]);
    assert_eq!(unpaired.code, 2, "stderr={}", unpaired.stderr);
    assert!(
        unpaired.stderr.contains("--idle-seconds"),
        "错误信息必须点名缺失的取值:\n{}",
        unpaired.stderr
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

/// 判据 B7b: **默认产物层面**拒绝 `--enable-mcp-http`（`[ROAD-M4-001]` / `[MUST-GATE-009]`）。
///
/// 这是"危险能力默认关"在 **app 二进制产物**上的断言（与
/// `scripts/gates/check_release_defaults.sh` 对 `yeban-mcp` 二进制做的那一条同一条纪律、
/// 另一个产物）：默认构建里 `in-process-mcp` 没被编译，于是这个开关**必须**以用法错误
/// （退出码 2）被拒，并**点名**缺的是哪个 feature —— 不许静默地"以为开了其实没开"。
///
/// 为什么只在默认构建里跑：带 `in-process-mcp` 时它是**合法**的 GUI 开关，给它会走
/// 开窗口那一档，而本文件只用无窗口参数（见模块文档的纪律）。带 feature 的正面判据在
/// `tests/in_process_mcp.rs`（真环回 socket 上的完整往返）。
#[cfg(not(feature = "in-process-mcp"))]
#[test]
fn default_artifact_refuses_the_in_process_mcp_switch() {
    let run = invoke(&["--enable-mcp-http"]);
    assert_eq!(
        run.code, 2,
        "默认产物必须拒绝 --enable-mcp-http; stderr={}",
        run.stderr
    );
    assert!(
        run.stderr.contains("--enable-mcp-http") && run.stderr.contains("in-process-mcp"),
        "拒绝必须同时点名开关与缺的 feature: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("用法:"),
        "用法错误必须带用法提示: {}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "不该有 stdout");
}

/// 判据 B7d: **默认产物层面**拒绝 `--export-logic`（`[ARCH-FMT-002]` / `[ROAD-M4-007]`）。
///
/// 与 B7c 同一条纪律、另一个产物形态（`.logicx` 是**目录**而不是单文件）：默认构建里
/// `experimental-logic-export` 没被编译，于是这个开关**必须**以用法错误（退出码 2）被拒，
/// 并**点名**缺的是哪个 feature。
#[cfg(not(feature = "experimental-logic-export"))]
#[test]
fn default_artifact_refuses_the_experimental_logic_switch() {
    let dir = scratch_dir("logic-not-compiled");
    let target = dir.join("never.logicx");
    let run = invoke(&["--export-logic", target.to_str().expect("utf8")]);
    assert_eq!(
        run.code, 2,
        "默认产物必须拒绝 --export-logic; stderr={}",
        run.stderr
    );
    assert!(
        run.stderr.contains("--export-logic") && run.stderr.contains("experimental-logic-export"),
        "拒绝必须同时点名开关与缺的 feature: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("用法:"),
        "用法错误必须带用法提示: {}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "不该有 stdout");
    assert!(!target.exists(), "被拒的开关不许写出任何文件或目录");

    let _ = std::fs::remove_dir_all(&dir);
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
    let dir = scratch_dir("als-not-compiled");
    let target = dir.join("never.als");
    let run = invoke(&["--export-als", target.to_str().expect("utf8")]);
    assert_eq!(
        run.code, 2,
        "默认产物必须拒绝 --export-als; stderr={}",
        run.stderr
    );
    assert!(
        run.stderr.contains("--export-als") && run.stderr.contains("experimental-als-export"),
        "拒绝必须同时点名开关与缺的 feature: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("用法:"),
        "用法错误必须带用法提示: {}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "不该有 stdout");
    assert!(!target.exists(), "被拒的开关不许写出任何文件");

    let _ = std::fs::remove_dir_all(&dir);
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

    let dir = scratch_dir("lock-refuse");
    let source = write_real_container(&dir, "source.yeban");
    let target = dir.join("target.yeban");

    // 先让 target 里是 `source` 那份工程（带 `binary-history` 与非空资产池）。
    let first = invoke(&[
        "--open",
        source.to_str().expect("utf8"),
        "--save-as",
        target.to_str().expect("utf8"),
    ]);
    assert_eq!(first.code, 0, "第一次保存必须成功; stderr={}", first.stderr);
    let before = std::fs::read(&target).expect("读原文");

    // 另一个持有者拿同一把锁（排他写）。
    let guard = acquire(&target, LockMode::ExclusiveWrite).expect("取排他写建议锁");
    // 这次**不带 --open** ⇒ 写的是样本工程，字节与 `before` 必然不同。
    let blocked = invoke(&["--save-as", target.to_str().expect("utf8")]);
    assert_eq!(
        blocked.code, 4,
        "被别的持有者持锁时 --save-as 必须拒绝（退出 4）; stderr={}",
        blocked.stderr
    );
    assert!(
        blocked.stderr.contains("保存到") && blocked.stderr.contains("拒绝写入"),
        "拒绝的原因必须写在 stderr 里: {}",
        blocked.stderr
    );
    assert_eq!(
        std::fs::read(&target).expect("旧文件仍在"),
        before,
        "被拒绝的保存绝不能碰旧文件（这就是『取锁』与『没取锁』的区别）"
    );
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "被拒绝的保存不得留下临时文件: {leftovers:?}"
    );

    // 释放 ⇒ 同一条命令必须成功（证明第 2 步拒绝的原因就是那把锁）。
    drop(guard);
    let after = invoke(&["--save-as", target.to_str().expect("utf8")]);
    assert_eq!(
        after.code, 0,
        "释放锁之后必须能保存; stderr={}",
        after.stderr
    );
    assert_ne!(
        std::fs::read(&target).expect("读回"),
        before,
        "成功的那一次必须真的换了内容（否则上面的『逐字节未变』是假证据）"
    );
    assert!(
        !lock_path(&target).exists(),
        "成功的排他保存必须收走自己的锁文件: {}",
        lock_path(&target).display()
    );

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

/// 判据 B14: 带 `experimental-als-export` 时，`--export-als` 写出 Gzip 文档并把
/// **映射损失表**逐条打到 stdout（`ADR-0001 D47` 的价值 = 损失表对用户可见）。
///
/// 与 B12 同一条思路，但断言的是另一件事：B12 证"字节能被 SMF 读取面读回"，
/// 这里证"**没被映射的东西真的被说出来了**" —— 只打印一个计数不算交付。
#[cfg(feature = "experimental-als-export")]
#[test]
fn export_als_writes_a_gzip_document_and_prints_the_loss_summary() {
    let dir = scratch_dir("export-als");
    let source = write_real_container(&dir, "song.yeban");
    let source = source.to_str().expect("utf8");
    let first = dir.join("song.als");
    let second = dir.join("song-again.als");

    let run = invoke(&[
        "--open",
        source,
        "--export-als",
        first.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);

    let line = line_with(&run.stdout, "exported-als:").expect("必须有 exported-als: 行");
    let losses: usize = field(line, "losses")
        .expect("losses=")
        .parse()
        .expect("条数是整数");
    assert!(losses >= 1, "本仓库无参考 .als ⇒ 必然带损失条目: {line}");
    assert!(
        line.contains("from=") && line.contains("song.yeban"),
        "来源必须明写: {line}"
    );
    let expected = std::fs::metadata(&first).expect("文件在").len();
    assert_eq!(
        field(line, "bytes").as_deref(),
        Some(expected.to_string().as_str()),
        "打印的字节数必须是实际落盘字节数: {line}"
    );

    // 损失表**真的**打出来了：计数行 + 逐条行, 且条数与 `count=` 对得上（超上限则截断并明写）。
    let count_line = line_with(&run.stdout, "als-losses:").expect("als-losses: 行");
    let counted: usize = field(count_line, "count")
        .expect("count=")
        .parse()
        .expect("整数");
    assert_eq!(counted, losses, "{count_line}");
    let shown = run
        .stdout
        .lines()
        .filter(|text| text.starts_with("als-loss: "))
        .count();
    let cap = yeban_app::cli::MAX_ALS_LOSS_LINES;
    assert_eq!(shown, counted.min(cap), "显示条数 = min(条数, 上限)");
    if counted > cap {
        assert!(
            run.stdout.contains("als-losses-truncated:") && run.stdout.contains("more"),
            "超上限必须礼貌截断并写明还剩几条:\n{}",
            run.stdout
        );
    }
    assert!(
        run.stdout.contains("未映射:") || run.stdout.contains("非等价:"),
        "条目必须带机器可读的两分法前缀: {}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("panicked") && !run.stderr.contains("panicked"),
        "不许 panic: {}",
        run.stderr
    );

    // 产物是 Gzip（魔数），且同一工程两次导出**逐字节相同**（与 --export-midi 同款确定性口径）。
    let bytes = std::fs::read(&first).expect("读 .als");
    assert_eq!(&bytes[..2], &[0x1F, 0x8B], "Gzip 魔数必须是 1f 8b");
    let again = invoke(&[
        "--open",
        source,
        "--export-als",
        second.to_str().expect("utf8"),
    ]);
    assert_eq!(again.code, 0, "stderr={}", again.stderr);
    assert_eq!(
        std::fs::read(&first).expect("读 a"),
        std::fs::read(&second).expect("读 b"),
        "同一工程两次 --export-als 的字节必须完全相同"
    );

    // 目标父目录不存在 ⇒ 复用导出失败那一档（退出码 5），且不留半个文件。
    let blocked = dir.join("missing").join("out.als");
    let failed = invoke(&[
        "--open",
        source,
        "--export-als",
        blocked.to_str().expect("utf8"),
    ]);
    assert_eq!(
        failed.code, 5,
        "不可写路径必须退出 5; stderr={}",
        failed.stderr
    );
    assert!(failed.stderr.contains("导出 .als 到"), "{}", failed.stderr);
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

/// 判据 B15: 带 `experimental-logic-export` 时，`--export-logic` 写出 `.logicx`
/// **bundle 目录**（四个文件）并把映射损失表逐条打到 stdout。
///
/// 与 B14 同一条思路，但产物是多文件目录：因此它另外钉住 ① 目录与四个文件真的在、
/// ② `ProjectData` 的**供体**头部（`23 47 C0 AB` / 版本码 `0x09CF` / 声明长度 @0x10 /
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
    let source = source.to_str().expect("utf8");
    let first = dir.join("Song.logicx");
    let second = dir.join("Song-again.logicx");

    let run = invoke(&[
        "--open",
        source,
        "--export-logic",
        first.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);

    let line = line_with(&run.stdout, "exported-logic:").expect("必须有 exported-logic: 行");
    let losses: usize = field(line, "losses")
        .expect("losses=")
        .parse()
        .expect("条数是整数");
    assert!(losses >= 1, "打开结论的适用范围那条必然在: {line}");
    assert_eq!(field(line, "files").as_deref(), Some("4"), "{line}");
    assert!(
        line.contains("from=") && line.contains("song.yeban"),
        "来源必须明写: {line}"
    );

    // ① bundle 目录与四个文件真的落盘了。
    let project_data = first.join("Alternatives/000/ProjectData");
    for relative in [
        "Alternatives/000/ProjectData",
        "Alternatives/000/MetaData.plist",
        "Alternatives/000/DisplayState.plist",
        "Resources/ProjectInformation.plist",
    ] {
        assert!(first.join(relative).is_file(), "{relative} 必须存在");
    }

    // ② `ProjectData` 的实测头部。
    //
    // ⚠ 这一条随**供体选择**改过（`ROAD-M4-011` 多轨导出）：`--export-logic` 现在按工程的
    // **MIDI 轨条数**选骨架 —— 1 条（及 0 条）仍用 `jonkubis/logicproformatwriter`（MIT）的
    // `F0_baseline`（版本码 `0x09CF`），**2 条起**用**负责人自己**用 Logic Pro 12.2
    // （2026-10-06）存的两轨工程（版本码 `0x09D0`，sha256
    // `cfeabcfc11c5f001edfb48d5711cbccb57944aa23c22bd926483c704dde36db6`）。演示工程有
    // **4** 条 MIDI 轨 ⇒ 走负责人那份，**2** 条被映射（实测容量 2），其余 2 条逐条登记为未映射。
    // 根头仍**逐字节保留所选供体的**：版本码声明的是**这份文档的落盘格式**，改写它等于让 Logic
    // 用更新的解析器去读更老的记录。自研写入器（`yeban_render::logic::project_data`）的
    // `0x09D0` 仍由 render 侧的 `container_header_fields_carry_the_measured_modern_values` 钉住。
    let bytes = std::fs::read(&project_data).expect("读 ProjectData");
    assert_eq!(&bytes[..4], &[0x23, 0x47, 0xC0, 0xAB], "根魔数");
    assert_eq!(
        &bytes[0x04..0x10],
        &[
            0xD0, 0x09, 0x03, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08, 0x00
        ],
        "根头 +0x04 必须是**所选供体**落盘的格式版本码 —— 负责人 2 轨那份是 0x09D0"
    );
    assert_eq!(&bytes[0x18..0x1c], b"gnoS", "第一个 chunk 名必须是 gnoS");
    let declared =
        u32::from_le_bytes([bytes[0x10], bytes[0x11], bytes[0x12], bytes[0x13]]) as usize;
    assert_eq!(declared, bytes.len() - 0x18, "声明载荷长度 @0x10");

    // ②b 它真的**带着供体的通道簇**：按 36 字节记录头走完全文，数四个家族。
    //     自研写入器这四族一个都没有（`Envi`/`AuCO`/`GenM`/`Trak`），这正是它被判为
    //     "没有轨道"的原因；供体路线把它们原样带进来。读数 = 负责人 2 轨那份的实测逐族表。
    let mut families: std::collections::BTreeMap<[u8; 4], usize> =
        std::collections::BTreeMap::new();
    let mut at = 0x18usize;
    let mut records = 0usize;
    while at + 0x24 <= bytes.len() {
        let tag = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        let size = u32::from_le_bytes([
            bytes[at + 0x1c],
            bytes[at + 0x1d],
            bytes[at + 0x1e],
            bytes[at + 0x1f],
        ]) as usize;
        *families.entry(tag).or_default() += 1;
        records += 1;
        at += 0x24 + size;
    }
    assert_eq!(at, bytes.len(), "记录流必须恰好铺满声明载荷");
    assert_eq!(records, 507, "负责人 2 轨供体的记录条数是实测的 507 条");
    // 落盘字节是**可读名的反序**（`Envi` 落盘为 `ivnE`），与 `yeban_render::logic` 的记录模型一致。
    assert_eq!(
        families.get(b"ivnE"),
        Some(&45),
        "ivnE 环境对象条数必须是负责人 2 轨那份的实测值"
    );
    assert_eq!(families.get(b"OCuA"), Some(&286), "OCuA 混音条必须来自供体");
    assert_eq!(families.get(b"MneG"), Some(&1), "MneG 必须来自供体");
    assert_eq!(
        families.get(b"karT"),
        Some(&31),
        "karT 轨道家族必须来自供体"
    );
    assert_eq!(
        families.get(b"UCuA"),
        Some(&19),
        "UCuA 通道条状态（含第二条轨道新激活的那 8 条）必须来自供体"
    );

    // ③ `MetaData.plist` 是二进制 plist。
    let meta = std::fs::read(first.join("Alternatives/000/MetaData.plist")).expect("读 MetaData");
    assert_eq!(&meta[..8], b"bplist00", "标准二进制 plist 魔数");

    // ④ 损失表逐条可见，且两分法前缀在。
    let count_line = line_with(&run.stdout, "logic-losses:").expect("logic-losses: 行");
    let counted: usize = field(count_line, "count")
        .expect("count=")
        .parse()
        .expect("整数");
    assert_eq!(counted, losses, "{count_line}");
    let shown = run
        .stdout
        .lines()
        .filter(|text| text.starts_with("logic-loss: "))
        .count();
    let cap = yeban_app::cli::MAX_LOGIC_LOSS_LINES;
    assert_eq!(shown, counted.min(cap), "显示条数 = min(条数, 上限)");
    assert!(
        run.stdout.contains("未映射:") || run.stdout.contains("非等价:"),
        "条目必须带机器可读的两分法前缀: {}",
        run.stdout
    );

    // ⑤ 两次导出（不同目录）逐文件相同。
    let again = invoke(&[
        "--open",
        source,
        "--export-logic",
        second.to_str().expect("utf8"),
    ]);
    assert_eq!(again.code, 0, "stderr={}", again.stderr);
    for relative in [
        "Alternatives/000/ProjectData",
        "Alternatives/000/MetaData.plist",
        "Alternatives/000/DisplayState.plist",
        "Resources/ProjectInformation.plist",
    ] {
        assert_eq!(
            std::fs::read(first.join(relative)).expect("读第一次"),
            std::fs::read(second.join(relative)).expect("读第二次"),
            "{relative} 两次导出必须逐字节相同"
        );
    }

    // ⑥ 目标被一个**普通文件**挡住 ⇒ 建不出目录 ⇒ 复用导出失败那一档（退出码 5）。
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, b"not a directory").expect("写遮挡文件");
    let failed = invoke(&[
        "--open",
        source,
        "--export-logic",
        blocker.join("Sub.logicx").to_str().expect("utf8"),
    ]);
    assert_eq!(failed.code, 5, "stderr={}", failed.stderr);
    assert!(
        failed.stderr.contains("导出 .logicx 到"),
        "{}",
        failed.stderr
    );

    let _ = std::fs::remove_dir_all(&dir);
}
