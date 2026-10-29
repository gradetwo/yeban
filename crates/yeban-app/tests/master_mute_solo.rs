//! **主控通道条的静音 / 独奏** —— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 混音台主控通道条上的 M / S 两个按钮**不是装饰**：点击 ⇒
//! `host::wire_mixer_master_switch` 按**投影里的主总线身份**
//! （`bridge::master_device_track`）提交**恰好一次** `Op::SetTrackMute` /
//! `Op::SetTrackSolo` —— 与轨道头 / 通道条的开关是同一对 `Op`、同一条撤销栈。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | 两个 ID 在运行时树里、角色 = `Button`、登记名正确，且都登记过 |
//! | ②（静音 = 一步编辑） | `invoke_mixer_master_mute_toggle()` ⇒ `commit_count + 1`，权威工程里主总线 `mute = true`，界面 `master-mute` 跟着变；撤销回到原值 |
//! | ③（独奏 = 一步编辑） | 同上（`solo`） |
//! | ④（幂等边界） | 连续两次点击 = 两步编辑（一开一关），工程回到原值 |
//! | ⑤（文本层接线） | 主控按钮有 `TouchArea` ＋ `accessible-action-default` 同一落点；`console_tabs` / `app.slint` 原样上抛 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_macro_panel.rs` 同款）。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/ui_bpm_field.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::elements::ElementKind;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;

#[test]
fn the_master_mute_and_solo_buttons_commit_one_op_each() {
    let project = yeban_model::samples::filled_project();
    let master =
        yeban_app::bridge::master_device_track(&project).expect("filled_project 必须有主总线");
    let master_id = master.id;
    assert!(!master.mute && !master.solo, "夹具的主总线开关必须初始关闭");

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据: 主控静音/独奏>", "yeban-app", project.clone(), 1)
            .expect("打开撤销会话"),
    ));
    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        // Tab 1 = 调音台；主控通道条只在它可见时进运行时控件树。
        console_tab: 1,
        undo: Some(Rc::clone(&port)),
        ..LiveWiringOptions::default()
    };
    let live_ui = build_live_ui_with(&project, &options).expect("Tier-1 平台 + 活窗口 + 混音接线");
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());
    let mut plane = live_ui.into_control_plane(Permission::Interactive);
    let ui = &window;

    // ---- ① 可寻址 ----
    {
        let (tree, _) = plane.plane().tree().expect("ui/tree");
        for (id, label) in [
            ("mixer-master-mute-button", "主控静音"),
            ("mixer-master-solo-button", "主控独奏"),
        ] {
            let node = tree
                .nodes
                .iter()
                .find(|node| node.id == id)
                .unwrap_or_else(|| panic!("`{id}` 必须在运行时控件树里"));
            assert_eq!(
                node.role,
                ElementKind::Button.accessible_role(),
                "`{id}` 的角色必须与注册表一致"
            );
            assert_eq!(node.label, label, "`{id}` 的无障碍名必须与注册表一致");
            assert!(
                plane.registry().contains(id),
                "`{id}` 不在装配时的注册表里（[UI-TEST-001]）"
            );
        }
    }
    assert!(!ui.get_master_mute(), "装配后主控静音必须是关");
    assert!(!ui.get_master_solo(), "装配后主控独奏必须是关");

    // ---- ② 静音：一次点击 = 一步可撤销的编辑 ----
    let commits_before = port.display().commit_count;
    ui.invoke_mixer_master_mute_toggle();
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次点击 = 恰好一次 commit_ops"
    );
    assert!(
        port.project().tracks[&master_id].mute,
        "权威工程里主总线的 mute 必须变成 true"
    );
    assert!(ui.get_master_mute(), "界面读数必须跟着权威工程");
    let _ = port.perform(UiAction::Undo);
    assert!(
        !port.project().tracks[&master_id].mute,
        "撤销之后主总线必须回到不静音"
    );

    // ---- ③ 独奏 ----
    let commits_before = port.display().commit_count;
    ui.invoke_mixer_master_solo_toggle();
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次点击 = 恰好一次 commit_ops"
    );
    assert!(
        port.project().tracks[&master_id].solo,
        "权威工程里主总线的 solo 必须变成 true"
    );
    assert!(ui.get_master_solo(), "界面读数必须跟着权威工程");
    let _ = port.perform(UiAction::Undo);
    assert!(!port.project().tracks[&master_id].solo);

    // ---- ④ 一开一关 = 两步编辑，回到原值 ----
    let commits_before = port.display().commit_count;
    ui.invoke_mixer_master_mute_toggle();
    ui.invoke_mixer_master_mute_toggle();
    assert_eq!(
        port.display().commit_count,
        commits_before + 2,
        "一开一关 = 两步（⛔ 不是判重吞掉一次点击）"
    );
    assert!(
        !port.project().tracks[&master_id].mute,
        "两次点击之后必须回到不静音"
    );

    // ---- ⑤ 文本层接线 ----
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mixer = std::fs::read_to_string(manifest_dir.join("ui/console/mixer_console.slint"))
        .expect("读 mixer_console.slint");
    for (id, callback) in [
        ("mixer-master-mute-button", "mixer-master-mute-toggle"),
        ("mixer-master-solo-button", "mixer-master-solo-toggle"),
    ] {
        let start = mixer
            .find(&format!("accessible-id: \"{id}\""))
            .unwrap_or_else(|| panic!("找不到 `{id}`"));
        // 从语义 ID 起取一段窗口：无障碍动作在前、`TouchArea` 在 `UiText` 之后，
        // 两者都在同一个按钮块内（块长 < 900 字符，见 `mixer_console.slint`）。
        let block = &mixer[start..(start + 900).min(mixer.len())];
        assert!(
            block.contains(&format!(
                "accessible-action-default => {{ root.{callback}(); }}"
            )),
            "`{id}` 必须有无障碍动作（`{callback}`）"
        );
        assert!(
            block.contains(&format!("root.{callback}();")),
            "`{id}` 的 TouchArea 必须真的调用 `{callback}`"
        );
    }
    let tabs = std::fs::read_to_string(manifest_dir.join("ui/console/console_tabs.slint"))
        .expect("读 console_tabs.slint");
    let app = std::fs::read_to_string(manifest_dir.join("ui/app.slint")).expect("读 app.slint");
    for callback in ["mixer-master-mute-toggle", "mixer-master-solo-toggle"] {
        assert!(
            tabs.contains(&format!("callback {callback}();")),
            "`console_tabs.slint` 必须声明 `{callback}`"
        );
        assert!(
            app.contains(&format!("callback {callback}();")),
            "`app.slint` 必须声明 `{callback}`"
        );
        assert!(
            app.contains(&format!("root.{callback}();")),
            "`app.slint` 必须把 `{callback}` 原样转给宿主"
        );
    }
}
