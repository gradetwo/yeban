//! 帮助系统状态存储：新手引导的「不再显示 / 已完成」与教学课程的完成进度。
//!
//! # 选型（line/onboarding-help 普查结论）
//!
//! `main` 上的 yeban-app 没有用户级设置存储（无 config 目录、无首启概念）。
//! 仓内唯一的同类先例是模型层 `yeban_model::local_config::LocalMachineConfig`
//! （`~/.yeban/config.json`）：它的字段集由规范钉死为本机绑定
//! （声卡 / 外部编辑器 / 密钥引用），引导与课程状态不在其中 ⇒ ⛔ 不往里塞。
//! 本模块在**同一目录** `~/.yeban/` 下另立 `onboarding.json`，并逐字沿用
//! 那份先例的落盘纪律：
//!
//! * 目录权限 0700、文件权限 0600（Unix），不依赖 umask；
//! * 写入序：建目录 → 以 0600 打开 → `write_all` → `sync_all` → 权限复核；
//! * 序列化：`deny_unknown_fields` + 字段全带默认值（旧文件缺字段不判死），
//!   映射用 `BTreeMap` 保证字节确定性；
//! * 家目录解析：`HOME` → `USERPROFILE`，解析不到即报错，⛔ 不回退临时目录。
//!
//! 测试隔离：环境变量 `YEBAN_ONBOARDING_FILE` 给出完整路径时优先用它
//! （判据用临时文件验往返，不碰真实家目录）。

use std::collections::BTreeMap;
use std::path::PathBuf;

/// 状态文件格式版本。本线写 `1`；读到更高版本按未知处理（回默认，不覆盖）。
pub const HELP_STATE_VERSION: u32 = 1;

/// 状态目录名（与 `local_config` / MCP 令牌目录同为 `~/.yeban`）。
pub const HELP_STATE_DIR_NAME: &str = ".yeban";

/// 状态文件名。
pub const HELP_STATE_FILE_NAME: &str = "onboarding.json";

/// 测试隔离用的路径覆盖环境变量。
pub const HELP_STATE_PATH_ENV: &str = "YEBAN_ONBOARDING_FILE";

/// 一节课的持久化进度。
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LessonProgress {
    /// 已通过判定的步数（只由判定通过推进，见 `lessons::evaluate`）。
    #[serde(default)]
    pub steps_done: u32,
    /// 整节课是否已完成。
    #[serde(default)]
    pub finished: bool,
}

/// 正在进行中的一节课（判定基准随它一起持久化：关掉面板 / 重启应用，
/// 「相对开课时新增」的判定口径不丢）。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveLesson {
    /// 课程 id（`lessons::CATALOG` 里的 id）。
    pub lesson_id: String,
    /// 开课那一刻工程里已有的轨道 id 集合（规范文本形式）。
    /// 完成判定只认这个集合**之外**的新轨道，演示工程自带的音符不会假过。
    #[serde(default)]
    pub baseline_track_ids: Vec<String>,
    /// 当前步序号（0 起）。
    #[serde(default)]
    pub current_step: u32,
}

/// 帮助系统的全部持久化状态。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelpState {
    /// 格式版本，见 [`HELP_STATE_VERSION`]。
    pub version: u32,
    /// 用户在引导里点过「不再显示」。
    #[serde(default)]
    pub onboarding_dismissed: bool,
    /// 用户完整走完过一遍引导。
    #[serde(default)]
    pub onboarding_finished: bool,
    /// 逐课进度，键 = 课程 id。
    #[serde(default)]
    pub lessons: BTreeMap<String, LessonProgress>,
    /// 进行中的课程（同一时刻至多一节）。
    #[serde(default)]
    pub active_lesson: Option<ActiveLesson>,
}

impl Default for HelpState {
    fn default() -> Self {
        Self {
            version: HELP_STATE_VERSION,
            onboarding_dismissed: false,
            onboarding_finished: false,
            lessons: BTreeMap::new(),
            active_lesson: None,
        }
    }
}

impl HelpState {
    /// 首次启动是否应自动弹出引导：没被「不再显示」关掉、也没走完过。
    #[must_use]
    pub fn should_auto_show_onboarding(&self) -> bool {
        !self.onboarding_dismissed && !self.onboarding_finished
    }

    /// 某节课的进度（没开始过 ⇒ 零进度）。
    #[must_use]
    pub fn lesson_progress(&self, lesson_id: &str) -> LessonProgress {
        self.lessons.get(lesson_id).cloned().unwrap_or_default()
    }
}

/// 状态存储的错误面。读不到 / 解析失败**不**是错误（见 [`load_from`] 的回退
/// 语义）；只有落盘失败与家目录不可知才报错，且调用方一律降级为
/// 「本次会话不持久化」，⛔ 不许因此挡住应用启动。
#[derive(Debug)]
pub enum HelpStateError {
    /// 家目录不可知（`HOME` 与 `USERPROFILE` 都没有）。
    HomeUnknown,
    /// 落盘 IO 失败。
    Io(std::io::Error),
    /// 序列化失败（结构上不应发生，留作响亮失败面）。
    Serialize(serde_json::Error),
}

impl std::fmt::Display for HelpStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HomeUnknown => write!(f, "无法确定用户家目录，帮助状态不落盘"),
            Self::Io(error) => write!(f, "帮助状态落盘失败：{error}"),
            Self::Serialize(error) => write!(f, "帮助状态序列化失败：{error}"),
        }
    }
}

impl std::error::Error for HelpStateError {}

/// 解析状态文件路径：环境变量覆盖优先，其次 `~/.yeban/onboarding.json`。
///
/// # Errors
/// 家目录不可知时返回 [`HelpStateError::HomeUnknown`]。
pub fn state_path() -> Result<PathBuf, HelpStateError> {
    if let Some(path) = std::env::var_os(HELP_STATE_PATH_ENV)
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    let home = home_dir().ok_or(HelpStateError::HomeUnknown)?;
    Ok(home.join(HELP_STATE_DIR_NAME).join(HELP_STATE_FILE_NAME))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
        .map(PathBuf::from)
}

/// 从指定路径读状态。文件不存在 / 内容损坏 / 版本更高 ⇒ 回默认状态
/// （首次启动与文件损坏在用户眼里没有区别：都该看到引导）。
#[must_use]
pub fn load_from(path: &std::path::Path) -> HelpState {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HelpState::default();
    };
    match serde_json::from_str::<HelpState>(&text) {
        Ok(state) if state.version <= HELP_STATE_VERSION => state,
        _ => HelpState::default(),
    }
}

/// 读当前生效路径的状态（路径不可知 ⇒ 默认状态）。
#[must_use]
pub fn load() -> HelpState {
    match state_path() {
        Ok(path) => load_from(&path),
        Err(_) => HelpState::default(),
    }
}

/// 把状态写到指定路径（落盘纪律见模块头）。
///
/// # Errors
/// 目录创建 / 写入 / 同步失败时返回 [`HelpStateError::Io`]。
pub fn save_to(path: &std::path::Path, state: &HelpState) -> Result<(), HelpStateError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(HelpStateError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let mut text = serde_json::to_string_pretty(state).map_err(HelpStateError::Serialize)?;
    text.push('\n');
    write_private_file(path, text.as_bytes())
}

fn write_private_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), HelpStateError> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(HelpStateError::Io)?;
    file.write_all(bytes).map_err(HelpStateError::Io)?;
    file.sync_all().map_err(HelpStateError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(HelpStateError::Io)?;
    }
    Ok(())
}

/// 写当前生效路径的状态。
///
/// # Errors
/// 路径不可知或落盘失败时返回 [`HelpStateError`]。
pub fn save(state: &HelpState) -> Result<(), HelpStateError> {
    let path = state_path()?;
    save_to(&path, state)
}
