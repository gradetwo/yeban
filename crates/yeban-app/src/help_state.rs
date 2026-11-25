//! 帮助系统状态：新手引导的「不再显示 / 已完成」与教学课程的完成进度。
//!
//! # 存储（line/fix-onboarding-settings 起并入 settings）
//!
//! 帮助状态**不再**另立文件：它以 `help` 键存在 settings 文件
//! （`yeban/settings.json`，见 [`crate::settings`]）里，与语言、窗口布局、
//! 最近工程共用一份设置存储。`load` / `save` / `load_from` / `save_to`
//! 的 API 不变，但语义改为读-改-写 settings 文件 —— 每次写回都先读出
//! 现有设置（保住语言 / 布局 / 最近工程），再改 `help` 字段。
//!
//! 落盘纪律改随 settings：写临时文件再 rename，不留半截文件
//! （旧独立文件的 0700/0600 权限纪律随之退役，settings 文件不设
//! 特殊权限 —— 与语言、布局同一待遇）。
//!
//! # 旧文件折入
//!
//! 旧独立文件 `~/.yeban/onboarding.json`（`state_path` 定位）在首次读取
//! 时折入：settings 里已有非默认帮助状态时以 settings 为准；settings
//! 还是默认时采用旧文件状态，并立即把折入结果写回 settings，此后以
//! settings 为准（折入规则见 [`fold_legacy_state`]）。旧文件本身不删
//! （不做破坏性操作），只作一次性迁移源。用户真实状态不会因折入丢失：
//! 折入只发生在 settings 尚无状态时，且折入后立即落盘。
//!
//! # 测试隔离
//!
//! `YEBAN_SETTINGS_PATH`（settings 的路径覆盖）优先于默认设置路径；
//! `YEBAN_ONBOARDING_FILE` 只用来定位**旧文件**（折入测试用），
//! 不再是帮助状态的主存储路径。判据同时设这两个变量指到临时文件，
//! 绝不碰真实配置目录与家目录。

use std::collections::BTreeMap;
use std::path::PathBuf;

/// 状态格式版本。本线写 `1`；读到更高版本按未知处理（回默认，不覆盖）。
pub const HELP_STATE_VERSION: u32 = 1;

/// 旧状态目录名（与 `local_config` / MCP 令牌目录同为 `~/.yeban`）。
pub const HELP_STATE_DIR_NAME: &str = ".yeban";

/// 旧状态文件名。
pub const HELP_STATE_FILE_NAME: &str = "onboarding.json";

/// 旧文件路径覆盖环境变量（仅折入测试用，见模块文档）。
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

/// 状态存储的错误面。读不到 / 解析失败**不**是错误（见 [`load`] 的回退
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

/// 旧独立状态文件路径：环境变量覆盖优先，其次 `~/.yeban/onboarding.json`。
/// 只在折入时用（见模块文档），不是帮助状态的主存储路径。
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

/// 从旧独立文件读状态（折入专用）：文件不存在 / 内容损坏 / 版本更高 ⇒
/// 回默认状态（旧文件损坏时就当它从没写过状态，不因此挡住折入）。
#[must_use]
fn load_legacy_file(path: &std::path::Path) -> HelpState {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HelpState::default();
    };
    match serde_json::from_str::<HelpState>(&text) {
        Ok(state) if state.version <= HELP_STATE_VERSION => state,
        _ => HelpState::default(),
    }
}

/// 旧文件状态的折入口径（纯函数）：settings 里已有非默认状态时以
/// settings 为准；settings 还是默认时采用旧文件状态。
#[must_use]
pub fn fold_legacy_state(current: &HelpState, legacy: &HelpState) -> HelpState {
    if *current != HelpState::default() {
        current.clone()
    } else {
        legacy.clone()
    }
}

/// 从 settings 文件读帮助状态。文件不存在 / 解析失败 ⇒ 默认状态
/// （首次启动与文件损坏在用户眼里没有区别：都该看到引导）。
#[must_use]
pub fn load_from(path: &std::path::Path) -> HelpState {
    crate::settings::load_from(path).help
}

/// 读当前生效的帮助状态：settings 文件的 `help` 键为准；settings 里还
/// 没有状态时，把旧独立文件的状态折入并写回 settings（见模块文档），
/// 此后以 settings 为准。
#[must_use]
pub fn load() -> HelpState {
    let settings = crate::settings::load();
    if settings.help != HelpState::default() {
        return settings.help;
    }
    let Ok(legacy_path) = state_path() else {
        return settings.help;
    };
    let folded = fold_legacy_state(&settings.help, &load_legacy_file(&legacy_path));
    if folded != settings.help {
        // 折入落盘失败不挡读取：本次先按折入结果走，下次启动再试；但失败
        // 必须让用户看见（T-FIX-299）：记进设置警告台账，UI 侧取走展示。
        // 走 settings::update 的锁内读-改-写，避免与另一实例的设置写回互相覆盖。
        if let Err(error) = crate::settings::update(|settings| settings.help = folded.clone()) {
            crate::settings::record_save_failure("帮助状态折入", &error);
        }
    }
    folded
}

/// 把帮助状态写到 settings 文件（读-改-写：先读出现有设置，保住语言 /
/// 布局 / 最近工程，再改 `help` 写回；落盘纪律见 [`crate::settings`]）。
///
/// # Errors
/// 读-改-写中的 IO 失败时返回 [`HelpStateError::Io`]。
pub fn save_to(path: &std::path::Path, state: &HelpState) -> Result<(), HelpStateError> {
    crate::settings::update_at(path, |settings| settings.help = state.clone())
        .map(|_| ())
        .map_err(HelpStateError::Io)
}

/// 写当前生效的 settings 文件里的帮助状态（读-改-写，见 [`save_to`]）。
///
/// # Errors
/// 落盘失败时返回 [`HelpStateError::Io`]。
pub fn save(state: &HelpState) -> Result<(), HelpStateError> {
    crate::settings::update(|settings| settings.help = state.clone())
        .map(|_| ())
        .map_err(HelpStateError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 折入纯函数：settings 已有状态 ⇒ 以 settings 为准，旧文件状态丢弃；
    /// settings 还是默认 ⇒ 采用旧文件状态；两边都默认 ⇒ 仍是默认。
    #[test]
    fn fold_legacy_state_prefers_settings_then_legacy() {
        let default = HelpState::default();
        let dismissed = HelpState {
            onboarding_dismissed: true,
            ..HelpState::default()
        };
        let finished = HelpState {
            onboarding_finished: true,
            ..HelpState::default()
        };

        assert_eq!(fold_legacy_state(&dismissed, &finished), dismissed);
        assert_eq!(fold_legacy_state(&default, &finished), finished);
        assert_eq!(fold_legacy_state(&default, &default), default);
    }

    /// 旧文件解析回退：不存在 / 损坏 / 未来版本 ⇒ 默认。
    #[test]
    fn legacy_file_falls_back_to_default_when_unreadable() {
        let dir =
            std::env::temp_dir().join(format!("yeban-help-legacy-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let missing = dir.join("missing.json");
        assert_eq!(load_legacy_file(&missing), HelpState::default());
        let corrupt = dir.join("corrupt.json");
        std::fs::write(&corrupt, "{not json").expect("写坏文件");
        assert_eq!(load_legacy_file(&corrupt), HelpState::default());
        let future = dir.join("future.json");
        std::fs::write(&future, "{\"version\": 99}").expect("写未来版本");
        assert_eq!(load_legacy_file(&future), HelpState::default());
        let good = dir.join("good.json");
        std::fs::write(&good, "{\"version\": 1, \"onboarding_dismissed\": true}")
            .expect("写旧格式");
        assert!(load_legacy_file(&good).onboarding_dismissed);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
