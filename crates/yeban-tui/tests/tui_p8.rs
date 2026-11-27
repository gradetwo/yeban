//! T-FIX-97：TUI 自动化补齐其余四类目标（批 5）的判据。
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | `all_six_targets_create_update_and_delete_points` | 六类目标各一条：可建泳道、可加点、可改点（同 tick 同身份）、可删点（空泳道随点消失） |
//! | `the_lane_list_shows_gui_names_and_opens_by_index` | 清单用 GUI 同一命名口径（设备参数显示参数名、宏显示宏名、发送显示边 ULID、CC 显示控制器号），序号能打开对应泳道 |
//! | `missing_targets_are_refused_loudly_and_build_no_lane` | 设备槽/参数序/宏号越界、路由边不存在、CC 越界 ⇒ 响亮报错且不建泳道 |
//! | `midi_cc_domain_is_the_seven_bit_controller_range` | CC 的有效值域来自模型（0..=127），越界写入被拒 |
//! | `device_params_keep_the_unknown_domain_honest` | 设备参数值域模型不声明 ⇒ TUI 照实说"值域未知"，不夹紧、不编造 |
//! | `six_lanes_survive_a_save_and_reopen` | 六类泳道的点经 `save_to` / `open_file` 往返后逐点一致 |

use std::path::{Path, PathBuf};

use yeban_model::ids::EntityId;
use yeban_model::project::{
    AutomationTarget, ClipContent, DeviceDefinition, DeviceKind, MacroParameter, ParameterValue,
};
use yeban_tui::app::{App, AutomationKind};

/// 夹具：演示工程 + 一条带两个参数的设备 + 一个宏；返回 `(app, 轨道, 一条真实路由边)`。
fn fixture() -> (App, EntityId, EntityId) {
    let mut project = yeban_model::samples::demo_project();
    // T-FIX-799：本夹具走 `with_project`（空归档），演示工程的音频片段引用
    // 没有对应资产字节 ⇒ 保存会被资产门拒绝。自动化判据不需要音频，先摘掉
    // 音频片段与它的摆放，避免一个与自动化无关的资产门把套件染红。
    let audio_ids: Vec<EntityId> = project
        .clip_pool
        .iter()
        .filter(|(_, clip)| matches!(clip.content, ClipContent::Audio { .. }))
        .map(|(id, _)| *id)
        .collect();
    for id in &audio_ids {
        project.clip_pool.remove(id);
    }
    for track in project.tracks.values_mut() {
        track
            .clips
            .retain(|_, placement| !audio_ids.contains(&placement.clip_id));
    }
    let track_id = *project.tracks.keys().next().expect("演示工程至少一轨");
    let edge_id = *project
        .routing_graph
        .edges
        .keys()
        .next()
        .expect("演示工程有路由边（发送目标需要一条真边）");
    {
        let track = project.tracks.get_mut(&track_id).expect("轨存在");
        track.devices.push(DeviceDefinition {
            id: EntityId::new(),
            name: "Pro-Q 4".to_owned(),
            kind: DeviceKind::default(),
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "Cutoff".to_owned(),
                    value: 0.5,
                    unit: Some("Hz".to_owned()),
                },
                ParameterValue {
                    name: "Resonance".to_owned(),
                    value: 0.2,
                    unit: None,
                },
            ],
            latency_samples: 0,
            plugin: None,
        });
        track.macros.push(MacroParameter {
            name: "亮度".to_owned(),
            value: 0.4,
            mappings: Vec::new(),
        });
    }
    project.validate().expect("夹具工程必须自己就是合法的");

    let mut app = App::with_project(project).expect("装配 TUI 会话");
    // 轨道条按 `display_track_ids()` 的显示序（不是身份序）⇒ 显式选中那一条。
    let index = app
        .track_ids
        .iter()
        .position(|id| *id == track_id)
        .expect("夹具轨必须在轨道条里");
    app.selected = index;
    assert_eq!(app.selected_track_id(), Some(track_id));
    (app, track_id, edge_id)
}

fn six_kinds(edge_id: EntityId) -> Vec<AutomationKind> {
    vec![
        AutomationKind::Volume,
        AutomationKind::Pan,
        AutomationKind::Send { edge_id },
        AutomationKind::Device {
            slot_index: 0,
            param_index: 0,
        },
        AutomationKind::Macro { macro_index: 0 },
        AutomationKind::MidiCc { controller: 7 },
    ]
}

fn mid_value(app: &App) -> f32 {
    app.automation_domain()
        .map_or(0.5, |(min, max)| (min + max) / 2.0)
}

fn last_log(app: &App) -> String {
    app.log.last().cloned().unwrap_or_default()
}

fn temp_path(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "yeban-tui-p8-{tag}-{}-{:?}.yeban",
        std::process::id(),
        std::thread::current().id()
    ));
    path
}

fn lane_points(app: &App, track_id: EntityId, target: &AutomationTarget) -> Vec<(u64, f32)> {
    app.project
        .tracks
        .get(&track_id)
        .and_then(|track| track.automation_lanes.get(target))
        .map(|lane| {
            lane.points_in_tick_order()
                .into_iter()
                .map(|point| (point.tick, point.value))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn all_six_targets_create_update_and_delete_points() {
    let (mut app, track_id, edge_id) = fixture();

    for kind in six_kinds(edge_id) {
        app.open_automation_view(kind);
        let target = kind.target(track_id);
        assert_eq!(
            app.automation_target(),
            Some(target),
            "{}：`:auto` 应打开对应目标",
            kind.label()
        );

        // 建泳道：首个 `Op::SetAutomationPoint`（模型语义：空泳道不落盘）。
        let value = mid_value(&app);
        app.set_automation_value(value);
        let points = lane_points(&app, track_id, &target);
        assert_eq!(
            points,
            vec![(app.automation_cursor_tick(), value)],
            "{}：首个点必须落到模型里的泳道",
            kind.label()
        );

        // 改点：同 tick 重复写入 ⇒ 同一个点（身份不变、不堆积）。
        let lane = app.project.tracks[&track_id]
            .automation_lanes
            .get(&target)
            .expect("泳道存在")
            .clone();
        let first = lane.points_in_tick_order()[0];
        let updated = mid_value(&app);
        app.set_automation_value(updated);
        let lane = app.project.tracks[&track_id]
            .automation_lanes
            .get(&target)
            .expect("泳道仍在")
            .clone();
        let after = lane.points_in_tick_order();
        assert_eq!(after.len(), 1, "{}：同 tick 不应堆积第二个点", kind.label());
        assert_eq!(
            after[0].id,
            first.id,
            "{}：改点必须复用点身份",
            kind.label()
        );
        assert!(
            (after[0].value - updated).abs() < 1e-6,
            "{}：值必须更新",
            kind.label()
        );

        // 删点：空泳道随点一起消失（模型语义）。
        app.remove_automation_point_at_cursor();
        assert!(
            !app.project.tracks[&track_id]
                .automation_lanes
                .contains_key(&target),
            "{}：删掉最后一个点后泳道必须消失",
            kind.label()
        );
    }
}

#[test]
fn the_lane_list_shows_gui_names_and_opens_by_index() {
    let (mut app, _track_id, edge_id) = fixture();
    for kind in six_kinds(edge_id) {
        app.open_automation_view(kind);
        let value = mid_value(&app);
        app.set_automation_value(value);
    }

    let lanes = app.automation_lanes();
    assert_eq!(lanes.len(), 6, "六类目标都应出现在清单里");
    let mut sorted = lanes.clone();
    sorted.sort_by_key(|lane| lane.0);
    assert_eq!(
        lanes, sorted,
        "清单顺序必须是 `AutomationTarget::Ord`（与 GUI 同一序）"
    );

    let lines = app.automation_lane_lines();
    assert_eq!(lines.len(), 6);
    let joined = lines.join("\n");
    assert!(
        joined.contains("Cutoff"),
        "设备参数必须显示参数名（GUI 口径）：{joined}"
    );
    assert!(
        joined.contains("亮度"),
        "宏必须显示宏名（GUI 口径）：{joined}"
    );
    assert!(
        joined.contains(&format!("发送 {}", edge_id.to_canonical_string())),
        "发送泳道必须显示边身份（GUI 口径）：{joined}"
    );
    assert!(joined.contains("CC 7"), "CC 泳道必须显示控制器号：{joined}");

    // 序号打开：第 1 条（0 基）打开的就是清单里的那条目标。
    app.open_automation_lane(1);
    assert_eq!(app.automation_target(), Some(lanes[1].0));
    // 越界序号：响亮报错，不静默换目标。
    let before = app.automation_target();
    app.open_automation_lane(99);
    assert_eq!(app.automation_target(), before, "越界序号不得改变当前视图");
    assert!(
        last_log(&app).contains("没有第 99 条泳道"),
        "越界序号必须响亮：{}",
        last_log(&app)
    );
}

#[test]
fn missing_targets_are_refused_loudly_and_build_no_lane() {
    let (mut app, track_id, edge_id) = fixture();
    let foreign_edge = EntityId::new();
    // 错误文本一律是模型 `validate_against` 给的那条（具体到哪个槽/哪个下标），
    // TUI 只负责把它连同目标键一起响亮写出（`未打开泳道`）。
    let cases = [
        ("device 9 0".to_owned(), "out of range"),
        ("device 0 9".to_owned(), "out of range"),
        ("macro 9".to_owned(), "out of range"),
        ("send".to_owned(), "用法"),
        (
            format!("send {}", foreign_edge.to_canonical_string()),
            "routing edge",
        ),
        ("cc 200".to_owned(), "MIDI CC"),
    ];

    for (spec, needle) in cases {
        app.execute_command(&format!("auto {spec}"));
        assert!(
            app.automation_target().is_none(),
            "`auto {spec}` 被拒后不得留下自动化视图"
        );
        assert!(
            last_log(&app).contains(needle),
            "`auto {spec}` 必须响亮报错（含 `{needle}` 的模型错误原文），实际：{}",
            last_log(&app)
        );
        assert!(
            app.project.tracks[&track_id].automation_lanes.is_empty(),
            "`auto {spec}` 被拒后不得建出任何泳道"
        );
    }

    // 对照：真实存在的边能开（说明上面的拒绝不是"一律拒绝"）。
    app.execute_command(&format!("auto send {}", edge_id.to_canonical_string()));
    assert_eq!(
        app.automation_target(),
        Some(AutomationTarget::SendGain { track_id, edge_id })
    );
}

#[test]
fn midi_cc_domain_is_the_seven_bit_controller_range() {
    let (mut app, track_id, edge_id) = fixture();
    app.open_automation_view(AutomationKind::MidiCc { controller: 7 });
    let target = AutomationTarget::MidiCc {
        track_id,
        controller: 7,
    };
    assert_eq!(
        app.automation_domain(),
        Some((0.0, 127.0)),
        "CC 值域来自模型"
    );
    assert_eq!(app.automation_target(), Some(target));

    app.set_automation_value(128.0);
    assert!(
        !app.project.tracks[&track_id]
            .automation_lanes
            .contains_key(&target),
        "128 越出 7 位值域，必须拒绝且不建泳道"
    );
    assert!(
        last_log(&app).contains("越出"),
        "越界写入必须响亮：{}",
        last_log(&app)
    );

    app.set_automation_value(127.0);
    assert_eq!(
        lane_points(&app, track_id, &target),
        vec![(app.automation_cursor_tick(), 127.0)]
    );

    // 步长：整数域 ⇒ 一次微调 1。
    assert!((AutomationKind::MidiCc { controller: 7 }.step(None) - 1.0).abs() < 1e-6);
    let _ = edge_id;
}

#[test]
fn device_params_keep_the_unknown_domain_honest() {
    let (mut app, track_id, edge_id) = fixture();
    app.open_automation_view(AutomationKind::Device {
        slot_index: 0,
        param_index: 1,
    });
    let target = AutomationTarget::DeviceParam {
        track_id,
        slot_index: 0,
        param_index: 1,
    };
    assert!(
        app.automation_domain().is_none(),
        "设备参数值域模型不声明 ⇒ TUI 必须照实说未知（⛔ 不编一个 0..=1）"
    );

    // 值域未知 ⇒ 任何有限值都收（面板里那个参数的真实域由渲染/插件决定）。
    app.set_automation_value(1234.5);
    assert_eq!(
        lane_points(&app, track_id, &target),
        vec![(app.automation_cursor_tick(), 1234.5)]
    );

    // 非有限照旧拒绝。
    app.set_automation_value(f32::NAN);
    assert!(
        last_log(&app).contains("不是有限数"),
        "NaN 必须被拒：{}",
        last_log(&app)
    );
    assert_eq!(lane_points(&app, track_id, &target).len(), 1);

    // 设备视图的人读名 = 参数名（GUI 口径）。
    assert_eq!(app.automation_lane_name(), "Resonance");
    let _ = edge_id;
}

#[test]
fn six_lanes_survive_a_save_and_reopen() {
    let (mut app, track_id, edge_id) = fixture();
    let mut expected: Vec<(AutomationTarget, Vec<(u64, f32)>)> = Vec::new();
    for kind in six_kinds(edge_id) {
        app.open_automation_view(kind);
        let target = kind.target(track_id);
        let value = mid_value(&app);
        app.set_automation_value(value);
        // 第二个点：换一个小节，验证多点排序与身份都随归档往返。
        // `set_automation_value` 在已有选中点时按**选中点**的 tick 写；换小节后
        // 先用 `a` 在新光标处落点，再改值，否则会更新第一个点而不是新增第二个。
        app.cursor_bar += 2;
        app.add_automation_point_at_cursor();
        app.set_automation_value(
            value
                + app
                    .automation_domain()
                    .map_or(0.1, |(min, max)| (max - min) * 0.25),
        );
        expected.push((target, lane_points(&app, track_id, &target)));
    }

    let path = temp_path("six-lanes");
    app.save_to(&path).expect("保存");
    let reopened = App::open_file(Path::new(&path)).expect("重开");
    let _ = std::fs::remove_file(&path);

    for (target, points) in expected {
        assert_eq!(points.len(), 2, "{target:?}：夹具应写两个点");
        assert_eq!(
            lane_points(&reopened, track_id, &target),
            points,
            "{target:?}：往返后逐点（tick, 值）必须一致"
        );
    }
}
