//! T-FIX-810：预制面板必须响亮列出被跳过的坏用户预制，不能静默只显示“其余行”。
//! 修前 `rows()` 只读 `listing["presets"]`，`skipped` 不进 `preset-error` / toast。

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::Preset;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::render::Tier1Window;

const NOW_MS: u64 = 1_760_000_000_000;
const GOOD_ID: &str = "user:01J810GOODPRESET0000000000";
const BAD_NAME: &str = "broken-810.json";

struct HomeGuard(Option<OsString>);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.0.take() {
            // SAFETY: 本测试进程独占 HOME 环境变量，guard 在 Drop 时恢复。
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            // SAFETY: 同上；这是测试进程的局部环境变量改动。
            None => unsafe { std::env::remove_var("HOME") },
        }
    }
}

fn isolated_home(tag: &str) -> (PathBuf, HomeGuard) {
    let dir = std::env::temp_dir().join(format!(
        "yeban-preset-skipped-{tag}-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时 HOME");
    let old = std::env::var_os("HOME");
    // SAFETY: 测试进程没有其他线程在读写 HOME；guard 负责恢复。
    unsafe { std::env::set_var("HOME", &dir) };
    (dir, HomeGuard(old))
}

#[test]
fn skipped_broken_user_preset_is_reported_while_other_rows_still_list() {
    let (home, _guard) = isolated_home("broken");
    let library = home.join("Music").join("Audio Music Apps").join("Patches");
    std::fs::create_dir_all(&library).expect("建用户预制库");

    let mut preset =
        Preset::parse(include_str!("../../../assets/presets/piano-warm-room.json").as_bytes())
            .expect("工厂预制必须能解析");
    preset.id = GOOD_ID.to_owned();
    preset.name = "T810 Good User Preset".to_owned();
    std::fs::write(
        library.join("01J810GOODPRESET0000000000.json"),
        serde_json::to_vec(&preset).expect("预制必须能序列化"),
    )
    .expect("写好的用户预制");
    std::fs::write(library.join(BAD_NAME), b"{ this is not a preset").expect("写坏预制");

    let project = yeban_model::samples::demo_project();
    let _tier1 = Tier1Window::install(Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 1).expect("构造窗口");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<preset-skipped>", "yeban-app", project, NOW_MS).expect("会话"),
    ));
    host::wire_preset_library(&ui, Some(port));

    ui.invoke_open_preset_library();
    let rows = ui.get_preset_rows();
    assert!(
        rows.iter().any(|row| row.id.as_str() == GOOD_ID),
        "好的用户预制仍须列出；实际 {} 行",
        rows.row_count()
    );
    assert!(
        rows.iter().any(|row| row.kind.as_str() == "system"),
        "系统预制也必须仍列出"
    );
    let toast = ui.get_toast_message().to_string();
    assert!(
        toast.contains(BAD_NAME),
        "跳过的坏文件必须响亮进 toast，实际: {toast:?}"
    );
    let error = ui.get_preset_error().to_string();
    assert!(
        error.contains(BAD_NAME),
        "preset-error 也必须点名坏文件，实际: {error:?}"
    );
    assert!(!toast.contains("0 个"), "非空 skipped 不得报 0 个");
}
