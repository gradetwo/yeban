//! **自动化泳道可见时，片段本体点击优先于泳道命中面**（T-FIX-968）。
//!
//! 旧判据（T-FIX-714）只断言源码文本：泳道 `TouchArea` 覆盖片段覆盖区。
//! 本任务把验收改成**真实合成点击**：默认 `automation-visible == true` 的活窗口里
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① | 单击片段本体（`clip-{ulid}-body` 中心，落在泳道带内）⇒ 片段被选中，`REGION` 显示片段名/起始小节，自动化点数不变 |
//! | ② | 单击片段**外**的同一泳道热区 ⇒ 自动化点数 +1，片段选中不被清掉 |
//!
//! 两击都在同一条轨、同一条泳道带内，唯一差别是 x 是否落在片段覆盖区 —
//! 区分的正是「泳道命中面是否盖过片段本体」这条命中路径。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的且每线程只能装配一次 ⇒ 本文件
//! 只保留这一条建窗判据；其余纯函数判据在别的测试文件里。

#![allow(unused)]
use std::cell::RefCell;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{ClipView, ViewState, demo_project};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

const NOW_MS: u64 = 1_700_000_000_000;
const WINDOW: Size = Size::new(1440, 900);
const FLICKABLE: &str = "arr-lanes-flickable";

/// 片段绘制几何（与 `host::clip_marquee_hits` 的判据侧复算同源）。
fn block_rect(view: &ViewState, clip: &ClipView) -> (f32, f32, f32, f32) {
    let ys = view.track_ys();
    let heights = view.track_heights();
    (
        clip.x.max(0.0),
        ys[clip.track_index] + 4.0,
        clip.width.max(12.0),
        (heights[clip.track_index] - 8.0).max(52.0),
    )
}

fn lane_point_count(undo: &Rc<UndoPort>, target: &yeban_model::AutomationTarget) -> usize {
    let project = undo.try_project().expect("工程必须可读");
    let track = project
        .tracks
        .get(&target.track_id())
        .expect("目标轨道必须存在");
    track
        .automation_lanes
        .get(target)
        .map_or(0, |lane| lane.points.len())
}

#[test]
fn clip_body_click_wins_over_visible_automation_lane() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 主窗口");
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<lane-click-layer>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开"),
    ));
    let live_view = Rc::new(RefCell::new(view.clone()));
    host::wire_clip_scope(port.ui(), Rc::clone(&live_view), Some(Rc::clone(&undo)));
    // 片段本体的 `up` 只有在 `clip-drag-grab` 返回 true（未拖动）时才报
    // `clip-select`；产品进程同样接这个端口。
    host::wire_clip_drag(port.ui(), &undo);
    host::wire_automation_lane_click(port.ui(), &undo);
    assert!(
        port.ui().get_automation_visible(),
        "本判据测的是默认态：自动化泳道必须可见"
    );
    port.refresh_tree(None).expect("必须能抓控件树");

    let bounds = port
        .tree()
        .find_by_id(FLICKABLE)
        .expect("控件树里必须有编曲滚动面")
        .bounds
        .expect("滚动面必须有几何");
    let origin_x = f64::from(port.ui().get_track_headers_width()) + 5.0;
    let visible_tx_max = (bounds.width as f32) - (origin_x as f32) - 24.0;

    // 取一条轨、它的第一卷片段、同一轨的第一条泳道。
    let clip = view
        .clips
        .iter()
        .find(|clip| {
            let (x, _, w, _) = block_rect(&view, clip);
            x + w / 2.0 < visible_tx_max
        })
        .expect("演示工程必须有可见片段");
    let lane = view
        .automation_lanes
        .iter()
        .find(|lane| lane.track_index == clip.track_index)
        .expect("片段所属轨必须有自动化泳道");
    let (cx, cy, cw, ch) = block_rect(&view, clip);
    let clip_center_y = cy + ch / 2.0;
    // 缺陷路径守卫：片段本体中心必须落在一条泳道带内。
    assert!(
        view.automation_lanes.iter().any(|lane| {
            lane.track_index == clip.track_index
                && clip_center_y >= lane.band_y
                && clip_center_y < lane.band_y + lane.band_height
        }),
        "片段本体中心必须落在泳道带内（否则测不到被吞路径）：track {} y {clip_center_y}",
        clip.track_index
    );

    // ① 片段本体中心：真实合成按下/释放。
    let body_id = format!("clip-{}-body", clip.placement_id);
    let body_bounds = port
        .tree()
        .find_by_id(&body_id)
        .and_then(|node| node.bounds)
        .expect("片段本体必须进控件树");
    let points_before_clip_click = lane_point_count(&undo, &lane.target);
    port.dispatch_pointer_down(
        &body_id,
        f64::from(body_bounds.width) / 2.0,
        f64::from(body_bounds.height) / 2.0,
        PointerButton::Left,
    )
    .expect("片段本体必须可按下");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");

    let selected: Vec<String> = port
        .ui()
        .get_selected_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(
        selected,
        vec![clip.placement_id.clone()],
        "泳道可见时单击片段本体必须选中该片段（不得被泳道命中面吞掉）"
    );
    assert_eq!(
        port.ui().get_selected_clip_ulid().as_str(),
        clip.placement_id.as_str(),
        "主选中必须是被点的片段"
    );
    assert_eq!(
        port.ui().get_region_title().as_str(),
        clip.clip_name.as_str(),
        "Channel Inspector 的 REGION 必须显示片段名"
    );
    assert!(
        port.ui().get_region_detail().contains("第 1 小节"),
        "REGION 位置必须显示片段起始小节：{}",
        port.ui().get_region_detail()
    );
    assert_eq!(
        lane_point_count(&undo, &lane.target),
        points_before_clip_click,
        "片段本体点击不得被泳道当成加点（点数必须不变）"
    );

    // ② 同一泳道带、片段右侧的空白热区：单击仍归泳道。
    let lane_click_x = cx + cw + 20.0;
    let lane_click_y = lane.band_y + lane.band_height / 2.0;
    assert!(
        lane_click_x < visible_tx_max,
        "片段外泳道点击必须在可见宽内：{lane_click_x} vs {visible_tx_max}"
    );
    let points_before_lane_click = lane_point_count(&undo, &lane.target);
    port.dispatch_pointer_down(
        FLICKABLE,
        origin_x + f64::from(lane_click_x),
        f64::from(lane_click_y),
        PointerButton::Left,
    )
    .expect("片段外泳道必须可按下");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");
    assert_eq!(
        lane_point_count(&undo, &lane.target),
        points_before_lane_click + 1,
        "片段外泳道热区单击必须新增一个自动化点"
    );
    assert_eq!(
        port.ui().get_selected_clip_ulid().as_str(),
        clip.placement_id.as_str(),
        "泳道加点不得清掉片段选中"
    );
}
