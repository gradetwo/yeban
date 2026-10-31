//! 判据：`Alt+↑` / `Alt+↓` 从**真事件源**落到**既有**的选轨回调（`host::wire_track_select`）。
//!
//! 本切片补的缺口：**选中轨此前只能靠鼠标点轨道行** —— 键盘-only 用户没有任何办法
//! 换轨。`[UI-NOTE-005]` 只讲音符（⛔ 没点名方向键选轨）⇒ 键位是**界面决定**；
//! `Alt+↑/↓` 此前落在 `resolve_logical` 第 4 步的 `_ => PassThrough`，邻居一格不动。
//!
//! ## 为什么两层（不可互相代替）
//!
//! | 层 | 位置 | 证的事 |
//! | :--- | :--- | :--- |
//! | A 纯解析 | `src/input.rs` 的 `alt_up_and_down_move_the_track_selection_and_do_not_disturb_its_neighbours` | 物理码 / 逻辑键两条入口都解出 `SelectTrackBy(±1)`；裸键 / `Shift` / `Cmd+Alt` / `Alt+←→` 保持原义 |
//! | B 活窗口（本文件） | `build_live_ui_with` 装**生产同一个** `host::wire_keys` ＋ 生产同一个 `host::wire_track_select` | 真按键 ⇒ `selected-track-id` 真的动；落点就是那**一个**回调（换计数器后恰好一次）；边界与空工程的行为如实 |
//!
//! ## 判据表
//!
//! | # | 判据 | 断言 |
//! | :--- | :--- | :--- |
//! | B1 | `alt_up_down_walk_the_track_selection_and_stop_at_the_ends` | 无选中按 `Alt+↓` ⇒ 第一条；再按 ⇒ 第二条；到末条再按 ⇒ **仍是末条**且**仍消费**；`Alt+↑` 反向同理；全程 `commit_count` / `undoable` 一位不动（选中态是视图态，⛔ 不进撤销栈） |
//! | B2 | `the_track_keys_invoke_the_registered_track_select_callback_with_the_neighbour_index` | 把 `on_track_select` 换成记录器 ⇒ 报出的下标 = **身份表里的邻居下标**，两端**夹紧**（末条再按下发末下标、首条再按上发 `0`）；证明键打到的是回调本身，⛔ 不是旁路写属性 |
//! | B3 | `neighbour_combinations_leave_the_selection_alone` | `Alt+Shift+↓` 未绑定 ⇒ **不消费**；`Cmd/Ctrl+Alt+↓`（音量微调）**消费**但选中态不动 |
//! | B4 | `without_any_track_the_track_keys_are_not_consumed` | 只有主总线（`track-ids` 为空）⇒ 两向都 `false`（⛔ 不吞键），选中态保持空串 |
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | `SelectTrackBy` 臂改成写 `selected-track-id` 属性（绕过回调） | B2 红（记录器收不到下标） |
//! | 删掉 `clamp`（两端越界） | B2 红（末条再按发 `3`、首条再按发 `-1`，记录器读得到） |
//! | 空工程仍返回 `true` | B4 红（吞键） |
//! | 删掉 `resolve_logical` 第 4 步的两个 `UpArrow` / `DownArrow` 臂 | 本文件全红（键不消费）＋ `src/input.rs` 那条红 |
//! | `cli.rs` 把某一行的 `direction` 写反 | `tests/cli_contract.rs` 的 B11b ②红（标签绑的键解析出的动作 ≠ 行声称的动作） |

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `record_key_ui.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::host;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::YebanProjectV1;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;

/// 会话打开时刻（与其余界面判据同一个常量，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 逻辑键入口的两个方向（Slint 的私有码点，与 `input.rs` 的 `LogicalKey::from_text` 同源）。
const UP: &str = "\u{f700}";
const DOWN: &str = "\u{f701}";

/// 一条**接了键盘与选轨**的活窗口（两者都走生产同一个接线函数）。
///
/// `host::wire_keys` 由 `live_surface.rs` 的装配负责（与 `main.rs` 同一个函数）；
/// `host::wire_track_select` 由本判据补上（`live_surface.rs` 不接它，与
/// `tests/track_selection.rs` / `tests/mixer_keyboard_nudge.rs` 同一做法），
/// 因此 `selected-track-id` 的写者仍然只有生产那一个。
fn live_with_keys_and_select(project: &YebanProjectV1) -> (MainWindow, Rc<UndoPort>, live::LiveUi) {
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据 选轨键盘>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let ui = live::build_live_ui_with(
        project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 端口");
    // `clone_strong` 是 Slint 句柄的**共享**（`Rc` 语义）而非复制 ⇒ 与 `ui` 里
    // 那个活窗口是同一个组件实例；`ui` 必须留在判据作用域里把组件撑着。
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    host::wire_track_select(&window, &port);
    (window, port, ui)
}

/// `track-ids`（非主总线轨道的身份表，与 `track-names` 同索引集）的整列读数。
///
/// 判据**不**自己猜顺序或身份：一律读窗口上生产注入的那一列（⛔ 不拿夹具快照对账，
/// 否则"宿主查表写错"与"判据抄错"分不开）。
fn track_ids(window: &MainWindow) -> Vec<String> {
    use slint::Model as _;
    window
        .get_track_ids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// 当前选中轨（`selected-track-id` 的原文；没有选中就是空串）。
fn selected(window: &MainWindow) -> String {
    window.get_selected_track_id().to_string()
}

/// 只留主总线的工程（`track-ids` 为空的那种边界）。
fn master_only(mut project: YebanProjectV1) -> YebanProjectV1 {
    let master = project.master_bus_track_id;
    project.tracks.retain(|_, track| track.id == master);
    project
}

/// B1：`Alt+↓` / `Alt+↑` 逐条走完选中态，两端**停住但仍消费**，且不产生任何提交。
#[test]
fn alt_up_down_walk_the_track_selection_and_stop_at_the_ends() {
    let project = filled_project();
    let (window, port, _ui) = live_with_keys_and_select(&project);

    let ids = track_ids(&window);
    assert_eq!(
        ids.len(),
        3,
        "夹具的非主总线轨数（改了 `filled_project()` 的轨数时**这里先红**）: {ids:?}"
    );
    assert!(
        ids.iter().all(|id| !id.is_empty()),
        "身份表不得有空洞: {ids:?}"
    );
    assert_eq!(
        selected(&window),
        "",
        "起点：投影不写选中态（`.slint` 初值）"
    );

    // 选中态是**视图态**：起点口径与撤销栈无关，但记下来做"一位不动"的对照。
    let commits_at_start = port.display().commit_count;
    assert_eq!(port.display().undoable, 0);

    // ① 没有选中时按 `↓` ⇒ 落到**第一条**（键盘-only 用户必须能起步）。
    assert!(
        window.invoke_key_action(DOWN.into(), false, false, true, false),
        "`Alt+↓` 必须被消费"
    );
    assert_eq!(selected(&window), ids[0], "无选中 ⇒ 第一条");

    // ② 再按 ⇒ 下一条；再按 ⇒ 末条。
    for expected in [&ids[1], &ids[2]] {
        assert!(window.invoke_key_action(DOWN.into(), false, false, true, false));
        assert_eq!(&selected(&window), expected);
    }

    // ③ 到末条再按 ⇒ **仍是末条**，且**仍消费**（不把键漏给焦点系统）。
    assert!(
        window.invoke_key_action(DOWN.into(), false, false, true, false),
        "到末条再按也必须消费"
    );
    assert_eq!(selected(&window), ids[2], "末条停住");

    // ④ 反向：上一条、再上一条、顶住。
    assert!(window.invoke_key_action(UP.into(), false, false, true, false));
    assert_eq!(selected(&window), ids[1]);
    assert!(window.invoke_key_action(UP.into(), false, false, true, false));
    assert_eq!(selected(&window), ids[0]);
    assert!(
        window.invoke_key_action(UP.into(), false, false, true, false),
        "到第一条再按也必须消费"
    );
    assert_eq!(selected(&window), ids[0], "第一条停住");

    // ⑤ 全程是视图态：工程与撤销栈一位不动。
    assert_eq!(
        port.display().commit_count,
        commits_at_start,
        "选轨 ⛔ 不产生提交"
    );
    assert_eq!(port.display().undoable, 0, "选轨 ⛔ 进撤销栈");
}

/// B2：键打到的**就是** `on_track_select` 这个回调 —— 换成记录器后，报出的下标是
/// 身份表里的**邻居下标**且两端**夹紧**（`wire_track_select` 的查表口径），
/// ⛔ 不是旁路写属性、也不把越界下标漏给回调。
#[test]
fn the_track_keys_invoke_the_registered_track_select_callback_with_the_neighbour_index() {
    let project = filled_project();
    let (window, _port, _ui) = live_with_keys_and_select(&project);
    let ids = track_ids(&window);

    // 先用生产的点击源选中第二条，再用生产的键走到末条（这一步证明真事件源会改选中态）。
    window.invoke_track_select(1);
    assert_eq!(selected(&window), ids[1]);
    assert!(window.invoke_key_action(DOWN.into(), false, false, true, false));
    assert_eq!(selected(&window), ids[2], "`Alt+↓` 走到末条");

    // 换成记录器。之后选中态不再移动（生产回调已被替换），于是每次按键都从**末条**
    // 这个起点计算下标 ⇒ "夹紧"与"邻居"都能被逐次读出。
    let calls = Rc::new(RefCell::new(Vec::<i32>::new()));
    let calls_for_callback = Rc::clone(&calls);
    window.on_track_select(move |index| {
        calls_for_callback.borrow_mut().push(index);
    });

    for _ in 0..2 {
        assert!(
            window.invoke_key_action(DOWN.into(), false, false, true, false),
            "`Alt+↓` 必须被消费"
        );
    }
    for _ in 0..2 {
        assert!(
            window.invoke_key_action(UP.into(), false, false, true, false),
            "`Alt+↑` 必须被消费"
        );
    }
    assert_eq!(
        *calls.borrow(),
        vec![2, 2, 1, 0],
        "起点是末条：`Alt+↓` 夹紧在末下标（⛔ 不是 `3`），`Alt+↑` 是邻居，再上夹紧在 `0`（⛔ 不是 `-1`）"
    );
}

/// B3：邻居组合 —— `Alt+Shift+↓` 未绑定 ⇒ 放行；`Cmd/Ctrl+Alt+↓` 是**音量微调**
/// ⇒ 消费但选中态不动（两族键各走各的臂）。
#[test]
fn neighbour_combinations_leave_the_selection_alone() {
    let project = filled_project();
    let (window, _port, _ui) = live_with_keys_and_select(&project);
    let ids = track_ids(&window);

    window.invoke_track_select(0);
    assert_eq!(selected(&window), ids[0]);

    // ① `Alt+Shift+↓` 不在策略表里 ⇒ 如实放行。
    assert!(
        !window.invoke_key_action(DOWN.into(), true, false, true, false),
        "`Alt+Shift+↓` 未绑定 ⇒ 不得消费"
    );
    assert_eq!(selected(&window), ids[0], "放行的键不得改选中态");

    // ② `Cmd/Ctrl+Alt+↓` = 选中轨的音量微调：消费，但选中态一格不动。
    assert!(
        window.invoke_key_action(DOWN.into(), false, true, true, false),
        "`Cmd/Ctrl+Alt+↓` 走音量微调臂 ⇒ 消费"
    );
    assert_eq!(selected(&window), ids[0], "音量微调不得改选中态");
}

/// B4：只有主总线（`track-ids` 为空）⇒ 两向都**不消费**（⛔ 不吞键），选中态保持空串。
#[test]
fn without_any_track_the_track_keys_are_not_consumed() {
    let project = master_only(filled_project());
    let (window, _port, _ui) = live_with_keys_and_select(&project);

    assert!(track_ids(&window).is_empty(), "夹具已经被裁到只剩主总线");
    assert_eq!(selected(&window), "");

    for key in [DOWN, UP] {
        assert!(
            !window.invoke_key_action(key.into(), false, false, true, false),
            "没有可选的轨道 ⇒ `Alt+方向` 必须放行（⛔ 不假装成功）"
        );
    }
    assert_eq!(selected(&window), "", "没有可选的轨道 ⇒ 选中态不编造");
}
