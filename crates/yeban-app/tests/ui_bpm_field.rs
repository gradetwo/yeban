//! **走带条 BPM 敲入**（`ui/transport.slint` 的 `transport-bpm-field`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 走带条的 BPM 读数**不只是一个文本**：它的编辑面（`bpm-input`）在回车时把原样文本
//! 交给宿主，`host::wire_bpm_commit` 解析 / 校验 / 判重之后提交**恰好一次**
//! `Op::SetTempo` —— 一步可撤销的编辑。非法输入（非数字 / 越出 `MIN_BPM..=MAX_BPM` /
//! 非有限）与同值输入**不产生**提交，并且输入框被写回权威读数（⛔ 不留冒充权威的草稿）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | `transport-bpm-field` 在树里、角色 = `TextInput`、登记名 = "速度 BPM"，且读数 = 权威工程的 `{:.2}` |
//! | ②（回车 = 一步编辑） | `invoke_bpm_commit("140")` ⇒ `commit_count + 1`，工程 `bpm = 140.0`，界面读数 "140.00"；撤销回到夹具速度 |
//! | ③（四条不提交） | 非数字 / 低于 `MIN_BPM` / 高于 `MAX_BPM` / 非有限（`NaN` / `inf`）各不提交，且读数被写回权威值 |
//! | ④（判重） | 同值输入不产生提交（`to_bits` 相同 ⇒ 不造空撤销步） |
//! | ⑤（提交边界） | 两端闭区间 `20` / `999` 可提交；`19.999` / `999.001` 不可 |
//! | ⑥（文本层接线） | `transport.slint` 的 `accepted` ⇒ `bpm-commit`，`app.slint` 把回调原样转给主窗口 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_macro_panel.rs` 同款）。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/ui_macro_panel.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::elements::ElementKind;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;

/// 夹具速度（`yeban_model::samples::filled_project`）。判据的读数以它为准。
const FIXTURE_BPM: f64 = 128.0;

fn displayed(bpm: f64) -> String {
    format!("{bpm:.2}")
}

#[test]
fn the_bpm_field_commits_exactly_one_set_tempo_on_enter() {
    let project = yeban_model::samples::filled_project();
    assert_eq!(
        project.bpm, FIXTURE_BPM,
        "夹具速度变了：本判据的读数要一起改"
    );

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据: BPM 敲入>", "yeban-app", project.clone(), 1)
            .expect("打开撤销会话"),
    ));
    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        undo: Some(Rc::clone(&port)),
        ..LiveWiringOptions::default()
    };
    let live_ui = build_live_ui_with(&project, &options).expect("Tier-1 平台 + 活窗口 + BPM 接线");
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());
    let mut plane = live_ui.into_control_plane(Permission::Interactive);
    let ui = &window;

    // ---- ① 可寻址：语义节点在场，读数 = 权威工程的 `{:.2}` ----
    {
        let (tree, _) = plane.plane().tree().expect("ui/tree");
        let node = tree
            .nodes
            .iter()
            .find(|node| node.id == "transport-bpm-field")
            .expect("`transport-bpm-field` 必须在运行时控件树里");
        assert_eq!(
            node.role,
            ElementKind::TextInput.accessible_role(),
            "BPM 输入框的角色必须与注册表一致"
        );
        assert_eq!(node.label, "速度 BPM", "无障碍名必须与注册表一致");
        assert!(
            plane.registry().contains("transport-bpm-field"),
            "`transport-bpm-field` 不在装配时的注册表里（[UI-TEST-001]）"
        );
    }
    assert_eq!(
        ui.get_bpm_display().as_str(),
        displayed(FIXTURE_BPM),
        "装配后的读数必须来自权威工程"
    );

    // ---- ② 回车 = 恰好一次 SetTempo；撤销回去 ----
    let commits_before = port.display().commit_count;
    ui.invoke_bpm_commit("140".into());
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次回车 = 恰好一次 commit_ops"
    );
    assert!(
        (port.project().bpm - 140.0).abs() < f64::EPSILON,
        "权威工程的 bpm 必须是 140，实得 {}",
        port.project().bpm
    );
    assert_eq!(
        ui.get_bpm_display().as_str(),
        "140.00",
        "提交之后必须重投影到界面上"
    );

    // 撤销走**生产路径**（`.slint` 的 `key-action` 那同一格）：`port.perform` 只改权威工程，
    // 不经过 `refresh_undo_window`/`apply_view`，界面读数不会跟着动。
    assert!(
        ui.invoke_key_action("z".into(), false, false, false, true),
        "`Cmd+Z` 必须被 DAW 消费"
    );
    assert!(
        (port.project().bpm - FIXTURE_BPM).abs() < f64::EPSILON,
        "撤销之后必须回到夹具速度，实得 {}",
        port.project().bpm
    );
    assert_eq!(
        ui.get_bpm_display().as_str(),
        displayed(FIXTURE_BPM),
        "撤销之后界面读数必须跟着权威工程"
    );

    // ---- ③ 四条不提交：非法 / 越界 / 非有限 ----
    let commits_before = port.display().commit_count;
    for bad in ["abc", "", "0", "19.999", "1000", "999.001", "NaN", "inf"] {
        // 先模拟用户已经在输入框里敲了草稿，再按回车。
        ui.set_bpm_display("12345".into());
        ui.invoke_bpm_commit(bad.into());
        assert_eq!(
            port.display().commit_count,
            commits_before,
            "非法输入 {bad:?} ⛔ 不许提交"
        );
        assert!(
            (port.project().bpm - FIXTURE_BPM).abs() < f64::EPSILON,
            "非法输入 {bad:?} 之后权威工程必须原封不动"
        );
        assert_eq!(
            ui.get_bpm_display().as_str(),
            displayed(FIXTURE_BPM),
            "非法输入 {bad:?} 之后输入框必须被写回权威读数（不留草稿）"
        );
    }

    // ---- ④ 判重：同值输入不造空撤销步 ----
    ui.set_bpm_display("随便一个草稿".into());
    ui.invoke_bpm_commit(displayed(FIXTURE_BPM).into());
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "同值输入 ⛔ 不许提交"
    );
    assert_eq!(
        ui.get_bpm_display().as_str(),
        displayed(FIXTURE_BPM),
        "同值输入之后读数仍是权威值"
    );

    // ---- ⑤ 提交边界：两端闭区间可提交 ----
    for edge in [yeban_model::project::MIN_BPM, yeban_model::project::MAX_BPM] {
        let before = port.display().commit_count;
        ui.invoke_bpm_commit(displayed(edge).into());
        assert_eq!(
            port.display().commit_count,
            before + 1,
            "闭区间端点 {edge} 必须可提交"
        );
        assert!((port.project().bpm - edge).abs() < f64::EPSILON);
        assert_eq!(ui.get_bpm_display().as_str(), displayed(edge));
        assert!(
            ui.invoke_key_action("z".into(), false, false, false, true),
            "闭区间端点 {edge} 提交后 `Cmd+Z` 必须被 DAW 消费"
        );
    }

    // ---- ⑥ 文本层接线（本机不编译 Slint 的那一半，读源码取证） ----
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let transport = std::fs::read_to_string(manifest_dir.join("ui/transport.slint"))
        .expect("读 transport.slint");
    assert!(
        transport.contains("text <=> root.bpm-display;"),
        "输入框必须**双向**绑定 —— 单向绑定会被第一次按键打断"
    );
    assert!(
        transport.contains("accepted => {") && transport.contains("root.bpm-commit(self.text);"),
        "回车（`accepted`）必须把原样文本交给 `bpm-commit`"
    );
    let app = std::fs::read_to_string(manifest_dir.join("ui/app.slint")).expect("读 app.slint");
    assert!(
        app.contains("bpm-display <=> root.bpm-display;"),
        "主窗口与走带条的读数必须双向同源"
    );
    assert!(
        app.contains("bpm-commit(text) => {") && app.contains("root.bpm-commit(text);"),
        "主窗口必须把走带条的回调原样转给宿主"
    );
}
