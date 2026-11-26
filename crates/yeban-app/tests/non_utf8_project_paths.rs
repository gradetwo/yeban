//! T-FIX-310：非 UTF-8 工程路径的端到端判据（Linux / Unix）。
//!
//! 旧链在 `wire_open_project` / `on_new_project` / 启动器里拿到 `PathBuf` 后
//! 立刻 `to_string_lossy()`，文件名里的 `0xFF` 被换成 `�`：
//! - 打开：读盘读的是一个**不存在的**路径 ⇒ 失败；
//! - 最近：`settings.json` 里存的是 `�` ⇒ 重启后仍打不开；
//! - 新建：落盘写到 `�` 路径。
//!
//! 本判据用**真实** `0xFF` 文件名走完三条腿：
//! 写出工程 → 最近清单存盘/重启回读 → `PortOpenProject::open_runtime` 真换工程
//! → `create_new_project_file` 把空工程写到另一个非 UTF-8 路径。
//!
//! ⚠ 本文件只有一个 `#[test]`：`slint::platform::set_platform` 是线程局部的，
//! `Tier1Window::install` 每线程只能成功一次（与 `tests/open_project_ui.rs` 同一条约束）。

#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host::{self, OpenProjectRuntime};
use yeban_app::scene::DemoScene;
use yeban_app::settings;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_760_000_000_000;

fn non_utf8_name(stem: &[u8], suffix: &[u8]) -> OsString {
    let mut bytes = stem.to_vec();
    bytes.push(0xFF);
    bytes.extend_from_slice(suffix);
    OsString::from_vec(bytes)
}

#[test]
fn non_utf8_project_paths_survive_open_recent_and_new_project_round_trip() {
    let dir = std::env::temp_dir().join(non_utf8_name(b"yeban-non-utf8-", b""));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");

    // ---- ① 磁盘上真有一份文件名含 0xFF 的工程 ----
    //
    // ⭐ T-FIX-436：落盘必须按**生产装载形态**带上资产字节。
    // `filled_project()` 的 Audio 片段引用会话资产池里的 kick，而
    // `save_project_file` 的契约明确是"history.dag 空 + 资产池空"，含 Audio
    // 片段时在写任何字节之前就 `RequiresArchive` 响亮拒绝（T-FIX-329）。
    // 这里改走全保真归档入口（同一处唯一写入口），容器里才真的有被引用的
    // `assets/{sha256}`；③ 的严格打开（T-FIX-330）才不会报"缺 1 个资产"。
    // ⛔ 不是放宽：`open_runtime` 的 `strict_missing_assets: true` 一字未改，
    // 缺资产工程经 GUI 打开仍响亮失败。
    // 同一机理的先例：`tests/project_templates.rs`（T-FIX-411）。
    let project = yeban_model::samples::filled_project();
    let referenced_assets = yeban_model::samples::filled_project_assets();
    assert_eq!(referenced_assets.len(), 1, "filled 夹具应恰好带一份 kick");
    assert!(
        referenced_assets
            .iter()
            .any(|(hash, _)| project.assets.contains_key(hash)),
        "夹具前提：filled 工程的资产索引必须引用这份 kick"
    );
    let project_path = dir.join(non_utf8_name(b"fixture-", b".yeban"));
    let kick_hash = referenced_assets[0].0.clone();
    let archive = yeban_model::container::ProjectArchive {
        project,
        // `history.dag` 恒空字节：本判据只判路径编解码，提交图谱不是它的关注点。
        history_dag: Vec::new(),
        proposals: None,
        assets: referenced_assets,
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    };
    yeban_app::save::save_archive_file(&archive, &project_path).expect("写出非 UTF-8 工程");
    // 容器里**真的**有被 Audio 片段引用的字节：不是"存了但池空"，否则 ③ 的严格
    // 打开必报缺资产。这一条与 ③ 是两条腿：它钉保存侧，③ 钉打开侧。
    let round = yeban_app::open::open_project_archive_file(
        &project_path,
        &yeban_app::open::ProjectOpenOptions::default(),
    )
    .expect("刚写出的工程必须能按生产口径读回");
    assert!(
        round.assets.iter().any(|(hash, _)| *hash == kick_hash),
        "容器必须真的带上被 Audio 片段引用的 kick 字节（否则 ③ 的严格打开必报缺资产）"
    );
    assert!(project_path.exists(), "工程必须真的落在原路径上");
    assert!(
        project_path.to_str().is_none(),
        "夹具前提：文件名必须真的不是 UTF-8"
    );

    // ---- ② 最近清单写盘 → 模拟重启回读：路径必须逐字节回来 ----
    //
    // `settings::save_to` 是 crate-private（产品只经 `record_recent_project` 写），
    // 这里直接写它**写出的同一种 JSON 形状**（`path_bytes` 数组），再走公开的
    // `load_from` —— 正好判"重启读回"这一跳；保存端的往返由 `settings.rs` 单测覆盖。
    let settings_path = dir.join("settings.json");
    let bytes = project_path.as_os_str().as_bytes();
    let bytes_json = bytes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        &settings_path,
        format!(
            r#"{{"version":1,"recent_projects":[{{"path_bytes":[{bytes_json}],"opened_at_unix_ms":42}}]}}"#
        ),
    )
    .expect("写设置");
    let reloaded = settings::load_from(&settings_path);
    assert_eq!(reloaded.recent_projects.len(), 1, "最近条目不得丢");
    assert_eq!(
        reloaded.recent_projects[0].path, project_path,
        "重启后最近路径必须逐字节一致（⛔ 不能被 to_string_lossy 换成 �）"
    );
    let recent_path = reloaded.recent_projects[0].path.clone();

    // ---- ③ 走生产形态的 open_runtime：真读盘、真换工程 ----
    let initial = yeban_model::samples::default_project();
    let view = ViewState::from_project(&initial).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<non-utf8-path>", "yeban-app", initial, NOW_MS).expect("打开会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    let ui = live.ui();
    let after: host::OpenProjectAfter = Rc::new(|_, _, _| Ok(()));
    let runtime = host::PortOpenProject {
        port: Rc::clone(&port),
        production: None,
        quanta: 0,
        after,
    };
    runtime
        .open_runtime(ui, &recent_path)
        .expect("非 UTF-8 路径必须能打开");
    let rows = slint::Model::row_count(&ui.get_track_names());
    assert!(
        rows > 0,
        "打开后界面必须换成新工程（轨道行数应 > 0，实际 {rows}）"
    );

    // ---- ④ 新建工程到另一个非 UTF-8 路径：必须落盘到原路径 ----
    let new_path = dir.join(non_utf8_name(b"new-", b".yeban"));
    host::create_new_project_file(&new_path).expect("新建工程落盘");
    assert!(
        new_path.exists(),
        "新建工程必须写到原路径（⛔ 不是 � 路径）"
    );
    let opened = yeban_app::open::open_project_document_file(&new_path, &Default::default())
        .expect("刚写出的新工程必须能再读回");
    assert_eq!(
        opened.archive.project.tracks.len(),
        1,
        "新工程容器应只有主总线一条轨"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
