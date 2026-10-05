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

use crate::bridge::{DEFAULT_TRACK_COLOR, RgbColor, ViewState};
use crate::engine_host::EngineHost;
use crate::input::{Focus, InputContext};
use crate::meters::{MeterSnapshot, silent_snapshot};
use crate::scene::DemoScene;
use crate::ui::MainWindow;
use crate::undo::{UiAction, UndoPort};

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
    // `[ROAD-M3-002 / BASELINE-003]` **视口裁剪**：只注入可见窗口内的音符。
    // 六个平行数组（ulids / velocities / positions / widths / ys / rows）必须共用**同一**索引集，
    // 否则语义 ID 与力度会和几何错位 —— 所以用 `visible_notes` 一次取走，而不是各数组各裁一遍。
    // 本步先按 `scroll_x = 0`（尚无滚动模型, 见账本第 183 轮）：第一屏正确、屏外正确地不画。
    let visible = view.visible_notes(scroll_x, viewport_width);
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
/// 键盘那条路（`Cmd+Z` / `Cmd+Shift+Z` / `Cmd+Shift+H`）走的是
/// [`crate::undo::dispatch_key`] + [`crate::undo::perform_key`]：策略表仍然是
/// `crate::input` 那一份（物理扫描码 + IME 合成态防护），本函数不复制它。
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

/// 撤销动作之后把界面拉回**模型读数**（显示态）+ **投影**（工程画面）。
///
/// `reproject` 为 `false` 时只回写显示态：`toggle-undo-tree` 不改工程，
/// 重新投影一遍是白费（而且 `apply_view` 会重置电平快照）。
fn refresh_undo(weak: &slint::Weak<MainWindow>, port: &UndoPort, reproject: bool) {
    let Some(ui) = weak.upgrade() else {
        debug_assert!(false, "MainWindow 在回调执行期间被销毁");
        return;
    };
    apply_undo(&ui, port);
    if !reproject {
        return;
    }
    match ViewState::from_project(&port.project()) {
        Ok(view) => apply_view(&ui, &view, ui.window().size().width as f32, 0.0),
        Err(error) => {
            // 投影失败**出声**：工程已经在内存里回退了，但这一帧画不出来。
            // 静默吞掉会让"撤销没反应"变成一个查不出的现象。
            eprintln!("[yeban-app] 撤销后重新投影失败: {error}");
        }
    }
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
    Ok(ui)
}
