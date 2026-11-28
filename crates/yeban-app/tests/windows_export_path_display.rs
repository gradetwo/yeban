//! **T-FIX-919**：WAV 导出默认/浏览路径的 Windows 扩展长度前缀显示。
//!
//! Windows `std::fs::canonicalize` 返回 `\\?\D:\...`；把它原样写进导出路径
//! 输入框会让用户看到内核前缀并被迫手工替换整段路径。本判据钉住：
//!
//! 1. 普通盘符与 UNC 路径的显示形式不带 `\\?\`；
//! 2. 去前缀后仍属长路径（≥ MAX_PATH）⇒ 保留前缀，长路径语义不因显示丢；
//! 3. `Volume{}` 等非盘符扩展路径原样保留，不猜；
//! 4. 活窗口：默认路径播种与浏览填回都走同一条显示归一化；用户已填路径不被覆盖。
//!
//! ## 边界
//!
//! 测试盒是 Linux，不能现场调用 Windows `canonicalize`；归一化输入使用
//! Windows 实机记录的前缀形态（盘符 / UNC / 长路径）。真 WAV 写出仍由
//! `export_wav` 的既有套件覆盖；本文件只验证**显示层**不破坏路径语义。

#![allow(unused)]

use std::path::{Path, PathBuf};
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host::{self, user_visible_path};
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::Tier1Window;

#[path = "common/mod.rs"]
mod common;

use common::ScriptedPicker;

fn window() -> (Tier1Window, yeban_app::ui::MainWindow) {
    let tier1 =
        Tier1Window::install(yeban_ui_test_port::Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = yeban_app::host::build_main_window_with_console_tab(&view, &scene, 1)
        .expect("生产窗口构造入口");
    (tier1, ui)
}

#[test]
fn windows_extended_prefix_is_only_stripped_when_normal_path_still_fits() {
    assert_eq!(
        user_visible_path(Path::new(r"\\?\D:\yeban-nightly\T-FIX-614\ui-export.wav")),
        PathBuf::from(r"D:\yeban-nightly\T-FIX-614\ui-export.wav"),
        "普通盘符路径显示不得带 `\\\\?\\` 前缀"
    );
    assert_eq!(
        user_visible_path(Path::new(r"\\?\UNC\studio\share\ui-export.wav")),
        PathBuf::from(r"\\studio\share\ui-export.wav"),
        "UNC 扩展路径显示应还原成 `\\\\server\\share\\...`"
    );
    assert_eq!(
        user_visible_path(Path::new(
            r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\ui-export.wav"
        )),
        PathBuf::from(r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\ui-export.wav"),
        "Volume GUID 形态不是盘符/UNC，显示层不得猜语义"
    );
    assert_eq!(
        user_visible_path(Path::new("relative/ui-export.wav")),
        PathBuf::from("relative/ui-export.wav"),
        "相对路径原样"
    );

    // 去前缀后 ≥ 260 字符 ⇒ 必须保留扩展前缀，否则真正长路径会被截断。
    let long = format!(r"\\?\D:\{}\\ui-export.wav", "a".repeat(280));
    assert_eq!(
        user_visible_path(Path::new(&long)),
        PathBuf::from(&long),
        "长路径显示必须保留 `\\\\?\\` 前缀"
    );
    let long_unc = format!(r"\\?\UNC\server\share\{}\ui-export.wav", "b".repeat(270));
    assert_eq!(
        user_visible_path(Path::new(&long_unc)),
        PathBuf::from(&long_unc),
        "长 UNC 路径显示必须保留扩展前缀"
    );
}

#[test]
fn export_dialog_seed_and_browse_fill_back_use_the_visible_form() {
    let (_tier1, ui) = window();

    // ① 缺省路径（模拟 Windows canonicalize 的结果）播种进输入框：无前缀。
    host::seed_export_wav_path(
        &ui,
        Path::new(r"\\?\D:\yeban-nightly\T-FIX-614\ui-export.wav"),
    );
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        r"D:\yeban-nightly\T-FIX-614\ui-export.wav",
        "默认导出路径显示不得带扩展长度前缀"
    );

    // ② 用户已填的普通路径不被默认值覆盖，也不被额外改写。
    ui.set_export_wav_path(r"D:\music\my-take.wav".into());
    host::seed_export_wav_path(&ui, Path::new(r"\\?\D:\elsewhere\ui-export.wav"));
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        r"D:\music\my-take.wav",
        "用户填过的路径不得被默认值覆盖"
    );

    // ③ 浏览填回：选择器给出扩展前缀形态，输入框仍是普通形式。
    host::wire_export_wav_browse(
        &ui,
        ScriptedPicker::answering_save(r"\\?\D:\yeban-nightly\T-FIX-614\take.wav")
            as Rc<dyn yeban_app::file_picker::FilePicker>,
    );
    ui.invoke_browse_export_wav_path();
    assert_eq!(
        ui.get_export_wav_path().as_str(),
        r"D:\yeban-nightly\T-FIX-614\take.wav",
        "浏览填回不得把 `\\\\?\\` 写进用户可见路径"
    );
}
