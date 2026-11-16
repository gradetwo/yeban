//! **心跳降频判据**（性能实测 P2-B）。
//!
//! 背景：main.rs 的 HEARTBEAT_PERIOD_MS = 16（60 Hz）常驻，即使**不播放**也在跑；
//! 空闲实测 0.6–3.5% 单核，是笔记本风扇/耗电来源；轨数越多越贵。
//!
//! 修法：心跳闭包按 ui.get_playing() 分流 —— 停播时每 40 ms 才做一次重活
//! （约 20 Hz），播放时保持 60 Hz（走带/电平要跟手）。
//!
//! ⚠ 纯 include_str! 判据（本机编不了 Slint 重依赖）；生产心跳不参与单测
//! （测试直接驱动 ProductionLoop::tick），故本判据钉的是接线与阈值。

const MAIN: &str = include_str!("../src/main.rs");

#[test]
fn heartbeat_throttles_to_about_20hz_when_not_playing() {
    let start = MAIN
        .find("heartbeat.start(")
        .expect("main.rs 必须有 heartbeat.start");
    let tail = &MAIN[start..];
    assert!(
        tail.contains("get_playing()"),
        "心跳必须按播放态决定频率（性能 P2-B）"
    );
    assert!(
        tail.contains("from_millis(40)"),
        "停播阈值必须 40 ms（约 20 Hz），否则空闲仍是 60 Hz"
    );
    assert!(
        tail.contains("last_heavy_at"),
        "必须记录上次重活时间戳，才能跳过被节流的那些跳"
    );
}

/// 阈值必须是停播专用：播放态那条分支不得也被 40 ms 拦。
#[test]
fn playing_path_is_not_throttled() {
    let start = MAIN.find("heartbeat.start(").expect("heartbeat");
    let tail = &MAIN[start..];
    assert!(
        tail.contains("} else {"),
        "必须有播放态分支（否则缺失会连播放也降频）"
    );
    assert!(
        tail.matches("last_heavy_at.set").count() >= 2,
        "停播（节流后）与播放（每跳）两条路径都要刷新时间戳"
    );
}
