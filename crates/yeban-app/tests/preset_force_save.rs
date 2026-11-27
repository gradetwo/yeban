//! T-ISS-MWXVPJKY-C2DC76：控制面 `ui/force_save` 保存含 `user:` 预制的工程时，
//! 归档必须带上 `presets/user/<id>.json`，不能像修前那样写空内嵌表。
//!
//! 修前 `live_surface.rs` 的 `ProjectArchive { embedded_presets: Vec::new() }`
//! 让这条保存路径结构性丢掉用户预制副本；本判据在修复前必红。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::path::PathBuf;
use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::Preset;
use yeban_ui_test_port::port::{Permission, UiTestPort};

const NOW_MS: u64 = 1_760_000_000_000;
const USER_ID: &str = "user:01JFORCESAVE0000000000000";

fn embedded_bytes() -> Vec<u8> {
    let mut preset =
        Preset::parse(include_str!("../../../assets/presets/piano-warm-room.json").as_bytes())
            .expect("工厂预制必须能解析");
    preset.id = USER_ID.to_owned();
    preset.name = "Force Save Embedded".to_owned();
    serde_json::to_vec(&preset).expect("预制必须能序列化")
}

#[test]
fn control_plane_force_save_writes_the_referenced_embedded_user_preset() {
    let scratch = std::env::temp_dir().join(format!(
        "yeban-preset-force-save-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&scratch).expect("建输出目录");
    let target: PathBuf = scratch.join("force-save.yeban");

    let mut project = yeban_model::samples::demo_project();
    let track_id = project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
        .expect("默认工程至少有一条非主总线轨")
        .id;
    let entry = yeban_model::preset::embedded_entry_name(USER_ID).expect("条目名");
    let bytes = embedded_bytes();
    project.tracks.get_mut(&track_id).expect("轨存在").preset =
        Some(Box::new(yeban_model::preset::TrackPresetState {
            preset_id: Some(USER_ID.to_owned()),
            ..Default::default()
        }));

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<force-save>", "yeban-app", project.clone(), NOW_MS).expect("会话"),
    ));
    port.replace_embedded_presets(vec![(entry.clone(), bytes)]);
    port.replace_assets(yeban_model::samples::demo_project_assets());

    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Administrative,
            save_path: Some(target.clone()),
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    let mut live_plane = ui.into_control_plane(Permission::Administrative);
    let saved = live_plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":1,"method":"ui/force_save","params":{}}"#);
    assert_eq!(saved.status, 200, "控制面保存必须成功: {saved:?}");
    assert!(
        saved.result.is_some(),
        "ui/force_save 必须返回回执: {saved:?}"
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
    let parsed = yeban_mcp::domain::preset::parse_embedded_presets(&archive.embedded_presets)
        .expect("必须能从归档解析内嵌预制");
    assert_eq!(
        parsed.get(USER_ID).map(|preset| preset.name.as_str()),
        Some("Force Save Embedded")
    );

    std::fs::remove_dir_all(&scratch).ok();
}
