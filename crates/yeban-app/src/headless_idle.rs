//! `--headless-idle` —— **真的**构造 Slint 控件树，然后空闲 N 秒（`BASELINE-002` 的取样形态）。
//!
//! ## 为什么需要这一档（实测得出的，不是偏好）
//!
//! `[BASELINE-002]` 的判据句是"**空**工程空闲常驻内存 ≤ 35 MB"。此前两次取样都不覆盖这句话：
//!
//! 1. `--headless` 走 [`crate::cli::run_batch`]，而那个模块的第一句就是"**零 Slint 依赖**" ——
//!    它连一个 `MainWindow` 都不构造。实测（`docs/ledger/baseline-memory-notes.md` §3.2）：
//!    `yeban-app --headless` = **11.14–11.33 MB**，看起来漂亮，但规范所指的对象
//!    （Slint 运行时：组件树 / 属性 / 回调 / 字形与光栅缓存）**根本没进这个读数**；
//! 2. 用 `--project-sample empty` 修好了"**空**工程"那一半，但"**空闲常驻**"那一半仍然是
//!    1 的同一个洞。
//!
//! 本模块把第二半补上：装一个**自研**的 Slint 平台（[`MinimalSoftwareWindow`] +
//! `SoftwareRenderer`，来自 `yeban-app` **已经启用**的 `renderer-software` feature ⇒
//! **零新增依赖**），构造**真** `MainWindow`（唯一注入点仍是
//! [`crate::host::build_main_window`]，D28），逐行光栅化一帧当见证，然后空闲 N 秒后退出。
//!
//! | 命令 | 构造了什么 | 本机 debug 实测 `ru_maxrss` |
//! | :--- | :--- | ---: |
//! | `--headless` | **一个 Slint 对象都不构造** | 11.14 MB |
//! | `--headless-idle --idle-seconds N` | 真 `MainWindow` + 一帧逐行光栅化 | 见 `docs/ledger/baseline-headless-idle-notes.md` |
//! | （参照）Tier-1 端口建 1 个真窗口、不渲染 | 真 `MainWindow` + testing backend 控件树 | 55.97 MB |
//!
//! ## 见证（防空转）：为什么是"逐行光栅化"
//!
//! 光说"我建了控件树"不算证据。这里给两个**量出来的**数：
//!
//! 1. `windows-created=`：平台自己的 [`Platform::create_window_adapter`] 被调用了几次
//!    （0 ⇒ Slint 连窗口都没要，谈何控件树）；
//! 2. `lines=` / `non-black-pixels=` / `distinct-colors=`：真的把窗口**光栅化**了一遍。
//!    像素是伪造不了的 —— 只有存在一棵被布局过的活控件树，`SoftwareRenderer` 才吐得出
//!    非零行数与非黑像素；纯色块与多图元界面在 `distinct-colors` 上也是可区分的。
//!
//! 为什么用 [`LineBufferProvider`]（逐行）而不是 `render()`（整帧）：整帧缓冲
//! `1920×1080×3B ≈ 5.93 MiB` 属于 **Tier-1 截图设施**的账，`baseline-memory-notes.md`
//! §4.1 明确写了"别把这笔钱记到空工程空闲占用上"。逐行只保留**一行**，光栅化工作量
//! 与整帧完全相同（同一个 `render_window_frame_by_line`），读数因而更贴近"控件树常驻"
//! 而不是"截图设施常驻"。
//!
//! ## 线程与平台约束（上游语义，已核验）
//!
//! `slint::platform::set_platform` 是**线程局部**的，且每个线程只能装一次；`MainWindow`
//! 与 `MinimalSoftwareWindow` 都是 `Rc` 语义（不是 `Send`）。因此本模块**只**在
//! `yeban-app` 的主线程上跑一次，和 GUI 路径同款。
//!
//! ## 它**不**做什么（边界，写在 stdout 的边界行里）
//!
//! 不创建 OS 窗口（不需要显示器）、不进阻塞事件循环、不连声卡 / 不建引擎、
//! 不做运行时控件树遍历（那需要 `crates/yeban-ui-test-port` 的 testing backend，
//! 属于另一条线）。

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{
    LineBufferProvider, MinimalSoftwareWindow, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, Rgb8Pixel};

use crate::cli::{
    self, CliError, HEADLESS_IDLE_BOUNDARY, HEADLESS_IDLE_HANDSHAKE, IdleReport, IdleWitness,
    Options,
};
use crate::elements::ElementRegistry;
use crate::host;
use crate::scene::DemoScene;

/// 空闲循环的一拍（毫秒）。
///
/// 10 ms 是个折中：足够小，所以"空闲 N 秒"的实测时长不会被拍长拖出明显误差；
/// 足够大，所以一个有 60 秒上限的读数不会把 CPU 打满（本机纪律：不跑高耗 CPU 任务）。
const IDLE_TICK_MS: u64 = 10;

/// 把窗口适配器交给同一个 [`MinimalSoftwareWindow`] 的平台实现。
///
/// 结构与 `crates/yeban-ui-test-port/src/render.rs` 的同名实现一致（那是仓内已核验的
/// 无头路径）：`create_window_adapter` 是 `Platform` 在 Slint 1.18.1 的**唯一**必需方法，
/// 其余都有默认实现。唯一多出来的是 `windows_created` 计数器 —— 它是本线的**见证**之一。
struct IdlePlatform {
    /// 唯一那个软件窗口。
    window: Rc<MinimalSoftwareWindow>,
    /// `create_window_adapter` 被调用的次数（见证：应当恰好 1）。
    windows_created: Rc<Cell<usize>>,
}

impl Platform for IdlePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        self.windows_created.set(self.windows_created.get() + 1);
        Ok(self.window.clone())
    }
}

/// 逐行光栅化的见证收集器：**只**保留一行像素，永不分配整帧缓冲。
struct LineWitness {
    /// 当前行的工作缓冲（长度 = 窗口宽度）。
    line: Vec<Rgb8Pixel>,
    /// 上一条被渲染的行号（用来数**不同的**行，而不是数回调次数）。
    last_line: Option<usize>,
    /// 被渲染过的不同行数。
    lines: u64,
    /// 非黑（RGB 非全零）像素数。
    non_black: u64,
    /// 出现过的不同颜色数。
    colors: BTreeSet<u32>,
}

impl LineWitness {
    /// 造一个宽度为 `width` 的单行见证器。
    fn new(width: usize) -> Self {
        Self {
            line: vec![Rgb8Pixel::new(0, 0, 0); width],
            last_line: None,
            lines: 0,
            non_black: 0,
            colors: BTreeSet::new(),
        }
    }

    /// 记下一行（`range` 之内）已经被渲染的像素。
    ///
    /// 参数是 `range` 而不是切片：调用方在 `&mut self` 上不能同时借出 `&self.line`
    /// （E0502），而"先渲染、再统计"本来就该是两步。
    fn observe(&mut self, line_index: usize, range: std::ops::Range<usize>) {
        if self.last_line != Some(line_index) {
            self.lines += 1;
            self.last_line = Some(line_index);
        }
        let mut non_black = 0u64;
        for pixel in &self.line[range] {
            let packed =
                (u32::from(pixel.r) << 16) | (u32::from(pixel.g) << 8) | u32::from(pixel.b);
            if packed != 0 {
                non_black += 1;
            }
            self.colors.insert(packed);
        }
        self.non_black += non_black;
    }
}

impl LineBufferProvider for &mut LineWitness {
    type TargetPixel = Rgb8Pixel;

    fn process_line(
        &mut self,
        line: usize,
        range: std::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Rgb8Pixel]),
    ) {
        // 防御性扩容：渲染器给的 range 理论上不会超过窗口宽度，但**越界就是 panic**，
        // 而 panic 在渲染回调里会把一次"读数不对"变成"进程崩了"，不好排障。
        if self.line.len() < range.end {
            self.line.resize(range.end, Rgb8Pixel::new(0, 0, 0));
        }
        render_fn(&mut self.line[range.clone()]);
        self.observe(line, range);
    }
}

/// 跑一次 `--headless-idle`：构造真控件树 → 光栅化一帧 → 空闲 N 秒 → 返回该打印的行。
///
/// 行的顺序是**契约**（与 [`crate::cli::run_batch`] 同款）：
/// 1. 握手行 [`HEADLESS_IDLE_HANDSHAKE`]（与 `headless ok` **刻意不同**）；
/// 2. [`cli::project_report`]（来源 / 工程读数）；
/// 3. [`cli::view_report`]（`view-counts:`，含 `elements=`）；
/// 4. 见证行（窗口数 / 尺寸 / 行数 / 非黑像素 / 颜色数 / 元素数）；
/// 5. 空闲行（请求秒数 / **实测**毫秒 / tick 数 —— 实测值，不是把输入抄一遍）；
/// 6. 边界行 [`HEADLESS_IDLE_BOUNDARY`]。
///
/// # Errors
///
/// [`CliError`]：打开 / 投影失败（与其它命令同源），或平台 / 组件构造失败（[`CliError::Ui`]）。
pub fn run(options: &Options) -> Result<Vec<String>, CliError> {
    let seconds = options.idle_seconds.ok_or_else(|| CliError::Ui {
        detail: "内部错误: --headless-idle 缺 --idle-seconds —— parse 已保证成对, \
                 这条只可能来自手工构造的 Options"
            .to_owned(),
    })?;

    // 与其它命令**同源**的取工程 / 投影路径：`--open` / `--project-sample` 语义一字不差。
    let loaded = cli::load_project(options)?;
    let view = cli::project_view(&loaded.archive.project)?;
    let scene = DemoScene::from_view(&view);
    let width = scene.viewport_width.max(1);
    let height = scene.viewport_height.max(1);

    // ① 装平台 —— 必须在**构造任何 Slint 组件之前**（上游要求）。
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(width, height));
    let windows_created = Rc::new(Cell::new(0usize));
    slint::platform::set_platform(Box::new(IdlePlatform {
        window: window.clone(),
        windows_created: Rc::clone(&windows_created),
    }))
    .map_err(|error| CliError::Ui {
        detail: format!("无法安装无头软件平台 (每个线程只能装一次): {error}"),
    })?;

    // ② 真的构造主窗口（唯一注入点，D28）并 show —— 这一刻起控件树是真的存在的。
    let ui = host::build_main_window(&view, &scene).map_err(|error| CliError::Ui {
        detail: format!("无法创建主窗口 ({error})"),
    })?;
    ui.show().map_err(|error| CliError::Ui {
        detail: format!("主窗口 show 失败: {error}"),
    })?;
    // show 之后再钉一次尺寸：与 Tier-1 端口同款（组件布局要拿到最终尺寸）。
    window.set_size(PhysicalSize::new(width, height));

    // ③ 逐行光栅化**一帧** = 见证。`draw_if_needed` 只在"需要重绘"时才调回调，
    //    所以先显式 request_redraw（否则会拿到"没有渲染"，表现为见证全零 = 判红）。
    let mut witness_line = LineWitness::new(width as usize);
    window.request_redraw();
    let rendered = window.draw_if_needed(|renderer| {
        renderer.render_by_line(&mut witness_line);
    });

    let witness = IdleWitness {
        windows_created: windows_created.get(),
        width,
        height,
        rendered,
        lines: witness_line.lines,
        non_black_pixels: witness_line.non_black,
        distinct_colors: witness_line.colors.len(),
        elements: ElementRegistry::from_view(&view).len(),
    };

    // ④ 空闲 N 秒。**刻意不跑事件循环**：本档要量的是"控件树建好之后静静待着"的常驻集，
    //    而不是"事件循环在忙"的峰值。空闲期间不做任何分配、不重绘。
    let started = Instant::now();
    let target = Duration::from_secs(u64::from(seconds));
    let mut ticks = 0u64;
    while started.elapsed() < target {
        std::thread::sleep(Duration::from_millis(IDLE_TICK_MS));
        ticks += 1;
    }
    let report = IdleReport {
        seconds,
        elapsed_ms: started.elapsed().as_millis(),
        ticks,
    };

    // 控件树必须在整个空闲期间**活着**（上面的 `ui` / `window` 到函数返回才 drop）；
    // 这里显式再摸一次活窗口，确保"空闲期间树还在"不是靠编译器的 drop 时机侥幸成立。
    let live = ui.window().size();
    if live.width != width || live.height != height {
        return Err(CliError::Ui {
            detail: format!(
                "空闲之后活窗口尺寸变了: 请求 {width}x{height}, 实测 {}x{} —— \
                 控件树已不完整, 这个读数不能用作基线",
                live.width, live.height
            ),
        });
    }

    let mut lines = Vec::new();
    lines.push(HEADLESS_IDLE_HANDSHAKE.to_owned());
    lines.extend(cli::project_report(&loaded));
    lines.extend(cli::view_report(&view));
    lines.push(cli::idle_witness_line(&witness));
    lines.push(cli::idle_report_line(&report));
    lines.push(HEADLESS_IDLE_BOUNDARY.to_owned());
    Ok(lines)
}
