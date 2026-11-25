//! 最小设置存储（2026-11-17，i18n 语言持久化，Wang 05:57 令；
//! 同日 line/ui-window-state 扩入窗口布局，仍是同一个文件、同一个模块；
//! 同日 line/startup-chooser 再扩入最近打开工程清单，仍是同一份文件；
//! 同日 line/fix-onboarding-settings 再扩入帮助系统状态 —— 新手引导线
//! 原先另立的 `~/.yeban/onboarding.json` 就此并入，全应用只此一份
//! 设置存储，旧文件的折入见 `help_state` 模块）。
//!
//! 存六样用户偏好：界面语言（显式选过才存）、窗口布局（退出时快照）、
//! 最近打开工程（打开时记录，启动选择器的「最近」分类读它）、帮助系统
//! 状态（新手引导的「不再显示 / 已完成」与教学课程进度）、调试输出
//! 开关（T-FIX-70，主菜单「设置 → 调试输出」写回，`--debug` /
//! `--log-file` 是同开关的命令行入口，决策见 [`resolve_debug_output`]）
//! 与音频设备选择（T-FIX-71，「设置 → 音频」面板写回，存的是引擎枚举
//! 的设备显示名，`None` = 该方向用系统默认设备）。
//! 语言的启动优先级（定案）是 `--lang` 参数（临时覆盖、不写回）＞
//! 已存选择 ＞ 系统 locale 自动检测 ＞ zh-CN，决策函数见
//! [`resolve_startup_language`]，单测钉死。布局的恢复/写回/重置语义在
//! `host::apply_window_layout` / `host::save_window_layout` /
//! `host::reset_window_layout`，启动时 `main.rs` 在语言之后应用。
//!
//! 选址：仓内**没有**既有设置路径先例（全仓 grep `config_dir` / `settings.json`
//! 零命中），故新建平台配置目录下 `yeban/settings.json`：
//! Linux `$XDG_CONFIG_HOME/yeban/settings.json`（未设则 `$HOME/.config/yeban/`），
//! macOS `$HOME/Library/Application Support/yeban/settings.json`，
//! Windows `%APPDATA%/yeban/settings.json`。目录名全平台统一小写 `yeban`
//!（与仓库名、CLI 二进制名一致），不随平台改大小写。
//!
//! 格式：`{"version": 1, "language": "fr-FR", "layout": {...}}`。
//! `version` 仍为 1 —— 演进口径是**在 version 1 下加可选键**：读端只取
//! 自己认识的键，未知键忽略。于是旧构建（只认 `language`）读新文件时
//! 布局键被忽略、语言选择不丢；新构建读旧文件时 `layout` 缺失 ⇒ 没有
//! 已存布局。`help` 键同理（见 [`parse_help_state`] 的逐字段容错，
//! 只有它自带的 `version` 高于本构建所识时整份回默认 —— 那是旧独立
//! 文件留下的语义）。读不到（文件不存在）⇒ 当作"什么都没存过"，不报错、
//! 不崩溃（设置文件是偏好，不是工程数据）；读坏（读不出 / 解析失败 /
//! 顶层不是对象）⇒ 同样回默认、不崩溃，但记一条待发警告（UI 侧展示，
//! 见 [`take_pending_warnings`]），损坏文件先备份再丢弃（T-FIX-299）。
//!
//! 布局（[`WindowLayout`]）逐字段容错：单字段类型错 / 越界只让该字段
//! 落回默认值（尺寸按 .slint 拖拽同界钳位、标签页只认闭集），不连坐
//! 整份布局；窗口几何是例外 —— 四个数任一不合逻辑就整份丢弃（半份
//! 几何可能把窗口丢到屏幕外）。纳入哪些真实布局量、普查后排除哪些，
//! 见 `docs/app-i18n-notes.md` §11。
//!
//! 测试隔离：设置了 `YEBAN_SETTINGS_PATH` 时 load/save 走该路径（单测与
//! example 用它指到临时目录，绝不碰真用户配置；生产路径永不设此变量）。

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cli::Language;
use crate::help_state::HelpState;

/// 设置文件在配置目录下的相对路径（目录名 + 文件名）。
pub const SETTINGS_DIR_NAME: &str = "yeban";
/// 设置文件名（与目录名拼成 `yeban/settings.json`）。
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// 测试隔离环境变量（见模块文档）。
pub const SETTINGS_PATH_ENV: &str = "YEBAN_SETTINGS_PATH";

/// 同一进程内的临时文件序号；跨进程的部分由 pid + 毫秒补齐。
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 设置文件的跨进程锁文件：`settings.json.lock`（与目标文件同目录）。
fn settings_lock_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map_or_else(
        || std::ffi::OsString::from(SETTINGS_FILE_NAME),
        |name| name.to_os_string(),
    );
    name.push(".lock");
    path.with_file_name(name)
}

/// 在设置文件的跨进程排他锁内执行一次动作。锁覆盖完整的 load → mutate → save，
/// 而不是只覆盖 rename 那一跳；锁文件与目标文件同目录，进程崩溃时由 OS 释放。
fn with_settings_lock<T>(path: &Path, action: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = settings_lock_path(path);
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    lock_file.lock()?;
    let result = action();
    let unlock = lock_file.unlock();
    match result {
        Ok(value) => unlock.map(|()| value),
        Err(error) => {
            // 动作已失败时优先返回动作错误；解锁错误不覆盖它。
            let _ = unlock;
            Err(error)
        }
    }
}

/// 生成唯一临时文件路径：`settings.json.tmp.<pid>.<毫秒>.<进程内序号>`。
/// 固定名 `settings.json.tmp` 会让两个写者互相覆盖。
fn unique_temp_path(path: &Path) -> PathBuf {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let seq = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().map_or_else(
        || std::ffi::OsString::from(SETTINGS_FILE_NAME),
        |name| name.to_os_string(),
    );
    name.push(format!(".tmp.{}.{millis}.{seq}", std::process::id()));
    path.with_file_name(name)
}

/// 一条待展示的设置警告（T-FIX-299）。
///
/// 设置写回失败 / 设置文件损坏时，设置模块只把失败记在这里 —— 调用方
/// 不失败（设定已在本会话生效），UI 侧由 [`take_pending_warnings`]
/// 取走并经 `host::report_notice` 以错误级常驻提示展示。无窗口路径
/// （CLI / headless）下它只留在 stderr 行里（见 [`record_save_failure`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsWarning {
    /// 展示给用户的完整文案（含“本次已生效、下次启动会回退”与原因）。
    pub message: String,
}

/// 待发设置警告台账（进程内；取走即清空，见 [`take_pending_warnings`]）。
static PENDING_WARNINGS: Mutex<Vec<SettingsWarning>> = Mutex::new(Vec::new());

fn record_warning(message: String) {
    let mut pending = PENDING_WARNINGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    pending.push(SettingsWarning { message });
}

/// 取走全部待发设置警告（取后台账清空）。UI 侧在每次设置写回动作之后
/// 与启动建窗后调一次；无警告时返回空。
#[must_use]
pub fn take_pending_warnings() -> Vec<SettingsWarning> {
    let mut pending = PENDING_WARNINGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *pending)
}

/// 把一次设置写回失败记进待发警告（同时保留 stderr 行：无窗口 / CLI
/// 路径的唯一可见处）。调用方不失败 —— 设定已在本会话生效，只是下次
/// 启动会回到旧值（T-FIX-299：此前这里只打印一行 stderr，桌面双击启动
/// 的用户看不到终端，下次启动静默回退）。
pub fn record_save_failure(what: &str, error: &io::Error) {
    let message = format!("{what}写回设置失败：已在本会话生效，但下次启动会回到旧值（{error}）");
    eprintln!("[yeban-app] {message}");
    record_warning(message);
}

/// 把一份已损坏的设置文件备份到同目录（`settings.json.corrupt-<毫秒>`），
/// 返回备份路径；rename 与 copy 都失败时返回 `None`（原文件一位未动）。
fn backup_corrupt_settings_file(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let file_name = path.file_name()?.to_string_lossy().into_owned();
    let backup = path.with_file_name(format!("{file_name}.corrupt-{millis}"));
    if std::fs::rename(path, &backup).is_ok() {
        return Some(backup);
    }
    // rename 失败（例如同名备份已存在）：退回 copy，拷走再删原文件。
    if std::fs::copy(path, &backup).is_ok() {
        let _ = std::fs::remove_file(path);
        return Some(backup);
    }
    None
}

/// 设置内容：`language`（i18n 线）＋ `layout`（窗口布局线）＋
/// `recent_projects`（启动选择器线）＋ `help`（新手引导线，存储并入）＋
/// `debug_output`（T-FIX-70 调试输出线），同一份文件。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// 用户显式选过的语言；`None` = 从未存过（首次启动走自动检测）。
    pub language: Option<Language>,
    /// 已存的窗口布局；`None` = 从未存过（启动时不恢复，界面保持
    /// .slint 初值 —— 含 persona 派生的面板显隐初值，不被强行覆盖）。
    pub layout: Option<WindowLayout>,
    /// 最近打开过的工程（最近的在最前，上限
    /// [`RECENT_PROJECTS_MAX`]）；空 = 从未记录过。本机配置
    /// （MODEL-ISO-001 第三层），不进工程文件。
    pub recent_projects: Vec<RecentProject>,
    /// 帮助系统状态（新手引导与课程进度，类型与语义见
    /// [`HelpState`]）；`HelpState::default()` = 从未存过。
    pub help: HelpState,
    /// 调试输出开关（主菜单「设置 → 调试输出」的已存值，T-FIX-70）；
    /// `false` = 从未开过（键缺失与存 `false` 同义，写回时只在 `true`
    /// 时落键）。启动决策见 [`resolve_debug_output`]。
    pub debug_output: bool,
    /// 用户在「设置 → 音频」里选定的**输出**设备显示名（T-FIX-71）；
    /// `None` = 从未选过（或选回了"系统默认"），启动时用系统默认输出
    /// 设备（现行为）。名字来自引擎枚举的 `DeviceInfo::name`，按名精确
    /// 命中；设备不在场时引擎**响亮报错点名**，⛔ 不静默换默认（见
    /// `yeban_engine::device::open_output_boxed_named`）。
    pub audio_output_device: Option<String>,
    /// 用户选定的**输入**设备显示名（T-FIX-71）：与输出侧同款语义，
    /// `None` = 系统默认输入设备。
    pub audio_input_device: Option<String>,
    /// T-FIX-157：编曲视图的跟随模式。`false`（默认）= Logic 经典 Catch
    /// （播放头贴左缘、越界整页跳）；`true` = Scroll in Play（播放头钉中央、
    /// 视图逐跳平滑滚动）。键缺失与 `false` 同义（默认行为不变）。
    pub scroll_in_play: bool,
}

/// 一条「最近打开工程」记录：路径 + 最近一次打开时刻。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentProject {
    /// 工程文件的**规范化绝对路径**（最近清单的身份键）：显示名由消费方
    /// 从路径派生，⛔ 不在这里另存一份会漂移的名字。
    pub path: std::path::PathBuf,
    /// 最近一次打开的时刻（Unix 毫秒）。
    pub opened_at_unix_ms: u64,
}

/// 最近清单的上限：超出时从尾部（最旧）丢弃。
pub const RECENT_PROJECTS_MAX: usize = 10;

/// T-FIX-310：路径身份统一为**规范化绝对路径**。
///
/// 最近清单是"同一份工程只占一格"的集合 ⇒ 去重键必须先绝对化、再尽量
/// canonicalize（消掉 `./`、`../`、符号链接与相对路径的写法差异）。
/// 工程文件此时通常已存在；canonicalize 失败（例如条目已失效）退回
/// `std::path::absolute`，它仍会得到绝对路径而不碰文件系统。
/// 显示名**不**参与这里：它由消费方从 `path` 派生。
fn recent_project_identity(path: &std::path::Path) -> std::path::PathBuf {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// 把一次打开记进最近清单（纯函数）：同路径旧记录先摘掉，新记录插到
/// 最前，超出上限从尾部丢弃。顺序即显示顺序（最近在前）。
#[must_use]
pub fn touch_recent_project(
    list: &[RecentProject],
    path: &std::path::Path,
    opened_at_unix_ms: u64,
) -> Vec<RecentProject> {
    let path = recent_project_identity(path);
    let mut out: Vec<RecentProject> = Vec::with_capacity(list.len() + 1);
    out.push(RecentProject {
        path: path.clone(),
        opened_at_unix_ms,
    });
    for entry in list {
        if recent_project_identity(&entry.path) != path {
            out.push(entry.clone());
        }
    }
    out.truncate(RECENT_PROJECTS_MAX);
    out
}

/// 窗口几何（**物理像素**，与 `slint::Window::size` / `position` 的口径
/// 一致，存取往返不经逻辑像素换算 ⇒ 与显示缩放无关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowGeometry {
    /// 窗口左上角 x（物理像素，可为负 —— 多显示器时副屏在主屏左侧）。
    pub x: i32,
    /// 窗口左上角 y（物理像素）。
    pub y: i32,
    /// 窗口宽（物理像素）。最大化时这是最大化后的尺寸（Slint 不暴露
    /// 还原几何），此时以 `maximized` 为准，宽高只作参考。
    pub width: u32,
    /// 窗口高（物理像素），口径同 `width`。
    pub height: u32,
    /// 退出时是否最大化。
    pub maximized: bool,
}

/// 五条面板尺寸的钳位界（逻辑像素）：与 `ui/app.slint` 拖拽处理器里的
/// `Math.min/max` 同值 —— 拖拽改不到界外，读回的坏值也钳回界内。
/// 两处同值由 `tests/window_layout_state.rs` 的源码对账钉住。
pub const LEFT_PANEL_WIDTH_RANGE: (f32, f32) = (200.0, 380.0);
/// 同 [`LEFT_PANEL_WIDTH_RANGE`]。
pub const CLIP_MATRIX_WIDTH_RANGE: (f32, f32) = (180.0, 360.0);
/// 同 [`LEFT_PANEL_WIDTH_RANGE`]。
pub const CONTROL_ROOM_WIDTH_RANGE: (f32, f32) = (160.0, 300.0);
/// 同 [`LEFT_PANEL_WIDTH_RANGE`]。
pub const LOWER_DOCK_HEIGHT_RANGE: (f32, f32) = (180.0, 480.0);
/// 同 [`LEFT_PANEL_WIDTH_RANGE`]。
pub const TRACK_HEADERS_WIDTH_RANGE: (f32, f32) = (180.0, 320.0);

/// 窗口布局快照：字段与 `ui/app.slint` 的 `MainWindow` 属性一一对应
///（宿主的写/读在 `host::apply_window_layout` /
/// `host::snapshot_window_layout`）。这是**本机配置**（MODEL-ISO-001
/// 的第三层），不进工程文件、不进撤销栈。
#[derive(Debug, Clone, PartialEq)]
pub struct WindowLayout {
    /// 窗口几何；`None` = 没有可恢复的几何（快照恒为 `Some`，
    /// `None` 只出现在默认值与几何被判坏丢弃时）。
    pub window: Option<WindowGeometry>,
    /// 左栏（检查器/音色库）宽，逻辑像素。
    pub left_panel_width: f32,
    /// Clip 激发矩阵抽屉宽，逻辑像素。
    pub clip_matrix_width: f32,
    /// 监听控制室栏宽，逻辑像素。
    pub control_room_width: f32,
    /// 底部统一坞高，逻辑像素。
    pub lower_dock_height: f32,
    /// 编曲区轨道头列宽，逻辑像素。
    pub track_headers_width: f32,
    /// 左栏检查器是否显示。
    pub show_left_inspector: bool,
    /// Clip 激发矩阵是否显示。
    pub show_clip_matrix: bool,
    /// 底部统一坞是否显示。
    pub show_lower_dock: bool,
    /// 监听控制室是否显示。
    pub show_control_room: bool,
    /// 左栏标签页，闭集 `"inspector" | "browser"`。
    pub left_panel_tab: String,
    /// 底部控制台标签页（0 卷帘 / 1 混音 / 2 设备 / 3 堆叠，
    /// 与 .slint 的 `console-tab` 映射一致）。
    pub console_tab: i32,
    /// 底部控制台是否最大化（占满下半区）。
    pub console_maximized: bool,
}

impl Default for WindowLayout {
    /// .slint 初值：`ui/app.slint` 里 `MainWindow` 各属性的默认值
    ///（显隐两项 persona 派生量取默认 persona（Yeban）的初值：
    /// `show-clip-matrix` 与 `show-control-room` 均为 false）。
    /// 「恢复默认布局」应用的就是这一份；与 .slint 的对账由
    /// `tests/window_layout_state.rs` 钉住。
    fn default() -> Self {
        WindowLayout {
            window: None,
            left_panel_width: 256.0,
            clip_matrix_width: 240.0,
            control_room_width: 200.0,
            lower_dock_height: 320.0,
            track_headers_width: 240.0,
            show_left_inspector: true,
            show_clip_matrix: false,
            show_lower_dock: true,
            show_control_room: false,
            left_panel_tab: "inspector".to_owned(),
            console_tab: 0,
            console_maximized: false,
        }
    }
}

/// 平台配置目录（`dirs` 风格，不引新依赖：三个平台各三行 env 读法）。
fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
            && !xdg.is_empty()
        {
            return Some(PathBuf::from(xdg));
        }
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
    }
}

/// 设置文件路径：测试隔离变量优先，否则平台配置目录下
/// `yeban/settings.json`；连 `HOME` 都读不到时落到当前目录的
/// `.config/yeban/settings.json`（桌面应用正常启动不会走到这里）。
#[must_use]
pub fn settings_path() -> PathBuf {
    if let Some(overridden) = std::env::var_os(SETTINGS_PATH_ENV)
        && !overridden.is_empty()
    {
        return PathBuf::from(overridden);
    }
    config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join(SETTINGS_DIR_NAME)
        .join(SETTINGS_FILE_NAME)
}

/// 从 `path` 读设置：文件不存在 ⇒ 默认设置（全 `None`），调用方按
/// “什么都没存过”处理；文件在但读不出来 / 解析失败 / 顶层不是对象 ⇒
/// 同样回默认、不崩溃，但记一条待发警告（UI 侧展示，见
/// [`take_pending_warnings`]），损坏文件先备份到同目录再丢弃
/// （T-FIX-299：此前读坏也静默回默认，下次启动用户只看到旧选择被清空）。
/// 单字段的容错见 [`parse_layout`] 与 `language` 的九语闭集校验。
#[must_use]
pub fn load_from(path: &std::path::Path) -> Settings {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // 首次启动：没有文件就是“什么都没存过”，不警告。
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Settings::default();
        }
        Err(error) => {
            // 文件在但读不出来（权限等）：原文件一位未动，记警告。
            record_warning(format!(
                "设置文件无法读取（{}），已用默认设置，原文件未改动：{error}",
                path.display()
            ));
            return Settings::default();
        }
    };
    let value = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => value,
        Err(error) => {
            let backup = backup_corrupt_settings_file(path);
            let message = match &backup {
                Some(backup) => format!(
                    "设置文件已损坏（{}），已用默认设置，原文件已备份到 {}。",
                    path.display(),
                    backup.display()
                ),
                None => format!(
                    "设置文件已损坏（{}），已用默认设置，原文件备份失败，原文件未动：{error}",
                    path.display()
                ),
            };
            record_warning(message);
            return Settings::default();
        }
    };
    if !value.is_object() {
        // 顶层不是对象：保存端永远写对象，见到的只能是损坏文件。
        let backup = backup_corrupt_settings_file(path);
        let message = match &backup {
            Some(backup) => format!(
                "设置文件已损坏（{}），已用默认设置，原文件已备份到 {}。",
                path.display(),
                backup.display()
            ),
            None => format!(
                "设置文件已损坏（{}），已用默认设置，原文件备份失败，原文件未动。",
                path.display()
            ),
        };
        record_warning(message);
        return Settings::default();
    }
    Settings {
        language: value
            .get("language")
            .and_then(|name| name.as_str())
            .and_then(Language::from_name),
        layout: parse_layout(&value),
        recent_projects: parse_recent_projects(&value),
        help: parse_help_state(&value),
        // 键缺失 / 不是布尔 ⇒ 没开过（逐字段容错，与本文件其余键同款）。
        debug_output: value
            .get("debug_output")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        // 键缺失 / 不是字符串 / 空串 ⇒ 没选过（逐字段容错，同上）。
        // 空串必须当没选过：空名字不可能精确命中任何设备，留着只会让
        // 每次启动都撞一遍"指定设备不在场"的响亮错误。
        audio_output_device: parse_audio_device_name(&value, "audio_output_device"),
        audio_input_device: parse_audio_device_name(&value, "audio_input_device"),
        // T-FIX-157：键缺失 / 非布尔 ⇒ 默认经典 Catch（行为不变）。
        scroll_in_play: value
            .get("scroll_in_play")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    }
}

/// 解析一个音频设备名键（T-FIX-71）：是字符串且非空 ⇒ `Some`；
/// 其余（键缺失 / 类型不对 / 空串）⇒ `None`（= 用系统默认设备）。
fn parse_audio_device_name(root: &serde_json::Value, key: &str) -> Option<String> {
    root.get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// 把一条最近工程记录写成 JSON：UTF-8 路径保留可读的 `path`；
/// Unix 上另外写 `path_bytes`（`OsStrExt::as_bytes`），保证非 UTF-8 路径
/// 可**逐字节**回读。两者同时存在时，解析器优先 `path_bytes`。
#[cfg(unix)]
fn recent_project_to_json(entry: &RecentProject) -> Option<serde_json::Value> {
    use std::os::unix::ffi::OsStrExt;
    let mut object = serde_json::Map::new();
    if let Some(text) = entry.path.to_str() {
        object.insert(
            "path".to_owned(),
            serde_json::Value::String(text.to_owned()),
        );
    }
    object.insert(
        "path_bytes".to_owned(),
        serde_json::Value::Array(
            entry
                .path
                .as_os_str()
                .as_bytes()
                .iter()
                .map(|byte| serde_json::json!(byte))
                .collect(),
        ),
    );
    object.insert(
        "opened_at_unix_ms".to_owned(),
        serde_json::json!(entry.opened_at_unix_ms),
    );
    Some(serde_json::Value::Object(object))
}

/// 非 Unix 平台没有可移植的 `OsStr` 字节表示 ⇒ 只写 UTF-8 `path`；
/// 非 UTF-8 路径在这些平台上本来也无法由原生 picker 产生。
#[cfg(not(unix))]
fn recent_project_to_json(entry: &RecentProject) -> Option<serde_json::Value> {
    let text = entry.path.to_str()?;
    Some(serde_json::json!({
        "path": text,
        "opened_at_unix_ms": entry.opened_at_unix_ms,
    }))
}

/// 从一条最近记录里取回路径：优先 `path_bytes`（Unix 逐字节），
/// 缺失时兼容旧的 `path` 字符串；`path_bytes` 形状坏 ⇒ 只丢这一条。
fn recent_project_path_from_json(item: &serde_json::Value) -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    if let Some(bytes) = item.get("path_bytes") {
        use std::os::unix::ffi::OsStringExt;
        let array = bytes.as_array()?;
        let mut out = Vec::with_capacity(array.len());
        for value in array {
            out.push(u8::try_from(value.as_u64()?).ok()?);
        }
        if out.is_empty() {
            return None;
        }
        return Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(out)));
    }
    let path = item.get("path").and_then(|value| value.as_str())?;
    if path.is_empty() {
        return None;
    }
    Some(std::path::PathBuf::from(path))
}

/// 解析 `recent_projects` 键：键缺失 / 不是数组 ⇒ 空清单；逐条容错
/// （单条形状坏只丢那一条，不连坐整份清单）；重复路径只留第一条
/// （清单语义上最近在前，第一条就是最近的那次）；超出上限截断。
fn parse_recent_projects(root: &serde_json::Value) -> Vec<RecentProject> {
    let Some(items) = root.get("recent_projects").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<RecentProject> = Vec::new();
    for item in items {
        let Some(path) = recent_project_path_from_json(item) else {
            continue;
        };
        // 先去重键规范化再比较：Windows 上 `/music/a` 与 `C:\music\a`
        // 原始写法不同、身份是同一个；拿规范化后的值跟已存条目比，
        // 否则重复条目会漏过去（T-FIX-313 在 Windows 判据里暴露）。
        let path = recent_project_identity(&path);
        if out.iter().any(|entry| entry.path == path) {
            continue;
        }
        let opened_at_unix_ms = item
            .get("opened_at_unix_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        out.push(RecentProject {
            path,
            opened_at_unix_ms,
        });
        if out.len() >= RECENT_PROJECTS_MAX {
            break;
        }
    }
    out
}

/// 解析 `help` 键：键缺失 / 不是对象 ⇒ 默认状态（从未存过）；
/// `version` 高于本构建所识 ⇒ 整份回默认（与旧独立文件的语义一致：
/// 新版写的状态不许被旧构建误读成半份）；其余逐字段容错 —— 坏字段
/// 落回默认、坏课程条目只丢自己，不连坐整份状态与其它键。
fn parse_help_state(root: &serde_json::Value) -> HelpState {
    let Some(object) = root.get("help").and_then(|v| v.as_object()) else {
        return HelpState::default();
    };
    if let Some(version) = object.get("version").and_then(serde_json::Value::as_u64)
        && version > u64::from(crate::help_state::HELP_STATE_VERSION)
    {
        return HelpState::default();
    }
    let mut state = HelpState::default();
    state.onboarding_dismissed =
        json_bool(object, "onboarding_dismissed").unwrap_or(state.onboarding_dismissed);
    state.onboarding_finished =
        json_bool(object, "onboarding_finished").unwrap_or(state.onboarding_finished);
    if let Some(lessons) = object.get("lessons").and_then(|v| v.as_object()) {
        for (id, entry) in lessons {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            state.lessons.insert(
                id.clone(),
                crate::help_state::LessonProgress {
                    steps_done: entry
                        .get("steps_done")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|n| u32::try_from(n).ok())
                        .unwrap_or(0),
                    finished: json_bool(entry, "finished").unwrap_or(false),
                },
            );
        }
    }
    state.active_lesson = object.get("active_lesson").and_then(parse_active_lesson);
    state
}

/// 解析 `help.active_lesson`：`lesson_id` 缺失 / 为空 ⇒ `None`
/// （没有它整条进行中课程无从谈起）；其余字段缺失落回默认。
fn parse_active_lesson(value: &serde_json::Value) -> Option<crate::help_state::ActiveLesson> {
    let object = value.as_object()?;
    let lesson_id = object.get("lesson_id").and_then(|v| v.as_str())?;
    if lesson_id.is_empty() {
        return None;
    }
    let baseline_track_ids = object
        .get("baseline_track_ids")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let current_step = object
        .get("current_step")
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0);
    Some(crate::help_state::ActiveLesson {
        lesson_id: lesson_id.to_owned(),
        baseline_track_ids,
        current_step,
    })
}

/// 取一个有限 f32 字段（JSON 数字）；缺失 / 类型错 / 非有限 ⇒ `None`。
fn json_f32(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<f32> {
    object
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .map(|value| value as f32)
        .filter(|value| value.is_finite())
}

/// 取一个 bool 字段；缺失 / 类型错 ⇒ `None`。
fn json_bool(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<bool> {
    object.get(key).and_then(serde_json::Value::as_bool)
}

/// 尺寸字段的读法：有合法数字就钳回拖拽界内，没有就落回默认值。
fn clamped_size(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    range: (f32, f32),
    default: f32,
) -> f32 {
    json_f32(object, key).map_or(default, |value| value.clamp(range.0, range.1))
}

/// 解析窗口几何：四个数（x / y / width / height）任一缺失、类型错或
/// 出理智界 ⇒ 整份丢弃（半份几何可能把窗口丢到屏幕外，比不恢复更坏）。
/// 尺寸理智界：下限 320×240（远低于 .slint 的 min 1200×700，留给
/// 多显示器/缩放下的合法小窗），上限 16384（单边像素数的物理上限量级）。
/// 位置界 ±10000：多显示器排列的合法范围量级，之外视为损坏数据。
/// `maximized` 缺失/类型错只落回 false（它只是个标记，不连坐几何）。
fn parse_window_geometry(value: &serde_json::Value) -> Option<WindowGeometry> {
    let object = value.as_object()?;
    let width = object.get("width").and_then(serde_json::Value::as_u64)?;
    let height = object.get("height").and_then(serde_json::Value::as_u64)?;
    if !(320..=16384).contains(&width) || !(240..=16384).contains(&height) {
        return None;
    }
    let x = object.get("x").and_then(serde_json::Value::as_i64)?;
    let y = object.get("y").and_then(serde_json::Value::as_i64)?;
    if !(-10000..=10000).contains(&x) || !(-10000..=10000).contains(&y) {
        return None;
    }
    Some(WindowGeometry {
        x: i32::try_from(x).ok()?,
        y: i32::try_from(y).ok()?,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
        maximized: json_bool(object, "maximized").unwrap_or(false),
    })
}

/// 解析 `layout` 键：键缺失 / 不是对象 ⇒ `None`（没有已存布局）；
/// 是对象则逐字段容错填充（坏字段落回 [`WindowLayout::default`] 的
/// 对应值），一份布局不会因单个坏字段整份作废。
fn parse_layout(root: &serde_json::Value) -> Option<WindowLayout> {
    let object = root.get("layout")?.as_object()?;
    let defaults = WindowLayout::default();
    let mut layout = WindowLayout {
        window: object.get("window").and_then(parse_window_geometry),
        ..defaults.clone()
    };
    layout.left_panel_width = clamped_size(
        object,
        "left_panel_width",
        LEFT_PANEL_WIDTH_RANGE,
        defaults.left_panel_width,
    );
    layout.clip_matrix_width = clamped_size(
        object,
        "clip_matrix_width",
        CLIP_MATRIX_WIDTH_RANGE,
        defaults.clip_matrix_width,
    );
    layout.control_room_width = clamped_size(
        object,
        "control_room_width",
        CONTROL_ROOM_WIDTH_RANGE,
        defaults.control_room_width,
    );
    layout.lower_dock_height = clamped_size(
        object,
        "lower_dock_height",
        LOWER_DOCK_HEIGHT_RANGE,
        defaults.lower_dock_height,
    );
    layout.track_headers_width = clamped_size(
        object,
        "track_headers_width",
        TRACK_HEADERS_WIDTH_RANGE,
        defaults.track_headers_width,
    );
    layout.show_left_inspector =
        json_bool(object, "show_left_inspector").unwrap_or(defaults.show_left_inspector);
    layout.show_clip_matrix =
        json_bool(object, "show_clip_matrix").unwrap_or(defaults.show_clip_matrix);
    layout.show_lower_dock =
        json_bool(object, "show_lower_dock").unwrap_or(defaults.show_lower_dock);
    layout.show_control_room =
        json_bool(object, "show_control_room").unwrap_or(defaults.show_control_room);
    // 标签页只认闭集：认不出的值落回默认，不把坏字符串交给界面。
    if let Some(tab) = object
        .get("left_panel_tab")
        .and_then(serde_json::Value::as_str)
        && (tab == "inspector" || tab == "browser")
    {
        layout.left_panel_tab = tab.to_owned();
    }
    // 控制台标签页只认 0..=3：界外落回默认 0（钳到 3 会静默换一个标签页）。
    if let Some(tab) = object
        .get("console_tab")
        .and_then(serde_json::Value::as_i64)
        && (0..=3).contains(&tab)
    {
        layout.console_tab = i32::try_from(tab).unwrap_or(defaults.console_tab);
    }
    layout.console_maximized =
        json_bool(object, "console_maximized").unwrap_or(defaults.console_maximized);
    Some(layout)
}

/// 把布局序列化成 JSON 值（字段名与 [`parse_layout`] 的读法一一对应）。
fn layout_to_json(layout: &WindowLayout) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    if let Some(window) = layout.window {
        object.insert(
            "window".to_owned(),
            serde_json::json!({
                "x": window.x,
                "y": window.y,
                "width": window.width,
                "height": window.height,
                "maximized": window.maximized,
            }),
        );
    }
    object.insert(
        "left_panel_width".to_owned(),
        serde_json::json!(layout.left_panel_width),
    );
    object.insert(
        "clip_matrix_width".to_owned(),
        serde_json::json!(layout.clip_matrix_width),
    );
    object.insert(
        "control_room_width".to_owned(),
        serde_json::json!(layout.control_room_width),
    );
    object.insert(
        "lower_dock_height".to_owned(),
        serde_json::json!(layout.lower_dock_height),
    );
    object.insert(
        "track_headers_width".to_owned(),
        serde_json::json!(layout.track_headers_width),
    );
    object.insert(
        "show_left_inspector".to_owned(),
        serde_json::json!(layout.show_left_inspector),
    );
    object.insert(
        "show_clip_matrix".to_owned(),
        serde_json::json!(layout.show_clip_matrix),
    );
    object.insert(
        "show_lower_dock".to_owned(),
        serde_json::json!(layout.show_lower_dock),
    );
    object.insert(
        "show_control_room".to_owned(),
        serde_json::json!(layout.show_control_room),
    );
    object.insert(
        "left_panel_tab".to_owned(),
        serde_json::Value::String(layout.left_panel_tab.clone()),
    );
    object.insert(
        "console_tab".to_owned(),
        serde_json::json!(layout.console_tab),
    );
    object.insert(
        "console_maximized".to_owned(),
        serde_json::json!(layout.console_maximized),
    );
    serde_json::Value::Object(object)
}

/// 把帮助状态序列化成 JSON 值（字段名与 [`parse_help_state`] 的读法
/// 一一对应；`lessons` 沿 `BTreeMap` 迭代序输出，字节确定性与旧独立
/// 文件一致）。
fn help_to_json(state: &HelpState) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("version".to_owned(), serde_json::json!(state.version));
    object.insert(
        "onboarding_dismissed".to_owned(),
        serde_json::json!(state.onboarding_dismissed),
    );
    object.insert(
        "onboarding_finished".to_owned(),
        serde_json::json!(state.onboarding_finished),
    );
    let lessons: serde_json::Map<String, serde_json::Value> = state
        .lessons
        .iter()
        .map(|(id, progress)| {
            (
                id.clone(),
                serde_json::json!({
                    "steps_done": progress.steps_done,
                    "finished": progress.finished,
                }),
            )
        })
        .collect();
    object.insert("lessons".to_owned(), serde_json::Value::Object(lessons));
    if let Some(active) = &state.active_lesson {
        object.insert(
            "active_lesson".to_owned(),
            serde_json::json!({
                "lesson_id": active.lesson_id,
                "baseline_track_ids": active.baseline_track_ids,
                "current_step": active.current_step,
            }),
        );
    }
    serde_json::Value::Object(object)
}

/// 从默认路径读设置（见 [`load_from`] 的容错口径）。
#[must_use]
pub fn load() -> Settings {
    let path = settings_path();
    match with_settings_lock(&path, || {
        let mut settings = load_from(&path);
        // ⭐ T-FIX-312：启动时把**已经不存在**的最近条目清掉并写回设置。
        // `load_from` 保持纯读（判据仍可用它读原始文件）；清理只在默认路径的
        // `load()` 发生，且只有真的变了才写盘（⛔ 不让每次读设置都制造一次写）。
        let pruned = prune_recent_projects(&settings.recent_projects);
        if pruned != settings.recent_projects {
            settings.recent_projects = pruned;
            save_to_unlocked(&path, &settings)?;
        }
        Ok(settings)
    }) {
        Ok(settings) => settings,
        Err(error) => {
            // 锁文件不可用时仍要让启动读到设置；只取消清理写回，不假装成功。
            record_warning(format!(
                "设置文件加锁失败（{}），本次读取未执行最近清单清理：{error}",
                path.display()
            ));
            load_from(&path)
        }
    }
}

/// 在锁内做一次读-改-写：所有字段变更都基于锁内刚读到的完整设置。
pub(crate) fn update_at(path: &Path, mutate: impl FnOnce(&mut Settings)) -> io::Result<Settings> {
    with_settings_lock(path, || {
        let mut settings = load_from(path);
        let pruned = prune_recent_projects(&settings.recent_projects);
        if pruned != settings.recent_projects {
            settings.recent_projects = pruned;
        }
        mutate(&mut settings);
        save_to_unlocked(path, &settings)?;
        Ok(settings)
    })
}

/// 在默认设置文件上做一次读-改-写（供 `help_state` 等同 crate 调用）。
pub(crate) fn update(mutate: impl FnOnce(&mut Settings)) -> io::Result<Settings> {
    update_at(&settings_path(), mutate)
}

/// ⭐ **剔除失效的最近条目**（T-FIX-312；纯函数）。
///
/// 判据只有一条：`path.is_file()`。目录 / 不存在 / 无权限都算"失效" ——
/// 启动选择器里没有可打开的文件就是死行。保留原顺序（最近在前）与上限无关：
/// 剔除只会**腾出**槽位，新打开的自然能进来。
#[must_use]
pub fn prune_recent_projects(list: &[RecentProject]) -> Vec<RecentProject> {
    list.iter()
        .filter(|entry| entry.path.is_file())
        .cloned()
        .collect()
}

/// ⭐ 从最近清单**移除一条**（T-FIX-312）：读-改-写同一条默认路径，
/// 保住 `language` / `layout` 等其它字段。路径按**规范化身份**匹配
/// （与 [`touch_recent_project`] 的去重键同一条口径）。
pub fn forget_recent_project(path: &std::path::Path) {
    let path = recent_project_identity(path);
    if let Err(error) = update(|settings| {
        settings
            .recent_projects
            .retain(|entry| recent_project_identity(&entry.path) != path);
    }) {
        eprintln!("[yeban-app] 从最近清单移除写回失败（下次启动仍在）: {error}");
    }
}

/// ⭐ **清空最近清单**（T-FIX-312）：写回空数组，⛔ 不再写回死路径。
pub fn clear_recent_projects() {
    if let Err(error) = update(|settings| settings.recent_projects.clear()) {
        eprintln!("[yeban-app] 清空最近清单写回失败（下次启动仍在）: {error}");
    }
}

/// 把设置写到 `path`（调用方必须已经持有跨进程锁）。
fn save_to_unlocked(path: &Path, settings: &Settings) -> io::Result<()> {
    let mut object = serde_json::Map::new();
    object.insert("version".to_owned(), serde_json::json!(1));
    if let Some(lang) = settings.language {
        object.insert(
            "language".to_owned(),
            serde_json::Value::String(lang.name().to_owned()),
        );
    }
    if let Some(layout) = &settings.layout {
        object.insert("layout".to_owned(), layout_to_json(layout));
    }
    if !settings.recent_projects.is_empty() {
        let items: Vec<serde_json::Value> = settings
            .recent_projects
            .iter()
            .filter_map(recent_project_to_json)
            .collect();
        object.insert(
            "recent_projects".to_owned(),
            serde_json::Value::Array(items),
        );
    }
    if settings.help != HelpState::default() {
        object.insert("help".to_owned(), help_to_json(&settings.help));
    }
    if settings.debug_output {
        object.insert("debug_output".to_owned(), serde_json::json!(true));
    }
    if let Some(name) = &settings.audio_output_device {
        object.insert(
            "audio_output_device".to_owned(),
            serde_json::Value::String(name.clone()),
        );
    }
    if let Some(name) = &settings.audio_input_device {
        object.insert(
            "audio_input_device".to_owned(),
            serde_json::Value::String(name.clone()),
        );
    }
    if settings.scroll_in_play {
        object.insert("scroll_in_play".to_owned(), serde_json::json!(true));
    }
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(object))
        .expect("内存里的设置对象一定能序列化成 JSON");
    let tmp = unique_temp_path(path);
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        file.write_all(format!("{text}\n").as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        // rename 失败时只清理自己的临时文件；不碰目标文件。
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// 把设置写到 `path`（先建父目录，锁内写唯一临时文件再 rename，不留半截文件）。
/// `pub(crate)`：`help_state` 按显式路径做读-改-写时复用同一份写盘纪律。
pub(crate) fn save_to(path: &Path, settings: &Settings) -> io::Result<()> {
    with_settings_lock(path, || save_to_unlocked(path, settings))
}

/// 把设置写到默认路径。
pub fn save(settings: &Settings) -> io::Result<()> {
    save_to(&settings_path(), settings)
}

/// 存下一次显式语言选择：先读出现有设置（保住后续线加的字段），再改
/// `language` 写回。写失败记一条待发警告（UI 侧展示）、不让调用方把
/// 一次切换变失败（语言已应用，设置下次再存，见 [`record_save_failure`]）。
pub fn save_language(lang: Language) {
    if let Err(error) = update(|settings| settings.language = Some(lang)) {
        record_save_failure("语言选择", &error);
    }
}

/// 把一次「打开了这个工程文件」记进最近清单：先读出现有设置
/// （保住 `language` / `layout`），再 [`touch_recent_project`] 写回。
/// 与 [`save_language`] 同款契约：写失败记一条待发警告（UI 侧展示），
/// 不让调用方把一次打开变失败（工程已经打开，最近清单下次再记）。
pub fn record_recent_project(path: &std::path::Path, opened_at_unix_ms: u64) {
    if let Err(error) = update(|settings| {
        settings.recent_projects =
            touch_recent_project(&settings.recent_projects, path, opened_at_unix_ms);
    }) {
        record_save_failure("最近工程清单", &error);
    }
}

/// 存下调试输出开关（主菜单「设置 → 调试输出」触发）：先读出现有设置
/// （保住其余键），再改 `debug_output` 写回。与 [`save_language`] 同款
/// 契约：写失败记一条待发警告（UI 侧展示），不让调用方把一次切换变失败
/// （开关已在本会话生效，设置下次再存）。
pub fn save_debug_output(enabled: bool) {
    if let Err(error) = update(|settings| settings.debug_output = enabled) {
        record_save_failure("调试输出开关", &error);
    }
}

/// 存下音频设备选择（「设置 → 音频」面板触发，T-FIX-71）：先读出现有
/// 设置（保住其余键），再改两个设备名写回。`None` = 该方向用系统默认
/// （键不落盘）。与 [`save_debug_output`] 同款契约：写失败记一条待发
/// 警告（UI 侧展示），不让调用方把一次选择变失败（选择已在本会话生效，
/// 设置下次再存）。
pub fn save_audio_devices(output: Option<&str>, input: Option<&str>) {
    if let Err(error) = update(|settings| {
        settings.audio_output_device = output.map(str::to_owned);
        settings.audio_input_device = input.map(str::to_owned);
    }) {
        record_save_failure("音频设备选择", &error);
    }
}

/// 存下编曲视图跟随模式（T-FIX-157）：`true` = Scroll in Play（播放头钉中央），
/// `false` = 经典 Catch（播放头贴左缘）。与 [`save_debug_output`] 同款契约：
/// 先读出现有设置（保住其余键），写失败记一条待发警告（UI 侧展示），
/// 不把一次切换变失败。
pub fn save_scroll_in_play(enabled: bool) {
    if let Err(error) = update(|settings| settings.scroll_in_play = enabled) {
        record_save_failure("跟随模式", &error);
    }
}

/// 启动调试输出决策（纯函数，单测钉死）：`--debug`、`--log-file`、
/// 已存的菜单开关**任一开启即生效**（三者是同一个开关的三个入口，
/// 不是三档级别）。`--log-file` 给出即算开启的理由见
/// `cli::Options::log_file` 的字段文档：不开调试就没有行可写，文件
/// 恒空只会误导回传诊断的人。
#[must_use]
pub fn resolve_debug_output(cli_debug: bool, log_file_given: bool, stored: bool) -> bool {
    cli_debug || log_file_given || stored
}

/// 存下（`Some`）或清掉（`None`，恢复默认布局用）窗口布局：先读出
/// 现有设置（保住 `language`），再改 `layout` 写回。与 [`save_language`]
/// 同款契约：写失败记一条待发警告（UI 侧展示），不让调用方把一次退出/
/// 重置变失败（布局已应用或已退出，设置下次再存）。
pub fn store_layout(layout: Option<WindowLayout>) {
    if let Err(error) = update(|settings| settings.layout = layout) {
        record_save_failure("窗口布局", &error);
    }
}

/// 启动语言决策（纯函数，优先级定案，单测钉死）：
/// `--lang`（显式给过才有值）＞ 已存选择 ＞ 系统 locale 自动检测；
/// 自动检测本身认不出时落回 zh-CN（见 [`detect_system_language`]），
/// 所以这里不需要第四档。
#[must_use]
pub fn resolve_startup_language(
    cli: Option<Language>,
    stored: Option<Language>,
    detected: Language,
) -> Language {
    if let Some(lang) = cli {
        return lang;
    }
    if let Some(lang) = stored {
        return lang;
    }
    detected
}

/// 从一条 locale 字面值（如 `fr_FR.UTF-8`、`pt-BR`、`C`）映射到九语：
/// 语言段决定归属，地区段只在葡萄牙语上有意义（本线只有 pt-BR 一支，
/// `pt-PT` 同样落到 pt-BR，并在文档里明说）；认不出 / `C` / `POSIX` /
/// 空串 ⇒ zh-CN（定案的最终落点）。
#[must_use]
pub fn detect_from_locale_string(raw: &str) -> Language {
    let base = raw.split(['.', '@']).next().unwrap_or("").trim();
    let mut parts = base.split(['_', '-']);
    let lang = parts.next().unwrap_or("").to_ascii_lowercase();
    match lang.as_str() {
        "zh" => Language::ZhCn,
        "en" => Language::EnUs,
        "fr" => Language::FrFr,
        "es" => Language::EsEs,
        "pt" => Language::PtBr,
        "ja" => Language::JaJp,
        "ko" => Language::KoKr,
        "ru" => Language::RuRu,
        "de" => Language::DeDe,
        _ => Language::ZhCn,
    }
}

/// 从进程环境读系统 locale（POSIX 优先级 `LC_ALL` ＞ `LC_MESSAGES` ＞
/// `LANG`），映射到九语；三个变量全空 ⇒ zh-CN。
///
/// 口径：这是 Rust 侧对 Slint 首帧自动选择（`set_bundled_languages`
/// 按系统 locale 选词表）的**同义复述**，不是第二套检测 —— 启动时
/// `apply_language` 仍以这里算出的语言显式覆盖首帧选择。
/// 已知缺口：Windows 上 `LANG` 类变量通常未设，检测落回 zh-CN
///（Slint 自身的自动选择在 Windows 上能读到系统语言，但本函数
/// 读不到；Windows 用户首次启动为 zh-CN，菜单里选一次后即持久化）。
/// 缺口登记在 `docs/app-i18n-notes.md` §10，不静默。
#[must_use]
pub fn detect_system_language() -> Language {
    detect_from_env(
        std::env::var("LC_ALL").ok(),
        std::env::var("LC_MESSAGES").ok(),
        std::env::var("LANG").ok(),
    )
}

/// [`detect_system_language`] 的纯函数核（环境变量以参数传入，单测直接喂值）。
#[must_use]
pub fn detect_from_env(
    lc_all: Option<String>,
    lc_messages: Option<String>,
    lang: Option<String>,
) -> Language {
    for raw in [lc_all, lc_messages, lang].into_iter().flatten() {
        if !raw.trim().is_empty() {
            return detect_from_locale_string(&raw);
        }
    }
    Language::ZhCn
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每条文件类测试一份独立临时目录（`tag` 区分）：同进程并行跑时
    /// 共用一个目录，各测试开头的 `remove_dir_all` 会删掉别条测试正写
    /// 的文件（2026-11-17 布局测试加入后实测竞态，两条写文件测试
    /// 随机 NotFound）。目录名带 pid 防跨进程撞。
    fn test_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("yeban-settings-test-{}-{tag}", std::process::id()))
    }

    /// 优先级定案：`--lang` ＞ 已存选择 ＞ 自动检测。
    #[test]
    fn startup_resolution_priority_is_cli_then_stored_then_detected() {
        use Language::{DeDe, EnUs, FrFr, JaJp, ZhCn};
        // 三档全给 ⇒ 命令行赢。
        assert_eq!(resolve_startup_language(Some(DeDe), Some(FrFr), JaJp), DeDe);
        // 没给命令行 ⇒ 已存选择赢过检测。
        assert_eq!(resolve_startup_language(None, Some(FrFr), JaJp), FrFr);
        // 两档都没给 ⇒ 检测值。
        assert_eq!(resolve_startup_language(None, None, JaJp), JaJp);
        // 检测本身落回 zh-CN 时 ⇒ zh-CN。
        assert_eq!(resolve_startup_language(None, None, ZhCn), ZhCn);
        // 命令行显式给 zh-CN 也不写回（不写回是调用方的责任，这里只证
        // 决策本身把它当最高优先级）。
        assert_eq!(resolve_startup_language(Some(ZhCn), Some(EnUs), FrFr), ZhCn);
    }

    /// locale 字面值 → 九语的映射表（编码后缀、分隔符两形态、大小写不敏感）。
    #[test]
    fn locale_string_detection_maps_all_nine_languages() {
        use Language::{DeDe, EnUs, EsEs, FrFr, JaJp, KoKr, PtBr, RuRu, ZhCn};
        let cases = [
            ("fr_FR.UTF-8", FrFr),
            ("fr-FR", FrFr),
            ("es_ES.UTF-8", EsEs),
            ("pt_BR.UTF-8", PtBr),
            ("pt-PT", PtBr), // 只有 pt-BR 一支，欧洲葡语同样落到它
            ("ja_JP.eucjp", JaJp),
            ("ko_KR.UTF-8", KoKr),
            ("ru_RU.UTF-8", RuRu),
            ("de_DE.UTF-8", DeDe),
            ("en_US.UTF-8", EnUs),
            ("zh_CN.UTF-8", ZhCn),
            ("zh_TW.UTF-8", ZhCn), // 只有简体中文一支
            ("C", ZhCn),
            ("POSIX", ZhCn),
            ("", ZhCn),
            ("xx_YY.UTF-8", ZhCn),
        ];
        for (raw, expected) in cases {
            assert_eq!(detect_from_locale_string(raw), expected, "locale: {raw}");
        }
    }

    /// 环境变量优先级：`LC_ALL` ＞ `LC_MESSAGES` ＞ `LANG`；空串跳过。
    #[test]
    fn env_detection_prefers_lc_all_then_lc_messages_then_lang() {
        use Language::{DeDe, FrFr, JaJp};
        assert_eq!(
            detect_from_env(
                Some("de_DE.UTF-8".to_owned()),
                Some("fr_FR.UTF-8".to_owned()),
                Some("ja_JP.UTF-8".to_owned()),
            ),
            DeDe
        );
        assert_eq!(
            detect_from_env(
                None,
                Some("fr_FR.UTF-8".to_owned()),
                Some("ja_JP.UTF-8".to_owned()),
            ),
            FrFr
        );
        assert_eq!(
            detect_from_env(None, None, Some("ja_JP.UTF-8".to_owned())),
            JaJp
        );
        assert_eq!(
            detect_from_env(Some("".to_owned()), None, Some("ja_JP.UTF-8".to_owned())),
            JaJp
        );
        assert_eq!(detect_from_env(None, None, None), Language::ZhCn);
    }

    /// 存取往返：写 `fr-FR` 再读回同一支；文件内容是带 version 的 JSON。
    #[test]
    fn settings_roundtrip_through_a_temp_file() {
        let dir = test_dir("roundtrip");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        let settings = Settings {
            language: Some(Language::FrFr),
            layout: None,
            recent_projects: Vec::new(),
            help: HelpState::default(),
            debug_output: false,
            audio_output_device: None,
            audio_input_device: None,
            scroll_in_play: false,
        };
        save_to(&path, &settings).expect("临时目录写设置必须成功");
        let text = std::fs::read_to_string(&path).expect("刚写的文件必须能读");
        assert!(text.contains("\"version\": 1"), "文件须带 version: {text}");
        assert!(
            text.contains("\"language\": \"fr-FR\""),
            "文件须存 fr-FR: {text}"
        );
        assert_eq!(load_from(&path), settings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 调试输出开关往返（T-FIX-70）：存 `true` 落 `debug_output` 键、读回
    /// 为真；默认（`false`）不落键、读回为假；键形状坏只丢这一格。
    #[test]
    fn debug_output_roundtrip_through_a_temp_file() {
        let dir = test_dir("debug-output");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        let settings = Settings {
            debug_output: true,
            ..Settings::default()
        };
        save_to(&path, &settings).expect("临时目录写设置必须成功");
        let text = std::fs::read_to_string(&path).expect("刚写的文件必须能读");
        assert!(
            text.contains("\"debug_output\": true"),
            "开启时须落 debug_output 键: {text}"
        );
        assert_eq!(load_from(&path), settings);
        // 默认不落键。
        save_to(&path, &Settings::default()).expect("写默认设置");
        let text = std::fs::read_to_string(&path).expect("读回文件");
        assert!(
            !text.contains("debug_output"),
            "默认（关）不许写 debug_output 键: {text}"
        );
        assert!(!load_from(&path).debug_output);
        // 键形状坏（字符串而非布尔）⇒ 当作没开过，不连坐其它键。
        std::fs::write(&path, r#"{"version":1,"debug_output":"yes"}"#).expect("写坏形状");
        assert!(!load_from(&path).debug_output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-157：Scroll in Play 跟随模式持久化往返。开启落 `scroll_in_play`
    /// 键、读回为真；默认（经典 Catch）不落键、读回为假；坏形状不连坐。
    #[test]
    fn scroll_in_play_roundtrip_through_a_temp_file() {
        let dir = test_dir("scroll-in-play");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        let settings = Settings {
            scroll_in_play: true,
            ..Settings::default()
        };
        save_to(&path, &settings).expect("临时目录写设置必须成功");
        let text = std::fs::read_to_string(&path).expect("刚写的文件必须能读");
        assert!(
            text.contains("\"scroll_in_play\": true"),
            "开启时须落 scroll_in_play 键: {text}"
        );
        assert_eq!(load_from(&path), settings);
        save_to(&path, &Settings::default()).expect("写默认设置");
        let text = std::fs::read_to_string(&path).expect("读回文件");
        assert!(
            !text.contains("scroll_in_play"),
            "默认（经典 Catch）不许写 scroll_in_play 键: {text}"
        );
        assert!(!load_from(&path).scroll_in_play);
        std::fs::write(&path, r#"{"version":1,"scroll_in_play":"yes"}"#).expect("写坏形状");
        assert!(!load_from(&path).scroll_in_play);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⭐⭐ **判据（T-FIX-71，音频设备选择持久化）**：选过的输出/输入设备名
    /// 落键并原样读回；没选过（默认）不落键、读回 `None`；键形状坏 / 空串
    /// ⇒ 当作没选过，且不连坐其它键。
    ///
    /// 反向腿：`save_to` 漏写任一键 ⇒ 读回那行红；`parse_audio_device_name`
    /// 放行空串 ⇒ 空串那行红。
    #[test]
    fn audio_device_selection_roundtrip_through_a_temp_file() {
        let dir = test_dir("audio-devices");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        let settings = Settings {
            audio_output_device: Some("USB Audio Codec".to_owned()),
            audio_input_device: Some("Built-in Microphone".to_owned()),
            ..Settings::default()
        };
        save_to(&path, &settings).expect("临时目录写设置必须成功");
        let text = std::fs::read_to_string(&path).expect("刚写的文件必须能读");
        assert!(
            text.contains("\"audio_output_device\": \"USB Audio Codec\""),
            "选过的输出设备须落键: {text}"
        );
        assert!(
            text.contains("\"audio_input_device\": \"Built-in Microphone\""),
            "选过的输入设备须落键: {text}"
        );
        assert_eq!(load_from(&path), settings);
        // 默认（没选过）不落键。
        save_to(&path, &Settings::default()).expect("写默认设置");
        let text = std::fs::read_to_string(&path).expect("读回文件");
        assert!(
            !text.contains("audio_output_device") && !text.contains("audio_input_device"),
            "没选过不许写音频设备键: {text}"
        );
        let loaded = load_from(&path);
        assert_eq!(loaded.audio_output_device, None);
        assert_eq!(loaded.audio_input_device, None);
        // 键形状坏（数字）/ 空串 ⇒ 当作没选过；同文件的语言键照常读。
        std::fs::write(
            &path,
            r#"{"version":1,"language":"fr-FR","audio_output_device":42,"audio_input_device":""}"#,
        )
        .expect("写坏形状");
        let loaded = load_from(&path);
        assert_eq!(loaded.audio_output_device, None, "非字符串必须当没选过");
        assert_eq!(loaded.audio_input_device, None, "空串必须当没选过");
        assert_eq!(
            loaded.language,
            Some(Language::FrFr),
            "坏音频键不许连坐语言键"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 启动调试输出决策（T-FIX-70）：三个入口任一开启即生效，全关才关。
    #[test]
    fn resolve_debug_output_any_entry_turns_it_on() {
        assert!(!resolve_debug_output(false, false, false));
        assert!(resolve_debug_output(true, false, false));
        assert!(resolve_debug_output(false, true, false));
        assert!(resolve_debug_output(false, false, true));
        assert!(resolve_debug_output(true, true, true));
    }

    /// 容错：缺文件 / 坏 JSON / 未知语言值 ⇒ 一律当作"没有已存选择"。
    /// T-FIX-299：坏 JSON 不再静默 —— 记一条待发警告，原文件备份后丢弃；
    /// 缺文件与未知语言值仍是静默容错（前者是首次启动，后者是逐字段容错）。
    #[test]
    fn settings_load_tolerates_missing_corrupt_and_unknown() {
        let dir = test_dir("tolerates");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let _ = take_pending_warnings();
        let missing = dir.join("missing.json");
        assert_eq!(load_from(&missing), Settings::default());
        assert!(
            take_pending_warnings().is_empty(),
            "缺文件是首次启动，不警告"
        );
        let corrupt = dir.join("corrupt.json");
        std::fs::write(&corrupt, "{ not json").expect("写坏文件");
        assert_eq!(load_from(&corrupt), Settings::default());
        let warnings = take_pending_warnings();
        assert_eq!(warnings.len(), 1, "坏文件必须记一条警告");
        assert!(
            warnings[0].message.contains("已损坏") && warnings[0].message.contains("备份"),
            "警告须点名损坏与备份：{}",
            warnings[0].message
        );
        assert!(!corrupt.exists(), "损坏文件必须被移走（备份后丢弃）");
        let mut backups: Vec<_> = std::fs::read_dir(&dir)
            .expect("读临时目录")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("corrupt.json.corrupt-"))
            })
            .collect();
        assert_eq!(backups.len(), 1, "恰好留一份备份");
        let backup = backups.pop().expect("一份备份");
        assert_eq!(
            std::fs::read_to_string(&backup).expect("备份必须能读"),
            "{ not json",
            "备份须是原文件原样"
        );
        // 备份后文件已不在 ⇒ 第二次读是“缺文件”，不再警告。
        assert_eq!(load_from(&corrupt), Settings::default());
        assert!(take_pending_warnings().is_empty(), "备份后不再重复警告");
        let unknown = dir.join("unknown.json");
        std::fs::write(&unknown, "{\"version\": 1, \"language\": \"xx-YY\"}").expect("写未知语言");
        assert_eq!(load_from(&unknown), Settings::default());
        assert!(
            take_pending_warnings().is_empty(),
            "未知语言值是逐字段容错，不警告"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写回失败的记账本体：`record_save_failure` 同时保留 stderr 行与
    /// 一条待发警告（文案须点名“本次已生效、下次启动会回退”）；取走即清空。
    #[test]
    fn record_save_failure_queues_one_warning_with_rollback_wording() {
        let _ = take_pending_warnings();
        record_save_failure(
            "语言选择",
            &io::Error::new(io::ErrorKind::PermissionDenied, "拒绝访问"),
        );
        let warnings = take_pending_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].message.contains("语言选择")
                && warnings[0].message.contains("已在本会话生效")
                && warnings[0].message.contains("下次启动会回到旧值"),
            "警告文案须点名事项与回退：{}",
            warnings[0].message
        );
        assert!(take_pending_warnings().is_empty(), "取走即清空，不重复展示");
    }

    /// 一份与默认值处处不同的布局（每个字段都离开初值，往返才验得住）。
    fn distinctive_layout() -> WindowLayout {
        WindowLayout {
            window: Some(WindowGeometry {
                x: -120,
                y: 48,
                width: 2048,
                height: 1152,
                maximized: true,
            }),
            left_panel_width: 300.0,
            clip_matrix_width: 200.0,
            control_room_width: 260.0,
            lower_dock_height: 400.0,
            track_headers_width: 200.0,
            show_left_inspector: false,
            show_clip_matrix: true,
            show_lower_dock: false,
            show_control_room: true,
            left_panel_tab: "browser".to_owned(),
            console_tab: 2,
            console_maximized: true,
        }
    }

    /// 布局存取往返：语言与布局同存一份文件，读回逐字段相等。
    #[test]
    fn layout_roundtrip_through_a_temp_file() {
        let dir = test_dir("layout-roundtrip");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        let settings = Settings {
            language: Some(Language::JaJp),
            layout: Some(distinctive_layout()),
            recent_projects: Vec::new(),
            help: HelpState::default(),
            debug_output: false,
            audio_output_device: None,
            audio_input_device: None,
            scroll_in_play: false,
        };
        save_to(&path, &settings).expect("临时目录写设置必须成功");
        let text = std::fs::read_to_string(&path).expect("刚写的文件必须能读");
        assert!(text.contains("\"layout\""), "文件须带 layout: {text}");
        assert!(
            text.contains("\"left_panel_tab\": \"browser\""),
            "文件须存标签页: {text}"
        );
        assert_eq!(load_from(&path), settings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 格式兼容：旧文件（只有 language、没有 layout）⇒ 语言照读、
    /// 布局为 None；`layout` 不是对象 ⇒ 同样当作没有已存布局。
    #[test]
    fn old_file_without_layout_keeps_language_only() {
        let dir = test_dir("old-file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let old = dir.join("old.json");
        std::fs::write(&old, "{\"version\": 1, \"language\": \"fr-FR\"}").expect("写旧文件");
        let loaded = load_from(&old);
        assert_eq!(loaded.language, Some(Language::FrFr));
        assert_eq!(loaded.layout, None);
        let junk = dir.join("junk.json");
        std::fs::write(&junk, "{\"version\": 1, \"layout\": \"not-an-object\"}").expect("写坏布局");
        assert_eq!(load_from(&junk).layout, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 布局逐字段降级：越界尺寸钳回界内、类型错落回默认、标签页只认
    /// 闭集、控制台标签界外落回 0、坏几何整份丢弃但不连坐其余字段、
    /// 语言不连坐。
    #[test]
    fn layout_fields_degrade_individually() {
        let dir = test_dir("degrade");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("degraded.json");
        std::fs::write(
            &path,
            "{\
                \"version\": 1,\
                \"language\": \"de-DE\",\
                \"layout\": {\
                    \"window\": {\"x\": 10, \"y\": 20, \"width\": 12, \"height\": 900, \"maximized\": true},\
                    \"left_panel_width\": 9999.0,\
                    \"clip_matrix_width\": \"wide\",\
                    \"lower_dock_height\": 100.0,\
                    \"left_panel_tab\": \"bogus\",\
                    \"console_tab\": 7\
                }\
            }",
        )
        .expect("写降级样本");
        let loaded = load_from(&path);
        assert_eq!(loaded.language, Some(Language::DeDe), "语言不连坐");
        let layout = loaded.layout.expect("layout 是对象就该解析出布局");
        assert_eq!(layout.window, None, "宽度 12 不合逻辑 ⇒ 几何整份丢弃");
        assert_eq!(layout.left_panel_width, 380.0, "越界钳回上界");
        assert_eq!(layout.clip_matrix_width, 240.0, "类型错落回默认");
        assert_eq!(layout.lower_dock_height, 180.0, "越界钳回下界");
        assert_eq!(layout.left_panel_tab, "inspector", "闭集外落回默认");
        assert_eq!(layout.console_tab, 0, "界外落回默认 0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 几何的好值原样保留（负坐标合法：副屏在主屏左侧），`maximized`
    /// 缺失落回 false 而不连坐几何。
    #[test]
    fn window_geometry_keeps_valid_values_and_defaults_maximized() {
        let dir = test_dir("geometry");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("geometry.json");
        std::fs::write(
            &path,
            "{\"layout\": {\"window\": {\"x\": -1920, \"y\": 0, \"width\": 1920, \"height\": 1080}}}",
        )
        .expect("写几何样本");
        let layout = load_from(&path).layout.expect("布局须解析");
        assert_eq!(
            layout.window,
            Some(WindowGeometry {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
                maximized: false,
            })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 一份各字段都离开默认值的帮助状态（往返与互不连坐才验得住）。
    fn distinctive_help() -> HelpState {
        let mut help = HelpState {
            onboarding_dismissed: true,
            onboarding_finished: true,
            ..HelpState::default()
        };
        help.lessons.insert(
            "first-melody".to_owned(),
            crate::help_state::LessonProgress {
                steps_done: 3,
                finished: true,
            },
        );
        help.active_lesson = Some(crate::help_state::ActiveLesson {
            lesson_id: "first-drum-track".to_owned(),
            baseline_track_ids: vec!["track-a".to_owned(), "track-b".to_owned()],
            current_step: 1,
        });
        help
    }

    /// 帮助状态存取往返：与语言同存一份文件，读回逐字段相等；
    /// 默认状态不写出 `help` 键（旧文件不因读改写而膨胀）。
    #[test]
    fn help_state_roundtrip_through_a_temp_file() {
        let dir = test_dir("help-roundtrip");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        let settings = Settings {
            language: Some(Language::FrFr),
            help: distinctive_help(),
            ..Settings::default()
        };
        save_to(&path, &settings).expect("临时目录写设置必须成功");
        let text = std::fs::read_to_string(&path).expect("刚写的文件必须能读");
        assert!(text.contains("\"help\""), "文件须带 help: {text}");
        assert!(
            text.contains("\"first-melody\""),
            "文件须存课程进度: {text}"
        );
        assert_eq!(load_from(&path), settings);
        // 默认帮助状态 ⇒ 不写 help 键，读回仍是默认。
        let plain = Settings {
            language: Some(Language::FrFr),
            ..Settings::default()
        };
        save_to(&path, &plain).expect("写默认帮助状态");
        let text = std::fs::read_to_string(&path).expect("读回文件");
        assert!(!text.contains("\"help\""), "默认状态不许写 help 键: {text}");
        assert_eq!(load_from(&path).help, HelpState::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 格式兼容：没有 `help` 键的旧文件 ⇒ 帮助状态为默认，其余键照读；
    /// `help` 不是对象 ⇒ 同样当作从未存过。
    #[test]
    fn old_file_without_help_key_reads_default_help() {
        let dir = test_dir("old-file-help");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let old = dir.join("old.json");
        std::fs::write(&old, "{\"version\": 1, \"language\": \"fr-FR\"}").expect("写旧文件");
        let loaded = load_from(&old);
        assert_eq!(loaded.language, Some(Language::FrFr));
        assert_eq!(loaded.help, HelpState::default());
        let junk = dir.join("junk.json");
        std::fs::write(&junk, "{\"version\": 1, \"help\": \"not-an-object\"}").expect("写坏 help");
        assert_eq!(load_from(&junk).help, HelpState::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 帮助状态逐字段降级：未来版本整份回默认；坏课程条目只丢自己；
    /// `active_lesson` 缺 id 整条丢弃，都不连坐合法字段与语言。
    #[test]
    fn help_state_fields_degrade_individually() {
        let dir = test_dir("help-degrade");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let future = dir.join("future.json");
        std::fs::write(
            &future,
            "{\"help\": {\"version\": 99, \"onboarding_dismissed\": true}}",
        )
        .expect("写未来版本");
        assert_eq!(
            load_from(&future).help,
            HelpState::default(),
            "未来版本整份回默认"
        );
        let path = dir.join("degraded.json");
        std::fs::write(
            &path,
            "{\
                \"version\": 1,\
                \"language\": \"de-DE\",\
                \"help\": {\
                    \"version\": 1,\
                    \"onboarding_dismissed\": true,\
                    \"onboarding_finished\": \"yes\",\
                    \"lessons\": {\
                        \"first-melody\": {\"steps_done\": 2, \"finished\": true},\
                        \"broken\": \"not-an-object\",\
                        \"half\": {\"steps_done\": \"three\"}\
                    },\
                    \"active_lesson\": {\"current_step\": 2}\
                }\
            }",
        )
        .expect("写降级样本");
        let loaded = load_from(&path);
        assert_eq!(loaded.language, Some(Language::DeDe), "语言不连坐");
        let help = loaded.help;
        assert!(help.onboarding_dismissed, "合法字段照常生效");
        assert!(!help.onboarding_finished, "类型错落回默认 false");
        assert_eq!(help.lessons.len(), 2, "坏条目只丢自己");
        assert_eq!(
            help.lessons["first-melody"],
            crate::help_state::LessonProgress {
                steps_done: 2,
                finished: true,
            }
        );
        assert_eq!(
            help.lessons["half"],
            crate::help_state::LessonProgress::default(),
            "字段类型错落回默认，不连坐整条"
        );
        assert_eq!(help.active_lesson, None, "缺 lesson_id 整条丢弃");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 读-改-写互不连坐（`save_language` / `store_layout` 的机制本体）：
    /// 改语言保住布局与帮助状态，改布局保住语言与帮助状态。
    #[test]
    fn rewriting_one_field_preserves_the_other() {
        let dir = test_dir("preserve");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        let initial = Settings {
            language: Some(Language::FrFr),
            layout: Some(distinctive_layout()),
            recent_projects: Vec::new(),
            help: distinctive_help(),
            debug_output: false,
            audio_output_device: None,
            audio_input_device: None,
            scroll_in_play: false,
        };
        save_to(&path, &initial).expect("写初始设置");
        // 改语言（save_language 的读-改-写形态）。
        let mut by_language = load_from(&path);
        by_language.language = Some(Language::KoKr);
        save_to(&path, &by_language).expect("改语言写回");
        let after_language = load_from(&path);
        assert_eq!(after_language.language, Some(Language::KoKr));
        assert_eq!(after_language.layout, initial.layout, "改语言不丢布局");
        assert_eq!(after_language.help, initial.help, "改语言不丢帮助状态");
        // 改布局（store_layout 的读-改-写形态，含清掉布局）。
        let mut by_layout = load_from(&path);
        by_layout.layout = None;
        save_to(&path, &by_layout).expect("改布局写回");
        let after_layout = load_from(&path);
        assert_eq!(
            after_layout.language,
            Some(Language::KoKr),
            "改布局不丢语言"
        );
        assert_eq!(after_layout.layout, None);
        assert_eq!(after_layout.help, initial.help, "改布局不丢帮助状态");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-313：两个写者并发 `record_recent_project` 时，锁必须覆盖
    /// 完整读-改-写，最终清单同时保留两条，不出现后写覆盖前写。
    #[test]
    fn concurrent_recent_project_updates_keep_both_entries() {
        use std::sync::{Arc, Barrier};

        let dir = test_dir("concurrent-recent");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("settings.json");
        let a = dir.join("a.yeban");
        let b = dir.join("b.yeban");
        std::fs::write(&a, b"a").expect("写工程 a");
        std::fs::write(&b, b"b").expect("写工程 b");
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for (file, at) in [(a.clone(), 100_u64), (b.clone(), 200_u64)] {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                update_at(&path, |settings| {
                    settings.recent_projects =
                        touch_recent_project(&settings.recent_projects, &file, at);
                })
                .expect("并发记录最近工程");
            }));
        }
        for handle in handles {
            handle.join().expect("写线程结束");
        }

        let loaded = load_from(&path);
        assert_eq!(loaded.recent_projects.len(), 2, "两条都不能丢");
        let got: Vec<_> = loaded
            .recent_projects
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        let canonical_a = std::fs::canonicalize(&a).expect("工程 a 存在");
        let canonical_b = std::fs::canonicalize(&b).expect("工程 b 存在");
        assert!(got.contains(&canonical_a), "并发后缺 a：{got:?}");
        assert!(got.contains(&canonical_b), "并发后缺 b：{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-313：`save_language` 与 `record_recent_project` 并发时，
    /// 语言和最近工程都在；锁不能只保护单个键。
    #[test]
    fn concurrent_language_and_recent_update_keep_both_fields() {
        use std::sync::{Arc, Barrier};

        let dir = test_dir("concurrent-language-recent");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("settings.json");
        let project = dir.join("project.yeban");
        std::fs::write(&project, b"p").expect("写工程文件");
        let barrier = Arc::new(Barrier::new(2));

        let path_a = path.clone();
        let barrier_a = Arc::clone(&barrier);
        let language_writer = std::thread::spawn(move || {
            barrier_a.wait();
            update_at(&path_a, |settings| {
                settings.language = Some(Language::FrFr);
            })
            .expect("并发写语言");
        });

        let path_b = path.clone();
        let barrier_b = Arc::clone(&barrier);
        let recent_writer = std::thread::spawn(move || {
            barrier_b.wait();
            update_at(&path_b, |settings| {
                settings.recent_projects =
                    touch_recent_project(&settings.recent_projects, &project, 300);
            })
            .expect("并发写最近工程");
        });

        language_writer.join().expect("语言线程结束");
        recent_writer.join().expect("最近工程线程结束");

        let loaded = load_from(&path);
        assert_eq!(loaded.language, Some(Language::FrFr), "语言不能丢");
        assert_eq!(loaded.recent_projects.len(), 1, "最近工程不能丢");
        assert_eq!(loaded.recent_projects[0].opened_at_unix_ms, 300);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-313：锁是真正的排他锁，而不是只换了一个临时文件名的装饰。
    /// 主线程先握住 `settings.json.lock`，另一个写者必须等锁释放后才能完成。
    #[test]
    fn settings_lock_blocks_another_writer_until_released() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = test_dir("lock-blocks");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("settings.json");
        let lock_path = settings_lock_path(&path);
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("打开锁文件");
        lock.lock().expect("握住锁");

        let (tx, rx) = mpsc::channel();
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            update_at(&writer_path, |settings| {
                settings.language = Some(Language::FrFr);
            })
            .expect("锁释放后写入");
            tx.send(()).expect("通知主线程");
        });

        let was_blocked = rx.recv_timeout(Duration::from_millis(200)).is_err();
        lock.unlock().expect("释放锁");
        writer.join().expect("写线程结束");
        assert!(
            was_blocked,
            "另一个写者在锁住期间就完成了，说明 update_at 没有真正等锁"
        );
        assert_eq!(load_from(&path).language, Some(Language::FrFr));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-313：临时文件名必须唯一，且锁文件落在目标同目录的
    /// `settings.json.lock`；不再使用固定 `settings.json.tmp`。
    #[test]
    fn settings_temp_name_is_unique_and_lock_uses_sibling_file() {
        let dir = test_dir("unique-temp");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("settings.json");
        let first = unique_temp_path(&path);
        let second = unique_temp_path(&path);
        assert_ne!(first, second, "两次写必须拿不同的临时名");
        assert!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("settings.json.tmp.")),
            "临时名应以原文件名 + .tmp. 开头：{}",
            first.display()
        );
        assert_eq!(
            settings_lock_path(&path),
            dir.join("settings.json.lock"),
            "锁文件必须与目标同目录"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn touch_recent_project_dedupes_front_inserts_and_caps() {
        let dir = test_dir("recent-order");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let a = dir.join("a.yeban");
        let b = dir.join("b.yeban");
        std::fs::write(&a, b"a").expect("写工程 a");
        std::fs::write(&b, b"b").expect("写工程 b");
        let canonical_a = std::fs::canonicalize(&a).expect("工程 a 存在");
        let canonical_b = std::fs::canonicalize(&b).expect("工程 b 存在");
        let list = touch_recent_project(&[], &a, 100);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, canonical_a);
        let list = touch_recent_project(&list, &b, 200);
        assert_eq!(list[0].path, canonical_b, "最新打开的在最前");
        // 再开 a：a 回到最前且不重复，时刻更新。
        let list = touch_recent_project(&list, &a, 300);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].path, canonical_a);
        assert_eq!(list[0].opened_at_unix_ms, 300);
        // 灌满后继续加：长度封顶、最旧的被丢掉。
        let mut list = list;
        for i in 0..20 {
            let path = dir.join(format!("p{i}.yeban"));
            list = touch_recent_project(&list, &path, 400 + i);
        }
        assert_eq!(list.len(), RECENT_PROJECTS_MAX);
        assert_eq!(list[0].opened_at_unix_ms, 419);
        assert!(
            list.iter().all(|entry| entry.path != canonical_b),
            "最旧的条目应被上限丢弃"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-310：最近清单去重键 = 规范化绝对路径。
    /// `dir/./a.yeban` 与 `dir/a.yeban` 是同一条工程 ⇒ 只占一格。
    #[test]
    fn touch_recent_project_normalizes_path_identity() {
        let dir =
            std::env::temp_dir().join(format!("yeban-settings-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let file = dir.join("a.yeban");
        std::fs::write(&file, b"x").expect("写文件");
        let canonical = std::fs::canonicalize(&file).expect("文件已存在");
        let weird = dir.join(".").join("a.yeban");

        let list = touch_recent_project(&[], &weird, 1);
        assert_eq!(list.len(), 1);
        assert_eq!(
            list[0].path, canonical,
            "记录时必须先绝对化 + canonicalize（显示名不参与身份）"
        );
        let list = touch_recent_project(&list, &file, 2);
        assert_eq!(list.len(), 1, "同一工程的两种写法只能占一格");
        assert_eq!(list[0].opened_at_unix_ms, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_projects_round_trip_and_tolerant_parse() {
        let dir =
            std::env::temp_dir().join(format!("yeban-settings-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("settings.json");
        let raw_a = dir.join("a.yeban");
        let raw_b = dir.join("b.yeban");
        std::fs::write(&raw_a, b"a").expect("写工程 a");
        std::fs::write(&raw_b, b"b").expect("写工程 b");
        let a = std::fs::canonicalize(&raw_a).expect("工程 a 存在");
        let b = std::fs::canonicalize(&raw_b).expect("工程 b 存在");
        let settings = Settings {
            recent_projects: vec![
                RecentProject {
                    path: a,
                    opened_at_unix_ms: 111,
                },
                RecentProject {
                    path: b,
                    opened_at_unix_ms: 99,
                },
            ],
            ..Default::default()
        };
        save_to(&path, &settings).expect("写设置");
        let back = load_from(&path);
        assert_eq!(back.recent_projects, settings.recent_projects);
        // 坏条目只丢自己：形状坏的、空路径的、重复的都不连坐其余条目。
        std::fs::write(
            &path,
            r#"{"version":1,"recent_projects":[
                {"path":"/music/a.yeban","opened_at_unix_ms":5},
                {"nope":true},
                {"path":"","opened_at_unix_ms":6},
                {"path":"/music/a.yeban","opened_at_unix_ms":7},
                {"path":"/music/c.yeban"}
            ]}"#,
        )
        .expect("写坏设置");
        let parsed = load_from(&path);
        assert_eq!(parsed.recent_projects.len(), 2);
        assert_eq!(parsed.recent_projects[0].opened_at_unix_ms, 5);
        assert_eq!(parsed.recent_projects[1].opened_at_unix_ms, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-310：最近清单里的非 UTF-8 路径必须**逐字节**往返；
    /// `path_bytes` 坏掉的单条只丢自己，不连坐同文件里的其它条目。
    #[cfg(unix)]
    #[test]
    fn recent_projects_round_trip_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt;
        let dir = std::env::temp_dir().join(format!(
            "yeban-settings-recent-non-utf8-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("settings.json");
        let non_utf8 = dir.join(std::ffi::OsString::from_vec(vec![
            b'b', b'a', b'd', b'-', 0xFF, b'.', b'y', b'e', b'b', b'a', b'n',
        ]));
        let settings = Settings {
            recent_projects: vec![
                RecentProject {
                    path: non_utf8.clone(),
                    opened_at_unix_ms: 42,
                },
                RecentProject {
                    path: std::path::PathBuf::from("/music/ok.yeban"),
                    opened_at_unix_ms: 7,
                },
            ],
            ..Default::default()
        };
        save_to(&path, &settings).expect("写设置");
        let back = load_from(&path);
        assert_eq!(
            back.recent_projects, settings.recent_projects,
            "非 UTF-8 路径必须可逆回读（⛔ 不能被 to_string_lossy 换成 �）"
        );

        // `path_bytes` 形状坏（300 不是字节）⇒ 只丢这一条；下一条 UTF-8 仍读得出。
        std::fs::write(
            &path,
            r#"{"version":1,"recent_projects":[
                {"path_bytes":[98,97,100,300],"opened_at_unix_ms":1},
                {"path":"/music/ok.yeban","opened_at_unix_ms":2}
            ]}"#,
        )
        .expect("写坏设置");
        let parsed = load_from(&path);
        assert_eq!(parsed.recent_projects.len(), 1, "坏条目不得连坐其余条目");
        assert_eq!(parsed.recent_projects[0].opened_at_unix_ms, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
