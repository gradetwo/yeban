//! **界面上按保存 = 写活会话**（`ROAD-M4-008` 选项 (a) 的收口判据）。
//!
//! ## 这条判据要证的句子
//!
//! 默认构建（**没有**控制面权威）里，`host::wire_save` 装的保存回调写出去的是
//! **活会话此刻**那一版：工程 + 它的 `CommitGraph`（编码成 `history.dag`）。
//! 此前它写的是装配时刻的快照 + 零字节 `history.dag`，于是界面里的编辑与整条撤销链
//! 都到不了磁盘，而状态栏还报"已保存"。
//!
//! ## 判据的**承重步**（对照活会话，而不是"有没有提交"）
//!
//! ⚠ 新鲜的 GUI 会话**恒有**一条 genesis 提交 ⇒ "写出的 DAG 能解出 ≥1 条提交"
//! 会因为**错误的原因**通过。因此本判据比的是**活会话自己编码出来的字节**：
//! `reopened.history_dag == encode_history_dag(&live_port.graph())`。
//!
//! | 判据 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | A（DAG，承重） | 文件里的 `history.dag` 与**活会话**编码的图谱**逐字节相同**，且**长于**装配时刻的起始图谱 | 证"保存写的是活会话的图谱"。不证"图谱内容正确"（那是 `yeban-model` 的编解码判据） |
//! | B（工程，承重） | 重开的容器里，那个被编辑的音符力度 = **活会话**里的力度；起始容器里它是别的值 | 证"保存写的是活会话的工程"。不证"编辑本身对"（那是 `undo_wiring_ui.rs` 的判据） |
//! | 读的一侧（承重） | 用 `decode_history_dag` 把文件里的图谱读回、`open_with_graph` 重开会话，**重做**还能回到编辑后的工程 | 证"保存 → 重开"之后撤销链**真的**活着。不证"跨进程"（那要跑真二进制） |
//! | 落点 | `by_authority == false`（本地原子写） | 证这条用例走的确实是默认构建那条路 |
//!
//! ## 这条判据怎么变红
//!
//! - 保存回调退回 `SaveStatus.project`（装配快照）⇒ B 红（力度是起始值）；
//! - 保存写出空 `history.dag`（`save_project_file` 的老行为）⇒ A 红（0 字节）；
//! - 打开时丢掉 `history.dag`（`UndoSession::open` 而不是 `open_with_graph`）⇒ 读的一侧红。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::open::open_project_document_file;
use yeban_app::save_action::{SaveOutcome, dispatch_save};
use yeban_app::scene::DemoScene;
use yeban_app::undo::undo_session::wiring_fixture;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::CommitGraph;
use yeban_model::container::{self, write_project_container};
use yeban_model::samples::filled_project;

const NOW: u64 = 1_760_000_000_000;
const WINDOW: yeban_ui_test_port::Size = yeban_ui_test_port::Size::new(1440, 900);

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-gui-save-live-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
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

/// 工程里某个音符的力度（读不到就是 `None`）。
fn velocity_of(
    project: &yeban_model::YebanProjectV1,
    note_id: &yeban_model::ids::EntityId,
) -> Option<u8> {
    project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values())
        .find(|note| note.id == *note_id)
        .map(|note| note.velocity)
}

#[test]
fn the_local_save_entry_writes_the_live_session_project_and_graph() {
    // ---- 1. 无头 Tier-1 平台 + 磁盘上一份真容器（`--open <path>` 的等价物）----
    let tier1 =
        yeban_ui_test_port::render::Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let scratch = Scratch::new();
    let target = scratch.dir.join("live.yeban");
    let pristine = filled_project();
    let starting_graph = CommitGraph::new();
    let starting_dag = serde_json::to_vec(&starting_graph).expect("起始图谱 JSON");
    std::fs::write(
        &target,
        write_project_container(&pristine, &starting_dag, &BTreeMap::new()).expect("写起始容器"),
    )
    .expect("落起始容器");

    // ---- 2. `cli::load_project` + `open_undo_session`：活会话（`main.rs` 的顺序）----
    let loaded = open_project_document_file(&target, &Default::default())
        .expect("打开起始容器（= 启动读数）");
    let pristine_velocity = velocity_of(
        &loaded.archive.project,
        &wiring_fixture(&loaded.archive.project)
            .expect("夹具音符")
            .note_id,
    );
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            target.display().to_string(),
            "yeban-app",
            loaded.archive.project.clone(),
            NOW,
        )
        .expect("打开 GUI 自己的撤销会话"),
    ));

    // ---- 3. 生产窗口 + 生产接线（与 `run_gui` 同一条顺序）----
    let view = ViewState::from_project(&port.project()).expect("装配时投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口构造入口");
    tier1
        .resize(WINDOW)
        .expect("窗口尺寸定下来之后再注入一次布局");
    // 第三参数 = **活会话**（`main.rs` 传的就是这个 `undo_port`）。
    host::wire_save(
        &ui,
        host::SaveStatus {
            target: Some(target.clone()),
            // 刻意是**装配时刻**的快照（`main.rs` 传的是 `loaded.archive.project.clone()`）：
            // 若回调退回它，下面 B 会读到一个"没编辑过"的力度。
            project: loaded.archive.project.clone(),
        },
        Some(Rc::clone(&port)),
        // ⭐ `#86` 乙：本判据的装配**给了**目标路径（`target: Some(..)`）⇒ 一次都不该弹窗。
        // `NoPicker` 让那句承诺**可执行**：真弹了也只会拿到 `None`（本判据因此仍然绿）。
        Rc::new(yeban_app::file_picker::NoPicker),
        None,
    );
    assert_eq!(
        ui.get_save_status().to_string(),
        "",
        "还没按过保存 ⇒ 状态文本必须是空的（不是'已保存'）"
    );

    // ---- 4. 界面里真的发生一次编辑：改一个音符的力度（与卷帘铅笔同一个 `commit_ops`）----
    let fixture = wiring_fixture(&port.project()).expect("夹具音符");
    let note_id = fixture.note_id;
    let expected_velocity = fixture.new_velocity;
    assert_ne!(
        Some(expected_velocity),
        pristine_velocity,
        "夹具必须真的改力度（否则 B 的对照没有意义）"
    );
    port.commit_ops(NOW + 1, "pencil: velocity", vec![fixture.op()])
        .expect("提交编辑");
    let live_project = port.project();
    let live_graph = port.graph();
    let expected_dag = yeban_model::commit::encode_history_dag(&live_graph);
    assert_eq!(
        velocity_of(&live_project, &note_id),
        Some(expected_velocity),
        "活会话里那一个音符必须已经是新力度"
    );

    // ---- 5. 按保存：`.slint` 回调 ⇒ 默认构建那条本地落点 ----
    ui.invoke_save_project();
    let status = ui.get_save_status().to_string();
    assert!(ui.get_save_succeeded(), "生产保存入口必须成功: {status}");
    assert!(
        status.contains("本地原子写"),
        "默认构建里状态文本必须点名本地原子写: {status}"
    );

    // ---- 6. 读回容器 ----
    let reopened = container::read_project_container(
        &std::fs::read(&target).expect("读回容器"),
        &container::ContainerLimits::default(),
    )
    .expect("容器读回");

    // 判据 A：写的是**活会话**的图谱，不是装配时刻的起始图谱。
    assert_ne!(
        expected_dag, starting_dag,
        "活会话的图谱必须与起始图谱不同（编辑推进了它）"
    );
    assert_eq!(
        reopened.history_dag,
        expected_dag,
        "A：文件里的 `history.dag`（{} 字节）必须与活会话编码的图谱（{} 字节）逐字节相同",
        reopened.history_dag.len(),
        expected_dag.len()
    );

    // 判据 B：写的是**活会话**的工程。
    assert_eq!(
        velocity_of(&reopened.project, &note_id),
        Some(expected_velocity),
        "B：磁盘上那个音符的力度必须 = 活会话里的力度（{expected_velocity}），\
         而不是装配快照里的 {pristine_velocity:?}"
    );

    // ---- 7. 读的一侧：把文件里的图谱读回、重开会话，撤销链必须还活着 ----
    let graph = yeban_model::decode_history_dag(&reopened.history_dag)
        .expect("读回 `history.dag`")
        .expect("空条目才会是 None；刚写出的图谱必须非空");
    let reopened_port = UndoPort::new(
        UndoSession::open_with_graph(
            "<重开>",
            "yeban-app",
            reopened.project.clone(),
            graph,
            NOW + 2,
        )
        .expect("带图谱重开会话"),
    );
    // 重开之后游标由（文档, 图谱）推导：文档是编辑后的那一版 ⇒ 撤销可用、重做不可用。
    assert_eq!(
        reopened_port.display().undoable,
        1,
        "重开之后必须还能撤销那一步（游标由文档与图谱推导出来）"
    );
    assert!(
        !reopened_port.display().can_redo,
        "文档已经是图谱的顶端 ⇒ 没有可重做的"
    );
    // 撤销那一步 ⇒ 回到起始力度；重做 ⇒ 又回到编辑后的力度。撤销链因此**真的**活着。
    assert!(
        reopened_port.perform(UiAction::Undo).changed(),
        "重开之后的会话必须能撤销"
    );
    assert_eq!(
        velocity_of(&reopened_port.project(), &note_id),
        pristine_velocity,
        "撤销之后必须回到起始力度"
    );
    assert!(
        reopened_port.perform(UiAction::Redo).changed(),
        "重开之后的会话必须能重做"
    );
    assert_eq!(
        velocity_of(&reopened_port.project(), &note_id),
        Some(expected_velocity),
        "重做之后必须回到编辑后的力度（撤销链跨保存/重开存活）"
    );

    // ---- 落点附证：没有权威时同一个 `dispatch_save` 走的是本地原子写 ----
    let request = yeban_app::save_action::SaveRequest::with_history_dag(
        scratch.dir.join("again.yeban"),
        live_project,
        expected_dag,
    );
    let outcome = dispatch_save(&request, None);
    assert!(
        matches!(
            outcome,
            SaveOutcome::Saved {
                by_authority: false,
                ..
            }
        ),
        "没有权威时生产保存入口必须走本地原子写: {outcome:?}"
    );
}
