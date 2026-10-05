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
//! | `yeban-app --export-midi <path>` | 把当前工程导出成标准 MIDI 文件（SMF 1，ADR-0001 **D47** 指定的唯一出口），字节出自 `yeban-render` 的**唯一** SMF 编码器 |
//! | `yeban-app --dump-elements` / `--export-elements <path>` | 语义元素清单打到 stdout / 原子写到文件 |
//! | `yeban-app --print-shortcuts` | 快捷键策略表（供 CI 与人类核对） |
//! | `yeban-app --project-sample <default\|filled\|empty>` | 选择"没有 `--open` 时"用哪个工程（默认 `default` = `bridge::demo_project()`，6 轨；`empty` = 真的 0 轨空工程，即 `BASELINE-002` 所指的那个对象） |
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
//!
//! ## 撤销入口（ADR-0001 **D45**）
//!
//! 界面侧的撤销**只有一条路**：`Cmd+Z` / 时光机弹窗的"撤销一步" → [`yeban_app::undo::UndoPort`]
//! → `crate::undo_session`（**与 MCP 的 `yeban_undo` 是同一份源码**，用 `#[path]` 引入）
//! → `CommitGraph::undo_with`。因此"人按 `Cmd+Z` 与 AI 发工具调用"改的是同一串字节。
//!
//! 键盘那一跳（OS 键事件 → `input.rs` 的策略表）**本进程还没有事件源**：
//! [`yeban_app::undo::perform_key`] 已经把"解析结果 → 工程回退"整条链做完并可判据化，
//! 缺的只是把键事件喂进来（Slint 不暴露物理扫描码，见台账的 needs）。

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;

use yeban_app::cli::{self, Options};
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};

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

    // 走带要有东西可驱动 ⇒ GUI 路径真的建一代引擎（快照 + 无锁通道 + 量子驱动）。
    // 用 0 个量子重建（不空转；`reload` **不改**走带状态），随后显式发一条 `Stop`
    // 把这一代停在 `Stopped`，与界面的初始 `playing: false` 一致 ——
    // 这一步是**引擎侧的动作**（真的过无锁通道、真的在量子边界生效），
    // 不是把界面属性改一下了事。
    // 撤销会话（会话运行态）[ADR-0001 D45 / MODEL-ISO-001]。
    // 打开一个工程 = **新会话**：游标与活跃分支都从头开始，因此撤销不可能跨越打开边界。
    // `history.dag` 的**图谱**恢复由 `--open` 的容器层负责，这里只接当前这一份工程。
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });
    let undo_source = match &loaded.source {
        cli::ProjectSource::File { path, .. } => path.display().to_string(),
        cli::ProjectSource::Sample(sample) => format!("sample:{}", sample.name()),
    };
    let undo_session = match UndoSession::open(
        undo_source,
        "yeban-app",
        loaded.archive.project.clone(),
        now_ms,
    ) {
        Ok(session) => session,
        Err(error) => {
            return Err(cli::CliError::Ui {
                detail: format!("撤销会话无法初始化: {error}"),
            });
        }
    };
    let undo_port = Rc::new(UndoPort::new(undo_session));

    let mut engine = EngineHost::new();
    if let Err(error) = engine.reload(&loaded.archive.project, 0) {
        // 引擎建不起来时**出声**：界面照常打开（工程投影本身是好的），
        // 但走带回调会明确报告"没有引擎"，而不是静默地假装在播。
        cli::emit(&[format!(
            "yeban-app: 走带未接线 —— 引擎快照投影失败: {error}"
        )]);
    }
    engine.stop();
    let engine = Rc::new(RefCell::new(engine));
    host::apply_transport(&ui, engine.borrow().transport());

    wire_callbacks(&ui, &engine);
    // `[UI-A11Y-002]` §7.2 的**事件源**：BPM 敲入控件里真 `TextInput` 的
    // `preedit-text` / `has-focus` 变化 → `ime-composition-changed` / `ime-focus-changed`
    // → `InputContext`（合成态的唯一载体）。
    //
    // ⚠ **诚实边界**：在 GUI 路径上，这个状态机目前**还没有读者** ——
    // "Slint 键盘事件 → `input::dispatch_key` → `invoke_*`"那一段仍然没接线
    // （本线只补事件源，不假装守卫已经生效；消费者缺口记在
    // docs/ledger/app-projection-notes.md 的 needs 里）。判据侧（`live_surface`）
    // 的读者是齐的：`ui/property isComposing` 与按键预览都问这个对象。
    let input = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
    host::wire_input(&ui, Rc::clone(&input));
    // 撤销的两条界面入口（弹窗开关 + "撤销一步"按钮）都汇到**同一个** `UndoPort`。
    host::wire_undo(&ui, &undo_port);
    // 启动时先把**模型读数**注入一次（显示态的唯一来源）。
    host::apply_undo(&ui, &undo_port);
    let undo_display = undo_port.display();
    cli::emit(&[format!(
        "撤销: 可撤销 {} 步 · 已撤销 {} 步 · 分支 {} · 实现 = undo_session (与 MCP 的 yeban_undo 同一份源码)",
        undo_display.undoable, undo_display.undone, undo_display.branch
    )]);

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
/// **走带两条已经真的接线**（本切片）：
/// `toggle-play` / `stop` → [`host::wire_transport`] → `EngineHost` → 无锁事件通道
/// → `EngineRuntime` 的量子边界。显示态（`playing` / `timecode`）由
/// `host::apply_transport` 从**引擎读数**回写，界面不再自己翻转状态。
///
/// **撤销两条也已经真的接线**（ADR-0001 D45）：`toggle-undo-tree` 与时光机的
/// "撤销一步"按钮由 [`host::wire_undo`] 接到 [`yeban_app::undo::UndoPort`]，
/// 动作真的会走 `CommitGraph::undo_with`，显示态由 `host::apply_undo` 从模型读数回写。
///
/// 其余六个回调**仍然故意什么都不做**, 只打一行 stderr。原因不是省事:
/// Op 归约、AI 采纳的语义都住在 `yeban-engine` / `yeban-model`,
/// 在这里写一个"看起来在工作"的本地状态翻转, 只会制造"UI 已经通了"的假象。
///
/// 回调跑在 UI 线程上; `[ARCH-TOP-002]` / `[ARCH-RT-001]` 约束的是音频线程,
/// 所以这里的 `eprintln!` 不触碰红线 —— 但接线真实动作时**仍然不许**做长阻塞等待。
fn wire_callbacks(ui: &yeban_app::ui::MainWindow, engine: &Rc<RefCell<EngineHost>>) {
    host::wire_transport(ui, Rc::clone(engine));
    ui.on_toggle_view(|| trace("toggle-view"));
    ui.on_toggle_sidebar(|| trace("toggle-sidebar"));
    ui.on_toggle_ai_drawer(|| trace("toggle-ai-drawer"));
    ui.on_open_musical_pr(|| trace("open-musical-pr"));
    ui.on_accept_ai_proposal(|| trace("accept-ai-proposal"));
    ui.on_reject_ai_proposal(|| trace("reject-ai-proposal"));
    ui.on_run_acoustic_diagnosis(|| trace("run-acoustic-diagnosis"));
}

/// 回调接线点的统一落点。
fn trace(callback: &'static str) {
    eprintln!("[yeban-app] ui callback `{callback}` (未接线: 等待 yeban-engine / yeban-model)");
}
