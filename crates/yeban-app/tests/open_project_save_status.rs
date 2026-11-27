//! T-FIX-798：打开另一份工程后，状态栏的保存格必须指向**当前**工程。
//!
//! 缺陷：工程 A 保存后（状态栏留下 A 的路径），再打开工程 B / 内置 demo，
//! `status-bar-save-status` 仍显示 A 的旧路径 —— 用户读到的是假的"当前落点"。
//!
//! 判据（单测试，避免 Slint 平台每线程只能装一次的冲突）：
//! ① 先让状态栏留下 A 的路径；② 经生产形态的 `PortOpenProject::open_runtime`
//! 打开 B；③ 状态栏必须含 B 路径、不得含 A 路径；④ 无磁盘来源时明确显示"未保存"。
//!
//! 红对照：删掉 `open_archive_runtime` 里的 `apply_opened_project_save_status` 调用，
//! ③ 即红（仍留 A 路径）；删掉其 `None` 分支，④ 即红。

#![allow(unused)]
use std::path::{Path, PathBuf};
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host::{self, OpenProjectRuntime, PortOpenProject};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW: u64 = 1_760_000_000_000;
const WINDOW: Size = Size::new(1440, 900);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("yeban-t798-{tag}-{}", std::process::id()));
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

fn write_container(dir: &Path, name: &str, project: &yeban_model::YebanProjectV1) -> PathBuf {
    let path = dir.join(name);
    // filled_project 自带音频片段：纯工程入口按契约报 RequiresArchive，
    // 必须走归档入口并带上它引用的真实字节，夹具才自洽。
    yeban_app::save::save_archive_file(
        &yeban_model::container::ProjectArchive {
            project: project.clone(),
            history_dag: Vec::new(),
            proposals: None,
            assets: yeban_model::samples::filled_project_assets(),
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        },
        &path,
    )
    .expect("写出真容器");
    assert!(path.exists(), "夹具：必须真的落盘");
    path
}

#[test]
fn opening_another_project_refreshes_status_bar_save_path() {
    let scratch = Scratch::new("open-switch");
    let a = filled_project();
    let b = filled_project();
    let path_a = write_container(&scratch.dir, "project-a.yeban", &a);
    let path_b = write_container(&scratch.dir, "project-b.yeban", &b);

    let view = ViewState::from_project(&a).expect("工程 A 必须能投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // ① 前提：状态栏先留下工程 A 的保存路径（模拟"保存 A"之后的读数）。
    ui.set_save_status(format!("已保存 1234 字节 → {}", path_a.display()).into());
    ui.set_save_succeeded(true);
    assert!(
        ui.get_save_status().to_string().contains("project-a.yeban"),
        "夹具前提：状态栏必须先显示 A 的路径"
    );

    // ② 经生产形态的打开路径换入 B（open_runtime → open_archive_runtime）。
    let port = Rc::new(UndoPort::new(
        UndoSession::open("t798", "yeban-app", a, NOW).expect("撤销会话"),
    ));
    let runtime = PortOpenProject {
        port,
        production: None,
        quanta: 0,
        after: Rc::new(|_, _, _| Ok(())),
    };
    runtime.open_runtime(ui, &path_b).expect("打开 B 必须成功");

    // ③ 状态栏路径必须跟着当前工程走。
    let status = ui.get_save_status().to_string();
    assert!(
        status.contains("project-b.yeban"),
        "打开新工程后状态栏必须显示当前路径，实际 status = {status:?}"
    );
    assert!(
        !status.contains("project-a.yeban"),
        "不得残留上一份工程的路径，实际 status = {status:?}"
    );

    // ④ 无磁盘来源（内置 demo / 新建）必须明确写"未保存"，且不是成功态。
    host::apply_opened_project_save_status(ui, None);
    let unsaved = ui.get_save_status().to_string();
    assert!(
        unsaved.contains("未保存") && !unsaved.contains("project-a.yeban"),
        "无来源时必须明说未保存且不留旧路径，实际 status = {unsaved:?}"
    );
    assert!(
        !ui.get_save_succeeded(),
        "未保存必须是醒目状态（save-succeeded=false）"
    );
}
