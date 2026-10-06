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
use crate::input::{Action, Focus, InputContext, LogicalKey, Modifiers, Resolution, View};
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

/// 逻辑键解析出的 [`Action`] → 界面行为的**唯一**落点。
///
/// 判据（`tests/live_ui_mcp.rs` 的判据 16）从无头端口注入逻辑键，观测的就是这里写下的属性。
///
/// ## 为什么有些动作**不消费**
///
/// `Action::DeleteSelection` / `Duplicate` / `ZoomToSelection` / `ZoomToFit` /
/// `AuditionMain` / `AuditionProposal` / `AcceptAiSuggestion` / `Cancel` 目前**没有**可作用的
/// 实现（模型侧的编辑语义还没落地）。这里让它们落到 `false`（`reject`）—— 把键吞掉却什么
/// 都不做，比不处理更糟：用户会以为"这个功能坏了"，而日志里没有任何东西能解释。
fn apply_action(ui: &MainWindow, undo: Option<&Rc<UndoPort>>, action: Action) -> bool {
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
        // 没有可作用实现的动作：**不消费**（见函数文档）。
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
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        if port
            .commit_ops(now_ms, "pencil: add note", vec![op])
            .is_err()
        {
            return;
        }
        // 重新投影, 让新音符出现在界面上（与 `refresh_undo` 同一手法）。
        if let Some(refreshed) = port
            .try_project()
            .and_then(|project| ViewState::from_project(&project).ok())
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
    match ViewState::from_project(&project) {
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
}
