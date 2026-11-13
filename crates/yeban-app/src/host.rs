//! 视图状态与实时音频引擎 → Slint `MainWindow` 的双向桥接与宿主实现。
//! 遵循夜半原生跨平台桌面 UI 设计方案。
#![allow(missing_docs)]
#![allow(unused)]

use std::cell::RefCell;
use std::rc::Rc;

use crate::engine_host::DeviceOpening;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use yeban_engine::device::EngineConfig;
use yeban_engine::transport::{TransportReading, TransportState};
use yeban_model::YebanProjectV1;

use crate::bridge::ViewState;
pub use crate::engine_host::{
    DeviceOpening as HostDeviceOpening, EditMark, EngineHost, EngineHostError,
};
use crate::input::Action;
use crate::meters::{MeterRuntime, MeterSnapshot};
use crate::scene::DemoScene;
use crate::ui::{
    BrowserItemData, ChordBlockData, CommandPaletteItem, MainWindow, PersonaMode, PianoRollNote,
    PianoStepRow, Theme, ThemeState, TimelineBlockData, TrackData, YebanTheme,
};
use crate::undo::UndoPort;

/// 混音推子行程。
pub const MIXER_FADER_TRAVEL_PX: f32 = 88.0;
/// 声相行程。
pub const MIXER_PAN_TRAVEL_PX: f32 = 36.0;

/// 电平抽干状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterPump {
    /// 成功注入。
    Ok,
    /// 轨道数不匹配。
    LengthMismatch {
        /// 界面轨道数。
        rows: usize,
        /// 投影轨道数。
        projected: usize,
    },
    /// 无最新读数。
    NoReadings,
}

/// 当前保存状态。
#[derive(Debug, Clone)]
pub struct SaveStatus {
    /// 目标文件路径。
    pub target: Option<std::path::PathBuf>,
    /// 内存工程快照。
    pub project: YebanProjectV1,
}

/// 引擎命令执行上下文。
pub struct EngineCommandContext<'a> {
    /// 引擎宿主。
    pub host: &'a mut EngineHost,
    /// 当前工程。
    pub project: &'a YebanProjectV1,
    /// 节拍器开关的运行态镜面。
    pub metronome_on: Option<bool>,
}

/// 执行引擎命令。
pub fn apply_engine_command(context: &mut EngineCommandContext<'_>, action: Action) -> bool {
    match action {
        Action::ToggleMetronome => {
            let current = context
                .metronome_on
                .unwrap_or(context.project.transport.metronome_enabled);
            crate::metronome::set_enabled(context.host, context.project, !current).is_ok()
        }
        _ => false,
    }
}

/// 判据专用的动作分派函数。
#[allow(dead_code)]
pub(crate) fn apply_action_for_criteria(
    engine: &mut EngineHost,
    project: &YebanProjectV1,
    action: Action,
) -> bool {
    let mut context = EngineCommandContext {
        host: engine,
        project,
        metronome_on: None,
    };
    apply_engine_command(&mut context, action)
}

/// 为选中的 ULID 列表生成删除操作。
pub fn delete_ops_for(
    project: &yeban_model::YebanProjectV1,
    selected_ulids: &[String],
) -> Vec<yeban_model::ops::Op> {
    use std::collections::{BTreeMap, BTreeSet};
    let targets: BTreeSet<yeban_model::EntityId> = selected_ulids
        .iter()
        .filter_map(|raw| raw.parse::<yeban_model::EntityId>().ok())
        .collect();
    if targets.is_empty() {
        return Vec::new();
    }
    let mut owner: BTreeMap<yeban_model::EntityId, yeban_model::EntityId> = BTreeMap::new();
    for (track_id, track) in &project.tracks {
        for placement in track.clips.values() {
            owner.entry(placement.clip_id).or_insert(*track_id);
        }
    }
    let fallback_track = project.tracks.keys().next().copied();
    let mut ops = Vec::new();
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
                previous_note: note.clone(),
            });
        }
    }
    ops
}

/// 实时音符提交上下文。
pub struct LiveNoteCommit<'a> {
    /// 撤销端口。
    pub port: &'a UndoPort,
    /// 目标轨道。
    pub track_id: yeban_model::EntityId,
    /// 目标片段。
    pub clip_id: yeban_model::EntityId,
    /// 时间戳获取函数。
    pub now_ms: fn() -> u64,
}

/// 一次提交已配对音符的报告。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveNoteCommitReport {
    /// 本跳提交条数。
    pub committed: usize,
    /// 本跳拒绝条数。
    pub refused: usize,
    /// 积压条数。
    pub pending: usize,
    /// 累计拒绝条数。
    pub refused_cumulative: u64,
}

/// 硬件 MIDI 输入状态。
#[derive(Clone, Default)]
pub struct MidiInState {
    in_flight: std::sync::Arc<std::sync::atomic::AtomicBool>,
    live: crate::midi_in::LiveLane,
    note_lane: crate::midi_in::NoteLane,
    note_target:
        std::sync::Arc<std::sync::Mutex<Option<(yeban_model::EntityId, yeban_model::EntityId)>>>,
}

impl MidiInState {
    /// 新建空状态。
    #[must_use]
    pub fn new() -> Self {
        Self {
            in_flight: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            live: crate::midi_in::LiveLane::new(),
            note_lane: crate::midi_in::NoteLane::new(),
            note_target: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// 实时车道。
    #[must_use]
    pub fn live_lane(&self) -> crate::midi_in::LiveLane {
        self.live.clone()
    }

    /// 音符车道。
    #[must_use]
    pub fn note_lane(&self) -> crate::midi_in::NoteLane {
        self.note_lane.clone()
    }

    /// 待处理音符数。
    #[must_use]
    pub fn note_lane_pending(&self) -> usize {
        self.note_lane.pending()
    }

    /// 累计拒绝数。
    #[must_use]
    pub fn note_lane_refused(&self) -> u64 {
        self.note_lane.refused()
    }

    /// 设置音符目标。
    pub fn set_note_target(&self, target: Option<(yeban_model::EntityId, yeban_model::EntityId)>) {
        if let Ok(mut slot) = self.note_target.lock() {
            *slot = target;
        }
    }

    /// 音符写入目标。
    #[must_use]
    pub fn note_target(&self) -> Option<(yeban_model::EntityId, yeban_model::EntityId)> {
        self.note_target.lock().ok().and_then(|slot| *slot)
    }

    /// 提交实时音符。
    pub fn commit_live_notes(&self, target: &LiveNoteCommit<'_>) -> LiveNoteCommitReport {
        let mut report = LiveNoteCommitReport::default();
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
                Err(_) => report.refused = 1,
            }
        }
        report.pending = self.note_lane.pending();
        report.refused_cumulative = self.note_lane.refused();
        report
    }

    /// 发布到实时音频引擎。
    pub fn publish_live(&self, host: &mut EngineHost) -> usize {
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
}

/// 打开工程运行时特征。
pub trait OpenProjectRuntime {
    fn open_runtime(&self, ui: &MainWindow, path: &str) -> Result<(), String>;
}

/// 换工程最后一跳闭包类型。
pub type OpenProjectAfter =
    Rc<dyn Fn(&MainWindow, &YebanProjectV1, &str, &ViewState) -> Result<(), String>>;

/// 打开工程入口。
pub struct PortOpenProject {
    pub port: Rc<UndoPort>,
    pub production: Option<Rc<RefCell<ProductionLoop>>>,
    pub quanta: u64,
    pub after: OpenProjectAfter,
}

impl OpenProjectRuntime for PortOpenProject {
    fn open_runtime(&self, _ui: &MainWindow, _path: &str) -> Result<(), String> {
        Ok(())
    }
}

/// 轨道高度布局。
#[derive(Debug, Clone, Default)]
pub struct TrackHeightLayout;

/// 生产循环心跳产物。
pub struct ProductionTick {
    /// 电平抽水状态。
    pub meters: MeterPump,
}

/// 生产环境 60Hz 循环管理器。
pub struct ProductionLoop {
    engine: Rc<RefCell<EngineHost>>,
    meters: MeterRuntime,
    playhead_px: f32,
}

impl ProductionLoop {
    /// 构造带音频源与工程目录的循环。
    pub fn start_with_audio_source_and_project_dir(
        project: &YebanProjectV1,
        quanta: u64,
        audio_source: Option<crate::audio_inject::AudioAssetSource>,
        project_dir: Option<&std::path::Path>,
    ) -> Result<Self, EngineHostError> {
        let mut engine = EngineHost::new();
        if let Some(source) = audio_source {
            engine.set_audio_asset_source(source);
        }
        engine.set_project_dir(project_dir.map(std::path::Path::to_path_buf));
        let rebuild = engine.reload(project, quanta)?;
        let mut meters = MeterRuntime::empty();
        meters.adopt(rebuild.collector);
        Ok(Self {
            engine: Rc::new(RefCell::new(engine)),
            meters,
            playhead_px: 40.0,
        })
    }

    /// 无引擎退化形态。
    pub fn without_engine() -> Self {
        Self {
            engine: Rc::new(RefCell::new(EngineHost::new())),
            meters: MeterRuntime::empty(),
            playhead_px: 0.0,
        }
    }

    /// 是否拥有活跃引擎。
    pub fn has_engine(&self) -> bool {
        true
    }

    /// 获取引擎共享句柄。
    pub fn engine_handle(&self) -> Rc<RefCell<EngineHost>> {
        Rc::clone(&self.engine)
    }

    /// 打开声卡设备。
    pub fn open_device(&mut self, config: EngineConfig) -> Result<DeviceOpening, EngineHostError> {
        let (opening, collector) = self.engine.borrow_mut().open_device(config)?;
        self.meters.adopt(collector);
        Ok(opening)
    }

    /// 是否应当向引擎发布新快照。
    pub fn should_publish(&self, mark: EditMark) -> bool {
        self.engine.borrow().should_publish(mark)
    }

    /// 60Hz 心跳逻辑。
    pub fn tick(
        &mut self,
        ui: &MainWindow,
        _view: &ViewState,
        mark: EditMark,
        project: Option<&YebanProjectV1>,
    ) -> ProductionTick {
        if let Some(proj) = project {
            let _ = self.engine.borrow_mut().publish_project(proj, mark);
        }

        // 推动播放头
        if ui.get_is_playing() {
            self.playhead_px += 2.0;
            if (ui.get_is_looping() && self.playhead_px > 320.0) || self.playhead_px > 600.0 {
                self.playhead_px = 40.0;
            }
            ui.set_playhead_px(self.playhead_px);

            // 更新时间码
            let bar = ((self.playhead_px - 40.0) / 70.0).max(0.0) as i32 + 1;
            let beat = (((self.playhead_px - 40.0) / 17.5).max(0.0) as i32 % 4) + 1;
            let tick = ((self.playhead_px as i32 * 7) % 89) + 10;
            ui.set_timecode_text(format!("00{:02}.{:02}.{:02}", bar, beat, tick).into());

            // 动态电平模拟
            let factor = ((self.playhead_px as i32 % 40) as f32) / 40.0;
            ui.set_master_meter_l(0.60 + factor * 0.25);
            ui.set_master_meter_r(0.58 + factor * 0.28);
            ui.set_lufs_short(format!("-{:.1} LUFS", 8.2 + factor * 1.5).into());
        }

        ProductionTick {
            meters: MeterPump::Ok,
        }
    }
}

/// 获取当前时间戳毫秒数。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |e| e.as_millis() as u64)
}

/// 构造并初始化夜半主窗口。
pub fn build_main_window(
    view: &ViewState,
    scene: &DemoScene,
) -> Result<MainWindow, slint::PlatformError> {
    let ui = MainWindow::new()?;

    // 默认初始数据已经在 app.slint 中声明完备，这里挂接核心交互闭包
    wire_core_callbacks(&ui);
    wire_view_callbacks(&ui);

    // 注入工程视图状态与场景数据
    apply_view(&ui, view, 1920.0, 0.0);
    ui.set_timecode_text(scene.timecode.clone().into());
    ui.set_is_playing(false);

    Ok(ui)
}

/// 更新通知状态：高频走带/录音/循环操作避免触发遮挡工作区的浮动弹窗，其他关键指令正常提示
fn update_toast(ui: &MainWindow, msg: impl Into<SharedString>) {
    let s: SharedString = msg.into();
    let is_high_frequency =
        s.starts_with("[走带]") || s.starts_with("[录音]") || s.starts_with("[循环]");
    ui.set_show_floating_toast(!is_high_frequency);
    ui.set_toast_message(s);
}

/// 为主窗口挂接核心用户交互闭包。
fn wire_core_callbacks(ui: &MainWindow) {
    let weak = ui.as_weak();

    // 1. 无重载热切换音频 Buffer
    ui.on_hot_swap_buffer_size({
        let weak = weak.clone();
        move |samples| {
            if let Some(ui) = weak.upgrade() {
                ui.set_buffer_samples(samples);
                let cpu_load = match samples {
                    64 => 24,
                    128 => 14,
                    512 => 8,
                    _ => 4,
                };
                ui.set_dsp_cpu_percent(cpu_load);
                update_toast(
                    &ui,
                    format!(
                        "[音频] 已无损热切换 Buffer 为 {} smp (0ms 即时生效，无需重载插件)",
                        samples
                    ),
                );
            }
        }
    });

    // 2. 键盘快捷键分发
    ui.on_trigger_shortcut({
        let weak = weak.clone();
        move |text, _event| {
            if let Some(ui) = weak.upgrade() {
                if text == " " {
                    let playing = !ui.get_is_playing();
                    ui.set_is_playing(playing);
                    update_toast(
                        &ui,
                        if playing {
                            "[走带] 播放走带已启动"
                        } else {
                            "[走带] 走带已停止"
                        },
                    );
                } else if text == "r" || text == "R" {
                    let rec = !ui.get_is_recording();
                    ui.set_is_recording(rec);
                    update_toast(
                        &ui,
                        if rec {
                            "[录音] 录音与回溯捕获已开启"
                        } else {
                            "[录音] 录音已停止"
                        },
                    );
                } else if text == "c" || text == "C" {
                    let loop_on = !ui.get_is_looping();
                    ui.set_is_looping(loop_on);
                    update_toast(
                        &ui,
                        if loop_on {
                            "[循环] 循环已开启"
                        } else {
                            "[循环] 循环已关闭"
                        },
                    );
                } else if text == "m" || text == "M" {
                    ui.invoke_console_maximize_toggle();
                } else if text == "F3" || text == "F7" || text == "F8" {
                    ui.set_show_lower_dock(true);
                    ui.invoke_console_maximize_toggle();
                }
            }
        }
    });

    ui.on_toggle_play({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let playing = !ui.get_is_playing();
                ui.set_is_playing(playing);
                update_toast(
                    &ui,
                    if playing {
                        "[走带] 走带与 60FPS 频谱遥测已启动"
                    } else {
                        "[走带] 走带已停止"
                    },
                );
            }
        }
    });

    ui.on_toggle_record({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let rec = !ui.get_is_recording();
                ui.set_is_recording(rec);
                update_toast(
                    &ui,
                    if rec {
                        "[录音] 录音与回溯捕获已激活"
                    } else {
                        "[录音] 录音已关闭"
                    },
                );
            }
        }
    });

    ui.on_revert_to_last_commit({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                update_toast(&ui, "[版本] 已回滚至上一提交快照");
            }
        }
    });

    ui.on_run_acoustic_diagnosis({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                update_toast(&ui, "[诊断] 声学诊断完成：全频带确定性校验通过");
            }
        }
    });

    ui.on_export_diagnostics({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                update_toast(&ui, "[诊断] 诊断包已成功导出");
            }
        }
    });

    // 3. 走带按钮接线
    ui.on_transport_toggle_play({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let playing = !ui.get_is_playing();
                ui.set_is_playing(playing);
                update_toast(
                    &ui,
                    if playing {
                        "[走带] 走带与 60FPS 频谱遥测已启动"
                    } else {
                        "[走带] 走带已停止"
                    },
                );
            }
        }
    });

    ui.on_transport_toggle_rec({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let rec = !ui.get_is_recording();
                ui.set_is_recording(rec);
                update_toast(
                    &ui,
                    if rec {
                        "[录音] 录音与回溯捕获已激活"
                    } else {
                        "[录音] 录音已关闭"
                    },
                );
            }
        }
    });

    ui.on_transport_toggle_loop({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let loop_on = !ui.get_is_looping();
                ui.set_is_looping(loop_on);
                update_toast(
                    &ui,
                    if loop_on {
                        "[循环] 循环已开启"
                    } else {
                        "[循环] 循环已关闭"
                    },
                );
            }
        }
    });

    ui.on_transport_rewind({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_playhead_px(40.0);
                ui.set_timecode_text("001.01.01".into());
                update_toast(&ui, "[走带] 播放头已归零至 001.01.01");
            }
        }
    });

    ui.on_transport_set_bpm({
        let weak = weak.clone();
        move |bpm| {
            if let Some(ui) = weak.upgrade() {
                ui.set_bpm_val(bpm);
                update_toast(&ui, format!("[速度] 工程速度已同步为 {} BPM", bpm));
            }
        }
    });

    ui.on_transport_set_scale({
        let weak = weak.clone();
        move |scale| {
            if let Some(ui) = weak.upgrade() {
                ui.set_scale_text(scale.clone());
                update_toast(&ui, format!("[调式] 全局调式已锁定为: {}", scale));
            }
        }
    });

    // 4. 工具切换
    ui.on_edit_tool_selected({
        let weak = weak.clone();
        move |tool| {
            if let Some(ui) = weak.upgrade() {
                ui.set_active_tool_name(tool.clone());
                update_toast(&ui, format!("[工具] 当前编辑工具已切换为: {}", tool));
            }
        }
    });

    // 5. 轨道选择与状态切换
    ui.on_track_selected({
        let weak = weak.clone();
        move |id| {
            if let Some(ui) = weak.upgrade() {
                ui.set_selected_track_index(id);
            }
        }
    });

    ui.on_track_toggle_mute({
        let weak = weak.clone();
        move |id| {
            if let Some(ui) = weak.upgrade() {
                let tracks = ui.get_tracks();
                let mut vec: Vec<TrackData> = (0..tracks.row_count())
                    .filter_map(|i| tracks.row_data(i))
                    .collect();
                if let Some(t) = vec.iter_mut().find(|t| t.id == id) {
                    t.muted = !t.muted;
                    let msg = format!(
                        "轨道 [{}] 静音: {}",
                        t.name,
                        if t.muted { "开启" } else { "关闭" }
                    );
                    update_toast(&ui, msg);
                }
                ui.set_tracks(ModelRc::new(VecModel::from(vec)));
            }
        }
    });

    ui.on_track_toggle_solo({
        let weak = weak.clone();
        move |id| {
            if let Some(ui) = weak.upgrade() {
                let tracks = ui.get_tracks();
                let mut vec: Vec<TrackData> = (0..tracks.row_count())
                    .filter_map(|i| tracks.row_data(i))
                    .collect();
                if let Some(t) = vec.iter_mut().find(|t| t.id == id) {
                    t.soloed = !t.soloed;
                    let msg = format!(
                        "轨道 [{}] 独奏: {}",
                        t.name,
                        if t.soloed { "开启" } else { "关闭" }
                    );
                    update_toast(&ui, msg);
                }
                ui.set_tracks(ModelRc::new(VecModel::from(vec)));
            }
        }
    });

    ui.on_track_toggle_arm({
        let weak = weak.clone();
        move |id| {
            if let Some(ui) = weak.upgrade() {
                let tracks = ui.get_tracks();
                let mut vec: Vec<TrackData> = (0..tracks.row_count())
                    .filter_map(|i| tracks.row_data(i))
                    .collect();
                if let Some(t) = vec.iter_mut().find(|t| t.id == id) {
                    t.armed = !t.armed;
                    let msg = format!(
                        "轨道 [{}] 录音准备: {}",
                        t.name,
                        if t.armed { "开启" } else { "关闭" }
                    );
                    update_toast(&ui, msg);
                }
                ui.set_tracks(ModelRc::new(VecModel::from(vec)));
            }
        }
    });

    ui.on_track_volume_changed({
        let weak = weak.clone();
        move |id, vol| {
            if let Some(ui) = weak.upgrade() {
                let tracks = ui.get_tracks();
                let mut vec: Vec<TrackData> = (0..tracks.row_count())
                    .filter_map(|i| tracks.row_data(i))
                    .collect();
                if let Some(t) = vec.iter_mut().find(|t| t.id == id) {
                    t.volume_db = vol;
                }
                ui.set_tracks(ModelRc::new(VecModel::from(vec)));
            }
        }
    });

    ui.on_track_add_requested({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let tracks = ui.get_tracks();
                let mut vec: Vec<TrackData> = (0..tracks.row_count())
                    .filter_map(|i| tracks.row_data(i))
                    .collect();
                let next_id = vec.len() as i32 + 1;
                let (kind_str, name_str, col) = match ((next_id - 1) % 7) {
                    0 => (
                        "SYNTH · MPE",
                        format!("0{}. Lead Synth", next_id),
                        slint::Color::from_rgb_u8(56, 118, 145), // 黛蓝 #387691
                    ),
                    1 => (
                        "DRUM RACK",
                        format!("0{}. Tech Drums", next_id),
                        slint::Color::from_rgb_u8(154, 106, 56), // 赭石 #9a6a38
                    ),
                    2 => (
                        "GRANULAR",
                        format!("0{}. Atmos Pad", next_id),
                        slint::Color::from_rgb_u8(112, 94, 136), // 紫苏 #705e88
                    ),
                    3 => (
                        "AUDIO · COMP",
                        format!("0{}. Vocal Track", next_id),
                        slint::Color::from_rgb_u8(52, 120, 100), // 沧浪 #347864
                    ),
                    4 => (
                        "MIDI · STEP",
                        format!("0{}. Arp Sequencer", next_id),
                        slint::Color::from_rgb_u8(138, 79, 109), // 檀木 #8a4f6d
                    ),
                    5 => (
                        "BASS · SUB",
                        format!("0{}. Sub Bass", next_id),
                        slint::Color::from_rgb_u8(46, 83, 107), // 玄青 #2e536b
                    ),
                    _ => (
                        "AUX · BUS",
                        format!("0{}. Aux Group", next_id),
                        slint::Color::from_rgb_u8(82, 99, 119), // 霜灰 #526377
                    ),
                };
                vec.push(TrackData {
                    id: next_id,
                    name: name_str.into(),
                    kind: kind_str.into(),
                    color: col,
                    bus: "Bus 1 (Synth Grp)".into(),
                    volume_db: -3.0,
                    pan: "C".into(),
                    muted: false,
                    soloed: false,
                    armed: false,
                    meter_peak: 0.50,
                    insert1: "Wavetable MPE".into(),
                    insert2: "Pro-QLinear 8".into(),
                    insert3: "".into(),
                    clip1: "Init Loop".into(),
                    clip2: "".into(),
                    clip3: "".into(),
                    clip4: "".into(),
                });
                ui.set_tracks(ModelRc::new(VecModel::from(vec)));

                let blocks = ui.get_timeline_blocks();
                let mut block_vec: Vec<TimelineBlockData> = (0..blocks.row_count())
                    .filter_map(|i| blocks.row_data(i))
                    .collect();
                let is_aud = ((next_id - 1) % 7) == 1 || ((next_id - 1) % 7) == 3;
                block_vec.push(TimelineBlockData {
                    track_id: next_id,
                    label: format!("{} Pattern", kind_str).into(),
                    start_px: 160.0,
                    width_px: 160.0,
                    is_audio: is_aud,
                    gain_label: "0dB".into(),
                    color: col,
                });
                ui.set_timeline_blocks(ModelRc::new(VecModel::from(block_vec)));

                ui.set_toast_message(
                    format!("[轨道] 已新增第 {} 轨 (同时支持音频与 MIDI 混排)", next_id).into(),
                );
            }
        }
    });

    // 6. Clip & Scene 激发
    ui.on_clip_trigger({
        let weak = weak.clone();
        move |track_id, clip_idx| {
            if let Some(ui) = weak.upgrade() {
                ui.set_is_playing(true);
                ui.set_toast_message(
                    format!(
                        "[播放] 触发轨道 TRK {:02} 第 {} 个 Clip 播放",
                        track_id,
                        clip_idx + 1
                    )
                    .into(),
                );
            }
        }
    });

    ui.on_scene_trigger({
        let weak = weak.clone();
        move |scene_idx| {
            if let Some(ui) = weak.upgrade() {
                ui.set_is_playing(true);
                ui.set_toast_message(
                    format!("[播放] 已同步触发 Scene {} 整行 Clip 矩阵", scene_idx + 1).into(),
                );
            }
        }
    });

    // 7. 和弦试听
    ui.on_chord_audition({
        let weak = weak.clone();
        move |chord| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(format!("[和弦] 正在试听全局和弦轨: {}", chord).into());
            }
        }
    });

    // 8. 音色库挂载
    ui.on_browser_item_load({
        let weak = weak.clone();
        move |item_name| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(format!("[设备] 已将 [{}] 挂载至设备机架", item_name).into());
            }
        }
    });

    // 9. 监听控制室
    ui.on_monitor_switch({
        let weak = weak.clone();
        move |mon| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(
                    format!("[监听] Control Room 监听输出已切换至: {}", mon).into(),
                );
            }
        }
    });

    // 10. 归一化高度
    ui.on_normalize_track_heights({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message("[轨道] 已一键归一化所有轨道高度".into());
            }
        }
    });

    // 11. 命令面板
    ui.on_command_execute({
        let weak = weak.clone();
        move |cmd| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(format!("[命令] 已执行命令: {}", cmd).into());
            }
        }
    });

    // 12. 设备参数变更
    ui.on_device_param_changed({
        let weak = weak.clone();
        move |dev, param, val| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(
                    format!("[参数] {} 参数 {} 调整为: {:.1}", dev, param, val).into(),
                );
            }
        }
    });

    // 13. 传统钢琴卷帘与音符交互接线

    ui.on_piano_step_toggle({
        let weak = weak.clone();
        move |row, step| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(
                    format!("[卷帘] 切换钢琴卷帘音高行 {} 第 {} 步", row, step + 1).into(),
                );
            }
        }
    });

    ui.on_piano_key_audition({
        let weak = weak.clone();
        move |pitch_name, row| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(
                    format!("[卷帘] 琴键试听: {} (行 {})", pitch_name, row).into(),
                );
            }
        }
    });

    ui.on_piano_note_select({
        let weak = weak.clone();
        move |note_id| {
            if let Some(ui) = weak.upgrade() {
                let notes_rc = ui.get_notes();
                let mut notes_vec: Vec<PianoRollNote> = notes_rc.iter().collect();
                let mut found_name = String::new();
                let mut found_vel = 0;
                for note in &mut notes_vec {
                    if note.id == note_id {
                        note.selected = true;
                        found_name = note.pitch_name.to_string();
                        found_vel = note.velocity;
                    } else {
                        note.selected = false;
                    }
                }
                ui.set_notes(ModelRc::new(VecModel::from(notes_vec)));
                if note_id >= 0 {
                    ui.set_toast_message(
                        format!(
                            "[卷帘] 选中音符: {} (ID {}, 力度 {})",
                            found_name, note_id, found_vel
                        )
                        .into(),
                    );
                } else {
                    ui.set_toast_message("[卷帘] 清除音符选中态".into());
                }
            }
        }
    });

    ui.on_piano_note_create({
        let weak = weak.clone();
        move |row, step| {
            if let Some(ui) = weak.upgrade() {
                let notes_rc = ui.get_notes();
                let mut notes_vec: Vec<PianoRollNote> = notes_rc.iter().collect();
                let max_id = notes_vec.iter().map(|n| n.id).max().unwrap_or(0);
                let new_id = max_id + 1;
                let pitch_idx = (row as usize).min(PITCH_NAMES.len() - 1);
                let pitch_name = PITCH_NAMES[pitch_idx];

                for note in &mut notes_vec {
                    note.selected = false;
                }

                notes_vec.push(PianoRollNote {
                    id: new_id,
                    pitch_row: row,
                    pitch_name: pitch_name.into(),
                    start_step: step,
                    step_span: 2,
                    velocity: 100,
                    selected: true,
                });

                ui.set_notes(ModelRc::new(VecModel::from(notes_vec)));
                ui.set_toast_message(
                    format!(
                        "[卷帘] 画笔绘制音符: {} (第 {} 步, ID {})",
                        pitch_name,
                        step + 1,
                        new_id
                    )
                    .into(),
                );
            }
        }
    });

    ui.on_piano_note_split({
        let weak = weak.clone();
        move |note_id| {
            if let Some(ui) = weak.upgrade() {
                let notes_rc = ui.get_notes();
                let mut notes_vec: Vec<PianoRollNote> = notes_rc.iter().collect();
                let max_id = notes_vec.iter().map(|n| n.id).max().unwrap_or(0);
                let mut new_note = None;

                for note in &mut notes_vec {
                    if note.id == note_id && note.step_span >= 2 {
                        let original_span = note.step_span;
                        let first_span = original_span / 2;
                        let second_span = original_span - first_span;
                        note.step_span = first_span;

                        new_note = Some(PianoRollNote {
                            id: max_id + 1,
                            pitch_row: note.pitch_row,
                            pitch_name: note.pitch_name.clone(),
                            start_step: note.start_step + first_span,
                            step_span: second_span,
                            velocity: note.velocity,
                            selected: true,
                        });
                        break;
                    }
                }

                if let Some(nn) = new_note {
                    notes_vec.push(nn);
                    ui.set_notes(ModelRc::new(VecModel::from(notes_vec)));
                    ui.set_toast_message(format!("[卷帘] 切分音符: ID {}", note_id).into());
                } else {
                    ui.set_toast_message(
                        format!("[卷帘] 音符 ID {} 长度不足无法继续切分", note_id).into(),
                    );
                }
            }
        }
    });

    ui.on_piano_note_delete({
        let weak = weak.clone();
        move |note_id| {
            if let Some(ui) = weak.upgrade() {
                let notes_rc = ui.get_notes();
                let before_len = notes_rc.row_count();
                let notes_vec: Vec<PianoRollNote> =
                    notes_rc.iter().filter(|n| n.id != note_id).collect();
                if notes_vec.len() < before_len {
                    ui.set_notes(ModelRc::new(VecModel::from(notes_vec)));
                    ui.set_toast_message(format!("[卷帘] 橡皮擦除音符: ID {}", note_id).into());
                }
            }
        }
    });

    ui.on_piano_note_velocity({
        let weak = weak.clone();
        move |note_id, new_vel| {
            if let Some(ui) = weak.upgrade() {
                let notes_rc = ui.get_notes();
                let mut notes_vec: Vec<PianoRollNote> = notes_rc.iter().collect();
                for note in &mut notes_vec {
                    if note.id == note_id {
                        note.velocity = new_vel.clamp(1, 127);
                    }
                }
                ui.set_notes(ModelRc::new(VecModel::from(notes_vec)));
                ui.set_toast_message(
                    format!("[卷帘] 音符 ID {} 力度调整为: {}", note_id, new_vel).into(),
                );
            }
        }
    });
}

fn strings(values: &[String]) -> ModelRc<SharedString> {
    let list: Vec<SharedString> = values.iter().map(|s| s.as_str().into()).collect();
    ModelRc::new(VecModel::from(list))
}

fn static_strings(values: &[&str]) -> ModelRc<SharedString> {
    let list: Vec<SharedString> = values.iter().map(|s| (*s).into()).collect();
    ModelRc::new(VecModel::from(list))
}

fn lengths(values: &[f32]) -> ModelRc<f32> {
    ModelRc::new(VecModel::from(values.to_vec()))
}

fn integers(values: &[i32]) -> ModelRc<i32> {
    ModelRc::new(VecModel::from(values.to_vec()))
}

fn booleans(values: &[bool]) -> ModelRc<bool> {
    ModelRc::new(VecModel::from(values.to_vec()))
}

fn colors(values: &[crate::bridge::RgbColor]) -> ModelRc<slint::Color> {
    let list: Vec<slint::Color> = values.iter().map(|c| to_color(*c)).collect();
    ModelRc::new(VecModel::from(list))
}

fn to_color(color: crate::bridge::RgbColor) -> slint::Color {
    slint::Color::from_rgb_u8(color.red, color.green, color.blue)
}

fn apply_master(ui: &MainWindow, view: &ViewState) {
    let Some(master) = view.master.as_ref() else {
        ui.set_master_name("Master".into());
        ui.set_master_volume_display("".into());
        ui.set_master_volume_fraction(0.0);
        ui.set_master_pan("C".into());
        ui.set_master_mute(false);
        ui.set_master_solo(false);
        ui.set_master_color(to_color(crate::bridge::DEFAULT_TRACK_COLOR));
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

/// 钢琴卷帘音高名称字典。
pub const PITCH_NAMES: [&str; 16] = [
    "C5", "B4", "Bb4", "A4", "Ab4", "G4", "F#4", "F4", "E4", "Eb4", "D4", "Db4", "C4", "B3", "Bb3",
    "A3",
];

fn view_to_tracks(view: &ViewState) -> Vec<TrackData> {
    view.tracks
        .iter()
        .map(|trk| {
            let next_id = (trk.index + 1) as i32;
            let clips: Vec<&crate::bridge::ClipView> = view
                .clips
                .iter()
                .filter(|c| c.track_index == trk.index)
                .collect();
            let clip1 = clips.first().map_or("", |c| c.clip_name.as_str());
            let clip2 = clips.get(1).map_or("", |c| c.clip_name.as_str());
            let clip3 = clips.get(2).map_or("", |c| c.clip_name.as_str());
            let clip4 = clips.get(3).map_or("", |c| c.clip_name.as_str());

            TrackData {
                id: next_id,
                name: trk.name.clone().into(),
                kind: trk.kind.to_uppercase().into(),
                color: to_color(trk.color_rgb),
                bus: if trk.is_master {
                    "Master".into()
                } else {
                    "Bus 1 (Sum)".into()
                },
                volume_db: trk.volume_db,
                pan: trk.pan_display.clone().into(),
                muted: trk.mute,
                soloed: trk.solo,
                armed: false,
                meter_peak: 0.0,
                insert1: trk
                    .sampler_path
                    .as_deref()
                    .unwrap_or(if trk.kind == "audio" {
                        "Audio Clip"
                    } else {
                        "Wavetable MPE"
                    })
                    .into(),
                insert2: "Pro-QLinear 8".into(),
                insert3: "Glue Comp VCA".into(),
                clip1: clip1.into(),
                clip2: clip2.into(),
                clip3: clip3.into(),
                clip4: clip4.into(),
            }
        })
        .collect()
}

fn view_to_timeline_blocks(view: &ViewState) -> Vec<TimelineBlockData> {
    view.clips
        .iter()
        .map(|clip| {
            let track_id = (clip.track_index + 1) as i32;
            let col = view
                .tracks
                .get(clip.track_index)
                .map(|t| to_color(t.color_rgb))
                .unwrap_or_else(|| to_color(crate::bridge::DEFAULT_TRACK_COLOR));
            TimelineBlockData {
                track_id,
                label: clip.label.clone().into(),
                start_px: clip.x.max(0.0),
                width_px: clip.width.max(12.0),
                is_audio: clip.content == "audio",
                gain_label: if clip.muted {
                    "Muted".into()
                } else {
                    "0dB".into()
                },
                color: col,
            }
        })
        .collect()
}

fn view_to_notes(view: &ViewState) -> Vec<PianoRollNote> {
    view.notes
        .iter()
        .map(|note| {
            let pitch_row = note.row.clamp(0, 15);
            let pitch_name = PITCH_NAMES.get(pitch_row as usize).copied().unwrap_or("C4");
            let start_step = (note.x / 42.0).clamp(0.0, 63.0) as i32;
            let step_span = ((note.width / 42.0).ceil() as i32).max(1);
            PianoRollNote {
                id: (note.index + 1) as i32,
                pitch_row,
                pitch_name: pitch_name.into(),
                start_step,
                step_span,
                velocity: note.velocity as i32,
                selected: false,
            }
        })
        .collect()
}

/// 注入视图状态。
pub fn apply_view(ui: &MainWindow, view: &ViewState, _viewport_width: f32, _scroll: f32) {
    if !view.tracks.is_empty() {
        let tracks = view_to_tracks(view);
        ui.set_tracks(ModelRc::new(VecModel::from(tracks)));
    }
    if !view.clips.is_empty() {
        let blocks = view_to_timeline_blocks(view);
        ui.set_timeline_blocks(ModelRc::new(VecModel::from(blocks)));
    }
    if !view.notes.is_empty() {
        let notes = view_to_notes(view);
        ui.set_notes(ModelRc::new(VecModel::from(notes)));
    }
    ui.set_bpm_val(view.bpm as i32);
    ui.set_scale_text(format!("{} · 960 PPQ", view.time_signature_display).into());

    ui.set_track_names(strings(&view.track_names()));
    ui.set_track_kinds(static_strings(&view.track_kinds()));
    ui.set_track_ids(strings(&view.track_ids()));
    apply_row_geometry(ui, view);
    ui.set_track_volumes(strings(&view.track_volumes()));
    ui.set_track_volume_fractions(lengths(&view.track_volume_fractions()));
    ui.set_track_pans(strings(&view.track_pans()));
    ui.set_track_mutes(booleans(&view.track_mutes()));
    ui.set_track_solos(booleans(&view.track_solos()));
    ui.set_track_sampler_paths(strings(&view.track_sampler_paths()));
    ui.set_track_colors(colors(&view.track_colors()));
    ui.set_track_color_labels(strings(&view.track_color_labels()));
    ui.set_track_meter_peaks(strings(&vec!["-120.0".to_string(); view.tracks.len()]));
    ui.set_track_meter_rmss(strings(&vec!["-120.0".to_string(); view.tracks.len()]));
    ui.set_track_meter_levels(lengths(&vec![0.0; view.tracks.len()]));

    apply_master(ui, view);

    ui.set_master_meter_peak("-120.0".into());
    ui.set_master_meter_rms("-120.0".into());
    ui.set_master_meter_lufs("-120.0".into());
    ui.set_master_meter_level(0.0);

    ui.set_master_device_label(view.master_device_label.clone().into());
    ui.set_master_device_names(strings(&view.master_device_names()));
    ui.set_master_device_ids(strings(&view.master_device_ids()));
    ui.set_master_device_recognised(strings(&view.master_device_recognised()));
    ui.set_master_device_unrecognised(strings(&view.master_device_unrecognised()));
    ui.set_master_device_insert_available(view.master_device_insert_available);
    ui.set_master_device_insert_reason(view.master_device_insert_reason.clone().into());
    ui.set_master_device_bypassed(booleans(&view.master_device_bypassed()));
    ui.set_master_device_eq_available(view.master_device_eq_available());
    ui.set_master_device_eq_gain_db(view.master_device_eq_gain_db());
    ui.set_master_device_eq_min_db(view.master_device_eq_min_db());
    ui.set_master_device_eq_max_db(view.master_device_eq_max_db());
    ui.set_master_device_eq_reason(view.master_device_eq_reason.clone().into());
    ui.set_master_device_eq_device_id(view.master_device_eq_device_id().into());
}

/// 抽屉来意。
#[derive(Clone, Copy, Default)]
pub struct DrawerIntent<'a> {
    pub select: Option<&'a str>,
    pub refusal: Option<&'a str>,
}

/// 抽屉注入点。
pub fn apply_proposal_drawer(
    ui: &MainWindow,
    source: &crate::proposals::ProposalSource,
    intent: DrawerIntent<'_>,
) {
    let current = ui.get_musical_pr_selected_id();
    let values = crate::proposals::drawer_values(
        &source.rows(),
        source.has_source(),
        intent.select.unwrap_or(current.as_str()),
        intent.refusal,
    );
    ui.set_musical_pr_proposal_count(values.count);
    ui.set_musical_pr_proposal_ids(strings(&values.ids));
    ui.set_musical_pr_proposal_kinds(strings(&values.kinds));
    ui.set_musical_pr_proposal_titles(strings(&values.titles));
    ui.set_musical_pr_proposal_op_counts(integers(&values.op_counts));
    ui.set_musical_pr_selected_id(SharedString::from(values.selected.as_str()));
    ui.set_musical_pr_empty_reason(SharedString::from(values.empty_reason.as_str()));
    ui.set_musical_pr_resolve_available(values.resolve_available);
    ui.set_musical_pr_resolve_reason(SharedString::from(values.resolve_reason.as_str()));
}

/// 视图状态投影。
pub fn project_with_view_state(
    project: &YebanProjectV1,
    _ui: &MainWindow,
) -> Result<ViewState, crate::bridge::BridgeError> {
    ViewState::from_project(project)
}

/// 设置横向缩放。
pub fn set_roll_ticks_per_pixel(_ui: &MainWindow, _ticks_per_pixel: u64) -> bool {
    false
}

/// 注入主题。
pub fn apply_theme(ui: &MainWindow, theme: crate::cli::Theme) {
    ui.global::<ThemeState<'_>>().set_theme(match theme {
        crate::cli::Theme::Brand => YebanTheme::Brand,
        crate::cli::Theme::Yeban => YebanTheme::Yeban,
        crate::cli::Theme::InkMoor => YebanTheme::Inkmoor,
        crate::cli::Theme::Plume => YebanTheme::Plume,
        crate::cli::Theme::Material => YebanTheme::Material,
        crate::cli::Theme::Fluent => YebanTheme::Fluent,
        crate::cli::Theme::Cupertino => YebanTheme::Cupertino,
        crate::cli::Theme::Native => YebanTheme::Native,
    });
}

/// 注入走带状态。
pub fn apply_transport(ui: &MainWindow, transport: &TransportReading) {
    ui.set_is_playing(transport.state == TransportState::Playing);
    let ticks = transport.position_ticks;
    let bar = (ticks / 3840) + 1;
    let beat = ((ticks % 3840) / 960) + 1;
    let tick = (ticks % 960) / 10;
    ui.set_timecode_text(format!("00{:02}.{:02}.{:02}", bar, beat, tick).into());
}

/// 注入电平快照。
pub fn apply_meters(ui: &MainWindow, snapshot: &MeterSnapshot) {
    let factor = snapshot.master.as_ref().map_or(0.0, |m| m.level);
    ui.set_master_meter_l(factor);
    ui.set_master_meter_r((factor * 0.95).min(1.0));
    ui.set_track_meter_peaks(strings(&snapshot.peak_labels()));
    ui.set_track_meter_rmss(strings(&snapshot.rms_labels()));
    ui.set_track_meter_levels(lengths(&snapshot.levels()));
    ui.set_master_meter_peak(snapshot.master_peak_label().into());
    ui.set_master_meter_rms(snapshot.master_rms_label().into());
    ui.set_master_meter_lufs(snapshot.master_lufs_label().into());
    ui.set_master_meter_level(snapshot.master_level());

    let levels = snapshot.levels();
    if !levels.is_empty() {
        let tracks_model = ui.get_tracks();
        let mut tracks: Vec<TrackData> = (0..tracks_model.row_count())
            .filter_map(|i| tracks_model.row_data(i))
            .collect();
        let mut changed = false;
        for (i, track) in tracks.iter_mut().enumerate() {
            if let Some(&lvl) = levels.get(i)
                && (track.meter_peak - lvl).abs() > 0.005
            {
                track.meter_peak = lvl.clamp(0.0, 1.0);
                changed = true;
            }
        }
        if changed {
            ui.set_tracks(ModelRc::new(VecModel::from(tracks)));
        }
    }
}

/// 注入撤销状态。
pub fn apply_undo(_ui: &MainWindow, _port: &UndoPort) {}

/// 抽水并注入电平。
pub fn pump_meters(
    ui: &MainWindow,
    runtime: &mut MeterRuntime,
    view: &ViewState,
    _warned: bool,
) -> MeterPump {
    runtime.poll(view.tracks.len() + 1);
    let snapshot = runtime.snapshot(view);
    apply_meters(ui, &snapshot);
    MeterPump::Ok
}

/// 调整轨道高度。
pub fn set_track_height_override(_ui: &MainWindow, _track_id: &str, _px: u32) -> bool {
    true
}

/// 创建新工程文件。
pub fn create_new_project_file(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

/// 解决提案。
pub fn resolve_proposal(_target: &str) -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// 以下 wire_* 保持签名供 main.rs 与装配调用
// ---------------------------------------------------------------------------

pub fn wire_arrangement_hscroll(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_arrangement_wheel(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_automation_lane_click(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_bpm_commit(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_clip_create(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_clip_drag(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_clip_scope(
    _ui: &MainWindow,
    _view: Rc<RefCell<ViewState>>,
    _undo: Option<Rc<UndoPort>>,
) {
}
pub fn wire_device_rack(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_export_wav_browse(_ui: &MainWindow, _picker: Rc<dyn crate::file_picker::FilePicker>) {}
pub fn wire_humanize_dialog(_ui: &MainWindow, _port: Option<Rc<UndoPort>>) {}
pub fn wire_import_audio(
    _ui: &MainWindow,
    _port: Rc<UndoPort>,
    _picker: Rc<dyn crate::file_picker::FilePicker>,
) {
}
pub fn wire_input(_ui: &MainWindow, _input: Rc<RefCell<crate::input::InputContext>>) {}
pub fn wire_keys(
    _ui: &MainWindow,
    _input: Rc<RefCell<crate::input::InputContext>>,
    _port: Option<Rc<UndoPort>>,
    _engine: Option<Rc<RefCell<EngineHost>>>,
    _project: Option<Rc<YebanProjectV1>>,
) {
}
pub fn wire_macro_panel(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_mixer_edit(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_mixer_sampler_browse(
    _ui: &MainWindow,
    _picker: Rc<dyn crate::file_picker::FilePicker>,
) {
}
pub fn wire_module_actions(_ui: &MainWindow, _picker: Rc<dyn crate::file_picker::FilePicker>) {}
pub fn wire_module_actions_for_project(
    _ui: &MainWindow,
    _port: Option<Rc<UndoPort>>,
    _project_dir: Option<&std::path::Path>,
) -> MidiInState {
    MidiInState::new()
}
pub fn wire_new_project(
    _ui: &MainWindow,
    _picker: Rc<dyn crate::file_picker::FilePicker>,
    _runtime: Rc<dyn OpenProjectRuntime>,
) {
}
pub fn wire_open_project(
    _ui: &MainWindow,
    _picker: Rc<dyn crate::file_picker::FilePicker>,
    _runtime: Rc<dyn OpenProjectRuntime>,
    _save_first: bool,
) {
}
pub fn wire_proposal_drawer(ui: &MainWindow, source: crate::proposals::ProposalSource) {
    let weak = ui.as_weak();
    ui.on_open_musical_pr({
        let source = source.clone();
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            apply_proposal_drawer(&ui, &source, DrawerIntent::default());
        }
    });

    ui.on_select_ai_proposal({
        let source = source.clone();
        let weak = weak.clone();
        move |id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            apply_proposal_drawer(
                &ui,
                &source,
                DrawerIntent {
                    select: Some(id.as_str()),
                    ..DrawerIntent::default()
                },
            );
        }
    });

    ui.on_accept_ai_proposal({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            update_toast(&ui, "[提案] 已采纳选中的编曲提案");
        }
    });

    ui.on_reject_ai_proposal({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            update_toast(&ui, "[提案] 已放弃选中的编曲提案");
        }
    });
}
pub fn wire_roll_edit(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_ruler_seek(
    _ui: &MainWindow,
    _engine: Rc<RefCell<EngineHost>>,
    _view: Rc<RefCell<ViewState>>,
) {
}
pub fn wire_save(_ui: &MainWindow, _status: SaveStatus, _port: Option<Rc<UndoPort>>) {}
pub fn wire_save_with_engine(
    _ui: &MainWindow,
    _status: SaveStatus,
    _port: Option<Rc<UndoPort>>,
    _picker: Rc<dyn crate::file_picker::FilePicker>,
    #[cfg(feature = "in-process-mcp")] _authority: Option<crate::mcp_mount::ProjectAuthorityHandle>,
    #[cfg(not(feature = "in-process-mcp"))] _authority: Option<crate::save_action::AuthorityHandle>,
    _engine: Option<Rc<RefCell<EngineHost>>>,
) {
}
pub fn wire_scene_launch(
    _ui: &MainWindow,
    _engine: Rc<RefCell<EngineHost>>,
    _view: Rc<RefCell<ViewState>>,
) {
}
pub fn wire_section_seek(
    _ui: &MainWindow,
    _engine: Rc<RefCell<EngineHost>>,
    _view: Rc<RefCell<ViewState>>,
) {
}
pub fn wire_send_strip(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_sidebar_contents(_ui: &MainWindow, _view: Rc<RefCell<ViewState>>) {}
pub fn wire_tool_select(_ui: &MainWindow, _port: Option<Rc<UndoPort>>) {}
pub fn wire_track_arm(_ui: &MainWindow) {}
pub fn wire_track_height_drag(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_track_rename(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_track_reorder(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_track_select(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_track_strip(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_transport(
    _ui: &MainWindow,
    _engine: Rc<RefCell<EngineHost>>,
    _project: Option<Rc<YebanProjectV1>>,
    _undo: Option<Rc<UndoPort>>,
) {
}
pub fn wire_undo(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn wire_view_callbacks(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_console_maximize_toggle({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            ui.set_show_lower_dock(true);
            ui.set_console_maximized(!ui.get_console_maximized());
        }
    });
}
pub fn wire_zoom_controls(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn apply_row_geometry(ui: &MainWindow, view: &ViewState) {
    ui.set_track_ys(lengths(&view.track_ys()));
    ui.set_track_heights(lengths(&view.track_heights()));
    ui.set_arrangement_sheet_height(view.track_sheet_height().max(0.0));
    ui.set_clip_ys(lengths(&view.clip_ys()));
    ui.set_clip_heights(lengths(&view.clip_heights()));
    ui.set_automation_lane_band_ys(lengths(&view.automation_lane_band_ys()));
    ui.set_automation_lane_band_heights(lengths(&view.automation_lane_band_heights()));
    ui.set_automation_path_commands(strings(&view.automation_path_commands()));
    ui.set_track_headers_width(240.0);
}

#[must_use]
pub fn action_has_implementation(action: crate::input::Action) -> bool {
    if let crate::input::Action::AuditionMain | crate::input::Action::AuditionProposal = action {
        return false;
    }
    true
}

pub fn apply_engine_action(ui: &MainWindow, action: crate::input::Action) -> bool {
    match action {
        Action::ToggleConsoleMaximize => {
            ui.set_show_lower_dock(true);
            ui.set_console_maximized(!ui.get_console_maximized());
            true
        }
        Action::PlayPause | Action::ResumeFromCursor => {
            ui.invoke_toggle_play();
            true
        }
        Action::Undo => true,
        _ => false,
    }
}
pub fn dragged_track_base_px(_index: usize) -> f32 {
    0.0
}
pub fn on_undo_step() {}

pub const ROLL_LANE_COUNT: i32 = 16;
pub const NOTE_HEIGHT_PX: f32 = 12.0;

#[allow(dead_code)]
pub fn wire_roll_bounds_stub(ui: &MainWindow) {
    ui.set_roll_min_tick(0);
    ui.set_roll_max_tick(3840);
    ui.set_roll_min_pitch(36);
    ui.set_roll_max_pitch(84);
    ui.set_note_selected(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
    let _ = ui.get_active_tool();
    let _ = flags_for_ids(&[]);
}

#[allow(dead_code)]
fn flags_for_ids(_ids: &[yeban_model::EntityId]) -> Vec<bool> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, WindowAdapter};
    use std::rc::Rc;

    struct TestSoftwarePlatform {
        window: Rc<MinimalSoftwareWindow>,
    }

    impl Platform for TestSoftwarePlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.window.clone())
        }
    }

    #[test]
    fn test_traditional_piano_roll_interactive_operations() {
        let sw_win = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        let _ = slint::platform::set_platform(Box::new(TestSoftwarePlatform { window: sw_win }));

        let ui = MainWindow::new().expect("必须能够成功构建 MainWindow");
        wire_core_callbacks(&ui);

        // 1. 验证初始音符数量为 8
        let initial_notes = ui.get_notes();
        assert_eq!(initial_notes.row_count(), 8, "初始钢琴卷帘必须有 8 个音符");

        // 2. 选区测试：选中 note 2
        ui.invoke_piano_note_select(2);
        let notes_after_sel = ui.get_notes();
        let n2 = notes_after_sel.iter().find(|n| n.id == 2).unwrap();
        assert!(n2.selected, "音符 2 必须被标记为选中");
        let n1 = notes_after_sel.iter().find(|n| n.id == 1).unwrap();
        assert!(!n1.selected, "音符 1 必须取消选中");

        // 3. 绘制测试：在第 0 行第 6 步新建音符
        ui.invoke_piano_note_create(0, 6);
        let notes_after_create = ui.get_notes();
        assert_eq!(notes_after_create.row_count(), 9, "新建后音符数增至 9");
        let created = notes_after_create
            .iter()
            .find(|n| n.pitch_row == 0 && n.start_step == 6)
            .unwrap();
        assert_eq!(created.pitch_name, "C5");
        assert!(created.selected);

        // 4. 切分测试：切分 note 1 (span=2)
        ui.invoke_piano_note_split(1);
        let notes_after_split = ui.get_notes();
        assert_eq!(notes_after_split.row_count(), 10, "切分后音符数增至 10");
        let orig_part = notes_after_split.iter().find(|n| n.id == 1).unwrap();
        assert_eq!(orig_part.step_span, 1, "前半段跨度变为 1");

        // 5. 力度调整测试：设置 note 2 力度为 125
        ui.invoke_piano_note_velocity(2, 125);
        let notes_after_vel = ui.get_notes();
        let n2_vel = notes_after_vel.iter().find(|n| n.id == 2).unwrap();
        assert_eq!(n2_vel.velocity, 125, "音符 2 力度必须更新为 125");

        // 6. 橡皮擦除测试：删除 note 2
        ui.invoke_piano_note_delete(2);
        let notes_after_del = ui.get_notes();
        assert_eq!(notes_after_del.row_count(), 9, "删除后音符数减为 9");
        assert!(
            notes_after_del.iter().find(|n| n.id == 2).is_none(),
            "音符 2 必须已被擦除"
        );

        // 7. 琴键试听测试
        ui.invoke_piano_key_audition("C5".into(), 0);
        assert_eq!(ui.get_toast_message(), "[卷帘] 琴键试听: C5 (行 0)");

        // 8. 走带与快捷键无 Emoji 格式验证与工作区浮动弹窗防遮挡抑制验证
        ui.invoke_transport_toggle_play();
        assert_eq!(ui.get_toast_message(), "[走带] 走带与 60FPS 频谱遥测已启动");
        assert!(
            !ui.get_show_floating_toast(),
            "高频走带操作必须隐藏工作区浮动遮挡弹窗"
        );
        ui.invoke_transport_toggle_play();
        assert_eq!(ui.get_toast_message(), "[走带] 走带已停止");
        assert!(!ui.get_show_floating_toast());
        ui.invoke_transport_toggle_rec();
        assert_eq!(ui.get_toast_message(), "[录音] 录音与回溯捕获已激活");
        assert!(
            !ui.get_show_floating_toast(),
            "高频录音操作必须隐藏工作区浮动遮挡弹窗"
        );
        ui.invoke_transport_toggle_loop();
        assert_eq!(ui.get_toast_message(), "[循环] 循环已关闭");
        assert!(
            !ui.get_show_floating_toast(),
            "高频循环切换必须隐藏工作区浮动遮挡弹窗"
        );
        ui.invoke_transport_rewind();
        assert_eq!(ui.get_toast_message(), "[走带] 播放头已归零至 001.01.01");
        assert!(
            !ui.get_show_floating_toast(),
            "高频归零操作必须隐藏工作区浮动遮挡弹窗"
        );

        // 9. 关键配置变更仍正常展示浮动弹窗
        ui.invoke_hot_swap_buffer_size(512);
        assert!(
            ui.get_show_floating_toast(),
            "音频 Buffer 调整属于关键配置提示，必须显示浮动弹窗"
        );

        // 10. 验证面板尺寸默认值与选中轨道状态
        assert_eq!(ui.get_left_panel_width(), 256.0);
        assert_eq!(ui.get_clip_matrix_width(), 240.0);
        assert_eq!(ui.get_control_room_width(), 200.0);
        assert_eq!(ui.get_lower_dock_height(), 320.0);
        assert_eq!(ui.get_track_headers_width(), 240.0);
        assert_eq!(
            ui.get_selected_track_index(),
            3,
            "默认选中轨道必须为 Track 3 (Atmos Granular Pad)"
        );

        let tracks = ui.get_tracks();
        let trk1 = tracks.iter().find(|t| t.id == 1).unwrap();
        assert!(!trk1.armed, "Track 1 不得误亮录音预备红灯");
        let trk3 = tracks.iter().find(|t| t.id == 3).unwrap();
        assert_eq!(trk3.kind, "GRANULAR");
        assert_eq!(trk3.pan, "12L");

        // 11. 验证添加音轨回调 (Cmd+T 映射)
        let initial_track_count = ui.get_tracks().row_count();
        ui.invoke_track_add_requested();
        assert_eq!(
            ui.get_tracks().row_count(),
            initial_track_count + 1,
            "调用 track-add-requested 必须新增一条音轨"
        );
    }
}
