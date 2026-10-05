//! `yeban-app` 可执行入口 —— **分发**（命令行面住在 [`yeban_app::cli`]）。
//!
//! ## 运行形态
//!
//! | 命令 | 行为 |
//! | :-- | :--- |
//! | `yeban-app` | 把一个 `YebanProjectV1` 投影成 `ViewState`、注入 `MainWindow`、进 Slint 事件循环 |
//! | `yeban-app --open <path>` | **打开一个真实的工程文档**（`.yeban` 容器 —— 唯一工程格式），把它作为当前工程驱动界面 |
//! | `yeban-app --headless`（或 `SLINT_BACKEND=headless`） | **不构造任何 Slint 组件**，打印 `headless ok` 与工程读数后退出 0 |
//! | `yeban-app --headless --open <path>` | 无显示器环境下的"打开这个工程"自检：真的读文件、真的投影、打印读数 |
//! | `yeban-app --save-as <path>` | 把当前工程**原子**写成 `.yeban` 容器（`[ARCH-SEC-004]`），不构造窗口 |
//! | `yeban-app --dump-elements` / `--export-elements <path>` | 语义元素清单打到 stdout / 原子写到文件 |
//! | `yeban-app --print-shortcuts` | 快捷键策略表（供 CI 与人类核对） |
//! | `yeban-app --project-sample <default\|filled>` | 选择"没有 `--open` 时"用哪个工程（默认 `default` = `bridge::demo_project()`） |
//! | `yeban-app --version` / `--help` | 真实版本 / 完整用法（含全部开关、组合语义、退出码） |
//!
//! 命令行语法、用法文本、报告格式与退出码**全部**住在 `yeban_app::cli`（零 Slint 依赖），
//! 本文件只做两件事：解析 argv，然后决定"开窗口"还是"走无窗口批处理"。
//! 两条路径共用同一条数据流：`YebanProjectV1` → `bridge::ViewState` → `host`，
//! 因此 `--headless` 打印的计数、`--dump-elements` 打印的语义 ID、GUI 画的像素**必然一致**。
//!
//! ## 无头路径为什么"什么都不画"（实测记录，不是偷懒）
//!
//! 规范 `[ARCH-UI-003]`（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 222-228 行）与
//! `[UI-TEST-003]`（UI/UX 规范 §12.1）都写了：
//!
//! ```text
//! SLINT_BACKEND=headless ./target/debug/yeban-app --headless
//! ```
//!
//! 但 Slint **1.18.1 没有名为 `headless` 的后端**。实测（docs.rs + 官方指南）：
//! `SLINT_BACKEND` 只接受后端名 `qt` / `winit` / `linuxkms`，可用 `-` 追加渲染器后缀
//! （`winit-software` / `linuxkms-skia` / `winit-vello` …）。出处：
//! <https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/>
//! 与 <https://docs.rs/slint/1.18.1/slint/struct.BackendSelector.html>（"programmatic substitute
//! for the `SLINT_BACKEND` environment variable"，`backend_name` 只接受已编译进去的后端名）。
//!
//! 于是只有两条路：
//!
//! 1. **构造真窗口 + 真后端**：`MainWindow::new()` 需要一个能创建窗口适配器的平台。
//!    在无显示器环境这会失败；在有显示器的环境又会阻塞事件循环 —— 两者都不满足
//!    "CI 在无显示器环境跑得起来"。
//! 2. **走 `i-slint-backend-testing`**（`slint::platform::set_platform` + 该 crate 的
//!    `init_*`）：这是 `[ARCH-UI-005]` 指定的路，但它属于 `crates/yeban-ui-test-port`
//!    这条**别的工作线**；它的 `init_no_event_loop()` 一类 API 名字我**没有核验过**，
//!    不凭记忆写（见 notes 的 pending）。
//!
//! 本次交付取法 2 的**诚实子集**：无头时不构造任何 Slint 对象，只走纯 Rust 的
//! 场景/注册表路径并打印握手行。这样 CI 拿到的是一个**真的**不阻塞、不依赖显示器的进程，
//! 而不是一个假装无头的 GUI。代价是这一版无头模式**不能**做控件树断言 ——
//! 那需要 `yeban-ui-test-port` 把 testing backend 接起来。边界写清楚了，不是静默降级。
//!
//! 注：`--open` **不改变**上面这条结论 —— 它读文件、投影、注入，走的都是纯 Rust 层；
//! GUI 路径仍然需要一个真显示器。

use std::process::ExitCode;

use yeban_app::cli::{self, Options};
use yeban_app::host;
use yeban_app::scene::DemoScene;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut options = match cli::parse(&args) {
        Ok(options) => options,
        Err(error) => {
            // 用法错误：原因 + 完整用法（这里**是**该补用法文本的地方 —— 用户正需要它）。
            eprintln!("yeban-app: {error}\n\n{}", cli::usage_text());
            return ExitCode::from(error.exit_code());
        }
    };

    // `SLINT_BACKEND=headless` 是**规范写的字面值**（`[ARCH-UI-003]` / `[UI-TEST-003]`），
    // 但 Slint 1.18.1 不认它。我们保留这个字面值作为 yeban 自己的哨兵，这样规范里的
    // 命令行原样可跑，而不是让规范去迁就上游。折算成 `--headless` 之后，
    // "哪些开关算无窗口"仍然只有 `Options` 里那一张表。
    if matches!(std::env::var("SLINT_BACKEND").as_deref(), Ok("headless")) {
        options.headless = true;
    }

    if options.wants_gui() {
        cli::finish(run_gui(&options))
    } else {
        cli::finish(cli::run_batch(&options))
    }
}

/// 正常 GUI 路径（`--open` 时用它打开的那个工程驱动界面）。
///
/// 数据流：`YebanProjectV1` → [`yeban_app::bridge::ViewState`] → [`host::build_main_window`]。
/// 这里**没有**任何"演示数据"分支 —— `--open` / `--project-sample` 只换工程。
///
/// 报告行在**进入事件循环之前**打印：用户要能先看到"打开了哪个文件、多少轨道"，
/// 而不是盯着一个还没画出来的窗口猜。
///
/// 返回值恒为 `Ok(vec![])`：报告已经由 [`cli::emit`] 打过了，`cli::finish` 不该再打第二遍。
fn run_gui(options: &Options) -> Result<Vec<String>, cli::CliError> {
    let loaded = cli::load_project(options)?;
    let view = cli::project_view(&loaded.archive.project)?;
    let scene = DemoScene::from_view(&view);

    let mut lines = cli::project_report(&loaded);
    lines.extend(cli::view_report(&view));
    cli::emit(&lines);

    let ui = match host::build_main_window(&view, &scene) {
        Ok(ui) => ui,
        Err(error) => {
            return Err(cli::CliError::Ui {
                detail: format!(
                    "无法创建主窗口 ({error})。无显示器环境请用 `--headless`; \
                     注意 Slint 1.18.1 没有名为 headless 的后端, 详见 src/main.rs 的模块文档。"
                ),
            });
        }
    };

    wire_callbacks(&ui);

    // 用 UFCS 而不是 `ui.run()`: `run()` 是 `slint::ComponentHandle` 的**trait 方法**,
    // 直接调用要求该 trait 在作用域内; 而显式 `use slint::ComponentHandle;` 在生成代码
    // 恰好把它带进作用域时会变成 unused import, 直接撞上 `-D warnings`。
    // UFCS 两种情况下都成立, 且不需要任何 import。
    match slint::ComponentHandle::run(&ui) {
        Ok(()) => Ok(Vec::new()),
        Err(error) => Err(cli::CliError::Ui {
            detail: format!("事件循环异常退出: {error}"),
        }),
    }
}

/// 把 `MainWindow` 的回调接到动作上。
///
/// 现在**故意**什么都不做, 只打一行 stderr。原因不是省事:
/// 走带、Op 归约、AI 采纳的语义都住在 `yeban-engine` / `yeban-model`,
/// 在这里写一个"看起来在工作"的本地状态翻转, 只会制造"UI 已经通了"的假象。
///
/// 回调跑在 UI 线程上; `[ARCH-TOP-002]` / `[ARCH-RT-001]` 约束的是音频线程,
/// 所以这里的 `eprintln!` 不触碰红线 —— 但接线真实动作时**仍然不许**做长阻塞等待。
fn wire_callbacks(ui: &yeban_app::ui::MainWindow) {
    ui.on_toggle_play(|| trace("toggle-play"));
    ui.on_toggle_view(|| trace("toggle-view"));
    ui.on_toggle_sidebar(|| trace("toggle-sidebar"));
    ui.on_toggle_ai_drawer(|| trace("toggle-ai-drawer"));
    ui.on_toggle_undo_tree(|| trace("toggle-undo-tree"));
    ui.on_open_musical_pr(|| trace("open-musical-pr"));
    ui.on_accept_ai_proposal(|| trace("accept-ai-proposal"));
    ui.on_reject_ai_proposal(|| trace("reject-ai-proposal"));
    ui.on_run_acoustic_diagnosis(|| trace("run-acoustic-diagnosis"));
}

/// 回调接线点的统一落点。
fn trace(callback: &'static str) {
    eprintln!("[yeban-app] ui callback `{callback}` (未接线: 等待 yeban-engine / yeban-model)");
}
