//! `yeban-app` 可执行入口。
//!
//! ## 三种运行形态
//!
//! | 命令 | 行为 |
//! | :-- | :--- |
//! | `yeban-app` | 把一个 `YebanProjectV1` 投影成 `ViewState`、注入 `MainWindow`、进 Slint 事件循环 |
//! | `yeban-app --headless`（或 `SLINT_BACKEND=headless`） | **不构造任何 Slint 组件**，打印 `headless ok` 后立刻退出 0 |
//! | `yeban-app --dump-elements` / `--print-shortcuts` | 打印语义元素清单 / 快捷键策略表，供 CI 与人类核对 |
//! | `--project-sample <default\|filled>` | 选择驱动界面的工程（默认 `default` = `bridge::demo_project()`） |
//!
//! 三条路径共用同一条数据流：`YebanProjectV1` → `bridge::ViewState` → `host`。因此
//! `--headless` 打印的计数、`--dump-elements` 打印的语义 ID、GUI 画的像素**必然一致**。
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

use std::process::ExitCode;

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::elements::ElementRegistry;
use yeban_app::host;
use yeban_app::input::{InputContext, Modifiers, PhysicalKey};
use yeban_app::scene::{self, DemoScene};
use yeban_model::project::YebanProjectV1;

/// 命令行用法。
const USAGE: &str = "\
夜半 Yeban — Slint 桌面主程序 (由 YebanProjectV1 驱动)

用法:
  yeban-app                      启动 GUI (需要显示器; 会进阻塞事件循环)
  yeban-app --headless           无头握手模式: 不构造窗口, 打印 `headless ok` 后退出 0
  yeban-app --dump-elements      打印语义元素注册表 (每行一个元素, 稳定顺序) [UI-TEST-001]
  yeban-app --print-shortcuts    打印快捷键策略表在本版本的判定结果 [UI-A11Y-001/002]
  yeban-app --project-sample <default|filled>
                                 选择驱动界面的工程样本 (见 crate::bridge::demo_project 与
                                 yeban_model::samples::filled_project); 默认 `default`
  yeban-app --help               本帮助

环境变量:
  SLINT_BACKEND=headless         与 --headless 等价 (yeban 自研哨兵值; Slint 1.18.1 无此后端)

退出码:
  0 成功 / 无头握手完成
  1 窗口创建失败或事件循环异常退出
  2 无法识别的参数
";

/// 驱动界面的工程样本。
///
/// 两个样本走的是**同一条**投影 + 注入路径（`bridge::from_project` → `host::apply_view`），
/// 区别只在"哪个 `YebanProjectV1`"。这就是本工作线的验收形态：
/// 换工程 ⇒ 换像素，中间没有任何"演示数据分支"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sample {
    /// 演示夹具（`bridge::demo_project()`）—— 就是 `scene::*` 常量对应的那个工程。
    Default,
    /// `yeban-model` 的规范级丰富样本（`samples::filled_project()`）。
    Filled,
}

impl Sample {
    /// 构造样本工程。
    fn project(self) -> YebanProjectV1 {
        match self {
            Self::Default => demo_project(),
            Self::Filled => yeban_model::samples::filled_project(),
        }
    }
}

/// 规范 §7.1 的核心快捷键表, 用于 `--print-shortcuts`。
///
/// 每一项都是 (规范表格里的写法, 物理键, 修饰键)。这张表的**意义**是让策略表的判定结果
/// 可被人与 CI 直接阅读 —— 尤其是 `[UI-A11Y-002]` 的 IME 分支。
const KEYMAP: [(&str, PhysicalKey, Modifiers); 18] = [
    ("Space → 播放/暂停", PhysicalKey::Space, Modifiers::none()),
    (
        "Shift+Space → 从光标处续播",
        PhysicalKey::Space,
        Modifiers::shift(),
    ),
    (
        "Tab → 视图切换 (仅画布聚焦)",
        PhysicalKey::Tab,
        Modifiers::none(),
    ),
    ("F5 → Session 视图", PhysicalKey::F5, Modifiers::none()),
    ("F6 → Arrangement 视图", PhysicalKey::F6, Modifiers::none()),
    ("Cmd/Ctrl+Z → 撤销", PhysicalKey::KeyZ, Modifiers::meta()),
    (
        "Cmd/Ctrl+Shift+Z → 重做",
        PhysicalKey::KeyZ,
        Modifiers::ctrl_shift(),
    ),
    (
        "Cmd/Ctrl+Shift+H → 时光机",
        PhysicalKey::KeyH,
        Modifiers::ctrl_shift(),
    ),
    (
        "Cmd/Ctrl+D → 原位复制",
        PhysicalKey::KeyD,
        Modifiers::meta(),
    ),
    (
        "Delete/Backspace → 删除",
        PhysicalKey::Delete,
        Modifiers::none(),
    ),
    ("B → 箭头/铅笔切换", PhysicalKey::KeyB, Modifiers::none()),
    (
        "Cmd/Ctrl+Alt+B → 左抽屉",
        PhysicalKey::KeyB,
        Modifiers::ctrl_alt(),
    ),
    ("Z → 选区撑满视口", PhysicalKey::KeyZ, Modifiers::none()),
    ("Shift+Z → 全曲总览", PhysicalKey::KeyZ, Modifiers::shift()),
    (
        "Cmd/Ctrl+Alt+M → 控制台最大化",
        PhysicalKey::KeyM,
        Modifiers::ctrl_alt(),
    ),
    ("1 → 选择工具", PhysicalKey::Digit(1), Modifiers::none()),
    (
        "Shift+Enter → 采纳 AI 建议",
        PhysicalKey::Enter,
        Modifiers::shift(),
    ),
    ("[ → 试听主线", PhysicalKey::BracketLeft, Modifiers::none()),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let sample = match parse_sample(&args) {
        Ok(sample) => sample,
        Err(message) => {
            eprintln!("yeban-app: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.iter().any(|arg| arg == "--print-shortcuts") {
        print_shortcuts();
        return ExitCode::SUCCESS;
    }
    if wants_headless(&args) {
        return run_headless(&args, sample);
    }
    if let Some(unknown) = first_unknown_arg(&args) {
        eprintln!("yeban-app: 无法识别的参数 `{unknown}`\n\n{USAGE}");
        return ExitCode::from(2);
    }

    run_gui(sample)
}

/// 解析 `--project-sample <default|filled>`；缺省为 [`Sample::Default`]。
fn parse_sample(args: &[String]) -> Result<Sample, String> {
    let mut sample = Sample::Default;
    let mut cursor = 0;
    while cursor < args.len() {
        if args[cursor] == "--project-sample" {
            let value = args
                .get(cursor + 1)
                .ok_or("`--project-sample` 需要一个取值 (default|filled)")?;
            sample = match value.as_str() {
                "default" | "demo" => Sample::Default,
                "filled" => Sample::Filled,
                other => return Err(format!("未知的工程样本 `{other}` (可用: default|filled)")),
            };
            cursor += 2;
            continue;
        }
        cursor += 1;
    }
    Ok(sample)
}

/// 第一个"不是选项、也不是 `--project-sample` 的取值"的参数。
///
/// 为什么不能写成"任何不以 `--` 开头的参数"：`--project-sample filled` 里的 `filled`
/// 也不以 `--` 开头。这里的扫描**跳过被选项消费掉的那一格**，因此
/// `yeban-app filled`（漏了选项名）会正确地被当成无法识别的参数并退出 2，
/// 而不是静默地按默认样本启动 GUI。
fn first_unknown_arg(args: &[String]) -> Option<&String> {
    let mut cursor = 0;
    while cursor < args.len() {
        let arg = &args[cursor];
        if arg == "--project-sample" {
            cursor += 2; // 跳过选项本身与它的取值
            continue;
        }
        if !arg.starts_with("--") {
            return Some(arg);
        }
        cursor += 1;
    }
    None
}

/// 判断是否要求无头运行: `--headless`、`--dump-elements`, 或 `SLINT_BACKEND=headless`。
///
/// `--dump-elements` 也算无头: 它是纯诊断输出, 没有任何理由为它开一个真窗口。
///
/// `SLINT_BACKEND=headless` 是**规范写的字面值**（`[ARCH-UI-003]` / `[UI-TEST-003]`），
/// 但 Slint 1.18.1 不认它。我们保留这个字面值作为 yeban 自己的哨兵, 这样规范里的
/// 命令行原样可跑, 而不是让规范去迁就上游。
fn wants_headless(args: &[String]) -> bool {
    if args
        .iter()
        .any(|arg| arg == "--headless" || arg == "--dump-elements")
    {
        return true;
    }
    matches!(std::env::var("SLINT_BACKEND").as_deref(), Ok("headless"))
}

/// 无头握手：不构造任何 Slint 对象, 不进事件循环。
///
/// 注意它现在走的是**投影路径**：工程 → `ViewState` → 注册表 / 计数。
/// 因此握手行里的数字（轨道数 / 段落数 / 剪辑数 / 音符数）是**工程**的属性，
/// 不是 `.slint` 里的字面量。
fn run_headless(args: &[String], sample: Sample) -> ExitCode {
    let project = sample.project();
    let view = match ViewState::from_project(&project) {
        Ok(view) => view,
        Err(error) => {
            eprintln!("yeban-app: 工程投影失败 ({error}) —— 界面无法由此工程驱动");
            return ExitCode::FAILURE;
        }
    };
    let scene = DemoScene::from_view(&view);
    let registry = ElementRegistry::from_view(&view);

    if args.iter().any(|arg| arg == "--dump-elements") {
        for line in registry.dump_lines() {
            println!("{line}");
        }
    }
    if args.iter().any(|arg| arg == "--print-shortcuts") {
        print_shortcuts();
    }

    // CI 的握手行: 它出现 = 进程真的没进阻塞事件循环。
    // 断言脚本只认这一行; 它必须**恰好**是这个字面值。
    println!("headless ok");

    // 说清无头路径到底证明了什么 —— 不要让 "headless ok" 看着像 "UI 已验证"。
    println!(
        "headless: project={} title={} viewport={}x{} compact={} elements={} dynamic-regions={} tabs={} tracks={} sections={} scenes={} clips={} notes={} bpm={} ts={}",
        view.project_id,
        scene.title,
        scene.viewport_width,
        scene.viewport_height,
        scene.compact(),
        registry.len(),
        registry.dynamic_regions().count(),
        scene::CONSOLE_TABS.len(),
        view.tracks.len(),
        view.sections.len(),
        view.scenes.len(),
        view.clips.len(),
        view.note_ulids.len(),
        view.bpm_display,
        view.time_signature_display,
    );
    println!(
        "headless: 未构造 MainWindow, 未初始化 Slint 后端, 未渲染任何像素 —— 控件树断言需要 crates/yeban-ui-test-port 的 testing backend"
    );
    ExitCode::SUCCESS
}

/// 正常 GUI 路径。
///
/// 数据流：`YebanProjectV1` → [`ViewState`] → [`host::build_main_window`]。
/// 这里**没有**任何"演示数据"分支 —— `--project-sample` 只换工程。
fn run_gui(sample: Sample) -> ExitCode {
    let project = sample.project();
    let view = match ViewState::from_project(&project) {
        Ok(view) => view,
        Err(error) => {
            eprintln!("yeban-app: 工程投影失败 ({error}) —— 界面无法由此工程驱动");
            return ExitCode::FAILURE;
        }
    };
    let scene = DemoScene::from_view(&view);

    let ui = match host::build_main_window(&view, &scene) {
        Ok(ui) => ui,
        Err(error) => {
            eprintln!(
                "yeban-app: 无法创建主窗口 ({error})。无显示器环境请用 `--headless`; \
                 注意 Slint 1.18.1 没有名为 headless 的后端, 详见 src/main.rs 的模块文档。"
            );
            return ExitCode::FAILURE;
        }
    };

    wire_callbacks(&ui);

    // 用 UFCS 而不是 `ui.run()`: `run()` 是 `slint::ComponentHandle` 的**trait 方法**,
    // 直接调用要求该 trait 在作用域内; 而显式 `use slint::ComponentHandle;` 在生成代码
    // 恰好把它带进作用域时会变成 unused import, 直接撞上 `-D warnings`。
    // UFCS 两种情况下都成立, 且不需要任何 import。
    match slint::ComponentHandle::run(&ui) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("yeban-app: 事件循环异常退出: {error}");
            ExitCode::FAILURE
        }
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

/// 打印 `[UI-A11Y-001]` / `[UI-A11Y-002]` 策略表在本版本下的判定结果。
///
/// 同时打印**画布聚焦**与**IME 合成态**两列 —— 这两列必须不同, 而且差异必须是
/// "合成态什么都收不到"。这个输出本身就是给 CI 与人看的一份活文档。
fn print_shortcuts() {
    println!("# 快捷键策略表 (yeban-app scaffold)");
    println!("# 第三列是 [UI-A11Y-002] 的核心: 合成态下所有非 F5/F6 的键都必须被输入法吞掉");
    println!(
        "# {:<34} {:<28} {:<28}",
        "规范写法", "画布聚焦", "文本输入 + IME 合成态"
    );

    let mut canvas = InputContext::new();
    canvas.set_focus(yeban_app::input::Focus::MainCanvas);

    let mut composing = InputContext::new();
    composing.set_focus(yeban_app::input::Focus::TextInput);
    composing.begin_composition();

    for (label, key, modifiers) in KEYMAP {
        println!(
            "  {:<34} {:<28} {:<28}",
            label,
            format!("{:?}", canvas.resolve(key, modifiers)),
            format!("{:?}", composing.resolve(key, modifiers)),
        );
    }
}
