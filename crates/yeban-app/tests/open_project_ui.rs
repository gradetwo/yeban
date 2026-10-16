//! 判据（任务单 `#93`）：**运行期「打开工程」在 GUI 里有一个可点的入口**，而且点下去
//! **真的换掉当前工程**（取消不动一位 / 失败响亮且不毁工程）。
//!
//! ## 本单补的是哪一格
//!
//! | 层 | 现状（本单之前） |
//! | :--- | :--- |
//! | 能力（Rust） | `open::open_project_file` ✓ / `host::apply_view` ✓ / `EngineHost::reload` ✓ |
//! | 运行期入口（执行面） | `LiveUi::open_project` / 控制面 `ui/open_project` ✓ —— `#86` 乙 把**原生对话框**接在它上面（`LiveUi::open_project_via_picker`） |
//! | CLI 出口 | `yeban-app --open <path>` ✓ —— **但只在启动时读一次** |
//! | **GUI 出口（产品 `run_gui`）** | ⛔ **一个都没有**（`#86` 乙 的边界登记点名的就是这一格） |
//! | ⭐ **本单** | ⭐ `module-open-project-button`（控制台 **Tab 1 = 调音台**）⇒ `host::wire_open_project` ⇒ `UndoPort::open_project_session`（**新会话**）＋ `host::apply_view` ＋ `EngineHost::reload` ⇒ `module-status` 回执 |
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 树里有没有 `module-open-project-button` | 活窗口的运行时控件树（Tab 0 / Tab 1 各读一次） | 布尔 |
//! | `role` / `label` / `bounds` | 该节点的 `accessible-role` / `accessible-label` / 实几何 | kebab-case 角色 / 中文标签 / 像素 |
//! | `calls` | 假选择器收到的 `choose_open` 请求数 | 次 |
//! | `request.title` / `.extensions` | 最近一次请求的两个字段 | 字符串 / 条 |
//! | 工程指纹 | `UndoPort::fingerprint()`（模型容器字节的 SHA-256） | 十六进制串 |
//! | 界面的轨道行数 | `MainWindow.track-names` 的行数 | 行 |
//! | `undoable` | `UndoPort::display().undoable`（可撤销步数） | 步 |
//! | `module-status` | 宿主写的**回执行**原文（既有投影通道） | 结构化文本 |
//!
//! ## 为什么入口住在控制台 Tab 1（与 `#90` 甲**同一条纪律**）
//!
//! ⭐ `[UI-MCP-003]` 的 5 张 Linux 基准**只覆盖默认帧**（Arrangement / Tab 0 卷帘）。
//! `ai-rail` 在默认帧里是**可见的静态像素** ⇒ 在那里多一个按钮会让 5 张基准**确定性过期**，
//! 而本机是 macOS、`tests/golden/<platform>` **只有** `linux` ⇒ 本机**无法**重录。
//! `MixerConsole` 只在 `visible: root.active-tab == 1` 时可见 ⇒ 默认帧**一个像素都不画**。
//! 同一纪律的既有先例：`#90` 甲的 `module-export-midi-button`、`#86` 乙的「浏览…」按钮。
//!
//! ## 两个方向（本文件都测）
//!
//! 1. **Tab 0（默认帧）里没有它** ⇒ 5 张基准不受影响（与 `src/test_port_adapter.rs` 的
//!    `DEFAULT_VIEW_MUST_NOT_HAVE` 是同一条断言的第二种写法）；
//! 2. **Tab 1 里有它**，且注入它的**默认无障碍动作**真的换掉工程 —— 而结果**不是**
//!    "元素存在"这种空断言：断言的是**工程指纹真的换成了对话框选的那一份**、
//!    界面的轨道行数跟着换、`undoable` 归零（**新会话**）、回执的字段可读。
//!
//! ## 这条判据怎么变红（本单注入读数见交付报告）
//!
//! - `.slint` 的 `accessible-action-default` / `TouchArea.clicked` 不调 `root.open-project()`
//!   （**断开入口**）⇒ 注入之后工程一位不动 ⇒ 正向腿红；
//! - `host::wire_open_project` 不接回调 / 回执不写 `module-status` ⇒ 同上；
//! - 失败路径也去换工程（"先清空再读"）⇒ §失败腿的两条"一位不动"断言红；
//! - 换工程不换会话（只换投影）⇒ `undoable` 归零那条红；
//! - 界面重投影没跟着换 ⇒ 轨道行数那条红。
//!
//! ## 装配为什么是**生产形态**（而不是 `build_live_ui_with`）
//!
//! 本单补的那一格在**产品 `run_gui`** 里，而它建的窗口是
//! `host::build_main_window_with_console_tab`（与 `tests/production_save_ui.rs` 同一个口径）。
//! `LiveUi`（`src/live_surface.rs`）是**控制面执行面**：它自己的运行期打开入口是
//! `ui/open_project`（`undo: None` 那一种装配），工程由**执行面自己**持有 —— 与本单要补的
//! "把 `UndoPort` 里的会话换成新的一份"不是同一条腿。⇒ 本判据用**产品形态**装配，
//! 因此"产品里点一下真的换工程"这句话在判据里是可机械复核的。
//!
//! ## 平台约束：本文件只有一个 `#[test]`
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程只能成功
//! 一次（第二次 `AlreadySet`）⇒ 同款用法见 `tests/export_midi_ui.rs` / `tests/production_save_ui.rs`。

#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use common::ScriptedPicker;
use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::file_picker::{FilePicker, PickerRequest};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession, project_fingerprint};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::render::{Tier1Window, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 新入口的语义 ID（`ui/console/mixer_console.slint` ↔ `src/elements.rs` 两处必须逐字相同）。
const OPEN_BUTTON_ID: &str = "module-open-project-button";

/// 新入口的无障碍名称（与 `elements.rs` 登记的标签**逐字相同**，运行时对账会断言相等）。
const OPEN_BUTTON_LABEL: &str = "打开工程（用文件对话框挑一个 .yeban 容器并就地换掉当前工程）";

/// 既有投影通道的落点（回执行的**可寻址**节点，住在右栏 —— 默认帧里就可见）。
const MODULE_STATUS_ID: &str = "module-last-status";

/// 撤销会话的"现在"（与 `export_midi_ui.rs` 的常量同款：判据不需要墙钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 窗口尺寸（非零即合法；判据只要求"窗口真的建起来过"）。
const WINDOW: Size = Size::new(1440, 900);

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照 `file_picker_dialogs.rs`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-open-project-ui-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 读活窗口的运行时控件树（`[UI-DoD §3.6]` 的载体：全树内省 + JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 界面此刻的轨道行数（`[UI-TEST-001]` 的语义属性；⛔ 不数像素）。
fn rows(window: &MainWindow) -> usize {
    use slint::Model as _;
    window.get_track_names().iter().count()
}

/// 触发一个按钮的**默认无障碍动作**（与鼠标点击是逐字同一段落点）。
fn press(window: &MainWindow, id: &str) {
    let handle = find_by_accessible_id(window, id)
        .unwrap_or_else(|| panic!("控件树里没有 `{id}` —— 入口不在当前可见分支里（或 ID 改了）"));
    handle.invoke_accessible_default_action();
}

/// 把入口接到**本单的唯一实现**上（重复调用 = `on_*` setter 覆盖，与
/// `tests/export_midi_ui.rs` 反复调 `wire_module_actions_with` 同款）。
fn wire(
    window: &MainWindow,
    picker: &Rc<ScriptedPicker>,
    port: &Rc<UndoPort>,
    production: Option<Rc<RefCell<host::ProductionLoop>>>,
    save_first: bool,
) {
    let owned: Rc<ScriptedPicker> = Rc::clone(picker);
    // 两步：先克隆**具体**类型，再让 unsize 强制转换把它交给 `Rc<dyn FilePicker>`
    // （一步写会让 `Rc::clone` 的参数类型被期望类型反过来定成 trait 对象）。
    let picker: Rc<dyn FilePicker> = owned;
    // `after` = 装配方自己的最后一跳。本判据测的是**入口 + 换工程**这条腿，
    // 因此这里如实给一个"什么都不额外做"的装配（产品的 `run_gui` 那一个会更新心跳基准
    // 并重接三处静态快照 —— 那是 `main.rs` 的装配细节，不是本判据的对象）。
    let after: host::OpenProjectAfter = Rc::new(|_, _, _, _| Ok(()));
    host::wire_open_project(
        window,
        picker,
        Rc::new(host::PortOpenProject {
            port: Rc::clone(port),
            production,
            quanta: 0,
            after,
        }),
        save_first,
    );
}

/// 一份**真的**落盘的工程容器（用 `save` 那一条腿写出去，再用它当"打开"的输入）。
fn write_container(dir: &Path, name: &str, project: &yeban_model::YebanProjectV1) -> PathBuf {
    let path = dir.join(name);
    yeban_app::save::save_project_file(project, &path).expect("写出一份真容器");
    assert!(path.exists(), "夹具：必须真的写出一份容器");
    path
}

#[test]
fn the_gui_open_project_entry_really_swaps_the_current_project() {
    let scratch = Scratch::new();
    // 当前工程 = `filled_project()`（3 条非主总线轨）；对话框将要选的是**另一份**
    // （`demo_project()`，轨数不同）⇒ "换没换"在**两条独立读数**上都可见。
    let current = yeban_model::samples::filled_project();
    let other = yeban_model::samples::demo_project();
    let other_path = write_container(&scratch.dir, "other.yeban", &other);
    let other_fingerprint = project_fingerprint(&other).expect("另一份工程的指纹");
    let other_rows = ViewState::from_project(&other).expect("投影").tracks.len();

    // ---- 装配：**生产形态**的窗口（`run_gui` 用的同一个构造入口）＋ Tier-1 无头平台 ----
    let tier1 = Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&current).expect("投影");
    let scene = DemoScene::from_view(&view);
    // ⚠ 起始 Tab = 0（默认帧）—— 与产品启动态一致；下面显式切到 Tab 1 来对照。
    let window = host::build_main_window_with_console_tab(&view, &scene, 0).expect("生产窗口");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据 #93>", "yeban-app", current.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    // ⭐ 先做**一次可撤销的编辑** ⇒ `undoable` 从 0 变成 >0。
    // 下面用它证明"打开一个工程 = **新会话**"（游标与活跃分支从头开始，撤销不跨越打开边界）。
    let fixture = yeban_app::undo::undo_session::wiring_fixture(&port.project()).expect("夹具");
    port.commit_ops(NOW_MS + 1, "判据 #93", vec![fixture.op()])
        .expect("提交");
    let undoable_before = port.display().undoable;
    assert!(
        undoable_before > 0,
        "夹具必须先制造出可撤销的历史，否则下面那条『归零』没有对照"
    );
    let rows_current = rows(&window);
    assert_eq!(rows_current, view.tracks.len(), "初始投影必须已经注入界面");

    // 一代**真引擎**：换工程必须换代（新快照 / 新队列），而**不是**重建窗口。
    let production = Rc::new(RefCell::new(
        host::ProductionLoop::start(&current, 0).expect("起一代引擎"),
    ));

    // =====================================================================
    // 判据 ①：**默认帧（Tab 0）里没有它** ⇒ 5 张 Linux 基准不受影响
    // =====================================================================
    let cancelling = ScriptedPicker::cancelling();
    wire(
        &window,
        &cancelling,
        &port,
        Some(Rc::clone(&production)),
        false,
    );
    let tab0 = runtime_tree(&window);
    report_line(&format!(
        "[#93] 控制台 Tab 0（默认帧）运行时控件树 {} 条；含 `{OPEN_BUTTON_ID}` = {}",
        tab0.len(),
        tab0.contains(OPEN_BUTTON_ID)
    ));
    assert!(
        !tab0.contains(OPEN_BUTTON_ID),
        "⛔ 默认帧（Tab 0 = 卷帘）里不得出现 `{OPEN_BUTTON_ID}` —— 它是 `[UI-MCP-003]` 金标\
         覆盖的那一帧，多一个可见控件就会让 5 张 Linux 基准确定性过期（本机无法重录）"
    );

    // 切到 Tab 1（调音台）—— 这一跳必须**真的**发生，否则下面"在树里"可能只是别的原因。
    window.set_console_tab(1);
    let tab1 = runtime_tree(&window);
    assert!(
        tab1.contains("mixer-console"),
        "切到 Tab 1 之后 `mixer-console` 必须在树里（否则 Tab 切换没生效，下面的断言没有对照）"
    );
    report_line(&format!(
        "[#93] 控制台 Tab 1（调音台）运行时控件树 {} 条；含 `{OPEN_BUTTON_ID}` = {}",
        tab1.len(),
        tab1.contains(OPEN_BUTTON_ID)
    ));

    // =====================================================================
    // 判据 ②：**Tab 1 里有它**（ID + 角色 + 标签 + 几何），且静态注册表也在
    // =====================================================================
    let registry = ElementRegistry::from_view(&view);
    assert!(
        registry.contains(OPEN_BUTTON_ID),
        "`{OPEN_BUTTON_ID}` 不在静态语义注册表里 —— UI 加了 ID 却忘了登记（[UI-TEST-001]）"
    );
    let node = tab1.find_by_id(OPEN_BUTTON_ID).unwrap_or_else(|| {
        panic!(
            "`{OPEN_BUTTON_ID}` 不在运行时控件树里 —— 被裁掉 / 改了 accessible-id / 没接进 app.slint。\
             树里 {} 条节点",
            tab1.len()
        )
    });
    assert_eq!(
        node.role.as_str(),
        ElementKind::Button.accessible_role(),
        "入口的角色必须是 `{}`（它在 elements.rs 里登记成 Button）",
        ElementKind::Button.accessible_role()
    );
    assert_eq!(
        node.label, OPEN_BUTTON_LABEL,
        "运行时 `accessible-label` 必须与 `elements.rs` 登记的**逐字相同**"
    );
    let bounds = node.bounds.expect("可见控件必须有几何");
    report_line(&format!(
        "[#93] 入口节点: role={} label={:?} bounds={bounds:?}",
        node.role.as_str(),
        node.label
    ));
    assert!(
        bounds.width > 0 && bounds.height > 0,
        "入口必须有实几何（宽 {} × 高 {}）",
        bounds.width,
        bounds.height
    );
    // 既有投影通道的落点也在（回执行住在右栏）—— 回执要写在**它**上面。
    let status = tab1
        .find_by_id(MODULE_STATUS_ID)
        .expect("回执行 `module-last-status` 必须在树里");
    assert_eq!(status.role.as_str(), ElementKind::Text.accessible_role());
    // 初始态 = **空回执**（"本次会话还没按过任何模块入口"），不是编造的读数。
    assert_eq!(
        window.get_module_status(),
        "",
        "刚装配的状态行必须是空的 —— 任何非空值都意味着宿主在没人按之前就写了回执"
    );
    assert!(!window.get_module_status_succeeded());

    // =====================================================================
    // 判据 ③：**取消不动一位**（用户按了对话框的取消 ⇒ 什么都不做）
    // =====================================================================
    let fingerprint_before = port.fingerprint();
    press(&window, OPEN_BUTTON_ID);
    report_line(&format!(
        "[#93] 取消: choose_open 调用 {} 次；工程指纹 {fingerprint_before:?} → {:?}；\
         track-names {rows_current} → {}；回执 {:?}",
        cancelling.call_count("open"),
        port.fingerprint(),
        rows(&window),
        window.get_module_status().to_string(),
    ));
    assert_eq!(
        cancelling.call_count("open"),
        1,
        "取消也要先开一次对话框（⛔ 不能『没选就不问』）"
    );
    assert_eq!(
        cancelling.last_request("open"),
        Some(PickerRequest::open("打开工程").with_extensions(["yeban"])),
        "请求的标题与后缀过滤必须与 `open_project_via_picker` **逐字相同**"
    );
    assert_eq!(
        port.fingerprint(),
        fingerprint_before,
        "⛔ 取消之后工程指纹一位不得变"
    );
    assert_eq!(rows(&window), rows_current, "⛔ 取消之后界面行数一位不得变");
    assert_eq!(
        port.display().undoable,
        undoable_before,
        "⛔ 取消之后撤销会话一位不得变"
    );
    assert_eq!(
        window.get_module_status(),
        "",
        "取消**不是**失败：状态行必须保持空（⛔ 不写成一条错误）"
    );
    assert!(!window.get_module_status_succeeded(), "取消不得报成功");

    // =====================================================================
    // 判据 ④：**打开失败要响亮**，且**当前工程不被毁掉**（本单最重要的一条）
    // =====================================================================
    for (what, bad_target) in [
        ("不存在的路径", scratch.join("does-not-exist.yeban")),
        ("存在但不是容器", {
            let path = scratch.join("garbage.yeban");
            std::fs::write(&path, b"this is not a .yeban container").expect("写坏夹具");
            path
        }),
    ] {
        let picker = ScriptedPicker::answering_open(&bad_target);
        let rows_before = rows(&window);
        let fingerprint_now = port.fingerprint();
        let undoable_now = port.display().undoable;
        wire(&window, &picker, &port, Some(Rc::clone(&production)), false);
        press(&window, OPEN_BUTTON_ID);
        let receipt = window.get_module_status().to_string();
        report_line(&format!(
            "[#93] 坏输入（{what}）：choose_open {} 次；回执 {receipt:?}；\
             工程指纹 {fingerprint_now:?} → {:?}；track-names {rows_before} → {}",
            picker.call_count("open"),
            port.fingerprint(),
            rows(&window),
        ));
        assert_eq!(picker.call_count("open"), 1, "{what}: 必须问了恰好一次");
        assert!(
            !window.get_module_status_succeeded(),
            "{what}: 打不开必须**如实失败**，实得 {receipt:?}"
        );
        assert!(
            !receipt.is_empty(),
            "{what}: 失败必须有**非空**的错误文本（⛔ 不静默）"
        );
        assert!(
            receipt.contains("outcome=error") && receipt.contains("open-project"),
            "{what}: 错误必须走**既有的** `module-status` 投影通道（结构化回执），实得 {receipt:?}"
        );
        assert!(
            receipt.contains(bad_target.to_string_lossy().as_ref()),
            "{what}: 回执必须点名那个打不开的路径，实得 {receipt:?}"
        );
        assert_eq!(
            port.fingerprint(),
            fingerprint_now,
            "⛔ {what}: 失败**不得**把当前工程换掉/毁掉"
        );
        assert_eq!(
            rows(&window),
            rows_before,
            "⛔ {what}: 失败之后界面必须还是原来那一份工程"
        );
        assert_eq!(
            port.display().undoable,
            undoable_now,
            "⛔ {what}: 失败不得动撤销会话"
        );
    }

    // =====================================================================
    // 判据 ⑤（正向腿）：注入默认无障碍动作 ⇒ **工程真的换了**（三条独立读数）
    // =====================================================================
    let picker = ScriptedPicker::answering_open(&other_path);
    wire(&window, &picker, &port, Some(Rc::clone(&production)), false);
    assert_ne!(
        rows(&window),
        other_rows,
        "夹具必须让两份工程的轨数不同，否则这条判据没有对照"
    );
    press(&window, OPEN_BUTTON_ID);
    let receipt = window.get_module_status().to_string();
    report_line(&format!(
        "[#93] 正向：choose_open {} 次；结局 succeeded={}；回执 {receipt:?}；\
         工程指纹 {fingerprint_before:?} → {:?}（对话框选的那份 = {other_fingerprint}）；\
         track-names {rows_current} → {}；undoable {undoable_before} → {}",
        picker.call_count("open"),
        window.get_module_status_succeeded(),
        port.fingerprint(),
        rows(&window),
        port.display().undoable,
    ));
    assert!(
        window.get_module_status_succeeded(),
        "界面上的打开必须成功: {receipt}"
    );
    assert_eq!(
        picker.call_count("open"),
        1,
        "⭐ 对话框必须被问**恰好一次**（多一次 = 每次点击弹两次；少一次 = 路径来历不明）"
    );
    let request = picker.last_request("open").expect("请求");
    assert_eq!(request.title, "打开工程");
    assert_eq!(request.extensions, vec!["yeban".to_owned()]);
    // ⭐ ① 工程**真的**换成了对话框选的那一份（指纹逐字相等 —— 不是"轨数看起来对"）。
    assert_eq!(
        port.fingerprint().as_deref(),
        Some(other_fingerprint.as_str()),
        "⛔ 端口里的工程必须**逐字节**等于对话框选的那一份（指纹是模型容器字节的 SHA-256）"
    );
    assert_eq!(
        port.project().tracks.len(),
        other.tracks.len(),
        "轨道数必须换成新工程的"
    );
    // ⭐ ② 界面跟着换（唯一注入点 `host::apply_view`）。
    assert_eq!(
        rows(&window),
        other_rows,
        "⛔ 界面行数必须跟着换工程（否则界面还画着旧工程）"
    );
    // ⭐ ③ 「打开一个工程 = **新会话**」：可撤销步数归零（撤销不跨越打开边界）。
    assert_eq!(
        port.display().undoable,
        0,
        "⭐ 打开一个工程 = **新会话**：`undoable` 必须从 {undoable_before} 归零"
    );
    // ⭐ ④ 引擎跟着换代（**不是**重建窗口）：新工程的轨道必须在当前快照里。
    {
        let engine = production.borrow().engine_handle();
        let snapshot = engine
            .borrow()
            .current_snapshot()
            .expect("换工程之后必须有快照");
        assert!(
            other
                .tracks
                .keys()
                .any(|track_id| snapshot.track(track_id).is_some()),
            "⭐ 引擎必须已经换成**新**工程的那一代快照（同一个 `EngineHost::reload` 落点）"
        );
        assert!(
            !window.get_playing(),
            "换工程之后界面必须停在停止态（与装配点 `ProductionLoop::start` 之后那两行同款）"
        );
    }
    // ⭐ ⑤ 回执的字段可读，且路径逐字等于对话框给的那一条（⛔ 界面不猜路径）。
    assert!(receipt.contains("outcome=ok"), "{receipt}");
    assert!(
        receipt.contains(&format!("path={}", other_path.display())),
        "{receipt}"
    );
    assert!(
        receipt.contains(&format!("tracks={other_rows}")),
        "{receipt}"
    );
    assert!(receipt.contains("savedFirst=false"), "{receipt}");

    // =====================================================================
    // 判据 ⑥（反向腿）：`saveFirst = true` 而**保存入口没接线** ⇒ 响亮拒绝、一位不动
    // =====================================================================
    //
    // 这条腿钉住的是"先保存"那一步的口径：它**复用**已有的保存入口（`save-project` 回调），
    // 而那个入口没接线时**必须拒绝换工程**，⛔ 不把"没人管保存"当成"保存过了"
    // （未保存的改动被静默丢掉是本仓最忌讳的那类缺陷）。
    let rows_now = rows(&window);
    let fingerprint_now = port.fingerprint();
    let undoable_now = port.display().undoable;
    let picker = ScriptedPicker::answering_open(scratch.join("never-read.yeban"));
    wire(&window, &picker, &port, Some(Rc::clone(&production)), true);
    press(&window, OPEN_BUTTON_ID);
    let receipt = window.get_module_status().to_string();
    report_line(&format!(
        "[#93] saveFirst（保存入口未接线）：回执 {receipt:?}；工程指纹 {fingerprint_now:?} → {:?}",
        port.fingerprint()
    ));
    assert_eq!(picker.call_count("open"), 1, "先问对话框再谈保存");
    assert!(
        !window.get_module_status_succeeded(),
        "保存没落盘 ⇒ 必须拒绝换工程，实得 {receipt:?}"
    );
    assert!(
        receipt.contains("save-first") && receipt.contains("outcome=error"),
        "拒绝理由必须走既有的回执通道并点名 `save-first`，实得 {receipt:?}"
    );
    assert_eq!(
        port.fingerprint(),
        fingerprint_now,
        "⛔ 拒绝之后工程一位不动"
    );
    assert_eq!(rows(&window), rows_now, "⛔ 拒绝之后界面一位不动");
    assert_eq!(
        port.display().undoable,
        undoable_now,
        "⛔ 拒绝之后会话一位不动"
    );

    drop(tier1);
}
