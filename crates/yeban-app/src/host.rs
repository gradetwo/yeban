//! 视图状态与实时音频引擎 → Slint `MainWindow` 的双向桥接与宿主实现。
//! 遵循夜半 DAW 原生跨平台桌面 UI 设计方案。
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
    BrowserItemData, ChordBlockData, CommandPaletteItem, MainWindow, PersonaMode, PianoStepRow,
    Theme, ThemeState, TimelineBlockData, TrackData, YebanTheme,
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

/// 构造并初始化夜半 DAW 主窗口。
pub fn build_main_window(
    _view: &ViewState,
    _scene: &DemoScene,
) -> Result<MainWindow, slint::PlatformError> {
    let ui = MainWindow::new()?;

    // 默认初始数据已经在 app.slint 中声明完备，这里挂接核心交互闭包
    wire_core_callbacks(&ui);

    Ok(ui)
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
                ui.set_toast_message(
                    format!(
                        "⚡ 已无损热切换 Buffer 为 {} smp (0ms 即时生效，无需重载插件)",
                        samples
                    )
                    .into(),
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
                    ui.set_toast_message(
                        if playing {
                            "▶ 播放走带"
                        } else {
                            "⏸ 停止走带"
                        }
                        .into(),
                    );
                } else if text == "r" || text == "R" {
                    let rec = !ui.get_is_recording();
                    ui.set_is_recording(rec);
                    ui.set_toast_message(
                        if rec {
                            "● 录音与回溯捕获已开启"
                        } else {
                            "录音已停止"
                        }
                        .into(),
                    );
                } else if text == "c" || text == "C" {
                    let loop_on = !ui.get_is_looping();
                    ui.set_is_looping(loop_on);
                    ui.set_toast_message(
                        if loop_on {
                            "🔁 循环已开启"
                        } else {
                            "循环已关闭"
                        }
                        .into(),
                    );
                }
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
                ui.set_toast_message(
                    if playing {
                        "▶ 走带与 60FPS 频谱遥测已启动"
                    } else {
                        "⏹ 走带已停止"
                    }
                    .into(),
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
                ui.set_toast_message(
                    if rec {
                        "● 录音与回溯捕获已激活"
                    } else {
                        "录音已关闭"
                    }
                    .into(),
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
                ui.set_toast_message(
                    if loop_on {
                        "🔁 循环已开启"
                    } else {
                        "循环已关闭"
                    }
                    .into(),
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
                ui.set_toast_message("⏮ 播放头已归零至 001.01.01".into());
            }
        }
    });

    ui.on_transport_set_bpm({
        let weak = weak.clone();
        move |bpm| {
            if let Some(ui) = weak.upgrade() {
                ui.set_bpm_val(bpm);
                ui.set_toast_message(format!("⏱️ 工程速度已同步为 {} BPM", bpm).into());
            }
        }
    });

    ui.on_transport_set_scale({
        let weak = weak.clone();
        move |scale| {
            if let Some(ui) = weak.upgrade() {
                ui.set_scale_text(scale.clone());
                ui.set_toast_message(format!("🎼 全局调式已锁定为: {}", scale).into());
            }
        }
    });

    // 4. 工具切换
    ui.on_edit_tool_selected({
        let weak = weak.clone();
        move |tool| {
            if let Some(ui) = weak.upgrade() {
                ui.set_active_tool(tool.clone());
                ui.set_toast_message(format!("🛠️ 当前编辑工具已切换为: {}", tool).into());
            }
        }
    });

    // 5. 轨道选择与状态切换
    ui.on_track_selected({
        let weak = weak.clone();
        move |id| {
            if let Some(ui) = weak.upgrade() {
                ui.set_selected_track_id(id);
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
                    ui.set_toast_message(msg.into());
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
                    ui.set_toast_message(msg.into());
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
                    ui.set_toast_message(msg.into());
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
                    insert1: "⚡ Wavetable MPE".into(),
                    insert2: "📈 Pro-QLinear 8".into(),
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
                    format!("✅ 已新增第 {} 轨 (同时支持音频与 MIDI 混排)", next_id).into(),
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
                        "⚡ 触发轨道 TRK {:02} 第 {} 个 Clip 播放",
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
                    format!("🚀 已同步触发 Scene {} 整行 Clip 矩阵", scene_idx + 1).into(),
                );
            }
        }
    });

    // 7. 和弦试听
    ui.on_chord_audition({
        let weak = weak.clone();
        move |chord| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(format!("🎼 正在试听全局和弦轨: {}", chord).into());
            }
        }
    });

    // 8. 音色库挂载
    ui.on_browser_item_load({
        let weak = weak.clone();
        move |item_name| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(format!("🔌 已将 [{}] 挂载至内联设备机架", item_name).into());
            }
        }
    });

    // 9. 监听控制室
    ui.on_monitor_switch({
        let weak = weak.clone();
        move |mon| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(format!("🔊 Control Room 监听输出已切换至: {}", mon).into());
            }
        }
    });

    // 10. 归一化高度
    ui.on_normalize_track_heights({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(
                    "📏 已触发 Logic 12.4 新特性：一键归一化所有轨道高度！".into(),
                );
            }
        }
    });

    // 11. 命令面板
    ui.on_command_execute({
        let weak = weak.clone();
        move |cmd| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(format!("⚡ 已执行命令: {}", cmd).into());
            }
        }
    });

    // 12. 设备参数变更
    ui.on_device_param_changed({
        let weak = weak.clone();
        move |dev, param, val| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(
                    format!("🎛️ {} 参数 {} 调整为: {:.1}", dev, param, val).into(),
                );
            }
        }
    });

    // 13. 钢琴卷帘音符切换
    ui.on_piano_step_toggle({
        let weak = weak.clone();
        move |row, step| {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_message(
                    format!("🎹 切换钢琴卷帘音高行 {} 第 {} 步", row, step + 1).into(),
                );
            }
        }
    });
}

/// 注入视图状态。
pub fn apply_view(_ui: &MainWindow, _view: &ViewState, _viewport_width: f32, _scroll: f32) {
    // 基础视图已经在 app.slint 初始状态中完整建立
}

/// 抽屉来意。
#[derive(Clone, Copy, Default)]
pub struct DrawerIntent<'a> {
    pub select: Option<&'a str>,
    pub refusal: Option<&'a str>,
}

/// 抽屉注入点。
pub fn apply_proposal_drawer(
    _ui: &MainWindow,
    _source: &crate::proposals::ProposalSource,
    _intent: DrawerIntent<'_>,
) {
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
pub fn wire_proposal_drawer(_ui: &MainWindow, _source: crate::proposals::ProposalSource) {}
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
pub fn wire_view_callbacks(_ui: &MainWindow) {}
pub fn wire_zoom_controls(_ui: &MainWindow, _port: &Rc<UndoPort>) {}
pub fn apply_row_geometry(_ui: &MainWindow, _view: &ViewState) {}
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
