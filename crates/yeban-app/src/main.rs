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
//! | `yeban-app --headless-idle --idle-seconds N` | **真的构造 Slint 控件树**（自研软件平台 = `MinimalSoftwareWindow` + `SoftwareRenderer`，不需要显示器）、逐行光栅化一帧当见证、空闲 N 秒后退出 —— `[BASELINE-002]` 那句"**空**工程空闲常驻内存"要量的对象（住在 [`yeban_app::headless_idle`]） |
//! | `yeban-app --save-as <path>` | 把当前工程**原子**写成 `.yeban` 容器（`[ARCH-SEC-004]`），不构造窗口 |
//! | `yeban-app --export-midi <path>` | 把当前工程导出成标准 MIDI 文件（SMF 1，ADR-0001 **D47** 指定的唯一出口），字节出自 `yeban-render` 的**唯一** SMF 编码器 |
//! | `yeban-app --dump-elements` / `--export-elements <path>` | 语义元素清单打到 stdout / 原子写到文件 |
//! | `yeban-app --print-shortcuts` | 快捷键策略表（供 CI 与人类核对） |
//! | `yeban-app --project-sample <default\|filled\|empty>` | 选择"没有 `--open` 时"用哪个工程（默认 `default` = `bridge::demo_project()`，6 轨；`empty` = 0 音轨 ＋ 1 主总线的空工程，即 `BASELINE-002` 所指的那个对象） |
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
//! **谁是权威**（`ROAD-M4-008` 选项 (a) 第二片）：`run_gui` 先挂进程内控制面
//! （`in-process-mcp`（默认 feature，2026-11-12 裁决起）+ 运行期开关），再据结果构造 `UndoPort` ——
//! 挂上了就用 [`yeban_app::undo::UndoPort::from_authority`]（端口只握
//! `ProjectAuthorityHandle`，写入口落到控制面正在服务的那一个 `Domain`），
//! 没挂上才用 [`yeban_app::undo::UndoPort::new`]（自己的一份会话，那时进程里没有第二个写者）。
//! 因此**运行期未挂载控制面时，行为与无控制面构建一位没变**。
//!
//! 键盘那一跳（OS 键事件 → `input.rs` 的策略表）**已经接上**（`N2` 裁决 (1)）：
//! `.slint` 的 `FocusScope.key-pressed` 把**逻辑键**（`event.text` + 修饰位）交给
//! [`yeban_app::host::wire_keys`] → `input::resolve_logical` →（撤销族经
//! [`yeban_app::undo::dispatch_key`] 这个唯一下发点）→ [`yeban_app::undo::UndoPort`]。
//! Slint 不暴露物理扫描码，因此 GUI 绑逻辑键；无头端口保留物理码判据（见 `N2` 行）。

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::cli::{self, Options};
use yeban_app::engine_host::EditMark;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};

/// 主线程心跳的周期（毫秒）。
///
/// `16 ms` ⇒ **62.5 Hz**，不低于 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2
/// 要求的 60Hz（那一节要求主线程以 60Hz 轮询退役队列并负责旧快照的 `Drop`）。
/// 这一跳同时承担"编辑 ⇒ 增量发布快照"与"写者侧回收"两件事，见 `run_gui` 的注释。
const HEARTBEAT_PERIOD_MS: u64 = 16;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // `--inspect-logic <dir>`（#143）是**独立**的只读探针：它不需要工程、不进 GUI、
    // 也不与任何其它开关组合（组合 = 用法错误，⛔ 不静默丢掉别的开关）。
    // 因此它在 `cli::parse` **之前**分流 —— `Options` 里没有它的字段，也不该有。
    // 默认构建里 `parse_inspect_logic` 会把"开关在但没编译"变成一次点名 feature 的
    // 用法错误（退出码 2）；带 feature 时 `run_inspect_logic` 只打印 demo 读数。
    if let Some(request) = cli::parse_inspect_logic(&args) {
        return match request {
            Ok(dir) => {
                #[cfg(feature = "experimental-logic-import")]
                {
                    cli::finish(cli::run_inspect_logic(&dir))
                }
                #[cfg(not(feature = "experimental-logic-import"))]
                {
                    // 不可达（默认构建的 `parse_inspect_logic` 已返回
                    // `InspectLogicNotCompiled`）；保留响亮的兜底而不是静默成功。
                    let _ = dir;
                    eprintln!(
                        "yeban-app: `{}` 需要本次构建带 `--features {}`",
                        cli::LOGIC_INSPECT_SWITCH,
                        cli::LOGIC_INSPECT_FEATURE
                    );
                    ExitCode::from(cli::EXIT_USAGE)
                }
            }
            Err(error) => {
                eprintln!("yeban-app: {error}\n\n{}", cli::usage_text());
                ExitCode::from(error.exit_code())
            }
        };
    }

    // `--import-logic <demo> --save-as <out.yeban>`（#149 / 刀 2）也是**独立**命令：
    // 它不打开当前工程、不进 GUI、不与别的动作组合。输入 demo 只读，产出只写
    // `--save-as`；路径不存在 = skip（不红），路径在但读不懂 = 错误（红）。
    if let Some(request) = cli::parse_import_logic(&args) {
        return match request {
            Ok((input, output)) => {
                #[cfg(feature = "experimental-logic-import")]
                {
                    match yeban_app::import_logic::run(&input, &output) {
                        Ok(lines) => cli::finish(Ok(lines)),
                        Err(error) => {
                            eprintln!("yeban-app: {error}");
                            ExitCode::from(error.exit_code())
                        }
                    }
                }
                #[cfg(not(feature = "experimental-logic-import"))]
                {
                    // 不可达（默认构建的 `parse_import_logic` 已返回
                    // `ImportLogicNotCompiled`）；保留响亮的兜底而不是静默成功。
                    let _ = (input, output);
                    eprintln!(
                        "yeban-app: `{}` 需要本次构建带 `--features {}`",
                        cli::IMPORT_LOGIC_SWITCH,
                        cli::IMPORT_LOGIC_FEATURE
                    );
                    ExitCode::from(cli::EXIT_USAGE)
                }
            }
            Err(error) => {
                eprintln!("yeban-app: {error}\n\n{}", cli::usage_text());
                ExitCode::from(error.exit_code())
            }
        };
    }

    // `--migrate-groove <old.groove> --save-as <new.yeban>`（#146）同样是**独立**命令：
    // 它不打开当前工程、不进 GUI、也不与别的动作组合。旧文件是**只读**输入，
    // 产出只写 `--save-as`（旧文件前后逐字节比对，见 `migrate_groove::run`）。
    // 与 `--inspect-logic` 一样在 `cli::parse` **之前**分流 —— `Options` 里没有它的字段。
    if let Some(request) = cli::parse_migrate_groove(&args) {
        return match request {
            Ok((input, output)) => {
                match yeban_app::migrate_groove::run(&input, &output) {
                    Ok(lines) => cli::finish(Ok(lines)),
                    Err(error) => {
                        // 失败只打一行：`MigrateError` 的 Display 已经带路径与逐字原因
                        // （缺哪个字段 / 哪种 format / 保存层的原样裁决）。
                        eprintln!("yeban-app: {error}");
                        ExitCode::from(error.exit_code())
                    }
                }
            }
            Err(error) => {
                eprintln!("yeban-app: {error}\n\n{}", cli::usage_text());
                ExitCode::from(error.exit_code())
            }
        };
    }

    // ⭐ 任务单 `#211`：`--probe-instruments` 是**独立**的只读探针：不需要工程、
    // 不进 GUI、不写任何文件，也不与任何其它开关组合（组合 = 用法错误，⛔ 不静默丢掉
    // 别的开关）。与 `--inspect-logic` / `--migrate-groove` 同一条分流形状。
    //
    // ⚠ 它逐条**真的读盘解码**（与点击装乐器同一条判据）⇒ 真库全量一次约 10 分钟。
    // 因此每一行算完就 `println!` + `flush`，而不是攒到最后 —— 让终端看得见进度。
    if let Some(request) = cli::parse_probe_instruments(&args) {
        return match request {
            Ok(()) => {
                use std::io::Write as _;
                cli::run_probe_instruments(|line| {
                    println!("{line}");
                    // 管道 / 重定向时 stdout 是块缓冲的：不 flush 就看不见进度。
                    let _ = std::io::stdout().flush();
                });
                ExitCode::from(cli::EXIT_OK)
            }
            Err(error) => {
                eprintln!("yeban-app: {error}\n\n{}", cli::usage_text());
                ExitCode::from(error.exit_code())
            }
        };
    }

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

    // 调试输出（T-FIX-70）：在**任何**路径分流之前定案并初始化汇点 ——
    // `--debug` / `--log-file` / 已存的菜单开关任一开启即生效
    // （`settings::resolve_debug_output`）。日志文件打不开 = 响亮的启动
    // 错误：用户明确要了这份回传文件，静默丢掉比启动失败更坏。
    let stored_settings = yeban_app::settings::load();
    let debug_enabled = yeban_app::settings::resolve_debug_output(
        options.debug,
        options.log_file.is_some(),
        stored_settings.debug_output,
    );
    if let Err(error) = yeban_app::debug_out::init(debug_enabled, options.log_file.as_deref()) {
        eprintln!("yeban-app: {error}");
        return ExitCode::from(cli::EXIT_USAGE);
    }
    if debug_enabled {
        let mut first = vec![format!(
            "[debug] 调试输出已开启（--debug={} --log-file={} 已存菜单开关={}）",
            options.debug,
            if options.log_file.is_some() {
                "已给"
            } else {
                "未给"
            },
            stored_settings.debug_output,
        )];
        if let Some(path) = &options.log_file {
            first.push(format!("[debug] 调试日志文件: {}", path.display()));
        }
        yeban_app::debug_out::emit(&first);
    }

    // 四条路径，判定顺序是**承重**的：
    //   `--headless-idle` 也在 `batch()` 里（它确实不建 OS 窗口），但它是唯一
    //   **会构造 Slint 对象**的无窗口开关 ⇒ 必须在 `wants_gui()` 之前先分流出去，
    //   而且绝不能落到 `run_batch`（那会静默降级成"不建树"，让 `BASELINE-002` 假绿；
    //   `run_batch` 自己也有一道守卫兜底）。
    //   `--enable-ui-mcp-http` 是**同族但更强**的一档（同一棵活控件树 + 环回控制面），
    //   因此排在 `--headless-idle` 之前：两个开关同时给 = 走 UI 控制面（见 `cli.rs`）。
    //   默认构建里 `ui_mcp_http` 永远为 false（`parse()` 已经把它变成用法错误），
    //   所以这个分支被 `cfg` 掉也不会漏掉任何能跑的组合。
    #[cfg(feature = "ui-mcp-http")]
    if options.ui_mcp_http {
        return cli::finish(yeban_app::ui_mcp_serve::run(&options));
    }
    if options.headless_idle {
        cli::finish(yeban_app::headless_idle::run(&options))
    } else if options.wants_gui() {
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
    let mut loaded = cli::load_project(options)?;
    // ⭐ line/startup-chooser：带 `--open` 启动 = 一次「打开」⇒ 记进
    //    最近清单（启动选择器的「最近」分类读的就是它）。样本形态没有
    //    磁盘路径，不记。写失败只在设置模块里响亮报，⛔ 不挡启动。
    if let Some(path) = &options.open {
        yeban_app::settings::record_recent_project(path, yeban_model::project::now_unix_ms());
    }
    let view = cli::project_view(&loaded.archive.project)?;
    let scene = DemoScene::from_view(&view);

    let mut lines = cli::project_report(&loaded);
    lines.extend(cli::view_report(&view));
    cli::emit(&lines);
    // 调试明细（T-FIX-70）：装载逐项明细紧跟装载报告，只在调试开启时出。
    if yeban_app::debug_out::enabled() {
        yeban_app::debug_out::emit(&cli::project_debug_report(&loaded));
    }

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

    // 主题（`--theme`）：**唯一**的写入口是 `host::apply_theme`，与 `apply_view` 同款
    // 单向注入。放在 `run()` 之前 —— 窗口一旦进了事件循环，任何"换主题"就只是重绘。
    //
    // 默认值 `Theme::default()` = `Theme::Yeban`（CLI `--theme default`，yeban 调色板）
    // 与 `ui/tokens.slint` 里 `ThemeState.theme` 的初值**同一个**（初值也是 `YebanTheme.yeban`，
    // 由 `tests/theme_selection.rs` 的 `the_default_theme_is_the_slint_initial_value` 钉住），
    // 因此不给 `--theme` 时这一步是幂等的。**默认外观因此与 2026-10-07 之前不同**：
    // 5 张 Linux 基准要按手动档 `gates-manual.yml gate=goldens` 重录 + 人工复核（`HD-56`）。
    host::apply_theme(&ui, options.theme);
    // 质感（`--texture`）：与主题同一个写入口纪律（`host::apply_texture`），
    // 同样在 `run()` 之前。默认 `flat` 与 Slint 初值相同 ⇒ 不给开关时幂等。
    host::apply_texture(&ui, options.texture);

    // 语言（`--lang`）：**唯一**的写入口是 `host::apply_language`，紧跟主题之后、
    // 同样在 `run()` 之前。它必须显式调用一次：bundled 词表在首个组件创建时按
    // 系统 locale 自动选语言，而启动语言按优先级定案
    // `--lang`（显式给过才算）＞ 已存选择 ＞ 系统检测 ＞ zh-CN
    // （`settings::resolve_startup_language`，单测钉死）—— 不覆盖就会在
    // 英文系统上首帧英文。运行期切换走主菜单
    // 「设置 → 语言」（host 里接的回调）；`--lang` 只是临时覆盖，不写回。
    let startup_lang = yeban_app::settings::resolve_startup_language(
        options.lang_given.then_some(options.lang),
        yeban_app::settings::load().language,
        yeban_app::settings::detect_system_language(),
    );
    host::apply_language(&ui, startup_lang);

    // 窗口布局（line/ui-window-state）：紧跟语言之后、同样在 `run()` 之前，
    // 把上次退出时存下的布局写回（`host::apply_window_layout` 是唯一写
    // 入口，与同一设置文件的 `layout` 键）。没有已存布局时什么都不做 ——
    // 界面保持 .slint 初值（含 persona 派生的面板显隐初值），不强行覆盖。
    // 写回在事件循环结束时（见本函数末尾）；`--headless-idle` 路径不经
    // 这里 —— 它按场景视口定尺寸做见证测量，恢复真实布局会污染读数。
    //
    // [T-FIX-224] 窗口几何的收口两条路各走一条：
    //   · 有已存几何 ⇒ `apply_window_layout` 内部按**物理**口径夹（那份几何就是
    //     物理像素），夹不进就推回工作区内；
    //   · 没有已存几何（含"存了布局但没存几何"）⇒ 这里按**逻辑**口径把
    //     `.slint` 的 preferred 1600×920 收进工作区。
    // 两条都必须发生在 `run()` 之前：之后再收，用户会先看到一帧溢出。
    match yeban_app::settings::load().layout {
        Some(layout) => {
            let has_geometry = layout.window.is_some();
            host::apply_window_layout(&ui, &layout);
            if !has_geometry {
                host::fit_window_to_work_area(&ui);
            }
        }
        None => host::fit_window_to_work_area(&ui),
    }

    // T-FIX-299：启动期攒下的设置警告（损坏的设置文件、启动时的最近
    // 工程记录失败、帮助状态折入失败）在这里展示。建窗末尾已调过一次
    // （幂等：取走即清空），这里是第二道 —— 覆盖建窗后、布局应用前
    // 这段的读操作可能新记的警告。
    host::report_pending_settings_warnings(&ui);

    // 走带与电平都要有东西可驱动 ⇒ GUI 路径真的建一代引擎（快照 + 无锁事件通道 +
    // 电平 SPSC + 量子驱动）。`ProductionLoop::start` 做两件事，第二件是本票修的缺口：
    //
    // ① 用 0 个量子重建（不空转；`reload` **不改**走带状态），随后显式发一条 `Stop`
    //    把这一代停在 `Stopped`，与界面的初始 `playing: false` 一致 ——
    //    这一步是**引擎侧的动作**（真的过无锁通道、真的在量子边界生效），
    //    不是把界面属性改一下了事；
    // ② **采纳** `reload` 交出的电平消费端（`EngineRebuild::collector`）。
    //    ⛔ 这一条以前被丢掉：`if let Err(error) = engine.reload(..)` 只看了错误那一侧，
    //    于是引擎侧那条 SPSC 没有消费者，界面上的电平表永远不动
    //    （`crate::meters` 的模块文档与 `engine_host.rs:289` 都要求 UI 线程采纳它）。
    //    现在"建引擎 + 采纳消费端 + 每一跳"都住在 [`host::ProductionLoop`] 里，
    //    生产路径与判据走同一段代码。
    //
    // 撤销会话（会话运行态）[ADR-0001 D45 / MODEL-ISO-001]。
    // 打开一个工程 = **新会话**：游标与活跃分支都从头开始，因此撤销不可能跨越打开边界。
    // `history.dag` 的**图谱**恢复由 `--open` 的容器层负责，这里只接当前这一份工程。
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });
    let undo_source = loaded.undo_source();

    // `[ROAD-M4-001]` 形态 A：把领域 MCP 的环回 HTTP 控制面挂进**本进程**。
    // 两道开关都在里面判：编译期是 `--features in-process-mcp`（这个 `cfg`），
    // 运行期是 `--enable-mcp-http` / `YEBAN_MCP_HTTP=1`（`mcp_mount` 的纯函数）。
    // 返回值必须**活到事件循环结束**（`Drop` 才是停机），所以绑在一个具名局部变量上。
    // 默认构建里这段整个不存在（`cfg`），`parse()` 也已经把"要求开但没编译进来"变成用法错误。
    //
    // `[ROAD-M4-008]` 选项 (a) 第二片：挂载点**提前到构造撤销端口之前** ——
    // 下一步要据"挂上了没有"决定撤销端口的权威是谁。
    #[cfg(feature = "in-process-mcp")]
    let in_process_mcp = mount_in_process_mcp(options, &loaded)?;

    // 撤销端口的**权威**（`ROAD-M4-008` 选项 (a) 第二片）：
    //
    // - 控制面挂上了 ⇒ 端口只持 `ProjectAuthorityHandle`，**不持有任何工程副本**；
    //   于是"人在界面按 `Cmd+Z`/卷帘编辑"与"AI 发 `yeban_undo`/工具调用"改的是
    //   同一个 `Domain`（唯一可变权威），投影也只有一份来源。
    // - 没挂上（运行期开关关着 / `.yeban.lock` 被别的形态持有 / 默认构建）⇒ 沿用
    //   GUI 自己的会话 —— 那时**不存在**第二个写者，所以它并不违反"唯一权威"。
    #[cfg(feature = "in-process-mcp")]
    let undo_port = match &in_process_mcp {
        Some(mount) => Rc::new(UndoPort::from_authority(mount.project_authority())),
        None => Rc::new(UndoPort::new(open_undo_session(
            &undo_source,
            &mut loaded,
            now_ms,
        )?)),
    };
    #[cfg(not(feature = "in-process-mcp"))]
    let undo_port = Rc::new(UndoPort::new(open_undo_session(
        &undo_source,
        &mut loaded,
        now_ms,
    )?));
    // ⭐ T-ISS-MWW4H8J6：装配面读工程的**唯一句柄** —— 会话里那一份（`Local` 后端，
    // 打开时从归档**搬**进来，⛔ 不再整工程克隆）；权威后端（`Authority`）下会话
    // 里没有工程 ⇒ 从 `loaded.archive.project` 另起一份句柄（那一支的写回
    // 仍走控制面的 `Domain`，这份句柄只服务装配期的只读读数与保存回退）。
    // ⚠ `open_undo_session` 之后 `loaded.archive.project` 是默认的空工程 ——
    // 下面**不许**再读它（`grep "loaded.archive.project"` 在这条之后必须为零，
    // 只有 `mount_in_process_mcp` 那一处在会话建立之前、仍可读）。
    let project_rc = match undo_port.project_shared() {
        Some(shared) => shared,
        None => Rc::new(RefCell::new(loaded.archive.project.clone())),
    };
    // ⭐ **会话资产池的装配点**（任务单 `#132`）：`--open` 的容器里若已有
    // `assets/{sha256}`，它们必须活在这一份端口里 —— 否则"打开一个带资产的工程，
    // 再按保存"会把资产池写空（`#132` 的 L2）。录音 / 导入随后往同一个池里加字节。
    // ⚠ 控制面权威形态下保存走权威自己的字节；这里的池只服务本地保存那条腿
    // （`UndoPort` 的文档已登记这条边界）。
    undo_port.replace_assets(loaded.archive.assets.clone());
    // T-FIX-806：启动 `--open` 也是 GUI 打开工程的一条腿 —— 内嵌用户预制必须
    // 与工程一起进运行期端口，否则面板 / 保存侧拿不到副本。
    undo_port.replace_embedded_presets(loaded.archive.embedded_presets.clone());
    // ⭐ 任务单 `#136`：把**音频片段字节的来源**交给引擎宿主 —— 就是上面那一池
    // （`UndoPort::asset_bytes`，⛔ 不复制第二份字节）。来源必须在 `start` **之前**
    // 接上：`reload` 的第 1.3 步要当场给快照注入 `ClipContent::Audio` 的 PCM。
    // 录音（`#132`）与导入落进这一池的后续字节，下一次 edited-publish（`publish_project`
    // 的第 1.3 步）就会读到它 ⇒ "采得到 → 进工程 → 摆上轨道 → 听得到"闭环。
    let audio_asset_port = Rc::clone(&undo_port);
    // ⭐ 任务单 `#168`：`--open` 的那个文件所在目录 = 相对 SFZ 路径的第一个候选根。
    //    必须在 `start` **之前**给出（第一份快照的 SFZ 摊平就在 `reload` 里）。
    //    `ProjectSource::Sample` 没有磁盘对应物 ⇒ `None`（解析从仓库根开始）。
    let project_dir = match &loaded.source {
        cli::ProjectSource::File { path, .. } => path.parent().map(std::path::Path::to_path_buf),
        cli::ProjectSource::Sample(_) => None,
    };
    // ⭐ T-FIX-422：分轨导出的护栏要真实工程文件名（不是 `project.yeban` 合成名）。
    //    启动时把同一个 `ProjectSource` 的路径写下；样本工程为 `None`（导出回退 cwd）。
    let project_path = match &loaded.source {
        cli::ProjectSource::File { path, .. } => Some(path.clone()),
        cli::ProjectSource::Sample(_) => None,
    };
    // ⭐ **缺音源清单**（本票）：与渲染用**同一条**解析（`resolve_sampler_path`），
    // 因此"这里说缺、引擎却读得到"这种自相矛盾不可能出现。只在进界面之前算一次。
    let missing_sounds =
        yeban_app::missing_sounds::collect(&project_rc.borrow(), project_dir.as_deref());
    // ⭐ T-FIX-371：模块入口的路径状态只装一次，换工程时原地更新（⛔ 不重接回调）。
    let module_paths = host::module_paths_for_project(project_dir.as_deref());
    let mut engine = host::ProductionLoop::start_with_audio_source_and_project_dir(
        &project_rc.borrow(),
        0,
        Some(Box::new(move |hash| audio_asset_port.asset_bytes(hash))),
        project_dir.as_deref(),
    )
    .unwrap_or_else(|error| {
        // 引擎建不起来时**出声**：界面照常打开（工程投影本身是好的），
        // 但走带回调会明确报告"没有引擎"，而不是静默地假装在播。
        // ⛔ 这一段与接电平之前**逐字相同** —— 电平腿不许改这条语义。
        //
        // ⭐ **界面也要出声**（本票）：此前这条只走终端（`cli::emit`），GUI 里
        // 一片正常 —— 用户按播放没反应、电平不动，而唯一的原因只写在控制台。
        // 现在同一段原文同时投进既有的「引擎错误横幅」（`engine-error`，
        // 唯一写者仍是 `host::apply_engine_error`，⛔ 不新增第二条通道）：
        // 横幅可关闭、可在无头控件树里按 ID 寻址，读屏也会播。
        let text = format!("走带未接线 —— 引擎快照投影失败: {error}");
        cli::emit(&[format!("yeban-app: {text}")]);
        host::apply_engine_error(&ui, host::EnginePublishOutcome::Failed(text.as_str()));
        host::ProductionLoop::without_engine()
    });
    // ⭐ T-FIX-422：启动装配的真实工程路径（与上面 `project_dir` 同一个来源）。
    if let Some(path) = project_path {
        engine
            .engine_handle()
            .borrow_mut()
            .set_project_path(Some(path));
    }
    if engine.has_engine() {
        // 初始快照**就是**这份工程的投影 ⇒ 记下开局标记，免得第一次心跳白发一份
        // （`reload` 不知道"这一份投影对应哪个撤销标记"，只有装配点知道）。
        // 随后显式发一条 `Stop`：新引擎沿用"自由跑"默认值，而界面的初始态是 `playing: false`。
        engine
            .engine_handle()
            .borrow_mut()
            .mark_applied(EditMark::from_display(&undo_port.display()));
        engine.engine_handle().borrow_mut().stop();
    }
    // -----------------------------------------------------------------------
    // 设备腿（本票）：把这一代引擎真的交给声卡 —— **真实时钟驱动音频线程**
    // -----------------------------------------------------------------------
    //
    // 为什么放在 `stop()` **之后**：`EngineHost::open_device` 把新运行时带到**当前**
    // 走带位置（`Stop` + `SeekTicks(位置)`），因此这一步不会把播放位置重置成 tick 0。
    //
    // 为什么走 `yeban_engine::device` 而不是自己写 cpal 调用：`device.rs` 早就把
    // "枚举能力 → 纯函数协商 → 建流 → 回调体"整条路修好了（三条硬要求见它的模块文档），
    // 本 crate 只做**接线**（D19：app 里一个裸 `cpal::*` 都不出现）。
    //
    // **失败不是致命错误**（`device.rs` 硬要求 #1：绝不 panic）：`DeviceError` 走既有
    // 错误路径 ⇒ 产品退化成"控制面驱动"的形态（无声卡机器与托管 CI 的常态；墙钟泵送
    // 按真实时间推量子，播放头照常走动，只是无声），界面、走带回调、电平表照常工作。
    // `EngineHost::open_device` 的前四步都不动手里那一代，因此这里的 `Err` 分支
    // **一位状态都不需要回滚**。
    //
    // 两处**不实现**的缺口（如实登记，见 `engine_host.rs` 的模块文档）：
    // `[ROAD-M2-001]` 实时线程优先级（自实现要新的 `libc` 依赖 ⇒ 依赖图裁决）；
    // 独占模式（cpal 0.18 没有该 API ⇒ `ShareMode::PreferExclusive` 静默降级为共享）。
    if engine.has_engine() {
        // ⭐ T-FIX-71：把「设置 → 音频」里存过的输出设备选择交给生产循环
        // （`open_device` 按它开；没存过 = `None` = 系统默认设备，现行为）。
        // 指定设备不在场时 `open_device` 会在下面走进 `Err` 分支响亮报错
        // （点名），产品降级为控制面驱动，⛔ 不静默换一台默认设备出声。
        engine.set_audio_output_device(yeban_app::settings::load().audio_output_device);
        let config = yeban_engine::device::EngineConfig::from_project(&project_rc.borrow());
        // 调试明细（T-FIX-70）：打开之前先摊设备枚举（全部输出/输入设备、
        // 默认配置与支持范围），打开之后再摊这一次协商的逐步 trace。
        if yeban_app::debug_out::enabled() {
            yeban_app::debug_out::emit(&yeban_app::debug_out::device_enumeration_lines());
        }
        match engine.open_device(config) {
            Ok(opening) => {
                cli::emit(&[opening.status_line()]);
                if yeban_app::debug_out::enabled() {
                    let trace: Vec<String> = opening
                        .negotiation_trace
                        .iter()
                        .map(|line| format!("[debug] {line}"))
                        .collect();
                    yeban_app::debug_out::emit(&trace);
                }
            }
            Err(error) => {
                cli::emit(&[format!(
                    "音频设备: **未接线**（{error}）⇒ 降级为控制面驱动：墙钟泵送按真实时间推量子（无声）；\
                     界面、走带与电平照常可用"
                )]);
                // 同上一条口径：无声卡 / 打开失败这条**用户能听见差别**的事，
                // ⛔ 不该只留在控制台。走既有 toast（不占错误横幅，因为引擎本身是好的）。
                //
                // ⭐ 文案纪律（2026-11-19 实测）：这里原先直接把 `{error}` 拼进提示，
                // 于是界面上出现的是 `音频设备未接线：本会话无声（audio backend error:
                // The requested audio device is not available…）`——elide 之后**只剩英文后半段**，
                // 用户读到一行后端报错，以为软件坏了。环境降级不是错误：说清「什么受影响、
                // 什么不受影响」就够了，英文原文留在控制台与诊断里。
                // T-FIX-59：采样率不符不是"没有设备"——设备在，只是跑不到工程
                // 采样率。toast 必须点名两侧数值与补救办法，⛔ 不套用通用文案
                // 把用户引去查"设备不存在"。
                // T-FIX-72 起协商已改为"接受设备率＋实时采样率转换"，本错误
                // 不再由协商产生；本分流保留（错误类型仍是公共 API），万一
                // 它从别处冒出来，文案仍是点名两侧数值的正确口径。
                let text = match &error {
                    yeban_app::engine_host::EngineHostError::Device(
                        yeban_engine::device::DeviceError::SampleRateMismatch {
                            want_rate,
                            device_rate,
                        },
                    ) => format!(
                        "音频设备采样率 {device_rate} Hz 与工程 {want_rate} Hz 不一致 · 播放将静音；\
                         请换用支持 {want_rate} Hz 的输出设备，或把工程采样率改为 {device_rate} Hz"
                    ),
                    _ => "没有可用的音频输出设备 · 播放将静音；编辑、渲染与导出不受影响".to_owned(),
                };
                ui.set_toast_message(text.into());
                // 非错误级：中性提示，几秒后自动收走（⛔ 别让它变成常驻红条）。
                ui.set_toast_is_error(false);
                ui.set_show_floating_toast(true);
                // 调试明细（T-FIX-70）：打开失败时句柄不存在、拿不到那一次的
                // trace ⇒ 对默认设备做一次只协商不建流的重演，把协商在哪
                // 一步落到这个结果摊开（重演是纯函数 + 只读枚举，与失败的
                // 那一次同政策同输入，结果一致）。
                if yeban_app::debug_out::enabled() {
                    yeban_app::debug_out::emit(
                        &yeban_app::debug_out::output_negotiation_trace_lines(&config),
                    );
                }
            }
        }
    }
    // ⭐ `#290`：把**场景启动口**接到形态 A 的进程内控制面。
    //
    // 顺序是契约：必须等引擎真的建起来（`reload` 成功）之后再 attach —— 没有引擎时
    // `yeban_launch_scene` 要**响亮**报 `engineUnavailable`，⛔ 不是"先接上再假装"。
    // 句柄在心跳闭包里用：取走 MCP 排队的请求 → `EngineHost::queue_scene_launch`
    // （与界面按钮**同一个**入口）；再回注 `scene_launches_applied` 读数。
    #[cfg(feature = "in-process-mcp")]
    let scene_launch_handle = in_process_mcp.as_ref().map(|mount| {
        let handle = mount.scene_launch_handle();
        if engine.has_engine() {
            handle.attach();
        }
        handle
    });
    #[cfg(feature = "in-process-mcp")]
    let engine_readings_handle = in_process_mcp
        .as_ref()
        .map(yeban_app::mcp_mount::InProcessMcp::engine_readings_handle);
    let engine_handle = engine.engine_handle();
    host::apply_transport(&ui, engine_handle.borrow().transport());
    // -----------------------------------------------------------------------
    // 60Hz 主线程心跳：**编辑 ⇒ 发声** 的发布点 ＋ 退役回收 ＋ **电平抽干**
    // -----------------------------------------------------------------------
    //
    // 这一跳做五件事，缺一不可（实现住在 [`host::ProductionLoop::tick`]，
    // 生产窗口与判据因此走**同一段**代码）：
    //
    // ① 读撤销端口的**模型读数**（`UndoPort::display` 的 `head` / `undone`）：一次提交会
    //    换 `head`，一次撤销 / 重做会改 `undone` ⇒ 这是"工程改过没有"的轻量信号，同时
    //    覆盖 GUI 编辑（推子 / 声相 / 静音 / 独奏 / 卷帘）与挂了控制面时的 AI 编辑
    //    （两种后端都经同一个 `UndoPort::display`，见 `undo::UndoBackend`）。
    // ② 改过 ⇒ `EngineHost::publish_project` **增量**发布：只换快照，不重建槽 / 队列 /
    //    `EngineRuntime`。⛔ 不用 `EngineHost::reload` —— 它会改掉 `ui/reload_engine` 的
    //    `quanta` 契约（既有判据 `admin_reload_engine_*` 钉着那一条）。
    // ③ 不论改没改，`EngineHost::heartbeat` 都要跑：`SnapshotSlot::prune` 释放写者侧
    //    待回收清单、`RetireQueue::drain` 把读者交出的旧快照 **Drop** 掉。
    //    ⛔ 没有 ③，②就是一条"只涨不回收"的慢泄漏。
    // ④ 不论改没改，`host::pump_meters` 也要跑：抽干电平 SPSC → 对齐投影 →
    //    `host::apply_meters` 写 Slint 属性。⛔ 没有 ④，引擎侧的队列没有消费者，
    //    而界面上的电平表永远不动（本票修的就是这一条；消费端的采纳在
    //    [`host::ProductionLoop::start`]）。
    // ⑤ 不论改没改，`host::apply_transport` 也要跑（任务单 `#228`）：把引擎的
    //    `transport()` 读数写回 `playing` ＋ `timecode`。⛔ 没有 ⑤，引擎在播而顶栏的
    //    数字**一位不动** ⇒ 用户以为卡了。`apply_transport` 是这两个属性的唯一写者，
    //    且它在 `drop(engine)` 之后才写界面（⛔ 不持借写界面，`#212` 的崩法）。
    //
    // 为什么 ②③④⑤ 共用**同一个** tick：见 `host::ProductionLoop::tick` 的文档
    // （同为 60Hz 一条腿 ⇒ 拆开只会多一次唤醒，还要另证"电平在快照之后被抽"）。
    //
    // 线程：Slint 事件循环线程 —— 它就是建这一代引擎（= 建退役队列）的那一个线程，
    // 因此 `RetireQueue::release_thread_is_main()` 保持为真、`foreign_drains == 0`
    // [MUST-GATE-012]。音频回调那条读路径（`SnapshotReader::begin_block`）一位没动
    // [ARCH-RT-001 / MUST-GATE-001]：本跳只**读**队列，从不给音频侧加任何东西。
    //
    // 启停：句柄必须活到事件循环结束（drop 掉它 = 取消回调；上游
    // `i-slint-core-1.18.1/timers.rs:253` 的 `impl Drop for Timer` 就是"从定时器表里摘掉
    // 这个槽并 drop 回调"）。窗口关闭 ⇒ `ComponentHandle::run` 返回 ⇒ `run_gui` 返回 ⇒
    // 句柄被 drop ⇒ tick 停 ⇒ **不再 poll**；控制面会话被关掉 ⇒ `try_project()` 读到
    // `None` ⇒ 发布自然停止（回收与电平继续跑到窗口关闭为止）。
    let heartbeat = slint::Timer::default();
    // 长度契约被换工程打破时**只出声一次**（16 ms 一跳的 eprintln 会淹掉终端）。
    let meter_mismatch_warned = Rc::new(std::cell::Cell::new(false));
    // ⭐ **硬件 MIDI 输入的实时腿**：`MidiInState` 由 `host::wire_module_actions` 建（在下面），
    // 而本心跳闭包在这之前就建好了 ⇒ 先用一个共享槽把句柄交出去，装配完成后再填进去。
    // ⛔ 不挪 `wire_module_actions` 的位置（那会改回调注册顺序 ⇒ 可能被后面的 wire_* 覆盖）。
    //
    // 实时腿的形状（见 `crate::midi_in` 的"实时会话"一节）：
    //   工作线程: 会话开着 ⇒ 到达的事件 ⇒ `midi_in::LiveLane`（Send 的控制侧车厢）
    //   UI 线程: 本心跳（60Hz）⇒ `MidiInState::publish_live` ⇒ `EngineHost::send_engine_events`
    //           ⇒ 设备回调（或控制驱动）在下一个块边界让音符**发声**
    let midi_live_state: Rc<RefCell<Option<host::MidiInState>>> = Rc::new(RefCell::new(None));
    // `ProductionLoop` 归心跳闭包所有（`Rc` +1）：闭包被 drop（窗口关闭）即释放，
    // 里面的电平消费端随之 drop ⇒ tick 停 ⇒ **不再 poll**。
    // 走带回调拿的是**同一个** `EngineHost`（`engine_handle()` 只是 `Rc` 计数 +1）。
    let production = Rc::new(RefCell::new(engine));
    // ⭐ **心跳的对齐基准是共享格**（任务单 `#93`）：心跳闭包在**装配期**建好，而工程可以在
    // **运行期**换（`host::wire_open_project`）⇒ 基准必须是"此刻那一份"。
    // ⛔ 用装配期的 `view.clone()` 会让换工程之后的电平表按**旧**工程的轨道数抽帧
    // （`host::MeterPump::LengthMismatch` ⇒ 电平表停在静音值）。
    let view_cell: Rc<RefCell<ViewState>> = Rc::new(RefCell::new(view.clone()));
    {
        let production_tick = Rc::clone(&production);
        let port = Rc::clone(&undo_port);
        let ui_tick = slint::ComponentHandle::clone_strong(&ui);
        let view_tick = Rc::clone(&view_cell);
        let warned = Rc::clone(&meter_mismatch_warned);
        let midi_live = Rc::clone(&midi_live_state);
        // ⭐ 停播降频（性能实测 P2-B）：16 ms 心跳在**不播放**时降到 ~20 Hz，
        // 播放时保持 60 Hz（走带/电平要跟手）。空闲 CPU / 风扇的主要来源就是这条
        // 常驻 60 Hz；工程越大（轨数越多）降频收益越大。
        let last_heavy_at = Rc::new(std::cell::Cell::new(std::time::Instant::now()));
        #[cfg(feature = "in-process-mcp")]
        let scene_launch_tick = scene_launch_handle.clone();
        #[cfg(feature = "in-process-mcp")]
        let readings_tick = engine_readings_handle.clone();
        heartbeat.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(HEARTBEAT_PERIOD_MS),
            move || {
                // 停播时每 40 ms 才做一次重活（≈20 Hz）；播放时每跳都做（60 Hz）。
                if !ui_tick.get_playing() {
                    let now = std::time::Instant::now();
                    if now.duration_since(last_heavy_at.get())
                        < std::time::Duration::from_millis(40)
                    {
                        return;
                    }
                    last_heavy_at.set(now);
                } else {
                    last_heavy_at.set(std::time::Instant::now());
                }
                // ⭐ ADR-0076 B1 第二步：波形峰值缓存的泵。空闲时这一步是两次
                // 哈希查表 + 一次借用读（微秒级）；刚解完一批时它会重投影一次。
                host::pump_waveform_analysis(&ui_tick, &port);
                let mark = EditMark::from_display(&port.display());
                let mut production = production_tick.borrow_mut();
                // ⭐ T-FIX-225（`ISS-MWTUUMJU-8BB641`）：**端点看护**。用户插拔耳机、
                // 拔掉 USB 声卡或切到蓝牙之后，系统默认端点变了而这条流还挂在旧端点上
                // （轻则继续往失效端点写 = 静音无报错，重则流被后端作废 = 无声且无提示）。
                // 这里每跳问一次，真正干活被 1 Hz 闸门挡在里面（`poll_endpoint` 会枚举
                // 输出设备，有分配 ⇒ 不能每帧问）；判定需要重建时用**同一份期望**重开
                // 设备腿，走带位置/监听开关由 `open_device_on` 的交接步骤带走。
                production.watch_device(std::time::Instant::now());
                // "没改就不克隆工程"是这一跳的成本契约 ⇒ 先问标记，再决定要不要 `try_project`。
                let project = if production.should_publish(mark) {
                    port.try_project()
                } else {
                    None
                };
                let mut tick =
                    production.tick(&ui_tick, &view_tick.borrow(), mark, project.as_ref());
                // ⭐ **心跳投影自愈**（本轮实测的根因）：`view-tick` 是装配期/上次打开工程时的
                // 那份投影副本，而"新建轨道""控制面改轨道集合"这类编辑只会经
                // `apply_view` 重画界面、**不会**回写本循环手里那份副本（B1 波形与电平腿都读它）。
                // 后果：新增一条轨之后 `pump_meters` 的长度契约永远不成立 ⇒ 混音台电平表
                // **永久**停在静音值（现场实例：Lead/Bass/Aux + 新建的 MIDI 2 共 4 行，
                // 而心跳那份投影仍是 3 条）。
                //
                // 处置：只在**第一次**发现不等长时用权威工程重建一次投影（每"轨数变化"一次
                // 代价，稳态零成本），重建后立刻补跑一跳让电平当帧恢复。重建后仍不等长
                // ⇒ 照旧如实出声（下面的 `LengthMismatch` 日志），⛔ 不静默。
                if let host::MeterPump::LengthMismatch { .. } = &tick.meters {
                    // 缩放取**当前**那份副本的读数（窗口里的刻度没变过 ⇒ 重建不能顺带改缩放，
                    // 否则自动化泳道/波形柱数会跟着跳）。
                    let zoom = view_tick.borrow().ticks_per_pixel;
                    if let Some(fresh) = port
                        .with_project(|project| {
                            ViewState::from_project_with_zoom(project, zoom).ok()
                        })
                        .flatten()
                    {
                        *view_tick.borrow_mut() = fresh;
                        tick =
                            production.tick(&ui_tick, &view_tick.borrow(), mark, project.as_ref());
                    }
                }
                // 新 UI 的显示态(is-playing / timecode / 电平 / 播放头)与
                // 契约面(旧接线写入)对齐; 数据数组同名, apply_view 已直接驱动组件树。
                host::sync_aether_display(&ui_tick);
                // ⭐ **墙钟泵送**（控制面模式）：播放时按真实时间推量子（无声）。
                //
                // 只挂在生产 16 ms 定时器上（⛔ 不进 `ProductionLoop::tick`）：tick 是
                // 确定性的（只由显式泵送推进，既有判据据此钉"没有推量子 ⇒ 位置不变"）；
                // 墙钟泵送是生产才有的真实时间驱动。设备腿恒返回 0（音频时钟负责，
                // 绝不双泵）；停住时返回 0 且不清不欠。返回值刻意忽略（泵送是尽力而为
                // 的推进，不是命令）。
                production.engine_handle().borrow_mut().pump_wall_clock();
                // ⭐ `#290`：**MCP 场景启动**的唯一搬运点（与界面按钮同一个 60Hz 节拍、
                // 同一个 `EngineHost::queue_scene_launch`）。取走即清空；再回注生效计数。
                // ⚠ 在 `tick` **之后**取引擎（`tick` 内部已经借用过它，`#212` 的纪律）。
                #[cfg(feature = "in-process-mcp")]
                if let Some(handle) = scene_launch_tick.as_ref() {
                    let engine = production.engine_handle();
                    let drain = yeban_app::mcp_mount::apply_scene_launch_requests(
                        handle,
                        &mut engine.borrow_mut(),
                    );
                    if let Some(readings) = readings_tick.as_ref() {
                        yeban_app::mcp_mount::publish_scene_launches_applied(
                            readings,
                            &engine.borrow(),
                        );
                    }
                    if drain.refused > 0 {
                        eprintln!(
                            "[yeban-app] MCP 场景启动：{} 条没有送到引擎（见上面的逐条日志）",
                            drain.refused
                        );
                    }
                }
                // ⭐ **stage 29**：PDC 三读数的搬运点（同一拍、同一个 `EngineHost`）。
                // 与场景启动分开：它不依赖场景启动口是否接上 —— 只要控制面在，
                // `engine.pdcChainCompensationMax` / `pdcCompensatedNodes` /
                // `pdcEdgeCompensationMax` 就该每拍刷新（没有引擎时如实注入 `None`）。
                #[cfg(feature = "in-process-mcp")]
                if let Some(handle) = readings_tick.as_ref() {
                    let engine = production.engine_handle();
                    yeban_app::mcp_mount::publish_performance_ledger(handle, &engine.borrow());
                }
                // ⭐ **实时腿的发布点**（⭐ 与"编辑 ⇒ 发声"共用同一个 60Hz 节拍、同一个
                // `EngineHost`）：会话线程在窗口内采到的事件在这里进引擎的音符路径。
                // ⚠ 车道空时是一次 `Mutex` 判空（⛔ 不进音频线程、⛔ 不加任何音频侧工作）；
                // ⚠ 必须在 `tick` **之后**取引擎（`tick` 内部已经借用过它）。
                if let Some(state) = midi_live.borrow().as_ref() {
                    // ⭐ **已配对音符的提交点**（⭐ 每条音符 = 一次提交 = 一步撤销）。
                    // ⚠ 只在**车道非空**时才动工程（车道空时这一跳的成本与接线前相同）。
                    // ⚠ 判据路径（注入替身）`live_note_target` 是 `None` ⇒ 一位不改。
                    // ⭐ T-FIX-355：落点**每一跳重新解析**（用户可能刚切换了选中轨/
                    // 片段）。解析不出来（工程被关、库里没有 MIDI 片段）时才退回装配期
                    // 那一份快照 —— 后者是判据路径与"装配后立刻弹琴"的兜底。
                    let live_target = host::live_note_target_of(
                        Some(port.as_ref()),
                        host::selected_track_entity(&ui_tick),
                    )
                    .or_else(|| state.note_target());
                    if state.note_lane_pending() > 0
                        && let Some((track_id, clip_id)) = live_target
                    {
                        let target = host::LiveNoteCommit {
                            port: &port,
                            track_id,
                            clip_id,
                            now_ms: host::now_ms,
                        };
                        let report = state.commit_live_notes(&target);
                        // ⭐ 提交被模型拒绝时**不再静默**：状态行变红并给累计读数；
                        // ⚠ 拒绝数与车道溢出分开（见 `LiveNoteCommitReport`）。
                        if let Some(status) = host::live_note_commit_status(&report) {
                            eprintln!("[yeban-app] {status}");
                            ui_tick.set_module_midi_in_status(status.into());
                            ui_tick.set_module_midi_in_status_succeeded(false);
                        }
                    }
                    // ⭐ **实时腿的发布点**（⭐ 与"编辑 ⇒ 发声"共用同一个 60Hz 节拍、同一个
                    // `EngineHost`）：会话线程在窗口内采到的事件在这里进引擎的音符路径。
                    // ⚠ 车道空时是一次 `Mutex` 判空（⛔ 不进音频线程、⛔ 不加任何音频侧工作）；
                    // ⚠ 必须在 `tick` **之后**取引擎（`tick` 内部已经借用过它）。
                    let engine = production.engine_handle();
                    state.publish_live(&mut engine.borrow_mut());
                }
                if let host::MeterPump::LengthMismatch { rows, projected } = tick.meters
                    && !warned.get()
                {
                    eprintln!(
                        "[yeban-app] 电平未注入: 界面有 {rows} 条轨道、手里那份投影有 \
                         {projected} 条 —— 控制面换过工程? 电平表停在静音值"
                    );
                    warned.set(true);
                }
            },
        );
    }
    // ⭐ AI 提案抽屉的**来源**：挂了控制面 ⇒ 那句柄（抽屉画权威里真实的未合并清单）；
    // 没挂 ⇒ `none()`（抽屉画"这个装配没有提案来源"那句空态）。两种装配走的是**同一个**
    // 注入函数 ⇒ 界面行为不分叉（`crates/yeban-app/src/proposals.rs`）。
    #[cfg(feature = "in-process-mcp")]
    let proposal_source = match in_process_mcp.as_ref() {
        Some(mount) => {
            yeban_app::proposals::ProposalSource::from_authority(&mount.project_authority())
        }
        None => yeban_app::proposals::ProposalSource::none(),
    };
    #[cfg(not(feature = "in-process-mcp"))]
    let proposal_source = yeban_app::proposals::ProposalSource::none();
    wire_callbacks(
        &ui,
        &production.borrow().engine_handle(),
        // ⭐ 当前工程（录音键用它算**预备拍**）—— 与会话共享同一份（T-ISS-MWW4H8J6）。
        Some(Rc::clone(&project_rc)),
        // ⭐ 录音落进工程的写入面（`#132`）：活会话那个端口（⛔ 不是第二份）。
        Some(Rc::clone(&undo_port)),
        // ⭐ `#283`：场景按钮的投影来源（与心跳同一个 `Rc`）。
        Rc::clone(&view_cell),
        proposal_source.clone(),
    );
    // `[UI-A11Y-002]` §7.2 的**事件源**：BPM 敲入控件里真 `TextInput` 的
    // `preedit-text` / `has-focus` 变化 → `ime-composition-changed` / `ime-focus-changed`
    // → `InputContext`（合成态的唯一载体）。
    //
    // 生产路径上这个状态机**有读者了**（`N2` 裁决 (1)，2026-10-06）：下面的 `wire_keys`
    // 把 `.slint` 的 `FocusScope.key-pressed`（`event.text` + 四个修饰位）交给
    // `input::resolve_logical`，因此 §7.2 的守卫与 §7.1 的快捷键在生产二进制里**真的生效**。
    //
    // 残余（如实登记，见 docs/ledger/app-projection-notes.md 的 `N2` 行）：Slint 不暴露物理键码，
    // 因此这里的绑定是**逻辑键**；"与布局无关的键位"（例如"Z 左边那个键"）只有无头端口的
    // 物理码判据能表达。另外，键盘源需要 Slint 焦点（`forward-focus` 在窗口建立时给到），
    // 焦点落在 BPM 敲入框上时单键快捷键按规范归文本控件，没有"自动回焦"的判据。
    let input = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
    host::wire_input(&ui, Rc::clone(&input));
    // `[ROAD-M4-008]` 选项 (a) 的**最后一处**：产品二进制里的保存入口。
    //
    // 在这之前，保存能力（宿主保存动作 `ProjectAuthorityHandle::save_to` / 本地原子写
    // `save::save_project_file`）只在**测试目标**的装配（`src/live_surface.rs` 的
    // `ui/force_save`）里可达 ⇒ 用户按不到。现在它是 `ui/transport.slint` 的"保存"按钮
    // （`accessible-id: "transport-save-button"`）经 `host::wire_save` 落到
    // `save_action::dispatch_save`：挂了控制面就走那个会话（唯一写者），没挂就走本地原子写。
    //
    // 目标路径由 [`yeban_app::cli::ProjectSource::save_target`] 从 `--open` 的那个文件给出；
    // 样本形态没有磁盘对应物 ⇒ 保存会**如实报错**（不是假成功，也不是猜一个文件名）。
    //
    // 第三个参数是**活会话**：保存按钮每次点击都从它现读"工程 + 提交图谱"，因此写出去的
    // 是此刻的那一版（`SaveStatus.project` 只是拿不到活会话时的兜底 —— 现在它与会话
    // 共享同一份工程，⛔ 不再是装配时刻的快照，T-ISS-MWW4H8J6）。默认构建里
    // 这个端口就是 `open_undo_session` 建的那一个；挂了控制面时它是权威端口的委派者。
    //
    // ⭐ 任务单 `#170`：保存成功 ⇒ `EngineHost.project_dir` 跟着**写出的那个文件**走
    // （`wire_save_with_engine` 的最后一跳）。于是"另存为到另一个目录"之后，新加的相对
    // 路径样本按**新**目录解析 —— 与「打开」那一跳（`PortOpenProject::open_runtime`）
    // 同一条语义。传的是与走带 / 按键**同一个** `Rc<RefCell<EngineHost>>`。
    host::wire_save_with_engine(
        &ui,
        host::SaveStatus {
            target: loaded.source.save_target(),
            project: Rc::clone(&project_rc),
        },
        Some(Rc::clone(&undo_port)),
        // ⭐ `#86` 乙：**产品腿**接真原生对话框。它只在"装配期没给目标路径"时被问到
        // （`--open` / `--save` 那条路一次都不弹窗）—— 判据腿接的是自己的假实现。
        Rc::new(yeban_app::file_picker::NativeFilePicker),
        #[cfg(feature = "in-process-mcp")]
        in_process_mcp
            .as_ref()
            .map(yeban_app::mcp_mount::InProcessMcp::project_authority),
        #[cfg(not(feature = "in-process-mcp"))]
        None,
        Some(Rc::clone(&engine_handle)),
    );
    // `N2` 裁决 (1)：GUI 的逻辑键快捷键 → 界面动作。撤销族经 `undo::dispatch_key`
    // （唯一下发点）落到 `UndoPort`，因此 `Cmd+Z` / `Cmd+Shift+Z` / `Cmd+Shift+H`
    // 与时光机按钮是**同一条链**（ADR-0001 D45 的"人按 `Cmd+Z` 真的能撤销"）。
    // ⭐ 引擎上下文（通行形状）：`engine_handle` 与走带回调拿的是**同一个**
    // `Rc<RefCell<EngineHost>>`（`engine_handle()` 只是 `Rc` 计数 +1）；工程由本层持有
    // （`EngineHost` 没有工程访问器）⇒ ⭐ `Cmd/Ctrl+Alt+N`／`+D` 这两个"要换一代引擎"的
    // 动作才有落点（其它装配传 `None` ⇒ 如实不消费）。
    host::wire_keys(
        &ui,
        Rc::clone(&input),
        Some(Rc::clone(&undo_port)),
        Some(Rc::clone(&engine_handle)),
        Some(Rc::clone(&project_rc)),
    );
    // ⭐ **运行期「打开工程」的可见入口**（任务单 `#93`）—— 本单补的那一格。
    //
    // 缺口（`#86` 乙 如实登记过）：原生对话框那时只接到**已有的**运行期入口
    // （`ui/open_project` / `LiveUi::open_project`）上，而产品 `run_gui` 里**没有任何
    // 可点的入口** —— `--open` 只在启动时读一次，运行期换工程没有实现。
    //
    // 入口 = `ui/console/mixer_console.slint` 的 `module-open-project-button`
    // （控制台 **Tab 1 = 调音台**，与 `#90` 甲的「导出 MIDI」并排）。`MixerConsole` 只在
    // `active-tab == 1` 时可见 ⇒ **默认帧（Tab 0 卷帘）里一个像素都不画** ⇒
    // `[UI-MCP-003]` 的 5 张 Linux 基准逐位不变（本机是 macOS，无法重录）。
    //
    // 这一跳复用**已经存在**的每一件东西（逐条见 `host::wire_open_project` 的文档表）：
    // `FilePicker`（`#86` 乙 的对话框能力面）/ `open::open_project_file`（读盘）/
    // `UndoPort::open_project_session`（换会话 = **新会话**）/ `host::apply_view`（唯一注入点）/
    // `EngineHost::reload`（换代）/ `apply_module_receipt`（**既有**的 `module-status` 通道）。
    // ⛔ 不需要换窗口、⛔ 不需要重新装配工作区：同一个 `Rc<UndoPort>` 被十来处接线共用，
    // 它们都在点击那一刻现读端口 ⇒ 换掉端口里的会话，那一批接线自动跟着换。
    {
        // `after` = **装配方**自己的最后一跳：视图注入与引擎换代已经在
        // `host::PortOpenProject::open_runtime` 里做过（那是固定动作），这里只做三件与
        // **本装配**绑定的事：
        // ① 心跳的对齐基准换成新的投影（⛔ 不换 ⇒ 电平表按旧轨道数抽帧）；
        // ② 重接三处**静态快照**（保存落点 / 走带 / 按键）—— 它们都是装配期的 `clone`，
        //    不重接就会拿着旧工程；保存那一处尤其危险：新工程会被写进**旧**工程的文件；
        // ③ 引擎的"已应用标记"跟上新会话（与装配点那一句同款，免得下一跳白发布一遍）。
        let production_after = Rc::clone(&production);
        // ⭐ 闭包体收成一个**工厂**（2026-11-17，工程模板线）：普通「打开」与
        //    「从模板新建」共用同一份 `after`，唯一差别是 ② 的保存落点 ——
        //    普通打开跟着新文件走（`Some(path)`）；模板副本必须**未命名**
        //    （`untitled = true` ⇒ 落点 `None`，首存走「保存工程」的问路径
        //    流程，**永不**回写模板文件 —— 照 Logic 的副本语义）。其余动作
        //    （① 心跳基准、③ 快照、④ 已应用标记）逐字相同，⛔ 不复制第二份。
        let make_after = |untitled: bool| -> host::OpenProjectAfter {
            let view_after = Rc::clone(&view_cell);
            let engine_after = Rc::clone(&engine_handle);
            let port_after = Rc::clone(&undo_port);
            let module_paths_after = Rc::clone(&module_paths);
            let input_after = Rc::clone(&input);
            #[cfg(feature = "in-process-mcp")]
            let authority_after = in_process_mcp
                .as_ref()
                .map(yeban_app::mcp_mount::InProcessMcp::project_authority);
            let save_picker_after: Rc<dyn yeban_app::file_picker::FilePicker> =
                Rc::new(yeban_app::file_picker::NativeFilePicker);
            // ⭐ 抽屉的来源**不用换**：它拿的是权威的句柄，而换工程不换权威（`Domain` 还是
            // 那一个 `Domain`）。这里只是把同一份读数的第二份克隆交给新那代回调。
            let proposal_source_after = proposal_source.clone();
            Rc::new(
                move |ui: &yeban_app::ui::MainWindow,
                      path: &std::path::Path,
                      view: &yeban_app::bridge::ViewState|
                      -> Result<(), String> {
                    // ⭐ T-FIX-428：`after` 不再拿工程当入参 —— 从端口取此刻那一份，
                    //    取不到如实报错（当前工程一位没动）。
                    let Some(project) = port_after.try_project() else {
                        return Err("打开后端口里没有工程（当前工程一位没动）".to_owned());
                    };
                    // ⭐ T-ISS-MWW4H8J6：与活会话共享同一份工程句柄（保存回退 / 走带 /
                    //    按键读此刻的工程，⛔ 不是装配时刻快照）。
                    let shared = host::shared_project_for_session(&port_after, &project);
                    // ① 心跳对齐基准。
                    *view_after.borrow_mut() = view.clone();
                    // ② 保存落点**跟着新工程走**（`--open` 那个文件 → 这次打开的那个文件）；
                    //    模板副本（`untitled`）例外：落点为空 ⇒ 它是未命名新工程，
                    //    首存走问路径流程，模板文件永不被写。
                    //    ⭐ `#170`：这一跳与「打开」同源 —— 保存成功时 `project_dir` 也跟着
                    //    落点走（`engine_after` 就是走带 / 按键那一代的同一个宿主）。
                    host::wire_save_with_engine(
                        ui,
                        host::SaveStatus {
                            target: if untitled {
                                None
                            } else {
                                Some(path.to_path_buf())
                            },
                            project: Rc::clone(&shared),
                        },
                        Some(Rc::clone(&port_after)),
                        Rc::clone(&save_picker_after),
                        #[cfg(feature = "in-process-mcp")]
                        authority_after.clone(),
                        #[cfg(not(feature = "in-process-mcp"))]
                        None,
                        Some(Rc::clone(&engine_after)),
                    );
                    // ③ 走带 / 按键那两处静态快照（`on_*` setter 覆盖旧闭包）。
                    wire_callbacks(
                        ui,
                        &engine_after,
                        Some(Rc::clone(&shared)),
                        Some(Rc::clone(&port_after)),
                        // ⭐ `#283`：`view_after` 就是上面刚换过的那份投影（同一个 `Rc`）。
                        Rc::clone(&view_after),
                        proposal_source_after.clone(),
                    );
                    host::wire_keys(
                        ui,
                        Rc::clone(&input_after),
                        Some(Rc::clone(&port_after)),
                        Some(Rc::clone(&engine_after)),
                        Some(Rc::clone(&shared)),
                    );
                    // ③-b 最后再装一次生产回调层。`wire_callbacks` 会重跑
                    // `wire_view_callbacks`，其中轨道 MUTE/SOLO/ARM、控制室
                    // DIM/MONO/SIDE、走带等同名回调是 UI-only 版本；Slint 的 on_*
                    // 是覆盖语义，不在这里按启动顺序补回 aether 层，它们会把启动时
                    // 接好的真实引擎/Op handler 静默擦掉（T-FIX-270）。
                    host::rewire_aether_actions_after_project_switch(
                        ui,
                        Rc::clone(&port_after),
                        Rc::clone(&engine_after),
                        Rc::clone(&view_after),
                    );
                    // ③-c 重绑纯编辑面：这些 handler 闭包着启动期 `UndoPort`，
                    // `wire_callbacks` 不含它们；不重跑会把 Op 写进旧撤销会话（T-FIX-323）。
                    host::wire_edit_surfaces(ui, &port_after);
                    // ③-d 模块入口的缺省路径跟着新工程目录走（T-FIX-371）。
                    // 路径是一个共享 `Rc<RefCell<..>>`，回调每次触发现读 ⇒ 原地更新即可，
                    // ⛔ 不用重接出第二套 MIDI 实时线程。
                    host::set_module_project_dir(
                        &module_paths_after,
                        if untitled {
                            None
                        } else {
                            std::path::Path::new(path).parent()
                        },
                    );
                    // ④ 已应用标记（与装配点 `engine_handle().mark_applied(..)` 同款）。
                    engine_after
                        .borrow_mut()
                        .mark_applied(EditMark::from_display(&port_after.display()));
                    Ok(())
                },
            )
        };
        let after = make_after(false);
        // ⭐ 换工程的**唯一装配**（任务单 `#93`）——「打开」与「新建」共用这**一个**
        // `Rc<dyn OpenProjectRuntime>`（新建 = 写一个空容器 + 走同一段换工程路径）。
        let open_runtime: Rc<dyn host::OpenProjectRuntime> = Rc::new(host::PortOpenProject {
            port: Rc::clone(&undo_port),
            production: Some(Rc::clone(&production_after)),
            quanta: 0,
            after,
        });
        host::wire_open_project(
            &ui,
            Rc::new(yeban_app::file_picker::NativeFilePicker),
            Rc::clone(&open_runtime),
            // ⭐ `saveFirst = true`：与 `ui/open_project` 的**默认值**同义 —— 换工程之前
            // 先把当前工程写出去；保存没落盘就**响亮**拒绝（⛔ 不静默丢掉未保存的改动）。
            true,
        );
        // ⭐ **运行期「新建工程」的可见入口**（任务单 `#129`）—— 本单补的那一格。
        //
        // 缺口（`#93` 之后仍然卡着人类实测）：GUI 只能"打开"一个**已经存在**的 `.yeban`；
        // 从零开始时唯一的出路是 CLI `--project-sample empty --save-as <path>`
        // （`#128` 实测 884 B／0 轨；`#161` 起是 1136 B／0 音轨 ＋ 1 主总线；
        // `#215` 起是 1148 B，因为创建时刻不再是 `0`）。
        // 入口 = `ui/console/mixer_console.slint` 的
        // `module-new-project-button`（控制台 **Tab 1 = 调音台**，"打开工程"正下方）⇒
        // `host::wire_new_project`：问一次**同一个** `FilePicker` 的 `choose_save` ⇒
        // `host::create_new_project_file`（`samples::default_project()` ＋
        // `save::save_archive_file`，与 `--save-as` **同一条**写入口）⇒
        // **同一个** `open_runtime`（`#93` 的换工程路径：读盘 → 换会话 → 注入 → 换代 → `after`）。
        // ⛔ `after` 不必复制第二份：新建 = "写一个空文件 + 打开它"，保存落点 / 心跳基准 /
        // 按键快照都由上面那**同一个** `after` 跟着新工程走。
        host::wire_new_project(
            &ui,
            Rc::new(yeban_app::file_picker::NativeFilePicker),
            // 克隆而非移动：同一个 `open_runtime` 还要喂给下面的启动
            // 选择器（line/startup-chooser 的 titled 腿）。
            Rc::clone(&open_runtime),
        );
        // ⭐ **工程模板的两个入口**（2026-11-17，照 Logic 的模板机制）：
        //    文件菜单「从模板新建…」/「另存为模板…」⇒
        //    `host::wire_project_templates`。换工程仍走 `open_runtime` 那一段
        //    固定动作，但装的是**未命名**变体（`make_after(true)`）：模板副本
        //    的保存落点为空 ⇒ 编辑后首存走问路径流程，模板文件一字节不动。
        //    相对路径样本在副本未保存前仍按模板所在目录解析（`open_runtime`
        //    第 ⑤ 步的 `set_project_dir(模板父目录)`，与 Logic 把模板资源
        //    放在模板旁边同义）；首次保存后落点与工程目录一起跟着新文件走。
        let untitled_port: Rc<host::PortOpenProject> = Rc::new(host::PortOpenProject {
            port: Rc::clone(&undo_port),
            production: Some(Rc::clone(&production_after)),
            quanta: 0,
            after: make_after(true),
        });
        let open_runtime_untitled: Rc<dyn host::OpenProjectRuntime> = untitled_port.clone();
        let template_roots =
            yeban_app::templates::TemplateRoots::production().map_err(cli::CliError::Usage)?;
        host::wire_project_templates(
            &ui,
            Some(Rc::clone(&undo_port)),
            open_runtime_untitled,
            template_roots.clone(),
        );
        // ⭐ **启动工程选择器**（line/startup-chooser，学 Logic
        //    「Choose a Project」的内容与思想）：无 `--open` 启动 ⇒
        //    先出选择器，占位工程（默认样本）只作装配底座，用户选定
        //    后经上面同一批换工程实现换入，⛔ 不新造第二条打开路径。
        //    带 `--open` ⇒ 路由为 Direct，这里整段跳过、直进工程。
        //    Details 初值取自占位工程（= 规范空工程的出厂字段）。
        if options.startup_route() == cli::StartupRoute::Chooser {
            host::wire_startup_chooser(
                &ui,
                Rc::new(yeban_app::file_picker::NativeFilePicker),
                Rc::clone(&open_runtime),
                Rc::clone(&untitled_port),
                template_roots,
                yeban_app::startup_chooser::NewProjectDetails::from_project(&project_rc.borrow()),
            );
            ui.set_startup_chooser_open(true);
        }
    }
    // ⭐ **运行期「导入音频」的可见入口**（控制台 Tab 1 的素材池面板）：问一次对话框 ⇒
    // `crate::import_audio::import_audio_file`（经同一个 `UndoPort`，来源 `OpOrigin::Import`
    // ⇒ 可撤销）⇒ 资产字节登记进会话池 ⇒ 重投影 ⇒ `module-status` 回执。
    // 原生预制库（T-FIX-77）：列表 / 应用 / 另存 / 旁通四个入口共用
    // `host::wire_preset_library`；与 MCP 工具同一份 `preset` 构造层。
    host::wire_preset_library(&ui, Some(Rc::clone(&undo_port)));
    host::wire_import_audio(
        &ui,
        Rc::clone(&undo_port),
        Rc::new(yeban_app::file_picker::NativeFilePicker),
    );
    // `[UI-NOTE-003]` §3.3 工具矩阵的**点击**入口（台账 R16）：界面报行号，宿主把它送进
    // **同一个** `Action::SelectTool` 臂（与上面 `wire_keys` 的 `1`..`5` 数字键同一条链）。
    // 因此"点工具按钮"与"按数字键"不可能分叉。
    host::wire_tool_select(&ui, Some(Rc::clone(&undo_port)));
    // 卷帘网格选择（1/1..1/64 · 附点 / 三连音）：视图态 + 唯一 tick 换算。
    host::wire_roll_grid(&ui);
    // 撤销的两条界面入口（弹窗开关 + "撤销一步"按钮）都汇到**同一个** `UndoPort`。
    host::wire_undo(&ui, &undo_port);
    // 纯编辑面统一接线（T-FIX-323）：启动与换工程 `after` 共用同一份清单。
    host::wire_edit_surfaces(&ui, &undo_port);
    // **四个模块入口**（导出 WAV / 导入 MIDI / 装载 SFZ / 录音）：与其余接线同款 ——
    // `host::wire_module_actions` 是它们的**唯一**实现，无头端口的装配
    // （`src/live_surface.rs` 的 `build_live_ui_with`）调的是**同一个**函数。
    // 四条回调各自落到**已经存在**的模块（`export_wav` / `import_midi` / `sfz_load` /
    // `record_wav`），⛔ 不另写一套。目标路径由宿主解析
    // （`ModulePaths::from_env_for_project`：`export_wav` 缺省取工程目录，见 P2-3）。
    let midi_state = host::wire_module_actions_for_project_shared(
        &ui,
        Some(Rc::clone(&undo_port)),
        Rc::clone(&module_paths),
    );
    // ⭐ 钢琴卷帘右键菜单的人性化 / 量化对话框 —— 与上面 `wire_module_actions`
    // 同一条接线（无头装配在 `live_surface.rs` 的同一处调它）。
    host::wire_humanize_dialog(&ui, Some(Rc::clone(&undo_port)));
    // ⭐ 鼓机生成对话框（line/drum-machine-p1）：「编辑 → 鼓机生成 (Drums)...」。
    host::wire_drum_gen_dialog(&ui, Some(Rc::clone(&undo_port)));
    // ⭐ 音源管理对话框（line/sound-library-manager）：主菜单「设置 →
    // 音源管理...」的接线（打开才拉目录，⛔ 启动不联网）。
    host::wire_sound_library_dialog_with(&ui, Some(Rc::clone(&undo_port)));
    // ⭐ 自动化钩子（与 `YEBAN_UI_EXPORT_WAV_PATH` 同一类约定）：设了
    // `YEBAN_UI_OPEN_SOUND_LIBRARY=1` ⇒ 启动即打开音源管理。
    // 用途只有一个：截图 / 判据要看到**这个面板**，而 Slint 窗口收不到合成点击
    // （`PostMessage` 点击在本仓试过两轮，不生效）。⛔ 不改变任何默认行为。
    if std::env::var("YEBAN_UI_OPEN_SOUND_LIBRARY")
        .map(|value| value.trim() == "1")
        .unwrap_or(false)
    {
        ui.invoke_open_sound_library();
    }
    // ⭐ **缺音源的救援面板**（Wang 2026-11-19 口径）：打开工程时若某条轨的音源
    // 没装好，不再"直接把人扔进音源管理再自己找" —— 而是先摆出**救援面板**：
    // 每条轨给 3 个同类候选（已装的点一下就换、没装的点一下去下载），
    // 外加"下载原音源 / 先不管"。真正的换绑仍走混音台同一条提交路径。
    // ⭐ 自动化钩子（截图用）：`YEBAN_UI_RESCUE_DOWNLOAD=1` ⇒ 启动后替用户**点一次**
    // 救援面板第一行的「下载原音源」（走的就是那个按钮的同一个 callback，⛔ 不是另写一条
    // 旁路）。用途：截图要看见"面板让位、音源管理在前台"的现场，而 Slint 窗口收不到合成点击。
    let rescue_first_pack: Option<(String, String)> = missing_sounds.first().and_then(|row| {
        row.pack_id
            .clone()
            .map(|id| (id, row.pack_name.clone().unwrap_or_default()))
    });
    if !missing_sounds.is_empty() {
        host::wire_missing_sounds_dialog(&ui, Rc::clone(&undo_port), missing_sounds);
    }
    if std::env::var("YEBAN_UI_RESCUE_DOWNLOAD")
        .map(|value| value.trim() == "1")
        .unwrap_or(false)
        && let Some((pack_id, pack_name)) = rescue_first_pack
    {
        ui.invoke_missing_open_pack(0, pack_id.into(), pack_name.into());
    }
    // ⭐ **③ 选 SFZ 路径 —— 原生对话框腿**（任务单 `#86` 乙）：混音台通道条的"…"按钮
    // ⇒ 系统文件对话框 ⇒ 把选中的路径**填进** `track-{i}-sampler-path` 那个文本框。
    // ⛔ 它**不**提交工程内容：提交仍是既有的 SET / 回车那一处
    // （`host::wire_mixer_edit` ⇒ `wire_mixer_sampler`）⇒ 文本框与 CLI 两条路一位没变。
    host::wire_mixer_sampler_browse(&ui, Rc::new(yeban_app::file_picker::NativeFilePicker));
    // ⭐ `[PATCH UI-WIRING-01]`：**WAV 导出选项框的 `Ctrl/Cmd+O` 路径浏览** —— 同一个
    // `FilePicker` 能力面的产品腿。选中路径只写回 `export-wav-path`（两向绑定），
    // ⛔ 不写盘、⛔ 不跳过「导出」钮；与上面 SFZ 采样器那条逐字同款。
    host::wire_export_wav_browse(&ui, Rc::new(yeban_app::file_picker::NativeFilePicker));
    // ⭐ T-FIX-71：「设置 → 音频」面板接线 —— 输出/输入设备选择（落盘到
    // 设置文件）、实际设备状态行、播放测试音、面板期间的输入电平。
    // 输出选择的活值住在 `production` 里（与启动时从设置读入的是同一个）。
    host::wire_audio_settings(&ui, Rc::clone(&production), Rc::clone(&undo_port));
    // ⭐ 把刚建好的实时腿句柄填进心跳的共享槽（见上面 `midi_live_state` 的注释）：
    // 从这一刻起，硬件 MIDI 会话在窗口内采到的事件就会在 60Hz 心跳里进引擎的音符路径。
    // ⭐ 实时腿的落点（音轨 ＋ 片段）也一并记下：提交音符时按它构造 `Op::AddNote`。
    // ⚠ 上面那一步已经解析过（`wire_module_actions_with_capture` 里同一处），这里只是取回。
    *midi_live_state.borrow_mut() = Some(midi_state);
    // 启动时先把**模型读数**注入一次（显示态的唯一来源）。
    host::apply_undo(&ui, &undo_port);
    // 生产桥接层: 放在全部 wire_* 之后, 用 on_* 覆盖语义接管新 UI 独有回调,
    // 转发到已接线的旧回调 / 真实 Op。测试装配(只 build_main_window)不跑这一段。
    host::wire_aether_actions(
        &ui,
        Rc::clone(&undo_port),
        production.borrow().engine_handle(),
        Rc::clone(&view_cell),
    );
    // ⭐ T-FIX-208：退出 / 关窗前的「未保存改动」保护（数据丢失级）。
    // 必须在 wire_aether_actions（app-quit 臂）与 wire_save_with_engine（保存记账）
    // 之后装配：它安装保存点初值 + 三个三选 callback + 窗口 close 拦截。
    // 保存点初值 = 此刻的 head（从文件打开的工程即"已保存"态），启动瞬间不弹框。
    host::wire_quit_guard(&ui, &undo_port);

    // 新手引导首次启动自动打开（line/onboarding-help）：只在真实装配里判，
    // 无头 / 测试装配不跑这一句；落盘状态说弹过或走完过就不再弹。
    let _ = yeban_app::help::show_onboarding_if_first_launch(&ui);
    let undo_display = undo_port.display();
    cli::emit(&[format!(
        "撤销: 可撤销 {} 步 · 已撤销 {} 步 · 分支 {} · 实现 = undo_session (与 MCP 的 yeban_undo 同一份源码)",
        undo_display.undoable, undo_display.undone, undo_display.branch
    )]);

    // `[ROAD-M4-008]` 选项 (a) 第 (b) 项：**生产窗口的运行期重投影钩子**。
    //
    // 上面接好的全部写入口只覆盖"人在界面上动手"这一侧。会话侧（AI 经环回控制面发
    // `tools/call`）改的是**同一个** `Domain`，但它不经过任何 GUI 回调 ——
    // 没有这一步，生产窗口就会一直画着打开时那一版工程。
    //
    // 形态（**不是**定时器、**不是**后台线程）：`yeban-mcp` 的 `HttpServer::respond`
    // 在"这次请求真的推进了施加修订号"时、**释放分发器锁之后、写出响应之前**调用
    // 装上去的那个 `Fn(u64)`；观察者再把重投影 marshal 到 UI 线程
    // （`slint::invoke_from_event_loop`）。判据因此可以确定性地驱动它：
    // "客户端收到响应"蕴含"通知已经发生"，接下来跑掉事件循环里那一条排队调用即可。
    //
    // 返回值必须**活到事件循环结束**（它持有窗口与权威的弱引用 + 投影游标）。
    #[cfg(feature = "in-process-mcp")]
    let _reprojection = match in_process_mcp.as_ref() {
        Some(mount) => {
            let mirror = yeban_app::reproject::AuthorityMirror::install(&ui, mount);
            cli::emit(&[match &mirror {
                Some(mirror) => format!(
                    "重投影: 已装 (会话侧改工程 ⇒ UI 线程重投影; 起点修订号 {})",
                    mirror.projected_revision()
                ),
                None => "重投影: **未装** (控制面已停机) —— 会话侧改动不会自动刷新界面".to_owned(),
            }]);
            mirror
        }
        None => None,
    };

    // 用 UFCS 而不是 `ui.run()`: `run()` 是 `slint::ComponentHandle` 的**trait 方法**,
    // 直接调用要求该 trait 在作用域内; 而显式 `use slint::ComponentHandle;` 在生成代码
    // 恰好把它带进作用域时会变成 unused import, 直接撞上 `-D warnings`。
    // UFCS 两种情况下都成立, 且不需要任何 import。
    match slint::ComponentHandle::run(&ui) {
        Ok(()) => {
            // ⭐ T-FIX-408：事件循环结束是最后一道出口。菜单退出 / 窗口关闭都已先
            // stop 落盘；这里再兜一次，覆盖平台直接结束循环、close 回调未走的路径。
            // 已经停过就是 no-op，不会写第二个文件。
            let _ = host::finish_recording_before_exit(&ui);
            // 布局写回（line/ui-window-state）：事件循环正常结束 = 关窗 /
            // 退出，此刻窗口虽已隐藏，尺寸、位置与面板属性仍可读，快照
            // 存回设置（语言字段由设置的读-改-写保住）。异常退出不写回。
            host::save_window_layout(&ui);
            Ok(Vec::new())
        }
        Err(error) => Err(cli::CliError::Ui {
            detail: format!("事件循环异常退出: {error}"),
        }),
    }
}

/// 打开 GUI **自己的**撤销会话（`ROAD-M4-008`：只在没有挂载控制面时用它）。
///
/// 与 `yeban-mcp` 的 `yeban_undo` 是**同一份源码**（`#[path]` 引入 `undo_session.rs`），
/// 因此模型侧的提交 / 逆操作只有一份实现；差别只在"谁是权威"：
/// 挂上了控制面时权威是那个 `Domain`（[`UndoPort::from_authority`]），没有时是这一份。
///
/// ## `history.dag`：读的一侧（与保存那一侧成对）
///
/// 保存路径把活会话的图谱写进容器（`host::wire_save`），因此打开必须把它**读回来**，
/// 否则"撤销 → 保存 → 重开"之后历史就断了 —— 这正是本函数此前只接工程、不接图谱
/// 留下的缺口（`--open` 的容器层已经保真读出 `archive.history_dag`）。
///
/// 判定用与 `yeban-mcp` 同一份口径（[`yeban_model::decode_history_dag`]）：
/// 空条目 ⇒ 新会话（一条根提交）；合法图谱 ⇒ `open_with_graph`（它自己再拒"没有 `main`
/// 分支"的半个历史）；非法 JSON / 不自洽的图谱 ⇒ **如实报错**，⛔ 不降级成空历史
/// （降级等于把用户的历史悄悄丢掉 —— 与 `store.rs` 的拒绝口径一致）。
///
/// # Errors
///
/// 根提交被模型拒绝（容器版本门 / 结构校验不通过），或 `history.dag` 不是合法的
/// `CommitGraph`（`ARCH-OPS-002`）。
fn open_undo_session(
    source: &str,
    loaded: &mut cli::Loaded,
    now_ms: u64,
) -> Result<UndoSession, cli::CliError> {
    // 实现与报文住在 [`yeban_app::undo::open_undo_session`]（服务端测试装配
    // `ADR-0057` A 共用同一条）；这里只转交，不再各自造会话。
    yeban_app::undo::open_undo_session(source, loaded, now_ms)
}

/// `[ROAD-M4-001]` 形态 A 的运行态挂载（**只在 `--features in-process-mcp` 下存在**）。
///
/// 这里刻意只做三件事，判定与实现都住在 [`yeban_app::mcp_mount`]：
///
/// 1. 把命令行开关与环境变量合成一个布尔（[`yeban_app::mcp_mount::switch_requested`]）；
/// 2. 交给 [`yeban_app::mcp_mount::InProcessMcp::start_for_project`] ——
///    **开关关着时它返回 `Ok(None)`，连分发器都不构造**（没有 socket、没有令牌、没有线程）；
///    真工程文件还会以**共享读锁**参与 `.yeban.lock` 跨形态互斥（`ROAD-M0-007`），
///    拿不到锁 ⇒ `MountError::Locked` ⇒ **只拒绝控制面，界面照常运行**；
/// 3. 开启成功时把令牌按 `[ARCH-SEC-002]` 落到 `~/.yeban/session.token`（`0600`），
///    报告**端点与令牌文件路径**，绝不打印令牌本身。
///
/// 令牌落盘失败 ⇒ **停掉控制面**并如实报告：一个外部 Agent 拿不到令牌的监听口
/// 只是多出来的攻击面，没有存在的理由（fail-closed）。
///
/// # Errors
///
/// 绑定 / 注入工程失败（`CliError::Ui`，退出码见 [`cli::CliError::exit_code`]）。
#[cfg(feature = "in-process-mcp")]
fn mount_in_process_mcp(
    options: &Options,
    loaded: &cli::Loaded,
) -> Result<Option<yeban_app::mcp_mount::InProcessMcp>, cli::CliError> {
    use yeban_app::mcp_mount;

    let requested = options.enable_mcp_http || mcp_mount::switch_from_env();
    // 会话来源决定要不要参与 `.yeban.lock` 跨形态互斥（`ROAD-M0-007`），以及**谁是写者**
    // （`ROAD-M4-008` 选项 (a) 的单一写者会话）：
    //
    // - 磁盘上的真工程文件 ⇒ [`mcp_mount::SessionSource::WritableFile`]：GUI 是这份文档的
    //   编辑者，因此那个会话取**排他写**锁、`read_only = false` ⇒ 它是**唯一**磁盘写者
    //   （宿主保存动作 `ui/force_save` 走它；别的形态在它存活期间拿不到这个文件）；
    // - 样本 / 未落盘会话 ⇒ 没有锁可取，也不落盘。
    let source = match &loaded.source {
        cli::ProjectSource::File { path, .. } => {
            mcp_mount::SessionSource::WritableFile(path.clone())
        }
        cli::ProjectSource::Sample(sample) => mcp_mount::SessionSource::InMemory(
            std::path::PathBuf::from(format!("sample:{}", sample.name())),
        ),
    };
    let mounted = match mcp_mount::InProcessMcp::start_for_project(
        requested,
        loaded.archive.project.clone(),
        source,
        // ⭐ `#292`：**会话 CAS 池跟着工程一起进控制面**。少了这一句，导入过音频的
        // 工程经内嵌控制面导出时片段会被当静音（`unsupported: ["audioClips"]`）。
        // 来源与保存路径读的是**同一份** `ProjectArchive::assets`（`#132` 的三层之一）。
        loaded.archive.assets.clone(),
    ) {
        Ok(mounted) => mounted,
        // 工程被别的形态排他持有 ⇒ **只**拒绝控制面，界面照常启动。
        // 界面是主功能；fail-closed 的方向是"少一个监听口"，不是"GUI 起不来"。
        Err(error @ mcp_mount::MountError::Locked { .. }) => {
            cli::emit(&[format!("mcp-http: {error} —— 控制面未挂载, 界面照常运行")]);
            return Ok(None);
        }
        Err(error) => {
            return Err(cli::CliError::Ui {
                detail: format!("[ROAD-M4-001] 进程内 MCP 控制面挂载失败: {error}"),
            });
        }
    };
    let Some(mount) = mounted else {
        // 运行期开关关着 ⇒ 什么都不建（这就是"默认不监听"的可观察形态）。
        return Ok(None);
    };

    cli::emit(&[format!(
        "mcp-http: 进程内环回控制面已监听 {} (只绑环回; 会话按{}注入)",
        mount.endpoint(),
        if mount.project_authority().is_writable() {
            "单一写者 (取排他写锁, GUI 保存经它落盘)"
        } else {
            "只读"
        }
    )]);
    match mcp_mount::publish_token(mount.token()) {
        Ok(path) => {
            cli::emit(&[format!(
                "mcp-http: 会话令牌已写 {} (权限 0600, 内容不打印)",
                path.display()
            )]);
            Ok(Some(mount))
        }
        Err(error) => {
            // fail-closed：令牌没落盘 ⇒ 没人能鉴权 ⇒ 不留监听口。
            let endpoint = mount.endpoint();
            mount.stop().map_err(|stop_error| cli::CliError::Ui {
                detail: format!(
                    "[ROAD-M4-001] 令牌落盘失败后停机也失败 ({endpoint}): {stop_error}"
                ),
            })?;
            cli::emit(&[format!(
                "mcp-http: 会话令牌落盘失败 ({error}) ⇒ 控制面已停止并释放 {endpoint}"
            )]);
            Ok(None)
        }
    }
}

/// 把 `MainWindow` 的回调接到动作上。
///
/// **走带两条已经真的接线**：
/// `toggle-play` / `stop` → [`host::wire_transport`] → `EngineHost` → 无锁事件通道
/// → `EngineRuntime` 的量子边界。显示态（`playing` / `timecode`）由
/// `host::apply_transport` 从**引擎读数**回写，界面不再自己翻转状态。
///
/// **撤销两条也已经真的接线**（ADR-0001 D45）：`toggle-undo-tree` 与时光机的
/// "撤销一步"按钮由 [`host::wire_undo`] 接到 [`yeban_app::undo::UndoPort`]，
/// 动作真的会走 `CommitGraph::undo_with`，显示态由 `host::apply_undo` 从模型读数回写。
///
/// **视图态八条也已经真的接线**（分类切换、控制台 ⤢、资源条目选中、Session 返回编曲与
/// 剪辑选中陆续落地）：
/// `toggle-view` / `toggle-ai-drawer` /
/// `console-maximize-toggle` / `back-to-arrangement` → [`host::wire_view_callbacks`] →
/// 分别写 `arrangement-view` / `ai-drawer-open` /
/// `console-maximized` / `arrangement-view`；
/// `select-sidebar-category` / `select-sidebar-item` / `sidebar-search-changed`
/// → [`host::wire_sidebar_contents`] →
/// 写 `sidebar-category` / `sidebar-item` **并重注入** `sidebar-items` /
/// `sidebar-empty-reason`（分类 0 = 库扫描、1 = 素材池、2 = 模型缺口；钳位用**当次投影条数**；
/// 第三条是 `D-UI-14` 的搜索框 —— 每敲一个字重算 `bridge::filter_sidebar_content` 后重注入，
/// 宿主⛔ 不写回那个串，字的唯一写者是输入框的两向绑定）；
/// `clip-select` → [`host::wire_clip_scope`] → `selected-clip-ulid`（`back-to-arrangement`
/// 只写 `true`，与 `toggle-view` 同一属性；`clip-select` 先校验身份确实在 `clip-ulids`
/// 里再写，并按新身份重注入卷帘作用域 —— `[UI-NOTE-001]` 的"当前编辑片段"）。
/// 原先写在 `.slint` 里的 `root.* = !root.*` 翻转**已删除** —— 同一条动作只有一个写者。
///
/// 抽屉那**四条**（`open-musical-pr` / `select-ai-proposal` / `accept-ai-proposal` /
/// `reject-ai-proposal`）是这八条里搬出去的：它们现在由
/// [`host::wire_proposal_drawer`] 注册（打开抽屉时要**现读**权威那份未合并提案清单再
/// 注入 —— Slint 的 `on_*` 是覆盖语义，同一个回调只能有一个注册者，因此
/// [`host::wire_view_callbacks`] 里那一处已删）。
///
/// 其余**一条**回调仍然故意什么都不做，只打一行 stderr。原因不是省事，而是
/// "接上也只能是什么都不做"（本仓库的明文纪律：把动作吞掉却什么都不做，比不处理更糟）：
///
/// - `accept-ai-proposal` / `reject-ai-proposal` / `select-ai-proposal`：**这三条已经真的
///   接线**（2026-10-31），因此**不在**下面这份"故意空转"的名单里 —— 它们由
///   [`host::wire_proposal_drawer`] 注册，落点是
///   [`host::resolve_proposal`] → `proposals::ProposalSource::resolve` →
///   `ProjectAuthorityHandle::apply_host(HostAction::{MergeProposal, RejectProposal})`，
///   与 MCP 工具 `yeban_merge_proposal` / `yeban_reject_proposal` 复用**同一个**
///   `Plan::Merge` / `Plan::Reject`（⛔ 不是界面上的第二份合并实现）。
///   这一票之前的历史（登记两次、结论都被推翻）：① 界面上没有提案的表示 ⇒ 2026-10-30
///   接上读面（九个属性由同一个宿主写者从唯一可变权威注入）；② "看得见"仍不等于
///   "能采纳"，因为写入口要携带 `proposalId` 而 `HostAction` 只有 `Undo`/`Redo`/`Commit`
///   三臂 ⇒ 2026-10-31 领域侧长出 `MergeProposal`/`RejectProposal` 两臂 + 界面侧
///   "点一行 = 选中"这一格（选中态是**窗口属性**，不是新造的领域状态，因此不撞
///   `ADR-0005`）。`musical-pr-resolve-available` 的口径在有写入口之外还要求**点选了
///   当前清单里的一行**；不满足时**不画**假控件，逐字上屏 `NO_WRITE_SOURCE_REASON` /
///   `NO_SELECTION_REASON` / 权威拒绝的那句话。
/// - `run-acoustic-diagnosis`：仓库里**没有**声学分析（掩蔽 / 相位 / 动态范围）的实现。
///   最近的既有机制是 `[D56]` 的 `UiAction::ExportDiagnostics`，而 [`host::wire_undo`]
///   已经把它接在**另一个**按钮（`diagnostics-export-action`）上，产物是 env/git/日志的
///   诊断 zip，不是声学读数 ⇒ 接上去等于用一个无关产物消费掉这一次点击。
///   2026-11-25（T-FIX-425，路线 b）：在真实声学分析落地前，两个入口已禁用 ——
///   诊断菜单项 `enabled: false`、AI 轨道诊断键的 `TouchArea` 亦 `enabled: false`
///   （整条 AI 轨道今天还藏在 `if false` 里）；`host::wire_aether_callbacks` 里那条
///   假完成 toast 的注册已删除，本回调仍只打 stderr。接真实分析是另立的功能线，
///   不在本单做；谁真接了就必须把这两处禁用一并解除并更新判据。
///
/// 这条仍然未接的回调登记在 `docs/ledger/feature-alignment.md` 的错位 7。
///
/// 回调跑在 UI 线程上; `[ARCH-TOP-002]` / `[ARCH-RT-001]` 约束的是音频线程,
/// 所以这里的 `eprintln!` 不触碰红线 —— 但接线真实动作时**仍然不许**做长阻塞等待。
fn wire_callbacks(
    ui: &yeban_app::ui::MainWindow,
    engine: &Rc<RefCell<yeban_app::engine_host::EngineHost>>,
    // ⭐ T-ISS-MWW4H8J6：与活会话共享同一份 `Rc<RefCell<…>>`（⛔ 不再整工程克隆）；
    // 只原样转交给 [`host::wire_transport`]（录音键读预备拍）。
    project: Option<Rc<RefCell<yeban_model::YebanProjectV1>>>,
    undo: Option<Rc<yeban_app::undo::UndoPort>>,
    // ⭐ 任务单 `#283`：场景按钮要读**当前投影**（量化档 / 场景 tempo / 拍号）并把
    // "当前场景"写回同一份视图 ⇒ 传的是心跳对齐用的**同一个** `Rc<RefCell<ViewState>>`
    // （⛔ 不是装配期的一份 clone：换工程之后它必须跟着换）。
    view: Rc<RefCell<yeban_app::bridge::ViewState>>,
    // ⭐ AI 提案抽屉的**来源**（权威 or 没有）。生产窗口两条装配路径传的是同一个来源的
    // 两份克隆 —— 抽屉的数据面只有 [`host::wire_proposal_drawer`] 这一个注册者。
    proposal_source: yeban_app::proposals::ProposalSource,
) {
    host::wire_transport(ui, Rc::clone(engine), project, undo.clone());
    host::wire_view_callbacks(ui);
    // ⭐ 左侧资源栏的条目**取数**（分类 / 条目点击）：与无头装配调的是**同一个**
    // `host::wire_sidebar_contents`（换分类 ⇒ 数组跟着换 ⇒ 需要当前投影）。
    host::wire_sidebar_contents(ui, Rc::clone(&view));
    // ⭐ 抽屉：注册 `open` / `select` / `accept` / `reject` 四条回调 ＋ 打开时从权威
    // **现读**未合并提案清单注入（`wire_view_callbacks` 不再碰这些回调 —— Slint 的
    // `on_*` 是**覆盖**语义，只能有一个注册者）。
    //
    // ⚠ 这里原先还各写着一行 `ui.on_accept_ai_proposal(|| trace(..))` /
    // `ui.on_reject_ai_proposal(|| trace(..))`：**必须删掉**，不是清理装饰。Slint 的
    // `on_*` 是覆盖语义 ⇒ 留着就会把 `wire_proposal_drawer` 刚接好的写入口**静默擦掉**，
    // 而日志里那句"未接线"照旧打出来（写面接了、注册被自己盖掉，界面上按下去什么都不发生）。
    host::wire_proposal_drawer(ui, proposal_source);
    // ⭐ 插件与 ARA 面板（T-FIX-83）：设备机架的「插件」入口。扫描 / 渲染
    // 在后台线程，装载走普通 `commit_ops`；没有撤销会话时装载如实拒绝。
    host::wire_plugin_panel(ui, undo.clone());
    host::wire_plugin_jsfx_browse(ui, Rc::new(yeban_app::file_picker::NativeFilePicker));
    // ⭐ `[UI-NOTE-001]` 剪辑选中态 ⇒ 卷帘作用域：与无头装配（`live_surface.rs`）调的是
    // **同一个** `host::wire_clip_scope`（选中是视图态，写属性 + 重注入，⛔ 不 `commit_ops`）。
    host::wire_clip_scope(ui, Rc::clone(&view), undo.clone());
    // ⭐ 点编曲标尺 ⇒ 走带定位：引擎 `Rc` 与 `wire_transport` **同一个**（点标尺与按
    // `Space` 回起点作用在同一条走带上），投影 `Rc` 与 `wire_clip_scope` / `wire_scene_launch`
    // **同一个**（网格右缘要从活投影读，换工程之后跟着换）。⛔ 不提交 `Op`。
    host::wire_ruler_seek(ui, Rc::clone(engine), Rc::clone(&view));
    // ⭐ 整曲多层总览图：面板接进正式应用二进制（与测试台调的是**同一个**
    // `wire_song_overview`）。装配时已经按活工程算过一次真实数据并注入；
    // 换工程会重跑 `wire_callbacks`（`main.rs` 的 `after` 闭包），于是接线 +
    // 刷新各再跑一次 —— 这里的刷新是换工程后的那一次（缓存命中，不重算）。
    // ⭐ Hub #711：OTCP / 相位对齐两框。相位 provider 需要工程目录 ⇒ 在
    // `engine` 被 `wire_song_overview` 消费前先取一份。
    let phase_align_project_dir = engine.borrow().project_dir().map(|path| path.to_path_buf());
    yeban_app::otcp_gui::wire_otcp_dialog(ui, undo.clone());
    yeban_app::phase_align_gui::wire_phase_align_dialog(ui, undo.clone(), phase_align_project_dir);
    yeban_app::song_overview::wire_song_overview(ui, Rc::clone(engine), Rc::clone(&view), undo);
    // T-FIX-429：总览首算已在 `wire_song_overview` 里投给后台线程（回调即返回）；
    // 这里不再同步调 `refresh_song_overview`，否则换工程装配会重新阻塞 UI 线程。
    // ⭐ 章节卡片点击 ⇒ 走带定位到段起点（`U-05`）：与上一行**同一对** `Rc`
    // （同一条走带、同一份活投影），落点也是同一个 `commit_seek`。
    host::wire_section_seek(ui, Rc::clone(engine), Rc::clone(&view));
    host::wire_scene_launch(ui, Rc::clone(engine), view);
    ui.on_run_acoustic_diagnosis(|| trace("run-acoustic-diagnosis"));
}

/// 回调接线点的统一落点。
fn trace(callback: &'static str) {
    eprintln!("[yeban-app] ui callback `{callback}` (未接线: 等待 yeban-engine / yeban-model)");
}
