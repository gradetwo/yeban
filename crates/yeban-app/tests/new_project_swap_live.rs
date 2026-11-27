//! 探针（T-FIX-927）：活窗口里从已打开工程执行「新建空白工程 → 保存」。
//!
//! 复现 dou-win-134：已打开含 MIDI/Clip/EQ 的工程时，文件菜单「新建空白工程」
//! 选保存路径后文件已生成（1187 B），但窗口消失、进程退出。
//!
//! 本文件用**真实**的换工程实现（`PortOpenProject` + 控制驱动的
//! `ProductionLoop`），不用 `RecordingRuntime` 假实现 —— 既有的
//! `new_project_ui.rs` / `new_project_overwrite_ui.rs` 只断了写文件与
//! 回执，换工程那一段在活窗口里从未被真的跑过。
//!
//! 复现保真度：起点工程 = `filled_project()` ＋ 主音轨上一台真 EQ
//! （`InternalEffect`，含 `eq_low_gain`）—— 与报告里的「MIDI 1/2、Clip 1、
//! EQ」同形；`after` = 生产 `main.rs::make_after(false)` 的等价复刻
//! （`wire_save_with_engine` + `wire_keys` + aether 重绑 + 编辑面重绑 +
//! `mark_applied`），⛔ 不是空 `after`。
//!
//! ⚠ 本文件只有一个 `#[test]`：`slint::platform::set_platform` 是线程局部的，
//! 与 `tests/open_project_ui.rs` 同款纪律。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::ScriptedPicker;
use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::{LivePort, Permission, Size};

/// 撤销会话的"现在"（与 `open_project_ui.rs` 的常量同款）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 窗口尺寸（与 `open_project_ui.rs` 同款）。
const WINDOW: Size = Size::new(1440, 900);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-new-project-swap-live-{}-{}",
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

/// 界面此刻的轨道行数（`[UI-TEST-001]` 的语义属性；⛔ 不数像素）。
fn rows(window: &MainWindow) -> usize {
    use slint::Model as _;
    window.get_track_names().iter().count()
}

#[test]
fn new_blank_project_save_keeps_the_app_alive_and_swaps_to_the_empty_project() {
    let scratch = Scratch::new();
    let target = scratch.join("scratch-empty.yeban");

    // 起点：与复现一致的已打开工程（多轨 + MIDI 音符 + 片段 + 一台真 EQ）。
    let mut project = yeban_model::samples::filled_project();
    // 复现里的 EQ：挂在主音轨（身份序第一条非主总线轨）上，含契约参数 eq_low_gain。
    {
        use yeban_model::{DeviceDefinition, DeviceKind, ParameterValue};
        let master = project.master_bus_track_id;
        let lead = project
            .tracks
            .iter_mut()
            .find(|(id, _)| **id != master)
            .expect("起点工程必须有一条非主总线轨可挂 EQ");
        lead.1.devices.push(DeviceDefinition {
            id: yeban_model::ids::EntityId::new(),
            name: "EQ".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "eq_low_gain".to_owned(),
                    value: 0.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "eq_high_gain".to_owned(),
                    value: 0.0,
                    unit: Some("dB".to_owned()),
                },
            ],
            latency_samples: 0,
            plugin: None,
        });
    }
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<new-project-swap-live>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let before_fingerprint = undo.fingerprint();
    let before_rows = view.tracks.len();
    assert!(before_rows >= 2, "起点工程必须有多轨，否则判据是空绿");

    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口必须装配成功");
    let window = slint::ComponentHandle::clone_strong(live.ui());
    assert_eq!(
        rows(&window),
        before_rows,
        "装配后界面轨道行数必须与起点工程一致"
    );

    // 真实引擎（控制驱动，不开声卡）+ 真实换工程实现。
    let production = Rc::new(RefCell::new(
        host::ProductionLoop::start(&project, 0).expect("引擎重建"),
    ));
    // 生产 `after`（`main.rs::make_after(false)` 的等价复刻）：换工程之后重接
    // 保存 / 按键 / aether 动作 / 编辑面、跟进模块路径、打已应用标记。
    // ⛔ 空 `after` 会漏掉这一整段 —— 退出若藏在这里，空 `after` 的探针是空绿。
    let engine_handle = production.borrow().engine_handle();
    let view_cell: Rc<RefCell<ViewState>> = Rc::new(RefCell::new(view.clone()));
    let module_paths: Rc<RefCell<host::ModulePaths>> =
        Rc::new(RefCell::new(host::ModulePaths::from_env_for_project(None)));
    let input = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
    let save_picker: Rc<dyn yeban_app::file_picker::FilePicker> =
        Rc::new(yeban_app::file_picker::NativeFilePicker);
    let undo_after = Rc::clone(&undo);
    let engine_after = Rc::clone(&engine_handle);
    let view_cell_after = Rc::clone(&view_cell);
    let module_paths_after = Rc::clone(&module_paths);
    let input_after = Rc::clone(&input);
    let save_picker_after = Rc::clone(&save_picker);
    let after: host::OpenProjectAfter = Rc::new(move |ui, path, new_view| {
        let Some(new_project) = undo_after.try_project() else {
            return Err("打开后端口里没有工程（当前工程一位没动）".to_owned());
        };
        let shared = host::shared_project_for_session(&undo_after, &new_project);
        *view_cell_after.borrow_mut() = new_view.clone();
        host::wire_save_with_engine(
            ui,
            host::SaveStatus {
                target: Some(path.to_path_buf()),
                project: Rc::clone(&shared),
            },
            Some(Rc::clone(&undo_after)),
            Rc::clone(&save_picker_after),
            None,
            Some(Rc::clone(&engine_after)),
        );
        host::wire_keys(
            ui,
            Rc::clone(&input_after),
            Some(Rc::clone(&undo_after)),
            Some(Rc::clone(&engine_after)),
            Some(Rc::clone(&shared)),
        );
        host::rewire_aether_actions_after_project_switch(
            ui,
            Rc::clone(&undo_after),
            Rc::clone(&engine_after),
            Rc::clone(&view_cell_after),
        );
        host::wire_edit_surfaces(ui, &undo_after);
        host::set_module_project_dir(&module_paths_after, path.parent());
        engine_after
            .borrow_mut()
            .mark_applied(yeban_app::engine_host::EditMark::from_display(
                &undo_after.display(),
            ));
        Ok(())
    });
    let runtime: Rc<dyn host::OpenProjectRuntime> = Rc::new(host::PortOpenProject {
        port: Rc::clone(&undo),
        production: Some(Rc::clone(&production)),
        quanta: 0,
        after,
    });
    let picker = ScriptedPicker::answering_save(&target);
    // T-FIX-927：本探针不装脏保护（干净起点；脏腿由 `new_project_dirty_guard_live.rs` 另证）。
    host::wire_new_project(&window, picker, Rc::clone(&runtime), None);

    // ---- 执行：新建空白工程 → 保存 ----
    window.invoke_new_project();

    // ---- 断言 1：文件已生成，且是 1187 B 空工程容器 ----
    let written = std::fs::read(&target).expect("新建工程文件必须已生成");
    assert_eq!(written.len(), 1187, "新建工程必须是 1187 B 空工程容器");

    // ---- 断言 2：窗口还活着（属性可读、弱引用可升级） ----
    assert!(
        window.as_weak().upgrade().is_some(),
        "保存后窗口必须还活着（弱引用可升级）"
    );
    assert_eq!(rows(&window), 0, "新建后界面必须是 0 轨（空工程）");

    // ---- 断言 3：端口里已是新工程，旧工程内容无残留 ----
    let current = undo.try_project().expect("换工程后端口里必须有活工程");
    assert_ne!(
        undo.fingerprint(),
        before_fingerprint,
        "换工程后工程指纹必须变了（旧工程不得残留）"
    );
    assert_eq!(
        current.tracks.len(),
        1,
        "新工程只剩主总线一条轨（0 音轨 ＋ 1 主总线），实测 {} 条",
        current.tracks.len()
    );
    assert!(current.clip_pool.is_empty(), "新工程不得残留旧工程的片段池");

    // ---- 断言 4：回执是成功，不是 error=create / error=swap ----
    let receipt = window.get_module_status();
    assert!(
        receipt.contains("outcome=ok"),
        "新建回执必须是 outcome=ok，实测 {receipt:?}"
    );
    assert!(
        !receipt.contains("error="),
        "新建回执不得带 error=，实测 {receipt:?}"
    );
}
