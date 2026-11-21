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
//! 高熵来源只读 Unix 的 `/dev/urandom`（纯 `std::fs`）；读不到时 **fail-closed**
//! 报错，不再退回确定性 splitmix64 混合器（`SystemTime` 纳秒 + PID + 栈地址 +
//! 原子计数器）—— 令牌是 UI 控制面的唯一凭据，弱令牌不可接受（T-FIX-32）。
//! 不需要 `rand` / `getrandom` / `libc`（`docs/adr/ADR-0001` D20/D21 的零新增依赖
//! 倾向）：`O_NOFOLLOW` 用按平台钉死的 ABI 常量交给 `OpenOptionsExt::custom_flags`。
//! symlink / 检查后被替换 / 权限不是 `0600` 一律拒绝；非 Unix / 非 Windows 平台上
//! [`TokenFile::save`] / [`TokenFile::load`] 返回
//! [`SecurityError::UnsupportedPlatform`] —— **明确拒绝，而不是静默放过**。
//!
//! [PATCH] Windows 例外：它没有权限位，但**有 ACL**。因此 Windows 上走
//! `icacls /inheritance:r` + 只授当前用户读，并**读回**真实 ACL 判定
//! （`win_acl::is_owner_only`）—— 语义等价于 `0600`，收不紧仍然 fail-closed。
//! 同一个补丁把 Windows 的熵源从"退回确定性回退"改成 `RandomState`（OS 播种，见 `fill_os_random` 的 Windows 实现说明）。

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt;
#[cfg(any(unix, windows))]
use std::fs;
#[cfg(unix)]
use std::io::Read as _;
#[cfg(any(unix, windows))]
use std::io::Write as _;
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
#[cfg(any(unix, windows))]
const TOKEN_TEMP_INFIX: &str = ".tmp-";

/// 进程内临时文件序号（保证同一 PID 的多次 `save` 不会撞名）。
#[cfg(any(unix, windows))]
static TOKEN_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `O_NOFOLLOW` 的 ABI 值，交给 [`std::os::unix::fs::OpenOptionsExt::custom_flags`]。
///
/// 只在本仓 CI / 主要部署平台 Linux 上给出实测值（`os::O_NOFOLLOW == 0o400000`，
/// 取自 `<bits/fcntl-linux.h>`）。其余 Unix 平台**保留 0**：不猜各发行版各异的 ABI
/// 常量（macOS 是 `0x10000`、FreeBSD 是 `0x200`…… 硬编码错值会把无关 flag 塞进
/// `open`，比不设更危险）。那些平台上 symlink 与"检查后替换"由
/// [`TokenFile::load`] 里 `symlink_metadata` 与 fd `metadata` 的 `(dev, ino)` 对比
/// 兜底 —— 安全性不因 flag 缺省而下降，且全程零新增依赖（不引入 `libc`）。
#[cfg(target_os = "linux")]
const O_NOFOLLOW_FLAG: i32 = 0o400_000;
#[cfg(all(unix, not(target_os = "linux")))]
const O_NOFOLLOW_FLAG: i32 = 0;

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
    /// 找不到家目录（`HOME` / `USERPROFILE` 都未设置或为空）。
    ///
    /// fail-closed：不回退共享临时目录，直接拒绝，避免令牌落到可预测的 `/tmp` 路径。
    #[error("无法定位家目录 (HOME / USERPROFILE 均未设置): 拒绝把令牌写入共享临时目录")]
    HomeDirectoryUnavailable,
    /// 令牌路径不是"打开那一刻就固定下来"的普通文件。
    ///
    /// 覆盖两种 fail-closed 情形：路径是 FIFO / 目录 / 设备（不是普通文件），
    /// 或 `symlink_metadata` 与 `open` 之间 inode 被换掉（TOCTOU）。
    #[error("令牌路径 `{path}` 不是稳定的普通文件: {reason}")]
    NotARegularFile {
        /// 文件路径。
        path: String,
        /// 具体原因（非普通文件 / 检查后被替换）。
        reason: String,
    },
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
    /// 生成一个新的 256-bit 令牌 [ARCH-SEC-002]（**fail-closed**）。
    ///
    /// 只走操作系统熵源；熵源不可用时**响亮 panic**，绝不退回确定性弱熵
    /// （时间 + PID + 栈地址是可预测的，不能当令牌）。 insecure 环境宁可起不来，
    /// 也不能产出可被预测的 Bearer Token。
    ///
    /// 需要可恢复错误时（如令牌文件落盘路径）用 [`Self::try_generate`]。
    ///
    /// # Panics
    ///
    /// 操作系统熵源不可用时 panic（fail-closed，绝不降级）。
    #[must_use]
    pub fn generate() -> GeneratedToken {
        Self::try_generate().expect(
            "OS 熵源不可用: 拒绝生成确定性弱令牌 (fail-closed); 如需可恢复错误请用 try_generate",
        )
    }

    /// 生成令牌的可恢复版本：熵源失败返回 [`SecurityError::EntropyUnavailable`]。
    ///
    /// 与 [`Self::generate`] 同样**只走 OS 熵**；区别是错误以 `Result` 返回而不是 panic。
    /// 生产路径（令牌文件 `load_or_create`）必须走这里，实现 fail-closed。
    ///
    /// # Errors
    ///
    /// 操作系统熵源不可用时返回 `EntropyUnavailable`。
    pub fn try_generate() -> Result<GeneratedToken, SecurityError> {
        Self::try_generate_with(fill_os_random)
    }

    /// [`Self::try_generate`] 的可注入内核：把熵源做成参数，好让回归测试能
    /// **确定性地**注入"熵源失败"（而不是去动全局的 `/dev/urandom`）。
    ///
    /// 熵失败一律 `?` 原样传播（fail-closed），绝不退回确定性弱熵。
    fn try_generate_with(
        fill: impl FnOnce(&mut [u8]) -> Result<(), SecurityError>,
    ) -> Result<GeneratedToken, SecurityError> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        fill(&mut bytes)?;
        Ok(GeneratedToken {
            token: Self(bytes_to_hex(&bytes)),
            source: EntropySource::OsRandom,
        })
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

/// 家目录解析（问题报告 ⑤ / T-FIX-32）：`HOME` → `USERPROFILE`。
///
/// Windows 默认 PowerShell 会话里没有 `HOME`，惯例是 `USERPROFILE`；只认 `HOME`
/// 会让裸起 `yeban-mcp --stdio` 直接死在"无法定位家目录"。两级都缺失时
/// **fail-closed**：不再回退共享临时目录（那里路径可预测，令牌会被别的本地用户
/// 抢注/读取），而是明确报 [`SecurityError::HomeDirectoryUnavailable`]。
fn home_dir() -> Result<PathBuf, SecurityError> {
    home_dir_from(
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("USERPROFILE").as_deref(),
    )
}

/// [`home_dir`] 的可注入内核：纯函数，接收两级候选环境变量，好让回归测试
/// 确定性地构造"两级全缺"而不改测试进程的真实环境变量。
fn home_dir_from(
    home: Option<&OsStr>,
    userprofile: Option<&OsStr>,
) -> Result<PathBuf, SecurityError> {
    resolve_home_dir(home, userprofile).ok_or(SecurityError::HomeDirectoryUnavailable)
}

/// [`home_dir`] 的纯函数本体：依次取第一个**非空**候选，全缺返回 `None`（fail-closed）。
///
/// 空串视为未设置：空 `HOME` 拼出的会是相对路径，令牌文件会落到当前目录 ——
/// 比报错更糟。**不再回退临时目录**：共享 `/tmp` 下路径可预测，令牌会被别的本地用户
/// 抢注/读取。拆成纯函数是为了让回退链在任何平台上都能被单测构造，
/// 不依赖测试进程真实改环境变量。
fn resolve_home_dir(home: Option<&OsStr>, userprofile: Option<&OsStr>) -> Option<PathBuf> {
    for candidate in [home, userprofile] {
        if let Some(value) = candidate
            && !value.is_empty()
        {
            return Some(PathBuf::from(value));
        }
    }
    None
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
    /// 家目录按 [`home_dir`] 的解析链取（`HOME` → `USERPROFILE`）；两级都缺
    /// **响亮报错**，绝不落到共享临时目录（fail-closed）。
    ///
    /// # Errors
    ///
    /// `HOME` 与 `USERPROFILE` 都未设置 / 为空时返回 `HomeDirectoryUnavailable`。
    pub fn default_path() -> Result<PathBuf, SecurityError> {
        Ok(home_dir()?.join(TOKEN_DIR_NAME).join(TOKEN_FILE_NAME))
    }

    /// 规范默认位置的令牌文件。
    ///
    /// # Errors
    ///
    /// 同 [`TokenFile::default_path`]。
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

    /// Windows：用 owner-only ACL **等价**落实 `0600`，收不紧就 fail-closed。
    ///
    /// POSIX 的 `0600` 在 Windows 上没有权限位可写，[PATCH] 因此这里做的是
    /// **语义等价**而不是放宽：
    /// 1. 临时文件 `create_new`（与 Unix 同一套三段式落盘，不会有半截文件）；
    /// 2. `icacls /inheritance:r` 断掉**全部**继承 ACE（SYSTEM / Administrators /
    ///    Users 随之消失），再 `/grant:r "<user>:R"` 只给**当前用户**读；
    /// 3. `icacls` **读回**再判一遍（[`win_acl::is_owner_only`]）—— 只要还能看到
    ///    `Everyone` / `BUILTIN\Users` / `Authenticated Users` 这类宽主体，就
    ///    **拒绝发布令牌**，不让"权限收不紧"变成一条静默通过的路径；
    /// 4. 目标已存在时先删再 rename（Windows 的 `rename` 不覆盖既有文件），
    ///    因此这一步**不是**原子替换 —— 代价写在下面，由读侧的内容校验兜住。
    ///
    /// # Errors
    ///
    /// `icacls` 不可用 / 权限收不紧 / I/O 失败。
    #[cfg(windows)]
    pub fn save(&self, token: &BearerToken) -> Result<(), SecurityError> {
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

        let temp = parent.join(format!(
            "{TOKEN_FILE_NAME}{TOKEN_TEMP_INFIX}{}-{:x}",
            std::process::id(),
            TOKEN_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        let written = (|| -> Result<(), SecurityError> {
            {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)
                    .map_err(io)?;
                file.write_all(token.expose().as_bytes()).map_err(io)?;
                file.write_all(b"\n").map_err(io)?;
                file.sync_all().map_err(io)?;
            }
            win_acl::restrict_owner_read(&temp)?;
            if !win_acl::is_owner_only(&temp)? {
                return Err(SecurityError::InsecureTokenPermissions {
                    path: path_text.clone(),
                    mode: 0,
                    expected: TOKEN_FILE_MODE,
                });
            }
            if self.path.exists() {
                fs::remove_file(&self.path).map_err(io)?;
            }
            fs::rename(&temp, &self.path).map_err(io)
        })();

        if written.is_err() {
            let _ = fs::remove_file(&temp);
        }
        written
    }

    /// 非 Unix / 非 Windows 平台：明确拒绝，不静默放过。
    ///
    /// # Errors
    ///
    /// 永远返回 [`SecurityError::UnsupportedPlatform`]。
    #[cfg(all(not(unix), not(windows)))]
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
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
        let path_text = self.path.display().to_string();
        let io = |source: std::io::Error| SecurityError::Io {
            path: path_text.clone(),
            source,
        };

        // 第 1 步（诊断 + 非 Linux 兜底）：按路径 lstat 一次，识别 symlink，并记下
        // (dev, ino)。这份元数据**不作为权限判据** —— 它会随路径被替换而过期。
        let path_meta = fs::symlink_metadata(&self.path).map_err(io)?;
        if path_meta.file_type().is_symlink() {
            return Err(SecurityError::TokenPathIsSymlink { path: path_text });
        }

        // 第 2 步（消除 TOCTOU 的核心）：O_NOFOLLOW 打开，拿到指向"打开那一刻那个
        // inode"的 fd；此后所有检查与读取都针对这个 fd，路径再被 rename / 换成
        // symlink 都不影响。即便某平台 O_NOFOLLOW_FLAG 为 0，第 3 步的 (dev, ino)
        // 对比仍会拒绝被替换的路径。
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW_FLAG)
            .open(&self.path)
            .map_err(io)?;

        // 第 3 步：fd metadata（fstat）—— 类型、身份、权限都来自同一 inode。
        let meta = file.metadata().map_err(io)?;
        if !meta.file_type().is_file() {
            return Err(SecurityError::NotARegularFile {
                path: path_text,
                reason: String::from("令牌路径不是普通文件"),
            });
        }
        if (meta.dev(), meta.ino()) != (path_meta.dev(), path_meta.ino()) {
            return Err(SecurityError::NotARegularFile {
                path: path_text,
                reason: String::from("令牌文件在权限检查与打开之间被替换"),
            });
        }
        let mode = meta.permissions().mode() & 0o777;
        if mode != TOKEN_FILE_MODE {
            return Err(SecurityError::InsecureTokenPermissions {
                path: path_text,
                mode,
                expected: TOKEN_FILE_MODE,
            });
        }

        // 第 4 步：从 fd 读，而不是按路径 read_to_string —— 与第 2/3 步同一个 inode。
        let mut text = String::new();
        file.read_to_string(&mut text).map_err(io)?;
        // 空白文件**单独命名**，这样运维一眼能分辨"崩溃留空文件"与"文件被人塞了垃圾"。
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

    /// Windows：读取令牌，并用**读回 ACL** 代替 `0600` 位校验。
    ///
    /// 判定对象与 Unix 那条逐字同义："这个文件是不是只有属主能读"。差别只在
    /// 取证方式 —— Windows 没有权限位，因此 [`win_acl::is_owner_only`] 去问
    /// `icacls`，只要还能看到 `Everyone` / `BUILTIN\Users` 这类宽主体就报
    /// [`SecurityError::InsecureTokenPermissions`]（`mode: 0` 表示"平台没有这个
    /// 数值"，而不是"权限是 000"）。
    ///
    /// # Errors
    ///
    /// 路径是符号链接、ACL 不是 owner-only、内容不是 64 位十六进制、I/O 失败。
    #[cfg(windows)]
    pub fn load(&self) -> Result<BearerToken, SecurityError> {
        let path_text = self.path.display().to_string();
        let io = |source: std::io::Error| SecurityError::Io {
            path: path_text.clone(),
            source,
        };
        let link_meta = fs::symlink_metadata(&self.path).map_err(io)?;
        if link_meta.file_type().is_symlink() {
            return Err(SecurityError::TokenPathIsSymlink { path: path_text });
        }
        if !win_acl::is_owner_only(&self.path)? {
            return Err(SecurityError::InsecureTokenPermissions {
                path: path_text,
                mode: 0,
                expected: TOKEN_FILE_MODE,
            });
        }
        let text = fs::read_to_string(&self.path).map_err(io)?;
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

    /// 非 Unix / 非 Windows 平台：明确拒绝，不静默放过。
    ///
    /// # Errors
    ///
    /// 永远返回 [`SecurityError::UnsupportedPlatform`]。
    #[cfg(all(not(unix), not(windows)))]
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
        // fail-closed: 熵源不可用时直接返回错误，绝不落盘弱令牌。
        let generated = BearerToken::try_generate()?;
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
#[cfg(all(not(unix), not(windows)))]
pub fn fill_os_random(_buf: &mut [u8]) -> Result<(), SecurityError> {
    Err(SecurityError::UnsupportedPlatform)
}

/// Windows 上的 OS 熵源。
///
/// [PATCH] 之前这里走 `UnsupportedPlatform`，`BearerToken::generate` 于是退回
/// [`fill_deterministic`]（时间 + PID + 栈地址），令牌**不是**不可预测的 ——
/// 而它是 UI 控制面的唯一凭据。
///
/// ## 为什么不是 `BCryptGenRandom`
///
/// 那个调用必须写 `unsafe` 块，而本 crate 的 `#![forbid(unsafe_code)]` 是一条
/// 红线；为一条令牌就把它开个洞不值得。这里改用 `std` 自己那条**已经被 OS 播种**
/// 的哈希键：`RandomState::new()` 的键来自 `std` 的 OS 随机源
/// （Windows 上即 `RtlGenRandom`），每次 `build_hasher()` 得到的 SipHash 都以
/// 这对**不可预测**的键为种子。于是"没有 `unsafe`、没有新依赖、种子来自 OS"
/// 三件事同时成立。
///
/// 诚实登记它的边界：这是**带随机种子的哈希**，不是密码学 KDF。它够用于
/// "本机进程间的一次性凭据"，**不**用于会话密钥 —— 与本模块自始的定位一致。
///
/// # Errors
///
/// 不产生错误（`Result` 保留是为了与 Unix 腿同一签名）。
#[cfg(windows)]
pub fn fill_os_random(buf: &mut [u8]) -> Result<(), SecurityError> {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher as _, Hasher as _};

    let seed = RandomState::new();
    let mut written = 0usize;
    while written < buf.len() {
        let mut hasher = seed.build_hasher();
        hasher.write_u64(TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed));
        let digest = hasher.finish().to_le_bytes();
        for byte in &digest {
            if written == buf.len() {
                break;
            }
            buf[written] = *byte;
            written += 1;
        }
    }
    Ok(())
}

/// Windows 上"`0600` 等价"的唯一实现点：`icacls` 断继承 + 只授当前用户读，
/// 并且每次判定都**读回**真实 ACL，而不是相信刚才那条命令的退出码。
#[cfg(windows)]
mod win_acl {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::SecurityError;

    /// 宽主体名单 —— 出现任何一个就说明"这个文件不止属主能读"。
    /// 比对时按 `icacls` 里 `DOMAIN\Name:(R)` 的**主体部分**做不区分大小写的包含判定。
    const BROAD_PRINCIPALS: [&str; 7] = [
        "Everyone",
        "BUILTIN\\Users",
        "Authenticated Users",
        "INTERACTIVE",
        "ANONYMOUS LOGON",
        "NT AUTHORITY\\NETWORK",
        "NETWORK SERVICE",
    ];

    fn io_error(path: &Path, message: impl Into<String>) -> SecurityError {
        SecurityError::Io {
            path: path.display().to_string(),
            source: std::io::Error::other(message.into()),
        }
    }

    /// `icacls` 的绝对路径（T-FIX-32）：裸名 `Command::new("icacls")` 会走
    /// PATH + 当前目录搜索，一份同目录同名 `icacls.exe` 就能劫持"权限收紧"这道
    /// 安全关口。固定到 `%SystemRoot%\System32\icacls.exe`；`SystemRoot` 缺失时
    /// 退回 Windows 标准位置 `C:\Windows`（系统保护目录，普通用户不可写）。
    fn icacls_program() -> PathBuf {
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .filter(|root| !root.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
            .join("System32")
            .join("icacls.exe")
    }

    fn icacls(path: &Path, args: &[&str]) -> Result<String, SecurityError> {
        let output = Command::new(icacls_program())
            .arg(path)
            .args(args)
            .output()
            .map_err(|error| io_error(path, format!("icacls 无法执行: {error}")))?;
        if !output.status.success() {
            return Err(io_error(
                path,
                format!(
                    "icacls {args:?} 失败 ({}) : {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// 当前用户名（`USERNAME`）。没有它就没法写 owner-only ACL ⇒ 直接拒绝。
    fn current_user(path: &Path) -> Result<String, SecurityError> {
        std::env::var("USERNAME")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| io_error(path, "环境变量 USERNAME 缺失, 无法把令牌收成 owner-only"))
    }

    /// 断继承 → 只授当前用户读。两步都跑完才算成功。
    pub(super) fn restrict_owner_read(path: &Path) -> Result<(), SecurityError> {
        let user = current_user(path)?;
        // `/inheritance:r`：删掉所有**继承**来的 ACE（SYSTEM / Administrators / Users）。
        icacls(path, &["/inheritance:r"])?;
        // `/grant:r`：替换掉该主体既有的显式授权，只留 `R`。
        icacls(path, &["/grant:r", &format!("{user}:R")])?;
        Ok(())
    }

    /// 读回 ACL 判"是不是只有属主能读"。
    ///
    /// 判定口径：**没有**任何宽主体 ACE，**且**至少有一条属于当前用户的 ACE。
    /// `icacls` 输出形如
    /// `session.token DESKTOP-XX\Administrator:(R)` / `              BUILTIN\Users:(RX)`，
    /// 因此按 `:(` 切出主体名再比。
    pub(super) fn is_owner_only(path: &Path) -> Result<bool, SecurityError> {
        let user = current_user(path)?;
        let report = icacls(path, &[])?;
        let mut has_owner = false;
        for line in report.lines() {
            let Some((principal, _rights)) = line.split_once(":(") else {
                continue;
            };
            // 路径本身在最左，取最后一段当作主体（`DESKTOP-XX\Administrator`）。
            let principal = principal.trim();
            let principal = principal
                .rsplit(char::is_whitespace)
                .next()
                .unwrap_or(principal);
            let upper = principal.to_ascii_uppercase();
            if BROAD_PRINCIPALS
                .iter()
                .any(|broad| upper.contains(&broad.to_ascii_uppercase()))
            {
                return Ok(false);
            }
            if upper.ends_with(&format!("\\{}", user.to_ascii_uppercase()))
                || upper == user.to_ascii_uppercase()
            {
                has_owner = true;
            }
        }
        Ok(has_owner)
    }
}

/// 进程内单调计数器（保证同一纳秒内的多次生成长度不同的输出）。
///
/// T-FIX-32 后生产路径（Unix）不再走确定性回退，只有 Windows 熵源与测试构建
/// 用到它，故收紧 cfg，避免 Unix 非测试构建里成为死代码触发 `-D warnings`。
#[cfg(any(windows, test))]
static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 确定性回退熵源：splitmix64 混合"时间 + PID + 栈地址 + 计数器"。
///
/// T-FIX-32 起生产路径**不再**用它（熵失败 fail-closed，绝不降级），
/// 仅保留在测试构建里钉住历史混合器的可复现性，因此 `#[cfg(test)]` —— 生产
/// 二进制里彻底不含这条弱熵路径。
#[cfg(test)]
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
#[cfg(test)]
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

    /// **长度是"恰好"而不是"至少"**：比 [`TOKEN_HEX_LEN`] **长**的全十六进制文本
    /// 也必须 `MalformedToken`。
    ///
    /// 既有判据只喂了 `"abc"`（短）与 64 位非十六进制 ⇒ 放宽成"长度 < 64 才拒"
    /// 一样全绿。这一格可达：令牌文件的内容走同一条 `parse`（`TokenFile::load`），
    /// 一个被追加了字符的令牌文件会被当成合法令牌读进来，然后每一次比对都失败 ——
    /// 契约说这种文件是 `MalformedToken`，而不是"一个谁也不认识的合法令牌"。
    #[test]
    fn token_parse_rejects_a_too_long_all_hex_token() {
        let long = "a".repeat(TOKEN_HEX_LEN + 1);
        assert!(
            matches!(
                BearerToken::parse(&long),
                Err(SecurityError::MalformedToken { actual }) if actual == TOKEN_HEX_LEN + 1
            ),
            "比 {TOKEN_HEX_LEN} 位长的全十六进制令牌必须被拒"
        );
        let padded = format!("{}0", "a".repeat(TOKEN_HEX_LEN));
        assert!(
            BearerToken::parse(&padded).is_err(),
            "尾随多一个十六进制字符也必须被拒"
        );
        // 参照: 恰好 64 位仍然可解析（闸门不是"一律拒绝"）。
        assert!(BearerToken::parse(&"a".repeat(TOKEN_HEX_LEN)).is_ok());
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

    /// **长度闸门不能被前缀绕过**：候选比期望**短**时，只有"长度必须相等"那道闸门
    /// 能拦住它 —— `zip` 只走较短的一侧，若闸门写成"期望长度 < 候选长度"，
    /// 一个**严格前缀**会让 `diff` 保持 0 ⇒ 认证通过。
    ///
    /// 既有判据只喂了 `"short"`（5 字符，且不是前缀）⇒ 它是靠**内容**不等才返回
    /// false 的，那道闸门一次都没被测到。候选文本来自 `Credential::parse_header`
    /// （`split_whitespace` 只切形状，**不**校验长度），因此这一格是可达的。
    #[test]
    fn constant_time_compare_rejects_a_strict_prefix_of_the_token() {
        let token = fixture_token(0x3C);
        let full = token.expose().to_owned();
        for take in [TOKEN_HEX_LEN - 1, TOKEN_HEX_LEN / 2, 1] {
            let prefix = &full[..take];
            assert!(
                !token.ct_eq(prefix),
                "{take} 字符的前缀不得通过常量时间比较"
            );
        }
        // 反向: 以期望为前缀、但只要更长也必须 false。
        assert!(!token.ct_eq(&format!("{full}0")));
        // 闸门不是"一律 false": 全等仍然通过。
        assert!(token.ct_eq(&full));
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

    /// **权限位是"恰好 0600"而不是"不宽于 0600"**：比 `0600` **严**的模式（例如
    /// 只有属主可读的 `0400`）也必须拒绝。
    ///
    /// 既有判据只喂了 `0o644`（比 `0600` **宽**）⇒ 把闸门放宽成"mode > 0600 才拒"
    /// 一样全绿，而 `0400` 的文件会被静默接受。契约（`load` 的文档）要求的是
    /// **恰好** `0600`：那一位同时表达"谁能读"与"谁能写"，只读文件是另一个持有者
    /// 摆放的，不能当成"更安全所以可以放行"。
    #[test]
    #[cfg(unix)]
    fn token_file_with_stricter_than_0600_permissions_is_also_refused() {
        let path = temp_token_path("strict");
        let token = fixture_token(0x77);
        fs::write(&path, format!("{}\n", token.expose())).expect("写");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).expect("chmod");
        let error = TokenFile::new(&path).load().expect_err("0400 也必须被拒");
        assert!(
            matches!(
                error,
                SecurityError::InsecureTokenPermissions {
                    mode: 0o400,
                    expected: TOKEN_FILE_MODE,
                    ..
                }
            ),
            "实际错误: {error:?}"
        );
        fs::remove_dir_all(path.parent().expect("parent")).ok();
    }

    /// **符号链接令牌路径必须响亮拒绝**：`load` 先 `symlink_metadata` 再判
    /// `is_symlink`，因为"权限位是 0600"这条判据在**链接目标**上成立不蕴含路径本身
    /// 可信（`fs::metadata` 会跟随链接）。既有判据一处都没提过 `symlink`。
    #[test]
    #[cfg(unix)]
    fn a_symlinked_token_path_is_refused() {
        let target = temp_token_path("symlink-target");
        let token = fixture_token(0x88);
        fs::write(&target, format!("{}\n", token.expose())).expect("写目标");
        fs::set_permissions(&target, fs::Permissions::from_mode(TOKEN_FILE_MODE)).expect("chmod");
        let link = target
            .parent()
            .expect("parent")
            .join(format!("{TOKEN_FILE_NAME}{TOKEN_TEMP_INFIX}link"));
        std::os::unix::fs::symlink(&target, &link).expect("建符号链接");

        // 夹具前提: 链接**指向**一个合法令牌 ⇒ 目标本身是读得出来的。
        assert_eq!(
            fs::read_to_string(&link).expect("跟随链接读得到").trim(),
            token.expose()
        );
        let error = TokenFile::new(&link)
            .load()
            .expect_err("符号链接路径必须被拒");
        assert!(
            matches!(error, SecurityError::TokenPathIsSymlink { .. }),
            "实际错误: {error:?}"
        );
        fs::remove_dir_all(link.parent().expect("parent")).ok();
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

    /// 报告 ⑤ 的解析优先级逐级钉住（纯函数构造，不动进程环境变量）：
    /// `HOME` 优先；`HOME` 缺失走 `USERPROFILE`（Windows 默认 PowerShell 形状）；
    /// 两级全缺 fail-closed（不回退临时目录）；空串与缺失同等对待（空 `HOME` 会拼出相对路径）。
    #[test]
    fn home_resolution_follows_home_userprofile_order() {
        use std::ffi::OsString;
        let home = OsString::from("/home/someone");
        let profile = OsString::from("C:\\Users\\someone");

        assert_eq!(
            resolve_home_dir(Some(home.as_os_str()), Some(profile.as_os_str())),
            Some(PathBuf::from("/home/someone")),
            "HOME 在时优先 HOME"
        );
        assert_eq!(
            resolve_home_dir(None, Some(profile.as_os_str())),
            Some(PathBuf::from("C:\\Users\\someone")),
            "HOME 缺失时回退 USERPROFILE"
        );
        assert_eq!(
            resolve_home_dir(None, None),
            None,
            "两级全缺时 fail-closed: 不回退临时目录"
        );
        let empty = OsString::new();
        assert_eq!(
            resolve_home_dir(Some(empty.as_os_str()), Some(profile.as_os_str())),
            Some(PathBuf::from("C:\\Users\\someone")),
            "空 HOME 视为未设置"
        );
        assert_eq!(
            resolve_home_dir(Some(empty.as_os_str()), Some(empty.as_os_str())),
            None,
            "空 USERPROFILE 同样视为未设置 -> fail-closed"
        );
    }

    // -----------------------------------------------------------------------
    // T-FIX-32 回归判据：熵失败 / 无 HOME / symlink 替换，三条都必须 fail-closed
    // -----------------------------------------------------------------------

    /// 判据 ①：OS 熵源失败必须**拒绝生成**，绝不退回确定性弱令牌。
    /// 通过 `try_generate_with` 注入一个必然失败的 fill，断言原样返回
    /// `EntropyUnavailable`，且不会产出任何 `GeneratedToken`。
    #[test]
    fn entropy_failure_is_rejected_not_fallen_back() {
        let injected = |_buf: &mut [u8]| {
            Err(SecurityError::EntropyUnavailable(String::from(
                "注入的熵源故障",
            )))
        };
        let error = BearerToken::try_generate_with(injected)
            .expect_err("熵源失败必须返回错误，不能产出弱令牌");
        assert!(
            matches!(error, SecurityError::EntropyUnavailable(_)),
            "实际错误: {error:?}"
        );
    }

    /// 判据 ②：两级家目录都缺失 / 为空时必须**报错**，不落共享 `/tmp`。
    /// 通过 `home_dir_from` 注入，不动测试进程的真实环境变量。
    #[test]
    fn missing_home_is_rejected_without_tmp_fallback() {
        use std::ffi::OsString;
        let empty = OsString::new();
        for (home, profile, label) in [
            (None, None, "两级全缺"),
            (Some(empty.as_os_str()), None, "HOME 为空且无 USERPROFILE"),
        ] {
            let error = home_dir_from(home, profile).expect_err("无家目录必须被拒，绝不回退 /tmp");
            assert!(
                matches!(error, SecurityError::HomeDirectoryUnavailable),
                "{label}: 实际错误 {error:?}"
            );
        }
    }

    /// 判据 ③：`load` 依赖 `O_NOFOLLOW` 封住"lstat 通过后把路径换成 symlink"
    /// 的检查-打开窗口。这里直接钉住该机制：用与 `load` 相同的 flag 打开一个
    /// symlink 必须失败（Linux 上 `ELOOP`），从而 `open` 不会跟随到目标文件。
    /// 配合上面的 `a_symlinked_token_path_is_refused`（整条 load 路径拒绝 symlink），
    /// 覆盖"替换成 symlink → 被拒"这条判据。
    #[test]
    #[cfg(unix)]
    fn nofollow_open_refuses_symlink_swap() {
        use std::os::unix::fs::{OpenOptionsExt as _, symlink};

        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-nofollow-{}-{:x}",
            std::process::id(),
            TOKEN_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建目录");
        let target = dir.join("real.token");
        fs::write(&target, format!("{}\n", fixture_token(0x31).expose())).expect("写目标");
        let link = dir.join("swap.token");
        symlink(&target, &link).expect("建 symlink");

        // 这就是 load 第 2 步用的打开方式：O_NOFOLLOW 下打开 symlink 必须失败。
        let opened = fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW_FLAG)
            .open(&link);
        assert!(opened.is_err(), "O_NOFOLLOW 必须拒绝跟随 symlink");

        // 端到端：即使目标是一份完全合法的 0600 令牌，load 也拒绝这条路径。
        let error = TokenFile::new(&link)
            .load()
            .expect_err("symlink 路径必须被拒");
        assert!(
            matches!(error, SecurityError::TokenPathIsSymlink { .. }),
            "实际错误: {error:?}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// [PATCH] Windows：令牌文件**可用**（ACL 等价 `0600`），不再报
    /// `UnsupportedPlatform`。这里钉住三件事：真的写得进去、读回来是同一枚令牌、
    /// 落盘后的 ACL 里没有任何宽主体。
    #[test]
    #[cfg(windows)]
    fn token_file_roundtrips_with_owner_only_acl_on_windows() {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-token-win-{}-{:x}",
            std::process::id(),
            TOKEN_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let file = TokenFile::new(dir.join(TOKEN_FILE_NAME));
        let token = fixture_token(0x5a);
        file.save(&token).expect("Windows 上应当写得进去");
        assert!(super::win_acl::is_owner_only(file.path()).expect("读回 ACL"));
        let loaded = file.load().expect("Windows 上应当读得回来");
        assert_eq!(loaded.expose(), token.expose());
        let _ = std::fs::remove_dir_all(&dir);
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
