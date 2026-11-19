//! TUI 应用状态与动作分发。
//!
//! 接缝（路线甲）：工程读写走 `yeban_app::open` / `yeban_app::save` 的生产路径；
//! 引擎与走带走 `yeban_app::engine_host::EngineHost`（与 GUI 同一个宿主，无 Slint
//! 依赖）；导出走 `yeban_app::render_core` + `yeban_app::export_wav` 的生产渲染路径；
//! 共享键位走 `yeban_app::input::InputContext::resolve`（GUI 策略表的唯一解析器）。
//!
//! 二期起，编辑（静音/独奏/音量/声相/改名/片段移动与删除/速度）一律经
//! `yeban_app::undo::UndoPort::commit_ops` 提交（与 GUI 同一个撤销端口、同一种
//! `Op`），撤销/重做走 `UndoPort::perform`；工程文档以端口的当前版为准
//! （`port.project()`），本地 `project` 只是它的镜像。保存时把提交图谱写回
//! 归档的 `history.dag`，重开后历史可恢复（`session_from_archive`）。

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use yeban_app::engine_host::{EditMark, EngineHost};
use yeban_app::export_wav::WavExportOptions;
use yeban_app::input::{Action, InputContext, MixerParam, Resolution};
use yeban_app::undo::{ActionOutcome, UiAction, UndoPort};
use yeban_engine::meter::{MeterCollector, MeterFrame};
use yeban_engine::ring::TransportCommand;
use yeban_engine::transport::{TransportReading, TransportState};
use yeban_model::container::ProjectArchive;
use yeban_model::ids::EntityId;
use yeban_model::music::MidiNote;
use yeban_model::ops::Op;
use yeban_model::project::{
    AutomationTarget, ClipContent, ClipPlacement, ClipPoolEntry, MAX_BPM, MIN_BPM, TrackKind,
    YebanProjectV1,
};

use crate::keymap::{Mode, Pane, ViMachine, ViMotion, ViOutcome, to_logical};

/// 一次导出 WAV 的证据读数（回执与测试用）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportReceipt {
    /// 落盘字节数。
    pub bytes: u64,
    /// 渲染帧数。
    pub frames: u64,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 位深（bits per sample）。
    pub bits: u16,
}

/// 片段网格窗格的列表视图（三期）：普通网格之外的两种内容变体。
/// 焦点循环仍是三窗格（K6 不变），视图只是网格窗格里换了内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListView {
    /// 音符列表：对象在打开时捕获（轨道 / 摆放 / 池条目），视图内编辑
    /// 该池条目的音符（`selected` 是按 (起点, 音高, 身份) 排序后的下标）。
    Notes {
        /// 音符所属轨道（op 寻址用）。
        track_id: EntityId,
        /// 打开视图的摆放（加音符时换算片段内相对起点用）。
        placement_id: EntityId,
        /// 池条目身份（音符住在条目的 content 里）。
        clip_id: EntityId,
        /// 选中音符的下标。
        selected: usize,
    },
    /// 片段池列表：`selected` 是按 (名, 身份) 排序后的下标。
    Pool {
        /// 选中条目的下标。
        selected: usize,
    },
}

/// 改名目标（三期起 Insert 改名有两种对象：轨道 / 片段池条目）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenameTarget {
    /// 轨道名（`Op::SetTrackName`，二期行为）。
    Track(EntityId),
    /// 片段池条目名（`Op::SetClipName`，三期新增）。
    Clip(EntityId),
}

/// 当前墙钟（毫秒，Unix 纪元）：撤销会话的提交时间戳来源（与 GUI 同一口径，
/// 只作提交元数据，不进工程语义）。
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// 应用状态。
pub struct App {
    /// 当前工程文档（`UndoPort` 当前版的镜像：每次提交/撤销后从端口刷新）。
    pub project: YebanProjectV1,
    /// 撤销端口（与 GUI 同一个 `UndoPort`：编辑提交与撤销/重做的唯一落点）。
    port: Rc<UndoPort>,
    /// 打开时保住的归档资产（保存时走全保真路径，导出时喂给渲染）。
    archive: Option<ProjectArchive>,
    /// 当前工程文件路径（`None` = 内存中的演示工程，尚未落盘）。
    pub project_path: Option<PathBuf>,
    /// 引擎宿主（控制面模式：不开音频设备，墙钟泵送推进走带）。
    host: EngineHost,
    /// reload 时领到的电平消费端（本宿主负责抽干，纪律同 GUI）。
    collector: Option<MeterCollector>,
    /// 当前模态。
    pub mode: Mode,
    /// 当前窗格焦点。
    pub focus: Pane,
    /// 显示轨序（含主总线，恒在末尾）：`display_track_ids()` + master。
    pub track_ids: Vec<EntityId>,
    /// 选中轨在 `track_ids` 里的下标。
    pub selected: usize,
    /// 小节光标（网格列光标，0 起）。
    pub cursor_bar: usize,
    /// 网格横向窗口起点（小节）。
    pub bar_offset: usize,
    /// 每小节占的终端列数（缩放：2..=8）。
    pub bar_cols: usize,
    /// Vi 状态机（计数前缀 / gg）。
    vi: ViMachine,
    /// GUI 键位解析上下文（画布聚焦；编辑态切文本域语义后我们绕过解析）。
    input_ctx: InputContext,
    /// 命令行缓冲（Command 模式）。
    pub cmdline: String,
    /// 改名缓冲（Insert 模式；`None` = 未在改名）。
    pub rename: Option<String>,
    /// 改名目标（与 `rename` 同生同灭；`None` = 未在改名）。
    rename_target: Option<RenameTarget>,
    /// 进程内片段剪贴板（三期；与 GUI 同形状：池条目 + 源摆放快照）。
    pub clipboard: Option<(ClipPoolEntry, ClipPlacement)>,
    /// 片段网格的列表视图（三期；`None` = 普通网格）。
    pub list_view: Option<ListView>,
    /// 日志行（最新在尾；底栏显示最后一条）。
    pub log: Vec<String>,
    /// 有未保存修改。
    pub dirty: bool,
    /// 主总线最新峰值（线性幅度，带衰减保持，供底栏电平条）。
    pub master_peak: f32,
    /// 是否请求退出（主循环读）。
    pub quit: bool,
    /// 上次导出证据。
    pub last_export: Option<ExportReceipt>,
}

impl App {
    /// 以演示工程起一个新会话（无文件路径）。
    ///
    /// # Errors
    /// 引擎图构建失败时返回错误文本。
    pub fn with_demo() -> Result<Self, String> {
        let project = yeban_model::samples::demo_project();
        let assets = yeban_model::samples::demo_project_assets();
        let archive = ProjectArchive {
            project: project.clone(),
            history_dag: Vec::new(),
            assets,
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        };
        Self::assemble(project, Some(archive), None)
    }

    /// 以给定工程起一个新会话（无文件路径；资产走空归档）。
    /// 测试夹具与嵌入场景用；生产路径是 [`Self::with_demo`] / [`Self::open_file`]。
    ///
    /// # Errors
    /// 引擎图构建失败时返回错误文本。
    pub fn with_project(project: YebanProjectV1) -> Result<Self, String> {
        Self::assemble(project, None, None)
    }

    /// 从文件打开工程（全保真归档路径）。
    ///
    /// # Errors
    /// 文件读不到 / 容器被拒 / 引擎图构建失败时返回错误文本。
    pub fn open_file(path: &Path) -> Result<Self, String> {
        let archive = yeban_app::open::open_project_archive_file(
            path,
            &yeban_app::open::ProjectOpenOptions::default(),
        )
        .map_err(|error| format!("打开失败：{error}"))?;
        let project = archive.project.clone();
        Self::assemble(project, Some(archive), Some(path.to_path_buf()))
    }

    fn assemble(
        project: YebanProjectV1,
        archive: Option<ProjectArchive>,
        project_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        // 撤销会话：与 GUI 共用同一份配方（`session_from_archive`）：归档里有
        // 合法 `history.dag` 就恢复历史，空图谱开新会话，非法图谱如实报错。
        let archive = archive.unwrap_or_else(|| ProjectArchive {
            project: project.clone(),
            history_dag: Vec::new(),
            assets: Vec::new(),
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        });
        let source = project_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<demo>".to_owned());
        let session = yeban_app::undo::session_from_archive(&source, &archive, now_ms())?;
        let port = Rc::new(UndoPort::new(session));
        let project = port.project();
        let mut host = EngineHost::new();
        if let Some(dir) = project_path.as_deref().and_then(Path::parent) {
            host.set_project_dir(Some(dir.to_path_buf()));
        }
        let rebuild = host
            .reload(&project, 1)
            .map_err(|error| format!("引擎图构建失败：{error:?}"))?;
        // 与 GUI 同一条纪律：加载即停住（reload 本身不碰走带状态）。
        host.stop();
        // 初始快照已经反映开局标记（与 GUI 装配同一条，首笔发布走增量）。
        host.mark_applied(EditMark::from_display(&port.display()));
        let mut app = Self {
            project,
            port,
            archive: Some(archive),
            project_path,
            host,
            collector: Some(rebuild.collector),
            mode: Mode::Normal,
            focus: Pane::Tracks,
            track_ids: Vec::new(),
            selected: 0,
            cursor_bar: 0,
            bar_offset: 0,
            bar_cols: 4,
            vi: ViMachine::new(),
            input_ctx: InputContext::new(),
            cmdline: String::new(),
            rename: None,
            rename_target: None,
            clipboard: None,
            list_view: None,
            log: Vec::new(),
            dirty: false,
            master_peak: 0.0,
            quit: false,
            last_export: None,
        };
        app.refresh_track_ids();
        app.log_line(format!(
            "已载入「{}」（{} 轨，{:.0} BPM）",
            app.project.title,
            app.track_ids.len(),
            app.project.bpm
        ));
        Ok(app)
    }

    // ------------------------------------------------------------------
    // 基本读数
    // ------------------------------------------------------------------

    /// 走带读数（引擎镜面，非界面自造状态）。
    #[must_use]
    pub fn transport(&self) -> TransportReading {
        self.host.transport()
    }

    /// 是否播放中。
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.transport().state == TransportState::Playing
    }

    /// 每拍 tick 数（按拍号分母换算：拍 = 分母音符）。
    #[must_use]
    pub fn ticks_per_beat(&self) -> u64 {
        let denominator = u64::from(self.project.time_signature.denominator.max(1));
        yeban_model::ids::PPQ * 4 / denominator
    }

    /// 每小节 tick 数。
    #[must_use]
    pub fn bar_ticks(&self) -> u64 {
        self.ticks_per_beat() * u64::from(self.project.time_signature.numerator.max(1))
    }

    /// 工程总长（tick）：最晚的片段结尾，至少一小节。
    #[must_use]
    pub fn project_len_ticks(&self) -> u64 {
        let mut end = 0_u64;
        for track in self.project.tracks.values() {
            for clip in track.clips.values() {
                end = end.max(clip.start_tick.saturating_add(clip.duration_ticks));
            }
        }
        end.max(self.bar_ticks())
    }

    /// 工程总小节数（向上取整）。
    #[must_use]
    pub fn total_bars(&self) -> usize {
        let bar = self.bar_ticks().max(1);
        usize::try_from(self.project_len_ticks().div_ceil(bar)).unwrap_or(usize::MAX)
    }

    /// 当前位置（小节，拍，tick 余数），均 0 起。
    #[must_use]
    pub fn position_bar_beat(&self) -> (u64, u64, u64) {
        let ticks = self.transport().position_ticks;
        let per_beat = self.ticks_per_beat().max(1);
        let per_bar = self.bar_ticks().max(1);
        (
            ticks / per_bar,
            (ticks % per_bar) / per_beat,
            ticks % per_beat,
        )
    }

    /// 选中轨身份。
    #[must_use]
    pub fn selected_track_id(&self) -> Option<EntityId> {
        self.track_ids.get(self.selected).copied()
    }

    /// 日志追加一行（底栏显示最后一条；保留最近 50 条）。
    pub fn log_line(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > 50 {
            self.log.remove(0);
        }
    }

    fn refresh_track_ids(&mut self) {
        let mut ids = self.project.display_track_ids();
        let master = self.project.master_bus_track_id;
        if self.project.tracks.contains_key(&master) {
            ids.push(master);
        }
        self.track_ids = ids;
        if self.selected >= self.track_ids.len() {
            self.selected = self.track_ids.len().saturating_sub(1);
        }
    }

    // ------------------------------------------------------------------
    // 每跳驱动（主循环以约 16 ms 一跳调用）
    // ------------------------------------------------------------------

    /// 墙钟泵送 + 快照回收心跳 + 电平抽干。播放时位置按真实时间推进（控制面模式，无声）。
    pub fn tick(&mut self) {
        self.host.pump_wall_clock();
        let _ = self.host.heartbeat();
        if let Some(collector) = &mut self.collector {
            let mut scratch = vec![MeterFrame::default(); 128];
            let drained = collector.drain_latest(&mut scratch);
            let master = self.project.master_bus_track_id;
            for frame in &scratch[..drained] {
                if frame.node == master {
                    self.master_peak = frame.peak.max(self.master_peak * 0.92);
                }
            }
            if drained == 0 {
                self.master_peak *= 0.92;
            }
        }
    }

    // ------------------------------------------------------------------
    // 走带与编辑动作
    // ------------------------------------------------------------------

    /// 播放/暂停切换（GUI `Space` 的落点，与 GUI 同一个宿主方法）。
    pub fn toggle_play(&mut self) {
        let reading = self.host.toggle_play();
        self.log_line(match reading.state {
            TransportState::Playing => "播放".to_owned(),
            _ => "暂停".to_owned(),
        });
    }

    /// 停止（位置保留）。
    pub fn stop(&mut self) {
        self.host.stop();
        self.log_line("停止".to_owned());
    }

    /// 回到起点（GUI `Enter` 的语义：定位到 0，不改播放状态）。
    pub fn go_to_beginning(&mut self) {
        self.host.send_transport(&[TransportCommand::SeekTicks(0)]);
        self.cursor_bar = 0;
        self.log_line("回到起点".to_owned());
    }

    /// 一次编辑的唯一落点：经撤销端口提交 `ops`（进提交史、可撤销），
    /// 成功后以端口当前版刷新镜像并增量发布到引擎。
    ///
    /// `message` 是提交信息（英文，进 `history.dag`）；`what` 是状态行的人话
    /// （中文）。提交被拒（模型校验不过）时工程一位不动、如实记日志。
    fn commit_edit(&mut self, message: &str, ops: Vec<Op>, what: &str) {
        match self.port.commit_ops(now_ms(), message, ops) {
            Ok(_) => {
                self.sync_from_port();
                self.publish_snapshot(what);
            }
            Err(refusal) => self.log_line(format!("{what} 被拒绝：{refusal}")),
        }
    }

    /// 以撤销端口的当前版刷新本地镜像（提交/撤销/重做后必走）。
    fn sync_from_port(&mut self) {
        self.project = self.port.project();
        self.refresh_track_ids();
        // 音符视图的对象若已不在池中（撤销删池条目等），关掉视图，
        // ⛔ 不留一个指向不存在片段的编辑面。
        if let Some(ListView::Notes { clip_id, .. }) = self.list_view
            && !self.project.clip_pool.contains_key(&clip_id)
        {
            self.list_view = None;
        }
    }

    /// 把当前镜像增量发布到引擎（标记取端口显示态，与 GUI 同一口径）。
    fn publish_snapshot(&mut self, what: &str) {
        let mark = EditMark::from_display(&self.port.display());
        match self.host.publish_project(&self.project, mark) {
            Ok(_) => {
                self.dirty = true;
                self.log_line(format!("{what}（已发布到引擎）"));
            }
            Err(error) => self.log_line(format!("{what} 发布失败：{error:?}")),
        }
    }

    /// 撤销 `steps` 步（GUI `Ctrl+Z` 与 Vi `u` 的共同落点）。
    pub fn undo(&mut self, steps: usize) {
        let action = if steps == 1 {
            UiAction::Undo
        } else {
            UiAction::UndoMany(steps)
        };
        self.apply_history_outcome("撤销", self.port.perform(action));
    }

    /// 重做 `steps` 步（GUI `Ctrl+Shift+Z` 与 Vi `Ctrl+R` 的共同落点）。
    pub fn redo(&mut self, steps: usize) {
        let action = if steps == 1 {
            UiAction::Redo
        } else {
            UiAction::RedoMany(steps)
        };
        self.apply_history_outcome("重做", self.port.perform(action));
    }

    fn apply_history_outcome(&mut self, what: &str, outcome: ActionOutcome) {
        match outcome {
            ActionOutcome::Changed { steps, .. } => {
                self.sync_from_port();
                self.publish_snapshot(&format!("{what} {steps} 步"));
            }
            ActionOutcome::Refused { detail } => {
                self.log_line(format!("{what}被拒绝：{detail}"));
            }
            ActionOutcome::DisplayOnly => {}
        }
    }

    /// 切换选中轨静音（GUI 裸 `M`）。
    pub fn toggle_mute(&mut self) {
        let Some(id) = self.selected_track_id() else {
            return;
        };
        let Some(track) = self.project.tracks.get(&id) else {
            return;
        };
        let (old_mute, name) = (track.mute, track.name.clone());
        let state = if old_mute { "取消静音" } else { "静音" };
        self.commit_edit(
            "tui: set track mute",
            vec![Op::SetTrackMute {
                track_id: id,
                old_mute,
                new_mute: !old_mute,
            }],
            &format!("{name} {state}"),
        );
    }

    /// 切换选中轨独奏（GUI 裸 `S`）。
    pub fn toggle_solo(&mut self) {
        let Some(id) = self.selected_track_id() else {
            return;
        };
        let Some(track) = self.project.tracks.get(&id) else {
            return;
        };
        let (old_solo, name) = (track.solo, track.name.clone());
        let state = if old_solo { "取消独奏" } else { "独奏" };
        self.commit_edit(
            "tui: set track solo",
            vec![Op::SetTrackSolo {
                track_id: id,
                old_solo,
                new_solo: !old_solo,
            }],
            &format!("{name} {state}"),
        );
    }

    /// 混音微调（GUI `Cmd/Ctrl+Alt+方向` 的落点）：音量 ±1.0 dB / 声相 ±0.1，
    /// 值域与 GUI 推子同一套夹紧（`FADER_MIN_DB..=FADER_MAX_DB` / `-1.0..=1.0`）。
    /// 提交走 `Op::SetParam`（与 GUI 混音台推子松手同一条路径）。
    pub fn nudge_mixer(&mut self, param: MixerParam, direction: i32) {
        let Some(id) = self.selected_track_id() else {
            return;
        };
        let Some(track) = self.project.tracks.get(&id) else {
            return;
        };
        let (target, old_val, new_val, what) = match param {
            MixerParam::Volume => {
                let next = (track.volume_db + direction as f32 * 1.0).clamp(
                    yeban_app::bridge::FADER_MIN_DB,
                    yeban_app::bridge::FADER_MAX_DB,
                );
                (
                    AutomationTarget::TrackVolume { track_id: id },
                    track.volume_db,
                    next,
                    format!("{} 音量 {next:+.1} dB", track.name),
                )
            }
            MixerParam::Pan => {
                let next = (track.pan + direction as f32 * 0.1).clamp(-1.0, 1.0);
                (
                    AutomationTarget::TrackPan { track_id: id },
                    track.pan,
                    next,
                    format!("{} 声相 {next:+.1}", track.name),
                )
            }
        };
        self.commit_edit(
            "tui: set mixer param",
            vec![Op::SetParam {
                target,
                old_val,
                new_val,
            }],
            &what,
        );
    }

    /// 光标处的片段摆放（选中轨上）：优先「包含光标 tick 的摆放」，其次
    /// 「起点 ≥ 光标 tick 的第一个摆放」；都没有返回 `None`。
    /// 返回 `(轨道, 摆放身份, 摆放快照)`。
    fn clip_at_cursor(&self) -> Option<(EntityId, EntityId, ClipPlacement)> {
        let track_id = self.selected_track_id()?;
        let track = self.project.tracks.get(&track_id)?;
        let tick = u64::try_from(self.cursor_bar).unwrap_or(0) * self.bar_ticks();
        let containing = track
            .clips
            .values()
            .find(|p| tick >= p.start_tick && tick < p.start_tick + p.duration_ticks);
        let placement = containing.or_else(|| {
            track
                .clips
                .values()
                .filter(|p| p.start_tick >= tick)
                .min_by_key(|p| p.start_tick)
        })?;
        Some((track_id, placement.id, *placement))
    }

    /// 平移光标处片段 `bars` 小节（负数向左；片段网格焦点下的 `H`/`L`）。
    /// 走 `Op::MoveClipPlacement`，可撤销；光标跟随片段新起点。
    pub fn move_clip_at_cursor(&mut self, bars: i32) {
        let Some((track_id, placement_id, placement)) = self.clip_at_cursor() else {
            self.log_line("光标处无片段可移动".to_owned());
            return;
        };
        let delta = i64::try_from(self.bar_ticks()).unwrap_or(i64::MAX) * i64::from(bars);
        let new_start = i64::try_from(placement.start_tick)
            .unwrap_or(i64::MAX)
            .saturating_add(delta)
            .max(0);
        let new_start = u64::try_from(new_start).unwrap_or(0);
        if new_start == placement.start_tick {
            self.log_line("片段已在最左，无变化".to_owned());
            return;
        }
        self.commit_edit(
            "tui: move clip placement",
            vec![Op::MoveClipPlacement {
                track_id,
                placement_id,
                old_start_tick: placement.start_tick,
                new_start_tick: new_start,
            }],
            "移动片段",
        );
        if self.project.tracks[&track_id]
            .clips
            .contains_key(&placement_id)
        {
            self.cursor_bar =
                usize::try_from(new_start / self.bar_ticks().max(1)).unwrap_or(usize::MAX);
        }
    }

    /// 删除光标处片段（片段网格焦点下 GUI `Delete`/`Backspace` 的落点）。
    /// 走 `Op::RemoveClipPlacement`（只删摆放，片段池条目不动），可撤销。
    pub fn delete_clip_at_cursor(&mut self) {
        let Some((track_id, placement_id, placement)) = self.clip_at_cursor() else {
            self.log_line("光标处无片段可删除".to_owned());
            return;
        };
        self.commit_edit(
            "tui: remove clip placement",
            vec![Op::RemoveClipPlacement {
                track_id,
                placement_id,
                previous_placement: placement,
            }],
            "删除片段",
        );
    }

    // ------------------------------------------------------------------
    // 三期：切分 / 复制粘贴 / 片段改名 / 音符编辑 / 池条目删除
    // （核心能力审计见 docs/phase3-core-audit.md；全部经 commit_edit 提交）
    // ------------------------------------------------------------------

    /// 在播放头处切分光标处片段（GUI `Ctrl+T` 的落点）。
    /// op 组合与 GUI 共用 `yeban_app::host::clip_split_ops_for`（同一构造点）：
    /// 左半缩短、跨点音符截断续接、右半是新池条目 + 新摆放，一步撤销。
    /// 切分点不严格落在片段内部时组合为空，如实报不可切（先 `:seek` 到片段内）。
    pub fn split_clip_at_playhead(&mut self) {
        let Some((_track_id, placement_id, _placement)) = self.clip_at_cursor() else {
            self.log_line("光标处无片段可切分".to_owned());
            return;
        };
        let playhead = self.transport().position_ticks;
        let Some((ops, _right_id)) =
            yeban_app::host::clip_split_ops_for(&self.project, &placement_id.to_string(), playhead)
        else {
            self.log_line("切分点不在片段内部：先 :seek 到片段内再切".to_owned());
            return;
        };
        self.commit_edit("tui: split clip at playhead", ops, "切分片段");
    }

    /// 复制光标处片段到进程内剪贴板（GUI `Ctrl+C` 的落点）。
    /// 剪贴板形状与 GUI 同：池条目 + 源摆放各一份快照；本动作不改工程。
    pub fn copy_clip_at_cursor(&mut self) {
        let Some((_track_id, _placement_id, placement)) = self.clip_at_cursor() else {
            self.log_line("光标处无片段可复制".to_owned());
            return;
        };
        let Some(entry) = self.project.clip_pool.get(&placement.clip_id) else {
            return;
        };
        let name = entry.name.clone();
        self.clipboard = Some((entry.clone(), placement));
        self.log_line(format!("已复制片段「{name}」"));
    }

    /// 把剪贴板粘贴到选中轨的光标处（GUI `Ctrl+V` 的落点）。
    /// 组合逐项照 GUI 的 `clipboard_paste`：片段与每个音符换新身份，
    /// `AddClip` + `AddClipPlacement` 一次提交、一步撤销。
    /// 形态差异（审计已登记）：落点用小节光标（GUI 用播放头；
    /// TUI 的 `:seek` 同步移动两者，光标即 TUI 的编辑点）；主总线不接收粘贴。
    pub fn paste_clipboard(&mut self) {
        let Some((entry, base)) = self.clipboard.clone() else {
            self.log_line("剪贴板为空：先 Ctrl+C 复制片段".to_owned());
            return;
        };
        let Some(track_id) = self.selected_track_id() else {
            return;
        };
        if track_id == self.project.master_bus_track_id {
            self.log_line("主总线不接收粘贴".to_owned());
            return;
        }
        if !self.project.tracks.contains_key(&track_id) {
            return;
        }
        let new_clip_id = EntityId::new();
        let content = match &entry.content {
            ClipContent::Midi { notes } => {
                let mut fresh = std::collections::BTreeMap::new();
                for note in notes.values() {
                    let mut copy = note.clone();
                    copy.id = EntityId::new();
                    fresh.insert(copy.id, copy);
                }
                ClipContent::Midi { notes: fresh }
            }
            ClipContent::Audio { asset, gain_db } => ClipContent::Audio {
                asset: asset.clone(),
                gain_db: *gain_db,
            },
        };
        let new_entry = ClipPoolEntry {
            id: new_clip_id,
            name: entry.name.clone(),
            content,
        };
        let start_tick = u64::try_from(self.cursor_bar).unwrap_or(0) * self.bar_ticks();
        let placement = ClipPlacement {
            id: EntityId::new(),
            clip_id: new_clip_id,
            start_tick,
            duration_ticks: base.duration_ticks,
            loop_config: base.loop_config,
            muted: false,
            offset_samples: base.offset_samples,
        };
        self.commit_edit(
            "tui: paste clip",
            vec![
                Op::AddClip { clip: new_entry },
                Op::AddClipPlacement {
                    track_id,
                    placement,
                },
            ],
            "粘贴片段",
        );
    }

    /// 剪切光标处片段（GUI `Ctrl+X` 的落点）= 复制 + 删摆放，**一次**提交：
    /// 与 GUI 的 `clipboard_cut` 同形，撤销一次同时回滚两者。
    /// 删摆放的 op 走 GUI 同一构造点 `clip_delete_ops_for`（只删摆放，
    /// 池条目留下——粘贴靠剪贴板快照，不靠池条目）。
    pub fn cut_clip_at_cursor(&mut self) {
        let Some((_track_id, placement_id, placement)) = self.clip_at_cursor() else {
            self.log_line("光标处无片段可剪切".to_owned());
            return;
        };
        let Some(entry) = self.project.clip_pool.get(&placement.clip_id) else {
            return;
        };
        self.clipboard = Some((entry.clone(), placement));
        let ops = yeban_app::host::clip_delete_ops_for(&self.project, &placement_id.to_string());
        if ops.is_empty() {
            return;
        }
        self.commit_edit("tui: cut clip", ops, "剪切片段");
    }

    /// 打开音符视图（对象 = 光标处片段；必须是 MIDI 片段）。
    /// 裸 `n` 与 `:notes` 的共同落点。
    pub fn open_notes_view(&mut self) {
        let Some((track_id, placement_id, placement)) = self.clip_at_cursor() else {
            self.log_line("光标处无片段：先把光标移到片段上".to_owned());
            return;
        };
        let Some(entry) = self.project.clip_pool.get(&placement.clip_id) else {
            return;
        };
        if !matches!(entry.content, ClipContent::Midi { .. }) {
            self.log_line("光标处片段不是 MIDI 片段，无音符可编辑".to_owned());
            return;
        }
        let name = entry.name.clone();
        self.list_view = Some(ListView::Notes {
            track_id,
            placement_id,
            clip_id: placement.clip_id,
            selected: 0,
        });
        self.log_line(format!("音符视图「{name}」：Esc 返回片段网格"));
    }

    /// 打开片段池视图（`:pool` 的落点）。
    pub fn open_pool_view(&mut self) {
        if self.project.clip_pool.is_empty() {
            self.log_line("片段池为空".to_owned());
            return;
        }
        self.list_view = Some(ListView::Pool { selected: 0 });
        self.log_line("片段池视图：Esc 返回片段网格".to_owned());
    }

    /// 关掉列表视图，回到普通片段网格（`Esc` 的视图内落点）。
    pub fn close_list_view(&mut self) {
        self.list_view = None;
    }

    /// 音符视图的对象片段的音符快照（按 (起点, 音高, 身份) 排序）。
    /// 视图外或对象不是 MIDI 片段时返回空。
    #[must_use]
    pub fn view_notes(&self) -> Vec<MidiNote> {
        let Some(ListView::Notes { clip_id, .. }) = self.list_view else {
            return Vec::new();
        };
        let Some(entry) = self.project.clip_pool.get(&clip_id) else {
            return Vec::new();
        };
        let ClipContent::Midi { notes } = &entry.content else {
            return Vec::new();
        };
        let mut list: Vec<MidiNote> = notes.values().cloned().collect();
        list.sort_by_key(|a| (a.start_tick, a.pitch, a.id));
        list
    }

    /// 片段池行快照（按 (名, 身份) 排序）：条目 + 引用它的摆放数。
    #[must_use]
    pub fn pool_rows(&self) -> Vec<(ClipPoolEntry, usize)> {
        let mut counts: std::collections::BTreeMap<EntityId, usize> =
            std::collections::BTreeMap::new();
        for track in self.project.tracks.values() {
            for placement in track.clips.values() {
                *counts.entry(placement.clip_id).or_insert(0) += 1;
            }
        }
        let mut rows: Vec<(ClipPoolEntry, usize)> = self
            .project
            .clip_pool
            .values()
            .map(|entry| (entry.clone(), counts.get(&entry.id).copied().unwrap_or(0)))
            .collect();
        rows.sort_by(|a, b| a.0.name.cmp(&b.0.name).then(a.0.id.cmp(&b.0.id)));
        rows
    }

    /// 列表视图内的选中下标移动（Vi `j/k` 与 `gg/G` 按视图分流到这里）。
    fn view_move_selection(&mut self, delta: i32) {
        let (len, selected) = match self.list_view {
            Some(ListView::Notes { .. }) => {
                let len = self.view_notes().len();
                let Some(ListView::Notes { selected, .. }) = self.list_view else {
                    return;
                };
                (len, selected)
            }
            Some(ListView::Pool { selected }) => (self.pool_rows().len(), selected),
            None => return,
        };
        let next = if len == 0 {
            0
        } else {
            usize::try_from((selected as i32 + delta).clamp(0, len as i32 - 1)).unwrap_or(0)
        };
        match &mut self.list_view {
            Some(ListView::Notes { selected, .. }) | Some(ListView::Pool { selected }) => {
                *selected = next;
            }
            None => {}
        }
    }

    /// 列表视图内跳到首/尾（Vi `gg`/`G` 按视图分流到这里）。
    fn view_select_edge(&mut self, last: bool) {
        let len = match self.list_view {
            Some(ListView::Notes { .. }) => self.view_notes().len(),
            Some(ListView::Pool { .. }) => self.pool_rows().len(),
            None => return,
        };
        let next = if last { len.saturating_sub(1) } else { 0 };
        match &mut self.list_view {
            Some(ListView::Notes { selected, .. }) | Some(ListView::Pool { selected }) => {
                *selected = next;
            }
            None => {}
        }
    }

    /// 当前选中的音符（音符视图内）：(轨道, 池条目, 音符快照)。
    fn selected_note(&self) -> Option<(EntityId, EntityId, MidiNote)> {
        let Some(ListView::Notes {
            track_id,
            clip_id,
            selected,
            ..
        }) = self.list_view
        else {
            return None;
        };
        self.view_notes()
            .get(selected)
            .map(|note| (track_id, clip_id, note.clone()))
    }

    /// 移动选中音符（`MoveNote` 增量；起点不为负、音高夹 0..=127，
    /// 夹紧后无变化就不提交）。方向键与 `h/l`、`J/K` 的共同落点，
    /// 换算走 GUI 同一函数 `nudge_delta_for` 的调用方在这里组装增量。
    pub fn move_selected_note(&mut self, delta_tick: i64, delta_pitch: i8) {
        let Some((track_id, clip_id, note)) = self.selected_note() else {
            return;
        };
        let delta_tick = delta_tick.max(-i64::try_from(note.start_tick).unwrap_or(i64::MAX));
        let new_pitch = (i16::from(note.pitch) + i16::from(delta_pitch)).clamp(0, 127);
        let delta_pitch = i8::try_from(new_pitch - i16::from(note.pitch)).unwrap_or(0);
        if delta_tick == 0 && delta_pitch == 0 {
            self.log_line("音符已在边界，无变化".to_owned());
            return;
        }
        self.commit_edit(
            "tui: move note",
            vec![Op::MoveNote {
                track_id,
                clip_id,
                note_id: note.id,
                delta_tick,
                delta_pitch,
            }],
            "移动音符",
        );
    }

    /// 在光标处加一个音符（音符视图内裸 `a`）：新身份，起点 = 光标在
    /// 片段内的相对 tick，音高 = 选中音符的音高（无选中时 60），
    /// 时值 = 一拍，力度 = 100（`MidiNote::new` 的缺省力度）。
    pub fn add_note_at_cursor(&mut self) {
        let Some(ListView::Notes {
            track_id,
            placement_id,
            clip_id,
            ..
        }) = self.list_view
        else {
            return;
        };
        let Some(track) = self.project.tracks.get(&track_id) else {
            return;
        };
        let Some(placement) = track.clips.get(&placement_id) else {
            self.log_line("视图的摆放已不存在：Esc 返回后重开".to_owned());
            return;
        };
        let cursor_tick = u64::try_from(self.cursor_bar).unwrap_or(0) * self.bar_ticks();
        let start_tick = cursor_tick.saturating_sub(placement.start_tick);
        let pitch = self.selected_note().map_or(60, |(_, _, note)| note.pitch);
        let note = MidiNote::new(EntityId::new(), start_tick, pitch, self.ticks_per_beat());
        let note_id = note.id;
        self.commit_edit(
            "tui: add note",
            vec![Op::AddNote {
                track_id,
                clip_id,
                note,
            }],
            "加音符",
        );
        // 选中新加的音符（按排序后的位置回填下标）。
        let position = self.view_notes().iter().position(|n| n.id == note_id);
        if let (Some(ListView::Notes { selected, .. }), Some(index)) =
            (&mut self.list_view, position)
        {
            *selected = index;
        }
    }

    /// 删除选中音符（音符视图内 `Delete` 的落点；`DeleteNote` 自带撤销载荷）。
    pub fn delete_selected_note(&mut self) {
        let Some((track_id, clip_id, note)) = self.selected_note() else {
            self.log_line("音符视图内无选中音符".to_owned());
            return;
        };
        self.commit_edit(
            "tui: delete note",
            vec![Op::DeleteNote {
                track_id,
                clip_id,
                note_id: note.id,
                previous_note: note,
            }],
            "删除音符",
        );
        self.clamp_view_selection();
    }

    /// 改选中音符的时值（音符视图内 `<`/`>`）：一步 = 一拍，
    /// 下限 = 一个吸附网格（240 tick，与 GUI 微移网格同值），走 `SetNoteTiming`。
    pub fn resize_selected_note(&mut self, beats: i32) {
        let Some((track_id, clip_id, note)) = self.selected_note() else {
            return;
        };
        let step = i64::try_from(self.ticks_per_beat()).unwrap_or(i64::MAX) * i64::from(beats);
        let new_duration = i64::try_from(note.duration_ticks)
            .unwrap_or(i64::MAX)
            .saturating_add(step)
            .max(i64::try_from(yeban_app::host::ROLL_SNAP_GRID_TICKS).unwrap_or(240));
        let new_duration = u64::try_from(new_duration).unwrap_or(note.duration_ticks);
        if new_duration == note.duration_ticks {
            self.log_line("时值已在下限，无变化".to_owned());
            return;
        }
        self.commit_edit(
            "tui: set note timing",
            vec![Op::SetNoteTiming {
                track_id,
                clip_id,
                note_id: note.id,
                old_start_tick: note.start_tick,
                old_duration_ticks: note.duration_ticks,
                new_start_tick: note.start_tick,
                new_duration_ticks: new_duration,
            }],
            "改音符时值",
        );
    }

    /// 改选中音符的力度（音符视图内 `-`/`+` 分流）：±10，夹 1..=127，
    /// 走 `ModifyNoteVelocity`。
    pub fn nudge_selected_note_velocity(&mut self, delta: i32) {
        let Some((track_id, clip_id, note)) = self.selected_note() else {
            return;
        };
        let new_vel = (i32::from(note.velocity) + delta).clamp(1, 127);
        if new_vel == i32::from(note.velocity) {
            self.log_line("力度已在边界，无变化".to_owned());
            return;
        }
        self.commit_edit(
            "tui: set note velocity",
            vec![Op::ModifyNoteVelocity {
                track_id,
                clip_id,
                note_id: note.id,
                old_vel: note.velocity,
                new_vel: u8::try_from(new_vel).unwrap_or(note.velocity),
            }],
            &format!("力度 {} → {new_vel}", note.velocity),
        );
    }

    /// 删除选中的片段池条目（池视图内 `Delete` 的落点；`Op::RemoveClip`）。
    /// 被摆放引用时模型以 `ClipInUse` 硬拒，拒绝原因经提交口如实记日志，
    /// ⛔ 不代删摆放。
    pub fn delete_selected_pool_entry(&mut self) {
        let Some(ListView::Pool { selected }) = self.list_view else {
            return;
        };
        let rows = self.pool_rows();
        let Some((entry, refs)) = rows.get(selected) else {
            return;
        };
        let (entry, refs) = (entry.clone(), *refs);
        let name = entry.name.clone();
        self.commit_edit(
            "tui: remove clip from pool",
            vec![Op::RemoveClip {
                clip_id: entry.id,
                previous_clip: entry,
            }],
            &format!("删除池条目「{name}」（{refs} 处引用）"),
        );
        self.clamp_view_selection();
    }

    /// 视图内删除后把选中下标夹回有效范围。
    fn clamp_view_selection(&mut self) {
        self.view_move_selection(0);
    }

    /// 定位播放头到第 `bar` 小节（0 起；走带状态，非文档编辑，不进撤销史）。
    pub fn seek_to_bar(&mut self, bar: usize) {
        let ticks = u64::try_from(bar).unwrap_or(0) * self.bar_ticks();
        self.host
            .send_transport(&[TransportCommand::SeekTicks(ticks)]);
        self.cursor_bar = bar;
        self.log_line(format!("定位到第 {} 小节", bar + 1));
    }

    /// 设工程速度（BPM）：走 `Op::SetTempo`，可撤销。
    pub fn set_tempo(&mut self, new_bpm: f64) {
        if !new_bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(&new_bpm) {
            self.log_line(format!(
                "速度越界：BPM 须为 {MIN_BPM:.0}..={MAX_BPM:.0} 之间的有限值"
            ));
            return;
        }
        let old_bpm = self.project.bpm;
        if new_bpm.to_bits() == old_bpm.to_bits() {
            self.log_line(format!("速度已是 {new_bpm:.1} BPM，无变化"));
            return;
        }
        self.commit_edit(
            "tui: set tempo",
            vec![Op::SetTempo { old_bpm, new_bpm }],
            &format!("速度 {old_bpm:.1} → {new_bpm:.1} BPM"),
        );
    }

    /// Vi 计数前缀的当前值（状态行显示用）。
    pub fn vi_count(&self) -> Option<usize> {
        self.vi.count()
    }

    /// 选中轨名（状态行显示用）。
    pub fn selected_track_name(&self) -> Option<&str> {
        let id = self.selected_track_id()?;
        self.project
            .tracks
            .get(&id)
            .map(|track| track.name.as_str())
    }

    /// 选轨移动（正数向后），带计数折算与夹紧。
    pub fn move_selection(&mut self, delta: i32) {
        if self.track_ids.is_empty() {
            return;
        }
        let last = self.track_ids.len() - 1;
        let next = (self.selected as i32 + delta).clamp(0, last as i32);
        self.selected = usize::try_from(next).unwrap_or(0);
    }

    // ------------------------------------------------------------------
    // 保存与导出
    // ------------------------------------------------------------------

    /// 保存到当前路径（无路径时报错提示用 `:w <path>`）。
    ///
    /// # Errors
    /// 无路径或写盘失败时返回错误文本。
    pub fn save(&mut self) -> Result<String, String> {
        let Some(path) = self.project_path.clone() else {
            return Err("尚无文件路径：用 :w <path> 另存".to_owned());
        };
        self.save_to(&path)
    }

    /// 保存到指定路径（有归档走全保真 `save_archive_file`，否则 `save_project_file`）。
    ///
    /// # Errors
    /// 写盘失败时返回错误文本。
    pub fn save_to(&mut self, path: &Path) -> Result<String, String> {
        if let Some(archive) = &mut self.archive {
            // 工程与提交图谱一次从端口读出（同一版），重开时历史可恢复；
            // 资产池不动（本线不新增资产，装载时的资产原样保留）。
            archive.project = self.port.project();
            archive.history_dag = yeban_model::commit::encode_history_dag(&self.port.graph());
            yeban_app::save::save_archive_file(archive, path)
                .map_err(|error| format!("保存失败：{error}"))?;
        } else {
            yeban_app::save::save_project_file(&self.project, path)
                .map_err(|error| format!("保存失败：{error}"))?;
        }
        self.project_path = Some(path.to_path_buf());
        if let Some(dir) = path.parent() {
            self.host.set_project_dir(Some(dir.to_path_buf()));
        }
        self.dirty = false;
        Ok(format!("已保存 {}", path.display()))
    }

    /// 离线导出 WAV（生产渲染路径：引擎乐器，非最小音源）。
    ///
    /// # Errors
    /// 渲染或写盘失败时返回错误文本。
    pub fn export_wav(&mut self, path: &Path) -> Result<ExportReceipt, String> {
        let sample_rate = sample_rate_hz(self.project.audio_config.sample_rate);
        let seconds = self.project_len_ticks() as f64 / self.ticks_per_beat().max(1) as f64 * 60.0
            / self.project.bpm.max(1.0);
        let frames = (seconds * f64::from(sample_rate)).ceil() as u64;
        let options = WavExportOptions {
            frames: frames.max(1),
            sample_rate,
            channels: 2,
            ..Default::default()
        };
        let assets: Vec<(yeban_model::ids::AssetHash, Vec<u8>)> = self
            .archive
            .as_ref()
            .map(|archive| archive.assets.clone())
            .unwrap_or_default();
        let project_dir = self
            .project_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let output = yeban_app::render_core::render_project_f32_with_assets_under(
            &self.project,
            options,
            None,
            &assets,
            project_dir.as_deref(),
        )
        .map_err(|error| format!("渲染失败：{error}"))?;
        let (bytes, bits) = yeban_app::export_wav::write_interleaved_wav(
            path,
            &output.samples,
            2,
            sample_rate,
            self.project.rng_seed,
            options.bit_depth,
        )
        .map_err(|error| format!("写 WAV 失败：{error}"))?;
        let receipt = ExportReceipt {
            bytes,
            frames: output.frames,
            sample_rate,
            bits,
        };
        self.last_export = Some(receipt);
        self.log_line(format!(
            "已导出 {}（{} 字节，{:.2} 秒）",
            path.display(),
            receipt.bytes,
            receipt.frames as f64 / f64::from(receipt.sample_rate)
        ));
        Ok(receipt)
    }
}

/// 工程采样率枚举 → Hz 数值。
#[must_use]
pub fn sample_rate_hz(rate: yeban_model::project::SampleRate) -> u32 {
    use yeban_model::project::SampleRate;
    match rate {
        SampleRate::Hz44100 => 44_100,
        SampleRate::Hz48000 => 48_000,
        SampleRate::Hz88200 => 88_200,
        SampleRate::Hz96000 => 96_000,
        SampleRate::Hz192000 => 192_000,
    }
}

/// 轨道类型 → 显示名。
#[must_use]
pub fn track_kind_label(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Midi => "MIDI",
        TrackKind::Audio => "音频",
        TrackKind::AuxReturn => "辅助",
        TrackKind::Master => "主总线",
    }
}

// ----------------------------------------------------------------------
// 按键分发
// ----------------------------------------------------------------------

impl App {
    /// 处理一次按键（主循环与测试共用入口）。
    pub fn handle_key(&mut self, event: KeyEvent) {
        if event.kind == KeyEventKind::Release {
            return;
        }
        match self.mode {
            Mode::Normal => self.handle_normal_key(event),
            Mode::Insert => self.handle_insert_key(event),
            Mode::Command => self.handle_command_key(event),
        }
    }

    fn handle_normal_key(&mut self, event: KeyEvent) {
        // Esc：列表视图开着时先关视图（三期），否则清计数与半成品序列
        // （GUI 解析对 Esc 是 PassThrough，落到这里）。
        if event.code == KeyCode::Esc {
            if self.list_view.is_some() {
                self.close_list_view();
                self.log_line("返回片段网格".to_owned());
            }
            self.vi.reset();
            return;
        }
        let modified = event.modifiers.contains(KeyModifiers::CONTROL)
            || event.modifiers.contains(KeyModifiers::ALT);
        // 冲突 K1 的解：无修饰数字归 Vi 计数前缀，先于 GUI 解析
        // （GUI 的裸数字是选工具，TUI v1 无工具系统，无落点）。
        if !modified
            && let KeyCode::Char(ch @ '0'..='9') = event.code
            && self.vi.feed_char(ch) == ViOutcome::Pending
        {
            return;
        }
        // 第一段：GUI 共享键 —— 与 GUI 同一张策略表、同一个解析器。
        if let Some((logical, modifiers)) = to_logical(event) {
            match self.input_ctx.resolve_logical(logical, modifiers) {
                Resolution::Action(action) => {
                    self.dispatch_gui_action(action);
                    return;
                }
                Resolution::ConsumedByIme => return,
                Resolution::PassThrough => {}
            }
        }
        // TUI 自加键（K3 改判）：`Ctrl+R` = 重做（Vi 习惯；GUI 表无此格，
        // 解析已 PassThrough 到这里）。crossterm 对 Ctrl+R 给 Char('r')+CONTROL。
        if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('r') {
            self.redo(1);
            return;
        }
        // 第二段：Vi 导航层（只吃无修饰/仅 Shift 的字符键）。
        if !modified && let KeyCode::Char(ch) = event.code {
            // 音符视图内的编辑键（三期，TUI 自加；这些键在共享表里是
            // PassThrough 或未占，视图外不拦截、行为不变）：
            // `J`/`K` 移音高 ±1 半音、`a` 加音符、`<`/`>` 改时值 ∓/± 一拍。
            // 时间移动与选择走 Vi 动作层（h/l、j/k 按视图分流，见 apply_motion）
            // 与方向键（NudgeSelection 分流，见 dispatch_gui_action）。
            if matches!(self.list_view, Some(ListView::Notes { .. })) {
                match ch {
                    'J' => {
                        self.vi.reset();
                        self.move_selected_note(0, 1);
                        return;
                    }
                    'K' => {
                        self.vi.reset();
                        self.move_selected_note(0, -1);
                        return;
                    }
                    'a' => {
                        self.vi.reset();
                        self.add_note_at_cursor();
                        return;
                    }
                    '<' => {
                        self.vi.reset();
                        self.resize_selected_note(-1);
                        return;
                    }
                    '>' => {
                        self.vi.reset();
                        self.resize_selected_note(1);
                        return;
                    }
                    _ => {}
                }
            }
            // 片段网格焦点下开音符视图（TUI 自加键）：裸 `n`（共享表
            // PassThrough、Vi 层未占）。视图已开时不重复开。
            if ch == 'n' && self.focus == Pane::Grid && self.list_view.is_none() {
                self.vi.reset();
                self.open_notes_view();
                return;
            }
            // 片段网格焦点下的片段平移键（TUI 自加）：`H`/`L` 按小节平移
            // 光标处片段（计数前缀 = 小节数）。删除不用自加键 —— 走 GUI 的
            // `Delete`/`Backspace`（画布语义 = `DeleteSelection`，见分发层）。
            // 轨道/检查器焦点下 `H`/`L` 不绑定（落 Vi 层得 NotVi）；
            // 列表视图开着时也不绑定（视图内 h/l 是音符级移动，粒度不同）。
            if self.focus == Pane::Grid && self.list_view.is_none() && matches!(ch, 'H' | 'L') {
                let count = i32::try_from(self.vi.count().unwrap_or(1)).unwrap_or(i32::MAX);
                self.vi.reset();
                if ch == 'H' {
                    self.move_clip_at_cursor(-count);
                } else {
                    self.move_clip_at_cursor(count);
                }
                return;
            }
            match self.vi.feed_char(ch) {
                ViOutcome::Pending => {}
                ViOutcome::Motion(motion, count) => self.apply_motion(motion, count),
                ViOutcome::Undo(steps) => self.undo(steps),
                ViOutcome::EnterCommand => {
                    self.mode = Mode::Command;
                    self.cmdline.clear();
                }
                ViOutcome::EnterInsert => self.begin_rename(),
                ViOutcome::NotVi => {}
            }
        }
    }

    fn apply_motion(&mut self, motion: ViMotion, count: usize) {
        let count = i32::try_from(count).unwrap_or(i32::MAX);
        // 三期：列表视图开着时，Vi 动作按视图分流——j/k/gg/G 动视图内的
        // 选中下标；音符视图里 h/l 动选中音符的时间（一格 = 吸附网格
        // 240 tick，与 GUI 微移的 Grid 档同值）；池视图里 h/l 无动作。
        if self.list_view.is_some() {
            match motion {
                ViMotion::TrackDown => self.view_move_selection(count),
                ViMotion::TrackUp => self.view_move_selection(-count),
                ViMotion::BarRight | ViMotion::BarLeft => {
                    if matches!(self.list_view, Some(ListView::Notes { .. })) {
                        let grid =
                            i64::try_from(yeban_app::host::ROLL_SNAP_GRID_TICKS).unwrap_or(240);
                        let delta = grid * i64::from(count);
                        if motion == ViMotion::BarRight {
                            self.move_selected_note(delta, 0);
                        } else {
                            self.move_selected_note(-delta, 0);
                        }
                    }
                }
                ViMotion::First => self.view_select_edge(false),
                ViMotion::Last => self.view_select_edge(true),
            }
            return;
        }
        match motion {
            ViMotion::TrackDown => self.move_selection(count),
            ViMotion::TrackUp => self.move_selection(-count),
            ViMotion::BarRight => {
                let last = self.total_bars().saturating_sub(1);
                self.cursor_bar = self.cursor_bar.saturating_add(count as usize).min(last);
            }
            ViMotion::BarLeft => {
                self.cursor_bar = self.cursor_bar.saturating_sub(count as usize);
            }
            ViMotion::First => match self.focus {
                Pane::Grid => self.cursor_bar = 0,
                _ => self.selected = 0,
            },
            ViMotion::Last => match self.focus {
                Pane::Grid => self.cursor_bar = self.total_bars().saturating_sub(1),
                _ => self.selected = self.track_ids.len().saturating_sub(1),
            },
        }
    }

    /// GUI 共享动作的分发：v1 接住的子集真实执行，其余如实记日志（不假装）。
    fn dispatch_gui_action(&mut self, action: Action) {
        match action {
            Action::PlayPause => self.toggle_play(),
            Action::ResumeFromCursor => {
                // GUI 是「从光标处续播」；TUI 的小节光标即其对应物：定位后播放。
                let ticks = u64::try_from(self.cursor_bar).unwrap_or(0) * self.bar_ticks();
                self.host
                    .send_transport(&[TransportCommand::SeekTicks(ticks), TransportCommand::Play]);
                self.log_line("从光标处续播".to_owned());
            }
            Action::GoToBeginning => self.go_to_beginning(),
            Action::SaveProject => match self.save() {
                Ok(line) => self.log_line(line),
                Err(error) => self.log_line(error),
            },
            Action::SaveProjectAs => {
                self.log_line("另存为：用 :w <path>".to_owned());
            }
            Action::OpenProject => {
                self.log_line("打开工程：用 :open <path>".to_owned());
            }
            Action::ToggleTrackMute => self.toggle_mute(),
            Action::ToggleTrackSolo => self.toggle_solo(),
            Action::NudgeMixerChannel { param, direction } => {
                self.nudge_mixer(param, i32::from(direction));
            }
            Action::SelectTrackBy(delta) => self.move_selection(i32::from(delta)),
            Action::ToggleView => {
                self.focus = self.focus.next();
                self.log_line(format!("焦点 → {}", self.focus.label()));
            }
            Action::ToggleMixer => {
                self.focus = Pane::Inspector;
                self.log_line("焦点 → 检查器（TUI 对 GUI 混音台的形态适配）".to_owned());
            }
            Action::ToggleEditor => {
                self.focus = Pane::Grid;
                self.log_line("焦点 → 片段（TUI 对 GUI 编辑器的形态适配）".to_owned());
            }
            Action::ZoomIn => {
                // 三期分流：音符视图内裸 `+` 是力度 +10（与 `-` 成对，
                // 同 DeleteSelection 的按视图分流手法）；视图外照旧缩放。
                if matches!(self.list_view, Some(ListView::Notes { .. })) {
                    self.nudge_selected_note_velocity(10);
                } else {
                    self.bar_cols = (self.bar_cols + 1).min(8);
                }
            }
            Action::ZoomOut => {
                if matches!(self.list_view, Some(ListView::Notes { .. })) {
                    self.nudge_selected_note_velocity(-10);
                } else {
                    self.bar_cols = self.bar_cols.saturating_sub(1).max(2);
                }
            }
            Action::SplitAtPlayhead => {
                // GUI `Ctrl+T`（三期）：片段网格焦点下切分光标处片段。
                if self.focus == Pane::Grid {
                    self.split_clip_at_playhead();
                } else {
                    self.log_line("切分只在片段网格焦点下接入（Tab 切焦点）".to_owned());
                }
            }
            Action::CopyClipboard => {
                // GUI `Ctrl+C`（三期）：片段网格焦点下复制光标处片段。
                if self.focus == Pane::Grid {
                    self.copy_clip_at_cursor();
                } else {
                    self.log_line("复制片段只在片段网格焦点下接入（Tab 切焦点）".to_owned());
                }
            }
            Action::PasteClipboard => {
                // GUI `Ctrl+V`（三期）：粘贴到选中轨的光标处。
                if self.focus == Pane::Grid {
                    self.paste_clipboard();
                } else {
                    self.log_line("粘贴片段只在片段网格焦点下接入（Tab 切焦点）".to_owned());
                }
            }
            Action::CutClipboard => {
                // GUI `Ctrl+X`（三期）：剪切光标处片段（复制 + 删摆放，一步撤销）。
                if self.focus == Pane::Grid {
                    self.cut_clip_at_cursor();
                } else {
                    self.log_line("剪切片段只在片段网格焦点下接入（Tab 切焦点）".to_owned());
                }
            }
            Action::NudgeSelection { step, direction } => {
                // GUI 方向键（`[UI-NOTE-005]`，三期）：只在音符视图内接落点
                // ——移动选中音符，换算走 GUI 同一函数 `nudge_delta_for`
                // （Grid 240 tick / Tick 1 / Semitone ±1 / Octave ±12）。
                // 视图外 TUI 无音符选区，保持未接、如实记日志。
                if matches!(self.list_view, Some(ListView::Notes { .. })) {
                    let (delta_tick, delta_pitch) =
                        yeban_app::host::nudge_delta_for(step, direction);
                    self.move_selected_note(delta_tick, delta_pitch);
                } else {
                    self.log_line("微移音符只在音符视图内接入（片段网格里按 n 打开）".to_owned());
                }
            }
            Action::Undo => self.undo(1),
            Action::Redo => self.redo(1),
            Action::DeleteSelection => {
                // GUI 画布的 `Delete`/`Backspace`：删「当前选中的对象」，
                // 对象随视图定（三期）——音符视图删选中音符、池视图删选中
                // 池条目、普通网格删光标处片段摆放；都经撤销端口可撤销。
                // 其它焦点下 TUI 尚无选区删除，如实报未接，不假装。
                match self.list_view {
                    Some(ListView::Notes { .. }) => self.delete_selected_note(),
                    Some(ListView::Pool { .. }) => self.delete_selected_pool_entry(),
                    None => {
                        if self.focus == Pane::Grid {
                            self.delete_clip_at_cursor();
                        } else {
                            self.log_line(
                                "删除选区只在片段网格焦点下接入（Tab 切焦点）".to_owned(),
                            );
                        }
                    }
                }
            }
            other => {
                self.log_line(format!("GUI 动作 {other:?} 未在 TUI v1 实现"));
            }
        }
    }

    // ------------------------------------------------------------------
    // Insert（改名）
    // ------------------------------------------------------------------

    fn begin_rename(&mut self) {
        if self.focus == Pane::Grid {
            // 三期：片段网格焦点下改的是光标处片段的池条目名
            // （`Op::SetClipName`；二期时此处只有轨道改名、片段改名未接）。
            let Some((_track_id, _placement_id, placement)) = self.clip_at_cursor() else {
                self.log_line("光标处无片段可改名".to_owned());
                return;
            };
            let Some(entry) = self.project.clip_pool.get(&placement.clip_id) else {
                return;
            };
            self.rename = Some(entry.name.clone());
            self.rename_target = Some(RenameTarget::Clip(placement.clip_id));
            self.mode = Mode::Insert;
            return;
        }
        let Some(id) = self.selected_track_id() else {
            return;
        };
        if let Some(track) = self.project.tracks.get(&id) {
            self.rename = Some(track.name.clone());
            self.rename_target = Some(RenameTarget::Track(id));
            self.mode = Mode::Insert;
        }
    }

    fn handle_insert_key(&mut self, event: KeyEvent) {
        match event.code {
            KeyCode::Esc => {
                self.rename = None;
                self.rename_target = None;
                self.mode = Mode::Normal;
                self.log_line("改名取消".to_owned());
            }
            KeyCode::Enter => {
                let new_name = self.rename.take().unwrap_or_default();
                let target = self.rename_target.take();
                self.mode = Mode::Normal;
                let trimmed = new_name.trim().to_owned();
                if trimmed.is_empty() {
                    self.log_line("改名取消：名字为空".to_owned());
                    return;
                }
                match target {
                    Some(RenameTarget::Track(id)) => {
                        if let Some(track) = self.project.tracks.get(&id) {
                            let old_name = track.name.clone();
                            if old_name != trimmed {
                                self.commit_edit(
                                    "tui: set track name",
                                    vec![Op::SetTrackName {
                                        track_id: id,
                                        old_name,
                                        new_name: trimmed.clone(),
                                    }],
                                    &format!("改名为「{trimmed}」"),
                                );
                            }
                        }
                    }
                    Some(RenameTarget::Clip(clip_id)) => {
                        // 旧名从当前文档读（模型的 OpStateMismatch 前置条件
                        // 逐值比较，⛔ 不采信打开视图时的旧快照）。
                        if let Some(entry) = self.project.clip_pool.get(&clip_id) {
                            let old_name = entry.name.clone();
                            if old_name != trimmed {
                                self.commit_edit(
                                    "tui: set clip name",
                                    vec![Op::SetClipName {
                                        clip_id,
                                        old_name,
                                        new_name: trimmed.clone(),
                                    }],
                                    &format!("片段改名为「{trimmed}」"),
                                );
                            }
                        }
                    }
                    None => {}
                }
            }
            KeyCode::Backspace => {
                if let Some(buffer) = &mut self.rename {
                    buffer.pop();
                }
            }
            KeyCode::Char(ch) => {
                if event.modifiers.contains(KeyModifiers::CONTROL)
                    || event.modifiers.contains(KeyModifiers::ALT)
                {
                    return;
                }
                if let Some(buffer) = &mut self.rename {
                    buffer.push(ch);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Command（`:` 命令行）
    // ------------------------------------------------------------------

    fn handle_command_key(&mut self, event: KeyEvent) {
        match event.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.cmdline.clear();
            }
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.cmdline);
                self.mode = Mode::Normal;
                self.execute_command(&line);
            }
            KeyCode::Backspace => {
                self.cmdline.pop();
            }
            KeyCode::Char(ch) => {
                if event.modifiers.contains(KeyModifiers::CONTROL)
                    || event.modifiers.contains(KeyModifiers::ALT)
                {
                    return;
                }
                self.cmdline.push(ch);
            }
            _ => {}
        }
    }

    /// 执行一行 `:` 命令。
    pub fn execute_command(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let verb = parts.next().unwrap_or("");
        let rest = parts.next().map(str::trim).unwrap_or("");
        match verb {
            "q" => {
                if self.dirty {
                    self.log_line("有未保存修改：先 :w 保存，或 :q! 强退".to_owned());
                } else {
                    self.quit = true;
                }
            }
            "q!" => self.quit = true,
            "w" => {
                let result = if rest.is_empty() {
                    self.save()
                } else {
                    self.save_to(Path::new(rest))
                };
                match result {
                    Ok(done) => self.log_line(done),
                    Err(error) => self.log_line(error),
                }
            }
            "wq" => {
                let result = if rest.is_empty() {
                    self.save()
                } else {
                    self.save_to(Path::new(rest))
                };
                match result {
                    Ok(done) => {
                        self.log_line(done);
                        self.quit = true;
                    }
                    Err(error) => self.log_line(error),
                }
            }
            "play" => {
                self.host.play();
                self.log_line("播放".to_owned());
            }
            "seek" => {
                // :seek <bar> —— 播放头定位到第 bar 小节（0 起，与小节光标同读数）。
                match rest.parse::<usize>() {
                    Ok(bar) => self.seek_to_bar(bar),
                    Err(_) => self.log_line("用法：:seek <小节号（0 起）>".to_owned()),
                }
            }
            "tempo" => {
                // :tempo <bpm> —— 设工程速度（经撤销端口提交，可撤销）。
                match rest.parse::<f64>() {
                    Ok(bpm) if bpm.is_finite() && (MIN_BPM..=MAX_BPM).contains(&bpm) => {
                        self.set_tempo(bpm);
                    }
                    Ok(_) => self.log_line(format!(
                        "速度越界：BPM 须为 {MIN_BPM:.0}..={MAX_BPM:.0} 之间的有限值"
                    )),
                    Err(_) => {
                        self.log_line(format!("用法：:tempo <BPM>（{MIN_BPM:.0}..={MAX_BPM:.0}）"))
                    }
                }
            }
            "stop" => self.stop(),
            "rewind" => self.go_to_beginning(),
            "notes" => {
                // :notes —— 打开光标处片段的音符视图（与裸 `n` 同一落点）。
                self.open_notes_view();
            }
            "pool" => {
                // :pool —— 打开片段池视图（删条目在视图内 Delete）。
                self.open_pool_view();
            }
            "export" => {
                // 形态：export wav <path> 或 export <path>（默认 wav）。
                let target = rest.strip_prefix("wav ").map(str::trim).unwrap_or(rest);
                if target.is_empty() {
                    self.log_line("用法：:export wav <path>".to_owned());
                } else if let Err(error) = self.export_wav(Path::new(target)) {
                    self.log_line(error);
                }
            }
            "open" | "open!" => {
                let force = verb == "open!";
                if rest.is_empty() {
                    self.log_line("用法：:open <path>".to_owned());
                } else if self.dirty && !force {
                    self.log_line(format!("有未保存修改：先 :w 保存，或 :open! {rest} 强开"));
                } else {
                    match Self::open_file(Path::new(rest)) {
                        Ok(next) => {
                            *self = next;
                        }
                        Err(error) => self.log_line(error),
                    }
                }
            }
            "demo" | "demo!" => {
                let force = verb == "demo!";
                if self.dirty && !force {
                    self.log_line("有未保存修改：先 :w 保存，或 :demo! 强开演示工程".to_owned());
                } else {
                    match Self::with_demo() {
                        Ok(next) => {
                            *self = next;
                        }
                        Err(error) => self.log_line(error),
                    }
                }
            }
            other => self.log_line(format!("未知命令：{other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_calls_heartbeat_and_reclaims_retired_snapshots() {
        let mut app = App::with_demo().expect("demo app");
        assert!(!app.is_playing());
        const EDITS: u64 = 10;
        for _ in 0..EDITS {
            app.toggle_mute();
            app.host.drive_audio(1);
        }
        let before = app.host.snapshot_counts().expect("snapshot counts");
        assert_eq!(before.pending_len, usize::try_from(EDITS).unwrap());
        assert_eq!(before.retire_pending, usize::try_from(EDITS).unwrap());
        assert_eq!(before.pruned, 0);
        assert_eq!(before.retire_dropped, 0);

        app.tick();

        let after = app.host.snapshot_counts().expect("snapshot counts");
        assert_eq!(after.pending_len, 0);
        assert_eq!(after.retire_pending, 0);
        assert_eq!(after.pruned, EDITS);
        assert_eq!(after.retire_dropped, EDITS);
    }

    #[test]
    fn open_dirty_guard_blocks_and_force_opens() {
        struct TempCleanup(PathBuf);
        impl Drop for TempCleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }

        let saved_path =
            std::env::temp_dir().join(format!("yeban-tui-open-dirty-{}.yeban", std::process::id()));
        let _cleanup = TempCleanup(saved_path.clone());

        let mut writer = App::with_demo().expect("demo app");
        writer.save_to(&saved_path).expect("save demo");

        let mut app = App::with_demo().expect("demo app");
        assert!(!app.dirty);
        app.execute_command(&format!("open {}", saved_path.display()));
        assert!(!app.dirty);
        assert_eq!(app.project_path.as_deref(), Some(saved_path.as_path()));

        let track_id = app.track_ids[0];
        assert!(!app.project.tracks[&track_id].mute);
        app.toggle_mute();
        assert!(app.dirty);
        assert!(app.project.tracks[&track_id].mute);

        // Empty argument reports usage and keeps dirty edits.
        app.execute_command("open");
        assert!(app.dirty);
        assert!(app.project.tracks[&track_id].mute);
        assert!(
            app.log
                .last()
                .is_some_and(|line| line.contains("用法：:open"))
        );

        // Unforced :open is blocked while dirty.
        app.execute_command(&format!("open {}", saved_path.display()));
        assert!(app.dirty);
        assert!(app.project.tracks[&track_id].mute);
        assert!(
            app.log
                .last()
                .is_some_and(|line| line.contains("有未保存修改") && line.contains(":open!"))
        );

        // Failed :open! on missing path preserves unsaved edits.
        let missing_path =
            std::env::temp_dir().join(format!("yeban-tui-missing-{}.yeban", std::process::id()));
        app.execute_command(&format!("open! {}", missing_path.display()));
        assert!(app.dirty);
        assert!(app.project.tracks[&track_id].mute);

        // Forced :open! discards dirty edits and loads saved file.
        app.execute_command(&format!("open! {}", saved_path.display()));
        assert!(!app.dirty);
        assert!(!app.project.tracks[&track_id].mute);
        assert_eq!(app.project_path.as_deref(), Some(saved_path.as_path()));

        // Clean :open works without ! after :w clears dirty.
        app.toggle_mute();
        assert!(app.dirty);
        app.execute_command("w");
        assert!(!app.dirty);
        app.execute_command(&format!("open {}", saved_path.display()));
        assert!(!app.dirty);
    }

    #[test]
    fn demo_dirty_guard_blocks_and_force_resets() {
        let mut app = App::with_demo().expect("demo app");
        let default_bpm = app.project.bpm;
        assert!(!app.dirty);

        // Clean state allows :demo without !.
        app.execute_command("demo");
        assert!(!app.dirty);

        // Mutate state and verify :demo is blocked while dirty.
        app.set_tempo(140.0);
        assert!(app.dirty);
        assert_eq!(app.project.bpm.to_bits(), 140.0_f64.to_bits());

        app.execute_command("demo");
        assert!(app.dirty);
        assert_eq!(app.project.bpm.to_bits(), 140.0_f64.to_bits());
        assert!(
            app.log
                .last()
                .is_some_and(|line| line.contains("有未保存修改") && line.contains(":demo!"))
        );

        // Forced :demo! resets back to demo default.
        app.execute_command("demo!");
        assert!(!app.dirty);
        assert_eq!(app.project.bpm.to_bits(), default_bpm.to_bits());
    }

    #[test]
    fn tempo_rejects_non_finite_and_out_of_bounds_via_command_and_method() {
        let mut app = App::with_demo().expect("demo app");
        let initial_bpm = app.project.bpm;

        for cmd in [
            "tempo NaN",
            "tempo inf",
            "tempo +inf",
            "tempo -inf",
            "tempo 0",
            "tempo -10",
            "tempo 19.9",
            "tempo 999.1",
            "tempo 1000",
        ] {
            app.execute_command(cmd);
            assert_eq!(
                app.project.bpm.to_bits(),
                initial_bpm.to_bits(),
                "{cmd} should not change bpm"
            );
            assert!(!app.dirty, "{cmd} should not mark dirty");
            assert!(
                app.log.last().is_some_and(|line| line.contains("速度越界")),
                "{cmd} should log range error, got {:?}",
                app.log.last()
            );
        }

        for cmd in ["tempo abc", "tempo"] {
            app.execute_command(cmd);
            assert_eq!(app.project.bpm.to_bits(), initial_bpm.to_bits());
            assert!(!app.dirty);
            assert!(
                app.log.last().is_some_and(|line| line.contains("用法")),
                "{cmd} should log usage, got {:?}",
                app.log.last()
            );
        }

        for bad in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
            -5.0,
            MIN_BPM - 0.1,
            MAX_BPM + 0.1,
        ] {
            app.set_tempo(bad);
            assert_eq!(app.project.bpm.to_bits(), initial_bpm.to_bits());
            assert!(!app.dirty);
            assert!(app.log.last().is_some_and(|line| line.contains("速度越界")));
        }
    }

    #[test]
    fn tempo_accepts_inclusive_bounds_and_dedups_same_value() {
        let mut app = App::with_demo().expect("demo app");

        app.execute_command("tempo 20");
        assert_eq!(app.project.bpm.to_bits(), MIN_BPM.to_bits());
        assert!(app.dirty);

        app.execute_command("tempo 999");
        assert_eq!(app.project.bpm.to_bits(), MAX_BPM.to_bits());

        app.execute_command("tempo 128.5");
        assert!((app.project.bpm - 128.5).abs() < 1e-9);

        app.undo(1);
        assert_eq!(app.project.bpm.to_bits(), MAX_BPM.to_bits());

        // Same-value tempo is a no-op and does not push a duplicate undo entry.
        app.execute_command("tempo 999");
        assert!(app.log.last().is_some_and(|line| line.contains("无变化")));
        app.undo(1);
        assert_eq!(app.project.bpm.to_bits(), MIN_BPM.to_bits());
    }
}
