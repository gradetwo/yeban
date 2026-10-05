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
use crate::export_midi::{MidiExportError, MidiExportReport, export_project_to_file};
use crate::input::{InputContext, Modifiers, PhysicalKey};
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
/// `--export-elements` / `--export-midi` 失败（I/O、编码被拒、工程无 MIDI 内容、PPQ 漂移）。
///
/// **不发明新码**（ADR-0001 D25 的口径）：MIDI 导出复用既有的"导出失败"这一档。
pub const EXIT_EXPORT: u8 = 5;

/// CI 的握手行：它出现 = 进程真的没构造窗口、没进阻塞事件循环。
///
/// 它必须**恰好**是这个字面值（`docs/ledger/ui-shell-notes.md` 与既有判据脚本只认它）。
pub const HEADLESS_HANDSHAKE: &str = "headless ok";

/// 无头路径的边界声明行（**不许**让 `headless ok` 看着像"UI 已验证"）。
const HEADLESS_BOUNDARY: &str = "headless: 未构造 MainWindow, 未初始化 Slint 后端, 未渲染任何像素 —— 控件树断言需要 crates/yeban-ui-test-port 的 testing backend";

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
  --print-shortcuts        打印快捷键策略表在本版本的判定结果 [UI-A11Y-001/002]
  --project-sample <default|filled>
                           选择\"没有 --open 时\"用哪个工程 (默认 default);
                           重复给以最后一个为准

运行形态:
  yeban-app                启动 GUI (需要显示器; 进入阻塞事件循环)
  yeban-app --open a.yeban 用打开的那个工程启动 GUI
  任一\"无窗口开关\"(--headless / --dump-elements / --export-elements /
  --export-midi / --print-shortcuts / --save-as) 都不构造窗口、不进事件循环,
  并打印握手行 `{handshake}`。
  `--help` / `--version` 是短路命令, 不打印握手行。

组合语义 (都是有意的, 不是碰巧):
  --headless 与 --open 同时给  不构造窗口, 但**真的**打开文件并打印它的读数
                               ⇒ 无显示器环境下的\"打开这个工程\"自检
  --headless 单独给            不打开任何文件, 用演示工程自检
                               (输出里 `project-source:` 会说明)
  --save-as 不给 --open        保存的是演示工程, 输出 `saved: ... from=sample=default` 明说
  --save-as 与 --headless      两者都是无窗口路径, 可以一起给 (保存不需要窗口)
  --export-elements 与 --export-midi 与 --save-as 任意组合
                               顺序固定: **先**导出元素, **再**导出 MIDI,
                               **最后**保存工程; 任一导出失败 ⇒ 不写工程 (退出码 {export})
  --export-midi 不给 --open    导出的是演示工程, 输出 `exported-midi: ... from=sample=...` 明说
  --help / -h, --version / -V  短路: 出现即打印并退出 {ok}, 其余参数(含未知参数)不再检查
  --open / --save-as / --export-elements / --export-midi
                               各只能给一次; 重复给 = 用法错误 (退出码 {usage})

环境变量:
  SLINT_BACKEND=headless   与 --headless 等价 (yeban 自研哨兵值; Slint 1.18.1 无此后端)

退出码:
  {ok} 成功 (含 --help / --version / 无头自检完成)
  {ui} 界面路径失败 (无法创建窗口 / 事件循环异常 / 工程无法投影成界面)
  {usage} 命令行用法错误 (未知开关 / 缺取值 / 重复给只能给一次的开关 / 未知工程样本)
  {open} --open 失败 (读文件失败 / 超过 4 GiB 上限 / 不是 `.yeban` 容器 /
     容器拒绝: 压缩法 / Zip-Slip / 解压炸弹 / 截断 / CRC 不匹配 / 缺件 / 非法 project.json …)
  {save} --save-as 失败 (临时文件 / 刷盘 / 原子重命名任一步失败, 或容器写出被拒)
  {export} --export-elements 失败 / --export-midi 失败 (I/O;
      或工程里没有可导出的 MIDI 音符 / 拍号分母不是 2 的幂 /
      工程 PPQ 与编码器默认 PPQ 不一致 / 编码器拒绝越界的音高或力度)

示例 (全部已在真二进制上跑过):
  yeban-app --headless
  yeban-app --open song.yeban --headless
  yeban-app --open song.yeban --save-as copy.yeban
  yeban-app --open song.yeban --dump-elements
  yeban-app --open song.yeban --export-elements elements.txt
  yeban-app --open song.yeban --export-midi song.mid
  yeban-app --version
",
        handshake = HEADLESS_HANDSHAKE,
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
/// 两个样本走的是**同一条**投影 + 注入路径（`bridge::from_project` → `host::apply_view`），
/// 区别只在"哪个 `YebanProjectV1`"。这就是本工作线的验收形态：换工程 ⇒ 换像素，
/// 中间没有任何"演示数据分支"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sample {
    /// 演示夹具（`bridge::demo_project()`）。
    #[default]
    Default,
    /// `yeban-model` 的规范级丰富样本（`samples::filled_project()`）。
    Filled,
}

impl Sample {
    /// 报告用的稳定短名（`--project-sample` 的取值）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Filled => "filled",
        }
    }

    /// 构造样本工程。
    #[must_use]
    pub fn project(self) -> YebanProjectV1 {
        match self {
            Self::Default => crate::bridge::demo_project(),
            Self::Filled => yeban_model::samples::filled_project(),
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
    /// `--open <path>`：当前工程来自这个文件（否则来自 [`Self::sample`]）。
    pub open: Option<PathBuf>,
    /// `--save-as <path>`：把当前工程原子落盘到这里。
    pub save_as: Option<PathBuf>,
    /// `--project-sample <default|filled>`。
    pub sample: Sample,
}

impl Options {
    /// 是否是**无窗口**路径（不构造窗口、不进事件循环）。
    ///
    /// 语义表（`--help` 的"组合语义"一节与判据都按这张表）：
    /// `--help` / `--version` / `--headless` / `--dump-elements` / `--print-shortcuts` /
    /// `--export-elements` / `--export-midi` / `--save-as` 各自都能单独把进程推离 GUI 路径。
    #[must_use]
    pub fn batch(&self) -> bool {
        self.help
            || self.version
            || self.headless
            || self.dump_elements
            || self.print_shortcuts
            || self.export_elements.is_some()
            || self.export_midi.is_some()
            || self.save_as.is_some()
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
    #[must_use]
    pub fn needs_projection(&self) -> bool {
        self.headless
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
                "未知的工程样本 `{sample}` (可用: default|demo|filled)"
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
/// - `--project-sample` 重复给以最后一个为准（沿用旧行为）。
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
            "--project-sample" => {
                let value = take_value("--project-sample", inline, args, &mut cursor)?;
                options.sample = match value.as_str() {
                    "default" | "demo" => Sample::Default,
                    "filled" => Sample::Filled,
                    other => return Err(ParseError::UnknownSample(other.to_owned())),
                };
            }
            other => return Err(ParseError::UnknownArgument(other.to_owned())),
        }
    }
    Ok(options)
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
    #[must_use]
    pub fn report_line(&self) -> String {
        match self {
            Self::File { path, bytes } => format!(
                "opened: path={} bytes={bytes} format={DOCUMENT_FORMAT}",
                path.display()
            ),
            Self::Sample(sample) => format!(
                "project-source: sample={} (内置演示工程; 未给 --open ⇒ 未读任何文件)",
                sample.name()
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
                format!("sample={} (内置演示工程, 不是从文件打开的)", sample.name())
            }
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
}

impl CliError {
    /// 这一档失败的退出码（`--help` 里对用户逐条写明）。
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Projection { .. } | Self::Ui { .. } => EXIT_UI,
            Self::Open { .. } => EXIT_OPEN,
            Self::Save { .. } => EXIT_SAVE,
            Self::Export { .. } | Self::ExportMidi { .. } => EXIT_EXPORT,
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
            Self::Ui { .. } => None,
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

/// 打印 `[UI-A11Y-001]` / `[UI-A11Y-002]` 策略表在本版本下的判定结果。
///
/// 同时给出**画布聚焦**与**IME 合成态**两列 —— 这两列必须不同, 而且差异必须是
/// "合成态什么都收不到"。这个输出本身就是给 CI 与人看的一份活文档。
#[must_use]
pub fn shortcut_lines() -> Vec<String> {
    let mut lines = vec![
        "# 快捷键策略表 (yeban-app scaffold)".to_owned(),
        "# 第三列是 [UI-A11Y-002] 的核心: 合成态下所有非 F5/F6 的键都必须被输入法吞掉".to_owned(),
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

    for (label, key, modifiers) in KEYMAP {
        lines.push(format!(
            "  {:<34} {:<28} {:<28}",
            label,
            format!("{:?}", canvas.resolve(key, modifiers)),
            format!("{:?}", composing.resolve(key, modifiers)),
        ));
    }
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
/// 8. `--save-as` 的 `saved:`。
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
            "--print-shortcuts",
            "--project-sample",
            "--headless",
            "--help",
            "--version",
        ] {
            assert!(usage.contains(switch), "用法文本必须列出 `{switch}`");
        }
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

        let cases: [(Options, bool, bool); 9] = [
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
        ];
        for (options, gui, projection) in cases {
            assert_eq!(options.wants_gui(), gui, "{options:?}");
            assert_eq!(options.needs_projection(), projection, "{options:?}");
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
}
