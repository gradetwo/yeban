//! 最小设置存储（2026-11-17，i18n 语言持久化，Wang 05:57 令）。
//!
//! 只存用户在界面上显式选过的语言；启动优先级（定案）是
//! `--lang` 参数（临时覆盖、不写回）＞ 已存选择 ＞ 系统 locale 自动检测 ＞ zh-CN，
//! 决策函数见 [`resolve_startup_language`]，单测钉死。
//!
//! 选址：仓内**没有**既有设置路径先例（全仓 grep `config_dir` / `settings.json`
//! 零命中），故新建平台配置目录下 `yeban/settings.json`：
//! Linux `$XDG_CONFIG_HOME/yeban/settings.json`（未设则 `$HOME/.config/yeban/`），
//! macOS `$HOME/Library/Application Support/yeban/settings.json`，
//! Windows `%APPDATA%/yeban/settings.json`。目录名全平台统一小写 `yeban`
//!（与仓库名、CLI 二进制名一致），不随平台改大小写。
//!
//! 格式：`{"version": 1, "language": "fr-FR"}`。`version` 是给后续线的
//! 演进位 —— line/ui-window-state（窗口几何＋面板布局持久化、菜单
//! 「恢复默认布局」）直接往同一文件加字段，不另建存储（见
//! `docs/app-i18n-notes.md` §10）。读不到 / 读坏 / 取值不在九语里 ⇒
//! 当作"没有已存选择"，不报错、不崩溃（设置文件是偏好，不是工程数据）。
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

/// 设置内容：本线只存 `language` 一项；后续字段直接往这里加。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// 用户显式选过的语言；`None` = 从未存过（首次启动走自动检测）。
    pub language: Option<Language>,
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

/// 从 `path` 读设置：文件不存在 / 解析失败 / `language` 取值不在九语里 ⇒
/// 默认设置（`None`），调用方按"没有已存选择"处理。
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
    }
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
        let dir = std::env::temp_dir().join(format!("yeban-settings-test-{}", std::process::id()));
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        let settings = Settings {
            language: Some(Language::FrFr),
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
        let dir = std::env::temp_dir().join(format!("yeban-settings-test-{}", std::process::id()));
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
}
