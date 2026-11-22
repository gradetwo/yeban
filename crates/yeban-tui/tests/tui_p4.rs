//! T-FIX-76：四方对齐 TUI 补齐第一批的判据。
//!
//! 四项各有一组"红→绿"判据，落点都在 `App` 的公开接口上（与 TUI 按键/命令行
//! 同一批函数），因此判据钉住的是**行为**，不是某个键位字符串：
//!
//! | 项 | 判据 | 钉住的事实 |
//! | :--- | :--- | :--- |
//! | 移轨 | `move_track_reorders_display_order_and_is_undoable` | 一条 `Op::MoveTrack`、显示顺序真的变了、可撤销 |
//! | 移轨拒绝 | `master_bus_and_bounds_refuse_with_loud_reasons` | 主总线/越界/原地三类拒绝与 `yeban_move_track` 同理由 |
//! | 轨道高度 | `track_height_reuses_the_single_clamp` | 每轨基准 + 全局乘子；夹紧只有 `MAX_TRACK_HEIGHT_PX` 一处 |
//! | 双视图 | `f5_f6_and_view_command_switch_main_view` | F5/F6 与 `:view` 落同一个 `View` 枚举 |
//! | 自动化 | `automation_entry_reads_and_writes_through_the_model` | 读走唯一求值入口、写一条 `Op::SetAutomationPoint`、同 tick 更新不堆积、可撤销 |
//! | 自动化值域 | `out_of_domain_automation_value_is_refused_without_writing` | 越界**响亮拒绝**且一位不写（与 `yeban_edit_automation` 同一条闸门） |

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yeban_app::input::View;
use yeban_model::project::AutomationTarget;
use yeban_tui::app::{App, AutomationKind};

fn demo_app() -> App {
    App::with_demo().expect("演示工程")
}

fn last_log(app: &App) -> String {
    app.log.last().cloned().unwrap_or_default()
}

/// 选一条**没有任何自动化泳道**的轨（演示工程的前两条轨带泳道，不能用来断言"空泳道"）。
fn select_clean_track(app: &mut App) -> yeban_model::ids::EntityId {
    let ids = app.project.display_track_ids();
    for (index, id) in ids.iter().enumerate() {
        let clean = app
            .project
            .tracks
            .get(id)
            .is_some_and(|track| track.automation_lanes.is_empty());
        if clean {
            app.selected = index;
            return *id;
        }
    }
    panic!("演示工程里应有一条没有自动化泳道的轨");
}

#[test]
fn move_track_reorders_display_order_and_is_undoable() {
    let mut app = demo_app();
    let before = app.project.display_track_ids();
    assert!(before.len() >= 2, "演示工程至少两条非主总线轨");
    app.selected = 0;
    let moved = before[0];

    app.move_selected_track(1);

    let after = app.project.display_track_ids();
    assert_eq!(after[0], before[1], "下一轨应顶到原位");
    assert_eq!(after[1], moved, "被移动的轨应落到第 2 位");
    assert_eq!(
        app.selected_track_id(),
        Some(moved),
        "选中必须跟随被移动的轨，而不是跟随下标"
    );

    app.undo(1);
    assert_eq!(
        app.project.display_track_ids(),
        before,
        "一条 Op::MoveTrack 必须整步可撤销"
    );
}

#[test]
fn master_bus_and_bounds_refuse_with_loud_reasons() {
    let mut app = demo_app();
    let master = app.project.master_bus_track_id;
    let index = app
        .track_ids
        .iter()
        .position(|id| *id == master)
        .expect("显示列表末尾是主总线");
    app.selected = index;
    let before = app.project.display_track_ids();
    app.move_selected_track(-1);
    assert_eq!(app.project.display_track_ids(), before, "主总线不可移");
    assert!(
        last_log(&app).contains("主总线"),
        "拒绝理由必须点名主总线：{}",
        last_log(&app)
    );

    app.selected = 0;
    app.move_selected_track(-1);
    assert!(last_log(&app).contains("已在首位"), "{}", last_log(&app));

    let count = app.project.display_track_ids().len();
    app.move_selected_track_to(count + 3);
    assert!(
        last_log(&app).contains("越界"),
        "越界必须报出条数：{}",
        last_log(&app)
    );
    app.move_selected_track_to(0);
    assert!(
        last_log(&app).contains("已在位置"),
        "原地移动与 yeban_move_track 的 alreadyAtTargetIndex 同一条拒绝：{}",
        last_log(&app)
    );
    assert_eq!(
        app.project.display_track_ids().len(),
        count,
        "拒绝路径不得改动工程"
    );
}

#[test]
fn track_height_reuses_the_single_clamp() {
    let mut app = demo_app();
    app.selected = 0;
    let id = app.selected_track_id().expect("有选中轨");

    assert_eq!(app.track_height_px(&id), (56, 56), "默认基准 56 px");
    assert_eq!(app.track_rows(&id), 1, "56 px = 1 行");

    app.set_track_height_px(112);
    assert_eq!(app.track_height_px(&id), (112, 112));
    assert_eq!(app.track_rows(&id), 2);

    app.set_height_percent(200);
    assert_eq!(
        app.track_height_px(&id),
        (112, 224),
        "全局乘子乘在每轨基准之后"
    );
    assert_eq!(app.track_rows(&id), 4);

    // 夹紧只有一处：基准给到 1000 px，有效值仍止于 MAX_TRACK_HEIGHT_PX。
    app.set_track_height_px(1000);
    assert_eq!(
        app.track_height_px(&id).1,
        yeban_app::bridge::MAX_TRACK_HEIGHT_PX,
        "有效值必须被 effective_track_height_px 夹住"
    );
    assert_eq!(
        app.track_rows(&id),
        u16::try_from(yeban_app::bridge::MAX_TRACK_HEIGHT_PX.div_ceil(56)).unwrap()
    );

    // 0 = 取消覆盖（与 ui/set_track_height 的 heightPx = 0 同一语义）。
    app.set_track_height_px(0);
    assert_eq!(app.track_height_px(&id), (56, 112));
    app.set_height_percent(100);
    assert_eq!(app.track_rows(&id), 1);
}

#[test]
fn f5_f6_and_view_command_switch_main_view() {
    let mut app = demo_app();
    assert_eq!(app.view(), View::Arrangement);

    app.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    assert_eq!(
        app.view(),
        View::Session,
        "F5 = Session（GUI 同一张策略表）"
    );
    app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
    assert_eq!(app.view(), View::Arrangement, "F6 = Arrangement");

    app.execute_command("view session");
    assert_eq!(app.view(), View::Session);
    app.execute_command("view arrangement");
    assert_eq!(app.view(), View::Arrangement);

    // 白名单之外的实参不许静默当成某个视图。
    app.execute_command("view matrix");
    assert_eq!(app.view(), View::Arrangement);
    assert!(last_log(&app).contains("用法：:view"), "{}", last_log(&app));
}

#[test]
fn switching_to_session_closes_grid_list_views() {
    let mut app = demo_app();
    app.selected = 0;
    app.open_automation_view(AutomationKind::Volume);
    assert!(app.automation_target().is_some());
    app.set_view(View::Session);
    assert!(
        app.automation_target().is_none(),
        "Session 视图下自动化列表视图必须关掉（不在看不见的地方吃键）"
    );
}

#[test]
fn automation_entry_reads_and_writes_through_the_model() {
    let mut app = demo_app();
    select_clean_track(&mut app);
    app.cursor_bar = 1;
    app.open_automation_view(AutomationKind::Volume);
    let target = app.automation_target().expect("泳道目标");
    assert!(matches!(target, AutomationTarget::TrackVolume { .. }));
    let tick = app.automation_cursor_tick();
    assert_eq!(tick, app.bar_ticks(), "光标在第 2 小节 = 一小节的 tick 数");

    let static_value = target.static_value(&app.project).expect("静态值");
    let readout = app.automation_readout().expect("读数");
    assert_eq!(readout.source, "static");
    assert_eq!(readout.automation_value, None);
    assert_eq!(readout.effective_value, static_value);

    // `a`：按当前生效值落一个点（一条 Op::SetAutomationPoint）。
    app.add_automation_point_at_cursor();
    let points = app.automation_points();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].tick, tick);
    assert_eq!(
        app.project
            .automation_value_at(&target, tick)
            .expect("求值"),
        Some(static_value),
        "读回必须走唯一求值入口"
    );
    assert_eq!(app.automation_readout().expect("读数").source, "automation");

    // 同 tick 改值 = 更新同一个点（不堆积，与 MCP 的"重复写入是更新"同一承诺）。
    app.set_automation_value(static_value + 3.0);
    assert_eq!(app.automation_points().len(), 1);
    assert_eq!(app.automation_points()[0].value, static_value + 3.0);

    // 一步撤销回到改值前，再一步回到没有点。
    app.undo(1);
    assert_eq!(app.automation_points()[0].value, static_value);
    app.undo(1);
    assert!(app.automation_points().is_empty());

    // Delete 路径：落点后删除。
    app.add_automation_point_at_cursor();
    assert_eq!(app.automation_points().len(), 1);
    app.remove_automation_point_at_cursor();
    assert!(app.automation_points().is_empty());
}

#[test]
fn existing_demo_lane_reads_through_the_unique_entry() {
    let mut app = demo_app();
    // 演示工程第 1 轨（鼓）有一条音量泳道：tick 0 / 1920 / 3840 = -3.2 / -8 / -1。
    app.selected = 0;
    app.cursor_bar = 0;
    app.open_automation_view(AutomationKind::Volume);
    let target = app.automation_target().expect("泳道目标");
    assert_eq!(app.automation_points().len(), 3);
    let readout = app.automation_readout().expect("读数");
    assert_eq!(readout.source, "automation");
    assert_eq!(readout.automation_value, Some(-3.2));
    assert_eq!(
        readout.automation_value,
        app.project.automation_value_at(&target, 0).expect("求值"),
        "读数必须与唯一求值入口逐位相同"
    );
}

#[test]
fn out_of_domain_automation_value_is_refused_without_writing() {
    let mut app = demo_app();
    select_clean_track(&mut app);
    app.open_automation_view(AutomationKind::Volume);
    app.set_automation_value(1000.0);
    assert!(app.automation_points().is_empty(), "越界值必须一位不写");
    assert!(
        last_log(&app).contains("越出本泳道的有效取值域"),
        "拒绝理由必须点名值域：{}",
        last_log(&app)
    );

    app.set_automation_value(-6.0);
    assert_eq!(app.automation_points().len(), 1, "边界内写得进去");

    // 声相泳道：值域 [-1, 1]，步长 0.05。
    app.open_automation_view(AutomationKind::Pan);
    app.set_automation_value(2.0);
    assert!(app.automation_points().is_empty());
    app.set_automation_value(0.0);
    assert_eq!(app.automation_points().len(), 1);
    app.nudge_automation_value(1);
    let value = app.automation_points()[0].value;
    assert!(
        (value - 0.05).abs() < f32::EPSILON * 4.0,
        "声相一档 = 0.05，实际 {value}"
    );
}

#[test]
fn documented_keys_reach_their_handlers() {
    let mut app = demo_app();
    select_clean_track(&mut app);

    // 裸 `P` = 声相泳道（实测共享表里 `P` 是 PassThrough：GUI 只认小写 `p`/`e`）。
    app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT));
    assert!(
        matches!(
            app.automation_target(),
            Some(AutomationTarget::TrackPan { .. })
        ),
        "P 必须打开声相自动化视图"
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.automation_target().is_none(), "Esc 必须关掉视图");

    // GUI `Ctrl+Shift+A`（ToggleAutomation）→ 音量泳道开关。
    let toggle = || {
        KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )
    };
    app.handle_key(toggle());
    assert!(matches!(
        app.automation_target(),
        Some(AutomationTarget::TrackVolume { .. })
    ));
    app.handle_key(toggle());
    assert!(app.automation_target().is_none(), "再按一次关掉");

    // `{` / `}` = 全局高度乘子（裸 `[`/`]` 是 GUI 试听，不许抢）。
    let before = app.height_percent();
    app.handle_key(KeyEvent::new(KeyCode::Char('}'), KeyModifiers::SHIFT));
    assert_eq!(app.height_percent(), before + 10);
    app.handle_key(KeyEvent::new(KeyCode::Char('{'), KeyModifiers::SHIFT));
    assert_eq!(app.height_percent(), before);

    // 裸 `J` = 把选中轨往下移一格。
    let order = app.project.display_track_ids();
    app.selected = 0;
    let moved = order[0];
    app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT));
    assert_eq!(app.project.display_track_ids()[1], moved);
}
