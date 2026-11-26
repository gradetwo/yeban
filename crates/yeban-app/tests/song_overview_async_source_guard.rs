//! **T-FIX-429 反向腿**：预设 / 等级回调一旦退回同步 `refresh_song_overview`，本判据变红。
//!
//! 这是结构性判据，不是运行时判据；运行时行为由 `song_overview_async.rs` 的慢分析
//! 接缝覆盖。两者一起防止“只把增量代码搬到后台、回调又偷偷同步跑一次”。

#[test]
fn song_overview_cycle_callbacks_dispatch_instead_of_rendering_inline() {
    let source = include_str!("../src/song_overview.rs");
    let profile_start = source
        .find("ui.on_song_overview_profile_cycle")
        .expect("必须存在预设循环回调");
    let profile_end = source[profile_start..]
        .find("ui.on_song_overview_target_cycle")
        .map(|end| profile_start + end)
        .expect("预设回调后必须紧跟等级循环回调");
    let profile_block = &source[profile_start..profile_end];
    assert!(
        profile_block.contains("request_song_overview_refresh"),
        "预设回调必须投后台任务"
    );
    assert!(
        !profile_block.contains("refresh_song_overview(&ui"),
        "预设回调不得退回同步 refresh_song_overview（反向腿）"
    );

    let target_start = source
        .find("ui.on_song_overview_target_cycle")
        .expect("必须存在等级循环回调");
    let target_end = source[target_start..]
        .find("ui.on_song_overview_capture_a")
        .map(|end| target_start + end)
        .expect("等级回调后必须紧跟 A/B 快照回调");
    let target_block = &source[target_start..target_end];
    assert!(
        target_block.contains("request_song_overview_refresh"),
        "等级回调必须投后台任务"
    );
    assert!(
        !target_block.contains("refresh_song_overview(&ui"),
        "等级回调不得退回同步 refresh_song_overview（反向腿）"
    );
}
