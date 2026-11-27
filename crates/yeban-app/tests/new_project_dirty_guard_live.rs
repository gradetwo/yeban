//! 判据（T-FIX-927）：活窗口里「新建空白工程」的脏保护与保存失败两条腿。
//!
//! 与 `new_project_swap_live.rs` 分工：那一份证干净起点的成功换工程；
//! 本文件证**脏起点**的三选（取消 / 保存并继续 / 放弃并新建）与**保存失败**
//! 的可见错误 —— 三条腿跑在同一个活窗口、同一个线程里（`Tier1Window::install`
//! 每线程只能成功一次，故本文件只有一个 `#[test]`），按顺序：
//!
//! 1. **取消**：脏 → 新建 → 三选框开 → 取消 ⇒ 不建文件、不换工程、不退出；
//! 2. **保存并继续**：仍脏 → 新建 → 三选框开 → 保存并继续 ⇒ 旧工程先落盘，
//!    再建新文件、换成空工程；
//! 3. **保存失败**：新工程上以不可写路径新建 ⇒ `error=create` 回执 ＋ 浮动
//!    toast 可见，当前窗口/工程一位不动。
//!
//! 复现保真度与 `new_project_swap_live.rs` 同款：起点 = `filled_project()` ＋
//! 主音轨上一台真 EQ；`after` = 生产 `main.rs::make_after(false)` 的等价复刻。
//!
//! ## 语言前提（派单 seq 19067 的要求）
//!
//! 本文件断言的 UI 文案只有 toast 那一句（`新建空白工程失败：…`）：它是
//! `host.rs` 里写死的中文串，⛔ 不走 `.po` 词表、不随语言切换 —— 因此在
//! `en_US.UTF-8` 与 `zh_CN.UTF-8` 两处跑必须逐字相同。回执里记两次运行。
//! 三选框自己的文案（`@tr`）本文件只断布尔属性 `new-project-confirm-open`，
//! 不断文案原文（文案由 `i18n_coverage.rs` 的词表判据另行看守）。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::ScriptedPicker;
use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::dirty_guard::{self, SavedHead};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession, undo_session};
use yeban_model::Op;
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
            "yeban-new-project-guard-live-{}-{}",
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

/// 把夹具音符力度改一次并提交一条 op —— 造一次**真实**的未保存改动。
/// （与 `tests/dirty_undo_fingerprint.rs` 的 `commit_velocity` 同款。）
/// ⚠ 新力度必须从当前值现算（`current + 1`）—— 写死 100 时若夹具音符
/// 恰好已是 100，提交就是幂等空操作，指纹不变、脏判定恒 false（本判据
/// 第一版就栽在这里）。
fn commit_velocity(port: &UndoPort, seq: u64) {
    let fixture = undo_session::wiring_fixture(&port.project()).expect("夹具");
    let current = port
        .project()
        .clip_pool
        .get(&fixture.clip_id)
        .and_then(|entry| entry.content.notes())
        .and_then(|notes| notes.get(&fixture.note_id))
        .expect("音符")
        .velocity;
    // 力度域 1..=127：现值 +1，顶格回 1 —— 一定与现值不同。
    let new_vel = if current >= 127 { 1 } else { current + 1 };
    let op = Op::ModifyNoteVelocity {
        track_id: fixture.track_id,
        clip_id: fixture.clip_id,
        note_id: fixture.note_id,
        old_vel: current,
        new_vel,
    };
    port.commit_ops(NOW_MS + seq, "判据夹具", vec![op])
        .expect("提交");
}

/// 判定当前是否脏（先设当前指纹，再走真实判定入口；与
/// `tests/dirty_undo_fingerprint.rs` 的 `is_dirty_now` 同款）。
fn is_dirty_now(port: &UndoPort) -> bool {
    dirty_guard::set_current_fingerprint(port.fingerprint());
    dirty_guard::current_is_dirty(port.display().head)
}

#[test]
fn dirty_new_project_offers_save_discard_cancel_and_failed_save_stays_visible() {
    let scratch = Scratch::new();
    let current_path = scratch.join("current.yeban");
    let target = scratch.join("scratch-empty.yeban");
    let unwritable = scratch.join("no-such-dir").join("scratch-empty.yeban");

    // 起点：与复现一致的已打开工程（多轨 + MIDI 音符 + 片段 + 一台真 EQ），
    // 并先落盘一次 —— 保存并继续那条腿要写回的就是这一个文件。
    let mut project = yeban_model::samples::filled_project();
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
    // filled_project 带一条音频片段（Kick，资产字节不在本机）⇒
    // `save_project_file` 会以 RequiresArchive 拒绝。脏保护判据测的是 MIDI
    // 工程的保存，与音频资产无关 ⇒ 去掉音频片段及其摆放，保留 MIDI 音符 + EQ。
    // （ plain for 循环：`clip_pool.iter().filter().map()` 会触发 clippy
    //  的 map 迭代 lint —— 本仓 `-D warnings` 下是硬错误。）
    let mut audio_clip_ids: Vec<yeban_model::ids::EntityId> = Vec::new();
    project.clip_pool.retain(|id, entry| {
        if matches!(entry.content, yeban_model::ClipContent::Audio { .. }) {
            audio_clip_ids.push(*id);
            false
        } else {
            true
        }
    });
    for track in project.tracks.values_mut() {
        track
            .clips
            .retain(|_, placement| !audio_clip_ids.contains(&placement.clip_id));
    }
    yeban_app::save::save_project_file(&project, &current_path).expect("起点工程必须能落盘");
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<new-project-guard-live>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
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

    // 真实引擎（控制驱动，不开声卡）+ 真实换工程实现 + 生产等价 after。
    let production = Rc::new(RefCell::new(
        host::ProductionLoop::start(&project, 0).expect("引擎重建"),
    ));
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

    // 保存入口：当前工程的落点就是刚写出的那一个文件（保存并继续要写回它）。
    // （`after` 换工程后会重接一次保存入口 —— 与生产装配同款。）
    {
        let shared = host::shared_project_for_session(&undo, &project);
        host::wire_save_with_engine(
            &window,
            host::SaveStatus {
                target: Some(current_path.clone()),
                project: Rc::clone(&shared),
            },
            Some(Rc::clone(&undo)),
            Rc::new(yeban_app::file_picker::NativeFilePicker),
            None,
            Some(Rc::clone(&engine_handle)),
        );
    }

    // 脏保护的保存点：此刻（刚落盘）就是「已保存」态。
    let saved = Rc::new(SavedHead::new());
    saved.mark_saved(undo.display().head);
    saved.mark_saved_fingerprint(undo.fingerprint());
    dirty_guard::install(Rc::clone(&saved));
    dirty_guard::set_current_fingerprint(undo.fingerprint());
    assert!(!is_dirty_now(&undo), "刚落盘的起点工程必须干净");

    let picker = ScriptedPicker::answering_save(&target);
    host::wire_new_project(&window, picker, Rc::clone(&runtime), Some(&undo));

    // ---- 腿 1：取消 ----
    commit_velocity(&undo, 1);
    assert!(is_dirty_now(&undo), "提交改动后必须脏，否则三选框不会弹");
    let dirty_fingerprint = undo.fingerprint();
    window.invoke_new_project();
    assert!(
        window.get_new_project_confirm_open(),
        "脏时新建必须弹三选框（new-project-confirm-open=true）"
    );
    assert!(!target.exists(), "三选框开着时不得提前建文件");
    window.invoke_new_project_cancel();
    assert!(
        !window.get_new_project_confirm_open(),
        "取消后三选框必须关掉"
    );
    assert!(!target.exists(), "取消不得创建文件");
    assert!(window.as_weak().upgrade().is_some(), "取消后窗口必须还活着");
    assert_eq!(rows(&window), before_rows, "取消后界面轨道行数一位不动");
    assert_eq!(
        undo.fingerprint(),
        dirty_fingerprint,
        "取消后工程指纹一位不动（改动还在，只是没换工程）"
    );
    assert!(is_dirty_now(&undo), "取消后改动仍是未保存的");

    // ---- 腿 2：保存并继续 ----
    let saved_before = std::fs::read(&current_path).expect("旧工程文件必须在");
    window.invoke_new_project();
    assert!(
        window.get_new_project_confirm_open(),
        "仍脏时再次新建必须再弹三选框"
    );
    window.invoke_new_project_save_and_continue();
    assert!(
        !window.get_new_project_confirm_open(),
        "保存并继续后三选框必须关掉"
    );
    let saved_after = std::fs::read(&current_path).expect("旧工程文件必须还在");
    assert_ne!(
        saved_after, saved_before,
        "保存并继续必须先把旧工程写回磁盘（改动后的字节与落盘前不同）"
    );
    let written = std::fs::read(&target).expect("新建工程文件必须已生成");
    assert_eq!(written.len(), 1187, "新建工程必须是 1187 B 空工程容器");
    assert!(
        window.as_weak().upgrade().is_some(),
        "保存并继续后窗口必须还活着"
    );
    assert_eq!(rows(&window), 0, "新建后界面必须是 0 轨（空工程）");
    let current = undo.try_project().expect("换工程后端口里必须有活工程");
    assert_eq!(current.tracks.len(), 1, "新工程只剩主总线一条轨");
    assert!(current.clip_pool.is_empty(), "新工程不得残留旧工程的片段池");
    let receipt = window.get_module_status();
    assert!(
        receipt.contains("outcome=ok"),
        "新建回执必须是 outcome=ok，实测 {receipt:?}"
    );
    assert!(
        !is_dirty_now(&undo),
        "换成新空工程后脏基线必须跟着换（新工程此刻就是已保存态）"
    );

    // ---- 腿 3：保存失败（不可写路径）----
    // 新工程干净 ⇒ 直接走新建流程；换一个答不可写路径的选择器重接一次入口
    // （`on_*` setter 覆盖，与 `tests/export_midi_ui.rs` 反复调
    // `wire_module_actions_with` 同款）。
    let failing_picker = ScriptedPicker::answering_save(&unwritable);
    host::wire_new_project(&window, failing_picker, Rc::clone(&runtime), Some(&undo));
    window.invoke_new_project();
    let failed_receipt = window.get_module_status();
    assert!(
        failed_receipt.contains("error=create"),
        "不可写路径必须报 error=create，实测 {failed_receipt:?}"
    );
    assert!(
        window.get_show_floating_toast(),
        "保存失败必须亮浮动 toast（失败也要被看见）"
    );
    assert!(window.get_toast_is_error(), "保存失败的 toast 必须是错误态");
    assert!(
        window.get_toast_message().contains("新建空白工程失败"),
        "保存失败的 toast 必须点名新建失败，实测 {:?}",
        window.get_toast_message()
    );
    assert!(
        window.as_weak().upgrade().is_some(),
        "保存失败后窗口必须还活着"
    );
    assert_eq!(rows(&window), 0, "保存失败后界面仍是新空工程的 0 轨");
    assert!(!unwritable.exists(), "保存失败不得建出文件");
    let still = undo.try_project().expect("保存失败后端口里必须仍有活工程");
    assert_eq!(still.tracks.len(), 1, "保存失败后端口里仍是新空工程");
}
