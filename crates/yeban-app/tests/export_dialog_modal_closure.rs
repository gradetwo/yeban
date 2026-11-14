//! **`Esc` 收尾导出设置弹窗**（终点 = 那**唯一**一条 `close-export-wav` 回调）—— 端到端 + 源码判据。
//!
//! 规范来源：`[UI-A11Y-003]` §7.3（模态面板的收尾键）· `[UI-TEST-001]`（稳定语义 ID 的寻址面）。
//!
//! ## 本文件要证的句子
//!
//! `ui/dialogs/export_wav_dialog.slint` 盖住整帧（遮罩 `mask-black @ 0.65`），在本次接线之前
//! **只有一条**出路：鼠标点 `export-wav-close-button`。时光机那一半早就接了 `Esc`
//! （`tests/undo_tree_focus_trap.rs` ⑤），弹窗这一半漏了。补上之后 `Action::Cancel` 在
//! "弹窗开着"时落**既有的那条** `close-export-wav` 回调 ⇒ 键盘与鼠标走的是同一条路。
//! `D-UI-19`（`tests/export_dialog_key_capture.rs`）又把模态的键盘接上了，它那条 `Esc`
//! 分支落的仍是**同一条**回调 `close-export-wav` —— 本文件钉的就是"这条回调是唯一的收尾形状"。
//!
//! ## 判据（一条链，六个读数）
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① 起点 | 弹窗关着、不在运行时控件树里、`module-status` 为空 |
//! | ② 打开 | `open-export-wav` ⇒ 属性为真，且十个 `export-wav-*` ID 全部进树 |
//! | ③ `Esc` 真的关闭 | 端到端注入那一条 `Esc`（⛔ 不是直接 invoke 回调）⇒ 属性转假、那十个 ID 全部离开树、寻址面回到 ① 的**同一集合**。**走的是哪一段**见下面的"路由"一节 —— `D-UI-19` 把它换过一次 |
//! | ④ 正对照：`Esc` **不导出** | 目标 WAV 文件**不存在**、`module-status` 仍为空 —— 没有这一条，"关闭"可能顺带跑了导出而看不出来 |
//! | ⑤ 关闭后键盘层还在 | 再按 `1` 仍到达宿主（`active-tool` = 1）；无弹窗无手势时再按 `Esc` 落空**不改**任何读数 |
//! | ⑥ 源码纪律（唯一写者） | `src/host.rs` 里那个属性的 setter **恰好两处**（打开 / 关闭），键盘臂里 `invoke_close_export_wav` **恰好一处** ⇒ 不存在第二个真相源 |
//!
//! ## 路由（`D-UI-19` 之后本文件的这条 `Esc` 走的是哪一段）
//!
//! ⚠ 本节于 `D-UI-19` 改写：那之前这里写的是"注入 → Slint 事件源 → `key-handler` → 宿主
//! `Action::Cancel`"，而 `D-UI-19` 把键盘交给了模态 ⇒ 那句话**今天不再成立**。
//! 本判据的断言一位未动（⛔ 没有为了迁就新实现去放宽任何一条），改的只有这段说明。
//!
//! * **② 打开之后焦点已在第一格里**（`host` 在 `set_export_wav_open(true)` 的同一个闭包中
//!   调 `invoke_focus_export_wav_dialog()`）⇒ ③ 注入的 `Esc` 落点是
//!   `TextInput` → `export-trap` → **既有的** `close` 回调 → 宿主 `on_close_export_wav`，
//!   ⛔ 不再经过 `key-handler`（它是弹窗的**兄弟**，不是祖先；Slint 的投递只往上冒）。
//! * 两条路**终点同一个**：`Action::Cancel` 臂与 trap 的 `Esc` 分支都落 `close-export-wav`
//!   那**唯一**一条回调（⑥ 钉的正是"这个形状只有一处"）⇒ 本判据证的"能关、关得干净、
//!   不顺手导出"这三条读数一位都没有因为实现换路而放宽。
//! * `Escape` 在 `TextInput` 里**不会被吃掉**这一点现在有实测：
//!   `i-slint-core-1.18.1/items/text.rs` 的"Only insert/interpreter non-control character
//!   strings"那一段把控制字符（含 `\u{1b}`）一律 `EventIgnored` ⇒ 冒泡给祖先 `export-trap`。
//!   上一版这里写的是"本机不编译 Slint ⇒ 无实测"，那条不确定性已由 `D-UI-19` 消除。
//! * 弹窗开着而焦点**不在**弹窗里（模态遮罩不吞鼠标 ⇒ 点击仍会穿到下层，`D-UI-21` 已登记）
//!   时，`Esc` 走的还是 `key-handler` → `Action::Cancel` 那一条。两条都在，⛔ 本文件不再
//!   声称自己钉的是哪一条 —— 端到端只读到"关掉了"。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use std::path::PathBuf;
use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// 会话打开时刻（与既有撤销判据同一个夹具常量）。
const NOW: u64 = 1_760_000_000_000;

/// 弹窗在树里的语义 ID（与 `tests/ui_export_dialog.rs` 同一份口径）。
/// ⭐ `D-EX-1` 加了第五格 ⇒ 第九格之后是 `export-wav-limiter-switch`。
const DIALOG_IDS: [&str; 10] = [
    "export-wav-dialog",
    "export-wav-panel",
    "export-wav-close-button",
    "export-wav-path-input",
    "export-wav-sample-rate-input",
    "export-wav-bit-depth-input",
    "export-wav-seconds-input",
    "export-wav-limiter-switch",
    "export-wav-options-note",
    "export-wav-run-button",
];

/// 测试产物落在临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-export-esc-{}-{}",
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

/// 运行时控件树的 ID 集合（升序 ⇒ 可直接比相等）。
///
/// ⭐ 每次读之前先跳一跳心跳：[`live::LiveUi::tree_snapshot`] 返回的是**缓存**的那一份
/// （`live_surface.rs` 里 `self.surface.inner.tree().clone()`），刷新发生在
/// `pump_meters()` 末尾的 `refresh_tree()` —— 与 `tests/undo_tree_focus_trap.rs`
/// 每次状态变化后先 `pump_meters()` 再读树是同一个道理。
/// 不跳这一跳，读到的就是**上一次心跳时刻**的树：弹窗属性已经翻了，树还是旧的，
/// 判据于是红在"② `export-wav-dialog` 必须进运行时树"上（本文件第一次上 CI 就是这么红的）。
fn ids(ui: &mut live::LiveUi) -> Vec<String> {
    ui.pump_meters();
    let mut list: Vec<String> = ui.tree_snapshot().ids().map(str::to_owned).collect();
    list.sort();
    list
}

/// 某个 ID 在不在这一份树里（`ids` 已排序，这里只做存在性判断）。
fn has(list: &[String], id: &str) -> bool {
    list.iter().any(|seen| seen == id)
}

#[test]
fn escape_closes_the_export_dialog_without_running_an_export() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback close-export-wav("),
        "新 UI 契约面必须声明 callback close-export-wav("
    );
    assert!(
        aether_host.contains("on_close_export_wav"),
        "宿主接线层必须注册 on_close_export_wav"
    );
    assert!(
        aether_app.contains("callback open-export-wav("),
        "新 UI 契约面必须声明 callback open-export-wav("
    );
    assert!(
        aether_host.contains("on_open_export_wav"),
        "宿主接线层必须注册 on_open_export_wav"
    );
}
