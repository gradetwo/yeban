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
//! （`--features in-process-mcp` + 运行期开关），再据结果构造 `UndoPort` ——
//! 挂上了就用 [`yeban_app::undo::UndoPort::from_authority`]（端口只握
//! `ProjectAuthorityHandle`，写入口落到控制面正在服务的那一个 `Domain`），
//! 没挂上才用 [`yeban_app::undo::UndoPort::new`]（自己的一份会话，那时进程里没有第二个写者）。
//! 因此**默认构建与"控制面关闭"两种形态一位没变**。
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

    // 主题（`--theme`）：**唯一**的写入口是 `host::apply_theme`，与 `apply_view` 同款
    // 单向注入。放在 `run()` 之前 —— 窗口一旦进了事件循环，任何"换主题"就只是重绘。
    //
    // 默认值 `Theme::default()` = `Theme::Yeban`（CLI `--theme default`，yeban 调色板）
    // 与 `ui/tokens.slint` 里 `ThemeState.theme` 的初值**同一个**（初值也是 `YebanTheme.yeban`，
    // 由 `tests/theme_selection.rs` 的 `the_default_theme_is_the_slint_initial_value` 钉住），
    // 因此不给 `--theme` 时这一步是幂等的。**默认外观因此与 2026-10-07 之前不同**：
    // 5 张 Linux 基准要按手动档 `gates-manual.yml gate=goldens` 重录 + 人工复核（`HD-56`）。
    host::apply_theme(&ui, options.theme);

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
    let undo_source = match &loaded.source {
        cli::ProjectSource::File { path, .. } => path.display().to_string(),
        cli::ProjectSource::Sample(sample) => format!("sample:{}", sample.name()),
    };

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
            &loaded,
            now_ms,
        )?)),
    };
    #[cfg(not(feature = "in-process-mcp"))]
    let undo_port = Rc::new(UndoPort::new(open_undo_session(
        &undo_source,
        &loaded,
        now_ms,
    )?));
    // ⭐ **会话资产池的装配点**（任务单 `#132`）：`--open` 的容器里若已有
    // `assets/{sha256}`，它们必须活在这一份端口里 —— 否则"打开一个带资产的工程，
    // 再按保存"会把资产池写空（`#132` 的 L2）。录音 / 导入随后往同一个池里加字节。
    // ⚠ 控制面权威形态下保存走权威自己的字节；这里的池只服务本地保存那条腿
    // （`UndoPort` 的文档已登记这条边界）。
    undo_port.replace_assets(loaded.archive.assets.clone());
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
    let mut engine = host::ProductionLoop::start_with_audio_source_and_project_dir(
        &loaded.archive.project,
        0,
        Some(Box::new(move |hash| audio_asset_port.asset_bytes(hash))),
        project_dir.as_deref(),
    )
    .unwrap_or_else(|error| {
        // 引擎建不起来时**出声**：界面照常打开（工程投影本身是好的），
        // 但走带回调会明确报告"没有引擎"，而不是静默地假装在播。
        // ⛔ 这一段与接电平之前**逐字相同** —— 电平腿不许改这条语义。
        cli::emit(&[format!(
            "yeban-app: 走带未接线 —— 引擎快照投影失败: {error}"
        )]);
        host::ProductionLoop::without_engine()
    });
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
    // 错误路径 ⇒ 产品退化成"控制面显式推量子"的形态（无声卡机器与托管 CI 的常态），
    // 界面、走带回调、电平表照常工作。`EngineHost::open_device` 的前四步都不动手里那一代，
    // 因此这里的 `Err` 分支**一位状态都不需要回滚**。
    //
    // 两处**不实现**的缺口（如实登记，见 `engine_host.rs` 的模块文档）：
    // `[ROAD-M2-001]` 实时线程优先级（自实现要新的 `libc` 依赖 ⇒ 依赖图裁决）；
    // 独占模式（cpal 0.18 没有该 API ⇒ `ShareMode::PreferExclusive` 静默降级为共享）。
    if engine.has_engine() {
        let config = yeban_engine::device::EngineConfig::from_project(&loaded.archive.project);
        match engine.open_device(config) {
            Ok(opening) => cli::emit(&[format!(
                "音频设备: `{}` · {} Hz / {} ch · 请求每回调 {} 帧 (后端接受 Fixed = {}) · \
                 共享模式 {:?} · 交接量子 {} @ tick {} ({:?}) · **真实时钟驱动音频线程**",
                opening.device_name,
                opening.sample_rate,
                opening.channels,
                opening.block_frames,
                opening.fixed_block_accepted,
                opening.share_mode,
                opening.carryover_quanta,
                opening.carryover_position_ticks,
                opening.carryover_state,
            )]),
            Err(error) => cli::emit(&[format!(
                "音频设备: **未接线**（{error}）⇒ 降级为控制面推量子驱动（每次走带动作 1 个量子）；\
                 界面、走带与电平照常可用"
            )]),
        }
    }
    let engine_handle = engine.engine_handle();
    host::apply_transport(&ui, engine_handle.borrow().transport());
    // -----------------------------------------------------------------------
    // 60Hz 主线程心跳：**编辑 ⇒ 发声** 的发布点 ＋ 退役回收 ＋ **电平抽干**
    // -----------------------------------------------------------------------
    //
    // 这一跳做三件事，缺一不可（实现住在 [`host::ProductionLoop::tick`]，
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
    //
    // 为什么 ②③④ 共用**同一个** tick：见 `host::ProductionLoop::tick` 的文档
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
        heartbeat.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(HEARTBEAT_PERIOD_MS),
            move || {
                let mark = EditMark::from_display(&port.display());
                let mut production = production_tick.borrow_mut();
                // "没改就不克隆工程"是这一跳的成本契约 ⇒ 先问标记，再决定要不要 `try_project`。
                let project = if production.should_publish(mark) {
                    port.try_project()
                } else {
                    None
                };
                let tick = production.tick(&ui_tick, &view_tick.borrow(), mark, project.as_ref());
                // ⭐ **实时腿的发布点**（⭐ 与"编辑 ⇒ 发声"共用同一个 60Hz 节拍、同一个
                // `EngineHost`）：会话线程在窗口内采到的事件在这里进引擎的音符路径。
                // ⚠ 车道空时是一次 `Mutex` 判空（⛔ 不进音频线程、⛔ 不加任何音频侧工作）；
                // ⚠ 必须在 `tick` **之后**取引擎（`tick` 内部已经借用过它）。
                if let Some(state) = midi_live.borrow().as_ref() {
                    // ⭐ **已配对音符的提交点**（⭐ 每条音符 = 一次提交 = 一步撤销）。
                    // ⚠ 只在**车道非空**时才动工程（车道空时这一跳的成本与接线前相同）。
                    // ⚠ 判据路径（注入替身）`live_note_target` 是 `None` ⇒ 一位不改。
                    if state.note_lane_pending() > 0
                        && let Some((track_id, clip_id)) = state.note_target()
                    {
                        let target = host::LiveNoteCommit {
                            port: &port,
                            track_id,
                            clip_id,
                            now_ms: host::now_ms,
                        };
                        let _ = state.commit_live_notes(&target);
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
    wire_callbacks(
        &ui,
        &production.borrow().engine_handle(),
        // ⭐ 当前工程（录音键用它算**预备拍**）。
        Some(Rc::new(loaded.archive.project.clone())),
        // ⭐ 录音落进工程的写入面（`#132`）：活会话那个端口（⛔ 不是第二份）。
        Some(Rc::clone(&undo_port)),
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
    // 是此刻的那一版（`SaveStatus.project` 只是拿不到活会话时的兜底快照）。默认构建里
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
            project: loaded.archive.project.clone(),
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
        Some(Rc::new(loaded.archive.project.clone())),
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
        let view_after = Rc::clone(&view_cell);
        let production_after = Rc::clone(&production);
        let engine_after = Rc::clone(&engine_handle);
        let port_after = Rc::clone(&undo_port);
        let input_after = Rc::clone(&input);
        #[cfg(feature = "in-process-mcp")]
        let authority_after = in_process_mcp
            .as_ref()
            .map(yeban_app::mcp_mount::InProcessMcp::project_authority);
        let save_picker_after: Rc<dyn yeban_app::file_picker::FilePicker> =
            Rc::new(yeban_app::file_picker::NativeFilePicker);
        let after = Rc::new(
            move |ui: &yeban_app::ui::MainWindow,
                  project: &yeban_model::YebanProjectV1,
                  path: &str,
                  view: &yeban_app::bridge::ViewState|
                  -> Result<(), String> {
                // ① 心跳对齐基准。
                *view_after.borrow_mut() = view.clone();
                // ② 保存落点**跟着新工程走**（`--open` 那个文件 → 这次打开的那个文件）。
                //    ⭐ `#170`：这一跳与「打开」同源 —— 保存成功时 `project_dir` 也跟着
                //    落点走（`engine_after` 就是走带 / 按键那一代的同一个宿主）。
                host::wire_save_with_engine(
                    ui,
                    host::SaveStatus {
                        target: Some(std::path::PathBuf::from(path)),
                        project: project.clone(),
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
                    Some(Rc::new(project.clone())),
                    Some(Rc::clone(&port_after)),
                );
                host::wire_keys(
                    ui,
                    Rc::clone(&input_after),
                    Some(Rc::clone(&port_after)),
                    Some(Rc::clone(&engine_after)),
                    Some(Rc::new(project.clone())),
                );
                // ④ 已应用标记（与装配点 `engine_handle().mark_applied(..)` 同款）。
                engine_after
                    .borrow_mut()
                    .mark_applied(EditMark::from_display(&port_after.display()));
                Ok(())
            },
        );
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
            open_runtime,
        );
    }
    // `[UI-NOTE-003]` §3.3 工具矩阵的**点击**入口（台账 R16）：界面报行号，宿主把它送进
    // **同一个** `Action::SelectTool` 臂（与上面 `wire_keys` 的 `1`..`5` 数字键同一条链）。
    // 因此"点工具按钮"与"按数字键"不可能分叉。
    host::wire_tool_select(&ui, Some(Rc::clone(&undo_port)));
    // 撤销的两条界面入口（弹窗开关 + "撤销一步"按钮）都汇到**同一个** `UndoPort`。
    host::wire_undo(&ui, &undo_port);
    // `[UI-NOTE-003]` 卷帘编辑入口（铅笔）：与撤销端口共用同一实现 ⇒ 可撤销是构造上的。
    host::wire_roll_edit(&ui, &undo_port);
    // `ADR-0004` S1 的**纵向入口**：轨道头按住并上下拖动 ⇒ 改这一轨的行高。
    //
    // 高度是**视图态**（不进 `.yeban`、不进 schema），因此它落到的不是 `UndoPort::perform`
    // 而是 `host::set_track_height_override`（唯一 setter）+ 用同一个端口重投影；
    // 端口在这里的作用与 `wire_roll_edit` 逐字相同：给出此刻权威的工程。
    // `Escape`（`Action::Cancel`）也经同一个 `wire_keys` 端口取消这一拖（见 `apply_action`）。
    host::wire_track_height_drag(&ui, &undo_port);
    // ⭐ 任务单 `#192`：轨道头点击 ⇒ **选中这条轨**（编辑器视图态）。
    //
    // 与行高同一条纪律：选中态不进 `.yeban`、不进 `projectDigest`、不进撤销栈 ⇒
    // 落点是 `ui.set_selected-track-id`（唯一写点）+ 用同一个端口重投影，
    // ⛔ 不调用 `UndoPort::commit_ops`。端口在这里只用来取"此刻权威的工程"。
    host::wire_track_select(&ui, &undo_port);
    // 混音台（控制台 Tab 2）的**写入面**：推子 / 声相 / 静音 / 独奏。
    //
    // 工程内容是**领域内容** ⇒ 走 `Op` + `UndoPort`（`ADR-0005`）：拖动期间只写视图态，
    // 松手 / 点击才 `commit_ops` **一次**，因此一次拖动 = 一步可撤销的编辑。
    // 与 `wire_roll_edit` 共用同一个 `UndoPort`（唯一可变权威）。
    host::wire_mixer_edit(&ui, &undo_port);
    // ⭐ 设备效果链（控制台 Tab 3）的**写入面**（任务单 `#157`）：旁通开关 + 一格
    // `eq_low_gain` 滑块。与混音台同一条纪律 —— 拖动期只写视图态，松手 / 点击才
    // `commit_ops` **一次**（一次手势 = 一步可撤销的编辑），且共用同一个 `UndoPort`。
    host::wire_device_rack(&ui, &undo_port);
    // ⭐ 发送条（控制台 Tab 3 的同一面板）的**写入面**（任务单 `#159`）：改发送增益 ＋
    // 新建发送。与设备机架 / 混音台同一条纪律 —— 拖动期只写视图态，松手 / 点击才
    // `commit_ops` **一次**，且共用同一个 `UndoPort`（工程内容走 `Op`，`ADR-0005`）。
    host::wire_send_strip(&ui, &undo_port);
    // ⭐ 轨 / 路由（控制台 Tab 3 的**同一面板**）的**写入面**（任务单 `#160`）：
    // 三个建轨按钮 ＋ 删除展示轨。与上面三条同一条纪律 —— 每个手势**恰好一次**
    // `commit_ops`（建轨 / 删轨各自打包成一个 `Op::Batch`），共用同一个 `UndoPort`。
    host::wire_track_strip(&ui, &undo_port);
    // ⭐ 建空的 MIDI 片段（控制台 Tab 3 的**同一面板**，任务单 `#163`）：一次点击 =
    // **恰好一次** `commit_ops`（`AddClip` ＋ `AddClipPlacement` 一个原子批次），
    // 共用同一个 `UndoPort`。设备的增删在同一条面板的 `host::wire_device_rack` 里。
    host::wire_clip_create(&ui, &undo_port);
    // **四个模块入口**（导出 WAV / 导入 MIDI / 装载 SFZ / 录音）：与其余接线同款 ——
    // `host::wire_module_actions` 是它们的**唯一**实现，无头端口的装配
    // （`src/live_surface.rs` 的 `build_live_ui_with`）调的是**同一个**函数。
    // 四条回调各自落到**已经存在**的模块（`export_wav` / `import_midi` / `sfz_load` /
    // `record_wav`），⛔ 不另写一套。目标路径由宿主解析（`ModulePaths::from_env`）。
    let midi_state = host::wire_module_actions(&ui, Some(Rc::clone(&undo_port)));
    // ⭐ **③ 选 SFZ 路径 —— 原生对话框腿**（任务单 `#86` 乙）：混音台通道条的"…"按钮
    // ⇒ 系统文件对话框 ⇒ 把选中的路径**填进** `track-{i}-sampler-path` 那个文本框。
    // ⛔ 它**不**提交工程内容：提交仍是既有的 SET / 回车那一处
    // （`host::wire_mixer_edit` ⇒ `wire_mixer_sampler`）⇒ 文本框与 CLI 两条路一位没变。
    host::wire_mixer_sampler_browse(&ui, Rc::new(yeban_app::file_picker::NativeFilePicker));
    // ⭐ 把刚建好的实时腿句柄填进心跳的共享槽（见上面 `midi_live_state` 的注释）：
    // 从这一刻起，硬件 MIDI 会话在窗口内采到的事件就会在 60Hz 心跳里进引擎的音符路径。
    // ⭐ 实时腿的落点（音轨 ＋ 片段）也一并记下：提交音符时按它构造 `Op::AddNote`。
    // ⚠ 上面那一步已经解析过（`wire_module_actions_with_capture` 里同一处），这里只是取回。
    *midi_live_state.borrow_mut() = Some(midi_state);
    // 启动时先把**模型读数**注入一次（显示态的唯一来源）。
    host::apply_undo(&ui, &undo_port);
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
        Ok(()) => Ok(Vec::new()),
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
    loaded: &cli::Loaded,
    now_ms: u64,
) -> Result<UndoSession, cli::CliError> {
    let graph = yeban_model::decode_history_dag(&loaded.archive.history_dag).map_err(|error| {
        cli::CliError::Ui {
            detail: format!(
                "`{}` 的 `history.dag` 无法恢复（{error}）—— 拒绝猜一个空历史",
                source
            ),
        }
    })?;
    match graph {
        Some(graph) => UndoSession::open_with_graph(
            source,
            "yeban-app",
            loaded.archive.project.clone(),
            graph,
            now_ms,
        ),
        None => UndoSession::open(source, "yeban-app", loaded.archive.project.clone(), now_ms),
    }
    .map_err(|error| cli::CliError::Ui {
        detail: format!("撤销会话无法初始化: {error}"),
    })
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
/// **视图态四条也已经真的接线**（本切片）：`toggle-view` / `toggle-sidebar` /
/// `toggle-ai-drawer` / `open-musical-pr` → [`host::wire_view_callbacks`] →
/// 分别写 `arrangement-view` / `sidebar-collapsed` / `ai-drawer-open` / `musical-pr-open`。
/// `.slint` 里那四处 `root.* = !root.*` 的翻转**已删除** —— 同一条动作只有一个写者。
///
/// 其余**三条**回调仍然故意什么都不做，只打一行 stderr。原因不是省事，而是
/// "接上也只能是什么都不做"（本仓库的明文纪律：把动作吞掉却什么都不做，比不处理更糟）：
///
/// - `accept-ai-proposal` / `reject-ai-proposal`：**提案在界面侧没有表示** ——
///   `ui/dialogs/musical_pr_drawer.slint` 的 `proposal-count` / `proposal-labels` /
///   `proposal-kinds` / `confidences` 默认**全空**（`0` 与四个空数组），并且**没有任何**
///   宿主代码写它们（判据：`elements.rs` 的 `no_host_writes_the_musical_pr_proposal_properties`）。
///   界面在零提案时只画一句可见的空态，**不画**「采纳 / 放弃」两个按钮
///   （一个可点的「采纳」在空列表上是假控件）。
///   **2026-10-08 更正**：这四个属性此前是内联演示常量（三条中文假提案 + 三个假置信度），
///   界面因此显示"AI 提了 3 条"而用户分不出真假；本切片删掉了那批数据。
///   清空数据**不**改变本条结论：界面上仍然没有任何"当前提案身份"的属性，
///   而 `yeban_merge_proposal` / `yeban_reject_proposal` 要 `proposalId`。
///   在界面侧新造一条提案状态会与领域权威冲突（`ADR-0005`）⇒ 登记为缺口，不接。
/// - `run-acoustic-diagnosis`：仓库里**没有**声学分析（掩蔽 / 相位 / 动态范围）的实现。
///   最近的既有机制是 `[D56]` 的 `UiAction::ExportDiagnostics`，而 [`host::wire_undo`]
///   已经把它接在**另一个**按钮（`diagnostics-export-action`）上，产物是 env/git/日志的
///   诊断 zip，不是声学读数 ⇒ 接上去等于用一个无关产物消费掉这一次点击。
///
/// 三条逐条登记在 `docs/ledger/feature-alignment.md` 的错位 7。
///
/// 回调跑在 UI 线程上; `[ARCH-TOP-002]` / `[ARCH-RT-001]` 约束的是音频线程,
/// 所以这里的 `eprintln!` 不触碰红线 —— 但接线真实动作时**仍然不许**做长阻塞等待。
fn wire_callbacks(
    ui: &yeban_app::ui::MainWindow,
    engine: &Rc<RefCell<yeban_app::engine_host::EngineHost>>,
    project: Option<Rc<yeban_model::YebanProjectV1>>,
    undo: Option<Rc<yeban_app::undo::UndoPort>>,
) {
    host::wire_transport(ui, Rc::clone(engine), project, undo);
    host::wire_view_callbacks(ui);
    ui.on_accept_ai_proposal(|| trace("accept-ai-proposal"));
    ui.on_reject_ai_proposal(|| trace("reject-ai-proposal"));
    ui.on_run_acoustic_diagnosis(|| trace("run-acoustic-diagnosis"));
}

/// 回调接线点的统一落点。
fn trace(callback: &'static str) {
    eprintln!("[yeban-app] ui callback `{callback}` (未接线: 等待 yeban-engine / yeban-model)");
}
