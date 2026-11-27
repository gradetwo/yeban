//! T-FIX-158：demo 选择页的**远程 demo 下载**核心（无 UI 依赖）。
//!
//! 8 首 demo 常驻清单（名字/子目录/工程文件名/封面文件名）＋ 下载执行器：
//! - 基址可用 `YEBAN_DEMOS_REMOTE_BASE` 覆盖（判据用本地夹具服务器，不依赖真 GitHub）；
//! - 下载写**临时文件**，校验通过才 `rename` 成正式名（半成品绝不冒充已下载）；
//! - 进度事件单调递增；可取消（取消后不留任何文件）；失败带可重试的错误文本。
//!
//! ⛔ 本模块不做 UI：卡片状态由调用方按事件驱动。

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 一首远程 demo 的静态描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemoSpec {
    /// extras 仓 `demos/` 下的子目录名。
    pub dir: &'static str,
    /// 工程文件名（`.yeban` 容器）。
    pub file: &'static str,
    /// 封面文件名（与工程同目录）。
    pub cover: &'static str,
    /// 干净歌名（卡片主面显示）。
    pub title: &'static str,
}

/// 8 首内置远程 demo（与 gradetwo/yeban-extras 的 demos/ 逐项对应）。
pub const DEMOS: [DemoSpec; 8] = [
    DemoSpec {
        dir: "雨声太吵",
        file: "雨声太吵.yeban",
        cover: "雨声太吵.png",
        title: "雨声太吵",
    },
    DemoSpec {
        dir: "潮",
        file: "潮.yeban",
        cover: "潮.png",
        title: "潮",
    },
    DemoSpec {
        dir: "失重",
        file: "失重.yeban",
        cover: "失重.png",
        title: "失重",
    },
    DemoSpec {
        dir: "失语",
        file: "失语.yeban",
        cover: "失语.jpg",
        title: "失语",
    },
    DemoSpec {
        dir: "呢喃",
        file: "呢喃.yeban",
        cover: "呢喃.jpg",
        title: "呢喃",
    },
    DemoSpec {
        dir: "下一个路口",
        file: "下一个路口.yeban",
        cover: "下一个路口.png",
        title: "下一个路口",
    },
    DemoSpec {
        dir: "回头路",
        file: "回头路.yeban",
        cover: "回头路.png",
        title: "回头路",
    },
    DemoSpec {
        dir: "那个午后",
        file: "那个午后.yeban",
        cover: "那个午后.png",
        title: "那个午后",
    },
];

/// 默认远端基址（GitHub media 端点；LFS 对象必须走 media 域）。
pub const DEFAULT_REMOTE_BASE: &str =
    "https://media.githubusercontent.com/media/gradetwo/yeban-extras/main/demos";

/// 远端基址：`YEBAN_DEMOS_REMOTE_BASE` 覆盖优先，否则用默认值。
#[must_use]
pub fn remote_base() -> String {
    std::env::var("YEBAN_DEMOS_REMOTE_BASE").unwrap_or_else(|_| DEFAULT_REMOTE_BASE.to_owned())
}

/// 某首 demo 的工程下载 URL。
#[must_use]
pub fn project_url(base: &str, spec: &DemoSpec) -> String {
    format!(
        "{}/{}/{}",
        base.trim_end_matches('/'),
        encode_segment(spec.dir),
        encode_segment(spec.file)
    )
}

/// 路径段百分号编码（非 ASCII 与控制字符全部转义）。
///
/// http 1.5 的 URI 只接受 ASCII（127 以上非法）；中文目录名不编码时 ureq 在
/// agent.get() 处就解析失败，一个字节都不会发出。
#[must_use]
pub fn encode_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        let ch = *byte;
        if ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'.' | b'_' | b'~') {
            out.push(ch as char);
        } else {
            out.push('%');
            out.push_str(&format!("{ch:02X}"));
        }
    }
    out
}

/// 某首 demo 的封面下载 URL。
#[must_use]
pub fn cover_url(base: &str, spec: &DemoSpec) -> String {
    format!(
        "{}/{}/{}",
        base.trim_end_matches('/'),
        encode_segment(spec.dir),
        encode_segment(spec.cover)
    )
}

/// 下载进度事件（UI 按它更新卡片）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadEvent {
    /// 已收字节数 / 总量（`None` = 服务端没给 Content-Length）。
    Progress {
        /// 已收字节数。
        received: u64,
        /// 总字节数（None = 服务端没给 Content-Length）。
        total: Option<u64>,
    },
    /// 完成：正式路径（已完成校验与改名）。
    Finished(PathBuf),
    /// 失败：人读原因（可重试）。
    Failed(String),
    /// 已取消（半成品已清理）。
    Cancelled,
}

/// 下载失败的原因分类（判据与卡片文案都读它）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadError {
    /// 网络/HTTP 失败。
    Network,
    /// 落盘失败。
    Io,
    /// 校验失败（不是合法容器）。
    Corrupt,
    /// 调用方取消。
    Cancelled,
}

impl DownloadError {
    /// 机器可读原因名。
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Io => "io",
            Self::Corrupt => "corrupt",
            Self::Cancelled => "cancelled",
        }
    }
}

/// 取消把手：`cancel()` 置位后，下载循环在下一次读块时退出并清理半成品。
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// 新建未置位的取消把手。
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    /// 请求取消：下载循环在下一次读块时退出并清理半成品。
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    /// 是否已请求取消。
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// 半成品临时名：`<file>.part`（完成才 `rename` 成正式名）。
#[must_use]
pub fn part_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    target.with_file_name(name)
}

/// 容器校验：ZIP 魔数 `PK\x03\x04`（`.yeban` 是自包含 ZIP 容器）。
///
/// 拿不到 LFS 指针 size 时的兜底口径：**至少**能认出是容器；读不出魔数 ⇒ `Corrupt`。
pub fn looks_like_container(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == *b"PK\x03\x04"
}

/// `.part` 半成品的生命周期守卫：从临时文件创建起接管，任何错误返回
/// 都由 `Drop` 统一删除；成功 rename 后 `disarm`，不碰 target。
struct PartCleanup {
    path: PathBuf,
    armed: bool,
}

impl PartCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    /// 成功 rename 后解除守卫（`.part` 已不存在，target 不能被误删）。
    fn disarm(&mut self) {
        self.armed = false;
    }

    /// 显式清理并解除守卫；清理失败只记日志，不改变调用方正在返回的
    /// 原始下载错误。
    fn cleanup_now(&mut self) {
        if self.armed {
            self.armed = false;
            self.remove_part();
        }
    }

    fn remove_part(&self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                eprintln!(
                    "demo-download: 清理半成品 `{}` 失败: {error}",
                    self.path.display()
                );
            }
        }
    }
}

impl Drop for PartCleanup {
    fn drop(&mut self) {
        if self.armed {
            self.remove_part();
        }
    }
}

/// 同步下载到 `target`：临时文件 → 校验 → 改名。返回最终路径。
///
/// `notify` 收到每一次 Progress；取消时返回 `Err(Cancelled)` 且**不留文件**；
/// 每条失败路径在返回 `Err` 前恰好发一次 `DownloadEvent::Failed`（人读原因，
/// 可重试）；成功路径只发一次 `Finished`，取消路径只发一次 `Cancelled`。
pub fn download_to<F>(
    url: &str,
    target: &Path,
    cancel: &CancelFlag,
    mut notify: F,
) -> Result<PathBuf, (DownloadError, String)>
where
    F: FnMut(DownloadEvent),
{
    // T-FIX-640：失败终态统一收口 —— 内层只管干活（Progress/Cancelled/
    // Finished 照旧），任何非取消的 Err 在这里恰好补一次 Failed（人读原因
    // 即返回的错误文本），再把 Err 原样返回。单出口 ⇒ 每条失败路径恰好一次，
    // 成功与取消路径零次。
    match download_to_inner(url, target, cancel, &mut notify) {
        Ok(path) => Ok(path),
        Err((error, message)) => {
            if error != DownloadError::Cancelled {
                notify(DownloadEvent::Failed(message.clone()));
            }
            Err((error, message))
        }
    }
}

/// `download_to` 的内层：临时文件 → 校验 → 改名。`notify` 收到每一次
/// Progress；取消时发一次 `Cancelled` 并返回 `Err(Cancelled)`。
fn download_to_inner<F>(
    url: &str,
    target: &Path,
    cancel: &CancelFlag,
    mut notify: F,
) -> Result<PathBuf, (DownloadError, String)>
where
    F: FnMut(DownloadEvent),
{
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| (DownloadError::Io, format!("建目录失败: {error}")))?;
    }
    let part = part_path(target);
    let _ = std::fs::remove_file(&part);
    let mut part_cleanup = PartCleanup::new(part.clone());

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(15)))
        .timeout_per_read(Some(std::time::Duration::from_secs(30)))
        .build()
        .into();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| (DownloadError::Network, format!("请求失败: {error}")))?;
    let total: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(&part)
        .map_err(|error| (DownloadError::Io, format!("建临时文件失败: {error}")))?;
    let mut received: u64 = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancel.is_cancelled() {
            drop(file);
            part_cleanup.cleanup_now();
            notify(DownloadEvent::Cancelled);
            return Err((DownloadError::Cancelled, "已取消".to_owned()));
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| (DownloadError::Network, format!("读取失败: {error}")))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| (DownloadError::Io, format!("写入失败: {error}")))?;
        received += read as u64;
        notify(DownloadEvent::Progress { received, total });
    }
    file.flush()
        .map_err(|error| (DownloadError::Io, format!("flush 失败: {error}")))?;
    drop(file);

    if let Some(total) = total
        && received != total
    {
        return Err((
            DownloadError::Network,
            format!("下载不完整: {received} / {total} 字节"),
        ));
    }
    if !looks_like_container(&part) {
        return Err((
            DownloadError::Corrupt,
            "下载内容不是合法的工程容器".to_owned(),
        ));
    }
    if let Err(error) = std::fs::rename(&part, target) {
        return Err((DownloadError::Io, format!("改名失败: {error}")));
    }
    part_cleanup.disarm();
    notify(DownloadEvent::Finished(target.to_path_buf()));
    Ok(target.to_path_buf())
}

/// demo 的本地缓存路径：`<root>/<dir>/<file>`。
#[must_use]
pub fn cached_project(root: &Path, spec: &DemoSpec) -> PathBuf {
    root.join(spec.dir).join(spec.file)
}

/// 已下载判定：正式文件存在且是容器（半成品不算）。
#[must_use]
pub fn is_downloaded(root: &Path, spec: &DemoSpec) -> bool {
    let path = cached_project(root, spec);
    path.is_file() && looks_like_container(&path)
}

/// 官方 demo 的本地缓存根目录（T-FIX-951）：`YEBAN_DEMOS_DIR` 设了就与清单扫描
/// 同一处（下载完成即出现在演示分类）；未设时退到用户缓存目录，⛔ 不写仓库。
///
/// 不引入 `dirs` 之类的新依赖：XDG / Windows 本地应用数据 / HOME 依次尝试，
/// 全拿不到才退到临时目录（功能仍可用，只是不保证跨会话持久）。
#[must_use]
pub fn cache_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("YEBAN_DEMOS_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA").map(|value| PathBuf::from(value).join("cache"))
        })
        .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("yeban").join("demos")
}
