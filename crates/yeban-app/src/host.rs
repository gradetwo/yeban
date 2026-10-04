//! 投影结果 → Slint `MainWindow` 的**单向注入**（本文件是唯一的注入实现）。
//!
//! 规范来源 (Normative):
//! - `[MODEL-AST-002]` 界面只读 `YebanProjectV1` 的投影，绝不反向写模型。
//! - `[UI-TEST-001]` §12.2 语义 ID 的 `{ulid}` 段来自 [`crate::bridge`]，不是 `.slint` 字面量。
//! - `[UI-GRID-001]` §1.1 网格：轨道数 / 剪辑数 / 段落数现在由**工程**决定，
//!   `.slint` 里不再有 `for … in 6` 这种把演示数据刻进界面的写法。
//! - `[ARCH-TOP-002]` / `[UI-GRID-004]` UI 线程与实时音频线程物理隔离：本文件只做
//!   属性注入（O(轨道数) 的拷贝），不做 I/O、不阻塞、不进音频路径。
//! - `AGENTS.md` §3 DoD 6「UI 变更必须双重验证」：`main.rs` 与
//!   `src/test_port_adapter.rs`（以及自动发现的 `tests/real_ui_tier1.rs`）**共用**本文件，
//!   因此"命令行看到的界面"与"CI 断言的界面"是同一段代码构造的，不存在两份注入实现。
//!
//! ## 数据流方向（不可逆）
//!
//! ```text
//! YebanProjectV1 --(bridge::from_project)--> ViewState --(host::apply_view)--> MainWindow
//! ```
//!
//! 反方向的任何写入（UI → 模型）都必须经过 `yeban-model` 的领域操作（`Op`）与
//! `yeban-engine` 的调度，不属于本模块 —— 本模块**故意**不提供任何"读回 UI 状态"的入口。
//!
//! ## 为什么这些数组用 `ModelRc` 而不是常量默认值
//!
//! Slint 的数组属性需要默认值才能在**没有宿主**时独立渲染。上一版因此把演示 ULID /
//! 轨道名内联进 `.slint`，形成"同一份常量写两处"的债（`docs/ledger/ui-shell-notes.md` pending 4）。
//! 本线把 arrangement / session 两处的默认值改成**空数组**，数据一律由 Rust 注入 ——
//! 代价是"直接预览 `.slint` 会看到空轨道"，收益是**只有一个事实源**。

use slint::{ModelRc, SharedString, VecModel};

use crate::bridge::ViewState;
use crate::scene::DemoScene;
use crate::ui::MainWindow;

/// `[string]` 属性 ← `&[String]`。
fn strings(values: &[String]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        values
            .iter()
            .map(|value| SharedString::from(value.as_str()))
            .collect::<Vec<_>>(),
    ))
}

/// `[length]` 属性 ← `&[f32]`（Slint 的 `length` 在 Rust 侧就是 `f32`）。
fn lengths(values: &[f32]) -> ModelRc<f32> {
    ModelRc::new(VecModel::from(values.to_vec()))
}

/// `[int]` 属性 ← `&[i32]`。
fn integers(values: &[i32]) -> ModelRc<i32> {
    ModelRc::new(VecModel::from(values.to_vec()))
}

/// `[bool]` 属性 ← `&[bool]`。
fn booleans(values: &[bool]) -> ModelRc<bool> {
    ModelRc::new(VecModel::from(values.to_vec()))
}

/// 把投影状态**单向**注入一个已存在的 `MainWindow`。
///
/// 幂等：对同一个 `ViewState` 重复调用得到同一个界面（判据
/// `project_projection_reaches_the_control_tree_and_the_pixels` 就是靠这一点
/// 在同一个活窗口上切换两个工程）。
pub fn apply_view(ui: &MainWindow, view: &ViewState) {
    ui.set_window_title(view.title.clone().into());
    ui.set_bpm_display(view.bpm_display.clone().into());
    ui.set_track_names(strings(&view.track_names()));
    ui.set_track_volumes(strings(&view.track_volumes()));
    ui.set_track_mutes(booleans(&view.track_mutes()));
    ui.set_track_solos(booleans(&view.track_solos()));
    ui.set_scene_names(strings(&view.scene_names()));
    ui.set_section_names(strings(&view.section_names()));
    ui.set_clip_ulids(strings(&view.clip_ulids()));
    ui.set_clip_labels(strings(&view.clip_labels()));
    ui.set_clip_positions(lengths(&view.clip_positions()));
    ui.set_clip_widths(lengths(&view.clip_widths()));
    ui.set_clip_lanes(integers(&view.clip_lanes()));
    ui.set_section_positions(lengths(&view.section_positions()));
    ui.set_section_widths(lengths(&view.section_widths()));
    ui.set_bar_positions(lengths(&view.bar_positions));
    ui.set_note_ulids(strings(&view.note_ulids));
    ui.set_note_velocities(lengths(&view.note_velocities));
}

/// 构造主窗口：注入**外壳场景**（会话运行态 / 本机视口）+ 投影状态。
///
/// 这是 `main.rs` 与全部 UI 判据的**唯一**构造入口。
///
/// # Errors
///
/// 平台后端不可用时返回 `slint::PlatformError`（无显示器环境的常见形态；
/// CI 走 `yeban-ui-test-port` 的 Tier-1 软件平台，不经过这里）。
pub fn build_main_window(
    view: &ViewState,
    scene: &DemoScene,
) -> Result<MainWindow, slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.set_timecode(scene.timecode.clone().into());
    ui.set_branch_name(scene.branch_name.clone().into());
    ui.set_arrangement_view(scene.arrangement_by_default);
    ui.set_playing(false);
    ui.set_console_tab(0);
    ui.set_compact(scene.compact());
    apply_view(&ui, view);
    Ok(ui)
}
