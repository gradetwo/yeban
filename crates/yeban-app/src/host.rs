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

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};
use yeban_engine::transport::TransportReading;
use yeban_model::PPQ;

use crate::bridge::{DEFAULT_TRACK_COLOR, RgbColor, ViewState};
use crate::engine_host::EngineHost;
use crate::meters::{MeterSnapshot, silent_snapshot};
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
pub fn apply_view(ui: &MainWindow, view: &ViewState) {
    ui.set_window_title(view.title.clone().into());
    ui.set_bpm_display(view.bpm_display.clone().into());
    ui.set_track_names(strings(&view.track_names()));
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
    ui.set_clip_lanes(integers(&view.clip_lanes()));
    ui.set_section_positions(lengths(&view.section_positions()));
    ui.set_section_widths(lengths(&view.section_widths()));
    ui.set_bar_positions(lengths(&view.bar_positions));
    ui.set_note_ulids(strings(&view.note_ulids));
    ui.set_note_velocities(lengths(&view.note_velocities));
    // 卷帘音符的位置（x / y / 宽）—— 由 `MidiNote::start_tick` / `pitch` 整数派生，
    // 界面只做"取数组下标"，不做任何位置算术。
    ui.set_note_positions(lengths(&view.note_positions()));
    ui.set_note_widths(lengths(&view.note_widths()));
    ui.set_note_ys(lengths(&view.note_ys()));
    // 轨道色标：解析 / 回退都在投影层完成，这里只转成 Slint 的 `Color`。
    ui.set_track_colors(colors(&view.track_colors()));
    ui.set_track_color_labels(strings(&view.track_color_labels()));
    // 自动化泳道：9 个平行数组全部来自 `automation.rs` 的投影。顶点已经是逻辑像素、
    // 数值已经是格式化文本，因此这里只做"数组搬运"——`.slint` 侧零算术。
    ui.set_automation_lane_target_keys(strings(&view.automation_lane_target_keys()));
    ui.set_automation_lane_track_indexes(integers(&view.automation_lane_track_indexes()));
    ui.set_automation_lane_labels(strings(&view.automation_lane_labels()));
    ui.set_automation_lane_axis_labels(strings(&view.automation_lane_axis_labels()));
    ui.set_automation_lane_band_ys(lengths(&view.automation_lane_band_ys()));
    ui.set_automation_lane_band_heights(lengths(&view.automation_lane_band_heights()));
    ui.set_automation_lane_read_enabled(booleans(&view.automation_lane_read_enabled()));
    ui.set_automation_lane_badges(strings(&view.automation_lane_badges()));
    ui.set_automation_path_commands(strings(&view.automation_path_commands()));
    apply_master(ui, view);
    // 电平：先重置成"与当前工程等长的静音"，再由 `apply_meters` 填真实读数（见模块文档）。
    apply_meters(ui, &silent_snapshot(view));
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

/// 一拍里的 tick 数（[MODEL-AST-001] 的 960 PPQ）。
///
/// 时间码的**拍号**假定 4/4：`TimeSignature` 已经在模型里，但它**没有**被投影进
/// `ViewState`（登记为 needs）—— 在拿到它之前按 4/4 格式化，而不是编一个"已支持拍号"的假象。
const BEATS_PER_BAR: u64 = 4;

/// tick → 时间码文本（`BBB.BB.TTT`：小节.拍.拍内 tick，全部 **1 起 / 0 起**按 `SESSION_TIMECODE` 口径）。
///
/// 纯函数、只做整数除法 —— 因此"显示的时间码来自引擎读数"这件事可以被机械断言：
/// 输入是 `TransportReading::position_ticks`，输出是界面上的字符串。
#[must_use]
pub fn timecode_for_ticks(ticks: u64) -> String {
    let ticks_per_bar = PPQ * BEATS_PER_BAR;
    let bar = ticks / ticks_per_bar + 1;
    let beat = (ticks % ticks_per_bar) / PPQ + 1;
    let in_beat = ticks % PPQ;
    format!("{bar:03}.{beat:02}.{in_beat:03}")
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
/// `timecode` 用的是引擎读数（[`TransportReading::position_ticks`]），不是界面自己算的。
pub fn apply_transport(ui: &MainWindow, reading: TransportReading) {
    ui.set_playing(reading.state.is_running());
    ui.set_timecode(timecode_for_ticks(reading.position_ticks).into());
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

/// 构造主窗口：注入**外壳场景**（会话运行态 / 本机视口）+ 投影状态（底部控制台默认 Tab 0）。
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
    apply_view(&ui, view);
    Ok(ui)
}
