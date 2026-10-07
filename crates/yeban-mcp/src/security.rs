//! MCP 安全模型：Bearer Token、POSIX `0600` 令牌文件、六级权限作用域 [ARCH-SEC-002, MUST-GATE-009]。
//!
//! 本模块只做**判定**，不做 I/O 之外的事，且判定全部是**纯函数 + 可断言判据**：
//!
//! | 红线 | 落点 |
//! | :--- | :--- |
//! | 网络监听默认关闭 | 不在本模块；见 [`crate::transport`] 与 `Cargo.toml` 的 `default = []` |
//! | 只允许绑 `127.0.0.1` | 不在本模块；见 [`crate::transport::http::assert_loopback`] |
//! | 256-bit 高熵 Token | [`BearerToken::generate`] + [`TokenFile`] |
//! | `~/.yeban/session.token` 权限 `0600` | [`TokenFile::save`] / [`TokenFile::load`] |
//! | 请求必须带 `Authorization: Bearer <TOKEN>` | [`authenticate`] / [`AuthContext::from_headers`] |
//! | 六级 scope + `ui:inject` 生产硬禁 | [`Scope`] / [`ScopeSet::grants`] / [`authorize`] |
//!
//! ## 为什么 token 与 scope 判定要在同一个纯函数里
//!
//! "缺 token 就放行"、"scope 不足就当没看见"、"生产环境把 `ui:inject` 放过去"
//! 这三类事故有一个共同形状：**判定散落在多个地方**，于是改一处漏一处。
//! 因此本模块把"这一条请求能不能做这件事"收敛成唯一入口：
//!
//! ```text
//! authenticate() -> authorize() -> ScopeSet::grants()
//! ```
//!
//! 传输层（stdio / HTTP）只能通过 [`AuthContext`] 表达"我看到的凭据是什么"，
//! **不能自己决定通过与否**。[`crate::dispatch::Dispatcher`] 也一样。
//!
//! ## 零新增依赖
//!
//! 高熵来源优先读 Unix 的 `/dev/urandom`（纯 `std::fs`）；读不到时退回一个
//! **确定性**的 splitmix64 混合器（`SystemTime` 纳秒 + PID + 栈地址 + 原子计数器）。
//! 两者都不需要 `rand` / `getrandom`（`docs/adr/ADR-0001` D20/D21 的零新增依赖倾向）。
//! 非 Unix 平台上 [`TokenFile::save`] / [`TokenFile::load`] 一律返回
//! [`SecurityError::UnsupportedPlatform`] —— **明确拒绝，而不是静默放过**。

use std::collections::BTreeSet;
use std::fmt;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::jsonrpc::{FORBIDDEN, UNAUTHORIZED};

/// 令牌的原始熵字节数（256 bit）[ARCH-SEC-002]。
pub const TOKEN_BYTES: usize = 32;

/// 令牌的规范文本长度（32 字节 = 64 位小写十六进制）。
pub const TOKEN_HEX_LEN: usize = 64;

/// `Authorization` 头的方案名（RFC 6750；比较时大小写不敏感）。
pub const BEARER_SCHEME: &str = "Bearer";

/// 令牌文件所在目录名（`~` 之下）。
pub const TOKEN_DIR_NAME: &str = ".yeban";

/// 令牌文件名。
pub const TOKEN_FILE_NAME: &str = "session.token";

/// 令牌文件的强制权限位（POSIX `0600`）[ARCH-SEC-002]。
pub const TOKEN_FILE_MODE: u32 = 0o600;

/// 令牌目录的强制权限位（POSIX `0700`）。
pub const TOKEN_DIR_MODE: u32 = 0o700;

/// Unix 上的操作系统熵源路径。
pub const OS_ENTROPY_PATH: &str = "/dev/urandom";

/// 原子落盘用的临时文件**中缀**（`session.token.tmp-{pid}-{nonce}`）。
///
/// 与 `[ARCH-SEC-004]` 的 `.yeban.tmp-{ulid}` 同形：临时文件与目标**同目录**，
/// 因此 `rename` 是同目录内的原子替换，不可能 `EXDEV`。
#[cfg(unix)]
const TOKEN_TEMP_INFIX: &str = ".tmp-";

/// 进程内临时文件序号（保证同一 PID 的多次 `save` 不会撞名）。
#[cfg(unix)]
static TOKEN_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 安全模型相关的错误。
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    /// 令牌文本不是 64 位十六进制。
    #[error("令牌格式非法: 期望 {TOKEN_HEX_LEN} 位十六进制 (256-bit), 实际 {actual} 位")]
    MalformedToken {
        /// 实际长度。
        actual: usize,
    },
    /// 令牌文件存在，但内容**不是**一个可用的令牌（空文件 / 只有空白）。
    ///
    /// 这是"崩溃安全"的安全侧：`save` 的 `open(truncate)` 一返回，磁盘上就是 0 字节，
    /// 而 `exists()` 已经是真。把空文件当成"已有令牌"会静默换掉令牌。
    ///
    /// **fail closed**：拒绝，并且**不**替调用方删文件 —— 删掉一个可能正被另一个进程
    /// 使用的令牌文件必须由人（或运维脚本）显式决定。
    #[error(
        "令牌文件 `{path}` 存在但不是可用令牌 ({reason}): 拒绝使用; \
         确认没有进程正在使用该令牌后删除它, 下次启动会重新生成"
    )]
    MalformedTokenFile {
        /// 文件路径。
        path: String,
        /// 具体原因（空文件 / 只有空白 / 长度不对 / 含非十六进制字符）。
        reason: String,
    },
    /// 令牌文件权限不是 `0600`。
    #[error("令牌文件权限不安全: `{path}` 是 {mode:o}, 必须恰好是 {expected:o}")]
    InsecureTokenPermissions {
        /// 文件路径。
        path: String,
        /// 实际权限位。
        mode: u32,
        /// 期望权限位。
        expected: u32,
    },
    /// 令牌路径是符号链接（拒绝跟随，避免被替换成别的文件）。
    #[error("令牌路径 `{path}` 是符号链接: 拒绝读写")]
    TokenPathIsSymlink {
        /// 文件路径。
        path: String,
    },
    /// 非 Unix 平台无法校验 POSIX 权限位。
    #[error("非 Unix 平台不支持 POSIX 0600 权限校验: 明确拒绝而不是静默放过")]
    UnsupportedPlatform,
    /// 找不到家目录。
    #[error("无法定位家目录 (HOME 未设置)")]
    HomeDirectoryUnavailable,
    /// 令牌文件 I/O 失败。
    #[error("令牌文件 I/O 失败 (`{path}`): {source}")]
    Io {
        /// 文件路径。
        path: String,
        /// 底层错误。
        source: std::io::Error,
    },
    /// 无法从操作系统熵源取字节。
    #[error("无法从操作系统熵源读取: {0}")]
    EntropyUnavailable(String),
    /// 未知权限作用域。
    #[error("未知权限作用域 `{name}`; 合法值: {expected}")]
    UnknownScope {
        /// 传入的名字。
        name: String,
        /// 合法值清单。
        expected: String,
    },
}

/// 六级权限作用域 [ARCH-SEC-002]。
///
/// 枚举顺序**就是**规范里的顺序，`Ord` 因此与规范同序：`BTreeSet<Scope>` 的迭代
/// 顺序跨进程稳定（`MODEL-AST-003` 的确定性精神，红线 4）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scope {
    /// `ui:read`：只读审查控件树几何尺寸与文本。
    UiRead,
    /// `ui:screenshot`：读取离屏光栅化 Framebuffer PNG 截图。
    UiScreenshot,
    /// `ui:inject`：模拟键盘鼠标事件注入 —— **生产环境硬编码封禁**。
    UiInject,
    /// `app:save`：触发工程落盘。
    AppSave,
    /// `app:reload-engine`：触发引擎快照重新加载。
    AppReloadEngine,
    /// `app:admin`：全量乐理意图操作、音符增删、分支合并与离线母带渲染。
    AppAdmin,
}

impl Scope {
    /// 六个作用域，规范顺序。
    pub const ALL: [Self; 6] = [
        Self::UiRead,
        Self::UiScreenshot,
        Self::UiInject,
        Self::AppSave,
        Self::AppReloadEngine,
        Self::AppAdmin,
    ];

    /// 规范字符串（`ui:read` 等）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UiRead => "ui:read",
            Self::UiScreenshot => "ui:screenshot",
            Self::UiInject => "ui:inject",
            Self::AppSave => "app:save",
            Self::AppReloadEngine => "app:reload-engine",
            Self::AppAdmin => "app:admin",
        }
    }

    /// 解析规范字符串（`ui:read` 等）。大小写敏感：scope 是安全边界，不做模糊匹配。
    ///
    /// # Errors
    ///
    /// 传入的名字不在六级作用域内。
    pub fn parse(name: &str) -> Result<Self, SecurityError> {
        Self::ALL
            .into_iter()
            .find(|scope| scope.as_str() == name)
            .ok_or_else(|| SecurityError::UnknownScope {
                name: name.to_owned(),
                expected: Self::ALL
                    .iter()
                    .map(|scope| scope.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            })
    }

    /// `true` 表示该作用域在**生产模式**下必须被硬拒绝 [ARCH-SEC-002]。
    ///
    /// 目前只有 `ui:inject`。它是纯函数：没有环境变量、没有编译期特性参与判断。
    #[must_use]
    pub const fn is_production_forbidden(self) -> bool {
        matches!(self, Self::UiInject)
    }

    /// `true` 表示这是"本机界面能力"作用域（`ui:*`）。
    ///
    /// `app:admin` **不**隐含 `ui:*` —— 领域 admin 与界面控制是两条独立的授权线。
    #[must_use]
    pub const fn is_ui(self) -> bool {
        matches!(self, Self::UiRead | Self::UiScreenshot | Self::UiInject)
    }

    /// `true` 表示这是领域意图 API 的作用域（`app:*`）。
    #[must_use]
    pub const fn is_app(self) -> bool {
        !self.is_ui()
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一组已授予的作用域。
///
/// 内部是 `BTreeSet`：迭代顺序确定（红线 4 的精神），错误信息里列出的 scope 顺序
/// 因此逐字节稳定，判据可以对着字符串断言。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeSet(BTreeSet<Scope>);

impl ScopeSet {
    /// 空集合（什么都不授权）。
    #[must_use]
    pub fn empty() -> Self {
        Self(BTreeSet::new())
    }

    /// 全部六级作用域。
    #[must_use]
    pub fn all() -> Self {
        Self(Scope::ALL.into_iter().collect())
    }

    /// 由若干作用域构造。
    pub fn from_scopes(scopes: impl IntoIterator<Item = Scope>) -> Self {
        Self(scopes.into_iter().collect())
    }

    /// 解析逗号分隔的作用域清单（`"ui:read,app:save"`）。
    ///
    /// 空白项被忽略；空清单得到空集合。
    ///
    /// # Errors
    ///
    /// 任一项不是合法作用域。
    pub fn parse_list(text: &str) -> Result<Self, SecurityError> {
        let mut set = BTreeSet::new();
        for item in text.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            set.insert(Scope::parse(item)?);
        }
        Ok(Self(set))
    }

    /// 是否包含该作用域（精确包含）。
    #[must_use]
    pub fn contains(&self, scope: Scope) -> bool {
        self.0.contains(&scope)
    }

    /// **授权判定**：这组作用域是否足以执行需要 `required` 的操作。
    ///
    /// 规则（纯函数，三条）：
    ///
    /// 1. 精确包含 `required` ⇒ 通过；
    /// 2. `required` 是 `ui:*` ⇒ 只有精确包含才通过。**`app:admin` 不隐含界面控制**，
    ///    尤其不隐含 `ui:inject`（否则"硬封禁"就变成一句空话）；
    /// 3. `required` 是 `app:*` ⇒ `app:admin` 是超集（规范定义它是"全量"领域权限）。
    #[must_use]
    pub fn grants(&self, required: Scope) -> bool {
        if self.contains(required) {
            return true;
        }
        if required.is_ui() {
            return false;
        }
        self.contains(Scope::AppAdmin)
    }

    /// 作用域个数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 按规范顺序迭代。
    pub fn iter(&self) -> impl Iterator<Item = Scope> + '_ {
        self.0.iter().copied()
    }

    /// 规范字符串，逗号分隔、规范顺序。
    #[must_use]
    pub fn to_spec_string(&self) -> String {
        self.iter().map(Scope::as_str).collect::<Vec<_>>().join(",")
    }
}

/// 运行模式 [ARCH-SEC-002]：生产模式是**默认**，测试模式必须被显式打开。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunMode {
    /// 生产模式（默认）：`ui:inject` 硬禁。
    #[default]
    Production,
    /// 测试模式：仅允许 `ui:inject`，必须由显式开关打开。
    Test,
}

impl RunMode {
    /// 由显式 `--test-mode` / `YEBAN_MCP_TEST_MODE` 开关得到运行模式。
    #[must_use]
    pub const fn from_test_flag(flag: bool) -> Self {
        if flag { Self::Test } else { Self::Production }
    }

    /// 规范字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Test => "test",
        }
    }

    /// 是否是生产模式。
    #[must_use]
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

/// 请求到达本进程的通道。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// 标准输入输出（形态 B；进程边界即能力边界）。
    Stdio,
    /// 环回 HTTP（形态 A）。
    Http,
}

impl Channel {
    /// 规范字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }
}

/// 请求携带的凭据。
///
/// 这是传输层唯一能"表达"的东西 —— 它不能表达"通过"或"不通过"。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Credential<'a> {
    /// 完全没有 `Authorization` 头（HTTP 上就是这个形状）。
    Missing,
    /// 有 `Authorization` 头，但不是 `Bearer <token>`。
    Malformed(&'a str),
    /// 合法的 `Bearer <token>`，token 文本待比对。
    Bearer(&'a str),
    /// 同进程 stdio：没有 HTTP 头，凭据是"我就在这个进程里"。
    LocalProcess,
}

impl<'a> Credential<'a> {
    /// 严格解析 `Authorization` 头的值。
    ///
    /// 只接受"恰好两个 token，第一个大小写不敏感等于 `Bearer`"的形状。
    /// `Basic ...` / 空串 / `Bearer` 单独出现 / `Bearer  a b` 一律 `Malformed`。
    #[must_use]
    pub fn parse_header(value: &'a str) -> Self {
        let parts: Vec<&str> = value.split_whitespace().collect();
        match parts.as_slice() {
            [scheme, token] if scheme.eq_ignore_ascii_case(BEARER_SCHEME) => Self::Bearer(token),
            _ => Self::Malformed(value),
        }
    }

    /// 凭据种类名（用于错误对象 `data`）。
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Malformed(_) => "malformed",
            Self::Bearer(_) => "bearer",
            Self::LocalProcess => "local-process",
        }
    }
}

/// 一次请求的完整鉴权上下文。
#[derive(Clone, Debug)]
pub struct AuthContext<'a> {
    /// 通道。
    pub channel: Channel,
    /// 凭据。
    pub credential: Credential<'a>,
    /// 调用方被授予的作用域。
    pub granted: ScopeSet,
    /// 运行模式。
    pub mode: RunMode,
}

impl<'a> AuthContext<'a> {
    /// 由 `Authorization` 头的原始值构造（传输层唯一入口）。
    ///
    /// stdio 上"没有头"被解释为 [`Credential::LocalProcess`]；HTTP 上"没有头"
    /// 就是 [`Credential::Missing`]，会被 [`authenticate`] 直接拒掉。
    #[must_use]
    pub fn from_headers(
        channel: Channel,
        authorization: Option<&'a str>,
        granted: ScopeSet,
        mode: RunMode,
    ) -> Self {
        let credential = match authorization {
            Some(value) => Credential::parse_header(value),
            None => match channel {
                Channel::Stdio => Credential::LocalProcess,
                Channel::Http => Credential::Missing,
            },
        };
        Self {
            channel,
            credential,
            granted,
            mode,
        }
    }

    /// HTTP 形态。
    #[must_use]
    pub fn http(authorization: Option<&'a str>, granted: ScopeSet, mode: RunMode) -> Self {
        Self::from_headers(Channel::Http, authorization, granted, mode)
    }

    /// stdio 形态。
    #[must_use]
    pub fn stdio(authorization: Option<&'a str>, granted: ScopeSet, mode: RunMode) -> Self {
        Self::from_headers(Channel::Stdio, authorization, granted, mode)
    }
}

/// 拒绝理由。每一个都能映射到明确的 HTTP 状态码与 JSON-RPC 错误码。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denial {
    /// 缺少 `Authorization` 头。
    MissingToken,
    /// `Authorization` 头不是合法 Bearer 形状。
    MalformedAuthorization,
    /// token 不匹配。
    InvalidToken,
    /// 该通道不接受这种凭据（例如 HTTP 上声称"我就是本进程"）。
    CredentialNotAcceptedOnChannel {
        /// 通道。
        channel: Channel,
        /// 凭据种类。
        credential: &'static str,
    },
    /// 作用域不足。
    InsufficientScope {
        /// 需要的作用域。
        required: Scope,
        /// 实际授予的作用域（规范顺序）。
        granted: ScopeSet,
    },
    /// 该作用域在生产模式下被硬禁（`ui:inject`）。
    ForbiddenInProduction {
        /// 被禁的作用域。
        scope: Scope,
        /// 实际运行模式。
        mode: RunMode,
    },
}

impl Denial {
    /// 稳定的机器可读种类名（进 JSON-RPC 错误对象的 `data.kind`）。
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::MissingToken => "missing-token",
            Self::MalformedAuthorization => "malformed-authorization",
            Self::InvalidToken => "invalid-token",
            Self::CredentialNotAcceptedOnChannel { .. } => "credential-not-accepted",
            Self::InsufficientScope { .. } => "insufficient-scope",
            Self::ForbiddenInProduction { .. } => "forbidden-in-production",
        }
    }

    /// HTTP 状态码：**401 全部集中在"凭据"上，403 全部集中在"授权"上**。
    ///
    /// 缺失与错误 token 都返回 401 且不回退放开 —— 见 [`authenticate`]。
    #[must_use]
    pub const fn http_status(&self) -> u16 {
        match self {
            Self::MissingToken
            | Self::MalformedAuthorization
            | Self::InvalidToken
            | Self::CredentialNotAcceptedOnChannel { .. } => 401,
            Self::InsufficientScope { .. } | Self::ForbiddenInProduction { .. } => 403,
        }
    }

    /// JSON-RPC 错误码。
    #[must_use]
    pub const fn rpc_code(&self) -> i64 {
        if self.http_status() == 401 {
            UNAUTHORIZED
        } else {
            FORBIDDEN
        }
    }

    /// 人话信息。
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::MissingToken => {
                "缺少 Authorization 头: 必须是 `Authorization: Bearer <TOKEN>`".to_owned()
            }
            Self::MalformedAuthorization => {
                "Authorization 头形状非法: 只接受 `Bearer <TOKEN>`".to_owned()
            }
            Self::InvalidToken => "Bearer Token 不匹配".to_owned(),
            Self::CredentialNotAcceptedOnChannel {
                channel,
                credential,
            } => format!(
                "通道 `{}` 不接受凭据 `{credential}`: HTTP 形态必须携带 Bearer Token",
                channel.as_str()
            ),
            Self::InsufficientScope { required, granted } => format!(
                "作用域不足: 需要 `{required}`, 实际授予 [{}]",
                granted.to_spec_string()
            ),
            Self::ForbiddenInProduction { scope, mode } => format!(
                "作用域 `{scope}` 在 `{}` 模式下被硬禁 (仅测试模式可开)",
                mode.as_str()
            ),
        }
    }

    /// 错误对象的 `data` 载荷。
    #[must_use]
    pub fn data(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("kind".to_owned(), self.kind().into());
        map.insert("httpStatus".to_owned(), self.http_status().into());
        match self {
            Self::InsufficientScope { required, granted } => {
                map.insert("requiredScope".to_owned(), required.as_str().into());
                map.insert("grantedScopes".to_owned(), granted.to_spec_string().into());
            }
            Self::ForbiddenInProduction { scope, mode } => {
                map.insert("scope".to_owned(), scope.as_str().into());
                map.insert("mode".to_owned(), mode.as_str().into());
            }
            Self::CredentialNotAcceptedOnChannel {
                channel,
                credential,
            } => {
                map.insert("channel".to_owned(), channel.as_str().into());
                map.insert("credential".to_owned(), (*credential).into());
            }
            Self::MissingToken | Self::MalformedAuthorization | Self::InvalidToken => {
                map.insert(
                    "hint".to_owned(),
                    "服务器不会在缺少/错误 token 时回退放开".into(),
                );
            }
        }
        serde_json::Value::Object(map)
    }
}

impl fmt::Display for Denial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

/// 令牌的熵来源（用于让"高熵"这件事可核验，而不是一句口头保证）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntropySource {
    /// 操作系统熵源（Unix 上是 `/dev/urandom`）。
    OsRandom,
    /// 确定性回退：splitmix64 混合 `SystemTime` 纳秒 + PID + 栈地址 + 原子计数器。
    DeterministicFallback,
}

impl EntropySource {
    /// 规范字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OsRandom => "os-random",
            Self::DeterministicFallback => "deterministic-fallback",
        }
    }
}

/// 一次令牌生成的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedToken {
    /// 生成的令牌。
    pub token: BearerToken,
    /// 熵来源。
    pub source: EntropySource,
}

/// 256-bit Bearer Token。
///
/// 规范文本形式是 **64 位小写十六进制**。`Debug` 被刻意脱敏 —— 令牌绝不能进日志。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BearerToken(String);

impl BearerToken {
    /// 生成一个新的 256-bit 令牌 [ARCH-SEC-002]。
    ///
    /// 优先读操作系统熵源；失败时退回确定性混合器（来源如实写在返回值里，
    /// 不假装它是 OS 熵）。
    #[must_use]
    pub fn generate() -> GeneratedToken {
        let mut bytes = [0_u8; TOKEN_BYTES];
        let source = match fill_os_random(&mut bytes) {
            Ok(()) => EntropySource::OsRandom,
            Err(_) => {
                fill_deterministic(&mut bytes);
                EntropySource::DeterministicFallback
            }
        };
        GeneratedToken {
            token: Self(bytes_to_hex(&bytes)),
            source,
        }
    }

    /// 由规范文本解析令牌（接受大小写十六进制，规范化成小写）。
    ///
    /// # Errors
    ///
    /// 长度不是 [`TOKEN_HEX_LEN`]，或含非十六进制字符。
    pub fn parse(text: &str) -> Result<Self, SecurityError> {
        let text = text.trim();
        if text.len() != TOKEN_HEX_LEN {
            return Err(SecurityError::MalformedToken { actual: text.len() });
        }
        if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(SecurityError::MalformedToken { actual: text.len() });
        }
        Ok(Self(text.to_ascii_lowercase()))
    }

    /// 规范文本（64 位小写十六进制）。
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// **常量时间**比较（不因前缀相同而提前返回）。
    #[must_use]
    pub fn ct_eq(&self, candidate: &str) -> bool {
        let expected = self.0.as_bytes();
        let candidate = candidate.as_bytes();
        if expected.len() != candidate.len() {
            return false;
        }
        let mut diff = 0_u8;
        for (left, right) in expected.iter().zip(candidate.iter()) {
            diff |= left ^ right;
        }
        diff == 0
    }
}

impl fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 脱敏: 只暴露长度, 绝不暴露内容。
        write!(f, "BearerToken(<redacted {} hex chars>)", self.0.len())
    }
}

/// 凭据校验：**唯一的 token 判定入口**。
///
/// 规则（缺一不可）：
///
/// - 缺少 `Authorization` ⇒ [`Denial::MissingToken`]（401），**不回退放开**；
/// - 形状非法 ⇒ [`Denial::MalformedAuthorization`]（401）；
/// - HTTP 通道上的 `LocalProcess` 凭据 ⇒ [`Denial::CredentialNotAcceptedOnChannel`]（401）；
/// - 常量时间比对失败 ⇒ [`Denial::InvalidToken`]（401）。
///
/// # Errors
///
/// 上述任一条成立。
pub fn authenticate(
    expected: &BearerToken,
    credential: Credential<'_>,
    channel: Channel,
) -> Result<(), Denial> {
    match credential {
        Credential::Missing => Err(Denial::MissingToken),
        Credential::Malformed(_) => Err(Denial::MalformedAuthorization),
        Credential::LocalProcess => {
            if channel == Channel::Http {
                Err(Denial::CredentialNotAcceptedOnChannel {
                    channel,
                    credential: credential.kind(),
                })
            } else {
                Ok(())
            }
        }
        Credential::Bearer(candidate) => {
            if expected.ct_eq(candidate) {
                Ok(())
            } else {
                Err(Denial::InvalidToken)
            }
        }
    }
}

/// **唯一授权入口**：先硬禁判定，再 token，再 scope。
///
/// 顺序是刻意的：`ui:inject` 的生产硬禁**不依赖 token 是否正确**，
/// 因此"顺手拿个合法 token 去注入事件"在生产模式下也一定被拒。
///
/// # Errors
///
/// 生产模式下的 `ui:inject`、凭据不合法、作用域不足。
pub fn authorize(
    expected: &BearerToken,
    context: &AuthContext<'_>,
    required: Scope,
) -> Result<(), Denial> {
    if required.is_production_forbidden() && context.mode.is_production() {
        return Err(Denial::ForbiddenInProduction {
            scope: required,
            mode: context.mode,
        });
    }
    authenticate(expected, context.credential, context.channel)?;
    if context.granted.grants(required) {
        Ok(())
    } else {
        Err(Denial::InsufficientScope {
            required,
            granted: context.granted.clone(),
        })
    }
}

/// `~/.yeban/session.token` 的读写（POSIX `0600`）[ARCH-SEC-002]。
#[derive(Clone, Debug)]
pub struct TokenFile {
    path: PathBuf,
}

impl TokenFile {
    /// 指定路径。
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 规范默认路径 `~/.yeban/session.token`。
    ///
    /// # Errors
    ///
    /// `HOME` 未设置。
    pub fn default_path() -> Result<PathBuf, SecurityError> {
        let home = std::env::var_os("HOME").ok_or(SecurityError::HomeDirectoryUnavailable)?;
        Ok(PathBuf::from(home)
            .join(TOKEN_DIR_NAME)
            .join(TOKEN_FILE_NAME))
    }

    /// 规范默认位置的令牌文件。
    ///
    /// # Errors
    ///
    /// `HOME` 未设置。
    pub fn at_default_path() -> Result<Self, SecurityError> {
        Ok(Self::new(Self::default_path()?))
    }

    /// 文件路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 写入令牌，权限强制 `0600`（目录 `0700`）。
    ///
    /// ## 原子落盘（`[ARCH-SEC-004]` 的三阶段，本轮修复）
    ///
    /// 1. 同目录临时文件 `session.token.tmp-{pid}-{nonce}`，**创建时**就是 `0600`
    ///    （`OpenOptions::mode` + `create_new`，不是"先建宽权限再 `chmod`"）；
    /// 2. 写完 `token` + `\n` 之后 `sync_all()`（`fsync`），数据真正进非易失介质；
    /// 3. 同目录 `rename` 覆盖目标 —— 目标要么是**旧的完整文件**，要么是**新的完整文件**，
    ///    不存在"0 字节"或"半截"的中间态。
    ///
    /// 最后把**目录项**也 `fsync`（`[ARCH-SEC-004]` 第 3.5 步）。
    ///
    /// 格式**不变**：`64 位小写十六进制 + "\n"`（既有部署的旧令牌原样可读）。
    ///
    /// 已存在且权限宽松的目标会被**整份替换**成 `0600` 的新文件（不是原地 `chmod`）。
    ///
    /// ## 并发语义（多进程）
    ///
    /// 临时文件名含 PID 与进程内序号，因此两个进程**不会**撞名（`create_new` 只会
    /// 因自己重名而失败）。两个进程同时 `save` 是"后写者赢"，最后一个 `rename`
    /// 决定磁盘内容 —— 与修复前的"后写者赢（原地覆盖）"语义相同，但**任何一个进程
    /// 的任何时刻**读到的都是完整令牌，不会再读到半截文件。
    ///
    /// # Errors
    ///
    /// 非 Unix 平台返回 [`SecurityError::UnsupportedPlatform`]；I/O 失败。
    /// 失败路径会清掉本次的临时文件，**目标文件保持逐字节不变**。
    #[cfg(unix)]
    pub fn save(&self, token: &BearerToken) -> Result<(), SecurityError> {
        use std::os::unix::fs::PermissionsExt as _;

        let path_text = self.path.display().to_string();
        let io = |source: std::io::Error| SecurityError::Io {
            path: path_text.clone(),
            source,
        };
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        fs::create_dir_all(&parent).map_err(io)?;
        fs::set_permissions(&parent, fs::Permissions::from_mode(TOKEN_DIR_MODE)).map_err(io)?;

        // 第 1 步: 同目录临时文件。`create_new` + `mode(0600)` ⇒ 文件**一出生**就是 0600。
        let temp = parent.join(format!(
            "{TOKEN_FILE_NAME}{TOKEN_TEMP_INFIX}{}-{:x}",
            std::process::id(),
            TOKEN_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        if let Err(source) = self.write_temp_then_replace(&temp, token) {
            let _ = fs::remove_file(&temp);
            return Err(SecurityError::Io {
                path: path_text,
                source,
            });
        }

        // 第 3.5 步: 目录项也要落盘, 否则"文件在了"这件事可能还只在页缓存里。
        match fs::File::open(&parent) {
            Ok(handle) => handle.sync_all().map_err(io)?,
            Err(source) => return Err(io(source)),
        }
        Ok(())
    }

    /// 第 1、2、3 步的本体：写临时文件 → `sync_all` → 同目录 `rename`。
    ///
    /// 返回 `std::io::Result` 而不是 [`SecurityError`]：错误路径由调用方统一带上路径
    /// （临时文件路径是诊断信息，目标路径才是运维要看的路径）。
    #[cfg(unix)]
    fn write_temp_then_replace(&self, temp: &Path, token: &BearerToken) -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt as _;

        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(TOKEN_FILE_MODE)
                .open(temp)?;
            file.write_all(token.expose().as_bytes())?;
            file.write_all(b"\n")?;
            // `flush` 只把 `BufWriter` 之类的用户态缓冲交给内核；裸 `File` 本来就无缓冲。
            // 真正保证"数据进非易失介质"的是 `sync_all`（`fsync`）。
            file.sync_all()?;
        }
        fs::rename(temp, &self.path)
    }

    /// 非 Unix 平台：明确拒绝，不静默放过。
    ///
    /// # Errors
    ///
    /// 永远返回 [`SecurityError::UnsupportedPlatform`]。
    #[cfg(not(unix))]
    pub fn save(&self, _token: &BearerToken) -> Result<(), SecurityError> {
        Err(SecurityError::UnsupportedPlatform)
    }

    /// 读取令牌，并**校验**权限位恰好是 `0600`。
    ///
    /// 内容校验是**全部**而非只看长度：空文件、只有空白、长度不对、含非十六进制字符
    /// 一律明确报错（空文件走 [`SecurityError::MalformedTokenFile`]，其余走
    /// [`SecurityError::MalformedToken`]）。**没有**"读不出来就当没有"的分支 ——
    /// 静默重新生成会换掉一个可能正被另一个进程使用的令牌。
    ///
    /// # Errors
    ///
    /// 非 Unix 平台、路径是符号链接、权限不是 `0600`、内容不是 64 位十六进制。
    #[cfg(unix)]
    pub fn load(&self) -> Result<BearerToken, SecurityError> {
        use std::os::unix::fs::PermissionsExt as _;
        let path_text = self.path.display().to_string();
        let io = |source: std::io::Error| SecurityError::Io {
            path: path_text.clone(),
            source,
        };
        let link_meta = fs::symlink_metadata(&self.path).map_err(io)?;
        if link_meta.file_type().is_symlink() {
            return Err(SecurityError::TokenPathIsSymlink { path: path_text });
        }
        let mode = fs::metadata(&self.path).map_err(io)?.permissions().mode() & 0o777;
        if mode != TOKEN_FILE_MODE {
            return Err(SecurityError::InsecureTokenPermissions {
                path: path_text,
                mode,
                expected: TOKEN_FILE_MODE,
            });
        }
        let text = fs::read_to_string(&self.path).map_err(io)?;
        // `read_to_string` 已经保证是 UTF-8。空白文件**单独命名**，这样运维一眼能分辨
        // "崩溃留空文件"与"文件被人塞了垃圾"。
        if text.trim().is_empty() {
            return Err(SecurityError::MalformedTokenFile {
                path: path_text,
                reason: if text.is_empty() {
                    String::from("空文件")
                } else {
                    String::from("只有空白字符")
                },
            });
        }
        BearerToken::parse(&text)
    }

    /// 非 Unix 平台：明确拒绝，不静默放过。
    ///
    /// # Errors
    ///
    /// 永远返回 [`SecurityError::UnsupportedPlatform`]。
    #[cfg(not(unix))]
    pub fn load(&self) -> Result<BearerToken, SecurityError> {
        Err(SecurityError::UnsupportedPlatform)
    }

    /// 读取已有令牌；不存在则生成并落盘。
    ///
    /// 返回 `(令牌, 是否新建)`。
    ///
    /// ## 失败语义（本轮修复）
    ///
    /// `exists()` 只用来区分"**路径上什么都没有**"与"**路径上有东西**"。
    /// - 没有东西 ⇒ 生成并**原子**落盘（[`TokenFile::save`]）；
    /// - 有东西但不可用 ⇒ **拒绝启动**（权限不安全 / 空文件 / 半截文件 / 非法内容），
    ///   并把路径写进错误信息。
    ///
    /// **为什么是拒绝而不是重新生成**：这个文件可能正被另一个进程（`yeban-app`
    /// 的 `--enable-ui-mcp-http`，或另一个 `yeban-mcp --enable-mcp-http`）用来鉴权。
    /// 静默换掉令牌会让那个进程的已授权客户端全部开始 401。拒绝是可恢复的：
    /// 人确认没有进程在用之后删掉文件，下次启动就重新生成（判据 D 钉住这条路）。
    /// 这条语义与既有的"权限不安全就拒绝读"完全同向，只是把校验面从权限位扩到内容。
    ///
    /// # Errors
    ///
    /// 同 [`TokenFile::load`] / [`TokenFile::save`]。
    pub fn load_or_create(&self) -> Result<(BearerToken, bool), SecurityError> {
        if self.path.exists() {
            return Ok((self.load()?, false));
        }
        let generated = BearerToken::generate();
        self.save(&generated.token)?;
        Ok((generated.token, true))
    }
}

/// 填充操作系统熵源（Unix）。
///
/// # Errors
///
/// 打不开 / 读不满 [`OS_ENTROPY_PATH`]。
#[cfg(unix)]
pub fn fill_os_random(buf: &mut [u8]) -> Result<(), SecurityError> {
    let mut file = fs::File::open(OS_ENTROPY_PATH)
        .map_err(|error| SecurityError::EntropyUnavailable(error.to_string()))?;
    file.read_exact(buf)
        .map_err(|error| SecurityError::EntropyUnavailable(error.to_string()))
}

/// 非 Unix 平台没有统一的熵源路径。
///
/// # Errors
///
/// 永远返回 [`SecurityError::UnsupportedPlatform`]。
#[cfg(not(unix))]
pub fn fill_os_random(_buf: &mut [u8]) -> Result<(), SecurityError> {
    Err(SecurityError::UnsupportedPlatform)
}

/// 进程内单调计数器（保证同一纳秒内的多次生成长度不同的输出）。
static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 确定性回退熵源：splitmix64 混合"时间 + PID + 栈地址 + 计数器"。
///
/// 这不是密码学强度的 OS 熵，因此 [`EntropySource`] 会如实标注它。
/// 用它的唯一理由是：`/dev/urandom` 不可用时**也必须**产出不重复的令牌，
/// 而不是退回一个常量。
fn fill_deterministic(buf: &mut [u8]) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |delta| delta.as_nanos() as u64);
    let pid = u64::from(std::process::id());
    let stack = &nanos as *const u64 as usize as u64;
    let counter = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut state = nanos
        ^ pid.rotate_left(17)
        ^ stack.rotate_left(31)
        ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for chunk in buf.chunks_mut(8) {
        let word = splitmix64(&mut state).to_le_bytes();
        let len = chunk.len();
        chunk.copy_from_slice(&word[..len]);
    }
}

/// splitmix64（`yeban-model` / `yeban-theory` 用的同一个混合器，见 ADR-0001 D5 的说明）。
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 小写十六进制编码。
fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    // `std::fs` 只在 `#[cfg(unix)]` 的令牌文件测试里用到(权限位 / 落盘)。
    // **必须有 cfg**: 否则 Windows 上这条 import 是未使用的, 而 CI 用 `-D warnings` 跑 clippy
    // ⇒ Windows 侧编译失败(实测: run 37236922758 的 windows job, `unused import: std::fs`)。
    // 这类"只在某个平台用得上的 import"只有真在另一个平台编译才会暴露。
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    /// 每个测试独立的临时令牌目录（不碰真实的 `~/.yeban`）。
    #[cfg(unix)]
    fn temp_token_path(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yeban-mcp-token-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建立临时目录");
        dir.join(TOKEN_FILE_NAME)
    }

    fn fixture_token(byte: u8) -> BearerToken {
        BearerToken::parse(&bytes_to_hex(&[byte; TOKEN_BYTES])).expect("夹具令牌")
    }

    #[test]
    fn generated_token_is_256_bit_hex() {
        let generated = BearerToken::generate();
        assert_eq!(generated.token.expose().len(), TOKEN_HEX_LEN);
        assert!(
            generated
                .token
                .expose()
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
            "令牌必须是十六进制"
        );
        assert!(
            generated
                .token
                .expose()
                .bytes()
                .all(|b| !b.is_ascii_uppercase()),
            "规范文本必须是小写十六进制"
        );
        #[cfg(unix)]
        assert_eq!(
            generated.source,
            EntropySource::OsRandom,
            "Unix 上必须走操作系统熵源"
        );
    }

    #[test]
    fn generated_tokens_are_distinct_and_not_all_zero() {
        let mut seen = BTreeSet::new();
        for _ in 0..128 {
            let token = BearerToken::generate().token;
            assert_ne!(token.expose(), "0".repeat(TOKEN_HEX_LEN), "熵源退化成常量");
            assert!(seen.insert(token.expose().to_owned()), "令牌重复");
        }
    }

    #[test]
    fn deterministic_fallback_also_produces_distinct_tokens() {
        let mut seen = BTreeSet::new();
        for _ in 0..64 {
            let mut bytes = [0_u8; TOKEN_BYTES];
            fill_deterministic(&mut bytes);
            assert!(seen.insert(bytes), "回退熵源重复");
        }
    }

    #[test]
    fn token_parse_rejects_wrong_length_and_non_hex() {
        assert!(matches!(
            BearerToken::parse("abc"),
            Err(SecurityError::MalformedToken { actual: 3 })
        ));
        let not_hex = "z".repeat(TOKEN_HEX_LEN);
        assert!(BearerToken::parse(&not_hex).is_err());
        // 大小写混写被规范化成小写。
        let upper = "A".repeat(TOKEN_HEX_LEN);
        assert_eq!(
            BearerToken::parse(&upper).expect("大写可解析").expose(),
            "a".repeat(TOKEN_HEX_LEN)
        );
    }

    #[test]
    fn debug_format_never_leaks_the_token() {
        let token = fixture_token(0xAB);
        let rendered = format!("{token:?}");
        assert!(!rendered.contains("ab"), "Debug 泄漏了令牌内容: {rendered}");
        assert!(rendered.contains("redacted"), "Debug 必须明示脱敏");
    }

    #[test]
    fn constant_time_compare_matches_and_rejects() {
        let token = fixture_token(0x5A);
        assert!(token.ct_eq(token.expose()));
        assert!(!token.ct_eq(&"5".repeat(TOKEN_HEX_LEN)));
        assert!(!token.ct_eq("short"));
    }

    #[test]
    fn scope_round_trip_covers_six_scopes() {
        assert_eq!(Scope::ALL.len(), 6, "六级权限分层");
        for scope in Scope::ALL {
            assert_eq!(Scope::parse(scope.as_str()).expect("解析"), scope);
            assert_eq!(scope.to_string(), scope.as_str());
        }
        let text = ScopeSet::all().to_spec_string();
        assert_eq!(
            text, "ui:read,ui:screenshot,ui:inject,app:save,app:reload-engine,app:admin",
            "规范顺序"
        );
    }

    #[test]
    fn unknown_scope_is_rejected() {
        assert!(matches!(
            Scope::parse("ui:write"),
            Err(SecurityError::UnknownScope { .. })
        ));
        assert!(ScopeSet::parse_list("ui:read,ui:write").is_err());
        assert!(ScopeSet::parse_list("").expect("空清单").is_empty());
    }

    #[test]
    fn app_admin_never_implies_ui_scopes() {
        let admin = ScopeSet::from_scopes([Scope::AppAdmin]);
        assert!(admin.grants(Scope::AppSave), "app:admin 是 app:* 的超集");
        assert!(admin.grants(Scope::AppReloadEngine));
        assert!(admin.grants(Scope::AppAdmin));
        for ui in [Scope::UiRead, Scope::UiScreenshot, Scope::UiInject] {
            assert!(!admin.grants(ui), "app:admin 不得隐含 {ui}");
        }
    }

    #[test]
    fn ui_read_never_grants_domain_tools() {
        let read_only = ScopeSet::from_scopes([Scope::UiRead]);
        assert!(read_only.grants(Scope::UiRead));
        for scope in [Scope::AppSave, Scope::AppReloadEngine, Scope::AppAdmin] {
            assert!(!read_only.grants(scope), "ui:read 不得授予 {scope}");
        }
    }

    #[test]
    fn ui_inject_is_forbidden_in_production() {
        assert!(Scope::UiInject.is_production_forbidden());
        assert_eq!(RunMode::default(), RunMode::Production, "生产模式是默认");

        let expected = fixture_token(0x11);
        let header = format!("Bearer {}", expected.expose());
        let context = AuthContext::http(Some(&header), ScopeSet::all(), RunMode::Production);
        assert_eq!(
            authorize(&expected, &context, Scope::UiInject),
            Err(Denial::ForbiddenInProduction {
                scope: Scope::UiInject,
                mode: RunMode::Production,
            }),
            "生产模式必须硬拒 ui:inject"
        );
        assert_eq!(
            Denial::ForbiddenInProduction {
                scope: Scope::UiInject,
                mode: RunMode::Production,
            }
            .http_status(),
            403
        );

        // 测试模式（且显式授予）才放行。
        let test_context = AuthContext::http(Some(&header), ScopeSet::all(), RunMode::Test);
        assert_eq!(authorize(&expected, &test_context, Scope::UiInject), Ok(()));
        // 即使测试模式, 没有 ui:inject 授予也不行。
        let no_inject = AuthContext::http(
            Some(&header),
            ScopeSet::from_scopes([Scope::AppAdmin]),
            RunMode::Test,
        );
        assert!(matches!(
            authorize(&expected, &no_inject, Scope::UiInject),
            Err(Denial::InsufficientScope { .. })
        ));
    }

    #[test]
    fn missing_and_wrong_tokens_are_rejected_on_every_channel() {
        let expected = fixture_token(0x22);
        let wrong = fixture_token(0x33);

        // 1) 完全没有 Authorization 头。
        let missing_http = AuthContext::http(None, ScopeSet::all(), RunMode::Production);
        assert_eq!(missing_http.credential, Credential::Missing);
        assert_eq!(
            authorize(&expected, &missing_http, Scope::AppAdmin),
            Err(Denial::MissingToken)
        );

        // 2) 形状非法。
        let malformed = AuthContext::http(
            Some("Basic dXNlcjpwYXNz"),
            ScopeSet::all(),
            RunMode::Production,
        );
        assert_eq!(
            authorize(&expected, &malformed, Scope::AppAdmin),
            Err(Denial::MalformedAuthorization)
        );

        // 3) 错误 token。
        let wrong_header = format!("Bearer {}", wrong.expose());
        let wrong_context =
            AuthContext::http(Some(&wrong_header), ScopeSet::all(), RunMode::Production);
        assert_eq!(
            authorize(&expected, &wrong_context, Scope::AppAdmin),
            Err(Denial::InvalidToken)
        );

        // 4) 正确 token 才通过。
        let right_header = format!("Bearer {}", expected.expose());
        let right_context =
            AuthContext::http(Some(&right_header), ScopeSet::all(), RunMode::Production);
        assert_eq!(
            authorize(&expected, &right_context, Scope::AppAdmin),
            Ok(())
        );

        // 5) HTTP 上"我就是本进程"这种凭据不被接受。
        let local_on_http = AuthContext {
            channel: Channel::Http,
            credential: Credential::LocalProcess,
            granted: ScopeSet::all(),
            mode: RunMode::Production,
        };
        assert!(matches!(
            authorize(&expected, &local_on_http, Scope::AppAdmin),
            Err(Denial::CredentialNotAcceptedOnChannel { .. })
        ));

        // 6) 连 stdio 上传入 Missing 也必须拒（判定不依赖通道）。
        let missing_stdio = AuthContext {
            channel: Channel::Stdio,
            credential: Credential::Missing,
            granted: ScopeSet::all(),
            mode: RunMode::Production,
        };
        assert_eq!(
            authorize(&expected, &missing_stdio, Scope::AppAdmin),
            Err(Denial::MissingToken)
        );

        // 7) stdio 上的 LocalProcess（由 from_headers 在"无头"时给出）通过。
        let stdio = AuthContext::stdio(None, ScopeSet::all(), RunMode::Production);
        assert_eq!(stdio.credential, Credential::LocalProcess);
        assert_eq!(authorize(&expected, &stdio, Scope::AppAdmin), Ok(()));
    }

    #[test]
    fn authorization_header_parsing_is_strict() {
        assert_eq!(
            Credential::parse_header("Bearer abc"),
            Credential::Bearer("abc")
        );
        assert_eq!(
            Credential::parse_header("bearer abc"),
            Credential::Bearer("abc"),
            "方案名大小写不敏感 (RFC 6750)"
        );
        assert_eq!(
            Credential::parse_header("Bearer  abc"),
            Credential::Bearer("abc")
        );
        for bad in ["", "Bearer", "Basic abc", "Token abc", "abc"] {
            assert!(
                matches!(Credential::parse_header(bad), Credential::Malformed(_)),
                "`{bad}` 必须被判为形状非法"
            );
        }
        assert!(matches!(
            Credential::parse_header("Bearer a b"),
            Credential::Malformed(_)
        ));
    }

    #[test]
    #[cfg(unix)]
    fn token_file_is_written_0600_and_read_back() {
        let path = temp_token_path("write");
        let file = TokenFile::new(&path);
        let token = fixture_token(0x44);
        file.save(&token).expect("写令牌");
        let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, TOKEN_FILE_MODE, "令牌文件必须是 0600");
        let parent_mode = fs::metadata(path.parent().expect("parent"))
            .expect("stat dir")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(parent_mode, TOKEN_DIR_MODE, "令牌目录必须是 0700");
        assert_eq!(file.load().expect("读令牌"), token);
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    #[test]
    #[cfg(unix)]
    fn token_file_with_loose_permissions_is_refused() {
        let path = temp_token_path("loose");
        let token = fixture_token(0x55);
        fs::write(&path, format!("{}\n", token.expose())).expect("写");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod");
        let error = TokenFile::new(&path).load().expect_err("宽松权限必须被拒");
        assert!(
            matches!(
                error,
                SecurityError::InsecureTokenPermissions {
                    mode: 0o644,
                    expected: TOKEN_FILE_MODE,
                    ..
                }
            ),
            "实际错误: {error:?}"
        );
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    #[test]
    #[cfg(unix)]
    fn token_file_save_repairs_loose_permissions() {
        let path = temp_token_path("repair");
        fs::write(&path, "old\n").expect("预置文件");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).expect("chmod");
        let token = fixture_token(0x66);
        TokenFile::new(&path)
            .save(&token)
            .expect("保存必须纠正权限");
        let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, TOKEN_FILE_MODE);
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    #[test]
    #[cfg(unix)]
    fn load_or_create_writes_once_then_reuses() {
        let path = temp_token_path("loc");
        let file = TokenFile::new(&path);
        let (first, created) = file.load_or_create().expect("首次");
        assert!(created);
        let (second, created_again) = file.load_or_create().expect("二次");
        assert!(!created_again);
        assert_eq!(first, second, "第二次必须复用同一令牌");
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    // -----------------------------------------------------------------------
    // 令牌文件的原子落盘 + 坏文件 fail-closed（本轮新增）
    // -----------------------------------------------------------------------

    /// 判据 A（牙齿）：保存必须是"同目录临时文件 + `rename`"，不是原地截断写。
    ///
    /// 注入证明：把 `save` 退回 `truncate(true)` 原地写 ⇒ inode 不变 ⇒ 本判据在
    /// 「磁盘上的字节其实是对的」的情况下依然变红。这是本轮唯一一条**只要写入回退成
    /// 非原子就变红**的判据，因此它必须报出 inode 字面值，而不是只说"不相等"。
    #[test]
    #[cfg(unix)]
    fn save_replaces_the_inode_instead_of_truncating_in_place() {
        use std::os::unix::fs::MetadataExt as _;

        let path = temp_token_path("atomic-inode");
        let file = TokenFile::new(&path);
        // 先放一份**合法**旧令牌：原地截断写会毁掉它，原子替换会整份换掉它。
        let old = fixture_token(0x11);
        fs::write(&path, format!("{}\n", old.expose())).expect("预置旧令牌");
        fs::set_permissions(&path, fs::Permissions::from_mode(TOKEN_FILE_MODE)).expect("chmod");
        let before = fs::metadata(&path).expect("旧文件元数据").ino();

        let new = fixture_token(0x22);
        file.save(&new).expect("保存");
        let after = fs::metadata(&path).expect("新文件元数据").ino();
        assert_ne!(
            before, after,
            "令牌文件保存必须是'新文件 + rename'（inode 从 {before} 变成 {after}）；\
             inode 不变意味着原地截断写，两次 write_all 之间被打断就会留下空/半截文件"
        );
        assert_eq!(file.load().expect("读回"), new);
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    /// 判据 B（牙齿）：空令牌文件是**坏文件**，必须被明确拒绝，且**不得**被静默重新生成。
    ///
    /// 这个字节形态是**实测**的中断现场（不是猜的）：旧 `save` 的 `open(O_TRUNC)`
    /// 一返回，磁盘上就是 0 字节，而 `exists()` 已经是真。
    #[test]
    #[cfg(unix)]
    fn empty_token_file_is_rejected_and_never_silently_replaced() {
        let path = temp_token_path("atomic-empty");
        fs::write(&path, b"").expect("造空文件");
        fs::set_permissions(&path, fs::Permissions::from_mode(TOKEN_FILE_MODE)).expect("chmod");

        let error = TokenFile::new(&path)
            .load_or_create()
            .expect_err("空文件必须被拒绝");
        assert!(
            matches!(error, SecurityError::MalformedTokenFile { .. }),
            "空文件必须走'坏令牌文件'的专用错误，实际: {error:?}"
        );
        let reported = error.to_string();
        assert!(
            reported.contains(&path.display().to_string()),
            "错误必须指出路径，实际: {reported}"
        );
        assert_eq!(
            fs::read(&path).expect("读回"),
            Vec::<u8>::new(),
            "坏文件不得被静默覆盖成新令牌"
        );
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    /// 判据 C（牙齿）：**中断现场**必须被拒绝，且**不得**被静默重新生成。
    ///
    /// ## 两个"半截"状态，必须分开说（第一版判据在这里错过一次）
    ///
    /// `save` 的旧 I/O 序列是 `open(O_TRUNC)` → `write_all(64B)` → `write_all(1B, "\n")`。
    /// 被打断时磁盘上可能是：
    ///
    /// | 中断点 | 磁盘内容 | 旧 `exists() ⇒ load()` 的判定 |
    /// | :--- | :--- | :--- |
    /// | `open(O_TRUNC)` 之后、第一次 `write_all` 之前 | **空文件** | 拒绝（`parse` 长度 0） |
    /// | 第一次 `write_all` 之后、第二次 `write_all` 之前 | **恰好 64 位十六进制（无换行）** | **接受 —— 危险** |
    /// | 第二次 `write_all` 写了一半 | 64 位十六进制 + 半个换行 | 拒绝（65 字节但尾部不是合法文本） |
    ///
    /// 中间那一行最危险：它是**格式合法**的令牌。`BearerToken::parse` 无从分辨
    /// "这是半截文件"与"这就是一份没带换行的合法令牌"（两种都在既有格式之内）。
    /// 所以安全不能靠读侧的长度启发式，只能靠**写侧原子**：
    /// `save_replaces_the_inode_instead_of_truncating_in_place` 才是这条风险的直接判据；
    /// 本判据钉住的是**读侧的处置**——遇到不可用的内容必须报错，绝不静默换令牌。
    ///
    /// 本判据因此用**空文件**（`open(O_TRUNC)` 之后那个状态，真实且可区分）走
    /// `MalformedTokenFile`；下面第二段再钉住"垃圾内容"也被拒绝。
    #[test]
    #[cfg(unix)]
    fn torn_token_file_is_rejected_and_never_silently_replaced() {
        let path = temp_token_path("atomic-torn");
        // 中断点 1: `open(O_TRUNC)` 之后, 一个字节都没写。
        fs::write(&path, b"").expect("造中断现场");
        fs::set_permissions(&path, fs::Permissions::from_mode(TOKEN_FILE_MODE)).expect("chmod");
        assert_eq!(
            fs::read(&path).expect("读回"),
            Vec::<u8>::new(),
            "造现场失败: 磁盘上不是空文件"
        );
        let file = TokenFile::new(&path);
        let error = file.load_or_create().expect_err("中断现场必须被拒绝");
        assert!(
            matches!(error, SecurityError::MalformedTokenFile { .. }),
            "中断现场必须走'坏令牌文件'的专用错误，实际: {error:?}"
        );
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "错误必须指出路径，实际: {error}"
        );
        assert_eq!(
            fs::read(&path).expect("读回"),
            Vec::<u8>::new(),
            "坏文件不得被静默覆盖成新令牌"
        );

        // 中断点 3 的等价形态: 长度为 64 但含非十六进制字符(半个换行 / 垃圾)。
        let junk = format!("{}{}", "c".repeat(TOKEN_HEX_LEN - 1), "\u{0}");
        fs::write(&path, junk.as_bytes()).expect("造垃圾现场");
        let error = file.load_or_create().expect_err("垃圾内容必须被拒绝");
        assert!(
            matches!(error, SecurityError::MalformedToken { .. }),
            "垃圾内容必须报令牌格式非法，实际: {error:?}"
        );
        assert_eq!(
            fs::read_to_string(&path).expect("读回"),
            junk,
            "坏文件不得被静默覆盖成新令牌"
        );
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    /// 判据 D：坏文件被拒绝之后，**删掉它**再启动必须能恢复（正常路径没有被堵死）。
    ///
    /// 这条是判据 B/C 的"反面"：拒绝不是死锁 —— 运维动作（删文件）之后必须立刻能拿到新令牌。
    #[test]
    #[cfg(unix)]
    fn deleting_a_rejected_token_file_restores_the_normal_path() {
        let path = temp_token_path("atomic-recover");
        fs::write(&path, b"junk").expect("造坏文件");
        fs::set_permissions(&path, fs::Permissions::from_mode(TOKEN_FILE_MODE)).expect("chmod");
        let file = TokenFile::new(&path);
        // 只断言"被拒绝"：具体是哪一种坏文件不是这条判据的对象
        // （空文件走 `MalformedTokenFile`，垃圾内容走 `MalformedToken`，两条都在 B/C 里钉住）。
        assert!(file.load_or_create().is_err(), "坏文件必须被拒绝");

        fs::remove_file(&path).expect("运维动作: 删掉坏文件");
        let (token, created) = file.load_or_create().expect("删掉之后必须能重新生成");
        assert!(created, "删掉之后这一次必须是'新建'");
        assert_eq!(token.expose().len(), TOKEN_HEX_LEN);
        assert_eq!(file.load().expect("读回"), token);
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    /// 判据 E（权限窗口）：临时文件**在创建时**就必须是 `0600`，不留"先宽后收"的窗口。
    ///
    /// 手法：在 `save` 运行期间反复 `read_dir` 目标目录，记录每一个出现过的临时文件的权限位。
    /// 旧实现的临时文件**就是目标文件本身**（没有临时文件），因此这条判据在有临时文件之后才有意义。
    ///
    /// **如实登记（不许弱化）**：采样是概率性的。它只能证明"本次采样没抓到宽权限窗口"，
    /// 不能证明"任何时刻都没有窗口"。真正的结构保证是写入路径上的
    /// `OpenOptions::mode(TOKEN_FILE_MODE)` + `create_new(true)`：文件**一出生**就是 `0600`。
    #[test]
    #[cfg(unix)]
    fn temp_file_is_never_visible_with_loose_permissions() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let path = temp_token_path("atomic-mode-window");
        let file = TokenFile::new(&path);
        file.save(&fixture_token(0x33)).expect("先放一份合法令牌");
        let parent = path.parent().expect("parent").to_path_buf();
        let name = path
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();

        let stop = Arc::new(AtomicBool::new(false));
        let watcher_stop = Arc::clone(&stop);
        let watcher_dir = parent.clone();
        let watcher_name = name.clone();
        let watcher = std::thread::spawn(move || {
            let mut samples = 0_usize;
            let mut loose: Vec<(u32, String)> = Vec::new();
            while !watcher_stop.load(Ordering::Relaxed) {
                if let Ok(entries) = fs::read_dir(&watcher_dir) {
                    for entry in entries.flatten() {
                        let entry_name = entry.file_name().to_string_lossy().into_owned();
                        if entry_name == watcher_name {
                            continue;
                        }
                        if let Ok(meta) = entry.metadata() {
                            samples += 1;
                            let mode = meta.permissions().mode() & 0o777;
                            if mode != TOKEN_FILE_MODE {
                                loose.push((mode, entry_name));
                            }
                        }
                    }
                }
            }
            (samples, loose)
        });

        for i in 0..64_u8 {
            file.save(&fixture_token(i)).expect("保存");
        }
        stop.store(true, Ordering::Relaxed);
        let (samples, loose) = watcher.join().expect("采样线程");
        assert!(
            loose.is_empty(),
            "临时文件出现过非 0600 权限（mode, name）= {loose:?}；\
             这说明存在'先建宽权限再 chmod'的可读窗口（本次采样 {samples} 次）"
        );
        let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, TOKEN_FILE_MODE, "最终文件必须仍恰好是 0600");
        let leftovers: Vec<String> = fs::read_dir(&parent)
            .expect("读目录")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|entry| entry != &name)
            .collect();
        assert!(
            leftovers.is_empty(),
            "保存成功后不得留下临时文件: {leftovers:?}"
        );
        fs::remove_dir_all(&parent).ok();
    }

    #[test]
    fn default_path_is_the_dot_yeban_session_token() {
        let path = TokenFile::default_path().expect("HOME 通常存在");
        assert!(path.ends_with(TOKEN_DIR_NAME.to_owned() + "/" + TOKEN_FILE_NAME));
        assert!(path.is_absolute());
    }

    #[test]
    #[cfg(not(unix))]
    fn token_file_is_unsupported_on_non_unix() {
        // 明确拒绝, 而不是静默放过权限校验。
        let path = std::env::temp_dir()
            .join("yeban-mcp-token-non-unix")
            .join(TOKEN_FILE_NAME);
        let file = TokenFile::new(path);
        assert!(matches!(
            file.save(&fixture_token(0x77)),
            Err(SecurityError::UnsupportedPlatform)
        ));
        assert!(matches!(
            file.load(),
            Err(SecurityError::UnsupportedPlatform)
        ));
    }

    #[test]
    fn denial_status_and_shape_are_machine_readable() {
        let denial = Denial::InsufficientScope {
            required: Scope::AppAdmin,
            granted: ScopeSet::from_scopes([Scope::UiRead]),
        };
        assert_eq!(denial.http_status(), 403);
        assert_eq!(denial.rpc_code(), FORBIDDEN);
        let data = denial.data();
        assert_eq!(data["kind"], "insufficient-scope");
        assert_eq!(data["requiredScope"], "app:admin");
        assert_eq!(data["grantedScopes"], "ui:read");
        assert_eq!(Denial::MissingToken.rpc_code(), UNAUTHORIZED);
        assert_eq!(Denial::MissingToken.data()["httpStatus"], 401);
    }
}
