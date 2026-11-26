//! T-FIX-764 装配判据（源码文本级）：卷帘试听行号不得夹到 15。
//! 本机按 AGENTS.md §5 不跑 Slint 重依赖构建；GUI 点击/高亮实测交 CI。

const ROLL: &str = include_str!("../ui/console/piano_roll.slint");
const BRIDGE: &str = include_str!("../src/bridge.rs");
const HOST: &str = include_str!("../src/host.rs");

#[test]
fn no_row_clamp_to_15_remains() {
    assert_eq!(
        ROLL.matches("Math.min(15").count(),
        0,
        "琴键按下/滑奏不得再把行号夹到 15"
    );
    assert_eq!(
        ROLL.matches("root.roll-rows - 1").count(),
        2,
        "按下与滑奏两处都须按 roll-rows 全量映射"
    );
}

#[test]
fn pitch_mapping_keeps_base_plus_15_minus_row() {
    let f = BRIDGE
        .find("pub fn pitch_for_roll_row_with_base(")
        .expect("bridge 必须有全量行号音高映射");
    let tail = &BRIDGE[f..];
    let end = tail
        .find("/// `[UI-NOTE-002/003]` Map a piano-roll")
        .unwrap_or(tail.len());
    let body = &tail[..end];
    assert!(
        body.contains("(PITCH_LANE_COUNT - 1) - row"),
        "音高口径须为 base + 15 - row: {body}"
    );
    assert!(
        body.contains("u8::try_from(pitch).ok()"),
        "越界音高须显式拒绝，不得静默夹值: {body}"
    );
}

#[test]
fn audition_call_sites_use_full_row_mapping() {
    assert_eq!(
        HOST.matches("pitch_for_roll_row_with_base(lane, roll_base_pitch(&ui))")
            .count(),
        3,
        "按下/释放/绘制试听三处都须用全量行号映射"
    );
    // 16 车道标签构造仍用旧映射，属另一路语义，不得被误改。
    assert!(
        HOST.contains("pitch_for_lane_with_base(lane, next)"),
        "16 车道标签路径保持原样"
    );
    assert!(
        HOST.matches("试听: 行号超出可发声范围").count() >= 2,
        "行号越界必须响亮报告，不得静默 return"
    );
}
