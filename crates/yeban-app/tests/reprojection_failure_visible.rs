//! **重投影失败必须是一个可区分的结局，并且看得见**（T-FIX-300）。
//!
//! ## 要证的句子
//!
//! 会话侧改了工程、权威修订号前进了，但这一帧投影失败时：
//!
//! 1. `sync_now` 返回 [`ProjectionOutcome::ProjectionFailed`]，**不是** `Unchanged`；
//! 2. 生产窗口的错误级常驻回执（提示条 ＋ 模块回执行）写着“重投影…旧画面…重试”；
//! 3. 已投影修订号**不**推进 ⇒ 下一次通知会重试，重试成功就回到 `Reprojected`。
//!
//! ## 判据缝
//!
//! [`AuthorityMirror::fail_next_projection_for_test`] 只让**下一次**投影走 `Err` 分支，
//! 生产路径永不置位；它不另造成功路径，喂的是同一个 `BridgeError`。
#![cfg(feature = "in-process-mcp")]

use std::path::PathBuf;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::mcp_mount::{InProcessMcp, SessionSource};
use yeban_app::reproject::{AuthorityMirror, ProjectionOutcome};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_mcp::domain::HostAction;
use yeban_model::ops::{Op, OpOrigin};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

/// 无头 Tier-1 窗口（平台是进程级单例；本文件只放一个 `#[test]`）。
fn build_window(project: &yeban_model::project::YebanProjectV1) -> LivePort<MainWindow> {
    let view = ViewState::from_project(project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口")
}

#[test]
fn a_failed_reprojection_is_a_distinct_outcome_and_says_so_in_the_window() {
    let project = yeban_model::samples::filled_project();
    let mount = InProcessMcp::start_for_project(
        true,
        project.clone(),
        SessionSource::InMemory(PathBuf::from("<test:reprojection-failure>")),
        Vec::new(),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    let live = build_window(&project);
    let ui = live.ui();
    let mirror = AuthorityMirror::install(ui, &mount).expect("服务未停机，观察者必须装上");

    // ① 起点已投影 == 权威当前：失败不是因为“本来就没新版本”。
    let before = mirror.projected_revision();
    assert_eq!(before, mount.project_authority().apply_revision());

    // ② 会话侧改工程：提交一改名 op，权威修订号 +1。
    let (track_id, old_name) = project
        .tracks
        .iter()
        .next()
        .map(|(id, track)| (*id, track.name.clone()))
        .expect("样本工程必须有音轨");
    mount
        .project_authority()
        .apply_host(HostAction::Commit {
            now_ms: 1_700_000_000_000,
            origin: OpOrigin::UserUi,
            message: "T-FIX-300 test: rename a track".to_owned(),
            ops: vec![Op::SetTrackName {
                track_id,
                old_name,
                new_name: "T-FIX-300 renamed".to_owned(),
            }],
        })
        .expect("权威提交必须成功");
    let revision = mount.project_authority().apply_revision();
    assert_eq!(revision, before + 1, "提交必须推进修订号");

    // ③ 让这一帧投影失败：结局必须独立，窗口必须看见。
    mirror.fail_next_projection_for_test();
    let outcome = mirror.sync_now(ui);
    assert_eq!(
        outcome,
        ProjectionOutcome::ProjectionFailed { revision },
        "失败不得复用 Unchanged（那是“没有新版本”的结局）"
    );
    assert_eq!(
        mirror.projected_revision(),
        before,
        "失败不得推进已投影修订号"
    );
    let status = ui.get_module_status().to_string();
    assert!(
        status.contains("重投影") && status.contains("旧画面") && status.contains("重试"),
        "错误级常驻回执必须写明重投影失败与重试，实得: {status}"
    );
    assert!(ui.get_toast_is_error(), "提示条必须按错误级显示");
    assert!(ui.get_show_floating_toast(), "提示条必须真的弹出来");
    assert!(!ui.get_module_status_succeeded(), "模块回执行必须标为失败");

    // ④ 下一次通知重试：同一份权威、同一窗口，成功路径回到 Reprojected。
    let retry = mirror.sync_now(ui);
    assert_eq!(
        retry,
        ProjectionOutcome::Reprojected { revision },
        "失败后下一次必须重试并成功，而不是永久停在旧画面"
    );
    assert_eq!(mirror.projected_revision(), revision);
    assert_eq!(mirror.dispatch_failures(), 0, "本判据没有走到调度失败");
}
