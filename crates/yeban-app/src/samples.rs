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

    /// 组装**将要执行**的命令（判据用 [`Self::command_line`] 留证，⛔ 不靠猜）。
    ///
    /// ⚠ 固定带 `--all`：这是"下载整个采样库"（不是 `--limit` 的有界档）。
    #[must_use]
    pub fn command(&self) -> Command {
        let mut command = Command::new("python3");
        command
            .arg(Self::script_path())
            .arg("--all")
            .arg("--manifest")
            .arg(&self.manifest)
            .arg("--dest")
            .arg(&self.dest);
        if let Some(source_root) = &self.source_root {
            command.arg("--source-root").arg(source_root);
        }
        command
    }

    /// 命令行的**可读留证**（路径含空格时加引号；与 [`Self::command`] 同序同参）。
    #[must_use]
    pub fn command_line(&self) -> String {
        let mut parts = vec![
            "python3".to_owned(),
            quote_path(&Self::script_path()),
            "--all".to_owned(),
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
    /// 子进程启动失败（没有 `python3` / 权限 / 沙箱）。
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
/// 2. 跑 [`DownloadOptions::command`]（`--all --manifest … --dest … [--source-root …]`）；
/// 3. 从 stdout 的 `fetch-samples:` 汇总行解析 `取回 / 校验通过 / 不符 / 清单共`；
/// 4. 退出码 `0` ⇒ 写安装戳（[`SAMPLE_LIBRARY_STAMP`]，含通过数 / 总项数）；
///    退出码 `1` ⇒ **删掉**旧戳（一次已证实的字节不符不许留下"已安装"的假读数）；
///    退出码 `2`（无法判定）⇒ 旧戳原样保留（离线不等于字节坏了）。
///
/// ⚠ 判据**不联网**：`source_root = Some(file:// 假源)` 时脚本读本地字节；
/// 命令行原文保存在 [`DownloadOutcome::command`]，可直接证明"没有走上游"。
///
/// # Errors
///
/// [`DownloadError`]：脚本缺失 / 子进程启动失败 / 没有退出码。
pub fn run_download(options: &DownloadOptions) -> Result<DownloadOutcome, DownloadError> {
    let script = DownloadOptions::script_path();
    if !script.is_file() {
        return Err(DownloadError::ScriptMissing(script));
    }
    let output = options.command().output().map_err(DownloadError::Spawn)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit_code = output.status.code().ok_or(DownloadError::NoExitCode)?;
    let summary = stdout
        .lines()
        .find(|line| line.starts_with("fetch-samples:"))
        .unwrap_or("");
    let outcome = DownloadOutcome {
        exit_code,
        fetched: parse_count(summary, "取回 "),
        verified: parse_count(summary, "校验通过 "),
        total: parse_count(summary, "清单共 "),
        mismatched: parse_count(summary, "不符 "),
        command: options.command_line(),
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
}
