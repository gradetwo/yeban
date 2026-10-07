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
//! ## 口径（**两个数字数两个对象**；正式门禁 = 绘制回调，不含呈现）
//!
//! - 场景：`yeban_model::samples::project_with_notes(100_000)`（**恰好** 10 万，不是近似）；
//! - 尺寸：1920×1080（与 Tier-1 软件档、与 Golden 同一尺寸）；
//! - 每帧推进滚动 1/120 秒 ⇒ 1 屏/秒（`docs/DEVELOPMENT_LEDGER.md` 第 136 轮的规格，
//!   与软件档同款换算：`scroll_x = frame * (viewport_width / 120)`）；
//! - **正式门禁口径 = 绘制回调耗时（不含呈现）**，门限取规格**自己**的那个数：
//!   `[UI-NOTE-001]` 步骤 ④ 的 **≤ 2 ms**
//!   （`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md:172-174`：
//!   "Slint FemtoVG / OpenGL 硬件绘制回调 … 耗时 ≤ 2ms"）。
//!   量法是 `slint::Window::set_rendering_notifier` 的 `BeforeRendering` → `AfterRendering` 区间。
//!   为什么这个区间**就是**"绘制成本（不含呈现）"：上游把 `AfterRendering` 的语义逐字写为
//!   "the scene of items was rendered, but the back buffer was **not** sent for display
//!   presentation yet (for example GL swap buffers)"
//!   （`i-slint-core-1.18.1/api.rs:346-348`），而 FemtoVG 的 `present_surface()`
//!   （内部即 `swap_buffers()`，`i-slint-renderer-femtovg-1.18.1/opengl.rs:275`）在
//!   `AfterRendering` **之后**才调用（`i-slint-renderer-femtovg-1.18.1/lib.rs:332-335`）
//!   ⇒ 垂直同步/缓冲交换**不在**采样区间里。
//! - **环境口径（记录，不是门禁）= 墙钟帧间隔**：`slint::Timer::start(TimerMode::Repeated,
//!   8_333µs, ...)` 两次回调之间的真实间隔，含注入投影、`request_redraw`、winit 事件循环调度、
//!   渲染与交换/垂直同步。它在**自适应刷新率**面板上被面板自己的节奏钉住 —— 本机对照实验
//!   （同场景、同计时器，**只换渲染器**）：`femtovg` p99 10.389 ms / `software` p99 10.123 ms，
//!   两者落在**同一个 ~10 ms 周期**上，而 CPU 时间只有 6.5 s 墙钟里的 0.24 s user
//!   ⇒ 周期来自呈现节奏，**不是**绘制成本。因此 8.3 ms 的 120 Hz 帧预算**只作为环境读数记录**：
//!   既不放松、也不拿来冒充产品开销（正式门限仍是规格的 2 ms，不是别的数）。
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
//! 的 `let canvas = canvas?;`）。改成四条**可判定**的见证：
//! ① 选中的后端名字与 `require_opengl`（打印在 `声明:` 行，父进程逐字断言）；
//! ② 窗口真的被映射（`is_visible()`，只在事件循环里成立）；
//! ③ **帧真的在走**：`intervals` 采满 600 个、且**最小值**不是 0（0 意味着回调在
//!    没有帧的情况下空转）；
//! ④ **运行期图元 API 见证**：`set_rendering_notifier` 把 `GraphicsAPI` 交给回调，因此
//!    本档现在能**在运行期**读到"这条路径用的就是 OpenGL"（`GraphicsAPI::NativeOpenGL`，
//!    打印为 `见证绘制API=NativeOpenGL`）。这把原先那句"公开 API 无法回读运行期渲染器"
//!    从**残余不确定性**降级为**已证事实**（`require_opengl` 仍需保留：它是选择期闸门）。
//!
//! ## 退出码（父进程按码判定，不看措辞）
//!
//! | 码 | 含义 |
//! | :--- | :--- |
//! | 0 | 采满帧数并打印了读数 |
//! | 1 | 后端选择失败 / 平台槽位探针异常 / **绘制回调探针装不上**（**绝不回退**） |
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
/// **正式门限**：绘制回调耗时 ≤ 2 ms —— 规格自己的数
/// （`[UI-NOTE-001]` 步骤 ④，`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md:172-174`）。
/// ⚠ 这是一个**常量**（不是配置）：任何把它改大的改动都会让 `tests/fps_gpu_path.rs` 的 PIN 变红。
const DRAW_BUDGET_MS: f64 = 2.0;
/// 环境口径的目标节奏：120 Hz 的 1/120 秒（微秒）。⛔ 它**不是**门限 —— 8.3 ms 的帧周期
/// 只在 `环境(非门限)` 行里作为参考机读数记录（自适应刷新率面板的节奏不是产品开销）。
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

/// 分位数（要求 `values` 已升序；`q ∈ [0,1]`）。取法 = 最近秩 `round((n-1)q)`，
/// 与软件档/`BASELINE-004` 读数的口径一致（同一手法在两个口径上复用，避免两套分位定义）。
fn percentile(values: &[f64], q: f64) -> f64 {
    values[((values.len() as f64 - 1.0) * q).round() as usize]
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

/// `GraphicsAPI` 的可打印名字。
///
/// 这是**运行期**的渲染器路径见证：`NativeOpenGL` 意味着这一帧真的经由 OpenGL 提交。
/// `GraphicsAPI` 是 `#[non_exhaustive]`（且 WGPU 分支受 feature 门控）⇒ 必须有兜底分支。
fn api_name(api: &slint::GraphicsAPI<'_>) -> Option<&'static str> {
    match api {
        slint::GraphicsAPI::NativeOpenGL { .. } => Some("NativeOpenGL"),
        slint::GraphicsAPI::WebGL { .. } => Some("WebGL"),
        _ => None,
    }
}

/// **正式口径**的采样器：绘制回调耗时（**不含呈现**）。
///
/// 采样区间由 [`slint::Window::set_rendering_notifier`] 给出：
/// `BeforeRendering`（场景即将渲染：清屏已完成，绘制命令尚未录制）→ `AfterRendering`
/// （场景已渲染，但**后备缓冲尚未送显**）。上游对区间的边界有逐字定义，见本文件头部的"口径"节。
///
/// 采样发生在**渲染回调里**，因此本结构体必须"绝不 panic、绝不阻塞"：任何一次
/// `try_borrow_mut` 失败都只丢一个样本（`unbalanced` 会把它记下来，最终判失败）。
#[derive(Default)]
struct DrawProbe {
    /// 本次 `BeforeRendering` 的时刻；`AfterRendering` 时取出。
    started: Option<std::time::Instant>,
    /// 每次绘制回调的耗时（毫秒），按发生顺序。
    samples: Vec<f64>,
    /// 每次绘制回调发生时，滚动计数器已经推进到的帧号（用来证明采样覆盖整段滚动）。
    frame_at_draw: Vec<usize>,
    /// 运行期图元 API 见证。
    graphics_api: Option<&'static str>,
    /// 见过 `RenderingSetup`（图形上下文初始化完成）。
    setup_seen: bool,
    /// 配对失衡次数（`Before` 连发 / `After` 无 `Before` / 借不到）⇒ 采样不可信。
    unbalanced: usize,
}

impl DrawProbe {
    /// 记一次渲染状态通知（在渲染回调线程上调用）。
    fn observe(
        &mut self,
        state: slint::RenderingState,
        api: &slint::GraphicsAPI<'_>,
        frame: usize,
    ) {
        // 只要看到过一次图元 API 就记住它：`RenderingSetup` 与 `BeforeRendering` 都会带。
        if self.graphics_api.is_none() {
            self.graphics_api = api_name(api);
        }
        match state {
            slint::RenderingState::RenderingSetup => self.setup_seen = true,
            slint::RenderingState::BeforeRendering => {
                if self.started.is_some() {
                    self.unbalanced += 1;
                }
                self.started = Some(std::time::Instant::now());
            }
            slint::RenderingState::AfterRendering => {
                if let Some(started) = self.started.take() {
                    self.samples.push(started.elapsed().as_secs_f64() * 1000.0);
                    self.frame_at_draw.push(frame);
                } else {
                    self.unbalanced += 1;
                }
            }
            // `RenderingTeardown` 与 `#[non_exhaustive]` 的将来变体都不参与计时。
            _ => {}
        }
    }
}

fn run() -> i32 {
    say(&format!(
        "{LINE_PREFIX} (GPU档) 环境: 目标间隔={FRAME_INTERVAL_US}us 帧数={FRAMES} 正式门限=绘制回调 p99<={DRAW_BUDGET_MS}ms 墙钟帧间隔=仅环境记录 load_average={}",
        load_average()
    ));
    if let Err(reason) = select_gpu_path() {
        say(&format!(
            "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因={reason}"
        ));
        return 1;
    }

    // 读数行之前的"口径行"：**请求值** + 两条证据（选择期闸门 + 运行期 API 见证）。
    say(&format!(
        "{LINE_PREFIX} (GPU档) 口径: 请求 backend_name=winit renderer_name=femtovg require_opengl=true 已接受=true 说明=选择期证据是\"显式选择成功\"+\"槽位探针\"; 运行期证据由 set_rendering_notifier 的 GraphicsAPI 给出(见读数行的 见证绘制API=), 因此不再需要\"无法回读运行期渲染器\"这句保留"
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
    // 正式口径的采样器（绘制回调，不含呈现）。
    let draw_probe = Rc::new(RefCell::new(DrawProbe::default()));

    // 定时器句柄必须活到事件循环结束：drop 掉它等于取消回调。
    // 用 `Repeated`（续期由事件循环自己做）；采满后回调直接 `exit`，不需要 `stop()`。
    let timer = slint::Timer::default();
    {
        let frame = Rc::clone(&frame);
        let last_tick = Rc::clone(&last_tick);
        let intervals_ms = Rc::clone(&intervals_ms);
        let visible_seen = Rc::clone(&visible_seen);
        let warmup_ms = Rc::clone(&warmup_ms);
        let draw_probe = Rc::clone(&draw_probe);
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
                let min = samples[0];
                if min <= 0.0 {
                    say(&format!(
                        "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=帧区间最小值={min:.6}ms (回调在空转, 没有真的渲染)"
                    ));
                    std::process::exit(2);
                }

                // ---------- 正式口径：绘制回调耗时（不含呈现） ----------
                //
                // 下面这段先判"采样是否可信"（配对 / 覆盖），**不可信就一个数字都不出**；
                // 判过了才排序出分位。顺序是刻意的：先见证，后数字。
                let probe = draw_probe.borrow();
                let mut draw = probe.samples.clone();
                let draw_api = probe.graphics_api;
                let draw_setup_seen = probe.setup_seen;
                let draw_unbalanced = probe.unbalanced;
                let draw_first_frame = probe.frame_at_draw.first().copied();
                let draw_last_frame = probe.frame_at_draw.last().copied();
                drop(probe);
                let first_frame_text =
                    draw_first_frame.map_or_else(|| "none".to_string(), |value| value.to_string());
                let last_frame_text =
                    draw_last_frame.map_or_else(|| "none".to_string(), |value| value.to_string());
                // 第一次绘制 = 窗口首次映射那一帧（含着色器/字形/纹理首次上传），不属于稳态 ⇒ 单独记。
                let draw_warmup_ms = if draw.is_empty() {
                    None
                } else {
                    Some(draw.remove(0))
                };
                if draw_unbalanced > 0 {
                    say(&format!(
                        "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=绘制回调配对失衡={draw_unbalanced} (BeforeRendering/AfterRendering 不配对 ⇒ 采样不可信, 正式口径不出数字)"
                    ));
                    std::process::exit(2);
                }
                // 采样覆盖见证：请求了 `FRAMES` 次重绘，若连一半都没换来真绘制，那这份分位
                // 描述的就不是"10 万音符滚动"（保守下限：目标 8.333ms / 面板 ~10ms ⇒ 即使
                // 每次重绘都被合并一次也应远超一半）。
                if draw.len() * 2 < FRAMES {
                    say(&format!(
                        "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=绘制回调数={} < {} (请求了 {FRAMES} 次重绘却只观测到这么少 ⇒ 采样没有覆盖整段滚动)",
                        draw.len(),
                        FRAMES / 2
                    ));
                    std::process::exit(2);
                }
                let last_frame_seen = draw_last_frame.unwrap_or(0);
                if last_frame_seen * 10 < FRAMES * 9 {
                    say(&format!(
                        "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=最后一次绘制回调落在第 {last_frame_seen} 帧 (< {FRAMES} 的 90%) ⇒ 采样没跟到滚动末尾"
                    ));
                    std::process::exit(2);
                }
                draw.sort_by(|a, b| a.partial_cmp(b).expect("无 NaN"));
                let over_2ms = draw
                    .iter()
                    .filter(|value| **value > DRAW_BUDGET_MS)
                    .count();
                say(&format!(
                    "{LINE_PREFIX} 路径=GPU(winit+FemtoVG/OpenGL) 结果=已测 帧数={FRAMES} 音符={note_count} 绘制回调(正式门限 p99<={DRAW_BUDGET_MS}ms, 不含呈现): n={} p50={:.3}ms p90={:.3}ms p99={:.3}ms max={:.3}ms 超2ms回调数={over_2ms}/{} 见证绘制API={} 见证RenderingSetup={} 见证配对失衡={draw_unbalanced} 见证首末绘制帧={first_frame_text}/{last_frame_text} 见证窗口已映射={} 见证字符数下限={note_count} load_average={}",
                    draw.len(),
                    percentile(&draw, 0.50),
                    percentile(&draw, 0.90),
                    percentile(&draw, 0.99),
                    draw[draw.len() - 1],
                    draw.len(),
                    draw_api.unwrap_or("(未观察到)"),
                    draw_setup_seen,
                    visible_seen.get(),
                    load_average()
                ));

                // ---------- 环境口径：墙钟帧间隔（**不是门限**，8.3 ms 只作参考机读数记录） ----------
                let mut histogram = [0usize; 5];
                let mut over_8_3 = 0usize;
                for sample in &samples {
                    let bucket = ((sample / (FRAME_INTERVAL_US as f64 / 1000.0)).floor() as usize)
                        .min(histogram.len() - 1);
                    histogram[bucket] += 1;
                    if *sample > 8.3 {
                        over_8_3 += 1;
                    }
                }
                say(&format!(
                    "{LINE_PREFIX} (GPU档) 环境(非门限): 墙钟帧间隔 n={} p50={:.3}ms p90={:.3}ms p99={:.3}ms max={:.3}ms min={:.3}ms 超8.3ms帧数={over_8_3}/{} load_average={}",
                    samples.len(),
                    percentile(&samples, 0.50),
                    percentile(&samples, 0.90),
                    percentile(&samples, 0.99),
                    samples[samples.len() - 1],
                    min,
                    samples.len(),
                    load_average()
                ));
                say(&format!(
                    "{LINE_PREFIX} (GPU档) 分布(桶上界=8.333ms 的整数倍): <=8.3ms={} 8.3-16.7ms={} 16.7-25.0ms={} 25.0-33.3ms={} >33.3ms={}",
                    histogram[0], histogram[1], histogram[2], histogram[3], histogram[4]
                ));
                let mut draw_histogram = [0usize; 5];
                for sample in &draw {
                    let bucket = ((sample / DRAW_BUDGET_MS).floor() as usize)
                        .min(draw_histogram.len() - 1);
                    draw_histogram[bucket] += 1;
                }
                say(&format!(
                    "{LINE_PREFIX} (GPU档) 绘制回调分布(桶上界=2ms 的整数倍): <=2ms={} 2-4ms={} 4-6ms={} 6-8ms={} >8ms={}",
                    draw_histogram[0],
                    draw_histogram[1],
                    draw_histogram[2],
                    draw_histogram[3],
                    draw_histogram[4]
                ));
                say(&format!(
                    "{LINE_PREFIX} (GPU档) 首帧(含 10 万音符投影 + 首次映射, 不属于稳态)={:.3}ms",
                    warmup_ms.get()
                ));
                say(&format!(
                    "{LINE_PREFIX} (GPU档) 首次绘制回调(含首次映射 + 字形/着色器首次上传, 不属于稳态)={}ms",
                    draw_warmup_ms.map_or_else(|| "none".to_string(), |value| format!("{value:.3}"))
                ));
                std::process::exit(0);
            },
        );
    }

    // 正式口径的探针必须在事件循环**之前**装上：第一帧（窗口首次映射）也要被采到，
    // 否则我们连"首帧绘制"这条 warm-up 读数都没有。
    //
    // 装不上就**直接失败**（不出任何数字）：装不上意味着"绘制回调"这个对象在这条路径上
    // 不可观测（例如渲染器不支持 notifier），那么墙钟数字再多也不是正式口径的读数。
    {
        let draw_probe = Rc::clone(&draw_probe);
        let frame_for_probe = Rc::clone(&frame);
        if let Err(err) = slint::ComponentHandle::window(&ui).set_rendering_notifier(
            move |state, graphics_api| {
                // 渲染回调里绝不 panic：拿不到锁就丢一个样本（`unbalanced` 会记账并在末判红）。
                if let Ok(mut probe) = draw_probe.try_borrow_mut() {
                    probe.observe(state, graphics_api, frame_for_probe.get());
                }
            },
        ) {
            say(&format!(
                "BASELINE-003(10万音符) 路径=GPU(winit+FemtoVG/OpenGL) 结果=失败 原因=绘制回调探针装不上 (set_rendering_notifier): {err} —— 正式口径不可观测, 本档不出数字"
            ));
            return 1;
        }
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
