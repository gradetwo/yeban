//! **T-FIX-797 判据**：运行中打开另一工程后，脏标记基线换成新会话。
//!
//! 场景：保存 A（记下 A 的 head + 指纹）→ 运行中打开 B → 不编辑 → 退出判定。
//! 修前基线仍是 A，`current_is_dirty(B.head)` 为真 ⇒ 误弹「未保存」。
//! 修后基线随换会话更新为 B 的 head + 指纹 ⇒ 退出不拦。
//!
//! 本文件只有一个 `#[test]`：`Tier1Window::install` 每线程只能成功一次。

use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::dirty_guard::{self, SavedHead};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::container::ProjectArchive;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::render::Tier1Window;

const NOW: u64 = 1_760_000_000_000;

fn archive(project: yeban_model::YebanProjectV1) -> ProjectArchive {
    ProjectArchive {
        project,
        history_dag: Vec::new(),
        proposals: None,
        assets: Vec::new(),
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    }
}

#[test]
fn opening_another_project_resets_the_dirty_baseline() {
    let tier1 = Tier1Window::install(Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let project_a = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project_a).expect("投影 A");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 1).expect("建窗口");
    tier1.resize(Size::new(1440, 900)).expect("resize");

    let port = Rc::new(UndoPort::new(
        UndoSession::open("t-fix-797-a", "yeban-app", project_a, NOW).expect("打开 A 会话"),
    ));
    // 「保存 A」：记下 A 的 head + 文档指纹，并安装给退出判定读。
    let saved = Rc::new(SavedHead::new());
    saved.mark_saved(port.display().head);
    saved.mark_saved_fingerprint(port.fingerprint());
    dirty_guard::install(Rc::clone(&saved));
    dirty_guard::set_current_fingerprint(port.fingerprint());
    assert!(
        !dirty_guard::current_is_dirty(port.display().head),
        "刚保存的 A 必须干净"
    );

    let a_head = port.display().head;
    let a_fingerprint = port.fingerprint();
    let project_b = yeban_model::samples::default_project();
    let opener = host::PortOpenProject {
        port: Rc::clone(&port),
        production: None,
        quanta: 0,
        after: Rc::new(|_, _, _| Ok(())),
    };
    opener
        .open_archive_runtime(&ui, "t-fix-797-b", archive(project_b), None, None)
        .expect("运行中打开 B 必须成功");

    let b_head = port.display().head;
    let b_fingerprint = port.fingerprint();
    assert_ne!(a_head, b_head, "换工程必须换会话头");
    assert_ne!(a_fingerprint, b_fingerprint, "B 的文档指纹必须不同于 A");

    // 退出判定走的就是这两个线程槽读数。
    dirty_guard::set_current_fingerprint(b_fingerprint.clone());
    assert_eq!(
        saved.saved(),
        b_head,
        "保存点 head 必须跟着换到 B（否则退出拿 A 的头比 B ⇒ 误报）"
    );
    assert_eq!(
        saved.saved_fingerprint(),
        b_fingerprint,
        "保存点指纹必须跟着换到 B"
    );
    assert!(
        !dirty_guard::current_is_dirty(b_head),
        "打开 B 后不编辑 ⇒ 退出不得被拦"
    );
}
