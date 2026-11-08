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
    let (_tier1, ui) = window();
    assert!(
        ui.get_export_wav_path().is_empty(),
        "夹具前提：路径属性初始为空（`export-wav-path` 的默认形态）"
    );

    let dir = scratch();

    // ---- ① 选中一条带 `.wav` 的路径 --------------------------------------
    let picked = dir.join("my-mix.wav");
    let picker = ScriptedPicker::answering_save(&picked);
    yeban_app::host::wire_export_wav_browse(&ui, picker.clone());
    ui.invoke_browse_export_wav_path();
    assert_eq!(
        ui.get_export_wav_path().to_string(),
        picked.display().to_string(),
        "① 选中的路径必须**原样**写回 `export-wav-path`（与手敲同一条两向绑定）"
    );
    assert_eq!(
        picker.call_count("save"),
        1,
        "① `choose_save` 必须被问恰好一次"
    );

    // ---- ② 请求构造：标题 / 后缀 / 建议名 / 起始目录 ------------------------
    let request = picker.last_request("save").expect("请求必须被记下");
    assert_eq!(request.title, "选择 WAV 导出路径");
    assert_eq!(request.extensions, vec!["wav".to_owned()]);
    assert_eq!(
        request.file_name.as_deref(),
        Some("ui-export.wav"),
        "② 空路径时建议名 = `ModulePaths::default().export_wav` 的同名文件"
    );
    assert!(
        request.directory.is_none(),
        "② 空路径时起始目录交给平台（⛔ 不猜一个目录）"
    );

    // ---- ②b 重开：建议名与起始目录跟着**当前框里的值**走 --------------------
    let next = dir.join("another.wav");
    let picker2 = ScriptedPicker::answering_save(&next);
    // Slint 的 `on_*` 是覆盖语义 ⇒ 重新注册后只有新替身会收到请求。
    yeban_app::host::wire_export_wav_browse(&ui, picker2.clone());
    ui.invoke_browse_export_wav_path();
    let request2 = picker2.last_request("save").expect("第二次请求");
    assert_eq!(
        request2.file_name.as_deref(),
        Some("my-mix.wav"),
        "②b 建议名必须取当前路径的文件名"
    );
    assert_eq!(
        request2.directory.as_deref(),
        Some(dir.as_path()),
        "②b 起始目录必须取当前路径的父目录"
    );
    assert_eq!(
        ui.get_export_wav_path().to_string(),
        next.display().to_string()
    );

    // ---- ③ 用户挑了没有后缀的路径 ⇒ 统一补 `.wav` ---------------------------
    let bare = dir.join("no-extension");
    let picker3 = ScriptedPicker::answering_save(&bare);
    yeban_app::host::wire_export_wav_browse(&ui, picker3.clone());
    ui.invoke_browse_export_wav_path();
    assert_eq!(
        ui.get_export_wav_path().to_string(),
        dir.join("no-extension.wav").display().to_string(),
        "③ `with_default_extension` 必须补后缀（跨平台口径只此一处）"
    );
    assert!(!bare.exists(), "③ 判据只拼字符串，⛔ 不许真的落盘");

    // ---- ④ 取消（`None`）⇒ 一位不动 ---------------------------------------
    let cancelling = ScriptedPicker::cancelling();
    yeban_app::host::wire_export_wav_browse(&ui, cancelling.clone());
    let keep = ui.get_export_wav_path().to_string();
    ui.invoke_browse_export_wav_path();
    assert_eq!(
        cancelling.call_count("save"),
        1,
        "④ 取消也算一次**被问过**的对话框"
    );
    assert_eq!(
        ui.get_export_wav_path().to_string(),
        keep,
        "④ 取消不是错误：⛔ 不改路径、⛔ 不写回执"
    );

    // ---- ⑤ 只填框、⛔ 不导出 ----------------------------------------------
    assert_eq!(
        ui.get_module_status().to_string(),
        "",
        "⑤ 浏览只填框：没有任何模块动作发生 ⇒ 回执行必须仍是空的"
    );
    assert!(
        !ui.get_export_wav_open(),
        "⑤ 浏览⛔ 不该顺手打开/关闭弹窗（弹窗开关的写者仍只有两处）"
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
    let dialog = include_str!("../ui/dialogs/export_wav_dialog.slint");
    let app = include_str!("../ui/app.slint");
    let host = include_str!("../src/host.rs");

    assert!(
        dialog.contains("callback browse-path();"),
        "弹窗必须声明 `browse-path` 回调"
    );
    assert!(
        dialog.contains("event.modifiers.control || event.modifiers.meta")
            && dialog.contains("event.text == \"o\""),
        "`Ctrl/Cmd+O` 分支必须在 `export-trap.key-pressed` 里"
    );
    assert!(
        dialog.contains("root.browse-path();"),
        "键盘分支必须调到弹窗的 `browse-path`"
    );
    assert!(
        app.contains("callback browse-export-wav-path();")
            && app.contains("browse-path => { root.browse-export-wav-path(); }"),
        "`app.slint` 必须把弹窗回调转发到 `MainWindow`（否则宿主无处注册）"
    );
    assert!(
        host.contains("pub fn wire_export_wav_browse")
            && host.contains("ui.on_browse_export_wav_path")
            && host.contains("picker.choose_save(&request)"),
        "宿主必须有唯一的接线落点，并且真的问 `choose_save`"
    );
    // 接线必须真的装到两条腿上（产品 / 无头），否则 Slint 的 `on_*` 空实现。
    let main = include_str!("../src/main.rs");
    let live = include_str!("../src/live_surface.rs");
    assert!(
        main.contains("host::wire_export_wav_browse") && main.contains("NativeFilePicker"),
        "产品腿 `run_gui` 必须接 `NativeFilePicker`"
    );
    assert!(
        live.contains("host::wire_export_wav_browse") && live.contains("NoPicker"),
        "无头腿必须接 `NoPicker`（⛔ 不能把模态原生框装进没有窗口的执行面）"
    );
    // ⛔ 不新增可见控件 ⇒ 既有活文档判据的计数一位不改。
    assert!(
        !dialog.contains("export-wav-browse-button"),
        "本切片刻意⛔ 不新增可寻址按钮（ROI：见交付说明）；要加就同步更新计数判据"
    );
}
