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
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use yeban_model::EntityId;
use yeban_model::container::ProjectArchive;
use yeban_model::project::{SampleRate, TrackV3, YebanProjectV1};
use yeban_render::dither::BitDepth;
use yeban_render::midi::MidiFormat;

use crate::bridge::{BridgeError, ViewState};
use crate::elements::ElementRegistry;
use crate::export_als::AlsExportError;
use crate::export_logic::LogicExportError;
use crate::export_midi::{MidiExportError, MidiExportReport, export_project_to_file};
use crate::input::{
    Action, InputContext, MixerParam, Modifiers, NudgeStep, PhysicalKey, Tool, View,
};
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

/// `--enable-ui-mcp-http` 的字面值（**UI 控制面**的环回入口，`[ARCH-UI-004]` / `[MUST-GATE-009]`）。
///
/// 与 [`MCP_HTTP_SWITCH`] 是**两条**开关而不是一条：领域 MCP 与 UI 控制面是两个方法命名空间
/// （`yeban_*` vs `ui/*`）、两个端点路径（`/mcp` vs `/ui-mcp`），它们的可用进程形态也不同
/// —— 详见 `src/ui_mcp_serve.rs` 的模块文档。
pub const UI_MCP_HTTP_SWITCH: &str = "--enable-ui-mcp-http";

/// 让 `--enable-ui-mcp-http` 真的可执行的那个**非默认** feature 名。
///
/// 默认构建里既没有 `yeban-ui-mcp` 也没有 UI 控制面的执行面 —— 它是
/// `[MUST-GATE-009]` 的第一道开关（`AGENTS.md` §2 红线 6）。用法错误必须**点名**它。
pub const UI_MCP_HTTP_FEATURE: &str = "ui-mcp-http";

/// UI 控制面的端点路径（权威定义在 `yeban-ui-mcp` 的 `transport::UI_MCP_PATH`）。
///
/// 与 [`MCP_HTTP_SWITCH`] 保留副本**同一个理由**：默认构建里 `yeban-ui-mcp` 只是
/// dev-dependency（`[dependencies]` 里的那条是 optional 且默认关），而用法文本在默认
/// 构建里也要打得出来。两边**逐字节相同**这件事由判据钉住 ——
/// `crates/yeban-app/tests/live_ui_mcp.rs` 断言它等于 `yeban_ui_mcp::transport::UI_MCP_PATH`
/// （它必须与领域 MCP 的 `/mcp` 不同：接错端口要能一眼看出来）。
pub const UI_MCP_PATH: &str = "/ui-mcp";

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

/// `--inspect-logic` 的字面值（Logic Pro `.logicx` bundle 的**只读探针**，`#143`）。
///
/// ⚠ 它**不是导入器**：不写工程、不写 demo、不产出 `.yeban` 文档。它只在
/// [`LOGIC_INSPECT_FEATURE`] 打开的构建里存在；默认构建给它 = 一次点名 feature 的
/// 用法错误（[`ParseError::InspectLogicNotCompiled`]）。
pub const LOGIC_INSPECT_SWITCH: &str = "--inspect-logic";

/// 让 `--inspect-logic` 真的可执行的那个**非默认** feature 名。
///
/// 与 [`LOGIC_EXPORT_FEATURE`] 同族：默认构建里 `yeban-render` 的 `logic_read`
/// 模块不存在，用法错误必须**点名**它。⚠ 集成者待办：它也要进
/// `scripts/guards/policy_check.py` 的 `FORBIDDEN_DEFAULT_FEATURES`。
pub const LOGIC_INSPECT_FEATURE: &str = "experimental-logic-import";

/// `--import-logic` 的字面值（**真导入** Logic Pro `.logicx`，任务单 `#149` / 刀 2）。
///
/// 它与 [`LOGIC_INSPECT_SWITCH`] 是两条命令：探针只打印读数、一个文件都不写；
/// 本开关把读数建成本仓工程并**必须**配 `--save-as <out.yeban>`（⛔ 不猜落点）。
/// 默认构建里给它 = 一次点名 feature 的用法错误
/// （[`ParseError::ImportLogicNotCompiled`]，退出码 2）。
pub const IMPORT_LOGIC_SWITCH: &str = "--import-logic";

/// 让 `--import-logic` 真的可执行的那个**非默认** feature 名。
///
/// 与 [`LOGIC_INSPECT_FEATURE`] 是**同一个** feature：`yeban-render` 的只读读取器
/// （`logic_read`）只在它打开时存在。本开关额外还要 app 侧的 [`crate::import_logic`]
/// 模块 —— 两者都在这个 `cfg` 后面。
pub const IMPORT_LOGIC_FEATURE: &str = "experimental-logic-import";

/// `--migrate-groove` 的字面值（旧 `.groove` ⇒ 新 `.yeban` 的**唯一**用户出口，任务单 `#146`）。
///
/// 它是**独立**的批处理命令：只读旧文件、只写 `--save-as` 指定的新容器，
/// 因此 [`parse_migrate_groove`] 在 [`parse`] **之前**分流，也不进 [`Options`]
/// （它不需要当前工程，也不该让别的开关与它组合后被静默丢掉）。
pub const MIGRATE_GROOVE_SWITCH: &str = "--migrate-groove";

/// ⭐ 任务单 `#211`：`--probe-instruments` 的字面值（"哪些能装"的**只读**读数）。
///
/// 它把库内 `manifest.json × 库根` 的每一条 SFZ 都跑一遍**字节探针**
/// （`bridge::probe_instrument_samples` —— 与点击装乐器**同一条**判据，⛔ 不是
/// `probe` 的内存正弦），逐条打印"可装 / 缺样本（N 个文件）/ SFZ 读不出来"，
/// 最后一行给 `installable/total` 读数。
///
/// ⚠ **慢**：真库全量一次约 10 分钟（`#210` 实测 597,530 ms）。它是给人**偶尔**核对的
/// 只读探针，⛔ 不进启动路径、⛔ 不进投影。
///
/// 与 `--inspect-logic` / `--migrate-groove` 同一条形状：**独立**命令，
/// 在 [`parse`] **之前**分流，也不进 [`Options`]（它不需要工程，也不该让别的开关
/// 与它组合后被静默丢掉）。
pub const PROBE_INSTRUMENTS_SWITCH: &str = "--probe-instruments";

/// `--theme` 的字面值（运行期调色板切换的**唯一**用户出口）。
pub const THEME_SWITCH: &str = "--theme";

/// `--print-theme` 的字面值（把"请求的主题 / 本二进制实际编译进来的风格"如实打出来）。
///
/// 与 `--print-shortcuts` 同族：一条**只读**的自述命令，让人与 CI 能核对
/// "我说要 material，这个二进制到底编的是不是 material"。
pub const PRINT_THEME_SWITCH: &str = "--print-theme";

/// ⭐ 任务单 `#287`：`--sampler-track` 单独给时的**逐字**用法错误。
///
/// 它同时是 [`ParseError::SamplerTrackWithoutTrackSampler`] 与 [`run_batch`] 第二道
/// 守卫的文案（一处定义 ⇒ 两条入口不会漂移）。
pub const SAMPLER_TRACK_PAIRING_ERROR: &str =
    "`--sampler-track` 必须与 `--track-sampler` 一起给 (它只选目标轨; 单独给会被静默忽略)";

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
  --migrate-groove <old.groove>
                           把旧 Groove Web 的 **JSON** 工程包 (`.groove`,
                           `format=groove-project` v1/v2) **迁移**成新 `.yeban` 容器;
                           必须与 --save-as <new.yeban> 成对给出 (⛔ 不默认落点);
                           **只读**旧文件 (迁移前后逐字节比对), 输出行:
                           migrated-groove: / migrated-groove-counts: /
                           migrated-groove-loss: (有损字段逐条) / migrated: ;
                           `format=groove-package` 或坏 JSON ⇒ 响亮报错 (退出码 {open}),
                           ⛔ 不产半成品; 本开关不与别的动作组合 (组合 = 用法错误)
  --import-logic <demo>    把一台 Logic Pro `.logicx` bundle (或含 bundle 的目录 —— 取
                           字典序第一台) **真导入**成 `.yeban` 工程; 必须与
                           --save-as <out.yeban> 成对给出 (⛔ 不默认落点);
                           默认构建不可用 (需 `experimental-logic-import` 特性);
                           打印读数 (轨 / 区域 / 音符) + 损失表 (AuRg / GenM / 自动化 /
                           轨道归属: 实测绑定数 + 余下 provisional 数); 输入 demo **只读**;
                           路径不存在 ⇒ 打印「跳过」并成功退出 (不红),
                           路径在但读不懂 ⇒ 报错 (退出码 {open});
                           本开关不与别的动作组合 (组合 = 用法错误)
  --probe-instruments      【⭐ 任务单 #211】**只读**读数: 把库内 `manifest.json × 库根`
                           的每一条 SFZ 逐条跑**字节探针** (与点击装乐器**同一条**判据,
                           ⛔ 不是 `probe` 的内存正弦), 逐条打印可装 / 缺样本(逐字原因) /
                           SFZ 读不出来, 最后一行给 total=… installable=… missing=…;
                           ⚠ 慢: 真库全量一次约 10 分钟 (#210 实测 597,530 ms);
                           **不写**库根 / 工程, 不开窗口; 不与别的参数组合 (组合 = 用法错误)
  --dump-elements          把语义元素注册表打印到 stdout (每行一个元素, 稳定顺序) [UI-TEST-001]
  --export-elements <path> 把同一份元素注册表**原子**写到文件, 便于脚本 / AI 消费;
                           与 --dump-elements 同时给 = 既打印又落盘
  --record-wav <path>      录一段**测试音**成 24-bit WAV (音源 = 内存正弦 440Hz, ⛔ 不是声卡);
                           三段走 ARCH-REC-001: 构造期建环 -> 逐帧推 -> 抽干落盘;
                           输出行 recorded-wav: path=… frames=… dropped=… bytes=… source=…
  --record-seconds <n>     --record-wav 的秒数 (默认 2.0)
  --record-source <s>      project (默认) = 录**工程离线渲染的声音** (与 --export-wav 同一条
                           核心 ＋ 同一种子 ⇒ **逐位一致**); tone = 内存正弦测试音
                           与 --sfz 同时给 ⇒ 录的就是**该 SFZ 音色**的声音
  --sfz <path>             装载一份 SFZ 乐器: 报告区域数 + 一次音符探测(帧数与能量);
                           与 --export-wav 同时给 ⇒ **导出的声音来自这份 SFZ**;
                           `sample=<path>` 会被**真读** (只支持本仓 read_plain_wav 能读的
                           PCM WAV; 读不到 ⇒ 如实回退到内存正弦并把原因写进 source=)
  --track-sampler <path>   把这份 SFZ **装到一条轨** (TrackV3::sampler), 走既有的
                           Op::SetTrackSampler ⇒ 一次提交、可撤销;
                           目标轨 = --sampler-track 给的身份, 缺省 = 身份序第一条
                           非主总线轨 (旧行为, 逐字节不变; CLI 没有选中态);
                           样本字节缺失 ⇒ 响亮拒绝 (不把一条有声的轨装成静音);
                           与 --export-wav / --save-as 同给 ⇒ 它们看到的是装好之后的工程
  --sampler-track <trackId>
                           把 --track-sampler 装到**哪条**轨: <trackId> = 音轨身份
                           (26 字符 ULID, 大小写不敏感; 即 project.json `tracks` 的键,
                           yeban_query_project 的 tracks[].id);
                           身份形状不对 / 工程里没有 / 是主总线 ⇒ 响亮报错并列出可用身份
                           (不回落); 主总线被拒的理由: 它是唯一混音出口, 渲染的
                           renderable 排除它 ⇒ 装上去的 sampler 永远不会有消费点;
                           必须与 --track-sampler 一起给 (单独给 = 用法错误, 退出码 {usage});
                           两种形状:
                             yeban-app --track-sampler p.sfz          # 旧 = 身份序第一条非主总线轨
                             yeban-app --sampler-track <trackId> --track-sampler p.sfz
  --midi-in <port>         打开一个**物理 MIDI 输入端口**并采集 (**需要** midi-input 特性;
                           默认构建会给出结构化错误并说明怎么开)
                           <port> = 端口序号 (全十进制) 或端口名**子串** (大小写敏感);
                           子串命中多于一个 ⇒ 报错并**列出候选** (不猜)
  --midi-in-seconds <n>    采集时长 (默认 3; 上限 600 秒) —— 到点**必然退出**
                           报告行的口径 (两个**累计条数**, 单位都是**条**):
                             unsupported=     收到的消息里**不承载**的条数
                                              (系统专用/可变长/长度不符; 不是环满)
                             dropped-events=  环满而**被丢弃的事件**条数
                                              (从建环那一刻累计到**采集结束时那一次读**)
                           ⇒ 采了 3 秒却少听到几条时, `dropped-events` 会**明说丢了几条**,
                             而不是悄悄少几条
  --midi-in-write-notes    把这次采到的音符**写进工程** (谱面有反应): 每个音符一次
                           Op::AddNote, 来源标记 `OpOrigin::MidiInput`, **一次**提交
                           (可撤销、进提交史); 同一命令里的导出 / --save-as 能看到它们
  --midi-in-render <path>  把**同一批事件**推进 `EngineEvent` 环并**离线渲染**成
                           24-bit WAV (声音有反应): 事件经既有的 UI→引擎 SPSC
                           (EngineHost::send_engine_events), 由**与设备回调同一个**
                           `process_quantum` 渲染 ⇒ 产物是「事件真的作用到合成器」的声音;
                           报告行 `poly-notes-triggered=` 是「事件到了器件」的见证;
                           **不需要**先写工程 (谱面与声音是两件事, 见下);
                           帧数/采样率/声道/种子取 --export-wav 的默认 (8 秒 48 kHz 立体声);
                           与 --export-wav 共用同一个量化实现 ⇒ 产物可比
  --midi-in-track <序号|名字子串>
                           音符写进哪条 / 事件发往哪条音轨; 规则与 --midi-in 的**端口**选择
                           同一套 (全十进制 ⇒ 序号; 否则名字子串, 大小写敏感; 命中 0 个 ⇒
                           列出可用音轨; 命中 >1 个 ⇒ 列出候选, 不猜);
                           缺省 = **第一条非 master 音轨**
  --import-midi <path>     把标准 MIDI 文件里的音符导入**第一条非 master 音轨**, 每个音符的
                           来源标记是 `OpOrigin::MidiInput`; 在导出与 --save-as **之前**执行
                           (同一命令里的导出/保存都能看到导入的音符); 非法输入不改坏工程
  --import-audio <path>    把一份音频文件导入工程的片段池 (clip_pool), 来源标记是
                           `OpOrigin::Import`; 字节按内容寻址 (源字节的 SHA-256) 落进
                           --save-as 容器的 `assets/{{sha256}}`; 与 --import-midi 同位置
                           (在导出与 --save-as **之前**执行); 非法/非音频输入不改坏工程;
                           ⚠ 只吃 PCM/float **WAV** (16/24-bit 整数、32-bit 浮点 —— 本仓
                           `read_plain_wav` 能读的集合; FLAC/OGG 在本入口被明确拒绝);
                           ⚠ 只**登记片段**、不摆放 (摆放见台账的下一片);
                           ⚠ 只在**无窗口路径**上执行: 只给本开关 (不给 --save-as /
                           --export-* / --headless 等) 会进 GUI 路径, 导入**不会发生**
                           (与 --import-midi 同一条既存口径)
  --export-wav <path>      把当前工程**离线渲染**成 WAV (默认 24-bit 立体声 8 秒): 音源是
                           最小音源 (**方波 + 首尾包络**, ⛔ 非引擎乐器, 见 `export_wav.rs` 的
                           诚实登记); 与 --export-midi 同族: 不给 --open 时导出演示工程;
                           位深 / 采样率 / 时长 / 帧数由下面四条选
  --wav-bit-depth <d>      --export-wav 的位深: 16 | 24 | 32f (默认 24)
                           (16/24 走 TPDF 抖动; 32f 是浮点**透传**, 不抖动;
                            ⛔ 写 `32` 是用法错误 —— 本仓 32 位只有浮点一种容器)
  --wav-sample-rate <hz>   --export-wav 的输出采样率, 取值必须是规范枚举之一:
                           44100 | 48000 | 88200 | 96000 | 192000
                           (默认**跟随工程** audio_config.sample_rate; 要旧口径就显式给 48000)
  --wav-seconds <秒>       --export-wav 的**时长** (默认 8 秒, 与旧行为一致;
                           帧数 = round(秒 × 采样率)); ⛔ 缺省不是\"工程可闻末端\"
  --limiter                --export-wav 在**主总线推子之后**跑母线限制器
                           (默认关; 开启时补偿 33 帧前瞻延迟, 输出长度不变)
  --wav-frames <n>         --export-wav 的**精确帧数** (与 MCP render_master.frames 同量纲):
                           原样使用, 不再做秒×采样率换算; 与 --wav-seconds **互斥**;
                           必须是**正**整数 (0 帧是空产物, 按用法错误拒绝)
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
                             **后缀**: 目标**必须**以 `.logicx` 结尾 —— 缺了会**自动补**
                             (`hello` ⇒ `hello.logicx`; macOS 的 `open -a` 靠扩展名认
                             bundle, `open -a \"Logic Pro\" hello` 会报错); 已带则**不**重复加
                             (`foo.logicx` 不变 `foo.logicx.logicx`), 回执的 `path=` 打印
                             **实际**路径 (见下面的用法示例);
                             **映射损失表逐条打到 stdout** (`logic-losses:` / `logic-loss:` 行),
                             因此\"哪些构造没被映射\"对用户可见 (太长时礼貌截断并写明还剩几条);
                             **打开结论（实测，勿外推）**: 本机 **Logic Pro 12.2** 能打开**供体拼接**
                             产物 (负责人实测, 2026-10-06); **3 轨**那份另有一次人类验收 ——
                             负责人 2026-10-18 在真机 Logic Pro 12.2 上报\"三条轨正常播放\",
                             且**三条轨各有自己的音色** (`Studio Grand` / `Studio Grand` /
                             `Epic Cloud Formation`); ⚠ 但那三个音色来自**供体**的通道条,
                             ⛔ **不是用户工程的设备链** —— 用户挂的 EQ / 混响 / 采样器**不会**
                             出现在 `.logicx` 里 (我们自己的设备链另按 `未映射:` 条目逐条点名);
                             仍**未验**: 第 3 槽 1 小节 region 长度 / 其它版本与机器 /
                             `b0`/`b1` 的 16 字节结构 (已证明**不是**音色路由的必要条件);
                             自研 (无供体) 写入器的产物被拒绝过两次
                             (账本第 403、405 轮), 结论不覆盖其它 Logic 版本与其它机器; **只在**
                             `--features {logic_feature}` 的构建里存在 —— 默认构建给这个开关
                             = 用法错误 (退出码 {usage})
  {logic_inspect_switch} <dir>
                            Logic Pro demo bundle 的**只读探针** (⛔ **不是导入器**) [ARCH-FMT-002]
                             / #143: 扫 <dir> 下的 Apple demo (`*.logicx` 目录), 逐台打印
                             MetaData 七键 / 轨道 / 区域 (**过滤前后两个读数都给**) / 音符 / AuRg,
                             最后一行是合计; 目录不存在 ⇒ 打印\"无 demo、跳过\"并退出 {ok}
                             (CI/其它机器不受影响); 目录存在但解析失败 ⇒ 失败 (退出码 {open})
                             —— ⛔ **不**静默跳过; ⛔ 不写 demo、不写工程、不产出可导入文档
                             (Apple 演示工程有版权); **只在** `--features {logic_import_feature}`
                             的构建里存在 —— 默认构建给这个开关 = 用法错误 (退出码 {usage})
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
                           empty = 0 音轨 + 1 主总线的空工程 (规范 空工程空闲常驻内存
                           [BASELINE-002] 所指的那个对象); 重复给以最后一个为准
  --headless-idle          无窗口模式下**真的**构造 Slint 控件树: 自研软件平台
                           (MinimalSoftwareWindow + SoftwareRenderer, 不需要显示器),
                           逐行光栅化一帧当见证, 空闲 N 秒后退出 (N 由 --idle-seconds 给)。
                           这是 [BASELINE-002] 那句 空工程空闲常驻内存 要量的**对象**
                           (--headless 一个 Slint 对象都不构造); 与 --headless 同时给
                           = 只走本模式 (它是更强的形态), 握手行换成 {idle_handshake}。
  --idle-seconds <N>        --headless-idle 的空闲秒数 / UI 控制面的服务秒数
                            (整数 1..={max_idle});
                            只对 --headless-idle 与 {ui_mcp_switch} 有意义,
                            单独给 = 用法错误 (退出码 {usage}); 重复给以最后一个为准
  --enable-mcp-http        把**领域 MCP** 的环回 HTTP JSON-RPC 控制面挂进**本进程**
                           (形态 A, [ROAD-M4-001]): 只绑 127.0.0.1:0 (端口由系统分配,
                           绑后回读断言是环回)、强制 256-bit Bearer 令牌鉴权;
                           默认**关**, 且只在 `--features in-process-mcp` 的构建里存在;
                           令牌落 ~/.yeban/session.token (0600), 只报路径不报令牌。
                           等价入口: 环境变量 {mcp_env}=1
  {ui_mcp_switch}  把 **UI 控制面** (14 条 `ui/*` 方法, [ARCH-UI-004]) 挂成一个
                           真的能 curl 的**环回**入口: 只绑 127.0.0.1:0 (端口由系统
                           分配, 绑后回读断言是环回)、强制 256-bit Bearer 令牌鉴权
                           (缺失 / 错误 ⇒ 401 + WWW-Authenticate)、**生产模式** ⇒
                           `ui:inject` 族硬禁 (403 `forbidden-in-production`, 判定
                           **先于**令牌校验); 默认**关**, 且只在
                           `--features {ui_mcp_feature}` 的构建里存在; 令牌落
                           ~/.yeban/session.token (0600), 只报路径不报令牌;
                           令牌写不进去 ⇒ **停机并释放监听口** (fail closed)。
                           端点路径 {ui_mcp_path} (与领域 MCP 的 /mcp 刻意不同)。
                           进程形态: 软件光栅化平台上的**活控件树** —— 不创建 OS 窗口、
                           不进阻塞事件循环, 因为 `slint::platform::set_platform`
                           每线程只能装一次, 而 GUI 路径已经把 winit 后端装上了。
                           配 --headless-idle + --idle-seconds <N> = 服务 N 秒后自动
                           停机并释放; 不配 = 服务到进程被终止 (Ctrl-C / kill)。

运行形态:
  yeban-app                启动 GUI (需要显示器; 进入阻塞事件循环)
  yeban-app --open a.yeban 用打开的那个工程启动 GUI
  任一\"无窗口开关\"(--headless / --dump-elements / --print-shortcuts /
  --print-theme / --export-elements / --export-midi / --export-wav /
  --export-als / --export-logic / --save-as / --record-wav / --sfz /
  --track-sampler / --midi-in / --midi-in-render / --midi-in-write-notes)
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
  --midi-in-write-notes / --midi-in-render / --midi-in-track
                               **必须**与 --midi-in 一起给 (单独给 = 用法错误, 退出码 {usage})
                               —— 否则它们会被静默忽略, 而**静默忽略**正是本仓最忌讳的形态
  --midi-in 两条落地路径        谱面: --midi-in-write-notes (音符进工程, 可撤销/可导出)
                               声音: --midi-in-render <path> (事件进环 ⇒ 离线渲染)
                               两者**互相独立**、可单给、可同给 (同给时都生效)
  两条同给时的顺序              **先声音 → 再谱面**: 先渲染的是**还没写进工程**的那一份
                               ⇒ 每条音符只被触发一次 (否则环与快照排程会各触发一次 = 双触发)
                               两者都不给时 --midi-in 仍是**只读**诊断 (⛔ 不建会话、⛔ 不写盘)
  --sampler-track 与 --track-sampler
                               --sampler-track 只**选目标轨**, 必须与 --track-sampler 一起给
                               (单独给 = 用法错误, 退出码 {usage}); 缺省 = 身份序第一条
                               非主总线轨 (旧行为, 逐字节不变); 指定不存在的身份 / 主总线
                               = 响亮报错 (退出码 {open}), ⛔ 不静默回落到第一条
  --save-as 不给 --open        保存的是演示工程, 输出 `saved: ... from=sample=default` 明说
  --save-as 与 --headless      两者都是无窗口路径, 可以一起给 (保存不需要窗口)
  --export-elements 与 --export-midi 与 --export-als 与 --export-logic 与 --save-as 任意组合
                               顺序固定: **先**导出元素, **再**导出 MIDI, **再**导出 .als,
                               **再**导出 .logicx, **最后**保存工程; 任一导出失败 ⇒ 不写工程
                               (退出码 {export})
  --export-midi 不给 --open    导出的是演示工程, 输出 `exported-midi: ... from=sample=...` 明说
  --export-wav 不给 --open    同上: 导出演示工程, 输出 `exported-wav: ... from=sample=...` 明说
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
  {ui_mcp_switch} 与 --headless / --dump-elements / --print-shortcuts /
  --export-* / --save-as / {mcp_switch}
                               不能组合 = 用法错误 (退出码 {usage}): 前一组会被**静默丢掉**;
                               与领域 MCP 那条不在同一个进程形态里 —— 领域控制面要挂在
                               **正在跑的 GUI 事件循环**里, 而 ui/* 的 Tier-1 执行面只能装在
                               软件光栅化平台上 (set_platform 每线程一次且必须先于组件构造)
  {ui_mcp_switch} 与 --headless-idle 可以一起给
                               两个开关说的是同一档进程形态 (软件平台上的活控件树);
                               同时给 => 走 UI 控制面 (它更强: 真的应答 ui/*)
  {ui_mcp_switch} 与 --idle-seconds
                               --idle-seconds 可选: 给了就服务到点自动停机并释放监听口,
                               不给就一直服务到进程被终止 (Ctrl-C / kill)

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
      {ui_mcp_switch} 与不该组合的开关同给或本次构建未编译 `{ui_mcp_feature}` /
      --export-als 在本次构建未编译 `{als_feature}` /
      --export-logic 在本次构建未编译 `{logic_feature}` /
      {logic_inspect_switch} 在本次构建未编译 `{logic_import_feature}`,
      或与其它参数组合 (它是独立只读探针) /
      {import_logic_switch} 在本次构建未编译 `{import_logic_feature}` /
      没配 --save-as / 与其它参数组合 (它是独立导入命令))
  {open} --import-midi 失败 (读文件 / 解析 SMF / 空文件 / 没有目标音轨或摆放 / 提交被拒)
  {open} --import-audio 失败 (读文件 / 超过 64 MiB 单文件上限 / 非 PCM-float WAV / 提交被拒)
  {open} --import-logic 失败 (读 demo / ProjectData 记录流不合法 / 采样率不在规范枚举 /
     没有 karT 轨道 / 建出的工程未通过模型校验; 输入路径不存在是 skip, ⛔ 不算失败)
  {open} --sfz 失败 (读文件 / SFZ 文本非法)
  {open} --track-sampler 失败 (没有展示轨 / 路径不是 UTF-8 / 样本字节缺失 / 提交被拒 /
      --sampler-track 的身份形状不对 / 工程里没有该身份 / 该身份是主总线)
  {usage} --sampler-track 没和 --track-sampler 一起给
  {open} --midi-in 失败 (端口不存在或名字有歧义 / 宿主不可用 / 本次构建未编译 `midi-input` 特性)
  {open} --midi-in-write-notes / --midi-in-render 失败 (一条音符都没有 / 没有可写目标 /
     没有可发声的目标音轨 / 音轨选择器命中 0 或多条 / 提交被拒 / 引擎重建失败)
  {export} --midi-in-render 的 WAV 落盘失败 (与 --export-wav 同一档)
  {usage} --record-seconds 没和 --record-wav 一起给
  {usage} --wav-bit-depth / --wav-sample-rate / --wav-seconds / --wav-frames 没和
      --export-wav 一起给, 或取值非法 (位深只有 16|24|32f; 采样率只有规范枚举那五个;
      秒数必须为正; 帧数必须为正整数; --wav-frames 与 --wav-seconds 互斥)
  {open} --open 失败 (读文件失败 / 超过 4 GiB 上限 / 不是 `.yeban` 容器 /
     容器拒绝: 压缩法 / Zip-Slip / 解压炸弹 / 截断 / CRC 不匹配 / 缺件 / 非法 project.json …)
  {save} --save-as 失败 (临时文件 / 刷盘 / 原子重命名任一步失败, 或容器写出被拒)
  {export} --export-elements 失败 / --export-midi 失败 / --export-wav 失败 / --export-als 失败 / --export-logic 失败
      (I/O; 或工程里没有可导出的 MIDI 音符 / 拍号分母不是 2 的幂 /
      工程 PPQ 与编码器默认 PPQ 不一致 / 编码器拒绝越界的音高或力度 /
      .als 的映射或 Gzip 封装失败 / .logicx 的 bundle 目录建不出来)

示例 (全部已在真二进制上跑过):
  yeban-app --headless
  yeban-app --headless --project-sample empty
  yeban-app --open song.yeban --headless
  yeban-app --headless-idle --idle-seconds 2 --project-sample empty
  yeban-app --enable-ui-mcp-http --headless-idle --idle-seconds 30
  yeban-app --open song.yeban --save-as copy.yeban
  yeban-app --open song.yeban --dump-elements
  yeban-app --open song.yeban --export-elements elements.txt
  yeban-app --open song.yeban --record-wav take.wav --record-seconds 8
  yeban-app --record-wav tone.wav --record-seconds 3 --record-source tone
  yeban-app --open p.yeban --sfz strings.sfz --record-wav take.wav --record-seconds 8
  yeban-app --sfz strings.sfz --export-wav out.wav   # 导出的音源 = 该 SFZ
  yeban-app --open song.yeban --track-sampler p.sfz --export-wav out.wav  # 轨上的乐器先装好再导出
  yeban-app --open song.yeban --sampler-track 01J8Z5Q0R7K3M9X2V4B6N8P0A0 --track-sampler p.sfz --save-as tuned.yeban  # 指定轨 (身份)
  yeban-app --import-midi take.mid --save-as song.yeban
  yeban-app --open song.yeban --import-audio kick.wav --save-as song.yeban
  yeban-app --open song.yeban --export-midi song.mid
  yeban-app --open song.yeban --export-wav song.wav
  yeban-app --open song.yeban --export-wav shaped.wav --wav-bit-depth 16 --wav-sample-rate 44100 --wav-seconds 4
  yeban-app --open song.yeban --export-wav exact.wav --wav-frames 192000  # 与 MCP render_master.frames 对齐
  yeban-app --open song.yeban --export-logic out/Song.logicx
  yeban-app --open song.yeban --export-logic out/hello   # ⭐ 缺 `.logicx` 会补成 out/hello.logicx
  yeban-app --inspect-logic \"/Library/Application Support/Logic/Logic Pro X Demosongs\"
  yeban-app --probe-instruments   # ⭐ 逐条读库内 SFZ 的样本字节 (约 10 分钟), 末尾给 total=
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
        ui_mcp_switch = UI_MCP_HTTP_SWITCH,
        ui_mcp_feature = UI_MCP_HTTP_FEATURE,
        ui_mcp_path = UI_MCP_PATH,
        mcp_switch = MCP_HTTP_SWITCH,
        als_feature = ALS_EXPORT_FEATURE,
        logic_feature = LOGIC_EXPORT_FEATURE,
        logic_inspect_switch = LOGIC_INSPECT_SWITCH,
        logic_import_feature = LOGIC_INSPECT_FEATURE,
        import_logic_switch = IMPORT_LOGIC_SWITCH,
        import_logic_feature = IMPORT_LOGIC_FEATURE,
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
    /// 逐项事实（由判据 `empty_sample_is_a_zero_audio_track_project_with_witness` 钉住）：
    /// **0 条非主总线轨 ＋ 恰 1 条 `TrackKind::Master`**、`master_bus_track_id` 指向它、
    /// `routing_graph.nodes` 里恰好有它；无片段 / 场景 / 段落 / 资产 / `history.dag`。
    /// 它在模型侧**合法且可读**（`YebanProjectV1::validate()` 接受"只有主总线"的工程），
    /// 因此 app 侧不需要任何"空工程特例" —— 停用某个样本就等于没量到这个对象。
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
    /// `empty` **不**沿用 `default` 那句"内置演示工程"：它是**没有音轨**的空工程
    /// （`#161` 起出厂自带 1 条主总线），输出里不能出现与事实相反的描述
    /// （本仓库的第一条纪律是不写假话）。
    /// `default` / `filled` 的措辞保持**逐字不变** —— 改它们要同步
    /// `tests/cli_contract.rs` 与其它工作线的账本，超出本线范围。
    #[must_use]
    pub const fn origin_label(self) -> &'static str {
        match self {
            Self::Empty => "内置空工程 0 音轨 ＋ 1 主总线",
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

    /// 构造样本自带的**会话资产池字节**（任务单 `#142`）。
    ///
    /// 为什么需要这一层：`ClipContent::Audio` 在工程模型里只有 `AssetHash`，PCM 住在
    /// `ProjectArchive.assets`（会话资产池）。内置样本此前把这一层**如实置空** ⇒
    /// 样本里那条音频片段引用一份谁也没有的字节，`EngineHost` 第 1.3 步（`#136`）
    /// 只能把它记成"跳过" ⇒ **演示工程出厂即静音**。
    ///
    /// 现在字节由 `yeban-model` 在**运行期合成**（⛔ 不入库二进制，⛔ 不加依赖），
    /// 且哈希与 `Sample::project()` 的引用**同源**：两者都调用同一个合成器。
    ///
    /// `empty` 没有片段 ⇒ 空池（如实置空，⛔ 不是丢失）。
    #[must_use]
    pub fn assets(self) -> Vec<(yeban_model::AssetHash, Vec<u8>)> {
        match self {
            Self::Default => yeban_model::samples::demo_project_assets(),
            Self::Filled => yeban_model::samples::filled_project_assets(),
            Self::Empty => Vec::new(),
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
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// `--import-midi <path>`：把一段标准 MIDI 文件（SMF）里的音符**导入**当前工程的
    /// 第一条非 master 音轨，每个音符的来源标记是 `OpOrigin::MidiInput`
    /// （⛔ 不是 `UserUi` —— 来源是一等公民元数据）。
    ///
    /// **执行顺序**：导入是**编辑**，所以在导出与 `--save-as` **之前**执行
    /// ⇒ 同一命令里的 `--export-midi` / `--export-wav` / `--save-as` 都能看到导入的音符。
    /// 解析与映射全部完成后**才**发出一次提交 ⇒ 非法输入不会改坏工程。
    pub import_midi: Option<PathBuf>,
    /// `--import-audio <path>`：把一份音频文件**导入**当前工程的 `clip_pool`（音频片段），
    /// 来源标记是 `OpOrigin::Import`（⛔ 不是 `UserUi` —— 来源是一等公民元数据）。
    ///
    /// **执行顺序**：与 `--import-midi` 同一条契约（导入是**编辑**，在导出与 `--save-as`
    /// **之前**执行）⇒ 同一命令里的 `--save-as` 保存的是**导入后**的工程。
    /// 解码与派生全部完成后**才**发出一次提交 ⇒ 非音频输入不会改坏工程。
    ///
    /// ⚠ 两条**如实登记**的边界（见 `crate::import_audio` 的模块文档）：
    /// ① 格式面 = `yeban_render::wav::read_plain_wav` 能读的 **PCM／float WAV**
    /// （16/24-bit 整数、32-bit 浮点）；MCP 的 `yeban_import_audio` 走 `yeban-decode`
    /// 还能吃 FLAC／OGG，那需要一条本切片**没有加**的依赖边；
    /// ② 只登记 `clip_pool`、**不摆放**（`Op::AddClipPlacement` 留作下一片）。
    ///
    /// ⚠ 第 3 条边界（**路径**）：本开关只在**无窗口（批处理）路径**上执行，而那条路径由
    /// `--save-as` / `--export-elements` / `--export-midi` / `--headless` 等开关选中。
    /// 只给 `--import-audio`（一个批处理开关都不给）⇒ `Options::batch()` 为假 ⇒ 进程进
    /// GUI 路径（`run_gui` **不消费**本字段）⇒ **导入不会发生**。这是 `--import-midi` 早就
    /// 存在的**同一条口径**（两者都没有"静默忽略"守卫），本切片**如实登记**它、⛔ 不假装
    /// 它不存在，也⛔ 不在这里单独发明一条只约束本开关的守卫（那会让两条导入的语义分叉）。
    ///
    /// ⚠ 字节落在 `--save-as` 容器的 `assets/{sha256}`；没给 `--save-as` 时编辑只存在于
    /// 内存（同一命令里的 `--export-*` 看得见它）。
    pub import_audio: Option<PathBuf>,
    /// `--record-wav <path>`：录一段**测试音**并落成 24-bit WAV（`[ARCH-REC-001]` 三段：
    /// 构造期建环 ⇒ 生产端逐帧推 ⇒ 消费端抽干落盘）。
    ///
    /// ⚠ 本切片**不碰声卡**（cpal 输入流不在本轮）⇒ 数据来源是一条**内存正弦测试音**
    /// （报告行 `source=in-memory-sine(440Hz)`）；`--record-seconds` 给秒数（默认 2.0）。
    /// 环满 ⇒ 丢弃并计数（输出行 `dropped=`），⛔ 不阻塞、⛔ 不 panic。
    pub record_wav: Option<PathBuf>,
    /// `--record-seconds <n>`：`--record-wav` 的录制秒数（默认 2.0）。
    pub record_seconds: Option<f32>,
    /// `--record-source <project|tone>`：录什么（默认 **project** = 工程离线渲染的声音；
    /// `tone` = 内存正弦测试音）。⭐ `project` **已接 `--sfz`**：同时给了 `--sfz` ⇒ 录的就是
    /// 那份 SFZ 的乐器（`run_batch` 里 `record_project` 的调用会收到 `loaded_sfz`），
    /// 与 `--export-wav` 同一条渲染核心。
    pub record_source: Option<String>,
    /// `--sfz <path>`：**装载**一份 SFZ 乐器并报告区域数与一次音符探测读数。
    ///
    /// ⭐ **已接线**：与 `--export-wav` 或 `--record-wav` 同时给 ⇒ 音源链就用这份 SFZ
    /// （报告行 `wired_to_export=yes`；接线见 `run_batch` 里 `report.wired_to_export` 的赋值与
    /// `export_wav_with_sfz_file_and_assets` 的调用，GUI 侧见 `host.rs` 的 `on_export_wav`）。
    /// 样本数据**真读** SFZ 里 `sample=` 指到的 PCM WAV（`sfz_load::read_sample_file`）；
    /// 读不到才**如实回退**到内存正弦并把原因写进报告行。
    /// 本开关**只读**（不建会话、不改工程）⇒ 与导出/保存可以任意组合。
    pub sfz: Option<PathBuf>,
    /// ⭐ 任务单 `#210`：`--track-sampler <path>`：把一份 SFZ **装到一条轨**
    /// （`TrackV3::sampler`），走**既有**的 `Op::SetTrackSampler`。
    ///
    /// 它是 GUI"点一下换乐器"的**命令行等价路径**：
    /// 1. 目标轨 = [`Self::sampler_track`] 给的身份（`#287`），缺省 =
    ///    [`crate::bridge::device_rack_track`]（身份序第一条非主总线轨；
    ///    GUI 用"选中轨 or 兜底"——CLI 没有选中态，取同一个兜底）；
    /// 2. `Op` 由 [`crate::bridge::instrument_install_ops_on`] 构造 ⇒ **同一条**
    ///    样本字节谓词（缺样本 ⇒ 响亮拒绝，⛔ 不把有声轨装成静音）；
    /// 3. 经 [`crate::undo::UndoPort`] **一次提交**（进提交史、可撤销），
    ///    并把结果落回归档 ⇒ 同一命令里的 `--export-wav` / `--save-as` 看得见它。
    ///
    /// **执行顺序**：与 `--import-midi` 同一条契约（编辑 ⇒ 在导出与保存**之前**）。
    /// 只给本开关（没有别的批处理开关）也走 `run_batch`（[`ACTION_SWITCHES`] 里是
    /// `Batch`）：它会真的改内存里的工程并打印一行读数，然后退出。
    ///
    /// ⭐ 任务单 `#287`：目标轨可以用 [`Self::sampler_track`] **显式指定**；不给时
    /// 仍是上面第 1 条的老规则（逐字节不变）。
    pub track_sampler: Option<PathBuf>,
    /// ⭐ 任务单 `#287`：`--sampler-track <trackId>`：`--track-sampler` 装到**哪条**轨。
    ///
    /// 取值是工程里的**音轨身份**（26 字符 ULID，大小写不敏感）—— 即 `project.json`
    /// `tracks` 的键 / `yeban_query_project` 的 `tracks[].id`。
    ///
    /// * 缺省 ⇒ 旧行为：身份序第一条非主总线轨（[`crate::bridge::device_rack_track`]）；
    /// * 给了 ⇒ **只**装到那一条：身份形状不对 / 查不到 / 是主总线 ⇒ **响亮报错**
    ///   （⛔ 静默回落到第一条）；
    /// * **必须**与 `--track-sampler` 一起给（单独给 = 用法错误，见
    ///   [`SAMPLER_TRACK_PAIRING_ERROR`]）。
    ///
    /// 为什么是**独立开关**，而不是把身份塞进 `--track-sampler <id>:<path>`：路径里
    /// 允许出现 `:`（POSIX 文件名的合法字符；Windows 盘符是第二个例子）⇒ 那种形状
    /// 会把一个**合法**的旧调用重新解释成 `id:path`，破坏"旧用法逐字节不变"。独立
    /// 开关也与既有的 `--midi-in-track`（同样是独立的目标选择器）同形。
    pub sampler_track: Option<String>,
    /// `--midi-in <端口>`：⭐ **打开一个物理 MIDI 输入端口并采集**（⭐ 本仓第一条
    /// **用户可触发**的硬件 MIDI 入口）。
    ///
    /// ⭐ 取值接受**端口序号**（全十进制）或**端口名子串**（⭐ 大小写敏感）⇒
    /// ⭐ 命中 0 个 ⇒ 结构化错误**列出可用端口**；⭐ 命中 >1 个 ⇒ 结构化错误**列出候选**
    /// （⛔ 不猜）。⚠ 本开关**只在** `midi-input` feature 下可用；⭐ 默认档给的是
    /// **结构化错误**（⭐ 点名 feature ＋ 一句怎么开），⛔ 不是"未知参数"。
    pub midi_in: Option<String>,
    /// `--midi-in-seconds <n>`：`--midi-in` 的采集时长（秒；默认 [`crate::midi_in::DEFAULT_SECONDS`]）。
    ///
    /// ⭐ **到点必然退出**（⛔ 永不无限阻塞）＋ ⭐ 上限
    /// [`crate::midi_in::MAX_SECONDS`]（⭐ 防止误敲一个巨大的数让命令卡住）。
    pub midi_in_seconds: Option<f32>,
    /// `--midi-in-write-notes`：⭐ 把这次采到的音符**写进工程**（谱面有反应）。
    ///
    /// 走**既有**的 `Op::AddNote` ＋ 一次带 [`yeban_model::OpOrigin::MidiInput`] 的提交
    /// （与 `--import-midi` 同一条）⇒ 可以撤销、进提交史，并且同一命令里的
    /// 导出 / `--save-as` 都能看到这些音符。
    pub midi_in_write_notes: bool,
    /// `--midi-in-render <path.wav>`：⭐ 把**写完音符之后**的工程**离线渲染**成 24-bit WAV
    /// （声音有反应）。**必须**与 [`Self::midi_in_write_notes`] 一起给。
    ///
    /// ⛔ 不新建第二条音频路径：走的就是 `--export-wav` 那条
    /// （`SynthEngine::arm_from_snapshot` + `render_track` ＋ 同一个量化实现）。
    ///
    /// ⚠ 为什么不是"把事件推进 `EngineEvent` 环再渲染"：那条路今天不通 —— 引擎的出队闭包
    /// 只认 `SetParam` / `Transport`，`NoteOn` / `NoteOff` **没有消费者**（实测 4/4 事件进环
    /// ⇒ 全静音）。断点登记与证据在 [`crate::midi_notes`] 的模块文档里。
    pub midi_in_render: Option<PathBuf>,
    /// `--midi-in-track <序号|名字子串>`：音符写进哪条音轨 / 事件发往哪条音轨。
    ///
    /// ⭐ 规则与 `--midi-in` 的端口选择**同一套**：全十进制 ⇒ 按**序号**（非 master 音轨的
    /// 身份序）；否则按**名字子串**（大小写敏感）；命中 0 个 ⇒ 列出可用音轨；
    /// 命中 >1 个 ⇒ 列出候选（⛔ 不猜）。缺省 = **第一条非 master 音轨**。
    pub midi_in_track: Option<String>,
    /// `--export-midi <path>`：当前工程导出成标准 MIDI 文件（SMF 1）[ADR-0001 **D47**]。
    pub export_midi: Option<PathBuf>,
    /// `--export-wav <path>`：把当前工程**离线渲染**成一段 WAV（导出族）。
    ///
    /// 与 `--export-midi` 同族：不给 `--open` 时导出**演示工程**并在输出里明说；
    /// 渲染与量化全部在内存里完成，**最后一步**才落盘（拒绝时不留半成品）。
    ///
    /// 位深 / 采样率 / 时长 / 精确帧数由下面四个开关选（不给 ⇒ 保持旧行为，见各自文档）。
    pub export_wav: Option<PathBuf>,
    /// `--wav-bit-depth <16|24|32f>`：`--export-wav` 的位深。
    ///
    /// 缺省 = **24**（与旧行为逐字一致）。16 / 24 走 TPDF 抖动（`yeban-render` 的
    /// 唯一量化实现），`32f` 是**浮点透传**（⛔ 不做抖动：抖动的定义域是"降位深"）。
    /// ⛔ 写 `32` 是用法错误 —— 本仓只有 32 位**浮点**一种 32 位容器，`32` 歧义。
    ///
    /// 只在给了 `--export-wav` 时才有意义：单独给 ⇒ 用法错误（[`run_batch`] 的组合检查，
    /// ⛔ 不静默忽略）。
    pub wav_bit_depth: Option<BitDepth>,
    /// `--wav-sample-rate <Hz>`：`--export-wav` 的输出采样率。
    ///
    /// 缺省（不给）= **跟随工程** `audio_config.sample_rate`（`[MODEL-AST-002]` 的唯一来源；
    /// 工程缺省 48 kHz）。⚠ 这与"固定 48000"不同，属**行为变更**，但内置三份夹具工程都是
    /// 48 kHz ⇒ 默认产物与旧行为**逐字节相同**；要旧口径请显式 `--wav-sample-rate 48000`。
    ///
    /// 取值必须是规范枚举之一（44.1/48/88.2/96/192 kHz），⛔ 不接受任意 Hz。
    pub wav_sample_rate: Option<SampleRate>,
    /// `--wav-seconds <秒>`：`--export-wav` 的**时长**（帧数 = `round(秒 × 采样率)`）。
    ///
    /// 缺省（不给）= **8 秒**（[`crate::export_wav::DEFAULT_WAV_SECONDS`]，与旧行为逐字一致）。
    /// ⚠ 本切片**没有**把缺省改成"工程可闻末端"：导出路径今天没有"工程有多长"的唯一权威
    /// 读数，那属**新能力**而不是接线；要更长的产物就显式给秒数。
    pub wav_seconds: Option<f32>,
    /// `--wav-frames <n>`：`--export-wav` 的**精确帧数**（与 MCP `render_master.frames`
    /// 同一量纲，用来让两侧的产物长度逐字节对齐）。
    ///
    /// 给了它就原样用，⛔ 不再做"秒 × 采样率"的取整换算；与 `--wav-seconds` **互斥**
    /// （同时给 ⇒ [`ParseError::WavFramesConflict`]，⛔ 不靠"谁优先"发明语义）。
    /// 取值必须是正的十进制整数（`0` 是**空产物**，按用法错误拒绝）。
    pub wav_frames: Option<u64>,
    /// `--limiter`：`--export-wav` 是否在**主总线推子之后**跑母线限制器
    /// （任务单 `#302`；默认 **关** ⇒ 与旧导出逐字节相同）。
    ///
    /// 这是**每次调用**的开关（⛔ 工程字段、⛔ `project.schema.json`）。
    /// 开启时限制器带 33 帧前瞻延迟，导出路径会**补偿**这 33 帧 ⇒ 输出长度与
    /// 时间轴不变，只有压限本身改变样本。
    pub wav_limiter: bool,
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
    ///
    /// ⭐ `#200`：这里的值是**用户在命令行里写的字面值**（`parse()` 不补后缀）；真正落盘的
    /// 目录**必然**以 `.logicx` 结尾 —— 补全发生在导出侧
    /// （[`crate::export_logic::logic_bundle_directory`]），回执的 `path=` 打印实际路径。
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
    /// `--enable-ui-mcp-http`：把 **UI 控制面**（14 条 `ui/*` 方法）挂成一个真的能 `curl` 的
    /// **环回**入口（`[ARCH-UI-004]` / `[MUST-GATE-009]`；实现在 `src/ui_mcp_serve.rs`）。
    ///
    /// 编译期那道开关是**非默认** feature [`UI_MCP_HTTP_FEATURE`]（默认构建里这个开关 =
    /// [`ParseError::UiMcpHttpNotCompiled`]，退出码 2，绝不静默忽略）。
    ///
    /// ## 为什么它**自己**就是一条无窗口路径
    ///
    /// `ui/*` 的执行面是 `LivePort<MainWindow>` —— Tier-1 **软件**光栅化
    /// （`MinimalSoftwareWindow` + `SoftwareRenderer`，`[MUST-GATE-015]` 允许的唯一像素路径）
    /// 加上 `i-slint-backend-testing` 的运行时控件树。而 `Tier1Window::install` 调用的
    /// `slint::platform::set_platform` 是**线程局部且每线程一次**，并且**必须在构造任何
    /// Slint 组件之前**完成。GUI 路径（`run_gui`）先 `MainWindow::new()` 把 winit 后端装上了，
    /// 因此那里**再也装不下** Tier-1 平台（`set_platform` 会返回 `Err`）。
    /// 结论：这个开关承载的进程形态是"**软件平台上的活控件树**"，也就是与
    /// `--headless-idle` 同族的那一档 —— 它不创建 OS 窗口、不进阻塞事件循环，
    /// 但**真的**有一棵被布局、被光栅化的控件树，事件注入真的落进窗口。
    ///
    /// `--idle-seconds` 可选：给了就服务到点自动停机（并释放监听口），
    /// 不给就一直服务到进程被终止（Ctrl-C / `kill`）。
    pub ui_mcp_http: bool,
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

/// 一个动作开关在**进程形态**上的归属。
///
/// 本类型是"哪些开关自己就把进程推离 GUI 路径"的**唯一事实源**的一半
/// （另一半是 [`ACTION_SWITCHES`]）：[`Options::batch()`] 与 [`Options::wants_gui()`]
/// 都从那张表派生，判据也遍历它 —— ⛔ 不再手抄第二张会漂移的清单
/// （本仓已经漂移过两张：`batch()` 曾漏掉 `--export-wav` 等六个动作开关，
/// 于是它们"文档写着跑过、真二进制上却开窗口什么都不做"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchPath {
    /// 给出这个开关 ⇒ 进程走**无窗口（批处理）路径**。
    Batch,
    /// 给出这个开关**自己**不选路径（进程仍走 GUI ⇒ 它的动作不会发生），
    /// 但动作只能在**无窗口路径**上执行 —— 由别的批处理开关（如 `--save-as`）带进去。
    ///
    /// ⭐ 今天**恰好**是两条导入（`--import-midi` / `--import-audio`），而这是**有意的
    /// 既存口径**：它**已经**写在 `--help`（`--import-audio` 那条"⚠ 只在**无窗口路径**
    /// 上执行"）里，并被单元判据
    /// `a_lone_import_audio_flag_selects_the_gui_path_so_the_import_never_runs` 钉住
    /// （`src/cli.rs` 判据区）。本枚举把这条例外**点名**在代码里，于是判据可以断言
    /// "例外就是这两条、且只有这两条"，而不是让它**静默缺席**在表外。
    WindowlessOnly,
}

/// [`ACTION_SWITCHES`] 表里的一项。
#[derive(Debug, Clone, Copy)]
pub struct ActionSwitch {
    /// 开关字面值（含前导 `--`），必须与 `--help` 里印的**逐字**相同
    /// （判据 B 用它去认示例行里"点了名"的动作开关）。
    pub name: &'static str,
    /// 这个开关在 [`Options`] 里"给了"吗。
    ///
    /// 用谓词而不是字段名，是因为两类字段的判法不同（`bool` 看真假、
    /// `Option` 看 `is_some`）—— 表只暴露一个统一形状。
    pub present: fn(&Options) -> bool,
    /// 判据用它把开关**设到**一份 `Options` 上：这样才能绕过 `parse()` 里的
    /// **编译期门禁**（`--export-als` / `--export-logic` / `--enable-ui-mcp-http`
    /// 在默认构建里解析不成），单独验证它们的**路由**归属。
    /// ⛔ 生产代码不用它：`parse()` 仍是唯一的用户入口。
    pub grant: fn(&mut Options),
    /// 给出这个开关时进程的路径归属。
    pub path: SwitchPath,
}

impl ActionSwitch {
    /// 给出本开关 ⇒ 是否走**无窗口（批处理）路径**。
    #[must_use]
    pub const fn selects_batch(&self) -> bool {
        matches!(self.path, SwitchPath::Batch)
    }
}

/// **动作开关**的唯一直表（名字 / 在场谓词 / 设置器 / 路径归属）。
///
/// 动作开关 = "给出它就点了一次名"的开关 ⇒ 它**自己**决定（或对两条导入明确
/// **不**决定）进程形态。修饰性开关（`--record-seconds` / `--record-source` /
/// `--midi-in-seconds` / `--midi-in-track` / `--project-sample` / `--theme` /
/// `--idle-seconds` / `--open`）不在表里：它们不点名动作，只给表里的动作补参数。
/// `--enable-mcp-http` 也**不在**表里：它要求**正在跑的 GUI 事件循环**（形态 A），
/// 与"选无窗口路径"是相反的一档。
///
/// ⛔ 加一个"自己决定进程形态"的开关 = 加一行这里。[`Options::batch()`] 只认这张表。
pub const ACTION_SWITCHES: &[ActionSwitch] = &[
    ActionSwitch {
        name: "--help",
        present: |options| options.help,
        grant: |options| options.help = true,
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--version",
        present: |options| options.version,
        grant: |options| options.version = true,
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--headless",
        present: |options| options.headless,
        grant: |options| options.headless = true,
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--dump-elements",
        present: |options| options.dump_elements,
        grant: |options| options.dump_elements = true,
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--print-shortcuts",
        present: |options| options.print_shortcuts,
        grant: |options| options.print_shortcuts = true,
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: PRINT_THEME_SWITCH,
        present: |options| options.print_theme,
        grant: |options| options.print_theme = true,
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--export-elements",
        present: |options| options.export_elements.is_some(),
        grant: |options| options.export_elements = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--export-midi",
        present: |options| options.export_midi.is_some(),
        grant: |options| options.export_midi = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    // `--export-wav` 与 `--export-midi` **同族**（`--help` 的 `--export-wav` 一条逐字
    // 写着"与 --export-midi 同族"）：它当然自己就能把进程推离 GUI 路径。
    ActionSwitch {
        name: "--export-wav",
        present: |options| options.export_wav.is_some(),
        grant: |options| options.export_wav = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: ALS_EXPORT_SWITCH,
        present: |options| options.export_als.is_some(),
        grant: |options| options.export_als = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: LOGIC_EXPORT_SWITCH,
        present: |options| options.export_logic.is_some(),
        grant: |options| options.export_logic = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--save-as",
        present: |options| options.save_as.is_some(),
        grant: |options| options.save_as = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    // 下面这些（`--record-wav` / `--sfz` / `--midi-in*`）的动作与守卫都住在
    // [`run_batch`] 里（`--midi-in*` 的"必须与 `--midi-in` 一起给"四条守卫见
    // `run_batch` 的组合性检查块）⇒ 它们单独给时**必须**走到 `run_batch`，
    // 否则那些守卫在 GUI 路径上根本不存在。
    ActionSwitch {
        name: "--record-wav",
        present: |options| options.record_wav.is_some(),
        grant: |options| options.record_wav = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--sfz",
        present: |options| options.sfz.is_some(),
        grant: |options| options.sfz = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    // ⭐ 任务单 `#210`：装乐器是**编辑**（不是只读装载）⇒ 自己就把进程推离 GUI 路径。
    ActionSwitch {
        name: "--track-sampler",
        present: |options| options.track_sampler.is_some(),
        grant: |options| options.track_sampler = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--midi-in",
        present: |options| options.midi_in.is_some(),
        grant: |options| options.midi_in = Some("probe".to_owned()),
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--midi-in-render",
        present: |options| options.midi_in_render.is_some(),
        grant: |options| options.midi_in_render = Some(PathBuf::from("probe")),
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--midi-in-write-notes",
        present: |options| options.midi_in_write_notes,
        grant: |options| options.midi_in_write_notes = true,
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: "--headless-idle",
        present: |options| options.headless_idle,
        grant: |options| options.headless_idle = true,
        path: SwitchPath::Batch,
    },
    ActionSwitch {
        name: UI_MCP_HTTP_SWITCH,
        present: |options| options.ui_mcp_http,
        grant: |options| options.ui_mcp_http = true,
        path: SwitchPath::Batch,
    },
    // ⭐ **点名**的例外（`SwitchPath::WindowlessOnly`）：给出它们**自己**不选路径，
    // 但动作只在无窗口路径上执行。这条口径已经在 `--help` 与既有判据里。
    ActionSwitch {
        name: "--import-midi",
        present: |options| options.import_midi.is_some(),
        grant: |options| options.import_midi = Some(PathBuf::from("probe")),
        path: SwitchPath::WindowlessOnly,
    },
    ActionSwitch {
        name: "--import-audio",
        present: |options| options.import_audio.is_some(),
        grant: |options| options.import_audio = Some(PathBuf::from("probe")),
        path: SwitchPath::WindowlessOnly,
    },
];

impl Options {
    /// 动作开关的唯一直表（见 [`ACTION_SWITCHES`] / [`ActionSwitch`]）。
    ///
    /// [`Self::batch()`] 与判据都遍历它；⛔ 不要再手抄第二份会漂移的清单。
    #[must_use]
    pub const fn action_switches() -> &'static [ActionSwitch] {
        ACTION_SWITCHES
    }

    /// 是否是**无窗口**路径（不创建 OS 窗口、不进事件循环）。
    ///
    /// **唯一事实源**是 [`ACTION_SWITCHES`]（见 [`Self::action_switches()`]）：
    /// 本函数 = "表里有没有一项 `path == Batch` 且**在场**"。
    /// ⛔ 这里**不再**手抄任何开关名 —— 手抄的那一份曾经漏掉 `--export-wav` /
    /// `--record-wav` / `--sfz` / `--midi-in` / `--midi-in-render` /
    /// `--midi-in-write-notes` 六个动作开关（它们的动作与守卫都住在 [`run_batch`] 里，
    /// 于是"单独给 = 开窗口什么都不做"）。加开关只改那张表。
    ///
    /// 两条导入（`--import-midi` / `--import-audio`）在表里被**点名**为
    /// [`SwitchPath::WindowlessOnly`]：它们**自己**不选路径（这是 `--help` 与既有判据
    /// 钉住的既存口径），必须由别的批处理开关（如 `--save-as`）带进无窗口路径。
    ///
    /// `--headless-idle` **也**在这一档里（它确实不建 OS 窗口），但它是**唯一**
    /// 会构造 Slint 对象的无窗口开关 ⇒ `main.rs` 必须在 `wants_gui()` 之前先看它，
    /// 而 [`run_batch`] 见到它会明确报错而不是静默降级（见那里的守卫）。
    #[must_use]
    pub fn batch(&self) -> bool {
        Self::action_switches()
            .iter()
            .any(|switch| switch.selects_batch() && (switch.present)(self))
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
    /// `--midi-in-seconds` 的取值不是十进制数。
    MidiInSecondsInvalid(String),
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
    /// 给了 `--enable-ui-mcp-http`，但这次构建**没有**编译 [`UI_MCP_HTTP_FEATURE`]。
    ///
    /// 与 [`Self::McpHttpNotCompiled`] 逐字同一条纪律（也是同一个红线的第一道开关）：
    /// 用户要开一个能 `curl` 的 UI 控制面，而二进制里根本没有那段监听代码 ——
    /// 那必须是一次**响亮的**用法错误，而不是"以为开了其实没开"。
    UiMcpHttpNotCompiled,
    /// `--enable-ui-mcp-http` 与一个它承载不了的开关同时给。
    ///
    /// 两类：① 会写盘 / 导出的批处理开关（会被静默丢掉，与 [`Self::IdleConflict`] 同款）；
    /// ② `--enable-mcp-http`（领域 MCP 要求**正在跑的 GUI 事件循环**，而 UI 控制面的
    /// 执行面只能装在软件光栅化平台上 —— 两者不在同一个进程形态里；见 `src/ui_mcp_serve.rs`）。
    UiMcpHttpConflict(&'static str),
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
    /// 给了 `--inspect-logic`，但这次构建**没有**编译 [`LOGIC_INSPECT_FEATURE`]（`#143`）。
    ///
    /// 与 [`Self::LogicNotCompiled`] 同一条纪律：默认构建里 `yeban-render` 的
    /// `logic_read` 模块不存在，用户要读的是 demo 字节而二进制里根本没有那个读取器 ——
    /// 那必须是一次**点名 feature 的**用法错误（退出码 [`EXIT_USAGE`]），⛔ 不静默忽略。
    InspectLogicNotCompiled,
    /// `--inspect-logic` 与别的参数组合了（`#143`）。
    ///
    /// 它是**独立的只读探针**：不需要工程，也不执行任何其它动作。接受组合只会把
    /// 被组合的那个开关**静默丢掉**（例如 `--save-as` 什么都没保存却退出 0）——
    /// 那正是本仓库最忌讳的假绿。payload 是第一个多余的参数。
    InspectLogicConflict(String),
    /// 给了 `--import-logic`，但这次构建**没有**编译 [`IMPORT_LOGIC_FEATURE`]（`#149`）。
    ///
    /// 与 [`Self::InspectLogicNotCompiled`] 同一条纪律：默认构建里 `yeban-render` 的
    /// `logic_read` 与 app 的 [`crate::import_logic`] 都不存在，用户要的是一个 `.yeban`
    /// 文件而二进制里根本没有那个导入器 —— 那必须是一次**点名 feature 的**用法错误
    /// （退出码 [`EXIT_USAGE`]），⛔ 不静默忽略。
    ImportLogicNotCompiled,
    /// `--import-logic` 与别的参数组合了（`#149`）。
    ///
    /// 它是一条**独立**命令：读一个 demo、写 `--save-as` 指定的容器，不打开当前工程、
    /// 不导出任何别的东西。接受组合只会把被组合的开关**静默丢掉**。payload 是第一个
    /// 多余的参数。唯一允许共存的开关是 `--headless`（本命令本来就不建窗口）。
    ImportLogicConflict(String),
    /// `--import-logic` 没配 `--save-as <out.yeban>`（`#149`）。
    ///
    /// 导入的产出**只能**是 `--save-as` 那一个路径：不给它就没有"导到哪里"，
    /// 而"默认写到 demo 旁边"会让一条只读 demo 的命令突然变成写 demo 目录 ——
    /// 本仓库不猜落点（与 [`Self::MigrateGrooveOutputMissing`] 同一条）。
    ImportLogicOutputMissing,
    /// `--migrate-groove` 没配 `--save-as <new.yeban>`（`#146`）。
    ///
    /// 迁移器的产出**只能**是 `--save-as` 那一个路径：不给它就没有"迁移到哪里"，
    /// 而"默认写回旧文件旁"会让一条只读命令突然变成写命令 —— 本仓库不猜落点。
    MigrateGrooveOutputMissing,
    /// `--migrate-groove` 与别的参数组合了（`#146`）。
    ///
    /// 它是一条**独立**的批处理命令：读旧文件、写新容器，不打开当前工程、不导出任何
    /// 别的东西。接受组合只会把被组合的那个开关**静默丢掉**（例如
    /// `--migrate-groove a.groove --save-as b.yeban --export-midi x.mid` 里 `.mid`
    /// 根本不会写）。payload 是第一个多余的参数。
    MigrateGrooveConflict(String),
    /// `--migrate-groove` 的输入与 `--save-as` 输出是同一个路径（`#146`）。
    ///
    /// 旧文件是**只读**输入：原地改写会在迁移失败时同时失去旧工程与新工程。
    /// 因此这在解析阶段就是用法错误，⛔ 不等到写盘时才判。
    MigrateGrooveSamePath,
    /// ⭐ 任务单 `#211`：`--probe-instruments` 与别的参数组合了。
    ///
    /// 它是一条**独立**的只读读数：不打开工程、不进 GUI、不写任何文件。接受组合只会把
    /// 被组合的开关**静默丢掉**（例如 `--probe-instruments --save-as x` 里 `x` 根本不会写）
    /// —— 那正是本仓库最忌讳的假绿。payload 是第一个多余的参数。
    ProbeInstrumentsConflict(String),
    /// `--wav-bit-depth` 的取值不是 `16` / `24` / `32f`。
    ///
    /// ⛔ 不接受 `32`：本仓只有 32-bit **浮点**一种 32 位容器，而 `32` 这个写法在
    /// "32 位整数"与"32 位浮点"之间是**歧义**的 —— 猜一个就是发明语义。
    WavBitDepthInvalid(String),
    /// `--wav-sample-rate` 的取值不在规范枚举 `{44100,48000,88200,96000,192000}` 里
    /// （`[MODEL-AST-002]`）。
    ///
    /// ⛔ 不静默接受任意 Hz：`yeban-model` 的采样率是**枚举**（工程的唯一权威来源），
    /// 放行一个枚举外的速率会产生"工程写不回这一份配置"的产物。
    WavSampleRateInvalid(String),
    /// `--wav-seconds` 的取值不是**正的有限**十进制秒数。
    WavSecondsInvalid(String),
    /// `--wav-frames` 的取值不是**正的**十进制整数帧数。
    ///
    /// ⛔ 不接受 `0`：零帧是**空产物**，按用法错误拒绝比写一个只有头的 WAV 更诚实。
    WavFramesInvalid(String),
    /// `--wav-frames` 与 `--wav-seconds` 同时给了。
    ///
    /// 一个给**精确帧数**、一个给**秒数换算**，同时给必然有一个被静默丢掉 ——
    /// 本仓不接受这种组合（⛔ 不靠"谁优先"发明语义）。
    WavFramesConflict,
    /// ⭐ 任务单 `#287`：`--sampler-track` 单独给（没配 `--track-sampler`）。
    ///
    /// 它只是 `--track-sampler` 的**目标选择器**，没有主语时什么也不选。⛔ 不接受
    /// "独立给"：`--sampler-track` 自己不选无窗口路径 ⇒ 静默进 GUI 路径、彻底
    /// 什么也不做（本仓最忌讳的假绿）。文本见 [`SAMPLER_TRACK_PAIRING_ERROR`]。
    SamplerTrackWithoutTrackSampler,
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
            Self::MidiInSecondsInvalid(value) => write!(
                formatter,
                "`--midi-in-seconds` 的取值 `{value}` 不是十进制秒数"
            ),
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
            Self::UiMcpHttpNotCompiled => write!(
                formatter,
                "`{UI_MCP_HTTP_SWITCH}` 需要本次构建带 `--features {UI_MCP_HTTP_FEATURE}` \
                 (它就是 [MUST-GATE-009] / AGENTS.md §2 红线 6 的第一道开关: \
                 默认构建里既没有 UI 控制面这段监听代码, 也没有 yeban-ui-mcp 这个依赖边)"
            ),
            Self::UiMcpHttpConflict(other) => write!(
                formatter,
                "`{UI_MCP_HTTP_SWITCH}` 不能与 `{other}` 组合 —— 本开关的进程形态是\
                 \"软件光栅化平台上的一棵活控件树\" (它承载 ui/* 的 Tier-1 执行面), \
                 接受这个组合只会让 `{other}` 被静默丢掉, 或者让人以为另一个控制面也起来了"
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
            Self::InspectLogicNotCompiled => write!(
                formatter,
                "`{LOGIC_INSPECT_SWITCH}` 需要本次构建带 `--features {LOGIC_INSPECT_FEATURE}` \
                 (只读探针默认关: AGENTS.md §2 红线 6 同一条纪律; ⚠ 它**不是导入器** —— \
                 不写工程、不写 demo、不产出可导入文档, 只打印 demo 的实测读数)"
            ),
            Self::InspectLogicConflict(other) => write!(
                formatter,
                "`{LOGIC_INSPECT_SWITCH}` 是独立的只读探针, 不能与 `{other}` 组合 —— \
                 接受它只会把 `{other}` 静默丢掉"
            ),
            Self::ImportLogicNotCompiled => write!(
                formatter,
                "`{IMPORT_LOGIC_SWITCH}` 需要本次构建带 `--features {IMPORT_LOGIC_FEATURE}` \
                 (实验性 .logicx 导入默认关: AGENTS.md §2 红线 6 同一条纪律; 它读 demo 字节 \
                 建 `.yeban` 工程, 输出只写 `--save-as`)"
            ),
            Self::ImportLogicConflict(other) => write!(
                formatter,
                "`{IMPORT_LOGIC_SWITCH}` 是独立的导入命令, 不能与 `{other}` 组合 —— \
                 接受它只会把 `{other}` 静默丢掉"
            ),
            Self::ImportLogicOutputMissing => write!(
                formatter,
                "`{IMPORT_LOGIC_SWITCH}` 必须配 `--save-as <out.yeban>` 指定工程落点 \
                 (⛔ 不默认写到 demo 旁边: demo 只读, 落点必须由调用方给)"
            ),
            Self::MigrateGrooveOutputMissing => write!(
                formatter,
                "`{MIGRATE_GROOVE_SWITCH}` 必须配 `--save-as <new.yeban>` 指定新容器落点 \
                 (⛔ 不默认写回旧文件旁: 旧文件只读, 落点必须由调用方给)"
            ),
            Self::MigrateGrooveConflict(other) => write!(
                formatter,
                "`{MIGRATE_GROOVE_SWITCH}` 是独立的批处理命令, 不能与 `{other}` 组合 —— \
                 接受它只会把 `{other}` 静默丢掉"
            ),
            Self::MigrateGrooveSamePath => write!(
                formatter,
                "`{MIGRATE_GROOVE_SWITCH}` 的输入与 `--save-as` 输出是同一个路径 ⇒ 拒绝 \
                 (旧文件只读, 绝不原地改写)"
            ),
            Self::ProbeInstrumentsConflict(other) => write!(
                formatter,
                "`{PROBE_INSTRUMENTS_SWITCH}` 是独立的只读读数, 不能与 `{other}` 组合 —— \
                 接受它只会把 `{other}` 静默丢掉"
            ),
            Self::WavBitDepthInvalid(value) => write!(
                formatter,
                "`--wav-bit-depth` 的取值 `{value}` 非法 (可用: 16|24|32f; \
                 32 位在本仓只有**浮点**一种容器, 所以写 `32f`, `32` 是歧义的)"
            ),
            Self::WavSampleRateInvalid(value) => write!(
                formatter,
                "`--wav-sample-rate` 的取值 `{value}` 非法 (可用: {} ; 这是 [MODEL-AST-002] \
                 的采样率枚举 —— 工程侧写不回枚举外的值)",
                SampleRate::ALL
                    .iter()
                    .map(|rate| rate.hz().to_string())
                    .collect::<Vec<_>>()
                    .join("|")
            ),
            Self::WavSecondsInvalid(value) => write!(
                formatter,
                "`--wav-seconds` 的取值 `{value}` 非法 (需要正的有限秒数)"
            ),
            Self::WavFramesInvalid(value) => write!(
                formatter,
                "`--wav-frames` 的取值 `{value}` 非法 (需要正的十进制整数帧数; 0 帧是空产物)"
            ),
            Self::WavFramesConflict => write!(
                formatter,
                "`--wav-frames` 与 `--wav-seconds` 互斥 —— 一个给精确帧数、一个给秒数换算, \
                 同时给必然有一个被静默丢掉"
            ),
            Self::SamplerTrackWithoutTrackSampler => {
                formatter.write_str(SAMPLER_TRACK_PAIRING_ERROR)
            }
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
            "--record-wav" => {
                let value = take_value("--record-wav", inline, args, &mut cursor)?;
                set_once(
                    &mut options.record_wav,
                    "--record-wav",
                    PathBuf::from(value),
                )?;
            }
            "--record-source" => {
                let value = take_value("--record-source", inline, args, &mut cursor)?;
                if value != "project" && value != "tone" {
                    return Err(ParseError::UnexpectedValue("--record-source"));
                }
                options.record_source = Some(value.to_owned());
            }
            "--record-seconds" => {
                let value = take_value("--record-seconds", inline, args, &mut cursor)?;
                let seconds: f32 = value
                    .parse()
                    .map_err(|_| ParseError::UnexpectedValue("--record-seconds"))?;
                options.record_seconds = Some(seconds);
            }
            "--sfz" => {
                let value = take_value("--sfz", inline, args, &mut cursor)?;
                set_once(&mut options.sfz, "--sfz", PathBuf::from(value))?;
            }
            "--track-sampler" => {
                let value = take_value("--track-sampler", inline, args, &mut cursor)?;
                set_once(
                    &mut options.track_sampler,
                    "--track-sampler",
                    PathBuf::from(value),
                )?;
            }
            "--sampler-track" => {
                let value = take_value("--sampler-track", inline, args, &mut cursor)?;
                set_once_string(&mut options.sampler_track, "--sampler-track", value)?;
            }
            "--midi-in" => {
                let value = take_value("--midi-in", inline, args, &mut cursor)?;
                set_once_string(&mut options.midi_in, "--midi-in", value)?;
            }
            "--midi-in-seconds" => {
                let value = take_value("--midi-in-seconds", inline, args, &mut cursor)?;
                let seconds = value
                    .parse::<f32>()
                    .map_err(|_| ParseError::MidiInSecondsInvalid(value.clone()))?;
                set_once_seconds(&mut options.midi_in_seconds, "--midi-in-seconds", seconds)?;
            }
            "--midi-in-write-notes" => {
                reject_inline("--midi-in-write-notes", inline)?;
                options.midi_in_write_notes = true;
                cursor += 1;
            }
            "--midi-in-render" => {
                let value = take_value("--midi-in-render", inline, args, &mut cursor)?;
                set_once(
                    &mut options.midi_in_render,
                    "--midi-in-render",
                    PathBuf::from(value),
                )?;
            }
            "--midi-in-track" => {
                let value = take_value("--midi-in-track", inline, args, &mut cursor)?;
                set_once_string(&mut options.midi_in_track, "--midi-in-track", value)?;
            }
            "--import-midi" => {
                let value = take_value("--import-midi", inline, args, &mut cursor)?;
                set_once(
                    &mut options.import_midi,
                    "--import-midi",
                    PathBuf::from(value),
                )?;
            }
            "--import-audio" => {
                let value = take_value("--import-audio", inline, args, &mut cursor)?;
                set_once(
                    &mut options.import_audio,
                    "--import-audio",
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
            "--export-wav" => {
                let value = take_value("--export-wav", inline, args, &mut cursor)?;
                set_once(
                    &mut options.export_wav,
                    "--export-wav",
                    PathBuf::from(value),
                )?;
            }
            // `--wav-*`：只改 `--export-wav` 的产物形状，⛔ 不选路径
            // （与 `--midi-in-seconds` / `--idle-seconds` 同档的**修饰**开关，
            //  因此 ⛔ 不进 `ACTION_SWITCHES`）。
            "--wav-bit-depth" => {
                let value = take_value("--wav-bit-depth", inline, args, &mut cursor)?;
                let depth = match value.as_str() {
                    "16" => BitDepth::Int16,
                    "24" => BitDepth::Int24,
                    "32f" => BitDepth::Float32,
                    // ⛔ `32` 不静默当成 `32f`：那是"32 位整数 vs 浮点"的歧义，
                    // 猜一个就是发明语义。
                    _ => return Err(ParseError::WavBitDepthInvalid(value)),
                };
                if options.wav_bit_depth.is_some() {
                    return Err(ParseError::DuplicateOption("--wav-bit-depth"));
                }
                options.wav_bit_depth = Some(depth);
            }
            "--wav-sample-rate" => {
                let value = take_value("--wav-sample-rate", inline, args, &mut cursor)?;
                // 取值域 = `yeban-model` 的采样率**枚举**（[MODEL-AST-002]）：
                // ⛔ 不放行枚举外的 Hz（那会产出工程侧写不回的配置）。
                let rate = value
                    .parse::<u32>()
                    .ok()
                    .and_then(|hz| SampleRate::from_hz(hz).ok())
                    .ok_or_else(|| ParseError::WavSampleRateInvalid(value.clone()))?;
                if options.wav_sample_rate.is_some() {
                    return Err(ParseError::DuplicateOption("--wav-sample-rate"));
                }
                options.wav_sample_rate = Some(rate);
            }
            "--wav-seconds" => {
                let value = take_value("--wav-seconds", inline, args, &mut cursor)?;
                let seconds = value
                    .parse::<f32>()
                    .map_err(|_| ParseError::WavSecondsInvalid(value.clone()))?;
                if !seconds.is_finite() || seconds <= 0.0 {
                    return Err(ParseError::WavSecondsInvalid(value));
                }
                if options.wav_seconds.is_some() {
                    return Err(ParseError::DuplicateOption("--wav-seconds"));
                }
                if options.wav_frames.is_some() {
                    return Err(ParseError::WavFramesConflict);
                }
                options.wav_seconds = Some(seconds);
            }
            "--wav-frames" => {
                let value = take_value("--wav-frames", inline, args, &mut cursor)?;
                let frames = value
                    .parse::<u64>()
                    .map_err(|_| ParseError::WavFramesInvalid(value.clone()))?;
                if frames == 0 {
                    return Err(ParseError::WavFramesInvalid(value));
                }
                if options.wav_frames.is_some() {
                    return Err(ParseError::DuplicateOption("--wav-frames"));
                }
                if options.wav_seconds.is_some() {
                    return Err(ParseError::WavFramesConflict);
                }
                options.wav_frames = Some(frames);
            }
            "--limiter" => {
                // 布尔修饰开关：⛔ 不吃值（与 `--wav-*` 同档，只改 `--export-wav`
                // 的产物形状）。重复给 ⇒ 明确报错，⛔ 不静默。
                if options.wav_limiter {
                    return Err(ParseError::DuplicateOption("--limiter"));
                }
                options.wav_limiter = true;
                cursor += 1;
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
            "--enable-ui-mcp-http" => {
                reject_inline(UI_MCP_HTTP_SWITCH, inline)?;
                options.ui_mcp_http = true;
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
        // `--enable-ui-mcp-http` 是**另一档**同样"真的建树"的无窗口形态，
        // 它也吃 `--idle-seconds`（给了就服务到点自动停机）。
        (false, Some(_)) if !options.ui_mcp_http => {
            return Err(ParseError::IdleSecondsWithoutHeadlessIdle);
        }
        _ => {}
    }
    if options.headless_idle {
        for (given, name) in [
            (options.dump_elements, "--dump-elements"),
            (options.print_shortcuts, "--print-shortcuts"),
            (options.export_elements.is_some(), "--export-elements"),
            (options.import_midi.is_some(), "--import-midi"),
            (options.import_audio.is_some(), "--import-audio"),
            (options.midi_in.is_some(), "--midi-in"),
            (options.export_midi.is_some(), "--export-midi"),
            (options.export_wav.is_some(), "--export-wav"),
            (options.export_als.is_some(), ALS_EXPORT_SWITCH),
            (options.export_logic.is_some(), LOGIC_EXPORT_SWITCH),
            (options.save_as.is_some(), "--save-as"),
        ] {
            if given {
                return Err(ParseError::IdleConflict(name));
            }
        }
    }
    // ⚠ 本块必须排在 `--enable-mcp-http` 那块**之前**：`--enable-ui-mcp-http` 在默认构建里
    // 是一次"点名 feature 的用法错误"。排在后面的话，`--enable-ui-mcp-http --enable-mcp-http`
    // 会被领域那块以 `McpHttpNotCompiled` 拒掉 —— 而在"只编了 in-process-mcp"的构建里
    // 领域那块会**通过**（它不认 UI 开关），于是这个开关就被**静默忽略**了。
    // 顺序在这里是契约：先判"这个开关在这个构建里存不存在"，再判"能不能与别的开关组合"。
    // `--enable-ui-mcp-http` 的两条前置（与 `--enable-mcp-http` 同款，逐条理由见
    // `src/ui_mcp_serve.rs` 的模块文档）：
    //   ① 编译期那道开关（非默认 feature `ui-mcp-http`）；
    //   ② 组合面 —— 会写盘 / 导出的开关会被**静默丢掉**，而 `--enable-mcp-http`
    //      与它不在同一个进程形态里（领域 MCP 要 GUI 事件循环，UI 控制面要软件平台）。
    if options.ui_mcp_http {
        if !cfg!(feature = "ui-mcp-http") {
            return Err(ParseError::UiMcpHttpNotCompiled);
        }
        for (given, name) in [
            (options.headless, "--headless"),
            (options.dump_elements, "--dump-elements"),
            (options.print_shortcuts, "--print-shortcuts"),
            (options.export_elements.is_some(), "--export-elements"),
            (options.import_midi.is_some(), "--import-midi"),
            (options.import_audio.is_some(), "--import-audio"),
            (options.midi_in.is_some(), "--midi-in"),
            (options.export_midi.is_some(), "--export-midi"),
            (options.export_wav.is_some(), "--export-wav"),
            (options.export_als.is_some(), ALS_EXPORT_SWITCH),
            (options.export_logic.is_some(), LOGIC_EXPORT_SWITCH),
            (options.save_as.is_some(), "--save-as"),
            (options.enable_mcp_http, MCP_HTTP_SWITCH),
        ] {
            if given {
                return Err(ParseError::UiMcpHttpConflict(name));
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
            (options.import_midi.is_some(), "--import-midi"),
            (options.import_audio.is_some(), "--import-audio"),
            (options.midi_in.is_some(), "--midi-in"),
            (options.export_midi.is_some(), "--export-midi"),
            (options.export_wav.is_some(), "--export-wav"),
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
    // ⭐ 任务单 `#287`：`--sampler-track` 只是 `--track-sampler` 的**目标选择器**。
    // 单独给 ⇒ 用法错误。这道检查放在**解析期**（而不是 `run_batch` 里）：后者只在
    // 无窗口路径上跑，而 `--sampler-track` 自己不选路径 —— 放在那里会让"单独给"
    // 悄悄进 GUI 路径、什么也不做（本仓最忌讳的假绿）。`run_batch` 仍留第二道守卫
    // （`Options` 的字段是公开的，判据可以直接构造组合）。
    if options.sampler_track.is_some() && options.track_sampler.is_none() {
        return Err(ParseError::SamplerTrackWithoutTrackSampler);
    }
    Ok(options)
}

/// 解析**独立**的 `--inspect-logic <dir>`（`#143`）。
///
/// 返回：
/// - `None`：argv 里没有这个开关（照常走 [`parse`]）；
/// - `Some(Err(_))`：开关在但用法不对 / 本次构建没有编译它；
/// - `Some(Ok(dir))`：可以执行只读探针。
///
/// 为什么独立于 [`Options`]：探针不需要工程，也不执行任何其它动作。它**只接受**
/// 这一个开关（连同它的取值）—— 与别的参数组合 = 用法错误，⛔ 不静默丢掉它们
/// （否则 `--inspect-logic ... --save-as x` 会"成功"却什么都没保存）。
/// `--help` / `--version` 是短路命令：出现时返回 `None`，让 [`parse`] 按既有契约先打印。
#[must_use]
pub fn parse_inspect_logic(args: &[String]) -> Option<Result<PathBuf, ParseError>> {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return None;
    }

    let mut consumed: Vec<usize> = Vec::new();
    let mut target: Option<PathBuf> = None;
    for (index, argument) in args.iter().enumerate() {
        let inline = argument
            .strip_prefix(LOGIC_INSPECT_SWITCH)
            .and_then(|rest| rest.strip_prefix('='));
        if argument != LOGIC_INSPECT_SWITCH && inline.is_none() {
            continue;
        }
        if !consumed.is_empty() {
            return Some(Err(ParseError::DuplicateOption(LOGIC_INSPECT_SWITCH)));
        }
        consumed.push(index);
        match inline {
            Some(value) if !value.is_empty() => target = Some(PathBuf::from(value)),
            Some(_) => return Some(Err(ParseError::MissingValue(LOGIC_INSPECT_SWITCH))),
            None => match args.get(index + 1) {
                Some(value) if !value.starts_with('-') => {
                    target = Some(PathBuf::from(value));
                    consumed.push(index + 1);
                }
                _ => return Some(Err(ParseError::MissingValue(LOGIC_INSPECT_SWITCH))),
            },
        }
    }

    if consumed.is_empty() {
        return None;
    }
    if let Some(extra) = args
        .iter()
        .enumerate()
        .find(|(index, _)| !consumed.contains(index))
        .map(|(_, argument)| argument.clone())
    {
        return Some(Err(ParseError::InspectLogicConflict(extra)));
    }
    let Some(dir) = target else {
        return Some(Err(ParseError::MissingValue(LOGIC_INSPECT_SWITCH)));
    };
    if !cfg!(feature = "experimental-logic-import") {
        return Some(Err(ParseError::InspectLogicNotCompiled));
    }
    Some(Ok(dir))
}

/// 解析**独立**的 `--import-logic <demo> --save-as <out.yeban>`（`#149` / 刀 2）。
///
/// 返回：
/// - `None`：argv 里没有这个开关（照常走 [`parse`]）；
/// - `Some(Err(_))`：开关在但用法不对 / 本次构建没有编译它；
/// - `Some(Ok((input, output)))`：可以执行真导入。
///
/// 为什么独立于 [`Options`]：导入器不需要当前工程，也不该与任何其它动作组合 ——
/// 组合只会把另一个开关**静默丢掉**。唯一允许共存的开关是 `--headless`
/// （它描述"不建窗口"，而本命令本来就不建窗口；接受它不是忽略，而是它的语义已经成立）。
/// 与 `--migrate-groove` 同一形状：输入只读、输出只写 `--save-as`。
#[must_use]
pub fn parse_import_logic(args: &[String]) -> Option<Result<(PathBuf, PathBuf), ParseError>> {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return None;
    }
    let present = args.iter().any(|argument| {
        argument == IMPORT_LOGIC_SWITCH
            || argument
                .strip_prefix(IMPORT_LOGIC_SWITCH)
                .is_some_and(|rest| rest.starts_with('='))
    });
    if !present {
        return None;
    }

    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut cursor = 0;
    while cursor < args.len() {
        let raw = args[cursor].as_str();
        let (name, inline) = match raw.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (raw, None),
        };
        match name {
            IMPORT_LOGIC_SWITCH => {
                if input.is_some() {
                    return Some(Err(ParseError::DuplicateOption(IMPORT_LOGIC_SWITCH)));
                }
                let value = match inline {
                    Some(value) if !value.is_empty() => value,
                    Some(_) => return Some(Err(ParseError::MissingValue(IMPORT_LOGIC_SWITCH))),
                    None => {
                        cursor += 1;
                        match args.get(cursor) {
                            Some(value) if !value.starts_with('-') => value.clone(),
                            _ => {
                                return Some(Err(ParseError::MissingValue(IMPORT_LOGIC_SWITCH)));
                            }
                        }
                    }
                };
                input = Some(PathBuf::from(value));
                cursor += 1;
            }
            "--save-as" => {
                if output.is_some() {
                    return Some(Err(ParseError::DuplicateOption("--save-as")));
                }
                let value = match inline {
                    Some(value) if !value.is_empty() => value,
                    Some(_) => return Some(Err(ParseError::MissingValue("--save-as"))),
                    None => {
                        cursor += 1;
                        match args.get(cursor) {
                            Some(value) if !value.starts_with('-') => value.clone(),
                            _ => return Some(Err(ParseError::MissingValue("--save-as"))),
                        }
                    }
                };
                output = Some(PathBuf::from(value));
                cursor += 1;
            }
            "--headless" => {
                if inline.is_some() {
                    return Some(Err(ParseError::UnexpectedValue("--headless")));
                }
                cursor += 1;
            }
            other => return Some(Err(ParseError::ImportLogicConflict(other.to_owned()))),
        }
    }

    let Some(input) = input else {
        return Some(Err(ParseError::MissingValue(IMPORT_LOGIC_SWITCH)));
    };
    let Some(output) = output else {
        return Some(Err(ParseError::ImportLogicOutputMissing));
    };
    if !cfg!(feature = "experimental-logic-import") {
        return Some(Err(ParseError::ImportLogicNotCompiled));
    }
    Some(Ok((input, output)))
}

/// 解析**独立**的 `--migrate-groove <old.groove> --save-as <new.yeban>`（`#146`）。
///
/// 返回：
/// - `None`：argv 里没有这个开关（照常走 [`parse`]）；
/// - `Some(Err(_))`：开关在但用法不对；
/// - `Some(Ok((input, output)))`：可以执行迁移。
///
/// 为什么独立于 [`Options`]：迁移器不需要当前工程，也不该与任何其它动作组合 ——
/// 组合只会把另一个开关**静默丢掉**。唯一允许共存的开关是 `--headless`
/// （它描述"不建窗口"，而本命令本来就不建窗口；接受它不是忽略，而是它的语义已经成立）。
#[must_use]
pub fn parse_migrate_groove(args: &[String]) -> Option<Result<(PathBuf, PathBuf), ParseError>> {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return None;
    }
    let present = args.iter().any(|argument| {
        argument == MIGRATE_GROOVE_SWITCH
            || argument
                .strip_prefix(MIGRATE_GROOVE_SWITCH)
                .is_some_and(|rest| rest.starts_with('='))
    });
    if !present {
        return None;
    }

    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut cursor = 0;
    while cursor < args.len() {
        let raw = args[cursor].as_str();
        let (name, inline) = match raw.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (raw, None),
        };
        match name {
            MIGRATE_GROOVE_SWITCH => {
                if input.is_some() {
                    return Some(Err(ParseError::DuplicateOption(MIGRATE_GROOVE_SWITCH)));
                }
                let value = match inline {
                    Some(value) if !value.is_empty() => value,
                    Some(_) => {
                        return Some(Err(ParseError::MissingValue(MIGRATE_GROOVE_SWITCH)));
                    }
                    None => {
                        cursor += 1;
                        match args.get(cursor) {
                            Some(value) if !value.starts_with('-') => value.clone(),
                            _ => {
                                return Some(Err(ParseError::MissingValue(MIGRATE_GROOVE_SWITCH)));
                            }
                        }
                    }
                };
                input = Some(PathBuf::from(value));
                cursor += 1;
            }
            "--save-as" => {
                if output.is_some() {
                    return Some(Err(ParseError::DuplicateOption("--save-as")));
                }
                let value = match inline {
                    Some(value) if !value.is_empty() => value,
                    Some(_) => return Some(Err(ParseError::MissingValue("--save-as"))),
                    None => {
                        cursor += 1;
                        match args.get(cursor) {
                            Some(value) if !value.starts_with('-') => value.clone(),
                            _ => return Some(Err(ParseError::MissingValue("--save-as"))),
                        }
                    }
                };
                output = Some(PathBuf::from(value));
                cursor += 1;
            }
            "--headless" => {
                if inline.is_some() {
                    return Some(Err(ParseError::UnexpectedValue("--headless")));
                }
                cursor += 1;
            }
            other => return Some(Err(ParseError::MigrateGrooveConflict(other.to_owned()))),
        }
    }

    let Some(input) = input else {
        return Some(Err(ParseError::MissingValue(MIGRATE_GROOVE_SWITCH)));
    };
    let Some(output) = output else {
        return Some(Err(ParseError::MigrateGrooveOutputMissing));
    };
    if input == output {
        return Some(Err(ParseError::MigrateGrooveSamePath));
    }
    Some(Ok((input, output)))
}

/// 解析**独立**的 `--probe-instruments`（⭐ 任务单 `#211`）。
///
/// 返回：
/// - `None`：argv 里没有这个开关（照常走 [`parse`]）；
/// - `Some(Err(_))`：开关在但与别的参数组合了（⛔ 不静默丢掉别的开关）；
/// - `Some(Ok(()))`：可以执行只读探针。
///
/// 为什么独立于 [`Options`]：探针不需要工程、不建窗口、不写文件，也不该与任何动作组合
/// —— 与 `--inspect-logic` / `--migrate-groove` 同一条纪律。
/// `--help` / `--version` 是短路命令：出现时返回 `None`，让 [`parse`] 按既有契约先打印。
#[must_use]
pub fn parse_probe_instruments(args: &[String]) -> Option<Result<(), ParseError>> {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return None;
    }
    let (first, inline) = args.iter().enumerate().find_map(|(index, argument)| {
        if argument == PROBE_INSTRUMENTS_SWITCH {
            Some((index, false))
        } else {
            argument
                .strip_prefix(PROBE_INSTRUMENTS_SWITCH)
                .filter(|rest| rest.starts_with('='))
                .map(|_| (index, true))
        }
    })?;
    // 这个开关**不接受取值**：`--probe-instruments=1` 是用法错误。
    if inline {
        return Some(Err(ParseError::UnexpectedValue(PROBE_INSTRUMENTS_SWITCH)));
    }
    if args.len() != 1 {
        let extra = args
            .iter()
            .enumerate()
            .find(|(index, _)| *index != first)
            .map(|(_, argument)| argument.clone())
            .unwrap_or_default();
        return Some(Err(ParseError::ProbeInstrumentsConflict(extra)));
    }
    Some(Ok(()))
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
fn set_once_string(
    slot: &mut Option<String>,
    name: &'static str,
    value: String,
) -> Result<(), ParseError> {
    if slot.is_some() {
        return Err(ParseError::DuplicateOption(name));
    }
    *slot = Some(value);
    Ok(())
}

fn set_once_seconds(
    slot: &mut Option<f32>,
    name: &'static str,
    value: f32,
) -> Result<(), ParseError> {
    if slot.is_some() {
        return Err(ParseError::DuplicateOption(name));
    }
    *slot = Some(value);
    Ok(())
}

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

impl Loaded {
    /// ⭐ **相对 SFZ 路径的工程目录根**（任务单 `#168`）：`--open` 那个文件的父目录。
    ///
    /// 工程不是从文件打开的（`--project-sample` / 不带 `--open`）⇒ `None` ⇒ 解析从
    /// **仓库根**开始，再到 cwd（`#164`／`#165` 的旧口径）。
    ///
    /// ⚠ ⛔ 这不是"工程里的一个字段"：模型只存路径字符串，目录是**读文件这一侧**的知识
    /// （工程必须可迁移，⛔ 不把绝对路径写进工程）。
    #[must_use]
    pub fn project_dir(&self) -> Option<PathBuf> {
        match &self.source {
            ProjectSource::File { path, .. } => path.parent().map(Path::to_path_buf),
            ProjectSource::Sample(_) => None,
        }
    }
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
    /// `--midi-in` 失败（无端口 / 歧义 / 宿主不可用 / 未启用 feature）。
    MidiIn {
        /// `midi-in` 层的原样裁决。
        source: crate::midi_in::MidiInError,
    },
    /// `--midi-in-write-notes` / `--midi-in-render` 失败（没有音符 / 没有目标音轨 /
    /// 提交被拒 / 引擎重建失败 / WAV 写出失败）。
    MidiNotes {
        /// `midi_notes` 层的原样裁决。
        source: crate::midi_notes::MidiNoteError,
    },
    /// `--record-wav` 失败（抽干 I/O / WAV 写出被拒 / 参数非法）。
    Record {
        /// 目标路径。
        path: PathBuf,
        /// 录音层的原样裁决。
        source: crate::record_wav::RecordError,
    },
    /// 用法错误（带说明）。
    Usage(String),
    /// `--sfz` 失败（读文件 / SFZ 文本非法）。
    Sfz {
        /// SFZ 文件路径。
        path: PathBuf,
        /// 装载层的原样裁决。
        source: crate::sfz_load::SfzLoadError,
    },
    /// ⭐ 任务单 `#210`：`--track-sampler` 失败（没有展示轨 / 路径不是 UTF-8 /
    /// 样本字节缺失 / 提交被拒）。
    ///
    /// ⭐ 任务单 `#287`：`--sampler-track` 的目标轨解析失败也走这一档（身份形状不对 /
    /// 工程里没有该身份 / 该身份是主总线），`reason` 逐字带可用身份清单。
    ///
    /// `reason` **逐字**来自 [`crate::bridge::instrument_install_ops_on`] 的拒绝原因
    /// （或 [`resolve_sampler_track`] 的裁决，或提交层的裁决）—— ⛔ 不换成一句通用错误。
    TrackSampler {
        /// 调用方给的 SFZ 路径。
        path: PathBuf,
        /// 拒绝 / 失败原因（逐字）。
        reason: String,
    },
    /// `--import-midi` 失败（读文件 / 解析 / 没有目标 / 提交被拒）。
    ImportMidi {
        /// 源文件路径。
        path: PathBuf,
        /// 导入层的原样裁决（含 `yeban-midi` 解析器的拒绝原因）。
        source: crate::import_midi::MidiImportError,
    },
    /// `--import-audio` 失败（读文件 / 超上限 / 解码 / 提交被拒）。
    ImportAudio {
        /// 源文件路径。
        path: PathBuf,
        /// 导入层的原样裁决（含 `yeban-render` WAV 读取器的拒绝原因）。
        source: crate::import_audio::AudioImportError,
    },
    /// `--export-midi` 失败（投影 / 编码 / 落盘）。
    ExportMidi {
        /// 目标路径。
        path: PathBuf,
        /// 导出层的原样裁决（含 `yeban-render` 编码器的拒绝原因）。
        source: MidiExportError,
    },
    /// `--export-wav` 失败（渲染 / 量化 / 落盘）。
    ExportWav {
        /// 目标路径。
        path: PathBuf,
        /// 导出层的原样裁决（含 `yeban-render` 渲染器与 WAV 写出器的拒绝原因）。
        source: crate::export_wav::WavExportError,
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
    /// `--inspect-logic` 失败（读目录 / 读 demo / 解析 demo，`#143`）。
    ///
    /// 只读探针的失败只有一种语义：**读不动或读不懂**。目录不存在是 **skip**
    /// （由 `run_inspect_logic` 正常返回一行"无 demo、跳过"），⛔ 不走这一档。
    InspectLogic {
        /// 被扫的 demo 目录。
        path: PathBuf,
        /// 读取/解析层的原样文案。
        source: String,
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
    /// `--enable-ui-mcp-http` 被送进了 [`run_batch`]。
    ///
    /// 同款防假绿，**而且更硬**：UI 控制面的执行面是"软件平台上的活控件树"
    /// （`ui/*` 的 `ui/tree` / `ui/screenshot` 都要它），而 [`run_batch`] 一个 Slint
    /// 对象都不构造。放它过去 ⇒ 服务会绑上，但每条 `ui/*` 都会诚实地失败 ——
    /// 那不是"服务起来了"，只是"端口开着"。正确入口是 `crate::ui_mcp_serve::run`。
    UiMcpHttpNotBatch,
}

impl CliError {
    /// 这一档失败的退出码（`--help` 里对用户逐条写明）。
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Projection { .. }
            | Self::Ui { .. }
            | Self::HeadlessIdleNotBatch
            | Self::UiMcpHttpNotBatch
            | Self::McpHttpNotBatch => EXIT_UI,
            Self::AlsNotCompiled => EXIT_USAGE,
            Self::LogicNotCompiled => EXIT_USAGE,
            Self::InspectLogic { .. } => EXIT_OPEN,
            Self::Open { .. }
            | Self::ImportMidi { .. }
            | Self::ImportAudio { .. }
            | Self::Sfz { .. }
            | Self::TrackSampler { .. }
            | Self::MidiIn { .. } => EXIT_OPEN,
            // ⭐ `--midi-in-*` 的写音符 / 渲染：退出码由 `MidiNoteError` 自己按变体给
            //（WAV 落盘失败 ⇒ `EXIT_EXPORT`，其余 ⇒ `EXIT_OPEN`）⇒ ⛔ 不在这里再写第二张表。
            Self::MidiNotes { source } => source.exit_code(),
            Self::Usage(_) => EXIT_USAGE,
            Self::Save { .. } => EXIT_SAVE,
            Self::Record { .. } => EXIT_EXPORT,
            Self::Export { .. }
            | Self::ExportMidi { .. }
            | Self::ExportWav { .. }
            | Self::ExportAls { .. }
            | Self::ExportLogic { .. } => EXIT_EXPORT,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MidiIn { source } => write!(formatter, "--midi-in 失败: {source}"),
            Self::MidiNotes { source } => write!(formatter, "--midi-in 音符路径失败: {source}"),
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
            Self::Record { path, source } => {
                write!(formatter, "录音到 `{}` 失败: {source}", path.display())
            }
            Self::Usage(reason) => write!(formatter, "用法错误: {reason}"),
            Self::Sfz { path, source } => {
                write!(formatter, "装载 SFZ `{}` 失败: {source}", path.display())
            }
            Self::TrackSampler { path, reason } => {
                write!(formatter, "把 `{}` 装到音轨失败: {reason}", path.display())
            }
            Self::ImportMidi { path, source } => {
                write!(
                    formatter,
                    "从 `{}` 导入 MIDI 失败: {source}",
                    path.display()
                )
            }
            Self::ImportAudio { path, source } => {
                write!(formatter, "从 `{}` 导入音频失败: {source}", path.display())
            }
            Self::ExportMidi { path, source } => {
                write!(
                    formatter,
                    "导出 MIDI 到 `{}` 失败: {source}",
                    path.display()
                )
            }
            Self::ExportWav { path, source } => {
                write!(formatter, "导出 WAV 到 `{}` 失败: {source}", path.display())
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
            Self::InspectLogic { path, source } => {
                write!(
                    formatter,
                    "只读扫描 demo 目录 `{}` 失败: {source}",
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
            Self::UiMcpHttpNotBatch => write!(
                formatter,
                "`{UI_MCP_HTTP_SWITCH}` 不能走 run_batch: 那条路径零 Slint 依赖, \
                 一个控件树对象都不构造, 而 ui/* 的全部内容都长在那棵树上面 ⇒ \
                 正确入口是 crate::ui_mcp_serve::run"
            ),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } => Some(source),
            Self::ImportMidi { source, .. } => Some(source),
            Self::ImportAudio { source, .. } => Some(source),
            Self::Sfz { source, .. } => Some(source),
            Self::MidiIn { source } => Some(source),
            Self::MidiNotes { source } => Some(source),
            Self::Record { source, .. } => Some(source),
            Self::Projection { source } => Some(source),
            Self::Save { source, .. } | Self::Export { source, .. } => Some(source),
            Self::ExportMidi { source, .. } => Some(source),
            Self::ExportWav { source, .. } => Some(source),
            Self::ExportAls { source, .. } => Some(source),
            Self::ExportLogic { source, .. } => Some(source),
            Self::Usage(_)
            | Self::Ui { .. }
            | Self::HeadlessIdleNotBatch
            | Self::UiMcpHttpNotBatch
            | Self::McpHttpNotBatch
            | Self::AlsNotCompiled
            | Self::TrackSampler { .. }
            | Self::LogicNotCompiled
            | Self::InspectLogic { .. } => None,
        }
    }
}

/// 取出当前工程：`--open` 的文件，或 `--project-sample` 选中的内置样本。
///
/// 内置样本的 `history.dag` 为**空**（样本本来就没有这一层；
/// `[MODEL-ISO-001]` 的三层状态里它是第二层）—— 这是如实置空，不是丢失。
///
/// ⭐ 资产池**不再**一律置空（任务单 `#142`）：`default` / `filled` 两个样本自带
/// 运行期合成的音频字节，走 [`Sample::assets`] 进同一份归档；`empty` 没有片段 ⇒
/// 仍是空池。
///
/// ⭐ `#215`：样本分支没有磁盘对应物 ⇒ 这一份工程是**此刻**创建出来的，因此创建时刻
/// 由 [`load_project_with_clock`] 取**一次**写进 `metadata.created_at_unix_ms`。
/// `--open` 分支⛔ 一次都不取时钟：文件里的创建时刻是既有事实，打开不改写它。
///
/// # Errors
///
/// [`CliError::Open`]：读文件失败 / 超过上限 / 不是 `.yeban` 容器 / 容器拒绝。
pub fn load_project(options: &Options) -> Result<Loaded, CliError> {
    load_project_with_clock(options, yeban_model::project::now_unix_ms)
}

/// [`load_project`] 的**可注入时钟**形态（判据用）：样本分支把 `clock` 读**恰好一次**，
/// `--open` 分支一次都不读 —— 两个读数都可以被数出来。
///
/// # Errors
///
/// 同 [`load_project`]。
pub fn load_project_with_clock(
    options: &Options,
    clock: impl FnOnce() -> u64,
) -> Result<Loaded, CliError> {
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
        None => {
            let mut project = options.sample.project();
            project.record_creation_time(clock());
            Ok(Loaded {
                archive: ProjectArchive {
                    project,
                    history_dag: Vec::new(),
                    assets: options.sample.assets(),
                },
                source: ProjectSource::Sample(options.sample),
            })
        }
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
/// **两个**自由文本字段都是**引号包裹**的（最小转义：`\\` `\"` `\n` `\r` `\t`，
/// 其余控制字符写作 `\u{…}`）：行内的 `title="…"`，以及 ⭐ 与它**并列**的 `author="…"`
/// （署名此前没有任何展示通路，见 `project_report` 里那段说明）。
#[must_use]
pub fn project_report(loaded: &Loaded) -> Vec<String> {
    let counts = project_counts(loaded);
    let project = &loaded.archive.project;
    vec![
        loaded.source.report_line(),
        // ⭐ `author` 与 `title` **并列**（同一条通路）：署名是自由文本注记，空串是
        // "未署名"的确定编码（`YebanProjectV1::author` 的 `#[serde(default)]` 文档）。
        //
        // ⚠ **为什么落在这里**：`author` 此前**没有任何展示或导出通路**（UI 零命中、
        // `export_logic` 不写它、`als.rs:482` 逐字写着 "author 不写入本子集"、
        // `yeban-midi` 零命中）⇒ 用户改了署名**在任何界面、任何导出物里都看不到**，
        // 而它**可写**（`setAuthor` 已在工具面）⇒ "写进去了但不可见"比"不能写"更误导。
        // ⭐ 本行是**既有**的报告通路（与 `title` 同一个消费面），⛔ 不新造展示面、
        // ⛔ 不动像素、⛔ 不需要新控件。
        format!(
            "project: id={} bpm={:.2} ts={}/{} title={} author={}",
            project.id.to_canonical_string(),
            project.bpm,
            project.time_signature.numerator,
            project.time_signature.denominator,
            quoted(&project.title),
            quoted(&project.author),
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
const SHORTCUTS: [ShortcutRow; 42] = [
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
        label: "Cmd/Ctrl+D → 复制到下一格",
        action: Action::Duplicate,
        key: PhysicalKey::KeyD,
        modifiers: Modifiers::meta(),
        implemented: true,
    },
    ShortcutRow {
        label: "Delete/Backspace → 删除",
        action: Action::DeleteSelection,
        key: PhysicalKey::Delete,
        modifiers: Modifiers::none(),
        implemented: true,
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
        implemented: true,
    },
    ShortcutRow {
        label: "Shift+Z → 全曲总览",
        action: Action::ZoomToFit,
        key: PhysicalKey::KeyZ,
        modifiers: Modifiers::shift(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+M → 控制台最大化",
        action: Action::ToggleConsoleMaximize,
        key: PhysicalKey::KeyM,
        modifiers: Modifiers::ctrl_alt(),
        implemented: true,
    },
    // ⭐ 节拍器族（`[ARCH-RT-001]`）：两条**加性**键，⛔ 不动既有键语义。
    // ⚠ `implemented: true` 是**如实**的：两条都在 `apply_action` 里接了臂
    // （产品装配 `main.rs` 传引擎上下文；其它装配传 `None` ⇒ 那一刻不消费，见臂上的注释）。
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+N → 节拍器开关",
        action: Action::ToggleMetronome,
        key: PhysicalKey::KeyN,
        modifiers: Modifiers::ctrl_alt(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+D → 预备拍开关",
        action: Action::ToggleCountIn,
        key: PhysicalKey::KeyD,
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
    // `[UI-NOTE-005]` §3.5 项 1–2 的八条方向键绑定（2026-10-13 接线；四个修饰组合 × 两个方向）。
    //
    // 为什么进这张表：`--print-shortcuts` 就是**用户可见的键位声明**，而本仓库的规矩是
    // "表 / 解析 / 宿主逐行对账"（判据 B11b）—— 新键不写进表就等于让 `--print-shortcuts`
    // 少报一个已经能用的键。用**物理码**行表达（`PhysicalKey::LeftArrow` 等）而不是只标逻辑键，
    // 是为了让 `InputContext::resolve` 那条入口也在判据里被逐行对账。
    ShortcutRow {
        label: "← → 选区左移一个网格",
        action: Action::NudgeSelection {
            step: NudgeStep::Grid,
            direction: -1,
        },
        key: PhysicalKey::LeftArrow,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "→ → 选区右移一个网格",
        action: Action::NudgeSelection {
            step: NudgeStep::Grid,
            direction: 1,
        },
        key: PhysicalKey::RightArrow,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "Alt+← → 选区左移 1 tick",
        action: Action::NudgeSelection {
            step: NudgeStep::Tick,
            direction: -1,
        },
        key: PhysicalKey::LeftArrow,
        modifiers: Modifiers::alt(),
        implemented: true,
    },
    ShortcutRow {
        label: "Alt+→ → 选区右移 1 tick",
        action: Action::NudgeSelection {
            step: NudgeStep::Tick,
            direction: 1,
        },
        key: PhysicalKey::RightArrow,
        modifiers: Modifiers::alt(),
        implemented: true,
    },
    ShortcutRow {
        label: "↑ → 选区升一个半音",
        action: Action::NudgeSelection {
            step: NudgeStep::Semitone,
            direction: 1,
        },
        key: PhysicalKey::UpArrow,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "↓ → 选区降一个半音",
        action: Action::NudgeSelection {
            step: NudgeStep::Semitone,
            direction: -1,
        },
        key: PhysicalKey::DownArrow,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "Shift+↑ → 选区升一个八度",
        action: Action::NudgeSelection {
            step: NudgeStep::Octave,
            direction: 1,
        },
        key: PhysicalKey::UpArrow,
        modifiers: Modifiers::shift(),
        implemented: true,
    },
    ShortcutRow {
        label: "Shift+↓ → 选区降一个八度",
        action: Action::NudgeSelection {
            step: NudgeStep::Octave,
            direction: -1,
        },
        key: PhysicalKey::DownArrow,
        modifiers: Modifiers::shift(),
        implemented: true,
    },
    // `[UI-NOTE-005]` §3 项 3：`Shift+←/→` = 时值缩短 / 延长一个吸附网格
    //（`Op::SetNoteTiming`）。⚠ 标签以 `Shift+` 开头 ⇒ 与无修饰 `← / →` 那两条的
    // `starts_with` 对账不会互相误配。
    ShortcutRow {
        label: "Shift+← → 选区时值缩短一个网格",
        action: Action::ResizeSelectionDuration { direction: -1 },
        key: PhysicalKey::LeftArrow,
        modifiers: Modifiers::shift(),
        implemented: true,
    },
    ShortcutRow {
        label: "Shift+→ → 选区时值延长一个网格",
        action: Action::ResizeSelectionDuration { direction: 1 },
        key: PhysicalKey::RightArrow,
        modifiers: Modifiers::shift(),
        implemented: true,
    },
    // ⭐ **键盘微调选中轨的混音参数**（四条 = 两格 × 两方向）。
    //
    // 与上面八条音符微调的区别是**作用域**：那八条动 `selected-ulids`（音符身份集合），
    // 这四条动 `selected-track-id`（一个轨道身份）的 `volume_db` / `pan`。⚠ **规范没有点名
    // 这一动作**（`[UI-NOTE-005]` §3.5 四条项只讲音符）⇒ 这是**界面决定**，登记在
    // `docs/ledger/app-mixer-notes.md` §7 第 10 行。为什么占 `Cmd/Ctrl+Alt+方向` 这一格，
    // 理由逐条写在 `Action::NudgeMixerChannel` 的文档里（六个已被 §3.5 点名的方向键组合、
    // 以及被判据如实钉成放行的 `Cmd/Ctrl+←/→` 与 `Alt+↑/↓`，都不能占 ⇒ 只剩这一格是空的）。
    //
    // `implemented: true` 的**主张范围**要说清：它陈述的是"宿主有这条臂、按下会发生什么"，
    // 不是"这一键在**任何**装配下都会改工程"。没有选中轨时 `apply_action` 返回 `false`
    // （不消费），这与 `Undo` 在没有撤销端口的装配下返回 `false` 同一条口径 ——
    // 逐行对账的是"表 / 解析 / 宿主有没有落点"这三面（判据 B11b）。
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+↑ → 选中轨音量 +1.0 dB",
        action: Action::NudgeMixerChannel {
            param: MixerParam::Volume,
            direction: 1,
        },
        key: PhysicalKey::UpArrow,
        modifiers: Modifiers::ctrl_alt(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+↓ → 选中轨音量 -1.0 dB",
        action: Action::NudgeMixerChannel {
            param: MixerParam::Volume,
            direction: -1,
        },
        key: PhysicalKey::DownArrow,
        modifiers: Modifiers::ctrl_alt(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+→ → 选中轨声相右 0.1",
        action: Action::NudgeMixerChannel {
            param: MixerParam::Pan,
            direction: 1,
        },
        key: PhysicalKey::RightArrow,
        modifiers: Modifiers::ctrl_alt(),
        implemented: true,
    },
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+← → 选中轨声相左 0.1",
        action: Action::NudgeMixerChannel {
            param: MixerParam::Pan,
            direction: -1,
        },
        key: PhysicalKey::LeftArrow,
        modifiers: Modifiers::ctrl_alt(),
        implemented: true,
    },
    // ⭐ **键盘拨动**选中轨的静音 / 独奏（两条 = 两个开关）。
    //
    // 与上面四条的分工是"**连续量 vs 布尔量**"：微调有"一格"，拨动只有"当前值取反"。
    // 同样**不是**规范点名（登记在 `docs/ledger/app-mixer-notes.md` §7 第 10 行的 ② 那半句）；
    // 裸 `m` / `s` 此前都落到 `PassThrough`（`Cmd/Ctrl+Alt+M` 是控制台最大化，另一格），
    // 因此加进表里不动任何既有绑定。逐条理由见 `Action::ToggleTrackMute` 的文档。
    ShortcutRow {
        label: "M → 选中轨静音开/关",
        action: Action::ToggleTrackMute,
        key: PhysicalKey::KeyM,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    ShortcutRow {
        label: "S → 选中轨独奏开/关",
        action: Action::ToggleTrackSolo,
        key: PhysicalKey::KeyS,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    // ⭐ **界面决定**（⛔ 不是规范点名）：标准 DAW 的保存键 `Cmd/Ctrl+S`。
    //
    // 与上面两条的关系：裸 `S` 是独奏 ⇒ 保存**必须带** `Cmd/Ctrl`；这一格此前落到
    // `PassThrough`（`command()` 分支里 `s` 一条都没有），因此加它不动任何既有绑定。
    // `Cmd/Ctrl+Shift+S` / `Cmd/Ctrl+Alt+S` 仍放行（"另存为"是另一票界面决定）。
    // 落点是**既有**的 `save-project` 回调（`host::wire_save` → `save_action::dispatch_save`），
    // 与 transport 的「保存」按钮、与「换工程前先保存」是同一个入口。逐条理由见
    // `Action::SaveProject` 的文档；登记在 `docs/ledger/app-cli-notes.md` §5.4。
    ShortcutRow {
        label: "Cmd/Ctrl+S → 保存工程",
        action: Action::SaveProject,
        key: PhysicalKey::KeyS,
        modifiers: Modifiers::meta(),
        implemented: true,
    },
    // ⭐ **界面决定**（⛔ 不是规范点名）：标准 DAW 的打开键 `Cmd/Ctrl+O`。
    //
    // `o` 在 `command()` 分支里此前一格都没有、裸 `o` 也从未绑定 ⇒ 加它不动任何既有
    // 绑定；`Cmd/Ctrl+Shift+O` / `Cmd/Ctrl+Alt+O` 仍放行。落点是**既有**的
    // `open-project` 回调（`host::wire_open_project`），与控制台「打开工程」按钮同一条链。
    // 逐条理由见 `Action::OpenProject` 的文档；登记在 `docs/ledger/app-cli-notes.md` §5.4。
    ShortcutRow {
        label: "Cmd/Ctrl+O → 打开工程",
        action: Action::OpenProject,
        key: PhysicalKey::KeyO,
        modifiers: Modifiers::meta(),
        implemented: true,
    },
    // ⭐ **界面决定**（⛔ 不是规范点名）：走带「录音」按钮的键盘入口，裸 `R`。
    //
    // 裸 `r` 此前在策略表任何一步里都不存在 ⇒ 加它不动任何既有绑定；`Cmd/Ctrl+R` /
    // `Shift+R` / `Alt+R` 仍放行。落点是**既有**的 `toggle-record` 回调
    // （`wire_transport` → `wire_record_at`），与走带「录音」按钮同一条链。
    // 逐条理由见 `Action::ToggleRecord` 的文档；登记在 `docs/ledger/app-cli-notes.md` §5.4。
    ShortcutRow {
        label: "R → 录制开/关",
        action: Action::ToggleRecord,
        key: PhysicalKey::KeyR,
        modifiers: Modifiers::none(),
        implemented: true,
    },
    // ⭐ **界面决定**（⛔ 不是规范点名）：走带 AI 抽屉按钮的键盘入口 `Cmd/Ctrl+Alt+A`。
    //
    // 该组合此前落在 `command()` 分支的 `_ => PassThrough`（当时只有 `b` / `m` / `n` /
    // `d` / 方向键），裸 `a` 与 `Alt+A` 也从未绑定 ⇒ 加它不动任何既有绑定；
    // `Cmd/Ctrl+Alt+Shift+A` 仍放行。落点是**既有**的 `toggle-ai-drawer` 回调
    // （`wire_view_callbacks`），与走带 `transport-ai-drawer-button` 同一条链。
    // 逐条理由见 `Action::ToggleAiDrawer` 的文档；登记在 `docs/ledger/ui-modules-notes.md`。
    ShortcutRow {
        label: "Cmd/Ctrl+Alt+A → AI 抽屉开/关",
        action: Action::ToggleAiDrawer,
        key: PhysicalKey::KeyA,
        modifiers: Modifiers::ctrl_alt(),
        implemented: true,
    },
    // ⭐ **界面决定**（⛔ 不是规范点名）：键盘-only 用户的**选轨**入口，`Alt+↑` / `Alt+↓`。
    //
    // 该组合此前落在第 4 步的 `_ => PassThrough`（当时只有 `1` / `2` / `←` / `→`）；
    // 裸 `↑/↓`（网格 / 半音微调）、`Shift+↑/↓`（八度）与 `Cmd/Ctrl+Alt+↑/↓`（音量微调）
    // 一格不动。落点是**既有**的 `track-select` 回调（`host::wire_track_select`），
    // 与鼠标点轨道行同一条链。逐条理由见 `Action::SelectTrackBy` 的文档；
    // 登记在 `docs/ledger/app-cli-notes.md` §5.4。
    ShortcutRow {
        label: "Alt+↑ → 选上一条轨",
        action: Action::SelectTrackBy(-1),
        key: PhysicalKey::UpArrow,
        modifiers: Modifiers::alt(),
        implemented: true,
    },
    ShortcutRow {
        label: "Alt+↓ → 选下一条轨",
        action: Action::SelectTrackBy(1),
        key: PhysicalKey::DownArrow,
        modifiers: Modifiers::alt(),
        implemented: true,
    },
    // 以下**一条**是本版本仅剩的 `implemented: false` 条目。
    //
    // ⭐ **两个数字都不在这里手抄**（此前这一行写着"表共 26 条"，而表的**总数**其实是 28 ——
    // 26 是 `implemented: true` 的条数，被当成了总数 ⇒ 读的人按它对账会**差 2**，
    // 而差的正是下面这类 `false` 条目）。⭐ 事实源是**类型标注**与**判据**，逐条复算：
    //   - 表有多大 ⇒ 本数组的**类型标注**（编译期）＋ 判据
    //     `shortcut_table_status_matches_the_resolution_and_host_pipeline` 的 `rows.len()` 断言；
    //   - 有几条未实现 ⇒ 同文件的 `the_ai_suggestion_and_audition_keys_are_still_declared_unimplemented`
    //     断言未实现条目**恰好**是哪一条（精确到条目，比只数条数更强）＋ `printed_marks`。
    // ⇒ ⭐ 注释只负责**指路**，⛔ 不再复述数字（复述过的数字会陈旧，这一行就是实例）。
    //
    // `implemented` 是"宿主会不会消费这一键"的主张，事实源是 `host::action_has_implementation`
    // （逐行对账见本文件 `ShortcutRow` 的文档与 `tests/cli_contract.rs` 的判据 B11b）。
    // 这一条之所以**仍是** `false`：缺的是**能力**，不是接线 ——
    //   - `[`（试听主线）：规范 `[ARCH-RT-005]` 要"主线与提案分支并发渲染 + 30ms 等功率瞬切
    //     + 2048 采样预滚"，这套机制在仓库里一处都不存在。
    // ⭐ 而 `Shift+Enter`（采纳 AI 建议）于 2026-10-31 **移出**本名单：抽屉现在既有
    //   看得见提案的数据面（`host::wire_proposal_drawer` 从唯一可变权威注入那九个属性），
    //   也有**写入口**（`HostAction` 的 `MergeProposal` / `RejectProposal` 两臂，复用工具面
    //   `yeban_merge_proposal` 所调的**同一个** `merge_plan`），而"选中哪一条"是窗口属性
    //   `musical-pr-selected-id`（值从权威的 `ProposalRow::id` 来，⛔ 界面不自造身份）。
    //   ⚠ 本列说"有落点"，运行时仍然会**如实不消费**：`host::apply_action` 的那一条要求
    //   抽屉开着 ∧ `musical-pr-resolve-available`（= 有提案来源 ∧ 点选了当前清单里的一行）
    //   ⇒ 没挂控制面的装配按 `Shift+Enter` 一位都不动，与 `DeleteSelection` /
    //   `ZoomToSelection` / `Duplicate` 空前置条件时的取向一致。
    // 逐条证据（`grep` 字面命中与 `file:line`）见 `host.rs` 的 `action_has_implementation` 文档；
    // "它还不可用"这件事由 `tests/cli_contract.rs` 的判据 B11f 机械钉住（谁真接了、忘了改判据即变红）。
    ShortcutRow {
        label: "Shift+Enter → 采纳 AI 建议",
        action: Action::AcceptAiSuggestion,
        key: PhysicalKey::Enter,
        modifiers: Modifiers::shift(),
        implemented: true,
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
/// `--inspect-logic <dir>` 的执行（`#143`）：**只读**，不装载工程、不写任何文件。
///
/// 输出契约（与判据共用，见 `yeban-render` 的 `tests/logic_inspect.rs`）：
/// * 每台 demo 一行 [`LogicInspection::report_line`]（`logic-demo:` 前缀）；
/// * 最后一行 [`LogicTotals::report_line`]（`logic-total:` 前缀）；
/// * 目录不存在 ⇒ **只**返回一行 `logic-inspect: 无 demo（… 不存在），跳过（0 台）`
///   并成功退出（路径门控：CI/其它机器没有 Apple demo 时 ⛔ 不变红）；
/// * 目录在但解析失败 ⇒ `Err(CliError::InspectLogic)`（退出码 [`EXIT_OPEN`]）——
///   ⛔ 不静默跳过。
///
/// # Errors
///
/// [`CliError::InspectLogic`]：目录存在但读不动、没有 `.logicx`、或任意一台解析失败。
///
/// ⚠ 本函数只在 `experimental-logic-import` 下编译：默认构建里
/// `yeban_render::logic_read` 根本不存在。
#[cfg(feature = "experimental-logic-import")]
pub fn run_inspect_logic(dir: &std::path::Path) -> Result<Vec<String>, CliError> {
    let inspections =
        yeban_render::logic_read::inspect_demos(dir).map_err(|source| CliError::InspectLogic {
            path: dir.to_path_buf(),
            source,
        })?;
    let Some(inspections) = inspections else {
        return Ok(vec![format!(
            "logic-inspect: 无 demo（{} 不存在），跳过（0 台）",
            dir.display()
        )]);
    };
    let mut lines: Vec<String> = inspections
        .iter()
        .map(yeban_render::logic_read::LogicInspection::report_line)
        .collect();
    lines.push(yeban_render::logic_read::LogicTotals::of(&inspections).report_line());
    Ok(lines)
}

/// ⭐ 任务单 `#211`：`--probe-instruments` 的**执行** —— 逐条跑字节探针并**立即**交出一行。
///
/// 为什么用回调而不是返回 `Vec<String>`：真库全量一次约 **10 分钟**（`#210` 实测
/// 597,530 ms）。把 28 行全部攒到结束再打印，终端会安静 10 分钟 —— 那看起来正好像
/// "卡死"。回调让每一行**算完就出去**，人能看见进度。
///
/// 输出契约（判据逐字断言）：
/// * `lib: <库根>` —— 先声明这次读的是哪个库；
/// * `instrument-ok: <名字> | <路径> | decoded=<n>` —— 可装；
/// * `instrument-fail: <名字> | <路径> | <逐字原因>` —— 不可装（原因与界面**同一份核心
///   措辞**，来自 [`crate::bridge::instrument_probe_failure_reason`]；终端里没有"右侧按钮"，
///   因此界面那一层多加的"点右侧…"指引**不**出现在这里）；
/// * `instrument-total: total=<n> installable=<n> missing=<n> unreadable=<n>` —— 最后一行。
///
/// 返回四元组 `(total, installable, missing, unreadable)`，判据不必解析文本。
///
/// ⚠ 只读：不写库根、不写工程、不开窗口。
pub fn run_probe_instruments(mut emit: impl FnMut(String)) -> (usize, usize, usize, usize) {
    emit(format!("lib: {}", crate::samples::library_root().display()));
    let (mut installable, mut missing, mut unreadable) = (0_usize, 0_usize, 0_usize);
    let instruments = crate::bridge::library_instruments();
    let total = instruments.len();
    for instrument in instruments {
        match crate::bridge::probe_instrument_samples(&instrument.path) {
            crate::bridge::InstrumentSampleProbe::Installable { decoded_samples } => {
                installable += 1;
                emit(format!(
                    "instrument-ok: {} | {} | decoded={decoded_samples}",
                    instrument.name, instrument.path
                ));
            }
            outcome @ crate::bridge::InstrumentSampleProbe::MissingSamples { .. } => {
                missing += 1;
                emit(format!(
                    "instrument-fail: {} | {} | {}",
                    instrument.name,
                    instrument.path,
                    crate::bridge::instrument_probe_failure_reason(&instrument.name, &outcome)
                ));
            }
            outcome @ crate::bridge::InstrumentSampleProbe::UnreadableSfz => {
                unreadable += 1;
                emit(format!(
                    "instrument-fail: {} | {} | {}",
                    instrument.name,
                    instrument.path,
                    crate::bridge::instrument_probe_failure_reason(&instrument.name, &outcome)
                ));
            }
        }
    }
    emit(format!(
        "instrument-total: total={total} installable={installable} missing={missing} \
         unreadable={unreadable}"
    ));
    (total, installable, missing, unreadable)
}

/// ⭐ 任务单 `#287`：`--sampler-track <trackId>` 的目标轨解析（**唯一**实现）。
///
/// 规则（顺序即优先级，**没有"猜"也没有"回落"**）：
/// 1. 取值必须是**音轨身份**（26 字符 ULID，大小写不敏感；`EntityId::from_str` 负责
///    规范化）。形状不对 ⇒ 错误 —— ⛔ 不把它当名字 / 序号去匹配（那正是"选错轨"的
///    来源：同一串文本在别的选择器里可能有别的含义）；
/// 2. 该身份必须**在** `project.tracks` 里 ⇒ 否则错误，并列出可用 `id=名字`；
/// 3. 该身份**不是**主总线 ⇒ 否则错误。
///
/// 为什么主总线要**响亮拒绝**（而不是"装了也算成功"）：主总线是整份工程的唯一混音
/// 出口，不是乐器轨 —— `render_core` 的 `renderable` 逐字把它排除在外
/// （`filter(|id| *id != project.master_bus_track_id && …)`）⇒ 主总线的
/// `TrackV3.sampler` **没有任何消费点**，装上去只会静默无效。⛔ 不接受一个
/// "看起来成功、其实永远不发声"的目标。
fn resolve_sampler_track<'a>(
    project: &'a YebanProjectV1,
    wanted: &str,
) -> Result<&'a TrackV3, String> {
    let available = selectable_track_list(project);
    let id = wanted.parse::<EntityId>().map_err(|_| {
        format!(
            "`--sampler-track` 的取值 `{wanted}` 不是音轨身份 (26 字符 ULID, 大小写不敏感); \
             可用: {available}"
        )
    })?;
    let track = project
        .tracks
        .get(&id)
        .ok_or_else(|| format!("工程里没有身份 `{id}` 的音轨; 可用: {available}"))?;
    if id == project.master_bus_track_id {
        return Err(format!(
            "身份 `{id}` (`{}`) 是**主总线**: 主总线是整份工程的唯一混音出口, 不是乐器轨 —— \
             渲染的 `renderable` 明确把它排除在外, 装上去的 `TrackV3.sampler` 永远不会有\
             消费点 ⇒ 拒绝; 可用: {available}",
            track.name
        ));
    }
    Ok(track)
}

/// ⭐ 任务单 `#287`：可用（= **非主总线**）音轨身份的**人可读**清单。
///
/// 空清单也要**明说**（⛔ 不留空白）：错误报文里"可用: "后面必须有东西，否则
/// 用户会以为选择器坏了。
fn selectable_track_list(project: &YebanProjectV1) -> String {
    let list: Vec<String> = project
        .tracks
        .values()
        .filter(|track| track.id != project.master_bus_track_id)
        .map(|track| format!("{}={}", track.id, track.name))
        .collect();
    if list.is_empty() {
        "工程里没有非主总线轨".to_owned()
    } else {
        list.join(" / ")
    }
}

/// ⭐ 任务单 `#210`／`#239`：`--track-sampler` 的**唯一**应用实现。
///
/// 一次装 ＝ **恰好一次** `Op::SetTrackSampler` ＋ 一次提交（已经装有这份乐器 ⇒ 0 次提交）。
/// 返回**应用之后的工程**与**回执行**；由调用方决定把它落到哪里。
///
/// ## 目标轨（⭐ 任务单 `#287`）
///
/// `track_id` 为 `None` ⇒ **旧行为**：展示轨 = [`crate::bridge::device_rack_track`]
/// （身份序第一条非主总线轨），逐字节不变；为 `Some` ⇒ 只认
/// [`resolve_sampler_track`] 解析出的那一条（形状不对 / 查不到 / 主总线 ⇒ 响亮报错，
/// ⛔ 不回落）。两条都只影响"装到谁"，⛔ 不碰渲染语义（`F6`／`#240` 已修好）。
///
/// ## 为什么返回值里带工程（`#239`）
///
/// `--record-wav` 读的是**工程快照**，而绑定原先排在录音块之后（`#222` 实测：录制在旧
/// `cli.rs:3986`、绑定在 `4138`）⇒ `--record-wav --track-sampler X` 录到的是**绑定前**的
/// 音源。录音块因此在**副本**上先跑一次本函数（`run_batch` 的 `recorded_bound_project`），
/// 真身仍在导入之后的原位置应用 ⇒ 其它组合的次序与字节不变。
fn apply_track_sampler(
    project: &YebanProjectV1,
    path: &Path,
    track_id: Option<&str>,
) -> Result<(YebanProjectV1, String), CliError> {
    let sfz_text = path.to_str().ok_or_else(|| CliError::TrackSampler {
        path: path.to_path_buf(),
        reason: "路径不是 UTF-8，无法逐字写进 `TrackV3::sampler.path`".to_owned(),
    })?;
    // 目标轨：显式身份（`#287`）或展示轨（身份序第一条非主总线轨；CLI 没有选中态
    // ⇒ 与 GUI 的兜底同一档）。显式身份**没有**第二条解析路径。
    let track = match track_id {
        Some(wanted) => Some(
            resolve_sampler_track(project, wanted)
                .map_err(|reason| CliError::TrackSampler {
                    path: path.to_path_buf(),
                    reason,
                })?
                .clone(),
        ),
        None => crate::bridge::device_rack_track(project).cloned(),
    };
    let track_label = track
        .as_ref()
        .map_or_else(|| "<无>".to_owned(), |track| track.name.clone());
    let ops =
        crate::bridge::instrument_install_ops_on(track.as_ref(), sfz_text).map_err(|reason| {
            CliError::TrackSampler {
                path: path.to_path_buf(),
                reason: reason.to_owned(),
            }
        })?;
    if ops.is_empty() {
        // 已经是这份乐器：⛔ 不提交（不制造"什么也没改"的提交）。
        return Ok((
            project.clone(),
            format!(
                "track-sampler: track={track_label} path={sfz_text} ops=0 (已经是这份乐器, 工程未改)"
            ),
        ));
    }
    let now = crate::host::now_ms();
    let session =
        crate::undo::UndoSession::open("<cli:track-sampler>", "yeban-app", project.clone(), now)
            .map_err(|error| CliError::TrackSampler {
                path: path.to_path_buf(),
                reason: format!("建会话失败: {error}"),
            })?;
    let port = crate::undo::UndoPort::new(session);
    port.commit_ops(now, "set track sampler", ops)
        .map_err(|error| CliError::TrackSampler {
            path: path.to_path_buf(),
            reason: format!("提交被拒: {error}"),
        })?;
    let display = port.display();
    let line = format!(
        "track-sampler: track={track_label} path={sfz_text} ops=1 \
         commit_count={} undoable={}",
        display.commit_count, display.undoable
    );
    // 会话是权威 ⇒ 交给调用方落回归档：同一命令里的 `--export-wav` / `--save-as` /
    // `--record-wav` 看的是这一份。
    Ok((port.project(), line))
}

/// 无窗口路径的执行：返回**它应当打印的每一行**（打印由 [`emit`] / [`finish`] 做）。
///
/// 顺序是契约的一部分：
/// 1. `--dump-elements` 的元素行（先于握手行 —— 沿用旧行为）;
/// 2. `--print-shortcuts` 的策略表;
/// 3. 握手行 [`HEADLESS_HANDSHAKE`]（无窗口路径**一定**打印它）;
/// 4. 来源行 / `project:` / `project-counts:` / **`automation-engine:`**
///    （`#259`：实时引擎不执行的自动化泳道；需要投影时再加 `view-counts:`）;
/// 5. 边界声明行（`headless:` 开头，说清这一版无头**没有**验证什么）;
///    ⚠ **导入（编辑）先于一切导出与保存**：`--import-midi` / `--import-audio` 的
///    `imported-midi:` / `imported-audio:` 报告行落在这里（顺序契约 —— 同一命令里的
///    `--export-*` 与 `--save-as` 看到的必须是**导入后**的工程）;
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
    // ⭐ 任务单 `#287`：`--sampler-track` 的第二道守卫（`parse()` 已拒；这里兜住
    // 直接构造的 [`Options`] —— 字段是公开的，判据可以绕过解析器）。
    if options.sampler_track.is_some() && options.track_sampler.is_none() {
        return Err(CliError::Usage(SAMPLER_TRACK_PAIRING_ERROR.to_owned()));
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
    if options.ui_mcp_http {
        // 同款防假绿（更硬的那种，见 `CliError::UiMcpHttpNotBatch` 的文档）：
        // 端口会绑上，而每一条 ui/* 都会因为"没有控件树"而失败。
        return Err(CliError::UiMcpHttpNotBatch);
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

    let mut loaded = load_project(options)?;
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
    // ⭐ `#259`：**自动化泳道的落地读数**。这一行说的是"监听里哪几条泳道不会生效"，
    // 不是工程统计里多一格：`SendGain` 画得出来、MCP 写得进去，而实时引擎没有发送量
    // 槽位 ⇒ 用户会以为它生效。数出来的口径与 `EngineHost::publish_automation` **同一条**
    // 函数（`unapplied_automation_lanes`），⛔ 不另数一份。
    //
    // 位置：紧跟 `project-counts:` —— 它读的就是这一份工程；放在 `view-counts:` 之前，
    // 因为 `--save-as` 这类不投影的命令也要看到它。
    lines.push(crate::engine_host::automation_unapplied_line(
        crate::engine_host::unapplied_automation_lanes(&loaded.archive.project),
    ));
    if let Some(view) = view.as_ref() {
        lines.extend(view_report(view));
    }
    lines.push(HEADLESS_BOUNDARY.to_owned());

    // 录音：把一段测试音录成 WAV（三段见 `record_wav.rs`）。它只写**目标文件**，不改工程。
    // SFZ 装载 ＋ 音符探测：**只读**（不改工程、不写文件），所以它只影响输出行。
    // ⭐ 与 `--export-wav`（或 `--record-wav`）同时给 ⇒ 音源就是这份 SFZ（见下面的导出/录音块）。
    // 样本数据：`sample=` 指到的 PCM WAV 会被**真读**；读不到才回退内存正弦（原因进报告行）。
    let mut loaded_sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        std::path::PathBuf,
    )> = None;
    if let Some(path) = options.sfz.as_ref() {
        let (mut report, instrument) = crate::sfz_load::load_sfz_with_instrument(
            path,
            Some(crate::sfz_load::DEFAULT_PROBE_NOTE),
            48_000.0,
        )
        .map_err(|source| CliError::Sfz {
            path: path.clone(),
            source,
        })?;
        // 导出**或录音**真的会用到它，才报 `wired_to_export=yes`（⛔ 不夸大）。
        report.wired_to_export = options.export_wav.is_some() || options.record_wav.is_some();
        lines.push(report.summary());
        // ⭐ `#206`：被跳过的 `#include` **逐条**打印（`loaded-sfz` 汇总行里只有条数）。
        for issue in &report.include_issues {
            lines.push(format!("sfz-include-issue: {issue}"));
        }
        loaded_sfz = Some((std::sync::Arc::new(instrument), path.clone()));
    }

    // ⭐ 任务单 `#239`（缺陷 F3）：`--record-wav` 与 `--track-sampler` **同一调用**时，录音块
    // 原先排在绑定之前（`#222` 实测：录制在旧 `cli.rs:3986`、绑定在 `4138`）⇒ 用户写
    // `--record-wav … --track-sampler X` 却录到**绑定前**的音源（默认音源）。
    // 录音读的是**工程快照**，所以这里先在**副本**上把本次绑定应用一次，只喂给录音；真身仍在
    // 原位置（导入之后）应用到归档 ⇒ 单独 `--record-wav`、`--export-wav` ＋绑定、导入次序
    // **逐字节不变**（§1.2／§2.3–§2.5 的判据）。
    //
    // ⚠ 两条**不做**预应用（录音根本不读工程绑定，预应用只会挪动失败位次）：
    //   * `--record-source tone` ⇒ 录的是内存正弦，⛔ 不读工程；
    //   * 给了 `--sfz` ⇒ 录音走 SFZ 旁路（`record_project_under` 的 `sfz` 参数），
    //     `TrackV3.sampler` 不参与该次渲染。
    let recorded_bound_project: Option<YebanProjectV1> =
        match (options.record_wav.as_ref(), options.track_sampler.as_ref()) {
            (Some(_), Some(sampler))
                if options.record_source.as_deref() != Some("tone") && options.sfz.is_none() =>
            {
                Some(
                    apply_track_sampler(
                        &loaded.archive.project,
                        sampler,
                        options.sampler_track.as_deref(),
                    )?
                    .0,
                )
            }
            _ => None,
        };

    if let Some(path) = options.record_wav.as_ref() {
        let mut record_options = crate::record_wav::RecordOptions::default();
        if let Some(seconds) = options.record_seconds {
            record_options.seconds = seconds;
        }
        // 默认录**工程**（与 `--export-wav` 同一条渲染核心 ＋ 同一种子 ⇒ 逐位一致，判据断言）；
        // `--record-source tone` 才录内存正弦测试音。⭐ 给了 `--sfz` ⇒ 把乐器（＋SFZ 文件路径）
        // 一并交给 `record_project_under`（下面那两行）⇒ `sample=` 指向的 WAV 会被真读。
        //
        // ⭐ 任务单 `#170`：`--open` 那个文件所在目录 = 相对 SFZ 路径的第一个候选根
        //    （与 `--export-wav` **同一个** `Loaded::project_dir()`）⇒ "样本放在工程旁边"
        //    在**任意 cwd** 下对录制也成立。样本形态（没有 `--open`）给 `None` ⇒ 旧口径
        //    （仓库根 ⇒ cwd），⛔ 不假装有一个工程目录。
        // ⭐ 任务单 `#239`：工程参数取 `recorded_bound_project`（本次绑定已应用）优先。
        let project_dir = loaded.project_dir();
        let report = if options.record_source.as_deref() == Some("tone") {
            crate::record_wav::record_test_tone(path, &record_options)
        } else {
            crate::record_wav::record_project_under(
                recorded_bound_project
                    .as_ref()
                    .unwrap_or(&loaded.archive.project),
                path,
                &record_options,
                loaded_sfz
                    .clone()
                    .map(|(instrument, sfz_path)| (instrument, Some(sfz_path))),
                project_dir.as_deref(),
            )
        }
        .map_err(|source| CliError::Record {
            path: path.clone(),
            source,
        })?;
        lines.push(report.summary());
    }
    if options.record_seconds.is_some() && options.record_wav.is_none() {
        return Err(CliError::Usage(
            "`--record-seconds` 必须与 `--record-wav` 一起给".to_owned(),
        ));
    }
    // `--wav-*` 是 `--export-wav` 的**修饰**开关（⛔ 不选进程形态）⇒ 单独给就是
    // 一次"以为改了产物形状、其实一个字节都没动"的假绿。与 `--record-seconds` 同款。
    for (given, name) in [
        (options.wav_bit_depth.is_some(), "--wav-bit-depth"),
        (options.wav_sample_rate.is_some(), "--wav-sample-rate"),
        (options.wav_seconds.is_some(), "--wav-seconds"),
        (options.wav_frames.is_some(), "--wav-frames"),
        (options.wav_limiter, "--limiter"),
    ] {
        if given && options.export_wav.is_none() {
            return Err(CliError::Usage(format!(
                "`{name}` 必须与 `--export-wav` 一起给（⛔ 单独给会被静默忽略）"
            )));
        }
    }

    // ⚠ 组合性/用法检查**先于**任何工作：默认档下 `capture_messages()` 会先撞
    // `FeatureDisabled`，若把用法错误放在它之后，用户永远看不到那条用法提示。
    for (given, name) in [
        (options.midi_in_seconds.is_some(), "--midi-in-seconds"),
        (options.midi_in_write_notes, "--midi-in-write-notes"),
        (options.midi_in_render.is_some(), "--midi-in-render"),
        (options.midi_in_track.is_some(), "--midi-in-track"),
    ] {
        if given && options.midi_in.is_none() {
            return Err(CliError::Usage(format!(
                "`{name}` 必须与 `--midi-in` 一起给（⛔ 单独给会被静默忽略）"
            )));
        }
    }

    // ⭐ `--midi-in`：⭐ 采集**一次**，然后按开关走两条落地路径（⭐ 顺序 = 谱面 → 声音）。
    if let Some(wanted) = options.midi_in.as_ref() {
        let seconds = options
            .midi_in_seconds
            .unwrap_or(crate::midi_in::DEFAULT_SECONDS);
        // ⭐ **同一次**采集既喂报告、也喂音符路径（⛔ 不采两遍、⛔ 不用两份解析）。
        let captured = crate::midi_in::capture_messages(wanted, seconds)
            .map_err(|source| CliError::MidiIn { source })?;
        lines.push(captured.report().summary());
        apply_captured_midi(&mut loaded, options, &captured, &mut lines)?;
    }

    // 顺序契约（`--help` 的组合语义一节）：**导入 → 导出 → 保存工程**。
    // 导入是编辑，所以它先发生，同一命令里的导出与保存都能看到导入的音符。
    if let Some(path) = options.import_midi.as_ref() {
        let session = crate::undo::UndoSession::open(
            "<cli:import-midi>",
            "yeban-app",
            loaded.archive.project.clone(),
            crate::host::now_ms(),
        )
        .map_err(|error| CliError::ImportMidi {
            path: path.clone(),
            source: crate::import_midi::MidiImportError::Refused(error.to_string()),
        })?;
        let port = crate::undo::UndoPort::new(session);
        let report = crate::import_midi::import_midi_file(
            &port,
            &loaded.archive.project,
            path,
            &crate::import_midi::MidiImportOptions::default(),
            crate::host::now_ms(),
        )
        .map_err(|source| CliError::ImportMidi {
            path: path.clone(),
            source,
        })?;
        // 会话是权威 ⇒ 把导入后的工程落回归档，后续导出/保存看的是这一份。
        // ⭐ **此处的 `project()` ⛔ 不会 panic**：不变式是「**端口存在 ⇒ 权威会话必有一份
        // 活跃工程**」，⭐ 出处是 `undo.rs:327-329` 的 `# Panics` 文档（逐字：「`main.rs` 的
        // 装配路径保证端口存在时工程一定在」）＋ `main.rs` 的装配顺序：端口**由那一份工程
        // 建起来**、此后不被置空。⚠ 若将来「端口在而工程不在」变成**可达**状态，那正是
        // **新增 `CliError` 变体**的时机（本单⛔ 不加：那会牵动 `Display` 与退出码映射、
        // 属**用户可见的 CLI 面变更**，应由一次产品决定带来）。
        // ⭐ 同上：`project()` 在此⛔ 不会 panic（依据 `undo.rs:327-329` 的不变式）。
        // ⭐ 同上：`project()` 在此⛔ 不会 panic（依据 `undo.rs:327-329` 的不变式）。
        loaded.archive.project = port.project();
        lines.push(report.summary());
    }

    // `--import-audio`：与 `--import-midi` **同一条**位置与失败语义（导入是编辑 ⇒ 先于
    // 导出与保存；失败 ⇒ 直接 `Err`，⛔ 不继续保存）。
    if let Some(path) = options.import_audio.as_ref() {
        let session = crate::undo::UndoSession::open(
            "<cli:import-audio>",
            "yeban-app",
            loaded.archive.project.clone(),
            crate::host::now_ms(),
        )
        .map_err(|error| CliError::ImportAudio {
            path: path.clone(),
            source: crate::import_audio::AudioImportError::Refused(error.to_string()),
        })?;
        let port = crate::undo::UndoPort::new(session);
        let report = crate::import_audio::import_audio_file(
            &port,
            path,
            &loaded.archive.project,
            &crate::import_audio::AudioImportOptions::default(),
            crate::host::now_ms(),
        )
        .map_err(|source| CliError::ImportAudio {
            path: path.clone(),
            source,
        })?;
        // 会话是权威 ⇒ 把导入后的工程落回归档，后续保存看的是这一份。
        loaded.archive.project = port.project();
        // ⚠ CAS 字节**不是** `Op` 的载荷（模型的 `Op` 全集没有资产变体）⇒ 由**这里**
        // 登记进归档的资产池，`--save-as` 才会把它写成 `assets/{sha256}`。
        register_audio_asset(&mut loaded.archive, &report);
        lines.push(report.summary());
    }

    // ⭐ 任务单 `#210`：`--track-sampler` —— **装乐器**是编辑 ⇒ 与两条导入同一位次
    // （导出与保存都能看到它）。一次装 ＝ **恰好一次** `Op::SetTrackSampler` ＋ 一次提交。
    // ⭐ 任务单 `#239`：应用逻辑唯一实现在 [`apply_track_sampler`] —— 录音块在**副本**上先用
    // 同一份实现（`recorded_bound_project`），这里仍是**真身**的落点（导入之后）。
    if let Some(path) = options.track_sampler.as_ref() {
        let (project, line) = apply_track_sampler(
            &loaded.archive.project,
            path,
            options.sampler_track.as_deref(),
        )?;
        // 会话是权威 ⇒ 落回归档：同一命令里的 `--export-wav` / `--save-as` 看的是这一份。
        loaded.archive.project = project;
        lines.push(line);
    }

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

    // `--export-wav`：与 `--export-midi` 同一位置与同一失败语义（失败 ⇒ 直接 `Err`, ⛔ 不继续保存）。
    if let Some(path) = options.export_wav.as_ref() {
        // ⭐ `&loaded.archive.assets` 是**归档资产池**（`--open` 读进来的 `assets/{sha256}`）：
        // 没有它，工程里摆放的 `ClipContent::Audio` 会静音 —— 引擎快照把音频片段当静音，
        // 见 `crate::audio_clip` 的模块文档。
        // ⭐ 位深／采样率／时长／精确帧数在这里解析出**唯一一份**可执行选项
        // （缺省口径见 `WavExportOptions::resolve`：24-bit / 跟随工程 / 8 秒）。
        let mut wav_options = crate::export_wav::WavExportOptions::resolve(
            &loaded.archive.project,
            options.wav_sample_rate,
            options.wav_bit_depth,
            options.wav_seconds,
            options.wav_frames,
        );
        // ⭐ 任务单 `#302`：per-call 限制器开关；默认关 ⇒ 旧导出逐字节不变。
        wav_options.limiter = options.wav_limiter;
        // ⭐ 任务单 `#168`：`--open` 那个文件所在目录 = 相对 SFZ 路径的第一个候选根
        //    ⇒ "样本放在工程旁边"在**任意 cwd** 下都读得到。样本形态（没有 `--open`）
        //    给 `None`，解析从仓库根开始（旧口径）。
        let project_dir = loaded.project_dir();
        let report = crate::export_wav::export_wav_with_sfz_file_and_assets_under(
            &loaded.archive.project,
            path,
            wav_options,
            loaded_sfz
                .clone()
                .map(|(instrument, sfz_path)| (instrument, Some(sfz_path))),
            &loaded.archive.assets,
            project_dir.as_deref(),
        )
        .map_err(|source| CliError::ExportWav {
            path: path.clone(),
            source,
        })?;
        lines.push(exported_wav_line(&report, &loaded));
        // ⭐ 任务单 `#226`：削波时**多打一行**（含 `clippedSamples` / `suggestedTrimDb`）。
        //    ⛔ 不削波时**一行都不多** ⇒ `--export-wav` 的既有 stdout 逐字不变。
        if let Some(note) = report.clipping_note() {
            lines.push(note);
        }
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
        // ⭐ `#200`：目标目录**强制**以 `.logicx` 结尾。原因是 macOS 的 `open -a`
        //    （LaunchServices）靠**扩展名**认 bundle —— 负责人跑 `--export-logic hello`
        //    导出成功、紧接着 `open -a "Logic Pro" hello` 却报错。用户不该为这个后缀踩坑，
        //    所以缺后缀时**自动补**（已有则不加，`foo.logicx` ⛔ 不变 `foo.logicx.logicx`），
        //    回执的 `path=` 打印**实际**路径（见 `exported_logic_line`）。
        //    ⚠ 补全点刻意放在**导出侧**而不是 `parse()`：`Options::export_logic` 保持用户在
        //    命令行里写的字面值（`parse()` 的既有契约，判据在 `parse_export_logic` 一族里），
        //    而错误文案用的是**补全后**的实际目标 —— 那才是用户需要去看的路径。
        let path = crate::export_logic::logic_bundle_directory(path);
        let report = crate::export_logic::export_project_to_bundle(&loaded.archive.project, &path)
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

/// ⭐ **采集之后的落地步骤**（谱面 → 声音）。
///
/// ⚠ ⭐ **为什么这一步要单独存在**：真 MIDI 端口只在本机／真设备上存在
/// （`capture_messages()` 必须 `midir` 真的连上端口）⇒ ⭐ 若把"写音符 / 离线渲染"写死在
/// 采集调用之后，⭐ 判据就**永远只能断言采集本身**。把它拆出来之后，判据可以喂一批
/// **注入**的消息（与 `yeban-midi-io` 的模拟回调体 `MidiEventSender::handle_message`
/// 同手法）走**同一条**落地路径 —— 这正是"真设备那层不可覆盖、但可判的那层做干净"的落点。
///
/// 顺序（`--help` 的"组合语义"一节）：**先谱面（写工程）→ 再声音（离线渲染）**。
/// ⛔ 两个开关都不给时本函数**什么都不做**（`--midi-in` 单独给仍然是只读诊断）。
///
/// # Errors
///
/// [`CliError::MidiNotes`]：一条音符都没有 / 没有目标音轨 / 提交被拒 / 引擎重建失败 /
/// WAV 落盘失败（退出码见 [`crate::midi_notes::MidiNoteError::exit_code`]）。
pub fn apply_captured_midi(
    loaded: &mut Loaded,
    options: &Options,
    captured: &crate::midi_in::CapturedMessages,
    lines: &mut Vec<String>,
) -> Result<(), CliError> {
    if !options.midi_in_write_notes && options.midi_in_render.is_none() {
        return Ok(());
    }
    let pairing = crate::midi_notes::pair_notes(
        &captured.messages,
        captured.origin_us,
        crate::midi_notes::UNCLOSED_TAIL_TICKS,
    );
    lines.push(pairing.summary());

    let selected = match options.midi_in_track.as_deref() {
        Some(selector) => Some(
            crate::midi_notes::resolve_track(&loaded.archive.project, selector)
                .map_err(|source| CliError::MidiNotes { source })?,
        ),
        None => None,
    };

    // ---- ② 声音：**同一批事件 ⇒ `EngineEvent` 环 ⇒ 离线渲染**（引擎已接线 `eca718ab`）----
    //
    // ⭐ 这条是"弹一下 ⇒ 有声音"的**实质**：事件进**既有**的 UI→引擎 SPSC
    // （`EngineHost::send_engine_events`），由**与设备回调同一个** `process_quantum` 渲染
    // ⇒ 产物里是**事件真的作用到合成器**的声音（报告行的 `poly-notes-triggered=` 是见证）。
    //
    // ⚠ **顺序**：声音在**谱面之前**做。原因不是偏好：⭐ 若先把音符写进工程，⚠ 同一批音符会
    // 被**两条路**各触发一次 —— 环里的 `NoteOn` **与** 快照排程（`render_track`）
    // ⇒ 双触发（更响/相位差）。先渲染（基于**未写**这批音符的工程）⇒ 每条音符只触发一次，
    // 而工程**原有**的内容照旧由快照排程发声 ✓。
    if let Some(path) = options.midi_in_render.as_ref() {
        let track_id = match selected {
            Some(explicit) => explicit,
            None => crate::midi_notes::default_track(&loaded.archive.project)
                .map_err(|source| CliError::MidiNotes { source })?,
        };
        let defaults = crate::export_wav::WavExportOptions::default();
        let mut host = crate::engine_host::EngineHost::new();
        // ⭐ 任务单 `#168`：与 `--export-wav` 同一个工程目录根（`--open` 的父目录）。
        host.set_project_dir(loaded.project_dir());
        host.reload(&loaded.archive.project, 0)
            .map_err(|error| CliError::MidiNotes {
                source: crate::midi_notes::MidiNoteError::Engine(error.to_string()),
            })?;
        let report = crate::midi_notes::render_notes_to_wav(
            &mut host,
            &pairing.notes,
            track_id,
            path,
            defaults.frames,
            u16::try_from(defaults.channels).unwrap_or(2),
            defaults.sample_rate,
            crate::midi_notes::bpm_millis(loaded.archive.project.bpm),
            defaults.seed,
        )
        .map_err(|source| CliError::MidiNotes { source })?;
        lines.push(report.summary(Some(path)));
    }

    // ---- ① 谱面：把音符写进工程（走既有 Op + 一次带来源的提交）----
    if options.midi_in_write_notes {
        let session = crate::undo::UndoSession::open(
            "<cli:midi-in>",
            "yeban-app",
            loaded.archive.project.clone(),
            crate::host::now_ms(),
        )
        .map_err(|error| CliError::MidiNotes {
            source: crate::midi_notes::MidiNoteError::Refused(error.to_string()),
        })?;
        let port = crate::undo::UndoPort::new(session);
        let report = crate::midi_notes::write_notes(
            &port,
            &loaded.archive.project,
            &pairing,
            selected,
            crate::host::now_ms(),
        )
        .map_err(|source| CliError::MidiNotes { source })?;
        // 会话是权威 ⇒ 落回归档，后续 `--save-as` / 导出看到的是**带音符的那一份**。
        loaded.archive.project = port.project();
        lines.push(report.summary());
    }

    Ok(())
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
fn exported_wav_line(report: &crate::export_wav::WavExportReport, loaded: &Loaded) -> String {
    // ⭐ 模型 SFZ 链条的解码读数（任务单 `#79`）：有内容才追加 ⇒ ⛔ 缺样本不静默
    //    （`sfz(... missing=[...] silent=N)`；没有模型 `sampler` 时本段为空，行不变）。
    let sfz = if report.sfz_decoded.is_empty() {
        String::new()
    } else {
        let regions: usize = report
            .sfz_decoded
            .iter()
            .map(|(_, entry)| entry.regions)
            .sum();
        let samples: usize = report
            .sfz_decoded
            .iter()
            .map(|(_, entry)| entry.decoded_samples)
            .sum();
        let silent: usize = report
            .sfz_decoded
            .iter()
            .map(|(_, entry)| entry.silent_region_count)
            .sum();
        let missing: Vec<&str> = report
            .sfz_decoded
            .iter()
            .flat_map(|(_, entry)| entry.missing_sample_paths.iter().map(String::as_str))
            .collect();
        // ⭐ `#83`：读失败的**可判别**计数（not_found / unsupported / other）。
        //    ⇒ FLAC／OGG 这类"格式不受支持"⛔ 不再与"文件不存在"混成同一个数。
        let not_found: usize = report
            .sfz_decoded
            .iter()
            .map(|(_, entry)| entry.unreadable_not_found_count())
            .sum();
        let unsupported: usize = report
            .sfz_decoded
            .iter()
            .map(|(_, entry)| entry.unreadable_unsupported_format_count())
            .sum();
        let other: usize = report
            .sfz_decoded
            .iter()
            .map(|(_, entry)| entry.unreadable_other_count())
            .sum();
        format!(
            " sfz(tracks={} regions={regions} samples={samples} silent={silent} missing={missing:?} \
             unreadable(not_found={not_found},unsupported={unsupported},other={other}))",
            report.sfz_decoded.len()
        )
    };
    format!(
        "exported-wav: path={} bytes={} frames={} sample_rate={} channels={} bits={} digest={} \
         source={}{sfz} from={}",
        report.path.display(),
        report.bytes,
        report.frames,
        report.sample_rate,
        report.channels,
        report.bits,
        report.digest,
        report.source,
        loaded.source.save_origin(),
    )
}

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

/// 把 `--import-audio` 得到的**容器字节**登记进归档的 CAS 资产池（`assets/{sha256}`）。
///
/// 为什么由 CLI 这一层做（而不是 `crate::import_audio`）：CAS 池的字节**不是** `Op` 的
/// 载荷（模型的 `Op` 全集没有资产变体），导入层只负责"解码 ＋ 登记片段 ＋ 把字节交回来"
/// —— 字节落在**哪一个归档**上，是持有归档的调用方的事（这里是 `loaded.archive`）。
///
/// 幂等：同一哈希已在池里就**不重复登记**（内容寻址 ⇒ 同键必同值）。
/// [`ProjectArchive::assets`] 的文档要求"按哈希升序"，因此用**二分插入**而不是 `push`
/// （`save_archive_file` 内部会重建成 `BTreeMap`，但归档自身的形态要守住它声明的不变式）。
fn register_audio_asset(
    archive: &mut ProjectArchive,
    report: &crate::import_audio::AudioImportReport,
) {
    if archive.assets.iter().any(|(hash, _)| *hash == report.asset) {
        return;
    }
    let index = match archive
        .assets
        .binary_search_by(|(hash, _)| hash.as_str().cmp(report.asset.as_str()))
    {
        Ok(index) | Err(index) => index,
    };
    archive
        .assets
        .insert(index, (report.asset.clone(), report.bytes.clone()));
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
        // ⚠ **行尾归一**：`usage_text()` 是**源码里的多行字面量** ⇒ Windows 工作树（无
        // `.gitattributes` ＋ `core.autocrlf=true`）里它是 CRLF，而 `joined` 产出 `\n`
        // ⇒ ⛔ `trim_end_matches('\n')` 会留下尾随 `\r`。
        assert_eq!(
            joined(&lines),
            usage.replace("\r\n", "\n").trim_end_matches('\n')
        );
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

        // 存下来的确实是演示工程 —— ⭐ `#215`：**除创建时刻外**逐字段相等，
        // 且创建时刻必须真的写进去（不再是 `0` ⇒ BWF 不再写 1970-01-01）。
        let read_back = crate::open::open_project_file(&target).expect("读回");
        assert!(
            read_back.metadata.created_at_unix_ms > 0,
            "新建工程必须写入真实创建时刻（实得 0 ⇒ BWF 起源日期会退回 1970-01-01）"
        );
        let mut expected = crate::bridge::demo_project();
        expected.metadata.created_at_unix_ms = read_back.metadata.created_at_unix_ms;
        assert_eq!(
            read_back, expected,
            "除创建时刻外，存下来的必须逐字段等于演示工程"
        );

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
        assert!(
            filled_project.metadata.created_at_unix_ms > 0,
            "`--project-sample filled --save-as` 同样是创建，必须写真实时刻"
        );
        let mut expected_filled = yeban_model::samples::filled_project();
        expected_filled.metadata.created_at_unix_ms = filled_project.metadata.created_at_unix_ms;
        assert_eq!(filled_project, expected_filled);
        assert_ne!(filled_project, read_back);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 37a（`#215`）：样本分支取时钟**一次**，`--open` 分支**一次都不取**
    // ------------------------------------------------------------------

    /// 两条腿用同一个计数闭包量"时钟被读了几次"：
    ///
    /// - 样本（没有磁盘对应物）⇒ **1** 次，值原样落进 `created_at_unix_ms`；
    /// - `--open`（文件里的创建时刻是既有事实）⇒ **0** 次，且旧文件的 `0` 不被补写。
    ///
    /// 这条防的是"把墙钟塞进打开/保存路径" —— 那会让旧文件在每次打开后都换一个
    /// 创建时刻，导出的 BWF 也就不再逐字节可复现。
    #[test]
    fn the_sample_path_reads_the_clock_once_and_the_file_path_never() {
        let calls = std::cell::Cell::new(0_u32);
        let options = parse(&["--project-sample".to_owned(), "empty".to_owned()]).expect("解析");
        let loaded = load_project_with_clock(&options, || {
            calls.set(calls.get() + 1);
            1_760_000_000_123
        })
        .expect("样本工程");
        assert_eq!(calls.get(), 1, "样本分支必须只取一次时钟");
        assert_eq!(
            loaded.archive.project.metadata.created_at_unix_ms, 1_760_000_000_123,
            "样本分支取到的时刻必须落进工程元数据"
        );

        // 旧文件：`created_at_unix_ms == 0`（本单之前所有新工程的样子）。
        let dir = scratch_dir("clock");
        let path = dir.join("old.yeban");
        crate::save::save_archive_file(
            &yeban_model::container::ProjectArchive {
                project: yeban_model::samples::default_project(),
                history_dag: Vec::new(),
                assets: Vec::new(),
            },
            &path,
        )
        .expect("写旧文件");
        let calls = std::cell::Cell::new(0_u32);
        let options = parse(&["--open".to_owned(), path.display().to_string()]).expect("解析");
        let loaded = load_project_with_clock(&options, || {
            calls.set(calls.get() + 1);
            1
        })
        .expect("打开旧文件");
        assert_eq!(calls.get(), 0, "`--open` 分支不得读时钟");
        assert_eq!(
            loaded.archive.project.metadata.created_at_unix_ms, 0,
            "打开旧文件不得补写创建时刻（补写会改字节 ⇒ 破坏打开-保存的稳定性）"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // 判据 37b: `--project-sample empty` = 0 音轨 ＋ 1 主总线的空工程（BASELINE-002 的取样对象）
    // ------------------------------------------------------------------

    /// `BASELINE-002` 判据句是"**空**工程空闲常驻内存 ≤ 35 MB"。本判据钉住两件事：
    ///
    /// ① `empty` 样本**真的是**没有音轨的空工程（0 条非主总线轨 ＋ 恰 1 条主总线 ——
    ///    `#161` 起主总线是出厂骨架；不是"某个更小的演示工程"）；
    /// ② 无头路径**真的把它打印出来**（`project-counts:` 的轨道数是见证），
    ///    且与 `default` 的读数**不同**（否则就是空转，读数函数返回常数也能"绿"）。
    #[test]
    fn empty_sample_is_a_zero_audio_track_project_with_witness() {
        let empty = Sample::Empty.project();
        assert_eq!(Sample::Empty.name(), "empty", "命令行取值必须是 `empty`");
        assert_eq!(
            empty,
            yeban_model::samples::default_project(),
            "empty 必须就是模型侧的规范空工程样本（不是另一份手写夹具）"
        );
        assert_eq!(empty.tracks.len(), 1, "empty 必须恰有 1 条轨（主总线）");
        assert!(
            !empty.master_bus_track_id.is_nil(),
            "主总线身份不得是 nil（`#161` 起样本自带主总线）"
        );
        let master = empty
            .tracks
            .get(&empty.master_bus_track_id)
            .expect("`master_bus_track_id` 必须指向一条存在的轨");
        assert_eq!(
            master.kind,
            yeban_model::project::TrackKind::Master,
            "主总线轨的类型必须是 `Master`"
        );
        assert_eq!(
            empty.tracks.len() - 1,
            0,
            "empty 的**非主总线轨**必须是 0 条"
        );
        assert_eq!(
            empty.routing_graph.nodes,
            vec![empty.master_bus_track_id],
            "主总线必须在 `routing_graph.nodes` 里（这样它才够得着）"
        );
        assert!(
            empty.clip_pool.is_empty() && empty.scenes.is_empty() && empty.sections.is_empty(),
            "空工程不许夹带片段 / 场景 / 段落"
        );
        empty
            .validate()
            .expect("0 音轨 ＋ 1 主总线工程必须通过模型层校验（合法且可读）");

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
            empty_text.contains("project-counts: tracks-all=1 master-track=1"),
            "empty 的轨道数见证必须是 1（1 主总线 ＋ 0 音轨）:\n{empty_text}"
        );
        assert!(
            empty_text.contains("project-source: sample=empty (内置空工程 0 音轨 ＋ 1 主总线"),
            "来源行必须如实说明这是空工程（不许写成演示工程）:\n{empty_text}"
        );
        // 投影把主总线放在 `master` 那一格，`tracks`（音轨行）仍然是 0。
        assert!(
            empty_text.contains("view-counts: tracks=0 master=1"),
            "0 音轨空工程必须**能**被投影（主总线投影到 `master`）:\n{empty_text}"
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
    // 判据 40b: 动作开关路由的**唯一事实源**
    //           (`ACTION_SWITCHES` / `Options::batch()` / `Options::wants_gui()`)
    //
    // 本切片修的缺陷: `batch()` 曾是一张手抄清单, 漏掉了 `--export-wav` /
    // `--record-wav` / `--sfz` / `--midi-in` / `--midi-in-render` /
    // `--midi-in-write-notes` 六个动作开关 ⇒ 它们"文档说在真二进制上跑过、
    // 实际却开窗口什么都不做". 修法 = 让 `batch()` 从这一张表派生
    // (⛔ 不是补第二张清单).
    // ------------------------------------------------------------------

    /// 把参数折成 `Vec<String>`（判据里反复用）。
    fn argv(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|item| (*item).to_owned()).collect()
    }

    /// 单独解析一个动作开关；需要取值的开关补一个**探针值**。
    fn parse_lone_switch(name: &str) -> Result<Options, ParseError> {
        match parse(&argv(&[name])) {
            Err(ParseError::MissingValue(_)) => parse(&argv(&[name, "probe"])),
            other => other,
        }
    }

    /// 本构建里由**编译期门禁**（feature 没开）拒绝的解析错误。
    ///
    /// 它们在默认构建里"解析不成"是"这段代码没编进来"，⛔ 不是"路由错了"
    /// ⇒ 判据用 [`ActionSwitch::grant`] 直接设字段来单独证明这些开关的**路由**归属。
    fn is_compile_time_gate(error: &ParseError) -> bool {
        matches!(
            error,
            ParseError::AlsNotCompiled
                | ParseError::LogicNotCompiled
                | ParseError::UiMcpHttpNotCompiled
                | ParseError::McpHttpNotCompiled
        )
    }

    /// 判据 A: 表里**每一个**动作开关单独给出时，`batch()` 的取值必须与它在表里的
    /// `path` 一致；两条导入是**点名**的 `WindowlessOnly` 例外。
    ///
    /// 红字（修复前）: `--export-wav` / `--record-wav` / `--sfz` / `--midi-in` /
    /// `--midi-in-render` / `--midi-in-write-notes` 六条 —— 表说 `Batch`，`batch()` 说 `false`。
    #[test]
    fn every_action_switch_routes_exactly_as_the_shared_table_says() {
        let mut failures: Vec<String> = Vec::new();
        for switch in Options::action_switches() {
            // ① 直接设字段（绕过编译期门禁）: `present` / `grant` / 路由必须三方一致。
            let mut granted = Options::default();
            (switch.grant)(&mut granted);
            if !(switch.present)(&granted) {
                failures.push(format!(
                    "`{}`: grant 设了字段, present 却说没给",
                    switch.name
                ));
            }
            if granted.batch() != switch.selects_batch() {
                failures.push(format!(
                    "`{}`: batch()={} 但表说 {}",
                    switch.name,
                    granted.batch(),
                    switch.selects_batch()
                ));
            }
            if granted.wants_gui() != !switch.selects_batch() {
                failures.push(format!(
                    "`{}`: wants_gui()={} 必须是 batch() 的取反",
                    switch.name,
                    granted.wants_gui()
                ));
            }

            // ② 走**解析器**（用户真正的入口）: 能解析的必须选对路径。
            match parse_lone_switch(switch.name) {
                Ok(parsed) => {
                    if parsed.batch() != switch.selects_batch() {
                        failures.push(format!(
                            "`{}`: 解析后 batch()={} 但表说 {}",
                            switch.name,
                            parsed.batch(),
                            switch.selects_batch()
                        ));
                    }
                }
                // `--headless-idle` 单独给是**成对性**用法错误（必须配 `--idle-seconds`），
                // 三条编译期门禁开关在默认构建里也解析不成 —— 两者的**路由**归属
                // 都已由 ① 证明，这里只登记"解析不成"的原因。
                Err(error)
                    if is_compile_time_gate(&error)
                        || matches!(error, ParseError::IdleSecondsMissing) => {}
                Err(other) => failures.push(format!(
                    "`{}`: 单独解析失败 (不该是这个原因): {other}",
                    switch.name
                )),
            }
        }
        assert!(
            failures.is_empty(),
            "动作开关路由与唯一事实源 (`ACTION_SWITCHES`) 漂移 ({} 条):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// 判据 A 的**例外**（`SwitchPath::WindowlessOnly`）: 例外必须**恰好**是
    /// 两条导入，且"只给导入 = 走 GUI ⇒ 导入不会发生"的既存口径仍然成立
    /// （⛔ 本切片**不改**这两条的路径: 它们的行为已被 `--help` 与既有判据钉住）。
    #[test]
    fn the_two_imports_are_the_only_named_windowless_only_exception() {
        let exceptions: Vec<&str> = Options::action_switches()
            .iter()
            .filter(|switch| !switch.selects_batch())
            .map(|switch| switch.name)
            .collect();
        assert_eq!(
            exceptions,
            vec!["--import-midi", "--import-audio"],
            "⛔ 例外必须**恰好**是两条导入: 多一条 = 悄悄少了一条无窗口动作; 少一条 = 悄悄换了例外"
        );

        // 只给导入: 不选批处理路径 ⇒ 进程走 GUI ⇒ 导入不会发生（既存口径）。
        for name in ["--import-midi", "--import-audio"] {
            let entry = Options::action_switches()
                .iter()
                .find(|switch| switch.name == name)
                .expect("例外必须在表里被点名");
            let mut options = Options::default();
            (entry.grant)(&mut options);
            assert!(!options.batch(), "`{name}` 自己**不**选批处理路径");
            assert!(options.wants_gui(), "`{name}` ⇒ 进程走 GUI ⇒ 导入不会发生");
        }

        // 无窗口路径上的**执行**仍然成立: 导入 + `--save-as` = 批处理路径。
        for name in ["--import-midi", "--import-audio"] {
            let parsed = parse(&argv(&[name, "take.bin", "--save-as", "song.yeban"]))
                .expect("导入 + --save-as 是合法组合");
            assert!(
                parsed.batch() && !parsed.wants_gui(),
                "`{name}` + `--save-as`: 导入必须在无窗口路径上真的执行\n{parsed:?}"
            );
        }
    }

    /// 判据 A 的**后果**: `--midi-in-write-notes` 单独给现在会**到达** `run_batch` 里的
    /// 用法守卫（它必须与 `--midi-in` 一起给）。修路由之前，这条守卫住在 GUI 路径
    /// 到不了的 `run_batch` 里 —— 用户连那条用法错误都看不到。
    #[test]
    fn a_lone_midi_in_modifier_now_reaches_its_usage_guard_instead_of_the_gui() {
        let options = parse(&argv(&["--midi-in-write-notes"])).expect("解析");
        assert!(options.batch(), "⇒ `run_batch` 会被调用（守卫才存在）");
        let error = run_batch(&options).expect_err("单独给必须是用法错误");
        assert!(
            matches!(error, CliError::Usage(_)),
            "必须是用法错误而不是别的: {error:?}"
        );
        let text = error.to_string();
        assert!(
            text.contains("--midi-in-write-notes") && text.contains("--midi-in"),
            "错误信息必须点名两个开关: {text}"
        );
    }

    /// 判据 B: `usage_text()` 的**示例块**里每一条点了动作开关的 `yeban-app …` 行，
    /// 解析后都必须选无窗口路径。⛔ 不许出现"帮助说在真二进制上跑过、其实开窗口"的行。
    ///
    /// 红字（修复前）: 5 条 —— `:499` / `:500` / `:501` / `:502` / `:506`
    /// （`--record-wav` / `--sfz` / `--export-wav` 的组合）。
    #[test]
    fn every_usage_example_that_names_an_action_switch_selects_batch() {
        const HEADING: &str = "示例 (全部已在真二进制上跑过):";
        let usage = usage_text();
        let lines: Vec<&str> = usage
            .lines()
            .skip_while(|line| line.trim() != HEADING)
            .skip(1)
            .filter(|line| line.trim_start().starts_with("yeban-app"))
            .collect();
        assert!(!lines.is_empty(), "示例块必须存在");

        let action_names: Vec<&str> = Options::action_switches()
            .iter()
            .map(|switch| switch.name)
            .collect();
        let mut checked = 0usize;
        let mut failures: Vec<String> = Vec::new();
        for line in lines {
            // 去掉 `# 注释` 与 `yeban-app` 这个词，得到真正的 argv。
            let args: Vec<String> = line
                .split('#')
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .filter(|token| *token != "yeban-app")
                .map(str::to_owned)
                .collect();
            let named: Vec<&str> = args
                .iter()
                .filter(|token| action_names.contains(&token.as_str()))
                .map(String::as_str)
                .collect();
            if named.is_empty() {
                // 纯展示行（例如只给 `--theme`）: 不点名动作，不受本判据约束。
                continue;
            }
            match parse(&args) {
                Ok(parsed) => {
                    if !parsed.batch() {
                        failures.push(format!(
                            "`yeban-app {}` 点了动作开关 {} 却进 GUI 路径",
                            args.join(" "),
                            named.join(" / ")
                        ));
                    }
                }
                Err(error) if is_compile_time_gate(&error) => {
                    // 默认构建里 `--enable-ui-mcp-http` / `--export-logic` 示例行本来
                    // 就被编译期门禁拒; 它们的路由归属由判据 A 的表覆盖。
                }
                Err(other) => {
                    failures.push(format!("`yeban-app {}` 解析失败: {other}", args.join(" ")))
                }
            }
            checked += 1;
        }
        assert!(
            failures.is_empty(),
            "示例块里有 {} 条点了动作开关却选 GUI 路径的行:\n{}",
            failures.len(),
            failures.join("\n")
        );
        assert!(
            checked >= 5,
            "至少必须真的检查到 5 条点名动作开关的示例行 (实得 {checked})"
        );
    }

    /// 判据 D（结构性）: 表里 `Batch` 的条数 = 单独给出后真的选 `batch()` 的动作开关条数；
    /// `wants_gui()` 仍是 `batch()` 的取反。
    ///
    /// **计数规则**（单位 = [`Options::action_switches()`] 里的**表项条数**，
    /// ⛔ 不是 `--help` 文本的行数、⛔ 不是源码里的条件数）:
    /// `table_batch` = 表里 `path == Batch` 的条数;
    /// `measured` = 对表里每一项各构造一份 `Options`、只 `grant` 那一个开关后
    /// `batch() && !wants_gui()` 为真的条数。两者必须相等。
    #[test]
    fn the_number_of_action_switches_selecting_batch_equals_the_table() {
        let table_batch = Options::action_switches()
            .iter()
            .filter(|switch| switch.selects_batch())
            .count();
        let table_exceptions = Options::action_switches().len() - table_batch;
        let measured = Options::action_switches()
            .iter()
            .filter(|switch| {
                let mut options = Options::default();
                (switch.grant)(&mut options);
                options.batch() && !options.wants_gui()
            })
            .count();
        assert_eq!(
            measured, table_batch,
            "表里 Batch={table_batch} / 例外={table_exceptions} / 实测选 batch 的={measured}"
        );
        // 本切片明确要求新增的六条必须在 Batch 档（⛔ 少一条 = 能力仍然缺失）。
        for name in [
            "--export-wav",
            "--record-wav",
            "--sfz",
            "--midi-in",
            "--midi-in-render",
            "--midi-in-write-notes",
        ] {
            let entry = Options::action_switches()
                .iter()
                .find(|switch| switch.name == name)
                .unwrap_or_else(|| panic!("`{name}` 必须在唯一事实源表里"));
            assert!(entry.selects_batch(), "`{name}` 必须自己就选无窗口路径");
        }
    }

    /// 判据 E: `usage_text()` 的 `运行形态:` 块里那份"无窗口开关"名单必须**恰好**
    /// 等于共享表 [`ACTION_SWITCHES`] 里 `path == Batch` 的动作开关，减去四条
    /// **在别处单独讲了**的开关：
    ///
    /// - `--help` / `--version`：短路命令，`运行形态:` 块自己的末句就点明了它们
    ///   "不打印握手行"；
    /// - `--headless-idle` / `--enable-ui-mcp-http`：同一档"会构造软件窗口"的形态
    ///   （`--headless-idle` 在名单下一句里被点名为**另一档**，UI 控制面在它自己的
    ///   条目里单讲），因此**不**属于那句"并打印握手行 {handshake}"的名单。
    ///
    /// 判据拿**渲染后的文本**（`usage_text()` 的返回值）与**共享表**对账 ⇒
    /// 名单无论是手写还是派生，只要与表漂移就会红。
    ///
    /// **它证明什么**：名单里每一条都是表里 `Batch` 的动作开关，且表里每一条
    /// `Batch`（除上述四条）都在名单里 —— 名单不会再"只报旧子集"。
    /// **它不证明什么**：四条被排除的开关**在别处**的措辞仍可能单独漂移（本判据只认
    /// "它们不在名单里"这个事实，不读它们各自的段落）；也不检查名单的**顺序**、
    /// 不检查句式（自由文本）。
    ///
    /// 红字（修复前）: 漏 7 条 —— `--print-theme` / `--export-wav` / `--record-wav` /
    /// `--sfz` / `--midi-in` / `--midi-in-render` / `--midi-in-write-notes`。
    #[test]
    fn the_running_form_prose_names_exactly_the_windowless_action_switches() {
        // 名单的边界标记：夹在 `(` 与 `)` 之间，锚在 `运行形态:` 块那句定义上。
        const OPEN: &str = "任一\"无窗口开关\"(";
        // 单位 = **表项条数**（`Options::action_switches()`），⛔ 不是源码行数。
        const SPOKEN_FOR_ELSEWHERE: [&str; 4] =
            ["--help", "--version", "--headless-idle", UI_MCP_HTTP_SWITCH];

        let usage = usage_text();
        let start = usage
            .find(OPEN)
            .unwrap_or_else(|| panic!("`运行形态:` 块里必须有 `{OPEN}` 这份名单"))
            + OPEN.len();
        let end = usage[start..]
            .find(')')
            .map(|offset| start + offset)
            .unwrap_or_else(|| panic!("名单必须用 `)` 收尾"));
        let named: Vec<&str> = usage[start..end]
            .split('/')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();

        let expected: Vec<&str> = Options::action_switches()
            .iter()
            .filter(|switch| switch.selects_batch() && !SPOKEN_FOR_ELSEWHERE.contains(&switch.name))
            .map(|switch| switch.name)
            .collect();

        let missing: Vec<&str> = expected
            .iter()
            .copied()
            .filter(|name| !named.contains(name))
            .collect();
        let unexpected: Vec<&str> = named
            .iter()
            .copied()
            .filter(|name| !expected.contains(name))
            .collect();
        assert!(
            missing.is_empty() && unexpected.is_empty(),
            "`运行形态:` 的名单与唯一事实源 (`ACTION_SWITCHES`) 漂移:\n\
             漏掉 (表里有、名单没有，{} 条): {missing:?}\n\
             多出/错名 (名单有、表里没有或不在本档，{} 条): {unexpected:?}",
            missing.len(),
            unexpected.len()
        );
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
    // 判据 46b: --enable-ui-mcp-http 的两条前置 + 防假绿守卫 [ARCH-UI-004] [MUST-GATE-009]
    //            (与判据 46 逐条同款, 因为两条控制面说的是同一条红线;
    //             差别只有"哪个进程形态承载执行面"与"哪个 feature 门"）
    // ------------------------------------------------------------------

    #[test]
    fn the_ui_mcp_http_switch_is_opt_in_names_its_feature_and_owns_its_process_shape() {
        let args = |raw: &[&str]| {
            raw.iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };

        // ① 字面值与 feature 名是**契约**（`crates/yeban-ui-mcp` 的
        //    `transport::ENABLE_HTTP_FLAG` / `HTTP_FEATURE_NAME` 是权威定义，
        //    逐字节相同这件事由 `tests/live_ui_mcp.rs` 的判据 18b 对账）。
        assert_eq!(UI_MCP_HTTP_SWITCH, "--enable-ui-mcp-http");
        assert_eq!(UI_MCP_HTTP_FEATURE, "ui-mcp-http");
        assert_eq!(UI_MCP_PATH, "/ui-mcp");

        // ② 编译期那道开关：默认构建里这个开关**不存在**，给了就是用法错误。
        if cfg!(feature = "ui-mcp-http") {
            let parsed = parse(&args(&["--enable-ui-mcp-http"])).expect("带 feature 时必须被接受");
            assert!(parsed.ui_mcp_http);
            // 它是**无窗口**路径（软件平台上的活控件树），因此绝不能落到 run_batch，
            // 而且 `--idle-seconds` 对它是有意义的（给了就服务到点自动停机）。
            assert!(parsed.batch() && !parsed.wants_gui(), "{parsed:?}");
            let bounded = parse(&args(&["--enable-ui-mcp-http", "--idle-seconds", "3"]))
                .expect("--idle-seconds 对 UI 控制面有意义");
            assert_eq!(bounded.idle_seconds, Some(3));
        } else {
            let error = parse(&args(&["--enable-ui-mcp-http"])).expect_err("默认构建里必须被拒");
            assert_eq!(error, ParseError::UiMcpHttpNotCompiled);
            assert!(
                error.to_string().contains(UI_MCP_HTTP_FEATURE),
                "错误必须点名缺的是哪个 feature: {error}"
            );
            // `--idle-seconds` 单独给仍然是用法错误（默认构建里本开关根本进不来，
            // 所以这条前置仍然由 `--headless-idle` 独占）。
            assert_eq!(
                parse(&args(&["--idle-seconds", "3"])),
                Err(ParseError::IdleSecondsWithoutHeadlessIdle)
            );
        }

        // ③ 不接受取值。
        assert_eq!(
            parse(&args(&["--enable-ui-mcp-http=1"])),
            Err(ParseError::UnexpectedValue(UI_MCP_HTTP_SWITCH))
        );

        // ④ 与"会被静默丢掉"或"不在同一进程形态"的开关同给 = 用法错误。
        //    带 feature 时是 UiMcpHttpConflict；不带 feature 时更早一步就被挡住。
        for (flag, name) in [
            (vec!["--headless"], "--headless"),
            (vec!["--dump-elements"], "--dump-elements"),
            (vec!["--print-shortcuts"], "--print-shortcuts"),
            (vec!["--export-elements", "e.txt"], "--export-elements"),
            (vec!["--export-midi", "m.mid"], "--export-midi"),
            (vec!["--export-als", "a.als"], ALS_EXPORT_SWITCH),
            (vec!["--save-as", "b.yeban"], "--save-as"),
        ] {
            let mut raw = vec!["--enable-ui-mcp-http"];
            raw.extend(flag.iter().copied());
            let expected = if cfg!(feature = "ui-mcp-http") {
                ParseError::UiMcpHttpConflict(name)
            } else {
                ParseError::UiMcpHttpNotCompiled
            };
            assert_eq!(parse(&args(&raw)), Err(expected), "{raw:?} 必须被拒");
        }

        // ④c 两个控制面开关同给：带 feature 时是本侧的组合规则（两者**不在同一个进程形态**
        //     里）；不带 feature 时更早一步就被"这个构建里没有它"挡住。
        //     这一条同时钉住 parse() 里的**检查顺序**（见那里的注释）：UI 那块在前，
        //     因此"只编了 in-process-mcp"的构建里这个开关也不会被静默忽略。
        let pair = vec!["--enable-ui-mcp-http", "--enable-mcp-http"];
        let expected = if cfg!(feature = "ui-mcp-http") {
            ParseError::UiMcpHttpConflict(MCP_HTTP_SWITCH)
        } else {
            ParseError::UiMcpHttpNotCompiled
        };
        assert_eq!(parse(&args(&pair)), Err(expected), "{pair:?} 必须被拒");

        // ④b 与 `--headless-idle` 同给是**允许**的（同一档进程形态；UI 控制面更强）。
        if cfg!(feature = "ui-mcp-http") {
            let both = parse(&args(&[
                "--enable-ui-mcp-http",
                "--headless-idle",
                "--idle-seconds",
                "5",
            ]))
            .expect("--headless-idle 可以与 --enable-ui-mcp-http 同给");
            assert!(both.ui_mcp_http && both.headless_idle);
        }

        // ⑤ 防假绿第二道：`Options` 被**直接构造**（绕过 parse）时，`run_batch` 也必须拒绝 ——
        //    那条路径零 Slint 依赖，一个控件树对象都不构造，而 ui/* 全长在那棵树上。
        let smuggled = Options {
            ui_mcp_http: true,
            ..Options::default()
        };
        assert!(matches!(
            run_batch(&smuggled),
            Err(CliError::UiMcpHttpNotBatch)
        ));
        assert_eq!(CliError::UiMcpHttpNotBatch.exit_code(), EXIT_UI);
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
    // 判据 50: --inspect-logic 是**独立只读探针** (#143)
    // ------------------------------------------------------------------

    #[test]
    fn the_inspect_logic_probe_is_standalone_and_opt_in() {
        let args = |raw: &[&str]| {
            raw.iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };

        // ① 字面值与 feature 名是**契约**。
        assert_eq!(LOGIC_INSPECT_SWITCH, "--inspect-logic");
        assert_eq!(LOGIC_INSPECT_FEATURE, "experimental-logic-import");

        // ② 没有这个开关 ⇒ None（照常走 `parse`，⛔ 不影响既有命令行）。
        assert_eq!(parse_inspect_logic(&args(&["--headless"])), None);

        // ③ 编译期那道开关：默认构建里给了它 = 点名 feature 的用法错误。
        let expected = if cfg!(feature = "experimental-logic-import") {
            Some(Ok(PathBuf::from("/tmp/demos")))
        } else {
            Some(Err(ParseError::InspectLogicNotCompiled))
        };
        assert_eq!(
            parse_inspect_logic(&args(&["--inspect-logic", "/tmp/demos"])),
            expected
        );
        if cfg!(feature = "experimental-logic-import") {
            assert_eq!(
                parse_inspect_logic(&args(&["--inspect-logic=/tmp/demos"])),
                Some(Ok(PathBuf::from("/tmp/demos"))),
                "内联取值必须被接受"
            );
        } else {
            let error = expected.expect("Some").expect_err("默认构建必须拒");
            assert_eq!(error.exit_code(), EXIT_USAGE);
            let text = error.to_string();
            assert!(text.contains(LOGIC_INSPECT_SWITCH), "必须点名开关: {text}");
            assert!(
                text.contains(LOGIC_INSPECT_FEATURE),
                "必须点名 feature: {text}"
            );
        }

        // ④ 缺取值 / 重复 / 与别的参数组合 = 用法错误（⛔ 不静默忽略）。
        assert_eq!(
            parse_inspect_logic(&args(&["--inspect-logic"])),
            Some(Err(ParseError::MissingValue(LOGIC_INSPECT_SWITCH)))
        );
        assert_eq!(
            parse_inspect_logic(&args(&["--inspect-logic", "--headless"])),
            Some(Err(ParseError::MissingValue(LOGIC_INSPECT_SWITCH))),
            "下一个 token 是开关 ⇒ 缺取值（⛔ 不把它当路径）"
        );
        assert_eq!(
            parse_inspect_logic(&args(&["--inspect-logic=a", "--inspect-logic=b"])),
            Some(Err(ParseError::DuplicateOption(LOGIC_INSPECT_SWITCH)))
        );
        assert_eq!(
            parse_inspect_logic(&args(&[
                "--inspect-logic",
                "/tmp/demos",
                "--save-as",
                "x.yeban"
            ])),
            Some(Err(ParseError::InspectLogicConflict(
                "--save-as".to_owned()
            ))),
            "组合会把 --save-as 静默丢掉"
        );

        // ⑤ `--help` 短路：交给 `parse`，帮助永远打得开。
        assert_eq!(
            parse_inspect_logic(&args(&["--help", "--inspect-logic", "x"])),
            None
        );

        // ⑥ 用法文本必须点名开关与 feature。
        let usage = usage_text();
        assert!(usage.contains(LOGIC_INSPECT_SWITCH), "用法文本缺开关");
        assert!(usage.contains(LOGIC_INSPECT_FEATURE), "用法文本缺 feature");
    }

    // ------------------------------------------------------------------
    // 判据 51: --import-logic 是**独立导入命令** (#149 / 刀 2)
    // ------------------------------------------------------------------

    #[test]
    fn the_import_logic_command_is_standalone_and_opt_in() {
        let args = |raw: &[&str]| {
            raw.iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };

        // ① 字面值与 feature 名是**契约**。
        assert_eq!(IMPORT_LOGIC_SWITCH, "--import-logic");
        assert_eq!(IMPORT_LOGIC_FEATURE, "experimental-logic-import");

        // ② 没有这个开关 ⇒ None（照常走 `parse`，⛔ 不影响既有命令行）。
        assert_eq!(parse_import_logic(&args(&["--headless"])), None);

        // ③ 编译期那道开关：默认构建里给了它 = 点名 feature 的用法错误。
        let expected = if cfg!(feature = "experimental-logic-import") {
            Some(Ok((
                PathBuf::from("/tmp/Swing!.logicx"),
                PathBuf::from("/tmp/out.yeban"),
            )))
        } else {
            Some(Err(ParseError::ImportLogicNotCompiled))
        };
        assert_eq!(
            parse_import_logic(&args(&[
                "--import-logic",
                "/tmp/Swing!.logicx",
                "--save-as",
                "/tmp/out.yeban"
            ])),
            expected
        );
        if cfg!(feature = "experimental-logic-import") {
            assert_eq!(
                parse_import_logic(&args(&[
                    "--import-logic=/tmp/Swing!.logicx",
                    "--save-as=/tmp/out.yeban",
                    "--headless"
                ])),
                expected,
                "内联取值 + --headless 必须被接受"
            );
        } else {
            let error = expected.expect("Some").expect_err("默认构建必须拒");
            assert_eq!(error.exit_code(), EXIT_USAGE);
            let text = error.to_string();
            assert!(text.contains(IMPORT_LOGIC_SWITCH), "必须点名开关: {text}");
            assert!(
                text.contains(IMPORT_LOGIC_FEATURE),
                "必须点名 feature: {text}"
            );
        }

        // ④ 缺 `--save-as` / 缺取值 / 重复 / 与别的参数组合 = 用法错误。
        assert_eq!(
            parse_import_logic(&args(&["--import-logic", "/tmp/Swing!.logicx"])),
            Some(Err(ParseError::ImportLogicOutputMissing)),
            "没有 --save-as ⇒ 不猜落点"
        );
        assert_eq!(
            parse_import_logic(&args(&["--import-logic"])),
            Some(Err(ParseError::MissingValue(IMPORT_LOGIC_SWITCH)))
        );
        assert_eq!(
            parse_import_logic(&args(&[
                "--import-logic",
                "/tmp/Swing!.logicx",
                "--import-logic",
                "/tmp/ocean eyes.logicx",
                "--save-as",
                "/tmp/out.yeban"
            ])),
            Some(Err(ParseError::DuplicateOption(IMPORT_LOGIC_SWITCH)))
        );
        assert_eq!(
            parse_import_logic(&args(&[
                "--import-logic",
                "/tmp/Swing!.logicx",
                "--save-as",
                "/tmp/out.yeban",
                "--export-midi",
                "/tmp/out.mid"
            ])),
            Some(Err(ParseError::ImportLogicConflict(
                "--export-midi".to_owned()
            ))),
            "组合会把 --export-midi 静默丢掉"
        );

        // ⑤ `--help` 短路：交给 `parse`，帮助永远打得开。
        assert_eq!(
            parse_import_logic(&args(&["--help", "--import-logic", "x"])),
            None
        );

        // ⑥ 用法文本必须点名开关与 feature。
        let usage = usage_text();
        assert!(usage.contains(IMPORT_LOGIC_SWITCH), "用法文本缺开关");
        assert!(usage.contains(IMPORT_LOGIC_FEATURE), "用法文本缺 feature");
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

    /// `--record-wav`：录一段测试音 ⇒ 输出行报 `frames`／`dropped`／`source`，文件可回读且非静音。
    #[test]
    fn record_wav_writes_a_readable_take_and_says_where_the_samples_came_from() {
        let dir = scratch_dir("record-wav");
        let target = dir.join("take.wav");
        let lines = run_batch(
            &parse(&[
                "--record-wav".to_owned(),
                target.display().to_string(),
                "--record-seconds".to_owned(),
                "0.5".to_owned(),
                "--record-source".to_owned(),
                "tone".to_owned(),
            ])
            .expect("解析"),
        )
        .expect("录音必须成功");
        let line = lines
            .iter()
            .find(|line| line.starts_with("recorded-wav:"))
            .expect("必须有 recorded-wav 报告行");
        assert!(
            line.contains("frames=24000"),
            "0.5 秒 @48kHz = 24000 帧: {line}"
        );
        assert!(line.contains("dropped=0"), "默认环足够大 ⇒ 不丢弃: {line}");
        assert!(
            line.contains("source=in-memory-sine(440Hz)"),
            "必须如实说明样本来源（⛔ 不是声卡）: {line}"
        );
        let (format, buffer) = yeban_render::wav::read_plain_wav(&target).expect("可回读");
        let frames = u64::try_from(buffer.len() / usize::from(format.channels)).expect("帧数");
        assert_eq!(frames, 24_000);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `--record-seconds` 单独给 ⇒ **用法错误**（⛔ 不静默忽略）。
    #[test]
    fn record_seconds_without_record_wav_is_a_usage_error() {
        let error =
            run_batch(&parse(&["--record-seconds".to_owned(), "3".to_owned()]).expect("解析"))
                .expect_err("单独给 --record-seconds 必须报用法错误");
        assert_eq!(error.exit_code(), EXIT_USAGE);
    }

    /// `--record-wav` 的**拒绝不留半成品**：目标是目录 ⇒ 失败且不新增文件。
    #[test]
    fn record_wav_refuses_an_unwritable_target_without_a_half_file() {
        let dir = scratch_dir("record-bad");
        let target = dir.join("a-directory.wav");
        std::fs::create_dir_all(&target).expect("建目录");
        let error = run_batch(
            &parse(&["--record-wav".to_owned(), target.display().to_string()]).expect("解析"),
        )
        .expect_err("目录不可作为目标");
        assert!(
            error.to_string().contains("录音"),
            "必须转达录音层裁决: {error}"
        );
        let entries: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["a-directory.wav".to_owned()], "⛔ 不留半成品");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `--sfz`：装载一份最小 SFZ ⇒ 报告里的区域数与文件内容一致，且输出行**如实**说明
    /// 样本来源，并在**没请求**导出时报 `wired_to_export=no`。
    #[test]
    fn sfz_load_reports_the_region_count_and_says_what_is_not_wired() {
        let dir = scratch_dir("sfz-load");
        let sfz = dir.join("two-regions.sfz");
        std::fs::write(
            &sfz,
            "<region> sample=a.wav key=60\n<region> sample=b.wav key=64\n",
        )
        .expect("写夹具");
        let lines =
            run_batch(&parse(&["--sfz".to_owned(), sfz.display().to_string()]).expect("解析"))
                .expect("装载必须成功");
        let line = lines
            .iter()
            .find(|line| line.starts_with("loaded-sfz:"))
            .expect("必须有 loaded-sfz 报告行");
        assert!(line.contains("regions=2"), "区域数必须与文件一致: {line}");
        assert!(
            line.contains("sample_source=in-memory-sine(440Hz)"),
            "必须如实说明样本来源（⛔ 不是 wav）: {line}"
        );
        assert!(
            line.contains("wired_to_export=no"),
            "必须如实说明尚未接到导出: {line}"
        );
        assert!(line.contains("bytes="), "必须报告字节数: {line}");
    }

    /// `--sfz` ＋ `--export-wav`：两行都在（`loaded-sfz` 报 `wired_to_export=yes`、
    /// `exported-wav` 报 `source=sfz(…)` 且**明说内存正弦**），并且导出**真的**成功。
    #[test]
    fn sfz_plus_export_wav_wires_the_instrument_into_the_export_and_says_so() {
        let dir = scratch_dir("sfz-export");
        let (container, project) = write_container_file(&dir, "song.yeban");
        // 按工程里的音高逐键写区域 ⇒ SFZ 一定匹配得上（否则只会得到静音）。
        let mut pitches: Vec<u8> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values().map(|note| note.pitch))
            .collect();
        pitches.sort_unstable();
        pitches.dedup();
        let sfz = dir.join("keys.sfz");
        let text: String = pitches
            .iter()
            .map(|pitch| format!("<region> sample=a.wav key={pitch}\n"))
            .collect();
        std::fs::write(&sfz, text).expect("写夹具");
        let target = dir.join("with-sfz.wav");
        let lines = run_batch(
            &parse(&[
                "--open".to_owned(),
                container.display().to_string(),
                "--sfz".to_owned(),
                sfz.display().to_string(),
                "--export-wav".to_owned(),
                target.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("带 SFZ 的导出必须成功");
        let loaded = lines
            .iter()
            .find(|line| line.starts_with("loaded-sfz:"))
            .expect("必须有 loaded-sfz 行");
        assert!(
            loaded.contains("wired_to_export=yes"),
            "接到导出时必须如实报 yes: {loaded}"
        );
        assert!(
            loaded.contains("sample_source=in-memory-sine(440Hz)"),
            "必须明说样本来源: {loaded}"
        );
        let exported = lines
            .iter()
            .find(|line| line.starts_with("exported-wav:"))
            .expect("必须有 exported-wav 行");
        assert!(
            exported.contains("source=sfz"),
            "导出行必须明说走了 SFZ: {exported}"
        );
        // ⭐ 判据 b 的证据：本夹具的 `sample=a.wav` **不存在**（相对 SFZ 目录）⇒ 必须**如实回退**，
        // 且把原因写进来源文案（⛔ 不假装读到了文件、⛔ 不 panic）。
        assert!(
            exported.contains("in-memory-sine"),
            "读不到样本文件时必须如实回退到内存正弦: {exported}"
        );
        assert!(
            exported.contains("回退"),
            "回退必须把原因写进来源文案: {exported}"
        );
        assert!(target.exists(), "导出必须真的落盘");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `--sfz` 指向不存在的文件 ⇒ **报错、⛔ 不 panic、且导出目标不存在**（不留半成品）。
    #[test]
    fn a_missing_sfz_prevents_the_export_and_leaves_no_target() {
        let dir = scratch_dir("sfz-export-bad");
        let (container, _project) = write_container_file(&dir, "song.yeban");
        let target = dir.join("never.wav");
        let error = run_batch(
            &parse(&[
                "--open".to_owned(),
                container.display().to_string(),
                "--sfz".to_owned(),
                dir.join("missing.sfz").display().to_string(),
                "--export-wav".to_owned(),
                target.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect_err("SFZ 不存在必须失败");
        assert_eq!(
            error.exit_code(),
            EXIT_OPEN,
            "与 `--open` 同族：输入文件不可用"
        );
        assert!(!target.exists(), "失败 ⇒ 导出目标一个字节都不许有");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `--sfz` 的**失败路径**：文件不存在 ⇒ 与 `--open` 同族退出码，⛔ 不 panic，且**不新增任何文件**。
    #[test]
    fn sfz_load_refuses_a_missing_file_without_writing_anything() {
        let dir = scratch_dir("sfz-missing");
        let missing = dir.join("nope.sfz");
        let before: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        let error =
            run_batch(&parse(&["--sfz".to_owned(), missing.display().to_string()]).expect("解析"))
                .expect_err("文件不存在必须失败");
        assert_eq!(
            error.exit_code(),
            EXIT_OPEN,
            "与 `--open` 同族：输入文件不可用"
        );
        assert!(
            error.to_string().contains("装载 SFZ"),
            "必须转达装载层的精确裁决: {error}"
        );
        let after: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(after, before, "失败 ⇒ 一个文件都不许新增（⛔ 不留半成品）");
    }

    /// `--sfz` 的**实测登记（一个真缺口）**：SFZ 解析器**容忍**自由文本 ⇒ 垃圾输入**不报错**，
    /// 而是被当作**零区域**文件接受（`regions=0`）⇒ 用户不会得到"这份 SFZ 不对"的提示。
    ///
    /// ⚠ 本判据**钉住今天的事实**并把缺口写进输出行；它变红之日 = 解析器开始拒绝垃圾输入之时，
    /// 届时更新本判据（⛔ 不是删掉它）。⛔ 本判据不声称"零区域 = 合法乐器"。
    #[test]
    fn sfz_load_registers_that_free_text_is_accepted_as_zero_regions() {
        let dir = scratch_dir("sfz-garbage");
        let junk = dir.join("junk.sfz");
        std::fs::write(&junk, "this is not sfz at all").expect("写垃圾");
        let lines =
            run_batch(&parse(&["--sfz".to_owned(), junk.display().to_string()]).expect("解析"))
                .expect("引擎的解析器容忍自由文本（本机实测）");
        let line = lines
            .iter()
            .find(|line| line.starts_with("loaded-sfz:"))
            .expect("必须有报告行");
        assert!(
            line.contains("regions=0"),
            "垃圾输入今天被当成零区域（缺口登记）: {line}"
        );
        assert!(
            line.contains("wired_to_export=no"),
            "必须继续如实说明未接到导出: {line}"
        );
    }

    /// `--import-midi`：导入一段真 SMF（由 `--export-midi` 现造）⇒ 音符数按文件增加，
    /// **来源标记是 `midi-input`**（本切片的核心语义），且 `--save-as` 保存的是**导入后**的工程。
    #[test]
    fn import_midi_adds_the_notes_and_marks_the_origin_as_midi_input() {
        let dir = scratch_dir("import-midi");
        let (source, project) = write_container_file(&dir, "song.yeban");
        let mid = dir.join("take.mid");
        run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--export-midi".to_owned(),
                mid.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("先导出一份 SMF 当输入");

        let before: usize = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .map(std::collections::BTreeMap::len)
            .sum();
        assert!(before > 0, "夹具必须有音符（实际 {before}）");

        let saved = dir.join("imported.yeban");
        let lines = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--import-midi".to_owned(),
                mid.display().to_string(),
                "--save-as".to_owned(),
                saved.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("导入必须成功");

        let line = lines
            .iter()
            .find(|line| line.starts_with("imported-midi:"))
            .expect("必须有 imported-midi 报告行");
        assert!(
            line.contains(&format!("notes={before}")),
            "导入条数必须等于文件里的音符数（期望 {before}）: {line}"
        );
        assert!(
            line.contains("origin=midi-input"),
            "来源标记必须是 `MidiInput`（⛔ 不是 `UserUi`）: {line}"
        );
        assert!(
            line.contains(&format!("path={}", mid.display())),
            "报告行必须写出来源文件: {line}"
        );

        // `--save-as` 保存的必须是**导入后**的工程：把保存的文件重新导出成 SMF，
        // 数它里面的音符 —— 期望 = 原来的 ＋ 导入的（走**公开入口**回读，⛔ 不碰内部字段）。
        let probe = dir.join("probe.mid");
        run_batch(
            &parse(&[
                "--open".to_owned(),
                saved.display().to_string(),
                "--export-midi".to_owned(),
                probe.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("保存的工程必须能重新打开并导出");
        let after = yeban_render::midi::parse_smf(&std::fs::read(&probe).expect("读"))
            .expect("回读")
            .notes
            .len();
        assert_eq!(
            after,
            before * 2,
            "保存的工程里音符数必须是原来的两倍（原 {before}）"
        );
    }

    /// `--import-midi` 的**拒绝不留半成品**：非 SMF 字节 ⇒ 与 `--open` 同族的退出码，
    /// 且 `--save-as` 的目标**一个字节都不写**。
    #[test]
    fn import_midi_refuses_a_bad_file_without_writing_or_editing_anything() {
        let dir = scratch_dir("import-midi-bad");
        let (source, _project) = write_container_file(&dir, "song.yeban");
        let junk = dir.join("junk.mid");
        std::fs::write(&junk, b"not a midi file at all").expect("写垃圾");
        let target = dir.join("never.yeban");

        let error = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--import-midi".to_owned(),
                junk.display().to_string(),
                "--save-as".to_owned(),
                target.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect_err("非 SMF 必须失败");
        assert_eq!(
            error.exit_code(),
            EXIT_OPEN,
            "与 `--open` 同族：输入文件被拒"
        );
        assert!(
            error.to_string().contains("解析 MIDI 失败"),
            "必须转达解析器的精确裁决: {error}"
        );
        assert!(!target.exists(), "失败 ⇒ 一个字节都不许写出去");
    }

    /// `--import-audio`：把一份**真 WAV** 导入工程 ⇒ 工程真的拿到**解码后的音频**，
    /// ⛔ 而不是只证明"开关能解析"。
    ///
    /// ## 证明什么 / 不证明什么
    ///
    /// 证明：`--import-audio <wav> --save-as <容器>` 这一条路真的①读了文件、②过了字节
    /// 闸门、③用本仓**唯一**的 WAV 读取器解码出**事实**（帧数／采样率／声道）、
    /// ④按**内容寻址**（源字节的真 SHA-256）在 `clip_pool` 登记一条音频片段、
    /// ⑤把**逐字节相同**的字节落进容器的 `assets/{sha256}`、⑥让 `--save-as` 把这条片段
    /// 带进工程文档 —— 回读走公开入口 `open_project_archive_file`，且从容器里取回的字节
    /// **再解码一次**得到同样的帧数／采样率／声道。
    ///
    /// 不证明：①**格式面** —— 本路径只吃 `read_plain_wav` 能读的 PCM／float WAV
    /// （16/24-bit 整数、32-bit 浮点）；MCP 那份走 `yeban-decode`(symphonia) 的
    /// FLAC／OGG／Vorbis **不在**覆盖内（见 `crate::import_audio` 的模块文档）。
    /// ②**摆放／可听** —— 本片只登记 `clip_pool`、⛔ 不摆放（`frames_to_ticks_ceil` 的
    /// 唯一实现在 `yeban-mcp/src/domain/render_math.rs`，app 够不着）。
    /// ③ **重复导入同一份字节的幂等**（由 `crate::import_audio` 的模块内判据覆盖）。
    #[test]
    fn import_audio_puts_the_decoded_wav_bytes_into_the_project_and_the_saved_container() {
        use yeban_render::dither::PcmBuffer;
        use yeban_render::rf64::PcmFormat;

        let dir = scratch_dir("import-audio");
        let (source, project) = write_container_file(&dir, "song.yeban");

        // 夹具：**真 WAV**，由本仓唯一写出器现造；100 帧单声道 16-bit，样本非全零。
        let wav = dir.join("take.wav");
        let samples: Vec<i16> = (0..100_i16).map(|index| index * 100 - 5_000).collect();
        yeban_render::wav::write_plain_wav(
            &wav,
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        let wav_bytes = std::fs::read(&wav).expect("读夹具 WAV");
        let expected = AssetHash::of_bytes(&wav_bytes);

        // ⚠ 夹具工程**自己**带一条音频片段（`demo_project` 的假哈希，见
        // `yeban-model/src/samples.rs:876`）⇒ 判据必须按**这份字节的哈希**找，
        // ⛔ 不能按"工程里有没有音频片段"找（那会与夹具的假片段混淆）。
        assert!(
            !project
                .clip_pool
                .values()
                .any(|entry| match &entry.content {
                    yeban_model::ClipContent::Audio { asset, .. } =>
                        asset.as_str() == expected.as_str(),
                    _ => false,
                }),
            "夹具工程初始不该已有**这份字节**的片段（否则'新增了一条'无从判定）"
        );

        let saved = dir.join("imported.yeban");
        let lines = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--import-audio".to_owned(),
                wav.display().to_string(),
                "--save-as".to_owned(),
                saved.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("音频导入必须成功（未修码上这里就是红的第一现场）");

        let line = lines
            .iter()
            .find(|line| line.starts_with("imported-audio:"))
            .unwrap_or_else(|| panic!("必须有 imported-audio 报告行: {lines:?}"));
        assert_eq!(
            field(line, "asset").as_deref(),
            Some(expected.as_str()),
            "报告行的资产哈希必须是**源字节**的 SHA-256: {line}"
        );
        assert_eq!(
            field(line, "frames").as_deref(),
            Some("100"),
            "解码事实：帧数: {line}"
        );
        assert_eq!(
            field(line, "rate").as_deref(),
            Some("48000"),
            "解码事实：采样率: {line}"
        );
        assert_eq!(
            field(line, "channels").as_deref(),
            Some("1"),
            "解码事实：声道数: {line}"
        );
        assert_eq!(
            field(line, "origin").as_deref(),
            Some("import"),
            "来源标记必须是 `OpOrigin::Import`: {line}"
        );

        // 公开入口回读 `--save-as` 写出的容器：工程拿到音频条目 ＋ 资产池拿到同一份字节。
        let reopened = crate::open::open_project_archive_file(
            &saved,
            &crate::open::ProjectOpenOptions::default(),
        )
        .expect("回读保存的容器");
        let entry = reopened
            .project
            .clip_pool
            .values()
            .find(|entry| match &entry.content {
                yeban_model::ClipContent::Audio { asset, .. } => {
                    asset.as_str() == expected.as_str()
                }
                _ => false,
            })
            .expect("保存的工程里必须多一条**这份字节**的音频片段");
        match entry.content.clone() {
            yeban_model::ClipContent::Audio { asset, gain_db } => {
                assert_eq!(asset, expected, "片段必须指向源字节的 SHA-256");
                assert_eq!(gain_db, 0.0, "本路径的片段增益是单位增益 0.0 dB");
            }
            other => panic!("必须是音频片段: {other:?}"),
        }
        let stored = reopened
            .assets
            .iter()
            .find(|(hash, _)| *hash == expected)
            .map(|(_, bytes)| bytes.clone())
            .expect("容器的资产池必须有该 sha256 的字节");
        assert_eq!(stored, wav_bytes, "容器里的字节必须与源文件**逐字节**相同");

        // **解码**证据：从容器取回的字节再走同一个唯一读取器 ⇒ 帧数／采样率／声道不变。
        let extracted = dir.join("extracted.wav");
        std::fs::write(&extracted, &stored).expect("写出取回的资产");
        let (back, buffer) =
            yeban_render::wav::read_plain_wav(&extracted).expect("取回的资产必须可解码");
        assert_eq!(back.sample_rate, 48_000);
        assert_eq!(back.channels, 1);
        assert_eq!(buffer.len(), 100, "帧数 = 100（单声道）");
    }

    /// **边界钉**（⛔ 不是能力判据、⛔ 不是"静默忽略是好事"）：`--import-audio` **单独**给
    /// （一个批处理开关都不给）⇒ `Options::batch()` 为假 ⇒ 进程走 GUI 路径，而 `run_gui`
    /// **不消费**本字段 ⇒ **导入不会发生**。
    ///
    /// 这是 `--import-midi` 早就存在的**同一条口径**（两条导入都没有"静默忽略"守卫），
    /// 本切片把它**如实登记**在 `Options::import_audio` 的文档与 `--help` 里。本条判据钉住
    /// "文档说的"与"代码做的"不许漂移：将来若为两条导入加了"必须与批处理开关同给"的守卫，
    /// 本条会红，届时应按**新语义**更新它（⛔ 不是删掉它）。
    ///
    /// ⚠ 本条**从未在未修码上跑红过**（它今天描述的就是代码本来的行为）⇒ 它是**钉子**，
    /// ⛔ 不是"这段能力曾经缺失"的证据。能力判据是上面那条
    /// `import_audio_puts_the_decoded_wav_bytes_into_the_project_and_the_saved_container`
    /// （它在未修码上的红字是 `UnknownArgument("--import-audio")`）。
    #[test]
    fn a_lone_import_audio_flag_selects_the_gui_path_so_the_import_never_runs() {
        let options = parse(&["--import-audio".to_owned(), "kick.wav".to_owned()]).expect("解析");
        assert!(options.import_audio.is_some(), "字段必须被解析器填上");
        assert!(
            !options.batch(),
            "批处理路径没被选中（与 `--import-midi` 同款：导入开关自己不选路径）"
        );
        assert!(
            options.wants_gui(),
            "⇒ 进程走 GUI 路径 ⇒ `run_batch` 根本不跑 ⇒ 导入不会发生"
        );
    }

    /// `--export-wav`：写出的 WAV 可被 `read_plain_wav` **读回**，且帧数/采样率/声道与请求一致，
    /// 样本非全零（真的有声音），报告行的数字与回读一致，两次导出**逐字节相同**（`ARCH-DET-*`）。
    #[test]
    fn export_wav_writes_a_readable_wav_whose_shape_matches_the_request() {
        use yeban_render::wav::read_plain_wav;

        let dir = scratch_dir("export-wav");
        let (source, project) = write_container_file(&dir, "song.yeban");
        let target = dir.join("song.wav");

        let lines = run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--export-wav".to_owned(),
                target.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("导出必须成功");

        // ① 报告行存在且带 `from=`（不给 --open 时那条"演示工程"的诚实口径同一处）。
        let line = lines
            .iter()
            .find(|line| line.starts_with("exported-wav:"))
            .expect("必须有 exported-wav 报告行");
        assert!(line.contains("from="), "报告行必须明说来源: {line}");
        assert!(line.contains("source="), "报告行必须登记音源: {line}");

        // ② 回读：帧数 / 采样率 / 声道 与请求一致（24-bit 整数 PCM）。
        let (format, buffer) = read_plain_wav(&target).expect("写出的 WAV 必须可回读");
        assert_eq!(format.sample_rate, 48_000, "采样率");
        assert_eq!(format.channels, 2, "声道");
        assert_eq!(format.bits_per_sample, 24, "位深");
        assert!(!format.is_float, "整数 PCM");
        let frames = u64::try_from(buffer.len() / usize::from(format.channels)).expect("帧数");
        assert_eq!(
            frames,
            48_000 * 8,
            "帧数与 `WavExportOptions::default()` 一致"
        );

        // ③ 非全零：导出的是**有内容**的音频（这条防的是"静默 WAV"）。
        let peak = match &buffer {
            yeban_render::dither::PcmBuffer::Int24(samples) => samples
                .iter()
                .map(|sample| sample.unsigned_abs())
                .max()
                .unwrap_or(0),
            other => panic!("默认导出必须是 24-bit 整数: {other:?}"),
        };
        assert!(peak > 0, "导出的 WAV 不能是静音（peak = {peak}）");

        // ④ 同一工程两次导出 ⇒ 逐字节相同（确定性）。
        let again = dir.join("song-again.wav");
        run_batch(
            &parse(&[
                "--open".to_owned(),
                source.display().to_string(),
                "--export-wav".to_owned(),
                again.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect("第二次导出必须成功");
        assert_eq!(
            std::fs::read(&target).expect("读第一次"),
            std::fs::read(&again).expect("读第二次"),
            "同一工程的两次导出必须逐字节相同"
        );
        // 演示工程的音符数（独立读一遍）必须非零，否则上面的"非静音"是运气。
        let notes: usize = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .map(std::collections::BTreeMap::len)
            .sum();
        assert!(notes > 0, "夹具必须有音符（实际 {notes}）");
    }

    /// ⭐ 任务单 `#302`：`--limiter` 是**每次调用**的导出修饰开关，默认关，
    /// 重复给明确报错（⛔ 不静默）。
    #[test]
    fn limiter_is_a_per_call_export_modifier_and_defaults_off() {
        let off = parse(&["--export-wav".to_owned(), "out.wav".to_owned()]).expect("默认解析");
        assert!(!off.wav_limiter, "缺省必须是关");

        let on = parse(&[
            "--export-wav".to_owned(),
            "out.wav".to_owned(),
            "--limiter".to_owned(),
        ])
        .expect("开启解析");
        assert!(on.wav_limiter, "--limiter 必须打开开关");

        let duplicate = parse(&[
            "--export-wav".to_owned(),
            "out.wav".to_owned(),
            "--limiter".to_owned(),
            "--limiter".to_owned(),
        ])
        .expect_err("重复 --limiter 必须拒绝");
        assert_eq!(
            duplicate,
            ParseError::DuplicateOption("--limiter"),
            "重复开关必须逐字报同一变体"
        );
    }

    /// `--export-wav` 的**拒绝不留半成品**：目标目录不可写 / 目标不是容器 ⇒ 一个字节都不写。
    #[test]
    fn export_wav_refuses_without_leaving_a_half_file() {
        let dir = scratch_dir("export-wav-bad");
        let junk = dir.join("junk.bin");
        std::fs::write(&junk, b"not a zip at all").expect("写垃圾");
        let target = dir.join("never.wav");

        let error = run_batch(
            &parse(&[
                "--open".to_owned(),
                junk.display().to_string(),
                "--export-wav".to_owned(),
                target.display().to_string(),
            ])
            .expect("解析"),
        )
        .expect_err("非容器输入必须失败");
        assert_eq!(
            error.exit_code(),
            EXIT_OPEN,
            "与 `--export-midi` 同族：打开失败复用 --open 的退出码"
        );
        assert!(!target.exists(), "打开失败 ⇒ 一个字节都不许写出去");
    }

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

        // ⭐ 那一行**整行**由下面两条判据钉住（⛔ 不只断言"含 author="）：
        // `report_author_column_is_pinned_field_by_field`（改 ⇒ 变）
        // ＋ `report_author_column_is_stable_without_a_change`（⛔ 不改 ⇒ 不变）。

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

#[cfg(test)]
mod author_report_tests {
    use super::*;

    /// ⭐ 造一个**产品路径**的报告（`project_report` 是界面路径与批处理路径**共用**的那一份）。
    fn loaded_with_author(author: &str) -> Loaded {
        let mut project = crate::bridge::demo_project();
        project.author = author.to_owned();
        Loaded {
            archive: ProjectArchive {
                project,
                history_dag: Vec::new(),
                assets: Vec::new(),
            },
            source: ProjectSource::Sample(Sample::Default),
        }
    }

    /// ⭐ **独立**拼出那一行的**期望**（⛔ 不调 `project_report`）⇒ 字段名、顺序、引号口径
    /// 全部被钉住 ⇒ ⭐ **该行任何字段漂移都会红**（这正是"该行此前没有判据"的补课）。
    fn expected_project_line(project: &YebanProjectV1) -> String {
        format!(
            "project: id={} bpm={:.2} ts={}/{} title={} author={}",
            project.id.to_canonical_string(),
            project.bpm,
            project.time_signature.numerator,
            project.time_signature.denominator,
            quoted(&project.title),
            quoted(&project.author),
        )
    }

    fn line_for(loaded: &Loaded) -> String {
        project_report(loaded)
            .into_iter()
            .find(|line| line.starts_with("project: id="))
            .expect("报告里必须有那一行")
    }

    /// ⭐ **判据 ①**：改 `author` ⇒ 报告行**逐字**变，且与**独立拼出**的期望**逐字相等**。
    #[test]
    fn report_author_column_is_pinned_field_by_field() {
        let loaded = loaded_with_author("夜半作者");
        let line = line_for(&loaded);
        assert_eq!(
            line,
            expected_project_line(&loaded.archive.project),
            "⭐ 报告行必须与**独立拼出**的期望逐字相等（字段名/顺序/引号口径全钉住）"
        );
        assert!(
            line.ends_with("author=\"夜半作者\""),
            "⭐ 改 author ⇒ 行尾必须出现它（实测：{line}）"
        );

        // ⭐ 再改一次 ⇒ **整行**随之变（⛔ 不只是"含 author="）。
        let other = loaded_with_author("another");
        let other_line = line_for(&other);
        assert_ne!(line, other_line, "⭐ 改 author ⇒ 整行必须变");
        assert_eq!(other_line, expected_project_line(&other.archive.project));

        // ⭐ 空串是"未署名"的确定编码 ⇒ 也必须**如实出现**在那一行里（⛔ 不省略字段）。
        // ⚠ 这条断言**归 ①**（它依赖产品行为）⇒ ⛔ 不放进控制臂，否则控制臂会跟着 ① 一起红、
        // 就**无法演示**"① 红而控制臂仍绿"（第 46/47 条；实测踩过）。
        let empty = line_for(&loaded_with_author(""));
        assert!(
            empty.ends_with("author=\"\""),
            "⭐ 未署名必须如实写成 author=\"\"（⛔ 不是省略字段）：{empty}"
        );
        assert_eq!(
            empty,
            expected_project_line(&loaded_with_author("").archive.project)
        );
    }

    /// ⭐ **判据 ②（控制臂，独立判据、走产品路径）**：⛔ **不改** `author` ⇒ 报告行**逐字不变**。
    ///
    /// ⚠ **单独一条**（第 46/47 条）：若与 ① 同条，① 一红就**无法演示**"控制臂仍绿"。
    /// ⚠ 它走的是**产品路径**（`project_report`，界面与批处理共用那一份），⛔ 不是另写旁路。
    #[test]
    fn report_author_column_is_stable_without_a_change() {
        let first = line_for(&loaded_with_author("same"));
        let second = line_for(&loaded_with_author("same"));
        assert_eq!(
            first, second,
            "⭐ 控制臂：author 不变 ⇒ 报告行必须**逐字**不变"
        );
    }
}
