//! T-FIX-714 等价装配判据（源码文本级）。
//!
//! 本机按 AGENTS.md §5 不跑 Slint 重依赖构建，故以源码文本判据验证：
//! 泳道可见时整条带（含片段覆盖区）由泳道自身 TouchArea 接单击并报
//! `automation-lane-click`；泳道隐藏时 `visible:false` 不接指针，片段命中回归。
//! 像素级点击实测仍由 CI/GUI 节点负责，本判据不冒充渲染验证。

const ARR: &str = include_str!("../ui/arrangement.slint");

fn lane_block() -> &'static str {
    let start = ARR
        .find("for _lane[lane_index] in root.automation-lane-target-keys")
        .expect("泳道循环必须存在");
    let tail = &ARR[start..];
    let end = tail.find("Splitter Divider").unwrap_or(tail.len());
    &tail[..end]
}

#[test]
fn visible_lane_receives_click_in_clip_covered_area() {
    let block = lane_block();
    assert!(
        block.contains("TouchArea {"),
        "泳道必须自带 TouchArea，否则片段覆盖区点击仍只选片段:\n{block}"
    );
    assert!(
        block.contains("root.automation-lane-click("),
        "泳道 TouchArea 必须上报 automation-lane-click:\n{block}"
    );
    assert!(
        block.contains("root.automation-lane-band-ys[lane_index] - 42px"),
        "y 坐标必须按 band_y - 42px 还原（宿主口径）:\n{block}"
    );
    assert!(
        block.contains("visible: root.automation-visible"),
        "泳道仍须受 automation-visible 控制，隐藏时不得接指针:\n{block}"
    );
}

#[test]
fn single_lane_click_report_no_duplicate_from_blank_gesture() {
    let count = ARR.matches("root.automation-lane-click(").count();
    assert_eq!(
        count, 1,
        "automation-lane-click 只能由泳道 TouchArea 上报一次；空白手势里的旧上报必须移除"
    );
}

#[test]
fn hidden_lane_keeps_clip_hit_and_selection() {
    let block = lane_block();
    assert!(
        block.contains("visible: root.automation-visible"),
        "泳道隐藏态开关丢失"
    );
    // 隐藏时泳道不接指针，原框选/选轨状态机保持。
    assert!(ARR.contains("root.select-track(trk.id);"), "选轨语义丢失");
    assert!(ARR.contains("root.clip-marquee-select("), "框选语义丢失");
}
