//! **自动化泳道的「当前值」跟随播放头** —— 端到端判据（[UI-A11Y-001] / [UI-TEST-001]）。
//!
//! ## 本文件要证的句子
//!
//! 自动化泳道的无障碍标签里那个"当前值"**不是**工程起点的静态快照：播放推进 / 定位之后，
//! [`host::ProductionLoop::tick`] 按引擎的 `TransportReading::position_ticks` 重算标签，
//! 经**唯一注入点** `.slint` 的 `automation-lane-labels` 落到控件树的 `accessible-label`。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（装配态） | 窗口标签 == 投影在 tick 0 的标签 == 控件树里那条泳道的 `accessible-label`；引擎的工程缓存**已种上**（开局没有编辑也能算） |
//! | ②（真播放 → 真标签） | `play` + 推量子 + 一跳 ⇒ 标签 == 工程在**引擎当前** `position_ticks` 处的独立投影，且音量泳道标签与装配态**不同** |
//! | ③（位置没变 ⇒ 不动） | 位置不变的下一跳标签逐字不变 |
//! | ④（定位） | `SeekTicks` 之后标签跟到**引擎实际读数**（⛔ 不是命令里的目标 tick，⛔ 不是上一跳的陈旧值） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/playhead_follows_transport.rs` 同款）。

use std::collections::BTreeMap;
use std::str::FromStr as _;

use slint::Model as _;
use yeban_app::automation::labels_at_cursor;
use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EditMark;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_engine::ring::TransportCommand;
use yeban_model::ids::EntityId;
use yeban_model::music::CurveType;
use yeban_model::project::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, YebanProjectV1,
};
use yeban_ui_test_port::LivePort;
use yeban_ui_test_port::image::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

fn tid(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 合法")
}

/// 一条音量泳道的工程夹具：tick 0 ⇒ −60 dB、tick 3840 ⇒ +12 dB（线性）。
///
/// 斜率足够陡（约 0.019 dB/tick）⇒ 位置一小步就能在标签的十进制文本里分辨出来。
fn fixture_project() -> (YebanProjectV1, AutomationTarget) {
    let mut project = yeban_app::bridge::demo_project();
    let track_id = *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("演示工程必须有非主总线轨道");
    let volume = AutomationTarget::TrackVolume { track_id };
    let track = project.tracks.get_mut(&track_id).expect("轨道存在");
    track.automation_lanes.clear();
    let mut points: BTreeMap<EntityId, AutomationPoint> = BTreeMap::new();
    for (tail, tick, value) in [("Z1", 0_u64, -60.0_f32), ("Z2", 3840, 12.0)] {
        let point_id = tid(tail);
        points.insert(
            point_id,
            AutomationPoint {
                id: point_id,
                tick,
                value,
                curve: CurveType::Linear,
            },
        );
    }
    track.automation_lanes.insert(
        volume,
        AutomationLane {
            target: volume,
            points,
            read_enabled: true,
            write_mode: AutomationWriteMode::Off,
            domain: None,
        },
    );
    project.validate().expect("夹具工程必须合法");
    (project, volume)
}

/// 窗口上 `automation-lane-labels` 的当前值（生产唯一注入点写的那个属性）。
fn ui_labels(ui: &yeban_app::ui::MainWindow) -> Vec<String> {
    ui.get_automation_lane_labels()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

/// 控件树里一条泳道节点的 `accessible-label`。
fn lane_node_label(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) -> String {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到泳道 `{id}`"))
        .label
        .clone()
}

#[test]
fn automation_lane_values_follow_the_transport_playhead() {
    let (project, volume) = fixture_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let mut port = LivePort::new(size, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    let volume_lane = view
        .automation_lanes
        .iter()
        .find(|lane| lane.target == volume)
        .expect("投影里必须有那条音量泳道")
        .clone();
    let lane_id = volume_lane.element_id.clone();

    // 与 `src/main.rs` 同一段装配：建引擎 → 记开局标记 → 显式 `Stop` → 回写一次读数。
    let mut production = ProductionLoop::start(&project, 0).expect("引擎重建");
    assert!(
        production
            .engine_handle()
            .borrow()
            .automation_project()
            .is_some(),
        "装配点必须把工程种进引擎缓存（否则「标记没变」的跳没有工程可算）"
    );
    production
        .engine_handle()
        .borrow_mut()
        .mark_applied(EditMark::new(None, 0));
    production.engine_handle().borrow_mut().stop();
    host::apply_transport(port.ui(), production.engine_handle().borrow().transport());

    // ---- ① 装配态：tick 0 的标签（投影 = 窗口 = 控件树） ----
    let rest_labels = labels_at_cursor(&project, &view.automation_lanes, 0);
    assert_eq!(
        ui_labels(port.ui()),
        rest_labels,
        "装配态窗口标签必须等于工程在 tick 0 的投影"
    );
    assert_eq!(
        ui_labels(port.ui()),
        view.automation_lane_labels(),
        "装配态的标签必须与静态投影逐字相同（默认帧不变的由来）"
    );
    assert_eq!(
        lane_node_label(&mut port, &lane_id),
        rest_labels
            .iter()
            .zip(&view.automation_lanes)
            .find(|(_, lane)| lane.element_id == lane_id)
            .map(|(label, _)| label.clone())
            .expect("音量泳道必须有一份标签"),
        "控件树里 `{lane_id}` 的 accessible-label 必须是投影写下的那一份"
    );

    // ---- ② 真播放 ⇒ 标签跟着引擎位置走 ----
    production.engine_handle().borrow_mut().play();
    production.engine_handle().borrow_mut().drive_audio(32);
    let tick = production.tick(port.ui(), &view, EditMark::new(None, 0), None);
    assert!(tick.readings.is_some(), "这一跳必须真的走到引擎心跳");
    let position = production
        .engine_handle()
        .borrow()
        .transport()
        .position_ticks;
    assert!(position > 0, "夹具前提：播放推进后位置必须大于 0");

    let expected = labels_at_cursor(&project, &view.automation_lanes, position);
    let expected_volume = view
        .automation_lanes
        .iter()
        .position(|lane| lane.element_id == lane_id)
        .map(|index| expected[index].clone())
        .expect("音量泳道必须在同一序位上");
    let rest_volume = rest_labels
        .iter()
        .zip(&view.automation_lanes)
        .find(|(_, lane)| lane.element_id == lane_id)
        .map(|(label, _)| label.clone())
        .expect("音量泳道必须有一份标签");
    assert_ne!(
        expected_volume, rest_volume,
        "夹具前提：位置 {position} 的当前值必须与 tick 0 不同（否则本条判据区分不出「标签从不刷新」）"
    );
    assert_eq!(
        ui_labels(port.ui()),
        expected,
        "播放后窗口标签必须等于工程在引擎当前位置 {position} 的投影"
    );
    assert_eq!(
        lane_node_label(&mut port, &lane_id),
        expected_volume,
        "控件树里的 accessible-label 必须也跟着新位置更新"
    );
    report_line(&format!(
        "[automation-cursor] position={position} lane={lane_id} label={expected_volume}"
    ));

    // ---- ③ 位置不变的下一跳：标签逐字不动 ----
    let unchanged = production.tick(port.ui(), &view, EditMark::new(None, 0), None);
    assert!(unchanged.readings.is_some());
    assert_eq!(
        production
            .engine_handle()
            .borrow()
            .transport()
            .position_ticks,
        position,
        "夹具前提：没有推量子 ⇒ 位置不变"
    );
    assert_eq!(
        ui_labels(port.ui()),
        expected,
        "位置没变的跳不得改写标签数组"
    );

    // ---- ④ 定位：标签跟**引擎实际读数**，不跟命令里的目标 tick ----
    let sought = 1920_u64;
    production
        .engine_handle()
        .borrow_mut()
        .send_transport(&[TransportCommand::SeekTicks(sought)]);
    production.engine_handle().borrow_mut().drive_audio(1);
    let after_seek = production.tick(port.ui(), &view, EditMark::new(None, 0), None);
    assert!(after_seek.readings.is_some());
    let actual = production
        .engine_handle()
        .borrow()
        .transport()
        .position_ticks;
    assert_ne!(actual, position, "夹具前提：定位必须真的移动了引擎位置");
    let expected_after_seek = labels_at_cursor(&project, &view.automation_lanes, actual);
    assert_eq!(
        ui_labels(port.ui()),
        expected_after_seek,
        "定位后窗口标签必须等于工程在引擎实际位置 {actual} 的投影（⛔ 命令里的 {sought}）"
    );
}
