//! 整工程克隆残留：保存快照 + 撤销会话与打开者共享同一份工程（T-ISS-MWW4H8J6）。
//!
//! ## 缺陷本身（T-FIX-428 回执 §5 遗留）
//!
//! T-FIX-428 消灭了打开路径上的整工程克隆，但同一类克隆还剩两处：
//!
//! 1. `host::SaveStatus.project: YebanProjectV1` —— 装配时刻的一份整工程快照；
//!    没有活会话时 `dispatch_save` 直接用它写盘。
//! 2. `undo::session_from_archive` 里 `archive.project.clone()` —— 一次进
//!    `UndoSession::open`、一次进 `open_with_graph`。
//!
//! ## 修法（本单）
//!
//! 两处都改成共享同一份 `Rc<RefCell<YebanProjectV1>>`：
//!
//! * `UndoSession.project` 是共享句柄；`open_undo_session` 从归档里**搬**
//!   （`std::mem::take`）进这份句柄，⛔ 不再克隆；`session_from_archive`
//!   接手柄 + `history_dag`，不再接整份归档。
//! * `SaveStatus.project` 是同一份句柄（装配时从会话取，换工程时从端口取）；
//!   没有活会话时保存读的是**此刻**的工程，⛔ 不再是装配时刻的快照
//!   （有意的语义变化，见 `host::SaveStatus` 的文档）。
//!
//! ## 四组判据（④ 是 T-FIX-457 的三条）
//!
//! | # | 断言 | 红法（一变量回退） |
//! | - | ---- | ---------------- |
//! | ① | `session_from_archive` 共享打开者那一份工程（`Rc::ptr_eq` + `strong_count == 2`）；打开后改工程，会话当场看到；会话指纹跟着变 | 把 `session_from_archive` 改回内部克隆（`Rc::new(RefCell::new(project.borrow().clone()))`）⇒ ① 全红 |
//! | ② | 没有活会话时按保存，写出的是**装配之后才改的**那一版（读回容器验轨道名）；共享句柄经 `host::shared_project_for_session`（就是 `make_after` 用的那条接线） | 把 helper 改成快照（`Rc::new(RefCell::new(fallback_project.clone()))`）⇒ ② 红 |
//! | ③ | `open_undo_session` 把归档里的工程**搬**进会话（指纹一致；归档里只剩默认空工程） | 把 `open_undo_session` 改回克隆 ⇒ ③ 的"归档被搬空"红 |
//! | ④a | `SaveRequest::new_shared` 存的就是调用方那一份句柄（`Rc::ptr_eq` + `strong_count`） | 把 `new_shared` 改成构造时克隆（`ProjectSource::Owned(project.borrow().clone())`）⇒ ④a 红（`project_shared()` 返回 `None`） |
//! | ④b | 经共享请求 `dispatch_save` 写出的是**构造之后才改的**那一版（读回容器验轨道名）；仍如实写空 `history.dag` / 空资产池 | 同上 ⇒ ④b 写出旧名字而红 |
//! | ④c | 同一份工程经共享请求与经自有请求写出的容器**逐字节相同**（借用形态与旧归档形态是同一条写出路） | 同上 ⇒ ④c 仍绿（它只证两条写出路等价，不证共享）—— 故 ④a/④b 才是共享的红证明 |
//!
//! ⚠ ①/②/③ 的红法都是"改回旧形状" —— 旧形状在基线 `5a5bbce0` 上就是
//! 红的（新判据在旧代码上编不过：`SaveStatus.project` 是值类型、
//! `session_from_archive` 签名不同），所以"红"在这里是**构造性**的：
//! 回退 diff 即红，恢复即绿。判据数的是**共享**，不是"某个函数被调用过"。
//!
//! ⚠ ④a/④b 的红法是同一处一变量回退（`new_shared` 改成构造时克隆）：
//! 它在基线 `a37b498b` 上也是构造性红（`new_shared` 不存在，判据编不过）。
//! ④c 在该回退下仍绿 —— 它只证明两条写出路等价，不证明共享，⛔ 不能单独
//! 拿它当"无克隆"的证据。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::cli;
use yeban_app::host;
use yeban_app::save_action::{SaveRequest, dispatch_save};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{
    UndoPort, UndoSession, open_undo_session, project_fingerprint, session_from_archive,
};
use yeban_model::YebanProjectV1;
use yeban_model::container::ProjectArchive;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

/// 撤销会话的"现在"（判据不需要墙钟，与同类判据同款常量）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 判据 ①：`session_from_archive` 共享打开者那一份工程，⛔ 不复制。
#[test]
fn session_from_archive_shares_the_archive_project_without_copying() {
    let project = filled_project();
    let track_id = *project.tracks.keys().next().expect("filled 工程必须有轨道");
    let original_name = project.tracks[&track_id].name.clone();
    let shared = Rc::new(RefCell::new(project));

    let session =
        session_from_archive("test", Rc::clone(&shared), &[], NOW_MS).expect("会话必须能打开");
    // 同一份分配（`ptr_eq`），不是内容相同的第二份。
    assert!(
        Rc::ptr_eq(&shared, &session.project_shared()),
        "会话必须共享打开者那一份工程（ptr_eq），而不是复制一份"
    );
    // 打开者 + 会话 = 2 个句柄；多一个就是多复制了一份。
    assert_eq!(Rc::strong_count(&shared), 2, "打开者 + 会话 = 2 个句柄");

    // 打开后改工程：会话必须当场看到（同一份文档），指纹也跟着变。
    let fingerprint_before = session.fingerprint().expect("指纹");
    shared.borrow_mut().tracks.get_mut(&track_id).unwrap().name = "改名之后".to_owned();
    assert_eq!(
        session.project().tracks[&track_id].name,
        "改名之后",
        "打开者改了工程，会话必须当场看到新名字"
    );
    assert_ne!(
        session.fingerprint().expect("指纹"),
        fingerprint_before,
        "会话指纹必须跟着共享工程一起变（否则会话读的是冻结的副本）"
    );
    assert_ne!(
        session.project().tracks[&track_id].name,
        original_name,
        "新名字必须真的不同于旧名字（否则这条判据没测到东西）"
    );
}

/// 判据 ②：没有活会话时按保存，写出的是**此刻**共享的工程，⛔ 不是装配时刻的快照。
/// 共享句柄经 `host::shared_project_for_session` 拿（就是 `make_after` 用的那条接线）。
#[test]
fn save_status_fallback_writes_the_shared_live_project_not_the_assembly_snapshot() {
    let project = filled_project();
    let track_id = *project.tracks.keys().next().expect("filled 工程必须有轨道");
    // 活会话持有工程（`project.clone()` 是判据自己的装配代价，不是产品克隆）。
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<save-shared>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    // 保存回退拿会话的共享句柄（与 `make_after` 同一条接线）。
    let shared = host::shared_project_for_session(&port, &project);

    let scratch = std::env::temp_dir().join(format!("yeban-save-shared-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("建临时目录");
    let path = scratch.join("shared.yeban");

    let view = ViewState::from_project(&shared.borrow()).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // 装配：保存回退拿共享句柄，⛔ 没有活会话（`live_port = None`）。
    host::wire_save_with_engine(
        ui,
        host::SaveStatus {
            target: Some(path.clone()),
            project: Rc::clone(&shared),
        },
        None,
        Rc::new(yeban_app::file_picker::NoPicker),
        None,
        None,
    );

    // 装配**之后**才改工程——经会话句柄改（活文档）：快照形状会在这里写出旧名字，共享形状写出新名字。
    port.project_shared()
        .expect("端口有会话")
        .borrow_mut()
        .tracks
        .get_mut(&track_id)
        .unwrap()
        .name = "保存时才改的名字".to_owned();
    ui.invoke_save_project();

    let bytes = std::fs::read(&path).expect("保存必须写出文件");
    let archive = yeban_model::container::read_project_container(&bytes, &Default::default())
        .expect("写出的必须是可读容器");
    assert_eq!(
        archive.project.tracks[&track_id].name, "保存时才改的名字",
        "没有活会话时保存必须写此刻共享的工程，⛔ 不是装配时刻的快照"
    );

    std::fs::remove_dir_all(&scratch).ok();
}

/// 判据 ③：`open_undo_session` 把归档里的工程**搬**进会话（`mem::take`），⛔ 不克隆。
#[test]
fn open_undo_session_moves_the_archive_project_into_the_session() {
    let project = filled_project();
    let fingerprint = project_fingerprint(&project).expect("指纹");
    let mut loaded = cli::Loaded {
        source: cli::ProjectSource::File {
            path: PathBuf::from("test.yeban"),
            bytes: 0,
        },
        archive: ProjectArchive {
            project,
            history_dag: Vec::new(),
            proposals: None,
            assets: Vec::new(),
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        },
    };

    let session = open_undo_session("test", &mut loaded, NOW_MS).expect("会话必须能打开");
    // 会话里的工程与归档里的同一份内容（指纹逐字节一致）。
    assert_eq!(
        session.fingerprint().expect("指纹"),
        fingerprint,
        "会话里的工程必须与归档里的同一份内容"
    );
    // 归档里的工程被搬空了 —— 留下一份复制品就不是 move。
    assert_eq!(
        loaded.archive.project,
        YebanProjectV1::default(),
        "归档里的工程必须被搬空（move），而不是留下一份复制品"
    );
}

/// 判据 ④a：`SaveRequest::new_shared` 存的就是调用方那一份句柄，⛔ 不复制。
#[test]
fn save_request_new_shared_stores_the_same_handle_without_copying() {
    let project = filled_project();
    let shared = Rc::new(RefCell::new(project));
    let request = SaveRequest::new_shared(PathBuf::from("shared.yeban"), Rc::clone(&shared));

    let stored = request.project_shared().expect("共享形态必须能拿回句柄");
    assert!(
        Rc::ptr_eq(&stored, &shared),
        "请求必须存调用方那一份句柄（ptr_eq），而不是复制一份"
    );
    assert_eq!(
        Rc::strong_count(&shared),
        3,
        "调用方 + 请求 + 这次拿回 = 3 个句柄"
    );
    drop(stored);
    assert_eq!(
        Rc::strong_count(&shared),
        2,
        "调用方 + 请求 = 2 个句柄；多一个就是多复制了一份"
    );
}

/// 判据 ④b：经共享请求 `dispatch_save` 写出的是**构造之后才改的**那一版 ——
/// 证明写出发生在落盘时刻、且读的是共享句柄（构造时克隆会在这里写出旧名字）。
/// 仍如实写空 `history.dag` / 空资产池（与旧回退同一语义）。
#[test]
fn dispatch_save_through_a_shared_request_writes_the_live_project() {
    let project = filled_project();
    let track_id = *project.tracks.keys().next().expect("filled 工程必须有轨道");
    let shared = Rc::new(RefCell::new(project));

    let scratch = std::env::temp_dir().join(format!("yeban-save-live-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("建临时目录");
    let path = scratch.join("live.yeban");

    let request = SaveRequest::new_shared(path.clone(), Rc::clone(&shared));
    // 构造之后才改工程 —— 构造时克隆的形状会在这里写出旧名字。
    shared.borrow_mut().tracks.get_mut(&track_id).unwrap().name = "构造之后才改的名字".to_owned();

    let outcome = dispatch_save(&request, None);
    assert!(outcome.succeeded(), "共享请求保存必须成功：{outcome:?}");

    let bytes = std::fs::read(&path).expect("保存必须写出文件");
    let archive = yeban_model::container::read_project_container(&bytes, &Default::default())
        .expect("写出的必须是可读容器");
    assert_eq!(
        archive.project.tracks[&track_id].name, "构造之后才改的名字",
        "共享请求保存必须写构造之后才改的那一版，⛔ 不是构造时刻的快照"
    );
    assert!(
        archive.history_dag.is_empty(),
        "共享回退仍如实写空 history.dag"
    );
    assert!(archive.assets.is_empty(), "共享回退仍如实写空资产池");

    std::fs::remove_dir_all(&scratch).ok();
}

/// 判据 ④c：同一份工程经共享请求与经自有请求写出的容器**逐字节相同** ——
/// 证明借用形态的写出入口与旧归档入口是同一条路（没有第二份序列化）。
/// 容器写出是确定性的（固定 DOS 时间戳 `1980-01-01`、资产按哈希升序），
/// 因此逐字节比较是合法的等价断言。
#[test]
fn shared_and_owned_requests_write_byte_identical_containers() {
    let project = filled_project();
    let shared = Rc::new(RefCell::new(project.clone()));

    let scratch = std::env::temp_dir().join(format!("yeban-save-identical-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("建临时目录");
    let shared_path = scratch.join("shared.yeban");
    let owned_path = scratch.join("owned.yeban");

    let shared_request = SaveRequest::new_shared(shared_path.clone(), Rc::clone(&shared));
    let owned_request = SaveRequest::new(owned_path.clone(), project);
    let shared_outcome = dispatch_save(&shared_request, None);
    let owned_outcome = dispatch_save(&owned_request, None);
    assert!(
        shared_outcome.succeeded(),
        "共享请求保存必须成功：{shared_outcome:?}"
    );
    assert!(
        owned_outcome.succeeded(),
        "自有请求保存必须成功：{owned_outcome:?}"
    );

    let shared_bytes = std::fs::read(&shared_path).expect("共享请求必须写出文件");
    let owned_bytes = std::fs::read(&owned_path).expect("自有请求必须写出文件");
    assert_eq!(
        shared_bytes, owned_bytes,
        "共享请求与自有请求写出的容器必须逐字节相同"
    );

    std::fs::remove_dir_all(&scratch).ok();
}
