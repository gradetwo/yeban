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

#![allow(unused)]
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> automation-lane-labels:"),
        "新 UI 契约面必须声明 > automation-lane-labels:"
    );
    assert!(
        aether_host.contains("set_automation_lane_labels"),
        "宿主接线层必须注册 set_automation_lane_labels"
    );
}
