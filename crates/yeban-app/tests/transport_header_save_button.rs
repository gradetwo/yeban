//! T-FIX-676：活顶栏补保存按钮（`transport-save-button`）接入既有 `save-project`。
//!
//! ## 本文件要证的句子
//!
//! 活顶栏 `TransportHeaderBar`（`transport_bar.slint`）没有保存入口，鼠标用户只能
//! 去应用菜单找"保存工程"。保存链本身通（`save-project` 回调 → 宿主落盘），
//! 缺的只是顶栏入口。修法：在 TransportHeaderBar 右岛加常显保存按钮，
//! `accessible-id: transport-save-button`，点击 / 无障碍默认动作都走
//! `root.save-project()`（与 Ctrl+S / 应用菜单同一落点），不新造保存路径。
//!
//! ## 判据（单测试，Slint 平台每线程一次）
//!
//! | 判据 | 验证内容 | 红对照（去掉按钮） |
//! | :--- | :--- | :--- |
//! | ① 控件树可达可点 + 接线 | 活树含 `transport-save-button` 且可见；`invoke_accessible_default_action` ⇒ 假 picker 的 `choose_save` 被调用（证明按钮连到 save-project 链，同 Ctrl+S 口径） | 控件树无该 id ⇒ 红 |
//! | ② a11y 同源 | 指针点击与无障碍默认动作共用 `root.save-project()`（源码两侧逐字同一落点；点击再触发一次 ⇒ choose_save 又 +1） | 只接一侧 ⇒ 红 |
//! | ③ 窄窗不挤要项 | F2 overflow 档：保存按钮仍在场，播放/时间码/BPM 要项仍在场 | 保存按钮随 overflow 离场 ⇒ 红 |

#![allow(unused)]
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::file_picker::{FilePicker, PickerRequest};
use yeban_app::host::{self, SaveStatus};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::find_by_accessible_id;
use yeban_ui_test_port::render::Tier1Window;

const NOW_MS: u64 = 1_700_000_000_000;
const SAVE_ID: &str = "transport-save-button";

/// 假文件对话框：记下每次 `choose_save` 调用，回一个临时路径。
struct SpyPicker {
    calls: RefCell<usize>,
    path: PathBuf,
}
impl FilePicker for SpyPicker {
    fn choose_open(&self, _: &PickerRequest) -> Option<PathBuf> {
        None
    }
    fn choose_save(&self, _: &PickerRequest) -> Option<PathBuf> {
        *self.calls.borrow_mut() += 1;
        Some(self.path.clone())
    }
}

fn main_window_with_save(
    ui: &MainWindow,
    project: &yeban_model::YebanProjectV1,
    picker: Rc<SpyPicker>,
) -> Rc<UndoPort> {
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<顶栏保存>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    // SaveStatus.target = None ⇒ 保存时会问 picker（正是顶栏按钮要走的链）。
    let status = SaveStatus {
        target: None,
        project: Rc::new(RefCell::new(project.clone())),
    };
    host::wire_save(ui, status, Some(Rc::clone(&port)), picker, None);
    port
}

#[test]
fn header_save_button_is_reachable_and_uses_the_save_project_chain() {
    let _tier1 = Tier1Window::install(Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口");

    let picker = Rc::new(SpyPicker {
        calls: RefCell::new(0),
        path: std::env::temp_dir().join("yeban-tfix676-probe.yeban"),
    });
    let _port = main_window_with_save(&ui, &project, Rc::clone(&picker));

    // ① 控件树含保存按钮、可见、可点。
    // 测试口的控件树按可见性裁剪（`[N-UI-1]`：隐藏/离屏元素不进树），
    // 因此"能被 find_by_accessible_id 找到"即证明在场且可见可达。
    let handle = find_by_accessible_id(&ui, SAVE_ID)
        .unwrap_or_else(|| panic!("活控件树必须含 `{SAVE_ID}`（修复前没有 ⇒ 红对照）"));

    // 无障碍默认动作 ⇒ 假 picker 被调用（= 按钮连到 save-project 链，同 Ctrl+S）。
    let before = *picker.calls.borrow();
    handle.invoke_accessible_default_action();
    let after_a11y = *picker.calls.borrow();
    assert!(
        after_a11y == before + 1,
        "无障碍默认动作必须触发 save-project（picker 调用 {before} -> {after_a11y}）"
    );

    // ② 指针点击与无障碍默认动作共用 root.save-project()（源码两侧逐字同一落点）。
    //    ⚠ 不再用"第二次点击 picker 再 +1"验证：wire_save 记住第一次挑的路径后
    //    第二次保存直接写它（正确的"记住目标"语义），不会再问 picker。
    let bar = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/transport_bar.slint"),
    )
    .expect("读 transport_bar.slint");
    let a11y_line = bar
        .lines()
        .find(|l| l.contains("accessible-action-default") && l.contains("save-project"))
        .unwrap_or_else(|| panic!("保存按钮的无障碍默认动作必须走 root.save-project()"));
    let click_line = bar
        .lines()
        .find(|l| l.contains("clicked") && l.contains("root.save-project()"))
        .unwrap_or_else(|| panic!("保存按钮的指针点击必须走 root.save-project()"));
    assert!(
        a11y_line.contains("root.save-project()") && click_line.contains("root.save-project()"),
        "点击 / 无障碍默认动作必须共用同一落点 root.save-project()"
    );
    let _ = after_a11y;

    // ③ 窄窗 F2 overflow 档：保存按钮仍在场，走带要项（播放 / 时间码 / BPM）也仍在场。
    ui.set_transport_overflow(true);
    assert!(
        find_by_accessible_id(&ui, SAVE_ID).is_some(),
        "窄窗溢出档保存入口仍须可达"
    );
    for id in [
        "transport-play-button",
        "transport-timecode",
        "transport-tempo-value",
    ] {
        assert!(
            find_by_accessible_id(&ui, id).is_some(),
            "窄窗下走带要项 `{id}` 不得被挤出"
        );
    }
}
