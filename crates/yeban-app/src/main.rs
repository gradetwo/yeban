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

use yeban_app::cli::{self, Options};
use yeban_app::engine_host::{EditMark, EngineHost};
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

    // 走带要有东西可驱动 ⇒ GUI 路径真的建一代引擎（快照 + 无锁通道 + 量子驱动）。
    // 用 0 个量子重建（不空转；`reload` **不改**走带状态），随后显式发一条 `Stop`
    // 把这一代停在 `Stopped`，与界面的初始 `playing: false` 一致 ——
    // 这一步是**引擎侧的动作**（真的过无锁通道、真的在量子边界生效），
    // 不是把界面属性改一下了事。
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
    let mut engine = EngineHost::new();
    if let Err(error) = engine.reload(&loaded.archive.project, 0) {
        // 引擎建不起来时**出声**：界面照常打开（工程投影本身是好的），
        // 但走带回调会明确报告"没有引擎"，而不是静默地假装在播。
        cli::emit(&[format!(
            "yeban-app: 走带未接线 —— 引擎快照投影失败: {error}"
        )]);
    } else {
        // 初始快照**就是**这份工程的投影 ⇒ 记下开局标记，免得第一次心跳白发一份
        // （`reload` 不知道"这一份投影对应哪个撤销标记"，只有装配点知道）。
        engine.mark_applied(EditMark::from_display(&undo_port.display()));
    }
    engine.stop();
    let engine = Rc::new(RefCell::new(engine));
    host::apply_transport(&ui, engine.borrow().transport());

    // -----------------------------------------------------------------------
    // 60Hz 主线程心跳：**编辑 ⇒ 发声** 的发布点 ＋ 退役回收（本票补上的那一段）
    // -----------------------------------------------------------------------
    //
    // 这一跳做三件事，缺一不可：
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
    //
    // 线程：Slint 事件循环线程 —— 它就是建这一代引擎（= 建退役队列）的那一个线程，
    // 因此 `RetireQueue::release_thread_is_main()` 保持为真、`foreign_drains == 0`
    // [MUST-GATE-012]。音频回调那条读路径（`SnapshotReader::begin_block`）一位没动
    // [ARCH-RT-001 / MUST-GATE-001]。
    //
    // 启停：句柄必须活到事件循环结束（drop 掉它 = 取消回调）。窗口关闭 ⇒
    // `ComponentHandle::run` 返回 ⇒ `run_gui` 返回 ⇒ 句柄被 drop ⇒ 心跳停止；
    // 控制面会话被关掉 ⇒ `try_project()` 读到 `None` ⇒ 发布自然停止（回收继续跑到
    // 窗口关闭为止）。
    let heartbeat = slint::Timer::default();
    {
        let engine = Rc::clone(&engine);
        let port = Rc::clone(&undo_port);
        heartbeat.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(HEARTBEAT_PERIOD_MS),
            move || {
                let mark = EditMark::from_display(&port.display());
                let mut engine = engine.borrow_mut();
                if !engine.has_engine() {
                    return;
                }
                if engine.should_publish(mark)
                    && let Some(project) = port.try_project()
                    && let Err(error) = engine.publish_project(&project, mark)
                {
                    // 投影失败**不改播放侧**（旧快照原样保留）⇒ 如实出声，不静默。
                    eprintln!("[yeban-app] 快照发布失败（播放侧仍是上一版）: {error}");
                }
                let _ = engine.heartbeat();
            },
        );
    }

    wire_callbacks(&ui, &engine);
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
    host::wire_save(
        &ui,
        host::SaveStatus {
            target: loaded.source.save_target(),
            project: loaded.archive.project.clone(),
        },
        #[cfg(feature = "in-process-mcp")]
        in_process_mcp
            .as_ref()
            .map(yeban_app::mcp_mount::InProcessMcp::project_authority),
        #[cfg(not(feature = "in-process-mcp"))]
        None,
    );
    // `N2` 裁决 (1)：GUI 的逻辑键快捷键 → 界面动作。撤销族经 `undo::dispatch_key`
    // （唯一下发点）落到 `UndoPort`，因此 `Cmd+Z` / `Cmd+Shift+Z` / `Cmd+Shift+H`
    // 与时光机按钮是**同一条链**（ADR-0001 D45 的"人按 `Cmd+Z` 真的能撤销"）。
    host::wire_keys(&ui, Rc::clone(&input), Some(Rc::clone(&undo_port)));
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
    // 混音台（控制台 Tab 2）的**写入面**：推子 / 声相 / 静音 / 独奏。
    //
    // 工程内容是**领域内容** ⇒ 走 `Op` + `UndoPort`（`ADR-0005`）：拖动期间只写视图态，
    // 松手 / 点击才 `commit_ops` **一次**，因此一次拖动 = 一步可撤销的编辑。
    // 与 `wire_roll_edit` 共用同一个 `UndoPort`（唯一可变权威）。
    host::wire_mixer_edit(&ui, &undo_port);
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
/// # Errors
///
/// 根提交被模型拒绝（容器版本门 / 结构校验不通过）。
fn open_undo_session(
    source: &str,
    loaded: &cli::Loaded,
    now_ms: u64,
) -> Result<UndoSession, cli::CliError> {
    UndoSession::open(source, "yeban-app", loaded.archive.project.clone(), now_ms).map_err(
        |error| cli::CliError::Ui {
            detail: format!("撤销会话无法初始化: {error}"),
        },
    )
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
///   `proposal-kinds` / `confidences` 全是内联演示常量（该文件自陈"静态骨架 + 演示数据"），
///   界面上没有任何"当前提案身份"的属性；而 `yeban_merge_proposal` /
///   `yeban_reject_proposal` 要 `proposalId`。在界面侧新造一条提案状态会与领域权威冲突
///   （`ADR-0005`）⇒ 登记为缺口，不接。
/// - `run-acoustic-diagnosis`：仓库里**没有**声学分析（掩蔽 / 相位 / 动态范围）的实现。
///   最近的既有机制是 `[D56]` 的 `UiAction::ExportDiagnostics`，而 [`host::wire_undo`]
///   已经把它接在**另一个**按钮（`diagnostics-export-action`）上，产物是 env/git/日志的
///   诊断 zip，不是声学读数 ⇒ 接上去等于用一个无关产物消费掉这一次点击。
///
/// 三条逐条登记在 `docs/ledger/feature-alignment.md` 的错位 7。
///
/// 回调跑在 UI 线程上; `[ARCH-TOP-002]` / `[ARCH-RT-001]` 约束的是音频线程,
/// 所以这里的 `eprintln!` 不触碰红线 —— 但接线真实动作时**仍然不许**做长阻塞等待。
fn wire_callbacks(ui: &yeban_app::ui::MainWindow, engine: &Rc<RefCell<EngineHost>>) {
    host::wire_transport(ui, Rc::clone(engine));
    host::wire_view_callbacks(ui);
    ui.on_accept_ai_proposal(|| trace("accept-ai-proposal"));
    ui.on_reject_ai_proposal(|| trace("reject-ai-proposal"));
    ui.on_run_acoustic_diagnosis(|| trace("run-acoustic-diagnosis"));
}

/// 回调接线点的统一落点。
fn trace(callback: &'static str) {
    eprintln!("[yeban-app] ui callback `{callback}` (未接线: 等待 yeban-engine / yeban-model)");
}
