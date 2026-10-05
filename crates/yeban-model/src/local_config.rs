//! `LocalMachineConfig` —— **本机配置层**（`MODEL-ISO-001` 第 3 层）。
//!
//! 规范来源：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §2 `[MODEL-ISO-001]`：
//!
//! > **`LocalMachineConfig`（本机配置层）**：本机声卡物理端口绑定、外部编辑器绝对路径、
//! > 云端 API Token。敏感凭据存入系统安全密钥链（OS Keychain），工程内仅存引用指针。
//!
//! ## 三层里最敏感的一层，落盘**必须在工程之外**
//!
//! 本层与 [`crate::project::YebanProjectV1`] 的区别不是"存哪儿方便"，而是**谁能看到它**：
//!
//! | 事实 | 本机配置 | 工程文档 |
//! | :--- | :--- | :--- |
//! | 落盘位置 | `~/.yeban/config.json`（[`LocalMachineConfig::default_path`]） | `.yeban` 容器 |
//! | 是否随工程分发 | **否**（绝不进 Commit、绝不进容器） | 是（这是作品本体） |
//! | 文件权限 | **`0600`**（[`LOCAL_CONFIG_FILE_MODE`]） | 由容器/仓库策略决定 |
//! | 是否含凭据 | 只含**引用指针** [`SecretRef`] | 一格都不许有 |
//!
//! 判据 `crates/yeban-model/tests/model_isolation.rs` 逐个断言：
//! 容器 `project.json` 的递归键集合里**没有**本模块的任何字段名、
//! 容器字节里**没有**配置里的引用名、容器往返之后配置仍在 `~/.yeban/…` 那一侧。
//!
//! ## 凭据边界：**类型上拿不到密钥 material**
//!
//! [`SecretRef`] 是一个**只装条目名**的字符串（`newtype`，字段私有，反序列化时校验）。
//! 它**没有**任何字段能承载密钥本体；密钥本体只存在于 [`SecretMaterial`] 里，而
//! `SecretMaterial`：
//!
//! - **没有** `Serialize` / `Deserialize`（`compile_fail` doc-test 常驻，见该类型文档）；
//! - `Debug` 输出被打码（[`SecretMaterial`] 的 `Debug` 实现只打印长度）；
//! - `Drop` 时对缓冲区做**best-effort 清零**（安全代码，无 `unsafe`）。
//!
//! 真实 OS Keychain 后端需要**新的外部依赖**（`security-framework` / `keyring` /
//! `windows` 等）⇒ 那要人类 / 集成者裁决，本线**不**引入。因此这里只给
//! [`SecretStore`] trait + 一个**默认的"不可用"实现** [`UnavailableSecretStore`]：
//! 它**永远返回明确错误**，绝不静默成功、绝不 panic。
//! `secret_store_for` 对**未知**后端名返回 [`SecretStoreError::UnknownBackend`]。
//!
//! ## 确定性
//!
//! 三个集合全部 `BTreeMap`（红线 4 / `MODEL-AST-003`）⇒ 同一配置两次序列化逐字节相同
//! （判据 ⑥）。`Vec` 只用在 [`ExternalEditor::arguments`]：参数**顺序本身是语义**，
//! 用集合表达反而是错的。
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// 本机配置目录名（相对用户主目录）。
pub const LOCAL_CONFIG_DIR_NAME: &str = ".yeban";

/// 本机配置文件名。
pub const LOCAL_CONFIG_FILE_NAME: &str = "config.json";

/// 本机配置的 schema 版本（独立于工程的 `SCHEMA_VERSION`）。
pub const LOCAL_CONFIG_VERSION: u32 = 1;

/// 本机配置文件权限（`0600`：仅属主可读写）—— **Unix 语义**。
#[cfg(unix)]
pub const LOCAL_CONFIG_FILE_MODE: u32 = 0o600;

/// 本机配置目录权限（`0700`：仅属主可进入）—— **Unix 语义**。
#[cfg(unix)]
pub const LOCAL_CONFIG_DIR_MODE: u32 = 0o700;

/// 密钥引用名长度上限（字节）。
pub const MAX_SECRET_REF_LEN: usize = 128;

/// 默认密钥后端名（= **不可用**：真实集成需要新外部依赖）。
pub const DEFAULT_SECRET_BACKEND: &str = "unavailable";

/// 已登记（"名字认识"）的密钥后端。
///
/// 认名字不等于**实现**：除 `unavailable` 外，其余后端当前一律返回
/// [`SecretStoreError::BackendUnavailable`]（明确错误），因为把真实密钥链接进来需要
/// 新外部依赖 —— 那属于人类 / 集成者裁决。
pub const KNOWN_SECRET_BACKENDS: [&str; 5] = [
    "unavailable",
    "os-keychain",
    "macos-keychain",
    "windows-credential-manager",
    "secret-service",
];

/// 本机配置层的错误类型。
///
/// 全部变体都**只**描述"为什么拒绝"，不带平台相关的模糊措辞：同一条坏配置在三个平台上
/// 必须得到同一个错误（判据能跨机复现的前提，同 `ContainerError` 的理由）。
#[derive(Debug, Error)]
pub enum LocalConfigError {
    // ------------------------------------------------------------------
    // 密钥引用名
    // ------------------------------------------------------------------
    /// 密钥引用名为空。
    #[error("secret reference name is empty")]
    EmptySecretRef,
    /// 密钥引用名超长。
    #[error("secret reference name is {len} bytes, over the {max}-byte limit")]
    SecretRefTooLong {
        /// 实际长度（字节）。
        len: usize,
        /// 上限（字节）。
        max: usize,
    },
    /// 密钥引用名含禁止字符（控制字符 / 空白）。
    ///
    /// 关键是**拒绝空白与控制字符**：它们让"条目名"变成可以夹带任意文本的通道
    /// （`MODEL-ISO-001` 要求这里只放**指针**，不是自由文本）。
    #[error("secret reference name contains a forbidden character {ch:?}")]
    SecretRefForbiddenChar {
        /// 被拒绝的字符。
        ch: char,
    },
    /// 标识符（设备名 / 端口名 / 服务商名）为空。
    #[error("{field} identifier is empty")]
    EmptyIdentifier {
        /// 字段名（`input_device` / `output_device` / `provider` …）。
        field: &'static str,
    },
    /// 标识符含控制字符（会把"标识"变成可以夹带任意文本的通道）。
    #[error("{field} identifier contains a forbidden control character {ch:?}")]
    IdentifierForbiddenChar {
        /// 字段名。
        field: &'static str,
        /// 被拒绝的字符。
        ch: char,
    },
    // ------------------------------------------------------------------
    // 外部编辑器
    // ------------------------------------------------------------------
    /// 外部编辑器路径不是绝对路径。
    #[error("external editor path `{path}` is not absolute")]
    EditorPathNotAbsolute {
        /// 被拒绝的路径。
        path: PathBuf,
    },
    // ------------------------------------------------------------------
    // 版本 / IO / JSON
    // ------------------------------------------------------------------
    /// 配置文件版本超出本读取器支持范围。
    #[error("local config version {found} is newer than the supported {supported}")]
    UnsupportedConfigVersion {
        /// 文件里的版本。
        found: u32,
        /// 本读取器支持的版本。
        supported: u32,
    },
    /// 主目录未知（`HOME` / `USERPROFILE` 都未设置）。
    #[error("cannot locate the home directory: neither HOME nor USERPROFILE is set")]
    HomeUnknown,
    /// 路径没有父目录（无法建目录 / 无法判断权限）。
    #[error("path `{path}` has no parent directory")]
    NoParentDirectory {
        /// 被拒绝的路径。
        path: PathBuf,
    },
    /// 文件系统错误。
    #[error("io error at `{path}`: {source}")]
    Io {
        /// 出错路径。
        path: PathBuf,
        /// 底层错误。
        #[source]
        source: std::io::Error,
    },
    /// JSON 序列化 / 反序列化错误。
    #[error("local config json error: {detail}")]
    Json {
        /// serde 的错误文本。
        detail: String,
    },
}

/// **云端 Token 的引用指针**（`MODEL-ISO-001` 第 3 层）。
///
/// 本类型只装**密钥链条目名**，例如 `yeban/cloud/anthropic`。它**不是**密钥本体，
/// 也没有任何字段能承载密钥本体 —— 这是类型层面的保证，不是约定。
///
/// 序列化形式是**一个 JSON 字符串**（不是对象）：即使日后有人想加字段，也得先改
/// `Serialize` 实现并让键集合判据变红。
///
/// ```
/// use yeban_model::local_config::SecretRef;
/// let r = SecretRef::new("yeban/cloud/anthropic").expect("合法条目名");
/// assert_eq!(serde_json::to_string(&r).unwrap(), "\"yeban/cloud/anthropic\"");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretRef(String);

impl SecretRef {
    /// 校验并构造一个引用指针。
    ///
    /// # Errors
    ///
    /// - 空串 ⇒ [`LocalConfigError::EmptySecretRef`]；
    /// - 超过 [`MAX_SECRET_REF_LEN`] 字节 ⇒ [`LocalConfigError::SecretRefTooLong`]；
    /// - 含控制字符或空白 ⇒ [`LocalConfigError::SecretRefForbiddenChar`]。
    pub fn new(entry: impl Into<String>) -> Result<Self, LocalConfigError> {
        let entry = entry.into();
        if entry.is_empty() {
            return Err(LocalConfigError::EmptySecretRef);
        }
        if entry.len() > MAX_SECRET_REF_LEN {
            return Err(LocalConfigError::SecretRefTooLong {
                len: entry.len(),
                max: MAX_SECRET_REF_LEN,
            });
        }
        if let Some(ch) = entry.chars().find(|c| c.is_control() || c.is_whitespace()) {
            return Err(LocalConfigError::SecretRefForbiddenChar { ch });
        }
        Ok(Self(entry))
    }

    /// 条目名（**可以**安全地写进本机配置文件；它只是指针）。
    #[must_use]
    pub fn entry(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for SecretRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SecretRef {
    /// 反序列化**也**走 [`SecretRef::new`] 的校验：坏条目名在读取时就被拒绝，
    /// 而不是等到使用时才炸。
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

/// **密钥 material**：只存在于内存，**永不**落盘（`MODEL-ISO-001` 第 3 层）。
///
/// 类型层面禁止持久化 —— 没有 `Serialize` / `Deserialize`：
///
/// ```compile_fail
/// // [MODEL-ISO-001 第 3 层]：密钥本体没有 Serialize ⇒ 这一行必须编译失败。
/// fn assert_serializable<T: serde::Serialize>() {}
/// assert_serializable::<yeban_model::local_config::SecretMaterial>();
/// ```
///
/// 对照（**必须编译成功**）证明上面那条不是探针写错了：
///
/// ```
/// fn assert_serializable<T: serde::Serialize>() {}
/// assert_serializable::<yeban_model::local_config::SecretRef>();
/// ```
pub struct SecretMaterial {
    /// 密钥字节。除 [`SecretMaterial::expose`] 外没有任何出口。
    bytes: Vec<u8>,
}

impl SecretMaterial {
    /// 从原始字节构造。
    #[must_use]
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    /// 从 UTF-8 文本构造（例如 `Bearer` token）。
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self::from_bytes(text.as_bytes().to_vec())
    }

    /// 借出密钥字节 —— 名字刻意叫 `expose`：每次调用都应该是一次**审计点**。
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.bytes
    }

    /// 密钥长度（字节）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// 是否为空密钥。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl fmt::Debug for SecretMaterial {
    /// **打码**：`Debug` 是最容易被顺手打进日志 / 断言消息的通道，所以它不能泄密。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretMaterial(<redacted {} bytes>)", self.bytes.len())
    }
}

impl Drop for SecretMaterial {
    /// best-effort 清零（安全代码，无 `unsafe`）：降低密钥在释放后仍留在堆页里的窗口。
    ///
    /// 诚实边界：这**不是**密码学意义上的擦除保证（编译器优化、swap、换页都可能留下副本）。
    /// 真正的保证需要 `zeroize` 之类依赖 —— 那要人类裁决，本线不引入。
    fn drop(&mut self) {
        // `slice::fill` 是安全代码里最直接的整段覆写；诚实边界见方法文档。
        self.bytes.fill(0);
    }
}

/// 密钥链后端错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecretStoreError {
    /// 后端名**不认识**。
    #[error("unknown secret backend `{requested}` (known backends: {known})")]
    UnknownBackend {
        /// 调用方请求的后端名。
        requested: String,
        /// 已登记的后端名（逗号分隔）。
        known: String,
    },
    /// 后端**认识但没接进来**（含默认的 `unavailable`）。
    #[error("secret backend `{backend}` is unavailable: {detail}")]
    BackendUnavailable {
        /// 后端名。
        backend: &'static str,
        /// 人话原因。
        detail: &'static str,
    },
    /// 条目不存在。
    #[error("secret entry `{entry}` not found in backend `{backend}`")]
    NotFound {
        /// 后端名。
        backend: &'static str,
        /// 条目名。
        entry: String,
    },
}

/// OS 安全密钥链的**抽象边界**（`MODEL-ISO-001` 第 3 层）。
///
/// 真实实现（macOS Keychain / Windows Credential Manager / Secret Service）需要新的
/// 外部依赖 ⇒ 留给后续线 + 人类裁决。本线只固定**边界**与**默认的不可用实现**。
///
/// 刻意不加 `Send + Sync` 约束：那会替后续线做一个它可能不想要的裁决
/// （例如后端句柄是否需要跨线程），登记在 notes 的 needs 里。
pub trait SecretStore {
    /// 后端名（用于错误信息与审计）。
    fn backend_name(&self) -> &'static str;

    /// 写入 / 覆盖一个条目。
    ///
    /// # Errors
    ///
    /// 后端不可用或写入失败时返回 [`SecretStoreError`]。
    fn put(&self, reference: &SecretRef, material: &SecretMaterial)
    -> Result<(), SecretStoreError>;

    /// 读取一个条目。
    ///
    /// # Errors
    ///
    /// 后端不可用或条目不存在时返回 [`SecretStoreError`]。
    fn get(&self, reference: &SecretRef) -> Result<SecretMaterial, SecretStoreError>;

    /// 删除一个条目。
    ///
    /// # Errors
    ///
    /// 后端不可用或条目不存在时返回 [`SecretStoreError`]。
    fn delete(&self, reference: &SecretRef) -> Result<(), SecretStoreError>;
}

/// **默认的"不可用"后端**：每个方法都返回明确错误。
///
/// 它的存在本身就是一条防线：在没有真实后端时，"写入密钥"**不可能**静默成功，
/// 于是也**不可能**有人误以为密钥已经被安全保存了。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnavailableSecretStore {
    /// 它替哪个后端名"占位"（`unavailable` / `os-keychain` …）。
    backend: &'static str,
    /// 不可用的原因（人话）。
    detail: &'static str,
}

impl Default for UnavailableSecretStore {
    fn default() -> Self {
        Self {
            backend: DEFAULT_SECRET_BACKEND,
            detail: "real OS keychain integration requires a new external dependency; \
                     see needs N1 in docs/ledger/model-session-state-notes.md",
        }
    }
}

impl UnavailableSecretStore {
    /// 某个**已登记但未接入**的后端占位实现。
    #[must_use]
    pub fn for_known_backend(backend: &'static str) -> Self {
        Self {
            backend,
            detail: "backend name is registered but not linked in yet: binding a real keychain \
                     requires a new external dependency (human decision required)",
        }
    }

    /// 它占位的后端名。
    #[must_use]
    pub fn backend(&self) -> &'static str {
        self.backend
    }

    /// 不可用的原因。
    #[must_use]
    pub fn detail(&self) -> &'static str {
        self.detail
    }

    /// 统一的"不可用"错误。
    fn unavailable(&self) -> SecretStoreError {
        SecretStoreError::BackendUnavailable {
            backend: self.backend,
            detail: self.detail,
        }
    }
}

impl SecretStore for UnavailableSecretStore {
    fn backend_name(&self) -> &'static str {
        self.backend
    }

    fn put(
        &self,
        _reference: &SecretRef,
        _material: &SecretMaterial,
    ) -> Result<(), SecretStoreError> {
        Err(self.unavailable())
    }

    fn get(&self, _reference: &SecretRef) -> Result<SecretMaterial, SecretStoreError> {
        Err(self.unavailable())
    }

    fn delete(&self, _reference: &SecretRef) -> Result<(), SecretStoreError> {
        Err(self.unavailable())
    }
}

/// 按名字取一个密钥链后端。
///
/// # Errors
///
/// 后端名不在 [`KNOWN_SECRET_BACKENDS`] 里 ⇒ [`SecretStoreError::UnknownBackend`]。
/// **未知后端绝不静默降级**成默认后端：那会让"配置写错了"看起来像"密钥存好了"。
pub fn secret_store_for(backend: &str) -> Result<Box<dyn SecretStore>, SecretStoreError> {
    if !KNOWN_SECRET_BACKENDS.contains(&backend) {
        return Err(SecretStoreError::UnknownBackend {
            requested: backend.to_owned(),
            known: KNOWN_SECRET_BACKENDS.join(", "),
        });
    }
    if backend == DEFAULT_SECRET_BACKEND {
        return Ok(Box::new(UnavailableSecretStore::default()));
    }
    // 已登记但未接入：返回**明确错误**的后端，而不是 panic / 静默成功。
    let placeholder: &'static str = KNOWN_SECRET_BACKENDS
        .iter()
        .copied()
        .find(|known| *known == backend)
        .unwrap_or(DEFAULT_SECRET_BACKEND);
    Ok(Box::new(UnavailableSecretStore::for_known_backend(
        placeholder,
    )))
}

/// 外部编辑器的角色（本机绑定：用哪个程序打开哪一类目标）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorRole {
    /// 波形编辑器。
    Waveform,
    /// 乐谱编辑器。
    Score,
    /// 采样切片 / 素材编辑器。
    Sample,
}

/// 一个外部编辑器的本机绑定。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalEditor {
    /// **绝对**路径（相对路径会被拒绝：它取决于启动目录，不是本机事实）。
    pub absolute_path: PathBuf,
    /// 启动参数（顺序是语义 ⇒ `Vec`）。
    #[serde(default)]
    pub arguments: Vec<String>,
}

impl ExternalEditor {
    /// 校验路径是绝对路径。
    ///
    /// # Errors
    ///
    /// 路径非绝对 ⇒ [`LocalConfigError::EditorPathNotAbsolute`]。
    pub fn validate(&self) -> Result<(), LocalConfigError> {
        if !self.absolute_path.is_absolute() {
            return Err(LocalConfigError::EditorPathNotAbsolute {
                path: self.absolute_path.clone(),
            });
        }
        Ok(())
    }
}

/// 本机声卡的物理端口绑定（`MODEL-ISO-001` 第 3 层）。
///
/// "物理端口"是**本机事实**：同一份工程在另一台机器上插口编号完全不同，
/// 因此这一层永远不能进工程文件。端口用 `BTreeMap<u32, String>` 表达：
/// 键是端口序号（确定性顺序），值是端口标识串。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioPortBinding {
    /// 输入设备标识（`None` = 未绑定，语义上等于"用系统默认"）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_device: Option<String>,
    /// 输出设备标识。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
    /// 输入物理端口（序号 → 端口标识）。
    #[serde(default)]
    pub input_ports: BTreeMap<u32, String>,
    /// 输出物理端口（序号 → 端口标识）。
    #[serde(default)]
    pub output_ports: BTreeMap<u32, String>,
}

impl AudioPortBinding {
    /// 校验端口绑定自洽性。
    ///
    /// # Errors
    ///
    /// 设备名 / 端口名为空 ⇒ [`LocalConfigError::EmptyIdentifier`]。
    pub fn validate(&self) -> Result<(), LocalConfigError> {
        validate_optional_identifier("input_device", self.input_device.as_deref())?;
        validate_optional_identifier("output_device", self.output_device.as_deref())?;
        for port in self.input_ports.values() {
            validate_identifier("input_port", port)?;
        }
        for port in self.output_ports.values() {
            validate_identifier("output_port", port)?;
        }
        Ok(())
    }

    /// 是否一个端口都没绑定。
    #[must_use]
    pub fn is_unbound(&self) -> bool {
        self.input_device.is_none()
            && self.output_device.is_none()
            && self.input_ports.is_empty()
            && self.output_ports.is_empty()
    }
}

/// **第 3 层：本机配置**（`MODEL-ISO-001`）。
///
/// 落盘位置在**工程之外**（[`LocalMachineConfig::default_path`] ⇒ `~/.yeban/config.json`），
/// 权限 `0600`（[`LocalMachineConfig::save_to`]）。凭据只以 [`SecretRef`] 指针形式出现。
///
/// `#[serde(deny_unknown_fields)]` 是刻意的：本文件由本实现独占写入，
/// 多出未知字段只可能意味着"被手改"或"来自未来版本" ⇒ **拒绝**而不是静默丢弃。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalMachineConfig {
    /// 本机配置 schema 版本（[`LOCAL_CONFIG_VERSION`]）。
    pub version: u32,
    /// 声卡物理端口绑定。
    pub audio_binding: AudioPortBinding,
    /// 外部编辑器角色 → 绝对路径绑定。
    pub external_editors: BTreeMap<EditorRole, ExternalEditor>,
    /// 云端服务商名 → **密钥链引用指针**（绝不含密钥本体）。
    pub cloud_tokens: BTreeMap<String, SecretRef>,
}

impl Default for LocalMachineConfig {
    /// 空配置（未绑定任何设备 / 编辑器 / 凭据）。
    fn default() -> Self {
        Self {
            version: LOCAL_CONFIG_VERSION,
            audio_binding: AudioPortBinding::default(),
            external_editors: BTreeMap::new(),
            cloud_tokens: BTreeMap::new(),
        }
    }
}

impl LocalMachineConfig {
    /// 某个主目录下的配置文件路径（`<home>/.yeban/config.json`）。
    #[must_use]
    pub fn path_for_home(home: &Path) -> PathBuf {
        home.join(LOCAL_CONFIG_DIR_NAME)
            .join(LOCAL_CONFIG_FILE_NAME)
    }

    /// 本机默认配置文件路径。
    ///
    /// Unix 用 `HOME`，Windows 用 `USERPROFILE`；两者都没有 ⇒
    /// [`LocalConfigError::HomeUnknown`]（**不**回退到当前目录 —— 那会把本机配置
    /// 悄悄写进工程目录，正是 `MODEL-ISO-001` 禁止的形态）。
    ///
    /// # Errors
    ///
    /// 主目录未知 ⇒ [`LocalConfigError::HomeUnknown`]。
    pub fn default_path() -> Result<PathBuf, LocalConfigError> {
        home_dir()
            .map(|home| Self::path_for_home(&home))
            .ok_or(LocalConfigError::HomeUnknown)
    }

    /// 结构校验（读取与写入**都**走这一步）。
    ///
    /// # Errors
    ///
    /// - 版本超出 [`LOCAL_CONFIG_VERSION`] ⇒ [`LocalConfigError::UnsupportedConfigVersion`]；
    /// - 声卡绑定非法 ⇒ [`LocalConfigError::EmptyIdentifier`]；
    /// - 外部编辑器路径非绝对 ⇒ [`LocalConfigError::EditorPathNotAbsolute`]；
    /// - 服务商名为空 ⇒ [`LocalConfigError::EmptyIdentifier`]。
    pub fn validate(&self) -> Result<(), LocalConfigError> {
        if self.version > LOCAL_CONFIG_VERSION {
            return Err(LocalConfigError::UnsupportedConfigVersion {
                found: self.version,
                supported: LOCAL_CONFIG_VERSION,
            });
        }
        self.audio_binding.validate()?;
        for editor in self.external_editors.values() {
            editor.validate()?;
        }
        for provider in self.cloud_tokens.keys() {
            validate_identifier("provider", provider)?;
        }
        Ok(())
    }

    /// 确定性 JSON 字节（校验通过才序列化）。
    ///
    /// # Errors
    ///
    /// 校验失败或 serde 失败时返回 [`LocalConfigError`]。
    pub fn to_json(&self) -> Result<Vec<u8>, LocalConfigError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| LocalConfigError::Json {
            detail: error.to_string(),
        })
    }

    /// 从 JSON 字节读取（反序列化后**必须**通过校验）。
    ///
    /// # Errors
    ///
    /// serde 失败或校验失败时返回 [`LocalConfigError`]。
    pub fn from_json(bytes: &[u8]) -> Result<Self, LocalConfigError> {
        let config: Self =
            serde_json::from_slice(bytes).map_err(|error| LocalConfigError::Json {
                detail: error.to_string(),
            })?;
        config.validate()?;
        Ok(config)
    }

    /// 把配置写到 `path`，权限 `0600`（Unix）。
    ///
    /// 目录用 `0700` 创建。创建文件时就用 `0600`（**不是**先建再 chmod：那会留下
    /// 一个权限过宽的时间窗），并且**额外**显式 `set_permissions(0600)` ——
    /// `OpenOptions::mode` 只对**新建**生效，对已存在（可能过宽）的文件是空转。
    ///
    /// # Errors
    ///
    /// 路径无父目录、配置非法、或 IO 失败时返回 [`LocalConfigError`]。
    pub fn save_to(&self, path: &Path) -> Result<(), LocalConfigError> {
        let bytes = self.to_json()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| LocalConfigError::NoParentDirectory {
                path: path.to_path_buf(),
            })?;
        create_private_dir(parent, path)?;

        #[cfg(unix)]
        {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;

            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(LOCAL_CONFIG_FILE_MODE)
                .open(path)
                .map_err(|source| LocalConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
            file.write_all(&bytes)
                .map_err(|source| LocalConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
            file.sync_all().map_err(|source| LocalConfigError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            drop(file);
            set_file_mode(path, LOCAL_CONFIG_FILE_MODE)?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(path, &bytes).map_err(|source| LocalConfigError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        }
        Ok(())
    }

    /// 从 `path` 读取配置（**不**检查 / 不修改权限：读取路径上的静默 chmod
    /// 会掩盖"这份文件曾被别人读过"的事实，权限由 [`Self::save_to`] 与判据负责）。
    ///
    /// # Errors
    ///
    /// IO、serde 或校验失败时返回 [`LocalConfigError`]。
    pub fn load_from(path: &Path) -> Result<Self, LocalConfigError> {
        let bytes = std::fs::read(path).map_err(|source| LocalConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_json(&bytes)
    }
}

/// 读取本机默认配置文件。
///
/// # Errors
///
/// 主目录未知、IO、serde 或校验失败时返回 [`LocalConfigError`]。
pub fn load_default() -> Result<LocalMachineConfig, LocalConfigError> {
    LocalMachineConfig::load_from(&LocalMachineConfig::default_path()?)
}

/// 主目录探测：Unix 看 `HOME`，Windows 看 `USERPROFILE`。
fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
    }
}

/// 建一个私有目录（Unix `0700`）。
fn create_private_dir(dir: &Path, path: &Path) -> Result<(), LocalConfigError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;

        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(LOCAL_CONFIG_DIR_MODE);
        builder.create(dir).map_err(|source| LocalConfigError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir).map_err(|source| LocalConfigError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// 显式收紧文件权限（Unix）。
#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> Result<(), LocalConfigError> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|source| {
        LocalConfigError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// 非空校验（`None` 合法）。
fn validate_optional_identifier(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), LocalConfigError> {
    if let Some(value) = value {
        validate_identifier(field, value)?;
    }
    Ok(())
}

/// 非空 + 无控制字符校验。
fn validate_identifier(field: &'static str, value: &str) -> Result<(), LocalConfigError> {
    if value.is_empty() {
        return Err(LocalConfigError::EmptyIdentifier { field });
    }
    if let Some(ch) = value.chars().find(|c| c.is_control()) {
        return Err(LocalConfigError::IdentifierForbiddenChar { field, ch });
    }
    Ok(())
}
