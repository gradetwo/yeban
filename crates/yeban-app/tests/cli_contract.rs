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

/// 判据 B3b: `--project-sample empty` 在**真二进制**上是 0 条音轨 ＋ 1 条主总线，
/// `default` 是 6 条普通轨。
///
/// 为什么这条必须走真二进制：`BASELINE-002` 的读数是**进程级**的
/// （`scripts/gates/measure_rss.py -- <本二进制> --project-sample empty --headless`），
/// 所以"取样对象真的是空工程"必须在同一条进程命令上被钉住 —— 单元判据证明不了 argv 与
/// 二进制一致。`tracks-all` **含**主总线，因此普通轨数 = `tracks-all - master-track`。
#[test]
fn empty_sample_reports_zero_audio_tracks_and_default_reports_six() {
    let empty = invoke(&["--project-sample", "empty", "--headless"]);
    assert_eq!(empty.code, 0, "stderr={}", empty.stderr);
    let counts = line_with(&empty.stdout, "project-counts:").expect("必须有 project-counts 行");
    assert_eq!(
        field(counts, "tracks-all").as_deref(),
        Some("1"),
        "empty 必须是 1 条轨（`#161` 起出厂自带主总线）: {counts}"
    );
    assert_eq!(
        field(counts, "master-track").as_deref(),
        Some("1"),
        "empty 的 1 条轨必须就是主总线: {counts}"
    );
    assert!(
        empty.stdout.contains("project-source: sample=empty"),
        "来源必须明写是 empty 样本:\n{}",
        empty.stdout
    );
    assert!(
        empty.stdout.contains("内置空工程 0 音轨 ＋ 1 主总线"),
        "输出不许把空工程说成演示工程:\n{}",
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

    // ---- 取样对象真的是规范所指的那个（0 音轨 ＋ 1 主总线的空工程）----
    let counts = line_with(&empty.stdout, "project-counts:").expect("必须有 project-counts 行");
    assert_eq!(
        field(counts, "tracks-all").as_deref(),
        Some("1"),
        "{counts}"
    );
    assert_eq!(
        field(counts, "master-track").as_deref(),
        Some("1"),
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
    // ⭐ `#215`：`--save-as` 不给 `--open` 是**创建**一份新工程 ⇒ 创建时刻写真实时间。
    // 因此对账口径是"除创建时刻外逐字段等于演示工程"，并单独断言时刻真的非 0。
    let read_back = yeban_app::open::open_project_file(&target).expect("读回");
    assert!(
        read_back.metadata.created_at_unix_ms > 0,
        "新建工程必须写真实创建时刻（实得 0 ⇒ BWF 起源日期会退回 1970-01-01）"
    );
    let mut expected = yeban_app::bridge::demo_project();
    expected.metadata.created_at_unix_ms = read_back.metadata.created_at_unix_ms;
    assert_eq!(read_back, expected, "除创建时刻外必须逐字段等于演示工程");
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
    assert_eq!(
        run.code, 2,
        "默认产物必须拒绝 --enable-ui-mcp-http; stderr={}",
        run.stderr
    );
    assert!(
        run.stderr.contains("--enable-ui-mcp-http") && run.stderr.contains("ui-mcp-http"),
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
/// 今天的样例是 `Shift+Enter → 采纳 AI 建议` 与 `[ → 试听主线`。
///
/// 能无头查的就是这三面。`apply_action` 对 `Undo`/`Redo`/`OpenTimeMachine` 在**没有撤销
/// 端口**的装配下也返回 `false` —— 那是装配缺件, 不是动作没实现, 所以这条判据判的是
/// "有没有落地实现", 而不是"当前这个装配有没有接上它"。真实窗口上的消费 / 拒绝由
/// `tests/live_ui_mcp.rs` 的判据 16 用真事件源直接见证。
#[test]
fn shortcut_table_status_matches_the_resolution_and_host_pipeline() {
    use yeban_app::cli::{UNIMPLEMENTED_MARKER, shortcut_rows};
    use yeban_app::host::action_has_implementation;
    use yeban_app::input::{Focus, InputContext, Resolution};

    // ① 真二进制的 stdout：表**渲染出来**的样子（用户看到的就是它）。
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);

    let rows = shortcut_rows();
    assert_eq!(
        rows.len(),
        34,
        "快捷键表的**条目**数（不是行数）：18 条 §7.1 核心快捷键 ＋ `[UI-NOTE-005]` §3.5 项 1–2 的 \
         8 条方向键微调（四个修饰组合 × 两个方向）＋ ⭐ `[UI-NOTE-005]` §3 项 3 的 2 条 \
         时值伸缩（`Shift+←/→`）＋ ⭐ `[ARCH-RT-001]` 的 2 条节拍器键 \
         （`Cmd/Ctrl+Alt+N` 节拍器 / `Cmd/Ctrl+Alt+D` 预备拍）＋ ⭐ 2 条混音微调 \
         （`Cmd/Ctrl+Alt+↑/↓` 音量 / `+←/→` 声相，两格 × 两方向）"
    );
    // 表头那三行 `# …` 是说明文字, 其中一行**定义**了标记; 数标记只数**条目行**。
    let printed_marks: usize = run
        .stdout
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .map(|line| line.matches(UNIMPLEMENTED_MARKER).count())
        .sum();
    assert_eq!(
        printed_marks,
        rows.iter().filter(|row| !row.implemented).count(),
        "stdout 条目行里的 `{UNIMPLEMENTED_MARKER}` 出现次数必须等于表里未实现的**条目**数:\n{}",
        run.stdout
    );

    // 下表"画布聚焦"列用的就是它。
    let mut canvas = InputContext::new();
    canvas.set_focus(Focus::MainCanvas);

    let mut checked = 0usize;
    let mut marked = 0usize;
    for row in rows {
        // ② 标签绑的键必须解析出标签声称的动作（表内部不能自相矛盾）。
        let resolved = canvas.resolve(row.key, row.modifiers);
        assert_eq!(
            resolved,
            Resolution::Action(row.action),
            "`{}` 的键位解析出的不是它声称的动作: 得到 {resolved:?}",
            row.label
        );
        // ③ 表的主张必须与宿主的事实一致 —— 分叉即红。
        assert_eq!(
            action_has_implementation(row.action),
            row.implemented,
            "`{}` ({:?}) 的落地状态分叉: 表说 implemented={}, 宿主说 {}",
            row.label,
            row.action,
            row.implemented,
            action_has_implementation(row.action),
        );

        // ① stdout 那一行必须**恰好**按主张渲染（标了就要打印出来, 没标就不能偷偷标）。
        let rendered = if row.implemented {
            row.label.to_owned()
        } else {
            marked += 1;
            format!("{} {}", row.label, UNIMPLEMENTED_MARKER)
        };
        let line = run
            .stdout
            .lines()
            .find(|line| line.trim_start().starts_with(&rendered))
            .unwrap_or_else(|| {
                panic!("`--print-shortcuts` 里找不到 `{rendered}`:\n{}", run.stdout);
            });
        assert!(
            line.contains(&format!("{resolved:?}")),
            "`{rendered}` 那一行的画布聚焦列与 resolve 不符:\n{line}"
        );
        checked += 1;
    }

    eprintln!(
        "[shortcut-honesty] 逐条对账 {checked} 条快捷键: 渲染列 / resolve / host::action_has_implementation \
         三面一致; 其中标记 `{UNIMPLEMENTED_MARKER}` 的 {marked} 条（例如 `Shift+Enter → 采纳 AI 建议` 与 \
         `[ → 试听主线`）被宿主如实拒绝"
    );
}

/// 判据 B11c: `Delete/Backspace → 删除` 已落地 —— 宿主认它, 快捷表也说能用, 渲染不带标记。
///
/// 这一条是"删除回退成不实现"的**直接牙**：把 `Action::DeleteSelection` 放回
/// `host::action_has_implementation` 的 `!matches!` 名单（或把 `cli.rs` 的
/// `implemented` 改回 `false`）⇒ 这里立刻红。B11b 也覆盖这一条, 但它的失败信息是
/// "三面之一分叉"; 本判据的失败信息直接点名"删除被回退了"。
#[test]
fn delete_selection_is_implemented_in_the_host_table_and_rendered_output() {
    use yeban_app::cli::{UNIMPLEMENTED_MARKER, shortcut_rows};
    use yeban_app::host::action_has_implementation;
    use yeban_app::input::Action;

    assert!(
        action_has_implementation(Action::DeleteSelection),
        "宿主必须认 `DeleteSelection`（回退成不实现即红）"
    );
    let row = shortcut_rows()
        .iter()
        .find(|row| row.action == Action::DeleteSelection)
        .expect("快捷表必须有 `Delete/Backspace` 这一条");
    assert!(
        row.implemented,
        "快捷表必须说 `Delete/Backspace` 能用（否则用户看到 `(未实现)`）"
    );
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    let delete_line = run
        .stdout
        .lines()
        .find(|line| line.contains("Delete/Backspace"))
        .unwrap_or_else(|| panic!("`--print-shortcuts` 里找不到删除那一行:\n{}", run.stdout));
    assert!(
        !delete_line.contains(UNIMPLEMENTED_MARKER),
        "已实现的删除行**不得**带 `{UNIMPLEMENTED_MARKER}`: {delete_line}"
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
    use yeban_app::cli::{UNIMPLEMENTED_MARKER, shortcut_rows};
    use yeban_app::host::action_has_implementation;
    use yeban_app::input::Action;

    for action in [Action::ZoomToSelection, Action::ZoomToFit] {
        assert!(
            action_has_implementation(action),
            "宿主必须认 `{action:?}`（回退成不实现即红）"
        );
        let row = shortcut_rows()
            .iter()
            .find(|row| row.action == action)
            .unwrap_or_else(|| panic!("快捷表必须有 `{action:?}` 这一条"));
        assert!(
            row.implemented,
            "快捷表必须说 `{}` 能用（否则用户看到 `{UNIMPLEMENTED_MARKER}`）",
            row.label
        );
    }
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    for label in ["Z → 选区撑满视口", "Shift+Z → 全曲总览"] {
        let line = run
            .stdout
            .lines()
            .find(|line| line.trim_start().starts_with(label))
            .unwrap_or_else(|| panic!("`--print-shortcuts` 里找不到 `{label}`:\n{}", run.stdout));
        assert!(
            !line.contains(UNIMPLEMENTED_MARKER),
            "已实现的缩放行**不得**带 `{UNIMPLEMENTED_MARKER}`: {line}"
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
    use yeban_app::cli::{UNIMPLEMENTED_MARKER, shortcut_rows};
    use yeban_app::host::action_has_implementation;
    use yeban_app::input::Action;

    assert!(
        action_has_implementation(Action::Duplicate),
        "宿主必须认 `Duplicate`（回退成不实现即红）"
    );
    let row = shortcut_rows()
        .iter()
        .find(|row| row.action == Action::Duplicate)
        .expect("快捷表必须有 `Cmd/Ctrl+D` 这一条");
    assert!(
        row.implemented,
        "快捷表必须说 `Cmd/Ctrl+D` 能用（否则用户看到 `{UNIMPLEMENTED_MARKER}`）"
    );
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    let line = run
        .stdout
        .lines()
        .find(|line| line.contains("Cmd/Ctrl+D"))
        .unwrap_or_else(|| panic!("`--print-shortcuts` 里找不到复制那一行:\n{}", run.stdout));
    assert!(
        !line.contains(UNIMPLEMENTED_MARKER),
        "已实现的复制行**不得**带 `{UNIMPLEMENTED_MARKER}`: {line}"
    );
    // 标签必须写明**偏移**（不是"原位"）：读者据此知道副本会落在下一格。
    assert!(
        row.label.contains("复制到下一格"),
        "标签必须命名偏移量, 实际 `{}`",
        row.label
    );
}

/// 判据 B11f: **能力缺失**的那两条快捷键仍然如实标记为 `(未实现)` —— 本判据是"它还不可用"的牙。
///
/// ## 为什么要有这一条（它不是 B11b 的重复）
///
/// B11b（`shortcut_table_status_matches_the_resolution_and_host_pipeline`）判的是**三面一致**
/// ——"表 / 解析 / 宿主有没有各说各话"。它**允许**这三面一起改成"已实现"。而本判据钉住的是
/// **事实本身**：`Shift+Enter → 采纳 AI 建议` 与 `[ → 试听主线` 的能力**今天不存在**
/// （缺的不是接线，是"待采纳的提案身份"与 `[ARCH-RT-005]` 的 A/B 并发渲染）。
///
/// 因此它会在两种情况下变红，两种都该红：
/// 1. **假实现**：把 `implemented` 改成 `true`（或把动作从 `!matches!` 移出）而能力仍未落地
///    ⇒ 第 ①②③ 步全部失败；
/// 2. **真实现但忘了改判据**：谁真的把能力接上了（例如引擎把 `ARCH-RT-005` 落进
///    `IMPLEMENTED_SPEC_IDS`）⇒ 第 ⑤ 步失败，逼作者回来更新这一条、`cli.rs` 的
///    `implemented` 与 `host::action_has_implementation` 的名单。
///
/// ## 各步的"单位"（防止把行数当条数）
///
/// - 第 ③ 步数的是 `--print-shortcuts` 的**条目行**里 `(未实现)` 的出现**次数**（单位：条目标记），
///   表头那一行 `# (未实现) = …` 是**说明文字**，不计入（口径与 B11b 第 ① 步同款）。
/// - 第 ④ 步数的是 `shortcut_rows()` 里 `implemented == false` 的**条目数**（单位：条目）。
///
/// 运行时的"按下不消费且一位不变"由 `tests/live_ui_mcp.rs` 的
/// `the_ai_suggestion_and_audition_keys_are_rejected_without_any_state_change` 用真事件源见证。
#[test]
fn the_ai_suggestion_and_audition_keys_are_still_declared_unimplemented() {
    use yeban_app::cli::{UNIMPLEMENTED_MARKER, shortcut_rows};
    use yeban_app::host::action_has_implementation;
    use yeban_app::input::{Action, Focus, InputContext, Modifiers, PhysicalKey, Resolution};

    // ① 宿主自己的陈述：三条都**没有**可作用的实现。
    //    第三条（`AuditionProposal`）不在快捷表里，但同一份陈述同样挡住它。
    let still_unimplemented = [
        Action::AcceptAiSuggestion,
        Action::AuditionMain,
        Action::AuditionProposal,
    ];
    for action in still_unimplemented {
        assert!(
            !action_has_implementation(action),
            "`{action:?}` 的能力今天不存在 ⇒ `host::action_has_implementation` 必须返回 false \
             （回退成 `true` 就是一次假实现）"
        );
    }

    // ② 键**仍然绑定**到这两个动作（缺口是"动作没落地"，不是"键丢了"）。
    let mut canvas = InputContext::new();
    canvas.set_focus(Focus::MainCanvas);
    assert_eq!(
        canvas.resolve(PhysicalKey::Enter, Modifiers::shift()),
        Resolution::Action(Action::AcceptAiSuggestion),
        "`Shift+Enter` 必须仍然解析出 `AcceptAiSuggestion`（物理码路径）"
    );
    assert_eq!(
        canvas.resolve(PhysicalKey::BracketLeft, Modifiers::none()),
        Resolution::Action(Action::AuditionMain),
        "`[` 必须仍然解析出 `AuditionMain`（物理码路径）"
    );

    // ③ 表里只有这两条 `implemented: false`，且就是它们（单位：条目）。
    let unimplemented_rows: Vec<Action> = shortcut_rows()
        .iter()
        .filter(|row| !row.implemented)
        .map(|row| row.action)
        .collect();
    assert_eq!(
        unimplemented_rows,
        vec![Action::AcceptAiSuggestion, Action::AuditionMain],
        "本版本快捷键表里未实现的条目必须**恰好**是 `Shift+Enter` 与 `[` 这两条"
    );

    // ④ 真二进制的渲染面：条目行上恰好 2 个标记，且两条被点名的行都带标记。
    let run = invoke(&["--print-shortcuts"]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    let printed_marks: usize = run
        .stdout
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .map(|line| line.matches(UNIMPLEMENTED_MARKER).count())
        .sum();
    assert_eq!(
        printed_marks,
        unimplemented_rows.len(),
        "`--print-shortcuts` 的**条目行**里 `{UNIMPLEMENTED_MARKER}` 的次数必须等于未实现条目数:\n{}",
        run.stdout
    );
    assert_eq!(printed_marks, 2, "这两条是本版本仅剩的未实现快捷键");
    for label in ["Shift+Enter → 采纳 AI 建议", "[ → 试听主线"] {
        let line = run
            .stdout
            .lines()
            .find(|line| line.trim_start().starts_with(label))
            .unwrap_or_else(|| panic!("`--print-shortcuts` 里找不到 `{label}`:\n{}", run.stdout));
        assert!(
            line.contains(UNIMPLEMENTED_MARKER),
            "能力仍缺失的行**必须**带 `{UNIMPLEMENTED_MARKER}`: {line}"
        );
    }
    // 反方向：别的行**不许**被顺手标上标记（否则"未实现"这一列就不再可读）。
    for line in run
        .stdout
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
    {
        if line.contains(UNIMPLEMENTED_MARKER) {
            assert!(
                line.contains("Shift+Enter → 采纳 AI 建议") || line.contains("[ → 试听主线"),
                "只有那两条能力缺失的行可以带标记, 实际: {line}"
            );
        }
    }

    // ⑤ 能力缺失的**机械锚点**：`[` / `]` 的规范依据 `[ARCH-RT-005]`（主线与提案分支
    //    并发渲染 + 30ms 等功率瞬切 + 2048 采样预滚）**不在**引擎已实现的规范 ID 清单里。
    //    引擎哪天真把它接上了，这一步变红 ⇒ 走第 1 条的更新路径（不是删断言）。
    assert!(
        !yeban_engine::IMPLEMENTED_SPEC_IDS.contains(&"ARCH-RT-005"),
        "引擎已把 `ARCH-RT-005` 标为已实现 ⇒ 请把 `Action::AuditionMain` / `Action::AuditionProposal` \
         从 `host::action_has_implementation` 的 `!matches!` 移出、把 `cli.rs` 的 `implemented` 改成 `true`，\
         并更新本判据（而不是删掉它）"
    );

    eprintln!(
        "[unimplemented-pins] 未实现快捷键条目 {printed_marks} 条（`Shift+Enter → 采纳 AI 建议` / \
         `[ → 试听主线`）: 宿主对三条动作（含不在表里的 `AuditionProposal`）一律拒绝, \
         键仍绑定, `ARCH-RT-005` 不在引擎的 IMPLEMENTED_SPEC_IDS 里"
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
    let dir = scratch_dir("export-wav-routing");
    let source = write_real_container(&dir, "probe.yeban");
    let out = dir.join("probe.wav");

    let run = invoke(&[
        "--open",
        source.to_str().expect("utf8"),
        "--export-wav",
        out.to_str().expect("utf8"),
    ]);
    assert_eq!(
        run.code, 0,
        "`--export-wav` 单独给必须自己选无窗口路径; stderr={}",
        run.stderr
    );
    assert!(
        run.stdout.lines().any(|line| line == "headless ok"),
        "⇒ 真的走了 `run_batch`（握手行是那条路径的契约）:\n{}",
        run.stdout
    );
    let line = line_with(&run.stdout, "exported-wav:").expect("必须有 exported-wav: 行");
    assert!(
        line.contains("from=") && line.contains("probe.yeban"),
        "来源必须明写: {line}"
    );
    let bytes = std::fs::metadata(&out).expect("WAV 必须真的落盘").len();
    assert!(bytes > 0, "WAV 不能是空文件");
    assert_eq!(
        field(line, "bytes").as_deref(),
        Some(bytes.to_string().as_str()),
        "打印的字节数必须是实际落盘字节数: {line}"
    );

    // 回读: 真的是 WAV，且形状 = 导出契约（48 kHz / 立体声 / 24-bit / 8 秒）。
    let (format, buffer) = yeban_render::wav::read_plain_wav(&out).expect("回读导出的 WAV");
    assert_eq!(format.sample_rate, 48_000);
    assert_eq!(format.channels, 2);
    assert_eq!(format.bits_per_sample, 24);
    assert_eq!(
        buffer.len(),
        384_000 * 2,
        "8 秒 @ 48 kHz 立体声的帧数 × 声道"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 B16: `--export-wav` 的**位深 / 采样率 / 时长 / 精确帧数**可被 CLI 选，
/// 且**缺省兼容**。
///
/// 本条钉四件事（都**回读真文件**，⛔ 不读 stdout 的自报值）：
/// ① 不给 `--wav-*` ⇒ 与旧行为一致（48 kHz / 立体声 / 24-bit / 8 秒）—— 演示工程
///    的 `audio_config.sample_rate` 就是 48 kHz，所以"跟随工程"在缺省档**不改变产物**；
/// ② 显式给三条 ⇒ 产物形状**逐条**跟着变（16-bit / 44.1 kHz / 1 秒）；
///    ②b `--wav-frames` 原样生效（与 MCP `render_master.frames` 同量纲）；
///    ②c `--wav-frames` 与 `--wav-seconds` 互斥（两种顺序都是用法错误）；
/// ③ `--wav-*` 单独给（没有 `--export-wav`）⇒ 用法错误退出码 **2**，
///    ⛔ 不是"静默忽略"（本仓对修饰开关的一贯纪律）。
#[test]
fn export_wav_bit_depth_sample_rate_and_seconds_are_selectable_and_default_stays_compatible() {
    let dir = scratch_dir("export-wav-shape");
    let source = write_real_container(&dir, "shape.yeban");

    // ---- ① 缺省：旧行为（48 kHz / 2ch / 24-bit / 8 秒）----
    let default_out = dir.join("default.wav");
    let run = invoke(&[
        "--open",
        source.to_str().expect("utf8"),
        "--export-wav",
        default_out.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "缺省导出必须成功; stderr={}", run.stderr);
    let (format, buffer) = yeban_render::wav::read_plain_wav(&default_out).expect("回读缺省档");
    assert_eq!(format.sample_rate, 48_000, "缺省采样率 = 工程采样率");
    assert_eq!(format.channels, 2, "缺省声道");
    assert_eq!(format.bits_per_sample, 24, "缺省位深");
    assert!(!format.is_float, "缺省是整数 PCM");
    assert_eq!(
        buffer.len(),
        384_000 * 2,
        "缺省 = 8 秒 @ 48 kHz 立体声的帧数 × 声道"
    );

    // ---- ② 显式三条：16-bit / 44.1 kHz / 1 秒 ----
    let chosen_out = dir.join("chosen.wav");
    let run = invoke(&[
        "--open",
        source.to_str().expect("utf8"),
        "--export-wav",
        chosen_out.to_str().expect("utf8"),
        "--wav-bit-depth",
        "16",
        "--wav-sample-rate",
        "44100",
        "--wav-seconds",
        "1",
    ]);
    assert_eq!(run.code, 0, "显式形状必须成功; stderr={}", run.stderr);
    let (format, buffer) = yeban_render::wav::read_plain_wav(&chosen_out).expect("回读选定档");
    assert_eq!(format.sample_rate, 44_100, "采样率必须真的是请求值");
    assert_eq!(format.bits_per_sample, 16, "位深必须真的是请求值");
    assert!(!format.is_float, "16-bit 是整数 PCM");
    assert_eq!(
        buffer.len(),
        44_100 * 2,
        "1 秒 @ 44.1 kHz 立体声的帧数 × 声道"
    );
    assert!(
        matches!(buffer, yeban_render::dither::PcmBuffer::Int16(_)),
        "容器里的样本必须是 16-bit 整数（⛔ 不是 24-bit 换个头）"
    );
    // 报告行的自报值必须与回读一致。
    let line = line_with(&run.stdout, "exported-wav:").expect("必须有 exported-wav: 行");
    assert_eq!(
        field(line, "sample_rate").as_deref(),
        Some("44100"),
        "{line}"
    );
    assert_eq!(field(line, "bits").as_deref(), Some("16"), "{line}");
    assert_eq!(field(line, "frames").as_deref(), Some("44100"), "{line}");

    // ---- ②b 精确帧数：`--wav-frames` 原样用（⛔ 不再做秒×采样率换算）----
    //
    // 与 MCP `render_master.frames` 同一量纲 ⇒ 两侧的产物长度可以逐字节对齐。
    // 取一个**不是**任何整秒 × 48 kHz 的帧数（12345），钉住"原样"而不是"取整后碰巧相等"。
    let exact_out = dir.join("exact.wav");
    let run = invoke(&[
        "--open",
        source.to_str().expect("utf8"),
        "--export-wav",
        exact_out.to_str().expect("utf8"),
        "--wav-frames",
        "12345",
    ]);
    assert_eq!(run.code, 0, "精确帧数导出必须成功; stderr={}", run.stderr);
    let (format, buffer) = yeban_render::wav::read_plain_wav(&exact_out).expect("回读精确档");
    assert_eq!(format.sample_rate, 48_000, "帧数不改变采样率缺省");
    assert_eq!(format.bits_per_sample, 24, "帧数不改变位深缺省");
    assert_eq!(
        buffer.len(),
        12_345 * 2,
        "产物的**帧数**必须逐帧等于 --wav-frames（× 声道）"
    );
    let line = line_with(&run.stdout, "exported-wav:").expect("必须有 exported-wav: 行");
    assert_eq!(field(line, "frames").as_deref(), Some("12345"), "{line}");

    // ---- ②c `--wav-frames` 与 `--wav-seconds` 互斥（⛔ 不靠"谁优先"发明语义）----
    for pair in [
        ["--wav-frames", "100", "--wav-seconds", "1"],
        ["--wav-seconds", "1", "--wav-frames", "100"],
    ] {
        let run = invoke(&[
            "--headless",
            "--export-wav",
            dir.join("never.wav").to_str().expect("utf8"),
            pair[0],
            pair[1],
            pair[2],
            pair[3],
        ]);
        assert_eq!(
            run.code, 2,
            "两种顺序都必须被拒 (退出码 2); stderr={}",
            run.stderr
        );
        assert!(
            run.stderr.contains("--wav-frames") && run.stderr.contains("--wav-seconds"),
            "错误必须点名两个开关; stderr={}",
            run.stderr
        );
    }

    // ---- ③ 修饰开关单独给 = 用法错误（⛔ 不静默忽略）----
    //
    // ⚠ 必须带一个**选无窗口路径**的开关（这里用 `--headless`）：`--wav-*` 是**修饰**
    // 开关（与 `--record-seconds` / `--midi-in-seconds` 同档），它自己**不**把进程推离
    // GUI 路径 ⇒ 单独给会开窗口并阻塞（本仓已登记的"修饰开关不选路径"口径）。
    // 守卫住在 `run_batch` 里 ⇒ 本判据必须真的走到 `run_batch` 才钉得住它。
    for (flag, value) in [
        ("--wav-bit-depth", "16"),
        ("--wav-sample-rate", "44100"),
        ("--wav-seconds", "1"),
        ("--wav-frames", "100"),
    ] {
        let run = invoke(&["--headless", flag, value]);
        assert_eq!(
            run.code, 2,
            "`{flag}` 单独给（不给 --export-wav）必须是用法错误 (退出码 2); stderr={}",
            run.stderr
        );
        assert!(
            run.stderr.contains(flag) && run.stderr.contains("--export-wav"),
            "错误必须点名两个开关; stderr={}",
            run.stderr
        );
    }

    // ---- ④ 值域护栏：⛔ 不静默接受非法取值 ----
    for (flag, value) in [
        ("--wav-bit-depth", "32"), // 歧义（只有 32f 一种 32 位容器）
        ("--wav-bit-depth", "8"),
        ("--wav-sample-rate", "9600"), // 枚举外
        ("--wav-sample-rate", "abc"),
        ("--wav-seconds", "0"),
        ("--wav-seconds", "-1"),
        ("--wav-frames", "0"), // 空产物
        ("--wav-frames", "-1"),
        ("--wav-frames", "abc"),
        ("--wav-frames", "1.5"),
    ] {
        let run = invoke(&[
            "--headless",
            "--export-wav",
            dir.join("never.wav").to_str().expect("utf8"),
            flag,
            value,
        ]);
        assert_eq!(
            run.code, 2,
            "`{flag} {value}` 必须被拒 (退出码 2); stderr={}",
            run.stderr
        );
    }

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
    // `F0_baseline`（版本码 `0x09CF`），**2 条**用**负责人自己**用 Logic Pro 12.2
    // （2026-10-06）存的两轨工程（版本码 `0x09D0`，sha256
    // `cfeabcfc11c5f001edfb48d5711cbccb57944aa23c22bd926483c704dde36db6`），**3 条及以上**
    // 用负责人同日（2026-10-18）存的三轨工程。⭐ `#194` 把多轨导出的容量 **2 → 3**
    // （`LOGIC_OWNER_DONOR_3T_TRACK_CAPACITY` = 3），演示工程的 **4** 条 MIDI 轨因此从
    // "2 条被映射"变成"**3** 条被映射"，其余 1 条逐条登记为未映射。
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
        "根头 +0x04 必须是**所选供体**落盘的格式版本码 —— 3 轨那份同为 0x09D0"
    );
    assert_eq!(&bytes[0x18..0x1c], b"gnoS", "第一个 chunk 名必须是 gnoS");
    let declared =
        u32::from_le_bytes([bytes[0x10], bytes[0x11], bytes[0x12], bytes[0x13]]) as usize;
    assert_eq!(declared, bytes.len() - 0x18, "声明载荷长度 @0x10");

    // ②b 它真的**带着供体的通道簇**：按 36 字节记录头走完全文，数四个家族。
    //     自研写入器这四族一个都没有（`Envi`/`AuCO`/`GenM`/`Trak`），这正是它被判为
    //     "没有轨道"的原因；供体路线把它们原样带进来。读数 = 负责人 3 轨那份的实测逐族表。
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
    // ⭐ `#199`（落实 `#197` 的建议①）：记录数**不再抄字面量**，改读产出它的那个 crate 的
    //    公开常量 `yeban_render::logic::LOGIC_OWNER_DONOR_3T_RECORDS`（3 轨供体 = 515）。
    //    判据本身仍是真判据：「**产物**里走出来的记录条数 == **供体**声明的记录条数」——
    //    常量被改错 ⇒ 这里红（见 `#199` 的注入读数），所以 ⛔ 不是恒真；
    //    未来容量 3 → 4 时 render 侧随新供体改常量，本判据**自动跟随**。
    assert_eq!(
        records,
        yeban_render::logic::LOGIC_OWNER_DONOR_3T_RECORDS,
        "产物记录条数必须等于 3 轨供体声明的记录条数（`#199`：读 render 的公开常量，\
         不再抄 515；`#194` 把容量 2 提到 3 时 2 轨的 507 + 8 = 515）"
    );
    // 落盘字节是**可读名的反序**（`Envi` 落盘为 `ivnE`），与 `yeban_render::logic` 的记录模型一致。
    // ⭐ 五个族条数同样改读 render 的公开逐族表（`LOGIC_OWNER_DONOR_3T_CHUNK_FAMILIES`），
    //    经 `donor_family_count` 反序查表 ⇒ 容量变化时自动跟随，且常量被改错仍会红。
    assert_eq!(
        families.get(b"ivnE").copied(),
        Some(donor_family_count(b"ivnE")),
        "ivnE 环境对象条数必须是供体逐族表里的值（`#199` 读 render 常量；`#194`：2 轨 45 → 3 轨 46）"
    );
    assert_eq!(
        families.get(b"OCuA").copied(),
        Some(donor_family_count(b"OCuA")),
        "OCuA 混音条必须来自供体"
    );
    assert_eq!(
        families.get(b"MneG").copied(),
        Some(donor_family_count(b"MneG")),
        "MneG 必须来自供体"
    );
    assert_eq!(
        families.get(b"karT").copied(),
        Some(donor_family_count(b"karT")),
        "karT 轨道家族必须来自供体（`#194`：2 轨 31 → 3 轨 33，`Trak +2`）"
    );
    assert_eq!(
        families.get(b"UCuA").copied(),
        Some(donor_family_count(b"UCuA")),
        "UCuA 通道条状态必须来自供体（`#194`: 2 轨 19 → 3 轨 22，`AuCU +3`；\
         2 轨那份的 19 含第二条轨道新激活的 8 条，第 3 条 `control track` 只多 3 条）"
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
    let source = source.to_str().expect("utf8");

    // ① 缺后缀：`hello` ⇒ `hello.logicx`。
    let bare = dir.join("hello");
    let run = invoke(&[
        "--open",
        source,
        "--export-logic",
        bare.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    let line = line_with(&run.stdout, "exported-logic:").expect("exported-logic: 行");
    let actual = field(line, "path").expect("path=");
    assert!(
        actual.ends_with(".logicx"),
        "回执的 path= 必须以 .logicx 结尾（`open -a` 靠它认 bundle）: {line}"
    );
    assert_eq!(
        actual,
        format!("{}.logicx", bare.display()),
        "实际路径必须恰好是 `<给的目录>.logicx`: {line}"
    );
    assert!(!bare.exists(), "⛔ 无后缀的那个名字上不许留任何东西");
    assert!(
        Path::new(&actual)
            .join("Alternatives/000/ProjectData")
            .is_file(),
        "产物必须真的落在补过后缀的目录里: {actual}"
    );
    assert_eq!(field(line, "files").as_deref(), Some("4"), "{line}");

    // ② 已带后缀：`foo.logicx` ⇒ 路径**不变**，不重复加。
    let named = dir.join("foo.logicx");
    let again = invoke(&[
        "--open",
        source,
        "--export-logic",
        named.to_str().expect("utf8"),
    ]);
    assert_eq!(again.code, 0, "stderr={}", again.stderr);
    let line = line_with(&again.stdout, "exported-logic:").expect("exported-logic: 行");
    assert_eq!(
        field(line, "path"),
        Some(named.display().to_string()),
        "已带后缀的路径必须**原样**落盘: {line}"
    );
    assert!(
        !dir.join("foo.logicx.logicx").exists(),
        "⛔ 不许出现 `foo.logicx.logicx`"
    );

    let _ = std::fs::remove_dir_all(&dir);
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
    let dir = scratch_dir("export-logic-timbre");
    let source = write_real_container(&dir, "song.yeban");
    let target = dir.join("timbre.logicx");
    let run = invoke(&[
        "--open",
        source.to_str().expect("utf8"),
        "--export-logic",
        target.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);

    for fragment in [
        "Studio Grand",
        "Epic Cloud Formation",
        "供体",
        "不是用户工程的设备链",
    ] {
        assert!(
            run.stdout.contains(fragment),
            "损失表文本必须逐字含 `{fragment}`; stdout={}",
            run.stdout
        );
    }
    // 且这四个片段必须落在**同一条** `logic-loss:` 行上（否则"同时出现"只是碰巧）。
    let line = run
        .stdout
        .lines()
        .find(|text| {
            text.starts_with("logic-loss: ")
                && text.contains("Studio Grand")
                && text.contains("Epic Cloud Formation")
        })
        .expect("打开范围那条 logic-loss: 行必须同时点名两条音色");
    assert!(
        line.contains("供体") && line.contains("不是用户工程的设备链"),
        "音色与设备链缺口必须写在同一行: {line}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
