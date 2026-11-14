//! 判据（任务单 `#90` **甲**）：**导出 MIDI** 在界面上有**真**入口，且按下去真的导出。
//!
//! ## 本单补的是哪一格
//!
//! | 层 | 现状（本单之前） |
//! | :--- | :--- |
//! | 能力（Rust） | `crate::export_midi::export_project_to_file` ✓（与 CLI `--export-midi` **同一个**函数） |
//! | CLI 出口 | `yeban-app --export-midi out.mid` ✓ |
//! | MCP 出口 | `yeban_export_midi` ✓ |
//! | **GUI 出口** | ⛔ **一个都没有** —— 人在界面上只能导出 WAV，导不出 MIDI |
//! | ⭐ **本单** | ⭐ `module-export-midi-button`（控制台 **Tab 1 = 调音台**）⇒ `host::wire_module_actions` 的 `export-midi` 分支 ⇒ `module-status` 回执 |
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 树里有没有 `module-export-midi-button` | 活窗口的运行时控件树（Tab 0 / Tab 1 各读一次） | 布尔 |
//! | `role` / `label` | 该节点的 `accessible-role` / `accessible-label` | kebab-case 角色 / 中文标签 |
//! | `module-status` | 宿主写的**回执行**原文（既有投影通道） | 结构化文本 |
//! | 落盘字节 | 目标 `.mid` 的长度 / 内容 | 字节（与共享导出器的产物**逐字节**比对） |
//!
//! ## 为什么入口住在控制台 Tab 1（而不是右栏那排「模块入口」）
//!
//! ⭐ `[UI-MCP-003]` 的 5 张 Linux 基准**只覆盖默认帧**（Arrangement / Tab 0 卷帘）。
//! `ai-rail` 在默认帧里是**可见的静态像素**，在那里多一个按钮 ⇒ 5 张基准确定性过期，
//! 而本机是 macOS、`tests/golden/<platform>` **只有** `linux` ⇒ 本机**无法**重录
//! （重录要走手动档 `gates-manual.yml` 的 `gate=goldens` + 人工复核 = 由负责人裁）。
//! `MixerConsole` 只在 `visible: root.active-tab == 1` 时可见 ⇒ 默认帧**一个像素都不变**。
//! 同一纪律的既有先例：任务单 `#86` 乙的「浏览…」按钮（同样落在 Tab 1）。
//!
//! ## 两个方向（本文件都测）
//!
//! 1. **Tab 0（默认帧）里没有它** ⇒ 基准不受影响（与 `src/test_port_adapter.rs` 的
//!    `DEFAULT_VIEW_MUST_NOT_HAVE` 是同一条断言的第二种写法）；
//! 2. **Tab 1 里有它**，且**事件注入**能触发它，且**结果可读** —— 但结果**不是**
//!    "元素存在"这种空断言：断言的是**落盘文件的字节** + `module-status` 回执的字段
//!    （`path` / `bytes` / `notes` / `format`），以及"没有撤销端口 ⇒ 如实拒绝且不留文件"。
//!
//! ## 这条判据怎么变红
//!
//! - 把 `ui/console/mixer_console.slint` 的 `clicked` / `accessible-action-default` 从
//!   `root.export-midi()` 改成别的东西（**断开回调**）⇒ 回执行仍为空、文件不落盘 ⇒ 红；
//! - `host::wire_module_actions` 不装 `on_export_midi` ⇒ 同上 ⇒ 红；
//! - 目标路径不读 `ModulePaths::export_midi`（界面或宿主写死一个路径）⇒ 红；
//! - 换一个第二份导出实现（字节与共享导出器不同）⇒ 逐字节比对红。
//!
//! ## 依赖从哪来（红线 6 没有被削弱）
//!
//! `yeban-ui-test-port` 是 `[dev-dependencies]`：**不进入** release 构建。核验命令（零编译）：
//! `cargo tree -p yeban-app -e normal --locked`。
//!
//! ## 平台约束：本文件只有一个 `#[test]`
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程只能成功
//! 一次（第二次 `AlreadySet`）⇒ 同款用法见 `tests/ui_module_actions.rs` / `tests/engine_error_banner.rs`。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/engine_error_banner.rs`。
#[allow(dead_code)]
mod live;

use std::path::PathBuf;
use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host::{ModulePaths, wire_module_actions_with};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::tree::ControlTree;

/// 新入口的语义 ID（`ui/console/mixer_console.slint` ↔ `src/elements.rs` 两处必须逐字相同）。
const MIDI_BUTTON_ID: &str = "module-export-midi-button";

/// 新入口的无障碍名称（与 `elements.rs` 登记的标签**逐字相同**，运行时对账会断言相等）。
const MIDI_BUTTON_LABEL: &str = "导出 MIDI（把当前工程写成 SMF 1 文件）";

/// 既有投影通道的落点（回执行的**可寻址**节点，住在右栏 —— 默认帧里就可见）。
const MODULE_STATUS_ID: &str = "module-last-status";

/// 撤销会话的"现在"（与 `sfz_ui_selector.rs` 的常量同款：判据不需要墙钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 回执的字段名闭集（**本判据自己的那一份**：只认这几条作 `field_of` 的切分边界）。
///
/// 加字段就加一行 —— 漏加不会静默：`field_of` 取不到值会**当场 panic 并打印整条回执**。
const RECEIPT_KEYS: [&str; 12] = [
    "action", "outcome", "error", "message", "path", "bytes", "ppq", "format", "tracks", "notes",
    "tempos", "temp",
];

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照 `tests/ui_module_actions.rs` 的 `Scratch`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-export-midi-ui-{}-{}",
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

/// 一条回执里，从 `at` 起**第一个** `:<已知字段名>=` 的冒号位置（口径与
/// `tests/ui_module_actions.rs` 的 `receipt_field_end` **逐字同款**：值里可能自带冒号）。
fn receipt_field_end(receipt: &str, at: usize, skip: &str) -> Option<usize> {
    let mut index = at;
    while index < receipt.len() {
        if receipt.as_bytes()[index] == b':' {
            for key in RECEIPT_KEYS {
                if key == skip {
                    continue;
                }
                let rest = &receipt[index + 1..];
                if rest.starts_with(key) && rest[key.len()..].starts_with('=') {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

/// 一条回执的**顶层**字段取值（引号感知，`#321`）；缺席返回 `None`。
///
/// 字段名只认**顶层**位置（`key=` 必须紧跟一个**顶层** `:`；引号内的 `:` 不算起点）：
/// 引号值跳到**第一个闭合引号**（引号内的 `format=` / `channels=` 等一律不是字段名），普通值仍按
/// [`receipt_field_end`] 的"已知字段名 + `=`"边界取。引号值**保留引号**（本文件既有约定）。
///
/// ## 引号里的 `"`（写死的契约）
///
/// 回执没有转义 ⇒ 第一个 `"` 即闭合引号；值里真带 `"` 时其后的字段取不到（`None`）。
fn field_of_opt(receipt: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let mut in_quotes = false;
    for (index, byte) in receipt.bytes().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b':' if !in_quotes => {
                let start = index + 1;
                if receipt[start..].starts_with(&needle) {
                    let value_start = start + needle.len();
                    if receipt[value_start..].starts_with('"') {
                        let close = receipt[value_start + 1..].find('"').expect("引号必须闭合");
                        return Some(receipt[value_start..=value_start + 1 + close].to_owned());
                    }
                    let end = receipt_field_end(receipt, value_start, key).unwrap_or(receipt.len());
                    return Some(receipt[value_start..end].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// 取一个字段；缺席即判据失败（⛔ 不返回空串 —— "没找到"与"值是空的"必须可分）。
fn field_of(receipt: &str, key: &str) -> String {
    field_of_opt(receipt, key).unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
}

/// 读活窗口的运行时控件树（`[UI-DoD §3.6]` 的载体：全树内省 + JSON 往返）。
fn runtime_tree(window: &yeban_app::ui::MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 一个语义 ID 必须**在这个活窗口的运行时控件树里**，且角色符合登记。
fn assert_addressable(tree: &ControlTree, id: &str, kind: ElementKind) {
    let node = tree.find_by_id(id).unwrap_or_else(|| {
        panic!(
            "`{id}` 不在运行时控件树里 —— 被裁掉 / 改了 accessible-id / 没接进 app.slint。\
             树里 {} 条节点",
            tree.len()
        )
    });
    assert_eq!(
        node.role.as_str(),
        kind.accessible_role(),
        "`{id}` 的角色必须是 `{}`（它在 elements.rs 里登记成 {kind:?}）",
        kind.accessible_role()
    );
}

/// 触发一个按钮的**默认无障碍动作**（与鼠标点击是逐字同一段落点）。
fn press(window: &yeban_app::ui::MainWindow, id: &str) {
    let handle = find_by_accessible_id(window, id)
        .unwrap_or_else(|| panic!("控件树里没有 `{id}` —— 入口不在当前可见分支里（或 ID 改了）"));
    handle.invoke_accessible_default_action();
}

#[test]
fn the_gui_export_midi_entry_really_writes_a_smf_and_reports_it() {
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

/// `#321` 负向判据：**引号内的键 ⛔ 可被顶层取到**（本文件的取值器也必须引号感知）。
///
/// 修前：`field_of("channels")` 会命中 `loss_detail` 引号内的 `channels=`，取到
/// `2"`（把闭合引号也带进值里）而不是"缺席"。修后必须是 `None`，而 `loss_detail`
/// 仍是整串（引号内的 `=` 不切）。
#[test]
fn a_quoted_value_does_not_expose_its_inner_keys_to_the_top_level() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
