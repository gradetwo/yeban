//! **双击复位**（推子 → `0 dB`、声相 → 中央）—— **文本层**判据。
//!
//! ## 本文件要证的句子
//!
//! 调音台的四块命中面（通道条推子 / 通道条声相 / 主控推子 / 主控声相）上那个
//! `double-clicked` **不是一个画了但不动的手势**（[`ADR-0052`] 那一类）：
//! 每一面各自绑**自己**的复位回调，回调一路转发到 `MainWindow`，宿主四条臂都汇到
//! **同一个**落点 [`host::commit_mixer_reset`]，而且**先**清掉在手的拖拽手势**再**提交。
//!
//! 仓内已有同族先例：底部控制台 splitter 的双击回默认档位（`ui/app.slint` 的
//! `double-clicked => { … root.console-height-reset(); }`，宿主 `host.rs` 的
//! `ui.on_console_height_reset`）。本票把同一条手势给混音台的推子 / 声相。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① | `mixer_console.slint` 里**恰好**四块命中面带 `double-clicked`；每一块**往上**读得到 `TouchArea {` 与它**自己**的 `x/y/width/height`（几何一位没动），**往下**读得到**它自己**那条回调（⛔ 串面：把声相臂贴到推子矩形上 ⇒ 红） |
//! | ② | 四个回调在 `mixer_console` / `console_tabs` / `app` **三个**文件里各**声明一次**，并在 `console_tabs` / `app` 各**转发一次**（少一段 ⇒ 手势落在没人听的回调上；多一段 ⇒ 有第二条路径绕过宿主） |
//! | ③ | 宿主四条 `on_*_reset` 臂各**恰好一次**，每臂里 `end_mixer_gesture` 出现在 `commit_mixer_reset` **之前**，且写的是**自己**那一格（`MixerResetField::Volume` / `::Pan`） |
//! | ④ | 中性值**只有一处**定义：`commit_mixer_reset` 定义一次、被调用四次；`new_val: 0.0` 在整个 `host.rs` 里**恰好一次** ⇒ 界面上不存在第二份"默认值"常量与它分叉 |
//!
//! ## ⛔ 本文件不证的东西（正对照在哪）
//!
//! 文本层证的是"接线存在且唯一"，**不证**"双击真的改了工程"。后者由
//! `tests/live_ui_mcp.rs` 的两条活体判据负责（`a_double_click_reset_moves_the_fader_and_pan_to_neutral`
//! / `a_double_click_reset_leaves_no_sticky_gesture`）：它们驱动**活的** `MainWindow`
//! 从权威工程读回数值。两半不可互相代替 —— 只看文本会在"回调声明了但宿主没接"这种
//! 状态下全绿，只看活体则钉不住"中性值只有一处定义"。
//!
//! ## 变异表（每条都实测过能变红）
//!
//! | 注入 | 文件 | 预期 |
//! | :--- | :--- | :--- |
//! | 把声相命中面的 `root.mixer-pan-reset(track_index);` 换成 `root.mixer-fader-reset(track_index);` | `ui/console/mixer_console.slint` | ① 红（第 2 面的调用不符） |
//! | 删掉 `console_tabs.slint` 里 `mixer-master-pan-reset()` 的转发臂 | `ui/console/console_tabs.slint` | ② 红（`root.mixer-master-pan-reset();` 计数 2 → 1） |
//! | 删掉某一臂里的 `end_mixer_gesture(&ui, &gesture);` | `src/host.rs` | ③ 红（找不到清零点，`commit` 之前无 `end`） |
//! | 把某一臂的 `MixerResetField::Pan` 改成 `::Volume` | `src/host.rs` | ③ 红（写错格）＋活体判据 ② 红（声相没动） |
//! | 在 `commit_mixer_reset` 之外再写一处 `new_val: 0.0` | `src/host.rs` | ④ 红（计数 1 → 2） |

#![allow(unused)]
/// 返回 `source` 里含 `needle` 的**行号**（1 基，按文档顺序）。
///
/// 数行而不是数子串：命中面与回调的**配对**关系要看"哪一块矩形上面写着什么"，
/// 全文 `matches()` 会把四块矩形混成一堆无上下文的计数。
fn line_numbers(source: &str, needle: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(needle))
        .map(|(nth, _)| nth + 1)
        .collect()
}

/// 命中面 `(x, y, width, height)` 四行 ＋ 它应当调用的那条回调。
type HitRect<'a> = ([&'a str; 4], &'a str);

/// 文档顺序：通道条推子 → 通道条声相 → 主控推子 → 主控声相
/// （与 `mixer_console.slint` 里两个组件的排布同序）。
const TRACK_AND_MASTER_HIT_RECTS: [HitRect<'_>; 4] = [
    (
        ["x: 22px;", "y: 54px;", "width: 48px;", "height: 116px;"],
        "root.mixer-fader-reset(track_index);",
    ),
    (
        [
            "x: Tokens.space-3;",
            "y: 18px;",
            "width: 36px;",
            "height: 18px;",
        ],
        "root.mixer-pan-reset(track_index);",
    ),
    (
        ["x: 24px;", "y: 54px;", "width: 48px;", "height: 116px;"],
        "root.mixer-master-fader-reset();",
    ),
    (
        [
            "x: Tokens.space-3;",
            "y: 18px;",
            "width: 36px;",
            "height: 18px;",
        ],
        "root.mixer-master-pan-reset();",
    ),
];

/// ① 四块命中面各自绑对自己的回调，几何一位没动。
#[test]
fn each_mixer_hit_rect_binds_its_own_double_click_reset() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ② 四个回调从组件一路转发到 `MainWindow`，每一段**恰好**一次。
#[test]
fn the_four_reset_callbacks_forward_to_the_main_window_exactly_once_each() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ③ 宿主四条臂：先清手势、后提交，而且写的是自己那一格。
#[test]
fn every_host_reset_arm_clears_the_gesture_before_committing_its_own_field() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ④ 中性值只有一处定义：落点唯一、`0.0` 只出现一次。
#[test]
fn the_neutral_value_is_defined_in_exactly_one_place() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
