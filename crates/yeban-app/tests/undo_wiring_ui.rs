//! **撤销接线判据（界面侧）** —— ADR-0001 **D45** 的"人按 `Cmd+Z` 真的能撤销"。
//!
//! 与 MCP 侧那份（`crates/yeban-mcp/tests/undo_wiring.rs`）**互补**：
//!
//! | 判据 | 落点 |
//! | :--- | :--- |
//! | ⑧ UI 路径与 MCP 路径**逐字节相同**（同一串 `Op`、同一个 `undo_session`） | [`the_ui_path_matches_the_shared_implementation_byte_for_byte`] |
//! | ⑨ 界面动作**真的**回退工程（动作日志断言，不用控件树） | [`a_click_really_rolls_the_project_back`] |
//! | ⑨ `Cmd+Z` 整条链（策略表 → 动作 → 工程回退） | [`the_cmd_z_chain_reaches_the_model`] |
//! | ⑫ 撤销不越过"工程打开"边界（新会话 = 新端口） | [`a_new_project_open_starts_a_fresh_undo_session`] |
//! | ⑪ 撤销 → 保存（真容器）→ 重开 ⇒ 与内存态一致 | [`saved_after_an_undo_reopens_to_the_same_bytes`] |
//! | ⑩ 撤销后工程仍然 `validate()` 通过 | [`the_project_still_validates_after_a_ui_undo`] |
//! | 显示态来自模型读数（不是界面算的） | [`the_display_state_comes_from_the_model_readings`] |
//! | 重投影：撤销之后界面拿到的是**回退后**的工程 | [`the_projection_after_a_ui_undo_is_the_rolled_back_project`] |
//!
//! 为什么不写"控件树里有 `undo-tree-undo-button`"当证据：三方对齐矩阵的错位 5
//! 明确警告过这种假接线（元素都在、动作全是 `trace()`）。本文件断言的是
//! **工程字节真的回退了**，元素 ID 只是语义寻址（由 `elements.rs` 的判据管）。

use yeban_app::cli::{self, ProjectSource};
use yeban_app::undo::undo_session::wiring_fixture;
use yeban_app::undo::{
    ActionOutcome, UiAction, UndoPort, UndoSession, dispatch_key, perform_key, project_fingerprint,
};
use yeban_model::Op;
use yeban_model::container::{self, ContainerLimits};
use yeban_model::samples::filled_project;

const NOW: u64 = 1_760_000_000_000;

fn port() -> UndoPort {
    UndoPort::new(UndoSession::open("<判据>", "yeban-app", filled_project(), NOW).expect("打开"))
}

/// 提交夹具那次"改一个音符力度"（与 MCP 侧 `yeban_edit_notes` 编译出的 op 同形）。
fn commit_fixture(port: &UndoPort) -> Op {
    let fixture = wiring_fixture(&port.project()).expect("夹具");
    port.commit_ops(NOW + 1, "接线判据", vec![fixture.op()])
        .expect("提交");
    fixture.op()
}

/// 判据 ⑧（界面侧那一半）：界面路径撤出来的工程与**原始工程**逐字节相同。
///
/// MCP 侧的同名判据断言"工具路径 = 原始工程"，两条合起来就得到
/// "UI 路径 = MCP 路径"（同一串字节，同一个 `undo_session`）。
#[test]
fn the_ui_path_matches_the_shared_implementation_byte_for_byte() {
    let port = port();
    let pristine = project_fingerprint(&filled_project()).expect("指纹");
    commit_fixture(&port);
    assert_ne!(
        port.fingerprint().expect("指纹"),
        pristine,
        "夹具必须真的改了工程"
    );

    let outcome = port.perform(UiAction::Undo);
    assert!(outcome.changed(), "{outcome:?}");
    assert_eq!(
        port.fingerprint().expect("指纹"),
        pristine,
        "界面路径撤销后必须逐字节回到原始工程"
    );
    // 会话态与 MCP 侧一致（都是"已撤销 1 步"），但工程里没有它。
    assert_eq!(port.display().undone, 1);
    assert_eq!(port.display().undoable, 0);
    assert_eq!(
        project_fingerprint(&port.project()).expect("指纹"),
        pristine,
        "指纹与工程字节同源"
    );
}

/// 判据 ⑨：**"点击"⇒ 工程真的回退了一版**（动作日志是证据，不是控件树）。
#[test]
fn a_click_really_rolls_the_project_back() {
    let port = port();
    let pristine = port.fingerprint().expect("指纹");
    commit_fixture(&port);
    let edited = port.fingerprint().expect("指纹");
    assert_ne!(pristine, edited);

    // 这就是"点了 `undo-tree-undo-button`"落到的那一个动作。
    let outcome = port.perform(UiAction::Undo);
    assert_eq!(
        outcome,
        ActionOutcome::Changed {
            steps: 1,
            undone_total: 1,
            op_kinds: vec!["Batch".to_owned()],
        }
    );
    let record = port.last_record().expect("动作日志");
    assert_eq!(record.action, "undo");
    assert!(
        record.changed_project(),
        "动作日志必须证明工程真的变了: {record:?}"
    );
    assert_eq!(record.fingerprint_before.as_deref(), Some(edited.as_str()));
    assert_eq!(record.fingerprint_after.as_deref(), Some(pristine.as_str()));
    assert_eq!(port.fingerprint().expect("指纹"), pristine);
    // 二次点击（没有历史了）⇒ 拒绝，但**照样留痕**。
    assert!(matches!(
        port.perform(UiAction::Undo),
        ActionOutcome::Refused { .. }
    ));
    assert!(!port.last_record().expect("日志").changed_project());
    assert_eq!(port.records().len(), 2, "两次点击两条记录");
}

/// 判据 ⑨：`Cmd+Z` / `Cmd+Shift+Z` / `Cmd+Shift+H` 整条链都落到同一个实现。
#[test]
fn the_cmd_z_chain_reaches_the_model() {
    let port = port();
    let pristine = port.fingerprint().expect("指纹");
    commit_fixture(&port);
    let edited = port.fingerprint().expect("指纹");

    // 策略表 → 动作映射（`input.rs` 的解析结果）。
    assert_eq!(
        dispatch_key(yeban_app::input::Action::Undo),
        Some(UiAction::Undo)
    );
    assert_eq!(
        dispatch_key(yeban_app::input::Action::Redo),
        Some(UiAction::Redo)
    );

    // 整条链：`Cmd+Z` ⇒ 工程回退。
    let canvas = yeban_app::input::InputContext::new();
    let outcome = perform_key(
        &port,
        &canvas,
        yeban_app::input::PhysicalKey::KeyZ,
        yeban_app::input::Modifiers::meta(),
    );
    assert!(matches!(
        outcome,
        Some(ActionOutcome::Changed { steps: 1, .. })
    ));
    assert_eq!(port.fingerprint().expect("指纹"), pristine);

    // `Cmd+Shift+Z` ⇒ 回到编辑过的那一版。
    let outcome = perform_key(
        &port,
        &canvas,
        yeban_app::input::PhysicalKey::KeyZ,
        yeban_app::input::Modifiers::ctrl_shift(),
    );
    assert!(matches!(
        outcome,
        Some(ActionOutcome::Changed { steps: 1, .. })
    ));
    assert_eq!(port.fingerprint().expect("指纹"), edited);

    // 文本输入框聚焦时 `Cmd+Z` 属于文本框（策略表说 PassThrough）⇒ 不动工程。
    let mut text = yeban_app::input::InputContext::new();
    text.set_focus(yeban_app::input::Focus::TextInput);
    assert_eq!(
        perform_key(
            &port,
            &text,
            yeban_app::input::PhysicalKey::KeyZ,
            yeban_app::input::Modifiers::meta(),
        ),
        None
    );
    assert_eq!(
        port.fingerprint().expect("指纹"),
        edited,
        "文本框里的 Cmd+Z 不得改工程"
    );
}

/// 判据 ⑫：打开**另一个**工程 = 新会话 ⇒ 不能撤到上一个工程的状态。
#[test]
fn a_new_project_open_starts_a_fresh_undo_session() {
    let first = port();
    commit_fixture(&first);
    assert_eq!(first.display().undoable, 1);

    // `main.rs` 的打开路径就是"新建一个会话 + 新建一个端口"。
    let second = UndoPort::new(
        UndoSession::open("另一个工程", "yeban-app", filled_project(), NOW).expect("打开"),
    );
    let before = second.fingerprint().expect("指纹");
    assert_eq!(second.display().undoable, 0);
    assert_eq!(second.display().commit_count, 1, "新会话只有一条根提交");
    assert!(matches!(
        second.perform(UiAction::Undo),
        ActionOutcome::Refused { .. }
    ));
    assert_eq!(second.fingerprint().expect("指纹"), before);
}

/// 判据 ⑪：撤销 → 保存（真容器）→ 重开 ⇒ 与内存态一致；游标不落盘但可推导。
#[test]
fn saved_after_an_undo_reopens_to_the_same_bytes() {
    let port = port();
    let pristine = port.fingerprint().expect("指纹");
    commit_fixture(&port);
    let edited = port.fingerprint().expect("指纹");
    port.perform(UiAction::Undo);
    assert_eq!(port.fingerprint().expect("指纹"), pristine);

    // "保存" = 容器写出。`history.dag` 的**字节**在这里是占位：`yeban-app` 没有
    // `serde_json` 依赖，因此它既不写也不解析那份 JSON（图谱由模型对象交回，
    // 见台账的 needs：`history.dag` 的公开编解码面应住在 `yeban-model`）。
    let dag = b"fixture-history-dag";
    let bytes = container::write_project_container(
        &port.project(),
        dag,
        &std::collections::BTreeMap::new(),
    )
    .expect("容器写出");
    // "再打开" = 容器读回（唯一工程格式 ADR-0001 D43）。
    let archive =
        container::read_project_container(&bytes, &ContainerLimits::default()).expect("容器读回");
    assert_eq!(archive.history_dag, dag, "history.dag 必须保真");
    assert_eq!(
        container::write_project_container(
            &archive.project,
            &archive.history_dag,
            &std::collections::BTreeMap::new()
        )
        .expect("再写出"),
        bytes,
        "容器往返必须逐字节稳定"
    );

    // 图谱由**调用方**以模型对象交回（见上面的 needs）。
    let reopened = UndoPort::new(
        UndoSession::open_with_graph("<reopen>", "yeban-app", archive.project, port.graph(), NOW)
            .expect("再打开"),
    );
    assert_eq!(
        reopened.fingerprint().expect("指纹"),
        pristine,
        "再打开之后的工程 = 撤销后的内存态"
    );
    // 游标**没有**落盘，但重开之后由（文档, 图谱）推导出来 ⇒ 重做仍然可用。
    assert_eq!(reopened.display().undone, 1, "游标是推导出来的");
    assert_eq!(reopened.display().undoable, 0);
    assert!(reopened.display().can_redo);
    assert!(reopened.perform(UiAction::Redo).changed());
    assert_eq!(
        reopened.fingerprint().expect("指纹"),
        edited,
        "重做回到编辑过的状态"
    );
}

/// 判据 ⑩：撤销之后工程仍然通过 `validate()`（路由 / 自动化 / 片段池）。
#[test]
fn the_project_still_validates_after_a_ui_undo() {
    let port = port();
    commit_fixture(&port);
    port.project().validate().expect("提交后合法");
    port.perform(UiAction::Undo);
    port.project().validate().expect("撤销后合法");
    port.perform(UiAction::Redo);
    port.project().validate().expect("重做后合法");
}

/// 判据：显示态（能否撤销 / 还能撤几步 / 提交数）**来自模型读数**。
#[test]
fn the_display_state_comes_from_the_model_readings() {
    let port = port();
    let display = port.display();
    assert!(!display.can_undo, "新会话没有可撤销的步骤");
    assert!(!display.can_redo);
    assert_eq!(display.undoable, 0);
    assert_eq!(display.undone, 0);
    assert_eq!(display.commit_count, 1, "模型图谱里只有根提交");
    assert_eq!(display.branch, "main");

    commit_fixture(&port);
    let display = port.display();
    assert!(display.can_undo);
    assert_eq!(display.undoable, 1);
    assert_eq!(display.commit_count, 2, "提交数来自图谱");
    // 界面上显示的三个数就是这三个（`host::apply_undo` 只做读取与注入）。
    assert_eq!(
        (display.undoable, display.undone, display.commit_count),
        (1, 0, 2)
    );
}

/// 判据：撤销之后**投影**拿到的是回退后的工程（界面画的是回退后那一版）。
#[test]
fn the_projection_after_a_ui_undo_is_the_rolled_back_project() {
    let port = port();
    let pristine_view = cli::project_view(&filled_project()).expect("投影");
    commit_fixture(&port);
    // 编辑之后投影**仍然可用**（撤销不能把工程搞成投影不了的状态）。
    let edited_view = cli::project_view(&port.project()).expect("编辑后投影");
    assert_eq!(edited_view.title, pristine_view.title);

    port.perform(UiAction::Undo);
    let rolled_back = cli::project_view(&port.project()).expect("回退后投影");
    // 撤销之后的投影 = 原始工程的投影（界面拿到的是回退后那一版，不是游标动了画面没动）。
    assert_eq!(rolled_back.title, pristine_view.title);
    assert_eq!(rolled_back.track_names(), pristine_view.track_names());
    assert_eq!(rolled_back.clip_ulids(), pristine_view.clip_ulids());
    assert_eq!(
        project_fingerprint(&port.project()).expect("指纹"),
        project_fingerprint(&filled_project()).expect("指纹")
    );
}

/// 判据：`--open` 的真实来源标签会进撤销会话（报告行可核对）。
#[test]
fn the_session_label_comes_from_the_project_source() {
    let loaded_file = ProjectSource::File {
        path: std::path::PathBuf::from("/tmp/demo.yeban"),
        bytes: 42,
    };
    let label = match &loaded_file {
        ProjectSource::File { path, .. } => path.display().to_string(),
        ProjectSource::Sample(_) => String::from("sample"),
    };
    assert!(label.ends_with("demo.yeban"));
    // 会话本身对"来源"只做留痕（提交信息用），不影响撤销语义。
    let session = UndoSession::open(label, "yeban-app", filled_project(), NOW).expect("打开");
    assert_eq!(session.label(), "/tmp/demo.yeban");
    assert_eq!(session.graph().commit_count(), 1);
}

/// 判据：`OpOrigin` 与会话作者如实落在提交上（撤销重做**不**产生新提交）。
#[test]
fn undo_and_redo_never_add_commits() {
    let port = port();
    let before = port.display().commit_count;
    commit_fixture(&port);
    let after_commit = port.display().commit_count;
    assert_eq!(after_commit, before + 1);
    port.perform(UiAction::Undo);
    assert_eq!(
        port.display().commit_count,
        after_commit,
        "撤销不动提交图谱"
    );
    port.perform(UiAction::Redo);
    assert_eq!(port.display().commit_count, after_commit, "重做也不动");
    assert_eq!(
        port.graph().commit_count(),
        after_commit,
        "图谱本身就是这个读数"
    );
}
