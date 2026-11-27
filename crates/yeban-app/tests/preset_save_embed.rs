//! T-FIX-808：GUI 本地保存必须把当前工程引用的 `user:` 预制收进归档
//! （`presets/user/<id>.json`），且用户库为空时重开仍能解析。
//!
//! 修前 `SaveRequest` 三个构造器逐字写 `embedded_presets: Vec::new()` ⇒
//! 保存后的容器里没有内嵌副本，本判据在 T-FIX-807 之后、T-FIX-808 之前必红。

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::Preset;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::render::Tier1Window;

const NOW_MS: u64 = 1_760_000_000_000;
const USER_ID: &str = "user:01J808EMBEDDEDSAVE000000000";

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
        "yeban-preset-save-{tag}-{}-{}",
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
    preset.name = "T808 Embedded Save".to_owned();
    serde_json::to_vec(&preset).expect("预制必须能序列化")
}

#[test]
fn gui_local_save_writes_the_referenced_embedded_user_preset() {
    let (_home, _guard) = isolated_home("embedded");
    let scratch = std::env::temp_dir().join(format!(
        "yeban-preset-save-out-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&scratch).expect("建输出目录");
    let target = scratch.join("embedded-save.yeban");

    let mut project = yeban_model::samples::demo_project();
    let track_id = project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
        .expect("默认工程至少有一条非主总线轨")
        .id;
    let entry = yeban_model::preset::embedded_entry_name(USER_ID).expect("条目名");
    let bytes = embedded_bytes();
    {
        let track = project.tracks.get_mut(&track_id).expect("轨存在");
        let state = yeban_model::preset::TrackPresetState {
            preset_id: Some(USER_ID.to_owned()),
            ..Default::default()
        };
        track.preset = Some(Box::new(state));
    }

    let _tier1 = Tier1Window::install(Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 1).expect("构造窗口");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<preset-save>", "yeban-app", project, NOW_MS).expect("会话"),
    ));
    port.replace_embedded_presets(vec![(entry.clone(), bytes.clone())]);
    host::wire_save(
        &ui,
        host::SaveStatus {
            target: Some(target.clone()),
            project: port.project_shared().expect("会话共享工程"),
        },
        Some(Rc::clone(&port)),
        Rc::new(yeban_app::file_picker::NoPicker),
        None,
    );

    ui.invoke_save_project();
    assert!(
        target.exists(),
        "保存必须落盘；状态: {}",
        ui.get_save_status()
    );
    let archive = yeban_model::container::read_project_container(
        &std::fs::read(&target).expect("读归档"),
        &Default::default(),
    )
    .expect("写出的必须是可读容器");
    assert!(
        archive
            .embedded_presets
            .iter()
            .any(|(name, _)| name == &entry),
        "归档必须含 `{entry}`，实际条目: {:?}",
        archive
            .embedded_presets
            .iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
    );
    // 用户库为空：只靠归档里的内嵌副本也必须解析回同一条预制。
    let parsed = yeban_mcp::domain::preset::parse_embedded_presets(&archive.embedded_presets)
        .expect("空用户库下必须能从归档解析内嵌预制");
    assert_eq!(
        parsed.get(USER_ID).map(|preset| preset.name.as_str()),
        Some("T808 Embedded Save"),
        "归档里的内嵌副本必须是轨引用的那一条"
    );
    assert_eq!(
        archive
            .project
            .track(&track_id)
            .expect("轨存在")
            .preset
            .as_ref()
            .and_then(|state| state.preset_id.as_deref()),
        Some(USER_ID),
        "归档工程里的轨仍须引用那条 user 预制"
    );

    std::fs::remove_dir_all(&scratch).ok();
}
