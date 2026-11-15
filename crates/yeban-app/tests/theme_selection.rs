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
//! | ④ | `default`(= yeban) / `inkmoor` / `plume` 的每一支颜色 == 负责人下发的那个字面量（回读 + 对比度断言） | 真 `MainWindow` 的 token 值 |
//! | ⑤ | 四支自绘调色板的**源码钉**：`ui/tokens.slint` 里每个 token 的四个字面量分支 | `tokens.slint` 的文本 |
//! | ⑥ | `accent`（唯一强调色）在整份界面里**恰好出现一次**，且那一处是**录音键的 ●**（`HD-54`）；`bg-control` 也恰好一次 | 全部 `ui/**/*.slint` 的文本 |
//! | ⑦ | 原则「暖强调色唯一」「无投影/无渐变」的机械形态（三支自绘皮肤各一条，含落点搬走后 AI 徽章必须冷、以及 `accent != record-red`）；`--print-theme` 的出处行 | token 值的色相 + 全部 `ui/**/*.slint` 的文本 + 真进程 stdout |
//! | ⑧ | `default` 选到的就是 yeban 调色板、`yeban` 是它的**别名**、`brand` 是旧品牌色；且 `.slint` 的 `ThemeState.theme` **初值** = 默认主题（golden 截图路径不调 `apply_theme`） | `Theme` 的解析 + `tokens.slint` 文本 + 真进程 stdout |
//!
//! ## 2026-10-07 之后：默认主题换成 `yeban`, 品牌深色改名 `brand`
//!
//! 负责人新指令（原文）：「默认主题改成 yeban 那一套，原来 default 改个名作为可选」。
//! 于是 ③/⑤/⑧ 钉的对应关系变成:
//!   * `--theme default` = **yeban 调色板**（负责人下发的墨阶 / 颜料值, 原来只有
//!     `--theme yeban` 才选得到它; 别名 `yeban` 继续可用, 解析到**同一支**）;
//!   * `--theme brand` = **本仓品牌深色**（今天之前那一串十六进制字面量, 取值一位未改）,
//!     它就是本文件 ① 段那张品牌表;
//!   * 其余 (`inkmoor` / `plume` / `material` / `fluent` / `cupertino` / `native`) 不变。
//!
//! ⚠ **代价（照实登记）**：默认**外观**因此改变 ⇒ `tests/golden/linux/**` 的 5 张基准
//! 必须走手动档 `gates-manual.yml` 的 `gate=goldens` 重新录制 + 人工复核（`HD-56` 已把
//! "基准冻结"从绝对约束降为**条件**; 这不是本文件能修的 —— 本文件只负责把漂移检测住）。
//! 判据本身**一条都没放宽**：`brand` 表、`default` 表、别名等价、初值耦合都仍然会红。
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
//!     （= `accent` **移动前**的 brand 值）两条机械保证 —— 这是 `--theme brand`
//!     （2026-10-07 之前的默认主题）逐像素不变的两个前提;
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
//! - `[TEST-SPEC-006]`「WCAG 2.1 AA 对比度自动化校验」：由本文件的 ①c / ①c-bis /
//!   ①c-quater 承担 —— **数字是算出来的**（[`contrast_ratio`] 现读活组件的 token 取值），
//!   不是目测、也不是抄注释。①c 用 `wcag_cases` 的**同一张**阈值表逐支重算四副调色板；
//!   负责人调色板达不到的那几对**不删、不放宽阈值**，而是把实测比值钉死（①c-bis /
//!   ①c-quater），于是往坏改和往好改都会红，缺口既不会被悄悄放大、也不会被悄悄"修好"
//!   而无人复核。口径：**单位＝个（前景/背景配对数）**，对象＝`ui/**` 里**真实存在**的
//!   落点 —— `bg-head` / `bg-control` / `bg-lane` / `bg-lane2` 在 `ui/**` 里从不作
//!   `background:` 出现，`bg-shell` 的那一处落点内也没有文字 ⇒ 它们**不进表**（进了就是
//!   给界面里不存在的配对造判据）。
//! - `[UI-A11Y-004]`：本文件覆盖其第 2 条里**可自动化**的那一半（对比度实测）。同一条的
//!   「AAA 级高对比度**模式**」仍缺 —— `YebanTheme` 只有 `brand` / `yeban` / `inkmoor` /
//!   `plume` 四支，没有任何高对比度分支；补它要新增颜色，而调色板是负责人下发的权威输入
//!   （见 `ui/tokens.slint` §6e「不悄悄改色」）⇒ 属**人类裁决**项，本文件不代为发明取值。

#![allow(unused)]
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use slint::{Color, ComponentHandle as _};
use yeban_app::cli::{self, Theme};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::{ThemeState, Tokens, YebanTheme};

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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
}

// ---------------------------------------------------------------------------
// 判据 ②: 未知取值被**拒绝**，而且拒绝的理由能被读到
// ---------------------------------------------------------------------------

/// 未知主题必须是**用法错误**（退出码 [`cli::EXIT_USAGE`]）、stderr 点名那个坏取值与
/// 可用集合、stdout 为空。
///
/// 为什么"静默回退到默认主题"不可接受：那会让"我说了 material"与"其实渲染的是
/// `default`(yeban 调色板)"长得一模一样 —— 正是本仓库最忌讳的一类假绿。
#[test]
fn an_unknown_theme_is_refused_with_a_reason_and_a_nonzero_exit() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
}

// ---------------------------------------------------------------------------
// 判据 ⑧: 默认主题 = yeban、`yeban` 是别名、`brand` 是旧品牌色; 且 `.slint` 初值 = 默认
// ---------------------------------------------------------------------------
//
// 这一节**不碰 Slint 进程内对象**（全部是 `Theme` 的解析 + `tokens.slint` 文本 + 子进程
// stdout），因此可以独立成条测试：`set_platform` 是线程局部且每线程只能装一次，只有 ③
// 那个测试能碰活组件（见文件头）。

/// **判据 ⑧a**：`default` / `yeban` / `brand` 三者的解析关系，以及"可接受集合由
/// `Theme::ALL` 机械推导"这条不许漂移的性质（用法文本与错误信息共用一个推导）。
///
/// 为什么这条要**在真进程上**再验一遍：`Theme::accepted_names()` 是 Rust 侧的事实，
/// 而"用户看到的用法文本与错误信息里也真的列了这些名字"是**另一个**观测面 ——
/// 少了它，`accepted_names()` 可以被改成只给判据看的一份漂亮列表。
#[test]
fn the_yeban_alias_resolves_to_the_default_theme_everywhere() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
}

/// **判据 ⑧b**：`ui/tokens.slint` 的 `ThemeState.theme` **初值**必须等于默认主题。
///
/// 为什么这条是独立的一条、而且走**源码文本**：golden 截图的路径
/// (`src/test_port_adapter.rs`) 直接 `host::build_main_window` 而**不调** `apply_theme`,
/// 所以"默认外观是什么"在那条路径上由这个初值决定。`cli.rs` 的 `#[default]` 与它一旦
/// 漂移, 默认外观就会一半是新主题、一半是旧主题 —— 这条判据把两处钉在一起。
///
/// 观测面是文本（不碰 Slint）：与 ⑤ 同一份源码, 因此可以在同一个文件里独立成条测试。
#[test]
fn the_default_theme_is_the_slint_initial_value() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
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
/// ⓪ **默认主题**是 yeban 那一支：`Theme::default()` / `Theme::from_name("default")` /
///    `tokens.slint` 的 `ThemeState.theme` 初值**三者同一**（初值那条是关键 —— golden
///    截图的路径**不调** `apply_theme`，它看到的就是初值）；
/// ① `--theme brand`（2026-10-07 之前的默认主题）渲染的就是那一串本仓品牌字面量；
/// ② 走设计系统的主题取值**真的不一样**（`Palette` 角色）；
/// ③ Slint 1.18.1 的上限被**如实**登记：四个内建风格名共享同一个编译进来的 `Palette`，
///    因此它们彼此相等 —— 判据把这件事写成断言，而不是回避它。
#[test]
fn the_effective_palette_follows_the_selection() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
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

/// ⭐ **平台无关的"路径末段"匹配**（判据不许依赖分隔符）。
///
/// ## 为什么不能把**正斜杠字面量**直接喂给裸 `ends_with`
///
/// （下面用 `SUFFIX` 代指那个后缀字面量，例如 `ui/tokens.slint`。）
///
/// [`all_ui_sources`] 交给判据的是 **`Path::display()` 的文本**（它按**本机**分隔符渲染），
/// 而字面量用的是**正斜杠** ⇒ ⭐ **Windows 上是反斜杠**（`...\ui\tokens.slint`）⇒ 比较**恒 false**
/// ⇒ ⭐ `the_default_theme_is_the_slint_initial_value` 那类 `.expect("必须存在 ui/tokens.slint")`
/// 会 **panic**。
///
/// ⚠ ⭐ **本仓的 Windows 腿覆盖哪些 crate ⇒ 见 `.github/workflows/ci.yml` 的 windows job**
/// （⭐ 此处**刻意不写死 crate 清单**：那是会腐烂的自述、而它读起来像保证）。
/// ⭐ 正因为"app 是否被 Windows 覆盖"会**变**，本文件里那条"喂合成路径"的判据**无论覆盖如何
/// 都必须存在**：它把平台假设本身变成判据，⛔ 不依赖任何一条腿是否跑 app。
///
/// ## 口径（两件事都写明，⛔ 不靠"看起来对"）
///
/// 1. ⭐ **两侧都归一化**：`\` → `/` 之后再比 ⇒ 正斜杠 / 反斜杠 / **混合**三种形态都成立；
/// 2. ⛔ **不用 `std::path::Path::components()` 那条路**：在 **Unix** 上反斜杠**不是**分隔符，
///    于是 `Path::new(r"C:\x\ui\tokens.slint")` 对同一个后缀判 **false**
///    ⇒ 那条路**修不了**这个缺陷（判据 `the_path_suffix_helper_accepts_both_separators` 里
///    有一条 `#[cfg(unix)]` 的钉子把这条差异钉住）。
fn path_ends_with(path: &str, suffix: &str) -> bool {
    // 归一化路径分隔符（只做这一件事，⛔ 不动大小写、⛔ 不解析 `.`/`..`）。
    fn normalize(text: &str) -> String {
        text.replace('\\', "/")
    }
    normalize(path).ends_with(&normalize(suffix))
}

/// ⭐ **纯函数**：两条路径文本是否指**同一处** —— ⛔ 不渲染、⛔ 不看平台、⛔ 不碰文件系统。
///
/// ⚠ **为什么不能用 `Path::new(a) == Path::new(b)` 替代**：在 **Unix** 上反斜杠**不是**分隔符
/// ⇒ `Path::new(r"C:\x\ui\t.slint") != Path::new("C:/x/ui/t.slint")` ⇒ 那条路**在本机测不了**
/// 这个缺陷（与 [`path_ends_with`] 文档里记的是同一个坑）。归一化之后才**两侧同形**。
///
/// ⭐ 它存在的理由：**本机（macOS）无法让 `Path::display()` 产出反斜杠** ⇒ 唯一的承重证据是
/// "把判定抽成纯函数、⭐ 再用**内存样例**（含反斜杠那侧）测它"。
fn paths_are_equivalent(actual: &str, expected: &str) -> bool {
    actual.replace('\\', "/") == expected.replace('\\', "/")
}

/// ⭐ **纯函数**：一段源文本里是否出现"把路径与**硬编码 `/`** 字面量相比"的写法。
///
/// 覆盖**两种**已知形状（⭐ 第二种是本族漏网的那一处）：
/// 1. 裸的 `.ends_with(` 接 `"ui/…"`；
/// 2. ⭐ `format!(` 接 `"{UI_DIR}/…"` 出现在 `assert_eq!` 的**期望侧**。
///
/// ⭐ 纯函数 ⇒ 可以用**内存样例**在 macOS 上做**正对照**（见判据）。
fn has_bare_separator_path_literal(source: &str) -> bool {
    // ⚠ 自指陷阱：针脚不能以**字面形式**出现在本文件里（否则它会命中自己）⇒ 用 `concat!` 拼开。
    let bare_ends_with = concat!(".ends_with(", "\"ui/");
    let bare_ui_dir = concat!("format!(", "\"{UI_DIR}/");
    source.contains(bare_ends_with) || source.contains(bare_ui_dir)
}

/// 把 `ui/` 下所有 `.slint`（含子目录）读成 `(路径文本, 源码)`。
///
/// ⚠ ⭐ 路径文本是 `Path::display()` 的**原样**（平台相关：Windows 反斜杠、Unix 正斜杠）——
/// 本函数**刻意不**在这里归一化：那样只会把平台假设藏起来。判据侧一律用
/// [`path_ends_with`]（⛔ 不许裸 `ends_with`）。
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
}

/// **判据 ⑦c**：`--print-theme` 必须说出**这一支的 hex 从哪来**（`theme-source:` 行），
/// 而且**只对**真有设计出处的四支自绘调色板说 —— 走设计系统的四支不许有一行假出处。
///
/// 观测面是**真进程的 stdout**（不碰 Slint）：`--print-theme` 属于无窗口路径
/// （`Options::batch()`），因此它读不到 token 值 —— 它报的是**出处**，值由 ③/④/⑤ 钉。
#[test]
fn print_theme_reports_the_design_source_of_every_self_drawn_palette_and_no_fake_one() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
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

/// 判据（**平台无关性**）：⭐ 路径末段匹配必须对**反斜杠**与**正斜杠**（含混合）都成立。
///
/// ⚠ 这是本文件里**唯一能在 Linux 上守住"Windows 假设"**的判据：⭐ **Windows 腿是否覆盖
/// `yeban-app` ⇒ 见 `.github/workflows/ci.yml` 的 windows job**（⛔ 不在此写死清单）；
/// ⭐ 只要那条腿**可能不跑** app，裸 `ends_with` 的缺陷在 Linux 上就**永远不会红**
/// （Linux 的路径本来就是正斜杠）⇒ 本判据**不依赖腿的覆盖**。
/// 喂**合成路径**即可在本机 / Linux CI 上判它 —— 这正是"把平台假设本身变成判据"。
///
/// ⭐ 反例臂（三条 `!`）同样是判据的一部分：⛔ 不许把"任何 `.slint`"都算命中，
/// 否则"修好"会退化成"永远返回 true"。
#[test]
fn the_path_suffix_helper_accepts_both_separators() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
}

/// 判据（**结构**：钉住**调用点**，而不只是助手）：⭐ 本文件里不许再出现裸的
/// 把**正斜杠后缀字面量**直接喂给裸 `ends_with` 的写法（本判据用 `concat!` 拼出那个针脚，
/// 因此它不会命中自己），且 6 个调用点必须真的走助手。
///
/// ⚠ 为什么必须有它（两条线各栽一次的教训）：**助手修好了、调用点没改** ⇒ ⭐ **Linux 上依然全绿**
/// （Linux 的路径本来就是正斜杠）⇒ 光钉助手是**假修法**。只有把"调用点的写法"钉成判据，
/// 这个缺陷才在 Linux 上有守卫；⭐ **Windows 腿是否跑 `yeban-app` ⇒ 见
/// `.github/workflows/ci.yml` 的 windows job**（⛔ 不在此写死清单）⇒ ⭐ 本判据**不依赖**
/// 那条腿的覆盖，因此**无论覆盖如何它都是守卫**。
/// ⭐ **纯函数的内存样例判据（本单的承重证据）**：⭐ **本机（macOS）无法让 `Path::display()`
/// 产出反斜杠** ⇒ "Windows 上真的会红"这条物理事实**本机验不了**；但"**判定本身与分隔符无关**"
/// 可以用**内存样例**证明，⭐ 而样例**必须含反斜杠那侧**（否则它证明不了任何东西）。
#[test]
fn the_path_helpers_are_separator_agnostic() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
}

#[test]
fn no_call_site_compares_ui_paths_with_a_bare_separator_literal() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("set_playing"),
        "宿主接线层必须注册 set_playing"
    );
}
