//! 投影结果 + 电平读数 → Slint `MainWindow` 的**单向注入**（本文件是唯一的注入实现）。
//!
//! 规范来源 (Normative):
//! - `[MODEL-AST-002]` 界面只读 `YebanProjectV1` 的投影，绝不反向写模型。
//! - `[ARCH-UI-002]` 电平：**"UI 线程定时器批量出队更新 Slint Properties"** ——
//!   [`apply_meters`] 就是那句话里的"更新 Slint Properties"（出队在
//!   [`crate::meters::MeterRuntime::poll`]，本文件只负责把结果写进界面）。
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
//!                                                              ^
//! meter_channel(SPSC) --(meters::MeterRuntime::poll)--> MeterSnapshot --(host::apply_meters)
//! ```
//!
//! 反方向的任何写入（UI → 模型）都必须经过 `yeban-model` 的领域操作（`Op`）与
//! `yeban-engine` 的调度，不属于本模块 —— 本模块**故意**不提供任何"读回 UI 状态"的入口。
//!
//! ## 为什么这些数组用 `ModelRc` 而不是常量默认值
//!
//! Slint 的数组属性需要默认值才能在**没有宿主**时独立渲染。上一版因此把演示 ULID /
//! 轨道名内联进 `.slint`，形成"同一份常量写两处"的债（`docs/ledger/ui-shell-notes.md` pending 4）。
//! 本线把 arrangement / session / mixer 三处的默认值改成**空数组**，数据一律由 Rust 注入 ——
//! 代价是"直接预览 `.slint` 会看到空轨道"，收益是**只有一个事实源**。
//!
//! ## 为什么电平要**重置**再填
//!
//! [`apply_view`] 末尾会先写一份 `[`silent_snapshot`](crate::meters::silent_snapshot)`：
//! 换工程时轨道数会变，而电平数组的长度**必须与 `track-names` 等长**（`.slint` 按下标取，
//! 不等长就会取到别的轨道的值或越界）。因此顺序是"先按新工程重置成静音 → 再由 60Hz 的
//! [`apply_meters`] 填真实读数"，而不是"只写一次、之后靠运气对齐"。

use slint::ComponentHandle as _;
use std::cell::RefCell;
use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};
use yeban_engine::transport::TransportReading;

use crate::bridge::{DEFAULT_TRACK_COLOR, RgbColor, TrackHeightLayout, ViewState};
use crate::engine_host::EngineHost;
use crate::input::{Action, Focus, InputContext, LogicalKey, Modifiers, Resolution, View};
use crate::meters::{MeterSnapshot, silent_snapshot};
use crate::scene::DemoScene;
use crate::ui::{MainWindow, ThemeState, YebanTheme};
use crate::undo::{UiAction, UndoPort};

/// [`wire_save`] 的**装配输入**：保存到哪 + 没有权威时用哪一份工程。
///
/// 为什么把这两样捆成一个结构体：它们是"保存这个动作"在界面这一侧的全部输入，
/// 而**权威**（有的话）由 `wire_save` 的第二个参数单独给 —— 于是"有权威时
/// `project` 字段不被使用"这件事在签名上是可见的，而不是埋在函数体里。
#[derive(Debug, Clone)]
pub struct SaveStatus {
    /// 保存目标（`None` = 这个会话没有磁盘对应物，保存会从此如实报错）。
    pub target: Option<std::path::PathBuf>,
    /// GUI 当前持有的工程（**只在没有控制面权威时**被 `dispatch_save` 使用）。
    pub project: yeban_model::YebanProjectV1,
}

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

/// `[color]` 属性 ← `&[RgbColor]`。
///
/// 十六进制 **解析**发生在投影层（`bridge::parse_hex_color`，可判据化）；
/// 这里只做"已解析的 u8 三元组 → `slint::Color`"这一次转换 ——
/// 因此全仓库只有一处颜色语法实现（见 `docs/ledger/app-completion-notes.md` §2）。
fn colors(values: &[RgbColor]) -> ModelRc<slint::Color> {
    ModelRc::new(VecModel::from(
        values
            .iter()
            .map(|color| to_color(*color))
            .collect::<Vec<_>>(),
    ))
}

/// 已解析的 RGB → Slint 颜色（**唯一**的转换点）。
fn to_color(color: RgbColor) -> slint::Color {
    slint::Color::from_rgb_u8(color.red, color.green, color.blue)
}

/// 把投影状态**单向**注入一个已存在的 `MainWindow`。
///
/// 幂等：对同一个 `ViewState` 重复调用得到同一个界面（判据
/// `project_projection_reaches_the_control_tree_and_the_pixels` 就是靠这一点
/// 在同一个活窗口上切换两个工程）。
pub fn apply_view(ui: &MainWindow, view: &ViewState, viewport_width: f32, scroll_x: f32) {
    ui.set_window_title(view.title.clone().into());
    ui.set_bpm_display(view.bpm_display.clone().into());
    // 时间码的拍号网格：**唯一**的注入点（本函数）。`None`（模型层拒绝该拍号）
    // 一律写 0，`apply_transport` 见到 0 就如实退回 tick 文本 —— 见 `timecode_grid_of`。
    let grid = view.timecode_grid();
    ui.set_timecode_ticks_beat(
        grid.map_or(0, |grid| i32::try_from(grid.ticks_per_beat()).unwrap_or(0)),
    );
    ui.set_timecode_ticks_bar(
        grid.map_or(0, |grid| i32::try_from(grid.ticks_per_bar()).unwrap_or(0)),
    );
    ui.set_track_names(strings(&view.track_names()));
    // 轨道**身份**数组：拖拽手势落在轨道头上（界面只能报下标），而每轨高度覆盖按
    // **身份**键控 ⇒ 宿主需要这一格"下标 → 身份"的查表。它与下面两行的行几何来自同一次
    // 注入，因此换工程之后两者一起更新，不会有一份过期的身份副本。
    ui.set_track_ids(strings(&view.track_ids()));
    // 编排行几何（`y` / `height` 与它们的全部消费者）：**唯一**的一份列表在
    // `apply_row_geometry` 里 —— 拖拽手势的每一次 move 也调它，两条路因此不可能漂移。
    apply_row_geometry(ui, view);
    ui.set_track_volumes(strings(&view.track_volumes()));
    ui.set_track_volume_fractions(lengths(&view.track_volume_fractions()));
    ui.set_track_pans(strings(&view.track_pans()));
    ui.set_track_mutes(booleans(&view.track_mutes()));
    ui.set_track_solos(booleans(&view.track_solos()));
    ui.set_scene_names(strings(&view.scene_names()));
    ui.set_section_names(strings(&view.section_names()));
    ui.set_clip_ulids(strings(&view.clip_ulids()));
    ui.set_clip_labels(strings(&view.clip_labels()));
    ui.set_clip_positions(lengths(&view.clip_positions()));
    ui.set_clip_widths(lengths(&view.clip_widths()));
    ui.set_section_positions(lengths(&view.section_positions()));
    ui.set_section_widths(lengths(&view.section_widths()));
    ui.set_bar_positions(lengths(&view.bar_positions));
    // `[ROAD-M3-002 / BASELINE-003]` **视口裁剪**：只注入可见窗口内的音符。
    // 六个平行数组（ulids / velocities / positions / widths / ys / rows）必须共用**同一**索引集，
    // 否则语义 ID 与力度会和几何错位 —— 所以用 `visible_notes` 一次取走，而不是各数组各裁一遍。
    // 位置**相对视口**（投影侧已减去 `scroll_x`）⇒ `.slint` 不做位置算术, 事件路径传 0.0。
    let visible = view.visible_notes(scroll_x, viewport_width);
    // 选中标志与其它数组**同一索引集**（第七个）：每次注入都从界面上的 `selected-ulids` 重算,
    // 因此滚动/撤销重注入后不会错位（账本第 504 轮）。
    // `ModelRc` 的迭代要 `Model` trait 在作用域内；构造要 `VecModel`（`Vec` 不直接 `Into<ModelRc>`）。
    let selected_ulids: Vec<String> = {
        use slint::Model as _;
        ui.get_selected_ulids()
            .iter()
            .map(|id| id.to_string())
            .collect()
    };
    let flags = crate::input::flags_for_ids(&selected_ulids, &visible.ulids);
    ui.set_note_selected(slint::ModelRc::new(slint::VecModel::from(flags)));
    // 把偏移**留在界面对象上**：撤销刷新要复用同一个值（账本第 200/201 轮）。
    ui.set_roll_scroll_x(scroll_x);
    // `[UI-NOTE-001]` 步骤 ①：把裁剪窗口换算成 tick 上下界并发布（音高上下界待做, 见账本第 201 轮）。
    let (min_tick, max_tick) = view.visible_tick_range(scroll_x, viewport_width);
    ui.set_roll_min_tick(tick_to_i32_saturating(min_tick));
    ui.set_roll_max_tick(tick_to_i32_saturating(max_tick));
    // 纵向：泳道数 16 与 `piano_roll.slint` 的 `for lane_index in 16` 一致（第 202 轮记录了它的来源问题）。
    let (min_pitch, max_pitch) = view.visible_pitch_range(ROLL_LANE_COUNT);
    ui.set_roll_min_pitch(i32::from(min_pitch));
    ui.set_roll_max_pitch(i32::from(max_pitch));
    ui.set_note_ulids(strings(&visible.ulids));
    ui.set_note_velocities(lengths(&visible.velocities));
    ui.set_note_positions(lengths(&visible.positions));
    ui.set_note_widths(lengths(&visible.widths));
    ui.set_note_ys(lengths(&visible.ys));
    // 轨道色标：解析 / 回退都在投影层完成，这里只转成 Slint 的 `Color`。
    ui.set_track_colors(colors(&view.track_colors()));
    ui.set_track_color_labels(strings(&view.track_color_labels()));
    // 自动化泳道：9 个平行数组全部来自 `automation.rs` 的投影。顶点已经是逻辑像素、
    // 数值已经是格式化文本，因此这里只做"数组搬运"——`.slint` 侧零算术。
    ui.set_automation_lane_target_keys(strings(&view.automation_lane_target_keys()));
    ui.set_automation_lane_track_indexes(integers(&view.automation_lane_track_indexes()));
    ui.set_automation_lane_labels(strings(&view.automation_lane_labels()));
    ui.set_automation_lane_axis_labels(strings(&view.automation_lane_axis_labels()));
    ui.set_automation_lane_read_enabled(booleans(&view.automation_lane_read_enabled()));
    ui.set_automation_lane_badges(strings(&view.automation_lane_badges()));
    apply_master(ui, view);
    // 电平：先重置成"与当前工程等长的静音"，再由 `apply_meters` 填真实读数（见模块文档）。
    apply_meters(ui, &silent_snapshot(view));
}

/// **行几何**（`ADR-0004` S0/S1）的全部注入数组 —— [`apply_view`] 的**子集**，
/// 而且列表**只有这一份**：[`apply_view`] 自己调用的就是这个函数。
///
/// ## 为什么需要一个子集（它不是"第二个注入点"）
///
/// 7 个数组只在这里列出来，`apply_view` 不再各写一遍 ⇒ 不存在"两份必须一致的列表"。
/// 它单独存在的理由是**指针抓取**，实测换来的：
///
/// - [`apply_view`] 会把 repeater 的**模型**（`track-names` / `clip-ulids` /
///   `automation-lane-target-keys`）换成**新的** `ModelRc`，Slint 因此**重建**那些条目
///   —— 包括正在被拖拽的那个轨道头 `TouchArea`。条目一被重建，指针抓取就没了：
///   实测连发第二次 `ui/dispatch_pointer_move` 时 `.slint` 收不到事件，行高停在第 1 个像素；
/// - 本函数只写条目**内部**用到的属性，属性变化只触发布局、不重建条目 ⇒ 抓取活到手势结束。
///
/// 于是分工是：**高频的手势**（每移动 1 像素一次）走本函数；**完整**重投影（换工程 /
/// 撤销 / 打开工程）走 [`apply_view`]。两条路给出的几何由判据
/// `the_row_geometry_subset_agrees_with_the_full_injection` 逐位对账（把新的行几何派生物
/// 加进 `apply_view` 却忘记加到这里时，那条判据会红）。
///
/// 7 个数组就是行几何的**全部**消费者：包头 / 车道（`track-ys` / `track-heights`）、
/// 剪辑（`clip-ys` / `clip-heights`）、自动化带（`automation-lane-band-ys` /
/// `-band-heights`）与自动化折线（`automation-path-commands` —— 顶点是逐像素的逻辑坐标，
/// 行高一变它就得重算）。
pub fn apply_row_geometry(ui: &MainWindow, view: &ViewState) {
    // `.slint` 直接画注入值、不做 `42px + 56px * i`（`ADR-0004` S0 的前缀和）。
    ui.set_track_ys(lengths(&view.track_ys()));
    ui.set_track_heights(lengths(&view.track_heights()));
    // 剪辑的行几何同源：由所在行的 `RowGeometry` 给出（旧版 `.slint` 自己乘 `clip-lanes`）。
    ui.set_clip_ys(lengths(&view.clip_ys()));
    ui.set_clip_heights(lengths(&view.clip_heights()));
    // 自动化带与折线：带高 = (行槽高 − 2 × 内缩) / 泳道数，顶点 y 也出自同一份行几何。
    ui.set_automation_lane_band_ys(lengths(&view.automation_lane_band_ys()));
    ui.set_automation_lane_band_heights(lengths(&view.automation_lane_band_heights()));
    ui.set_automation_path_commands(strings(&view.automation_path_commands()));
}

/// 轨道高度的**会话/视图态**读入口（`ADR-0004` S1 / Q4-A）—— **唯一**的一处
/// "窗口上的三个属性 → 投影参数"的转换。
///
/// 数据流（与卷帘偏移 `roll-scroll-x` 同款："宿主拥有、重新注入"）：
///
/// ```text
/// MainWindow 的 track-height-* 三个 in-out 属性        ← 宿主/界面写（set_* 两个函数）
///   └─ host::track_height_layout(ui)                   ← 本函数（唯一读点）
///        └─ ViewState::from_project_with_layout(...)   ← 投影（唯一的 clamp）
///             └─ host::apply_view(ui, view, ..)        ← 行几何注入 track-ys / track-heights
/// ```
///
/// 属性缺失 / 空数组 ⇒ [`TrackHeightLayout::default`]（乘子 100、无覆盖）⇒ 默认几何
/// **逐位**等于 S0（判据 `default_layout_geometry_is_bit_for_bit_the_s0_geometry`）。
/// 因此"没有任何人碰过高度"这条路径不引入任何渲染差异。
///
/// 它**不是**第二份权威：布局只影响**投影输入**，工程（`Domain` / `YebanProjectV1`）
/// 一位没动；反方向的写仍然是 `Op` + `apply`（`ADR-0005`）。
#[must_use]
pub fn track_height_layout(ui: &MainWindow) -> TrackHeightLayout {
    TrackHeightLayout::from_view_state(
        &read_strings(&ui.get_track_height_override_ids()),
        &read_lengths(&ui.get_track_height_override_pxs()),
        ui.get_track_height_percent(),
    )
}

/// 把一份布局写回窗口的三个属性（**唯一**的写点）。
///
/// 覆盖表按身份键序（`BTreeMap`）落成两个平行数组：顺序是确定的（跨进程同一结果），
/// 且总与 [`track_height_layout`] 的解读一致（`ids[i]` ↔ `pxs[i]`）。
fn write_track_height_layout(ui: &MainWindow, layout: &TrackHeightLayout) {
    let ids: Vec<String> = layout.overrides().keys().cloned().collect();
    #[allow(clippy::cast_precision_loss)] // ≤ u32：Slint 的 `length` 就是 f32
    let pxs: Vec<f32> = layout.overrides().values().map(|px| *px as f32).collect();
    ui.set_track_height_override_ids(strings(&ids));
    ui.set_track_height_override_pxs(lengths(&pxs));
    ui.set_track_height_percent(i32::try_from(layout.percent()).unwrap_or(i32::MAX));
}

/// 设置一条轨道的**每轨基准高**（整数逻辑像素）—— S1 唯一的"设高度"入口。
///
/// 契约（每条都可判据）：
///
/// - **按身份**：`track_id` 是轨道的 26 字符 `EntityId` 规范文本（与 `TrackView::id`
///   同一个身份）。不是视图下标 ⇒ 插/删轨道不会把高度错配到别的轨道；
/// - `px == 0` ⇒ **删除**该轨覆盖（回到 [`crate::bridge::DEFAULT_TRACK_HEIGHT_PX`]）并返回"是否真的变了"。
///   0 是退化值（`ADR-0004` Q2：损坏，不是布局选择），在这里被当成"取消覆盖"而不是
///   "高 0 的行"；
/// - 返回值 `false` = 布局**一位没变**（同一个值 / 未知身份 / 没有可删的覆盖）⇒
///   调用方不必重投影、不必重注入；
/// - 本函数**只改视图态**，不重投影：重投影由调用方走既有路径
///   （GUI：`refresh_undo_window`；测试/执行面：`live_surface` 的 `apply_project`）,
///   因为"什么工程"不属于视图态（本函数拿不到也不该拿）。
///
/// 不发明任何新的 JSON-RPC 错误码（`D25`）：这里没有错误分支 —— 身份不合法/值没变
/// 就是 `false`，"改不动"与"没改"在返回值上不可区分是有意的（**幂等**优先）。
pub fn set_track_height_override(ui: &MainWindow, track_id: &str, px: u32) -> bool {
    let mut layout = track_height_layout(ui);
    if !layout.set_track_px(track_id, px) {
        return false;
    }
    write_track_height_layout(ui, &layout);
    true
}

/// 设置**全局高度缩放级**（百分比，100 = 不缩放）—— S1 的第二个旋钮。
///
/// 返回"是否真的变了"（同 [`set_track_height_override`]）。夹紧**不在这里**：
/// 任何 `percent` 都写进视图态，有意义的上/下界由投影
/// （[`crate::bridge::effective_track_height_px`]）在执行时施加 —— 边界只有一处
/// （`ADR-0004` Q2-B）。因此 `percent = 0` 不是错误：它表示"缩到最小"（有效高被夹到
/// [`crate::bridge::MIN_TRACK_HEIGHT_PX`]），不是一个 0 高的行。
pub fn set_track_height_percent(ui: &MainWindow, percent: u32) -> bool {
    let mut layout = track_height_layout(ui);
    if !layout.set_percent(percent) {
        return false;
    }
    write_track_height_layout(ui, &layout);
    true
}

/// 拖拽手势的**像素换算**（纯函数，可判据）：把"从按下点开始的指针纵向位移"换算成这一轨的
/// **基准**行高（整数逻辑像素）。
///
/// ## 三条口径（每条都可被注入打红）
///
/// 1. **向上拖 = 变高**：`delta_px = 按下时的 y − 当前的 y`（Slint 的 y 轴向下），
///    因此 `delta_px > 0` 表示"把这一行拖高"；
/// 2. **乘子的逆**：视图态里住的是**基准**像素，而用户在屏幕上看到的是**有效**像素
///    （`有效 = clamp(基准 × 百分比 / 100)`，`ADR-0004` Q2-B / Q1-C）⇒ 基准增量取
///    `位移 × 100 / 百分比`，这样"拖 n 像素，画面就长 n 像素"在**任何**乘子下都成立。
///    本函数**不夹上界**：夹紧仍只在 [`crate::bridge::effective_track_height_px`] 里发生
///    一次（边界只有一处）。`percent == 0` 是"缩到最小"的退化设置 —— 那时乘子不可反演
///    （任何基准都映到 `MIN_TRACK_HEIGHT_PX`，手势在画面上本来就看不见），因此按
///    "当作 100"处理，而**不是**除以 0；
/// 3. **整数**：结果四舍五入到整数像素（与 `[length] → 基准像素` 的读取口径一致，
///    `ADR-0004` D28）。下界取 `1` 而**不是** `0`：[`TrackHeightLayout::set_track_px`]
///    把 `0` 当作"删除这条覆盖"（`ADR-0004` Q2 的退化值处置）—— 手势**不借用**那个语义，
///    因此它永远不会把一次拖拽变成"这条轨回到默认高"。真正的行高下界仍由投影给出
///    （[`crate::bridge::MIN_TRACK_HEIGHT_PX`]）。
#[must_use]
pub fn dragged_track_base_px(start_px: u32, delta_px: f32, percent: u32) -> u32 {
    #[allow(clippy::cast_precision_loss)] // ≤ u32：Slint 的 `length` 与它同为 f32
    let start = start_px as f32;
    let scaled = if percent == 0 {
        delta_px
    } else {
        #[allow(clippy::cast_precision_loss)]
        let percent = percent as f32;
        delta_px * 100.0 / percent
    };
    let target = (start + scaled).round();
    if !target.is_finite() {
        // 非有限位移（NaN / ±∞）**不**改写这一轨的高度：原地返回合法的起点值。
        return start_px.max(1);
    }
    // `f32 → u32` 的 `as` 转换在 Rust 里**饱和**（1.45 起），NaN 已在上面排除。
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let px = target.max(1.0) as u32;
    px.max(1)
}

/// 第 `index` 条轨道的**身份**（[`crate::bridge::TrackView::id`]，26 字符 `EntityId` 文本）。
///
/// 身份从**注入的** `track-ids` 读（与行几何同一次注入），因此换工程之后不会拿旧身份
/// 去改新工程。下标越界 / 身份为空 ⇒ `None`（调用方什么都不做，而不是猜第一条轨道）。
fn track_id_at(ui: &MainWindow, index: i32) -> Option<String> {
    let index = usize::try_from(index).ok()?;
    read_strings(&ui.get_track_ids())
        .into_iter()
        .nth(index)
        .filter(|id| !id.is_empty())
}

/// 行高视图态变了之后的**重投影**（**唯一**一处，拖拽的每一次 move 与 `Escape` 的收尾都用它）。
///
/// 与 `wire_roll_edit` 同一手法（那里是"工程从端口来、视图态从窗口读回"）：
/// [`UndoPort::try_project`] 给出此刻**权威**的工程，[`track_height_layout`] 给出视图态，
/// 于是"重投影"不会把刚设好的行高清零，也不会拿一个装配期的快照去画换过工程之后的界面。
///
/// 端口此刻没有活跃工程（控制面关掉了工程）⇒ **什么都不做**：视图态已经写进去了，
/// 下一次重投影会带上它 —— 不 panic、也不编一帧。
/// 行高视图态变了之后的**重投影**（**唯一**一处，拖拽的每一次 move 与 `Escape` 的收尾都用它）。
///
/// 与 `wire_roll_edit` 同一手法（那里是"工程从端口来、视图态从窗口读回"）：
/// [`UndoPort::try_project`] 给出此刻**权威**的工程，[`track_height_layout`] 给出视图态，
/// 于是"重投影"不会把刚设好的行高清零，也不会拿一个装配期的快照去画换过工程之后的界面。
///
/// 写的是 [`apply_row_geometry`]（**不是** `apply_view`）：拖动是每像素一次的高频写入，
/// 而完整注入会重建 repeater 条目、把正在拖拽的那个 `TouchArea` 连同指针抓取一起丢掉
/// （实测：第二次 `ui/dispatch_pointer_move` 到不了界面）。两者的几何由判据逐位对账。
///
/// 端口此刻没有活跃工程（控制面关掉了工程）⇒ **什么都不做**：视图态已经写进去了，
/// 下一次重投影会带上它 —— 不 panic、也不编一帧。
fn reproject_track_heights(ui: &MainWindow, port: &UndoPort) {
    let Some(project) = port.try_project() else {
        return;
    };
    let layout = track_height_layout(ui);
    match ViewState::from_project_with_layout(&project, &layout) {
        Ok(view) => apply_row_geometry(ui, &view),
        Err(error) => {
            // 投影失败**出声**：视图态在内存里已经变了，但这一帧画不出来。
            eprintln!("[yeban-app] 轨道高度重投影失败: {error}");
        }
    }
}

/// **轨道高度拖拽手势的宿主侧接线**（`ADR-0004` S1 的纵向入口，唯一实现）。
///
/// ## 数据流（三条回调，全部落在既有 setter 上）
///
/// ```text
/// arrangement_view.slint 的轨道头 TouchArea
///   --track-height-grab(index, mouse-y)----> 记下"这一拖从哪一行、哪个基准高、哪个 y 开始"
///   --track-height-drag(index, mouse-y)----> dragged_track_base_px(基准, 位移, 乘子)
///                                              └─ host::set_track_height_override（唯一 setter）
///                                                   └─ 重投影 → apply_view → track-ys / track-heights
///   --track-height-release(index)----------> 手势状态清零（= 收尾）
/// ```
///
/// ## 三条收尾路径（一条都不能少）
///
/// | 路径 | 谁发出 | 语义 |
/// | :--- | :--- | :--- |
/// | 松手 | `.slint` 的 `PointerEventKind.up` | 这一拖算数 |
/// | 指针离开窗口 / 控件被禁用 | 上游导出的 `PointerEventKind.cancel` | 这一拖算数（最后位置） |
/// | `Escape` | `Action::Cancel` → [`cancel_track_height_drag`] | **取消**这一拖（写回起点高度） |
///
/// 三条都汇到 [`end_track_height_drag`]：**手势状态在那一处清零**，因此不存在"拖到一半
/// 松在窗口外 ⇒ 下一次移动继续改高度"这种粘住的状态（判据
/// `a_stray_pointer_move_without_a_grab_never_touches_the_row_geometry` 钉住这一点）。
///
/// 本函数只做接线，不做像素算术（算术在 [`dragged_track_base_px`] 里，可单独判据）。
pub fn wire_track_height_drag(ui: &MainWindow, port: &Rc<UndoPort>) {
    let weak = ui.as_weak();
    ui.on_track_height_grab({
        let weak = weak.clone();
        move |index, pointer_y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            begin_track_height_drag(&ui, index, pointer_y);
        }
    });
    ui.on_track_height_drag({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move |index, pointer_y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Some((track_id, px)) = track_height_drag_target_px(&ui, index, pointer_y) else {
                // 没有进行中的手势 / 下标不是开始那一行 ⇒ **什么都不做**。
                // 这是"粘住的拖拽态"的第一道防线：一个孤立的 move 永远改不动几何。
                return;
            };
            if !set_track_height_override(&ui, &track_id, px) {
                // 同一个值（例如指针在同一像素格内抖动）⇒ 不必重投影。
                return;
            }
            reproject_track_heights(&ui, &port);
        }
    });
    ui.on_track_height_release({
        let weak = weak.clone();
        move |_index| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // 松手 / 离开窗口都是"这一拖算数"：值已经在视图态里，这里只收尾。
            end_track_height_drag(&ui, false);
        }
    });
}

/// 开始一次拖拽：记下这一拖的**行身份 / 起点基准高 / 起点指针 y**。
///
/// 已经有一次手势在手时**重新开始**（而不是叠加）：新的一次按下覆盖旧状态，
/// 因此"上一次没收尾"最多影响这一次拖拽，不会累积成一个改不掉的偏移。
fn begin_track_height_drag(ui: &MainWindow, index: i32, pointer_y: f32) {
    let Some(track_id) = track_id_at(ui, index) else {
        return;
    };
    // 起点基准高 = **当前**布局里这一轨的基准（有覆盖就用覆盖）—— 于是"再拖一次"
    // 从用户看到的高度继续，而不是从默认 56 重新开始。
    let start_px = track_height_layout(ui).base_px(&track_id);
    ui.set_track_height_drag_track(track_id.into());
    ui.set_track_height_drag_start_px(i32::try_from(start_px).unwrap_or(i32::MAX));
    ui.set_track_height_drag_start_y(pointer_y);
    // `active` 最后写：读者要么看到"还没开始"，要么看到一份**完整**的起点状态。
    ui.set_track_height_drag_active(true);
}

/// 一次 move 的目标基准像素（`None` = 这个 move 不属于当前手势 ⇒ 调用方什么都不做）。
fn track_height_drag_target_px(
    ui: &MainWindow,
    index: i32,
    pointer_y: f32,
) -> Option<(String, u32)> {
    if !ui.get_track_height_drag_active() {
        return None;
    }
    let track_id = ui.get_track_height_drag_track().to_string();
    // 手势只作用于**按下那一行**：另一个下标发来的 move（多指 / 事件串扰）一律忽略。
    if track_id_at(ui, index).as_deref() != Some(track_id.as_str()) {
        return None;
    }
    let delta_px = ui.get_track_height_drag_start_y() - pointer_y;
    let start_px = u32::try_from(ui.get_track_height_drag_start_px()).unwrap_or(0);
    let percent = u32::try_from(ui.get_track_height_percent()).unwrap_or(0);
    Some((track_id, dragged_track_base_px(start_px, delta_px, percent)))
}

/// **一次拖拽手势的收尾**（松手 / 指针离开窗口 / `Escape` 三条路径的唯一落点）。
///
/// `revert == true` 时把基准高写回**按下时**的值 —— 那是 `Escape` 的语义（取消这一拖）；
/// `revert == false` 时保留拖出来的值（松手 / 离开窗口）。
///
/// 无论哪条路径，四个手势状态位**一律清零**，返回 `true` 表示"确实有手势在手、
/// 已经收尾"。没有手势在手时返回 `false` 且一位不动 —— 调用方（`Action::Cancel`）
/// 因此可以如实说"这一键与 DAW 无关"（`Escape` 在别的语义下照旧放行）。
///
/// ## 为什么下界是 `start_px > 0` 才回写
///
/// 手势起点记录的是 [`TrackHeightLayout::base_px`] 的读数，它恒 `> 0`
/// （`base_px` 的兜底是 `DEFAULT_TRACK_HEIGHT_PX`）；`0` 只可能来自"这个属性被外力
/// 改成了 0" ⇒ 那时**不**回写，免得把 `0`（= 删除覆盖）当成一次高度设置。
pub fn end_track_height_drag(ui: &MainWindow, revert: bool) -> bool {
    if !ui.get_track_height_drag_active() {
        return false;
    }
    let track_id = ui.get_track_height_drag_track().to_string();
    let start_px = u32::try_from(ui.get_track_height_drag_start_px()).unwrap_or(0);
    ui.set_track_height_drag_active(false);
    ui.set_track_height_drag_track("".into());
    ui.set_track_height_drag_start_px(0);
    ui.set_track_height_drag_start_y(0.0);
    if revert && start_px > 0 && !track_id.is_empty() {
        set_track_height_override(ui, &track_id, start_px);
    }
    true
}

/// `Escape`（`Action::Cancel`）在**有手势在手**时的收尾：取消这一拖（写回起点高度）并重投影。
///
/// 返回 `false` = 此刻没有进行中的手势 ⇒ 这一键与 DAW 无关，照旧放行
/// （与 `Escape` 接线之前的行为**逐位相同**）。
///
/// `port` 是 [`UndoPort`]（生产路径上就是 `wire_keys` 拿到的那一个）：取消之后必须把
/// **几何**也拉回起点，否则用户看到的是一帧"按了 Esc 没反应"的界面。
fn cancel_track_height_drag(ui: &MainWindow, port: Option<&Rc<UndoPort>>) -> bool {
    if !end_track_height_drag(ui, true) {
        return false;
    }
    if let Some(port) = port {
        reproject_track_heights(ui, port);
    }
    true
}

/// `[string]` 属性 → `Vec<String>`（读回视图态；写方向是 [`strings`]）。
fn read_strings(model: &ModelRc<SharedString>) -> Vec<String> {
    use slint::Model as _;
    model.iter().map(|value| value.to_string()).collect()
}

/// `[length]` 属性 → `Vec<f32>`（读回视图态；写方向是 [`lengths`]）。
fn read_lengths(model: &ModelRc<f32>) -> Vec<f32> {
    use slint::Model as _;
    model.iter().collect()
}

/// 主控通道条的**投影**字段（名字 / 音量 / 声相 / 静音 / 独奏 / 色标）。
///
/// 工程没有主总线（`view.master == None`，例如空工程）时用中性默认值：
/// 名字 `"Master"`、其余为空 / 居中 / 假 / 文档回退色。**不 panic、不留上一次的值**。
fn apply_master(ui: &MainWindow, view: &ViewState) {
    let Some(master) = view.master.as_ref() else {
        ui.set_master_name("Master".into());
        ui.set_master_volume_display("".into());
        ui.set_master_volume_fraction(0.0);
        ui.set_master_pan("C".into());
        ui.set_master_mute(false);
        ui.set_master_solo(false);
        ui.set_master_color(to_color(DEFAULT_TRACK_COLOR));
        ui.set_master_color_label("".into());
        return;
    };
    ui.set_master_name(master.name.clone().into());
    ui.set_master_volume_display(master.volume_display.clone().into());
    ui.set_master_volume_fraction(master.volume_fraction);
    ui.set_master_pan(master.pan_display.clone().into());
    ui.set_master_mute(master.mute);
    ui.set_master_solo(master.solo);
    ui.set_master_color(to_color(master.color_rgb));
    ui.set_master_color_label(master.color_hex.clone().into());
}

/// 把一次电平抽帧的结果写进界面（`[ARCH-UI-002]` 的"更新 Slint Properties"）。
///
/// 只写**电平**那几个属性，不碰任何投影字段 —— 60Hz 的写入面越小，脏矩形越局部
/// （`[ARCH-UI-001]` 保留模式 + 局部脏矩形）。
///
/// 长度契约：`snapshot.tracks` 与 `track-names` **必须等长**。等长由构造方保证
/// （[`crate::meters::snapshot`] / [`crate::meters::silent_snapshot`] 都按 `view.tracks` 生成），
/// 这里用 `debug_assert` 把契约写在代码里；不 panic 是因为"少一条电平"不该让整个界面崩掉。
pub fn apply_meters(ui: &MainWindow, snapshot: &MeterSnapshot) {
    // UFCS（而不是 `model.row_count()`）：`Model` trait 不在本文件的 import 里，
    // 显式写出路径与 `main.rs` 的 `slint::ComponentHandle::run(&ui)` 是同一个理由 ——
    // 既拿到方法，又不引入一个可能变成 unused import 的 trait。
    debug_assert_eq!(
        slint::Model::row_count(&ui.get_track_names()),
        snapshot.tracks.len(),
        "电平数组必须与轨道数组等长（否则 .slint 按下标取会串到别的轨道）"
    );
    ui.set_track_meter_peaks(strings(&snapshot.peak_labels()));
    ui.set_track_meter_rmss(strings(&snapshot.rms_labels()));
    ui.set_track_meter_levels(lengths(&snapshot.levels()));
    ui.set_master_meter_peak(snapshot.master_peak_label().into());
    ui.set_master_meter_rms(snapshot.master_rms_label().into());
    ui.set_master_meter_level(snapshot.master_level());
}

/// 把**引擎的**走带读数注入界面（`playing` 显示态 + 时间码）。
///
/// 这是全仓库**唯一**写 `playing` 的地方 —— 界面的 `toggle-play` 处理器**不再自己翻转**
/// 那个属性（上一版 `app.slint` 里的 `root.playing = !root.playing;` 是"UI 自造状态"，
/// 会让"显示"与"引擎"各说各话）。数据流因此是单向的：
///
/// ```text
/// TransportMirror(原子读数) --(host::apply_transport)--> MainWindow.playing / .timecode
/// ```
///
/// `timecode` 用的是引擎读数（[`TransportReading::position_ticks`]）**配上投影的拍号网格**：
/// 位置来自引擎，单拍 / 单小节的 tick 数来自工程投影（[`ViewState::timecode_grid`]，
/// 由 [`apply_view`] 钉在窗口上）。界面侧没有任何拍号算术，也没有 4/4 的默认假设。
///
/// ## 写死的拍号是怎么被删掉的（本线的核心修复）
///
/// 上一版这里调用 `timecode_for_ticks(ticks)`，而它的拍号是从
/// `const BEATS_PER_BAR: u64 = 4;` 来的 —— 那行常量的注释写着"`TimeSignature` **没有**被
/// 投影进 `ViewState`（登记为 needs）"，**与事实相反**：`ViewState` 早就带着
/// `time_signature_numerator/denominator`（`bridge.rs`）与 `bar_length_ticks`。
/// 结果是 3/4、6/8 工程的时间码**静默算错**。现在的链路：
///
/// ```text
/// YebanProjectV1 --(bridge::ViewState)--> numerator/denominator --(apply_view)-->
/// MainWindow.timecode-ticks-{beat,bar} --(apply_transport)--> bridge::timecode_for_ticks
/// ```
///
/// 唯一的换算实现在 [`crate::bridge::timecode_for_ticks`]（投影层、零 Slint），
/// 本文件只做"把投影算好的两个整数读出来、把算好的字符串写回去"。
pub fn apply_transport(ui: &MainWindow, reading: TransportReading) {
    ui.set_playing(reading.state.is_running());
    let grid = timecode_grid_of(ui);
    ui.set_timecode(crate::bridge::timecode_for_ticks(grid, reading.position_ticks).into());
}

/// 从窗口上**投影注入的**两个整数重建时间码网格（唯一读点）。
///
/// 两个 `0`（= 模型层拒绝该拍号 ⇒ [`apply_view`] 写的就是 0）以及任何畸形组合
/// 都会退化成 `None`，于是 [`crate::bridge::timecode_for_ticks`] 如实写 `tick N` ——
/// 不会退回写死的 4/4。
fn timecode_grid_of(ui: &MainWindow) -> Option<crate::bridge::TimecodeGrid> {
    crate::bridge::TimecodeGrid::from_injected(
        ui.get_timecode_ticks_beat(),
        ui.get_timecode_ticks_bar(),
    )
}

/// 把 `MainWindow` 的**走带回调**接到引擎（[`crate::engine_host::EngineHost`]）。
///
/// ## 为什么它住在 `host.rs`
///
/// ADR-0001 **D28** 的口径是"注入面只有一处"：界面属性由 [`apply_view`] /
/// [`apply_meters`] 写，而"界面 → 引擎"的走带命令由本函数接线。`main.rs` 与
/// Tier-1 判据（`src/test_port_adapter.rs` 的 `#[path]` 双目标）**共用这一份实现**，
/// 因此"命令行看到的界面"与"CI 断言的界面"接到的是同一个引擎。
///
/// ## 线程与实时安全
///
/// 回调跑在 **UI 线程**：`Rc<RefCell<..>>` 是"单线程内的可变共享"，**不是** RT 锁；
/// 真正的实时侧（`EngineRuntime::process_quantum`）只从无锁 SPSC 出队。
/// 回调里不做任何长阻塞等待 —— 它只发 1–2 条命令并推 1 个量子（微秒级）。
///
/// ## 状态来源
///
/// 每次动作之后都从**引擎读数**回写界面（[`apply_transport`]），因此：
/// - `toggle-play` 触发引擎状态变化，显示态跟着引擎走；
/// - 直接改引擎状态（例如测试里的 `host.play()`）之后调用 [`apply_transport`]，
///   显示态同样跟着变 —— 显示态**没有**自己的状态机。
pub fn wire_transport(ui: &MainWindow, engine: Rc<RefCell<EngineHost>>) {
    let toggle = Rc::clone(&engine);
    // UFCS（而不是 `ui.as_weak()`）：`slint::ComponentHandle` 不在本文件的 import 里，
    // 与 `main.rs` 的 `slint::ComponentHandle::run(&ui)` 是同一个理由 —— 既拿到方法，
    // 又不引入一个可能变成 unused import 的 trait（`-D warnings` 下会直接失败）。
    let toggle_ui = slint::ComponentHandle::as_weak(ui);
    ui.on_toggle_play(move || {
        let reading = toggle.borrow_mut().toggle_play();
        if let Some(ui) = toggle_ui.upgrade() {
            apply_transport(&ui, reading);
        } else {
            // 窗口已经销毁：动作已经发给引擎了（不留半途状态），只是没人可通知。
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
        }
    });

    let stop = Rc::clone(&engine);
    let stop_ui = slint::ComponentHandle::as_weak(ui);
    ui.on_stop(move || {
        // UI 的"停止"按钮的语义（`transport.slint` 的 accessible-label）是
        // **停止并回到起始点** ⇒ 引擎侧是 `Stop` + `SeekTicks(0)` 两条命令。
        let reading = stop.borrow_mut().stop_and_rewind();
        if let Some(ui) = stop_ui.upgrade() {
            apply_transport(&ui, reading);
        } else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
        }
    });
}

/// 把**纯视图态**的四条界面回调接到宿主上（`ADR-0004` 同款口径：视图态、零 schema、重启即失）。
///
/// | 回调 | 唯一写的属性 | 界面侧的可观察后果 |
/// | :--- | :--- | :--- |
/// | `toggle-view` | `arrangement-view` | `ArrangementView` / `SessionView` 的 `visible` 互斥互换 |
/// | `toggle-sidebar` | `sidebar-collapsed` | 左资源栏 240px ↔ 36px 图标导轨（`min/max-width`） |
/// | `toggle-ai-drawer` | `ai-drawer-open` | `compact` 断点下 280px AI 抽屉的 `visible` |
/// | `open-musical-pr` | `musical-pr-open` | `MusicalPrDrawer` 的 `visible`（**只打开**） |
///
/// ## 为什么这四处翻转必须住在这里，而不是 `.slint` 的 `clicked =>` 里
///
/// `arrangement-view` 与 `sidebar-collapsed` 已经有**第二个宿主写者**：
/// [`apply_action`] 的 `Action::ToggleView` / `Action::ShowView` / `Action::ToggleSidebar`
/// 写的就是这两个属性（`ui/switch_main_view` 也写同一个）。若 `.slint` 的转发块自己再翻转
/// 一次，同一个动作就有两个真相源；而"界面**不**自己翻转状态"正是 `playing` 那条已经
/// 付过学费的契约（`ui/app.slint` 的 `toggle-play` 注释、`docs/ledger/feature-alignment.md`
/// 错位 7）。因此本函数是这四条回调的**唯一**落点，组件里只剩"原样转发"。
///
/// ## 为什么这不是"假接线"
///
/// 它**只改视图态**：不碰 `Op`、不碰 [`UndoPort`]、不进 `.yeban`、不动 schema。
/// 这与 [`set_track_height_override`] 是同一档东西（`ADR-0004` S1 / Q4-A）——
/// 视图态本来就可以由宿主拥有，因此没有绕过 `ADR-0005` 的领域权威（那条约束的是**工程内容**）。
///
/// 四条都只用"打开时那个窗口" ⇒ 共用一份 [`slint::Weak`]：强引用若被闭包自己持有，
/// 就形成 `MainWindow → 回调 → MainWindow` 的自环（窗口永不解构）。
///
/// `close`（关抽屉）**不在这里**：`.slint` 的 `close =>` 没有对应的 Rust 回调，
/// 因此那一条仍写在组件里（本函数不发明第五条回调）。
pub fn wire_view_callbacks(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_toggle_view({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            ui.set_arrangement_view(!ui.get_arrangement_view());
        }
    });
    ui.on_toggle_sidebar({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            ui.set_sidebar_collapsed(!ui.get_sidebar_collapsed());
        }
    });
    ui.on_toggle_ai_drawer({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            ui.set_ai_drawer_open(!ui.get_ai_drawer_open());
        }
    });
    ui.on_open_musical_pr({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // **只打开**（本切片的范围纪律）：提案的生成 / 采纳 / 拒绝不在这里 ——
            // 它们要么需要界面侧并不存在的"提案身份"，要么属于工程内容（`ADR-0005`）。
            ui.set_musical_pr_open(true);
        }
    });
}

/// 把界面的**输入法事件源**接到 `[UI-A11Y-002]` 的状态机上（§7.2）。
///
/// ## 接的是什么（上游核验，不是猜的）
///
/// | 环节 | 事实 | 出处 |
/// | :--- | :--- | :--- |
/// | 平台 → Slint | winit 的 `Ime::Preedit` 变成 `KeyEventType::UpdateComposition` | `i-slint-backend-winit-1.18.1/winitwindowadapter.rs:1429` |
/// | Slint → `.slint` | `TextInput.preedit-text` 被写成候选词 | `i-slint-compiler-1.18.1/builtin_elements.rs:2323`（`out property <string> preedit-text`） |
/// | `.slint` → 宿主 | `changed preedit-text` / `changed has-focus` 调本函数的两个回调 | `ui/transport.slint` 的 `bpm-input` |
/// | 宿主 → 状态机 | [`InputContext::begin_composition`] / [`InputContext::end_composition`] / [`InputContext::set_focus`] | 本函数 |
///
/// **不新造状态机**：合成态的唯一载体是 [`InputContext`]（`src/input.rs`，纯 Rust、
/// 22 条判据），本函数只把事件翻成它的三个既有入口。
///
/// ## 为什么"焦点"也要接
///
/// `is_composing` 只有在**文本域聚焦**时才有意义：焦点离开时输入法会把候选词上屏 /
/// 取消，`preedit-text` 被清空 —— 但"清空事件"本身可能因为窗口失活而**永远不来**
/// （`i-slint-core` 的 `FocusOut` 直接 `preedit_text.set(Default::default())`，
/// 而那一刻我们的回调确实会跑；可"平台根本没通知"的情况仍然存在）。
/// `InputContext::set_focus` 自带"离开文本域 ⇒ 合成态清零"这条既有语义
/// （`src/input.rs` 的 `leaving_the_text_field_ends_composition`），因此焦点事件是
/// 合成态的**兜底清零**：不然一次丢事件就会把后续所有单键热键永久吞掉。
pub fn wire_input(ui: &MainWindow, context: Rc<RefCell<InputContext>>) {
    // 合成：preedit 有内容 ⇒ 开始合成；preedit 清空（上屏 / 取消）⇒ 结束合成。
    let composing = Rc::clone(&context);
    ui.on_ime_composition_changed(move |composing_now| {
        let mut context = composing.borrow_mut();
        if composing_now {
            context.begin_composition();
        } else {
            context.end_composition();
        }
    });
    // 焦点：文本域 ⇄ 画布。`set_focus` 会在离开文本域时顺手结束合成态。
    let focused = context;
    ui.on_ime_focus_changed(move |focused_now| {
        let mut context = focused.borrow_mut();
        context.set_focus(if focused_now {
            Focus::TextInput
        } else {
            // 本装配里只有 BPM 一个敲入控件，因此"失焦"的落点就是画布
            // （`InputContext::new()` 的启动态）。将来有第二个敲入控件时，
            // 这里必须改成"问焦点系统当前焦点是谁"，而不是假定画布。
            Focus::MainCanvas
        });
    });
}

/// 把界面的**键盘事件源**接到动作上（`N2` 裁决 **(1)**：GUI 绑**逻辑键**）。
///
/// ## 为什么是逻辑键
///
/// Slint 的公开按键事件没有物理码（`KeyEvent { text, modifiers, repeat }`），而
/// `[UI-A11Y-001]` 的扫描码表要物理键身份 ⇒ GUI 路径只能绑逻辑键。裁决与残余见
/// `docs/ledger/open-questions.md` 问题 1 与 `docs/ledger/app-projection-notes.md` 的 `N2` 行。
/// 物理码那条路**一位没改**：`input::PhysicalKey` + `InputContext::resolve` 仍是无头端口
/// （`live_surface.rs` 的 `physical_key_of`、`undo::perform_key`）的判据入口。
///
/// ## 链路（唯一的一条）
///
/// ```text
/// ui/app.slint 的 key-handler(FocusScope).key-pressed
///   --callback key-action(text, shift, control, alt, meta)--> 本函数
///   --LogicalKey::from_text--> InputContext::resolve_logical   （与物理入口同一张策略表）
///   --Resolution::Action--> apply_action                        （界面行为的唯一落点）
/// ```
///
/// `undo` 是**可选的**：判据侧的装配（`build_live_ui_with`）没有撤销会话，那时撤销族
/// 快捷键**如实不消费**（`.slint` 收到 `reject`），而不是假装处理了却什么都不做。
/// 生产路径（`main.rs`）传入真的 `UndoPort`，于是 `Cmd+Z` 经 `undo::dispatch_key`
/// 这个**唯一下发点**落到 `CommitGraph`（ADR-0001 **D45** 的那一半）。
///
/// ## 返回值（`.slint` 的 `EventResult`）
///
/// `true` = 这一键被 DAW 消费（`accept`，不再冒泡）；`false` = 放行给焦点系统 / 文本控件
/// （`reject`）。`ConsumedByIme` 也返回 `true` —— §7.2 要求合成态下**彻底拦截**，
/// 这是 GUI 侧的兜底（正常情况下敲入控件自己就会吃掉这些键）。
pub fn wire_keys(ui: &MainWindow, context: Rc<RefCell<InputContext>>, undo: Option<Rc<UndoPort>>) {
    let weak = slint::ComponentHandle::as_weak(ui);
    ui.on_key_action(move |text, shift, ctrl, alt, meta| -> bool {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return false;
        };
        // 一次按键只携带一个字符；非按键文本（多字符 / 裸修饰键 / 尚未接线的键）
        // 一律 `None` ⇒ 不消费，交给焦点系统。
        let Some(key) = LogicalKey::from_text(text.as_str()) else {
            return false;
        };
        let modifiers = Modifiers {
            ctrl,
            shift,
            alt,
            meta,
        };
        match context.borrow().resolve_logical(key, modifiers) {
            Resolution::PassThrough => false,
            Resolution::ConsumedByIme => true,
            Resolution::Action(action) => apply_action(&ui, undo.as_ref(), action),
        }
    });
}

/// **用户按"保存"的接线**（`ROAD-M4-008` 选项 (a) 剩下的"保存 UI"缺口）。
///
/// 它把两件事接起来，别的一件都不做：
///
/// 1. `.slint` 的 `save-project` 回调（`ui/transport.slint` 的"保存"按钮，
///    `accessible-id: "transport-save-button"`）→ [`crate::save_action::dispatch_save`]；
/// 2. 回执 → **界面状态文本**（`save-status` 属性；同一个属性由 `status_bar.slint`
///    以 `accessible-id: "status-bar-save-status"` 画出来，因此屏读器与判据都读得到）。
///
/// ## 为什么状态写进界面属性而不是只打日志
///
/// 本仓库最忌讳的失败模式是"保存失败而用户以为成功"。因此无论成功还是被拒，
/// 那条消息都**必须**留在界面上：`save_status` 是屏读器可达的（`accessible-label`
/// 由 `.slint` 直接取这个属性），也是判据可断言的（`ui.get_save_status()`）。
/// 拒绝的原因（谁占着 `.yeban.lock`、会话是不是只读）由
/// [`crate::save_action::dispatch_save`] 写在消息里，这里**不再做第二次分类**。
///
/// 工程从哪来：[`SaveRequest::project`]。有权威时它**不被使用**（字节由权威自己产出，
/// 见 `save_action` 模块文档），因此"界面缓存陈旧 ⇒ 写错内容"不可能发生。
pub fn wire_save(
    ui: &MainWindow,
    port: SaveStatus,
    #[cfg(feature = "in-process-mcp")] authority: Option<crate::mcp_mount::ProjectAuthorityHandle>,
    #[cfg(not(feature = "in-process-mcp"))]
    #[cfg_attr(not(feature = "in-process-mcp"), allow(unused_variables))]
    authority: Option<crate::save_action::AuthorityHandle>,
) {
    let weak = ui.as_weak();
    ui.on_save_project(move || {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        // 默认构建里 `authority` 是那个**不可构造**的占位类型 ⇒ 这里恒为 `None`，
        // 策略表里"没有控制面"那一行就是唯一的行（没有条件编译出来的第二条策略）。
        #[cfg(feature = "in-process-mcp")]
        let authority = authority.as_ref();
        #[cfg(not(feature = "in-process-mcp"))]
        let authority = None;
        let request = crate::save_action::SaveRequest {
            target: port.target.clone(),
            project: port.project.clone(),
        };
        let outcome = crate::save_action::dispatch_save(&request, authority);
        apply_save_outcome(&ui, &outcome);
    });
}

/// 一次保存的**界面回执**：把 [`crate::save_action::SaveOutcome`] 写进状态文本。
///
/// 成功与失败用**同一**条路径写界面（只有文字与颜色不同）—— 于是"失败时忘了刷新界面"
/// 这种错法不存在；而"成功时说的话"与"失败时说的话"都能在控件树里读到。
fn apply_save_outcome(ui: &MainWindow, outcome: &crate::save_action::SaveOutcome) {
    let (status, ok) = match outcome {
        crate::save_action::SaveOutcome::Saved {
            path,
            bytes,
            by_authority,
        } => (
            format!(
                "已保存 {bytes} 字节 → {}（{}）",
                path.display(),
                if *by_authority {
                    "经控制面会话写盘（唯一写者）"
                } else {
                    "本地原子写"
                }
            ),
            true,
        ),
        crate::save_action::SaveOutcome::Failed { message, .. } => {
            (format!("保存失败：{message}"), false)
        }
    };
    ui.set_save_status(status.into());
    ui.set_save_succeeded(ok);
}

/// `Action` 是否有**可作用的实现** —— 宿主对此的**唯一**陈述。
///
/// 这张名单回答的是"按下去会不会真的发生一件事", 因此它是**用户可见承诺**的事实源：
/// `--print-shortcuts` 的 [`crate::cli::UNIMPLEMENTED_MARKER`] 标记、下面 `apply_action`
/// 的拒绝分支都从这里读。`tests/cli_contract.rs` 的判据
/// `shortcut_table_status_matches_the_resolution_and_host_pipeline` 把快捷键表逐行与它
/// 对账 —— 表与行为因此不可能各说各话。
///
/// 返回 `false` 的六条（`Duplicate` / `ZoomToSelection` / `ZoomToFit` /
/// `AuditionMain` / `AuditionProposal` / `AcceptAiSuggestion`）是模型侧的编辑 /
/// 视口语义还没落地的动作。**不消费**它们是刻意的：把键吞掉却什么都不做，比不处理更糟 ——
/// 用户会以为"这个功能坏了"，而日志里没有任何东西能解释。
///
/// `DeleteSelection` 曾在这张名单里；它现在有落地实现（下面的 `apply_action` 分支），
/// 因此**同时**从这张名单与 `cli.rs` 的快捷表 `implemented` 标记里移出 ——
/// B11b（`tests/cli_contract.rs`）把这两处与 `--print-shortcuts` 的渲染逐条对账。
#[must_use]
pub fn action_has_implementation(action: Action) -> bool {
    !matches!(
        action,
        Action::Duplicate
            | Action::ZoomToSelection
            | Action::ZoomToFit
            | Action::AuditionMain
            | Action::AuditionProposal
            | Action::AcceptAiSuggestion
    )
}

/// 逻辑键解析出的 [`Action`] → 界面行为的**唯一**落点。
///
/// 判据（`tests/live_ui_mcp.rs` 的判据 16）从无头端口注入逻辑键，观测的就是这里写下的属性。
///
/// ## 为什么有些动作**不消费**
///
/// 没有可作用实现的动作由 [`action_has_implementation`] **同一份陈述**在这里挡下，
/// 返回 `false`（`reject`）。
fn apply_action(ui: &MainWindow, undo: Option<&Rc<UndoPort>>, action: Action) -> bool {
    if !action_has_implementation(action) {
        return false;
    }
    match action {
        // `[UI-NOTE-003]` 工具选择：数字键 → 矩阵行号 → `active-tool`（**单一数字口径**）。
        // 撤销端口在场时同时记一条动作日志（`UiAction::SelectTool` 只报显示态、不改工程）。
        Action::SelectTool(tool) => {
            if let Some(port) = undo {
                port.perform(UiAction::SelectTool(tool));
            }
            ui.set_active_tool(i32::from(tool.digit()));
            true
        }
        // `B`：在箭头与铅笔之间快速切换（规范 §3.3 的"快速切换"，不是"选中铅笔"）。
        Action::TogglePencilTool => {
            let pencil = i32::from(PENCIL_TOOL_DIGIT);
            let next = if ui.get_active_tool() == pencil {
                i32::from(crate::input::Tool::Select.digit())
            } else {
                pencil
            };
            ui.set_active_tool(next);
            true
        }
        // `Escape` = **取消当前手势**（`ADR-0004` S1 的纵向拖拽）。
        //
        // 没有手势在手时返回 `false`（照旧放行给焦点系统）—— 与 `Escape` 接线之前
        // 的行为逐位相同。这里**没有**新增 `Action` 变体：`Action::Cancel` 本来就在
        // 策略表里（`input.rs` 的 `Key::Escape => Resolution::Action(Action::Cancel)`），
        // 它此前只是没有落地实现（`action_has_implementation` 把它列在未实现里）。
        // 它也不进 `--print-shortcuts` 的表（`cli.rs` 的 18 条里没有 `Escape`），
        // 因此判据 B11b 的三面（渲染 / 解析 / 宿主）一位不动。
        Action::Cancel => cancel_track_height_drag(ui, undo),
        // 双视图：`Tab` 是切换，`F5`/`F6`/`Alt+1`/`Alt+2` 是直达。写的是**界面属性**，
        // 与 `ui/switch_main_view` 走的是同一个属性（管理动作那边有回读判据）。
        Action::ToggleView => {
            ui.set_arrangement_view(!ui.get_arrangement_view());
            true
        }
        Action::ShowView(View::Session) => {
            ui.set_arrangement_view(false);
            true
        }
        Action::ShowView(View::Arrangement) => {
            ui.set_arrangement_view(true);
            true
        }
        Action::ToggleSidebar => {
            ui.set_sidebar_collapsed(!ui.get_sidebar_collapsed());
            true
        }
        Action::ToggleConsoleMaximize => {
            ui.set_console_expanded(!ui.get_console_expanded());
            true
        }
        // 走带：两条都落到界面**已有**的 `toggle-play`（`wire_transport` 接到引擎的那一个
        // 回调）。引擎没有单独的"从光标继续"命令，`EngineHost::toggle_play` 在停住时
        // 就是从当前位置起播 —— 因此不在这里发明第二条走带语义。
        Action::PlayPause | Action::ResumeFromCursor => {
            ui.invoke_toggle_play();
            true
        }
        // 撤销族：经 `undo::dispatch_key`（**唯一下发点**）落到 `UndoPort::perform`，
        // 与时光机按钮、`Cmd+Z` 是同一条链。
        Action::Undo | Action::Redo | Action::OpenTimeMachine => {
            let Some(port) = undo else {
                return false;
            };
            let Some(ui_action) = crate::undo::dispatch_key(action) else {
                return false;
            };
            port.perform(ui_action);
            // 工程真的变了就重新投影（`Undo`/`Redo` 会改工程；时光机只开关弹窗）。
            refresh_undo_window(ui, port, matches!(action, Action::Undo | Action::Redo));
            true
        }
        // `Delete` / `Backspace`：删除**当前选区**（`[UI-NOTE-003]`）。
        //
        // 语义（三条都是决定, 不是意外）：
        // 1. 选区是**视图态**，事实源是界面属性 `selected-ulids`（`apply_view` 每次由它重算
        //    `note-selected` 标志, 见本文件 `apply_view`）—— 这里读它, 不读第二份状态；
        // 2. **无选区（或选中的身份已不存在）⇒ 什么都不做**，返回 `false`（不吞键）。
        //    这比"删最后一个音符"安全：没有选区时的删除意图**没有**可判定的对象,
        //    猜一个对象去删是数据损失。它与 `Action::Cancel`（没有手势在手时返回 `false`）同一取向；
        // 3. 一次按键 = **一次** `commit_ops`。`undo_session::commit` 把整批包成**一个**
        //    `Op::Batch` ⇒ 删 k 个音符**一步撤销**全部回来（`ARCH-OPS-002` 的原子性）。
        //    这里**不自己**构造 `Batch`：那会在模型之外长出第二个批次语义。
        //
        // 提交后必须重新投影：删掉的音符仍在界面数组里, 不重投影就是"模型删了、画面还在"。
        Action::DeleteSelection => {
            let Some(port) = undo else {
                return false;
            };
            let selected: Vec<String> = {
                use slint::Model as _;
                ui.get_selected_ulids()
                    .iter()
                    .map(|id| id.to_string())
                    .collect()
            };
            let Some(project) = port.try_project() else {
                return false;
            };
            let ops = delete_ops_for(&project, &selected);
            if ops.is_empty() {
                // 选区的对象一个都不在工程里 ⇒ 没有可停靠的编辑, 如实不消费。
                return false;
            }
            if port.commit_ops(now_ms(), "delete selection", ops).is_err() {
                return false;
            }
            // 被删的身份已经不存在 ⇒ 选区必须清空, 否则 `selected-note-count` 会停在旧值上
            // （下一次 `apply_view` 重算的标志会全为 `false`, 于是"选中数"与标志分叉）。
            let cleared: Vec<slint::SharedString> = Vec::new();
            ui.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(cleared)));
            ui.set_selected_note_count(0);
            refresh_undo_window(ui, port, true);
            true
        }
        // 其余取值已在函数开头被 `action_has_implementation` 挡下；这一支只为让 match 穷尽。
        _ => false,
    }
}

/// 把**撤销的**模型读数注入界面（显示态 + 时光机弹窗开关）。
///
/// 与 [`apply_transport`] 同一条纪律：界面**不自己算**"能不能撤销"。
/// 唯一的事实源是 [`UndoPort::display`]（= `undo_session::UndoDisplay`：
/// `CommitGraph` + `UndoCursor` 的读数）。
///
/// 为什么 `undo-tree-open` 也在这里写：它是**界面运行态**（不是模型读数），
/// 但它必须与"撤销动作"共用同一个写者，否则弹窗开关就会有两个真相源。
pub fn apply_undo(ui: &MainWindow, port: &UndoPort) {
    let display = port.display();
    let clamp = |value: usize| i32::try_from(value).unwrap_or(i32::MAX);
    ui.set_undo_tree_open(port.undo_tree_open());
    ui.set_undo_can_undo(display.can_undo);
    ui.set_undo_can_redo(display.can_redo);
    ui.set_undo_depth(clamp(display.undoable));
    ui.set_undo_undone(clamp(display.undone));
    ui.set_undo_commit_count(clamp(display.commit_count));
    ui.set_branch_name(display.branch.into());
}

/// 把撤销入口接到界面上（`toggle-undo-tree` 与时光机的"撤销一步"按钮）。
///
/// 两条回调都汇到**同一个** [`UndoPort::perform`]：
///
/// | 界面回调 | 动作 | 工程会不会变 |
/// | :--- | :--- | :--- |
/// | `toggle-undo-tree` | `UiAction::ToggleUndoTree` | 不会（只开关弹窗） |
/// | `undo-step`（弹窗按钮） | `UiAction::Undo` | **会**（走模型的撤销入口） |
///
/// 每次动作之后：
///
/// 1. 用**模型读数**回写显示态（[`apply_undo`]）；
/// 2. 工程真的变了就**重新投影**（[`ViewState::from_project`] → [`apply_view`]）——
///    界面显示的是回退后的那一版工程，而不是"游标动了但画面没动"。
///
/// 键盘那条路（`Cmd+Z` / `Cmd+Shift+Z` / `Cmd+Shift+H`）由 [`wire_keys`] 经
/// [`crate::undo::dispatch_key`]（唯一下发点）落到同一个 `UndoPort`：策略表仍然是
/// `crate::input` 那一份（`N2` 裁决 (1) 之后 GUI 用逻辑键入口、无头端口用物理码入口，
/// IME 合成态防护两条都在），本函数不复制它。
pub fn wire_undo(ui: &MainWindow, port: &Rc<UndoPort>) {
    {
        let port = Rc::clone(port);
        let weak = slint::ComponentHandle::as_weak(ui);
        ui.on_toggle_undo_tree(move || {
            port.perform(UiAction::ToggleUndoTree);
            refresh_undo(&weak, &port, false);
        });
    }
    {
        let port = Rc::clone(port);
        ui.on_export_diagnostics(move || {
            // [D56] 与 MCP 工具**同一个**实现（判据 4）；不改工程状态。
            port.perform(UiAction::ExportDiagnostics);
        });
    }
    {
        let port = Rc::clone(port);
        let weak = slint::ComponentHandle::as_weak(ui);
        ui.on_undo_step(move || {
            port.perform(UiAction::Undo);
            refresh_undo(&weak, &port, true);
        });
    }
}

/// `[UI-NOTE-003]` 卷帘的**编辑入口**（目前只接铅笔）：把一次点击变成**可撤销**的模型操作。
///
/// 规则（账本第 234/237 轮）：音符进"**点击 tick 所在片段**"、落在"**摆放该片段的轨道**"；
/// **位置不在任何片段内 ⇒ 拒绝**（不新建片段）。撤销与提交图由 `commit_ops` 提供, 无需另写反向逻辑。
/// `grid` 取 1/16（240 tick）作为当前吸附口径 —— 吸附设置将来若可配, 它应来自配置而不是这里。
/// 把一次点击解析成**待提交的操作**，纯函数以便判据（`wire_roll_edit` 只负责把它接到端口）。
///
/// 返回 `None` 的三种情形都是**决定**而非意外（第 234/237 轮）：工具不是铅笔；点击处吸附不出计划；
/// **位置不在任何片段内**（拒绝, 不新建片段）。返回 `Some(op)` 表示"应当提交这一个操作"。
#[must_use]
pub fn pencil_op_for(
    project: &yeban_model::YebanProjectV1,
    scroll_x: f32,
    x: f32,
    y: f32,
    grid_ticks: u64,
    active_tool: i32,
) -> Option<yeban_model::ops::Op> {
    if active_tool != i32::from(PENCIL_TOOL_DIGIT) {
        return None;
    }
    // 这里的命中测试**只**看卷帘的 14px 音高泳道（`lane_at_y`）与 tick 落在哪个片段，
    // 与编排行高无关：行高变高/变矮**不**改这条路径的答案（因此 S1 不给它加布局参数
    // —— 一个不影响结果的参数是假依赖）。
    let view = ViewState::from_project(project).ok()?;
    let plan = view.pencil_plan(scroll_x, x, y, grid_ticks)?;
    let mut target: Option<(yeban_model::EntityId, yeban_model::EntityId)> = None;
    for (track_id, track) in &project.tracks {
        let triples: Vec<(u64, u64, yeban_model::EntityId)> = track
            .clips
            .values()
            .map(|placement| {
                (
                    placement.start_tick,
                    placement.duration_ticks,
                    placement.clip_id,
                )
            })
            .collect();
        if let Some(clip_id) = crate::bridge::clip_at_tick(&triples, plan.start_tick) {
            target = Some((*track_id, clip_id));
            break;
        }
    }
    let (track_id, clip_id) = target?;
    Some(crate::bridge::plan_to_add_note(
        plan,
        track_id,
        clip_id,
        yeban_model::EntityId::new(),
    ))
}

/// 铅笔工具的**数字键值**（与 `active-tool` 及 `Tool::from_digit` 同一口径）。
pub const PENCIL_TOOL_DIGIT: u8 = 2;

/// 当前吸附网格（tick）：1/16 = 240。将来若可配, 应由配置注入而不是在这里长第二个真相源。
pub const ROLL_SNAP_GRID_TICKS: u64 = 240;

/// 当前 UNIX 毫秒（提交时间戳）。溢出与时钟倒退都夹到 `0` / `u64::MAX`，**不 panic**。
///
/// 抽出来是因为它有两个调用点（铅笔与删除）：两处各写一遍 `SystemTime` 就是第二个真相源。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `[UI-NOTE-003]` 把"选中的音符身份"解析成待提交的 **`Op::DeleteNote` 集合**（纯函数, 有判据）。
///
/// 返回**空 `Vec`** 的两种情形都是**决定**：选区为空；选中的身份在工程里一个都解析不到
/// （例如它们已被上一次删除移除）—— 那时调用方**什么都不做**。
///
/// 为什么一次返回**一批**而不是一个：`undo_session::commit` 把整批包成一个 `Op::Batch`，
/// 于是"一次按键 = 一次提交 = 一步撤销"是构造上的（`ARCH-OPS-002`）。逐个提交会让
/// 删 5 个音符需要按 5 次 `Cmd+Z` 才回来 —— 那不是原子。
///
/// `track_id` 只用于 `DeleteNote::precondition` 的"这条轨道存在吗"检查（音符住在
/// **片段池**里, 不住在轨道上）。这里取**摆放了该片段的第一条轨道**（键序确定）；
/// 一个谁都没摆放的片段池条目回退到工程的**第一条轨道**。工程一条轨道都没有 ⇒ 跳过该音符。
#[must_use]
pub fn delete_ops_for(
    project: &yeban_model::YebanProjectV1,
    selected_ulids: &[String],
) -> Vec<yeban_model::ops::Op> {
    use std::collections::{BTreeMap, BTreeSet};
    // 身份集合：`BTreeSet` 迭代确定（与红线 4 的取向一致），顺带自动去重。
    let targets: BTreeSet<yeban_model::EntityId> = selected_ulids
        .iter()
        .filter_map(|raw| raw.parse::<yeban_model::EntityId>().ok())
        .collect();
    if targets.is_empty() {
        return Vec::new();
    }
    // "片段池条目 → 摆放它的第一条轨道"（`or_insert` ⇒ 第一次命中胜出, 键序确定）。
    let mut owner: BTreeMap<yeban_model::EntityId, yeban_model::EntityId> = BTreeMap::new();
    for (track_id, track) in &project.tracks {
        for placement in track.clips.values() {
            owner.entry(placement.clip_id).or_insert(*track_id);
        }
    }
    let fallback_track = project.tracks.keys().next().copied();
    let mut ops = Vec::new();
    // 片段池键序 → 音符键序：与 `ViewState.notes` 的既有顺序同源（`bridge.rs` 的投影顺序）。
    for (clip_id, entry) in &project.clip_pool {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        for (note_id, note) in notes {
            if !targets.contains(note_id) {
                continue;
            }
            let Some(track_id) = owner.get(clip_id).copied().or(fallback_track) else {
                continue;
            };
            ops.push(yeban_model::ops::Op::DeleteNote {
                track_id,
                clip_id: *clip_id,
                note_id: *note_id,
                // 自带撤销载荷：撤销不需要回放历史（`ops.rs` 的设计约束 2）。
                previous_note: note.clone(),
            });
        }
    }
    ops
}

/// `[UI-NOTE-003]` 把卷帘的点击接到**撤销端口**上（目前只接铅笔的工具语义）。
///
/// 解析本身在 [`pencil_op_for`] 里（纯函数, 有判据）；这里只做三件事: 读界面状态、
/// 交给端口提交（`commit_ops` ⇒ 撤销与提交图随之而来）、重新投影让新音符出现。
/// **拒绝**的情形（工具不对 / 吸附不出计划 / 位置不在任何片段内）在这里就是"什么都不做"。
pub fn wire_roll_edit(ui: &MainWindow, port: &Rc<UndoPort>) {
    let weak = ui.as_weak();
    let port = Rc::clone(port);
    ui.on_clicked(move |x, y| {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        let scroll = ui.get_roll_scroll_x();
        let width = slint::ComponentHandle::window(&ui).size().width as f32;
        // 权威会话可能被控制面关掉工程（`try_project` 如实返回 `None`）——
        // 那时**什么都不做**，而不是在一个 Slint 回调里 panic。
        let Some(project) = port.try_project() else {
            return;
        };
        let Some(op) = pencil_op_for(
            &project,
            scroll,
            x,
            y,
            ROLL_SNAP_GRID_TICKS,
            ui.get_active_tool(),
        ) else {
            // 工具不对、吸附不出计划、或位置不在任何片段内 ⇒ **拒绝**（第 234 轮的决定）。
            return;
        };
        let now_ms = now_ms();
        if port
            .commit_ops(now_ms, "pencil: add note", vec![op])
            .is_err()
        {
            return;
        }
        // 重新投影, 让新音符出现在界面上（与 `refresh_undo` 同一手法）。
        // 行高布局（`ADR-0004` S1）是视图态 ⇒ 重投影必须读回来，
        // 否则"设了行高之后用铅笔写一个音符"会把行高静默清零。
        let layout = track_height_layout(&ui);
        if let Some(refreshed) = port
            .try_project()
            .and_then(|project| ViewState::from_project_with_layout(&project, &layout).ok())
        {
            apply_view(&ui, &refreshed, width, scroll);
        }
    });
}

/// 撤销动作之后把界面拉回**模型读数**（显示态）+ **投影**（工程画面）。
///
/// `reproject` 为 `false` 时只回写显示态：`toggle-undo-tree` 不改工程，
/// 重新投影一遍是白费（而且 `apply_view` 会重置电平快照）。
fn refresh_undo(weak: &slint::Weak<MainWindow>, port: &UndoPort, reproject: bool) {
    let Some(ui) = weak.upgrade() else {
        debug_assert!(false, "MainWindow 在回调执行期间被销毁");
        return;
    };
    refresh_undo_window(&ui, port, reproject);
}

/// 同 [`refresh_undo`]，但调用方**已经**持有活窗口（键盘路径就是这样）。
///
/// 抽出来的理由不是省一次 `upgrade`：两条入口必须回写**同一批**属性与投影，
/// 否则"按钮撤销"和"快捷键撤销"会在界面上留下不同的显示态。
fn refresh_undo_window(ui: &MainWindow, port: &UndoPort, reproject: bool) {
    apply_undo(ui, port);
    if !reproject {
        return;
    }
    // 权威会话可能被控制面关掉工程 ⇒ `try_project` 如实返回 `None`，
    // 那时**出声不画**，而不是在一个 Slint 回调里 panic。
    let Some(project) = port.try_project() else {
        eprintln!("[yeban-app] 撤销后没有可投影的工程: 权威会话此刻没有活跃工程");
        return;
    };
    // 撤销只改工程，**不该**丢掉视图态：行高布局从窗口读回来（与 `scroll_x` 同款）。
    let layout = track_height_layout(ui);
    match ViewState::from_project_with_layout(&project, &layout) {
        Ok(view) => {
            // 复用当前偏移：撤销**不应**把卷帘滚回起点（账本第 200 轮记录的缺陷）。
            let scroll_x = ui.get_roll_scroll_x();
            apply_view(ui, &view, ui.window().size().width as f32, scroll_x)
        }
        Err(error) => {
            // 投影失败**出声**：工程已经在内存里回退了，但这一帧画不出来。
            // 静默吞掉会让"撤销没反应"变成一个查不出的现象。
            eprintln!("[yeban-app] 撤销后重新投影失败: {error}");
        }
    }
}

/// 卷帘当前绘制的泳道数（与 `piano_roll.slint` 的 `for lane_index in 16` 必须一致）。
///
/// 它是**固定值**，因为卷帘目前没有纵向滚动/缩放模型；若将来有，它必须由 `.slint` 上报（账本第 202 轮）。
const ROLL_LANE_COUNT: i32 = 16;

/// 卷帘音符矩形的高度（逻辑像素）—— 必须与 `piano_roll.slint` 里音符 `Rectangle` 的 `height` 一致。
///
/// 实测教训: 我第一次写 6.0 而 `.slint` 是 `height: 12px` —— 注释声称"两处必须一致", 实际不一致,
/// 而判据没抓到（它只点了顶部 3px 内）。现在由 `check_viewport_bounds_wiring.py` 机械钉住。
const NOTE_HEIGHT_PX: f32 = 12.0;

/// `[UI-NOTE-001]` tick 值转为界面的 `i32` 属性：**饱和**而非回绕。
///
/// 天真的 `as i32` 会把超过 `i32::MAX` 的 tick 变成**负数** —— 视口下界变负会让裁剪核心
/// 得到一个自相矛盾的范围，而且不会报错。饱和到 `i32::MAX` 至少是"很大"，语义上仍然单调。
#[must_use]
fn tick_to_i32_saturating(tick: u64) -> i32 {
    i32::try_from(tick).unwrap_or(i32::MAX)
}

/// `[ROAD-M3-002]` 卷帘滚动的**状态推进**：把一次手势增量并入当前偏移。
///
/// 抽成纯函数是为了可判据（回调本身需要 UI 线程与真实指针事件）；下限 0 表示**不滚到时间轴之前**。
/// 上限暂不设：内容长度由投影决定，超过末端的部分自然裁空（见账本第 199 轮）。
#[must_use]
fn advance_scroll(current: f32, delta: f32) -> f32 {
    (current + delta).max(0.0)
}

/// 构造主窗口：注入**外壳场景**/// 构造主窗口：注入**外壳场景**（会话运行态 / 本机视口）+ 投影状态（底部控制台默认 Tab 0）。
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
    build_main_window_with_console_tab(view, scene, 0)
}

/// 把 `--theme` 选中的主题写进 Slint 的 `ThemeState.theme`（**唯一**的主题写入口）。
///
/// ## 与 `apply_view` 同一条纪律
///
/// 单向注入：`.slint` 侧只**读** `ThemeState.theme`（`ui/tokens.slint` 的颜色令牌
/// 都是它的多选一表达式），Rust 侧只**写**。界面文件因此一行都不用改 ——
/// "换主题"落在唯一一处：这个函数。
///
/// ## 它**不**做什么（边界，写在 `--print-theme` 与用法文本里）
///
/// 它**不**改变 Slint 编译进来的**风格**。Slint 1.18.1 没有运行时换风格的 API
/// （`slint::select_built_in_style` 在该版本的源码里不存在），风格由 `build.rs` 的
/// `SLINT_STYLE` → `slint_build::CompilerConfiguration::with_style` 在**编译期**定死。
/// 于是 [`crate::cli::Theme::Material`] / `Fluent` / `Cupertino` / `Native` 这四个值
/// 在当前二进制里指向**同一个** `Palette` —— 这是实测出来的上限，不是实现偷懒。
/// 要真的换风格，重新构建时给 `SLINT_STYLE=<style>`（用法文本里有完整取值表）。
///
/// [`crate::cli::Theme::Yeban`] 是 2026-10-06 新增的**第三类**：它既不是品牌字面量、
/// 也不是设计系统角色，而是一串自己的十六进制字面量（**2026-10-07 起取值由负责人下发的
/// 具名调色板指定**，见 `ui/tokens.slint` §6b 的出处表；早期那版采样推导的取值已作废）。
/// 它同样不读 `Palette`，因此与编进来的风格无关。**2026-10-07 起它是默认主题**：
/// CLI 规范名是 `default`（`Theme::Yeban`），`yeban` 保留为**别名**。
///
/// [`crate::cli::Theme::InkMoor`] / [`crate::cli::Theme::Plume`] 是 2026-10-07 新增的
/// **同款第三类**：负责人设计稿「墨泊 InkMoor」/「孤烟 Plume」各是一串自己的十六进制
/// 字面量（`ui/tokens.slint` §6e 的逐条映射），同样不读 `Palette`。四支自绘调色板
/// (`Brand` / `Yeban` / `InkMoor` / `Plume` —— 这四个是 **CLI 侧** [`crate::cli::Theme`]
/// 的变体名；`.slint` 侧写的是 `YebanTheme.brand` / `.yeban` / `.inkmoor` / `.plume`，
/// 而 Slint 的 Rust 生成器只把每段的**首字母**大写，所以 `inkmoor` 那支**生成出来**
/// 是 `Brand` / `Yeban` / `Inkmoor` / `Plume` —— `Inkmoor` 与本段列出的 CLI 变体名
/// `InkMoor` 差一个大写，两者不是同一个名字，别混用；实测见 `target/debug/build/
/// yeban-app-*/out/app.rs` 的 `pub enum YebanTheme`) 与四个设计系统名字共享**这一个**写入口。
///
/// 为什么仍然值得给四支自绘调色板之外留四个名字：`Brand` / `Yeban` ↔ 其余四个的差别
/// 是**真实的**像素差别（自绘十六进制字面量 vs 设计系统 `Palette` 角色），而且 `Palette`
/// 会跟随系统的浅色/深色设置（`SlintInternal.color-scheme`）—— 那正是"搬到真实设计系统上"
/// 的收益。
/// 返回 `()` 而不是全局句柄：调用方（`main.rs` / `headless_idle.rs`）只需要"写进去"，
/// 而判据自己用 `ui.global::<ThemeState>()` 回读 —— 把句柄当返回值只会让每个调用点
/// 多一个"忽略了必须使用的返回值"的告警。
pub fn apply_theme(ui: &MainWindow, theme: crate::cli::Theme) {
    ui.global::<ThemeState<'_>>().set_theme(match theme {
        crate::cli::Theme::Brand => YebanTheme::Brand,
        crate::cli::Theme::Yeban => YebanTheme::Yeban,
        // Slint 的 Rust 代码生成把 `.slint` 里的 `inkmoor` 转成 `Inkmoor`
        // （单个小写词没有分隔符 ⇒ 只首字母大写，不是 `InkMoor`；后者是负责人设计稿的
        // `ThemeKind.InkMoor` 拼法，不是这里生成的拼法）。实测见 `target/debug/build/
        // yeban-app-*/out/app.rs` 的 `pub enum YebanTheme`。
        crate::cli::Theme::InkMoor => YebanTheme::Inkmoor,
        crate::cli::Theme::Plume => YebanTheme::Plume,
        crate::cli::Theme::Material => YebanTheme::Material,
        crate::cli::Theme::Fluent => YebanTheme::Fluent,
        crate::cli::Theme::Cupertino => YebanTheme::Cupertino,
        crate::cli::Theme::Native => YebanTheme::Native,
    });
}

/// 同 [`build_main_window`]，但显式指定底部控制台的初始 Tab。
///
/// 存在的理由：混音台通道条（Tab 1）只在被选中时**进入运行时控件树**（`visible` 为假的分支
/// 不进树，见 `[ARCH-UI-005]` 的实测语义）。"通道条数 == 工程轨道数"这条判据因此必须能
/// 建一个"混音台可见"的窗口 —— 而构造实现仍然**只有这一份**（D28 的"唯一注入点"）。
///
/// 生产路径继续用 [`build_main_window`]（默认 Tab 0），发行行为一行未改。
///
/// # Errors
///
/// 同 [`build_main_window`]。
pub fn build_main_window_with_console_tab(
    view: &ViewState,
    scene: &DemoScene,
    console_tab: i32,
) -> Result<MainWindow, slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.set_timecode(scene.timecode.clone().into());
    ui.set_branch_name(scene.branch_name.clone().into());
    ui.set_arrangement_view(scene.arrangement_by_default);
    ui.set_playing(false);
    ui.set_console_tab(console_tab);
    ui.set_compact(scene.compact());
    apply_view(&ui, view, ui.window().size().width as f32, 0.0);
    // `[ROAD-M3-002]` 滚动手势（账本第 199 轮）：偏移由**宿主**拥有 —— 因为裁剪也在宿主侧。
    // 卷帘只报告增量, 宿主 clamp + 累加后**重新注入**；不用 `Flickable`, 否则会与宿主偏移双重计算。
    let scroll = std::rc::Rc::new(std::cell::RefCell::new(0.0_f32));
    // `[UI-NOTE-003]` 选区：点击的**命中与语义**都在 Rust 侧（账本第 495/496 轮），界面只报坐标。
    let selection = std::rc::Rc::new(std::cell::RefCell::new(crate::input::Selection::new()));
    let view_snapshot = std::rc::Rc::new(view.clone());
    {
        let weak = ui.as_weak();
        let scroll = std::rc::Rc::clone(&scroll);
        let view_snapshot = std::rc::Rc::clone(&view_snapshot);
        ui.on_scroll_requested(move |delta| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let next = {
                let mut current = scroll.borrow_mut();
                // 下限 0：不滚到时间轴之前；上限暂不设（内容长度由投影决定, 见账本第 199 轮）。
                *current = advance_scroll(*current, delta);
                *current
            };
            // `Rc<ViewState>` 是不可变共享 ⇒ 直接解引用, 无需 `borrow`。
            apply_view(&ui, &view_snapshot, ui.window().size().width as f32, next);
        });
    }

    {
        let weak = ui.as_weak();
        let selection = std::rc::Rc::clone(&selection);
        let view_snapshot = std::rc::Rc::clone(&view_snapshot);
        ui.on_clicked(move |x, y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let scroll = ui.get_roll_scroll_x();
            let width = slint::ComponentHandle::window(&ui).size().width as f32;
            // 音符框高与 `.slint` 的矩形一致；值写在一处, 免得命中测试去猜（账本第 495 轮）。
            let hit = view_snapshot.hit_test_visible(scroll, width, x, y, NOTE_HEIGHT_PX);
            let mut sel = selection.borrow_mut();
            match hit {
                Some(index) => sel.select_only(&view_snapshot.notes[index].id),
                None => sel.clear(),
            }
            ui.set_selected_note_count(i32::try_from(sel.len()).unwrap_or(i32::MAX));
            // 把选中 id 写回界面属性 —— `apply_view` 由它重算标志, 保证重注入后仍对齐。
            let ids: Vec<slint::SharedString> = sel.iter().map(|id| id.as_str().into()).collect();
            ui.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(ids)));
            let visible = view_snapshot.visible_notes(scroll, width);
            let flags = crate::input::flags_for_ids(
                &sel.iter().cloned().collect::<Vec<_>>(),
                &visible.ulids,
            );
            ui.set_note_selected(slint::ModelRc::new(slint::VecModel::from(flags)));
        });
    }
    Ok(ui)
}

#[cfg(test)]
mod tests {
    use super::advance_scroll;

    #[test]
    fn advance_scroll_accumulates_and_clamps_at_zero() {
        // 判据: 累加生效, 且**不滚到时间轴之前**（负增量在 0 处停住, 不会变成负数）。
        assert!((advance_scroll(0.0, 16.0) - 16.0).abs() < f32::EPSILON);
        assert!((advance_scroll(16.0, 16.0) - 32.0).abs() < f32::EPSILON);
        assert!((advance_scroll(16.0, -16.0) - 0.0).abs() < f32::EPSILON);
        assert!(
            (advance_scroll(0.0, -100.0) - 0.0).abs() < f32::EPSILON,
            "负偏移必须被夹到 0"
        );
        assert!((advance_scroll(8.0, -100.0) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn tick_to_i32_saturating_does_not_wrap() {
        // 牙: 超过 i32::MAX 的 tick 必须**饱和**, 不得变成负数（`as i32` 会回绕成负）。
        assert_eq!(super::tick_to_i32_saturating(0), 0);
        assert_eq!(super::tick_to_i32_saturating(24_000_000), 24_000_000);
        assert_eq!(super::tick_to_i32_saturating(i32::MAX as u64), i32::MAX);
        assert_eq!(super::tick_to_i32_saturating(i32::MAX as u64 + 1), i32::MAX);
        assert_eq!(super::tick_to_i32_saturating(u64::MAX), i32::MAX);
        assert!(
            super::tick_to_i32_saturating(u32::MAX as u64) > 0,
            "不得为负"
        );
    }

    /// 判据（`ADR-0004` S1 的纵向入口）：拖拽手势的**像素换算**（纯函数）。
    ///
    /// 六条口径，每条都能单独被打红：
    ///
    /// 1. **向上拖 = 变高**（`delta_px > 0`），向下拖 = 变矮；
    /// 2. **乘子的逆**：乘子 150% 时"拖 30 像素"⇒ 基准 +20（画面上仍然长 30 像素）；
    /// 3. **整数**：结果四舍五入（`D28`）；
    /// 4. **下界是 1 而不是 0**：`0` 在 `set_track_px` 的契约里是"删除这条覆盖"
    ///    （`ADR-0004` Q2 的退化值处置）—— 手势不许借用那个语义；
    /// 5. **不夹上界**：夹紧只在 `bridge::effective_track_height_px` 里发生一次（Q2-B），
    ///    因此 1000 像素的位移给出 1056 的**基准**（投影再把它夹到 `MAX_TRACK_HEIGHT_PX`）；
    /// 6. **非有限位移不改写**：`NaN` / `±∞` 原样返回起点（合法的、`> 0` 的高度）。
    #[test]
    fn dragged_track_base_px_is_the_inverse_of_the_percent_multiplier_and_stays_positive() {
        use super::dragged_track_base_px;

        // ① 方向：向上拖（delta > 0）变高，向下拖变矮。
        assert_eq!(dragged_track_base_px(56, 40.0, 100), 96);
        assert_eq!(dragged_track_base_px(96, -40.0, 100), 56);
        assert_eq!(dragged_track_base_px(56, 0.0, 100), 56);
        // ② 乘子的逆：150% 下拖 30 像素 ⇒ 基准 +20（画面增量仍是 30）。
        assert_eq!(dragged_track_base_px(80, 30.0, 150), 100);
        assert_eq!(dragged_track_base_px(80, -30.0, 150), 60);
        // ③ 整数：四舍五入，不是截断（0.6 ⇒ +1）。
        assert_eq!(dragged_track_base_px(56, 0.6, 100), 57);
        assert_eq!(dragged_track_base_px(56, 0.4, 100), 56);
        // ④ 下界 1：往下拖到底也**不会**变成 0（0 = 删除覆盖，不是"高 0 的行"）。
        assert_eq!(dragged_track_base_px(56, -10_000.0, 100), 1);
        assert_eq!(dragged_track_base_px(1, -10_000.0, 100), 1);
        // ⑤ 上界不在这里：投影才夹紧（本函数原样给出基准）。
        assert_eq!(dragged_track_base_px(56, 1_000.0, 100), 1_056);
        assert!(
            dragged_track_base_px(56, 1_000.0, 100) > crate::bridge::MAX_TRACK_HEIGHT_PX,
            "基准可以超过上界 —— 夹紧是投影的事（边界只有一处）"
        );
        // ⑥ 非有限位移 / 乘子为 0（"缩到最小"的退化设置）都有确定行为。
        assert_eq!(dragged_track_base_px(56, f32::NAN, 100), 56);
        assert_eq!(dragged_track_base_px(56, f32::INFINITY, 100), 56);
        assert_eq!(dragged_track_base_px(56, -40.0, 0), 16);
        assert_eq!(
            dragged_track_base_px(0, 0.0, 100),
            1,
            "起点 0 也不可能产出 0"
        );
    }
}

#[cfg(test)]
mod pencil_op_tests {
    use super::*;
    use yeban_model::samples::filled_project;

    #[test]
    fn the_pencil_resolution_refuses_the_wrong_tool_and_positions_outside_every_clip() {
        // 判据: 三种**拒绝**都是决定（第 234/237 轮）, 不是意外。
        let project = filled_project();
        let grid = ROLL_SNAP_GRID_TICKS;
        // ① 工具不是铅笔（选择工具 = 1）⇒ None。
        assert!(
            pencil_op_for(&project, 0.0, 100.0, 100.0, grid, 1).is_none(),
            "选择工具下点击**不得**产生编辑"
        );
        // ② 位置远在所有片段之外 ⇒ None（**拒绝而不是新建片段**）。
        assert!(
            pencil_op_for(
                &project,
                0.0,
                1.0e6,
                100.0,
                grid,
                i32::from(PENCIL_TOOL_DIGIT)
            )
            .is_none(),
            "片段之外的点击必须被拒绝"
        );
        // ③ 片段内**存在**能给出操作的位置 ⇒ Some, 且目标片段正是该位置所属的片段。
        // 不假设坐标映射的名字（第一版写 `view.tick_to_px` 并不存在）⇒ 扫描 x, 断言**存在性**。
        let clip_id = project
            .tracks
            .values()
            .find_map(|track| track.clips.values().next().map(|p| p.clip_id))
            .expect("夹具里至少有一条带片段的轨道");
        let digit = i32::from(PENCIL_TOOL_DIGIT);
        let hit = (0..240)
            .map(|step| 10.0 * f32::from(u8::try_from(step).unwrap_or(0)))
            .find_map(|x| pencil_op_for(&project, 0.0, x, 100.0, grid, digit));
        match hit {
            Some(yeban_model::ops::Op::AddNote {
                clip_id: target, ..
            }) => {
                assert_eq!(target, clip_id, "必须落在点击位置所属的片段里");
            }
            Some(other) => panic!("铅笔必须产生 AddNote, 实际是 {other:?}"),
            None => panic!("片段所在区域**必须**存在能给出操作的点击位置"),
        }
    }

    #[test]
    fn delete_ops_for_covers_every_selected_note_and_the_batch_reverses_whole() {
        // 判据：`delete_ops_for` 是"选区身份 → 待提交 op"的**唯一**解析点。
        // 空选区与垃圾身份 ⇒ **空批**（"什么都不做"是返回空, 不是 panic, 也不是删一个猜的对象）。
        use yeban_model::ops::Op;
        let project = filled_project();
        let count = |project: &yeban_model::YebanProjectV1| -> usize {
            project
                .clip_pool
                .values()
                .filter_map(|entry| entry.content.notes())
                .map(std::collections::BTreeMap::len)
                .sum()
        };
        let total = count(&project);
        assert!(total >= 3, "夹具至少要有 3 个音符, 实际 {total}");
        let ids: Vec<String> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.keys().map(ToString::to_string))
            .collect();

        assert!(delete_ops_for(&project, &[]).is_empty(), "空选区 ⇒ 空批");
        assert!(
            delete_ops_for(&project, &["not-a-ulid".to_owned()]).is_empty(),
            "解析不出的身份 ⇒ 空批"
        );

        let ops = delete_ops_for(&project, &ids);
        assert_eq!(ops.len(), total, "每个选中的音符恰好一条 `DeleteNote`");
        assert!(
            ops.iter().all(|op| matches!(op, Op::DeleteNote { .. })),
            "只允许 `DeleteNote`"
        );

        // 原子 + 可逆：`Op::Batch` 的逆是**逆序**的逆操作批 —— 与 `undo_session::commit` 同一条路。
        let batch = Op::Batch {
            ops,
            description: "delete selection".to_owned(),
        };
        let mut doc = project.clone();
        batch.apply(&mut doc).expect("整批必须可应用（原子）");
        assert_eq!(count(&doc), 0, "全选删除后一个音符都不剩");
        batch.apply_inverse(&mut doc).expect("一步撤销必须整批回来");
        assert_eq!(count(&doc), total, "撤销一次必须**全部**回来");
    }
}
