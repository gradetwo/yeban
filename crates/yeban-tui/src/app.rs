//! TUI 应用状态与动作分发。
//!
//! 接缝（路线甲）：工程读写走 `yeban_app::open` / `yeban_app::save` 的生产路径；
//! 引擎与走带走 `yeban_app::engine_host::EngineHost`（与 GUI 同一个宿主，无 Slint
//! 依赖）；导出走 `yeban_app::render_core` + `yeban_app::export_wav` 的生产渲染路径；
//! 共享键位走 `yeban_app::input::InputContext::resolve`（GUI 策略表的唯一解析器）。
//!
//! v1 的编辑（静音/独奏/音量/声相/改名）是**直接改工程文档再 `publish_project`**
//! 发布新快照：不经 GUI 的撤销端口 ⇒ 撤销/重做在 v1 未接（键位照 GUI，按下如实
//! 报错），这是已登记的缺口，不是静默假装。

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use yeban_app::engine_host::{EditMark, EngineHost};
use yeban_app::export_wav::WavExportOptions;
use yeban_app::input::{Action, InputContext, MixerParam, Resolution};
use yeban_engine::meter::{MeterCollector, MeterFrame};
use yeban_engine::ring::TransportCommand;
use yeban_engine::transport::{TransportReading, TransportState};
use yeban_model::container::ProjectArchive;
use yeban_model::ids::EntityId;
use yeban_model::project::{TrackKind, YebanProjectV1};

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

/// 应用状态。
pub struct App {
    /// 当前工程文档（唯一可变权威，与 GUI 同一条口径）。
    pub project: YebanProjectV1,
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
    /// 日志行（最新在尾；底栏显示最后一条）。
    pub log: Vec<String>,
    /// 有未保存修改。
    pub dirty: bool,
    /// 文档代次计数（充当 `EditMark` 的单调标记；v1 无撤销栈，见模块文档）。
    edit_seq: usize,
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
        };
        Self::assemble(project, Some(archive), None)
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
        let mut host = EngineHost::new();
        if let Some(dir) = project_path.as_deref().and_then(Path::parent) {
            host.set_project_dir(Some(dir.to_path_buf()));
        }
        let rebuild = host
            .reload(&project, 1)
            .map_err(|error| format!("引擎图构建失败：{error:?}"))?;
        // 与 GUI 同一条纪律：加载即停住（reload 本身不碰走带状态）。
        host.stop();
        let mut app = Self {
            project,
            archive,
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
            log: Vec::new(),
            dirty: false,
            edit_seq: 0,
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

    /// 墙钟泵送 + 电平抽干。播放时位置按真实时间推进（控制面模式，无声）。
    pub fn tick(&mut self) {
        self.host.pump_wall_clock();
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

    fn publish_edit(&mut self, what: &str) {
        self.edit_seq = self.edit_seq.saturating_add(1);
        let mark = EditMark::new(None, self.edit_seq);
        match self.host.publish_project(&self.project, mark) {
            Ok(_) => {
                self.dirty = true;
                self.log_line(format!("{what}（已发布到引擎）"));
            }
            Err(error) => self.log_line(format!("{what} 发布失败：{error:?}")),
        }
    }

    /// 切换选中轨静音（GUI 裸 `M`）。
    pub fn toggle_mute(&mut self) {
        let Some(id) = self.selected_track_id() else {
            return;
        };
        if let Some(track) = self.project.tracks.get_mut(&id) {
            track.mute = !track.mute;
            let state = if track.mute { "静音" } else { "取消静音" };
            let name = track.name.clone();
            self.publish_edit(&format!("{name} {state}"));
        }
    }

    /// 切换选中轨独奏（GUI 裸 `S`）。
    pub fn toggle_solo(&mut self) {
        let Some(id) = self.selected_track_id() else {
            return;
        };
        if let Some(track) = self.project.tracks.get_mut(&id) {
            track.solo = !track.solo;
            let state = if track.solo { "独奏" } else { "取消独奏" };
            let name = track.name.clone();
            self.publish_edit(&format!("{name} {state}"));
        }
    }

    /// 混音微调（GUI `Cmd/Ctrl+Alt+方向` 的落点）：音量 ±1.0 dB / 声相 ±0.1，
    /// 值域与 GUI 推子同一套夹紧（`FADER_MIN_DB..=FADER_MAX_DB` / `-1.0..=1.0`）。
    pub fn nudge_mixer(&mut self, param: MixerParam, direction: i32) {
        let Some(id) = self.selected_track_id() else {
            return;
        };
        if let Some(track) = self.project.tracks.get_mut(&id) {
            let what = match param {
                MixerParam::Volume => {
                    let next = (track.volume_db + direction as f32 * 1.0).clamp(
                        yeban_app::bridge::FADER_MIN_DB,
                        yeban_app::bridge::FADER_MAX_DB,
                    );
                    track.volume_db = next;
                    format!("{} 音量 {next:+.1} dB", track.name)
                }
                MixerParam::Pan => {
                    let next = (track.pan + direction as f32 * 0.1).clamp(-1.0, 1.0);
                    track.pan = next;
                    format!("{} 声相 {next:+.1}", track.name)
                }
            };
            self.publish_edit(&what);
        }
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
            archive.project = self.project.clone();
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
        // Esc：清计数与半成品序列（GUI 解析对 Esc 是 PassThrough，落到这里）。
        if event.code == KeyCode::Esc {
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
        // 第二段：Vi 导航层（只吃无修饰/仅 Shift 的字符键）。
        if !modified && let KeyCode::Char(ch) = event.code {
            match self.vi.feed_char(ch) {
                ViOutcome::Pending => {}
                ViOutcome::Motion(motion, count) => self.apply_motion(motion, count),
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
                self.bar_cols = (self.bar_cols + 1).min(8);
            }
            Action::ZoomOut => {
                self.bar_cols = self.bar_cols.saturating_sub(1).max(2);
            }
            Action::Undo | Action::Redo => {
                self.log_line("撤销/重做未在 TUI v1 接入（无撤销栈，已登记缺口）".to_owned());
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
            self.log_line("改名在轨道/检查器焦点下可用（Tab 切换焦点）".to_owned());
            return;
        }
        let Some(id) = self.selected_track_id() else {
            return;
        };
        if let Some(track) = self.project.tracks.get(&id) {
            self.rename = Some(track.name.clone());
            self.mode = Mode::Insert;
        }
    }

    fn handle_insert_key(&mut self, event: KeyEvent) {
        match event.code {
            KeyCode::Esc => {
                self.rename = None;
                self.mode = Mode::Normal;
                self.log_line("改名取消".to_owned());
            }
            KeyCode::Enter => {
                let new_name = self.rename.take().unwrap_or_default();
                self.mode = Mode::Normal;
                let trimmed = new_name.trim().to_owned();
                if trimmed.is_empty() {
                    self.log_line("改名取消：名字为空".to_owned());
                    return;
                }
                if let Some(id) = self.selected_track_id() {
                    if let Some(track) = self.project.tracks.get_mut(&id) {
                        track.name = trimmed.clone();
                    }
                    self.publish_edit(&format!("改名为「{trimmed}」"));
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
            "stop" => self.stop(),
            "rewind" => self.go_to_beginning(),
            "export" => {
                // 形态：export wav <path> 或 export <path>（默认 wav）。
                let target = rest.strip_prefix("wav ").map(str::trim).unwrap_or(rest);
                if target.is_empty() {
                    self.log_line("用法：:export wav <path>".to_owned());
                } else if let Err(error) = self.export_wav(Path::new(target)) {
                    self.log_line(error);
                }
            }
            "open" => {
                if rest.is_empty() {
                    self.log_line("用法：:open <path>".to_owned());
                } else {
                    match Self::open_file(Path::new(rest)) {
                        Ok(next) => {
                            *self = next;
                        }
                        Err(error) => self.log_line(error),
                    }
                }
            }
            "demo" => match Self::with_demo() {
                Ok(next) => {
                    *self = next;
                }
                Err(error) => self.log_line(error),
            },
            other => self.log_line(format!("未知命令：{other}")),
        }
    }
}
