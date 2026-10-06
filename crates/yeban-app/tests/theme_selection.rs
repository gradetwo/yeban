//! 主题切换（`--theme` / `--print-theme` / `SLINT_STYLE`）的判据。
//!
//! 2026-10-06，负责人要求「提供切换这几个主题的功能」（Material / Fluent / Cupertino /
//! Native），并指出官方主题比我们手挑的颜色好看。本文件是那条要求的**可失败判据**。
//!
//! ## 为什么单独一个文件（而不是加进 `tests/cli_contract.rs`）
//!
//! `tests/cli_contract.rs` 那一刻正被另一条工作线写着（`git status` 里它是 `M`）。
//! 同一个文件两个写者必然冲突，所以本线的判据全部住在**这个新文件**里。
//!
//! ## 三层判据，各自能独立变红
//!
//! | # | 判据 | 观测面 |
//! | :-- | :--- | :--- |
//! | ① | 每个合法取值都被**接受**（`--theme <v> --print-theme` 与 `--headless`） | 真进程的退出码 + stdout |
//! | ② | 未知取值被**拒绝**：退出码 `2`、stderr 点名原因与可用集合、stdout 为空 | 真进程的退出码 + stderr |
//! | ③ | **生效的调色板真的跟着选择变**：从活的 Slint 组件**回读** `Tokens.*` | 真 `MainWindow` 的 token 值（无像素） |
//!
//! ③ 是这条工作线的核心 —— 因为"换主题"这件事最容易做成"改了命令行、什么都没变"。
//! 它读的是 `ui/tokens.slint` 里那十三支颜色令牌的**实际取值**，不是像素、不是文件内容。
//!
//! ## 为什么 ③ 需要自己装一个平台
//!
//! `host::build_main_window` 会真的构造 `MainWindow`（`ui.window()` 在 `apply_view` 里
//! 被调用），而 `MainWindow::new()` 需要一个窗口适配器 ⇒ 一个平台。CI 没有显示器，
//! 所以这里装的是**自研软件平台**（`MinimalSoftwareWindow` + `SoftwareRenderer`，
//! 来自本 crate 已启用的 `renderer-software` feature ⇒ **零新增依赖**），与
//! `src/headless_idle.rs` 用的是**同一套**上游 API。
//!
//! ⚠ `slint::platform::set_platform` 是**线程局部且每线程只能装一次** —— 因此本文件里
//! **只有 ③ 这一个** `#[test]` 碰 Slint 进程内对象；①② 全部走子进程。
//! 这样无论 `--test-threads=1` 还是并行，都不会出现"平台被装了第二次"。
//!
//! ## 规范来源
//!
//! - `[ARCH-SLINT-001]`：Slint 上游能力核验与自研兜底 —— 本文件把"Slint 1.18.1 没有
//!   运行时选风格 API"这条**核验结论**变成了可执行判据（见 ③ 的 `compiled_slint_style`）。
//! - `AGENTS.md` §3 DoD 6「UI 变更必须双重验证」：③ 是无头控件属性断言那一半。

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use slint::{Color, ComponentHandle as _};
use yeban_app::cli::{self, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::Tokens;

/// 被测二进制的绝对路径（由 Cargo 注入，与 `tests/cli_contract.rs` 同款）。
const BIN: &str = env!("CARGO_BIN_EXE_yeban-app");

/// 一次子进程执行的硬超时。
///
/// 与 `cli_contract.rs` 的 `RUN_TIMEOUT` 同一个理由：如果某个"无窗口"开关因为回归被送进
/// GUI 路径，进程会进阻塞事件循环，`Command::output()` 会永远等下去 —— 把一次明确的失败
/// 变成一次"卡住"。这里只需要 `--print-theme` / `--headless` 这种毫秒级命令，
/// 60 秒已经宽到不可能是"慢"，只可能是"挂住"。
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// 一次子进程执行的可观测输出。
#[derive(Debug)]
struct Run {
    /// 退出码（被信号杀死时为 `-1`）。
    code: i32,
    /// stdout 全文。
    stdout: String,
    /// stderr 全文。
    stderr: String,
}

/// 跑一次真二进制并**带超时**收走它的输出。
fn invoke(args: &[&str]) -> Run {
    let mut child = Command::new(BIN)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("无法执行 {BIN}: {error}"));
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("`{BIN} {args:?}` 超过 {RUN_TIMEOUT:?} 未退出 —— 它多半进了 GUI 路径");
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => panic!("等待 {BIN} 失败: {error}"),
        }
    }
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("收取 {BIN} 的输出失败: {error}"));
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

// ---------------------------------------------------------------------------
// 判据 ①: 每个合法取值都被**接受**
// ---------------------------------------------------------------------------

/// `--theme` 的全部合法取值都必须被接受，而且报告里的 `requested=` 必须是**回读**到的
/// 请求值（不是命令行原样抄一遍 —— 那样判据证明不了解析真的发生了）。
#[test]
fn every_valid_theme_value_is_accepted_and_reported() {
    for theme in Theme::ALL {
        let name = theme.name();

        let run = invoke(&["--theme", name, "--print-theme"]);
        assert_eq!(
            run.code, 0,
            "`--theme {name} --print-theme` 必须成功; stderr={}",
            run.stderr
        );
        assert!(
            run.stdout.contains(&format!("requested={name}")),
            "报告必须回读请求的主题 `{name}`; stdout={}",
            run.stdout
        );
        // 调色板来源必须与 `cli::Theme::uses_design_system` 的口径**同源**。
        let expected_palette = if theme.uses_design_system() {
            "design-system-palette"
        } else {
            "brand"
        };
        assert!(
            run.stdout.contains(&format!("palette={expected_palette}")),
            "`{name}` 的调色板来源必须是 {expected_palette}; stdout={}",
            run.stdout
        );

        // 无窗口批处理路径也必须接受它（`--theme` 不改变"哪些开关算无窗口"那张表）。
        let headless = invoke(&["--theme", name, "--headless"]);
        assert_eq!(
            headless.code, 0,
            "`--theme {name} --headless` 必须成功; stderr={}",
            headless.stderr
        );
        assert!(
            headless.stdout.contains(cli::HEADLESS_HANDSHAKE),
            "`--theme {name} --headless` 必须仍然打印握手行; stdout={}",
            headless.stdout
        );
    }
}

// ---------------------------------------------------------------------------
// 判据 ②: 未知取值被**拒绝**，而且拒绝的理由能被读到
// ---------------------------------------------------------------------------

/// 未知主题必须是**用法错误**（退出码 [`cli::EXIT_USAGE`]）、stderr 点名那个坏取值与
/// 可用集合、stdout 为空。
///
/// 为什么"静默回退到默认主题"不可接受：那会让"我说了 material"与"其实渲染的是品牌色"
/// 长得一模一样 —— 正是本仓库最忌讳的一类假绿。
#[test]
fn an_unknown_theme_is_refused_with_a_reason_and_a_nonzero_exit() {
    let bad = "nope-not-a-theme";
    let run = invoke(&["--theme", bad, "--headless"]);

    assert_eq!(
        run.code,
        i32::from(cli::EXIT_USAGE),
        "未知主题必须是用法错误; stderr={}",
        run.stderr
    );
    assert!(
        run.stdout.is_empty(),
        "用法错误不许往 stdout 写东西; stdout={}",
        run.stdout
    );
    assert!(
        run.stderr.contains(bad),
        "stderr 必须点名那个坏取值 `{bad}`; stderr={}",
        run.stderr
    );
    // 可用集合必须逐个点名 —— 用户要能照着改。
    for theme in Theme::ALL {
        assert!(
            run.stderr.contains(theme.name()),
            "stderr 必须列出可用取值 `{}`; stderr={}",
            theme.name(),
            run.stderr
        );
    }
    // 用法错误时 `main.rs` 会补上完整用法文本（"帮助永远打得开"）。
    assert!(
        run.stderr.contains(cli::SLINT_STYLE_ENV),
        "用法文本必须告诉用户换**风格**的正确入口; stderr={}",
        run.stderr
    );

    // `--theme` 缺取值同样是用法错误（不是"当成没给"）。
    let missing = invoke(&["--theme"]);
    assert_eq!(
        missing.code,
        i32::from(cli::EXIT_USAGE),
        "`--theme` 缺取值必须是用法错误; stderr={}",
        missing.stderr
    );

    // 合法取值**不**能被误伤（否则上面那条判据可以被"什么都拒绝"骗过）。
    assert_eq!(invoke(&["--theme", "default", "--print-theme"]).code, 0);
}

// ---------------------------------------------------------------------------
// 判据 ③: 生效的调色板**真的**跟着选择变（回读活的 Slint 组件的 token 值）
// ---------------------------------------------------------------------------

/// 把窗口适配器交给同一个 [`MinimalSoftwareWindow`] 的平台实现。
///
/// 结构与 `crates/yeban-app/src/headless_idle.rs` 的 `IdlePlatform` 一致（那是仓内已核验的
/// 无头路径）：`create_window_adapter` 是 `Platform` 在 Slint 1.18.1 的**唯一**必需方法。
struct ThemePlatform {
    /// 唯一那个软件窗口。
    window: std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
}

impl slint::platform::Platform for ThemePlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

/// 装自研软件平台（**每个线程只能装一次**，见模块文档）。
fn install_software_platform() {
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(slint::PhysicalSize::new(1600, 900));
    slint::platform::set_platform(Box::new(ThemePlatform { window }))
        .expect("本线程还没有平台 (本文件只有这一个测试碰 Slint)");
}

/// 构造一次投影结果 + 外壳场景（与 `--headless-idle` 同一条取数路径）。
fn demo_view_and_scene() -> (yeban_app::bridge::ViewState, DemoScene) {
    let project = yeban_model::samples::default_project();
    let view = cli::project_view(&project).expect("投影空工程");
    let scene = DemoScene::from_view(&view);
    (view, scene)
}

/// **核心判据**：从活的 `MainWindow` 回读 `Tokens.*`，证明
///
/// ① 默认主题（`--theme default`）渲染的就是**今天那一串十六进制字面量** ——
///    这是"默认外观一位未改、Linux golden 基线不需要重生成"的机械见证；
/// ② 非默认主题下的取值**真的不一样**（走 Slint 设计系统的 `Palette` 角色）；
/// ③ Slint 1.18.1 的上限被**如实**登记：四个内建风格名共享同一个编译进来的 `Palette`，
///    因此它们彼此相等 —— 判据把这件事写成断言，而不是回避它。
#[test]
fn the_effective_palette_follows_the_selection() {
    install_software_platform();
    let (view, scene) = demo_view_and_scene();
    let ui = host::build_main_window(&view, &scene).expect("构造真 MainWindow");
    let tokens = ui.global::<Tokens<'_>>();

    // ---- ① 默认 = 今天的外观（**逐字节**的品牌字面量）----
    host::apply_theme(&ui, Theme::Brand);
    let brand: [(&str, Color, u8, u8, u8); 8] = [
        ("bg-panel", tokens.get_bg_panel(), 0x15, 0x1d, 0x38),
        ("bg-panel-alt", tokens.get_bg_panel_alt(), 0x1b, 0x24, 0x47),
        ("bg-raised", tokens.get_bg_raised(), 0x23, 0x2f, 0x5c),
        ("ink-0", tokens.get_ink_0(), 0xf0, 0xeb, 0xe3),
        ("gold", tokens.get_gold(), 0xe2, 0xc7, 0x7e),
        ("line", tokens.get_line(), 0x1e, 0x27, 0x45),
        ("bg-void", tokens.get_bg_void(), 0x06, 0x0a, 0x14),
        ("bg-shell", tokens.get_bg_shell(), 0x0d, 0x13, 0x26),
    ];
    for (name, value, red, green, blue) in brand {
        assert_eq!(
            value,
            Color::from_rgb_u8(red, green, blue),
            "默认主题的 `{name}` 必须仍是品牌字面量 #{red:02x}{green:02x}{blue:02x} \
             —— 它变了就意味着 Linux golden 基线要重生成"
        );
    }

    // ---- ② 非默认主题必须**真的**换掉调色板 ----
    let brand_panel = tokens.get_bg_panel();
    let brand_accent = tokens.get_gold();
    let mut seen: Vec<(Theme, Color, Color)> = Vec::new();
    for theme in Theme::ALL {
        if !theme.uses_design_system() {
            continue;
        }
        host::apply_theme(&ui, theme);
        let panel = tokens.get_bg_panel();
        let accent = tokens.get_gold();
        assert_ne!(
            panel,
            brand_panel,
            "`--theme {}` 之后 `bg-panel` 仍是品牌色 {:?} ⇒ 主题开关是空转的",
            theme.name(),
            brand_panel
        );
        assert_ne!(
            accent,
            brand_accent,
            "`--theme {}` 之后主色仍是品牌金 {:?} ⇒ 主题开关是空转的",
            theme.name(),
            brand_accent
        );
        // 设计系统主题内部必须自洽：`ink-0` 不该等于背景（否则文字看不见）。
        assert_ne!(
            tokens.get_ink_0(),
            panel,
            "`--theme {}` 的前景与背景撞色",
            theme.name()
        );
        seen.push((theme, panel, accent));
    }

    // ---- ③ 如实登记上限：四个内建名字在**同一个二进制**里共享一个 `Palette` ----
    //
    // 这不是缺陷而是上游事实：Slint 1.18.1 的 `select_built_in_style` 不存在
    // （已对该版本的 slint / i-slint-core / i-slint-backend-selector / i-slint-compiler
    //  全文检索），风格只能编译期定（`build.rs` 的 `SLINT_STYLE`）。
    // 因此"四个名字给出四个不同调色板"这条今天**做不到** —— 判据把现状钉住，
    // 一旦上游支持运行时选风格、或我们把四个风格各编一份，这条断言会**变红**，
    // 逼着后来者把它改成"两两不同"。这就是一条会过期的判据该有的样子。
    assert!(
        !seen.is_empty(),
        "至少有一个走设计系统的主题（否则 ② 是空转的）"
    );
    let first = seen[0].1;
    for (theme, panel, _) in &seen {
        assert_eq!(
            *panel,
            first,
            "`--theme {}` 与 `--theme {}` 的 Palette 取值不同 —— \
             那说明这个二进制支持了运行时选风格: 请把这条断言升级成『两两不同』",
            theme.name(),
            seen[0].0.name()
        );
    }

    // 回读到的主题状态必须就是刚写进去的那个（写入口唯一：`host::apply_theme`）。
    host::apply_theme(&ui, Theme::Native);
    assert_eq!(
        ui.global::<yeban_app::ui::ThemeState<'_>>().get_theme(),
        yeban_app::ui::YebanTheme::Native,
        "`apply_theme` 必须把状态真的写进 Slint 侧"
    );

    // ---- 编译期事实也是事实: `--print-theme` 报的风格必须与 build.rs 注入的一致 ----
    //
    // 这一条把"报告"与"构建"钉在一起：`build.rs` 把 `SLINT_STYLE` 的解析结果注进
    // `YEBAN_SLINT_STYLE`，`cli::compiled_slint_style()` 读它，`--print-theme` 打它。
    // 三者一旦漂移（例如有人只改了其中一处），这条会红。
    let report = invoke(&["--theme", "material", "--print-theme"]);
    assert_eq!(report.code, 0);
    assert!(
        report
            .stdout
            .contains(&format!("compiled-style={}", cli::compiled_slint_style())),
        "--print-theme 报的编译期风格必须来自 build.rs 注入的事实; stdout={}",
        report.stdout
    );
}
