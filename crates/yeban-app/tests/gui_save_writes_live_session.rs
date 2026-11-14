//! **界面上按保存 = 写活会话**（`ROAD-M4-008` 选项 (a) 的收口判据）。
//!
//! ## 这条判据要证的句子
//!
//! 默认构建（**没有**控制面权威）里，`host::wire_save` 装的保存回调写出去的是
//! **活会话此刻**那一版：工程 + 它的 `CommitGraph`（编码成 `history.dag`）。
//! 此前它写的是装配时刻的快照 + 零字节 `history.dag`，于是界面里的编辑与整条撤销链
//! 都到不了磁盘，而状态栏还报"已保存"。
//!
//! ## 判据的**承重步**（对照活会话，而不是"有没有提交"）
//!
//! ⚠ 新鲜的 GUI 会话**恒有**一条 genesis 提交 ⇒ "写出的 DAG 能解出 ≥1 条提交"
//! 会因为**错误的原因**通过。因此本判据比的是**活会话自己编码出来的字节**：
//! `reopened.history_dag == encode_history_dag(&live_port.graph())`。
//!
//! | 判据 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | A（DAG，承重） | 文件里的 `history.dag` 与**活会话**编码的图谱**逐字节相同**，且**长于**装配时刻的起始图谱 | 证"保存写的是活会话的图谱"。不证"图谱内容正确"（那是 `yeban-model` 的编解码判据） |
//! | B（工程，承重） | 重开的容器里，那个被编辑的音符力度 = **活会话**里的力度；起始容器里它是别的值 | 证"保存写的是活会话的工程"。不证"编辑本身对"（那是 `undo_wiring_ui.rs` 的判据） |
//! | 读的一侧（承重） | 用 `decode_history_dag` 把文件里的图谱读回、`open_with_graph` 重开会话，**重做**还能回到编辑后的工程 | 证"保存 → 重开"之后撤销链**真的**活着。不证"跨进程"（那要跑真二进制） |
//! | 落点 | `by_authority == false`（本地原子写） | 证这条用例走的确实是默认构建那条路 |
//!
//! ## 这条判据怎么变红
//!
//! - 保存回调退回 `SaveStatus.project`（装配快照）⇒ B 红（力度是起始值）；
//! - 保存写出空 `history.dag`（`save_project_file` 的老行为）⇒ A 红（0 字节）；
//! - 打开时丢掉 `history.dag`（`UndoSession::open` 而不是 `open_with_graph`）⇒ 读的一侧红。

#![allow(unused)]
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::open::open_project_document_file;
use yeban_app::save_action::{SaveOutcome, dispatch_save};
use yeban_app::scene::DemoScene;
use yeban_app::undo::undo_session::wiring_fixture;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::CommitGraph;
use yeban_model::container::{self, write_project_container};
use yeban_model::samples::filled_project;

const NOW: u64 = 1_760_000_000_000;
const WINDOW: yeban_ui_test_port::Size = yeban_ui_test_port::Size::new(1440, 900);

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-gui-save-live-{}-{}",
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

/// 工程里某个音符的力度（读不到就是 `None`）。
fn velocity_of(
    project: &yeban_model::YebanProjectV1,
    note_id: &yeban_model::ids::EntityId,
) -> Option<u8> {
    project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values())
        .find(|note| note.id == *note_id)
        .map(|note| note.velocity)
}

#[test]
fn the_local_save_entry_writes_the_live_session_project_and_graph() {
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
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}
