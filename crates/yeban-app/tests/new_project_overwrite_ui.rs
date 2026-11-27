//! ⭐ T-FIX-308：**新建工程遇到已存在目标路径**时的 GUI 出口。
//!
//! 修前：文件选择器返回的 `choose_save` 路径只要有 `.yeban` 后缀就直接走
//! `create_new_project_file` → `save_archive_file`（覆盖式 rename）⇒ 一个脚本假
//! picker / 控制面装配直接返回已有路径，就能用空白工程静默覆盖原工程。
//!
//! 修后（两道闸，缺一不可）：
//! 1. [`host::create_new_project_file`] 用 `save_new_archive_file`：目标已存在 ⇒
//!    `NewProjectError::Exists`，一字节都不写（存在性判定在 `.yeban.lock` 内）；
//! 2. [`host::wire_new_project`] 拿路径后调 [`FilePicker::confirm_overwrite`]：
//!    **取消 ⇒ 直接返回**（不写文件、不写回执、不换工程）；确认 ⇒ 才走名字就写着
//!    overwrite 的显式入口，并在回执里写 `overwrite=true`。
//!
//! ## 判据
//!
//! | 腿 | 注入 | 断言 |
//! | :--- | :--- | :--- |
//! | 取消 | 脚本 picker 返回已有路径，覆盖确认答 `false` | 原文件字节逐位不变、runtime 未被调用（当前工程不变）、confirm 被问 1 次 |
//! | 确认 | 同一窗口把确认答案改成 `true` | 文件被换成 1187 B 空工程、runtime 收到同一个路径、回执含 `overwrite=true`、confirm 累计 2 次 |
//!
//! ⚠ 本文件只有一个 `#[test]`：`slint::platform::set_platform` 是线程局部的，
//! 与 `tests/new_project_ui.rs` / `tests/open_project_ui.rs` 同款纪律。真弹窗
//! 的外观 / 取消按钮的系统语义**不在**本判据范围内（见 `file_picker.rs` 边界 3）。

#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::ScriptedPicker;
use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::host;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::port::Permission;

/// 只记录"被要求打开哪个路径"的假 runtime（⛔ 不真的换工程 / 不碰端口）。
struct RecordingRuntime {
    opened: RefCell<Vec<PathBuf>>,
}

impl host::OpenProjectRuntime for RecordingRuntime {
    fn open_runtime(&self, _ui: &MainWindow, path: &std::path::Path) -> Result<(), String> {
        self.opened.borrow_mut().push(path.to_path_buf());
        Ok(())
    }

    fn open_archive(
        &self,
        _ui: &MainWindow,
        _label: &str,
        _archive: yeban_model::container::ProjectArchive,
        _project_dir: Option<&std::path::Path>,
        _source_path: Option<&std::path::Path>,
    ) -> Result<(), String> {
        // T-FIX-428：`wire_new_project` 永远不调这一跳（新建 = 写空容器 + 走
        // `open_runtime` 的换工程路径）；真走到就是装配错误 ⇒ 响亮报错，
        // ⛔ 不静默假装换了工程。
        Err("RecordingRuntime 不支持 open_archive（wire_new_project 不应调它）".to_owned())
    }
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-new-project-overwrite-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
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

#[test]
fn an_existing_new_project_target_is_only_overwritten_after_confirmation() {
    let scratch = Scratch::new();
    let target = scratch.dir.join("already-here.yeban");
    let sentinel = b"ORIGINAL PROJECT - DO NOT TOUCH";
    std::fs::write(&target, sentinel).expect("夹具：目标已存在");

    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 端口");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let _ui = ui;

    let picker = ScriptedPicker::answering_save_and_refusing_overwrite(&target);
    let runtime = Rc::new(RecordingRuntime {
        opened: RefCell::new(Vec::new()),
    });
    // T-FIX-927：本判据不装脏保护（`None` ⇒ 脏判定恒 false，行为与改动前逐位一致）。
    host::wire_new_project(&window, picker.clone(), runtime.clone(), None);

    // ---- ① 取消确认：文件与当前工程都不变 ----
    window.invoke_new_project();
    assert_eq!(
        picker.overwrite_call_count(),
        1,
        "目标已存在 ⇒ 必须问一次显式覆盖确认（⛔ 不能只靠 picker 自己的替换提示）"
    );
    assert_eq!(
        std::fs::read(&target).expect("原文件必须还在"),
        sentinel,
        "取消 ⇒ 原文件字节逐位不变"
    );
    assert!(
        runtime.opened.borrow().is_empty(),
        "取消 ⇒ 不换工程（runtime 一次都没被调用）"
    );

    // ---- ② 确认后：真的覆盖，回执自报 overwrite=true ----
    picker.set_overwrite_answer(true);
    window.invoke_new_project();
    assert_eq!(picker.overwrite_call_count(), 2, "第二次仍要问确认");
    assert_eq!(
        runtime.opened.borrow().as_slice(),
        std::slice::from_ref(&target),
        "确认 ⇒ runtime 打开的就是刚覆盖的那个路径"
    );
    let written = std::fs::read(&target).expect("覆盖后的文件必须存在");
    assert_eq!(
        written.len(),
        1187,
        "确认 ⇒ 写出的仍是同一条 1187 B 空工程容器"
    );
    let receipt = window.get_module_status();
    assert!(
        receipt.contains("overwrite=true"),
        "覆盖成功 ⇒ 回执必须自报 overwrite=true，实测 {receipt:?}"
    );
    assert!(
        !receipt.contains("error=create"),
        "确认路径不该报创建失败，实测 {receipt:?}"
    );
}
