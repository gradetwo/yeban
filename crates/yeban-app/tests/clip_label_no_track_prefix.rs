//! **片段标签只放片段名，不重复轨道名**（用户 2026-11-16 反馈）。
//!
//! 修前：`ClipView.label` 拼成 `"{track_name} · {clip_name}"`，每个片段都画一遍
//! 轨道名 —— 而左侧轨道头已经写了是哪条轨，纯属重复/信息过载。
//!
//! 修法：`label = clip_name`（空名时 `"未命名片段"`）；轨道名只由轨道头显示。
//!
//! ⚠ 纯 `include_str!` 判据；像素效果由 5 张 golden 基准看守（本改动会触发重录）。

const BRIDGE: &str = include_str!("../src/bridge.rs");

#[test]
fn clip_label_does_not_duplicate_the_track_name() {
    assert!(
        !BRIDGE.contains(r#"format!("{track_name} · {clip_name}")"#),
        "片段标签不得再拼「轨道名 · 片段名」（轨道头已显示轨道名）"
    );
    assert!(
        BRIDGE.contains("未命名片段"),
        "空片段名必须用「未命名片段」兜底，⛔ 不回退成轨道名"
    );
    assert!(
        BRIDGE.contains("let label = if clip_name.is_empty() {"),
        "label 必须由 clip_name 决定"
    );
}
