//! `[BASELINE-003]` 的 **GPU 路径**帧时测量 —— 10 万音符滚动，600 帧。
//!
//! ## 为什么必须是一条独立的可执行文件（而不是 `#[test]`）
//!
//! 本档要量的渲染器就是产品 GUI 用的那条路径：`winit` + `FemtoVG`（OpenGL/glutin）。
//! 而 winit 0.30 的事件循环**只能在进程的主线程上创建**（本机实测的原话：
//! `on macOS, EventLoop must be created on the main thread!`，
//! `winit-0.30.13/src/platform_impl/macos/event_loop.rs:221`）。`cargo test` 的每个用例
//! 跑在**测试线程**上，所以 GPU 档**不可能**是一个 `#[test]` 函数体 ——
//! 连"测试里再 fork 一个测试二进制"也不行（那是同一个 libtest，测试体依旧在测试线程上；
//! 本机实测过，失败信息与上面逐字相同）。
//!
//! 因此本档是一个 **example 目标**（`examples/` 是 cargo 的自动发现目录，不需要改
//! `Cargo.toml`，也不需要动 `main.rs`/`lib.rs`），`main()` 就是真主线程。
//! 它由 `crates/yeban-app/tests/fps_gpu_path.rs` 拉起并**机械校验**输出，
//! 由 `.github/workflows/gates-manual.yml` 的 `fps` 档做判决。
//!
//! ## 口径（与规格逐条对齐）
//!
//! - 场景：`yeban_model::samples::project_with_notes(100_000)`（**恰好** 10 万，不是近似）；
//! - 尺寸：1920×1080（与 Tier-1 软件档、与 Golden 同一尺寸）；
//! - 每帧推进滚动 1/120 秒 ⇒ 1 屏/秒（`docs/DEVELOPMENT_LEDGER.md` 第 136 轮的规格，
//!   与软件档同款换算：`scroll_x = frame * (viewport_width / 120)`）；
//! - 计时区间 = **帧回调之间的真实间隔**，含：注入投影 + `request_redraw` +
//!   winit 事件循环调度 + **GPU 渲染** + 交换/垂直同步。这正是 UI 线程每帧承担的墙钟成本。
//! - 量法：`slint::Timer::start(TimerMode::Repeated, 8_333µs, ...)` —— 目标节奏就是
//!   120 Hz 的 1/120 秒，所以"p99 > 8.333 ms"与"丢帧"是同一个意思。
//!
//! ## 三道闸：**静默回退到软件光栅化**是最坏结果
//!
//! 1. **显式选择 + 必失败语义**：`BackendSelector::new().backend_name("winit")
//!    .renderer_name("femtovg").require_opengl().select()?`。上游语义：
//!    渲染器名非空但不可用 ⇒ `Err`（只有 `allow_fallback` 才回退，而
//!    `BackendBuilder::build()` 把它置为 `false`；
//!    `i-slint-backend-winit-1.18.1/lib.rs:1308-1316`），且 `require_opengl` 让
//!    FemtoVG 之外的渲染器在 `:1238-1240` 就被拒。⇒ 选不到 GL 路径**只会失败**。
//! 2. **不订阅 `SLINT_BACKEND`**：它是全局的，一旦外部设成 `winit-software`，
//!    量出来的就是软件路径而输出里看不出来。本档在**最开头** `remove_var` 掉它
//!    （`BackendSelector` 只在自身字段为空时才读它，
//!    `i-slint-backend-selector-1.18.1/api.rs:278-289`）。
//! 3. **平台槽位探针**：`set_platform` 的语义是"槽位空着才成功"
//!    （`i-slint-core-1.18.1/platform.rs:261-263` 返回 `Err(AlreadySet)`）。选择成功之后
//!    再装一次（软件档同款的 `MinimalSoftwareWindow`）**必须失败**；若它成功了，
//!    说明选中的后端没有占住槽位 ⇒ 直接判失败，绝不出数字。
//!
//! ## 见证（"真的渲染了"而不是"窗口存在"）
//!
//! 本档**不用** `take_snapshot()`：FemtoVG 的 OpenGL 后端在 draw 请求之外没有 canvas，
//! `take_snapshot()` 会返回 `take_snapshot is not supported by this FemtoVG backend`
//! （本机实测；上游 `i-slint-renderer-femtovg-1.18.1/opengl.rs:321-333`
//! 的 `let canvas = canvas?;`）。改成三条**可判定**的见证：
//! ① 选中的后端名字与 `require_opengl`（打印在 `声明:` 行，父进程逐字断言）；
//! ② 窗口真的被映射（`is_visible()`，只在事件循环里成立）；
//! ③ **帧真的在走**：`intervals` 采满 600 个、且**最小值**不是 0（0 意味着回调在
//!    没有帧的情况下空转）。
//!
//! ## 退出码（父进程按码判定，不看措辞）
//!
//! | 码 | 含义 |
//! | :--- | :--- |
//! | 0 | 采满帧数并打印了读数 |
//! | 1 | 后端选择失败 / 平台槽位探针异常（**绝不回退**） |
//! | 2 | 窗口、事件循环或帧推进异常 |
//! | 3 | 看门狗超时（不留窗口、不留进程） |
//!
//! 运行（判决命令，手动档 `fps` 用的就是它）：
//!
//! ```text
//! cargo run --release -p yeban-app --locked --example fps_gpu
//! ```
//!
//! `--locked` 与 `-p` 是刻意的：判决读数必须能被逐字复跑。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::ComponentHandle as _;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;

/// 帧数（规格：账本第 136 轮要求 600 帧，每帧推进 1/120 秒）。
const FRAMES: usize = 600;
/// 目标节奏：120 Hz 的 1/120 秒（微秒）。门限 p99 ≤ 8.3 ms 就是它。
const FRAME_INTERVAL_US: u64 = 8_333;
/// 看门狗：到这个墙钟还没跑完就判失败退出（不留窗口、不留进程）。
const WATCHDOG: std::time::Duration = std::time::Duration::from_secs(180);
/// 读数行前缀（父进程与 workflow 都按它 grep）。
const LINE_PREFIX: &str = "BASELINE-003(10万音符)";

fn main() {
    let exit_code = run();
    std::process::exit(exit_code);
}

/// 报告一行（**直接写进程 stderr**，不经任何缓冲层）。
fn say(line: &str) {
    eprintln!("{line}");
}

/// 机器状态：load average（macOS: `sysctl -n vm.loadavg`，形如 `{ 2.31 2.10 1.98 }`）。
///
/// 噪声必须可读：`BASELINE-001/002/004` 的参考机读数都连着 load 一起记
/// （见 `docs/ledger/gate-status.md` 那几行的"诚实边界"）。取不到就如实写 `unknown`。
fn load_average() -> String {
    match std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Ok(out) => format!("unknown (sysctl 退出码 {:?})", out.status.code()),
        Err(err) => format!("unknown ({err})"),
    }
}

/// 选择 GPU 路径（闸 ①②③）。
///
/// # Errors
/// `BackendSelector` 选不出来（⇒ **绝不回退**），或槽位探针发现选中的后端其实没生效。
fn select_gpu_path() -> Result<(), String> {
    // 闸 ②：环境变量是全局的，且会影响 `BackendSelector` 的字段默认值 ⇒ 先摘掉。
    // SAFETY: 本函数只在 `main()` 最开始调用（此时还没有别的线程读环境变量）。
    unsafe { std::env::remove_var("SLINT_BACKEND") };

    // 闸 ①：显式选择，且不允许回退（渲染器名非空 + 不可用 ⇒ Err）。
    //
    // ⚠️ 这里**必须**用下面这两个变量（而不是在调用点再写字面量）：打印出去的"声明"行
    // 因此是**请求值的反射**。2026-10-07 的注入实验证明了为什么：把调用点的字面量改成
    // `software` 而 `say()` 里仍写死 `femtovg` 时，输出会**声称**自己选了 GPU 路径 ——
    // 那种"证据"是假的。本函数把两处绑在一起，并且本文件的自己那两条 PIN（见
    // `tests/fps_gpu_path.rs`）钉住这两个变量名。
    let requested_backend = "winit";
    let requested_renderer = "femtovg";
    slint::BackendSelector::new()
        .backend_name(requested_backend.to_string())
        .renderer_name(requested_renderer.to_string())
        .require_opengl()
        .select()
        .map_err(|err| {
            format!("必失败语义: {requested_backend}+{requested_renderer}+OpenGL 选不出来 => {err}")
        })?;

    // 闸 ③：槽位探针。选择成功之后再装一次**必须失败**（`set_platform` 只在槽位空着时
    // 成功）。探针用软件档同款的 `MinimalSoftwareWindow`：它成功 = 槽位真的空着
    // = 选中的后端没生效 ⇒ 判失败，绝不出数字。
    let probe_window: Rc<dyn slint::platform::WindowAdapter> =
        slint::platform::software_renderer::MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::NewBuffer,
        );
    let probe = Box::new(ProbePlatform {
        window: probe_window,
    });
    if slint::platform::set_platform(probe).is_ok() {
        return Err(
            "平台槽位探针异常: 选择 GPU 路径之后槽位仍是空的 —— 拒绝在错误的渲染路径上出数字"
                .to_string(),
        );
    }
    Ok(())
}

/// 只用来回答"这个线程的 Slint 平台槽位还空着吗"的探针平台。
struct ProbePlatform {
    window: Rc<dyn slint::platform::WindowAdapter>,
}

impl slint::platform::Platform for ProbePlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

fn run() -> i32 {
    say(&format!(
        "{LINE_PREFIX} (GPU档) 环境: 目标间隔={FRAME_INTERVAL_US}us 帧数={FRAMES} load_average={}",
        load_average()
    ));
    if let Err(reason) = select_gpu_path() {
        say(&format!(
            "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因={reason}"
        ));
        return 1;
    }

    // 读数行之前的"口径行"：**请求值** + 已知的残余不确定性（不假装能查询运行期渲染器）。
    say(&format!(
        "{LINE_PREFIX} (GPU档) 口径: 请求 backend_name=winit renderer_name=femtovg require_opengl=true 已接受=true 说明=Slint 1.18.1 公开 API 无法回读运行期渲染器名(已查证), 因此本条证据是\"显式选择成功\"+\"槽位探针\", 不是运行期查询"
    ));

    let project = yeban_model::samples::project_with_notes(100_000);
    let project_view = match ViewState::from_project(&project) {
        Ok(view) => view,
        Err(err) => {
            say(&format!(
                "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=10 万音符工程无法投影: {err}"
            ));
            return 2;
        }
    };
    let scene = DemoScene::from_view(&project_view);
    let viewport_width = scene.viewport_width as f32;
    let note_count: usize = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(std::collections::BTreeMap::len)
        .sum();

    let ui = match host::build_main_window(&project_view, &scene) {
        Ok(ui) => ui,
        Err(err) => {
            say(&format!(
                "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=主窗口构造失败: {err}"
            ));
            return 2;
        }
    };
    slint::ComponentHandle::window(&ui).set_size(slint::PhysicalSize::new(
        scene.viewport_width,
        scene.viewport_height,
    ));
    if let Err(err) = ui.show() {
        say(&format!(
            "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=show() 失败: {err}"
        ));
        return 2;
    }

    say(&format!(
        "{LINE_PREFIX} (GPU档) 场景: 音符={note_count} 尺寸={}x{} 帧数={FRAMES} 每帧滚动={:.3}px (1 屏/秒)",
        scene.viewport_width,
        scene.viewport_height,
        viewport_width / 120.0
    ));

    // 看门狗：vsync 不来的机器上宁可**失败并说清楚**，也不留一个挂着的窗口/进程。
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        say(&format!(
            "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=超时 看门狗={}s —— 事件循环没有在预算内跑完, 本档记失败(不出数字)",
            WATCHDOG.as_secs()
        ));
        std::process::exit(3);
    });

    let frame = Rc::new(Cell::new(0usize));
    let last_tick = Rc::new(Cell::new(std::time::Instant::now()));
    let intervals_ms = Rc::new(RefCell::new(Vec::<f64>::with_capacity(FRAMES)));
    let visible_seen = Rc::new(Cell::new(false));
    let warmup_ms = Rc::new(Cell::new(0.0_f64));

    // 定时器句柄必须活到事件循环结束：drop 掉它等于取消回调。
    // 用 `Repeated`（续期由事件循环自己做）；采满后回调直接 `exit`，不需要 `stop()`。
    let timer = slint::Timer::default();
    {
        let frame = Rc::clone(&frame);
        let last_tick = Rc::clone(&last_tick);
        let intervals_ms = Rc::clone(&intervals_ms);
        let visible_seen = Rc::clone(&visible_seen);
        let warmup_ms = Rc::clone(&warmup_ms);
        let ui = ui.clone_strong();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_micros(FRAME_INTERVAL_US),
            move || {
                let Some(mut intervals) = intervals_ms.try_borrow_mut().ok() else {
                    say("BASELINE-003(GPU档) 内部错误: 帧回调重入 —— 本档记失败");
                    std::process::exit(2);
                };
                let index = frame.get();
                // 第 0 帧不产生区间（`last_tick` 是刚启动的时刻，量它等于量调度延迟）。
                // 第 0 帧的区间量的是"首帧投影 10 万音符 + 首帧渲染 + 窗口首次映射"，
                // 它不属于稳态（规格要的是**滚动**帧时）⇒ 单独记，不进分位。
                let since_last = last_tick.get().elapsed().as_secs_f64() * 1000.0;
                if index == 0 {
                    warmup_ms.set(since_last);
                } else {
                    intervals.push(since_last);
                }
                last_tick.set(std::time::Instant::now());

                // 见证 ②：窗口真的被映射（只在事件循环里成立）。
                if slint::ComponentHandle::window(&ui).is_visible() {
                    visible_seen.set(true);
                }

                // 口径：每帧推进 1/120 秒 ⇒ 1 屏/秒（与软件档同款换算）。
                let scroll_x = index as f32 * (viewport_width / 120.0);
                host::apply_view(&ui, &project_view, viewport_width, scroll_x);
                slint::ComponentHandle::window(&ui).request_redraw();
                frame.set(index + 1);

                if index + 1 < FRAMES {
                    return;
                }

                let mut samples = std::mem::take(&mut *intervals);
                if samples.len() != FRAMES - 1 {
                    say(&format!(
                        "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=帧区间数={} != {}",
                        samples.len(),
                        FRAMES - 1
                    ));
                    std::process::exit(2);
                }
                samples.sort_by(|a, b| a.partial_cmp(b).expect("无 NaN"));
                let pct = |q: f64| samples[((samples.len() as f64 - 1.0) * q).round() as usize];
                let min = samples[0];
                if min <= 0.0 {
                    say(&format!(
                        "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=帧区间最小值={min:.6}ms (回调在空转, 没有真的渲染)"
                    ));
                    std::process::exit(2);
                }
                // 分布（读的人要能分辨"整体都慢"与"只丢了几帧"）：
                // 统计口径 = 每个 8.333ms 桶里的帧区间个数（桶上界 = (i+1)*8.333ms）。
                let mut histogram = [0usize; 5];
                let mut over_budget = 0usize;
                for sample in &samples {
                    let bucket = ((sample / (FRAME_INTERVAL_US as f64 / 1000.0)).floor() as usize)
                        .min(histogram.len() - 1);
                    histogram[bucket] += 1;
                    if *sample > 8.3 {
                        over_budget += 1;
                    }
                }
                say(&format!(
                    "{LINE_PREFIX} 路径=GPU(winit+FemtoVG/OpenGL) 结果=已测 帧数={FRAMES} 音符={note_count} p50={:.3}ms p90={:.3}ms p99={:.3}ms max={:.3}ms min={:.3}ms 超8.3ms帧数={over_budget}/{} 见证窗口已映射={} 见证字符数下限={} load_average={}",
                    pct(0.50),
                    pct(0.90),
                    pct(0.99),
                    samples[samples.len() - 1],
                    min,
                    samples.len(),
                    visible_seen.get(),
                    note_count,
                    load_average()
                ));
                say(&format!(
                    "{LINE_PREFIX} (GPU档) 分布(桶上界=8.333ms 的整数倍): <=8.3ms={} 8.3-16.7ms={} 16.7-25.0ms={} 25.0-33.3ms={} >33.3ms={}",
                    histogram[0], histogram[1], histogram[2], histogram[3], histogram[4]
                ));
                say(&format!(
                    "{LINE_PREFIX} (GPU档) 首帧(含 10 万音符投影 + 首次映射, 不属于稳态)={:.3}ms",
                    warmup_ms.get()
                ));
                std::process::exit(0);
            },
        );
    }

    // `timer` 必须活到 `run()` 返回（局部绑定天然如此；这一句把意图写显式）。
    let _keep_timer_alive = &timer;
    match slint::ComponentHandle::run(&ui) {
        Ok(()) => {
            say(
                "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=事件循环在采满帧数前退出",
            );
            2
        }
        Err(err) => {
            say(&format!(
                "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=事件循环异常: {err}"
            ));
            2
        }
    }
}
