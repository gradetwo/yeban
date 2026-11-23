//! T-FIX-162 判据：走带 TEMPO 的 `bpm-commit` 必须有活的触发面。
//!
//! 旧界面只有点击三档循环；`app.slint` 的 `bpm-commit` 只有声明、没有任何子控件
//! 触发，宿主 `wire_bpm_commit` 是死线。本判据同时检查三层：
//! 1. `app.slint` import 的 `transport_bar.slint` 里存在可编辑 `ImeAwareTextInput`
//!    且 `accepted` 调用 `root.bpm-commit(self.text)`；
//! 2. `app.slint` 把子控件的 `bpm-commit` 转发到根回调；
//! 3. 活 `LivePort` 控件树能按稳定 ID 找到 TEMPO 点击区与 BPM 输入面，宿主
//!    `wire_bpm_commit` 能把 `132.5` 写成工程值并把非法输入还原为权威读数。

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use slint::LogicalPosition;
use slint::platform::{Key, PointerEventButton};
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::input::InputContext;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

fn build_live() -> (LivePort<yeban_app::ui::MainWindow>, Rc<UndoPort>) {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("filled_project 必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<t-fix-162>", "yeban-app", project, NOW_MS).expect("撤销会话必须能打开"),
    ));
    let live = LivePort::new(
        Size::new(1440, 900),
        Permission::Interactive,
        None,
        move || host::build_main_window(&view, &scene),
    )
    .expect("Tier-1 活窗口");
    host::wire_input(live.ui(), Rc::new(RefCell::new(InputContext::default())));
    host::wire_bpm_commit(live.ui(), &undo);
    (live, undo)
}

#[test]
fn transport_tempo_input_is_wired_and_commits_fractional_bpm() {
    let (mut live, undo) = build_live();

    // 1. 源码层：触发面在被 app.slint import 的 transport_bar.slint 里。
    let ui_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let transport =
        fs::read_to_string(ui_dir.join("transport_bar.slint")).expect("读取 transport_bar.slint");
    assert!(
        transport.contains("bpm-input := ImeAwareTextInput"),
        "TEMPO 区必须存在 ImeAwareTextInput，不能只剩点击循环"
    );
    assert!(
        transport.contains("accepted =>") && transport.contains("root.bpm-commit(self.text)"),
        "BPM 输入必须在 accepted 时触发 bpm-commit"
    );
    assert!(
        transport.contains("double-clicked =>")
            && transport.contains("bpm-input.focus()")
            && transport.contains("root.bpm-cycle-pending = false"),
        "TEMPO 点击区必须能双击进入编辑、聚焦输入并取消待执行的三档循环"
    );
    assert!(
        transport.contains("bpm-click-timer := Timer")
            && transport.contains("interval: 500ms;")
            && transport.contains("running: root.bpm-cycle-pending")
            && transport.contains("root.change-bpm(next-bpm)"),
        "TEMPO 单击三档循环必须由 bpm-click-timer 在双击窗口后执行"
    );
    assert!(
        transport.contains("clicked => {") && transport.contains("root.bpm-cycle-pending = true"),
        "TEMPO 单击只能登记待执行循环，不能立即改速"
    );

    let app = fs::read_to_string(ui_dir.join("app.slint")).expect("读取 app.slint");
    assert!(
        app.contains("import { TransportHeaderBar } from \"transport_bar.slint\";")
            && app.contains("bpm-commit(value) => { root.bpm-commit(value); }"),
        "app.slint 必须把活树里的 TransportHeaderBar.bpm-commit 转发到根回调"
    );

    // 2. 活控件树层：稳定 ID 必须真的可寻址，而不是源码孤岛。
    let tree = live.refresh_tree(None).expect("抓取活控件树");
    let tempo = tree
        .find_by_id("transport-tempo-button")
        .expect("TEMPO 点击区必须在活树里");
    assert_eq!(tempo.role.as_str(), "button", "TEMPO 点击区角色");
    assert!(
        tempo.label.contains("双击输入任意速度"),
        "TEMPO 点击区标签必须告诉用户可双击编辑，实际 {:?}",
        tempo.label
    );

    let input = tree
        .find_by_id("transport-bpm-field")
        .expect("BPM 输入面必须在活树里");
    assert_eq!(input.role.as_str(), "text-input", "BPM 输入面角色");
    assert!(
        input.value.as_deref() == Some("128.00"),
        "BPM 输入面必须显示宿主投影的权威读数，实际 {:?}",
        input.value
    );

    // 3. 真实事件路径：双击 TEMPO 点击区进入编辑，键入 132.5 后回车。
    let bounds = tempo.bounds.expect("TEMPO 点击区必须有几何");
    let center = LogicalPosition::new(
        (bounds.x + i32::try_from(bounds.width / 2).unwrap_or(0)) as f32 + 0.5,
        (bounds.y + i32::try_from(bounds.height / 2).unwrap_or(0)) as f32 + 0.5,
    );
    for _ in 0..2 {
        live.window().pointer_down(center, PointerEventButton::Left);
        live.window().pointer_up(center, PointerEventButton::Left);
        std::thread::sleep(Duration::from_millis(80));
    }

    live.ui().set_bpm_display("".into());
    assert!(
        live.ui().get_bpm_field_editing(),
        "双击 TEMPO 后宿主必须收到 BPM 焦点进入编辑"
    );
    assert_eq!(
        live.ui().get_bpm_display().as_str(),
        "",
        "双击后 BPM 输入面必须可编辑并接受清空"
    );
    for ch in "132.5".chars() {
        live.window().type_char(ch);
    }
    assert_eq!(
        live.ui().get_bpm_display().as_str(),
        "132.5",
        "键入内容必须实时写回 bpm-display"
    );
    live.window().key_press(Key::Return);

    // 4. 宿主行为：合法小数写入工程并按两位小数回写；非法输入回写权威值。
    let project = undo.try_project().expect("提交后必须仍有工程");
    assert!(
        (project.bpm - 132.5).abs() < 1e-9,
        "132.5 必须写入工程，实际 {}",
        project.bpm
    );
    assert_eq!(
        live.ui().get_bpm_display().as_str(),
        "132.50",
        "合法 BPM 必须回写成两位小数"
    );

    live.ui().invoke_bpm_commit("not-a-number".into());
    let project = undo.try_project().expect("非法提交后必须仍有工程");
    assert!(
        (project.bpm - 132.5).abs() < 1e-9,
        "非法输入不能改工程，实际 {}",
        project.bpm
    );
    assert_eq!(
        live.ui().get_bpm_display().as_str(),
        "132.50",
        "非法输入后读数必须回到权威值"
    );

    // 5. 第二次双击 + 真实键入非法值：回车后输入面本身也必须回到权威读数，
    //    不能只改根属性而把 "abc" 草稿留在控件里。
    for _ in 0..2 {
        live.window().pointer_down(center, PointerEventButton::Left);
        live.window().pointer_up(center, PointerEventButton::Left);
        std::thread::sleep(Duration::from_millis(80));
    }
    live.ui().set_bpm_display("".into());
    for ch in "abc".chars() {
        live.window().type_char(ch);
    }
    live.window().key_press(Key::Return);

    let tree = live.refresh_tree(None).expect("抓取非法提交后的控件树");
    let input = tree
        .find_by_id("transport-bpm-field")
        .expect("BPM 输入面必须在活树里");
    assert_eq!(
        input.value.as_deref(),
        Some("132.50"),
        "非法输入后输入面草稿必须被权威读数替换"
    );
    assert!(
        (undo.try_project().expect("工程").bpm - 132.5).abs() < 1e-9,
        "非法输入不能改工程"
    );
}
