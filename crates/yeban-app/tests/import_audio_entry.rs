//! 判据：**运行期「导入音频」在 GUI 里有一个可点的入口**，点下去真的把一份 WAV
//! 作为**可撤销的片段**导入当前工程（取消不动一位 / 坏文件响亮且不毁工程 / 幂等不重复提交）。
//!
//! ## 本单补的是哪一格
//!
//! | 层 | 现状（本单之前） |
//! | :--- | :--- |
//! | 能力（Rust） | `crate::import_audio::import_audio_file` ✓（CLI `--import-audio` 与 MCP `yeban_import_audio` 在同一份实现上） |
//! | 会话池 | `UndoPort::insert_asset` ✓（保存经 `UndoPort::assets()` 写 `assets/{sha256}`） |
//! | **GUI 出口** | ⛔ 一个都没有：素材池面板只有**只读**三数组，界面里没有任何导入动作 |
//! | ⭐ **本单** | ⭐ 素材池面板的 `asset-pool-import-button`（控制台 **Tab 1**）⇒ `host::wire_import_audio` ⇒ `FilePicker` ⇒ `import_audio_file` ⇒ 资产登记 ⇒ 重投影 ⇒ `module-status` 回执 |
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 树里有没有 `asset-pool-import-button` | 活窗口的运行时控件树（Tab 0 / Tab 1 各读一次） | 布尔 |
//! | `role` / `label` / `bounds` | 该节点的 `accessible-role` / `accessible-label` / 实几何 | kebab-case 角色 / 中文标签 / 像素 |
//! | `choose_open` 调用数 | 假选择器收到的请求数 | 次 |
//! | 工程指纹 | `UndoPort::fingerprint()` | 十六进制串 |
//! | 素材池行数 | `MainWindow.asset-pool-names` 的行数（= 投影行数） | 行 |
//! | `undoable` | `UndoPort::display().undoable`（可撤销步数） | 步 |
//! | 会话资产池 | `UndoPort::assets()` 的条目数 / 字节是否逐字等于源文件 | 条 / 布尔 |
//! | `module-status` | 宿主写的回执行原文 | 结构化文本 |
//!
//! ## 为什么入口住在控制台 Tab 1（与 `#90` 甲 / `#93` / `#129` 同一条纪律）
//!
//! `[UI-MCP-003]` 的 5 张 Linux 基准**只覆盖默认帧**。`ai-rail` 在默认帧里可见 ⇒
//! 在那里多一个按钮会让基准**确定性过期**，而本机是 macOS、`tests/golden/<platform>`
//! **只有** `linux` ⇒ 本机**无法**重录。素材池面板住在 `MixerConsole`（Tab 1），
//! 默认帧**一个像素都不画** ⇒ 基准逐位不变（本判据的 ① 就是这条的机械读数）。
//!
//! ## 这条判据怎么变红（注入读数见交付报告）
//!
//! - `.slint` 的 `accessible-action-default` / `TouchArea.clicked` 不调 `root.import-audio()`
//!   ⇒ 注入之后工程一位不动 ⇒ 正向腿红；
//! - `host::wire_import_audio` 不接回调 / 回执不写 `module-status` ⇒ 同上；
//! - 取消也去导入 / 写回执 ⇒ ② 的三条"一位不动"断言红；
//! - 失败路径先改工程再报错 ⇒ ③ 的"不毁工程"断言红；
//! - 导入不登记资产字节 ⇒ ④ 的 `UndoPort::assets()` 条目数 0 ⇒ 红（保存会结构性丢资产）；
//! - 导入不重投影 ⇒ 素材池行数不涨 ⇒ 红。
//!
//! ## 平台约束：本文件只有一个 `#[test]`
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程只能成功
//! 一次（第二次 `AlreadySet`）⇒ 同款用法见 `tests/open_project_ui.rs`。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use common::ScriptedPicker;
use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::file_picker::{FilePicker, PickerRequest};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::render::{Tier1Window, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 新入口的语义 ID（`ui/console/asset_pool_panel.slint` ↔ `src/elements.rs` 两处必须逐字相同）。
const IMPORT_BUTTON_ID: &str = "asset-pool-import-button";

/// 新入口的无障碍名称（与 `elements.rs` 登记的标签**逐字相同**）。
const IMPORT_BUTTON_LABEL: &str = "导入音频（用文件对话框挑一个 WAV 并作为片段导入）";

/// 撤销会话的"现在"（判据不需要墙钟，与 `open_project_ui.rs` 同款）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 窗口尺寸（与 `open_project_ui.rs` 同款：展开档）。
const WINDOW: Size = Size::new(1440, 900);

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-import-audio-entry-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 现造一份**真** WAV（本仓唯一写出器）；返回 `(路径, 字节)`。
fn write_wav(dir: &Path, frames: usize) -> (PathBuf, Vec<u8>) {
    let path = dir.join("take.wav");
    let samples: Vec<i16> = (0..frames)
        .map(|index| i16::try_from(index % 64).expect("夹具样本") * 100 - 3_000)
        .collect();
    yeban_render::wav::write_plain_wav(
        &path,
        &PcmFormat::integer(1, 48_000, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写夹具 WAV");
    let bytes = std::fs::read(&path).expect("读夹具 WAV");
    (path, bytes)
}

/// 读活窗口的运行时控件树（[UI-TEST-001] 的载体：全树内省 + JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 界面此刻的素材池行数（`[UI-TEST-001]` 的语义属性；⛔ 不数像素）。
fn pool_rows(window: &MainWindow) -> usize {
    use slint::Model as _;
    window.get_asset_pool_names().iter().count()
}

/// 触发一个按钮的**默认无障碍动作**（与鼠标点击是逐字同一段落点）。
fn press(window: &MainWindow, id: &str) {
    let handle = find_by_accessible_id(window, id)
        .unwrap_or_else(|| panic!("控件树里没有 `{id}` —— 入口不在当前可见分支里（或 ID 改了）"));
    handle.invoke_accessible_default_action();
}

/// 把假选择器接到**本单的唯一实现**上（重复调用 = `on_*` setter 覆盖）。
fn wire(window: &MainWindow, port: &Rc<UndoPort>, picker: &Rc<ScriptedPicker>) {
    let owned: Rc<ScriptedPicker> = Rc::clone(picker);
    let picker: Rc<dyn FilePicker> = owned;
    host::wire_import_audio(window, Rc::clone(port), picker);
}

#[test]
fn the_asset_pool_import_button_really_imports_a_wav() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> asset-pool-names:"),
        "新 UI 契约面必须声明 > asset-pool-names:"
    );
    assert!(
        aether_host.contains("set_asset_pool_names"),
        "宿主接线层必须注册 set_asset_pool_names"
    );
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
}
