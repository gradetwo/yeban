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
//! | ④ | `yeban` 的每一支颜色 == 测量表里的那个字面量（回读 + 对比度断言） | 真 `MainWindow` 的 token 值 |
//! | ⑤ | `yeban` 调色板的**源码钉**：`ui/tokens.slint` 里每个 token 的 brand/yeban 两个分支 | `tokens.slint` 的文本 |
//! | ⑥ | `accent`（朱砂）在整份界面里**恰好出现一次**；`bg-control` 也恰好一次 | 全部 `ui/**/*.slint` 的文本 |
//!
//! ④ 是"颜色是**测量**的、不是挑的"那条要求的可失败形态：把 `#23252a` 改成别的值，
//! 或者把测量表里的数字抄错一位，它就会红。⑤ 是 ④ 的**独立**见证 —— 它不看运行时，
//! 直接读源码文本，因此"改对了 Slint 但抄错了表"和"表对了但源码漂了"都会被抓住。
//! ⑥ 是"强调色唯一"的机械形态：多写一处 `Tokens.accent` 就红。
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
        // 调色板来源必须与 `cli::Theme::palette_source` 的口径**同源**。
        let expected_palette = theme.palette_source();
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
    let brand: [(&str, Color, u8, u8, u8); 10] = [
        ("bg-panel", tokens.get_bg_panel(), 0x15, 0x1d, 0x38),
        ("bg-panel-alt", tokens.get_bg_panel_alt(), 0x1b, 0x24, 0x47),
        ("bg-raised", tokens.get_bg_raised(), 0x23, 0x2f, 0x5c),
        ("ink-0", tokens.get_ink_0(), 0xf0, 0xeb, 0xe3),
        ("gold", tokens.get_gold(), 0xe2, 0xc7, 0x7e),
        ("line", tokens.get_line(), 0x1e, 0x27, 0x45),
        ("bg-void", tokens.get_bg_void(), 0x06, 0x0a, 0x14),
        ("bg-shell", tokens.get_bg_shell(), 0x0d, 0x13, 0x26),
        // 2026-10-06 新增的两支（见 `ui/tokens.slint` §6c）: 默认主题下它们必须**分别**
        // 等于被它们替换掉的那两个字面量，否则顶栏的渲染字节就变了。
        ("bg-control", tokens.get_bg_control(), 0x1b, 0x24, 0x47),
        ("accent", tokens.get_accent(), 0xa8, 0x55, 0xf7),
    ];
    for (name, value, red, green, blue) in brand {
        assert_eq!(
            value,
            Color::from_rgb_u8(red, green, blue),
            "默认主题的 `{name}` 必须仍是品牌字面量 #{red:02x}{green:02x}{blue:02x} \
             —— 它变了就意味着 Linux golden 基线要重生成"
        );
    }
    // 这两条是**顶栏缺陷修复的机械保证**（2026-10-06）:
    // 保存键的板与 AI 提案徽章的板必须分别等于它们原来用的那个字面量,
    // 于是 `transport.slint` 里那两处替换在默认主题下是**零像素差**的。
    assert_eq!(
        tokens.get_bg_control(),
        tokens.get_bg_panel_alt(),
        "默认主题下 `bg-control` 必须逐字节等于 `bg-panel-alt` —— 保存键的板换了令牌之后 \
         渲染字节不许变（它原来用的就是 bg-panel-alt）"
    );
    assert_eq!(
        tokens.get_accent(),
        tokens.get_ai_suggestion(),
        "默认主题下 `accent` 必须逐字节等于 `ai-suggestion` —— AI 提案徽章的描边换了令牌 \
         之后渲染字节不许变（它原来用的就是 ai-suggestion）"
    );

    // ---- ①b `yeban`: 每一支都必须等于测量表里的那个字面量 ----
    //
    // 表里的每一个十六进制都有一条**测量**出处（`ui/tokens.slint` §6b/§6c 逐条写明了
    // 采样源、方法与像素频次）。改一个数字、或把测量抄错一位，这里就红。
    host::apply_theme(&ui, Theme::Yeban);
    let yeban: [(&str, Color, u32); 15] = [
        ("bg-void", tokens.get_bg_void(), 0x0c0d0e),
        ("bg-shell", tokens.get_bg_shell(), 0x17181c),
        ("bg-panel", tokens.get_bg_panel(), 0x23252a),
        ("bg-panel-alt", tokens.get_bg_panel_alt(), 0x2d2f35),
        ("bg-raised", tokens.get_bg_raised(), 0x3a3d45),
        ("bg-control", tokens.get_bg_control(), 0x42454d),
        ("line", tokens.get_line(), 0x2e2f35),
        ("line-strong", tokens.get_line_strong(), 0x464a53),
        ("ink-0", tokens.get_ink_0(), 0xf0ebe3),
        ("ink-1", tokens.get_ink_1(), 0xc8c2b8),
        ("ink-2", tokens.get_ink_2(), 0x696f7b),
        ("gold-bright", tokens.get_gold_bright(), 0xf7e6b0),
        ("gold", tokens.get_gold(), 0xe2c77e),
        ("gold-deep", tokens.get_gold_deep(), 0x22aa88),
        ("accent", tokens.get_accent(), 0xd94a4a),
    ];
    for (name, value, packed) in yeban {
        let expected = Color::from_rgb_u8(
            ((packed >> 16) & 0xff) as u8,
            ((packed >> 8) & 0xff) as u8,
            (packed & 0xff) as u8,
        );
        assert_eq!(
            value, expected,
            "`--theme yeban` 的 `{name}` 必须是测量表里的 #{packed:06x} —— \
             颜色是**测量**出来的, 不是挑出来的; 表在 ui/tokens.slint §6b/§6c"
        );
    }

    // ---- ①c `yeban`: 对比度必须**算出来**达标, 不许目测 ----
    //
    // 阈值取自 WCAG 2.1: 正文 7:1 (AAA) / 4.5:1 (AA) / 3:1 (AA 大字与非文本 UI 边界
    // [1.4.11])。为什么用 WCAG 而不是自定的数: 它是本仓库 [UI-A11Y-004] 已经引用的
    // 那份标准, 而且这些数是可以逐条复核的。
    let case: [(&str, Color, Color, f64); 10] = [
        (
            "ink-0 / bg-void",
            tokens.get_ink_0(),
            tokens.get_bg_void(),
            7.0,
        ),
        (
            "ink-0 / bg-panel",
            tokens.get_ink_0(),
            tokens.get_bg_panel(),
            7.0,
        ),
        (
            "ink-0 / bg-panel-alt",
            tokens.get_ink_0(),
            tokens.get_bg_panel_alt(),
            7.0,
        ),
        (
            "ink-1 / bg-panel",
            tokens.get_ink_1(),
            tokens.get_bg_panel(),
            4.5,
        ),
        (
            "ink-2 / bg-panel",
            tokens.get_ink_2(),
            tokens.get_bg_panel(),
            3.0,
        ),
        (
            "gold / bg-panel-alt",
            tokens.get_gold(),
            tokens.get_bg_panel_alt(),
            4.5,
        ),
        (
            "gold / bg-void",
            tokens.get_gold(),
            tokens.get_bg_void(),
            4.5,
        ),
        (
            "gold-bright / bg-void",
            tokens.get_gold_bright(),
            tokens.get_bg_void(),
            7.0,
        ),
        (
            "gold-deep / bg-panel-alt",
            tokens.get_gold_deep(),
            tokens.get_bg_panel_alt(),
            4.5,
        ),
        (
            "accent / bg-panel-alt",
            tokens.get_accent(),
            tokens.get_bg_panel_alt(),
            3.0,
        ),
    ];
    for (label, a, b, threshold) in case {
        let ratio = contrast_ratio(a, b);
        assert!(
            ratio >= threshold,
            "`yeban` 的对比度不达标: {label} = {ratio:.2}:1 < {threshold}:1 (WCAG 2.1)"
        );
    }

    // ---- ①d 顶栏两块板必须**可区分**（负责人报的缺陷） ----
    //
    // 默认主题下两块板逐字节相同（`bg-control == bg-panel-alt`, 见 ①）⇒ 那正是缺陷本身。
    // `yeban` 下它们必须分开, 而且**给出数字**: 亮度比要高于默认主题自己用来区分
    // `bg-panel` 与 `bg-panel-alt` 的那一档 (实测 1.10:1), 否则"分开了"只是一句话。
    let slab = contrast_ratio(tokens.get_bg_panel_alt(), tokens.get_bg_control());
    assert!(
        slab > 1.10,
        "`yeban` 顶栏的保存键板 / AI 徽章板亮度比只有 {slab:.2}:1, 不高于默认主题自己 \
         的 panel↔panel-alt 那一档 (1.10:1) ⇒ 两块板仍然分不开"
    );
    // 强调色也必须真的不同 (否则"朱砂只用一次"就只是把同一个紫换了个名字)。
    assert_ne!(
        tokens.get_accent(),
        tokens.get_ai_suggestion(),
        "`yeban` 的 `accent` 不许等于 AI 建议的语义紫 —— 那意味着强调色其实没换"
    );

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

// ---------------------------------------------------------------------------
// WCAG 2.1 相对亮度比 —— 对比度是**算**出来的, 不是看出来的
// ---------------------------------------------------------------------------

/// sRGB 单通道 → 线性值（WCAG 2.1 定义的那种线性化，不是简单的 /255）。
fn linear(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 2.1 相对亮度 `L = 0.2126 R + 0.7152 G + 0.0722 B`（线性通道）。
fn relative_luminance(color: Color) -> f64 {
    0.2126 * linear(color.red()) + 0.7152 * linear(color.green()) + 0.0722 * linear(color.blue())
}

/// WCAG 2.1 对比度 `(L_亮 + 0.05) / (L_暗 + 0.05)`，落在 `1.0..=21.0`。
fn contrast_ratio(a: Color, b: Color) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

// ---------------------------------------------------------------------------
// 判据 ⑤/⑥: 源码钉 —— 不碰 Slint, 直接把 `ui/**/*.slint` 当文本读
// ---------------------------------------------------------------------------
//
// 为什么还要一条源码判据（运行时回读已经是判据 ④）: 两者抓的是**不同**的漂移。
// ④ 抓"编译出来的值 != 测量表"; ⑤ 抓"源码里的字面量 != 测量表" —— 例如有人把
// 两个分支的表达式写反了、或者给某个 token 多加了一层三元但恰好让 brand 那支没变。
// 一条判据只写一遍自己的观测面, 这里是文本。

/// `ui/` 的绝对路径（`CARGO_MANIFEST_DIR` 由 Cargo 注入, 与 `BIN` 同款）。
const UI_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ui");

/// 把 `ui/` 下所有 `.slint`（含子目录）读成 `(相对路径, 全文)`。
fn all_ui_sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("读 ui/ 目录") {
            let path = entry.expect("读目录项").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "slint") {
                let text = std::fs::read_to_string(&path).expect("读 .slint 源文件");
                out.push((path.display().to_string(), text));
            }
        }
    }
    let mut out = Vec::new();
    walk(std::path::Path::new(UI_DIR), &mut out);
    out.sort();
    assert!(!out.is_empty(), "`{UI_DIR}` 下必须有 .slint 文件");
    out
}

/// 取出某个 token 那一行里的**前两个** `#rrggbb`（顺序 = brand 分支、yeban 分支）。
fn token_literals(source: &str, token: &str) -> Vec<String> {
    let needle = format!("out property <color> {token}:");
    let line = source
        .lines()
        .find(|line| line.trim_start().starts_with(&needle))
        .unwrap_or_else(|| panic!("`ui/tokens.slint` 里找不到 `{needle}`"));
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 7 <= bytes.len() {
        if bytes[i] == b'#' && bytes[i + 1..i + 7].iter().all(u8::is_ascii_hexdigit) {
            out.push(line[i..i + 7].to_ascii_lowercase());
            i += 7;
        } else {
            i += 1;
        }
    }
    assert!(
        out.len() >= 2,
        "`{needle}` 那一行必须同时写死 brand 与 yeban 两个字面量; 实际={out:?}"
    );
    out
}

/// **判据 ⑤**：`ui/tokens.slint` 里每个主题令牌的两个字面量分支都等于测量表。
///
/// 表里的 yeban 值与判据 ④ 是**同一份数字的两个独立观测面**（一个读源码、一个读活组件）。
#[test]
fn the_theme_palette_literals_in_the_source_are_the_measured_ones() {
    let sources = all_ui_sources();
    let (_, tokens_slint) = sources
        .iter()
        .find(|(path, _)| path.ends_with("ui/tokens.slint"))
        .expect("必须存在 ui/tokens.slint");

    // (token, brand 字面量, yeban 字面量)
    let table: [(&str, &str, &str); 15] = [
        ("bg-void", "#060a14", "#0c0d0e"),
        ("bg-shell", "#0d1326", "#17181c"),
        ("bg-panel", "#151d38", "#23252a"),
        ("bg-panel-alt", "#1b2447", "#2d2f35"),
        ("bg-raised", "#232f5c", "#3a3d45"),
        ("bg-control", "#1b2447", "#42454d"),
        ("line", "#1e2745", "#2e2f35"),
        ("line-strong", "#2c3a63", "#464a53"),
        ("ink-0", "#f0ebe3", "#f0ebe3"),
        ("ink-1", "#c8c2b8", "#c8c2b8"),
        ("ink-2", "#5a6b8a", "#696f7b"),
        ("gold-bright", "#f7e6b0", "#f7e6b0"),
        ("gold", "#e2c77e", "#e2c77e"),
        ("gold-deep", "#b8933e", "#22aa88"),
        ("accent", "#a855f7", "#d94a4a"),
    ];
    for (token, brand, yeban) in table {
        let found = token_literals(tokens_slint, token);
        assert_eq!(
            found[0], brand,
            "`{token}` 的 brand 分支必须仍是 {brand}（默认外观逐像素不变）; 实际 {}",
            found[0]
        );
        assert_eq!(
            found[1], yeban,
            "`{token}` 的 yeban 分支必须是测量表里的 {yeban}（出处见 §6b/§6c）; 实际 {}",
            found[1]
        );
    }

    // 品牌色 `#151d38` 这条**既有**的字面量判据（本仓库"品牌色从母版 SVG 提取"那条）
    // 必须仍然成立: 它一旦消失, "默认主题 = 今天的外观"就没有源码侧的见证。
    assert!(
        tokens_slint.contains("#151d38"),
        "`ui/tokens.slint` 必须仍然写死品牌面板色 `#151d38`"
    );
}

/// **判据 ⑥**：唯一的强调色在整份界面里**恰好引用一次**，而且那一处就是 AI 提案徽章的描边。
///
/// 这是"强调色必须唯一"那条设计原则的机械形态：任何第二处引用（例如顺手也把保存键
/// 描成朱砂）都会让这条红。`bg-control` 同样只准一处 —— 它是为那一块板而加的。
#[test]
fn the_accent_appears_in_exactly_one_role_in_the_ui_sources() {
    // ⚠ 数的是**代码**，不是注释：源码里用反引号提到某个令牌名是文档，不是引用。
    // （第一次跑这条判据时它数到了 3 处 —— 两处在注释里 —— 这正是"有牙"的样子。）
    let sources: Vec<(String, String)> = all_ui_sources()
        .into_iter()
        .map(|(path, text)| (path, strip_slint_comments(&text)))
        .collect();
    let files_with = |needle: &str| -> Vec<String> {
        sources
            .iter()
            .filter(|(_, text)| text.contains(needle))
            .map(|(path, _)| path.clone())
            .collect()
    };

    let mut hits = Vec::new();
    for (path, text) in &sources {
        for (line_no, line) in text.lines().enumerate() {
            if line.contains("Tokens.accent") {
                hits.push(format!("{path}:{}: {}", line_no + 1, line.trim()));
            }
        }
    }
    assert_eq!(
        hits.len(),
        1,
        "`Tokens.accent`（朱砂）必须**恰好**出现一次, 实际 {} 处: {hits:#?}",
        hits.len()
    );
    assert!(
        hits[0].contains("transport.slint") && hits[0].contains("border-color"),
        "唯一那一处必须是顶栏 AI 提案徽章的 `border-color`（保存键与它相隔 4 px, \
         强调色必须落在一个唯一的位置）; 实际 {}",
        hits[0]
    );

    let control = files_with("Tokens.bg-control");
    assert_eq!(
        control,
        vec![format!("{UI_DIR}/transport.slint")],
        "`Tokens.bg-control` 必须**只**用在 transport.slint（保存键的板）; 实际 {control:#?}"
    );

    // 语义色不许被我们顺手删掉: `ai-suggestion` 仍必须按 §9.1 用在 AI 建议层上。
    assert!(
        !files_with("Tokens.ai-suggestion").is_empty(),
        "`ai-suggestion` 仍是 AI 建议层的语义色（AI 徽章换用 `accent` 之后它不该消失）"
    );
}

/// 去掉 Slint 的 `//` 与 `/* */` 注释（保留换行，因此行号仍然对得上）。
fn strip_slint_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_block = false;
    while let Some(c) = chars.next() {
        if in_block {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block = false;
            }
            continue;
        }
        if c == '/' {
            match chars.peek() {
                Some('/') => {
                    for next in chars.by_ref() {
                        if next == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    in_block = true;
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}
