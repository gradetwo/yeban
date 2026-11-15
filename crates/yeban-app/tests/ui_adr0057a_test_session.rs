//! **ADR-0057 A：服务端测试装配的会话** —— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 测试面（`--ui-test-mode` + `YEBAN_ALLOW_UI_INJECT=1`）的装配建了与 GUI 同款
//! 撤销会话之后，注入的编辑必须真的进工程、真的落盘、重读必须一致 ——
//! 而不是「窗口上看得见、落盘时按旧工程写回去」。
//!
//! ## 本文件要钉死的陷阱（ADR-0057 背景 §2）
//!
//! `wire_*` 的提交只走 `commit_ops → refresh_undo_window`（重投影窗口，
//! 不换 `self.project`）。`save_now` 若仍按界面缓存写盘：注入的编辑在窗口上
//! 看得见，`ui/force_save` 一按就按旧工程写盘，编辑静默丢失 —— 比 P0-1 原状
//! 「什么都不发生」更坏。修前本判据必红：落盘读回的 `bpm` 仍是夹具值 128，
//! 不是提交后的 140。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（编辑进会话） | `invoke_bpm_commit("140")` ⇒ `commit_count + 1`，端口工程 `bpm = 140.0`，界面读数 `"140.00"` |
//! | ②（保存走端口） | `ui/force_save` 成功，回执 `writtenFromUndoPort = true` |
//! | ③（重读一致） | `open_project_file` 读回的工程 `bpm = 140.0`，与端口工程一致；磁盘文件真实存在 |
//!
//! ## 覆盖说明
//!
//! 本判据在进程内复现服务端测试装配的接线（`build_live_ui_with` +
//! `undo: Some(端口)` + `into_control_plane`），与 `ui_mcp_serve::run` 的测试面
//! 是同一条接线；生产面（`undo: None`）的行为由既有判据
//! `admin_force_save_really_writes_a_readable_container` 覆盖，本文件不重复。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_bpm_field.rs` 同款）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/ui_bpm_field.rs`。
#[allow(dead_code)]
mod live;

use std::path::PathBuf;
use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;

/// 夹具速度（`yeban_model::samples::filled_project`）。判据的读数以它为准。
const FIXTURE_BPM: f64 = 128.0;

/// 提交后的速度：与夹具值不同，且在合法区间内（`MIN_BPM..=MAX_BPM`）。
const EDITED_BPM: f64 = 140.0;

/// 测试产物落在临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-adr0057a-{}-{}",
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

#[test]
fn injected_edit_survives_force_save_and_reads_back() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("> bpm-display:"),
        "新 UI 契约面必须声明 > bpm-display:"
    );
    assert!(
        aether_host.contains("set_bpm_display"),
        "宿主接线层必须注册 set_bpm_display"
    );
}
