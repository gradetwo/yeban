//! 音源管理（line/sound-library-manager，Wang 2026-11-17 令）：主菜单
//! 「设置 → 音源管理...」背后的**纯逻辑层** —— 已安装音源库扫描、
//! 可下载目录解析与拉取、包下载 → 校验 → 解包安装、删除。
//!
//! 学的是 Logic Pro Sound Library 管理器的**思想**（已安装 / 可下载分开
//! 管、下载完先校验再装、装完出现在已安装列表里），布局与代码形态按
//! 夜半自己的规矩走：本模块只做事，界面住在
//! `ui/dialogs/sound_library_dialog.slint`，接线住在
//! `host::wire_sound_library_dialog`。
//!
//! ## 三个落点（⛔ 本模块不另造路径口径）
//!
//! - **用户音源目录** = [`crate::samples::library_root`]（`#196` 的既定
//!   库根：平台用户数据目录，`YEBAN_SAMPLE_LIBRARY` 可覆盖）。下载的包
//!   解包到 `<库根>/<包 id>/`，与既有采样库的 `<库根>/<乐器>/…` 布局
//!   并排共存；每个包目录里写一份标记 [`PACK_MARKER_FILE`] 记名。
//! - **内置音色库** = 仓库 `assets/samples/`（随仓分发的 `.sfz` 定义；
//!   样本字节按清单口径不随仓分发，见 `assets/samples/manifest.json`）。
//!   它在已安装列表里是**一条、不可删**。
//! - **临时区** = `<库根>/.incoming/`：下载先落 `.part`，校验与解包全
//!   在临时区里完成，最后才把成品目录整体改名进库根 ⇒ 失败 / 中断
//!   **不留半成品**（已安装列表只认库根下的正式目录，扫描跳过 `.` 开头
//!   的隐藏目录，临时区天然不可见）。
//!
//! ## 目录（catalog）
//!
//! 地址 [`CATALOG_URL`]（`yeban-extras` 仓的 `sounds/catalog.json`，
//! 格式版本 [`CATALOG_FORMAT_VERSION`]）。目录初始为空是**真实状态**：
//! 解析出零个包就如实报零个，⛔ 不编造示例包。拉不到（离线 / 404 /
//! 地址被环境覆盖到不可达处）⇒ [`CatalogError::Unavailable`]，界面只
//! 显示已安装部分并明确提示，⛔ 不拿缓存或假数据充数。
//!
//! 两个环境钩子（与 `samples::INTERPRETER_ENV` 同款的测试 / 取证分界，
//! 生产不设置）：
//! - [`CATALOG_URL_ENV`]：覆盖目录地址；
//! - [`CATALOG_FILE_ENV`]：设置后从本地文件读目录 JSON（判据与证据图
//!   用它走**同一条**解析路径，不另设假入口）。
//!
//! ## 依赖裁决（本线唯一的新增包）
//!
//! 仓内此前**没有任何** HTTP 客户端，目录与包都在 https 上 ⇒ 引入
//! `ureq`（rustls / ring，许可在 `deny.toml` 白名单内，ring 的 clarify
//! 豁免本就是为 TLS 预留的）。校验用 `sha2`、解包用 `zip`，两者早已是
//! workspace 依赖（`yeban-diagnostics` / `yeban-model` 在用），⛔ 不是
//! 本线新增。归档格式因此定为 **`.zip`**。
//!
//! ## ⚠ 本模块**不**做的事
//!
//! - ⛔ 不解析 SFZ、不决定样本路径在哪找 —— 那是
//!   [`crate::sfz_load::resolve_sampler_path`] 的唯一职责；装进库根的
//!   包只是躺在库根候选能覆盖的目录里。
//! - ⛔ 不在启动 / 导入时自动联网：拉取只由界面显式打开管理器或点
//!   下载触发（接线层保证），本模块自己永不被投影路径调用。
//! - ⛔ 不动既有采样库下载（`crate::samples` 的 `fetch-samples.py`
//!   清单机制）：两条机制落点同为库根、互不覆盖对方的目录。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::Digest;

/// 可下载目录的固定地址（`yeban-extras` 仓 `sounds/catalog.json`）。
pub const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/gradetwo/yeban-extras/main/sounds/catalog.json";

/// 覆盖目录地址的环境变量（非空即以它为准；测试 / 镜像用）。
pub const CATALOG_URL_ENV: &str = "YEBAN_SOUND_CATALOG_URL";

/// 从本地文件读目录的环境变量（非空即以它为准，**优先于**网络地址；
/// 判据与证据图的注入钩子，生产不设置）。
pub const CATALOG_FILE_ENV: &str = "YEBAN_SOUND_CATALOG_FILE";

/// 目录格式版本：解析时只认这一个版本，其余如实报错、不猜。
pub const CATALOG_FORMAT_VERSION: u64 = 1;

/// 装好的包目录里的标记文件（JSON：`id` / `name` / `version` / `sha256`）。
/// 扫描时用它取包的显示名；没有标记的目录按目录名显示（那是既有采样库
/// 机制装下的乐器目录，同为用户内容、同样可删）。
pub const PACK_MARKER_FILE: &str = ".yeban-pack.json";

/// 临时区目录名（库根下，`.` 开头 ⇒ 扫描跳过、界面不可见）。
pub const STAGING_DIR_NAME: &str = ".incoming";

/// 内置音色库条目 id 的前缀（后接目录名）：`builtin:<目录名>`。
///
/// 早先是单条聚合行（id 恒为 `builtin`），用户看得出"有东西"却看不出装了哪几件；
/// 逐件列出后每个内置子目录各占一行，删除入口统一按**前缀**拒绝（内置在仓库里，
/// 删了等于拆工程，不是卸载音源）。
pub const BUILTIN_ID_PREFIX: &str = "builtin:";

/// 用户库条目 id 的前缀（后接目录名）：`user:<目录名>`。
pub const USER_ID_PREFIX: &str = "user:";

/// 解包总量上限（zip 炸弹护栏）：单包解出超过这个字节数即中止报错。
pub const MAX_UNPACKED_BYTES: u64 = 20 * 1024 * 1024 * 1024;

/// 目录 JSON 的拉取上限（目录只是清单，超过即视为不可用）。
const MAX_CATALOG_BYTES: u64 = 4 * 1024 * 1024;

/// 音源管理的两个根（判据显式注入，生产走 [`SoundRoots::production`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundRoots {
    /// 用户音源目录（包的安装落点、用户库的扫描根）。
    pub library_root: PathBuf,
    /// 内置音色库根（仓库 `assets/samples/`）。
    pub builtin_root: PathBuf,
}

impl SoundRoots {
    /// 生产口径：库根取 [`crate::samples::library_root`]（环境覆盖在
    /// 同一处生效），内置根取仓库根下的 `assets/samples/`。
    #[must_use]
    pub fn production() -> Self {
        Self {
            library_root: crate::samples::library_root(),
            builtin_root: crate::sfz_load::repo_root().join("assets/samples"),
        }
    }
}

/// 已安装列表里的一条（内置每件一条 + 用户目录各一条）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledLibrary {
    /// 稳定 id：内置为 `builtin:<目录名>`，用户库为 `user:<目录名>`。
    pub id: String,
    /// 显示名：包取标记里的名字，其余取目录名。
    pub name: String,
    /// 位置（目录的绝对路径）。
    pub location: PathBuf,
    /// 占用大小（字节，递归求和；读不到的条目按 0 计，不中断扫描）。
    pub size_bytes: u64,
    /// 是否可删（内置恒 `false`）。
    pub deletable: bool,
}

/// 目录里的一个可下载包（字段与 `sounds/catalog.json` 逐字对应）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogPack {
    /// 包 id（同时是安装后的目录名，必须是合法的单段目录名）。
    pub id: String,
    /// 显示名。
    pub name: String,
    /// 一句说明。
    pub description: String,
    /// 包版本（显示用字符串）。
    pub version: String,
    /// 归档字节数（目录里的 `sizeBytes`；下载后逐字节对账）。
    pub size_bytes: u64,
    /// 归档的 SHA-256（64 位十六进制；下载后逐字节对账）。
    pub sha256: String,
    /// 归档直链。
    pub url: String,
}

/// 一份解析好的目录。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    /// 包列表（可为空 —— 空是真实状态）。
    pub packs: Vec<CatalogPack>,
}

/// 目录侧失败（界面据此只显示已安装部分并明确提示离线 / 不可用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    /// 拉不到：离线、404、地址不可达、文件钩子读不到。
    Unavailable(String),
    /// 拿到了但不是合法目录：JSON 坏 / 缺字段 / 字段类型不对。
    Malformed(String),
    /// 格式版本不是 [`CATALOG_FORMAT_VERSION`]。
    UnsupportedVersion(u64),
}

impl CatalogError {
    /// 用户可见的中文原因（界面原样显示，⛔ 不在这里吞细节）。
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Unavailable(detail) => format!("无法获取下载目录：{detail}"),
            Self::Malformed(detail) => format!("下载目录格式不对：{detail}"),
            Self::UnsupportedVersion(version) => {
                format!("下载目录版本 {version} 不受支持（本机只认版本 {CATALOG_FORMAT_VERSION}）")
            }
        }
    }
}

/// 安装 / 删除侧失败。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoundError {
    /// 下载传输失败（HTTP 状态 / 连接 / 读流）。
    Download(String),
    /// 文件系统失败（建目录 / 写文件 / 改名 / 删除）。
    Io(String),
    /// 下载到的字节数与目录登记不符。
    SizeMismatch {
        /// 目录登记的字节数。
        expected: u64,
        /// 实际拿到的字节数。
        actual: u64,
    },
    /// 下载到的字节算出的 SHA-256 与目录登记不符（包已弃）。
    ChecksumMismatch {
        /// 目录登记的 SHA-256。
        expected: String,
        /// 实际算出的 SHA-256。
        actual: String,
    },
    /// 归档本身不可装：打不开、含不安全路径 / 符号链接、解包超限。
    BadArchive(String),
    /// 内置音色库不可删。
    BuiltInNotDeletable,
    /// 删除目标不存在或 id 不认识。
    NotFound(String),
}

impl SoundError {
    /// 用户可见的中文原因。
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Download(detail) => format!("下载失败：{detail}"),
            Self::Io(detail) => format!("文件操作失败：{detail}"),
            Self::SizeMismatch { expected, actual } => {
                format!("包大小不符：目录登记 {expected} 字节，实际下载 {actual} 字节，已弃")
            }
            Self::ChecksumMismatch { expected, actual } => {
                format!("包校验不符：目录登记 sha256 {expected}，实际 {actual}，已弃")
            }
            Self::BadArchive(detail) => format!("包不可用：{detail}"),
            Self::BuiltInNotDeletable => "内置音色库不可删除".to_owned(),
            Self::NotFound(id) => format!("找不到要删除的音源库：{id}"),
        }
    }
}

/// 目录名 / 包 id 的合法形态：单段、非空、不以 `.` 开头、无路径分隔符。
/// 安装落点与删除目标都由它把关（⛔ 不许 `../` 之类逃出库根）。
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// 递归求和一个目录的字节数（不跟随符号链接指向的目录；读不到的条目
/// 按 0 计 —— 扫描是展示口径，⛔ 不许一条坏条目让整个列表消失）。
#[must_use]
pub fn dir_size(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                total += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            }
        }
    }
    total
}

/// 读包标记里的显示名（标记缺失 / 坏 ⇒ `None`，调用方按目录名显示）。
fn marker_name(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join(PACK_MARKER_FILE)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = value.get("name")?.as_str()?;
    (!name.is_empty()).then(|| name.to_owned())
}

/// 扫描已安装列表：**内置逐件一条**（`builtin_root` 下每个非隐藏子目录 = 一件）
/// + 库根下每个非隐藏子目录一条，两组内各自按目录名排序（确定性口径）。
///
/// ⚠ 早先内置只有一条聚合行（「内置音色库 17 MiB」）——用户看得见"有东西"，却
/// 看不出**装了哪几件**、更没法对着某一件去换。逐件列出后"我装没装这个包"变成
/// 一眼可查；内置仍**不可删**（它在仓库里）。
#[must_use]
pub fn scan_installed(roots: &SoundRoots) -> Vec<InstalledLibrary> {
    let mut out = Vec::new();
    let mut builtin_dirs: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = fs::read_dir(&roots.builtin_root) {
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            builtin_dirs.push((name, entry.path()));
        }
    }
    builtin_dirs.sort();
    for (dir_name, path) in builtin_dirs {
        out.push(InstalledLibrary {
            id: format!("{BUILTIN_ID_PREFIX}{dir_name}"),
            name: marker_name(&path).unwrap_or_else(|| dir_name.clone()),
            size_bytes: dir_size(&path),
            location: path,
            deletable: false,
        });
    }
    let mut user_dirs: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = fs::read_dir(&roots.library_root) {
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            user_dirs.push((name, entry.path()));
        }
    }
    user_dirs.sort();
    for (dir_name, path) in user_dirs {
        out.push(InstalledLibrary {
            id: format!("{USER_ID_PREFIX}{dir_name}"),
            name: marker_name(&path).unwrap_or_else(|| dir_name.clone()),
            size_bytes: dir_size(&path),
            location: path,
            deletable: true,
        });
    }
    out
}

/// 解析目录 JSON（判据与生产共用同一条路径）。
///
/// 字段口径逐字对 `yeban-extras/sounds/README.md` 的格式说明：根对象
/// `version`（必须等于 [`CATALOG_FORMAT_VERSION`]）+ `packs` 数组；
/// 每包 `id` / `name` / `description` / `version` / `sizeBytes` /
/// `sha256` / `url` 七个字段全是必填，缺哪个就在错误里点名哪个。
pub fn parse_catalog(text: &str) -> Result<Catalog, CatalogError> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|err| CatalogError::Malformed(format!("JSON 无法解析（{err}）")))?;
    let root = value
        .as_object()
        .ok_or_else(|| CatalogError::Malformed("根不是 JSON 对象".to_owned()))?;
    let version = match root.get("version") {
        None => return Err(CatalogError::Malformed("缺少字段 version".to_owned())),
        Some(v) => v
            .as_u64()
            .ok_or_else(|| CatalogError::Malformed("字段 version 不是整数".to_owned()))?,
    };
    if version != CATALOG_FORMAT_VERSION {
        return Err(CatalogError::UnsupportedVersion(version));
    }
    let packs_value = match root.get("packs") {
        None => return Err(CatalogError::Malformed("缺少字段 packs".to_owned())),
        Some(v) => v
            .as_array()
            .ok_or_else(|| CatalogError::Malformed("字段 packs 不是数组".to_owned()))?,
    };
    let mut packs = Vec::with_capacity(packs_value.len());
    for (index, item) in packs_value.iter().enumerate() {
        packs.push(parse_pack(index, item)?);
    }
    Ok(Catalog { packs })
}

/// 解析一个包条目；缺字段 / 类型不对 / id 不合法 / sha256 形态不对，
/// 都在错误里点名（第几条 + 哪个字段）。
fn parse_pack(index: usize, item: &serde_json::Value) -> Result<CatalogPack, CatalogError> {
    let obj = item
        .as_object()
        .ok_or_else(|| CatalogError::Malformed(format!("packs[{index}] 不是 JSON 对象")))?;
    let string_field = |field: &str| -> Result<String, CatalogError> {
        match obj.get(field) {
            None => Err(CatalogError::Malformed(format!(
                "packs[{index}] 缺少字段 {field}"
            ))),
            Some(v) => v.as_str().map(str::to_owned).ok_or_else(|| {
                CatalogError::Malformed(format!("packs[{index}] 的字段 {field} 不是字符串"))
            }),
        }
    };
    let id = string_field("id")?;
    if !is_plain_name(&id) {
        return Err(CatalogError::Malformed(format!(
            "packs[{index}] 的 id 不是合法目录名：{id}"
        )));
    }
    let sha256 = string_field("sha256")?;
    if sha256.len() != 64 || !sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(CatalogError::Malformed(format!(
            "packs[{index}] 的 sha256 不是 64 位十六进制"
        )));
    }
    let size_bytes = match obj.get("sizeBytes") {
        None => {
            return Err(CatalogError::Malformed(format!(
                "packs[{index}] 缺少字段 sizeBytes"
            )));
        }
        Some(v) => v.as_u64().ok_or_else(|| {
            CatalogError::Malformed(format!("packs[{index}] 的字段 sizeBytes 不是非负整数"))
        })?,
    };
    Ok(CatalogPack {
        id,
        name: string_field("name")?,
        description: string_field("description")?,
        version: string_field("version")?,
        size_bytes,
        sha256,
        url: string_field("url")?,
    })
}

/// 带超时的 HTTP 客户端（`global` 是整次操作的墙钟上限；下载大包时
/// 传 `None` 不设全局上限，只留连接超时，⛔ 不让慢网被误杀）。
fn http_agent(global: Option<Duration>) -> ureq::Agent {
    // ureq 3：`config_builder().build()` 产 `Config`，`Into` 转 `Agent`。
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(global)
        .build()
        .into()
}

/// 拉取并解析目录。环境钩子见模块头：文件钩子优先，其次地址覆盖，
/// 最后固定地址。任何传输失败都是 [`CatalogError::Unavailable`]。
pub fn fetch_catalog() -> Result<Catalog, CatalogError> {
    if let Some(file) = std::env::var_os(CATALOG_FILE_ENV)
        && !file.is_empty()
    {
        let text = fs::read_to_string(&file).map_err(|err| {
            CatalogError::Unavailable(format!(
                "读本地目录文件失败（{}）：{err}",
                file.to_string_lossy()
            ))
        })?;
        return parse_catalog(&text);
    }
    let url = std::env::var(CATALOG_URL_ENV)
        .ok()
        .filter(|raw| !raw.trim().is_empty())
        .unwrap_or_else(|| CATALOG_URL.to_owned());
    let agent = http_agent(Some(Duration::from_secs(30)));
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|err| CatalogError::Unavailable(err.to_string()))?;
    let mut text = String::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_CATALOG_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|err| CatalogError::Unavailable(format!("读目录响应失败：{err}")))?;
    if (text.len() as u64) > MAX_CATALOG_BYTES {
        return Err(CatalogError::Malformed("目录文件过大".to_owned()));
    }
    parse_catalog(&text)
}

/// 流式算一个文件的 SHA-256（十六进制小写）。
fn sha256_file(path: &Path) -> Result<String, SoundError> {
    let mut file = fs::File::open(path)
        .map_err(|err| SoundError::Io(format!("打开 {} 失败：{err}", path.display())))?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| SoundError::Io(format!("读 {} 失败：{err}", path.display())))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    // sha2 0.11 的摘要数组不实现 `LowerHex`，逐字节拼小写 hex。
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(hex)
}

/// 把一个**已在本地**的归档装进库根：大小对账 → SHA-256 对账 →
/// 临时区解包（路径安全 + 符号链接拒绝 + 总量护栏）→ 写标记 →
/// 整体改名落位。任何一步失败都不动已安装区，临时区由调用方与
/// 本函数的收尾共同清干净。
pub fn install_pack_archive(
    pack: &CatalogPack,
    archive_path: &Path,
    roots: &SoundRoots,
) -> Result<InstalledLibrary, SoundError> {
    if !is_plain_name(&pack.id) {
        return Err(SoundError::BadArchive(format!(
            "包 id 不是合法目录名：{}",
            pack.id
        )));
    }
    let actual_size = fs::metadata(archive_path)
        .map_err(|err| SoundError::Io(format!("读归档 {} 失败：{err}", archive_path.display())))?
        .len();
    if pack.size_bytes > 0 && actual_size != pack.size_bytes {
        return Err(SoundError::SizeMismatch {
            expected: pack.size_bytes,
            actual: actual_size,
        });
    }
    let actual_sha = sha256_file(archive_path)?;
    // ⭐ 空 `sha256` = 这一条**没有上游摘要**（内置音源走 GitHub 归档，GitHub 不发布
    // 归档摘要）。这一档只做结构校验，⛔ 不假装校验过：界面与状态行会明确写"未校验"。
    // 非空时仍是**硬校验**（一个字符不符就拒装，语义一位不改）。
    if !pack.sha256.trim().is_empty() && !actual_sha.eq_ignore_ascii_case(&pack.sha256) {
        return Err(SoundError::ChecksumMismatch {
            expected: pack.sha256.clone(),
            actual: actual_sha,
        });
    }
    if pack.sha256.trim().is_empty() {
        // 本仓没有 `log` 依赖（判据也不许新增依赖边），保持与既有安装腿同款：
        // 终端一行说明 + 界面状态行如实写"未校验"（不静默、也不假装校验过）。
        eprintln!(
            "yeban-app: 安装 {}：该包没有上游 SHA-256（{}），已跳过摘要校验，仅做结构校验",
            pack.id, pack.url
        );
    }
    fs::create_dir_all(&roots.library_root).map_err(|err| {
        SoundError::Io(format!(
            "建音源目录 {} 失败：{err}",
            roots.library_root.display()
        ))
    })?;
    let staging_root = roots.library_root.join(STAGING_DIR_NAME);
    let staging = staging_root.join(format!("extract-{}", pack.id));
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    fs::create_dir_all(&staging)
        .map_err(|err| SoundError::Io(format!("建临时区 {} 失败：{err}", staging.display())))?;
    let outcome = extract_and_place(pack, archive_path, &staging, roots);
    if outcome.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    outcome
}

/// [`install_pack_archive`] 的解包 + 落位段（失败时临时区由调用方清）。
fn extract_and_place(
    pack: &CatalogPack,
    archive_path: &Path,
    staging: &Path,
    roots: &SoundRoots,
) -> Result<InstalledLibrary, SoundError> {
    let file = fs::File::open(archive_path)
        .map_err(|err| SoundError::Io(format!("打开归档失败：{err}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| SoundError::BadArchive(format!("打不开 zip 归档：{err}")))?;
    let mut unpacked: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| SoundError::BadArchive(format!("读归档条目 {index} 失败：{err}")))?;
        // 路径安全：`enclosed_name` 为 `None` = 绝对路径 / `..` 逃逸，一律拒。
        let Some(relative) = entry.enclosed_name() else {
            return Err(SoundError::BadArchive(format!(
                "归档含不安全路径：{}",
                String::from_utf8_lossy(entry.name_raw())
            )));
        };
        // 符号链接条目一律拒（解出来会把后续写入带出包目录）。
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(SoundError::BadArchive(format!(
                "归档含符号链接条目：{}",
                String::from_utf8_lossy(entry.name_raw())
            )));
        }
        let dest = staging.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&dest)
                .map_err(|err| SoundError::Io(format!("建目录 {} 失败：{err}", dest.display())))?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                SoundError::Io(format!("建目录 {} 失败：{err}", parent.display()))
            })?;
        }
        let mut out = fs::File::create(&dest)
            .map_err(|err| SoundError::Io(format!("写 {} 失败：{err}", dest.display())))?;
        let copied = std::io::copy(&mut entry, &mut out)
            .map_err(|err| SoundError::Io(format!("解包 {} 失败：{err}", dest.display())))?;
        unpacked = unpacked.saturating_add(copied);
        if unpacked > MAX_UNPACKED_BYTES {
            return Err(SoundError::BadArchive(format!(
                "解包总量超过上限 {} 字节",
                MAX_UNPACKED_BYTES
            )));
        }
    }
    // 标记：扫描靠它取显示名，删除 / 重装靠它认包。
    let marker = serde_json::json!({
        "id": pack.id,
        "name": pack.name,
        "version": pack.version,
        "sha256": pack.sha256,
    });
    fs::write(staging.join(PACK_MARKER_FILE), marker.to_string())
        .map_err(|err| SoundError::Io(format!("写包标记失败：{err}")))?;
    // 落位：同名旧目录先删（重装 = 替换），然后整体改名（同盘 rename）。
    let dest = roots.library_root.join(&pack.id);
    if dest.exists() {
        fs::remove_dir_all(&dest)
            .map_err(|err| SoundError::Io(format!("替换旧目录 {} 失败：{err}", dest.display())))?;
    }
    fs::rename(staging, &dest)
        .map_err(|err| SoundError::Io(format!("安装落位 {} 失败：{err}", dest.display())))?;
    Ok(InstalledLibrary {
        id: format!("{USER_ID_PREFIX}{}", pack.id),
        name: pack.name.clone(),
        size_bytes: dir_size(&dest),
        location: dest,
        deletable: true,
    })
}

/// 下载一个包并安装：字节先落临时区 `.part`，下完改名 `.zip`，再走
/// [`install_pack_archive`] 的校验 + 解包 + 落位。任何失败都把临时
/// 文件清掉，已安装区一位不动。`progress` 在每块字节后回调
/// `(已下字节, 总字节)`（总字节取响应头，缺失时用目录登记值）。
pub fn download_pack(
    pack: &CatalogPack,
    roots: &SoundRoots,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<InstalledLibrary, SoundError> {
    fs::create_dir_all(roots.library_root.join(STAGING_DIR_NAME))
        .map_err(|err| SoundError::Io(format!("建临时区失败：{err}")))?;
    let staging_root = roots.library_root.join(STAGING_DIR_NAME);
    let part_path = staging_root.join(format!("{}.zip.part", pack.id));
    let zip_path = staging_root.join(format!("{}.zip", pack.id));
    let outcome = download_to_file(pack, &part_path, progress)
        .and_then(|()| {
            fs::rename(&part_path, &zip_path)
                .map_err(|err| SoundError::Io(format!("下载落盘改名失败：{err}")))
        })
        .and_then(|()| install_pack_archive(pack, &zip_path, roots));
    let _ = fs::remove_file(&part_path);
    let _ = fs::remove_file(&zip_path);
    outcome
}

/// [`download_pack`] 的传输段：流式写 `.part`，进度逐块回调。
fn download_to_file(
    pack: &CatalogPack,
    part_path: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), SoundError> {
    let agent = http_agent(None);
    let mut response = agent
        .get(&pack.url)
        .call()
        .map_err(|err| SoundError::Download(err.to_string()))?;
    let total = response.body().content_length().or(if pack.size_bytes > 0 {
        Some(pack.size_bytes)
    } else {
        None
    });
    let mut reader = response.body_mut().as_reader();
    let mut out = fs::File::create(part_path)
        .map_err(|err| SoundError::Io(format!("建临时文件失败：{err}")))?;
    let mut downloaded: u64 = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| SoundError::Download(format!("读下载流失败：{err}")))?;
        if read == 0 {
            break;
        }
        out.write_all(&buffer[..read])
            .map_err(|err| SoundError::Io(format!("写临时文件失败：{err}")))?;
        downloaded += read as u64;
        progress(downloaded, total);
    }
    out.flush()
        .map_err(|err| SoundError::Io(format!("写临时文件失败：{err}")))?;
    Ok(())
}

/// 删除一个用户音源库（删后由接线层重扫）。内置 id 与非法 id 都在
/// 这里挡下，⛔ 不许调用方自己拼路径删。
pub fn delete_library(roots: &SoundRoots, id: &str) -> Result<(), SoundError> {
    if id.starts_with(BUILTIN_ID_PREFIX) {
        return Err(SoundError::BuiltInNotDeletable);
    }
    let Some(dir_name) = id.strip_prefix(USER_ID_PREFIX) else {
        return Err(SoundError::NotFound(id.to_owned()));
    };
    if !is_plain_name(dir_name) {
        return Err(SoundError::NotFound(id.to_owned()));
    }
    let path = roots.library_root.join(dir_name);
    if !path.is_dir() {
        return Err(SoundError::NotFound(id.to_owned()));
    }
    fs::remove_dir_all(&path)
        .map_err(|err| SoundError::Io(format!("删除 {} 失败：{err}", path.display())))
}

/// 界面用的大小读数（`B` 原样，往上 `KiB` / `MiB` / `GiB` / `TiB`
/// 一位小数，与仓内 `9.4 GiB` 的既有写法同形态）。
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// 下载读数（**唯一**把"进度 → 人话"的地方）：`42% · 12.1 / 67.1 MiB · 3.2 MiB/s`。
///
/// 用户原话："下载看不到速度和进度 也不知道下载成功与否"。界面此前只有一根进度条 +
/// `下载中 N%`：**没有总量、没有速度**，几十 MiB 的包看起来像卡住。
///
/// 三种降级都要如实（⛔ 不编造数字）：
/// - 总量未知（服务器没给 `Content-Length`）⇒ 只报已下的字节；
/// - 速度未知（第一个采样点还没有间隔）⇒ 省掉最后一节；
/// - 进度未知（`percent < 0`）⇒ 不写百分号。
#[must_use]
pub fn format_download_readout(
    percent: i32,
    done: u64,
    total: Option<u64>,
    bytes_per_sec: Option<u64>,
) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(3);
    if percent >= 0 {
        parts.push(format!("{percent}%"));
    }
    match total {
        Some(total) if total > 0 => {
            parts.push(format!("{} / {}", format_size(done), format_size(total)))
        }
        _ => parts.push(format_size(done)),
    }
    if let Some(rate) = bytes_per_sec.filter(|rate| *rate > 0) {
        parts.push(format!("{}/s", format_size(rate)));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 一次判据一份独立根（临时目录 + 用完自清），互不污染。
    struct TestRoots {
        roots: SoundRoots,
        base: PathBuf,
    }

    impl TestRoots {
        fn new(tag: &str) -> Self {
            let serial = COUNTER.fetch_add(1, Ordering::SeqCst);
            let base = std::env::temp_dir().join(format!(
                "yeban-sound-library-test-{}-{tag}-{serial}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&base);
            let library_root = base.join("library");
            let builtin_root = base.join("builtin");
            fs::create_dir_all(&library_root).expect("建库根");
            fs::create_dir_all(&builtin_root).expect("建内置根");
            fs::write(builtin_root.join("builtin.sfz"), b"<group>").expect("写内置夹具");
            // ⭐ 内置现在**逐件列出**（`builtin/<目录>` = 一件），判据需要一个真子目录。
            // 根下那份 `builtin.sfz` 不再算"一件"（它不是目录）。
            let builtin_pack = builtin_root.join("pack-a");
            fs::create_dir_all(&builtin_pack).expect("建内置子目录");
            fs::write(builtin_pack.join("a.sfz"), b"<group>").expect("写内置子目录夹具");
            Self {
                roots: SoundRoots {
                    library_root,
                    builtin_root,
                },
                base,
            }
        }
    }

    impl Drop for TestRoots {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    /// 造一份 stored 模式的微型 zip 夹具（不依赖外网、不依赖压缩算法）。
    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(path).expect("建 zip 夹具");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(name, options).expect("写 zip 条目");
            writer.write_all(bytes).expect("写 zip 内容");
        }
        writer.finish().expect("收尾 zip 夹具");
    }

    /// 按一份已造好的 zip 的真实大小与 sha256 拼包元数据。
    fn pack_for(zip_path: &Path, id: &str) -> CatalogPack {
        CatalogPack {
            id: id.to_owned(),
            name: format!("测试包 {id}"),
            description: "判据夹具".to_owned(),
            version: "1.0.0".to_owned(),
            size_bytes: fs::metadata(zip_path).expect("读 zip 大小").len(),
            sha256: sha256_file(zip_path).expect("算 zip sha256"),
            url: "https://example.invalid/pack.zip".to_owned(),
        }
    }

    const FULL_CATALOG: &str = r#"{
        "version": 1,
        "packs": [
            {
                "id": "grand-piano",
                "name": "大钢琴",
                "description": "测试用说明",
                "version": "1.2.0",
                "sizeBytes": 12345,
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "url": "https://example.invalid/grand-piano.zip"
            }
        ]
    }"#;

    #[test]
    fn parse_catalog_accepts_full_and_empty() {
        let catalog = parse_catalog(FULL_CATALOG).expect("合法目录必须解析成功");
        assert_eq!(catalog.packs.len(), 1);
        let pack = &catalog.packs[0];
        assert_eq!(pack.id, "grand-piano");
        assert_eq!(pack.name, "大钢琴");
        assert_eq!(pack.description, "测试用说明");
        assert_eq!(pack.version, "1.2.0");
        assert_eq!(pack.size_bytes, 12345);
        assert_eq!(pack.sha256.len(), 64);
        assert_eq!(pack.url, "https://example.invalid/grand-piano.zip");
        // 空目录是真实状态：零包、零错误。
        let empty = parse_catalog(r#"{"version": 1, "packs": []}"#).expect("空目录必须解析成功");
        assert!(empty.packs.is_empty());
    }

    #[test]
    fn parse_catalog_names_the_missing_field() {
        let broken = FULL_CATALOG.replace(
            "\"sha256\": \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
            "",
        );
        match parse_catalog(&broken) {
            Err(CatalogError::Malformed(detail)) => {
                assert!(
                    detail.contains("sha256"),
                    "错误必须点名缺的字段，实得：{detail}"
                );
            }
            other => panic!("缺 sha256 必须是 Malformed，实得：{other:?}"),
        }
        match parse_catalog(r#"{"version": 1}"#) {
            Err(CatalogError::Malformed(detail)) => {
                assert!(
                    detail.contains("packs"),
                    "错误必须点名 packs，实得：{detail}"
                );
            }
            other => panic!("缺 packs 必须是 Malformed，实得：{other:?}"),
        }
    }

    #[test]
    fn parse_catalog_rejects_other_versions() {
        match parse_catalog(r#"{"version": 2, "packs": []}"#) {
            Err(CatalogError::UnsupportedVersion(2)) => {}
            other => panic!("版本 2 必须是 UnsupportedVersion(2)，实得：{other:?}"),
        }
        match parse_catalog(r#"{"packs": []}"#) {
            Err(CatalogError::Malformed(detail)) => {
                assert!(
                    detail.contains("version"),
                    "错误必须点名 version，实得：{detail}"
                );
            }
            other => panic!("缺 version 必须是 Malformed，实得：{other:?}"),
        }
    }

    #[test]
    fn parse_catalog_rejects_unsafe_id_and_bad_sha() {
        let bad_id = FULL_CATALOG.replace("\"grand-piano\"", "\"../evil\"");
        match parse_catalog(&bad_id) {
            Err(CatalogError::Malformed(detail)) => {
                assert!(detail.contains("id"), "错误必须点名 id，实得：{detail}");
            }
            other => panic!("逃逸 id 必须是 Malformed，实得：{other:?}"),
        }
        let bad_sha = FULL_CATALOG.replace(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "xyz",
        );
        match parse_catalog(&bad_sha) {
            Err(CatalogError::Malformed(detail)) => {
                assert!(
                    detail.contains("sha256"),
                    "错误必须点名 sha256，实得：{detail}"
                );
            }
            other => panic!("坏 sha256 必须是 Malformed，实得：{other:?}"),
        }
    }

    #[test]
    fn install_scan_delete_roundtrip() {
        let fixture = TestRoots::new("roundtrip");
        // 库根先删掉：安装路径必须自己把它建回来。
        fs::remove_dir_all(&fixture.roots.library_root).expect("删库根");
        let zip_path = fixture.base.join("pack.zip");
        write_zip(
            &zip_path,
            &[
                ("TestPack/instrument.sfz", b"<group> hi".as_slice()),
                ("TestPack/samples/a.wav", &[7u8; 2048]),
            ],
        );
        let pack = pack_for(&zip_path, "test-pack");
        let installed =
            install_pack_archive(&pack, &zip_path, &fixture.roots).expect("校验全对的包必须装上");
        assert_eq!(installed.id, "user:test-pack");
        assert_eq!(installed.name, "测试包 test-pack");
        assert!(installed.deletable);
        let dest = fixture.roots.library_root.join("test-pack");
        assert!(
            dest.join("TestPack/instrument.sfz").is_file(),
            "包内容必须落位"
        );
        assert!(dest.join(PACK_MARKER_FILE).is_file(), "包标记必须落位");
        assert!(
            !fixture
                .roots
                .library_root
                .join(STAGING_DIR_NAME)
                .join("extract-test-pack")
                .exists(),
            "落位后临时区不许留解包残骸"
        );
        // 扫描：内置一件（不可删）+ 本包一条（名字取标记、大小为正）。
        let listed = scan_installed(&fixture.roots);
        assert_eq!(
            listed.len(),
            2,
            "应有 1 件内置 + 本包两条，实得：{listed:?}"
        );
        assert!(
            listed[0].id.starts_with(BUILTIN_ID_PREFIX),
            "内置必须逐件列出：{listed:?}"
        );
        assert!(!listed[0].deletable);
        assert_eq!(listed[1].id, "user:test-pack");
        assert_eq!(listed[1].name, "测试包 test-pack");
        assert!(listed[1].size_bytes >= 2048, "大小必须含样本字节");
        // 删除往返：删掉后扫描只剩内置，再删一次如实报 NotFound。
        delete_library(&fixture.roots, "user:test-pack").expect("用户库必须可删");
        assert!(!dest.exists(), "删除必须真的移除目录");
        let listed = scan_installed(&fixture.roots);
        assert_eq!(listed.len(), 1);
        assert!(listed[0].id.starts_with(BUILTIN_ID_PREFIX));
        assert_eq!(
            delete_library(&fixture.roots, "user:test-pack"),
            Err(SoundError::NotFound("user:test-pack".to_owned()))
        );
    }

    /// 造一段 16-bit PCM 单声道 WAV（`frames` 个 440 Hz 正弦采样）。
    fn sine_wav(rate: u32, frames: usize) -> Vec<u8> {
        let data_bytes = u32::try_from(frames * 2).expect("夹具帧数");
        let mut out = Vec::with_capacity(44 + data_bytes as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // 单声道
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes()); // 字节率
        out.extend_from_slice(&2u16.to_le_bytes()); // 块对齐
        out.extend_from_slice(&16u16.to_le_bytes()); // 位深
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        for index in 0..frames {
            let seconds = index as f32 / rate as f32;
            let sample = (seconds * 440.0 * std::f32::consts::TAU).sin() * 0.5;
            out.extend_from_slice(&((sample * 32_767.0) as i16).to_le_bytes());
        }
        out
    }

    /// ⭐ **用户口径的硬判据**（"下载或替换后能正常发声与渲染"，不是只改界面）：
    /// 一份真的从 zip 装进库的音源（真实 `.sfz` + 真实 WAV）必须
    /// ① 解包后的样本读得出来（`missing` 为空）② 单音出声 ③ 整工程渲染出声；
    /// 删掉库之后同一条渲染必须**明确报 SFZ 错误**（⛔ 不许静默导出空带子）。
    #[test]
    fn installed_pack_is_audible_and_deleting_it_is_an_explicit_error() {
        use std::sync::Arc;

        let fixture = TestRoots::new("audible");
        let wav = sine_wav(44_100, 8_820);
        let sfz_text =
            b"<group>\n<region> sample=probe.wav pitch_keycenter=60 lokey=0 hikey=127\n";
        let zip_path = fixture.base.join("audible.zip");
        write_zip(
            &zip_path,
            &[
                ("AudiblePack/Probe.sfz", sfz_text.as_slice()),
                ("AudiblePack/probe.wav", wav.as_slice()),
            ],
        );
        let pack = pack_for(&zip_path, "audible-pack");
        let installed =
            install_pack_archive(&pack, &zip_path, &fixture.roots).expect("校验全对的包必须装上");
        let sfz_path = installed.location.join("AudiblePack/Probe.sfz");
        assert!(sfz_path.is_file(), "装好后 SFZ 必须落在库根里");

        let (report, instrument) =
            crate::sfz_load::load_sfz_with_instrument(&sfz_path, Some(60), 44_100.0)
                .expect("装好的音源必须能读");
        assert!(report.regions >= 1, "夹具必须至少有 1 个区域");
        // 样本必须来自解包目录，而不是回退正弦（回退会被登记进 `missing`）。
        let table = crate::sfz_load::build_region_samples(&instrument, &sfz_path);
        assert!(
            table.missing.is_empty(),
            "解包出来的样本必须读得出来，实得：{:?}",
            table.missing
        );
        assert_eq!(table.samples.len(), 1, "一个 sample= ⇒ 一条样本");
        let mut single = vec![0.0_f32; 22_050];
        instrument
            .render_note_into_from_samples(
                &table.samples,
                60,
                100,
                None,
                None,
                &mut single,
                44_100.0,
            )
            .expect("单音渲染必须不失败");
        let single_energy: f32 = single.iter().map(|sample| sample * sample).sum();
        assert!(
            single_energy > 1e-6,
            "装好后的音源单音必须出声，实得能量 {single_energy}"
        );

        // 整工程渲染（用户按播放 / 导出走的那条链）。
        let project = yeban_model::samples::filled_project();
        let options = crate::export_wav::WavExportOptions {
            frames: 96_000,
            ..Default::default()
        };
        let instrument = Arc::new(instrument);
        let rendered = crate::render_core::render_project_f32(
            &project,
            options,
            Some((Arc::clone(&instrument), Some(sfz_path.clone()))),
        )
        .expect("装好的音源必须能整工程渲染");
        let project_energy: f32 = rendered.0.iter().map(|sample| sample * sample).sum();
        assert!(
            project_energy > 1e-6,
            "整工程渲染必须出声（静音 = 装了等于没装），实得能量 {project_energy}"
        );

        // 删库之后：路径真的没了。⭐ 判的是**装载腿**（生产里重新打开工程 / 重新绑音源
        // 都会重新读这个 `.sfz`）：删掉之后必须**点名文件**失败，⛔ 不许当没事。
        // 注：把已装载的 `Arc<SfzInstrument>` 直接传给渲染不会重读文件——那是"进程内
        // 已经拿在手里的乐器"，不是"删库后还能用"，所以这里判装载而不是判渲染。
        delete_library(&fixture.roots, &installed.id).expect("用户库必须可删");
        assert!(!sfz_path.exists(), "删除必须真的移除目录");
        match crate::sfz_load::load_sfz_with_instrument(&sfz_path, Some(60), 44_100.0) {
            Err(crate::sfz_load::SfzLoadError::Read(reason)) => {
                assert!(
                    reason.contains("Probe.sfz"),
                    "删掉音源后的报错必须点名文件，实得：{reason}"
                );
            }
            Ok(_) => panic!("删掉音源后重新装载必须失败，⛔ 不许静默成功"),
            Err(other) => panic!("必须是 Read 错误（点名文件），实得：{other:?}"),
        }
    }

    #[test]
    fn checksum_mismatch_installs_nothing() {
        let fixture = TestRoots::new("checksum");
        let zip_path = fixture.base.join("pack.zip");
        write_zip(&zip_path, &[("a.txt", b"hello".as_slice())]);
        let mut pack = pack_for(&zip_path, "bad-pack");
        pack.sha256 = "0".repeat(64);
        match install_pack_archive(&pack, &zip_path, &fixture.roots) {
            Err(SoundError::ChecksumMismatch { expected, .. }) => {
                assert_eq!(expected, "0".repeat(64));
            }
            other => panic!("sha 不符必须是 ChecksumMismatch，实得：{other:?}"),
        }
        assert!(
            !fixture.roots.library_root.join("bad-pack").exists(),
            "校验不过不许落位"
        );
        assert_eq!(
            scan_installed(&fixture.roots).len(),
            1,
            "校验不过不许出现在已安装列表（只剩内置）"
        );
    }

    #[test]
    fn size_mismatch_installs_nothing() {
        let fixture = TestRoots::new("size");
        let zip_path = fixture.base.join("pack.zip");
        write_zip(&zip_path, &[("a.txt", b"hello".as_slice())]);
        let mut pack = pack_for(&zip_path, "small-pack");
        pack.size_bytes += 1;
        match install_pack_archive(&pack, &zip_path, &fixture.roots) {
            Err(SoundError::SizeMismatch { actual, .. }) => {
                assert_eq!(actual, pack.size_bytes - 1);
            }
            other => panic!("大小不符必须是 SizeMismatch，实得：{other:?}"),
        }
        assert!(!fixture.roots.library_root.join("small-pack").exists());
    }

    #[test]
    fn zip_slip_is_rejected_and_writes_nothing_outside() {
        let fixture = TestRoots::new("slip");
        let zip_path = fixture.base.join("slip.zip");
        write_zip(&zip_path, &[("../evil.txt", b"pwned".as_slice())]);
        let pack = pack_for(&zip_path, "slip-pack");
        match install_pack_archive(&pack, &zip_path, &fixture.roots) {
            Err(SoundError::BadArchive(detail)) => {
                assert!(detail.contains("不安全路径"), "实得：{detail}");
            }
            other => panic!("路径逃逸必须是 BadArchive，实得：{other:?}"),
        }
        assert!(
            !fixture.base.join("evil.txt").exists(),
            "逃逸文件绝不许写到库根之外"
        );
        assert!(!fixture.roots.library_root.join("slip-pack").exists());
    }

    #[test]
    fn scan_skips_hidden_dirs_and_prefers_marker_name() {
        let fixture = TestRoots::new("scan");
        let plain = fixture.roots.library_root.join("legacy-instrument");
        fs::create_dir_all(&plain).expect("建旧乐器目录");
        fs::write(plain.join("x.sfz"), b"<group>").expect("写旧乐器文件");
        let hidden = fixture.roots.library_root.join(STAGING_DIR_NAME);
        fs::create_dir_all(&hidden).expect("建临时区");
        fs::write(hidden.join("junk.zip.part"), b"junk").expect("写临时残件");
        let listed = scan_installed(&fixture.roots);
        assert_eq!(listed.len(), 2, "隐藏目录必须跳过，实得：{listed:?}");
        assert_eq!(listed[1].id, "user:legacy-instrument");
        assert_eq!(listed[1].name, "legacy-instrument", "无标记时按目录名显示");
        // 有标记时取标记里的名字。
        fs::write(
            plain.join(PACK_MARKER_FILE),
            r#"{"id":"legacy-instrument","name":"标记里的名字","version":"2.0","sha256":"x"}"#,
        )
        .expect("写标记");
        let listed = scan_installed(&fixture.roots);
        assert_eq!(listed[1].name, "标记里的名字");
    }

    #[test]
    fn delete_rejects_builtin_and_bad_ids() {
        let fixture = TestRoots::new("delete");
        assert_eq!(
            delete_library(&fixture.roots, "builtin:pack-a"),
            Err(SoundError::BuiltInNotDeletable)
        );
        assert_eq!(
            delete_library(&fixture.roots, "user:../builtin"),
            Err(SoundError::NotFound("user:../builtin".to_owned()))
        );
        assert_eq!(
            delete_library(&fixture.roots, "nonsense"),
            Err(SoundError::NotFound("nonsense".to_owned()))
        );
        assert!(
            fixture.roots.builtin_root.join("builtin.sfz").is_file(),
            "内置内容一位不许动"
        );
    }

    #[test]
    fn format_size_reads() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(9_844_170_377), "9.2 GiB");
    }
}

#[cfg(test)]
mod readout_tests {
    use super::format_download_readout;

    #[test]
    fn the_readout_never_invents_numbers() {
        // 全信息：百分比 · 已下/总量 · 速度
        assert_eq!(
            format_download_readout(42, 12_700_000, Some(70_000_000), Some(3_400_000)),
            "42% · 12.1 MiB / 66.8 MiB · 3.2 MiB/s"
        );
        // 总量未知 ⇒ 只报已下
        assert_eq!(
            format_download_readout(42, 2048, None, None),
            "42% · 2.0 KiB"
        );
        // 速度未知（第一个采样点）⇒ 省掉最后一节
        assert_eq!(
            format_download_readout(7, 1_000_000, Some(20_000_000), None),
            "7% · 976.6 KiB / 19.1 MiB"
        );
        // 进度未知 ⇒ 不写百分号；速度为 0 ⇒ 不写"0 B/s"
        assert_eq!(format_download_readout(-1, 0, None, Some(0)), "0 B");
    }
}
