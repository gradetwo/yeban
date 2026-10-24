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
use yeban_model::{EntityId, YebanProjectV1};

use crate::bridge::{CurrentScene, DEFAULT_TRACK_COLOR, RgbColor, TrackHeightLayout, ViewState};
use crate::engine_host::{
    DeviceOpening, EditMark, EngineHost, EngineHostError, HeartbeatReadings, SceneLaunchRequest,
};
use crate::input::{Action, Focus, InputContext, LogicalKey, Modifiers, Resolution, View};
use crate::meters::{MeterRuntime, MeterSnapshot, silent_snapshot};
use crate::scene::DemoScene;
use crate::ui::{MainWindow, ThemeState, YebanTheme};
use crate::undo::{UiAction, UndoPort};

/// [`wire_save`] 的**装配输入**：保存到哪 + 拿不到活会话时用哪一份工程。
///
/// 为什么把这两样捆成一个结构体：它们是"保存这个动作"在界面这一侧**只来自装配期**的
/// 全部输入，而运行期的两样（活会话 / 控制面权威）由 `wire_save` 的另外两个参数单独给
/// —— 于是"有权威时 `project` 字段不被使用"这件事在签名上是可见的，而不是埋在函数体里。
///
/// ⚠ 这里的那一份工程是**装配时刻的快照**，⛔ 不是"界面此刻的工程"。界面上的保存
/// 每次都优先从**活会话**（[`wire_save`] 的 `live_port`）现读工程与提交图谱，只有
/// 拿不到活会话时才退回这个快照（见 [`wire_save`] 的回调体）。
#[derive(Debug, Clone)]
pub struct SaveStatus {
    /// 保存目标（`None` = 这个会话没有磁盘对应物，保存会从此如实报错）。
    pub target: Option<std::path::PathBuf>,
    /// **装配时刻**的那一份工程（只在没有活会话可读、且没有控制面权威时被
    /// `dispatch_save` 使用）。
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
    // ⭐ `#82`：SFZ 采样器绑定的**只读**注入面（`TrackV3::sampler` 的路径原文，
    // `None` ⇒ `""`）。写方向是两条 `mixer-sampler-*` 回调（`wire_mixer_sampler`）。
    ui.set_track_sampler_paths(strings(&view.track_sampler_paths()));
    ui.set_scene_names(strings(&view.scene_names()));
    // ⭐ 任务单 `#283`：**当前场景**的唯一注入点（完整重投影 ⇒ 回到投影的读数；
    // 场景按钮点击走同一个写者 [`apply_current_scene`]，⛔ 不各写一份）。
    apply_current_scene(ui, view.current_scene.as_ref());
    ui.set_section_names(strings(&view.section_names()));
    ui.set_clip_ulids(strings(&view.clip_ulids()));
    ui.set_clip_labels(strings(&view.clip_labels()));
    ui.set_clip_positions(lengths(&view.clip_positions()));
    ui.set_clip_widths(lengths(&view.clip_widths()));
    // ⭐ 素材池（任务单 `#133`）：三个平行数组全部来自 `ViewState::asset_pool`
    // （`clip_pool` 的**唯一**投影产物，见 `bridge.rs` 的 `AssetPoolRowView`）。
    // 它们只在完整重投影（换工程 / 撤销 / 打开）时变化，但 `apply_view` 每次都会新建
    // `VecModel` ⇒ 计入 `PER_FRAME_ARRAY_REWRITE_BUDGET`（见 `tests/perf_draw_budget.rs`
    // 的读数与注释）。
    ui.set_asset_pool_names(strings(&view.asset_pool_names()));
    ui.set_asset_pool_kinds(strings(&view.asset_pool_kinds()));
    ui.set_asset_pool_details(strings(&view.asset_pool_details()));
    // ⭐ 设备机架（任务单 `#157`）：`ViewState::devices` / `eq_low_gain` 是**唯一**事实源。
    // 四个平行数组同索引集；五个标量来自同一次投影。它们只在完整重投影（换工程 /
    // 撤销 / 打开 / 一次设备手势提交之后）时变化，但 `apply_view` 每次都会新建
    // `VecModel` ⇒ 计入 `PER_FRAME_ARRAY_REWRITE_BUDGET`（见 `tests/perf_draw_budget.rs`
    // 的读数与注释）。
    ui.set_device_track_label(view.device_track_label.clone().into());
    ui.set_device_names(strings(&view.device_names()));
    ui.set_device_ids(strings(&view.device_ids()));
    ui.set_device_bypassed(booleans(&view.device_bypassed()));
    ui.set_device_recognised(strings(&view.device_recognised()));
    ui.set_device_unrecognised(strings(&view.device_unrecognised()));
    // ⭐ 任务单 `#180`：**主总线**设备链 —— 设备架面板的第二段（可编辑面：装 / 删）。
    // 四个平行数组同索引集（名字 / 身份 / 已识别 / 未识别）＋ 三个标量（标题 / 可用性 /
    // 逐字原因）。唯一事实源是 `ViewState::master_devices` 与同名字段；⛔ 界面不拼身份、
    // ⛔ 不持有第二份设备列表。它们只在完整重投影上变化，但 `apply_view` 每次都会新建
    // `VecModel` ⇒ 计入 `PER_FRAME_ARRAY_REWRITE_BUDGET`（见 `tests/perf_draw_budget.rs`
    // 的读数与注释）。
    ui.set_master_device_label(view.master_device_label.clone().into());
    ui.set_master_device_names(strings(&view.master_device_names()));
    ui.set_master_device_ids(strings(&view.master_device_ids()));
    ui.set_master_device_recognised(strings(&view.master_device_recognised()));
    ui.set_master_device_unrecognised(strings(&view.master_device_unrecognised()));
    ui.set_master_device_insert_available(view.master_device_insert_available);
    ui.set_master_device_insert_reason(view.master_device_insert_reason.clone().into());
    // ⭐ 任务单 `#181`：**主总线**设备的两个可编辑面（旁通 / EQ 低架增益）。
    // 一个平行数组（逐台旁通位，与 `master-device-names` 同索引集）＋ 五个标量
    // （可用性 / 当前值 / 值域下界 / 值域上界 / 逐字原因）。唯一事实源是
    // `ViewState::master_devices` 与 `ViewState::master_eq_low_gain` 及其访问器；
    // 值域**只**来自效果器契约（`effect_params::recognise` 的 `spec.min/max`），
    // ⛔ 界面不造第二份表。这一个数组计入 `PER_FRAME_ARRAY_REWRITE_BUDGET`。
    ui.set_master_device_bypassed(booleans(&view.master_device_bypassed()));
    ui.set_master_device_eq_available(view.master_device_eq_available());
    ui.set_master_device_eq_gain_db(view.master_device_eq_gain_db());
    ui.set_master_device_eq_min_db(view.master_device_eq_min_db());
    ui.set_master_device_eq_max_db(view.master_device_eq_max_db());
    ui.set_master_device_eq_reason(view.master_device_eq_reason.clone().into());
    // ⭐ 任务单 `#182`：两个 EQ 滑块的**设备身份**（`""` = 没有目标）。手势把它原样带回
    // 宿主（`device-eq-grab(device-id, fraction)`），宿主在按下 / 松手两侧按身份重解析槽位
    // ⇒ 投影与点击之间链被增删也⛔ 不会改到另一台。两个**标量**，⛔ 没有新数组。
    ui.set_master_device_eq_device_id(view.master_device_eq_device_id().into());
    ui.set_device_eq_available(view.device_eq_available());
    ui.set_device_eq_gain_db(view.device_eq_gain_db());
    ui.set_device_eq_min_db(view.device_eq_min_db());
    ui.set_device_eq_max_db(view.device_eq_max_db());
    ui.set_device_eq_device_id(view.device_eq_device_id().into());
    // ⭐ 设备增删 ＋ 建空片段（任务单 `#163`）：四个**标量**（可用性 ＋ 逐字原因）。
    // ⛔ 没有新数组（设备 / 片段数都由既有的 `device-*` / 工程投影反映）⇒ 只增
    // `APPLY_VIEW_WRITE_SITE_BUDGET`。唯一事实源是 `ViewState` 的同名字段。
    ui.set_device_insert_available(view.device_insert_available);
    ui.set_device_insert_reason(view.device_insert_reason.clone().into());
    ui.set_clip_create_available(view.clip_create_available);
    ui.set_clip_create_reason(view.clip_create_reason.clone().into());
    // ⭐ 任务单 `#164`：一键装乐器的投影 —— 一个数组（按钮名）＋ 三个标量（当前乐器 /
    // 可用性 / 逐字原因）。唯一事实源是 `ViewState` 的同名字段；⛔ 界面不持有第二份
    // 路径表，手势只上报下标，路径由宿主从 `bridge::builtin_instrument` 求。
    // ⭐ `#165`：这个数组是**按样本字节过滤后**的可装清单（今天为空 ⇒ 0 个按钮）；
    // 写点没有增减（数组仍在这里换实例），预算读数见 `tests/perf_draw_budget.rs`。
    ui.set_instrument_names(strings(&view.instrument_names));
    ui.set_instrument_label(view.instrument_label.clone().into());
    ui.set_instrument_install_available(view.instrument_install_available);
    ui.set_instrument_install_reason(view.instrument_install_reason.clone().into());
    // ⭐ 任务单 `#196`：采样库的**投影读数**（是否已安装 / 逐字提示 / 按钮可见性 / 预设路径）。
    // 四个都是标量 ⇒ 只增 `APPLY_VIEW_WRITE_SITE_BUDGET`，⛔ 没有新数组。
    // ⚠ 运行期的下载状态（"下载中…" / 结果）**不在这里**：它住在
    // `sample-library-status`，只由下载回调写（换工程 / 撤销不会清掉一条正在进行的回执）。
    ui.set_sample_library_installed(view.sample_library_installed);
    ui.set_sample_library_notice(view.sample_library_notice.clone().into());
    ui.set_sample_library_download_visible(view.sample_library_download_visible);
    ui.set_sample_library_root(view.sample_library_root.clone().into());
    // ⭐ 发送条（任务单 `#159`）：`ViewState::sends` 是**唯一**事实源。四个平行数组同
    // 索引集（边身份 / 目的地 / 增益文本 / 滑块值）；四个标量给值域与"新建发送"的可用性。
    // 值域**来自模型**（`bridge::send_gain_range` ⇒ `NOMINAL_GAIN_MIN_DB/MAX_DB`），
    // ⛔ 不是界面里的第二份范围表。它们只在完整重投影（换工程 / 撤销 / 打开 / 一次发送
    // 手势提交之后）时变化，但 `apply_view` 每次都会新建 `VecModel` ⇒ 计入
    // `PER_FRAME_ARRAY_REWRITE_BUDGET`（见 `tests/perf_draw_budget.rs` 的读数与注释）。
    ui.set_send_edge_ids(strings(&view.send_edge_ids()));
    ui.set_send_destination_names(strings(&view.send_destination_names()));
    ui.set_send_gain_labels(strings(&view.send_gain_labels()));
    ui.set_send_gain_dbs(lengths(&view.send_gain_dbs()));
    ui.set_send_gain_min_db(view.send_gain_min_db());
    ui.set_send_gain_max_db(view.send_gain_max_db());
    ui.set_send_create_available(view.send_create_available);
    ui.set_send_create_reason(view.send_create_reason.clone().into());
    // ⭐ 轨 / 路由（任务单 `#160`）：显示轨数 ＋ 建轨 / "删除展示轨"的可用性与逐字原因
    // （主总线说明那一行兼作"没有主总线 ⇒ 不能建轨"的原因）。五个都是**标量**（不是新
    // 数组）⇒ 只增 `APPLY_VIEW_WRITE_SITE_BUDGET`，不动
    // `PER_FRAME_ARRAY_REWRITE_BUDGET`。唯一事实源是 `ViewState` 的同名字段。
    ui.set_routing_count_label(view.track_count_label.clone().into());
    ui.set_routing_create_available(view.track_create_available);
    ui.set_routing_master_label(view.master_bus_label.clone().into());
    ui.set_routing_delete_available(view.track_delete_available);
    ui.set_routing_delete_reason(view.track_delete_reason.clone().into());
    ui.set_section_positions(lengths(&view.section_positions()));
    ui.set_section_widths(lengths(&view.section_widths()));
    ui.set_bar_positions(lengths(&view.bar_positions));
    // `[ROAD-M3-002 / BASELINE-003]` **视口裁剪**：只注入可见窗口内的音符。
    // 五个平行数组（ulids / velocities / positions / widths / ys）必须共用**同一**索引集，
    // 否则语义 ID 与力度会和几何错位 —— 所以用 `visible_notes` 一次取走，而不是各数组各裁一遍。
    // `VisibleNotes::rows`（音高车道索引）**有意不注入**：纵向几何已由投影算进 `ys`
    // （`pitch_lane` 一次乘法），宿主与 `.slint` 两侧都没有车道索引的消费者
    // ⇒ 登记与判据见 `bridge.rs` 的 `VisibleNotes::rows` 与 `tests/roll_injection_registration.rs`
    // （台账第 283 条：生产读点 0；改这里之前先读那条登记）。
    // 位置**相对视口**（投影侧已减去 `scroll_x`）⇒ `.slint` 不做位置算术, 事件路径传 0.0。
    let visible = view.visible_notes(scroll_x, viewport_width);
    // 选中标志与其它数组**同一索引集**（第六个：五个平行数组之后的那一个）：每次注入都从界面上的 `selected-ulids` 重算,
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
    // 状态栏：与上面同一份投影（选区跨度 + 工程音频配置）。台账 R9 —— 见 `publish_status_bar`。
    publish_status_bar(ui, view);
}

/// ⭐ 任务单 `#283`：**当前场景**的唯一写者（Session View 的高亮 + 无障碍选中态）。
///
/// 读数是 [`ViewState::current_scene`]（[`CurrentScene`]）：投影一律给 `None`，
/// 场景按钮的宿主回调（[`wire_scene_launch`]）成功后写进视图并调本函数。
///
/// ⛔ 不写场景名第二份：`.slint` 按 `current-scene-index` 在 `scene-names` 里取名字
/// ⇒ "高亮的那一条"与"显示的名字"不可能指向两个场景。
pub fn apply_current_scene(ui: &MainWindow, current: Option<&CurrentScene>) {
    ui.set_current_scene_index(current.map_or(-1, CurrentScene::ui_index));
    ui.set_current_scene_name(
        current
            .map_or_else(String::new, |scene| scene.name.clone())
            .into(),
    );
    // 回执与"当前场景"属于同一次启动：读数被清回 `None`（换工程 / 打开工程）时，
    // 上一次的"已排队"不再适用 ⇒ 一并作废。点击那条路先清、再由回调写新回执。
    if current.is_none() {
        ui.set_scene_launch_status("".into());
    }
}

/// ⭐ 任务单 `#283`：把 **Session View 的场景按钮**接到引擎的量化启动上。
///
/// ## 这条链（每一环都是既有的那一个）
///
/// `scene-launch-<i>-button`（`.slint` 的 `TouchArea.clicked` ＋
/// `accessible-action-default`，**同一落点**）
/// → `MainWindow::on_scene_launch`（本函数）
/// → [`EngineHost::queue_scene_launch`]（控制驱动 ⇒ 直接调
/// `EngineRuntime::queue_scene_launch`；设备腿 ⇒ 既有事件环，见
/// [`crate::engine_host::SceneLaunchDisposition`]）
/// → 引擎在下一个**小节线**（或 `Off` 档的当前帧）生效：`transport.retune(场景 tempo)`
/// ＋ `stats().scene_launches_applied += 1`。
///
/// ## 为什么"当前场景"在这里才写
///
/// 只有**已送到引擎**的请求才改读数（`SceneLaunchDisposition::is_delivered()`）：
/// 没有引擎 / 事件环满时界面照旧、`scene_launch_status` 如实给出原因 ——
/// ⛔ 不像"假装启动了一个场景"那样把按钮做成死按钮的另一种形态。
///
/// ⚠ 三个数据都从**视图**现读（量化档来自工程投影，tempo 覆盖来自场景，拍号来自工程）
/// ⇒ 换工程 / 改走带配置之后点按钮用的是**当时**那一份，⛔ 不用装配期的快照。
///
/// ⚠ 作用域纪律：引擎的 `RefCell` 借用在写任何界面属性**之前**释放（`#212` 的崩法）。
pub fn wire_scene_launch(
    ui: &MainWindow,
    engine: Rc<RefCell<EngineHost>>,
    view: Rc<RefCell<ViewState>>,
) {
    let weak = slint::ComponentHandle::as_weak(ui);
    ui.on_scene_launch(move |scene_index| {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        // 下标来自 `.slint` 的导航循环 ⇒ 先按**当前**投影解析成身份与 tempo。
        let Ok(index) = usize::try_from(scene_index) else {
            set_scene_launch_status(&ui, "场景下标非法：没有启动任何场景");
            return;
        };
        let request = {
            let view = view.borrow();
            let Some(scene) = view.scenes.get(index) else {
                set_scene_launch_status(&ui, "这个场景已经不在当前工程里（投影之后被删？）");
                return;
            };
            SceneLaunchRequest {
                // 引擎只把身份当读数；解析失败不阻止启动（tempo 才是可听的那一半）。
                scene_id: scene.id.parse().ok(),
                tempo_bpm: scene.tempo.unwrap_or(view.bpm),
                quantization: view.launch_quantization,
                time_signature: yeban_model::TimeSignature {
                    numerator: view.time_signature_numerator,
                    denominator: view.time_signature_denominator,
                },
            }
        };
        let (disposition, name) = {
            let mut engine = engine.borrow_mut();
            let disposition = engine.queue_scene_launch(request);
            let name = view
                .borrow()
                .scenes
                .get(index)
                .map(|scene| scene.name.clone());
            (disposition, name)
        };
        if !disposition.is_delivered() {
            let reason = match disposition {
                crate::engine_host::SceneLaunchDisposition::EventQueueFull => {
                    "事件环满：这次启动没有送到引擎（再按一次）"
                }
                crate::engine_host::SceneLaunchDisposition::NoEngine => {
                    "引擎没起来：这次启动没有送到引擎"
                }
                _ => "启动没有送到引擎",
            };
            set_scene_launch_status(&ui, reason);
            return;
        }
        let Some(name) = name else {
            // 投递成功之后投影换了（同一线程上不可能，除非回调里有人改了视图）⇒ 不改读数。
            return;
        };
        {
            let mut view = view.borrow_mut();
            let id = view.scenes[index].id.clone();
            view.current_scene = Some(CurrentScene {
                index,
                id,
                name: name.clone(),
            });
            apply_current_scene(&ui, view.current_scene.as_ref());
        }
        set_scene_launch_status(&ui, &format!("已排队：{name}"));
    });
}

/// [`wire_scene_launch`] 的状态行（**如实**报告"送到没有"）。
///
/// 它只写一个文本属性；默认是 `""`（没有启动过 ⇒ 不占视觉空间）。
fn set_scene_launch_status(ui: &MainWindow, text: &str) {
    ui.set_scene_launch_status(text.into());
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

// ===========================================================================
// 横向缩放（`Z` / `Shift+Z`）的**视图态**：一个属性、两个函数
// ===========================================================================
//
// 数据流与行高**同款**（"宿主拥有、重新注入"）：
//
// ```text
// MainWindow.roll-ticks-per-pixel              ← host::set_roll_ticks_per_pixel（唯一写点）
//   └─ host::roll_ticks_per_pixel(ui)          ← 唯一读点（读回 + 回退到默认）
//        └─ host::project_with_view_state(..)  ← 唯一的"窗口 → 投影"入口
//             └─ host::apply_view              ← 注入 x / 宽度 / 小节线
// ```
//
// 它是**视图态**（`ADR-0004` Q5：横向缩放的数字归宿主 / 会话暂态），因此：
// 零 schema、不进 `.yeban`、重启丢失、**不进撤销栈**（撤销是工程内容的事）。

/// 时间轴当前的横向缩放（`ticks_per_pixel`）—— 窗口属性的**唯一读点**。
///
/// 回退规则（两条都是决定，不是意外）：
///
/// - 属性 `<= 0`（含 Slint 的默认 0 与任何外力写入）⇒ [`crate::bridge::DEFAULT_TICKS_PER_PIXEL`]；
/// - 属性超出 [`crate::bridge::MIN_TICKS_PER_PIXEL`]..=[`crate::bridge::MAX_TICKS_PER_PIXEL`]
///   ⇒ 夹到区间内（走 [`crate::bridge::clamp_ticks_per_pixel`]，**同一个**夹紧函数）。
///
/// 这样"没人碰过缩放"的路径与 `from_project_with_layout` **逐位**同值（默认 30），
/// 因此默认外观不引入任何渲染差异。
#[must_use]
pub fn roll_ticks_per_pixel(ui: &MainWindow) -> u64 {
    let raw = ui.get_roll_ticks_per_pixel();
    match u64::try_from(raw) {
        Ok(value) if value > 0 => crate::bridge::clamp_ticks_per_pixel(value),
        _ => crate::bridge::DEFAULT_TICKS_PER_PIXEL,
    }
}

/// 写时间轴横向缩放（整数 tick / 逻辑像素）—— 唯一的写点。
///
/// 返回"是否真的变了"（与 [`set_track_height_override`] 同款：`false` = 一位没变，
/// 调用方不必重投影）。写入前先夹紧（[`crate::bridge::clamp_ticks_per_pixel`]），
/// 因此属性里**永远**是一个投影能接受的合法值。
///
/// **不重投影**：重投影由调用方走既有路径（缩放动作走 [`project_with_view_state`]），
/// 因为"画哪一份工程"不属于视图态（本函数拿不到工程）。
pub fn set_roll_ticks_per_pixel(ui: &MainWindow, ticks_per_pixel: u64) -> bool {
    let clamped = crate::bridge::clamp_ticks_per_pixel(ticks_per_pixel);
    let Ok(value) = i32::try_from(clamped) else {
        return false;
    };
    if ui.get_roll_ticks_per_pixel() == value {
        return false;
    }
    ui.set_roll_ticks_per_pixel(value);
    true
}

/// **窗口的视图态 → 一份投影**（唯一的"从窗口造投影"入口）。
///
/// 它把**三个**视图态拼在一起：横向缩放（[`roll_ticks_per_pixel`]）、行高布局
/// （[`track_height_layout`]）与 ⭐ 选中轨（[`selected_track_id`]，任务单 `#192`），
/// 一起交给 [`ViewState::from_project_with_zoom_layout_and_selection`]。
/// 存在的理由是**别让每个重投影点各拼一遍** —— 拼漏一处，那一条路径就会
/// 把用户调过的缩放 / 行高 / 选中轨静默清零（行高在 `ADR-0004` S1 已经付过同一种学费）。
///
/// # Errors
///
/// 见 [`crate::bridge::BridgeError`]（缩放为 0 / 拍号退化 / 溢出）。
pub fn project_with_view_state(
    project: &yeban_model::YebanProjectV1,
    ui: &MainWindow,
) -> Result<ViewState, crate::bridge::BridgeError> {
    let selected = selected_track_id(ui);
    ViewState::from_project_with_zoom_layout_and_selection(
        project,
        roll_ticks_per_pixel(ui),
        &track_height_layout(ui),
        selected.as_deref(),
    )
}

/// ⭐ 任务单 `#192`：窗口上的**选中轨原文**（编辑器视图态）—— 唯一的读点。
///
/// 事实源是窗口属性 `selected-track-id`（默认空串 = 没有明确选中 ⇒ 面板走兜底）。
/// **空串 ⇒ `None`**，于是"没点过任何轨"与"点了一条已经不存在的轨"由
/// [`crate::bridge::display_track`] 的兜底规则**同一个出口**处理。
///
/// ⛔ 它不是工程内容：不进 `.yeban`、不进 `projectDigest`、不进撤销栈、不碰
/// `UndoPort::commit_ops`（判据：选中前后的 `commit_count` / `undoable` / 摘要全 Δ0）。
/// ⛔ 它也不在这里被"归一化"成一条现存的轨 —— 归一化会把"被删的选中轨在撤销后回来"
/// 这条恢复路径一起抹掉；面板的回落由 [`crate::bridge::display_track`] 每次解析时给出。
#[must_use]
fn selected_track_id(ui: &MainWindow) -> Option<String> {
    let id = ui.get_selected_track_id();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// ⭐ 任务单 `#192`：面板服务的那条轨 —— 宿主侧**唯一**的解析点。
///
/// 与投影里的解析是**同一个** [`crate::bridge::display_track`] ⇒ 界面显示哪条轨
/// 与设备 / 发送 / 片段 / 铅笔提交到哪条轨不可能各说一套。
fn display_track<'a>(
    ui: &MainWindow,
    project: &'a yeban_model::YebanProjectV1,
) -> Option<&'a yeban_model::project::TrackV3> {
    let selected = selected_track_id(ui);
    crate::bridge::display_track(project, selected.as_deref())
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
    // 从**窗口**造投影（`project_with_view_state`）：行高与横向缩放一起读回来 ——
    // 只读一个的话，拖动行高会把用户调过的缩放静默清零。
    match project_with_view_state(&project, ui) {
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

// ===========================================================================
// ⭐ 任务单 `#192`：**选中轨**（编辑器视图态）的接线
// ===========================================================================
//
// 数据流（与横向缩放 / 行高**同一款**："宿主拥有、重新注入"）：
//
// ```text
// arrangement_view.slint 的轨道头（TouchArea 的 pointer down / 无障碍默认动作）
//   └─ track-select(index)                    ← 界面只报**下标**
//        └─ host::wire_track_select           ← 下标 → 身份（track-ids，唯一查表）
//             ├─ ui.set_selected-track-id(id) ← 视图态的唯一写点
//             └─ project_with_view_state      ← 选中轨 + 缩放 + 行高 → 一次投影
//                  └─ apply_view              ← 设备架 / 发送条 / 轨路由 / 乐器 / 可用性
// ```
//
// ⛔ 本函数**不**调用 `UndoPort::commit_ops`：选中轨是**视图状态**（与滚动位置、缩放、
// 行高同级），⛔ 不进 `.yeban`、⛔ 不进 `projectDigest`、⛔ 不进撤销栈 —— 否则"每点一下
// 轨道头就多一步撤销"、摘要会因为点了一下就变。判据在 `tests/track_selection.rs`。
//
// 兜底不在**这里**做：`selected-track-id` 保留用户点的**原文**，投影每次经
// [`crate::bridge::display_track`] 解析 ⇒ 选中轨被删 / 被撤销掉 / 换工程时自然回落到
// 身份序第一条非主总线轨，⛔ 不 panic、⛔ 不空面板（原因由各面板的可用性字段给出）。
// 保留原文还有一个可观测的好处：撤销掉的那条轨若被重做回来（身份相同），选中态自动恢复。

/// ⭐ 任务单 `#192`：轨道头点击 → **选中那条轨**（唯一写点），然后重投影。
///
/// 界面报的是**下标**（`track-{i}-header` 的 `i`）；宿主用 `apply_view` 注入的
/// `track-ids` 把下标翻成**身份**（⛔ 不拿装配期快照猜）—— 与行高拖拽同一条"下标 → 身份"
/// 查表（[`track_id_at`]）。下标越界 / 身份为空 ⇒ 什么都不做（⛔ 不 panic）。
///
/// 点的是**已经选中**的那条轨 ⇒ 不重投影（画面本来就对，与 `zoom_action` 的"已经有这个
/// 缩放"同一条取向）。
pub fn wire_track_select(ui: &MainWindow, port: &Rc<UndoPort>) {
    let weak = ui.as_weak();
    ui.on_track_select({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move |index| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // 下标 → 身份：唯一查表（与行高拖拽共用 `track_id_at`）。
            let Some(track_id) = track_id_at(&ui, index) else {
                return;
            };
            // 同一条轨再点一次 = 一位不动（⛔ 不白重投影一帧）。
            if selected_track_id(&ui).as_deref() == Some(track_id.as_str()) {
                return;
            }
            ui.set_selected_track_id(track_id.into());
            // 权威会话可能被控制面关掉工程（`try_project` 如实返回 `None`）——
            // 那时**出声不画**：选中态已经记在窗口上，下一次投影会带上它。
            let Some(project) = port.try_project() else {
                eprintln!("[yeban-app] 选轨：权威会话此刻没有活跃工程（选中态已记下）");
                return;
            };
            match project_with_view_state(&project, &ui) {
                Ok(view) => {
                    // 与撤销刷新同款：复用当前横向偏移，选轨⛔ 不把卷帘滚回起点。
                    let scroll_x = ui.get_roll_scroll_x();
                    apply_view(&ui, &view, ui.window().size().width as f32, scroll_x);
                }
                Err(error) => {
                    // 投影失败**出声**：选中态在内存里已经变了，但这一帧画不出来。
                    eprintln!("[yeban-app] 选轨后重新投影失败: {error}");
                }
            }
        }
    });
}

// ===========================================================================
// 混音台（控制台 Tab 2）的**写入面**：推子 / 声相 / 静音 / 独奏
// ===========================================================================
//
// 缺口（本切片补的那一格）：`crates/yeban-app/ui/console/mixer_console.slint` 的
// `TouchArea` 数量在本次之前是 **0** —— 推子帽、静音、独奏都是纯 `Rectangle`，
// 于是"音量 / 声相 / 静音 / 独奏"四项在界面上**完全不可操作**。
//
// ## 权威链（`ADR-0005`：工程内容必须走 `Op` + `UndoPort`）
//
// ```text
// mixer_console.slint 的 TouchArea
//   --mixer-fader-grab/drag/release--+--mixer-pan-grab/drag/release--+--mixer-mute/solo-toggle-->
//        └─ host::wire_mixer_edit（本文件，唯一实现）
//             ├─ 拖动期：**只**写视图态（`track-volumes` / `track-volume-fractions` / `track-pans`）
//             └─ 松手 / 点击：`UndoPort::commit_ops(vec![Op])` —— **恰一次**
//                  └─ `refresh_undo_window` 重投影（值回到"从工程读出来"的形态）
// ```
//
// ## 三条收尾路径（一条都不能少）
//
// | 路径 | 谁发出 | 语义 |
// | :--- | :--- | :--- |
// | 松手 | `.slint` 的 `PointerEventKind.up` | 这一拖算数（`commit_ops` 一次） |
// | 指针离开窗口 | 上游导出的 `PointerEventKind.cancel` | 这一拖算数（最后位置）；既有语义，未改 |
// | `Escape` | `Action::Cancel` → `mixer-cancel-gesture` → [`cancel_mixer_gesture`] | **取消**这一拖（写回起点值，**不提交**） |
//
// 三条都汇到 [`end_mixer_gesture`]：**手势记录与 `mixer-drag-active` 在那一处一起清零**，
// 因此不存在"拖到一半松在窗口外 ⇒ 下一次移动继续改音量"这种粘住的手势态。
//
// ## 为什么"一次拖动 = 一次提交"（照抄 `wire_track_height_drag` 的形状）
//
// 每一个 `move` 都提交会把"拖一次推子"碎成几百步撤销，用户按一次 `Cmd+Z` 只回退一个像素。
// 因此：`drag` 只改视图态（数值文本与推子帽位置**立即**跟着动），`release` 才提交一次。
// 这与 `host::wire_track_height_drag`（`host.rs:432` 起）的形状逐条相同。
//
// ## 为什么不用 `SetParam` 表达静音 / 独奏
//
// `Op::SetParam`（`crates/yeban-model/src/ops.rs:315`）的两个载荷都是 `f32`，
// 而 `TrackV3::mute` / `TrackV3::solo` 是 `bool`（`crates/yeban-model/src/project.rs:1130` /
// `:1132`），且 `AutomationTarget`（`project.rs:512`）没有布尔变体 ⇒ 静音 / 独奏必须有自己的
// `Op`（`Op::SetTrackMute` / `Op::SetTrackSolo`，本切片新增；裁决见交付报告）。
// 音量 / 声相**复用**既有的 `Op::SetParam` + `AutomationTarget::TrackVolume` / `TrackPan`。
//
// ## 主控通道条（`mixer-master-*`）：接了**音量 ＋ 声相**，没接**静音 / 独奏**
//
// 主总线是一条普通的 `TrackV3`（`TrackKind::Master`，`crates/yeban-model/src/project.rs`
// 的 `TrackV3::kind`），它同样有 `volume_db` / `pan` / `mute` / `solo`
// （同文件 `project.rs:1125-1132`）⇒ 既有 `Op` **够用**，不需要新的 `Op`、也不需要动
// `crates/yeban-model/**`：`Op::SetParam` 的 `read_param` / `write_param`
// （`ops.rs:1555-1556` / `ops.rs:1596-1600`）只按 `track_id` 取轨道
// （`YebanProjectV1::track` / `track_mut`，`project.rs:1765/1776`），**没有任何**"排除主总线"
// 的前置条件；`validate_param_value`（`ops.rs:1655`）也只管值域。
//
// 主控**只接音量与声相**，理由（静音 / 独奏的语义在本仓库里**不成立**）：
// - 本仓库的规范文档没有主控静音 / 独奏的定义（`grep -n "solo" docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`
//   零命中）⇒ 没有可引用的明文语义；
// - **独奏会主动制造错误结果**：`any_solo = project.tracks.values().any(|track| track.solo)`
//   （`crates/yeban-engine/src/snapshot.rs:645`）把主总线的 `solo` 也算进去，而
//   `track_is_audible(mute, solo, solo_safe, any_solo) = !mute && (!any_solo || solo || solo_safe)`
//   （`synth.rs:1194`）⇒ 主总线一独奏，**每一条别的轨道都变成不可闻**，母线只剩静音；
// - **静音是空操作**：母线自己不产生音符（`project_schedules` 只按 `track.clips` 生成调度表），
//   而母线的渲染与电平都与 `TrackV3::mute` 无关 ⇒ 那个开关改了模型却什么也不改。
// 因此这两项**登记为缺口**，不接线：`.slint` 里那两个 `Rectangle` 保持原样
// （没有 `TouchArea`，与接线之前**外观和行为都逐字节相同**），不做一个"点了没反应"的假入口。
//
// ⚠ 未接线的另一半（**引擎侧**，不是本文件的问题，登记在交付报告里）：引擎今天
// **不消费**主总线的 `volume_db` / `pan` —— `crates/yeban-engine/src/rt.rs:752` 的声相表
// 明确 `if *id == master … continue`，母线也没有自己的增益级。因此主控推子的写入面是
// "模型 ＋ 视图态 + 撤销栈"闭环，**听觉效果未接线**。

/// 推子面的**像素行程**（逻辑像素）。
///
/// 与 `.slint` 的推子帽几何**同源**：`mixer_console.slint` 里推子帽的
/// `y: 60px + 88px * (1.0 - root.track-volume-fractions[track_index])`
/// ⇒ 走完 0.0–1.0 的位置正好 88px。两处必须一致，否则"拖 1 像素"在数值上不等于
/// "推子帽动 1 像素"（判据 `the_mixer_fader_travel_matches_the_slint_geometry` 钉住那个字面量）。
pub const MIXER_FADER_TRAVEL_PX: f32 = 88.0;

/// 声相面的**像素行程**（逻辑像素）＝ `mixer_console.slint` 里声相 `TouchArea` 的宽度。
///
/// 口径：走完这个宽度 = `pan` 从 `-1.0` 走到 `+1.0`（2.0 的跨度）。
pub const MIXER_PAN_TRAVEL_PX: f32 = 36.0;

/// 一次混音手势**寻址的目标** —— 通道条（下标 ＋ 身份）或主控通道条（只有身份）。
///
/// ## 为什么主控没有下标（本切片的测量结论）
///
/// 主总线**不在** `track-names` / `track-ids` / `track-volumes` / `track-pans` 这些平行
/// 数组里：投影刻意排除它（[`crate::bridge::ViewState::tracks`] 只装非主总线轨道；
/// `bridge.rs` 的 `track_rows_with_layout` 与 `ViewState::track_names` 都按
/// `master_bus_track_id` 过滤），它单独走 `ViewState::master: Option<TrackView>`。
/// 因此"按下标寻址"这条路对主控**不存在** —— `0..len` 里没有一个格子是主控。
///
/// ## 方案：主控走自己的一组回调，身份从权威工程取（不做身份→下标算术）
///
/// `.slint` 侧给主控声明 `mixer-master-fader-grab(length)` 等 6 个**不带下标**的回调，
/// 宿主在**手势开始**那一刻从权威工程取身份
/// （`UndoPort::try_project().master_bus_track_id`，要求非 nil 且真的在 `tracks` 里）。
/// 于是：
/// - `.slint` 里**没有任何**身份→下标的算术（[`apply_row_geometry`] 之后的同一条纪律）；
/// - 不发明哨兵下标（`u32::MAX` / `-1`）：哨兵会与"真实下标"共用同一个参数位，
///   读代码的人必须记住一个魔法值，而且 `track_id_at(哨兵)` 只会静默返回 `None`；
/// - 身份**每次按下都重新读**（不是装配期快照），换工程之后不会拿旧身份去改新工程。
#[derive(Clone, Debug, PartialEq)]
enum MixerTarget {
    /// 通道条：下标（写视图态那一格用）＋ 身份（提交用）。
    Track {
        /// 本次手势按下那一轨的**下标**（`track-volumes` / `track-pans` 的索引集）。
        index: i32,
        /// 本次手势按下那一轨的身份（26 字符 `EntityId` 文本）。
        track_id: String,
    },
    /// 主控通道条：**只有身份**（理由见本枚举的文档）。
    Master {
        /// `master_bus_track_id` 的 26 字符 `EntityId` 文本。
        track_id: String,
    },
}

/// 一次混音手势（推子 / 声相）的状态。
///
/// 为什么住在宿主而不是界面上：它只在**一次拖动**里有意义，而且必须在 `release`
/// 那一刻拿得到"这一拖从哪开始"（提交的 `old_val` 就是它）。
///
/// `Escape`（`Action::Cancel`）要**取消**这一拖，于是"手势是否还没结束"必须有**一个
/// 窗口上的读数**（`mixer-drag-active`）—— 与 `track-height-drag-active` 同一个角色。
/// 取消本身走窗口回调 `mixer-cancel-gesture`，落回持有这份记录的**同一个闭包**
/// （[`cancel_mixer_gesture`]）：记录因此只有一份，没有第二份状态要同步。
///
/// 寻址的目标是 [`MixerTarget`]（通道条**下标**或主控**身份**）。
#[derive(Clone, Debug, PartialEq)]
enum MixerGesture {
    /// 没有进行中的手势。
    Idle,
    /// 推子拖动中。
    Fader {
        /// 本次手势寻址的目标（通道条 / 主控）。
        target: MixerTarget,
        /// 手势开始时的音量 (dB) —— 提交时的 `old_val`。
        start_db: f32,
        /// 按下时的指针 y（推子面坐标系）。
        start_y: f32,
        /// 最后一次 `drag` 算出来的音量（提交时的 `new_val`）。
        ///
        /// 为什么把结果**存下来**而不是松手时从界面读回来：界面上的文本是 `{:.1}` 的
        /// 有损形态，读回来会把"用户拖出来的值"改成另一个数。存下来 ⇒ "用户看到的数"
        /// 与"提交的数"是同一个 `f32`（显示是它的函数）。
        last_db: f32,
        /// 这一拖是否**真的动过**（收到过至少一个 `drag`）。
        ///
        /// 为什么需要它：`volume_fraction` ⇄ dB 的往返在 f32 下**不是**恒等
        /// （实测 `-3.2` ⇒ 位置 ⇒ 回 dB 差 3 ulp）。因此"动过没有"是**事件事实**，
        /// 不是数值比较；没有它，"按下就松手"会提交一步什么都没改的编辑。
        moved: bool,
    },
    /// 声相拖动中。
    Pan {
        /// 本次手势寻址的目标（理由同 [`MixerGesture::Fader::target`]）。
        target: MixerTarget,
        /// 手势开始时的声相 -1.0..=1.0 —— 提交时的 `old_val`。
        start_pan: f32,
        /// 按下时的指针 x（声相面坐标系）。
        start_x: f32,
        /// 最后一次 `drag` 算出来的声相（提交时的 `new_val`；理由同
        /// [`MixerGesture::Fader::last_db`]）。
        last_pan: f32,
        /// 这一拖是否真的动过（理由同 [`MixerGesture::Fader::moved`]）。
        moved: bool,
    },
}

/// 推子：起点**位置**（0.0–1.0）与纵向位移 ⇒ 新位置（纯函数，可单独判据）。
///
/// 这是"推子帽动 n 像素"与"数值动多少"的**唯一**加法点：`.slint` 用
/// [`crate::bridge::volume_fraction`] 把位置画成像素，宿主用本函数把像素变回位置。
/// 夹紧只发生一次（位置夹到 `0.0..=1.0`，等价于 dB 夹到量程两端）。
///
/// `delta_px == 0.0` 时**逐位**返回起点：否则"拖出去再拖回来"会因为多一次
/// `f32` 加法而差几个 ulp，进而提交一步"数值没变"的编辑（实测 3 ulp）。
#[must_use]
pub fn dragged_fader_fraction(start_fraction: f32, delta_px: f32) -> f32 {
    let start = start_fraction.clamp(0.0, 1.0);
    if delta_px == 0.0 {
        return start;
    }
    if !delta_px.is_finite() {
        return start;
    }
    (start + delta_px / MIXER_FADER_TRAVEL_PX).clamp(0.0, 1.0)
}

/// 推子**位置 → dB**（`[`FADER_MIN_DB`, `FADER_MAX_DB`]` 上线性）。
///
/// 它是 [`crate::bridge::volume_fraction`] 的反解：两处必须同时改（判据
/// `dragged_fader_db_maps_pixels_to_db_and_clamps` 钉住端到端一致）。
#[must_use]
pub fn fader_db_from_fraction(fraction: f32) -> f32 {
    crate::bridge::FADER_MIN_DB
        + fraction.clamp(0.0, 1.0) * (crate::bridge::FADER_MAX_DB - crate::bridge::FADER_MIN_DB)
}

/// 推子：从起点 dB 与**纵向位移**算新的 dB（[`dragged_fader_fraction`] 的 dB 形态）。
///
/// - 口径：`delta_px = start_y − current_y`（Slint 的 y 轴向下）⇒ 向上拖 = 变大；
/// - 位置**一位没变**时逐位返回起点（见 [`dragged_fader_fraction`]）；
/// - 非有限位移（`NaN` / `±∞`）⇒ 原地返回起点（不让 `NaN` 进工程）。
#[must_use]
pub fn dragged_fader_db(start_db: f32, delta_px: f32) -> f32 {
    let start_fraction = crate::bridge::volume_fraction(start_db);
    let fraction = dragged_fader_fraction(start_fraction, delta_px);
    if fraction.to_bits() == start_fraction.to_bits() {
        return start_db.clamp(crate::bridge::FADER_MIN_DB, crate::bridge::FADER_MAX_DB);
    }
    fader_db_from_fraction(fraction)
}

/// 声相：从起点 `pan` 与**横向位移**算新的 `pan`（纯函数，可单独判据）。
///
/// - 口径：`delta_px = current_x − start_x`（向右拖 = 偏右）；
/// - 跨度 2.0 铺满 [`MIXER_PAN_TRAVEL_PX`]，越界夹到 `-1.0..=1.0`
///   （与 `ModelError::PanOutOfRange` 的边界同一处值域，见 `ops.rs` 的 `validate_param_value`）；
/// - 起点非有限 ⇒ 当居中（`0.0`）处理；位移非有限或为 `0.0` ⇒ 原地返回起点
///   （`0.0` 那一路是逐位的，理由同 [`dragged_fader_fraction`]）。
#[must_use]
pub fn dragged_pan(start_pan: f32, delta_px: f32) -> f32 {
    let start = if start_pan.is_finite() {
        start_pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    if delta_px == 0.0 || !delta_px.is_finite() {
        return start;
    }
    (start + delta_px / MIXER_PAN_TRAVEL_PX * 2.0).clamp(-1.0, 1.0)
}

/// 声相 `f32` → 千分之一整数（**唯一**口径，与投影同源：`bridge.rs:2093`）。
fn pan_millis_of(pan: f32) -> i32 {
    (f64::from(pan) * 1000.0).round() as i32
}

/// 音量 dB → 界面文本（**唯一**口径，与投影同源：`bridge.rs:2114` 的 `{:.1}`）。
fn volume_display_of(volume_db: f32) -> String {
    format!("{volume_db:.1}")
}

/// 把某一轨的**视图态预览**写进界面（拖动期唯一的写入面）。
///
/// 只写两个数组、只改一个下标：不重投影（重投影会重建 repeater 条目、把正在拖拽的
/// `TouchArea` 连同指针抓取一起丢掉 —— 实测记录见 [`apply_row_geometry`] 的文档）。
/// `index` 越界时**什么都不做**（不 panic、也不猜一条轨道）。
fn preview_track_volume(ui: &MainWindow, index: i32, volume_db: f32) {
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    let mut volumes = read_strings(&ui.get_track_volumes());
    let mut fractions = read_lengths(&ui.get_track_volume_fractions());
    if index >= volumes.len() || index >= fractions.len() {
        return;
    }
    volumes[index] = volume_display_of(volume_db);
    fractions[index] = crate::bridge::volume_fraction(volume_db);
    ui.set_track_volumes(strings(&volumes));
    ui.set_track_volume_fractions(lengths(&fractions));
}

/// 同 [`preview_track_volume`]，写声相数组（文本与千分位整数同源）。
fn preview_track_pan(ui: &MainWindow, index: i32, pan: f32) {
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    let mut pans = read_strings(&ui.get_track_pans());
    if index >= pans.len() {
        return;
    }
    pans[index] = crate::bridge::pan_display(pan_millis_of(pan));
    ui.set_track_pans(strings(&pans));
}

/// **主控通道条**的视图态预览（音量）：只写那两个**标量**属性。
///
/// 与 [`preview_track_volume`] 是同一个写入面的两种形态（数组一格 vs 标量），
/// 写出的文本 / 位置与投影层**同源**：`{:.1}`（[`volume_display_of`]）与
/// [`crate::bridge::volume_fraction`]，而 `apply_master` 写的正是同一对函数的结果
/// ⇒ "取消时写回起点"写出的字节与一次完整重投影逐位相同。
fn preview_master_volume(ui: &MainWindow, volume_db: f32) {
    ui.set_master_volume_display(volume_display_of(volume_db).into());
    ui.set_master_volume_fraction(crate::bridge::volume_fraction(volume_db));
}

/// **主控通道条**的视图态预览（声相）：写 `master-pan`（口径同 [`preview_track_pan`]）。
fn preview_master_pan(ui: &MainWindow, pan: f32) {
    ui.set_master_pan(crate::bridge::pan_display(pan_millis_of(pan)).into());
}

/// 按**目标**分派音量预览（[`cancel_mixer_gesture`] 与收尾路径的唯一入口）。
fn preview_target_volume(ui: &MainWindow, target: &MixerTarget, volume_db: f32) {
    match target {
        MixerTarget::Track { index, .. } => preview_track_volume(ui, *index, volume_db),
        MixerTarget::Master { .. } => preview_master_volume(ui, volume_db),
    }
}

/// 按**目标**分派声相预览（理由同 [`preview_target_volume`]）。
fn preview_target_pan(ui: &MainWindow, target: &MixerTarget, pan: f32) {
    match target {
        MixerTarget::Track { index, .. } => preview_track_pan(ui, *index, pan),
        MixerTarget::Master { .. } => preview_master_pan(ui, pan),
    }
}

/// 权威工程里**主总线**的身份文本（26 字符 `EntityId`）。
///
/// 主总线**不在**注入给界面的 `track-ids` 里（投影刻意排除它），因此这里按**身份**从
/// [`UndoPort::try_project`] 的权威工程取：`master_bus_track_id` 非 nil **且**真的在
/// `tracks` 里 ⇒ `Some(身份文本)`；否则 `None`（调用方**不留手势**，不猜）。
///
/// 口径与模型校验同源：`YebanProjectV1::validate`（`project.rs:1646-1665`）要求
/// 非空工程里 `master_bus_track_id` 必须指向一条 `TrackKind::Master`。
fn master_track_id(port: &UndoPort) -> Option<String> {
    let project = port.try_project()?;
    let id = project.master_bus_track_id;
    if id.is_nil() {
        return None;
    }
    project.tracks.get(&id)?;
    Some(id.to_canonical_string())
}

/// 权威工程里按**身份文本**取音轨的音量 / 声相 / 静音 / 独奏读数。
///
/// 身份来自界面注入的 `track-ids`（`host::track_id_at`），因此这里按同一个口径回查模型：
/// 找不到就是 `None`（工程被换掉 / 控制面关掉了工程），调用方**什么都不做**，不猜。
fn mixer_track_state(port: &UndoPort, track_id: &str) -> Option<MixerTrackState> {
    let project = port.try_project()?;
    project
        .tracks
        .values()
        .find(|track| track.id.to_canonical_string() == track_id)
        .map(|track| MixerTrackState {
            volume_db: track.volume_db,
            pan: track.pan,
            mute: track.mute,
            solo: track.solo,
        })
}

/// [`mixer_track_state`] 的读数（四格，全是模型字段的原文）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct MixerTrackState {
    /// `TrackV3::volume_db`。
    volume_db: f32,
    /// `TrackV3::pan`。
    pan: f32,
    /// `TrackV3::mute`。
    mute: bool,
    /// `TrackV3::solo`。
    solo: bool,
}

/// **开始**一次混音手势：把记录写进宿主，最后写"进行中"标志。
///
/// `active` 最后写（与 [`begin_track_height_drag`] 同款）：读者要么看到"还没开始"，
/// 要么看到一份**完整**的起点记录。
fn begin_mixer_gesture(ui: &MainWindow, gesture: &Rc<RefCell<MixerGesture>>, next: MixerGesture) {
    *gesture.borrow_mut() = next;
    ui.set_mixer_drag_active(true);
}

/// **一次混音手势的收尾**（松开 / 指针离开窗口 / `Escape` 三条路径的唯一落点）。
///
/// 返回被取走的记录；[`MixerGesture::Idle`] 表示"此刻没有手势在手"。
///
/// 无论哪条路径，`mixer-drag-active` 都在这里归位 —— 它是"混音手势是否还没结束"的
/// 唯一窗口读数（与 [`end_track_height_drag`] 的形状逐条相同：先读标志、再清零、
/// 返回"是否确实有手势在手"）。标志已经归位时顺手把记录也归零，因此不可能留下
/// "标志 `false` 而记录还在"的不一致状态。
fn end_mixer_gesture(ui: &MainWindow, gesture: &Rc<RefCell<MixerGesture>>) -> MixerGesture {
    if !ui.get_mixer_drag_active() {
        *gesture.borrow_mut() = MixerGesture::Idle;
        return MixerGesture::Idle;
    }
    let taken = std::mem::replace(&mut *gesture.borrow_mut(), MixerGesture::Idle);
    ui.set_mixer_drag_active(false);
    taken
}

/// `Escape`（`Action::Cancel`）在**有混音手势在手**时的收尾：取消这一拖并写回起点值。
///
/// 语义与 [`cancel_track_height_drag`] 同口径：**写回起点**、**不提交**。推子 / 声相是
/// 工程内容（`ADR-0005`），因此"不提交"意味着 `UndoPort` 一次都不碰 —— 工程与撤销栈
/// 与拖动之前逐位相同。
///
/// 写回走的是 [`preview_target_volume`] / [`preview_target_pan`]（拖动期那**同一个**视图态
/// 写入面），而**不是**一次完整重投影：那几个函数与投影层同源（`bridge.rs` 的
/// `format!("{:.1}")` / `volume_fraction` / `pan_display(pan_millis)`），写出的字节与
/// 重投影逐位相同；反过来，完整 `apply_view` 会重建 repeater 条目、把**此刻仍被按住**的
/// 那个 `TouchArea` 连同指针抓取一起丢掉（实测记录见 [`reproject_track_heights`]）。
///
/// 通道条与主控走**同一条**取消路径（同一个 `mixer-cancel-gesture` 回调）：两类目标只在
/// 写回的那一个函数上分派，因此不存在"只有轨道能 `Escape`、主控不能"这种半接线状态。
///
/// 返回 `false` = 此刻没有混音手势 ⇒ 这一键与混音台无关，照旧放行。
fn cancel_mixer_gesture(ui: &MainWindow, gesture: &Rc<RefCell<MixerGesture>>) -> bool {
    match end_mixer_gesture(ui, gesture) {
        MixerGesture::Idle => false,
        MixerGesture::Fader {
            target, start_db, ..
        } => {
            preview_target_volume(ui, &target, start_db);
            true
        }
        MixerGesture::Pan {
            target, start_pan, ..
        } => {
            preview_target_pan(ui, &target, start_pan);
            true
        }
    }
}

/// **混音台写入面的接线**（唯一实现；`main.rs` 与 `live_surface` 调的是同一个函数）。
///
/// 十四个入口分三类：
/// - **通道条手势**（推子竖直 / 声相水平）：按下记起点 → 移动只写视图态 → 松手提交**一次**；
/// - **主控手势**（`mixer-master-fader-*` / `mixer-master-pan-*`）：与通道条逐条同形，
///   区别只有寻址方式（身份，见 [`MixerTarget`]）与视图态的落点（标量属性）；
/// - **开关**（静音 / 独奏）：点击即提交**一次**（没有中间态）。
///
/// 提交失败（控制面拒绝、工程被关掉）⇒ **什么都不回写**并打一行 stderr：
/// 不假装成功，也不 panic（与 `wire_roll_edit` 同款）。
///
/// ## 两类手势**互不串扰**（这是一条机械性质，不是约定）
///
/// 通道条回调只驱动 [`MixerTarget::Track`]、主控回调只驱动 [`MixerTarget::Master`]：
/// 每一条 `drag` 都**先核对目标**再动记录（`moved` / `last_*`），目标不符就原地返回。
/// 因此"主控拖动期间来一次轨道 `move`"（或反过来）既不写视图态、也不污染收尾要提交的值。
pub fn wire_mixer_edit(ui: &MainWindow, port: &Rc<UndoPort>) {
    let weak = ui.as_weak();
    let gesture = Rc::new(RefCell::new(MixerGesture::Idle));

    // ---------------------------------------------------------------- 推子
    ui.on_mixer_fader_grab({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move |index, pointer_y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Some(track_id) = track_id_at(&ui, index) else {
                // 按下落在没有身份的格子上 ⇒ **不留手势**：标志与记录一起归零。
                end_mixer_gesture(&ui, &gesture);
                return;
            };
            match mixer_track_state(&port, &track_id) {
                Some(state) => begin_mixer_gesture(
                    &ui,
                    &gesture,
                    MixerGesture::Fader {
                        target: MixerTarget::Track { index, track_id },
                        start_db: state.volume_db,
                        start_y: pointer_y,
                        last_db: state.volume_db,
                        moved: false,
                    },
                ),
                None => {
                    end_mixer_gesture(&ui, &gesture);
                }
            }
        }
    });
    ui.on_mixer_fader_drag({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        move |index, pointer_y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // 没有进行中的手势 / 下标不是按下那一轨 ⇒ **什么都不做**（黏住的拖拽态第一道防线）。
            // 第一道防线就是下面这一行标志：`Escape` 已经取消过这一拖之后，迟到的 `move`
            // 在这里就被挡下（与 `track_height_drag_target_px` 的开头同款）。
            // 每一次 `drag` 都从**起点**重算（不从上一个结果累加）：累加会让 f32 的
            // 舍入误差随移动次数漂移，同一个指针位置在"直着拖过去"与"绕一圈拖过去"
            // 两种走法下会给出不同的数。
            if !ui.get_mixer_drag_active() {
                return;
            }
            // **先核对目标、再动记录**：被拒的 `move`（错下标 / 主控手势）不许改
            // `moved` / `last_db`，否则收尾会提交一个没人拖出来的值。
            let (gesture_index, track_id, start_db, start_y) = match &*gesture.borrow() {
                MixerGesture::Fader {
                    target:
                        MixerTarget::Track {
                            index: gesture_index,
                            track_id,
                        },
                    start_db,
                    start_y,
                    ..
                } => (*gesture_index, track_id.clone(), *start_db, *start_y),
                // 主控手势（或空手势）：轨道下标寻址的 `move` 与它无关。
                _ => return,
            };
            if gesture_index != index {
                return;
            }
            if track_id_at(&ui, index).as_deref() != Some(track_id.as_str()) {
                return;
            }
            let next_db = dragged_fader_db(start_db, start_y - pointer_y);
            if let MixerGesture::Fader { last_db, moved, .. } = &mut *gesture.borrow_mut() {
                *moved = true;
                *last_db = next_db;
            }
            preview_track_volume(&ui, index, next_db);
        }
    });
    ui.on_mixer_fader_release({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move |index| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // 收尾走**唯一清零点**（与 `Escape` 的取消路径共用）：标志与记录一起归零，
            // 因此 `Escape` 已经取消过之后，这一松手读到的是 `Idle` ⇒ 什么都不提交。
            let taken = end_mixer_gesture(&ui, &gesture);
            let MixerGesture::Fader {
                target:
                    MixerTarget::Track {
                        index: gesture_index,
                        track_id,
                    },
                start_db,
                last_db,
                moved,
                ..
            } = taken
            else {
                // 主控手势 / 空手势：轨道下标寻址的松手与它无关（**不许**顺手提交）。
                return;
            };
            if !moved {
                // 按下就松手（一个 `drag` 都没收到）⇒ **不提交**：不留一步什么都没改的撤销。
                return;
            }
            if gesture_index != index {
                return;
            }
            if track_id_at(&ui, index).as_deref() != Some(track_id.as_str()) {
                return;
            }
            // 动过但算回了同一个数（拖出去又拖回来）⇒ 同样不提交。
            // 比较的是 `f32` 的**位**：`f32: PartialEq` 会把 `-0.0 == 0.0` 判真，
            // 而这里要的是"一个字节都没变"。
            if last_db.to_bits() == start_db.to_bits() {
                return;
            }
            let Ok(track_id) = track_id.parse::<yeban_model::EntityId>() else {
                return;
            };
            let op = yeban_model::Op::SetParam {
                target: yeban_model::AutomationTarget::TrackVolume { track_id },
                old_val: start_db,
                new_val: last_db,
            };
            if port
                .commit_ops(now_ms(), "mixer: set track volume", vec![op])
                .is_err()
            {
                eprintln!("[yeban-app] 推子提交被拒绝（工程未改）");
                return;
            }
            refresh_undo_window(&ui, &port, true);
        }
    });

    // ---------------------------------------------------------------- 声相
    ui.on_mixer_pan_grab({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move |index, pointer_x| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Some(track_id) = track_id_at(&ui, index) else {
                // 按下落在没有身份的格子上 ⇒ **不留手势**（同推子）。
                end_mixer_gesture(&ui, &gesture);
                return;
            };
            match mixer_track_state(&port, &track_id) {
                Some(state) => begin_mixer_gesture(
                    &ui,
                    &gesture,
                    MixerGesture::Pan {
                        target: MixerTarget::Track { index, track_id },
                        start_pan: state.pan,
                        start_x: pointer_x,
                        last_pan: state.pan,
                        moved: false,
                    },
                ),
                None => {
                    end_mixer_gesture(&ui, &gesture);
                }
            }
        }
    });
    ui.on_mixer_pan_drag({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        move |index, pointer_x| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // 标志是第一道防线（理由同推子的 `drag`）。
            if !ui.get_mixer_drag_active() {
                return;
            }
            // 先核对目标、再动记录（理由同推子的 `drag`）。
            let (gesture_index, track_id, start_pan, start_x) = match &*gesture.borrow() {
                MixerGesture::Pan {
                    target:
                        MixerTarget::Track {
                            index: gesture_index,
                            track_id,
                        },
                    start_pan,
                    start_x,
                    ..
                } => (*gesture_index, track_id.clone(), *start_pan, *start_x),
                _ => return,
            };
            if gesture_index != index {
                return;
            }
            if track_id_at(&ui, index).as_deref() != Some(track_id.as_str()) {
                return;
            }
            let next_pan = dragged_pan(start_pan, pointer_x - start_x);
            if let MixerGesture::Pan {
                last_pan, moved, ..
            } = &mut *gesture.borrow_mut()
            {
                *moved = true;
                *last_pan = next_pan;
            }
            preview_track_pan(&ui, index, next_pan);
        }
    });
    ui.on_mixer_pan_release({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move |index| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // 收尾走唯一清零点（理由同推子）：`Escape` 取消过之后这一松手读到 `Idle`。
            let taken = end_mixer_gesture(&ui, &gesture);
            let MixerGesture::Pan {
                target:
                    MixerTarget::Track {
                        index: gesture_index,
                        track_id,
                    },
                start_pan,
                last_pan,
                moved,
                ..
            } = taken
            else {
                return;
            };
            if !moved {
                // 按下就松手 ⇒ 不提交（同推子）。
                return;
            }
            if gesture_index != index {
                return;
            }
            if track_id_at(&ui, index).as_deref() != Some(track_id.as_str()) {
                return;
            }
            // 提交的是 `drag` 算出来的那个 `f32`（位比较，理由同推子）。
            // 界面上那一格文本是它的函数（`pan_display(pan_millis_of(..))`），
            // 因此"用户看到的数"与"提交的数"不可能两说。
            if last_pan.to_bits() == start_pan.to_bits() {
                return;
            }
            let Ok(track_id) = track_id.parse::<yeban_model::EntityId>() else {
                return;
            };
            let op = yeban_model::Op::SetParam {
                target: yeban_model::AutomationTarget::TrackPan { track_id },
                old_val: start_pan,
                new_val: last_pan,
            };
            if port
                .commit_ops(now_ms(), "mixer: set track pan", vec![op])
                .is_err()
            {
                eprintln!("[yeban-app] 声相提交被拒绝（工程未改）");
                return;
            }
            refresh_undo_window(&ui, &port, true);
        }
    });

    // ---------------------------------------------------------------- 主控推子
    //
    // 形状与上面的通道条推子**逐条相同**，只有两处不同（都是 [`MixerTarget`] 带来的）：
    // ① 寻址：`.slint` 不送下标（主总线不在 `track-names` 里），身份在按下那一刻从权威
    //    工程取（[`master_track_id`]）；② 视图态：写 `master-volume-display` /
    //    `master-volume-fraction` 两个标量，而不是数组的一格。
    ui.on_mixer_master_fader_grab({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move |pointer_y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Some(track_id) = master_track_id(&port) else {
                // 工程没有主总线（`master_bus_track_id` 是 nil 或不在 `tracks` 里）
                // ⇒ **不留手势**：标志与记录一起归零。
                end_mixer_gesture(&ui, &gesture);
                return;
            };
            match mixer_track_state(&port, &track_id) {
                Some(state) => begin_mixer_gesture(
                    &ui,
                    &gesture,
                    MixerGesture::Fader {
                        target: MixerTarget::Master { track_id },
                        start_db: state.volume_db,
                        start_y: pointer_y,
                        last_db: state.volume_db,
                        moved: false,
                    },
                ),
                None => {
                    end_mixer_gesture(&ui, &gesture);
                }
            }
        }
    });
    ui.on_mixer_master_fader_drag({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        move |pointer_y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            if !ui.get_mixer_drag_active() {
                return;
            }
            // 先核对目标、再动记录（理由同推子的 `drag`）：只有**主控**手势能走这一格。
            let (start_db, start_y) = match &*gesture.borrow() {
                MixerGesture::Fader {
                    target: MixerTarget::Master { .. },
                    start_db,
                    start_y,
                    ..
                } => (*start_db, *start_y),
                _ => return,
            };
            let next_db = dragged_fader_db(start_db, start_y - pointer_y);
            if let MixerGesture::Fader { last_db, moved, .. } = &mut *gesture.borrow_mut() {
                *moved = true;
                *last_db = next_db;
            }
            preview_master_volume(&ui, next_db);
        }
    });
    ui.on_mixer_master_fader_release({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // 收尾走唯一清零点（理由同推子）。
            let taken = end_mixer_gesture(&ui, &gesture);
            let MixerGesture::Fader {
                target: MixerTarget::Master { track_id },
                start_db,
                last_db,
                moved,
                ..
            } = taken
            else {
                // 通道条手势 / 空手势：主控松手与它无关。
                return;
            };
            if !moved {
                return;
            }
            if last_db.to_bits() == start_db.to_bits() {
                return;
            }
            let Ok(track_id) = track_id.parse::<yeban_model::EntityId>() else {
                return;
            };
            let op = yeban_model::Op::SetParam {
                target: yeban_model::AutomationTarget::TrackVolume { track_id },
                old_val: start_db,
                new_val: last_db,
            };
            if port
                .commit_ops(now_ms(), "mixer: set master volume", vec![op])
                .is_err()
            {
                eprintln!("[yeban-app] 主控推子提交被拒绝（工程未改）");
                return;
            }
            refresh_undo_window(&ui, &port, true);
        }
    });

    // ---------------------------------------------------------------- 主控声相
    ui.on_mixer_master_pan_grab({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move |pointer_x| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Some(track_id) = master_track_id(&port) else {
                end_mixer_gesture(&ui, &gesture);
                return;
            };
            match mixer_track_state(&port, &track_id) {
                Some(state) => begin_mixer_gesture(
                    &ui,
                    &gesture,
                    MixerGesture::Pan {
                        target: MixerTarget::Master { track_id },
                        start_pan: state.pan,
                        start_x: pointer_x,
                        last_pan: state.pan,
                        moved: false,
                    },
                ),
                None => {
                    end_mixer_gesture(&ui, &gesture);
                }
            }
        }
    });
    ui.on_mixer_master_pan_drag({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        move |pointer_x| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            if !ui.get_mixer_drag_active() {
                return;
            }
            // 先核对目标、再动记录（理由同推子的 `drag`）。
            let (start_pan, start_x) = match &*gesture.borrow() {
                MixerGesture::Pan {
                    target: MixerTarget::Master { .. },
                    start_pan,
                    start_x,
                    ..
                } => (*start_pan, *start_x),
                _ => return,
            };
            let next_pan = dragged_pan(start_pan, pointer_x - start_x);
            if let MixerGesture::Pan {
                last_pan, moved, ..
            } = &mut *gesture.borrow_mut()
            {
                *moved = true;
                *last_pan = next_pan;
            }
            preview_master_pan(&ui, next_pan);
        }
    });
    ui.on_mixer_master_pan_release({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let taken = end_mixer_gesture(&ui, &gesture);
            let MixerGesture::Pan {
                target: MixerTarget::Master { track_id },
                start_pan,
                last_pan,
                moved,
                ..
            } = taken
            else {
                return;
            };
            if !moved {
                return;
            }
            if last_pan.to_bits() == start_pan.to_bits() {
                return;
            }
            let Ok(track_id) = track_id.parse::<yeban_model::EntityId>() else {
                return;
            };
            let op = yeban_model::Op::SetParam {
                target: yeban_model::AutomationTarget::TrackPan { track_id },
                old_val: start_pan,
                new_val: last_pan,
            };
            if port
                .commit_ops(now_ms(), "mixer: set master pan", vec![op])
                .is_err()
            {
                eprintln!("[yeban-app] 主控声相提交被拒绝（工程未改）");
                return;
            }
            refresh_undo_window(&ui, &port, true);
        }
    });

    // ---------------------------------------------------------------- 取消（`Escape`）
    //
    // 这是 `Escape`（`Action::Cancel`）在混音台上的落点：`apply_action` 调
    // `ui.invoke_mixer_cancel_gesture()`，落到这里持有手势记录的**同一个闭包**
    // ⇒ 取消与记录只有一份状态（与 `cancel_track_height_drag` 同一个角色）。
    // 没人接线时（装配没有 `UndoPort` ⇒ 没有写入面）Slint 返回 `bool` 的默认值
    // `false`，因此"没有混音手势"这一读数在两种装配下都成立。
    ui.on_mixer_cancel_gesture({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        move || -> bool {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return false;
            };
            cancel_mixer_gesture(&ui, &gesture)
        }
    });

    // ---------------------------------------------------------------- 两个开关
    wire_mixer_switch(ui, port, &weak, MixerSwitch::Mute);
    wire_mixer_switch(ui, port, &weak, MixerSwitch::Solo);

    // ---------------------------------------------------------------- SFZ 采样器绑定
    // ⭐ 任务单 `#82`：模型 `TrackV3::sampler`（`#76`）第一次有**用户入口**。
    wire_mixer_sampler(ui, port);
}

/// ⭐ **SFZ 采样器绑定**的接线（任务单 `#82`）—— 两条回调的**唯一**实现。
///
/// 形状照 [`wire_mixer_switch`]：读当前值 → 构造 `Op` → `commit_ops` **一次** → 重投影。
/// 差别只有一处：新值来自 `.slint` 传进来的**文本**（本仓没有文件对话框 ⇒ 文本输入，
/// 见交付报告的"删减了什么"），而 `Clear` 的新值是 `None`（**解绑**，一等状态）。
///
/// ## 空文本为什么是"**不提交**"，而不是"解绑"
///
/// `""` 不是 `None`：`SamplerBinding { path: "" }` 会让发布点把"读不到的文件"如实报错
/// （`EngineHostError::Sfz`）。而用户清空输入框**往往只是想重打一遍** ⇒ 这里什么都不做
/// 并出声。解绑只有一个入口：`Clear`（它写的就是 `None`）。
///
/// ## 值没变时不提交
///
/// 提交一条 `old == new` 的 `Op` 会在撤销树上留下**一步什么也没改**的历史
/// （`Op::SetTrackSampler` 的陈旧载荷比对只拦"读到的基线不对"，不拦 `old == new`）
/// ⇒ 这里先比一次再提交。
///
/// ## 这条消息不带来源视图前缀（台账 R17 的同一条纪律）
///
/// 它是撤销树的节点标签。今天只有一个界面入口（调音台通道条），但"不带来源"的措辞
/// 在任何入口下都成立 —— 与静音 / 独奏的 `"toggle track mute"` 同款。
fn wire_mixer_sampler(ui: &MainWindow, port: &Rc<UndoPort>) {
    let weak = ui.as_weak();
    ui.on_mixer_sampler_set({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move |index, path| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // ⛔ 空文本不落到 `Some("")`（那会被引擎当成一条读不到的文件路径）。
            // 只报"这一下没有提交"，不假装做过什么。
            //
            // ⚠ 界面侧对路径**只做一处**规范化：去掉首尾空白（粘贴常常带一个换行）。
            // 除此之外**原样**交给模型（⛔ 不规范化、⛔ 不查存在性、⛔ 不判扩展名 ——
            // 与 `TrackV3::sampler` 的字段口径逐字相同）。
            let trimmed = path.trim();
            if trimmed.is_empty() {
                eprintln!("[yeban-app] SFZ 路径是空的 ⇒ 没有提交（解绑请用 Clear）");
                return;
            }
            commit_sampler_binding(&ui, &port, index, Some(trimmed), "set track sampler");
        }
    });
    ui.on_mixer_sampler_clear({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move |index| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            commit_sampler_binding(&ui, &port, index, None, "clear track sampler");
        }
    });
}

/// ⭐ **③ 选 SFZ 路径 —— 原生对话框那一条入口**（任务单 `#86` 乙）。
///
/// ## 它做什么（只有一件事）
///
/// `.slint` 的 `mixer-sampler-browse(index)` ⇒ 问一次 [`crate::file_picker::FilePicker`]
/// ⇒ 把选中的路径**写进** `track-sampler-paths[index]`（= `track-{i}-sampler-path`
/// 那个 `TextInput` 的 `text`）。
///
/// ## ⛔ 它**不**提交任何工程内容（这是刻意的，不是没做完）
///
/// 本回调**不**碰 `UndoPort`、**不**构造 `Op::SetTrackSampler`、**不**改 `TrackV3::sampler`。
/// 提交仍然是**已有的**那一条：用户按 SET（或输入框里回车）⇒
/// [`wire_mixer_sampler`] ⇒ [`commit_sampler_binding`]。
///
/// 为什么不做成"选完直接提交"：那会让"浏览"变成一个**隐式提交** —— 用户只是想看看
/// 文件在哪，工程却已经被改了。而且本单的硬要求是"**文本框与 CLI 两条路仍然可用**"，
/// 保持"浏览只填框、SET 才提交"这条**既有**语义正是那句话的机械形态。
///
/// ## 边界（如实登记）
///
/// * 取消（`None`）⇒ **一位不动**：不改模型、不改输入框、⛔ 不出声、⛔ 不报错。
/// * `index` 越界（投影换了、界面还拿着旧下标）⇒ **一位不动**（⛔ 不 panic、⛔ 不猜一条轨道）。
/// * 路径**原样**填进输入框（⛔ 不去空白、⛔ 不加后缀、⛔ 不查存在性）—— 与
///   [`wire_mixer_sampler`] 对"用户手敲的文本"的口径**逐字相同**："界面侧只做一处
///   规范化"这条纪律因此没有被第二次破坏。
pub fn wire_mixer_sampler_browse(ui: &MainWindow, picker: Rc<dyn crate::file_picker::FilePicker>) {
    let weak = ui.as_weak();
    ui.on_mixer_sampler_browse(move |index| {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        let request =
            crate::file_picker::PickerRequest::open("选择 SFZ 音色文件").with_extensions(["sfz"]);
        // ⚠ 取消 ⇒ 一位不动（⛔ 不把"我不想"写成一条错误）。
        let Some(path) = picker.choose_open(&request) else {
            return;
        };
        let text = SharedString::from(path.to_string_lossy().as_ref());
        let mut paths: Vec<SharedString> = {
            use slint::Model as _;
            ui.get_track_sampler_paths().iter().collect()
        };
        // ⚠ 越界 ⇒ 一位不动（⛔ 不 panic、⛔ 不 append 到别的轨道上）。
        let Some(slot) = usize::try_from(index).ok().and_then(|i| paths.get_mut(i)) else {
            return;
        };
        *slot = text;
        ui.set_track_sampler_paths(ModelRc::new(VecModel::from(paths)));
    });
}

/// `Set` / `Clear` 的**唯一**提交实现（两条回调共用，因此不可能有一份漂移的 `Op` 构造）。
///
/// `new_path`：`Some(text)` = 绑定那条路径；`None` = **解绑**。
fn commit_sampler_binding(
    ui: &MainWindow,
    port: &UndoPort,
    index: i32,
    new_path: Option<&str>,
    message: &str,
) {
    let Some(track_id) = track_id_at(ui, index) else {
        return;
    };
    let Ok(id) = track_id.parse::<yeban_model::EntityId>() else {
        return;
    };
    let Some(old_sampler) = authoritative_sampler(port, &track_id) else {
        return;
    };
    let new_sampler = new_path.map(|path| yeban_model::SamplerBinding {
        path: path.to_owned(),
    });
    // 值没变 ⇒ 不制造"什么也没改"的一步撤销历史（见本函数调用者的文档）。
    if old_sampler == new_sampler {
        return;
    }
    let op = yeban_model::Op::SetTrackSampler {
        track_id: id,
        old_sampler,
        new_sampler,
    };
    if port.commit_ops(now_ms(), message, vec![op]).is_err() {
        eprintln!("[yeban-app] SFZ 绑定提交被拒绝（工程未改）");
        return;
    }
    refresh_undo_window(ui, port, true);
}

/// 权威工程里某轨的 `TrackV3::sampler` **原值**。
///
/// 返回 `None` = "问不出来"（端口此刻没有活跃工程 / 工程里没有这条轨）——
/// 与 [`Some(None)`]（"这条轨不用采样器"）是**两件不同的事**，因此这里用双层 `Option`
/// 而不是把两者压成一个 `None`（压掉之后，"没有活跃工程"会被当成"解绑"而静默提交）。
fn authoritative_sampler(
    port: &UndoPort,
    track_id: &str,
) -> Option<Option<yeban_model::SamplerBinding>> {
    let project = port.try_project()?;
    project
        .tracks
        .values()
        .find(|track| track.id.to_canonical_string() == track_id)
        .map(|track| track.sampler.clone())
}

/// 装乐器被拒绝时**唯一**的 stderr 写法（⛔ 不散在回调里）。
///
/// 为什么单独成函数：`tests/mixer_toggle_labels.rs` 用"整行就是一个字面量"的形状扫
/// `fn wire_mixer_switch` 到 `fn read_strings` 之间的源码。那个区间覆盖了本文件一大段
/// 与混音开关无关的实现 —— 一个被 rustfmt 换到行首的 `eprintln!` 格式串会被它误数成
/// 第三条提交消息。本函数住在扫描区间**之外**（`wire_mixer_switch` 之前），调用点因此
/// 没有行首字面量。⚠ 这是对既有判据锚点宽度的**规避**，不是把判据改松。
fn warn_install_rejected(reason: &str, path: &str) {
    eprintln!("{INSTALL_REJECTED_STDERR_PREFIX}{reason}（{path}）");
}

/// 混音台上的两个**布尔开关**（静音 / 独奏）—— 除了目标字段不同，接线逐字相同。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MixerSwitch {
    /// `TrackV3::mute`。
    Mute,
    /// `TrackV3::solo`。
    Solo,
}

/// 一个开关的**唯一**接线实现。
///
/// 点击 = 读现值 → 取反 → `commit_ops` **一次** → 重投影。没有中间态，
/// 因此不存在"拖动期视图态"这一档（与推子 / 声相的形状差异是有意的）。
///
/// ## 为什么提交消息**不带** `mixer:` 前缀（台账 R17）
///
/// 这条消息就是撤销树的**节点标签**（`UndoPort::commit_ops` → `Commit::message`）。
/// 而本函数有**两个**界面入口：调音台的开关（`console/mixer_console.slint`）与**编曲视图**
/// 轨道头上的开关（`ui/workspace/arrangement_view.slint` 的 `track-mute-toggle` /
/// `track-solo-toggle` 经 `app.slint` 转到同一个 `mixer-mute-toggle`）。
/// 原先写死的 `"mixer: …"` 于是会在编曲视图里说一句**假话** —— 用户在编曲视图点了一下，
/// 撤销树的节点却声称那是"混音台"的动作。改中性文本（去掉来源前缀）后，这条标签在两个
/// 入口下都成立；带来源的措辞要等宿主能**分别**知道是哪个入口点的（那需要两条回调 + 两条
/// 标签，本票不做）。
fn wire_mixer_switch(
    ui: &MainWindow,
    port: &Rc<UndoPort>,
    weak: &slint::Weak<MainWindow>,
    which: MixerSwitch,
) {
    let weak = weak.clone();
    let port = Rc::clone(port);
    let handler = move |index: i32| {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        let Some(track_id) = track_id_at(&ui, index) else {
            return;
        };
        let Some(state) = mixer_track_state(&port, &track_id) else {
            return;
        };
        let Ok(id) = track_id.parse::<yeban_model::EntityId>() else {
            return;
        };
        let (op, message) = match which {
            MixerSwitch::Mute => (
                yeban_model::Op::SetTrackMute {
                    track_id: id,
                    old_mute: state.mute,
                    new_mute: !state.mute,
                },
                "toggle track mute",
            ),
            MixerSwitch::Solo => (
                yeban_model::Op::SetTrackSolo {
                    track_id: id,
                    old_solo: state.solo,
                    new_solo: !state.solo,
                },
                "toggle track solo",
            ),
        };
        if port.commit_ops(now_ms(), message, vec![op]).is_err() {
            eprintln!("[yeban-app] 混音开关提交被拒绝（工程未改）");
            return;
        }
        refresh_undo_window(&ui, &port, true);
    };
    match which {
        MixerSwitch::Mute => ui.on_mixer_mute_toggle(handler),
        MixerSwitch::Solo => ui.on_mixer_solo_toggle(handler),
    }
}

/// ⭐ 任务单 `#196`：采样库下载的**跨线程槽**（工作线程只写这里，⛔ 不碰 Slint 对象）。
///
/// 与 `MidiInState` 同一条形状：`in_flight` 挡住"第二次点击开第二个下载"，
/// 结果留在槽里等 UI 线程排空（没有事件循环的平台由下一次点击 / 判据排空）。
#[derive(Clone, Default)]
struct SampleLibraryDownloadState {
    slot: std::sync::Arc<std::sync::Mutex<SampleLibraryDownloadSlot>>,
}

/// 下载槽的内容（`in_flight` 与"上一次的结果"互斥地活着）。
#[derive(Default)]
struct SampleLibraryDownloadSlot {
    in_flight: bool,
    outcome: Option<Result<crate::samples::DownloadOutcome, crate::samples::DownloadError>>,
}

impl SampleLibraryDownloadState {
    fn begin(&self) -> bool {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.in_flight {
            return false;
        }
        slot.in_flight = true;
        true
    }

    fn is_in_flight(&self) -> bool {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .in_flight
    }

    fn finish(
        &self,
        outcome: Result<crate::samples::DownloadOutcome, crate::samples::DownloadError>,
    ) {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slot.in_flight = false;
        slot.outcome = Some(outcome);
    }

    fn take(
        &self,
    ) -> Option<Result<crate::samples::DownloadOutcome, crate::samples::DownloadError>> {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .outcome
            .take()
    }
}

/// 把**已交回**的下载结果落到界面（唯一写 `sample-library-status` 的地方）。
///
/// 生产路径：工作线程完成 ⇒ `upgrade_in_event_loop` ⇒ 本函数；没有事件循环的环境由
/// 下一次点击 / 判据调本函数（自愈，⛔ 不把"结果没落界面"变成永久 pending）。
///
/// 成功（退出码 `0`）⇒ 重新投影：安装戳已经由 [`crate::samples::run_download`] 写好，
/// `installable_builtin_instruments` 的缓存键随之变化 ⇒ "装乐器"按钮**同进程**出现。
fn apply_pending_sample_library_download(
    ui: &MainWindow,
    state: &SampleLibraryDownloadState,
    port: &UndoPort,
) -> bool {
    let Some(outcome) = state.take() else {
        return false;
    };
    match outcome {
        Ok(outcome) => {
            ui.set_sample_library_status(outcome.summary().into());
            if outcome.ok() {
                refresh_undo_window(ui, port, true);
            }
        }
        Err(error) => {
            ui.set_sample_library_status(format!("采样库下载失败：{error}").into());
        }
    }
    true
}

/// 装乐器被拒绝时的 stderr 前缀（与既有口径逐字一致）。
///
/// 为什么抽成常量而不是把字面量留在 `eprintln!` 的参数行：`tests/mixer_toggle_labels.rs`
/// 用"整行就是一个字符串字面量"的形状数 `wire_mixer_switch` 到 `read_strings` 之间的
/// 提交标签。该区间**很大**，任何一行行首引号都会被它误数 ⇒ 本单把前缀提出来，
/// ⛔ 不去改那条既有判据（它的锚点问题另行登记，不在本单）。
const INSTALL_REJECTED_STDERR_PREFIX: &str = "[yeban-app] 装乐器被拒绝：";

/// ⭐ 任务单 `#211`：一次"装乐器"点击的**探针请求**（纯数据 ⇒ 可 `Send`）。
///
/// ⛔ 不带任何 Slint 对象、⛔ 不带 `Rc<UndoPort>`：跨线程只搬这三样。
/// `track_id` 在 **UI 线程**上按点击那一刻的"选中轨 or 兜底"解析（`#192` 的同一条口径），
/// 工作线程与排空都只认这个身份 —— 探测期间用户换选区也不会把乐器装到另一条轨。
#[derive(Clone, Debug)]
struct InstrumentProbeRequest {
    /// 点击那一刻解析出来的目标轨身份（ULID）。
    track_id: EntityId,
    /// 界面上的短名（只用于逐字回执）。
    name: String,
    /// 逐字进模型的 SFZ 路径（与 `library_instruments()` 同一条）。
    path: String,
}

/// ⭐ 任务单 `#211`：装乐器探针的**跨线程槽**（工作线程写结果，UI 线程读并提交）。
///
/// 与 [`SampleLibraryDownloadState`] / `MidiInState` 同一条形状：
/// * `in_flight` 挡住"第二次点击开第二个探针"（⛔ 不并发、⛔ 不排队）；
/// * 结果留在槽里等 UI 线程排空（没有事件循环的平台由下一次点击 / 判据排空）；
/// * 工作线程句柄留在槽里，判据用 [`wait_for_pending_instrument_probe`] **确定性** join。
///
/// ⚠ 槽里只有纯数据（[`InstrumentProbeRequest`] / [`InstrumentSampleProbe`]），
/// ⛔ 没有任何 Slint 对象 —— UI 写入仍然只发生在 UI 线程上。
#[derive(Clone, Default)]
pub struct InstrumentProbeState {
    slot: std::sync::Arc<std::sync::Mutex<InstrumentProbeSlot>>,
}

/// [`InstrumentProbeState`] 的内容（`in_flight` / 请求 / 结果 / 工作线程）。
#[derive(Default)]
struct InstrumentProbeSlot {
    /// 当前（或最近一次）探测的请求；`take` 时与结果**成对**取走。
    request: Option<InstrumentProbeRequest>,
    /// 是否有一条探测正在工作线程上跑。
    in_flight: bool,
    /// 工作线程交回的结果（`None` = 还没交回）。
    outcome: Option<crate::bridge::InstrumentSampleProbe>,
    /// 当前工作线程的句柄（判据用它做**确定性**等待）。
    worker: Option<std::thread::JoinHandle<()>>,
}

impl InstrumentProbeState {
    /// 开始一次探测（`false` = 已有一条在飞 ⇒ 调用方**逐字拒绝**）。
    fn begin(&self, request: InstrumentProbeRequest) -> bool {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.in_flight {
            return false;
        }
        slot.request = Some(request);
        slot.outcome = None;
        slot.worker = None;
        slot.in_flight = true;
        true
    }

    /// 是否有一条探测在飞（UI 线程只用于"先排空上一次"的自愈判断）。
    #[must_use]
    pub fn is_in_flight(&self) -> bool {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .in_flight
    }

    /// 工作线程交回结果（⛔ 不碰 UI）。
    fn finish(&self, outcome: crate::bridge::InstrumentSampleProbe) {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slot.in_flight = false;
        slot.outcome = Some(outcome);
    }

    /// 取走"请求 ＋ 结果"这一对（没有结果 / 没开始过 ⇒ `None`）。
    ///
    /// ⚠ 顺序承重：**先看有没有结果，再动请求**。第二次点击的"先排空上一次"自愈会在
    /// 探针还在飞（`outcome == None`）时调本函数 —— 若先 `take` 请求，就会把请求取走、
    /// 之后结果回来时再也凑不成一对（真实踩过：终态永远排不出来）。
    fn take(&self) -> Option<(InstrumentProbeRequest, crate::bridge::InstrumentSampleProbe)> {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slot.outcome.as_ref()?;
        let request = slot.request.take()?;
        let outcome = slot.outcome.take()?;
        Some((request, outcome))
    }

    /// 记下工作线程（UI 线程调；随后 [`wait_for_pending_instrument_probe`] 用它 join）。
    fn set_worker(&self, handle: std::thread::JoinHandle<()>) {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .worker = Some(handle);
    }

    /// 取走工作线程句柄。
    fn take_worker(&self) -> Option<std::thread::JoinHandle<()>> {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .worker
            .take()
    }
}

/// ⭐ 任务单 `#211`：**确定性**等待探针工作线程结束（判据用；生产路径不调）。
///
/// 返回 `false` = 没有工作线程可 join（没点过 / 已被 [`wait_for_pending_instrument_probe`]
/// 取走过）。与 [`wait_for_pending_midi_in`] 同一条纪律：join 之后结果**已经**在槽里，
/// 再在 UI 线程上调 [`apply_pending_instrument_probe`] 就得到终态。
pub fn wait_for_pending_instrument_probe(state: &InstrumentProbeState) -> bool {
    let Some(handle) = state.take_worker() else {
        return false;
    };
    handle.join().is_ok()
}

/// ⭐ 任务单 `#211`：把**已交回**的探针结果落到界面，并在成功时提交**恰好一次**
/// `Op::SetTrackSampler`（唯一提交点）。
///
/// 生产路径：工作线程完成 ⇒ `upgrade_in_event_loop` ⇒ `instrument-probe-drain-requested`
/// ⇒ 本函数；没有事件循环的环境由下一次点击 / 判据调本函数（自愈，⛔ 不把结果永远留在
/// pending）。
///
/// ## 提交时机（为什么撤销仍对）
///
/// 探测可能跑几分钟，期间工程会被别的手势 / MCP / 撤销改动 ⇒ 本函数**不**用点击那一刻
/// 的 `old_sampler` 快照，而是在**排空的这一帧**按 `request.track_id` 从权威工程**重新读**
/// 目标轨、用**当前** `old_sampler` 构造 `Op`（[`crate::bridge::instrument_install_ops_on_probed`]
/// 只做这一件事，⛔ 不再读盘）。于是：
/// * 轨还在 ⇒ 一次 `commit_ops` = **一步**可撤销的编辑（与 `#210` 的同步路径逐字相同）；
/// * 轨没了 ⇒ 逐字回执、Δ0；
/// * 用户先选了另一条轨 ⇒ 仍装到**点击时的那一条**（身份寻址，⛔ 不漂移）。
pub fn apply_pending_instrument_probe(
    ui: &MainWindow,
    state: &InstrumentProbeState,
    port: Option<&UndoPort>,
) -> bool {
    let Some((request, outcome)) = state.take() else {
        return false;
    };
    match outcome {
        crate::bridge::InstrumentSampleProbe::Installable { .. } => {
            let Some(port) = port else {
                ui.set_instrument_install_status(
                    crate::bridge::INSTRUMENT_INSTALL_NO_PROJECT_STATUS.into(),
                );
                return true;
            };
            let Some(project) = port.try_project() else {
                ui.set_instrument_install_status(
                    crate::bridge::INSTRUMENT_INSTALL_NO_PROJECT_STATUS.into(),
                );
                return true;
            };
            let Some(track) = project.tracks.get(&request.track_id) else {
                ui.set_instrument_install_status(
                    crate::bridge::INSTRUMENT_INSTALL_TRACK_GONE_STATUS.into(),
                );
                return true;
            };
            match crate::bridge::instrument_install_ops_on_probed(Some(track), &request.path) {
                Ok(ops) if ops.is_empty() => {
                    ui.set_instrument_install_status(
                        crate::bridge::instrument_already_installed_status(&request.name).into(),
                    );
                }
                Ok(ops) => {
                    if port
                        .commit_ops(now_ms(), "install instrument", ops)
                        .is_err()
                    {
                        ui.set_instrument_install_status(
                            crate::bridge::INSTRUMENT_INSTALL_COMMIT_REJECTED_STATUS.into(),
                        );
                        return true;
                    }
                    // ⚠ 顺序承重：`refresh_undo_window` 会把运行期回执清空（撤销后那句
                    // "已装上"就不该再留着）⇒ 先重投影，再写终态回执。
                    refresh_undo_window(ui, port, true);
                    ui.set_instrument_install_status(
                        crate::bridge::instrument_installed_status(&request.name).into(),
                    );
                }
                Err(reason) => {
                    ui.set_instrument_install_status(format!("装不了：{reason}").into());
                }
            }
        }
        failure => {
            // ⭐ `#211` 的核心：拒绝原因**进界面**（⛔ 只进 stderr）。
            let reason = crate::bridge::instrument_probe_failure_status(&request.name, &failure);
            ui.set_instrument_install_status(reason.clone().into());
            warn_install_rejected(&reason, &request.path);
        }
    }
    true
}

/// 设备机架（控制台 **Tab 3**）的**写入面** —— 任务单 `#157` 的"最小可编辑"。
///
/// 八个手势，每个 = **恰好一次** [`UndoPort::commit_ops`]（前两个是 `#157`，接下来两个是
/// `#163`，主总线那四个是 `#180` 的增删 ＋ `#181` 的旁通 / EQ）：
/// ⭐ `#181` 起**展示轨与主总线的旁通 / EQ 各走同一份实现体**
/// （[`bypass_device`] / [`DeviceEqWiring`]），`DeviceTarget` 是唯一自变量。
///
/// | 手势 | `.slint` 回调 | 提交的 `Op` | 撤销标签 |
/// | :--- | :--- | :--- | :--- |
/// | 旁通开关（逐台） | `device-bypass-toggle(device-id)` | [`yeban_model::Op::SetDeviceBypassed`]（按**身份**寻址） | `toggle device bypass` |
/// | EQ 低架增益拖动 | `device-eq-grab(device-id, fraction)` / `drag/release(fraction)` | [`yeban_model::Op::SetParam`] + `AutomationTarget::DeviceParam`（槽位由身份在两侧重解析） | `set device eq low gain` |
/// | 装一台内置效果器 | `device-insert-eq()` / `device-insert-reverb()` | [`yeban_model::Op::InsertDevice`]（参数取共享契约缺省值） | `insert device` |
/// | 删一台设备 | `device-remove(device-id)` | [`yeban_model::Op::RemoveDevice`]（按**身份**寻址） | `remove device` |
/// | ⭐ 给**主总线**装一台（`#180`） | `master-device-insert-eq()` / `master-device-insert-reverb()` | [`yeban_model::Op::InsertDevice`]，`track_id = master_bus_track_id` | `insert master device` |
/// | ⭐ 删**主总线**上的一台（`#180`） | `master-device-remove(device-id)` | [`yeban_model::Op::RemoveDevice`]，`track_id = master_bus_track_id`，按**身份**寻址 | `remove master device` |
/// | ⭐ 主总线旁通开关（`#181` / `#182`） | `master-device-bypass-toggle(device-id)` | [`yeban_model::Op::SetDeviceBypassed`]，`track_id = master_bus_track_id`，按**身份**寻址 | `toggle master device bypass` |
/// | ⭐ 主总线 EQ 低架增益（`#181` / `#182`） | `master-device-eq-grab(device-id, fraction)` / `drag/release(fraction)` | [`yeban_model::Op::SetParam`] + `AutomationTarget::DeviceParam`，目标在主总线上 | `set master device eq low gain` |
///
/// ## 为什么拖动期**不**提交（与推子同一条纪律）
///
/// 拖动期间只写**界面视图态**（`device-eq-gain-db`），松手时若值**真的变了**
/// （逐位比较，`-0.0` 与 `0.0` 不是同一个值）才提交一次 ⇒ 一次拖动 = 一步可撤销的编辑。
/// 按下就松手、拖出去又拖回同一个数、提交被模型拒绝 —— 三种情况都**不留下**撤销步。
///
/// ## 寻址：三条腿都按**设备身份**（⭐ 任务单 `#182`）
///
/// `#157`／`#181` 登记过一个真实窗口：界面只报**下标**，而"投影之后、点击之前"若设备链
/// 被增删（`#163` 起界面自己就能增删，MCP 也能），同一个下标可能指向**另一台**。
///
/// `#182` 起：
/// - **两条旁通腿**（展示轨 / 主总线）的 `.slint` 回调携带 `device-ids[i]` /
///   `master-device-ids[i]`（投影里那台设备的 `DeviceDefinition::id`）⇒ 宿主在权威链里
///   **按 id 查**，查不到就**响亮拒绝**（Δ0 ＋ 逐字原因），⛔ 不作用到另一台；
/// - **两条 EQ 腿**的回调携带 `device-eq-device-id` / `master-device-eq-device-id`
///   （同样是身份）⇒ [`DeviceEqWiring`] 在**按下**与**松手**两侧都用这个身份重解析
///   `slot_index`（模型层的 `AutomationTarget::DeviceParam` 只吃槽位），解析不到就拒绝。
///
/// 因此"下标 → 身份"的换算只发生在**投影内部**（`.slint` 的数组下标只用于显示顺序与
/// 无障碍 id），工程提交的那一格永远由身份定位。
///
/// ## ⭐ 任务单 `#211`：装乐器的字节探针**不在 UI 线程上跑**
///
/// `#210` 的裁决是"投影里不跑 `sfz_samples_present`（否则每次投影冻结界面），点击时才跑"。
/// 代价是点击那一帧会解码该乐器的**全部样本**（真库最慢 253,973 ms）⇒ 界面像卡死。
/// 本单把探针搬到工作线程：点击只做三件 O(1) 的事（排空上一次 / 同步写"检查中…" /
/// `spawn`），提交仍在 **UI 线程**的排空里（[`apply_pending_instrument_probe`]）。
/// 这一点与 `midi-in` / 采样库下载**同一条线程形态**，⛔ 不引入第三种。
///
/// ## 它**不**做什么
///
/// ⛔ 不做多选、⛔ 不写第二个 EQ 参数；⛔ 不碰引擎（插入链在
/// 实时侧的路由由 `yeban-engine` 自己的文档说清 —— 见交付报告的"能验与不能验"）。
pub fn wire_device_rack(ui: &MainWindow, port: &Rc<UndoPort>) -> InstrumentProbeState {
    let weak = ui.as_weak();
    let sample_download = SampleLibraryDownloadState::default();
    // ⭐ 任务单 `#211`：装乐器的字节探针**跨线程状态**（与 `SampleLibraryDownloadState`
    // 同一条形状）。生产路径忽略返回值 —— 那一跳由工作线程里的
    // `upgrade_in_event_loop` 完成；判据用它做确定性排空（见
    // [`wait_for_pending_instrument_probe`] / [`apply_pending_instrument_probe`]）。
    let instrument_probe = InstrumentProbeState::default();

    // ---------------------------------------------------------------- ① 旁通开关
    //
    // ⭐ 任务单 `#181`：展示轨与主总线共用**同一个**实现体 [`bypass_device`]，唯一自变量
    // 是 [`DeviceTarget`]：`DisplayTrack` ⇒ `bridge::device_bypass_ops_on`（`display_track`），
    // `Master` ⇒ `bridge::master_device_bypass_ops`（`master_device_track`）。
    // ⛔ 没有"默认 = 展示轨"的回退；两个回调各自**点名**目标。
    //
    // ⭐ 任务单 `#182`：两个回调的参数都是**设备身份**（`DeviceDefinition::id` 的 26 字符
    // 文本），⛔ 不是行下标 —— 下标在"投影之后、点击之前"会随增删漂移。
    ui.on_device_bypass_toggle({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move |device_id: SharedString| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Ok(device_id) = device_id.parse::<yeban_model::EntityId>() else {
                // 身份无法解析 ⇒ **Δ0**，⛔ 不退回按下标找一台替代品。
                eprintln!("[yeban-app] 展示轨设备旁通被拒绝：设备身份不是合法的 ULID 文本");
                return;
            };
            bypass_device(&ui, &port, DeviceTarget::DisplayTrack, device_id);
        }
    });
    ui.on_master_device_bypass_toggle({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move |device_id: SharedString| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Ok(device_id) = device_id.parse::<yeban_model::EntityId>() else {
                eprintln!("[yeban-app] 主总线设备旁通被拒绝：设备身份不是合法的 ULID 文本");
                return;
            };
            bypass_device(&ui, &port, DeviceTarget::Master, device_id);
        }
    });

    // ---------------------------------------------------------------- ② EQ 低架增益
    //
    // ⭐ 任务单 `#181`：两条腿共用 [`DeviceEqWiring`]（同一份手势状态机 + 同一份提交
    // 逻辑），唯一自变量是 [`DeviceTarget`] 与"预览写到哪个界面属性"。
    // 拖动期只写**视图态**；松手且值**逐位变了**才提交一次。
    //
    // ⭐ 任务单 `#182`：按下的回调携带**设备身份**（`device-eq-device-id` /
    // `master-device-eq-device-id`，投影里那一台设备的 `DeviceDefinition::id`）——
    // 宿主在按下 / 松手两侧都用它重解析 `slot_index`（⛔ 不用下标寻址）。
    let display_eq = Rc::new(DeviceEqWiring::new(
        Rc::clone(port),
        DeviceTarget::DisplayTrack,
        MainWindow::set_device_eq_gain_db,
    ));
    ui.on_device_eq_grab({
        let weak = weak.clone();
        let wiring = Rc::clone(&display_eq);
        move |device_id: SharedString, fraction| {
            let Ok(device_id) = device_id.parse::<yeban_model::EntityId>() else {
                eprintln!("[yeban-app] EQ 按下被拒绝：设备身份不是合法的 ULID 文本");
                return;
            };
            wiring.grab(&weak, device_id, fraction);
        }
    });
    ui.on_device_eq_drag({
        let weak = weak.clone();
        let wiring = Rc::clone(&display_eq);
        move |fraction| wiring.drag(&weak, fraction)
    });
    ui.on_device_eq_release({
        let weak = weak.clone();
        let wiring = Rc::clone(&display_eq);
        move || wiring.release(&weak)
    });

    let master_eq = Rc::new(DeviceEqWiring::new(
        Rc::clone(port),
        DeviceTarget::Master,
        MainWindow::set_master_device_eq_gain_db,
    ));
    ui.on_master_device_eq_grab({
        let weak = weak.clone();
        let wiring = Rc::clone(&master_eq);
        move |device_id: SharedString, fraction| {
            let Ok(device_id) = device_id.parse::<yeban_model::EntityId>() else {
                eprintln!("[yeban-app] 主总线 EQ 按下被拒绝：设备身份不是合法的 ULID 文本");
                return;
            };
            wiring.grab(&weak, device_id, fraction);
        }
    });
    ui.on_master_device_eq_drag({
        let weak = weak.clone();
        let wiring = Rc::clone(&master_eq);
        move |fraction| wiring.drag(&weak, fraction)
    });
    ui.on_master_device_eq_release({
        let weak = weak.clone();
        let wiring = Rc::clone(&master_eq);
        move || wiring.release(&weak)
    });

    // ---------------------------------------------------------------- ③ 装设备（⭐ `#163`）
    //
    // 两个按钮共用一处实现体（`effect` 是唯一自变量）⇒ 参数表 / 追加位置 / 提交标签
    // 不可能在两处漂移。一次点击 = **恰好一次** `commit_ops`（一个 `Op::InsertDevice`）。
    for (effect, callback) in [
        (crate::bridge::BuiltinEffect::Eq, "eq"),
        (crate::bridge::BuiltinEffect::Reverb, "reverb"),
    ] {
        let weak = weak.clone();
        let port = Rc::clone(port);
        let handler = move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            insert_builtin_device(&ui, &port, effect, DeviceTarget::DisplayTrack);
        };
        match callback {
            "eq" => ui.on_device_insert_eq(handler),
            _ => ui.on_device_insert_reverb(handler),
        }
    }

    // ---------------------------------------------------------------- ④ 删设备（⭐ `#163`）
    //
    // 携带**设备身份**（`device-id` 数组，⛔ 不是行下标）：宿主按身份在权威工程的展示轨里
    // 重新寻址；一次点击 = **恰好一次** `commit_ops`（一个 `Op::RemoveDevice`）。
    ui.on_device_remove({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move |device_id: SharedString| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Ok(device_id) = device_id.parse::<yeban_model::EntityId>() else {
                eprintln!("[yeban-app] 删设备被拒绝：设备身份不是合法的 ULID 文本");
                return;
            };
            let Some(project) = port.try_project() else {
                eprintln!("[yeban-app] 删设备：权威会话此刻没有活跃工程");
                return;
            };
            // ⭐ `#192`：主语 = 选中轨 or 兜底（与投影同一次解析的口径）。
            let ops = match crate::bridge::device_remove_ops_on(
                display_track(&ui, &project),
                device_id,
            ) {
                Ok(ops) => ops,
                Err(reason) => {
                    eprintln!("[yeban-app] 删设备被拒绝：{reason}");
                    refresh_undo_window(&ui, &port, true);
                    return;
                }
            };
            if port.commit_ops(now_ms(), "remove device", ops).is_err() {
                eprintln!("[yeban-app] 删设备被拒绝（工程未改）");
            }
            refresh_undo_window(&ui, &port, true);
        }
    });

    // ------------------------------------------------- ③b / ④b 主总线设备（⭐ `#180`）
    //
    // ⭐ 与上面 ③ / ④ **同一份实现体**，唯一自变量是 `DeviceTarget::Master`：
    // 目标轨 = `master_bus_track_id`（`bridge::master_device_track`），⛔ 不是展示轨
    // （`device_rack_track` **排除**主总线 —— 用它就会把设备装到展示轨上）。
    // 一次点击 = **恰好一次** `commit_ops`（一个 `Op::InsertDevice` / `RemoveDevice`）。
    for (effect, callback) in [
        (crate::bridge::BuiltinEffect::Eq, "eq"),
        (crate::bridge::BuiltinEffect::Reverb, "reverb"),
    ] {
        let weak = weak.clone();
        let port = Rc::clone(port);
        let handler = move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            insert_builtin_device(&ui, &port, effect, DeviceTarget::Master);
        };
        match callback {
            "eq" => ui.on_master_device_insert_eq(handler),
            _ => ui.on_master_device_insert_reverb(handler),
        }
    }

    // 携带**设备身份**（`master-device-ids`，⛔ 不是行下标）：宿主按身份在权威工程的
    // **主总线**设备链里重新寻址；一次点击 = 恰好一次 `commit_ops`。
    ui.on_master_device_remove({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move |device_id: SharedString| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Ok(device_id) = device_id.parse::<yeban_model::EntityId>() else {
                eprintln!("[yeban-app] 删主总线设备被拒绝：设备身份不是合法的 ULID 文本");
                return;
            };
            let Some(project) = port.try_project() else {
                eprintln!("[yeban-app] 删主总线设备：权威会话此刻没有活跃工程");
                return;
            };
            let ops = match crate::bridge::master_device_remove_ops(&project, device_id) {
                Ok(ops) => ops,
                Err(reason) => {
                    eprintln!("[yeban-app] 删主总线设备被拒绝：{reason}");
                    refresh_undo_window(&ui, &port, true);
                    return;
                }
            };
            if port
                .commit_ops(now_ms(), "remove master device", ops)
                .is_err()
            {
                eprintln!("[yeban-app] 删主总线设备被拒绝（工程未改）");
            }
            refresh_undo_window(&ui, &port, true);
        }
    });

    // ---------------------------------------------------------------- ⑤ 装乐器（⭐ `#164`）
    //
    // 界面只上报**下标**（⛔ 不拼路径、⛔ 不写 Op）。宿主从 `bridge::library_instrument`
    // 取那条**确定的**库内路径，一次点击 = **恰好一次** `commit_ops`。
    // 值没变 ⇒ `ops` 为空 ⇒ **不提交**（⛔ 不制造"什么也没改"的撤销步）。
    //
    // ⭐ 任务单 `#210`：下标索引的是**库内全部**可装乐器（`manifest.json` × 库根），
    // 不再是 2 份硬编码候选。库根为空 ⇒ 0 个按钮；万一有人直接调回调（判据会这么做），
    // 下标越界 ⇒ **Δ0** 并把**清单为空**的逐字原因打出来（⛔ 不静默、⛔ 不装成静音）。
    //
    // ⭐ ⭐ **任务单 `#211`：字节探针搬到后台**（缺陷 = "点一下界面卡 4 分钟"）。
    // `#210` 的裁决是"投影里不跑 `sfz_samples_present`（否则每次投影冻结界面），点击时才跑"
    // ⇒ 点击那一帧因此要解码该乐器的**全部样本**（真库最慢 253,973 ms）。本单把这一步移到
    // **工作线程**上，UI 线程只做三件 O(1) 的事：排空上一次、同步写"检查中…"、`spawn`。
    //
    // 形态与 ⑥ 的采样库下载 / `midi-in` 逐条同款（⛔ 不复用它们的槽，因为结果类型不同）：
    //
    // ```text
    //   UI 线程: on_instrument_install(index)
    //     ├─ 先排空上一次结果（自愈：没有事件循环的环境也不会永远停在 pending）
    //     ├─ 解析**这一次**的目标轨（选中轨 or 兜底）⇒ 只带 `EntityId` 过线程
    //     ├─ `begin` 挡住并发的第二次点击（⛔ 不排队、⛔ 不开第二个解码线程）
    //     ├─ 同步写 "检查中…：<名字>"（**spawn 之前** ⇒ 点击后立刻可观察）
    //     └─ thread::spawn { bridge::probe_instrument_samples(path) → 槽
    //                        → upgrade_in_event_loop { invoke_instrument_probe_drain_requested() } }
    //   工作线程: ⛔ 不碰任何 Slint 对象（只读盘 / 解码 / 写 Send 槽）
    //   UI 线程: instrument-probe-drain-requested → apply_pending_instrument_probe
    //            （**唯一**提交 `Op::SetTrackSampler` 的地方；失败 ⇒ 原因进界面）
    // ```
    {
        let weak_for_drain = weak.clone();
        let state = instrument_probe.clone();
        let port = Rc::clone(port);
        ui.on_instrument_probe_drain_requested(move || {
            let Some(ui) = weak_for_drain.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            apply_pending_instrument_probe(&ui, &state, Some(&port));
        });
    }
    ui.on_instrument_install({
        let weak = weak.clone();
        let port = Rc::clone(port);
        let state = instrument_probe.clone();
        move |index| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // 自愈：先排空上一次的结果（即使上次没有事件循环来排空）。
            apply_pending_instrument_probe(&ui, &state, Some(&port));
            let Some(instrument) = usize::try_from(index)
                .ok()
                .and_then(crate::bridge::library_instrument)
            else {
                // ⭐ `#165`／`#210`：清单为空时把"为什么一个都不能装"逐字说出来。
                let reason = if crate::bridge::library_instruments().is_empty() {
                    crate::bridge::INSTRUMENT_INSTALL_NO_BUILTIN_REASON
                } else {
                    "下标不在库内可装乐器清单里"
                };
                ui.set_instrument_install_status(format!("装不了：{reason}").into());
                eprintln!("[yeban-app] 装乐器被拒绝：{reason}（下标 {index}）");
                return;
            };
            let Some(project) = port.try_project() else {
                ui.set_instrument_install_status(
                    crate::bridge::INSTRUMENT_INSTALL_NO_PROJECT_STATUS.into(),
                );
                eprintln!("[yeban-app] 装乐器：权威会话此刻没有活跃工程");
                return;
            };
            // ⭐ `#192`：乐器装到**选中轨 or 兜底**（与服务面板的那条轨同一条）。
            // ⚠ 只在**这里**解析一次：探测期间换选区不会把乐器装到另一条轨。
            let Some(track) = display_track(&ui, &project) else {
                let reason = crate::bridge::INSTRUMENT_INSTALL_NO_TRACK_REASON;
                ui.set_instrument_install_status(format!("装不了：{reason}").into());
                eprintln!("{INSTALL_REJECTED_STDERR_PREFIX}{reason}");
                return;
            };
            let request = InstrumentProbeRequest {
                track_id: track.id,
                name: instrument.name.clone(),
                path: instrument.path.clone(),
            };
            // ⭐ 在飞 ⇒ **逐字拒绝**（⛔ 不并发）。第二次点击仍然立刻返回，界面不卡。
            if !state.begin(request.clone()) {
                ui.set_instrument_install_status(
                    crate::bridge::INSTRUMENT_INSTALL_BUSY_STATUS.into(),
                );
                return;
            }
            // ⭐ **中间态**：在 spawn **之前**同步写 ⇒ 点击后立刻可观察（⛔ 不靠竞态）。
            ui.set_instrument_install_status(
                crate::bridge::instrument_checking_status(&request.name).into(),
            );
            let worker_state = state.clone();
            let worker_weak = weak.clone();
            let path = request.path.clone();
            let handle = std::thread::spawn(move || {
                // ⭐ 慢的就是这一句（读 SFZ ＋ 解码全部样本）；它在**工作线程**上跑。
                let outcome = crate::bridge::probe_instrument_samples(&path);
                worker_state.finish(outcome);
                // 唯一跨线程的一跳：`Weak` 是 `Send`，升级后的写入仍发生在 UI 线程。
                // 没有事件循环的平台这里返回 `Err` —— 结果仍在槽里，下一次点击 / 判据排空。
                let _ = worker_weak.upgrade_in_event_loop(|ui| {
                    ui.invoke_instrument_probe_drain_requested();
                });
            });
            state.set_worker(handle);
        }
    });

    // ------------------------------------------------- ⑥ 下载采样库（⭐ `#196`）
    //
    // ⭐ **只在用户显式点按钮时联网**：启动 / 投影 / 判据都不调这里。
    // 工作线程只做三件事：跑 `fetch-samples.py --all --dest <库根>`、把结果放进
    // Send 槽、请求 UI 线程排空 ⇒ ⛔ 它一个 Slint 对象都不碰。
    {
        let weak_for_drain = weak.clone();
        let state = sample_download.clone();
        let port = Rc::clone(port);
        ui.on_sample_library_download_drain_requested(move || {
            let Some(ui) = weak_for_drain.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            apply_pending_sample_library_download(&ui, &state, &port);
        });
    }
    ui.on_sample_library_download({
        let weak = weak.clone();
        let state = sample_download;
        let port = Rc::clone(port);
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // 自愈：先排空上一次的结果（即使上次没有事件循环来排空）。
            apply_pending_sample_library_download(&ui, &state, &port);
            if state.is_in_flight() {
                ui.set_sample_library_status(
                    "采样库下载已在进行中⏳（⛔ 不并发、⛔ 不排队）".into(),
                );
                return;
            }
            let options = crate::samples::DownloadOptions::for_app();
            // 中间态：在 spawn **之前**同步写 ⇒ 点击后立刻可观察。
            ui.set_sample_library_status(crate::samples::in_flight_status(&options.dest).into());
            if !state.begin() {
                return;
            }
            let worker_state = state.clone();
            let worker_weak = weak.clone();
            std::thread::spawn(move || {
                let outcome = crate::samples::run_download(&options);
                worker_state.finish(outcome);
                // 唯一跨线程的一跳：`Weak` 是 `Send`，升级后的写入仍发生在 UI 线程。
                // 没有事件循环的平台这里返回 `Err` —— 结果仍在槽里，下一次点击排空。
                let _ = worker_weak.upgrade_in_event_loop(|ui| {
                    ui.invoke_sample_library_download_drain_requested();
                });
            });
        }
    });

    // ⭐ `#211`：把跨线程状态交出去 —— 判据用它做**确定性**排空（join + apply）。
    // 生产路径（`main.rs` / `live_surface.rs`）忽略返回值 —— 那一跳由工作线程里的
    // `upgrade_in_event_loop` 完成。
    instrument_probe
}

/// 设备机架的**目标轨**（⭐ 任务单 `#180` 起：装 / 删；⭐ `#181` 起：旁通 / EQ 滑块）。
///
/// ⚠ 两个变体各自绑定一条**显式**的轨：
/// - [`Self::DisplayTrack`] ⇒ [`crate::bridge::display_track`]（⭐ `#192`：选中轨 or 兜底）；
/// - [`Self::Master`] ⇒ [`crate::bridge::master_device_track`]（`master_bus_track_id`）。
///
/// ⛔ 不存在"默认 = 展示轨"的回退：主总线的四个手势（装 / 删 / 旁通 / EQ）都必须
/// **点名** `Master`，否则 `InsertDevice` / `RemoveDevice` / `SetDeviceBypassed` /
/// `SetParam` 会指错轨（`#163` 的洞）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeviceTarget {
    /// 设备架面板第一段（展示轨）。
    DisplayTrack,
    /// 设备架面板第二段（主总线）。
    Master,
}

impl DeviceTarget {
    /// 撤销标签 / 日志里的主语（⛔ 只是文案；寻址仍走 [`bypass_device`] /
    /// [`device_eq_track`] 的显式 `match`）。
    fn label(self) -> &'static str {
        match self {
            Self::DisplayTrack => "展示轨设备",
            Self::Master => "主总线设备",
        }
    }
}
/// 切换一台设备旁通的**唯一**实现体（展示轨与主总线共用；`target` ＋ `device_id` 是
/// 唯一自变量）。
///
/// ⭐ 任务单 `#182`：`device_id` 是**设备身份**（`DeviceDefinition::id`），⛔ 不是行下标
/// —— 权威链在"投影之后、点击之前"可能被增删，身份不会漂移。
/// `Op` 由 [`crate::bridge::device_bypass_ops`] / [`crate::bridge::master_device_bypass_ops`]
/// 从权威工程按身份现查（`old_bypassed` 永远是当前值），宿主⛔ 不硬编码任何设备身份。
/// 一次点击 = **恰好一次** `commit_ops`；身份查不到 ⇒ **Δ0** ＋ 逐字原因。
fn bypass_device(ui: &MainWindow, port: &UndoPort, target: DeviceTarget, device_id: EntityId) {
    let Some(project) = port.try_project() else {
        eprintln!("[yeban-app] 设备旁通：权威会话此刻没有活跃工程");
        return;
    };
    let ops = match target {
        // ⭐ `#192`：展示轨那条腿的主语 = 选中轨 or 兜底（与投影同一次解析的口径）。
        DeviceTarget::DisplayTrack => {
            crate::bridge::device_bypass_ops_on(display_track(ui, &project), device_id)
        }
        DeviceTarget::Master => crate::bridge::master_device_bypass_ops(&project, device_id),
    };
    let commit_label = match target {
        DeviceTarget::DisplayTrack => "toggle device bypass",
        DeviceTarget::Master => "toggle master device bypass",
    };
    let log_label = target.label();
    let ops = match ops {
        Ok(ops) => ops,
        Err(reason) => {
            eprintln!("[yeban-app] {log_label}旁通被拒绝：{reason}");
            refresh_undo_window(ui, port, true);
            return;
        }
    };
    if port.commit_ops(now_ms(), commit_label, ops).is_err() {
        eprintln!("[yeban-app] {log_label}旁通提交被拒绝（工程未改）");
        return;
    }
    refresh_undo_window(ui, port, true);
}

/// EQ 手势的**目标轨**（展示轨 / 主总线；`target` ＋ `selected` 是唯一自变量）。
///
/// ⭐ 任务单 `#182`：EQ 不再按"第一台带 `eq_low_gain` 的设备"寻址 —— 目标由界面带回来的
/// **设备身份**钉住（[`crate::bridge::device_eq_low_gain_address_on_device`]）。本函数只
/// 回答"身份该在哪条轨里查"，⛔ 不返回下标（那会重新引入窗口）。
///
/// ⭐ 任务单 `#192`：展示轨那一条腿由 [`crate::bridge::display_track`] 解析
/// （选中轨 or 兜底），`selected` 是窗口属性 `selected-track-id` 的原文。
fn device_eq_track<'a>(
    project: &'a yeban_model::project::YebanProjectV1,
    target: DeviceTarget,
    selected: Option<&str>,
) -> Option<&'a yeban_model::project::TrackV3> {
    match target {
        DeviceTarget::DisplayTrack => crate::bridge::display_track(project, selected),
        DeviceTarget::Master => crate::bridge::master_device_track(project),
    }
}

/// ⭐ 任务单 `#182`：EQ 手势**按身份**解析不到目标时的逐字原因（按目标轨选主语）。
fn eq_device_gone_reason(target: DeviceTarget) -> &'static str {
    match target {
        DeviceTarget::DisplayTrack => crate::bridge::DEVICE_EQ_DEVICE_GONE_REASON,
        DeviceTarget::Master => crate::bridge::MASTER_DEVICE_EQ_DEVICE_GONE_REASON,
    }
}

/// EQ 低架增益滑块的**唯一**手势实现（展示轨与主总线共用；`target` ＋ 预览落点是唯一
/// 自变量）。
///
/// 三条纪律与 `#157` 逐字相同：
/// 1. **拖动期不提交**：`grab` / `drag` 只写界面视图态（`set_gain` 指向的属性）；
/// 2. **松手才提交**：`release` 只在"值逐位变了且不等于起点"时提交一次
///    `Op::SetParam`，成功与失败都重投影一次（成功读新值 / 失败抹掉预览）；
/// 3. **不信界面**：目标在 `grab` 与 `release` 两侧都从权威工程现求；起点不再是
///    当前值（撤销 / 换工程）⇒ ⛔ 不按过期基线提交。
///
/// ⭐ 任务单 `#182`：寻址单位是**设备身份**（界面在 `grab` 时带回来的
/// `DeviceDefinition::id`）。`AutomationTarget::DeviceParam` 只吃 `slot_index`，
/// 所以槽位在按下 / 松手两侧都用身份**重解析**
/// （[`crate::bridge::device_eq_low_gain_address_on_device`]）—— 身份查不到 ⇒ 拒绝
/// （Δ0 ＋ 逐字原因），⛔ 不改到"此刻停在同一格的那台"。
struct DeviceEqWiring {
    /// 权威撤销端口（提交与重投影的唯一出口）。
    port: Rc<UndoPort>,
    /// 目标轨（展示轨 / 主总线）。
    target: DeviceTarget,
    /// 拖动期的预览写到哪个界面属性（`device-eq-gain-db` / `master-device-eq-gain-db`）。
    set_gain: fn(&MainWindow, f32),
    /// 当前手势（每次拖动一份；`Idle` 之外的状态只在三条回调之间传递）。
    gesture: RefCell<DeviceEqGesture>,
}

impl DeviceEqWiring {
    fn new(port: Rc<UndoPort>, target: DeviceTarget, set_gain: fn(&MainWindow, f32)) -> Self {
        Self {
            port,
            target,
            set_gain,
            gesture: RefCell::new(DeviceEqGesture::Idle),
        }
    }

    /// 提交成功时写进撤销栈的标签（按目标轨命名；⛔ 不含来源视图前缀）。
    fn commit_label(&self) -> &'static str {
        match self.target {
            DeviceTarget::DisplayTrack => "set device eq low gain",
            DeviceTarget::Master => "set master device eq low gain",
        }
    }

    /// 日志里的主语（与撤销标签同一条口径，⛔ 不各写一套）。
    fn log_label(&self) -> &'static str {
        match self.target {
            DeviceTarget::DisplayTrack => "EQ",
            DeviceTarget::Master => "主总线 EQ",
        }
    }

    /// 按下：用界面带回来的**设备身份**在权威链里解析 `slot_index` / 当前值 / 值域。
    ///
    /// ⭐ 任务单 `#182`：解析不到（身份被删 / 那一台已经没有 `eq_low_gain` / 没有目标轨）
    /// ⇒ **Δ0** ＋ 逐字原因，并重投影一次把界面拉回权威状态。
    fn grab(
        &self,
        weak: &slint::Weak<MainWindow>,
        device_id: yeban_model::EntityId,
        fraction: f32,
    ) {
        let Some(project) = self.port.try_project() else {
            *self.gesture.borrow_mut() = DeviceEqGesture::Idle;
            return;
        };
        // ⭐ `#192`：目标轨的主语由窗口上的选中轨决定（读不到窗口 ⇒ 走兜底 = 旧行为）。
        let selected = weak.upgrade().and_then(|ui| selected_track_id(&ui));
        let resolved =
            device_eq_track(&project, self.target, selected.as_deref()).and_then(|track| {
                crate::bridge::device_eq_low_gain_address_on_device(track, device_id)
            });
        let Some((_, _, value, min_db, max_db)) = resolved else {
            *self.gesture.borrow_mut() = DeviceEqGesture::Idle;
            // 单行：`tests/mixer_toggle_labels.rs` 用"整行就是字面量"的形状扫这段源码
            // （`fn wire_mixer_switch` 之后到 `fn read_strings` 之前），多行字面量会被数成
            // 第三 / 第四条提交消息 ⇒ 这里⛔ 不换行。
            let reason = eq_device_gone_reason(self.target);
            eprintln!("[yeban-app] {} 按下被拒绝：{reason}", self.log_label());
            if let Some(ui) = weak.upgrade() {
                refresh_undo_window(&ui, &self.port, true);
            }
            return;
        };
        // 按下即定位（点一下滑块就跳到那一格）⇒ 一次**单击**也是一次可撤销的编辑，
        // 而不是"点了没反应"。`start_db` 仍是按下那一刻的工程值（提交的 `old_val`）。
        let pressed_db = db_from_fraction(fraction, min_db, max_db);
        *self.gesture.borrow_mut() = DeviceEqGesture::Dragging {
            device_id,
            start_db: value,
            last_db: pressed_db,
            min_db,
            max_db,
            moved: pressed_db.to_bits() != value.to_bits(),
        };
        if pressed_db.to_bits() != value.to_bits()
            && let Some(ui) = weak.upgrade()
        {
            (self.set_gain)(&ui, pressed_db);
        }
    }

    fn drag(&self, weak: &slint::Weak<MainWindow>, fraction: f32) {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        // ⭐ `#212`：**借的作用域只包住状态本身** —— 记下这一移的新值，然后**放借**，
        // 再调界面（`set_gain` 是 `ui.set_*`）。持借调界面正是 `#212` 的崩法：
        // 界面写入若回调到同一份 `RefCell`，`borrow_mut()` 就会 panic。
        let next_db = {
            let mut state = self.gesture.borrow_mut();
            let DeviceEqGesture::Dragging {
                last_db,
                min_db,
                max_db,
                moved,
                ..
            } = &mut *state
            else {
                return;
            };
            let next_db = db_from_fraction(fraction, *min_db, *max_db);
            if next_db.to_bits() == last_db.to_bits() {
                return;
            }
            *last_db = next_db;
            *moved = true;
            next_db
        };
        // 拖动期只写**视图态**（投影下一次注入会覆盖它）—— ⛔ 不提交工程内容。
        (self.set_gain)(&ui, next_db);
    }

    /// 松手：用**同一个设备身份**重解析槽位 / 参数下标，再提交一次 `Op::SetParam`。
    ///
    /// ⭐ 任务单 `#182`：身份查不到 ⇒ 拒绝（Δ0 ＋ 逐字原因）并重投影。
    fn release(&self, weak: &slint::Weak<MainWindow>) {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        let taken = std::mem::replace(&mut *self.gesture.borrow_mut(), DeviceEqGesture::Idle);
        let DeviceEqGesture::Dragging {
            device_id,
            start_db,
            last_db,
            moved,
            ..
        } = taken
        else {
            return;
        };
        if !moved || last_db.to_bits() == start_db.to_bits() {
            return;
        }
        let log_label = self.log_label();
        let commit_label = self.commit_label();
        let Some(project) = self.port.try_project() else {
            eprintln!("[yeban-app] {log_label} 提交：权威会话此刻没有活跃工程");
            refresh_undo_window(&ui, &self.port, true);
            return;
        };
        // ⭐ `#192`：松手侧读**同一个**窗口选中轨 ⇒ 与按下时的目标轨同一条。
        let selected = selected_track_id(&ui);
        let resolved =
            device_eq_track(&project, self.target, selected.as_deref()).and_then(|track| {
                crate::bridge::device_eq_low_gain_address_on_device(track, device_id)
                    .map(|address| (track.id, address.0, address.1, address.2))
            });
        let Some((track_id, slot_index, param_index, current_db)) = resolved else {
            // 同上：单行字面量（⛔ 不换行，见 `grab` 里的注释）。
            let reason = eq_device_gone_reason(self.target);
            eprintln!("[yeban-app] {log_label} 提交被拒绝：{reason}");
            refresh_undo_window(&ui, &self.port, true);
            return;
        };
        // 手势期间工程被撤销 / 换掉 ⇒ 起点已经不是当前值：⛔ 不按一个过期基线提交。
        if current_db.to_bits() != start_db.to_bits() {
            eprintln!("[yeban-app] {log_label} 提交被放弃：拖动期间工程值已变（陈旧基线）");
            refresh_undo_window(&ui, &self.port, true);
            return;
        }
        let op = yeban_model::Op::SetParam {
            target: yeban_model::AutomationTarget::DeviceParam {
                track_id,
                slot_index,
                param_index,
            },
            old_val: start_db,
            new_val: last_db,
        };
        if self
            .port
            .commit_ops(now_ms(), commit_label, vec![op])
            .is_err()
        {
            eprintln!("[yeban-app] {log_label} 提交被拒绝（工程未改）");
        }
        // 成功与失败都重投影一次：成功 ⇒ 读新值；失败 ⇒ 把拖动期的预览值抹掉。
        refresh_undo_window(&ui, &self.port, true);
    }
}

/// 装一台内置效果器的唯一实现体（展示轨与主总线共用；`effect` ＋ `target` 是唯一自变量）。
///
/// 参数缺省值 / 单位 / 追加位置全部由 [`crate::bridge::device_insert_ops`] /
/// [`crate::bridge::master_device_insert_ops`] 从权威工程与共享契约求出 ⇒ 宿主不硬编码
/// 任何参数值，也不信任界面上的设备链长度。
fn insert_builtin_device(
    ui: &MainWindow,
    port: &UndoPort,
    effect: crate::bridge::BuiltinEffect,
    target: DeviceTarget,
) {
    let Some(project) = port.try_project() else {
        eprintln!("[yeban-app] 装设备：权威会话此刻没有活跃工程");
        return;
    };
    let (commit_label, log_label) = match target {
        DeviceTarget::DisplayTrack => ("insert device", "装设备"),
        DeviceTarget::Master => ("insert master device", "给主总线装设备"),
    };
    let ops = match target {
        // ⭐ `#192`：展示轨那条腿的主语 = 选中轨 or 兜底（与投影同一次解析的口径）。
        DeviceTarget::DisplayTrack => {
            crate::bridge::device_insert_ops_on(display_track(ui, &project), effect)
        }
        DeviceTarget::Master => crate::bridge::master_device_insert_ops(&project, effect),
    };
    let ops = match ops {
        Ok(ops) => ops,
        Err(reason) => {
            eprintln!("[yeban-app] {log_label}被拒绝：{reason}");
            refresh_undo_window(ui, port, true);
            return;
        }
    };
    if port.commit_ops(now_ms(), commit_label, ops).is_err() {
        eprintln!("[yeban-app] {log_label}被拒绝（工程未改）");
    }
    refresh_undo_window(ui, port, true);
}

/// 建一个**空 MIDI 片段**（控制台 Tab 3 的同一面板，任务单 `#163`）的写入面。
///
/// 一次点击 = **恰好一次** [`UndoPort::commit_ops`]；那一次提交里是两条 `Op`
/// （`AddClip` ＋ `AddClipPlacement`，`Op::Batch` 原子施加）⇒ 一步可撤销的编辑。
/// 片段身份 / 名称 / 时长全部由 [`crate::bridge::clip_create_ops`] 从权威工程求出
/// （时长 = 4 × 工程拍号的小节长度，⛔ 界面与宿主都不写 tick 字面量）。
pub fn wire_clip_create(ui: &MainWindow, port: &Rc<UndoPort>) {
    let weak = ui.as_weak();
    ui.on_clip_create({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Some(project) = port.try_project() else {
                eprintln!("[yeban-app] 建片段：权威会话此刻没有活跃工程");
                return;
            };
            // ⭐ `#192`：片段摆到**选中轨 or 兜底**上（与设备架 / 发送条同一条）。
            let ops =
                match crate::bridge::clip_create_ops_on(&project, display_track(&ui, &project)) {
                    Ok(ops) => ops,
                    Err(reason) => {
                        eprintln!("[yeban-app] 建片段被拒绝：{reason}");
                        refresh_undo_window(&ui, &port, true);
                        return;
                    }
                };
            if port.commit_ops(now_ms(), "create midi clip", ops).is_err() {
                eprintln!("[yeban-app] 建片段被拒绝（工程未改）");
            }
            refresh_undo_window(&ui, &port, true);
        }
    });
}

/// EQ 滑块手势的中间态（**会话运行态**，不进 `.yeban`）。
///
/// 与 [`MixerGesture`] 同一条纪律：`Idle` 是唯一的空态；`Escape` / 指针离开窗口走的是
/// `.slint` 的上报（`PointerEventKind.cancel` ⇒ `device-eq-release`），因此不存在
/// "黏住的拖动"。`start_db` 是**提交时的 `old_val`**，`last_db` 是收尾要提交的 `new_val`。
///
/// ⭐ 任务单 `#182`：`device_id` 是按下时界面带回来的**设备身份** —— 松手时用它重解析
/// `slot_index`（⛔ 不用按下那一刻的下标）。
#[derive(Clone, Copy, Debug, PartialEq)]
enum DeviceEqGesture {
    /// 没有手势。
    Idle,
    /// 正在拖：身份 / 起点 / 最近值 / 值域（来自契约）/ 是否真的动过。
    Dragging {
        /// 目标设备的身份（`DeviceDefinition::id`）—— 松手时按它重解析槽位。
        device_id: yeban_model::EntityId,
        /// 按下时的工程值（`old_val`）。
        start_db: f32,
        /// 最近一次算出的值（`new_val`）。
        last_db: f32,
        /// 值域下界 (dB)。
        min_db: f32,
        /// 值域上界 (dB)。
        max_db: f32,
        /// 是否收到过至少一次**改变数值**的 move。
        moved: bool,
    },
}

/// 滑块位置（0.0–1.0 归一化）→ dB：在**模型给的** `[min_db, max_db]` 上线性映射并夹紧。
///
/// `NaN` / `±∞` 归中值（`0.5`）而不是 `clamp` 的端点：越界输入本来就该被 `clamp`
/// 拦掉，但"非数"不是"拖到最左"，把它当端点会制造一个没人拖出来的值。
///
/// ⚠ 两个消费者共用这**一处**实现：EQ 低架增益（`#157`，值域来自效果器契约）与发送
/// 增益（`#159`，值域来自 `NOMINAL_GAIN_MIN_DB/MAX_DB`）—— 两套值域的**来源**不同，
/// 但"归一化位置 → dB"是同一段算术，⛔ 不复制第二份。
fn db_from_fraction(fraction: f32, min_db: f32, max_db: f32) -> f32 {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.5
    };
    min_db + fraction * (max_db - min_db)
}

// ===========================================================================
// 发送条（控制台 Tab 3，任务单 `#159`）
// ===========================================================================

/// 发送条（控制台 **Tab 3** 设备机架面板里的"发送"区）的**写入面**。
///
/// 三个手势，每个 = **恰好一次** [`UndoPort::commit_ops`]：
///
/// | 手势 | `.slint` 回调 | 提交的 `Op` | 撤销标签 |
/// | :--- | :--- | :--- | :--- |
/// | 改发送增益 | `send-gain-grab/drag/release(edge-id)` | [`yeban_model::Op::SetRoutingGain`] | `"set send gain"` |
/// | 新建发送 | `send-create()` | [`yeban_model::Op::ConnectRouting`]（端点不在图里时，同一次提交先补 `AddRoutingNode`） | `"create send"` |
/// | 删除发送（⭐ `#160`） | `send-remove(edge-id)` | [`yeban_model::Op::DisconnectRouting`] | `"remove send"` |
///
/// ## 为什么拖动期**不**提交（与 EQ 滑块同一条纪律）
///
/// 拖动期间只写**界面视图态**（`send-gain-dbs`），松手时若值**真的变了**才提交一次
/// ⇒ 一次拖动 = 一步可撤销的编辑。按下就松手、拖出去又拖回同一个数、提交被模型拒绝
/// —— 三种情况都**不留下**撤销步。`None`（单位增益）与 `Some(0.0)` 的区别住在模型里：
/// 滑块把 `None` 画在 0 dB 的位置，但拖回 0 dB **不会**把 `None` 改写成 `Some(0.0)`
/// （那会改变 `Option` 语义而听觉上什么都没发生）。
///
/// ## 寻址：空间上只认身份，不认行下标
///
/// `.slint` 的三个滑块回调与删除按钮都携带**路由边身份**（`RoutingEdge::id` 的 26 字符
/// 文本）。宿主在**按下**与**松手**两侧都从权威工程重新查一次
/// （[`crate::bridge::send_gain_target`]），因此 `Op` 携带的 `old_gain_db` 永远是
/// **当前**值（陈旧会被模型的逐位比对拒掉）；行下标只用于拖动期的视觉预览。
///
/// “新建发送”的设备身份与目的地都在提交前从权威工程重求：
/// 源 = [`crate::bridge::send_strip_track`]（与上方设备链**同一条**轨），
/// 目的地 = [`crate::bridge::send_aux_target`]（身份序第一条 `AuxReturn` 轨）。
/// 可用性（含**防重复**：同源同目的地的 `SendToAux` 已存在 ⇒ 拒绝）来自
/// [`crate::bridge::send_create_availability`] —— 与投影**同一份**判定。
/// 新边的 `id` 由项目既有的身份分配器 [`yeban_model::EntityId::new`] 生成
/// （⛔ 不硬编码、⛔ 不复用已有边身份）。
///
/// ## 它**不**做什么
///
/// ⛔ 不建轨（那是 [`wire_track_strip`] 的 `+ MIDI / + 音频 / + 辅助返回`）；
/// ⛔ 不碰引擎（发送在实时侧的可听性由 `yeban-engine` 自己文档化 —— 见交付报告的
/// "能验与不能验"）。
pub fn wire_send_strip(ui: &MainWindow, port: &Rc<UndoPort>) {
    let weak = ui.as_weak();
    let gesture = Rc::new(RefCell::new(SendGainGesture::Idle));

    // ---------------------------------------------------------------- ① 改发送增益
    ui.on_send_gain_grab({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move |edge_id: SharedString, fraction: f32| {
            let Ok(edge_id) = edge_id.parse::<yeban_model::EntityId>() else {
                *gesture.borrow_mut() = SendGainGesture::Idle;
                return;
            };
            let Some(project) = port.try_project() else {
                *gesture.borrow_mut() = SendGainGesture::Idle;
                return;
            };
            // ⭐ `#192`：边的源必须是**面板服务的那条轨**（选中轨 or 兜底）。
            let selected = weak.upgrade().and_then(|ui| selected_track_id(&ui));
            let track = crate::bridge::display_track(&project, selected.as_deref());
            let Some((current_db, min_db, max_db)) =
                crate::bridge::send_gain_target_on(&project, track, edge_id)
            else {
                *gesture.borrow_mut() = SendGainGesture::Idle;
                return;
            };
            // 按下即定位（点一下滑块就跳到那一格）⇒ 一次**单击**也是一次可撤销的编辑。
            // `start_db` 仍是按下那一刻的工程值（提交的 `old_gain_db`）。
            let pressed_db = db_from_fraction(fraction, min_db, max_db);
            let moved = pressed_db.to_bits() != current_db.unwrap_or(0.0).to_bits();
            *gesture.borrow_mut() = SendGainGesture::Dragging {
                edge_id,
                start_db: current_db,
                last_db: pressed_db,
                min_db,
                max_db,
                moved,
            };
            if moved && let Some(ui) = weak.upgrade() {
                preview_send_gain(&ui, edge_id, pressed_db);
            }
        }
    });
    ui.on_send_gain_drag({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        move |_edge_id: SharedString, fraction: f32| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            // ⭐ `#212`：借的作用域只包住状态本身；**先放借，再写界面**
            // （`preview_send_gain` 会 `ui.set_*`）。持借调界面 = 重入 panic 的形态。
            let (edge_id, next_db) = {
                let mut state = gesture.borrow_mut();
                let SendGainGesture::Dragging {
                    edge_id,
                    last_db,
                    min_db,
                    max_db,
                    moved,
                    ..
                } = &mut *state
                else {
                    return;
                };
                let next_db = db_from_fraction(fraction, *min_db, *max_db);
                if next_db.to_bits() == last_db.to_bits() {
                    return;
                }
                *last_db = next_db;
                *moved = true;
                (*edge_id, next_db)
            };
            // 拖动期只写**视图态**（下一次完整重投影会覆盖它）—— ⛔ 不提交工程内容。
            preview_send_gain(&ui, edge_id, next_db);
        }
    });
    ui.on_send_gain_release({
        let weak = weak.clone();
        let gesture = Rc::clone(&gesture);
        let port = Rc::clone(port);
        move |_edge_id: SharedString| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let taken = std::mem::replace(&mut *gesture.borrow_mut(), SendGainGesture::Idle);
            let SendGainGesture::Dragging {
                edge_id,
                start_db,
                last_db,
                moved,
                ..
            } = taken
            else {
                return;
            };
            if !moved || last_db.to_bits() == start_db.unwrap_or(0.0).to_bits() {
                return;
            }
            let Some(project) = port.try_project() else {
                eprintln!("[yeban-app] 发送增益提交：权威会话此刻没有活跃工程");
                refresh_undo_window(&ui, &port, true);
                return;
            };
            // ⭐ `#192`：松手侧重新解析"选中轨 or 兜底"，与投影同一份口径。
            let Some((current_db, _, _)) =
                crate::bridge::send_gain_target_on(&project, display_track(&ui, &project), edge_id)
            else {
                // 手势期间边被删掉 / 展示轨换了 ⇒ 目标已不存在，⛔ 不按一个过期基线提交。
                refresh_undo_window(&ui, &port, true);
                return;
            };
            // 逐位比对 `Option<f32>`：起点必须仍是当前值（陈旧基线 ⇒ 放弃提交）。
            if current_db.map(f32::to_bits) != start_db.map(f32::to_bits) {
                eprintln!("[yeban-app] 发送增益提交被放弃：拖动期间工程值已变（陈旧基线）");
                refresh_undo_window(&ui, &port, true);
                return;
            }
            let op = yeban_model::Op::SetRoutingGain {
                edge_id,
                old_gain_db: start_db,
                new_gain_db: Some(last_db),
            };
            if port
                .commit_ops(now_ms(), "set send gain", vec![op])
                .is_err()
            {
                eprintln!("[yeban-app] 发送增益提交被拒绝（工程未改）");
            }
            // 成功与失败都重投影一次：成功 ⇒ 读新值；失败 ⇒ 把拖动期的预览值抹掉。
            refresh_undo_window(&ui, &port, true);
        }
    });

    // ---------------------------------------------------------------- ② 新建发送
    ui.on_send_create({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Some(project) = port.try_project() else {
                eprintln!("[yeban-app] 新建发送：权威会话此刻没有活跃工程");
                return;
            };
            // ⭐ 任务单 `#160`：可用性（含**防重复**）与投影**同一份**口径
            // （`bridge::send_create_availability_on`）。⛔ 不在这里写第二份判定。
            // ⭐ `#192`：主语 = 选中轨 or 兜底（与发送条 / 设备架同一条）。
            let track = display_track(&ui, &project);
            let (available, reason) = crate::bridge::send_create_availability_on(&project, track);
            if !available {
                eprintln!("[yeban-app] 新建发送被拒绝：{reason}");
                refresh_undo_window(&ui, &port, true);
                return;
            }
            let Some(track) = track else {
                eprintln!("[yeban-app] 新建发送被拒绝：没有可显示的音轨");
                return;
            };
            let Some(aux) = crate::bridge::send_aux_target(&project) else {
                eprintln!("[yeban-app] 新建发送被拒绝：没有辅助返回轨（AuxReturn）");
                refresh_undo_window(&ui, &port, true);
                return;
            };
            let source = track.id;
            let destination = aux.id;
            // 新边身份来自项目既有的分配器；`EntityId::new()` 是 `ulid::Ulid::generate()`
            // 的唯一包装（见 `yeban-model/src/ids.rs` 的规范漂移记录）。
            let edge = yeban_model::RoutingEdge {
                id: yeban_model::EntityId::new(),
                source_node: source,
                destination_node: destination,
                kind: yeban_model::RoutingKind::SendToAux,
                gain_db: None,
            };
            // `ConnectRouting` 的前置条件是"两端已在 `routing_graph.nodes` 里"。
            // 端点缺失时在**同一次** `commit_ops` 里先补节点（`Op::Batch` 顺序施加）
            // ⇒ 仍是恰好一次提交；不假设所有工程都已经登记过节点。
            let mut ops = Vec::new();
            for node in [source, destination] {
                if !project.routing_graph.nodes.contains(&node) {
                    ops.push(yeban_model::Op::AddRoutingNode { node });
                }
            }
            ops.push(yeban_model::Op::ConnectRouting { edge });
            if port.commit_ops(now_ms(), "create send", ops).is_err() {
                eprintln!("[yeban-app] 新建发送被拒绝（工程未改）");
            }
            refresh_undo_window(&ui, &port, true);
        }
    });

    // ---------------------------------------------------------------- ③ 删除发送（⭐ 任务单 `#160`）
    //
    // `#159` 登记过"没删发送"；这里是那一条的落点。一次点击 = **恰好一次**
    // `commit_ops`（一个 `Op::DisconnectRouting`）⇒ 一步可撤销的编辑。
    ui.on_send_remove({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move |edge_id: SharedString| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Ok(edge_id) = edge_id.parse::<yeban_model::EntityId>() else {
                eprintln!("[yeban-app] 删除发送被拒绝：边身份不是合法的 ULID 文本");
                return;
            };
            let Some(project) = port.try_project() else {
                eprintln!("[yeban-app] 删除发送：权威会话此刻没有活跃工程");
                return;
            };
            // 权重检查与增益手势**同一条**：边必须存在、类型是 `SendToAux`、源是展示轨。
            // ⛔ 不信界面上的行下标。⭐ `#192`：源 = 选中轨 or 兜底。
            let Some(track) = display_track(&ui, &project) else {
                eprintln!("[yeban-app] 删除发送被拒绝：没有可显示的音轨");
                return;
            };
            let Some(edge) = project.routing_graph.edges.get(&edge_id).copied() else {
                eprintln!("[yeban-app] 删除发送被拒绝：那条路由边已经不存在");
                refresh_undo_window(&ui, &port, true);
                return;
            };
            if edge.kind != yeban_model::RoutingKind::SendToAux || edge.source_node != track.id {
                eprintln!("[yeban-app] 删除发送被拒绝：不是展示轨的发送边");
                refresh_undo_window(&ui, &port, true);
                return;
            }
            let op = yeban_model::Op::DisconnectRouting {
                edge_id,
                previous_edge: edge,
            };
            if port.commit_ops(now_ms(), "remove send", vec![op]).is_err() {
                eprintln!("[yeban-app] 删除发送被拒绝（工程未改）");
            }
            refresh_undo_window(&ui, &port, true);
        }
    });
}

// ===========================================================================
// 轨 / 路由（控制台 Tab 3 的同一面板，任务单 `#160`）
// ===========================================================================

/// 轨 / 路由段（控制台 **Tab 3** 里与发送条**同一面板**）的**写入面**。
///
/// 四个手势，每个 = **恰好一次** [`UndoPort::commit_ops`]：
///
/// | 手势 | `.slint` 回调 | 提交的 `Op`（一次 `Batch`） | 撤销标签 |
/// | :--- | :--- | :--- | :--- |
/// | 新建 MIDI 轨 | `track-create-midi()` | `AddTrack` ＋ `AddRoutingNode` ＋ `ConnectRouting{TrackToBus}` | `"create track"` |
/// | 新建音频轨 | `track-create-audio()` | 同上 | `"create track"` |
/// | 新建辅助返回轨 | `track-create-aux()` | 同上，但边的 `kind` 是 `BusToMaster` | `"create track"` |
/// | 删除展示轨 | `track-delete()` | `DisconnectRouting`×n ＋ `RemoveRoutingNode` ＋ `RemoveTrack` | `"delete track"` |
///
/// ## 目标与规则（全部来自权威工程，⛔ 不信界面）
///
/// - 建轨的 `Op` 批次由 [`crate::bridge::track_create_ops`] 构造：新轨身份来自项目既有的
///   分配器 [`yeban_model::EntityId::new`]，并且**同时**给它一条到主总线的显式路由边；
/// - 删轨的目标是**面板服务的那条轨**（⭐ `#192`：选中轨 or 兜底，`host::display_track`）
///   ⇒ 主总线**永远**不是目标；只剩主总线时按钮禁用并逐字写出
///   [`crate::bridge::TRACK_DELETE_MASTER_REASON`]；直接调用回调也会被
///   [`crate::bridge::track_delete_ops_on`] 拒掉（Δ0 提交）；
/// - 没有主总线的 0 轨空工程 ⇒ 建轨被 [`crate::bridge::TRACK_CREATE_NO_MASTER_REASON`]
///   拒绝（`Op` 全集写不了 `master_bus_track_id`，写下去就是一份非法文档）。
///
/// ## 它**不**做什么
///
/// ⛔ 不改音量 / 声相 / 静音 / 独奏（那是 `wire_mixer_edit`）；⛔ 不碰引擎；
/// ⛔ 不猜轨：展示轨由 `host::display_track` **唯一**解析（⭐ `#192` 起 = 选中轨 or 兜底；
/// 原文住在窗口属性 `selected-track-id`，⛔ 不是工程内容）。
pub fn wire_track_strip(ui: &MainWindow, port: &Rc<UndoPort>) {
    let weak = ui.as_weak();

    // ---------------------------------------------------------------- ① 新建轨
    ui.on_track_create_midi({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            create_track(&ui, &port, crate::bridge::NewTrackKind::Midi);
        }
    });
    ui.on_track_create_audio({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            create_track(&ui, &port, crate::bridge::NewTrackKind::Audio);
        }
    });
    ui.on_track_create_aux({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            create_track(&ui, &port, crate::bridge::NewTrackKind::AuxReturn);
        }
    });

    // ---------------------------------------------------------------- ② 删除展示轨
    ui.on_track_delete({
        let weak = weak.clone();
        let port = Rc::clone(port);
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let Some(project) = port.try_project() else {
                eprintln!("[yeban-app] 删除轨：权威会话此刻没有活跃工程");
                return;
            };
            // ⭐ `#192`：删的是**选中轨 or 兜底**（界面按钮的可用性也由同一次解析给出）。
            let ops =
                match crate::bridge::track_delete_ops_on(&project, display_track(&ui, &project)) {
                    Ok(ops) => ops,
                    Err(reason) => {
                        // 主总线 / 没有展示轨：**逐字**原因进日志（界面另有一份同样的原因）。
                        eprintln!("[yeban-app] 删除轨被拒绝：{reason}");
                        refresh_undo_window(&ui, &port, true);
                        return;
                    }
                };
            if port.commit_ops(now_ms(), "delete track", ops).is_err() {
                eprintln!("[yeban-app] 删除轨被拒绝（工程未改）");
            }
            refresh_undo_window(&ui, &port, true);
        }
    });
}

/// 建轨手势的唯一实现体（三个按钮共用；`kind` 是唯一的自变量）。
fn create_track(ui: &MainWindow, port: &UndoPort, kind: crate::bridge::NewTrackKind) {
    let Some(project) = port.try_project() else {
        eprintln!("[yeban-app] 新建轨：权威会话此刻没有活跃工程");
        return;
    };
    let ops = match crate::bridge::track_create_ops(&project, kind) {
        Ok(ops) => ops,
        Err(reason) => {
            eprintln!("[yeban-app] 新建轨被拒绝：{reason}");
            refresh_undo_window(ui, port, true);
            return;
        }
    };
    if port.commit_ops(now_ms(), "create track", ops).is_err() {
        eprintln!("[yeban-app] 新建轨被拒绝（工程未改）");
    }
    refresh_undo_window(ui, port, true);
}

/// 发送增益手势的中间态（**会话运行态**，不进 `.yeban`）。
///
/// 与 [`DeviceEqGesture`] 同一条纪律：`Idle` 是唯一的空态；`cancel` 也走上报的
/// `send-gain-release`，因此不存在"黏住的拖动"。`start_db` 是**提交时的
/// `old_gain_db`**（`Option` 的原样，保留"单位增益 ≠ 0 dB"），`last_db` 是收尾要提交的
/// `new_gain_db`。
#[derive(Clone, Copy, Debug, PartialEq)]
enum SendGainGesture {
    /// 没有手势。
    Idle,
    /// 正在拖某一条发送边。
    Dragging {
        /// 被拖的边身份（三个回调的寻址单位）。
        edge_id: yeban_model::EntityId,
        /// 按下时的工程值（`old_gain_db`；`None` = 单位增益）。
        start_db: Option<f32>,
        /// 最近一次算出的值（`new_gain_db` 的 `Some` 部分）。
        last_db: f32,
        /// 值域下界 (dB)，来自模型。
        min_db: f32,
        /// 值域上界 (dB)，来自模型。
        max_db: f32,
        /// 是否收到过至少一次**改变数值**的 move / 按下。
        moved: bool,
    },
}

/// 拖动期的**视觉预览**：只改 `send-gain-dbs` 里那一格，⛔ 不碰工程。
///
/// 行下标由边身份现查（`send-edge-ids` 是同一份投影的数组）⇒ 即便投影在拖动期间被
/// 换掉、下标漂移，也不会把预览写到另一条发送上（找不到就不写）。
fn preview_send_gain(ui: &MainWindow, edge_id: yeban_model::EntityId, value_db: f32) {
    let edge_id = edge_id.to_canonical_string();
    let ids = read_strings(&ui.get_send_edge_ids());
    let Some(index) = ids.iter().position(|id| *id == edge_id) else {
        return;
    };
    let mut gains = read_lengths(&ui.get_send_gain_dbs());
    let Some(slot) = gains.get_mut(index) else {
        return;
    };
    *slot = value_db;
    ui.set_send_gain_dbs(lengths(&gains));
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

// ===========================================================================
// 生产心跳：引擎（增量发布 ＋ 回收）与电平（抽干 ＋ 注入）的**唯一**一跳
// ===========================================================================

/// 一次**生产心跳**的完整读数（`src/main.rs` 的 16 ms 定时器与判据**共用**这一处）。
///
/// 为什么把三样东西捆成一个返回值：这一跳的三件事有**顺序契约**
/// （发布 ⇒ 回收 ⇒ 抽电平），而"哪一跳真的做了哪几件"正是判据要读的字面读数。
#[derive(Debug)]
pub struct ProductionTick {
    /// 这一跳增量发布的快照版本号；`None` = 没发布（标记没变 / 没有工程可发 / 发布失败）。
    pub published_revision: Option<u64>,
    /// ⭐ **这一跳发布失败**的错误原文（`EngineHostError` 的 `Display` 原文）；任务单 `#86` 甲。
    ///
    /// 与 `crate::live_surface::EngineTick::publish_error` **同形同义**（两份执行面，
    /// 同一个读数形状）：`Some(text)` = 发布了但失败 ⇒ 播放侧仍是**上一版**快照。
    ///
    /// ⚠ **本字段是 `#86` 甲补上的那一格**：在这之前，本函数把发布错误只写进
    /// `eprintln!`，`ProductionTick` 里**没有**它 ⇒ 生产窗口读不到，"点了 SET 一个坏路径
    /// 而界面毫无反应"在界面上不可见（用户以为"没生效"）。现在它既进这个读数
    /// **又**经 [`apply_engine_error`] 投到界面（错误横幅）。
    pub publish_error: Option<String>,
    /// 这一跳的回收读数；`None` = **没有引擎**（这一跳一位没动）。
    pub readings: Option<HeartbeatReadings>,
    /// 电平腿的结局。
    pub meters: MeterPump,
}

/// ⭐ **引擎发布腿的一次结局** ⇒ [`apply_engine_error`] 的唯一输入（任务单 `#86` 甲）。
///
/// 为什么是三档而不是 `Option<&str>`：三种结局在界面上必须是**三件不同的事**
/// （与 `EngineTick::publish_error` 的表格同一口径，⛔ 不许压成一个 `None`）：
///
/// | 值 | 界面上的动作 | 为什么 |
/// | :--- | :--- | :--- |
/// | [`Published`](Self::Published) | **清掉**横幅 | 这一跳真的换了一份新快照 ⇒ 上一次的错误已经作废（"成功时消失"） |
/// | [`Unchanged`](Self::Unchanged) | **一位不动** | 没有发布（标记没变 / 没有工程可发）⇒ 横幅仍代表"**最近一次**"错误，⛔ 不能因为"这一跳什么都没发生"就清掉它 |
/// | [`Failed`](Self::Failed) | **显示**这条原文 | 发布了但失败 ⇒ 用户必须看见 |
///
/// ⚠ 把 `Unchanged` 也当成"成功清屏"是**最容易写错**的一档：心跳每秒跑 60 次，而
/// 发布失败之后标记通常**不再变**（用户没再改），于是下一跳就是 `Unchanged`
/// ⇒ 那样写会让横幅**闪一下就走**，等于没做。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnginePublishOutcome<'a> {
    /// 这一跳真的发布了一份新快照。
    Published,
    /// 这一跳没有发布（标记没变 / 没有工程可发 / 没有引擎）。
    Unchanged,
    /// 这一跳发布了但失败（`EngineHostError` 的 `Display` 原文）。
    Failed(&'a str),
}

/// **「引擎错误横幅」的唯一写者**（任务单 `#86` 甲）—— 全仓只有这一处写
/// `MainWindow.engine-error`。
///
/// ## 它与既有投影通道的关系（⚠ 本单**没有**造第二条通道）
///
/// 本仓库"引擎/宿主状态 ⇒ 界面"的既有投影通道有**三条**，本函数是第四条的**同款**：
///
/// | 通道 | 写者 | 数据源 | 目标属性 |
/// | :--- | :--- | :--- | :--- |
/// | 电平 | [`pump_meters`] → [`apply_meters`] | `MeterRuntime`（引擎 SPSC 消费端） | `track-meter-*` |
/// | 走带 | [`apply_transport`] | `EngineHost::transport()`（RT→UI 原子镜面） | `playing` / `timecode` |
/// | 保存 | [`apply_save_outcome`] | `save_action::SaveOutcome` | `save-status` / `save-succeeded` |
/// | ⭐ **发布失败** | ⭐ **本函数** | ⭐ `EngineHost::publish_project` 的 `Err` | ⭐ `engine-error` |
///
/// 共同的纪律（四条一模一样）：**宿主是唯一写者**、`.slint` 只显示不算、界面属性是
/// **原文**而不是一个压扁的 `bool`、失败与成功走**同一条**写路径（于是"失败时忘了刷新
/// 界面"这种错法不存在）。
///
/// ⚠ 本函数**不进音频线程**（调用点只有生产 60Hz 心跳与无头执行面两处），因此允许
/// 分配（`SharedString` 那一次）。
///
/// ## 为什么图标 / tooltip / 可关闭的条都没做（如实登记）
///
/// 本单选了**最小**的形状：一条可见文本 ＋ 一个稳定的语义 ID。理由是可判性 ——
/// `DoD §3.6` 要的是"无头控件树 JSON 里能按 ID 寻址到**内容**"，而图标与 tooltip 在
/// `accessible-value` 上给不出比原文更多的东西；"可关闭"会引入**第二个**状态
/// （关掉之后错误还在、横幅没了 ⇒ 那正是本仓最忌讳的"用户以为没事了"）。
pub fn apply_engine_error(ui: &MainWindow, outcome: EnginePublishOutcome<'_>) {
    match outcome {
        // 成功 ⇒ 横幅消失。空串是 `.slint` 侧 `visible` 的**唯一**判据（⛔ 不另设 bool：
        // 两个属性就有"文字还在、bool 说没有"这种自相矛盾的中间态）。
        EnginePublishOutcome::Published => ui.set_engine_error(slint::SharedString::new()),
        // 没有发布 ⇒ 一位不动（见 [`EnginePublishOutcome::Unchanged`] 的说明）。
        EnginePublishOutcome::Unchanged => {}
        EnginePublishOutcome::Failed(text) => ui.set_engine_error(text.into()),
    }
}

/// 电平腿（[`pump_meters`]）的三种结局 —— 三种都能被调用方与判据**分开**读。
///
/// 用枚举而不是 `Option<MeterSnapshot>`：`None` 会把"没有引擎"与"长度契约不成立"
/// 混成同一件事，而这两种情况的处置完全不同（一个什么都不做，一个只抽干不注入）。
#[derive(Debug)]
pub enum MeterPump {
    /// 没有引擎（`reload` 从未成功过）⇒ 一位没写，电平表停在 [`apply_view`] 写的静音值。
    NoEngine,
    /// 这一轮的电平已经写进界面（经由 [`apply_meters`]）。
    Applied(MeterSnapshot),
    /// 界面**此刻**的轨道数与手里那份投影**不等长** ⇒ 只抽干、**不注入**。
    ///
    /// 造出这种状态的只有一件事：**控制面换了工程**（`yeban_open_project` 换掉轨道集合，
    /// `crate::reproject::AuthorityMirror` 随后用新投影调 [`apply_view`]）。界面自己是对的
    /// ——`apply_view` 已经按新工程把电平数组重置成静音——但持有本循环的那一段代码手里
    /// 的 `view` 还是旧的那一份。拿它注入会让 [`apply_meters`] 的"等长"契约失效
    /// （`.slint` 按下标取 ⇒ A 轨的电平画到 B 轨上）。
    ///
    /// 因此这一跳**仍然抽干队列**（音频侧不该因为界面换了工程而积压/丢帧），
    /// 只是不注入。两个长度如实回报，让调用方能出声。
    LengthMismatch {
        /// 界面此刻 `track-names` 的长度。
        rows: usize,
        /// 调用方手里那份投影的轨道数。
        projected: usize,
    },
}

/// GUI 主线程的**生产心跳**：引擎与电平两条腿的装配，以及逐跳驱动。
///
/// ## 它补的是哪一个洞（本票的核心，实测）
///
/// 在它之前，`src/main.rs` 的 `run_gui` 写的是
/// `if let Err(error) = engine.reload(&project, 0) { … }` —— 于是 `Ok` 那一侧
/// **整个被丢掉**，而 `Ok` 装的 [`crate::engine_host::EngineRebuild`] 里就有
/// [`crate::engine_host::EngineRebuild::collector`]（新引擎的电平消费端，
/// `engine_host.rs:289` 的注释写着"**UI 线程必须采纳它**"）。
/// 后果有两条，都在生产窗口里：引擎侧那条 SPSC **没有消费者**（满则丢最新帧），
/// 而界面上的电平表永远停在 `apply_view` 写下的静音值。
///
/// 把"建引擎 ＋ 采纳消费端 ＋ 每一跳"收进**这一个**类型之后，产品路径与判据走的是
/// 同一段代码：在 `run_gui` 里"丢掉消费端"这件事**已经无法表达**（它拿不到 collector）。
///
/// ## 线程
///
/// 全部方法都跑在 **UI / 主线程**上（`slint::Timer` 的回调线程就是建这一代引擎的线程）。
/// 因此 [`crate::engine_host::EngineHost::heartbeat`] 的
/// `RetireQueue::release_thread_is_main()` 保持为真，而音频回调那条读路径
/// （`SnapshotReader::begin_block`）一位没动 `[ARCH-RT-001 / MUST-GATE-001]`。
pub struct ProductionLoop {
    /// 引擎宿主。`run_gui` 还要把它交给走带回调（[`wire_transport`]），
    /// 因此这里存 `Rc<RefCell<..>>` 而不是裸值 —— [`Self::engine_handle`] 交出**同一个**实例。
    engine: Rc<RefCell<EngineHost>>,
    /// UI 线程持有的电平消费端（[`crate::meters::MeterRuntime`]）。
    meters: MeterRuntime,
}

impl ProductionLoop {
    /// **生产方式**：真的建一代引擎，并把 `reload` 交出的电平**消费端**采纳进来。
    ///
    /// 这是电平腿的**唯一**开关。`EngineRebuild::collector` 是**所有权**
    /// （消费端只能被采纳一次），因此"丢掉它"在这里等价于删掉下面那一行 ——
    /// 而删掉之后 [`Self::tick`] 的 `MeterPump` 会一直停在"没有读数"
    /// （判据 `production_tick_drains_the_engine_meter_queue_and_advances_the_quantum`
    /// 因此变红）。
    ///
    /// # Errors
    ///
    /// [`EngineHostError`]：工程投影不成快照（没有主总线 / 路由图成环 / 模型校验失败）。
    /// 这时调用方改用 [`Self::without_engine`] —— 界面照常打开，与接电平之前**一位不变**。
    pub fn start(
        project: &yeban_model::YebanProjectV1,
        quanta: u64,
    ) -> Result<Self, EngineHostError> {
        Self::start_with_audio_source(project, quanta, None)
    }

    /// ⭐ **生产方式 ＋ 音频片段字节来源**（任务单 `#136`）：与 [`Self::start`] 逐字相同，
    /// 但**先**把来源交给 [`EngineHost::set_audio_asset_source`] **再** `reload`。
    ///
    /// 顺序是契约：`reload` 的第 1.3 步要**当场**给快照注入 `ClipContent::Audio` 的 PCM
    /// （PCM 不在工程模型里）⇒ 来源晚一步接上，第一份快照就是静音的（要等一次编辑发布
    /// 才补上）。产品 `src/main.rs` 因此走本函数，来源 = 会话 CAS 池
    /// （`UndoPort::asset_bytes`）—— 录音（`#132`）与导入落进那一池的**真实字节**。
    ///
    /// `None` = 不接来源（与 [`Self::start`] 等价）：工程里的音频片段会被逐条记为
    /// "跳过"（可见，⛔ 不静默），输出与接线前逐位相同。
    ///
    /// # Errors
    ///
    /// 与 [`Self::start`] 相同。
    pub fn start_with_audio_source(
        project: &yeban_model::YebanProjectV1,
        quanta: u64,
        audio_source: Option<crate::audio_inject::AudioAssetSource>,
    ) -> Result<Self, EngineHostError> {
        Self::start_with_audio_source_and_project_dir(project, quanta, audio_source, None)
    }

    /// ⭐ **生产方式 ＋ 音频片段字节来源 ＋ 工程文件所在目录**（任务单 `#168`）：
    /// 与 [`Self::start_with_audio_source`] 逐字相同，但**先**把工程目录交给
    /// [`EngineHost::set_project_dir`] **再** `reload`。
    ///
    /// 顺序同样是契约：第一份快照的 SFZ 摊平发生在 `reload` 里 ⇒ 目录晚一步写，
    /// 那一份快照就会按"仓库根 ⇒ cwd"去解析工程旁边的样本（很可能找不到 ⇒ 响亮失败）。
    ///
    /// `project_dir = None` ⇒ 与 [`Self::start_with_audio_source`] **逐位相同**
    /// （工程不是从文件打开的：内置样本 / 判据装配）。
    ///
    /// # Errors
    ///
    /// 与 [`Self::start`] 相同（外加相对 SFZ 路径找不到时的 [`EngineHostError::Sfz`]）。
    pub fn start_with_audio_source_and_project_dir(
        project: &yeban_model::YebanProjectV1,
        quanta: u64,
        audio_source: Option<crate::audio_inject::AudioAssetSource>,
        project_dir: Option<&std::path::Path>,
    ) -> Result<Self, EngineHostError> {
        let mut engine = EngineHost::new();
        if let Some(source) = audio_source {
            engine.set_audio_asset_source(source);
        }
        // ⭐ `#168`：工程目录必须在 `reload` **之前**写下（见函数文档的顺序契约）。
        engine.set_project_dir(project_dir.map(std::path::Path::to_path_buf));
        let rebuild = engine.reload(project, quanta)?;
        let mut meters = MeterRuntime::empty();
        // ⛔ 本票修的那一处。`adopt` 的语义是"引擎换代 ⇒ 旧读数全部作废"
        // （见 `crate::meters::MeterRuntime::adopt`），它同时把消费端的所有权接过来。
        meters.adopt(rebuild.collector);
        Ok(Self {
            engine: Rc::new(RefCell::new(engine)),
            meters,
        })
    }

    /// **没有引擎**的形态（`reload` 返回 `Err` 时用它）。
    ///
    /// 语义与接电平之前**逐字相同**：窗口照常开、走带回调明确报告"没有引擎"、
    /// 电平表停在 [`apply_view`] 写的静音值（[`MeterPump::NoEngine`]）。
    #[must_use]
    pub fn without_engine() -> Self {
        Self {
            engine: Rc::new(RefCell::new(EngineHost::new())),
            meters: MeterRuntime::empty(),
        }
    }

    /// **设备腿**：把这一代引擎真的交给声卡（[`EngineHost::open_device`]）。
    ///
    /// 与 [`Self::start`] 同一条纪律：`EngineHost::open_device` 交出的电平**消费端**
    /// 是所有权，必须在这里被采纳 —— 否则设备腿发布的电平没有人抽，界面上的电平表
    /// 停在最后一次控制驱动的读数上（那是**假绿**：表在动，但动的是旧驱动）。
    ///
    /// 失败**不改**本循环的任何状态（`EngineHost::open_device` 的 1–4 步都不动手里那一代）
    /// ⇒ 调用方拿到 `Err` 后产品仍然按控制驱动形态工作。
    ///
    /// # Errors
    ///
    /// [`EngineHostError::NoEngine`]（还没 `start` 过）或 [`EngineHostError::Device`]
    /// （设备不存在 / 采样率不支持 / 格式不是 `f32` / 独占模式 / 后端错误）。
    pub fn open_device(
        &mut self,
        config: yeban_engine::device::EngineConfig,
    ) -> Result<DeviceOpening, EngineHostError> {
        let (opening, collector) = self.engine.borrow_mut().open_device(config)?;
        self.meters.adopt(collector);
        Ok(opening)
    }

    /// 引擎在这个进程里活没活着（`reload` 成功过）。
    #[must_use]
    pub fn has_engine(&self) -> bool {
        self.engine.borrow().has_engine()
    }

    /// 引擎宿主句柄 —— 与 [`wire_transport`] 共用**同一个** `EngineHost`
    /// （`Rc` 计数 +1，不是复制一份宿主）。
    #[must_use]
    pub fn engine_handle(&self) -> Rc<RefCell<EngineHost>> {
        Rc::clone(&self.engine)
    }

    /// UI 线程持有的电平消费端（判据读它的面板与计数器）。
    #[must_use]
    pub const fn meters(&self) -> &MeterRuntime {
        &self.meters
    }

    /// 电平消费端的**可写**入口。
    ///
    /// 存在的理由与 [`crate::meters::MeterRuntime::adopt`] 相同：引擎换代（`ui/reload_engine`
    /// 那一类动作）会把**新**的消费端交给 UI 线程，而持有本循环的那一侧是唯一的落点。
    /// 生产路径上没有任何调用者（生产只有 [`Self::start`] 一处装配）。
    pub const fn meters_mut(&mut self) -> &mut MeterRuntime {
        &mut self.meters
    }

    /// 这一跳要不要读工程（`true` = 标记变了）。
    ///
    /// 单独暴露它，是为了让调用方在**克隆工程之前**问这一句：
    /// 生产路径传进来的是 `UndoPort::try_project()`（整份工程的克隆），
    /// "没改就不克隆"是 [`Self::tick`] 的成本契约。
    #[must_use]
    pub fn should_publish(&self, mark: EditMark) -> bool {
        self.engine.borrow().should_publish(mark)
    }

    /// **一跳**（`run_gui` 的 16 ms 定时器体）。
    ///
    /// 顺序是契约，四步在**同一个** tick 上：
    ///
    /// 1. 标记变了且有工程可发 ⇒ [`EngineHost::publish_project`] **增量**发布
    ///    （只换快照；**不用** `reload`，那会改掉 `ui/reload_engine` 的 `quanta` 契约）；
    /// 2. 不论改没改 ⇒ [`EngineHost::heartbeat`]：`SnapshotSlot::prune` ＋ `RetireQueue::drain`
    ///    （⛔ 不做 ② ⇒ ①就是一条"只涨不回收"的慢泄漏）；
    /// 3. 不论改没改 ⇒ [`pump_meters`]：抽干 SPSC → 对齐投影 → 写 Slint 属性；
    /// 4. 不论改没改 ⇒ 把 [`EngineHost::transport`] 的读数交给 [`apply_transport`]。
    ///    它是 `playing` 与 `timecode` 的**唯一**写者（任务单 `#228`）⇒ ⛔ 少了这一跳，
    ///    引擎在播而界面上的数字**一位不动**（用户以为卡了）。停止时同样写：引擎的
    ///    `Stop` 是暂停语义（位置保留）⇒ 停住后的显示与引擎 `position_ticks` 一致。
    ///
    /// ## 为什么电平与发布**共用**这一个 tick
    ///
    /// - `[ARCH-UI-002]` 要的就是"UI 线程定时器"这一条腿，两者的目标频率同为 60Hz；
    /// - 共用一跳给出一个**顺序保证**：电平永远在"快照已经换过"之后被抽。
    ///   两个定时器要做到同一件事，得再论证一次两者的先后；
    /// - 成本上没有拆的理由：标记没变时 ① 是一次 `u64` 比较，② 是两条队列的长度检查，
    ///   ③ 是一次 SPSC 抽干（空队列时零拷贝），④ 是一次走带镜面读 ＋ 两个标量属性
    ///   （`playing` / `timecode`，⛔ 不换任何数组模型实例）。四件都是 O(1)～O(轨道数)，
    ///   且都不分配（分配只发生在 `MeterRuntime::poll` 第一次扩容搬运缓冲时，那也在 UI
    ///   线程上；`timecode` 的格式化是每跳一次的小字符串，见 `apply_transport`）。
    ///
    /// 没有引擎 ⇒ **一位不动**（不发布、不回收、不注入），与接电平之前逐字相同。
    #[must_use]
    pub fn tick(
        &mut self,
        ui: &MainWindow,
        view: &ViewState,
        mark: EditMark,
        project: Option<&yeban_model::YebanProjectV1>,
    ) -> ProductionTick {
        // ⭐ `#212`：**借只包住引擎调用**，界面写入（[`apply_engine_error`] /
        // [`pump_meters`]）一律在 `drop(engine)` 之后 ⇒ 界面回调（若将来有）不可能
        // 重入同一份 `RefCell`。持借调界面正是 `#212` 崩溃的形态。
        let mut engine = self.engine.borrow_mut();
        if !engine.has_engine() {
            drop(engine);
            // 没有引擎 ⇒ 这一跳"没有发布" ⇒ 横幅**一位不动**（⛔ 不因为"引擎没起来"
            // 就清掉上一次的错误：那不是"错误已解决"的证据）。见 [`EnginePublishOutcome`]。
            apply_engine_error(ui, EnginePublishOutcome::Unchanged);
            return ProductionTick {
                published_revision: None,
                publish_error: None,
                readings: None,
                meters: MeterPump::NoEngine,
            };
        }
        let mut published_revision = None;
        let mut publish_error = None;
        if engine.should_publish(mark)
            && let Some(project) = project
        {
            match engine.publish_project(project, mark) {
                Ok(revision) => published_revision = Some(revision),
                // 投影失败**不改播放侧**（旧快照原样保留）⇒ 如实出声，不静默。
                Err(error) => {
                    eprintln!("[yeban-app] 快照发布失败（播放侧仍是上一版）: {error}");
                    // ⭐ `#86` 甲：保留原文（⛔ 不是只 `eprintln!`）⇒ 控制面能读、
                    // 下面经 [`apply_engine_error`] 投到界面。
                    publish_error = Some(error.to_string());
                }
            }
        }
        let readings = engine.heartbeat();
        // **④ R55：自动化泳道 → `SetParam`**（与前三步同一个 tick、同一个 UI 线程）。
        //
        // 为什么在这里、而不是在 `EngineHost` 内部：采样必须是**每一跳**都发生的事，
        // 而它需要工程；工程的克隆按成本契约只在"标记变了"的那一跳做 ⇒ 由 `EngineHost`
        // 缓存最近一份（见下一段的 R68 说明）。
        //
        // 采样时点 = `EngineHost::transport().position_ticks`（RT→UI 镜面 = **音频时钟**）
        // ⇒ 同一个工程在同一条音频时间线上给出同一批值（⛔ 没有任何墙钟输入）；
        // 每个 tick 至多一批 `SetParam`（≤ `PARAM_SLOTS` 条）。
        // 这一步**不**碰音频线程：值走既有无锁 SPSC，平滑仍由引擎侧既有的
        // `yeban_engine::param` 目标表（τ ≈ 5 ms）完成 ⇒ `ParamTable` 一字未动。
        //
        // 60 Hz 够不够：τ = 5 ms ⇒ 16.67 ms 心跳 = 3.33 τ ⇒ 每步收敛 ≈96.4 %；
        // 保真上限 ≈10–15 Hz（够推子/渐强，不够音频速率调制）。
        //
        // ⚠ **R68**：把**入参原样**传进去，⛔ 不要写 `if let Some(p) { … }` —— 生产循环
        // （`src/main.rs` 的 16 ms 定时器）只在"标记变了"的那一跳 `try_project()`，
        // 其余各跳传的是 `None`；`EngineHost` 缓存最近一份工程 ⇒ 采样**每一跳都进行**。
        // 在这里判 `Some` 会让自动化只在工程改动的那一跳生效（静默丢失）。
        engine.publish_automation(project);
        // ⭐ `#228`：**走带读数**（`Copy` 的一帧，一次原子镜面读）。
        // ⛔ 在借内只读、不写界面：界面写入一律在 `drop(engine)` 之后（`#212`）。
        let transport = engine.transport();
        // ⭐ `#86` 甲：**引擎发布腿 ⇒ 界面**的投影点（与上面三步同一个 tick、同一个 UI 线程）。
        //
        // ⛔ 顺序：必须在 `publish_project` 之后 —— 它读的就是那一步的结局。
        // ⛔ 它**不是**第 5 个"必做步骤"：它只写一个 Slint 属性（无 I/O、无分配上限、
        //   不阻塞），即使失败也不影响前三步的读数。
        drop(engine);
        apply_engine_error(
            ui,
            match (&publish_error, published_revision) {
                (Some(error), _) => EnginePublishOutcome::Failed(error),
                (None, Some(_)) => EnginePublishOutcome::Published,
                (None, None) => EnginePublishOutcome::Unchanged,
            },
        );
        // ⭐ `#228`：**播放推进期间时间码跟着走**。读数是上面在借内取好的
        // （`drop(engine)` 之后才写界面 ⇒ ⛔ 不重新引入持借写界面，`#212` 的崩法）。
        // 每一跳都写：播放中数字每帧变；停止后 `position_ticks` 保留 ⇒ 显示与引擎一致。
        apply_transport(ui, transport);
        ProductionTick {
            published_revision,
            publish_error,
            readings: Some(readings),
            meters: pump_meters(ui, &mut self.meters, view),
        }
    }
}

/// **电平腿的一跳**：抽干队列 → 对齐投影 → 写进界面。
///
/// 顺序是契约：先抽干（把队列里的积压全部消费掉），再对齐，最后注入。
///
/// ⚠ **长度契约**：注入之前先比"界面此刻的 `track-names` 长度"与
/// `snapshot.tracks.len()`。不等长时**跳过注入**并如实回报两个长度
/// （理由见 [`MeterPump::LengthMismatch`]）。抽干**照做** —— 音频侧不该因为
/// 界面的轨道集合换了而积压。
///
/// 这个函数跑在 UI 线程上，因此它**允许**分配（`MeterRuntime::poll` 可能扩容搬运缓冲）；
/// 红线 7（`MUST-GATE-001`）约束的是音频线程，而这里没有一行代码跑在音频线程上。
pub fn pump_meters(ui: &MainWindow, meters: &mut MeterRuntime, view: &ViewState) -> MeterPump {
    // `+ 1` 是主总线：引擎每个量子发布的帧数是"非母线轨数 + 1"
    // （`docs/ledger/engine-meters-notes.md` §2 的结构性契约）。
    meters.poll(view.tracks.len() + 1);
    let snapshot = meters.snapshot(view);
    let rows = slint::Model::row_count(&ui.get_track_names());
    if rows != snapshot.tracks.len() {
        return MeterPump::LengthMismatch {
            rows,
            projected: snapshot.tracks.len(),
        };
    }
    apply_meters(ui, &snapshot);
    MeterPump::Applied(snapshot)
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
///
/// ## 调用点（任务单 `#228`）
///
/// - 走带动作之后（[`wire_transport`] / [`wire_keys`]）—— 动作的即时回写；
/// - **每一跳**：`ProductionLoop::tick` 在 `drop(engine)` 之后调它 ⇒ 播放推进期间
///   界面上的数字跟着引擎的 `position_ticks` 走；停住时同样写，显示与引擎一致。
///   ⛔ 心跳里不调它就是本单的缺陷形态："按了播放、数字不动"。
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

/// ⭐ **走带切换的语义**（任务单 `#227`：负责人裁决"用 Logic 方案"）。
///
/// 三选一，`wire_transport` 的 `on_toggle_play` 是**唯一**消费者：
///
/// | 当前 | 按 `Space` | 命令 |
/// | :--- | :--- | :--- |
/// | 播放中 | 停住（**位置保留**） | `Stop` |
/// | 停住 ＋ 上一次 `Space` 停住在 [`HEAD_DOUBLE_TAP_WINDOW`] 内 | **回到起点** | `SeekTicks(0)` |
/// | 其余停住（含在起点、或上一次停住已过期 / 不是 `Space` 停的） | 从当前位置起播 | `Play` |
///
/// ## 为什么"连按"必须带时间窗（⛔ 不是随便挑的 1.5 s）
///
/// 任务单给的表里第 3 / 第 4 行**都是**"停在中途 ＋ `Space`"这一个状态与同一个键：
/// 第一次 ⇒ 继续、连按第二次 ⇒ 回起点。**只有时间**能把这两行区分开 —— 不判时间的
/// 实现会让两行变成同一条规则（要么永远继续、要么永远回起点），那张表就自相矛盾。
/// 因此本实现采用"停住时刻 ＋ 时间窗"：**窗内的下一次 `Space` 是第二次**（回起点），
/// **窗外的是新的一次**（继续）。
///
/// ⚠ **窗口值改过（`#268`，负责人实测裁决）**：初版取 `1.5 s`（任务单举例值，理由是
/// "别让手慢的人丢掉回起点"）。负责人**真机按过四格之后**逐字反馈：**"这个 1.5 秒
/// 我觉得设置太长，应该快速两下 space 这样比较合适"** ⇒ 改成 **`500 ms`** —— 与
/// 系统双击的肌肉记忆同量级（也就是本节原本就写着的"一般 ≤ 500 ms"）。
/// ⛔ 这不是"调参数试手感"，是**负责人对产品的裁决**；再动它要有新的裁决。
///
/// ## 为什么它是纯函数
///
/// 判据要覆盖窗口**两侧**（`1.499 s` ⇒ 回起点、`1.501 s` ⇒ 继续）而不睡两秒；
/// 纯函数让判据能用合成的时间戳把边界逐点钉住。生产路径把 `Instant::now()` 传进来。
///
/// ⚠ `host.rs` **不能**包含内联测试模块（`tests/roll_edit_tools.rs` 用"测试模块标记"的
/// 字面子串划分"生产区"）⇒ 本条判据住在 `src/test_port_adapter.rs`，因此本函数是 `pub`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadCommand {
    /// 从**当前位置**起播（`EngineHost::play`，位置保留）。
    Play,
    /// 停住、**位置保留**（`EngineHost::stop`；引擎的 `Stop` 本就是暂停语义）。
    Stop,
    /// **回到起点**并保持停住（`EngineHost::seek(0)`）。
    RewindToStart,
}

/// Logic 的"连按 `Space` 回起点"时间窗：**1.5 s**（含边界）。
///
/// 口径 = 从"上一次 `Space` 停住"到"这一次 `Space`"的**单调时钟**间隔（`Instant`，
/// 不受墙钟跳变影响）。
pub const HEAD_DOUBLE_TAP_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

/// 一次 `Space`（或 `Shift+Space`）在给定走带状态 / 上一次停住时刻下**该做什么**。
///
/// `running` = 引擎此刻的走带读数（⛔ 不是界面属性 —— 界面属性只是它的镜面）；
/// `last_space_stop` = 上一次由 `Space` 停住的时刻（`None` = 没有待配对的停住）。
///
/// ⚠ 时间比较用 [`std::time::Instant::saturating_duration_since`]：若传进来的
/// `last_space_stop` 在 `now` 之后（合成时间戳写错 / 时钟异常），按"刚停住"处理
/// （保守 → 回到起点），⛔ 不 panic。
#[must_use]
pub fn head_command(
    running: bool,
    last_space_stop: Option<std::time::Instant>,
    now: std::time::Instant,
) -> HeadCommand {
    if running {
        // 播放中：停住、位置保留（Logic 的第一次 `Space`）。
        return HeadCommand::Stop;
    }
    if last_space_stop
        .is_some_and(|stopped| now.saturating_duration_since(stopped) <= HEAD_DOUBLE_TAP_WINDOW)
    {
        // 停在中途 ＋ 连按的第二次：回到起点（仍是停住）。
        return HeadCommand::RewindToStart;
    }
    // 在起点 / 上一次停住已过期 / 上一次停住不是 `Space` 停的 ⇒ 从当前位置继续。
    HeadCommand::Play
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
///
/// ## 走带语义（⭐ 任务单 `#227`：负责人裁决"用 Logic 方案"）
///
/// `on_toggle_play`（`Space` / `Shift+Space` / 播放按钮）**不是**"把引擎状态取反"，
/// 而是 [`head_command`] 的三选一：播放中 ⇒ 停住（位置保留）；停住且上一次 `Space`
/// 停住在 [`HEAD_DOUBLE_TAP_WINDOW`]（500 ms）内 ⇒ **回到起点**；其余停住 ⇒ 继续播放。
/// 三者的时刻判定用的是**引擎读数**与回调自己记的停住时刻（⛔ 不读界面属性）。
///
/// `on_stop`（停止按钮）仍是**停止 ＋ 回起点**（`Stop` + `SeekTicks(0)`），并**清掉**
/// 那个停住时刻 —— 停止按钮本身就是"回起点"，后面的 `Space` 必须是新的第一次。
pub fn wire_transport(
    ui: &MainWindow,
    engine: Rc<RefCell<EngineHost>>,
    // ⭐ **当前工程**（可选；⭐ 与 `wire_keys` 的 `project` 同款形状）：⭐ 录音键要读
    // `transport.count_in_bars` 算**预备拍长度** ⇒ ⭐ 它是该字段的**第一个读者**。
    // ⚠ `None` ⇒ 预备拍 0（⛔ 不假装知道工程）。
    project: Option<Rc<YebanProjectV1>>,
    // ⭐ **录音落进工程的写入面**（任务单 `#132`）：停录成功后要提交一次 `Op::AddClip`
    // ＋ 把那段 WAV 的字节登记进会话资产池 ⇒ 它必须是**活会话那个** `Rc<UndoPort>`。
    // ⚠ `None` ⇒ 录音只落盘、不进工程（判据/无头装配的既有形态：⛔ 不假装提交过）。
    undo: Option<Rc<UndoPort>>,
) {
    // ⭐ 任务单 `#227`：**连按 `Space` 回起点**的唯一状态。
    //
    // 只存"上一次 `Space` 停住"的时刻（`Cell<Option<Instant>>`：`Instant` 是 `Copy`，
    // 单线程 UI 线程上不需要 `RefCell`）。`on_toggle_play` 写 / 读它，`on_stop`
    // **清掉**它（停止按钮 = 明确的回起点动作 ⇒ 下一下 `Space` 必须是"新的第一次"，
    // ⛔ 不让一个陈旧的停住时刻把用户的下一次起播变成 `SeekTicks(0)`）。
    let head_stop = Rc::new(std::cell::Cell::new(None::<std::time::Instant>));
    let toggle = Rc::clone(&engine);
    // UFCS（而不是 `ui.as_weak()`）：`slint::ComponentHandle` 不在本文件的 import 里，
    // 与 `main.rs` 的 `slint::ComponentHandle::run(&ui)` 是同一个理由 —— 既拿到方法，
    // 又不引入一个可能变成 unused import 的 trait（`-D warnings` 下会直接失败）。
    let toggle_ui = slint::ComponentHandle::as_weak(ui);
    let toggle_head = Rc::clone(&head_stop);
    ui.on_toggle_play(move || {
        // ⭐⭐ 任务单 `#212` 的**边界兜底**：本回调与 [`wire_keys`] 拿的是**同一个**
        // `Rc<RefCell<EngineHost>>`，而 `wire_keys` 的 `Action::PlayPause` 臂会
        // `ui.invoke_toggle_play()` —— 若那边在**持借**期间调进来，`borrow_mut()` 就会
        // `RefCell already borrowed`。这个回调跑在 `winit` 的 `key_down`（FFI 边界，
        // **panic 不能 unwind**）里 ⇒ 一次重入 = **进程 abort**。
        //
        // ⇒ 根因已在 [`wire_keys`]（`action_needs_engine`：借的范围只包要动引擎的动作）；
        //   这里是**第二道防线**：借不到就**降级**（打一行、引擎与界面都一位不动），
        //   ⛔ 不 panic、⛔ 不 abort、⛔ 不假装切换过。
        let Ok(mut engine) = toggle.try_borrow_mut() else {
            eprintln!("[yeban-app] 忽略 toggle-play：引擎正被同一线程借用（重入）⇒ 本次按键无效");
            return;
        };
        // ⭐ 任务单 `#227`：判定只读**引擎读数**（`transport().state`）＋ 本回调自己
        // 记下的停住时刻，⛔ 不读界面属性（`playing` 只是引擎的镜面，不能当第二事实源）。
        let now = std::time::Instant::now();
        let reading = match head_command(
            engine.transport().state.is_running(),
            toggle_head.get(),
            now,
        ) {
            HeadCommand::Stop => {
                // 停住时刻只在这里写：它是"连按"的另一半。
                toggle_head.set(Some(now));
                engine.stop()
            }
            HeadCommand::RewindToStart => {
                // 已经停住 ⇒ 只需要定位；清掉时刻，因此**下一下** `Space` 是新的第一次
                //（否则用户会被困在"回起点"里，永远起播不了）。
                toggle_head.set(None);
                engine.seek(0)
            }
            HeadCommand::Play => {
                toggle_head.set(None);
                engine.play()
            }
        };
        // ⭐ 作用域纪律：**先放借，再碰界面**（`apply_transport` 写两个属性；将来若有人在
        // 属性变更里回调引擎，这里也不会重入）。`#212` 的根因就是"持借调界面"。
        drop(engine);
        if let Some(ui) = toggle_ui.upgrade() {
            apply_transport(&ui, reading);
        } else {
            // 窗口已经销毁：动作已经发给引擎了（不留半途状态），只是没人可通知。
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
        }
    });

    let stop = Rc::clone(&engine);
    let stop_ui = slint::ComponentHandle::as_weak(ui);
    let stop_head = Rc::clone(&head_stop);
    ui.on_stop(move || {
        // UI 的"停止"按钮的语义（`transport.slint` 的 accessible-label）是
        // **停止并回到起始点** ⇒ 引擎侧是 `Stop` + `SeekTicks(0)` 两条命令。
        //
        // ⭐ `#212`：与 `on_toggle_play` 同款的两道防线（边界 `try_borrow_mut` ＋ 先放借
        // 再写界面）—— 这两个回调是 `wire_keys` 唯一会重入的两个落点。
        let Ok(mut engine) = stop.try_borrow_mut() else {
            eprintln!("[yeban-app] 忽略 stop：引擎正被同一线程借用（重入）⇒ 本次按键无效");
            return;
        };
        let reading = engine.stop_and_rewind();
        // ⭐ `#227`：停止按钮已经**回起点** ⇒ 清掉"待配对的停住时刻"。否则用户在
        // 按下停止按钮之后 1.5 s 内按 `Space`，会被当成"连按的第二次"而**只是**再定位一次，
        // 看起来像"按了播放却不动"。
        stop_head.set(None);
        drop(engine);
        if let Some(ui) = stop_ui.upgrade() {
            apply_transport(&ui, reading);
        } else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
        }
    });

    // ⭐ **录音键**（`[ARCH-REC-001]`，任务单 `#88`）：接线搬进 [`wire_record_at`] ——
    // 那里有**可注入的工厂缝**（真设备源 / 显式静音腿 / 判据的假源）。
    // 走带这一层只把"产品的四个端口"装配进去（⛔ 不再把源写死在这里）。
    //
    // ⭐ 第 4 个端口（任务单 `#134`）：**播放头读数**。停录落点要拿它当摆放起点，
    // 而播放头的事实源是引擎走带读数（`position_ticks`，音频时钟）⇒ 这里注入一个
    // 读 `EngineHost` 的闭包，⛔ 不在录音侧另存一份位置。
    let playhead_engine = Rc::clone(&engine);
    wire_record_at(
        ui,
        project,
        undo,
        crate::record_wav::RecordPlan::production(),
        crate::record_device::default_input_factory(),
        Rc::new(UiRecordOutcome),
        Rc::new(move || playhead_engine.borrow().transport().position_ticks),
    );
}

/// ⭐ **录音结果落到界面的端口**（任务单 `#88` 的可注入面之一）。
///
/// ## 为什么需要一个 trait（⛔ 不是包装癖）
///
/// 判据要断言"打开失败 ⇒ **界面收到非空错误文本**"，而界面今天**只有** `recording`
/// 一个布尔属性（录音状态**文本**控件属 `.worktrees/mod-app-86` 那条线，本单⛔ 不碰
/// `ui/app.slint`）。把落点抽成一个端口 ⇒ ⭐ 判据能**观察**到产品落点收到了什么，
/// 而产品路径一行没变（[`UiRecordOutcome`] 转调**既有**的 [`apply_record_outcome`]）。
pub trait RecordOutcomePort {
    /// 落一次录音结果（`Some(文本)` ⇒ 失败**出声**；`None` ⇒ 无事）。
    fn report(&self, ui: &MainWindow, error: Option<String>);
}

/// **产品落点**：转调既有的 [`apply_record_outcome`]（`stderr` ＋ 读 `recording`）。
#[derive(Debug, Clone, Copy, Default)]
pub struct UiRecordOutcome;

impl RecordOutcomePort for UiRecordOutcome {
    fn report(&self, ui: &MainWindow, error: Option<String>) {
        apply_record_outcome(ui, error);
    }
}

/// ⭐ 任务单 `#213`：录音器已被同一线程借用时的**一行**降级文案。
///
/// 单独提成常量：① 判据按**逐字子串**数 stderr 行数（子进程读回）；② 让这条边界的
/// 文案只有一个真相源（⛔ 两处手抄）。
const RECORD_REENTRY_STDERR: &str =
    "[yeban-app] 忽略 toggle-record：录音器正被同一线程借用（重入）⇒ 本次按键无效";

/// ⭐⭐ **录音键的接线**（任务单 `#88`）：把"按下去"这件事接到**可注入的源工厂**上。
///
/// ## 形状（与 `[ARCH-REC-001]` 的三段链一一对应）
///
/// ```text
/// 按录音键 ──▶ Recorder::start_with_factory(plan.path, plan.options, factory, 预备拍)
///                     │                                     │
///                     │  ① 工厂开不了 ⇒ RecordError::Source ─┘（⭐ 响亮失败：无文件、不置 recording）
///                     ▼
///           Recorder ⇒ 建环 ⇒ 后台线程从**源**拉块 ⇒ push_block ⇒ RecordDrain ⇒ 落盘
/// 再按一次 ──▶ Recorder::stop ⇒ 报告 ⇒ ⭐ [`land_recording`](crate::record_landing::land_recording)
///                                       （frames > 0 且给了 `undo`）⇒ `outcome.report(ui, err)`
/// ```
///
/// ## 四个注入点（⭐ 判据全部从**代码缝**进，⛔ 不读环境变量）
///
/// | 参数 | 产品给的 | 判据给的 |
/// | :--- | :--- | :--- |
/// | [`RecordPlan`](crate::record_wav::RecordPlan) | 默认路径 ＋ 默认参数 | 判据自己的 scratch 路径 |
/// | `factory` | [`default_input_factory`](crate::record_device::default_input_factory)（真设备源） | 脚本化假源 / 返回 `Err` 的工厂 / [`SilenceFactory`](crate::record_device::SilenceFactory) |
/// | `undo` | ⭐ 活会话那个 `Rc<UndoPort>`（`main.rs` 的 `undo_port`） | `None`（只落盘，不提交）或一个探针端口 |
/// | `outcome` | [`UiRecordOutcome`] | 记录读数的观察端口（内部**仍**转调产品落点） |
///
/// ## ⭐ 停录之后：真的进工程（任务单 `#132` 的 L2＋L3，`#134` 的 L5）
///
/// `frames > 0` 且 `undo` 在场 ⇒ 调
/// [`land_recording_at`](crate::record_landing::land_recording_at)：
/// 读回**那段 WAV 的字节** ⇒ 按内容哈希登记进会话资产池 ⇒ **一次**提交
/// `Op::AddClip`（`ClipContent::Audio { asset }`）＋ `Op::AddClipPlacement`
/// ⇒ `clip_pool` 多一条、**第一条音频轨**的**播放头 tick** 上多一条摆放。
/// ⛔ 0 帧**不**走这一步：0 帧的 WAV 只有头（`#130` 已让它响亮），把它写进
/// `clip_pool` 会在工程里留下一条"播放时什么都没有"的片段。
/// ⛔ `undo` 为 `None` 时也不假装提交过（只打一行如实登记）。
///
/// ⚠ **L4 由 `#133` 接力**（素材池面板）；本函数仍然只做"模型 ＋ 提交"。
/// ⚠ **实时听不到**（`yeban-engine` 的快照不消费 `ClipContent::Audio`，见
/// `crate::record_landing` 的模块文档）：摆放会被**离线渲染**（`render_core`）真的混进
/// 母带，但走带播放那半属引擎层，本单 ⛔ 不改 `crates/yeban-engine/**`。
///
/// ⛔ **失败语义（裁决 §2.3）**：打开失败**绝不**退化成"录一条静音"。
/// `recorder.start_with_factory` 返回 `Err` ⇒ `recording` 保持 `false`、
/// ⛔ 不创建会话、⛔ 不落盘；错误文本走 [`RecordOutcomePort`]（产品落点 = `stderr`）。
pub fn wire_record(
    ui: &MainWindow,
    project: Option<Rc<YebanProjectV1>>,
    undo: Option<Rc<UndoPort>>,
    plan: crate::record_wav::RecordPlan,
    factory: Rc<dyn crate::record_device::InputSourceFactory>,
    outcome: Rc<dyn RecordOutcomePort>,
) -> Rc<RefCell<crate::record_wav::Recorder>> {
    // 兼容形态：起点 tick 取 0（见 [`wire_record_at`] 的说明）。
    wire_record_at(ui, project, undo, plan, factory, outcome, Rc::new(|| 0))
}

/// [`wire_record`] 的**播放头形态**（任务单 `#134`）：停录落点需要一个起点 tick。
///
/// ## 为什么起点是**可注入的读数**（⛔ 不是常量）
///
/// 摆放的 `start_tick` 取"**停录落地这一刻**播放头在哪"（⚠ 不是起录位置：起录位置没有
/// 被记录，⛔ 不假装知道）。产品路径的播放头 = 引擎
/// 走带读数 [`EngineHost::transport().position_ticks`](crate::engine_host::EngineHost::transport)
/// （RT→UI 镜面 = **音频时钟**，⛔ 不用墙钟）⇒ `wire_transport` 用
/// `Rc::new(move || engine.borrow().transport().position_ticks)` 注入。
/// 判据给固定值 ⇒ 读数可复现；⛔ 不读环境变量。
///
/// 其余语义与 [`wire_record`] 逐字相同（0 帧 ⇒ 不落地；`undo = None` ⇒ 只落盘）。
///
/// ## ⭐ 借用边界（任务单 `#213`）
///
/// **返回**本回调手里那个 `Rc<RefCell<Recorder>>`（⛔ 不是副本）：产品调用方可以忽略；
/// 判据用它把"界面落点执行时录音器是否仍被借用"变成**可读的**事实（第 5 个注入面）。
///
/// 回调里 `try_borrow_mut()` **只包住** `is_recording()` ＋ **一次**状态变更
/// （`stop` / `start_with_factory`）—— 结果装进 [`RecordToggle`]，`};` 处放借；
/// `ui.set_recording` / `outcome.report` / 停录后的落盘进工程全在**借外**。
/// 借不到（同一线程上已有借）⇒ 打**一行** stderr 后直接返回：⛔ 不 panic、⛔ 不 abort、
/// ⛔ 不假装切换过（与 `#212` 给 `on_toggle_play` / `on_stop` 加的第二道防线同款）。
///
/// ⚠ 修之前 `borrow_mut()` 写在闭包第一行 ⇒ `RefMut` 活到闭包结束，**跨过**了
/// `ui.set_recording` 与 `outcome.report`。今天同一份 `Recorder` 只有本回调一个借用点，
/// 所以撞不上；但只要将来有人在"录音状态变化"上挂一个要碰录音器的回调，就是
/// `RefCell already borrowed`（`#212` 已实测：落在 FFI 边界 ⇒ **进程 abort**）。
pub fn wire_record_at(
    ui: &MainWindow,
    project: Option<Rc<YebanProjectV1>>,
    undo: Option<Rc<UndoPort>>,
    plan: crate::record_wav::RecordPlan,
    factory: Rc<dyn crate::record_device::InputSourceFactory>,
    outcome: Rc<dyn RecordOutcomePort>,
    playhead_tick: Rc<dyn Fn() -> u64>,
) -> Rc<RefCell<crate::record_wav::Recorder>> {
    let recorder = Rc::new(RefCell::new(crate::record_wav::Recorder::new()));
    // ⭐ 闭包要持有句柄、函数末尾还要把它**原样**交回去 ⇒ 先克隆（`Rc` 计数 +1）。
    let recorder_cell = Rc::clone(&recorder);
    // ⭐ 闭包要读工程算预备拍 ⇒ 先克隆（`Rc` 计数 +1）。
    let record_ui = slint::ComponentHandle::as_weak(ui);
    ui.on_toggle_record(move || {
        // ⭐⭐ 任务单 `#213`：**借只包住"状态变更 / 取结果"**，放借后再碰界面。
        //
        // 崩法与 `#212` 的 `engine` 逐字同形：持借期间调界面 ⇒ 界面回调（若将来有人
        // 在 `recording` 变化上挂）重入同一个 `RefCell` ⇒ `RefCell already borrowed`；
        // 录音键若从 `winit::key_down` 一类 FFI 边界进来，panic 不能 unwind ⇒ **abort**。
        // 因此这里同时上两道防线：① 作用域收窄（[`RecordToggle`] 只装结果）；
        // ② `try_borrow_mut()` 兜底（借不到 ⇒ 一行 stderr ＋ 本次按键无效）。
        let count_in = project.as_ref().map_or(0, |project| {
            crate::record_wav::count_in_frames(project, plan.options.sample_rate)
        });
        let toggle = {
            let mut recorder = match recorder_cell.try_borrow_mut() {
                Ok(recorder) => recorder,
                Err(_) => {
                    eprintln!("{RECORD_REENTRY_STDERR}");
                    return;
                }
            };
            if recorder.is_recording() {
                RecordToggle::Stopped(recorder.stop())
            } else {
                // ⭐ **源从工厂缝拿**：打不开 ⇒ `Err`（⛔ 不静默换成静音源）。
                RecordToggle::Started(recorder.start_with_factory(
                    &plan.path,
                    &plan.options,
                    factory.as_ref(),
                    count_in,
                ))
            }
        }; // ⭐ 借在这里释放（⛔ 不再跨 `ui.` / `outcome.report`）
        match toggle {
            RecordToggle::Stopped(stopped) => {
                // ⭐⭐ **0 帧 ⇒ 响亮**（任务单 `#130`）：停录**成功**但**一帧都没录到**时，
                // `stop` 仍会落一条 0 帧的合法 WAV（头 68 字节、音频 0 字节）并返回 `Ok` ——
                // 这正是"按录音键没用"的**不可见**形态（文件在、内容是空、界面只归零）。
                // ⛔ 不许把它画成成功：把"0 帧 ＋ 真实计数器（`pushed` / `dropped` / `source`）"
                // 拼成一句人话，走**同一个** [`RecordOutcomePort`]（产品落点 = `stderr`）。
                let empty_report = match &stopped {
                    Ok(report) if report.frames == 0 => Some(format!(
                        "录音写入 0 帧（pushed={} dropped={} source={}）⇒ 输入设备没有送来任何帧；\
                     {} 是一条**空** WAV",
                        report.pushed,
                        report.dropped,
                        report.source,
                        report.path.display()
                    )),
                    _ => None,
                };
                // ⭐⭐ **停录成功 ⇒ 真的进工程**（任务单 `#132`；`#130` 分层分析的 L2 ＋ L3）。
                //
                // 只在 `frames > 0` 时落地：0 帧的 WAV 只有头（`#130` 已让它**响亮**），
                // 把它写进 `clip_pool` 等于在工程里登记一条"播放时什么都没有"的片段 ——
                // 判据据 `clip_pool.len()` **不变**读这件事。
                //
                // `undo = None`（判据 / 无头装配）⇒ ⛔ 不假装提交过：如实打一行。
                let mut landing_report: Option<String> = None;
                match &stopped {
                    Ok(report) => {
                        eprintln!("[yeban-app] {}", report.summary());
                        if report.frames > 0 {
                            match undo.as_ref() {
                                Some(port) => {
                                    // 起点 = **停录落地这一刻**的播放头（音频时钟读数）。
                                    // ⚠ 读点在 `recorder.stop()` **之后**（走带若在播放，它会比
                                    // 按下录音键那一刻更靠后）—— 本单的口径是"当前播放头"，
                                    // ⛔ 不假装知道起录位置。只读一次：随后播放头会继续走。
                                    let playhead = playhead_tick();
                                    match crate::record_landing::land_recording_at(
                                        port,
                                        &report.path,
                                        now_ms(),
                                        playhead,
                                    ) {
                                        // 成功：`summary` 带 asset / clip / 字节数（可机械复核）。
                                        Ok(landed) => eprintln!("[yeban-app] {}", landed.summary()),
                                        // ⛔ 停录成功但进工程失败必须**出声**：文件在、工程却没变，
                                        // 这正是本单要消灭的"看不见的丢数据"。
                                        Err(error) => {
                                            landing_report = Some(format!(
                                                "录音已落盘但没有进工程（{}）: {error}",
                                                report.path.display()
                                            ));
                                        }
                                    }
                                }
                                None => eprintln!(
                                    "[yeban-app] 录音落盘但未接入工程（这个装配没有 UndoPort）: {}",
                                    report.path.display()
                                ),
                            }
                        }
                    }
                    Err(error) => eprintln!("[yeban-app] 停录失败: {error}"),
                }
                if let Some(ui) = record_ui.upgrade() {
                    ui.set_recording(false);
                    outcome.report(
                        &ui,
                        stopped
                            .err()
                            .map(|error| error.to_string())
                            .or(empty_report)
                            .or(landing_report),
                    );
                }
            }
            RecordToggle::Started(started) => {
                match &started {
                    Ok(()) => eprintln!(
                        "[yeban-app] 开始录音 ⇒ {}（源={}）",
                        plan.path.display(),
                        factory.label()
                    ),
                    Err(error) => eprintln!("[yeban-app] 起录失败: {error}"),
                }
                if let Some(ui) = record_ui.upgrade() {
                    // ⭐ 起录失败 ⇒ `recording` 保持 `false`（⛔ 不许把失败画成"正在录"）。
                    ui.set_recording(started.is_ok());
                    outcome.report(&ui, started.err().map(|error| error.to_string()));
                }
            }
        }
    });
    recorder
}

/// ⭐ 任务单 `#213`：**一次按录音键的结果** —— 借用作用域里只算出这个值。
///
/// 它存在的唯一理由是**把"借"与"界面"分开**：`stop` / `start_with_factory` 在借内
/// 执行并产出结果，`match` 在借外执行并负责落盘进工程、`eprintln!`、`ui.set_recording`、
/// `outcome.report`。⛔ 不要把这两个变体合并回"先借、再 if/else 顺手写界面"的形状 ——
/// 那正是 `#213` 修的形态。
#[derive(Debug)]
enum RecordToggle {
    /// 本次是**停录**：`Recorder::stop` 的返回值（含 0 帧判定所需的计数器读数）。
    Stopped(Result<crate::record_wav::RecordReport, crate::record_wav::RecordError>),
    /// 本次是**起录**：`Recorder::start_with_factory` 的返回值。
    Started(Result<(), crate::record_wav::RecordError>),
}

/// ⭐ 把录音结果写进界面（⭐ 失败**出声**：⛔ 不静默）。
///
/// ⚠ 界面今天**没有**录音状态文本控件 ⇒ 失败走 `stderr`（与 `host` 的其它模块回调同款：
/// 一个"起录失败"的假成功按钮比没有按钮更糟）。成功不打印在这里 —— `stop` 的
/// `RecordReport::summary` 已经打了一行。
fn apply_record_outcome(ui: &MainWindow, error: Option<String>) {
    if let Some(reason) = error {
        eprintln!("[yeban-app] 录音: {reason}");
    }
    // ⚠ 界面侧现在只有 `recording` 一个录音属性；这里读一次，保证"写了就有读者"
    // （⛔ 不让一个没人读的属性悄悄腐烂）。将来加状态文本时，落点在这里。
    let _ = ui.get_recording();
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

/// ⭐ **这个动作要不要动引擎**（⭐ 任务单 `#212`：`wire_keys` **借不借宿主**的唯一判据）。
///
/// ## 为什么它是**安全属性**的一部分（⛔ 不是分类癖）
///
/// [`wire_keys`] 手里的 `engine` 与 [`wire_transport`] 的两个回调
/// （`on_toggle_play` / `on_stop`）拿的是**同一个** `Rc<RefCell<EngineHost>>`。
/// 凡是在**持借期间**调到界面的动作，都会让那个回调重入 `borrow_mut()` ⇒
/// `RefCell already borrowed`；而按键链从 `winit` 的 `key_down` 进来，那里
/// **panic 不能 unwind** ⇒ 进程 **abort**（`#212` 的真机现象：`Space` 一按就崩）。
///
/// ⇒ 允许在"借内"执行的动作集合必须**一个界面回调都不碰**。
///
/// ## 与 [`apply_engine_command`] 的对应（⭐ 两处必须同时改）
///
/// 返回 `true` 的集合必须**恰好等于** [`apply_engine_command`] 处理的那几条
/// （今天：`Action::ToggleMetronome` / `Action::ToggleCountIn` —— 它们只碰
/// `metronome::set_enabled` / `set_count_in_bars`，那两个函数的入参是 `&mut EngineHost`
/// ＋ `&YebanProjectV1`，**不碰界面**）。
///
/// ⚠ 这条耦合由 `src/test_port_adapter.rs` 的 `only_engine_commands_take_the_engine_borrow`
/// 从**源码文本**对账（两个函数的 `Action::*` 集合必须逐条相同）—— 新增一条"要动引擎"
/// 的动作时，忘记改这里会红，而不是在真机上再崩一次。
///
/// ⚠ **为什么这里不列 `Action::PlayPause` / `Action::ResumeFromCursor`**：它们各自在
/// `host.rs` 的代码里**只能出现一次**（`src/elements.rs` 的
/// `space_and_shift_space_share_one_host_arm_today` 钉住了"两键共用一个臂"）⇒ 在这里
/// 再列一遍会让那条判据红。它们的"不借宿主"由**本函数的 `_ => false`** 保证。
///
/// ⚠ **为什么这里也不列 `Action::GoToBeginning`（任务单 `#227` 的 `Return`）**：它的
/// 落点会调 `ui.invoke_stop()` / `ui.invoke_toggle_play()` 两个界面回调 ⇒ 必须在**借外**
/// 执行。把它加进来就是 `#212` 的重入（`on_stop` 里 `stop.try_borrow_mut()` 会失败，
/// 结果"按了 `Return` 引擎一位不动"）。它的"不借宿主"由本函数的 `matches!` **不列它**
/// 保证；`test_port_adapter.rs` 的 `only_engine_commands_take_the_engine_borrow` 把
/// `GoToBeginning` 放进"界面回调族"逐条断言它不在借内。
#[must_use]
pub fn action_needs_engine(action: Action) -> bool {
    matches!(action, Action::ToggleMetronome | Action::ToggleCountIn)
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
pub fn wire_keys(
    ui: &MainWindow,
    context: Rc<RefCell<InputContext>>,
    undo: Option<Rc<UndoPort>>,
    // ⭐ **引擎上下文**（⭐ 通行形状，与 `wire_transport(ui, engine)` **同款**的
    // `Rc<RefCell<EngineHost>>` 范式；⛔ 零 `unsafe`、⛔ 不造借用延长）。
    // ⚠ `None` ⇒ 那类"要动引擎"的动作**不消费**（照旧放行，⛔ 不假装成功）。
    engine: Option<Rc<RefCell<EngineHost>>>,
    // ⭐ **当前工程**（由装配层持有并传进来 —— `EngineHost` **没有**工程访问器）。
    project: Option<Rc<YebanProjectV1>>,
) {
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
        // ⭐ `#212`：**先结算成一个值**，再进 `match`。
        //
        // ⚠ `match context.borrow().resolve_logical(..) { … }` 会让那个 `Ref`
        // **活着穿过整个 match 体**（scrutinee 的临时值活到 match 结束）⇒ 体内任何
        // `context.borrow_mut()` 都会 panic（`Already borrowed`）。今天体内不碰
        // `context`，但 `wire_input` 的两个回调会 `borrow_mut()` 同一份
        // `InputContext`，而体内会调界面回调（`ui.invoke_toggle_play()` 等）——
        // 这正是 `#212` 那类"持借调界面"的形态。⇒ 结算成值，借在语句结束就放掉。
        let resolution = context.borrow().resolve_logical(key, modifiers);
        match resolution {
            Resolution::PassThrough => false,
            Resolution::ConsumedByIme => true,
            Resolution::Action(action) => {
                // ⭐⭐ 任务单 `#212`：**只有"要动引擎"的动作才借宿主**。
                //
                // ## 为什么"先借住、再看动作"是崩溃（真机 abort 的根因）
                //
                // 这里的 `host` 与 [`wire_transport`] 的 `on_toggle_play` / `on_stop`
                // 拿的是**同一个** `Rc<RefCell<EngineHost>>`（`main.rs` 传的
                // `Some(engine_handle)`）。走带那两条动作的落点是
                // `ui.invoke_toggle_play()` —— 一次**界面回调**，而那个回调自己就要
                // `borrow_mut()` 同一份宿主 ⇒ 持借期间的重入 ⇒ `RefCell already borrowed`。
                // 崩溃点还落在 `winit::platform_impl::macos::view::WinitView::key_down`
                // （FFI 边界，**不能 unwind**）⇒ `thread caused non-unwinding panic. aborting.`
                //
                // ⇒ 借用范围必须与 [`action_needs_engine`] **同集**：那个集合里的动作
                // **一个界面回调都不调**（见该函数的文档）。走带 / 撤销 / 取消族在
                // **借外**执行，因此它们随时可以 `ui.invoke_*`。
                //
                // ⚠ 借到的这一段仍然只活在这条臂里（⛔ 不跨回调、⛔ 不存起来），并且
                // 已经在借用中就**不消费**（⛔ 不 panic）。
                if !action_needs_engine(action) {
                    return apply_action(&ui, undo.as_ref(), None, action);
                }
                match (engine.as_ref(), project.as_ref()) {
                    (Some(host), Some(project)) => match host.try_borrow_mut() {
                        Ok(mut host) => {
                            let mut context = EngineCommandContext {
                                host: &mut host,
                                project: project.as_ref(),
                            };
                            apply_action(&ui, undo.as_ref(), Some(&mut context), action)
                        }
                        Err(_) => false,
                    },
                    _ => apply_action(&ui, undo.as_ref(), None, action),
                }
            }
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
/// 工程从哪来：[`SaveRequest::archive`]。有权威时它**不被使用**（字节由权威自己产出，
/// 见 `save_action` 模块文档）。**没有**权威时它从 `live_port` **现读** —— 因此
/// "界面缓存陈旧 ⇒ 写错内容"不可能发生：写出去的工程与提交图谱就是活会话此刻持有的
/// 那一份，不是装配时刻的快照。
///
/// `live_port` 为 `None`（判据侧没有撤销会话 / 窗口还没接端口）时退回 `port.project`
/// 那份快照，并按"没有历史"写出空 `history.dag`（与此前行为逐字相同）。
///
/// ⭐ 任务单 `#170`：本入口**不知道引擎** ⇒ 落盘成功后**不**更新
/// [`EngineHost::project_dir`]（旧行为，逐位不变）。产品 `run_gui` 走
/// [`wire_save_with_engine`]（多一个引擎句柄，落盘后让工程目录跟着新文件走）。
pub fn wire_save(
    ui: &MainWindow,
    port: SaveStatus,
    live_port: Option<Rc<UndoPort>>,
    // ⭐ **原生文件对话框**（任务单 `#86` 乙）—— "保存到哪"的**可注入**能力面。
    // 只有在**装配期没给目标路径**时才会被问到（既有 `--open` / `--save` 那条路
    // 一次都不弹窗）；判据传自己的假实现，于是"对话框被调用了"与"选了路径 ⇒ 写盘"
    // 两句话都能被机械读数（⛔ 不依赖真弹窗）。
    picker: Rc<dyn crate::file_picker::FilePicker>,
    #[cfg(feature = "in-process-mcp")] authority: Option<crate::mcp_mount::ProjectAuthorityHandle>,
    #[cfg(not(feature = "in-process-mcp"))]
    #[cfg_attr(not(feature = "in-process-mcp"), allow(unused_variables))]
    authority: Option<crate::save_action::AuthorityHandle>,
) {
    wire_save_with_engine(ui, port, live_port, picker, authority, None);
}

/// 与 [`wire_save`] 相同，但**知道这一代的引擎**：落盘成功后把
/// [`EngineHost::set_project_dir`] 指到**新文件**的父目录（任务单 `#170`）。
///
/// ## 补的是哪一格（逐字）
///
/// "另存为"（装配期没有目标路径 ⇒ 对话框挑了一个新位置，或装配方重接了一个新落点）
/// 落盘之后，`EngineHost.project_dir` 仍指向**旧**工程文件所在目录 ⇒ 之后新加的相对
/// 路径 `sampler.path` 会按旧目录解析（界面说"有样本"，播放却读不到）。本入口把"目录跟
/// **当前工程文件**走"这条语义与「打开」那一跳对齐：
///
/// | 这一跳 | 落点 |
/// | :--- | :--- |
/// | 「打开」（`host::PortOpenProject::open_runtime`） | `set_project_dir(新文件的父目录)`，**在** `reload` 之前 |
/// | 「另存为」（本入口） | `set_project_dir(新文件的父目录)`，**在** `dispatch_save` 报 `Saved` 之后 |
///
/// ## 三条口径（承重）
///
/// * ⛔ **取消 / 失败不动它**：用户按取消 ⇒ 回调提前返回；`dispatch_save` 报
///   [`crate::save_action::SaveOutcome::Failed`] ⇒ 不调 `set_project_dir` —— 没落盘就没有
///   "新工程文件"，把目录挪过去会让**当前**工程解析到错的根（假读数）。
/// * ⭐ **路径取自回执**（`SaveOutcome::Saved.path`），不是对话框给的原值：后缀补全
///   （`with_default_extension`）与原子写在落点那侧发生，回执才是"真的写到哪"的真源。
/// * `engine = None` ⇒ 与 [`wire_save`] **逐位相同**（旧装配 / 判据的旧腿一条都不变）。
///
/// # Errors
///
/// 无（失败经 `save-status` 如实回报，与 [`wire_save`] 同一条路）。
pub fn wire_save_with_engine(
    ui: &MainWindow,
    port: SaveStatus,
    live_port: Option<Rc<UndoPort>>,
    picker: Rc<dyn crate::file_picker::FilePicker>,
    #[cfg(feature = "in-process-mcp")] authority: Option<crate::mcp_mount::ProjectAuthorityHandle>,
    #[cfg(not(feature = "in-process-mcp"))]
    #[cfg_attr(not(feature = "in-process-mcp"), allow(unused_variables))]
    authority: Option<crate::save_action::AuthorityHandle>,
    // ⭐ `#170`：落盘成功后要重定向的那个宿主（`None` ⇒ 旧行为，逐位不变）。
    engine: Option<Rc<RefCell<EngineHost>>>,
) {
    // ⭐ `#86` 乙：用户在对话框里挑过一次之后**记住**它（否则每次按保存都要再问一次 ——
    // 那是"每保存一次弹一次窗"的坏体验）。装配期给了路径时本格**永远**是 `None`。
    let chosen: std::rc::Rc<std::cell::RefCell<Option<std::path::PathBuf>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
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
        // ⭐ **目标路径**（`#86` 乙）：装配期的路径优先；否则用上次在对话框里挑的；
        // 再否则**问一次原生对话框**。
        //
        // ⚠ **取消不是失败**：用户按了"取消" ⇒ 本回调**什么都不做**（⛔ 不写
        // `save-status`、⛔ 不调 `dispatch_save`）。把取消写成"保存失败：没有目标路径"
        // 是把"我不想"和"出错了"混成一句 —— 本仓最忌讳的那类假读数。
        let target = match port.target.clone().or_else(|| chosen.borrow().clone()) {
            Some(path) => Some(path),
            None => {
                let picked = picker.choose_save(
                    &crate::file_picker::PickerRequest::save("保存工程", "project.yeban")
                        .with_extensions(["yeban"]),
                );
                match picked {
                    Some(path) => {
                        let path = crate::file_picker::with_default_extension(path, "yeban");
                        *chosen.borrow_mut() = Some(path.clone());
                        Some(path)
                    }
                    None => None,
                }
            }
        };
        let Some(target) = target else {
            return;
        };
        // 保存的是**此刻**的活会话：工程 + 提交图谱 + ⭐ **会话资产池**
        // （`assets/{sha256}`；任务单 `#132`）。编码口径只有
        // `yeban_model::commit::encode_history_dag` 一份（本处不写第二份序列化）。
        // 目标路径是**装配期**的知识（`--open` 的那个文件），不随会话变；它保持
        // `Option`，于是"没有磁盘对应物"仍然如实报 `SaveStage::NoTarget`。
        //
        // ⚠ 工程 / 图谱 / 资产池**一次**从同一个端口读出（同一个 `live` binding）：
        // 分几次读会给"保存的那一版"与"图谱/资产的那一版"留下漂移的余地。
        //
        // ⭐ 资产池的来源：`UndoPort::assets()` —— 这是录音（`crate::record_landing`）与
        // 导入（`crate::import_audio` 的调用方）登记字节的**唯一**会话池。此前这里逐字写
        // `assets: Vec::new()` ⇒ ⭐ GUI 保存会**结构性**丢掉全部资产字节（`#132` 的 L2）。
        let live = live_port.as_ref().and_then(|live| {
            live.try_project()
                .map(|project| (project, live.graph(), live.assets()))
        });
        let request = match live {
            // ⭐ 活会话这一版：工程 ＋ 图谱 ＋ **资产池**（`#132` 补的第三层）。
            Some((project, graph, assets)) => {
                crate::save_action::SaveRequest::with_history_dag_and_assets(
                    target,
                    project,
                    yeban_model::commit::encode_history_dag(&graph),
                    assets,
                )
            }
            // 没有活会话 ⇒ 退回装配时刻的快照，如实写"没有历史 / 没有资产"（既有语义）。
            None => crate::save_action::SaveRequest::new(target, port.project.clone()),
        };
        let outcome = crate::save_action::dispatch_save(&request, authority);
        apply_save_outcome(&ui, &outcome);
        // ⭐ 任务单 `#170`：「另存为」落盘成功 ⇒ 工程目录跟着**这个文件**走（与「打开」
        //    那一跳同一条语义）。⛔ 取消（上面提前 return）与失败（`Failed`）都**不动**它 ——
        //    没有新工程文件就没有可跟的目录；挪过去会让当前工程按错的根解析相对样本。
        //    路径取自**回执**（`Saved.path`）而不是对话框原值：后缀补全与原子写在落点那侧
        //    发生，回执才是"真的写到哪"的真源。
        if let (Some(engine), crate::save_action::SaveOutcome::Saved { path, .. }) =
            (engine.as_ref(), &outcome)
        {
            engine
                .borrow_mut()
                .set_project_dir(path.parent().map(std::path::Path::to_path_buf));
        }
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

// ===========================================================================
// ⭐ 运行期「打开工程」的**唯一**接线（任务单 `#93`）
// ===========================================================================
//
// ## 本单补的是哪一格（`#86` 乙 的边界登记所指的那一格）
//
// `#86` 乙 把**原生文件对话框**接到**已有的**运行期打开入口上了
// （`LiveUi::open_project` / `open_project_via_picker`，落点 `ui/open_project`），
// 但它在文档里如实登记：**产品 `run_gui` 里没有可点的入口**（`--open` 只在启动时读一次，
// 运行期换工程没有实现）。本节就是那个入口的**唯一**实现。
//
// ## 它不新造任何东西（逐条点名复用了什么）
//
// | 这一步 | 复用的既有实现 | ⛔ 没有新造 |
// | :--- | :--- | :--- |
// | 挑一个路径 | [`crate::file_picker::FilePicker`]（`#86` 乙 的能力面） | 第二套对话框 |
// | 读盘 | [`crate::open::open_project_file`]（控制面 `ui/open_project` 的同一个入口） | 第二个容器读法 |
// | 换工程 | [`UndoPort::open_project_session`]（「打开一个工程 = 新会话」） | 第二条换工程实现 |
// | 注入界面 | [`apply_view`]（本文件的唯一注入实现） | 第二个注入点 |
// | 引擎换代 | [`EngineHost::reload`]（`ui/reload_engine` 的同一个落点） | 第二条换代路径 |
// | 回执 | [`apply_module_receipt`] ⇒ **既有**的 `module-status` 通道 | 第二条投影通道 |
//
// ## 顺序是承重的（失败时当前工程**一位不动**）
//
// 1. **先问对话框**：`None`（取消）⇒ **什么都不做** —— ⛔ 不写回执、⛔ 不报错
//    （"我不想"与"出错了"是两件事，见 `crate::file_picker` 的模块文档第 2 条）；
// 2. **（可选）先保存当前工程**：走**已有的**保存入口（`save-project` 回调 =
//    [`wire_save`] 的唯一落点），保存没落盘 ⇒ 拒绝换工程并**响亮**报出一句；
// 3. **读新文件**：读失败 ⇒ 立即返回，当前工程 / 窗口 / 引擎一个字节都没碰；
// 4. **投影一次**（只读）：投影失败（没有主总线 / 拍号退化 / 缩放为 0）⇒ 同上，
//    一个字节都没碰 —— 这一跳存在的理由就是"把失败挡在改状态**之前**"；
// 5. **换会话**：`UndoPort::open_project_session`；权威后端在场 ⇒ 如实拒绝（仍是一位不动）；
// 6. **注入 + 换代**（[`OpenProjectRuntime`] 的实现里）；
// 7. **回执**：成功与失败都经 [`apply_module_receipt`] 写**同一条** `module-status` 通道。

/// ⭐ **「打开一个已经挑好的路径」的能力面**（[`wire_open_project`] 的装配输入）。
///
/// 为什么是 trait 而不是把逻辑写死在一个函数里：本仓的**工程持有者**有两种装配
/// （产品 `run_gui` 的权威是 [`UndoPort`] 里那份会话；控制面形态是执行面自己的投影），
/// 而"读盘 + 换会话 + 重投影"这一段在两种装配里**是同一条**——
/// 差别只在最后那几件装配方自己的事（心跳对齐基准、保存落点、静态快照）。
/// 于是：固定动作住在 [`PortOpenProject`]，装配相关的一跳由 `after` 注入。
///
/// ⚠ `open_runtime` **不**问对话框：路径由调用方（[`wire_open_project`]）给出 ——
/// 那样"对话框被问了几次"与"路径被打开了没有"是两件可分别断言的事。
pub trait OpenProjectRuntime {
    /// 打开**已经挑好**的那个路径。
    ///
    /// # Errors
    ///
    /// 给用户看的完整原因（**非空**）。调用方原样写进 `module-status` 回执。
    fn open_runtime(&self, ui: &MainWindow, path: &str) -> Result<(), String>;
}

/// 换工程之后**装配方最后一跳**的签名：`(ui, 新工程, 新落点, 新投影)`。
///
/// 单独起一个别名的两个理由：① `clippy::type_complexity`（`-D warnings` 下是硬错误）；
/// ② 这**是**一条契约（调用方要拿哪四样东西）—— 别名比内联的函数类型好读。
pub type OpenProjectAfter =
    Rc<dyn Fn(&MainWindow, &YebanProjectV1, &str, &ViewState) -> Result<(), String>>;

/// **产品形态**的「打开工程」：工程的权威是那个 [`UndoPort`]（打开 = 换掉它里面的会话）。
///
/// 任务单 `#93`：`run_gui` 接的就是这一个。判据接的也是**同一个**结构（`production: None`），
/// 因此"产品里点一下真的换工程"这句话在判据里是可机械复核的，而不是靠读 `main.rs`。
pub struct PortOpenProject {
    /// 工程的**唯一可变权威**（`run_gui` 那个 `Rc<UndoPort>` 的**同一份**，不是克隆出来的第二份）。
    pub port: Rc<UndoPort>,
    /// 产品心跳那一代（`None` ⇒ 这个装配没有产品心跳；只重投影界面）。
    ///
    /// `Some` 时本结构负责**换代**（[`EngineHost::reload`] ＋ 采纳新的电平消费端 ＋
    /// 停在停止态），与装配点（`run_gui` 的 [`ProductionLoop::start`] 之后那两行）逐字同款。
    pub production: Option<Rc<RefCell<ProductionLoop>>>,
    /// `reload` 的量子数（与装配时给 [`ProductionLoop::start`] 的那一个同源）。
    pub quanta: u64,
    /// **装配方自己的最后一跳**（`(ui, 新工程, 新落点, 新投影)`）：产品用它更新心跳的对齐
    /// 基准并重接三处**静态快照**（保存落点 / 走带 / 按键）。⛔ 它**不**注入视图、
    /// ⛔ 它**不**换代引擎 —— 那两件是[`Self::open_runtime`]的固定动作。
    pub after: OpenProjectAfter,
}

impl OpenProjectRuntime for PortOpenProject {
    fn open_runtime(&self, ui: &MainWindow, path: &str) -> Result<(), String> {
        // ① 读盘（唯一读取核心）。读失败 ⇒ 到这里就返回：下面一个字节都没改。
        //
        // ⭐ 任务单 `#132`：读的是**全保真归档**（工程 ＋ `history.dag` ＋ 资产池），
        // ⛔ 不是裸工程 —— 否则运行期换工程之后再保存，新工程的 `assets/{sha256}`
        // 会静默消失（保存写的是端口里那一池，而那一池从没被新归档填过）。
        let document = crate::open::open_project_document_file(path, &Default::default())
            .map_err(|error| format!("打开 `{path}` 失败（当前工程一位没动）: {error}"))?;
        let project = document.archive.project.clone();
        // ② 只读投影一次：把"投影不成"这种失败**挡在改状态之前**。
        //    用的是与 ③ 里 `apply_view` **同一个** `project_with_view_state`（不读第二份）。
        let view = project_with_view_state(&project, ui)
            .map_err(|error| format!("`{path}` 投影失败（当前工程一位没动）: {error}"))?;
        // ③ 换会话（「打开一个工程 = **新会话**」的唯一实现）。
        //    权威后端在场 / 根提交被模型拒绝 ⇒ 到这里就返回：界面与引擎都还没碰。
        //    ⭐ 资产池跟着新归档走（替换，⛔ 不合并 —— 见 [`UndoPort::replace_assets`]）。
        self.port.open_project_session(
            format!("open {path}"),
            project.clone(),
            document.archive.assets,
            now_ms(),
        )?;
        // ④ 注入界面（唯一注入点）—— 视图态从**当前窗口**读回（行高 / 缩放不因换工程清零）。
        let width = slint::ComponentHandle::window(ui).size().width as f32;
        let scroll_x = ui.get_roll_scroll_x();
        apply_view(ui, &view, width, scroll_x);
        // ⑤ 引擎换代：新快照 / 新队列 / 新的电平消费端。
        //
        // ⚠ 边界（**既有的**，不是本单引入）：若 `reload` 失败，界面**已经是**新工程
        //    而引擎还是旧一代 —— `live_surface::LiveAdminSurface::apply_project` 的第 3 步
        //    是同一条边界（同一个 `EngineHost::reload`），本处照旧，⛔ 不假装有回滚。
        //
        // ⭐ **没有主总线的工程这一格**（任务单 `#129` 补的，`#161` 收窄了触发面）：
        //    `#161` 起 `samples::default_project()` 出厂自带主总线 ⇒ **新建工程**走的是
        //    下面的 `else` 分支（新快照照常换代）。仍会进这一格的是**真的没有主总线**的
        //    工程（`master_bus_track_id` 为 nil，例如 `YebanProjectV1::default()`，或身份
        //    指向一条不存在的轨），而 `EngineSnapshot::from_project` 的**既有**裁决是
        //    `NoMasterBus`（引擎面一个字没改）。
        //    装配点对同一情况的处置是 [`ProductionLoop::without_engine`]：界面照常开、
        //    播放侧明确报"没有引擎"（`main.rs` 的 `ProductionLoop::start` 失败分支）。
        //    运行期换工程时等价的处置是**清空**这个 `Rc<RefCell<EngineHost>>` 的**内容**
        //    （⛔ 不是换掉 `Rc` —— 走带 / 按键 / 模块那批接线都持有它，换掉会指向旧引擎），
        //    于是播放侧不会继续拿着**上一份**工程的快照（那会让"界面 0 轨、按播放却响旧工程"）。
        //    ⛔ 不清空则 `reload` 必然失败、换工程回执变成"失败"，而界面其实已经换了 —— 假读数。
        let mut reload_error = None;
        if let Some(production) = self.production.as_ref() {
            let engine = production.borrow().engine_handle();
            // ⭐ 任务单 `#168`：相对 SFZ 路径从**新工程文件所在目录**开始解析。
            //    必须在 `reload` **之前**写下（那一跳就要摊平第一份快照）；0 轨分支会把
            //    宿主整体换成 `EngineHost::new()`，那里没有可解析的 SFZ，不重写也无害。
            engine.borrow_mut().set_project_dir(
                std::path::Path::new(path)
                    .parent()
                    .map(std::path::Path::to_path_buf),
            );
            // ⚠ **先读成一个布尔**（⛔ 不写 `if engine.borrow().has_engine() { … }`）：那样写会让
            // 那个 `Ref` **活着穿过整个 `if` 体**，于是体内任何 `engine.borrow_mut()` 都会
            // `RefCell already borrowed` —— `tests/open_project_ui.rs` 的正向腿第一版实测抓到的
            // 就是这一条（本处是那次实测的修法）。
            let has_engine = engine.borrow().has_engine();
            if view.master.is_none() {
                // 0 轨（或主总线不在）⇒ 快照投影不可能。旧一代先停、再清空。
                if has_engine {
                    // 设备腿先放掉（与 `EngineHost::reload` 的第 1.5 步同款），再清空内容。
                    let _ = engine.borrow_mut().close_device();
                    *engine.borrow_mut() = EngineHost::new();
                }
                // 界面必须停在停止态（与装配点 `ProductionLoop::start` 失败后那一条同款；
                // `TransportReading::cold` 的 `state` 不是 running ⇒ `playing: false`）。
                let reading = engine.borrow().transport();
                apply_transport(ui, reading);
            } else if has_engine {
                // ⭐ 任务单 `#136`：把**音频片段字节来源**重新接到这一代宿主上。
                //
                // ③ 的 `open_project_session` 已经把会话资产池**换成新归档的**（替换，⛔ 不合并），
                // 而来源是"读同一个 `Rc<UndoPort>` 的**现读**闭包" ⇒ 重接一次即可让
                // `reload` 第 1.3 步看到新工程的 `assets/{sha256}`。0 轨分支（上面）会把
                // 宿主整体换成 `EngineHost::new()` ⇒ 来源也会丢，所以每次换代都重接。
                {
                    let audio_port = Rc::clone(&self.port);
                    engine
                        .borrow_mut()
                        .set_audio_asset_source(Box::new(move |hash| audio_port.asset_bytes(hash)));
                }
                // ⚠ **同一个坑的第二处**：`match engine.borrow_mut().reload(..) { … }` 也会让那个
                // `RefMut` 活着穿过整个 `match` ⇒ 分支里的 `engine.borrow_mut()` 直接 panic。
                // ⇒ 先把结局**结算**成一个值，再进 `match`。
                let rebuild = engine.borrow_mut().reload(&project, self.quanta);
                match rebuild {
                    Ok(rebuild) => {
                        production
                            .borrow_mut()
                            .meters_mut()
                            .adopt(rebuild.collector);
                        // 与装配点**逐字同款**：新引擎沿用"自由跑"默认值，而界面此刻应当停在
                        // 停止态（`ProductionLoop::start` 的文档第 ① 条）。⛔ 不 stop 会让
                        // `playing: false` 的界面背后真的在跑。
                        engine.borrow_mut().stop();
                        let reading = engine.borrow().transport();
                        apply_transport(ui, reading);
                    }
                    Err(error) => reload_error = Some(error.to_string()),
                }
            }
        }
        // ⑥ 装配方自己的那一跳（心跳对齐基准 / 保存落点 / 静态快照）。
        (self.after)(ui, &project, path, &view)?;
        if let Some(error) = reload_error {
            // 界面已经换了新工程 ⇒ 这一句**如实**说明"播放侧还是上一版"（不静默）。
            return Err(format!(
                "界面已换成 `{path}`，但引擎换代失败（播放侧仍是上一版）: {error}"
            ));
        }
        Ok(())
    }
}

/// ⭐ **运行期「打开工程」的唯一接线**：`.slint` 的 `open-project` 回调 ⇒ 对话框 ⇒
/// [`OpenProjectRuntime`] ⇒ **既有**的 `module-status` 回执通道。
///
/// ## 取消**不是**失败（⛔ 不写回执）
///
/// 用户在对话框里按取消 ⇒ [`FilePicker::choose_open`](crate::file_picker::FilePicker::choose_open)
/// 回 `None` ⇒ 本回调**什么都不做**：既不换工程，也不写 `module-status`
/// （把它写成"打开失败"是把"我不想"和"出错了"混成一句 —— 与 [`wire_save`] 的取消口径、
/// `crate::file_picker` 的模块文档第 2 条**逐字一致**）。
///
/// ## `save_first`：**复用**已有的保存入口
///
/// 为真时，先让**已有的**保存入口（`save-project` 回调，宿主侧唯一实现是 [`wire_save`]）
/// 把当前工程写出去，只有它报告**真的落盘**才继续。⛔ 这里**不**再调一次
/// `save_action::dispatch_save` —— 那会把 `wire_save` 的装配输入（目标路径 / 权威句柄 /
/// 另存为对话框）复制成第二份，两份迟早漂移。
///
/// 保存没落盘（没有目标路径 / 权威只读 / 入口未接线）⇒ **响亮**报 `error=save-first`，
/// 且**当前工程与界面一位不动**（返回得比读新文件还早）。
pub fn wire_open_project(
    ui: &MainWindow,
    picker: Rc<dyn crate::file_picker::FilePicker>,
    runtime: Rc<dyn OpenProjectRuntime>,
    save_first: bool,
) {
    let weak = ui.as_weak();
    ui.on_open_project(move || {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        let started = std::time::Instant::now();
        // ⭐ ① 问**一次**对话框（`#86` 乙 的能力面；请求的标题与后缀过滤与
        //    `open_project_via_picker` **逐字相同**：`"打开工程"` + `["yeban"]`）。
        let picked = picker.choose_open(
            &crate::file_picker::PickerRequest::open("打开工程").with_extensions(["yeban"]),
        );
        let Some(path) = picked else {
            // 取消：什么都不做（⛔ 不写回执、⛔ 不动工程），见上面的文档。
            return;
        };
        let path = path.to_string_lossy().into_owned();
        // ② 先保存（可选）：唯一的保存入口就是那个回调。
        if save_first && let Err(message) = save_first_through_the_existing_entry(&ui) {
            let receipt =
                module_receipt("open-project", &module_error_fields("save-first", &message));
            apply_module_receipt(&ui, &receipt, false);
            module_trace("open-project", started, &receipt);
            return;
        }
        // ③ 打开（唯一实现在 runtime 里）。
        let (receipt, ok) = match runtime.open_runtime(&ui, &path) {
            Ok(()) => (
                module_receipt(
                    "open-project",
                    &[
                        "outcome=ok".to_owned(),
                        format!("path={path}"),
                        // **回读**（不是回显入参）：界面此刻的轨道行数。
                        format!("tracks={}", slint::Model::row_count(&ui.get_track_names())),
                        format!("savedFirst={save_first}"),
                    ],
                ),
                true,
            ),
            Err(message) => (
                module_receipt(
                    "open-project",
                    &module_error_fields("open-project", &message),
                ),
                false,
            ),
        };
        apply_module_receipt(&ui, &receipt, ok);
        module_trace("open-project", started, &receipt);
    });
}

// ⭐ **新建工程**（任务单 `#129`）—— GUI 里"打开工程"旁边的那一格。
//
// ## 本单补的是哪一格
//
// `#93` 给了运行期「打开工程」一个可点的入口，于是 GUI 能**换**工程；但它要求磁盘上
// **已经**有一个 `.yeban`。用户从零开始时唯一的出路是 CLI
// `--project-sample empty --save-as <path>`（`#128` 实测：`saved: … bytes=884 …`，
// `project-counts: tracks-all=0 master-track=0`；`#161` 起是
// `bytes=1136` / `tracks-all=1 master-track=1`，因为空工程自带主总线；
// `#215` 起是 `bytes=1148`，因为创建时刻不再是 `0`）。
// 本单把这同一个落点搬进 GUI。
//
// ## 它不新造任何东西（逐条点名复用了什么）
//
// | 这一步 | 复用的既有实现 | ⛔ 没有新造 |
// | :--- | :--- | :--- |
// | 挑一个落点 | [`crate::file_picker::FilePicker::choose_save`]（`#86` 乙 的能力面） | 第二套对话框 |
// | 补后缀 | [`crate::file_picker::with_default_extension`]（`wire_save` 同一条口径） | 第二套后缀规则 |
// | 空工程对象 | [`yeban_model::samples::default_project()`]（= CLI 的 `--project-sample empty`） | 第二份手写空夹具 |
// | 落盘 | [`crate::save::save_archive_file`]（`--save-as` / GUI 保存**同一条**写入口） | 第二份容器写入 |
// | 换成新工程 | [`OpenProjectRuntime::open_runtime`]（`#93` 的**同一段**换工程代码：读盘 → 投影 → 换会话 → 注入 → 换代 → `after`） | 第二条换工程实现 |
// | 回执 | [`apply_module_receipt`] ⇒ **既有**的 `module-status` 通道 | 第二条投影通道 |
//
// ## 为什么"落盘"要单独是一个**纯函数**
//
// 去掉对话框与界面之后，"新建"剩下的那一半是**纯的**：`ProjectArchive`（空工程 +
// 空图谱 + 空资产池）→ 一个 `.yeban` 文件。把它切成 [`new_project_archive`] ＋
// [`create_new_project_file`] 之后，判据可以在**没有窗口、没有 Tauri/winit、没有对话框**
// 的环境里直接量"写出的字节数 / 读回来的轨道数"，而 `.slint` 那一跳只剩"上抛一个回调"。
// 这不是为了测试而分层：产品的"换成新工程"那一跳也**必须**走真正的读盘路径
// （写出 → 读回 → 换会话），否则"文件里的字节"与"界面上的工程"会各说各话。

/// ⭐ **一份"新建工程"的归档**（任务单 `#129`）= 模型侧的规范空工程 + 空图谱 + 空资产池。
///
/// 与 CLI `--project-sample empty --save-as <path>` 写出的归档**除创建时刻外**逐字段
/// 相同：`history.dag` / 资产池为空是**如实置空**（样本本来就没有第二、第三层状态），
/// 不是丢失 —— 与 [`crate::cli::load_project`] 的样本分支同一条口径。
///
/// "规范空工程"的唯一来源是 [`yeban_model::samples::default_project()`]（`#161` 起是
/// `1148 B／0 音轨 ＋ 1 主总线` 那一条的模型侧对象；`#161` 前是 `884 B／0 轨`，
/// `#215` 前是 `1136 B` —— 创建时刻从 `0` 变成 13 位毫秒数 ⇒ `+12 B`）。
/// —— ⛔ 本函数不手写第二份空工程夹具。
///
/// ⭐ `#215`：创建时刻就是**此刻**，由 [`new_project_archive_with_clock`] 取一次；
/// 同一份归档写两遍仍逐字节相同（时刻是**数据**，不是写盘时取的）。
#[must_use]
pub fn new_project_archive() -> yeban_model::container::ProjectArchive {
    new_project_archive_with_clock(yeban_model::project::now_unix_ms)
}

/// [`new_project_archive`] 的**可注入时钟**形态：新建工程的创建时刻从 `clock` 取
/// **恰好一次**（判据用它把次数数出来）。
///
/// 为什么创建时刻要在这里落：`default_project()` 是**纯**样本（同一对象喂给
/// schema 对账与投影判据，必须逐字节稳定），因此时钟⛔ 不能藏在它里面；
/// "这一份工程是刚建出来的"这件事只有本函数知道（`#215`）。
#[must_use]
pub fn new_project_archive_with_clock(
    clock: impl FnOnce() -> u64,
) -> yeban_model::container::ProjectArchive {
    let mut project = yeban_model::samples::default_project();
    project.record_creation_time(clock());
    yeban_model::container::ProjectArchive {
        project,
        history_dag: Vec::new(),
        assets: Vec::new(),
    }
}

/// ⭐ **「把一个空工程写到 `path`」的纯函数部分**（任务单 `#129`）—— 无界面、无对话框。
///
/// 落点复用**唯一**的归档写入口 [`crate::save::save_archive_file`]：它与 CLI 的
/// `--save-as`、与界面的"保存"争**同一把** `.yeban.lock`、走**同一条**原子写
/// （临时文件 → `sync_all` → `rename` → 刷目录）⇒ 新建出来的文件与 CLI 那条路
/// **逐字节相同**。
///
/// # Errors
///
/// 给用户看的完整原因（**非空**）：容器被拒 / `.yeban.lock` 被占 / I/O 失败，原样上报。
/// 调用方（[`wire_new_project`]）把它写进 `module-status` 回执，⛔ 不静默、⛔ 不假装成功。
pub fn create_new_project_file(
    path: impl AsRef<std::path::Path>,
) -> Result<crate::save::SaveReport, String> {
    crate::save::save_archive_file(&new_project_archive(), path)
        .map_err(|error| format!("新建工程落盘失败: {error}"))
}

/// ⭐ **运行期「新建工程」的唯一接线**（任务单 `#129`）：`.slint` 的 `new-project` 回调 ⇒
/// 保存对话框（"新工程存到哪里"）⇒ [`create_new_project_file`] ⇒ [`OpenProjectRuntime::open_runtime`]
/// （**打开工程那条换工程路径**）⇒ **既有**的 `module-status` 回执通道。
///
/// ## 顺序是承重的（失败时当前工程**一位不动**）
///
/// 1. **先问"有没有对话框"**：`FilePicker::can_prompt()` 为假（[`crate::file_picker::NoPicker`]
///    ⇒ 无头 / 装配期已定路径的档）⇒ **如实报错** `error=no-picker` 并返回。
///    ⛔ 这里**不**走"取消"那一支：把"没有对话框"写成"用户取消"会让一次点击**静默消失**
///    （本仓最忌讳的假读数；任务单 `#129` §1.4 点名要求）。
/// 2. **问一次保存对话框**：`None`（用户取消）⇒ **什么都不做** —— ⛔ 不写回执、⛔ 不建文件
///    （"我不想"与"出错了"是两件事，与 [`wire_save`] / [`wire_open_project`] **逐字一致**）。
/// 3. **落盘**（[`create_new_project_file`]）：失败 ⇒ **响亮**报 `error=create`，当前工程一位不动。
/// 4. **换成新工程**：走 [`OpenProjectRuntime::open_runtime`]（与 [`wire_open_project`] **同一个**
///    换工程实现）。它读的是**刚写出的那个文件**（⛔ 不把内存里的对象直接塞进会话）⇒
///    "界面上的工程"与"磁盘上的字节"不可能分叉。
///    失败 ⇒ 报 `error=swap` 并**点名**文件已经写出（不静默、不假装换成功）。
///
/// ## 为什么**不**先保存当前工程
///
/// [`wire_open_project`] 的 `save_first` 是为"换掉**已有**工程"设计的。新建工程的语义是
/// "从空白开始"，而样本会话（`save_target()` = `None`）连目标路径都没有 ⇒ 若在这里套
/// `save_first`，用户会在**新建**时先被弹一次保存框、并因为"没有目标路径"被拒绝。
/// 用户想留下当前工程时，应当先按"保存"。
///
/// ⚠ **如实登记这个取舍的代价**（⛔ 不粉饰）：当前工程里**未保存的改动会被新工程替换**
/// —— 与 [`wire_open_project`] 的 `saveFirst = true` 不同，本切片**不**先保存。
/// 给新建套 `saveFirst` 会让样本会话（`save_target()` = `None`）先弹一次"保存当前工程"、
/// 并因为"没有目标路径"**拒绝新建** —— 那正好堵死本单要补的"从零开始"。
/// 正确的完整形态（后续切片）是"有未保存改动时问一次『保存 / 丢弃 / 取消』"，
/// 那需要一个新的三态对话框能力面，⛔ 不在本单假装有它。
pub fn wire_new_project(
    ui: &MainWindow,
    picker: Rc<dyn crate::file_picker::FilePicker>,
    runtime: Rc<dyn OpenProjectRuntime>,
) {
    let weak = ui.as_weak();
    ui.on_new_project(move || {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        let started = std::time::Instant::now();
        // ⭐ ① **没有对话框**不是"用户取消"：新工程的落点只能来自对话框 ⇒ 如实报错。
        if !picker.can_prompt() {
            let receipt = module_receipt(
                "new-project",
                &module_error_fields(
                    "no-picker",
                    "新建工程需要先问『新工程存到哪里』，但本装配没有可用的文件对话框\
                     （`FilePicker::can_prompt() = false`）⇒ 拒绝创建（⛔ 不静默、\
                     ⛔ 不猜一个文件名，当前工程一位没动）",
                ),
            );
            apply_module_receipt(&ui, &receipt, false);
            module_trace("new-project", started, &receipt);
            return;
        }
        // ⭐ ② 问**一次**保存对话框（标题与建议文件名是本单的语义；后缀过滤与 `wire_save` 同款）。
        let picked = picker.choose_save(
            &crate::file_picker::PickerRequest::save("新建工程", "project.yeban")
                .with_extensions(["yeban"]),
        );
        let Some(path) = picked else {
            // 取消：什么都不做（⛔ 不写回执、⛔ 不建文件），见上面的文档。
            return;
        };
        // 与 `wire_save` **同一条**后缀口径：用户敲了 `demo` ⇒ 写 `demo.yeban`。
        let path = crate::file_picker::with_default_extension(path, "yeban");
        let path = path.to_string_lossy().into_owned();
        // ⭐ ③ 落盘（纯函数，唯一的归档写入口）。
        let bytes = match create_new_project_file(&path) {
            Ok(report) => report.bytes,
            Err(message) => {
                let receipt = module_receipt(
                    "new-project",
                    &module_error_fields("create", &format!("{message}（当前工程一位没动）")),
                );
                apply_module_receipt(&ui, &receipt, false);
                module_trace("new-project", started, &receipt);
                return;
            }
        };
        // ⭐ ④ 换成新工程 = **打开**刚写出的那个文件（`#93` 的同一段换工程代码）。
        let (receipt, ok) = match runtime.open_runtime(&ui, &path) {
            Ok(()) => (
                module_receipt(
                    "new-project",
                    &[
                        "outcome=ok".to_owned(),
                        format!("path={path}"),
                        format!("bytes={bytes}"),
                        // **回读**（不是回显入参）：界面此刻的轨道行数（新建 = 0 轨）。
                        format!("tracks={}", slint::Model::row_count(&ui.get_track_names())),
                    ],
                ),
                true,
            ),
            Err(message) => (
                module_receipt(
                    "new-project",
                    &module_error_fields(
                        "swap",
                        &format!("已写出 `{path}`（{bytes} 字节），但换成新工程失败: {message}"),
                    ),
                ),
                false,
            ),
        };
        apply_module_receipt(&ui, &receipt, ok);
        module_trace("new-project", started, &receipt);
    });
}

/// 「打开之前先保存」——**复用**已有的保存入口（`save-project` 回调 / [`wire_save`]）。
///
/// 做法只有三步：清掉**上一次**的读数（否则"上次保存成功过"会被读成"这一次也成功了"）
/// ⇒ 触发那**一个**回调 ⇒ 读它的结果。回调体（目标路径解析 / 权威分流 / 原子写 /
/// 回执文本）**一位都不重复**。
///
/// ⚠ 如实登记：本装配的保存入口**未接线**时，`invoke_save_project` 什么都不做，
/// 于是这里如实拒绝换工程（⛔ 不把"没人管保存"当成"保存过了"）。
fn save_first_through_the_existing_entry(ui: &MainWindow) -> Result<(), String> {
    ui.set_save_succeeded(false);
    ui.set_save_status(SharedString::new());
    ui.invoke_save_project();
    if ui.get_save_succeeded() {
        return Ok(());
    }
    let status = ui.get_save_status().to_string();
    Err(if status.is_empty() {
        "先保存当前工程**没有落盘**：本装配的保存入口未接线（或它没有可用的目标路径）\
         ⇒ 拒绝换工程（当前工程一位没动）"
            .to_owned()
    } else {
        format!("{status} ⇒ 拒绝换工程（当前工程一位没动）")
    })
}

/// **模块入口的目标路径／参数** —— `host::wire_module_actions` 的唯一装配输入。
///
/// 为什么目标路径由**宿主**给出而不是界面：
/// 1. `[MODEL-AST-002]` / `ADR-0005` 的方向是"界面只报事件、宿主决定语义"；
/// 2. 文件系统路径是**宿主**的知识（工作目录、环境、未来的配置层），放进 `.slint`
///    只会多一个真相源；
/// 3. 判据因此可以**确定性地**驱动整条链（把四条路径指向 `tempfile` 那样的临时目录），
///    而不需要模拟原生文件对话框。
///
/// 覆盖方式（每一条都独立）：`YEBAN_UI_EXPORT_WAV_PATH` / `YEBAN_UI_EXPORT_MIDI_PATH` /
/// `YEBAN_UI_IMPORT_MIDI_PATH` / `YEBAN_UI_LOAD_SFZ_PATH` / `YEBAN_UI_RECORD_WAV_PATH`，
/// 秒数用
/// `YEBAN_UI_RECORD_SECONDS`（默认 8.0 —— 与 CLI 的 `--export-wav` 默认 8 秒同档）。
///
/// ⭐ **导出 MIDI**（第六条入口，任务单 `#90` 甲）的落盘路径 = `export_midi`，缺省
/// `ui-export.mid`（与 `export_wav` 的 `ui-export.wav` 同款落在当前工作目录）。
/// ⚠ 它**没有**位深 / 采样率 / 时长这类参数 —— SMF 的 PPQ 与格式由投影与**共享**编码器
/// 决定（`crate::export_midi::export_project_to_file`，与 CLI 的 `--export-midi`
/// 是**同一个**函数）。
///
/// ⭐ **硬件 MIDI 输入**（第五条入口）另有两个：`YEBAN_UI_MIDI_IN_PORT`（默认 `"0"`，
/// 取值口径与 CLI 的 `--midi-in` **逐字相同**）与 `YEBAN_UI_MIDI_IN_SECONDS`
/// （缺省 = `crate::midi_in::DEFAULT_SECONDS`；上下限仍由 `midi_in::capture_messages` 判定）。
/// ⚠ 这一条**没有**路径参数 —— 它读的是**物理端口**，不是文件。
/// ⭐ 这一条的语义（**字段文档就是这条映射的唯一出处**）：界面那个按钮 = CLI 的
/// `--midi-in` **＋** `--midi-in-write-notes`。⇒ 采到的消息经 `pair_notes` 配对后，
/// 由 `midi_notes` 的写入原语写进**第一条非主总线音轨**（规则在
/// `crate::import_midi::destination`）。⚠ 界面**没有**开关、**没有**轨道选择器；
/// ⛔ CLI 自己的默认值一位不动（`--midi-in` 单给仍然是只读诊断）。
///
/// 缺省值落在**当前工作目录**下（`ui-export.wav` / `ui-export.mid` / `ui-import.mid` /
/// `ui-instrument.sfz` / `ui-record.wav`），⛔ **不**落到系统临时目录：后者的路径里带
/// 随机分量，会让回执行不可复现，而"回执里的路径"正是判据要断言的东西。
///
/// ⚠ 边界（如实登记）：本票**没有**原生文件对话框 —— 用户不能在这里挑文件，
/// 只能走上面那几个环境变量。那是**另一票**（需要 platform 侧的新能力）；
/// `#86` 乙给 SFZ 采样器加的那条"浏览…"是**另一条**通道（`host::wire_mixer_sampler_browse`），
/// ⛔ 本结构不消费它。
#[derive(Clone, Debug, PartialEq)]
pub struct ModulePaths {
    /// `导出 WAV` 的落盘路径。
    pub export_wav: std::path::PathBuf,
    /// ⭐ `导出 MIDI` 的落盘路径（`SMF 1`；与 CLI 的 `--export-midi` 同一条落盘实现）。
    pub export_midi: std::path::PathBuf,
    /// `导入 MIDI` 的来源文件。
    pub import_midi: std::path::PathBuf,
    /// `装载 SFZ` 的来源文件。
    pub load_sfz: std::path::PathBuf,
    /// `录音` 的落盘路径。
    pub record_wav: std::path::PathBuf,
    /// `录音` 的秒数（`record_wav::RecordOptions::seconds`）。
    pub record_seconds: f32,
    /// ⭐ `硬件 MIDI 输入` 的目标端口（`midi_in::capture_messages` 的第一个参数）。
    ///
    /// 取值口径与 CLI 的 `--midi-in` **逐字相同**（由 `midi_in::resolve_port` 判定）：
    /// 全十进制 ⇒ 端口序号；否则 ⇒ 端口名的**大小写敏感**子串。
    pub midi_in_port: String,
    /// ⭐ `硬件 MIDI 输入` 的采集时长（秒；`midi_in::capture_messages` 的第二个参数）。
    pub midi_in_seconds: f32,
}

impl Default for ModulePaths {
    fn default() -> Self {
        Self {
            export_wav: std::path::PathBuf::from("ui-export.wav"),
            export_midi: std::path::PathBuf::from("ui-export.mid"),
            import_midi: std::path::PathBuf::from("ui-import.mid"),
            load_sfz: std::path::PathBuf::from("ui-instrument.sfz"),
            record_wav: std::path::PathBuf::from("ui-record.wav"),
            record_seconds: 8.0,
            // ⭐ 缺省 = 端口序号 `0`。⚠ 一个端口都没有时 `capture_messages` 会给出
            // `PortNotFound` 并**列出可用端口**（空清单也会说出来）—— 这正是 CI 的形态。
            midi_in_port: "0".to_owned(),
            // ⭐ 与 CLI 的 `--midi-in` 缺省**同一个常量**（⛔ 不另写一个数）。
            midi_in_seconds: crate::midi_in::DEFAULT_SECONDS,
        }
    }
}

impl ModulePaths {
    /// 从环境变量读覆盖值；未设的项用 [`ModulePaths::default`]。
    ///
    /// 为什么放 `host` 而不是 `cli`：这四条路径**只在 GUI 回调里**被用，CLI 的
    /// `--export-wav` 等开关走的是 `cli.rs` 自己的那一份（两者共用的是**模块实现**，
    /// 不是路径解析），因此这里不制造第二个 CLI 面。
    #[must_use]
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let read = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
        };
        let seconds_from = |name: &str, fallback: f32| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
                .unwrap_or(fallback)
        };
        let record_seconds = seconds_from("YEBAN_UI_RECORD_SECONDS", defaults.record_seconds);
        // ⚠ MIDI 的上下限仍由 `midi_in::capture` 自己校验（⛔ 这里不复制那份规则）：
        // 本函数只做"读环境变量 / 给缺省值"，超出 `MAX_SECONDS` 的值会让那条路
        // 如实返回 `InvalidSeconds`（⭐ 与 CLI 完全同一条判定）。
        let midi_in_seconds = seconds_from("YEBAN_UI_MIDI_IN_SECONDS", defaults.midi_in_seconds);
        Self {
            export_wav: read("YEBAN_UI_EXPORT_WAV_PATH").unwrap_or(defaults.export_wav),
            export_midi: read("YEBAN_UI_EXPORT_MIDI_PATH").unwrap_or(defaults.export_midi),
            import_midi: read("YEBAN_UI_IMPORT_MIDI_PATH").unwrap_or(defaults.import_midi),
            load_sfz: read("YEBAN_UI_LOAD_SFZ_PATH").unwrap_or(defaults.load_sfz),
            record_wav: read("YEBAN_UI_RECORD_WAV_PATH").unwrap_or(defaults.record_wav),
            record_seconds,
            midi_in_port: std::env::var("YEBAN_UI_MIDI_IN_PORT")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(defaults.midi_in_port),
            midi_in_seconds,
        }
    }
}

/// 模块入口的回执前缀 —— 成功与失败**共用**，因此"这一次是谁的回执"永远可判。
const MODULE_RECEIPT_PREFIX: &str = "module";

/// 已装载的 SFZ 乐器（`load-sfz` 写、`export-wav` 读）。
///
/// 形状与 `cli.rs` 的 `loaded_sfz` 同：`(乐器, 文件路径)`。路径**必须**一起留着 ——
/// `export_wav_with_sfz_file` 的第二个元素是 `Option<PathBuf>`，给了它 `sample=` 指向的
/// wav 才会被**真读**（`export_wav.rs` 的接线语义）。
///
/// 单独起别名是 clippy 的要求（`clippy::type_complexity`，`-D warnings` 下是硬错误）：
/// 一个内联的四层泛型（`Rc<RefCell<Option<(Arc<..>, PathBuf)>>>`）读不出意图。
type LoadedSfz = Rc<
    RefCell<
        Option<(
            std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
            std::path::PathBuf,
        )>,
    >,
>;

/// 一行回执：`module:action=<动作>:outcome=ok:…` 或
/// `module:action=<动作>:outcome=error:error=<码>:message="<原文>"`。
///
/// 为什么**成功与失败都走这一条**：与 `apply_save_outcome` 同款 —— "失败时忘了刷新
/// 界面"这种错法在构造上不存在；而"失败了却看起来像成功"是本仓库最忌讳的失败模式。
/// 为什么错误要带一个**码**：调用方（判据 / 未来的调用面）不该去解析中文原文，
/// 码是稳定的，原文是给人看的。
fn module_receipt(action: &str, fields: &[String]) -> String {
    let mut out = format!("{MODULE_RECEIPT_PREFIX}:action={action}");
    for field in fields {
        out.push(':');
        out.push_str(field);
    }
    out
}

/// 失败回执的字段（`outcome=error` + 稳定码 + 原文）。
fn module_error_fields(code: &str, message: &str) -> Vec<String> {
    vec![
        "outcome=error".to_owned(),
        format!("error={code}"),
        format!("message=\"{message}\""),
    ]
}

/// 把一条回执写进界面（**唯一**的写入点：成功与失败都从这里过）。
///
/// 它同时把**两条**专用行（母带削波警示 / 导入损失摘要）清空 —— 除各自唯一的
/// 成功分支以外，任何模块动作都不该留下上一条动作的读数（否则"打开工程"之后窗口里
/// 还挂着旧工程的数字）。⭐ `#319`：两条行**互斥**，由这个唯一写入点保证
/// （每次动作都同时写两条，没给的那一条写空串）。
fn apply_module_receipt(ui: &MainWindow, receipt: &str, ok: bool) {
    apply_module_receipt_with_alerts(ui, receipt, ok, "", "");
}

/// 与 [`apply_module_receipt`] 相同，但**同一次**写入里带上削波警示行的文本。
///
/// 空串 = 不显示（`.slint` 侧 `visible: text != ""`）。这条短句只属于
/// `export-wav` 的成功分支；完整读数（`clippedSamples` / `suggestedTrimDb` /
/// 最响的源）仍然在回执最后那一条 `clipping_note="…"` 字段里（`#229`）。
fn apply_module_receipt_with_clipping_warning(
    ui: &MainWindow,
    receipt: &str,
    ok: bool,
    clipping_warning: &str,
) {
    apply_module_receipt_with_alerts(ui, receipt, ok, clipping_warning, "");
}

/// 两条专用行的**同一次**写入（互斥：每次动作都给两条赋值，没给的那条是空串）。
///
/// - `clipping_warning`：`export-wav` 削波时的可见短句（`#230`）；
/// - `import_losses`：`import-midi` 成功时的损失摘要（`#319`，与 CLI 的
///   `imported-midi:` 行读**同一个** `MidiImportLosses`）。
fn apply_module_receipt_with_alerts(
    ui: &MainWindow,
    receipt: &str,
    ok: bool,
    clipping_warning: &str,
    import_losses: &str,
) {
    ui.set_module_status(receipt.into());
    ui.set_module_status_succeeded(ok);
    ui.set_module_clipping_warning(clipping_warning.into());
    ui.set_module_import_losses(import_losses.into());
}

/// 母带削波警示行的**可见短句**（`""` = 不削波 ⇒ 不显示）。
///
/// 为什么只留三件事：回执行宽 264px，`#229` 实测光
/// `module:action=export-wav:outcome=ok:` 就占 216px ⇒ 完整 `clipping_note`
/// 会被 `elide`。这一行是**独立的一行**（`module_actions.slint` 的
/// `module-clipping-warning`，`y = 448`、宽 264），文案压到一行放得下：
/// 削波样本数 ＋ 建议减多少 dB。完整读数照旧在回执字段与
/// `YEBAN_UI_MODULE_TRACE=1` 的 stderr 行里。
fn clipping_warning_line(clipped_samples: u64, suggested_trim_db: f32) -> String {
    format!(
        "⚠ 母带削波：{clipped_samples} 个样本，建议减 {:.1} dB",
        suggested_trim_db.abs()
    )
}

/// `YEBAN_UI_MODULE_TRACE=1` 时把每一步的墙钟读数打到 stderr。
///
/// 为什么要有它：本机纪律禁止在这里跑基准，而"经 UI 触发一次动作要多久"是交付读数。
/// 它**默认关闭**，因此判据与产品行为的确定性不受影响 —— 打开时只多两行 stderr
/// （`eprintln!` 在 UI 线程上，触碰不到音频线程的红线 7）。
fn module_trace(action: &str, started: std::time::Instant, receipt: &str) {
    if std::env::var_os("YEBAN_UI_MODULE_TRACE").is_none() {
        return;
    }
    eprintln!(
        "[yeban-app] ui-module-trace: action={action} elapsed_ms={:.3} receipt={receipt}",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

/// **六个模块入口的接线**：`.slint` 的按钮 → 已经存在的模块实现。
///
/// | 界面回调 | 落点（**复用**，⛔ 不另写一套） | 会不会改工程 |
/// | :--- | :--- | :--- |
/// | `export-wav` | [`crate::export_wav::export_wav_with_sfz_file`] | 不会（只写目标 WAV） |
/// | `export-midi` | [`crate::export_midi::export_project_to_file`] | 不会（只写目标 `.mid`；= CLI `--export-midi` **同一个**函数） |
/// | `import-midi` | [`crate::import_midi::import_midi_file`]（经 `UndoPort`） | **会**（`OpOrigin::MidiInput`，可撤销） |
/// | `load-sfz` | [`crate::sfz_load::load_sfz_with_instrument`] | 不会（只读 SFZ；之后给导出用） |
/// | `record-wav` | [`crate::record_wav::record_test_tone`] | 不会（只写目标 WAV） |
/// | `midi-in` | [`crate::midi_in::capture_messages`] ＋ `midi_notes` 的配对/写入原语 | **会**（`OpOrigin::MidiInput`，可撤销；= CLI `--midi-in` **＋** `--midi-in-write-notes`） |
///
/// ## 工程从哪来（唯一权威）
///
/// 六条里 `import-midi` 与 `midi-in` 改工程，两条走的都是装配给的那个 [`UndoPort`] —— 与
/// `Cmd+Z` / `yeban_undo` **同一个**端口，因此"界面上弹/导入出来的音符"与"撤销栈"不可能分叉。
/// 另外四条（含 `export-midi`）读工程时用 `port.project()`（端口的当前读数），**不**用装配时的
/// 快照 ⇒ 撤销之后按导出，导出的是撤销之后的那一版。
///
/// ## `port` 为 `None` 时
///
/// 装配方没有撤销会话（若干既有判据就是这种形态）⇒ `load-sfz` / `record-wav` 两条
/// 不读工程的动作照常可用，**读工程**的三条（`export-wav` / `export-midi`）如实拒绝
/// `error=no-project`，而 `import-midi` 与 `midi-in` 的**落地**那一半**如实拒绝**
/// （`outcome=error:error=no-undo-port`），⛔ 不静默用一个临时会话把编辑吞掉
/// （那会让"可撤销"变成假话）。
///
/// ## 装载过的 SFZ 会真的被导出用上
///
/// `load-sfz` 成功 ⇒ 乐器存进本闭包的共享槽（`Rc<RefCell<Option<…>>>`），
/// 之后的 `export-wav` 把它交给 `export_wav_with_sfz_file` 的第四个参数
/// ⇒ 与 CLI 的 `--sfz <f> --export-wav out.wav` 是**同一条**音源路径。
pub fn wire_module_actions(ui: &MainWindow, port: Option<Rc<UndoPort>>) -> MidiInState {
    wire_module_actions_with(ui, port, ModulePaths::from_env())
}

/// 与 [`wire_module_actions`] 相同，但路径／参数由调用方**显式**给出（判据用）。
pub fn wire_module_actions_with(
    ui: &MainWindow,
    port: Option<Rc<UndoPort>>,
    paths: ModulePaths,
) -> MidiInState {
    wire_module_actions_with_capture(ui, port, paths, None)
}

/// ⭐ **硬件 MIDI 采集入口**的签名（默认 = [`crate::midi_in::capture_messages`]）。
///
/// 为什么要有这个类型：判据必须**确定性地**观察"采集进行中"那个窗口。真入口在**无端口**
/// 的机器（= CI）上会**立刻**返回（`PortNotFound` 在进入采集循环之前），因此那个窗口只有
/// 微秒级 —— 靠它写判据必然抖，或者变成假绿（本仓明令禁止时序脆弱的判据）。
/// ⇒ 允许调用方注入一个**替身**：替身把窗口拉住，判据就能确定地断言
/// "在飞期间：① 界面处于 `outcome=pending` ② 控件树仍可读 ③ 第二次触发被拒"。
///
/// ⚠ 边界（如实登记）：替身证的是**接线**（工作线程 → 通道 → UI 线程 → 两条状态行），
/// ⛔ **不**证"硬件 MIDI 真的能收事件"。后者由 `--features midi-input` 那条腿上的
/// **真**入口判据（结构化 + 不 panic）与 CLI 的 `--midi-in` 覆盖。
///
/// ## 返回**全量**消息（⛔ 不是给人看的报告）
///
/// 界面这一条与 CLI `--midi-in` 走的是**同一个**函数（[`crate::midi_in::capture_messages`]），
/// 因此拿到的是 [`CapturedMessages`](crate::midi_in::CapturedMessages)：**每一条**原始消息
/// ＋ 真读数。原因是"把这次采到的东西变成音乐"必须看得到每一条 —— 而给人看的
/// `MidiInReport` 只留前 8 条（`FIRST_EVENTS`）。给人看的那一份由
/// [`CapturedMessages::report`](crate::midi_in::CapturedMessages::report) 从**同一批数据**
/// 装配（唯一装配点 [`crate::midi_in::build_report`]），⇒ ⛔ 不二次采集、⛔ 不双份解析，
/// 报告字段一个都不少。
pub type MidiInCapture =
    fn(&str, f32) -> Result<crate::midi_in::CapturedMessages, crate::midi_in::MidiInError>;

/// 与 [`wire_module_actions_with`] 相同，但可注入采集入口（`None` ⇒ 用真入口）。
///
/// 返回**跨线程状态句柄**：[`MidiInState`]。调用方（判据）用
/// [`wait_for_pending_midi_in`] ＋ [`apply_pending_midi_in`] 确定性地排空；
/// 生产路径（`main.rs` / `live_surface.rs`）忽略返回值 —— 那一跳由
/// `Weak::upgrade_in_event_loop` 完成。
pub fn wire_module_actions_with_capture(
    ui: &MainWindow,
    port: Option<Rc<UndoPort>>,
    paths: ModulePaths,
    capture: Option<MidiInCapture>,
) -> MidiInState {
    // 已装载的 SFZ 乐器（`load-sfz` 写、`export-wav` 读）—— 与 CLI 的 `loaded_sfz` 同角色。
    let loaded_sfz: LoadedSfz = Rc::new(RefCell::new(None));

    // ------------------------------------------------------------ 导出 WAV
    {
        let weak = ui.as_weak();
        let port = port.clone();
        let loaded_sfz = Rc::clone(&loaded_sfz);
        let target = paths.export_wav.clone();
        ui.on_export_wav(move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let started = std::time::Instant::now();
            // 工程读数来自端口（撤销之后按导出 ⇒ 导出的是撤销之后那一版）。
            // 没有端口时退回**这一次装配**能看到的界面态：没有工程可导出 ⇒ 如实拒绝。
            let Some(port) = port.as_ref() else {
                let receipt = module_receipt(
                    "export-wav",
                    &module_error_fields("no-project", "没有撤销端口 ⇒ 拿不到当前工程的权威读数"),
                );
                apply_module_receipt(&ui, &receipt, false);
                module_trace("export-wav", started, &receipt);
                return;
            };
            let project = port.project();
            // 装载过 SFZ ⇒ 连**文件路径**一起交下去，于是 `sample=` 指向的 wav 会被**真读**
            // （与 CLI 的 `--sfz <f> --export-wav out.wav` 是同一个参数形态）。
            let sfz = loaded_sfz
                .borrow()
                .clone()
                .map(|(instrument, path)| (instrument, Some(path)));
            // ⭐ 与 CLI 的 `--export-wav` **共用唯一解析处**（⛔ 不各写一份缺省）：
            // UI 今天**没有**位深／采样率／时长的控件（见 `ModulePaths` 的登记与
            // `export_wav::WavExportOptions::resolve` 的文档），四个覆盖值都是 `None`
            // ⇒ 与 CLI 不给 `--wav-*` 时**逐字节相同**的产物（8 秒 / 24-bit / 采样率跟随工程）。
            // ⚠ UI 侧"可选"这半件登记为**待做**：需要 Slint 控件 + 元素 ID + 无头判据。
            let wav_options = crate::export_wav::WavExportOptions::resolve(
                &project,
                None::<yeban_model::project::SampleRate>,
                None::<yeban_render::dither::BitDepth>,
                None::<f32>,
                None::<u64>,
            );
            let (receipt, ok, clipping_warning) = match crate::export_wav::export_wav_with_sfz_file(
                &project,
                &target,
                wav_options,
                sfz,
            ) {
                Ok(report) => {
                    let mut fields = vec![
                        "outcome=ok".to_owned(),
                        format!("path={}", report.path.display()),
                        format!("frames={}", report.frames),
                        format!("sample_rate={}", report.sample_rate),
                        format!("channels={}", report.channels),
                        format!("bits={}", report.bits),
                        format!("bytes={}", report.bytes),
                        format!("source={}", report.source),
                        format!("digest={}", report.digest),
                    ];
                    // ⭐ 任务单 `#229`：**削波时多一条字段**，内容是 CLI 的同一条
                    // [`crate::export_wav::WavExportReport::clipping_note`]（`clippedSamples` /
                    // `suggestedTrimDb` / 最响的源都在里面）。⛔ 不削波时**一个字段都不加**
                    // ⇒ 既有回执逐字不变（判据
                    // `tests/gui_export_clipping.rs` 的干净臂：整条字符串等值断言）。
                    //
                    // 为什么放在**末尾**而不是把字段拆开：① 与 CLI `--export-wav` 的
                    // "多一行"同一个口径（`#226`），区别只是这一行的行首有字段名；
                    // ② `module-status` 是**一行**文本，再拆成几个字段只会让这一行更长、
                    // 可见部分更少；③ 长文案里的 `:` / `=` 不参与回执语法 —— 它整体被
                    // 双引号包住（与 `module_error_fields` 的 `message="…"` 同款）。
                    if let Some(note) = report.clipping_note() {
                        fields.push(format!("clipping_note=\"{note}\""));
                    }
                    // ⭐ 任务单 `#230`：窗口里**看得见**的那一行（与上面那条回执字段
                    // 是同一份报告的两个显示位，⛔ 不是第二份读数）。
                    let warning = if report.clipped_samples == 0 {
                        String::new()
                    } else {
                        clipping_warning_line(report.clipped_samples, report.suggested_trim_db)
                    };
                    (module_receipt("export-wav", &fields), true, warning)
                }
                Err(error) => (
                    module_receipt(
                        "export-wav",
                        &module_error_fields("export-wav", &error.to_string()),
                    ),
                    false,
                    String::new(),
                ),
            };
            apply_module_receipt_with_clipping_warning(&ui, &receipt, ok, &clipping_warning);
            module_trace("export-wav", started, &receipt);
        });
    }

    // ------------------------------------------------------------ 导出 MIDI
    //
    // 任务单 `#90` 甲：与 `export-wav` **逐条同形**（同一份 `port.project()` 读数、
    // 同一个 `apply_module_receipt` 写入点、同一条 `module_trace`），只是落点是
    // **已经存在**的 `crate::export_midi::export_project_to_file` —— 那正是 CLI 的
    // `--export-midi` 调用的**同一个**函数（`cli.rs:3386`），⛔ 本票不另写导出器。
    //
    // 为什么 `port` 是 `Some` 才可用：导出的对象是**当前工程**，而工程的权威读数只有
    // 撤销端口给得出（撤销之后按导出 ⇒ 导出的是撤销之后那一版）。没有端口 ⇒ 如实
    // `outcome=error:error=no-project`，⛔ 不用装配时的快照假装知道。
    {
        let weak = ui.as_weak();
        let port = port.clone();
        let target = paths.export_midi.clone();
        ui.on_export_midi(move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let started = std::time::Instant::now();
            let Some(port) = port.as_ref() else {
                let receipt = module_receipt(
                    "export-midi",
                    &module_error_fields("no-project", "没有撤销端口 ⇒ 拿不到当前工程的权威读数"),
                );
                apply_module_receipt(&ui, &receipt, false);
                module_trace("export-midi", started, &receipt);
                return;
            };
            let project = port.project();
            let (receipt, ok) = match crate::export_midi::export_project_to_file(&project, &target)
            {
                Ok(report) => (
                    module_receipt(
                        "export-midi",
                        &[
                            "outcome=ok".to_owned(),
                            format!("path={}", report.path.display()),
                            format!("bytes={}", report.bytes),
                            format!("ppq={}", report.ppq),
                            format!(
                                "format={}",
                                match report.format {
                                    yeban_midi::midi::MidiFormat::SingleTrack => "single-track",
                                    yeban_midi::midi::MidiFormat::Parallel => "parallel",
                                }
                            ),
                            format!("tracks={}", report.tracks),
                            format!("notes={}", report.notes),
                            format!("tempos={}", report.tempos),
                        ],
                    ),
                    true,
                ),
                Err(error) => (
                    module_receipt(
                        "export-midi",
                        &module_error_fields("export-midi", &error.to_string()),
                    ),
                    false,
                ),
            };
            apply_module_receipt(&ui, &receipt, ok);
            module_trace("export-midi", started, &receipt);
        });
    }

    // ------------------------------------------------------------ 导入 MIDI
    {
        let weak = ui.as_weak();
        let port = port.clone();
        let source = paths.import_midi.clone();
        ui.on_import_midi(move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let started = std::time::Instant::now();
            let Some(port) = port.as_ref() else {
                let receipt = module_receipt(
                    "import-midi",
                    &module_error_fields(
                        "no-undo-port",
                        "没有撤销会话 ⇒ 拒绝导入（编辑必须可撤销，⛔ 不静默吞掉）",
                    ),
                );
                apply_module_receipt(&ui, &receipt, false);
                module_trace("import-midi", started, &receipt);
                return;
            };
            let project = port.project();
            let (receipt, ok, loss_line) = match crate::import_midi::import_midi_file(
                port,
                &project,
                &source,
                &crate::import_midi::MidiImportOptions::default(),
                now_ms(),
            ) {
                Ok(report) => {
                    // ⭐ `#319`：GUI 的第二个出口。两样东西读的是**同一份**
                    // `report.losses`（同一个 `MidiImportLosses`，⛔ 不重解析 SMF、
                    // ⛔ 不重数事件）：
                    // ① 回执末尾两个**明细字段**（引号包住，`=`/空格不参与回执语法）；
                    // ② 界面那条**可见摘要行**（`module-import-losses`）。
                    // 明细与 CLI 的 `imported-midi:` 行尾部共用
                    // `MidiImportLosses::loss_detail()`／`aux_fields()` ⇒ 没有第二份格式化。
                    let loss_line = report.losses.loss_summary();
                    let receipt = module_receipt(
                        "import-midi",
                        &[
                            "outcome=ok".to_owned(),
                            format!(
                                "path={}",
                                report.path.as_ref().map_or_else(
                                    || "<bytes>".to_owned(),
                                    |path| path.display().to_string()
                                )
                            ),
                            format!("notes={}", report.notes),
                            format!("track={}", report.track_id.to_canonical_string()),
                            format!("clip={}", report.clip_id.to_canonical_string()),
                            format!("commit={}", report.commit.to_canonical_string()),
                            format!(
                                "origin={}",
                                match report.origin {
                                    yeban_model::OpOrigin::MidiInput => "midi-input",
                                    yeban_model::OpOrigin::UserUi => "user-ui",
                                    _ => "other",
                                }
                            ),
                            format!("losses=\"{loss_line}\""),
                            format!("loss_detail=\"{}\"", report.losses.loss_detail()),
                        ],
                    );
                    (receipt, true, loss_line)
                }
                Err(error) => (
                    module_receipt(
                        "import-midi",
                        &module_error_fields("import-midi", &error.to_string()),
                    ),
                    false,
                    String::new(),
                ),
            };
            apply_module_receipt_with_alerts(&ui, &receipt, ok, "", &loss_line);
            module_trace("import-midi", started, &receipt);
        });
    }

    // ------------------------------------------------------------- 装载 SFZ
    {
        let weak = ui.as_weak();
        let loaded_sfz = Rc::clone(&loaded_sfz);
        let source = paths.load_sfz.clone();
        ui.on_load_sfz(move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let started = std::time::Instant::now();
            let (receipt, ok) = match crate::sfz_load::load_sfz_with_instrument(
                &source,
                Some(crate::sfz_load::DEFAULT_PROBE_NOTE),
                48_000.0,
            ) {
                Ok((report, instrument)) => {
                    // 记住这份乐器：之后的"导出 WAV"用它发声（与 CLI 的同一语义）。
                    *loaded_sfz.borrow_mut() =
                        Some((std::sync::Arc::new(instrument), report.path.clone()));
                    let fields = vec![
                        "outcome=ok".to_owned(),
                        format!("path={}", report.path.display()),
                        format!("bytes={}", report.bytes),
                        format!("regions={}", report.regions),
                        format!("sample_source={}", report.sample_source),
                        format!(
                            "probe={}",
                            report.probe.as_ref().map_or_else(
                                || "-".to_owned(),
                                |reading| format!(
                                    "{}:{}:{}:{}",
                                    reading.note, reading.velocity, reading.frames, reading.energy
                                )
                            )
                        ),
                        // 接上之后**一定会**被导出用（下一次 export-wav），如实报 yes。
                        "wired_to_export=yes".to_owned(),
                    ];
                    (module_receipt("load-sfz", &fields), true)
                }
                Err(error) => (
                    module_receipt(
                        "load-sfz",
                        &module_error_fields("sfz-load", &error.to_string()),
                    ),
                    false,
                ),
            };
            apply_module_receipt(&ui, &receipt, ok);
            module_trace("load-sfz", started, &receipt);
        });
    }

    // ---------------------------------------------------------------- 录音
    {
        let weak = ui.as_weak();
        let target = paths.record_wav.clone();
        let seconds = paths.record_seconds;
        ui.on_record_wav(move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let started = std::time::Instant::now();
            let mut options = crate::record_wav::RecordOptions {
                seconds,
                ..crate::record_wav::RecordOptions::default()
            };
            // 环容量跟着时长走：默认的 1 秒环在 8 秒请求下会**故意**丢帧（`record_wav.rs`
            // 的实测教训），而界面入口的语义是"把这段录完"，不是"演示丢弃"。
            options.capacity_frames = ((seconds * options.sample_rate as f32) as usize).max(1);
            let (receipt, ok) = match crate::record_wav::record_test_tone(&target, &options) {
                Ok(report) => (
                    module_receipt(
                        "record-wav",
                        &[
                            "outcome=ok".to_owned(),
                            format!("path={}", report.path.display()),
                            format!("frames={}", report.frames),
                            format!("pushed={}", report.pushed),
                            format!("dropped={}", report.dropped),
                            format!("bytes={}", report.bytes),
                            format!("bits={}", report.bits),
                            format!("source={}", report.source),
                        ],
                    ),
                    true,
                ),
                Err(error) => (
                    module_receipt(
                        "record-wav",
                        &module_error_fields("record-wav", &error.to_string()),
                    ),
                    false,
                ),
            };
            apply_module_receipt(&ui, &receipt, ok);
            module_trace("record-wav", started, &receipt);
        });
    }

    // ------------------------------------------------------- 硬件 MIDI 输入
    //
    // ⭐ **这一条不在 UI 线程上跑采集**（用户可感知的缺陷：同步采集那 3 秒里界面卡死）。
    // 形态是"起工作线程 ⇒ 结果经安全通道回来 ⇒ 在 **UI 线程**上落界面 **并写进谱面**"：
    //
    // ```text
    //   UI 线程: on_midi_in
    //     ├─ 已有一次在飞 ⇒ 结构化拒绝（⛔ 不排队、⛔ 不开第二个线程）
    //     ├─ 写 "outcome=pending" 回执（**同步**写 ⇒ 中间态一定可观察）
    //     └─ thread::spawn { crate::midi_in::capture_messages(...) → 存进 Arc<Mutex<…>>
    //                                    → upgrade_in_event_loop { invoke_midi_in_drain_requested() } }
    //   工作线程: ⛔ 不碰任何 Slint 对象（只写 Send 的共享槽）
    //   UI 线程: midi-in-drain-requested handler → apply_pending_midi_in
    //            （**唯一**把结果写进界面、也**唯一**把音符写进工程的地方）
    // ```
    //
    // ## 为什么"排空"是**一个函数**、而不是散在两处
    //
    // `Tier1Platform`（`crates/yeban-ui-test-port`）**只**实现 `create_window_adapter`，
    // **没有** `new_event_loop_proxy` ⇒ 在 Tier-1 平台上 `slint::invoke_from_event_loop`
    // **必然失败**。若"落界面"只挂在事件循环上，那个平台上的结果会**永远停在 pending**。
    // ⇒ 于是把落界面收成 [`apply_pending_midi_in`] 这一个纯函数：生产路径由
    // `invoke_from_event_loop` 调它，没有事件循环的环境由调用方（判据 / 下一个动作）调它。
    // ⛔ 两条路走的是**同一段**代码 ⇒ 界面上的终态不可能因"谁排空的"而不同。
    let midi_state = MidiInState::new();
    {
        let weak = ui.as_weak();
        let wanted = paths.midi_in_port.clone();
        let seconds = paths.midi_in_seconds;
        let state = midi_state.clone();
        // ⭐ 落地端口：与其余模块入口**同一个** `Rc`（唯一可变权威 ⇒ 可撤销是构造上的）。
        //
        // ⚠ 它 **⛔ 不能**被工作线程的 marshal 闭包捕获：`Rc` 不是 `Send`，而
        // `upgrade_in_event_loop` 的闭包**必须** `Send`。⇒ 跨线程只传**一句"请排空"**：
        // 工作线程调 `invoke_midi_in_drain_requested()`，由下面这个在 **UI 线程**上注册的
        // handler（手里握着端口与状态）去调 [`apply_pending_midi_in`]。
        // ⛔ 不引入 `thread_local`、⛔ 不引入 `unsafe`：落地仍然只发生在 UI 线程上。
        let port = port.clone();
        // ⭐ **实时腿的目标音轨**：与"写音符"那一半**同一条**规则（`midi_notes::default_track`）
        // ⇒ 听到的音符与落进谱面的音符在**同一条轨**上。
        //
        // ⚠ `port`（`Rc<UndoPort>`）⛔ 不是 `Send` ⇒ 轨道 id 必须在 **UI 线程**上算好再交给
        // 工作线程（`EntityId` 是纯数据）。⚠ 注入替身（`capture = Some(_)`）时**不走**实时腿：
        // 判据钉住的接线不许被本票改掉（那些判据喂的是**批次**替身）。
        // ⭐ **实时腿的目标**（⭐ **成对**：音轨 ＋ 片段 —— 提交一条音符两样都要）。
        // 与"写音符"那一半**同一条**规则（`midi_notes::destination`：`track = None` ⇒
        // 第一条非 master 音轨 ＋ 它的第一个摆放）⇒ 听到的音与落谱的音在同一条轨、
        // 同一个片段上。⚠ 在 **UI 线程**上解析（`EntityId` 是纯数据，可 `Send`）。
        let live_note_target: Option<(yeban_model::EntityId, yeban_model::EntityId)> =
            if capture.is_none() {
                port.as_ref()
                    .and_then(|port| crate::midi_notes::live_destination(&port.project()).ok())
            } else {
                None
            };
        let live_track = live_note_target.map(|(track, _clip)| track);
        // ⭐ 把落点**记在状态里**：提交点（UI 线程的心跳）从 `MidiInState` 读它
        // （⚠ `EntityId` 是纯数据 ⇒ 可跨线程读；⛔ 不为此每跳克隆工程）。
        state.set_note_target(live_note_target);
        // ⭐ 真入口（与 CLI 的 `--midi-in` 同一个函数）—— 除非调用方注入了替身。
        let capture: MidiInCapture = capture.unwrap_or(crate::midi_in::capture_messages);
        {
            let weak = weak.clone();
            let state = state.clone();
            let port = port.clone();
            ui.on_midi_in_drain_requested(move || {
                let Some(ui) = weak.upgrade() else {
                    debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                    return;
                };
                apply_pending_midi_in(&ui, &state, port.as_deref());
            });
        }
        ui.on_midi_in(move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let started = std::time::Instant::now();
            // ⭐ 先排空上一次的结果（自愈：即使没有事件循环，下一次交互也会把终态落下来）。
            apply_pending_midi_in(&ui, &state, port.as_deref());
            if state.is_in_flight() {
                // ⭐ 在飞期间的第二次触发：**明确拒绝**（⛔ 不排队、⛔ 不并发）。
                let receipt = module_receipt(
                    "midi-in",
                    &module_error_fields(
                        "capture-in-progress",
                        "上一次硬件 MIDI 采集还没结束 —— 本次点击被拒绝（⛔ 不排队、⛔ 不并发）",
                    ),
                );
                apply_module_receipt(&ui, &receipt, false);
                module_trace("midi-in", started, &receipt);
                return;
            }
            // ⭐ **中间态**：在 spawn **之前**同步写 ⇒ 触发后立刻可观察，⛔ 不靠竞态。
            let pending = module_receipt(
                "midi-in",
                &[
                    "outcome=pending".to_owned(),
                    format!("port={wanted}"),
                    format!("seconds={seconds}"),
                ],
            );
            ui.set_module_midi_in_status(pending.clone().into());
            ui.set_module_midi_in_status_succeeded(false);
            apply_module_receipt(&ui, &pending, false);
            state.begin();

            let worker_state = state.clone();
            let worker_wanted = wanted.clone();
            let worker_weak = weak.clone();
            // ⭐ 工作线程**只**做三件事：跑会话、把结果放进 Send 的槽、**请求** UI 线程排空。
            // ⛔ 它一个 Slint 对象都不碰（UI 对象只许在 UI 线程上碰）。
            //
            // ⭐ **实时腿**：会话**开着**的时候就把事件交给车道（`live_lane`），
            // 由 UI 线程的心跳 `MidiInState::publish_live` 送进引擎的音符路径 ——
            // ⛔ 不是"等采集窗口结束再一次性处理"（那正是本票要消灭的时间形状）。
            let live_lane = state.live_lane();
            // ⭐ **音符车道**（⭐ 第二个跨线程车厢）：已配对的音符 ⇒ UI 线程逐条提交。
            let note_lane = state.note_lane();
            // ⭐ 旁路配对器的**会话起点**：与 `monitor_messages_into` 内部 `open_session(wanted, 0)`
            // 的 `origin_us = 0` **同值** ⇒ 两条路算出的 tick 逐位相同（⛔ 不许分叉）。
            const LIVE_ORIGIN_US: u64 = 0;
            let handle = std::thread::spawn(move || {
                let result = match live_track {
                    Some(track) => {
                        // ⭐ 记录器**归工作线程所有**（⭐ 它只在会话线程上被喂）。
                        let mut recorder =
                            crate::midi_notes::IncrementalNoteRecorder::new(LIVE_ORIGIN_US);
                        match crate::midi_in::monitor_messages_into(
                            &worker_wanted,
                            seconds,
                            track,
                            &live_lane,
                            &note_lane,
                            |timestamp_us, status, data1, data2| {
                                recorder.consume(timestamp_us, status, data1, data2)
                            },
                        ) {
                            Ok((captured, mut fields)) => {
                                // ⚠ 收尾：会话结束时还开着的音符**当刻**进车道
                                // （⛔ 不丢：它们按 `last_tick + 尾巴` 闭合，与批次路同一口径）。
                                let unclosed =
                                    recorder.finish(crate::midi_notes::UNCLOSED_TAIL_TICKS);
                                for note in recorder.notes() {
                                    note_lane.push(note);
                                }
                                // ⭐ 积压**如实报出**（⭐ 限流不许变成静默丢弃的伪装）。
                                fields.push(format!(
                                    "live-note-lane-pending={}",
                                    note_lane.pending()
                                ));
                                fields.push(format!(
                                    "live-note-lane-refused={}",
                                    note_lane.refused()
                                ));
                                fields.push(format!("live-note-lane-unclosed={unclosed}"));
                                worker_state.finish_live(fields);
                                Ok(captured)
                            }
                            Err(error) => Err(error),
                        }
                    }
                    // 没有权威工程（或调用方注入了替身）⇒ 保持**批次**形态，一位不变。
                    None => capture(&worker_wanted, seconds),
                };
                worker_state.finish(result);
                // ⭐ **唯一**跨线程的一跳：`Weak` 是 `Send`（上游 `api.rs:1247` 的
                // `unsafe impl<T: StrongHandle> Send for Weak<T>`），
                // `upgrade_in_event_loop` 在 **UI 线程**上升级句柄并执行闭包 ⇒
                // 界面的写入仍然只发生在 UI 线程上。
                // ⚠ 这一跳搬的**只有一句"请排空"** —— 端口（`Rc`，⛔ 不 `Send`）留在 UI 线程上，
                // 由注册好的 `midi-in-drain-requested` handler 使用（见本段开头的注释）。
                // ⚠ 没有事件循环的平台（Tier-1 判据）这里返回 `Err` —— **不是错误**：
                // 结果仍在槽里，调用方用 [`wait_for_pending_midi_in`] ＋
                // [`apply_pending_midi_in`] 排空即可。⛔ 因此这里**不** panic、不报错。
                let _ = worker_weak.upgrade_in_event_loop(move |ui| {
                    ui.invoke_midi_in_drain_requested();
                });
            });
            state.set_worker(handle);
            module_trace("midi-in", started, &pending);
        });
    }

    // ⭐ 把跨线程状态交出去：判据用它做**确定性**排空（join + apply）。
    // 生产路径忽略它 —— 那一跳由工作线程里的 `upgrade_in_event_loop` 完成。
    midi_state
}

/// ⭐ **硬件 MIDI 采集的跨线程状态**（`Arc` ⇒ UI 线程与工作线程各持一份句柄）。
///
/// ## 为什么是 `Arc<Mutex<…>>` / `Arc<AtomicBool>` 而不是 `Rc<RefCell<…>>`
///
/// 工作线程**必须**能把结果放进来 ⇒ 这三个字段都要 `Send + Sync`。
/// ⚠ 槽里装的是**纯数据**（[`crate::midi_in::CapturedMessages`] /
/// [`crate::midi_in::MidiInError`]）⇒ ⛔ 没有任何 Slint 对象跨线程。
/// ⚠ 也**不装**落地端口（`Rc<UndoPort>`，UI 线程独占）：它与结果槽分开，由 UI 线程上的
/// `midi-in-drain-requested` handler 持有（见 [`wire_module_actions_with_capture`] 的注释）。
///
/// ## 生命周期
///
/// `begin` → （工作线程）`finish` → （UI 线程）`apply_pending_midi_in` 取走并落界面。
/// `is_in_flight` 在 `finish` 那一刻就为 `false`，因此"在飞"的窗口 = "已 begin、还没 finish"。
#[derive(Clone)]
pub struct MidiInState {
    /// 在飞标志（`begin` 置真、`finish` 置假）。
    in_flight: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 工作线程交回的结果槽（`Some` ⇒ 有未落界面的结果）。
    slot: std::sync::Arc<
        std::sync::Mutex<
            Option<Result<crate::midi_in::CapturedMessages, crate::midi_in::MidiInError>>,
        >,
    >,
    /// 当前工作线程的句柄（`wait_for_pending_midi_in` 用它做**确定性**等待）。
    worker: std::sync::Arc<std::sync::Mutex<Option<std::thread::JoinHandle<()>>>>,
    /// ⭐ **实时车道**（会话线程（工作线程）⇒ UI 线程）：见
    /// [`crate::midi_in`] 的"实时会话"一节。它是本状态里**唯一**有实时读者的字段 ——
    /// [`Self::publish_live`] 在 60Hz 心跳里把它交给引擎的音符路径。
    live: crate::midi_in::LiveLane,
    /// ⭐ 实时会话交回的**紧凑回执字段**（`Some` ⇒ 这一次会话真的走了实时腿；
    /// ⛔ 注入替身的判据路径永远是 `None` ⇒ 回执不可能多出字段）。
    live_fields: std::sync::Arc<std::sync::Mutex<Option<Vec<String>>>>,
    /// ⭐ **已配对音符的车道**（会话线程配对 ⇒ UI 线程逐条提交）。
    ///
    /// 与 `live` 同型：它是**第二个**跨线程车厢，装的是 `CapturedNote`（已配对结果）
    /// ⇒ UI 线程可以"一条音符 = 一次提交"（⭐ 弹一个音 = 一步撤销）。
    /// ⚠ 容量与语义见 [`crate::midi_in::NoteLane`]（512 条，满则如实计数）。
    note_lane: crate::midi_in::NoteLane,
    /// ⭐ **实时腿的落点**（音轨 ＋ 片段）：由 UI 线程在装配时解析一次，
    /// 提交音符时按它构造 `Op::AddNote`（⭐ `EntityId` 是纯数据 ⇒ 可跨线程读）。
    /// ⚠ 注入采集替身的判据路径是 `None` ⇒ 实时腿整体不生效（既有判据一位不改）。
    note_target:
        std::sync::Arc<std::sync::Mutex<Option<(yeban_model::EntityId, yeban_model::EntityId)>>>,
}

/// ⭐ **已配对音符的提交目标**（见 [`MidiInState::commit_live_notes`]）。
pub struct LiveNoteCommit<'a> {
    /// 唯一写者（⭐ 提交必须经撤销图 ⇒ 音符合法可撤销）。
    pub port: &'a crate::undo::UndoPort,
    /// 目标音轨（⭐ 与配对时用的**同一对** id）。
    pub track_id: yeban_model::EntityId,
    /// 目标片段（⭐ 同上）。
    pub clip_id: yeban_model::EntityId,
    /// 提交时刻（⭐ 函数指针 ⇒ 判据可以喂一个**确定的**时钟）。
    pub now_ms: fn() -> u64,
}

/// ⭐ 一次"提交已配对音符"的读数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveNoteCommitReport {
    /// 本跳真的提交的条数（0 或 1）。
    pub committed: usize,
    /// 本跳被撤销图拒绝的条数（0 或 1）。
    pub refused: usize,
    /// 提交之后车道的积压条数（⭐ 限流必须**看得见**）。
    pub pending: usize,
    /// 收不下的**累计**条数（⭐ ⛔ 不静默丢）。
    pub refused_cumulative: u64,
}

impl core::fmt::Debug for MidiInState {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MidiInState")
            .field("in_flight", &self.is_in_flight())
            .field("has_pending", &self.has_pending())
            .field("live_pending", &self.live.pending())
            .field("note_pending", &self.note_lane.pending())
            .finish()
    }
}

impl MidiInState {
    /// 新建空状态（⛔ 不启动任何线程）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            in_flight: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            slot: std::sync::Arc::new(std::sync::Mutex::new(None)),
            worker: std::sync::Arc::new(std::sync::Mutex::new(None)),
            live: crate::midi_in::LiveLane::new(),
            live_fields: std::sync::Arc::new(std::sync::Mutex::new(None)),
            note_lane: crate::midi_in::NoteLane::new(),
            note_target: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// ⭐ **实时车道的句柄**（工作线程用它把会话的事件交给 UI 线程；容量与口径见
    /// [`crate::midi_in::LiveLane`]）。
    #[must_use]
    pub fn live_lane(&self) -> crate::midi_in::LiveLane {
        self.live.clone()
    }

    /// ⭐ **音符车道的句柄**（工作线程把**已配对**的音符交给 UI 线程）。
    ///
    /// ⚠ 与 [`Self::live_lane`] 分开：两条车道装的东西不同（原始事件 ｖｓ 已配对音符），
    /// 且各自的容量与读数独立（见 [`crate::midi_in::NoteLane`]）。
    #[must_use]
    pub fn note_lane(&self) -> crate::midi_in::NoteLane {
        self.note_lane.clone()
    }

    /// ⭐ 音符车道此刻的积压条数（⭐ 读数：限流必须**看得见**）。
    #[must_use]
    pub fn note_lane_pending(&self) -> usize {
        self.note_lane.pending()
    }

    /// ⭐ 音符车道**收不下**的累计条数（⭐ ⛔ 不静默丢）。
    #[must_use]
    pub fn note_lane_refused(&self) -> u64 {
        self.note_lane.refused()
    }

    /// ⭐ 记下**实时腿的落点**（装配时由 UI 线程解析一次）。
    pub fn set_note_target(&self, target: Option<(yeban_model::EntityId, yeban_model::EntityId)>) {
        if let Ok(mut slot) = self.note_target.lock() {
            *slot = target;
        }
    }

    /// ⭐ 读回落点（⭐ 提交流程用；判据路径是 `None`）。
    #[must_use]
    pub fn note_target(&self) -> Option<(yeban_model::EntityId, yeban_model::EntityId)> {
        self.note_target.lock().ok().and_then(|slot| *slot)
    }

    /// ⭐ **发布实时车道** —— 实时腿在**产品路径上**的落点。
    ///
    /// ⚠ **只在 UI 线程上调用**：`EngineHost` 是 `Rc<RefCell<…>>`（⛔ 不是 `Send`），
    /// 而车道存在的**唯一**理由就是"会话线程够不着引擎"（见 [`crate::midi_in`] 的注释）。
    ///
    /// 返回真的写进引擎环的条数；⚠ 环满时没进去的那一段**放回车道**（下一次心跳重试），
    /// ⛔ 不静默丢（口径与 [`crate::midi_live::pump_once_with`] 的 `refused` 一致）。
    /// ⭐ **已配对音符的提交目标**（⭐ UI 线程才拿得到这些）。
    ///
    /// ⚠ 三个字段都是**构造时**要用的：`port` 是唯一写者（⭐ 提交必须经撤销图），
    /// `track` / `clip` 是落点（⭐ 与会话线程配对时用的**同一对** id ⇒ 听到的音与
    /// 落谱的音在同一条轨、同一个片段上）。
    pub fn commit_live_notes(&self, target: &LiveNoteCommit<'_>) -> LiveNoteCommitReport {
        let mut report = LiveNoteCommitReport::default();
        // ⚠ **每跳最多提交一条**（⛔ 不在一个心跳里连打上百个 commit）：车道保留其余，
        // 下一跳继续。⭐ 积压**看得见**（[`Self::note_lane_pending`]），⛔ 不是静默丢弃。
        if let Some(note) = self.note_lane.pop() {
            let mut model_note = yeban_model::MidiNote::new(
                yeban_model::EntityId::new(),
                note.start_tick,
                note.pitch,
                note.duration_ticks(),
            );
            model_note.velocity = note.velocity;
            let op = yeban_model::Op::AddNote {
                track_id: target.track_id,
                clip_id: target.clip_id,
                note: model_note,
            };
            match target.port.commit_ops_with_origin(
                (target.now_ms)(),
                yeban_model::OpOrigin::MidiInput,
                "midi live note",
                vec![op],
            ) {
                Ok(_) => report.committed = 1,
                // ⚠ 拒绝 ⇒ 计数并**不再放回**（放回会让下一跳重试同一条而卡住心跳）。
                Err(_) => report.refused = 1,
            }
        }
        report.pending = self.note_lane.pending();
        report.refused_cumulative = self.note_lane.refused();
        report
    }

    /// ⭐ **发布实时车道** —— 实时腿在**产品路径上**的落点。
    ///
    /// ⚠ **只在 UI 线程上调用**：`EngineHost` 是 `Rc<RefCell<…>>`（⛔ 不是 `Send`），
    /// 而车道存在的**唯一**理由就是"会话线程够不着引擎"（见 [`crate::midi_in`] 的注释）。
    ///
    /// 返回真的写进引擎环的条数；⚠ 环满时没进去的那一段**放回车道**（下一次心跳重试），
    /// ⛔ 不静默丢（口径与 [`crate::midi_live::pump_once_with`] 的 `refused` 一致）。
    pub fn publish_live(&self, host: &mut crate::engine_host::EngineHost) -> usize {
        let batch = self.live.take();
        if batch.is_empty() {
            return 0;
        }
        let accepted = host.send_engine_events(&batch);
        if accepted < batch.len() {
            self.live.push_front(&batch[accepted..]);
        }
        accepted
    }

    /// 工作线程交回实时腿的回执字段（⛔ 只有工作线程调它；`Ok` 才算"走过实时腿"）。
    fn finish_live(&self, fields: Vec<String>) {
        if let Ok(mut slot) = self.live_fields.lock() {
            *slot = Some(fields);
        }
    }

    /// ⭐ UI 线程**取走**实时腿的回执字段（⭐ `take` ⇒ 上一次会话的读数不会串到下一次）。
    fn take_live_fields(&self) -> Option<Vec<String>> {
        self.live_fields
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// ⭐ 是否**已有一次采集在飞**（界面的"重复触发"闸门读它）。
    #[must_use]
    pub fn is_in_flight(&self) -> bool {
        self.in_flight.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// ⭐ 槽里是否有**还没落到界面**的结果。
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.slot.lock().is_ok_and(|slot| slot.is_some())
    }

    /// 标记"一次采集开始"（由 `midi-in` 回调在 spawn 前调用）。
    fn begin(&self) {
        self.in_flight
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// 工作线程交回结果（⛔ 只有工作线程调它）。
    fn finish(
        &self,
        result: Result<crate::midi_in::CapturedMessages, crate::midi_in::MidiInError>,
    ) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = Some(result);
        }
        self.in_flight
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// 记下当前工作线程（UI 线程调；随后 `wait_for_pending_midi_in` 用它 join）。
    fn set_worker(&self, handle: std::thread::JoinHandle<()>) {
        if let Ok(mut worker) = self.worker.lock() {
            *worker = Some(handle);
        }
    }
}

impl Default for MidiInState {
    fn default() -> Self {
        Self::new()
    }
}

/// ⭐ **确定性等待**：等当前工作线程**真的结束**（join），⛔ 不用 sleep、⛔ 不用轮询。
///
/// 返回 `true` = 这一次真的 join 了一个线程。⭐ 判据在"中间态"窗口之后调它，
/// 于是接下来的排空**一定**能拿到结果（时序由调用方控制，不是由调度器决定）。
pub fn wait_for_pending_midi_in(state: &MidiInState) -> bool {
    let handle = state
        .worker
        .lock()
        .ok()
        .and_then(|mut worker| worker.take());
    handle.is_some_and(|handle| {
        // ⚠ join 的**唯一**后果是"等它跑完"；工作线程自己不做任何 UI 操作，
        // 因此这里不会与 UI 线程互相等待（⛔ 无死锁环）。
        handle.join().is_ok()
    })
}

/// ⭐ **把已交回的结果落到界面上** —— 唯一写 `module-midi-in-status` 的地方。
///
/// ⚠ **必须**在 UI 线程上调用（它写 Slint 属性）。
/// 返回 `true` = 这一次真的落了一个结果（调用方据此知道状态变了）。
///
/// 生产路径：工作线程完成 ⇒ `invoke_from_event_loop` ⇒ 在 UI 线程上执行本函数。
/// 判据路径：`wait_for_pending_midi_in` ⇒ 在 UI 线程上执行本函数。
/// ⇒ ⭐ 两条路是**同一段**代码（终态不可能因"谁排空的"而不同）。
///
/// ## 落地（写音符）
///
/// `port = Some(_)` ⇒ 这一次排空**同时**把采到的音符写进工程（[`land_captured_notes`]）：
/// 界面那个按钮的语义 = CLI 的 `--midi-in` **＋** `--midi-in-write-notes`（见
/// [`ModulePaths`] 的字段文档；界面⛔ 不加开关）。
/// `port = None` ⇒ 落地**结构上**做不了，回执如实给 `error=no-undo-port`
/// （与 `import-midi` 同一道门：⛔ 不静默用一个临时会话把编辑吞掉）。
pub fn apply_pending_midi_in(
    ui: &MainWindow,
    state: &MidiInState,
    port: Option<&UndoPort>,
) -> bool {
    let Some(result) = state.slot.lock().ok().and_then(|mut slot| slot.take()) else {
        return false;
    };
    // ⭐ 实时腿的回执字段：**只在这一次会话真的走过实时腿时**才是 `Some`
    // （`Some` 也就在这里被取走 ⇒ ⛔ 上一次的读数不会串到下一次）。
    let live_fields = state.take_live_fields();
    let (receipt, ok) = match result {
        Ok(captured) => {
            let report = captured.report();
            match land_captured_notes(port, &captured) {
                Ok(landing) => {
                    let mut fields = midi_in_ok_fields(&report);
                    if let Some(live) = live_fields {
                        // ⭐ 实时腿的读数（`live-*`）：说清"这一次有多少事件在**窗口内**
                        // 就进了引擎的音符路径"，⛔ 不夸大（`live-refused` 也一并给）。
                        fields.extend(live);
                    }
                    match landing {
                        // ⭐ 采到的东西里**一条音符都配不出来**（例：只采到 CC）——
                        // 这不是错误（采集本身成功了），但也⛔ 不产生任何提交。
                        None => {
                            fields.push("notes=0".to_owned());
                            fields.push("landing=nothing-to-write".to_owned());
                        }
                        Some(written) => {
                            fields.push(format!("notes={}", written.notes));
                            fields.push(format!("unclosed={}", written.unclosed));
                            fields
                                .push(format!("track={}", written.track_id.to_canonical_string()));
                            fields.push("landing=written".to_owned());
                        }
                    }
                    (module_receipt("midi-in", &fields), true)
                }
                // ⭐ 有音符但落不了地 ⇒ **如实报错**（⛔ 不把"没写进去"说成成功）。
                Err(error) => (
                    module_receipt(
                        "midi-in",
                        &module_error_fields(error.code(), &error.to_string()),
                    ),
                    false,
                ),
            }
        }
        Err(error) => (
            module_receipt(
                "midi-in",
                &module_error_fields(midi_in_error_code(&error), &error.to_string()),
            ),
            false,
        ),
    };
    // ⭐ 两条状态行写的是**同一串字节**（一次计算、写两处）⇒ 不可能分叉。
    ui.set_module_midi_in_status(receipt.clone().into());
    ui.set_module_midi_in_status_succeeded(ok);
    apply_module_receipt(ui, &receipt, ok);
    true
}

/// 成功档回执的**读数**字段（`outcome=ok` ＋ 采集侧六项）—— ⛔ 不含落地读数（调用方按分支追加）。
///
/// `outcome=ok` 在这里：采集**本身**成功。落地失败会走 [`module_error_fields`] 那条分支
/// （那时⛔ 不带这一串，否则回执里会出现两个 `outcome=`）。
fn midi_in_ok_fields(report: &crate::midi_in::MidiInReport) -> Vec<String> {
    vec![
        "outcome=ok".to_owned(),
        format!("seconds={}", report.seconds),
        format!("events={}", report.events),
        format!("unsupported={}", report.unsupported),
        // ⭐ 与 CLI（`midi_in.rs` 的 `summary()`）**同名同口径**：单位 = 事件条数、
        // **累计**量（从建环那一刻到采集结束时那一次读，读它不消费）。
        format!("dropped-events={}", report.dropped_events),
        format!(
            "jitter={}",
            report.jitter.map_or_else(
                || "n=0".to_owned(),
                |reading| format!(
                    "n={}:p50={}:p99={}:max={}",
                    reading.n, reading.p50, reading.p99, reading.max
                )
            )
        ),
        format!("first={}", report.first_events.len()),
    ]
}

/// ⭐ **把一次采集真的变成谱面** —— 与 CLI `--midi-in-write-notes` 走**同一批原语**。
///
/// 顺序与 [`crate::cli::apply_captured_midi`] 的**谱面那一半**逐字相同：
/// [`crate::midi_notes::pair_notes`] ⇒ `midi_notes` 的写入原语（`write_notes`）。
/// ⛔ 这里**不**写第二份配对 / 写音符实现（那正是本仓最忌讳的第二个真相源）。
///
/// ## 目标（与 CLI 同一条规则）
///
/// `track = None` ⇒ **第一条非 master 音轨 ＋ 它的第一个摆放**。规则**不在本函数**：
/// 它住在 [`crate::import_midi::destination`]（`midi_notes` 的写入原语转调它，
/// 与 `--import-midi` / `--midi-in-write-notes` 共用）。本函数**只传 `None`**，
/// ⇒ "界面选哪条轨"这件事在界面这一侧根本没有第二个说法（界面**没有**轨道选择器）。
///
/// ## 返回
///
/// * `Ok(None)`：配对结果里**一条音符都没有**（例：只采到 CC / 空窗口）⇒ 无话可写；
/// * `Ok(Some(_))`：真的写进去了（读数带出去，`origin = OpOrigin::MidiInput`）；
/// * `Err(_)`：有音符但落不了地（没有端口 / 没有目标 / 提交被拒）。
fn land_captured_notes(
    port: Option<&UndoPort>,
    captured: &crate::midi_in::CapturedMessages,
) -> Result<Option<crate::midi_notes::NoteWriteReport>, MidiInLandingError> {
    let Some(port) = port else {
        return Err(MidiInLandingError::NoPort);
    };
    let pairing = crate::midi_notes::pair_notes(
        &captured.messages,
        captured.origin_us,
        crate::midi_notes::UNCLOSED_TAIL_TICKS,
    );
    if pairing.notes.is_empty() {
        return Ok(None);
    }
    let project = port.project();
    crate::midi_notes::write_notes(port, &project, &pairing, None, now_ms())
        .map(Some)
        .map_err(MidiInLandingError::Notes)
}

/// ⭐ 落地失败的**两种**原因（稳定码由 [`Self::code`] 给出，⛔ 不用 `Display` 原文当判别式）。
#[derive(Debug)]
enum MidiInLandingError {
    /// 这一次装配没有撤销端口 ⇒ 谱面那一半**结构上**做不了（与 `import-midi` **同码**）。
    NoPort,
    /// 有音符，但写不进工程（没有目标音轨 / 摆放，或提交被撤销端口拒绝）。
    Notes(crate::midi_notes::MidiNoteError),
}

impl MidiInLandingError {
    /// 回执里的稳定错误码。
    fn code(&self) -> &'static str {
        match self {
            Self::NoPort => "no-undo-port",
            Self::Notes(_) => "midi-notes",
        }
    }
}

impl core::fmt::Display for MidiInLandingError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoPort => write!(
                formatter,
                "这一次装配没有撤销端口 ⇒ 采到的音符写不进工程（与 `import-midi` 同一道门：\
                 ⛔ 不静默用一个临时会话把编辑吞掉）"
            ),
            Self::Notes(error) => write!(formatter, "{error}"),
        }
    }
}

/// ⭐ **稳定错误码**：把 [`crate::midi_in::MidiInError`] 的六个变体映成一个可断言的码。
///
/// 为什么需要它（而不是只留 `Display` 的原文）：回执的 `message="…"` 是**人话**、会随措辞
/// 变化；判据（与未来的调用面）要的是**稳定**的判别式。⛔ 这里不复制任何判定逻辑 ——
/// 端口解析 / 秒数校验 / 宿主可用性**全部**由 `midi_in` 那条路决定，本函数只做**分类**。
///
/// ⚠ 本函数**不改** `midi_in.rs`：它是对既有公开枚举的一次 `match`（那个文件正在
/// 另一条线上被改，本票按纪律不碰它）。
fn midi_in_error_code(error: &crate::midi_in::MidiInError) -> &'static str {
    use crate::midi_in::MidiInError;
    match error {
        // ⭐ 默认档（feature 关）走这条：⭐ 措辞由 `Display` 给出（点名 feature + 怎么开）。
        MidiInError::FeatureDisabled => "feature-disabled",
        MidiInError::InvalidSeconds { .. } => "midi-invalid-seconds",
        MidiInError::PortNotFound { .. } => "midi-port-not-found",
        MidiInError::AmbiguousPort { .. } => "midi-ambiguous-port",
        MidiInError::HostUnavailable(_) => "midi-host-unavailable",
        MidiInError::ConnectFailed { .. } => "midi-connect-failed",
    }
}

/// `Action` 是否有**可作用的实现** —— 宿主对此的**唯一**陈述。
///
/// 这张名单回答的是"按下去会不会真的发生一件事", 因此它是**用户可见承诺**的事实源：
/// `--print-shortcuts` 的 [`crate::cli::UNIMPLEMENTED_MARKER`] 标记、下面 `apply_action`
/// 的拒绝分支都从这里读。`tests/cli_contract.rs` 的判据
/// `shortcut_table_status_matches_the_resolution_and_host_pipeline` 把快捷键表逐行与它
/// 对账 —— 表与行为因此不可能各说各话。
///
/// 返回 `false` 的三条（`AuditionMain` / `AuditionProposal` / `AcceptAiSuggestion`）
/// 是**能力**还没落地的动作。**不消费**它们是刻意的：
/// 把键吞掉却什么都不做，比不处理更糟 —— 用户会以为"这个功能坏了"，而日志里没有任何东西能解释。
///
/// ## 三条各自缺什么（2026-10-08 实测；这是**缺口登记**，不是待办装饰）
///
/// 缺的是**能力**，不是"接线"。把它们接上所需的机制在仓库里**不存在**，因此本轮**有意**
/// 保持这三条在名单里（`docs/ledger/feature-alignment.md` 的 AI 提案行与走带行有就地加注）。
///
/// **① `AcceptAiSuggestion`（`Shift+Enter` 采纳 AI 建议）—— 没有"待采纳的提案"这个对象。**
/// 领域侧**有**提案概念：`crates/yeban-mcp/src/domain/proposal.rs` 的 `Proposal`
/// （`ProposalStatus::Open`）与 `yeban_merge_proposal`（要 `proposalId`）。
/// 但界面侧**没有提案身份的来源**：`grep -rn 'proposalId' crates/yeban-app/src` 的命中
/// **全部落在注释里**（本文件的这一段与 `main.rs` 的 `wire_callbacks` 文档），没有一处是取值。
/// 抽屉里的 `proposal-count` / `proposal-labels` / `proposal-kinds` / `confidences`
/// 是**空的**（`crates/yeban-app/ui/dialogs/musical_pr_drawer.slint` 的 `:32-35`：
/// `0` 与四个空数组），界面在零提案时画一句用户可见的空态
/// （`musical-pr-empty-state`，`:12` 自陈"静态骨架, **不含演示数据**"）。
/// **2026-10-08 更正**：这四个属性此前是**内联演示常量**（三条中文假提案 + 三个假置信度），
/// 于是界面显示"AI 提了 3 条提案"而用户分不出真假；本切片删掉了那批数据并加了空态。
/// 两条判据钉住它：`elements.rs` 的 `musical_pr_drawer_declares_no_demo_proposals` /
/// `no_host_writes_the_musical_pr_proposal_properties`（文本层 + 属性层）与
/// `tests/live_ui_mcp.rs` 的 `the_musical_pr_drawer_shows_an_honest_empty_state`（运行时树）。
/// 清空数据**不**改变本条结论：界面侧**仍然没有**"当前待采纳的提案身份"这个表示，
/// 因此 `Shift+Enter` 仍然如实不消费。
/// 提案注册表（`Domain.proposals`）住在 `crates/yeban-mcp/src/domain/mod.rs:155`，
/// 由 `crates/yeban-app/src/mcp_mount.rs` 在**非默认 feature** `in-process-mcp` 且
/// 运行期 `--enable-mcp-http` 打开时才建，**不在** UI 线程的 `apply_action` 路径上。
/// ⇒ 空前置条件（今天**任何**装配）下 `Shift+Enter` 都返回 `false`（不消费），
/// 与 `DeleteSelection` / `ZoomToSelection` / `Duplicate` 空选区时的取向一致。
///
/// **② `AuditionMain`（`[` 试听主线）与 ③ `AuditionProposal`（`]` 试听 AI 提案分支）——
/// 规范 `[ARCH-RT-005]` 的 A/B 盲听机制未实现。**
/// 规范 `[ARCH-RT-005]` 要求"主线与 AI 提案分支**并发渲染**、30ms 等功率瞬切、
/// 下一拍对齐、2048 采样预滚"（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:391-392`）。
/// 实测这套机制**一处都不存在**：
/// `grep -rn 'ARCH-RT-005' crates/ schemas/ scripts/` 命中 **0**，
/// `crates/yeban-engine/src/lib.rs:128-147` 的 `IMPLEMENTED_SPEC_IDS` 里**没有**它；
/// 引擎的走带命令只有四条（`crates/yeban-engine/src/ring.rs:82-91` 的
/// `Play` / `Stop` / `Pause` / `SeekTicks`）—— **没有**"切到哪条分支"这一维；
/// 引擎只持**一份**当前快照（`crates/yeban-engine/src/snapshot.rs:1140-1156` 的
/// `SnapshotSlot` 是单个 `AtomicPtr` + 单个 `anchor`），`crates/yeban-engine/src/rt.rs`
/// 的渲染循环只读那一份 ⇒ 没有"双分支并发"可言，也**没有任何接线到渲染循环的**交叉淡化代码（原语存在，见下）
/// （⚠ **更正（2026-10-13）**：该 grep **不再命中 0** —— 原语已于 2026-10-11 落地（`683c72d7` / `4efc2bbf`）：`crates/yeban-engine/src/mixer.rs:213` `AUDITION_CROSSFADE_MS=30.0`、`:219` `AUDITION_PREROLL_SAMPLES=2048`、`:229` `crossfade_len_frames`、`:247` `crossfade_gains`（等功率），`crates/yeban-engine/src/transport.rs:473` `audition_switch_frame`、`:482` `frames_until_audition_switch`（下一拍对齐）。**但它们的**外部调用点为 0**（`crates/` 内 25 处出现全在 `mixer.rs` 与 `transport.rs` 的定义与自测里），且 `[ARCH-RT-005]` **仍不在** `IMPLEMENTED_SPEC_IDS` 里 ⇒ **故本函数的返回值不变**）；
/// 界面侧也没有第二条工程版本：宿主播放的就是活跃工程（= 主线），
/// `crates/yeban-app/src/engine_host.rs` 的 `publish_project` 换的是**那一份**快照。
/// ⇒ 今天按 `[` 唯一能做的事是"播放"——那已经是 `Space`（`Action::PlayPause`），
/// 再接一次就等于用一个重名动作冒充 A/B 盲听（`[ARCH-RT-005]` 的可见承诺）。
/// 因此空前置条件下（今天**任何**装配）`[` / `]` 都返回 `false`（不消费）。
///
/// `DeleteSelection` / `ZoomToSelection` / `ZoomToFit` / `Duplicate` 都曾在这张名单里；
/// 它们现在都有落地实现（下面的 `apply_action` 分支），因此**同时**从这张名单与 `cli.rs`
/// 的快捷表 `implemented` 标记里移出 —— B11b（`tests/cli_contract.rs`）把这两处与
/// `--print-shortcuts` 的渲染逐条对账。
/// 注：`ZoomToSelection` / `ZoomToFit` 与 `Duplicate` 的**运行时**语义比"有实现"更细一层 ——
/// 空选区（或选中的身份一个都解析不到）时 `apply_action` 仍返回 `false`（不消费）；
/// 那一条由 `tests/live_ui_mcp.rs` 的端到端判据见证，本名单回答的是"这个动作有没有落点"。
#[must_use]
pub fn action_has_implementation(action: Action) -> bool {
    // ⭐ ⭐ **穷尽匹配（⛔ 不是名字黑名单）** —— 原先写 `!matches!(action, A | B | C)`
    // ⇒ ⭐ 那句是"**名单**"：新增一个 `Action` 变体它**一声不响**，名单外的动作自动被当成
    // "有实现"，哪怕 `apply_action` 根本没接（本仓实测确认：接线前后它都返回 `true`）。
    //
    // ⚠ 穷尽匹配把"**忘了分类**"变成**编译错误**：新变体一加，这里与 `apply_action` 的
    // 穷尽臂**同时**报错 ⇒ 作者必须显式表态。⭐ 这比维护一份可忘记的名单强。
    //
    // ⚠ 与 `apply_action` 的分工：本函数回答"**规范/表上算不算能用**"（`cli.rs` 的
    // `implemented` 列、`--print-shortcuts` 的 `(未实现)` 标记、判据 B11b/B11c 都对它），
    // `apply_action` 回答"**这一键按下去发生什么**"。两者**都必须穷尽**，⛔ 不共享名单。
    // ⚠ **这里仍是"名单"，但它与 `apply_action` 的穷尽臂**分工**：本函数回答
    // "**规范/快捷表上算不算能用**"（`cli.rs` 的 `implemented`、`--print-shortcuts` 的标记、
    // 判据 B11b/B11c 都问它），而"**每个动作按键之后发生什么**"由 `apply_action` 的
    // **穷尽匹配**保证（⭐ 那里⛔ 没有 `_` 通配 ⇒ 新增变体在**编译期**就被逼处理）。
    //
    // ⚠ **为什么本函数不能也穷尽列名**（实测）：`src/elements.rs:2472` 有一条判据要求
    // `Action::PlayPause` / `Action::ResumeFromCursor` 在 `host.rs` 的**代码区恰好各出现一次**
    //（它证明"两键共用同一个臂"）⇒ ⭐ 在这里把它们**再列一遍**会让那条判据红（实测踩过）。
    if let Action::AuditionMain | Action::AuditionProposal | Action::AcceptAiSuggestion = action {
        return false;
    }
    true
}

/// 逻辑键解析出的 [`Action`] → 界面行为的**唯一**落点。
///
/// 判据（`tests/live_ui_mcp.rs` 的判据 16）从无头端口注入逻辑键，观测的就是这里写下的属性。
///
/// ## 为什么有些动作**不消费**
///
/// 没有可作用实现的动作由 [`action_has_implementation`] **同一份陈述**在这里挡下，
/// 返回 `false`（`reject`）。
/// ⭐ **键盘动作可用的引擎/工程上下文**（⭐ **通行形状**，⛔ 不是为节拍器开的旁路）。
///
/// ## 为什么需要它（以及为什么是"通行"的）
///
/// `apply_action` 原先只拿得到 `ui` 与 `undo`，而有一类动作**必须**动引擎：凡是
/// "改 `TransportConfig` 才生效"的开关（节拍器、预备拍，以及将来任何投影进快照的走带
/// 设置）都要**换一代引擎**再恢复走带 ⇒ 那个落点需要 `host` ＋ 一份**工程**。
///
/// ⚠ **借用形状是借用、⛔ 不是拥有**：调用方（[`wire_keys`]）从既有的
/// `Rc<RefCell<EngineHost>>` 现借（⭐ 与 `wire_transport(ui, engine)` **同款**范式）
/// ⇒ ⛔ 零 `unsafe`、⛔ 不造借用延长。
///
/// ⚠ 工程由**调用方持有**并传进来：`EngineHost` **没有** `project` / `transport_config`
/// 访问器（实测 `grep` 命中 0），"当前工程"的真相在装配它的那一层（`main.rs`）。
pub struct EngineCommandContext<'a> {
    /// 引擎宿主（换代与恢复走带都经它）。
    pub host: &'a mut EngineHost,
    /// **当前工程**（动作要基于它算"下一个值"）。
    pub project: &'a YebanProjectV1,
}

/// ⭐ **"要动引擎"的动作的落点**（⭐ **不依赖界面** ⇒ 判据可无窗口地测它）。
///
/// ⚠ 这正是本切片要求的"**通行**形状"：凡是要改 `TransportConfig`（＝必须换一代引擎）
/// 的开关都走这一处；⛔ 界面不参与，⛔ 不为节拍器开特例。
///
/// ⚠ `MainWindow` 需要 Tier-1 平台且 macOS 上 `EventLoop` 是**单例 + 必须主线程**
/// ⇒ 若把这些臂写在 `apply_action` 里、只能经真窗口测，判据就建不出窗口（实测踩过：
/// `EventLoop can't be recreated`）⇒ ⭐ 抽出来是**可测性**的直接要求，不是洁癖。
///
/// # Panics
///
/// ⛔ 不 panic（引擎拒绝 ⇒ 返回 `false`）。
pub fn apply_engine_command(context: &mut EngineCommandContext<'_>, action: Action) -> bool {
    match action {
        // 拨**节拍器**开关：取反当前值。
        Action::ToggleMetronome => {
            let next = !context.project.transport.metronome_enabled;
            crate::metronome::set_enabled(context.host, context.project, next).is_ok()
        }
        // 拨**预备拍**：`count_in_bars` 0 ⇄ 1。
        Action::ToggleCountIn => {
            let next = if context.project.transport.count_in_bars == 0 {
                1
            } else {
                0
            };
            crate::metronome::set_count_in_bars(context.host, context.project, next).is_ok()
        }
        // ⚠ 其余动作**不**在这条链上（它们不动引擎）⇒ 如实返回 `false`。
        _ => false,
    }
}

/// ⭐ **判据用的落点入口**（`pub(crate)` ⇒ ⛔ 不进公共 API、⛔ 不需要文档）。
///
/// ⚠ **它为什么必须 `pub(crate)` 而不是 `pub`**：本仓的 missing-docs 是 `-D warnings`
/// ⇒ 一个 `pub` 函数必须带文档；而 `tests/roll_edit_tools.rs:1231` 又把"**每个文件第一个
/// "测试模块标记"之前的文本**"当作该文件的生产区 ⇒ ⭐ 文档注释**不能**用 `///`
///（那会让它挤在生产区之外的位置上被当作生产代码）。
///
/// ⚠ 本函数**走与产品同一处落点**（[`apply_engine_command`]），⛔ 不是测试旁路。
// ⚠ **不用测试模块标记、改用 `#[allow(dead_code)]`**：本仓的结构判据
// `tests/roll_edit_tools.rs:1258` 在源码里**找那个标记的字面子串**来决定"每个文件的生产区"
// ⇒ ⭐ 在这里写那个标记会**成为边界**、把 `host.rs` 后半段（`ToolDrag` 的真实消费者在
// 5289/5487）划到区外 ⇒ 那两条判据会红（实测踩过，连**注释里的**该字面都会触发）。
// ⚠ 用 `allow` 而**不是** `expect`：本函数只在判据里被调用，生产二进制里确实没有调用者，
// 而 `expect` 在"没触发"时会报错（生产编不过）。
#[allow(dead_code)]
pub(crate) fn apply_action_for_criteria(
    engine: &mut EngineHost,
    project: &YebanProjectV1,
    action: Action,
) -> bool {
    let mut context = EngineCommandContext {
        host: engine,
        project,
    };
    // ⭐ ⚠ **必须走与产品同一处落点**（`apply_engine_command`），⛔ 不是另写一条测试旁路；
    // 界面参数**不参与**（那类臂不碰界面）⇒ 判据无需 Tier-1 窗口。
    apply_engine_command(&mut context, action)
}

fn apply_action(
    ui: &MainWindow,
    undo: Option<&Rc<UndoPort>>,
    context: Option<&mut EngineCommandContext<'_>>,
    action: Action,
) -> bool {
    if !action_has_implementation(action) {
        return false;
    }
    match action {
        // 拨**节拍器**开关（`Cmd/Ctrl+Alt+N`）。⚠ 它更换一代引擎（计划在投影期进快照）
        // ⇒ 走 `metronome::set_enabled` 的"先停 ⇒ 换 ⇒ 恢复位置与状态"。
        // ⚠ 引擎/工程不在手（`context == None`）⇒ ⛔ 不消费（照旧放行，⛔ 不假装成功）。
        Action::ToggleMetronome | Action::ToggleCountIn => {
            let Some(context) = context else { return false };
            apply_engine_command(context, action)
        }
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
        // `Escape` = **收尾**：取消当前手势（`ADR-0004` S1 的纵向拖拽 / `ADR-0005` 的
        // 推子与声相），或关闭全屏时光机（`[UI-A11Y-003]` §7.3）。
        //
        // 五个来源各自独立、各自有唯一清零点，因此这里**都问一遍**（任一命中即消费）：
        // 1. 时光机开着 ⇒ 落 [`UiAction::CloseUndoTree`]（**只改界面运行态**，不碰工程）；
        // 2. 纵向拖拽在手 ⇒ 落 [`cancel_track_height_drag`]（写回起点高度）；
        // 3. 混音手势在手 ⇒ 落窗口回调 `mixer-cancel-gesture` → [`cancel_mixer_gesture`]
        //    （写回起点音量 / 声相）；
        // 4. `[UI-NOTE-003]` 卷帘音符移动手势在手 ⇒ 落 [`cancel_note_drag`]
        //    （**丢弃草稿**；拖动期从未写进工程 ⇒ ⛔ 不提交、⛔ 不重投影）；
        // 5. `[UI-NOTE-003]` 卷帘**力度**手势在手 ⇒ 落 [`cancel_velocity_drag`]
        //    （同上：只丢弃草稿；两个卷帘手势互斥，但两条清零各自独立 ⇒ 谁也不会漏收尾）。
        //
        // 顺序是**决定**，不是巧合：模态面板在最前面，因此"手势还在手"时按 `Escape`
        // 先关时光机（与 `[UI-A11Y-003]` 的模态语义同向）。五条各自独立：关掉时光机
        // **不会**顺带取消手势（用户还要按第二次 `Escape`），没有手势在手也不会把
        // 关闭动作算作"已收尾"。
        //
        // 五条都没有命中时返回 `false`（照旧放行给焦点系统）—— 与 `Escape` 接线之前
        // 的行为逐位相同。这里**没有**新增 `Action` 变体：`Action::Cancel` 本来就在
        // 策略表里（`input.rs` 的 `Key::Escape => Resolution::Action(Action::Cancel)`），
        // 它此前只是没有落地实现（`action_has_implementation` 把它列在未实现里）。
        // 它也不进 `--print-shortcuts` 的表（`cli.rs` 的 18 条里没有 `Escape`），
        // 因此判据 B11b 的三面（渲染 / 解析 / 宿主）一位不动。
        Action::Cancel => {
            // 时光机是模态面板，且 `.slint` 的底部提示行本来就写着 "Esc 关闭" ——
            // 在本次接线之前**没有任何**落点能读到 `UiAction::CloseUndoTree`
            // （全仓只有 `undo.rs` 的定义与自判据），因此那句话在真实界面上是假的。
            // 这里复用按钮那条链：[`UndoPort::perform`] 写运行态，[`refresh_undo_window`]
            // 把同一批显示态属性回写 —— 关闭与打开/切换因此**不可能**分叉。
            // `reproject = false`：关闭不改工程（与 `ToggleUndoTree` 同口径）。
            if let Some(port) = undo
                && port.undo_tree_open()
            {
                port.perform(UiAction::CloseUndoTree);
                refresh_undo_window(ui, port, false);
                return true;
            }
            let height = cancel_track_height_drag(ui, undo);
            let mixer = ui.invoke_mixer_cancel_gesture();
            let note = cancel_note_drag(ui);
            let velocity = cancel_velocity_drag(ui);
            height || mixer || note || velocity
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
        // ⭐ 任务单 `#227`（Logic 的 `Go to Beginning`）：`Return` / `Enter`（裸键）。
        //
        // 落点**只能是界面回调**：`apply_action` 手里没有引擎（`context` 只在
        // `action_needs_engine` 的集合里才给），而"回起点"必须同时把**引擎位置**与
        // **时间码显示**一起改 —— 二者唯一同步的入口就是 `wire_transport` 的那两个回调。
        //
        // 语义（Logic 口径）：
        // - 停住 ⇒ `invoke_stop()` =`Stop` + `SeekTicks(0)`（停住并回到起点）；
        // - 播放中 ⇒ 先 `invoke_stop()`（回起点、停住），再 `invoke_toggle_play()`
        //   （从起点起播）⇒ 命令序列 `Stop` / `SeekTicks(0)` / `Play`，**继续播放**。
        //
        // ⚠ 为什么读 `ui.get_playing()` 而不是引擎：本动作**不在** `action_needs_engine`
        // 集合里（它要调界面回调，进那个集合就是 `#212` 的重入）⇒ 借不到引擎；而
        // `playing` 是 `apply_transport` 从**引擎读数**写下的镜面（本仓唯一写者），
        // 不是界面自己的状态机。`test_port_adapter.rs` 的判据逐状态对账两者相等。
        //
        // ⚠ 位置：**必须**在这条 `Action::PlayPause | Action::ResumeFromCursor` 臂
        // **之前**任意处、且**不得**插进该臂与 `Action::Undo` 之间 —— `elements.rs` 的
        // `space_and_shift_space_share_one_host_arm_today` 用 `Action::Undo` 当那一条臂的
        // 右边界（把本臂插进去会让它把"回起点"的代码当成走带臂的一部分）。
        Action::GoToBeginning => {
            let was_playing = ui.get_playing();
            ui.invoke_stop();
            if was_playing {
                ui.invoke_toggle_play();
            }
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
        // `Cmd`/`Ctrl+D`：**原位复制**当前选区的音符（`[UI-NOTE-003]`）。
        //
        // 语义（四条都是决定, 不是意外）：
        // 1. 选区是**视图态**，事实源是界面属性 `selected-ulids`（与 `DeleteSelection` 同一个读点）
        //    —— 这里读它, 不读第二份状态；
        // 2. **无选区（或选中的身份已不存在）⇒ 什么都不做**，返回 `false`（不吞键）。
        //    与 `DeleteSelection` / `ZoomToSelection` / `Action::Cancel` 同一取向；
        // 3. 一次按键 = **一次** `commit_ops`。`undo_session::commit` 把整批包成**一个**
        //    `Op::Batch` ⇒ 复制 k 个音符**一步撤销**全部撤掉（`ARCH-OPS-002` 的原子性）。
        //    这里**不自己**构造 `Batch`：那会在模型之外长出第二个批次语义；
        // 4. **选区留在原文上**（不迁到副本）。这不是省事, 是选区一致性的要求：
        //    `selected-ulids` 是**视图态**，撤销不回滚它。若把选区写到副本身份上,
        //    一次 `Cmd+Z` 撤掉副本之后选区就指向**不存在的身份** —— 正是
        //    `DeleteSelection` 那一段警告的"选中数与标志分叉"。原文始终存在, 因此选它最稳。
        //
        // 提交后必须重新投影：副本是新身份, 不重投影就是"模型加了、画面没有"。
        Action::Duplicate => {
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
            let ops = duplicate_ops_for(&project, &selected, ROLL_SNAP_GRID_TICKS);
            if ops.is_empty() {
                // 选区的对象一个都不在工程里 ⇒ 没有可复制的对象, 如实不消费。
                return false;
            }
            if port
                .commit_ops(now_ms(), "duplicate selection", ops)
                .is_err()
            {
                return false;
            }
            refresh_undo_window(ui, port, true);
            true
        }
        // `[UI-NOTE-005]` §3.5 项 1–2：方向键把**当前选区**按一档步长平移 / 移调。
        //
        // 语义（五条都是决定, 不是意外）：
        // 1. 选区是**视图态**，事实源是界面属性 `selected-ulids`（与 `DeleteSelection` /
        //    `Duplicate` 同一个读点）—— 这里读它, 不读第二份状态；
        // 2. 幅度由 [`nudge_delta_for`] **唯一**换算（`Grid` → `ROLL_SNAP_GRID_TICKS`、
        //    `Tick` → 1、`Semitone` → 1、`Octave` → 12）；
        // 3. 批解析走 [`nudge_ops_for`]（纯函数）：**组夹紧** + 整块平移；
        // 4. **无选区 / 身份解析不到 / 夹紧后位移为 0 ⇒ 0 提交**，返回 `false`（不吞键）。
        //    与 `DeleteSelection` / `Duplicate` / `ZoomToSelection` 的空前置条件同一取向；
        // 5. 一次按键 = **一次** `commit_ops`。`undo_session::commit` 把整批包成**一个**
        //    `Op::Batch` ⇒ 多音符选区也是一步撤销（`ARCH-OPS-002`）；键重复是**多个**离散
        //    按键事件，因此每个事件各自一步 —— 与拖拽臂"一次手势一步"同构。
        //
        // 提交后必须重新投影：音符挪了位置，不重投影就是"模型动了、画面还在老地方"。
        // 选区**不**改：`Op::MoveNote` 不改身份，因此 `selected-ulids` 仍然有效。
        Action::NudgeSelection { step, direction } => {
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
            let (delta_tick, delta_pitch) = nudge_delta_for(step, direction);
            let ops = nudge_ops_for(&project, &selected, delta_tick, delta_pitch);
            if ops.is_empty() {
                // 没有选区 / 没有可平移的对象 / 整块已在边界上 ⇒ 没有可停靠的编辑, 如实不消费。
                return false;
            }
            if port.commit_ops(now_ms(), "nudge selection", ops).is_err() {
                return false;
            }
            refresh_undo_window(ui, port, true);
            true
        }
        // 横向缩放（`Z` / `Shift+Z`）：**视图态**，不碰 `Op`、不进撤销栈。
        //
        // 语义（三条都是决定）：
        // 1. 输入是**投影自己的读数**：选区跨度走 `ViewState::selection_tick_span`
        //    （`selected-ulids` 解析到投影里的音符），全曲长度走 `ViewState::content_end_tick`
        //    （与铺标尺的那次 `max` 同一口径）—— 宿主**不**再遍历一遍工程去算第二份几何；
        // 2. **没有可作用的对象 ⇒ 不消费**：空选区（或选中的身份已不存在）按 `Z`、
        //    空工程按 `Shift+Z` 都返回 `false`。这与 `DeleteSelection`（空选区返回 `false`）
        //    和 `Action::Cancel`（没有手势返回 `false`）同一取向：吞掉一个没有对象的键，
        //    用户只会看到"这个功能坏了"；
        // 3. 目标值由 [`crate::bridge::ticks_per_pixel_to_fit`] 一次算出（**唯一**的换算 +
        //    夹紧），再走 [`project_with_view_state`] 重新投影 ⇒ 没有第二套几何。
        Action::ZoomToSelection | Action::ZoomToFit => {
            let Some(port) = undo else {
                return false;
            };
            zoom_action(ui, port, action)
        }
        // ⭐ ⭐ **穷尽匹配（⛔ 已删掉原先的 `_ => false` 通配）** —— 它现在是与
        // `action_has_implementation` **同一份陈述**的"哪些动作有落点"的唯一真相源之一。
        //
        // ⚠ **为什么必须穷尽**：原先这里是一个 `_ => false` 通配 ＋ 函数开头的**名字黑名单**
        // ⇒ 两处**都不看** `match` 臂。后果（实测确认）：**新增一个 `Action` 变体时两处都
        // 一声不响**，名单外的动作被当成"有实现"，而臂根本没接。穷尽匹配把"**忘了接**"
        // 变成**编译错误**。
        //
        // ⭐ 下面每一条都**显式**列出并写明**为什么不消费**（⛔ 不用 `_` 吞掉）。
        //
        // ## 已知未实现族（规范要求存在、实现一处都没有）
        Action::AuditionMain | Action::AuditionProposal => {
            // `[ARCH-RT-005]` 的**听觉** A/B 试听：渲染侧有两份可渲染版本，但"按 `[`/`]`
            // 真的听到瞬切"这条**用户可见承诺**还没接 ⇒ 空前置条件下不消费。
            // ⚠ 与 `crate::audition` 的**控制面**区分：那一半（准入/拒绝/回基线）已落地。
            false
        }
        Action::AcceptAiSuggestion => {
            // 界面侧没有"当前待采纳的提案身份"这一表示（提案抽屉的数据面默认全空），
            // 而领域的 `yeban_merge_proposal` 要 `proposalId` ⇒ 缺的是**能力**。
            false
        }
    }
}

/// `Z`（选区撑满视口）/ `Shift+Z`（全曲总览）的**唯一**落点（纯宿主，走投影）。
///
/// 链路（每一步都是既有实现）：
///
/// ```text
/// Action::ZoomToSelection / ZoomToFit
///   ├─ UndoPort::try_project           ← 权威工程（视图态不持有工程）
///   ├─ host::project_with_view_state   ← 当前缩放 + 行高 → 当前投影（读**投影**的读数）
///   │    ├─ ViewState::selection_tick_span(selected-ulids)   （`Z`）
///   │    └─ ViewState::content_end_tick()                    （`Shift+Z`）
///   ├─ bridge::ticks_per_pixel_to_fit  ← ceil(跨度 / 视口像素) + 唯一夹紧
///   └─ host::set_roll_ticks_per_pixel → host::project_with_view_state → host::apply_view
/// ```
///
/// ## 返回值（= 这一键是否被消费）
///
/// | 情形 | 返回 | 理由 |
/// | :--- | :--- | :--- |
/// | 空选区按 `Z` / 空工程按 `Shift+Z` | `false` | 没有可作用的对象（与 `DeleteSelection` 同取向） |
/// | 没有撤销端口 / 端口没有活跃工程 / 视口宽度为 0 | `false` | 没有可投影的输入；不假装处理 |
/// | 已经有这个缩放 | `true` | 对象在、语义已达成；重投影是白费（画面本来就对） |
/// | 真的改了缩放 | `true` | 投影 + 注入都做了 |
///
/// ## 为什么不进撤销栈
///
/// 本函数**不**调用 `UndoPort::commit_ops`，因此 `port.display().undoable` 一位不涨。
/// 撤销是**工程内容**的事（`ADR-0001` 的提交图谱记的是 `Op`）；缩放只是"这一帧画多宽"，
/// 与 `roll-scroll-x` / `track-height-*` 同属视图态。
fn zoom_action(ui: &MainWindow, port: &UndoPort, action: Action) -> bool {
    let Some(project) = port.try_project() else {
        return false;
    };
    // 当前投影：选区身份与内容长度都从它读，不另建索引。
    let Ok(current) = project_with_view_state(&project, ui) else {
        return false;
    };
    let span = match action {
        Action::ZoomToSelection => {
            let selected: Vec<String> = {
                use slint::Model as _;
                ui.get_selected_ulids()
                    .iter()
                    .map(|id| id.to_string())
                    .collect()
            };
            current.selection_tick_span(&selected)
        }
        // 全曲总览：从 tick 0 到内容末端。内容为 0 ⇒ 没有可看的东西 ⇒ 不消费。
        _ => {
            let end = current.content_end_tick();
            (end > 0).then_some((0, end))
        }
    };
    let Some((start_tick, end_tick)) = span else {
        return false;
    };
    // 视口宽度：与 `apply_view` / 裁剪用的是**同一个**读数（窗口逻辑宽度）。
    // `slint::Window::size()` 的 `width` 是 `u32`（`PhysicalSize`），因此不需要转换检查。
    let viewport_px = slint::ComponentHandle::window(ui).size().width;
    let Some(target) =
        crate::bridge::ticks_per_pixel_to_fit(end_tick.saturating_sub(start_tick), viewport_px)
    else {
        return false;
    };
    if target == current.ticks_per_pixel {
        // 已经在这个缩放上：对象在、语义已达成 ⇒ 消费，但不重投影。
        return true;
    }
    if !set_roll_ticks_per_pixel(ui, target) {
        // `target != current.ticks_per_pixel` 时不该发生；如实不消费而不是假装改了。
        return false;
    }
    let Ok(next) = project_with_view_state(&project, ui) else {
        return false;
    };
    // 选区总览要**看得见选区**：把它的左沿滚到视口左沿。换算走 `tick_to_px`
    // （与投影同一个整数除法），不新写第二套。
    let scroll_x = if action == Action::ZoomToSelection {
        crate::bridge::tick_to_px(start_tick, target).map_or(0.0, |px| px as f32)
    } else {
        0.0
    };
    apply_view(ui, &next, viewport_px as f32, scroll_x);
    true
}

/// 把**撤销的**模型读数注入界面（显示态 + 时光机弹窗开关 + 图谱节点）。
///
/// 与 [`apply_transport`] 同一条纪律：界面**不自己算**"能不能撤销"。
/// 唯一的事实源是 [`UndoPort::display`]（= `undo_session::UndoDisplay`：
/// `CommitGraph` + `UndoCursor` 的读数）。
///
/// 时光机画的那排节点同样来自**权威图谱**（[`UndoPort::graph`]）—— 见
/// [`undo_tree_projection`]。界面里**没有**任何自带的节点（`ui/dialogs/
/// undo_tree_modal.slint` 的两个数据面属性默认是 `0` / `[]`）：本函数是它们唯一的写者。
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
    // 图谱节点 = 真实提交链（同一个权威图谱的读数；见 `undo_tree_projection`）。
    let (node_labels, node_hidden) = undo_tree_projection(port);
    ui.set_undo_node_count(clamp(node_labels.len()));
    ui.set_undo_node_labels(strings(&node_labels));
    ui.set_undo_node_hidden(clamp(node_hidden));
}

/// 时光机画布能画下的**最多**节点数。
///
/// 这是**布局常量**，不是数据：面板宽 `720px`、节点自 `x = space-6 (24px)` 起、
/// 间距 `108px`、节点宽 `100px` ⇒ `24 + 108 × 5 + 100 = 664 ≤ 720`；第 7 个节点
/// （`x = 772`）会被面板裁掉。`ui/dialogs/undo_tree_modal.slint` 的边循环用同一个
/// 数字（那里写成字面量 `6`，因为 Slint 侧引用不到 Rust 常量）。
pub const UNDO_TREE_MAX_NODES: usize = 6;

/// 时光机图谱的**真实**节点标签，以及**被画布截掉**的版本数。
///
/// 数据来源是**权威提交图谱**（[`UndoPort::graph`]）—— 不是界面自己算的，
/// 也不是第二份状态：
///
/// 1. `CommitGraph::ancestry(head)` 给出活跃分支的祖先链 `[head, parent, …, root]`
///    （**最新的在前**）；
/// 2. 每个节点的标签 = 那条 `Commit` 的 `message`（提交时给出的原话）；
/// 3. 只取最近 [`UNDO_TREE_MAX_NODES`] 个（画布放不下更多），**其余如实计数**交给
///    界面报出来 —— 不静默截断。
///
/// 只调**一次** `UndoPort::graph()`（它返回一份 `CommitGraph` 拷贝，代价 `O(提交数)`），
/// 因此本函数不引入第二次拷贝。
fn undo_tree_projection(port: &UndoPort) -> (Vec<String>, usize) {
    let graph = port.graph();
    // `head` 为 `None` = 图谱还没有头（例如权威侧此刻没有活跃工程）⇒ 一个真实节点都没有。
    let Some(head) = port.display().head else {
        return (Vec::new(), 0);
    };
    let Ok(chain) = graph.ancestry(&head) else {
        return (Vec::new(), 0);
    };
    let labels = chain
        .iter()
        .take(UNDO_TREE_MAX_NODES)
        .map(|id| {
            graph
                .commits
                .get(id)
                .map_or_else(String::new, |commit| commit.message.clone())
        })
        .collect();
    (labels, chain.len().saturating_sub(UNDO_TREE_MAX_NODES))
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

/// `[UI-NOTE-003]` §3.3 工具矩阵的**点击**接线（台账 R16）。
///
/// 修复的是这一条真缺陷：`ui/console/piano_roll.slint` 的五个工具按钮原先声明了
/// `accessible-role: button`，但既没有 `TouchArea` 也没有回调 ⇒ 用户点它**没有任何反应**
/// （`active-tool` 只由键盘写）。
///
/// ## 为什么这里不重复键盘的落点
///
/// 数字键那条链是 `wire_keys` → `input::resolve_logical` → [`apply_action`] 的
/// `Action::SelectTool` 臂。本函数把那**同一个**臂当作唯一落点：界面报上来的行号经
/// [`Tool::from_digit`] 还原成 [`Tool`]，随后调用 [`apply_action`]。
/// 因此"点了哪个工具"与"按了哪个数字键"在宿主侧**不可能**分叉
/// （动作日志 `UiAction::SelectTool` 与 `ui.set_active_tool` 都发生在那一个臂里）。
///
/// 不在 1..5 内的行号**什么都不做**（`Tool::from_digit` 返回 `None`）—— 界面上没有
/// 第六个按钮，若真出现就说明是第二个真相源，宁可不动也不猜一个工具。
///
/// 端口可有可无：`None` 时照旧写界面属性（与 [`apply_action`] 的口径一致 —— 没有撤销
/// 会话不等于工具不可选）。
pub fn wire_tool_select(ui: &MainWindow, undo: Option<Rc<UndoPort>>) {
    let weak = slint::ComponentHandle::as_weak(ui);
    ui.on_tool_selected(move |tool_index: i32| {
        let Some(ui) = weak.upgrade() else {
            debug_assert!(false, "MainWindow 在回调执行期间被销毁");
            return;
        };
        // 界面报的是行号（1..5）；出界的行号不是工具 ⇒ 拒绝，不猜。
        let Ok(digit) = u8::try_from(tool_index) else {
            return;
        };
        let Some(tool) = crate::input::Tool::from_digit(digit) else {
            return;
        };
        // 与键盘**同一个**臂（唯一写者）：不在这里复制 `set_active_tool` 或动作日志。
        let _ = apply_action(&ui, undo.as_ref(), None, Action::SelectTool(tool));
    });
}

/// ⭐ 铅笔拒绝写入时给用户看的**逐字原因**（`#166`）：展示轨上没有覆盖该位置的片段。
///
/// 为什么必须有一句：旧行为会**静默**回退到"第一条有片段覆盖该 tick 的轨" ⇒ 用户看到的
/// 面板说的是 A 轨、铅笔写到 B 轨。本单改成"只认展示轨"，于是"展示轨没覆盖"这条拒绝
/// 必须说出来（⛔ 不是静默丢弃、⛔ 不是回退到别的轨）。
pub const PENCIL_NO_CLIP_ON_DISPLAYED_TRACK_REASON: &str =
    "当前显示的轨上没有覆盖该位置的片段，未写入音符";

/// 铅笔拒绝的逐字原因：点击位置吸附不出计划（不在卷帘的音高范围内）。
pub const PENCIL_NO_PLAN_REASON: &str = "该位置不在卷帘的音符范围内，未写入音符";

/// 铅笔拒绝的逐字原因：工程里没有可显示的音轨（与 [`crate::bridge::display_track`] 同一条规则）。
pub const PENCIL_NO_DISPLAYED_TRACK_REASON: &str = "工程里没有可显示的音轨，未写入音符";

/// 铅笔拒绝的逐字原因：工程无法投影（拍号 / 缩放退化）。
pub const PENCIL_NO_PROJECTION_REASON: &str = "工程无法投影（拍号非法），未写入音符";

/// 铅笔点击被拒绝的**结构化**原因（纯函数返回值 ⇒ 界面能选一句逐字文本，⛔ 不靠猜）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PencilRefusal {
    /// 当前工具不是铅笔（数字键 2）—— 这次"拒绝"不是用户手势的失败，⛔ 不产生可见文本。
    NotPencil,
    /// 工程投影失败（拍号 / 缩放退化）。
    NoProjection,
    /// 点击位置吸附不出计划（不在卷帘的音高范围内）。
    NoPlan,
    /// 工程里没有可显示的音轨。
    NoDisplayedTrack,
    /// ⭐ 展示轨上没有覆盖该 tick 的片段（⛔ 不回退到别的轨）。
    NoClipOnDisplayedTrack,
}

impl PencilRefusal {
    /// 给用户看的逐字原因；`None` = 这次拒绝不该产生用户可见文本（工具门）。
    #[must_use]
    pub const fn reason(self) -> Option<&'static str> {
        match self {
            Self::NotPencil => None,
            Self::NoProjection => Some(PENCIL_NO_PROJECTION_REASON),
            Self::NoPlan => Some(PENCIL_NO_PLAN_REASON),
            Self::NoDisplayedTrack => Some(PENCIL_NO_DISPLAYED_TRACK_REASON),
            Self::NoClipOnDisplayedTrack => Some(PENCIL_NO_CLIP_ON_DISPLAYED_TRACK_REASON),
        }
    }
}

/// `[UI-NOTE-003]` 卷帘的**编辑入口**（目前只接铅笔）：把一次点击变成**可撤销**的模型操作。
///
/// 规则（账本第 234/237 轮；⭐ 任务单 `#166` 补上**轨**这一半）：音符进"**点击 tick 所在片段**"、
/// 落在"**设备架 / 界面正在显示的那条轨**"—— 即 [`crate::bridge::device_rack_track`] 的**同一条**
/// 规则（身份序第一条非主总线轨；⭐ `#192` 起这条规则降级为**兜底**，主语由
/// [`crate::bridge::display_track`] 解析成"选中轨 or 兜底"）。⭐ 展示轨上**没有**覆盖该 tick 的片段 ⇒ **拒绝**：
/// ⛔ 不回退到别的轨（那正是 `#166` 的缺陷：多轨时写到 B 轨）、⛔ 不新建片段。
/// 任务单 `#164` 的账本把"现在选**第一条有片段覆盖的 tick**、⛔ 不是设备架展示轨"登记为待修项
/// ⇒ 本单把**两套规则并成一套**（设备架 / 发送条 / 面板 / 铅笔共用 `display_track`）。
///
/// 撤销与提交图由 `commit_ops` 提供, 无需另写反向逻辑。
/// `grid` 取 1/16（240 tick）作为当前吸附口径 —— 吸附设置将来若可配, 它应来自配置而不是这里。
/// 把一次点击解析成**待提交的操作**，纯函数以便判据（`wire_roll_edit` 只负责把它接到端口）。
///
/// 返回 `None` 的四种情形都是**决定**而非意外（第 234/237 轮 ＋ `#166`）：工具不是铅笔；
/// 工程投影失败；点击处吸附不出计划；⭐ **展示轨上没有覆盖该位置的片段**。返回 `Some(op)`
/// 表示"应当提交这一个操作"。
///
/// **帧口径**：`x` / `y` 是**宿主帧**（网格局部，与 [`ViewState::hit_test_visible`] /
/// `lane_at_y` 同一口径）。界面回调报的是**绘制帧** ⇒ 唯一派发器先经
/// [`crate::bridge::painted_to_host`] 换算再调这里（⛔ 本函数内部不再做第二次换算）。
#[must_use]
pub fn pencil_op_for(
    project: &yeban_model::YebanProjectV1,
    scroll_x: f32,
    x: f32,
    y: f32,
    grid_ticks: u64,
    active_tool: i32,
) -> Option<yeban_model::ops::Op> {
    pencil_attempt_for(project, scroll_x, x, y, grid_ticks, active_tool).ok()
}

/// 同 [`pencil_op_for`]，但把**拒绝的原因**结构化返回 —— 界面据此写可见反馈（`#166`）。
///
/// 它是铅笔寻址的"无选中态"入口：转发到 [`pencil_attempt_for_selected`] 并传
/// `selected = None`（⇒ 兜底轨 = 旧行为），因此既有判据的语义一位不变。
///
/// # Errors
///
/// 见 [`PencilRefusal`]：工具门 / 投影失败 / 吸附不出计划 / 没有展示轨 / ⭐ 展示轨无覆盖。
pub fn pencil_attempt_for(
    project: &yeban_model::YebanProjectV1,
    scroll_x: f32,
    x: f32,
    y: f32,
    grid_ticks: u64,
    active_tool: i32,
) -> Result<yeban_model::ops::Op, PencilRefusal> {
    pencil_attempt_for_selected(project, None, scroll_x, x, y, grid_ticks, active_tool)
}

/// ⭐ 任务单 `#192`：铅笔寻址的**唯一**实现 —— 轨由 [`crate::bridge::display_track`]
/// （选中轨 or 兜底）决定，与设备架 / 发送条 / 轨路由**同一次解析**。
///
/// `selected` 是窗口属性 `selected-track-id` 的原文；`None` / 空串 / 已不存在的身份 /
/// 主总线都回落到兜底（身份序第一条非主总线轨），⛔ 不回退到"第一条有片段覆盖"的轨
/// —— 那正是 `#166` 的缺陷（面板说 A、铅笔写 B）。
///
/// # Errors
///
/// 见 [`PencilRefusal`]。
pub fn pencil_attempt_for_selected(
    project: &yeban_model::YebanProjectV1,
    selected: Option<&str>,
    scroll_x: f32,
    x: f32,
    y: f32,
    grid_ticks: u64,
    active_tool: i32,
) -> Result<yeban_model::ops::Op, PencilRefusal> {
    if active_tool != i32::from(PENCIL_TOOL_DIGIT) {
        return Err(PencilRefusal::NotPencil);
    }
    // 这里的命中测试**只**看卷帘的 14px 音高泳道（`lane_at_y`）与 tick 落在哪个片段，
    // 与编排行高无关：行高变高/变矮**不**改这条路径的答案（因此 S1 不给它加布局参数
    // —— 一个不影响结果的参数是假依赖）。
    let view = ViewState::from_project(project).map_err(|_| PencilRefusal::NoProjection)?;
    let plan = view
        .pencil_plan(scroll_x, x, y, grid_ticks)
        .ok_or(PencilRefusal::NoPlan)?;
    // ⭐ 任务单 `#166` 的**唯一**改动点：轨由展示轨决定（设备架 / 发送条 / 面板正在显示的
    // 那一条），⛔ 不再遍历 `project.tracks` 找"第一条有片段覆盖"的轨。
    // ⭐ 任务单 `#192`：展示轨 = **选中轨 or 兜底**（`display_track`，与投影同一份实现）。
    let track =
        crate::bridge::display_track(project, selected).ok_or(PencilRefusal::NoDisplayedTrack)?;
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
    let clip_id = crate::bridge::clip_at_tick(&triples, plan.start_tick)
        .ok_or(PencilRefusal::NoClipOnDisplayedTrack)?;
    Ok(crate::bridge::plan_to_add_note(
        plan,
        track.id,
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
/// `#215` 起实现委托给 [`yeban_model::project::now_unix_ms`]：全仓只有**那一份**
/// `SystemTime::now()`，新建工程的创建时刻与提交时间戳因此同源、不会各走一套夹取口径。
pub fn now_ms() -> u64 {
    yeban_model::project::now_unix_ms()
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

/// `[UI-NOTE-003]` 剪刀的**左键单击**：把指针下那**一颗**音符沿吸附网格竖线切成两半
/// （纯函数, 有判据）。
///
/// # 形状（四条都是决定）
///
/// 1. **指针范围**：只切派发器**已经命中的那一颗**（`hit_test_visible`）—— ⛔ 不是"查询所有跨
///    过某条网格竖线的音符"（那需要一条新的 tick-span 查询, 本切片不引入第二条命中路径）；
/// 2. **吸附 tick 由调用方给**：它来自 [`crate::bridge::ViewState::snapped_tick_at`]（唯一真相源）,
///    本函数⛔ 不自己再做一次像素 → tick 换算；
/// 3. **两个新身份、原文被删**：与 [`duplicate_ops_for`] 的副本形状同款
///    （`copy.id = EntityId::new()`）—— 原文的 id **不**留给左半, 因此本函数**不需要任何新
///    `Op` 变体**（`DeleteNote` ＋ 两个 `AddNote` 已能表达）；
/// 4. **一次提交 = 一步撤销**：返回 `DeleteNote` ＋ `AddNote(左)` ＋ `AddNote(右)` **一个批**,
///    `undo_session::commit` 把整批包成一条 `Op::Batch`（`ARCH-OPS-002`）。
///
/// # 两半恰好铺满原文（`[start, split)` ＋ `[split, end)`）
///
/// 左半 `start_tick = 原文 start`、`duration_ticks = split − start`；右半 `start_tick = split`、
/// `duration_ticks = end − split`。`pitch` / `velocity` 及其余表现力字段由 `clone` 逐位继承。
///
/// # 拒绝退化切分（返回**空 `Vec`** ⇒ 调用方什么都不做）
///
/// 吸附 tick **不严格落在**音符内部（`split <= start || split >= end`）时拒绝。这一条同时排除
/// 了两个"零时长半边"：严格不等式保证 `duration_ticks > 0`, 而模型**只**拒绝零时长
/// （`music.rs` 的 `ZeroDuration`）。身份非法 / 解析不到 / 工程无轨道 ⇒ 也返回空。
///
/// ⚠️ `AddNote` 的前置条件**没有**片段边界检查（`ops.rs` 只查轨道存在 ＋ `note.validate()` ＋
/// 重复身份）—— 但本函数把两半都夹在**原文自己的跨度内**, 因此切分不会把音符送出新边界。
#[must_use]
pub fn split_ops_for(
    project: &yeban_model::YebanProjectV1,
    note_ulid: &str,
    split_tick: u64,
) -> Vec<yeban_model::ops::Op> {
    let Ok(note_id) = note_ulid.parse::<yeban_model::EntityId>() else {
        return Vec::new();
    };
    // 身份解析走**唯一**的 [`note_identity_for`]（与移动 / 力度臂同一口径）。
    let Some((clip_id, track_id, _velocity)) = note_identity_for(project, note_id) else {
        return Vec::new();
    };
    let Ok(note) = project.note(&clip_id, &note_id) else {
        return Vec::new();
    };
    let start = note.start_tick;
    let Some(end) = start.checked_add(note.duration_ticks) else {
        return Vec::new();
    };
    // 严格不等式：吸附点必须落在音符**内部** ⇒ 两半的时值都 > 0（模型只拒绝零时长）。
    if split_tick <= start || split_tick >= end {
        return Vec::new();
    }
    let mut left = note.clone();
    left.id = yeban_model::EntityId::new();
    left.start_tick = start;
    left.duration_ticks = split_tick - start;
    let mut right = note.clone();
    right.id = yeban_model::EntityId::new();
    right.start_tick = split_tick;
    right.duration_ticks = end - split_tick;
    vec![
        yeban_model::ops::Op::DeleteNote {
            track_id,
            clip_id,
            note_id,
            // 自带撤销载荷：撤销不需要回放历史（`ops.rs` 的设计约束 2）。
            previous_note: note.clone(),
        },
        yeban_model::ops::Op::AddNote {
            track_id,
            clip_id,
            note: left,
        },
        yeban_model::ops::Op::AddNote {
            track_id,
            clip_id,
            note: right,
        },
    ]
}

/// `[UI-NOTE-003]` 把"选中的音符身份"解析成待提交的 **`Op::AddNote` 副本集合**（纯函数, 有判据）。
///
/// 返回**空 `Vec`** 的两种情形都是**决定**：选区为空；选中的身份在工程里一个都解析不到
/// （例如它们已被上一次删除移除）—— 那时调用方**什么都不做**（`apply_action` 返回 `false`,
/// 与 `DeleteSelection` / `ZoomToSelection` 同一取向）。
///
/// ## 副本的形状（三条都是决定）
///
/// 1. **新身份**：每条副本取一个 [`yeban_model::EntityId::new`]（内部是
///    `ulid::Ulid::generate`）。复用原文的身份会被 `Op::AddNote::precondition` 以
///    `ModelError::DuplicateEntityId` 拒绝（`crates/yeban-model/src/ops.rs` 的前置条件 /
///    `crates/yeban-model/src/project.rs` 的 `insert_note`）—— 这是 `AddNote` 唯一的冲突判据。
/// 2. **右移一个吸附网格**（`grid_ticks`）：`start_tick.saturating_add(grid_ticks)`。
///    音高 / 时值 / 力度与全部表现力字段逐项沿用原文（`MidiNote` 的 `clone`, 只改
///    `id` 与 `start_tick` 两处）。取"**一个网格**"而不是"一个时值"：选区因此**保持内部
///    相对形状**（整块平移）, 且网格值已经有唯一真相源（`ROLL_SNAP_GRID_TICKS`）,
///    不新增第二个几何常量。
/// 3. **落在原文所在的片段里**（同一个 `clip_id`）。音符住在**片段池**里, 不住在轨道上；
///    选区跨片段 / 跨轨时**每条副本跟着自己的原文**, 不跨片段搬运。`track_id` 只用于
///    `AddNote::precondition` 的"这条轨道存在吗"检查, 取**摆放了该片段的第一条轨道**
///    （键序确定）；一个谁都没摆放的片段池条目回退到工程的**第一条轨道**;
///    工程一条轨道都没有 ⇒ 跳过该音符（与 [`delete_ops_for`] 同一口径）。
///
/// ⚠️ **重叠是允许的**：`AddNote` 的前置条件只拒绝**重复身份**, 不拒绝同 tick / 同音高的
/// 既有音符。因此右移一格压到别的音符（或压到原文自己）上**不会失败**, 不需要夹紧。
///
/// ⚠️ **`grid_ticks == 0` 时副本与原文完全重叠**（同 tick 同音高, 只有身份不同）。
/// 生产路径传的是 [`ROLL_SNAP_GRID_TICKS`]（240）, 因此这条只在调用方显式传 0 时成立。
///
/// 为什么一次返回**一批**而不是一个：与 [`delete_ops_for`] 逐字同理 ——
/// `undo_session::commit` 把整批包成一个 `Op::Batch`, 于是"一次按键 = 一次提交 = 一步撤销"
/// 是构造上的（`ARCH-OPS-002`）。逐个提交会让复制 5 个音符需要按 5 次 `Cmd+Z` 才回来。
#[must_use]
pub fn duplicate_ops_for(
    project: &yeban_model::YebanProjectV1,
    selected_ulids: &[String],
    grid_ticks: u64,
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
            let mut copy = note.clone();
            copy.id = yeban_model::EntityId::new();
            copy.start_tick = note.start_tick.saturating_add(grid_ticks);
            ops.push(yeban_model::ops::Op::AddNote {
                track_id,
                clip_id: *clip_id,
                note: copy,
            });
        }
    }
    ops
}

/// `[UI-NOTE-005]` §3.5 项 1–2：一档**步长 + 方向** ⇒ `(delta_tick, delta_pitch)`（唯一换算）。
///
/// 幅度的事实源只有这一处：`Grid` 取 [`ROLL_SNAP_GRID_TICKS`]（与铅笔 / `Cmd+D` 同一个吸附网格
/// 常量），`Octave` 取 12 —— 规范逐字写"±12 半音"，因此这里用**字面量 12** 而不是
/// `bridge::PITCH_LANE_COUNT / 12` 之类的巧合等式（那会是第二个真相源）。
///
/// 步长决定**轴**：`Grid`/`Tick` 只动 tick，`Semitone`/`Octave` 只动音高 ⇒ 不存在
/// "两边都改"的组合，也不存在 `direction` 之外的自由度。
#[must_use]
pub fn nudge_delta_for(step: crate::input::NudgeStep, direction: i8) -> (i64, i8) {
    use crate::input::NudgeStep;
    let sign = i64::from(direction.signum());
    match step {
        NudgeStep::Grid => (
            sign * i64::try_from(ROLL_SNAP_GRID_TICKS).unwrap_or(i64::MAX),
            0,
        ),
        NudgeStep::Tick => (sign, 0),
        NudgeStep::Semitone => (0, direction.signum()),
        NudgeStep::Octave => (0, direction.signum() * 12),
    }
}

/// `[UI-NOTE-005]` §3.5 项 1–2：把"选中的音符身份 + 一档位移"解析成待提交的
/// **`Op::MoveNote` 集合**（纯函数, 有判据）。
///
/// # 作用域 = **选区**（不是指针下的一个音符）
///
/// 规范项 1 逐字写"平移**选区**音符"。这与既有的拖拽臂是**两件事**：`begin_note_drag` /
/// `commit_note_drag` 只持**一个** `note-drag-ulid`（指针下那一颗），所以键盘这一路
/// 不可能复用拖拽臂的作用域 —— 它是本动作新增的那一半。身份解析与
/// [`delete_ops_for`] / [`duplicate_ops_for`] **逐条同口径**（`clip_pool` → 摆放它的第一条轨道，
/// 键序确定；选区为空 / 身份一个都解析不到 ⇒ 返回空 `Vec`）。
///
/// # 整块平移 + **组夹紧**（两条都是决定, 不是意外）
///
/// 1. **一个公共位移**：选区里所有音符用**同一对** `(delta_tick, delta_pitch)` ⇒ 选区保持
///    内部相对形状（和弦不会散架）。口径与 [`duplicate_ops_for`] 的"取**一个网格**而不是
///    一个时值 ⇒ 整块平移"是同一条理由；
/// 2. **组夹紧**：公共位移先被**夹到所有目标音符都合法**的那个区间
///    （`start_tick + Δ ≥ 0`；`0 ≤ pitch + Δ ≤ 127`）。⛔ 不是"逐个音符各自夹"——
///    那会让碰到边界的音符留在原地、其余音符继续走，从而**改变选区的相对形状**。
///    夹紧之后位移为 `(0, 0)`（例如整个选区的最左音符已在 tick 0 时按 `←`）⇒ 返回
///    **空 `Vec`**：调用方什么都不做，**不写空 op**（与 `velocity_op_for` 的"没变 ⇒ `None`"
///    同一取向）。
///
/// 夹紧是**必要**的：`Op::MoveNote` 的前置条件对越界是**拒绝**（`shifted_tick` / `shifted_pitch`
/// 为 `None` ⇒ `ModelError::OpStateMismatch`），而 `undo_session::commit` 把整批包成
/// **一个** `Op::Batch`、整批一起校验。逐音符各自算一份增量会让"有一个音符碰边界"变成
/// 整批被拒（一次按键什么都不发生），组夹紧则让它变成"整块走不动那么远"—— 后者是语义，
/// 前者是失败。
///
/// # 为什么一次返回一批
///
/// 与 [`delete_ops_for`] 逐字同理：`undo_session::commit` 把整批包成**一个** `Op::Batch`
/// ⇒ "一次按键 = 一次提交 = 一步撤销"是构造上的（`ARCH-OPS-002`）。单音符选区时这一批里
/// 就只有**一条** `Op::MoveNote`。
///
/// ⚠️ 本函数**不**改 `note.duration_ticks` / `velocity` / 身份：移动只动
/// `start_tick` 与 `pitch`（`Op::MoveNote` 的载荷形状），因此
/// "`MidiNote::duration_ticks != 0`" 这条模型不变量不受影响（本函数不碰时值）。
#[must_use]
pub fn nudge_ops_for(
    project: &yeban_model::YebanProjectV1,
    selected_ulids: &[String],
    delta_tick: i64,
    delta_pitch: i8,
) -> Vec<yeban_model::ops::Op> {
    use std::collections::{BTreeMap, BTreeSet};
    // 身份集合：`BTreeSet` 迭代确定（红线 4），顺带自动去重（与 `delete_ops_for` 同款）。
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
    // 第一遍：把目标音符按**确定顺序**收集成 `(clip, track, note)`，并算组夹紧所需的极值。
    let mut hits: Vec<(
        yeban_model::EntityId,
        yeban_model::EntityId,
        yeban_model::EntityId,
        u64,
        u8,
    )> = Vec::new();
    let mut min_tick = u64::MAX;
    let mut max_tick = 0_u64;
    let mut min_pitch = u8::MAX;
    let mut max_pitch = 0_u8;
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
            min_tick = min_tick.min(note.start_tick);
            max_tick = max_tick.max(note.start_tick);
            min_pitch = min_pitch.min(note.pitch);
            max_pitch = max_pitch.max(note.pitch);
            hits.push((*clip_id, track_id, *note_id, note.start_tick, note.pitch));
        }
    }
    if hits.is_empty() {
        return Vec::new();
    }
    // 第二遍：**组夹紧** —— 一个公共位移，夹到所有目标都合法的区间。
    let feasible_tick = clamp_tick_delta(min_tick, max_tick, delta_tick);
    let feasible_pitch = clamp_pitch_delta(min_pitch, max_pitch, delta_pitch);
    if feasible_tick == 0 && feasible_pitch == 0 {
        // 整块走不动 ⇒ **不提交空 op**（调用方据此返回 `false`, 不吞这一键）。
        return Vec::new();
    }
    hits.into_iter()
        .map(
            |(clip_id, track_id, note_id, _, _)| yeban_model::ops::Op::MoveNote {
                track_id,
                clip_id,
                note_id,
                delta_tick: feasible_tick,
                delta_pitch: feasible_pitch,
            },
        )
        .collect()
}

/// tick 增量的**组夹紧**：把 `delta` 收到"`min_tick + Δ ≥ 0` 且 `max_tick + Δ ≤ u64::MAX`"。
///
/// `i128` 中间量 ⇒ 不依赖 `u64::MAX + 1` 之类的溢出行为（与 `ops.rs` 的 `shifted_tick`
/// 同一手法）。结果为 0 = 这一档在 tick 轴上一步都走不动。
#[must_use]
fn clamp_tick_delta(min_tick: u64, max_tick: u64, delta: i64) -> i64 {
    if delta < 0 {
        // 下界：最左那个音符不许掉到 0 以下。
        let lowest = i128::from(min_tick) + i128::from(delta);
        if lowest >= 0 {
            return delta;
        }
        // `min_tick < -delta ≤ i64::MAX` ⇒ 取负一定落在 `i64` 里。
        return i64::try_from(-i128::from(min_tick)).unwrap_or(0);
    }
    if delta > 0 {
        let room = u128::from(u64::MAX) - u128::from(max_tick);
        let want = u128::try_from(delta).unwrap_or(u128::MAX);
        if want <= room {
            return delta;
        }
        return i64::try_from(room).unwrap_or(0);
    }
    0
}

/// 音高增量的**组夹紧**：把 `delta` 收到"`min_pitch + Δ ≥ 0` 且 `max_pitch + Δ ≤ 127`"。
///
/// 区间恒非空（`min_pitch ≤ max_pitch` 且都在 `0..=127`），且落在 `i8` 范围内。
#[must_use]
fn clamp_pitch_delta(min_pitch: u8, max_pitch: u8, delta: i8) -> i8 {
    let lower = -i16::from(min_pitch);
    let upper = 127 - i16::from(max_pitch);
    let clamped = i16::from(delta).clamp(lower, upper);
    i8::try_from(clamped).unwrap_or(0)
}

/// `[UI-NOTE-003]` 把卷帘的点击接到**撤销端口**上（工具矩阵的编辑语义）。
///
/// 它**不**自己注册回调：唯一注册点是 [`install_roll_click`]（`ui.on_clicked(` 在 `src/**`
/// 只允许一个写者）。本函数只做一件事 —— 把端口交给**同一个**派发器**重新**装一次。
/// 因为派发器**总是**先做命中与选区更新，这次重装是"铅笔/橡皮擦到位"的**超集**，
/// 而不是覆盖掉选区。
///
/// ## 它修掉的缺陷（为什么必须是一个派发器）
///
/// Slint 的一个 `callback` **只留一个 handler**（`i-slint-core-1.18.1/src/callbacks.rs`：
/// "There can only be one single handler per callback."）。旧实现有两个写者：
/// `build_main_window_with_console_tab` 注册选区、本函数再注册一个**只认铅笔**的 handler
/// ⇒ 后者把前者整个换掉。于是默认选择工具下点音符**没有任何反应**，全部依赖选区的动作
/// （`Delete` / `Cmd+D` / `Z`）在人手这一侧不可达。
///
/// 端口此刻没有活跃工程（`try_project` 如实返回 `None`）时**不重装**：
/// `build_main_window*` 装好的只做选区的派发器不需要端口，保留它比装一个连命中视图都没有的
/// 回调更诚实。
pub fn wire_roll_edit(ui: &MainWindow, port: &Rc<UndoPort>) {
    let Some(project) = port.try_project() else {
        return;
    };
    let Ok(view) = ViewState::from_project(&project) else {
        return;
    };
    let view = Rc::new(view);
    install_roll_click(ui, Rc::clone(&view), Some(Rc::clone(port)));
    // `[UI-NOTE-003]` 左键**拖拽**的两条臂（移动 / 力度）：与点击**同一个**装配点
    // （本函数是卷帘编辑面唯一的接线函数）⇒ 产品进程（`main.rs`）与无头面
    // （`live_surface.rs`）不可能只接一半。
    install_roll_note_drag(ui, view, Some(Rc::clone(port)));
}

/// `[UI-NOTE-003]` 卷帘点击的**唯一派发器**（`src/**` 里 `ui.on_clicked(` 只有这一处写者）。
///
/// # 派发顺序（顺序本身是契约）
///
/// 0. **先做一次帧换算**：界面报的是绘制坐标（相对 `PianoRoll` 根），宿主命中测试用的是
///    网格局部坐标 ⇒ `bridge::painted_to_host`（唯一真相源）搬一次，下面的 `(x, y)` 全部是
///    宿主帧；
/// 1. **总是**命中测试并更新选区（写 `selected-note-count` / `selected-ulids` /
///    `note-selected`，并同步状态栏）—— 无论当前是哪个工具。选择工具到此为止；
/// 2. 再按 `Tool::click()` 落编辑：
///    - `DrawNote` ⇒ [`pencil_attempt_for`] + `commit_ops`（`pencil: add note`）；拒绝 ⇒
///      不提交，并把 [`PencilRefusal::reason`] 的逐字原因写进 `roll-edit-status`；
///    - `DeleteNote` ⇒ 用刚命中的身份经 [`delete_ops_for`] + `commit_ops`
///      （`eraser: delete note`，与键盘 `DeleteSelection` **同一个**批解析点），
///      并清空已不存在的选区；
///    - `SelectOrClear`、尚未接线的剪刀、以及力度工具的**单击** ⇒ 什么都不做；
///    - 力度工具的**双击**（`double_click == true`，来自 `.slint` 的 `double-clicked`）⇒
///      把刚命中的那颗音符恢复成 `music::DEFAULT_VELOCITY`（**一个**原子
///      `Op::ModifyNoteVelocity`，已经是 100 则不写）。手势是**音符**范围的：最需要它的
///      0 力度音符柱高 0px、[`velocity_bar_at`] 也拒绝它 ⇒ 柱范围的手势够不着，
///      而这里用的正是单击那一个 [`ViewState::hit_test_visible`] 命中。
///
/// 端口是**可选**的：`None` 时（判据侧没有撤销会话、或窗口还没接端口）派发器照旧做选区，
/// 只是铅笔 / 橡皮擦如实什么都不做 —— 与"没有撤销会话就没有编辑写入面"同一取向。
///
/// `fallback_view` 只在端口缺席或它此刻没有工程时用于命中测试；端口在场时每次点击都从
/// **当前**工程重新投影，因此铅笔刚写下的音符也能立刻被点中（不拿一份过期视图做命中）。
fn install_roll_click(ui: &MainWindow, fallback_view: Rc<ViewState>, port: Option<Rc<UndoPort>>) {
    // 选区的事实源是界面属性 `selected-ulids`（`apply_view` 由它重算标志）⇒ 重装派发器时
    // 从它把内存镜像播种回来，免得"重新接线"把已有选区丢成空集。
    let selection = Rc::new(RefCell::new(crate::input::Selection::new()));
    {
        use slint::Model as _;
        let mut sel = selection.borrow_mut();
        for id in ui.get_selected_ulids().iter() {
            sel.insert(id.as_str());
        }
    }
    let weak = ui.as_weak();
    // `[UI-NOTE-003]` **单击与双击共用同一个派发体**（同一次 `hit_test_visible`、同一套工具门、
    // 同一次 `painted_to_host`）：`clicked` 传 `double_click = false`，`double-clicked` 传 `true`。
    // 这是**两个** Slint 回调（Slint 一个回调只留一个 handler），但派发逻辑**只有这一份**
    // ⇒ 双击不会长成第二条命中路径。
    let dispatch: Rc<dyn Fn(f32, f32, bool)> =
        Rc::new(move |x: f32, y: f32, double_click: bool| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            let scroll = ui.get_roll_scroll_x();
            let width = slint::ComponentHandle::window(&ui).size().width as f32;
            // 界面报的是**绘制坐标**（相对 `PianoRoll` 根：琴键列 56px ＋ 内缩 12px，表头 28px 之下），
            // 而下面的命中测试 / 铅笔 / 橡皮擦用**网格局部**坐标。帧换算**只做这一次**
            // （`bridge::painted_to_host` 是唯一真相源）—— 旧实现从不减这个偏移 ⇒ 人手点得到的
            // 音符矩形纵向差 28px（超过音符高 12px），点可见音符恒不中（本票的缺陷）。
            let (x, y) = crate::bridge::painted_to_host(x, y);
            // 权威会话可能被控制面关掉工程（`try_project` 如实返回 `None`）——
            // 那时退回装配时的视图，而不是在一个 Slint 回调里 panic。
            let project = port.as_ref().and_then(|port| port.try_project());
            let fresh = project
                .as_ref()
                .and_then(|project| ViewState::from_project(project).ok());
            let view: &ViewState = fresh.as_ref().unwrap_or(&fallback_view);
            // ---- 第一段：命中 + 选区（**总是**执行）----
            // 音符框高与 `.slint` 的矩形一致；值写在一处, 免得命中测试去猜（账本第 495 轮）。
            let hit = view.hit_test_visible(scroll, width, x, y, NOTE_HEIGHT_PX);
            // ⭐ `#212`：**借只包住选区的内存镜像** —— 先算出要写的三个读数，放借，
            // 再写界面。持借调界面（`ui.set_selected_*`）是同一类重入形态：
            // 界面写入若回调到同一份 `RefCell`，`borrow_mut()` 就会 panic。
            let (selected_count, selected_ids, note_flags) = {
                let mut sel = selection.borrow_mut();
                match hit {
                    Some(index) => sel.select_only(&view.notes[index].id),
                    None => sel.clear(),
                }
                let count = i32::try_from(sel.len()).unwrap_or(i32::MAX);
                // 把选中 id 写回界面属性 —— `apply_view` 由它重算标志, 保证重注入后仍对齐。
                let ids: Vec<slint::SharedString> =
                    sel.iter().map(|id| id.as_str().into()).collect();
                let visible = view.visible_notes(scroll, width);
                let flags = crate::input::flags_for_ids(
                    &sel.iter().cloned().collect::<Vec<_>>(),
                    &visible.ulids,
                );
                (count, ids, flags)
            };
            ui.set_selected_note_count(selected_count);
            ui.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(selected_ids)));
            ui.set_note_selected(slint::ModelRc::new(slint::VecModel::from(note_flags)));
            // 台账 R9：这一次点击**改的就是** `selected-ulids`，而这条回调**不**重投影
            // ⇒ 状态栏那一格必须在这里同步刷新，否则点击之后它停在旧选区上（= 新的假话）。
            publish_status_bar(&ui, view);
            // ---- 第二段：按工具落编辑 ----
            // ⭐ 任务单 `#166`：拒绝文本只描述**最近一次**点击 —— 每次点击先清掉上一次的。
            // 写的是**已有**的 `roll-edit-status` 属性（唯一写者就是这里），⛔ 不另造日志通道。
            ui.set_roll_edit_status("".into());
            let active_tool = ui.get_active_tool();
            let Some(tool) = u8::try_from(active_tool)
                .ok()
                .and_then(crate::input::Tool::from_digit)
            else {
                return;
            };
            match tool.click() {
                // 选择工具的全部语义就是上面那一段（命中 ⇒ 单选 / 空白 ⇒ 清除）。
                crate::input::ToolClick::SelectOrClear => {}
                crate::input::ToolClick::DrawNote => {
                    let (Some(project), Some(port)) = (project.as_ref(), port.as_ref()) else {
                        return;
                    };
                    // ⭐ `#166`：寻址只认**展示轨**（`bridge::display_track`）。展示轨没有
                    // 覆盖该 tick 的片段 ⇒ **不写**（⛔ 不回退到别的轨）＋ 把逐字原因写进可见的
                    // `roll-edit-status`（⛔ 不静默丢弃）。⭐ `#192`：展示轨 = 选中轨 or 兜底。
                    let selected = selected_track_id(&ui);
                    match pencil_attempt_for_selected(
                        project,
                        selected.as_deref(),
                        scroll,
                        x,
                        y,
                        ROLL_SNAP_GRID_TICKS,
                        active_tool,
                    ) {
                        Ok(op) => {
                            if port
                                .commit_ops(now_ms(), "pencil: add note", vec![op])
                                .is_err()
                            {
                                return;
                            }
                            reproject_roll(&ui, port, width, scroll);
                        }
                        Err(refusal) => {
                            if let Some(reason) = refusal.reason() {
                                ui.set_roll_edit_status(reason.into());
                            }
                        }
                    }
                }
                crate::input::ToolClick::DeleteNote => {
                    let (Some(project), Some(port), Some(index)) =
                        (project.as_ref(), port.as_ref(), hit)
                    else {
                        return;
                    };
                    let ops = delete_ops_for(project, &[view.notes[index].id.clone()]);
                    if ops.is_empty() {
                        return;
                    }
                    if port
                        .commit_ops(now_ms(), "eraser: delete note", ops)
                        .is_err()
                    {
                        return;
                    }
                    // 被删的身份已经不存在 ⇒ 选区必须清空（与键盘 `DeleteSelection` 同一取向，
                    // 否则 `selected-note-count` 停在旧值上、与重算出来的标志分叉）。
                    ui.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(Vec::<
                        slint::SharedString,
                    >::new(
                    ))));
                    ui.set_selected_note_count(0);
                    reproject_roll(&ui, port, width, scroll);
                }
                // `[UI-NOTE-003]` 剪刀的**左键单击**：沿吸附网格竖线把**指针下那一颗**音符
                // 切成两个独立音符（规范 §3.3 的"沿网格竖线将穿过的音符切分为两个独立音符"）。
                //
                // 手势是**指针范围**的：用的正是上面那**一个** `hit_test_visible` 命中
                // （⛔ 不新增 tick-span 查询、⛔ 不建第二条命中路径）。吸附 tick 来自唯一真相源
                // [`crate::bridge::ViewState::snapped_tick_at`]；退化切分（吸附点不严格落在音符
                // 内部）由 [`split_ops_for`] 返回空集 ⇒ **一个 op 都不写**。
                //
                // **一个原子批 = 一步撤销**（`DeleteNote` ＋ 两个 `AddNote` ⇒ `Op::Batch`）。
                crate::input::ToolClick::SplitNote => {
                    let (Some(project), Some(port), Some(index)) =
                        (project.as_ref(), port.as_ref(), hit)
                    else {
                        return;
                    };
                    let split_tick = view.snapped_tick_at(scroll, x, ROLL_SNAP_GRID_TICKS);
                    let ops = split_ops_for(project, &view.notes[index].id, split_tick);
                    if ops.is_empty() {
                        return;
                    }
                    if port.commit_ops(now_ms(), "knife: split note", ops).is_err() {
                        return;
                    }
                    // 原文的身份已经不存在（被删了）⇒ 选区必须清空，与橡皮擦同一取向：
                    // 否则 `selected-note-count` 停在旧值上、与重算出来的标志分叉。
                    ui.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(Vec::<
                        slint::SharedString,
                    >::new(
                    ))));
                    ui.set_selected_note_count(0);
                    reproject_roll(&ui, port, width, scroll);
                }
                // `[UI-NOTE-003]` **力度工具的单击 / 双击**。
                //
                // **单击**（规范 §3.3 的"选中对应音符的底部力度柱"）仍未接线 ⇒ 如实不做。
                //
                // **双击** = 规范 §3.3 aux 列的"双击恢复默认力度 100"，本切片接上。手势是
                // **音符范围**的（⛔ 不是力度柱范围）：最需要恢复的那颗音符力度为 0 ⇒ 它的柱高
                // `28px * 0` 是 **0px**、根本抓不住（`velocity_bar_at` 也如实拒绝 `velocity == 0`），
                // 而这里用的正是**单击那一个** `hit_test_visible` 命中 ⇒ 0 力度音符与别的音符
                // **同一条路径**可达。⛔ 不给柱体加最小高度、⛔ 不放宽 `velocity > 0` 那道门。
                crate::input::ToolClick::SelectVelocity => {
                    if !double_click {
                        return;
                    }
                    let (Some(project), Some(port), Some(index)) =
                        (project.as_ref(), port.as_ref(), hit)
                    else {
                        return;
                    };
                    // `velocity_op_for` 在 `old_vel == new_vel` 时返回 `None` ⇒ 已经是 100 的音符
                    // **一个 op 都不写**（模型的前置条件**不会**拒绝空 op ⇒ "不提交"必须在这里决定，
                    // 否则一次什么都没改的双击会在撤销栈上留一步）。
                    let Some(op) = velocity_op_for(
                        project,
                        &view.notes[index].id,
                        yeban_model::music::DEFAULT_VELOCITY,
                    ) else {
                        return;
                    };
                    // **一个原子 op = 一步撤销**（⛔ 不走拖拽臂的草稿路径：这是离散点击手势）。
                    if port
                        .commit_ops(now_ms(), "velocity: restore default", vec![op])
                        .is_err()
                    {
                        return;
                    }
                    reproject_roll(&ui, port, width, scroll);
                }
            }
        });
    {
        let dispatch = Rc::clone(&dispatch);
        ui.on_clicked(move |x, y| dispatch(x, y, false));
    }
    {
        let dispatch = Rc::clone(&dispatch);
        ui.on_double_clicked(move |x, y| dispatch(x, y, true));
    }
}

// ===========================================================================
// `[UI-NOTE-003]` 卷帘左键**拖拽**的两条臂（选择工具的移动 / 力度工具的力度）
// ===========================================================================
//
// 工具矩阵"左键拖拽"列的五格里有**两格**已接线，共用**同一条**手势通道
// （`note-drag-grab/move/release/cancel` 四条回调 + `note-drag-*` 草稿读数）：
//
// | 格 | 变体 | 工具门 | 松手提交的 op |
// | :--- | :--- | :--- | :--- |
// | 选择工具 | `ToolDrag::MoveNote` | [`begin_note_drag`] | **一个** `Op::MoveNote`（相对位移） |
// | 力度工具 | `ToolDrag::AdjustVelocity` | [`begin_velocity_drag`] | **一个** `Op::ModifyNoteVelocity`（绝对赋值） |
//
// 其余三格（`ResizeDuration` / `SliceAcross` / `EraseSweep`）**仍未消费**，各自的裁决见
// `src/input.rs` 的逐变体说明；结构上由判据 `move_arm_e_...` 与
// `velocity_arm_e_...` 分别钉住"这两格在 `src/input.rs` 之外真的有生产消费者"。
//
// ## 数据流（形状照抄 [`wire_track_height_drag`] / [`wire_mixer_edit`]）
//
// ```text
// piano_roll.slint 的拖动 TouchArea
//   --note-drag-grab(x, y) -> bool----+ 工具门（`Tool::drag()`）+ 命中
//                                     |   ⇒ true = **接管**这一拖（⛔ 本组件不滚动）
//   --note-drag-move(x, y)------------+ 只写**草稿**：预览位移 / 预览力度
//                                     |   （⛔ 不碰工程、⛔ 不进撤销栈）
//   --note-drag-release(x, y)---------+ 至多**一个**原子 `Op` —— **恰一次**
//   --note-drag-cancel()--------------+ `PointerEventKind.cancel` ⇒ 丢弃草稿
//   Escape（`Action::Cancel`）--------+ [`cancel_note_drag`] / [`cancel_velocity_drag`] ⇒ 丢弃草稿
// ```
//
// ## 一次拖动 = 一次提交（为什么草稿必须在手）
//
// 每个 `move` 都提交会把"拖一个音符 / 一根力度柱"碎成几百步撤销（判据
// `move_arm_d_...` / `velocity_arm_c_...` 量这件事）。因此拖动期**只**写 `.slint` 的
// `note-drag-preview-*` 属性，`release` 才把**最终位置**包成**一个** op 提交一次 ——
// 规范 §6.1「拖拽过程仅在 Slint 本地维护瞬态草稿…绝不记录任何撤销记录；松开瞬间严格
// 仅向数据核心提交 1 个原子 Op」的落点。
//
// ## 两条臂的语义差别（都是决定，不是巧合）
//
// - 移动臂的 `Op::MoveNote` 载荷是**相对**增量（`crates/yeban-model/src/ops.rs` 的变体文档：
//   集合里唯一的相对变换，重放会把增量**再加一次**）⇒ 这里**只**提交一次，⛔ 不重试；
// - 力度臂的 `Op::ModifyNoteVelocity` 载荷是 `old_vel` / `new_vel` 的**绝对**赋值 ⇒
//   指针的 y **决定**新力度本身（拖到哪就是哪），⛔ 不是"从按下点算增量"。`old_vel` 从
//   **松手那一刻的权威工程**读（拖动期没写过工程 ⇒ 它就是按下时的值），⛔ 不从草稿倒推。
//
// ## ⛔ 本切片**不**接的部分（如实登记，见交付报告）
//
// - **力度臂的多选作用域**：只作用**指针下那一个音符**（拖一根柱**不**带动选区）；
// - **零力度音符**：柱高为 0（`piano_roll.slint` 的 `28px * velocity`）⇒ 不可命中；
//   恢复它的**规范路径**是 `[UI-NOTE-003]` aux 列的**双击到 100**（已接线：见
//   [`install_roll_click`] 的 `ToolClick::SelectVelocity` 臂 ⇒ 手势是**音符**范围的，
//   ⛔ 不给柱体加最小高度、⛔ 不放宽本臂的 `velocity > 0` 门 —— 那会发明新 affordance）；
// - **辅助键**（`Shift` 微移 / `Alt` 复制）：与 `[UI-NOTE-005]` 有未裁决冲突 ⇒ ⛔ 不读修饰键；
// - **空白处拖拽**：语义一位不改（仍是滚动；marquee 未裁决）⇒ ⛔ 不做框选。

/// `[UI-NOTE-003]` 一次指针拖动在**宿主帧**下解析出的音符位移：`(delta_tick, delta_pitch)`。
///
/// 口径（两条都写明）：
/// - `delta_tick` = **两端各自的吸附点之差**（[`ViewState::snapped_tick_at`]，与铅笔 / 剪刀
///   同一个吸附真相源）—— ⛔ 不是"把像素位移换算成 tick 再吸附一次"，两者在网格边界上
///   可能差一格；
/// - `delta_pitch` = **起点泳道 − 当前泳道**（泳道索引越小 = 音高越高）。`y` 先夹进 16 条
///   泳道那一段：拖到卷帘上 / 下边缘**不该**让手势失效，而夹紧后仍由 [`crate::bridge::lane_at_y`]
///   这**唯一**的 y→泳道换算给出结果（⛔ 不在这里重写一遍除法）。
///
/// 返回 `None` = 这一点解析不出泳道（非有限数）⇒ 调用方什么都不做。
#[must_use]
pub fn note_move_delta_for(
    view: &ViewState,
    scroll_x: f32,
    origin: (f32, f32),
    current: (f32, f32),
    grid_ticks: u64,
) -> Option<(i64, i8)> {
    // `lane_at_y` 的合法输入区间：0 号泳道的顶沿 ↔ 最后一条泳道的顶沿。
    let lane_top = crate::bridge::NOTE_INSET_Y_PX;
    #[allow(clippy::cast_precision_loss)]
    let lane_bottom = lane_top
        + crate::bridge::PITCH_LANE_HEIGHT_PX * (crate::bridge::PITCH_LANE_COUNT as f32 - 1.0);
    let clamp_lane_y = |y: f32| {
        if y.is_finite() {
            y.clamp(lane_top, lane_bottom)
        } else {
            lane_top
        }
    };
    let origin_lane = crate::bridge::lane_at_y(clamp_lane_y(origin.1))?;
    let current_lane = crate::bridge::lane_at_y(clamp_lane_y(current.1))?;
    let origin_tick = view.snapped_tick_at(scroll_x, origin.0, grid_ticks);
    let current_tick = view.snapped_tick_at(scroll_x, current.0, grid_ticks);
    let delta_tick = i64::try_from(current_tick).ok()? - i64::try_from(origin_tick).ok()?;
    let delta_pitch = i8::try_from(origin_lane - current_lane).ok()?;
    Some((delta_tick, delta_pitch))
}

/// `[UI-NOTE-003]` 音符身份 ⇒ `(clip_id, track_id, 该音符**当前**力度)`（**唯一**的身份解析，
/// 移动臂与力度臂共用）。
///
/// 口径与 [`delete_ops_for`] **逐条相同**（那里是批量版）：音符住在**片段池**里
/// （`clip_pool`），`track_id` 取**摆放了该片段的第一条轨道**（`BTreeMap` 键序确定）；
/// 一个谁都没摆放的片段池条目回退到工程的第一条轨道。身份不在任何片段 / 工程没有轨道 ⇒ `None`。
///
/// 为什么力度读数也从这里取：`Op::ModifyNoteVelocity` 的 `old_vel` 必须与**权威工程**
/// 逐值相同（模型前置条件），从同一份遍历里带出来就不可能拿到第二份读数。
fn note_identity_for(
    project: &yeban_model::YebanProjectV1,
    note_id: yeban_model::EntityId,
) -> Option<(yeban_model::EntityId, yeban_model::EntityId, u8)> {
    let mut owner: std::collections::BTreeMap<yeban_model::EntityId, yeban_model::EntityId> =
        std::collections::BTreeMap::new();
    for (track_id, track) in &project.tracks {
        for placement in track.clips.values() {
            owner.entry(placement.clip_id).or_insert(*track_id);
        }
    }
    let fallback_track = project.tracks.keys().next().copied();
    for (clip_id, entry) in &project.clip_pool {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        let Some(note) = notes.get(&note_id) else {
            continue;
        };
        let track_id = owner.get(clip_id).copied().or(fallback_track)?;
        return Some((*clip_id, track_id, note.velocity));
    }
    None
}

/// `[UI-NOTE-003]` 把"被拖动音符的身份 + 位移"解析成**待提交的 `Op::MoveNote`**（纯函数）。
///
/// 身份解析走 [`note_identity_for`]（与 [`delete_ops_for`] 同一口径）。音符不在工程里 /
/// 工程没有轨道 / 身份文本非法 ⇒ `None`（调用方**什么都不做** —— 与 [`delete_ops_for`]
/// 返回空集同一取向）。
///
/// ⛔ 这里**不**自己判越界：`Op::MoveNote` 的前置条件（`crates/yeban-model/src/ops.rs`）
/// 已经拒绝 `shifted_tick` / `shifted_pitch` 为 `None` 的情形，越界的提交会被端口**如实拒绝**。
/// 在这里再算一遍就是第二个真相源（也就多一处会与模型漂移的代码）。
#[must_use]
pub fn move_note_op_for(
    project: &yeban_model::YebanProjectV1,
    note_ulid: &str,
    delta_tick: i64,
    delta_pitch: i8,
) -> Option<yeban_model::ops::Op> {
    let note_id = note_ulid.parse::<yeban_model::EntityId>().ok()?;
    let (clip_id, track_id, _velocity) = note_identity_for(project, note_id)?;
    Some(yeban_model::ops::Op::MoveNote {
        track_id,
        clip_id,
        note_id,
        delta_tick,
        delta_pitch,
    })
}

/// `[UI-NOTE-003]` 把"被拖动音符的身份 + **目标力度**"解析成**待提交的
/// `Op::ModifyNoteVelocity`**（纯函数，力度臂的唯一构造点）。
///
/// 语义（三条都是决定）：
/// - **绝对赋值**：`new_vel` 就是结果，`old_vel` 从权威工程读 —— 载荷形状（`old_vel` /
///   `new_vel` 成对）本身就是这个口径的表达，⛔ 不是增量；
/// - **没变 ⇒ `None`**：`old_vel == new_vel` 时**不提交空 op**（模型**不会**拒绝这种 op ——
///   前置条件只比对 `old_vel` —— 所以"不提交"必须在这里决定，否则一次没改变任何东西的
///   手势会在撤销栈上留一步）；
/// - 身份解析走 [`note_identity_for`]；身份非法 / 不在工程里 ⇒ `None`。
#[must_use]
pub fn velocity_op_for(
    project: &yeban_model::YebanProjectV1,
    note_ulid: &str,
    new_vel: u8,
) -> Option<yeban_model::ops::Op> {
    let note_id = note_ulid.parse::<yeban_model::EntityId>().ok()?;
    let (clip_id, track_id, old_vel) = note_identity_for(project, note_id)?;
    if old_vel == new_vel {
        return None;
    }
    Some(yeban_model::ops::Op::ModifyNoteVelocity {
        track_id,
        clip_id,
        note_id,
        old_vel,
        new_vel,
    })
}

/// 移动手势的**草稿清零**（松手提交之后 / `Escape` / 指针离开窗口三条路径共用）。
///
/// 五个手势读数**一律**在这一次清零 ⇒ 不存在"拖到一半松在窗口外，下一次移动继续拖"这种
/// 粘住的状态（与 [`end_track_height_drag`] 同一取向）。
fn clear_note_drag(ui: &MainWindow) {
    ui.set_note_drag_active(false);
    ui.set_note_drag_ulid("".into());
    ui.set_note_drag_origin_x(0.0);
    ui.set_note_drag_origin_y(0.0);
    ui.set_note_drag_preview_dx(0.0);
    ui.set_note_drag_preview_dy(0.0);
}

/// 指针按下：判断这一按**是不是**移动音符（工具门 + 命中），是则**接管**并记下起点。
///
/// 返回 `true` = 接管这次手势（`.slint` 因此**不**滚动、抬起时也不顺带 fired 一次 `clicked`）。
/// 四种情况返回 `false`：当前工具的左键拖拽语义不是 [`crate::input::ToolDrag::MoveNote`]
/// （⛔ 只接选择工具，别的工具照旧走它们自己的语义）、`active-tool` 不是合法数字、
/// 这一点没有命中任何**可见**音符、以及没有工程可投影。
fn begin_note_drag(
    ui: &MainWindow,
    fallback_view: &ViewState,
    port: Option<&Rc<UndoPort>>,
    x: f32,
    y: f32,
) -> bool {
    let width = slint::ComponentHandle::window(ui).size().width as f32;
    let scroll = ui.get_roll_scroll_x();
    // 界面报的是**绘制坐标** ⇒ 与点击派发器同一次、同一个 `painted_to_host` 换算。
    let (x, y) = crate::bridge::painted_to_host(x, y);
    let project = port.and_then(|port| port.try_project());
    let fresh = project
        .as_ref()
        .and_then(|project| ViewState::from_project(project).ok());
    let view: &ViewState = fresh.as_ref().unwrap_or(fallback_view);
    // ---- 工具门：`Tool::drag()` 的**第一个生产消费者**（在此之前整列是死代码）----
    let Some(tool) = u8::try_from(ui.get_active_tool())
        .ok()
        .and_then(crate::input::Tool::from_digit)
    else {
        return false;
    };
    if tool.drag() != crate::input::ToolDrag::MoveNote {
        return false;
    }
    let Some(index) = view.hit_test_visible(scroll, width, x, y, NOTE_HEIGHT_PX) else {
        return false;
    };
    let note = &view.notes[index];
    // 身份写进 `note-drag-ulid`：它**同时**是宿主的"这一拖是哪个音符"读数与界面的
    // 草稿预览选择器（`piano_roll.slint` 按 `note_ulid == root.note-drag-ulid` 选中被拖的
    // 那一个音符矩形 / 那一根力度柱）。⛔ 不另开第二个同义字符串属性。
    ui.set_note_drag_ulid(note.id.as_str().into());
    ui.set_note_drag_origin_x(x);
    ui.set_note_drag_origin_y(y);
    ui.set_note_drag_preview_dx(0.0);
    ui.set_note_drag_preview_dy(0.0);
    // `active` **最后**写：读者要么看到"还没开始"，要么看到一份**完整**的起点状态。
    ui.set_note_drag_active(true);
    true
}

/// 拖动中的**草稿**：把位移换算成界面上的预览像素（宿主算，界面只取属性）。
///
/// ⛔ 这里**不碰工程、不碰撤销栈** —— 这正是"拖动期不记录任何撤销记录"的落点。
/// 没有进行中的手势时**什么都不做**：一个孤立的 `move` 改不动任何东西。
fn preview_note_drag(ui: &MainWindow, fallback_view: &ViewState, x: f32, y: f32) {
    if !ui.get_note_drag_active() {
        return;
    }
    let (x, y) = crate::bridge::painted_to_host(x, y);
    let scroll = ui.get_roll_scroll_x();
    let origin = (ui.get_note_drag_origin_x(), ui.get_note_drag_origin_y());
    let Some((delta_tick, delta_pitch)) =
        note_move_delta_for(fallback_view, scroll, origin, (x, y), ROLL_SNAP_GRID_TICKS)
    else {
        return;
    };
    // 预览位移的**单位是逻辑像素**（与 `.slint` 的 `note-positions` 同帧），且是**近似**：
    // tick→px 在投影里是整数除法（`bridge::tick_to_px`），这里用浮点除 ⇒ 两者最多差不到
    // 1px。**提交之后**的位置以投影为准（重投影会把这一帧覆盖掉），所以这个近似不会残留。
    let ticks_per_pixel = roll_ticks_per_pixel(ui).max(1);
    #[allow(clippy::cast_precision_loss)]
    let dx = delta_tick as f32 / ticks_per_pixel as f32;
    #[allow(clippy::cast_precision_loss)]
    let dy = -(delta_pitch as f32) * crate::bridge::PITCH_LANE_HEIGHT_PX;
    ui.set_note_drag_preview_dx(dx);
    ui.set_note_drag_preview_dy(dy);
}

/// 松手：**至多一次**原子提交（位移没跨过网格 ⇒ 一次都没有）。
///
/// 手势读数在函数**入口处**无条件清零（先取出、再清零）：无论下面走哪条出口，
/// 手势都不留在手上。
fn commit_note_drag(
    ui: &MainWindow,
    fallback_view: &ViewState,
    port: Option<&Rc<UndoPort>>,
    x: f32,
    y: f32,
) {
    if !ui.get_note_drag_active() {
        return;
    }
    let note = ui.get_note_drag_ulid().to_string();
    let origin = (ui.get_note_drag_origin_x(), ui.get_note_drag_origin_y());
    clear_note_drag(ui);
    let (x, y) = crate::bridge::painted_to_host(x, y);
    let scroll = ui.get_roll_scroll_x();
    let Some((delta_tick, delta_pitch)) =
        note_move_delta_for(fallback_view, scroll, origin, (x, y), ROLL_SNAP_GRID_TICKS)
    else {
        return;
    };
    if delta_tick == 0 && delta_pitch == 0 {
        // 位移没跨过网格 ⇒ **不提交空 op**（"一次拖动"没有产生任何编辑）。
        return;
    }
    let Some(port) = port else {
        return;
    };
    if note.is_empty() {
        return;
    }
    let Some(project) = port.try_project() else {
        return;
    };
    let Some(op) = move_note_op_for(&project, &note, delta_tick, delta_pitch) else {
        return;
    };
    if port
        .commit_ops(now_ms(), "select: move note", vec![op])
        .is_err()
    {
        // 模型拒绝（结果越界 / 身份已不存在）⇒ 如实什么都不做，⛔ **不重试**：
        // `Op::MoveNote` 是**相对**变换，重试会把同一个增量再加一次。
        return;
    }
    let width = slint::ComponentHandle::window(ui).size().width as f32;
    reproject_roll(ui, port, width, scroll);
}

/// `Escape` / `PointerEventKind.cancel` 的收尾：**丢弃**草稿（⛔ 不提交），预览位移清零。
///
/// 返回 `true` = 确实有手势在手、已经收尾；`false` = 此刻没有手势 ⇒ 这一键与 DAW 无关，
/// 照旧放行（`apply_action` 的 `Action::Cancel` 链靠它决定要不要消费这一键）。
///
/// ⚠ 这里**不重投影**（与 [`cancel_track_height_drag`] 的差别，理由写明）：拖动期改的
/// **只有** `note-drag-preview-*` 这三个属性，清零它们就是"还原初始位置"；
/// 重投影会白做一次 `apply_view` 并把一堆视图态与电平快照重写一遍 —— 那是副作用而非还原。
pub fn cancel_note_drag(ui: &MainWindow) -> bool {
    if !ui.get_note_drag_active() {
        return false;
    }
    clear_note_drag(ui);
    true
}

// ===========================================================================
// `[UI-NOTE-003]` 力度臂（力度工具；与移动臂共用 `note-drag-*` 手势通道）
// ===========================================================================

/// 指针的**绘制帧** y ⇒ **车道内** y（0 = 力度车道顶沿）。
///
/// 车道顶沿由 `.slint` **上报**（`MainWindow::roll-velocity-lane-top` ← `PianoRoll` 的
/// `velocity-lane.y`）；⛔ 宿主**不**在这里写 `height - 44` —— 那会是第二份布局算术，正是
/// `tool_matrix_wiring::roll_click_has_exactly_one_dispatcher_writer` 的禁用常量表要挡的东西
/// （`host.rs` 里不许出现帧原点常量或手写偏移）。
///
/// 非有限 y ⇒ `None`（调用方什么都不做：⛔ 不把 NaN 静默当成"拖到最低 = 0"）。
fn velocity_lane_y(ui: &MainWindow, painted_y: f32) -> Option<f32> {
    let lane_y = painted_y - ui.get_roll_velocity_lane_top();
    lane_y.is_finite().then_some(lane_y)
}

/// 力度臂的**唯一命中口径**：`(x, y)`（**绘制帧**）落在哪一根可拖的力度柱上。
///
/// 两条坐标口径**不同源**，这里写明（这正是力度臂最容易接错的地方）：
/// - **x 用绘制帧原样**：[`ViewState::velocity_bar_hit_test`] 自己就按 `.slint` 的
///   `56px + Tokens.space-5 + note-positions[i] + 30px` 算柱体左沿，而 `note.x - scroll`
///   恰好就是绘制帧的 `note-positions[i]` ⇒ ⛔ **不**做 `painted_to_host`（做了反而偏 68px）；
/// - **y 先换算成车道内 y**（[`velocity_lane_y`]），因为命中测试的 y 以车道顶沿为 0。
///
/// 零力度音符**显式排除**：柱高为 0（`.slint` 的 `28px * velocity`）⇒ 命中区间退化成
/// 车道底沿往上 4px 处的**一条线**（闭区间），那不是可用的 affordance。本切片按裁决
/// **处理可达情形（velocity > 0）**：⛔ 不给柱体加最小高度、⛔ 不发明新交互；
/// 把 0 提起来的规范路径是 `[UI-NOTE-003]` aux 列的**双击到 100**（已接线，走的是
/// [`install_roll_click`] 的**音符**命中 ⇒ 这里那道 `velocity > 0` 的门**一位不动**）。
fn velocity_bar_at(ui: &MainWindow, view: &ViewState, x: f32, y: f32) -> Option<usize> {
    let scroll = ui.get_roll_scroll_x();
    let width = slint::ComponentHandle::window(ui).size().width as f32;
    let lane_y = velocity_lane_y(ui, y)?;
    let index = view.velocity_bar_hit_test(
        scroll,
        width,
        x,
        lane_y,
        crate::bridge::VelocityLaneGeometry::roll_velocity_lane(),
    )?;
    (view.notes[index].velocity > 0).then_some(index)
}

/// 力度手势的**草稿清零**（松手提交之后 / `Escape` / 指针离开窗口三条路径共用）。
///
/// 与 [`clear_note_drag`] 同一取向：**所有**力度手势读数在这一次清零 ⇒ 不留粘住的状态。
/// 身份 `note-drag-ulid` 也一并清（它同时是界面预览的选择器）。
fn clear_velocity_drag(ui: &MainWindow) {
    ui.set_velocity_drag_active(false);
    ui.set_note_drag_ulid("".into());
    ui.set_note_drag_preview_velocity(0.0);
}

/// 指针按下：判断这一按**是不是**拖力度柱（工具门 + 柱命中），是则**接管**并写下草稿起点。
///
/// 返回 `true` = 接管（`.slint` 因此不滚动、抬起时也不顺带 fired 一次 `clicked`）。
/// 五种情况返回 `false`：当前工具的拖拽语义不是 [`crate::input::ToolDrag::AdjustVelocity`]、
/// `active-tool` 不是合法数字、这一点没命中任何**可拖**的力度柱（含零力度柱）、
/// 命中测试所需的车道顶沿不是有限数、以及没有工程可投影。
///
/// 草稿起点 = 该音符**当前**力度的归一化值（界面按它画柱）⇒ 松手前画面就是"现在这样"，
/// 第一次 `move` 才动。
fn begin_velocity_drag(
    ui: &MainWindow,
    fallback_view: &ViewState,
    port: Option<&Rc<UndoPort>>,
    x: f32,
    y: f32,
) -> bool {
    let project = port.and_then(|port| port.try_project());
    let fresh = project
        .as_ref()
        .and_then(|project| ViewState::from_project(project).ok());
    let view: &ViewState = fresh.as_ref().unwrap_or(fallback_view);
    // ---- 工具门：`Tool::drag()` 的**第二个**生产消费者（第一格是移动臂）----
    let Some(tool) = u8::try_from(ui.get_active_tool())
        .ok()
        .and_then(crate::input::Tool::from_digit)
    else {
        return false;
    };
    if tool.drag() != crate::input::ToolDrag::AdjustVelocity {
        return false;
    }
    let Some(index) = velocity_bar_at(ui, view, x, y) else {
        return false;
    };
    let note = &view.notes[index];
    // 身份与预览：与移动臂**共用** `note-drag-ulid`（界面只按它选被拖的那一个）。
    ui.set_note_drag_ulid(note.id.as_str().into());
    ui.set_note_drag_preview_velocity(note.velocity_normalized);
    // `active` **最后**写：读者要么看到"还没开始"，要么看到一份**完整**的起点状态。
    ui.set_velocity_drag_active(true);
    true
}

/// 拖动中的**草稿**：把指针 y 换算成界面上的柱高（宿主算，界面只取归一化值）。
///
/// ⛔ 这里**不碰工程、不碰撤销栈** —— "拖动期不记录任何撤销记录"的落点。
/// 没有进行中的手势时**什么都不做**：一个孤立的 `move` 改不动任何东西。
fn preview_velocity_drag(ui: &MainWindow, y: f32) {
    if !ui.get_velocity_drag_active() {
        return;
    }
    let Some(lane_y) = velocity_lane_y(ui, y) else {
        return;
    };
    let Some(velocity) =
        crate::bridge::VelocityLaneGeometry::roll_velocity_lane().velocity_at_lane_y(lane_y)
    else {
        return;
    };
    ui.set_note_drag_preview_velocity(
        f32::from(velocity) / f32::from(yeban_model::music::MIDI_VELOCITY_MAX),
    );
}

/// 松手：**至多一次**原子 `Op::ModifyNoteVelocity`（结果与按下时的力度相同 ⇒ 一次都没有）。
///
/// 手势读数在函数**入口处**无条件清零（先取出、再清零）：无论下面走哪条出口，
/// 手势都不留在手上。`x` 用不到（力度只由 y 决定）—— 签名与另外三条回调同形，
/// 因此这里显式 `_x` 而不是换一个形状。
///
/// `old_vel` 在**这里**从权威工程读（[`velocity_op_for`]），⛔ 不从草稿倒推：
/// 拖动期没写过工程 ⇒ 读到的一定是按下时那个值；万一真的不是，模型的前置条件会如实拒绝
/// （`ModelError::OpStateMismatch`），那就什么都不做 —— 与移动臂同一取向。
fn commit_velocity_drag(ui: &MainWindow, port: Option<&Rc<UndoPort>>, _x: f32, y: f32) {
    if !ui.get_velocity_drag_active() {
        return;
    }
    let note = ui.get_note_drag_ulid().to_string();
    clear_velocity_drag(ui);
    let Some(lane_y) = velocity_lane_y(ui, y) else {
        return;
    };
    let Some(new_vel) =
        crate::bridge::VelocityLaneGeometry::roll_velocity_lane().velocity_at_lane_y(lane_y)
    else {
        return;
    };
    let Some(port) = port else {
        return;
    };
    if note.is_empty() {
        return;
    }
    let Some(project) = port.try_project() else {
        return;
    };
    // `None` = 身份解析不出**或**力度没变（后者不提交空 op —— 模型不会拒绝它，
    // 所以必须在这里决定，否则一次"什么都没改"的手势会留一步撤销）。
    let Some(op) = velocity_op_for(&project, &note, new_vel) else {
        return;
    };
    if port
        .commit_ops(now_ms(), "velocity: set note velocity", vec![op])
        .is_err()
    {
        return;
    }
    let width = slint::ComponentHandle::window(ui).size().width as f32;
    let scroll = ui.get_roll_scroll_x();
    reproject_roll(ui, port, width, scroll);
}

/// `Escape` / `PointerEventKind.cancel` 的收尾：**丢弃**力度草稿（⛔ 不提交）。
///
/// 返回 `true` = 确实有力度手势在手、已经收尾；`false` = 此刻没有 ⇒ 这一键与 DAW 无关，
/// 照旧放行（`apply_action` 的 `Action::Cancel` 链靠它决定要不要消费这一键）。
///
/// ⚠ 与 [`cancel_note_drag`] 一样**不重投影**：拖动期改的只有 `note-drag-preview-velocity`
/// （与身份 `note-drag-ulid`），清零它们就是"还原画面"。
pub fn cancel_velocity_drag(ui: &MainWindow) -> bool {
    if !ui.get_velocity_drag_active() {
        return false;
    }
    clear_velocity_drag(ui);
    true
}

/// `[UI-NOTE-003]` 卷帘拖拽四条回调的接线（本文件是唯一实现，**两条臂共用**）。
///
/// 分派按工具门：每条回调都把两条臂**各问一遍**（两条臂各自门自己的 `ToolDrag` 变体 ⇒
/// 互斥）。⛔ 不在这里读 `active-tool` 再 `match` —— 那样工具门就有两处（这里与各臂内部）。
fn install_roll_note_drag(
    ui: &MainWindow,
    fallback_view: Rc<ViewState>,
    port: Option<Rc<UndoPort>>,
) {
    let weak = ui.as_weak();
    ui.on_note_drag_grab({
        let weak = weak.clone();
        let view = Rc::clone(&fallback_view);
        let port = port.clone();
        move |x, y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return false;
            };
            begin_note_drag(&ui, &view, port.as_ref(), x, y)
                || begin_velocity_drag(&ui, &view, port.as_ref(), x, y)
        }
    });
    ui.on_note_drag_move({
        let weak = weak.clone();
        let view = Rc::clone(&fallback_view);
        move |x, y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            preview_note_drag(&ui, &view, x, y);
            preview_velocity_drag(&ui, y);
        }
    });
    ui.on_note_drag_release({
        let weak = weak.clone();
        let view = Rc::clone(&fallback_view);
        let port = port.clone();
        move |x, y| {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            commit_note_drag(&ui, &view, port.as_ref(), x, y);
            commit_velocity_drag(&ui, port.as_ref(), x, y);
        }
    });
    ui.on_note_drag_cancel({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                debug_assert!(false, "MainWindow 在回调执行期间被销毁");
                return;
            };
            cancel_note_drag(&ui);
            cancel_velocity_drag(&ui);
        }
    });
}

/// 卷帘编辑（铅笔 / 橡皮擦）提交之后的**重投影**：视图态从窗口读回来，
/// 因此"调过行高 / 缩放之后编辑一个音符"不会把那些视图态静默清零
/// （与 [`wire_roll_edit`] 旧实现的两处逐字同一手法）。
fn reproject_roll(ui: &MainWindow, port: &UndoPort, width: f32, scroll: f32) {
    if let Some(refreshed) = port
        .try_project()
        .and_then(|project| project_with_view_state(&project, ui).ok())
    {
        apply_view(ui, &refreshed, width, scroll);
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
    refresh_undo_window(&ui, port, reproject);
}

/// 状态栏「选区信息」那一格在**没有选区**时的文本（台账 R9）。
///
/// 它必须与 `crates/yeban-app/ui/status_bar.slint` 的 `selection` 默认值**逐字符相同**：
/// 那一格在宿主写者跑之前显示的就是那条默认值，两者不同就是"没选区时显示什么"
/// 有两个口径。由 `crates/yeban-app/tests/status_bar_honesty.rs` 机械看守。
pub const STATUS_SELECTION_EMPTY: &str = "选区 无";

/// 把状态栏的 `status-selection` / `status-device` 两格写进界面（台账 R9）。
///
/// # 调用点（只有两处，都在本文件）
///
/// 1. [`apply_view`] 的末尾 —— 每一次重新投影（换工程 / 撤销 / 打开工程 / 滚动）；
/// 2. [`build_main_window_with_console_tab`] 里音符点击回调 —— 它改 `selected-ulids`
///    但**不**重投影（`apply_view` 不在那条路上）。
///
/// ⇒ 状态栏读数与画面来自**同一份投影**：没有"第三份选区状态"。
///
/// # 每一格的数据源（都可追到 `file:line`）
///
/// - `status-selection` ← [`ViewState::selection_tick_span`]（`crate::bridge`）解析界面属性
///   `selected-ulids`；格式化走**唯一**的 [`crate::bridge::timecode_for_ticks`]
///   （与走带时间码同一个实现）。该函数返回 `None`（空选区 / 选中的身份已不在工程里）
///   ⇒ 写 [`STATUS_SELECTION_EMPTY`]，**不**编一个区间。
/// - `status-device` ← [`ViewState::audio_config_display`]，它在投影期由
///   `project.audio_config.sample_rate` / `.bit_depth` 经
///   [`crate::bridge::audio_config_display`] 算出。
///
/// # 它**不**写什么
///
/// `chord` 那一格不在这里：app 侧没有和弦识别能力（见 `ui/status_bar.slint` 的 R9 登记），
/// 因此那一格停在"无读数"形态，而不是被谁填一个看起来像和弦的值。
fn publish_status_bar(ui: &MainWindow, view: &ViewState) {
    let selected: Vec<String> = {
        use slint::Model as _;
        ui.get_selected_ulids()
            .iter()
            .map(|id| id.to_string())
            .collect()
    };
    // 两个端点各格式化一次：`timecode_for_ticks` 是纯函数，两次调用同一个网格
    // ⇒ 不存在"起点与终点用了两个网格"的可能。
    let selection = match view.selection_tick_span(&selected) {
        Some((start_tick, end_tick)) => {
            let grid = view.timecode_grid();
            format!(
                "选区 {} – {}",
                crate::bridge::timecode_for_ticks(grid, start_tick),
                crate::bridge::timecode_for_ticks(grid, end_tick),
            )
        }
        None => STATUS_SELECTION_EMPTY.to_owned(),
    };
    ui.set_status_selection(selection.into());
    ui.set_status_device(view.audio_config_display.clone().into());
}

/// 同 [`refresh_undo`]，但调用方**已经**持有活窗口（键盘路径就是这样）。
///
/// 抽出来的理由不是省一次 `upgrade`：两条入口必须回写**同一批**属性与投影，
/// 否则"按钮撤销"和"快捷键撤销"会在界面上留下不同的显示态。
fn refresh_undo_window(ui: &MainWindow, port: &UndoPort, reproject: bool) {
    apply_undo(ui, port);
    // ⭐ 任务单 `#211`：运行期回执**不得**活得比它描述的那一帧久。
    // "已装上：X" 是**点击那一刻**的事实；撤销 / 别的手势 / 换工程之后它就不再为真
    // ⇒ 任何一次重投影都把这一格清空（`apply_pending_instrument_probe` 在成功路径上
    // **先**调本函数、**再**写终态，因此那条回执不会被这里抹掉）。
    // ⚠ 住在**本函数**而不是 `apply_view`：`apply_view` 的属性写入点预算是 93
    // （`tests/perf_draw_budget.rs` 的 `assert_eq!`），本单不让它涨。
    ui.set_instrument_install_status("".into());
    if !reproject {
        return;
    }
    // 权威会话可能被控制面关掉工程 ⇒ `try_project` 如实返回 `None`，
    // 那时**出声不画**，而不是在一个 Slint 回调里 panic。
    let Some(project) = port.try_project() else {
        eprintln!("[yeban-app] 撤销后没有可投影的工程: 权威会话此刻没有活跃工程");
        return;
    };
    // 撤销只改工程，**不该**丢掉视图态：行高布局与横向缩放从窗口读回来（与 `scroll_x` 同款）。
    match project_with_view_state(&project, ui) {
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
    // ⭐ `#191`：这里**不再**写 `compact`。`ui/app.slint` 的 `MainWindow.compact` 是
    // `width < Tokens.breakpoint-full-hd` 的表达式绑定 ⇒ 断点跟着**真实窗口宽**走
    //（⭐ `#195` 起该令牌是 `1180`）。原来这一行写的是
    // `ui.set_compact(scene.compact())`，而 `scene.viewport_width` 恒为 1920
    //（`scene.rs` 的 `DEMO_VIEWPORT_WIDTH`）且只在装配时算一次 —— 那正是
    //"窗口拉窄、左栏不折叠"的根因。
    apply_view(&ui, view, ui.window().size().width as f32, 0.0);
    // `[ROAD-M3-002]` 滚动手势（账本第 199 轮）：偏移由**宿主**拥有 —— 因为裁剪也在宿主侧。
    // 卷帘只报告增量, 宿主 clamp + 累加后**重新注入**；不用 `Flickable`, 否则会与宿主偏移双重计算。
    let scroll = std::rc::Rc::new(std::cell::RefCell::new(0.0_f32));
    // `[UI-NOTE-003]` 选区与工具派发共用**唯一**的点击派发器 `install_roll_click`：
    // 这里先装"只做选区"的形态（此刻还没有撤销端口），`wire_roll_edit` 随后用**同一个**
    // 派发器重装一次把铅笔/橡皮擦接上。两次装的是同一份实现 ⇒ 重装是超集而不是覆盖
    // （旧实现两个写者各写一半，后装的铅笔派发器把选区写者挤掉 —— 缺陷见 `wire_roll_edit`）。
    let view_snapshot = std::rc::Rc::new(view.clone());
    install_roll_click(&ui, std::rc::Rc::clone(&view_snapshot), None);
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

    Ok(ui)
}

#[cfg(test)]
mod tests {
    use super::advance_scroll;

    /// 判据（混音台的**纯算术**那一半）：推子位移 → 位置 / dB 的映射、夹紧与退化输入。
    ///
    /// 口径（与 `.slint` 的几何同源）：位置空间 0.0–1.0 铺满 `[-60, 6]` dB，
    /// 走完整个行程需要 `MIXER_FADER_TRAVEL_PX` 像素 ⇒ **每像素 66 ÷ 88 = 0.75 dB**。
    #[test]
    fn dragged_fader_db_maps_pixels_to_db_and_clamps() {
        use super::{
            MIXER_FADER_TRAVEL_PX, dragged_fader_db, dragged_fader_fraction, fader_db_from_fraction,
        };
        use crate::bridge::{FADER_MAX_DB, FADER_MIN_DB, volume_fraction};

        // 向上拖（`delta_px > 0`）= 变大：88px 恰好走完 66 dB。
        assert!((dragged_fader_db(-3.2, MIXER_FADER_TRAVEL_PX) - FADER_MAX_DB).abs() < 1e-4);
        // 向下拖 40px = −30 dB（40 ÷ 88 × 66 = 30）。
        assert!(
            (dragged_fader_db(-3.2, -40.0) - (-33.2)).abs() < 1e-3,
            "实测 {}",
            dragged_fader_db(-3.2, -40.0)
        );
        // 位移为 0 时**逐位**返回起点：这是"拖出去再拖回来不提交"的前提
        // （第一版没有这条短路，实测差 3 ulp，于是会提交一步"数值没变"的编辑）。
        assert_eq!(dragged_fader_db(-3.2, 0.0).to_bits(), (-3.2_f32).to_bits());
        assert_eq!(
            dragged_fader_fraction(volume_fraction(-3.2), 0.0).to_bits(),
            volume_fraction(-3.2).to_bits()
        );
        // 位置 ⇄ dB 的反解必须与投影的 `volume_fraction` 是同一把尺子。
        assert_eq!(fader_db_from_fraction(0.0), FADER_MIN_DB);
        assert_eq!(fader_db_from_fraction(1.0), FADER_MAX_DB);
        assert!((fader_db_from_fraction(volume_fraction(-3.2)) - (-3.2)).abs() < 1e-3);
        // 越界：夹到量程两端（不是绕回、不是 `NaN`）。
        assert_eq!(dragged_fader_db(-3.2, -10_000.0), FADER_MIN_DB);
        assert_eq!(dragged_fader_db(-3.2, 10_000.0), FADER_MAX_DB);
        // 非有限位移：原地返回起点（`NaN` 进工程是最难查的一类缺陷）。
        assert_eq!(
            dragged_fader_db(-3.2, f32::NAN).to_bits(),
            (-3.2_f32).to_bits()
        );
        assert_eq!(
            dragged_fader_db(-3.2, f32::INFINITY).to_bits(),
            (-3.2_f32).to_bits()
        );
    }

    /// 判据（混音台的**纯算术**那一半）：声相位移 → `pan` 的映射、夹紧与退化输入。
    ///
    /// 口径：`MIXER_PAN_TRAVEL_PX` 像素走完 `-1.0..=+1.0`（跨度 2.0）⇒ **每像素 2/36**。
    #[test]
    fn dragged_pan_maps_pixels_to_pan_and_clamps() {
        use super::{MIXER_PAN_TRAVEL_PX, dragged_pan};

        // 走完整个行程 ⇒ 两端。
        assert!((dragged_pan(0.0, MIXER_PAN_TRAVEL_PX) - 1.0).abs() < 1e-6);
        assert!((dragged_pan(0.0, -MIXER_PAN_TRAVEL_PX) + 1.0).abs() < 1e-6);
        // 一半行程 ⇒ ±0.5（判据 ② 的 `R50` 就是这一格：9 ÷ 36 × 2 = 0.5）。
        assert!((dragged_pan(0.0, 9.0) - 0.5).abs() < 1e-6);
        assert!((dragged_pan(0.0, -9.0) + 0.5).abs() < 1e-6);
        // 位移 0 ⇒ 逐位返回起点（同推子）。
        assert_eq!(dragged_pan(0.0, 0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(dragged_pan(-0.25, 0.0).to_bits(), (-0.25_f32).to_bits());
        // 越界夹紧（与 `ModelError::PanOutOfRange` 的边界同一处）。
        assert_eq!(dragged_pan(0.5, 10_000.0), 1.0);
        assert_eq!(dragged_pan(-0.5, -10_000.0), -1.0);
        // 非有限：位移非有限 ⇒ 起点；起点非有限 ⇒ 当居中（0.0）再施加位移。
        assert_eq!(dragged_pan(0.25, f32::NAN).to_bits(), 0.25_f32.to_bits());
        assert_eq!(dragged_pan(f32::NAN, 0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(dragged_pan(f32::NAN, MIXER_PAN_TRAVEL_PX), 1.0);
        // 起点越界（模型不该给，但投影不假设输入可信）：先夹再施加位移。
        assert_eq!(dragged_pan(5.0, 0.0), 1.0);
        assert_eq!(dragged_pan(-5.0, 0.0), -1.0);
    }

    /// 判据（**主总线身份的取法**）：`master_track_id` 只认权威工程里非 nil、且真的在
    /// `tracks` 里的 `master_bus_track_id`。
    ///
    /// 为什么这条必须存在：主控手势的寻址**完全**依赖这一个函数（主总线不在注入给界面的
    /// `track-ids` / `track-names` 里，见 `host.rs` 的 `wire_mixer_edit` 模块文档）。
    /// 它返回 `None` 时调用方**不留手势**（不是猜一条轨道），因此"nil 工程 ⇒ 主控不可操作"
    /// 是设计行为而不是缺陷。
    #[test]
    fn master_track_id_reads_the_authoritative_project_and_refuses_nil() {
        use std::rc::Rc;

        use crate::undo::{UndoPort, UndoSession};

        let demo = crate::bridge::demo_project();
        let port = Rc::new(UndoPort::new(
            UndoSession::open("<判据>", "yeban-app", demo.clone(), 0).expect("打开"),
        ));
        let expected = demo.master_bus_track_id.to_canonical_string();
        assert_eq!(
            super::master_track_id(&port).as_deref(),
            Some(expected.as_str())
        );
        // 主总线必须是 `TrackKind::Master`（与 `YebanProjectV1::validate` 同一口径）。
        let master = demo
            .tracks
            .get(&demo.master_bus_track_id)
            .expect("主总线必须在 `tracks` 里");
        assert_eq!(master.kind, yeban_model::project::TrackKind::Master);
        // 既有 `Op` 对主总线**没有**额外的前置条件：`SetParam` 的 `read_param` / `write_param`
        // 只按 `track_id` 取轨道（`crates/yeban-model/src/ops.rs`）。这里用**真实提交**证明：
        // 一次 `TrackVolume{master}` 的 `SetParam` 必须被接受，且模型真的改了。
        let before = master.volume_db;
        let op = yeban_model::Op::SetParam {
            target: yeban_model::AutomationTarget::TrackVolume {
                track_id: demo.master_bus_track_id,
            },
            old_val: before,
            new_val: -12.5,
        };
        port.commit_ops(0, "probe: master volume", vec![op])
            .expect("主总线上的 `SetParam` 必须被接受（既有 `Op` 够用）");
        let after = super::mixer_track_state(&port, &expected).expect("主总线读数");
        assert_eq!(after.volume_db.to_bits(), (-12.5_f32).to_bits());

        // `master_bus_track_id` 是 nil 的工程（`YebanProjectV1::default()`）⇒ `None`。
        let empty = yeban_model::project::YebanProjectV1::default();
        assert!(empty.master_bus_track_id.is_nil());
        let port = Rc::new(UndoPort::new(
            UndoSession::open("<判据>", "yeban-app", empty, 0).expect("打开"),
        ));
        assert_eq!(super::master_track_id(&port), None);
    }

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

    #[test]
    fn duplicate_ops_for_gives_every_selected_note_a_fresh_id_one_grid_to_the_right() {
        // 判据：`duplicate_ops_for` 是"选区身份 → 待提交副本"的**唯一**解析点。
        // 空选区与垃圾身份 ⇒ **空批**（"什么都不做"是返回空, 不是 panic, 也不是复制一个猜的对象）。
        use std::collections::{BTreeMap, BTreeSet};
        use yeban_model::ModelError;
        use yeban_model::ops::Op;

        let project = filled_project();
        let count = |project: &yeban_model::YebanProjectV1| -> usize {
            project
                .clip_pool
                .values()
                .filter_map(|entry| entry.content.notes())
                .map(BTreeMap::len)
                .sum()
        };
        // 原文表：身份 → (所在片段, 音符)。键序确定（`BTreeMap`），单位是**条目**。
        let mut originals: BTreeMap<
            yeban_model::EntityId,
            (yeban_model::EntityId, yeban_model::MidiNote),
        > = BTreeMap::new();
        for (clip_id, entry) in &project.clip_pool {
            let Some(notes) = entry.content.notes() else {
                continue;
            };
            for (note_id, note) in notes {
                originals.insert(*note_id, (*clip_id, note.clone()));
            }
        }
        let total = originals.len();
        assert!(total >= 4, "夹具至少有 4 个音符, 实际 {total}");

        // ① 空选区 / 垃圾身份 ⇒ 空批（不消费, 与 `delete_ops_for` 同一取向）。
        assert!(
            duplicate_ops_for(&project, &[], ROLL_SNAP_GRID_TICKS).is_empty(),
            "空选区 ⇒ 空批"
        );
        assert!(
            duplicate_ops_for(&project, &["not-a-ulid".to_owned()], ROLL_SNAP_GRID_TICKS)
                .is_empty(),
            "解析不出的身份 ⇒ 空批"
        );

        let ids: Vec<String> = originals.keys().map(ToString::to_string).collect();
        let ops = duplicate_ops_for(&project, &ids, ROLL_SNAP_GRID_TICKS);
        assert_eq!(ops.len(), total, "每个选中的音符恰好一条 `AddNote`");

        // ② 每条副本：身份**全新**、起点**恰好右移一个网格**、其余字段逐项沿用原文。
        let mut copy_ids: BTreeSet<yeban_model::EntityId> = BTreeSet::new();
        for op in &ops {
            let Op::AddNote {
                track_id,
                clip_id,
                note,
            } = op
            else {
                panic!("只允许 `AddNote`, 实际是 {op:?}");
            };
            assert!(
                !originals.contains_key(&note.id),
                "副本身份 {} 撞上了一个原文身份 —— 复用 id 会被 `AddNote::precondition` 以 \
                 `DuplicateEntityId` 拒绝",
                note.id
            );
            assert!(copy_ids.insert(note.id), "副本身份必须互不相同");
            // 原文 = 起点减去一个网格、音高相同的那一条（夹具的 (tick, pitch) 互不相同）。
            let (source_clip, source) = originals
                .values()
                .find(|(_, original)| {
                    original.start_tick.saturating_add(ROLL_SNAP_GRID_TICKS) == note.start_tick
                        && original.pitch == note.pitch
                })
                .unwrap_or_else(|| panic!("副本 {} 找不到对应的原文", note.id));
            assert_eq!(
                note.start_tick,
                source.start_tick + ROLL_SNAP_GRID_TICKS,
                "副本必须右移恰好一个吸附网格"
            );
            // 除 `id` 与 `start_tick` 外**逐项**相同（力度 / 时值 / 全部表现力字段）。
            let mut expected = source.clone();
            expected.id = note.id;
            expected.start_tick = note.start_tick;
            assert_eq!(*note, expected, "副本只允许改 `id` 与 `start_tick` 两处");
            // 副本落在原文**所在的片段**里（同一个 `clip_id`）, 不跨片段搬运。
            assert_eq!(
                *clip_id, *source_clip,
                "副本必须进原文所在的那个片段, 不跨片段搬运"
            );
            assert!(
                project
                    .clip_pool
                    .get(clip_id)
                    .is_some_and(|entry| entry.content.notes().is_some()),
                "目标片段必须存在且是 MIDI 片段（`insert_note` 的前置条件）"
            );
            // `track_id` 必须真的摆放过这个片段（`AddNote::precondition` 会查它存在）。
            assert!(
                project.tracks.get(track_id).is_some_and(|track| {
                    track
                        .clips
                        .values()
                        .any(|placement| placement.clip_id == *clip_id)
                }),
                "副本的轨道必须真的摆放过它的片段"
            );
        }

        // ③ 模型只拒绝**重复身份**, 不拒绝**重叠**：每条副本的前置条件单独成立。
        for op in &ops {
            op.precondition(&project)
                .unwrap_or_else(|error| panic!("副本必须合法（重叠不构成拒绝）, 实际 {error:?}"));
        }
        // 把一条**原文的身份原样**再插一遍（夹具的四个音符同在一个片段里）⇒ 唯一冲突判据必须拒绝。
        let Op::AddNote {
            track_id,
            clip_id,
            note,
        } = &ops[0]
        else {
            unreachable!("上面已断言全部是 `AddNote`")
        };
        let mut same_id = note.clone();
        same_id.id = *originals.keys().next().expect("至少一个原文");
        let clash = Op::AddNote {
            track_id: *track_id,
            clip_id: *clip_id,
            note: same_id.clone(),
        };
        assert!(
            matches!(
                clash.precondition(&project),
                Err(ModelError::DuplicateEntityId { id }) if id == same_id.id
            ),
            "复用原本身份必须被 `DuplicateEntityId` 拒绝, 实际 {:?}",
            clash.precondition(&project)
        );

        // ④ 原子 + 可逆：`Op::Batch` 的逆是**逆序**的逆操作批 —— 与 `undo_session::commit` 同一条路。
        let batch = Op::Batch {
            ops: ops.clone(),
            description: "duplicate selection".to_owned(),
        };
        let mut doc = project.clone();
        batch
            .apply(&mut doc)
            .expect("整批必须可应用（原子；重叠不是拒绝）");
        assert_eq!(count(&doc), 2 * total, "复制后音符条目数必须翻倍");
        batch.apply_inverse(&mut doc).expect("一步撤销必须整批撤掉");
        assert_eq!(count(&doc), total, "撤销一次必须回到 N");
    }

    /// 判据：`nudge_ops_for` 的**组夹紧**（两条轴、两个方向、四个边界）与**空批拒绝**
    /// —— `[UI-NOTE-005]` §3.5 项 1–2 在宿主侧的算术。
    ///
    /// ## 为什么需要纯函数这一条（端到端三条在 `tests/roll_edit_tools.rs`）
    ///
    /// 样本夹具里没有音高 127 / 0 的音符，也没有靠近 `u64::MAX` 的 tick ⇒ 端到端判据
    /// 只能覆盖"时钟下界 + 组夹紧的全体拒绝"这一格。这里用手改字段的克隆把
    /// **四个边界**都走到：`pitch = 127` 之上、`pitch = 0` 之下、`tick = 0` 之下、
    /// `tick = u64::MAX` 之上。
    ///
    /// ## 它证明什么
    ///
    /// 1. 空选区 / 解析不出的身份 / **零位移** ⇒ 空 `Vec`（⛔ 不写空 op）；
    /// 2. 位移是**一个公共量**（每条 `MoveNote` 的 `delta_tick` / `delta_pitch` 逐条相同）；
    /// 3. 四个边界各自**只在那一侧**被夹掉：撞上界的组合返回空批，反方向仍照常产出
    ///    —— 夹紧**不是**"越界的位移一律不用"；
    /// 4. 产出的整批 `Op::Batch` 在真工程上**可应用**（夹过的位移必然满足 `MoveNote`
    ///    的前置条件，⛔ 不靠模型去拒）。
    ///
    /// ## ⛔ 它不证明什么
    ///
    /// - ⛔ 不证明可达性（`apply_action` 那条臂由 `tests/roll_edit_tools.rs` 的真事件 / 回调注入管）；
    /// - ⛔ 不证明幅度**常量**（那是 `nudge_delta_for` 的取值；本判据传的就是字面量 240 / 1 / 12）。
    #[test]
    fn nudge_ops_for_group_clamps_both_axes_and_refuses_no_ops() {
        use yeban_model::ops::Op;

        let project = filled_project();
        let ids: Vec<String> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.keys().map(ToString::to_string))
            .collect();
        let total = ids.len();
        assert!(total >= 4, "夹具至少有 4 个音符, 实际 {total}");

        // ① 空选区 / 垃圾身份 / 零位移 ⇒ 空批（"什么都不做"是返回空, 不是猜一个对象去动）。
        assert!(
            nudge_ops_for(&project, &[], 240, 0).is_empty(),
            "空选区 ⇒ 空批"
        );
        assert!(
            nudge_ops_for(&project, &["not-a-ulid".to_owned()], 240, 0).is_empty(),
            "解析不出的身份 ⇒ 空批"
        );
        assert!(
            nudge_ops_for(&project, &ids, 0, 0).is_empty(),
            "零位移 ⇒ 空批（⛔ 不许留下一条什么都不改的 op）"
        );

        // ② 全选右移一个网格：每个音符恰好一条 `MoveNote`，且**公共位移**逐条相同。
        let ops = nudge_ops_for(&project, &ids, 240, 0);
        assert_eq!(ops.len(), total, "每个选中的音符恰好一条 `MoveNote`");
        for op in &ops {
            let Op::MoveNote {
                delta_tick,
                delta_pitch,
                ..
            } = op
            else {
                panic!("只允许 `MoveNote`, 实际是 {op:?}");
            };
            assert_eq!(
                (*delta_tick, *delta_pitch),
                (240, 0),
                "整块平移 ⇒ 每一颗的位移必须与公共位移逐值相同"
            );
        }
        let batch = Op::Batch {
            ops,
            description: "nudge".to_owned(),
        };
        let mut doc = project.clone();
        batch
            .apply(&mut doc)
            .expect("整批必须可应用（公共位移满足每一条 `MoveNote` 的前置条件）");

        // 把**所有**音符的某个字段改掉（组夹紧的边界因此由**全体**决定, 不靠挑一颗）。
        let all_notes = |mutate: &dyn Fn(&mut yeban_model::MidiNote)| {
            let mut copy = project.clone();
            for entry in copy.clip_pool.values_mut() {
                let Some(notes) = entry.content.notes_mut() else {
                    continue;
                };
                for note in notes.values_mut() {
                    mutate(note);
                }
            }
            copy
        };

        // ③ 音高**上界**：全体在 127 ⇒ `↑`/八度都空批；反方向的 `↓` 仍照常产出（夹紧不过度）。
        let top = all_notes(&|note| note.pitch = 127);
        assert!(
            nudge_ops_for(&top, &ids, 0, 1).is_empty(),
            "全体已在 127 ⇒ 上行半音必须空批"
        );
        assert!(
            nudge_ops_for(&top, &ids, 0, 12).is_empty(),
            "全体已在 127 ⇒ 上行八度必须空批"
        );
        let down = nudge_ops_for(&top, &ids, 0, -1);
        assert_eq!(
            down.len(),
            total,
            "反方向必须照常产出（夹紧只夹撞上界的那一侧）"
        );
        assert!(
            down.iter().all(|op| matches!(
                op,
                Op::MoveNote {
                    delta_tick: 0,
                    delta_pitch: -1,
                    ..
                }
            )),
            "下行必须逐条是 `delta_pitch = -1`"
        );

        // ④ 音高**下界**：全体在 0 ⇒ 下行空批；上行照常。
        let bottom = all_notes(&|note| note.pitch = 0);
        assert!(
            nudge_ops_for(&bottom, &ids, 0, -1).is_empty(),
            "全体已在 0 ⇒ 下行半音必须空批"
        );
        assert!(
            nudge_ops_for(&bottom, &ids, 0, -12).is_empty(),
            "全体已在 0 ⇒ 下行八度必须空批"
        );
        assert_eq!(
            nudge_ops_for(&bottom, &ids, 0, 1).len(),
            total,
            "反方向必须照常产出"
        );

        // ⑤ 时钟**下界**：全体在 tick 0 ⇒ 左移空批；右移照常。
        let at_zero = all_notes(&|note| note.start_tick = 0);
        assert!(
            nudge_ops_for(&at_zero, &ids, -240, 0).is_empty(),
            "全体已在 tick 0 ⇒ 左移必须空批"
        );
        assert!(
            nudge_ops_for(&at_zero, &ids, -1, 0).is_empty(),
            "全体已在 tick 0 ⇒ `Alt` 的 1 tick 左移也必须空批"
        );
        assert_eq!(
            nudge_ops_for(&at_zero, &ids, 240, 0).len(),
            total,
            "反方向必须照常产出"
        );

        // ⑥ 时钟**上界**：全体在 `u64::MAX` ⇒ 右移空批；左移照常。
        let at_top = all_notes(&|note| note.start_tick = u64::MAX);
        assert!(
            nudge_ops_for(&at_top, &ids, 240, 0).is_empty(),
            "全体已在时钟顶端 ⇒ 右移必须空批"
        );
        assert!(
            nudge_ops_for(&at_top, &ids, 1, 0).is_empty(),
            "全体已在时钟顶端 ⇒ `Alt` 的 1 tick 右移也必须空批"
        );
        assert_eq!(
            nudge_ops_for(&at_top, &ids, -240, 0).len(),
            total,
            "反方向必须照常产出"
        );
    }
}
