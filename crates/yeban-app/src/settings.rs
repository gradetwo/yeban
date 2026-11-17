//! 最小设置存储（2026-11-17，i18n 语言持久化，Wang 05:57 令；
//! 同日 line/ui-window-state 扩入窗口布局，仍是同一个文件、同一个模块）。
//!
//! 存两样用户偏好：界面语言（显式选过才存）与窗口布局（退出时快照）。
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
//! 已存布局。读不到 / 读坏 ⇒ 当作"什么都没存过"，不报错、不崩溃
//!（设置文件是偏好，不是工程数据）。
//!
//! 布局（[`WindowLayout`]）逐字段容错：单字段类型错 / 越界只让该字段
//! 落回默认值（尺寸按 .slint 拖拽同界钳位、标签页只认闭集），不连坐
//! 整份布局；窗口几何是例外 —— 四个数任一不合逻辑就整份丢弃（半份
//! 几何可能把窗口丢到屏幕外）。纳入哪些真实布局量、普查后排除哪些，
//! 见 `docs/app-i18n-notes.md` §11。
//!
//! 测试隔离：设置了 `YEBAN_SETTINGS_PATH` 时 load/save 走该路径（单测与
//! example 用它指到临时目录，绝不碰真用户配置；生产路径永不设此变量）。

use std::io;
use std::path::PathBuf;

use crate::cli::Language;

/// 设置文件在配置目录下的相对路径（目录名 + 文件名）。
pub const SETTINGS_DIR_NAME: &str = "yeban";
/// 设置文件名（与目录名拼成 `yeban/settings.json`）。
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// 测试隔离环境变量（见模块文档）。
pub const SETTINGS_PATH_ENV: &str = "YEBAN_SETTINGS_PATH";

/// 设置内容：`language`（i18n 线）＋ `layout`（窗口布局线），同一份文件。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// 用户显式选过的语言；`None` = 从未存过（首次启动走自动检测）。
    pub language: Option<Language>,
    /// 已存的窗口布局；`None` = 从未存过（启动时不恢复，界面保持
    /// .slint 初值 —— 含 persona 派生的面板显隐初值，不被强行覆盖）。
    pub layout: Option<WindowLayout>,
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
    /// 左侧资源栏是否折叠成图标导轨。
    pub sidebar_collapsed: bool,
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
            sidebar_collapsed: false,
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

/// 从 `path` 读设置：文件不存在 / 解析失败 ⇒ 默认设置（全 `None`），
/// 调用方按"什么都没存过"处理；单字段的容错见 [`parse_layout`] 与
/// `language` 的九语闭集校验。
#[must_use]
pub fn load_from(path: &std::path::Path) -> Settings {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Settings::default();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Settings::default();
    };
    Settings {
        language: value
            .get("language")
            .and_then(|name| name.as_str())
            .and_then(Language::from_name),
        layout: parse_layout(&value),
    }
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
    layout.sidebar_collapsed =
        json_bool(object, "sidebar_collapsed").unwrap_or(defaults.sidebar_collapsed);
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
    object.insert(
        "sidebar_collapsed".to_owned(),
        serde_json::json!(layout.sidebar_collapsed),
    );
    serde_json::Value::Object(object)
}

/// 从默认路径读设置（见 [`load_from`] 的容错口径）。
#[must_use]
pub fn load() -> Settings {
    load_from(&settings_path())
}

/// 把设置写到 `path`（先建父目录，写临时文件再 rename，不留半截文件）。
fn save_to(path: &std::path::Path, settings: &Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
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
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(object))
        .expect("内存里的设置对象一定能序列化成 JSON");
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, format!("{text}\n"))?;
    std::fs::rename(&tmp, path)
}

/// 把设置写到默认路径。
pub fn save(settings: &Settings) -> io::Result<()> {
    save_to(&settings_path(), settings)
}

/// 存下一次显式语言选择：先读出现有设置（保住后续线加的字段），再改
/// `language` 写回。写失败只打印、不让调用方把一次切换变失败
/// （语言已应用，设置下次再存）。
pub fn save_language(lang: Language) {
    let mut settings = load();
    settings.language = Some(lang);
    if let Err(error) = save(&settings) {
        eprintln!("[yeban-app] 语言选择写回设置失败（下次启动仍走旧选择）: {error}");
    }
}

/// 存下（`Some`）或清掉（`None`，恢复默认布局用）窗口布局：先读出
/// 现有设置（保住 `language`），再改 `layout` 写回。与 [`save_language`]
/// 同款契约：写失败只打印，不让调用方把一次退出/重置变失败
///（布局已应用或已退出，设置下次再存）。
pub fn store_layout(layout: Option<WindowLayout>) {
    let mut settings = load();
    settings.layout = layout;
    if let Err(error) = save(&settings) {
        eprintln!("[yeban-app] 窗口布局写回设置失败（下次启动仍走旧布局）: {error}");
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

    /// 容错：缺文件 / 坏 JSON / 未知语言值 ⇒ 一律当作"没有已存选择"。
    #[test]
    fn settings_load_tolerates_missing_corrupt_and_unknown() {
        let dir = test_dir("tolerates");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let missing = dir.join("missing.json");
        assert_eq!(load_from(&missing), Settings::default());
        let corrupt = dir.join("corrupt.json");
        std::fs::write(&corrupt, "{ not json").expect("写坏文件");
        assert_eq!(load_from(&corrupt), Settings::default());
        let unknown = dir.join("unknown.json");
        std::fs::write(&unknown, "{\"version\": 1, \"language\": \"xx-YY\"}").expect("写未知语言");
        assert_eq!(load_from(&unknown), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
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
            sidebar_collapsed: true,
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
                    \"console_tab\": 7,\
                    \"sidebar_collapsed\": true\
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
        assert!(layout.sidebar_collapsed, "合法字段照常生效");
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

    /// 读-改-写互不连坐（`save_language` / `store_layout` 的机制本体）：
    /// 改语言保住布局，改布局保住语言。
    #[test]
    fn rewriting_one_field_preserves_the_other() {
        let dir = test_dir("preserve");
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        let initial = Settings {
            language: Some(Language::FrFr),
            layout: Some(distinctive_layout()),
        };
        save_to(&path, &initial).expect("写初始设置");
        // 改语言（save_language 的读-改-写形态）。
        let mut by_language = load_from(&path);
        by_language.language = Some(Language::KoKr);
        save_to(&path, &by_language).expect("改语言写回");
        let after_language = load_from(&path);
        assert_eq!(after_language.language, Some(Language::KoKr));
        assert_eq!(after_language.layout, initial.layout, "改语言不丢布局");
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
