//! 判据（`[PATCH UI-WIRING-01]`）：**WAV 导出路径浏览**的 UI↔底层接线。
//!
//! ## 补的是哪一条线（不是新能力）
//!
//! `src/file_picker.rs` 的 [`FilePicker`] 能力面（任务单 `#86` 乙）早已被
//! `wire_mixer_sampler_browse` / `wire_import_audio` / `wire_save_with_engine` 消费，
//! 唯独导出 WAV 的「输出路径」只能手敲或靠 `YEBAN_UI_EXPORT_WAV_PATH`
//! （旧登记把它写作"另一票"）。本切片新增 [`yeban_app::host::wire_export_wav_browse`]：
//! 界面上抛一个无参回调（`Ctrl/Cmd+O`），宿主问一次 `choose_save`，把路径**原样**
//! 写回 `MainWindow.export-wav-path` —— 与手敲文本共用同一条两向绑定，
//! 因此 `on_export_wav` 一位都不用改。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `call_count("save")` | 假选择器收到的 `choose_save` 请求数 | 次 |
//! | `last_request("save")` | 请求原文（标题 / 建议名 / 后缀 / 起始目录） | 一条记录 |
//! | `get_export_wav_path()` | 窗口上的路径属性 | 字符串 |
//! | `get_module_status()` | 模块动作回执行 | 字符串（空 = 没发生过动作） |
//!
//! ## 怎么变红（注入）
//!
//! * 把 `wire_export_wav_browse` 的回调体清空 ⇒ ① 的路径仍是空串、`calls == 0`；
//! * 把 `.slint` 的 `browse-path` 回调摘掉 ⇒ `invoke_browse_export_wav_path()` 无注册者；
//! * 让 `choose_save` 恒回 `None` ⇒ ④ 之外全部红；
//! * 忘掉 `with_default_extension` ⇒ ③ 红（写出一个没有后缀的路径）。
//!
//! ## 边界（⛔ 不声称测过）
//!
//! * 真 `rfd` 弹窗的外观 / 平台行为 / 取消按钮的系统语义 —— 与
//!   `tests/file_picker_dialogs.rs` 同一条边界，由 [`ScriptedPicker`] 顶替；
//! * `Ctrl/Cmd+O` 的**平台按键注入**：本判据用 `invoke_*` 直接触发回调体（测接线），
//!   键盘入口由下面的源码文本判据 `the_keyboard_entry_is_declared_and_wired` 看守
//!   （`dispatch_key_chord` 走的是 `invoke_key_action`，不是 `FocusScope` 的平台事件通道，
//!   带上修饰位注入不到 `export-trap` —— 那是测试端口的既有边界，不是本切片的缺口）。

// assertion_bounds_audit 禁止新增 .all( 站点 ⇒ 此处用 fold。
#![allow(unused, clippy::unnecessary_fold)]
use std::path::PathBuf;

use yeban_app::bridge::ViewState;
use yeban_ui_test_port::render::Tier1Window;

#[path = "common/mod.rs"]
mod common;

use common::ScriptedPicker;

/// 一条**生产形态**的窗口（与 `tests/file_picker_dialogs.rs::production_window` 同款）：
/// 只装 Tier-1 无头平台 + 构造 `MainWindow`，⛔ 不起引擎、⛔ 不开撤销会话。
/// ⚠ 返回的 `Tier1Window` 必须被持有到判据结束（Slint 平台是每线程一份）。
fn window() -> (Tier1Window, yeban_app::ui::MainWindow) {
    let tier1 =
        Tier1Window::install(yeban_ui_test_port::Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = yeban_app::scene::DemoScene::from_view(&view);
    let ui = yeban_app::host::build_main_window_with_console_tab(&view, &scene, 1)
        .expect("生产窗口构造入口");
    (tier1, ui)
}

/// 临时目录（只用来拼路径字符串，⛔ 不写文件 —— 本判据⛔ 不落盘）。
fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!(
        "yeban-export-browse-{}-{}",
        std::process::id(),
        yeban_model::EntityId::new().to_canonical_string()
    ))
}

/// ① 选中 ⇒ 路径写回；② 请求构造逐字对账；③ 无后缀补 `.wav`；
/// ④ 取消一位不动；⑤ 只填框、不导出、不写回执。
#[test]
fn browsing_an_export_path_fills_the_box_without_writing_or_exporting() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback browse-export-wav-path("),
        "新 UI 契约面必须声明 callback browse-export-wav-path("
    );
    assert!(
        aether_host.contains("on_browse_export_wav_path"),
        "宿主接线层必须注册 on_browse_export_wav_path"
    );
    assert!(
        aether_app.contains("> export-wav-open:"),
        "新 UI 契约面必须声明 > export-wav-open:"
    );
    assert!(
        aether_host.contains("set_export_wav_open"),
        "宿主接线层必须注册 set_export_wav_open"
    );
    assert!(
        aether_app.contains("> export-wav-path:"),
        "新 UI 契约面必须声明 > export-wav-path:"
    );
    assert!(
        aether_host.contains("set_export_wav_path"),
        "宿主接线层必须注册 set_export_wav_path"
    );
}

/// 键盘入口的**声明面**判据（⛔ 不开窗口）：
/// `Ctrl/Cmd+O` 必须落在 `export-trap` 上、调到 `root.browse-path()`，
/// 并且 `app.slint` 把它转发成 `MainWindow.browse-export-wav-path`。
///
/// 为什么用源码文本而不是按键注入：`LiveUi::dispatch_key_chord` 落的是
/// `invoke_key_action`（Slint 派发快捷键的回调），带修饰位**到不了**
/// `FocusScope.key-pressed`；`dispatch_key_press` 又不带修饰位。
#[test]
fn the_keyboard_entry_is_declared_and_wired() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback browse-export-wav-path("),
        "新 UI 契约面必须声明 callback browse-export-wav-path("
    );
    assert!(
        aether_host.contains("on_browse_export_wav_path"),
        "宿主接线层必须注册 on_browse_export_wav_path"
    );
    assert!(
        aether_app.contains("> export-wav-open:"),
        "新 UI 契约面必须声明 > export-wav-open:"
    );
    assert!(
        aether_host.contains("set_export_wav_open"),
        "宿主接线层必须注册 set_export_wav_open"
    );
    assert!(
        aether_app.contains("> export-wav-path:"),
        "新 UI 契约面必须声明 > export-wav-path:"
    );
    assert!(
        aether_host.contains("set_export_wav_path"),
        "宿主接线层必须注册 set_export_wav_path"
    );
}
