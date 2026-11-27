//! T-FIX-807：工程内嵌 `user:` 预制必须出现在预制面板，并能被面板应用；
//! 用户库为空时也一样。修前 `rows()` 只列工厂+空用户库、`apply` 硬编码空表，
//! 所以本判据在 c510 + T-FIX-806 之后、T-FIX-807 之前必红。

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
const USER_ID: &str = "user:01J807EMBEDDEDPANEL00000000";
const USER_NAME: &str = "T807 Embedded Only";

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
        "yeban-preset-panel-{tag}-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时 HOME");
    let old = std::env::var_os("HOME");
    // SAFETY: 测试进程没有其他线程在读写 HOME；guard 负责恢复。
    unsafe { std::env::set_var("HOME", &dir) };
    (dir, HomeGuard(old))
}

fn embedded_bytes() -> Vec<u8> {
    let mut preset =
        Preset::parse(include_str!("../../../assets/presets/piano-warm-room.json").as_bytes())
            .expect("工厂预制必须能解析");
    preset.id = USER_ID.to_owned();
    preset.name = USER_NAME.to_owned();
    serde_json::to_vec(&preset).expect("预制必须能序列化")
}

#[test]
fn embedded_user_preset_is_listed_and_applied_while_user_library_is_empty() {
    let (home, _guard) = isolated_home("embedded");

    let mut project = yeban_model::samples::demo_project();
    let track_id = project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
        .expect("默认工程至少有一条非主总线轨")
        .id;
    project.tracks.get_mut(&track_id).expect("轨存在").preset = None;
    let entry = yeban_model::preset::embedded_entry_name(USER_ID).expect("条目名");
    let bytes = embedded_bytes();

    let _tier1 = Tier1Window::install(Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 1).expect("构造窗口");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<preset-panel>", "yeban-app", project, NOW_MS).expect("会话"),
    ));
    port.replace_embedded_presets(vec![(entry, bytes)]);
    ui.set_selected_track_id(track_id.to_canonical_string().into());
    host::wire_preset_library(&ui, Some(Rc::clone(&port)));

    ui.invoke_open_preset_library();
    let rows = ui.get_preset_rows();
    let row = (0..rows.row_count())
        .map(|index| rows.row_data(index).expect("行"))
        .find(|row| row.id.as_str() == USER_ID)
        .expect("内嵌 user 预制必须出现在面板行里（用户库为空也不许消失）");
    assert_eq!(row.name.as_str(), USER_NAME);
    assert_eq!(row.kind.as_str(), "user");
    let user_library_dir = yeban_model::preset::user_preset_library_dir().expect("用户库路径");
    assert!(
        !user_library_dir
            .join(format!("{}.json", &USER_ID["user:".len()..]))
            .exists(),
        "本判据要求用户库里没有同名副本，实际存在: {}",
        user_library_dir.display()
    );
    assert_eq!(
        ui.get_preset_error().to_string(),
        "",
        "列出内嵌副本不应报错"
    );

    let before = port.fingerprint().expect("初始指纹");
    ui.invoke_preset_apply(USER_ID.into());
    assert_eq!(
        ui.get_preset_error().to_string(),
        "",
        "应用内嵌副本不应报错"
    );
    let after = port.try_project().expect("工程仍在");
    assert_eq!(
        after
            .track(&track_id)
            .expect("轨存在")
            .preset
            .as_ref()
            .and_then(|state| state.preset_id.as_deref()),
        Some(USER_ID),
        "应用后轨上必须写进内嵌预制标识"
    );
    assert_ne!(
        port.fingerprint().as_deref(),
        Some(before.as_str()),
        "应用内嵌预制必须真的提交进会话"
    );
    let _ = home;
}
