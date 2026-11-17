//! 音源管理判据（line/sound-library-manager）：菜单落点 + 对话框挂载 +
//! 公开逻辑面。重活（目录解析四路、校验两路、装/删往返、路径安全）在
//! `src/sound_library.rs` 的内联判据里；本文件只钉**接线事实**：
//! 菜单点了必须真有宿主落点、对话框必须真被挂进主窗口。

const APP: &str = include_str!("../ui/app.slint");
const HOST: &str = include_str!("../src/host.rs");

/// ① 「设置」菜单里有「音源管理...」，且指向真命令、宿主有分发臂。
#[test]
fn settings_menu_has_sound_library_entry_with_host_landing() {
    assert!(
        APP.contains("@tr(\"音源管理...\")"),
        "设置菜单必须有「音源管理...」入口"
    );
    assert!(
        APP.contains("command-execute(\"open-sound-library\")"),
        "菜单项必须指向 open-sound-library 命令"
    );
    assert!(
        HOST.contains("\"open-sound-library\" =>"),
        "dispatch_aether_command 必须有 open-sound-library 臂"
    );
    assert!(
        HOST.contains("pub fn wire_sound_library_dialog"),
        "host 必须有音源管理对话框的接线函数"
    );
}

/// ② 对话框被 import 并在 `sound-library-open` 为真时挂载（默认关着，
/// 默认帧不变），且把宿主的平行数组与四条回调都接上。
#[test]
fn dialog_is_imported_and_mounted() {
    assert!(
        APP.contains("import { SoundLibraryDialog } from \"dialogs/sound_library_dialog.slint\""),
        "app.slint 必须 import 音源管理对话框"
    );
    assert!(
        APP.contains("if root.sound-library-open : SoundLibraryDialog"),
        "对话框必须挂在 sound-library-open 条件下"
    );
    for binding in [
        "installed-names: root.sound-installed-names",
        "pack-names: root.sound-pack-names",
        "catalog-state: root.sound-catalog-state",
        "download-pack(id) => { root.sound-library-download(id); }",
        "delete-library(id) => { root.sound-library-delete(id); }",
    ] {
        assert!(APP.contains(binding), "挂载缺少绑定：{binding}");
    }
}

/// ③ 公开逻辑面：目录解析在集成层同样成立（空目录是真实状态、版本
/// 不符如实拒绝），且固定目录地址与格式版本和 yeban-extras 的
/// `sounds/catalog.json` 契约一致。
#[test]
fn catalog_contract_is_pinned() {
    assert_eq!(
        yeban_app::sound_library::CATALOG_URL,
        "https://raw.githubusercontent.com/gradetwo/yeban-extras/main/sounds/catalog.json"
    );
    assert_eq!(yeban_app::sound_library::CATALOG_FORMAT_VERSION, 1);
    let empty = yeban_app::sound_library::parse_catalog(r#"{"version": 1, "packs": []}"#)
        .expect("空目录必须解析成功");
    assert!(empty.packs.is_empty());
    assert!(matches!(
        yeban_app::sound_library::parse_catalog(r#"{"version": 2, "packs": []}"#),
        Err(yeban_app::sound_library::CatalogError::UnsupportedVersion(
            2
        ))
    ));
}
