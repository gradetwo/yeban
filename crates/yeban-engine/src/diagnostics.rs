//! `[D56]` **按需诊断包**：把调试信息与相关文件采集进一个 `.zip`，供人工取回复现与排查。
//!
//! 负责人指令：「UI和MCP都要有采集调试信息及相关文件然后压缩包导出的功能，在遇到特殊问题时候，手工调用后采集信息回来复现和排查」。
//!
//! **为什么住在 `yeban-engine`**：UI（`yeban-app`）与 MCP（`yeban-mcp`）**都已**依赖引擎，而 D56 的判据要求两条入口
//! 调**同一个**函数。引擎也是唯一能满足红线 3（引擎层无 GUI 依赖）的共享位置 —— 本模块只用 `std` / `zip` / `sha2`。
//!
//! **依赖是预留的**：`zip 8.6.0`、`flate2 1.1.10`、`sha2 0.11.0` 在根清单里**早已声明**（本模块零新增依赖）。
//!
//! **隐私默认**：工程文件默认**不含**（`BundleInputs::project` 为空即不含）。写入前对所有条目做一次**脱敏**：
//! 把 `$HOME` 的绝对路径替换为字面量 `$HOME`。
use std::io::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// 采集内容由**调用方**提供 —— 采集器不猜业务状态。
#[derive(Default)]
pub struct BundleInputs<'a> {
    /// `engine-state.json`：足以复现"当时处于什么状态"的引擎/会话快照。
    pub state_json: Option<&'a str>,
    /// `config.json`：本机配置层（调用方应已初筛；本模块仍会做一次路径脱敏）。
    pub config_json: Option<&'a str>,
    /// `logs/`：日志环形缓冲的副本。
    pub logs: &'a [(String, Vec<u8>)],
    /// `crashes/`：崩溃报告或上次异常退出的标记。
    pub crashes: &'a [(String, Vec<u8>)],
    /// `project/`：**仅在用户显式勾选时**才非空（默认空 ⇒ 不含工程，这是隐私决定）。
    pub project: &'a [(String, Vec<u8>)],
}

/// 包内一个条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleEntry {
    /// zip 内的相对路径。
    pub name: String,
    /// 字节数。
    pub bytes: u64,
    /// `sha256` 十六进制小写。
    pub sha256: String,
}

/// 导出结果。
#[derive(Debug, Clone)]
pub struct BundleReport {
    /// 落盘的 `.zip` 路径。
    pub path: PathBuf,
    /// 压缩包字节数。
    pub bytes: u64,
    /// 压缩包自身的 `sha256`。
    pub sha256: String,
    /// 包内条目（含 `MANIFEST.txt` 自身）。
    pub entries: Vec<BundleEntry>,
}

/// 失败原因。**不**吞异常：调用方（UI 对话框 / MCP 错误码）需要区分。
#[derive(Debug)]
pub enum DiagError {
    /// 文件系统失败。
    Io(String),
    /// 打包失败。
    Zip(String),
    /// 输出目录不存在。
    MissingOutDir(PathBuf),
}

impl std::fmt::Display for DiagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(m) => write!(f, "诊断包 I/O 失败: {m}"),
            Self::Zip(m) => write!(f, "诊断包打包失败: {m}"),
            Self::MissingOutDir(p) => write!(f, "输出目录不存在: {}", p.display()),
        }
    }
}

impl std::error::Error for DiagError {}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// 脱敏：把 `$HOME` 的绝对路径替换为字面量 `$HOME`。
///
/// 判据要求它**机械可验证**（对包内每个文件扫描，断言不出现真实 `$HOME` 字符串）。
#[must_use]
pub fn redact(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec(); // 二进制条目原样保留
    };
    let mut out = text.to_string();
    // 拉平写法：`if let ... { if ... }` 会被 clippy 的 collapsible_if 拦下（-D warnings 下即失败）。
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        out = out.replace(&home, "$HOME");
    }
    out.into_bytes()
}

fn env_txt() -> Vec<u8> {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let mut s = String::new();
    s.push_str(&format!("os={}\n", std::env::consts::OS));
    s.push_str(&format!("arch={}\n", std::env::consts::ARCH));
    s.push_str(&format!("family={}\n", std::env::consts::FAMILY));
    s.push_str(&format!("pkg=yeban-engine {}\n", env!("CARGO_PKG_VERSION")));
    s.push_str(&format!("profile={profile}\n"));
    s.push_str(&format!(
        "features={}\n",
        if cfg!(feature = "default") {
            "default"
        } else {
            "custom"
        }
    ));
    s.into_bytes()
}

fn git_txt() -> Vec<u8> {
    // 无 git 时写"不可用" —— 判据要求**不得留空**。
    fn run(args: &[&str]) -> Option<String> {
        let out = std::process::Command::new("git").args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
    let head = run(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unavailable".into());
    let branch =
        run(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "unavailable".into());
    let dirty = run(&["status", "--porcelain"]).unwrap_or_else(|| "unavailable".into());
    let dirty_flag = if dirty == "unavailable" {
        "unavailable".to_string()
    } else if dirty.is_empty() {
        "clean".to_string()
    } else {
        format!("dirty ({} entries)", dirty.lines().count())
    };
    format!("head={head}\nbranch={branch}\nworktree={dirty_flag}\n").into_bytes()
}

fn now_stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `MANIFEST.txt` 由**同一份条目列表**生成 —— 即"实际写进 zip 的那份"，
/// 所以 sha256 判据比对的是真实产物，而不是第二份单独拼的清单。
fn manifest_txt(entries: &[BundleEntry]) -> Vec<u8> {
    let mut s = format!(
        "# yeban diagnostic bundle\nschema=1\ngenerated_unix={}\nentries={}\n",
        now_stamp(),
        entries.len()
    );
    for e in entries {
        s.push_str(&format!("{}\t{}\t{}\n", e.sha256, e.bytes, e.name));
    }
    s.into_bytes()
}

/// 采集并打包。返回落盘路径、字节数与包内清单。
///
/// # Errors
/// 输出目录不存在、写盘失败或打包失败时返回 [`DiagError`]。
pub fn export_diagnostics(
    out_dir: &Path,
    inputs: BundleInputs<'_>,
) -> Result<BundleReport, DiagError> {
    if !out_dir.is_dir() {
        return Err(DiagError::MissingOutDir(out_dir.to_path_buf()));
    }

    // 1) 收集内容条目（先脱敏，再哈希 —— 哈希必须对应"实际写出的字节"）。
    let mut items: Vec<(String, Vec<u8>)> =
        vec![("env.txt".into(), env_txt()), ("git.txt".into(), git_txt())];
    if let Some(s) = inputs.state_json {
        items.push(("engine-state.json".into(), s.as_bytes().to_vec()));
    }
    if let Some(c) = inputs.config_json {
        items.push(("config.json".into(), c.as_bytes().to_vec()));
    }
    for (name, bytes) in inputs.logs {
        items.push((format!("logs/{name}"), bytes.clone()));
    }
    for (name, bytes) in inputs.crashes {
        items.push((format!("crashes/{name}"), bytes.clone()));
    }
    for (name, bytes) in inputs.project {
        items.push((format!("project/{name}"), bytes.clone()));
    }
    let items: Vec<(String, Vec<u8>)> = items.into_iter().map(|(n, b)| (n, redact(&b))).collect();

    // 2) 逐条哈希。
    let mut entries: Vec<BundleEntry> = items
        .iter()
        .map(|(name, bytes)| BundleEntry {
            name: name.clone(),
            bytes: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        })
        .collect();

    // 3) MANIFEST 由上面那份列表生成，然后它自己也成为一个条目。
    let manifest = manifest_txt(&entries);
    entries.push(BundleEntry {
        name: "MANIFEST.txt".into(),
        bytes: manifest.len() as u64,
        sha256: sha256_hex(&manifest),
    });

    // 4) 写 zip（依赖是根清单预留的 `zip 8.6`）。
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, bytes) in &items {
            zw.start_file(name.as_str(), opts)
                .map_err(|e| DiagError::Zip(e.to_string()))?;
            zw.write_all(bytes)
                .map_err(|e| DiagError::Io(e.to_string()))?;
        }
        zw.start_file("MANIFEST.txt", opts)
            .map_err(|e| DiagError::Zip(e.to_string()))?;
        zw.write_all(&manifest)
            .map_err(|e| DiagError::Io(e.to_string()))?;
        zw.finish().map_err(|e| DiagError::Zip(e.to_string()))?;
    }
    let zip_bytes = buf.into_inner();

    // 5) 落盘。文件名确定性可排序：时间戳 + 内容短 sha。
    let digest = sha256_hex(&zip_bytes);
    let short = &digest[..7];
    let path = out_dir.join(format!("yeban-diagnostics-{}-{short}.zip", now_stamp()));
    std::fs::write(&path, &zip_bytes).map_err(|e| DiagError::Io(e.to_string()))?;

    Ok(BundleReport {
        path,
        bytes: zip_bytes.len() as u64,
        sha256: digest,
        entries,
    })
}
