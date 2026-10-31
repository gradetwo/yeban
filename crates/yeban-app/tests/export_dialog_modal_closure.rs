//! **`Esc` 收尾导出设置弹窗**（`host::apply_action` 的 `Action::Cancel` 臂）—— 端到端 + 源码判据。
//!
//! 规范来源：`[UI-A11Y-003]` §7.3（模态面板的收尾键）· `[UI-TEST-001]`（稳定语义 ID 的寻址面）。
//!
//! ## 本文件要证的句子
//!
//! `ui/dialogs/export_wav_dialog.slint` 盖住整帧（遮罩 `mask-black @ 0.65`），在本次接线之前
//! **只有一条**出路：鼠标点 `export-wav-close-button`。时光机那一半早就接了 `Esc`
//! （`tests/undo_tree_focus_trap.rs` ⑤），弹窗这一半漏了。现在 `Action::Cancel` 在
//! "弹窗开着"时落**既有的那条** `close-export-wav` 回调 ⇒ 键盘与鼠标走的是同一条路。
//!
//! ## 判据（一条链，六个读数）
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① 起点 | 弹窗关着、不在运行时控件树里、`module-status` 为空 |
//! | ② 打开 | `open-export-wav` ⇒ 属性为真，且九个 `export-wav-*` ID 全部进树 |
//! | ③ `Esc` 真的关闭 | 走**注入 → Slint 事件源 → `key-handler` → 宿主**全链路（⛔ 不是直接 invoke 回调）⇒ 属性转假、那九个 ID 全部离开树、寻址面回到 ① 的**同一集合** |
//! | ④ 正对照：`Esc` **不导出** | 目标 WAV 文件**不存在**、`module-status` 仍为空 —— 没有这一条，"关闭"可能顺带跑了导出而看不出来 |
//! | ⑤ 关闭后键盘层还在 | 再按 `1` 仍到达宿主（`active-tool` = 1）；无弹窗无手势时再按 `Esc` 落空**不改**任何读数 |
//! | ⑥ 源码纪律（唯一写者） | `src/host.rs` 里那个属性的 setter **恰好两处**（打开 / 关闭），键盘臂里 `invoke_close_export_wav` **恰好一处** ⇒ 不存在第二个真相源 |
//!
//! ## 如实登记的边界（⛔ 不假装覆盖）
//!
//! 本判据证的是**焦点仍在快捷键层**时的那条 `Esc`（打开弹窗这一步不移焦 ⇒ ② 之后就是这一态）。
//! 用户**点进**某个输入框之后再按 `Esc` 归谁，取决于 Slint 内建 `TextInput` 对 `Escape` 的
//! 内部处置（本机不编译 Slint ⇒ 无实测）；本文件因此**不**声称"输入框里按 `Esc` 也会关"，
//! ✕ 按钮那条出路一位未动。

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

/// 弹窗在树里的九个语义 ID（与 `tests/ui_export_dialog.rs` 同一份口径）。
const DIALOG_IDS: [&str; 9] = [
    "export-wav-dialog",
    "export-wav-panel",
    "export-wav-close-button",
    "export-wav-path-input",
    "export-wav-sample-rate-input",
    "export-wav-bit-depth-input",
    "export-wav-seconds-input",
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
    let scratch = Scratch::new();
    let target = scratch.dir.join("esc-must-not-write-this.wav");

    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<判据:导出弹窗 Esc 收尾>",
            "yeban-app",
            project.clone(),
            NOW,
        )
        .expect("打开撤销会话"),
    ));

    // ⚠ 导出落点指向**临时目录里一个从不预期的文件**：判据 ④ 要证它**不存在**。
    // SAFETY: 本测试二进制只有一个 `#[test]` ⇒ 没有第二个线程读这些变量；
    // 设置发生在本函数内部、早于任何装配。
    unsafe {
        std::env::set_var("YEBAN_UI_EXPORT_WAV_PATH", &target);
    }

    let mut ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 0,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    yeban_app::host::apply_undo(ui.ui(), &port);
    ui.pump_meters();

    // ---- ① 起点 ----
    let before = ids(&mut ui);
    let start_closed = !ui.ui().get_export_wav_open();
    assert!(
        start_closed,
        "① 起点：弹窗必须是关闭态（默认帧因此与基准一位不差）"
    );
    assert!(
        !has(&before, "export-wav-dialog"),
        "① 起点：对话框不许在树里"
    );
    assert_eq!(
        ui.ui().get_module_status().to_string(),
        "",
        "① 起点：状态行必须是空的（④ 要靠它证明 `Esc` 什么都没干）"
    );

    // ---- ② 打开 ----
    ui.ui().invoke_open_export_wav();
    assert!(
        ui.ui().get_export_wav_open(),
        "② `open-export-wav` 必须打开弹窗"
    );
    let opened = ids(&mut ui);
    for id in DIALOG_IDS {
        assert!(has(&opened, id), "② `{id}` 必须进运行时树");
    }
    let opened_delta = opened.len() - before.len();
    assert!(
        opened_delta > 0,
        "② 打开之后寻址面必须**变大**（否则 ③ 的'回到同一集合'没有判别力）: 前 {} 后 {}",
        before.len(),
        opened.len()
    );

    // ---- ③ 真的按键：注入 → `key-handler` → 宿主 `Action::Cancel` ----
    ui.dispatch_key_press(KeyCode::Escape)
        .expect("③ `Esc` 必须落到真实窗口（Interactive 档）");
    let esc_closed = !ui.ui().get_export_wav_open();
    assert!(
        esc_closed,
        "③ 弹窗开着时按 `Esc` 必须关掉它（键盘与鼠标 `export-wav-close-button` 同一条路）"
    );
    let closed = ids(&mut ui);
    for id in DIALOG_IDS {
        assert!(!has(&closed, id), "③ 关闭之后 `{id}` 必须离开树");
    }
    assert_eq!(
        closed, before,
        "③ 关闭之后的寻址面必须回到 ① 的**同一集合**（不许留下半个弹窗节点，也不许顺手改出新节点）"
    );

    // ---- ④ 正对照：`Esc` 不导出 ----
    let status_empty = ui.ui().get_module_status().is_empty();
    assert!(
        status_empty,
        "④ `Esc` 之后状态行必须**还是**空的（跑了任何动作都会写它）"
    );
    let target_written = target.exists();
    assert!(!target_written, "④ 收尾键不许落盘: {}", target.display());

    // ---- ⑤ 关闭后键盘层照旧 + 落空的 `Esc` 不改任何读数 ----
    ui.dispatch_key_press(KeyCode::Character('1'))
        .expect("⑤ `1` 必须仍经同一个键盘入口到达宿主");
    assert_eq!(
        ui.ui().get_active_tool(),
        1,
        "⑤ 关闭弹窗不许把键盘层一起关掉"
    );
    let tool_before = ui.ui().get_active_tool();
    let view_before = ui.ui().get_arrangement_view();
    ui.dispatch_key_press(KeyCode::Escape)
        .expect("⑤ 落空的 `Esc` 也必须被窗口接受（放行给焦点系统）");
    assert!(
        !ui.ui().get_export_wav_open(),
        "⑤ 弹窗已关 ⇒ 再按 `Esc` 不许把它打开"
    );
    assert_eq!(
        ui.ui().get_active_tool(),
        tool_before,
        "⑤ 落空的 `Esc` 不许顺手改工具"
    );
    assert_eq!(
        ui.ui().get_arrangement_view(),
        view_before,
        "⑤ 落空的 `Esc` 不许顺手切视图"
    );

    // ---- ⑥ 源码纪律：那个属性只有一个真相源 ----
    let host_src = include_str!("../src/host.rs");
    let writers = host_src.matches("set_export_wav_open(").count();
    assert_eq!(
        writers, 2,
        "⑥ `export-wav-open` 的写者必须**恰好**两处（`on_open_export_wav` / `on_close_export_wav`），实测 {writers} 处 —— 多一处就是第二个真相源"
    );
    let invokers = host_src.matches("invoke_close_export_wav(").count();
    assert_eq!(
        invokers, 1,
        "⑥ 键盘臂必须**只**触发那条既有回调一次（本次接线的形状），实测 {invokers} 处"
    );

    report_line(&format!(
        "[export-esc] 起点关={start_closed} ⇒ 打开(+{opened_delta}) ⇒ Esc 关={esc_closed} ⇒ \
         状态行空={status_empty} 落盘={target_written} ⇒ `1` 后 tool={tool_before}"
    ));

    // SAFETY: 同上 —— 单 `#[test]` 的进程。
    unsafe {
        std::env::remove_var("YEBAN_UI_EXPORT_WAV_PATH");
    }
}
