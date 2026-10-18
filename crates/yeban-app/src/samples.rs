//! ⭐ 任务单 `#196`：采样库的**预设路径**、**按需下载**、**完整性自证**。
//!
//! ## 这条线要回答的三件事
//!
//! 1. **库根是预设路径**（⛔ 不是仓库内目录）：`~/Library/Application Support/dev.yeban.app/Samples/`
//!    （macOS）／`$XDG_DATA_HOME/yeban/samples/`（Linux）／`%APPDATA%\yeban\samples\`（Windows），
//!    且**可被 [`SAMPLE_LIBRARY_ENV`] 覆盖**（判据因此指向临时目录，⛔ 不碰用户真实目录）。
//! 2. **下载只在用户显式触发时发生**：GUI 那个按钮 → [`DownloadOptions::for_app`] →
//!    [`run_download`] → `scripts/dev/fetch-samples.py --all --dest <库根>`。
//!    ⛔ 启动时不联网、⛔ 判据不联网（判据用本地假清单 ＋ `file://` 假源，见 `tests/sample_library.rs`）。
//! 3. **"装好了"是字节级的**：取回脚本逐项比对清单的 `sha256` ＋ `size_bytes`；
//!    全部通过（退出码 `0`）才写**安装戳** [`SAMPLE_LIBRARY_STAMP`]，并把"下载 N／校验通过 N／失败 0"
//!    如实写回界面。退出码口径与脚本一致：`0` 全通过／`1` 有字节不符／`2` 无法判定。
//!
//! ## 为什么**调脚本**而不是在 Rust 里重写取回（⛔ 零新依赖）
//!
//! `fetch-samples.py` 已经有：清单解析、`repo`＋`pin` → 上游 URL、上游剥离表、
//! `sha256`／字节数逐项校验、`0/1/2` 退出口径。在 Rust 里重写要引入 `ureq`／`reqwest`
//! （新依赖 ＋ 第二份校验逻辑）—— 本单因此只给脚本加 `--dest` / `--manifest` /
//! `--source-root` 三个**落点**参数，校验逻辑仍然只有一份。
//!
//! ⭐ 任务单 `#201`：脚本的取回方式多了一个**归档模式**（每个 instrument 一次
//! `…/archive/<pin>.tar.gz` ⇒ **30** 次请求，而不是逐文件的 **20 594** 次），且它是**缺省**。
//! 本模块因此由 [`DownloadOptions::retrieval_mode_flag`] **显式**传模式：真下载带 `--archives`，
//! 假源（`#196` 的扁平文件树）带 `--per-file` —— ⛔ 不靠缺省值，GUI 侧零行为变化。
//!
//! ## ⭐ 任务单 `#202`：⛔ 不许假定每个平台都有 `python3`（也不许假定它的 stdio 是 UTF-8）
//!
//! v1 把解释器写死成 `Command::new("python3")`。`#196` 自己登记过这条风险（逐字：
//! "运行 GUI 下载要求 `python3` 在 PATH（macOS 自带 `/usr/bin/python3`，Windows 需另装；未做回退）"），
//! `#202` 的 Windows 腿红把它变成了实测。现在有三层：
//!
//! 1. **候选表**：[`interpreter_candidates`] —— 先 `python3`，再 `py -3`（Windows 的官方启动器），
//!    最后 `python`；**逐个探测**（[`Interpreter`] ＋ `-c` 打印 [`INTERPRETER_PROBE_MARKER`]），
//!    只认"真的能跑 Python 3"的那一个。⛔ 不看文件存不存在，而是**跑一次**才算数
//!    （`python3` 这个名字可能被商店别名 / 坏 shim 占住）。
//! 2. **显式失败**：一个都探测不到 ⇒ [`DownloadError::NoInterpreter`]（逐字列出试过的名字），
//!    ⛔ 不静默跳过、⛔ 不算通过、⛔ 不退化成"字节不符"。
//! 3. **强制子进程 stdio 走 UTF-8**（`PYTHONUTF8=1` ＋ `PYTHONIOENCODING=utf-8`，见
//!    [`DownloadOptions::command`]）：Windows 的管道 stdout 用 ANSI 代码页，脚本的**第一句中文
//!    `print`** 就会抛 `UnicodeEncodeError` ⇒ 退出码 `1` ＋ stdout **空**（`#202` 的实测读数，
//!    逐字见 `tests/sample_library.rs` 的判据 Ⓐ）。那种 `1` 不是"有字节不符"，由
//!    [`DownloadError::NoReceipt`] 分开（脚本口径的 `1` 一定先打印汇总行）。
//!
//! ## ⚠ 本模块**不**做的事
//!
//! - ⛔ 不解析 SFZ、⛔ 不决定"哪条路径在哪找"——那是 [`crate::sfz_load::resolve_sampler_path`]
//!   的唯一职责（库根是它的一个候选根，见 [`crate::sfz_load::SfzRoots`]）；
//! - ⛔ 不在启动 / 导入 / 测试里自动下载；
//! - ⛔ 不把采样字节写进仓库：下载落点永远是 [`library_root`]（仓库外的用户数据目录）。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 覆盖库根的环境变量：设置成**非空**路径即以它为准（判据用它指向临时目录）。
pub const SAMPLE_LIBRARY_ENV: &str = "YEBAN_SAMPLE_LIBRARY";

/// ⭐ 未安装时**逐字**显示在设备架"装乐器"区的提示（任务单 `#196` 的原文）。
///
/// ⚠ 括号里的 `9.4 GiB` 是 `fetch-samples.py --all` 的既有口径（脚本帮助与清单
/// `policy` 都写 9.4 GiB）；清单 `counts.bytes_registered` 的精确值是
/// `9_844_170_377` 字节（= 9.168 GiB，20 594 项）—— 两个数不是同一个量，
/// 本常量按工单原文逐字使用，精确字节数见报告 §1。
pub const SAMPLE_LIBRARY_NOT_INSTALLED_HINT: &str = "采样库未安装，点此下载（9.4 GiB）";

/// 下载**全部成功**后写在库根下的安装戳文件名（JSON；内容见 [`run_download`]）。
pub const SAMPLE_LIBRARY_STAMP: &str = ".yeban-samples-installed.json";

/// 取回脚本相对**仓库根**的路径（[`crate::sfz_load::repo_root`] 是唯一仓库根取值）。
pub const DOWNLOAD_SCRIPT_RELATIVE: &str = "scripts/dev/fetch-samples.py";

/// 清单相对仓库根的路径（与 [`DOWNLOAD_SCRIPT_RELATIVE`] 同一个根）。
pub const MANIFEST_RELATIVE: &str = "assets/samples/manifest.json";

/// ⭐ `#202`：**解释器覆盖**。设置成非空即以它为唯一的解释器，⛔ 不回退、⛔ 不探测。
///
/// 为什么需要它（两件事都要它）：
/// - 判据要能在一台**没有真 Python**的机器上证明"找不到解释器 ⇒ 显式失败"（指向一个不存在的名字）；
/// - 判据要能用**假解释器**（只打印一行假回执）证明"我们真的执行了被选中的那个可执行文件"。
///
/// ⚠ 生产路径**不设置**它 ⇒ 走 [`interpreter_candidates`] 的探测。覆盖值不做探测，
/// 因为假解释器当然不会打印探测标记；这是"测试钩子"与"生产探测"的**唯一**分界。
pub const INTERPRETER_ENV: &str = "YEBAN_PYTHON";

/// 探测解释器时要求它打印的这一行（"真的能跑 Python 3"的机械证据）。
pub const INTERPRETER_PROBE_MARKER: &str = "yeban-python3-ok";

/// 探测用的 `-c` 源码：只有 **Python 3** 才打印 [`INTERPRETER_PROBE_MARKER`]。
///
/// ⚠ 刻意**不**用 `--version`：那会把版本号写进 stdout（还得解析），而且 Python 2 也会成功。
const INTERPRETER_PROBE: &str = "import sys;sys.stdout.write('yeban-python3-ok' if sys.version_info[0]==3 else 'yeban-python2')";

/// 一个**取回脚本解释器**的可执行形态：程序名 ＋ 前置参数。
///
/// 为什么需要前置参数：Windows 的官方启动器是 `py -3`（两个 argv 段），而 `python3` / `python`
/// 只有一段。用一个结构体承载两者，比在每个调用点写 `if cfg!(windows)` 干净。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interpreter {
    /// 传给 [`Command::new`] 的程序名（`python3` / `py` / `python`）。
    pub program: String,
    /// 程序之后、脚本之前的参数（`py -3` 的 `-3`；其余候选为空）。
    pub pre_args: Vec<String>,
}

impl Interpreter {
    /// 只有程序名、没有前置参数（`python3` / `python`）。
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            pre_args: Vec::new(),
        }
    }

    /// 带前置参数（Windows 的 `py -3`）。
    #[must_use]
    pub fn with_pre_args(program: impl Into<String>, pre_args: &[&str]) -> Self {
        Self {
            program: program.into(),
            pre_args: pre_args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    /// 留证 / 报错用的**逐字**形态（`python3` ／ `py -3`）。
    #[must_use]
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.pre_args.iter().cloned());
        parts.join(" ")
    }

    /// 一条以它为程序的 [`Command`]（前置参数已带上，脚本/`-c` 由调用方追加）。
    #[must_use]
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.pre_args);
        command
    }
}

/// ⭐ `#202` 的**解释器候选表**（顺序即尝试顺序）。
///
/// | 平台 | 候选 | 为什么是这个顺序 |
/// | :--- | :--- | :--- |
/// | 全部 | `python3` | 既有口径（macOS/Linux 的常规名字）；先试它 ⇒ **行为对既有平台零变化** |
/// | Windows | `py -3` | Windows 官方启动器（`python.org` 安装器默认装它，且**带版本选择**） |
/// | 全部 | `python` | 最后兜底（Windows 上通常就是 Python 3；macOS 上可能是 Python 2 ⇒ 探测会拒掉） |
///
/// ⚠ **不看** `cfg!(windows)` 之外的任何环境：`PATH` 里有什么由探测回答，⛔ 不由本函数猜。
#[must_use]
pub fn interpreter_candidates() -> Vec<Interpreter> {
    interpreter_candidates_for(cfg!(windows))
}

/// 候选表的**纯函数形态**（⭐ 与 `preset_library_root(platform, …)` 同一条纪律：
/// 平台只是参数，于是判据能在**一台机器**上读出两档，⛔ 不靠"只有 Windows 才跑"）。
///
/// `windows = true` ⇒ `python3` ⇒ `py -3` ⇒ `python`；`false` ⇒ `python3` ⇒ `python`。
#[must_use]
pub fn interpreter_candidates_for(windows: bool) -> Vec<Interpreter> {
    if windows {
        vec![
            Interpreter::new("python3"),
            Interpreter::with_pre_args("py", &["-3"]),
            Interpreter::new("python"),
        ]
    } else {
        vec![Interpreter::new("python3"), Interpreter::new("python")]
    }
}

/// 按序探测，第一个通过 `works` 的候选胜出；一个都没有 ⇒ [`DownloadError::NoInterpreter`]。
///
/// ⚠ 探测函数是**参数**（⛔ 不写死）：判据因此能在任何平台上用**一个假的探测函数**
/// 读出"顺序 / 回退 / 一个都没有"三档，而不用真的装/卸 Python。
fn first_working<F>(candidates: &[Interpreter], mut works: F) -> Result<Interpreter, DownloadError>
where
    F: FnMut(&Interpreter) -> bool,
{
    for candidate in candidates {
        if works(candidate) {
            return Ok(candidate.clone());
        }
    }
    Err(DownloadError::NoInterpreter {
        tried: candidates.iter().map(Interpreter::display).collect(),
    })
}

/// 跑一次 `<程序> [前置参数] -c <探测源码>`：只认"退出码 `0` ＋ stdout 有标记"。
///
/// ⚠ 这是**唯一**能区分"名字存在"与"它真的能跑 Python 3"的办法：`python3` 这个名字
/// 可能被商店别名 / 坏 shim / Python 2 占住（`#202` 的 Windows 腿就是被"名字对、行为不对"
/// 的一类差异绊倒的）。代价是一次子进程（约几十毫秒），只在**没有覆盖值**时发生。
fn interpreter_runs(interpreter: &Interpreter) -> bool {
    let mut command = interpreter.command();
    command.arg("-c").arg(INTERPRETER_PROBE);
    match command.output() {
        Ok(output) => {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains(INTERPRETER_PROBE_MARKER)
        }
        Err(_) => false,
    }
}

/// ⭐ `#202`：解析出**本次要用的**解释器。
///
/// 优先级：
/// 1. [`INTERPRETER_ENV`] 非空 ⇒ **只用它**（⛔ 不回退、⛔ 不探测：判据要指向假解释器）；
/// 2. 否则按 [`interpreter_candidates`] 逐个探测。
///
/// # Errors
///
/// [`DownloadError::NoInterpreter`]：一个能跑 Python 3 的解释器都没有（`tried` 逐字列出试过的名字）。
pub fn resolve_interpreter() -> Result<Interpreter, DownloadError> {
    if let Some(raw) = std::env::var_os(INTERPRETER_ENV)
        && !raw.is_empty()
    {
        return Ok(Interpreter::new(raw.to_string_lossy().into_owned()));
    }
    first_working(&interpreter_candidates(), interpreter_runs)
}

/// 预设库根的**平台形状**（判据要在同一台机器上读三档读数）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryPlatform {
    /// macOS：`<home>/Library/Application Support/dev.yeban.app/Samples`。
    MacOs,
    /// Linux：`$XDG_DATA_HOME/yeban/samples`，缺省 `<home>/.local/share/yeban/samples`。
    Linux,
    /// Windows：`%APPDATA%\yeban\samples`。
    Windows,
}

impl LibraryPlatform {
    /// 当前进程所在平台的形状（`cfg!` 只在这里出现一次）。
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// ⭐ **预设库根的唯一公式**（纯函数：判据可在任意平台上读三档读数）。
///
/// 参数是三个平台各自的**环境事实**（`None` = 该变量缺失）：
///
/// | 平台 | 主变量 | 缺省 |
/// | :--- | :--- | :--- |
/// | macOS | `home` | ——（没有 home 时用相对路径 `<cwd>/Library/Application Support/dev.yeban.app/Samples`） |
/// | Linux | `xdg_data_home` | `<home>/.local/share` |
/// | Windows | `appdata` | ——（没有 `%APPDATA%` 时用相对路径 `yeban/samples`） |
///
/// ⚠ 返回值**不创建目录**、⛔ 不查存在性：它只是路径公式。
#[must_use]
pub fn preset_library_root(
    platform: LibraryPlatform,
    home: Option<&Path>,
    xdg_data_home: Option<&Path>,
    appdata: Option<&Path>,
) -> PathBuf {
    match platform {
        LibraryPlatform::MacOs => home
            .map_or_else(PathBuf::new, Path::to_path_buf)
            .join("Library/Application Support/dev.yeban.app/Samples"),
        LibraryPlatform::Linux => {
            if let Some(xdg) = xdg_data_home {
                xdg.join("yeban/samples")
            } else if let Some(home) = home {
                home.join(".local/share/yeban/samples")
            } else {
                PathBuf::from("yeban/samples")
            }
        }
        LibraryPlatform::Windows => appdata
            .map_or_else(PathBuf::new, Path::to_path_buf)
            .join("yeban/samples"),
    }
}

/// ⭐ **采样库根**（一处实现）：`YEBAN_SAMPLE_LIBRARY` 非空 ⇒ 用它；否则按平台惯例。
///
/// ⚠ **每次调用都重读环境变量**（⛔ 不 `OnceLock` 缓存）：判据在同一进程里换临时库根，
/// 且"用户在会话中改环境变量"不该被上一份缓存盖住。代价只是一次 `getenv`，可忽略。
///
/// ⚠ 本函数**不创建目录**：下载脚本在真正落盘时 `mkdir -p`；只读路径的探针不会产生副作用。
#[must_use]
pub fn library_root() -> PathBuf {
    if let Some(raw) = std::env::var_os(SAMPLE_LIBRARY_ENV)
        && !raw.is_empty()
    {
        return PathBuf::from(raw);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    preset_library_root(
        LibraryPlatform::current(),
        home.as_deref(),
        xdg.as_deref(),
        appdata.as_deref(),
    )
}

/// 库根下的安装戳路径（`<库根>/.yeban-samples-installed.json`）。
#[must_use]
pub fn stamp_path() -> PathBuf {
    library_root().join(SAMPLE_LIBRARY_STAMP)
}

/// 安装戳的**轻量指纹**（长度 ＋ 修改时间；`None` = 戳不存在）。
///
/// 用途：`bridge::installable_builtin_instruments` 的进程内缓存按它失效 ——
/// 下载完成后安装戳出现，下一次投影就会重新探测（⛔ 不要求重启进程）。
#[must_use]
pub fn stamp_signature() -> Option<(u64, u128)> {
    let metadata = std::fs::metadata(stamp_path()).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_nanos());
    Some((metadata.len(), modified))
}

/// 采样库**是否已按清单校验安装**（安装戳存在 = 至少有一次**全部通过**的下载）。
///
/// ⚠ 它只是 UI 的"装没装"读数；"这份 SFZ 的样本在不在"仍由
/// [`crate::bridge::sfz_samples_present`]（同一条解码路径）回答，⛔ 不用戳代替字节。
#[must_use]
pub fn is_library_installed() -> bool {
    stamp_path().is_file()
}

/// 下载**进行中**的逐字状态（GUI 在 spawn 工作线程**之前**同步写它）。
///
/// 抽在这里而不是写进 `host.rs`：`tests/mixer_toggle_labels.rs` 用"整行就是一个字符串
/// 字面量"的形状数 `wire_mixer_switch` 的提交消息，宿主里多一条这样的字面量会被它误计
/// （这条判据的代理指标边界见该文件 §"代理指标的局限"）。
#[must_use]
pub fn in_flight_status(dest: &Path) -> String {
    format!("采样库下载中⏳：{}（完成后自动逐字节校验）", dest.display())
}

/// 未安装时逐字显示 [`SAMPLE_LIBRARY_NOT_INSTALLED_HINT`]；已安装时是空串。
#[must_use]
pub fn library_notice() -> String {
    if is_library_installed() {
        String::new()
    } else {
        SAMPLE_LIBRARY_NOT_INSTALLED_HINT.to_owned()
    }
}

/// ⭐ 一次下载的**全部输入**（判据可把清单 / 落点 / 假源换成临时目录）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadOptions {
    /// 清单文件（默认 [`MANIFEST_RELATIVE`]）。
    pub manifest: PathBuf,
    /// 落点库根（默认 [`library_root`]；文件按"库布局"写：`<dest>/<乐器>/…`）。
    pub dest: PathBuf,
    /// 可选的**源根**（本地目录或 `file://` URL）：给了就不再走上游 `raw.githubusercontent.com`。
    pub source_root: Option<PathBuf>,
}

impl DownloadOptions {
    /// 生产默认值：仓库清单 → 预设库根；⛔ 不指定假源（真下载走上游）。
    #[must_use]
    pub fn for_app() -> Self {
        Self {
            manifest: crate::sfz_load::repo_root().join(MANIFEST_RELATIVE),
            dest: library_root(),
            source_root: None,
        }
    }

    /// 取回脚本的绝对路径（唯一仓库根取值 ⇒ 不写第二份 `../../`）。
    #[must_use]
    pub fn script_path() -> PathBuf {
        crate::sfz_load::repo_root().join(DOWNLOAD_SCRIPT_RELATIVE)
    }

    /// ⭐ `#201`：取回模式的**显式**选择（⛔ 不依赖脚本的缺省值）。
    ///
    /// `fetch-samples.py` 的缺省已是**归档模式**（每个 instrument 一个 tarball ⇒ **30** 次请求，
    /// 而不是逐文件的 **20 594** 次）。但 `#196` 的判据桩是**扁平的假上游文件树**（不是 tarball）⇒
    /// 有 [`Self::source_root`] 时必须显式要 `--per-file`；真下载（无假源）显式要 `--archives`。
    ///
    /// ⚠ 两条路径都**写明**模式：脚本日后改缺省，也不会悄悄换掉这条命令的语义
    /// （这正是任务单要求在"默认值变了"之后**显式传参**的那一条）。
    #[must_use]
    pub const fn retrieval_mode_flag(&self) -> &'static str {
        if self.source_root.is_some() {
            "--per-file"
        } else {
            "--archives"
        }
    }

    /// 组装**将要执行**的命令（判据用 [`Self::command_line`] 留证，⛔ 不靠猜）。
    ///
    /// ⚠ 固定带 `--all`：这是"下载整个采样库"（不是 `--limit` 的有界档）。
    ///
    /// ⭐ `#202` 的两处硬要求：
    /// - 解释器由 [`resolve_interpreter`] 给出（⛔ 不再写死 `python3`）；
    /// - 子进程 stdio **强制 UTF-8**：Windows 的**管道** stdout 用 ANSI 代码页，脚本在
    ///   **第一句中文 `print`** 上抛 `UnicodeEncodeError` ⇒ 退出码 `1` ＋ stdout 空。
    ///   实测读数（`#202` 的 Windows 腿）：`left: 1`／`right: 0`（退出码）与 `stdout=`（空）。
    ///   `PYTHONIOENCODING` 必须**显式**设：父进程若已导出它，`PYTHONUTF8=1` 会被它压过
    ///   （本地 `PYTHONIOENCODING=cp1252` 的复现就是这一条）。
    #[must_use]
    pub fn command(&self, interpreter: &Interpreter) -> Command {
        let mut command = interpreter.command();
        command
            .arg(Self::script_path())
            .arg("--all")
            .arg(self.retrieval_mode_flag())
            .arg("--manifest")
            .arg(&self.manifest)
            .arg("--dest")
            .arg(&self.dest)
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8");
        if let Some(source_root) = &self.source_root {
            command.arg("--source-root").arg(source_root);
        }
        command
    }

    /// 命令行的**可读留证**（路径含空格时加引号；与 [`Self::command`] 同序同参）。
    #[must_use]
    pub fn command_line(&self, interpreter: &Interpreter) -> String {
        let mut parts = vec![
            interpreter.display(),
            quote_path(&Self::script_path()),
            "--all".to_owned(),
            self.retrieval_mode_flag().to_owned(),
            "--manifest".to_owned(),
            quote_path(&self.manifest),
            "--dest".to_owned(),
            quote_path(&self.dest),
        ];
        if let Some(source_root) = &self.source_root {
            parts.push("--source-root".to_owned());
            parts.push(quote_path(source_root));
        }
        parts.join(" ")
    }
}

/// 路径的 shell 形态（只用于**留证文本**，⛔ 不参与执行）。
fn quote_path(path: &Path) -> String {
    let text = path.display().to_string();
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text
    }
}

/// ⭐ 一次下载的**读数**（单位逐项写在字段名里）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadOutcome {
    /// 子进程退出码（`0` 全通过／`1` 有字节不符／`2` 无法判定）。
    pub exit_code: i32,
    /// 本次**取回**的文件数（已存在而只校验的不计）。
    pub fetched: usize,
    /// **校验通过**的文件数。
    pub verified: usize,
    /// 清单总项数（`fetch-samples.py` 的"清单共 N 项"）。
    pub total: usize,
    /// **字节不符**的文件数（`sha256` 或 `size_bytes` 对不上）。
    pub mismatched: usize,
    /// 实际执行的命令行（留证；判据据此证明"跑的是假源"）。
    pub command: String,
    /// 子进程 stdout 原文。
    pub stdout: String,
    /// 子进程 stderr 原文。
    pub stderr: String,
}

impl DownloadOutcome {
    /// 全通过 ⇔ 退出码 `0`（与脚本的 `0/1/2` 口径**同一个**判据）。
    #[must_use]
    pub fn ok(&self) -> bool {
        self.exit_code == 0
    }

    /// 界面 / 报告里那一句**逐字**读数。
    #[must_use]
    pub fn summary(&self) -> String {
        if self.ok() {
            format!(
                "采样库下载完成：下载 {}｜校验通过 {}/{}｜失败 {}",
                self.fetched, self.verified, self.total, self.mismatched
            )
        } else if self.exit_code == 2 {
            format!(
                "采样库下载无法判定（退出码 2）：下载 {}｜校验通过 {}/{}｜失败 {}",
                self.fetched, self.verified, self.total, self.mismatched
            )
        } else {
            format!(
                "采样库下载失败（退出码 {}）：下载 {}｜校验通过 {}/{}｜失败 {}",
                self.exit_code, self.fetched, self.verified, self.total, self.mismatched
            )
        }
    }
}

/// 下载**没能启动 / 无法执行**（⛔ 与"下载了但有字节不符"分开：前者是环境问题）。
#[derive(Debug)]
pub enum DownloadError {
    /// 取回脚本不在仓库里（安装形态不对 / 工作树缺文件）。
    ScriptMissing(PathBuf),
    /// **一个能跑 Python 3 的解释器都没找到**（⭐ `#202`）。
    ///
    /// ⚠ 它与 [`Self::Spawn`] 是两件事：本变体是"**这台机器上没有可用的 Python**"，
    /// ⛔ 不是下载失败、⛔ 不是字节不符、⛔ 更不是通过。`tried` 是**逐字**试过的可执行文件
    /// （顺序即尝试顺序），报错文本直接把它列出来 ⇒ 用户/CI 一眼看出该装什么。
    NoInterpreter {
        /// 逐字试过的解释器（`python3` / `py -3` / `python`）。
        tried: Vec<String>,
    },
    /// 子进程跑完了，但 **stdout 里没有 `fetch-samples:` 汇总行** ⇒ 它没跑到判定那一步。
    ///
    /// ⭐ `#202` 的实测动因：Windows 腿的脚本在**第一句中文 `print`** 上抛
    /// `UnicodeEncodeError`（管道 stdout 是 ANSI 代码页）⇒ 退出码 `1` ＋ stdout **空**。
    /// 脚本口径的 `1` 是"**已证实**的字节不符"，而崩溃的 `1` 是"环境判不了" ——
    /// 把后者报成前者＝把环境问题说成数据损坏。所以这条**显式分开**，
    /// 且**不碰**安装戳（旧戳原样保留：字节一位没动）。
    NoReceipt {
        /// 子进程退出码（崩溃时常见 `1`，与"有字节不符"同码）。
        exit_code: i32,
        /// 实际执行的命令行（留证；含解释器的逐字形态）。
        command: String,
        /// 子进程 stderr 原文（Python 的 traceback 在这里）。
        stderr: String,
    },
    /// 子进程启动失败（权限 / 沙箱 / 非 `NotFound` 的其他 `io::Error`）。
    Spawn(std::io::Error),
    /// 子进程被信号终止（没有退出码）。
    NoExitCode,
}

impl core::fmt::Display for DownloadError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ScriptMissing(path) => {
                write!(formatter, "取回脚本不存在：{}", path.display())
            }
            Self::NoInterpreter { tried } => write!(
                formatter,
                "找不到能跑 Python 3 的解释器（试过：{}）—— 采样库下载需要 Python 3；\
                 ⛔ 不静默跳过，也⛔ 不算通过",
                tried.join(" / ")
            ),
            Self::NoReceipt {
                exit_code,
                command,
                stderr,
            } => {
                let last = stderr
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("<stderr 空>");
                write!(
                    formatter,
                    "取回脚本没有回执（退出码 {exit_code}；stdout 里没有 `fetch-samples:` 汇总行）\
                     ⇒ 它没跑到判定那一步，⛔ 不是\"字节不符\"；命令：{command}；\
                     stderr 最后一行：{last}"
                )
            }
            Self::Spawn(error) => write!(formatter, "取回脚本启动失败：{error}"),
            Self::NoExitCode => write!(formatter, "取回脚本没有退出码（被信号终止？）"),
        }
    }
}

impl std::error::Error for DownloadError {}

/// ⭐ **执行一次下载并逐项校验**（唯一实现；GUI 按钮与判据走**同一段**代码）。
///
/// ## 步骤
///
/// 1. 找 [`DownloadOptions::script_path`]（不在 ⇒ [`DownloadError::ScriptMissing`]）；
/// 2. 解析解释器 [`resolve_interpreter`]（一个都没有 ⇒ [`DownloadError::NoInterpreter`]）；
/// 3. 跑 [`DownloadOptions::command`]（`--all --manifest … --dest … [--source-root …]`，
///    子进程 stdio 强制 UTF-8）；
/// 4. 从 stdout 的 `fetch-samples:` 汇总行解析 `取回 / 校验通过 / 不符 / 清单共`
///    （**没有这一行** ⇒ [`DownloadError::NoReceipt`]：崩溃不是"字节不符"）；
/// 5. 退出码 `0` ⇒ 写安装戳（[`SAMPLE_LIBRARY_STAMP`]，含通过数 / 总项数）；
///    退出码 `1` ⇒ **删掉**旧戳（一次已证实的字节不符不许留下"已安装"的假读数）；
///    退出码 `2`（无法判定）⇒ 旧戳原样保留（离线不等于字节坏了）。
///
/// ⚠ 判据**不联网**：`source_root = Some(file:// 假源)` 时脚本读本地字节；
/// 命令行原文保存在 [`DownloadOutcome::command`]，可直接证明"没有走上游"。
///
/// # Errors
///
/// [`DownloadError`]：脚本缺失 / **找不到能跑 Python 3 的解释器**（`#202`）/
/// 子进程启动失败 / 没有退出码 / **stdout 里没有汇总行**（`#202`：崩溃不是"字节不符"）。
pub fn run_download(options: &DownloadOptions) -> Result<DownloadOutcome, DownloadError> {
    let script = DownloadOptions::script_path();
    if !script.is_file() {
        return Err(DownloadError::ScriptMissing(script));
    }
    let interpreter = resolve_interpreter()?;
    let output = options
        .command(&interpreter)
        .output()
        .map_err(|error| match error.kind() {
            // ⭐ `#202`：覆盖值（或探测之后仍失败的候选）找不到 ⇒ 仍是"没有解释器"，
            // ⛔ 不冒充"下载失败"。
            std::io::ErrorKind::NotFound => DownloadError::NoInterpreter {
                tried: vec![interpreter.display()],
            },
            _ => DownloadError::Spawn(error),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit_code = output.status.code().ok_or(DownloadError::NoExitCode)?;
    // ⭐ `#202`：脚本口径的 `0/1/2` **一定**先打印汇总行（`main()` 里汇总行在退出码判定之前）。
    // 没有汇总行 ⇒ 它崩在判定之前（第一句中文 `print` / import / 参数）⇒ 这不是判定。
    let Some(summary) = stdout
        .lines()
        .find(|line| line.starts_with("fetch-samples:"))
    else {
        return Err(DownloadError::NoReceipt {
            exit_code,
            command: options.command_line(&interpreter),
            stderr,
        });
    };
    let outcome = DownloadOutcome {
        exit_code,
        fetched: parse_count(summary, "取回 "),
        verified: parse_count(summary, "校验通过 "),
        total: parse_count(summary, "清单共 "),
        mismatched: parse_count(summary, "不符 "),
        command: options.command_line(&interpreter),
        stdout,
        stderr,
    };
    write_or_clear_stamp(&options.dest, &outcome);
    Ok(outcome)
}

/// 从 `"…取回 12 | …"` 里取紧跟在 `label` 之后的那串十进制数字。
fn parse_count(line: &str, label: &str) -> usize {
    let Some(start) = line.find(label) else {
        return 0;
    };
    let rest = &line[start + label.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

/// 退出码 `0` ⇒ 原子写戳；`1`（字节不符）⇒ 删掉旧戳；`2`（无法判定）⇒ **原样保留**。
///
/// 为什么 `2` 不删：离线 / 被沙箱拦时库里的字节**一位没动**，删戳只会把一个原本完好的
/// 安装变成"未安装"的假读数。而 `1` 是**已证实**的字节不符 —— 那一份"已安装"必须作废。
///
/// ⚠ `run_download` 提前返回 [`DownloadError`]（`NoInterpreter` / `NoReceipt`）时本函数
/// **根本不会被调用** ⇒ 旧戳原样保留：判不了 ≠ 字节坏了（`#202` 的 Windows 腿曾以退出码 `1`
/// 假装"字节不符"，那会把一份完好的安装误作废）。
fn write_or_clear_stamp(dest: &Path, outcome: &DownloadOutcome) {
    let stamp = dest.join(SAMPLE_LIBRARY_STAMP);
    if outcome.exit_code == 1 {
        let _ = std::fs::remove_file(&stamp);
        return;
    }
    if !outcome.ok() {
        return;
    }
    let document = serde_json::json!({
        "schema_version": 1,
        "verified": outcome.verified,
        "total": outcome.total,
        "mismatched": outcome.mismatched,
        "fetched": outcome.fetched,
        "library_root": dest.display().to_string(),
    });
    let Ok(text) = serde_json::to_vec_pretty(&document) else {
        return;
    };
    if std::fs::create_dir_all(dest).is_err() {
        return;
    }
    // 原子落盘（`ARCH-SEC-004` 的同一条纪律：先写临时文件再 rename）。
    let temporary = dest.join(format!("{SAMPLE_LIBRARY_STAMP}.tmp-{}", std::process::id()));
    if std::fs::write(&temporary, text).is_ok() {
        let _ = std::fs::rename(&temporary, &stamp);
    }
    let _ = std::fs::remove_file(&temporary);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 1（纯函数腿）：三平台路径公式逐字。
    #[test]
    fn preset_paths_for_three_platforms() {
        let home = Path::new("/Users/crow");
        let xdg = Path::new("/xdg");
        let appdata = Path::new(r"C:\Users\crow\AppData\Roaming");
        assert_eq!(
            preset_library_root(LibraryPlatform::MacOs, Some(home), Some(xdg), Some(appdata)),
            PathBuf::from("/Users/crow/Library/Application Support/dev.yeban.app/Samples")
        );
        assert_eq!(
            preset_library_root(LibraryPlatform::Linux, Some(home), Some(xdg), Some(appdata)),
            PathBuf::from("/xdg/yeban/samples")
        );
        assert_eq!(
            preset_library_root(LibraryPlatform::Linux, Some(home), None, Some(appdata)),
            PathBuf::from("/Users/crow/.local/share/yeban/samples")
        );
        assert_eq!(
            preset_library_root(
                LibraryPlatform::Windows,
                Some(home),
                Some(xdg),
                Some(appdata)
            ),
            PathBuf::from(r"C:\Users\crow\AppData\Roaming").join("yeban/samples")
        );
    }

    /// 判据 5 的汇总行解析：只吃 `fetch-samples:` 那一行，⛔ 不把别的数字认成读数。
    #[test]
    fn the_summary_counts_are_parsed_from_the_one_receipt_line() {
        let line = "fetch-samples: 取回 3 | 校验通过 2 | 不符 1 | 上游 404（路径推不出来）0 | \
                    跳过 0（清单共 3 项，磁盘上原有 0 项）";
        assert_eq!(parse_count(line, "取回 "), 3);
        assert_eq!(parse_count(line, "校验通过 "), 2);
        assert_eq!(parse_count(line, "不符 "), 1);
        assert_eq!(parse_count(line, "清单共 "), 3);
        assert_eq!(parse_count("", "取回 "), 0);
    }

    /// ⭐ `#201` 判据③的 GUI 侧：取回模式**显式**写进命令行，⛔ 不靠脚本缺省值。
    ///
    /// 无假源（真下载）⇒ `--archives`（脚本缺省也是它，但这里必须**写明**）；
    /// 有假源（`#196` 的扁平假上游树）⇒ `--per-file`，否则脚本会去找 tarball 而全 404。
    #[test]
    fn the_command_line_names_the_retrieval_mode_explicitly() {
        let mut options = DownloadOptions {
            manifest: PathBuf::from("/tmp/manifest.json"),
            dest: PathBuf::from("/tmp/library"),
            source_root: None,
        };
        let python = Interpreter::new("python3");
        let production = options.command_line(&python);
        assert!(production.contains("--archives"), "{production}");
        assert!(!production.contains("--per-file"), "{production}");
        assert_eq!(options.retrieval_mode_flag(), "--archives");

        options.source_root = Some(PathBuf::from("file:///tmp/fake-source"));
        let judged = options.command_line(&python);
        assert!(judged.contains("--per-file"), "{judged}");
        assert!(!judged.contains("--archives"), "{judged}");
        assert_eq!(options.retrieval_mode_flag(), "--per-file");
        // ⚠ 模式开关必须落在 `--all` 之后、`--manifest` 之前（与 `command()` 同序同参）。
        assert_eq!(
            judged.split(' ').take(3).collect::<Vec<_>>(),
            vec![
                "python3",
                &quote_path(&DownloadOptions::script_path()),
                "--all"
            ]
        );
        // ⭐ `#202`：`py -3` 的两段必须**都**出现在命令行首部（⛔ 把 `-3` 丢在后面）。
        let launcher = Interpreter::with_pre_args("py", &["-3"]);
        let via_launcher = options.command_line(&launcher);
        assert_eq!(
            via_launcher.split(' ').take(4).collect::<Vec<_>>(),
            vec![
                "py",
                "-3",
                &quote_path(&DownloadOptions::script_path()),
                "--all"
            ]
        );
    }

    /// ⭐ `#202` 判据 A（纯函数腿）：候选表**逐字**，且 `python3` 永远排第一（既有平台零变化）。
    ///
    /// ⚠ 两档**都在本机读**：平台是参数（`interpreter_candidates_for`），
    /// 所以 Windows 的 `py -3` 分支不是"只有 Windows 才跑"的死代码。
    #[test]
    fn the_interpreter_candidates_start_with_python3_and_windows_gets_the_launcher() {
        let unix: Vec<String> = interpreter_candidates_for(false)
            .iter()
            .map(Interpreter::display)
            .collect();
        assert_eq!(unix, vec!["python3", "python"]);
        let windows = interpreter_candidates_for(true);
        let displays: Vec<String> = windows.iter().map(Interpreter::display).collect();
        assert_eq!(displays, vec!["python3", "py -3", "python"]);
        assert_eq!(
            windows[1].program, "py",
            "Windows 的启动器是 `py` ＋ 前置参数 `-3`"
        );
        assert_eq!(windows[1].pre_args, vec!["-3".to_owned()]);
        assert_eq!(
            displays.first().map(String::as_str),
            Some("python3"),
            "⛔ 既有平台（macOS/Linux）的行为必须零变化：`python3` 永远排第一"
        );
        // 生产入口 = 当前平台的表（`cfg!(windows)` 只出现在这一处）。
        assert_eq!(
            interpreter_candidates(),
            interpreter_candidates_for(cfg!(windows))
        );
    }

    /// ⭐ `#202` 判据 A2（纯函数腿）：**只有 `py -3` 能用**时，它必须被选中，
    /// 且命令行首两段逐字是 `py -3`（⛔ 不是 `python3`）。
    #[test]
    fn the_py_launcher_wins_when_it_is_the_only_candidate_that_runs() {
        let tried = interpreter_candidates_for(true);
        let chosen = first_working(&tried, |candidate| candidate.program == "py")
            .expect("`py -3` 必须被选中");
        assert_eq!(chosen.display(), "py -3");
        let options = DownloadOptions {
            manifest: PathBuf::from("/tmp/manifest.json"),
            dest: PathBuf::from("/tmp/library"),
            source_root: None,
        };
        let line = options.command_line(&chosen);
        assert_eq!(
            line.split(' ').take(2).collect::<Vec<_>>(),
            vec!["py", "-3"],
            "{line}"
        );
    }

    /// ⭐ `#202` 判据 B（纯函数腿，⛔ 不装/卸任何 Python）：探测是**按序回退**的，
    /// 而且"一个都探不到"是 [`DownloadError::NoInterpreter`]（逐字列出试过的名字），⛔ 不是 `Ok`。
    #[test]
    fn the_first_working_candidate_wins_and_none_found_is_an_explicit_error() {
        let candidates = vec![
            Interpreter::new("first-imaginary-python"),
            Interpreter::new("second-imaginary-python"),
            Interpreter::new("third-imaginary-python"),
        ];
        // 档 1：只有第三个"能跑" ⇒ 选它（前两个被拒，⛔ 不是"名字存在就用"）。
        let probe_order = std::cell::RefCell::new(Vec::new());
        let chosen = first_working(&candidates, |candidate| {
            probe_order.borrow_mut().push(candidate.display());
            candidate.program == "third-imaginary-python"
        })
        .expect("第三个必须被选中");
        assert_eq!(chosen.program, "third-imaginary-python");
        assert_eq!(
            probe_order.into_inner(),
            vec![
                "first-imaginary-python",
                "second-imaginary-python",
                "third-imaginary-python"
            ],
            "探测顺序必须与候选表同序"
        );
        // 档 2：一个都跑不了 ⇒ 显式错误，⛔ 不是静默通过。
        let error = first_working(&candidates, |_| false).expect_err("全都跑不了⛔不许算成功");
        let DownloadError::NoInterpreter { tried } = &error else {
            panic!("必须是 NoInterpreter：{error:?}");
        };
        assert_eq!(
            tried,
            &vec![
                "first-imaginary-python".to_owned(),
                "second-imaginary-python".to_owned(),
                "third-imaginary-python".to_owned()
            ]
        );
        let text = error.to_string();
        assert!(text.contains("first-imaginary-python"), "{text}");
        assert!(text.contains("找不到能跑 Python 3 的解释器"), "{text}");
    }
}
