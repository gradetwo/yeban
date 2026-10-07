//! `yeban-app` 的**命令行面** —— 解析 / 用法 / 输出 / 批处理执行。
//!
//! ## 为什么它单独成一个模块，而且**零 Slint 依赖**
//!
//! 命令行的可判据部分（参数解析、用法文本、版本、打开/保存/导出、输出格式、退出码）
//! **完全不需要**界面。把它们关在这一个模块里带来三件实测到的好事：
//!
//! 1. 本机可以 `rustc --edition 2024 --test -D warnings` **真跑**全部判据
//!    （`yeban-app` 含 Slint ⇒ `run-gates.sh crate yeban-app` 本机 SKIP，见
//!    `docs/ledger/app-cli-notes.md` §4）；
//! 2. `main.rs` 只剩"分发 + 开窗口"，事件循环那一侧没有一份会漂移的第二实现；
//! 3. 界面路径与批处理路径**共用同一份报告实现**（[`project_report`] / [`view_report`]），
//!    因此 `--open x --headless` 打印的数字与 GUI 里那份投影必然同源 ——
//!    这是 D28「注入实现只能有一份」在**输出**上的对应物。
//!
//! ## 数据流
//!
//! ```text
//! argv --(parse)--> Options --(load_project)--> Loaded{archive, source}
//!                                                   |
//!                          project_report ----------+---------- project_view --> ViewState
//!                                                                            |
//!                                                     view_report / ElementRegistry::dump_lines
//! ```
//!
//! ## 诚实的输出（不是"好看"的输出）
//!
//! 每一条报告行都对应一次**真的发生过的**动作或一次**真的读到的**数字：
//! `opened:` 是实际读入的字节数（不是 `metadata` 声明值），`saved:` 是实际落盘的字节数
//! 与用过的临时文件名，"没有 `--open` 就用演示工程"这件事用 `from=sample=…` **明写**出来。
//! 失败一律 `Err`（退出码见 [`CliError::exit_code`]），**绝不**"打开成空工程"。
//!
//! ## 本模块**不**打印
//!
//! [`run_batch`] 返回它"应该打印"的每一行；打印由 [`emit`] / [`finish`] 做
//! （`main.rs` 调用）。这样判据可以直接断言**输出内容**，而不必去抓自己的 stdout。
//!
//! ## 与 `main.rs` 的分工（边界）
//!
//! 本模块不构造 Slint 对象、不碰事件循环。GUI 路径（`host::build_main_window` +
//! `ComponentHandle::run`）住在 `main.rs`，它用本模块的 [`load_project`] /
//! [`project_report`] / [`project_view`] 拿到工程与视图。

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;

use yeban_model::container::ProjectArchive;
use yeban_model::project::YebanProjectV1;
use yeban_render::midi::MidiFormat;

use crate::bridge::{BridgeError, ViewState};
use crate::elements::ElementRegistry;
use crate::export_als::AlsExportError;
use crate::export_logic::LogicExportError;
use crate::export_midi::{MidiExportError, MidiExportReport, export_project_to_file};
use crate::input::{Action, InputContext, Modifiers, PhysicalKey, Tool, View};
use crate::open::{
    DOCUMENT_FORMAT, OpenError, OpenedProject, ProjectOpenOptions, open_project_document_file,
};
use crate::save::{SaveError, SaveReport, save_archive_file, write_file_atomically};

// ---------------------------------------------------------------------------
// 退出码：**契约**，不是实现细节（`--help` 里逐条写出来给用户看）
// ---------------------------------------------------------------------------

/// 成功（含 `--help` / `--version` / 无头自检完成）。
pub const EXIT_OK: u8 = 0;
/// 界面路径失败（无法创建窗口 / 事件循环异常 / 工程无法投影成界面）。
pub const EXIT_UI: u8 = 1;
/// 命令行用法错误（未知开关 / 缺取值 / 重复 / 未知工程样本）。
pub const EXIT_USAGE: u8 = 2;
/// `--open` 失败（读文件 / 超上限 / 容器拒绝）。
pub const EXIT_OPEN: u8 = 3;
/// `--save-as` 失败（I/O 或容器写出被拒）。
pub const EXIT_SAVE: u8 = 4;
/// `--export-elements` / `--export-midi` / `--export-als` 失败（I/O、编码被拒、
/// 工程无 MIDI 内容、PPQ 漂移）。
///
/// **不发明新码**（ADR-0001 D25 的口径）：导出失败复用同一档。
pub const EXIT_EXPORT: u8 = 5;

/// CI 的握手行：它出现 = 进程真的没构造窗口、没进阻塞事件循环。
///
/// 它必须**恰好**是这个字面值（`docs/ledger/ui-shell-notes.md` 与既有判据脚本只认它）。
pub const HEADLESS_HANDSHAKE: &str = "headless ok";

/// 无头路径的边界声明行（**不许**让 `headless ok` 看着像"UI 已验证"）。
const HEADLESS_BOUNDARY: &str = "headless: 未构造 MainWindow, 未初始化 Slint 后端, 未渲染任何像素 —— 控件树断言需要 crates/yeban-ui-test-port 的 testing backend";

/// `--headless-idle` 的握手行 —— 与 [`HEADLESS_HANDSHAKE`] **刻意不同**。
///
/// 为什么不复用 `headless ok`：那一行是**契约**，它出现 = "这个进程一个 Slint 对象都没构造"
/// （见它自己的文档）。`--headless-idle` **真的**构造了 `MainWindow` 并逐行光栅化过一帧，
/// 让它打同一行就是一句假话 —— 而 CI 脚本只认那个字面值，会把差别吃掉。
pub const HEADLESS_IDLE_HANDSHAKE: &str = "headless-idle ok";

/// `--headless-idle` 的边界声明行（说清它**做**了什么、**没做**什么）。
///
/// 做：装自研软件平台（`MinimalSoftwareWindow` + `SoftwareRenderer`）、构造真 `MainWindow`、
/// 逐行光栅化一帧当见证、空闲 N 秒。
/// 没做：**没有 OS 窗口**（因而也不需要显示器）、没进阻塞事件循环、没连声卡 / 没建引擎、
/// 没做运行时控件树遍历（那需要 `crates/yeban-ui-test-port` 的 testing backend）。
pub const HEADLESS_IDLE_BOUNDARY: &str = "headless-idle: 已构造真 MainWindow + 逐行光栅化一帧; 未创建 OS 窗口 \
(平台 = MinimalSoftwareWindow/SoftwareRenderer)、未进阻塞事件循环、未连声卡/未建引擎; \
运行时控件树遍历仍属 yeban-ui-test-port 的 testing backend";

/// `--idle-seconds` 的上限（秒）。
///
/// 它是**护栏**而不是能力上限：`crates/yeban-app/tests/cli_contract.rs` 的 `RUN_TIMEOUT`
/// 是 120 秒，这里取 60 秒 —— 于是"有人把秒数敲成 3600"会变成一条**用法错误**，
/// 而不是一次"CI 卡住几十分钟"。
pub const MAX_IDLE_SECONDS: u32 = 60;

/// `--enable-mcp-http` 的字面值（`[ROAD-M4-001]` 的**运行期**开关）。
///
/// 权威定义在 `yeban-mcp` 的 `transport::ENABLE_HTTP_FLAG`（形态 A 与形态 B 说的是
/// 同一个词）。这里保留一份的理由是**编译期**的：默认构建里 `yeban-mcp` 根本不在依赖图上
/// （`in-process-mcp` 默认关），而用法文本与错误信息在默认构建里也要打得出来。
/// 两边**逐字节相同**这件事由判据钉住 —— `crates/yeban-app/tests/in_process_mcp.rs`
/// 断言它等于 `yeban_mcp::transport::ENABLE_HTTP_FLAG`。
pub const MCP_HTTP_SWITCH: &str = "--enable-mcp-http";

/// `YEBAN_MCP_HTTP` 的字面值（运行期开关的**环境变量**形态；判定见 `src/mcp_mount.rs`）。
///
/// 与 [`MCP_HTTP_SWITCH`] 同一个理由：用法文本在默认构建里也要能提到它。
pub const MCP_HTTP_ENV: &str = "YEBAN_MCP_HTTP";

/// `--export-als` 的字面值（实验性 `.als` 导出的**唯一**用户出口，ADR-0001 **D47**）。
///
/// 与 [`MCP_HTTP_SWITCH`] 同一个理由：用法文本与"本次构建没编译它"的错误信息在
/// **默认构建**里也要打得出来，因此字面值住在这里，而不住在会被 `cfg` 掉的模块里。
pub const ALS_EXPORT_SWITCH: &str = "--export-als";

/// 让 `--export-als` 真的可执行的那个**非默认** feature 名。
///
/// 用法错误必须**点名**它（`ParseError::AlsNotCompiled`）：说"不支持导出"没用，
/// 用户需要知道重编译时要加哪个开关。它同时是 `scripts/guards/policy_check.py` 的
/// `FORBIDDEN_DEFAULT_FEATURES` 成员 —— 默认构建里它必须关着。
pub const ALS_EXPORT_FEATURE: &str = "experimental-als-export";

/// `--export-logic` 的字面值（实验性 Logic Pro `.logicx` bundle 导出的**唯一**用户出口）。
///
/// 同 [`ALS_EXPORT_SWITCH`] 的理由：默认构建的用法文本与"没编译它"的错误信息都要打得出来。
pub const LOGIC_EXPORT_SWITCH: &str = "--export-logic";

/// 让 `--export-logic` 真的可执行的那个**非默认** feature 名。
///
/// 同 [`ALS_EXPORT_FEATURE`] 的理由：用法错误必须**点名**它，而且它同时是
/// `scripts/guards/policy_check.py` 的 `FORBIDDEN_DEFAULT_FEATURES` 成员。
pub const LOGIC_EXPORT_FEATURE: &str = "experimental-logic-export";

/// `--theme` 的字面值（运行期调色板切换的**唯一**用户出口）。
pub const THEME_SWITCH: &str = "--theme";

/// `--print-theme` 的字面值（把"请求的主题 / 本二进制实际编译进来的风格"如实打出来）。
///
/// 与 `--print-shortcuts` 同族：一条**只读**的自述命令，让人与 CI 能核对
/// "我说要 material，这个二进制到底编的是不是 material"。
pub const PRINT_THEME_SWITCH: &str = "--print-theme";

/// `SLINT_STYLE` 的字面值 —— Slint **自己**的编译期风格选择入口（本仓库不改它的语义）。
///
/// 为什么要在用法文本里写它：Slint 1.18.1 的**风格**只能在编译期选，而 `--theme` 只能在
/// 运行期换调色板。两者是不同层次的东西，用户必须能从一个地方读到这个区别
/// （另一处是 `--print-theme` 的输出）。
pub const SLINT_STYLE_ENV: &str = "SLINT_STYLE";

/// 这个二进制**实际**编译进来的 Slint 风格名。
///
/// 取值由 `build.rs` 用 `cargo:rustc-env=YEBAN_SLINT_STYLE=…` 注入 —— 它必须与
/// `slint_build::CompilerConfiguration::with_style` 收到的是**同一个**字符串，否则
/// `--print-theme` 会开始说假话。
///
/// `option_env!` 而不是 `env!`：本文件同时被"零 Slint 探针"（`rustc --test` 直接编译
/// `cli.rs`，见 `docs/ledger/app-cli-notes.md` §4.1）使用，那时没有 build script 注入。
/// 缺省值取 `"fluent"` —— 它**就是** Slint 在没给风格时的默认
/// （`i-slint-compiler` 的 `typeloader.rs:957`），因此探针路径不会凭空发明一个风格。
#[must_use]
pub fn compiled_slint_style() -> &'static str {
    option_env!("YEBAN_SLINT_STYLE").unwrap_or("fluent")
}

// ---------------------------------------------------------------------------
// 用法与版本
// ---------------------------------------------------------------------------

/// 命令行用法（`--help` / `-h` 的原样输出；用法错误时也会把它打到 stderr）。
///
/// 纪律：**这里出现的每一条命令都真的能跑**。写进去之前必须在真二进制上跑过一遍
/// （见 `docs/ledger/app-cli-notes.md` §3 的 Quick Start 可引用清单）。
#[must_use]
pub fn usage_text() -> String {
    format!(
        "\
夜半 Yeban — Slint 桌面主程序 (由 YebanProjectV1 驱动)

用法:
  yeban-app [选项]

打开 / 保存 / 导出:
  --open <path>            打开一个 `.yeban` 工程容器并把它作为**当前工程**驱动界面。
                           `.yeban` 容器是**唯一**工程格式 (ADR-0001 D43):
                           非容器文件 (例如散落的 project.json / 随机字节 / 空文件)
                           被**明确拒绝**, 打不开就报错退出, 绝不退化成空工程。
  --save-as <path>         把当前工程**原子**落盘成 `.yeban` 容器 ([ARCH-SEC-004]:
                           同目录临时文件 → fsync → rename → 刷目录)。
                           没有 --open 时当前工程 = 内置演示工程, 输出里
                           `saved: ... from=sample=...` 会明说这一点。
  --dump-elements          把语义元素注册表打印到 stdout (每行一个元素, 稳定顺序) [UI-TEST-001]
  --export-elements <path> 把同一份元素注册表**原子**写到文件, 便于脚本 / AI 消费;
                           与 --dump-elements 同时给 = 既打印又落盘
  --export-midi <path>     把当前工程导出成**标准 MIDI 文件** (SMF 1): conductor 轨
                           (tick 0 的 tempo + 拍号) + 每条含 MIDI 的轨道一条 MTrk
                           (通道按导出顺序 0,1,2,…); **PPQ 与工程一致 (960)**;
                           字节由 yeban-render 的**唯一** SMF 编码器产出 (ADR-0001 D47),
                           与 --export-elements / --save-as 共用同一份**原子**落盘实现
  --export-als <path>      把当前工程导出成**实验性** Ableton Live Set (`.als`, 即
                           Gzip 压缩的 LiveSet XML) [ARCH-FMT-002]; 与 --export-midi
                           并列, 同属 ADR-0001 D47 的\"导出唯一出口 = app CLI\";
                           **映射损失表逐条打到 stdout** (`als-losses:` / `als-loss:` 行),
                           因此\"哪些构造没被映射\"对用户可见 (太长时礼貌截断并写明还剩几条);
                           产物是\"Ableton 风格\"而**不**声称能被 Live 11/12 打开
                           (仓库内无参考 .als); **只在** `--features {als_feature}` 的
                           构建里存在 —— 默认构建给这个开关 = 用法错误 (退出码 {usage})
  --export-logic <dir>      把当前工程导出成**实验性** Logic Pro 工程 bundle (`.logicx`,
                             一个**目录**, 不是文件) [ARCH-FMT-002]; 与 --export-als 并列,
                             同属 ADR-0001 D47 的\"导出唯一出口 = app CLI\"; 目录**会被创建**
                             (含父目录), 里面写 `Alternatives/<NNN>/ProjectData` +
                             `MetaData.plist` + `DisplayState.plist` 与
                             `Resources/ProjectInformation.plist`;
                             **映射损失表逐条打到 stdout** (`logic-losses:` / `logic-loss:` 行),
                             因此\"哪些构造没被映射\"对用户可见 (太长时礼貌截断并写明还剩几条);
                             **打开结论（实测，勿外推）**: 本机 **Logic Pro 12.2** 能打开**供体拼接**
                             产物 (负责人实测, 2026-10-06); 自研 (无供体) 写入器的产物被拒绝过两次
                             (账本第 403、405 轮), 结论不覆盖其它 Logic 版本与其它机器; **只在**
                             `--features {logic_feature}` 的构建里存在 —— 默认构建给这个开关
                             = 用法错误 (退出码 {usage})
  --print-shortcuts        打印快捷键策略表在本版本的判定结果 [UI-A11Y-001/002]
  --theme <{themes}>
                           选择界面主题; 重复给以最后一个为准 (默认 default = yeban 调色板)
                             default    水墨 + 颜料 (**2026-10-07 起是默认主题**): 背景/面板/线
                                        几乎零饱和, 颜色只留在内容上, 唯一强调色 (`accent`)
                                        整份界面只出现一次。取值**由负责人下发的具名调色板指定**
                                        (2026-10-07 HTML mock; 取代 2026-10-06 的采样推导),
                                        逐条映射见 ui/tokens.slint §6b。
                                        `yeban` 是它的**别名**, 解析到同一支调色板 (旧名继续可用)
                             brand      本仓品牌深色 —— **2026-10-07 之前的默认外观** (那时 CLI
                                        名是 `default`, 本次改名让给 yeban 调色板)。它是本仓
                                        自己的品牌色 (assets/brand/ 从母版 SVG 提取), 不读 Palette
                             inkmoor    负责人设计稿「墨泊 InkMoor · 枫桥夜泊」: 冷墨阶 +
                                        渔火唯一暖强调, 圆角偏柔。取值来自那份 mock 的
                                        `body.inkmoor` 变量块, 逐条映射见 ui/tokens.slint §6e
                             plume      负责人设计稿「孤烟 Plume · 使至塞上」: 与墨泊成对的
                                        暖沙阶 + 落日唯一暖强调, 几何偏锐。取值来自同一份 mock
                                        的 `body.plume` 变量块, 逐条映射见 ui/tokens.slint §6e。
                                        **本切片只换颜色**: 圆角 / 指针形态 / 录音光环 /
                                        长河一线 / 标题栏主题标签等几何·动画·结构差异
                                        **没有**实现 (见 ui/tokens.slint §6f)
                             material   Material Design 的 Slint `Palette` 角色
                             fluent     Fluent Design System 的 `Palette` 角色
                             cupertino  macOS 观感的 `Palette` 角色
                             native     平台原生风格别名 (macOS→cupertino / Windows→fluent /
                                        Android→material / Linux·BSD→有 Qt 则 qt 否则 fluent)
                           **边界 (实测, 别外推)**: 本仓界面 100% 自绘 (Rectangle x74,
                           Slint 内建控件 x0), 所以 Slint 内建风格本身改不动我们的像素;
                           `--theme` 改的是 ui/tokens.slint 的颜色令牌 —— 十三支品牌色在
                           default / brand / inkmoor / plume 之外的主题下改为读 `Palette.*`。而 Slint 1.18.1
                           **没有**运行时换风格的 API (风格只能编译期定, 见下面的
                           {slint_style_env}), 因此四个内建名字共享**本二进制编进来的那一个**
                           风格; `{print_theme}` 会把这件事如实打出来。
                           生效范围: 只有真的构造窗口的路径 (GUI / --headless-idle);
                           `--headless` 一个 Slint 对象都不构造, 因此接受本开关但不生效
  {print_theme}          打印主题报告: 请求的主题 / 生效的调色板来源 / 本二进制实际
                           编译进来的 Slint 风格 (由 build.rs 注入, 不是从命令行推出来的)
  --project-sample <default|filled|empty>
                           选择\"没有 --open 时\"用哪个工程 (默认 default);
                           empty = 真的 0 轨空工程 (规范 空工程空闲常驻内存 [BASELINE-002]
                           所指的那个对象); 重复给以最后一个为准
  --headless-idle          无窗口模式下**真的**构造 Slint 控件树: 自研软件平台
                           (MinimalSoftwareWindow + SoftwareRenderer, 不需要显示器),
                           逐行光栅化一帧当见证, 空闲 N 秒后退出 (N 由 --idle-seconds 给)。
                           这是 [BASELINE-002] 那句 空工程空闲常驻内存 要量的**对象**
                           (--headless 一个 Slint 对象都不构造); 与 --headless 同时给
                           = 只走本模式 (它是更强的形态), 握手行换成 {idle_handshake}。
  --idle-seconds <N>        --headless-idle 的空闲秒数 (整数 1..={max_idle});
                           只对 --headless-idle 有意义, 单独给 = 用法错误 (退出码 {usage});
                           重复给以最后一个为准
  --enable-mcp-http        把**领域 MCP** 的环回 HTTP JSON-RPC 控制面挂进**本进程**
                           (形态 A, [ROAD-M4-001]): 只绑 127.0.0.1:0 (端口由系统分配,
                           绑后回读断言是环回)、强制 256-bit Bearer 令牌鉴权;
                           默认**关**, 且只在 `--features in-process-mcp` 的构建里存在;
                           令牌落 ~/.yeban/session.token (0600), 只报路径不报令牌。
                           等价入口: 环境变量 {mcp_env}=1

运行形态:
  yeban-app                启动 GUI (需要显示器; 进入阻塞事件循环)
  yeban-app --open a.yeban 用打开的那个工程启动 GUI
  任一\"无窗口开关\"(--headless / --dump-elements / --export-elements /
  --export-midi / --export-als / --export-logic / --print-shortcuts / --save-as)
  都不构造窗口、不进事件循环, 并打印握手行 `{handshake}`。
  `--headless-idle` 是**另一档**: 它同样不创建 OS 窗口、不进阻塞事件循环, 但它**会**
  构造一个软件窗口 + 真控件树并光栅化一帧, 因此握手行是 `{idle_handshake}`
  (与 `{handshake}` 刻意不同 —— 后者的语义是\"一个 Slint 对象都没构造\")。
  `--help` / `--version` 是短路命令, 不打印握手行。

组合语义 (都是有意的, 不是碰巧):
  --headless 与 --open 同时给  不构造窗口, 但**真的**打开文件并打印它的读数
                               ⇒ 无显示器环境下的\"打开这个工程\"自检
  --headless 单独给            不打开任何文件, 用演示工程自检
                               (输出里 `project-source:` 会说明)
  --save-as 不给 --open        保存的是演示工程, 输出 `saved: ... from=sample=default` 明说
  --save-as 与 --headless      两者都是无窗口路径, 可以一起给 (保存不需要窗口)
  --export-elements 与 --export-midi 与 --export-als 与 --export-logic 与 --save-as 任意组合
                               顺序固定: **先**导出元素, **再**导出 MIDI, **再**导出 .als,
                               **再**导出 .logicx, **最后**保存工程; 任一导出失败 ⇒ 不写工程
                               (退出码 {export})
  --export-midi 不给 --open    导出的是演示工程, 输出 `exported-midi: ... from=sample=...` 明说
  --export-als 不给 --open    导出的是演示工程, 输出 `exported-als: ... from=sample=...` 明说
  --export-logic 不给 --open  导出的是演示工程, 输出 `exported-logic: ... from=sample=...` 明说
  --help / -h, --version / -V  短路: 出现即打印并退出 {ok}, 其余参数(含未知参数)不再检查
  --open / --save-as / --export-elements / --export-midi / --export-als / --export-logic
                               各只能给一次; 重复给 = 用法错误 (退出码 {usage})
  --headless-idle 与 --save-as / --export-elements / --export-midi /
  --export-als / --export-logic / --dump-elements / --print-shortcuts
                               不能组合 = 用法错误 (退出码 {usage}): 那会把写盘 / 导出
                               **静默**丢掉, 而本模式的输出契约只有一条 —— 建树 + 空闲 + 读数
  --headless-idle 与 --idle-seconds
                               必须成对; 缺一个 = 用法错误 (退出码 {usage}), 绝不默认空闲时长
  --enable-mcp-http 与任一\"无窗口开关\"
                               不能组合 = 用法错误 (退出码 {usage}): 控制面要挂在**正在跑的
                               GUI 进程**里 (形态 A), 批处理路径挂上去只会\"刚绑好就拆掉\"

环境变量:
  SLINT_BACKEND=headless   与 --headless 等价 (yeban 自研哨兵值; Slint 1.18.1 无此后端)
  {slint_style_env}=<style>       **编译期**选 Slint 内建风格 (Slint 自己的入口, 见 build.rs)。
                           这是本版本换**风格**的唯一办法 —— 运行时没有这个 API;
                           可用取值 = fluent | fluent-light | fluent-dark | material |
                           material-light | material-dark | cupertino | cupertino-light |
                           cupertino-dark | cosmic | cosmic-light | cosmic-dark | qt | native。
                           没设时 = fluent (Slint 自己的默认)。注意这是**风格**层的默认,
                            与 `--theme` 的默认 (`default` = yeban 调色板) 是两层, 互不代替。
                           非法的取值会让**构建**失败 (不由编译器给英文诊断), 因为一个
                           拼错的风格名静默回落到 fluent 正是本仓库最忌讳的假绿
  {mcp_env}=1              与 --enable-mcp-http 等价 (同样只在带 `in-process-mcp`
                           的构建里有效; 默认关)

退出码:
  {ok} 成功 (含 --help / --version / 无头自检完成)
  {ui} 界面路径失败 (无法创建窗口 / 事件循环异常 / 工程无法投影成界面)
  {usage} 命令行用法错误 (未知开关 / 缺取值 / 重复给只能给一次的开关 / 未知工程样本 /
      未知主题 ({theme_switch} 的取值不在 {themes} 里) /
      --idle-seconds 单独给或与 --headless-idle 组合不当 / 非法空闲秒数 / 不该组合的开关同给 /
      --enable-mcp-http 与无窗口开关同给或本次构建未编译 `in-process-mcp` /
      --export-als 在本次构建未编译 `{als_feature}` /
      --export-logic 在本次构建未编译 `{logic_feature}`)
  {open} --open 失败 (读文件失败 / 超过 4 GiB 上限 / 不是 `.yeban` 容器 /
     容器拒绝: 压缩法 / Zip-Slip / 解压炸弹 / 截断 / CRC 不匹配 / 缺件 / 非法 project.json …)
  {save} --save-as 失败 (临时文件 / 刷盘 / 原子重命名任一步失败, 或容器写出被拒)
  {export} --export-elements 失败 / --export-midi 失败 / --export-als 失败 / --export-logic 失败
      (I/O; 或工程里没有可导出的 MIDI 音符 / 拍号分母不是 2 的幂 /
      工程 PPQ 与编码器默认 PPQ 不一致 / 编码器拒绝越界的音高或力度 /
      .als 的映射或 Gzip 封装失败 / .logicx 的 bundle 目录建不出来)

示例 (全部已在真二进制上跑过):
  yeban-app --headless
  yeban-app --headless --project-sample empty
  yeban-app --open song.yeban --headless
  yeban-app --headless-idle --idle-seconds 2 --project-sample empty
  yeban-app --open song.yeban --save-as copy.yeban
  yeban-app --open song.yeban --dump-elements
  yeban-app --open song.yeban --export-elements elements.txt
  yeban-app --open song.yeban --export-midi song.mid
  yeban-app --open song.yeban --export-logic out/Song.logicx
  yeban-app --print-theme
  yeban-app --theme material
  yeban-app --theme default
  yeban-app --theme yeban
  yeban-app --theme brand
  {slint_style_env}=cupertino cargo build -p yeban-app
  yeban-app --version
",
        handshake = HEADLESS_HANDSHAKE,
        idle_handshake = HEADLESS_IDLE_HANDSHAKE,
        max_idle = MAX_IDLE_SECONDS,
        mcp_env = MCP_HTTP_ENV,
        als_feature = ALS_EXPORT_FEATURE,
        logic_feature = LOGIC_EXPORT_FEATURE,
        print_theme = PRINT_THEME_SWITCH,
        slint_style_env = SLINT_STYLE_ENV,
        theme_switch = THEME_SWITCH,
        // 合法取值集合**机械地**来自 `Theme::ALL`（唯一事实源，别名也在同一份推导里）：
        // 用法文本与 `ParseError::UnknownTheme` 的可用列表因此不可能各说各话。
        themes = Theme::accepted_names().join("|"),
        ok = EXIT_OK,
        ui = EXIT_UI,
        usage = EXIT_USAGE,
        open = EXIT_OPEN,
        save = EXIT_SAVE,
        export = EXIT_EXPORT,
    )
}

/// 版本行（`--version` / `-V` 的原样输出）。
///
/// 取值来自 `env!("CARGO_PKG_VERSION")` —— 由 Cargo 从 `Cargo.toml` 的
/// `version.workspace = true`（唯一事实源是根 `Cargo.toml` 的 `[workspace.package]`）
/// 注入，因此它**不可能**与清单漂移，也**不许**被写成字面量常量。
/// 判据 `version_matches_the_workspace_manifest` 会去读根清单对账。
///
/// 注：本机零 Slint 探针用 `rustc` 直接编译本文件，因此探针脚本显式导出
/// `CARGO_PKG_VERSION`（从根 `Cargo.toml` 读出）—— 生产构建由 Cargo 注入，见 notes §4。
#[must_use]
pub fn version_text() -> String {
    format!("yeban-app {}", env!("CARGO_PKG_VERSION"))
}

// ---------------------------------------------------------------------------
// 参数
// ---------------------------------------------------------------------------

/// `--project-sample` 可选的内置工程样本。
///
/// 三个样本走的是**同一条**投影 + 注入路径（`bridge::from_project` → `host::apply_view`），
/// 区别只在"哪个 `YebanProjectV1`"。这就是本工作线的验收形态：换工程 ⇒ 换像素，
/// 中间没有任何"演示数据分支"。
///
/// `empty` 存在的理由（`BASELINE-002`）：规范的判据句是"**空**工程空闲常驻内存 ≤ 35 MB"，
/// 而 `default` 是 6 轨演示夹具、`filled` 更重 —— 两者都**不是**规范所指的那个对象。
/// 没有 `empty` 时，`scripts/gates/measure_rss.py` 量不到规范的对象；有了它，
/// 集成者可以在**同一条命令**上分别打 `default` / `empty` 两个标签读数（见
/// `docs/ledger/baseline-memory-notes.md`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sample {
    /// 演示夹具（`bridge::demo_project()`，6 条普通轨 + 主总线）。
    #[default]
    Default,
    /// `yeban-model` 的规范级丰富样本（`samples::filled_project()`）。
    Filled,
    /// `yeban-model` 的规范级**空**工程样本（`samples::default_project()`）。
    ///
    /// 逐项事实（由判据 `empty_sample_is_a_zero_track_project_with_witness` 钉住）：
    /// `tracks` 为空、`master_bus_track_id` 为 nil、无片段 / 场景 / 段落 / 资产 / `history.dag`。
    /// 它在模型侧**合法且可读**（`YebanProjectV1::validate()` 接受 0 轨工程），因此
    /// app 侧不需要任何"空工程特例" —— 停用某个样本就等于没量到这个对象。
    Empty,
}

impl Sample {
    /// 报告用的稳定短名（`--project-sample` 的取值）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Filled => "filled",
            Self::Empty => "empty",
        }
    }

    /// 报告行里对"这是哪种内置工程"的措辞。
    ///
    /// `empty` **不**沿用 `default` 那句"内置演示工程"：它真的是 0 轨空工程，
    /// 输出里不能出现与事实相反的描述（本仓库的第一条纪律是不写假话）。
    /// `default` / `filled` 的措辞保持**逐字不变** —— 改它们要同步
    /// `tests/cli_contract.rs` 与其它工作线的账本，超出本线范围。
    #[must_use]
    pub const fn origin_label(self) -> &'static str {
        match self {
            Self::Empty => "内置空工程 0 轨",
            Self::Default | Self::Filled => "内置演示工程",
        }
    }

    /// 构造样本工程。
    #[must_use]
    pub fn project(self) -> YebanProjectV1 {
        match self {
            Self::Default => crate::bridge::demo_project(),
            Self::Filled => yeban_model::samples::filled_project(),
            Self::Empty => yeban_model::samples::default_project(),
        }
    }
}

/// `--theme` 可选的主题（运行期调色板选择）。
///
/// ## 它到底能改什么（实测，不是期望）
///
/// 本仓的界面 **100% 自绘**：13 个 `.slint` 里 `Rectangle` 出现 74 次，而 Slint 内建控件
/// （`StandardButton` / `LineEdit` / `ScrollView` / `ListView` / `ComboBox` / `Slider` /
/// `TabWidget` …）出现 **0 次**。所以"换一个 Slint 内建风格"本身**改不动我们的像素** ——
/// 它只改 Slint 自己的控件与 `Palette` 全局。
///
/// 能让我们的像素跟着走的，是 [`crate::host::apply_theme`] 把
/// `ui/tokens.slint` 的 `ThemeState.theme` 写成下面的值：`Yeban` 走**负责人下发的
/// yeban 字面量**（2026-10-07 起是**默认主题**，CLI 字面值 `default`，`yeban` 是它的
/// 别名），`Brand` 走**本仓品牌深色的那一串十六进制字面量**（CLI 字面值 `brand`；
/// 2026-10-07 之前它是默认主题、CLI 名字叫 `default`），`InkMoor` / `Plume` 各走
/// **负责人下发的另一串十六进制字面量**（`.slint` 侧写作 `brand` / `yeban` / `inkmoor` /
/// `plume`；Slint 的 Rust 生成器只把每段首字母大写，所以 `inkmoor` **生成出来**是
/// `YebanTheme::Inkmoor`，与本枚举的 **CLI 变体名** `Theme::InkMoor` 差一个大写 ——
/// 两个名字各属于一层，别混用），剩下四个走 Slint 设计系统的 `Palette.*` 角色。
///
/// 于是八支主题分成两族：**四支自绘**（`Brand` / `Yeban` / `InkMoor` / `Plume` —— 都是
/// 本枚举的变体名；它们在 `YebanTheme` 里对应 `Brand` / `Yeban` / `Inkmoor` / `Plume`，
/// [`Self::uses_design_system`] 为假、[`Self::requested_slint_style`] 为 `None`）与
/// **四个设计系统名字**（`Material` / `Fluent` / `Cupertino` / `Native`）。
/// **命令行取值比主题多一个**：`yeban` 是 `Yeban` 的别名（见 [`Self::aliases`]），
/// 因此 `--theme` 的可接受集合 = [`Self::accepted_names`] 的 9 个名字 / 8 支调色板。
///
/// ## 为什么四个内建名字在**同一个二进制**里长得一样
///
/// Slint 1.18.1 **没有运行时选风格的 API**（`slint::select_built_in_style` 在该版本的
/// slint / i-slint-core / i-slint-backend-selector / i-slint-compiler 全文检索里不存在），
/// 风格是**编译期**定死的（`build.rs` 的 `SLINT_STYLE` → `with_style`）。`Palette` 也只有
/// 编译进来的那一个。于是 `--theme material` 与 `--theme fluent` 在不重新编译时共享同一套
/// `Palette` 值 —— 这不是缺陷而是本版本的上限；[`theme_lines`] / [`PRINT_THEME_SWITCH`]
/// 把这件事**如实**打给用户，而不是假装四个主题各不相同。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// 本仓品牌深色（`assets/brand/`，从母版 SVG 提取）—— **2026-10-07 之前的默认外观**。
    ///
    /// 在 `.slint` 侧它的枚举名是 `YebanTheme.brand`（Slint 里 `default` 是保留字），
    /// 对用户的字面值是 `--theme brand`：负责人把默认主题改成 `yeban` 那一套之后，
    /// `default` 这个名字让给了 yeban 调色板；品牌调色板自身**一位未改**，仍然可选。
    Brand,
    /// 水墨 + 颜料（2026-10-06 负责人批准的第二个**自绘**调色板）——
    /// **2026-10-07 起是默认主题**（`#[default]`，CLI 字面值 `default`，`yeban` 为别名）。
    ///
    /// 在 `.slint` 侧它的枚举名是 `YebanTheme.yeban`；对用户的字面值是 `default`，
    /// 而 `yeban` 解析到**同一支**调色板（[`Self::aliases`]），旧脚本不用改。
    /// 它与 [`Self::Brand`] 一样**不经过** `Palette`：那是一串自己的十六进制字面量。
    /// **2026-10-07 起取值不再由我们推导**：负责人把一份具名的完整调色板（HTML mock）
    /// 作为权威设计输入交下来（墨阶六级 + 两级发丝线 + 三级文字 + 枫桥夜泊颜料槽），
    /// 逐条映射写在 `ui/tokens.slint` §6b，判据是
    /// `tests/theme_selection.rs` 的 `the_effective_palette_follows_the_selection` 与
    /// `the_theme_palette_literals_in_the_source_are_the_measured_ones`。
    /// 2026-10-06 那版「按 8% 饱和度从品牌色采样推导」的取值**全部作废**。
    ///
    /// 与 `Brand` 的**结构**差别（这才是它存在的理由）：
    /// 背景/面板/分隔线全部落在负责人给的墨阶上，颜色只留在内容上，而
    /// [`Self::palette_source`] 里那支 `accent` 在整份界面里**只出现一次**。
    #[default]
    Yeban,
    /// 墨泊 InkMoor（2026-10-07 负责人下发的**第三支自绘调色板**，第一支成对皮肤）。
    ///
    /// 在 `.slint` 侧它的枚举名是 `YebanTheme.inkmoor`，对用户的字面值也就是 `inkmoor`。
    /// 取值**不是**我们推导的：负责人设计稿（HTML mock）的 `body.inkmoor` CSS 变量块
    /// 与同一份稿子的 `export global InkMoor { … }` 是权威输入，逐条映射写在
    /// `ui/tokens.slint` §6e。它与 [`Self::Yeban`] **不经过** `Palette`，也与 `yeban`
    /// 一样是一条自己的十六进制字面量链。
    ///
    /// 与 [`Self::Yeban`] 的关系**如实登记**：两者都出自"枫桥夜泊"同一套意象，但数字
    /// **逐条不同**（例：渔火 `yeban #c6a47c` vs `inkmoor #c9a26b`），是同一套语言的
    /// 两版修订。**已裁决 `HD-53`：两支都留**（两版都能选，没有谁取代谁；`Theme::ALL`
    /// 因此是 8 支）。
    InkMoor,
    /// 孤烟 Plume（2026-10-07 负责人下发的**第四支自绘调色板**，与 [`Self::InkMoor`] 成对）。
    ///
    /// 在 `.slint` 侧它的枚举名是 `YebanTheme.plume`，对用户的字面值也就是 `plume`。
    /// 取值来自同一份设计稿的 `body.plume` 变量块与 `export global Plume { … }`，
    /// 逐条映射写在 `ui/tokens.slint` §6e。它与 [`Self::InkMoor`] 是同一系统的两种性格
    /// （冷墨/圆润 ↔ 暖沙/锐利），但**本切片只换颜色**：圆角、指针形态、录音光环、
    /// 长河一线、标题栏主题标签等几何/动画/结构差异**没有**实现（见 §6f）。
    Plume,
    /// Material Design（<https://m3.material.io>）对应的 `Palette` 角色。
    Material,
    /// Fluent Design System 对应的 `Palette` 角色。
    Fluent,
    /// macOS 观感（Cupertino）对应的 `Palette` 角色。
    Cupertino,
    /// 平台原生风格别名：macOS → `cupertino`，Windows → `fluent`，Android → `material`，
    /// Linux/BSD → 有 Qt 则 `qt` 否则 `fluent`（`i-slint-common` 的 `get_native_style`）。
    Native,
}

impl Theme {
    /// 全部主题（每支一个；别名不在这里，见 [`Self::aliases`]）。
    ///
    /// 顺序 = **默认主题在前**，其余按"自绘调色板 → 设计系统名字"排；用法文本与错误信息
    /// 共用**这一份**顺序，所以两处的枚举解释不会各说各话。
    pub const ALL: [Self; 8] = [
        Self::Yeban,
        Self::Brand,
        Self::InkMoor,
        Self::Plume,
        Self::Material,
        Self::Fluent,
        Self::Cupertino,
        Self::Native,
    ];

    /// 命令行**规范名**（`--theme <name>`；每支主题恰好一个）。
    ///
    /// `Yeban` 的规范名是 `default` —— 2026-10-07 负责人把默认主题改成 yeban 那一套之后，
    /// "默认"这个名字就归它；`yeban` 作为**别名**保留（见 [`Self::aliases`]），旧脚本不用改。
    ///
    /// `Brand` 的规范名是 `brand`：它内部就叫品牌色（`.slint` 侧是 `YebanTheme.brand`），
    /// 2026-10-07 之前对用户的字面值才是 `default`。
    ///
    /// `InkMoor` / `Plume` 的字面值**就是文档里的名字**（`inkmoor` / `plume`）：
    /// 负责人设计稿用 `ThemeKind.InkMoor` / `.Plume` 与 `theme-name` 的拼音/英文名，
    /// 命令行取小写的同一串字母，不做任何再命名。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Brand => "brand",
            Self::Yeban => "default",
            Self::InkMoor => "inkmoor",
            Self::Plume => "plume",
            Self::Material => "material",
            Self::Fluent => "fluent",
            Self::Cupertino => "cupertino",
            Self::Native => "native",
        }
    }

    /// 这一支主题的**别名**（解析到同一支调色板；规范名见 [`Self::name`]）。
    ///
    /// 为什么 `Yeban` 要留一个别名：2026-10-06 起 `--theme yeban` 就已经是公开取值，
    /// 负责人 2026-10-07 只是把**默认**换成它、并要求品牌色改名。删掉 `yeban` 会把所有
    /// 已经写着 `--theme yeban` 的脚本打断，而保留它的成本是零 —— 两支名字解析到同一支
    /// 调色板，别名不会产生第二套像素。**没有**任何理由不留，故留。
    ///
    /// 别再给别的主题加别名：`--print-theme` 的 `requested=` 打的是**规范名**，别名越多，
    /// "我请求的名字"与报告里那一行就越容易对不上。
    #[must_use]
    pub const fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Yeban => &["yeban"],
            _ => &[],
        }
    }

    /// **全部可接受的命令行取值**（规范名 + 别名，顺序 = [`Self::ALL`] 的顺序）。
    ///
    /// 这是用法文本、`ParseError::UnknownTheme` 的可用列表与判据**共用**的唯一推导：
    /// 三者都从 `Theme::ALL` 机械地长出来，所以"帮助里写了什么"与"解析器真的接受什么"
    /// 不可能漂移。别在别处再手写一遍主题名字列表。
    #[must_use]
    pub fn accepted_names() -> Vec<&'static str> {
        Self::ALL
            .into_iter()
            .flat_map(|theme| std::iter::once(theme.name()).chain(theme.aliases().iter().copied()))
            .collect()
    }

    /// 从命令行字面值解析（规范名或别名；未知取值返回 `None`，由调用方转成用法错误）。
    #[must_use]
    pub fn from_name(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|theme| theme.name() == value || theme.aliases().contains(&value))
    }

    /// 是否请求"走设计系统"（而不是自绘的十六进制字面量）。
    ///
    /// `false` 对 [`Self::Brand`] / [`Self::Yeban`] / [`Self::InkMoor`] / [`Self::Plume`]
    /// 成立 —— 这四支都自带一串十六进制字面量，都不读 `Palette`。它同时是判据里
    /// "哪些主题会被 `SLINT_STYLE` 影响"的入口（默认主题 `Yeban` 在其中，因此默认外观
    /// **不**随编译进来的风格变）。
    #[must_use]
    pub const fn uses_design_system(self) -> bool {
        !matches!(
            self,
            Self::Brand | Self::Yeban | Self::InkMoor | Self::Plume
        )
    }

    /// 这一支主题的颜色**从哪来**（`--print-theme` 如实打出来的那个词）。
    ///
    /// 三分法而不是布尔：`yeban` 既不是品牌字面量、也不是设计系统角色，用一个
    /// `bool` 表达它必然要么说假话、要么被并进"brand"里 —— 两种都是本仓库忌讳的
    /// 假绿。判据 `every_valid_theme_value_is_accepted_and_reported` 用**同一个**
    /// 方法算期望值，所以报告与实现不会各说各话。
    ///
    /// `yeban-measured-literals` 是一个**历史键名**（2026-10-06 那版取值确实是我们
    /// 自己测量推导的；2026-10-07 换成负责人下发的调色板后键名没动，以免改动 CLI
    /// 契约）。2026-10-07 新增的两支**不再沿用那个会误导的键名**：`inkmoor` / `plume`
    /// 的取值从第一天起就是负责人下发的，所以键名直接写 `owner-literals`。
    #[must_use]
    pub const fn palette_source(self) -> &'static str {
        match self {
            Self::Brand => "brand",
            Self::Yeban => "yeban-measured-literals",
            Self::InkMoor => "inkmoor-owner-literals",
            Self::Plume => "plume-owner-literals",
            _ => "design-system-palette",
        }
    }

    /// 这个主题请求的 Slint 内建风格名（四支自绘调色板不请求任何内建风格 ⇒ `None`）。
    #[must_use]
    pub const fn requested_slint_style(self) -> Option<&'static str> {
        match self {
            Self::Brand | Self::Yeban | Self::InkMoor | Self::Plume => None,
            Self::Material => Some("material"),
            Self::Fluent => Some("fluent"),
            Self::Cupertino => Some("cupertino"),
            Self::Native => Some("native"),
        }
    }

    /// 这一支自绘调色板的**设计出处**（`--print-theme` 的 `theme-source:` 行）。
    ///
    /// 四支自绘调色板各自有一份权威设计输入：`brand` 是本仓品牌色（`assets/brand/`），
    /// 另三支是负责人下发的 HTML mock。把出处打成一行，是为了让"这个 hex 从哪来"
    /// 在**命令输出**里可核对，而不是只活在我们的报告里。
    ///
    /// 走设计系统的四支没有这一行 ⇒ `None`（它们没有"负责人下发的具体色值"这回事）。
    #[must_use]
    pub const fn design_source(self) -> Option<&'static str> {
        match self {
            Self::Brand => Some("assets/brand/README.md (本仓品牌色, 从母版 SVG 提取)"),
            Self::Yeban => {
                Some("负责人 2026-10-07 下发的具名调色板 (HTML mock; 见 ui/tokens.slint §6b)")
            }
            Self::InkMoor => Some(
                "负责人设计稿「墨泊 InkMoor · 枫桥夜泊」(HTML mock 的 body.inkmoor 变量块; \
                 见 ui/tokens.slint §6e)",
            ),
            Self::Plume => Some(
                "负责人设计稿「孤烟 Plume · 使至塞上」(HTML mock 的 body.plume 变量块; \
                 见 ui/tokens.slint §6e)",
            ),
            _ => None,
        }
    }
}

/// 解析后的命令行选项。
///
/// 字段全公开是为了让判据能直接构造组合（例如只给 `save_as` 而不给 `open`），
/// 而不是靠解析字符串去间接构造。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    /// `--help` / `-h`：短路命令。
    pub help: bool,
    /// `--version` / `-V`：短路命令。
    pub version: bool,
    /// `--headless`（或 `SLINT_BACKEND=headless` 哨兵，由 `main.rs` 折算进来）。
    pub headless: bool,
    /// `--dump-elements`：元素注册表打到 stdout。
    pub dump_elements: bool,
    /// `--print-shortcuts`：快捷键策略表打到 stdout。
    pub print_shortcuts: bool,
    /// `--export-elements <path>`：元素注册表原子写到文件。
    pub export_elements: Option<PathBuf>,
    /// `--export-midi <path>`：当前工程导出成标准 MIDI 文件（SMF 1）[ADR-0001 **D47**]。
    pub export_midi: Option<PathBuf>,
    /// `--export-als <path>`：当前工程导出成实验性 Ableton `.als`（Gzip XML）
    /// `[ARCH-FMT-002]` / `[ROAD-M4-007]`，与 `--export-midi` 并列同属 `D47` 的 CLI 出口。
    ///
    /// **只在** [`ALS_EXPORT_FEATURE`] 打开的构建里可执行；默认构建给它 =
    /// [`ParseError::AlsNotCompiled`]（退出码 [`EXIT_USAGE`]）—— 与
    /// [`Self::enable_mcp_http`] 同款，绝不静默忽略。它**不需要**界面投影
    /// （只依赖 `YebanProjectV1`，与 `--export-midi` / `--save-as` 同族）。
    pub export_als: Option<PathBuf>,
    /// `--export-logic <dir>`：当前工程导出成实验性 Logic Pro 工程 bundle（`.logicx`，一个**目录**）
    /// `[ARCH-FMT-002]` / `[ROAD-M4-007]`，与 `--export-als` 并列同属 `D47` 的 CLI 出口。
    ///
    /// **只在** [`LOGIC_EXPORT_FEATURE`] 打开的构建里可执行；默认构建给它 =
    /// [`ParseError::LogicNotCompiled`]（退出码 [`EXIT_USAGE`]）—— 与 [`Self::export_als`] 同款。
    /// 取值是目录而不是文件：bundle 会被**创建**（含父目录）。它**不需要**界面投影。
    pub export_logic: Option<PathBuf>,
    /// `--open <path>`：当前工程来自这个文件（否则来自 [`Self::sample`]）。
    pub open: Option<PathBuf>,
    /// `--save-as <path>`：把当前工程原子落盘到这里。
    pub save_as: Option<PathBuf>,
    /// `--project-sample <default|filled|empty>`。
    pub sample: Sample,
    /// `--headless-idle`：无窗口模式下**真的构造 Slint 控件树**并空闲 N 秒。
    ///
    /// 与 [`Self::headless`] 的关系：两者都是无窗口路径，但 `--headless` 走
    /// [`run_batch`]（**零 Slint 对象**），而本开关走 `crate::headless_idle::run`
    /// （真 `MainWindow` + 自研软件平台 + 逐行光栅化一帧）。同时给两者 ⇒ 只走后者
    /// （它是**更强**的形态：报告行完全一样，只是握手行换成
    /// [`HEADLESS_IDLE_HANDSHAKE`]）。
    pub headless_idle: bool,
    /// `--idle-seconds <N>`：`--headless-idle` 的空闲秒数（整数 `1..=`[`MAX_IDLE_SECONDS`]）。
    ///
    /// 与 `--headless-idle` **必须成对**：只给一个 = 用法错误。这条不是形式主义 ——
    /// 少了它就无法区分"空闲 0 秒"与"参数没生效"，而 `BASELINE-002` 的读数正建立在
    /// "空闲了多少秒"这件事上（判据 ③ 要证空闲期间读数不再攀升）。
    pub idle_seconds: Option<u32>,
    /// `--enable-mcp-http`：把**领域 MCP 的环回 HTTP 控制面**挂进本进程（`[ROAD-M4-001]`）。
    ///
    /// 这是**运行期**那道开关（编译期那道是 `--features in-process-mcp`，见
    /// `crates/yeban-app/Cargo.toml` 与 `src/mcp_mount.rs`）。默认 `false`；
    /// 环境变量 `YEBAN_MCP_HTTP=1` 是等价入口（判定在 `crate::mcp_mount::switch_requested`，
    /// 但那个模块只在 feature 打开时存在，因此这里只存命令行那一路的事实）。
    ///
    /// 为什么它**不**让进程离开 GUI 路径：控制面要挂在**正在跑的 app 进程**里
    /// （形态 A 的定义），而不是把进程变成一个无头服务器。
    pub enable_mcp_http: bool,
    /// `--theme <default|yeban|brand|inkmoor|plume|material|fluent|cupertino|native>`：
    /// 运行期调色板选择（`yeban` 是 `default` 的别名，见 [`Theme::accepted_names`]）。
    ///
    /// 默认 [`Theme::Yeban`]（= CLI `default`）= **负责人下发的 yeban 调色板**，
    /// 也就是 2026-10-07 起的外观。默认外观**因此与之前不同** ⇒ 5 张 Linux 基准要按
    /// 手动档 `gates-manual.yml gate=goldens` 重录 + 人工复核（`HD-56` 把"基准冻结"
    /// 降为条件；`--theme brand` 才是原来那一屏）。
    ///
    /// 生效范围（如实登记）：**只有真的构造窗口的路径**才会调用
    /// [`crate::host::apply_theme`] —— 也就是 GUI 与 `--headless-idle` 两档。
    /// `--headless` 是"一个 Slint 对象都不构造"的路径（见 `main.rs` 的模块文档），
    /// 那里**没有**可写的 `ThemeState`，所以它接受这个开关但不生效。
    pub theme: Theme,
    /// `--print-theme`：把请求的主题与"本二进制实际编译进来的 Slint 风格"打到 stdout。
    ///
    /// 为什么它必须存在（而不是只写在我的报告里）：`--theme material` 在一个用
    /// `SLINT_STYLE=fluent` 编译出来的二进制里**拿不到** Material 的调色板 —— 因为
    /// Slint 1.18.1 没有运行时选风格 API。用户需要一个**命令**能读到这件事，
    /// 否则"我选了 material"与"我看到的是 fluent"之间就没有任何可核对的地方。
    pub print_theme: bool,
}

impl Options {
    /// 是否是**无窗口**路径（不创建 OS 窗口、不进事件循环）。
    ///
    /// 语义表（`--help` 的"组合语义"一节与判据都按这张表）：
    /// `--help` / `--version` / `--headless` / `--dump-elements` / `--print-shortcuts` /
    /// `--export-elements` / `--export-midi` / `--export-als` / `--save-as`
    /// 各自都能单独把进程推离 GUI 路径。
    ///
    /// `--headless-idle` **也**在这一档里（它确实不建 OS 窗口），但它是**唯一**
    /// 会构造 Slint 对象的无窗口开关 ⇒ `main.rs` 必须在 `wants_gui()` 之前先看它，
    /// 而 [`run_batch`] 见到它会明确报错而不是静默降级（见那里的守卫）。
    #[must_use]
    pub fn batch(&self) -> bool {
        self.help
            || self.version
            || self.headless
            || self.dump_elements
            || self.print_shortcuts
            || self.print_theme
            || self.export_elements.is_some()
            || self.export_midi.is_some()
            || self.export_als.is_some()
            || self.export_logic.is_some()
            || self.save_as.is_some()
            || self.headless_idle
    }

    /// 是否要真的开窗口。
    ///
    /// 取反而不是另列一张表：两张表迟早会漂移（"哪些开关算无头"必须只有一个事实源）。
    #[must_use]
    pub fn wants_gui(&self) -> bool {
        !self.batch()
    }

    /// 是否需要把工程**投影成界面状态**。
    ///
    /// `--save-as` **不**需要投影：保存只依赖 `YebanProjectV1`。
    /// 于是"工程能存下来、但投影成界面会失败"这种情况会**如实保存并只在需要投影的
    /// 命令上失败**，而不是让保存被一个与它无关的理由（画不出来）挡住。
    ///
    /// `--headless-idle` 需要投影：它的见证行里带 `view-counts` 的 `elements=`
    /// （复用既有的无头内省），而且它**必须**真的建树 —— 一个不投影的"空闲模式"
    /// 只会量到一个空进程。
    #[must_use]
    pub fn needs_projection(&self) -> bool {
        self.headless
            || self.headless_idle
            || self.dump_elements
            || self.print_shortcuts
            || self.export_elements.is_some()
    }
}

/// 参数解析失败（一律对应退出码 [`EXIT_USAGE`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// 不认识这个参数（**绝不静默忽略**）。
    UnknownArgument(String),
    /// 这个开关需要一个取值，但没给（或下一个 token 本身是另一个开关）。
    MissingValue(&'static str),
    /// 这个开关不接受取值（`--headless=1`）。
    UnexpectedValue(&'static str),
    /// 只能给一次的开关给了两次。
    DuplicateOption(&'static str),
    /// `--project-sample` 的取值不在允许集合里。
    UnknownSample(String),
    /// `--theme` 的取值不在允许集合里。
    ///
    /// 与 [`Self::UnknownSample`] 同款：**绝不静默回退到默认主题** —— 那会让
    /// "我选了 material"与"其实渲染的是 `default`(yeban 调色板)"长得一模一样，而本仓库最忌讳
    /// 的就是这一类"以为生效了"的假绿。
    UnknownTheme(String),
    /// `--headless-idle` 没配 `--idle-seconds`。
    ///
    /// 为什么是错误而不是"默认空闲 1 秒"：默认值会让"参数没生效"与"空闲 0 秒"长得一样，
    /// 而 `BASELINE-002` 的判据 ③（空闲期间读数不攀升）正是靠"空闲了多久"来定阈值的。
    IdleSecondsMissing,
    /// `--idle-seconds` 给了，但没有 `--headless-idle`（它只对这一档有意义）。
    IdleSecondsWithoutHeadlessIdle,
    /// `--idle-seconds` 的取值不是 `1..=`[`MAX_IDLE_SECONDS`] 的整数。
    IdleSecondsInvalid(String),
    /// `--headless-idle` 与一个会写盘 / 导出的开关同时给。
    ///
    /// 语义上不是"不能做"，而是**不能静默丢掉**：本模式的输出契约只有一条
    /// （建树 + 空闲 + 读数），如果接受这些组合，`--save-as` / `--export-*` 就会被
    /// 悄悄忽略 —— 那正是本仓库最忌讳的一类假绿。要保存就先跑一次不带本开关的命令。
    IdleConflict(&'static str),
    /// 给了 `--enable-mcp-http`，但这次构建**没有**编译 `in-process-mcp`
    /// （`[ROAD-M4-001]` / `[MUST-GATE-009]` 的第一道开关）。
    ///
    /// 不静默忽略是本仓库的通则：用户要开一个网络控制面，而二进制里根本没有那段代码 ——
    /// 那必须是一次**响亮的**用法错误，而不是"以为开了其实没开"。
    McpHttpNotCompiled,
    /// `--enable-mcp-http` 与一个**无窗口**开关同时给。
    ///
    /// 控制面要挂在**正在跑的 app 进程**里（形态 A 的定义）；批处理路径跑完就退出，
    /// 挂上去等于刚一绑好就拆掉。接受这个组合只会让人以为"服务起来了"。
    McpHttpNeedsGui(&'static str),
    /// 给了 `--export-als`，但这次构建**没有**编译 [`ALS_EXPORT_FEATURE`]
    /// （`[ARCH-FMT-002]` / `[ROAD-M4-007]`）。
    ///
    /// 与 [`Self::McpHttpNotCompiled`] 同一条纪律：默认构建里那段导出代码与 `flate2`
    /// 都**不在依赖图上**，用户要的是一个 `.als` 文件而二进制里根本没有那个出口 ——
    /// 那必须是一次**点名 feature 的**用法错误，而不是"以为写了其实没写"。
    AlsNotCompiled,
    /// 给了 `--export-logic`，但这次构建**没有**编译 [`LOGIC_EXPORT_FEATURE`]
    /// （`[ARCH-FMT-002]` / `[ROAD-M4-007]`）。
    ///
    /// 与 [`Self::AlsNotCompiled`] 同一条纪律：默认构建里 `yeban-render` 的 `logic` 模块
    /// 不存在，用户要的是一个 `.logicx` bundle 而二进制里根本没有那个出口 ——
    /// 那必须是一次**点名 feature 的**用法错误。
    LogicNotCompiled,
}

impl ParseError {
    /// 用法错误一律 [`EXIT_USAGE`]（把"哪一档退出码"写进类型，而不是散在 `main.rs` 里）。
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        EXIT_USAGE
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownArgument(argument) => write!(formatter, "无法识别的参数 `{argument}`"),
            Self::MissingValue(option) => write!(formatter, "`{option}` 需要一个取值"),
            Self::UnexpectedValue(option) => write!(formatter, "`{option}` 不接受取值"),
            Self::DuplicateOption(option) => {
                write!(formatter, "`{option}` 只能给一次 (给两次 = 目标不明确)")
            }
            Self::UnknownSample(sample) => write!(
                formatter,
                "未知的工程样本 `{sample}` (可用: default|demo|filled|empty)"
            ),
            Self::UnknownTheme(theme) => write!(
                formatter,
                "未知的主题 `{theme}` (可用: {}; 默认 `default` = yeban 调色板 \
                 (`yeban` 是它的别名, 同一个取值); `brand` = 本仓品牌深色, \
                 也就是 2026-10-07 之前的默认外观; `inkmoor` / `plume` 也各自带一串\
                 负责人下发的十六进制字面量, 不读 Palette; 只有剩下那四个内建名字走 Slint \
                 设计系统的 Palette 角色, 而 Slint 1.18.1 的风格只能**编译期**选 —— \
                 见 `{PRINT_THEME_SWITCH}` 与 `{SLINT_STYLE_ENV}`)",
                Theme::accepted_names().join("|")
            ),
            Self::IdleSecondsMissing => write!(
                formatter,
                "`--headless-idle` 必须配 `--idle-seconds <N>` (不设默认: 否则 \
                 \"参数没生效\" 与 \"空闲 0 秒\" 无法区分)"
            ),
            Self::IdleSecondsWithoutHeadlessIdle => write!(
                formatter,
                "`--idle-seconds` 只对 `--headless-idle` 有意义 (本仓库不静默忽略参数)"
            ),
            Self::IdleSecondsInvalid(value) => write!(
                formatter,
                "`--idle-seconds` 的取值 `{value}` 非法 (需要 1..={MAX_IDLE_SECONDS} 的整数; \
                 上限是护栏: tests/cli_contract.rs 的子进程超时是 120 秒)"
            ),
            Self::IdleConflict(other) => write!(
                formatter,
                "`--headless-idle` 不能与 `{other}` 组合 —— 本模式的输出契约只有 \
                 \"建树 + 空闲 + 读数\", 接受它会把 {other} 静默丢掉"
            ),
            Self::McpHttpNotCompiled => write!(
                formatter,
                "`{MCP_HTTP_SWITCH}` 需要本次构建带 `--features in-process-mcp` \
                 (它就是 [MUST-GATE-009] 的第一道开关: 默认构建里没有网络控制面这段代码)"
            ),
            Self::McpHttpNeedsGui(other) => write!(
                formatter,
                "`{MCP_HTTP_SWITCH}` 不能与无窗口开关 `{other}` 组合 —— 控制面要挂在正在跑的 \
                 GUI 进程里 (形态 A), 批处理路径挂上去只会刚绑好就拆掉"
            ),
            Self::AlsNotCompiled => write!(
                formatter,
                "`{ALS_EXPORT_SWITCH}` 需要本次构建带 `--features {ALS_EXPORT_FEATURE}` \
                 (实验性 .als 导出默认关: AGENTS.md §2 红线 6 与 [MUST-GATE-009] 同一条纪律; \
                 它就是 [ARCH-FMT-002] / [ROAD-M4-007] 那条导出的唯一用户出口)"
            ),
            Self::LogicNotCompiled => write!(
                formatter,
                "`{LOGIC_EXPORT_SWITCH}` 需要本次构建带 `--features {LOGIC_EXPORT_FEATURE}` \
                 (实验性 .logicx 导出默认关: AGENTS.md §2 红线 6 与 [MUST-GATE-009] 同一条纪律; \
                 它就是 [ARCH-FMT-002] / [ROAD-M4-007] 那条导出的唯一用户出口)"
            ),
        }
    }
}

impl std::error::Error for ParseError {}

/// 解析 argv（**不含** `argv[0]`）。
///
/// 规则（每条都有判据）：
/// - `--help` / `-h` 与 `--version` / `-V` **短路**：只要出现就立刻返回，其余参数
///   （含未知参数）不再检查 —— 与 GNU 惯例一致，也是"帮助永远打得开"的前提；
/// - 带取值的开关同时支持 `--opt value` 与 `--opt=value` 两种写法；
///   `--opt --other` 会被判为**缺取值**（而不是把 `--other` 当成路径 —— 那是会静默
///   把工程写到奇怪地方的一类 bug）；
/// - 位置参数（不以 `-` 开头）与未知开关一律 [`ParseError::UnknownArgument`]；
/// - `--open` / `--save-as` / `--export-elements` 各只能给一次（重复 = 目标不明确）；
/// - `--project-sample` 重复给以最后一个为准（沿用旧行为）；
/// - `--headless-idle` 与 `--idle-seconds` 必须**成对**，且不得与会写盘 / 导出的开关
///   同给（那会把后者静默丢掉，见 [`ParseError::IdleConflict`]）。
///
/// # Errors
///
/// 见 [`ParseError`]（退出码一律 [`EXIT_USAGE`]）。
pub fn parse(args: &[String]) -> Result<Options, ParseError> {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        return Ok(Options {
            help: true,
            ..Options::default()
        });
    }
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "--version" | "-V"))
    {
        return Ok(Options {
            version: true,
            ..Options::default()
        });
    }

    let mut options = Options::default();
    let mut cursor = 0;
    while cursor < args.len() {
        let raw = args[cursor].as_str();
        let (name, inline) = match raw.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (raw, None),
        };
        match name {
            "--headless" => {
                reject_inline("--headless", inline)?;
                options.headless = true;
                cursor += 1;
            }
            "--dump-elements" => {
                reject_inline("--dump-elements", inline)?;
                options.dump_elements = true;
                cursor += 1;
            }
            "--print-shortcuts" => {
                reject_inline("--print-shortcuts", inline)?;
                options.print_shortcuts = true;
                cursor += 1;
            }
            // `--print-theme`: 只读自述命令 (与 `--print-shortcuts` 同族)。
            // 它是**无窗口**路径: 不构造控件树, 只把"请求的主题 vs 编进来的风格"打出来。
            PRINT_THEME_SWITCH => {
                reject_inline(PRINT_THEME_SWITCH, inline)?;
                options.print_theme = true;
                cursor += 1;
            }
            "--open" => {
                let value = take_value("--open", inline, args, &mut cursor)?;
                set_once(&mut options.open, "--open", PathBuf::from(value))?;
            }
            "--save-as" => {
                let value = take_value("--save-as", inline, args, &mut cursor)?;
                set_once(&mut options.save_as, "--save-as", PathBuf::from(value))?;
            }
            "--export-elements" => {
                let value = take_value("--export-elements", inline, args, &mut cursor)?;
                set_once(
                    &mut options.export_elements,
                    "--export-elements",
                    PathBuf::from(value),
                )?;
            }
            "--export-midi" => {
                let value = take_value("--export-midi", inline, args, &mut cursor)?;
                set_once(
                    &mut options.export_midi,
                    "--export-midi",
                    PathBuf::from(value),
                )?;
            }
            "--export-als" => {
                let value = take_value("--export-als", inline, args, &mut cursor)?;
                set_once(
                    &mut options.export_als,
                    ALS_EXPORT_SWITCH,
                    PathBuf::from(value),
                )?;
            }
            "--export-logic" => {
                let value = take_value(LOGIC_EXPORT_SWITCH, inline, args, &mut cursor)?;
                set_once(
                    &mut options.export_logic,
                    LOGIC_EXPORT_SWITCH,
                    PathBuf::from(value),
                )?;
            }
            "--headless-idle" => {
                reject_inline("--headless-idle", inline)?;
                options.headless_idle = true;
                cursor += 1;
            }
            "--idle-seconds" => {
                let value = take_value("--idle-seconds", inline, args, &mut cursor)?;
                // 重复给以最后一个为准（与 `--project-sample` 同款）。
                options.idle_seconds = Some(parse_idle_seconds(&value)?);
            }
            "--enable-mcp-http" => {
                reject_inline("--enable-mcp-http", inline)?;
                options.enable_mcp_http = true;
                cursor += 1;
            }
            "--project-sample" => {
                let value = take_value("--project-sample", inline, args, &mut cursor)?;
                options.sample = match value.as_str() {
                    "default" | "demo" => Sample::Default,
                    "filled" => Sample::Filled,
                    "empty" => Sample::Empty,
                    other => return Err(ParseError::UnknownSample(other.to_owned())),
                };
            }
            // `--theme`: 重复给以最后一个为准（与 `--project-sample` 同款）。
            // 未知取值 ⇒ **点名**用法错误, 绝不静默回退（那会把"选了 material"变成
            // "其实渲染的是 `default` 的 yeban 调色板"而用户看不出来）。
            // 别名也在这里解析（`Theme::from_name` 同时认规范名与 [`Theme::aliases`]），
            // 因此 `--theme yeban` 与 `--theme default` 落在**同一个**枚举值上。
            THEME_SWITCH => {
                let value = take_value(THEME_SWITCH, inline, args, &mut cursor)?;
                options.theme = Theme::from_name(&value)
                    .ok_or_else(|| ParseError::UnknownTheme(value.clone()))?;
            }
            other => return Err(ParseError::UnknownArgument(other.to_owned())),
        }
    }
    // 成对性与组合性在这里**一次**判完（解析循环里判会让"后给的开关覆盖前者"这类
    // 顺序问题变成第二个事实源）。
    match (options.headless_idle, options.idle_seconds) {
        (true, None) => return Err(ParseError::IdleSecondsMissing),
        (false, Some(_)) => return Err(ParseError::IdleSecondsWithoutHeadlessIdle),
        _ => {}
    }
    if options.headless_idle {
        for (given, name) in [
            (options.dump_elements, "--dump-elements"),
            (options.print_shortcuts, "--print-shortcuts"),
            (options.export_elements.is_some(), "--export-elements"),
            (options.export_midi.is_some(), "--export-midi"),
            (options.export_als.is_some(), ALS_EXPORT_SWITCH),
            (options.export_logic.is_some(), LOGIC_EXPORT_SWITCH),
            (options.save_as.is_some(), "--save-as"),
        ] {
            if given {
                return Err(ParseError::IdleConflict(name));
            }
        }
    }
    // `--enable-mcp-http` 的两条前置（`[ROAD-M4-001]`）：编译期那道开关，以及"只能在
    // GUI 路径上开"。两条都是**用法错误**而不是静默忽略 —— 前者让人以为网络控制面开了，
    // 后者让人以为服务在跑（批处理路径跑完就退出）。
    if options.enable_mcp_http {
        if !cfg!(feature = "in-process-mcp") {
            return Err(ParseError::McpHttpNotCompiled);
        }
        for (given, name) in [
            (options.headless, "--headless"),
            (options.dump_elements, "--dump-elements"),
            (options.print_shortcuts, "--print-shortcuts"),
            (options.export_elements.is_some(), "--export-elements"),
            (options.export_midi.is_some(), "--export-midi"),
            (options.export_als.is_some(), ALS_EXPORT_SWITCH),
            (options.export_logic.is_some(), LOGIC_EXPORT_SWITCH),
            (options.save_as.is_some(), "--save-as"),
            (options.headless_idle, "--headless-idle"),
        ] {
            if given {
                return Err(ParseError::McpHttpNeedsGui(name));
            }
        }
    }
    // `--export-als` 的**编译期**那道开关（`[ARCH-FMT-002]` / `[ROAD-M4-007]`）：
    // 默认构建里 `yeban-render` 的 `als` 模块与可选的 `flate2` 都不在依赖图上，
    // 因此这个开关必须是一次**点名 feature** 的用法错误 —— 与 `--enable-mcp-http`
    // 同款，绝不静默忽略（静默忽略只会让人以为 `.als` 写了）。
    if options.export_als.is_some() && !cfg!(feature = "experimental-als-export") {
        return Err(ParseError::AlsNotCompiled);
    }
    // `--export-logic` 的**编译期**那道开关（同族）：默认构建里 `yeban-render` 的 `logic`
    // 模块不存在，因此同样必须是一次**点名 feature** 的用法错误。
    if options.export_logic.is_some() && !cfg!(feature = "experimental-logic-export") {
        return Err(ParseError::LogicNotCompiled);
    }
    Ok(options)
}

/// `--idle-seconds` 的取值：`1..=`[`MAX_IDLE_SECONDS`] 的**整数**秒。
///
/// 明确拒绝小数（`1.5`）：空闲时长的判据是"两次读数是否相等"，秒级整数足够，
/// 而多一种数值形态就多一处"看起来生效其实被截断"的余地。
fn parse_idle_seconds(value: &str) -> Result<u32, ParseError> {
    let seconds: u32 = value
        .parse()
        .map_err(|_| ParseError::IdleSecondsInvalid(value.to_owned()))?;
    if seconds == 0 || seconds > MAX_IDLE_SECONDS {
        return Err(ParseError::IdleSecondsInvalid(value.to_owned()));
    }
    Ok(seconds)
}

/// 不接受取值的开关若写成 `--flag=value` 就是用法错误。
fn reject_inline(name: &'static str, inline: Option<String>) -> Result<(), ParseError> {
    match inline {
        Some(_) => Err(ParseError::UnexpectedValue(name)),
        None => Ok(()),
    }
}

/// 取一个带取值开关的值（`--opt value` / `--opt=value`），并推进游标。
fn take_value(
    name: &'static str,
    inline: Option<String>,
    args: &[String],
    cursor: &mut usize,
) -> Result<String, ParseError> {
    if let Some(value) = inline {
        if value.is_empty() {
            return Err(ParseError::MissingValue(name));
        }
        *cursor += 1;
        return Ok(value);
    }
    let Some(value) = args.get(*cursor + 1) else {
        return Err(ParseError::MissingValue(name));
    };
    if value.starts_with("--") {
        // 下一步会把它当成另一个开关处理 —— 如果这里接受它, `--open --headless`
        // 就会把 `--headless` 当成路径, 从而**静默**丢掉无头语义。
        return Err(ParseError::MissingValue(name));
    }
    *cursor += 2;
    Ok(value.clone())
}

/// 只能给一次的开关。
fn set_once(
    slot: &mut Option<PathBuf>,
    name: &'static str,
    value: PathBuf,
) -> Result<(), ParseError> {
    if slot.is_some() {
        return Err(ParseError::DuplicateOption(name));
    }
    *slot = Some(value);
    Ok(())
}

// ---------------------------------------------------------------------------
// 当前工程
// ---------------------------------------------------------------------------

/// 当前工程**从哪里来**（报告里必须能说清，见模块文档）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectSource {
    /// 来自磁盘上的一个文档。
    File {
        /// 调用方给的路径（原样回显）。
        path: PathBuf,
        /// 实际读入的字节数。
        bytes: u64,
    },
    /// 来自内置样本（**没有**读任何文件）。
    Sample(Sample),
}

impl ProjectSource {
    /// 第一行报告：文件形态给 `opened: …`，样本形态给 `project-source: …`。
    ///
    /// `format=` 只有一个取值（[`DOCUMENT_FORMAT`]）：容器是唯一工程格式，报告里
    /// 不再有"我按哪种格式读的"这种歧义。
    ///
    /// 样本那一行**必须**明写"未读任何文件"：`--save-as` 不带 `--open` 时，
    /// 用户最容易误以为"它保存的是某个默认工程文件"。
    ///
    /// "哪种内置工程"的措辞由 [`Sample::origin_label`] 给（`empty` = 0 轨空工程，
    /// 不是演示工程）。
    #[must_use]
    pub fn report_line(&self) -> String {
        match self {
            Self::File { path, bytes } => format!(
                "opened: path={} bytes={bytes} format={DOCUMENT_FORMAT}",
                path.display()
            ),
            Self::Sample(sample) => format!(
                "project-source: sample={} ({}; 未给 --open ⇒ 未读任何文件)",
                sample.name(),
                sample.origin_label()
            ),
        }
    }

    /// `saved:` 行里的 `from=` 取值（自由文本，取到行尾）。
    #[must_use]
    pub fn save_origin(&self) -> String {
        match self {
            Self::File { path, .. } => {
                format!("{} (format={DOCUMENT_FORMAT})", path.display())
            }
            Self::Sample(sample) => {
                format!(
                    "sample={} ({}, 不是从文件打开的)",
                    sample.name(),
                    sample.origin_label()
                )
            }
        }
    }

    /// 界面上的"保存"该写回哪个文件（`ROAD-M4-008` 选项 (a) 的保存 UI 入口）。
    ///
    /// **为什么样本形态是 `None` 而不是猜一个文件名**：`--project-sample` / 不带 `--open`
    /// 的会话**没有磁盘对应物**（[`Self::report_line`] 已经如实写了"未读任何文件"）。
    /// 那时"保存"唯一诚实的行为是报"未配置保存路径"，而不是在用户的当前目录里凭空造出
    /// 一个 `untitled.yeban` —— 那是"界面上多了一个文件"这种查不出的现象。
    /// 想把样本落盘的路径今天就存在：`--save-as <path>`（本函数不改变那条路）。
    #[must_use]
    pub fn save_target(&self) -> Option<PathBuf> {
        match self {
            Self::File { path, .. } => Some(path.clone()),
            Self::Sample(_) => None,
        }
    }
}

/// 当前工程 + 它的来源。
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// 全保真的归档（`--save-as` 用它 ⇒ 资产池与 `history.dag` 不会静默丢失）。
    pub archive: ProjectArchive,
    /// 来源（文件 / 样本）。
    pub source: ProjectSource,
}

/// 命令行的失败（每一种都带**精确**原因与退出码）。
#[derive(Debug)]
pub enum CliError {
    /// `--open` 失败。
    Open {
        /// 调用方给的路径。
        path: PathBuf,
        /// 容器 / I/O 层的原样裁决。
        source: OpenError,
    },
    /// 工程无法投影成界面状态（只有需要投影的命令会撞上）。
    Projection {
        /// `bridge` 的裁决。
        source: BridgeError,
    },
    /// 界面路径失败：无法创建窗口 / 事件循环异常。
    ///
    /// 为什么要有**独立**一档而不是复用 [`Self::Projection`]：这两件事的失败原因完全不同
    /// （"工程画不出来" vs "这个环境没有显示器"），把后者塞进前者会打印一条**假的**原因
    /// —— 那正是本模块最忌讳的失败模式。
    Ui {
        /// 精确原因（原样带着 Slint 的错误文本）。
        detail: String,
    },
    /// `--save-as` 失败。
    Save {
        /// 目标路径。
        path: PathBuf,
        /// 落盘 / 容器层的原样裁决。
        source: SaveError,
    },
    /// `--export-elements` 失败。
    Export {
        /// 目标路径。
        path: PathBuf,
        /// 落盘层的原样裁决。
        source: SaveError,
    },
    /// `--export-midi` 失败（投影 / 编码 / 落盘）。
    ExportMidi {
        /// 目标路径。
        path: PathBuf,
        /// 导出层的原样裁决（含 `yeban-render` 编码器的拒绝原因）。
        source: MidiExportError,
    },
    /// `--export-als` 失败（映射 / Gzip 封装 / 落盘）。
    ExportAls {
        /// 目标路径。
        path: PathBuf,
        /// 导出层的原样裁决（含 `yeban-render` 的 `als` 导出器的拒绝原因）。
        source: AlsExportError,
    },
    /// `--export-logic` 失败（bundle 目录 / 原子落盘）。
    ExportLogic {
        /// 目标 bundle 目录。
        path: PathBuf,
        /// 导出层的原样裁决。
        source: LogicExportError,
    },
    /// `--export-als` 被送进了**没有**编译 `experimental-als-export` 的 [`run_batch`]。
    ///
    /// 防假绿的第二道（第一道在 `parse()`）：那段导出代码在默认构建里根本不存在，
    /// 静默跳过只会让人以为 `.als` 写了。`Options` 的字段是公开的，判据可以直接
    /// 构造这个组合，因此这一档必须存在。
    AlsNotCompiled,
    /// `--export-logic` 被送进了**没有**编译 `experimental-logic-export` 的 [`run_batch`]。
    ///
    /// 与 [`Self::AlsNotCompiled`] 同款的第二道防假绿。
    LogicNotCompiled,
    /// `--headless-idle` 被送进了**零 Slint 依赖**的 [`run_batch`]。
    ///
    /// 这一档存在的唯一理由是**防假绿**：该开关的语义是"**真的**构造 Slint 控件树"，
    /// 而 [`run_batch`] 一个 Slint 对象都不构造。若它在这里被静默执行，
    /// `BASELINE-002` 就会拿到一个"看着成功、其实没建树"的读数 ——
    /// 那正是本工作线要消灭的那种绿。正确入口是 `crate::headless_idle::run`。
    HeadlessIdleNotBatch,
    /// `--enable-mcp-http` 被送进了 [`run_batch`]（`[ROAD-M4-001]`）。
    ///
    /// 同款防假绿：控制面要挂在**正在跑的** GUI 进程里；批处理路径跑完就退出，
    /// 在它里面挂控制面等于刚一绑好就拆掉。`parse()` 已经拒了这条组合，
    /// 这里是 `Options` 被直接构造时的第二道。
    McpHttpNotBatch,
}

impl CliError {
    /// 这一档失败的退出码（`--help` 里对用户逐条写明）。
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Projection { .. }
            | Self::Ui { .. }
            | Self::HeadlessIdleNotBatch
            | Self::McpHttpNotBatch => EXIT_UI,
            Self::AlsNotCompiled => EXIT_USAGE,
            Self::LogicNotCompiled => EXIT_USAGE,
            Self::Open { .. } => EXIT_OPEN,
            Self::Save { .. } => EXIT_SAVE,
            Self::Export { .. }
            | Self::ExportMidi { .. }
            | Self::ExportAls { .. }
            | Self::ExportLogic { .. } => EXIT_EXPORT,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { path, source } => {
                write!(formatter, "打开 `{}` 失败: {source}", path.display())
            }
            Self::Projection { source } => {
                write!(formatter, "工程投影失败 ({source}) —— 界面无法由此工程驱动")
            }
            Self::Ui { detail } => write!(formatter, "{detail}"),
            Self::Save { path, source } => {
                write!(formatter, "保存到 `{}` 失败: {source}", path.display())
            }
            Self::Export { path, source } => write!(
                formatter,
                "导出元素清单到 `{}` 失败: {source}",
                path.display()
            ),
            Self::ExportMidi { path, source } => {
                write!(
                    formatter,
                    "导出 MIDI 到 `{}` 失败: {source}",
                    path.display()
                )
            }
            Self::ExportAls { path, source } => {
                write!(
                    formatter,
                    "导出 .als 到 `{}` 失败: {source}",
                    path.display()
                )
            }
            Self::ExportLogic { path, source } => {
                write!(
                    formatter,
                    "导出 .logicx 到 `{}` 失败: {source}",
                    path.display()
                )
            }
            Self::AlsNotCompiled => write!(
                formatter,
                "`{ALS_EXPORT_SWITCH}` 需要本次构建带 `--features {ALS_EXPORT_FEATURE}` \
                 (默认构建里那条导出路径不存在; parse() 与 run_batch() 都会拒绝, 绝不静默忽略)"
            ),
            Self::LogicNotCompiled => write!(
                formatter,
                "`{LOGIC_EXPORT_SWITCH}` 需要本次构建带 `--features {LOGIC_EXPORT_FEATURE}` \
                 (默认构建里那条导出路径不存在; parse() 与 run_batch() 都会拒绝, 绝不静默忽略)"
            ),
            Self::HeadlessIdleNotBatch => write!(
                formatter,
                "`--headless-idle` 不能走 run_batch: 那条路径零 Slint 依赖, \
                 一个控件树对象都不构造 ⇒ 读数会假绿; 正确入口是 crate::headless_idle::run"
            ),
            Self::McpHttpNotBatch => write!(
                formatter,
                "`{MCP_HTTP_SWITCH}` 不能走 run_batch: 那条路径是一次性命令 (跑完就退出), \
                 而控制面要挂在**正在跑的** GUI 进程里 (形态 A) ⇒ 挂上去只会刚绑好就拆掉"
            ),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } => Some(source),
            Self::Projection { source } => Some(source),
            Self::Save { source, .. } | Self::Export { source, .. } => Some(source),
            Self::ExportMidi { source, .. } => Some(source),
            Self::ExportAls { source, .. } => Some(source),
            Self::ExportLogic { source, .. } => Some(source),
            Self::Ui { .. }
            | Self::HeadlessIdleNotBatch
            | Self::McpHttpNotBatch
            | Self::AlsNotCompiled
            | Self::LogicNotCompiled => None,
        }
    }
}

/// 取出当前工程：`--open` 的文件，或 `--project-sample` 选中的内置样本。
///
/// 内置样本的归档 `history.dag` / 资产池为**空**（样本本来就没有这两层；
/// `[MODEL-ISO-001]` 的三层状态里它们是第二、第三层）—— 这是如实置空，不是丢失。
///
/// # Errors
///
/// [`CliError::Open`]：读文件失败 / 超过上限 / 不是 `.yeban` 容器 / 容器拒绝。
pub fn load_project(options: &Options) -> Result<Loaded, CliError> {
    match options.open.as_ref() {
        Some(path) => {
            let opened: OpenedProject =
                open_project_document_file(path, &ProjectOpenOptions::default()).map_err(
                    |source| CliError::Open {
                        path: path.clone(),
                        source,
                    },
                )?;
            Ok(Loaded {
                archive: opened.archive,
                source: ProjectSource::File {
                    path: path.clone(),
                    bytes: opened.file_bytes,
                },
            })
        }
        None => Ok(Loaded {
            archive: ProjectArchive {
                project: options.sample.project(),
                history_dag: Vec::new(),
                assets: Vec::new(),
            },
            source: ProjectSource::Sample(options.sample),
        }),
    }
}

/// 把工程投影成界面状态（**唯一**的投影入口，界面路径与判据共用）。
///
/// # Errors
///
/// [`CliError::Projection`]：`bridge` 的裁决（越界 tick / 像素溢出 / 零缩放 …）。
pub fn project_view(project: &YebanProjectV1) -> Result<ViewState, CliError> {
    ViewState::from_project(project).map_err(|source| CliError::Projection { source })
}

// ---------------------------------------------------------------------------
// 报告（诚实输出）
// ---------------------------------------------------------------------------

/// 工程侧的只读事实（**从模型结构直接数出来的**，不经过界面投影）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectCounts {
    /// `project.tracks` 条目数（**含**主总线）。
    pub tracks: usize,
    /// 主总线轨道是否存在（0 / 1）。
    pub master_track: usize,
    /// 场景数。
    pub scenes: usize,
    /// 段落数。
    pub sections: usize,
    /// 片段池条目数。
    pub clips: usize,
    /// MIDI 音符数（按音符身份去重 —— 与界面投影**同一口径**）。
    pub midi_notes: usize,
    /// 工程资产索引条目数（`project.assets`）。
    pub assets_indexed: usize,
    /// 归档里实际存在的资产字节对象数（`assets/{sha256}`）。
    pub asset_blobs: usize,
    /// `history.dag` 的字节数。
    pub history_bytes: usize,
}

/// 数出工程侧的只读事实（见 [`ProjectCounts`]）。
///
/// 音符去重口径与 `bridge::from_project` 一致（同一音符身份出现在两个片段里只算一次）：
/// 两份口径不一致的话，报告里的 `midi-notes` 与 `view-counts: notes` 会互相打脸。
#[must_use]
pub fn project_counts(loaded: &Loaded) -> ProjectCounts {
    let project = &loaded.archive.project;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for entry in project.clip_pool.values() {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        for note in notes.values() {
            seen.insert(note.id.to_canonical_string());
        }
    }
    ProjectCounts {
        tracks: project.tracks.len(),
        master_track: usize::from(project.tracks.contains_key(&project.master_bus_track_id)),
        scenes: project.scenes.len(),
        sections: project.sections.len(),
        clips: project.clip_pool.len(),
        midi_notes: seen.len(),
        assets_indexed: project.assets.len(),
        asset_blobs: loaded.archive.assets.len(),
        history_bytes: loaded.archive.history_dag.len(),
    }
}

/// 当前工程的报告行（第一行说来源，后面两行说事实）。
///
/// 格式是 `key=value` 的空格分隔串（稳定、可被脚本与 AI 直接消费）；
/// 唯一的自由文本字段是行尾的 `title="…"`（最小转义：`\\` `\"` `\n` `\r` `\t`，
/// 其余控制字符写作 `\u{…}`）。
#[must_use]
pub fn project_report(loaded: &Loaded) -> Vec<String> {
    let counts = project_counts(loaded);
    let project = &loaded.archive.project;
    vec![
        loaded.source.report_line(),
        format!(
            "project: id={} bpm={:.2} ts={}/{} title={}",
            project.id.to_canonical_string(),
            project.bpm,
            project.time_signature.numerator,
            project.time_signature.denominator,
            quoted(&project.title),
        ),
        format!(
            "project-counts: tracks-all={} master-track={} scenes={} sections={} clips-pool={} \
             midi-notes={} assets-indexed={} asset-blobs={} history-bytes={}",
            counts.tracks,
            counts.master_track,
            counts.scenes,
            counts.sections,
            counts.clips,
            counts.midi_notes,
            counts.assets_indexed,
            counts.asset_blobs,
            counts.history_bytes,
        ),
    ]
}

/// 界面投影侧的读数（`view-counts:` 一行）。
///
/// 这一行的意义：证明"工程**真的**能被投影成界面状态"（而不是只被反序列化过）。
/// 元素数 / 动态区域数来自 [`ElementRegistry`] —— 与 `--dump-elements` 打的是同一份注册表。
#[must_use]
pub fn view_report(view: &ViewState) -> Vec<String> {
    let registry = ElementRegistry::from_view(view);
    vec![format!(
        "view-counts: tracks={} master={} clips={} notes={} sections={} scenes={} \
         elements={} dynamic-regions={}",
        view.tracks.len(),
        usize::from(view.master.is_some()),
        view.clips.len(),
        view.notes.len(),
        view.sections.len(),
        view.scenes.len(),
        registry.len(),
        registry.dynamic_regions().count(),
    )]
}

/// `--headless-idle` 的**见证读数**（由 Slint 侧测出，格式留在这里 —— 与其它报告行同源）。
///
/// 为什么要有这个结构：`BASELINE-002` 的判据不能只是"进程跑完了"。它必须能回答
/// "**真的**建了控件树吗、建的是哪个尺寸的窗口、有没有真的光栅化出像素"。
/// 这里的每个字段都是**当场量出来的**事实：
///
/// - `windows_created`：平台 `create_window_adapter` 被调用了几次（由平台自己数，
///   不是猜的）。0 ⇒ Slint 根本没要窗口，也就没有控件树；
/// - `rendered` + `lines` + `non_black_pixels`：逐行光栅化的结果。**像素是伪造不了的**：
///   只有真的存在一棵被布局过的控件树，`SoftwareRenderer` 才吐得出非零行数与非黑像素；
/// - `elements`：复用 [`view_report`] 的**同一份**注册表读数（`--headless` 也有这一行），
///   因此这个"元素数"在同一命令的两个模式之间**可比**；
/// - `width` / `height`：窗口物理尺寸，来自 `DemoScene` 的视口（与 GUI / Tier-1 端口同源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleWitness {
    /// 平台被要求创建窗口适配器的次数（应当恰好 1）。
    pub windows_created: usize,
    /// 窗口物理宽度。
    pub width: u32,
    /// 窗口物理高度。
    pub height: u32,
    /// 本次是否真的发生了重绘（`draw_if_needed` 的返回值）。
    pub rendered: bool,
    /// 逐行光栅化回调被调用的行数（应当等于 `height`）。
    pub lines: u64,
    /// 光栅化后非黑（RGB 非全零）的像素数。
    pub non_black_pixels: u64,
    /// 出现过的不同颜色数（> 1 ⇒ 不是一块纯色，说明真的画了多个图元）。
    pub distinct_colors: usize,
    /// 投影侧的元素数（与 `view-counts:` 的 `elements=` 同一份注册表）。
    pub elements: usize,
}

impl IdleWitness {
    /// 是否是**非平凡**的见证：窗口恰好一个、尺寸非零、真的光栅化出非黑像素。
    ///
    /// 这条方法就是"防空转"的机械判据 —— 真二进制判据与 CI 都问它，而不是各自写一套阈值。
    #[must_use]
    pub fn is_non_trivial(&self) -> bool {
        self.windows_created == 1
            && self.width > 0
            && self.height > 0
            && self.rendered
            && self.lines == u64::from(self.height)
            && self.non_black_pixels > 0
    }

    /// 非黑像素占全窗口的**千分比**（整数，避免浮点格式漂移）。
    #[must_use]
    pub fn non_black_permille(&self) -> u64 {
        let total = u64::from(self.width) * u64::from(self.height);
        if total == 0 {
            return 0;
        }
        self.non_black_pixels.saturating_mul(1000) / total
    }
}

/// 见证行（`headless-idle-witness:`）—— 一行 `key=value`，机器可读。
#[must_use]
pub fn idle_witness_line(witness: &IdleWitness) -> String {
    format!(
        "headless-idle-witness: windows-created={} size={}x{} rendered={} lines={} \
         non-black-pixels={} non-black-permille={} distinct-colors={} elements={}",
        witness.windows_created,
        witness.width,
        witness.height,
        witness.rendered,
        witness.lines,
        witness.non_black_pixels,
        witness.non_black_permille(),
        witness.distinct_colors,
        witness.elements,
    )
}

/// `--headless-idle` 的空闲读数（`headless-idle-idle:` 一行）。
///
/// `elapsed-ms` 是**实测**墙钟时间（不是 `seconds × 1000`）—— 判据 ③ 要比较"空闲 N 秒"
/// 与"空闲 M 秒"两档，若把请求值原样回显，这一行就只是把输入抄了一遍，什么也没证明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleReport {
    /// 请求的空闲秒数。
    pub seconds: u32,
    /// 实测的空闲墙钟毫秒数。
    pub elapsed_ms: u128,
    /// 空闲期间睡过的 tick 数。
    pub ticks: u64,
}

/// 空闲行（`headless-idle-idle:`）。
#[must_use]
pub fn idle_report_line(report: &IdleReport) -> String {
    format!(
        "headless-idle-idle: seconds={} elapsed-ms={} ticks={}",
        report.seconds, report.elapsed_ms, report.ticks
    )
}

/// 把一行文本转成"引号包裹 + 最小转义"的形式（见 [`project_report`] 的格式说明）。
#[must_use]
pub fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other.is_control() => {
                out.push_str(&format!("\\u{{{:x}}}", u32::from(other)));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// 快捷键表（`--print-shortcuts`）
// ---------------------------------------------------------------------------

/// 快捷键表里"这一行还没有可作用的实现"的标记。
///
/// 它出现在 [`shortcut_lines`] 的**规范写法**一列。存在的理由只有一条：表里写着某个键,
/// 用户就会去按; 而 `host::apply_action` 对没有落地的动作返回 `false`（`reject`）。
/// 没有这个标记, 表就是一份**假承诺** —— 用户按了没反应, 只能自己猜是键坏了还是功能没做。
pub const UNIMPLEMENTED_MARKER: &str = "(未实现)";

/// 快捷键表的一行：规范写法 + 领域动作 + 物理键 + 修饰位 + **本版本是否已有落地**。
///
/// `implemented` 是**写下来的主张**, 不是从这里推出来的事实：本模块是**零 Slint 依赖**
/// 的命令行面（见 `cli.rs` 的模块文档）, 不能引用 `crate::host`（那个模块依赖 Slint,
/// 引用它会把 Slint 拉进命令行的零依赖探针）。主张与事实的对账住在
/// `tests/cli_contract.rs` 的 `shortcut_table_status_matches_the_resolution_and_host_pipeline`：
/// 它把 `implemented` 与 [`InputContext::resolve`] 的解析结果、`host::action_has_implementation`
/// （宿主自己的陈述）逐行比对 —— 任一侧分叉即变红。
pub struct ShortcutRow {
    /// 规范 §7.1 表格里的写法（**不含** [`UNIMPLEMENTED_MARKER`]）。
    pub label: &'static str,
    /// [`ShortcutRow::label`] 声称的那个领域动作。
    pub action: Action,
    /// 绑定用的物理键。
    pub key: PhysicalKey,
    /// 绑定用的修饰位。
    pub modifiers: Modifiers,
    /// 本版本是否已有可作用的实现（宿主会不会消费这一键）。
    pub implemented: bool,
}

/// 规范 §7.1 的核心快捷键表, 用于 `--print-shortcuts`。
///
/// 每一项把**规范写法**、**动作**、**键位**与**落地状态**放在一起。这张表的**意义**是
/// 让策略表的判定结果可被人与 CI 直接阅读 —— 尤其是 `[UI-A11Y-002]` 的 IME 分支,
/// 以及"规范点名了、本版本还没做"的那一列（见 [`UNIMPLEMENTED_MARKER`]）。
const SHORTCUTS: [ShortcutRow; 18] = [
    ShortcutRow {
        label: "Space → 播放/暂停",
        action: Action::PlayPause,
        key: PhysicalKey::Space,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "Shift+Space → 从光标处续播",
        action: Action::ResumeFromCursor,
        key: PhysicalKey::Space,
        modifiers: Modifiers::shift(),
        implemented: true,
    },
    ShortcutRow {
        label: "Tab → 视图切换 (仅画布聚焦)",
        action: Action::ToggleView,
        key: PhysicalKey::Tab,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "F5 → Session 视图",
        action: Action::ShowView(View::Session),
        key: PhysicalKey::F5,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "F6 → Arrangement 视图",
        action: Action::ShowView(View::Arrangement),
        key: PhysicalKey::F6,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Z → 撤销",
        action: Action::Undo,
        key: PhysicalKey::KeyZ,
        modifiers: Modifiers::meta(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Shift+Z → 重做",
        action: Action::Redo,
        key: PhysicalKey::KeyZ,
        modifiers: Modifiers::ctrl_shift(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Shift+H → 时光机",
        action: Action::OpenTimeMachine,
        key: PhysicalKey::KeyH,
        modifiers: Modifiers::ctrl_shift(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+D → 原位复制",
        action: Action::Duplicate,
        key: PhysicalKey::KeyD,
        modifiers: Modifiers::meta(),
        implemented: false,
    },
    ShortcutRow {
        label: "Delete/Backspace → 删除",
        action: Action::DeleteSelection,
        key: PhysicalKey::Delete,
        modifiers: Modifiers::none(),
        implemented: false,
    },
    ShortcutRow {
        label: "B → 箭头/铅笔切换",
        action: Action::TogglePencilTool,
        key: PhysicalKey::KeyB,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+B → 左抽屉",
        action: Action::ToggleSidebar,
        key: PhysicalKey::KeyB,
        modifiers: Modifiers::ctrl_alt(),
        implemented: true,
    },
    ShortcutRow {
        label: "Z → 选区撑满视口",
        action: Action::ZoomToSelection,
        key: PhysicalKey::KeyZ,
        modifiers: Modifiers::none(),
        implemented: false,
    },
    ShortcutRow {
        label: "Shift+Z → 全曲总览",
        action: Action::ZoomToFit,
        key: PhysicalKey::KeyZ,
        modifiers: Modifiers::shift(),
        implemented: false,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+M → 控制台最大化",
        action: Action::ToggleConsoleMaximize,
        key: PhysicalKey::KeyM,
        modifiers: Modifiers::ctrl_alt(),
        implemented: true,
    },
    ShortcutRow {
        label: "1 → 选择工具",
        action: Action::SelectTool(Tool::Select),
        key: PhysicalKey::Digit(1),
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "Shift+Enter → 采纳 AI 建议",
        action: Action::AcceptAiSuggestion,
        key: PhysicalKey::Enter,
        modifiers: Modifiers::shift(),
        implemented: false,
    },
    ShortcutRow {
        label: "[ → 试听主线",
        action: Action::AuditionMain,
        key: PhysicalKey::BracketLeft,
        modifiers: Modifiers::none(),
        implemented: false,
    },
];

/// [`SHORTCUTS`] 的只读视图。
///
/// 给判据用：`--print-shortcuts` 的文本是这张表渲染出来的，判据要能把**渲染结果**与
/// `input` 的解析、`host` 的接受/拒绝逐行对账（见 `tests/cli_contract.rs`）。
#[must_use]
pub fn shortcut_rows() -> &'static [ShortcutRow] {
    &SHORTCUTS
}

/// 打印 `[UI-A11Y-001]` / `[UI-A11Y-002]` 策略表在本版本下的判定结果。
///
/// 同时给出**画布聚焦**与**IME 合成态**两列 —— 这两列必须不同, 而且差异必须是
/// "合成态什么都收不到"。这个输出本身就是给 CI 与人看的一份活文档。
///
/// 规范写法一列还带**落地状态**：宿主还没有可作用实现的键会被标上
/// [`UNIMPLEMENTED_MARKER`]（键**已绑定**、解析得出动作、但 `host::apply_action` 会
/// `reject`）。表与宿主的一致性由 `tests/cli_contract.rs` 的判据逐行对账，不再靠人记得同步。
#[must_use]
pub fn shortcut_lines() -> Vec<String> {
    let mut lines = vec![
        "# 快捷键策略表 (yeban-app scaffold)".to_owned(),
        "# 第三列是 [UI-A11Y-002] 的核心: 合成态下所有非 F5/F6 的键都必须被输入法吞掉".to_owned(),
        format!(
            "# {UNIMPLEMENTED_MARKER} = 键已绑定、解析得出动作, 但宿主尚无落地实现 (按下被 reject, 不是键坏了)"
        ),
        format!(
            "# {:<34} {:<28} {:<28}",
            "规范写法", "画布聚焦", "文本输入 + IME 合成态"
        ),
    ];

    let mut canvas = InputContext::new();
    canvas.set_focus(crate::input::Focus::MainCanvas);

    let mut composing = InputContext::new();
    composing.set_focus(crate::input::Focus::TextInput);
    composing.begin_composition();

    for row in SHORTCUTS {
        let label = if row.implemented {
            row.label.to_owned()
        } else {
            format!("{} {}", row.label, UNIMPLEMENTED_MARKER)
        };
        lines.push(format!(
            "  {:<34} {:<28} {:<28}",
            label,
            format!("{:?}", canvas.resolve(row.key, row.modifiers)),
            format!("{:?}", composing.resolve(row.key, row.modifiers)),
        ));
    }
    lines
}

/// `--print-theme` 的行（`--theme` 的**自述**报告）。
///
/// 每一行都对应一次**真的读到**的事实，而不是把输入抄一遍：
/// - `requested=` 是解析出来的主题的**规范名**（[`Theme::name`]）：别名 `yeban` 解析到
///   `default`，所以两种写法打出来的这一行**逐字相同**（别名不产生第二支调色板）；
///   不给 `--theme` 时也是 `default`（默认主题）；
/// - `palette=` 是这次请求**实际**会让 `ui/tokens.slint` 走的调色板来源
///   （`brand` = 本仓品牌深色那一串十六进制字面量 / `yeban-measured-literals` = 第二串
///   十六进制字面量
///   —— 这个键名是 2026-10-06 的**历史标识**, 现在那串值的来源是负责人下发的调色板,
///   键名保持不变以免改动 CLI 契约 / `inkmoor-owner-literals` / `plume-owner-literals`
///   = 2026-10-07 新增的两支负责人下发的字面量（键名不再沿用那个会误导的
///   `measured`）/ `design-system-palette` = Slint 的 `Palette.*` 角色）；
/// - `compiled-style=` 是 `build.rs` 注入的**编译期**事实（`cargo:rustc-env`），
///   不是从命令行推出来的；
/// - `style-switch=` 是那个**必须**说出来的区别：Slint 1.18.1 换风格只能在编译期，
///   所以四个内建名字共享这一个二进制里编进来的风格；
/// - `theme-source:` 是**这一支的字面量从哪来**（四支自绘调色板各有权威设计输入；
///   走设计系统的四支没有这一行 —— 绝不写一句假出处）。
#[must_use]
pub fn theme_lines(theme: Theme) -> Vec<String> {
    let requested = theme.name();
    let palette = theme.palette_source();
    let compiled = compiled_slint_style();
    let mut lines = vec![
        "# 主题报告 (yeban-app)".to_owned(),
        format!("theme: requested={requested} palette={palette} compiled-style={compiled}"),
    ];
    match theme.requested_slint_style() {
        Some(style) if style == compiled => lines.push(format!(
            "theme-style: 请求的风格 `{style}` 与编译进来的风格**一致** —— \
             `Palette` 就是这个风格的调色板"
        )),
        Some(style) => lines.push(format!(
            "theme-style: 请求的风格 `{style}` **没有**编进这个二进制 (编进来的是 `{compiled}`) \
             —— Slint 1.18.1 换风格只能编译期做, 因此 `Palette` 现在给的是 `{compiled}` 的调色板; \
             要真的拿到 `{style}`, 重新构建时把环境变量 {SLINT_STYLE_ENV}={style} 交给 cargo"
        )),
        None => lines.push(format!(
            "theme-style: 没有请求任何内建风格 —— {} 不经过 Slint `Palette`, \
             因此与编进来的风格 (`{compiled}`) 无关",
            match theme {
                Theme::Yeban =>
                    "`default` (别名 `yeban`) 用的是负责人下发的那串十六进制字面量 \
                     (2026-10-07 HTML mock; 取代 2026-10-06 的采样推导) —— 它就是**默认主题**",
                Theme::InkMoor =>
                    "`inkmoor` 用的是负责人设计稿「墨泊 InkMoor」的那串十六进制字面量 \
                     (HTML mock 的 body.inkmoor 变量块; 见 ui/tokens.slint §6e)",
                Theme::Plume =>
                    "`plume` 用的是负责人设计稿「孤烟 Plume」的那串十六进制字面量 \
                     (HTML mock 的 body.plume 变量块; 见 ui/tokens.slint §6e)",
                _ => "`brand` 用的是本仓品牌深色的那串十六进制字面量 (2026-10-07 之前的默认外观)",
            }
        )),
    }
    // 出处行: 这一支的 hex 从哪来。四支自绘调色板各有权威设计输入, 走设计系统的
    // 四支没有 ⇒ 没有这一行 (绝不写一句"出处"骗人)。
    if let Some(source) = theme.design_source() {
        lines.push(format!("theme-source: {source}"));
    }
    lines.push(format!(
        "theme-note: `{THEME_SWITCH}` 只换**调色板**; 本仓界面 100% 自绘 \
         (Rectangle x74 / 内建控件 x0), 所以 Slint 内建风格本身改不动我们的像素"
    ));
    lines
}

// ---------------------------------------------------------------------------
// 执行
// ---------------------------------------------------------------------------
/// 无窗口路径的执行：返回**它应当打印的每一行**（打印由 [`emit`] / [`finish`] 做）。
///
/// 顺序是契约的一部分：
/// 1. `--dump-elements` 的元素行（先于握手行 —— 沿用旧行为）;
/// 2. `--print-shortcuts` 的策略表;
/// 3. 握手行 [`HEADLESS_HANDSHAKE`]（无窗口路径**一定**打印它）;
/// 4. 来源行 / `project:` / `project-counts:`（需要投影时再加 `view-counts:`）;
/// 5. 边界声明行（`headless:` 开头，说清这一版无头**没有**验证什么）;
/// 6. `--export-elements` 的 `exported:`（失败 ⇒ 直接 `Err`，**不**继续保存）;
/// 7. `--export-midi` 的 `exported-midi:`（失败 ⇒ 直接 `Err`，**不**继续保存）;
/// 8. `--export-als` 的 `exported-als:` + 损失表行（同样失败即止）;
/// 9. `--export-logic` 的 `exported-logic:` + 损失表行（同样失败即止）;
/// 10. `--save-as` 的 `saved:`。
///
/// # Errors
///
/// [`CliError`]：打开 / 投影 / 保存 / 导出失败（退出码见 [`CliError::exit_code`]）。
pub fn run_batch(options: &Options) -> Result<Vec<String>, CliError> {
    if options.help {
        return Ok(usage_text().lines().map(str::to_owned).collect());
    }
    if options.version {
        return Ok(vec![version_text()]);
    }
    if options.headless_idle {
        // 防假绿守卫，**不是**形式主义：本函数的模块文档第一句就是"零 Slint 依赖"，
        // 而 `--headless-idle` 的全部价值在于"真的建了控件树"。放它过去 ⇒
        // `BASELINE-002` 会拿到一个"命令成功、但一个 Slint 对象都没构造"的读数。
        return Err(CliError::HeadlessIdleNotBatch);
    }
    if options.enable_mcp_http {
        // 同款防假绿：`--enable-mcp-http` 的语义是"把控制面挂在**正在跑的** GUI 进程里"。
        // 批处理路径是一次性命令（跑完就退出），在这里挂上去等于刚一绑好就拆掉 ——
        // 静默接受只会让人以为服务起来了。`parse()` 已经拒了这条组合，这里是第二道
        // （`Options` 的字段是公开的，判据可以直接构造组合）。
        return Err(CliError::McpHttpNotBatch);
    }
    // `--export-als` 的防假绿第二道（与 `parse()` 同一条纪律）：默认构建里那条导出
    // 路径根本不存在（`yeban-render` 的 `als` 模块与 `flate2` 都不在依赖图上）。
    // 静默跳过 = 用户以为 `.als` 写了而磁盘上什么都没有。
    #[cfg(not(feature = "experimental-als-export"))]
    if options.export_als.is_some() {
        return Err(CliError::AlsNotCompiled);
    }
    // `--export-logic` 的防假绿第二道（同款）：默认构建里 `yeban-render` 的 `logic`
    // 模块根本不存在，静默跳过 = 用户以为 bundle 写了而磁盘上什么都没有。
    #[cfg(not(feature = "experimental-logic-export"))]
    if options.export_logic.is_some() {
        return Err(CliError::LogicNotCompiled);
    }

    let loaded = load_project(options)?;
    let mut lines: Vec<String> = Vec::new();

    // 需要投影的命令先投影：投影失败要**在**任何写盘动作之前失败。
    let view = if options.needs_projection() {
        Some(project_view(&loaded.archive.project)?)
    } else {
        None
    };

    if options.dump_elements {
        let view = view.as_ref().expect("needs_projection 保证视图已构造");
        lines.extend(ElementRegistry::from_view(view).dump_lines());
    }
    if options.print_shortcuts {
        lines.extend(shortcut_lines());
    }
    if options.print_theme {
        lines.extend(theme_lines(options.theme));
    }

    lines.push(HEADLESS_HANDSHAKE.to_owned());
    lines.extend(project_report(&loaded));
    if let Some(view) = view.as_ref() {
        lines.extend(view_report(view));
    }
    lines.push(HEADLESS_BOUNDARY.to_owned());

    if let Some(path) = options.export_elements.as_ref() {
        let view = view.as_ref().expect("needs_projection 保证视图已构造");
        let text = element_dump_text(view);
        let report =
            write_file_atomically(text.as_bytes(), path).map_err(|source| CliError::Export {
                path: path.clone(),
                source,
            })?;
        lines.push(format!(
            "exported: path={} lines={} bytes={} temp={}",
            report.path.display(),
            text.lines().count(),
            report.bytes,
            report.temp_name,
        ));
    }

    if let Some(path) = options.export_midi.as_ref() {
        let report = export_project_to_file(&loaded.archive.project, path).map_err(|source| {
            CliError::ExportMidi {
                path: path.clone(),
                source,
            }
        })?;
        lines.push(exported_midi_line(&report, &loaded));
    }

    // 顺序契约（`--help` 的"组合语义"一节）：元素 → MIDI → .als → .logicx → 保存工程。
    // 与 `--export-midi` 一样, 导出失败 ⇒ 直接 `Err`, **不**继续保存。
    #[cfg(feature = "experimental-als-export")]
    if let Some(path) = options.export_als.as_ref() {
        let report = crate::export_als::export_project_to_file(&loaded.archive.project, path)
            .map_err(|source| CliError::ExportAls {
                path: path.clone(),
                source,
            })?;
        lines.push(exported_als_line(&report, &loaded));
        lines.extend(als_loss_lines(&report.losses));
    }

    #[cfg(feature = "experimental-logic-export")]
    if let Some(path) = options.export_logic.as_ref() {
        let report = crate::export_logic::export_project_to_bundle(&loaded.archive.project, path)
            .map_err(|source| CliError::ExportLogic {
            path: path.clone(),
            source,
        })?;
        lines.push(exported_logic_line(&report, &loaded));
        lines.extend(logic_loss_lines(&report.losses));
    }

    if let Some(path) = options.save_as.as_ref() {
        let report = save_archive_file(&loaded.archive, path).map_err(|source| CliError::Save {
            path: path.clone(),
            source,
        })?;
        lines.push(saved_line(&report, &loaded));
    }

    Ok(lines)
}

/// `--export-elements` 的字节内容：与 `--dump-elements` 打到 stdout 的**同一份**行。
///
/// 末尾补一个换行: 让文件以 `\n` 结尾（POSIX 文本文件），因此 `lines()` 数与
/// 打印出的行数一致。
fn element_dump_text(view: &ViewState) -> String {
    let mut text = ElementRegistry::from_view(view).dump_lines().join("\n");
    text.push('\n');
    text
}

/// `saved:` 行（说清落点、字节数、用过的临时文件、以及**保住的两层**与**来源**）。
fn saved_line(report: &SaveReport, loaded: &Loaded) -> String {
    format!(
        "saved: path={} bytes={} history-bytes={} assets={} temp={} from={}",
        report.path.display(),
        report.bytes,
        loaded.archive.history_dag.len(),
        loaded.archive.assets.len(),
        report.temp_name,
        loaded.source.save_origin(),
    )
}

/// `exported-midi:` 行（说清落点、字节数、**写进 `MThd` 的 PPQ 与格式**、轨道 / 音符数、
/// 用过的临时文件与来源）。
///
/// `ppq=` 是**从导出结果读回来的**事实（不是本文件里第二个 960 字面量）：
/// 判据把这一行与文件头的大端字段逐字段对账。
fn exported_midi_line(report: &MidiExportReport, loaded: &Loaded) -> String {
    let format = match report.format {
        MidiFormat::SingleTrack => "single-track",
        MidiFormat::Parallel => "parallel",
    };
    format!(
        "exported-midi: path={} bytes={} ppq={} format={} tracks={} notes={} tempos={} \
         temp={} from={}",
        report.path.display(),
        report.bytes,
        report.ppq,
        format,
        report.tracks,
        report.notes,
        report.tempos,
        report.temp_name,
        loaded.source.save_origin(),
    )
}

/// `--export-als` 打到 stdout 的损失条目**上限**（超过就截断并明写还剩几条）。
///
/// 为什么要有上限：损失表随工程规模增长，一个大工程能吐出上千条 —— 全打到 stdout
/// 会把真正的诊断淹掉。**截断不等于隐藏**：完整的表同时写进了文件的
/// `<!-- yeban-loss … -->` 注释（导出器保证两者同源），而截断行会写明还剩多少条。
///
/// 公开它是为了让真二进制判据能算"应该显示几条"，而不是在判据里抄一个魔数。
#[cfg(feature = "experimental-als-export")]
pub const MAX_ALS_LOSS_LINES: usize = 20;

/// `exported-als:` 行（说清落点、字节数、映射计数、**损失条数**、用过的临时文件与来源）。
///
/// `losses=` 是导出器**返回的**条数（不是本文件里第二个数字）：判据把它与随后打印的
/// `als-loss:` 行数对账。
#[cfg(feature = "experimental-als-export")]
fn exported_als_line(report: &crate::export_als::AlsExportReport, loaded: &Loaded) -> String {
    format!(
        "exported-als: path={} bytes={} tracks={} clips={} notes={} losses={} temp={} from={}",
        report.path.display(),
        report.bytes,
        report.mapped_tracks,
        report.mapped_clips,
        report.mapped_notes,
        report.losses.len(),
        report.temp_name,
        loaded.source.save_origin(),
    )
}

/// 把 `--export-als` 的映射损失表变成**给用户看的行**（`D47` 的价值就在这里）。
///
/// 形状（与既有报告行同一套 `key=value` / `quoted()` 转义）：
///
/// ```text
/// als-losses: count=<总条数>
/// als-loss: entity=<稳定寻址> bounced-to-audio=<true|false> reason="<转义后的人话>"
/// …（最多 [`MAX_ALS_LOSS_LINES`] 条）
/// als-losses-truncated: ... and <N> more (完整表也在文件的 `<!-- yeban-loss ... -->` 注释里)
/// ```
///
/// 顺序与导出器返回的**完全一致**（导出器按 `BTreeMap` 键序产出），因此输出可复现；
/// 本函数只做"取前 N 条 + 报剩余数"，绝不重排、绝不丢字段。
#[cfg(feature = "experimental-als-export")]
fn als_loss_lines(losses: &[yeban_render::als::AlsLoss]) -> Vec<String> {
    let mut lines = vec![format!("als-losses: count={}", losses.len())];
    for loss in losses.iter().take(MAX_ALS_LOSS_LINES) {
        lines.push(format!(
            "als-loss: entity={} bounced-to-audio={} reason={}",
            loss.entity,
            loss.bounced_to_audio,
            quoted(&loss.reason),
        ));
    }
    if losses.len() > MAX_ALS_LOSS_LINES {
        lines.push(format!(
            "als-losses-truncated: ... and {} more (完整表也在文件的 \
             `<!-- yeban-loss ... -->` 注释里)",
            losses.len() - MAX_ALS_LOSS_LINES
        ));
    }
    lines
}

/// `--export-logic` 打到 stdout 的损失条目**上限**（与 `.als` 同一条纪律）。
///
/// 完整的表同时写进了 bundle 的 `Alternatives/<NNN>/MetaData.plist`
/// （`YebanMappingLosses` 键，导出器保证两者同源），而截断行会写明还剩多少条。
#[cfg(feature = "experimental-logic-export")]
pub const MAX_LOGIC_LOSS_LINES: usize = 20;

/// `exported-logic:` 行（说清落点、文件数、映射计数、**损失条数**与来源）。
#[cfg(feature = "experimental-logic-export")]
fn exported_logic_line(report: &crate::export_logic::LogicExportReport, loaded: &Loaded) -> String {
    let bytes: usize = report.files.iter().map(|(_, size)| *size).sum();
    format!(
        "exported-logic: path={} files={} bytes={} regions={} notes={} losses={} from={}",
        report.directory.display(),
        report.files.len(),
        bytes,
        report.mapped_regions,
        report.mapped_notes,
        report.losses.len(),
        loaded.source.save_origin(),
    )
}

/// 把 `--export-logic` 的映射损失表变成**给用户看的行**。
///
/// 形状（与 `als-loss` 同款，去掉 `.als` 特有的 `bounced-to-audio`）：
///
/// ```text
/// logic-losses: count=<总条数>
/// logic-loss: entity=<稳定寻址> reason="<转义后的人话>"
/// …（最多 [`MAX_LOGIC_LOSS_LINES`] 条）
/// logic-losses-truncated: ... and <N> more (完整表也在 MetaData.plist 的 YebanMappingLosses)
/// ```
#[cfg(feature = "experimental-logic-export")]
fn logic_loss_lines(losses: &[yeban_render::logic::LogicLoss]) -> Vec<String> {
    let mut lines = vec![format!("logic-losses: count={}", losses.len())];
    for loss in losses.iter().take(MAX_LOGIC_LOSS_LINES) {
        lines.push(format!(
            "logic-loss: entity={} reason={}",
            loss.entity,
            quoted(&loss.reason),
        ));
    }
    if losses.len() > MAX_LOGIC_LOSS_LINES {
        lines.push(format!(
            "logic-losses-truncated: ... and {} more (完整表也在 MetaData.plist 的 \
             `{}` 键里)",
            losses.len() - MAX_LOGIC_LOSS_LINES,
            yeban_render::logic::META_DATA_LOSS_KEY,
        ));
    }
    lines
}

/// 把行打到 stdout（唯一打印点，保证"返回的"与"打印的"逐行一致）。
pub fn emit(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// 收尾：成功 ⇒ 打印并退出 [`EXIT_OK`]；失败 ⇒ stderr 一行原因 + 对应退出码。
///
/// 失败信息**只**打一行（`yeban-app: <原因>`）：原因的 `Display` 里已经带上了路径与
/// 容器的原始描述，再叠一层用法文本只会把真正的诊断淹掉（用法错误那条另说 ——
/// 它由 `main.rs` 在有用法文本可给时才补）。
pub fn finish(result: Result<Vec<String>, CliError>) -> ExitCode {
    match result {
        Ok(lines) => {
            emit(&lines);
            ExitCode::from(EXIT_OK)
        }
        Err(error) => {
            eprintln!("yeban-app: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::container::{ContainerLimits, write_project_container};
    use yeban_model::ids::{AssetHash, EntityId};

    // ------------------------------------------------------------------
    // 工具
    // ------------------------------------------------------------------

    /// 一次性的临时目录（**不用外部 crate**：`tempfile` 不在本 crate 的依赖里）。
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-cli-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 造一个**真容器**并写到磁盘；返回路径与工程。
    fn write_container_file(dir: &std::path::Path, name: &str) -> (PathBuf, YebanProjectV1) {
        let project = crate::bridge::demo_project();
        let asset = b"yeban-cli-asset".to_vec();
        let hash = AssetHash::of_bytes(&asset);
        let mut assets = std::collections::BTreeMap::new();
        assets.insert(hash, asset);
        let bytes =
            write_project_container(&project, b"cli-history-dag", &assets).expect("写出真容器");
        let path = dir.join(name);
        std::fs::write(&path, &bytes).expect("写容器文件");
        (path, project)
    }

    /// 把报告行拼成一整块文本（判据断言用）。
    fn joined(lines: &[String]) -> String {
        lines.join("\n")
    }

    /// 取某一行里 `key=value` 的 `value`（到下一个空白为止）。
    fn field(line: &str, key: &str) -> Option<String> {
        let needle = format!("{key}=");
        let start = line.find(&needle)? + needle.len();
        let rest = &line[start..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        Some(rest[..end].to_owned())
    }

    // ------------------------------------------------------------------
    // 判据 29: --help 覆盖全部开关 + 短路 + 退出码 0
    // ------------------------------------------------------------------

    #[test]
    fn help_lists_every_switch_and_short_circuits() {
        let usage = usage_text();
        for switch in [
            "--open",
            "--save-as",
            "--dump-elements",
            "--export-elements",
            "--export-midi",
            "--export-als",
            "--export-logic",
            "--print-shortcuts",
            "--theme",
            PRINT_THEME_SWITCH,
            "--project-sample",
            "--headless",
            "--headless-idle",
            "--idle-seconds",
            "--enable-mcp-http",
            "--help",
            "--version",
        ] {
            assert!(usage.contains(switch), "用法文本必须列出 `{switch}`");
        }
        // 环境变量那一路也要在用法里（它是等价的第二入口，藏起来等于没有）。
        assert!(
            usage.contains(MCP_HTTP_ENV),
            "用法文本必须列出 {MCP_HTTP_ENV}"
        );
        // 主题那一路的两个入口同理: `--theme` 是**运行期**换调色板, `SLINT_STYLE` 是
        // **编译期**换风格。只写前者会让用户以为风格也能运行时换。
        assert!(
            usage.contains(SLINT_STYLE_ENV),
            "用法文本必须列出 {SLINT_STYLE_ENV}"
        );
        // 退出码语义必须在用法里逐条写出（用户与脚本的唯一去处）。
        for code in [
            EXIT_OK,
            EXIT_UI,
            EXIT_USAGE,
            EXIT_OPEN,
            EXIT_SAVE,
            EXIT_EXPORT,
        ] {
            assert!(
                usage.contains(&code.to_string()),
                "用法文本必须写出退出码 {code}"
            );
        }
        // 短路: 未知参数也不影响 --help（"帮助永远打得开"）。
        let options = parse(&["--help".to_owned(), "--bogus".to_owned()]).expect("--help 短路");
        assert!(options.help);
        assert!(options.batch() && !options.wants_gui());
        let lines = run_batch(&options).expect("--help 必须成功");
        assert_eq!(joined(&lines), usage.trim_end_matches('\n'));
        assert_eq!(EXIT_OK, 0);
    }

    // ------------------------------------------------------------------
    // 判据 30: --version 与 Cargo.toml 的版本一致（**读清单对账**）
    // ------------------------------------------------------------------

    #[test]
    fn version_matches_the_workspace_manifest() {
        let printed = version_text();
        assert_eq!(printed, format!("yeban-app {}", env!("CARGO_PKG_VERSION")));

        // 直接读根 `Cargo.toml` 的 `[workspace.package] version` 再对一次 ——
        // 这条正是"把 --version 写死常量"会红掉的地方。
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|error| panic!("必须能读根清单 {}: {error}", manifest.display()));
        let expected = workspace_version(&text)
            .unwrap_or_else(|| panic!("根清单里必须能解析出 [workspace.package] version"));
        assert_eq!(
            expected,
            env!("CARGO_PKG_VERSION"),
            "清单与 Cargo 注入的版本必须一致"
        );
        assert_eq!(printed, format!("yeban-app {expected}"));
        assert!(printed.lines().count() == 1, "版本行必须是单行");
    }

    /// 从 `Cargo.toml` 文本里取 `[workspace.package]` 段的 `version = "…"`。
    fn workspace_version(manifest: &str) -> Option<String> {
        let mut in_section = false;
        for line in manifest.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_section = trimmed == "[workspace.package]";
                continue;
            }
            if !in_section {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("version") {
                let value = rest.trim_start().strip_prefix('=')?.trim();
                return Some(value.trim_matches('"').to_owned());
            }
        }
        None
    }

    // ------------------------------------------------------------------
    // 判据 31: --open 真容器 ⇒ 打印的读数与工程一致
    // ------------------------------------------------------------------

    #[test]
    fn open_reports_counts_that_match_the_project() {
        let dir = scratch_dir("open");
        let (path, project) = write_container_file(&dir, "song.yeban");

        let options = parse(&[
            "--open".to_owned(),
            path.display().to_string(),
            "--headless".to_owned(),
        ])
        .expect("解析");
        assert!(!options.wants_gui() && options.needs_projection());
        let lines = run_batch(&options).expect("必须打开成功");
        let text = joined(&lines);

        // 容器里的工程 = 测试手上的那个工程（写进去的就是它）。
        assert_eq!(
            project.tracks.len(),
            7,
            "演示夹具的轨道数变了 ⇒ 本判据需同步"
        );
        let model = lines
            .iter()
            .find(|line| line.starts_with("project-counts:"))
            .expect("必须有 project-counts 行");
        assert_eq!(
            field(model, "tracks-all").as_deref(),
            Some(project.tracks.len().to_string().as_str()),
            "打印的轨道数必须与工程一致: {model}"
        );
        assert!(text.contains("master-track=1"), "工程有主总线\n{text}");
        assert!(
            text.contains(&format!("history-bytes={}", b"cli-history-dag".len())),
            "history.dag 的字节数必须如实报告\n{text}"
        );
        assert!(text.contains("asset-blobs=1"), "资产池必须如实报告\n{text}");
        assert!(text.contains("format=yeban-container"), "{text}");

        // 音符数：与投影同口径地独立数一遍。
        let expected_notes = {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for entry in project.clip_pool.values() {
                let Some(notes) = entry.content.notes() else {
                    continue;
                };
                for note in notes.values() {
                    seen.insert(note.id.to_canonical_string());
                }
            }
            seen.len()
        };
        assert!(expected_notes > 0, "演示夹具必须含 MIDI 音符");
        let midi = lines
            .iter()
            .find(|line| line.starts_with("project-counts:"))
            .expect("必须有 project-counts 行");
        assert_eq!(
            field(midi, "midi-notes").as_deref(),
            Some(expected_notes.to_string().as_str()),
            "打印的音符数必须与工程一致: {midi}"
        );
        let view = lines
            .iter()
            .find(|line| line.starts_with("view-counts:"))
            .expect("--headless 必须给出 view-counts");
        assert_eq!(
            field(view, "tracks").as_deref(),
            Some((project.tracks.len() - 1).to_string().as_str()),
            "投影的轨道数 = 工程轨道数 - 主总线: {view}"
        );
        assert_eq!(
            field(view, "notes").as_deref(),
            Some(expected_notes.to_string().as_str()),
            "界面投影的音符数必须与工程同口径: {view}"
        );
        assert_eq!(
            field(view, "master").as_deref(),
            Some("1"),
            "主总线必须出现在投影里: {view}"
        );

        // 无头握手行必须**恰好**是那个字面值，且只出现一次。
        assert_eq!(
            lines
                .iter()
                .filter(|line| *line == HEADLESS_HANDSHAKE)
                .count(),
            1
        );
        assert!(text.contains("未构造 MainWindow"), "边界声明必须出现");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 32: 坏输入 ⇒ 精确原因 + 退出码 3（不是 panic、不是空工程）
    // ------------------------------------------------------------------

    #[test]
    fn broken_inputs_fail_with_exit_code_three_and_a_precise_reason() {
        let dir = scratch_dir("broken");
        let (path, _) = write_container_file(&dir, "good.yeban");
        let bytes = std::fs::read(&path).expect("读容器");

        let truncated = dir.join("truncated.yeban");
        std::fs::write(&truncated, &bytes[..bytes.len() / 2]).expect("写截断容器");
        let error = run_batch(
            &parse(&["--open".to_owned(), truncated.display().to_string()]).expect("解析"),
        )
        .expect_err("截断容器必须失败");
        assert_eq!(error.exit_code(), EXIT_OPEN);
        let text = error.to_string();
        assert!(text.contains("容器被拒绝"), "必须转达容器裁决: {text}");
        assert!(
            text.contains("truncated") || text.contains("end-of-central-directory"),
            "必须是**精确**的容器原因: {text}"
        );

        // 非容器文件：明确拒绝，理由精确（不是"未知格式"、不是空工程）。
        let junk = dir.join("junk.bin");
        std::fs::write(&junk, b"not a zip").expect("写垃圾");
        let error =
            run_batch(&parse(&["--open".to_owned(), junk.display().to_string()]).expect("解析"))
                .expect_err("垃圾文件必须失败");
        assert_eq!(error.exit_code(), EXIT_OPEN);
        assert!(
            error.to_string().contains("不是 `.yeban` 容器"),
            "实测: {error}"
        );

        // 不存在的文件 / 目录：都是精确的 I/O 裁决，都不是 panic。
        let missing = dir.join("nope.yeban");
        for bad in [missing.clone(), dir.clone()] {
            let error =
                run_batch(&parse(&["--open".to_owned(), bad.display().to_string()]).expect("解析"))
                    .expect_err("必须失败");
            assert_eq!(error.exit_code(), EXIT_OPEN);
            assert!(
                error.to_string().contains("无法读取"),
                "实测: {error} (path={})",
                bad.display()
            );
        }

        // 头号失败模式：错误**不得**退化成空工程 —— 成功路径与失败路径必须互斥。
        assert!(
            run_batch(&parse(&["--open".to_owned(), truncated.display().to_string()]).unwrap())
                .is_err()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 33: --save-as 写出的文件能读回，且**归档等价**（含资产池与 history.dag）
    // ------------------------------------------------------------------

    #[test]
    fn save_as_round_trips_through_the_public_open_entry() {
        let dir = scratch_dir("roundtrip");
        let (source, project) = write_container_file(&dir, "in.yeban");
        let target = dir.join("out.yeban");

        let lines = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--save-as".to_owned(),
                target.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("保存必须成功");
        let text = joined(&lines);
        let saved = lines
            .iter()
            .find(|line| line.starts_with("saved:"))
            .expect("必须有 saved: 行");
        let bytes = std::fs::metadata(&target).expect("文件必须真的在").len();
        assert_eq!(
            field(saved, "bytes").as_deref(),
            Some(bytes.to_string().as_str()),
            "打印的字节数必须是实际落盘字节数: {saved}"
        );
        assert!(
            text.contains(&format!("history-bytes={}", b"cli-history-dag".len())),
            "history.dag 必须被保真写出\n{text}"
        );
        assert!(saved.contains("assets=1"), "资产池不得静默丢失: {saved}");
        assert!(
            saved.contains("from=") && saved.contains("in.yeban"),
            "saved: 行必须说清来源: {saved}"
        );

        // 公开入口读回：工程等价 + 归档等价（**不是**只比 project.json）。
        let read_back =
            crate::open::open_project_archive_file(&target, &ProjectOpenOptions::default())
                .expect("保存出的文件必须能被公开入口读回");
        assert_eq!(read_back.project, project);
        let before =
            crate::open::open_project_archive_file(&source, &ProjectOpenOptions::default())
                .expect("源容器");
        assert_eq!(read_back, before, "另存为必须逐字段保真");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 34: 只读目录下 --save-as ⇒ 退出码 4，且**原文件一字未改**
    // ------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn save_as_into_a_read_only_directory_fails_and_leaves_the_old_file_intact() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = scratch_dir("readonly");
        let (source, _) = write_container_file(&dir, "source.yeban");
        let target = dir.join("target.yeban");
        crate::save::save_archive_file(
            &crate::open::open_project_archive_file(&source, &ProjectOpenOptions::default())
                .expect("源归档"),
            &target,
        )
        .expect("先放一个真容器");
        let before = std::fs::read(&target).expect("读原文");

        let mut permissions = std::fs::metadata(&dir).expect("目录元数据").permissions();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&dir, permissions).expect("降权");

        let probe = dir.join(".probe");
        let writable = std::fs::write(&probe, b"x").is_ok();
        let _ = std::fs::remove_file(&probe);

        if writable {
            eprintln!("[yeban-app/cli] 只读目录仍可写 (特权进程?), 本条判据无从判定 —— 响亮跳过");
        } else {
            let error = run_batch(
                &parse(&[
                    "--open".to_owned(),
                    source.display().to_string(),
                    "--save-as".to_owned(),
                    target.display().to_string(),
                ])
                .expect("解析"),
            )
            .expect_err("只读目录必须失败");
            assert_eq!(error.exit_code(), EXIT_SAVE);
            let text = error.to_string();
            assert!(
                text.contains("保存到") && text.contains("失败"),
                "必须说清是保存失败: {text}"
            );
            assert_eq!(
                std::fs::read(&target).expect("旧文件必须还在"),
                before,
                "失败的保存绝不能破坏旧文件 (这就是原子替换的意义)"
            );
        }

        let mut permissions = std::fs::metadata(&dir).expect("目录元数据").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&dir, permissions).expect("还原权限");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 35: 同一输入两次 --save-as ⇒ 字节完全相同（确定性）
    // ------------------------------------------------------------------

    #[test]
    fn two_save_as_runs_of_the_same_input_are_byte_identical() {
        let dir = scratch_dir("determinism");
        let (source, _) = write_container_file(&dir, "src.yeban");
        let first = dir.join("a.yeban");
        let second = dir.join("b.yeban");

        let mut printed = Vec::new();
        for target in [&first, &second] {
            let lines = run_batch(
                &parse(&[
                    "--open".to_owned(),
                    source.display().to_string(),
                    "--save-as".to_owned(),
                    target.display().to_string(),
                ])
                .expect("解析"),
            )
            .expect("保存必须成功");
            let saved = lines
                .iter()
                .find(|line| line.starts_with("saved:"))
                .expect("saved:");
            printed.push(field(saved, "bytes").expect("bytes="));
            // 临时文件名带 ULID ⇒ 两次**必然不同**（它是"真的建了临时文件"的痕迹）。
            assert!(
                saved.contains(crate::save::TEMP_INFIX),
                "必须用临时文件: {saved}"
            );
        }

        assert_eq!(printed[0], printed[1], "两次保存的字节数必须相同");
        assert_eq!(
            std::fs::read(&first).expect("读 a"),
            std::fs::read(&second).expect("读 b"),
            "同一输入两次 --save-as 的字节必须完全相同 (容器保证 stored + BTreeMap 顺序)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 36: 未知开关 / 缺取值 / 重复 ⇒ 退出码 2 + 用法提示（不许静默忽略）
    // ------------------------------------------------------------------

    #[test]
    fn unknown_and_malformed_arguments_are_usage_errors() {
        for args in [
            vec!["--bogus"],
            vec!["filled"],
            vec!["--open"],
            vec!["--save-as"],
            vec!["--open", "--headless"],
            vec!["--headless=1"],
            vec!["--project-sample", "nope"],
            vec!["--open", "a", "--open", "b"],
            vec!["--save-as", "a", "--save-as", "b"],
            vec!["--export-elements", "a", "--export-elements", "b"],
            vec!["--export-midi"],
            vec!["--export-midi", "a", "--export-midi", "b"],
            vec!["--open="],
        ] {
            let owned: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            let error = parse(&owned).expect_err(&format!("必须拒绝 {args:?}"));
            assert_eq!(error.exit_code(), EXIT_USAGE, "{args:?} ⇒ {error}");
            assert!(!error.to_string().is_empty(), "错误必须有可读原因");
        }

        // 未知开关的具体诊断（`main.rs` 会在它后面补上用法文本）。
        let error = parse(&["--bogus".to_owned()]).expect_err("未知开关");
        assert_eq!(
            error,
            ParseError::UnknownArgument("--bogus".to_owned()),
            "必须点名那个参数"
        );
        // 用法文本真的有内容可提示（未知开关的名字当然不在里面）。
        assert!(usage_text().contains("--save-as"));
        assert!(!usage_text().contains("无法识别的参数"));
    }

    // ------------------------------------------------------------------
    // 判据 37: --save-as 不带 --open ⇒ 演示工程，且输出**明说**这件事
    // ------------------------------------------------------------------

    #[test]
    fn save_as_without_open_saves_the_demo_project_and_says_so() {
        let dir = scratch_dir("sample");
        let target = dir.join("demo.yeban");

        let lines = run_batch(
            &parse(&["--save-as".to_owned(), target.display().to_string()]).expect("解析"),
        )
        .expect("保存演示工程");
        let text = joined(&lines);

        assert!(
            text.contains("project-source: sample=default"),
            "必须明写工程来自内置样本\n{text}"
        );
        assert!(
            text.contains("未读任何文件"),
            "必须明写没有读任何文件\n{text}"
        );
        let saved = lines
            .iter()
            .find(|line| line.starts_with("saved:"))
            .expect("saved:");
        assert!(
            saved.contains("from=sample=default") && saved.contains("不是从文件打开的"),
            "saved: 行必须再次说清来源: {saved}"
        );
        assert!(
            !text.contains("view-counts:"),
            "--save-as 不需要投影 ⇒ 不该有 view-counts（省掉与保存无关的失败面）\n{text}"
        );

        // 存下来的确实是演示工程。
        let read_back = crate::open::open_project_file(&target).expect("读回");
        assert_eq!(read_back, crate::bridge::demo_project());

        // `--project-sample filled` 换工程 ⇒ 换内容（同一条路径，没有第二个分支）。
        let filled = dir.join("filled.yeban");
        run_batch(
            &parse(&[
                "--project-sample".to_owned(),
                "filled".to_owned(),
                "--save-as".to_owned(),
                filled.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("保存 filled 样本");
        let filled_project = crate::open::open_project_file(&filled).expect("读回");
        assert_eq!(filled_project, yeban_model::samples::filled_project());
        assert_ne!(filled_project, read_back);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 37b: `--project-sample empty` = 真的 0 轨空工程（BASELINE-002 的取样对象）
    // ------------------------------------------------------------------

    /// `BASELINE-002` 判据句是"**空**工程空闲常驻内存 ≤ 35 MB"。本判据钉住两件事：
    ///
    /// ① `empty` 样本**真的是**空工程（0 轨 —— 不是"某个更小的演示工程"）；
    /// ② 无头路径**真的把它打印出来**（`project-counts:` 的轨道数是见证），
    ///    且与 `default` 的读数**不同**（否则就是空转，读数函数返回常数也能"绿"）。
    #[test]
    fn empty_sample_is_a_zero_track_project_with_witness() {
        let empty = Sample::Empty.project();
        assert_eq!(Sample::Empty.name(), "empty", "命令行取值必须是 `empty`");
        assert_eq!(
            empty,
            YebanProjectV1::default(),
            "empty 必须就是模型侧的规范空工程样本（不是另一份手写夹具）"
        );
        assert!(empty.tracks.is_empty(), "empty 的轨道集合必须为空");
        assert!(
            empty.master_bus_track_id.is_nil(),
            "0 轨工程没有主总线身份（validate 的 0 轨分支要求它是 nil）"
        );
        assert!(
            empty.clip_pool.is_empty() && empty.scenes.is_empty() && empty.sections.is_empty(),
            "空工程不许夹带片段 / 场景 / 段落"
        );
        empty
            .validate()
            .expect("0 轨空工程必须通过模型层校验（合法且可读）");

        // 对照：`default` = 6 条普通轨 + 主总线（`tracks-all` **含**主总线 ⇒ 7）。
        let demo = Sample::Default.project();
        assert_eq!(demo.tracks.len(), 7, "演示夹具的轨道数变了 ⇒ 本判据需同步");
        assert_eq!(demo.tracks.len() - 1, 6, "default 的普通轨必须是 6 条");

        // 见证：同一条无头命令分别打两个样本的读数。
        let report = |sample: &str| {
            let options = parse(&[
                "--project-sample".to_owned(),
                sample.to_owned(),
                "--headless".to_owned(),
            ])
            .expect("解析");
            joined(&run_batch(&options).expect("无头自检必须成功"))
        };
        let empty_text = report("empty");
        assert!(
            empty_text.contains("project-counts: tracks-all=0 master-track=0"),
            "empty 的轨道数见证必须是 0（tracks-all 含主总线 ⇒ 普通轨 = 0）:\n{empty_text}"
        );
        assert!(
            empty_text.contains("project-source: sample=empty (内置空工程 0 轨"),
            "来源行必须如实说明这是空工程（不许写成演示工程）:\n{empty_text}"
        );
        assert!(
            empty_text.contains("view-counts: tracks=0 master=0"),
            "0 轨空工程必须**能**被投影（不是被特判跳过、也不是投影失败被吞掉）:\n{empty_text}"
        );

        let demo_text = report("default");
        assert!(
            demo_text.contains("project-counts: tracks-all=7 master-track=1"),
            "default 的对照读数必须是 7（6 普通 + 1 主总线）:\n{demo_text}"
        );
        assert_ne!(
            empty_text, demo_text,
            "两个样本必须给出不同的读数（相同 ⇒ 模式没生效，见证是空转）"
        );
    }

    // ------------------------------------------------------------------
    // 判据 38: --export-elements 与 --dump-elements 同源；失败 ⇒ 不写工程（退出 5）
    // ------------------------------------------------------------------

    #[test]
    fn export_elements_writes_the_same_lines_and_blocks_a_failed_save() {
        let dir = scratch_dir("export");
        let (source, _) = write_container_file(&dir, "src.yeban");
        let target = dir.join("elements.txt");
        let saved = dir.join("out.yeban");

        let lines = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--dump-elements".to_owned(),
                "--export-elements".to_owned(),
                target.display().to_string(),
                "--save-as".to_owned(),
                saved.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("导出 + 保存");
        let text = joined(&lines);

        // 落盘的内容 = 打到 stdout 的那一串元素行（同源，不是第二份生成器）。
        let dumped: Vec<&String> = lines
            .iter()
            .filter(|line| line.starts_with("element "))
            .collect();
        assert!(!dumped.is_empty(), "必须有元素行\n{text}");
        let file = std::fs::read_to_string(&target).expect("读导出文件");
        assert_eq!(file.lines().count(), dumped.len(), "行数必须一致");
        for line in &dumped {
            assert!(file.contains(line.as_str()), "导出文件必须含 `{line}`");
        }
        let exported = lines
            .iter()
            .find(|line| line.starts_with("exported:"))
            .expect("exported:");
        assert_eq!(
            field(exported, "bytes").as_deref(),
            Some(file.len().to_string().as_str()),
            "导出的字节数必须是实际落盘字节数: {exported}"
        );
        assert!(saved.exists(), "两个都给时保存也要发生");

        // 导出失败 ⇒ **不**写工程（顺序是契约: 先导出, 再保存）。
        // 目标父目录不存在 ⇒ 同目录临时文件建不出来 ⇒ 导出必须失败。
        let blocked = dir.join("blocked").join("elements.txt");
        let lines = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--export-elements".to_owned(),
                blocked.display().to_string(),
                "--save-as".to_owned(),
                saved.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect_err("目录不存在 ⇒ 导出必须失败");
        assert_eq!(lines.exit_code(), EXIT_EXPORT);
        assert!(lines.to_string().contains("导出元素清单"), "{lines}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 39（**反转**）: --open **拒绝**裸 project.json（容器是唯一格式, D43）
    // ------------------------------------------------------------------

    #[test]
    fn open_rejects_a_bare_project_json_document_with_a_precise_reason() {
        use yeban_model::container::read_container;

        let dir = scratch_dir("bare");
        let (container, project) = write_container_file(&dir, "src.yeban");
        let bytes = std::fs::read(&container).expect("读容器");
        let json = read_container(&bytes, &ContainerLimits::default())
            .expect("真容器")
            .get("project.json")
            .expect("必有 project.json")
            .data
            .clone();
        let bare = dir.join("project.json");
        std::fs::write(&bare, &json).expect("写裸 JSON");

        // 旧判据在这里断言"能打开 + format=project-json"。反转之后它必须**失败**。
        let error = run_batch(
            &parse(&[
                "--open".to_owned(),
                bare.display().to_string(),
                "--headless".to_owned(),
            ])
            .expect("解析"),
        )
        .expect_err("裸 project.json 必须被明确拒绝");
        assert_eq!(error.exit_code(), EXIT_OPEN);
        let text = error.to_string();
        assert!(text.contains("不是 `.yeban` 容器"), "实测: {text}");
        assert!(
            text.contains("end-of-central-directory"),
            "必须带上容器的精确原裁决: {text}"
        );

        // 报告里 `format=` 只可能是一个值（容器）；用法文本里不再有裸 JSON 读法。
        assert_eq!(DOCUMENT_FORMAT, "yeban-container");
        let usage = usage_text();
        assert!(
            !usage.contains("裸") && !usage.contains("project-json"),
            "用法不得再提裸 JSON 读法:\n{usage}"
        );

        // 内容本身没问题：同一份 JSON 在真容器里就能打开（拒绝的是容器边界）。
        let wrapped = dir.join("wrapped.yeban");
        run_batch(
            &parse(&[
                "--open".to_owned(),
                container.display().to_string(),
                "--save-as".to_owned(),
                wrapped.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("容器必须能打开并另存");
        assert_eq!(
            crate::open::open_project_file(&wrapped).expect("读回"),
            project
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 40: 组合语义表（`--headless` / `--open` / `--save-as` 的每一种组合）
    // ------------------------------------------------------------------

    #[test]
    fn combination_semantics_are_exactly_the_documented_table() {
        let base = Options::default();
        assert!(base.wants_gui(), "无参数 = GUI");
        assert!(!base.needs_projection());

        let cases: [(Options, bool, bool); 12] = [
            (
                Options {
                    headless: true,
                    ..base.clone()
                },
                false,
                true,
            ),
            (
                Options {
                    open: Some(PathBuf::from("a.yeban")),
                    ..base.clone()
                },
                true,
                false,
            ),
            (
                Options {
                    open: Some(PathBuf::from("a.yeban")),
                    headless: true,
                    ..base.clone()
                },
                false,
                true,
            ),
            (
                Options {
                    save_as: Some(PathBuf::from("b.yeban")),
                    ..base.clone()
                },
                false,
                false,
            ),
            (
                Options {
                    open: Some(PathBuf::from("a.yeban")),
                    save_as: Some(PathBuf::from("b.yeban")),
                    ..base.clone()
                },
                false,
                false,
            ),
            (
                Options {
                    dump_elements: true,
                    ..base.clone()
                },
                false,
                true,
            ),
            (
                Options {
                    export_elements: Some(PathBuf::from("e.txt")),
                    save_as: Some(PathBuf::from("b.yeban")),
                    ..base.clone()
                },
                false,
                true,
            ),
            (
                Options {
                    print_shortcuts: true,
                    ..base.clone()
                },
                false,
                true,
            ),
            (
                // MIDI 导出**不需要**界面投影（与 `--save-as` 同族）：只依赖 `YebanProjectV1`。
                Options {
                    export_midi: Some(PathBuf::from("m.mid")),
                    ..base.clone()
                },
                false,
                false,
            ),
            (
                // `.als` 导出与 `--export-midi` **同族**：无窗口、不需要投影。
                Options {
                    export_als: Some(PathBuf::from("a.als")),
                    ..base.clone()
                },
                false,
                false,
            ),
            (
                // `.logicx` 导出与 `--export-als` **同族**：无窗口、不需要投影。
                Options {
                    export_logic: Some(PathBuf::from("out/Song.logicx")),
                    ..base.clone()
                },
                false,
                false,
            ),
            (
                // `--headless-idle` = 无窗口 + **需要**投影（见证行里有 `elements=`）。
                Options {
                    headless_idle: true,
                    idle_seconds: Some(2),
                    ..base.clone()
                },
                false,
                true,
            ),
        ];
        for (options, gui, projection) in cases {
            assert_eq!(options.wants_gui(), gui, "{options:?}");
            assert_eq!(options.needs_projection(), projection, "{options:?}");
        }
    }

    // ------------------------------------------------------------------
    // 判据 43: --headless-idle / --idle-seconds 的**成对性**与**不许静默丢参数**
    //           (含 ① 合法组合被解析 / ② 只给一个 = 用法错误 / ③ 值域护栏 /
    //            ④ 与写盘开关同给 = 用法错误而不是静默忽略)
    // ------------------------------------------------------------------

    #[test]
    fn headless_idle_is_paired_with_idle_seconds_and_never_silently_ignores_an_option() {
        let args = |raw: &[&str]| {
            raw.iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };

        // ① 合法组合（本线的取样形态：空工程 + 2 秒空闲）。
        let parsed = parse(&args(&[
            "--headless-idle",
            "--idle-seconds",
            "2",
            "--project-sample",
            "empty",
        ]))
        .expect("合法组合必须被接受");
        assert!(parsed.headless_idle);
        assert_eq!(parsed.idle_seconds, Some(2));
        assert_eq!(parsed.sample, Sample::Empty);
        assert!(parsed.batch() && !parsed.wants_gui(), "{parsed:?}");
        assert!(parsed.needs_projection(), "见证行里有 elements= ⇒ 必须投影");

        // ② 成对性：只给一个 = 用法错误（不设默认空闲时长）。
        assert_eq!(
            parse(&args(&["--headless-idle"])),
            Err(ParseError::IdleSecondsMissing)
        );
        assert_eq!(
            parse(&args(&["--idle-seconds", "2"])),
            Err(ParseError::IdleSecondsWithoutHeadlessIdle)
        );
        // 取值写法两种都要认（`--opt value` / `--opt=value`）。
        assert_eq!(
            parse(&args(&["--headless-idle", "--idle-seconds=3"]))
                .expect("内联取值")
                .idle_seconds,
            Some(3)
        );
        // 不接受取值的开关写成 `--flag=value` 仍是错误。
        assert_eq!(
            parse(&args(&["--headless-idle=1", "--idle-seconds", "2"])),
            Err(ParseError::UnexpectedValue("--headless-idle"))
        );

        // ③ 值域：0 / 超上限 / 小数 / 非数字 / 负数都必须是用法错误。
        for bad in ["0", "61", "1.5", "abc", "-1", ""] {
            assert_eq!(
                parse(&args(&["--headless-idle", "--idle-seconds", bad])),
                Err(ParseError::IdleSecondsInvalid(bad.to_owned())),
                "`--idle-seconds {bad}` 必须被拒"
            );
        }
        // 上限本身合法（护栏只挡笔误）。
        assert_eq!(
            parse(&args(&["--headless-idle", "--idle-seconds", "60"]))
                .expect("上限合法")
                .idle_seconds,
            Some(MAX_IDLE_SECONDS)
        );

        // ④ 与写盘 / 导出开关同给 = 用法错误（否则它们会被**静默**丢掉）。
        for (extra, name) in [
            (vec!["--dump-elements"], "--dump-elements"),
            (vec!["--print-shortcuts"], "--print-shortcuts"),
            (vec!["--export-elements", "e.txt"], "--export-elements"),
            (vec!["--export-midi", "m.mid"], "--export-midi"),
            (vec!["--export-als", "a.als"], ALS_EXPORT_SWITCH),
            (
                vec!["--export-logic", "out/Song.logicx"],
                LOGIC_EXPORT_SWITCH,
            ),
            (vec!["--save-as", "b.yeban"], "--save-as"),
        ] {
            let mut raw = vec!["--headless-idle", "--idle-seconds", "1"];
            raw.extend(extra.iter().copied());
            assert_eq!(
                parse(&args(&raw)),
                Err(ParseError::IdleConflict(name)),
                "{raw:?} 必须被拒"
            );
        }
        // `--headless` 同时给是**允许**的（它是更弱的形态，报告行完全一样）。
        let both = parse(&args(&[
            "--headless",
            "--headless-idle",
            "--idle-seconds",
            "1",
        ]))
        .expect("--headless 可以与 --headless-idle 同给");
        assert!(both.headless && both.headless_idle);

        // ⑤ 防假绿：`--headless-idle` 绝不许走零 Slint 的 run_batch。
        assert!(matches!(
            run_batch(&parsed),
            Err(CliError::HeadlessIdleNotBatch)
        ));
        assert_eq!(CliError::HeadlessIdleNotBatch.exit_code(), EXIT_UI);
    }

    // ------------------------------------------------------------------
    // 判据 46: --enable-mcp-http 的两条前置 + 防假绿守卫 [ROAD-M4-001]
    //           (含 ① 字面值是契约 / ② 编译期那道开关 / ③ 不接受取值 /
    //            ④ 与无窗口开关同给 = 用法错误 / ⑤ 绕过 parse 也被 run_batch 拒)
    // ------------------------------------------------------------------

    #[test]
    fn the_mcp_http_switch_is_opt_in_refuses_non_gui_paths_and_never_lies() {
        let args = |raw: &[&str]| {
            raw.iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };

        // ① 字面值与环境变量名是**契约**（另一个 crate 的 tests/in_process_mcp.rs 与
        //    yeban-mcp 的 ENABLE_HTTP_FLAG 也对这两个字面值对账）。
        assert_eq!(MCP_HTTP_SWITCH, "--enable-mcp-http");
        assert_eq!(MCP_HTTP_ENV, "YEBAN_MCP_HTTP");

        // ② 编译期那道开关：默认构建里这个开关**不存在**，给了就是用法错误
        //    （不许静默忽略 —— 那会让人以为网络控制面开了）。
        if cfg!(feature = "in-process-mcp") {
            let parsed = parse(&args(&["--enable-mcp-http"])).expect("带 feature 时必须被接受");
            assert!(parsed.enable_mcp_http);
            // 它**不**把进程推离 GUI 路径：形态 A 的定义就是"挂在正在跑的 app 进程里"。
            assert!(parsed.wants_gui() && !parsed.batch(), "{parsed:?}");
        } else {
            let error = parse(&args(&["--enable-mcp-http"])).expect_err("默认构建里必须被拒");
            assert_eq!(error, ParseError::McpHttpNotCompiled);
            assert!(
                error.to_string().contains("in-process-mcp"),
                "错误必须点名缺的是哪个 feature: {error}"
            );
        }

        // ③ 与其它"没有取值的开关"同款：`--flag=value` 是错误。
        assert_eq!(
            parse(&args(&["--enable-mcp-http=1"])),
            Err(ParseError::UnexpectedValue(MCP_HTTP_SWITCH))
        );

        // ④ 与任一"无窗口开关"同给：带 feature 时是 McpHttpNeedsGui（控制面要挂在正在跑的
        //    GUI 进程里）；不带 feature 时更早一步就被"这个构建里没有它"挡住。
        //    两种都必须是**错误** —— 绝不静默丢参数。
        for (flag, name) in [
            (vec!["--headless"], "--headless"),
            (vec!["--dump-elements"], "--dump-elements"),
            (vec!["--print-shortcuts"], "--print-shortcuts"),
            (vec!["--export-elements", "e.txt"], "--export-elements"),
            (vec!["--export-midi", "m.mid"], "--export-midi"),
            (vec!["--export-als", "a.als"], ALS_EXPORT_SWITCH),
            (vec!["--save-as", "b.yeban"], "--save-as"),
            (
                vec!["--headless-idle", "--idle-seconds", "1"],
                "--headless-idle",
            ),
        ] {
            let mut raw = vec!["--enable-mcp-http"];
            raw.extend(flag.iter().copied());
            let expected = if cfg!(feature = "in-process-mcp") {
                ParseError::McpHttpNeedsGui(name)
            } else {
                ParseError::McpHttpNotCompiled
            };
            assert_eq!(parse(&args(&raw)), Err(expected), "{raw:?} 必须被拒");
        }

        // ⑤ 防假绿第二道：`Options` 被**直接构造**（绕过 parse）时，`run_batch` 也必须拒绝 ——
        //    批处理路径跑完就退出，在那里挂控制面等于刚绑好就拆掉。
        let smuggled = Options {
            enable_mcp_http: true,
            ..Options::default()
        };
        assert!(matches!(
            run_batch(&smuggled),
            Err(CliError::McpHttpNotBatch)
        ));
        assert_eq!(CliError::McpHttpNotBatch.exit_code(), EXIT_UI);
    }

    // ------------------------------------------------------------------
    // 判据 47: --export-als 的编译期开关 + 损失表呈现 [ARCH-FMT-002] [ROAD-M4-007]
    //           (含 ① 字面值/feature 名是契约 / ② 默认构建点名 feature 的用法错误 /
    //            ③ 取值写法与只能给一次 / ④ 绕过 parse 也被 run_batch 拒 /
    //            ⑤ 损失表逐条可见 + 超长**礼貌截断**并写明还剩几条)
    // ------------------------------------------------------------------

    #[test]
    fn the_als_export_switch_is_opt_in_and_the_loss_table_is_never_silently_dropped() {
        let args = |raw: &[&str]| {
            raw.iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };

        // ① 字面值与 feature 名是**契约**（`--help`、用法错误、守卫的白名单都在引用它们）。
        assert_eq!(ALS_EXPORT_SWITCH, "--export-als");
        assert_eq!(ALS_EXPORT_FEATURE, "experimental-als-export");
        // 它必须与 `--export-midi` **同族**：无窗口、不需要投影（`D47` 的一条出口两种产物）。
        let model = Options {
            export_als: Some(PathBuf::from("a.als")),
            ..Options::default()
        };
        assert!(model.batch() && !model.wants_gui(), "{model:?}");
        assert!(
            !model.needs_projection(),
            "导出只依赖 YebanProjectV1: {model:?}"
        );

        // ② 编译期那道开关：默认构建里这个开关**不存在**，给了就是点名 feature 的用法错误。
        if cfg!(feature = "experimental-als-export") {
            let parsed = parse(&args(&["--export-als", "a.als"])).expect("带 feature 时必须被接受");
            assert_eq!(parsed.export_als, Some(PathBuf::from("a.als")));
            assert!(parsed.batch() && !parsed.wants_gui(), "{parsed:?}");
        } else {
            let error = parse(&args(&["--export-als", "a.als"])).expect_err("默认构建里必须被拒");
            assert_eq!(error, ParseError::AlsNotCompiled);
            assert_eq!(error.exit_code(), EXIT_USAGE);
            let text = error.to_string();
            assert!(text.contains(ALS_EXPORT_SWITCH), "必须点名开关: {text}");
            assert!(
                text.contains(ALS_EXPORT_FEATURE),
                "必须点名 feature: {text}"
            );
        }

        // ③ `--opt value` 与 `--opt=value` 都认；缺取值 / 重复给与其它带取值开关同款。
        //    （两条都在编译期检查**之前**判 —— 缺值/重复与"这个构建里有没有它"无关。）
        assert_eq!(
            parse(&args(&["--export-als"])),
            Err(ParseError::MissingValue(ALS_EXPORT_SWITCH))
        );
        assert_eq!(
            parse(&args(&["--export-als", "a.als", "--export-als", "b.als"])),
            Err(ParseError::DuplicateOption(ALS_EXPORT_SWITCH))
        );
        #[cfg(feature = "experimental-als-export")]
        assert_eq!(
            parse(&args(&["--export-als=b.als"]))
                .expect("内联取值必须被接受")
                .export_als,
            Some(PathBuf::from("b.als"))
        );

        // ④ 防假绿第二道：`Options` 被**直接构造**（绕过 parse）时, 默认构建的 `run_batch`
        //    也必须拒绝 —— 静默跳过 = 用户以为 `.als` 写了而磁盘上什么都没有。
        #[cfg(not(feature = "experimental-als-export"))]
        {
            let smuggled = Options {
                export_als: Some(PathBuf::from("a.als")),
                ..Options::default()
            };
            let error = run_batch(&smuggled).expect_err("默认构建里必须被拒");
            assert!(matches!(error, CliError::AlsNotCompiled), "{error:?}");
            assert_eq!(error.exit_code(), EXIT_USAGE);
            let text = error.to_string();
            assert!(text.contains(ALS_EXPORT_SWITCH), "{text}");
            assert!(text.contains(ALS_EXPORT_FEATURE), "{text}");
        }
    }

    /// 判据 48: 损失表呈现 —— 逐条可见、字段可判、超长时**礼貌截断**且写明还剩几条。
    ///
    /// 这条判据只用**合成**的损失表（不落盘、不依赖工程），因此它测的是呈现逻辑本身：
    /// `D47` 说"把损失报告写进日志"这件事若只是打印一个数字，那它就没有价值。
    #[cfg(feature = "experimental-als-export")]
    #[test]
    fn the_als_loss_report_lists_every_entry_and_truncates_politely() {
        use yeban_render::als::AlsLoss;

        let loss = |index: usize| AlsLoss {
            entity: format!("track:{index:04}"),
            reason: format!("未映射: 判据用合成条目 {index}"),
            bounced_to_audio: index.is_multiple_of(5),
        };

        // 少于上限：全部可见, 且没有截断行。
        let short: Vec<AlsLoss> = (0..MAX_ALS_LOSS_LINES - 1).map(loss).collect();
        let rendered = als_loss_lines(&short);
        assert_eq!(rendered[0], format!("als-losses: count={}", short.len()));
        assert_eq!(
            rendered
                .iter()
                .filter(|line| line.starts_with("als-loss: "))
                .count(),
            short.len(),
            "每一条都必须可见"
        );
        assert!(
            !rendered
                .iter()
                .any(|line| line.starts_with("als-losses-truncated:")),
            "没超上限就不该有截断行: {rendered:?}"
        );
        // 字段可判 + 人话被转义（与 `quoted()` 同一份实现）。
        let first = &rendered[1];
        assert_eq!(field(first, "entity").as_deref(), Some("track:0000"));
        assert_eq!(field(first, "bounced-to-audio").as_deref(), Some("true"));
        assert!(
            first.contains("reason=\"未映射: 判据用合成条目 0\""),
            "{first}"
        );

        // 超过上限：恰好显示上限条, 截断行**明写**还剩几条（不是静默吞掉）。
        let overflow = 3;
        let long: Vec<AlsLoss> = (0..MAX_ALS_LOSS_LINES + overflow).map(loss).collect();
        let rendered = als_loss_lines(&long);
        assert_eq!(rendered[0], format!("als-losses: count={}", long.len()));
        assert_eq!(
            rendered
                .iter()
                .filter(|line| line.starts_with("als-loss: "))
                .count(),
            MAX_ALS_LOSS_LINES,
            "显示条数必须恰好等于上限"
        );
        let tail = rendered.last().expect("有截断行");
        assert!(tail.starts_with("als-losses-truncated: "), "{tail}");
        assert!(tail.contains(&format!("and {overflow} more")), "{tail}");
        assert!(
            tail.contains("yeban-loss"),
            "截断行必须指出完整表在哪: {tail}"
        );
    }

    // ------------------------------------------------------------------
    // 判据 49: --export-logic 的编译期开关 + 损失表呈现 [ARCH-FMT-002] [ROAD-M4-007]
    //           （与判据 47 同族、另一个产物形态：bundle 目录而不是单文件）
    // ------------------------------------------------------------------

    #[test]
    fn the_logic_export_switch_is_opt_in_and_the_loss_table_is_never_silently_dropped() {
        let args = |raw: &[&str]| {
            raw.iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };

        // ① 字面值与 feature 名是**契约**。
        assert_eq!(LOGIC_EXPORT_SWITCH, "--export-logic");
        assert_eq!(LOGIC_EXPORT_FEATURE, "experimental-logic-export");
        let model = Options {
            export_logic: Some(PathBuf::from("out/Song.logicx")),
            ..Options::default()
        };
        assert!(model.batch() && !model.wants_gui(), "{model:?}");
        assert!(
            !model.needs_projection(),
            "导出只依赖 YebanProjectV1: {model:?}"
        );

        // ② 编译期那道开关：默认构建里这个开关**不存在**，给了就是点名 feature 的用法错误。
        if cfg!(feature = "experimental-logic-export") {
            let parsed = parse(&args(&["--export-logic", "out/Song.logicx"]))
                .expect("带 feature 时必须被接受");
            assert_eq!(parsed.export_logic, Some(PathBuf::from("out/Song.logicx")));
            assert!(parsed.batch() && !parsed.wants_gui(), "{parsed:?}");
        } else {
            let error = parse(&args(&["--export-logic", "out/Song.logicx"]))
                .expect_err("默认构建里必须被拒");
            assert_eq!(error, ParseError::LogicNotCompiled);
            assert_eq!(error.exit_code(), EXIT_USAGE);
            let text = error.to_string();
            assert!(text.contains(LOGIC_EXPORT_SWITCH), "必须点名开关: {text}");
            assert!(
                text.contains(LOGIC_EXPORT_FEATURE),
                "必须点名 feature: {text}"
            );
        }

        // ③ 取值写法与只能给一次。
        assert_eq!(
            parse(&args(&["--export-logic"])),
            Err(ParseError::MissingValue(LOGIC_EXPORT_SWITCH))
        );
        assert_eq!(
            parse(&args(&[
                "--export-logic",
                "a.logicx",
                "--export-logic",
                "b.logicx"
            ])),
            Err(ParseError::DuplicateOption(LOGIC_EXPORT_SWITCH))
        );
        #[cfg(feature = "experimental-logic-export")]
        assert_eq!(
            parse(&args(&["--export-logic=b.logicx"]))
                .expect("内联取值必须被接受")
                .export_logic,
            Some(PathBuf::from("b.logicx"))
        );

        // ④ 防假绿第二道：`Options` 被**直接构造**（绕过 parse）时, 默认构建的 `run_batch`
        //    也必须拒绝 —— 静默跳过 = 用户以为 bundle 写了而磁盘上什么都没有。
        #[cfg(not(feature = "experimental-logic-export"))]
        {
            let smuggled = Options {
                export_logic: Some(PathBuf::from("out/Song.logicx")),
                ..Options::default()
            };
            let error = run_batch(&smuggled).expect_err("默认构建里必须被拒");
            assert!(matches!(error, CliError::LogicNotCompiled), "{error:?}");
            assert_eq!(error.exit_code(), EXIT_USAGE);
            let text = error.to_string();
            assert!(text.contains(LOGIC_EXPORT_SWITCH), "{text}");
            assert!(text.contains(LOGIC_EXPORT_FEATURE), "{text}");
        }
    }

    /// 判据 50: `.logicx` 损失表呈现 —— 逐条可见、字段可判、超长时**礼貌截断**。
    ///
    /// 与判据 48 同款，只是没有 `.als` 的 `bounced-to-audio` 字段。
    #[cfg(feature = "experimental-logic-export")]
    #[test]
    fn the_logic_loss_report_lists_every_entry_and_truncates_politely() {
        use yeban_render::logic::LogicLoss;

        let loss = |index: usize| LogicLoss {
            entity: format!("track:{index:04}"),
            reason: format!("未映射: 判据用合成条目 {index}"),
        };

        let short: Vec<LogicLoss> = (0..MAX_LOGIC_LOSS_LINES - 1).map(loss).collect();
        let rendered = logic_loss_lines(&short);
        assert_eq!(rendered[0], format!("logic-losses: count={}", short.len()));
        assert_eq!(
            rendered
                .iter()
                .filter(|line| line.starts_with("logic-loss: "))
                .count(),
            short.len()
        );
        assert!(
            !rendered
                .iter()
                .any(|line| line.starts_with("logic-losses-truncated:")),
            "没超上限就不该有截断行: {rendered:?}"
        );
        let first = &rendered[1];
        assert_eq!(field(first, "entity").as_deref(), Some("track:0000"));
        assert!(
            first.contains("reason=\"未映射: 判据用合成条目 0\""),
            "{first}"
        );

        let overflow = 3;
        let long: Vec<LogicLoss> = (0..MAX_LOGIC_LOSS_LINES + overflow).map(loss).collect();
        let rendered = logic_loss_lines(&long);
        assert_eq!(rendered[0], format!("logic-losses: count={}", long.len()));
        assert_eq!(
            rendered
                .iter()
                .filter(|line| line.starts_with("logic-loss: "))
                .count(),
            MAX_LOGIC_LOSS_LINES
        );
        let tail = rendered.last().expect("有截断行");
        assert!(tail.starts_with("logic-losses-truncated: "), "{tail}");
        assert!(tail.contains(&format!("and {overflow} more")), "{tail}");
        assert!(
            tail.contains(yeban_render::logic::META_DATA_LOSS_KEY),
            "截断行必须指出完整表在哪: {tail}"
        );
    }

    // ------------------------------------------------------------------
    // 判据 44: 见证行 / 空闲行是**可判**的（字段齐全, 空转会被判红）
    // ------------------------------------------------------------------

    #[test]
    fn idle_witness_is_non_trivial_and_its_lines_are_machine_readable() {
        let witness = IdleWitness {
            windows_created: 1,
            width: 1920,
            height: 1080,
            rendered: true,
            lines: 1080,
            non_black_pixels: 500_000,
            distinct_colors: 42,
            elements: 99,
        };
        assert!(witness.is_non_trivial(), "{witness:?}");
        // 500_000 * 1000 / (1920 * 1080) = 241.1 ⇒ 241‰
        assert_eq!(witness.non_black_permille(), 241);

        let line = idle_witness_line(&witness);
        assert!(line.starts_with("headless-idle-witness:"));
        assert_eq!(field(&line, "windows-created").as_deref(), Some("1"));
        assert_eq!(field(&line, "size").as_deref(), Some("1920x1080"));
        assert_eq!(field(&line, "rendered").as_deref(), Some("true"));
        assert_eq!(field(&line, "lines").as_deref(), Some("1080"));
        assert_eq!(field(&line, "non-black-pixels").as_deref(), Some("500000"));
        assert_eq!(field(&line, "distinct-colors").as_deref(), Some("42"));
        // 见证行里的 elements= 与 view-counts 是同一份注册表读数 ⇒ 同一命令两模式可比。
        assert_eq!(field(&line, "elements").as_deref(), Some("99"));

        // 每一条"空转"都要被这条判据抓住（否则它只是个好看的字符串）。
        for degenerate in [
            IdleWitness {
                windows_created: 0,
                ..witness
            },
            IdleWitness {
                rendered: false,
                ..witness
            },
            IdleWitness {
                lines: 0,
                ..witness
            },
            IdleWitness {
                non_black_pixels: 0,
                ..witness
            },
            IdleWitness {
                height: 0,
                ..witness
            },
        ] {
            assert!(!degenerate.is_non_trivial(), "{degenerate:?} 必须判红");
        }

        let idle = idle_report_line(&IdleReport {
            seconds: 2,
            elapsed_ms: 2004,
            ticks: 200,
        });
        assert!(idle.starts_with("headless-idle-idle:"));
        assert_eq!(field(&idle, "seconds").as_deref(), Some("2"));
        assert_eq!(field(&idle, "elapsed-ms").as_deref(), Some("2004"));
        assert_eq!(field(&idle, "ticks").as_deref(), Some("200"));

        // 用法文本必须把这两个开关与它自己的握手行写出来。
        let usage = usage_text();
        for needle in ["--headless-idle", "--idle-seconds", HEADLESS_IDLE_HANDSHAKE] {
            assert!(usage.contains(needle), "用法文本缺少 `{needle}`");
        }
    }

    // ------------------------------------------------------------------
    // 判据 42: --export-midi 落盘的文件能被 SMF 读取面读回, 且逐音符与工程一致
    //           (含 ③ PPQ 头字段 / ④ 两次导出逐字节相同 / ① 回读 / ② 逐音符)
    // ------------------------------------------------------------------

    /// 演示夹具的 MIDI 事实（**独立**写下的期望值, 见 `export_midi.rs` 的同一组常量）。
    const DEMO_MIDI_NOTES: [(u8, u64); 6] = [
        (60, 0),
        (64, 480),
        (67, 960),
        (72, 1440),
        (74, 1920),
        (76, 2400),
    ];

    #[test]
    fn export_midi_writes_a_parseable_smf_whose_notes_match_the_project() {
        use yeban_render::midi::{MidiFormat, parse_smf, track_chunks};

        let dir = scratch_dir("export-midi");
        let (source, project) = write_container_file(&dir, "song.yeban");
        let target = dir.join("song.mid");

        let lines = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--export-midi".to_owned(),
                target.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("导出必须成功");
        let text = joined(&lines);
        let exported = lines
            .iter()
            .find(|line| line.starts_with("exported-midi:"))
            .expect("必须有 exported-midi: 行");

        // 报告行**自描述**: 落点 / 字节数 / PPQ / 格式 / 轨道 / 音符 / 来源。
        assert!(exported.contains("song.mid"), "{exported}");
        assert_eq!(field(exported, "ppq").as_deref(), Some("960"), "{exported}");
        assert_eq!(
            field(exported, "format").as_deref(),
            Some("parallel"),
            "{exported}"
        );
        assert_eq!(
            field(exported, "tracks").as_deref(),
            Some("1"),
            "{exported}"
        );
        assert_eq!(
            field(exported, "notes").as_deref(),
            Some("6"),
            "演示夹具的六颗音符\n{exported}"
        );
        assert_eq!(
            field(exported, "tempos").as_deref(),
            Some("1"),
            "{exported}"
        );
        assert!(
            exported.contains(crate::save::TEMP_INFIX),
            "必须用临时文件 (原子落盘): {exported}"
        );
        assert!(
            exported.contains("from=") && exported.contains("song.yeban"),
            "exported-midi: 行必须说清来源: {exported}"
        );

        let bytes = std::fs::read(&target).expect("文件必须真的在");
        assert_eq!(
            field(exported, "bytes").as_deref(),
            Some(bytes.len().to_string().as_str()),
            "打印的字节数必须是实际落盘字节数: {exported}"
        );

        // ① 字节能被 SMF 读取面读回; chunk 布局 = MThd + conductor + 一条音符轨。
        let chunks = track_chunks(&bytes).expect("chunk 布局");
        assert_eq!(&chunks[0].fourcc, b"MThd");
        assert_eq!(chunks.len(), 3);
        for chunk in &chunks[1..] {
            let payload = &bytes[chunk.payload.clone()];
            assert_eq!(payload[payload.len() - 3..], [0xFF, 0x2F, 0x00]);
        }
        let parsed = parse_smf(&bytes).expect("回读");
        assert_eq!(parsed.format, MidiFormat::Parallel);
        assert_eq!(parsed.ppq, 960);

        // ③ `MThd` 的时间分度字段（大端）= 960, 不是 480。
        assert_eq!(&bytes[12..14], &[0x03, 0xC0], "0x03C0 = 960");

        // ② 逐音符: (通道, 音高, 力度, 起始 tick, 时值) 与工程一致。
        let mut actual: Vec<(u8, u8, u8, u64, u64)> =
            parsed.notes.iter().map(|note| note.key()).collect();
        actual.sort_unstable();
        let mut expected: Vec<(u8, u8, u8, u64, u64)> = DEMO_MIDI_NOTES
            .iter()
            .map(|&(key, start)| (0, key, 100, start, 480))
            .collect();
        expected.sort_unstable();
        assert_eq!(actual, expected, "六颗音符逐项一致");

        // 工程侧的音符数（独立数一遍）与报告行一致。
        let model_notes: usize = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .map(std::collections::BTreeMap::len)
            .sum();
        assert_eq!(model_notes, 6, "演示夹具的音符数变了 ⇒ 本判据需同步");

        // ④ 同一工程两次导出 ⇒ 逐字节相同（`ARCH-DET-*` 口径）。
        let second = dir.join("song-again.mid");
        run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--export-midi".to_owned(),
                second.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("第二次导出必须成功");
        assert_eq!(
            std::fs::read(&target).expect("读第一次"),
            std::fs::read(&second).expect("读第二次"),
            "同一工程的两次导出必须逐字节相同"
        );
        assert!(
            text.contains("headless ok"),
            "无窗口路径必须打印握手行:\n{text}"
        );
        // 没有 `.tmp-` 残留（原子替换的痕迹）。
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(crate::save::TEMP_INFIX))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 43: 非容器输入 ⇒ 退出码 3（与 --open 同一语义）, 且**不写任何 MIDI 文件**
    // ------------------------------------------------------------------

    #[test]
    fn export_midi_on_a_non_container_exits_three_and_writes_nothing() {
        let dir = scratch_dir("export-midi-bad");
        let junk = dir.join("junk.bin");
        std::fs::write(&junk, b"not a zip at all").expect("写垃圾");
        let target = dir.join("never.mid");

        let error = run_batch(
            &parse(&[
                "--open".to_owned(),
                junk.display().to_string(),
                "--export-midi".to_owned(),
                target.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect_err("非容器输入必须失败");
        assert_eq!(error.exit_code(), EXIT_OPEN, "复用 --open 的退出码 3");
        assert!(
            error.to_string().contains("不是 `.yeban` 容器"),
            "必须转达容器的精确裁决: {error}"
        );
        assert!(!target.exists(), "打开失败 ⇒ 一个字节都不许写出去");

        // 空工程（没有 MIDI 内容）⇒ 复用导出失败那一档, 也不留文件。
        let error = crate::export_midi::export_project_to_file(
            &yeban_model::YebanProjectV1::default(),
            &target,
        )
        .expect_err("空工程必须被拒绝");
        assert!(
            error.to_string().contains("没有任何可导出的 MIDI 音符"),
            "空工程的理由必须精确: {error}"
        );
        assert!(!target.exists(), "失败不得留下目标文件");
        assert_eq!(
            CliError::ExportMidi {
                path: target.clone(),
                source: error
            }
            .exit_code(),
            EXIT_EXPORT
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 44: 目标路径不可写 ⇒ 退出码 5 + 精确原因 + **不留下半个文件**（原子性）
    // ------------------------------------------------------------------

    #[test]
    fn export_midi_into_an_unwritable_path_exits_five_without_a_half_file() {
        let dir = scratch_dir("export-midi-unwritable");
        let (source, _) = write_container_file(&dir, "src.yeban");
        let blocked = dir.join("missing").join("out.mid");

        let error = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--export-midi".to_owned(),
                blocked.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect_err("父目录不存在 ⇒ 必须失败");
        assert_eq!(error.exit_code(), EXIT_EXPORT);
        let text = error.to_string();
        assert!(
            text.contains("导出 MIDI 到") && text.contains("写临时文件"),
            "必须说清是导出 MIDI 失败以及精确的 I/O 动作: {text}"
        );
        assert!(!blocked.exists(), "失败不得留下目标文件");
        assert!(
            !dir.join("missing").exists(),
            "失败不得凭空造出目录（更不许留半个文件）"
        );
        // 目录里没有 `.tmp-` 残留。
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(crate::save::TEMP_INFIX))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 45: 组合语义：先元素、再 MIDI、最后保存；任一导出失败 ⇒ 不写工程
    // ------------------------------------------------------------------

    #[test]
    fn export_midi_runs_before_save_as_and_blocks_a_failed_save() {
        let dir = scratch_dir("export-midi-order");
        let (source, _) = write_container_file(&dir, "src.yeban");
        let midi = dir.join("out.mid");
        let saved = dir.join("out.yeban");

        let lines = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--export-elements".to_owned(),
                dir.join("elements.txt").display().to_string(),
                "--export-midi".to_owned(),
                midi.display().to_string(),
                "--save-as".to_owned(),
                saved.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("三条都该成功");
        let index = |prefix: &str| {
            lines
                .iter()
                .position(|line| line.starts_with(prefix))
                .unwrap_or_else(|| panic!("缺少 `{prefix}` 行:\n{}", joined(&lines)))
        };
        assert!(
            index("exported:") < index("exported-midi:")
                && index("exported-midi:") < index("saved:"),
            "顺序固定: 元素 → MIDI → 保存工程\n{}",
            joined(&lines)
        );
        assert!(midi.exists() && saved.exists(), "三条都真的落盘了");

        // MIDI 导出失败（父目录不存在）⇒ **不**写工程。
        let never = dir.join("never.yeban");
        let error = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--export-midi".to_owned(),
                dir.join("blocked").join("x.mid").display().to_string(),
                "--save-as".to_owned(),
                never.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect_err("MIDI 导出必须失败");
        assert_eq!(error.exit_code(), EXIT_EXPORT);
        assert!(!never.exists(), "MIDI 导出失败 ⇒ 不许写工程");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 46: --export-midi 不给 --open ⇒ 导出演示工程并**明说**来源
    // ------------------------------------------------------------------

    #[test]
    fn export_midi_without_open_exports_the_demo_project_and_says_so() {
        let dir = scratch_dir("export-midi-sample");
        let target = dir.join("demo.mid");
        let lines = run_batch(
            &parse(&["--export-midi".to_owned(), target.display().to_string()]).expect("解析"),
        )
        .expect("导出演示工程");
        let text = joined(&lines);
        assert!(
            text.contains("project-source: sample=default"),
            "必须明写工程来自内置样本\n{text}"
        );
        let exported = lines
            .iter()
            .find(|line| line.starts_with("exported-midi:"))
            .expect("exported-midi:");
        assert!(
            exported.contains("from=sample=default") && exported.contains("不是从文件打开的"),
            "exported-midi: 行必须再次说清来源: {exported}"
        );
        assert!(
            !text.contains("view-counts:"),
            "导出不需要投影 ⇒ 不该有 view-counts\n{text}"
        );
        assert_eq!(
            yeban_render::midi::parse_smf(&std::fs::read(&target).expect("读"))
                .expect("回读")
                .notes
                .len(),
            6
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 41: 报告格式的自描述性（引号转义 / 字段名）
    // ------------------------------------------------------------------

    #[test]
    fn report_lines_are_self_describing_and_escapable() {
        assert_eq!(quoted("夜半 Yeban"), "\"夜半 Yeban\"");
        assert_eq!(quoted("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(quoted("a\nb"), "\"a\\nb\"");

        let loaded = Loaded {
            archive: ProjectArchive {
                project: crate::bridge::demo_project(),
                history_dag: Vec::new(),
                assets: Vec::new(),
            },
            source: ProjectSource::Sample(Sample::Default),
        };
        let report = joined(&project_report(&loaded));
        assert!(report.contains("title=\"夜半 Yeban\""), "{report}");
        // 样本形态的报告**没有** opened: 行（来源由 project-source: 说清）。
        assert!(!report.contains("opened:"), "{report}");

        let loaded_file = Loaded {
            source: ProjectSource::File {
                path: PathBuf::from("/tmp/x.yeban"),
                bytes: 42,
            },
            ..loaded
        };
        let report = joined(&project_report(&loaded_file));
        assert!(
            report.contains("opened: path=/tmp/x.yeban bytes=42 format=yeban-container"),
            "{report}"
        );
        assert!(!report.contains("未读任何文件"), "{report}");
        assert!(project_report(&loaded_file)[0].starts_with("opened:"));
    }

    // ------------------------------------------------------------------
    // 判据 42: 界面保存的目标路径（`ROAD-M4-008` 选项 (a) 的保存 UI 入口）
    // ------------------------------------------------------------------

    /// 文件来源 ⇒ 保存写回**同一个**文件；样本来源 ⇒ `None`（不猜文件名）。
    ///
    /// 为什么它是一条独立判据：`save_target` 是"界面上按保存会写哪个文件"的**唯一**
    /// 判定点。若它在样本形态给出 `Some(untitled.yeban)`，用户会在自己的当前目录里
    /// 凭空得到一个文件 —— 那正是本仓库最忌讳的、查不出的现象。
    #[test]
    fn the_save_target_is_the_opened_file_and_none_for_a_sample() {
        let file = ProjectSource::File {
            path: PathBuf::from("/tmp/夜半/demo.yeban"),
            bytes: 7,
        };
        assert_eq!(
            file.save_target(),
            Some(PathBuf::from("/tmp/夜半/demo.yeban")),
            "文件来源的保存目标必须是**它自己**（不是同目录里另一个名字）"
        );
        assert_eq!(
            ProjectSource::Sample(Sample::Default).save_target(),
            None,
            "样本没有磁盘对应物 ⇒ 保存必须如实报\"未配置保存路径\""
        );
    }
}
