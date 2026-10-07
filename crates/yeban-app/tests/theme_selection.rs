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
//! | ④ | `yeban` / `inkmoor` / `plume` 的每一支颜色 == 负责人下发的那个字面量（回读 + 对比度断言） | 真 `MainWindow` 的 token 值 |
//! | ⑤ | 四支自绘调色板的**源码钉**：`ui/tokens.slint` 里每个 token 的四个字面量分支 | `tokens.slint` 的文本 |
//! | ⑥ | `accent`（唯一强调色）在整份界面里**恰好出现一次**，且那一处是**录音键的 ●**（`HD-54`）；`bg-control` 也恰好一次 | 全部 `ui/**/*.slint` 的文本 |
//! | ⑦ | 原则「暖强调色唯一」「无投影/无渐变」的机械形态（三支自绘皮肤各一条，含落点搬走后 AI 徽章必须冷、以及 `accent != record-red`）；`--print-theme` 的出处行 | token 值的色相 + 全部 `ui/**/*.slint` 的文本 + 真进程 stdout |
//!
//! ## 2026-10-07：三支自绘皮肤（`yeban` / `inkmoor` / `plume`）
//!
//! 上一版（提交 `a19b2e0`）的 `yeban` 取值是**我们自己**从品牌 logo 与 golden 渲染帧里
//! 采出来的。负责人随后把一份**具名的完整调色板**（HTML mock）作为权威设计输入交下来：
//! 背景六级的墨阶、两级发丝线、三级文字、以及枫桥夜泊的颜料槽（渔火 / 月华 / 江枫 /
//! 愁眠 / 铜绿 / 客船 / 寒山 / 苇白）。**他们的数字取代我们的推导** —— 旧采样值全部作废，
//! 这是设计权威的转移，不是回归。
//!
//! 2026-10-07 同日的**修订版**设计稿又下发了两套成对皮肤：**墨泊 InkMoor**（冷墨，
//! 枫桥夜泊）与**孤烟 Plume**（暖沙，使至塞上）。它们的取值与 `yeban` **逐条不同**
//! （同一套意象的两版数字，例：渔火 `yeban #c6a47c` / `inkmoor #c9a26b`）——本切片按
//! 硬约束**保留 `yeban` 原值**、新增两支，逐条映射见 `ui/tokens.slint` §6e。
//!
//! 因此 ④ 的表从 15 行扩到 **23 行**（新增 `bg-lane` / `bg-lane2` / `bg-head` / `playing` /
//! `selection` / `piano-key` 六支结构上缺失的令牌，并给 `ai-suggestion` / `record-red`
//! 补上分支），再**每支皮肤各一份**；判据的措辞也从「颜色是**测量**出来的」改成
//! 「颜色是负责人**下发**的」。每一条仍然**两侧独立见证**：④ 读活的组件，⑤ 读源码文本。
//!
//! 对比度断言**保留阈值**，把负责人的数字代进去重算（三支皮肤用**同一张**配对表
//! `wcag_cases`）。**如实报告**：`yeban` 的 `--txt-faint` 霜灰 `#5a6672` 对
//! `--bg-panel` 黛蓝·暗 `#131a22` 只有 **2.99:1 < 3:1**（对 `#161e28` 2.86:1、对
//! `#1e2733` 2.57:1）；`plume` 的三级灰 `#6f665a` 对 `--bg-raise` `#271f19` 是
//! **2.87:1 < 3:1**（修订版把 `yeban` 那三对中的两对修好了，第三对仍差 0.13）。
//! 我们**没有**改负责人的颜色，而是把这些对写进登记表 —— 数字被**钉住**（谁改了任一侧的
//! 十六进制都会红），但缺口本身被登记为负责人调色板的事实，等待裁决。同样登记为缺口的是
//! **顶栏两块板的亮度比**：`yeban` 是 1.11:1（高于 ①d 的 1.10 门槛），而两支新皮肤都只有
//! 1.08:1 —— 负责人的墨阶/沙阶在 `bg-head` 与 `bg-raise` 之间只有一级，**不**把硬门槛套到
//! 它们身上（套了就会把负责人的颜色判红），改成把数字钉住。
//!
//! ③ 是这条工作线的核心 —— 因为"换主题"这件事最容易做成"改了命令行、什么都没变"。
//! 它读的是 `ui/tokens.slint` 里那十三支颜色令牌的**实际取值**，不是像素、不是文件内容。
//!
//! ## 2026-10-07 `HD-54`：唯一暖强调的**落点**从 AI 徽章搬到录音键的 ●
//!
//! 负责人裁决：录音键——落日，唯一带光环的圆；`accent` 的落点要从 AI 徽章**移到录音键**，
//! AI 徽章另找角色（本仓取 `ai-suggestion`）。因此本文件里有四处**随落点一起移动**：
//!   * ③ 的品牌表：`accent` 的 brand 值写成 `#d94a4a`（= 录音键的 ● 原来画的
//!     `record-red`），并新增 `accent == record-red`、`ai-suggestion == #a855f7`
//!     （= `accent` **移动前**的 brand 值）两条机械保证 —— 这是 `--theme default`
//!     逐像素不变的两个前提;
//!   * `wcag_cases`：录音键的静置板 `bg-panel-alt`、按下板 `bg-raised` 两对，加上旧落点
//!     AI 徽章的 `ai-suggestion / bg-panel-alt`（16 → 18 对）;
//!   * ⑥：唯一那一处的落点判据从"AI 徽章的 `border-color`"改成"**录音键的 ●**"，并反向
//!     钉住旧落点必须已换成 `ai-suggestion`（`accent` 一旦被复制回徽章, 计数先红）;
//!   * ⑦a / ⑦a-bis：新增两条随落点走的断言 —— 旧落点的新角色必须是**冷**色相、
//!     `accent` 不许等于录音键的功能色 `record-red`（否则"移动"只是改了个名）。
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
    let brand: [(&str, Color, u8, u8, u8); 12] = [
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
        // 2026-10-07（`HD-54`）: 唯一强调色的**落点**从 AI 徽章描边移到录音键的 ●。
        // 落点移动不许改默认像素 ⇒ 被碰到的两支令牌各写出它们的 brand 值:
        //   `accent`        = #d94a4a = 录音键的 ● 原来画的 `record-red`（brand）;
        //   `record-red`    = #d94a4a（取值一位未改）;
        //   `ai-suggestion` = #a855f7 = AI 徽章描边原来画的 `accent`（移动前）的 brand 值。
        ("accent", tokens.get_accent(), 0xd9, 0x4a, 0x4a),
        ("record-red", tokens.get_record_red(), 0xd9, 0x4a, 0x4a),
        (
            "ai-suggestion",
            tokens.get_ai_suggestion(),
            0xa8,
            0x55,
            0xf7,
        ),
    ];
    for (name, value, red, green, blue) in brand {
        assert_eq!(
            value,
            Color::from_rgb_u8(red, green, blue),
            "默认主题的 `{name}` 必须仍是品牌字面量 #{red:02x}{green:02x}{blue:02x} \
             —— 它变了就意味着 Linux golden 基线要重生成"
        );
    }
    // 这三条是**顶栏两处令牌替换的机械保证**:
    //   ① 2026-10-06 保存键的板: `bg-control` == `bg-panel-alt`（它原来用的就是后者）;
    //   ② 2026-10-07 `HD-54` 录音键的 ●: `accent` == `record-red`（落点移动后它画的还是
    //      那个红, 所以 `--theme default` 一位未变）;
    //   ③ 2026-10-07 `HD-54` AI 徽章的描边: `ai-suggestion` == 移动前 `accent` 的 brand 值
    //      #a855f7（徽章也因此一位未变）。
    assert_eq!(
        tokens.get_bg_control(),
        tokens.get_bg_panel_alt(),
        "默认主题下 `bg-control` 必须逐字节等于 `bg-panel-alt` —— 保存键的板换了令牌之后 \
         渲染字节不许变（它原来用的就是 bg-panel-alt）"
    );
    assert_eq!(
        tokens.get_accent(),
        tokens.get_record_red(),
        "默认主题下 `accent` 必须逐字节等于 `record-red` —— 录音键的 ● 换成 `accent` 之后 \
         渲染字节不许变（它原来画的就是 record-red 的 #d94a4a）"
    );
    assert_eq!(
        tokens.get_ai_suggestion(),
        Color::from_rgb_u8(0xa8, 0x55, 0xf7),
        "默认主题下 `ai-suggestion` 必须仍是 #a855f7 —— AI 徽章的描边从 `accent` 换成它 \
         之后渲染字节不许变（`accent` **移动前**的 brand 值就是 #a855f7）"
    );

    // ---- ①b `yeban`: 每一支都必须等于负责人下发的那个字面量 ----
    //
    // 表里的每一个十六进制都是负责人 HTML mock 里的原话（出处 = 色名 + 角色，逐条写在
    // `ui/tokens.slint` §6b）。改一个数字、或把下发的值抄错一位，这里就红。
    host::apply_theme(&ui, Theme::Yeban);
    let yeban: [(&str, Color, u32); 23] = [
        ("bg-void", tokens.get_bg_void(), 0x0e1216),
        ("bg-shell", tokens.get_bg_shell(), 0x11161c),
        ("bg-lane", tokens.get_bg_lane(), 0x11161c),
        ("bg-lane2", tokens.get_bg_lane2(), 0x0d1116),
        ("bg-panel", tokens.get_bg_panel(), 0x131a22),
        ("bg-head", tokens.get_bg_head(), 0x161e28),
        ("bg-panel-alt", tokens.get_bg_panel_alt(), 0x161e28),
        ("bg-raised", tokens.get_bg_raised(), 0x1e2733),
        ("bg-control", tokens.get_bg_control(), 0x1e2733),
        ("line", tokens.get_line(), 0x212b36),
        ("line-strong", tokens.get_line_strong(), 0x2c3948),
        ("ink-0", tokens.get_ink_0(), 0xd7dee1),
        ("ink-1", tokens.get_ink_1(), 0x8d99a5),
        ("ink-2", tokens.get_ink_2(), 0x5a6672),
        ("gold-bright", tokens.get_gold_bright(), 0xe7dfc8),
        ("gold", tokens.get_gold(), 0xc6a47c),
        ("gold-deep", tokens.get_gold_deep(), 0x6d88a1),
        ("accent", tokens.get_accent(), 0xc6a47c),
        ("ai-suggestion", tokens.get_ai_suggestion(), 0x8e86a6),
        ("record-red", tokens.get_record_red(), 0xac6e60),
        ("playing", tokens.get_playing(), 0x6e9488),
        ("selection", tokens.get_selection(), 0x8e86a6),
        ("piano-key", tokens.get_piano_key(), 0xc8c2b2),
    ];
    for (name, value, packed) in yeban {
        let expected = Color::from_rgb_u8(
            ((packed >> 16) & 0xff) as u8,
            ((packed >> 8) & 0xff) as u8,
            (packed & 0xff) as u8,
        );
        assert_eq!(
            value, expected,
            "`--theme yeban` 的 `{name}` 必须是负责人下发的 #{packed:06x} —— \
             颜色是**负责人指定**的, 不是我们挑的; 表在 ui/tokens.slint §6b"
        );
    }

    // ---- ①b-bis `inkmoor` / `plume`: 每一支都必须等于负责人下发的那个字面量 ----
    //
    // 与 ①b 同一条纪律、同一张表：表里的每一个十六进制都是负责人设计稿（HTML mock）
    // 的原话，出处 = 同一份稿子的 `body.inkmoor` / `body.plume` CSS 变量块，逐条映射写在
    // `ui/tokens.slint` §6e。改一个数字、或把下发的值抄错一位，这里就红。
    //
    // 这两支与 `yeban` 是**同一套意象的两版数字**（例：渔火 yeban #c6a47c /
    // inkmoor #c9a26b），因此这张表本身就是"两版不同"的机械见证。
    for (skin, expected) in [
        (
            "inkmoor",
            [
                ("bg-void", 0x0e1114u32),
                ("bg-shell", 0x12161b),
                ("bg-lane", 0x12161b),
                ("bg-lane2", 0x0f1317),
                ("bg-panel", 0x141920),
                ("bg-head", 0x171c24),
                ("bg-panel-alt", 0x171c24),
                ("bg-raised", 0x1d232d),
                ("bg-control", 0x1d232d),
                ("line", 0x20242b),
                ("line-strong", 0x282d35),
                ("ink-0", 0xe9e7e2),
                ("ink-1", 0xa3a7ae),
                ("ink-2", 0x6a6f78),
                ("gold-bright", 0xefe9da),
                ("gold", 0xc9a26b),
                ("gold-deep", 0x7c93a8),
                ("accent", 0xc9a26b),
                ("ai-suggestion", 0x9e97ae),
                ("record-red", 0xb4715f),
                ("playing", 0x85a794),
                ("selection", 0x9e97ae),
                ("piano-key", 0xccc5b4),
            ],
        ),
        (
            "plume",
            [
                ("bg-void", 0x14100du32),
                ("bg-shell", 0x181310),
                ("bg-lane", 0x181310),
                ("bg-lane2", 0x151110),
                ("bg-panel", 0x1b1512),
                ("bg-head", 0x1f1814),
                ("bg-panel-alt", 0x1f1814),
                ("bg-raised", 0x271f19),
                ("bg-control", 0x271f19),
                ("line", 0x2a2520),
                ("line-strong", 0x332d26),
                ("ink-0", 0xefe7d9),
                ("ink-1", 0xaba091),
                ("ink-2", 0x6f665a),
                ("gold-bright", 0xeae2d3),
                ("gold", 0xc68252),
                ("gold-deep", 0x6f8d96),
                ("accent", 0xc68252),
                ("ai-suggestion", 0x9c93a2),
                ("record-red", 0xb25c43),
                ("playing", 0x7f9483),
                ("selection", 0x9c93a2),
                ("piano-key", 0xc2ad91),
            ],
        ),
    ] {
        let theme = Theme::from_name(skin).expect("皮肤名必须是合法 --theme 取值");
        host::apply_theme(&ui, theme);
        let read: [(&str, Color); 23] = [
            ("bg-void", tokens.get_bg_void()),
            ("bg-shell", tokens.get_bg_shell()),
            ("bg-lane", tokens.get_bg_lane()),
            ("bg-lane2", tokens.get_bg_lane2()),
            ("bg-panel", tokens.get_bg_panel()),
            ("bg-head", tokens.get_bg_head()),
            ("bg-panel-alt", tokens.get_bg_panel_alt()),
            ("bg-raised", tokens.get_bg_raised()),
            ("bg-control", tokens.get_bg_control()),
            ("line", tokens.get_line()),
            ("line-strong", tokens.get_line_strong()),
            ("ink-0", tokens.get_ink_0()),
            ("ink-1", tokens.get_ink_1()),
            ("ink-2", tokens.get_ink_2()),
            ("gold-bright", tokens.get_gold_bright()),
            ("gold", tokens.get_gold()),
            ("gold-deep", tokens.get_gold_deep()),
            ("accent", tokens.get_accent()),
            ("ai-suggestion", tokens.get_ai_suggestion()),
            ("record-red", tokens.get_record_red()),
            ("playing", tokens.get_playing()),
            ("selection", tokens.get_selection()),
            ("piano-key", tokens.get_piano_key()),
        ];
        for ((name, value), (expected_name, packed)) in read.into_iter().zip(expected) {
            assert_eq!(
                name, expected_name,
                "`{skin}` 的表与回读顺序不一致（表内下标错位）"
            );
            let expected_color = Color::from_rgb_u8(
                ((packed >> 16) & 0xff) as u8,
                ((packed >> 8) & 0xff) as u8,
                (packed & 0xff) as u8,
            );
            assert_eq!(
                value, expected_color,
                "`--theme {skin}` 的 `{name}` 必须是负责人下发的 #{packed:06x} —— \
                 颜色是**负责人指定**的, 不是我们挑的; 表在 ui/tokens.slint §6e"
            );
        }
    }

    // ---- ①c `yeban`: 对比度必须**算出来**达标, 不许目测 ----
    //
    // 阈值取自 WCAG 2.1: 正文 7:1 (AAA) / 4.5:1 (AA) / 3:1 (AA 大字与非文本 UI 边界
    // [1.4.11])。为什么用 WCAG 而不是自定的数: 它是本仓库 [UI-A11Y-004] 已经引用的
    // 那份标准, 而且这些数是可以逐条复核的。
    //
    // 2026-10-07: 阈值**一位未改**, 只把负责人的数字代进去重算。表里的每一对都是
    // **真实存在的配对**（前景画在哪个背景上, 逐条注明）。
    //
    // ⚠ 先写回 `yeban`（2026-10-07 编译修复时补上的一行）: `wcag_cases` 读的是**当前**
    // token 值, 而上面那段 inkmoor/plume 循环把 `ThemeState` 留在了 `plume`。不写回的话
    // 这一段会拿 **plume** 的数字去撞「`yeban` 的对比度不达标」这句话 —— 它照样会绿,
    // 于是 `yeban` 的 16 对配对**一条都不再被这段覆盖**(下面 ①c-ter 只跑 inkmoor/plume)。
    // 提交 4257d0f 的顺序是「`apply_theme(Yeban)` 之后立刻建表」, 这一行恢复那个语义。
    host::apply_theme(&ui, Theme::Yeban);
    let case = wcag_cases(&tokens);
    for (label, a, b, threshold) in case {
        let ratio = contrast_ratio(a, b);
        assert!(
            ratio >= threshold,
            "`yeban` 的对比度不达标: {label} = {ratio:.2}:1 < {threshold}:1 (WCAG 2.1)"
        );
    }

    // ---- ①c-ter 两支新皮肤：**同一张**阈值表逐支重算 ----
    //
    // 观测面与 ①c 完全相同（活组件的 token 值）与同一组配对，只是换了皮肤。
    // **没有**为它们把任何阈值调低 —— 失败时的措辞明确禁止"改负责人的颜色"。
    for skin in ["inkmoor", "plume"] {
        host::apply_theme(
            &ui,
            Theme::from_name(skin).expect("皮肤名必须是合法 --theme 取值"),
        );
        let cases = wcag_cases(&tokens);
        for (label, a, b, threshold) in cases {
            let ratio = contrast_ratio(a, b);
            assert!(
                ratio >= threshold,
                "`{skin}` 的对比度不达标: {label} = {ratio:.2}:1 < {threshold}:1 (WCAG 2.1) \
                 —— 颜色是负责人下发的, **不许**为了让它过而改色; 把数字交回负责人裁决"
            );
        }
    }

    // ---- ①c-quater 三级文字在三支皮肤上的**逐对实测**（缺口不隐藏） ----
    //
    // `ink-2` 是三级文字。`yeban` 的三对全部低于 3:1（见 ①c-bis）。负责人**修订版**
    // 调色板把霜灰/驼灰调亮了，于是实测：
    //   inkmoor  3.49 / 3.39 / 3.12 —— 三对都 ≥ 3:1（旧缺口消失）
    //   plume    3.20 / 3.11 / **2.87** —— `ink-2 / bg-raised` 仍低于 3:1
    // 我们**没有**改负责人的颜色（那是权威设计输入），只把每个数字钉住：任一侧改色，
    // 这里的"记录过期"与"缺口登记不符"两条断言至少有一条会红。
    for (skin, recorded) in [
        (
            "inkmoor",
            [
                ("ink-2 / bg-panel", 3.49_f64, false),
                ("ink-2 / bg-panel-alt", 3.39, false),
                ("ink-2 / bg-raised", 3.12, false),
            ],
        ),
        (
            "plume",
            [
                ("ink-2 / bg-panel", 3.20, false),
                ("ink-2 / bg-panel-alt", 3.11, false),
                ("ink-2 / bg-raised", 2.87, true),
            ],
        ),
    ] {
        host::apply_theme(
            &ui,
            Theme::from_name(skin).expect("皮肤名必须是合法 --theme 取值"),
        );
        let faint = tokens.get_ink_2();
        let surfaces = [
            tokens.get_bg_panel(),
            tokens.get_bg_panel_alt(),
            tokens.get_bg_raised(),
        ];
        for ((label, expected, shortfall), background) in recorded.iter().zip(surfaces.iter()) {
            let ratio = contrast_ratio(faint, *background);
            let rounded = (ratio * 100.0).round() / 100.0;
            assert!(
                (rounded - expected).abs() < f64::EPSILON,
                "`[{skin}] {label}` 的 `ink-2` 实测记录过期: {ratio:.4}:1 (两位小数 \
                 {rounded:.2}), 登记的是 {expected:.2} —— 任一侧改色都必须同时更新这条记录"
            );
            assert_eq!(
                ratio < 3.0,
                *shortfall,
                "`[{skin}] {label}` 的 WCAG 缺口登记与实际不符: 实测 {ratio:.2}:1, \
                 登记 shortfall={shortfall} —— 要么记录过期, 要么颜色被改过"
            );
        }
    }

    // ---- ①d-bis 两支新皮肤的**顶栏两块板**：登记比值，低于 yeban 那一档 ----
    //
    // ①d 的门槛 1.10:1 是 `yeban` 的取舍留下的（`ui/tokens.slint` §6c）。负责人修订版
    // 调色板里 `bg-head`(→ bg-panel-alt) 到 `bg-raise`(→ bg-control) 只有一级，实测：
    //   inkmoor 1.08:1 / plume 1.08:1 —— 都**低于** yeban 的 1.11:1。
    // 因此**不**把 ①d 的硬门槛套到这两支上（套了就会把负责人的颜色判红），改成把数字
    // 钉住：任一侧改色都要同时更新这条记录，缺口不会被悄悄"修好"或悄悄变大。
    for (skin, expected) in [("inkmoor", 1.08_f64), ("plume", 1.08_f64)] {
        host::apply_theme(
            &ui,
            Theme::from_name(skin).expect("皮肤名必须是合法 --theme 取值"),
        );
        let slab = contrast_ratio(tokens.get_bg_panel_alt(), tokens.get_bg_control());
        let rounded = (slab * 100.0).round() / 100.0;
        assert!(
            (rounded - expected).abs() < f64::EPSILON,
            "`{skin}` 顶栏两块板 (bg-panel-alt ↔ bg-control) 的亮度比记录过期: \
             实测 {slab:.4}:1 (两位小数 {rounded:.2}), 登记的是 {expected:.2}"
        );
    }

    // 下面的断言（①c-bis 的缺口记录、①d 的顶栏门槛、② 的参照值）都是在 `yeban` 的
    // 取值上做的 —— 把主题**写回**去，免得上面三块把观测面留在别的皮肤上。
    host::apply_theme(&ui, Theme::Yeban);

    // ---- ①c-bis 负责人调色板的**已知 WCAG 缺口**（报告, 不擅自改色） ----
    //
    // 负责人的 `--txt-faint` 霜灰 `#5a6672` 画在比 `--bg-void` 亮的墨阶上时低于 3:1。
    // 这不是我们抄错了数字 —— 是这副调色板自身的结果。处理方式:
    //   * **不**改负责人的颜色（那是权威设计输入）;
    //   * **不**把这一对从判据里删掉（那才是削弱门禁）;
    //   * 改成**把缺口钉住**: 记录每一对的实测比值, 并断言「它确实低于 3:1」。
    // 任何一侧的十六进制被改动（无论朝好还是朝坏）, 这三条都会红 —— 缺口不会被悄悄
    // 「修好」或悄悄变大。裁决权在负责人: 要么接受这个缺口, 要么由他们改自己的颜色。
    let faint_shortfall: [(&str, Color, Color, f64); 3] = [
        (
            "ink-2 / bg-panel",
            tokens.get_ink_2(),
            tokens.get_bg_panel(),
            2.99,
        ),
        (
            "ink-2 / bg-panel-alt",
            tokens.get_ink_2(),
            tokens.get_bg_panel_alt(),
            2.86,
        ),
        (
            "ink-2 / bg-raised",
            tokens.get_ink_2(),
            tokens.get_bg_raised(),
            2.57,
        ),
    ];
    for (label, a, b, recorded) in faint_shortfall {
        let ratio = contrast_ratio(a, b);
        let rounded = (ratio * 100.0).round() / 100.0;
        assert!(
            (rounded - recorded).abs() < f64::EPSILON,
            "负责人调色板的 `--txt-faint` 缺口记录过期: {label} 实测 {ratio:.4}:1 \
             (两位小数 {rounded:.2}), 登记的是 {recorded:.2} —— 任一侧改色都必须同时更新这条记录"
        );
        assert!(
            ratio < 3.0,
            "`{label}` 被登记为负责人调色板的 WCAG 缺口, 但实测 {ratio:.2}:1 ≥ 3:1 ⇒ \
             要么记录过期, 要么颜色被改过; 请复核 ui/tokens.slint §6b"
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
    // 强调色也必须真的不同 (否则「唯一强调色只用一次」就只是把 AI 语义色换了个名字)。
    // 2026-10-07（`HD-54`）之后这两支还在**两个不同的落点**上: `accent` 画录音键的 ●,
    // `ai-suggestion` 画 AI 徽章的描边 —— 它们不同色正是"暖强调离开了 AI 徽章"的见证。
    assert_ne!(
        tokens.get_accent(),
        tokens.get_ai_suggestion(),
        "`yeban` 的 `accent`（录音键的 ●）不许等于 `ai-suggestion`（AI 徽章描边）—— \
         那意味着强调色其实没换"
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

/// **唯一一份**真实配对表（前景 / 背景 / 阈值），从**当前** token 取值里读。
///
/// 阈值是 WCAG 2.1 的原值，一条都没为某支皮肤放宽：正文 7:1 (AAA) / 4.5:1 (AA) /
/// 3:1 (AA 大字与非文本 UI 边界 [1.4.11])。表只写一遍 ⇒ `yeban` / `inkmoor` / `plume`
/// 三支皮肤测的是**同一组**配对，不会出现"某一支偷偷少测几对"。
///
/// 2026-10-07（`HD-54`）：唯一强调色的落点从 AI 徽章描边移到**录音键的 ●**，因此这张表
/// **随落点一起移动**（16 → 18 对）：
///   * `accent / bg-panel-alt` —— 录音键的 ● 静置时画在 `bg-panel-alt` 上（原来这一对读
///     的是"AI 徽章描边画在它自己的板上"，同样的两支令牌、同一个背景）;
///   * `accent / bg-raised` —— **新增**：`transport-record-button` 按下时板变成 `bg-raised`
///     （`transport.slint` 的 `background: rec_area.pressed ? …`），● 就在它上面;
///   * `ai-suggestion / bg-panel-alt` —— **新增**：AI 徽章的描边改用 `ai-suggestion` 之后，
///     这是徽章那一处的真实配对（旧落点腾空后也得有人看着）。
fn wcag_cases(tokens: &Tokens<'_>) -> [(&'static str, Color, Color, f64); 18] {
    [
        (
            "ink-0 / bg-void",
            tokens.get_ink_0(),
            tokens.get_bg_void(),
            7.0,
        ),
        (
            "ink-0 / bg-lane",
            tokens.get_ink_0(),
            tokens.get_bg_lane(),
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
            "ink-2 / bg-void",
            tokens.get_ink_2(),
            tokens.get_bg_void(),
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
            "gold / bg-raised",
            tokens.get_gold(),
            tokens.get_bg_raised(),
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
            "gold-deep / bg-void",
            tokens.get_gold_deep(),
            tokens.get_bg_void(),
            4.5,
        ),
        (
            // 落点（`HD-54`）= 录音键的 ● 静置在 `bg-panel-alt` 上。
            "accent / bg-panel-alt",
            tokens.get_accent(),
            tokens.get_bg_panel_alt(),
            3.0,
        ),
        (
            // 落点（`HD-54`）= 录音键**按下**时的板（`bg-raised`）。
            "accent / bg-raised",
            tokens.get_accent(),
            tokens.get_bg_raised(),
            3.0,
        ),
        (
            // `HD-54` 腾空后的旧落点：AI 徽章的描边现在画 `ai-suggestion`。
            "ai-suggestion / bg-panel-alt",
            tokens.get_ai_suggestion(),
            tokens.get_bg_panel_alt(),
            3.0,
        ),
        (
            "selection / bg-panel-alt",
            tokens.get_selection(),
            tokens.get_bg_panel_alt(),
            3.0,
        ),
        (
            "playing / bg-panel-alt",
            tokens.get_playing(),
            tokens.get_bg_panel_alt(),
            3.0,
        ),
        (
            "record-red / bg-void",
            tokens.get_record_red(),
            tokens.get_bg_void(),
            3.0,
        ),
    ]
}

// ---------------------------------------------------------------------------
// 判据 ⑤/⑥/⑦: 源码钉 —— 不碰 Slint, 直接把 `ui/**/*.slint` 当文本读
// ---------------------------------------------------------------------------
//
// 为什么还要一条源码判据（运行时回读已经是判据 ④）: 两者抓的是**不同**的漂移。
// ④ 抓"编译出来的值 != 负责人下发的表"; ⑤ 抓"源码里的字面量 != 表" —— 例如有人把
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

/// 取出某个 token 那一行里的**前四个** `#rrggbb`（顺序 = brand、yeban、inkmoor、plume）。
///
/// 2026-10-07 起每支主题令牌都必须写死**四支自绘调色板**的字面量分支（`brand` /
/// `yeban` / `inkmoor` / `plume`），缺任何一支都是漂移 ⇒ 这里的下界从 2 提到 4。
/// 走设计系统的分支写的是 `Palette.*`，不是字面量，因此不计入。
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
        out.len() >= 4,
        "`{needle}` 那一行必须同时写死 brand / yeban / inkmoor / plume 四个字面量 \
         (否则某支自绘调色板会静默回落到 Palette 或其他皮肤); 实际={out:?}"
    );
    out
}

/// **判据 ⑤**：`ui/tokens.slint` 里每个主题令牌的**四个**自绘字面量分支都等于负责人
/// 下发的表（`brand` = 本仓品牌色 / 另三支 = 负责人两版设计稿）。
///
/// 表里的 inkmoor / plume 值与判据 ④ 是**同一份数字的两个独立观测面**（一个读源码、
/// 一个读活组件）；两条判据抓的是**不同**的漂移（见文件头的说明）。
#[test]
fn the_theme_palette_literals_in_the_source_are_the_measured_ones() {
    let sources = all_ui_sources();
    let (_, tokens_slint) = sources
        .iter()
        .find(|(path, _)| path.ends_with("ui/tokens.slint"))
        .expect("必须存在 ui/tokens.slint");

    // (token, brand, yeban, inkmoor, plume) —— 四支自绘调色板各一列。
    // `brand` / `yeban` 两列是 2026-10-07 之前的既有值；`yeban` 列本切片之后仍然一位未改，
    // `brand` 列**只有 `accent` 一行**因 `HD-54` 的落点移动而改动（#a855f7 → #d94a4a，
    // 见那一行的注释）；
    // `inkmoor` / `plume` 两列是负责人设计稿（HTML mock）的原话，逐条映射见 §6e。
    let table: [(&str, &str, &str, &str, &str); 23] = [
        ("bg-void", "#060a14", "#0e1216", "#0e1114", "#14100d"),
        ("bg-shell", "#0d1326", "#11161c", "#12161b", "#181310"),
        ("bg-lane", "#0d1326", "#11161c", "#12161b", "#181310"),
        ("bg-lane2", "#060a14", "#0d1116", "#0f1317", "#151110"),
        ("bg-panel", "#151d38", "#131a22", "#141920", "#1b1512"),
        ("bg-head", "#0d1326", "#161e28", "#171c24", "#1f1814"),
        ("bg-panel-alt", "#1b2447", "#161e28", "#171c24", "#1f1814"),
        ("bg-raised", "#232f5c", "#1e2733", "#1d232d", "#271f19"),
        ("bg-control", "#1b2447", "#1e2733", "#1d232d", "#271f19"),
        ("line", "#1e2745", "#212b36", "#20242b", "#2a2520"),
        ("line-strong", "#2c3a63", "#2c3948", "#282d35", "#332d26"),
        ("ink-0", "#f0ebe3", "#d7dee1", "#e9e7e2", "#efe7d9"),
        ("ink-1", "#c8c2b8", "#8d99a5", "#a3a7ae", "#aba091"),
        ("ink-2", "#5a6b8a", "#5a6672", "#6a6f78", "#6f665a"),
        ("gold-bright", "#f7e6b0", "#e7dfc8", "#efe9da", "#eae2d3"),
        ("gold", "#e2c77e", "#c6a47c", "#c9a26b", "#c68252"),
        ("gold-deep", "#b8933e", "#6d88a1", "#7c93a8", "#6f8d96"),
        // 2026-10-07（`HD-54`）: `accent` 的 brand 列随**落点**改动 —— 落点是录音键的 ●,
        // 而它原来画的是 `record-red` 的 brand 值 #d94a4a, 所以 `accent` 的 brand 分支
        // 必须写 #d94a4a（yeban / inkmoor / plume 三列一位未改）。
        ("accent", "#d94a4a", "#c6a47c", "#c9a26b", "#c68252"),
        ("ai-suggestion", "#a855f7", "#8e86a6", "#9e97ae", "#9c93a2"),
        ("record-red", "#d94a4a", "#ac6e60", "#b4715f", "#b25c43"),
        ("playing", "#e2c77e", "#6e9488", "#85a794", "#7f9483"),
        ("selection", "#f7e6b0", "#8e86a6", "#9e97ae", "#9c93a2"),
        ("piano-key", "#f0ebe3", "#c8c2b2", "#ccc5b4", "#c2ad91"),
    ];
    for (token, brand, yeban, inkmoor, plume) in table {
        let found = token_literals(tokens_slint, token);
        assert_eq!(
            found[0], brand,
            "`{token}` 的 brand 分支必须仍是 {brand}（默认外观逐像素不变）; 实际 {}",
            found[0]
        );
        assert_eq!(
            found[1], yeban,
            "`{token}` 的 yeban 分支必须是负责人下发的 {yeban}（出处见 §6b）; 实际 {}",
            found[1]
        );
        assert_eq!(
            found[2], inkmoor,
            "`{token}` 的 inkmoor 分支必须是负责人「墨泊 InkMoor」下发的 {inkmoor} \
             （出处见 §6e）; 实际 {}",
            found[2]
        );
        assert_eq!(
            found[3], plume,
            "`{token}` 的 plume 分支必须是负责人「孤烟 Plume」下发的 {plume} \
             （出处见 §6e）; 实际 {}",
            found[3]
        );
    }

    // 品牌色 `#151d38` 这条**既有**的字面量判据（本仓库"品牌色从母版 SVG 提取"那条）
    // 必须仍然成立: 它一旦消失, "默认主题 = 今天的外观"就没有源码侧的见证。
    assert!(
        tokens_slint.contains("#151d38"),
        "`ui/tokens.slint` 必须仍然写死品牌面板色 `#151d38`"
    );
}

/// **判据 ⑥**：唯一的强调色在整份界面里**恰好引用一次**，而且那一处就是**录音键的 ●**
/// （2026-10-07 `HD-54` 把落点从 AI 提案徽章的描边移到了这里）。
///
/// 这是"强调色必须唯一"那条设计原则的机械形态：任何第二处引用（例如顺手也把保存键
/// 描成 渔火、或把 AI 徽章的描边改回 `accent`）都会让这条红。`bg-control` 同样只准
/// 一处 —— 它是为那一块板而加的。
///
/// 落点怎么钉（判据随 `HD-54` **一起移动**）：那一行必须是**颜色**表达式（不是
/// `border-color`），而且从它往上数最近的一个 `accessible-id` 必须是
/// `transport-record-button` —— 也就是它画在录音键上。旧落点同时被**反向**钉住：
/// AI 徽章最近的那条 `border-color` 必须是 `Tokens.ai-suggestion`，不许再是 `accent`。
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

    // (文件, 行号, 那一行, 从它往上数最近的 `accessible-id`)
    let mut hits: Vec<(String, usize, String, Option<String>)> = Vec::new();
    for (path, text) in &sources {
        let mut enclosing: Option<String> = None;
        for (line_no, line) in text.lines().enumerate() {
            if let Some(id) = line
                .split("accessible-id:")
                .nth(1)
                .and_then(|rest| rest.split('"').nth(1))
            {
                enclosing = Some(id.to_string());
            }
            if line.contains("Tokens.accent") {
                hits.push((
                    path.clone(),
                    line_no + 1,
                    line.trim().to_string(),
                    enclosing.clone(),
                ));
            }
        }
    }
    assert_eq!(
        hits.len(),
        1,
        "`Tokens.accent`（唯一强调色）必须**恰好**出现一次, 实际 {} 处: {hits:#?}",
        hits.len()
    );
    let (path, line_no, line, enclosing) = &hits[0];
    assert!(
        path.ends_with("ui/transport.slint"),
        "唯一那一处必须仍住在顶栏 `transport.slint`（录音键与 AI 徽章都在那里）; \
         实际 {path}:{line_no}"
    );
    assert!(
        line.contains("color:") && !line.contains("border-color:"),
        "唯一那一处必须是**颜色**表达式（录音键 ● 的 `color:`），不是描边 —— `HD-54` 的 \
         落点是录音键; 实际 {path}:{line_no}: {line}"
    );
    assert_eq!(
        enclosing.as_deref(),
        Some("transport-record-button"),
        "唯一那一处必须画在**录音键**上（`HD-54`：暖强调的落点 = 录音键的 ●）; \
         实际它属于 {enclosing:?}（{path}:{line_no}: {line}）"
    );

    // 旧落点（`HD-54` 移走的那一处）必须真的腾空: AI 徽章最近的一条 `border-color`
    // 只能是 `Tokens.ai-suggestion`。把徽章的描边改回 `accent` 会先撞上面那条
    // "恰好一次", 再撞这一条。
    let (_, transport) = sources
        .iter()
        .find(|(p, _)| p.ends_with("ui/transport.slint"))
        .expect("必须存在 ui/transport.slint");
    let badge = transport
        .lines()
        .position(|l| l.contains("accessible-id: \"transport-ai-proposal-badge\""))
        .expect("transport.slint 里必须有 AI 提案徽章");
    let (border_no, border) = transport
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("border-color:"))
        .min_by_key(|(i, _)| i.abs_diff(badge))
        .expect("transport.slint 里必须有 `border-color:`");
    assert!(
        border.contains("Tokens.ai-suggestion"),
        "AI 徽章（旧落点）的 `border-color` 必须是 `Tokens.ai-suggestion` —— 暖强调移走后 \
         它必须换角色, 且换到的正是 brand 值等于移动前 `accent` 的那一支; \
         实际 ui/transport.slint:{}: {}",
        border_no + 1,
        border.trim()
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
        "`ai-suggestion` 仍是 AI 建议层的语义色（AI 徽章**改用** `ai-suggestion` 之后它 \
         也不该只在徽章上）"
    );

    // 2026-10-07 新增的一支: 负责人把「选区=愁眠」与「播放头=月华」分给了**两个**令牌
    // ⇒ 两支都必须真的被用到。把 `selection` 全量换回 `gold-bright`、或把 `gold-bright`
    // 删掉（让「选中」与「播放头」又合成一支）, 都会让这条红。
    for token in ["Tokens.selection", "Tokens.gold-bright"] {
        let files = files_with(token);
        assert!(
            !files.is_empty(),
            "`{token}` 必须真的被 ui/ 用到 —— 负责人的调色板把「选区/激活/AI」(`selection`) \
             与「播放头/时间码/读数」(`gold-bright`) 分成了两个颜色; 实际 {files:#?}"
        );
    }
}

/// **判据 ⑦a**：负责人原则 ② 的机械形态 —— 渔火 是**唯一**的暖强调色。
///
/// 观测面是 `ui/tokens.slint` 的**源码文本**（不是活组件）：本文件只有 ③ 那一个测试
/// 允许碰 Slint 进程内对象（平台是线程局部且每线程只能装一次），所以这条走文本。
///
/// 三条断言，各自能独立变红：
///   ① 主强调的两支（`accent` 与 `gold`）在 yeban 下必须**都是** 渔火 `#c6a47c` ——
///     负责人把「片段/主强调」与「唯一强调色」给了同一个色; 改任一支都红。
///   ② 渔火 必须是**暖**色相（H ∈ [0°,60°]）。
///   ③ 功能/AI 侧的两支强调（`gold-deep` 客船、`selection` 愁眠）必须是**冷**色相
///      （H ∈ [150°,300°]）—— 也就是说, 它们不是第二个暖强调。
///
/// 2026-10-07 `HD-54` 把 `accent` 的**落点**移到录音键的 ●（落点本身由判据 ⑥ 钉住），
/// 因此这条"暖强调"判据长出两条随落点走的断言：
///   ④ **旧落点必须冷下来** —— AI 徽章的新描边 `ai-suggestion` 在 yeban 下必须是冷色相
///      （否则暖强调只是从一个暖色换成另一个暖色, 落点根本没动）;
///   ⑤ **录音键的功能色不许顶替** —— `accent` 不许等于 `record-red`（相等就说明这次
///      "移动"只是把录音键原来的红改了个名, 暖强调并没有真的落到录音键上）。
///
/// 为什么用「指定的两支必须同色 + 其余强调必须冷」而不是「数暖色相的个数」：负责人
/// 自己的调色板里 月华 `#e7dfc8` 与 江枫 `#ac6e60` 也在暖区（前者是播放头、后者是
/// 录音功能色），所以"全界面只有一个暖色相"**不是**他们的规格；他们的规格是
/// "只有一个暖**强调**色"。这条判据写的就是那句话。
#[test]
fn the_single_warm_accent_is_yuhuo_and_the_other_accents_are_cool() {
    let sources = all_ui_sources();
    let (_, tokens_slint) = sources
        .iter()
        .find(|(path, _)| path.ends_with("ui/tokens.slint"))
        .expect("必须存在 ui/tokens.slint");

    let yeban_of = |token: &str| token_literals(tokens_slint, token)[1].clone();
    let accent = yeban_of("accent");
    let gold = yeban_of("gold");
    assert_eq!(
        accent, "#c6a47c",
        "yeban 的 `accent` 必须是负责人下发的 渔火 #c6a47c; 实际 {accent}"
    );
    assert_eq!(
        gold, "#c6a47c",
        "yeban 的 `gold`（片段/音符头/章节名 = 主强调）必须是 渔火 #c6a47c; 实际 {gold}"
    );
    assert_eq!(
        accent, gold,
        "负责人的原则 ② 是「渔火 是全界面唯一的暖强调色」⇒ `accent` 与 `gold` 必须同色"
    );

    let warm = hue_of(&accent).expect("渔火 必须有彩度");
    assert!(
        !(60.0..300.0).contains(&warm),
        "渔火 必须在暖色区 (H<60° 或 H>300°), 实测 H={warm:.1}°"
    );

    for token in ["gold-deep", "selection"] {
        let hex = yeban_of(token);
        let hue = hue_of(&hex).expect("这两支必须有彩度");
        assert!(
            (150.0..=300.0).contains(&hue),
            "`{token}` (yeban {hex}) 必须是冷色相 (150°..=300°), 实测 H={hue:.1}° —— \
             否则它就是负责人原则 ② 禁止的第二个暖强调色"
        );
    }

    // ④ `HD-54`: 旧落点 (AI 徽章) 拿到的新角色必须是冷色 —— 否则暖强调没真的离开徽章。
    let badge = yeban_of("ai-suggestion");
    let badge_hue = hue_of(&badge).expect("ai-suggestion 必须有彩度");
    assert!(
        (150.0..=300.0).contains(&badge_hue),
        "yeban 的 `ai-suggestion`（AI 徽章 `HD-54` 之后的新描边色 {badge}）必须是冷色相 \
         (150°..=300°), 实测 H={badge_hue:.1}° —— 否则徽章上还留着第二个暖强调"
    );

    // ⑤ `HD-54`: 录音键的**功能**色不许顶替暖强调 —— 相等就说明这次"移动"只是把
    // 录音键原来的红改了个名。(brand 下两者**故意**相等: 那是默认像素不变的机械前提,
    // 见测试 ③ 的 ① 段与 `ui/tokens.slint` §6e 的「值重合」登记。)
    assert_ne!(
        accent,
        yeban_of("record-red"),
        "yeban 的 `accent`（录音键的 ●）不许等于 `record-red`（录音功能色）—— 相等说明\
         暖强调只是被改了个名, 并没有落到录音键上"
    );
}

/// **判据 ⑦a-bis**：两支新皮肤的「唯一暖强调」也必须是**机械形态**的，而不是一句形容。
///
/// 观测面与 ⑦a 完全相同（`ui/tokens.slint` 的**源码文本**，不碰 Slint 进程内对象），
/// 断言结构也相同：指定的两支必须同色（`accent` == `gold`），该色必须是**暖**色相，
/// 而功能/AI 侧的两支强调（`gold-deep`、`selection`）必须是**冷**色相 —— 也就是说，
/// 它们不是第二个暖强调。
///
/// 负责人两版稿子对这件事的措辞不同但同义：
///   * 墨泊：「渔火 `#C9A26B` 是全界面**唯一**的暖强调色」（原则 ②）；
///   * 孤烟：「落日橙 `#C68252` …… 全界面唯一暖强调」（诗句→UI 语义表）。
///
/// 因此两支皮肤的 `accent` 与 `gold` 各自同色，但**它们彼此不同色** —— 这一条也断言。
///
/// 2026-10-07 `HD-54` 之后（落点 = 录音键的 ●，落点本身由 ⑥ 钉住）这条还逐支断言：
///   ④ 旧落点冷下来 —— `ai-suggestion`（AI 徽章的新描边色）必须是冷色相;
///   ⑤ 功能色不许顶替 —— `accent` 不许等于 `record-red`（否则只是改了个名）。
#[test]
fn the_two_new_skins_keep_one_warm_accent_and_cool_counterparts() {
    let sources = all_ui_sources();
    let (_, tokens_slint) = sources
        .iter()
        .find(|(path, _)| path.ends_with("ui/tokens.slint"))
        .expect("必须存在 ui/tokens.slint");

    // (皮肤, `token_literals` 里该皮肤的下标, 负责人下发的暖强调)
    let mut seen_accent: Vec<(&str, String)> = Vec::new();
    for (skin, index, expected) in [
        ("inkmoor", 2_usize, "#c9a26b"),
        ("plume", 3_usize, "#c68252"),
    ] {
        let of = |token: &str| token_literals(tokens_slint, token)[index].clone();
        let accent = of("accent");
        let gold = of("gold");
        assert_eq!(
            accent, expected,
            "`{skin}` 的 `accent`（唯一暖强调）必须是负责人下发的 {expected}; 实际 {accent}"
        );
        assert_eq!(
            gold, expected,
            "`{skin}` 的 `gold`（主强调 = 片段/音符头/章节名）必须是同一个暖强调色 \
             {expected}; 实际 {gold}"
        );
        assert_eq!(
            accent, gold,
            "`{skin}` 的设计原则是「全界面唯一的暖强调色」⇒ `accent` 与 `gold` 必须同色"
        );

        let warm = hue_of(&accent).expect("暖强调必须有彩度");
        assert!(
            !(60.0..300.0).contains(&warm),
            "`{skin}` 的暖强调必须在暖色区 (H<60° 或 H>300°), 实测 H={warm:.1}°"
        );

        for token in ["gold-deep", "selection"] {
            let hex = of(token);
            let hue = hue_of(&hex).expect("这两支必须有彩度");
            assert!(
                (150.0..=300.0).contains(&hue),
                "`{skin}` 的 `{token}` ({hex}) 必须是冷色相 (150°..=300°), 实测 \
                 H={hue:.1}° —— 否则它就是「唯一暖强调」原则禁止的第二个暖强调色"
            );
        }

        // ④ `HD-54`: 旧落点 (AI 徽章) 拿到的新角色必须是冷色。
        let badge = of("ai-suggestion");
        let badge_hue = hue_of(&badge).expect("ai-suggestion 必须有彩度");
        assert!(
            (150.0..=300.0).contains(&badge_hue),
            "`{skin}` 的 `ai-suggestion`（AI 徽章 `HD-54` 之后的新描边色 {badge}）必须是 \
             冷色相 (150°..=300°), 实测 H={badge_hue:.1}° —— 否则徽章上还留着第二个暖强调"
        );

        // ⑤ `HD-54`: 录音键的功能色不许顶替暖强调。
        assert_ne!(
            accent,
            of("record-red"),
            "`{skin}` 的 `accent`（录音键的 ●）不许等于 `record-red`（录音功能色）—— \
             相等说明暖强调只是被改了个名, 并没有落到录音键上"
        );
        seen_accent.push((skin, accent));
    }

    // 两支皮肤的暖强调**必须不同**（墨泊是渔火金、孤烟是落日橙）：它们一旦相同，
    // 上面那条"各自是唯一暖强调"就退化成"两支皮肤其实是同一支"。
    assert_ne!(
        seen_accent[0].1, seen_accent[1].1,
        "`{}` 与 `{}` 的唯一暖强调不该是同一个值",
        seen_accent[0].0, seen_accent[1].0
    );
}

/// **判据 ⑦c**：`--print-theme` 必须说出**这一支的 hex 从哪来**（`theme-source:` 行），
/// 而且**只对**真有设计出处的四支自绘调色板说 —— 走设计系统的四支不许有一行假出处。
///
/// 观测面是**真进程的 stdout**（不碰 Slint）：`--print-theme` 属于无窗口路径
/// （`Options::batch()`），因此它读不到 token 值 —— 它报的是**出处**，值由 ③/④/⑤ 钉。
#[test]
fn print_theme_reports_the_design_source_of_every_self_drawn_palette_and_no_fake_one() {
    for theme in Theme::ALL {
        let run = invoke(&["--theme", theme.name(), "--print-theme"]);
        assert_eq!(
            run.code,
            0,
            "`--theme {} --print-theme` 必须成功; stderr={}",
            theme.name(),
            run.stderr
        );
        match theme.design_source() {
            Some(source) => assert!(
                run.stdout.contains(&format!("theme-source: {source}")),
                "`--theme {}` 的出处行必须原样出现 (它是「这个 hex 从哪来」的唯一出口); \
                 stdout={}",
                theme.name(),
                run.stdout
            ),
            None => assert!(
                !run.stdout.contains("theme-source:"),
                "`--theme {}` 走的是设计系统 `Palette`, 没有「负责人下发的具体色值」\
                 这回事 ⇒ 不许打出处行; stdout={}",
                theme.name(),
                run.stdout
            ),
        }
    }
}

///
/// 观测面是全部 `ui/**/*.slint` 的**代码**（注释已剥掉：在注释里写"我们不用
/// `drop-shadow-*`"是文档，不是用法）。三条独立命中路径各自会红：
/// `drop-shadow-*` 系列属性、`@linear-gradient(...)`/`@radial-gradient(...)`/
/// `@conic-gradient(...)` 画笔、以及任何含 `shadow` 的标识符。
///
/// 为什么这条值得存在：这条纪律在本仓是**结构性成立**的（13 个 `.slint` 全部自绘
/// `Rectangle`，一个上游控件都没有），所以它很容易被"顺手加一个投影好看一点"打破，
/// 而那种改动不会撞上任何既有判据。
#[test]
fn the_ui_uses_no_shadow_and_no_gradient_brush() {
    let sources: Vec<(String, String)> = all_ui_sources()
        .into_iter()
        .map(|(path, text)| (path, strip_slint_comments(&text)))
        .collect();

    let needles = [
        "drop-shadow",
        "@linear-gradient",
        "@radial-gradient",
        "@conic-gradient",
        "gradient",
        "shadow",
    ];
    let mut hits = Vec::new();
    for (path, text) in &sources {
        for (line_no, line) in text.lines().enumerate() {
            let lowered = line.to_ascii_lowercase();
            for needle in needles {
                if lowered.contains(needle) {
                    hits.push(format!("{path}:{}: {}", line_no + 1, line.trim()));
                    break;
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "负责人原则 ④ 禁止投影与渐变光晕, 但 ui/ 的代码里出现了 {} 处: {hits:#?}",
        hits.len()
    );
}

/// 把 `#rrggbb` 解析成 `(r, g, b)`（0..=255）。
fn rgb_of(hex: &str) -> (f64, f64, f64) {
    assert_eq!(hex.len(), 7, "`{hex}` 必须是 `#rrggbb`");
    let channel = |at: usize| {
        f64::from(
            u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or_else(|err| panic!("{hex}: {err}")),
        )
    };
    (channel(1), channel(3), channel(5))
}

/// HSL 色相（度, `0..360`）。灰阶（max == min）返回 `None` —— 它没有色相, 既不是暖也不是冷。
fn hue_of(hex: &str) -> Option<f64> {
    let (r, g, b) = rgb_of(hex);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta == 0.0 {
        return None;
    }
    let hue = if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    Some(if hue < 0.0 { hue + 360.0 } else { hue })
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
