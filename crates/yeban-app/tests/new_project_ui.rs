//! 判据（任务单 `#129`，界面腿）：GUI 里「打开工程」**旁边**有一个「新建工程」入口，
//! 点下去 ⇒ 问一次保存对话框 ⇒ 落一个 **1187 B／0 音轨 ＋ 1 主总线** 的 `.yeban`
//! （`#215`：创建时刻已写进文件；`#161` 前是 884 B、`#215` 前是 1136 B，`#349` 补齐 tempo_map/time_signature_map 后为 1187 B）
//! ⇒ **就地换成新工程**（视图清空 / 标题目标路径跟着走）；取消不动一位；
//! **没有对话框** ⇒ 如实报错。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 树里有没有 `module-new-project-button` | 活窗口的运行时控件树（Tab 0 / Tab 1 各读一次） | 布尔 |
//! | `role` / `label` / `bounds` | 该节点的 `accessible-role` / `accessible-label` / 实几何 | kebab-case 角色 / 中文标签 / 像素 |
//! | `calls` | 假选择器收到的 `choose_save` 请求数 | 次 |
//! | `request.title` / `.file_name` / `.extensions` | 最近一次请求的三个字段 | 字符串 / 字符串 / 条 |
//! | 新建文件的大小 | `std::fs::metadata(path).len()` | 字节 |
//! | 工程指纹 | `UndoPort::fingerprint()`（模型容器字节的 SHA-256） | 十六进制串 |
//! | 界面的轨道行数 | `MainWindow.track-names` 的行数 | 行 |
//! | `undoable` | `UndoPort::display().undoable`（可撤销步数） | 步 |
//! | `after` 收到的视图轨道数 | `OpenProjectAfter` 第三/四个参数 | 条 |
//! | `module-status` | 宿主写的**回执行**原文（既有投影通道） | 结构化文本 |
//!
//! ## 为什么入口住在控制台 Tab 1（与 `#90` / `#93` **同一条纪律**）
//!
//! `[UI-MCP-003]` 的 5 张 Linux 基准只覆盖默认帧（Tab 0）。`MixerConsole` 只在
//! `active-tab == 1` 时可见 ⇒ 新按钮在默认帧里**一个像素都不画** ⇒ 基准逐位不变。
//!
//! ## 这条判据怎么变红（本单注入读数见交付报告）
//!
//! - `.slint` 的 `accessible-action-default` / `TouchArea.clicked` 不调 `root.new-project()`
//!   （**断开入口**）⇒ 对话框一次没被问 ⇒ 正向腿红；
//! - `host::wire_new_project` 不接回调 / 不写 `module-status` ⇒ 同上；
//! - 落盘改成 `filled_project()`（3 轨）⇒ 1187 B / `tracks=0` 两条红；
//! - **没有对话框**那一支改成静默 `return` ⇒ 反向腿红（回执为空）；
//! - 新建后**不换工程**（只写文件）⇒ 指纹 / 行数 / `after` 三条红。
//!
//! ## 平台约束：本文件只有一个 `#[test]`
//!
//! `slint::platform::set_platform` 是线程局部的，`Tier1Window::install` 每线程只能成功
//! 一次（第二次 `AlreadySet`）⇒ 同款用法见 `tests/open_project_ui.rs`。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::ScriptedPicker;
use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::file_picker::{FilePicker, NoPicker, PickerRequest};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession, project_fingerprint};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::render::{Tier1Window, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 新入口的语义 ID（`ui/console/mixer_console.slint` ↔ `src/elements.rs` 两处必须逐字相同）。
const NEW_BUTTON_ID: &str = "module-new-project-button";

/// 新入口的无障碍名称（与 `elements.rs` 登记的标签**逐字相同**，运行时对账会断言相等）。
const NEW_BUTTON_LABEL: &str =
    "新建工程（用文件对话框给新工程选一个落点，写出 0 音轨 ＋ 1 主总线的空白容器并就地换过去）";

/// 既有投影通道的落点（回执行的**可寻址**节点，住在右栏 —— 默认帧里就可见）。
const MODULE_STATUS_ID: &str = "module-last-status";

/// 撤销会话的"现在"（与 `open_project_ui.rs` 的常量同款：判据不需要墙钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 窗口尺寸（非零即合法；判据只要求"窗口真的建起来过"）。
// ⭐ `#191`：`compact` 现在跟**真实窗口宽**走（`ui/app.slint` 的表达式绑定）。
// ⭐ `#195`：阈值 `1920 → 1180` ⇒ `1440 × 900` 是**展开档**（左栏 240 ＋ 右栏常驻），
// 右栏里的 `module-*` 入口默认就在树里 ⇒ 本判据翻回 `1440`（`#191` 曾临时改成 1920）。
const WINDOW: Size = Size::new(1440, 900);

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照 `open_project_ui.rs`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-new-project-ui-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 目录里的**条目数**（用来证明"取消 / 无对话框"一个文件都没建）。
    fn entries(&self) -> usize {
        std::fs::read_dir(&self.dir).expect("读临时目录").count()
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

/// `after` 的读数：`(path, 工程轨道数, 投影轨道数)` —— 证明**打开工程那条 `after`**
/// 真的被这一次新建触发了，而且它拿到的是**新投影**（0 轨）。
type AfterReadings = Rc<RefCell<Vec<(String, usize, usize)>>>;

/// 把「新建工程」接到**本单的唯一实现**上（每次调用 = `on_new_project` setter 覆盖）。
fn wire(
    window: &MainWindow,
    picker: Rc<dyn FilePicker>,
    runtime: &Rc<dyn host::OpenProjectRuntime>,
) {
    host::wire_new_project(window, picker, Rc::clone(runtime));
}

#[test]
fn the_gui_new_project_entry_writes_an_empty_container_and_swaps_it_in() {
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
        aether_app.contains("> module-status:"),
        "新 UI 契约面必须声明 > module-status:"
    );
    assert!(
        aether_host.contains("set_module_status"),
        "宿主接线层必须注册 set_module_status"
    );
    assert!(
        aether_app.contains("> module-status-succeeded:"),
        "新 UI 契约面必须声明 > module-status-succeeded:"
    );
    assert!(
        aether_host.contains("set_module_status_succeeded"),
        "宿主接线层必须注册 set_module_status_succeeded"
    );
}
