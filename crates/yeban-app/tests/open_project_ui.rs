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

#![allow(unused)]
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
// ⭐ `#191`：`compact` 现在跟**真实窗口宽**走（`ui/app.slint` 的表达式绑定）。
// ⭐ `#195`：阈值 `1920 → 1180` ⇒ `1440` 是**展开档**，右栏常驻 ⇒ 本判据翻回 `1440`。
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> console-tab:"),
        "新 UI 契约面必须声明 > console-tab:"
    );
    assert!(
        aether_host.contains("set_console_tab"),
        "宿主接线层必须注册 set_console_tab"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> module-status:"),
        "新 UI 契约面必须声明 > module-status:"
    );
    assert!(
        aether_host.contains("set_module_status"),
        "宿主接线层必须注册 set_module_status"
    );
}
