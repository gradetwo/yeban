//! `[BASELINE-003]` 的**判决读数派遣器**：把 `examples/fps_gpu.rs` 跑起来，
//! 并**机械校验**它量的是 GPU 路径（不是软件光栅化）。
//!
//! ## 为什么判据在 example 里、而这里只是"派遣"
//!
//! 本档要量的渲染器就是产品 GUI 用的那条：`winit` + `FemtoVG`（OpenGL/glutin）。
//! winit 0.30 的事件循环**只能在进程主线程上创建**（本机实测原话：
//! `on macOS, EventLoop must be created on the main thread!`，
//! `winit-0.30.13/src/platform_impl/macos/event_loop.rs:221`），而 `cargo test` 的用例
//! 一律跑在**测试线程**上 —— 所以 GPU 档不可能是 `#[test]` 函数体（"测试里再 fork 一个
//! 测试二进制"也不行：那是同一个 libtest，测试体依旧在测试线程上；本机两次实测过）。
//! 因此真的测量住在 `examples/fps_gpu.rs`（`main()` 就是主线程），
//! 本文件只负责：拉起它、钉住它的入口/口径、**断言它不是软件路径**。
//!
//! ## 为什么这条测试不是"通过即达标"
//!
//! 它**只**断言"读数来自 GPU 路径"这一件事（入口、声明行、窗口见证、退出码）。
//! 帧时数字本身是否 ≤ 8.3 ms **不由它判定** —— 那要人来读数字，
//! 因此本文件在结论里**不出现"通过"字样**，只出现"已测"。这与 `[MUST-GATE-015]`
//! 的分平台 Golden 那条"未被判定 ≠ 通过"是同一条纪律。
//!
//! ## 运行
//!
//! ```text
//! cargo test -p yeban-app --locked --test fps_gpu_path -- --ignored --nocapture
//! ```

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// `[BASELINE-003]` GPU 档的读数行前缀（与 `examples/fps_gpu.rs` 同款）。
const LINE_PREFIX: &str = "BASELINE-003";
/// 派遣器的硬上限：超时**杀掉子进程**并判失败（不留窗口、不留进程）。
const CHILD_TIMEOUT: Duration = Duration::from_secs(600);

/// 报告一行到**进程级 stderr**（与 `yeban-ui-test-port::report_line` 同款理由：
/// 通过的测试也会把它留在 CI 日志里）。本目标**不引入** `yeban-ui-test-port`
/// （那是 `[MUST-GATE-015]` 的 Tier-1 依赖），所以这里只做最小实现：
/// 先 `eprintln!`，再往 `/dev/stderr` 写一份（后者不经 std 的输出捕获）。
fn shout(line: &str) {
    eprintln!("{line}");
    if let Ok(mut fd) = std::fs::OpenOptions::new().write(true).open("/dev/stderr") {
        use std::io::Write as _;
        let _ = fd.write_all(line.as_bytes());
        let _ = fd.write_all(b"\n");
    }
}

/// 子进程输出（stdout/stderr 分开留，失败时两段都打出来）。
struct ChildRun {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

impl ChildRun {
    fn combined(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

/// 拉起 `cargo run --release -p yeban-app --locked --example fps_gpu` 并等它自己结束。
///
/// **为什么用 `cargo run` 而不是直接执行 `target/.../examples/fps_gpu`**：
/// 判决命令必须能被逐字复跑，而 `cargo run` 自己保证"编译到最新 + 跑到"，不依赖
/// 调用者先在别处构建过一次。子进程有硬超时，超时即 `kill` 后判失败。
fn run_gpu_example() -> Result<ChildRun, String> {
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO"))
        .args([
            "run",
            "--release",
            "-p",
            "yeban-app",
            "--locked",
            "--example",
            "fps_gpu",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("无法拉起 GPU 档 (`cargo run --example fps_gpu`): {err}"))?;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(err) => return Err(format!("等待 GPU 档子进程失败: {err}")),
        }
        if started.elapsed() > CHILD_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "GPU 档子进程超过 {:?} 未结束 —— 已杀掉（不留窗口/不留进程）",
                CHILD_TIMEOUT
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    let output = child
        .wait_with_output()
        .map_err(|err| format!("收集 GPU 档子进程输出失败: {err}"))?;
    Ok(ChildRun {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        elapsed: started.elapsed(),
    })
}

/// `[BASELINE-003]`：**判决读数必须来自 GPU 路径**（`winit` + `FemtoVG`/OpenGL）。
///
/// # Panics
/// 子进程超时/非零退出，或输出里缺少"这是 GPU 路径"的任何一条证据。
#[test]
#[ignore = "会真的开一个 winit 窗口并在事件循环里跑 600 帧; 由手动档 fps 用 --ignored 拉起"]
fn baseline_003_reading_comes_from_the_gpu_path() {
    // ---- ① 入口 PIN：GPU 档必须住在 example 里（`main()` = 主线程），不许被搬回 `#[test]` ----
    //
    // 为什么把"入口在哪"也做成判据：本档最容易被后人"顺手"改回 `#[test]`，
    // 而那样它会在 winit 的 `must be created on the main thread` 上失败 —— 或者更糟，
    // 有人为了让它跑起来把渲染器换成软件路径，于是门禁静默量错东西。
    let example = include_str!("../examples/fps_gpu.rs");
    for needle in [
        "let requested_backend = \"winit\";",
        "let requested_renderer = \"femtovg\";",
        ".backend_name(requested_backend.to_string())",
        ".renderer_name(requested_renderer.to_string())",
        ".require_opengl()",
        "remove_var(\"SLINT_BACKEND\")",
        "fn main()",
    ] {
        assert!(
            example.contains(needle),
            "GPU 档的三个闸被改掉了: `examples/fps_gpu.rs` 里找不到 `{needle}` —— \
             回退到软件光栅化会变成静默的, 本判据拒绝"
        );
    }
    // 反向 PIN：不许引入"未设置就用默认后端"这种静默行为。
    assert!(
        !example.contains("SLINT_BACKEND\").is_err()"),
        "`examples/fps_gpu.rs` 不许把 SLINT_BACKEND 当事实源（它必须是全局的、被摘掉的那个）"
    );

    // ---- ② 真跑，并机械校验输出 ----
    let run = match run_gpu_example() {
        Ok(run) => run,
        Err(reason) => panic!("{reason}"),
    };
    // 判据行**逐行透传**到进程 stderr（CI 日志里能看到数字，不必翻 artifact）。
    for line in run.combined().lines().filter(|l| l.contains(LINE_PREFIX)) {
        shout(line);
    }

    let combined = run.combined();
    let expect_path = "路径=GPU(winit+FemtoVG/OpenGL)";
    let missing: Vec<&str> = [
        "请求 backend_name=winit renderer_name=femtovg require_opengl=true 已接受=true",
        "require_opengl=true",
        expect_path,
        "结果=已测",
        "见证窗口已映射=true",
        "见证字符数下限=100000",
    ]
    .into_iter()
    .filter(|needle| !combined.contains(needle))
    .collect();
    assert!(
        missing.is_empty(),
        "GPU 档没有给出「读数来自 GPU 路径」的证据 (缺 {missing:?}) —— 退出码 {:?}, 耗时 {:?}\n\
         ==== 子进程 stdout ====\n{}\n==== 子进程 stderr ====\n{}",
        run.status.code(),
        run.elapsed,
        run.stdout,
        run.stderr
    );
    // 显式拒绝"非零退出但输出里有数字"这种自相矛盾的读数。
    assert!(
        run.status.success(),
        "GPU 档子进程非零退出 ({:?}) —— 本判据记失败, **绝不**回退到软件读数\n\
         ==== 子进程 stdout ====\n{}\n==== 子进程 stderr ====\n{}",
        run.status.code(),
        run.stdout,
        run.stderr
    );
    // 反向证据：**不许**在这一次读数里同时出现软件路径的判决行（那是口径漂移的信号）。
    assert!(
        !combined.contains("路径=软件光栅化"),
        "这一次读数里同时出现了软件路径 —— 门禁的行读数必须是唯一的 GPU 档\n{}",
        run.combined()
    );

    shout(&format!(
        "{LINE_PREFIX}(GPU档) 派遣器结论: 读数**已测**(入口=examples/fps_gpu.rs, 退出码=0, 耗时={:?}); \
         帧时数字是否 ≤ 8.3ms 由人读上面那行判定 —— 本条判据**不**宣布门禁通过",
        run.elapsed
    ));
}
