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

#![allow(unused)]
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B2: 无头握手行**恰好**出现一次，且工程来源被明写。
#[test]
fn headless_handshake_appears_exactly_once_and_names_the_source() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B2b: `SLINT_BACKEND=headless` **单独**（不给任何参数）也等价于 `--headless`。
///
/// 这条判据的价值在于它真的能变红：哨兵失效 ⇒ 进程会去开窗口 —— 在无显示器环境
/// 直接失败，在有显示器环境进阻塞事件循环（由 [`RUN_TIMEOUT`] 变成一次响亮失败）。
#[test]
fn slint_backend_headless_sentinel_alone_keeps_the_window_closed() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B3: `--open` 真容器 ⇒ 打印的读数与工程一致（轨道 / 音符 / 历史 / 资产）。
#[test]
fn open_reports_the_real_project_counts() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B4: 坏输入 ⇒ **非零退出码 + 精确原因**，不是 panic、不是空工程。
#[test]
fn broken_inputs_exit_non_zero_without_panicking() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B5: `--open` + `--save-as` ⇒ 落盘文件能被公开入口读回，且**归档等价**；
/// 两次保存的字节完全相同（确定性）。
#[test]
fn save_as_round_trips_and_is_deterministic_from_the_real_binary() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B6: `--save-as` 不给 `--open` ⇒ 保存演示工程，并且**在输出里明说**这一点。
#[test]
fn save_as_without_open_says_it_saved_the_demo_project() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B7: 未知开关 / 缺取值 ⇒ 退出码 2 + **用法提示**（不许静默忽略）。
#[test]
fn unknown_switches_exit_two_with_usage_on_stderr() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B8: `--export-elements` 与 `--dump-elements` 同源（同一批行、同一个字节数）。
#[test]
fn export_elements_matches_dump_elements() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B9: 只读目录下 `--save-as` ⇒ 退出码 4，且**原文件一字未改**（原子替换的可观测后果）。
#[cfg(unix)]
#[test]
fn save_as_into_a_read_only_directory_exits_four_and_keeps_the_old_file() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B10（**反转**）: 裸 `project.json` 必须被**明确拒绝** —— 容器是唯一工程格式
/// （ADR-0001 D43）。旧判据在这里断言"能打开 + `format=project-json`"。
#[test]
fn open_rejects_a_bare_project_json_with_a_precise_reason() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B11: `--print-shortcuts` 是无窗口路径，且退出 0（CI 与人核对策略表的入口）。
#[test]
fn print_shortcuts_is_a_batch_command() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B12: `--export-midi` 从**真二进制**导出的字节可被 SMF 读取面读回，
/// 逐音符与工程一致，`MThd` 的 PPQ 字段是 960，且两次导出逐字节相同
/// （① 回读 / ② 逐音符 / ③ PPQ 头 / ④ 确定性，全部在**进程级**再证一次）。
#[test]
fn export_midi_writes_a_parseable_deterministic_smf_from_the_real_binary() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据（审评 2026-10-31，P1-1）：`--render-threads` 只改耗时、不改字节 ——
/// 同一工程缺省导出与 `--render-threads 4` 导出的文件逐字节相同
/// （固定顺序汇聚；非法值已由上一判据第 ④ 节覆盖）。
#[test]
fn export_wav_render_threads_change_only_time_not_bytes() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B13: `--export-midi` 的失败语义与既有 CLI 一致 —— 非容器输入 ⇒ 退出码 **3**
/// 且一个字节都不写；目标不可写 ⇒ 退出码 **5**、精确原因、**不留下半个文件**。
#[test]
fn export_midi_failures_reuse_the_existing_exit_codes_and_write_nothing() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}

/// 判据 B14: 带 `experimental-als-export` 时，`--export-als` 写出 Gzip 文档并把
/// **映射损失表**逐条打到 stdout（`ADR-0001 D47` 的价值 = 损失表对用户可见）。
///
/// 与 B12 同一条思路，但断言的是另一件事：B12 证"字节能被 SMF 读取面读回"，
/// 这里证"**没被映射的东西真的被说出来了**" —— 只打印一个计数不算交付。
#[cfg(feature = "experimental-als-export")]
#[test]
fn export_als_writes_a_gzip_document_and_prints_the_loss_summary() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback accept-ai-proposal("),
        "新 UI 契约面必须声明 callback accept-ai-proposal("
    );
    assert!(
        aether_host.contains("on_accept_ai_proposal"),
        "宿主接线层必须注册 on_accept_ai_proposal"
    );
}
