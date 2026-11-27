//! 判据：**缺失音源面板的文案必须指向面板里真的存在的入口**，且阻断错误不能被
//! 首启引导整块盖住。
//!
//! ## 判据表
//!
//! | 判据 | 答的问题 | 证什么 / **不**证什么 |
//! | :--- | :--- | :--- |
//! | [`a_no_candidate_row_offers_a_real_library_entry`] | 清单里连同类都没有时，用户点得到入口吗 | 摘要/行提示不再让点不存在的「下载」；真实指针点面板内「去音源管理」⇒ 音源管理打开、救援面板收起。⛔ 不证用户在馆里一定装得上第三方音源。 |
//! | [`first_launch_onboarding_does_not_cover_the_blocking_rescue_panel`] | 首启引导与阻断错误同时出现时，谁让位 | 两层同时在场，真实指针仍能落到救援面板的入口并打开音源管理。⛔ 不证像素观感（那要截图门）。 |
//! | [`a_candidate_download_still_opens_the_filtered_library`] | 有候选时原下载路径有没有被改坏 | 点候选「下载 X」⇒ 音源管理在前台、搜索框仍按包 id 筛好。 |
//!
//! ## 边界
//!
//! 本文件不联网、不下载；「去音源管理」只证面板到音源管理的接线与 z 序，
//! 不证第三方 SFZ 的实际安装。候选路径用一条**未安装**的同候选夹具，不依赖本机装了什么。

use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge;
use yeban_app::bridge::ViewState;
use yeban_app::help;
use yeban_app::host;
use yeban_app::missing_sounds::{Kind, MissingSound, Suggestion};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

const WINDOW: Size = Size::new(1440, 1000);
const NOW_MS: u64 = 1_760_000_000_000;
const MISSING_PATH: &str = "assets/samples/vendor-only/Kazoo.sfz";

/// 一条清单里没有同类、也没有原音源包 id 的轨：面板里不存在任何「下载」按钮。
fn no_candidate_row() -> MissingSound {
    MissingSound {
        track_index: 0,
        track_name: "Kazoo 主旋律".to_owned(),
        path: MISSING_PATH.to_owned(),
        pack_id: None,
        pack_name: None,
        kind: Kind::Other,
        suggestions: Vec::new(),
    }
}

/// 一条有候选、但候选尚未安装的轨：行内应出现「下载 X」。
fn candidate_row() -> MissingSound {
    MissingSound {
        track_index: 0,
        track_name: "Test Lead".to_owned(),
        path: MISSING_PATH.to_owned(),
        pack_id: Some("test-pack".to_owned()),
        pack_name: Some("Test Pack".to_owned()),
        kind: Kind::Other,
        suggestions: vec![Suggestion {
            id: "test-pack".to_owned(),
            name: "Test Pack".to_owned(),
            path: "assets/samples/test-pack/Test.sfz".to_owned(),
            installed: false,
        }],
    }
}

fn live_window(row: MissingSound) -> (LivePort<yeban_app::ui::MainWindow>, Rc<UndoPort>) {
    let project = bridge::demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<缺音源面板入口>", "yeban-app", project, NOW_MS).expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    let ui = live.ui();
    host::wire_sound_library_dialog_with(ui, Some(Rc::clone(&port)));
    host::wire_missing_sounds_dialog(ui, Rc::clone(&port), vec![row]);
    (live, port)
}

/// 真实指针按下 + 释放（先重抓几何；⛔ 不直接 invoke 回调）。
fn click(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) {
    port.refresh_tree(None).expect("状态变化后必须能重抓控件树");
    let node = port
        .tree()
        .find_by_id(id)
        .unwrap_or_else(|| panic!("控件树里必须有 {id}"));
    let bounds = node.bounds.unwrap_or_else(|| panic!("{id} 必须有几何"));
    assert!(
        bounds.width > 0 && bounds.height > 0,
        "{id} 的几何必须非零：{bounds:?}"
    );
    port.dispatch_pointer_down(id, 4.0, 4.0, PointerButton::Left)
        .expect("指针按下必须落到活窗口");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");
}

/// 无候选 ⇒ 文案不再让点下载，且面板内真的能点到「去音源管理」。
#[test]
fn a_no_candidate_row_offers_a_real_library_entry() {
    let (mut live, _port) = live_window(no_candidate_row());
    let ui = live.ui();
    assert!(ui.get_missing_sounds_open(), "有缺失就必须弹出救援面板");
    assert_eq!(ui.get_missing_row_count(), 1);

    let summary = ui.get_missing_sounds_summary().to_string();
    assert!(
        !summary.contains("点「下载」"),
        "无候选行没有任何下载按钮，摘要不能再指向「下载」：{summary:?}"
    );
    assert!(
        summary.contains("音源管理"),
        "摘要必须指向真实存在的音源管理入口：{summary:?}"
    );

    let hint = ui
        .get_missing_hints()
        .row_data(0)
        .expect("只有一行，必须有行提示")
        .to_string();
    assert!(
        !hint.contains("点「下载」"),
        "无候选行提示不能再指向「下载」：{hint:?}"
    );
    assert!(
        hint.contains("音源管理"),
        "无候选行提示必须指向音源管理入口：{hint:?}"
    );

    click(&mut live, "missing-sounds-browse-0");

    let ui = live.ui();
    assert!(
        ui.get_sound_library_open(),
        "点「去音源管理」必须真的打开音源管理"
    );
    assert!(
        !ui.get_missing_sounds_open(),
        "音源管理在前台时救援面板必须让位"
    );
}

/// 首启引导同时在场 ⇒ 阻断性的救援面板必须仍在最上面，入口可点。
#[test]
fn first_launch_onboarding_does_not_cover_the_blocking_rescue_panel() {
    let (mut live, _port) = live_window(no_candidate_row());
    let ui = live.ui();
    help::open_onboarding(ui);
    assert!(ui.get_onboarding_open(), "夹具必须真的把首启引导打开");

    click(&mut live, "missing-sounds-browse-0");

    let ui = live.ui();
    assert!(
        ui.get_sound_library_open(),
        "点击被首启引导吞掉：救援面板必须先于引导处理阻断错误"
    );
    assert!(
        !ui.get_missing_sounds_open(),
        "入口真的走通后救援面板才收起"
    );
}

/// 有候选 ⇒ 原「下载 X」路径不回归：面板让位、音源管理按包 id 筛好。
#[test]
fn a_candidate_download_still_opens_the_filtered_library() {
    let (mut live, _port) = live_window(candidate_row());
    let ui = live.ui();
    assert!(ui.get_missing_sounds_open());
    let summary = ui.get_missing_sounds_summary().to_string();
    assert!(
        summary.contains("点「下载」"),
        "有候选行仍应保留下载路径：{summary:?}"
    );

    click(&mut live, "missing-sounds-candidate-0-1");

    let ui = live.ui();
    assert!(ui.get_sound_library_open(), "点「下载 X」必须打开音源管理");
    assert!(!ui.get_missing_sounds_open(), "去下载时救援面板必须让位");
    assert_eq!(
        ui.get_sound_library_search().to_string(),
        "test-pack",
        "音源管理必须仍按被点候选的包 id 筛好"
    );
}
