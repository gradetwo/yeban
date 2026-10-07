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
//! 它**只**断言"读数来自 GPU 路径"以及"正式口径量的就是绘制回调"这两件事
//! （入口、声明行、闸门 PIN、绘制回调探针 PIN、窗口/API 见证、退出码）。
//! 绘制回调的 p99 是否 ≤ 2 ms **不由它判定** —— 那要人来读数字，
//! 因此本文件在结论里**不出现"通过"字样**，只出现"已测"。这与 `[MUST-GATE-015]`
//! 的分平台 Golden 那条"未被判定 ≠ 通过"是同一条纪律。
//!
//! ## 正式门限是规格自己的数（`[UI-NOTE-001]` 步骤 ④ 的 ≤ 2 ms）
//!
//! 2026-10-07 的负责人裁决把 `BASELINE-003` 的**正式口径**定为"绘制回调耗时（不含呈现）"，
//! 门限取规格原文的 **2 ms**；墙钟帧间隔（旧的 8.3 ms 观测对象）降级为**环境记录**。
//! ⛔ 这不是放松：2 ms 是规格里更紧的那个数，因此本文件用 PIN 把 `DRAW_BUDGET_MS = 2.0`
//! 钉死 —— 谁把它改成更松的数，这条判据就红。
//!
//! ## 运行
//!
//! ```text
//! cargo test -p yeban-app --locked --test fps_gpu_path                          # 文字契约（不开窗口）
//! cargo test -p yeban-app --locked --test fps_gpu_path -- --ignored --nocapture # 真跑 600 帧
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
/// 仪器 PIN 表：`(为什么重要, examples/fps_gpu.rs 里必须逐字出现的串)`。
///
/// 这张表由**两条**判据共用：无窗口的那条（常规 `cargo test` 就会跑）与开窗口的那条
/// （只在手动档 `fps` 里用 `--ignored` 拉起）。共用的理由是个实测教训：这些 PIN 原先
/// 只住在 `#[ignore]` 的用例里 ⇒ 常规测试**从来不执行**它们，PIN 烂掉没有任何人知道。
const INSTRUMENT_PINS: &[(&str, &str)] = &[
    (
        "闸 ①（显式选择）的后端名",
        "let requested_backend = \"winit\";",
    ),
    (
        "闸 ①（显式选择）的渲染器名",
        "let requested_renderer = \"femtovg\";",
    ),
    (
        "闸 ① 把**请求值**绑进调用点（否则打印的\"声明\"不是请求值的反射）",
        ".backend_name(requested_backend.to_string())",
    ),
    (
        "闸 ① 把**请求值**绑进调用点（同上）",
        ".renderer_name(requested_renderer.to_string())",
    ),
    (
        "闸 ① `require_opengl`（选不到 GL 路径只会失败, 不回退）",
        ".require_opengl()",
    ),
    (
        "闸 ② 不订阅全局 `SLINT_BACKEND`",
        "remove_var(\"SLINT_BACKEND\")",
    ),
    ("入口仍在**真主线程**上（example 的 `main()`）", "fn main()"),
    // ---- 正式口径 PIN（2026-10-07 负责人裁决，见 ADR-0002 的「裁决」一节）----
    (
        "正式门限**逐字**是规格自己的 2 ms（不是别的、更松的数）",
        "const DRAW_BUDGET_MS: f64 = 2.0;",
    ),
    (
        "正式读数来自**绘制回调**（而不是墙钟帧周期）",
        "set_rendering_notifier",
    ),
    (
        "绘制回调区间的起点",
        "slint::RenderingState::BeforeRendering",
    ),
    (
        "绘制回调区间的终点（上游语义 = 后备缓冲尚未送显 ⇒ 不含呈现）",
        "slint::RenderingState::AfterRendering",
    ),
    (
        "墙钟帧周期必须被**明确标注为环境**（非门限）",
        "环境(非门限)",
    ),
];

/// 断言仪器 PIN 全部成立（纯文字契约，不开窗口）。
///
/// # Panics
/// 任一 PIN 缺失，或出现了"未设置就用默认后端"这种静默行为。
fn assert_instrument_pins(example: &str) {
    for (why, needle) in INSTRUMENT_PINS {
        assert!(
            example.contains(*needle),
            "GPU 档的{why}被改掉了: `examples/fps_gpu.rs` 里找不到 `{needle}` —— \
             静默回退到软件光栅化、或把正式口径改回墙钟帧周期, 都会变成静默的; 本判据拒绝"
        );
    }
    // 反向 PIN：不许引入"未设置就用默认后端"这种静默行为。
    assert!(
        !example.contains("SLINT_BACKEND\").is_err()"),
        "`examples/fps_gpu.rs` 不许把 SLINT_BACKEND 当事实源（它必须是全局的、被摘掉的那个）"
    );
}

/// `[BASELINE-003]` 的**文字契约**：仪器必须仍是"GPU 路径 + 绘制回调 ≤2ms + 墙钟标注为环境"。
///
/// 为什么单独一条：真正出数字的那条要开 winit 窗口，只能在手动档 `fps` 里跑；若把文字契约
/// 只放在它里面，常规 `cargo test` 就**永远不执行**这些 PIN。本条不需要窗口、不需要 GPU、
/// 不产生任何数字，因此能在每次 `cargo test -p yeban-app --tests` 里跑。
#[test]
fn baseline_003_instrument_pins_hold_without_a_window() {
    assert_instrument_pins(include_str!("../examples/fps_gpu.rs"));
}

#[test]
#[ignore = "会真的开一个 winit 窗口并在事件循环里跑 600 帧; 由手动档 fps 用 --ignored 拉起"]
fn baseline_003_reading_comes_from_the_gpu_path() {
    // ---- ① 入口 PIN：GPU 档必须住在 example 里（`main()` = 主线程），不许被搬回 `#[test]` ----
    //
    // 为什么把"入口在哪"也做成判据：本档最容易被后人"顺手"改回 `#[test]`，
    // 而那样它会在 winit 的 `must be created on the main thread` 上失败 —— 或者更糟，
    // 有人为了让它跑起来把渲染器换成软件路径，于是门禁静默量错东西。
    let example = include_str!("../examples/fps_gpu.rs");
    assert_instrument_pins(example);

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
        // 正式口径的证据：绘制回调行 + 运行期图元 API 见证 + 配对见证 + 环境行的显式标注。
        "绘制回调(正式门限 p99<=2ms, 不含呈现)",
        "见证绘制API=NativeOpenGL",
        "见证配对失衡=0",
        "环境(非门限): 墙钟帧间隔",
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
         正式门限 = 绘制回调 p99 ≤ 2ms(`[UI-NOTE-001]` 步骤 ④)是否达标由人读上面那行判定, \
         墙钟帧间隔只是环境读数 —— 本条判据**不**宣布门禁通过",
        run.elapsed
    ));
}
