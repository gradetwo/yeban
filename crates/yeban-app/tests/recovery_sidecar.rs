//! T-FIX-739：旁挂恢复文件的检测/提示/清理两路判据。
//!
//! | 路 | 断言 |
//! | - | ---- |
//! | 有恢复文件 | `detect` = Available；`startup_notice` 点名 sidecar 与 `--open` 恢复入口 |
//! | 无恢复文件 | `detect` = None；`startup_notice` = None（不误报） |
//! | 正常保存/退出 | `note_saved`（host 成功保存时调的就是它）删除 sidecar，之后不再提示 |
//! | 坏文件 | 非 ZIP 头 ⇒ `Invalid`，提示里响亮说"无法读取"，⛔ 不冒充可恢复 |

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::recovery::{RecoveryState, detect, note_saved, sidecar_path, startup_notice};
use yeban_app::scene::DemoScene;
use yeban_model::EntityId;
use yeban_model::samples::filled_project;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-739-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
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
fn a_recovery_sidecar_is_detected_and_offers_an_entry() {
    let scratch = Scratch::new("available");
    let target = scratch.dir.join("song.yeban");
    let project = filled_project();

    // 与真实自动保存同一条写入口（save::save_archive_parts_file）。
    let sidecar = yeban_app::recovery::write_snapshot(&target, &project, &[], &[])
        .expect("恢复快照必须能写出");
    assert_eq!(sidecar, sidecar_path(&target));
    assert!(sidecar.exists());
    assert!(
        !target.exists(),
        "夹具前提：主文件还没被正常保存过（不然 mtime 规则会判旁挂失效）"
    );

    assert_eq!(
        detect(&target),
        RecoveryState::Available {
            sidecar: sidecar.clone()
        }
    );
    let notice = startup_notice(&target).expect("有恢复文件必须提示");
    assert!(notice.contains(&sidecar.display().to_string()), "{notice}");
    assert!(notice.contains("--open"), "必须给恢复入口: {notice}");
    assert!(
        notice.contains("原文件"),
        "必须说明原文件未被覆盖: {notice}"
    );
}

#[test]
fn no_sidecar_means_no_prompt() {
    let scratch = Scratch::new("missing");
    let target = scratch.dir.join("song.yeban");
    assert_eq!(detect(&target), RecoveryState::None);
    assert_eq!(startup_notice(&target), None);
}

#[test]
fn a_normal_save_invalidates_the_sidecar() {
    let scratch = Scratch::new("saved");
    let target = scratch.dir.join("song.yeban");
    let project = filled_project();
    yeban_app::recovery::write_snapshot(&target, &project, &[], &[]).expect("写恢复快照");
    assert!(startup_notice(&target).is_some());

    // host::wire_save_with_engine 在 SaveOutcome::Saved 分支调的就是这一条。
    note_saved(&target);

    assert_eq!(detect(&target), RecoveryState::None);
    assert_eq!(startup_notice(&target), None, "正常保存后不得再提示恢复");
    assert!(!sidecar_path(&target).exists());
}

#[test]
fn a_corrupt_sidecar_is_reported_loudly_not_as_recoverable() {
    let scratch = Scratch::new("corrupt");
    let target = scratch.dir.join("song.yeban");
    std::fs::write(sidecar_path(&target), b"not a zip").expect("写坏文件");

    match detect(&target) {
        RecoveryState::Invalid { reason, .. } => {
            assert!(reason.contains("ZIP"), "原因必须点名不是容器: {reason}");
        }
        other => panic!("坏恢复文件必须判 Invalid，不得当真: {other:?}"),
    }
    let notice = startup_notice(&target).expect("坏文件也要响亮提示");
    assert!(notice.contains("无法读取"), "{notice}");
    assert!(
        !notice.contains("--open"),
        "坏文件不得给误导性的恢复入口: {notice}"
    );
}

/// 正常保存的**接线**级判据：宿主保存成功后必须让旁挂文件失效，不只是 API 自觉。
#[test]
fn a_real_save_removes_the_sidecar_through_the_host_wiring() {
    let scratch = Scratch::new("host-save");
    let target = scratch.dir.join("song.yeban");
    let project = filled_project();
    yeban_app::recovery::write_snapshot(&target, &project, &[], &[]).expect("写恢复快照");
    assert!(startup_notice(&target).is_some(), "夹具前提：旁挂可用");

    let shared = Rc::new(RefCell::new(project));
    let view = ViewState::from_project(&shared.borrow()).expect("投影");
    let scene = DemoScene::from_view(&view);
    let live = yeban_ui_test_port::render::LivePort::new(
        yeban_ui_test_port::Size::new(1440, 900),
        yeban_ui_test_port::port::Permission::Interactive,
        None,
        || host::build_main_window(&view, &scene),
    )
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_save_with_engine(
        ui,
        host::SaveStatus {
            target: Some(target.clone()),
            project: Rc::clone(&shared),
        },
        None,
        Rc::new(yeban_app::file_picker::NoPicker),
        None,
        None,
    );

    ui.invoke_save_project();
    assert!(
        ui.get_save_status().contains("已保存"),
        "{:?}",
        ui.get_save_status()
    );
    assert!(target.exists(), "正常保存必须写出主文件");
    assert!(
        !sidecar_path(&target).exists(),
        "宿主保存成功后必须删除旁挂恢复文件"
    );
    assert_eq!(detect(&target), RecoveryState::None);
}
